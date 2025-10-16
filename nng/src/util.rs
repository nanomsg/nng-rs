use std::{
	os::raw::{c_char, c_int, c_void},
	ptr::NonNull,
	time::Duration,
};

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

/// Utility macro for creating a new option type.
///
/// This is 90% me just playing around with macros. It is probably a terrible
/// way to go around doing this task but, then again, this whole options
/// business has been a complete mess.
macro_rules! create_option
{
	(
		$(#[$attr:meta])*
		$opt:ident -> $ot:ty:
		Get $g:ident = $gexpr:stmt;
		Set $s:ident $v:ident = $sexpr:stmt;
	) => {
		$(#[$attr])*
		#[allow(missing_debug_implementations)]
		#[allow(missing_copy_implementations)]
		#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
		pub enum $opt {}
		#[allow(deprecated)]
		impl $crate::options::Opt for $opt
		{
			type OptType = $ot;
		}
		#[allow(deprecated)]
		#[allow(clippy::cast_possible_truncation)]
		impl $crate::options::private::OptOps for $opt
		{
			fn get<T: $crate::options::private::HasOpts>($g: &T) -> $crate::error::Result<Self::OptType> { $gexpr }
			fn set<T: $crate::options::private::HasOpts>($s: &T, $v: Self::OptType) -> $crate::error::Result<()> { $sexpr }
		}
		#[allow(deprecated)]
		#[allow(clippy::use_debug)]
		impl std::fmt::Display for $opt
		{
			fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result
			{
				write!(f, "{:?}", self)
			}
		}
	};

	(
		$(#[$attr:meta])*
		$opt:ident -> $ot:ty:
		Set $s:ident $v:ident = $sexpr:stmt;
	) => {
		create_option!(
		$(#[$attr])*
		$opt -> $ot:
		Get _g = unreachable!("should not have been implemented - option is write-only");
		Set $s $v = $sexpr;
		);
	};

	(
		$(#[$attr:meta])*
		$opt:ident -> $ot:ty:
		Get $g:ident = $gexpr:stmt;
	) => {
		create_option!(
		$(#[$attr])*
		$opt -> $ot:
		Get $g = $gexpr;
		Set _s _v = unreachable!("should not have been implemented - option is read-only");
		);
	};
}

/// Implements the specified options for the type.
macro_rules! expose_options
{
	(
		$struct:ident :: $($member:ident).+ -> $handle:ty;
		GETOPT_BOOL = $go_b:path;
		GETOPT_INT = $go_i:path;
		GETOPT_MS = $go_ms:path;
		GETOPT_SIZE = $go_sz:path;
		GETOPT_SOCKADDR = $go_sa:path;
		GETOPT_STRING = $go_str:path;
		GETOPT_UINT64 = $go_uint64:path;

		SETOPT = $so:path;
		SETOPT_BOOL = $so_b:path;
		SETOPT_INT = $so_i:path;
		SETOPT_MS = $so_ms:path;
		SETOPT_PTR = $so_ptr:path;
		SETOPT_SIZE = $so_sz:path;
		SETOPT_STRING = $so_str:path;

		Gets -> [$($($getters:ident)::+),*];
		Sets -> [$($($setters:ident)::+),*];
	) => {
		impl $crate::options::private::HasOpts for $struct
		{
			type Handle = $handle;
			fn handle(&self) -> Self::Handle { self.$($member).+ }

			const GETOPT_BOOL: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut bool) -> std::os::raw::c_int = $go_b;
			const GETOPT_INT: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut std::os::raw::c_int) -> std::os::raw::c_int = $go_i;
			const GETOPT_MS: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut nng_sys::nng_duration) -> std::os::raw::c_int = $go_ms;
			const GETOPT_SIZE: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut usize) -> std::os::raw::c_int = $go_sz;
			const GETOPT_SOCKADDR: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut nng_sys::nng_sockaddr) -> std::os::raw::c_int = $go_sa;
			const GETOPT_STRING: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut *mut std::os::raw::c_char) -> std::os::raw::c_int = $go_str;
			const GETOPT_UINT64: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut u64) -> std::os::raw::c_int = $go_uint64;

			const SETOPT: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *const std::os::raw::c_void, usize) -> std::os::raw::c_int = $so;
			const SETOPT_BOOL: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, bool) -> std::os::raw::c_int = $so_b;
			const SETOPT_INT: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, std::os::raw::c_int) -> std::os::raw::c_int = $so_i;
			const SETOPT_MS: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, nng_sys::nng_duration) -> std::os::raw::c_int = $so_ms;
			const SETOPT_PTR: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *mut std::os::raw::c_void) -> std::os::raw::c_int = $so_ptr;
			const SETOPT_SIZE: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, usize) -> std::os::raw::c_int = $so_sz;
			const SETOPT_STRING: unsafe extern "C" fn(Self::Handle, *const std::os::raw::c_char, *const std::os::raw::c_char) -> std::os::raw::c_int = $so_str;
		}

		$(
			#[allow(deprecated)]
			impl $crate::options::GetOpt<$crate::options::$($getters)::+> for $struct {}
		)*
		$(
			#[allow(deprecated)]
			impl $crate::options::SetOpt<$crate::options::$($setters)::+> for $struct {}
		)*
	}
}

/// A catch-all function for unsupported options operations.
#[allow(clippy::unimplemented)]
pub unsafe extern "C" fn fake_opt<H, T>(_: H, _: *const c_char, _: T) -> c_int
{
	unimplemented!("{} does not support the option operation on {}", stringify!(H), stringify!(T))
}

/// A catch-all function for unsupported generic options operations.
#[allow(clippy::unimplemented)]
pub unsafe extern "C" fn fake_genopt<H>(_: H, _: *const c_char, _: *const c_void, _: usize)
-> c_int
{
	unimplemented!("{} does not support the generic option operation", stringify!(H))
}

/// Converts a Rust `Duration` into an `nng_duration`.
#[allow(clippy::cast_possible_truncation)]
pub fn duration_to_nng(dur: Option<Duration>) -> nng_sys::nng_duration
{
	// The subsecond milliseconds is guaranteed to be less than 1000, which
	// means converting from `u32` to `i32` is safe. The only other
	// potential issue is converting the `u64` of seconds to an `i32`.
	use std::i32::MAX;

	match dur {
		None => nng_sys::NNG_DURATION_INFINITE,
		Some(d) => {
			let secs = if d.as_secs() > MAX as u64 { MAX } else { d.as_secs() as i32 };
			let millis = d.subsec_millis() as i32;

			secs.saturating_mul(1000).saturating_add(millis)
		},
	}
}

/// Converts an `nng_duration` into a Rust `Duration`.
pub fn nng_to_duration(ms: nng_sys::nng_duration) -> Option<Duration>
{
	if ms == nng_sys::NNG_DURATION_INFINITE {
		None
	}
	else if ms >= 0 {
		Some(Duration::from_millis(ms as u64))
	}
	else {
		panic!("Unexpected value for `nng_duration` ({})", ms)
	}
}

/// Checks an NNG return code and validates the pointer, returning a
/// `NonNull`.
#[inline]
pub fn validate_ptr<T>(rv: c_int, ptr: *mut T) -> Result<NonNull<T>>
{
	if let Some(e) = std::num::NonZeroU32::new(rv as u32) {
		Err(Error::from(e))
	}
	else {
		Ok(NonNull::new(ptr).expect("NNG returned a null pointer from a successful function"))
	}
}

/// Aborts the program if the provided closure panics.
///
/// This is meant to handle the fact that `UnwindSafe` is a little bit broken in
/// Rust and catching an unwind does not necessarily mean that the things should
/// be exception-safe. See #6 for a few examples of why this is fine.
pub fn abort_unwind<F: FnOnce() -> R, R>(f: F) -> R
{
	struct Guard;
	impl Drop for Guard
	{
		fn drop(&mut self)
		{
			if std::thread::panicking() {
				std::process::abort();
			}
		}
	}

	let _guard = Guard;
	f()
}
