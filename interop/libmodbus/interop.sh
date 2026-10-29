#!/usr/bin/env bash
# Usage: interop/libmodbus/interop.sh build|start|wait|tests|evidence|stop|run
#
# Drives the libmodbus interoperability run (slice S52a): libmodbus 3.1.11's unit-test-server in
# Docker, the second independent peer next to pymodbus (interop/pymodbus/interop.sh). The CI job
# "interop" calls the steps one by one and `mise run interop` calls `run`, so the two do the same
# thing.
#
#   build     docker build the image of interop/libmodbus/ as stonecrab-interop-libmodbus:local
#   start     run it detached as container stonecrab-interop-libmodbus on 127.0.0.1:1502
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus FC04 read (python3)
#   tests     cargo nextest run the ten interop_ tests of modbus_interop_libmodbus, one at a time
#   evidence  write the image digest, the package version and the server log to the evidence dir
#   stop      remove the container (no error when it is not there)
#   run       build, start, wait, tests, evidence, stop; the exit status is the first failure (the
#             container is always stopped; the evidence is written whenever it started)
#
# The server serves one connection and exits; the image's serve.sh starts it again. A probe is a
# connection, so it costs one restart, which is over long before the next client connects. The
# probe and the tests never send function code 07, which makes 3.1.11 exit without replying.
#
# Environment (all optional):
#   INTEROP_LIBMODBUS_PORT  host port, default 1502
#   INTEROP_EVIDENCE_DIR    evidence directory of both peers, default <repo>/target/interop-evidence;
#                           this peer writes to its libmodbus/ subdirectory
#
# The tests read STONECRAB_INTEROP_LIBMODBUS; this script sets it.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop-libmodbus:local"
NAME="stonecrab-interop-libmodbus"
PORT="${INTEROP_LIBMODBUS_PORT:-1502}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/libmodbus"
URL="modbus-tcp://127.0.0.1:${PORT}"
# nextest names a test by its module path, so the integration test's tests are tests::interop_*;
# the binary name pins the filter to the one interop test file (tests/modbus_interop_libmodbus.rs).
FILTER='binary(modbus_interop_libmodbus) & test(/^tests::interop_/)'
EXPECTED_TESTS=10

die() {
  echo "interop: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the interop run needs Docker to build and run the libmodbus server (see interop/libmodbus/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  DOCKER_BUILDKIT=1 docker build -f "$ROOT/interop/libmodbus/Dockerfile" -t "$IMAGE" "$ROOT"
}

stop() {
  command -v docker >/dev/null 2>&1 || return 0
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

start() {
  need_docker
  stop
  docker run -d --name "$NAME" -p "127.0.0.1:${PORT}:1502" "$IMAGE" >/dev/null
}

probe() {
  # Unit 17 reads the input register at wire address 0x108 (0x000A in unit-test.h).
  python3 -I -c '
import socket, sys
try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1) as s:
        s.settimeout(1)
        s.sendall(bytes.fromhex("000100000006110401080001"))
        data = b""
        while len(data) < 11:
            chunk = s.recv(11 - len(data))
            if not chunk:
                break
            data += chunk
    sys.exit(0 if len(data) >= 11 and data[7] == 4 else 1)
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
      die "the libmodbus container is not running"
    fi
    # Docker's published-port proxy accepts a connection before the container listens, so a
    # connect proves nothing. Ready means a real FC04 read of one register is answered.
    if probe; then
      echo "interop: libmodbus server ready on 127.0.0.1:${PORT} after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  die "the server did not answer a Modbus read on 127.0.0.1:${PORT} within 60 s (measured by the shell clock)"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export STONECRAB_INTEROP_LIBMODBUS="$URL"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'modbus_interop_libmodbus' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  # One test at a time: the server serves one connection and a test's writes die with it.
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  local package
  package="$(docker exec "$NAME" dpkg-query -W libmodbus5)" || return 1
  {
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'created: {{.Created}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    grep -E '^FROM ' "$ROOT/interop/libmodbus/Dockerfile" | sed 's/^/base: /'
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
