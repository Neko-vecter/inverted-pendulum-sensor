# Rust sensor server

This server receives the 24-byte ESP32 version 3 sample format over ESP-NOW gateway serial or UDP, maps each packet's station MAC address to a logical sensor name, handles V3 time-sync requests and completion reports, and reports each sensor's latest reading and reception statistics. Only version 3 packets are supported. The wire format is defined in [`../protocol/protocol.md`](../protocol/protocol.md).

## Configure

Copy the example file to a local configuration file:

```sh
cp config.example.toml config.toml
```

Select the transport with `server.mode`: `"espnow"` for gateway serial, or `"udp"` for direct Wi-Fi. The example defaults to ESP-NOW. Old configurations that omit `mode` continue to use UDP.

```toml
[server]
mode = "espnow"
stats_interval_secs = 1
bind_address = "0.0.0.0:5000" # Used only in UDP mode.

[gateway]
serial = "/dev/ttyACM0" # Or a stable /dev/serial/by-id/... path, or "COM7".
baud_rate = 921600     # Optional; default 921600.

[devices]
"AA:BB:CC:DD:EE:01" = "sensor_1"
"AA:BB:CC:DD:EE:02" = "sensor_2"
"AA:BB:CC:DD:EE:03" = "sensor_3"
```

In ESP-NOW mode, `[gateway] serial = "path"` is required; `bind_address` is optional and no UDP socket is opened. Use the OS serial device path or COM port identifier. The native USB gateway ignores the physical baud rate; an external UART adapter must match `gateway.baud_rate`. The port uses 8N1 and no flow control. Linux users need access to the serial device, commonly through the `dialout` group. If the port disconnects or becomes unreadable, the server exits with an error; reconnect and restart it.

In UDP mode, `server.bind_address` is required and `[gateway]` is unused. Use `0.0.0.0:5000` to listen on every IPv4 interface, or select a specific local IPv4 address. Device mappings always use the sensor station MAC printed by its firmware.

`config.toml` is ignored by Git. The example remains tracked, so local IPs, ports, and device MAC mappings do not have to be committed. Do not edit the example with machine-specific values.

## Run

From this directory:

```sh
cargo run --release -- --config config.toml
```

The default config path is `config.toml`, so `cargo run --release` also works. The process prints one summary per configured device at the selected interval. Use `control + c` to stop it. The server has no control or data API yet; a future UDS interface is outside the current MVP scope.

For UDP mode, allow incoming traffic on the configured port and configure each node with the server IPv4 address and port. For ESP-NOW mode, match the sensor and gateway radio channels (default 1), connect the gateway USB port, and keep other programs off that serial port.

## Current statistics

- Received rate and total well-formed packets per mapped sensor.
- Latest angle, firmware sequence, local timestamp, and packet age.
- P50, P75, P90, and P99 sample-to-server receive latency for synchronized version 3 samples, plus sync round-trip time and counts of unsynced or clock-ahead samples.
- Sequence gaps, estimated lost samples, duplicates, out-of-order packets, and timestamp-detected restarts.
- Sensor I2C errors, malformed packets, unknown MAC addresses, receive errors, and serial framing errors.

Sequence-based loss is an estimate from received sequence numbers. It cannot distinguish network loss from samples omitted by the firmware before sending. Latency uses the device timestamp taken immediately before the MT6701 I2C read and a four-timestamp clock-sync exchange over the selected transport. ESP-NOW latency includes gateway buffering and the serial hop; path asymmetry can affect the estimate. Device and server wire times are 32-bit wrapping microsecond counters; modular differences are unambiguous for intervals below 2^31 microseconds. The server time origin is captured at the start of main. Latency is an estimate affected by round-trip path asymmetry and clock scheduling. The metric does not model or subtract MT6701 internal measurement delay. The server retains at most 50,000 latency samples per device per statistics interval for percentile calculation.

## build

```sh
cargo build
```

## Serial protocol

The gateway and server independently implement the serial contract documented in `../protocol/protocol.md`: zero-delimited COBS frames and CRC-16/CCITT-FALSE around version 3 packets. The stream decoder accepts fragmented and concatenated frames, rejects CRC failures and oversized input, and resumes at the next delimiter. Time-sync responses use the same framing in the reverse direction. Framing rejection totals are printed separately from packet validation errors.

```sh
cargo test
```
