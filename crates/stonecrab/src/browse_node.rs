use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{ErrorKind, ItemError, PlcError, PlcValueType, ProtocolAddress, Result};

/// One immutable node in a query's flat browse result.
///
/// Parent indices are local to that query's node list and validated by `BrowseResponse`'s
/// builder. Same-name siblings and repeated addresses remain distinct nodes. Address
/// implementations must satisfy `ProtocolAddress`'s validation and redaction contract.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BrowseNode {
    address: Arc<dyn ProtocolAddress>,
    name: String,
    parent: Option<usize>,
    readable: bool,
    writable: bool,
    value_type: Option<PlcValueType>,
    array_dimensions: Option<Vec<u32>>,
    options: BTreeMap<String, String>,
    error: Option<Arc<ItemError>>,
}

impl BrowseNode {
    /// Maximum UTF-8 bytes in a display name or metadata value.
    pub const MAX_TEXT_BYTES: usize = 8192;
    /// Maximum metadata key size in UTF-8 bytes.
    pub const MAX_OPTION_KEY_BYTES: usize = 256;
    /// Maximum distinct metadata options on a node.
    pub const MAX_OPTIONS: usize = 32;
    /// Maximum array rank retained as metadata; dimensions never allocate value storage.
    pub const MAX_ARRAY_DIMENSIONS: usize = 32;

    /// Retain a validated address and bounded display name with no declared access.
    ///
    /// Empty names and Unicode remain data. The address supplies the initial type hint;
    /// shape is unknown, and parent/options are absent.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{BrowseNode, ProtocolAddress};
    /// fn describe(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<BrowseNode> {
    ///     Ok(BrowseNode::new(address, "Process value")?.with_readable(true))
    /// }
    /// ```
    ///
    /// # Errors
    /// `ResourceLimit` before copying an oversized display name.
    pub fn new(address: Arc<dyn ProtocolAddress>, name: &str) -> Result<Self> {
        text(name, Self::MAX_TEXT_BYTES)?;
        let value_type = address.value_type_hint();
        Ok(Self {
            address,
            name: name.to_owned(),
            parent: None,
            readable: false,
            writable: false,
            value_type,
            array_dimensions: None,
            options: BTreeMap::new(),
            error: None,
        })
    }

    /// Set the parent index; response assembly requires it to precede this node.
    #[must_use]
    pub const fn with_parent(mut self, parent: usize) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Retain a failure to expand this node while preserving successful siblings.
    #[must_use]
    pub fn with_error(mut self, error: ItemError) -> Self {
        self.error = Some(Arc::new(error));
        self
    }

    /// Failure encountered when expanding this node, including its original status.
    #[must_use]
    pub fn error(&self) -> Option<&ItemError> {
        self.error.as_deref()
    }

    /// Declare whether the driver permits reads of this node's value.
    #[must_use]
    pub const fn with_readable(mut self, readable: bool) -> Self {
        self.readable = readable;
        self
    }

    /// Declare whether the driver permits writes of this node's value.
    #[must_use]
    pub const fn with_writable(mut self, writable: bool) -> Self {
        self.writable = writable;
        self
    }

    /// Attach a type annotation without contradicting the address's explicit type.
    ///
    /// # Errors
    /// `InvalidConfig` when the annotation conflicts with the address hint.
    pub fn with_type(mut self, value_type: PlcValueType) -> Result<Self> {
        if self
            .address
            .value_type_hint()
            .is_some_and(|hint| hint != value_type)
        {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "browse type conflicts with protocol address",
            ));
        }
        self.value_type = Some(value_type);
        Ok(self)
    }

    /// Retain bounded dimensions: empty means scalar, and zero extents mean unknown length.
    ///
    /// # Errors
    /// `ResourceLimit` before copying an excessive rank. No element count is inferred.
    pub fn with_array_dimensions(mut self, dimensions: &[u32]) -> Result<Self> {
        if dimensions.len() > Self::MAX_ARRAY_DIMENSIONS {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "browse array rank exceeds metadata budget",
            ));
        }
        self.array_dimensions = Some(dimensions.to_vec());
        Ok(self)
    }

    /// Attach bounded driver metadata; replacing an existing key does not spend another slot.
    ///
    /// # Errors
    /// `InvalidConfig` for an empty/control-bearing key, `ResourceLimit` for oversized text
    /// or too many distinct keys. Rejected keys/values are never echoed.
    pub fn with_option(mut self, key: &str, value: &str) -> Result<Self> {
        text(key, Self::MAX_OPTION_KEY_BYTES)?;
        if key.is_empty() || key.chars().any(char::is_control) {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "invalid browse metadata key",
            ));
        }
        text(value, Self::MAX_TEXT_BYTES)?;
        if self.options.len() >= Self::MAX_OPTIONS && !self.options.contains_key(key) {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "browse metadata exceeds option budget",
            ));
        }
        self.options.insert(key.to_owned(), value.to_owned());
        Ok(self)
    }

    /// Address reusable with `Tag::new`; no parsing or device access occurs here.
    #[must_use]
    pub fn address(&self) -> &Arc<dyn ProtocolAddress> {
        &self.address
    }
    /// Display name, not an identity or uniqueness key.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Parent index within this query, or None for a root.
    #[must_use]
    pub const fn parent(&self) -> Option<usize> {
        self.parent
    }
    /// Whether reads are declared possible.
    #[must_use]
    pub const fn readable(&self) -> bool {
        self.readable
    }
    /// Whether writes are declared possible.
    #[must_use]
    pub const fn writable(&self) -> bool {
        self.writable
    }
    /// Scalar/element type, when known.
    #[must_use]
    pub const fn value_type(&self) -> Option<PlcValueType> {
        self.value_type
    }
    /// None is unknown shape; Some(empty) is scalar; zero extents have unknown length.
    #[must_use]
    pub fn array_dimensions(&self) -> Option<&[u32]> {
        self.array_dimensions.as_deref()
    }
    /// Immutable ordered driver metadata.
    #[must_use]
    pub const fn options(&self) -> &BTreeMap<String, String> {
        &self.options
    }
}

fn text(text: &str, max: usize) -> Result<()> {
    if text.len() > max {
        return Err(PlcError::new(
            ErrorKind::ResourceLimit,
            "browse metadata exceeds text budget",
        ));
    }
    Ok(())
}
