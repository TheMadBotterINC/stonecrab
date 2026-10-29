# Soak evidence

Slice S62 (ADR-0017 addendum "S62"). The soak run, the command for the 24-hour release
run, and the 10-minute runs that show the command works. The 24-hour run has NOT been
run: it ties up the machine for a day and needs the maintainer's approval first.

## What a soak run does

`scripts/soak.sh` (`mise run soak -- TARGET SECONDS`) builds the `soak` example
(`crates/stonecrab/examples/soak.rs`) in release mode and runs it for SECONDS against one
target. The example reads four tags in a loop through the public API (`DriverRegistry`,
`Connection::read`) and prints its counters once a second. The script samples the resident
memory of the example's process every `--interval` seconds (default 10) with `ps -o rss=`,
which gives KiB on macOS and Linux, joins it with the last counters and writes one CSV row:
`elapsed_s,rss_kib,reads,errors,reconnects`.

| Target | What it reads | Faults |
| --- | --- | --- |
| `sim` | the simulator in the same process, flat out (about 1.1 million reads a second) | a failure step pushed every 5 s, in turn a lost link, a lost reply and an undecodable reply, so the link is reopened about every 15 s |
| `pymodbus` | the interop peer (`interop/pymodbus/`, image `stonecrab-interop:local`) in a container of its own, `stonecrab-soak` on 127.0.0.1:5021, a read every 10 ms plus its own duration | the container is restarted every 120 s (`--restart-every`); the driver reconnects by itself. Docker's log of the container is capped at 20 MB |

Counters: `reads` is read calls; `errors` is read calls that failed or had a failed item,
plus failed attempts to reopen; `reconnects` is the link lost and back (a connection that
went through `Reconnecting` and returned to `Connected`, or one that ended `Failed` and was
reopened). Each injected fault is one error: the 10-minute simulator run, with a fault step
every 5 s, counted 119 errors (the example does not count the steps it pushes: 600 s / 5 s
is about 120 steps), and a third of the faults are lost links, which are the 40 reconnects.
The report takes the totals of the `soak finished` line of `TARGET.log`, the last sampled
counters when the example did not get to print it. A report of a run shorter than 6 hours
says so under the verdict, with the leak rate its bound can catch.

## The 24-hour run

```sh
caffeinate -i mise run soak -- sim 86400          # shell 1
caffeinate -i mise run soak -- pymodbus 86400     # shell 2, at the same time
```

`caffeinate -i` (macOS) keeps the machine awake; omit it on Linux. The pymodbus target
needs Docker and the image (`interop/pymodbus/interop.sh build`). Each target uses about
one core. Output goes to `target/soak/sim/` and `target/soak/pymodbus/` (`TARGET.csv`,
`TARGET.log`, `TARGET.md`); to record the run:

```sh
cp -R target/soak/sim docs/evidence/soak/24h-YYYY-MM-DD-sim
cp -R target/soak/pymodbus docs/evidence/soak/24h-YYYY-MM-DD-pymodbus
```

`scripts/soak.sh --help` lists the options (`--interval`, `--restart-every`, `--period-ms`,
`--max-growth-kib`, `--out`); `scripts/soak.sh --self-test` checks the verdict on built
CSVs (a flat series, a spiky one, a creep that stays under the bound, a leak, growth in the
warm-up only, too few samples) and runs without Docker or a build.

## Pass criteria

The first 20% of the run is a warm-up and is not judged. Of the samples after it, the
growth is the median of the last quarter minus the median of the first quarter. The run
passes when all of these hold:

1. the growth is at most 4096 KiB (`--max-growth-kib`);
2. the example exited 0;
3. in the last quarter of the run the read calls outnumber the errors (the link works at the
   end);
4. for `pymodbus`, the reconnects are at least the number of container restarts.

Fewer than 12 samples, or fewer than 8 after the warm-up, is inconclusive and fails.

### Why 4 MiB

RSS moves in 16 KiB pages on macOS arm64. In the 10-minute runs below, the growth after
the warm-up was 16 KiB on pymodbus and 80 KiB on the simulator, and 32 KiB in the
4-minute simulator run with a fault every second. 4 MiB is 51 times the worst of those.
The growth is the difference of two quarter medians that lie about 0.6 of the run apart
(the middle of the first and of the last quarter of the judged 80%), so a steady leak
reaches the bound at about 0.28 MiB an hour over 24 hours (4 MiB over 14.4 hours), and at
about 40 MiB an hour over 10 minutes. In 24 hours that catches two bytes per read on
pymodbus (about 61 reads a second as measured, so about 5.2 million reads) and 12 KiB
leaked per container restart (720 restarts); one byte per read and 6 KiB per restart pass,
and so does anything smaller. On the simulator (about 1.1 million reads a second) one byte
per read is caught within a 10-minute run. The 10-minute runs show that the command works
and that nothing large leaks, not that nothing small does. The self-test of
`scripts/soak.sh` checks a 24-hour-shaped series at half and at twice the rate of 0.28 MiB
an hour. The bound is the maintainer's to change with `--max-growth-kib`; the report prints the bound it used.

The report also prints the least-squares slope after the warm-up, for information. It is
not judged, because a series that rises in 16 KiB steps early and then goes flat has a
positive slope that says nothing about the end of the run (see the simulator below).

## Results of 2026-10-29: 10 minutes of each target

Apple M2 Max, macOS 26.6, rustc 1.99.1, release build, commit `bfff873` plus the
uncommitted S62 changes (nothing under `crates/stonecrab/src`). Both targets and the
4-minute run below ran at the same time, and `mise run check`, `mise run coverage` and a
`cargo package` ran on the machine during them; the read counts depend on that load, the
memory figures do not.

| | `sim` | `pymodbus` |
| --- | ---: | ---: |
| Duration, samples | 600 s, 119 | 600 s, 118 |
| Read calls | 669,161,300 | 36,220 |
| Errors | 119 | 7 |
| Reconnects | 40 | 4 (container restarts: 4) |
| RSS at the start of the judged part (KiB, median of the first quarter) | 2736 | 2736 |
| RSS at the end (median of the last quarter) | 2816 | 2752 |
| Growth (bound 4096) | 80 | 16 |
| Verdict | PASS | PASS |

The files are in `short-2026-10-29/`: `sim.md` and `pymodbus.md` (the reports), `sim.csv` and
`pymodbus.csv` (the samples), and the same under `sim-fault-1s/` for the extra run (4
minutes, a fault every second, `STONECRAB_SOAK_FAULT_SECS=1`: 80 reconnects, 279.7 million
reads, growth 32 KiB, PASS). The reports and CSVs are the unedited output of the script as
it stands in this tree (the runs were repeated after its last edit); only their directory
differs from `target/soak/`.

What they show. The pymodbus series is flat at 2720 to 2752 KiB from the second sample on,
through four restarts of the container. The simulator series rises in 16 to 32 KiB steps
from 2528 KiB to 2816 KiB and flattens in each stretch; that is the shape of an allocator reaching
its high-water mark and is also what a small leak per reopened link would look like (40
reopenings in the run, so about 7 KiB each if it were a leak). The 4-minute run with a
fault every second has twice the reopenings in a shorter time and rose by the same amount,
from 2560 to 2832 KiB, then went flat, which favours the high-water mark; only the 24-hour
run can settle it: a leak of 7 KiB per reopening would add about 40 MB over its 5,760
reopenings and fail the bound. The first sample of a process is taken between its fork and
its exec, so it is not a memory figure of the example and varies between runs: 32 KiB in
the simulator runs here, 2368 KiB in the pymodbus run, 176 KiB in another pymodbus run. It
falls in the warm-up and is not judged.

What they do not show: file descriptors, task counts and CPU, which SPEC.md lists for the
soak and `ps` does not give portably; a longer-term leak; behaviour on Linux or on a
target other than these two. The memory figure is the resident set size from `ps -o rss=`,
which counts resident pages only: on macOS, pages the system has compressed or swapped out
under memory pressure are not counted, so a leaking process could read flat while the
machine is short of memory. Run the 24-hour soak on a machine with free memory.
