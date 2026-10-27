//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::modbus::{
        AddressConvention, BatchLimits, Batcher, Chunk, ConnectionDefaults, ExceptionCode,
        FunctionCode, MbapFramer, MbapHeader, Member, ModbusAddress, ModbusAddressBuilder,
        ModbusAdu, ModbusArea, ModbusConnection, ModbusDataType, ModbusOptions, ModbusPdu,
        ModbusSession, ModbusTcpDriver, SessionStats, SystemClock, TransactionId, TypedDecoder,
        UnitId,
    };

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<UnitId>();
        assert_send_sync::<ModbusArea>();
        assert_send_sync::<ModbusDataType>();
        assert_send_sync::<AddressConvention>();
        assert_send_sync::<ModbusAddress>();
        assert_send_sync::<ModbusAddressBuilder>();
        assert_send_sync::<BatchLimits>();
        assert_send_sync::<Batcher>();
        assert_send_sync::<Chunk>();
        assert_send_sync::<Member>();
        assert_send_sync::<FunctionCode>();
        assert_send_sync::<ExceptionCode>();
        assert_send_sync::<ModbusPdu>();
        assert_send_sync::<MbapHeader>();
        assert_send_sync::<TransactionId>();
        assert_send_sync::<ModbusAdu>();
        assert_send_sync::<MbapFramer>();
        assert_send_sync::<TypedDecoder>();
        assert_send_sync::<ModbusSession>();
        assert_send_sync::<SessionStats>();
        assert_send_sync::<ConnectionDefaults>();
        assert_send_sync::<ModbusConnection>();
        assert_send_sync::<ModbusTcpDriver>();
        assert_send_sync::<ModbusOptions>();
        assert_send_sync::<SystemClock>();
    }
}
