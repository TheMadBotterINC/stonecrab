# Stonecrab modbus-tk interoperability peer

The third Modbus TCP peer of the interop run (slice S52c): a short first-party server on
modbus-tk 1.1.5 that serves three unit ids on one port, each with a map of its own, and says
nothing to any other unit id. It exists to test unit-ID correlation, which pymodbus
(`single` context, every unit answered) and libmodbus (the unit id echoed back) cannot. The
`interop-modbus-tk` job of `.github/workflows/ci.yml` and `mise run interop` build it, run
the driver against it and keep the evidence.

## First-party statement and provenance

`server.py`, the `Dockerfile`, `requirements.txt` and `interop.sh` are first-party and
Apache-2.0 (`PROVENANCE.toml` lists the files). The server script is ours, so this peer
supports `interoperability-tested` at most and cannot back a named device profile. It uses
modbus-tk 1.1.5 (LGPL-2.1-or-later) as a library, installed from PyPI by the Dockerfile with
its dependency pyserial 3.5 (BSD-3-Clause, not used: no serial port is opened). Nothing of
either is copied into the repository.

What is pinned: the base image `python:3.12-slim` by index digest (the `FROM` line and the
comment above it; the same digest as the pymodbus peer), and `modbus-tk==1.1.5` and
`pyserial==3.5` in `requirements.txt`. To move any of them, change the line, rebuild, and
record the new digest or version here, in `PROVENANCE.toml` and in the Dockerfile comment.
The package installs run with the network on: the pins are versions, and PyPI is the source.

## What it serves

One TCP port, 5030, on every interface of the container (the address is explicit: modbus-tk
binds all interfaces when it is left empty). Addresses below are wire addresses (zero-based);
a Stonecrab address is one more. No value is the same on two units where their maps overlap.

| Unit | Holding registers (FC03) | Input registers (FC04) | Coils (FC01) | Discrete inputs (FC02) |
| --- | --- | --- | --- | --- |
| 1 | 0 to 15, value 1000 + address; 16 and 17 scratch, zero | 0 to 3, value 100 + address | 0 to 7, true at even addresses | 0 to 7, true where address mod 3 is 0 |
| 2 | 0 to 15, value 2000 + address; 16 and 17 scratch, zero | 0 to 3, value 200 + address | 0 to 7, true at odd addresses | 0 to 7, true where address mod 3 is 1 |
| 247 | 0 to 3, value 24700 + address | 0 to 1, value 2470 + address | 0 to 2, all true | none |

An address outside a unit's block (holding register 8 or 16 of unit 247, any discrete input
of unit 247) is answered with exception 02 (ILLEGAL DATA ADDRESS). Any other unit id, 0, 255 and
9 among them, gets no reply at all: modbus-tk logs `Slave N doesn't exist` and sends
nothing, and the connection stays open. The server serves several connections side by side,
so the tests run in parallel. One test writes: it sets the scratch registers 16 and 17 of
units 1 and 2 and puts them back; no other test reads them.

The control surface is the process. modbus-tk ships an RPC simulator on port 2711 that does
not run on Python 3; the script does not start it. SIGTERM or SIGINT stops the server, and
`--address` and `--port` say where it listens.

## Build and run

From the repository root, with Docker running:

```sh
docker build -f interop/modbus-tk/Dockerfile -t stonecrab-interop-modbus-tk:local .
docker run -d --name stonecrab-interop-modbus-tk -p 127.0.0.1:5030:5030 stonecrab-interop-modbus-tk:local
STONECRAB_INTEROP_MODBUS_TK=modbus-tcp://127.0.0.1:5030 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only \
  -E 'binary(modbus_interop_modbus_tk) & test(/^tests::interop_/)'
docker rm -f stonecrab-interop-modbus-tk
```

The image runs as an unprivileged user (uid 10001) and logs every request and reply, and
every request for a unit it does not hold, to its log. `mise run interop-modbus-tk` does all
of it, including the readiness probe (a real FC04 read of unit 1), and writes the evidence to
`target/interop-evidence/modbus-tk/`: `image-digest.txt` (image id and base image),
`pip-freeze.txt`, `server.log` and `nextest.txt`. The image id depends on the platform that
built it; only the base image digest and the two versions in `PROVENANCE.toml` are fixed.
`mise run interop` runs the pymodbus, libmodbus and this peer one after the other. They need
the `docker` binary and a running daemon and fail with a message naming the missing piece
otherwise.

`interop.sh` takes `INTEROP_MODBUS_TK_PORT` (default 5030) and `INTEROP_EVIDENCE_DIR`
(default `target/interop-evidence`; this peer writes to its `modbus-tk/` subdirectory).
