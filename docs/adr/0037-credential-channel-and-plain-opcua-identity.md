# 0037: Credential channel and explicit plain OPC UA identity

## Status

Accepted design (2026-11-03), S99, under E18/E22 and ADR-0034. Implementation/evidence
must pass before merge; encrypted user tokens and X.509 activation remain S108.

## Design and implementation plan

Credentials move separately from URI text into the existing ConnectionConfig, through
registry connect_with and a per-cache provider invoked only when opening a new session.
OPC UA validates identity before I/O, selects a compatible endpoint/token, and builds
anonymous or explicitly opted-in plain UserName tokens; remove its blanket credential
rejection without weakening protected-policy refusal or URI-key rejection.

- Published Credentials is a non-exhaustive struct, not the enum the roadmap assumes.
  Preserve its public username/password fields and new constructor. Add a private
  identity discriminator, a public non-exhaustive CredentialIdentity with UserNamePassword
  and X509 { certificate_file, private_key_file } variants, Credentials::x509 and a
  read-only identity accessor. Certificate constructors leave the legacy fields empty;
  the runtime refuses X509 before I/O until S108. Paths are not key material and are never
  loaded here. Identity Debug redacts both paths, passwords stay Secret and complete
  config Debug/redacted_display continue to hide the entire credential. No Clone/Default
  or enum replacement that would break current field access. This is the concrete E18
  certificate representation and an amendment to ADR-0034, not a compatibility alias.
- Registry: connect_with(uri, Credentials) parses/validates the URI as before, then moves
  credentials onto the parsed config with a private consuming setter. Reuse one private
  connect-from-parsed routine for connect/connect_with; preserve all options/policies.
  URI username/password keys remain refused by OPC UA, even with supplied credentials.
- Cache: builder credential_provider accepts a synchronous Send+Sync callback from
  &ConnectionUri to Result<Option<Credentials>>. Store it privately in Arc; Debug reveals
  only presence, never the closure/captures. Resolve after registry validation, outside
  state locks, only for a new open (including replacements), never on reuse. Errors
  propagate without an anonymous retry and must satisfy the existing redacted PlcError
  contract. None preserves the driver's config. Programmatic credentials never become
  URI/key text. One cache has one identity policy; changing identity/rotating credentials
  requires remove/close or a new cache before reuse, not a credential-bearing key.
- Add generated UserNameIdentityToken (324) to opcua.toml, regenerate in a separate gen
  commit, and add complete/truncated independent service vectors and fuzz seeds.
- Private opcua/identity.rs resolves no/empty credentials as Anonymous, rejects a password
  without username as InvalidConfig, and requires allow-insecure-credentials=true for
  username/None before transport I/O. The strict boolean option defaults false.
  Borrow credentials through connect/session; send UTF-8 password bytes with no nonce or
  encryptionAlgorithm only when the selected effective token policy is None. Null/empty
  token policy inherits the channel. Any protected token policy remains Unsupported,
  without trying a weaker token or changing the requested policy. Endpoint matching keeps
  the S96 security-level/token-protection ranking and checks the resolved token type.
- Session uses the selected policy ID for either identity; log a static warning for the
  plain username path, with no username/password/token ID/certificate/path in it. Map
  activation identity/access refusal codes (including ServiceFault) to Authentication,
  preserving the exact device source chain; other status behavior remains unchanged.
- Existing open62541 scaffold already provides OPCUA_ALLOW_NONE_PASSWORD. Enable it
  explicitly for the native test server, leaving the in-image security checks unchanged.
  Verify actual plain-policy advertisement and correct/wrong-password public sessions;
  read the runtime password only from the existing ignored PKI, never print or commit it.
- Test config/registry/cache redaction, provider open/reuse/replacement/error behavior,
  identity selection/refusals, warning redaction, raw token fields and authentication
  error sources. All coverage floors remain unchanged; full checks, isolated features,
  semver default/all, regeneration/fuzz/mutations, package/unpacked build, dependencies,
  parity and independent peer evidence precede normal signed-off merge/push.

The core URI grammar is unchanged: callers must supply credential-free URI text; the
library never interpolates out-of-band credentials. Provider code/errors have the same
redaction/no-blocking expectations as existing caller-supplied drivers and callbacks.

Reference: [OPC Foundation UserNameIdentityToken](https://reference.opcfoundation.org/specs/OPC-10000-4/7.40.4)
defines the unencrypted UTF-8 password and empty/null encryption algorithm. The existing
peer's allowNonePolicyPassword setting is confined to this explicit software test path.

S99 feature validation found that the new static warning needs the existing optional
tracing dependency when opcua is enabled alone. The opcua feature now enables dep:tracing,
as sim/modbus already do. No dependency/package or lockfile entry is added. Feature-only
builds and archive verification must be repeated after this manifest correction.
