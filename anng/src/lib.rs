//! Async Rust bindings for NNG (nanomsg-next-generation)
//!
//! `anng` provides async-first, type-safe Rust bindings for the [NNG](https://nng.nanomsg.org/)
//! (nanomsg-next-generation) messaging library. NNG implements scalability protocols that enable
//! reliable, high-performance communication patterns across different transports.
//!
//! ## Key features
//!
//! - All operations are natively asynchronous and integrate seamlessly with Tokio.
//! - Protocol violations are made impossible using compile-time type-state.
//! - Uses libnng under the hood, so supports all the transports supported by NNG.
//! - All async operations are cancellation safe - futures can be dropped at any time without losing messages or leaking resources.
//!
//! ## Quick start
//!
//! ### Request/Reply pattern
//!
//! ```rust
//! use anng::{protocols::reqrep0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Server side
//! tokio::spawn(async {
//!     let socket = reqrep0::Rep0::listen(c"inproc://quick-start").await?;
//!     let mut ctx = socket.context();
//!     loop {
//!         let (request, responder) = ctx.receive().await?;
//!         assert_eq!(request.as_slice(), b"Hello server!");
//!
//!         let mut reply = Message::with_capacity(100);
//!         write!(&mut reply, "Hello back!")?;
//!         responder.reply(reply).await
//!           .expect("in production, handle error and retry with returned responder");
//!     }
//!     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! });
//! # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//!
//! // Client side
//! let socket = reqrep0::Req0::dial(c"inproc://quick-start").await?;
//! let mut ctx = socket.context();
//!
//! let mut request = Message::with_capacity(100);
//! write!(&mut request, "Hello server!")?;
//!
//! let reply_future = ctx.request(request).await
//!     .expect("in production, handle error and retry with returned request");
//! let reply = reply_future.await?;
//! assert_eq!(reply.as_slice(), b"Hello back!");
//! # Ok(())
//! # }
//! ```
//!
//! ### Publish/Subscribe pattern
//!
//! ```rust
//! use anng::{protocols::pubsub0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Publisher
//! tokio::spawn(async {
//!     let mut socket = pubsub0::Pub0::listen(c"inproc://pubsub").await?;
//!     loop {
//!         let mut msg = Message::with_capacity(100);
//!         write!(&mut msg, "news: Breaking news!")?;
//!         socket.publish(msg).await
//!           .expect("in production, handle error and retry with returned message");
//!         tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//!     }
//!     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! });
//! # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//!
//! // Subscriber
//! let socket = pubsub0::Sub0::dial(c"inproc://pubsub").await?;
//! let mut context = socket.context();
//! context.subscribe_to(b"news:");
//!
//! for i in 0..10 {
//!     let msg = context.next().await?;
//!     assert_eq!(msg.as_slice(), b"news: Breaking news!");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Error handling
//!
//! Most operations return [`AioError`] which represents different failure conditions:
//!
//! - [`AioError::TimedOut`]: Operation exceeded timeout
//! - [`AioError::Cancelled`]: Operation was cancelled
//! - [`AioError::Operation`]: Operation-specific error (connection failed, protocol violation, etc.)

use crate::{
    aio::{Aio, ImplicationOnMessage},
    context::Context,
};
use core::{ffi::CStr, fmt, marker::PhantomData, ptr::NonNull};
use nng_sys::nng_err;
use std::io;

mod aio;
mod context;
mod message;
pub mod pipes;
pub mod protocols;

pub use aio::AioError;
pub use message::Message;

/// A type-safe NNG socket with compile-time protocol verification.
///
/// The `Socket` type uses phantom types to ensure protocol correctness at compile time:
/// - `Protocol` specifies the messaging pattern (Req0, Rep0, Pub0, Sub0, etc.)
///
/// This prevents common errors like trying to receive on a request socket or
/// mixing incompatible protocols.
///
/// Sockets can have multiple dialers and listeners simultaneously, as NNG supports
/// flexible connection topologies where direction is independent of protocol type.
///
/// # Concurrency and contexts
///
/// While a `Socket` can send and receive messages directly, it **cannot handle
/// concurrent operations safely**. For concurrent request handling or parallel
/// operations, you must create contexts using [`Socket::context()`].
///
/// Each context maintains independent protocol state, allowing multiple concurrent
/// operations on the same underlying socket. This is essential for:
/// - Servers handling multiple requests simultaneously
/// - Clients making parallel requests
/// - Any scenario requiring concurrent socket operations
///
/// # Examples
///
/// ```rust,no_run
/// use anng::protocols::reqrep0;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// // Single-threaded request handling (blocking)
/// let socket = reqrep0::Rep0::listen(c"tcp://localhost:8080").await?;
/// let mut ctx = socket.context();
///
/// // This can only handle one request at a time
/// let (request, responder) = ctx.receive().await?;
///
/// // For concurrent handling, create multiple contexts
/// let socket = Arc::new(reqrep0::Rep0::listen(c"tcp://localhost:8081").await?);
/// for _ in 0..4 {
///     let socket = Arc::clone(&socket);
///     tokio::spawn(async move {
///         let mut ctx = socket.context();
///         loop {
///             let (request, responder) = ctx.receive().await?;
///             // Handle request concurrently
///         }
///         Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
///     });
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Type safety benefits
///
/// It is impossible to call, e.g., `publish` on a REQ0 socket or `reply` on a REP0 socket that has
/// not yet received a request:
///
/// ```compile_fail
/// use anng::{protocols::{reqrep0, pubsub0}, Message};
/// use std::io::Write;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// let req_socket = reqrep0::Req0::dial(c"tcp://localhost:8080").await?;
///
/// let mut msg = Message::with_capacity(100);
/// write!(&mut msg, "news: Breaking news!")?;
/// // This won't compile:
/// req_socket.publish(msg).await.unwrap();
/// # Ok(())
/// # }
/// ```
pub struct Socket<Protocol> {
    socket: nng_sys::nng_socket,
    pub(crate) aio: Aio,
    recovered_msg: Option<Message>,
    protocol: PhantomData<Protocol>,
}

// manual impl to avoid Protocol: Debug bound
impl<Protocol> fmt::Debug for Socket<Protocol> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Socket")
            .field("socket", &self.socket)
            .field("aio", &self.aio)
            .field("recovered_msg", &self.recovered_msg)
            .field("protocol", &self.protocol)
            .finish()
    }
}

/// An async-ready socket with an associated context for concurrent operations.
///
/// `ContextfulSocket` wraps a [`Socket`] with an NNG context, enabling safe concurrent
/// operations on the same underlying socket. Each context maintains independent protocol
/// state, which is crucial for protocols like Request/Reply where the state machine
/// must be maintained per operation.
///
/// # Why contexts are necessary
///
/// NNG sockets themselves are **not safe** for concurrent operations. Attempting
/// concurrent operations directly on a socket can lead to:
/// - Protocol state violations (e.g., receiving before sending in REQ/REP)
/// - Message corruption or loss
/// - Deadlocks or race conditions
///
/// Contexts solve this by providing independent protocol state machines that share
/// the same underlying transport and configuration.
///
/// # Cancellation safety
///
/// All async operations on `ContextfulSocket` are **cancellation safe**. You can safely
/// use this type with `tokio::select!` and other cancellation patterns:
///
/// - **Receive operations**: If cancelled after a message arrives, that message will generally be
///   returned by the next receive call on the same context, though the exact semantics depend on
///   the protocol used. Prefer pinning receives outside of `select!` loops to avoid potential
///   starvation.
/// - **Send operations**: If cancelled, the message may or may not be sent (depending on
///   timing). The message will be dropped if not sent.
/// - **Resource cleanup**: All resources are properly cleaned up on cancellation with no
///   leaks or undefined behavior.
///
/// The library handles all NNG cleanup requirements internally, including calling
/// `nng_aio_cancel` and waiting for operations to fully terminate.
pub struct ContextfulSocket<'socket, Protocol> {
    aio: Aio,
    context: Context<'socket, Protocol>,
}

// manual impl to avoid Protocol: Debug bound
impl<Protocol> fmt::Debug for ContextfulSocket<'_, Protocol> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContextfulSocket")
            .field("aio", &self.aio)
            .field("context", &self.context)
            .finish()
    }
}

impl<Protocol> Drop for Socket<Protocol> {
    fn drop(&mut self) {
        // SAFETY: socket is valid and not already closed (socket is live until `self` drops).
        crate::block_in_place(|| unsafe { nng_sys::nng_socket_close(self.socket) });
    }
}

impl<Protocol> Socket<Protocol> {
    /// Returns the underlying NNG socket handle.
    ///
    /// This is primarily used internally for setting socket options.
    pub(crate) fn id(&self) -> nng_sys::nng_socket {
        self.socket
    }

    /// Sends a message using this socket.
    ///
    /// If this operation fails, the message will be returned along with the error for potential retry.
    /// If this operation succeeds, the message remains under the ownership of NNG, which will eventually free it.
    ///
    /// Note that NNG does _not_ guarantee that the send has actually completed when this operation
    /// returns, and there is no automatic linger or flush to ensure that the socket send buffers
    /// have completely transmitted when a socket is closed. Thus, it is recommended to wait a
    /// brief period after calling send before letting the socket be dropped.
    pub(crate) async fn send_msg(&mut self, message: Message) -> Result<(), (AioError, Message)> {
        self.aio.set_message(message);
        // SAFETY: socket is valid and not closed (socket is live until `self` drops),
        //         AIO is valid and not busy (per `Aio` busy state invariant), and
        //         message has been set on AIO (just above).
        unsafe { nng_sys::nng_socket_send(self.socket, self.aio.as_ptr()) };
        // the above started an async operation (makes AIO busy).
        // we call wait() to preserve the Aio busy invariant.
        match self.aio.wait(ImplicationOnMessage::Sent).await {
            Ok(()) => Ok(()),
            Err(e) => {
                // Return the message that failed to send for potential retry
                let msg = self
                    .aio
                    .take_message()
                    .expect("we set the message, so it should still be there");
                Err((e, msg))
            }
        }
    }

    pub(crate) fn try_recv_msg(&mut self) -> Result<Option<Message>, AioError> {
        // if we previously recovered a message with this same socket,
        // it should be safe to return it here since the same socket implies same use.
        if let Some(msg) = self.recovered_msg.take() {
            tracing::debug!("returning recovered message from cancel");
            return Ok(Some(msg));
        }

        let mut msg = core::ptr::null_mut();

        // SAFETY: socket is valid and not closed (socket is live until `self` drops), and
        //         msg pointer pointer is valid.
        let errno = unsafe {
            nng_sys::nng_recvmsg(self.socket, &mut msg, nng_sys::NNG_FLAG_NONBLOCK as i32)
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {
                let msg = NonNull::new(msg).expect("successful nng_recvmsg initializes pointer");
                // SAFETY: message returned by nng_recvmsg is valid and _ours_.
                let msg = unsafe { Message::from_raw_unchecked(msg) };
                Ok(Some(msg))
            }
            x if x == nng_err::NNG_EAGAIN as u32 => Ok(None),
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("socket is still open since we have a reference to it");
            }
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("flags are valid for the call");
            }
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            x if x == nng_err::NNG_ENOTSUP as u32 => {
                // protocol does not support receiving
                Err(AioError::from_nng_err(nng_err::NNG_ENOTSUP))
            }
            x if x == nng_err::NNG_ESTATE as u32 => {
                // protocol does not support receiving in its current state
                Err(AioError::from_nng_err(nng_err::NNG_ESTATE))
            }
            x if x == nng_err::NNG_ETIMEDOUT as u32 => {
                // likely due to a protocol-level timeout (like surveys)
                Err(AioError::from_nng_err(nng_err::NNG_ETIMEDOUT))
            }
            errno => {
                unreachable!("nng_recvmsg documentation claims errno {errno} is never returned");
            }
        }
    }

    pub(crate) async fn recv_msg(&mut self) -> Result<Message, AioError> {
        // if we previously recovered a message with this same socket,
        // it should be safe to return it here since the same socket implies same use.
        if let Some(msg) = self.recovered_msg.take() {
            tracing::debug!("returning recovered message from cancel");
            return Ok(msg);
        }

        // SAFETY: socket is valid and not closed (socket is live until `self` drops), and
        //         AIO is valid and not busy (per `Aio` busy state invariant).
        unsafe { nng_sys::nng_socket_recv(self.socket, self.aio.as_ptr()) };
        // the above started an async operation (which makes the AIO busy), so we must eventually
        // call wait() later to preserve the Aio busy invariant.
        //
        // but first have to set up a cancellation guard such that if this future is canceled, we
        // we capture a message that was received _just_ too late (but _was_ received) so that we
        // don't drop it. even if dropping is _okay_ in nng, it's not ideal for the application.
        //
        // NOTE(jon): it is vital that there are no early returns or .await calls between this
        // point and the wait() call below, otherwise we'd violate the Aio busy invariant.

        struct CancellationGuard<'a, Protocol>(&'a mut Socket<Protocol>);
        impl<Protocol> Drop for CancellationGuard<'_, Protocol> {
            fn drop(&mut self) {
                // if we got canceled, then _first_ the `.wait()` future is dropped (which will run
                // `aio_stop`), and _then_ this drop code is run. it has to be that way for us to
                // be free to us to get `&mut` since `.wait()` uses the `&mut` to our aio.
                // as a result, _if_ a message was recovered by the AIO stop, then it's recoverable
                // by us here now:
                if let Some(msg) = self.0.aio.take_message() {
                    tracing::debug!("received message recovered on cancel");
                    self.0.recovered_msg = Some(msg);
                } else {
                    tracing::trace!("no received message recovered on cancel");
                }
            }
        }
        let guard = CancellationGuard(self);

        // this completes the AIO busy contract started above.
        let result = guard.0.aio.wait(ImplicationOnMessage::Received).await;

        // the future has completed, so no need for the teardown mechanism.
        core::mem::forget(guard);

        result?;

        let msg = self
            .aio
            .take_message()
            .expect("we executed a successful receive, so there should be a message");
        Ok(msg)
    }

    /// Adds a dialer to this socket that connects to the specified URL.
    ///
    /// This allows the socket to initiate connections to remote endpoints.
    /// Multiple dialers can be added to the same socket for load balancing
    /// or failover scenarios.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL format is invalid
    /// - The address is unreachable
    /// - Network interface issues occur
    /// - Out of memory conditions
    pub async fn dial<'socket>(
        &'socket self,
        url: impl AsRef<CStr>,
    ) -> io::Result<pipes::Dialer<'socket, Protocol>> {
        self.dial_with_opts(url, &Default::default()).await
    }

    /// Adds a dialer to this socket that connects to the specified URL with custom options.
    ///
    /// This allows the socket to initiate connections to remote endpoints with
    /// specific pipe configuration options. Multiple dialers can be added to the
    /// same socket for load balancing or failover scenarios.
    ///
    /// For most use cases, prefer [`Socket::dial`] which uses default options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL format is invalid
    /// - The address is unreachable
    /// - Network interface issues occur
    /// - Out of memory conditions
    pub async fn dial_with_opts<'socket>(
        &'socket self,
        url: impl AsRef<CStr>,
        options: &pipes::PipeOptions,
    ) -> io::Result<pipes::Dialer<'socket, Protocol>> {
        let dialer = crate::protocols::add_dialer_to_socket(self.socket, url.as_ref(), |_dialer| {
            let pipes::PipeOptions {} = options;
            Ok(())
        })
        .await?;
        Ok(pipes::Dialer {
            socket: self,
            dialer,
        })
    }

    /// Adds a listener to this socket that accepts connections on the specified URL.
    ///
    /// This allows the socket to accept incoming connections from remote endpoints.
    /// Multiple listeners can be added to the same socket to listen on different
    /// addresses or interfaces.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL format is invalid
    /// - The address is already in use
    /// - Permission denied for the requested address
    /// - Out of memory conditions
    pub async fn listen<'socket>(
        &'socket self,
        url: impl AsRef<CStr>,
    ) -> io::Result<pipes::Listener<'socket, Protocol>> {
        self.listen_with_opts(url, &Default::default()).await
    }

    /// Adds a listener to this socket that accepts connections on the specified URL with custom options.
    ///
    /// This allows the socket to accept incoming connections from remote endpoints
    /// with specific pipe configuration options. Multiple listeners can be added to
    /// the same socket to listen on different addresses or interfaces.
    ///
    /// For most use cases, prefer [`Socket::listen`] which uses default options.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The URL format is invalid
    /// - The address is already in use
    /// - Permission denied for the requested address
    /// - Out of memory conditions
    pub async fn listen_with_opts<'socket>(
        &'socket self,
        url: impl AsRef<CStr>,
        options: &pipes::PipeOptions,
    ) -> io::Result<pipes::Listener<'socket, Protocol>> {
        let listener =
            crate::protocols::add_listener_to_socket(self.socket, url.as_ref(), |_listener| {
                let pipes::PipeOptions {} = options;
                Ok(())
            })
            .await?;
        Ok(pipes::Listener {
            socket: self,
            listener,
        })
    }
}

impl<'socket, Protocol> ContextfulSocket<'socket, Protocol> {
    /// Returns a reference to the shared socket underpinning this context.
    ///
    /// Note that any operation performed on this socket will affect _all_ associated contexts!
    pub fn socket(&self) -> &'socket Socket<Protocol> {
        self.context.socket
    }
}

fn nng_strerror(err: nng_sys::nng_err) -> &'static CStr {
    // SAFETY: nng_strerror has no additional safety requirements.
    let raw = unsafe { nng_sys::nng_strerror(err) };
    // SAFETY: nng_strerror returns a valid null-terminated string.
    //         no allocation information is provided,
    //         which implies that this is a static string reference.
    #[allow(clippy::let_and_return)]
    let cstr = unsafe { CStr::from_ptr(raw) };
    cstr
}

fn nng_err_to_string(err: nng_sys::nng_err) -> String {
    nng_strerror(err).to_string_lossy().into_owned()
}

/// Helper function that calls `tokio::task::block_in_place` when appropriate.
///
/// This function checks if there's a tokio runtime and whether it is multi-threaded,
/// and only calls `block_in_place` in that case. Otherwise, it calls the function directly.
/// When the `tokio` feature is disabled, it always calls the function directly.
pub(crate) fn block_in_place<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    #[cfg(feature = "tokio")]
    {
        use tokio::runtime::RuntimeFlavor;
        #[allow(clippy::collapsible_if)]
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            if let RuntimeFlavor::MultiThread = rt.runtime_flavor() {
                return tokio::task::block_in_place(f);
            }
        }
    }
    f()
}
