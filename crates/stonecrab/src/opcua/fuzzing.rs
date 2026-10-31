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
}

/// Parse an `opcua` connection string as the driver does and report what was attached.
///
/// # Errors
/// As `OpcuaDriver::parse_config`: [`ErrorKind::InvalidConfig`](crate::ErrorKind) for text
/// that is not a valid `opcua` connection string.
pub fn parse_config(uri: &str) -> Result<Parsed> {
    let config = OpcuaDriver::parse_config(uri)?;
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
    Ok(Parsed {
        config,
        endpoint_url,
        security_policy,
        message_security,
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
    Ok(framer.decode(buffer)?.map(|decoded| {
        let mut out = BytesMut::new();
        Framed {
            written: framer.encode(&decoded, &mut out).map(|()| out.to_vec()),
        }
    }))
}
