use std::fmt;
use std::str::FromStr;

pub const PACKET_SIZE: usize = 26;
const MAGIC: u16 = 0x4950;
const VERSION: u8 = 1;
const TYPE_SAMPLE: u8 = 0x01;
const STATUS_ANGLE_VALID: u8 = 0x01;
const STATUS_I2C_ERROR: u8 = 0x02;
const STATUS_KNOWN_BITS: u8 = STATUS_ANGLE_VALID | STATUS_I2C_ERROR;
const INVALID_ANGLE: u16 = 0xFFFF;
const MAX_ANGLE: u16 = 0x3FFF;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub const fn from_octets(octets: [u8; 6]) -> Self {
        Self(octets)
    }
}

impl FromStr for MacAddress {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() != 6 {
            return Err("expected six colon-separated hexadecimal octets".to_owned());
        }

        let mut octets = [0_u8; 6];
        for (index, part) in parts.iter().enumerate() {
            if part.len() != 2 {
                return Err(format!(
                    "octet {} must contain two hexadecimal digits",
                    index + 1
                ));
            }
            octets[index] = u8::from_str_radix(part, 16)
                .map_err(|_| format!("octet {} is not hexadecimal", index + 1))?;
        }
        Ok(Self(octets))
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SensorSample {
    pub device_mac: MacAddress,
    pub sequence: u32,
    pub timestamp_us: u64,
    pub raw_angle: u16,
    pub status: u8,
}

impl SensorSample {
    pub const fn angle_is_valid(self) -> bool {
        self.status & STATUS_ANGLE_VALID != 0
    }

    pub const fn has_i2c_error(self) -> bool {
        self.status & STATUS_I2C_ERROR != 0
    }

    pub fn angle_degrees(self) -> Option<f64> {
        self.angle_is_valid()
            .then(|| f64::from(self.raw_angle) * 360.0 / 16_384.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DecodeError {
    Length,
    Magic,
    Version,
    PacketType,
    Status,
    Angle,
}

impl DecodeError {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Length => "length",
            Self::Magic => "magic",
            Self::Version => "version",
            Self::PacketType => "packet type",
            Self::Status => "status",
            Self::Angle => "angle",
        }
    }
}

pub fn decode_sample(packet: &[u8]) -> Result<SensorSample, DecodeError> {
    if packet.len() != PACKET_SIZE {
        return Err(DecodeError::Length);
    }

    if u16::from_be_bytes([packet[0], packet[1]]) != MAGIC {
        return Err(DecodeError::Magic);
    }
    if packet[2] != VERSION {
        return Err(DecodeError::Version);
    }
    if packet[3] != TYPE_SAMPLE {
        return Err(DecodeError::PacketType);
    }

    let mac = MacAddress::from_octets([
        packet[4], packet[5], packet[6], packet[7], packet[8], packet[9],
    ]);
    let sequence = u32::from_be_bytes([packet[10], packet[11], packet[12], packet[13]]);
    let timestamp_us = u64::from_be_bytes([
        packet[14], packet[15], packet[16], packet[17], packet[18], packet[19], packet[20],
        packet[21],
    ]);
    let raw_angle = u16::from_be_bytes([packet[22], packet[23]]);
    let status = packet[24];

    if status & !STATUS_KNOWN_BITS != 0 {
        return Err(DecodeError::Status);
    }

    let angle_is_valid = status & STATUS_ANGLE_VALID != 0;
    let i2c_error = status & STATUS_I2C_ERROR != 0;
    if angle_is_valid {
        if i2c_error {
            return Err(DecodeError::Status);
        }
        if raw_angle > MAX_ANGLE {
            return Err(DecodeError::Angle);
        }
    } else if !i2c_error || raw_angle != INVALID_ANGLE {
        return Err(DecodeError::Status);
    }

    // Offset 25 is reserved and explicitly ignored by version 1 receivers.
    Ok(SensorSample {
        device_mac: mac,
        sequence,
        timestamp_us,
        raw_angle,
        status,
    })
}
