# S97 ADR acceptance and contract alignment evidence

Date: 2026-11-03. Base: `12494d9`; plan: `e48bed4`; accepted contracts: `5966ee4`.
This slice changes Markdown and Connection rustdoc only. It adds no executable code,
public API, dependency, feature, version, capability or parity support state.

## Decisions recorded

ADR-0034 is Accepted. The outcome table in POST_ALPHA_PLAN and the S97 maintainer
ratification in STATUS record the already-approved E13/E14/E17/E18/E19/E21/E22 choices:
source/test redistribution with the approved notices; generated private service bodies
with hand-written codec/security; one opt-in feature outside default/full and version
0.3.0 unchanged; additive values/metadata/browse/credential/subscription APIs; caller-driven
renewal and Publish, typed subscription end and caller re-subscribe; preprovisioned server
certificate for secure discovery; explicit port, no URI credentials, and the recorded
verification opt-in. E14 is confirmed by S80/S88. E15/E16/E20 implementation acceptance
remains S101/S103/S122, and ADR-0035 stays Proposed.

Decision 9 ratifies S83's OpcuaIdentifier, OpcuaAttribute, OpcuaDataType and
OpcuaAddress::MAX_TEXT_BYTES. It explicitly keeps OpcuaOptions private and recognizes
that the additive core APIs are shared across features, without an OPC UA dependency
for builds that omit that feature. The old ad-hoc address-only public-surface wording
and expired generator fallback condition are corrected.

## Contract reconciliation

Architecture's module/support map, trait methods, value/metadata targets, lifecycle,
connect/reconnect diagrams and feature table now distinguish shipped S96 behavior from
accepted later-slice work. Modbus keeps its serialized ConnectionGuard and optional
supervisor; OPC UA keeps its bounded private lane and no spawned reader/renewal task.
The accepted private opcua::pump drives session protocol work from operations/recv and
is distinct from ADR-0031's public EventPump for named read batches.

The accepted native-subscription end preserves queued events, then recv returns None
with typed Disconnected through end_reason(); the caller re-subscribes after reconnect.
The diagram no longer promises automatic re-creation. No caller means no idle renewal.
These contracts remain staged in S109/S110/S114–S118, with current OPC UA subscriptions
still Unsupported. Secure discovery, trust and protected credentials remain later slices.

Connection rustdoc now allows driver-specific bounded concurrent requests instead of
implying one in-flight request for every protocol. The shipped README no longer claims
that no OPC UA driver or connection exists. Completed Phase H rows S76–S97 are annotated;
S98–S123 remain planned. The 48-slice table remains unique and has five columns per row.

## Validation

- `mise run check`: fmt, strict all-target/all-feature Clippy, 2321 nextest tests,
  the full cargo-test suite, 584 normal and five compile-fail doctests, warning-free
  rustdoc and no-default-features build pass.
- `mise run parity` plus its self-test and `mise run lint-attrs` pass. Global parity
  references remain 325; OPC UA remains 101 implemented/110 planned/38 deferred.
- A scratch documentation-consistency check confirms the accepted ADR status, seven
  outcome records, S83 public address surface, pump/end/bootstrap wording, unique Phase H
  ownership/status, five-column slice rows and relative Markdown file links. It also
  compares executable source after stripping Rustdoc, and compares the manifests, lock,
  package inventory and OPC UA parity record byte-for-byte against the base.
- `mise run package`: fresh-CARGO_HOME package verification and the unchanged reviewed
  198-file inventory pass. Archive source is 5966ee4591076b1c16db132be50f40fb9a5cd8a1;
  812027 bytes, SHA-256 08b302555093632bf979dbec657a90f3e735f5e8cff311043ca39fe29648f52c.
- DCO and final diff checks pass; every slice commit and the normal merge are signed off.

Runtime coverage, mutation, feature-matrix, fuzz and independent-peer evidence remains
[S96](opcua-s96.md); it was not rerun for documentation-only changes. No new behavioral
regression test is warranted because executable tokens are unchanged. The full existing
suite and documentation checks were rerun. No support level is raised by ADR acceptance.

Self-review follows ADR-0005's single-maintainer waiver. The plan was committed before
edits, current/target claims were checked against source and parity, and the published
README was included in package verification. No hardware, tag or publication was used.
Next: S98, additive Bytes/Struct values and read metadata under their own ADR.

Local logs: /tmp/stonecrab-s97-{check,package,consistency,parity,lint-attrs,dco}.log.
The scratch checker is /tmp/stonecrab-s97-consistency.py.
