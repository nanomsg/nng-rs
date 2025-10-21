
Rust FFI bindings to [NNG](https://github.com/nanomsg/nng):

> NNG, like its predecessors nanomsg (and to some extent ZeroMQ), is a lightweight, broker-less library, offering a simple API to solve common recurring messaging problems, such as publish/subscribe, RPC-style request/reply, or service discovery. The API frees the programmer from worrying about details like connection management, retries, and other common considerations, so that they can focus on the application instead of the plumbing.

[![docs.rs](https://docs.rs/nng-sys/badge.svg)](https://docs.rs/nng-sys)
[![crates.io](http://img.shields.io/crates/v/nng-sys.svg)](http://crates.io/crates/nng-sys)
![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rustc 1.31+](https://img.shields.io/badge/rustc-1.31+-lightgray.svg)
[![travis](https://travis-ci.org/jeikabu/nng-rust.svg?branch=master)](https://travis-ci.org/jeikabu/nng-rust)
[![Build Status](https://dev.azure.com/jeikabu/nng-rust/_apis/build/status/jeikabu.nng-rust?branchName=master)](https://dev.azure.com/jeikabu/nng-rust/_build/latest?definitionId=1&branchName=master)

## Usage

To use the latest version of this crate, add the following to your `Cargo.toml`:

```toml
[dependencies]
nng-sys = "0.3.0"
```

**Note:** `nng-sys` comes with a vendored version of NNG. See the [Versioning Scheme](#versioning-scheme) section for details on how to identify the exact version. The vendored NNG sources are used if the feature `build-nng` is enabled. If a custom version of NNG is needed, enable the `build-bindgen` feature (*and* disable `build-nng`) to generate bindings from NNG headers installed on the system. The [build script](./build.rs) will search default paths for the NNG headers and add `nng` as a dynamically linked library.

Requirements:
- [cmake](https://cmake.org/) v3.13 or newer in `PATH`
    - On Linux/macOS: default generator is "Unix Makefiles"
    - On Windows: default generator is generally latest version of Visual Studio installed
    - To use a different generator, set the `CMAKE_GENERATOR` environment variable
- _Optional_ libclang needed if using `build-bindgen` feature to run [bindgen](https://rust-lang.github.io/rust-bindgen/requirements.html)

## Versioning Scheme

This crate uses the format `<crate version>+<nng version>` following [Semantic Versioning 2.0.0](https://semver.org/#spec-item-10).

**Example:** `0.3.0+1.11.0`

- `0.3.0` - The crate's own semantic version, indicating API compatibility
- `+1.11.0` - Build metadata showing the wrapped NNG library version

### How Versions Change

| Scenario                    | Example Version | Explanation                                                                                |
| --------------------------- | --------------- | ------------------------------------------------------------------------------------------ |
| Patch fix in crate bindings | `0.3.1+1.11.0`  | Bug fixes, no API changes                                                                  |
| Update to newer NNG version | `0.3.2+1.12.0`  | Compatible update to the NNG library                                                       |
| Breaking change in bindings | `0.4.0+1.12.0`  | Breaking API changes introduced by e.g. a `bindgen` upgrade (pre-1.0, minor acts as major) |
| NNG mayor version update    | `0.5.0+2.0.0`   | A backwards-incompatible NNG release also results in a major version bump                                      |

**Note:** Cargo ignores the `+<nng version>` build metadata suffix when resolving dependencies, so version `0.3.0+1.11.0` and `0.3.0+1.12.0` are considered equivalent by Cargo and cannot coexist.

**Note:** Versions of this crate prior to `0.3.0` used a different scheme (`<NNG_version>-rc.<crate_version>`). This legacy format has been replaced to allow for proper semantic versioning of the crate's API.

## Features

- `build-nng`: use cmake to build NNG from source (enabled by default)
- `build-bindgen`: run bindgen to re-generate Rust FFI bindings to C
- `nng-stats`: enable NNG stats `NNG_ENABLE_STATS` (enabled by default)
- `nng-tls`: enable TLS `NNG_ENABLE_TLS` (requires mbedTLS)
- `nng-supplemental`: generate bindings to NNG's supplemental functions
- `nng-compat`: generate bindings to NNG's nanomsg compatible functions

_Example_) Re-generate FFI bindings with bindgen:
```toml
[dependencies]
nng-sys = { version = "0.3.0", features = ["build-bindgen"] }
```

_Example_) Disable stats and build from source:
```toml
[dependencies.nng-sys]
version = "0.3.0"
default-features = false
features = ["build-nng"]
```

## Examples
```rust
use nng_sys::*;
use std::{ffi::CString, os::raw::c_char, ptr::null_mut};

fn example() {
    unsafe {
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

        nng_close(req_socket);
        nng_close(rep_socket);
    }
}
```
