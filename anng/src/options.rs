//! Internal helpers for setting NNG options on sockets, contexts, dialers, and listeners.
//!
//! Each helper centralizes the bounds check on the value, the unsafe FFI call, and the
//! `unreachable!` arms for errnos that NNG documents but that callers in this crate are
//! statically responsible for never triggering. Callers in the protocol modules pass the
//! `nng_sys::NNG_OPT_*` byte slice and a human-readable label.
//!
//! ## Future maintenance
//!
//! Today only `_set_ms` exists, in plain (`set_socket_ms`) and optional/infinite-aware
//! (`set_socket_ms_opt`, `set_ctx_ms_opt`) flavours that share most of their bodies. When a
//! second value type lands (`_set_int`, `_set_bool`, `_set_size`, `_set_string`), do **not** keep
//! copy-pasting: that's the moment to extract a private trait over the handle (e.g. `trait
//! OptionTarget { fn set_ms(...); fn set_int(...); ... }`) with impls for `nng_socket` and
//! `nng_ctx`, and rewrite the helpers as a single generic over the trait. Doing it now (with one
//! value type) would be premature; doing it after the third would be painful.

use core::{
    ffi::{CStr, c_int},
    time::Duration,
};
use nng_sys::{ErrorCode, ErrorKind};
use std::io;

/// Sets a `nng_duration` (ms) option on a socket, panicking on any NNG-reported error.
///
/// Every `nng_socket_set_ms` failure is a programming error at this layer (closed handle, unknown
/// option, read-only option, invalid duration, or any errno NNG hasn't documented). The caller is
/// statically responsible for pairing a valid socket with a valid option supported by the
/// socket's protocol; the `label` argument is a human-readable name used in the panic messages
/// (e.g. `"resend time"`).
///
/// Note: NNG defines sentinel duration values (`NNG_DURATION_INFINITE`, `NNG_DURATION_DEFAULT`)
/// as negative numbers. They cannot be expressed via [`Duration`] and are deliberately not
/// supported by this helper — protocol-level helpers should expose them through their own API
/// surface if needed.
pub(crate) fn set_socket_ms(
    socket: nng_sys::nng_socket,
    option: &CStr,
    duration: Duration,
    label: &str,
) -> io::Result<()> {
    let ms = duration_to_nng_ms(duration, label);
    // SAFETY: caller guarantees the socket is valid and supports the named option;
    //         `option` is a borrowed `&CStr`, so its `.as_ptr()` is valid for the FFI call.
    let raw_errno = unsafe { nng_sys::nng_socket_set_ms(socket, option.as_ptr(), ms) };
    let errno = u32::try_from(raw_errno).expect("errno is never negative");
    check_map_set_ms_errno(errno, label, "nng_socket_set_ms")
}

/// Sets a `nng_duration` (ms) option on a socket, mapping "no duration" to the NNG
/// `NNG_DURATION_INFINITE` sentinel.
///
/// This is the [`set_socket_ms`] variant for options that distinguish "set a finite duration"
/// from "disable the duration-driven behaviour". `None` (and `Some(Duration::ZERO)`) map to
/// `NNG_DURATION_INFINITE`, which NNG treats as "no timer"; a positive `Some(duration)` maps to
/// that many milliseconds. See [`set_socket_ms`] for the shared invariants and panic conditions.
pub(crate) fn set_socket_ms_opt(
    socket: nng_sys::nng_socket,
    option: &CStr,
    duration: Option<Duration>,
    label: &str,
) -> io::Result<()> {
    let ms = duration_opt_to_nng_ms(duration, label);
    // SAFETY: caller guarantees the socket is valid and supports the named option;
    //         `option` is a borrowed `&CStr`, so its `.as_ptr()` is valid for the FFI call.
    let raw_errno = unsafe { nng_sys::nng_socket_set_ms(socket, option.as_ptr(), ms) };
    let errno = u32::try_from(raw_errno).expect("errno is never negative");
    check_map_set_ms_errno(errno, label, "nng_socket_set_ms")
}

/// Sets a `nng_duration` (ms) option on a context, mapping "no duration" to the NNG
/// `NNG_DURATION_INFINITE` sentinel. See [`set_socket_ms_opt`] for semantics.
pub(crate) fn set_ctx_ms_opt(
    ctx: nng_sys::nng_ctx,
    option: &CStr,
    duration: Option<Duration>,
    label: &str,
) -> io::Result<()> {
    let ms = duration_opt_to_nng_ms(duration, label);
    // SAFETY: caller guarantees the context is valid and supports the named option;
    //         `option` is a borrowed `&CStr`, so its `.as_ptr()` is valid for the FFI call.
    let raw_errno = unsafe { nng_sys::nng_ctx_set_ms(ctx, option.as_ptr(), ms) };
    let errno = u32::try_from(raw_errno).expect("errno is never negative");
    check_map_set_ms_errno(errno, label, "nng_ctx_set_ms")
}

/// Wraps a nul-terminated `nng_sys::NNG_OPT_*` byte constant as a `&'static CStr`.
pub(crate) const fn opt(name: &'static [u8]) -> &'static CStr {
    match CStr::from_bytes_with_nul(name) {
        Ok(c) => c,
        Err(_) => panic!("nng option name is not a valid C string"),
    }
}

fn duration_to_nng_ms(duration: Duration, label: &str) -> c_int {
    let ms = duration.as_millis();
    if ms > i32::MAX as u128 {
        panic!("{label} is too large: {duration:?}");
    }
    ms as c_int
}

/// Converts an optional duration to a `nng_duration`, mapping the "disabled" cases to
/// `NNG_DURATION_INFINITE`.
///
/// `None` means "disable the duration-driven behaviour" and maps to `NNG_DURATION_INFINITE`.
/// `Some(Duration::ZERO)` is treated the same way: a literal `0` would tell NNG to act with a
/// zero-length timer, which is almost never what a caller asking for "zero" actually wants, so we
/// fold it into the infinite sentinel as well. Any positive duration is bounds-checked and
/// converted to milliseconds by [`duration_to_nng_ms`].
fn duration_opt_to_nng_ms(duration: Option<Duration>, label: &str) -> c_int {
    match duration {
        None | Some(Duration::ZERO) => nng_sys::NNG_DURATION_INFINITE,
        Some(duration) => duration_to_nng_ms(duration, label),
    }
}

fn check_map_set_ms_errno(errno: u32, label: &str, fn_name: &str) -> io::Result<()> {
    match errno {
        0 => Ok(()),
        e if e == ErrorCode::ECLOSED as u32 => {
            unreachable!("{fn_name}({label}): NNG returned ECLOSED on a borrowed, live handle");
        }
        e if e == ErrorCode::EINVAL as u32 => {
            unreachable!(
                "{fn_name}({label}): NNG returned EINVAL despite a pre-validated duration and \
                 a static option name"
            );
        }
        e if e == ErrorCode::ENOTSUP as u32 => {
            unreachable!(
                "{fn_name}({label}): NNG returned ENOTSUP — option/protocol pairing is wrong \
                 in the calling protocol module"
            );
        }
        e if e == ErrorCode::EREADONLY as u32 => {
            unreachable!(
                "{fn_name}({label}): NNG returned EREADONLY — a read-only ms option was routed \
                 through a helper that assumes always-mutable options"
            );
        }
        e if e == ErrorCode::ESTATE as u32 => {
            Err(io::Error::other(ErrorKind::NngError(ErrorCode::ESTATE)))
        }
        _ => {
            // Any other errno is undocumented for `nng_socket_set_ms` / `nng_ctx_set_ms`.
            // Most likely cause: NNG added a new failure mode in a newer release. Re-check the
            // current `nng_*_set_ms` docs and add an explicit arm above for the new errno.
            unreachable!(
                "{fn_name}({label}) returned errno {errno}, which is not documented for \
                 `nng_*_set_ms` — NNG behaviour likely changed; update this match"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_to_nng_ms_accepts_zero() {
        assert_eq!(duration_to_nng_ms(Duration::ZERO, "test"), 0);
    }

    #[test]
    fn duration_to_nng_ms_accepts_max_i32() {
        assert_eq!(
            duration_to_nng_ms(Duration::from_millis(i32::MAX as u64), "test"),
            i32::MAX,
        );
    }

    #[test]
    #[should_panic(expected = "test is too large")]
    fn duration_to_nng_ms_rejects_overflow() {
        // Duration::MAX is the worst case a caller can hand us; it must be caught.
        duration_to_nng_ms(Duration::MAX, "test");
    }

    #[test]
    fn duration_opt_none_maps_to_infinite() {
        assert_eq!(
            duration_opt_to_nng_ms(None, "test"),
            nng_sys::NNG_DURATION_INFINITE,
        );
    }

    #[test]
    fn duration_opt_zero_maps_to_infinite() {
        assert_eq!(
            duration_opt_to_nng_ms(Some(Duration::ZERO), "test"),
            nng_sys::NNG_DURATION_INFINITE,
        );
    }

    #[test]
    fn duration_opt_positive_maps_to_ms() {
        assert_eq!(
            duration_opt_to_nng_ms(Some(Duration::from_millis(50)), "test"),
            50,
        );
    }

    #[test]
    #[should_panic(expected = "test is too large")]
    fn duration_opt_rejects_overflow() {
        duration_opt_to_nng_ms(Some(Duration::MAX), "test");
    }
}
