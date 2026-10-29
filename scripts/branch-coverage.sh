#!/usr/bin/env bash
# Usage: scripts/branch-coverage.sh [--no-run]
#
# Report-only branch coverage per component (ADR-0005 addendum S62, CODING_STANDARDS 13.10:
# "branch >= 80 % where available"). It never fails on a number: the line gate of
# scripts/coverage-gate.sh is unchanged and stays the only coverage gate.
#
# cargo-llvm-cov's --branch mode is unstable and needs a nightly compiler, so the run uses
# the nightly pinned for the fuzz workspace (fuzz/rust-toolchain.toml), installs its
# llvm-tools-preview component when it is missing, and builds into its own target
# directory (target/branch-coverage-target; delete it to get the disk back) so that the
# stable build of the line gate is neither disturbed nor rebuilt. One instrumented nextest
# run over the workspace with all features writes target/branch-coverage.lcov; the branch
# totals (BRF found, BRH hit) are then summed per component, with the same grouping of
# crates/stonecrab/src as the line gate, and written to target/branch-coverage.txt.
#
#   --no-run   reuse the profile data of the previous run (skip the instrumented run)
#
# A branch here is what the compiler's branch instrumentation counts: the true and the
# false outcome of each boolean condition (`if`, `while`, a match guard, the operands of
# `&&` and `||`), as llvm-cov reports them. A `match` arm that is not a guard is not a
# branch, so the count is far smaller than the number of lines, and a condition that a
# macro or a derive expands is counted like a written one. Compare the numbers between
# runs, not with the line thresholds.
#
# Exit status: 0 the report was written, non-zero when the run or the report failed.
# Needs rustup, cargo-llvm-cov 0.9.1 and cargo-nextest (pinned in mise.toml); run it through
# `mise run branch-coverage`.
set -euo pipefail

cd "$(dirname "$0")/.."

EXCLUDE='(^|/)(examples|benches|fuzz|generated)/'
SRC=crates/stonecrab/src
LCOV=target/branch-coverage.lcov
REPORT=target/branch-coverage.txt

run=1
case "${1:-}" in
  --no-run) run=0 ;;
  -h | --help)
    sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  '') ;;
  *)
    echo "branch-coverage: unknown argument '$1'" >&2
    exit 2
    ;;
esac

# The nightly of the fuzz workspace; mise exports RUSTUP_TOOLCHAIN for the stable compiler,
# which this run replaces.
nightly="$(sed -n 's/^channel = "\(.*\)"/\1/p' fuzz/rust-toolchain.toml)"
[ -n "$nightly" ] || {
  echo "branch-coverage: no channel in fuzz/rust-toolchain.toml" >&2
  exit 1
}
export RUSTUP_TOOLCHAIN="$nightly"
export CARGO_LLVM_COV_TARGET_DIR=target/branch-coverage-target

if [ "$run" -eq 1 ]; then
  rustup component add llvm-tools-preview --toolchain "$nightly"
  cargo llvm-cov clean --workspace
  cargo llvm-cov nextest --locked --workspace --all-features --branch --no-report
fi

mkdir -p target
cargo llvm-cov report --branch --lcov --ignore-filename-regex "$EXCLUDE" --output-path "$LCOV"

# Branch totals per component: the first path segment below src/ names a module (codec.rs
# and codec/ are both "codec"); a file directly in src/ that is not a module file is core.
{
  printf 'branch-coverage: report only, nightly %s, all features\n' "$nightly"
  printf '%-10s %8s %8s %8s\n' component branches hit percent
  awk -v SRC="$SRC" '
    BEGIN { split("codec transport sim modbus sensors testkit", m, " "); for (k in m) MODULE[m[k]] = 1 }
    function component(path,   rel, seg, i) {
      i = index(path, "/" SRC "/")
      if (i == 0) return "other"
      rel = substr(path, i + length(SRC) + 2)
      if (index(rel, "/") == 0) {
        seg = rel
        sub(/\.rs$/, "", seg)
        return (seg in MODULE) ? seg : "core"
      }
      return substr(rel, 1, index(rel, "/") - 1)
    }
    /^SF:/ { comp = component(substr($0, 4)) }
    /^BRF:/ { found[comp] += substr($0, 5); total_found += substr($0, 5) }
    /^BRH:/ { hit[comp] += substr($0, 5); total_hit += substr($0, 5) }
    function row(name, f, h) {
      if (f == 0) printf "%-10s %8d %8d %8s\n", name, f, h, "n/a"
      else printf "%-10s %8d %8d %7.2f%%\n", name, f, h, 100 * h / f
    }
    END {
      n = split("core codec transport sim modbus sensors testkit other", order, " ")
      for (i = 1; i <= n; i++) row(order[i], found[order[i]], hit[order[i]])
      row("all", total_found, total_hit)
    }' "$LCOV"
} | tee "$REPORT"
printf 'branch-coverage: lcov at %s, report at %s\n' "$LCOV" "$REPORT"
