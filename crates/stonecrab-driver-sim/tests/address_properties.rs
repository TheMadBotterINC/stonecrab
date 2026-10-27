//! Property tests of the simulator address grammar (coding standards 13.7 and 13.8).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use stonecrab_core::{ErrorKind, ProtocolAddress};
    use stonecrab_driver_sim::SimAddress;

    const AREAS: [&str; 3] = ["STATE", "RANDOM", "SINK"];
    const TYPES: [&str; 11] = [
        "bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64",
    ];

    proptest! {
        // covers: SIM-ADDRESS-GRAMMAR (no input panics; every failure is InvalidAddress)
        #[test]
        fn prop_sim_address_parse_never_panics(text in "\\PC{0,300}") {
            match SimAddress::parse(&text) {
                Ok(address) => prop_assert!(address.canonical().len() <= SimAddress::MAX_TEXT_BYTES),
                Err(error) => prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress),
            }
        }

        // covers: SIM-ADDRESS-GRAMMAR (control characters and arbitrary bytes)
        #[test]
        fn prop_sim_address_parse_never_panics_on_any_string(text in any::<String>()) {
            if let Err(error) = SimAddress::parse(&text) {
                prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
            }
        }

        // covers: SIM-ADDRESS-GRAMMAR (canonical text parses back to an equal address)
        #[test]
        fn prop_sim_address_canonical_round_trips(
            area in prop::sample::select(AREAS.to_vec()),
            value_type in prop::sample::select(TYPES.to_vec()),
            name in "[A-Za-z0-9_][A-Za-z0-9_ ./:-]{0,60}[A-Za-z0-9_]|[A-Za-z0-9_]",
        ) {
            let text = format!("{area}/{name}:{value_type}");
            let address = SimAddress::parse(&text).expect("a generated address is valid");
            prop_assert_eq!(address.canonical(), text.clone());
            prop_assert_eq!(SimAddress::parse(&address.canonical()).expect("canonical parses"), address);
        }
    }
}
