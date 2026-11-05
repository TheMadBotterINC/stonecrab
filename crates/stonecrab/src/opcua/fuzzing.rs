//! The entry points of `fuzz_opcua_connection_string`, `fuzz_opcua_decode` and
//! `fuzz_opcua_frame`, compiled only under cargo-fuzz's `--cfg fuzzing`: the connection string
//! parser, the recursive types of the binary encoding and the framer are crate-private until
//! slices S91, S88 and S90, and the fuzz workspace can reach only public items.

use bytes::BytesMut;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{DataValue, Depth, DiagnosticInfo, ExtensionObject, Variant};
use crate::opcua::driver::OpcuaDriver;
use crate::opcua::frame::OpcuaFramer;
use crate::opcua::options::OpcuaOptions;
use crate::transport::{FrameError, Framer};
use crate::{ConnectionConfig, ResourceLimits, Result};

pub use crate::opcua::crypto::OpnProbe;

/// Exercise bounded DER/time/EKU parsing and server leaf checks at a fixed test date.
///
/// # Errors
/// Trust for malformed, expired, wrong-usage or out-of-policy RSA certificates.
pub fn inspect_certificate(bytes: &[u8]) -> Result<()> {
    use crate::opcua::crypto::certificate;
    use std::time::{Duration, UNIX_EPOCH};
    let _chain = certificate::chain(bytes);
    let _element = certificate::tlv(&mut BoundedReader::new(bytes));
    let _eku = certificate::server_eku(bytes);
    let _properties = certificate::Properties::parse(bytes);
    let _utc = certificate::time(0x17, bytes);
    let _generalized = certificate::time(0x18, bytes);
    let now = UNIX_EPOCH
        .checked_add(Duration::from_secs(1_793_318_400))
        .ok_or_else(|| crate::PlcError::new(crate::ErrorKind::Trust, "test date is unavailable"))?;
    certificate::check_leaf(
        &rustls_pki_types::CertificateDer::from(bytes),
        crate::opcua::options::SecurityPolicy::Basic256Sha256,
        now,
    )
    .map(|_certificate| ())
}

/// What the fuzz target checks of an accepted connection string.
#[derive(Debug)]
pub struct Parsed {
    /// The configuration `parse_config` built.
    pub config: ConnectionConfig,
    /// The endpoint URL Hello would carry.
    pub endpoint_url: String,
    /// The name `security-policy` takes for the parsed policy.
    pub security_policy: &'static str,
    /// The name `message-security` takes for the parsed mode.
    pub message_security: &'static str,
    /// The configured receive buffer advertised before the resource cap.
    pub receive_buffer_size: u32,
    /// Requested session timeout, independent of connect and request bounds.
    pub session_timeout: std::time::Duration,
    /// Requested channel token lifetime in milliseconds.
    pub channel_lifetime: u32,
    /// Minimum renewal scheduling lifetime in milliseconds.
    pub min_channel_lifetime: u32,
    /// Per-node reference ceiling; zero disables the protocol ceiling.
    pub browse_max_references_per_node: u32,
    /// Total visited-node ceiling; zero leaves the finite core response bound.
    pub browse_max_total_nodes: u32,
    /// Traversal depth ceiling; zero leaves deadline/cancellation and node bounds.
    pub browse_max_depth: u32,
}

/// Parse an `opcua` connection string as the driver does and report what was attached.
///
/// # Errors
/// As `OpcuaDriver::parse_config`: [`ErrorKind::InvalidConfig`](crate::ErrorKind) for text
/// that is not a valid `opcua` connection string.
pub fn parse_config(uri: &str) -> Result<Parsed> {
    let config = OpcuaDriver::configuration(uri)?;
    let options = config.driver_options_as::<OpcuaOptions>().ok_or_else(|| {
        crate::PlcError::new(
            crate::ErrorKind::Internal,
            "parse_config attaches no options",
        )
    })?;
    let (endpoint_url, security_policy, message_security) = (
        options.endpoint_url.clone(),
        options.security_policy.name(),
        options.message_security.name(),
    );
    let receive_buffer_size = options.receive_buffer_size;
    let session_timeout = options.session_timeout;
    let channel_lifetime = options.channel_lifetime;
    let min_channel_lifetime = options.min_channel_lifetime;
    let browse_max_references_per_node = options.browse_max_references_per_node;
    let browse_max_total_nodes = options.browse_max_total_nodes;
    let browse_max_depth = options.browse_max_depth;
    Ok(Parsed {
        config,
        endpoint_url,
        security_policy,
        message_security,
        receive_buffer_size,
        session_timeout,
        channel_lifetime,
        min_channel_lifetime,
        browse_max_references_per_node,
        browse_max_total_nodes,
        browse_max_depth,
    })
}

/// The recursive and open types of the binary encoding that `decode` can be pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecursiveType {
    /// A Variant.
    Variant,
    /// A `DataValue`.
    DataValue,
    /// A `DiagnosticInfo`.
    DiagnosticInfo,
    /// An `ExtensionObject` envelope.
    ExtensionObject,
}

impl RecursiveType {
    /// The type that the first byte of a fuzz input selects.
    #[must_use]
    pub const fn from_selector(selector: u8) -> Self {
        match selector % 4 {
            0 => Self::Variant,
            1 => Self::DataValue,
            2 => Self::DiagnosticInfo,
            _ => Self::ExtensionObject,
        }
    }
}

/// What the fuzz target checks of bytes that parsed. A value that parsed was read with a full
/// budget, so every step after the parse must succeed; each is kept apart so that the target
/// can tell a refusal of the input from a failure of the codec.
#[derive(Debug)]
pub struct Decoded {
    /// The bytes the parse consumed.
    pub consumed: usize,
    /// What `length_in_bytes` said the value takes.
    pub length: core::result::Result<usize, ParseError>,
    /// The value written back.
    pub written: core::result::Result<Vec<u8>, ParseError>,
    /// The value parsed from `written`, written back again.
    pub rewritten: core::result::Result<Vec<u8>, ParseError>,
}

/// One value of `kind` through all the functions of its signature: parse `bytes`, measure the
/// value, write it and parse what was written.
///
/// # Errors
/// The [`ParseError`] of the first parse, which is how the codec refuses the input.
pub fn decode(kind: RecursiveType, bytes: &[u8]) -> core::result::Result<Decoded, ParseError> {
    match kind {
        RecursiveType::Variant => round_trip(
            bytes,
            Variant::parse,
            Variant::serialize,
            Variant::length_in_bytes,
        ),
        RecursiveType::DataValue => round_trip(
            bytes,
            DataValue::parse,
            DataValue::serialize,
            DataValue::length_in_bytes,
        ),
        RecursiveType::DiagnosticInfo => round_trip(
            bytes,
            DiagnosticInfo::parse,
            DiagnosticInfo::serialize,
            DiagnosticInfo::length_in_bytes,
        ),
        RecursiveType::ExtensionObject => round_trip(
            bytes,
            ExtensionObject::parse,
            ExtensionObject::serialize,
            ExtensionObject::length_in_bytes,
        ),
    }
}

type Parse<T> = fn(&mut BoundedReader<'_>, Depth) -> core::result::Result<T, ParseError>;
type Serialize<T> = fn(&T, &mut BoundedWriter, Depth) -> core::result::Result<(), ParseError>;
type Length<T> = fn(&T, Depth) -> core::result::Result<usize, ParseError>;

fn round_trip<T>(
    bytes: &[u8],
    parse: Parse<T>,
    serialize: Serialize<T>,
    length: Length<T>,
) -> core::result::Result<Decoded, ParseError> {
    let depth = Depth::new();
    let mut reader = BoundedReader::new(bytes);
    let value = parse(&mut reader, depth)?;
    let write = |value: &T| -> core::result::Result<Vec<u8>, ParseError> {
        let mut writer = BoundedWriter::new(usize::MAX);
        serialize(value, &mut writer, depth)?;
        Ok(writer.into_bytes())
    };
    let written = write(&value);
    let rewritten = written.clone().and_then(|written| {
        let again = parse(&mut BoundedReader::new(&written), depth)?;
        write(&again)
    });
    Ok(Decoded {
        consumed: bytes.len().saturating_sub(reader.remaining()),
        length: length(&value, depth),
        written,
        rewritten,
    })
}

/// The receive buffer, in bytes, that a selector byte stands for: 65535 (upstream's default),
/// 4096, 256 or 24 (a message of a few bytes of body), by its two low bits.
#[must_use]
pub const fn frame_limit(selector: u8) -> usize {
    match selector % 4 {
        0 => 65_535,
        1 => 4096,
        2 => 256,
        _ => 24,
    }
}

/// What `frame` reports of a message it took.
#[derive(Debug)]
pub struct Framed {
    /// The message written back, as the framer writes it.
    pub written: core::result::Result<Vec<u8>, FrameError>,
}

/// Take one message from the front of `buffer` with a framer whose receive buffer is
/// [`frame_limit`]`(selector)` and the default [`ResourceLimits`], and write it back.
///
/// # Errors
/// The framer's own refusal: [`FrameError::Malformed`] for bytes that are not a message and
/// [`FrameError::Oversized`] for a size above the limit. `Ok(None)` waits for more bytes. In
/// both cases the buffer is left as it was.
pub fn frame(
    selector: u8,
    buffer: &mut BytesMut,
) -> core::result::Result<Option<Framed>, FrameError> {
    let receive_buffer = u32::try_from(frame_limit(selector)).unwrap_or(u32::MAX);
    // The default limits are valid; the refusal only keeps the entry free of a panic.
    let Ok(limits) = ResourceLimits::builder().build() else {
        return Err(FrameError::Malformed);
    };
    let mut framer = OpcuaFramer::new(receive_buffer, limits);
    let mut raw = crate::opcua::frame::RawFramer(framer);
    if let Ok(Some(bytes)) = raw.decode(&mut buffer.clone()) {
        let _parsed = crate::opcua::frame::Frame::parse(&bytes);
        let _encoded = raw.encode(&bytes, &mut BytesMut::new());
    }
    Ok(framer.decode(buffer)?.map(|decoded| {
        let mut out = BytesMut::new();
        Framed {
            written: framer.encode(&decoded, &mut out).map(|()| out.to_vec()),
        }
    }))
}

/// Exercise the generated service dispatcher. The first four bytes are the little-endian
/// encoding id; the remaining input is the service body. Unknown/disabled ids are refused.
///
/// # Errors
/// Returns the codec's `ParseError` for malformed, excessive or unsupported input.
pub fn service(bytes: &[u8]) -> core::result::Result<Decoded, ParseError> {
    if bytes.len() > crate::opcua::codec::MAX_STRING_BYTES {
        return Err(ParseError::OversizedLength {
            length: bytes.len(),
            max: crate::opcua::codec::MAX_STRING_BYTES,
        });
    }
    round_trip(bytes, parse_service, serialize_service, measure_service)
}

type Service = (
    i32,
    crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition,
);

fn parse_service(
    reader: &mut BoundedReader<'_>,
    depth: Depth,
) -> core::result::Result<Service, ParseError> {
    let id = reader.read_i32(crate::ByteOrder::LittleEndian)?;
    let value =
        crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition::parse(
            reader, depth, id,
        )?;
    if let crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition::GetEndpointsResponse { endpoints: Some(endpoints), .. } = &value {
        for policy in ["NONE", "Basic256Sha256", "Aes128_Sha256_RsaOaep", "Aes256_Sha256_RsaPss"] {
            let options = OpcuaOptions::from_parameters("opc.tcp://fake:4840/path".to_owned(), &[("security-policy".to_owned(),policy.to_owned())]);
            if let Ok(options) = options {
                let _selection = crate::opcua::endpoint::select(endpoints, &options, crate::opcua::generated::user_token_type::UserTokenType::UserTokenTypeAnonymous);
            }
        }
    }
    Ok((id, value))
}
fn serialize_service(
    value: &Service,
    writer: &mut BoundedWriter,
    depth: Depth,
) -> core::result::Result<(), ParseError> {
    writer.write_i32(value.0, crate::ByteOrder::LittleEndian)?;
    value.1.serialize(writer, depth)
}
fn measure_service(value: &Service, depth: Depth) -> core::result::Result<usize, ParseError> {
    let mut writer = BoundedWriter::new(crate::opcua::codec::MAX_STRING_BYTES);
    serialize_service(value, &mut writer, depth)?;
    Ok(writer.as_slice().len())
}

/// Completed bodies and independently concatenated inputs from a bounded chunk stream.
#[derive(Debug)]
pub struct Chunked {
    /// Each completed body's output and its input fragments, in wire order.
    pub bodies: Vec<(Vec<u8>, Vec<u8>)>,
    /// Largest partial body retained during this run.
    pub peak: usize,
    /// Configured body bound for this run.
    pub limit: usize,
}

/// Exercise the production assembler with framed C/F/A inputs and small byte/count limits.
/// The first two bytes select 1..=4096 body bytes; the third selects 1..=16 chunks.
///
/// # Errors
/// Returns typed framing, annotation or message-limit errors for rejected streams.
pub fn chunk_reassembly(input: &[u8]) -> Result<Chunked> {
    use crate::opcua::chunk::{Assembly, MessageLimits, abort, limited};
    use crate::opcua::frame::{Body, ChunkType};
    let Some((&[low, high, count], stream)) = input.split_first_chunk::<3>() else {
        return Err(limited("missing fuzz settings"));
    };
    if stream.len() > 65536 {
        return Err(limited("fuzz stream bound"));
    }
    let limits = MessageLimits {
        bytes: usize::from(u16::from_le_bytes([low, high]) % 4096) + 1,
        chunks: u32::from(count % 16) + 1,
    };
    let resources = ResourceLimits::builder().build()?;
    let mut framer = OpcuaFramer::new(8192, resources);
    let mut buffer = BytesMut::from(stream);
    let mut assembly = Assembly::default();
    let mut expected = Vec::new();
    let mut result = Chunked {
        bodies: Vec::new(),
        peak: 0,
        limit: limits.bytes,
    };
    while let Some(frame) = framer.decode(&mut buffer).map_err(crate::PlcError::from)? {
        let Body::Message(chunk) = frame.body else {
            return Err(crate::PlcError::new(
                crate::ErrorKind::MalformedData,
                "fuzz stream expects MSG",
            ));
        };
        if frame.chunk == ChunkType::Abort {
            let _problem = abort(&chunk.body)?;
            assembly = Assembly::default();
            expected.clear();
        } else {
            let complete = assembly.push(&chunk.body, frame.chunk == ChunkType::Final, limits)?;
            expected.extend_from_slice(&chunk.body);
            result.peak = result.peak.max(assembly.len());
            if let Some(body) = complete {
                result
                    .bodies
                    .push((body.to_vec(), std::mem::take(&mut expected)));
            }
        }
    }
    Ok(result)
}

/// Map one binary Variant with a selected temporal suffix through the production value layer.
///
/// # Errors
/// Returns codec errors for malformed input, named Unsupported for deferred kinds,
/// `InvalidDataType` for unrepresentable overrides, and `ResourceLimit` for depth/node bounds.
pub fn value(bytes: &[u8], selector: u8) -> Result<crate::PlcValue> {
    use crate::opcua::OpcuaDataType;
    let mut reader = BoundedReader::new(bytes);
    let value = Variant::parse(&mut reader, Depth::new())?;
    reader.expect_eof()?;
    let target = match selector % 9 {
        0 => OpcuaDataType::Null,
        1 => OpcuaDataType::Time,
        2 => OpcuaDataType::LTime,
        3 => OpcuaDataType::Date,
        4 => OpcuaDataType::LDate,
        5 => OpcuaDataType::TimeOfDay,
        6 => OpcuaDataType::LTimeOfDay,
        7 => OpcuaDataType::DateAndTime,
        _ => OpcuaDataType::LDateAndTime,
    };
    let mut budget = crate::opcua::value::MAX_VALUE_NODES;
    crate::opcua::value::decode(value, target, &mut budget)
}

/// Authenticate one bounded symmetric chunk, then re-protect and reopen it with peer keys.
/// Two selector bytes choose the policy and `Sign`/`SignEncrypt`; the rest is complete wire data.
/// Nonces are fixed public fuzz fixtures, never production defaults.
///
/// # Errors
/// The outer result refuses malformed/untrusted input. An accepted chunk's inner result
/// must succeed and be true: it checks the canonical protection/peer reopening invariant.
pub fn symmetric_chunk(input: &[u8]) -> Result<Result<bool>> {
    use crate::opcua::crypto::{Native, Symmetric};
    use crate::opcua::options::{MessageSecurity, SecurityPolicy};
    let (selector, input) = input.split_first().ok_or(ParseError::Truncated {
        needed: 1,
        available: 0,
    })?;
    let policy = match selector % 3 {
        0 => SecurityPolicy::Basic256Sha256,
        1 => SecurityPolicy::Aes128Sha256RsaOaep,
        _ => SecurityPolicy::Aes256Sha256RsaPss,
    };
    let (mode, bytes) = input.split_first().ok_or(ParseError::Truncated {
        needed: 1,
        available: 0,
    })?;
    let mode = if mode % 2 == 0 {
        MessageSecurity::Sign
    } else {
        MessageSecurity::SignEncrypt
    };
    let client = Symmetric::new(&Native, policy, mode, 17, 29, &[1; 32], &[2; 32])?;
    let frame = client.open(&Native, bytes, 16_384)?;
    Ok((|| {
        let rewritten = client.seal(&Native, &frame, 16_384)?;
        let peer = Symmetric::new(&Native, policy, mode, 17, 29, &[2; 32], &[1; 32])?;
        Ok(peer.open(&Native, &rewritten, 16_384)? == frame)
    })())
}
