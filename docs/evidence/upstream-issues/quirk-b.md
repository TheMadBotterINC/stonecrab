# Draft upstream issue (quirk b): a split extended-register read drops its second item

Parity rows: `MB-QUIRK-B`, `MB-ADDR-EXTENDED` in `docs/parity/modbus.md`.

## Title

Modbus extended-register (FC20) read split into two file-record items returns only the
first item

## Affected code

`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/tcp/ModbusTcpConnection.java`:
`extractResponseData` at lines 517-518, and the construction of the split request at
lines 422-431 (key `Conn` in the Stonecrab parity record).

## Observed behaviour

When an extended-register tag is large enough that the read is built as several
file-record items in one FC20 request (lines 422-431), the code that extracts the data
from the response returns only the first file-record item (lines 517-518). The data of the
second item is not part of the result and no error is reported. This is a
reading of the code; it was not reproduced against a running upstream or a device.

## Expected behaviour

Every file-record item of the response is concatenated in request order, or the read
fails with an explicit error when the response cannot be mapped to the request.

## How Stonecrab handles it

Not ported. Extended registers (FC20 and FC21, the `6x` and `extended-register` address
forms) are rejected at parse time with a typed `InvalidAddress`
(`extended_register_is_rejected_at_parse_time`, rows `MB-QUIRK-B` and
`MB-ADDR-EXTENDED`); support is deferred to M2.

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
