Rust FFI bindings to [NNG](https://github.com/nanomsg/nng):

> NNG, like its predecessors nanomsg (and to some extent ZeroMQ), is a lightweight, broker-less library, offering a simple API to solve common recurring messaging problems, such as publish/subscribe, RPC-style request/reply, or service discovery. The API frees the programmer from worrying about details like connection management, retries, and other common considerations, so that they can focus on the application instead of the plumbing.

[![docs.rs](https://docs.rs/nng-sys/badge.svg)](https://docs.rs/nng-sys)
[![crates.io](http://img.shields.io/crates/v/nng-sys.svg)](http://crates.io/crates/nng-sys)
![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)

> [!WARNING]
> This crate wraps [NNG v2](https://github.com/nanomsg/nng), which is still in alpha.
> For [NNG v1 bindings](https://github.com/nanomsg/nng/tree/stable), see the [v1.xx branch](https://github.com/nanomsg/nng-rs/tree/v1.xx).

## Usage

To use the latest version of this crate, add the following to your `Cargo.toml`:

```toml
[dependencies]
nng-sys = "0.4.0"
```

If you need TLS support:

```toml
[dependencies]
nng-sys = { version = "0.4.0", features = ["tls"] }
```

This will bundle [mbedTLS](https://tls.mbed.org/). See the [NNG TLS documentation](https://github.com/nanomsg/nng/blob/main/docs/man/nng_tls.7.adoc) for details.

### Library Discovery

`nng-sys` comes with a vendored version of NNG, whose version is visible from the build metadata in the crate version (the part after `+`).
Vendoring is enabled through the `vendored` feature and is on by default.
This is mainly because NNG isn't generally going to be installed on users' systems,
unlike more widely used libraries like openssl or zlib.

**For most users, the default set of features (so a vendored build) is what you'll want.**
Just ensure you have `cmake` and a C compiler installed, and you're good to go.

If you do not want to use a vendored build, disable the `vendored` feature (or set `NNG_NO_VENDOR`).
This will make NNG instead search for a system-provided NNG install to link with.
Note that when using a system-provided NNG instead,
bindings are always re-generated with bindgen at build time since there is no guarantee that the system library is the same version
(or built with the same features)
as the pre-generated bindings.

The [build script](./build.rs) implements a preference hierarchy to discover system-provided NNG installations:

1. **Explicit env vars** (highest priority):
   - `NNG_DIR`: Root installation directory (containing `lib/` and `include/` subdirs)
   - `NNG_LIB_DIR` + `NNG_INCLUDE_DIR`: Separate library and header directories

2. **pkg-config** / **vcpkg**. NNG [does not yet ship `nng.pc` NNG](https://github.com/nanomsg/nng/issues/926), but it may be provided by certain distros.

3. **Platform-specific fallback paths**:
   - macOS: Homebrew (`/opt/homebrew`) and MacPorts (`/opt/local`)

If no system-provided NNG is found, the build errors.
There is no fallback to a vendored version.

### Requirements

**Default (vendored build)**:
- [cmake](https://cmake.org/) v3.13 or newer in `PATH`
  - On Linux/macOS: default generator is "Unix Makefiles"
  - On Windows: default generator is generally latest version of Visual Studio installed
  - To use a different generator, set the `CMAKE_GENERATOR` environment variable

**Optional** (for `bindgen` feature):
- libclang needed to run [bindgen](https://rust-lang.github.io/rust-bindgen/requirements.html)

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
| NNG major version update    | `0.5.0+2.0.0`   | A backwards-incompatible NNG release also results in a major version bump                  |

**Note:** Cargo ignores the `+<nng version>` build metadata suffix when resolving dependencies,
so version `0.3.0+1.11.0` and `0.3.0+1.12.0` are considered equivalent by Cargo and cannot coexist.

**Note:** Versions of this crate prior to `0.3.0` used a different scheme (`<NNG_version>-rc.<crate_version>`).
This legacy format has been replaced to allow for proper semantic versioning of the crate's API.

## Features

| Feature          | Default | Description                                                             |
| ---------------- | ------- | ----------------------------------------------------------------------- |
| `vendored`       | ✓       | Build NNG from bundled sources using CMake                              |
| `vendored-stats` | ✓       | Enable NNG stats (`NNG_ENABLE_STATS`) when building vendored NNG        |
| `std`            | ✓       | Enable standard library support                                         |
| `static`         |         | Force static linking (default: static for vendored, dynamic for system) |
| `bindgen`        |         | Generate bindings at build time (uses pre-generated by default)         |
| `tls`            |         | Enable TLS support (`NNG_ENABLE_TLS`, requires mbedTLS)                 |

**Note:** Features that control NNG build options (`vendored-stats`, `tls`) only affect vendored builds.
When using a system-provided NNG library, ensure it was built with the features you need.

## Environment Variables

### Quick Reference

| Variable                          | Purpose                                                  | Example                             |
| --------------------------------- | -------------------------------------------------------- | ----------------------------------- |
| `NNG_DIR`                         | Root NNG installation directory                          | `/usr/local`                        |
| `NNG_LIB_DIR` + `NNG_INCLUDE_DIR` | Separate library and header paths                        | `/opt/nng/lib`, `/opt/nng/include`  |
| `NNG_STATIC`                      | Force static (`1`) or dynamic (`0`) linking              | `1`                                 |
| `NNG_NO_VENDOR`                   | Prohibit vendored builds (error if system lib not found) | `1`                                 |
| `{TARGET}_NNG_*`                  | Target-specific override (cross-compilation)             | `AARCH64_UNKNOWN_LINUX_GNU_NNG_DIR` |

### Build Control

- `NNG_STATIC`: Set to `1` to force static linking, `0` for dynamic
  - Default: `1` (static) for vendored builds, `0` (dynamic) for system-provided libraries
- `NNG_NO_VENDOR`: Set to `1` to force the use of a system-provided NNG install (errors if none is found)
  - Overrides vendored build behavior even if the `vendored` feature is enabled
  - Matches the pattern from openssl-sys (`OPENSSL_NO_VENDOR`) and libgit2-sys (`LIBGIT2_NO_VENDOR`)
  - Useful for deployments where deep control over exact versions of dependencies is needed
    (e.g., systems that need to deal with compliance, SBOMs, etc.)

### Cross-Compilation

For cross-compilation, use target-prefixed variants which take precedence:

```bash
export AARCH64_UNKNOWN_LINUX_GNU_NNG_DIR=/path/to/nng-arm64
cargo build --target aarch64-unknown-linux-gnu
```

Target prefix format: `{TARGET}_{VAR_NAME}` where target uses `_` instead of `-` and is uppercased.

## Code Example

```rust
use nng_sys::*;
use std::{ffi::CString, os::raw::c_char, ptr::null_mut};

fn example() {
    unsafe {
        // Initialize NNG (required in NNG 2.0)
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

## Migration from NNG 1.x

This version wraps **NNG 2.0.0-alpha.6**, which is a major breaking release from
NNG 1.x. See the [NNG 2.0 release
notes](https://github.com/nanomsg/nng/releases) and the [NNG 1.x Migration
Guide](https://github.com/nanomsg/nng/blob/master/docs/ref/migrate/nng1.md) for details.
