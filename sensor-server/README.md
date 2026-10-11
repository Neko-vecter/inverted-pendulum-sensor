# Rust sensor server

This MVP server listens for the 38-byte ESP32 version 2 UDP sample format, maps each packet's station MAC address to a logical sensor name, replies to version 2 time-sync requests, and reports each sensor's latest reading and reception statistics. The earlier version 1 packets are not supported. The wire format is defined in [`../protocol/protocol.md`](../protocol/protocol.md).

## Configure

Copy the example file to a local configuration file:

```shell
cp config.example.toml config.toml
```

Edit `config.toml` and set `server.bind_address` to the server's IPv4 address and UDP port. Use `0.0.0.0:5000` to listen on every local IPv4 interface, or bind to one specific interface such as `192.168.1.20:5000`. Set each sensor's MAC address to the station MAC printed by its firmware, then choose a logical name for that sensor:

```toml
[server]
bind_address = "0.0.0.0:5000"
stats_interval_secs = 1

[devices]
"AA:BB:CC:DD:EE:01" = "sensor_1"
"AA:BB:CC:DD:EE:02" = "sensor_2"
"AA:BB:CC:DD:EE:03" = "sensor_3"
```

`config.toml` is ignored by Git. The example remains tracked, so local IPs, ports, and device MAC mappings do not have to be committed. Do not edit the example with machine-specific values.

## Run

From this directory:

```powershell
cargo run --release -- --config config.toml
```

The default config path is `config.toml`, so `cargo run --release` also works. The process prints one summary per configured device at the selected interval. Use `control + c` to stop it. The server has no control or data API yet; a future UDS interface is outside the current MVP scope.

The host firewall must allow incoming UDP traffic on the configured port, and each ESP32 must target the server interface address and the same UDP port.

## Current statistics

- Received rate and total well-formed packets per mapped sensor.
- Latest angle, firmware sequence, local timestamp, and packet age.
- P50, P75, P90, and P99 sample-to-server receive latency for synchronized version 2 samples, plus sync round-trip time and counts of unsynced or clock-ahead samples.
- Sequence gaps, estimated lost samples, duplicates, out-of-order packets, and timestamp-detected restarts.
- Sensor I2C errors, malformed datagrams, unknown MAC addresses, and socket receive errors.

Sequence-based loss is an estimate from received sequence numbers. It cannot distinguish network loss from samples omitted by the firmware before sending. Latency uses the device timestamp taken immediately before the MT6701 I2C read and a four-timestamp UDP clock-sync exchange. Device and server wire times are 32-bit wrapping microsecond counters; modular differences are unambiguous for intervals below 2^31 microseconds. The server time origin is captured at the start of main. Latency is an estimate affected by round-trip path asymmetry and clock scheduling. The metric does not model or subtract MT6701 internal measurement delay. The server retains at most 50,000 latency samples per device per statistics interval for percentile calculation.
