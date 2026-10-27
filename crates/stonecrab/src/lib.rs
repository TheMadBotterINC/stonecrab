#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

pub use stonecrab_core::{
    AccessPolicy, AddressOptions, AddressOptionsBuilder, ByteOrder, CancellationToken,
    Capabilities, Clock, Connection, ConnectionConfig, ConnectionConfigBuilder, ConnectionState,
    ConnectionUri, Credentials, Deadline, Diagnostics, Driver, DriverOptions, DriverRegistry,
    Endpoint, ErrorKind, FailureCategory, ItemError, OpFuture, Operation, PlcError, PlcValue,
    PlcValueType, ProtocolAddress, ProtocolStatus, ReadItem, ReadRequest, ReadRequestBuilder,
    ReadResponse, ReadResponseBuilder, ReconnectPolicy, ResourceLimits, ResourceLimitsBuilder,
    Result, RetryPolicy, Secret, StateCell, StateWatch, SubscriptionMode, Tag, TagId, Timeouts,
    UnknownReason, WordOrder, WriteItem, WriteOutcome, WriteRequest, WriteRequestBuilder,
    WriteResponse, WriteResponseBuilder, WriteResult,
};

/// The in-process simulator driver, behind the `sim` feature.
///
/// This module re-exports every public item of the `stonecrab-driver-sim` crate
/// (`pub use stonecrab_driver_sim::*;`), so `stonecrab::sim::SimDriver` is the type
/// `stonecrab_driver_sim::SimDriver` and a new simulator item appears here without a
/// facade change. The facade version pins the sibling crate exactly, so the glob cannot
/// drift from the version the facade was published with.
///
/// The `sim` scheme is registered by the application, not by the facade: build a
/// [`DriverRegistry`], register a [`SimDriver`](sim::SimDriver) holding the
/// [`SimDevice`](sim::SimDevice)s you want, and connect with `sim://<device>`.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab::sim::{SimDevice, SimDriver, SimTag};
/// use stonecrab::{DriverRegistry, PlcValue};
///
/// let device = SimDevice::new();
/// device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(SimDriver::new().with_device("bench", device)?))?;
/// assert_eq!(registry.schemes(), ["sim"]);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[cfg(feature = "sim")]
#[cfg_attr(docsrs, doc(cfg(feature = "sim")))]
pub mod sim {
    pub use stonecrab_driver_sim::*;
}

/// The native Modbus TCP driver, behind the `modbus` feature.
///
/// This module re-exports every public item of the `stonecrab-modbus` crate
/// (`pub use stonecrab_modbus::*;`), so `stonecrab::modbus::ModbusTcpDriver` is the type
/// `stonecrab_modbus::ModbusTcpDriver`, and so is [`SystemClock`](modbus::SystemClock), the
/// Tokio-backed clock the driver uses by default. The facade version pins the sibling crate
/// exactly, so the glob cannot drift from the version the facade was published with.
///
/// The `modbus-tcp` scheme is registered by the application, not by the facade: build a
/// [`DriverRegistry`], register a [`ModbusTcpDriver`](modbus::ModbusTcpDriver) and connect
/// with `modbus-tcp://host[:port]`. The driver opens real sockets and must run inside a
/// Tokio runtime with the time driver enabled; the `modbus_tcp_read` example shows the
/// whole path. Alpha scope is read-only (function codes 01 to 04).
///
/// # Examples
/// Parsing opens no socket. The configuration is parsed first so that its address options
/// can be taken before `connect` consumes it.
/// ```
/// use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver};
/// use stonecrab::{Driver, ProtocolAddress};
///
/// let driver = ModbusTcpDriver::new();
/// let config = driver
///     .parse_config("modbus-tcp://192.0.2.10:1502?address-convention=plc4x")?;
/// let options = config.address_options();
/// let address = driver.parse_address("40001:UINT", &options)?;
/// assert_eq!(address.canonical(), "holding-register:1:UINT");
/// assert!(config.driver_options_as::<ModbusOptions>().is_some());
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[cfg(feature = "modbus")]
#[cfg_attr(docsrs, doc(cfg(feature = "modbus")))]
pub mod modbus {
    pub use stonecrab_modbus::*;
}
