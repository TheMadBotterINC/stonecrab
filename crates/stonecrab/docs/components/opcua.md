# Stonecrab OPC UA
The OPC UA client of Stonecrab (`stonecrab::opcua`), behind the opt-in `opcua` feature. It is
in neither `default` nor `full`, as `tls` is (decision E17, ADR-0034), and it needs the
`transport` feature. The module is being built in slices (milestone M3, ADR-0034). It currently has the
address grammar, public `OpcuaDriver`, binary codecs, generated services, TCP framing,
None and the three modern RSA policies for channel/session setup and a bounded request lane. End-to-end scalar Read is available since S92; S95 adds extended values, arrays and matrices.
The 23 pinned upstream vectors introduced by S93 now have 17 byte-exact round-trips,
one existing numeric `NodeId` canonicalization and five exact disabled-service refusals
(S100 enables the two write bodies).
The notification envelope round-trip preserves an opaque body; full service/type support
remains its later slices. The independent XML adapter and fixtures are repository-only;
no runtime behavior changes in S93.
S94 adds bounded MSG splitting/reassembly under the existing request lane. A large request
is preflighted before any bytes are sent, C/F chunks keep their request id and advance
sequence numbers, and a peer ABORT fails only its request with the exact status. Partial
responses survive interrupted receives; dropping a request frees its partial body.

This module offers `OpcuaAddress`, a validated `ProtocolAddress` for the tag text
`ns=<n>;<i|s|g|b>=<id>[range][;a=<attribute>][;TYPE][|key=value,...]`, with the types its
accessors return: `OpcuaIdentifier`, `OpcuaAttribute` and `OpcuaDataType`. Parsing opens no
socket and starts no discovery.

| Capability | Support level |
| --- | --- |
| Tag address parse and canonical text (`OpcuaAddress`) | build-tested |
| Browse query parse (`OpcuaAddress::parse_query`) | build-tested |
| Connection string and driver-owned options (`OpcuaDriver`) | build-tested |
| Built-in types of the binary encoding: String, `ByteString`, Guid, `DateTime`, `StatusCode`, `NodeId`, `ExpandedNodeId`, `QualifiedName`, `LocalizedText` (crate-private) | build-tested |
| Recursive and open types of the binary encoding: Variant (25 types, arrays, dimensions), `DataValue`, `DiagnosticInfo`, the `ExtensionObject` envelope with Raw, Xml and Null bodies, bounded in depth and in array counts (crate-private) | build-tested |
| TCP framing: the 8-byte header, HEL, ACK, ERR, OPN, CLO and MSG bodies, the asymmetric header of policy None, the symmetric and sequence headers, and a bounded chunk `Framer` (crate-private) | build-tested |
| Generated 29-case connection/read/write/discovery/identity service bodies and 274 status names; Known `ExtensionObject` dispatch (crate-private) | build-tested |
| Bounded service exchange lane: correlation, sender priority, headers, sequence/channel/token checks, ERR and `ServiceFault` (crate-private) | build-tested |
| Policy-None HEL/ACK, OPN Issue, CLO and counter handoff (crate-private) | interoperability-tested with open62541 |
| Public driver and anonymous None connection/session | interoperability-tested with open62541 |
| Read (basic scalars, UTC `DateTime`, Guid/`NodeId`/`QualifiedName`/`StatusCode` text, arrays and matrices) | interoperability-tested with open62541 |
| Staged scalar, array and matrix writes with explicit read-write access | interoperability-tested with open62541 |
| Browse, subscriptions | planned |
| Bounded PEM client key loading (RSA PKCS#8), certificate chain/thumbprint and SAN application URI | build-tested |
| Asymmetric OPN signing/encryption and remote-key binding (crate-private) | interoperability-tested with open62541 for all three modern policies |
| Server trust preparation: explicit opt-out, PEM/DER store, constant-time pin and fail-closed verifier (crate-private) | build-tested |
| All three modern policies in SIGN/`SIGN_ENCRYPT`, secure discovery and session certificate proofs | interoperability-tested with open62541 |
| Protected user tokens and renewal | planned |
| Independent-server scalar Read interoperability | verified with open62541 |
| Hardware verification | planned |

Build-tested here means unit tests, a property test and a fuzz target on the address text, a
fuzz target on the connection string and, for the built-in types, first-party byte vectors
written from OPC UA Part 6 (`fixtures/opcua/codec`) with a property test. The recursive types
have hand-written byte vectors in their unit tests, property tests and the fuzz target
`fuzz_opcua_decode`; their nesting and array-count bounds are tested at the limit and beyond it.
The framing has hand-written byte vectors for each message type, property tests and the fuzz
target `fuzz_opcua_frame`; the size of a message is tested at the receive buffer and one byte
past it.
No parity claim above `planned` is made for the rest of the OPC UA record,
[`docs/parity/opcua.md`](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/opcua.md);
the rows marked `implemented` there (the `OPCUA-ADDR-*` rows, `OPCUA-QUIRK-BINARY-ID`, the
divergences `OPCUA-DIV-NAMESPACE-RANGE`, `OPCUA-DIV-BINARY-ID` and
`OPCUA-DIV-ADDRESS-ESCAPES`, and the connection string rows `OPCUA-CFG-SECURITY-POLICY`,
`OPCUA-CFG-MESSAGE-SECURITY`, `OPCUA-CFG-FRAMEWORK`, `OPCUA-CFG-UNKNOWN`,
`OPCUA-DIV-UNKNOWN-KEYS`, `OPCUA-TRANSPORT-TCP`, `OPCUA-TRANSPORT-ENDPOINT-URL`,
`OPCUA-TRANSPORT-PORT`, `OPCUA-QUIRK-INET-PATTERN` and `OPCUA-QUIRK-TLS-NAMES`, and the
framing rows `OPCUA-WIRE-HEADER`, `OPCUA-WIRE-MESSAGE-TYPES`, `OPCUA-WIRE-OPEN-HEADER` and
`OPCUA-QUIRK-CODEC-RHE`) are the ones that have a claim.

## Address syntax
```rust
use stonecrab::opcua::{OpcuaAddress, OpcuaAttribute, OpcuaDataType, OpcuaIdentifier};
use stonecrab::ProtocolAddress;

let address = OpcuaAddress::parse("ns=2;s=samples[3..8;1];a=Value;real")?;
assert_eq!(address.namespace(), 2);
assert_eq!(address.identifier(), &OpcuaIdentifier::String("samples".to_owned()));
assert_eq!(address.attribute(), OpcuaAttribute::Value);
assert_eq!(address.data_type(), OpcuaDataType::Real);
assert_eq!(address.index_range().as_deref(), Some("2:7"));
assert_eq!(address.canonical(), "ns=2;s=samples[3..8;1];REAL");
# Ok::<(), stonecrab::PlcError>(())
```

- `ns=<n>`: the namespace index, plain decimal digits from 0 to 65535, the 16 bits the node id
  carries. A sign, a space or 65536 is `InvalidAddress`. The `ns=` group is required.
- The identifier type letter: `i` is a number from 0 to 4294967295, always the full numeric
  node id; `s` is text; `g` is a GUID written 8-4-4-4-12, kept as the 16 bytes of the OPC UA
  binary encoding (the first three groups little-endian); `b` is a `ByteString` written in
  base64 with padding, in its own namespace, as the standard node-id text writes it (not
  upstream's two-byte numeric id; the bits before the pad must be zero, so each byte string
  has one text). An empty identifier is `InvalidAddress`.
- The identifier ends at the first `;` or `|` that is outside brackets and not escaped. A
  backslash before `\`, `;`, `[` or `|` makes that character part of the identifier; the
  canonical text always writes those four escaped. A bracketed segment may hold `;`.
- An array suffix: one or more bracket groups at the very end of the identifier, each
  `[index]`, `[lower..upper]` or either with `;base`, several dimensions in one group
  separated by commas. It becomes an OPC UA `IndexRange`, 0-based and inclusive, one entry per
  dimension (`[3..8;1]` is `2:7`, `[4]` is `4`, `[1..2][3..4]` is `1:2,3:4`). Reversed
  bounds, an index below its base and indexes above 2147483647 are `InvalidAddress`.
  Brackets that are not such groups stay in a string identifier.
- `;a=<attribute>`: a standard attribute by name (case-sensitive) or number, 1 to 27; the
  default is `Value`. An unknown one is `InvalidAddress`.
- `;TYPE`: one of the 24 names `NULL`, `BOOL`, `BYTE`, `SINT`, `INT`, `DINT`, `LINT`,
  `USINT`, `UINT`, `UDINT`, `ULINT`, `REAL`, `LREAL`, `TIME`, `LTIME`, `DATE`, `LDATE`,
  `TIME_OF_DAY`, `LTIME_OF_DAY`, `DATE_AND_TIME`, `LDATE_AND_TIME`, `CHAR`, `WCHAR` and
  `STRING`, in any case. `WORD`, `DWORD`, `LWORD` and `WSTRING` cannot be named. Without a
  suffix the type is `NULL`: use the type the server declares.
- `|key=value,...`: the key-value tail that carries the field names of an event
  subscription, in written order. Keys are letters, `-` and `_`; values add digits.
- Control characters and text over 8192 bytes are `InvalidAddress`, and so is text whose
  canonical form is over 8192 bytes (escapes and a spelled-out attribute make it longer).
  Messages echo at most 32 characters of the input.

`canonical()` and `Display` write the same grammar with the defaults left out, and the
canonical text parses back to an equal address.

## Browse queries
```rust
use stonecrab::opcua::{OpcuaAddress, OpcuaIdentifier};

let objects = OpcuaAddress::parse_query("**")?;
assert_eq!(objects.identifier(), &OpcuaIdentifier::Numeric(85));
assert_eq!(OpcuaAddress::parse_query("   ")?, objects);
let start = OpcuaAddress::parse_query("ns=2;s=plant")?;
assert_eq!(start.namespace(), 2);
# Ok::<(), stonecrab::PlcError>(())
```
A browse query is the address of the node to start from; an empty or blank query, or `**`,
means the Objects folder, `ns=0;i=85`. Browse itself is not implemented yet.

## Connection string
The string the driver will take, parsed since slice S84 but not yet reachable: the parser is
crate-private until the driver arrives.

```text
opcua://host:port[/path][?security-policy=NAME][&message-security=NAME]
opcua:tcp://[2001:db8::10]:4840/ua/server
```
- The scheme is `opcua` and the transport code is `tcp`, implicit when omitted. Any other
  code, `tls` and `https` included, is `InvalidConfig`.
- The port is required: there is no default and 4840 is not assumed. It is decimal digits,
  1 to 65535. The host is a name, an IPv4 address or an IPv6 address in brackets.
- The path, everything from the first `/`, is kept as written and becomes the endpoint URL
  `opc.tcp://host:port<path>` that Hello carries (at most 4096 bytes).
- `security-policy` is `NONE`, `Basic256Sha256` (the default), `Aes128_Sha256_RsaOaep` or
  `Aes256_Sha256_RsaPss`, case-sensitive. `Basic128Rsa15` and `Basic256` are recognised and
  refused as deprecated SHA-1 policies.
- `message-security` is `NONE`, `SIGN` or `SIGN_ENCRYPT` (the default), case-sensitive. A
  secure policy with `NONE` is `InvalidConfig`; with policy `NONE` the mode is kept as
  configured and the channel will ignore it.
- `request-timeout-ms` is a positive decimal millisecond count, 30000 by default. It sets
  `Timeouts.request` and the header timeout hint; a shorter caller deadline takes precedence.
- `encoding.receive-buffer-size` and `encoding.send-buffer-size` are decimal 8192 through
  2147483647, each defaulting to 65535. Both are capped by the local frame budget.
- `encoding.max-message-size` and `encoding.max-chunk-count` are positive decimal `UInt32`
  values, defaulting to 2097152 body bytes and 64 chunks. Hello caps message bytes to the
  local receive budget (262144 by default); negotiated limits apply before allocation and
  on send. Aggregate partial responses share that local receive budget. A peer zero never
  removes local bounds. Message bytes exclude chunk/security/sequence headers.
  `ResourceLimits` caps it before Hello, and ACK may reduce it further.
- Every other key is `InvalidConfig`, and so is a key written twice. That includes the keys
  the framework fills (`protocol-code`, `transport-code`, `transport-config`), the
  credentials a connection string must not carry (`username`, `password`; they arrive in
  `Credentials`) and the `tls.*` certificate options, which later slices implement.
- A message names the key (shortened to 32 characters) or a fixed reason; it never repeats a
  value, the host or the path.

## Built-in types
Crate-private since slice S85: the hand-written scalar types of the
OPC UA binary encoding (Part 6, 5.2.2) that the recursive types (S86) and the generated service
layer (S88) are built on. All are little-endian and go through `BoundedReader` and
`BoundedWriter`, and each has the one signature the generator calls external types through
(`parse(reader, depth)`, `serialize(&self, writer, depth)` and `length_in_bytes(&self, depth)`).

- A String is the bytes of UTF-8 text and a `ByteString` is bytes, each with an `Int32`
  length. The length -1 is null and is kept apart from the empty value; any other negative
  length is refused. A length is checked against the bytes that remain and against 16 MiB
  before anything is copied, so a declared length never reserves memory. A String keeps the
  bytes it read, so bytes that are not UTF-8 are not refused here (`text()` says whether they
  are text) and are written back unchanged; the value layer turns them into an error of that
  one item.
- A Guid is `UInt32`, `UInt16`, `UInt16` and eight bytes. A `DateTime` is an `Int64` of 100 ns
  ticks since 1601-01-01 and keeps every value, `0` and `i64::MAX` included; its conversions
  to and from `SystemTime` return `None` instead of overflowing.
- A `NodeId` is read in any of its six encodings (two-byte, four-byte, numeric, String, Guid and
  `ByteString` identifiers) and written in the most compact one. The top two bits of the first
  byte are reserved in a `NodeId` and ignored; an `ExpandedNodeId` uses them to say that a
  namespace URI and a server index follow.
- A `LocalizedText` has a mask byte; a part that is absent is not a part that is a null String.
  The six upper bits of the mask are reserved and ignored.

## Recursive types
Crate-private since slice S86: the Variant, `DataValue`,
`DiagnosticInfo` and the `ExtensionObject` envelope of the binary encoding. They have the same
three-function signature as the built-ins, and the first three are the ones a server can nest.

- Variant, `DataValue`, `DiagnosticInfo` and the envelope each spend one of 64 entries of a
  depth budget on entry, in `parse`, `serialize` and `length_in_bytes`. When none is left the
  result is `ParseError::RecursionLimit { limit: 64 }`, so a chain of Variants inside Variants
  (a Variant of type 24, or an array of them) is refused at the 65th and the stack cannot
  overflow. The limit is far above the 29 contexts of the deepest message in the upstream
  suites.
- A Variant has all 25 types, as a scalar or an array. A null array is kept apart from an empty
  one, and an array of one element keeps its array flag. A type id above 25 is refused.
- An array count is an `Int32`; -1 is null and any other negative count is refused. A count
  larger than the remaining bytes divided by the fewest bytes an element takes is
  `ParseError::CountLimit { field, count, max }` before any element is read, and the elements
  are pushed one at a time, so a declared count never reserves memory. An array of Variants
  or `DataValue`s is still larger in memory than on the wire (one byte per null element
  against 128 or 176 bytes), which the frame and message limits of the connection bound.
- `arrayDimensions` is checked: the product of the dimensions, computed without overflow,
  must equal the number of values, and a mismatch is refused on read and on write.
- A `DataValue` and a `DiagnosticInfo` have a mask byte that selects their parts; the upper
  bits that Part 6 reserves are ignored on read and written as zero.
- An `ExtensionObject` keeps its type id and its body as received: Raw (a `ByteString`), Xml
  (a String) or Null. An encoding byte other than 0, 1 and 2 is refused. The bodies of the
  standard types are recognized through the generated layer since S88.

Not checked against a server yet (UNVERIFIED, see the ADR-0034 addendum of S86): the order of
the locale and the localized text in a `DiagnosticInfo`, a dimensions flag on a scalar, and
whether servers set the unused bits of the `ExtensionObject` encoding byte.

## Framing
Crate-private since slice S87 and called by nothing yet: the OPC UA TCP framing (Part 6, 6.7
and 7.1) that the secure channel of slice S90 will drive through a `FramedTransport`.

- Every message starts with an 8-byte header: a three-character type (`HEL`, `ACK`, `ERR`,
  `OPN`, `CLO` or `MSG`), a chunk flag (`C`, `F` or `A`) and the little-endian `UInt32` size of
  the message, the header included. An unknown type (RHE too) or flag is refused when the
  header is read.
- The framer compares the size with the receive buffer that Hello declares and with
  `ResourceLimits::max_frame_bytes`, the smaller of the two, as soon as the 8 bytes are in,
  before it waits for or copies the body. A size below 8, or above `Int32::MAX` (negative to
  upstream's Java reader, which reads the size unchecked), is `MalformedData`; a size over the
  limit is `ResourceLimit`; both leave the buffer untouched.
- Hello carries the protocol version, the four limits (receive buffer, send buffer, maximum
  message size, maximum chunk count) and the endpoint URL; Acknowledge the version and the
  limits; Error a status code and a reason. `OpenSecureChannel` carries the secure channel id,
  the asymmetric header (the policy URI, the sender certificate and the receiver certificate
  thumbprint: 59 bytes for the policy None), the sequence header and the payload.
  `CloseSecureChannel` and Message carry the channel id, the token id, the sequence header and the
  payload. The payload stays bytes: the generated service layer reads it.
- The framer reads whole chunks and keeps the chunk flag; joining the chunks of one message
  is owned by the lane since S94. It writes a chunk back as the bytes it read, so the
  framer keeps no reassembly state.

## Generated service bodies
Since S88, the uniform service layer is generated from the three pinned PLC4X models
under `upstream/mspec/opcua/`. The production output is private to this crate. There are
22 enabled cases covering channel/session setup and Read, with first-party positive and
negative vectors under `fixtures/opcua/services`, deterministic regeneration and the
`fuzz_opcua_service` target. Generated code has vector coverage; all handwritten OPC UA
code keeps its 100% line coverage floor.

A local namespace-zero binary `ExtensionObject` with a recognized encoding id becomes a
Known body, bounded by its declared byte slice and the shared 64-entry recursion budget.
Disabled, unknown and custom identifiers retain Raw bytes. Invalid fields in a recognized
body and trailing bytes fail; they cannot silently become Raw. Known-body serialization
and length measurement use a bounded temporary encoding of at most 16 MiB. Private
constructors must pair each Known body with its matching encoding id.

`StatusCode` keeps its exact number and can name the 274 generated known values; an
unrecognized number has no invented name. This adds codec support only. Network services
and independent-server evidence remain planned for later slices.

## Service exchange lane
Since S89, an established policy-None channel can exchange services through a private,
bounded request lane above `FramedTransport`. Any waiting caller receives for every
request id; a queued sender interrupts the cancel-safe receive and gets transport
ownership before another read starts. No task runs between calls. `ResourceLimits` bounds
pending requests, including queued and completed entries until consumed, and the shared
framer/transport minimum bounds encoded requests before a sequence number is consumed.

Request headers carry the session token, injected wall-clock timestamp, request handle
and remaining timeout hint. Replies must use the expected channel/token and consecutive
nonzero sequence numbers. Unknown request ids are discarded and counted. Wrap is refused
explicitly; independent wrap behavior remains UNVERIFIED. ERR fails all pending requests immediately;
a `ServiceFault` fails only its request with the exact status number and known name in
a typed source. Server reason text is not echoed. No service is replayed.

Dropping or cancelling a receive releases its slot and retains partial frame bytes for
other callers. Interrupting a send can leave partial bytes on the wire and fails the
whole lane. Deterministic tests exercise both outcomes, out-of-order replies, simultaneous
waiters, sender priority, reentrant wake-ups, resource bounds and fake-clock deadlines.
S90 supplies Hello/channel negotiation; S91 adds public connect and anonymous sessions.
Scalar Read is available since S92.

## Policy-None channel
Since S90, the private channel owner exchanges Hello/Acknowledge, sends `OpenSecureChannel`
Issue and retains the exact server token, revised lifetime and nonce. Policy None uses
mode None for every configured mode, with a null client nonce and no certificate material,
signature or encryption. Selecting this path requires policy None explicitly. Configured
and effective mode appear in channel Debug output; nonce/certificate bytes do not.

Client receive merges with server send, and client send with server receive. Buffers are
at least 8192 bytes; a peer message-size or chunk-count zero imposes no additional peer
limit, while local limits remain finite. Negotiation keeps
buffered bytes and the framer enforces each direction separately. Message-body bounds
apply before service decoding/encoding. The current requested channel lifetime is
3600000 ms; configurable lifetime and renewal arrive in S109. One caller deadline and
cancellation token bound the handshake, including silent peers. `handshake-timeout-ms`
maps to the whole connect bound since S91.

Handoff preserves request/sequence counters and negotiated bounds. The lane sends
`CloseSecureChannel` without waiting for a reply. Six exact-count ignored tests against
open62541 v1.5.9 prove channel counter continuation, public session connect/close, scalar Read
and large array/Read-batch chunking under 8192-byte buffers. S95 also verifies extended public scalar/array/matrix mapping.
Evidence is recorded by `interop/open62541/interop.sh tests` and CI.


## Public connect and anonymous session

Register `Arc::new(OpcuaDriver::new())` with `DriverRegistry`, then connect to
`opcua://host:4840/path?security-policy=NONE` for an explicit unprotected session. The port
is required. `Basic256Sha256` is the secure default and requires the provisioned client key
and server certificate described below. Read and staged write are available; writes need
read-write access. Ping, browse and subscriptions remain unsupported.

TCP, HEL/ACK, OPN Issue, `CreateSession` and anonymous `ActivateSession` share one
`handshake-timeout-ms` deadline (60000 by default, exposed as `Timeouts.connect`). The
`CreateSession` request carries a fresh 32-byte OS nonce and `session-timeout-ms` (120000
by default). The returned authentication token and valid revised timeout are retained.
Activation uses the advertised anonymous policy id on a None/None UA TCP endpoint;
S96 selects by parsed URL, exact policy URI/effective mode, TCP profile and compatible
user token type. Highest security level wins, with advertised token protection as the
tie-break. The sanitized selected URL appears in connection Debug. Connection becomes Connected
only after successful activation. `with_clock` and `with_transport_factory` follow the
other drivers' deterministic injection pattern.

Close sends `CloseSession` (`deleteSubscriptions=true`, timeoutHint 50000), then CLO and
transport shutdown without a CLO response wait. The whole sequence is bounded by
`request-timeout-ms` (30000); timeout, failure or dropping close leaves Closed and wakes
state observers. Concurrent and repeated closes are bounded/idempotent. Dropping the
connection releases its transport and publishes Closed. Reads are sent once by default;
explicit `RetryPolicy` behavior is described below. The
independent peer test proves a full anonymous None session using the public registry API;
this supplies no evidence for secure policies, renewal or hardware.


## Read values

`Connection::read` sends one service request for all tags, with `maxAge=0` and both
source/server timestamps requested. Results remain in tag order; a result-count mismatch
is `MalformedData` for the whole call. Any standard attribute id and `IndexRange` are sent
as configured. Scalars map by their wire type: Boolean, eight integer widths/signs,
Float/Double bit patterns, UTF-8 String/XML, and Null. Invalid UTF-8 fails only its item;
strings keep NUL, whitespace and BOM. S95 maps `DateTime` directly to UTC `PlcDateTime`
fields with 100 ns precision across the full wire range, including the 1601 epoch and
negative ticks, independently of host clock range. Guid is canonical lower-case text;
`NodeId` text reuses address escaping/base64 and parses back; `QualifiedName` is
`ns=<n>;s=<name>` (null name is Null); `StatusCode` text carries its name and hex code.
S98 attaches source and server times independently when supplied and convertible, on
successful values and item errors. Missing or zero timestamps stay None. Picosecond
refinements are clamped and truncated to whole nanoseconds; no local/epoch substitute.

Arrays of supported types become Lists, retaining one-element, empty and null distinctions;
matrices become nested row-major Lists; zero-length axes produce empty Lists. Dimension/count mismatch
fails only the item. Rank is at most 64, all items share 1048576 value/container nodes,
and an empty matrix creates at most 64 Lists. Bounds are checked before building the tree;
failed mappings leave the remaining node budget available to other items.

The eight temporal suffixes apply per leaf to exact integer counts. `TIME` is milliseconds,
`LTIME` nanoseconds; `DATE` counts days since 1990, `LDATE` and `DATE_AND_TIME` seconds since
1990, and `LDATE_AND_TIME` milliseconds since 1990. Both `TIME_OF_DAY` and `LTIME_OF_DAY`
use milliseconds since midnight, matching the pinned upstream constructors. Null and
already temporal values remain unchanged. Float/Boolean/text counts, integer wrapping and
calendar/duration overflow are refused as per-item `InvalidDataType`; other suffixes leave
the actual server type unchanged. No numeric truncation is performed.

S98 maps `ByteString` and `LocalizedText`. `ExpandedNodeId`, `DataValue`, Variant,
`DiagnosticInfo` and custom `ExtensionObject` values remain named per-item Unsupported;
this includes null/empty arrays of deferred kinds. They do not fail mapped batch items.

`BadNodeIdUnknown` maps to `NotFound`, `BadTypeMismatch` to `InvalidDataType`, `BadNotWritable`
and `BadUserAccessDenied` to `AccessDenied`, and `BadAttributeIdInvalid`/`BadIndexRangeNoData`
to `InvalidAddress`. Unknown failure codes map to `Internal`; other known failures map
to `DeviceException`. Every item retains its exact status code and known symbolic name.
An Uncertain value succeeds with its status attached; an Uncertain item without a value
fails. Good absent/Null values and null strings map to Null.

A read uses the earlier of its explicit deadline and the per-attempt request timeout.
The default is one attempt; an explicit `RetryPolicy` permits bounded safe reads while the
same lane is usable and the explicit deadline has time left. Writes are not replayed and
reconnect remains S110. Cancelling/dropping receives preserves partial buffered bytes;
interrupted sends fail the lane. Close cancels pending reads with Disconnected, and a
late read failure cannot replace Closing/Closed. An unpolled read may hold its transport
lease until resumed/dropped; the close itself still has its request-timeout bound.

The 33-case shared contract suite passes 23 cases with 10 named skips for unsupported
write, supported-ping and polling-subscription behavior. The independent peer verifies
18 scalar types, arrays, 2D/3D matrices and large arrays plus named deferred-kind errors
in one batch. It also checks an unknown node and runs the finite
`opcua_read` example against `ns=1;s=scalar/Int32`.

## Endpoint selection and discovery primitives (S96)

`endpoint-host` and `endpoint-port` (1..=65535) add an alternative advertised host/port
match; the original pair remains accepted. Neither option changes the TCP destination.
The path must match exactly, including case and trailing slash. DNS names compare without
case and equivalent IPv6 spellings compare as addresses. Port 4840 never matches 48400.
A selection failure names these options and lists at most 16 sanitized endpoint URLs,
each bounded to 256 characters; invalid or credential-bearing URLs are replaced.

`discovery` is a strict boolean, default true. With a secure policy, `discovery=false`
requires `server-certificate-file`, as does discovery=true. Bounded files and trust
validation are prepared before I/O. Every modern policy runs `GetEndpoints` over the secure
channel when discovery is enabled, verifies its certificate and reuses the channel. None connect uses
`CreateSession`'s endpoint list and skips `GetEndpoints`. An explicit private probe against
open62541 verifies all seven advertised policy/mode combinations and token types; it
makes no certificate-trust or secure-channel claim.

## Byte strings, localized text and metadata (S98)

`ByteString` maps to `PlcValue::Bytes`, without text conversion or cloning. Null is Null,
empty is empty Bytes. `LocalizedText` maps to `PlcValue::Struct` with exactly `text` then
`locale` fields; missing/null fields are Null, explicit empty fields empty String.
Delimiters remain ordinary text. Arrays/matrices keep these values and their shape;
invalid UTF-8 fails only that item. Each localized value uses three shared budget nodes,
and its structure counts toward the 64-composite-level bound before tree construction.

Quality remains the exact item `ProtocolStatus` code/name. Good and value-bearing
Uncertain items keep their decoded value; Bad remains an item error even when the server
supplies a value. Both source and server timestamps stay attached on either outcome.
They are `SystemTime`, separate from monotonic `sampled_at`; absent/zero/unrepresentable
times remain None. 10 ps refinements clamp at 9999 then truncate to whole ns; without
a timestamp they are ignored. No native subscription or custom structure mapping is added.

## Credential channel and plain username sessions (S99)

Use `DriverRegistry::connect_with(uri, Credentials::new(username, password))`, or
`ConnectionCache::builder(...).credential_provider(...)`, to supply credentials separately.
URI username/password keys remain invalid. One cache uses one identity policy; remove or
close its sessions before rotating credentials. Provider callbacks run outside locks only
on new opens, must be nonblocking, and must return redacted errors; None retains the
parsed driver's config, and failure never retries as anonymous.

Username sessions on policy None require `allow-insecure-credentials=true`, default false.
The driver emits a static warning and sends plain UTF-8 password bytes only for effective
token policy None. Selected protected policies are refused without downgrade. The device's
identity/access refusals map to Authentication with the source retained. Missing credentials
or an empty pair selects anonymous; password without username is invalid before I/O.
`Credentials::x509` represents certificate/key paths without loading them; certificate
activation and protected username tokens are supported in S108 (see below). Eight native cases pass against
open62541, including correct/wrong-password username sessions; no hardware claim.

## Staged writes (S100)

Enable `access=read-write` in the connection URI, or use a config built with
`AccessPolicy::ReadWrite`. Read-only is the default and refuses writes before any I/O.
Build a normal `WriteRequest` and inspect confirmed/rejected/unknown results individually.
A suffix fixes the element type; otherwise the driver resolves `DataType`, `ValueRank`,
`ArrayDimensions` and `AccessLevel` and caches successful metadata in that session. Cold
nodes need a four-attribute Read, split if the item budget is smaller; cache hits need
no metadata I/O. Capacity is the connection item limit with FIFO replacement. A new
session has an empty cache; no cached identity/schema crosses reconnect.

Ranged tags hint List even for a single index. A whole-array write without a range uses
metadata/inference; scalar suffixes retain the core builder's exact scalar hint contract.
Lists flatten row-major, matrices carry dimensions, and rank/shape/bounds are checked.
Integers never truncate; floating narrowing must be exact. TIME is exact Int32 milliseconds
and `DateTime` is checked 1601 ticks aligned to 100 ns. `ByteString` uses Bytes. Character/
Unicode string forms use OPC String; unavailable WORD/DWORD/LWORD/WSTRING suffixes are
not added. Other IEC calendar/time suffix writes and custom structures return typed
errors; custom schema writes remain S120.

Eligible items are sent in one `WriteRequest` after local preparation; local failures remain
per-item rejections. `DataValue` contains only value, without invented status/timestamps.
Device result codes remain attached to each result. A lost/unusable exchange after possible
transmission is Unknown and is never replayed, including when close/cancellation interrupts
it. A verified `WriteResponse` with the wrong result count returns `MalformedData`; that error
must never be treated as permission to retry. Read retry settings do not apply to writes.
Deep caller Lists are traversed and rejected iteratively; owned request disposal also stays
iterative if an operation is abandoned before polling. No background task is introduced.

S100 treats an unexpected correlated service reply and an Uncertain Write status as
Unknown, preserving a device status when present. Bad statuses remain typed rejections.
Core's existing `TimeoutAfterTransmit` reason is the fallback for an unusable confirmation;
no new public reason variant is added. Only a verified `WriteResponse`'s wrong result count
remains the explicit `MalformedData` error described above, with no automatic replay.

## Crypto foundation and native build requirements (S101)

Enabling `opcua` compiles AWS-LC through optional aws-lc-rs 1.19.0/aws-lc-sys 0.47.0.
A C compiler is required; Windows `x86_64` additionally needs NASM (CI uses 2.16.03).
`CMake` is needed if the native build selects its `CMake` fallback. Defaults/prebuilt NASM
objects are disabled. Set `AWS_LC_SYS_USE_SYSTEM=0` to require the bundled source build;
do not combine that setting with `AWS_LC_SYS_SYSTEM_DIR`. Repository mise and CI set it.
The TLS feature retains its ring provider, so `tls,opcua` includes both native libraries.

S101 supplies policy descriptors, bounded `P_SHA1/P_SHA256` derivation and injectable OS
nonce generation. S106/S107 connect all three modern policies using the S102–S105 protection and trust primitives. No public RNG
configuration or new feature is introduced; `opcua` stays outside default/full. Key
formatting is redacted by Secret; this does not promise zeroization of allocations.

S102 adds private `MSG`/`CLO` protection for `Sign` and `SignEncrypt`: provider-backed
HMAC verification and AES-CBC with derived keys/IV, authenticated padding and header
binding. Decryption and authentication precede sequence/body parsing. Current/previous
keys are bounded by token id; installs re-derive from both nonces before replacing state.
Previous-token expiry/scheduling belongs to S109. S106/S107 wire these primitives into
all three modern policy connections.
Malformed protected data has one authentication error without provider/parser details;
this does not claim equal timing of the full receive path. No new dependency or feature.

## Client application key store

For a secure policy, `tls.keystore` names one PEM file containing the client certificate
chain (leaf first) and exactly one matching unencrypted RSA PKCS#8 `PRIVATE KEY`.
`tls.keystore-type=pem` is the default and only accepted format. PKCS#12, JVM stores,
passwords, encrypted keys, PKCS#1/SEC1 sections and duplicate keys are refused with
`InvalidConfig`. No client certificate is generated. None does not open a key store.

Loading opens the file once, reads at most 1 MiB plus one overflow-detection byte and
retains native signing/decryption handles. Errors and key-material Debug omit file paths,
contents and provider details. Secret formatting is redacted, without a zeroization claim.
The leaf must supply one nonempty absolute ASCII application URI in its SAN; secure
policies have no fallback URN. The chain and leaf SHA-1 thumbprint are prepared for the
later secure handshake; SHA-1 here is only a protocol identifier.

Client key validation precedes transport I/O. S106/S107 connect all modern policies with this
material. Loading client certificates does not establish server trust or chain validity.

## Server certificate trust preparation

S104 requires `server-certificate-file` for every secure policy before client-key or
transport I/O. It contains exactly one bootstrap server certificate, DER or PEM. Neither
`tls.trust-store` nor `tls.verify=false` replaces this file. No secure policy falls back
to None; `security-policy=NONE` is the explicit unprotected choice. None does not open
certificate or trust-store files.

Trust strategies are selected in this order: explicit `tls.verify=false`, a configured
`tls.trust-store`, the bootstrap certificate as an exact pin, otherwise reject. The store
is PEM by default (`tls.trust-store-type=pem`) or one DER certificate with type `der`.
Other formats and `tls.trust-store-password` are refused. Files are read once with a
1 MiB plus one overflow-detection-byte ceiling; errors omit paths, contents and sources.
The existing webpki individual-certificate size limit also applies. Only the selected
trust-store file is opened, but bootstrap structure and RSA bounds always apply.

The private strict verifier checks validity, RSA policy bounds and serverAuth EKU when
present. A byte-identical trusted leaf is accepted directly; other chains need a path to
configured, current anchors through the supplied issuers. Discovered or presented roots
never become anchors. Native PKCS#1/PSS SHA-256 verifies chain signatures. Pins compare
the leaf DER through AWS-LC's constant-time contents comparison; lengths are public.
Expired/future certificates, wrong usage, unknown issuers and malformed chains fail
with `Trust`. No OS/public roots, certificate downloads or revocation service is added.

Debug and structured tracing name the strategy; permissive preparation records
`tls.verify=false` at warning level. This bypasses trust acceptance, while cryptographic
certificate/key/signature requirements still apply. S106 applies the verifier before I/O,
to the authenticated OPN sender, to secure discovery and to the `CreateSession` certificate
before activation. Every presented leaf must match the provisioned server, even with
verification disabled.

## Asymmetric OPN protection

S105 adds a private OPN chunk engine. Every accepted secure policy signs and encrypts OPN,
including SIGN mode. PKCS1-SHA256/OAEP-SHA1 protect `Basic256Sha256` and `Aes128_Sha256_RsaOaep`;
PSS-SHA256/OAEP-SHA256 protect `Aes256_Sha256_RsaPss`. Sender signature size and receiver
cipher block size follow their respective RSA keys. Keys over 2048 bits use the two-byte
padding size. The clear header carries the sender chain and receiver leaf SHA-1 thumbprint.

The signature covers the final wire size, headers, sequence, payload and padding. Receive
checks the configured policy, provisioned sender leaf and local thumbprint, decrypts exact
RSA blocks and verifies the signature/padding before parsing the sequence or payload. No
presented certificate substitutes the provisioned cryptographic peer. Caller owns channel,
request-id, sequence and trust-policy checks. Invalid protected input has one `Authentication`
error without provider/parser details; input above its byte ceiling is `ResourceLimit`. This
is an error-shape guarantee, without a full-path constant-time claim.

Client/peer RSA preparation precedes transport I/O, with redacted errors and diagnostics.
The public open62541 tests pass all three modern policies in SIGN and `SIGN_ENCRYPT`, with discovery
on and off, chunked reads, writes and close. OPN is always signed and encrypted; MSG/CLO
are signed in SIGN and signed/encrypted in `SIGN_ENCRYPT`. Secure discovery runs over that
same protected connection, without a None fallback or a second socket.

`CreateSession` sends the client certificate chain, its SAN application URI and a fresh
32-byte nonce. Its request has no `clientSignature` field. The server signature must use the
policy algorithm and cover the exact client certificate bytes plus nonce. The returned
server certificate is verified and bound to the provisioned peer before `ActivateSession`
sends `clientSignature` over server certificate plus server nonce. Anonymous
`userTokenSignature` remains empty. Invalid or repeated server nonces fail authentication.

Protected user tokens are supported in S108; caller-driven renewal is supported in S109 (see below). A session currently lasts only as long as its
channel token. Native interop confirms RSA2048 and 8192-byte buffers; independent boundary
key-size and sequence-wrap behavior remain UNVERIFIED. Sequence exhaustion fails closed.

## Modern policy support (S107)

`Basic256Sha256` remains the default. `Aes128_Sha256_RsaOaep` and
`Aes256_Sha256_RsaPss` now use the same protected channel/session path, including secure
discovery, application proofs, chunked reads/writes and close. No policy downgrade or
extra socket is introduced. OPN is encrypted in both SIGN and `SIGN_ENCRYPT`.

| Policy | Symmetric encryption | Application/OPN signature | OPN encryption |
| --- | --- | --- | --- |
| `Basic256Sha256` | AES-256-CBC | RSA PKCS#1 SHA-256 | RSA-OAEP SHA-1 |
| `Aes128_Sha256_RsaOaep` | AES-128-CBC | RSA PKCS#1 SHA-256 | RSA-OAEP SHA-1 |
| `Aes256_Sha256_RsaPss` | AES-256-CBC | RSA-PSS SHA-256, 32-byte salt | RSA-OAEP SHA-256 |

Every modern policy uses 32-byte HMAC-SHA256/signing keys, 32-byte nonces and 16-byte IVs.
OAEP-SHA256 plaintext blocks hold RSA key bytes minus 66. SIGN omits symmetric encryption
and retains HMAC authentication on each MSG/CLO chunk.

Deprecated `Basic128Rsa15` and `Basic256` are refused with `InvalidConfig` naming their
deprecation and the unavailable `SHA1withRSA` signing operation. Endpoints offering only
these policies are not selectable. They remain deferred under E16; no legacy crypto
provider, opt-in bypass, dependency or feature has been added.


## Protected user identities (S108)

Use the existing `Credentials::new` for a username/password or
`Credentials::x509(certificate_file, private_key_file)` for a user certificate. The
application `tls.keystore` remains separate. The user certificate may be DER or PEM;
the key file must contain one unencrypted RSA PKCS#8 PEM key and no certificates.
Both files are bounded to 1 MiB, loaded before transport I/O, and must match. User
certificates need no application URI. Diagnostics redact paths and key contents.

All three modern token policies support username OAEP encryption and X.509 user
signatures. An explicit token policy controls its own algorithms; null/empty inherits
the endpoint policy. Password encryption includes a four-byte LE password+nonce length
and a fresh server nonce. Passwords above 64 UTF-8 bytes fail `ResourceLimit` pending
the enhanced secret format. X.509 sends only the user leaf and signs the exact server
certificate and nonce, separately from the application's client signature. Anonymous
and username `userTokenSignature` stay empty, confirmed by open62541. None token policy
is refused for X.509; plain username requires `allow-insecure-credentials=true` and a
static warning. SIGN also refuses plain username tokens.

Protected tokens on a None channel require `server-certificate-file`; trust and exact
bootstrap binding precede encryption/signing even with `tls.verify=false`. Deterministic
peers verify this path and refuse substitution, malformed certificates and bad/replayed
nonces. Native open62541 v1.5.9 requires an application certificate on None before
returning a usable session certificate; this adapter does not send one on None and
fails with `Trust`. Native successful username/X.509 evidence therefore covers secure
channels in both modes for every modern policy. Use a secure channel with that peer.

See ADR-0046 and the S108 evidence record. General transport-loss reconnect remains S110; no hardware claim.


## Caller-driven renewal and idle expiry (S109)

Operations renew a due channel token on the same socket and session, with no background
task. `channel-lifetime-ms` defaults to 3600000 and `min-channel-lifetime-ms` to 5000;
both accept positive `UInt32` milliseconds. The effective scheduling lifetime ignores a
zero or larger-than-requested server revision and raises a lower revision to the minimum
bounded by the request, with a static warning. Renewal is due at 75 percent, rounded up
to a positive millisecond using integer arithmetic. The scheduling minimum never extends
a server token's actual validity.

Every supported policy, including None, sends `OpenSecureChannel` Renew. Secure renewal
checks certificates/trust again, requires fresh nonces, derives new keys and preserves
request/sequence counters. The previous token is retained only through its original
lifetime plus 25 percent receive grace, or until the first authenticated new-token message.
Concurrent callers share a renewal. Full-table backpressure before admission is nonfatal;
a failed or dropped admitted renewal fails the channel immediately. No call sends user
requests with an uncertain old epoch.

Nothing happens while idle. If the channel or session has expired, the next read/write
repeats the handshake with a fresh session and empty metadata cache, using existing
credentials/trust settings and the operation's deadline/cancellation. Renewal itself
uses retained key material; a fresh handshake reloads the configured files. Read retry
limits are retained. Already-transmitted writes are never replayed and may return Unknown.
Cancelled/dropped idle preparation can be retried by another call; close prevents session
publication. Validating access, addresses, limits and cancellation still precedes recovery;
value/metadata checks retain the staged-write ordering.

Native open62541 tests keep one socket alive through more than three short lifetimes
for every modern `SIGN_ENCRYPT` policy and repeated None renewals, and recover after idle
channel/session expiry. Forced old-token delay and sequence wrap remain independently
UNVERIFIED. General transport-loss reconnect/backoff is S110. See ADR-0047 and S109 evidence;
no hardware claim.
