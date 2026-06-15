// SPDX-License-Identifier: GPL-3.0-or-later

/// Driver executable name (platform-dependent)
#[cfg(windows)]
const DRIVER_NAME: &str = "bear-driver.exe";
#[cfg(not(windows))]
const DRIVER_NAME: &str = "bear-driver";

/// Wrapper executable name (platform-dependent)
#[cfg(windows)]
const WRAPPER_NAME: &str = "bear-wrapper.exe";
#[cfg(not(windows))]
const WRAPPER_NAME: &str = "bear-wrapper";

/// Preload library name (platform-dependent)
#[cfg(target_os = "macos")]
const PRELOAD_NAME: &str = "libexec.dylib";
#[cfg(not(target_os = "macos"))]
const PRELOAD_NAME: &str = "libexec.so";

fn main() {
    // `src/installation.rs` resolves the runtime install layout from these
    // compile-time names via `env!()`. They are validated and forwarded the
    // same way `crates/bear/build.rs` does for its own consumers.
    let intercept_libdir = std::env::var("INTERCEPT_LIBDIR").unwrap_or_else(|_| "lib".to_string());
    validate_intercept_libdir(&intercept_libdir);

    println!("cargo:rustc-env=DRIVER_NAME={}", DRIVER_NAME);
    println!("cargo:rustc-env=WRAPPER_NAME={}", WRAPPER_NAME);
    println!("cargo:rustc-env=PRELOAD_NAME={}", PRELOAD_NAME);
    println!("cargo:rustc-env=INTERCEPT_LIBDIR={}", intercept_libdir);
    println!("cargo:rerun-if-env-changed=INTERCEPT_LIBDIR");

    // The compiler-specific environment variable names (`COMPILER_ENV_KEYS`)
    // are generated from the same `compilers/*.yaml` descriptions the `bear`
    // crate uses. The agent-side environment filtering in `src/environment.rs`
    // includes the result via `include!(OUT_DIR/env_keys.rs)`.
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let flags_dir = manifest_dir.join("../bear/compilers");
    let out_dir: std::path::PathBuf = std::env::var("OUT_DIR").unwrap().into();

    if let Err(e) = compilers_codegen::generate_env_keys_only(&flags_dir, &out_dir) {
        eprintln!("error: env-key codegen failed");
        for cause in e.chain() {
            eprintln!("  caused by: {}", cause);
        }
        std::process::exit(1);
    }
}

/// Validate the `INTERCEPT_LIBDIR` environment variable.
///
/// Valid values:
/// - A non-empty relative path (e.g. `"lib"`, `"lib64"`, `"lib/x86_64-linux-gnu"`).
/// - The literal string `"$LIB"` to defer directory selection to runtime/platform
///   conventions (commonly interpreted as either `"lib"` or `"lib64"` depending on
///   the system).
fn validate_intercept_libdir(value: &str) {
    let value = value.trim();

    if value.is_empty() {
        panic!("INTERCEPT_LIBDIR must not be empty or whitespace-only");
    }

    if value == "$LIB" {
        return;
    }

    let path = std::path::Path::new(value);
    if path.is_absolute() {
        panic!("INTERCEPT_LIBDIR must be a relative path, got: {}", value);
    }
}
