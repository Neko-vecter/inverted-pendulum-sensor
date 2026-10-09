# Rust sensor server

This MVP server listens for the ESP32-C6 version 1 UDP sample protocol, maps
the packet's station MAC address to a logical sensor name, and reports each
sensor's latest reading and reception statistics. The wire format is defined
in [`../protocol/protocol.md`](../protocol/protocol.md).

## Configure

Copy the example file to a local configuration file:

```powershell
Copy-Item config.example.toml config.toml
```

Edit `config.toml` and set `server.bind_address` to the server's IPv4 address
and UDP port. Use `0.0.0.0:5000` to listen on every local IPv4 interface, or
bind to one specific interface such as `192.168.1.20:5000`. Set each sensor's
MAC address to the station MAC printed by its firmware, then choose a logical
name for that sensor:

```toml
[server]
bind_address = "0.0.0.0:5000"
stats_interval_secs = 1

[devices]
"AA:BB:CC:DD:EE:01" = "pendulum_base"
"AA:BB:CC:DD:EE:02" = "pendulum_arm"
"AA:BB:CC:DD:EE:03" = "pendulum_wheel"
```

`config.toml` is ignored by Git. The example remains tracked, so local IPs,
ports, and device MAC mappings do not have to be committed. Do not edit the
example with machine-specific values.

## Run

From this directory:

```powershell
cargo run --release -- --config config.toml
```

The default config path is `config.toml`, so `cargo run --release` also works.
The process prints one summary per configured device at the selected interval.
Use Ctrl+C to stop it. The server has no control or data API yet; a future UDS
interface is outside the current MVP scope.

The host firewall must allow incoming UDP traffic on the configured port, and
each ESP32 must target the server interface address and the same UDP port.

## Current statistics

- Received rate and total well-formed packets per mapped sensor.
- Latest angle, firmware sequence, local timestamp, and packet age.
- Sequence gaps, estimated lost samples, duplicates, out-of-order packets,
  and timestamp-detected restarts.
- Sensor I2C errors, malformed datagrams, unknown MAC addresses, and socket
  receive errors.

Sequence-based loss is an estimate from received sequence numbers. It cannot
distinguish network loss from samples omitted by the firmware before sending.
