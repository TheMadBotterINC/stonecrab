# Draft upstream issue (quirk e): Modbus TCP replies are matched on transaction id only

Parity rows: `MB-QUIRK-E`, `MB-READ-MATCH` in `docs/parity/modbus.md`.

## Title

ModbusTcpConnection correlates a response with its request by transaction id alone;
unit identifier and function code are not checked

## Affected code

`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/tcp/ModbusTcpConnection.java`,
lines 145-157 (key `Conn` in the Stonecrab parity record).

## Observed behaviour

An incoming ADU is looked up in the table of pending requests by its MBAP transaction
identifier. If the identifier is unknown, the frame is logged and dropped. If it is
known, the response is delivered to that request without comparing the response's unit
identifier with the request's, and without checking that the response function code is
the request's function code (or that function with the exception bit set). A response
from another unit with the same function type that carries a pending transaction
identifier would therefore be decoded as the answer. A response of another function type
is delivered too, but `extractResponseData` then logs "Unexpected response type" and
returns null, so the read fails or returns no data.
Read from the code; not reproduced.

## Expected behaviour

A response is accepted only when its transaction identifier, unit identifier and function
code (exception bit cleared) match the pending request. Section 7 of the Modbus
Application Protocol Specification V1.1b has an exception response echo the function code
with the high bit set, and the Modbus Messaging on TCP/IP Implementation Guide defines
the unit identifier echo; a mismatch should be discarded and counted, not delivered.

## How Stonecrab handles it

Divergence, tested: the session accepts a reply only on a match of all three values and
discards, counts (`late_replies`, `mismatched_replies`) and logs everything else
(`reply_with_wrong_unit_is_discarded`, `reply_with_wrong_function_code_is_discarded`,
`reply_with_wrong_txn_is_discarded`, rows `MB-QUIRK-E` and `MB-READ-MATCH`). The unit
identifier echo rule comes from the TCP implementation guide, which is not retained in
the repository (UNVERIFIED in the parity record).

## Provenance and status

- Found by reading the source of Apache PLC4X v1.0.0 (tag object
  `32a524591841ab56d26b032914ce06918148fbb6`, commit
  `ea229918f5d07700e04d7d69ab1645da679cb700`) during the Stonecrab parity audit
  (`docs/parity/modbus.md`). It was not reproduced against a running upstream unless the
  Observed behaviour section says so; where the consequence is inferred from the code, the
  text says INFERRED.
- Status: DRAFT, not filed. Filing on the upstream tracker needs the Stonecrab
  maintainer's approval (ADR-0017). Nothing has been posted. When filed, replace this
  line with the issue URL and update the "Divergences from upstream" table of
  `docs/parity/modbus.md`.
- Line numbers refer to the pinned commit only; re-check them against the current
  upstream branch before filing.
