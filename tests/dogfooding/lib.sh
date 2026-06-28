#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sourced helpers for the dogfooding harness: logging, outcome/exit-code
# constants, preflight checks, and podman wrappers. Keeping these here keeps
# run.sh readable. POSIX sh only; no bashisms.

# --- outcome / exit codes (dogfood-build-failure-taxonomy) -------------------
#
# PASS         = 0  golden regression check passed
# FAIL         = 1  regression: golden mismatch (a real behavioral change)
# INCONCLUSIVE = 2  the target build failed for its own reasons
#                   (configure/make/source-fetch/network/sha/OOM)
# ERROR        = 3  harness or Bear-infra failure (podman missing, disk/digest
#                   preflight, base build, empty capture)
EXIT_PASS=0
EXIT_FAIL=1
EXIT_INCONCLUSIVE=2
EXIT_ERROR=3

# --- logging (everything to stderr; stdout is reserved for data) -------------

log()  { printf '%s\n' "$*" >&2; }
info() { printf '[dogfood] %s\n' "$*" >&2; }
warn() { printf '[dogfood] WARN: %s\n' "$*" >&2; }
err()  { printf '[dogfood] ERROR: %s\n' "$*" >&2; }

# Print the single final status line and exit with the matching code. Takes
# the outcome name and a one-line reason.
finish() {
    _outcome="$1"
    _reason="$2"
    case "$_outcome" in
        PASS)         _code="$EXIT_PASS" ;;
        FAIL)         _code="$EXIT_FAIL" ;;
        INCONCLUSIVE) _code="$EXIT_INCONCLUSIVE" ;;
        ERROR)        _code="$EXIT_ERROR" ;;
        *)            _code="$EXIT_ERROR"; _outcome="ERROR" ;;
    esac
    printf '\n[dogfood] OUTCOME: %s (exit %s) - %s\n' "$_outcome" "$_code" "$_reason" >&2
    exit "$_code"
}

# --- podman wrappers ---------------------------------------------------------

require_podman() {
    if ! command -v podman >/dev/null 2>&1; then
        err "podman not found on PATH"
        finish ERROR "podman is required but not installed"
    fi
}

# Remove a container if it exists; ignore errors (best-effort cleanup).
rm_container() {
    podman rm -f "$1" >/dev/null 2>&1 || true
}

# --- preflight (dogfood-preflight) -------------------------------------------

# (a) Free-disk check on the podman graphroot against a per-target minimum.
preflight_disk() {
    _min_kib="$1"
    _graphroot="$(podman info --format '{{.Store.GraphRoot}}' 2>/dev/null)"
    if [ -z "$_graphroot" ]; then
        err "could not determine podman graphroot"
        return 1
    fi
    # df -Pk: POSIX format, 1024-byte blocks. Field 4 is available blocks.
    _avail_kib="$(df -Pk "$_graphroot" 2>/dev/null | awk 'NR==2 {print $4}')"
    if [ -z "$_avail_kib" ]; then
        err "could not read free disk for graphroot $_graphroot"
        return 1
    fi
    info "graphroot $_graphroot has ${_avail_kib} KiB free (need ${_min_kib} KiB)"
    if [ "$_avail_kib" -lt "$_min_kib" ]; then
        err "insufficient free disk on $_graphroot: ${_avail_kib} KiB < ${_min_kib} KiB"
        return 1
    fi
    return 0
}

# (b) Resolve/verify the pinned base image is present or pullable. Pull by
# digest is idempotent; a failure here means the pin is unreachable.
preflight_image() {
    _image="$1"
    if podman image exists "$_image" 2>/dev/null; then
        info "pinned base image already present: $_image"
        return 0
    fi
    info "pulling pinned base image: $_image"
    if ! podman pull "$_image" >&2; then
        err "could not pull pinned base image: $_image"
        return 1
    fi
    return 0
}

# --- oracle validation helpers (dogfood-oracle-cmake, dogfood-divergence-report)
#
# The oracle path compares Bear's capture against the database CMake itself
# emits (CMAKE_EXPORT_COMPILE_COMMANDS=ON), scoped to the intersection of
# translation units. The host comparator (cdb-compare) does the matching and
# emits its three-set {only_in_a, only_in_b, differing} JSON; these helpers
# only PREPARE its inputs and POST-PROCESS its output with jq (bucket extras,
# apply the allow-list). This is not a second comparison implementation.

# The oracle path needs jq on the host to normalize the databases and to
# bucket/allow-list the comparator's report.
require_jq() {
    if ! command -v jq >/dev/null 2>&1; then
        err "jq not found on PATH (required for the oracle validation path)"
        finish ERROR "jq is required for oracle targets but not installed"
    fi
}

# Normalize a database's 'output' field to the ABSOLUTE object path, so the
# comparator matches translation units by (file, absolute-output) and the two
# producers' differing output encodings align.
#
# Bear records 'output' relative to the entry 'directory'; CMake records it
# relative to the top-level build dir. We therefore rebase each to an absolute
# path: Bear by joining 'directory', CMake by joining the pinned BUILD_DIR.
# Matching on the absolute object path (rather than stripping 'output') keeps a
# source compiled into several targets distinct, so a multi-target source is
# not collapsed onto one key and falsely paired against the wrong target.
#
# Args: <kind: bear|cmake> <build_dir> <in.json> <out.json>
oracle_normalize_output() {
    _kind="$1"; _build="$2"; _in="$3"; _out="$4"
    case "$_kind" in
        bear)  jq 'map(.output = (.directory + "/" + .output))' "$_in" > "$_out" ;;
        cmake) jq --arg b "$_build" 'map(.output = ($b + "/" + .output))' "$_in" > "$_out" ;;
        *) err "oracle_normalize_output: unknown kind '$_kind'"; return 1 ;;
    esac
}

# Read the committed allow-list and build the oracle report. The report buckets
# the comparator's three sets: only_in_a (Bear-only) and only_in_b (CMake-only)
# are EXTRAS (logged, never a failure - dogfood-divergence-report); differing
# entries are the gate. Each allow-list rule is applied SYMMETRICALLY to both
# sides of a differing entry; an entry whose argument lists become equal after
# stripping the allow-listed tokens is a known-benign difference and is dropped.
# Survivors are real divergences.
#
# Writes the full report JSON to <report.json> and prints the survivor count to
# stdout. Returns 0 iff there are no survivors.
#
# Args: <cmp.json> <allowlist.txt> <report.json>
oracle_report() {
    _cmp="$1"; _allow="$2"; _report="$3"

    # Two rule kinds (see the allow-list header): 'flag <TOK>' drops the literal
    # token; 'flag-with-arg <TOK>' drops the token and the one argument it
    # consumes. Comments (#) and blank lines are ignored.
    _flags="$(awk '!/^[[:space:]]*#/ && $1=="flag"{print $2}' "$_allow" | jq -R . | jq -s .)"
    _pairs="$(awk '!/^[[:space:]]*#/ && $1=="flag-with-arg"{print $2}' "$_allow" | jq -R . | jq -s .)"

    jq --argjson flags "$_flags" --argjson pairs "$_pairs" '
        def strip:
            reduce .[] as $t ({acc: [], eat: false};
                if .eat then {acc: .acc, eat: false}
                elif ($flags|index($t)) then {acc: .acc, eat: false}
                elif ($pairs|index($t)) then {acc: .acc, eat: true}
                else {acc: (.acc + [$t]), eat: false} end
            ) | .acc;
        {
            bear_only:       (.only_in_a | length),
            cmake_only:      (.only_in_b | length),
            differing_total: (.differing | length),
            extras: {
                bear_only:  [ .only_in_a[] | {file, output} ],
                cmake_only: [ .only_in_b[] | {file, output} ]
            },
            survivors: [ .differing[]
                | select((.a_directory != .b_directory)
                         or ((.a_arguments|strip) != (.b_arguments|strip))) ]
        } | . + { survivor_count: (.survivors | length) }
    ' "$_cmp" > "$_report"

    _survivors="$(jq '.survivor_count' "$_report")"
    printf '%s\n' "$_survivors"
    # A non-integer (empty or "null") means the jq report write failed: signal an
    # infra error (return 2) distinct from a real mismatch (return 1), so run.sh
    # routes it to ERROR rather than FAIL.
    case "$_survivors" in
        ''|*[!0-9]*) return 2 ;;
    esac
    [ "$_survivors" -eq 0 ]
}
