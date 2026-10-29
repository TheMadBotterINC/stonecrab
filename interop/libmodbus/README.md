# Stonecrab libmodbus interoperability peer

The second independent Modbus TCP peer of the interop run (slice S52a): libmodbus 3.1.11's
`unit-test-server`, a C stack that shares no code with Stonecrab or with the pymodbus peer.
The `interop-libmodbus` job of `.github/workflows/ci.yml` and `mise run interop` build it, run
the driver against it and keep the evidence.

## First-party statement and provenance

The recipe in this directory (`Dockerfile`, `serve.sh`, `interop.sh`) is first-party and
Apache-2.0 (`PROVENANCE.toml` lists the files). The server is not: the image compiles the
`unit-test-server.c` and `unit-test.h` that Debian trixie's `libmodbus-dev` 3.1.11-2 installs
under `/usr/share/doc/libmodbus-dev/examples/` (BSD-3-Clause, Copyright Stephane Raimbault)
and links the `libmodbus5` 3.1.11-2 shared library (LGPL-2.1-or-later). Nothing of libmodbus is
copied into the repository; the constants of `unit-test.h` that the tests expect are
transcribed in `crates/stonecrab/tests/modbus_interop_libmodbus.rs` with their source named.

What is pinned: the base image `debian:trixie-slim` by index digest (`FROM` line and the
comment above it), the two packages by exact version, and the two source files by SHA-256.
To move any of them, change the line, rebuild, and record the new digest or version here, in
`PROVENANCE.toml` and in the Dockerfile comment. A trixie archive that no longer has
3.1.11-2 fails the build at the install step; it does not build another server.

## What it serves

The server holds 37 coils from wire address 0x130, 22 input bits (0xAC 0xDB 0x35) from
0x1C4, 32 holding registers from 0x160 and one input register at 0x108 (0x000A). Coils and
holding registers start at zero. It answers any unit id with the same one, and:

| Wire address | Function | The server |
| --- | --- | --- |
| 0x170 | FC03 | exception 06 |
| 0x171 | FC03 | a reply with transaction id 0 and unit 255 |
| 0x172 | FC03 | the reply after 500 ms |
| 0x173 | FC03 | the 11-byte reply one byte at a time, 5 ms apart, unit 255 |
| any | FC03 of 2 registers | a reply that holds 1 register |

It serves one connection and exits when the client closes it. `serve.sh` is the container's
entry point and starts it again, so the server is up for the next client within
milliseconds; a connection made in that gap is closed at once by Docker's port proxy.
Because of that, and because a test's writes die with its connection, every test opens one
connection and `interop.sh` runs the tests one at a time (`--test-threads 1`). The readiness
probe is a connection too and costs one restart. Never send function code 07 (read exception
status): libmodbus 3.1.11 exits on it without replying.

## Build and run

From the repository root, with Docker running:

```sh
docker build -f interop/libmodbus/Dockerfile -t stonecrab-interop-libmodbus:local .
docker run -d --name stonecrab-interop-libmodbus -p 127.0.0.1:1502:1502 stonecrab-interop-libmodbus:local
STONECRAB_INTEROP_LIBMODBUS=modbus-tcp://127.0.0.1:1502 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only --test-threads 1 \
  -E 'binary(modbus_interop_libmodbus) & test(/^tests::interop_/)'
docker rm -f stonecrab-interop-libmodbus
```

The image runs as an unprivileged user (uid 10001) and prints libmodbus's trace of every
request and reply to its log. `mise run interop-libmodbus` does all of it, including the
readiness probe, and writes the evidence to `target/interop-evidence/libmodbus/`:
`image-digest.txt` (image id, base image, package version), `server.log` and `nextest.txt`.
The image id depends on the platform that built it (arm64 and x86_64 differ); only the
base image digest and package versions in `PROVENANCE.toml` are fixed, and the evidence file
of a run is the record of its image id.
`mise run interop` runs the pymodbus peer first and this one second. Both need the `docker`
binary and a running daemon and fail with a message naming the missing piece otherwise.

`interop.sh` takes `INTEROP_LIBMODBUS_PORT` (default 1502) and `INTEROP_EVIDENCE_DIR`
(default `target/interop-evidence`; this peer writes to its `libmodbus/` subdirectory).
