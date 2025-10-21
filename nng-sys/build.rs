fn main() {
    cfg();
    link_nng();
    build_bindgen();
}

fn cfg() {
    // Make the config directive try_from known to rustc to suppress the warning
    println!("cargo::rustc-check-cfg=cfg(try_from)");
    match version_check::is_min_version("1.34.0") {
        Some(true) => println!("cargo:rustc-cfg=try_from"),
        _ => {}
    }
}

#[cfg(feature = "build-nng")]
fn link_nng() {
    let stats = if cfg!(feature = "nng-stats") {
        "ON"
    } else {
        "OFF"
    };

    let tls = if cfg!(feature = "nng-tls") {
        println!("cargo:rustc-link-lib=dylib=mbedcrypto");
        println!("cargo:rustc-link-lib=dylib=mbedtls");
        println!("cargo:rustc-link-lib=dylib=mbedx509");
        "ON"
    } else {
        "OFF"
    };

    // Run cmake to build nng
    let mut config = cmake::Config::new("nng");
    config
        .define("NNG_ENABLE_STATS", stats)
        .define("NNG_ENABLE_TLS", tls)
        .define("BUILD_SHARED_LIBS", "OFF")
        .build_target("nng");

    if cfg!(target_env = "msvc") {
        // Rust always links against MSVC's Release CRT, so if we build nng against the Debug
        // version, we get linker errors. Thus, always build nng in Release to match that of Rust
        // here (by default, cmake will match the current `OPT_LEVEL`).
        config.profile("Release");
    }

    let dst = config.build();

    if cfg!(target_env = "msvc") {
        // MSVC does _not_ search recursively, so we have to point it _specifically_ at the
        // directory that holds nng.lib.
        println!(
            "cargo:rustc-link-search=native={}",
            dst.join("build/Release").display()
        );
    } else {
        println!(
            "cargo:rustc-link-search=native={}",
            dst.join("build").display()
        );
    }

    println!("cargo:rustc-link-lib=static=nng");

    // On Windows, nng has more dependencies
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=mswsock");
    }
}

#[cfg(not(feature = "build-nng"))]
fn link_nng() {
    // move to pkg-config once nng ships with one: https://github.com/nanomsg/nng/issues/926
    println!("cargo:rustc-link-lib=dylib=nng");
}

#[cfg(feature = "build-bindgen")]
fn build_bindgen() {
    use std::{env, path::PathBuf};

    let mut builder = bindgen::Builder::default()
        .header("src/wrapper.h")
        // #[derive(Default)]
        .derive_default(true)
        .allowlist_type("nng_.*")
        .allowlist_function("nng_.*")
        .allowlist_var("NNG_.*")
        .opaque_type("nng_.*_s")
        // Generate `pub const NNG_UNIT_EVENTS` instead of `nng_unit_enum_NNG_UNIT_EVENTS`
        .prepend_enum_name(false)
        // Generate `pub enum ...` instead of multiple `pub const ...`
        .default_enum_style(bindgen::EnumVariation::Rust {
            non_exhaustive: true,
        })
        .constified_enum("nng_flag_enum")
        // NNG_ESYSERR and NNG_ETRANERR are used like flag
        .constified_enum("nng_errno_enum")
        .constified_enum("nng_pipe_ev")
        .use_core()
        .parse_callbacks(Box::new(BindgenCallbacks::default()))
        .size_t_is_usize(true)
        // Layout tests are non-portable; 64-bit tests are "wrong" size on 32-bit and always fail.
        // Don't output tests if we're regenerating `src/bindings.rs` (shared by all platforms when bindgen not used)
        .layout_tests(!cfg!(feature = "source-update-bindings"));

    // Ensure that the headers from the vendored nng sources are used
    // when the `build-nng` feature builds and links the nng library.
    // If the `build-nng` feature is not set - do *not* add the directory
    // to the search path in order to make clang find headers "somewhere".
    if cfg!(feature = "build-nng") {
        builder = builder.clang_arg("-Inng/include/");
    }

    if cfg!(feature = "nng-compat") {
        builder = builder.header("src/compat.h");
    }
    if cfg!(feature = "nng-supplemental") {
        builder = builder.header("src/supplemental.h");
    }
    if cfg!(feature = "no_std") {
        // no_std support
        // https://rust-embedded.github.io/book/interoperability/c-with-rust.html#automatically-generating-the-interface
        builder = builder.ctypes_prefix("cty")
    }

    const BINDINGS_RS: &str = "bindings.rs";
    let out_file = PathBuf::from(env::var("OUT_DIR").unwrap()).join(BINDINGS_RS);
    builder
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(out_file.to_owned())
        .expect("Couldn't write bindings");

    #[cfg(feature = "source-update-bindings")]
    {
        let bindings = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("src")
            .join(BINDINGS_RS);
        std::fs::copy(out_file, bindings).expect("Unable to update bindings");
    }
}

#[cfg(not(feature = "build-bindgen"))]
fn build_bindgen() {
    // Nothing
}

#[cfg(feature = "build-bindgen")]
#[derive(Debug, Default)]
struct BindgenCallbacks;

#[cfg(feature = "build-bindgen")]
impl bindgen::callbacks::ParseCallbacks for BindgenCallbacks {
    fn enum_variant_behavior(
        &self,
        _enum_name: Option<&str>,
        original_variant_name: &str,
        _variant_value: bindgen::callbacks::EnumVariantValue,
    ) -> Option<bindgen::callbacks::EnumVariantCustomBehavior> {
        // nng_pipe_ev::NNG_PIPE_EV_NUM is only used in NNG internals to validate range of values.
        // We want to exclude it so it doesn't need to be included for `match` to be exhaustive.
        if original_variant_name == "NNG_PIPE_EV_NUM" {
            Some(bindgen::callbacks::EnumVariantCustomBehavior::Hide)
        } else {
            None
        }
    }
}
