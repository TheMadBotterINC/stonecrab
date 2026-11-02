# Commissioning a Modbus acquisition profile

No physical device is currently available to this project. This is a procedure, not a
hardware record. The shipped fixture profiles are synthetic and describe no manufacturer.
The acquisition release can be verified against software peers without changing that status.

## Prepare the map

Record manufacturer, exact model, firmware, manual title/revision/page and interface
settings. Derive each address, data type, byte/word order, scale, unit and diagnostic flag
from that manual. Stonecrab's current Modbus addresses are logical one-based PLC4X
addresses: logical register 1 maps to wire offset 0. Convert explicitly from a manual's
notation; never guess offsets or choose the order that merely looks plausible.

Choose polling intervals at or above the device's minimum recommendations and stale
thresholds suitable for the measurement. Omit a threshold only when age-based freshness
is deliberately unavailable. Profile timing does not detect a physically stuck sensor
that keeps returning a successful response.

A manual-derived profile starts as `validation = "documented"`. Add independent
expected-value fixtures, including a negative or multi-register reading and device fault
flags, before promoting it to `fixture_backed`. Preserve manufacturer/model/firmware and
source/fixture provenance in the profile. Only a completed physical record can justify
`hardware_verified`. A software server mirroring the map is not physical evidence.
Use `profile_read` first to inspect the raw typed values and declared conversions.

Profile `[connection.options]` is documentary. Explicitly set required unit id, byte
order and addressing options in the connection URL. Profiles do not apply those settings.

## Continuous run

From the repository root, replace the URL and profile with the actual test setup:

```sh
STONECRAB_URL='modbus-tcp://192.0.2.10:502?default-unit-identifier=1&address-convention=plc4x' \
  STONECRAB_PROFILE=profiles/your_device.toml \
  STONECRAB_INTERVAL_MS=1000 STONECRAB_DURATION_MS=86400000 \
  cargo run --locked -p stonecrab --example profile_poll --features full -- --expect-good
```

`192.0.2.10` is a documentation address. The example reads only. Retain the summary and
redacted observations, host OS/architecture, commit, poll rate and device load. A healthy
run must see every configured measurement with no read failures or non-good observations.
The profile loop is demand-driven: the example exposes observation/failure counts, rather
than the native subscription loss counters. Duration includes startup; completion drops
the pending receive and explicitly closes the connection.

Run outage/fault experiments separately without `--expect-good`: interrupt the network,
restart the device, exercise documented fault indications and restore the connection.
Verify that original samples age into stale when subsequent receives can run, timestamps
are retained, failures remain visible, recovery yields fresh samples, and missed polls
are not replayed. The published profile loop awaits an in-flight read before it delivers
subsequent events, including stale transitions.
Test the actual serial gateway and unit routing if one will be deployed. Record terminal
failure or exhausted reconnect behavior as well as successful recovery.

Repeat on the intended ARM/x86 edge host and relevant firmware. Complete
[TEMPLATE.md](TEMPLATE.md) with links to retained evidence and list precisely the reads,
scales, status flags, loads and recovery cases that ran. Unexecuted cases remain gaps.

## Software checks available today

`mise run examples` runs one-shot and finite continuous simulator profiles.
`mise run interop` uses the independent pymodbus server and records continuous output in
`target/interop-evidence/profiles-poll.txt`. The acquisition unit suite includes a
hand-advanced 24-hour run (86,401 one-second reads of each of two measurements) with
exact read counts, no retained timers after receives, and clean shutdown. Virtual time
verifies scheduling/resource behavior; it does not replace a real-time endurance run or physical-device testing.
For a longer software-peer endurance run, use the same finite example with the interop
server and a longer `STONECRAB_DURATION_MS`, and retain its summary.
