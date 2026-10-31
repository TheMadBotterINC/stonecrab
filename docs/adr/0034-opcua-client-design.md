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
   slice from about 22 enabled cases (S88) toward the 61-case Java-parity set (S80's count;
   S75 estimated 56). Hand-written
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

## Addendum (S79, 2026-10-30): the external signature as the generator emits it
Slice S79 added the `[external]` table and the recursion budget to the generator
(ADR-0019, S79 addendum). The status of this ADR is unchanged. Decision 3 stands, with one
amendment, made here because the generator cannot do what the decision's last sentences ask
of it, and two points the decision left open:
- **Who spends the budget.** Decision 3 says the budget is spent on entry into "a generated
  structure on such a cycle" as well as into Variant, DataValue, DiagnosticInfo and an
  ExtensionObject body. The generator cannot see which of its structures are on a cycle, as
  the cycle runs through hand-written types it does not read. It passes the budget on
  unchanged and spends nothing. That is enough: a cycle of generated types alone is still a
  generation error, so every cycle goes through an external type, and the external types
  spend. The hand-written Variant, DataValue, DiagnosticInfo and ExtensionObject envelope
  of S86 each spend one unit on entry, and the constant of 64 counts those entries (a
  Variant that holds an ExtensionObject whose generated body holds a Variant is two, not
  three). S86 measures the constant as decision 3 says.
- **The signature.** Each external type is an inherent `parse(reader: &mut BoundedReader<'_>,
  depth: Depth) -> Result<Self, ParseError>`, `serialize(&self, writer: &mut BoundedWriter,
  depth: Depth) -> Result<(), ParseError>` and `length_in_bytes(&self, depth: Depth) ->
  Result<usize, ParseError>`, the length taking the budget as well because the length of a
  Variant recurses. `Depth` is one `Copy` type that the options name; the generated code
  never looks inside it. The generated functions of every struct and `discriminatedType`
  take the same argument when the protocol has one (`parse(reader, depth, params...)`).
  Nothing else is passed: an external type that stands for an mspec type with parameters
  (`ExtensionObject(bit includeEncodingMask)`) is pinned to literal arguments by its entry,
  so the hand-written envelope is the masked one, and a use with other arguments is a
  generation error. The types derive nothing for the generated code but must be `Debug`,
  `Clone` and `PartialEq`; no generated type derives `Eq` beside externals.
- **Eq and floats.** A Variant holds a float, so the generated structures of the service
  layer are `PartialEq` only.

## Addendum (S85, 2026-10-30): the built-in types
Slice S85 added the hand-written scalar built-ins under `opcua/codec` (decision 2, decision 10),
crate-private and called by nothing until the recursive types (S86) and the generated layer
(S88). The status of this ADR is unchanged, and no public item and no `ParseError` variant is
added.
- **Types.** `UaString` and `UaByteString` (the `Ua` prefix keeps them apart from `String`),
  `Guid`, `DateTime`, `StatusCode`, `NodeId` with `Identifier`, `ExpandedNodeId`,
  `QualifiedName` and `LocalizedText`, each with the three functions of the S79 signature
  (`parse(reader, depth)`, `serialize(&self, writer, depth)`, `length_in_bytes(&self, depth)`).
  Their fields are `pub(in crate::opcua)`: they are plain data and the value layer (S92, S95,
  S98) reads them. The types are `pub(in crate::opcua)` in private modules because the
  workspace denies both `unreachable_pub` and `redundant_pub_crate`.
- **`Depth`.** The signature needs the budget type, so S85 defines `Depth` (one `Copy` value,
  `Depth::new()` full, 64 entries as decision 3 says) and nothing else: the built-ins have a
  fixed shape and ignore it. S86 adds the entry that spends one unit, `RecursionLimit` and the
  recursive types, and may change the representation, as the S79 addendum to ADR-0019 says.
- **Null and empty.** A String or ByteString of length -1 is null and is kept apart from the
  empty one (decision 4). A negative length other than -1 is `InvalidDiscriminator` with the
  field `String length` or `ByteString length` and the magnitude as the value, so it maps to
  `MalformedData` as decision 5 asks, without a new variant. A LocalizedText part that is
  absent (flag clear) is `None` and one that is present and null is `Some` of a null String;
  the same holds for an ExpandedNodeId namespace URI.
- **The length bound.** The signature carries no frame bound, so the codec bounds a String or
  ByteString by the bytes that remain (`read_bytes` fails with `Truncated` before any copy) and
  by one constant, `MAX_STRING_BYTES` of 16 MiB (`OversizedLength`), on read, write and
  `length_in_bytes` alike. The frame, message and chunk bounds of decision 5 stay with the
  framing and chunking slices. 16 MiB is a choice (above the 4 MiB message size most servers
  offer), not a measurement.
- **UTF-8.** A `UaString` holds the bytes it read (`Option<Vec<u8>>`) and `parse` never
  refuses them for being invalid UTF-8; `text()` answers `None` for null, `Some(Ok(&str))`
  for text and `Some(Err(Utf8Error))` for other bytes. A value therefore always writes back to
  the bytes it was read from, and no lossy `U+FFFD` text is made (CODING_STANDARDS 7.8). The
  reason is the per-item rule of `OPCUA-TYPE-STRING`: a String that is refused inside a
  Variant, a BrowseName or a String NodeId would fail the whole response, every item of a
  read included, so the value layer (S92) turns `Some(Err(_))` into the error of that one item
  and the rest of the response is read. (The first draft of S85 refused such bytes in the
  codec; the review found that it contradicted the row, and this is the correction.) Upstream
  decodes UTF-8 strings in `BaseStringEncoding.decodeString` (`new String(bytes, charset)`,
  lossy, `BaseStringEncoding.java:243`), cuts the text at the first NUL character, trims
  trailing spaces and removes U+FEFF (`:244-250`; read in this fix). Stonecrab does none of
  that: the bytes are kept verbatim, as a NUL or a trailing space is legal text on the wire.
  Divergence to record with the value mapping (S92): `OPCUA-TYPE-STRING` text keeps NUL,
  trailing spaces and U+FEFF. A ByteString holds any bytes.
- **Encodings.** A NodeId is read in any of the six encodings and written in the most compact
  one it fits (two-byte for namespace 0 and an identifier up to 255, four-byte up to namespace
  255 and identifier 65535, numeric otherwise), so a value read in a wider encoding than it
  needs is written back narrower and equal; bytes that must be kept verbatim (a signed or
  hashed region) are therefore never re-encoded by the caller. The top two bits of the first
  byte are reserved in a NodeId and the six upper bits of a LocalizedText mask are reserved:
  read and ignored, written as zero, as the generator treats a reserved field (ADR-0019, S79
  addendum). An ExpandedNodeId takes the two bits as the namespace-URI and server-index flags.
- **DateTime.** The codec keeps every `i64` (0 and `i64::MAX` are "not specified" and "end of
  time" in OPC UA, and negative ticks are before 1601). `to_system_time` and
  `from_system_time` compute in `i128` with checked operations and return `None` when the
  platform's `SystemTime` cannot hold the instant or the tick count does not fit an `i64`;
  `from_system_time` truncates to a whole tick towards 1970.
- **Vectors.** `fixtures/opcua/codec` holds 65 byte vectors (`protocol_spec` for the valid
  ones and `adversarial` for the 20 refusals and one String of bytes that are not UTF-8),
  first-party: written from the Part 6 encoding rules by a script that shares no code with
  the decoder, with expected files that name the value, or the `ParseError` variant, and
  `PROVENANCE.toml`. The `source` values are those of
  the loader (the row's `first_party` is not one of them, so the header comment of
  `PROVENANCE.toml` carries the first-party claim; every `license` is `Apache-2.0`). The worked examples of the specification and the revision named in every origin (1.05) are
  written from memory and marked UNVERIFIED there. The fixtures are not packaged. A test that reads them needs the
  `testkit` feature, like the Modbus equivalence tests; the unit tests cover every line
  without it.

Compliance: `crates/stonecrab/src/opcua/codec.rs` and `codec/` (`string.rs`, `guid.rs`,
`date_time.rs`, `status_code.rs`, `node_id.rs`, `name.rs`, and the test-only `support.rs`,
`vectors.rs`, `properties.rs`); tests `opcua_codec_null_string_is_distinct_from_empty`,
`opcua_codec_node_id_four_byte_encoding_round_trips`,
`opcua_codec_expanded_node_id_i321_is_01004101`,
`opcua_codec_date_time_zero_and_max_do_not_overflow`, `opcua_prop_builtin_round_trip` and
`opcua_codec_first_party_vectors_parse_and_reencode`; `mutants.toml` lists the module.

## Addendum (S86, 2026-10-30): the recursive and open types
Slice S86 added the hand-written recursive and open types under `opcua/codec` (decision 2,
decision 3, decision 5), crate-private and called by nothing until the generated service layer
(S88). The status of this ADR is unchanged. The one public change is the two `ParseError`
variants decision 5 names, additive on the `#[non_exhaustive]` enum.
- **`ParseError`.** `CountLimit { field: &'static str, count: usize, max: usize }` and
  `RecursionLimit { limit: u32 }`, with the fields the generator already emits (ADR-0019, S78
  addendum) and the stand-in of its fixtures already defines, so neither the generator nor the
  stand-in changed. `limit` is the number of nested entries that was allowed (64). Both map to
  `ErrorKind::ResourceLimit`, as decision 5 says; the only exhaustive match on `ParseError`
  in the crate, the Modbus framing's `frame_error`, maps both to `FrameError::Malformed`. ADR-0013
  decision 4 is amended in place. Semver against 0.3.0 stays green (default features and
  `--all-features`).
- **`Depth::enter`.** One unit is taken on entry into `Variant`, `DataValue`, `DiagnosticInfo`
  and `ExtensionObject`, in `parse`, `serialize` and `length_in_bytes` alike, before anything is
  read or written; at zero the result is `RecursionLimit { limit: 64 }`. The type that holds
  another passes the reduced budget on, so a `DataValue` that holds a Variant counts two, a
  Variant that holds a Variant two, and a chain of `DiagnosticInfo` inner causes one per link.
  64 is kept (decision 3): 63 holders and a Variant of nothing are read, 64 holders are
  refused (`opcua_codec_variant_nesting_at_the_limit_is_read`), a value built 65 deep is
  neither written nor measured, and 2000, 3000, 4000 and 16000 levels (`VariantNestingTest`)
  are `RecursionLimit` where upstream's unbounded parse ran out of stack. The stack check is
  `opcua_codec_variant_nesting_of_mixed_types_fits_a_small_stack`: Variant and `DataValue`
  alternating, and arrays of one Variant inside arrays of one Variant, each to the full
  budget and one past it on a 2 MiB thread in the test (dev) profile. Measured on one machine
  (macOS, aarch64, rustc 1.99.1, dev profile) with a thread of a chosen size and a 64-entry
  value: Variants and Variant-DataValue pairs parse in 256 KiB, an array chain needs between
  256 and 384 KiB and a chain of `DiagnosticInfo` parses in 384 KiB, so about 5 KiB for the
  deepest level and a margin of five to eight times on 2 MiB. UNVERIFIED on the other targets
  of the CI matrix (x86_64 and aarch64 Linux, armv7, Windows, whose frames and main-thread
  stacks differ); the test itself runs on all of them in the dev profile. The generated
  entries of S88 add frames between two re-entries (ADR-0019, S80 addendum), which this
  measurement does not include, so the constant stays 64 and S88 repeats the test with the
  generated dispatch.
- **Variant.** `Variant { value, dimensions }` with `VariantValue::{Null, Scalar, Array}`;
  `Scalar` and `Array` have one variant for each of the 25 types, so a value read as an array
  of one element is written as one (`OPCUA-DIV-ONE-ELEMENT-ARRAY` needs the flag; the value
  layer of S95 maps it). An array is `Option<Vec<T>>`: -1 is `None`, 0 is `Some(vec![])`. A
  type id above 25 is `InvalidDiscriminator` (field `VariantType`). A Boolean reads any
  non-zero byte as true and writes 1. A Variant of type 0 holds nothing and ignores the array
  flag, as upstream's mspec does; a dimensions flag on it is honoured and the product must
  then be 0. The recursive element types are boxed in `Scalar` (`Box<Variant>`,
  `Box<DataValue>`) and held directly in `Array`.
- **Counts.** An array count is read as an `Int32`, a negative count other than -1 is
  `InvalidDiscriminator` (field `arrayLength`, the magnitude), and a count larger than the
  bytes that remain can hold is `CountLimit` before an element is read: the bytes divided by
  the fewest bytes an element takes (its size for Boolean to Double, DateTime, Guid and
  StatusCode, one for every other type). The elements are pushed one at a time, so the
  reservation is bounded by the elements actually parsed
  (`opcua_codec_variant_array_count_larger_than_the_body_fails_before_allocation`, 2^31 - 1
  refused for all 25 types). That is bounded by the input, but not by it alone: a Variant
  array of Variants takes one byte per null element on the wire and
  `size_of::<Variant>()` of 128 bytes in memory (176 bytes for a `DataValue` that is one byte
  on the wire, 104 for an `ExtensionObject` of at least three), measured on the same machine,
  so an array count equal to the remaining bytes costs about 130 to 180 times the bytes in
  memory. The frame and message bounds of decision 5 (S87, S94) are what limit that product;
  this slice does not add a second one. `read_array` is the shared helper a structure field
  array (the `StructFieldArrayCountTest` shape) uses; S86 tests that shape itself, with the
  four upstream vectors (`opcua_codec_struct_field_array_count_larger_than_the_body_is_refused`).
  Upstream turns a count of -1 into an empty list where `read_array` returns `None`, and the
  value mapping of S119 decides what a null field is.
- **Dimensions.** The product of `arrayDimensions` is computed with `checked_mul` and must
  equal the number of values (0 for nothing and for a null array, 1 for a scalar). A
  mismatch, an empty or null list or a negative dimension is `InvalidDiscriminator` (field
  `arrayDimensions`), an overflowing product is `Overflow`, and a dimension count that the
  remaining bytes cannot hold is `CountLimit` (field `arrayDimensions`). The check also runs
  before `serialize` and `length_in_bytes`, so no inconsistent Variant is written. Both are
  `MalformedData` or `ResourceLimit` as decision 5 asks. A dimension of 0 is accepted with an
  empty array. UNVERIFIED against Part 6: whether a dimensions flag is legal on a scalar or
  null Variant, and whether a dimension may be 0; the codec accepts what is consistent.
- **DataValue.** The mask (value 0x01, status 0x02, source timestamp 0x04, server timestamp
  0x08, source picoseconds 0x10, server picoseconds 0x20) selects the parts, which follow in the
  order value, status, source timestamp, source picoseconds, server timestamp, server
  picoseconds; the two upper bits are read and ignored and written as zero. The picoseconds are
  kept as read and not checked against the 10,000 of a tick.
- **DiagnosticInfo.** Mask bits: symbolic id 0x01, namespace URI 0x02, localized text 0x04,
  locale 0x08, additional info 0x10, inner status code 0x20, inner `DiagnosticInfo` 0x40, bit 7
  reserved. The four indices are `Int32`s kept as read. The wire order is upstream's mspec
  (symbolic id, namespace URI, locale, localized text, additional info, inner status, inner
  info). UNVERIFIED: that the bit order and the field order differ for locale and localized text
  is from the mspec and from memory of Part 6; check the order against open62541 v1.5.9 in the
  interop run of S92 (a swap changes which index is which, not the length).
- **ExtensionObject envelope.** `ExtensionObject { type_id: ExpandedNodeId, body }` with
  `ExtensionBody::{Null, Raw(UaByteString), Xml(UaString)}`: the type id, an encoding byte
  (0, 1 or 2) and for 1 and 2 a length-prefixed body, kept as received (a null body stays null,
  an XML body is not checked for UTF-8). The type id is an `ExpandedNodeId`, as upstream's
  mspec has it, which reads the plain `NodeId` of Part 6 and also a server that sets the flag
  bits. An encoding byte other than 0, 1 and 2 is `InvalidDiscriminator` (field
  `ExtensionObject encoding`): upstream's mspec reads the byte as flags (binary 0x01, XML 0x02,
  a type-id-specified bit 0x04) and ignores the rest, which Stonecrab does not; UNVERIFIED
  that a server never sets them. Upstream reads no XML body (an XML object is a null one and
  the bytes are left in the stream); this envelope reads it. The Known body of a standard type
  is added by S88 as a third variant; a disabled case maps `InvalidDiscriminator` to `Raw`.
  The body length is bounded by `read_bytes` against the bytes that remain and by
  `MAX_STRING_BYTES` before any copy.
- **Fuzz.** `fuzz_opcua_decode` (35 seeds under `fuzz/corpus/fuzz_opcua_decode`, reached through
  `stonecrab::opcua::fuzzing`, which exists only under `--cfg fuzzing`): the first byte picks
  one of the four types and the rest is parsed; what parses must measure as it writes, write
  in no more bytes than were consumed, and parse back to a value that writes the same bytes
  (values are not compared, as a float may be NaN). The known-body dispatch target of decision 5
  waits for S88.
- **Vectors.** The wire vectors are written out in the unit tests (hex from the Part 6 rules,
  not produced by the encoder) rather than as files under `fixtures/opcua/codec`, which stay
  the S85 built-ins; the upstream corpus of S93 will cover these types from outside. This is a
  departure from CODING_STANDARDS 13.6 for this slice, taken because the expected values of
  these types are trees and the fixture loader reads flat ones. Property tests:
  `opcua_prop_recursive_types_round_trip`,
  `opcua_prop_recursive_types_parse_never_panics_and_is_stable` and
  `opcua_prop_recursive_types_survive_a_cut_or_flipped_byte`.
- **Coverage note for later slices.** `scripts/coverage-gate.sh` sums the line counts of the
  function groups that llvm-cov reports, and a group's covered count is the best single
  instantiation's: a generic function or a macro expansion of which no one instantiation
  executes every line counts the lines it misses even if the others execute them. A
  `match` on a type-dependent value with an arm for each case therefore costs a line;
  `read_array` computes the bound in one expression for that reason. The same applies to a
  `matches!` that is never false on a line of its own.

Compliance: `crates/stonecrab/src/codec/error.rs`; `crates/stonecrab/src/opcua/codec.rs`
(`Depth::enter`) and `codec/` (`variant.rs`, `data_value.rs`, `diagnostic_info.rs`,
`extension_object.rs`, and the test-only `support.rs` and `properties.rs`); tests
`opcua_codec_variant_nesting_beyond_the_limit_is_recursion_limit`,
`opcua_codec_variant_array_count_larger_than_the_body_fails_before_allocation`,
`opcua_codec_unknown_extension_object_body_is_kept_raw`,
`opcua_codec_struct_field_array_count_larger_than_the_body_is_refused`,
`opcua_codec_variant_dimensions_inconsistent_with_the_count_are_refused` and
`plc_error_mapping_is_malformed_data_except_the_three_limits`;
`fuzz/fuzz_targets/fuzz_opcua_decode.rs`; `mutants.toml` lists the files.
