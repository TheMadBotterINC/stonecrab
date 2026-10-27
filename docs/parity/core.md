# Foundation scope
Status: initial implementation; no upstream behavioral parity claim.

The architecture maps values, tags, errors and capabilities to PLC4J concepts.
This slice provides pure foundation types and validated request/result data.
There is no `DriverRegistry`, connection, subscription, codec, driver or sensor profile.

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
- `ProtocolAddress` and immutable `Tag`: driver-owned addresses, stable caller
  identity, type-hint consistency, positive counts and shared immutable ownership.
- `ReadRequest`: finite item budgets, duplicate rejection and explicit deadline
  metadata; no transmission or cancellation implementation.
- `ProtocolStatus`, `ItemError`, `ReadItem`: original device status and bounded raw
  diagnostics, redacted Debug, source chaining and actual source-time absence.
- `ReadResponse`: exact correlation and request ordering across partial/all-item
  failure, split-batch disclosure and monotonic interval checks.
- `WriteOutcome`: confirmed, rejected and unknown-after-transmit data; no write
  operation or retry behavior is implemented.

- `ByteOrder`, `WordOrder`, `Tag::with_byte_order`, `Tag::with_word_order`: explicit,
  never-inferred byte and word order as optional tag fields; additive `Ord` on
  `ErrorKind`, `Operation` and `PlcValueType`. See ADR-0006 and "Byte and word order".

Unit tests live beside each implementation; `crates/stonecrab/tests/public_contract.rs`
checks the consumer boundary. README examples are executable doctests.
`crates/stonecrab-core/tests/read_contract.rs` uses the unpublished shared testkit
to exercise addresses and batch contracts without ad hoc mocks or network access.

## Byte and word order
Status: implemented as data only (ADR-0006, slice S05). The types carry the setting; no
decoder consumes it yet. The Modbus mapping and its row ids are the `MB-BYTEORDER-*`
rows in `docs/parity/modbus.md` (`MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`,
`MB-BYTEORDER-BE-SWAP`, `MB-BYTEORDER-LE-SWAP`, `MB-BYTEORDER-PRECEDENCE`), which stay
`planned` there until the upstream little-endian vectors pass (S23).

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-BYTEORDER-MODEL | One setting `ModbusByteOrder` with four modes; no separate word order (`docs/parity/modbus.md` "Byte order"). | `ByteOrder` x `WordOrder`, both `Copy` and `Ord`, never inferred; `Tag::with_byte_order`/`with_word_order` set optional fields that default to `None`; precedence tag field, address block, driver default. | implemented | `byte_order_is_total_and_follows_declaration_order`, `word_order_is_total_and_follows_declaration_order`, `orders_are_copy_hash_and_clone_equal`, `debug_text_names_the_variant`, `orders_default_to_absent_and_are_never_inferred`, `each_builder_sets_exactly_one_field`, `both_orders_combine_independently_of_call_order_and_last_write_wins`, `orders_do_not_disturb_other_fields_and_survive_clone_and_debug`, `facade_reexports_ordering_types_and_additive_ord` |
| CORE-VALUE-ORD | Not applicable (Java enums are comparable by ordinal). | Additive `PartialOrd, Ord` on `ErrorKind`, `Operation`, `PlcValueType` in declaration order; supports `BTreeSet<ErrorKind>`. | implemented | `error_kinds_sort_in_declaration_order_and_round_trip_through_a_set`, `operations_sort_in_declaration_order_and_round_trip_through_a_set`, `value_types_sort_in_declaration_order_stay_copy_and_round_trip_through_a_set` |

`RawValue` and calendar, bytes and list values are deferred (ADR-0006).

## Connection string grammar
Status: planned. No `ConnectionUri` exists yet; the slice that implements it is S08 in
`docs/ALPHA_PLAN.md`.

| Row id | Upstream behavior (file:line) | Stonecrab planned behavior | Status | Planned test ids |
| --- | --- | --- | --- | --- |
| CORE-URI-GRAMMAR | `DriverBase.URI_PATTERN` is `^(?<protocolCode>[a-z0-9\-]*)(:(?<transportCode>[a-z0-9\-]*))?://(?<transportConfig>[^?]*)(\?(?<paramString>.*))?` (`plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/DriverBase.java:76-77`), applied with `matches()` at `DriverBase.java:149-150`. Protocol code, optional transport code, `://`, transport configuration up to the first `?`, then the raw parameter string. | `ConnectionUri::parse(&str)` for `scheme(:transport)?://authority(?k=v(&k=v)*)?`. Percent-decoding is rejected and documented. Credentials in the authority are rejected. Duplicate or empty keys are `InvalidConfig`. The input is bounded at 4096 bytes. Parsing never panics. Redacted display never contains credential text. | planned | `prop_uri_round_trips_valid_parts`, `prop_redacted_display_never_contains_password`, `uri_percent_encoding_is_rejected`, `uri_credentials_in_authority_are_rejected`, `uri_duplicate_key_is_rejected`, `uri_empty_key_is_rejected`, `uri_over_4096_bytes_is_rejected`, `fuzz_core_connection_string` |

Baseline: Apache PLC4X v1.0.0, commit `ea229918f5d07700e04d7d69ab1645da679cb700`
(`upstream/manifest.toml`). The Modbus use of the grammar, including transport codes and
default ports, is row `MB-TCP-TRANSPORTS` in `docs/parity/modbus.md`.
