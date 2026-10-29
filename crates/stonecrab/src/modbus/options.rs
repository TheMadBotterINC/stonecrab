//! `ModbusOptions`: the driver-owned connection parameters of the `modbus-tcp` scheme.

use std::num::{NonZeroU16, NonZeroU64};
use std::str::FromStr;
use std::time::Duration;

use crate::{
    AccessPolicy, AddressOptions, ByteOrder, DriverOptions, ErrorKind, PlcError, Result, WordOrder,
};

use crate::modbus::address::byte_order_by_name;
use crate::modbus::{ConnectionDefaults, ModbusAddress, UnitId};

/// `request-timeout-ms`.
const REQUEST_TIMEOUT: &str = "request-timeout-ms";
/// `default-unit-identifier`.
const UNIT_ID: &str = "default-unit-identifier";
/// `ping-address`.
const PING_ADDRESS: &str = "ping-address";
/// `default-payload-byte-order`.
const BYTE_ORDER: &str = "default-payload-byte-order";
/// `max-coils-per-request`.
const MAX_COILS: &str = "max-coils-per-request";
/// `max-registers-per-request`.
const MAX_REGISTERS: &str = "max-registers-per-request";
/// The shared key that selects the address convention.
const CONVENTION: &str = "address-convention";
/// `access`, the Stonecrab key that grants writes (`MB-CFG-ACCESS`).
const ACCESS: &str = "access";
/// The access value that grants writes; `read-only` is the other and the default.
const READ_WRITE: &str = "read-write";
const READ_ONLY: &str = "read-only";
/// The only convention name accepted at alpha.
const PLC4X: &str = "plc4x";

/// The most key characters an error message repeats.
const KEY_ECHO_LIMIT: usize = 32;

/// The six upstream connection parameters of the `modbus-tcp` driver, parsed and validated.
///
/// [`ModbusTcpDriver::parse_config`](crate::Driver::parse_config) reads them from
/// the query of a connection string and stores them in the [`ConnectionConfig`] through
/// [`ConnectionConfigBuilder::driver_options`](crate::ConnectionConfigBuilder::driver_options);
/// `connect` reads them back with
/// [`ConnectionConfig::driver_options_as`](crate::ConnectionConfig::driver_options_as).
/// A configuration built by hand carries none, and the driver then uses its own
/// [`ConnectionDefaults`]. A field the connection string does not name keeps the value
/// of the driver's defaults ([`from_defaults`](Self::from_defaults)), which are the
/// upstream defaults unless
/// [`ModbusTcpDriver::with_defaults`](crate::modbus::ModbusTcpDriver::with_defaults) replaced them;
/// [`Default`] is the configuration of `modbus-tcp://host` on a default driver.
///
/// | Field | Connection parameter | Default | Bounds |
/// | --- | --- | --- | --- |
/// | `request_timeout` | `request-timeout-ms` | 5000 ms | positive integer |
/// | `unit_id` | `default-unit-identifier` | 1 | 0 to 255 |
/// | `ping_address` | `ping-address` | `4x00001:BOOL` | a valid address under the `plc4x` convention |
/// | `byte_order`, `word_order` | `default-payload-byte-order` | `BIG_ENDIAN` | the four upstream names |
/// | `max_coils` | `max-coils-per-request` | 2000 | 1 to 2000 |
/// | `max_registers` | `max-registers-per-request` | 125 | 1 to 125 |
///
/// The byte-order names map to ([`ByteOrder`], [`WordOrder`]) pairs as ADR-0006 and the
/// parity record say: `BIG_ENDIAN` is (big, high word first), `LITTLE_ENDIAN` is (little,
/// low word first), `BIG_ENDIAN_BYTE_SWAP` is (little, high word first) and
/// `LITTLE_ENDIAN_BYTE_SWAP` is (big, low word first).
///
/// `request_timeout` is also stored in the configuration's `Timeouts.request`, which is
/// what the connection enforces. The `ping_address` is what
/// [`Connection::ping`](crate::Connection::ping) reads once (`MB-TCP-PING`).
///
/// The struct is `#[non_exhaustive]`; start from [`Default`] and assign the fields. Its
/// `Debug` output carries no secrets (it has none to carry), as [`DriverOptions`] requires.
///
/// # Compatibility: docs/parity/modbus.md#MB-CFG-REQUEST-TIMEOUT
/// The six rows `MB-CFG-REQUEST-TIMEOUT`, `MB-CFG-UNIT-ID`, `MB-CFG-PING-ADDRESS`,
/// `MB-CFG-BYTE-ORDER` (docs/parity/modbus.md#MB-CFG-BYTE-ORDER), `MB-CFG-MAX-COILS`
/// (docs/parity/modbus.md#MB-CFG-MAX-COILS) and `MB-CFG-MAX-REGISTERS`
/// (docs/parity/modbus.md#MB-CFG-MAX-REGISTERS) cover the fields. The unit range and the
/// two request-size bounds are Stonecrab limits (docs/parity/modbus.md#MB-CFG-UNIT-ID).
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::{ByteOrder, Driver, WordOrder};
/// use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver, UnitId};
///
/// let driver = ModbusTcpDriver::new();
/// let config = driver.parse_config(
///     "modbus-tcp://192.0.2.10:1502?default-unit-identifier=7&request-timeout-ms=800\
///      &default-payload-byte-order=LITTLE_ENDIAN",
/// )?;
/// let options = config
///     .driver_options_as::<ModbusOptions>()
///     .expect("parse_config attaches the options");
/// assert_eq!(options.unit_id, UnitId::from(7));
/// assert_eq!(options.request_timeout, Duration::from_millis(800));
/// assert_eq!(
///     (options.byte_order, options.word_order),
///     (ByteOrder::LittleEndian, WordOrder::LowWordFirst)
/// );
/// assert_eq!(options.max_registers.get(), 125, "an unnamed parameter keeps its default");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// [`ConnectionConfig`]: crate::ConnectionConfig
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ModbusOptions {
    /// How long one request waits for its reply (`request-timeout-ms`).
    pub request_timeout: Duration,
    /// The unit a read addresses when the tag's address names none
    /// (`default-unit-identifier`).
    pub unit_id: UnitId,
    /// The address a ping reads once (`ping-address`).
    pub ping_address: ModbusAddress,
    /// The byte order of a value whose tag and address name none
    /// (`default-payload-byte-order`).
    pub byte_order: ByteOrder,
    /// The word order of a value whose tag and address name none
    /// (`default-payload-byte-order`).
    pub word_order: WordOrder,
    /// The most coils or discrete inputs one wire request carries
    /// (`max-coils-per-request`).
    pub max_coils: NonZeroU16,
    /// The most registers one wire request carries (`max-registers-per-request`).
    pub max_registers: NonZeroU16,
}

impl Default for ModbusOptions {
    /// The upstream defaults: 5000 ms, unit 1, ping address `4x00001:BOOL`, `BIG_ENDIAN`,
    /// 2000 coils and 125 registers per request.
    fn default() -> Self {
        Self::from_defaults(ConnectionDefaults::default())
    }
}

impl DriverOptions for ModbusOptions {}

impl ModbusOptions {
    /// Options seeded from a driver's [`ConnectionDefaults`]: the unit, the byte and word
    /// order and the two request-size limits come from `defaults`, while `request_timeout`
    /// and `ping_address` keep the upstream defaults.
    ///
    /// This is what [`ModbusTcpDriver::parse_config`](crate::Driver::parse_config)
    /// starts from before the connection-string parameters override it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusOptions, UnitId};
    ///
    /// let mut defaults = ConnectionDefaults::default();
    /// defaults.unit_id = UnitId::from(9);
    /// let options = ModbusOptions::from_defaults(defaults);
    /// assert_eq!(options.unit_id, UnitId::from(9));
    /// assert_eq!(options.connection_defaults(), defaults);
    /// ```
    #[must_use]
    pub const fn from_defaults(defaults: ConnectionDefaults) -> Self {
        Self {
            request_timeout: Duration::from_millis(5000),
            unit_id: defaults.unit_id,
            ping_address: ModbusAddress::DEFAULT_PING,
            byte_order: defaults.byte_order,
            word_order: defaults.word_order,
            max_coils: defaults.max_coils,
            max_registers: defaults.max_registers,
        }
    }

    /// The connection-wide defaults these options stand for: the unit, the byte and word
    /// order and the two request-size limits, the part of the options a connection uses.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusOptions, UnitId};
    ///
    /// let mut options = ModbusOptions::default();
    /// options.unit_id = UnitId::from(9);
    /// assert_eq!(options.connection_defaults().unit_id, UnitId::from(9));
    /// ```
    #[must_use]
    pub const fn connection_defaults(&self) -> ConnectionDefaults {
        ConnectionDefaults {
            unit_id: self.unit_id,
            byte_order: self.byte_order,
            word_order: self.word_order,
            max_coils: self.max_coils,
            max_registers: self.max_registers,
        }
    }

    /// Read the query parameters of a connection string: the options, whether
    /// `address-convention=plc4x` was named and the access policy `access` names
    /// ([`AccessPolicy::ReadOnly`] when it is absent).
    ///
    /// The core grammar has already refused duplicate and empty keys, so each key appears
    /// once. Every key outside the six, `address-convention` and `access` is refused, which
    /// includes the misspelt `unit-identifier` of the upstream test suite.
    ///
    /// The parameters override `seed`; a key the string does not name keeps the seed's value.
    pub(crate) fn from_parameters(
        seed: Self,
        parameters: &[(String, String)],
    ) -> Result<(Self, bool, AccessPolicy)> {
        let mut options = seed;
        let mut plc4x = false;
        let mut access = AccessPolicy::ReadOnly;
        for (key, value) in parameters {
            match key.as_str() {
                REQUEST_TIMEOUT => {
                    let millis: u64 = number(REQUEST_TIMEOUT, value)?;
                    let millis = NonZeroU64::new(millis).ok_or_else(|| {
                        invalid(format!(
                            "{REQUEST_TIMEOUT} must be a whole number of milliseconds above 0"
                        ))
                    })?;
                    options.request_timeout = Duration::from_millis(millis.get());
                }
                UNIT_ID => {
                    let id: u64 = number(UNIT_ID, value)
                        .or_else(|refusal| digits_overflow(value, refusal))?;
                    let id = u8::try_from(id).map_err(|_range| {
                        invalid(format!("{UNIT_ID} must be a whole number from 0 to 255"))
                    })?;
                    options.unit_id = UnitId::from(id);
                }
                PING_ADDRESS => options.ping_address = ping_address(value)?,
                BYTE_ORDER => {
                    let (byte, word) = byte_order_by_name(value).ok_or_else(|| {
                        invalid(format!(
                            "{BYTE_ORDER} must be BIG_ENDIAN, LITTLE_ENDIAN, \
                             BIG_ENDIAN_BYTE_SWAP or LITTLE_ENDIAN_BYTE_SWAP"
                        ))
                    })?;
                    options.byte_order = byte;
                    options.word_order = word;
                }
                MAX_COILS => options.max_coils = limit(MAX_COILS, value, 2000)?,
                MAX_REGISTERS => options.max_registers = limit(MAX_REGISTERS, value, 125)?,
                CONVENTION if value == PLC4X => plc4x = true,
                CONVENTION => {
                    return Err(invalid(format!(
                        "{CONVENTION} accepts only {PLC4X} at alpha"
                    )));
                }
                ACCESS => {
                    access = match value.as_str() {
                        READ_WRITE => AccessPolicy::ReadWrite,
                        READ_ONLY => AccessPolicy::ReadOnly,
                        _ => {
                            return Err(invalid(format!(
                                "{ACCESS} must be {READ_ONLY} or {READ_WRITE}"
                            )));
                        }
                    };
                }
                other => return Err(unknown_key(other)),
            }
        }
        Ok((options, plc4x, access))
    }
}

/// A non-negative whole number in decimal digits only (no sign, no space, not empty) that
/// fits `T`.
fn number<T: FromStr>(key: &str, value: &str) -> Result<T> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid(format!(
            "{key} must be a whole number in decimal digits"
        )));
    }
    value
        .parse::<T>()
        .map_err(|_overflow| invalid(format!("{key} is out of range")))
}

/// A digits-only value too large even for `u64` counts as `u64::MAX`, so the caller's
/// bounds message applies; any other refusal stands.
fn digits_overflow(value: &str, refusal: PlcError) -> Result<u64> {
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(u64::MAX)
    } else {
        Err(refusal)
    }
}

/// A request-size limit: a whole number from 1 to `max`.
fn limit(key: &str, value: &str, max: u16) -> Result<NonZeroU16> {
    let parsed: u64 = number(key, value).or_else(|refusal| digits_overflow(value, refusal))?;
    u16::try_from(parsed)
        .ok()
        .and_then(NonZeroU16::new)
        .filter(|limit| limit.get() <= max)
        .ok_or_else(|| invalid(format!("{key} must be a whole number from 1 to {max}")))
}

/// The ping address, parsed under the `plc4x` convention whatever the connection's own.
/// The reason is not repeated: the address grammar is in the crate documentation.
fn ping_address(value: &str) -> Result<ModbusAddress> {
    let plc4x = AddressOptions::builder().convention(PLC4X).build()?;
    ModbusAddress::parse(value, &plc4x).map_err(|_refused| {
        invalid(format!(
            "{PING_ADDRESS} is not a Modbus address under the {PLC4X} convention"
        ))
    })
}

/// The refusal of a key the driver does not know; the key is shortened, nothing else of
/// the connection string is repeated.
fn unknown_key(key: &str) -> PlcError {
    let mut chars = key.chars();
    let mut shown: String = chars.by_ref().take(KEY_ECHO_LIMIT).collect();
    if chars.next().is_some() {
        shown.push_str("...");
    }
    let hint = if key == "unit-identifier" {
        " (the upstream name is default-unit-identifier)"
    } else {
        ""
    };
    invalid(format!(
        "unknown connection parameter '{shown}'{hint}; the parameters are {REQUEST_TIMEOUT}, \
         {UNIT_ID}, {PING_ADDRESS}, {BYTE_ORDER}, {MAX_COILS}, {MAX_REGISTERS}, {CONVENTION} \
         and {ACCESS}"
    ))
}

fn invalid(message: String) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> Result<(ModbusOptions, bool, AccessPolicy)> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        ModbusOptions::from_parameters(ModbusOptions::default(), &owned)
    }

    fn options(pairs: &[(&str, &str)]) -> ModbusOptions {
        parse(pairs).expect("valid parameters").0
    }

    fn refusal(pairs: &[(&str, &str)]) -> PlcError {
        parse(pairs).expect_err("the parameters must be refused")
    }

    #[test]
    fn overflowing_limits_and_unit_ids_name_their_bounds() {
        for (key, value, bounds) in [
            (MAX_COILS, "65536", "1 to 2000"),
            (MAX_COILS, "99999999999999999999999", "1 to 2000"),
            (MAX_REGISTERS, "126", "1 to 125"),
            (UNIT_ID, "256", "0 to 255"),
            (UNIT_ID, "99999999999999999999999", "0 to 255"),
        ] {
            let message = parse(&[(key, value)]).expect_err("refused").to_string();
            assert!(message.contains(bounds), "{key}={value}: {message}");
        }
    }

    #[test]
    fn the_default_is_the_upstream_default() {
        let defaults = ModbusOptions::default();
        assert_eq!(defaults.request_timeout, Duration::from_millis(5000));
        assert_eq!(defaults.unit_id, UnitId::from(1));
        assert_eq!(
            (defaults.byte_order, defaults.word_order),
            (ByteOrder::BigEndian, WordOrder::HighWordFirst)
        );
        assert_eq!(defaults.max_coils.get(), 2000);
        assert_eq!(defaults.max_registers.get(), 125);
        assert_eq!(
            parse(&[]).expect("none"),
            (defaults, false, AccessPolicy::ReadOnly)
        );
    }

    #[test]
    fn the_default_ping_address_is_what_the_grammar_reads_from_the_upstream_default_text() {
        let plc4x = AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("options");
        assert_eq!(
            ModbusAddress::parse("4x00001:BOOL", &plc4x).expect("the upstream default"),
            ModbusAddress::DEFAULT_PING
        );
    }

    #[test]
    fn connection_defaults_carry_the_connection_part_of_the_options() {
        let changed = ModbusOptions {
            unit_id: UnitId::from(200),
            byte_order: ByteOrder::LittleEndian,
            word_order: WordOrder::LowWordFirst,
            max_coils: NonZeroU16::new(64).expect("non-zero"),
            max_registers: NonZeroU16::new(10).expect("non-zero"),
            ..ModbusOptions::default()
        };
        let defaults = changed.connection_defaults();
        assert_eq!(defaults.unit_id, UnitId::from(200));
        assert_eq!(
            (defaults.byte_order, defaults.word_order),
            (ByteOrder::LittleEndian, WordOrder::LowWordFirst)
        );
        assert_eq!(
            (defaults.max_coils.get(), defaults.max_registers.get()),
            (64, 10)
        );
    }

    #[test]
    fn each_parameter_is_read_independently() {
        assert_eq!(
            options(&[(REQUEST_TIMEOUT, "1")]).request_timeout,
            Duration::from_millis(1)
        );
        assert_eq!(
            options(&[(REQUEST_TIMEOUT, "0000600")]).request_timeout,
            Duration::from_millis(600),
            "leading zeros are digits"
        );
        assert_eq!(options(&[(UNIT_ID, "0")]).unit_id, UnitId::from(0));
        assert_eq!(options(&[(UNIT_ID, "255")]).unit_id, UnitId::from(255));
        assert_eq!(options(&[(MAX_COILS, "1")]).max_coils.get(), 1);
        assert_eq!(options(&[(MAX_COILS, "2000")]).max_coils.get(), 2000);
        assert_eq!(options(&[(MAX_REGISTERS, "1")]).max_registers.get(), 1);
        assert_eq!(options(&[(MAX_REGISTERS, "125")]).max_registers.get(), 125);
        assert_eq!(
            options(&[(PING_ADDRESS, "holding-register:7:UINT")]).ping_address,
            options(&[(PING_ADDRESS, "40007:UINT")]).ping_address,
            "the ping address is read under the plc4x convention, bare forms included"
        );
        let (_, plc4x, _) = parse(&[(CONVENTION, "plc4x")]).expect("convention");
        assert!(plc4x, "address-convention=plc4x is reported");
        let (_, none, _) = parse(&[(UNIT_ID, "3")]).expect("no convention");
        assert!(!none, "no convention without the key");
    }

    #[test]
    fn access_grants_writes_only_when_it_says_read_write() {
        let access = |pairs: &[(&str, &str)]| parse(pairs).expect("valid parameters").2;
        assert_eq!(access(&[]), AccessPolicy::ReadOnly, "the default");
        assert_eq!(access(&[(ACCESS, "read-only")]), AccessPolicy::ReadOnly);
        assert_eq!(access(&[(ACCESS, "read-write")]), AccessPolicy::ReadWrite);
        for value in ["", "write", "READ-WRITE", "rw", "read-write ", "true"] {
            let error = refusal(&[(ACCESS, value)]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{value:?}");
            assert!(error.to_string().contains(ACCESS), "names the key: {error}");
        }
    }

    #[test]
    fn the_four_byte_order_names_map_per_adr_0006() {
        for (name, expected) in [
            (
                "BIG_ENDIAN",
                (ByteOrder::BigEndian, WordOrder::HighWordFirst),
            ),
            (
                "LITTLE_ENDIAN",
                (ByteOrder::LittleEndian, WordOrder::LowWordFirst),
            ),
            (
                "BIG_ENDIAN_BYTE_SWAP",
                (ByteOrder::LittleEndian, WordOrder::HighWordFirst),
            ),
            (
                "LITTLE_ENDIAN_BYTE_SWAP",
                (ByteOrder::BigEndian, WordOrder::LowWordFirst),
            ),
        ] {
            let read = options(&[(BYTE_ORDER, name)]);
            assert_eq!((read.byte_order, read.word_order), expected, "{name}");
        }
    }

    #[test]
    fn the_numeric_parameters_refuse_every_edge_of_their_range() {
        for (key, bad) in [
            (
                REQUEST_TIMEOUT,
                ["0", "-1", "+5", "", "1.5", "abc", " 5", "5 ", "1e3"],
            ),
            (
                UNIT_ID,
                [
                    "256",
                    "-1",
                    "+1",
                    "",
                    "0x10",
                    "abc",
                    " 1",
                    "1 ",
                    "99999999999",
                ],
            ),
            (
                MAX_COILS,
                ["0", "2001", "-1", "", "1.5", "abc", "+5", " 5", "70000"],
            ),
            (
                MAX_REGISTERS,
                ["0", "126", "-1", "", "1.5", "abc", "+5", " 5", "70000"],
            ),
        ] {
            for value in bad {
                let error = refusal(&[(key, value)]);
                assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}={value:?}");
                assert!(error.to_string().contains(key), "names the key: {error}");
            }
        }
        let overflow = refusal(&[(REQUEST_TIMEOUT, "99999999999999999999999")]);
        assert_eq!(overflow.kind(), ErrorKind::InvalidConfig);
    }

    #[test]
    fn a_bad_ping_address_or_byte_order_or_convention_is_refused() {
        for (key, value) in [
            (PING_ADDRESS, ""),
            (PING_ADDRESS, "garbage"),
            (PING_ADDRESS, "holding-register:0"),
            (PING_ADDRESS, "holding-register:1:STRING"),
            (BYTE_ORDER, ""),
            (BYTE_ORDER, "big_endian"),
            (BYTE_ORDER, "MIDDLE_ENDIAN"),
            (BYTE_ORDER, "'BIG_ENDIAN'"),
            (CONVENTION, ""),
            (CONVENTION, "PLC4X"),
            (CONVENTION, "zero-based"),
        ] {
            let error = refusal(&[(key, value)]);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{key}={value:?}");
            assert!(error.to_string().contains(key), "names the key: {error}");
        }
    }

    #[test]
    fn an_unknown_key_is_refused_and_a_long_one_is_shortened() {
        let error = refusal(&[("surprise", "1")]);
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("'surprise'"), "{error}");
        assert!(
            !error.to_string().contains("default-unit-identifier)"),
            "{error}"
        );

        let misspelt = refusal(&[("unit-identifier", "1")]);
        assert!(
            misspelt.to_string().contains("default-unit-identifier"),
            "the hint names the real key: {misspelt}"
        );

        let long = "k".repeat(200);
        let shortened = refusal(&[(long.as_str(), "1")]).to_string();
        assert!(shortened.contains("..."), "{shortened}");
        assert!(!shortened.contains(&long), "the key is shortened");
        let exact = "k".repeat(KEY_ECHO_LIMIT);
        let whole = refusal(&[(exact.as_str(), "1")]).to_string();
        assert!(whole.contains(&exact) && !whole.contains("..."), "{whole}");
    }
}
