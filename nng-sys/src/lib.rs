/*!

## Examples

```rust
use nng_sys::*;
use std::{ffi::CString, os::raw::c_char, ptr::null_mut};

fn example() {
    unsafe {
        nng_init(null_mut());

        let url = CString::new("inproc://nng_sys/tests/example").unwrap();
        let url = url.as_bytes_with_nul().as_ptr() as *const c_char;

        // Reply socket
        let mut rep_socket = nng_socket::default();
        nng_rep0_open(&mut rep_socket);
        nng_listen(rep_socket, url, null_mut(), 0);

        // Request socket
        let mut req_socket = nng_socket::default();
        nng_req0_open(&mut req_socket);
        nng_dial(req_socket, url, null_mut(), 0);

        // Send message
        let mut req_msg: *mut nng_msg = null_mut();
        nng_msg_alloc(&mut req_msg, 0);
        // Add a value to the body of the message
        let val = 0x12345678;
        nng_msg_append_u32(req_msg, val);
        nng_sendmsg(req_socket, req_msg, 0);

        // Receive it
        let mut recv_msg: *mut nng_msg = null_mut();
        nng_recvmsg(rep_socket, &mut recv_msg, 0);
        // Remove our value from the body of the received message
        let mut recv_val: u32 = 0;
        nng_msg_trim_u32(recv_msg, &mut recv_val);
        assert_eq!(val, recv_val);
        // Can't do this because nng uses network order (big-endian)
        //assert_eq!(val, *(nng_msg_body(recv_msg) as *const u32));

        nng_socket_close(req_socket);
        nng_socket_close(rep_socket);
    }
}
```
 */

// Don't use std unless we're allowed
#![cfg_attr(not(feature = "std"), no_std)]
// Suppress the flurry of warnings caused by using "C" naming conventions
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
// Disable clippy since this is all bindgen generated code
#![allow(clippy::all)]

// Either bindgen generated source, or the static copy
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[cfg(try_from)]
use core::convert::TryFrom;
#[cfg(feature = "std")]
use std::num::NonZeroU32;

pub use crate::bindings::*;

impl nng_pipe {
    pub const NNG_PIPE_INITIALIZER: nng_pipe = nng_pipe {
        _bindgen_opaque_blob: 0,
    };
}

impl nng_socket {
    pub const NNG_SOCKET_INITIALIZER: nng_socket = nng_socket {
        _bindgen_opaque_blob: 0,
    };
}

impl nng_dialer {
    pub const NNG_DIALER_INITIALIZER: nng_dialer = nng_dialer {
        _bindgen_opaque_blob: 0,
    };
}

impl nng_listener {
    pub const NNG_LISTENER_INITIALIZER: nng_listener = nng_listener {
        _bindgen_opaque_blob: 0,
    };
}

impl nng_ctx {
    pub const NNG_CTX_INITIALIZER: nng_ctx = nng_ctx {
        _bindgen_opaque_blob: 0,
    };
}

/// NNG error codes.
///
/// This enum represents the standard error codes returned by NNG library functions.
/// These are distinct from system errors ([`nng_err::NNG_ESYSERR`]) and transport
/// errors ([`nng_err::NNG_ETRANERR`]) which use flag bits to encode additional information.
///
/// # Relationship to [`nng_err`]
///
/// The raw [`nng_err`] type from bindgen includes `NNG_OK` (success) and flag markers
/// for system/transport errors. This `ErrorCode` enum provides a cleaner Rust-native
/// representation of just the NNG-specific error codes, excluding success and flags.
///
/// Use [`ErrorKind`] to handle all error categories (NNG errors, system errors,
/// transport errors) in a unified way.
#[repr(u32)]
#[non_exhaustive]
#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub enum ErrorCode {
    EINTR = 1,
    ENOMEM = 2,
    EINVAL = 3,
    EBUSY = 4,
    ETIMEDOUT = 5,
    ECONNREFUSED = 6,
    ECLOSED = 7,
    EAGAIN = 8,
    ENOTSUP = 9,
    EADDRINUSE = 10,
    ESTATE = 11,
    ENOENT = 12,
    EPROTO = 13,
    EUNREACHABLE = 14,
    EADDRINVAL = 15,
    EPERM = 16,
    EMSGSIZE = 17,
    ECONNABORTED = 18,
    ECONNRESET = 19,
    ECANCELED = 20,
    ENOFILES = 21,
    ENOSPC = 22,
    EEXIST = 23,
    EREADONLY = 24,
    EWRITEONLY = 25,
    ECRYPTO = 26,
    EPEERAUTH = 27,
    EBADTYPE = 30,
    ECONNSHUT = 31,
    ESTOPPED = 999,
    EINTERNAL = 1000,
}

#[cfg(feature = "std")]
impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", nng_err(*self as u32))
    }
}

/// Categorized representation of NNG error codes.
///
/// NNG functions return error codes as integers. This enum provides a typed
/// categorization of these errors, making it easier to handle different error
/// classes in Rust code.
///
/// # Error Categories
///
/// - [`NngError`](Self::NngError): Standard NNG library errors with well-defined semantics
/// - [`Transport`](Self::Transport): Transport-layer specific errors (e.g., TCP, IPC)
/// - [`System`](Self::System): Operating system errors, mapped to [`std::io::ErrorKind`]
/// - [`Other`](Self::Other): Unrecognized error codes that don't fit other categories
///
/// # Example
///
/// ```ignore
/// use nng_sys::{ErrorKind, ErrorCode, nng_err};
///
/// let err = ErrorKind::from_nng_err(nng_err::NNG_ETIMEDOUT);
/// match err {
///     ErrorKind::NngError(ErrorCode::ETIMEDOUT) => println!("Operation timed out"),
///     ErrorKind::System(kind) => println!("System error: {:?}", kind),
///     _ => println!("Other error"),
/// }
/// ```
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorKind {
    /// Standard NNG error with a known error code.
    ///
    /// These are errors defined by the NNG library itself, such as timeouts,
    /// connection refused, invalid arguments, etc.
    NngError(ErrorCode),
    /// Transport-specific error.
    ///
    /// Contains the transport-specific error code extracted from the original
    /// NNG error.
    Transport(u32),
    /// System/OS error mapped to Rust's I/O error kind.
    ///
    /// These errors originate from the operating system. The original NNG error
    /// has the `NNG_ESYSERR` marker bitset.
    System(std::io::ErrorKind),
    /// Other unrecognized error code.
    ///
    /// Contains non-zero error codes that don't match any known NNG error,
    /// transport error, or system error pattern.
    Other(NonZeroU32),
}

#[cfg(feature = "std")]
impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::NngError(err) => write!(f, "{err}"),
            ErrorKind::Transport(code) => write!(f, "Transport error #{code}"),
            ErrorKind::System(kind) => write!(f, "System error: {}", std::io::Error::from(*kind)),
            ErrorKind::Other(code) => write!(f, "Unknown error #{code}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ErrorKind {}

impl ErrorKind {
    /// Converts an [`nng_err`] into an [`ErrorKind`].
    ///
    /// This method categorizes NNG error codes into the appropriate [`ErrorKind`] variant.
    ///
    /// # Panics
    ///
    /// Panics if the error code is zero as error codes are expected to be non-zero.
    pub fn from_nng_err(err: nng_err) -> ErrorKind {
        match err {
            nng_err::NNG_EINTR => ErrorKind::NngError(ErrorCode::EINTR),
            nng_err::NNG_ENOMEM => ErrorKind::NngError(ErrorCode::ENOMEM),
            nng_err::NNG_EINVAL => ErrorKind::NngError(ErrorCode::EINVAL),
            nng_err::NNG_EBUSY => ErrorKind::NngError(ErrorCode::EBUSY),
            nng_err::NNG_ETIMEDOUT => ErrorKind::NngError(ErrorCode::ETIMEDOUT),
            nng_err::NNG_ECONNREFUSED => ErrorKind::NngError(ErrorCode::ECONNREFUSED),
            nng_err::NNG_ECLOSED => ErrorKind::NngError(ErrorCode::ECLOSED),
            nng_err::NNG_EAGAIN => ErrorKind::NngError(ErrorCode::EAGAIN),
            nng_err::NNG_ENOTSUP => ErrorKind::NngError(ErrorCode::ENOTSUP),
            nng_err::NNG_EADDRINUSE => ErrorKind::NngError(ErrorCode::EADDRINUSE),
            nng_err::NNG_ESTATE => ErrorKind::NngError(ErrorCode::ESTATE),
            nng_err::NNG_ENOENT => ErrorKind::NngError(ErrorCode::ENOENT),
            nng_err::NNG_EPROTO => ErrorKind::NngError(ErrorCode::EPROTO),
            nng_err::NNG_EUNREACHABLE => ErrorKind::NngError(ErrorCode::EUNREACHABLE),
            nng_err::NNG_EADDRINVAL => ErrorKind::NngError(ErrorCode::EADDRINVAL),
            nng_err::NNG_EPERM => ErrorKind::NngError(ErrorCode::EPERM),
            nng_err::NNG_EMSGSIZE => ErrorKind::NngError(ErrorCode::EMSGSIZE),
            nng_err::NNG_ECONNABORTED => ErrorKind::NngError(ErrorCode::ECONNABORTED),
            nng_err::NNG_ECONNRESET => ErrorKind::NngError(ErrorCode::ECONNRESET),
            nng_err::NNG_ECANCELED => ErrorKind::NngError(ErrorCode::ECANCELED),
            nng_err::NNG_ENOFILES => ErrorKind::NngError(ErrorCode::ENOFILES),
            nng_err::NNG_ENOSPC => ErrorKind::NngError(ErrorCode::ENOSPC),
            nng_err::NNG_EEXIST => ErrorKind::NngError(ErrorCode::EEXIST),
            nng_err::NNG_EREADONLY => ErrorKind::NngError(ErrorCode::EREADONLY),
            nng_err::NNG_EWRITEONLY => ErrorKind::NngError(ErrorCode::EWRITEONLY),
            nng_err::NNG_ECRYPTO => ErrorKind::NngError(ErrorCode::ECRYPTO),
            nng_err::NNG_EPEERAUTH => ErrorKind::NngError(ErrorCode::EPEERAUTH),
            nng_err::NNG_EBADTYPE => ErrorKind::NngError(ErrorCode::EBADTYPE),
            nng_err::NNG_ECONNSHUT => ErrorKind::NngError(ErrorCode::ECONNSHUT),
            nng_err::NNG_ESTOPPED => ErrorKind::NngError(ErrorCode::ESTOPPED),
            nng_err::NNG_EINTERNAL => ErrorKind::NngError(ErrorCode::EINTERNAL),
            err if err.0 & nng_err::NNG_ETRANERR.0 != 0 => {
                ErrorKind::Transport(err.0 & !nng_err::NNG_ETRANERR.0)
            }
            err if err.0 & nng_err::NNG_ESYSERR.0 != 0 => {
                let sys_err = err.0 & !nng_err::NNG_ESYSERR.0;
                let kind = std::io::Error::from_raw_os_error(sys_err as i32).kind();
                ErrorKind::System(kind)
            }
            err => ErrorKind::Other(NonZeroU32::new(err.0).expect("error codes are non-zero")),
        }
    }
}

impl nng_err {
    /// Returns a static C-string describing this error.
    ///
    /// This is a thin wrapper around `nng_strerror` that works in `no_std`
    /// environments. The returned string has static lifetime as NNG returns
    /// pointers to static null terminated string literals.
    pub fn as_cstr(&self) -> &'static core::ffi::CStr {
        // SAFETY: nng_strerror is safe to call with any nng_err value.
        let raw = unsafe { nng_strerror(*self) };
        // SAFETY: nng_strerror returns a valid, null-terminated, static string.
        unsafe { core::ffi::CStr::from_ptr(raw) }
    }
}

#[cfg(feature = "std")]
impl std::fmt::Display for nng_err {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        // TODO(flxo): update this once [Tracking Issue for
        // CStr::display](https://github.com/rust-lang/rust/issues/139984) is stable
        write!(fmt, "{}", self.as_cstr().to_string_lossy())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for nng_err {}

/// The error type returned when unable to convert an integer to an enum value.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[cfg(try_from)]
pub struct EnumFromIntError(pub i32);

#[cfg(all(try_from, feature = "std"))]
impl std::fmt::Display for EnumFromIntError {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(fmt, "EnumFromIntError({})", self.0)
    }
}

impl nng_stat_type_enum {
    /// Converts value returned by [nng_stat_type](https://nanomsg.github.io/nng/man/v1.1.0/nng_stat_type.3) into `nng_stat_type_enum`.
    pub fn try_convert_from(value: i32) -> Option<Self> {
        use crate::nng_stat_type_enum::*;
        match value {
            value if value == NNG_STAT_SCOPE as i32 => Some(NNG_STAT_SCOPE),
            value if value == NNG_STAT_LEVEL as i32 => Some(NNG_STAT_LEVEL),
            value if value == NNG_STAT_COUNTER as i32 => Some(NNG_STAT_COUNTER),
            value if value == NNG_STAT_STRING as i32 => Some(NNG_STAT_STRING),
            value if value == NNG_STAT_BOOLEAN as i32 => Some(NNG_STAT_BOOLEAN),
            value if value == NNG_STAT_ID as i32 => Some(NNG_STAT_ID),
            _ => None,
        }
    }
}

#[cfg(try_from)]
impl TryFrom<i32> for nng_stat_type_enum {
    type Error = EnumFromIntError;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        nng_stat_type_enum::try_convert_from(value).ok_or(EnumFromIntError(value))
    }
}

impl nng_unit_enum {
    /// Converts value returned by [nng_stat_unit](https://nanomsg.github.io/nng/man/v1.1.0/nng_stat_unit.3) into `nng_unit_enum`.
    pub fn try_convert_from(value: i32) -> Option<Self> {
        use crate::nng_unit_enum::*;
        match value {
            value if value == NNG_UNIT_NONE as i32 => Some(NNG_UNIT_NONE),
            value if value == NNG_UNIT_BYTES as i32 => Some(NNG_UNIT_BYTES),
            value if value == NNG_UNIT_MESSAGES as i32 => Some(NNG_UNIT_MESSAGES),
            value if value == NNG_UNIT_MILLIS as i32 => Some(NNG_UNIT_MILLIS),
            value if value == NNG_UNIT_EVENTS as i32 => Some(NNG_UNIT_EVENTS),
            _ => None,
        }
    }
}

#[cfg(try_from)]
impl TryFrom<i32> for nng_unit_enum {
    type Error = EnumFromIntError;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        nng_unit_enum::try_convert_from(value).ok_or(EnumFromIntError(value))
    }
}

impl nng_sockaddr_family {
    pub fn try_convert_from(value: i32) -> Option<Self> {
        use crate::nng_sockaddr_family::*;
        match value {
            value if value == NNG_AF_UNSPEC as i32 => Some(NNG_AF_UNSPEC),
            value if value == NNG_AF_INPROC as i32 => Some(NNG_AF_INPROC),
            value if value == NNG_AF_IPC as i32 => Some(NNG_AF_IPC),
            value if value == NNG_AF_INET as i32 => Some(NNG_AF_INET),
            value if value == NNG_AF_INET6 as i32 => Some(NNG_AF_INET6),
            _ => None,
        }
    }
}

#[cfg(try_from)]
impl TryFrom<i32> for nng_sockaddr_family {
    type Error = EnumFromIntError;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        nng_sockaddr_family::try_convert_from(value).ok_or(EnumFromIntError(value))
    }
}
