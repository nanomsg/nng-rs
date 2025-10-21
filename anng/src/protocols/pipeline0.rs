//! Push/Pull pattern (PUSH0/PULL0 protocol).
//!
//! This module implements the Push/Pull messaging pattern (also known as the
//! "pipeline" pattern), which provides unidirectional work distribution.
//! Push sockets send tasks to pull sockets, with automatic load balancing
//! across multiple workers.
//!
//! # Socket Types
//!
//! - [`Push0`] - Distributes work to pull sockets
//! - [`Pull0`] - Receives work from push sockets
//!
//! # Protocol Overview
//!
//! The pipeline pattern is designed for distributing work across multiple workers.
//! It's unidirectional (push-only send, pull-only receive) and stateless
//! (no acknowledgments or delivery tracking).
//!
//! # Reliability Considerations
//!
//! **Important**: The pipeline protocol is **unreliable** and provides **no delivery guarantees**.
//! Although the protocol honors flow control and attempts to avoid dropping messages,
//! messages may be lost under various conditions. There is no capability for message
//! acknowledgment or delivery confirmation.
//!
//! Applications that require reliable delivery should consider using the [`REQ/REP protocol`](crate::protocols::reqrep0)
//! instead, which provides acknowledgment through the request-reply cycle.
//!
//! # Examples
//!
//! ## Simple push-pull pipeline
//!
//! ```rust
//! use anng::{protocols::pipeline0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Pusher (work distributor), in one task
//! # tokio::spawn(async {
//! let mut push_socket = pipeline0::Push0::listen(c"inproc://work-queue").await?;
//!
//! let mut task = Message::with_capacity(100);
//! write!(&mut task, "process_data(42)")?;
//! // TODO: In production, handle error and retry with returned message
//! push_socket.push(task).await.unwrap();
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//!
//! // Puller (worker), in another task
//! let mut pull_socket = pipeline0::Pull0::dial(c"inproc://work-queue").await?;
//!
//! let task = pull_socket.pull().await?;
//! println!("Got task: {:?}", std::str::from_utf8(task.as_slice())?);
//! # Ok(())
//! # }
//! ```
//!
//! ## Load balancing with multiple workers
//!
//! ```rust
//! use anng::{protocols::pipeline0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Start multiple workers
//! for worker_id in 0..2 {
//!     tokio::spawn(async move {
//!         let mut worker = pipeline0::Pull0::dial(c"inproc://pipeline-loadbalance-demo").await?;
//!         loop {
//!             let task = worker.pull().await?;
//!             println!("Worker {} processing: {:?}", worker_id, task.as_slice());
//!             // Process task...
//! #           break; // Exit after processing one task
//!         }
//!         Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//!     });
//! }
//!
//! // Distribute work
//! let mut distributor = pipeline0::Push0::listen(c"inproc://pipeline-loadbalance-demo").await?;
//! for i in 0..2 {
//!     let mut task = Message::with_capacity(50);
//!     write!(&mut task, "task_{}", i)?;
//!     // TODO: In production, handle error and retry with returned message
//!     distributor.push(task).await.unwrap();
//! }
//! # Ok(())
//! # }
//! ```

use crate::{Socket, aio::AioError, message::Message};
use core::ffi::CStr;
use core::ffi::c_char;
use std::io;

/// Pull socket type for receiving work from push sockets.
///
/// PULL sockets act as workers that receive tasks distributed by push sockets.
/// Multiple pull sockets automatically share the workload through round-robin
/// distribution.
///
/// # Connection patterns
///
/// - **Typical**: Dial to connect to distributors (`Pull0::dial()`)
/// - **Reverse**: Listen for distributor connections (`Pull0::listen()`)
/// - **Multi-connection**: Connect to multiple distributors
#[derive(Debug, Clone, Copy)]
pub struct Pull0;

impl Pull0 {
    /// Creates a new PULL0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Pull0::dial`] or [`Pull0::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Pull0>> {
        // SAFETY: nng_pull0_open is the correct initialization function for Pull0 protocol.
        unsafe { super::create_socket(nng_sys::nng_pull0_open, Pull0) }
    }

    /// Creates a PULL0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Pull0::socket`] and [`Socket::dial`].
    /// For multiple connections or advanced configuration, use [`Pull0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Pull0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a PULL0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Pull0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Pull0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Pull0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Pull0> {
    /// Receives the next task from connected push sockets.
    ///
    /// Waits for the next message from any connected distributor. Messages arrive
    /// in order but with no guarantees across multiple connections.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled after a message is received,
    /// that message will be returned by the next call to `pull()` on the same socket.
    /// No work items are lost due to cancellation.
    pub async fn pull(&mut self) -> Result<Message, AioError> {
        self.recv_msg().await
    }

    /// Non-blocking variant of [`pull`](Self::pull).
    ///
    /// Returns `Ok(Some(message))` if immediately available, `Ok(None)` if no message is available.
    pub fn try_pull(&mut self) -> Result<Option<Message>, AioError> {
        self.try_recv_msg()
    }
}

/// Push socket type for distributing work to pull sockets.
///
/// PUSH sockets act as work distributors, sending tasks to connected pull sockets
/// using round-robin load balancing. Each message goes to exactly one worker.
///
/// # Connection patterns
///
/// - **Typical**: Listen for worker connections (`Push0::listen()`)
/// - **Reverse**: Connect to worker endpoints (`Push0::dial()`)
///
/// # Load balancing & flow control
///
/// - **Distribution**: Each message goes to exactly one worker
/// - **Round-robin**: Best-effort fair distribution among available workers
/// - **Natural back-pressure**: Slow workers don't get overwhelmed
/// - **Buffering**: Use `set_send_buffer()` to queue messages locally
#[derive(Debug, Clone, Copy)]
pub struct Push0;

impl Push0 {
    /// Creates a new PUSH0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Push0::listen`] or [`Push0::dial`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Push0>> {
        // SAFETY: nng_push0_open is the correct initialization function for Push0 protocol.
        unsafe { super::create_socket(nng_sys::nng_push0_open, Push0) }
    }

    /// Creates a PUSH0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Push0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Push0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Push0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a PUSH0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Push0::socket`] and [`Socket::dial`].
    /// For connecting to multiple addresses or advanced configuration, use [`Push0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Push0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Push0> {
    /// Sets the send buffer size for this push socket.
    ///
    /// Controls how many messages can be queued locally when workers are not
    /// immediately ready. Set before establishing connections.
    ///
    /// **Note**: Transport-level buffering (TCP send buffers, etc.) may occur
    /// independently of this setting.
    ///
    /// # Parameters
    ///
    /// - `size`: Messages to buffer (0-8192). Default is 0 (unbuffered).
    pub fn set_send_buffer(&mut self, size: u16) -> io::Result<()> {
        // Validate range
        if size > 8192 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("send buffer size {} exceeds maximum of 8192", size),
            ));
        }

        let errno = unsafe {
            nng_sys::nng_socket_set_int(
                self.id(),
                nng_sys::NNG_OPT_SENDBUF as *const _ as *const c_char,
                i32::from(size),
            )
        };

        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(()),
            nng_sys::NNG_ECLOSED => {
                unreachable!("socket is still open");
            }
            nng_sys::NNG_EINVAL => {
                unreachable!("we've checked the range of the input");
            }
            nng_sys::NNG_ENOMEM => Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                "insufficient memory to resize send buffer",
            )),
            errno => {
                unreachable!("nng documentation claims errno {errno} is never returned");
            }
        }
    }

    /// Distributes a task to one of the connected pull sockets.
    ///
    /// Sends the message to exactly one worker using round-robin distribution.
    /// Blocks until a worker is available if all are busy.
    ///
    /// Note that NNG does _not_ guarantee that the push has actually completed when this operation
    /// returns, and there is no automatic linger or flush to ensure that the socket send buffers
    /// have completely transmitted when a socket is closed. Thus, it is recommended to wait a
    /// brief period after calling send before letting the socket be dropped.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the message may or may not
    /// be sent to a worker (depending on when cancellation occurs). The message will be
    /// dropped if not sent. Note that the pipeline protocol provides no delivery
    /// guarantees even without cancellation.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the operation fails, giving you back
    /// the message for potential retry.
    pub async fn push(&mut self, message: Message) -> Result<(), (AioError, Message)> {
        self.send_msg(message).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_send_buffer_configuration() {
        let mut pusher = Push0::socket().unwrap();

        // Test setting various buffer sizes
        pusher.set_send_buffer(0).unwrap(); // Unbuffered
        pusher.set_send_buffer(1).unwrap(); // Minimal buffering
        pusher.set_send_buffer(128).unwrap(); // Moderate buffering
        pusher.set_send_buffer(8192).unwrap(); // Maximum buffering

        // Test invalid values
        let result = pusher.set_send_buffer(8193);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("8193"));
        assert!(error.to_string().contains("maximum of 8192"));
    }
}
