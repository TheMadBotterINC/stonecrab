# Draft upstream issue (quirk f): Modbus TCP test suite passes an undeclared key

Parity rows: `MB-QUIRK-F`, `MB-CFG-UNKNOWN` in `docs/parity/modbus.md`.

## Title

`ManualFactoryModbusTCPDriverTest-testsuite.xml` uses `unit-identifier`, which
`ModbusTcpConfiguration` does not declare (the parameter is `default-unit-identifier`)

## Affected code

- `protocols/modbus/src/test/resources/protocols/modbus/tcp/ManualFactoryModbusTCPDriverTest-testsuite.xml`
  lines 30-33 (key `MF` in the Stonecrab parity record).
- `plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/tcp/config/ModbusTcpConfiguration.java`
  lines 36-39 (key `Cfg`): the declared parameter is `default-unit-identifier`.
- Related: unknown connection parameters are reported but not rejected by `DriverBase`
  (`plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/DriverBase.java`
  lines 206-209 and 315-319; key `DB`).

## Observed behaviour

The test suite configures the unit identifier as `unit-identifier`. The configuration
class declares no such parameter, so the key is not applied; the suite passes only
because the value it sets equals the default (1). A misspelt or unsupported parameter is
silently ignored, so the test does not exercise what it appears to exercise. Read from
the code and the test resource; the suite was not run.

## Expected behaviour

The test resource uses the declared name `default-unit-identifier`, and (as a separate
question for the maintainers) an unknown parameter fails the connection string instead of
being silently ignored, so that a typo cannot leave a default in place.

## How Stonecrab handles it

Not ported. Stonecrab rejects unknown configuration keys with `InvalidConfig`
(`config_unknown_key_is_rejected`, `config_misspelt_unit_identifier_is_rejected`, rows
`MB-QUIRK-F` and `MB-CFG-UNKNOWN`).

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
