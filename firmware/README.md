# ESP32 MT6701 sensor firmware

This PlatformIO project supports the Seeed Studio XIAO ESP32-C6, XIAO ESP32-C3, and XIAO ESP32-C5. It samples one MT6701 over I2C and sends one version 3 sample per 1 ms sampling tick over ESP-NOW by default, with optional Wi-Fi / UDP. V3 samples use a 32-bit wrapping microsecond counter and a compact sync token; all boards use the same firmware. At startup, each sensor registers its Wi-Fi station MAC and receives a server-assigned 2-byte ID. Later packets identify the node with that ID.

## Wiring

The default MT6701 wiring uses D4/D5 on all three boards. The GPIO numbers differ:

| XIAO pin | MT6701  | ESP32-C6 GPIO | ESP32-C3 GPIO | ESP32-C5 GPIO |
| -------- | ------- | ------------: | ------------: | ------------: |
| 3V3      | VCC/VDD |             - |             - |             - |
| GND      | GND     |             - |             - |             - |
| D4 / SDA | SDA     |            22 |             6 |            23 |
| D5 / SCL | SCL     |            23 |             7 |            24 |

Use a 3.3 V-compatible MT6701 breakout. I2C needs pull-up resistors on SDA and SCL; use the breakout's pull-ups or add external pull-ups if it has none. The firmware enables the ESP32 internal pull-ups as a convenience, but they may be too weak for reliable operation at 400 kHz on some wiring.

## Transport configuration and build

ESP-NOW is the default transport on all three boards. Each project builds on its own; the sensor firmware has no source dependency on the gateway or server.

Before the first build, copy the example file:

```sh
cp src/config.example.h src/config.h
```

The local `config.h` is ignored by Git, so it can hold machine-specific
settings. Edit it to select the transport and set its options:

```c
#define SENSOR_USE_ESPNOW 1
#define SENSOR_ESPNOW_CHANNEL 1
```

The channel must match the gateway (default 1). Samples and clock-sync requests use unencrypted ESP-NOW broadcast, with one active gateway/server per channel. No SSID or access point is needed. The Wi-Fi radio driver remains enabled, while AP connection, reconnection, IPv4 configuration and UDP application code are excluded from the build.

To select Wi-Fi / UDP instead:

For Wi-Fi / UDP, set `SENSOR_USE_ESPNOW` to `0` and edit `SENSOR_WIFI_SSID`,
`SENSOR_WIFI_PASSWORD`, `SENSOR_SERVER_IPV4`, and `SENSOR_SERVER_PORT` in the
same header. CMake watches `src/config.h` and selects the matching transport
sources when it changes; no `menuconfig` or sdkconfig regeneration is needed.

An empty SSID in UDP mode keeps sampling active without networking. WPA2
passwords must be 8-63 bytes or a 64-digit hexadecimal PSK; an empty password
selects an open network. Keep private Wi-Fi credentials in the ignored local
`config.h` file.

```sh
pio run -e seeed_xiao_esp32c6
pio run -e seeed_xiao_esp32c6 -t upload
pio device monitor -e seeed_xiao_esp32c6
```

Use `seeed_xiao_esp32c3` or `seeed_xiao_esp32c5` for the other boards. I2C pins and clock remain in each environment's build flags. C5 builds select 8 MB flash and use the included `default_8MB.csv` partition table.

The firmware logs its station MAC. Map this MAC to a logical name in the server configuration. With ESP-NOW, flash the gateway separately and configure the server with `server.mode = "espnow"` and `[gateway] serial = "path"`.

## Implementation notes

- The high-resolution timer notifies the sampling task every 1,000 us. Acquisition reads MT6701 registers `0x03` and `0x04` over I2C.
- A bounded queue separates acquisition from transmission. Sequence gaps expose missed timer slots, queue overflow and radio/network losses.
- ESP-NOW sends are submitted immediately without waiting for the previous send callback; driver submission failures are counted. Receive callbacks only copy registration and sync replies to a bounded queue; they never block.
- V3 sensors register at startup with their station MAC and receive a server-assigned 2-byte ID in the range `0x0000` through `0x00FF`. Samples are 20 bytes; time-sync requests, responses and reports are 14, 26 and 22 bytes. Subsequent packets carry only the assigned ID. The sensor returns `t4` in the completion report; the server computes and stores the offset and RTT. ESP-NOW responses travel through the gateway serial port.
- Sampling targets 1 kHz. Delivered rates depend on radio contention, I2C and serial throughput; broadcast submission does not acknowledge remote reception and samples are not retransmitted.
- Five-second logs report sampling, queue drops, offline reads, transport submission failures and clock sync. ESP-NOW considers the radio ready after initialization even if no gateway is present.
- The timestamp precedes the MT6701 I2C read; latency does not subtract the sensor's internal measurement delay.

Protocol details are documented in `protocol/protocol.md` in the system documentation.
