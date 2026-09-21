//! Stable little-endian datagrams shared by the C shell and Rust client.
use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Request {
    opcode: u32,
    volume: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidRequest;

impl fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid Way Shell request")
    }
}
impl Error for InvalidRequest {}

impl Request {
    pub fn new(opcode: u32, volume: Option<f32>) -> Result<Self, InvalidRequest> {
        if opcode > 33 || (opcode == 4) != volume.is_some() {
            return Err(InvalidRequest);
        }
        if volume.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
            return Err(InvalidRequest);
        }
        Ok(Self { opcode, volume })
    }

    pub fn opcode(self) -> u32 {
        self.opcode
    }

    pub fn volume(self) -> Option<f32> {
        self.volume
    }

    pub fn encode(self) -> Vec<u8> {
        let mut bytes = self.opcode.to_le_bytes().to_vec();
        if let Some(volume) = self.volume {
            bytes.extend(volume.to_le_bytes());
        }
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, InvalidRequest> {
        let opcode = u32::from_le_bytes(bytes.get(..4).ok_or(InvalidRequest)?.try_into().unwrap());
        let volume = match (opcode, bytes.len()) {
            (4, 8) => Some(f32::from_le_bytes(bytes[4..8].try_into().unwrap())),
            (4, _) => return Err(InvalidRequest),
            (_, 4) => None,
            _ => return Err(InvalidRequest),
        };
        Self::new(opcode, volume)
    }
}

pub fn encode_response(success: bool) -> [u8; 4] {
    u32::from(success).to_le_bytes()
}

pub fn decode_response(bytes: &[u8]) -> Result<bool, InvalidRequest> {
    match bytes {
        [0, 0, 0, 0] => Ok(false),
        [1, 0, 0, 0] => Ok(true),
        _ => Err(InvalidRequest),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_wire_fixtures() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/cli.json")).unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let opcode = fixture["opcode"].as_u64().unwrap() as u32;
            let volume = fixture["volume"].as_f64().map(|v| v as f32);
            let request = Request::new(opcode, volume).unwrap();
            let bytes = request.encode();
            assert_eq!(&bytes[..4], opcode.to_le_bytes());
            assert_eq!(bytes.len(), if volume.is_some() { 8 } else { 4 });
            assert_eq!(Request::decode(&bytes), Ok(request));
        }
        assert_eq!(
            Request::new(4, Some(0.5)).unwrap().encode(),
            [4, 0, 0, 0, 0, 0, 0, 63]
        );
    }

    #[test]
    fn malformed_requests() {
        for length in 0..12 {
            for opcode in 0..=34u32 {
                let mut bytes = vec![0; length];
                if length >= 4 {
                    bytes[..4].copy_from_slice(&opcode.to_le_bytes());
                }
                assert_eq!(
                    Request::decode(&bytes).is_ok(),
                    opcode <= 33 && length == if opcode == 4 { 8 } else { 4 }
                );
            }
        }
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.1, 1.1] {
            let mut bytes = 4u32.to_le_bytes().to_vec();
            bytes.extend(value.to_le_bytes());
            assert!(Request::decode(&bytes).is_err());
        }
    }

    #[test]
    fn replies_are_exact_four_byte_booleans() {
        assert_eq!(decode_response(&encode_response(true)), Ok(true));
        assert_eq!(decode_response(&encode_response(false)), Ok(false));
        for bytes in [&[][..], &[1], &[2, 0, 0, 0], &[1, 0, 0, 0, 0]] {
            assert!(decode_response(bytes).is_err());
        }
    }
}
