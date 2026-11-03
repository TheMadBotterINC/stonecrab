//! Pure value selection and bounded row-major Variant encoding.

use crate::opcua::attributes::Attributes;
use crate::opcua::codec::{Array, DateTime, Scalar, UaByteString, UaString, Variant, VariantValue};
use crate::opcua::{OpcuaAddress, OpcuaDataType};
use crate::{ErrorKind, Operation, PlcError, PlcStruct, PlcValue, Result};

#[derive(Clone, Copy)]
pub(in crate::opcua) struct Budget {
    nodes: usize,
    bytes: usize,
}
impl Budget {
    pub(in crate::opcua) const fn new(bytes: usize) -> Self {
        Self {
            nodes: crate::opcua::value::MAX_VALUE_NODES,
            bytes,
        }
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.nodes = self.nodes.checked_sub(1).ok_or_else(resource)?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(resource)?;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Encoding {
    BuiltIn(u32),
    Milliseconds,
}

pub(in crate::opcua) fn encode(
    value: &PlcValue,
    address: &OpcuaAddress,
    attributes: &Attributes,
    budget: &mut Budget,
) -> Result<Variant> {
    let mut remaining = *budget;
    let (leaves, dimensions) = flatten(value, &mut remaining)?;
    check_shape(&dimensions, address, attributes)?;
    if attributes.access.is_some_and(|access| access & 2 == 0) {
        return Err(PlcError::new(
            ErrorKind::AccessDenied,
            "OPC UA variable is not writable",
        ));
    }
    let encoding = select(address.data_type(), attributes.data_type, &leaves)?;
    let value = if matches!(value, PlcValue::Null) {
        VariantValue::Null
    } else if dimensions.is_empty() {
        VariantValue::Scalar(scalar(value, encoding)?)
    } else {
        VariantValue::Array(array(&leaves, encoding)?)
    };
    let dimensions = if dimensions.len() > 1 {
        Some(
            dimensions
                .into_iter()
                .map(|size| i32::try_from(size).map_err(|_value| resource()))
                .collect::<Result<Vec<_>>>()?,
        )
    } else {
        None
    };
    *budget = remaining;
    Ok(Variant { value, dimensions })
}

fn flatten<'a>(
    value: &'a PlcValue,
    budget: &mut Budget,
) -> Result<(Vec<&'a PlcValue>, Vec<usize>)> {
    let mut pending = vec![(value, 0_usize)];
    let mut dimensions = Vec::new();
    let mut leaves = Vec::new();
    let mut rank = None;
    while let Some((value, depth)) = pending.pop() {
        let bytes = if let PlcValue::String(text) = value {
            text.len()
        } else if let PlcValue::Bytes(bytes) = value {
            bytes.len()
        } else {
            1
        };
        budget.charge(bytes)?;
        if let PlcValue::List(values) = value {
            if depth >= PlcStruct::MAX_DEPTH {
                return Err(resource());
            }
            if rank.is_some_and(|rank| depth >= rank) {
                return Err(invalid());
            }
            if let Some(size) = dimensions.get(depth) {
                if *size != values.len() {
                    return Err(invalid());
                }
            } else {
                dimensions.push(values.len());
            }
            if values
                .len()
                .checked_add(pending.len())
                .is_none_or(|nodes| nodes > budget.nodes)
            {
                return Err(resource());
            }
            for value in values.iter().rev() {
                pending.push((value, depth.saturating_add(1)));
            }
        } else {
            if rank.is_some_and(|rank| rank != depth) {
                return Err(invalid());
            }
            rank = Some(depth);
            leaves.push(value);
        }
    }
    if rank.is_some_and(|rank| rank != dimensions.len()) {
        return Err(invalid());
    }
    Ok((leaves, dimensions))
}

fn check_shape(
    dimensions: &[usize],
    address: &OpcuaAddress,
    attributes: &Attributes,
) -> Result<()> {
    if let Some(range) = address.index_range() {
        let wanted = range
            .split(',')
            .map(|dimension| {
                let (low, high) = dimension.split_once(':').unwrap_or((dimension, dimension));
                high.parse::<usize>()
                    .ok()
                    .and_then(|high| {
                        low.parse::<usize>()
                            .ok()
                            .and_then(|low| high.checked_sub(low))
                    })
                    .and_then(|width| width.checked_add(1))
                    .ok_or_else(invalid)
            })
            .collect::<Result<Vec<_>>>()?;
        if dimensions != wanted {
            return Err(invalid());
        }
        if attributes
            .rank
            .is_some_and(|rank| rank > 0 && usize::try_from(rank).ok() != Some(dimensions.len()))
        {
            return Err(invalid());
        }
        return Ok(());
    }
    let rank = dimensions.len();
    let fits = match attributes.rank {
        None | Some(-2) => true,
        Some(-3) => rank <= 1,
        Some(-1) => rank == 0,
        Some(0) => rank > 0,
        Some(expected) => usize::try_from(expected).ok() == Some(rank),
    };
    if !fits {
        return Err(invalid());
    }
    if !attributes.dimensions.is_empty()
        && (attributes.dimensions.len() != rank
            || attributes
                .dimensions
                .iter()
                .zip(dimensions)
                .any(|(&max, &size)| {
                    max != 0 && u32::try_from(size).map_or(true, |size| size > max)
                }))
    {
        return Err(invalid());
    }
    Ok(())
}

fn select(suffix: OpcuaDataType, declared: Option<u32>, leaves: &[&PlcValue]) -> Result<Encoding> {
    let kind = match suffix {
        OpcuaDataType::Bool => 1,
        OpcuaDataType::SInt => 2,
        OpcuaDataType::Byte | OpcuaDataType::USInt => 3,
        OpcuaDataType::Int => 4,
        OpcuaDataType::UInt => 5,
        OpcuaDataType::DInt => 6,
        OpcuaDataType::UDInt => 7,
        OpcuaDataType::LInt => 8,
        OpcuaDataType::ULInt => 9,
        OpcuaDataType::Real => 10,
        OpcuaDataType::LReal => 11,
        OpcuaDataType::Char | OpcuaDataType::WChar | OpcuaDataType::String => 12,
        OpcuaDataType::DateAndTime | OpcuaDataType::LDateAndTime => 13,
        OpcuaDataType::Time => return Ok(Encoding::Milliseconds),
        OpcuaDataType::LTime
        | OpcuaDataType::Date
        | OpcuaDataType::LDate
        | OpcuaDataType::TimeOfDay
        | OpcuaDataType::LTimeOfDay => return Err(unsupported()),
        OpcuaDataType::Null => match declared {
            Some(28) => {
                let maximum = leaves
                    .iter()
                    .map(|value| value.as_u64().ok_or_else(invalid))
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .max()
                    .ok_or_else(invalid)?;
                if u8::try_from(maximum).is_ok() {
                    3
                } else if u16::try_from(maximum).is_ok() {
                    5
                } else if u32::try_from(maximum).is_ok() {
                    7
                } else {
                    9
                }
            }
            Some(27) => 8,
            Some(26) => {
                let kind = infer(leaves.first().copied().ok_or_else(invalid)?)?;
                if !matches!(kind, Encoding::BuiltIn(2..=11)) {
                    return Err(invalid());
                }
                return Ok(kind);
            }
            Some(0 | 24) | None => return infer(leaves.first().copied().ok_or_else(invalid)?),
            Some(kind @ (1..=13 | 15)) => kind,
            Some(_) => return Err(unsupported()),
        },
    };
    Ok(Encoding::BuiltIn(kind))
}

fn infer(value: &PlcValue) -> Result<Encoding> {
    let kind = match value {
        PlcValue::Null => 0,
        PlcValue::Bool(_) => 1,
        PlcValue::I8(_) => 2,
        PlcValue::U8(_) => 3,
        PlcValue::I16(_) => 4,
        PlcValue::U16(_) => 5,
        PlcValue::I32(_) => 6,
        PlcValue::U32(_) => 7,
        PlcValue::I64(_) => 8,
        PlcValue::U64(_) => 9,
        PlcValue::F32(_) => 10,
        PlcValue::F64(_) => 11,
        PlcValue::Char(_) | PlcValue::String(_) => 12,
        PlcValue::DateTime(_) => 13,
        PlcValue::Bytes(_) => 15,
        PlcValue::Duration(_) | PlcValue::SignedDuration(_) => return Ok(Encoding::Milliseconds),
        PlcValue::List(_) | PlcValue::Struct(_) | PlcValue::Date(_) | PlcValue::TimeOfDay(_) => {
            return Err(unsupported());
        }
    };
    Ok(Encoding::BuiltIn(kind))
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "a requested wire type accepts only explicitly convertible PlcValue forms; every other form is invalid"
)]
fn scalar(value: &PlcValue, encoding: Encoding) -> Result<Scalar> {
    let Encoding::BuiltIn(kind) = encoding else {
        let nanos = match value {
            PlcValue::SignedDuration(duration) => i128::from(duration.as_nanos()),
            PlcValue::Duration(duration) => {
                i128::try_from(duration.as_nanos()).map_err(|_value| invalid())?
            }
            _ => return Err(invalid()),
        };
        if nanos.rem_euclid(1_000_000) != 0 {
            return Err(invalid());
        }
        return Ok(Scalar::Int32(
            i32::try_from(nanos.div_euclid(1_000_000)).map_err(|_value| invalid())?,
        ));
    };
    macro_rules! signed {
        ($variant:ident, $ty:ty) => {
            Scalar::$variant(
                <$ty>::try_from(value.as_i64().ok_or_else(invalid)?).map_err(|_value| invalid())?,
            )
        };
    }
    macro_rules! unsigned {
        ($variant:ident, $ty:ty) => {
            Scalar::$variant(
                <$ty>::try_from(value.as_u64().ok_or_else(invalid)?).map_err(|_value| invalid())?,
            )
        };
    }
    Ok(match kind {
        1 => match value {
            PlcValue::Bool(value) => Scalar::Boolean(*value),
            _ => return Err(invalid()),
        },
        2 => signed!(SByte, i8),
        3 => unsigned!(Byte, u8),
        4 => signed!(Int16, i16),
        5 => unsigned!(UInt16, u16),
        6 => signed!(Int32, i32),
        7 => unsigned!(UInt32, u32),
        8 => Scalar::Int64(value.as_i64().ok_or_else(invalid)?),
        9 => Scalar::UInt64(value.as_u64().ok_or_else(invalid)?),
        10 => match value {
            PlcValue::F32(value) => Scalar::Float(*value),
            PlcValue::F64(value) => Scalar::Float(narrow(*value)?),
            _ => return Err(invalid()),
        },
        11 => match value {
            PlcValue::F32(value) => Scalar::Double(f64::from(*value)),
            PlcValue::F64(value) => Scalar::Double(*value),
            _ => return Err(invalid()),
        },
        12 => Scalar::String(match value {
            PlcValue::Char(value) => UaString::new(value.to_string()),
            PlcValue::String(value) => UaString::new(value),
            _ => return Err(invalid()),
        }),
        13 => {
            let PlcValue::DateTime(value) = value else {
                return Err(invalid());
            };
            let nanos = i128::from(value.date().days_since_epoch())
                .checked_mul(86_400_000_000_000)
                .and_then(|days| days.checked_add(i128::from(value.time().nanos_since_midnight())))
                .ok_or_else(invalid)?;
            if nanos.rem_euclid(100) != 0 {
                return Err(invalid());
            }
            let ticks = nanos
                .div_euclid(100)
                .checked_add(116_444_736_000_000_000)
                .and_then(|ticks| i64::try_from(ticks).ok())
                .ok_or_else(invalid)?;
            Scalar::DateTime(DateTime(ticks))
        }
        15 => match value {
            PlcValue::Bytes(value) => Scalar::ByteString(UaByteString::new(value.clone())),
            _ => return Err(invalid()),
        },
        _ => return Err(unsupported()),
    })
}

/// Rust has no `TryFrom<f64>` for `f32`; decimal round-trip plus binary equality proves exact narrowing.
fn narrow(value: f64) -> Result<f32> {
    let narrowed = value
        .to_string()
        .parse::<f32>()
        .map_err(|_number| invalid())?;
    if value.is_nan() || f64::from(narrowed).to_bits() == value.to_bits() {
        Ok(narrowed)
    } else {
        Err(invalid())
    }
}

fn array(leaves: &[&PlcValue], encoding: Encoding) -> Result<Array> {
    let kind = match encoding {
        Encoding::BuiltIn(kind) => kind,
        Encoding::Milliseconds => 6,
    };
    macro_rules! elements {
        ($variant:ident) => {
            Array::$variant(Some(
                leaves
                    .iter()
                    .map(|value| match scalar(value, encoding)? {
                        Scalar::$variant(value) => Ok(value),
                        _ => Err(invalid()),
                    })
                    .collect::<Result<Vec<_>>>()?,
            ))
        };
    }
    Ok(match kind {
        1 => elements!(Boolean),
        2 => elements!(SByte),
        3 => elements!(Byte),
        4 => elements!(Int16),
        5 => elements!(UInt16),
        6 => elements!(Int32),
        7 => elements!(UInt32),
        8 => elements!(Int64),
        9 => elements!(UInt64),
        10 => elements!(Float),
        11 => elements!(Double),
        12 => elements!(String),
        13 => elements!(DateTime),
        15 => elements!(ByteString),
        _ => return Err(unsupported()),
    })
}

fn invalid() -> PlcError {
    PlcError::new(
        ErrorKind::InvalidDataType,
        "OPC UA write value does not exactly fit its type or shape",
    )
}
fn unsupported() -> PlcError {
    PlcError::new(
        ErrorKind::Unsupported(Operation::Write),
        "OPC UA write value type is not implemented",
    )
}
fn resource() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "OPC UA write value exceeds its node, depth or byte budget",
    )
}

#[cfg(test)]
mod tests;
