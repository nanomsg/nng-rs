//! A safe Rust wrapper for NNG
//!
//! ## What Is NNG
//!
//! From the [NNG Github Repository][1]:
//!
//! > NNG, like its predecessors nanomsg (and to some extent ZeroMQ), is a
//! lightweight, broker-less library, offering a simple API to solve common
//! recurring messaging problems, such as publish/subscribe, RPC-style
//! request/reply, or service discovery. The API frees the programmer
//! from worrying about details like connection management, retries, and other
//! common considerations, so that they can focus on the application instead of
//! the plumbing.
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

// The following lints are of critical importance.
#![forbid(improper_ctypes)]
// Utilize Clippy to try and keep this crate clean. At some point (cargo#5034, I think?) this
// specification should be possible in either the Clippy TOML file or in the Cargo TOML file. These
// should be moved there once possible.
#![deny(bare_trait_objects)]
#![deny(missing_debug_implementations)]
#![deny(missing_docs)]
#![deny(clippy::all)]
#![deny(clippy::wrong_self_convention)]
// Clippy doesn't enable these with "all". Best to keep them warnings.
#![warn(clippy::nursery)]
#![warn(clippy::pedantic)]
#![warn(clippy::cargo)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::decimal_literal_representation)]
#![warn(clippy::print_stdout)]
#![warn(clippy::unimplemented)]
#![warn(clippy::use_debug)]
// I would like to be able to keep these on, but due to the nature of the crate it just isn't
// feasible. For example, the "cast_sign_loss" will warn at every i32/u32 conversion. Normally, I
// would like that, but this library is a safe wrapper around a Bindgen-based binding of a C
// library, which means the types are a little bit up-in-the-air.
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::empty_enum)] // Revisit after RFC1861 and RFC1216.
#![allow(clippy::cargo_common_metadata)] // Can't control this.
#![allow(clippy::module_name_repetitions)] // Doesn't recognize public re-exports.
#![allow(clippy::cast_possible_wrap)]
// I want to enable this but it requires bumping the Rustc version and I don't want to do that just
// for a clippy lint.
#![allow(clippy::ptr_as_ptr)]
// In these cases, I just don't like what Clippy suggests.
#![allow(clippy::use_self)]
#![allow(clippy::if_not_else)]
#![allow(clippy::must_use_candidate)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::option_if_let_else)] // Semantically backwards when used with non-zero error codes
#![allow(clippy::wildcard_imports)] // I don't generally like them either but can be used well
#![allow(clippy::enum_glob_use)] // Same as wildcards
#![allow(clippy::manual_non_exhaustive)] // Not available in v1.36

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

pub mod options;

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
