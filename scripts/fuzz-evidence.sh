#!/usr/bin/env bash
# Usage: scripts/fuzz-evidence.sh [SECONDS]
#
# Recorded release fuzz run (ADR-0017, docs/evidence/alpha.md). Builds every fuzz target
# once, then runs ALL targets concurrently, one single-threaded libFuzzer process each, for
# SECONDS of wall-clock time (default 1800, the 30-minute minimum of the alpha plan). Each
# target gets its own scratch copy of its seed corpus under fuzz/target/evidence-corpus/ so
# libFuzzer's new inputs never touch the committed fuzz/corpus/ (ADR-0009); logs go to
# fuzz/target/evidence-logs/. At the end it prints one summary line per target (final run
# count, final coverage counter, wall-clock seconds, crash artifacts).
#
# Unlike scripts/fuzz-smoke.sh (targets one after another, with a cargo-deny check first),
# this script is for the one recorded run: it assumes 12 spare cores are not needed, only
# one per target. Needs cargo-fuzz on PATH; rustup installs the nightly of
# fuzz/rust-toolchain.toml on demand.
#
# Exit status: 0 every target ran to the end with no crash artifact, 1 a target failed or
# left an artifact, 2 usage error.
set -euo pipefail

case "${1:-}" in
  -h | --help)
    sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
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
printf '\n%-34s %-12s %-10s %-8s %-8s %s\n' target runs cov seconds exit artifacts
while IFS= read -r target; do
  log="$logs/$target.log"
  status_line="$(cat "$logs/$target.status" 2> /dev/null || echo 'exit=? seconds=?')"
  code="${status_line#exit=}"
  code="${code%% *}"
  secs="${status_line##*seconds=}"
  # Last libFuzzer status line, for example "#41231566 DONE cov: 612 ft: 2210 corp: ..."
  last="$(grep -E '^#[0-9]+' "$log" | tail -n 1 || true)"
  runs="$(printf '%s\n' "$last" | sed -n 's/^#\([0-9]*\).*/\1/p')"
  cov="$(printf '%s\n' "$last" | sed -n 's/.*cov: \([0-9]*\).*/\1/p')"
  artifacts="$(find "artifacts/$target" -type f 2> /dev/null | wc -l | tr -d ' ')"
  printf '%-34s %-12s %-10s %-8s %-8s %s\n' "$target" "${runs:-?}" "${cov:-?}" "$secs" "$code" "$artifacts"
  if [ "$code" != 0 ] || [ "$artifacts" != 0 ]; then
    failed=1
  fi
done <<< "$targets"
exit "$failed"
