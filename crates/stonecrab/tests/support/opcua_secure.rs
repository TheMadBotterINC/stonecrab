//! First-party secure responder: runtime RSA keys and the existing generated service codec.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    KeyPair, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, RSA_PSS_2048_8192_SHA256,
    RSA_PSS_SHA256, RsaEncoding, RsaKeyPair, RsaPssSaltLength, UnparsedPublicKey,
};
use bytes::Bytes;
use rustls_pki_types::CertificateDer;

use crate::Secret;
use crate::opcua::codec::{UaByteString, UaString};
use crate::opcua::crypto::{Asymmetric, KeyMaterial, Native, Symmetric};
use crate::opcua::frame::{Body, ChunkType, Frame, OpenChunk, SecureChunk, SequenceHeader};
use crate::opcua::generated::channel_security_token::ChannelSecurityToken;
use crate::opcua::generated::endpoint_description::EndpointDescription;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::response_header::ResponseHeader;
use crate::opcua::generated::signature_data::SignatureData;
use crate::opcua::generated::user_token_policy::UserTokenPolicy;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::options::{MessageSecurity, SecurityPolicy};
use crate::opcua::pki::{self, CertSpec, Scratch};
use crate::opcua::responder::{self, Device};
use crate::opcua::service::{decode_service, encode_service};
use crate::testkit::Reaction;

pub(super) struct Fixture {
    pub(super) scratch: Scratch,
    pub(super) client_store: PathBuf,
    pub(super) server_file: PathBuf,
    pub(super) server_certificate: Vec<u8>,
    pub(super) client_certificate: Vec<u8>,
    server_store: PathBuf,
    signing: Arc<Secret<RsaKeyPair>>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let scratch = Scratch::new();
        let client = pki::key_pair();
        let server = pki::other_key_pair();
        let client_certificate = pki::certificate(
            &CertSpec::application("client", "client", 1),
            &client,
            &client,
        );
        let server_certificate = pki::certificate(
            &CertSpec::application("server", "server", 2),
            &server,
            &server,
        );
        let mut client_pem = pki::pem("CERTIFICATE", &client_certificate);
        client_pem.push_str(&pki::pem("PRIVATE KEY", pki::pkcs8_2048()));
        let client_store = scratch.write("client.pem", &client_pem);
        let server_file =
            scratch.write("server.pem", &pki::pem("CERTIFICATE", &server_certificate));
        let mut server_pem = pki::pem("CERTIFICATE", &server_certificate);
        let key: Pkcs8V1Der<'_> = server.as_der().unwrap();
        server_pem.push_str(&pki::pem("PRIVATE KEY", key.as_ref()));
        let server_store = scratch.write("server-key.pem", &server_pem);
        Self {
            scratch,
            client_store,
            server_file,
            server_certificate,
            client_certificate,
            server_store,
            signing: Arc::new(Secret::new(server)),
        }
    }

    pub(super) fn uri(
        &self,
        policy: SecurityPolicy,
        mode: MessageSecurity,
        discovery: bool,
    ) -> String {
        format!(
            "opcua://fake:4840/path?security-policy={}&message-security={}&discovery={discovery}&tls.keystore={}&server-certificate-file={}&access=read-write",
            policy.name(),
            mode.name(),
            self.client_store.display(),
            self.server_file.display()
        )
    }

    pub(super) fn password(&self, policy: SecurityPolicy, ciphertext: &[u8]) -> Vec<u8> {
        use aws_lc_rs::rsa::{
            OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey,
            PrivateDecryptingKey,
        };
        let der: Pkcs8V1Der<'_> = self.signing.expose().as_der().unwrap();
        let key =
            OaepPrivateDecryptingKey::new(PrivateDecryptingKey::from_pkcs8(der.as_ref()).unwrap())
                .unwrap();
        let algorithm = if policy == SecurityPolicy::Aes256Sha256RsaPss {
            &OAEP_SHA256_MGF1SHA256
        } else {
            &OAEP_SHA1_MGF1SHA1
        };
        let mut plain = Vec::new();
        let mut block = vec![0; key.min_output_size()];
        for part in ciphertext.as_chunks::<256>().0 {
            plain.extend_from_slice(key.decrypt(algorithm, part, &mut block, None).unwrap());
        }
        plain
    }

    pub(super) fn peer(&self, policy: SecurityPolicy, mode: MessageSecurity) -> Peer {
        Peer {
            asymmetric: Asymmetric::new(
                policy,
                KeyMaterial::load(self.server_store.to_str()).unwrap(),
                Some(CertificateDer::from(self.client_certificate.clone())),
            )
            .unwrap(),
            policy,
            token: None,
            symmetric: None,
            mode,
            certificate: self.server_certificate.clone(),
            signing: Arc::clone(&self.signing),
            probe: Probe::default(),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Probe(Arc<Mutex<Observed>>);

#[derive(Default)]
struct Observed {
    requests: Vec<Service>,
    incoming: Vec<Vec<u8>>,
    outgoing: Vec<Vec<u8>>,
}

impl Probe {
    pub(super) fn requests(&self) -> Vec<Service> {
        self.0.lock().unwrap().requests.clone()
    }
    pub(super) fn incoming(&self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().incoming.clone()
    }
    pub(super) fn outgoing(&self) -> Vec<Vec<u8>> {
        self.0.lock().unwrap().outgoing.clone()
    }
}

pub(super) struct Peer {
    policy: SecurityPolicy,
    token: Option<UserTokenPolicy>,
    asymmetric: Asymmetric,
    symmetric: Option<Symmetric>,
    mode: MessageSecurity,
    certificate: Vec<u8>,
    signing: Arc<Secret<RsaKeyPair>>,
    pub(super) probe: Probe,
}

fn serialized(frame: &Frame) -> Vec<u8> {
    let length = frame.length_in_bytes().unwrap();
    let mut out = crate::codec::BoundedWriter::new(length);
    frame
        .serialize(u32::try_from(length).unwrap(), &mut out)
        .unwrap();
    out.into_bytes()
}

fn header(handle: u32) -> ResponseHeader {
    let Service::CreateSessionResponse {
        response_header, ..
    } = decode_service(&responder::create_response(handle, 120_000.0, 0)).unwrap()
    else {
        panic!("create")
    };
    response_header
}

impl Peer {
    pub(super) fn token(mut self, kind: UserTokenType, uri: UaString) -> Self {
        self.token = Some(UserTokenPolicy {
            policy_id: UaString::new("selected-user"),
            token_type: kind,
            issued_token_type: UaString::null(),
            issuer_endpoint_url: UaString::null(),
            security_policy_uri: uri,
        });
        self
    }

    fn endpoint(&self) -> EndpointDescription {
        let Service::CreateSessionResponse {
            server_endpoints, ..
        } = decode_service(&responder::create_response(1, 120_000.0, 0)).unwrap()
        else {
            panic!("create")
        };
        let mut endpoint = server_endpoints.unwrap().remove(0);
        endpoint.security_mode = match self.mode {
            MessageSecurity::Sign => MessageSecurityMode::MessageSecurityModeSign,
            MessageSecurity::SignEncrypt => MessageSecurityMode::MessageSecurityModeSignAndEncrypt,
            MessageSecurity::None => panic!("secure mode"),
        };
        endpoint.security_policy_uri = UaString::new(self.policy.uri());
        endpoint.server_certificate = UaByteString::new(self.certificate.clone());
        if let Some(token) = &self.token {
            endpoint.user_identity_tokens = Some(vec![token.clone()]);
        }
        endpoint
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "the responder supports only its generated service subset"
    )]
    pub(super) fn reactor(
        mut self,
        mut map: impl FnMut(&Service, &mut Service) + Send + 'static,
    ) -> impl FnMut(&[u8]) -> Reaction + Send + 'static {
        let device = Device::default();
        let mut device = device.reactor();
        let mut sequence = 41_u32;
        let mut token_id = 29_u32;
        let mut partial = Vec::new();
        let mut session_nonce = vec![0x66; 32];
        let mut session_certificate = self.certificate.clone();
        move |wire| {
            self.probe.0.lock().unwrap().incoming.push(wire.to_vec());
            if wire.starts_with(b"HEL") {
                return device(wire);
            }
            let frame = if wire.starts_with(b"OPN") {
                self.asymmetric.open(wire, 65535).unwrap()
            } else {
                self.symmetric
                    .as_ref()
                    .unwrap()
                    .open(&Native, wire, 65535)
                    .unwrap()
            };
            let (request_id, request, mut response, encoding, is_open) = match &frame.body {
                Body::Open(chunk) => {
                    let request = decode_service(&chunk.body).unwrap();
                    let Service::OpenSecureChannelRequest { request_type, .. } = &request else {
                        panic!("OPN request")
                    };
                    if *request_type == crate::opcua::generated::security_token_request_type::SecurityTokenRequestType::SecurityTokenRequestTypeRenew {
                        token_id = token_id.checked_add(1).unwrap();
                        sequence = sequence.checked_add(1).unwrap();
                    }
                    let response = Service::OpenSecureChannelResponse {
                        response_header: header(chunk.sequence.request_id),
                        server_protocol_version: 0,
                        security_token: ChannelSecurityToken {
                            channel_id: 17,
                            token_id,
                            created_at: 0,
                            revised_lifetime: 120_000,
                        },
                        server_nonce: UaByteString::new(vec![
                            u8::try_from(
                                token_id.checked_add(56).unwrap()
                            )
                            .unwrap();
                            32
                        ]),
                    };
                    (chunk.sequence.request_id, request, response, 449, true)
                }
                Body::Message(chunk) => {
                    partial.extend_from_slice(&chunk.body);
                    if frame.chunk != ChunkType::Final {
                        return Reaction::Silence;
                    }
                    let body = Bytes::from(std::mem::take(&mut partial));
                    let request = decode_service(&body).unwrap();
                    let mut plain = frame.clone();
                    let Body::Message(full) = &mut plain.body else {
                        panic!("MSG")
                    };
                    full.body = body;
                    let response = if matches!(request, Service::GetEndpointsRequest { .. }) {
                        Service::GetEndpointsResponse {
                            response_header: header(chunk.sequence.request_id),
                            endpoints: Some(vec![self.endpoint()]),
                        }
                    } else {
                        let Reaction::Reply(reply) = device(&serialized(&plain)) else {
                            panic!("reply")
                        };
                        let Body::Message(reply) = Frame::parse(&reply).unwrap().body else {
                            panic!("MSG")
                        };
                        decode_service(&reply.body).unwrap()
                    };
                    let encoding = match &response {
                        Service::GetEndpointsResponse { .. } => 431,
                        Service::CreateSessionResponse { .. } => 464,
                        Service::ActivateSessionResponse { .. } => 470,
                        Service::CloseSessionResponse { .. } => 476,
                        Service::ReadResponse { .. } => 634,
                        Service::WriteResponse { .. } => 676,
                        _ => panic!("reply type"),
                    };
                    (
                        chunk.sequence.request_id,
                        request,
                        response,
                        encoding,
                        false,
                    )
                }
                Body::Close(chunk) => {
                    self.probe
                        .0
                        .lock()
                        .unwrap()
                        .requests
                        .push(decode_service(&chunk.body).unwrap());
                    return Reaction::Silence;
                }
                Body::Hello(_) | Body::Acknowledge(_) | Body::Error(_) => panic!("secure message"),
            };
            self.probe.0.lock().unwrap().requests.push(request.clone());
            if let Service::CreateSessionResponse {
                server_nonce,
                server_certificate,
                server_endpoints,
                server_signature,
                ..
            } = &mut response
            {
                let Service::CreateSessionRequest {
                    client_certificate,
                    client_nonce,
                    ..
                } = &request
                else {
                    panic!("create")
                };
                let data = [
                    client_certificate.as_bytes().unwrap(),
                    client_nonce.as_bytes().unwrap(),
                ]
                .concat();
                let pair = self.signing.expose();
                let mut signature = vec![0; pair.public_modulus_len()];
                let encoding: &dyn RsaEncoding =
                    if self.policy == SecurityPolicy::Aes256Sha256RsaPss {
                        &RSA_PSS_SHA256
                    } else {
                        &RSA_PKCS1_SHA256
                    };
                pair.sign(encoding, &SystemRandom::new(), &data, &mut signature)
                    .unwrap();
                *server_signature = SignatureData {
                    algorithm: UaString::new(self.policy.descriptor().signature_uri),
                    signature: UaByteString::new(signature),
                };
                *server_nonce = UaByteString::new(vec![0x66; 32]);
                *server_certificate = UaByteString::new(self.certificate.clone());
                *server_endpoints = Some(vec![self.endpoint()]);
            }
            if let Service::ActivateSessionRequest {
                client_signature,
                user_token_signature,
                ..
            } = &request
            {
                assert_eq!(
                    client_signature.algorithm,
                    UaString::new(self.policy.descriptor().signature_uri)
                );
                let algorithm = if self.policy == SecurityPolicy::Aes256Sha256RsaPss {
                    &const { RSA_PSS_2048_8192_SHA256.with_pss_salt_length(RsaPssSaltLength::Custom(32)) }
                } else {
                    &RSA_PKCS1_2048_8192_SHA256
                };
                UnparsedPublicKey::new(algorithm, pki::key_pair().public_key())
                    .verify(
                        &[session_certificate.as_slice(), &session_nonce].concat(),
                        client_signature.signature.as_bytes().unwrap(),
                    )
                    .unwrap();
                if !self.token.as_ref().is_some_and(|token| {
                    token.token_type == UserTokenType::UserTokenTypeCertificate
                }) {
                    assert_eq!(user_token_signature.algorithm, UaString::null());
                    assert_eq!(user_token_signature.signature, UaByteString::null());
                }
                let Service::ActivateSessionResponse { server_nonce, .. } = &mut response else {
                    panic!("activate")
                };
                *server_nonce = UaByteString::new(vec![0x77; 32]);
            }
            map(&request, &mut response);
            if let Service::CreateSessionResponse {
                server_nonce,
                server_certificate,
                ..
            } = &response
            {
                session_nonce = server_nonce.as_bytes().unwrap_or_default().to_vec();
                session_certificate = server_certificate.as_bytes().unwrap_or_default().to_vec();
            }
            if is_open {
                let Service::OpenSecureChannelRequest { client_nonce, .. } = &request else {
                    panic!("OPN")
                };
                let Service::OpenSecureChannelResponse { server_nonce, .. } = &response else {
                    panic!("OPN response")
                };
                if server_nonce
                    .as_bytes()
                    .is_some_and(|bytes| bytes.len() == 32)
                {
                    self.symmetric = Some(
                        Symmetric::new(
                            &Native,
                            self.policy,
                            self.mode,
                            17,
                            token_id,
                            client_nonce.as_bytes().unwrap(),
                            server_nonce.as_bytes().unwrap(),
                        )
                        .unwrap()
                        .into_server(),
                    );
                }
            }
            let body = encode_service(encoding, &response, 1 << 20).unwrap();
            let mut out = Vec::new();
            if is_open {
                let frame = Frame::single(Body::Open(OpenChunk {
                    channel_id: 17,
                    security: self.asymmetric.header(),
                    sequence: SequenceHeader {
                        sequence_number: sequence,
                        request_id,
                    },
                    body,
                }));
                out.extend_from_slice(&self.asymmetric.seal(&frame, 65535).unwrap());
            } else {
                for (index, part) in body.chunks(8000).enumerate() {
                    sequence = sequence.checked_add(1).unwrap();
                    let final_chunk =
                        index.checked_add(1).unwrap().checked_mul(8000).unwrap() >= body.len();
                    let frame = Frame {
                        chunk: if final_chunk {
                            ChunkType::Final
                        } else {
                            ChunkType::Intermediate
                        },
                        body: Body::Message(SecureChunk {
                            channel_id: 17,
                            token_id,
                            sequence: SequenceHeader {
                                sequence_number: sequence,
                                request_id,
                            },
                            body: Bytes::copy_from_slice(part),
                        }),
                    };
                    out.extend_from_slice(
                        &self
                            .symmetric
                            .as_ref()
                            .unwrap()
                            .seal(&Native, &frame, 8192)
                            .unwrap(),
                    );
                }
            }
            self.probe.0.lock().unwrap().outgoing.push(out.clone());
            Reaction::Reply(out)
        }
    }
}
