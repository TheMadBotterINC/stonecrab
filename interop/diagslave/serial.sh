#!/usr/bin/env bash
# Usage: interop/diagslave/serial.sh fetch|start|wait|tests|evidence|stop|run|line-down|line-up
#
# The serial-port half of the diagslave evidence for Modbus RTU and Modbus ASCII (slice S59c,
# ADR-0024). interop.sh reaches diagslave's serial framing through a TCP bridge inside a
# container, so the drivers' serial carrier (the serial.* options, the t3.5 silence on the
# line, the reopen of a lost port) has not met it. Here both ends of a pseudo-terminal pair
# live on the host: socat makes the pair, diagslave (the pinned 3.6 archive, its static Linux
# binary, no container) opens one end as its serial port, and the tests open the other end
# through the serial carrier, modbus-rtu:///<dir>/ttyB or modbus-ascii:///<dir>/ttyB. Linux
# only: diagslave 3.6 ships Linux binaries, and macOS has no binary for it.
#
#   fetch       download the archive of interop/diagslave/Dockerfile, check its SHA-256 against
#               the pin of the Dockerfile before anything is unpacked, and take the binary of
#               this machine's architecture
#   start       stop what runs, fetch, make the pair and start diagslave on its first end
#   wait        probe until 60 s have elapsed with a real FC03 read (an RTU frame with its CRC
#               or an ASCII frame with its LRC) written to the second end
#   tests       cargo nextest run the nine interop_ tests of modbus_interop_diagslave_rtu (not
#               the one that expects diagslave's -m enc closed connection) or
#               modbus_interop_diagslave_ascii, over the serial carrier, and the two of
#               modbus_interop_diagslave_serial; fails unless the connection string is a serial
#               one (modbus-<mode>:///<path>?serial.baud-rate=...) and the filter selects
#               exactly 11
#   evidence    write the versions, the command lines, the connection string and the logs to
#               the evidence directory
#   stop        end diagslave and socat and remove the files of the pair (no error when none
#               runs); the downloaded archive and binary stay in the directory
#   run         start, wait, tests, evidence, stop; the exit status is the first failure
#   line-down   end diagslave and socat, which closes both ends of the pair (the tests call it)
#   line-up     make the pair again, start diagslave and wait until it answers (the tests call it)
#
# Environment (all optional):
#   INTEROP_DIAGSLAVE_SERIAL_MODE  rtu (default) or ascii
#   INTEROP_SERIAL_DIR             the temporary directory of the pair, the archive and the logs,
#                                  default ${TMPDIR:-/tmp}/stonecrab-interop-diagslave-serial-<uid>
#   INTEROP_EVIDENCE_DIR           default <repo>/target/interop-evidence; the evidence of a mode
#                                  is in its diagslave-serial-<mode>/ subdirectory
#
# The pty keeps a line's baud rate and stop bits and forces eight data bits and no parity (the
# kernel's pty_set_termios, see tests/transport_serial.rs), so the driver end uses 19200 8N1 in
# both modes; diagslave is started with the options of serve.sh, which include 7E1 for ASCII, as
# it was in the container: the pty carries the bytes unchanged either way.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SELF="$ROOT/interop/diagslave/serial.sh"
DOCKERFILE="$ROOT/interop/diagslave/Dockerfile"
MODE="${INTEROP_DIAGSLAVE_SERIAL_MODE:-rtu}"
DIR="${INTEROP_SERIAL_DIR:-${TMPDIR:-/tmp}/stonecrab-interop-diagslave-serial-$(id -u)}"
DIR="${DIR%/}"
EVIDENCE="${INTEROP_EVIDENCE_DIR:-$ROOT/target/interop-evidence}/diagslave-serial-${MODE}"
LINE_A="$DIR/ttyA"
LINE_B="$DIR/ttyB"
# The settings of the driver end; the tests build their connection strings on this one.
LINE_SETTINGS="serial.baud-rate=19200&serial.data-bits=8&serial.stop-bits=1&serial.parity=none"
EXPECTED_TESTS=11

die() {
  echo "interop-serial: $*" >&2
  exit 1
}

# The options of serve.sh for the same two modes (rtu is its rtu-pty).
case "$MODE" in
  rtu)
    SLAVE_ARGS=(-m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20)
    URL="modbus-rtu://${LINE_B}?${LINE_SETTINGS}"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE_RTU"
    BINARY="modbus_interop_diagslave_rtu"
    # A closed connection is what "-m enc" answers a unit it does not serve with; on a serial
    # line diagslave is silent, which the other test of the pair shows.
    EXCLUDED=" & not test(/_a_unit_the_slave_does_not_serve_is_a_closed_connection_not_a_reply\$/)"
    PROBE=probe_rtu
    ;;
  ascii)
    SLAVE_ARGS=(-m ascii -a 1 -b 19200 -d 7 -s 1 -p even)
    URL="modbus-ascii://${LINE_B}?${LINE_SETTINGS}"
    VARIABLE="STONECRAB_INTEROP_DIAGSLAVE_ASCII"
    BINARY="modbus_interop_diagslave_ascii"
    EXCLUDED=""
    PROBE=probe_ascii
    ;;
  *) die "INTEROP_DIAGSLAVE_SERIAL_MODE is '$MODE'; it must be rtu or ascii" ;;
esac
FILTER="(binary(${BINARY}) & test(/^tests::interop_/)${EXCLUDED}) | (binary(modbus_interop_diagslave_serial) & test(/^tests::interop_/))"

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

prepare_dir() {
  if [ -L "$DIR" ] || { [ -e "$DIR" ] && { [ ! -d "$DIR" ] || [ ! -O "$DIR" ]; }; }; then
    die "$DIR exists and is not a directory of this user; remove it or set INTEROP_SERIAL_DIR"
  fi
  mkdir -p -m 700 "$DIR"
}

need_tools() {
  [ "$(uname -s)" = Linux ] ||
    die "this run needs Linux: diagslave 3.6 ships Linux binaries only (see interop/diagslave/README.md); CI runs it, the job interop-serial-port"
  local tool
  for tool in socat python3 tar; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH"
  done
}

# The pin of the archive is the last sha256 of the Dockerfile (the base image digest is the first).
pinned_sha() {
  grep -Eo 'sha256:[0-9a-f]{64}' "$DOCKERFILE" | tail -1 | cut -d: -f2
}

pinned_url() {
  grep -Eo 'https://[^ ]*diagslave\.tgz' "$DOCKERFILE" | tail -1
}

fetch() {
  prepare_dir
  local archive="$DIR/diagslave.tgz" pin target
  pin="$(pinned_sha)"
  [ -n "$pin" ] || die "no archive checksum in $DOCKERFILE"
  if [ ! -f "$archive" ] || [ "$(sha256_of "$archive")" != "$pin" ]; then
    command -v curl >/dev/null 2>&1 || die "curl is not on PATH"
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
      --output "$archive" "$(pinned_url)"
  fi
  # Nothing is unpacked or run before this check.
  if [ "$(sha256_of "$archive")" != "$pin" ]; then
    local found
    found="$(sha256_of "$archive")"
    rm -f "$archive" "$DIR/diagslave"
    die "the diagslave archive is not the pinned one (sha256 $found, pin $pin); review the pin by hand, see interop/diagslave/README.md"
  fi
  case "$(uname -m)" in
    x86_64) target=x86_64-linux-gnu ;;
    aarch64 | arm64) target=aarch64-linux-gnu ;;
    *) die "diagslave 3.6 has no binary for $(uname -m)" ;;
  esac
  rm -rf "$DIR/diagslave-unpacked"
  mkdir "$DIR/diagslave-unpacked"
  tar -xzf "$archive" -C "$DIR/diagslave-unpacked" --no-same-owner \
    "diagslave/${target}/diagslave" diagslave/LICENSE-FREE.txt diagslave/README.txt
  install -m 0755 "$DIR/diagslave-unpacked/diagslave/${target}/diagslave" "$DIR/diagslave"
  cp "$DIR/diagslave-unpacked/diagslave/LICENSE-FREE.txt" "$DIR/diagslave-unpacked/diagslave/README.txt" "$DIR/"
  rm -rf "$DIR/diagslave-unpacked"
  echo "interop-serial: diagslave archive sha256 $pin, binary $target"
}

alive() {
  [ -s "$1" ] && kill -0 "$(cat "$1")" 2>/dev/null
}

# End the process whose pid is in $1: SIGTERM, then SIGKILL after 5 s. socat removes its links
# when it ends on SIGTERM.
end_process() {
  local pidfile="$1" pid i=0
  [ -s "$pidfile" ] || return 0
  pid="$(cat "$pidfile")"
  kill "$pid" 2>/dev/null || true
  while kill -0 "$pid" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 50 ]; then
      kill -KILL "$pid" 2>/dev/null || true
      break
    fi
    sleep 0.1
  done
  rm -f "$pidfile"
}

start_line() {
  need_tools
  prepare_dir
  rm -f "$LINE_A" "$LINE_B"
  nohup socat -d -d "pty,raw,echo=0,link=$LINE_A" "pty,raw,echo=0,link=$LINE_B" \
    </dev/null >>"$DIR/socat.log" 2>&1 &
  echo $! >"$DIR/socat.pid"
  disown
  # socat creates the links when both devices exist.
  local tries=0
  while [ ! -e "$LINE_A" ] || [ ! -e "$LINE_B" ]; do
    tries=$((tries + 1))
    alive "$DIR/socat.pid" || die "socat ended before it made a pseudo-terminal pair: $(tail -3 "$DIR/socat.log")"
    [ "$tries" -le 100 ] || die "socat made no pseudo-terminal pair within 10 s"
    sleep 0.1
  done
}

start_slave() {
  # stdin is /dev/null: diagslave is not to wait for a key.
  nohup "$DIR/diagslave" "${SLAVE_ARGS[@]}" "$LINE_A" \
    </dev/null >>"$DIR/diagslave.log" 2>&1 &
  echo $! >"$DIR/diagslave.pid"
  disown
}

stop_all() {
  end_process "$DIR/diagslave.pid"
  end_process "$DIR/socat.pid"
  rm -f "$LINE_A" "$LINE_B"
}

# An RTU frame (unit 1, FC03, address 0, one register; CRC 840A, low byte first) or an ASCII
# frame (:010300000001FB) written to the driver end; a valid reply is the answer. The bytes
# are read with the terminal in raw mode, as the driver does, and the probe then reads on for
# 300 ms so that a reply to an earlier probe does not reach the tests.
probe() {
  python3 -I -c '
import os, select, sys, termios, time, tty
mode, path = sys.argv[1], sys.argv[2]
def crc(data):
    value = 0xFFFF
    for byte in data:
        value ^= byte
        for _ in range(8):
            value = (value >> 1) ^ 0xA001 if value & 1 else value >> 1
    return value
def valid(data):
    if mode == "rtu":
        return len(data) == 7 and data[:3] == bytes([1, 3, 2]) and crc(data[:5]) == data[5] | data[6] << 8
    if not (data.startswith(b":") and data.endswith(b"\r\n")):
        return False
    try:
        body = bytes.fromhex(data[1:-2].decode())
    except ValueError:
        return False
    return len(body) == 6 and body[:3] == bytes([1, 3, 2]) and sum(body) & 0xFF == 0
def done(data):
    return len(data) >= 7 if mode == "rtu" else data.endswith(b"\r\n")
try:
    fd = os.open(path, os.O_RDWR | os.O_NOCTTY)
except OSError:
    sys.exit(1)
try:
    tty.setraw(fd)
    termios.tcflush(fd, termios.TCIOFLUSH)
    os.write(fd, bytes.fromhex("010300000001840a") if mode == "rtu" else b":010300000001FB\r\n")
    data, end = b"", time.monotonic() + 1.0
    while not done(data) and time.monotonic() < end:
        if select.select([fd], [], [], 0.05)[0]:
            data += os.read(fd, 64)
    ok = valid(data)
    settle = time.monotonic() + 0.3
    while time.monotonic() < settle:
        if select.select([fd], [], [], 0.05)[0]:
            os.read(fd, 64)
    sys.exit(0 if ok else 1)
except OSError:
    sys.exit(1)
finally:
    os.close(fd)
' "$1" "$LINE_B"
}

probe_rtu() { probe rtu; }
probe_ascii() { probe ascii; }

wait_ready() {
  local i=0 deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    i=$((i + 1))
    alive "$DIR/socat.pid" || { cat "$DIR/socat.log" >&2 || true; die "socat is not running"; }
    alive "$DIR/diagslave.pid" || { cat "$DIR/diagslave.log" >&2 || true; die "diagslave is not running"; }
    # Ready means a real FC03 read is answered through the second end of the pair.
    if "$PROBE"; then
      echo "interop-serial: diagslave ($MODE) ready on $LINE_B after ${i} probe(s)"
      return 0
    fi
    sleep 1
  done
  cat "$DIR/diagslave.log" >&2 || true
  die "diagslave ($MODE) did not answer a Modbus read on $LINE_B within 60 s (measured by the shell clock)"
}

start() {
  need_tools
  stop
  fetch
  : >"$DIR/socat.log"
  : >"$DIR/diagslave.log"
  start_line
  start_slave
}

run_tests() {
  mkdir -p "$EVIDENCE"
  cd "$ROOT"
  # The nine reused tests do not look at their carrier: this check is what ties them to the
  # serial one (a path after the empty authority, and the serial.* options).
  case "$URL" in
    "modbus-${MODE}:///"*"?serial.baud-rate="*) ;;
    *) die "the connection string '$URL' is not a serial one (modbus-${MODE}:///<path>?serial.baud-rate=...)" ;;
  esac
  export INTEROP_DIAGSLAVE_SERIAL_MODE="$MODE" INTEROP_SERIAL_DIR="$DIR"
  export "$VARIABLE=$URL"
  # The tests of the added behaviour read the same line and call this script to close and
  # reopen it.
  export STONECRAB_INTEROP_DIAGSLAVE_SERIAL="$URL"
  export STONECRAB_INTEROP_DIAGSLAVE_SERIAL_CONTROL="$SELF"
  # A filter that matches nothing must not pass: count the tests it selects first.
  local listed
  listed="$(cargo nextest list --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --message-format oneline | grep -c 'modbus_interop_diagslave' || true)"
  [ "$listed" = "$EXPECTED_TESTS" ] ||
    die "expected $EXPECTED_TESTS interop tests, the filter selects $listed"
  # One test at a time: there is one line and one master.
  cargo nextest run --locked -p stonecrab --all-features \
    --run-ignored ignored-only -E "$FILTER" --test-threads 1 --no-capture 2>&1 | tee "$EVIDENCE/nextest.txt"
}

evidence() {
  mkdir -p "$EVIDENCE"
  {
    echo "mode: $MODE"
    echo "host: $(uname -srm)"
    echo "connection string: $URL"
    echo "diagslave archive: $(pinned_sha)"
    [ ! -f "$DIR/diagslave" ] || echo "diagslave binary sha256: $(sha256_of "$DIR/diagslave")"
    echo "socat: $(socat -V 2>/dev/null | sed -n 2p)"
    if command -v dpkg-query >/dev/null 2>&1; then
      echo "package: $(dpkg-query -W socat 2>/dev/null || echo 'socat (no package)')"
    fi
    echo "python3: $(python3 -I -V 2>&1)"
    # What ran, read from /proc: the command lines of the pair and of diagslave.
    local file pid
    for file in socat.pid diagslave.pid; do
      pid="$(cat "$DIR/$file" 2>/dev/null || true)"
      if [ -n "$pid" ] && [ -r "/proc/$pid/cmdline" ]; then
        echo "command: $(tr '\0' ' ' <"/proc/$pid/cmdline")"
      fi
    done
    grep -m1 -i 'diagslave' "$DIR/diagslave.log" | sed 's/^/banner: /' || true
  } >"$EVIDENCE/serial-evidence.txt"
  cp "$DIR/socat.log" "$EVIDENCE/socat.log" 2>/dev/null || true
  cp "$DIR/diagslave.log" "$EVIDENCE/server.log" 2>/dev/null || true
  echo "interop-serial: evidence in $EVIDENCE"
  ls -1 "$EVIDENCE"
}

stop() {
  [ -d "$DIR" ] || return 0
  stop_all
}

run_all() {
  mkdir -p "$EVIDENCE"
  # Remove only the files this script writes, never the directory itself.
  (cd "$EVIDENCE" && rm -f serial-evidence.txt nextest.txt server.log socat.log)
  trap stop EXIT
  local status=0
  # Subshells, because die exits: a failed step must still reach the evidence step.
  (start) || status=$?
  if [ "$status" -eq 0 ]; then (wait_ready) || status=$?; fi
  if [ "$status" -eq 0 ]; then (run_tests) || status=$?; fi
  (evidence) || { [ "$status" -ne 0 ] || status=$?; }
  return "$status"
}

case "${1:-}" in
  fetch) fetch ;;
  start) start ;;
  wait) wait_ready ;;
  tests) run_tests ;;
  evidence) evidence ;;
  stop) stop ;;
  run) run_all ;;
  line-down) stop_all ;;
  line-up)
    start_line
    start_slave
    wait_ready
    ;;
  *) die "usage: serial.sh fetch|start|wait|tests|evidence|stop|run|line-down|line-up" ;;
esac
