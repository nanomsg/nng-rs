use std::{os::raw::c_int, ptr::NonNull, time::Duration};

use crate::error::{Error, Result};

/// Converts an NNG return code into a Rust `Result`.
macro_rules! rv2res {
    ($rv:expr, $ok:expr) => {
        match std::num::NonZeroU32::new($rv as u32) {
            None => Ok($ok),
            Some(e) => Err($crate::error::Error::from(e)),
        }
    };

    ($rv:expr) => {
        rv2res!($rv, ())
    };
}

/// Converts a Rust `Duration` into an `nng_duration`.
#[allow(clippy::cast_possible_truncation)]
pub fn duration_to_nng(dur: Option<Duration>) -> nng_sys::nng_duration {
    // The subsecond milliseconds is guaranteed to be less than 1000, which
    // means converting from `u32` to `i32` is safe. The only other
    // potential issue is converting the `u64` of seconds to an `i32`.
    match dur {
        None => nng_sys::NNG_DURATION_INFINITE,
        Some(d) => {
            let secs = if d.as_secs() > i32::MAX as u64 {
                i32::MAX
            } else {
                d.as_secs() as i32
            };
            let millis = d.subsec_millis() as i32;

            secs.saturating_mul(1000).saturating_add(millis)
        }
    }
}

/// Checks an NNG return code and validates the pointer, returning a
/// `NonNull`.
#[inline]
pub fn validate_ptr<T>(rv: c_int, ptr: *mut T) -> Result<NonNull<T>> {
    if let Some(e) = std::num::NonZeroU32::new(rv as u32) {
        Err(Error::from(e))
    } else {
        Ok(NonNull::new(ptr).expect("NNG returned a null pointer from a successful function"))
    }
}

/// Aborts the program if the provided closure panics.
///
/// This is meant to handle the fact that `UnwindSafe` is a little bit broken in
/// Rust and catching an unwind does not necessarily mean that the things should
/// be exception-safe. See #6 for a few examples of why this is fine.
pub fn abort_unwind<F: FnOnce() -> R, R>(f: F) -> R {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            if std::thread::panicking() {
                std::process::abort();
            }
        }
    }

    let _guard = Guard;
    f()
}
