#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# The in-image checks of the open62541 peer (slice S81), run by `serve.sh check` and by
# `interop.sh check` before any Stonecrab code is involved: open62541's own client, built from
# client_check.c, against servers started here from the PKI that serve.sh just made. It proves
# the peer is usable (the server answers with each policy and identity) and that its trust
# lists work (an untrusted certificate is refused, and so is every certificate when the list
# is empty). Output is one line per check, "ok" or "FAIL", then a summary; the exit status is
# non-zero if any check failed.
# The option strings below are split into words on purpose.
# shellcheck disable=SC2086
set -u

PKI="${OPCUA_PKI_DIR:-/pki}"
CLIENT=/usr/local/bin/client_check
SERVER=/usr/local/bin/opcua-server
LOGS="${TMPDIR:-/tmp}"
failed=0
pid=

start_server() { # PORT [VAR=VALUE ...]
  port=$1
  shift
  env OPCUA_PORT="$port" OPCUA_PKI_DIR="$PKI" "$@" "$SERVER" >"$LOGS/server-$port.log" 2>&1 &
  pid=$!
  i=0
  while [ "$i" -lt 60 ]; do
    if "$CLIENT" "opc.tcp://localhost:$port" None None >/dev/null 2>&1; then
      # main.c reports a node it could not add as a line "main: ..."
      if grep -aq '^main: ' "$LOGS/server-$port.log"; then
        echo "FAIL server on port $port: $(grep -a '^main: ' "$LOGS/server-$port.log" | tr '\n' ' ')"
        failed=$((failed + 1))
      fi
      return 0
    fi
    kill -0 "$pid" 2>/dev/null || break
    i=$((i + 1))
    sleep 1
  done
  echo "FAIL server on port $port did not answer within 60 s"
  cat "$LOGS/server-$port.log"
  failed=$((failed + 1))
  return 1
}

stop_server() {
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait "$pid" 2>/dev/null
  pid=
}

# expect_ok NAME PORT POLICY MODE [client options]
expect_ok() {
  name=$1 port=$2
  shift 2
  if out="$("$CLIENT" "opc.tcp://localhost:$port" "$@" 2>&1)"; then
    echo "ok   $name: $(echo "$out" | grep '^read:')"
  else
    echo "FAIL $name: $(echo "$out" | grep -v '^\[20' | tail -3)"
    failed=$((failed + 1))
  fi
}

# expect_refused NAME PORT POLICY MODE [client options]: the connect must be refused
expect_refused() {
  name=$1 port=$2
  shift 2
  if out="$("$CLIENT" "opc.tcp://localhost:$port" "$@" 2>&1)"; then
    echo "FAIL $name: connected, but it must be refused: $out"
    failed=$((failed + 1))
  elif echo "$out" | grep -q '^connect: refused: '; then
    echo "ok   $name: $(echo "$out" | grep '^connect: refused: ')"
  else
    echo "FAIL $name: failed, but not as a refusal: $out"
    failed=$((failed + 1))
  fi
}

# expect_output NAME PORT REGEX POLICY MODE [client options]: the client's own lines (not the
# library's log) must match REGEX, whatever the exit status; for facts about the peer
expect_output() {
  name=$1 port=$2 regex=$3
  shift 3
  out="$("$CLIENT" "opc.tcp://localhost:$port" "$@" 2>&1 | LC_ALL=C grep -av '^\[20' | LC_ALL=C tr '\n' ' ')"
  if echo "$out" | grep -Eq "$regex"; then
    echo "ok   $name: $(echo "$out" | cut -c1-110)"
  else
    echo "FAIL $name: wanted /$regex/, got: $out"
    failed=$((failed + 1))
  fi
}

SECURE="--cert $PKI/client.cert.der --key $PKI/client.key.der --trust $PKI/server.cert.der"
UNTRUSTED="--cert $PKI/untrusted.cert.der --key $PKI/untrusted.key.der --trust $PKI/server.cert.der --app-uri urn:stonecrab:interop:untrusted"

echo "check: server with the PKI's trust list"
if start_server 4850; then
  expect_ok "None, anonymous" 4850 None None
  expect_ok "Basic256Sha256 SignAndEncrypt" 4850 Basic256Sha256 SignAndEncrypt $SECURE
  expect_ok "Basic256Sha256 Sign" 4850 Basic256Sha256 Sign $SECURE
  expect_ok "Aes128Sha256RsaOaep SignAndEncrypt" 4850 Aes128Sha256RsaOaep SignAndEncrypt $SECURE
  expect_ok "Aes256Sha256RsaPss SignAndEncrypt" 4850 Aes256Sha256RsaPss SignAndEncrypt $SECURE
  expect_ok "Basic256Sha256 SignAndEncrypt, user name" 4850 Basic256Sha256 SignAndEncrypt $SECURE \
    --user stonecrab --password-file "$PKI/user-password"
  expect_ok "Basic256Sha256 SignAndEncrypt, X.509 token" 4850 Basic256Sha256 SignAndEncrypt $SECURE \
    --x509 "$PKI/user.cert.der" "$PKI/user.key.der"
  expect_refused "Basic256Sha256 SignAndEncrypt, X.509 token not in the user trust list" 4850 \
    Basic256Sha256 SignAndEncrypt $SECURE --x509 "$PKI/untrusted.cert.der" "$PKI/untrusted.key.der"
  expect_refused "Basic256Sha256 SignAndEncrypt, untrusted client certificate" 4850 \
    Basic256Sha256 SignAndEncrypt $UNTRUSTED
  expect_refused "Basic256Sha256 SignAndEncrypt, client trusting another certificate than the server's" 4850 \
    Basic256Sha256 SignAndEncrypt --cert "$PKI/client.cert.der" --key "$PKI/client.key.der" --trust "$PKI/client.cert.der"
  # The test namespace: values of the built-in types, the shapes, a write and the structure.
  expect_output "40 KB array in 64 KB chunks" 4850 '(^| )read: large/Int32_10000 = .*Value: \[0, 1, 2' None None --read large/Int32_10000
  expect_output "scalar/Int32" 4850 '= \{ +UaType: 6, +Value: -123456789 \}' None None --read scalar/Int32
  expect_output "scalar/Guid" 4850 '72962B91-FA75-4AE6-8D28-B404DC7DAF63' None None --read scalar/Guid
  expect_output "matrix/Int32_2x3 has dimensions 2x3" 4850 'Dimensions: \[2, 3\]' None None --read matrix/Int32_2x3
  expect_output "matrix/Double_2x2x2 has dimensions 2x2x2" 4850 'Dimensions: \[2, 2, 2\]' None None --read matrix/Double_2x2x2
  expect_output "array/String has three elements" 4850 '"alpha", +"beta", +"gamma"' None None --read array/String
  expect_output "structure is an extension object of encoding ns=1;i=5002" 4850 'UaTypeId: "ns=1;i=5002"' None None --read structure
  expect_output "writable/Int32 takes a write" 4850 '(^| )write: Good.*= \{ +UaType: 6, +Value: 31337 \}' \
    None None --read writable/Int32 --write-int32 31337
  expect_output "scalar/Int32 refuses a write (BadUserAccessDenied)" 4850 '(^| )write: BadUserAccessDenied' None None --read scalar/Int32 --write-int32 1
  # What the README states about the peer's sessions and subscriptions (the maximum session
  # timeout of this server is 5000 ms).
  expect_output "session timeout: 1000 ms is honored, alive after 300 ms" 4850 '(^| )read: counter' \
    None None --session-timeout 1000 --hold-ms 300
  expect_output "session timeout: 1000 ms is honored, gone after 3000 ms" 4850 '(^| )read: failed: Bad(SessionIdInvalid|SessionClosed)' \
    None None --session-timeout 1000 --hold-ms 3000
  expect_output "session timeout: 100 ms is honored, gone after 300 ms" 4850 '(^| )read: failed: Bad(SessionIdInvalid|SessionClosed)' \
    None None --session-timeout 100 --hold-ms 300
  expect_output "session timeout: 60000 ms is revised to the 5000 ms ceiling" 4850 '(^| )read: failed: Bad(SessionIdInvalid|SessionClosed)' \
    None None --session-timeout 60000 --hold-ms 7000
  expect_output "TransferSubscriptions on Basic256Sha256 SignAndEncrypt, anonymous" 4850 '(^| )transfer: Good' \
    Basic256Sha256 SignAndEncrypt $SECURE --probe-transfer
  expect_output "TransferSubscriptions with a user name token" 4850 '(^| )transfer: Good' \
    Basic256Sha256 SignAndEncrypt $SECURE --user stonecrab --password-file "$PKI/user-password" --probe-transfer
  expect_output "TransferSubscriptions on None, anonymous, is denied" 4850 '(^| )transfer: BadUserAccessDenied' \
    None None --probe-transfer
  expect_output "Republish is dispatched (the message was acknowledged long ago)" 4850 '(^| )republish: BadMessageNotAvailable' \
    None None --probe-transfer
  stop_server
fi

echo "check: server with a fast event source"
if start_server 4853 OPCUA_EVENT_PERIOD_MS=500; then
  expect_output "events of events/Source arrive" 4853 '(^| )events: [1-9]' None None --probe-events 3
  stop_server
fi

echo "check: server advertising another host"
if start_server 4854 OPCUA_ADVERTISED_HOST=interop.example; then
  # GetEndpoints answers with the URL the client used; FindServers lists the advertised one.
  expect_output "GetEndpoints: seven endpoints (None; three policies, Sign and SignAndEncrypt)" 4854 '(^| )endpoints: 7' \
    None None --probe-endpoints
  expect_output "GetEndpoints repeats the URL the client connected to" 4854 'endpoint: opc.tcp://localhost:4854 mode=1 ' \
    None None --probe-endpoints
  expect_output "FindServers lists the advertised host and port" 4854 'discovery: opc.tcp://interop.example:4854' \
    None None --probe-endpoints
  stop_server
fi

echo "check: server with an empty trust list"
if start_server 4851 OPCUA_TRUST=empty; then
  expect_refused "Basic256Sha256 SignAndEncrypt, empty trust list" 4851 Basic256Sha256 SignAndEncrypt $SECURE
  expect_ok "None, empty trust list (no certificate is checked)" 4851 None None
  stop_server
fi

echo "check: server with small chunks"
if start_server 4852 OPCUA_SMALL_CHUNKS=1; then
  expect_ok "Basic256Sha256 SignAndEncrypt, 8192-byte chunks" 4852 Basic256Sha256 SignAndEncrypt $SECURE
  expect_output "16 KB array crosses chunks (None, 8192-byte chunks)" 4852 '(^| )read: large/Int32_4000 = .*Value: \[0, 1, 2' \
    None None --read large/Int32_4000
  expect_output "16 KB array crosses chunks (Basic256Sha256 SignAndEncrypt)" 4852 '(^| )read: large/Int32_4000 = .*Value: \[0, 1, 2' \
    Basic256Sha256 SignAndEncrypt $SECURE --read large/Int32_4000
  expect_output "40 KB array is over the message limit" 4852 '(^| )read: failed: Bad' None None --read large/Int32_10000
  stop_server
fi

if [ "$failed" -eq 0 ]; then
  echo "check: all checks passed"
else
  echo "check: $failed check(s) failed"
  exit 1
fi
