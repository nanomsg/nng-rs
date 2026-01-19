//! Publish/Subscribe pattern (PUB0/SUB0 protocol).
//!
//! This module implements the Publish/Subscribe messaging pattern, which provides
//! one-to-many broadcasting with topic-based filtering. Publishers send messages
//! to all connected subscribers, with optional topic filtering at the subscriber.
//!
//! # Socket Types
//!
//! - [`Pub0`] - Publishers that broadcast messages to all subscribers
//! - [`Sub0`] - Subscribers that receive filtered messages from publishers
//!
//! # Protocol Overview
//!
//! The Publish/Subscribe pattern is unidirectional and best-effort with no
//! acknowledgment or delivery guarantees. Messages are broadcast to all
//! connected subscribers on a best-effort basis.
//!
//! # Key Features
//!
//! - **Topic filtering**: Subscribers filter by prefix (e.g., "news." matches "news.sports")
//! - **Best-effort delivery**: No acknowledgments or delivery guarantees
//! - **Broadcast**: Each message is sent to all connected subscribers
//!
//! # Examples
//!
//! ## Simple publisher-subscriber
//!
//! ```rust
//! use anng::{protocols::pubsub0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Publisher, in one task
//! # tokio::spawn(async {
//! let mut pub_socket = pubsub0::Pub0::listen(c"inproc://news").await?;
//!
//! # loop {
//! let mut msg = Message::with_capacity(100);
//! write!(&mut msg, "news.sports: Team wins!")?;
//! // TODO: In production, handle error and retry with returned message
//! pub_socket.publish(msg).await.unwrap();
//! # tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//! # }
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//! # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//!
//! // Subscriber, in another task
//! let sub_socket = pubsub0::Sub0::dial(c"inproc://news").await?;
//! let mut sub = sub_socket.context();
//! sub.subscribe_to(b"news."); // Subscribe to all news
//!
//! let msg = sub.next().await?;
//! println!("Received: {:?}", std::str::from_utf8(msg.as_slice())?);
//! # Ok(())
//! # }
//! ```
//!
//! ## Topic-based filtering
//!
//! ```rust
//! use anng::{protocols::pubsub0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! # tokio::spawn(async {
//! # let mut pub_socket = pubsub0::Pub0::listen(c"inproc://pubsub-topic-filter-demo").await?;
//! # loop {
//! #     let mut msg = Message::with_capacity(100);
//! #     write!(&mut msg, "news.sports: Team wins!")?;
//! #     pub_socket.publish(msg).await.unwrap();
//! #     tokio::time::sleep(std::time::Duration::from_millis(100)).await;
//! # }
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//! # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//! let sub_socket = pubsub0::Sub0::dial(c"inproc://pubsub-topic-filter-demo").await?;
//! let mut sub = sub_socket.context();
//!
//! // Subscribe to multiple topics
//! sub.subscribe_to(b"weather.");
//! sub.subscribe_to(b"news.sports");
//! sub.subscribe_to(b"alerts.");
//!
//! // Disable all filtering (receive everything)
//! // sub.disable_filtering();
//!
//! loop {
//!     let msg = sub.next().await?;
//!     println!("Got: {:?}", std::str::from_utf8(msg.as_slice())?);
//! #   break;
//! }
//! # Ok(())
//! # }
//! ```

use super::SupportsContext;
use crate::{ContextfulSocket, Socket, aio::AioError, message::Message};
use core::ffi::{CStr, c_char, c_void};
use nng_sys::nng_err;
use std::io;

/// Subscribe socket type for receiving filtered messages from publishers.
///
/// SUB sockets receive messages that match their topic subscriptions. By default,
/// they filter out ALL messages until subscriptions are explicitly added. Topic
/// filtering is based on byte-level prefix matching and occurs at the subscriber.
/// You may want to set up filters _before_ dialing/listening to avoid missing
/// messages in between connecting and adding them later.
///
/// # Connection patterns
///
/// - **Typical**: Dial to connect to publishers (`Sub0::dial()`)
/// - **Reverse**: Listen for connections from publishers (`Sub0::listen()`)
/// - **Multi-connection**: Connect to multiple publishers (may receive duplicates)
///
/// # Topic filtering
///
/// - Use `subscribe_to(b"prefix")` to receive messages starting with a prefix
/// - Use `disable_filtering()` to receive all messages (equivalent to `subscribe_to(b"")`)
/// - Multiple subscriptions are combined with OR
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::pubsub0::{Sub0, Pub0};
/// # use std::io::Write;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # tokio::spawn(async {
/// #     let mut pub_socket = Pub0::listen(c"inproc://sub0-usage-doctest").await?;
/// #     loop {
/// #         let mut msg = anng::Message::with_capacity(100);
/// #         write!(&mut msg, "news.breaking: Important update!")?;
/// #         pub_socket.publish(msg).await.unwrap();
/// #         tokio::time::sleep(std::time::Duration::from_millis(100)).await;
/// #     }
/// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
/// # });
/// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
/// // Connect to publisher
/// let socket = Sub0::dial(c"inproc://sub0-usage-doctest").await?;
/// let mut ctx = socket.context();
///
/// // Subscribe to specific topics
/// ctx.subscribe_to(b"news.");
/// ctx.subscribe_to(b"alerts.");
///
/// // Receive messages
/// loop {
///     let message = ctx.next().await?;
/// #   break;
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Sub0;

impl SupportsContext for Sub0 {}

impl Sub0 {
    /// Creates a new SUB0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Sub0::dial`] or [`Sub0::listen`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Sub0>> {
        // SAFETY: nng_sub0_open is the correct initialization function for Sub0 protocol.
        unsafe { super::create_socket(nng_sys::nng_sub0_open, Sub0) }
    }

    /// Creates a SUB0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Sub0::socket`] and [`Socket::dial`].
    /// For setting up subscriptions first or multiple connections, use [`Sub0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Sub0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a SUB0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Sub0::socket`] and [`Socket::listen`].
    /// For setting up subscriptions first or multiple connections, use [`Sub0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Sub0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl<'socket> ContextfulSocket<'socket, Sub0> {
    /// Receives the next message matching the socket's subscriptions.
    ///
    /// This method waits for the next published message that matches any of
    /// the socket's topic subscriptions. If no subscriptions have been set up,
    /// this will never return a message.
    ///
    /// # Message ordering
    ///
    /// Messages are delivered in the order they arrive, but there are no
    /// guarantees about ordering across different publishers or topics.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled after a message is received,
    /// that message will be returned by the next call to `next()` on the same context.
    /// Note that the PUB/SUB protocol allows message loss by design - messages may be
    /// dropped during normal operation due to slow subscribers or network conditions,
    /// independent of cancellation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::protocols::pubsub0;
    /// # use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let mut pub_socket = pubsub0::Pub0::listen(c"inproc://next-doctest-feed").await?;
    /// #     loop {
    /// #         let mut msg = anng::Message::with_capacity(100);
    /// #         write!(&mut msg, "news.sports: Team wins championship!")?;
    /// #         pub_socket.publish(msg).await.unwrap();
    /// #         tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    /// let socket = pubsub0::Sub0::dial(c"inproc://next-doctest-feed").await?;
    /// let mut sub = socket.context();
    ///
    /// sub.subscribe_to(b"news.");
    ///
    /// loop {
    ///     let msg = sub.next().await?;
    ///     println!("Received: {:?}", msg.as_slice());
    /// #     break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next(&mut self) -> Result<Message, AioError> {
        self.context.recv_msg(&mut self.aio).await
    }

    /// Non-blocking variant of [`next`](Self::next).
    ///
    /// Returns `Ok(Some(message))` if immediately available, `Ok(None)` if no message is available.
    pub fn try_next(&mut self) -> Result<Option<Message>, AioError> {
        self.context.try_recv_msg()
    }

    /// Sets preference for what to do with new messages when receive queue is full.
    ///
    /// When `true` (default): drops oldest message to make room for new ones.
    /// When `false`: rejects new messages, preserving older ones.
    pub fn prefer_new(&mut self, prefer: bool) {
        let errno = unsafe {
            nng_sys::nng_ctx_set_bool(
                self.context.id(),
                nng_sys::NNG_OPT_SUB_PREFNEW as *const _ as *const c_char,
                prefer,
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("socket is still open");
            }
            x if x == nng_err::NNG_EINVAL as u32 => {
                unreachable!("the value is valid for this setting");
            }
            x if x == nng_err::NNG_ENOTSUP as u32 => {
                unreachable!("this is a sub0 socket, and those support PREFNEW");
            }
            x if x == nng_err::NNG_EREADONLY as u32 => {
                unreachable!("PREFNEW is not read-only");
            }
            x if x == nng_err::NNG_ESTATE as u32 => {
                panic!("state machine for sub0 socket is in unexpected state for setting PREFNEW");
            }
            errno => {
                unreachable!(
                    "nng_socket_set_bool documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Disables topic filtering to receive all published messages.
    ///
    /// This is equivalent to subscribing to an empty topic (`b""`), which
    /// matches all messages. After calling this method, the socket will
    /// receive every message published to the connected publishers.
    ///
    /// **Note**: Subsequent `subscribe_to()` calls are effectively ignored
    /// since the empty subscription matches all messages regardless of other filters.
    ///
    /// # Performance impact
    ///
    /// Disabling filtering means receiving all messages, which can impact
    /// performance in high-volume scenarios. Use specific subscriptions
    /// when possible to reduce network and processing overhead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::protocols::pubsub0;
    /// # use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let mut pub_socket = pubsub0::Pub0::listen(c"inproc://sub0-disable-filtering-demo").await?;
    /// #     loop {
    /// #         let mut msg = anng::Message::with_capacity(100);
    /// #         write!(&mut msg, "test message")?;
    /// #         pub_socket.publish(msg).await.unwrap();
    /// #         tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    /// let socket = pubsub0::Sub0::dial(c"inproc://sub0-disable-filtering-demo").await?;
    /// let mut sub = socket.context();
    ///
    /// // Receive all messages regardless of topic
    /// sub.disable_filtering();
    ///
    /// // This will now receive every published message
    /// loop {
    ///     let msg = sub.next().await?;
    ///     println!("Got: {:?}", msg.as_slice());
    /// #   break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn disable_filtering(&mut self) {
        // SAFETY: context is valid SUB context, NULL topic with length 0 subscribes to all messages.
        let errno =
            unsafe { nng_sys::nng_sub0_ctx_subscribe(self.context.id(), core::ptr::null(), 0) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            x if x == nng_err::NNG_ENOTSUP as u32 => {
                unreachable!("this is a sub0 socket");
            }
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("socket is still open");
            }
            errno => {
                unreachable!(
                    "nng_sub0_socket_subscribe documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Subscribes to messages with the specified topic prefix.
    ///
    /// Only messages whose content starts with the given topic prefix will
    /// be delivered to this socket. Multiple subscriptions can be active
    /// simultaneously - a message matching _any_ subscription will be delivered.
    ///
    /// # Topic matching
    ///
    /// Topic matching is based on byte-level prefix comparison:
    /// - `b"news."` matches `b"news.sports"`, `b"news.weather"`, etc.
    /// - `b""` (empty) matches all messages (equivalent to [`disable_filtering`](Self::disable_filtering))
    /// - Matching is case-sensitive and exact
    ///
    /// # Performance
    ///
    /// Topic filtering happens at the subscriber side, so all messages are
    /// still transmitted over the network. For high-volume scenarios with
    /// many unwanted messages, consider protocol-level filtering if available.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::protocols::pubsub0;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # let _pub_socket = pubsub0::Pub0::listen(c"inproc://sub0-subscribe-to-demo").await?;
    /// let socket = pubsub0::Sub0::dial(c"inproc://sub0-subscribe-to-demo").await?;
    /// let mut sub = socket.context();
    ///
    /// // Subscribe to specific topics
    /// sub.subscribe_to(b"weather.forecast");
    /// sub.subscribe_to(b"news.breaking");
    /// sub.subscribe_to(b"alerts."); // Matches all alerts
    ///
    /// // Will receive any message starting with these prefixes
    /// # Ok(())
    /// # }
    /// ```
    pub fn subscribe_to(&mut self, topic: &[u8]) {
        // SAFETY: context is valid SUB context, topic slice is valid for reading topic.len() bytes.
        let errno = unsafe {
            nng_sys::nng_sub0_ctx_subscribe(
                self.context.id(),
                topic.as_ptr() as *const c_void,
                topic.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            x if x == nng_err::NNG_ENOTSUP as u32 => {
                unreachable!("this is a sub0 socket");
            }
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("socket is still open");
            }
            errno => {
                unreachable!(
                    "nng_sub0_socket_subscribe documentation claims errno {errno} is never returned"
                );
            }
        }
    }

    /// Removes a subscription for the specified topic prefix.
    ///
    /// After calling this method, messages with the specified prefix will
    /// no longer be delivered to this socket (unless they match other
    /// active subscriptions).
    ///
    /// # Return Value
    ///
    /// Returns `true` if the subscription was previously active and has been
    /// removed, and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::protocols::pubsub0;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # let _pub_socket = pubsub0::Pub0::listen(c"inproc://sub0-unsubscribe-demo").await?;
    /// let socket = pubsub0::Sub0::dial(c"inproc://sub0-unsubscribe-demo").await?;
    /// let mut sub = socket.context();
    ///
    /// sub.subscribe_to(b"news.");
    /// sub.subscribe_to(b"weather.");
    ///
    /// // Remove news subscription
    /// let was_subscribed = sub.unsubscribe_from(b"news.");
    /// assert!(was_subscribed);
    ///
    /// // Try to remove non-existent subscription
    /// let was_subscribed = sub.unsubscribe_from(b"sports.");
    /// assert!(!was_subscribed);
    /// # Ok(())
    /// # }
    /// ```
    pub fn unsubscribe_from(&mut self, topic: &[u8]) -> bool {
        let errno = unsafe {
            nng_sys::nng_sub0_ctx_unsubscribe(
                self.context.id(),
                topic.as_ptr() as *const c_void,
                topic.len(),
            )
        };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => true,
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            x if x == nng_err::NNG_ENOTSUP as u32 => {
                unreachable!("this is a sub0 socket");
            }
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("socket is still open");
            }
            x if x == nng_err::NNG_ENOENT as u32 => false,
            errno => {
                unreachable!(
                    "nng_sub0_socket_unsubscribe documentation claims errno {errno} is never returned"
                );
            }
        }
    }
}

/// Publish socket type for broadcasting messages to all subscribers.
///
/// PUB sockets send messages to all connected subscribers on a best-effort basis.
/// There are no delivery guarantees, acknowledgments, or retransmissions. Messages
/// are sent immediately to current subscribers; late-connecting subscribers miss
/// previous messages.
///
/// # Connection patterns
///
/// - **Typical**: Listen to accept subscriber connections (`Pub0::listen()`)
/// - **Reverse**: Connect to each subscriber's endpoint (`Pub0::dial()`)
///
/// # Message delivery
///
/// - **Best-effort**: No guarantees about successful delivery
/// - **At-most-once**: Messages are not retransmitted
/// - **Broadcast**: All connected subscribers receive each message
/// - **No buffering**: Late subscribers miss messages sent before connection
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::pubsub0::Pub0;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// // Listen for subscriber connections
/// let mut socket = Pub0::listen(c"inproc://pub0-usage-doctest").await?;
///
/// // Broadcast message to all subscribers
/// let message = anng::Message::from(&b"news: Breaking update!"[..]);
/// // TODO: In production, handle error and retry with returned message
/// socket.publish(message).await.unwrap();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Pub0;

impl Pub0 {
    /// Creates a new PUB0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Pub0::listen`] or [`Pub0::dial`] which create and connect in one step.
    pub fn socket() -> io::Result<Socket<Pub0>> {
        // SAFETY: nng_pub0_open is the correct initialization function for Pub0 protocol.
        unsafe { super::create_socket(nng_sys::nng_pub0_open, Pub0) }
    }

    /// Creates a PUB0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Pub0::socket`] and [`Socket::listen`].
    /// For listening on multiple addresses or advanced configuration, use [`Pub0::socket`] directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Pub0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a PUB0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Pub0::socket`] and [`Socket::dial`].
    /// For connecting to multiple addresses or advanced configuration, use [`Pub0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Pub0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }
}

impl Socket<Pub0> {
    /// Publishes a message to all connected subscribers.
    ///
    /// This method broadcasts the provided message to every subscriber
    /// connected to this publisher. The message is sent on a best-effort
    /// basis with no delivery guarantees.
    ///
    /// # Delivery semantics
    ///
    /// - **Best-effort**: No guarantees about successful delivery
    /// - **At-most-once**: Messages are not retransmitted
    /// - **Immediate**: Messages are sent immediately to current subscribers
    /// - **No buffering**: Late-connecting subscribers miss the message
    ///
    /// Note that NNG does _not_ guarantee that the publish has actually completed when this
    /// operation returns, and there is no automatic linger or flush to ensure that the socket send
    /// buffers have completely transmitted when a socket is closed. Thus, it is recommended to
    /// wait a brief period after calling send before letting the socket be dropped.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the message may or may not
    /// reach subscribers (depending on when cancellation occurs). The message will be
    /// dropped if not sent. Note that PUB/SUB is a best-effort protocol - there are no
    /// delivery guarantees even without cancellation.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the publish operation fails,
    /// giving you back the message for potential retry or logging.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::pubsub0, Message, AioError};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// let mut socket = pubsub0::Pub0::listen(c"inproc://publish-doctest-feed").await?;
    ///
    /// loop {
    ///     let mut msg = Message::with_capacity(100);
    ///     write!(&mut msg, "weather.forecast: Sunny, 25°C")?;
    ///
    ///     match socket.publish(msg).await {
    ///         Ok(()) => println!("Message published"),
    ///         Err((error, msg)) => {
    ///             println!("Publish failed: {:?}", error);
    ///             // Could log or retry with `msg`
    ///         }
    ///     }
    /// #     break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(&mut self, message: Message) -> Result<(), (AioError, Message)> {
        self.send_msg(message).await
    }
}
