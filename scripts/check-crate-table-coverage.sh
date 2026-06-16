#!/bin/sh
# Verify that the "### Workspace crates" table in the root CLAUDE.md lists
# every workspace member. The root Cargo.toml is the source of truth for
# which crates exist; the table must document each of them.
#
# Run from the repo root:
#     ./scripts/check-crate-table-coverage.sh
#
# Exit codes:
#   0 - every workspace member appears in the CLAUDE.md crate table
#   1 - at least one workspace member is missing from the table
#   2 - invocation error (e.g. expected files not found)

set -eu

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

cargo_toml="${repo_root}/Cargo.toml"
claude_md="${repo_root}/CLAUDE.md"

if [ ! -f "${cargo_toml}" ]; then
    echo "error: workspace manifest not found at ${cargo_toml}" >&2
    exit 2
fi
if [ ! -f "${claude_md}" ]; then
    echo "error: root CLAUDE.md not found at ${claude_md}" >&2
    exit 2
fi

# Members that legitimately should not appear in the table. Keep this list
# empty unless a member is genuinely not a documentable crate; prefer
# documenting every member instead.
skip_members=""

# Extract the [workspace] members glob patterns (e.g. "crates/*") from the
# manifest, then expand them to the directories that actually contain a
# Cargo.toml. Globbing is the source of truth, matching how cargo resolves
# the workspace.
patterns="$(awk '
    /^\[workspace\]/ { in_ws = 1; next }
    /^\[/            { in_ws = 0 }
    in_ws && /members[[:space:]]*=/ { collecting = 1 }
    collecting {
        line = $0
        while (match(line, /"[^"]+"/)) {
            print substr(line, RSTART + 1, RLENGTH - 2)
            line = substr(line, RSTART + RLENGTH)
        }
        if (line ~ /\]/) { collecting = 0 }
    }
' "${cargo_toml}")"

if [ -z "${patterns}" ]; then
    echo "error: no [workspace] members found in ${cargo_toml}" >&2
    exit 2
fi

members=""
for pattern in ${patterns}; do
    # Unquoted on purpose: the glob in ${pattern} (e.g. "crates/*") must
    # undergo pathname expansion to enumerate the member directories.
    # shellcheck disable=SC2086
    for dir in ${repo_root}/${pattern}; do
        [ -f "${dir}/Cargo.toml" ] || continue
        rel="${dir#"${repo_root}/"}"
        members="${members} ${rel}"
    done
done

if [ -z "${members}" ]; then
    echo "error: workspace member globs matched no crates" >&2
    exit 2
fi

# Extract the backtick-quoted path from the first column of each table row in
# the "### Workspace crates" section. The section ends at the next heading.
documented="$(awk '
    /^### Workspace crates/ { in_section = 1; next }
    in_section && /^#/      { in_section = 0 }
    in_section && /^\|/ {
        if (match($0, /`[^`]+`/)) {
            print substr($0, RSTART + 1, RLENGTH - 2)
        }
    }
' "${claude_md}")"

missing=0
checked=0

for member in ${members}; do
    skip=0
    for s in ${skip_members}; do
        if [ "${member}" = "${s}" ]; then
            skip=1
            break
        fi
    done
    [ "${skip}" -eq 1 ] && continue

    checked=$((checked + 1))

    found=0
    for entry in ${documented}; do
        if [ "${member}" = "${entry}" ]; then
            found=1
            break
        fi
    done

    if [ "${found}" -eq 0 ]; then
        echo "MISSING: ${member} is a workspace member but not in the CLAUDE.md crate table"
        missing=$((missing + 1))
    fi
done

echo
echo "Checked ${checked} workspace member(s); ${missing} missing from the crate table."

if [ "${missing}" -gt 0 ]; then
    exit 1
fi
