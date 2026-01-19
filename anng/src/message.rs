use crate::{AioError, pipes::Addr};
use bytes::{Buf, BufMut};
use core::{
    cmp::max,
    ffi::{c_char, c_void},
    fmt,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr::NonNull,
};
use nng_sys::nng_err;
use std::io;

/// A memory-managed wrapper around NNG messages.
///
/// # Message structure
///
/// Each message has two parts:
/// - **Header**: Protocol-specific metadata (usually empty for application use)
/// - **Body**: Application data payload
///
/// Both parts can be read and modified independently.
///
/// # Memory management
///
/// Message ownership follows Rust semantics:
/// - Creating a message allocates NNG memory
/// - Successful send operations transfer ownership to NNG
/// - Failed sends return ownership to the caller
/// - Receives transfer ownership from NNG to the caller
/// - Dropping a message frees the underlying NNG memory
///
/// ## Buffer growth strategy
///
/// The [`Message::reserve`] method uses an exponential growth strategy to
/// minimize reallocations during incremental growth. It follows the
/// implementation of [Vec::reserve].
///
/// Note that the growth strategy is an implementation detail and may change in
/// future versions, though it will always guarantee `O(1)` amortized push
/// operations.
///
/// For exact allocations without growth overhead, use
/// [`Message::reserve_exact`].
///
/// ### NNG internal allocation strategy
///
/// NNG's internal memory allocation strategy adds 32 bytes to non-power-of-two
/// allocations for sizes less than 1024 bytes. This implementation detail
/// provides additional buffer space for protocol headers and efficient memory
/// alignment.
///
/// ```rust
/// # use anng::Message;
/// // Power-of-two sizes get exact allocation (when >= 1024)
/// assert_eq!(Message::with_capacity(1024).capacity(), 1024);
/// assert_eq!(Message::with_capacity(2048).capacity(), 2048);
///
/// // Non-power-of-two sizes get +32 bytes (when < 1024)
/// assert_eq!(Message::with_capacity(0).capacity(), 32);
/// assert_eq!(Message::with_capacity(1).capacity(), 33);
/// assert_eq!(Message::with_capacity(32).capacity(), 64);  // 32 is power-of-two, gets doubled
///
/// // Sizes > 1024 that aren't power-of-two also get +32 bytes
/// assert_eq!(Message::with_capacity(1025).capacity(), 1057);
/// ```
///
/// # Examples
///
/// ## Creating and writing messages
///
/// ```rust
/// use anng::Message;
/// use std::io::Write;
///
/// let mut msg = Message::with_capacity(1024);
///
/// // Write to the message body
/// write!(&mut msg, "Hello, world!")?;
/// write!(&mut msg, " Additional data.")?;
///
/// assert_eq!(msg.as_slice(), b"Hello, world! Additional data.");
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// ## Working with headers
///
/// ```rust
/// use anng::Message;
///
/// let mut msg = Message::with_capacity(100);
/// msg.write_header(b"header-data")?;
/// msg.prepend_header(b"prefix-")?;
///
/// assert_eq!(msg.header(), b"prefix-header-data");
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// ## Message manipulation
///
/// ```rust
/// use anng::Message;
/// use std::io::Write;
///
/// let mut msg = Message::with_capacity(100);
/// write!(&mut msg, "Hello, world!")?;
///
/// // Truncate from the end
/// msg.truncate(5);
/// assert_eq!(msg.as_slice(), b"Hello");
///
/// // Truncate from the beginning
/// msg.truncate_front(2);
/// assert_eq!(msg.as_slice(), b"lo");
///
/// // Prepend data
/// msg.prepend(b"He")?;
/// assert_eq!(msg.as_slice(), b"Helo");
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Message {
    inner: NonNull<nng_sys::nng_msg>,
    /// Current position for reading operations (used by Buf trait)
    pos: usize,
}

// SAFETY: NNG messages are thread-safe and can be shared between threads.
unsafe impl Send for Message {}
unsafe impl Sync for Message {}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Message");
        debug.field("header", &self.header());
        debug.field("body", &self.as_slice());
        if self.pos != 0 {
            debug.field("pos", &self.pos);
        }
        debug.finish()
    }
}

impl Clone for Message {
    fn clone(&self) -> Self {
        let mut msg = core::ptr::null_mut::<nng_sys::nng_msg>();
        // SAFETY: msg pointer is valid for writing and the original message is valid.
        let errno = unsafe { nng_sys::nng_msg_dup(&mut msg, self.inner.as_ptr()) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            errno => {
                unreachable!("nng_msg_dup documentation claims errno {errno} is never returned");
            }
        }
        Message {
            inner: NonNull::new(msg).expect("nng_msg_dup always sets the pointer when it succeeds"),
            pos: self.pos,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.truncate(0);
        let _ = io::Write::write(self, source.as_slice()).expect("OOM");
        self.header_truncate(0);
        let _ = self.write_header(source.header()).expect("OOM");
        self.pos = source.pos;
    }
}

impl Drop for Message {
    fn drop(&mut self) {
        // SAFETY: message is valid and not already freed (message is live until `self` drops).
        unsafe { nng_sys::nng_msg_free(self.inner.as_ptr()) };
    }
}

impl Default for Message {
    fn default() -> Self {
        Self::new()
    }
}

impl Message {
    /// # Safety
    ///
    /// Caller guarantees that pointer is to a valid message with no other accessors.
    pub(crate) unsafe fn from_raw_unchecked(msg: NonNull<nng_sys::nng_msg>) -> Self {
        Self { inner: msg, pos: 0 }
    }

    /// Creates a new, empty message.
    ///
    /// This still allocates, as NNG creates messages with a minimum capacity 32.
    ///
    /// Use [`Message::with_capacity()`] if you know the expected size
    /// to avoid reallocations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::new();
    /// assert!(msg.is_empty());
    /// assert_eq!(msg.len(), 0);
    ///
    /// write!(&mut msg, "Hello, world!")?;
    /// assert_eq!(msg.as_slice(), b"Hello, world!");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    /// Creates a new message with the specified initial capacity.
    ///
    /// The message will have the requested capacity available for writing,
    /// but will start with zero length (both header and body will be empty).
    /// This pre-allocates memory to avoid reallocations during writing.
    ///
    /// # Panics
    ///
    /// Panics if memory allocation fails (out of memory condition).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(1024);
    /// assert!(msg.is_empty());
    /// assert!(msg.capacity() >= 1024);
    ///
    /// write!(&mut msg, "Hello")?;
    /// assert_eq!(msg.len(), 5);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn with_capacity(size: usize) -> Self {
        let mut msg = core::ptr::null_mut::<nng_sys::nng_msg>();
        // SAFETY: msg pointer is valid for writing.
        let errno = unsafe { nng_sys::nng_msg_alloc(&mut msg, size) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            errno => {
                unreachable!("nng_msg_alloc documentation claims errno {errno} is never returned");
            }
        }
        let mut msg = Message {
            inner: NonNull::new(msg)
                .expect("nng_msg_alloc always sets the pointer when it succeeds"),
            pos: 0,
        };
        // nng_msg_alloc allocates _and_ sets the length
        msg.truncate(0);
        msg
    }

    /// Returns the current capacity of the message buffer.
    ///
    /// This is the total amount of memory allocated for the message body,
    /// which may be larger than the current length to avoid frequent
    /// reallocations.
    ///
    /// Note: The capacity includes NNG's internal headroom (typically 32 bytes)
    /// reserved for protocol headers. This headroom _may_ be allocated before
    /// the message body, so the capacity does not necessarily represent
    /// contiguous space available for appending data.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    ///
    /// let msg = Message::with_capacity(1024);
    /// assert!(msg.capacity() >= 1024);
    /// ```
    pub fn capacity(&self) -> usize {
        // SAFETY: message is valid (message is live until `self` drops).
        unsafe { nng_sys::nng_msg_capacity(self.inner.as_ptr()) }
    }

    /// Returns the current length of the message body in bytes.
    ///
    /// This does not include the header length.
    /// Use [`Message::header_len()`] to get the header size.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// assert_eq!(msg.len(), 0);
    ///
    /// write!(&mut msg, "Hello")?;
    /// assert_eq!(msg.len(), 5);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn len(&self) -> usize {
        // SAFETY: message is valid (message is live until `self` drops).
        unsafe { nng_sys::nng_msg_len(self.inner.as_ptr()) }
    }

    /// Returns `true` if the message body is empty.
    ///
    /// This is equivalent to checking `msg.len() == 0`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// assert!(msg.is_empty());
    ///
    /// write!(&mut msg, "data")?;
    /// assert!(!msg.is_empty());
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the current length of the message header in bytes.
    ///
    /// Headers are typically used by protocols for metadata and are usually
    /// empty for application-level messages.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// assert_eq!(msg.header_len(), 0);
    ///
    /// msg.write_header(b"metadata")?;
    /// assert_eq!(msg.header_len(), 8);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn header_len(&self) -> usize {
        // SAFETY: message is valid (message is live until `self` drops).
        unsafe { nng_sys::nng_msg_header_len(self.inner.as_ptr()) }
    }

    /// Returns a slice of the message header data.
    ///
    /// Headers contain protocol-specific metadata and are separate from
    /// the message body. Most application code will not need to use headers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// msg.write_header(b"protocol-info")?;
    ///
    /// assert_eq!(msg.header(), b"protocol-info");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn header(&self) -> &[u8] {
        // SAFETY: message is valid (message is live until `self` drops).
        let header_ptr = unsafe { nng_sys::nng_msg_header(self.inner.as_ptr()) };
        let len = self.header_len();
        // SAFETY: nng_msg_header returns valid pointer into message with length from nng_msg_header_len.
        unsafe { core::slice::from_raw_parts(header_ptr as *const u8, len) }
    }

    fn pipe(&self) -> Option<nng_sys::nng_pipe> {
        // SAFETY: message is valid (message is live until `self` drops).
        let pipe = unsafe { nng_sys::nng_msg_get_pipe(self.inner.as_ptr()) };
        if pipe._bindgen_opaque_blob > 0 {
            Some(pipe)
        } else {
            None
        }
    }

    /// Returns the remote address of the message (ie, where it was received from), if available.
    pub fn remote_addr(&self) -> Option<Addr> {
        let pipe = self.pipe()?;

        // SAFETY: arg and addr are both valid, and on success, nng_pipe_get_addr initializes
        let addr = unsafe {
            let mut addr = MaybeUninit::<nng_sys::nng_sockaddr>::uninit();
            let err = nng_sys::nng_pipe_get_addr(
                pipe,
                nng_sys::NNG_OPT_REMADDR as *const _ as *const c_char,
                addr.as_mut_ptr(),
            );
            match err {
                nng_err::NNG_OK => addr.assume_init(),
                nng_err::NNG_ENOTSUP => {
                    tracing::warn!("Message pipe does not support REMADDR");
                    return None;
                }
                nng_err::NNG_ENOENT => {
                    tracing::warn!("Message does not have a pipe");
                    return None;
                }
                err => {
                    let errno = err as u32;
                    if (errno & nng_err::NNG_ESYSERR as u32) != 0 {
                        tracing::warn!(
                            "nng_pipe_get_addr returned a system error: {}",
                            io::Error::from_raw_os_error(
                                (errno & !(nng_err::NNG_ESYSERR as u32)) as i32
                            )
                        );
                        return None;
                    }
                    unreachable!(
                        "nng_pipe_get_addr documentation claims err {err:?} is never returned"
                    );
                }
            }
        };

        Addr::from_nng(addr)
    }

    /// Reserves the minimum capacity for at least additional more bytes to be
    /// inserted in the given message.
    ///
    /// Unlike [Message::reserve], this will not deliberately over-allocate to
    /// speculatively avoid frequent allocations.  After calling reserve_exact,
    /// capacity will be greater than or equal to `self.len() + additional`. If
    /// the current capacity is already sufficient, this is a no-op.
    ///
    /// Note: NNG internally eventually adds additional headroom as
    /// described in [NNG internal allocation strategy](crate::message::Message#nng-internal-allocation-strategy).
    ///
    /// # Panics
    ///
    /// Panics if memory allocation fails (out of memory condition).
    pub fn reserve_exact(&mut self, additional: usize) {
        let capacity = self.len() + additional;

        // SAFETY: message is valid (message is live until `self` drops).
        let errno = unsafe { nng_sys::nng_msg_reserve(self.inner.as_ptr(), capacity) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            errno => {
                unreachable!(
                    "nng_msg_reserve documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Reserves capacity for at least `additional` more bytes to be inserted
    /// in the message.
    ///
    /// Unlike [`reserve_exact`](Message::reserve_exact), this method uses an
    /// exponential growth strategy to minimize the number of reallocations when
    /// the message is growing incrementally.
    ///
    /// The growth strategy allocates to the next power of two of the current
    /// capacity, or the required capacity, whichever is larger. This ensures
    /// `O(log n)` reallocations for incremental growth while avoiding
    /// over-allocation for large single writes.
    ///
    /// If the current capacity is already sufficient, this is a no-op.
    ///
    /// # Panics
    ///
    /// Panics if memory allocation fails (out of memory condition).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(10);
    ///
    /// // Reserve space for 100 bytes
    /// msg.reserve(100);
    /// // Actual capacity will be at least 100, but may be larger
    /// // due to exponential growth strategy
    /// assert!(msg.capacity() >= 100);
    ///
    /// // Multiple small writes benefit from the growth strategy
    /// for i in 0..20 {
    ///     write!(&mut msg, "hello {i}")?;
    /// }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn reserve(&mut self, additional: usize) {
        let current_len = self.len();
        let current_capacity = self.capacity();
        let required_capacity = current_len + additional;

        if required_capacity > current_capacity {
            let current_next_power_or_two = current_capacity
                .checked_next_power_of_two()
                .unwrap_or(usize::MAX);
            let new_capacity = max(required_capacity, current_next_power_or_two);

            // Reserve the additional capacity needed
            let additional = new_capacity.saturating_sub(current_len);
            self.reserve_exact(additional);
        }
    }

    /// Truncates the message body to the specified length.
    ///
    /// If `len` is greater than the current length, this is a no-op.
    /// This removes data from the end of the message body.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "Hello, world!")?;
    /// assert_eq!(msg.len(), 13);
    ///
    /// msg.truncate(5);
    /// assert_eq!(msg.as_slice(), b"Hello");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn truncate(&mut self, len: usize) {
        let Some(chop) = self.len().checked_sub(len) else {
            return;
        };

        // SAFETY: message is valid (message is live until `self` drops).
        let errno = unsafe { nng_sys::nng_msg_chop(self.inner.as_ptr(), chop) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("we checked that we're chopping no more than the body length");
            }
            errno => {
                unreachable!("nng_msg_chop documentation claims errno {errno} is never returned");
            }
        }

        // Adjust position if it's beyond the new length
        if self.pos > len {
            self.pos = len;
        }
    }

    /// Truncates the message body from the front to the specified final length.
    ///
    /// This removes data from the beginning of the message body, keeping
    /// the last `len` bytes. If `len` is greater than the current length,
    /// this is a no-op.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "Hello, world!")?;
    ///
    /// msg.truncate_front(6); // Keep last 6 bytes
    /// assert_eq!(msg.as_slice(), b"world!");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn truncate_front(&mut self, len: usize) {
        let Some(trim) = self.len().checked_sub(len) else {
            return;
        };

        // SAFETY: message is valid (message is live until `self` drops).
        let errno = unsafe { nng_sys::nng_msg_trim(self.inner.as_ptr(), trim) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("we checked that we're trimming no more than the body length");
            }
            errno => {
                unreachable!("nng_msg_trim documentation claims errno {errno} is never returned");
            }
        }

        // When trimming from the front, adjust position accordingly
        if self.pos >= trim {
            self.pos -= trim;
        } else {
            self.pos = 0;
        }
    }

    pub fn header_truncate(&mut self, len: usize) {
        let Some(chop) = self.header_len().checked_sub(len) else {
            return;
        };

        // SAFETY: message is valid (message is live until `self` drops).
        let errno = unsafe { nng_sys::nng_msg_header_chop(self.inner.as_ptr(), chop) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("we checked that we're chopping no more than the header length");
            }
            errno => {
                unreachable!(
                    "nng_msg_header_chop documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    pub fn header_truncate_front(&mut self, len: usize) {
        let Some(trim) = self.header_len().checked_sub(len) else {
            return;
        };

        // SAFETY: message is valid (message is live until `self` drops).
        let errno = unsafe { nng_sys::nng_msg_header_trim(self.inner.as_ptr(), trim) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("we checked that we're trimming no more than the header length");
            }
            errno => {
                unreachable!(
                    "nng_msg_header_trim documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Prepends data to the beginning of the message body.
    ///
    /// This inserts the provided data at the start of the message,
    /// shifting existing content to the right.
    ///
    /// NNG usually reserves a small amount of space on the left-hand side of each message such
    /// that a small prepend is cheap (i.e., doesn't require shifting or re-allocation), but gives
    /// no guarantees about exactly how much such buffer space is kept.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if memory allocation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "world!")?;
    ///
    /// msg.prepend(b"Hello, ")?;
    /// assert_eq!(msg.as_slice(), b"Hello, world!");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn prepend(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: message is valid (message is live until `self` drops), and
        //         the passed-in data pointer is valid for the given length.
        let errno = unsafe {
            nng_sys::nng_msg_insert(
                self.inner.as_ptr(),
                buf.as_ptr() as *const c_void,
                buf.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(buf.len()),
            x if x == nng_err::NNG_ENOMEM as u32 => {
                Err(Into::into(AioError::from_nng_err(nng_err::NNG_ENOMEM)))
            }
            errno => {
                unreachable!("nng_msg_insert documentation claims errno {errno} is never returned");
            }
        }
    }

    pub fn write_header(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: message is valid (message is live until `self` drops), and
        //         the passed-in data pointer is valid for the given length.
        let errno = unsafe {
            nng_sys::nng_msg_header_append(
                self.inner.as_ptr(),
                buf.as_ptr() as *const c_void,
                buf.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(buf.len()),
            x if x == nng_err::NNG_ENOMEM as u32 => {
                Err(Into::into(AioError::from_nng_err(nng_err::NNG_ENOMEM)))
            }
            errno => {
                unreachable!(
                    "nng_msg_header_append documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    pub fn prepend_header(&mut self, buf: &[u8]) -> io::Result<usize> {
        // SAFETY: message is valid (message is live until `self` drops), and
        //         the passed-in data pointer is valid for the given length.
        let errno = unsafe {
            nng_sys::nng_msg_header_insert(
                self.inner.as_ptr(),
                buf.as_ptr() as *const c_void,
                buf.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(buf.len()),
            x if x == nng_err::NNG_ENOMEM as u32 => {
                Err(Into::into(AioError::from_nng_err(nng_err::NNG_ENOMEM)))
            }
            errno => {
                unreachable!(
                    "nng_msg_header_insert documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Clears the message, removing all data and resetting the position.
    ///
    /// This truncates both the body and header to zero length and resets
    /// the reading position to the beginning.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "Hello, world!")?;
    /// assert_eq!(msg.len(), 13);
    ///
    /// msg.clear();
    /// assert_eq!(msg.len(), 0);
    /// assert!(msg.is_empty());
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn clear(&mut self) {
        self.truncate(0);
        self.header_truncate(0);
        self.pos = 0;
    }

    /// Returns a slice of the message body data.
    ///
    /// The body contains the main application payload of the message.
    /// This does not include the header data.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "Hello, world!")?;
    ///
    /// assert_eq!(msg.as_slice(), b"Hello, world!");
    /// // Can also access via deref
    /// assert_eq!(&*msg, b"Hello, world!");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: message is valid (message is live until `self` drops).
        let body_ptr = unsafe { nng_sys::nng_msg_body(self.inner.as_ptr()) };
        let len = self.len();
        // SAFETY: body pointer is a valid pointer into the message, and
        //         must have been initialized up to `len`.
        unsafe { core::slice::from_raw_parts(body_ptr as *const u8, len) }
    }

    /// Returns a mutable slice of the message body data.
    ///
    /// This allows direct modification of the message contents without
    /// going through the write interface.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "hello")?;
    ///
    /// let slice = msg.as_mut_slice();
    /// slice[0] = b'H'; // Change 'h' to 'H'
    ///
    /// assert_eq!(msg.as_slice(), b"Hello");
    /// // Can also access via deref_mut
    /// msg[1] = b'E'; // Change 'e' to 'E'
    /// assert_eq!(&*msg, b"HEllo");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: message is valid (message is live until `self` drops).
        let body_ptr = unsafe { nng_sys::nng_msg_body(self.inner.as_ptr()) };
        let len = self.len();
        // SAFETY: body pointer is a valid pointer into the message,
        //         must have been initialized up to `len`, and
        //         no other accesses to `nng_msg` exist (which having a `&mut Message` guarantees)
        unsafe { core::slice::from_raw_parts_mut(body_ptr as *mut u8, len) }
    }

    /// Sets the length of the message body.
    ///
    /// This can be used to extend or truncate the message without
    /// initializing the new space. When extending, the new bytes
    /// will contain uninitialized data.
    ///
    /// # Safety
    ///
    /// When extending the message, the caller must ensure that any
    /// new bytes are properly initialized before reading them.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::Message;
    /// use std::io::Write;
    ///
    /// let mut msg = Message::with_capacity(100);
    /// write!(&mut msg, "Hello, world!")?;
    /// assert_eq!(msg.len(), 13);
    ///
    /// // Truncate to shorter length
    /// unsafe { msg.set_len(5) };
    /// assert_eq!(msg.as_slice(), b"Hello");
    ///
    /// // Extend back (data is still there)
    /// unsafe { msg.set_len(13) };
    /// assert_eq!(msg.as_slice(), b"Hello, world!");
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub unsafe fn set_len(&mut self, len: usize) {
        let current_len = self.len();
        if len > current_len {
            // Need to extend the message
            // Use realloc to ensure capacity, then chop back to desired length
            let errno = unsafe { nng_sys::nng_msg_realloc(self.inner.as_ptr(), len) };
            match u32::try_from(errno).expect("errno is never negative") {
                0 => {
                    // nng_msg_realloc sets the length to the new size, which is what we want
                }
                x if x == nng_err::NNG_ENOMEM as u32 => {
                    panic!("OOM");
                }
                errno => {
                    unreachable!(
                        "nng_msg_realloc documentation claims errno {errno} is never returned"
                    );
                }
            }
        } else if len < current_len {
            // Truncate
            self.truncate(len);
        }
        // Adjust position if it's beyond the new length
        if self.pos > len {
            self.pos = len;
        }
    }

    /// Turn this message into a raw pointer to hand ownership of it into NNG.
    ///
    /// When this function is called, the message is _not_ deallocated. This must thus either be
    /// done internally by NNG (such as if a message is sent successfully), or manually by the
    /// caller by pulling the message back out of, eg, an [`Aio`] on operation failure.
    pub(crate) fn into_ptr(self) -> *mut nng_sys::nng_msg {
        let ptr = self.inner.as_ptr();
        core::mem::forget(self);
        ptr
    }
}

impl From<&[u8]> for Message {
    fn from(data: &[u8]) -> Self {
        let mut msg = Message::with_capacity(data.len());
        let _ = io::Write::write(&mut msg, data).expect("write to new message should never fail");
        msg
    }
}

impl Deref for Message {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_slice()
    }
}

impl DerefMut for Message {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_slice()
    }
}

impl AsRef<[u8]> for Message {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsMut<[u8]> for Message {
    fn as_mut(&mut self) -> &mut [u8] {
        self.as_mut_slice()
    }
}

impl io::Write for Message {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Pre-allocate sufficient capacity to accommodate the incoming data.  This
        // follows an exponential growth strategy to minimize future reallocations while
        // ensuring the append operation completes without memory reallocation.
        self.reserve(buf.len());

        // SAFETY: message is valid (message is live until `self` drops), and
        //         the passed-in data pointer is valid for the given length.
        let errno = unsafe {
            nng_sys::nng_msg_append(
                self.inner.as_ptr(),
                buf.as_ptr() as *const c_void,
                buf.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(buf.len()),
            x if x == nng_err::NNG_ENOMEM as u32 => {
                Err(io::Error::from(AioError::from_nng_err(nng_err::NNG_ENOMEM)))
            }
            errno => {
                unreachable!("nng_msg_append documentation claims errno {errno} is never returned");
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Buf for Message {
    fn remaining(&self) -> usize {
        self.len().saturating_sub(self.pos)
    }

    fn chunk(&self) -> &[u8] {
        &self.as_slice()[self.pos..]
    }

    fn advance(&mut self, cnt: usize) {
        let new_pos = self.pos.saturating_add(cnt);
        if new_pos > self.len() {
            panic!(
                "cannot advance past end of message: pos={}, cnt={}, len={}",
                self.pos,
                cnt,
                self.len()
            );
        }
        self.pos = new_pos;
    }
}

// NOTE(jon): for BufMut, we use the length as the cursor position.
unsafe impl BufMut for Message {
    fn remaining_mut(&self) -> usize {
        // For BufMut, we can expand the message up to usize::MAX theoretically
        // but we'll use a reasonable upper bound to avoid potential issues
        usize::MAX - self.len()
    }

    unsafe fn advance_mut(&mut self, cnt: usize) {
        // This extends the length of the message by cnt bytes
        // The caller must have written to those bytes already
        let new_len = self.len() + cnt;
        unsafe {
            self.set_len(new_len);
        }
    }

    fn chunk_mut(&mut self) -> &mut bytes::buf::UninitSlice {
        // Ensure we have at least some capacity for writing
        let current_len = self.len();
        let current_capacity = self.capacity();

        if current_len >= current_capacity {
            // Need to reserve more space - grow by at least 64 bytes
            self.reserve(64);
        }

        // Get access to the uninitialized part of the buffer
        // SAFETY: message is valid (message is live until `self` drops).
        let body_ptr = unsafe { nng_sys::nng_msg_body(self.inner.as_ptr()) };
        let available_space = self.capacity() - current_len;

        // SAFETY: `body_ptr` is valid and exclusive (as per `&mut self`),
        //         it is valid from `+current_len` to `+current_len+available_space` as
        //         those are both < the message's capacity.
        unsafe {
            bytes::buf::UninitSlice::from_raw_parts_mut(
                body_ptr.add(current_len) as *mut u8,
                available_space,
            )
        }
    }

    fn put_slice(&mut self, src: &[u8]) {
        io::Write::write_all(self, src).expect("write should not fail on message");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Buf, BufMut};
    use std::io::Write;

    #[test]
    fn test_from_slice() {
        let data = b"hello world";
        let msg = Message::from(data.as_slice());
        assert_eq!(msg.as_slice(), data);
        assert_eq!(msg.len(), data.len());
        assert_eq!(msg.pos, 0);
    }

    #[test]
    fn test_deref() {
        let data = b"hello world";
        let msg = Message::from(data.as_slice());
        // Test Deref
        assert_eq!(&*msg, data);
        // Test AsRef
        assert_eq!(msg.as_ref(), data);
    }

    #[test]
    fn test_deref_mut() {
        let mut msg = Message::from(b"hello world".as_slice());
        // Test DerefMut
        msg[0] = b'H';
        assert_eq!(&*msg, b"Hello world");
        // Test AsMut
        let slice: &mut [u8] = msg.as_mut();
        slice[6] = b'W';
        assert_eq!(&*msg, b"Hello World");
    }

    #[test]
    fn test_clear() {
        let mut msg = Message::from(b"hello world".as_slice());
        msg.write_header(b"header").unwrap();
        msg.pos = 5; // Simulate some reading

        assert_eq!(msg.len(), 11);
        assert_eq!(msg.header_len(), 6);
        assert_eq!(msg.pos, 5);

        msg.clear();

        assert_eq!(msg.len(), 0);
        assert_eq!(msg.header_len(), 0);
        assert_eq!(msg.pos, 0);
        assert!(msg.is_empty());
    }

    #[test]
    fn test_as_slice_as_mut_slice() {
        let mut msg = Message::with_capacity(20);
        msg.write_all(b"hello").unwrap();

        assert_eq!(msg.as_slice(), b"hello");

        let slice = msg.as_mut_slice();
        slice[0] = b'H';
        assert_eq!(msg.as_slice(), b"Hello");
    }

    #[test]
    fn test_unsafe_set_len() {
        let mut msg = Message::with_capacity(20);
        msg.write_all(b"hello world").unwrap();
        assert_eq!(msg.len(), 11);

        // Truncate using set_len
        unsafe { msg.set_len(5) };
        assert_eq!(msg.len(), 5);
        assert_eq!(msg.as_slice(), b"hello");

        // Extend back using set_len
        unsafe { msg.set_len(11) };
        assert_eq!(msg.len(), 11);
        assert_eq!(msg.as_slice(), b"hello world");

        // Test position adjustment when extending beyond position
        msg.pos = 8;
        unsafe { msg.set_len(6) };
        assert_eq!(msg.len(), 6);
        assert_eq!(msg.pos, 6); // Position should be adjusted
    }

    #[test]
    fn test_buf_trait() {
        let mut msg = Message::from(b"hello world".as_slice());

        // Test initial state
        assert_eq!(msg.remaining(), 11);
        assert_eq!(msg.chunk(), b"hello world");

        // Test advance
        msg.advance(6);
        assert_eq!(msg.remaining(), 5);
        assert_eq!(msg.chunk(), b"world");
        assert_eq!(msg.pos, 6);

        // Test advance to end
        msg.advance(5);
        assert_eq!(msg.remaining(), 0);
        assert_eq!(msg.chunk(), b"");
        assert_eq!(msg.pos, 11);
    }

    #[test]
    #[should_panic(expected = "cannot advance past end of message")]
    fn test_buf_advance_panic() {
        let mut msg = Message::from(b"hello".as_slice());
        msg.advance(10); // Should panic
    }

    #[test]
    fn test_bufmut_put_slice() {
        let mut msg = Message::with_capacity(10);

        // Test remaining_mut
        assert!(msg.remaining_mut() > 0);

        // Test put_slice
        msg.put_slice(b"hello");
        assert_eq!(msg.as_slice(), b"hello");
        assert_eq!(msg.len(), 5);

        msg.put_slice(b" world");
        assert_eq!(msg.as_slice(), b"hello world");
        assert_eq!(msg.len(), 11);
    }

    #[test]
    fn test_bufmut_chunk_mut_advance_mut() {
        let mut msg = Message::with_capacity(20);

        // Get a chunk to write to
        let chunk = msg.chunk_mut();
        assert!(chunk.len() >= 10);

        // Write some data
        unsafe {
            let slice = chunk.as_uninit_slice_mut();
            slice[0].write(b'h');
            slice[1].write(b'e');
            slice[2].write(b'l');
            slice[3].write(b'l');
            slice[4].write(b'o');
        }

        // Advance to indicate we wrote 5 bytes
        unsafe { msg.advance_mut(5) };

        assert_eq!(msg.len(), 5);
        assert_eq!(msg.as_slice(), b"hello");
    }

    #[test]
    fn test_position_handling_with_truncate() {
        let mut msg = Message::from(b"hello world".as_slice());
        msg.advance(6); // Position at "world"

        // Truncate to remove the end
        msg.truncate(8); // Keep "hello wo"
        assert_eq!(msg.len(), 8);
        assert_eq!(msg.pos, 6);
        assert_eq!(msg.chunk(), b"wo");

        // Truncate beyond position
        msg.truncate(4); // Keep "hell"
        assert_eq!(msg.len(), 4);
        assert_eq!(msg.pos, 4); // Position adjusted
        assert_eq!(msg.remaining(), 0);
    }

    #[test]
    fn test_position_handling_with_truncate_front() {
        let mut msg = Message::from(b"hello world".as_slice());
        msg.advance(6); // Position at "world"

        // Truncate from front, keeping 8 chars: "lo world"
        msg.truncate_front(8);
        assert_eq!(msg.len(), 8);
        assert_eq!(msg.as_slice(), b"lo world");
        assert_eq!(msg.pos, 3); // Position adjusted: 6 - 3 = 3
        assert_eq!(msg.chunk(), b"world");

        // Truncate front more than position
        msg.truncate_front(2); // Keep "ld"
        assert_eq!(msg.len(), 2);
        assert_eq!(msg.as_slice(), b"ld");
        assert_eq!(msg.pos, 0); // Position reset to 0
        assert_eq!(msg.chunk(), b"ld");
    }

    #[test]
    fn test_buf_bufmut_integration() {
        let mut msg = Message::new();

        // Use BufMut to write data
        msg.put_slice(b"hello ");
        msg.put_slice(b"world");
        assert_eq!(msg.as_slice(), b"hello world");

        // Use Buf to read data
        assert_eq!(msg.remaining(), 11);
        let mut read_data = vec![0u8; 5];
        msg.copy_to_slice(&mut read_data);
        assert_eq!(&read_data, b"hello");
        assert_eq!(msg.remaining(), 6);
        assert_eq!(msg.chunk(), b" world");
    }

    #[test]
    fn test_debug_with_position() {
        let mut msg = Message::from(b"hello world".as_slice());
        let debug_str = format!("{:?}", msg);
        // Should not show position when it's 0
        assert!(!debug_str.contains("pos"));

        msg.advance(5);
        let debug_str = format!("{:?}", msg);
        // Should show position when it's non-zero
        assert!(debug_str.contains("pos"));
    }

    #[test]
    fn test_clone_preserves_position() {
        let mut msg = Message::from(b"hello world".as_slice());
        msg.advance(6);

        let cloned = msg.clone();
        assert_eq!(cloned.pos, 6);
        assert_eq!(cloned.chunk(), b"world");
        assert_eq!(cloned.as_slice(), b"hello world");

        // Test clone_from
        let mut msg2 = Message::new();
        msg2.clone_from(&msg);
        assert_eq!(msg2.pos, 6);
        assert_eq!(msg2.as_slice(), b"hello world");
    }

    #[test]
    fn test_reserve_exponential_growth() {
        // Initial message with "0" capacity
        let mut msg = Message::with_capacity(0);

        // NNG internally adds 32 bytes to the capacity.
        assert_eq!(msg.capacity(), 32);

        // Write exactly the number of bytes available
        msg.write_all(&[0u8; 32]).unwrap();

        // The capacity is not increased
        assert_eq!(msg.capacity(), 32);

        // Write one more byte to trigger growth
        // Since 32 is already a power of two, next_power_of_two(32) = 32
        // So we need required_capacity (33) > current_capacity (32)
        // The new capacity will be max(33, 32) = 33, but NNG will round up
        msg.write_all(&[1u8]).unwrap();

        // Verify growth occurred - should be at least 33
        let new_capacity = msg.capacity();
        assert!(new_capacity >= 33);

        // Now test the growth behavior with power-of-two capacities
        // When current capacity is a power of two, next_power_of_two returns the same value
        // So growth only happens when required_capacity > current_capacity
        let mut msg = Message::with_capacity(1024);
        assert_eq!(msg.capacity(), 1024);

        // Fill to capacity
        msg.write_all(&[0u8; 1024]).unwrap();
        assert_eq!(msg.capacity(), 1024);

        // Write one more byte - required is 1025, next_power_of_two(1024) = 1024
        // So new_capacity = max(1025, 1024) = 1025
        // NNG doesn't add +32 for sizes >= 1024
        msg.write_all(&[1u8]).unwrap();
        assert_eq!(msg.capacity(), 1025);

        // Now write enough to exceed next power of two threshold
        // Current capacity is 1025, write to get to 2048
        msg.write_all(&[2u8; 1023]).unwrap(); // Now at 2048 bytes
        assert_eq!(msg.len(), 2048);
        assert_eq!(msg.capacity(), 2048); // Grew to next power of two

        // Write one more byte to exceed 2048
        msg.write_all(&[3u8]).unwrap(); // Now at 2049 bytes
        // required_capacity = 2049, next_power_of_two(2048) = 2048
        // new_capacity = max(2049, 2048) = 2049
        assert_eq!(msg.capacity(), 2049);
    }

    #[test]
    fn test_reserve_exact_amount_for_large_amounts() {
        let mut msg = Message::new();

        // If the required capacity is larger than twice the current capacity,
        // the capacity is not doubled and the exact amount of memory reserved.

        msg.reserve(1024);
        assert_eq!(msg.capacity(), 1024); // 1024 > 2 * 32 (initial default capacity)

        msg.reserve(64 * 1024);
        assert_eq!(msg.capacity(), 64 * 1024); // 64 * 1024 > 1024
    }

    #[test]
    fn test_reserve_exact() {
        // Test 1: Basic reservation on empty message
        let mut msg = Message::new();
        assert_eq!(msg.len(), 0);

        // Reserve exact amount
        msg.reserve_exact(100);
        assert_eq!(msg.capacity(), 100);
        assert_eq!(msg.len(), 0); // Length should remain unchanged

        // Test 2: No-op when capacity is already sufficient
        let current_capacity = msg.capacity();
        msg.reserve_exact(50); // Less than current capacity
        assert_eq!(msg.capacity(), current_capacity); // Should not change

        // Test 3: Reserve exact with existing data
        let mut msg = Message::new();
        msg.write_all(b"hello").unwrap();
        assert_eq!(msg.len(), 5);

        // Reserve exact for 50 additional bytes (total: 5 + 50 = 55)
        msg.reserve_exact(50);
        assert_eq!(msg.capacity(), 55);
        assert_eq!(msg.len(), 5); // Length should remain unchanged
        assert_eq!(msg.as_slice(), b"hello"); // Data should be preserved

        // Test 4: Multiple reserve_exact calls
        let mut msg = Message::new();
        msg.write_all(b"test").unwrap();
        assert_eq!(msg.len(), 4);

        // First reserve_exact
        msg.reserve_exact(10); // Total needed: 4 + 10 = 14
        assert_eq!(msg.capacity(), 32); // NNG internal allocation strategy

        // Second reserve_exact with larger amount
        msg.reserve_exact(100); // Total needed: 4 + 100 = 104
        assert_eq!(msg.capacity(), 104);

        // Test 5: Zero additional bytes (should be no-op)
        let mut msg = Message::new();
        msg.write_all(b"content").unwrap();
        let capacity_before = msg.capacity();
        msg.reserve_exact(0);
        assert_eq!(msg.capacity(), capacity_before);

        // Test 7: Large reservation
        let mut msg = Message::new();
        msg.reserve_exact(4096);
        assert_eq!(msg.capacity(), 4096);

        // Write data and verify it works correctly
        msg.write_all(&vec![b'x'; 2048]).unwrap();
        assert_eq!(msg.len(), 2048);
        assert_eq!(msg.capacity(), 4096); // Capacity should still be what we reserved
    }
}
