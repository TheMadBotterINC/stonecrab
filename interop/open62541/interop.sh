#!/usr/bin/env bash
# Usage: interop/open62541/interop.sh build|start|wait|check|pki|tests|evidence|stop|run
#
# Drives the open62541 interoperability run (slice S81): an OPC UA server on open62541 v1.5.9
# in Docker, the independent peer of the OPC UA driver (milestone M3). The CI job
# "interop-opcua" calls the steps one by one and `mise run interop-opcua` calls `run`, so the
# two do the same thing. It is not part of the TCP `interop` aggregate (`mise run interop`)
# until the peer has proved stable.
#
#   build     docker build the image of interop/open62541/ as stonecrab-interop-open62541:local
#   start     run it detached as container stonecrab-interop-open62541, listening on, and
#             advertising, 127.0.0.1:4840 (the port inside is the port outside)
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real OPC UA Hello
#             through the published port: the answer must be an Acknowledge
#   check     run the checks of the image in a container of their own (open62541's own client
#             against servers started from a fresh PKI: None and Basic256Sha256 SignAndEncrypt
#             reads of the counter, the other policies and identities, refusals by trust list,
#             and the README's facts about timeouts and subscriptions); see interop/open62541/check.sh
#   pki       copy the client side of the running container's run-time PKI to the host (the
#             client and user certificates and keys, the untrusted ones, the server
#             certificate and the user password) into target/interop-pki/open62541/, for
#             the Stonecrab tests that run before `stop`
#   evidence  write the image digest, versions and the server log to the evidence dir
#   tests     run the exact-count Stonecrab None/all-modern-policy channel/session/read/write/refusal tests plus the finite example against the running peer
#   stop      remove the container and the `pki` files (no error when they are not there; no
#             key is kept after a run)
#   run       build, start, wait, check, pki, tests, evidence, stop; the exit status is the first
#             failure (the container is always stopped; the evidence is written whenever it
#             started)
#
# Every start of the container makes new keys (interop/open62541/pki.sh); nothing of them is in
# the image or the repository. The `pki` files are the only ones that leave the container, and
# they go to target/, which git ignores, never to the evidence directory; `stop` removes them.
#
# Environment (all optional):
#   INTEROP_OPEN62541_PORT  host and container port, default 4840
#   INTEROP_EVIDENCE_DIR    evidence directory of all peers, default <repo>/target/interop-evidence;
#                           this peer writes to its open62541/ subdirectory
#   INTEROP_PKI_DIR         where `pki` puts the files, default <repo>/target/interop-pki/open62541
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop-open62541:local"
NAME="stonecrab-interop-open62541"
PORT="${INTEROP_OPEN62541_PORT:-4840}"
HOST="127.0.0.1"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/open62541"
PKI_OUT="${INTEROP_PKI_DIR:-$ROOT/target/interop-pki/open62541}"
FILTER='binary(stonecrab) & test(/^opcua::(channel|session|connection|lane|endpoint)::tests::opcua_interop_/)'
EXPECTED_TESTS=14

die() {
  echo "interop: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the interop run needs Docker to build and run the open62541 server (see interop/open62541/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  DOCKER_BUILDKIT=1 docker build -f "$ROOT/interop/open62541/Dockerfile" -t "$IMAGE" "$ROOT"
}

# The files `pki` writes, so that `pki` and `stop` remove only these, never the directory.
PKI_FILES="client.cert.der client.cert.pem client.key.der client.key.pem
expired-client.cert.pem expired-client.key.pem expired-server.cert.der
  untrusted.cert.der untrusted.cert.pem untrusted.key.der untrusted.key.pem
  user.cert.der user.cert.pem user.key.der user.key.pem
  server.cert.der server.cert.pem user-password"

remove_pki() {
  [ -d "$PKI_OUT" ] || return 0
  # Word splitting of PKI_FILES is intended.
  # shellcheck disable=SC2086
  (cd "$PKI_OUT" && rm -f $PKI_FILES)
}

stop() {
  remove_pki
  command -v docker >/dev/null 2>&1 || return 0
  docker rm -f "$NAME" >/dev/null 2>&1 || true
}

start() {
  need_docker
  stop
  docker run -d --name "$NAME" -p "127.0.0.1:${PORT}:${PORT}" \
    -p "127.0.0.1:$((PORT + 1)):$((PORT + 1))" -p "127.0.0.1:$((PORT + 2)):$((PORT + 2))" \
    -p "127.0.0.1:$((PORT + 3)):$((PORT + 3))" \
    -e "OPCUA_PORT=${PORT}" -e "OPCUA_ADVERTISED_HOST=${HOST}" -e OPCUA_ALLOW_NONE_PASSWORD=1 "$IMAGE" >/dev/null
  wait_ready
  docker exec -d -e "OPCUA_PORT=$((PORT + 1))" -e OPCUA_TRUST=empty "$NAME" \
    sh -c '/usr/local/bin/opcua-server > /tmp/empty-trust.log 2>&1'
  docker exec -d -e "OPCUA_PORT=$((PORT + 2))" -e OPCUA_PKI_DIR=/pki/expired-server "$NAME" \
    sh -c '/usr/local/bin/opcua-server > /tmp/expired-server.log 2>&1'
  docker exec -d -e "OPCUA_PORT=$((PORT + 3))" -e OPCUA_ONLY_NONE=1 "$NAME" \
    sh -c '/usr/local/bin/opcua-server > /tmp/none-only.log 2>&1'
  (PORT=$((PORT + 1)); wait_ready)
  (PORT=$((PORT + 2)); wait_ready)
  (PORT=$((PORT + 3)); wait_ready)
}

probe() {
  # A Hello with the endpoint URL; a server that is up answers an Acknowledge ("ACKF").
  python3 -I -c '
import socket, struct, sys
url = ("opc.tcp://%s:%s" % (sys.argv[1], sys.argv[2])).encode()
body = struct.pack("<5I", 0, 65535, 65535, 0, 0) + struct.pack("<i", len(url)) + url
hello = b"HELF" + struct.pack("<I", 8 + len(body)) + body
try:
    with socket.create_connection((sys.argv[1], int(sys.argv[2])), timeout=1) as s:
        s.settimeout(1)
        s.sendall(hello)
        data = b""
        while len(data) < 4:
            chunk = s.recv(4 - len(data))
            if not chunk:
                break
            data += chunk
    sys.exit(0 if data == b"ACKF" else 1)
except OSError:
    sys.exit(1)
' "$HOST" "$PORT"
}

wait_ready() {
  need_docker
  local i=0 deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    if [ "$(docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" != "true" ]; then
      docker logs "$NAME" >&2 || true
      die "the open62541 container is not running"
    fi
    # Docker's published-port proxy accepts a connection before the container listens, so a
    # connect proves nothing. Ready means the Hello is answered with an Acknowledge.
    if probe; then
      echo "interop: open62541 server ready on ${HOST}:${PORT} after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  die "the server did not answer a Hello on ${HOST}:${PORT} within 60 s (measured by the shell clock)"
}

run_check() {
  need_docker
  mkdir -p "$EVIDENCE"
  # A container of its own, with a PKI of its own: the checks start their own servers.
  docker run --rm "$IMAGE" check 2>&1 | tee "$EVIDENCE/check.txt"
  # pipefail makes the pipeline's status the check's; PIPESTATUS keeps it explicit
  return "${PIPESTATUS[0]}"
}

copy_pki() {
  need_docker
  remove_pki
  mkdir -p "$PKI_OUT"
  # tar, not docker cp: the files belong to the container's uid and are mode 600, and the
  # extracted copy belongs to whoever runs this.
  # shellcheck disable=SC2086
  docker exec "$NAME" tar -C /pki -cf - $PKI_FILES | tar -C "$PKI_OUT" -xf -
  echo "interop: client side of the PKI in $PKI_OUT (not committed; git ignores target/)"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export STONECRAB_INTEROP_OPEN62541="opcua://${HOST}:${PORT}?security-policy=NONE"
  export STONECRAB_INTEROP_OPEN62541_PASSWORD_FILE="$PKI_OUT/user-password"
  export STONECRAB_INTEROP_OPEN62541_PKI_DIR="$PKI_OUT"
  export STONECRAB_INTEROP_OPEN62541_EMPTY_TRUST="opcua://${HOST}:$((PORT + 1))"
  export STONECRAB_INTEROP_OPEN62541_EXPIRED_SERVER="opcua://${HOST}:$((PORT + 2))"
  export STONECRAB_INTEROP_OPEN62541_NONE_ONLY="opcua://${HOST}:$((PORT + 3))"
  local listed
  # CI sets CARGO_TERM_COLOR=always; color codes split qualified test names.
  cargo nextest list --locked -p stonecrab --all-features --color never \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline \
    > "$EVIDENCE/selected-tests.txt" || return 1
  listed="$(grep -Ec 'opcua::(channel|session|connection|lane|endpoint)::tests::opcua_interop_' \
    "$EVIDENCE/selected-tests.txt" || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS Stonecrab interop tests, the filter selects $listed"
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt" || return 1
  STONECRAB_URL="$STONECRAB_INTEROP_OPEN62541" STONECRAB_ADDRESS='ns=1;s=scalar/Int32' \
    cargo run --locked -p stonecrab --example opcua_read --features opcua 2>&1 | tee "$EVIDENCE/example.txt" || return 1
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  {
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'created: {{.Created}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    grep -E '^FROM ' "$ROOT/interop/open62541/Dockerfile" | sed 's/^/base: /'
    grep -E 'OPEN62541_(TAG|COMMIT)=' "$ROOT/interop/open62541/Dockerfile" | sed -E 's/^(ENV)? *//; s/ *\\$//; s/^/source: /'
    docker run --rm --entrypoint sh "$IMAGE" -c \
      'dpkg-query -W openssl libssl3t64 | sed "s/^/package: /"; openssl version | sed "s/^/openssl: /"' || return 1
  } >"$EVIDENCE/image-digest.txt" || return 1
  # The server log has no key or password in it; strip the color codes for the reader.
  docker logs "$NAME" 2>&1 | sed -E $'s/\x1b\\[[0-9;]*m//g' >"$EVIDENCE/server.log" || true
  docker exec "$NAME" cat /tmp/empty-trust.log >"$EVIDENCE/empty-trust.log" || true
  docker exec "$NAME" cat /tmp/expired-server.log >"$EVIDENCE/expired-server.log" || true
  docker exec "$NAME" cat /tmp/none-only.log >"$EVIDENCE/none-only.log" || true
  echo "interop: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

run_all() {
  need_docker
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f image-digest.txt check.txt nextest.txt example.txt server.log)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (build) || status=$?
  if [ "$status" -eq 0 ]; then (start) || status=$?; fi
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then (run_check) || status=$?; fi
  if [ "$status" -eq 0 ]; then (copy_pki) || status=$?; fi
  if [ "$status" -eq 0 ]; then (run_tests) || status=$?; fi
  (evidence) || { rc=$?; [ "$status" -ne 0 ] || status=$rc; }
  return "$status"
}

case "${1:-}" in
  build) build ;;
  start) start ;;
  wait) wait_ready ;;
  check) run_check ;;
  pki) copy_pki ;;
  tests) run_tests ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  *) die "usage: interop.sh build|start|wait|check|pki|tests|evidence|stop|run" ;;
esac
