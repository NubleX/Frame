//! Machine-checked proofs, run with `cargo kani`. Each covers every possible
//! input within its stated size, not samples.
//!
//! These cover the parts every message goes through. The full decoders are not
//! proven: the checker did not finish on them, so they rest on tests and fuzzing.

use super::*;

/// A header and a small payload.
const FRAME_BYTES: usize = HEADER_LEN + 4;

fn bounded<const N: usize>() -> ([u8; N], usize) {
    let bytes: [u8; N] = kani::any();
    let length: usize = kani::any();
    kani::assume(length <= N);
    (bytes, length)
}

/// For any requested size, a read never leaves the buffer. Every field of
/// every message is read through this one function.
#[kani::proof]
#[kani::unwind(10)]
fn no_read_ever_leaves_the_buffer() {
    let (bytes, length) = bounded::<8>();
    let count: usize = kani::any();
    let mut reader = Reader {
        bytes: &bytes[..length],
    };
    match reader.take(count) {
        Ok(taken) => {
            assert!(count <= length);
            assert!(taken.len() == count);
            assert!(reader.bytes.len() == length - count);
        }
        Err(error) => {
            assert!(count > length);
            assert!(error == DecodeError::Truncated);
        }
    }
}

/// A declared length can never make a frame reach past the bytes received, or
/// past the size limit.
#[kani::proof]
#[kani::unwind(12)]
fn frames_stay_inside_the_buffer_and_the_limit() {
    let (bytes, length) = bounded::<FRAME_BYTES>();
    if let Ok((_, payload, consumed)) = split_frame(&bytes[..length]) {
        assert!(consumed <= length);
        assert!(consumed == HEADER_LEN + payload.len());
        assert!(payload.len() <= MAX_PAYLOAD);
    }
}

/// A kind is never both, so no message can be read in both directions.
#[kani::proof]
fn no_kind_belongs_to_both_directions() {
    let kind: u8 = kani::any();
    assert!(!(is_command_kind(kind) && is_observation_kind(kind)));
}

/// Modifiers round-trip, and bits outside the three known are refused.
#[kani::proof]
fn modifiers_round_trip_and_unknown_bits_are_refused() {
    let bits: u8 = kani::any();
    match Modifiers::from_bits(bits) {
        Some(modifiers) => assert_eq!(modifiers.to_bits(), bits),
        None => assert!(bits > 0b111),
    }
}

/// A typed character is never a control or direction character, for every
/// possible `u32` the wire can carry.
#[kani::proof]
fn typed_characters_are_never_control_characters() {
    let code: u32 = kani::any();
    if let Some(character) = char::from_u32(code).filter(|c| character_is_valid(*c)) {
        assert!(!character.is_control());
        assert!(!matches!(code, 0x202a..=0x202e | 0x2066..=0x2069));
    }
}
