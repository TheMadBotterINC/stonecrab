#!/usr/bin/env bash
# Usage: scripts/fuzz-evidence.sh [SECONDS]
#
# Recorded release fuzz run (ADR-0017 and its addendum S62, docs/evidence/alpha.md). Builds
# every fuzz target once, then runs ALL targets concurrently, one single-threaded libFuzzer
# process each, for SECONDS of wall-clock time (default 1800, the 30-minute minimum of the
# alpha plan; the release run is `scripts/fuzz-evidence.sh 28800`, 8 hours, or `mise run
# fuzz-evidence -- 28800`). Each target gets its own scratch copy of its seed corpus under
# fuzz/target/evidence-corpus/ so libFuzzer's new inputs never touch the committed
# fuzz/corpus/ (ADR-0009); logs go to fuzz/target/evidence-logs/.
#
# At the end it prints the report and writes it to fuzz/target/evidence-logs/report.md: a
# table of the run (date, tree, machine, nightly, cargo-fuzz, SECONDS) and one row per
# target (seed inputs, final run count, final coverage counter, features, corpus size,
# executions per second, wall-clock seconds, exit status, crash artifacts) and a verdict.
# Copy report.md to docs/evidence/fuzz/ to record a run.
#
# The run PASSES when every target exits 0 after at least SECONDS of wall-clock time with
# at least one execution and left no crash artifact under fuzz/artifacts/. A crash, a
# timeout or an out-of-memory input leaves an artifact and fails the run; so does a target
# that stops early.
#
# Unlike scripts/fuzz-smoke.sh (targets one after another, with a cargo-deny check first),
# this script is for the one recorded run: it assumes 12 spare cores are not needed, only
# one per target. Needs cargo-fuzz on PATH; rustup installs the nightly of
# fuzz/rust-toolchain.toml on demand.
#
# Exit status: 0 the run passed, 1 it did not, 2 usage error.
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
    exit 0
    ;;
esac
if [ "$#" -gt 1 ]; then
  echo "usage: scripts/fuzz-evidence.sh [SECONDS]" >&2
  exit 2
fi
seconds="${1:-1800}"
case "$seconds" in
  '' | *[!0-9]* | 0)
    echo "SECONDS must be a positive integer, got '$seconds'" >&2
    exit 2
    ;;
esac

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/fuzz"
# mise exports RUSTUP_TOOLCHAIN for the pinned stable compiler, which would override
# fuzz/rust-toolchain.toml (same reason as in fuzz-smoke.sh).
unset RUSTUP_TOOLCHAIN

host="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$host" ]; then
  echo "could not determine the host triple from rustc -vV" >&2
  exit 1
fi
targets="$(cargo fuzz list)"
if [ -z "$targets" ]; then
  echo "no fuzz targets; nothing to run" >&2
  exit 1
fi

logs=target/evidence-logs
corpus=target/evidence-corpus
rm -rf "$logs" "$corpus"
mkdir -p "$logs" "$corpus"

echo "building every target for $host"
cargo fuzz build --target "$host"

pids=""
started="$(date +%s)"
while IFS= read -r target; do
  mkdir -p "$corpus/$target"
  cp -R "corpus/$target/." "$corpus/$target/"
  seeds="$(find "$corpus/$target" -type f | wc -l | tr -d ' ')"
  echo "$seeds" > "$logs/$target.seeds"
  echo "starting $target ($seeds seed inputs) for ${seconds}s"
  (
    t0="$(date +%s)"
    status=0
    cargo fuzz run --target "$host" "$target" "$corpus/$target" -- \
      "-max_total_time=$seconds" > "$logs/$target.log" 2>&1 || status=$?
    echo "exit=$status seconds=$(($(date +%s) - t0))" > "$logs/$target.status"
  ) &
  pids="$pids $!"
done <<< "$targets"

for pid in $pids; do
  wait "$pid" || true
done
echo "all targets finished after $(($(date +%s) - started))s"

failed=0
rows=""
while IFS= read -r target; do
  log="$logs/$target.log"
  status_line="$(cat "$logs/$target.status" 2> /dev/null || echo 'exit=? seconds=?')"
  code="${status_line#exit=}"
  code="${code%% *}"
  secs="${status_line##*seconds=}"
  # Last libFuzzer status line, for example
  # "#41231566 DONE cov: 612 ft: 2210 corp: 71/4Kb lim: 4096 exec/s: 26143 rss: 580Mb"
  last="$(grep -E '^#[0-9]+' "$log" | tail -n 1 || true)"
  runs="$(printf '%s\n' "$last" | sed -n 's/^#\([0-9]*\).*/\1/p')"
  cov="$(printf '%s\n' "$last" | sed -n 's/.* cov: \([0-9]*\).*/\1/p')"
  ft="$(printf '%s\n' "$last" | sed -n 's/.* ft: \([0-9]*\).*/\1/p')"
  corp="$(printf '%s\n' "$last" | sed -n 's/.* corp: \([0-9]*\/[0-9A-Za-z]*\).*/\1/p')"
  rate="$(printf '%s\n' "$last" | sed -n 's/.* exec\/s: \([0-9]*\).*/\1/p')"
  seeds="$(cat "$logs/$target.seeds" 2> /dev/null || echo '?')"
  artifacts="$(find "artifacts/$target" -type f 2> /dev/null | wc -l | tr -d ' ')"
  rows="$rows$(printf '| `%s` | %s | %s | %s | %s | %s | %s | %s | %s | %s |' \
    "$target" "$seeds" "${runs:-?}" "${cov:-?}" "${ft:-?}" "${corp:-?}" "${rate:-?}" "$secs" "$code" "$artifacts")
"
  case "$code$secs${runs:-}" in *[!0-9]*) failed=1 ;; esac
  if [ "$code" != 0 ] || [ "$artifacts" != 0 ] || [ "${secs:-0}" -lt "$seconds" ] 2> /dev/null || [ "${runs:-0}" = 0 ]; then
    failed=1
  fi
done <<< "$targets"

commit="$(git -C "$root" rev-parse --short HEAD 2> /dev/null || echo unknown)"
[ -z "$(git -C "$root" status --porcelain 2> /dev/null)" ] || commit="$commit plus uncommitted changes"
cpu="$(sysctl -n machdep.cpu.brand_string 2> /dev/null || sed -n 's/^model name[^:]*: //p' /proc/cpuinfo 2> /dev/null | head -n 1)"
{
  echo "# Fuzz evidence run"
  echo
  echo "| Item | Value |"
  echo "| --- | --- |"
  echo "| Date (UTC) | $(date -u +%Y-%m-%dT%H:%M:%SZ) |"
  echo "| Tree | $commit |"
  echo "| Machine | ${cpu:-unknown}, $(uname -sr) $(uname -m) |"
  echo "| Nightly | $(rustc -V) |"
  echo "| cargo-fuzz | $(cargo fuzz --version 2> /dev/null | head -n 1 || echo unknown) |"
  echo "| Command | \`scripts/fuzz-evidence.sh $seconds\` |"
  echo "| Duration | $seconds s per target, all targets concurrently, $(($(date +%s) - started)) s wall-clock after the build |"
  echo
  echo "| Target | Seed inputs | Runs | Cov | Ft | Corpus | Exec/s | Seconds | Exit | Artifacts |"
  echo "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"
  printf '%s' "$rows"
  echo
  if [ "$failed" = 0 ]; then
    echo "**Verdict: PASS.** Every target exited 0 after at least $seconds s, executed inputs and left no crash artifact."
  else
    echo "**Verdict: FAIL.** A target exited non-zero, stopped early, executed nothing or left a crash artifact."
  fi
} | tee "$logs/report.md"
exit "$failed"
