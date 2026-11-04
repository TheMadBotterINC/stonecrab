//! RSA-OAEP ciphertext, authenticated OPN payloads and wire mutations with ephemeral keys.
#![no_main]

use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::OpnProbe;

fuzz_target!(|input: &[u8]| {
    static PROBE: OnceLock<OpnProbe> = OnceLock::new();
    let probe = PROBE.get_or_init(|| {
        OpnProbe::new(include_bytes!("../corpus/fuzz_opcua_certificate/rsa-pkcs1.der"))
            .expect("public certificate template and ephemeral RSA key")
    });
    probe.inspect(input).expect("bounded OPN round trip");
});
