#!/usr/bin/env bash
# Usage: scripts/soak.sh [OPTIONS] sim|pymodbus [SECONDS]
#        scripts/soak.sh --self-test | --help
#
# The soak run of the release evidence (ADR-0017 addendum S62, SPEC.md 'Performance': RSS
# and reconnect behaviour during soak tests). It builds the `soak` example in release mode,
# runs it for SECONDS (default 86400, 24 hours) against one target, samples the resident
# memory of the example's process every INTERVAL seconds with `ps` (macOS and Linux), and
# ends with a verdict on whether the memory is stable.
#
#   sim        the in-process simulator. The example reads flat out (period 0) and pushes a
#              failure step every 5 s (a lost link, a lost reply, an undecodable reply, in
#              turn), so the link is reopened about every 15 s.
#   pymodbus   the pymodbus interop peer (interop/pymodbus/) in a container of its own,
#              stonecrab-soak on 127.0.0.1:5021, so a running `mise run interop` does not
#              clash. The image stonecrab-interop:local must exist (interop/pymodbus/interop.sh
#              build). The script restarts the container every RESTART seconds, and the
#              driver reconnects. Docker's log of the container is capped at 20 MB.
#
# Options (defaults in brackets):
#   --interval S         seconds between memory samples [10]
#   --restart-every S    pymodbus: seconds between container restarts, 0 for none [120]
#   --period-ms N        pause after each read [sim 0, pymodbus 10]
#   --max-growth-kib N   the memory bound of the verdict, in KiB [4096]
#   --out DIR            where the three files go [target/soak/TARGET]
#
# Files written to DIR: TARGET.csv (elapsed_s,rss_kib,reads,errors,reconnects), TARGET.log
# (the example's own output) and TARGET.md (the report). Copy the CSV and the report to
# docs/evidence/soak/ to record a run.
#
# The verdict (analyze below). The first 20% of the run is a warm-up and is not judged: the
# allocator, the connection buffers and the runtime reach their working size there. Of the
# samples after it, the growth is the median of the last quarter minus the median of the
# first quarter, which a single spike does not move. The memory is STABLE when the growth is
# at most the bound (--max-growth-kib, default 4 MiB). The two medians lie about 0.6 of the
# run apart, so over 24 hours a steady leak is caught from about 0.28 MiB an hour (the 10
# minute runs catch only about 40 MiB an hour); the bound has to sit above the page-granular
# noise of the allocator (docs/evidence/soak/README.md has the measured noise). Fewer than 12
# samples, or fewer than 8 after the warm-up, is INCONCLUSIVE. The run passes when the
# memory is stable, the example exited 0, more reads than errors happened in the last
# quarter of the run (the link works at the end), and pymodbus reconnected at least as
# often as the container was restarted.
#
# Exit status: 0 pass, 1 fail or inconclusive or a setup failure, 2 usage error. Needs bash
# 3.2+, ps, awk, cargo, and for pymodbus Docker. Run it through `mise run soak -- ARGS` for
# the local toolchain settings.
set -euo pipefail

usage() {
  sed -n '2,/^set -/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
}

# analyze CSV BOUND_KIB: print the verdict facts of one run as key=value lines.
#   verdict=stable|leak|inconclusive   samples= warmup_s= early_kib= late_kib= growth_kib=
#   slope_kib_per_h= min_kib= max_kib= reads= errors= reconnects= late_reads= late_errors=
analyze() {
  awk -F, -v bound="$2" '
    function median(a, lo, hi,   i, j, k, v, n) { # median of a[lo..hi], by insertion sort
      n = 0
      for (i = lo; i <= hi; i++) { v = a[i]; j = n - 1; while (j >= 0 && s[j] > v) { s[j + 1] = s[j]; j-- } s[j + 1] = v; n++ }
      k = int(n / 2)
      return (n % 2) ? s[k] : (s[k - 1] + s[k]) / 2
    }
    NR == 1 && $1 == "elapsed_s" { next }
    NF >= 5 { t[n] = $1; r[n] = $2; rd[n] = $3; er[n] = $4; rc[n] = $5; n++ }
    END {
      if (n == 0) { print "verdict=inconclusive"; print "samples=0"; exit }
      warm = t[n - 1] * 0.2
      m = 0
      for (i = 0; i < n; i++) if (t[i] >= warm) { pt[m] = t[i]; pr[m] = r[i]; m++ }
      # Extremes of the judged samples (the first sample of a process can read a few KiB).
      min = pr[0]; max = pr[0]
      for (i = 1; i < m; i++) { if (pr[i] < min) min = pr[i]; if (pr[i] > max) max = pr[i] }
      # The last quarter of the run, by time, for the progress check.
      cut = t[n - 1] * 0.75
      for (i = 0; i < n && t[i] < cut; i++) ;
      if (i >= n) i = n - 1
      late_reads = rd[n - 1] - rd[i]; late_errors = er[n - 1] - er[i]
      printf "samples=%d\nwarmup_s=%d\nmin_kib=%d\nmax_kib=%d\n", n, warm, min, max
      printf "reads=%d\nerrors=%d\nreconnects=%d\nlate_reads=%d\nlate_errors=%d\n", rd[n - 1], er[n - 1], rc[n - 1], late_reads, late_errors
      if (n < 12 || m < 8) { print "verdict=inconclusive"; exit }
      q = int(m / 4)
      early = median(pr, 0, q - 1); late = median(pr, m - q, m - 1)
      growth = late - early
      # Least-squares slope of the samples after the warm-up, in KiB per hour.
      for (i = 0; i < m; i++) { sx += pt[i]; sy += pr[i]; sxx += pt[i] * pt[i]; sxy += pt[i] * pr[i] }
      den = m * sxx - sx * sx
      slope = (den == 0) ? 0 : (m * sxy - sx * sy) / den * 3600
      printf "early_kib=%.1f\nlate_kib=%.1f\ngrowth_kib=%.1f\nslope_kib_per_h=%.1f\n", early, late, growth, slope
      print (growth <= bound) ? "verdict=stable" : "verdict=leak"
    }' "$1"
}

# fact KEY: the value of a key=value line in $FACTS.
fact() {
  printf '%s\n' "$FACTS" | sed -n "s/^$1=//p"
}

# A CSV of N samples for the self-test: row i has RSS BASE + i*SLOPE (+ SPIKE every 7th row),
# reads growing by 100 a sample and no errors.
fake_csv() { # fake_csv N BASE SLOPE SPIKE
  awk -v n="$1" -v base="$2" -v slope="$3" -v spike="$4" 'BEGIN {
    print "elapsed_s,rss_kib,reads,errors,reconnects"
    for (i = 0; i < n; i++) printf "%d,%d,%d,0,%d\n", i * 10, base + i * slope + ((i % 7 == 0) ? spike : 0), i * 100, int(i / 30)
  }'
}

self_test() {
  local tmp failures=0 name csv want got
  tmp=$(mktemp -d)
  check() { # check NAME CSV WANT
    FACTS=$(analyze "$2" 4096)
    got=$(fact verdict)
    if [ "$got" = "$3" ]; then
      printf 'self-test ok   %s (%s)\n' "$1" "$got"
    else
      printf 'self-test FAIL %s: expected %s, got %s\n' "$1" "$3" "$got"
      failures=$((failures + 1))
    fi
  }
  fake_csv 120 50000 0 0 >"$tmp/flat.csv"
  check flat "$tmp/flat.csv" stable
  # A spike of 3 MiB every seventh sample is noise: the medians ignore it.
  fake_csv 120 50000 0 3000 >"$tmp/spiky.csv"
  check spiky "$tmp/spiky.csv" stable
  # 1 KiB a sample over 120 samples is 100 KiB after the warm-up: under the bound.
  fake_csv 120 50000 1 0 >"$tmp/creep.csv"
  check creep "$tmp/creep.csv" stable
  # 100 KiB a sample over 120 samples is about 7 MiB after the warm-up: a leak.
  fake_csv 120 50000 100 0 >"$tmp/leak.csv"
  check leak "$tmp/leak.csv" leak
  # Growth only in the warm-up (the first 24 samples), flat afterwards: stable.
  awk -F, 'NR == 1 { print; next } { if ($1 < 240) $2 = 20000 + $1 * 1000; else $2 = 20000 + 240 * 1000; print $1 "," $2 "," $3 "," $4 "," $5 }' "$tmp/flat.csv" >"$tmp/warm.csv"
  check warmup-growth "$tmp/warm.csv" stable
  # A 24-hour series (8640 samples, 10 s apart). The growth spans 0.6 of the run, so the bound
  # of 4096 KiB is reached at 4096 / 14.4 = 284 KiB an hour, 0.79 KiB a sample. Half of that
  # passes and twice that fails (slopes are in KiB per 10 s sample; fake_csv truncates).
  fake_csv 8640 50000 0.4 0 >"$tmp/day-half.csv"
  check day-half-rate "$tmp/day-half.csv" stable
  fake_csv 8640 50000 1.6 0 >"$tmp/day-double.csv"
  check day-double-rate "$tmp/day-double.csv" leak
  fake_csv 10 50000 0 0 >"$tmp/short.csv"
  check too-short "$tmp/short.csv" inconclusive
  printf 'elapsed_s,rss_kib,reads,errors,reconnects\n' >"$tmp/empty.csv"
  check empty "$tmp/empty.csv" inconclusive
  # The progress facts: reads keep growing by 100 a sample, so the last quarter has reads.
  FACTS=$(analyze "$tmp/flat.csv" 4096)
  want=$(((120 - 90) * 100))
  got=$(fact late_reads)
  if [ "$got" = "$want" ] || [ "$got" = "$((want - 100))" ] || [ "$got" = "$((want + 100))" ]; then
    printf 'self-test ok   late-reads (%s)\n' "$got"
  else
    printf 'self-test FAIL late-reads: expected about %s, got %s\n' "$want" "$got"
    failures=$((failures + 1))
  fi
  rm -rf "$tmp"
  if [ "$failures" -gt 0 ]; then
    printf 'self-test: %s failure(s)\n' "$failures" >&2
    return 1
  fi
  echo "self-test: ok"
}

die() {
  echo "soak: $*" >&2
  exit 1
}

# The value of one counter on the last sample line of the example's log.
counter() { # counter LOG KEY
  tail -n 200 "$1" | grep -a ' soak: sample ' | tail -n 1 | sed -n "s/.* $2=\([0-9]*\).*/\1/p" || true
}

# total KEY: the final total of a counter, from the 'soak finished' line of the example's log
# ($LOG); the last sampled value (the CSV facts) when the example did not get to print it.
total() {
  local value
  value=$(tail -n 200 "$LOG" | grep -a ' soak: soak finished ' | tail -n 1 | sed -n "s/.* $1=\([0-9]*\).*/\1/p" || true)
  printf '%s\n' "${value:-$(fact "$1")}"
}

# Stop what the run started when the script is interrupted or ends: the example (it ignores
# SIGINT as a background job of a non-interactive shell, so it would run on to its end) and
# the pymodbus container.
cleanup() {
  if [ -n "${SOAK_PID:-}" ]; then
    kill "$SOAK_PID" 2>/dev/null || true
    wait "$SOAK_PID" 2>/dev/null || true
  fi
  if [ -n "${CONTAINER:-}" ]; then
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
  fi
}

main() {
  local interval=10 restart_every=120 period="" bound=4096 out="" target="" seconds=""
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -h | --help) usage; exit 0 ;;
      --self-test) self_test || exit 1; exit 0 ;;
      --interval | --restart-every | --period-ms | --max-growth-kib | --out)
        [ "$#" -ge 2 ] || { echo "soak: $1 needs a value" >&2; exit 2; }
        case "$1" in
          --interval) interval=$2 ;;
          --restart-every) restart_every=$2 ;;
          --period-ms) period=$2 ;;
          --max-growth-kib) bound=$2 ;;
          --out) out=$2 ;;
        esac
        shift 2
        ;;
      -*) echo "soak: unknown option '$1'" >&2; exit 2 ;;
      *)
        if [ -z "$target" ]; then target=$1; elif [ -z "$seconds" ]; then seconds=$1
        else echo "soak: too many arguments" >&2; exit 2; fi
        shift
        ;;
    esac
  done
  case "$target" in sim | pymodbus) ;; *) echo "usage: scripts/soak.sh [OPTIONS] sim|pymodbus [SECONDS]" >&2; exit 2 ;; esac
  seconds=${seconds:-86400}
  local value
  for value in "$seconds" "$interval" "$restart_every" "$bound" ${period:+"$period"}; do
    case "$value" in '' | *[!0-9]*) echo "soak: '$value' is not a whole number" >&2; exit 2 ;; esac
  done
  { [ "$seconds" -gt 0 ] && [ "$interval" -gt 0 ]; } || { echo "soak: SECONDS and --interval must be positive" >&2; exit 2; }
  if [ -z "$period" ]; then
    if [ "$target" = sim ]; then period=0; else period=10; fi
  fi

  local root name=stonecrab-soak port=5021 image=stonecrab-interop:local
  root="$(cd "$(dirname "$0")/.." && pwd)"
  cd "$root"
  out=${out:-target/soak/$target}
  mkdir -p "$out"
  local csv="$out/$target.csv" log="$out/$target.log" report="$out/$target.md"
  : >"$log"
  LOG=$log
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  echo "soak: building the soak example (release)"
  cargo build --release --locked -p stonecrab --example soak --features full
  local bin="${CARGO_TARGET_DIR:-target}/release/examples/soak"
  [ -x "$bin" ] || die "the example binary $bin was not built"

  local url=sim://soak
  if [ "$target" = pymodbus ]; then
    command -v docker >/dev/null 2>&1 || die "docker is not on PATH; the pymodbus target needs Docker"
    docker image inspect "$image" >/dev/null 2>&1 ||
      die "the image $image does not exist; build it with interop/pymodbus/interop.sh build"
    url="modbus-tcp://127.0.0.1:$port?request-timeout-ms=500"
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker run -d --name "$name" --log-driver json-file --log-opt max-size=10m --log-opt max-file=2 \
      -p "127.0.0.1:$port:5020" "$image" >/dev/null
    CONTAINER=$name
    # Ready means the example reads a register without an error: a connect proves nothing,
    # because Docker's port proxy accepts before the container listens.
    local tries=0 probe
    while :; do
      probe=$(STONECRAB_URL="$url" STONECRAB_SOAK_SECONDS=1 STONECRAB_SOAK_PERIOD_MS=100 "$bin" 2>&1 | grep -a 'soak finished' || true)
      case "$probe" in *"errors=0 "*) break ;; esac
      tries=$((tries + 1))
      [ "$tries" -lt 60 ] || die "the pymodbus peer did not answer reads within 60 attempts"
      sleep 1
    done
  fi

  echo "elapsed_s,rss_kib,reads,errors,reconnects" >"$csv"
  STONECRAB_URL="$url" STONECRAB_SOAK_SECONDS="$seconds" STONECRAB_SOAK_PERIOD_MS="$period" \
    "$bin" >"$log" 2>&1 &
  SOAK_PID=$!
  local pid=$SOAK_PID restarts=0 next_restart=$restart_every status=0 rss reads errors reconnects started
  started=$SECONDS
  echo "soak: $target for ${seconds}s, sampling every ${interval}s, process $pid"
  while kill -0 "$pid" 2>/dev/null; do
    # ps reports the resident set size in KiB on macOS and Linux; empty when the process is gone.
    rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ' || true)
    if [ -n "$rss" ]; then
      reads=$(counter "$log" reads) errors=$(counter "$log" errors) reconnects=$(counter "$log" reconnects)
      printf '%s,%s,%s,%s,%s\n' "$((SECONDS - started))" "$rss" "${reads:-0}" "${errors:-0}" "${reconnects:-0}" >>"$csv"
    fi
    if [ "$target" = pymodbus ] && [ "$restart_every" -gt 0 ] && [ $((SECONDS - started)) -ge "$next_restart" ] &&
      [ $((SECONDS - started)) -lt $((seconds - restart_every / 2)) ]; then
      docker restart -t 2 "$name" >/dev/null 2>&1 || true
      restarts=$((restarts + 1))
      next_restart=$((next_restart + restart_every))
    fi
    if [ $((SECONDS - started)) -gt $((seconds + 120)) ]; then
      kill "$pid" 2>/dev/null || true
      echo "soak: the example did not stop within 120 s of its duration; killed" >&2
    fi
    sleep "$interval"
  done
  wait "$pid" || status=$?
  SOAK_PID=

  FACTS=$(analyze "$csv" "$bound")
  local verdict
  verdict=$(fact verdict)
  local pass=1 notes=""
  [ "$verdict" = stable ] || { pass=0; notes="$notes the memory verdict is $verdict;"; }
  [ "$status" -eq 0 ] || { pass=0; notes="$notes the example exited $status;"; }
  if [ "$(fact late_reads)" -le "$(fact late_errors)" ] 2>/dev/null; then
    pass=0
    notes="$notes the last quarter of the run had no more reads than errors;"
  fi
  if [ "$target" = pymodbus ] && [ "$(total reconnects)" -lt "$restarts" ] 2>/dev/null; then
    pass=0
    notes="$notes fewer reconnects than container restarts;"
  fi

  write_report "$report" "$target" "$url" "$seconds" "$interval" "$period" "$restart_every" "$restarts" \
    "$bound" "$status" "$pass" "$notes" "$(basename "$csv")" "$root" "$image"
  cat "$report"
  [ "$pass" -eq 1 ]
}

# write_report FILE TARGET URL SECONDS INTERVAL PERIOD RESTART_EVERY RESTARTS BOUND STATUS PASS NOTES CSV ROOT IMAGE
write_report() {
  local file=$1 target=$2 url=$3 seconds=$4 interval=$5 period=$6 restart_every=$7 restarts=$8
  local bound=$9 status=${10} pass=${11} notes=${12} csvname=${13} root=${14} image=${15}
  local commit dirty="" cpu os
  commit=$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo unknown)
  [ -z "$(git -C "$root" status --porcelain 2>/dev/null)" ] || dirty=" plus uncommitted changes"
  cpu=$(sysctl -n machdep.cpu.brand_string 2>/dev/null || sed -n 's/^model name[^:]*: //p' /proc/cpuinfo 2>/dev/null | head -n 1)
  os="$(uname -sr) $(uname -m)"
  {
    echo "# Soak run: $target"
    echo
    echo "| Item | Value |"
    echo "| --- | --- |"
    echo "| Date (UTC) | $(date -u +%Y-%m-%dT%H:%M:%SZ) |"
    echo "| Tree | commit \`$commit\`$dirty |"
    echo "| Machine | ${cpu:-unknown}, $os |"
    echo "| Toolchain | $(rustc -V) |"
    echo "| Build | \`cargo build --release --locked -p stonecrab --example soak --features full\` |"
    echo "| Command | \`$([ "$target" = sim ] && [ -n "${STONECRAB_SOAK_FAULT_SECS:-}" ] && echo "STONECRAB_SOAK_FAULT_SECS=$STONECRAB_SOAK_FAULT_SECS ")scripts/soak.sh --interval $interval$([ "$target" = pymodbus ] && echo " --restart-every $restart_every") --period-ms $period --max-growth-kib $bound $target $seconds\` |"
    echo "| Target | \`$url\`$([ "$target" = pymodbus ] && echo ", container from \`$image\`, restarted $restarts times") |"
    echo "| Duration | ${seconds} s, one memory sample every ${interval} s ($(fact samples) samples) |"
    echo "| Workload | four tags per read, a ${period} ms pause after each read |"
    if [ "$target" = sim ]; then
      echo "| Faults | one failure step every ${STONECRAB_SOAK_FAULT_SECS:-5} s (\`STONECRAB_SOAK_FAULT_SECS\`; 0 for none) |"
    else
      echo "| Faults | the container restarted every ${restart_every} s (0 for none) |"
    fi
    echo
    echo "## Result"
    echo
    echo "| Measure | Value |"
    echo "| --- | ---: |"
    echo "| Read calls (total) | $(total reads) |"
    echo "| Errors (failed reads, failed items, failed reopens; total) | $(total errors) |"
    echo "| Reconnects (total) | $(total reconnects) |"
    echo "| Read calls / errors in the last quarter | $(fact late_reads) / $(fact late_errors) |"
    echo "| RSS minimum / maximum after the warm-up (KiB) | $(fact min_kib) / $(fact max_kib) |"
    echo "| Warm-up, not judged (s) | $(fact warmup_s) |"
    echo "| RSS median of the first / last quarter after the warm-up (KiB) | $(fact early_kib) / $(fact late_kib) |"
    echo "| Growth (KiB), bound ${bound} | $(fact growth_kib) |"
    echo "| Least-squares slope after the warm-up (KiB per hour), informational | $(fact slope_kib_per_h) |"
    echo "| Example exit status | $status |"
    echo
    if [ "$pass" -eq 1 ]; then
      echo "**Verdict: PASS, memory stable.** The samples are in \`$csvname\`."
    else
      echo "**Verdict: FAIL**,${notes%;}. The samples are in \`$csvname\`."
    fi
    if [ "$seconds" -lt 21600 ]; then
      echo
      echo "A short run: with a bound of ${bound} KiB over ${seconds} s only a leak above about $(awk -v b="$bound" -v s="$seconds" 'BEGIN { printf "%.0f", b / 1024 / (0.6 * s / 3600) }') MiB an hour fails. The verdict shows that the command works, not that nothing small leaks; the 24 hour run (86400) does."
    fi
  } >"$file"
}

main "$@"
