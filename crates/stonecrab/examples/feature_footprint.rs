//! No-I/O driver registry workload for scripts/feature-footprint.sh.
#![forbid(unsafe_code)]

use std::hint::black_box;
use std::sync::Arc;

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::{DriverRegistry, Result};

fn main() -> Result<()> {
    for _ in 0..10_000 {
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(ModbusTcpDriver::new()))?;
        #[cfg(feature = "opcua")]
        registry.register(Arc::new(stonecrab::opcua::OpcuaDriver::new()))?;
        black_box(registry);
    }
    Ok(())
}
