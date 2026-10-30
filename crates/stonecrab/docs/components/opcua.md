# Stonecrab OPC UA
The OPC UA client of Stonecrab (`stonecrab::opcua`), behind the opt-in `opcua` feature. It is
in neither `default` nor `full`, as `tls` is (decision E17, ADR-0034), and it needs the
`transport` feature. The module is being built in slices (milestone M3, ADR-0034): so far it
has the tag address grammar and, crate-private, the connection string parser, and nothing
that opens a socket. There is no public driver, no connection, no secure channel, no session
and no codec yet, and the `opcua` scheme is not registered anywhere.

This module offers `OpcuaAddress`, a validated `ProtocolAddress` for the tag text
`ns=<n>;<i|s|g|b>=<id>[range][;a=<attribute>][;TYPE][|key=value,...]`, with the types its
accessors return: `OpcuaIdentifier`, `OpcuaAttribute` and `OpcuaDataType`. Parsing opens no
socket and starts no discovery.

| Capability | Support level |
| --- | --- |
| Tag address parse and canonical text (`OpcuaAddress`) | build-tested |
| Browse query parse (`OpcuaAddress::parse_query`) | build-tested |
| Connection string and driver options (crate-private until the driver) | build-tested |
| Driver, connection, secure channel, session | planned |
| Read, write, browse, subscriptions | planned |
| Security policies and certificates | planned |
| Interoperability with an independent server | planned |
| Hardware verification | planned |

Build-tested here means unit tests, a property test and a fuzz target on the address text and
a fuzz target on the connection string.
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
