#![no_main]

use frame_membrane::{decode_command, decode_observation};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok((observation, consumed)) = decode_observation(bytes) {
        assert!(consumed <= bytes.len());
        assert_eq!(observation.encode().ok().as_deref(), bytes.get(..consumed));
    }
    if let Ok((command, consumed)) = decode_command(bytes) {
        assert!(consumed <= bytes.len());
        assert_eq!(command.encode().ok().as_deref(), bytes.get(..consumed));
    }
});
