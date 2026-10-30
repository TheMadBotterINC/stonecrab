# Stonecrab open62541 interoperability peer

The independent OPC UA peer of milestone M3 (slice S81): an OPC UA server and a check client
built on open62541 v1.5.9 (C, MPL-2.0), a stack that shares no code with Stonecrab or with
Eclipse Milo. The `interop-opcua` job of `.github/workflows/ci.yml` and `mise run
interop-opcua` build it, run the checks below against it and keep the evidence. It is outside
`mise run interop`, which stays the Modbus TCP peers, until the peer has proved stable. No
Stonecrab OPC UA code exists yet; slice S90 adds the driver's `#[ignore]` interop tests that
use this image.

## First-party statement and provenance

The recipe in this directory (`Dockerfile`, `main.c`, `client_check.c`, `pki.sh`,
`serve.sh`, `check.sh`, `interop.sh`) is first-party and Apache-2.0 (`PROVENANCE.toml` lists
the files). `main.c` and `client_check.c` are written from the public API of open62541
v1.5.9; none of its source, and none of its examples, is copied. open62541 itself is not in
this repository: the Dockerfile fetches it, builds it inside the image and links it
statically into the two programs.

What is pinned:

- the base image `debian:trixie-slim` by index digest (both `FROM` lines), the digest of
  `interop/libmodbus/Dockerfile`;
- open62541 by the commit of the tag v1.5.9, `05c2b389b65cc35884eb40d800301af1f3d130cf`
  (resolved with `git ls-remote` on 2026-10-30; v1.5.9 is a lightweight tag). The Dockerfile
  fetches by tag and stops unless `git rev-parse HEAD` is that commit, so a moved tag cannot
  change what is built.

What is not pinned, on purpose: the Debian packages. OpenSSL (`libssl-dev` for the build,
`libssl3t64` and `openssl` in the image) is whatever Debian 13 ships when the image is built,
3.5.7 on 2026-10-30. The security archive keeps only the newest version of a package, so an
exact pin, as `interop/libmodbus` has for libmodbus, would fail the weekly scheduled run at
the first security update. `image-digest.txt` in the evidence records the versions of the
run. To move the source, change `OPEN62541_TAG` and `OPEN62541_COMMIT` in the Dockerfile,
rebuild, and record the new values here and in `PROVENANCE.toml`.

Encryption is open62541's OpenSSL backend (`UA_ENABLE_ENCRYPTION=OPENSSL`), inside the image
only; the crate links no OpenSSL. The image runs as an unprivileged user (uid 10001).

## What it serves

Every start of the container makes a new PKI with `openssl` (`pki.sh`): a server
application certificate and, so that trust can be tested, a trusted client application
certificate, an untrusted one, and a trusted X.509 user identity, all 2048-bit RSA, self-signed
leaves valid from the day before for 30 days, with the user name `stonecrab` and a random
password. Nothing of it is in the image or in the repository. The server's certificate
carries the application URI `urn:stonecrab:interop:open62541` and names `localhost`,
`127.0.0.1` and the advertised host.

| Aspect | The server |
| --- | --- |
| Policies | None, Basic256Sha256, Aes128Sha256RsaOaep, Aes256Sha256RsaPss; Sign and SignAndEncrypt for the three secure ones (seven endpoints). Basic128Rsa15 and Basic256 (deprecated, E16) are not offered |
| Identities | anonymous, user name (`stonecrab`), X.509 (the user certificate, validated against the user token trust list) |
| Trust | the secure channel trusts the client certificate only, the session trusts the user certificate only; `OPCUA_TRUST=empty` trusts nothing |
| Session timeout | 5000 ms ceiling (`OPCUA_MAX_SESSION_TIMEOUT_MS`) |
| Chunk limits | default 64 KiB buffers; `OPCUA_SMALL_CHUNKS=1` gives 8192-byte chunks, at most 4 per message and 24 KiB per message |
| Advertised address | `OPCUA_ADVERTISED_HOST` and `OPCUA_PORT`, which `interop.sh` sets to the address the container is reached on; see "Behaviors to know" |

The namespace is `urn:stonecrab:interop:open62541`, index 1 (the application URI is the
namespace URI). Everything is below the object `ns=1;s=Interop`, with string node ids:

| Node id | What |
| --- | --- |
| `scalar/<T>` | every built-in type but Variant and ExtensionObject (see `structure`) with a fixed sample (Boolean, SByte, Byte, Int16, UInt16, Int32, UInt32, Int64, UInt64, Float, Double, String, DateTime, Guid, ByteString, XmlElement, NodeId, ExpandedNodeId, StatusCode, QualifiedName, LocalizedText, DataValue, DiagnosticInfo), read only; `Int32` is -123456789 |
| `writable/<T>` | the same, initialized alike, read and write |
| `array/Boolean`, `array/Int32`, `array/Double`, `array/String` | arrays of 3, 5, 3 and 3 elements; `writable/Int32Array` takes any length |
| `large/Int32_4000`, `large/Int32_10000` | 16 KB and 40 KB arrays of 0..N-1, to cross chunk boundaries (the 40 KB one does not fit the small limits and is answered `BadResponseTooLarge`) |
| `matrix/Int32_2x3`, `matrix/Double_2x2x2` | value rank 2 and 3 with their dimensions |
| `counter` | UInt32, plus one every second |
| `structure` | an ExtensionObject of the custom structure `Reading { UInt32 Id; Double Value; String Label }` (type `ns=1;i=5001`, binary encoding `ns=1;i=5002`), value 42, 0.25, "sample" |
| `events/Source` | an event source (BaseEventType, severity 500, message `tick N`, every 2000 ms; `OPCUA_EVENT_PERIOD_MS`); also an event source of the Server object |
| `events/Fire` | a method with no arguments that fires one more event |

A write to a read-only node is answered `BadUserAccessDenied`, not `BadNotWritable`. A
variable's value type is its data type (no abstract types), and the matrices and arrays carry
`ArrayDimensions`.

## The three answers (open62541 v1.5.9)

Citations are paths and lines in the source at commit `05c2b389...`; each answer was also
measured by the checks of `check.sh` against the built image.

**Minimum session timeout: there is none.** `CreateSession` takes the requested timeout
as is when it is greater than 0 and not greater than the server's `maxSessionTimeout`;
otherwise (0, or too large) it uses the ceiling, which defaults to one hour
(`src/server/ua_services_session.c:446-449`; `plugins/ua_config_default.c:485`). The
lifetime is renewed by every service call (`src/server/ua_services.c:279-283`) and checked
when the session is used (`src/server/ua_services_session.c:185-191` and `:218-224`, so an
expired session is refused at once, with `BadSessionIdInvalid`, or `BadSessionClosed` on its
channel, `src/server/ua_server_binary.c:414-427`) and by a housekeeping callback every 1000 ms, which removes it
(`src/server/ua_server.c:499-505` and `:1290-1292`; `src/server/ua_services_session.c:158-168`).
Measured with the 5000 ms ceiling: a 1000 ms session is alive after 300 ms and gone after
3000 ms, a 100 ms session is gone after 300 ms, and a 60000 ms request is cut to the ceiling
(gone after 7000 ms). So renewal tests against this peer can use timeouts of a few hundred
milliseconds; the ceiling is `OPCUA_MAX_SESSION_TIMEOUT_MS`.

**TransferSubscriptions and Republish: both are supported.** Both services are in the
service table when subscriptions are built (`src/server/ua_services.c:95-117`, under
`UA_ENABLE_SUBSCRIPTIONS`, which is on by default, `CMakeLists.txt:114`).
`Service_Republish` answers a message from the subscription's retransmission queue and
`BadMessageNotAvailable` for one that is not there
(`src/server/ua_services_subscription.c:479-522`); the queue is on
(`plugins/ua_config_default.c:493`). `Service_TransferSubscriptions` moves the monitored
items, the notification queue and the retransmission queue to the calling session and can
send the initial values (`src/server/ua_services_subscription.c:552-740`). When a session
times out its subscriptions are detached, not deleted, so that a later session can take them
(`src/server/ua_services_session.c:77-92`). The default access control decides who may
transfer (`plugins/ua_accesscontrol_default.c:206-318`, registered at `:392`): for two
anonymous sessions both must be on a secure channel (not None) and have the same client
application URI; for authenticated sessions the user identity must be the same. Measured:
on Basic256Sha256 SignAndEncrypt a second session of the same application takes a
subscription by `TransferSubscriptions` (`Good`, anonymous and with a user name token);
on SecurityPolicy None the same transfer is `BadUserAccessDenied`; `Republish` of an
acknowledged message is `BadMessageNotAvailable`, so the service is dispatched. The measured
transfer reported no available sequence numbers, because the client had acknowledged every
message; a `Republish` of a message that is still in the queue is therefore read from the
source, not measured. This answers the open question of E19: the peer can carry the
caller's "end with Disconnected, then re-subscribe" and also TransferSubscriptions across a
channel loss on a secure policy.

**Aes256Sha256RsaPss under the OpenSSL backend: it works.** The policy has its own file in
the OpenSSL plugin (`plugins/crypto/openssl/securitypolicy_aes256sha256rsapss.c`): the
asymmetric signature is RSA-PSS with SHA-256 (`:244` verify, `:515` sign), the asymmetric
encryption is RSA-OAEP with SHA-256 (padding length 66, `:24`, `:525`; URI
`rsa-oaep-sha2-256`, `:633`), keys of 2048 to 4096 bits (`:29-30`) and the certificate-signature
functions are PKCS#1 v1.5 SHA-256 (`:370-391`). The PSS helpers set the salt length to
the digest length and the MGF1 hash to SHA-256
(`plugins/crypto/openssl/securitypolicy_common.c:138-148` verify, `:642-653` sign), which are
the parameters of the specification. The policy is added by
`UA_ServerConfig_addSecurityPolicyAes256Sha256RsaPss` (`plugins/ua_config_default.c:846-878`),
and the policy file has no conditional on the OpenSSL version. Measured: this image (OpenSSL 3.5.7)
connects with Aes256Sha256RsaPss SignAndEncrypt and reads the counter, as it does with
Aes128Sha256RsaOaep and Basic256Sha256.

## Behaviors to know

- **Trust lists.** A policy and a certificate group are set up the way `main.c` does it:
  `UA_CertificateGroup_Memorystore` with an explicit list, before the policies. The
  helper `UA_ServerConfig_setDefaultWithSecurityPolicies` does something else with an empty
  list: it logs "Empty trustlist passed" and keeps its accept-all verification
  (`plugins/ua_config_default.c:1452-1456`). The same holds for the client's
  `UA_ClientConfig_setDefaultEncryption` (`:2310-2314`): an open62541 client with no trust
  list trusts every server. With an explicit empty list the server refuses a secure channel
  with `BadSecurityChecksFailed` (measured), an untrusted client certificate likewise, and an
  X.509 user token that is not in the user list is `BadIdentityTokenRejected`.
- **A leaf certificate must not be a CA.** The OpenSSL verifier refuses a certificate that is
  a CA (`CA:TRUE` with the key usages keyCertSign and cRLSign) with `BadCertificateUseNotAllowed`
  (`plugins/crypto/openssl/certificategroup.c:461-475` and `:635`) and needs a start time
  strictly in the past (`:598`); `pki.sh` makes `CA:FALSE` leaves valid from the day before.
- **Endpoint URLs.** `GetEndpoints` mirrors back the endpoint URL the client asked with
  (`src/server/ua_services_discovery.c:554-562`), whatever the server advertises; the
  server also adds the hostname and port of each listen socket to its discovery URLs
  (`src/server/ua_server_binary.c:710-735` and `:786-793`: in a container, the container's
  hostname). `FindServers` lists those and the advertised
  `opc.tcp://OPCUA_ADVERTISED_HOST:OPCUA_PORT` that `main.c` puts first, which `interop.sh`
  sets to `127.0.0.1` and the published port. Because `GetEndpoints` mirrors the URL, a client
  that dials a proxy gets endpoints at the proxy's address; only a client that follows the
  discovery URLs of `FindServers` sees the advertised one, so behind a proxy set both
  variables to the proxy's address.
- **User name over None.** The server refuses a user name token on a None channel unless
  `OPCUA_ALLOW_NONE_PASSWORD=1` (`allowNonePolicyPassword`,
  `src/server/ua_services_session.c:855-861`).
- **Chunk limits.** With `OPCUA_SMALL_CHUNKS=1` the server proposes 8192-byte buffers,
  4 chunks and 24576 bytes per message (`src/server/ua_server_binary.c:620-662`); a client
  that asks for a larger message is answered `BadResponseTooLarge`.

## Checks

`check.sh` runs inside the image (`docker run --rm stonecrab-interop-open62541:local check`)
with open62541's own client (`client_check.c`): it starts servers from the container's fresh
PKI and checks, one line each,

- reads of `counter` with None, and with Basic256Sha256 (Sign and SignAndEncrypt),
  Aes128Sha256RsaOaep and Aes256Sha256RsaPss, and with the user name and X.509 tokens;
- refusals: an untrusted client certificate, an X.509 token not in the user list, a client that
  trusts another certificate than the server's, and, on a server started with an empty
  trust list, the client certificate that is otherwise trusted (`BadSecurityChecksFailed`);
- the test namespace: sample values, dimensions, the structure's encoding, a write and a
  refused write, events arriving, 16 KB and 40 KB reads with and without the small limits;
- the three answers above; and `GetEndpoints` and `FindServers` with another advertised host.

## Build and run

From the repository root, with Docker running:

```sh
docker build -f interop/open62541/Dockerfile -t stonecrab-interop-open62541:local .
docker run --rm stonecrab-interop-open62541:local check
docker run -d --name stonecrab-interop-open62541 -p 127.0.0.1:4840:4840 \
  -e OPCUA_PORT=4840 -e OPCUA_ADVERTISED_HOST=127.0.0.1 stonecrab-interop-open62541:local
docker rm -f stonecrab-interop-open62541
```

`mise run interop-opcua` (`interop/open62541/interop.sh run`) builds, starts the server,
waits for a real OPC UA Hello to be answered with an Acknowledge through the published port,
runs the checks, copies the client side of the PKI and writes the evidence. It needs the
`docker` binary and a running daemon and fails with a message naming the missing piece
otherwise. The image builds open62541 on every platform Docker offers; the first build takes
a few minutes natively and longer under emulation.

- Evidence, in `target/interop-evidence/open62541/`: `image-digest.txt` (image id,
  architecture, base images, source tag and commit, OpenSSL versions), `check.txt` (the output
  of the checks) and `server.log` (never a key or a password). The image id depends on the
  platform that built it.
- The PKI's client side, for the driver's tests, in `target/interop-pki/open62541/` (git
  ignores `target/`): `client.cert.der` and `.pem`, `client.key.*`, `untrusted.*`, `user.*`,
  `server.cert.*` and `user-password`. It is the only key material that leaves the container,
  it is made at run time, it is never uploaded by CI and `stop` removes it (only those
  files, never the directory), so a test step must run before `stop`.

`interop.sh` takes `INTEROP_OPEN62541_PORT` (default 4840), `INTEROP_EVIDENCE_DIR` and
`INTEROP_PKI_DIR`. The container takes the variables listed at the top of `main.c`
(`OPCUA_PORT`, `OPCUA_ADVERTISED_HOST`, `OPCUA_TRUST`, `OPCUA_SMALL_CHUNKS`,
`OPCUA_MAX_SESSION_TIMEOUT_MS`, `OPCUA_MAX_TOKEN_LIFETIME_MS`, `OPCUA_EVENT_PERIOD_MS`,
`OPCUA_ALLOW_NONE_PASSWORD`).
