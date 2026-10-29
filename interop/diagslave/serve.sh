#!/bin/sh
# Runs diagslave 3.6 in the mode the first argument names (slices S61, S59b and S60b).
#   udp (the default, no argument)  a Modbus UDP slave on port 1502 of every interface
#   enc     a Modbus RTU over TCP slave (encapsulated RTU: RTU frames with their CRC on a TCP
#           stream, which is what a serial-to-IP gateway speaks) on port 1503, serving unit 1
#           only (-a 1). Without -a diagslave answers every unit id; with it, a frame for
#           another unit ends the connection.
#   ascii   a Modbus ASCII slave on a serial device: diagslave -m ascii -a 1 -b 19200 -d 7
#           -s 1 -p even
#   rtu-pty a Modbus RTU slave on a serial device: diagslave -m rtu -a 1 -b 19200 -d 8 -s 1
#           -p none -f 20
#           The device is one end of a pseudo-terminal pair that socat makes; the other end is
#           bridged to TCP port 1504, one connection at a time, so a TCP gateway carrier
#           (modbus-ascii:tcp://, modbus-rtu:tcp://) reaches diagslave's serial framing,
#           which "-m enc" does not use.
# diagslave keeps its registers in memory (zero until written) and logs each request to
# stdout, which docker logs keeps as evidence.

LINE_A=/tmp/ttyA
LINE_B=/tmp/ttyB

# The serial modes: serve.sh stays PID 1, starts the pty pair, diagslave on one end and the
# TCP bridge on the other, and ends (so the container stops) as soon as one of them does.
serial_slave() {
  socat -d -d "pty,raw,echo=0,link=$LINE_A" "pty,raw,echo=0,link=$LINE_B" &
  pair=$!
  # socat creates the links when both devices exist.
  tries=0
  while [ ! -e "$LINE_A" ] || [ ! -e "$LINE_B" ]; do
    tries=$((tries + 1))
    if [ "$tries" -gt 100 ]; then
      echo "serve.sh: socat made no pseudo-terminal pair within 10 s" >&2
      exit 1
    fi
    sleep 0.1
  done
  /usr/local/bin/diagslave "$@" "$LINE_A" &
  slave=$!
  # max-children=1: a serial line has one master, so a second TCP connection waits in the
  # backlog until the first one is closed. -t 0.1 shortens socat's 0.5 s linger after the
  # client closes, so the next connection starts sooner.
  socat -d -d -t 0.1 "TCP-LISTEN:1504,reuseaddr,fork,max-children=1" "FILE:$LINE_B,raw,echo=0" &
  bridge=$!
  trap 'kill "$pair" "$slave" "$bridge" 2>/dev/null; exit 0' TERM INT
  while kill -0 "$pair" 2>/dev/null && kill -0 "$slave" 2>/dev/null && kill -0 "$bridge" 2>/dev/null; do
    sleep 1
  done
  echo "serve.sh: socat or diagslave ended" >&2
  kill "$pair" "$slave" "$bridge" 2>/dev/null
  exit 1
}

case "${1:-udp}" in
  udp) exec /usr/local/bin/diagslave -m udp -p 1502 ;;
  enc) exec /usr/local/bin/diagslave -m enc -p 1503 -a 1 ;;
  ascii) serial_slave -m ascii -a 1 -b 19200 -d 7 -s 1 -p even ;;
  rtu-pty) serial_slave -m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20 ;;
  *) echo "serve.sh: unknown mode '$1' (udp, enc, ascii or rtu-pty)" >&2; exit 2 ;;
esac
