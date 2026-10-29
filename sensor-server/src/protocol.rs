use std::fmt;
use std::str::FromStr;

pub const SAMPLE_PACKET_SIZE: usize = 20;
pub const TIME_SYNC_REQUEST_SIZE: usize = 14;
pub const TIME_SYNC_RESPONSE_SIZE: usize = 26;
pub const TIME_SYNC_REPORT_SIZE: usize = 22;
pub const REGISTRATION_REQUEST_SIZE: usize = 14;
pub const REGISTRATION_RESPONSE_SIZE: usize = 16;
pub const SENSOR_CONTROL_SIZE: usize = 15;
pub const SENSOR_CONTROL_ACK_SIZE: usize = 15;
const MAGIC: u16 = 0x4950;
const VERSION: u8 = 3;
const TYPE_SAMPLE: u8 = 0x01;
const TYPE_TIME_SYNC_REQUEST: u8 = 0x02;
const TYPE_TIME_SYNC_RESPONSE: u8 = 0x03;
const TYPE_TIME_SYNC_REPORT: u8 = 0x04;
const TYPE_REGISTRATION_REQUEST: u8 = 0x05;
const TYPE_REGISTRATION_RESPONSE: u8 = 0x06;
const TYPE_SENSOR_CONTROL: u8 = 0x07;
const TYPE_SENSOR_CONTROL_ACK: u8 = 0x08;
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SensorSample {
    pub device_id: u16,
    pub sequence: u32,
    pub timestamp_us: u32,
    pub raw_angle: u16,
    pub status: u8,
    pub sync_token: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct TimeSyncRequest {
    pub device_id: u16,
    pub sequence: u32,
    pub device_t1_us: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct TimeSyncReport {
    pub device_id: u16,
    pub sequence: u32,
    pub device_t1_us: u32,
    pub device_t4_us: u32,
    pub sync_token: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct RegistrationRequest {
    pub device_mac: MacAddress,
    pub sequence: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SensorControlAck {
    pub device_id: u16,
    pub command: SensorControl,
    pub session_token: u32,
    pub sequence: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SensorControl {
    StartMeasurement,
    StopMeasurement,
}
#[derive(Clone, Copy, Debug)]
pub enum DecodedDatagram {
    Sample(SensorSample),
    TimeSyncRequest(TimeSyncRequest),
    TimeSyncReport(TimeSyncReport),
    RegistrationRequest(RegistrationRequest),
    SensorControlAck(SensorControlAck),
}
impl SensorSample {
    pub const fn angle_is_valid(self) -> bool {
        self.status & STATUS_ANGLE_VALID != 0
    }
    pub const fn has_i2c_error(self) -> bool {
        self.status & STATUS_I2C_ERROR != 0
    }
    pub fn angle_radians(self) -> Option<f64> {
        self.angle_is_valid().then(|| {
            let angle = f64::from(self.raw_angle) * std::f64::consts::TAU / 16_384.0;
            if angle >= std::f64::consts::PI {
                angle - std::f64::consts::TAU
            } else {
                angle
            }
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DecodeError {
    Length,
    Magic,
    Version,
    PacketType,
    DeviceId,
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
            Self::DeviceId => "device id",
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
        TYPE_REGISTRATION_REQUEST => {
            decode_registration_request(packet).map(DecodedDatagram::RegistrationRequest)
        }
        TYPE_SENSOR_CONTROL_ACK => {
            decode_sensor_control_ack(packet).map(DecodedDatagram::SensorControlAck)
        }
        _ => Err(DecodeError::PacketType),
    }
}
fn read_id(packet: &[u8]) -> Result<u16, DecodeError> {
    let id = u16::from_be_bytes(packet[4..6].try_into().unwrap());
    if id > 0x00ff {
        Err(DecodeError::DeviceId)
    } else {
        Ok(id)
    }
}
fn decode_sample(p: &[u8]) -> Result<SensorSample, DecodeError> {
    if p.len() != SAMPLE_PACKET_SIZE {
        return Err(DecodeError::Length);
    }
    let id = read_id(p)?;
    let angle_state = u16::from_be_bytes(p[14..16].try_into().unwrap());
    let (status, raw_angle) = match angle_state >> 14 {
        0 => (STATUS_ANGLE_VALID, angle_state & MAX_ANGLE),
        1 if angle_state & MAX_ANGLE == 0 => (STATUS_I2C_ERROR, 0),
        1 => return Err(DecodeError::Angle),
        _ => return Err(DecodeError::Status),
    };
    let sync_token = u32::from_be_bytes(p[16..20].try_into().unwrap());
    Ok(SensorSample {
        device_id: id,
        sequence: u32::from_be_bytes(p[6..10].try_into().unwrap()),
        timestamp_us: u32::from_be_bytes(p[10..14].try_into().unwrap()),
        raw_angle,
        status,
        sync_token,
    })
}
fn decode_time_sync_request(p: &[u8]) -> Result<TimeSyncRequest, DecodeError> {
    if p.len() != TIME_SYNC_REQUEST_SIZE {
        return Err(DecodeError::Length);
    }
    Ok(TimeSyncRequest {
        device_id: read_id(p)?,
        sequence: u32::from_be_bytes(p[6..10].try_into().unwrap()),
        device_t1_us: u32::from_be_bytes(p[10..14].try_into().unwrap()),
    })
}
fn decode_time_sync_report(p: &[u8]) -> Result<TimeSyncReport, DecodeError> {
    if p.len() != TIME_SYNC_REPORT_SIZE {
        return Err(DecodeError::Length);
    }
    let sync_token = u32::from_be_bytes(p[18..22].try_into().unwrap());
    if sync_token == 0 {
        return Err(DecodeError::SyncToken);
    }
    Ok(TimeSyncReport {
        device_id: read_id(p)?,
        sequence: u32::from_be_bytes(p[6..10].try_into().unwrap()),
        device_t1_us: u32::from_be_bytes(p[10..14].try_into().unwrap()),
        device_t4_us: u32::from_be_bytes(p[14..18].try_into().unwrap()),
        sync_token,
    })
}
fn decode_registration_request(p: &[u8]) -> Result<RegistrationRequest, DecodeError> {
    if p.len() != REGISTRATION_REQUEST_SIZE {
        return Err(DecodeError::Length);
    }
    Ok(RegistrationRequest {
        device_mac: read_mac(&p[4..10]),
        sequence: u32::from_be_bytes(p[10..14].try_into().unwrap()),
    })
}
fn decode_sensor_control_ack(p: &[u8]) -> Result<SensorControlAck, DecodeError> {
    if p.len() != SENSOR_CONTROL_ACK_SIZE {
        return Err(DecodeError::Length);
    }
    let command = match p[6] {
        1 => SensorControl::StartMeasurement,
        2 => SensorControl::StopMeasurement,
        _ => return Err(DecodeError::PacketType),
    };
    Ok(SensorControlAck {
        device_id: read_id(p)?,
        command,
        session_token: u32::from_be_bytes(p[7..11].try_into().unwrap()),
        sequence: u32::from_be_bytes(p[11..15].try_into().unwrap()),
    })
}
pub fn encode_time_sync_response(
    r: TimeSyncRequest,
    t2: u32,
    t3: u32,
    token: u32,
) -> [u8; TIME_SYNC_RESPONSE_SIZE] {
    let mut p = [0; TIME_SYNC_RESPONSE_SIZE];
    write_header(&mut p, TYPE_TIME_SYNC_RESPONSE);
    p[4..6].copy_from_slice(&r.device_id.to_be_bytes());
    p[6..10].copy_from_slice(&r.sequence.to_be_bytes());
    p[10..14].copy_from_slice(&r.device_t1_us.to_be_bytes());
    p[14..18].copy_from_slice(&t2.to_be_bytes());
    p[18..22].copy_from_slice(&t3.to_be_bytes());
    p[22..26].copy_from_slice(&token.to_be_bytes());
    p
}
pub fn encode_registration_response(
    r: RegistrationRequest,
    id: u16,
) -> [u8; REGISTRATION_RESPONSE_SIZE] {
    let mut p = [0; REGISTRATION_RESPONSE_SIZE];
    write_header(&mut p, TYPE_REGISTRATION_RESPONSE);
    p[4..10].copy_from_slice(&r.device_mac.octets());
    p[10..14].copy_from_slice(&r.sequence.to_be_bytes());
    p[14..16].copy_from_slice(&id.to_be_bytes());
    p
}
pub fn encode_sensor_control(
    device_id: u16,
    command: SensorControl,
    session_token: u32,
    sequence: u32,
) -> [u8; SENSOR_CONTROL_SIZE] {
    let mut packet = [0; SENSOR_CONTROL_SIZE];
    write_header(&mut packet, TYPE_SENSOR_CONTROL);
    packet[4..6].copy_from_slice(&device_id.to_be_bytes());
    packet[6] = match command {
        SensorControl::StartMeasurement => 1,
        SensorControl::StopMeasurement => 2,
    };
    packet[7..11].copy_from_slice(&session_token.to_be_bytes());
    packet[11..15].copy_from_slice(&sequence.to_be_bytes());
    packet
}
#[cfg(test)]
pub fn encode_sensor_control_ack(ack: SensorControlAck) -> [u8; SENSOR_CONTROL_ACK_SIZE] {
    let mut packet = [0; SENSOR_CONTROL_ACK_SIZE];
    write_header(&mut packet, TYPE_SENSOR_CONTROL_ACK);
    packet[4..6].copy_from_slice(&ack.device_id.to_be_bytes());
    packet[6] = match ack.command {
        SensorControl::StartMeasurement => 1,
        SensorControl::StopMeasurement => 2,
    };
    packet[7..11].copy_from_slice(&ack.session_token.to_be_bytes());
    packet[11..15].copy_from_slice(&ack.sequence.to_be_bytes());
    packet
}
fn read_mac(b: &[u8]) -> MacAddress {
    MacAddress::from_octets(b.try_into().unwrap())
}
fn write_header(p: &mut [u8], ty: u8) {
    p[0..2].copy_from_slice(&MAGIC.to_be_bytes());
    p[2] = VERSION;
    p[3] = ty;
}
