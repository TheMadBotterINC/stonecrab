#!/usr/bin/env bash
# Usage: interop/pymodbus/interop.sh build|start|wait|tests|example|evidence|stop|run
#
# Drives the pymodbus interoperability run (slice S27, ADR-0015 decision 16). The CI job
# "interop" calls the steps one by one and `mise run interop` calls `run`, so the two do
# the same thing.
#
#   build     docker build the image of interop/pymodbus/ as stonecrab-interop:local
#   start     run it detached as container stonecrab-interop on 127.0.0.1:5020
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus FC03 read (python3)
#   tests     cargo nextest run the ten interop_ tests against the server
#   example   cargo run the modbus_tcp_read example against the server (must exit 0)
#   evidence  write the image digest, pip freeze and the server log to the evidence dir
#   stop      remove the container (no error when it is not there)
#   run       build, start, wait, tests, example, evidence, stop; the exit status is the
#             first failure (the container is always stopped; the evidence is written whenever it started)
#
# Environment (all optional):
#   INTEROP_PORT          host port, default 5020
#   INTEROP_EVIDENCE_DIR  evidence directory, default <repo>/target/interop-evidence
#
# The tests read STONECRAB_INTEROP_MODBUS and write the contract report to
# STONECRAB_INTEROP_EVIDENCE_DIR; this script sets both.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop:local"
NAME="stonecrab-interop"
PORT="${INTEROP_PORT:-5020}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}"
URL="modbus-tcp://127.0.0.1:${PORT}"
# nextest names a test by its module path, so the integration test's tests are tests::interop_*.
FILTER='test(/^tests::interop_/)'
EXPECTED_TESTS=10

die() {
  echo "interop: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the interop run needs Docker to build and run the pymodbus server (see interop/pymodbus/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  DOCKER_BUILDKIT=1 docker build -f "$ROOT/interop/pymodbus/Dockerfile" -t "$IMAGE" "$ROOT"
}

stop() {
  command -v docker >/dev/null 2>&1 || return 0
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

start() {
  need_docker
  stop
  docker run -d --name "$NAME" -p "127.0.0.1:${PORT}:5020" "$IMAGE" >/dev/null
}

probe() {
  python3 -I -c '
import socket, sys
try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=1) as s:
        s.settimeout(1)
        s.sendall(bytes.fromhex("000100000006010300000001"))
        data = b""
        while len(data) < 9:
            chunk = s.recv(9 - len(data))
            if not chunk:
                break
            data += chunk
    sys.exit(0 if len(data) >= 9 and data[7] == 3 else 1)
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
      die "the pymodbus container is not running"
    fi
    # Docker's published-port proxy accepts a connection before the container listens, so a
    # connect proves nothing. Ready means a real FC03 read of one register is answered.
    if probe; then
      echo "interop: server ready on 127.0.0.1:${PORT} after ${i} probe(s)"
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
  export STONECRAB_INTEROP_MODBUS="$URL"
  export STONECRAB_INTEROP_EVIDENCE_DIR="$EVIDENCE"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab-modbus --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'interop_pymodbus' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  cargo nextest run --locked -p stonecrab-modbus --all-features \
    --run-ignored ignored-only -E "$FILTER" --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

run_example() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  STONECRAB_URL="$URL" cargo run --locked -p stonecrab --example modbus_tcp_read \
    --features modbus 2>&1 | tee "$EVIDENCE/example.txt"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  {
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'created: {{.Created}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    echo "base: $(grep -E '^FROM ' "$ROOT/interop/pymodbus/Dockerfile")"
  } >"$EVIDENCE/image-digest.txt" || return 1
  docker logs "$NAME" >"$EVIDENCE/server.log" 2>&1 || true
  docker exec -e PIP_NO_CACHE_DIR=1 "$NAME" python -m pip freeze >"$EVIDENCE/pip-freeze.txt" ||
    return 1
  echo "interop: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

run_all() {
  need_docker
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f contract-report.txt example.txt image-digest.txt nextest.txt pip-freeze.txt server.log)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (build) || status=$?
  if [ "$status" -eq 0 ]; then (start) || status=$?; fi
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then
    (run_tests) || status=$?
    (run_example) || { [ "$status" -ne 0 ] || status=$?; }
  fi
  (evidence) || { [ "$status" -ne 0 ] || status=$?; }
  return "$status"
}

case "${1:-}" in
  build) build ;;
  start) start ;;
  wait) wait_ready ;;
  tests) run_tests ;;
  example) run_example ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  *) die "usage: interop.sh build|start|wait|tests|example|evidence|stop|run" ;;
esac
