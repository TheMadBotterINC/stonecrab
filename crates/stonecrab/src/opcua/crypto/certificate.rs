//! Local certificate properties for direct trust; webpki owns X.509/path validation.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aws_lc_rs::rsa::PublicEncryptingKey;
use aws_lc_rs::signature::{
    RSA_PKCS1_2048_8192_SHA256, RSA_PSS_2048_8192_SHA256, UnparsedPublicKey,
};
use rustls_pki_types::{
    AlgorithmIdentifier, CertificateDer, InvalidSignature, SignatureVerificationAlgorithm, alg_id,
};

use super::untrusted;
use crate::codec::BoundedReader;
use crate::opcua::options::SecurityPolicy;
use crate::{PlcDate, PlcTimeOfDay, Result};

pub(super) const MAX_CERT_BYTES: usize = 1 << 20;

#[derive(Debug)]
pub(super) enum RsaSha256 {
    Pkcs1,
    Pss,
}

pub(super) static PKCS1_SHA256: RsaSha256 = RsaSha256::Pkcs1;
pub(super) static PSS_SHA256: RsaSha256 = RsaSha256::Pss;

impl SignatureVerificationAlgorithm for RsaSha256 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> std::result::Result<(), InvalidSignature> {
        let algorithm = match self {
            Self::Pkcs1 => &RSA_PKCS1_2048_8192_SHA256,
            Self::Pss => &RSA_PSS_2048_8192_SHA256,
        };
        UnparsedPublicKey::new(algorithm, public_key)
            .verify(message, signature)
            .map_err(|_error| InvalidSignature)
    }
    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        alg_id::RSA_ENCRYPTION
    }
    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        match self {
            Self::Pkcs1 => alg_id::RSA_PKCS1_SHA256,
            Self::Pss => alg_id::RSA_PSS_SHA256,
        }
    }
}

/// Trust bypass never supplies a non-RSA or out-of-policy key to the later OPN code.
pub(in crate::opcua) fn server_key(
    der: &CertificateDer<'_>,
    policy: SecurityPolicy,
) -> Result<PublicEncryptingKey> {
    let cert = webpki::EndEntityCert::try_from(der)
        .ok()
        .ok_or_else(untrusted)?;
    rsa_key(&cert, policy)
}

fn rsa_key(
    cert: &webpki::EndEntityCert<'_>,
    policy: SecurityPolicy,
) -> Result<PublicEncryptingKey> {
    let key = PublicEncryptingKey::from_der(cert.subject_public_key_info().as_ref())
        .ok()
        .ok_or_else(untrusted)?;
    if !policy.descriptor().accepts_rsa_key(key.key_size_bytes()) {
        return Err(untrusted());
    }
    Ok(key)
}

pub(in crate::opcua) fn check_leaf<'a>(
    der: &'a CertificateDer<'a>,
    policy: SecurityPolicy,
    now: SystemTime,
) -> Result<webpki::EndEntityCert<'a>> {
    let cert = webpki::EndEntityCert::try_from(der)
        .ok()
        .ok_or_else(untrusted)?;
    rsa_key(&cert, policy)?;
    let properties = Properties::parse(der.as_ref()).ok_or_else(untrusted)?;
    if !properties.current(now) || !properties.server_auth {
        return Err(untrusted());
    }
    Ok(cert)
}

pub(super) fn current(der: &CertificateDer<'_>, now: SystemTime) -> bool {
    Properties::parse(der.as_ref()).is_some_and(|properties| properties.current(now))
}

pub(in crate::opcua) struct Properties {
    not_before: SystemTime,
    not_after: SystemTime,
    server_auth: bool,
}

impl Properties {
    fn current(&self, now: SystemTime) -> bool {
        self.not_before <= now && now <= self.not_after
    }

    pub(in crate::opcua) fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_CERT_BYTES {
            return None;
        }
        let mut outer = BoundedReader::new(bytes);
        let mut certificate = BoundedReader::new(element(&mut outer, 0x30)?);
        outer.expect_eof().ok()?;
        let mut tbs = BoundedReader::new(element(&mut certificate, 0x30)?);
        element(&mut certificate, 0x30)?;
        element(&mut certificate, 0x03)?;
        certificate.expect_eof().ok()?;
        if element(&mut tbs, 0xa0)? != [0x02, 0x01, 0x02] {
            return None;
        }
        element(&mut tbs, 0x02)?;
        element(&mut tbs, 0x30)?;
        element(&mut tbs, 0x30)?;
        let mut validity = BoundedReader::new(element(&mut tbs, 0x30)?);
        let (tag, value) = tlv(&mut validity)?;
        let not_before = time(tag, value)?;
        let (tag, value) = tlv(&mut validity)?;
        let not_after = time(tag, value)?;
        validity.expect_eof().ok()?;
        element(&mut tbs, 0x30)?;
        element(&mut tbs, 0x30)?;
        let mut server_auth = true;
        while tbs.remaining() != 0 {
            let (tag, value) = tlv(&mut tbs)?;
            if tag == 0xa3 {
                let mut explicit = BoundedReader::new(value);
                let mut extensions = BoundedReader::new(element(&mut explicit, 0x30)?);
                explicit.expect_eof().ok()?;
                while extensions.remaining() != 0 {
                    let mut extension = BoundedReader::new(element(&mut extensions, 0x30)?);
                    let oid = element(&mut extension, 0x06)?;
                    let (tag, value) = tlv(&mut extension)?;
                    let value = if tag == 0x01 {
                        element(&mut extension, 0x04)?
                    } else if tag == 0x04 {
                        value
                    } else {
                        return None;
                    };
                    extension.expect_eof().ok()?;
                    if oid == [0x55, 0x1d, 0x25] {
                        server_auth = server_eku(value)?;
                    }
                }
            }
        }
        Some(Self {
            not_before,
            not_after,
            server_auth,
        })
    }
}

pub(in crate::opcua) fn server_eku(bytes: &[u8]) -> Option<bool> {
    let mut outer = BoundedReader::new(bytes);
    let mut purposes = BoundedReader::new(element(&mut outer, 0x30)?);
    outer.expect_eof().ok()?;
    let mut server = false;
    while purposes.remaining() != 0 {
        server |= element(&mut purposes, 0x06)? == [0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01];
    }
    Some(server)
}

fn element<'a>(reader: &mut BoundedReader<'a>, expected: u8) -> Option<&'a [u8]> {
    let (tag, value) = tlv(reader)?;
    (tag == expected).then_some(value)
}

/// Only single-octet tags and canonical definite DER lengths are needed by X.509 fields.
pub(in crate::opcua) fn tlv<'a>(reader: &mut BoundedReader<'a>) -> Option<(u8, &'a [u8])> {
    let tag = reader.read_u8().ok()?;
    if tag & 0x1f == 0x1f {
        return None;
    }
    let first = reader.read_u8().ok()?;
    let length = if first < 0x80 {
        usize::from(first)
    } else {
        let count = first & 0x7f;
        if !(1..=4).contains(&count) {
            return None;
        }
        let bytes = reader.read_bytes(usize::from(count), 4).ok()?;
        if bytes.first() == Some(&0) {
            return None;
        }
        let mut word = [0; 4];
        for (slot, byte) in word.iter_mut().rev().zip(bytes.iter().rev()) {
            *slot = *byte;
        }
        let length = usize::try_from(u32::from_be_bytes(word)).ok()?;
        if length < 0x80 {
            return None;
        }
        length
    };
    Some((tag, reader.read_bytes(length, MAX_CERT_BYTES).ok()?))
}

fn digits(reader: &mut BoundedReader<'_>, width: usize) -> Option<u32> {
    reader
        .read_bytes(width, 4)
        .ok()?
        .iter()
        .try_fold(0_u32, |value, byte| {
            byte.is_ascii_digit().then_some(())?;
            value
                .checked_mul(10)?
                .checked_add(u32::from(byte.checked_sub(b'0')?))
        })
}

pub(in crate::opcua) fn time(tag: u8, bytes: &[u8]) -> Option<SystemTime> {
    let mut reader = BoundedReader::new(bytes);
    let year = match tag {
        0x17 => {
            let short = digits(&mut reader, 2)?;
            short.checked_add(if short >= 50 { 1900 } else { 2000 })?
        }
        0x18 => {
            let year = digits(&mut reader, 4)?;
            if year < 2050 {
                return None;
            }
            year
        }
        _ => return None,
    };
    let month = u8::try_from(digits(&mut reader, 2)?).ok()?;
    let day = u8::try_from(digits(&mut reader, 2)?).ok()?;
    let hour = u8::try_from(digits(&mut reader, 2)?).ok()?;
    let minute = u8::try_from(digits(&mut reader, 2)?).ok()?;
    let second = u8::try_from(digits(&mut reader, 2)?).ok()?;
    if reader.read_u8().ok()? != b'Z' {
        return None;
    }
    reader.expect_eof().ok()?;
    let date = PlcDate::from_ymd(i32::try_from(year).ok()?, month, day).ok()?;
    let clock = PlcTimeOfDay::from_hms_nano(hour, minute, second, 0).ok()?;
    let midnight = Duration::from_hours(24)
        .checked_mul(date.days_since_epoch().unsigned_abs())
        .and_then(|offset| {
            if date.days_since_epoch().is_negative() {
                UNIX_EPOCH.checked_sub(offset)
            } else {
                UNIX_EPOCH.checked_add(offset)
            }
        });
    midnight.and_then(|midnight| {
        midnight.checked_add(Duration::from_nanos(clock.nanos_since_midnight()))
    })
}

#[cfg(test)]
mod tests;
