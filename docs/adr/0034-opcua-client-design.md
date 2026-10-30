# 0034: OPC UA client design
## Status
Proposed (2026-10-30), as slice S76 of docs/POST_ALPHA_PLAN.md (Phase H, M3: "draft ADR-0034
'OPC UA client design' as Proposed"); slice S97 accepts it. It records how the maintainer's
decisions of 2026-10-30 shape the client: E14 (the hybrid codec, confirmed once S80 reports),
E17 (the feature and the version rule), E18 (the public additions) and E19 (the caller-driven
pump and the subscription end), with E21 and E22 as the parity record applies them. The
cryptography backend (E15) and the key and trust formats (E16) are ADR-0035's and slices
S101 to S104's; this ADR only names where they plug in. It amends ADR-0013 (two `ParseError`
variants, decision 5), extends ADR-0019 as the S80 addendum will record (decision 3), and
leaves `ConnectionGuard` (ADR-0014) unchanged for every other driver (decision 6). Nothing is
implemented by this slice.

## Context
Upstream's OPC UA driver (Apache PLC4X v1.0.0, commit ea229918) is a hand-written client
around Java classes generated from `opc-manual.mspec`, itself generated from the OPC
Foundation NodeSet (ADR-0019, Context). `Conversation.submit` gives each request a request id
and sequence numbers, encodes and chunks it, sends it at once and matches the response by
request id, so several requests are in flight on one channel (`Conversation.java:369-487`).
Channel renewal runs on a scheduler shared by every connection in the JVM, and each
subscription sends Publish from its own fixed-rate task (`SecureChannel.java:549-577`,
`OpcuaSubscriptionHandle.java:231-286`). There is no reconnect, no session re-activation and
no TransferSubscriptions. The parity record `docs/parity/opcua.md` has 241 rows (203 planned,
38 deferred) after S76; S75 and S76 found the losses a straight port would copy (the
divergence rows `OPCUA-DIV-*`).

The OPC UA binary encoding is little-endian and recursive. A Variant can hold Variants
(type 24) and arrays of them, a DataValue holds a Variant, a DiagnosticInfo holds an inner
DiagnosticInfo, and an ExtensionObject carries a body whose type the server names. Strings,
ByteStrings and arrays are length-prefixed with an `Int32` where -1 means null, distinct
from empty. Every length, count and depth is chosen by the server. Upstream added a nesting
bound after a stack overflow (`VariantNestingTest.java`: 2000, 3000, 4000 and 16000 nested
Variants refused, 32 read; the deepest message in its suites nests 29 contexts) and counts
checked against the remaining input (`VariantArrayCountLimitTest`,
`StructFieldArrayCountTest`).

Stonecrab is one crate (ADR-0018). Transports are deadline-bounded byte pipes with one owner
and no background task (ADR-0014, D15); `FramedTransport` takes `&mut self` for every
operation and `ConnectionGuard` owns it with a single in-flight slot. Codec primitives are
`BoundedReader` and `BoundedWriter`, and `ParseError` has no recursion or count variant yet
(ADR-0013 deferred `RecursionLimit`). The MSpec generator (ADR-0019) is proven on Modbus as a
`cfg(test)` cross-check; S77 to S79 add the constructs OPC UA needs and S80 measures it on the
real model. Subscriptions are polled since S55: a `Subscription`'s `recv` drives its producer
through `SubscriptionSender`, and dropping the sender ends the stream.

## Decision
1. **Feature and version (E17).** One `opcua` Cargo feature, off by default and not part of
   `full`, as `tls` is (ADR-0032). It enables `transport`; S101 adds the cryptography
   dependencies to it as optional ones (E15, ADR-0035). `pub mod opcua` carries
   `doc(cfg(feature = "opcua"))` and the lint promotions of `modbus` (`indexing_slicing`,
   `arithmetic_side_effects`, `as_conversions`). Every M3 change to the public API is additive,
   on a `#[non_exhaustive]` type or as a provided trait method, so the semver job against
   0.3.0 stays green through M3. The release number (0.3.x or 0.4.0) is a separate approval
   when M3 is done, because E12 is used. If S101 measures the CI time of building the
   cryptography for every `opcua` build as unacceptable, an additive `opcua-security` feature
   splits it off (E17 option 3); no other split is planned.
2. **The hybrid codec (E14).** The uniform service-message layer is generated: the request
   and response structures of the services in use, their enumerations, the status-code name
   table and the dispatch of known ExtensionObject bodies, from `opc-manual.mspec`,
   `opc-status.mspec` and `opc-attribute.mspec` vendored verbatim under
   `upstream/mspec/opcua/` (E13). It lives in `crate::opcua::generated`, ships `pub(crate)`
   in the library (not `cfg(test)`, unlike Modbus under ADR-0019 decision 8) and grows per
   slice from about 22 enabled cases (S88) toward the 56-case Java-parity set. Hand-written
   under `crate::opcua::codec`: the built-ins (String, ByteString, Guid, DateTime,
   StatusCode, NodeId in its six encodings, ExpandedNodeId, QualifiedName, LocalizedText),
   the recursive and open types (Variant, DataValue, DiagnosticInfo, the ExtensionObject
   envelope), and everything outside the message body: framing, chunking and security.
   Because `scripts/coverage-gate.sh` excludes `generated/` from the line floor, the
   generated layer is proven instead by a valid and a negative vector for every enabled case,
   a fuzz target on the known-body dispatch and byte-identical regeneration (`mise run regen`).
   A disabled case is refused with `InvalidDiscriminator`, which the ExtensionObject envelope
   turns into a Raw body, so a response carrying a type Stonecrab does not decode still
   succeeds. If S80 shows constructs the generator cannot emit, E14 falls back to option 2:
   the same structures hand-written in `crate::opcua::service`, with the generated output kept
   as a `cfg(test)` cross-check.
3. **The external-type contract.** The generator's `[external]` table (S79) maps an mspec
   type name to a hand-written Rust type with one fixed shape: a parse function from a
   `BoundedReader` and a depth budget, a serialize function to a `BoundedWriter` with the same
   budget, and an encoded length, each returning `Result<_, ParseError>`. The budget is a
   small `Copy` value passed by value through generated and hand-written code alike, because
   OPC UA recursion runs through both (a generated structure holds a Variant that holds an
   ExtensionObject whose body is generated again). It is spent on entry into every type that
   can contain itself directly or through others (Variant, DataValue, DiagnosticInfo, an
   ExtensionObject body and a generated structure on such a cycle), not on fixed-shape
   structures; at zero the parse fails with `ParseError::RecursionLimit` before reading
   further. The limit is one crate constant, 64 recursive entries unless S86 measures a
   reason to change it: above the 29 contexts of upstream's deepest test message, far below
   the 2000 upstream refuses, and small enough that no input reaches the stack. A missing
   external is a generation error recorded in `MANIFEST.toml`. ADR-0019 decision 5 makes a
   recursive type a generation error "until the first protocol that has one adds a depth
   argument and `ParseError::RecursionLimit`"; this is that protocol, and the S80 addendum to
   ADR-0019 amends decision 5 to match.
4. **Null and empty stay apart.** An array whose count is -1 is `None` and a count of 0 is
   `Some(vec![])`: generated arrays are `Option<Vec<T>>`, and the generator's
   `x == null ? -1 : COUNT(x)` (S78) writes them back the same way. A null String or
   ByteString (-1) is likewise distinct from an empty one in the codec. What a null becomes in
   a `PlcValue` is decided per type in the value mapping (`OPCUA-TYPE-STRING`,
   `OPCUA-TYPE-BYTE-STRING`: a null ByteString is `Null`, an empty one empty `Bytes`); the
   codec never merges the two, so a value read and written back keeps its form.
5. **Codec bounds.** Every server-declared length, count and depth is bounded before
   anything is allocated, through `BoundedReader` and `BoundedWriter` only, little-endian:
   - A String or ByteString length is read with `read_bytes` against the bytes present and
     the frame bound; it is never used to reserve memory.
   - An array count below -1 is no valid encoding and is refused as `MalformedData`. A count
     of fixed-width elements is checked as `count * width` against the remaining input; an
     array of variable elements grows one parsed element at a time, and its count must not
     exceed the remaining bytes (no element encodes in fewer than one byte). Either of these
     two failures is `ParseError::CountLimit`.
   - A Variant's `arrayDimensions` product is computed with checked arithmetic and must equal
     the element count; a mismatch is `MalformedData` and allocates nothing
     (`OPCUA-TYPE-SCALAR-ARRAY`).
   - An ExtensionObject body length is checked against the remaining input and the body is
     parsed inside exactly that many bytes, with trailing bytes refused.
   - The 8-byte message header's size is checked against the negotiated receive buffer and
     `ResourceLimits` before buffering, so a negative or oversized size is a typed error
     (upstream `OpcuaMessageCodec.java:59-64` is unbounded); reassembly is bounded by the
     negotiated maximum message size and chunk count, on receive and on send.
   - The value layer bounds what it builds on top: a `PlcValue::Struct`'s depth and field
     count, the browse walk's nodes, depth and references, the attribute cache and the
     structure-definition cache, each with a named limit.
   `ParseError` gains two variants, additive on the `#[non_exhaustive]` enum (an ADR-0013
   amendment): `RecursionLimit`, carrying the limit that applied, and `CountLimit`, carrying
   the field name, the declared count and the bound that applied. Like the others they carry
   no payload bytes. Both map to `ErrorKind::ResourceLimit`, as `OversizedLength` does: each
   is a bound the reader enforces, not a claim about what the peer meant. Fuzz targets:
   `fuzz_opcua_address`, `fuzz_opcua_connection_string`, `fuzz_opcua_decode`,
   `fuzz_opcua_frame` and `fuzz_opcua_chunk_reassembly`, plus the known-body dispatch.
6. **The pipelined request lane.** OPC UA keeps several requests in flight on one channel,
   and a Publish waits at the server for up to its timeout hint, so a single in-flight slot
   would hold every Read behind it. The OPC UA connection therefore does not use
   `ConnectionGuard`; a private lane in `crate::opcua::lane` owns the
   `FramedTransport` instead, and the guard stays as it is for Modbus and the simulator.
   - Sending: the lane gives each request its request id and sequence numbers, encodes and
     chunks it and sends it now, under a send lock, then registers a pending entry (request
     id to reply slot) in a table bounded by `ResourceLimits::max_pending_requests`; a full
     table is `Backpressure`.
   - Receiving: whichever caller is waiting for a reply reads the next frame for everyone,
     under a receive lock, and hands each reassembled message to the slot its request id
     names. No task is spawned. A reply for an unknown or abandoned request id is discarded
     and counted, never given to another request.
   - Checks per chunk: the sequence number is the previous plus one (the wrap rule stays
     UNVERIFIED until S106 checks it against open62541), and the channel id and token id are
     the channel's (upstream adopts them blindly). A ServiceFault fails its request as
     `DeviceException` with the status code and name; an ERR message fails every pending
     request at once with its status; an ABORT chunk fails its request at once
     (`OPCUA-DIV-ABORT-CHUNK`).
   - Deadlines and cancellation: each request has its own deadline on the injected clock
     (`request-timeout-ms`, the handshake timeout for the handshake steps). On the receive
     side a caller that is dropped or times out abandons its entry; a frame half read stays
     in the `FramedTransport` buffer for the next reader, so cancellation loses no other
     reply. The send side is not cancel-safe (`FramedTransport::send`): a send dropped or
     timed out part way through may leave a prefix of a chunk on the wire, which poisons the
     transport. The lane treats that as a transport loss of the whole channel: every pending
     request fails `Disconnected` (a write in flight is `WriteOutcome::Unknown`) and the next
     operation reopens, as the lifecycle bullet below says.
   - Lifecycle: the lane keeps the `ConnectionState` machine, `ReconnectPolicy` and backoff
     of ADR-0014 and D15, so `watch_state` reads as for any connection. On a transport loss
     every pending request fails `Disconnected` and is never replayed (a write in flight is
     `WriteOutcome::Unknown`); the next operation reopens and repeats the whole handshake
     with a new session (`OPCUA-OP-RECONNECT`).
7. **The caller-driven pump and the subscription end (E19).** Nothing runs between calls.
   Work that upstream schedules is done by the call in progress, on the injected clock:
   - Every operation first does what is due: a channel renewal at 75 percent of the
     effective lifetime, and the session-expiry check that replaces an expired session
     before the request is sent.
   - `Subscription::recv` drives a private session pump (`crate::opcua::pump`; named so that
     it is not confused with ADR-0031's `EventPump`), as a polling subscription's `recv`
     drives its reads since S55. The pump keeps a bounded number of Publish requests
     outstanding through the lane, routes each response by `subscriptionId`
     (`OPCUA-DIV-PUBLISH-ROUTING`), acknowledges only the sequence numbers it received
     (`OPCUA-DIV-PUBLISH-ACK`), re-reports cyclic values on their grid, renews the channel
     when due and delivers into each subscription's bounded queue. One `recv` drives the
     shared pump at a time; the others wait on their own queues. A slow subscriber never
     blocks a Read, because the lane serves both.
   - An idle connection with no caller renews nothing. Its channel or session may expire;
     the next call notices and reconnects. An opt-in owned task for idle renewal, in the style
     of `ConnectionGuard::supervise()`, is a later additive extension (E19 option 2), not part
     of M3.
   - After a transport loss every active native subscription ends: the session drops its
     sender, the consumer receives what is queued and then `None`, and `end_reason()` gives
     the typed `Disconnected`. Nothing is re-created and nothing is transferred; the caller
     subscribes again on the reconnected connection. The maintainer accepted this as SPEC's
     "subscription recovery" (E19); COMPATIBILITY.md says so when the rows land.
   A failed renewal fails the channel (`OPCUA-DIV-RENEWAL-FAILURE`), and a refused monitored
   item is a per-tag entry in `refused()`, not a log line.
8. **The credential channel (E18, E22).** Secrets never travel in the connection string.
   The URI keys `username` and `password` are refused (E22); a user name and password, or an
   X.509 user identity (S99 adds it to the existing `#[non_exhaustive]` `Credentials`), reach
   the driver through `ConnectionConfig`, which `DriverRegistry::connect_with(uri, secrets)`
   and a `ConnectionCache` credential provider supply. They never enter `ConnectionUri`, its
   `Debug` output or a cache key. Key and trust material is named by path
   (`tls.keystore`, `tls.trust-store`, `server-certificate-file`) and read once in `connect`
   with a size bound; refusals name the option, never the path or the content, as ADR-0032
   decision 6 does. A password with no user name is `InvalidConfig`
   (`OPCUA-DIV-PASSWORD-WITHOUT-USER`). A secure policy requires `server-certificate-file`
   and opens no NONE channel without it (E21); `tls.verify=false` is accepted only as that
   key and is recorded in diagnostics (E22).
9. **The public value set and API (E18).** Each addition is additive and lands in its own
   slice; the core value additions get their own ADR amending ADR-0006 and ADR-0025
   decision 9 (S98):
   - `PlcValue::Bytes` and `PlcValueType::Bytes`, appended after `List` so the `Ord` order
     holds, and a bounded, ordered-field `PlcValue::Struct`. Guid, NodeId, QualifiedName and
     StatusCode read as `String` (NodeId text parses back as an address); LocalizedText is a
     two-field `Struct`.
   - `ReadItem::server_time`, beside the source time and the quality
     (`OPCUA-DIV-READ-METADATA`).
   - A provided `Connection::browse` returning `Unsupported(Operation::Browse)` by default,
     with `BrowseRequest`, `BrowseResponse` (a per-query outcome, a flat list of items with
     parent indices and a truncated flag) and `BrowseNode`; a flat list is what keeps
     same-name siblings (`OPCUA-DIV-BROWSE-SIBLINGS`).
   - `SubscriptionSender::native`, a per-tag parameter side table on the subscribe request,
     `Subscription::refused()` and `end_reason()`, and new `LossCounters` fields (server
     overflow, sequence gaps, unknown notifications). `Subscription::recv` keeps its
     signature.
   - In `crate::opcua`, only the driver, its address type and the options a caller names are
     public. The lane, the codec, the generated layer, the pump and the security code are
     `pub(crate)` or private.
10. **Module layout.** `crates/stonecrab/src/opcua.rs` and `src/opcua/`, by the slice that
    adds each file:

    | Path | Contents | Slice |
    | --- | --- | --- |
    | `opcua.rs`, `opcua/driver.rs`, `opcua/connection.rs` | the module root, `OpcuaDriver`, the connection and its capabilities | S83, S91, S123 |
    | `opcua/address.rs` | `OpcuaAddress`, the tag and browse-query grammar | S83 |
    | `opcua/options.rs` | the connection string and the driver-owned options | S84 |
    | `opcua/codec.rs`, `opcua/codec/` | the hand-written built-ins, the recursive types, the depth budget | S85, S86 |
    | `opcua/frame.rs` | the header, the message types and the single-chunk `Framer` | S87 |
    | `opcua/generated.rs`, `opcua/generated/` | the generated service layer and its `MANIFEST.toml` | S88 |
    | `opcua/lane.rs` | the pipelined request lane (decision 6) | S89 |
    | `opcua/channel.rs`, `opcua/session.rs` | the secure channel (Hello, OpenSecureChannel, renewal, close) and the session | S90, S91, S109 |
    | `opcua/value.rs`, `opcua/read.rs`, `opcua/write.rs` | the value mapping, Read and Write | S92, S95, S98, S100 |
    | `opcua/chunk.rs` | chunking, reassembly and the per-policy size table | S94 |
    | `opcua/endpoint.rs` | endpoint selection | S96 |
    | `opcua/identity.rs` | the user identity tokens | S99, S108 |
    | `opcua/crypto.rs`, `opcua/crypto/` | the private backend trait, the policy table, key derivation, symmetric and asymmetric protection | S101, S102, S105 |
    | `opcua/keys.rs`, `opcua/trust.rs` | the client key store and the server-certificate verifiers | S103, S104 |
    | `opcua/browse.rs` | the browse walk and its items | S112, S113 |
    | `opcua/subscription.rs`, `opcua/pump.rs` | native subscriptions and the session pump (decision 7) | S115 to S118 |
    | `opcua/structure.rs` | custom structures | S119, S120 |

    A slice may split a file further and says so in its slice row; it adds no public item
    beyond decision 9 without amending this ADR. `opcua` has a coverage floor of 100 outside
    `generated/`.

## Consequences
- A build without `opcua` is unchanged: no new module, dependency or public item. Every
  public addition of M3 is additive, so 0.3.0 users upgrade without code changes.
- The generated layer ships in the library for the first time, so regeneration, the vector
  pair per case and the dispatch fuzz target become release gates for `opcua`.
- Reads, writes, browses and Publish share one channel concurrently, bounded by
  `max_pending_requests`, without a background task; a long-polling Publish does not delay a
  Read.
- An idle connection lets its channel and session lapse and pays a full handshake on the next
  call. A caller that needs the session kept warm must call something, until the owned-task
  extension exists.
- A transport loss ends native subscriptions; applications re-subscribe. No notification is
  replayed, and a gap the server reports is counted, not hidden.
- A recursion or count limit fails the one message with `ResourceLimit`; it never panics,
  overflows the stack or allocates by a declared size.
- OPC UA has its own request owner beside `ConnectionGuard`; a fix to one does not reach the
  other, so the shared parts (state machine, backoff, deadlines) stay in the shared types.

## Alternatives considered
- **Hand-write everything (E14 option 2).** Kept as the fallback if S80 finds constructs the
  generator cannot emit; it gives up the per-case vectors against the upstream model and
  costs more hand-written code to keep at 100 percent coverage.
- **Generate everything, built-ins and framing included (E14 option 3).** Generator gaps G11
  to G17, roughly four to six more weeks, for types that are few, stable and recursive in
  ways the mspec expresses poorly.
- **Use `ConnectionGuard` with its single in-flight slot.** Every Read would wait behind an
  outstanding Publish; a pipelined mode inside the guard would change the contract every
  other driver relies on.
- **A spawned reader task or scheduler, as upstream.** Ruled out by D15 and ADR-0014; it
  would also make the tests depend on scheduling rather than the injected clock.
- **Re-create subscriptions after a reconnect (E19 option 3).** A Stonecrab extension with
  no upstream counterpart (no TransferSubscriptions upstream) and an in-band contract nobody
  has asked for yet.
- **A global depth limit in `BoundedReader`.** It would count every nested read in every
  protocol; a budget passed through the recursive types bounds only what can recurse.
- **`Vec<T>` with empty for null.** Loses the distinction the wire carries and that a
  write-back must keep.
- **Credentials as URI keys with redaction (E22 option 3, E18 option 3).** Passwords would
  still reach `ConnectionUri`, logs and cache keys.

## Compliance
- To be added by the slices that implement it: `crates/stonecrab/src/opcua.rs` and
  `src/opcua/` as in decision 10; the `opcua` feature in `crates/stonecrab/Cargo.toml` (S83);
  `ParseError::RecursionLimit` and `CountLimit` in `crates/stonecrab/src/codec/error.rs` (S86);
  `protocols/opcua.toml` and the vendored mspec files with their manifest entries (S88).
- Gates: the `opcua` floor of 100 in `scripts/coverage-gate.sh` and the module lists of
  `scripts/lint-attrs.sh` and `mutants.toml` (S83); the `regen` job extended to OPC UA (S88);
  the semver job against 0.3.0 on every slice; Clippy's `disallowed-methods` ban on spawning;
  the fuzz targets of decision 5 in `fuzz-smoke`; the `interop-opcua` job against open62541
  v1.5.9 (S81) and the second peer of E20 (S122).
- `docs/parity/opcua.md` records every row with its planned tests and owning slice; the
  divergence rows `OPCUA-DIV-*` name the slice that implements each difference.
- S97 accepts this ADR and corrects the stale contract text (`ARCHITECTURE.md` and
  `connection.rs:15-17`) to agree with decisions 6 and 7.

## Addendum (S83, 2026-10-30): the address grammar
Slice S83 added the `opcua` feature, `crate::opcua` and `OpcuaAddress` (decisions 1 and 10),
and fixed the points the parity record left to it. The status of this ADR is unchanged.
- **Escapes.** A backslash before `\`, `;`, `[` or `|` makes that character part of a string
  identifier; a backslash before anything else is itself, so identifiers such as `C:\temp`
  read as upstream reads them. An unescaped `|` ends the identifier (and the attribute), so
  the key-value tail needs no `;TYPE` before it. `canonical()` always escapes the four
  characters. A string identifier with an unescaped `|` was valid upstream (upstream reads
  `s=a|b=c` as one identifier) and is not now: when the text after the `|` is a well-formed
  key-value list the address is re-read as the identifier `a` with the field `b=c`, a
  different node from upstream's, and otherwise it is `InvalidAddress`
  (`OPCUA-DIV-ADDRESS-ESCAPES`).
- **Array suffix.** The trailing run of numeric bracket groups of an identifier is the array
  selection and becomes an OPC UA `IndexRange`; other brackets stay in a string identifier,
  as upstream's `ArrayNotationParser.expressionPart` leaves them. Bounds are those of a Java
  `int`, as upstream reads them.
- **Identifier types.** `i` is an unsigned 32-bit number, `g` is 8-4-4-4-12 hexadecimal
  digits kept as the 16 bytes of the binary encoding, `b` is padded standard base64
  (`OPCUA-DIV-BINARY-ID`). The namespace is 0 to 65535 and the `ns=` group is required.
- **Bounds.** An address text, and the canonical text it parses to, is at most 8192 bytes
  and has no control character (the canonical text can be longer than the input, and it
  must parse back); messages echo at most 32 characters. A `b` identifier with non-zero
  bits before its pad is refused, so each byte string has one text.
- **Surface.** `OpcuaAddress`, `OpcuaIdentifier`, `OpcuaAttribute` and `OpcuaDataType` are
  public and the types of its accessors (decision 9 names only the address type; S97, which
  accepts this ADR, ratifies the three accessor enums and `OpcuaAddress::MAX_TEXT_BYTES`);
  `OpcuaAddress::parse_query` reads a browse query
  (empty and `**` mean `ns=0;i=85`). The driver's `parse_address` over it arrives with
  the connection string (S84) and stays crate-private until S91.

## Addendum (S84, 2026-10-30): the connection string
Slice S84 added the connection string and the driver-owned options (`opcua/options.rs`) and
a crate-private `OpcuaDriver` in `opcua/driver.rs`, whose `parse_config` and `parse_address`
take the parameters and return the result of the `Driver` trait methods, without the
receiver, so that S91, which adds the public driver and the connection, only wraps them (the
inherent and the trait method then share a name). The status of this ADR is unchanged.
- **Keys.** Only `security-policy` and `message-security` are accepted; every other key is
  `InvalidConfig` (D12), including the ones later slices implement (`discovery`, the
  timeouts, `encoding.*`, the browse and subscription bounds, `server-certificate-file`, the
  `tls.*` names), which each slice adds with its behavior. `username` and `password`, the
  framework keys and the legacy `insecure-certificate-verification` get a hint in the error.
- **Port and path.** The port is required (E22). The path is everything after the first `/`
  of the authority, verbatim, and the endpoint URL is `opc.tcp://host:port<path>`, refused
  over 4096 bytes (the Hello field limit; the core grammar's 4096-byte string limit would
  otherwise let a 4098-byte URL through).
- **Policy and mode.** `Basic128Rsa15` and `Basic256` are recognised and refused as
  deprecated (E16), so `SecurityPolicy` has four values. `message-security` is stored as
  configured; a secure policy, the default included, with mode `NONE` is refused, a pair that
  upstream accepts. With policy `NONE` the channel ignores the mode (`OPCUA-MODE-POLICY-NONE`,
  S90).
- **Visibility.** The options, the two enums and the driver are `pub(super)` and carry an
  expected `dead_code` until S91 calls them; S91 removes the expectation. The fuzz workspace
  reaches the parser through `stonecrab::opcua::fuzzing`, a public module that exists only
  under cargo-fuzz's `--cfg fuzzing` (declared in the workspace `unexpected_cfgs`), so no
  public API is added to a normal build and the semver job sees none.
- **Shared code.** The host and port parsing is a copy of the Modbus driver's (the `opcua`
  feature does not enable `modbus`), with a required port and a path; moving both to a shared
  module is a possible later cleanup.

