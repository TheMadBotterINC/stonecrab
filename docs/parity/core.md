# Foundation scope
Status: initial implementation; no upstream behavioral parity claim.

The architecture maps values, tags, errors and capabilities to PLC4J concepts.
This first slice provides only pure foundation types. There is no `DriverRegistry`,
connection, request/response batching, subscription, codec, driver or sensor profile.

Each implemented contract has unit and executable documentation tests. The complete
upstream protocol/transport/operation inventory is still required before M0 can close.
Calendar and structured values require a further design decision.

## Implemented foundation
- `Capabilities`, `Operation`, `SubscriptionMode`: explicit operation reporting.
- `PlcError`, `ErrorKind`, `Diagnostics`, `ConnectionState`, `FailureCategory`:
  boxed failures with concrete source retention and optional, never invented context.
- `TagId`: bounded, validated caller identifiers; not protocol address parsing.
- `Deadline`: deterministic explicit-time construction and evaluation.
- `ResourceLimits`: immutable finite budgets, positive values and buffer/frame
  consistency checked by a builder before allocation.
- `Secret<T>`: redacted Debug/Display; no memory-zeroization claim.
- `PlcValue`, `PlcValueType`: exact scalar widths, lossless checked integer widening
  and explicit float-only access. Byte/raw/calendar/collection values remain deferred.
- Public facade integration and compile-time Send/Sync assertions.

Unit tests live beside each implementation; `crates/stonecrab/tests/public_contract.rs`
checks the consumer boundary. README examples are executable doctests.
