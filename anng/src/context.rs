use crate::{
    ContextfulSocket, Socket,
    aio::{Aio, AioError, ImplicationOnMessage},
    message::Message,
};
use core::{fmt, mem::MaybeUninit, num::NonZeroU32, ptr::NonNull};

impl<Protocol: crate::protocols::SupportsContext> Socket<Protocol> {
    /// Creates a new context for concurrent operations on this socket.
    ///
    /// Each context maintains independent protocol state, enabling safe concurrent
    /// operations on the same underlying socket. This is **required** for:
    /// - Handling multiple requests simultaneously in server applications
    /// - Making parallel requests from client applications
    /// - Any scenario where multiple async operations need to be in progress
    ///
    /// # Performance Considerations
    ///
    /// Contexts have minimal overhead - they share the underlying socket's transport
    /// connections and configuration. Creating many contexts is generally acceptable,
    /// though each does consume some memory for state management.
    pub fn context(&self) -> ContextfulSocket<'_, Protocol> {
        let mut context = MaybeUninit::<nng_sys::nng_ctx>::uninit();
        // SAFETY: context pointer is valid for writing, and
        //         socket is valid and not closed (socket is live until `self` drops).
        let errno = unsafe { nng_sys::nng_ctx_open(context.as_mut_ptr(), self.socket) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            nng_sys::NNG_ENOMEM => {
                panic!("OOM");
            }
            nng_sys::NNG_ENOTSUP => {
                // the SupportsContext trait bound ensures this method is only callable
                // on protocols that support contexts, so this error should be impossible.
                unreachable!("protocol supports contexts per SupportsContext trait bound");
            }
            errno => {
                unreachable!("nng_ctx_open documentation claims errno {errno} is never returned");
            }
        }
        // SAFETY: nng_ctx_open initializes context on success.
        let context = unsafe { context.assume_init() };
        let context = Context {
            context,
            recovered_msg: None,
            socket: self,
        };

        let aio = Aio::new();
        ContextfulSocket { aio, context }
    }
}

pub(crate) struct Context<'socket, Protocol> {
    context: nng_sys::nng_ctx,
    recovered_msg: Option<Message>,
    pub(crate) socket: &'socket Socket<Protocol>,
}

// manual impl to avoid Protocol: Debug bound
impl<Protocol> fmt::Debug for Context<'_, Protocol> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Context")
            .field("context", &self.context)
            .field("socket", &self.socket)
            .finish()
    }
}

impl<Protocol> Drop for Context<'_, Protocol> {
    fn drop(&mut self) {
        // SAFETY: context is valid and not already closed (context is live until `self` drops).
        crate::block_in_place(|| unsafe { nng_sys::nng_ctx_close(self.context) });
    }
}

impl<Protocol> Context<'_, Protocol> {
    /// Sends a message using this context.
    ///
    /// If this operation fails, the message will be returned along with the error for potential retry.
    /// If this operation succeeds, the message remains under the ownership of NNG, which will eventually free it.
    ///
    /// Note that NNG does _not_ guarantee that the send has actually completed when this operation
    /// returns, and there is no automatic linger or flush to ensure that the socket send buffers
    /// have completely transmitted when a socket is closed. Thus, it is recommended to wait a
    /// brief period after calling send before letting the socket be dropped.
    pub(crate) async fn send_msg(
        &mut self,
        message: Message,
        aio: &mut Aio,
    ) -> Result<(), (AioError, Message)> {
        aio.set_message(message);
        // SAFETY: context is valid and not closed (context is live until `self` drops),
        //         AIO is valid and not busy (per `Aio` busy state invariant), and
        //         message has been set on AIO (just above).
        unsafe { nng_sys::nng_ctx_send(self.context, aio.as_ptr()) };
        // the above started an async operation (makes AIO busy).
        // we call wait() to preserve the Aio busy invariant.
        match aio.wait(ImplicationOnMessage::Sent).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Return the message that failed to send for potential retry
                let msg = aio
                    .take_message()
                    .expect("we set the message, so it should still be there");
                Err((e, msg))
            }
        }
    }

    pub(crate) fn try_recv_msg(&mut self) -> Result<Option<Message>, AioError> {
        // if we previously recovered a message with this same context,
        // it should be safe to return it here since the same context implies same use.
        if let Some(msg) = self.recovered_msg.take() {
            tracing::debug!("returning recovered message from cancel");
            return Ok(Some(msg));
        }

        let mut msg = core::ptr::null_mut();

        // SAFETY: context is valid and not closed (context is live until `self` drops), and
        //         msg pointer pointer is valid.
        let errno = unsafe {
            nng_sys::nng_ctx_recvmsg(self.context, &mut msg, nng_sys::NNG_FLAG_NONBLOCK as i32)
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {
                let msg =
                    NonNull::new(msg).expect("successful nng_ctx_recvmsg initializes pointer");
                // SAFETY: message returned by nng_ctx_recvmsg is valid and _ours_.
                let msg = unsafe { Message::from_raw_unchecked(msg) };
                Ok(Some(msg))
            }
            nng_sys::NNG_EAGAIN => Ok(None),
            nng_sys::NNG_ECLOSED => {
                unreachable!("socket is still open since we have a reference to it");
            }
            nng_sys::NNG_EINVAL => {
                unreachable!("flags are valid for the call");
            }
            nng_sys::NNG_ENOMEM => {
                panic!("OOM");
            }
            err @ nng_sys::NNG_ENOTSUP => {
                // protocol does not support receiving
                Err(AioError::Operation(
                    NonZeroU32::try_from(err).expect("statically checked to be >0"),
                ))
            }
            err @ nng_sys::NNG_ESTATE => {
                // protocol does not support receiving in its current state
                Err(AioError::Operation(
                    NonZeroU32::try_from(err).expect("statically checked to be >0"),
                ))
            }
            err @ nng_sys::NNG_ETIMEDOUT => {
                // likely due to a protocol-level timeout (like surveys)
                Err(AioError::Operation(
                    NonZeroU32::try_from(err).expect("statically checked to be >0"),
                ))
            }
            errno => {
                unreachable!(
                    "nng_ctx_recvmsg documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    pub(crate) async fn recv_msg(&mut self, aio: &mut Aio) -> Result<Message, AioError> {
        // if we previously recovered a message with this same context,
        // it should be safe to return it here since the same context implies same use.
        if let Some(msg) = self.recovered_msg.take() {
            tracing::debug!("returning recovered message from cancel");
            return Ok(msg);
        }

        // SAFETY: context is valid and not closed (context is live until `self` drops), and
        //         AIO is valid and not busy (per `Aio` busy state invariant).
        unsafe { nng_sys::nng_ctx_recv(self.context, aio.as_ptr()) };
        // the above started an async operation (which makes the AIO busy), so we must eventually
        // call wait() later to preserve the Aio busy invariant.
        //
        // but first have to set up a cancellation guard such that if this future is canceled, we
        // we capture a message that was received _just_ too late (but _was_ received) so that we
        // don't drop it. even if dropping is _okay_ in nng, it's not ideal for the application.
        //
        // NOTE(jon): it is vital that there are no early returns or .await calls between this
        // point and the wait() call below, otherwise we'd violate the Aio busy invariant.

        struct CancellationGuard<'a, 's, Protocol>(&'a mut Context<'s, Protocol>, &'a mut Aio);
        impl<Protocol> Drop for CancellationGuard<'_, '_, Protocol> {
            fn drop(&mut self) {
                // if we got canceled, then _first_ the `.wait()` future is dropped (which will run
                // `aio_stop`), and _then_ this drop code is run. it has to be that way for us to
                // be free to us to get `&mut` since `.wait()` uses the `&mut` to our `.1`.
                // as a result, _if_ a message was recovered by the AIO stop, then it's recoverable
                // by us here now:
                if let Some(msg) = self.1.take_message() {
                    tracing::debug!("received message recovered on cancel");
                    self.0.recovered_msg = Some(msg);
                } else {
                    tracing::trace!("no received message recovered on cancel");
                }
            }
        }
        let guard = CancellationGuard(self, aio);

        // this completes the AIO busy contract started above.
        let result = guard.1.wait(ImplicationOnMessage::Received).await;

        // the future has completed, so no need for the teardown mechanism.
        core::mem::forget(guard);

        result?;

        let msg = aio
            .take_message()
            .expect("we executed a successful receive, so there should be a message");
        Ok(msg)
    }

    pub(crate) fn id(&mut self) -> nng_sys::nng_ctx {
        self.context
    }
}
