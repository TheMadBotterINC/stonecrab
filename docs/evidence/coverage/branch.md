# Branch coverage

Slice S62 (ADR-0005 addendum "S62"). Branch coverage per component, report only. The line
gate (`mise run coverage`, `scripts/coverage-gate.sh`) is unchanged and is still the only
coverage gate; no number here can fail a job.

## Reproduce

```sh
mise run branch-coverage        # scripts/branch-coverage.sh, about 1 minute after the build
```

It runs one instrumented `cargo llvm-cov nextest --branch` over the workspace with all
features on the nightly pinned for the fuzz workspace (`nightly-2026-10-01`, rustc
1.101.0-nightly), because cargo-llvm-cov 0.9.1 offers `--branch` (unstable) on a nightly
compiler only. It installs that nightly's `llvm-tools-preview` when missing and builds
into `target/branch-coverage-target` (about 1 GB; delete it to get the disk back). The
lcov file is `target/branch-coverage.lcov`; the table below is `target/branch-coverage.txt`.
The weekly `scheduled` job runs the script as a report-only step and uploads both files.

A branch is a boolean condition outcome as rustc's branch instrumentation records it:
the true and the false side of an `if`, a `while`, a match guard, the operands of `&&` and
`||`. A plain `match` arm is not a branch, so the counts are small next to the line
counts, and a condition that a macro or a derive expands counts like a written one.

## First numbers, 2026-10-29

Tree: commit `bfff873` plus the uncommitted S62 changes (no change under
`crates/stonecrab/src`). Apple M2 Max, macOS 26.6, nightly-2026-10-01, all features,
1,478 tests run, 44 skipped.

| Component | Branches | Hit | Percent |
| --- | ---: | ---: | ---: |
| core | 336 | 322 | 95.83% |
| codec | 14 | 14 | 100.00% |
| transport | 198 | 186 | 93.94% |
| sim | 126 | 119 | 94.44% |
| modbus | 334 | 327 | 97.90% |
| sensors | 210 | 208 | 99.05% |
| testkit | 378 | 307 | 81.22% |
| other | 0 | 0 | n/a |
| all | 1596 | 1483 | 92.92% |

The count varies between runs of the same tree: a second run gave sensors 207 of 210
(98.57%) and all 1482 of 1596 (92.86%), the other rows unchanged, so a branch that depends
on timing is hit in one run and not in another. Read each figure as good to about one
branch, and do not derive a threshold from a single run.

Reading it: every component that ships in the runtime (core, codec, transport, sim, modbus,
sensors) is above the 80% that SPEC.md asks for "where the pinned coverage tool supports
meaningful instrumentation", and the weakest, transport at 93.94%, is the one whose line
threshold is 95 for the reason in ADR-0005 (the connect-timeout arm that loopback cannot
reach). The testkit is test support (line threshold 90) and is at 81.22%. The line gate
and this report use different compilers, so the two are not expected to agree on which
code is covered, only on the order of magnitude.
