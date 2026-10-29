#!/bin/sh
# Runs diagslave 3.6 in the mode the first argument names (slices S61 and S59b): "udp" (the
# default, no argument) is a Modbus UDP slave on port 1502 of every interface; "enc" is a
# Modbus RTU over TCP slave (encapsulated RTU: RTU frames with their CRC on a TCP stream, which
# is what a serial-to-IP gateway speaks) on port 1503, serving unit 1 only (-a 1). Without -a
# diagslave answers every unit id; with it, a frame for another unit ends the connection.
# diagslave keeps its registers in memory (zero until written) and logs each request to
# stdout, which docker logs keeps as evidence.
case "${1:-udp}" in
  udp) exec /usr/local/bin/diagslave -m udp -p 1502 ;;
  enc) exec /usr/local/bin/diagslave -m enc -p 1503 -a 1 ;;
  *) echo "serve.sh: unknown mode '$1' (udp or enc)" >&2; exit 2 ;;
esac
