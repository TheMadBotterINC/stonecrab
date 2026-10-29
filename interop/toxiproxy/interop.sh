#!/usr/bin/env bash
# Usage: interop/toxiproxy/interop.sh build|start|wait|capture|tests|check|evidence|stop|run
#
# Drives the failure-path run (slice S52b): the driver against pymodbus and libmodbus through
# Toxiproxy 2.13.0, with a peer that is paused or killed, on a real clock, and an independent
# tshark decode of everything that crossed the proxy. The CI job "interop-toxiproxy" calls the
# steps one by one and `mise run interop-toxiproxy` calls `run`, so the two do the same thing.
#
#   build     build the two peer images (their own interop.sh build), pull Toxiproxy by digest
#             and build the tshark image of interop/toxiproxy/tshark/
#   start     create the Docker network stonecrab-toxi and run on it the two peers (no published
#             port), Toxiproxy with the proxies of toxiproxy.json, and publish on 127.0.0.1 only
#             the API (18474) and the two proxied ports (15020 pymodbus, 15021 libmodbus)
#   wait      probe until 60 s have elapsed (about 1 s apart) with a real Modbus read through each
#             proxy (python3)
#   capture   start tshark in the network namespace of the Toxiproxy container, writing a pcap
#   tests     cargo nextest run the ten interop_toxi_ tests, one at a time
#   check     stop the capture, keep the pcap, and decode it with tshark: fail on a malformed
#             packet, on a client request whose Modbus/TCP header or length is wrong, or on a
#             peer reply that is not Modbus/TCP; also fail when a leg of the traffic is missing
#   evidence  write the image digests and versions, the capture counts and the container logs
#   stop      remove the containers and the network (no error when they are not there)
#   run       build, start, wait, capture, tests, check, evidence, stop; the exit status is the
#             first failure (everything is always stopped; the evidence is written whenever it
#             started)
#
# The tests control Toxiproxy over its HTTP API from the test process and run docker pause, kill
# and start on the pymodbus container, so they need `docker` on PATH. The API is unauthenticated:
# it is published on the loopback of the host and listens, inside the container, on a network
# that holds only the three containers of this run.
#
# Environment (all optional):
#   INTEROP_TOXIPROXY_API_PORT        host port of the API, default 18474
#   INTEROP_TOXIPROXY_PORT            host port of the pymodbus proxy, default 15020
#   INTEROP_TOXIPROXY_LIBMODBUS_PORT  host port of the libmodbus proxy, default 15021
#   INTEROP_EVIDENCE_DIR              evidence directory of all peers, default
#                                     <repo>/target/interop-evidence; this run writes to its
#                                     toxiproxy/ subdirectory
#
# The tests read STONECRAB_INTEROP_TOXIPROXY, STONECRAB_INTEROP_TOXIPROXY_PYMODBUS and
# STONECRAB_INTEROP_TOXIPROXY_LIBMODBUS; this script sets them. The container ports (15020,
# 15021) and the container names are fixed: toxiproxy.json and the tests name them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Toxiproxy 2.13.0 (MIT), the multi-architecture index digest pulled on 2026-10-28.
TOXIPROXY_IMAGE="ghcr.io/shopify/toxiproxy:2.13.0@sha256:5d3dae09bc185dcc5ff6cf43abf6062a4a720aff0c4306a471e773acdcd107aa"
PYMODBUS_IMAGE="stonecrab-interop:local"
LIBMODBUS_IMAGE="stonecrab-interop-libmodbus:local"
TSHARK_IMAGE="stonecrab-interop-tshark:local"
NETWORK="stonecrab-toxi"
PROXY="stonecrab-toxi-proxy"
PYMODBUS="stonecrab-toxi-pymodbus"
LIBMODBUS="stonecrab-toxi-libmodbus"
CAPTURE="stonecrab-toxi-capture"
API_PORT="${INTEROP_TOXIPROXY_API_PORT:-18474}"
PYMODBUS_PORT="${INTEROP_TOXIPROXY_PORT:-15020}"
LIBMODBUS_PORT="${INTEROP_TOXIPROXY_LIBMODBUS_PORT:-15021}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/toxiproxy"
# nextest names a test by its module path, so the integration test's tests are tests::interop_*;
# the binary name pins the filter to the one test file (tests/modbus_interop_toxiproxy.rs).
FILTER='binary(modbus_interop_toxiproxy) & test(/^tests::interop_/)'
EXPECTED_TESTS=10
# The ports tshark decodes as Modbus/TCP: the two proxy listeners and the two peers.
DECODE=(-d tcp.port==15020,mbtcp -d tcp.port==15021,mbtcp -d tcp.port==5020,mbtcp -d tcp.port==1502,mbtcp)

die() {
  echo "interop: $*" >&2
  exit 1
}

need_docker() {
  command -v docker >/dev/null 2>&1 ||
    die "the docker binary is not on PATH; the interop run needs Docker (see interop/toxiproxy/README.md)"
  docker info >/dev/null 2>&1 ||
    die "the Docker daemon is not reachable; start Docker and run again"
}

build() {
  need_docker
  "$ROOT/interop/pymodbus/interop.sh" build
  "$ROOT/interop/libmodbus/interop.sh" build
  docker pull "$TOXIPROXY_IMAGE"
  DOCKER_BUILDKIT=1 docker build -t "$TSHARK_IMAGE" "$ROOT/interop/toxiproxy/tshark"
}

stop() {
  command -v docker >/dev/null 2>&1 || return 0
  docker rm -f "$CAPTURE" "$PROXY" "$PYMODBUS" "$LIBMODBUS" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}

start() {
  need_docker
  stop
  docker network create "$NETWORK" >/dev/null
  docker run -d --name "$PYMODBUS" --network "$NETWORK" "$PYMODBUS_IMAGE" >/dev/null
  docker run -d --name "$LIBMODBUS" --network "$NETWORK" "$LIBMODBUS_IMAGE" >/dev/null
  # -host=0.0.0.0 is the image's default; the API is reachable from the host on loopback only.
  docker run -d --name "$PROXY" --network "$NETWORK" \
    -p "127.0.0.1:${API_PORT}:8474" \
    -p "127.0.0.1:${PYMODBUS_PORT}:15020" \
    -p "127.0.0.1:${LIBMODBUS_PORT}:15021" \
    -v "$ROOT/interop/toxiproxy/toxiproxy.json:/config/toxiproxy.json:ro" \
    "$TOXIPROXY_IMAGE" -host=0.0.0.0 -config=/config/toxiproxy.json >/dev/null
}

# probe PORT REQUEST-HEX REPLY-BYTES FUNCTION: one Modbus exchange through a proxy; succeeds
# when the reply is whole and carries FUNCTION.
probe() {
  python3 -I -c '
import socket, sys
port, request, size, function = int(sys.argv[1]), bytes.fromhex(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
try:
    with socket.create_connection(("127.0.0.1", port), timeout=1) as s:
        s.settimeout(1)
        s.sendall(request)
        data = b""
        while len(data) < size:
            chunk = s.recv(size - len(data))
            if not chunk:
                break
            data += chunk
    sys.exit(0 if len(data) >= size and data[7] == function else 1)
except OSError:
    sys.exit(1)
' "$@"
}

wait_ready() {
  need_docker
  local i=0 deadline=$((SECONDS + 60)) name
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    for name in "$PROXY" "$PYMODBUS" "$LIBMODBUS"; do
      if [ "$(docker inspect -f '{{.State.Running}}' "$name" 2>/dev/null || echo false)" != "true" ]; then
        docker logs "$name" >&2 || true
        die "the container $name is not running"
      fi
    done
    # Docker's published-port proxy and Toxiproxy accept a connection before the peer listens,
    # so a connect proves nothing. Ready means a real read is answered through each proxy:
    # FC03 of one register on pymodbus, FC04 of one input register of unit 17 on libmodbus.
    if probe "$PYMODBUS_PORT" 000100000006010300000001 9 3 &&
      probe "$LIBMODBUS_PORT" 000100000006110401080001 11 4; then
      echo "interop: both proxies answer a Modbus read after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  docker logs "$PROXY" >&2 || true
  die "the proxies did not answer a Modbus read within 60 s (measured by the shell clock)"
}

capture() {
  need_docker
  docker rm -f "$CAPTURE" >/dev/null 2>&1 || true
  # Not --rm: `check` copies the file out after the container has stopped. The container shares
  # the network namespace of Toxiproxy, so eth0 carries every leg: client to proxy, proxy to peer.
  docker run -d --name "$CAPTURE" --network "container:$PROXY" \
    --cap-add NET_RAW --cap-add NET_ADMIN \
    "$TSHARK_IMAGE" -i eth0 -f tcp -w /toxiproxy.pcap -q >/dev/null
  local i
  for i in $(seq 1 30); do
    if docker logs "$CAPTURE" 2>&1 | grep -q 'Capturing on'; then
      return 0
    fi
    sleep 1
  done
  docker logs "$CAPTURE" >&2 || true
  die "tshark did not start capturing within 30 s"
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  export STONECRAB_INTEROP_TOXIPROXY="127.0.0.1:${API_PORT}"
  export STONECRAB_INTEROP_TOXIPROXY_PYMODBUS="modbus-tcp://127.0.0.1:${PYMODBUS_PORT}"
  export STONECRAB_INTEROP_TOXIPROXY_LIBMODBUS="modbus-tcp://127.0.0.1:${LIBMODBUS_PORT}"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'modbus_interop_toxiproxy' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  # One test at a time: a toxic belongs to the proxy, and libmodbus serves one connection.
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

# decode ARGS...: tshark over the kept pcap, with the Modbus/TCP ports; a tshark error is an
# error of this script, never an empty result.
decode() {
  docker run -i --rm "$TSHARK_IMAGE" -r - "${DECODE[@]}" "$@" <"$EVIDENCE/toxiproxy.pcap"
}

# matches NAME FILTER: the number of frames FILTER selects, appended to the counts file.
matches() {
  local name="$1" filter="$2" frames
  frames="$(decode -Y "$filter" -T fields -e frame.number 2>/dev/null)" ||
    die "tshark could not apply the filter for $name: $filter"
  local count
  count="$(printf '%s' "$frames" | grep -c '^[0-9]' || true)"
  printf '%s %s\n' "$count" "$name" >>"$EVIDENCE/capture-check.txt"
  echo "$count"
}

check() {
  need_docker
  mkdir -p "$EVIDENCE"
  docker stop -t 10 "$CAPTURE" >/dev/null || die "the capture container is not running"
  docker cp "$CAPTURE:/toxiproxy.pcap" "$EVIDENCE/toxiproxy.pcap" ||
    die "cannot copy the pcap out of the capture container"
  docker logs "$CAPTURE" >"$EVIDENCE/capture.log" 2>&1 || true
  docker rm -f "$CAPTURE" >/dev/null 2>&1 || true
  : >"$EVIDENCE/capture-check.txt"
  local bad=0 count
  # Nothing may be malformed to the independent decoder (a malformed packet or any
  # expert-info error; severity 8388608 is Error).
  count="$(matches 'malformed packets or expert errors' '_ws.malformed || _ws.expert.severity >= 8388608')"
  [ "$count" = 0 ] || bad=1
  # What the driver sent: every segment to a proxy port carries one whole Modbus/TCP request
  # (protocol id 0, a length field that matches the segment), and a read or a single write
  # (function code 1 to 6) has the length 6.
  count="$(matches 'driver requests that are not one well-formed Modbus/TCP frame' \
    'tcp.len > 0 && tcp.dstport in {15020, 15021} && !(mbtcp && mbtcp.prot_id == 0 && mbtcp.len == tcp.len - 6)')"
  [ "$count" = 0 ] || bad=1
  count="$(matches 'driver read or single-write requests with a length other than 6' \
    'tcp.dstport in {15020, 15021} && modbus.func_code <= 6 && mbtcp.len != 6')"
  [ "$count" = 0 ] || bad=1
  # What the peers sent: every segment from a peer is Modbus/TCP with protocol id 0.
  count="$(matches 'peer segments that are not Modbus/TCP' \
    'tcp.len > 0 && tcp.srcport in {5020, 1502} && !(mbtcp && mbtcp.prot_id == 0)')"
  [ "$count" = 0 ] || bad=1
  # The check must not pass on an empty capture: each leg of each peer has frames.
  local leg
  for leg in 'client to the pymodbus proxy:mbtcp && tcp.dstport == 15020' \
    'client to the libmodbus proxy:mbtcp && tcp.dstport == 15021' \
    'pymodbus replies:mbtcp && tcp.srcport == 5020' \
    'libmodbus replies:mbtcp && tcp.srcport == 1502'; do
    count="$(matches "Modbus/TCP frames, ${leg%%:*}" "${leg#*:}")"
    [ "$count" -gt 0 ] || { echo "interop: the capture holds no ${leg%%:*} frame" >&2; bad=1; }
  done
  cat "$EVIDENCE/capture-check.txt"
  [ "$bad" = 0 ] || die "the capture check failed (see $EVIDENCE/capture-check.txt)"
  echo "interop: the capture check passed"
}

evidence() {
  need_docker
  mkdir -p "$EVIDENCE"
  {
    local image
    for image in "$TOXIPROXY_IMAGE" "$PYMODBUS_IMAGE" "$LIBMODBUS_IMAGE" "$TSHARK_IMAGE"; do
      echo "image: $image"
      docker image inspect "$image" --format 'image id: {{.Id}}' || return 1
      docker image inspect "$image" --format 'repo digests: {{.RepoDigests}}' || return 1
      docker image inspect "$image" --format 'architecture: {{.Os}}/{{.Architecture}}' || return 1
    done
    echo "tshark: $(docker run --rm --entrypoint dpkg-query "$TSHARK_IMAGE" -W tshark)"
    echo "toxiproxy: $(docker logs "$PROXY" 2>&1 | grep -m1 'Starting Toxiproxy' || echo 'no start line in the log')"
  } >"$EVIDENCE/image-digest.txt" || return 1
  docker logs "$PROXY" >"$EVIDENCE/toxiproxy.log" 2>&1 || true
  docker logs "$PYMODBUS" >"$EVIDENCE/pymodbus.log" 2>&1 || true
  docker logs "$LIBMODBUS" >"$EVIDENCE/libmodbus.log" 2>&1 || true
  echo "interop: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

run_all() {
  need_docker
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f capture-check.txt capture.log image-digest.txt libmodbus.log nextest.txt pymodbus.log toxiproxy.log toxiproxy.pcap)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (build) || status=$?
  if [ "$status" -eq 0 ]; then (start) || status=$?; fi
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then (capture) || status=$?; fi
  if [ "$status" -eq 0 ]; then
    (run_tests) || status=$?
    (check) || { [ "$status" -ne 0 ] || status=$?; }
  fi
  (evidence) || { [ "$status" -ne 0 ] || status=$?; }
  return "$status"
}

case "${1:-}" in
  build) build ;;
  start) start ;;
  wait) wait_ready ;;
  capture) capture ;;
  tests) run_tests ;;
  check) check ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  *) die "usage: interop.sh build|start|wait|capture|tests|check|evidence|stop|run" ;;
esac
