// Independent host implementation of the serial framing documented in protocol.md.
const PAYLOAD_MAX: usize = 30;
const ENCODED_MAX: usize = PAYLOAD_MAX + 3;

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for &byte in data {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

pub fn encode(packet: &[u8]) -> Vec<u8> {
    assert!(!packet.is_empty() && packet.len() <= PAYLOAD_MAX);
    let crc = crc16(packet).to_be_bytes();
    let mut frame = vec![0, 0];
    let mut code_index = 1;
    let mut code = 1_u8;
    for &byte in packet.iter().chain(crc.iter()) {
        if byte == 0 {
            frame[code_index] = code;
            code_index = frame.len();
            frame.push(0);
            code = 1;
        } else {
            frame.push(byte);
            code += 1;
        }
    }
    frame[code_index] = code;
    frame.push(0);
    frame
}

fn decode(encoded: &[u8]) -> Option<Vec<u8>> {
    let mut raw = Vec::with_capacity(PAYLOAD_MAX + 2);
    let mut input = 0;
    while input < encoded.len() {
        let code = usize::from(encoded[input]);
        input += 1;
        if code == 0 || input + code - 1 > encoded.len() {
            return None;
        }
        raw.extend_from_slice(&encoded[input..input + code - 1]);
        input += code - 1;
        if code != 255 && input < encoded.len() {
            raw.push(0);
        }
        if raw.len() > PAYLOAD_MAX + 2 {
            return None;
        }
    }
    if raw.len() < 3 {
        return None;
    }
    let length = raw.len() - 2;
    let checksum = u16::from_be_bytes([raw[length], raw[length + 1]]);
    raw.truncate(length);
    (crc16(&raw) == checksum).then_some(raw)
}

#[derive(Default)]
pub struct Decoder {
    encoded: Vec<u8>,
    overflow: bool,
    pub rejected: u64,
}

impl Decoder {
    pub fn push(&mut self, byte: u8) -> Option<Vec<u8>> {
        if byte != 0 {
            if self.encoded.len() < ENCODED_MAX {
                self.encoded.push(byte);
            } else {
                self.overflow = true;
            }
            return None;
        }
        if self.encoded.is_empty() && !self.overflow {
            return None;
        }
        let packet = if self.overflow {
            None
        } else {
            decode(&self.encoded)
        };
        self.encoded.clear();
        self.overflow = false;
        if packet.is_none() {
            self.rejected += 1;
        }
        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_and_wire_vector() {
        assert_eq!(crc16(b"123456789"), 0x29b1);
        assert_eq!(
            encode(&[0x49, 0, 0x50]),
            [0, 2, 0x49, 4, 0x50, 0x15, 0x55, 0]
        );
    }

    #[test]
    fn fragmented_and_concatenated_frames() {
        for length in [18, 24, 26, 30] {
            for fill in [0, 1, 255] {
                let packet = vec![fill; length];
                let frame = encode(&packet);
                for split in 0..=frame.len() {
                    let mut decoder = Decoder::default();
                    let packets: Vec<_> = frame[..split]
                        .iter()
                        .chain(&frame[split..])
                        .chain(frame.iter())
                        .filter_map(|&b| decoder.push(b))
                        .collect();
                    assert_eq!(packets, vec![packet.clone(), packet.clone()]);
                    assert_eq!(decoder.rejected, 0);
                }
            }
        }
    }

    #[test]
    fn corrupt_truncated_and_oversized_input_recovers() {
        let valid = encode(&[0x49, 0x50, 3, 1]);
        let mut corrupt = valid.clone();
        corrupt[2] ^= 1;
        let stream: Vec<_> = b"boot log\n"
            .iter()
            .copied()
            .chain(corrupt)
            .chain([0, 4, 1, 0])
            .chain(vec![1; 10000])
            .chain(valid.clone())
            .chain(valid)
            .collect();
        let mut decoder = Decoder::default();
        let packets: Vec<_> = stream.into_iter().filter_map(|b| decoder.push(b)).collect();
        assert_eq!(packets, vec![vec![0x49, 0x50, 3, 1]; 2]);
        assert_eq!(decoder.rejected, 4);
    }
}
