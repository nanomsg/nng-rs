use std::env;
use std::path::{Path, PathBuf};

/// Environment variables that control NNG discovery and build
const ENV_VARS: &[&str] = &[
    "NNG_DIR",         // Root installation directory
    "NNG_LIB_DIR",     // Library directory
    "NNG_INCLUDE_DIR", // Header directory
    "NNG_STATIC",      // Force static linking
    "NNG_NO_VENDOR",   // Prohibit vendored build (error if system lib not found)
];

/// How the NNG library was found (determines link directive handling)
#[allow(dead_code)]
enum LibrarySource {
    /// Built from vendored sources
    Vendored,
    /// Found via pkg-config (already emitted link directives)
    PkgConfig,
    /// Found via vcpkg (already emitted link directives)
    Vcpkg,
    /// Found via env vars or platform-specific paths
    Manual,
}

fn main() {
    // 1. Configuration and version detection
    emit_cfg_directives();

    // 2. Check if vendored build is prohibited
    let force_system = force_no_vendor();

    // 3. Find and link library
    // Prefer vendored when feature is enabled (unless NNG_NO_VENDOR is set)
    let (source, includes) = if cfg!(feature = "vendored") && !force_system {
        check_vendored_source();
        build_vendored()
    } else if let Some((source, includes)) = try_find_system_library() {
        (source, includes)
    } else if force_system {
        panic!("NNG_NO_VENDOR is set but no system NNG library was found");
    } else {
        panic!("NNG library not found and vendored feature not enabled");
    };

    // 4. Emit link-lib directive (unless pkg-config/vcpkg already did)
    match source {
        LibrarySource::PkgConfig | LibrarySource::Vcpkg => {
            // pkg-config/vcpkg already emitted cargo:rustc-link-lib=nng
        }
        LibrarySource::Vendored | LibrarySource::Manual => {
            // We need to emit the link-lib directive
            let static_link = should_link_static(matches!(source, LibrarySource::Vendored));
            if static_link {
                println!("cargo:rustc-link-lib=static=nng");
            } else {
                println!("cargo:rustc-link-lib=dylib=nng");
            }
        }
    }
    // if TLS was requested, make sure we also link mbedtls
    if cfg!(feature = "tls") {
        println!("cargo:rustc-link-lib=dylib=mbedcrypto");
        println!("cargo:rustc-link-lib=dylib=mbedtls");
        println!("cargo:rustc-link-lib=dylib=mbedx509");
    }

    // 5. Windows system libraries (needed regardless of how NNG was found)
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=mswsock");
    }

    // 6. Regenerate bindings if they aren't guaranteed to match pre-generated ones.
    // For example, the system-provided library could be a newer or older version.
    // We don't need to check for `compat` or `supplemental` directly here as they imply `bindgen`.
    if cfg!(feature = "bindgen") || !matches!(source, LibrarySource::Vendored) {
        println!("cargo:warning=running bindgen");
        build_bindgen(&includes);
    }

    // 7. Export common rerun-if-changed metadata
    // Rerun if build.rs changes
    println!("cargo:rerun-if-changed=build.rs");

    if let LibrarySource::Vendored = source {
        println!("cargo:rerun-if-changed=nng/CMakeLists.txt");
        println!("cargo:rerun-if-changed=nng/include/nng/nng.h");
    }

    if cfg!(feature = "bindgen") {
        println!("cargo:rerun-if-changed=src/wrapper.h");
        println!("cargo:rerun-if-changed=src/compat.h");
        println!("cargo:rerun-if-changed=src/supplemental.h");
    }

    // Rerun if any environment variables change
    for var in ENV_VARS {
        println!("cargo:rerun-if-env-changed={}", var);

        // Also check target-prefixed variants
        let target = env::var("TARGET").unwrap();
        let prefixed = format!("{}_{}", target.replace("-", "_").to_uppercase(), var);
        println!("cargo:rerun-if-env-changed={}", prefixed);
    }
}

fn emit_cfg_directives() {
    // Make the cfg directive try_from known to rustc to suppress warnings
    println!("cargo::rustc-check-cfg=cfg(try_from)");
    if version_check::is_min_version("1.34.0").unwrap_or(false) {
        println!("cargo:rustc-cfg=try_from");
    }
}

/// Determine whether to force linking with the system-provided library.
fn force_no_vendor() -> bool {
    get_env_var("NNG_NO_VENDOR")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(false)
}

/// Determine whether to use static linking.
///
/// If not set in the environment, returns true for vendored builds and false for system builds.
fn should_link_static(vendored: bool) -> bool {
    get_env_var("NNG_STATIC")
        .map(|v| v != "0" && v.to_lowercase() != "false")
        .unwrap_or(vendored)
}

/// Get environment variable with target-prefixed fallback
///
/// First checks TARGET_PREFIXED_VAR (e.g., X86_64_UNKNOWN_LINUX_GNU_NNG_DIR),
/// then falls back to unprefixed VAR (e.g., NNG_DIR).
fn get_env_var(name: &str) -> Option<String> {
    let target = env::var("TARGET").unwrap();
    let target_prefixed = format!("{}_{}", target.replace("-", "_").to_uppercase(), name);

    env::var(&target_prefixed).or_else(|_| env::var(name)).ok()
}

/// Attempt to find system-installed NNG library.
/// Emits cargo:rustc-link-search and metadata directives directly.
/// Returns the library source if found.
fn try_find_system_library() -> Option<(LibrarySource, Vec<PathBuf>)> {
    // 1. Explicit environment variables (highest priority)
    if let Some(dir) = get_env_var("NNG_DIR") {
        let path = PathBuf::from(&dir);
        if !path.exists() {
            panic!("NNG_DIR points to non-existent directory: {}", dir);
        }
        println!("cargo:warning=Using NNG from NNG_DIR: {}", dir);

        let lib_dir = path.join("lib");
        let include_dir = path.join("include");

        println!("cargo:rustc-link-search=native={}", lib_dir.display());
        println!("cargo:include={}", include_dir.display());
        println!("cargo:root={}", path.display());

        return Some((LibrarySource::Manual, vec![include_dir]));
    }

    if let Some(lib_dir) = get_env_var("NNG_LIB_DIR") {
        let inc_dir = get_env_var("NNG_INCLUDE_DIR")
            .expect("NNG_INCLUDE_DIR must be set when NNG_LIB_DIR is set");

        let lib_path = PathBuf::from(&lib_dir);
        let inc_path = PathBuf::from(&inc_dir);

        if !lib_path.exists() {
            panic!("NNG_LIB_DIR points to non-existent directory: {}", lib_dir);
        }
        if !inc_path.exists() {
            panic!(
                "NNG_INCLUDE_DIR points to non-existent directory: {}",
                inc_dir
            );
        }

        println!("cargo:warning=Using NNG from NNG_LIB_DIR/NNG_INCLUDE_DIR");
        println!("cargo:rustc-link-search=native={}", lib_path.display());
        println!("cargo:include={}", inc_path.display());

        return Some((LibrarySource::Manual, vec![inc_path]));
    }

    // 2. pkg-config (standard package manager on Unix)
    // See: https://github.com/nanomsg/nng/issues/926
    if let Some((source, inc)) = try_pkg_config() {
        return Some((source, inc));
    }

    // 3. vcpkg (standard package manager on Windows MSVC)
    if let Some((source, inc)) = try_vcpkg() {
        return Some((source, inc));
    }

    // 4. Platform-specific paths (fallback)
    try_platform_specific_paths()
}

fn try_platform_specific_paths() -> Option<(LibrarySource, Vec<PathBuf>)> {
    // Try to compile without explicit paths, letting cc use system defaults.
    // This picks up CFLAGS, LDFLAGS, and other environment configuration.
    if try_compile_probe_without_paths() {
        // Success! The system compiler can find NNG without any help.
        // No need to emit link-search or metadata paths.
        println!("cargo:warning=Found NNG via system compiler defaults");
        let mut include_dirs = Vec::new();
        let cflags = get_env_var("CFLAGS").unwrap_or_default();
        let mut cflags = cflags.split_whitespace();
        while let Some(flag) = cflags.next() {
            if let Some(mut path) = flag.strip_prefix("-I") {
                if path.is_empty() {
                    // -I $path
                    let Some(next_path) = cflags.next() else {
                        break;
                    };
                    path = next_path;
                } else {
                    // -I$path
                }
                include_dirs.push(path.into());
            }
        }
        return Some((LibrarySource::Manual, include_dirs));
    }

    // The basic probe failed. On macOS, try Homebrew and MacPorts locations explicitly
    // since users often install via those but don't have CFLAGS/LDFLAGS set.
    let target = env::var("TARGET").unwrap();
    if target.contains("darwin") {
        #[allow(clippy::single_element_loop)]
        for prefix in &["/opt/homebrew", "/opt/local"] {
            let lib_dir = Path::new(prefix).join("lib");
            let include_path = Path::new(prefix).join("include/nng/nng.h");

            if include_path.exists() {
                println!("cargo:warning=Found NNG in Homebrew location: {}", prefix);

                let include_dir = Path::new(prefix).join("include");

                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                println!("cargo:include={}", include_dir.display());
                println!("cargo:root={}", prefix);

                return Some((LibrarySource::Manual, vec![include_dir]));
            }
        }
    }

    // No system library found
    None
}

/// Try to compile a probe program using system defaults
/// This respects CFLAGS, LDFLAGS, and other compiler configuration
///
/// NOTE: This only tests compilation (headers found, symbols declared), not linking (symbols
/// defined in library). Testing actual linking is convoluted as it needs platform-specific linker
/// flags and such. In practice, if we can find the headers using system paths, the library paths
/// are probably also set up, so this is good enough.
fn try_compile_probe_without_paths() -> bool {
    cc::Build::new()
        .cargo_metadata(false)
        .file("build-probe.c")
        .try_compile("nng_probe")
        .is_ok()
}

fn try_pkg_config() -> Option<(LibrarySource, Vec<PathBuf>)> {
    #[cfg(unix)]
    {
        if let Ok(lib) = pkg_config::Config::new()
            .atleast_version("1.0.0")
            .probe("nng")
        {
            println!("cargo:warning=Found NNG via pkg-config");

            // pkg-config::probe() already emitted cargo:rustc-link-search and
            // cargo:rustc-link-lib directives for ALL found paths.

            return Some((LibrarySource::PkgConfig, lib.include_paths));
        }
    }

    None
}

fn try_vcpkg() -> Option<(LibrarySource, Vec<PathBuf>)> {
    #[cfg(target_env = "msvc")]
    {
        // vcpkg is commonly used on Windows MSVC for C/C++ dependencies
        if let Ok(lib) = vcpkg::find_package("nng") {
            println!("cargo:warning=Found NNG via vcpkg");

            // vcpkg::find_package() already emitted cargo:rustc-link-search and
            // cargo:rustc-link-lib directives for ALL found paths.

            return Some((LibrarySource::Vcpkg, lib.include_paths));
        }
    }

    None
}

fn check_vendored_source() {
    let nng_dir = Path::new("nng");

    // In published crates, NNG sources are bundled as regular files
    // If they're missing, it's a bug in the packaging
    if !nng_dir.join("CMakeLists.txt").exists() {
        // If we're in a git repository (development), try to initialize submodule
        if Path::new(".git").exists() {
            use std::process::Command;

            println!(
                "cargo:warning=nng/CMakeLists.txt not found, attempting to initialize submodule"
            );

            if let Ok(status) = Command::new("git")
                .args(["submodule", "update", "--init", "--recursive"])
                .status()
            {
                if status.success() {
                    println!("cargo:warning=Successfully initialized nng submodule");
                    return;
                }
            }
        }

        // Either not in git repo or submodule init failed
        panic!("NNG vendored sources not found - this is a bug in the crate packaging");
    }
}

#[cfg(feature = "vendored")]
fn build_vendored() -> (LibrarySource, Vec<PathBuf>) {
    let stats = if cfg!(feature = "vendored-stats") {
        "ON"
    } else {
        "OFF"
    };
    let tls = if cfg!(feature = "tls") { "ON" } else { "OFF" };
    let compat = if cfg!(feature = "compat") {
        "ON"
    } else {
        "OFF"
    };

    // Determine link type
    let static_link = should_link_static(true);

    // Run cmake to build nng
    let mut config = cmake::Config::new("nng");
    config
        .define("NNG_TESTS", "OFF")
        .define("NNG_TOOLS", "OFF")
        .define("NNG_ENABLE_STATS", stats)
        .define("NNG_ENABLE_TLS", tls)
        .define("NNG_ENABLE_NNGCAT", "OFF")
        .define("NNG_ENABLE_COVERAGE", "OFF")
        .define("NNG_ENABLE_COMPAT", compat);

    // Set BUILD_SHARED_LIBS based on desired linkage
    if static_link {
        config.define("BUILD_SHARED_LIBS", "OFF");
    } else {
        config.define("BUILD_SHARED_LIBS", "ON");
    }

    if cfg!(target_env = "msvc") {
        // Rust always links against MSVC's Release CRT, so if we build nng against the Debug
        // version, we get linker errors. Thus, always build nng in Release to match Rust.
        config.profile("Release");
    }

    let dst = config.build();

    let lib_dir = if cfg!(target_env = "msvc") {
        // MSVC does not search recursively, so point specifically at the directory
        // that holds nng.lib
        dst.join("build/Release")
    } else {
        dst.join("lib")
    };

    let include_dir = dst.join("include");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:include={}", include_dir.display());
    println!("cargo:root={}", dst.display());
    println!("cargo:vendored=1");

    (LibrarySource::Vendored, vec![include_dir])
}

#[cfg(not(feature = "vendored"))]
fn build_vendored() -> (LibrarySource, Vec<PathBuf>) {
    panic!("vendored feature not enabled but vendored build requested");
}

fn build_bindgen(include_dirs: &[PathBuf]) {
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
        // NNG_ESYSERR and NNG_ETRANERR are used like flags
        .constified_enum("nng_errno_enum")
        .constified_enum("nng_pipe_ev")
        .use_core()
        .parse_callbacks(Box::new(BindgenCallbacks::default()))
        .size_t_is_usize(true)
        // Layout tests are non-portable; 64-bit tests fail on 32-bit
        // Don't output tests if we're regenerating `src/bindings.rs`
        .layout_tests(!cfg!(feature = "source-update-bindings"));

    // Add include path for nng headers
    for include_dir in include_dirs {
        builder = builder.clang_arg(format!("-I{}", include_dir.display()));
    }

    if cfg!(feature = "compat") {
        builder = builder.header("src/compat.h");
    }
    if cfg!(feature = "supplemental") {
        builder = builder.header("src/supplemental.h");
    }

    let out_file = PathBuf::from(env::var("OUT_DIR").unwrap()).join("bindings.rs");
    builder
        .generate()
        .expect("Unable to generate bindings")
        .write_to_file(&out_file)
        .expect("Couldn't write bindings");

    #[cfg(feature = "source-update-bindings")]
    {
        let bindings = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("src")
            .join("bindings.rs");
        std::fs::copy(&out_file, bindings).expect("Unable to update bindings");
    }
}

#[derive(Debug, Default)]
struct BindgenCallbacks;

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
