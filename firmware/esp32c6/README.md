# ESP32-C6 sensor firmware

This PlatformIO project samples one MT6701 over I²C and sends one UDP sample
datagram per 1 ms sampling tick when Wi-Fi is connected. All nodes use the same
firmware. The Wi-Fi station MAC carried in each packet identifies the node;
the Rust server assigns its logical sensor name from its own configuration.

## Wiring

For the Seeed Studio XIAO ESP32-C6 board, the defaults are:

| XIAO pin | ESP32-C6 GPIO | MT6701 |
|---|---:|---|
| 3V3 | — | VCC/VDD |
| GND | — | GND |
| D4 / SDA | 22 | SDA |
| D5 / SCL | 23 | SCL |

Use a 3.3 V-compatible MT6701 breakout. I²C needs pull-up resistors on SDA
and SCL; use the breakout's pull-ups or add external pull-ups if it has none.
The firmware enables the ESP32-C6 internal pull-ups as a convenience, but they
may be too weak for reliable operation at 400 kHz on some wiring.

## Configure and build

From `firmware/esp32c6`:

1. Run `pio run -t menuconfig`.
2. Open **Pendulum sensor** and set the Wi-Fi SSID/password, the server IPv4
   address and UDP port. Defaults are GPIO22/GPIO23, I²C at 400 kHz, and UDP
   port 5000.
3. Run `pio run` to build, `pio run -t upload` to flash, and
   `pio device monitor` to view logs.

PlatformIO writes the active configuration to `sdkconfig.local`, which is
ignored by Git. The tracked `sdkconfig.seeed_xiao_esp32c6` supplies the shared
base settings. After the local file is created, you can edit the values
directly, for example:

```ini
CONFIG_SENSOR_SERVER_IPV4="192.168.1.20"
CONFIG_SENSOR_SERVER_PORT=5000
CONFIG_SENSOR_WIFI_SSID="your-network"
CONFIG_SENSOR_WIFI_PASSWORD="your-password"
```

Run `pio run` after changing the file. With an empty SSID, the node still
samples and logs but does not start Wi-Fi or send UDP packets.

The serial log prints the station MAC. Add that MAC to the Rust server's device
mapping configuration to assign the node a logical role. The firmware does
not contain a per-node numeric ID and does not receive configuration from the
server.

## Implementation notes

- The ESP high-resolution timer notifies a dedicated sampling task every
  1,000 µs. The task reads MT6701 registers `0x03` then `0x04` over I²C.
- A bounded queue separates acquisition from the nonblocking UDP sender. If
  Wi-Fi is down, the queue is cleared and acquisition continues. Sequence
  numbers therefore expose outages and local transmit drops to the receiver.
- Sampling is a 1 kHz target. Wi-Fi scheduling, I²C errors, and queue pressure
  can reduce the delivered rate; UDP does not retransmit lost packets.
- The wire format is documented in [`protocol/protocol.md`](../../protocol/protocol.md).
