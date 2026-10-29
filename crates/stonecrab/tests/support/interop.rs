//! The driver side of the interop tests, shared by `modbus_interop_pymodbus.rs` and
//! `modbus_interop_libmodbus.rs`: a [`Peer`] is a `ModbusTcpDriver` and its connection to
//! the server a URL names, with the calls the tests make on it and the readers of a
//! response. Which server and which URL belong to each test file, not to this module.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::sync::Arc;
use std::time::Duration;

use stonecrab::modbus::{ModbusTcpDriver, SystemClock};
use stonecrab::{
    AddressOptions, Clock, Connection, ConnectionConfig, Deadline, Driver, ItemError, PlcError,
    PlcValue, ReadRequest, ReadResponse, ResourceLimits, Tag, TagId, WriteRequest, WriteResponse,
};

/// The deadline of every read and write that does not name its own.
pub const DEADLINE: Duration = Duration::from_secs(2);

/// The most items any request of these tests carries: 2001 coils.
const MAX_ITEMS: usize = 2_001;

/// The configuration with the request item budget raised, everything else kept.
fn with_item_budget(config: &ConnectionConfig, items: usize) -> ConnectionConfig {
    let limits = ResourceLimits::builder()
        .max_items_per_request(items)
        .build()
        .expect("limits");
    let mut builder = ConnectionConfig::builder(config.scheme(), config.endpoint().clone())
        .timeouts(*config.timeouts())
        .reconnect(*config.reconnect())
        .retry(config.retry().clone())
        .access(config.access())
        .limits(limits);
    if let Some(name) = config.address_options().convention() {
        builder = builder.address_convention(name);
    }
    if let Some(options) = config.driver_options() {
        builder = builder.driver_options(Arc::clone(options));
    }
    builder.build().expect("the rebuilt configuration")
}

/// A driver, its connection to the server and the options addresses are parsed with.
pub struct Peer {
    driver: ModbusTcpDriver,
    options: AddressOptions,
    pub connection: Box<dyn Connection>,
}

impl Peer {
    /// Connect to the server `uri` names; the test fails with `hint` appended to the message
    /// when it cannot.
    pub async fn connect(uri: &str, hint: &str) -> Self {
        let driver = ModbusTcpDriver::new();
        let config = driver
            .parse_config(uri)
            .expect("the connection string parses");
        let config = with_item_budget(&config, MAX_ITEMS);
        let options = config.address_options();
        let connection = driver
            .connect(config)
            .await
            .unwrap_or_else(|error| panic!("cannot connect to {uri}: {error}. {hint}"));
        Self {
            driver,
            options,
            connection,
        }
    }

    pub fn tag(&self, id: &str, text: &str) -> Tag {
        let address = self
            .driver
            .parse_address(text, &self.options)
            .expect("the address parses");
        Tag::new(TagId::new(id).expect("tag id"), address)
    }

    /// Read the tags in one request with the [`DEADLINE`].
    pub async fn read(&self, tags: Vec<Tag>) -> ReadResponse {
        self.read_within(tags, DEADLINE).await
    }

    /// Read the tags in one request that must finish within `within` on the system clock.
    pub async fn read_within(&self, tags: Vec<Tag>, within: Duration) -> ReadResponse {
        self.try_read(tags, within)
            .await
            .expect("the read completes")
    }

    /// As [`read_within`](Self::read_within), but the failure of the whole read (a timeout,
    /// a lost link) is the result and not a test failure.
    pub async fn try_read(
        &self,
        tags: Vec<Tag>,
        within: Duration,
    ) -> Result<ReadResponse, PlcError> {
        let deadline =
            Deadline::after(SystemClock::default().now(), within).expect("a deadline that fits");
        let mut builder = ReadRequest::builder(self.connection.limits()).deadline(deadline);
        for tag in tags {
            builder = builder.tag(tag);
        }
        let request = builder.build().expect("the request is within its budgets");
        self.connection.read(request).await
    }

    /// Write the items in one request, with the same 2 s deadline as a read.
    pub async fn write(&self, items: Vec<(Tag, PlcValue)>) -> WriteResponse {
        let deadline =
            Deadline::after(SystemClock::default().now(), DEADLINE).expect("a deadline that fits");
        let mut builder = WriteRequest::builder(self.connection.limits()).deadline(deadline);
        for (tag, value) in items {
            builder = builder.item(tag, value);
        }
        let request = builder.build().expect("the request is within its budgets");
        self.connection
            .write(request)
            .await
            .expect("the write completes")
    }

    pub async fn close(self) {
        self.connection.close().await.expect("close");
    }
}

pub fn outcome<'a>(response: &'a ReadResponse, id: &str) -> &'a Result<PlcValue, ItemError> {
    response
        .get(&TagId::new(id).expect("tag id"))
        .unwrap_or_else(|| panic!("the response has an item for {id}"))
        .outcome()
}

/// The value of a tag that was read, or `None` when its item failed.
pub fn value_of(response: &ReadResponse, id: &str) -> Option<PlcValue> {
    outcome(response, id).as_ref().ok().cloned()
}

/// The error of a tag whose item failed; the test fails when it was read.
pub fn failure<'a>(response: &'a ReadResponse, id: &str) -> &'a ItemError {
    outcome(response, id)
        .as_ref()
        .expect_err("the item was expected to fail")
}
