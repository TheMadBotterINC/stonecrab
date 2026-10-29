#!/bin/sh
# Runs diagslave 3.6 as a Modbus UDP slave on port 1502 of every interface (slice S61). It
# answers every unit id when no -a is given, keeps its registers in memory (zero until written)
# and logs each request to stdout, which docker logs keeps as evidence.
exec /usr/local/bin/diagslave -m udp -p 1502
