use std::{
    cmp::{Eq, Ord, Ordering, PartialEq, PartialOrd},
    hash::{Hash, Hasher},
};

use nng_sys::nng_err;

use crate::{dialer::Dialer, listener::Listener};

/// An NNG communication pipe.
///
/// A pipe can be thought of as a single connection and are associated with
/// either the listener or dialer that created them. Therefore, they are
/// automatically associated with a single socket.
///
/// Most applications should never concern themselves with individual pipes.
/// However, it is possible to access a pipe when more information about the
/// source of the message is needed or when more control is required over
/// message delivery.
///
/// See the [NNG documentation][1] for more information.
///
/// [1]: https://nanomsg.github.io/nng/man/v1.2.2/nng_pipe.5
#[derive(Clone, Copy, Debug)]
pub struct Pipe {
    /// The underlying NNG pipe.
    handle: nng_sys::nng_pipe,
}
impl Pipe {
    /// Returns the dialer associated with this pipe, if any.
    pub fn dialer(self) -> Option<Dialer> {
        let (dialer, id) = unsafe {
            let dialer = nng_sys::nng_pipe_dialer(self.handle);
            let id = nng_sys::nng_dialer_id(dialer);
            (dialer, id)
        };

        if id > 0 {
            Some(Dialer::from_nng_sys(dialer))
        } else {
            None
        }
    }

    /// Returns the listener associated with this pipe, if any.
    pub fn listener(self) -> Option<Listener> {
        let (listener, id) = unsafe {
            let listener = nng_sys::nng_pipe_listener(self.handle);
            let id = nng_sys::nng_listener_id(listener);
            (listener, id)
        };

        if id > 0 {
            Some(Listener::from_nng_sys(listener))
        } else {
            None
        }
    }

    /// Closes the pipe.
    ///
    /// Messages that have been submitted for sending may be flushed or
    /// delivered, depending upon the transport and the linger option. Pipe are
    /// automatically closed when their creator closes or when the remote peer
    /// closes the underlying connection.
    #[allow(clippy::missing_panics_doc)]
    pub fn close(self) {
        // The pipe either closes succesfully, was already closed, or was never open. In
        // any of those scenarios, the pipe is in the desired state. As such, we don't
        // care about the return value.
        let nng_err(rv) = unsafe { nng_sys::nng_pipe_close(self.handle) };
        assert!(
            rv == 0 || nng_err(rv) == nng_sys::nng_err::NNG_ECLOSED,
            "Unexpected error code while closing pipe ({})",
            rv
        );
    }

    /// Returns the underlying NNG handle for the pipe.
    pub(crate) const fn handle(self) -> nng_sys::nng_pipe {
        self.handle
    }

    /// Create a new Pipe handle from a NNG handle.
    ///
    /// This function will panic if the handle is not valid.
    pub(crate) fn from_nng_sys(handle: nng_sys::nng_pipe) -> Self {
        assert!(
            unsafe { nng_sys::nng_pipe_id(handle) > 0 },
            "Pipe handle is not initialized"
        );
        Pipe { handle }
    }
}

#[cfg(feature = "ffi-module")]
impl Pipe {
    /// Returns the handle to the underlying `nng_pipe` object.
    pub fn nng_pipe(self) -> nng_sys::nng_pipe {
        self.handle()
    }
}

impl PartialEq for Pipe {
    fn eq(&self, other: &Pipe) -> bool {
        unsafe { nng_sys::nng_pipe_id(self.handle) == nng_sys::nng_pipe_id(other.handle) }
    }
}

impl Eq for Pipe {}

impl PartialOrd for Pipe {
    fn partial_cmp(&self, other: &Pipe) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Pipe {
    fn cmp(&self, other: &Pipe) -> Ordering {
        unsafe {
            let us = nng_sys::nng_pipe_id(self.handle);
            let them = nng_sys::nng_pipe_id(other.handle);
            us.cmp(&them)
        }
    }
}

impl Hash for Pipe {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let id = unsafe { nng_sys::nng_pipe_id(self.handle) };
        id.hash(state);
    }
}

/// An event that happens on a [`Pipe`] instance.
///
///
/// [`Pipe`]: struct.Pipe.html
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PipeEvent {
    /// Occurs after a connection and negotiation has completed but before the
    /// pipe is added to the socket.
    ///
    /// If the pipe is closed at this point, the socket will never see the pipe
    /// and no further events will occur for the given pipe.
    AddPre,

    /// This event occurs after the pipe is fully added to the socket.
    ///
    /// Prior to this time, it is not possible to communicate over the pipe with
    /// the socket.
    AddPost,

    /// Occurs after the pipe has been removed from the socket.
    ///
    /// The underlying transport may be closed at this point and it is not
    /// possible to communicate with this pipe.
    RemovePost,

    /// An unknown event.
    ///
    /// Should never happen - used for forward compatibility.
    #[doc(hidden)]
    Unknown(u32),
}
impl PipeEvent {
    /// Converts the NNG code into a `PipeEvent`.
    pub(crate) fn from_code(event: nng_sys::nng_pipe_ev) -> Self {
        match event {
            nng_sys::NNG_PIPE_EV_ADD_PRE => PipeEvent::AddPre,
            nng_sys::NNG_PIPE_EV_ADD_POST => PipeEvent::AddPost,
            nng_sys::NNG_PIPE_EV_REM_POST => PipeEvent::RemovePost,
            _ => PipeEvent::Unknown(event),
        }
    }
}
