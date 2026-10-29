# Stonecrab Modbus UDP interoperability peer

The independent peer of `modbus-tcp:udp` (slice S61, ADR-0027): diagslave 3.6, the free
Modbus slave simulator of proconX, run as a Modbus UDP slave (`diagslave -m udp`), with
`tc netem` inside the container to lose, delay, duplicate and reorder its replies, which
Toxiproxy cannot do for UDP. The `interop-udp` job of `.github/workflows/ci.yml`,
and `mise run interop-udp` build it, run the driver against it and keep the evidence. It is
not part of `mise run interop`, because it needs a Docker that forwards published UDP ports.

## First-party statement, license and provenance

The recipe in this directory (`Dockerfile`, `serve.sh`, `interop.sh`) is first-party and
Apache-2.0 (`PROVENANCE.toml` lists the files). The simulator is not: it is proprietary
freeware under proconX's "Free Software License for Binary Distributions" (revision 5,
October 2026), which allows use for commercial or non-commercial purposes without charge and
binary copies for your own purpose, and allows redistribution only of the whole, unmodified
distribution. Stonecrab downloads and runs it in a container and never redistributes it:
neither the archive nor an image holding it is committed or pushed. `PROVENANCE.toml` records
the terms in more detail.

What is pinned: the base image `debian:trixie-slim` by index digest (the one of
`interop/libmodbus/`), `iproute2` 6.15.0-1 by exact version, and the diagslave archive by
SHA-256 (`01c87fe4...f2fe97e`, the archive whose README is revision 3.6 of 2026-10-10). The
download URL, `https://www.modbusdriver.com/downloads/diagslave.tgz`, carries no version, so
the vendor can replace the archive under the same name. When that happens the build fails
with a digest mismatch at the `ADD --checksum` step; it does not run another simulator. To
move the pin, download the new archive, read its `LICENSE-FREE.txt` and `README.txt`, and
record the new checksum here, in `PROVENANCE.toml` and in the Dockerfile. The image takes the
binary of its own architecture (x86_64 or aarch64); the aarch64 binary's banner reads
"diagslave 3.5", and the server log of every run keeps the banner.

## What it serves

diagslave answers every unit id, keeps holding registers, input registers, coils and inputs in
memory, zero until written, and logs every request. The tests write the three holding
registers 101 to 103 (one-based, 0x1111, 0x2222 and 0x3333) before they read them, so a reply
to the wrong request shows as a wrong value.

## The faults

The tests run `docker exec -u 0 stonecrab-interop-diagslave tc qdisc replace dev eth0 root
netem ...` (the container has the `NET_ADMIN` capability) and remove the discipline at the end
of each test, also when it fails; each test first removes a `netem` discipline left over and
checks that none is left.
`netem` runs on the Docker host's kernel, which needs the `sch_netem` module: `interop.sh
start` puts a discipline on and takes it off again, and fails when it cannot, and the CI job
loads the module first. `netem` shapes what leaves the container, so it acts on the replies:

| Test | netem | What must happen |
| --- | --- | --- |
| `interop_udp_writes_and_reads_back_registers_and_coils` | none | writes, reads and ping over UDP |
| `interop_udp_netem_loss_of_every_reply_times_out_at_the_deadline` | `loss 100%` | `Timeout` at the 500 ms deadline, twice; then the next read is answered |
| `interop_udp_netem_delay_below_the_deadline_still_answers` | `delay 300ms` | the right values, after at least 250 ms |
| `interop_udp_netem_delay_past_the_deadline_times_out_and_the_late_reply_is_discarded` | `delay 1000ms` | `Timeout` at 400 ms; the late reply arrives during the next request and is not taken for it |
| `interop_udp_netem_duplicated_replies_are_discarded_by_correlation` | `duplicate 100%` | a raw probe gets two copies of one reply, then twelve reads, each with its own value |
| `interop_udp_netem_reordered_replies_never_answer_the_wrong_request` | `delay 400ms reorder 50%` | thirty reads with a 250 ms deadline: every answer has the right value, every failure is `Timeout`, and there is at least one of each |

## Build and run

From the repository root, with Docker running:

```sh
docker build -f interop/diagslave/Dockerfile -t stonecrab-interop-diagslave:local .
docker run -d --name stonecrab-interop-diagslave --cap-add NET_ADMIN \
  -p 127.0.0.1:1502:1502/udp stonecrab-interop-diagslave:local
STONECRAB_INTEROP_DIAGSLAVE=modbus-tcp:udp://127.0.0.1:1502 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only --test-threads 1 \
  -E 'binary(modbus_interop_diagslave) & test(/^tests::interop_/)'
docker rm -f stonecrab-interop-diagslave
```

The container must keep the name `stonecrab-interop-diagslave`, because the tests apply the
faults to it by name. The image runs diagslave as an unprivileged user (uid 10001); `tc` runs
as root through `docker exec`. Docker must forward the published UDP port to the host: Docker
Engine on Linux and Docker Desktop do, but colima's default ssh port forwarder forwards TCP
only, so the readiness probe fails there (colima's grpc port forwarder,
`--port-forwarder grpc`, is documented to forward UDP too; it was not tried for S61).
`mise run interop-udp` does all of it, including the check that the filter selects exactly
six tests, and writes the evidence to `target/interop-evidence/diagslave/`: the test output,
the image id with its base and the archive checksum, the `iproute2` version and the server log.
