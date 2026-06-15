## Bear crate

This is the main library crate. It contains the CLI definitions, semantic
analysis, and output generation. The `bear-driver` and `bear-wrapper`
binaries live in their own crates (`crates/bear-driver`, `crates/bear-wrapper`)
and depend on this library; the shared/agent-side interception runtime lives
in `crates/intercept`, and the driver-side interception (supervise, TCP
collector, build environment) lives in `crates/intercept-supervisor`.

## Key directories

| Directory | Responsibility |
|---|---|
| `src/modes/` | Modes of operation |
| `src/environment.rs` | Config-to-primitive adapter over `intercept_supervisor::runner` |
| `src/output/` | Output generation (JSON compilation database, statistics) |
| `src/semantic/` | Semantic analysis - compiler detection and flag parsing |
| `src/config/` | Configuration loading, validation, types |
| `compilers/` | Compiler definition YAML files (see `compilers/CLAUDE.md`) |

## Before modifying

- **CLI arguments** (`src/args.rs`): uses `clap` derive macros. Update man page -- see `man/CLAUDE.md` for instructions.
- **Compiler interpreters**: read `compilers/CLAUDE.md` before editing YAML files.
- **Output format**: check existing integration tests in `tests/integration/` to avoid regressions.
- **Configuration types** (`src/config/types.rs`): changes here affect YAML config parsing. Update validation in `src/config/validation.rs`.

## Build script

`build.rs` invokes `compilers_codegen::generate` to read `compilers/*.yaml`
and produce static Rust arrays into `OUT_DIR`.

The install-layout name vars (`DRIVER_NAME`, `WRAPPER_NAME`, `PRELOAD_NAME`,
`INTERCEPT_LIBDIR`) are emitted by `crates/intercept-supervisor/build.rs`,
which is where `installation.rs` lives and reads them.

The generated code is included via `include!()` in the interpreter
and recognition modules. After editing YAML files, run `cargo build`
to regenerate, then `cargo test` to validate. See
`build-support/compilers-codegen/CLAUDE.md` for codegen design and
snapshot tests.

## Shell completions

Generated from `clap` definitions at build time:

```sh
target/release/generate-completions target/release/completions
```
