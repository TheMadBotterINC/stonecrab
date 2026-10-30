#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Entry point of the open62541 peer image (slice S81). Usage:
#   serve.sh            make the PKI, then run the server (the default)
#   serve.sh check      make the PKI, then run the in-image checks of check.sh
# The server and the PKI take their settings from the environment (see main.c and pki.sh).
set -eu

OPCUA_PKI_DIR="${OPCUA_PKI_DIR:-/pki}"
export OPCUA_PKI_DIR
/usr/local/bin/pki.sh "$OPCUA_PKI_DIR"

case "${1:-serve}" in
  serve)
    exec /usr/local/bin/opcua-server
    ;;
  check)
    exec /usr/local/bin/check.sh
    ;;
  *)
    echo "serve.sh: unknown mode $1 (serve or check)" >&2
    exit 2
    ;;
esac
