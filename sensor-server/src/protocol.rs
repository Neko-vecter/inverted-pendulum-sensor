use std::fmt;
use std::str::FromStr;

pub const SAMPLE_PACKET_SIZE: usize = 24;
pub const TIME_SYNC_REQUEST_SIZE: usize = 18;
pub const TIME_SYNC_RESPONSE_SIZE: usize = 30;
pub const TIME_SYNC_REPORT_SIZE: usize = 26;
const MAGIC: u16 = 0x4950;
const VERSION: u8 = 3;
const TYPE_SAMPLE: u8 = 0x01;
const TYPE_TIME_SYNC_REQUEST: u8 = 0x02;
const TYPE_TIME_SYNC_RESPONSE: u8 = 0x03;
const TYPE_TIME_SYNC_REPORT: u8 = 0x04;
const STATUS_ANGLE_VALID: u8 = 0x01;
const STATUS_I2C_ERROR: u8 = 0x02;
const MAX_ANGLE: u16 = 0x3FFF;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub const fn from_octets(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    pub const fn octets(self) -> [u8; 6] {
        self.0
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
    pub timestamp_us: u32,
    pub raw_angle: u16,
    pub status: u8,
    pub sync_token: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TimeSyncRequest {
    pub device_mac: MacAddress,
    pub sequence: u32,
    pub device_t1_us: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct TimeSyncReport {
    pub device_mac: MacAddress,
    pub sequence: u32,
    pub device_t1_us: u32,
    pub device_t4_us: u32,
    pub sync_token: u32,
}

#[derive(Clone, Copy, Debug)]
pub enum DecodedDatagram {
    Sample(SensorSample),
    TimeSyncRequest(TimeSyncRequest),
    TimeSyncReport(TimeSyncReport),
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
    SyncToken,
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
            Self::SyncToken => "sync token",
        }
    }
}

pub fn decode_datagram(packet: &[u8]) -> Result<DecodedDatagram, DecodeError> {
    if packet.len() < 4 {
        return Err(DecodeError::Length);
    }
    if u16::from_be_bytes([packet[0], packet[1]]) != MAGIC {
        return Err(DecodeError::Magic);
    }
    if packet[2] != VERSION {
        return Err(DecodeError::Version);
    }

    match packet[3] {
        TYPE_SAMPLE => decode_sample(packet).map(DecodedDatagram::Sample),
        TYPE_TIME_SYNC_REQUEST => {
            decode_time_sync_request(packet).map(DecodedDatagram::TimeSyncRequest)
        }
        TYPE_TIME_SYNC_REPORT => {
            decode_time_sync_report(packet).map(DecodedDatagram::TimeSyncReport)
        }
        _ => Err(DecodeError::PacketType),
    }
}

fn decode_sample(packet: &[u8]) -> Result<SensorSample, DecodeError> {
    if packet.len() != SAMPLE_PACKET_SIZE {
        return Err(DecodeError::Length);
    }

    let angle_state = u16::from_be_bytes(packet[18..20].try_into().unwrap());
    let angle_status = angle_state >> 14;
    let raw_angle = angle_state & MAX_ANGLE;
    let (status, raw_angle) = match angle_status {
        0 => (STATUS_ANGLE_VALID, raw_angle),
        1 if raw_angle == 0 => (STATUS_I2C_ERROR, 0),
        1 => return Err(DecodeError::Angle),
        _ => return Err(DecodeError::Status),
    };
    let sync_token = u32::from_be_bytes(packet[20..24].try_into().unwrap());

    Ok(SensorSample {
        device_mac: read_mac(&packet[4..10]),
        sequence: u32::from_be_bytes(packet[10..14].try_into().unwrap()),
        timestamp_us: u32::from_be_bytes(packet[14..18].try_into().unwrap()),
        raw_angle,
        status,
        sync_token,
    })
}

fn decode_time_sync_report(packet: &[u8]) -> Result<TimeSyncReport, DecodeError> {
    if packet.len() != TIME_SYNC_REPORT_SIZE {
        return Err(DecodeError::Length);
    }
    let sync_token = u32::from_be_bytes(packet[22..26].try_into().unwrap());
    if sync_token == 0 {
        return Err(DecodeError::SyncToken);
    }
    Ok(TimeSyncReport {
        device_mac: read_mac(&packet[4..10]),
        sequence: u32::from_be_bytes(packet[10..14].try_into().unwrap()),
        device_t1_us: u32::from_be_bytes(packet[14..18].try_into().unwrap()),
        device_t4_us: u32::from_be_bytes(packet[18..22].try_into().unwrap()),
        sync_token,
    })
}

fn decode_time_sync_request(packet: &[u8]) -> Result<TimeSyncRequest, DecodeError> {
    if packet.len() != TIME_SYNC_REQUEST_SIZE {
        return Err(DecodeError::Length);
    }
    Ok(TimeSyncRequest {
        device_mac: read_mac(&packet[4..10]),
        sequence: u32::from_be_bytes(packet[10..14].try_into().unwrap()),
        device_t1_us: u32::from_be_bytes(packet[14..18].try_into().unwrap()),
    })
}

pub fn encode_time_sync_response(
    request: TimeSyncRequest,
    server_t2_us: u32,
    server_t3_us: u32,
    sync_token: u32,
) -> [u8; TIME_SYNC_RESPONSE_SIZE] {
    let mut packet = [0_u8; TIME_SYNC_RESPONSE_SIZE];
    write_header(&mut packet, TYPE_TIME_SYNC_RESPONSE);
    packet[4..10].copy_from_slice(&request.device_mac.octets());
    packet[10..14].copy_from_slice(&request.sequence.to_be_bytes());
    packet[14..18].copy_from_slice(&request.device_t1_us.to_be_bytes());
    packet[18..22].copy_from_slice(&server_t2_us.to_be_bytes());
    packet[22..26].copy_from_slice(&server_t3_us.to_be_bytes());
    packet[26..30].copy_from_slice(&sync_token.to_be_bytes());
    packet
}

fn read_mac(bytes: &[u8]) -> MacAddress {
    MacAddress::from_octets(bytes.try_into().unwrap())
}

fn write_header(packet: &mut [u8], packet_type: u8) {
    packet[0..2].copy_from_slice(&MAGIC.to_be_bytes());
    packet[2] = VERSION;
    packet[3] = packet_type;
}
