#!/usr/bin/env bash
# Usage: interop/pymodbus/tls.sh build|pki|start|wait|tests|evidence|stop|run
#
# Drives the Modbus TCP over TLS interoperability run (slice S71, ADR-0032): the pymodbus
# 3.8.6 server of interop/pymodbus/ in its TLS mode (MBAP inside TLS 1.2 or 1.3, a client
# certificate required), against the driver's `modbus-tcp:tls://` transport. The CI job
# "interop-tls" calls the steps one by one and `mise run interop-tls` (and `mise run
# interop`) calls `run`, so the two do the same thing.
#
#   build     docker build the image of interop/pymodbus/ as stonecrab-interop:local
#   pki       make a throwaway PKI with openssl inside that image (ECDSA P-256): an authority,
#             a server certificate for 127.0.0.1 and localhost, a client certificate and key
#             (the keystore), and a second, unrelated authority; copied out to the PKI dir
#   start     create the container stonecrab-interop-tls on 127.0.0.1:8802, copy the
#             server's certificate, key and authority into it, and start it
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus FC03 read
#             over mutual TLS (python3 and its ssl module)
#   tests     cargo nextest run the three interop_tls_ tests against the server
#   evidence  write the image digest, the certificates' summaries and the server log to the
#             evidence dir (never a key)
#   stop      remove the container and the PKI dir (no error when they are not there)
#   run       build, pki, start, wait, tests, evidence, stop; the exit status is the first
#             failure (the container and the PKI are always removed)
#
# Environment (all optional):
#   INTEROP_TLS_PORT      host port, default 8802
#   INTEROP_EVIDENCE_DIR  evidence directory, default <repo>/target/interop-evidence/tls
#   INTEROP_TLS_PKI_DIR   PKI directory, default <repo>/target/interop-tls-pki (removed by stop)
#
# The tests read STONECRAB_INTEROP_MODBUS_TLS (the connection string up to the `?`) and
# STONECRAB_INTEROP_TLS_PKI (the client side of the PKI); this script sets both. Every key is
# made at run time and removed with the PKI dir; none is committed or kept as evidence.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
IMAGE="stonecrab-interop:local"
NAME="stonecrab-interop-tls"
MAKER="stonecrab-interop-tls-pki"
PORT="${INTEROP_TLS_PORT:-8802}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence/tls}"
PKI="${INTEROP_TLS_PKI_DIR:-$ROOT/target/interop-tls-pki}"
URL="modbus-tcp:tls://127.0.0.1:${PORT}"
# nextest names a test by its module path; the binary name pins the filter to the one file.
FILTER='binary(modbus_interop_tls) & test(/^tests::interop_tls_/)'
EXPECTED_TESTS=3

die() {
  echo "interop-tls: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the TLS interop run needs Docker (see interop/pymodbus/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  DOCKER_BUILDKIT=1 docker build -f "$ROOT/interop/pymodbus/Dockerfile" -t "$IMAGE" "$ROOT"
}

# The openssl commands run in the image as its unprivileged user, in /tmp of a throwaway
# container. Validity runs from a day ago to two days ahead, so a container clock that is a
# little ahead of the host's does not make the certificates "not yet valid". Serial numbers
# are 16 random bytes, the first one forced to 7 so that the number is positive.
pki() {
  need_docker
  docker rm -f "$MAKER" >/dev/null 2>&1 || true
  rm -rf "$PKI"
  mkdir -p "$PKI"
  # The files keep the modes the image's unprivileged user needs (0644, and docker cp
  # preserves them), so the directory is what keeps other local users out of the keys.
  chmod 0700 "$PKI"
  docker run --name "$MAKER" --entrypoint sh "$IMAGE" -c '
set -eu
mkdir -p /tmp/pki/server /tmp/pki/client
cd /tmp/pki
from=$(date -u -d "-1 day" +%Y%m%d%H%M%SZ)
until=$(date -u -d "+2 days" +%Y%m%d%H%M%SZ)
authority() {
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$1.key" -out "$1.pem" -subj "/CN=$2" -not_before "$from" -not_after "$until" \
    -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign 2>/dev/null
}
leaf() {
  openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$1.key" -out "$1.csr" -subj "/CN=stonecrab interop $1" 2>/dev/null
  printf "basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=%s\nsubjectAltName=%s\n" \
    "$2" "$3" >"$1.ext"
  openssl x509 -req -in "$1.csr" -CA ca.pem -CAkey ca.key \
    -set_serial "0x$(openssl rand -hex 16 | sed "s/^./7/")" \
    -not_before "$from" -not_after "$until" -extfile "$1.ext" -out "$1.pem" 2>/dev/null
}
authority ca "stonecrab interop authority"
authority other-ca "stonecrab interop unrelated authority"
leaf server serverAuth "IP:127.0.0.1,DNS:localhost"
leaf client clientAuth "DNS:stonecrab-interop-client"
cp ca.pem server.pem server.key server/
cp ca.pem other-ca.pem client/
cat client.pem client.key >client/keystore.pem
chmod 0755 server client
chmod 0644 server/* client/*
'
  docker cp "$MAKER:/tmp/pki/server" "$PKI/server"
  docker cp "$MAKER:/tmp/pki/client" "$PKI/client"
  docker rm -f "$MAKER" >/dev/null
  for file in server/ca.pem server/server.pem server/server.key client/ca.pem client/other-ca.pem client/keystore.pem; do
    [ -s "$PKI/$file" ] || die "the PKI step did not make $file"
  done
  echo "interop-tls: PKI in $PKI"
}

stop() {
  if command -v docker >/dev/null 2>&1; then
    docker rm -f "$NAME" "$MAKER" >/dev/null 2>&1 || true
  fi
  rm -rf "$PKI"
}

start() {
  need_docker
  [ -s "$PKI/server/server.key" ] || die "no PKI in $PKI; run the pki step first"
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  docker create --name "$NAME" -p "127.0.0.1:${PORT}:8802" "$IMAGE" \
    --port 8802 --tls-cert /app/pki/server.pem --tls-key /app/pki/server.key \
    --tls-ca /app/pki/ca.pem >/dev/null
  docker cp "$PKI/server" "$NAME:/app/pki"
  docker start "$NAME" >/dev/null
}

probe() {
  python3 -I -c '
import socket, ssl, sys
port, pki = int(sys.argv[1]), sys.argv[2]
context = ssl.create_default_context(cafile=pki + "/ca.pem")
context.load_cert_chain(pki + "/keystore.pem")
try:
    with socket.create_connection(("127.0.0.1", port), timeout=1) as raw:
        with context.wrap_socket(raw, server_hostname="127.0.0.1") as s:
            s.settimeout(1)
            s.sendall(bytes.fromhex("000100000006010300000001"))
            data = b""
            while len(data) < 9:
                chunk = s.recv(9 - len(data))
                if not chunk:
                    break
                data += chunk
    sys.exit(0 if len(data) >= 9 and data[7] == 3 else 1)
except (OSError, ssl.SSLError):
    sys.exit(1)
' "$PORT" "$PKI/client"
}

wait_ready() {
  need_docker
  local i=0 deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    if [ "$(docker inspect -f '{{.State.Running}}' "$NAME" 2>/dev/null || echo false)" != "true" ]; then
      docker logs "$NAME" >&2 || true
      die "the pymodbus TLS container is not running"
    fi
    # Docker's published-port proxy accepts a connection before the container listens, so a
    # connect proves nothing. Ready means a real FC03 read over mutual TLS is answered.
    if probe; then
      echo "interop-tls: server ready on 127.0.0.1:${PORT} after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$NAME" >&2 || true
  die "the server did not answer a Modbus read over TLS on 127.0.0.1:${PORT} within 60 s (measured by the shell clock)"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export STONECRAB_INTEROP_MODBUS_TLS="$URL"
  export STONECRAB_INTEROP_TLS_PKI="$PKI/client"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'modbus_interop_tls' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS TLS interop tests, the filter selects $listed"
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  {
    echo "image: $IMAGE"
    docker image inspect "$IMAGE" --format 'image id: {{.Id}}' || return 1
    docker image inspect "$IMAGE" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    echo "base: $(grep -E '^FROM ' "$ROOT/interop/pymodbus/Dockerfile")"
  } >"$EVIDENCE/image-digest.txt" || return 1
  # Certificates only: subject, issuer, validity and names. No key leaves the PKI dir.
  if [ -s "$PKI/server/server.pem" ]; then
    docker run --rm -i --entrypoint sh "$IMAGE" -c \
      'openssl x509 -noout -subject -issuer -dates -ext subjectAltName,extendedKeyUsage' \
      <"$PKI/server/server.pem" >"$EVIDENCE/server-certificate.txt" 2>&1 || true
  fi
  docker logs "$NAME" >"$EVIDENCE/server.log" 2>&1 || true
  echo "interop-tls: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

run_all() {
  need_docker
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f image-digest.txt nextest.txt server-certificate.txt server.log)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (build) || status=$?
  if [ "$status" -eq 0 ]; then (pki) || status=$?; fi
  if [ "$status" -eq 0 ]; then (start) || status=$?; fi
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then (run_tests) || status=$?; fi
  (evidence) || { [ "$status" -ne 0 ] || status=$?; }
  return "$status"
}

case "${1:-}" in
  build) build ;;
  pki) pki ;;
  start) start ;;
  wait) wait_ready ;;
  tests) run_tests ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  *) die "usage: tls.sh build|pki|start|wait|tests|evidence|stop|run" ;;
esac
