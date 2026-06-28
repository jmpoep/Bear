#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Dogfooding harness entry point (Stage 2). One command spins up a per-project
# throwaway build container, runs the installed release Bear inside it against
# a pinned target, and passes/fails the captured compilation database against a
# committed golden (dogfood-golden-regression).
#
# Two validation modes, selected per-target by VALIDATION in config.env:
#   golden  (zlib, Stage 2): gate the capture against a committed golden CDB.
#   oracle  (curl, Stage 3): gate the capture against the database CMake itself
#           emits, on the intersection of translation units, with an allow-list
#           (dogfood-oracle-cmake, dogfood-divergence-report).
#
# Usage:
#   tests/dogfooding/run.sh [--label L] [--rebless] [--keep] [zlib|curl]
#
#   --label L   name the per-run results subdirectory (default: local)
#   --rebless   regenerate the committed golden from this run instead of
#               gating against it (dogfood-golden-rebless; golden targets only)
#   --keep      keep the throwaway container and scratch instead of removing
#   zlib|curl   target name (default: zlib)
#
# Outcomes / exit codes (dogfood-build-failure-taxonomy):
#   PASS=0  FAIL=1  INCONCLUSIVE=2  ERROR=3
# Runtime model: host-orchestrated rootless Podman (feasibility.md Option C).

set -eu

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

# shellcheck source=tests/dogfooding/lib.sh
. "$HERE/lib.sh"

# --- argument parsing --------------------------------------------------------

LABEL="local"
REBLESS=0
KEEP=0
TARGET=""

while [ $# -gt 0 ]; do
    case "$1" in
        --label) shift; [ $# -gt 0 ] || finish ERROR "--label needs a value"; LABEL="$1" ;;
        --label=*) LABEL="${1#--label=}" ;;
        --rebless) REBLESS=1 ;;
        --keep) KEEP=1 ;;
        -h|--help)
            sed -n '4,26p' "$HERE/run.sh" >&2
            exit 0 ;;
        --*) finish ERROR "unknown option: $1" ;;
        *)
            if [ -n "$TARGET" ]; then finish ERROR "only one target supported, got extra: $1"; fi
            TARGET="$1" ;;
    esac
    shift
done
[ -n "$TARGET" ] || TARGET="zlib"

# Both values become path segments and a container-name segment; reject anything
# that could traverse out of the harness tree or break a podman name.
case "$TARGET" in *[!A-Za-z0-9_-]*) finish ERROR "target must be [A-Za-z0-9_-]: '$TARGET'" ;; esac
case "$LABEL"  in *[!A-Za-z0-9_-]*) finish ERROR "label must be [A-Za-z0-9_-]: '$LABEL'" ;; esac

TARGET_DIR="$HERE/targets/$TARGET"
[ -d "$TARGET_DIR" ] || finish ERROR "unknown target '$TARGET' (no $TARGET_DIR)"

# shellcheck source=tests/dogfooding/targets/zlib/config.env
. "$TARGET_DIR/config.env"

# Per-target validation selector (dogfood-oracle-cmake). zlib's config predates
# this selector and omits it, so golden is the default; curl sets oracle.
VALIDATION="${VALIDATION:-golden}"
case "$VALIDATION" in
    golden|oracle) ;;
    *) finish ERROR "VALIDATION must be golden or oracle, got '$VALIDATION'" ;;
esac

GOLDEN="$HERE/goldens/$TARGET/compile_commands.json"
RESULTS_DIR="$HERE/results/$TARGET/$LABEL"
mkdir -p "$RESULTS_DIR"

# Image / container names derived from the Bear commit under test, so a stale
# cached image is never silently reused for a different Bear.
BEAR_SHA="$(cd "$REPO_ROOT" && git rev-parse --short HEAD)"
BASE_TAG="bear-dogfood-base:$BEAR_SHA"
TARGET_TAG="bear-dogfood-$TARGET:$BEAR_SHA"
CONTAINER="bear-dogfood-$TARGET-$LABEL-$$"

info "target=$TARGET label=$LABEL bear=$BEAR_SHA rebless=$REBLESS keep=$KEEP"

# Cleanup of the throwaway container unless --keep. Cached images are left in
# place (mentioned in the final report); the harness only removes what it spun
# up per run.
cleanup() {
    if [ "$KEEP" -eq 1 ]; then
        info "--keep: leaving container $CONTAINER in place"
    else
        rm_container "$CONTAINER"
    fi
}
trap cleanup EXIT INT TERM

# === STEP 1: PREFLIGHT (dogfood-preflight) ===================================
# Fail fast with a clear diagnostic BEFORE creating any scratch, so a run never
# starts only to leave a torn container behind.

require_podman

preflight_disk "$MIN_FREE_KIB" || finish ERROR "disk preflight failed"
preflight_image "$BASE_IMAGE"  || finish ERROR "pinned base image unavailable: $BASE_IMAGE"

# The host comparator is the gate; check it now so a missing binary fails fast
# instead of after the multi-minute image builds and the real run.
CDB_COMPARE="$REPO_ROOT/target/release/cdb-compare"
if [ ! -x "$CDB_COMPARE" ]; then
    err "host cdb-compare not found at $CDB_COMPARE"
    err "build it with: cargo build --release -p bear-test-tools --bin cdb-compare"
    finish ERROR "host cdb-compare binary missing"
fi

# The oracle path post-processes the comparator's JSON (bucket extras, apply the
# allow-list) with jq, and --rebless has no meaning without a golden.
if [ "$VALIDATION" = "oracle" ]; then
    require_jq
    if [ "$REBLESS" -eq 1 ]; then
        finish ERROR "--rebless applies only to golden targets; $TARGET validates against the CMake oracle"
    fi
fi

# === STEP 2: BUILD BASE IMAGE (dogfood-run-containerized) =====================
# Build Bear-under-test inside the base image from a 'git archive HEAD' context
# (committed files ONLY; the dirty working tree, plan.md, target/ never reach
# the image). A failure here is ERROR (harness / Bear build).

ARCHIVE_DIR="$(mktemp -d)"
cleanup_archive() { rm -rf "$ARCHIVE_DIR"; }
# Chain archive cleanup onto the container cleanup.
trap 'cleanup; cleanup_archive' EXIT INT TERM

info "exporting committed tree (git archive HEAD) to $ARCHIVE_DIR"
(cd "$REPO_ROOT" && git archive HEAD | tar -x -C "$ARCHIVE_DIR")

# 'git archive HEAD' inherently carries only committed files (never the dirty
# working tree, target/, or uncommitted scratch like plan.md). Assert positively
# that the harness itself made it in, rather than blocklisting scratch names that
# could one day be legitimately committed.
[ -f "$ARCHIVE_DIR/tests/dogfooding/run.sh" ] || \
    finish ERROR "git archive is missing tests/dogfooding (commit the harness before building)"

info "building base image $BASE_TAG"
if ! podman build \
        --build-arg "BASE_IMAGE=$BASE_IMAGE" \
        -f "$ARCHIVE_DIR/tests/dogfooding/base/Containerfile" \
        -t "$BASE_TAG" \
        "$ARCHIVE_DIR" >&2; then
    finish ERROR "base image build failed (Bear build or install)"
fi
cleanup_archive
trap cleanup EXIT INT TERM

# === STEP 3: BUILD TARGET IMAGE ==============================================
# FROM the locally-built base. dnf/curl/sha/network failures here are the
# target's own infra -> INCONCLUSIVE. A failure resolving the base would be
# ERROR, but step 2 just produced it, so a failure here is target infra.

# Each target pins its source under its own URL/SHA256 variable names (ZLIB_*,
# CURL_*); both Containerfiles consume the same SRC_DIR. Pass the variables the
# present target defines, so a target only ever sees its own build-args.
info "building target image $TARGET_TAG"
set -- --build-arg "BASE_TAG=$BASE_TAG" --build-arg "SRC_DIR=$SRC_DIR"
[ -n "${ZLIB_URL:-}" ]    && set -- "$@" --build-arg "ZLIB_URL=$ZLIB_URL"
[ -n "${ZLIB_SHA256:-}" ] && set -- "$@" --build-arg "ZLIB_SHA256=$ZLIB_SHA256"
[ -n "${CURL_URL:-}" ]    && set -- "$@" --build-arg "CURL_URL=$CURL_URL"
[ -n "${CURL_SHA256:-}" ] && set -- "$@" --build-arg "CURL_SHA256=$CURL_SHA256"
if ! podman build "$@" \
        -f "$TARGET_DIR/Containerfile" \
        -t "$TARGET_TAG" \
        "$TARGET_DIR" >&2; then
    finish INCONCLUSIVE "target image build failed (source fetch / sha / network / deps)"
fi

# === STEP 4: NON-EMPTY-CAPTURE SMOKE =========================================
# (dogfood-preflight + dogfood-run-containerized) Before the real build, prove
# interception actually works: a wrong libexec/INTERCEPT_LIBDIR layout makes
# Bear run yet capture nothing. Empty capture => ERROR.

info "smoke: verifying Bear captures a trivial compile"
SMOKE_OUT="$(podman run --rm --systemd=always "$TARGET_TAG" sh -c '
    set -e
    d="$(mktemp -d)"
    cd "$d"
    printf "int main(void){return 0;}\n" > smoke.c
    bear --output cc.json -- gcc -c smoke.c -o smoke.o >/dev/null 2>&1
    cat cc.json
' 2>/dev/null)" || SMOKE_OUT=""

case "$SMOKE_OUT" in
    *smoke.c*) info "smoke: capture OK" ;;
    *)
        err "smoke capture empty or missing smoke.c"
        err "diagnostic: libexec/INTERCEPT_LIBDIR mismatch: Bear ran but captured nothing"
        finish ERROR "non-empty-capture smoke failed (interception not working)"
        ;;
esac

# === STEP 5: REAL RUN (dogfood-run-containerized + dogfood-fixed-paths) =======
# Run the real build wrapped by Bear at fixed path /src. Configure/make failure
# => INCONCLUSIVE (target's own reasons). Empty captured CDB => ERROR.

FRESH="$RESULTS_DIR/compile_commands.json"
BUILD_LOG="$RESULTS_DIR/build.log"

info "running real build in container $CONTAINER"
set +e
podman run --systemd=always --name "$CONTAINER" "$TARGET_TAG" sh -c "
    set -e
    mkdir -p /out
    $TARGET_BUILD_CMD
" >"$BUILD_LOG" 2>&1
BUILD_RC=$?
set -e

if [ "$BUILD_RC" -ne 0 ]; then
    # Distinguish a container that never started (host/cgroup/image infra =>
    # ERROR) from one that ran but whose build failed for its own reasons
    # (=> INCONCLUSIVE). If the container does not exist, podman run never
    # launched it.
    if ! podman container exists "$CONTAINER" 2>/dev/null; then
        err "podman run never started the container; see $BUILD_LOG"
        finish ERROR "podman run failed to start container (systemd/cgroup/image infra)"
    fi
    err "target build exited $BUILD_RC; see $BUILD_LOG"
    finish INCONCLUSIVE "target build failed (configure/make) - log at $BUILD_LOG"
fi

# Copy the captured CDB out of the stopped container (sidesteps SELinux relabel).
if ! podman cp "$CONTAINER:/out/compile_commands.json" "$FRESH" >&2; then
    finish ERROR "could not copy compile_commands.json out of the container"
fi

# Empty / entry-less capture => ERROR (Bear ran but captured nothing).
if ! grep -q '"file"' "$FRESH" 2>/dev/null; then
    err "captured CDB has no entries: $FRESH"
    finish ERROR "empty capture from real build (interception produced nothing)"
fi
info "captured CDB: $FRESH"

# For oracle targets, also pull CMake's own database (the reference oracle the
# in-container build wrote to /out/oracle.json).
ORACLE="$RESULTS_DIR/oracle.json"
if [ "$VALIDATION" = "oracle" ]; then
    if ! podman cp "$CONTAINER:/out/oracle.json" "$ORACLE" >&2; then
        finish ERROR "could not copy oracle.json (CMake's database) out of the container"
    fi
    if ! grep -q '"file"' "$ORACLE" 2>/dev/null; then
        err "CMake oracle DB has no entries: $ORACLE"
        finish ERROR "empty CMake oracle database (configure did not export compile_commands)"
    fi
    info "captured oracle CDB: $ORACLE"
fi

# === STEP 6: GATE ============================================================
# Dispatch on the per-target validation selector: oracle (curl, Stage 3) or
# golden (zlib, Stage 2).

if [ "$VALIDATION" = "oracle" ]; then
    # --- oracle gate (dogfood-oracle-cmake, dogfood-divergence-report) -------
    # Match Bear's capture against CMake's database on the intersection of
    # translation units, identified by (file, absolute-output). The host
    # cdb-compare does the matching and emits its three-set JSON; jq then
    # buckets the only_in_* extras (logged, never a failure) and applies the
    # committed allow-list to the differing set (the gate).
    ALLOWLIST="$TARGET_DIR/oracle-allowlist.txt"
    [ -f "$ALLOWLIST" ] || finish ERROR "missing oracle allow-list at $ALLOWLIST"

    BEAR_NORM="$RESULTS_DIR/bear.norm.json"
    ORACLE_NORM="$RESULTS_DIR/oracle.norm.json"
    CMP_JSON="$RESULTS_DIR/oracle-compare.json"
    REPORT="$RESULTS_DIR/oracle-report.json"

    info "normalizing output fields to absolute object paths (build dir $BUILD_DIR)"
    oracle_normalize_output bear  "$BUILD_DIR" "$FRESH"  "$BEAR_NORM"   || finish ERROR "jq normalize of Bear DB failed"
    oracle_normalize_output cmake "$BUILD_DIR" "$ORACLE" "$ORACLE_NORM" || finish ERROR "jq normalize of CMake DB failed"

    # Bear is side A, CMake is side B; substitute-compiler cc absorbs the
    # compiler-driver path difference between the make-time command and CMake's
    # configure-time command. Non-zero exit here just means "not equivalent",
    # which is expected (the depfile flags differ); the gate is the allow-list.
    info "comparing matched TUs (cdb-compare, substitute-compiler cc)"
    CMP_ERR="$RESULTS_DIR/oracle-compare.stderr"
    "$CDB_COMPARE" compare --substitute-compiler cc --format json "$BEAR_NORM" "$ORACLE_NORM" >"$CMP_JSON" 2>"$CMP_ERR" || true
    if ! grep -q 'differing' "$CMP_JSON" 2>/dev/null; then
        finish ERROR "cdb-compare did not produce a report (see $CMP_JSON and $CMP_ERR)"
    fi

    info "applying allow-list and bucketing extras"
    set +e
    SURVIVORS="$(oracle_report "$CMP_JSON" "$ALLOWLIST" "$REPORT")"
    ORACLE_RC=$?
    set -e

    BEAR_ONLY="$(jq '.bear_only' "$REPORT")"
    CMAKE_ONLY="$(jq '.cmake_only' "$REPORT")"
    DIFF_TOTAL="$(jq '.differing_total' "$REPORT")"
    info "extras (logged, not a gate): ${BEAR_ONLY} Bear-only, ${CMAKE_ONLY} CMake-only TUs"
    info "matched TUs differing: ${DIFF_TOTAL}; after allow-list, survivors: ${SURVIVORS}"
    info "full divergence report: $REPORT"

    if [ "$ORACLE_RC" -eq 0 ]; then
        finish PASS "matched TUs equivalent under the allow-list (oracle: ${DIFF_TOTAL} benign diffs suppressed; ${BEAR_ONLY}+${CMAKE_ONLY} extras logged)"
    elif [ "$ORACLE_RC" -eq 1 ]; then
        err "oracle mismatch: ${SURVIVORS} matched TUs differ beyond the allow-list"
        err "survivors detailed in $REPORT (.survivors); add a documented allow-list rule only if the difference is genuinely benign"
        finish FAIL "oracle mismatch: ${SURVIVORS} matched TUs diverge after the allow-list - see $REPORT"
    else
        # oracle_report could not produce a valid survivor count (jq write/parse
        # failure): an infra problem, not a real mismatch.
        finish ERROR "oracle report generation failed (jq/arithmetic); see $REPORT"
    fi
fi

# === STEP 6 (golden): GATE (dogfood-golden-regression) =======================
# Host cdb-compare gates the fresh CDB against the committed golden. On
# --rebless, write the golden instead (dogfood-golden-rebless).
#
# Per the resolved decision, no normalization flags are used: the same pinned
# image and fixed /src path make the raw multiset reproducible. If a benign
# compiler-path diff ever appears, add --substitute-compiler cc to BOTH the
# bless and the check below, consistently.

if [ "$REBLESS" -eq 1 ]; then
    info "reblessing golden: $GOLDEN"
    mkdir -p "$(dirname "$GOLDEN")"
    if ! "$CDB_COMPARE" normalize --sort "$FRESH" -o "$GOLDEN" >&2; then
        finish ERROR "cdb-compare normalize failed during rebless"
    fi
    info "golden rewritten from this run; review and commit it"
    finish PASS "reblessed golden at $GOLDEN"
fi

if [ ! -f "$GOLDEN" ]; then
    err "no golden at $GOLDEN; produce one with: $0 --rebless $TARGET"
    finish ERROR "missing golden (run --rebless first)"
fi

info "gating fresh CDB against golden"
DIFF_HUMAN="$RESULTS_DIR/golden-diff.txt"
DIFF_JSON="$RESULTS_DIR/golden-diff.json"

if "$CDB_COMPARE" compare "$GOLDEN" "$FRESH" >"$DIFF_HUMAN" 2>&1; then
    cat "$DIFF_HUMAN" >&2
    finish PASS "fresh CDB matches golden (no regression)"
else
    cat "$DIFF_HUMAN" >&2
    # Save a machine-readable diff alongside the human one for review.
    "$CDB_COMPARE" compare --format json "$GOLDEN" "$FRESH" >"$DIFF_JSON" 2>&1 || true
    err "golden mismatch; diffs saved to $DIFF_HUMAN and $DIFF_JSON"
    finish FAIL "fresh CDB differs from golden (regression) - see $DIFF_HUMAN"
fi
