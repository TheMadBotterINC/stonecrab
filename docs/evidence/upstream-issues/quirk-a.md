# Draft upstream issue (quirk a): Modbus driver checks for a BouncyCastle UDPTransport

Parity rows: `MB-QUIRK-A`, `MB-TCP-TRANSPORTS-UDP` in `docs/parity/modbus.md`.

## Title

ModbusTcpDriver selects its configuration class with `instanceof` against
`org.bouncycastle.tls.UDPTransport`, an unrelated class

## Affected code

`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/tcp/ModbusTcpDriver.java`,
import at line 34 and the transport check at lines 66-67 (as cited by the Stonecrab parity
record, key `Drv`).

## Observed behaviour

The driver imports `org.bouncycastle.tls.UDPTransport`, a class of the BouncyCastle TLS
library, and tests the selected transport with `instanceof` against it to choose the UDP
configuration class. A PLC4X transport is not a BouncyCastle `UDPTransport`, so the
check cannot be true for the PLC4X `udp` transport. INFERRED consequence: a
`modbus-tcp:udp://` connection falls through to the default (TCP) transport
configuration class instead of the UDP one. The UDP connection itself is exercised
upstream at `ModbusDockerIT.java` lines 592-636, so the transport works; the question is
only which configuration class it receives. This was not run.

## Expected behaviour

The check should name the PLC4X UDP transport type (or compare the transport code) so
that a UDP connection receives the UDP configuration class, and the wrong import should
not compile into a dependency on BouncyCastle for a Modbus driver.

## How Stonecrab handles it

Not ported. Stonecrab does not support Modbus over UDP at the alpha; the driver rejects
any transport other than TCP with `InvalidConfig`
(`driver_rejects_other_transport_codes`, row `MB-QUIRK-A`).

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
