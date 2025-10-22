//! Surveyor/Respondent pattern (SURVEYOR0/RESPONDENT0 protocol).
//!
//! This module implements the Surveyor/Respondent messaging pattern, which provides
//! one-to-many request-response communication for distributed voting, service discovery,
//! and consensus-building scenarios.
//!
//! # Socket Types
//!
//! - [`Surveyor0`] - Broadcasts surveys and collects responses
//! - [`Respondent0`] - Receives surveys and optionally sends responses
//!
//! # Protocol Overview
//!
//! The Surveyor/Respondent pattern allows a surveyor to broadcast a "survey" question
//! to all connected respondents, who may optionally reply within a configurable time window.
//! Unlike request-reply (which is one-to-one), this provides one-to-many request-response.
//!
//! # Key Features
//!
//! - **Broadcast surveys**: One surveyor sends to all respondents
//! - **Optional responses**: Respondents may choose not to reply
//! - **Time-bounded**: Surveys have a configurable timeout
//! - **Multiple responses**: A surveyor can receive 0 to N responses per survey
//! - **One-at-a-time**: Sending a new survey cancels the previous one
//!
//! # Examples
//!
//! ## Simple surveyor-respondent
//!
//! ```rust
//! use anng::{protocols::survey0, Message};
//! use std::io::Write;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//! // Surveyor, in one task
//! # tokio::spawn(async {
//! let socket = survey0::Surveyor0::listen(c"inproc://survey").await?;
//! let mut ctx = socket.context();
//!
//! loop {
//!     let mut survey = Message::with_capacity(100);
//!     write!(&mut survey, "Who wants to be leader?")?;
//!
//!     let timeout = std::time::Duration::from_millis(200);
//!     // TODO: In production, handle error and retry with returned message
//!     let mut responses = ctx.survey(survey, timeout).await.unwrap();
//!     // Collect responses for the configured timeout period
//!     while let Some(response) = responses.next().await {
//!         match response {
//!             Ok(msg) => println!("Response: {:?}", msg.as_slice()),
//!             Err(e) => {
//!                 println!("Survey error: {:?}", e);
//!                 break;
//!             }
//!         }
//!     }
//! }
//! # Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
//! # });
//! # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
//!
//! // Respondent, in another task
//! let socket = survey0::Respondent0::dial(c"inproc://survey").await?;
//! let mut ctx = socket.context();
//!
//! let (survey, responder) = ctx.next_survey().await?;
//! println!("Got survey: {:?}", survey.as_slice());
//!
//! // Optionally respond to the survey
//! let mut response = Message::with_capacity(100);
//! write!(&mut response, "I volunteer!")?;
//! // TODO: In production, handle error and retry with returned responder and message
//! responder.respond(response).await.unwrap();
//! # Ok(())
//! # }
//! ```

use super::SupportsContext;
use crate::{ContextfulSocket, Socket, aio::AioError, message::Message};
use core::{
    ffi::{CStr, c_char, c_int},
    time::Duration,
};
use std::io;

/// Surveyor socket type for broadcasting surveys and collecting responses.
///
/// SURVEYOR sockets send surveys to all connected respondents and collect their optional responses
/// within a timeout window. Each survey has a unique ID (assigned by NNG) and can receive 0 to N
/// responses.
///
/// # Connection patterns
///
/// - **Typical**: Listen to accept respondent connections (`Surveyor0::listen()`)
/// - **Reverse**: Dial to connect to respondent endpoints (`Surveyor0::dial()`)
///
/// # Survey lifecycle
///
/// 1. Send a survey to all connected respondents
/// 2. Receive responses within the timeout window
/// 3. Survey automatically expires after timeout
/// 4. Sending a new survey cancels any ongoing survey
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::survey0::Surveyor0;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// // Listen for respondent connections
/// let socket = Surveyor0::listen(c"inproc://surveyor0-usage-doctest").await?;
/// let mut ctx = socket.context();
///
/// // Send survey and collect responses
/// let survey = anng::Message::from(&b"What's your status?"[..]);
/// let timeout = std::time::Duration::from_secs(1);
/// // TODO: In production, handle error and retry with returned message
/// let responses = ctx.survey(survey, timeout).await.unwrap();
/// // Process responses...
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Surveyor0;

impl SupportsContext for Surveyor0 {}

impl Surveyor0 {
    /// Creates a new SURVEYOR0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Surveyor0::dial`] or [`Surveyor0::listen`] which create and
    /// connect in one step.
    pub fn socket() -> io::Result<Socket<Surveyor0>> {
        // SAFETY: nng_surveyor0_open is the correct initialization function for Surveyor0 protocol.
        unsafe { super::create_socket(nng_sys::nng_surveyor0_open, Surveyor0) }
    }

    /// Creates a SURVEYOR0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Surveyor0::socket`] and [`Socket::listen`]. For
    /// listening on multiple addresses or advanced configuration, use [`Surveyor0::socket`]
    /// directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Surveyor0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a SURVEYOR0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Surveyor0::socket`] and [`Socket::dial`]. For
    /// connecting to multiple addresses or advanced configuration, use [`Surveyor0::socket`]
    /// directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Surveyor0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }
}

impl<'socket> ContextfulSocket<'socket, Surveyor0> {
    /// Sets the survey timeout for this context.
    ///
    /// This configures how long the surveyor will wait for responses before timing out. The
    /// timeout applies to subsequent survey operations on this context.
    fn set_survey_timeout(&mut self, timeout: Duration) {
        let timeout_ms = timeout.as_millis();
        if timeout_ms > i32::MAX as u128 {
            panic!("survey timeout is too large: {:?}", timeout);
        }

        let errno = unsafe {
            nng_sys::nng_ctx_set_ms(
                self.context.id(),
                nng_sys::NNG_OPT_SURVEYOR_SURVEYTIME as *const _ as *const c_char,
                timeout_ms as c_int,
            )
        };

        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            nng_sys::NNG_ECLOSED => {
                unreachable!("context is still open");
            }
            nng_sys::NNG_EINVAL => {
                unreachable!("timeout value should be valid");
            }
            nng_sys::NNG_ENOTSUP => {
                unreachable!("surveyor context should support SURVEYTIME option");
            }
            nng_sys::NNG_EREADONLY => {
                unreachable!("SURVEYTIME is not read-only");
            }
            errno => {
                unreachable!("nng_ctx_set_ms documentation claims errno {errno} is never returned");
            }
        }
    }

    /// Sends a survey and returns a stream of responses.
    ///
    /// This method implements the surveyor side of the Surveyor/Respondent pattern.
    /// It broadcasts the survey to all connected respondents and returns a response
    /// stream that yields responses as they arrive, up to the specified timeout.
    ///
    /// # Survey Timeout
    ///
    /// The provided timeout determines how long to wait for responses. After the timeout expires,
    /// [`SurveyResponses::next`] will `None`. The timeout cannot exceed `i32::MAX` milliseconds;
    /// if it does, this function panics.
    ///
    /// # Survey Cancellation
    ///
    /// Sending a new survey automatically cancels any ongoing survey. Late responses
    /// to the cancelled survey will be discarded.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled before the survey is sent,
    /// the message will be dropped. If the returned response stream is dropped, the
    /// survey remains active until its timeout expires or a new survey is sent on this
    /// context. Dropping the stream simply means you won't receive the responses, but
    /// respondents can still send them.
    ///
    /// # Errors
    ///
    /// Returns `Err((error, message))` if the survey operation fails, giving you
    /// back the message for potential retry.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::survey0, Message};
    /// use std::{io::Write, time::Duration};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = survey0::Respondent0::dial(c"inproc://vote").await?;
    /// #     let mut ctx = socket.context();
    /// #     loop {
    /// #         let (survey, responder) = ctx.next_survey().await?;
    /// #         let mut response = Message::with_capacity(50);
    /// #         write!(&mut response, "Yes")?;
    /// #         responder.respond(response).await.unwrap();
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// let socket = survey0::Surveyor0::listen(c"inproc://vote").await?;
    /// let mut ctx = socket.context();
    ///
    /// let mut survey = Message::with_capacity(100);
    /// write!(&mut survey, "Should we upgrade to version 2.0?")?;
    ///
    /// let timeout = Duration::from_secs(1);
    /// match ctx.survey(survey, timeout).await {
    ///     Ok(mut responses) => {
    ///         while let Some(response) = responses.next().await {
    ///             match response {
    ///                 Ok(msg) => println!("Vote: {:?}", msg.as_slice()),
    ///                 Err(e) => println!("Survey error: {:?}", e),
    ///             }
    ///         }
    ///         println!("Survey completed");
    ///     }
    ///     Err((error, msg)) => {
    ///         println!("Failed to send survey: {:?}", error);
    ///         // Could retry with `msg`
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn survey<'s>(
        &'s mut self,
        message: Message,
        timeout: Duration,
    ) -> Result<SurveyResponses<'s, 'socket>, (AioError, Message)> {
        tracing::trace!("send survey");

        // set the survey timeout before sending
        self.set_survey_timeout(timeout);

        // it's fine if send_msg is cancelled, but eventually succeeds; the context will implicitly
        // cancel the previous survey as a result and discard replies to that old survey.
        if let Err((e, msg)) = self.context.send_msg(message, &mut self.aio).await {
            tracing::error!(?e, "survey failed");
            Err((e, msg))
        } else {
            tracing::debug!("survey sent; ready to collect responses");
            Ok(SurveyResponses { contextful: self })
        }
    }
}

/// A stream of responses to a survey.
///
/// This type provides an async iterator over survey responses. See
/// [`SurveyResponses::next`] for details on response handling and timeout behavior.
#[derive(Debug)]
pub struct SurveyResponses<'s, 'socket> {
    contextful: &'s mut ContextfulSocket<'socket, Surveyor0>,
}

impl SurveyResponses<'_, '_> {
    /// Receives the next response from the survey.
    ///
    /// Returns `Some(Ok(message))` for each response received, `Some(Err(error))`
    /// for errors, and `None` when the survey timeout expires.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use anng::{protocols::survey0, Message, AioError};
    /// # use std::time::Duration;
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = survey0::Respondent0::dial(c"inproc://test").await?;
    /// #     let mut ctx = socket.context();
    /// #     loop {
    /// #         let (survey, responder) = ctx.next_survey().await?;
    /// #         let response = Message::from(&b"test response"[..]);
    /// #         responder.respond(response).await.unwrap();
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// # let socket = survey0::Surveyor0::listen(c"inproc://test").await?;
    /// # let mut ctx = socket.context();
    /// # let survey = Message::from(&b"test"[..]);
    /// let timeout = Duration::from_secs(1);
    /// let mut responses = ctx.survey(survey, timeout).await.unwrap();
    ///
    /// while let Some(response) = responses.next().await {
    ///     match response {
    ///         Ok(msg) => println!("Response: {:?}", msg.as_slice()),
    ///         Err(e) => {
    ///             println!("Survey error: {:?}", e);
    ///             break;
    ///         }
    ///     }
    /// #   break;
    /// }
    /// println!("Survey timeout reached");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next(&mut self) -> Option<Result<Message, AioError>> {
        match self
            .contextful
            .context
            .recv_msg(&mut self.contextful.aio)
            .await
        {
            Ok(message) => {
                tracing::trace!("received survey response");
                Some(Ok(message))
            }
            Err(AioError::TimedOut) => {
                // we hit this if we started the read _before_ the survey expired
                tracing::debug!("survey timeout reached");
                None
            }
            Err(AioError::Operation(errno)) if errno.get() == nng_sys::NNG_ESTATE => {
                // we hit this if we start the read _after_ the survey has already expired
                tracing::debug!("survey already timed out");
                None
            }
            Err(e) => {
                tracing::warn!(?e, "survey ended with error");
                Some(Err(e))
            }
        }
    }

    /// Non-blocking variant of [`next`](Self::next).
    ///
    /// Returns
    ///
    /// - `None` if no response is immediately available;
    /// - `Some(Ok(Some(message)))` if a response _is_ immediately available;
    /// - `Some(Ok(None))` if the survey has ended; and
    /// - `Some(Err(_))` on error
    pub fn try_next(&mut self) -> Option<Result<Option<Message>, AioError>> {
        match self.contextful.context.try_recv_msg() {
            Ok(Some(message)) => {
                tracing::trace!("received survey response");
                Some(Ok(Some(message)))
            }
            Ok(None) => None, // Would block - survey is still active
            Err(AioError::Operation(errno)) if errno.get() == nng_sys::NNG_ESTATE => {
                // Survey already timed out
                tracing::debug!("survey already timed out");
                Some(Ok(None))
            }
            Err(e) => {
                tracing::warn!(?e, "survey ended with error");
                Some(Err(e))
            }
        }
    }
}

/// Respondent socket type for receiving surveys and sending optional responses.
///
/// RESPONDENT sockets receive surveys from surveyors and can optionally send back
/// responses. Each respondent can choose whether to respond to any given survey.
///
/// # Connection patterns
///
/// - **Typical**: Dial to connect to surveyors (`Respondent0::dial()`)
/// - **Reverse**: Listen for surveyor connections (`Respondent0::listen()`)
/// - **Multi-connection**: Connect to multiple surveyors
///
/// # Response timing
///
/// Responses must be sent within the survey timeout window. Late responses are
/// automatically discarded by the surveyor.
///
/// # Usage
///
/// ```rust
/// # use anng::protocols::survey0::{Respondent0, Surveyor0};
/// # use anng::Message;
/// # use std::io::Write;
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
/// # tokio::spawn(async {
/// #     let socket = Surveyor0::listen(c"inproc://respondent0-usage-doctest").await?;
/// #     let mut ctx = socket.context();
/// #     loop {
/// #         let mut survey = Message::with_capacity(50);
/// #         write!(&mut survey, "health-check")?;
/// #         let timeout = std::time::Duration::from_millis(200);
/// #         let mut responses = ctx.survey(survey, timeout).await.unwrap();
/// #         while let Some(response) = responses.next().await {}
/// #     }
/// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
/// # });
/// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
/// // Connect to surveyor
/// let socket = Respondent0::dial(c"inproc://respondent0-usage-doctest").await?;
/// let mut ctx = socket.context();
///
/// // Receive survey and optionally respond
/// let (survey, responder) = ctx.next_survey().await?;
/// // Optionally respond to the survey
/// let response = anng::Message::from(&b"Available"[..]);
/// // TODO: In production, handle error and retry with returned responder and message
/// responder.respond(response).await.unwrap();
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Respondent0;

impl SupportsContext for Respondent0 {}

impl Respondent0 {
    /// Creates a new RESPONDENT0 socket without establishing connections.
    ///
    /// For simple cases, prefer [`Respondent0::dial`] or [`Respondent0::listen`] which create and
    /// connect in one step.
    pub fn socket() -> io::Result<Socket<Respondent0>> {
        // SAFETY: nng_respondent0_open is the correct initialization function for Respondent0 protocol.
        unsafe { super::create_socket(nng_sys::nng_respondent0_open, Respondent0) }
    }

    /// Creates a RESPONDENT0 socket and connects to the specified URL.
    ///
    /// Convenience function that combines [`Respondent0::socket`] and [`Socket::dial`]. For
    /// multiple connections or advanced configuration, use [`Respondent0::socket`] directly.
    pub async fn dial(url: impl AsRef<CStr>) -> io::Result<Socket<Respondent0>> {
        let socket = Self::socket()?;
        socket.dial(url.as_ref()).await?;
        Ok(socket)
    }

    /// Creates a RESPONDENT0 socket and listens on the specified URL.
    ///
    /// Convenience function that combines [`Respondent0::socket`] and [`Socket::listen`]. For
    /// listening on multiple addresses or advanced configuration, use [`Respondent0::socket`]
    /// directly.
    pub async fn listen(url: impl AsRef<CStr>) -> io::Result<Socket<Respondent0>> {
        let socket = Self::socket()?;
        socket.listen(url.as_ref()).await?;
        Ok(socket)
    }
}

impl<'socket> ContextfulSocket<'socket, Respondent0> {
    /// Receives the next survey and returns the message along with a responder.
    ///
    /// This method implements the respondent side of the Surveyor/Respondent pattern.
    /// It waits for an incoming survey and returns both the survey message and a
    /// [`SurveyResponder`] that can be used to send a response.
    ///
    /// # Responding to a survey
    ///
    /// The returned [`SurveyResponder`] allows you to send a response, but responding
    /// is entirely optional. You can choose to:
    /// - Respond immediately with [`SurveyResponder::respond`]
    /// - Drop the responder to not respond to this survey
    /// - Store the responder and respond later (within the timeout)
    ///
    /// Every survey has a timeout set by the surveyor, and if the response is not received within
    /// the timeout, it is discarded. This timeout is invisible to the respondents.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled after a survey message has
    /// been received, that message will be returned by the next call to `next_survey()`
    /// on the same context. No survey messages are lost due to cancellation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use anng::{protocols::survey0, Message};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = survey0::Surveyor0::listen(c"inproc://service-discovery").await?;
    /// #     let mut ctx = socket.context();
    /// #     loop {
    /// #         let mut survey = Message::with_capacity(50);
    /// #         write!(&mut survey, "health-check")?;
    /// #         let timeout = std::time::Duration::from_millis(200);
    /// #         let mut responses = ctx.survey(survey, timeout).await.unwrap();
    /// #         while let Some(response) = responses.next().await {}
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    /// let socket = survey0::Respondent0::dial(c"inproc://service-discovery").await?;
    /// let mut ctx = socket.context();
    ///
    /// loop {
    ///     let (survey, responder) = ctx.next_survey().await?;
    ///
    ///     // Check if we want to respond to this survey
    ///     if survey.as_slice() == b"health-check" {
    ///         let mut response = Message::with_capacity(50);
    ///         write!(&mut response, "healthy")?;
    ///
    ///         // TODO: In production, handle error and retry with returned responder and message
    ///         responder.respond(response).await.unwrap();
    ///     }
    ///     // Otherwise, ignore the survey by not using the responder
    /// #   break;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn next_survey<'s>(
        &'s mut self,
    ) -> Result<(Message, SurveyResponder<'s, 'socket>), AioError> {
        // NOTE(jon): if `recv_msg` succeeds after being dropped, it saves its Message so that the
        // next call to `next_survey` will return the same message.
        let message = self.context.recv_msg(&mut self.aio).await?;
        let responder = SurveyResponder { contextful: self };
        Ok((message, responder))
    }

    /// Non-blocking variant of [`next_survey`](Self::next_survey).
    ///
    /// Returns `Ok(Some((message, responder)))` if immediately available, `Ok(None)` if no survey is available.
    pub fn try_next_survey<'s>(
        &'s mut self,
    ) -> Result<Option<(Message, SurveyResponder<'s, 'socket>)>, AioError> {
        match self.context.try_recv_msg()? {
            Some(message) => {
                let responder = SurveyResponder { contextful: self };
                Ok(Some((message, responder)))
            }
            None => Ok(None),
        }
    }
}

/// A handle for sending a response to a specific survey.
///
/// This type provides a mechanism for sending survey responses. See [`SurveyResponder::respond`]
/// for details.
#[derive(Debug)]
pub struct SurveyResponder<'s, 'socket> {
    contextful: &'s mut ContextfulSocket<'socket, Respondent0>,
}

impl SurveyResponder<'_, '_> {
    /// Sends a response message to the survey.
    ///
    /// This method consumes the `SurveyResponder` and sends the provided message
    /// as a response to the original survey. The response will be delivered to
    /// the surveyor if sent within the survey timeout window.
    ///
    /// # Optional Response
    ///
    /// Surveys are designed for optional responses. A respondent may choose not to
    /// respond for various reasons (not interested, busy, etc.). This is normal
    /// protocol behavior and won't cause errors. To decline to respond, simply drop the
    /// `SurveyResponder`.
    ///
    /// # Survey Timeouts
    ///
    /// Responses sent after the survey timeout are automatically discarded by
    /// the surveyor. There's no way to know if your response arrived in time
    /// from the respondent side.
    ///
    /// # Cancellation safety
    ///
    /// This function is **cancellation safe**. If cancelled, the response may or may not
    /// be sent (depending on when cancellation occurs). The message and responder will be
    /// dropped if not sent. Note that responses are optional in the survey protocol - it's
    /// normal for respondents to not respond.
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
    /// use anng::{protocols::survey0, Message, AioError};
    /// use std::io::Write;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    /// # tokio::spawn(async {
    /// #     let socket = survey0::Surveyor0::listen(c"inproc://status").await?;
    /// #     let mut ctx = socket.context();
    /// #     loop {
    /// #         let survey = Message::from(&b"status check"[..]);
    /// #         let timeout = std::time::Duration::from_secs(1);
    /// #         let mut responses = ctx.survey(survey, timeout).await.unwrap();
    /// #         while let Some(response) = responses.next().await {}
    /// #     }
    /// #     Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    /// # });
    /// # tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    /// let socket = survey0::Respondent0::dial(c"inproc://status").await?;
    /// let mut ctx = socket.context();
    ///
    /// let (survey, responder) = ctx.next_survey().await?;
    ///
    /// // Optionally respond to the survey
    /// let mut response = Message::with_capacity(100);
    /// write!(&mut response, "status: operational")?;
    ///
    /// match responder.respond(response).await {
    ///     Ok(()) => println!("Response sent"),
    ///     Err((responder, error, message)) => {
    ///         println!("Failed to respond: {:?}", error);
    ///         // Could retry with `responder.respond(message)`
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn respond(self, message: Message) -> Result<(), (Self, AioError, Message)> {
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
