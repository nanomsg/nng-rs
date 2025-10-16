#[cfg(test)]
mod tests {

    use nng_sys::*;
    use std::{ffi::CString, os::raw::c_char, ptr::null_mut};

    #[test]
    fn basic() {
        unsafe {
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

            nng_close(req_socket);
            nng_close(rep_socket);
        }
    }
}
