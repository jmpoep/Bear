// SPDX-License-Identifier: GPL-3.0-or-later

//! Config-to-primitive adapter for the driver-side build environment.
//!
//! The interception mechanism itself (wrapper directory, preload injection,
//! masquerade resolution, supervised execution) lives in
//! `intercept_supervisor::runner`, which works in terms of primitives only
//! so it carries no `config`/`args`/`clap` dependency. This module is the thin
//! `bear`-side adapter that maps Bear's `config` and `args` types onto those
//! primitive calls.

use crate::{args, config};
use intercept_supervisor::context;
use intercept_supervisor::runner;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

pub use intercept_supervisor::ConfigurationError;

/// Driver-side build environment: a thin wrapper over
/// [`intercept_supervisor::BuildEnvironment`] that adapts Bear's `config` and
/// `args` types to the supervisor's primitive API.
pub struct BuildEnvironment {
    inner: runner::BuildEnvironment,
}

impl BuildEnvironment {
    /// Creates a new `BuildEnvironment` configured for the specified interception method.
    ///
    /// This method dispatches to the appropriate specialized creation method based on the
    /// configuration type (wrapper or preload mode). In both modes, the interceptor will
    /// report execution events via TCP sockets to the specified address.
    ///
    /// # Arguments
    ///
    /// * `intercept` - The interception configuration specifying the mode
    /// * `compilers` - The configured compilers (only the non-ignored ones are wrapped)
    /// * `address` - The socket address where the interceptor should report executions
    /// * `is_compiler` - Predicate used for PATH-based compiler discovery
    ///
    /// # Returns
    ///
    /// Returns a configured `BuildEnvironment` on success, or a `ConfigurationError`
    /// if the configuration is invalid or environment setup fails.
    pub fn create(
        context: &context::Context,
        intercept: &config::Intercept,
        compilers: &[config::Compiler],
        address: SocketAddr,
        is_compiler: impl Fn(&Path) -> bool,
    ) -> Result<Self, ConfigurationError> {
        let inner = match intercept {
            config::Intercept::Wrapper => {
                let executables: Vec<PathBuf> = compilers
                    .iter()
                    .filter(|compiler| !compiler.ignore)
                    .map(|compiler| compiler.path.clone())
                    .collect();
                runner::BuildEnvironment::create_as_wrapper(context, &executables, address, is_compiler)?
            }
            config::Intercept::Preload => runner::BuildEnvironment::create_as_preload(context, address)?,
        };
        Ok(Self { inner })
    }

    /// Executes a build command within the configured interception environment.
    ///
    /// Adapts the clap `args::BuildCommand` into the primitive argument slice
    /// the supervisor's `run_build` expects.
    pub fn run_build(
        &self,
        build_command: args::BuildCommand,
    ) -> Result<ExitStatus, intercept_supervisor::SuperviseError> {
        self.inner.run_build(&build_command.arguments)
    }
}

/// Creates an [`intercept::Execution`] from a build command with automatic
/// environment trimming.
///
/// The working directory is obtained from the current process, and environment
/// variables are filtered to include only those relevant for compilation
/// database generation.
///
/// This lives in `bear` (not the supervisor crate) because it depends on
/// [`args::BuildCommand`], which carries `clap` into the build graph; keeping it
/// here lets the supervisor stay `clap`-free.
pub fn execution_from_build_command(command: &args::BuildCommand) -> intercept::Execution {
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let environment = std::env::vars().collect();

    intercept::Execution {
        executable: PathBuf::from(&command.arguments[0]),
        arguments: command.arguments.clone(),
        working_dir,
        environment,
    }
    .trim()
}
