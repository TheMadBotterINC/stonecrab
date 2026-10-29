#!/bin/sh
# Run libmodbus 3.1.11's unit-test-server in TCP mode on port 1502, on every interface (the
# server binds INADDR_ANY when the address starts with "0"), and start it again each time it
# exits. The server accepts one connection, serves it until the client closes and then
# returns, so without this loop the second client of a run finds nobody listening.
#
# Function code 07 (read exception status) makes 3.1.11 exit without a reply, so a client
# never sends it. A line buffer keeps the server's trace in the container log as it happens.
set -u

child=
trap 'if [ -n "$child" ]; then kill "$child" 2>/dev/null; fi; exit 0' TERM INT

while :; do
  stdbuf -oL unit-test-server tcp 0.0.0.0 &
  child=$!
  wait "$child"
  echo "serve.sh: unit-test-server exited with status $?; starting it again"
done
