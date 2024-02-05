//! An asynchronous I/O context that integrates with Rust's async/await.
//!
//! NNG has its own asynchronous I/O system and driver that is distinct from Rust's async/await
//! paradigm. This module provides an [`crate::Aio`] object that has a callback that is specifically
//! designed to provide hooks into a normal Rust asynchronous project.
use std::{os::raw::c_void, ptr::{self, NonNull}, sync::{Arc, Mutex}, time::Duration};
use log::error;
use crate::{
	Context,
	error::Result,
	Message,
	util::{validate_ptr, duration_to_nng},
};

/// A [`crate::Aio`] object that can be integrated into a Rust async runtime.
#[derive(Debug)]
pub struct Aio
{
}

impl Aio
{
	/// Creates a new asynchronous I/O handle.
	///
	/// # Errors
	///
	/// * [`OutOfMemory`]: Insufficient memory available.
	///
	/// [`OutOfMemory`]: enum.Error.html#variant.OutOfMemory
	pub fn new() -> Result<Self>
	{
		todo!()
	}

	/// Set the timeout of asynchronous operations.
	///
	/// This causes a timer to be started when the operation is actually
	/// started. If the timer expires before the operation is completed, then it
	/// is aborted with [`TimedOut`].
	///
	/// As most operations involve some context switching, it is usually a good
	/// idea to allow a least a few tens of milliseconds before timing them out
	/// as a too small timeout might not allow the operation to properly begin
	/// before giving up!
	pub fn set_timeout(&self, dur: Option<Duration>)
	{
		todo!()
	}

	/// Causes the AIO to sleep for the provided duration.
	///
	/// This is essentially no different than causing the thread to asynchronously sleep for the
	/// specified time, so it is only included here for completion's sake.
	pub fn sleep(&mut self, dur: Duration) -> Sleep<'_>
	{
		todo!()
	}

	/// Sends the message using in the provided [`Context`].
	///
	/// # Errors
	///
	/// * [`Closed`]: The socket is not open.
	/// * [`IncorrectState`]: The socket cannot send messages in this state.
	/// * [`MessageTooLarge`]: The message is too large.
	/// * [`NotSupported`]: The protocol does not support sending messages.
	/// * [`OutOfMemory`]: Insufficient memory available.
	/// * [`TimedOut`]: The operation timed out.
	///
	/// [`Closed`]: enum.Error.html#variant.Closed
	/// [`IncorrectState`]: enum.Error.html#variant.IncorrectState
	/// [`MessageTooLarge`]: enum.Error.html#variant.MessageTooLarge
	/// [`NotSupported`]: enum.Error.html#variant.NotSupported
	/// [`OutOfMemory`]: enum.Error.html#variant.OutOfMemory
	/// [`TimedOut`]: enum.Error.html#variant.TimedOut
	/// [`IncorrectState`]: The protocol is not in the correct state.
	pub fn send<M>(&mut self, ctx: &Context, msg: M) -> Send<'_>
	where
		M: Into<Message>,
	{
		todo!()
	}

	/// Receives the next message on the given [`Context`].
	///
	/// # Errors
	///
	/// * [`Closed`]: The socket is not open.
	/// * [`IncorrectState`]: The socket cannot send messages in this state.
	/// * [`MessageTooLarge`]: The message is too large.
	/// * [`NotSupported`]: The protocol does not support sending messages.
	/// * [`OutOfMemory`]: Insufficient memory available.
	/// * [`TryAgain`]: The operation would block.
	///
	/// [`Closed`]: enum.Error.html#variant.Closed
	/// [`IncorrectState`]: enum.Error.html#variant.IncorrectState
	/// [`MessageTooLarge`]: enum.Error.html#variant.MessageTooLarge
	/// [`NotSupported`]: enum.Error.html#variant.NotSupported
	/// [`OutOfMemory`]: enum.Error.html#variant.OutOfMemory
	/// [`TryAgain`]: enum.Error.html#variant.TryAgain
	pub fn recv(&mut self, ctx: &Context) -> Recv<'_>
	{
		todo!()
	}

	/// The callback function for all AIO operations.
	extern "C" fn callback(arg: *mut c_void)
	{
		todo!()
	}
}

/// A future created by calling [`Aio::sleep`].
#[derive(Debug)]
pub struct Sleep<'a>
{
	_pd: std::marker::PhantomData<&'a ()>,
}

/// A future created by calling [`Aio::send`].
#[derive(Debug)]
pub struct Send<'a>
{
	_pd: std::marker::PhantomData<&'a ()>,
}

/// A future created by calling [`Aio::recv`].
#[derive(Debug)]
pub struct Recv<'a>
{
	_pd: std::marker::PhantomData<&'a ()>,
}

/// The inner state of the AIO callback.
#[derive(Debug)]
struct State
{
	/// The pointer to the underlying AIO object.
	
}
