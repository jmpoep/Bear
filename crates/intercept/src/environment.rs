// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::env::JoinPathsError;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

pub const KEY_INTERCEPT_STATE: &str = "BEAR_INTERCEPT";

// man page for `ld.so` (Linux dynamic linker/loader)
pub const KEY_OS__PRELOAD_PATH: &str = "LD_PRELOAD";
// man page for `dyld` (macOS dynamic linker)
pub const KEY_OS__MACOS_PRELOAD_PATH: &str = "DYLD_INSERT_LIBRARIES";
pub const KEY_OS__MACOS_FLAT_NAMESPACE: &str = "DYLD_FORCE_FLAT_NAMESPACE";
// man page for `exec` (Linux system call)
pub const KEY_OS__PATH: &str = "PATH";

// Compiler-specific environment variable names, generated from compilers/*.yaml.
include!(concat!(env!("OUT_DIR"), "/env_keys.rs"));

// https://www.gnu.org/software/make/manual/html_node/Implicit-Variables.html
pub const KEY_MAKE__C_COMPILER: &str = "CC";
pub const KEY_MAKE__CXX_COMPILER: &str = "CXX";
pub const KEY_MAKE__C_PREPROCESSOR: &str = "CPP";
pub const KEY_MAKE__FORTRAN_COMPILER: &str = "FC";
pub const KEY_MAKE__ARCHIVE: &str = "AR";
pub const KEY_MAKE__ASSEMBLER: &str = "AS";
pub const KEY_MAKE__MODULA_COMPILER: &str = "M2C";
pub const KEY_MAKE__PASCAL_COMPILER: &str = "PC";
pub const KEY_MAKE__LEX: &str = "LEX";
pub const KEY_MAKE__YACC: &str = "YACC";
pub const KEY_MAKE__LINT: &str = "LINT";

pub const KEY_MAKE__AR_FLAGS: &str = "ARFLAGS";
pub const KEY_MAKE__AS_FLAGS: &str = "ASFLAGS";
pub const KEY_MAKE__C_FLAGS: &str = "CFLAGS";
pub const KEY_MAKE__CXX_FLAGS: &str = "CXXFLAGS";
pub const KEY_MAKE__C_PREPROCESSOR_FLAGS: &str = "CPPFLAGS";
pub const KEY_MAKE__FORTRAN_FLAGS: &str = "FFLAGS";
pub const KEY_MAKE__LINKER_FLAGS: &str = "LDFLAGS";
pub const KEY_MAKE__LINKER_LIBS: &str = "LDLIBS";
pub const KEY_MAKE__LEX_FLAGS: &str = "LFLAGS";
pub const KEY_MAKE__YACC_FLAGS: &str = "YFLAGS";
pub const KEY_MAKE__PASCAL_FLAGS: &str = "PFLAGS";
pub const KEY_MAKE__LINT_FLAGS: &str = "LINTFLAGS";

// https://doc.rust-lang.org/cargo/reference/environment-variables.html
pub const KEY_CARGO__CARGO: &str = "CARGO";
pub const KEY_CARGO__RUSTC: &str = "RUSTC";
pub const KEY_CARGO__RUSTC_WRAPPER: &str = "RUSTC_WRAPPER";

pub const KEY_CARGO__RUSTFLAGS: &str = "RUSTFLAGS";

static MAKE_PROGRAM_KEYS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    [
        KEY_MAKE__C_COMPILER,
        KEY_MAKE__CXX_COMPILER,
        KEY_MAKE__C_PREPROCESSOR,
        KEY_MAKE__FORTRAN_COMPILER,
        KEY_MAKE__ARCHIVE,
        KEY_MAKE__ASSEMBLER,
        KEY_MAKE__MODULA_COMPILER,
        KEY_MAKE__PASCAL_COMPILER,
        KEY_MAKE__LEX,
        KEY_MAKE__YACC,
        KEY_MAKE__LINT,
    ]
    .iter()
    .cloned()
    .collect()
});

static MAKE_FLAGS_KEYS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    [
        KEY_MAKE__AR_FLAGS,
        KEY_MAKE__AS_FLAGS,
        KEY_MAKE__C_FLAGS,
        KEY_MAKE__CXX_FLAGS,
        KEY_MAKE__C_PREPROCESSOR_FLAGS,
        KEY_MAKE__FORTRAN_FLAGS,
        KEY_MAKE__LINKER_FLAGS,
        KEY_MAKE__LINKER_LIBS,
        KEY_MAKE__LEX_FLAGS,
        KEY_MAKE__YACC_FLAGS,
        KEY_MAKE__PASCAL_FLAGS,
        KEY_MAKE__LINT_FLAGS,
    ]
    .iter()
    .cloned()
    .collect()
});

static CARGO_PROGRAM_KEYS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    [KEY_CARGO__CARGO, KEY_CARGO__RUSTC, KEY_CARGO__RUSTC_WRAPPER].iter().cloned().collect()
});

static CARGO_FLAGS_KEYS: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| [KEY_CARGO__RUSTFLAGS].iter().cloned().collect());

pub fn relevant_env(key: &str) -> bool {
    matches!(key, KEY_INTERCEPT_STATE | KEY_OS__PRELOAD_PATH | KEY_OS__MACOS_PRELOAD_PATH | KEY_OS__MACOS_FLAT_NAMESPACE)
        || MAKE_PROGRAM_KEYS.contains(key)
        || MAKE_FLAGS_KEYS.contains(key)
        || CARGO_PROGRAM_KEYS.contains(key)
        || CARGO_FLAGS_KEYS.contains(key)
        || COMPILER_ENV_KEYS.contains(&key)
        // Windows PATH variable is case sensitive and not always capitalized
        || key.to_uppercase() == KEY_OS__PATH
}

pub fn program_env(key: &str) -> bool {
    MAKE_PROGRAM_KEYS.contains(key) || CARGO_PROGRAM_KEYS.contains(key)
}

/// Represents the state information needed for preload-based interception.
///
/// This struct is serialized to JSON and passed to the preloaded library via
/// an environment variable. It contains all the information the library needs
/// to report execution events back to the Bear process.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct PreloadState {
    /// The socket address where execution events should be reported
    pub destination: SocketAddr,
    /// The path to the preload library itself
    pub library: PathBuf,
}

impl TryInto<String> for PreloadState {
    type Error = serde_json::Error;

    fn try_into(self) -> Result<String, Self::Error> {
        serde_json::to_string(&self)
    }
}
impl TryFrom<&str> for PreloadState {
    type Error = serde_json::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(value)
    }
}

impl TryFrom<String> for PreloadState {
    type Error = serde_json::Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        serde_json::from_str(&value)
    }
}

/// Manipulates a `PATH`-like environment variable by inserting a path at the beginning.
///
/// This function ensures that the specified path appears first in a colon-separated
/// list of paths (like `PATH` or `LD_PRELOAD`). If the path already exists elsewhere
/// in the list, it is removed from its current position and moved to the front.
/// This guarantees that the specified path takes precedence over other paths.
///
/// # Arguments
///
/// * `original` - The original PATH-like environment variable value
/// * `first` - The path to insert at the beginning of the path list
///
/// # Returns
///
/// Returns `Ok(String)` containing the updated path list, or `Err(JoinPathsError)`
/// if path manipulation fails due to invalid characters or platform limitations.
///
/// # Behavior
///
/// - If `original` is empty, returns just the `first` path
/// - If `first` already exists in `original`, it's moved to the front
/// - If `first` doesn't exist, it's prepended to the existing paths
/// - Uses platform-appropriate path separators and handles path encoding
pub fn insert_to_path<P: AsRef<Path>>(original: &str, first: P) -> Result<String, JoinPathsError> {
    let first_path = first.as_ref();

    if original.is_empty() {
        return Ok(first_path.to_string_lossy().to_string());
    }

    let mut paths: Vec<PathBuf> =
        std::env::split_paths(original).filter(|path| path.as_path() != first_path).collect();
    paths.insert(0, first_path.to_owned());
    std::env::join_paths(paths).map(|os_string| os_string.into_string().unwrap_or_default())
}
