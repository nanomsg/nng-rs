//! Pair pattern (pair0 protocol).
//!
//! This module implements the Pair messaging pattern, which provides one-to-one
//! communication between exactly two endpoints. In a pair topology, only two
//! nodes can be connected at any time, forming a simple bidirectional channel.
//!
//! # Socket types
//!
//! - [`Pair0`] - One-to-one communication endpoint
//!
//! # Protocol overview
//!
//! The pair pattern creates a simple 1-to-1 connection where exactly two endpoints
//! can communicate bidirectionally. Unlike other patterns with multiple participants,
//! the pair pattern is limited to exactly two connected peers.
//!
//! # Use cases
//!
//! - **Point-to-point communication**: Direct communication between two specific processes
//! - **IPC channels**: Simple inter-process communication channels
//! - **Proxy connections**: Building blocks for creating proxy or gateway services
//! - **Simple request forwarding**: When you need to forward messages between exactly two endpoints
//!
//! # Reliability considerations
//!
//! **Important**: The pair protocol is **unreliable** and provides **no delivery guarantees**.
//! Messages may be lost under various conditions, and there is no capability for message
//! acknowledgment or delivery confirmation. The protocol honors flow control but cannot
//! guarantee message delivery.
//!
//! Applications that require reliable delivery should consider using the [`REQ/REP protocol`](crate::protocols::reqrep0)
//! for reliable point-to-point communication with acknowledgments.
//!
//! # Connection constraints
//!
//! **Important**: Only **exactly two** endpoints can be connected in a pair socket.
//! Additional dial attempts may succeed at the transport level, but the pair protocol
//! will immediately drop any new pipes, leaving the original connection intact and functional.
//!
//! # Examples
//!
//! ## Simple pair communication
//!
//! ```rust
//! use anng::{protocols::pair0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Set up both endpoints first
//! let mut endpoint1 = pair0::Pair0::listen(c"inproc://pair-example").await?;
//! let mut endpoint2 = pair0::Pair0::dial(c"inproc://pair-example").await?;
//!
//! // Now we can communicate bidirectionally
//! // Endpoint 1 sends first
//! let mut message = Message::with_capacity(100);
//! write!(&mut message, "Hello from endpoint 1")?;
//! // TODO: In production, handle error and retry with returned message
//! endpoint1.send(message).await.unwrap();
//!
//! // Endpoint 2 receives and responds
//! let received = endpoint2.receive().await?;
//! println!("Endpoint 2 received: {:?}", std::str::from_utf8(received.as_slice())?);
//!
//! let mut response = Message::with_capacity(100);
//! write!(&mut response, "Hello from endpoint 2")?;
//! // TODO: In production, handle error and retry with returned message
//! endpoint2.send(response).await.unwrap();
//!
//! // Endpoint 1 receives the response
//! let response = endpoint1.receive().await?;
//! println!("Endpoint 1 received: {:?}", std::str::from_utf8(response.as_slice())?);
//! # Ok(())
//! # }
//! ```
//!
//! ## Bidirectional communication
//!
//! ```rust
//! use anng::{protocols::pair0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Create a pair connection
//! let mut server = pair0::Pair0::listen(c"inproc://pair-bidirectional-demo").await?;
//! let mut client = pair0::Pair0::dial(c"inproc://pair-bidirectional-demo").await?;
//!
//! // Client sends request
//! let mut request = Message::with_capacity(50);
//! write!(&mut request, "What's the time?")?;
//! // TODO: In production, handle error and retry with returned message
//! client.send(request).await.unwrap();
//!
//! // Server receives and responds
//! let received = server.receive().await?;
//! println!("Server received: {:?}", std::str::from_utf8(received.as_slice())?);
//!
//! let mut response = Message::with_capacity(50);
//! write!(&mut response, "Current time: 14:30:00")?;
//! // TODO: In production, handle error and retry with returned message
//! server.send(response).await.unwrap();
//!
//! // Client receives response
//! let time_response = client.receive().await?;
//! println!("Client received: {:?}", std::str::from_utf8(time_response.as_slice())?);
//! # Ok(())
//! # }
//! ```

use crate::{Socket, aio::AioError, message::Message};
use core::ffi::{CStr, c_char};
use std::io;

/// Pair socket type for one-to-one communication.
///
/// PAIR sockets provide simple bidirectional communication between exactly
/// two endpoints. Unlike other protocols that support multiple connections,
/// pair sockets are limited to exactly two connected peers.
///
/// This implements the pair0 protocol which includes TTL/hop count support
/// for preventing loops in device topologies.
///
/// # Connection patterns
///
/// - **Point-to-point**: One endpoint listens, the other dials (`Pair0::listen()` + `pair0::dial()`)
/// - **Exactly two endpoints**: Additional connection attempts may succeed but any new pipes are immediately dropped, leaving the original connection active
#[derive(Debug, Clone, Copy)]
pub struct Pair0;

impl Pair0 {
    /// Creates a new pair0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Pair0::dial`] or [`pair0::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Pair0>> {
        // SAFETY: nng_pair0_open is the correct initialization function for Pair0 protocol.
        unsafe { super::create_socket(nng_sys::nng_pair0_open, Pair0) }
    }

    /// Creates a pair0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Pair0::socket`] and [`Socket::dial`].
    /// For advanced configuration, use [`Pair0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Pair0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a pair0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Pair0::socket`] and [`Socket::listen`].
    /// For advanced configuration, use [`Pair0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Pair0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Pair0> {
    /// Sends a message to the connected pair peer.
    ///
    /// Sends the message to the single connected peer. Since pair sockets
    /// support exactly one connection, the message is delivered to that
    /// specific peer endpoint.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the message may or may not
    /// be sent to the peer (depending on when cancellation occurs). The message will be
    /// dropped if not sent. Note that the pair protocol provides no delivery
    /// guarantees even without cancellation.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the operation fails, giving you back
    /// the message for potential retry.
    pub async fn send(&mut self, message: Message) -> Result<(), (AioError, Message)> {
        self.send_msg(message).await
    }

    /// Receives the next message from the connected pair peer.
    ///
    /// Waits for the next message from the connected peer endpoint.
    /// Since pair sockets have exactly one peer, all received messages
    /// come from that single connected endpoint.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled after a message is received,
    /// that message will be returned by the next call to `receive()` on the same socket.
    /// No messages are lost due to cancellation.
    pub async fn receive(&mut self) -> Result<Message, AioError> {
        self.recv_msg().await
    }

    /// Non-blocking variant of [`receive`](Self::receive).
    ///
    /// Returns `Ok(Some(message))` if immediately available, `Ok(None)` if no message is available.
    pub fn try_receive(&mut self) -> Result<Option<Message>, AioError> {
        self.try_recv_msg()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pair_socket_creation() {
        let socket = Pair0::socket();
        assert!(socket.is_ok());
    }
}
