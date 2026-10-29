# Stonecrab pymodbus interoperability peer

A Modbus TCP server built on pymodbus 3.8.6 that serves the Stonecrab interop register
map. The `interop` job of `.github/workflows/ci.yml` and `mise run interop` build it,
run the driver against it and keep the evidence (slice S27, ADR-0015 decision 16).

## First-party statement

Every file in this directory is first-party and Apache-2.0 (`PROVENANCE.toml` lists them).
`server.py` was authored from `fixtures/modbus/interop/register-map.toml` (with its
sibling `text-registers.toml` since S49) and the section
"Interop register map" of `docs/parity/modbus.md`. Nothing is copied from Apache PLC4X:
upstream's `modbus_server.py`, its Dockerfile and its test vectors are not in this
repository (decision D8). The facts shared with upstream's setup are the base image family
(`python:3.12-slim`), the pymodbus version (3.8.6), port 5020 and the block sizes of the
parity record.

## What it serves

- Holding registers 1 to 31, the little-endian REAL at 300 and 301, input registers 1 to
  7, coils 1 to 10 and discrete inputs 1 to 10, with the values and words of the map, and
  the text registers 310 to 328 of `fixtures/modbus/interop/text-registers.toml` (slice
  S49: `STRING`, `WSTRING`, `CHAR`, `WCHAR`, a byte-swapped string and one string whose
  bytes are not UTF-8). Every other position holds zero or false.
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
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only \
  -E 'binary(modbus_interop_pymodbus) & test(/^tests::interop_/)'
STONECRAB_URL=modbus-tcp://127.0.0.1:5020 \
  cargo run -p stonecrab --example modbus_tcp_read --features modbus
STONECRAB_URL='modbus-tcp://127.0.0.1:5020?address-convention=plc4x' \
  STONECRAB_PROFILE=profiles/synthetic/modbus-tcp/temperature_transmitter.toml \
  cargo run -p stonecrab --example profile_read --features full -- --expect-good
docker rm -f stonecrab-interop
```

The last command is one of the `profiles` step: the synthetic profiles of
`profiles/synthetic/modbus-tcp/` are authored from the same register map, and
`profile_read --expect-good` reads each against the server and exits 1 unless every
measurement is of quality `Good` (see `profiles/README.md`).

`mise run interop` does all of it, including the readiness probe, and writes the evidence
to `target/interop-evidence/`: `image-digest.txt`, `pip-freeze.txt`, `server.log`,
`contract-report.txt`, `nextest.txt`, `example.txt` and `profiles.txt` (the
`profile_read` output for each Modbus profile, written by `interop.sh profiles`). It
needs the `docker` binary and a running daemon and fails with a message naming the
missing piece otherwise.

## TLS mode (slice S71)

With `--tls-cert`, `--tls-key` and `--tls-ca`, `server.py` serves the same blocks over TLS 1.2
or 1.3 with pymodbus's TLS server (MBAP frames inside the session, Python's ssl module, which
is OpenSSL 3.5 in this image) and requires a client certificate issued by the authority in
`--tls-ca`. `tls.sh` drives that run (ADR-0032): it builds the image, makes a throwaway PKI
with openssl inside the image (ECDSA P-256: an authority, a server certificate for
`127.0.0.1` and `localhost`, a client certificate and key, an unrelated authority), copies
the server's three files into a container created on `127.0.0.1:8802` with `docker cp`,
probes with a real read over mutual TLS, runs the three `interop_tls_` tests of
`crates/stonecrab/tests/modbus_interop_tls.rs` and keeps the image digest, the server
certificate's summary and the server log in `target/interop-evidence/tls/`. No key is
committed or kept as evidence: the PKI lives in `target/interop-tls-pki/` and the `stop` step
removes it. `mise run interop-tls` runs it alone; `mise run interop` runs it last; the CI job
is `interop-tls`. When the server refuses a client without a certificate it closes the
connection without sending its TLS alert, so the driver sees `Disconnected` there, not
`Authentication`. `tls.sh` takes `INTEROP_TLS_PORT` (default 8802), `INTEROP_EVIDENCE_DIR`
and `INTEROP_TLS_PKI_DIR`.

`interop.sh` takes `INTEROP_PORT` (default 5020) and `INTEROP_EVIDENCE_DIR`. The
Dockerfile pins the base image by digest; to move it, pull the new tag, record the index
digest in the `FROM` line and in the comment above it, and rerun the job.
