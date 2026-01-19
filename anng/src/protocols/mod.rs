//! Protocol implementations for NNG scalability patterns.
//!
//! This module contains implementations of the various messaging patterns
//! supported by NNG. Each protocol implements specific communication semantics
//! and is designed for particular use cases.
//!
//! # Connection patterns
//!
//! This crate supports flexible connection patterns including multi-dial and multi-listener
//! scenarios. Sockets can have multiple dialers and listeners simultaneously, and the direction
//! of connection establishment (dial vs listen) is independent of the protocol type.

/// Marker trait for protocols that support contexts.
///
/// NNG protocols are divided into two categories:
/// - **Stateful protocols** that maintain per-operation state and support contexts
/// - **Stateless protocols** that have no per-operation state and do not support contexts
///
/// This trait is automatically implemented for protocols that support contexts,
/// enabling compile-time verification that `Socket::context()` is only called
/// on compatible protocols.
///
/// ## Context support by protocol:
///
/// **✓ Supports Contexts (Stateful):**
/// - `REQ0` - Request
/// - `REP0` - Reply
/// - `SUB0` - Subscribe
/// - `SURVEYOR0` - Surveyor
/// - `RESPONDENT0` - Respondent
///
/// **✗ No Context Support (Stateless):**
/// - `PUB0` - Publish
/// - `PUSH0` - Push
/// - `PULL0` - Pull
/// - `BUS0` - Bus
/// - `PAIR1` - Pair
pub trait SupportsContext {}

use crate::aio::Aio;
use crate::{AioError, Socket};
use core::mem::MaybeUninit;
use core::{
    ffi::{CStr, c_int},
    marker::PhantomData,
};
use nng_sys::nng_err;
use std::io;
use std::sync::Once;

static NNG_INIT: Once = Once::new();

/// Ensures NNG is initialized before any operations.
/// This is required by NNG 2.0 and must be called before using any NNG functions.
fn ensure_nng_initialized() {
    NNG_INIT.call_once(|| {
        // SAFETY: nng_init is safe to call once before any NNG operations.
        // Passing null uses NNG's default thread pool configuration.
        let result = unsafe { nng_sys::nng_init(std::ptr::null()) };
        if result != nng_err::NNG_OK {
            panic!("Failed to initialize NNG: {:?}", result);
        }
    });
}

pub mod bus0;
pub mod pair1;
pub mod pipeline0;
pub mod pubsub0;
pub mod reqrep0;
pub mod survey0;

/// Creates a new socket for the given protocol.
///
/// # Safety
///
/// `nng_proto_open` must be a valid nng socket initialization function, and must correspond to the
/// type indicated in `Protocol` as far as subsequent uses of the socket are concerned.
pub(crate) unsafe fn create_socket<Protocol: core::fmt::Debug>(
    nng_proto_open: unsafe extern "C" fn(*mut nng_sys::nng_socket) -> c_int,
    proto: Protocol,
) -> io::Result<Socket<Protocol>> {
    // ensure NNG is initialized before creating any sockets (required by NNG 2.0)
    ensure_nng_initialized();

    let mut socket = MaybeUninit::<nng_sys::nng_socket>::uninit();
    // SAFETY: socket pointer is valid for writing.
    let errno = unsafe { nng_proto_open(socket.as_mut_ptr()) };
    match u32::try_from(errno).expect("errno is never negative") {
        0 => {}
        x if x == nng_err::NNG_ENOMEM as u32 => {
            panic!("OOM");
        }
        x if x == nng_err::NNG_ENOTSUP as u32 => {
            unreachable!("{proto:?} is listed as an unsupported protocol");
        }
        errno => {
            unreachable!("nng_{proto:?}_open documentation claims errno {errno} is never returned");
        }
    }
    // SAFETY: nng_proto_open initializes socket on success.
    let socket = unsafe { socket.assume_init() };

    Ok(Socket {
        socket,
        aio: crate::aio::Aio::new(),
        recovered_msg: None,
        protocol: PhantomData::<Protocol>,
    })
}

/// Adds a listener to an existing socket.
pub(crate) async fn add_listener_to_socket(
    socket: nng_sys::nng_socket,
    url: &CStr,
    pre_start: impl FnOnce(nng_sys::nng_listener) -> io::Result<()>,
) -> io::Result<nng_sys::nng_listener> {
    let mut listener = MaybeUninit::<nng_sys::nng_listener>::uninit();

    // SAFETY: listener pointer is valid for writing, socket is valid, addr is valid C string.
    let errno =
        unsafe { nng_sys::nng_listener_create(listener.as_mut_ptr(), socket, url.as_ptr()) };
    match u32::try_from(errno).expect("errno is never negative") {
        0 => {}
        x if x == nng_err::NNG_ENOMEM as u32 => {
            panic!("OOM");
        }
        x if x == nng_err::NNG_ECLOSED as u32
            || x == nng_err::NNG_EADDRINVAL as u32
            || x == nng_err::NNG_EINVAL as u32
            || x == nng_err::NNG_ENOTSUP as u32 =>
        {
            return Err(AioError::try_from_i32(errno)
                .expect_err("checked above")
                .into());
        }
        errno => {
            unreachable!(
                "nng_listener_create documentation claims errno {errno} is never returned"
            );
        }
    }
    // SAFETY: nng_listen initializes listener on success.
    let listener = unsafe { listener.assume_init() };

    if let Err(e) = pre_start(listener) {
        let errno = unsafe { nng_sys::nng_listener_close(listener) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("the listener handle is valid");
            }
            errno => {
                unreachable!(
                    "nng_listener_close documentation claims errno {errno} is never returned"
                );
            }
        }
        return Err(e);
    }

    let errno = unsafe { nng_sys::nng_listener_start(listener, 0) };
    match u32::try_from(errno).expect("errno is never negative") {
        0 => {}
        x if x == nng_err::NNG_ECLOSED as u32 => {
            unreachable!("the listener handle is valid");
        }
        x if x == nng_err::NNG_ESTATE as u32 => {
            unreachable!("the listener is not already started");
        }
        x if x == nng_err::NNG_EADDRINUSE as u32 || x == nng_err::NNG_EPERM as u32 => {
            return Err(AioError::try_from_i32(errno)
                .expect_err("checked above")
                .into());
        }
        errno => {
            unreachable!("nng_listener_start documentation claims errno {errno} is never returned");
        }
    }

    Ok(listener)
}

/// Adds a dialer to an existing socket.
///
/// # Safety
///
/// The socket must be a valid, open NNG socket.
pub(crate) async fn add_dialer_to_socket(
    socket: nng_sys::nng_socket,
    url: &CStr,
    pre_start: impl FnOnce(nng_sys::nng_dialer) -> io::Result<()>,
) -> io::Result<nng_sys::nng_dialer> {
    // before we dial, register a callback for when the dialer completes
    // (since we're going to be dialing with the nonblock flag to avoid blocking).
    let mut _aio = Aio::new();

    let mut dialer = MaybeUninit::<nng_sys::nng_dialer>::uninit();

    // SAFETY: dialer pointer is valid for writing, socket is valid, and addr is valid C string.
    let errno = unsafe { nng_sys::nng_dialer_create(dialer.as_mut_ptr(), socket, url.as_ptr()) };
    match u32::try_from(errno).expect("errno is never negative") {
        0 => {}
        x if x == nng_err::NNG_ENOMEM as u32 => {
            panic!("OOM");
        }
        x if x == nng_err::NNG_ECLOSED as u32
            || x == nng_err::NNG_EADDRINVAL as u32
            || x == nng_err::NNG_EINVAL as u32
            || x == nng_err::NNG_ENOTSUP as u32 =>
        {
            return Err(AioError::try_from_i32(errno)
                .expect_err("checked above")
                .into());
        }
        errno => {
            unreachable!("nng_dialer_create documentation claims errno {errno} is never returned");
        }
    }
    // SAFETY: nng_dialer_create initializes dialer on success.
    let dialer = unsafe { dialer.assume_init() };

    if let Err(e) = pre_start(dialer) {
        let errno = unsafe { nng_sys::nng_dialer_close(dialer) };
        match u32::try_from(errno).expect("errno is never negative") {
            0 => {}
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("the dialer handle is valid");
            }
            errno => {
                unreachable!(
                    "nng_dialer_close documentation claims errno {errno} is never returned"
                );
            }
        }
        return Err(e);
    }

    // until we get nng_dialer_start_aio from https://github.com/nanomsg/nng/pull/2163, there is no
    // way to dial asynchronously (with nng_sys::NNG_FLAG_NONBLOCK as i32) _and_ learn the result
    // of that dial, so we use a synchronous dial in a `spawn_blocking`.
    //
    // TODO: replace this with nng_dialer_start_aio once we have it, and then also remove the "rt"
    // feature from the tokio dependency.
    let handle = tokio::task::spawn_blocking(move || {
        // SAFETY: dialer is valid.
        let errno = unsafe { nng_sys::nng_dialer_start(dialer, 0) };
        // NOTE(jon): when we eventually dial with the nonblocking flag and start_aio, most of
        // these errors cannot happen _now_, they'd happen later when the dialer _actually_ does
        // the dialing.
        match u32::try_from(errno).expect("errno is never negative") {
            0 => Ok(()),
            x if x == nng_err::NNG_ECLOSED as u32 => {
                unreachable!("the socket is still valid");
            }
            x if x == nng_err::NNG_ESTATE as u32 => {
                unreachable!("the dialer has not been started");
            }
            x if x == nng_err::NNG_ECANCELED as u32 => {
                // this can happen if the dial future is dropped (such as if the future is
                // cancelled), and that _also_ drops the referenced socket. if this occurrs, any
                // I/O operation on the socket is cancelled by nng, and thus we get that error.
                // we don't need to _do_ anything with it though, since we _know_ the caller has
                // gone away (and thus doesn't care about our return value).
                tracing::warn!("socket dropped while (now-cancelled) dial future still running");
                Err(io::Error::from(AioError::Cancelled))
            }
            x if x == nng_err::NNG_EAGAIN as u32 => {
                // this is returned from `getaddrinfo` if there's a temporary failure in name
                // resolution, such as in a nix build jail where the DNS resolver is specifically
                // configured to fail. this _should_ be caught and translated by NNG, but isn't at
                // the time of writing. the maintainer has confirmed that this behaviour _should_
                // change to return NNG_EADDRINVAL instead:
                //
                //   <https://discord.com/channels/639573728212156478/639574541743423491/1422961135652438149>
                //   <https://github.com/nanomsg/nng/blob/f716f61c81a5f120d61b58ee9b4a52b33b2ecb16/src/platform/posix/posix_resolv_gai.c#L118-L121>
                //
                // so we remap ourselves for the time being.
                Err(io::Error::from(AioError::from_nng_err(
                    nng_err::NNG_EADDRINVAL,
                )))
            }
            x if x == nng_err::NNG_ENOMEM as u32 => {
                panic!("OOM");
            }
            x if x == nng_err::NNG_EADDRINVAL as u32
                || x == nng_err::NNG_ECONNREFUSED as u32
                || x == nng_err::NNG_ECONNRESET as u32
                || x == nng_err::NNG_EINVAL as u32
                || x == nng_err::NNG_EPEERAUTH as u32
                || x == nng_err::NNG_EPROTO as u32
                || x == nng_err::NNG_EUNREACHABLE as u32 =>
            {
                Err(AioError::try_from_i32(errno)
                    .expect_err("checked above")
                    .into())
            }
            errno => {
                unreachable!(
                    "nng_dialer_start documentation claims errno {errno} is never returned"
                );
            }
        }
    });

    match handle.await {
        Ok(Ok(())) => {}
        e => {
            // if we failed to start, make sure the dialer is properly closed.
            // SAFETY: dialer is valid.
            unsafe { nng_sys::nng_dialer_close(dialer) };
            e??;
        }
    }

    // NOTE(jon): it is technically possible that the aio callback is still executing, but that's
    // okay -- while the callback runs, nng holds a lock on the socket on its behalf, so we can't
    // run into anything sad by using the socket now. it's just the callback itself that's not
    // allowed to try and touch the socket (which it doesn't).

    Ok(dialer)
}
