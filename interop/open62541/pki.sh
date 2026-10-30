#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
#
# Make the PKI of the open62541 peer, at run time, inside the container (slice S81). Nothing
# this script writes is in the image or in the repository: every container start makes new
# keys. Usage: pki.sh [DIR]   (default $OPCUA_PKI_DIR or /pki; the host of the server
# certificate's names is $OPCUA_ADVERTISED_HOST, default localhost)
#
# Files, each as PEM (*.pem) and DER (*.der):
#   server.cert / server.key       the server's application instance certificate, URI
#                                  urn:stonecrab:interop:open62541
#   client.cert / client.key       a client application certificate, URI
#                                  urn:stonecrab:interop:client; trusted by the server
#   untrusted.cert / untrusted.key a client application certificate, URI
#                                  urn:stonecrab:interop:untrusted; not in any trust list
#   user.cert / user.key           an X.509 user identity; in the server's user token trust list
# and user-password, the password of the user name "stonecrab" (no newline).
#
# All are self-signed 2048-bit RSA leaf certificates (basicConstraints CA:FALSE, as open62541
# requires of a leaf), valid from yesterday for 30 days, with the key usages OPC UA asks for.
set -eu

DIR="${1:-${OPCUA_PKI_DIR:-/pki}}"
HOST="${OPCUA_ADVERTISED_HOST:-localhost}"
umask 077
mkdir -p "$DIR"
cd "$DIR"

case "$HOST" in
  *[!0-9.]*) HOST_SAN="DNS:$HOST" ;;
  *) HOST_SAN="IP:$HOST" ;;
esac
NOT_BEFORE="$(date -u -d '1 day ago' +%Y%m%d%H%M%SZ)"
NOT_AFTER="$(date -u -d '30 days' +%Y%m%d%H%M%SZ)"

# make NAME CN URI EXTRA-SAN
make() {
  cat >"$1.cnf" <<CNF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ext
[dn]
O = Stonecrab interop
CN = $2
[ext]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,nonRepudiation,keyEncipherment,dataEncipherment
extendedKeyUsage = serverAuth,clientAuth
subjectAltName = URI:$3,$4
subjectKeyIdentifier = hash
CNF
  openssl req -x509 -newkey rsa:2048 -nodes -sha256 -config "$1.cnf" \
    -not_before "$NOT_BEFORE" -not_after "$NOT_AFTER" \
    -keyout "$1.key.pem" -out "$1.cert.pem" 2>/dev/null
  openssl x509 -in "$1.cert.pem" -outform DER -out "$1.cert.der"
  openssl pkey -in "$1.key.pem" -outform DER -out "$1.key.der"
  rm -f "$1.cnf"
}

make server "Stonecrab open62541 server" urn:stonecrab:interop:open62541 "DNS:localhost,IP:127.0.0.1,$HOST_SAN"
make client "Stonecrab interop client" urn:stonecrab:interop:client DNS:localhost
make untrusted "Stonecrab untrusted client" urn:stonecrab:interop:untrusted DNS:localhost
make user "Stonecrab interop user" urn:stonecrab:interop:user DNS:localhost

openssl rand -hex 12 | tr -d '\n' >user-password
echo "pki: made server, client, untrusted and user certificates in $DIR"
