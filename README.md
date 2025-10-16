# nng-rs

This repository contains [Rust](https://rust-lang.org/) bindings for the
[NNG](https://github.com/nanomsg/nng) library. Each crate is located in its own
subdirectory and maintained independently.

## `nng`

A safe, idiomatic Rust wrapper for [NNG](https://github.com/nanomsg/nng). It
aims to provide an API that is similar to the original C library, making it easy
to adapt existing examples.  For more details, please see the [`nng` crate's
README](./nng/README.md).

## `nng-sys`

![crates.io](https://img.shields.io/crates/v/nng-sys)

This crate provides the raw, unsafe FFI (Foreign Function Interface) bindings
to the NNG C library. It is responsible for compiling NNG from source and
linking it, offering various features to control the build process. This crate
is intended for developers who need direct access to the NNG C API or are
building higher-level wrappers like `nng`. For more details, please see the
[`nng-sys` crate's README](./nng-sys/README.md).
