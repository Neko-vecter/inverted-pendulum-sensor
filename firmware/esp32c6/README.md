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

## Local configuration and build

From `firmware/esp32c6`, create the ignored local configuration once:

```shell
cp sdkconfig.example.local sdkconfig.local
```

Edit only the `CONFIG_SENSOR_*` entries in `sdkconfig.local` for Wi-Fi
credentials, server IPv4 address/UDP port, or sensor I²C pins/clock. This file
is ignored by Git. The example shows all project-specific options; their
normal defaults are also defined in `src/Kconfig.projbuild`. Other settings
come from PlatformIO's board configuration and ESP-IDF defaults.

The local file is an `SDKCONFIG_DEFAULTS` fragment, not the generated full
ESP-IDF configuration. PlatformIO writes that full configuration under
`.pio/build/seeed_xiao_esp32c6/`, which is also ignored by Git. After editing
`sdkconfig.local`, run `pio run -t clean` and then `pio run` so the new local
values are applied while ESP-IDF regenerates its full configuration. Use
`pio run -t upload` to flash and `pio device monitor` to view logs.

`pio run -t menuconfig` edits the generated full configuration. To keep local
changes in the small ignored file, edit `sdkconfig.local` as the source of
truth. With an empty SSID, the node still samples and logs but does not start
Wi-Fi or send UDP packets.

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
