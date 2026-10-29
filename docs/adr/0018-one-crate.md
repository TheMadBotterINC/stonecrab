# 0018: One crate
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S33 of docs/ALPHA_PLAN.md. It
supersedes decision D1 of the plan (the 0.2.0 facade plus 0.1.0 sibling versions) and, in
part, ADR-0001 (the workspace crate graph), ADR-0011 (the facade re-export decisions),
ADR-0014 (the scoping of Tokio to a transport crate) and ADR-0017 (the publish order,
the per-crate tags and the yank rule for pinned siblings). It amends ADR-0003 and ADR-0009
(the shape of the dev-dependency cycle and the "unpublished testkit") and ADR-0005 (per-crate
coverage tiers become per-module thresholds). The architecture, the contracts, the
tests and the gates are unchanged; only the packaging of the code changes. Nothing is
published by this slice.

## Context
Through slice S32 the alpha was seven published crates and a test kit: a facade
(`stonecrab` 0.2.0), six sibling crates at 0.1.0 (`stonecrab-core`, `-codec`,
`-transport`, `-driver-sim`, `-modbus`, `-sensors`) and an unpublished test kit
(`stonecrab-testkit`). The split followed
the original architecture diagram and kept the foundation free of Tokio and of protocol
code. It also meant a reader had to learn seven names and three version numbers before
writing `use`; a release published seven archives in dependency order; a fix in one crate
needed a coordinated patch release of the facade that pinned it with `=`; and the audience
of the project's README includes people who are not Rust engineers and want one thing to
add and one version to ask about.

The maintainer decided on 2026-10-27 that Stonecrab ships as one package that is simple to
explain: `stonecrab`, one version, one line to add it. The separation that mattered (no
Tokio in the core, no protocol code in the foundation) is a property of the dependency
graph, and Cargo features and modules express that graph without separate packages.

## Decision
1. **One crate.** `crates/stonecrab` is the only package in the workspace and the only
   published crate. It is `stonecrab` 0.2.0. The former crates are modules, and the
   items of `stonecrab-core` are at the crate root, so the public paths are the paths of
   the old facade where it re-exported them (`stonecrab::DriverRegistry`) and
   `stonecrab::<module>::...` otherwise.

   | Module | Path | Former crate |
   | --- | --- | --- |
   | core contracts | crate root | `stonecrab-core` |
   | `codec` | `stonecrab::codec` | `stonecrab-codec` |
   | `transport` | `stonecrab::transport` | `stonecrab-transport` |
   | `sim` | `stonecrab::sim` | `stonecrab-driver-sim` |
   | `modbus` | `stonecrab::modbus` | `stonecrab-modbus` |
   | `sensors` | `stonecrab::sensors` | `stonecrab-sensors` |
   | `testkit` | `stonecrab::testkit` | `stonecrab-testkit` |

2. **Features.** The default features give the simulator, Modbus TCP and the sensor
   profiles, so `cargo add stonecrab` needs no flags. `default-features = false` is the
   Tokio-free core: the crate root and `codec`, which pull `thiserror` and `bytes` only.

   | Feature | Enables | Pulls in | Default |
   | --- | --- | --- | --- |
   | `sim` | `stonecrab::sim` | `tracing` | yes |
   | `modbus` | `stonecrab::modbus` | `transport`, `tracing` | yes |
   | `sensors` | `stonecrab::sensors` | `serde`, `toml` | yes |
   | `full` | the same set as the defaults, spelled out (the `profile_read` example requires it) | `sim`, `modbus`, `sensors` | no |
   | `transport` | `stonecrab::transport` | `tokio` (`rt`, `net`, `time`, `sync`, `io-util`) | no (on through `modbus`) |
   | `testkit` | `stonecrab::testkit` | `transport`, `serde`, `serde_json`, `toml`, `tokio/macros` | no |

   `transport` is the only feature that brings Tokio in, so the scoping rule of ADR-0014
   and CODING_STANDARDS 8.1 (Tokio only where a transport needs it, never `full`, none in
   the core) now reads: Tokio enters through the `transport` feature and no other. The
   former `tcp` and `toml` features are gone; the TCP transport is part of `transport`
   and the TOML loader is part of `sensors`. The `full` alias is kept. Features are
   additive and are part of the public API. Examples declare `required-features`, and
   so does each integration test file whose `#![cfg(...)]` names a feature, so a plain
   `cargo test` skips a file it cannot compile instead of building an empty test crate.

3. **The test kit is a feature.** `stonecrab::testkit` is behind the off-by-default
   `testkit` feature. A separate test crate cannot exist: the kit needs the `Driver`
   contract of `stonecrab`, and the tests of `stonecrab` need the kit, which is a
   dev-dependency cycle (the same shape ADR-0003 and ADR-0014 handled with features of
   the kit crate). Making the kit a module removes the cycle. It is not a runtime feature,
   an application never enables it, and it ships in the package so that authors of other
   drivers can run the same contract suite (`ContractSuite`) against their driver.
4. **Module-level lint promotions.** The crate-wide promotions (`missing_docs`,
   `clippy::as_conversions`) stay inner attributes of `lib.rs`. The promotions that
   were crate-level in the codec, the transport and the driver crates become outer
   attributes on the `pub mod` declarations in `lib.rs`: `codec`, `modbus` and `sim` deny
   `indexing_slicing`, `arithmetic_side_effects` and `as_conversions`; `transport` denies
   `indexing_slicing` and `as_conversions`; `sensors` and `testkit` carry none.
   `scripts/lint-attrs.sh` checks the crate-level table, a per-module table, and that the
   module root files carry no inner lint attribute (CODING_STANDARDS 5.1).
5. **Per-module coverage thresholds.** `scripts/coverage-gate.sh` runs one instrumented
   nextest run with `--all-features` and sums the lcov lines by path prefix of
   `crates/stonecrab/src`, with the thresholds of ADR-0005 unchanged: core, `codec`, `sim`,
   `modbus` and `sensors` 100 percent, `transport` 95, `testkit` 90. A directory under
   `src/` without a row fails the gate.
6. **One release unit.** One packaging allow-list (`packaging/stonecrab.txt`, checked by
   `mise run package`), one SBOM (`scripts/sbom.sh`), one `cargo package`, `cargo publish
   --dry-run` and `cargo publish`, one annotated tag `v0.2.0` on the release commit, one
   version and one changelog section. ADR-0017 is amended accordingly (publish
   commands, tag, yank).
7. **The documents.** The README describes one package. Each module's rustdoc root is a
   component document in `crates/stonecrab/docs/components/` (`core.md` is compiled for its
   doctests only, because the crate root's rustdoc is `README.md`). The deferred crates of
   ARCHITECTURE.md become deferred modules; whether a future large driver is split into
   its own crate is decided by the ADR that adds it.

What this supersedes or amends, precisely:

- **D1 (versioning).** Superseded: one version, 0.2.0, for everything. `-alpha.N` stays
  rejected for the reason in ADR-0017 (the published 0.1.0 placeholder would win
  resolution over a pre-release).
- **ADR-0001.** The "workspace crate graph" (separate crates for core, transport, codec,
  drivers and the kit) is superseded; the rule that dependencies point downward, that the
  core is executor-independent and that no public driver trait exists before the
  simulator can exercise it stands, now as module edges and features.
- **ADR-0003.** The dev-dependency cycle between the core and the test kit no longer
  exists; the kit is the `testkit` feature of the same crate.
- **ADR-0009.** "The testkit is unpublished" is superseded: it is published inside the
  package behind its feature. Its policies (golden vectors, fuzz workspace, `proptest`
  adoption, example rules) stand.
- **ADR-0011.** The decisions about what the facade re-exports and which feature turns
  on which sibling are superseded by the feature table above; the `Driver`,
  `Connection` and `DriverRegistry` boundary stands.
- **ADR-0014.** Tokio's scoping to the `stonecrab-transport` crate and its `tcp` feature is
  now the `transport` feature (decision 2). D15, D21 and D22 stand.
- **ADR-0005.** Coverage tiers per crate become thresholds per module (decision 5).
- **ADR-0017.** The seven-crate publish order, the per-crate tags and the rule to yank a
  pinned sibling with the facade are replaced (decision 6).
- **ADR-0012, ADR-0013, ADR-0015, ADR-0016.** They record the creation of a crate; read
  the crate name as the module name. Their decisions stand.

The text of the superseded ADRs is not rewritten, except ADR-0017, which is amended in
place (its own note says so). Each of the others carries a short note at the top of its
status section that points here.

## Alternatives considered
- **Keep the split.** Stronger compile-time isolation (a driver crate cannot reach into
  another's private items) and separate versions for stable and unstable parts. Rejected:
  the isolation is already enforced by the dependency edges between modules and by the
  `features` job, the separate versions were a cost without a user (nothing outside this
  repository depends on a sibling), and seven names and three versions are what the
  maintainer wanted to remove.
- **One crate with everything always on.** The simplest manifest. Rejected: it would put
  Tokio, `tracing`, `serde` and `toml` into every build including constrained targets,
  and make the Tokio-free core impossible to ship. The default-on features keep the
  one-line install and the opt-out keeps the lean build.
- **One crate, nothing on by default** (the old facade). Rejected for the audience: a
  newcomer who runs `cargo add stonecrab` and gets nothing usable is the problem this
  decision removes.
- **A test crate published separately.** Rejected: the dev-dependency cycle of decision 3.

## Consequences
- One version and one changelog: `CHANGELOG.md` has one "stonecrab 0.2.0" section grouped
  by module. A fix to any module is a release of the whole crate; there is no patch
  release of a single part.
- The crate's package is larger than the old facade, but one archive (102 files, the
  kit included behind its feature, no fixtures or test data) replaces seven.
- A plain `cargo test` builds the default features without `testkit` and runs the
  reduced suite; the test files that need the kit are skipped by `required-features` (cargo
  prints no notice for a skipped test file; naming one with `--test` reports the missing
  feature). The gates and CI run with `--all-features` and run everything, and
  `mise run check` does the same. The counts of tests and doctests are unchanged by the
  merge.
- The kit's in-source tests and doctests read repository files (`testdata/`,
  `fixtures/`), which the archive does not contain: the full suite runs from a checkout,
  not from the packaged archive. Users of the package get the kit's code and its
  documentation, not its repository fixtures.
- Lint promotions and coverage are per module, enforced by scripts, instead of per
  package by Cargo. A new module must be added to the tables of both scripts.
- The `semver` job now has a baseline (the published `stonecrab` 0.1.0) and will report
  the removed `placeholder::status` as a breaking change of a 0.x crate; the 0.2.0 minor
  bump is the intended answer, as in ADR-0017.
- A user who read "seven crates" in an earlier checkout, or the old crate names in the
  ADRs and CHANGELOG, must map them with the table in decision 1. Those names remain only
  as history.

## Compliance
- `crates/stonecrab/Cargo.toml` is the one package manifest; `Cargo.toml` at the root lists
  it as the only member. `cargo tree -p stonecrab -e normal` shows `tokio`, `tracing`,
  `serde` and `toml` with the default features and only `bytes` and `thiserror` with
  `--no-default-features`.
- `scripts/lint-attrs.sh`, `scripts/coverage-gate.sh`, `mutants.toml`, `release.toml`,
  `packaging/stonecrab.txt` and `scripts/sbom.sh` are written for the one crate; the
  `features` job checks the powerset and the lean build.
- The documents describe the single crate; the old names appear only in explicit history
  (superseded ADRs, the STATUS.md slice table, dated CHANGELOG entries, docs/ALPHA_PLAN.md),
  which the final grep of slice S33 enumerates.

## Addendum (S38): `sensors` denies `indexing_slicing`
Decision 4 said `sensors` carries no module-level promotion. The module parses untrusted
TOML, so it now takes `#[deny(clippy::indexing_slicing)]` on its declaration in `lib.rs`,
like `codec`, `transport` and the drivers; the other lints are not promoted there. The
`sensors` row of the per-module table in `scripts/lint-attrs.sh` and CODING_STANDARDS 5.1
list it. The module already had no indexing or slicing, so no code changed.

## Addendum (S59, 2026-10-29): `modbus` enables `serial`
ADR-0024 decision 13 amends the `modbus` row of the feature table of decision 2: `modbus`
pulls in `transport`, `serial` and `tracing`, so the default build contains `serial2`
(BSD-2-Clause OR Apache-2.0; it adds no other crate to the lock file), as the S58 addendum
of ADR-0014 announced. The Modbus RTU driver needs the serial transport, and the
`modbus-rtu` scheme is part of the `modbus` module. No feature is added or removed, and the
Tokio-free core of `default-features = false` is unchanged.

## Addendum (S71, 2026-10-29): the opt-in `tls` feature
ADR-0032 adds a `tls` feature to the feature table of decision 2: it pulls in `transport`,
`rustls` 0.23 with the `ring` provider and `tokio-rustls` 0.26, and gives `modbus-tcp` its
`tls` transport when `modbus` is on too. It is in neither the default set nor `full`, so the
default build and its lock-file closure are unchanged and a user opts in with
`features = ["tls"]`. It is still one crate with one version (decision 1); the new crates and
their licenses are in `docs/DEPENDENCY_AUDIT.md`.
