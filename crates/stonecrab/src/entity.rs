//! Object PLC Mapping: a type whose fields map to tags (ADR-0033).
//!
//! [`PlcEntity`] is the mapping: each field's name and address text, and the conversion of
//! each field from and to a [`PlcValue`]. [`FieldValue`] converts one field.
//! [`plc_entity!`](crate::plc_entity) writes the struct and its [`PlcEntity`] implementation
//! from a declaration, and a type can implement the trait by hand. The
//! [`EntityManager`](crate::EntityManager) reads and writes entities through a
//! [`ConnectionCache`](crate::ConnectionCache).

use std::time::Duration;

use crate::{
    ErrorKind, PlcDate, PlcDateTime, PlcError, PlcSignedDuration, PlcTimeOfDay, PlcValue, Result,
};

/// One mapped field of a [`PlcEntity`]: its name, its address text and how long a read of
/// it stays fresh.
///
/// The name is the field's name and becomes the [`TagId`](crate::TagId) of its item in
/// every request. The address is the text the connection's driver parses
/// ([`Driver::parse_address`](crate::Driver::parse_address)), or an alias of the form
/// `${name}` that the [`EntityManager`](crate::EntityManager) resolves. Both are fixed at
/// compile time, as upstream's annotations are.
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-ENTITY
/// Upstream's `@PlcTag(value, cacheDurationMillis)` on a field. A field without a cache
/// duration is upstream's default of -1: it is read again at every refresh.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::EntityField;
///
/// const LEVEL: EntityField =
///     EntityField::new("level", "STATE/level:u16").cache_for(Duration::from_millis(500));
/// assert_eq!(LEVEL.name(), "level");
/// assert_eq!(LEVEL.address(), "STATE/level:u16");
/// assert_eq!(LEVEL.cache_duration(), Some(Duration::from_millis(500)));
/// assert_eq!(EntityField::new("level", "${level}").cache_duration(), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityField {
    name: &'static str,
    address: &'static str,
    cache_duration: Option<Duration>,
}

impl EntityField {
    /// The field `name` mapped to `address`, read again at every refresh.
    ///
    /// # Examples
    /// See [`EntityField`].
    #[must_use]
    pub const fn new(name: &'static str, address: &'static str) -> Self {
        Self {
            name,
            address,
            cache_duration: None,
        }
    }

    /// The same field, whose value stays fresh for `duration` after it was read: a
    /// [`ConnectedEntity::refresh`](crate::ConnectedEntity::refresh) reads it again only
    /// once more than `duration` has passed on the manager's clock.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-CACHE
    /// Upstream's `cacheDurationMillis` with a value of zero or more.
    ///
    /// # Examples
    /// See [`EntityField`].
    #[must_use]
    pub const fn cache_for(mut self, duration: Duration) -> Self {
        self.cache_duration = Some(duration);
        self
    }

    /// The field's name, unique within its entity.
    ///
    /// # Examples
    /// See [`EntityField`].
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The field's address text or alias.
    ///
    /// # Examples
    /// See [`EntityField`].
    #[must_use]
    pub const fn address(&self) -> &'static str {
        self.address
    }

    /// How long a read of the field stays fresh; `None` when it is read at every refresh.
    ///
    /// # Examples
    /// See [`EntityField`].
    #[must_use]
    pub const fn cache_duration(&self) -> Option<Duration> {
        self.cache_duration
    }
}

/// A type whose fields map to tags: what the [`EntityManager`](crate::EntityManager) reads
/// and writes.
///
/// [`fields`](Self::fields) lists the mapped fields in a fixed order;
/// [`field_value`](Self::field_value) and [`set_field_value`](Self::set_field_value) convert
/// one of them, by name, to and from a [`PlcValue`]. A type may have fields that are not
/// mapped; the manager never touches them. [`plc_entity!`](crate::plc_entity) writes this
/// implementation for a plain struct, converting each field with [`FieldValue`]; the
/// example below writes it by hand.
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-ENTITY
/// Upstream's `@PlcEntity` type, whose `@PlcTag` fields are found by reflection; here the
/// table and the conversions are code. The default constructor upstream requires is
/// `Default`, required by the manager's [`read`](crate::EntityManager::read) and
/// [`connect`](crate::EntityManager::connect).
///
/// # Examples
/// ```
/// use stonecrab::{EntityField, ErrorKind, FieldValue, PlcEntity, PlcError, PlcValue, Result};
///
/// #[derive(Debug, Default)]
/// struct Pump {
///     running: bool,
///     /// Not mapped: the manager leaves it alone.
///     label: String,
/// }
///
/// impl PlcEntity for Pump {
///     fn fields(&self) -> &'static [EntityField] {
///         const FIELDS: &[EntityField] = &[EntityField::new("running", "STATE/running:bool")];
///         FIELDS
///     }
///
///     fn field_value(&self, field: &str) -> Option<PlcValue> {
///         (field == "running").then(|| self.running.to_plc_value())
///     }
///
///     fn set_field_value(&mut self, field: &str, value: PlcValue) -> Result<()> {
///         match field {
///             "running" => self.running = bool::from_plc_value(value)?,
///             _ => return Err(PlcError::new(ErrorKind::InvalidConfig, "no such field")),
///         }
///         Ok(())
///     }
/// }
///
/// let mut pump = Pump::default();
/// pump.set_field_value("running", PlcValue::Bool(true))?;
/// assert_eq!(pump.field_value("running"), Some(PlcValue::Bool(true)));
/// assert_eq!(pump.field_value("label"), None);
/// let refused = pump.set_field_value("running", PlcValue::U8(1)).map_err(|error| error.kind());
/// assert_eq!(refused, Err(ErrorKind::InvalidConfig));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
pub trait PlcEntity {
    /// The mapped fields, in the order the manager puts them into a request. The names are
    /// unique.
    fn fields(&self) -> &'static [EntityField];

    /// The value of the mapped field named `field`, or `None` when no mapped field has that
    /// name.
    fn field_value(&self, field: &str) -> Option<PlcValue>;

    /// Store `value` in the mapped field named `field`.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] when the field's type cannot hold `value` (see
    /// [`FieldValue`]) or no mapped field has that name. A refused value leaves the field
    /// unchanged.
    fn set_field_value(&mut self, field: &str, value: PlcValue) -> Result<()>;
}

/// The conversion of one field of a [`PlcEntity`] from and to a [`PlcValue`].
///
/// | Field type | Read from | Written as |
/// | --- | --- | --- |
/// | `bool` | `Bool` | `Bool` |
/// | `i8`, `i16`, `i32`, `i64` | any integer whose value fits | `I8` to `I64` |
/// | `u8`, `u16`, `u32`, `u64` | any integer whose value fits | `U8` to `U64` |
/// | `f32` | `F32` | `F32` |
/// | `f64` | `F32` or `F64` | `F64` |
/// | `char`, `String` | `Char`, `String` | `Char`, `String` |
/// | `Duration`, [`PlcSignedDuration`] | `Duration`, `SignedDuration` | the same |
/// | [`PlcDate`], [`PlcTimeOfDay`], [`PlcDateTime`] | `Date`, `TimeOfDay`, `DateTime` | the same |
/// | `Vec<T>` of any type above | `List` whose every element converts | `List` |
/// | [`PlcValue`] | any value | the value |
///
/// Reading is lenient where nothing is lost: an `i32` field takes a `U16` value, and a
/// `u8` field takes `U16(200)` but not `U16(840)`. An integer is never read into a float
/// field or a float into an integer one, as [`PlcValue::as_f64`] does not. Writing sends
/// the field type's own variant, so a field that is written must have the type of its
/// tag: drivers refuse a value of another type (`InvalidConfig`).
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-ENTITY
/// Upstream's `PlcEntityInterceptor.getTyped`, which picks the response getter by the
/// field's Java type (`getShort`, `getAllIntegers`, ...); `BigInteger` and `BigDecimal`
/// have no Stonecrab value and no row here.
///
/// # Examples
/// ```
/// use stonecrab::{ErrorKind, FieldValue, PlcValue};
///
/// assert_eq!(u8::from_plc_value(PlcValue::U16(200))?, 200);
/// let too_big = u8::from_plc_value(PlcValue::U16(840)).map_err(|error| error.kind());
/// assert_eq!(too_big, Err(ErrorKind::InvalidConfig));
/// assert_eq!(f64::from_plc_value(PlcValue::F32(1.5))?, 1.5);
/// assert_eq!(vec![1_i16, -2].to_plc_value(), PlcValue::List(vec![PlcValue::I16(1), PlcValue::I16(-2)]));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
pub trait FieldValue {
    /// The field's value read from `value`.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] when the type cannot hold `value`.
    fn from_plc_value(value: PlcValue) -> Result<Self>
    where
        Self: Sized;

    /// The value written for the field.
    fn to_plc_value(&self) -> PlcValue;
}

/// The error of a field of type `field_type` given a value it cannot hold.
fn cannot_hold(field_type: &str, value: &PlcValue) -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        format!(
            "a {field_type} field cannot hold this {:?} value",
            value.value_type()
        ),
    )
}

/// `FieldValue` for integer types: read from any integer whose value fits, through the
/// lossless widening named.
macro_rules! integer_fields {
    ($($ty:ty => $variant:ident by $widen:ident;)+) => {$(
        impl FieldValue for $ty {
            fn from_plc_value(value: PlcValue) -> Result<Self> {
                value
                    .$widen()
                    .and_then(|wide| Self::try_from(wide).ok())
                    .ok_or_else(|| cannot_hold(stringify!($ty), &value))
            }

            fn to_plc_value(&self) -> PlcValue {
                PlcValue::$variant(*self)
            }
        }
    )+};
}

integer_fields! {
    i8 => I8 by as_i64;
    i16 => I16 by as_i64;
    i32 => I32 by as_i64;
    i64 => I64 by as_i64;
    u8 => U8 by as_u64;
    u16 => U16 by as_u64;
    u32 => U32 by as_u64;
    u64 => U64 by as_u64;
}

/// `FieldValue` for `Copy` types read from their own variant only.
macro_rules! exact_fields {
    ($($ty:ty => $variant:ident;)+) => {$(
        impl FieldValue for $ty {
            fn from_plc_value(value: PlcValue) -> Result<Self> {
                if let PlcValue::$variant(inner) = value {
                    Ok(inner)
                } else {
                    Err(cannot_hold(stringify!($ty), &value))
                }
            }

            fn to_plc_value(&self) -> PlcValue {
                PlcValue::$variant(*self)
            }
        }
    )+};
}

exact_fields! {
    bool => Bool;
    f32 => F32;
    char => Char;
    Duration => Duration;
    PlcSignedDuration => SignedDuration;
    PlcDate => Date;
    PlcTimeOfDay => TimeOfDay;
    PlcDateTime => DateTime;
}

impl FieldValue for f64 {
    fn from_plc_value(value: PlcValue) -> Result<Self> {
        value.as_f64().ok_or_else(|| cannot_hold("f64", &value))
    }

    fn to_plc_value(&self) -> PlcValue {
        PlcValue::F64(*self)
    }
}

impl FieldValue for String {
    fn from_plc_value(value: PlcValue) -> Result<Self> {
        if let PlcValue::String(text) = value {
            Ok(text)
        } else {
            Err(cannot_hold("String", &value))
        }
    }

    fn to_plc_value(&self) -> PlcValue {
        PlcValue::String(self.clone())
    }
}

impl FieldValue for PlcValue {
    fn from_plc_value(value: PlcValue) -> Result<Self> {
        Ok(value)
    }

    fn to_plc_value(&self) -> PlcValue {
        self.clone()
    }
}

impl<T: FieldValue> FieldValue for Vec<T> {
    fn from_plc_value(value: PlcValue) -> Result<Self> {
        if let PlcValue::List(elements) = value {
            elements
                .into_iter()
                .enumerate()
                .map(|(index, element)| {
                    T::from_plc_value(element).map_err(|error| {
                        PlcError::new(error.kind(), format!("element {index}: {error}"))
                    })
                })
                .collect()
        } else {
            Err(cannot_hold("Vec", &value))
        }
    }

    fn to_plc_value(&self) -> PlcValue {
        PlcValue::List(self.iter().map(FieldValue::to_plc_value).collect())
    }
}

/// Declare a struct and implement [`PlcEntity`] for it: each field followed by
/// `=> "address"` is mapped to that address.
///
/// ```text
/// plc_entity! {
///     <attributes> <visibility> struct <Name> {
///         <attributes> <visibility> <field>: <type> [=> "<address>" [(cache = <duration>)]],
///         ...
///     }
/// }
/// ```
///
/// The struct is emitted as written, without the mappings. A mapped field's type must
/// implement [`FieldValue`]; the address is a string literal, the text the driver parses or
/// an alias `${name}`; the optional cache duration is a constant `Duration` expression
/// ([`EntityField::cache_for`]). A field without `=>` is not mapped. The mapped fields keep
/// their declaration order in [`PlcEntity::fields`]. A mapped field's name, and so the
/// [`TagId`](crate::TagId) of its item, is its identifier as written: a raw identifier keeps
/// its prefix, so the field `r#type` is named `"r#type"`. The manager's
/// [`read`](crate::EntityManager::read) and [`connect`](crate::EntityManager::connect) need
/// `Default` as well, which the struct derives or implements as any struct would.
///
/// # What does not compile
/// A mapped field whose type has no [`FieldValue`] conversion:
/// ```compile_fail,E0277
/// stonecrab::plc_entity! {
///     struct Station {
///         address: std::net::Ipv4Addr => "STATE/address:u32",
///     }
/// }
/// ```
/// An address that is not a string literal:
/// ```compile_fail,E0308
/// stonecrab::plc_entity! {
///     struct Station {
///         level: u16 => 40001,
///     }
/// }
/// ```
/// A cache duration that is not a constant:
/// ```compile_fail,E0015
/// stonecrab::plc_entity! {
///     struct Station {
///         level: u16 => "STATE/level:u16" (cache = std::time::Duration::from_secs_f64(0.5)),
///     }
/// }
/// ```
/// A generic or tuple struct, an enum, or a struct with a `where` clause: the grammar
/// above has no place for them.
/// ```compile_fail
/// stonecrab::plc_entity! {
///     struct Station<T> {
///         level: T => "STATE/level:u16",
///     }
/// }
/// ```
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-ENTITY
/// Upstream's `@PlcEntity` and `@PlcTag(value, cacheDurationMillis)` annotations; a field
/// without `@PlcTag` is a field without `=>`.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::{PlcEntity, PlcValue};
///
/// stonecrab::plc_entity! {
///     /// A tank on the bench.
///     #[derive(Debug, Default, PartialEq)]
///     pub struct Tank {
///         /// The level in millimetres.
///         pub level: u16 => "STATE/level:u16",
///         pub setpoint: i16 => "${setpoint}" (cache = Duration::from_secs(5)),
///         pub label: String,
///     }
/// }
///
/// let mut tank = Tank::default();
/// let names: Vec<&str> = tank.fields().iter().map(|field| field.name()).collect();
/// assert_eq!(names, ["level", "setpoint"]);
/// assert_eq!(tank.fields()[1].cache_duration(), Some(Duration::from_secs(5)));
/// tank.set_field_value("level", PlcValue::U16(840))?;
/// assert_eq!(tank.level, 840);
/// assert_eq!(tank.field_value("setpoint"), Some(PlcValue::I16(0)));
/// assert_eq!(tank.field_value("label"), None);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[macro_export]
macro_rules! plc_entity {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field:ident : $ty:ty
                $( => $address:literal $( (cache = $cache:expr) )? )?
            ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name {
            $( $(#[$field_meta])* $field_vis $field: $ty, )*
        }

        impl $crate::PlcEntity for $name {
            fn fields(&self) -> &'static [$crate::EntityField] {
                const FIELDS: &[$crate::EntityField] = &[
                    $($(
                        $crate::EntityField::new(::core::stringify!($field), $address)
                            $( .cache_for($cache) )?,
                    )?)*
                ];
                FIELDS
            }

            // In the two methods below, `let _ = $address;` only selects the mapped fields: a
            // repetition expands once per match of a variable of its own depth.
            fn field_value(&self, field: &str) -> ::core::option::Option<$crate::PlcValue> {
                match field {
                    $($(
                        ::core::stringify!($field) => {
                            let _ = $address;
                            ::core::option::Option::Some(
                                $crate::FieldValue::to_plc_value(&self.$field),
                            )
                        }
                    )?)*
                    _ => ::core::option::Option::None,
                }
            }

            fn set_field_value(
                &mut self,
                field: &str,
                value: $crate::PlcValue,
            ) -> $crate::Result<()> {
                match field {
                    $($(
                        ::core::stringify!($field) => {
                            let _ = $address;
                            self.$field = $crate::FieldValue::from_plc_value(value)?;
                            ::core::result::Result::Ok(())
                        }
                    )?)*
                    _ => ::core::result::Result::Err($crate::PlcError::new(
                        $crate::ErrorKind::InvalidConfig,
                        ::std::format!("the entity has no mapped field named {field:?}"),
                    )),
                }
            }
        }
    };
}
