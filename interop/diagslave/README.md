# Stonecrab diagslave interoperability peer (Modbus UDP, RTU over TCP, ASCII and RTU on a serial device, and the drivers' own serial port)

The independent peer of `modbus-tcp:udp` (slice S61, ADR-0027) and of `modbus-rtu:tcp` (slice
S59b, ADR-0024), and of `modbus-ascii:tcp` and `modbus-rtu:tcp` as diagslave frames them on a
serial line (slice S60b, ADR-0024): diagslave 3.6, the free Modbus slave simulator of
proconX, in one image run in four modes. `serve.sh` takes the mode as its argument.

- `udp` (the default): a Modbus UDP slave (`diagslave -m udp -p 1502`), with `tc netem` inside
  the container to lose, delay, duplicate and reorder its replies, which Toxiproxy cannot do
  for UDP. The `interop-udp` job of `.github/workflows/ci.yml` and `mise run interop-udp`
  build it, run the driver against it and keep the evidence. It is not part of `mise run
  interop`, because it needs a Docker that forwards published UDP ports.
- `enc`: a Modbus RTU over TCP slave (`diagslave -m enc -p 1503 -a 1`), which is the stream
  a serial-to-IP gateway carries: RTU frames with their CRC-16 on a TCP connection, the
  carrier of `modbus-rtu:tcp://host:port`. The `interop-rtu` job and `mise run interop-rtu`
  (which `mise run interop` includes, because TCP is forwarded everywhere) do the same for
  it. `interop.sh` reads the mode from `INTEROP_DIAGSLAVE_MODE` (`udp` by default, `enc`
  for the RTU run; the task and the job set it). The two modes use different container
  names, host ports (UDP 1502, TCP 1503), test files and evidence directories, so the two
  jobs never collide.
- `ascii`: a Modbus ASCII slave on a serial device (`diagslave -m ascii -a 1 -b 19200 -d 7
  -s 1 -p even /tmp/ttyA`), and `rtu-pty`: a Modbus RTU slave on a serial device
  (`diagslave -m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20 /tmp/ttyA`). Inside the container
  `socat -d -d pty,raw,echo=0,link=/tmp/ttyA pty,raw,echo=0,link=/tmp/ttyB` makes a
  pseudo-terminal pair, diagslave opens `/tmp/ttyA`, and `socat -d -d
  TCP-LISTEN:1504,reuseaddr,fork,max-children=1 FILE:/tmp/ttyB,raw,echo=0` bridges the other
  end to TCP port 1504 (published on host port 1504 for `ascii` and 1505 for `rtu-pty`), so
  `modbus-ascii:tcp://host:port` and `modbus-rtu:tcp://host:port` reach diagslave's serial
  framing. The `interop-ascii` and `interop-rtu-pty` jobs and `mise run interop-ascii` and
  `mise run interop-rtu-pty` (both in `mise run interop`) run them. Containers
  `stonecrab-interop-diagslave-ascii` and `stonecrab-interop-diagslave-rtu-pty`, evidence in
  `diagslave-ascii/` and `diagslave-rtu-pty/`.

- `serial.sh` (slice S59c, not a mode of the image): the same diagslave on a pseudo-terminal
  pair on the host, with no container, for the drivers' own serial carrier. See "Build and run
  (serial port)" below.

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
`interop/libmodbus/`), `iproute2` 6.15.0-1 and `socat` 1.8.0.3-1+deb13u1 by exact version
(socat's version read from the trixie archive on 2026-10-29; a trixie that no longer has
either fails the build step), and the diagslave archive by
SHA-256 (`01c87fe4...f2fe97e`, the archive whose README is revision 3.6 of 2026-10-10). The
download URL, `https://www.modbusdriver.com/downloads/diagslave.tgz`, carries no version, so
the vendor can replace the archive under the same name. When that happens the build fails
with a digest mismatch at the `ADD --checksum` step; it does not run another simulator. To
move the pin, download the new archive, read its `LICENSE-FREE.txt` and `README.txt`, and
record the new checksum here, in `PROVENANCE.toml` and in the Dockerfile. The image takes the
binary of its own architecture (x86_64 or aarch64); the aarch64 binary's banner reads
"diagslave 3.5", and the server log of every run keeps the banner.

## What it serves

Without `-a`, diagslave answers every unit id, which is how the `udp` mode runs it. It keeps
holding registers, input registers, coils and inputs in memory, zero until written, and logs
every request. The tests write the three holding registers 101 to 103 (one-based, 0x1111,
0x2222 and 0x3333) before they read them, so a reply to the wrong request shows as a wrong
value.

### The `enc` mode

diagslave 3.6 documents `-m enc` as "Encapsulated Modbus RTU over TCP" (README.txt, version
3.0). Its start-up banner reads "Protocol configuration: Encapsulated RTU over TCP". What
the RTU tests rely on, all of it observed on this image on 2026-10-29 and none of it
documented by proconX beyond the mode name:

- `-a 1`: the slave address 1. Without `-a` diagslave answers every unit id (the banner reads
  "address = -1"); with it, a frame for another unit (0, 2 and 255 were tried; 2 and 17
  without `-a` are answered) gets no reply and diagslave closes the connection at once. It is
  not silence: the peer answers a unit it does not serve by closing the TCP connection,
  which the driver reports as `Disconnected`. A frame whose CRC is wrong, an unknown function
  code (0x41 and 0x63) and function codes 07 and 17 get the same treatment.
- The areas are in memory, zero until written, and they outlive a connection (they live as
  long as the container). Input registers are the holding registers and discrete inputs are
  the coils: a write to one is read from the other. The tests write the cells they read.
- Functions 01 to 06, 15 and 16 are served as written, with the standard reply frames; a
  read of 125 registers (a reply of 255 bytes) and a write of 123 registers are served. A
  read of zero registers, of more than 125, or that runs past register 65536 gets exception
  02.
- Function codes 20 and 21 (read and write file record, which Stonecrab sends for the
  extended registers) are answered with exception 02, whatever the file and the record: the
  tests use that as the exception a real slave sends. Function 22 and 43/14 are echoed or
  answered, function 23 gets exception 03.
- Connections: more than one at a time are accepted; a connection idle for several seconds is
  not closed (4 s was tried), and the options `-o` (master activity time-out, 3 s) and `-c`
  (connection time-out, 60 s) are left at their defaults.
- The log names the operation (`readHoldingRegisters`, `writeCoils`, `readFileRecord`) with
  the first reference (one-based, like Stonecrab's addresses) and the count. It does not tell
  function code 05 from 15 or 06 from 16: both of a pair log as `writeCoils` or
  `writeHoldingRegisters`. Which codes the driver sends is pinned by `tests/modbus_rtu.rs`
  (FC06 for one register, FC16 for more, FC15 for coils: `FC05 is never sent`), not here.

The nine tests of `modbus_interop_diagslave_rtu` write the values they read, in cells of their
own, over `modbus-rtu:tcp://127.0.0.1:1503`:

| Test | What it shows |
| --- | --- |
| `interop_rtu_holding_registers_are_written_and_read_back` | FC06 and FC16 writes of a `UINT`, a `UDINT` and a `REAL`, read back by FC03 as the same values and as their halves |
| `interop_rtu_input_registers_read_what_diagslave_aliases_to_the_holding_registers` | FC04 reads of cells written through the holding area |
| `interop_rtu_coils_are_written_and_read_back` | ten coil writes, one FC01 read of the pattern |
| `interop_rtu_discrete_inputs_read_what_diagslave_aliases_to_the_coils` | FC02 reads of coils just written |
| `interop_rtu_a_range_selection_is_one_request_and_one_list_of_what_was_written` | `holding-register:501[0..5]:UINT` written and read as one list in one request |
| `interop_rtu_the_longest_frames_are_framed_and_checked_each_way` | a write of 123 registers and a read of 125: requests and replies of 255 bytes |
| `interop_rtu_exception_02_is_a_typed_device_exception_and_the_session_goes_on` | FC20 and FC21 refused with exception 02: `InvalidAddress`, status code 2, and the connection is still good |
| `interop_rtu_a_unit_the_slave_does_not_serve_is_a_closed_connection_not_a_reply` | unit 2: `Disconnected`, and unit 1 on another connection is untouched |
| `interop_rtu_ping_reads_the_ping_address` | ping |

### The `ascii` and `rtu-pty` modes

Observed on this image on 2026-10-29 (colima, an aarch64 binary whose banner reads
"diagslave 3.5"); none of it is documented by proconX beyond the options:

- The start-up banner reads "Protocol configuration: Modbus ASCII" or "Modbus RTU, frame
  tolerance = 20ms", "Slave configuration: address = 1" and "Serial port configuration:
  /tmp/ttyA, 19200, 7, 1, even" (ASCII) or "19200, 8, 1, none" (RTU). The baud rate does not
  throttle a pseudo-terminal, and parity is not checked on one; the settings are recorded
  because a real line would use them. Parity even and 7 data bits are diagslave's ASCII
  defaults; the unit is 1 (`-a 1`).
- Serial framing: one request written as one frame is answered. In RTU, diagslave cuts a
  frame by the gap between characters: with its default tolerance (`-f 0`) a request
  written in pieces of 1 or 4 bytes 10 ms apart gets no reply, with `-f 20` (the largest
  value it takes) it does. Stonecrab writes a frame in one write, so the tests would pass
  with either; `-f 20` guards against a slow runner splitting a frame.
- A frame for a unit it does not serve (unit 2 was tried) gets no reply and no log line, and
  the connection is not closed: the driver's answer is `Timeout`. A request after it is
  answered. This differs from `-m enc`, where diagslave closes the connection.
- ASCII: a frame with a wrong LRC, a frame in lower-case hex and a line with no colon get no
  reply; a frame ended by LF alone, and one with leading noise before the colon, are
  answered (seen in a probe by hand, not asserted by a test). The replies are upper-case hex
  ended by CR LF. A request of 123 registers (FC16) and a read of 125 (FC03) are 511
  characters each way and are served.
- RTU: a request with a wrong CRC gets no reply and the connection stays open (`-m enc`
  closes it).
- Function codes 20 and 21 get exception 02 as in `-m enc`; function code 0x41 (unknown) gets
  exception 01 in `-m ascii` (a probe by hand). Areas, aliases (input registers are the
  holding registers, discrete inputs are the coils) and the log are as in the `enc` mode.
- One master: the bridge's `max-children=1` keeps a second TCP connection waiting in the
  listen backlog until the first is closed. socat takes about 0.5 s (its default `-t`) after
  the client closes to end the child that held the device, so each test costs about that
  much. `serve.sh` stays PID 1, runs the pair, diagslave and the bridge as children and exits
  (the container stops) as soon as one of them ends.
- Readiness: `interop.sh wait` sends an FC03 read of register 0 as an ASCII frame (colon, LRC)
  or an RTU frame (CRC) through the bridge until a valid reply with a right LRC or CRC
  returns.
- The evidence (`image-digest.txt`) lists the `socat` and `diagslave` command lines read from
  `/proc`, the `iproute2` and `socat` package versions, the image id and the pins; the
  `server.log` holds socat's `-d -d` notices, the banner and diagslave's request log.

The nine tests of `modbus_interop_diagslave_ascii` are those of the RTU file with the ASCII
driver: `interop_ascii_holding_registers_are_written_and_read_back`,
`interop_ascii_input_registers_read_what_diagslave_aliases_to_the_holding_registers`,
`interop_ascii_coils_are_written_and_read_back`,
`interop_ascii_discrete_inputs_read_what_diagslave_aliases_to_the_coils`,
`interop_ascii_a_range_selection_is_one_request_and_one_list_of_what_was_written`,
`interop_ascii_the_longest_frames_are_framed_and_checked_each_way` (511 characters),
`interop_ascii_exception_02_is_a_typed_device_exception_and_the_session_goes_on`,
`interop_ascii_a_unit_the_slave_does_not_serve_is_silence` (`Timeout`, then unit 1 still
answers on a new connection) and `interop_ascii_ping_reads_the_ping_address`. The `rtu-pty`
run is the nine tests of `modbus_interop_diagslave_rtu` without
`interop_rtu_a_unit_the_slave_does_not_serve_is_a_closed_connection_not_a_reply` and with
`interop_rtu_a_unit_the_slave_does_not_serve_is_silence`; the `enc` run leaves that one out.
Every value read is compared with the one written.

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

## Build and run (udp)

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
the image id with its base and the archive checksum, the `iproute2` version, the command line
diagslave ran with and the server log.

## Build and run (enc)

```sh
docker build -f interop/diagslave/Dockerfile -t stonecrab-interop-diagslave:local .
docker run -d --name stonecrab-interop-diagslave-rtu \
  -p 127.0.0.1:1503:1503/tcp stonecrab-interop-diagslave:local enc
STONECRAB_INTEROP_DIAGSLAVE_RTU=modbus-rtu:tcp://127.0.0.1:1503 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only --test-threads 1 \
  -E 'binary(modbus_interop_diagslave_rtu) & test(/^tests::interop_/)'
docker rm -f stonecrab-interop-diagslave-rtu
```

`mise run interop-rtu` does all of it, including the check that the filter selects exactly
nine tests, and writes the evidence to `target/interop-evidence/diagslave-rtu/`: the test
output, the image id with its base and the archive checksum, the command line (`mode: enc`,
`command: /usr/local/bin/diagslave -m enc -p 1503 -a 1`) and the server log. The run needs
no `NET_ADMIN` capability and no kernel module.

## Build and run (ascii and rtu-pty)

```sh
docker build -f interop/diagslave/Dockerfile -t stonecrab-interop-diagslave:local .
docker run -d --name stonecrab-interop-diagslave-ascii \
  -p 127.0.0.1:1504:1504/tcp stonecrab-interop-diagslave:local ascii
STONECRAB_INTEROP_DIAGSLAVE_ASCII=modbus-ascii:tcp://127.0.0.1:1504 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only --test-threads 1 \
  -E 'binary(modbus_interop_diagslave_ascii) & test(/^tests::interop_/)'
docker rm -f stonecrab-interop-diagslave-ascii
# RTU on a serial device
docker run -d --name stonecrab-interop-diagslave-rtu-pty \
  -p 127.0.0.1:1505:1504/tcp stonecrab-interop-diagslave:local rtu-pty
STONECRAB_INTEROP_DIAGSLAVE_RTU=modbus-rtu:tcp://127.0.0.1:1505 \
  cargo nextest run -p stonecrab --all-features --run-ignored ignored-only --test-threads 1 \
  -E 'binary(modbus_interop_diagslave_rtu) & test(/^tests::interop_/) & not test(/_closed_connection_not_a_reply$/)'
docker rm -f stonecrab-interop-diagslave-rtu-pty
```

`mise run interop-ascii` and `mise run interop-rtu-pty` do all of it, including the check that
the filter selects exactly nine tests, and write the evidence to
`target/interop-evidence/diagslave-ascii/` and `diagslave-rtu-pty/`. The enc run of
`modbus_interop_diagslave_rtu` has to leave out `/_a_unit_the_slave_does_not_serve_is_silence$/`
the same way (`interop.sh` does). No `NET_ADMIN` capability and no kernel module are needed:
the container makes its own pseudo-terminals.

## Build and run (serial port)

`serial.sh` (slice S59c, ADR-0024 addendum) is the host-side script of the serial-port half:
`modbus-rtu:///path` and `modbus-ascii:///path`, the serial carrier of the drivers with its
`serial.*` options, the silence before a request and the reopen of a lost port, against
diagslave's serial framing. Linux only: the archive holds Linux binaries and macOS refuses the
baud-rate ioctl on a pseudo-terminal. It needs `socat`, `python3` and `tar` on the host, and
`cargo nextest` for the tests.

```sh
interop/diagslave/serial.sh fetch   # download, check the SHA-256, take this machine's binary
INTEROP_DIAGSLAVE_SERIAL_MODE=rtu interop/diagslave/serial.sh run   # or ascii
```

`mise run interop-serial-port` runs both modes, and so does the job of the same name. The
steps are `fetch`, `start`, `wait`, `tests`, `evidence` and `stop` (`run` does all of them and
always stops), and `line-down` and `line-up`, which the lost-port test calls. Everything is in
`${TMPDIR:-/tmp}/stonecrab-interop-diagslave-serial-<uid>` (or `INTEROP_SERIAL_DIR`): the
checked archive, the binary, `ttyA` and `ttyB` (socat's links to the two ends), pid files and
the logs. `stop` ends the processes and removes the links and the pid files; the logs (which
the next `start` truncates), the archive and the binary stay, and the next `fetch` checks the
archive again.

- The archive is the one of the Dockerfile (URL and SHA-256 are read from it); another archive
  stops the run, removes the download and unpacks nothing.
- diagslave runs with the options of `serve.sh`: `-m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20`
  or `-m ascii -a 1 -b 19200 -d 7 -s 1 -p even`, on `ttyA`, with stdin from `/dev/null`.
- The driver end is `ttyB` at `serial.baud-rate=19200&serial.data-bits=8&serial.stop-bits=1&
  serial.parity=none`: a Linux pseudo-terminal forces eight data bits and no parity, so an
  ASCII line of 7E1 cannot be opened on it (`tests/transport_serial.rs`), and what diagslave
  is told about parity changes nothing.
- `wait` writes an FC03 read of register 0 (an RTU frame with its CRC, or an ASCII frame with
  its LRC) to `ttyB` in raw mode and accepts only a valid reply, then reads on for 300 ms so
  that the reply of an earlier probe does not reach the tests.
- `tests` exports the full connection string in `STONECRAB_INTEROP_DIAGSLAVE_RTU` or
  `STONECRAB_INTEROP_DIAGSLAVE_ASCII` (the nine tests of the file, minus the RTU test of `-m
  enc`'s closed connection) and in `STONECRAB_INTEROP_DIAGSLAVE_SERIAL` (the two tests of
  `modbus_interop_diagslave_serial`), and the path of the script in
  `STONECRAB_INTEROP_DIAGSLAVE_SERIAL_CONTROL`; it fails unless the filter selects exactly 11
  tests. The evidence is in `target/interop-evidence/diagslave-serial-rtu/` and
  `diagslave-serial-ascii/`: `serial-evidence.txt` (host, connection string, archive and binary
  checksums, socat and package versions, the command lines read from `/proc`, diagslave's
  banner), `socat.log`, `server.log` and `nextest.txt`.
- `line-down` ends diagslave and then socat, which closes both ends of the pair and removes the
  links; `line-up` makes the pair again at the same paths, starts a new diagslave (its areas
  are zero) and waits for it.

Status: the script and the job `interop-serial-port` have not run as written. Never run end to
end: the `curl` download and the SHA-256 check of the archive, the x86_64 selection, the Python
probe on Linux, Ubuntu's socat and the CI job. The only run was a modified copy of the script
in a scratchpad, in a Linux container of the S60b image on colima (an aarch64 binary whose
banner reads "diagslave 3.5", not the "diagslave 3.6" archive; the copy's download and its
Python probe replaced by the image's binary and a short sleep). It is a note, not evidence.
In that run all nine tests of each framing and both new tests passed; a
second request waited 155 ms at 300 baud (t3.5 is 116.7 ms) and 286 ms with
`serial.interframe-delay=250`; diagslave logged each request as `readHoldingRegisters` or
`writeHoldingRegisters`; socat 1.8.0.3 ended on SIGTERM and made new links when started again.
