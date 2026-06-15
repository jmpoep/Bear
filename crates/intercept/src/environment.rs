// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::env::JoinPathsError;
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
pub(crate) const KEY_MAKE__C_COMPILER: &str = "CC";
pub(crate) const KEY_MAKE__CXX_COMPILER: &str = "CXX";
pub(crate) const KEY_MAKE__C_PREPROCESSOR: &str = "CPP";
pub(crate) const KEY_MAKE__FORTRAN_COMPILER: &str = "FC";
pub(crate) const KEY_MAKE__ARCHIVE: &str = "AR";
pub(crate) const KEY_MAKE__ASSEMBLER: &str = "AS";
pub(crate) const KEY_MAKE__MODULA_COMPILER: &str = "M2C";
pub(crate) const KEY_MAKE__PASCAL_COMPILER: &str = "PC";
pub(crate) const KEY_MAKE__LEX: &str = "LEX";
pub(crate) const KEY_MAKE__YACC: &str = "YACC";
pub(crate) const KEY_MAKE__LINT: &str = "LINT";

pub(crate) const KEY_MAKE__AR_FLAGS: &str = "ARFLAGS";
pub(crate) const KEY_MAKE__AS_FLAGS: &str = "ASFLAGS";
pub(crate) const KEY_MAKE__C_FLAGS: &str = "CFLAGS";
pub(crate) const KEY_MAKE__CXX_FLAGS: &str = "CXXFLAGS";
pub(crate) const KEY_MAKE__C_PREPROCESSOR_FLAGS: &str = "CPPFLAGS";
pub(crate) const KEY_MAKE__FORTRAN_FLAGS: &str = "FFLAGS";
pub(crate) const KEY_MAKE__LINKER_FLAGS: &str = "LDFLAGS";
pub(crate) const KEY_MAKE__LINKER_LIBS: &str = "LDLIBS";
pub(crate) const KEY_MAKE__LEX_FLAGS: &str = "LFLAGS";
pub(crate) const KEY_MAKE__YACC_FLAGS: &str = "YFLAGS";
pub(crate) const KEY_MAKE__PASCAL_FLAGS: &str = "PFLAGS";
pub(crate) const KEY_MAKE__LINT_FLAGS: &str = "LINTFLAGS";

// https://doc.rust-lang.org/cargo/reference/environment-variables.html
pub(crate) const KEY_CARGO__CARGO: &str = "CARGO";
pub(crate) const KEY_CARGO__RUSTC: &str = "RUSTC";
pub(crate) const KEY_CARGO__RUSTC_WRAPPER: &str = "RUSTC_WRAPPER";

pub(crate) const KEY_CARGO__RUSTFLAGS: &str = "RUSTFLAGS";

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Assert that the first entry in a path-like string equals the expected value.
    /// Works for PATH, LD_PRELOAD, or any path-separated environment variable.
    fn assert_first_path_entry(expected: &str, path_like: &str) {
        let path_entries: Vec<String> =
            std::env::split_paths(path_like).map(|p| p.to_string_lossy().to_string()).collect();
        let first_entry = path_entries.first().expect("Path-like string should not be empty");

        assert_eq!(
            first_entry, expected,
            "First path entry should match expected. First entry: {}, expected: {}",
            first_entry, expected
        );
    }

    fn assert_path_entry(expected: &str, path_like: &str) {
        let path_entries: Vec<String> =
            std::env::split_paths(path_like).map(|p| p.to_string_lossy().to_string()).collect();

        assert!(
            path_entries.contains(&expected.to_string()),
            "Path entry should contain expected. Path entries: {:?}, expected: {}",
            path_entries,
            expected
        );
    }

    #[test]
    fn test_insert_to_path_empty_original() {
        let original = "";
        let first = PathBuf::from("/usr/local/bin");
        let result = insert_to_path(original, first.clone()).unwrap();
        // For empty path case, we just return the path as a string
        assert_first_path_entry(&first.to_string_lossy(), &result);
    }

    #[test]
    fn test_insert_to_path_prepend_new() {
        let bin = PathBuf::from("/bin");
        let usr_bin = PathBuf::from("/usr/bin");
        let usr_local_bin = PathBuf::from("/usr/local/bin");

        // Join the original paths using platform-specific separator
        let original =
            std::env::join_paths([usr_bin.clone(), bin.clone()]).unwrap().to_string_lossy().to_string();

        // Apply our function
        let result = insert_to_path(&original, usr_local_bin.clone()).unwrap();

        // Check that the new path is first
        assert_first_path_entry(&usr_local_bin.to_string_lossy(), &result);
        assert_path_entry(&bin.to_string_lossy(), &result);
        assert_path_entry(&usr_bin.to_string_lossy(), &result);
    }

    #[test]
    fn test_insert_to_path_move_existing_to_front() {
        let bin = PathBuf::from("/bin");
        let usr_bin = PathBuf::from("/usr/bin");
        let usr_local_bin = PathBuf::from("/usr/local/bin");

        // Join the original paths using platform-specific separator
        let original = std::env::join_paths([usr_bin.clone(), usr_local_bin.clone(), bin.clone()])
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Apply our function
        let result = insert_to_path(&original, usr_local_bin.clone()).unwrap();

        // Check that the existing path was moved to front
        assert_first_path_entry(&usr_local_bin.to_string_lossy(), &result);
        assert_path_entry(&bin.to_string_lossy(), &result);
        assert_path_entry(&usr_bin.to_string_lossy(), &result);
    }

    #[test]
    fn test_insert_to_path_already_first() {
        let bin = PathBuf::from("/bin");
        let usr_bin = PathBuf::from("/usr/bin");
        let usr_local_bin = PathBuf::from("/usr/local/bin");

        // Join the original paths using platform-specific separator
        let original = std::env::join_paths([usr_local_bin.clone(), usr_bin.clone(), bin.clone()])
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Apply our function
        let result = insert_to_path(&original, usr_local_bin.clone()).unwrap();

        // Check that the path is still first (no change needed)
        assert_first_path_entry(&usr_local_bin.to_string_lossy(), &result);
        assert_path_entry(&bin.to_string_lossy(), &result);
        assert_path_entry(&usr_bin.to_string_lossy(), &result);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_insert_to_path_windows_mingw_preservation() {
        // Test the exact Windows CI failure scenario - MinGW PATH preservation
        let original = "C:\\mingw64\\bin;C:\\Windows\\System32;C:\\Program Files\\Git\\bin";
        let wrapper_dir = "C:\\Users\\RUNNER~1\\AppData\\Local\\Temp\\bear-xyz";
        let first = PathBuf::from(wrapper_dir);

        let result = insert_to_path(original, first).unwrap();

        // Wrapper should be first in PATH
        assert_first_path_entry(wrapper_dir, &result);
        assert_path_entry("C:\\mingw64\\bin", &result);
        assert_path_entry("C:\\Windows\\System32", &result);
        assert_path_entry("C:\\Program Files\\Git\\bin", &result);
    }
}
