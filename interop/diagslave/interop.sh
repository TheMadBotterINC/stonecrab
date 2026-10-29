#!/usr/bin/env bash
# Usage: interop/diagslave/interop.sh build|start|wait|tests|evidence|stop|run
#
# Drives the diagslave interoperability runs, which differ in the mode diagslave serves
# (INTEROP_DIAGSLAVE_MODE, below):
#   udp (default)  Modbus UDP (slice S61, ADR-0027), with tc netem applied inside the container
#                  by the tests; the CI job "interop-udp" and `mise run interop-udp`
#   enc            Modbus RTU encapsulated in TCP, the stream of a serial-to-IP gateway (slice
#                  S59b, ADR-0024); the CI job "interop-rtu" and `mise run interop-rtu` (and
#                  `mise run interop`)
#   ascii          Modbus ASCII on a serial device, a pseudo-terminal that socat bridges to a TCP
#                  port (slice S60b, ADR-0024); the CI job "interop-ascii" and `mise run
#                  interop-ascii` (and `mise run interop`)
#   rtu-pty        Modbus RTU on the same kind of device (slice S60b); the CI job
#                  "interop-rtu-pty" and `mise run interop-rtu-pty` (and `mise run interop`)
# The CI jobs call the steps one by one and the mise tasks call `run`, so the two do the same
# thing.
#
#   build     docker build the image of interop/diagslave/ as stonecrab-interop-diagslave:local;
#             the build fails on a diagslave archive whose SHA-256 is not the pinned one
#   start     run it detached (udp: container stonecrab-interop-diagslave on 127.0.0.1:1502/udp,
#             with the NET_ADMIN capability that tc needs, then check that a netem discipline
#             can be put on the container's eth0 and removed (the kernel needs sch_netem);
#             enc: container stonecrab-interop-diagslave-rtu on 127.0.0.1:1503/tcp;
#             ascii: container stonecrab-interop-diagslave-ascii on 127.0.0.1:1504/tcp;
#             rtu-pty: container stonecrab-interop-diagslave-rtu-pty on 127.0.0.1:1505/tcp)
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus FC03 read (python3):
#             over UDP, an RTU frame with its CRC over TCP (enc and rtu-pty) or an ASCII frame
#             with its LRC over TCP (ascii)
#   tests     cargo nextest run the interop_ tests of modbus_interop_diagslave (6, udp),
#             modbus_interop_diagslave_rtu (9, enc; 9 again, one of them another, in rtu-pty) or
#             modbus_interop_diagslave_ascii (9, ascii), one at a time
#   evidence  write the image id, the base, the archive checksum, the iproute2 and socat
#             versions, the command lines diagslave and socat run with and the server log to
#             the evidence dir
#   stop      remove the container (no error when it is not there)
#   run       build, start, wait, tests, evidence, stop; the exit status is the first failure (the
#             container is always stopped; the evidence is written whenever it started)
#
# The udp mode needs a Docker that forwards a published UDP port to the host. Docker Engine on
# Linux and Docker Desktop do; colima with its default ssh port forwarder forwards TCP only, so
# `wait` fails there (its grpc port forwarder, `--port-forwarder grpc`, is documented to forward
# UDP too). The enc, ascii and rtu-pty modes are TCP and work everywhere. The ascii and rtu-pty
# modes have one master, like a serial line: the container's TCP bridge serves one connection at a
# time and the next one waits until the first is closed.
#
# Environment (all optional):
#   INTEROP_DIAGSLAVE_MODE          udp (default), enc, ascii or rtu-pty
#   INTEROP_DIAGSLAVE_PORT          host UDP port of the udp mode, default 1502
#   INTEROP_DIAGSLAVE_RTU_PORT      host TCP port of the enc mode, default 1503
#   INTEROP_DIAGSLAVE_ASCII_PORT    host TCP port of the ascii mode, default 1504
#   INTEROP_DIAGSLAVE_RTU_PTY_PORT  host TCP port of the rtu-pty mode, default 1505
#   INTEROP_EVIDENCE_DIR            evidence directory of every peer, default
#                                   <repo>/target/interop-evidence; the udp mode writes to its
#                                   diagslave/ subdirectory, the enc mode to diagslave-rtu/, the
#                                   ascii mode to diagslave-ascii/ and the rtu-pty mode to
#                                   diagslave-rtu-pty/
#
# The udp tests read STONECRAB_INTEROP_DIAGSLAVE, the enc and rtu-pty tests
# STONECRAB_INTEROP_DIAGSLAVE_RTU and the ascii tests STONECRAB_INTEROP_DIAGSLAVE_ASCII; this
# script sets them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop-diagslave:local"
MODE="${INTEROP_DIAGSLAVE_MODE:-udp}"

die() {
  echo "interop: $*" >&2
  exit 1
}

# nextest names a test by its module path, so the integration test's tests are tests::interop_*;
# the binary name pins the filter to the one interop test file of the mode.
case "$MODE" in
  udp)
    NAME="stonecrab-interop-diagslave"
    PORT="${INTEROP_DIAGSLAVE_PORT:-1502}"
    PUBLISH="127.0.0.1:${PORT}:1502/udp"
    EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE"
    URL="modbus-tcp:udp://127.0.0.1:${PORT}"
    PROBE=probe_udp
    BINARY="modbus_interop_diagslave"
    EXPECTED_TESTS=6
    ;;
  enc)
    NAME="stonecrab-interop-diagslave-rtu"
    PORT="${INTEROP_DIAGSLAVE_RTU_PORT:-1503}"
    PUBLISH="127.0.0.1:${PORT}:1503/tcp"
    EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave-rtu"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE_RTU"
    URL="modbus-rtu:tcp://127.0.0.1:${PORT}"
    PROBE=probe_enc
    BINARY="modbus_interop_diagslave_rtu"
    EXPECTED_TESTS=9
    # A closed connection is what "-m enc -a 1" answers a unit it does not serve with; on a
    # serial device diagslave is silent, which the other test of the pair shows.
    EXCLUDED="test(/_a_unit_the_slave_does_not_serve_is_silence$/)"
    ;;
  ascii)
    NAME="stonecrab-interop-diagslave-ascii"
    PORT="${INTEROP_DIAGSLAVE_ASCII_PORT:-1504}"
    PUBLISH="127.0.0.1:${PORT}:1504/tcp"
    EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave-ascii"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE_ASCII"
    URL="modbus-ascii:tcp://127.0.0.1:${PORT}"
    PROBE=probe_ascii
    BINARY="modbus_interop_diagslave_ascii"
    EXPECTED_TESTS=9
    ;;
  rtu-pty)
    NAME="stonecrab-interop-diagslave-rtu-pty"
    PORT="${INTEROP_DIAGSLAVE_RTU_PTY_PORT:-1505}"
    PUBLISH="127.0.0.1:${PORT}:1504/tcp"
    EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave-rtu-pty"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE_RTU"
    URL="modbus-rtu:tcp://127.0.0.1:${PORT}"
    PROBE=probe_enc
    BINARY="modbus_interop_diagslave_rtu"
    EXPECTED_TESTS=9
    EXCLUDED="test(/_a_unit_the_slave_does_not_serve_is_a_closed_connection_not_a_reply$/)"
    ;;
  *) die "INTEROP_DIAGSLAVE_MODE is '$MODE'; it must be udp, enc, ascii or rtu-pty" ;;
esac
FILTER="binary(${BINARY}) & test(/^tests::interop_/)"
[ -z "${EXCLUDED:-}" ] || FILTER="${FILTER} & not ${EXCLUDED}"

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the diagslave interop run needs Docker to build and run diagslave (see interop/diagslave/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  DOCKER_BUILDKIT=1 docker build -f "$ROOT/interop/diagslave/Dockerfile" -t "$IMAGE" "$ROOT"
}

stop() {
  command -v docker >/dev/null 2>&1 || return 0
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

start() {
  need_docker
  stop
  if [ "$MODE" = udp ]; then
    docker run -d --name "$NAME" --cap-add NET_ADMIN -p "$PUBLISH" "$IMAGE" "$MODE" >/dev/null
    # The tests shape the replies with tc netem, which runs on the host's kernel: a kernel
    # without the sch_netem module fails here, before any test.
    { docker exec -u 0 "$NAME" tc qdisc replace dev eth0 root netem delay 0ms &&
      docker exec -u 0 "$NAME" tc qdisc del dev eth0 root; } ||
      die "tc netem does not work in the container: the Docker host's kernel needs the sch_netem module (on Ubuntu, sudo modprobe sch_netem)"
  else
    docker run -d --name "$NAME" -p "$PUBLISH" "$IMAGE" "$MODE" >/dev/null
  fi
}

probe_udp() {
  # Unit 1 reads holding register 0 over UDP: an answer is an MBAP reply of function 03.
  python3 -I -c '
import socket, sys
try:
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.settimeout(1)
        s.connect(("127.0.0.1", int(sys.argv[1])))
        s.send(bytes.fromhex("000100000006010300000001"))
        data = s.recv(260)
    sys.exit(0 if len(data) >= 11 and data[7] == 3 else 1)
except OSError:
    sys.exit(1)
' "$PORT"
}

probe_enc() {
  # Unit 1 reads holding register 0 as an RTU frame (010300000001, CRC 840A, low byte first)
  # on a TCP stream: an answer is a frame of five data bytes, from unit 1, function 03 and
  # a byte count of 2, with the CRC-16 the reply carries right.
  python3 -I -c '
import socket, sys
def crc(data):
    value = 0xFFFF
    for byte in data:
        value ^= byte
        for _ in range(8):
            value = (value >> 1) ^ 0xA001 if value & 1 else value >> 1
    return value
try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1) as s:
        s.settimeout(1)
        s.sendall(bytes.fromhex("010300000001840a"))
        data = b""
        while len(data) < 7:
            more = s.recv(7 - len(data))
            if not more:
                break
            data += more
    ok = len(data) == 7 and data[:3] == bytes([1, 3, 2]) and crc(data[:5]) == data[5] | data[6] << 8
    sys.exit(0 if ok else 1)
except OSError:
    sys.exit(1)
' "$PORT"
}

probe_ascii() {
  # Unit 1 reads holding register 0 as an ASCII frame (:010300000001FB, LRC FB) on a TCP stream
  # that socat bridges to diagslave's serial device: an answer is a colon, the hex of unit 1,
  # function 03, a byte count of 2, two data bytes and the LRC, then CR LF.
  python3 -I -c '
import socket, sys
try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1) as s:
        s.settimeout(1)
        s.sendall(b":010300000001FB\r\n")
        data = b""
        while not data.endswith(b"\r\n"):
            more = s.recv(64)
            if not more:
                break
            data += more
    body = bytes.fromhex(data[1:-2].decode()) if data.startswith(b":") and data.endswith(b"\r\n") else b""
    ok = len(body) == 6 and body[:3] == bytes([1, 3, 2]) and sum(body) & 0xFF == 0
    sys.exit(0 if ok else 1)
except (OSError, ValueError):
    sys.exit(1)
' "$PORT"
}

wait_ready() {
  need_docker
  local i=0 deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    if [ "$(docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" != "true" ]; then
      docker logs "$NAME" >&2 || true
      die "the diagslave container is not running"
    fi
    # Ready means a real FC03 read is answered through the published port.
    if "$PROBE"; then
      echo "interop: diagslave ($MODE) ready on 127.0.0.1:${PORT} after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  die "diagslave ($MODE) did not answer a Modbus read on 127.0.0.1:${PORT} within 60 s (measured by the shell clock); if the container log shows no request and the mode is udp, Docker does not forward UDP to this host (colima's default ssh port forwarder does not; see interop/diagslave/README.md)"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export "$VARIABLE=$URL"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c "$BINARY" || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  # One test at a time: the netem discipline of the udp tests belongs to the container.
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  local packages commands
  packages="$(docker exec "$NAME" dpkg-query -W iproute2 socat)" || return 1
  # The command lines diagslave and socat run with (serve.sh is PID 1 in the serial modes and
  # execs diagslave itself in the others): what ran, read from /proc.
  commands="$(docker exec "$NAME" sh -c 'for p in /proc/[0-9]*; do c="$(tr "\0" " " <"$p/cmdline" 2>/dev/null)"; case "$c" in /usr/local/bin/diagslave*|socat*) echo "command: $c";; esac; done' | awk '!seen[$0]++')" || return 1
  {
    echo "mode: $MODE"
    echo "$commands"
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'created: {{.Created}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    grep -E '^FROM ' "$ROOT/interop/diagslave/Dockerfile" | sed 's/^/base: /'
    grep -Eo 'sha256:[0-9a-f]{64}' "$ROOT/interop/diagslave/Dockerfile" | tail -1 | sed 's/^/diagslave archive: /'
    echo "$packages" | sed 's/^/package: /'
  } >"$EVIDENCE/image-digest.txt" || return 1
  docker logs "$NAME" >"$EVIDENCE/server.log" 2>&1 || true
  echo "interop: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

run_all() {
  need_docker
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f image-digest.txt nextest.txt server.log)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (build) || status=$?
  if [ "$status" -eq 0 ]; then (start) || status=$?; fi
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then (run_tests) || status=$?; fi
  (evidence) || { [ "$status" -ne 0 ] || status=$?; }
  return "$status"
}

case "${1:-}" in
  build) build ;;
  start) start ;;
  wait) wait_ready ;;
  tests) run_tests ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  *) die "usage: interop.sh build|start|wait|tests|evidence|stop|run" ;;
esac
