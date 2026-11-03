# Pinned OPC UA upstream corpus (S93)

`upstream/` retains the two Apache PLC4X XML suites verbatim at commit
`ea229918f5d07700e04d7d69ab1645da679cb700`. Their ASF headers, git blob identifiers and
SHA-256 fingerprints are retained and checked offline against `upstream/manifest.toml`.
Original capture attribution to Eclipse Milo is **UNVERIFIED-capture**; this is public
historical test data, not independent live-server interoperability evidence.

The test-only adapter is `crates/stonecrab/tests/support/opcua_upstream.rs`, included by
`opcua.rs` only with `test` and `testkit`. It serializes XML annotations independently of
Stonecrab's codecs, compares all 11 raw values with the upstream literal bytes, and
produces the 12 driver messages. The golden-loader reads the resulting 23 hex/JSON pairs.
The exact file set and deterministic output are checked on every all-feature test run.
To deliberately regenerate after review:

```sh
STONECRAB_REGENERATE_OPCUA_VECTORS=1 cargo test --locked -p stonecrab --all-features \
  --lib opcua::upstream_vectors::opcua_upstream_adapter_reproduces_the_complete_reviewed_corpus
```

No suites, adapter, hex vectors or provenance are shipped in the crate archive. There is
no runtime or dependency change. The crate NOTICE carries the attribution.

## Derived implicit lengths

The pinned `opc-manual.mspec` defines MessagePDU.totalLength and PascalString.sLength as
serializer-computed implicit lengths. Driver XML has stale annotations; the adapter
computes those fields from content and records this exact closed list in expected JSON:

| Message | Annotation | Declared | Derived |
| --- | --- | --- | --- |
| Acknowledge | totalLength | 24 | 28 |
| CreateSession request | sLength | 78 | 61 |
| CreateSession request | bitLength | 624 | 488 |
| CreateSession request | totalLength | 310 | 293 |
| CreateSession response | sLength | 25 | 14 |
| CreateSession response | bitLength | 200 | 112 |
| CreateSession response | totalLength | 6450 | 1867 |

The original XML is unchanged. This does not relax runtime malformed-length checks.

## Equivalence boundary

There are **15 exact round-trips, one named canonical round-trip and seven exact
service refusals** through the current codec subset. One of the exact round-trips,
`raw/opc-ua-extension-object`, preserves DataChangeNotification (encoding id 811) as a Raw
ExtensionObject body. It proves envelope preservation, not notification decoding.

`driver/send-read-request` uses the seven-byte numeric NodeId form for namespace zero,
id 2258. The existing Stonecrab codec emits the equivalent four-byte form, reducing the
frame from 131 to 128 bytes. Its `reencodes_as` metadata is derived independently; both
forms decode to the same Read request and the compact result is idempotent. This follows
the canonical-output precedent in the first-party built-in vectors and is recorded as
`OPCUA-DIV-NUMERIC-NODE-ID-ENCODING`; runtime behavior is unchanged.

The refusal test requires the exact disabled discriminator field and encoding id for
each entry; any other failure, missing vector or newly supported entry fails the test.

| Vector | Encoding id / disabled type | Owning slice |
| --- | --- | --- |
| raw/opc-ua-publish-response-payload-data-change | 829 / PublishResponse | S116 |
| raw/opc-ua-publish-response-payload-event-notification | 829 / PublishResponse | S116; event content S118 |
| raw/opc-ua-notification-message | 805 / NotificationMessage | S116 |
| raw/opc-ua-monitored-item-notification | 808 / MonitoredItemNotification | S116 |
| raw/opc-ua-monitoring-parameters | 742 / MonitoringParameters | S114 |
| driver/send-write-request | 673 / WriteRequest | S100 |
| driver/receive-write-response | 676 / WriteResponse | S100 |

S123 must empty the refusal list and replace the opaque id-811 assertion with semantic
DataChangeNotification coverage (S116). Full fourteen-service parity stays planned.
