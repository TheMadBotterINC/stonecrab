//! Synthetic bench shared by the one-shot and continuous examples.

use stonecrab::sim::{SimDevice, SimTag};
use stonecrab::{PlcError, PlcValue};

/// Coils 1 to 10 of the interop register map (row MB-INTEROP-COIL).
const COILS: [bool; 10] = [
    true, false, true, true, false, true, false, false, true, false,
];

/// Discrete inputs 1 to 10 of the interop register map (row MB-INTEROP-DI).
const DISCRETE_INPUTS: [bool; 10] = [
    false, true, false, true, true, false, true, true, false, true,
];

/// The simulated `bench` device: the values the interop register map serves over Modbus.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private example module"
)]
pub(crate) fn bench_device() -> Result<SimDevice, PlcError> {
    let device = SimDevice::new();
    let values = [
        // REAL 3.141593 at holding registers 26-27 (0x4049 0x0FDC, MB-INTEROP-HR-07).
        ("temperature", PlcValue::F32(f32::from_bits(0x4049_0FDC))),
        // INT -2424 at holding register 12 (MB-INTEROP-HR-04).
        ("pressure", PlcValue::I16(-2424)),
        // UDINT 4242442424 at holding registers 16-17 (MB-INTEROP-HR-05).
        ("flow", PlcValue::U32(4_242_442_424)),
        // WORD 0xA5B8 at holding register 3 (MB-INTEROP-HR-02).
        ("flow_status", PlcValue::U16(0xA5B8)),
        // UINT 42424 at holding register 13 (MB-INTEROP-HR-04).
        ("level", PlcValue::U16(42_424)),
    ];
    for (name, value) in values {
        device.set(name, SimTag::new(value))?;
    }
    for (number, state) in (1_u8..).zip(COILS) {
        device.set(
            &format!("coil_{number}"),
            SimTag::new(PlcValue::Bool(state)),
        )?;
    }
    for (number, state) in (1_u8..).zip(DISCRETE_INPUTS) {
        let name = format!("discrete_input_{number}");
        device.set(&name, SimTag::new(PlcValue::Bool(state)))?;
    }
    Ok(device)
}
