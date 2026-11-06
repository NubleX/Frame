//! Behaviour of the wire format against valid and hostile input.

#![allow(clippy::unwrap_used)]

use frame_membrane::{
    Command, DecodeError, EncodeError, HEADER_LEN, Header, Key, MAX_HEADERS, MAX_PAYLOAD, MAX_TEXT,
    Modifiers, MouseButton, Observation, Protocol, VERSION, decode_command, decode_observation,
};

fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![VERSION, kind];
    let length = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

fn samples() -> Vec<Observation> {
    vec![
        Observation::RequestStarted {
            request_id: 1,
            method: "GET".into(),
            url: "https://example.com/a?b=c".into(),
            resource_type: "document".into(),
        },
        Observation::ResponseReceived {
            request_id: 1,
            status: 200,
            headers: vec![Header {
                name: "content-length".into(),
                value: "42".into(),
            }],
        },
        Observation::RequestFinished {
            request_id: 1,
            duration_ms: 123,
            from_cache: true,
        },
        Observation::RequestFailed {
            request_id: 2,
            error: "net::ERR_FAILED".into(),
        },
        Observation::Flow {
            protocol: Protocol::Udp,
            host: "example.com".into(),
            port: 443,
            bytes_out: 10,
            bytes_in: 20,
        },
    ]
}

#[test]
fn every_observation_survives_a_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    for observation in samples() {
        let bytes = observation.encode()?;
        assert_eq!(
            decode_observation(&bytes)?,
            (observation.clone(), bytes.len())
        );
    }
    Ok(())
}

#[test]
fn every_command_survives_a_round_trip() -> Result<(), Box<dyn std::error::Error>> {
    let commands = [
        Command::Navigate {
            url: "https://example.com/".into(),
        },
        Command::Reload,
        Command::Stop,
    ];
    for command in commands {
        let bytes = command.encode()?;
        assert_eq!(decode_command(&bytes)?, (command.clone(), bytes.len()));
    }
    Ok(())
}

#[test]
fn back_to_back_frames_decode_one_at_a_time() -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = Vec::new();
    for observation in samples() {
        stream.extend(observation.encode()?);
    }
    let mut rest = stream.as_slice();
    let mut decoded = Vec::new();
    while !rest.is_empty() {
        let (observation, consumed) = decode_observation(rest)?;
        decoded.push(observation);
        rest = &rest[consumed..];
    }
    assert_eq!(decoded, samples());
    Ok(())
}

#[test]
fn outer_side_cannot_send_a_command() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = Command::Reload.encode()?;
    assert_eq!(decode_observation(&bytes), Err(DecodeError::WrongDirection));
    Ok(())
}

#[test]
fn inner_side_decoder_of_commands_refuses_observations() -> Result<(), Box<dyn std::error::Error>> {
    for observation in samples() {
        assert_eq!(
            decode_command(&observation.encode()?),
            Err(DecodeError::WrongDirection)
        );
    }
    Ok(())
}

#[test]
fn every_prefix_of_a_frame_is_incomplete() -> Result<(), Box<dyn std::error::Error>> {
    for observation in samples() {
        let bytes = observation.encode()?;
        for length in 0..bytes.len() {
            assert_eq!(
                decode_observation(&bytes[..length]),
                Err(DecodeError::Incomplete)
            );
        }
    }
    Ok(())
}

#[test]
fn wrong_version_is_refused() {
    let mut bytes = frame(0x01, &[]);
    bytes[0] = VERSION + 1;
    assert_eq!(decode_observation(&bytes), Err(DecodeError::BadVersion));
}

#[test]
fn undefined_kinds_are_refused() {
    for kind in [0x00, 0x07, 0x7f, 0x80, 0x8b, 0xff] {
        assert_eq!(
            decode_observation(&frame(kind, &[])),
            Err(DecodeError::UnknownKind),
            "kind {kind:#x}"
        );
    }
}

#[test]
fn oversize_length_is_refused_before_any_payload_arrives() {
    let mut bytes = vec![VERSION, 0x01];
    let length = u32::try_from(MAX_PAYLOAD + 1).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&length.to_be_bytes());
    assert_eq!(bytes.len(), HEADER_LEN);
    assert_eq!(decode_observation(&bytes), Err(DecodeError::Oversize));
}

#[test]
fn maximum_declared_length_cannot_overflow() {
    let mut bytes = vec![VERSION, 0x01];
    bytes.extend_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(decode_observation(&bytes), Err(DecodeError::Oversize));
}

#[test]
fn trailing_bytes_in_a_payload_are_refused() {
    let mut payload = 1u64.to_be_bytes().to_vec();
    payload.extend_from_slice(&5u64.to_be_bytes());
    payload.push(0);
    payload.push(0xaa);
    assert_eq!(
        decode_observation(&frame(0x03, &payload)),
        Err(DecodeError::TrailingBytes)
    );
}

#[test]
fn field_running_past_the_payload_is_refused() {
    let payload = [0u8; 4];
    assert_eq!(
        decode_observation(&frame(0x03, &payload)),
        Err(DecodeError::Truncated)
    );
}

#[test]
fn boolean_must_be_zero_or_one() {
    let mut payload = 1u64.to_be_bytes().to_vec();
    payload.extend_from_slice(&5u64.to_be_bytes());
    payload.push(2);
    assert_eq!(
        decode_observation(&frame(0x03, &payload)),
        Err(DecodeError::BadValue)
    );
}

fn failed_payload(text: &[u8]) -> Vec<u8> {
    let mut payload = 1u64.to_be_bytes().to_vec();
    let length = u16::try_from(text.len()).unwrap_or(u16::MAX);
    payload.extend_from_slice(&length.to_be_bytes());
    payload.extend_from_slice(text);
    payload
}

#[test]
fn invalid_utf8_is_refused() {
    assert_eq!(
        decode_observation(&frame(0x04, &failed_payload(&[0xff, 0xfe]))),
        Err(DecodeError::BadText)
    );
}

#[test]
fn control_characters_are_refused_so_logs_cannot_be_forged() {
    for text in [
        "line\nforged",
        "esc\u{1b}[31m",
        "nul\0",
        "del\u{7f}",
        "c1\u{85}",
    ] {
        assert_eq!(
            decode_observation(&frame(0x04, &failed_payload(text.as_bytes()))),
            Err(DecodeError::BadText),
            "{text:?}"
        );
    }
}

#[test]
fn direction_overrides_are_refused_so_urls_cannot_be_disguised() {
    for text in ["evil\u{202e}moc.elpmaxe", "isolate\u{2066}x"] {
        assert_eq!(
            decode_observation(&frame(0x04, &failed_payload(text.as_bytes()))),
            Err(DecodeError::BadText),
            "{text:?}"
        );
    }
}

#[test]
fn text_over_the_limit_is_refused() {
    let long = vec![b'a'; MAX_TEXT + 1];
    assert_eq!(
        decode_observation(&frame(0x04, &failed_payload(&long))),
        Err(DecodeError::BadText)
    );
}

#[test]
fn status_outside_the_http_range_is_refused() {
    for status in [0u16, 99, 600, u16::MAX] {
        let mut payload = 1u64.to_be_bytes().to_vec();
        payload.extend_from_slice(&status.to_be_bytes());
        payload.extend_from_slice(&0u16.to_be_bytes());
        assert_eq!(
            decode_observation(&frame(0x02, &payload)),
            Err(DecodeError::BadValue),
            "status {status}"
        );
    }
}

#[test]
fn too_many_headers_are_refused_before_allocating() {
    let mut payload = 1u64.to_be_bytes().to_vec();
    payload.extend_from_slice(&200u16.to_be_bytes());
    let count = u16::try_from(MAX_HEADERS + 1).unwrap_or(u16::MAX);
    payload.extend_from_slice(&count.to_be_bytes());
    assert_eq!(
        decode_observation(&frame(0x02, &payload)),
        Err(DecodeError::TooManyHeaders)
    );
}

#[test]
fn encoder_refuses_what_the_decoder_would_refuse() {
    let forged = Observation::RequestFailed {
        request_id: 1,
        error: "a\nb".into(),
    };
    assert_eq!(forged.encode(), Err(EncodeError::Invalid));
    let bad_status = Observation::ResponseReceived {
        request_id: 1,
        status: 42,
        headers: Vec::new(),
    };
    assert_eq!(bad_status.encode(), Err(EncodeError::Invalid));
}

/// xorshift64: deterministic, so a failure here always reproduces.
fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

#[test]
fn random_bytes_never_panic_and_never_over_consume() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for _ in 0..200_000 {
        let length = usize::try_from(next(&mut state) % 96).unwrap_or(0);
        let mut bytes: Vec<u8> = (0..length)
            .map(|_| next(&mut state).to_be_bytes()[0])
            .collect();
        if let Some(first) = bytes.first_mut() {
            *first = VERSION;
        }
        if let Ok((_, consumed)) = decode_observation(&bytes) {
            assert!(consumed <= bytes.len());
        }
        if let Ok((_, consumed)) = decode_command(&bytes) {
            assert!(consumed <= bytes.len());
        }
    }
}

#[test]
fn mutated_valid_frames_never_panic_and_stay_consistent() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = 0xdead_beef_cafe_f00d_u64;
    let frames = samples()
        .iter()
        .map(Observation::encode)
        .collect::<Result<Vec<_>, _>>()?;
    for _ in 0..200_000 {
        let index = usize::try_from(next(&mut state) % 5).unwrap_or(0);
        let mut bytes = frames[index].clone();
        for _ in 0..1 + next(&mut state) % 4 {
            let position = usize::try_from(next(&mut state)).unwrap_or(0) % bytes.len();
            bytes[position] = next(&mut state).to_be_bytes()[0];
        }
        if let Ok((observation, consumed)) = decode_observation(&bytes) {
            assert_eq!(observation.encode()?, bytes[..consumed].to_vec());
        }
    }
    Ok(())
}

#[test]
fn input_commands_round_trip() {
    let modifiers = Modifiers {
        shift: true,
        control: false,
        alt: true,
    };
    for command in [
        Command::Back,
        Command::Forward,
        Command::MouseMove { x: 1919, y: 1079 },
        Command::MouseButton {
            x: 10,
            y: 20,
            button: MouseButton::Right,
            pressed: true,
            clicks: 2,
        },
        Command::Wheel {
            x: 1,
            y: 2,
            delta_x: -120,
            delta_y: 120,
        },
        Command::Key {
            key: Key::PageDown,
            modifiers,
        },
        Command::Text { character: 'é' },
    ] {
        let frame = command.encode().unwrap();
        assert_eq!(decode_command(&frame), Ok((command, frame.len())));
    }
}

#[test]
fn address_observation_round_trips() {
    let observation = Observation::Address {
        url: "https://example.com/".into(),
    };
    let frame = observation.encode().unwrap();
    assert_eq!(decode_observation(&frame), Ok((observation, frame.len())));
}

#[test]
fn input_commands_stay_one_way() {
    let frame = Command::MouseMove { x: 1, y: 1 }.encode().unwrap();
    assert_eq!(decode_observation(&frame), Err(DecodeError::WrongDirection));
    let frame = Observation::Address { url: "a".into() }.encode().unwrap();
    assert_eq!(decode_command(&frame), Err(DecodeError::WrongDirection));
}

#[test]
fn refuses_bad_input_values() {
    let mouse = Command::MouseButton {
        x: 1,
        y: 1,
        button: MouseButton::Left,
        pressed: true,
        clicks: 1,
    };
    let mut frame = mouse.encode().unwrap();
    let button = HEADER_LEN + 4;
    frame[button] = 3;
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
    frame[button] = 0;
    frame[button + 1] = 2;
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
    frame[button + 1] = 1;
    frame[button + 2] = 0;
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));

    let key = Command::Key {
        key: Key::Enter,
        modifiers: Modifiers::default(),
    };
    let mut frame = key.encode().unwrap();
    frame[HEADER_LEN] = 200;
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
    frame[HEADER_LEN] = 0;
    frame[HEADER_LEN + 1] = 8;
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
}

#[test]
fn refuses_control_and_invalid_characters() {
    assert_eq!(
        Command::Text { character: '\n' }.encode(),
        Err(EncodeError::Invalid)
    );
    let mut frame = Command::Text { character: 'a' }.encode().unwrap();
    frame[HEADER_LEN..].copy_from_slice(&0xd800u32.to_be_bytes());
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
    frame[HEADER_LEN..].copy_from_slice(&0x0au32.to_be_bytes());
    assert_eq!(decode_command(&frame), Err(DecodeError::BadValue));
}
