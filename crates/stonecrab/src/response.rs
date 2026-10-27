use std::fmt;
use std::num::NonZeroUsize;
use std::time::{Instant, SystemTime};

use bytes::Bytes;

use crate::{ErrorKind, PlcError, PlcValue, Result, TagId};

/// Original device status alongside normalized error classification.
///
/// Protocol and symbolic name are driver-owned metadata, not untrusted payload.
/// Raw bytes are available explicitly but never included in [`fmt::Debug`].
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProtocolStatus {
    protocol: &'static str,
    code: u32,
    name: Option<&'static str>,
    raw: Option<Bytes>,
}

impl ProtocolStatus {
    /// Retain an original protocol code without guessing its meaning.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ProtocolStatus;
    /// let status = ProtocolStatus::new("modbus", 2).with_name("illegal data address");
    /// assert_eq!(status.code(), 2);
    /// assert_eq!(status.protocol(), "modbus");
    /// ```
    #[must_use]
    pub const fn new(protocol: &'static str, code: u32) -> Self {
        Self {
            protocol,
            code,
            name: None,
            raw: None,
        }
    }

    /// Attach a driver's symbolic name; never use a device-supplied secret here.
    #[must_use]
    pub const fn with_name(mut self, name: &'static str) -> Self {
        self.name = Some(name);
        self
    }

    /// Copy raw diagnostic bytes only after checking their explicit budget.
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] before allocating if `raw` exceeds
    /// `max_bytes`. Empty raw data is retained distinctly from absent data.
    #[must_use = "raw diagnostic bounds must be checked"]
    pub fn with_raw(mut self, raw: &[u8], max_bytes: NonZeroUsize) -> Result<Self> {
        if raw.len() > max_bytes.get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "protocol status bytes exceed diagnostic budget",
            ));
        }
        self.raw = Some(Bytes::copy_from_slice(raw));
        Ok(self)
    }

    /// Driver's protocol identifier.
    #[must_use]
    pub const fn protocol(&self) -> &'static str {
        self.protocol
    }

    /// Original status code without normalization.
    #[must_use]
    pub const fn code(&self) -> u32 {
        self.code
    }

    /// Driver's symbolic status name when known.
    #[must_use]
    pub const fn name(&self) -> Option<&'static str> {
        self.name
    }

    /// Explicit access to potentially sensitive diagnostic bytes.
    #[must_use]
    pub fn raw(&self) -> Option<&[u8]> {
        self.raw.as_deref()
    }
}

impl fmt::Debug for ProtocolStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProtocolStatus")
            .field("protocol", &self.protocol)
            .field("code", &self.code)
            .field("name", &self.name)
            .field("raw_bytes", &self.raw.as_ref().map(Bytes::len))
            .finish()
    }
}

/// Failure of one requested item, not necessarily failure of the whole batch.
///
/// The original error source and protocol status are retained independently.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[error(transparent)]
pub struct ItemError {
    inner: Box<ItemErrorInner>,
}

#[derive(Debug, thiserror::Error)]
#[error("{error}")]
struct ItemErrorInner {
    #[source]
    error: PlcError,
    status: Option<ProtocolStatus>,
}

impl ItemError {
    /// Wrap an already-redacted error without inventing a protocol status.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, ItemError, PlcError};
    /// let error = ItemError::new(PlcError::new(ErrorKind::DeviceException, "address rejected"));
    /// assert_eq!(error.kind(), ErrorKind::DeviceException);
    /// assert!(error.status().is_none());
    /// ```
    #[must_use]
    pub fn new(error: PlcError) -> Self {
        Self {
            inner: Box::new(ItemErrorInner {
                error,
                status: None,
            }),
        }
    }

    /// Retain the device's original status without changing the normalized kind.
    #[must_use]
    pub fn with_status(mut self, status: ProtocolStatus) -> Self {
        self.inner.status = Some(status);
        self
    }

    /// Normalized item-failure kind.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.inner.error.kind()
    }

    /// Original typed failure and its diagnostic/source chain.
    #[must_use]
    pub const fn error(&self) -> &PlcError {
        &self.inner.error
    }

    /// Original protocol status, when supplied by the driver.
    #[must_use]
    pub const fn status(&self) -> Option<&ProtocolStatus> {
        self.inner.status.as_ref()
    }
}

/// One correlated read result, including failures and honest timestamp absence.
#[derive(Debug)]
#[non_exhaustive]
pub struct ReadItem {
    tag: TagId,
    outcome: std::result::Result<PlcValue, ItemError>,
    sampled_at: Instant,
    source_time: Option<SystemTime>,
    successful_status: Option<ProtocolStatus>,
}

impl ReadItem {
    /// Record one result at its actual local sampling time.
    ///
    /// Source time defaults to absent, never to the local clock.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab::{PlcValue, ReadItem, TagId};
    /// let item = ReadItem::new(TagId::new("counter")?, Ok(PlcValue::U64(42)), Instant::now());
    /// assert_eq!(item.outcome().as_ref().ok().and_then(PlcValue::as_u64), Some(42));
    /// assert!(item.source_time().is_none());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub const fn new(
        tag: TagId,
        outcome: std::result::Result<PlcValue, ItemError>,
        sampled_at: Instant,
    ) -> Self {
        Self {
            tag,
            outcome,
            sampled_at,
            source_time: None,
            successful_status: None,
        }
    }

    /// Attach a timestamp actually supplied by the source device.
    #[must_use]
    pub const fn with_source_time(mut self, source_time: SystemTime) -> Self {
        self.source_time = Some(source_time);
        self
    }

    /// Attach original status to either the successful item or its item error.
    ///
    /// For failures this replaces the error's status, keeping one source of truth.
    #[must_use]
    pub fn with_status(mut self, status: ProtocolStatus) -> Self {
        match &mut self.outcome {
            Ok(_) => self.successful_status = Some(status),
            Err(error) => error.inner.status = Some(status),
        }
        self
    }

    /// Original caller-supplied identity.
    #[must_use]
    pub const fn tag(&self) -> &TagId {
        &self.tag
    }

    /// Successful value or individual failure without collapsing a batch.
    #[must_use]
    pub const fn outcome(&self) -> &std::result::Result<PlcValue, ItemError> {
        &self.outcome
    }

    /// Local monotonic sampling time.
    #[must_use]
    pub const fn sampled_at(&self) -> Instant {
        self.sampled_at
    }

    /// Actual device timestamp, absent when the device supplied none.
    #[must_use]
    pub const fn source_time(&self) -> Option<SystemTime> {
        self.source_time
    }

    /// Original status, using the item error's status for a failed read.
    #[must_use]
    pub const fn status(&self) -> Option<&ProtocolStatus> {
        match &self.outcome {
            Ok(_) => self.successful_status.as_ref(),
            Err(error) => error.status(),
        }
    }
}

/// Why a transmitted write cannot be confirmed or safely replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[expect(
    clippy::enum_variant_names,
    reason = "ADR-0003: after-transmit names prevent confusing ambiguous writes with safe pre-transmit failures"
)]
pub enum UnknownReason {
    /// A response deadline elapsed after bytes may have reached the device.
    TimeoutAfterTransmit,
    /// The connection was lost after bytes may have reached the device.
    DisconnectedAfterTransmit,
    /// Cancellation occurred after bytes may have reached the device.
    CancelledAfterTransmit,
}

/// Explicit write disposition; unknown is never a rejection or retry request.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "inspect write outcomes, especially unknown delivery"]
#[expect(
    variant_size_differences,
    reason = "ADR-0003: rejected carries one boxed error; boxing the scalar unknown reason would only add an allocation"
)]
pub enum WriteOutcome {
    /// The device explicitly confirmed completion.
    Confirmed,
    /// The device or local validation explicitly rejected the write.
    Rejected(ItemError),
    /// The device may have applied the write; never automatically replay it.
    Unknown(UnknownReason),
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::Duration;

    use super::*;

    fn id() -> TagId {
        TagId::new("counter").expect("valid fixture identity")
    }

    #[test]
    fn protocol_status_preserves_original_metadata_and_bounded_raw_bytes() {
        let status = ProtocolStatus::new("modbus", 2).with_name("illegal data address");
        assert_eq!(status.protocol(), "modbus", "protocol preserved");
        assert_eq!(status.code(), 2, "code preserved");
        assert_eq!(
            status.name(),
            Some("illegal data address"),
            "name preserved"
        );
        assert_eq!(status.raw(), None, "raw absence preserved");
        let unnamed = ProtocolStatus::new("modbus", 0);
        assert_eq!(unnamed.name(), None, "no invented name");
        let raw = status
            .with_raw(&[0x83, 2], NonZeroUsize::new(2).expect("positive"))
            .expect("fits exactly");
        assert_eq!(
            raw.raw(),
            Some([0x83, 2].as_slice()),
            "exact bytes preserved"
        );
        assert_eq!(raw.clone(), raw, "cloned bytes preserved");
        let empty = unnamed
            .with_raw(&[], NonZeroUsize::MIN)
            .expect("empty fits");
        assert_eq!(
            empty.raw(),
            Some([].as_slice()),
            "empty differs from absent"
        );
    }

    // covers: SPEC mandatory-failure-cases / resource limits and diagnostic redaction
    #[test]
    fn diagnostic_bytes_are_bounded_and_never_debug_formatted() {
        let error = ProtocolStatus::new("fixture", 1)
            .with_raw(b"sensitive", NonZeroUsize::MIN)
            .expect_err("oversize rejected");
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "typed budget failure"
        );
        assert!(
            !error.to_string().contains("sensitive"),
            "error contains no raw data"
        );
        let status = ProtocolStatus::new("fixture", 1)
            .with_raw(b"sensitive", NonZeroUsize::new(9).expect("positive"))
            .expect("fits");
        let debug = format!("{status:?}");
        assert!(!debug.contains("sensitive"), "no payload text");
        assert!(!debug.contains("115"), "no numeric payload bytes");
        assert!(
            debug.contains("raw_bytes: Some(9)"),
            "length remains diagnosable"
        );
    }

    #[test]
    fn item_error_preserves_normalized_error_and_concrete_source_chain() {
        let error = ItemError::new(
            PlcError::new(ErrorKind::DeviceException, "address rejected")
                .with_source(std::io::Error::from(std::io::ErrorKind::InvalidInput)),
        );
        assert!(error.status().is_none(), "no invented status");
        assert_eq!(
            error.kind(),
            ErrorKind::DeviceException,
            "classification retained"
        );
        assert_eq!(error.to_string(), "address rejected", "display retained");
        assert_eq!(
            error.error().kind(),
            ErrorKind::DeviceException,
            "original typed error retained"
        );
        assert!(
            error.source().expect("wrapped error").is::<PlcError>(),
            "source type retained"
        );
        let error = error.with_status(ProtocolStatus::new("modbus", 2));
        assert_eq!(
            error.status().expect("device status").code(),
            2,
            "status retained"
        );
    }

    #[test]
    fn successful_read_preserves_identity_sampling_and_actual_device_time() {
        let now = Instant::now();
        let item = ReadItem::new(id(), Ok(PlcValue::U64(u64::MAX)), now);
        assert_eq!(item.tag(), &id(), "correlation retained");
        assert_eq!(item.sampled_at(), now, "sampling instant retained");
        assert!(item.source_time().is_none(), "no invented device timestamp");
        assert!(item.status().is_none(), "no invented device status");
        assert_eq!(
            item.outcome().as_ref().expect("successful").as_u64(),
            Some(u64::MAX),
            "value exact"
        );
        let source = SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(42))
            .expect("representable");
        let item = item
            .with_source_time(source)
            .with_status(ProtocolStatus::new("fixture", 0));
        assert_eq!(
            item.source_time(),
            Some(source),
            "actual device time retained"
        );
        assert_eq!(
            item.status().expect("success status").code(),
            0,
            "success status retained"
        );
    }

    #[test]
    fn failed_read_uses_one_authoritative_protocol_status() {
        let error = ItemError::new(PlcError::new(
            ErrorKind::DeviceException,
            "device rejected address",
        ))
        .with_status(ProtocolStatus::new("modbus", 1));
        let item = ReadItem::new(id(), Err(error), Instant::now());
        assert_eq!(
            item.status().expect("original status").code(),
            1,
            "error status visible"
        );
        let item = item.with_status(ProtocolStatus::new("modbus", 2));
        assert_eq!(
            item.status().expect("updated status").code(),
            2,
            "updated status visible"
        );
        assert_eq!(
            item.outcome()
                .as_ref()
                .expect_err("failed")
                .status()
                .expect("same status")
                .code(),
            2,
            "no conflicting copies"
        );
    }

    // covers: SPEC mandatory-failure-cases / ambiguous writes
    #[test]
    fn transmitted_write_failures_remain_unknown_not_confirmed_or_rejected() {
        for reason in [
            UnknownReason::TimeoutAfterTransmit,
            UnknownReason::DisconnectedAfterTransmit,
            UnknownReason::CancelledAfterTransmit,
        ] {
            let outcome = WriteOutcome::Unknown(reason);
            assert!(
                matches!(outcome, WriteOutcome::Unknown(actual) if actual == reason),
                "ambiguity reason preserved"
            );
        }
        assert!(
            matches!(WriteOutcome::Confirmed, WriteOutcome::Confirmed),
            "confirmation is explicit"
        );
        let rejection = WriteOutcome::Rejected(ItemError::new(PlcError::new(
            ErrorKind::InvalidConfig,
            "read-only policy",
        )));
        assert!(
            matches!(rejection, WriteOutcome::Rejected(error) if error.kind() == ErrorKind::InvalidConfig),
            "explicit rejection retained"
        );
    }
}
