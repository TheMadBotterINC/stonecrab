#!/usr/bin/env bash
# Usage: scripts/bench-compare.sh [--rounds N] [--threshold PERCENT] BASE_REF
#        scripts/bench-compare.sh --self-test | --help
#
# The 10% benchmark regression gate (SPEC.md 'Performance', CODING_STANDARDS 16.4, ADR-0009
# addendum S52). It compares the working tree (HEAD) with BASE_REF on the machine it runs on,
# never with a stored number, because a baseline from another machine does not transfer
# (docs/evidence/benchmarks/README.md).
#
#   1. BASE_REF is exported with `git archive` into target/bench-compare/base-src, and both
#      trees build their Criterion benches with `cargo bench --no-run --locked`. A base that
#      has no benchmarks (it predates slice S52) ends the run with a notice and status 0.
#   2. Each bench binary then runs N rounds (default 3), the base and the head alternately and
#      in the opposite order every round, so a drift of the runner (a noisy neighbour, a
#      frequency change) hits both sides. A round is a full Criterion run that saves the
#      baseline "r<round>" (BENCH_ARGS overrides Criterion's options; the default is a
#      shortened run of 1 s warm-up, 3 s measurement and 50 samples per benchmark).
#   3. Per benchmark and round the script takes the median estimate and its 95% confidence
#      interval from Criterion's estimates.json. A round "separates" when the head's lower
#      bound is above the base's upper bound times (1 + threshold): the head is slower by
#      more than 10% even at its most favourable end of the noise band. A benchmark
#      REGRESSES when every round separates. A benchmark whose median ratio is above the
#      threshold without that is reported as "slower, within noise" and does not fail.
#
# Limits: a regression between 10% and 10% plus the width of the two bands is not caught,
# and neither is one that appears and disappears between rounds. Benchmarks that exist on one
# side only are listed and never fail. BASE_REF must be a commit the checkout has (fetch the
# history first).
#
# Writes target/bench-compare/report.md and report.json (BENCH_WORKDIR overrides the
# directory). Exit status: 0 no regression (or nothing to compare), 1 at least one
# regression or a build or run failure, 2 usage error. Needs bash 3.2+, git, tar, jq and
# cargo; run it through `mise run bench-compare -- BASE_REF` for the local toolchain settings.
set -euo pipefail

usage() {
  sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

# The first line of a Criterion benchmark's estimates, one JSON object per file:
# {"id","side","round","lo","point","hi"} for the median, in nanoseconds.
collect() { # collect HOME SIDE
  local home=$1 side=$2 file dir round
  [ -d "$home" ] || return 0
  find "$home" -type f -name estimates.json | while IFS= read -r file; do
    dir=$(dirname "$file")
    case $(basename "$dir") in
      r[0-9]*) ;;
      *) continue ;;
    esac
    round=${dir##*/r}
    jq -c --arg side "$side" --argjson round "$round" \
      --slurpfile bench "$dir/benchmark.json" \
      '{id: $bench[0].full_id, side: $side, round: $round,
        lo: .median.confidence_interval.lower_bound,
        point: .median.point_estimate,
        hi: .median.confidence_interval.upper_bound}' "$file"
  done
}

# compare BASE_HOME HEAD_HOME THRESHOLD_FRACTION REPORT_JSON
# Prints a Markdown table; status 1 when a benchmark regresses.
compare() {
  local base_home=$1 head_home=$2 threshold=$3 report=$4
  { collect "$base_home" base; collect "$head_home" head; } | jq -s --argjson t "$threshold" '
    def middle: sort | .[(length / 2 | floor)];
    def verdict($rounds; $median):
      if ($rounds | length) < 2 then "unpaired"
      elif all($rounds[]; .separated) then "REGRESSION"
      elif all($rounds[]; .faster) then "improved"
      elif $median > 1 + $t then "slower, within noise"
      else "ok" end;
    group_by(.id) | map(
      . as $rows
      | ([$rows[] | select(.side == "base")] | sort_by(.round)) as $base
      | ([$rows[] | select(.side == "head")] | sort_by(.round)) as $head
      | ([($base | length), ($head | length)] | min) as $n
      | [range(0; $n) | {
          ratio: ($head[.].point / $base[.].point),
          separated: ($head[.].lo > $base[.].hi * (1 + $t)),
          faster: ($head[.].hi < $base[.].lo / (1 + $t))
        }] as $rounds
      | ($rounds | map(.ratio)) as $ratios
      | {
          id: $rows[0].id,
          base_ns: (if $n > 0 then ($base | map(.point) | middle) else null end),
          head_ns: (if $n > 0 then ($head | map(.point) | middle) else null end),
          ratios: $ratios,
          verdict: (if $n == 0 then "unpaired"
                    else verdict($rounds; ($ratios | middle)) end)
        }
    )' >"$report"

  local compared regressions
  compared=$(jq '[.[] | select(.verdict != "unpaired")] | length' "$report")
  regressions=$(jq '[.[] | select(.verdict == "REGRESSION")] | length' "$report")
  echo "| benchmark | base median (ns) | head median (ns) | head/base per round | verdict |"
  echo "| --- | ---: | ---: | --- | --- |"
  jq -r '.[] | "| \(.id) | \(.base_ns // "-" | if type == "number" then (. * 10 | round / 10) else . end) | \(.head_ns // "-" | if type == "number" then (. * 10 | round / 10) else . end) | \(.ratios | map(. * 1000 | round / 1000) | join(", ")) | \(.verdict) |"' "$report"
  echo
  echo "Gate: $regressions regression(s) among $compared compared benchmark(s); threshold ${threshold} beyond the noise band."
  [ "$regressions" -eq 0 ]
}

# fake HOME ID ROUND LO POINT HI: one Criterion result for the self-test.
fake() {
  local dir="$1/$2/r$3"
  mkdir -p "$dir"
  printf '{"full_id":"%s"}\n' "$2" >"$dir/benchmark.json"
  printf '{"median":{"confidence_interval":{"lower_bound":%s,"upper_bound":%s},"point_estimate":%s}}\n' \
    "$4" "$6" "$5" >"$dir/estimates.json"
}

self_test() {
  local tmp status=0
  tmp=$(mktemp -d)
  self_test_in "$tmp" || status=$?
  rm -rf "$tmp"
  return "$status"
}

self_test_in() {
  local tmp=$1 status failures=0 round
  for round in 1 2 3; do
    fake "$tmp/base" steady "$round" 99 100 101
    fake "$tmp/head" steady "$round" 99 100 101
    fake "$tmp/base" noisy "$round" 99 100 101
    fake "$tmp/head" noisy "$round" 90 125 160
    fake "$tmp/base" spike "$round" 99 100 101
    if [ "$round" -eq 1 ]; then
      fake "$tmp/head" spike "$round" 128 130 132
    else
      fake "$tmp/head" spike "$round" 99 100 101
    fi
    fake "$tmp/base" faster "$round" 99 100 101
    fake "$tmp/head" faster "$round" 69 70 71
    fake "$tmp/head" added "$round" 9 10 11
  done

  expect() { # expect ID VERDICT
    local got
    got=$(jq -r --arg id "$1" '.[] | select(.id == $id) | .verdict' "$tmp/report.json")
    if [ "$got" != "$2" ]; then
      echo "self-test: $1 should be '$2', got '$got'" >&2
      failures=$((failures + 1))
    fi
  }

  status=0
  compare "$tmp/base" "$tmp/head" 0.10 "$tmp/report.json" >"$tmp/out.md" || status=$?
  [ "$status" -eq 0 ] || { echo "self-test: a run without a regression should pass" >&2; failures=$((failures + 1)); }
  expect steady ok
  expect noisy "slower, within noise"
  expect spike ok
  expect faster improved
  expect added unpaired

  for round in 1 2 3; do
    fake "$tmp/base" slower "$round" 99 100 101
    fake "$tmp/head" slower "$round" 112 113 114
    fake "$tmp/base" edge "$round" 90 100 110
    fake "$tmp/head" edge "$round" 100 112 124
  done
  status=0
  compare "$tmp/base" "$tmp/head" 0.10 "$tmp/report.json" >"$tmp/out.md" || status=$?
  [ "$status" -eq 1 ] || { echo "self-test: a 13% regression with tight bands should fail" >&2; failures=$((failures + 1)); }
  expect slower REGRESSION
  expect edge "slower, within noise"
  grep -q 'Gate: 1 regression' "$tmp/out.md" || { echo "self-test: the summary line is wrong" >&2; failures=$((failures + 1)); }

  if [ "$failures" -ne 0 ]; then
    echo "self-test: $failures failure(s)" >&2
    return 1
  fi
  echo "self-test: ok"
}

rounds=3
percent=10
base_ref=
while [ "$#" -gt 0 ]; do
  case $1 in
    -h | --help) usage; exit 0 ;;
    --self-test) self_test; exit $? ;;
    --rounds) rounds=${2:-}; shift 2 || { usage >&2; exit 2; } ;;
    --threshold) percent=${2:-}; shift 2 || { usage >&2; exit 2; } ;;
    -*) echo "unknown option $1" >&2; usage >&2; exit 2 ;;
    *) [ -z "$base_ref" ] || { usage >&2; exit 2; }; base_ref=$1; shift ;;
  esac
done
case $rounds in
  '' | *[!0-9]* | 0 | 1) echo "--rounds must be an integer of at least 2" >&2; exit 2 ;;
esac
printf '%s\n' "$percent" | grep -Eq '^[0-9]+([.][0-9]+)?$' \
  && awk -v p="$percent" 'BEGIN { exit !(p > 0) }' \
  || { echo "--threshold must be a positive number of percent" >&2; exit 2; }
[ -n "$base_ref" ] || { usage >&2; exit 2; }
threshold=$(awk -v p="$percent" 'BEGIN { printf "%.4f", p / 100 }')

cd "$(dirname "$0")/.."
repo=$(pwd)
git rev-parse --verify --quiet "$base_ref^{commit}" >/dev/null \
  || { echo "BASE_REF '$base_ref' is not a commit of this checkout" >&2; exit 2; }
work=${BENCH_WORKDIR:-$repo/target/bench-compare}
bench_args=${BENCH_ARGS:---warm-up-time 1 --measurement-time 3 --sample-size 50 --noplot}

rm -rf "$work"
mkdir -p "$work/base-src" "$work/bin"
git archive "$base_ref" | tar -x -C "$work/base-src"

# build TREE SIDE: build the benches of TREE into a target directory of its own (a shared one
# would reuse the base's artifacts for the head: Cargo's unit hash ignores the checkout path and
# the extracted base files are older than the artifacts), copy each executable to bin/SIDE-NAME and list the names in bin/SIDE.list.
build() {
  local tree=$1 side=$2 line name path
  : >"$work/bin/$side.list"
  (cd "$tree" && CARGO_TARGET_DIR="$work/target-$side" cargo bench --locked -p stonecrab --all-features \
    --no-run --message-format=json 2>"$work/build-$side.log") \
    | jq -r 'select(.reason == "compiler-artifact" and .executable != null
                    and (.target.kind | index("bench"))) | "\(.target.name) \(.executable)"' \
    | while read -r name path; do
      cp "$path" "$work/bin/$side-$name"
      echo "$name" >>"$work/bin/$side.list"
    done
}
build "$work/base-src" base || { echo "the base build failed; see $work/build-base.log" >&2; exit 1; }
if [ ! -s "$work/bin/base.list" ]; then
  echo "$base_ref has no benchmarks (it predates slice S52); nothing to compare." | tee "$work/report.md"
  exit 0
fi
build "$repo" head || { echo "the head build failed; see $work/build-head.log" >&2; exit 1; }

run_side() { # run_side SIDE ROUND
  local side=$1 round=$2 name
  while read -r name; do
    # Criterion options are word-split on purpose.
    # shellcheck disable=SC2086
    CRITERION_HOME="$work/criterion-$side" "$work/bin/$side-$name" --bench $bench_args \
      --save-baseline "r$round" >>"$work/run-$side.log" 2>&1 \
      || { echo "$side $name failed in round $round; see $work/run-$side.log" >&2; return 1; }
  done <"$work/bin/$side.list"
}
round=1
while [ "$round" -le "$rounds" ]; do
  if [ $((round % 2)) -eq 1 ]; then first=base second=head; else first=head second=base; fi
  echo "round $round: $first, then $second" >&2
  run_side "$first" "$round"
  run_side "$second" "$round"
  round=$((round + 1))
done

status=0
compare "$work/criterion-base" "$work/criterion-head" "$threshold" "$work/report.json" \
  | tee "$work/report.md" || status=$?
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then cat "$work/report.md" >>"$GITHUB_STEP_SUMMARY"; fi
exit "$status"
