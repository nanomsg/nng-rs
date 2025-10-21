//! Pair pattern (PAIR1 protocol).
//!
//! This module implements the Pair messaging pattern, which provides one-to-one
//! communication between exactly two endpoints. In a pair topology, only two
//! nodes can be connected at any time, forming a simple bidirectional channel.
//!
//! # Socket types
//!
//! - [`Pair1`] - One-to-one communication endpoint
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
//! use anng::{protocols::pair1, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Set up both endpoints first
//! let mut endpoint1 = pair1::Pair1::listen(c"inproc://pair-example").await?;
//! let mut endpoint2 = pair1::Pair1::dial(c"inproc://pair-example").await?;
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
//! use anng::{protocols::pair1, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Create a pair connection
//! let mut server = pair1::Pair1::listen(c"inproc://pair-bidirectional-demo").await?;
//! let mut client = pair1::Pair1::dial(c"inproc://pair-bidirectional-demo").await?;
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
/// This implements the PAIR1 protocol which includes TTL/hop count support
/// for preventing loops in device topologies.
///
/// # Connection patterns
///
/// - **Point-to-point**: One endpoint listens, the other dials (`Pair1::listen()` + `Pair1::dial()`)
/// - **Exactly two endpoints**: Additional connection attempts may succeed but any new pipes are immediately dropped, leaving the original connection active
#[derive(Debug, Clone, Copy)]
pub struct Pair1;

impl Pair1 {
    /// Creates a new PAIR1 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Pair1::dial`] or [`Pair1::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Pair1>> {
        // SAFETY: nng_pair1_open is the correct initialization function for Pair1 protocol.
        unsafe { super::create_socket(nng_sys::nng_pair1_open, Pair1) }
    }

    /// Creates a PAIR1 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Pair1::socket`] and [`Socket::dial`].
    /// For advanced configuration, use [`Pair1::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Pair1>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a PAIR1 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Pair1::socket`] and [`Socket::listen`].
    /// For advanced configuration, use [`Pair1::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Pair1>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Pair1> {
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

    /// Sets the maximum TTL (time-to-live) hop count for messages.
    ///
    /// The TTL controls how many times a message can be forwarded through
    /// [`nng_device`](https://nng.nanomsg.org/man/tip/nng_device.3.html) forwarders
    /// before being dropped. This prevents infinite forwarding loops in complex topologies.
    ///
    /// **Note**: This setting only affects **incoming** messages during receive operations.
    /// It does not limit outgoing messages sent by this socket.
    ///
    /// # Parameters
    ///
    /// - `ttl`: Maximum hop count (1-15). Default is 8.
    ///
    /// # Panics
    ///
    /// Panics if `ttl` is not in the range 1-15.
    ///
    /// # How TTL works
    ///
    /// 1. When a message is first sent, its hop count starts at 1
    /// 2. Each time the message is forwarded through a device, the hop count increments
    /// 3. If the hop count exceeds the MAXTTL value, the message is silently dropped during receive
    /// 4. This prevents messages from looping infinitely in device topologies
    ///
    /// # When to adjust TTL
    ///
    /// - **Increase TTL**: In complex topologies with many device forwarders
    /// - **Decrease TTL**: To detect routing loops faster or save resources
    /// - **Consistency**: All nodes in a topology should use the same MAXTTL for predictable behavior
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::protocols::pair1;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// let socket = pair1::Pair1::socket()?;
    ///
    /// // Set TTL to 5 for a simple topology
    /// socket.set_max_ttl(5);
    ///
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_max_ttl(&self, ttl: u8) {
        assert!(
            (1..=15).contains(&ttl),
            "TTL must be between 1 and 15, got {}",
            ttl
        );

        let errno = unsafe {
            nng_sys::nng_socket_set_int(
                self.socket,
                c"ttl-max".as_ptr() as *const c_char,
                i32::from(ttl),
            )
        };

        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            nng_sys::NNG_ECLOSED => {
                unreachable!("socket is still open");
            }
            nng_sys::NNG_EINVAL => {
                unreachable!("we've checked the range of the input");
            }
            errno => {
                unreachable!(
                    "nng_socket_set_int documentation claims errno {errno} is never returned"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pair_socket_creation() {
        let socket = Pair1::socket();
        assert!(socket.is_ok());
    }

    #[test]
    fn test_max_ttl_operations() {
        let socket = Pair1::socket().unwrap();

        // Test setting valid TTL values
        socket.set_max_ttl(5);
        socket.set_max_ttl(1); // minimum valid value
        socket.set_max_ttl(15); // maximum valid value (NNG limit)
    }

    #[test]
    #[should_panic(expected = "TTL must be between 1 and 15, got 0")]
    fn test_max_ttl_invalid_zero() {
        let socket = Pair1::socket().unwrap();
        socket.set_max_ttl(0); // Should panic
    }

    #[test]
    #[should_panic(expected = "TTL must be between 1 and 15, got 16")]
    fn test_max_ttl_invalid_too_high() {
        let socket = Pair1::socket().unwrap();
        socket.set_max_ttl(16); // Should panic
    }
}
