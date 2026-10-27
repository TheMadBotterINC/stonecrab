# Stonecrab pymodbus interoperability peer

A Modbus TCP server built on pymodbus 3.8.6 that serves the Stonecrab interop register
map. The `interop` job of `.github/workflows/ci.yml` and `mise run interop` build it,
run the driver against it and keep the evidence (slice S27, ADR-0015 decision 16).

## First-party statement

Every file in this directory is first-party and Apache-2.0 (`PROVENANCE.toml` lists them).
`server.py` was authored from `fixtures/modbus/interop/register-map.toml` and the section
"Interop register map" of `docs/parity/modbus.md`. Nothing is copied from Apache PLC4X:
upstream's `modbus_server.py`, its Dockerfile and its test vectors are not in this
repository (decision D8). The facts shared with upstream's setup are the base image family
(`python:3.12-slim`), the pymodbus version (3.8.6), port 5020 and the block sizes of the
parity record.

## What it serves

- Holding registers 1 to 31, the little-endian REAL at 300 and 301, input registers 1 to
  7, coils 1 to 10 and discrete inputs 1 to 10, with the values and words of the map.
  Every other position holds zero or false.
- Any unit id is answered. Logical address T is wire address T - 1; pymodbus 3.8.6 adds
  one before it indexes a block, so a block that starts at address 0 holds logical T at
  list index T.
- Register blocks have `N + 1` slots for a map size `N`, so logical `1..=N` is served and
  logical `N + 1` is refused with exception 02 for input registers (pymodbus validates
  FC03 and FC04). The holding block is larger, to cover registers 300 and 301, so holding
  `N + 1` is served.
- pymodbus 3.8.6 does not validate FC01 and FC02 reads: a coil read past the end of a block
  would be answered with a short bit list, not an exception. The coil and discrete-input
  blocks therefore cover the whole 16-bit address space (false where the map is silent),
  and no bit read is ever refused.
- Each request and reply is logged at INFO on stdout. The server exits with status 2 when
  the register map cannot be read or is invalid.

## Build and run

From the repository root, with Docker running:

```sh
docker build -f interop/pymodbus/Dockerfile -t stonecrab-interop:local .
docker run -d --name stonecrab-interop -p 127.0.0.1:5020:5020 stonecrab-interop:local
```

The image runs as an unprivileged user (uid 10001). To run the Stonecrab side by hand:

```sh
STONECRAB_INTEROP_MODBUS=modbus-tcp://127.0.0.1:5020 \
  cargo nextest run -p stonecrab-modbus --all-features --run-ignored ignored-only \
  -E 'test(/^tests::interop_/)'
STONECRAB_URL=modbus-tcp://127.0.0.1:5020 \
  cargo run -p stonecrab --example modbus_tcp_read --features modbus
docker rm -f stonecrab-interop
```

`mise run interop` does all of it, including the readiness probe, and writes the evidence
to `target/interop-evidence/`: `image-digest.txt`, `pip-freeze.txt`, `server.log`,
`contract-report.txt`, `nextest.txt` and `example.txt`. It needs the `docker` binary and a
running daemon and fails with a message naming the missing piece otherwise.

`interop.sh` takes `INTEROP_PORT` (default 5020) and `INTEROP_EVIDENCE_DIR`. The
Dockerfile pins the base image by digest; to move it, pull the new tag, record the index
digest in the `FROM` line and in the comment above it, and rerun the job.
