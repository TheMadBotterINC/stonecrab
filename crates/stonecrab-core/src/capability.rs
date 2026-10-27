use std::num::NonZeroUsize;

/// A protocol operation, independent of any transport or executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Operation {
    /// Establish a connection.
    Connect,
    /// Read tagged values.
    Read,
    /// Write tagged values.
    Write,
    /// Deliver subscription events.
    Subscribe,
    /// Enumerate an address hierarchy.
    Browse,
    /// Locate devices.
    Discover,
    /// Release a connection explicitly.
    Close,
}

/// How a driver obtains subscription updates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SubscriptionMode {
    /// Subscriptions are unavailable.
    #[default]
    Unsupported,
    /// Updates originate from the device's subscription mechanism.
    Native,
    /// Updates are acquired by scheduled reads, not native subscriptions.
    Polling,
}

/// Advertised operation support and device limits.
///
/// The default advertises no data operations, one concurrent operation, and no
/// advertised device limits. `None` means unknown, not permission to allocate
/// without a bound. Connection resource limits must be applied separately.
///
/// # Examples
/// ```
/// use stonecrab_core::{Capabilities, Operation};
/// let mut capabilities = Capabilities::default();
/// capabilities.read = true;
/// assert!(capabilities.supports(Operation::Read));
/// assert!(!capabilities.supports(Operation::Write));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Capabilities {
    /// Reads are implemented.
    pub read: bool,
    /// Writes are implemented; application policy may still prohibit them.
    pub write: bool,
    /// Actual subscription mechanism, if any.
    pub subscribe: SubscriptionMode,
    /// Hierarchical browsing is implemented.
    pub browse: bool,
    /// Explicit device discovery is implemented.
    pub discover: bool,
    /// Advertised device item limit; unknown when absent.
    pub max_items_per_request: Option<NonZeroUsize>,
    /// Advertised device PDU limit; unknown when absent.
    pub max_pdu_bytes: Option<NonZeroUsize>,
    /// Maximum number of simultaneous operations the device can handle.
    pub concurrency: NonZeroUsize,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            read: false,
            write: false,
            subscribe: SubscriptionMode::Unsupported,
            browse: false,
            discover: false,
            max_items_per_request: None,
            max_pdu_bytes: None,
            concurrency: NonZeroUsize::MIN,
        }
    }
}

impl Capabilities {
    /// Whether the driver advertises an operation.
    ///
    /// Connect and close are mandatory lifecycle operations. This is a capability
    /// query, not an authorization decision or a guarantee that a device is reachable.
    #[must_use]
    pub const fn supports(&self, operation: Operation) -> bool {
        match operation {
            Operation::Connect | Operation::Close => true,
            Operation::Read => self.read,
            Operation::Write => self.write,
            Operation::Subscribe => !matches!(self.subscribe, SubscriptionMode::Unsupported),
            Operation::Browse => self.browse,
            Operation::Discover => self.discover,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_report_each_operation_without_enabling_others() {
        let mut capabilities = Capabilities::default();
        for operation in [
            Operation::Read,
            Operation::Write,
            Operation::Subscribe,
            Operation::Browse,
            Operation::Discover,
        ] {
            assert!(
                !capabilities.supports(operation),
                "{operation:?} defaults to unsupported"
            );
        }
        assert!(
            capabilities.supports(Operation::Connect),
            "connect is mandatory"
        );
        assert!(
            capabilities.supports(Operation::Close),
            "close is mandatory"
        );
        assert_eq!(
            capabilities.max_items_per_request, None,
            "unknown item limit"
        );
        assert_eq!(capabilities.max_pdu_bytes, None, "unknown PDU limit");
        capabilities.read = true;
        capabilities.write = true;
        capabilities.browse = true;
        capabilities.discover = true;
        for operation in [
            Operation::Read,
            Operation::Write,
            Operation::Browse,
            Operation::Discover,
        ] {
            assert!(
                capabilities.supports(operation),
                "{operation:?} is now advertised"
            );
        }
        for mode in [SubscriptionMode::Native, SubscriptionMode::Polling] {
            capabilities.subscribe = mode;
            assert!(
                capabilities.supports(Operation::Subscribe),
                "{mode:?} delivers updates"
            );
        }
    }
}
