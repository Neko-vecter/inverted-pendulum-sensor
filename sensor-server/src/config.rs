use crate::protocol::MacAddress;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::net::{SocketAddr, SocketAddrV4};
use std::path::Path;
use std::time::Duration;

#[derive(Debug)]
pub struct ServerConfig {
    pub bind_address: SocketAddrV4,
    pub stats_interval: Duration,
    pub devices_by_mac: HashMap<MacAddress, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    server: RawServerConfig,
    devices: HashMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServerConfig {
    bind_address: String,
    #[serde(default = "default_stats_interval_secs")]
    stats_interval_secs: u64,
}

const fn default_stats_interval_secs() -> u64 {
    1
}

pub fn load(path: &Path) -> Result<ServerConfig, String> {
    let contents = fs::read_to_string(path)
        .map_err(|error| format!("cannot read config {}: {error}", path.display()))?;
    let raw: RawConfig = toml::from_str(&contents)
        .map_err(|error| format!("invalid TOML in {}: {error}", path.display()))?;

    let bind_address = match raw
        .server
        .bind_address
        .parse::<SocketAddr>()
        .map_err(|error| format!("invalid server.bind_address: {error}"))?
    {
        SocketAddr::V4(address) => address,
        SocketAddr::V6(_) => {
            return Err("server.bind_address must be an IPv4 socket address".to_owned());
        }
    };
    if bind_address.port() == 0 {
        return Err("server.bind_address must use a nonzero UDP port".to_owned());
    }

    if !(1..=3600).contains(&raw.server.stats_interval_secs) {
        return Err("server.stats_interval_secs must be between 1 and 3600".to_owned());
    }
    if raw.devices.is_empty() {
        return Err("at least one [devices] MAC-to-name mapping is required".to_owned());
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
        bind_address,
        stats_interval: Duration::from_secs(raw.server.stats_interval_secs),
        devices_by_mac,
    })
}
