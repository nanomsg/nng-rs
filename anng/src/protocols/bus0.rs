//! Bus pattern (BUS0 protocol).
//!
//! This module implements the Bus messaging pattern, which provides many-to-many
//! communication where every participant can both send and receive messages.
//! In a bus topology, all connected nodes can communicate with all other nodes.
//!
//! # Socket Types
//!
//! - [`Bus0`] - Many-to-many communication node
//!
//! # Protocol Overview
//!
//! The bus pattern creates a mesh topology where every node can send messages
//! to all other connected nodes. Unlike pub/sub where there's a clear publisher/subscriber
//! relationship, or req/rep where there's a client/server relationship, the bus pattern
//! treats all participants as equals.
//!
//! # Use Cases
//!
//! - **Distributed coordination**: Nodes announcing state changes to all peers
//! - **Chat applications**: Where every participant can send messages to all others
//! - **Peer discovery**: Nodes broadcasting their presence and capabilities
//! - **Event broadcasting**: Where multiple producers need to notify multiple consumers
//!
//! # Reliability Considerations
//!
//! **Important**: The bus protocol is **unreliable** and provides **no delivery guarantees**.
//! Messages may be lost under various conditions, and there is no capability for message
//! acknowledgment or delivery confirmation. The protocol honors flow control but cannot
//! guarantee message delivery.
//!
//! Applications that require reliable delivery should consider using the [`REQ/REP protocol`](crate::protocols::reqrep0)
//! for point-to-point reliable communication.
//!
//! # Examples
//!
//! ## Simple bus communication
//!
//! ```rust
//! use anng::{protocols::bus0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Node 1 (listener)
//! # tokio::spawn(async {
//! let mut node1 = bus0::Bus0::listen(c"inproc://bus-example").await?;
//!
//! // Send a message to all connected peers
//! let mut announcement = Message::with_capacity(100);
//! write!(&mut announcement, "Node 1 is online")?;
//! // TODO: In production, handle error and retry with returned message
//! node1.send(announcement).await.unwrap();
//!
//! // Receive messages from other nodes
//! let msg = node1.receive().await?;
//! println!("Node 1 received: {:?}", std::str::from_utf8(msg.as_slice())?);
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//!
//! // Node 2 (dialer)
//! let mut node2 = bus0::Bus0::dial(c"inproc://bus-example").await?;
//!
//! let mut greeting = Message::with_capacity(100);
//! write!(&mut greeting, "Hello from Node 2")?;
//! // TODO: In production, handle error and retry with returned message
//! node2.send(greeting).await.unwrap();
//! # Ok(())
//! # }
//! ```
//!
//! ## Multi-node bus network
//!
//! ```rust
//! use anng::{protocols::bus0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Central hub
//! let mut hub = bus0::Bus0::listen(c"inproc://bus-multinode-demo").await?;
//!
//! // Multiple worker nodes connecting to the hub
//! for node_id in 0..2 {
//!     tokio::spawn(async move {
//!         let mut node = bus0::Bus0::dial(c"inproc://bus-multinode-demo").await?;
//!
//!         // Send periodic announcements
//!         let mut msg = Message::with_capacity(50);
//!         write!(&mut msg, "Node {} status update", node_id)?;
//!         // TODO: In production, handle error and retry with returned message
//!         node.send(msg).await.unwrap();
//!
//!         // Listen for messages from other nodes
//!         loop {
//!             let received = node.receive().await?;
//!             println!("Node {} received: {:?}", node_id, received.as_slice());
//! #           break; // Exit after receiving one message
//!         }
//!         Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//!     });
//! }
//!
//! // Hub can also participate in communication
//! loop {
//!     let msg = hub.receive().await?;
//!     println!("Hub received: {:?}", msg.as_slice());
//! #   break; // Exit after receiving one message
//! }
//! # Ok(())
//! # }
//! ```

use crate::{Socket, aio::AioError, message::Message};
use core::ffi::CStr;
use std::io;

/// Bus socket type for many-to-many communication.
///
/// BUS sockets provide mesh-style communication where every node can send
/// messages to all other connected nodes. Unlike other protocols with distinct
/// sender/receiver roles, all bus participants are peers.
///
/// # Connection patterns
///
/// - **Hub pattern**: One node listens, others dial to it (`Bus0::listen()` + multiple `Bus0::dial()`)
/// - **Mesh pattern**: Nodes connect to each other in various configurations
/// - **Mixed pattern**: Some nodes listen while others dial for flexible topologies
#[derive(Debug, Clone, Copy)]
pub struct Bus0;

impl Bus0 {
    /// Creates a new BUS0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Bus0::dial`] or [`Bus0::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Bus0>> {
        // SAFETY: nng_bus0_open is the correct initialization function for Bus0 protocol.
        unsafe { super::create_socket(nng_sys::nng_bus0_open, Bus0) }
    }

    /// Creates a BUS0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Bus0::socket`] and [`Socket::dial`].
    /// For multiple connections or advanced configuration, use [`Bus0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Bus0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a BUS0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Bus0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Bus0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Bus0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Bus0> {
    /// Sends a message to all connected bus peers.
    ///
    /// Broadcasts the message to all nodes currently connected to this bus socket.
    /// The message will be delivered to every peer, making this suitable for
    /// announcements, status updates, and coordination messages.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the message may or may not
    /// be sent to peers (depending on when cancellation occurs). The message will be
    /// dropped if not sent. Note that the bus protocol provides no delivery
    /// guarantees even without cancellation.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the operation fails, giving you back
    /// the message for potential retry.
    pub async fn send(&mut self, message: Message) -> Result<(), (AioError, Message)> {
        self.send_msg(message).await
    }

    /// Receives the next message from any connected bus peer.
    ///
    /// Waits for the next message from any node connected to this bus socket.
    /// Messages arrive from any peer that sends to the bus network.
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
    fn test_bus_socket_creation() {
        let socket = Bus0::socket();
        assert!(socket.is_ok());
    }
}
