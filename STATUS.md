# Stonecrab status and engineering handoff

Updated: 2026-10-27.
Implementation baseline: `e7d887ee19d9285aa2eff7726cce51689cab83df`.
This document describes that code baseline; subsequent documentation commits may
follow it. Update this file when another implementation slice lands.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition. It is **not yet a functioning industrial protocol client**.
The repository currently contains tested, runtime-independent core data contracts.
There are no connections, device reads, protocol drivers or hardware integrations.

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.1.0**, a specification-stage placeholder.
- Current development versions: facade **0.2.0**, core **0.1.0**, both unreleased.
- The facade's minor bump records removal of the published `placeholder::status`
  API. Do not restore a compatibility shim without a real consumer requirement.

The code through the baseline above has been pushed. No new release was published.
There is no unfinished source edit at this handoff.

## Source documents and precedence

Read these before changing contracts:

1. [`docs/SPEC.md`](docs/SPEC.md): scope, parity definition and acceptance criteria.
2. [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md): normative coding policy,
   including links to repository-specific overrides.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): target crate graph and contracts.
4. [`docs/PROJECT_BRIEF.md`](docs/PROJECT_BRIEF.md): retained original README brief.
5. [`docs/adr/`](docs/adr/): accepted implementation decisions.
6. [`docs/parity/core.md`](docs/parity/core.md) and
   [`docs/parity/modbus.md`](docs/parity/modbus.md): current scope and compatibility notes.

The source documents contain **target** APIs, diagrams, future crates and CI gates.
Their presence does not mean those capabilities exist. Some original links refer
to an old repository owner or `main`; active metadata uses the repository above.
ADR-0004 explicitly overrides the original `main` branch policy with `master`.

## Implemented

### Workspace and development infrastructure

- Resolver-3 Cargo workspace, Rust 2024, shared metadata and strict safety lints.
- `crates/stonecrab`: public facade, re-exporting core.
- `crates/stonecrab-core`: no Tokio, executor or network dependency.
- `tools/stonecrab-testkit`: unpublished deterministic fixtures, development-only.
- Empty default feature sets in the publishable crates; no protocol feature flags yet.
- Apache license/notices, contribution/security policy, changelog and ADRs.
- Local mise tasks, CI workflow, package allow-lists and VS Code test debugging.

### Core data contracts

- `PlcValue` / `PlcValueType`: exact-width scalar values, including signed/unsigned
  integers, floats, Boolean, character, string, duration and explicit null.
  Integer access never silently converts counters to floating point.
- `TagId`: validated, nonempty UTF-8, at most 256 bytes, no control characters or
  edge whitespace; original identity is retained without normalization.
- `ProtocolAddress`: object-safe, driver-owned immutable address boundary with
  `Any + Debug + Send + Sync`. No real protocol address parser exists yet.
- `Tag`: shared immutable address, caller identity, positive count and type-hint
  consistency. Stable trait upcasting supports concrete-address downcasting.
- `Capabilities`, `Operation`, `SubscriptionMode`: explicit capability metadata,
  including the distinction between native and polling subscriptions.
- `PlcError`, `ErrorKind`, `Diagnostics`, `ConnectionState`, `FailureCategory`:
  small boxed errors, original source chains and optional honest context.
  `ConnectionState` is metadata, not an implemented lifecycle state machine.
- `Deadline`: explicit monotonic-time construction and expiry checks.
- `ResourceLimits`: finite positive budgets; immutable after builder validation.
- `Secret<T>`: redacted Debug/Display, explicit access; no zeroization claim.
- `ProtocolStatus`: original code/name and explicitly bounded retained raw bytes;
  Debug prints byte count, not payload.
- `ItemError` / `ReadItem`: original status, source chain, caller identity,
  sampling time and absent-or-actual source timestamp.
- `ReadRequest`: immutable bounded batches; duplicate and over-budget inputs
  cause sticky builder errors before growing internal collections.
- `ReadResponse`: exact request/result correlation, original request ordering,
  per-item failures, split-batch disclosure and monotonic interval validation.
  Missing, unexpected and duplicate results are rejected.
- `WriteOutcome` / `UnknownReason`: confirmed, rejected and unknown-after-transmit
  representations. These types do not implement a write operation or retries.

### Tests

Unit tests, consumer integration tests, executable documentation and Send/Sync
assertions cover the implemented contracts. The testkit's `FixtureAddress` is
deliberately not a driver or a real protocol parser.

Read correlation tests cover mixed success, all-item failure, missing/duplicate/
unexpected identities, time boundaries, and all six three-item result orderings
with distinct values attached to their identities.

## Verified evidence and reproduction

Development toolchain: **Rust 1.99.1**.
Declared MSRV: **1.97**; locally exercised with **1.97.1**.
Pinned tools: `cargo-llvm-cov` **0.9.1**, `cargo-deny` **0.20.2**.

From the repository root:

```sh
mise install
mise run check
mise run coverage
mise run dependencies
mise run package
```

Most recent local results for the implementation baseline:

- Formatting, strict Clippy, tests, doctests, documentation with warnings denied,
  and no-default-features checks passed.
- **100% measured executable line coverage** for released code, without excluding
  error paths. Maintainer testkit code is excluded from the released-code report,
  but its tests still run. Region coverage was approximately 99.92%; do not
  describe this as 100% branch coverage.
- Workspace tests and doctests passed on Rust 1.97.1.
- Cross-compilation checks passed for `aarch64-unknown-linux-gnu` and
  `armv7-unknown-linux-gnueabihf`. These are compile checks, not device tests.
- Dependency advisories, licenses, bans and registry-source checks passed.
  Unused license-allowance warnings are currently emitted by cargo-deny.
- Both publishable crate archives built and matched committed package allow-lists.

Additional commands used locally:

```sh
mise exec rust@1.97.1 -- rustup run 1.97.1 cargo test --locked --workspace --all-features
mise exec -- cargo check --locked --workspace --all-features --target aarch64-unknown-linux-gnu
mise exec -- cargo check --locked --workspace --all-features --target armv7-unknown-linux-gnueabihf
```

Hosted CI for `e7d887e` **completed successfully**:
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40442677439
This covers the configured current test, quality, cross-target, dependency and
package jobs. It does not establish the unimplemented fuzz, mutation, soak,
interoperability or hardware gates from the source standards.

## Important implementation and tooling details

- The actual API is `ReadRequest::builder(limits)`, not the brief's no-argument
  builder. It takes an absolute `.deadline(...)`; `.deadline_after(...)` does not
  exist. `Deadline::after(now, duration)` takes the clock reading explicitly.
- An absent request deadline requires a future connection to supply a bounded
  default. It does not authorize unbounded I/O.
- Requests and responses are data only. Passing their tests is not evidence of
  transport cancellation, reconnect, batching on the wire or protocol parity.
- Partial and all-item failures remain valid response data. Do not collapse them
  into a batch error or invent a device timestamp.
- Never turn an unknown write outcome into a rejection or automatically replay it.
- Address methods must return stable, credential-free metadata. Do not add an
  `as_any` shim; stable trait upcasting is already used.
- Core's testkit dependency is development-only. The path-only dependency policy
  exception is documented in ADR-0003; registry wildcard versions remain denied.
- Core integration tests using the unpublished testkit are workspace-only and not
  shipped in the core archive. Cargo emits an expected ignored-test warning while
  packaging. Packaged core unit tests and documentation remain self-contained.
- `mise run package` uses a **fresh Cargo cache**. Repeatedly verifying changed,
  unpublished sibling crates with the same version in a reused cache previously
  selected an older core archive. Do not remove the isolation as a cosmetic change.
- Update `packaging/stonecrab-core.txt` or `packaging/stonecrab.txt` when shipped
  files change, and review the resulting archive contents.
- New dependencies require a rationale and the applicable audit/ADR entry.
  Current runtime dependencies are `thiserror` and `bytes`, not a protocol client.

## Baseline and incomplete M0 work

[`upstream/manifest.toml`](upstream/manifest.toml) pins Apache PLC4X **v1.0.0**:

- Annotated tag object: `32a524591841ab56d26b032914ce06918148fbb6`.
- Peeled implementation commit: `ea229918f5d07700e04d7d69ab1645da679cb700`.
- Tag/commit relation verified against Git and the GitHub ref API.

**M0 is not complete.** Still required:

- Full protocol × transport × operation inventory checked against source.
- Capabilities present in non-Java implementations.
- Source-release signature verification.
- Requirements-to-test traceability and operation-level parity records.
- Reconciliation of address/configuration aliases before implementing parsers.

Verified Modbus pitfalls are recorded in `docs/parity/modbus.md`: numeric address
groups are decimal; `x` in forms such as `4x00016` is an area marker, not hexadecimal.
Upstream uses `holding-register:16:INT`, IEC-style type names and
`default-unit-identifier`, not the original brief's provisional syntax. The
protocol-address offset is one. Modbus TCP's upstream subscriptions are polling,
not native subscriptions. No Rust Modbus implementation or parity claim exists yet.

## Not implemented or not yet evidenced

- Driver registry, driver/connection traits and operational connection lifecycle.
- Typed connection configuration, access-policy enforcement and actual read/write I/O.
- Runtime-independent cancellation, state watching and clock/sleep abstractions.
- Simulator driver, scripted acquisition failures, fake time and common driver suite.
- Byte/raw/recursive/calendar `PlcValue` variants and byte/word ordering types.
- Complete write request/response batches, browse, discovery and subscriptions.
- Transport/codec crates, Modbus TCP/RTU, sensor profiles and board adapters.
- OPC UA, EtherNet/IP/Logix, S7 and remaining upstream protocols.
- Code generation or retained upstream fixture corpus.
- Parser fuzzing, mutation testing, soak/load/benchmark evidence, branch-coverage
  gates and the full proposed CI gate set in the source standards.
- Hardware verification, source-release signatures and a reviewed full parity matrix.
- Verified private vulnerability-reporting availability; this remains a release prerequisite.

## Recommended continuation

1. Review the source documents, ADRs and actual exported types; rerun the local
   gates and check hosted CI before changing code.
2. Complete M0 source inventory and test traceability. Do not equate protocol
   definitions or generated PDU variants with implemented upstream driver operations.
3. Settle the remaining core decisions needed for an operational simulator:
   connection configuration, deadline/cancellation ownership, state watching,
   missing value representations and complete write-batch contracts.
4. Build the next working vertical slice: explicit registry plus simulator and
   shared contract suite, using object-safe Send boxed futures and deterministic
   test time. Keep core independent of Tokio.
5. Follow with bounded codecs/TCP transport and native Modbus TCP acquisition,
   then sensor profiles/quality, RTU, optional bench adapters, OPC UA and PLC
   drivers in the staged order from `docs/SPEC.md`.

Keep changes reviewable, commit tested slices frequently, and maintain the existing
100% line-coverage gate. No first-party unsafe, panics on input, unbounded I/O,
silent sample loss, inferred byte order or automatic replay of ambiguous writes.
Do not introduce empty driver scaffolds or mark planned capabilities supported.

The maintainer normally pushes manually; push only when explicitly requested.
Do not publish another crate version or operate hardware without separate approval.
Never place credentials in files, logs, commit messages or handoff documents.
