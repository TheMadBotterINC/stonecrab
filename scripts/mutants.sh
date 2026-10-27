#!/usr/bin/env bash
# Usage: scripts/mutants.sh [JOBS]
#
# The recorded mutation run (ADR-0017, docs/evidence/mutants/README.md). Runs cargo-mutants
# 27.1.0 with mutants.toml (scope, nextest, exclusions) in a scratch copy of the tree
# (cargo-mutants' default; never --in-place) with JOBS parallel builds (default 4, or the
# MUTANTS_JOBS variable), writing the full output directory to target/mutants-run/ and then
# copying only outcomes.json to docs/evidence/mutants/outcomes.json. The rest of
# mutants.out holds per-mutant logs with machine paths and is not stored.
#
# Debug info is switched off for the scratch builds (it only costs disk and time); no
# profile setting that changes behavior is touched.
#
# Exit status is cargo-mutants': 0 every mutant caught or unviable, 1 usage error,
# 2 at least one mutant survived, 3 at least one test timed out, 4 the unmutated tree
# fails its tests; 1 also when the outcomes file could not be stored. The outcomes file is
# copied whenever the run produced one.
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac
if [ "$#" -gt 1 ]; then
  echo "usage: scripts/mutants.sh [JOBS]" >&2
  exit 2
fi
jobs="${1:-${MUTANTS_JOBS:-4}}"
case "$jobs" in
  '' | *[!0-9]* | 0)
    echo "JOBS must be a positive integer, got '$jobs'" >&2
    exit 2
    ;;
esac

cd "$(dirname "$0")/.."
out=target/mutants-run
rm -rf "$out"
mkdir -p "$out"

export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0

status=0
cargo mutants --config mutants.toml --jobs "$jobs" --output "$out" || status=$?

if [ -f "$out/mutants.out/outcomes.json" ]; then
  mkdir -p docs/evidence/mutants
  # The recorded argv of every phase names the cargo binary by its absolute path in the
  # local toolchain; keep only "cargo" so that no path of this machine is stored.
  sed -E 's#"/[^"]*/bin/cargo"#"cargo"#g' "$out/mutants.out/outcomes.json" \
    > docs/evidence/mutants/outcomes.json
  if grep -Eq '"/(Users|home|private|var|tmp)/' docs/evidence/mutants/outcomes.json; then
    echo "mutants: outcomes.json still holds an absolute path; not storing it" >&2
    rm -f docs/evidence/mutants/outcomes.json
    status=1
  else
    echo "mutants: outcomes copied to docs/evidence/mutants/outcomes.json"
  fi
fi
exit "$status"
