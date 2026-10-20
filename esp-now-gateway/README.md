# ESP-NOW serial gateway

Standalone ESP-IDF / PlatformIO firmware for the Seeed XIAO ESP32-C6. It receives version 2 sensor samples and time-sync requests over ESP-NOW, forwards them to the host serial port, and broadcasts host time-sync responses back to the nodes. No sensor firmware source files are required to build it.

## Build and configure

Run from this directory:

```sh
pio run -e seeed_xiao_esp32c6
pio run -e seeed_xiao_esp32c6 -t upload
pio run -e seeed_xiao_esp32c6 -t menuconfig
```

Under **ESP-NOW gateway**, select the channel (default 1, matching every sensor) and host serial interface:

- **Native USB Serial/JTAG**, the default: connect the XIAO USB port to the server. Linux usually exposes `/dev/ttyACM0`; prefer a stable `/dev/serial/by-id/...` path. Windows uses a COM port. The baud setting does not limit native USB throughput.
- **UART1**: use an external 3.3 V USB-UART adapter, common ground, gateway TX GPIO16 to adapter RX, gateway RX GPIO17 to adapter TX. Pins are configurable; default speed is 921600 baud, 8N1, no flow control. For three nodes targeting 1 kHz, UART bandwidth at 921600 is insufficient; use native USB or a higher supported baud rate.

The native USB port carries binary data. Keep the primary console on UART0 and disable the secondary USB console, as specified in `sdkconfig.defaults`. An existing generated sdkconfig overrides defaults: change these console settings in menuconfig before building. A build guard prevents USB console output from corrupting data. Logs and five-second forwarding/error counters are available on UART0 at 115200 baud. Do not open a serial monitor on the data port while the server is using it.

Configure the server:

```toml
[server]
mode = "espnow"
stats_interval_secs = 1

[gateway]
serial = "/dev/serial/by-id/your-gateway" # Or "COM7" on Windows.
baud_rate = 921600

[devices]
"AA:BB:CC:DD:EE:01" = "sensor_1"
```

Device mappings use the sensor station MAC, not the gateway MAC. ESP-NOW uses unencrypted broadcast; use one active gateway/server per channel. The gateway checks each incoming sensor packet's embedded MAC against its radio source MAC before forwarding it.

## Serial framing

Each message is `0x00 || COBS(payload || CRC16_BE) || 0x00`. CRC is CRC-16/CCITT-FALSE: polynomial `0x1021`, initial value `0xFFFF`, no reflection, no final XOR; it covers only the payload. Payload bytes are the unchanged version 2 wire packet. Samples are 38 bytes, requests 18 bytes and responses 34 bytes. The maximum serial frame is 43 bytes. Empty delimiters are ignored; invalid, oversized or corrupt frames are discarded, with recovery at the next zero delimiter.

Radio callbacks enqueue into a bounded queue and never write serial. Separate tasks handle each serial direction. Queues and USB buffers are bounded; when the host cannot keep up, packets are dropped and the firmware logs counters. A successful serial write or radio submission does not confirm end-to-end delivery.

The full packet contract is in the system's documentation-only `protocol/protocol.md`.
