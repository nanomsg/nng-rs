use crate::message::Message;
use core::{
    ffi::{c_int, c_void},
    fmt,
    num::{NonZero, NonZeroU32},
    ptr::NonNull,
    task::{Poll, Waker},
};
use std::io;
use tokio::sync::Notify;

/// Note that dropping this type will lead to any associated asynchronous operation to be
/// cancelled, and for the caller to be blocked until that cancellation has completely completed.
///
/// # AIO busy state invariant
///
/// Any operation that makes the AIO busy (such as starting an async send or receive)
/// **must** be followed by a call to [`Aio::wait`]. As long as this invariant is maintained,
/// having `&mut Aio` guarantees that the AIO is not busy with another operation.
#[derive(Debug)]
pub(crate) struct Aio {
    aio: NonNull<nng_sys::nng_aio>,
    // aliased, so only allowed to use as &Notify!
    notify: *mut Notify,
    msg_was_set: bool,
}

// SAFETY: NNG AIO operations are thread-safe and can be moved between threads.
//         The underlying nng_aio is protected by NNG's internal locking mechanisms.
unsafe impl Send for Aio {}
unsafe impl Sync for Aio {}

// TODO(jon): add timeout setters: https://nng.nanomsg.org/ref/api/aio.html#set-timeout.
// then, make use of those to allow timing out operations through `Socket` and `ContexfulSocket`.

#[derive(Debug, Clone, Copy)]
pub(crate) enum ImplicationOnMessage {
    Sent,
    Received,
}

impl Drop for Aio {
    fn drop(&mut self) {
        // NOTE(jon): this call will block until the async operation has been cancelled!
        // SAFETY: AIO is valid since it's live while `&self` is.
        crate::block_in_place(|| unsafe { nng_sys::nng_aio_free(self.aio.as_ptr()) });
        // SAFETY: after nng_aio_free, callback cannot be invoked so we own the `Notify` again.
        let _ = unsafe { Box::from_raw(self.notify) };
    }
}

impl Aio {
    pub(crate) fn new() -> Self {
        let notify = Box::into_raw(Box::new(Notify::new()));
        let aio = Self::new_inner(notify_on_call, notify as *mut c_void);
        Self {
            aio,
            notify,
            msg_was_set: false,
        }
    }

    pub(crate) fn new_inner(
        callback: unsafe extern "C" fn(*mut c_void),
        arg: *mut c_void,
    ) -> NonNull<nng_sys::nng_aio> {
        let mut aio = core::ptr::null_mut::<nng_sys::nng_aio>();
        // SAFETY: aio pointer is valid for writing and callback is a valid function pointer.
        let errno = unsafe { nng_sys::nng_aio_alloc(&mut aio, Some(callback), arg) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            nng_sys::NNG_ENOMEM => {
                panic!("OOM");
            }
            errno => {
                unreachable!("nng_aio_alloc documentation claims errno {errno} is never returned");
            }
        }
        NonNull::new(aio).expect("nng_aio_alloc always sets the pointer when it succeeds")
    }

    pub(crate) fn as_ptr(&mut self) -> *mut nng_sys::nng_aio {
        self.aio.as_ptr()
    }

    /// Waits for the current async operation to complete.
    ///
    /// This method **guarantees** that upon return (whether success or error), the AIO is no longer busy.
    /// This is essential for maintaining the [`Aio`] busy state invariant.
    ///
    /// To avoid memory leaks, it is the caller's responsibility to call [`Aio::take_message`] as
    /// appropriate (eg, if a send operation fails or a receive succeeds) before the next
    /// cancellation opportunity (ie, before the next .await) after this function returns.
    pub(crate) async fn wait(
        &mut self,
        msg_implication: ImplicationOnMessage,
    ) -> Result<(), AioError> {
        tracing::trace!("wait");
        struct CancellationGuard<'a>(&'a mut Aio);
        impl Drop for CancellationGuard<'_> {
            fn drop(&mut self) {
                // NOTE(jon): this call blocks until the AIO is fully cancelled (or completed).
                // we have to do that so that we can correctly handle the contained message, if
                // any. note that we _don't_ use nng_aio_stop, because that disallows future calls
                // to `nng_aio_begin`.
                tracing::warn!("wait cancelled");
                crate::block_in_place(|| unsafe {
                    nng_sys::nng_aio_cancel(self.0.aio.as_ptr());
                    nng_sys::nng_aio_wait(self.0.aio.as_ptr());
                });
                // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
                let errno = unsafe { nng_sys::nng_aio_result(self.0.aio.as_ptr()) };
                tracing::trace!(msg_set = self.0.msg_was_set, errno, "cancelled while");
                // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
                debug_assert!(!unsafe { nng_sys::nng_aio_busy(self.0.aio.as_ptr()) });
                // we also need to make sure we eat the notification so that it doesn't spuriously
                // wake up a later wait!
                // SAFETY: `Notify` pointer is valid until `Aio` is dropped.
                let notify = unsafe { &*self.0.notify };
                let fut = core::pin::pin!(notify.notified());
                let mut ctx = core::task::Context::from_waker(Waker::noop());
                match fut.poll(&mut ctx) {
                    Poll::Ready(_) => {}
                    Poll::Pending => {
                        // the call to cancel should guarantee that the callback has fired,
                        // either due to the cancel, or because the operation had already
                        // completed. the callback always invokes notify_one, and no-one else
                        // has consumed that signal (Notified.await got cancelled, that's how
                        // we ended up in this `Drop`), so this _must_ be ready immediately.
                        unreachable!("cancel guarantees that callback will be fired")
                    }
                }

                match (self.0.msg_was_set, errno) {
                    (true, 0) => {
                        // an operation where we set the message (send) succeeded,
                        // in which case NNG is responsible for freeing, so all is well.
                        // still set msg to NULL to make it clear it isn't usable any more.
                        // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
                        unsafe {
                            nng_sys::nng_aio_set_msg(self.0.aio.as_ptr(), core::ptr::null_mut())
                        };
                        self.0.msg_was_set = false;
                    }
                    (false, 0) => {
                        // an operation where we didn't set the message (recv) succeeded,
                        // so we now own the (incoming) message, if any.
                        // we'll leave it in the Aio so that subsequent calls have a hope to
                        // recover it when resumed.
                        tracing::debug!("receive completed so may be recoverable");
                    }
                    (true, _) => {
                        // an operation where we set the message (send) failed,
                        // so we own dropping the (outgoing) message.
                        let _ = self.0.take_message();
                    }
                    (false, _) => {
                        // an operation where we didn't set the message (recv) failed,
                        // so there is no incoming message we'd have to drop.
                    }
                }
            }
        }
        let guard = CancellationGuard(self);

        tracing::trace!(ptr = ?guard.0.notify, "awaiting notify");
        {
            // SAFETY: `Notify` pointer is valid until `Aio` is dropped.
            let notify = unsafe { &*guard.0.notify };
            notify.notified().await;
        }
        tracing::trace!("notified");

        // there are no .await points below this line, so we can no longer be cancelled!
        core::mem::forget(guard);

        // NOTE(jon): at this point we know that the callback has finished, but there is a small
        // chance that the callback is _still_ executing if it got pre-empted _just_ before
        // returning (but after notifying). so, we also call nng_aio_wait to ensure we're fully
        // past the callback section.

        // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
        tracing::trace!("nng_aio_wait");
        unsafe { nng_sys::nng_aio_wait(self.aio.as_ptr()) };
        tracing::trace!("nng_aio_wait returned");
        debug_assert!(
            !unsafe { nng_sys::nng_aio_busy(self.aio.as_ptr()) },
            "aio shouldn't be busy after wait"
        );

        // now that the operation is fully complete, we can get its result

        // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
        let errno = unsafe { nng_sys::nng_aio_result(self.aio.as_ptr()) };
        tracing::trace!(errno, "nng_aio_result");
        if errno == 0 {
            match msg_implication {
                ImplicationOnMessage::Sent => {
                    // the AIO's msg is now gone, so let's make sure we don't refer to it any more
                    unsafe { nng_sys::nng_aio_set_msg(self.as_ptr(), core::ptr::null_mut()) };
                    self.msg_was_set = false
                }
                ImplicationOnMessage::Received => {
                    // the AIO's msg is now set, but it's up to the caller to extract it (eg, with
                    // `take_message`) as documented in this method's signature.
                }
            }
        }

        match NonZeroU32::new(u32::try_from(errno).expect("errno is never negative")) {
            None => Ok(()),
            Some(errno) => Err(match errno.get() {
                nng_sys::NNG_ETIMEDOUT => AioError::TimedOut,
                nng_sys::NNG_ECANCELED => AioError::Cancelled,
                _ => AioError::Operation(errno),
            }),
        }
    }

    /// Returns the previously set [`Message`], if any.
    pub(crate) fn set_message(&mut self, msg: Message) -> Option<Message> {
        // we have &mut self, so per the `Aio` busy state invariant, there shouldn't be an ongoing
        // asynchronous operation, but let's check in debug mode just in case.
        //
        // SAFETY: AIO is valid (AIO is live until `self` drops).
        debug_assert!(!unsafe { nng_sys::nng_aio_busy(self.as_ptr()) });

        let mut prev = None;
        if self.msg_was_set {
            // SAFETY: AIO is valid (AIO is live until `self` drops).
            let old_msg = unsafe { nng_sys::nng_aio_get_msg(self.as_ptr()) };
            prev = NonNull::new(old_msg).map(|old_msg| {
                // SAFETY: nng_aio_get_msg only ever returns valid messages.
                //         we also know the message is owned since there is no ongoing operation,
                //         we always pass ownership _in_ when we call set_msg (here), and we set
                //         the pointer to a different value just below so that nothing can see the
                //         pointer to _this_ message from anywhere else.
                unsafe { Message::from_raw_unchecked(old_msg) }
            });
        } else {
            debug_assert_eq!(
                unsafe { nng_sys::nng_aio_get_msg(self.as_ptr()) },
                core::ptr::null_mut()
            );
        }

        // SAFETY: AIO is valid (AIO is live until `self` drops)
        unsafe { nng_sys::nng_aio_set_msg(self.as_ptr(), msg.into_ptr()) };
        self.msg_was_set = true;

        prev
    }

    /// Returns `None` if there is no [`Message`] associated with the current asynchronous
    /// operation (eg, if `take_msg` has already been called).
    pub(crate) fn take_message(&mut self) -> Option<Message> {
        // we have &mut self, so per the `Aio` busy state invariant, there shouldn't be an ongoing
        // asynchronous operation, but let's check in debug mode just in case.
        //
        // SAFETY: AIO is valid (AIO is live until `self` drops).
        debug_assert!(!unsafe { nng_sys::nng_aio_busy(self.as_ptr()) });

        // SAFETY: AIO is valid and not busy (per `Aio` busy state invariant).
        let msg = unsafe { nng_sys::nng_aio_get_msg(self.as_ptr()) };

        let msg = NonNull::new(msg)?;

        // reset the message in the AIO so we don't allow taking it again
        //
        // SAFETY: AIO is valid (AIO is live until `self` drops).
        unsafe { nng_sys::nng_aio_set_msg(self.as_ptr(), core::ptr::null_mut()) };
        self.msg_was_set = false;

        // SAFETY: nng_aio_get_msg only ever returns valid messages.
        //         we also know the message is owned since there is no ongoing operation, we always
        //         pass ownership _in_ when we call set_msg, and we reset the pointer to NULL
        //         once we've read the message out once (here).
        Some(unsafe { Message::from_raw_unchecked(msg) })
    }
}

/// Errors that can occur during async I/O operations.
///
/// `AioError` represents the various failure conditions that can occur when
/// performing asynchronous operations with NNG sockets.
///
/// # Error categories
///
/// ## Transient errors
/// - [`AioError::TimedOut`]: Operation exceeded its timeout period. Only returned if a timeout is
///   specifically set for the operation, which is not usually the case.
/// - [`AioError::Cancelled`]: Operation was cancelled (usually due to dropping the future). This
///   error variant is usually not exposed to application code, and tends to only arise internally
///   in the anng state machines.
///
/// ## Protocol/Network errors
/// - [`AioError::Operation`]: Underlying NNG operation failed (see error code for details).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AioError {
    /// Operation exceeded its timeout period.
    ///
    /// This indicates the operation was started but did not complete within
    /// the configured timeout. The operation has been cancelled and can be
    /// safely retried.
    ///
    /// # Common Causes
    /// - Network connectivity issues
    /// - Remote endpoint overloaded or unresponsive
    /// - Configured timeout too aggressive for network conditions
    TimedOut,

    /// Operation was cancelled before completion.
    ///
    /// This typically occurs when:
    /// - The future was dropped before completion
    /// - A timeout was applied externally (e.g., `tokio::time::timeout`)
    /// - The application is shutting down
    ///
    /// Cancelled operations are cleaned up properly and leave the socket
    /// in a consistent state.
    Cancelled,

    /// Underlying NNG operation failed with the given error code.
    ///
    /// The error code corresponds to NNG's error constants. Common codes include:
    /// - `NNG_ECONNREFUSED` (61): Connection refused by remote endpoint
    /// - `NNG_ECONNRESET` (54): Connection reset by peer
    /// - `NNG_EHOSTUNREACH` (65): Host unreachable
    /// - `NNG_EADDRINUSE` (48): Address already in use
    /// - `NNG_ESTATE` (71): Protocol state violation
    ///
    /// See the [NNG documentation](https://nng.nanomsg.org/man/v1.10.0/nng.7.html#error_codes)
    /// for a complete list of error codes. You can also get them from the [`nng_sys`] crate.
    Operation(NonZeroU32),
}

impl fmt::Display for AioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AioError::TimedOut => write!(f, "operation timed out"),
            AioError::Cancelled => write!(f, "operation was cancelled"),
            AioError::Operation(code) => {
                let error_str = crate::nng_strerror(code.get() as c_int);
                write!(f, "{}", error_str.to_string_lossy())
            }
        }
    }
}

impl core::error::Error for AioError {}

impl AioError {
    pub(crate) fn from_nz_u32(errno: NonZero<u32>) -> Self {
        match errno.get() {
            0 => unreachable!("from NonZero"),
            nng_sys::NNG_ECANCELED => AioError::Cancelled,
            nng_sys::NNG_ETIMEDOUT => AioError::TimedOut,
            _ => AioError::Operation(errno),
        }
    }

    pub(crate) fn try_from_u32(errno: u32) -> Result<(), Self> {
        if let Some(nz) = NonZero::new(errno) {
            Err(Self::from_nz_u32(nz))
        } else {
            Ok(())
        }
    }

    pub(crate) fn try_from_i32(errno: i32) -> Result<(), Self> {
        let errno = u32::try_from(errno).expect("nng errno is never negative");
        Self::try_from_u32(errno)
    }
}

impl From<AioError> for io::Error {
    fn from(err: AioError) -> Self {
        match err {
            AioError::TimedOut => io::Error::new(io::ErrorKind::TimedOut, "operation timed out"),
            AioError::Cancelled => {
                io::Error::new(io::ErrorKind::Interrupted, "operation cancelled")
            }
            AioError::Operation(errno) => match errno.get() {
                0 => unreachable!("came from NonZero<u32>"),
                nng_sys::NNG_ETIMEDOUT => unreachable!("always handled in outer variant"),
                nng_sys::NNG_ECANCELED => unreachable!("always handled in outer variant"),
                nng_sys::NNG_EINTR => unreachable!("nng never exposes this publicly"),
                nng_sys::NNG_EBUSY => io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EAGAIN => io::Error::new(
                    io::ErrorKind::WouldBlock,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ENOTSUP => io::Error::new(
                    io::ErrorKind::Unsupported,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EADDRINUSE => io::Error::new(
                    io::ErrorKind::AddrInUse,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ENOENT => io::Error::new(
                    io::ErrorKind::NotFound,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EPERM => io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EMSGSIZE => io::Error::new(
                    io::ErrorKind::FileTooLarge,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ECONNABORTED => io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ENOFILES => io::Error::new(
                    io::ErrorKind::QuotaExceeded,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ENOSPC => io::Error::new(
                    io::ErrorKind::StorageFull,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EEXIST => io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EREADONLY => io::Error::new(
                    io::ErrorKind::Unsupported,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EWRITEONLY => io::Error::new(
                    io::ErrorKind::Unsupported,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ECRYPTO => {
                    io::Error::other(crate::nng_errno_to_string(errno.get() as c_int))
                }
                nng_sys::NNG_ENOARG => io::Error::new(
                    io::ErrorKind::InvalidInput,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EAMBIGUOUS => {
                    io::Error::other(crate::nng_errno_to_string(errno.get() as c_int))
                }
                nng_sys::NNG_EBADTYPE => io::Error::new(
                    io::ErrorKind::InvalidInput,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ECONNSHUT => io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EINTERNAL => {
                    io::Error::other(crate::nng_errno_to_string(errno.get() as c_int))
                }
                nng_sys::NNG_EADDRINVAL => io::Error::new(
                    io::ErrorKind::InvalidInput,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ECLOSED => io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ESTATE => {
                    io::Error::other(crate::nng_errno_to_string(errno.get() as c_int))
                }
                nng_sys::NNG_ECONNREFUSED => io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ECONNRESET => io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EINVAL => io::Error::new(
                    io::ErrorKind::InvalidInput,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_ENOMEM => io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EPEERAUTH => io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                nng_sys::NNG_EPROTO => {
                    io::Error::other(crate::nng_errno_to_string(errno.get() as c_int))
                }
                nng_sys::NNG_EUNREACHABLE => io::Error::new(
                    io::ErrorKind::HostUnreachable,
                    crate::nng_errno_to_string(errno.get() as c_int),
                ),
                errno if (errno & nng_sys::NNG_ESYSERR) != 0 => {
                    io::Error::from_raw_os_error(errno as i32 & !(nng_sys::NNG_ESYSERR as i32))
                }
                errno if (errno & nng_sys::NNG_ETRANERR) != 0 => {
                    // this is a bit flag that indicates an underlying transport level error
                    // since this isn't a system error (like `NNG_ESYSERR`), we can't turn it into
                    // anything more specific, so we just bubble it up as "other".
                    io::Error::other(crate::nng_errno_to_string(errno as c_int))
                }
                errno => {
                    tracing::error!(errno, "unknown nng error code surfaced");
                    io::Error::other(crate::nng_errno_to_string(errno as c_int))
                }
            },
        }
    }
}

/// # Safety
///
/// `arg` must be valid to access as `&Notify`.
unsafe extern "C" fn notify_on_call(arg: *mut c_void) {
    // SAFETY: by the safety requirement of the function that sets this function as a callback
    let notify = unsafe { &*(arg as *const Notify) };
    tracing::trace!(ptr = ?arg, "notify_on_call notify");
    notify.notify_one();
    tracing::trace!(ptr = ?arg, "notify_on_call return");
}
