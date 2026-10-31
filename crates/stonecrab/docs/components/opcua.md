# Stonecrab OPC UA
The OPC UA client of Stonecrab (`stonecrab::opcua`), behind the opt-in `opcua` feature. It is
in neither `default` nor `full`, as `tls` is (decision E17, ADR-0034), and it needs the
`transport` feature. The module is being built in slices (milestone M3, ADR-0034): so far it
has the tag address grammar and, crate-private, the connection string parser and the
types of the binary encoding (the built-ins and the recursive Variant, `DataValue`,
`DiagnosticInfo` and `ExtensionObject` envelope), and nothing that opens a socket. There is no
public driver, no connection, no secure channel, no session and no message codec yet, and the
`opcua` scheme is not registered anywhere.

This module offers `OpcuaAddress`, a validated `ProtocolAddress` for the tag text
`ns=<n>;<i|s|g|b>=<id>[range][;a=<attribute>][;TYPE][|key=value,...]`, with the types its
accessors return: `OpcuaIdentifier`, `OpcuaAttribute` and `OpcuaDataType`. Parsing opens no
socket and starts no discovery.

| Capability | Support level |
| --- | --- |
| Tag address parse and canonical text (`OpcuaAddress`) | build-tested |
| Browse query parse (`OpcuaAddress::parse_query`) | build-tested |
| Connection string and driver options (crate-private until the driver) | build-tested |
| Built-in types of the binary encoding: String, `ByteString`, Guid, `DateTime`, `StatusCode`, `NodeId`, `ExpandedNodeId`, `QualifiedName`, `LocalizedText` (crate-private) | build-tested |
| Recursive and open types of the binary encoding: Variant (25 types, arrays, dimensions), `DataValue`, `DiagnosticInfo`, the `ExtensionObject` envelope with Raw, Xml and Null bodies, bounded in depth and in array counts (crate-private) | build-tested |
| Driver, connection, secure channel, session | planned |
| Read, write, browse, subscriptions | planned |
| Security policies and certificates | planned |
| Interoperability with an independent server | planned |
| Hardware verification | planned |

Build-tested here means unit tests, a property test and a fuzz target on the address text, a
fuzz target on the connection string and, for the built-in types, first-party byte vectors
written from OPC UA Part 6 (`fixtures/opcua/codec`) with a property test. The recursive types
have hand-written byte vectors in their unit tests, property tests and the fuzz target
`fuzz_opcua_decode`; their nesting and array-count bounds are tested at the limit and beyond it.
No parity claim above `planned` is made for the rest of the OPC UA record,
[`docs/parity/opcua.md`](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/opcua.md);
the rows marked `implemented` there (the `OPCUA-ADDR-*` rows, `OPCUA-QUIRK-BINARY-ID`, the
divergences `OPCUA-DIV-NAMESPACE-RANGE`, `OPCUA-DIV-BINARY-ID` and
`OPCUA-DIV-ADDRESS-ESCAPES`, and the connection string rows `OPCUA-CFG-SECURITY-POLICY`,
`OPCUA-CFG-MESSAGE-SECURITY`, `OPCUA-CFG-FRAMEWORK`, `OPCUA-CFG-UNKNOWN`,
`OPCUA-DIV-UNKNOWN-KEYS`, `OPCUA-TRANSPORT-TCP`, `OPCUA-TRANSPORT-ENDPOINT-URL`,
`OPCUA-TRANSPORT-PORT`, `OPCUA-QUIRK-INET-PATTERN` and `OPCUA-QUIRK-TLS-NAMES`) are the ones
that have a claim.

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
- Every other key is `InvalidConfig`, and so is a key written twice. That includes the keys
  the framework fills (`protocol-code`, `transport-code`, `transport-config`), the
  credentials a connection string must not carry (`username`, `password`; they arrive in
  `Credentials`) and the `tls.*` certificate options, which later slices implement.
- A message names the key (shortened to 32 characters) or a fixed reason; it never repeats a
  value, the host or the path.

## Built-in types
Crate-private since slice S85 and called by nothing yet: the hand-written scalar types of the
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
Crate-private since slice S86 and called by nothing yet: the Variant, `DataValue`,
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
  standard types are added with the generated layer.

Not checked against a server yet (UNVERIFIED, see the ADR-0034 addendum of S86): the order of
the locale and the localized text in a `DiagnosticInfo`, a dimensions flag on a scalar, and
whether servers set the unused bits of the `ExtensionObject` encoding byte.
