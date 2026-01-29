use crate::message::Message;
use core::{
    ffi::c_void,
    fmt,
    num::NonZeroU32,
    ptr::NonNull,
    task::{Poll, Waker},
};
use nng_sys::nng_err;
use std::{future::Future, io};
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
        let err = unsafe { nng_sys::nng_aio_alloc(&mut aio, Some(callback), arg) };
        match err {
            nng_err::NNG_OK => {}
            nng_err::NNG_ENOMEM => {
                panic!("OOM");
            }
            err => {
                unreachable!(
                    "nng_aio_alloc documentation claims nng_err \"{err}\" is never returned"
                );
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
                // any. note that we _don't_ use nng_aio_stop, because that permanently stops
                // the AIO and disallows future operations (returns NNG_ESTOPPED).
                tracing::warn!("wait cancelled");
                crate::block_in_place(|| unsafe {
                    nng_sys::nng_aio_cancel(self.0.aio.as_ptr());
                    nng_sys::nng_aio_wait(self.0.aio.as_ptr());
                });
                // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
                let err = unsafe { nng_sys::nng_aio_result(self.0.aio.as_ptr()) };
                tracing::trace!(msg_set = self.0.msg_was_set, %err, "cancelled while");
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

                match (self.0.msg_was_set, err) {
                    (true, nng_err::NNG_OK) => {
                        // an operation where we set the message (send) succeeded,
                        // in which case NNG is responsible for freeing, so all is well.
                        // still set msg to NULL to make it clear it isn't usable any more.
                        // SAFETY: AIO is valid (AIO is live until `Aio` is dropped).
                        unsafe {
                            nng_sys::nng_aio_set_msg(self.0.aio.as_ptr(), core::ptr::null_mut())
                        };
                        self.0.msg_was_set = false;
                    }
                    (false, nng_err::NNG_OK) => {
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
        // chance that the callback is _still_ executing if it got preempted _just_ before
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
        let err = unsafe { nng_sys::nng_aio_result(self.aio.as_ptr()) };
        tracing::trace!(err = ?err, "nng_aio_result");

        match err {
            nng_err::NNG_OK => {
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
                Ok(())
            }
            _ => Err(AioError::from_nng_err(err)),
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

    /// Underlying NNG operation failed.
    ///
    /// Contains an [`NngError`] which wraps either a legacy integer error code
    /// ([`NngError::V1`]) or a typed `nng_err` ([`NngError::V2`]) depending on
    /// which NNG API returned the error.
    ///
    /// Common error conditions include:
    /// - `NNG_ECONNREFUSED`: Connection refused by remote endpoint
    /// - `NNG_ECONNRESET`: Connection reset by peer
    /// - `NNG_EUNREACHABLE`: Host unreachable
    /// - `NNG_EADDRINUSE`: Address already in use
    /// - `NNG_ESTATE`: Protocol state violation
    ///
    /// See the [NNG documentation](https://nng.nanomsg.org/ref/api/errors.html)
    /// for the complete list of error codes.
    Operation(NngError),
}

impl fmt::Display for AioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AioError::TimedOut => write!(f, "operation timed out"),
            AioError::Cancelled => write!(f, "operation was cancelled"),
            AioError::Operation(nng_err) => write!(f, "{nng_err}"),
        }
    }
}

impl core::error::Error for AioError {}

impl AioError {
    /// Converts an `nng_err` to an `AioError`.
    ///
    /// Note: this function assumes `errno != NNG_OK`.
    pub(crate) fn from_nng_err(err: nng_err) -> Self {
        let code = err as u32;
        if code == nng_err::NNG_ECANCELED as u32 {
            AioError::Cancelled
        } else if code == nng_err::NNG_ETIMEDOUT as u32 {
            AioError::TimedOut
        } else if let Some(special) = try_extract_special_error(code) {
            AioError::Operation(special)
        } else {
            AioError::Operation(NngError::NngError(err))
        }
    }

    /// Converts a raw non-zero errno to a AioError.
    ///
    /// This is used for legacy NNG functions that return `int` error codes.
    pub(crate) fn from_nz_u32(errno: NonZeroU32) -> Self {
        let code = errno.get();
        if code == nng_err::NNG_ECANCELED as u32 {
            AioError::Cancelled
        } else if code == nng_err::NNG_ETIMEDOUT as u32 {
            AioError::TimedOut
        } else {
            let err = match code {
                c if c == nng_err::NNG_EINTR as u32 => nng_err::NNG_EINTR,
                c if c == nng_err::NNG_ENOMEM as u32 => nng_err::NNG_ENOMEM,
                c if c == nng_err::NNG_EINVAL as u32 => nng_err::NNG_EINVAL,
                c if c == nng_err::NNG_EBUSY as u32 => nng_err::NNG_EBUSY,
                c if c == nng_err::NNG_ETIMEDOUT as u32 => nng_err::NNG_ETIMEDOUT,
                c if c == nng_err::NNG_ECONNREFUSED as u32 => nng_err::NNG_ECONNREFUSED,
                c if c == nng_err::NNG_ECLOSED as u32 => nng_err::NNG_ECLOSED,
                c if c == nng_err::NNG_EAGAIN as u32 => nng_err::NNG_EAGAIN,
                c if c == nng_err::NNG_ENOTSUP as u32 => nng_err::NNG_ENOTSUP,
                c if c == nng_err::NNG_EADDRINUSE as u32 => nng_err::NNG_EADDRINUSE,
                c if c == nng_err::NNG_ESTATE as u32 => nng_err::NNG_ESTATE,
                c if c == nng_err::NNG_ENOENT as u32 => nng_err::NNG_ENOENT,
                c if c == nng_err::NNG_EPROTO as u32 => nng_err::NNG_EPROTO,
                c if c == nng_err::NNG_EUNREACHABLE as u32 => nng_err::NNG_EUNREACHABLE,
                c if c == nng_err::NNG_EADDRINVAL as u32 => nng_err::NNG_EADDRINVAL,
                c if c == nng_err::NNG_EPERM as u32 => nng_err::NNG_EPERM,
                c if c == nng_err::NNG_EMSGSIZE as u32 => nng_err::NNG_EMSGSIZE,
                c if c == nng_err::NNG_ECONNABORTED as u32 => nng_err::NNG_ECONNABORTED,
                c if c == nng_err::NNG_ECONNRESET as u32 => nng_err::NNG_ECONNRESET,
                c if c == nng_err::NNG_ECANCELED as u32 => nng_err::NNG_ECANCELED,
                c if c == nng_err::NNG_ENOFILES as u32 => nng_err::NNG_ENOFILES,
                c if c == nng_err::NNG_ENOSPC as u32 => nng_err::NNG_ENOSPC,
                c if c == nng_err::NNG_EEXIST as u32 => nng_err::NNG_EEXIST,
                c if c == nng_err::NNG_EREADONLY as u32 => nng_err::NNG_EREADONLY,
                c if c == nng_err::NNG_EWRITEONLY as u32 => nng_err::NNG_EWRITEONLY,
                c if c == nng_err::NNG_ECRYPTO as u32 => nng_err::NNG_ECRYPTO,
                c if c == nng_err::NNG_EPEERAUTH as u32 => nng_err::NNG_EPEERAUTH,
                c if c == nng_err::NNG_EBADTYPE as u32 => nng_err::NNG_EBADTYPE,
                c if c == nng_err::NNG_ECONNSHUT as u32 => nng_err::NNG_ECONNSHUT,
                c if c == nng_err::NNG_ESTOPPED as u32 => nng_err::NNG_ESTOPPED,
                c if c == nng_err::NNG_EINTERNAL as u32 => nng_err::NNG_EINTERNAL,
                _ => {
                    return if let Some(special) = try_extract_special_error(code) {
                        AioError::Operation(special)
                    } else {
                        AioError::Operation(NngError::Other(errno))
                    };
                }
            };
            AioError::Operation(NngError::NngError(err))
        }
    }
}

impl From<AioError> for io::Error {
    fn from(err: AioError) -> Self {
        match err {
            AioError::TimedOut => io::Error::new(io::ErrorKind::TimedOut, "operation timed out"),
            AioError::Cancelled => {
                io::Error::new(io::ErrorKind::Interrupted, "operation cancelled")
            }
            AioError::Operation(err) => match err {
                NngError::NngError(err) => match err {
                    nng_err::NNG_OK => unreachable!("NNG_OK should not be passed to this function"),
                    nng_err::NNG_ETIMEDOUT => unreachable!("always handled in outer variant"),
                    nng_err::NNG_ECANCELED => unreachable!("always handled in outer variant"),
                    nng_err::NNG_EINTR => unreachable!("nng never exposes this publicly"),
                    nng_err::NNG_EBUSY => {
                        io::Error::new(io::ErrorKind::ResourceBusy, err.to_string())
                    }
                    nng_err::NNG_EAGAIN => {
                        io::Error::new(io::ErrorKind::WouldBlock, err.to_string())
                    }
                    nng_err::NNG_ENOTSUP => {
                        io::Error::new(io::ErrorKind::Unsupported, err.to_string())
                    }
                    nng_err::NNG_EADDRINUSE => {
                        io::Error::new(io::ErrorKind::AddrInUse, err.to_string())
                    }
                    nng_err::NNG_ENOENT => io::Error::new(io::ErrorKind::NotFound, err.to_string()),
                    nng_err::NNG_EPERM => {
                        io::Error::new(io::ErrorKind::PermissionDenied, err.to_string())
                    }
                    nng_err::NNG_EMSGSIZE => {
                        io::Error::new(io::ErrorKind::InvalidData, err.to_string())
                    }
                    nng_err::NNG_ECONNABORTED => {
                        io::Error::new(io::ErrorKind::ConnectionAborted, err.to_string())
                    }
                    nng_err::NNG_ENOFILES => {
                        io::Error::new(io::ErrorKind::QuotaExceeded, err.to_string())
                    }
                    nng_err::NNG_ENOSPC => {
                        io::Error::new(io::ErrorKind::StorageFull, err.to_string())
                    }
                    nng_err::NNG_EEXIST => {
                        io::Error::new(io::ErrorKind::AlreadyExists, err.to_string())
                    }
                    nng_err::NNG_EREADONLY | nng_err::NNG_EWRITEONLY => {
                        io::Error::new(io::ErrorKind::Unsupported, err.to_string())
                    }
                    nng_err::NNG_EBADTYPE | nng_err::NNG_EADDRINVAL | nng_err::NNG_EINVAL => {
                        io::Error::new(io::ErrorKind::InvalidInput, err.to_string())
                    }
                    nng_err::NNG_ECONNSHUT | nng_err::NNG_ECLOSED => {
                        io::Error::new(io::ErrorKind::BrokenPipe, err.to_string())
                    }
                    nng_err::NNG_ECONNREFUSED => {
                        io::Error::new(io::ErrorKind::ConnectionRefused, err.to_string())
                    }
                    nng_err::NNG_ECONNRESET => {
                        io::Error::new(io::ErrorKind::ConnectionReset, err.to_string())
                    }
                    nng_err::NNG_ENOMEM => {
                        io::Error::new(io::ErrorKind::OutOfMemory, err.to_string())
                    }
                    nng_err::NNG_EPEERAUTH => {
                        io::Error::new(io::ErrorKind::ConnectionAborted, err.to_string())
                    }
                    nng_err::NNG_EUNREACHABLE => {
                        io::Error::new(io::ErrorKind::HostUnreachable, err.to_string())
                    }
                    // All other errors (NNG_ECRYPTO, NNG_EINTERNAL, NNG_ESTATE, NNG_EPROTO, etc.)
                    _ => io::Error::other(err.to_string()),
                },
                NngError::Transport(code) => io::Error::other(format!("Transport error #{}", code)),
                NngError::System(kind) => io::Error::from(kind),
                NngError::Other(code) => io::Error::other(code.to_string()),
            },
        }
    }
}

/// Wrapper for NNG error codes across different API styles.
///
/// NNG functions may return either raw integer error codes (legacy pattern) or typed
/// `nng_err` values (NNG 2.0 pattern). This enum provides a unified representation
/// that can hold either form.
///
/// # Variants
///
/// - [`NngError::NngError`]: Typed `nng_err` from functions that have been updated in NNG v2.
/// - [`NngError::Transport`]: Transport-specific error code extracted from nng_err::NNG_ETRANERR.
/// - [`NngError::System`]: System error (errno) mapped to [`io::ErrorKind`] extracted from nng_err::NNG_ESYSERR.
/// - [`NngError::Other`]: Raw error code not captured by other variants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NngError {
    /// Standard NNG error.
    NngError(nng_err),
    /// Transport-specific error.
    Transport(u32),
    /// System error.
    System(io::ErrorKind),
    /// Other error.
    Other(NonZeroU32),
}

impl fmt::Display for NngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NngError::NngError(err) => write!(f, "{err}"),
            NngError::Transport(code) => write!(f, "Transport error #{code}"),
            NngError::System(kind) => write!(f, "System error: {}", io::Error::from(*kind)),
            NngError::Other(code) => write!(f, "Unknown error #{code}"),
        }
    }
}

/// Checks if the error code represents a system or transport error and extracts it.
///
/// Returns `Some(NngError::System(...))` for system errors (NNG_ESYSERR flag set),
/// `Some(NngError::Transport(...))` for transport errors (NNG_ETRANERR flag set),
/// or `None` for standard NNG error codes.
fn try_extract_special_error(code: u32) -> Option<NngError> {
    if (code & (nng_err::NNG_ESYSERR as u32)) != 0 {
        let sys_err = (code & !(nng_err::NNG_ESYSERR as u32)) as i32;
        let kind = io::Error::from_raw_os_error(sys_err).kind();
        Some(NngError::System(kind))
    } else if (code & (nng_err::NNG_ETRANERR as u32)) != 0 {
        let tran_err = code & !(nng_err::NNG_ETRANERR as u32);
        Some(NngError::Transport(tran_err))
    } else {
        None
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
