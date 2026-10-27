#!/usr/bin/env bash
# Usage: scripts/fuzz-smoke.sh [SECONDS]
#
# Fuzz smoke run (ADR-0009). First checks the fuzz workspace with cargo-deny, then lists
# its fuzz targets and runs every one for SECONDS (default 60; the weekly schedule passes
# 1800) with the nightly pinned in fuzz/rust-toolchain.toml. With zero targets it says so
# and exits 0. All targets run on every invocation; the plan's "only targets whose crate
# changed" filter is deferred until targets exist (ADR-0009).
# Needs cargo-fuzz and cargo-deny on PATH; rustup installs the pinned nightly on demand.
# Proptest runs of the main workspace use PROPTEST_CASES=256, set in CI (test, quality).
#
# Exit status: 0 clean, 1 deny or fuzz failure, 2 usage error.
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac
if [ "$#" -gt 1 ]; then
  echo "usage: scripts/fuzz-smoke.sh [SECONDS]" >&2
  exit 2
fi
seconds="${1:-60}"
case "$seconds" in
  '' | *[!0-9]* | 0)
    echo "SECONDS must be a positive integer, got '$seconds'" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

# The fuzz policy is a copy of the root policy (cargo-deny has no include); its only
# permitted difference is the extra "NCSA" license for libfuzzer-sys.
if ! diff <(grep -v '^#' fuzz/deny.toml | sed 's/, "NCSA"\]/]/') <(grep -v '^#' deny.toml) > /dev/null; then
  echo "fuzz/deny.toml has drifted from deny.toml (only the NCSA license may differ)" >&2
  exit 1
fi

# Run inside fuzz/ so rustup picks the nightly from fuzz/rust-toolchain.toml for both
# cargo-deny's metadata call and cargo-fuzz (CI installs only that toolchain here).
cd fuzz
cargo deny --locked check

targets="$(cargo fuzz list)"
if [ -z "$targets" ]; then
  echo "no fuzz targets yet; nothing to run"
  exit 0
fi
while IFS= read -r target; do
  echo "fuzzing $target for ${seconds}s"
  cargo fuzz run "$target" -- "-max_total_time=$seconds"
done <<< "$targets"
