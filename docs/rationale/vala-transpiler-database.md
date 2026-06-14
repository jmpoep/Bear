# Recording Vala (valac) builds in the compilation database

## Context

`valac` is not a conventional compiler but a transpiler-driver: it
parses Vala (`.vala`) and Genie (`.gs`) sources, emits C, and -- unless
given `-C`/`--ccode` -- spawns a real C compiler (default command `cc`,
overridable by `--cc=` or `$CC`) on the generated C. Under interception
Bear therefore sees two processes: the `valac` invocation and the
internal `cc` on the generated C.

The consumer that motivated the request (issue #709) is
vala-language-server (VLS). Reading its source settled several forces:

- VLS reads only `directory`, `file`, and `command`; it classifies an
  entry as Vala by testing that `command[0]` contains `valac`, and it
  ignores the `output` field entirely.
- VLS understands only the `command` *string* form, not the `arguments`
  array. An entry carrying only `arguments` deserializes to an empty
  command and is silently skipped. Bear defaults to the array form.
- The same database may also be read by a C language server (clangd),
  which does not filter by driver: it background-indexes every entry and
  parses each with clang's driver, so `valac` entries produce
  unknown-argument noise.

Two valac specifics also surfaced during implementation: `valac`'s
source files (`.vala`, `.gs`) were not in Bear's source-extension
allowlist, so without adding them the `.vala` argument was classified as
a non-compilable input and produced no entry; and `-X`/`--Xcc` forward
one arbitrary token to the C compiler, a token that is ambiguous between
a compile flag (`-X -fPIC`) and a link flag (`-X -lm`).

## Decision

- **Emit valac entries; do not auto-switch the output format.** Bear
  records the `valac` invocation as a normal compilation. VLS needs the
  command-string form, so users point VLS at a database built with
  `format.entries.use_array_format: false`; this is documented in the man
  page rather than special-cased per compiler, keeping every entry in the
  database the same shape.
- **Keep the internal generated-C `cc` entries; do not filter them.** VLS
  ignores them, a C language server can use them, Bear cannot reliably
  attribute a `cc` invocation to a parent valac, and entry validation does
  not check file existence, so there is no clean suppression hook. The
  clangd-vs-`valac` friction is handled on the clangd side (a `.clangd`
  `PathMatch` excluding `*.vala`), not by changing the database.
- **Add only `.vala` and `.gs` to the source-extension allowlist.**
  `.vapi` and `.gir` are bindings consumed by valac, not translation
  units, so they must not generate entries.
- **Classify `-X`/`--Xcc` as compile-affecting, not linking.** Linking
  arguments are stripped from per-source compile entries; since the
  forwarded token is ambiguous, keeping it (and risking a harmless `-lm`)
  is preferable to dropping a compile-relevant flag.

## Consequences

- A default Bear database (array form) yields no usable entries for VLS;
  this is a documentation burden, not a silent failure -- the man page
  calls it out, and the format knob already exists.
- Vala source recognition is extension-gated, so a future Vala-family
  extension must be added to the allowlist explicitly (the same as every
  other language).
- The generated-C entries reference files that, for a non-`-C` build,
  live in a temporary directory and may not persist after the build.
  They are noise for clangd but are never wrong for VLS; revisit only if
  a concrete consumer needs them suppressed.
- `valac` is recognized with `versioned` and `cross_compilation` enabled:
  the former matches `valac-0.56`; the latter matches Debian/Ubuntu's
  triplet-prefixed `x86_64-linux-gnu-valac`, which is what their primary
  `valac` package installs.

## References

- Requirement: `output-compilation-entries`
- Issue #709 -- the feature request (valac support for vala-language-server)
- vala-language-server `src/projects/ccproject.vala` -- the `valac`
  command filter and command-string requirement
