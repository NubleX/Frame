//! The wire format between the outer side and the inner side.
//!
//! A compartment may be fully compromised, so every byte arriving here is
//! hostile. Decoding is strict: one version, known kinds, hard size limits,
//! no trailing bytes, no control characters in text.
//!
//! Flow is one way by type. The outer side sends [`Observation`]s and the
//! inner side sends [`Command`]s; each decoder refuses the other's kinds.
//! Messages name no compartment: the broker knows which channel a frame came
//! from, so a guest cannot claim to be another compartment.
//!
//! Frame layout: version `u8`, kind `u8`, payload length `u32` big-endian,
//! payload. Text is a `u16` length followed by UTF-8.

use std::fmt;

/// The only accepted format version.
pub const VERSION: u8 = 1;
/// Bytes before the payload.
pub const HEADER_LEN: usize = 6;
/// Largest accepted payload.
pub const MAX_PAYLOAD: usize = 64 * 1024;
/// Largest accepted text field, in bytes.
pub const MAX_TEXT: usize = 8 * 1024;
/// Most headers accepted on one response.
pub const MAX_HEADERS: usize = 128;

const KIND_REQUEST_STARTED: u8 = 0x01;
const KIND_RESPONSE_RECEIVED: u8 = 0x02;
const KIND_REQUEST_FINISHED: u8 = 0x03;
const KIND_REQUEST_FAILED: u8 = 0x04;
const KIND_FLOW: u8 = 0x05;
const KIND_NAVIGATE: u8 = 0x81;
const KIND_RELOAD: u8 = 0x82;
const KIND_STOP: u8 = 0x83;

/// One HTTP header as observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Header name.
    pub name: String,
    /// Header value.
    pub value: String,
}

/// Transport protocol of a network flow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// TCP.
    Tcp,
    /// UDP, which includes QUIC.
    Udp,
}

/// What the outer side reports. Outer to inner only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Observation {
    /// The engine began a request.
    RequestStarted {
        /// Engine request id, unique within the compartment.
        request_id: u64,
        /// HTTP method.
        method: String,
        /// Full URL.
        url: String,
        /// Resource type such as `document` or `script`.
        resource_type: String,
    },
    /// Response headers arrived.
    ResponseReceived {
        /// Engine request id.
        request_id: u64,
        /// HTTP status code.
        status: u16,
        /// Response headers.
        headers: Vec<Header>,
    },
    /// The request completed.
    RequestFinished {
        /// Engine request id.
        request_id: u64,
        /// Total time in milliseconds.
        duration_ms: u64,
        /// Served from cache.
        from_cache: bool,
    },
    /// The request failed.
    RequestFailed {
        /// Engine request id.
        request_id: u64,
        /// Engine error text.
        error: String,
    },
    /// The gate saw a network flow.
    Flow {
        /// Transport protocol.
        protocol: Protocol,
        /// Remote host name or address.
        host: String,
        /// Remote port.
        port: u16,
        /// Bytes sent.
        bytes_out: u64,
        /// Bytes received.
        bytes_in: u64,
    },
}

/// What the inner side asks for. Inner to outer only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Load a URL.
    Navigate {
        /// Target URL.
        url: String,
    },
    /// Reload the page.
    Reload,
    /// Stop loading.
    Stop,
}

/// Why a frame was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// More bytes are needed. Not an attack; read more and retry.
    Incomplete,
    /// Unsupported format version.
    BadVersion,
    /// Kind is not defined at all.
    UnknownKind,
    /// A valid kind sent in the forbidden direction.
    WrongDirection,
    /// Declared payload is over [`MAX_PAYLOAD`].
    Oversize,
    /// A field runs past the end of the payload.
    Truncated,
    /// Bytes remain after the last field.
    TrailingBytes,
    /// Text is over the limit, not UTF-8, or holds control or direction characters.
    BadText,
    /// A value is outside its allowed range.
    BadValue,
    /// More headers than [`MAX_HEADERS`].
    TooManyHeaders,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "refused frame: {self:?}")
    }
}

impl std::error::Error for DecodeError {}

/// A message cannot be represented on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// A field would be refused by the decoder.
    Invalid,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("message cannot be encoded")
    }
}

impl std::error::Error for EncodeError {}

/// Direction overrides can make a logged URL read as something else.
fn is_bidi_control(character: char) -> bool {
    matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn text_is_valid(text: &str) -> bool {
    text.len() <= MAX_TEXT
        && !text
            .chars()
            .any(|character| character.is_control() || is_bidi_control(character))
}

fn status_is_valid(status: u16) -> bool {
    (100..=599).contains(&status)
}

struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], DecodeError> {
        if count > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let (head, tail) = self.bytes.split_at(count);
        self.bytes = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.take(N)?.try_into().map_err(|_| DecodeError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        Ok(u64::from_be_bytes(self.array()?))
    }

    fn flag(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadValue),
        }
    }

    fn text(&mut self) -> Result<String, DecodeError> {
        let length = usize::from(self.u16()?);
        if length > MAX_TEXT {
            return Err(DecodeError::BadText);
        }
        let text = std::str::from_utf8(self.take(length)?).map_err(|_| DecodeError::BadText)?;
        if text_is_valid(text) {
            Ok(text.to_owned())
        } else {
            Err(DecodeError::BadText)
        }
    }

    fn finish(&self) -> Result<(), DecodeError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::TrailingBytes)
        }
    }
}

/// Splits one frame off the front of `buffer`.
fn split_frame(buffer: &[u8]) -> Result<(u8, &[u8], usize), DecodeError> {
    let Some(header) = buffer.first_chunk::<HEADER_LEN>() else {
        return Err(DecodeError::Incomplete);
    };
    let [version, kind, length @ ..] = *header;
    if version != VERSION {
        return Err(DecodeError::BadVersion);
    }
    let length = usize::try_from(u32::from_be_bytes(length)).map_err(|_| DecodeError::Oversize)?;
    if length > MAX_PAYLOAD {
        return Err(DecodeError::Oversize);
    }
    let total = HEADER_LEN + length;
    let payload = buffer
        .get(HEADER_LEN..total)
        .ok_or(DecodeError::Incomplete)?;
    Ok((kind, payload, total))
}

fn is_observation_kind(kind: u8) -> bool {
    (KIND_REQUEST_STARTED..=KIND_FLOW).contains(&kind)
}

fn is_command_kind(kind: u8) -> bool {
    (KIND_NAVIGATE..=KIND_STOP).contains(&kind)
}

/// Decodes one observation. Returns it with the bytes consumed.
///
/// # Errors
/// [`DecodeError::Incomplete`] asks for more bytes. Anything else means the
/// sender is broken or hostile, and the channel should be closed.
pub fn decode_observation(buffer: &[u8]) -> Result<(Observation, usize), DecodeError> {
    let (kind, payload, consumed) = split_frame(buffer)?;
    if is_command_kind(kind) {
        return Err(DecodeError::WrongDirection);
    }
    if !is_observation_kind(kind) {
        return Err(DecodeError::UnknownKind);
    }
    let mut reader = Reader { bytes: payload };
    let observation = match kind {
        KIND_REQUEST_STARTED => Observation::RequestStarted {
            request_id: reader.u64()?,
            method: reader.text()?,
            url: reader.text()?,
            resource_type: reader.text()?,
        },
        KIND_RESPONSE_RECEIVED => {
            let request_id = reader.u64()?;
            let status = reader.u16()?;
            if !status_is_valid(status) {
                return Err(DecodeError::BadValue);
            }
            let count = usize::from(reader.u16()?);
            if count > MAX_HEADERS {
                return Err(DecodeError::TooManyHeaders);
            }
            let mut headers = Vec::with_capacity(count);
            for _ in 0..count {
                headers.push(Header {
                    name: reader.text()?,
                    value: reader.text()?,
                });
            }
            Observation::ResponseReceived {
                request_id,
                status,
                headers,
            }
        }
        KIND_REQUEST_FINISHED => Observation::RequestFinished {
            request_id: reader.u64()?,
            duration_ms: reader.u64()?,
            from_cache: reader.flag()?,
        },
        KIND_REQUEST_FAILED => Observation::RequestFailed {
            request_id: reader.u64()?,
            error: reader.text()?,
        },
        KIND_FLOW => Observation::Flow {
            protocol: match reader.u8()? {
                0 => Protocol::Tcp,
                1 => Protocol::Udp,
                _ => return Err(DecodeError::BadValue),
            },
            host: reader.text()?,
            port: reader.u16()?,
            bytes_out: reader.u64()?,
            bytes_in: reader.u64()?,
        },
        _ => return Err(DecodeError::UnknownKind),
    };
    reader.finish()?;
    Ok((observation, consumed))
}

/// Decodes one command. Returns it with the bytes consumed.
///
/// # Errors
/// As for [`decode_observation`].
pub fn decode_command(buffer: &[u8]) -> Result<(Command, usize), DecodeError> {
    let (kind, payload, consumed) = split_frame(buffer)?;
    if is_observation_kind(kind) {
        return Err(DecodeError::WrongDirection);
    }
    let mut reader = Reader { bytes: payload };
    let command = match kind {
        KIND_NAVIGATE => Command::Navigate {
            url: reader.text()?,
        },
        KIND_RELOAD => Command::Reload,
        KIND_STOP => Command::Stop,
        _ => return Err(DecodeError::UnknownKind),
    };
    reader.finish()?;
    Ok((command, consumed))
}

struct Writer {
    payload: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self {
            payload: Vec::new(),
        }
    }

    fn u8(&mut self, value: u8) {
        self.payload.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.payload.extend_from_slice(&value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.payload.extend_from_slice(&value.to_be_bytes());
    }

    fn text(&mut self, text: &str) -> Result<(), EncodeError> {
        if !text_is_valid(text) {
            return Err(EncodeError::Invalid);
        }
        let length = u16::try_from(text.len()).map_err(|_| EncodeError::Invalid)?;
        self.u16(length);
        self.payload.extend_from_slice(text.as_bytes());
        Ok(())
    }

    fn finish(self, kind: u8) -> Result<Vec<u8>, EncodeError> {
        if self.payload.len() > MAX_PAYLOAD {
            return Err(EncodeError::Invalid);
        }
        let length = u32::try_from(self.payload.len()).map_err(|_| EncodeError::Invalid)?;
        let mut frame = Vec::with_capacity(HEADER_LEN + self.payload.len());
        frame.push(VERSION);
        frame.push(kind);
        frame.extend_from_slice(&length.to_be_bytes());
        frame.extend_from_slice(&self.payload);
        Ok(frame)
    }
}

impl Observation {
    /// Encodes this observation as one frame.
    ///
    /// # Errors
    /// [`EncodeError::Invalid`] if the decoder would refuse it.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut writer = Writer::new();
        let kind = match self {
            Self::RequestStarted {
                request_id,
                method,
                url,
                resource_type,
            } => {
                writer.u64(*request_id);
                writer.text(method)?;
                writer.text(url)?;
                writer.text(resource_type)?;
                KIND_REQUEST_STARTED
            }
            Self::ResponseReceived {
                request_id,
                status,
                headers,
            } => {
                if !status_is_valid(*status) || headers.len() > MAX_HEADERS {
                    return Err(EncodeError::Invalid);
                }
                writer.u64(*request_id);
                writer.u16(*status);
                writer.u16(u16::try_from(headers.len()).map_err(|_| EncodeError::Invalid)?);
                for header in headers {
                    writer.text(&header.name)?;
                    writer.text(&header.value)?;
                }
                KIND_RESPONSE_RECEIVED
            }
            Self::RequestFinished {
                request_id,
                duration_ms,
                from_cache,
            } => {
                writer.u64(*request_id);
                writer.u64(*duration_ms);
                writer.u8(u8::from(*from_cache));
                KIND_REQUEST_FINISHED
            }
            Self::RequestFailed { request_id, error } => {
                writer.u64(*request_id);
                writer.text(error)?;
                KIND_REQUEST_FAILED
            }
            Self::Flow {
                protocol,
                host,
                port,
                bytes_out,
                bytes_in,
            } => {
                writer.u8(match protocol {
                    Protocol::Tcp => 0,
                    Protocol::Udp => 1,
                });
                writer.text(host)?;
                writer.u16(*port);
                writer.u64(*bytes_out);
                writer.u64(*bytes_in);
                KIND_FLOW
            }
        };
        writer.finish(kind)
    }
}

impl Command {
    /// Encodes this command as one frame.
    ///
    /// # Errors
    /// [`EncodeError::Invalid`] if the decoder would refuse it.
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        let mut writer = Writer::new();
        let kind = match self {
            Self::Navigate { url } => {
                writer.text(url)?;
                KIND_NAVIGATE
            }
            Self::Reload => KIND_RELOAD,
            Self::Stop => KIND_STOP,
        };
        writer.finish(kind)
    }
}
