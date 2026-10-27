# Draft upstream issue (quirk g): the address `4x:INT` fails with a parse exception

Parity rows: `MB-QUIRK-G`, `MB-ADDR-SHORT-4X` in `docs/parity/modbus.md`.

## Title

Modbus short address pattern makes the digit group optional, so `4x:INT` is accepted by
the regex and reaches `Integer.parseInt(null)`

## Affected code

`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/base/tag/ModbusTag.java`
line 48 (the fixed-digit pattern) and
`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/base/tag/ModbusTagHoldingRegister.java`
line 83 (keys `Tag` and `TagHR` in the Stonecrab parity record). The sibling area classes
(`ModbusTagCoil`, `ModbusTagDiscreteInput`, `ModbusTagInputRegister`) use the same pattern
shape at line 34.

## Observed behaviour

In the short address pattern the group of digits after the area prefix is optional, so an
address such as `4x:INT` (prefix, no digits, data type) matches. The matching group is
then null when the address number is parsed. INFERRED consequence: `Integer.parseInt(null)`
throws `NumberFormatException` instead of the typed invalid-tag error the other malformed
addresses produce. Not reproduced at run time.

## Expected behaviour

An address without the digit group is rejected with the same typed invalid-tag error as
any other malformed address (make the group mandatory in the pattern, or check for null
before parsing), with a test.

## How Stonecrab handles it

Not ported. `ModbusAddress::parse` requires the digits and returns a typed
`InvalidAddress` (`four_x_without_digits_is_rejected`, rows `MB-QUIRK-G` and
`MB-ADDR-SHORT-4X`).

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
