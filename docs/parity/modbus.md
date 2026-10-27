# Modbus baseline: not implemented
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0,
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

The Java driver reference is `plc4j/drivers/modbus`; MSpec is
`protocols/modbus/src/main/resources/protocols/modbus/modbus.mspec`.
Reference vectors are in `protocols/modbus/src/test/resources/protocols/modbus`,
with TCP, RTU and ASCII parser/serializer suites. No files have been copied yet.
Retain Apache license headers and provenance when adapting fixtures.

The original project brief's `holding:0x0010:i16` and `unit-id` examples are
provisional, not established upstream-compatible syntax. The pinned Java source
uses a `holding-register` area name, IEC type names such as `INT`, and the
`default-unit-identifier` configuration parameter. Exact accepted syntax and
aliases must be source-checked before writing the Rust parser.

Upstream Modbus TCP subscription support uses polling emulation, not native
subscriptions. Do not infer implemented driver operations merely from MSpec PDU
variants; inspect the connection hooks and capability derivation.

## Required evidence before a read capability claim
- Address and configuration tests against pinned source semantics.
- Independent success/error vectors, malformed lengths and function codes.
- Fragmentation, coalescing, close boundaries and transaction/unit mismatch.
- Timeout, cancellation, late replies, resynchronization and reconnect.
- Tag correlation and partial failures across batch splitting.
- Independent server interoperability and explicit hardware status.

All Rust protocol/transport/operation records remain planned. Full baseline
inventory and test traceability are incomplete; M0 is not declared complete.
