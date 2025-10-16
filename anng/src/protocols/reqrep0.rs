//! Request/Reply pattern (REQ0/REP0 protocol).
//!
//! This module implements the Request/Reply messaging pattern, which provides
//! reliable, synchronous request-response communication between clients and servers.
//!
//! # Socket Types
//!
//! - [`Req0`] - Clients that send requests and await replies
//! - [`Rep0`] - Servers that receive requests and send replies
//!
//! # Protocol Overview
//!
//! The Request/Reply pattern enforces a strict state machine where request sockets
//! must send before receiving, and reply sockets must receive before sending.
//! Protocol violations are prevented at compile time by the type-safe bindings.
//!
//! # Key Features
//!
//! - **Automatic load balancing**: Requests distribute across available servers
//! - **Resilience**: Automatic retry and reconnection on failure
//!
//! # Examples
//!
//! ## Simple client-server
//!
//! ```rust
//! use anng::{protocols::reqrep0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Server, in one task
//! # tokio::spawn(async {
//! let socket = reqrep0::Rep0::listen(c"inproc://demo").await?;
//! let mut ctx = socket.context();
//!
//! let (request, responder) = ctx.receive().await?;
//! println!("Got request: {:?}", request.as_slice());
//!
//! let mut reply = Message::with_capacity(100);
//! write!(&mut reply, "Hello back!")?;
//! // TODO: In production, handle error and retry with returned responder and message
//! responder.reply(reply).await.unwrap();
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//!
//! // Client, in another (concurrent) task
//! let socket = reqrep0::Req0::dial(c"inproc://demo").await?;
//! let mut ctx = socket.context();
//!
//! let mut request = Message::with_capacity(100);
//! write!(&mut request, "Hello server!")?;
//!
//! // TODO: In production, handle error and retry with returned message
//! let reply_future = ctx.request(request).await.unwrap();
//! let reply = reply_future.await?;
//! println!("Got reply: {:?}", reply.as_slice());
//! # Ok(())
//! # }
//! ```
//!
//! ## Concurrent server
//!
//! ```rust
//! use anng::{protocols::reqrep0, Message};
//! use std::{io::Write, sync::Arc};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! let socket = Arc::new(reqrep0::Rep0::listen(c"inproc://reqrep-concurrent-demo").await?);
//!
//! // Spawn multiple workers for concurrent handling
//! for worker_id in 0..4 {
//!     let socket = Arc::clone(&socket);
//!     tokio::spawn(async move {
//!         let mut ctx = socket.context();
//!         loop {
//!             let (request, responder) = ctx.receive().await?;
//!             println!("Worker {} handling request", worker_id);
//!
//!             // Process request...
//!             let mut reply = Message::with_capacity(100);
//!             write!(&mut reply, "Processed by worker {}", worker_id)?;
//!             responder.reply(reply).await.map_err(|(_, err, _)| err)?;
//!         }
//!         Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//!     });
//! }
//! # Ok(())
//! # }
//! ```

use super::SupportsContext;
use crate::{ContextfulSocket, Socket, aio::AioError, message::Message};
use core::ffi::CStr;
use std::io;

/// Request socket type for the client side of Request/Reply communication.
///
/// REQ sockets send requests and wait for replies. The protocol enforces that each
/// request must receive exactly one reply before the next request can be sent.
/// This ordering is maintained automatically by the NNG library and enforced at
/// compile time by the type system.
///
/// # Connection patterns
///
/// - **Typical**: Dial to connect to reply servers (`Req0::dial()`)
/// - **Reverse**: Listen for connections from reply servers (`Req0::listen()`)
/// - **Multi-connection**: Connect to multiple servers for round-robin request distribution
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::reqrep0::{Req0, Rep0};
/// # use std::io::Write;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # tokio::spawn(async {
/// #     let socket = Rep0::listen(c"inproc://req0-usage-doctest").await?;
/// #     let mut ctx = socket.context();
/// #     loop {
/// #         let (request, responder) = ctx.receive().await?;
/// #         let reply = anng::Message::from(&b"Reply from server"[..]);
/// #         responder.reply(reply).await.unwrap();
/// #     }
/// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
/// # });
/// // Connect to a server
/// let socket = Req0::dial(c"inproc://req0-usage-doctest").await?;
/// let mut ctx = socket.context();
///
/// // Send request and await reply
/// let request = anng::Message::from(&b"Hello"[..]);
/// // TODO: In production, handle error and retry with returned message
/// let reply_future = ctx.request(request).await.unwrap();
/// let reply = reply_future.await?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Req0;

impl SupportsContext for Req0 {}

impl Req0 {
    /// Creates a new REQ0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Req0::dial`] or [`Req0::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Req0>> {
        // SAFETY: nng_req0_open is the correct initialization function for Req0 protocol.
        unsafe { super::create_socket(nng_sys::nng_req0_open, Req0) }
    }

    /// Creates a REQ0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Req0::socket`] and [`Socket::dial`].
    /// For multiple connections or advanced configuration, use [`Req0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Req0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a REQ0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Req0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Req0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Req0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl<'socket> ContextfulSocket<'socket, Req0> {
    /// Sends a request and returns a future that will resolve to the reply.
    ///
    /// This method implements the client side of the Request/Reply pattern.
    /// It sends the provided message as a request and returns a future that
    /// will resolve to the reply message when it arrives.
    ///
    /// # Two-Phase Operation
    ///
    /// The request operation is split into two phases:
    /// 1. **Send phase**: Returns immediately with a future if the send succeeds
    /// 2. **Reply phase**: The returned future awaits the reply
    ///
    /// This design allows you to handle send failures immediately while still
    /// benefiting from async reply handling.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If the send phase is cancelled, the message
    /// may or may not be sent (depending on when cancellation occurs relative to NNG's
    /// internal dispatch). The message will be dropped if not sent. If the reply future
    /// is cancelled, any incoming reply will be discarded, and a new request can be sent
    /// immediately.
    ///
    /// # Protocol state
    ///
    /// Each context maintains independent protocol state. You can have multiple
    /// outstanding requests across different contexts, but each individual context
    /// must complete its request-reply cycle before starting a new one.
    /// This is enforced at compile-time.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the send operation fails, giving you back
    /// the message for potential retry. The reply future can fail with
    /// network errors, timeouts, or cancellation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::reqrep0, Message, AioError};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = reqrep0::Rep0::listen(c"inproc://request-doctest").await?;
    /// #     let mut ctx = socket.context();
    /// #     loop {
    /// #         let (request, responder) = ctx.receive().await?;
    /// #         let mut reply = Message::with_capacity(100);
    /// #         write!(&mut reply, "Hello back!")?;
    /// #         responder.reply(reply).await.unwrap();
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// let socket = reqrep0::Req0::dial(c"inproc://request-doctest").await?;
    /// let mut ctx = socket.context();
    ///
    /// let mut request = Message::with_capacity(100);
    /// write!(&mut request, "Hello")?;
    ///
    /// match ctx.request(request).await {
    ///     Ok(reply_future) => {
    ///         match reply_future.await {
    ///             Ok(reply) => println!("Got reply: {:?}", reply.as_slice()),
    ///             Err(AioError::TimedOut) => println!("Request timed out"),
    ///             Err(e) => println!("Request failed: {:?}", e),
    ///         }
    ///     }
    ///     Err((error, msg)) => {
    ///         println!("Send failed: {:?}", error);
    ///         // Could retry with `msg`
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn request<'s>(
        &'s mut self,
        message: Message,
    ) -> Result<
        impl Future<Output = Result<Message, AioError>> + use<'s, 'socket>,
        (AioError, Message),
    > {
        tracing::trace!("send request");
        // NOTE(jon): it's fine if send_msg is cancelled, but eventually succeeds; the context will
        // implicitly cancel the previous request as a result and discard replies to that old
        // request.
        if let Err((e, msg)) = self.context.send_msg(message, &mut self.aio).await {
            tracing::error!(?e, "request failed");
            Err((e, msg))
        } else {
            tracing::debug!("request succeeded; ready to await reply");
            // NOTE(jon): here, too, it's fine if the user drops this reply future, because a new
            // request will reset the context and cancel the old request (plus drop its responses).
            Ok(async move {
                tracing::trace!("awaiting reply");
                let r = self.context.recv_msg(&mut self.aio).await;
                tracing::debug!("reply arrived");
                r
            })
        }
    }
}

/// Reply socket type for the server side of Request/Reply communication.
///
/// REP sockets receive requests and send back replies. The protocol enforces that each
/// received request must be replied to exactly once before the next request can be
/// received. This ordering is maintained automatically by the NNG library.
///
/// # Connection patterns
///
/// - **Typical**: Listen to accept connections from request clients (`Rep0::listen()`)
/// - **Reverse**: Dial to connect to request endpoints (`Rep0::dial()`)
///
/// # Concurrency
///
/// For concurrent request handling, use multiple contexts on the same socket.
/// Each context maintains independent protocol state, allowing parallel processing
/// of different requests.
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::reqrep0::{Rep0, Req0};
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # tokio::spawn(async {
/// #     let socket = Req0::dial(c"inproc://rep0-usage-demo").await?;
/// #     let mut ctx = socket.context();
/// #     let request = anng::Message::from(&b"Hello server"[..]);
/// #     let reply_future = ctx.request(request).await.unwrap();
/// #     let _reply = reply_future.await?;
/// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
/// # });
/// // Listen for client connections
/// let socket = Rep0::listen(c"inproc://rep0-usage-demo").await?;
/// let mut ctx = socket.context();
///
/// // Handle request-reply cycle
/// let (request, responder) = ctx.receive().await?;
/// let reply = anng::Message::from(&b"Hello back"[..]);
/// // TODO: In production, handle error and retry with returned responder and message
/// responder.reply(reply).await.unwrap();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Rep0;

impl SupportsContext for Rep0 {}

impl Rep0 {
    /// Creates a new REP0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Rep0::listen`] or [`Rep0::dial`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Rep0>> {
        // SAFETY: nng_rep0_open is the correct initialization function for Rep0 protocol.
        unsafe { super::create_socket(nng_sys::nng_rep0_open, Rep0) }
    }

    /// Creates a REP0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Rep0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Rep0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Rep0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a REP0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Rep0::socket`] and [`Socket::dial`].
    /// For connecting to multiple addresses or advanced configuration, use [`Rep0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Rep0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }
}

impl<'socket> ContextfulSocket<'socket, Rep0> {
    /// Receives a request and returns the message along with a means to reply.
    ///
    /// This method implements the server side of the Request/Reply pattern.
    /// It waits for an incoming request and returns both the request message
    /// and a [`Responder`] that must be used to send the reply.
    ///
    /// # Responder Pattern
    ///
    /// The returned [`Responder`] ensures that:
    /// - Each request gets exactly one reply
    /// - Protocol state is maintained correctly
    /// - Resources are cleaned up if the responder is dropped
    ///
    /// You **must** call [`Responder::reply`] to complete the request-reply cycle.
    /// Dropping the responder without replying will lead to the client re-sending
    /// their request.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled after a request message has
    /// been received, that message will be returned by the next call to `receive()` on
    /// the same context. No request messages are lost due to cancellation. This ensures
    /// that clients don't need to resend requests unnecessarily.
    ///
    /// # Protocol state
    ///
    /// Each context maintains independent protocol state. After receiving a
    /// request, that context cannot receive another request until it has
    /// sent a reply using the [`Responder`].
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::reqrep0, Message};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// let socket = reqrep0::Rep0::listen(c"inproc://receive-doctest-echo").await?;
    /// let mut ctx = socket.context();
    ///
    /// # tokio::spawn(async {
    /// #     let socket = reqrep0::Req0::dial(c"inproc://receive-doctest-echo").await?;
    /// #     let mut ctx = socket.context();
    /// #     let mut request = Message::with_capacity(100);
    /// #     write!(&mut request, "Test message")?;
    /// #     let reply_future = ctx.request(request).await.unwrap();
    /// #     let _reply = reply_future.await?;
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// loop {
    ///     let (request, responder) = ctx.receive().await?;
    ///
    ///     // Echo the request back as a reply
    ///     let mut reply = Message::with_capacity(request.len());
    ///     reply.write(request.as_slice())?;
    ///
    ///     // TODO: In production, handle error and retry with returned responder and message
    ///     responder.reply(reply).await.unwrap();
    /// #     break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn receive<'s>(&'s mut self) -> Result<(Message, Responder<'s, 'socket>), AioError> {
        // NOTE(jon): if `recv_msg` succeeds after being dropped, it saves its Message so that the
        // next call to `receive` will return the same message (and thus we won't get `NNG_ESTATE`
        // from trying to receive on a Rep0 socket that is expected to send next).
        let message = self.context.recv_msg(&mut self.aio).await?;
        // NOTE: Dropping the `Responder` without calling `reply()` means we'll call recv on a rep
        // protocol socket twice in a row. Upstream has confirmed that this is acceptable behavior,
        // and is effectively equivalent to a dropped reply (ie, the requester will retry).
        let responder = Responder {
            contextful: self,
            sent: false,
        };
        Ok((message, responder))
    }

    /// Non-blocking variant of [`receive`](Self::receive).
    ///
    /// Returns `Ok(Some((message, responder)))` if immediately available, `Ok(None)` if no request is available.
    pub fn try_receive<'s>(
        &'s mut self,
    ) -> Result<Option<(Message, Responder<'s, 'socket>)>, AioError> {
        match self.context.try_recv_msg()? {
            Some(message) => {
                let responder = Responder {
                    contextful: self,
                    sent: false,
                };
                Ok(Some((message, responder)))
            }
            None => Ok(None),
        }
    }
}

/// A handle for sending a reply to a specific request.
///
/// `Responder` ensures that each request gets exactly one reply and maintains
/// proper protocol state. It must be used to complete the request-reply cycle
/// after receiving a request via [`ContextfulSocket::receive`].
///
/// # Must use
///
/// This type is marked `#[must_use]` because dropping it without calling [`Responder::reply`]
/// means the client will have to retry the request.
///
/// If a `Responder` is dropped without sending a reply (e.g., due to an error or cancellation), a
/// warning is logged to indicate that the peer will need to re-send their request.
///
/// # Examples
///
/// ```rust
/// use anng::{protocols::reqrep0, Message};
/// use std::io::Write;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # tokio::spawn(async {
/// #     let socket = reqrep0::Req0::dial(c"inproc://service").await?;
/// #     let mut ctx = socket.context();
/// #     let request = Message::from(&b"Test request"[..]);
/// #     let reply_future = ctx.request(request).await.unwrap();
/// #     let _reply = reply_future.await?;
/// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
/// # });
/// let socket = reqrep0::Rep0::listen(c"inproc://service").await?;
/// let mut ctx = socket.context();
///
/// let (request, responder) = ctx.receive().await?;
///
/// // Process the request...
/// let mut reply = Message::with_capacity(100);
/// write!(&mut reply, "Processed: {:?}", request.as_slice())?;
///
/// // Send the reply
/// // TODO: In production, handle error and retry with returned responder and message
/// responder.reply(reply).await.unwrap();
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[must_use]
pub struct Responder<'s, 'socket> {
    contextful: &'s mut ContextfulSocket<'socket, Rep0>,
    sent: bool,
}

impl Drop for Responder<'_, '_> {
    fn drop(&mut self) {
        if !self.sent {
            tracing::warn!(
                "REP0 Responder dropped (likely cancelled); \
            peer will have to re-send the request"
            );
        }
    }
}

impl Responder<'_, '_> {
    /// Sends a reply message to complete the request-reply cycle.
    ///
    /// This method consumes the `Responder` and sends the provided message
    /// as a reply to the original request. This completes the protocol
    /// state machine and allows the context to receive new requests.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the reply message may or may
    /// not be sent (depending on when cancellation occurs). The message and responder will
    /// be dropped if not sent. Note that if the reply is not sent, the client will
    /// typically retry their request after a timeout, resulting in a new request to
    /// `receive()`.
    ///
    /// # Errors
    ///
    /// Returns `Err((responder, error, message))` if the send operation fails.
    /// This gives you back all the resources so you can handle the error
    /// appropriately (retry, log, etc.).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::reqrep0, Message, AioError};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = reqrep0::Req0::dial(c"inproc://replies").await?;
    /// #     let mut ctx = socket.context();
    /// #     let request = Message::from(&b"Test request"[..]);
    /// #     let reply_future = ctx.request(request).await.unwrap();
    /// #     let _reply = reply_future.await?;
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// let socket = reqrep0::Rep0::listen(c"inproc://replies").await?;
    /// let mut ctx = socket.context();
    ///
    /// let (request, responder) = ctx.receive().await?;
    ///
    /// let mut reply = Message::with_capacity(100);
    /// write!(&mut reply, "Hello back!")?;
    ///
    /// match responder.reply(reply).await {
    ///     Ok(()) => println!("Reply sent successfully"),
    ///     Err((responder, error, message)) => {
    ///         println!("Failed to send reply: {:?}", error);
    ///         // Could retry with `responder.reply(message)`
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reply(mut self, message: Message) -> Result<(), (Self, AioError, Message)> {
        // avoid the drop warning
        self.sent = true;

        if let Err((e, msg)) = self
            .contextful
            .context
            .send_msg(message, &mut self.contextful.aio)
            .await
        {
            Err((self, e, msg))
        } else {
            Ok(())
        }
    }
}
