# 0036: Bytes, bounded structures and read metadata

## Status

Accepted design (2026-11-03), S98 of POST_ALPHA_PLAN, implementing the E18 approval
recorded by S97. Amends ADR-0006's byte/structure deferral and ADR-0025 decision 9.
Implementation and validation evidence follow before merge; no release is authorized.

## Design and implementation plan

Append Bytes and Struct to the existing non-exhaustive PlcValue/PlcValueType enums,
preserving previous type ordering; an immutable PlcStruct validates owned ordered fields
before exposing them. The existing OPC UA mapper consumes decoded ByteStrings and
LocalizedText into those types, while the read mapper attaches original status and both
optional timestamps to ReadItem; replace only the named deferred mapping branches.

- `value.rs`: Bytes(Vec<u8>) and Struct(PlcStruct), with a root-exported PlcStruct whose
  fields are private. `new(Vec<(String, PlcValue)>)` preserves field order, rejects duplicate
  names as InvalidConfig, and bounds fields per structure to 1024, composite depth through
  lists/structures to 64, and total value/container nodes to 1,048,576. Iterative validation
  checks child counts before extending its work list. Rejected caller-owned trees are
  discarded iteratively, including deeply nested pre-existing List values. No field-name
  normalization, numeric coercion, map dependency, mutable escape or configurable limits.
  Expose fields/get/into_fields and associated limits; values remain caller-owned data,
  with no byte-length inference. ByteString wire lengths already have codec/lane bounds.
- `response.rs`: server_time defaults None; with_server_time/server_time mirror source_time.
  No quality enum is added: ProtocolStatus remains the quality/status channel. A Bad code
  still fails the item even if the peer supplies a value; status and both timestamps stay
  on that failure. An Uncertain value remains decoded with its exact code/name attached.
- `opcua/value.rs`: move ByteString bytes without cloning; null differs from empty Bytes.
  LocalizedText becomes exactly ordered text/locale fields, missing/null parts Null and
  explicit empty text String(""). Reject invalid UTF-8 per item. Arrays/matrices keep
  shape and charge three nodes per LocalizedText (structure plus two fields), preflighted
  against the existing shared response budget before construction. Failed mapping does
  not spend that budget; deferred complex types stay named Unsupported errors.
- `opcua/read.rs`: reuse DateTime::to_system_time and the source timestamp convention:
  absent/zero/unrepresentable timestamps stay None, never a local/epoch substitute.
  Preserve whole-nanosecond picosecond refinements: clamp the encoded 10 ps count to 9999,
  then divide by 100; the sub-nanosecond remainder cannot fit SystemTime and is discarded.
  A picosecond field without a timestamp is ignored. Source and server times use the same
  pure conversion, independent of the local monotonic sampling time.
- Update exhaustive matches and rejection tests in the simulator and sensor validation;
  these protocols gain no byte/structure support. Keep Modbus encodings unchanged.
  Extend existing value/response/mapping/property/fuzz and public peer tests; add no new
  service, background task, feature, dependency or version. Source layout stays in the
  existing modules; PlcStruct is necessary to enforce construction bounds for Struct.
- Keep all coverage floors/exclusions unchanged. Verify the full suite/doctests, core and
  OPC UA 100%, feature matrix in isolation, strict default/fuzz lint, semver default/all,
  regeneration, fuzz, focused mutations, package/unpacked build, parity and independent
  open62541 reads. Commit tested layers, then normal signed-off merge/push.

Reference checks: [Part 4 DataValue](https://reference.opcfoundation.org/specs/OPC-10000-4/7.11)
requires ignoring a Bad value; its item remains a failure. [Part 6 DataValue](https://reference.opcfoundation.org/specs/OPC-10000-6/5.2.2.17)
defines the optional timestamps and 10 ps refinements/clamp.
[Part 6 LocalizedText](https://reference.opcfoundation.org/specs/OPC-10000-6/5.2.2.14)
defines independently optional locale/text fields. The binary codec retains wire forms;
this ADR defines the public value representation, not new decoding rules.
