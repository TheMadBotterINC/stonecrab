//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab_driver_sim::{
        RealtimeClock, SimAddress, SimArea, SimConnection, SimDevice, SimDriver, SimTag,
    };

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<RealtimeClock>();
        assert_send_sync::<SimAddress>();
        assert_send_sync::<SimArea>();
        assert_send_sync::<SimConnection>();
        assert_send_sync::<SimDevice>();
        assert_send_sync::<SimDriver>();
        assert_send_sync::<SimTag>();
    }
}
