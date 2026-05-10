//! A safe Rust wrapper for NNG
//!
//! ## What Is NNG
//!
//! From the [NNG Github Repository][1]:
//!
//! > NNG, like its predecessors nanomsg (and to some extent ZeroMQ), is a
//! > lightweight, broker-less library, offering a simple API to solve common
//! > recurring messaging problems, such as publish/subscribe, RPC-style
//! > request/reply, or service discovery. The API frees the programmer
//! > from worrying about details like connection management, retries, and other
//! > common considerations, so that they can focus on the application instead of
//! > the plumbing.
//!
//! ## Nng-rs
//!
//! This crate provides a safe wrapper around the NNG library, seeking to
//! maintain an API that is similar to the original library. As such, the
//! majority of examples available online should be easy to apply to this crate.
//!
//! ### Rust Version Requirements
//!
//! The current version requires **Rustc v1.36 or greater**. In general, this
//! crate should always be able to compile with the Rustc version available on
//! the oldest currently-supported Ubuntu LTS release. Changes to the minimum
//! required Rustc version will only be considered a breaking change if the
//! newly required version is not available on the oldest currently-supported
//! Ubuntu LTS release.
//!
//! **NOTE:** This does not necessarily mean that this crate will build without
//! installing packages on Ubuntu LTS, as NNG currently requires a version of
//! CMake (v3.13) that is newer than the one available in the LTS repositories.
//!
//! ### Features
//!
//! * `build-nng` (default): Build NNG from source and statically link to the
//!   library.
//! * `ffi-module`: Expose the raw FFI bindings via the `nng::ffi` module. This
//!   is useful for utilizing NNG features that are implemented in the base
//!   library but not this wrapper. Note that this exposes some internal items
//!   of this library and it directly exposes the NNG library, so anything
//!   enabled by this can change without bumping versions.
//!
//! ### Building NNG
//!
//! Enabling the `build-nng` feature will cause the NNG library to be built
//! using the default settings and CMake generator.  Most of the time, this
//! should just work.  However, in the case that the default are not the desired
//! settings, there are three ways to change the build:
//!
//! 1. [Patch][5] the `nng-sys` dependency and enable the desired build features.
//! 2. Disable the `build-nng` feature and directly depend on `nng-sys`. 3. Disable
//!    the `build-nng` feature and manually compile NNG.
//!
//! The build features are not exposed in this crate because Cargo features are
//! currently [strictly additive][6] and there is no way to specify mutually
//! exclusive features (i.e., build settings).  Additionally, it does not seem
//! very ergonomic to have this crate expose all of the same build features as
//! the binding crate, which could lead to feature pollution in any dependent
//! crates.
//!
//! Merge requests for a better solution to this are more than welcome.
//!
//! ### Examples
//!
//! The following example uses the [intra-process][2] transport to set up a
//! [request][3]/[reply][4] socket pair. The "client" sends a string to the
//! "server" which responds with a nice phrase.
//!
//! ```
//! use nng::*;
//!
//! const ADDRESS: &'static str = "inproc://nng/example";
//!
//! fn request() -> Result<()> {
//!     // Set up the client and connect to the specified address
//!     let client = Socket::new(Protocol::Req0)?;
//!     # // Don't error if we hit here before the server does.
//!     # client.dial_async(ADDRESS)?;
//!     # if false {
//!     client.dial(ADDRESS)?;
//!     # }
//!
//!     // Send the request from the client to the server. In general, it will be
//!     // better to directly use a `Message` to enable zero-copy, but that doesn't
//!     // matter here.
//!     client.send("Ferris".as_bytes())?;
//!
//!     // Wait for the response from the server.
//!     let msg = client.recv()?;
//!     assert_eq!(&msg[..], b"Hello, Ferris");
//!     Ok(())
//! }
//!
//! fn reply() -> Result<()> {
//!     // Set up the server and listen for connections on the specified address.
//!     let server = Socket::new(Protocol::Rep0)?;
//!     server.listen(ADDRESS)?;
//!
//!     // Receive the message from the client.
//!     let mut msg = server.recv()?;
//!     assert_eq!(&msg[..], b"Ferris");
//!
//!     // Reuse the message to be more efficient.
//!     msg.push_front(b"Hello, ");
//!
//!     server.send(msg)?;
//!     Ok(())
//! }
//!
//! # // Start the server first, so the client can connect to it.
//! # let jh = std::thread::spawn(|| reply().unwrap());
//! # request().unwrap();
//! # jh.join().unwrap();
//! ```
//!
//! Additional examples are in the `examples` directory.
//!
//! [1]: https://github.com/nanomsg/nng
//! [2]: https://nanomsg.github.io/nng/man/v1.2.2/nng_inproc.7
//! [3]: https://nanomsg.github.io/nng/man/v1.2.2/nng_req.7
//! [4]: https://nanomsg.github.io/nng/man/v1.2.2/nng_rep.7
//! [5]: https://doc.rust-lang.org/cargo/reference/manifest.html#the-patch-section
//! [6]: https://github.com/rust-lang/cargo/issues/2980
use std::num::NonZeroU16;

#[macro_use]
mod util;

mod addr;
mod aio;
mod ctx;
mod device;
mod dialer;
mod error;
mod listener;
mod message;
mod pipe;
mod protocol;
mod socket;

#[cfg(feature = "ffi-module")]
/// Raw NNG foreign function interface.
pub use nng_sys as ffi;

#[cfg(feature = "ffi-module")]
pub use crate::aio::State as AioState;
pub use crate::{
    addr::SocketAddr,
    aio::{Aio, AioResult},
    ctx::Context,
    device::{forwarder, reflector},
    dialer::{Dialer, DialerBuilder},
    error::{Error, Result},
    listener::{Listener, ListenerBuilder},
    message::{Header, Message},
    pipe::{Pipe, PipeEvent},
    protocol::Protocol,
    socket::{RawSocket, Socket},
};

/// A handle to the NNG resources.
///
/// This type can be used to configure the parameters of NNG resources or to
/// control the life of said resources. Every [`Socket`] has an implicit handle
/// to those resources and will create it with the default values. If one of
/// these is created before any sockets, it can be used to set runtime tunables
/// for NNG.
#[non_exhaustive]
#[derive(Debug, Default)]
pub struct Params {
    /// The number of threads to use for tasks.
    ///
    /// These tasks are principally for callback completion. This number cannot
    /// exceed [`Init::max_task_threads`].
    pub num_task_threads: Option<NonZeroU16>,

    /// The maximum number of threads to use for tasks.
    ///
    /// These tasks are principally used for callback completion. This value can
    /// be used to provide an upper limit while still allowing the number of
    /// task threads to be dynamically calculated.
    pub max_task_threads: Option<NonZeroU16>,

    /// The number of threads used for expiring operations.
    ///
    /// Using a larger value will reduce contention on some common locks, and
    /// may improve performance. This number cannot exceed
    /// [`Init::max_expire_threads`].
    pub num_expire_threads: Option<NonZeroU16>,

    /// The maximum number of threads used for expiring operations.
    ///
    /// Using a larger value will reduce contention on some common locks, and
    /// may improve performance. This can be used to provide an upper limit
    /// while still allowing a dynamic count.
    pub max_expire_threads: Option<NonZeroU16>,

    /// The number of threads to use for performing I/O.
    ///
    /// Not all configurations will support this. Cannot exceed
    /// [`Init::max_poller_threads`].
    pub num_poller_threads: Option<NonZeroU16>,

    /// The maximum number of threads to use for performing I/O.
    ///
    /// Not all configurations will support this. This allows an upper limit on
    /// the number of polling threads while still allowing the count to be
    /// dynamically calculated.
    pub max_poller_threads: Option<NonZeroU16>,

    /// The number of threads used for asynchronous DNS lookup.
    pub num_resolver_threads: Option<NonZeroU16>,
}

impl Params {
    /// Initializes NNG with the specified tunables.
    ///
    /// This initializes NNG with the provided parameters and will prevent NNG
    /// from releasing certain resources until after the provided handle is
    /// dropped. Each [`Socket`] implicitly has their own handle and the
    /// resources will not be released until all [`Socket`] objects are dropped
    /// as well.
    ///
    /// # Errors
    ///
    /// If NNG has already been initialized, this will return [`Error::Busy`].
    pub fn init(self) -> Result<impl Drop> {
        let params = self.into();

        // SAFETY: We know that this is a valid pointer.
        let rv = unsafe { nng_sys::nng_init(&params) };

        struct Defer;
        impl Drop for Defer {
            fn drop(&mut self) {
                // SAFETY: We did the initialization above.
                unsafe { nng_sys::nng_fini() }
            }
        }

        rv2res!(rv.0, Defer)
    }
}

impl From<Params> for nng_sys::nng_init_params {
    fn from(params: Params) -> Self {
        let convert = |n: NonZeroU16| n.get() as i16;
        nng_sys::nng_init_params {
            num_task_threads: params.num_task_threads.map_or(0, convert),
            max_task_threads: params.max_task_threads.map_or(0, convert),
            num_expire_threads: params.num_expire_threads.map_or(0, convert),
            max_expire_threads: params.max_expire_threads.map_or(0, convert),
            num_poller_threads: params.num_poller_threads.map_or(0, convert),
            max_poller_threads: params.max_poller_threads.map_or(0, convert),
            num_resolver_threads: params.num_resolver_threads.map_or(0, convert),
        }
    }
}
