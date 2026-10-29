# Draft upstream issue (quirk c): the FC05 write-echo check is dead code

Parity rows: `MB-QUIRK-C`, `MB-TCP-WRITE` in `docs/parity/modbus.md`.

## Title

`ModbusTcpConnection`: the response check for Write Single Coil (function 05) can never
run because coil writes always use function 15

## Affected code

`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus/tcp/ModbusTcpConnection.java`:
the function 05 echo check at lines 371-377, against the request construction for coils at
lines 438-448 (key `Conn` in the Stonecrab parity record).

## Observed behaviour

The write path builds a Write Multiple Coils (function 15) request for every coil write,
including a single coil (lines 438-448). The response handler nevertheless contains a
branch for a Write Single Coil (function 05) response that verifies the echoed value
(lines 371-377). Because no code path emits function 05, that branch is unreachable: it
never runs and it misleads a reader about which function a coil write uses. Not
reproduced at run time.

## Expected behaviour

Either remove the unreachable function 05 branch, or send function 05 for a single-coil
write and keep the echo check, with a test for whichever is
chosen.

## How Stonecrab handles it

Not ported. Since S46 (2026-10-28) `modbus-tcp` writes (`Capabilities.write = true`), and
by default a single coil goes out as function 15, as upstream does. Since S73 (2026-10-29)
the opt-in connection-string parameter `single-coil-write=fc05` sends a write of exactly one
coil as function 05, for devices that accept nothing else, and then its echo is checked
(`write_single_coil_is_encoded_and_its_echo_is_decoded_and_proved`); `decode_request` still
refuses function 05 (`write_functions_that_upstream_does_not_send_are_not_decoded`).
The choice is the `MB-QUIRK-C` divergence row of `docs/parity/modbus.md`; the write path
is covered by `crates/stonecrab/tests/modbus_write.rs`.

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
