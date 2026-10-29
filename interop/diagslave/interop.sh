#!/usr/bin/env bash
# Usage: interop/diagslave/interop.sh build|start|wait|tests|evidence|stop|run
#
# Drives the UDP interoperability run (slice S61, ADR-0027): diagslave 3.6 as a Modbus UDP slave
# in Docker, with tc netem applied inside the container by the tests. The CI job "interop-udp"
# calls the steps one by one and `mise run interop-udp` (and `mise run interop`) calls `run`, so
# the two do the same thing.
#
#   build     docker build the image of interop/diagslave/ as stonecrab-interop-diagslave:local;
#             the build fails on a diagslave archive whose SHA-256 is not the pinned one
#   start     run it detached as container stonecrab-interop-diagslave on 127.0.0.1:1502/udp,
#             with the NET_ADMIN capability that tc needs, then check that a netem discipline
#             can be put on the container's eth0 and removed (the kernel needs sch_netem)
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus FC03 read over
#             UDP (python3)
#   tests     cargo nextest run the six interop_ tests of modbus_interop_diagslave, one at a time
#   evidence  write the image id, the base, the archive checksum, the iproute2 version and the
#             server log to the evidence dir
#   stop      remove the container (no error when it is not there)
#   run       build, start, wait, tests, evidence, stop; the exit status is the first failure (the
#             container is always stopped; the evidence is written whenever it started)
#
# Docker must forward a published UDP port to the host. Docker Engine on Linux and Docker
# Desktop do; colima with its default ssh port forwarder forwards TCP only, so `wait` fails there
# (its grpc port forwarder, `--port-forwarder grpc`, is documented to forward UDP too).
#
# Environment (all optional):
#   INTEROP_DIAGSLAVE_PORT  host UDP port, default 1502
#   INTEROP_EVIDENCE_DIR    evidence directory of every peer, default <repo>/target/interop-evidence;
#                           this peer writes to its diagslave/ subdirectory
#
# The tests read STONECRAB_INTEROP_DIAGSLAVE; this script sets it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop-diagslave:local"
NAME="stonecrab-interop-diagslave"
PORT="${INTEROP_DIAGSLAVE_PORT:-1502}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave"
URL="modbus-tcp:udp://127.0.0.1:${PORT}"
# nextest names a test by its module path, so the integration test's tests are tests::interop_*;
# the binary name pins the filter to the one interop test file (tests/modbus_interop_diagslave.rs).
FILTER='binary(modbus_interop_diagslave) & test(/^tests::interop_/)'
EXPECTED_TESTS=6

die() {
  echo "interop: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the UDP interop run needs Docker to build and run diagslave (see interop/diagslave/README.md)"
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
  docker run -d --name "$NAME" --cap-add NET_ADMIN -p "127.0.0.1:${PORT}:1502/udp" "$IMAGE" >/dev/null
  # The tests shape the replies with tc netem, which runs on the host's kernel: a kernel without
  # the sch_netem module fails here, before any test.
  { docker exec -u 0 "$NAME" tc qdisc replace dev eth0 root netem delay 0ms &&
    docker exec -u 0 "$NAME" tc qdisc del dev eth0 root; } ||
    die "tc netem does not work in the container: the Docker host's kernel needs the sch_netem module (on Ubuntu, sudo modprobe sch_netem)"
}

probe() {
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

wait_ready() {
  need_docker
  local i=0 deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    if [ "$(docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" != "true" ]; then
      docker logs "$NAME" >&2 || true
      die "the diagslave container is not running"
    fi
    # Ready means a real FC03 read over UDP is answered through the published port.
    if probe; then
      echo "interop: diagslave ready on 127.0.0.1:${PORT}/udp after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  die "diagslave did not answer a Modbus UDP read on 127.0.0.1:${PORT} within 60 s (measured by the shell clock); if the container log shows no request, Docker does not forward UDP to this host (colima's default ssh port forwarder does not; see interop/diagslave/README.md)"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export STONECRAB_INTEROP_DIAGSLAVE="$URL"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'modbus_interop_diagslave' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  # One test at a time: the netem discipline belongs to the container.
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  local package
  package="$(docker exec "$NAME" dpkg-query -W iproute2)" || return 1
  {
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'created: {{.Created}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    grep -E '^FROM ' "$ROOT/interop/diagslave/Dockerfile" | sed 's/^/base: /'
    grep -Eo 'sha256:[0-9a-f]{64}' "$ROOT/interop/diagslave/Dockerfile" | tail -1 | sed 's/^/diagslave archive: /'
    echo "package: $package"
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
