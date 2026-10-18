//! Transport-neutral framing for interactive remote terminals.
//!
//! Frames are `tag: u8`, `payload length: u32` (big endian), then the payload.
//! The protocol is binary-safe and deliberately independent of SSH so the same
//! messages can be carried by a future HTTP or WebSocket transport.

use anyhow::{Result, anyhow, bail};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_PAYLOAD: usize = 1024 * 1024;
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_CWD_BYTES: usize = 16 * 1024;
pub const MAX_ERROR_BYTES: usize = 16 * 1024;
pub const MAX_TERMINAL_DIMENSION: u16 = 1_000;
const HEADER_LEN: usize = 5;

const CLIENT_OPEN: u8 = 1;
const CLIENT_INPUT: u8 = 2;
const CLIENT_RESIZE: u8 = 3;
const CLIENT_CLOSE: u8 = 4;
const SERVER_OPENED: u8 = 0x81;
const SERVER_OUTPUT: u8 = 0x82;
const SERVER_EXIT: u8 = 0x83;
const SERVER_ERROR: u8 = 0x84;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dimensions {
    pub columns: u16,
    pub rows: u16,
}

impl Dimensions {
    pub fn new(columns: u16, rows: u16) -> Result<Self> {
        if columns == 0
            || rows == 0
            || columns > MAX_TERMINAL_DIMENSION
            || rows > MAX_TERMINAL_DIMENSION
        {
            bail!("terminal dimensions must be between 1 and {MAX_TERMINAL_DIMENSION}");
        }
        Ok(Self { columns, rows })
    }

    fn encode(self, payload: &mut Vec<u8>) {
        payload.extend_from_slice(&self.columns.to_be_bytes());
        payload.extend_from_slice(&self.rows.to_be_bytes());
    }

    fn decode(payload: &[u8]) -> Result<Self> {
        if payload.len() != 4 {
            bail!("terminal dimensions have an invalid length");
        }
        Self::new(
            u16::from_be_bytes([payload[0], payload[1]]),
            u16::from_be_bytes([payload[2], payload[3]]),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage {
    Open { dimensions: Dimensions, cwd: String },
    Input(Vec<u8>),
    Resize(Dimensions),
    Close,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    Opened,
    Output(Vec<u8>),
    Exit { code: Option<u32> },
    Error(String),
}

impl ClientMessage {
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Open { dimensions, cwd } => {
                if cwd.len() > MAX_CWD_BYTES {
                    bail!("terminal working directory is too long");
                }
                let mut payload = Vec::with_capacity(6 + cwd.len());
                payload.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
                dimensions.encode(&mut payload);
                payload.extend_from_slice(cwd.as_bytes());
                frame(CLIENT_OPEN, &payload)
            }
            Self::Input(bytes) => {
                if bytes.len() > MAX_INPUT_BYTES {
                    bail!("terminal input frame is too large");
                }
                frame(CLIENT_INPUT, bytes)
            }
            Self::Resize(dimensions) => {
                let mut payload = Vec::with_capacity(4);
                dimensions.encode(&mut payload);
                frame(CLIENT_RESIZE, &payload)
            }
            Self::Close => frame(CLIENT_CLOSE, &[]),
        }
    }
}

impl ServerMessage {
    pub fn encode(&self) -> Result<Vec<u8>> {
        match self {
            Self::Opened => frame(SERVER_OPENED, &[]),
            Self::Output(bytes) => frame(SERVER_OUTPUT, bytes),
            Self::Exit { code } => match code {
                Some(code) => frame(SERVER_EXIT, &code.to_be_bytes()),
                None => frame(SERVER_EXIT, &[]),
            },
            Self::Error(message) => {
                if message.len() > MAX_ERROR_BYTES {
                    bail!("terminal error frame is too large");
                }
                frame(SERVER_ERROR, message.as_bytes())
            }
        }
    }
}

fn frame(tag: u8, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.len() > MAX_FRAME_PAYLOAD {
        bail!("terminal frame is too large");
    }
    let length =
        u32::try_from(payload.len()).map_err(|_| anyhow!("terminal frame is too large"))?;
    let mut encoded = Vec::with_capacity(HEADER_LEN + payload.len());
    encoded.push(tag);
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

#[derive(Default)]
struct Decoder {
    buffered: Vec<u8>,
}

impl Decoder {
    fn finish(&self) -> Result<()> {
        if self.buffered.is_empty() {
            Ok(())
        } else {
            bail!("terminal transport ended with a truncated frame")
        }
    }

    fn push<T>(
        &mut self,
        mut bytes: &[u8],
        mut decode: impl FnMut(u8, &[u8]) -> Result<T>,
    ) -> Result<Vec<T>> {
        let limit = HEADER_LEN + MAX_FRAME_PAYLOAD;
        let mut messages = Vec::new();
        while !bytes.is_empty() {
            let capacity = limit.saturating_sub(self.buffered.len());
            if capacity == 0 {
                bail!("terminal decoder input exceeded its limit");
            }
            let take = capacity.min(bytes.len());
            self.buffered.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];

            let mut consumed = 0;
            loop {
                let available = &self.buffered[consumed..];
                if available.len() < HEADER_LEN {
                    break;
                }
                let length = u32::from_be_bytes(available[1..5].try_into().unwrap()) as usize;
                if length > MAX_FRAME_PAYLOAD {
                    bail!("terminal frame declares an oversized payload");
                }
                let frame_len = HEADER_LEN + length;
                if available.len() < frame_len {
                    break;
                }
                messages.push(decode(available[0], &available[HEADER_LEN..frame_len])?);
                consumed += frame_len;
            }
            if consumed != 0 {
                self.buffered.drain(..consumed);
            } else if self.buffered.len() == limit && !bytes.is_empty() {
                bail!("terminal decoder input exceeded its limit");
            }
        }
        Ok(messages)
    }
}

#[derive(Default)]
pub struct ClientDecoder(Decoder);

impl ClientDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ClientMessage>> {
        self.0.push(bytes, decode_client)
    }

    pub fn finish(&self) -> Result<()> {
        self.0.finish()
    }
}

#[derive(Default)]
pub struct ServerDecoder(Decoder);

impl ServerDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<ServerMessage>> {
        self.0.push(bytes, decode_server)
    }

    pub fn finish(&self) -> Result<()> {
        self.0.finish()
    }
}

fn decode_client(tag: u8, payload: &[u8]) -> Result<ClientMessage> {
    match tag {
        CLIENT_OPEN => {
            if payload.len() < 6 {
                bail!("terminal open frame is truncated");
            }
            let version = u16::from_be_bytes([payload[0], payload[1]]);
            if version != PROTOCOL_VERSION {
                bail!("unsupported terminal protocol version {version}");
            }
            let dimensions = Dimensions::decode(&payload[2..6])?;
            let cwd = &payload[6..];
            if cwd.len() > MAX_CWD_BYTES {
                bail!("terminal working directory is too long");
            }
            let cwd = std::str::from_utf8(cwd)
                .map_err(|_| anyhow!("terminal working directory is not UTF-8"))?
                .to_owned();
            Ok(ClientMessage::Open { dimensions, cwd })
        }
        CLIENT_INPUT => {
            if payload.len() > MAX_INPUT_BYTES {
                bail!("terminal input frame is too large");
            }
            Ok(ClientMessage::Input(payload.to_vec()))
        }
        CLIENT_RESIZE => Ok(ClientMessage::Resize(Dimensions::decode(payload)?)),
        CLIENT_CLOSE if payload.is_empty() => Ok(ClientMessage::Close),
        CLIENT_CLOSE => bail!("terminal close frame must be empty"),
        _ => bail!("unknown client terminal frame tag {tag}"),
    }
}

fn decode_server(tag: u8, payload: &[u8]) -> Result<ServerMessage> {
    match tag {
        SERVER_OPENED if payload.is_empty() => Ok(ServerMessage::Opened),
        SERVER_OPENED => bail!("terminal opened frame must be empty"),
        SERVER_OUTPUT => Ok(ServerMessage::Output(payload.to_vec())),
        SERVER_EXIT if payload.is_empty() => Ok(ServerMessage::Exit { code: None }),
        SERVER_EXIT if payload.len() == 4 => Ok(ServerMessage::Exit {
            code: Some(u32::from_be_bytes(payload.try_into().unwrap())),
        }),
        SERVER_EXIT => bail!("terminal exit frame has an invalid length"),
        SERVER_ERROR => {
            if payload.len() > MAX_ERROR_BYTES {
                bail!("terminal error frame is too large");
            }
            let message = std::str::from_utf8(payload)
                .map_err(|_| anyhow!("terminal error frame is not UTF-8"))?
                .to_owned();
            Ok(ServerMessage::Error(message))
        }
        _ => bail!("unknown server terminal frame tag {tag}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragmented_client_frames_round_trip() {
        let expected = vec![
            ClientMessage::Open {
                dimensions: Dimensions::new(120, 40).unwrap(),
                cwd: "/tmp/猫".into(),
            },
            ClientMessage::Input(vec![0, b'\n', 0xff, b'\r']),
            ClientMessage::Resize(Dimensions::new(80, 24).unwrap()),
            ClientMessage::Close,
        ];
        let encoded: Vec<u8> = expected
            .iter()
            .flat_map(|message| message.encode().unwrap())
            .collect();
        for split in 0..=encoded.len() {
            let mut decoder = ClientDecoder::new();
            let mut actual = decoder.push(&encoded[..split]).unwrap();
            actual.extend(decoder.push(&encoded[split..]).unwrap());
            decoder.finish().unwrap();
            assert_eq!(actual, expected, "split at {split}");
        }
    }

    #[test]
    fn one_byte_server_fragments_round_trip() {
        let expected = vec![
            ServerMessage::Opened,
            ServerMessage::Output(vec![0, 1, 2, 0xff]),
            ServerMessage::Exit { code: Some(7) },
        ];
        let encoded: Vec<u8> = expected
            .iter()
            .flat_map(|message| message.encode().unwrap())
            .collect();
        let mut decoder = ServerDecoder::new();
        let mut actual = Vec::new();
        for byte in encoded {
            actual.extend(decoder.push(&[byte]).unwrap());
        }
        decoder.finish().unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn aggregate_input_is_bounded_without_rejecting_valid_frame_boundaries() {
        let expected = vec![
            ServerMessage::Output(vec![0; MAX_FRAME_PAYLOAD]),
            ServerMessage::Opened,
        ];
        let encoded: Vec<u8> = expected
            .iter()
            .flat_map(|message| message.encode().unwrap())
            .collect();
        let mut decoder = ServerDecoder::new();
        assert_eq!(decoder.push(&encoded).unwrap(), expected);
        assert!(decoder.0.buffered.is_empty());

        let mut decoder = ClientDecoder::new();
        decoder.push(&[CLIENT_INPUT, 0]).unwrap();
        assert!(
            decoder
                .push(&vec![0; HEADER_LEN + MAX_FRAME_PAYLOAD])
                .is_err()
        );
        assert!(decoder.0.buffered.len() <= HEADER_LEN + MAX_FRAME_PAYLOAD);
    }

    #[test]
    fn rejects_bad_dimensions_unknown_tags_and_oversized_frames() {
        assert!(Dimensions::new(0, 24).is_err());
        assert!(ClientDecoder::new().push(&[99, 0, 0, 0, 0]).is_err());
        let length = u32::try_from(MAX_FRAME_PAYLOAD + 1).unwrap().to_be_bytes();
        let mut oversized = vec![CLIENT_INPUT];
        oversized.extend_from_slice(&length);
        assert!(ClientDecoder::new().push(&oversized).is_err());
    }

    #[test]
    fn rejects_invalid_message_shapes_and_truncated_eof() {
        let mut bad_close = vec![CLIENT_CLOSE, 0, 0, 0, 1, 1];
        assert!(ClientDecoder::new().push(&bad_close).is_err());
        bad_close[0] = SERVER_EXIT;
        assert!(ServerDecoder::new().push(&bad_close).is_err());
        assert!(
            ClientMessage::Input(vec![0; MAX_INPUT_BYTES + 1])
                .encode()
                .is_err()
        );

        let encoded = ServerMessage::Output(b"hello".to_vec()).encode().unwrap();
        let mut decoder = ServerDecoder::new();
        assert!(
            decoder
                .push(&encoded[..encoded.len() - 1])
                .unwrap()
                .is_empty()
        );
        assert!(decoder.finish().is_err());
        assert_eq!(
            decoder.push(&encoded[encoded.len() - 1..]).unwrap(),
            vec![ServerMessage::Output(b"hello".to_vec())]
        );
        decoder.finish().unwrap();
    }
}
