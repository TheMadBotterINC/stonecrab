#!/usr/bin/env bash
# Usage: scripts/coverage-gate.sh [--no-run]
#
# Per-component line-coverage gate (ADR-0005, docs/ALPHA_PLAN.md D7). Stonecrab is one crate
# whose components are modules, so the gate groups the files of crates/stonecrab/src by
# path prefix. Runs ONE instrumented `cargo llvm-cov nextest` over the workspace with all
# features, writes target/coverage.lcov and a text summary (target/coverage-summary.txt,
# region coverage reported but not gated), then sums the lcov line totals per component
# (the THRESHOLDS table below) and compares each sum with its threshold.
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

# Excluded from every pass (D7): not handwritten library logic. Generated code is any
# generated/ directory and the generated.rs that declares it (ADR-0019, Consequences).
EXCLUDE='(^|/)(examples|benches|fuzz|generated)/|(^|/)generated\.rs$'

# Where the single crate's sources live; a component is a path prefix below it.
SRC=crates/stonecrab/src

# component | files it owns | minimum line coverage in percent, or "na".
#
#   core       the files directly in src/ other than the module files named below (the
#              runtime-independent contracts: lib.rs, value.rs, uri.rs and so on)
#   codec      src/codec.rs and src/codec/
#   transport  src/transport.rs and src/transport/
#   sim        src/sim.rs and src/sim/
#   modbus     src/modbus.rs and src/modbus/
#   sensors    src/sensors.rs and src/sensors/
#   boards     src/boards.rs and src/boards/ (a directory since S64, ADR-0029)
#   opcua      src/opcua.rs and src/opcua/ (ADR-0034; the generated layer of a later slice
#              sits in a generated/ directory and is excluded like any other)
#   testkit    src/testkit.rs and src/testkit/
#   other      any other file in the lcov that is not under src/ (none today)
#
# "na" means the component has no executable lines and is skipped with a message. The row
# is listed on purpose: the gate fails loudly the day a listed n/a component gains an
# executable line. A directory under src/ with no row also fails.
#
# The transport row is 95 (ADR-0005, ADR-0014): the real-socket connect-timeout arm (the
# deadline passing while a connect is pending) cannot be induced on loopback, because a
# listening socket completes the handshake in the kernel backlog and non-loopback
# addresses are forbidden in tests; it is the first named OS-dependent uncovered arm. The
# testkit row is 90: it is test support, not shipped runtime logic. The modbus session and
# connection modules run entirely on the in-memory transport and the fake clock, so they
# have no OS-dependent arm and the row stays 100 (ADR-0015 decision 9). The boards row is 100
# (ADR-0026 decision 9, ADR-0029): the adapters run on scripted buses and the Linux SPI
# backend on in-memory readers, so no arm depends on a device. The opcua row is 100 outside
# generated code (ADR-0034 decision 10).
THRESHOLDS="
core|100
codec|100
sim|100
modbus|100
sensors|100
boards|100
opcua|100
transport|95
testkit|90
other|100
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

# Every directory under src/ must be one of the components above, and every component
# module file must exist, so a new module cannot slip past the gate unmeasured.
for dir in "$SRC"/*/; do
  [ -d "$dir" ] || continue
  name=$(basename "$dir")
  if ! printf '%s\n' "$THRESHOLDS" | grep -q "^${name}|"; then
    fail "$SRC/$name/ has no row in the THRESHOLDS table (add a threshold or na)"
  fi
done

for name in codec transport sim modbus sensors boards opcua testkit; do
  if [ ! -f "$SRC/$name.rs" ]; then
    fail "$SRC/$name.rs does not exist (a component module has a THRESHOLDS row but no module file)"
  fi
done

if [ "$run" -eq 1 ]; then
  cargo llvm-cov clean --workspace
  cargo llvm-cov nextest --locked --workspace --all-features --no-report
fi

mkdir -p target
cargo llvm-cov report --lcov --ignore-filename-regex "$EXCLUDE" --output-path "$LCOV"
cargo llvm-cov report --ignore-filename-regex "$EXCLUDE" | tee "$SUMMARY"

# Maps an lcov source path to its component name. Files under src/ belong to the module of
# their first path segment (codec.rs and codec/ are both "codec"); a file directly in src/
# that is not a module file is core; anything else is "other".
COMPONENT_AWK='
function component(path,   rel, seg, i) {
  i = index(path, "/" SRC "/")
  if (i == 0) return "other"
  rel = substr(path, i + length(SRC) + 2)
  if (index(rel, "/") == 0) {
    seg = rel
    sub(/\.rs$/, "", seg)
    if (seg in MODULE) return seg
    return "core"
  }
  seg = substr(rel, 1, index(rel, "/") - 1)
  return seg
}'

# lcov line totals for one component: prints "<found> <hit>".
lcov_lines() {
  awk -v SRC="$SRC" -v want="$1" "$COMPONENT_AWK"'
    BEGIN { split("codec transport sim modbus sensors boards opcua testkit", m, " "); for (k in m) MODULE[m[k]] = 1 }
    /^SF:/ { inside = (component(substr($0, 4)) == want) }
    inside && /^LF:/ { found += substr($0, 4) }
    inside && /^LH:/ { hit += substr($0, 4) }
    END { printf "%d %d\n", found, hit }' "$LCOV"
}

# The uncovered lines of one component, "<file>:<line>", for a failure message.
lcov_missing() {
  awk -v SRC="$SRC" -v want="$1" "$COMPONENT_AWK"'
    BEGIN { split("codec transport sim modbus sensors boards opcua testkit", m, " "); for (k in m) MODULE[m[k]] = 1 }
    /^SF:/ { file = substr($0, 4); inside = (component(file) == want) }
    inside && /^DA:/ { split(substr($0, 4), da, ","); if (da[2] == 0) print file ":" da[1] }' "$LCOV"
}

printf '\ncoverage-gate: per-component line coverage (region coverage above is reported, not gated)\n'
while IFS='|' read -r name threshold; do
  [ -n "$name" ] || continue
  read -r found hit <<EOT
$(lcov_lines "$name")
EOT
  if [ "$threshold" = na ]; then
    if [ "$found" -eq 0 ]; then
      printf 'coverage-gate: %-10s n/a  (no executable lines, skipped)\n' "$name"
    else
      fail "$name is listed n/a but has $found executable lines ($hit hit); give it a threshold"
    fi
    continue
  fi
  if [ "$found" -eq 0 ]; then
    # A component with no executable lines at all is n/a, except the real ones, which must
    # have lines when the tests ran. Only "other" may legitimately be empty.
    if [ "$name" = other ]; then
      printf 'coverage-gate: %-10s n/a  (no executable lines, skipped)\n' "$name"
    else
      fail "$name has no executable lines in $LCOV but requires $threshold% (tests not run?)"
    fi
    continue
  fi
  pct=$(awk -v h="$hit" -v f="$found" 'BEGIN { printf "%.2f", 100 * h / f }')
  # Integer comparison on the line counts, so 99.999 is never rounded up to a pass.
  if [ $((hit * 100)) -ge $((found * threshold)) ]; then
    printf 'coverage-gate: %-10s lines %s/%s = %s%%  required %s%%  PASS\n' \
      "$name" "$hit" "$found" "$pct" "$threshold"
  else
    lcov_missing "$name" >&2
    fail "$name lines $hit/$found = $pct% is below the required $threshold%"
    printf 'coverage-gate: %-10s lines %s/%s = %s%%  required %s%%  FAIL\n' \
      "$name" "$hit" "$found" "$pct" "$threshold" >&2
  fi
done <<EOT
$THRESHOLDS
EOT

if [ "$failures" -gt 0 ]; then
  printf 'coverage-gate: FAIL, %s problem(s)\n' "$failures" >&2
  exit 1
fi
printf 'coverage-gate: ok, lcov at %s, summary at %s\n' "$LCOV" "$SUMMARY"
