use crate::protocol::MacAddress;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct ServerConfig {
    pub transport: TransportConfig,
    pub stats_interval: Duration,
    pub devices_by_mac: HashMap<MacAddress, String>,
}

#[derive(Debug)]
pub enum TransportConfig {
    Udp(SocketAddrV4),
    EspNowSerial { port: String, baud_rate: u32 },
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ServerMode {
    #[default]
    Udp,
    Espnow,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    server: RawServerConfig,
    gateway: Option<RawGatewayConfig>,
    devices: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServerConfig {
    #[serde(default)]
    mode: ServerMode,
    bind_address: Option<String>,
    #[serde(default = "default_stats_interval_secs")]
    stats_interval_secs: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGatewayConfig {
    serial: String,
    #[serde(default = "default_serial_baud_rate")]
    baud_rate: u32,
}

const fn default_serial_baud_rate() -> u32 {
    921600
}

const fn default_stats_interval_secs() -> u64 {
    1
}

pub fn load(path: &Path) -> Result<ServerConfig, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read config {}: {error}", path.display()))?;
    let raw: RawConfig = toml::from_str(&contents)
        .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;

    validate(raw)
}

fn validate(raw: RawConfig) -> Result<ServerConfig, String> {
    let transport = match raw.server.mode {
        ServerMode::Espnow => {
            let gateway = raw
                .gateway
                .ok_or("server.mode = 'espnow' requires [gateway] with serial = 'path'")?;
            let port = gateway.serial.trim();
            if port.is_empty() {
                return Err("gateway.serial must be a nonempty serial device path or ID".to_owned());
            }
            if !(115200..=3000000).contains(&gateway.baud_rate) {
                return Err("gateway.baud_rate must be between 115200 and 3000000".to_owned());
            }
            TransportConfig::EspNowSerial {
                port: port.to_owned(),
                baud_rate: gateway.baud_rate,
            }
        }
        ServerMode::Udp => TransportConfig::Udp(parse_udp_address(
            raw.server
                .bind_address
                .as_deref()
                .ok_or("UDP requires server.bind_address")?,
        )?),
    };

    if !(1..=3600).contains(&raw.server.stats_interval_secs) {
        return Err("server.stats_interval_secs must be between 1 and 3600".to_owned());
    }
    if raw.devices.is_empty() {
        return Err("at least one [devices] MAC-to-name mapping is required".to_owned());
    }
    if raw.devices.len() > 256 {
        return Err("at most 256 devices can be assigned IDs 0x0000 through 0x00FF".to_owned());
    }

    let mut devices_by_mac = HashMap::with_capacity(raw.devices.len());
    let mut logical_names = BTreeSet::new();
    for (mac_text, name) in raw.devices {
        let mac = mac_text
            .parse::<MacAddress>()
            .map_err(|error| format!("invalid device MAC {mac_text:?}: {error}"))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(format!("logical name for {mac} cannot be empty"));
        }
        if !logical_names.insert(name.to_owned()) {
            return Err(format!(
                "logical device name {name:?} is configured more than once"
            ));
        }
        if devices_by_mac.insert(mac, name.to_owned()).is_some() {
            return Err(format!("MAC {mac} is configured more than once"));
        }
    }

    Ok(ServerConfig {
        transport,
        stats_interval: Duration::from_secs(raw.server.stats_interval_secs),
        devices_by_mac,
    })
}

fn parse_udp_address(value: &str) -> Result<SocketAddrV4, String> {
    match value
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid server.bind_address: {error}"))?
    {
        SocketAddr::V4(address) if address.port() != 0 => Ok(address),
        _ => Err("server.bind_address must be IPv4 with a nonzero UDP port".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(server: &str) -> Result<ServerConfig, String> {
        let raw = toml::from_str(&format!(
            "[server]\n{server}\n[devices]\n\"AA:BB:CC:DD:EE:01\" = \"sensor_1\""
        ))
        .map_err(|error| error.to_string())?;
        validate(raw)
    }

    #[test]
    fn legacy_udp_config_remains_supported() {
        assert!(matches!(
            parse("bind_address = '0.0.0.0:5000'").unwrap().transport,
            TransportConfig::Udp(_)
        ));
    }

    #[test]
    fn espnow_mode_requires_gateway_serial() {
        let config = parse("mode = 'espnow'\n[gateway]\nserial = 'COM7'").unwrap();
        assert!(matches!(
            config.transport,
            TransportConfig::EspNowSerial {
                baud_rate: 921600,
                ..
            }
        ));
        assert!(parse("mode = 'espnow'").is_err());
        assert!(parse("mode = 'espnow'\n[gateway]\nserial = ''").is_err());
        assert!(parse("mode = 'espnow'\n[gateway]\nserial = 'COM7'\nbaud_rate = 0").is_err());
        assert!(parse("mode = 'unknown'").is_err());
    }

    #[test]
    fn mode_selects_transport_even_with_both_settings() {
        let config = parse("mode = 'espnow'\nbind_address = 'ignored in espnow mode'\n[gateway]\nserial = '/dev/serial/by-id/usb-gateway'\nbaud_rate = 1000000").unwrap();
        assert!(
            matches!(config.transport, TransportConfig::EspNowSerial { port, baud_rate: 1000000 } if port == "/dev/serial/by-id/usb-gateway")
        );
        let config =
            parse("mode = 'udp'\nbind_address = '0.0.0.0:5000'\n[gateway]\nserial = 'COM7'")
                .unwrap();
        assert!(matches!(config.transport, TransportConfig::Udp(_)));
    }
}
