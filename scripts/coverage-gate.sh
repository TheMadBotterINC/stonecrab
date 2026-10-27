#!/usr/bin/env bash
# Usage: scripts/coverage-gate.sh [--no-run]
#
# Per-crate line-coverage gate (ADR-0005, docs/ALPHA_PLAN.md D7). Runs ONE instrumented
# `cargo llvm-cov nextest` over the workspace, writes target/coverage.lcov and a text
# summary (target/coverage-summary.txt, region coverage reported but not gated), then
# runs one `cargo llvm-cov report --fail-under-lines N` pass per crate in the THRESHOLDS
# table below, each restricted to that crate by --ignore-filename-regex.
#
#   --no-run   reuse the profile data of the previous run (skip the instrumented run)
#
# Linux is the gate platform (CI "quality" job). The script also runs on macOS for local
# use. NEXTEST_PROFILE=ci selects the CI nextest profile. Needs cargo-llvm-cov 0.9.1,
# cargo-nextest and the llvm-tools-preview component (all pinned in mise.toml).
#
# Exit status: 0 every gated crate meets its threshold, non-zero otherwise.
set -euo pipefail

cd "$(dirname "$0")/.."

# Excluded from every pass (D7): not handwritten library logic.
EXCLUDE='(^|/)(examples|benches|fuzz|generated)/'

# crate directory | minimum line coverage in percent, or "na".
#
# "na" means the crate has no executable lines (the facade only holds `pub use`) and is
# skipped with a message. The row is listed on purpose: `cargo llvm-cov report
# --fail-under-lines` exits 1 on an empty file set, so n/a crates are never passed to
# it, and the gate fails loudly the day a listed n/a crate gains an executable line.
# A crate directory with no row also fails. Later slices add rows here (driver-sim,
# sensors 100; transport 95; modbus 95 with the pure modules at 100); codec landed in S15.
THRESHOLDS="
crates/stonecrab-core|100
crates/stonecrab-codec|100
crates/stonecrab|na
tools/stonecrab-testkit|90
"

LCOV=target/coverage.lcov
SUMMARY=target/coverage-summary.txt

run=1
case "${1:-}" in
  --no-run) run=0 ;;
  -h | --help)
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  '') ;;
  *)
    echo "coverage-gate: unknown argument '$1'" >&2
    exit 2
    ;;
esac

failures=0
fail() {
  printf 'coverage-gate: FAIL %s\n' "$*" >&2
  failures=$((failures + 1))
}

# Every crate directory on disk needs a row.
for manifest in crates/*/Cargo.toml tools/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  dir=${manifest%/Cargo.toml}
  if ! printf '%s\n' "$THRESHOLDS" | grep -q "^${dir}|"; then
    fail "$dir has no row in the THRESHOLDS table (add a threshold or na)"
  fi
done

if [ "$run" -eq 1 ]; then
  cargo llvm-cov clean --workspace
  cargo llvm-cov nextest --locked --workspace --all-features --no-report
fi

mkdir -p target
cargo llvm-cov report --lcov --ignore-filename-regex "$EXCLUDE" --output-path "$LCOV"
cargo llvm-cov report --ignore-filename-regex "$EXCLUDE" | tee "$SUMMARY"

# lcov line totals for one crate directory: prints "<found> <hit>".
lcov_lines() {
  awk -v dir="/$1/" '
    /^SF:/ { inside = index($0, dir) > 0 }
    inside && /^LF:/ { found += substr($0, 4) }
    inside && /^LH:/ { hit += substr($0, 4) }
    END { printf "%d %d\n", found, hit }' "$LCOV"
}

printf '\ncoverage-gate: per-crate line coverage (region coverage above is reported, not gated)\n'
while IFS='|' read -r dir threshold; do
  [ -n "$dir" ] || continue
  name=${dir##*/}
  # The crate under test is every file except the other crates' directories.
  regex=$EXCLUDE
  while IFS='|' read -r other _; do
    if [ -n "$other" ] && [ "$other" != "$dir" ]; then
      regex="$regex|(^|/)$other/"
    fi
  done <<EOT
$THRESHOLDS
EOT
  read -r found hit <<EOT
$(lcov_lines "$dir")
EOT
  if [ "$threshold" = na ]; then
    if [ "$found" -eq 0 ]; then
      printf 'coverage-gate: %-18s n/a  (no executable lines, skipped)\n' "$name"
    else
      fail "$name is listed n/a but has $found executable lines ($hit hit); give it a threshold"
    fi
    continue
  fi
  if [ "$found" -eq 0 ]; then
    fail "$name has no executable lines in $LCOV but requires $threshold% (tests not run?)"
    continue
  fi
  pct=$(awk -v h="$hit" -v f="$found" 'BEGIN { printf "%.2f", 100 * h / f }')
  if cargo llvm-cov report --ignore-filename-regex "$regex" --fail-under-lines "$threshold" \
    --show-missing-lines >"target/coverage-$name.txt" 2>&1; then
    printf 'coverage-gate: %-18s lines %s/%s = %s%%  required %s%%  PASS\n' \
      "$name" "$hit" "$found" "$pct" "$threshold"
  else
    cat "target/coverage-$name.txt" >&2
    fail "$name lines $hit/$found = $pct% is below the required $threshold%"
  fi
done <<EOT
$THRESHOLDS
EOT

if [ "$failures" -gt 0 ]; then
  printf 'coverage-gate: FAIL, %s problem(s)\n' "$failures" >&2
  exit 1
fi
printf 'coverage-gate: ok, lcov at %s, summary at %s\n' "$LCOV" "$SUMMARY"
