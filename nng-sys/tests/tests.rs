#[cfg(test)]
mod tests {

    use nng_sys::*;
    use std::{
        ffi::{CStr, CString},
        os::raw::c_char,
        ptr::null_mut,
    };

    #[test]
    fn basic() {
        unsafe {
            // Initialize NNG (required in NNG 2.0)
            assert_eq!(nng_err::NNG_OK, nng_init(null_mut()));

            let url = CString::new("inproc://nng_sys/tests/basic").unwrap();
            let url = url.as_bytes_with_nul().as_ptr() as *const c_char;

            // Reply socket
            let mut rep_socket = nng_socket::default();
            assert_eq!(0, nng_rep0_open(&mut rep_socket));
            assert_eq!(0, nng_listen(rep_socket, url, null_mut(), 0));

            // Request socket
            let mut req_socket = nng_socket::default();
            assert_eq!(0, nng_req0_open(&mut req_socket));
            assert_eq!(0, nng_dial(req_socket, url, null_mut(), 0));

            // Send message
            let mut req_msg: *mut nng_msg = null_mut();
            assert_eq!(0, nng_msg_alloc(&mut req_msg, 0));
            // Add a value to the body of the message
            let val = 0x12345678;
            assert_eq!(0, nng_msg_append_u32(req_msg, val));
            assert_eq!(0, nng_sendmsg(req_socket, req_msg, 0));

            // Receive it
            let mut recv_msg: *mut nng_msg = null_mut();
            assert_eq!(0, nng_recvmsg(rep_socket, &mut recv_msg, 0));
            // Remove our value from the body of the received message
            let mut recv_val: u32 = 0;
            assert_eq!(0, nng_msg_trim_u32(recv_msg, &mut recv_val));
            assert_eq!(val, recv_val);

            nng_socket_close(req_socket);
            nng_socket_close(rep_socket);
        }
    }

    /// Test that verifies nng_version() runtime call matches compile-time constants
    #[test]
    fn verify_runtime_version_matches_constants() {
        unsafe {
            // Get the runtime version string from nng_version()
            // Note: nng_version() returns a pointer to a static string literal,
            // so we don't need to free it - it's valid for the program's lifetime
            let version_ptr = nng_version();
            assert!(!version_ptr.is_null(), "nng_version() returned null");

            let version_cstr = CStr::from_ptr(version_ptr);
            let version_str = version_cstr
                .to_str()
                .expect("nng_version() returned invalid UTF-8");

            // Build expected version string from compile-time constants
            let expected_version = format!(
                "{}.{}.{}{}",
                NNG_MAJOR_VERSION,
                NNG_MINOR_VERSION,
                NNG_PATCH_VERSION,
                CStr::from_bytes_with_nul_unchecked(NNG_RELEASE_SUFFIX).to_string_lossy(),
            );

            println!("Runtime version from nng_version(): {}", version_str);
            println!("Expected version from constants: {}", expected_version);

            // Verify the runtime version matches our compile-time constants
            assert_eq!(
                version_str, expected_version,
                "Runtime version '{}' doesn't match compile-time constants '{}'",
                version_str, expected_version
            );

            // Also verify by parsing the version string
            let parts: Vec<&str> = version_str.split('.').collect();
            assert_eq!(parts.len(), 3, "Version string should have 3 parts");

            let runtime_major: u32 = parts[0].parse().expect("a valid major version");
            let runtime_minor: u32 = parts[1].parse().expect("a valid minor version");
            // The patch part may contain a suffix like "0dev", so extract only the numeric prefix
            let patch_numeric: String = parts[2]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let runtime_patch: u32 = patch_numeric.parse().expect("a valid patch version");
            let runtime_suffix: String = parts[2]
                .chars()
                .skip_while(|c| c.is_ascii_digit())
                .collect();

            assert_eq!(
                runtime_major, NNG_MAJOR_VERSION,
                "runtime major version doesn't match constant"
            );
            assert_eq!(
                runtime_minor, NNG_MINOR_VERSION,
                "runtime minor version doesn't match constant"
            );
            assert_eq!(
                runtime_patch, NNG_PATCH_VERSION,
                "runtime patch version doesn't match constant"
            );
            assert_eq!(
                runtime_suffix,
                CStr::from_bytes_with_nul_unchecked(NNG_RELEASE_SUFFIX).to_string_lossy(),
                "runtime release suffix doesn't match constant"
            );
        }
    }
}
