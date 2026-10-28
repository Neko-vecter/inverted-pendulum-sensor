use crate::config::TransportConfig;
use crate::serial_frame::{self, Decoder};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};
use tracing::info;

const READ_TIMEOUT: Duration = Duration::from_millis(100);

pub struct ReceivedPacket {
    pub data: Vec<u8>,
    pub sender: Option<SocketAddr>,
    pub received_at: Instant,
}

pub enum Transport {
    Udp(UdpSocket),
    Serial {
        port: Box<dyn serialport::SerialPort>,
        decoder: Decoder,
        pending: VecDeque<ReceivedPacket>,
    },
}

impl Transport {
    pub fn open(config: &TransportConfig) -> Result<Self, String> {
        match config {
            TransportConfig::Udp(address) => {
                let socket = UdpSocket::bind(address)
                    .map_err(|e| format!("cannot bind UDP {address}: {e}"))?;
                socket
                    .set_read_timeout(Some(READ_TIMEOUT))
                    .map_err(|e| e.to_string())?;
                info!(%address, "listening for UDP sensor packets");
                Ok(Self::Udp(socket))
            }
            TransportConfig::EspNowSerial { port, baud_rate } => {
                let serial = serialport::new(port, *baud_rate)
                    .timeout(READ_TIMEOUT)
                    .flow_control(serialport::FlowControl::None)
                    .open()
                    .map_err(|e| format!("cannot open ESP-NOW serial port {port}: {e}"))?;
                info!(%port, baud_rate, "receiving ESP-NOW gateway serial data");
                Ok(Self::Serial {
                    port: serial,
                    decoder: Decoder::default(),
                    pending: VecDeque::new(),
                })
            }
        }
    }

    pub fn receive(&mut self) -> io::Result<Option<ReceivedPacket>> {
        let mut buffer = [0_u8; 2048];
        match self {
            Self::Udp(socket) => {
                let (length, sender) = socket.recv_from(&mut buffer)?;
                Ok(Some(ReceivedPacket {
                    data: buffer[..length].to_vec(),
                    sender: Some(sender),
                    received_at: Instant::now(),
                }))
            }
            Self::Serial {
                port,
                decoder,
                pending,
            } => {
                if let Some(packet) = pending.pop_front() {
                    return Ok(Some(packet));
                }
                let length = port.read(&mut buffer)?;
                if length == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "gateway serial port closed",
                    ));
                }
                let received_at = Instant::now();
                for &byte in &buffer[..length] {
                    if let Some(data) = decoder.push(byte) {
                        pending.push_back(ReceivedPacket {
                            data,
                            sender: None,
                            received_at,
                        });
                    }
                }
                Ok(pending.pop_front())
            }
        }
    }

    pub fn respond(&mut self, packet: &[u8], sender: Option<SocketAddr>) -> io::Result<()> {
        match self {
            Self::Udp(socket) => {
                let sender = sender.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "missing UDP sender")
                })?;
                socket.send_to(packet, sender)?;
                Ok(())
            }
            Self::Serial { port, .. } => port.write_all(&serial_frame::encode(packet)),
        }
    }

    pub fn framing_errors(&self) -> u64 {
        match self {
            Self::Udp(_) => 0,
            Self::Serial { decoder, .. } => decoder.rejected,
        }
    }
}
