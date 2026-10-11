# ESP32 MT6701 sensor firmware

This PlatformIO project supports the Seeed Studio XIAO ESP32-C6 and XIAO ESP32-C5. It samples one MT6701 over I2C and sends one UDP sample datagram per 1 ms sampling tick when Wi-Fi is connected. Version 2 packets include clock-sync metadata and a 32-bit wrapping microsecond counter; both boards use the same firmware. The Wi-Fi station MAC carried in each packet identifies the node; the Rust server assigns its logical sensor name from its own configuration.

## Wiring

The default MT6701 wiring uses D4/D5 on either board. The GPIO numbers differ:

| XIAO pin | MT6701  | ESP32-C6 GPIO | ESP32-C5 GPIO |
| -------- | ------- | ------------: | ------------: |
| 3V3      | VCC/VDD |             - |             - |
| GND      | GND     |             - |             - |
| D4 / SDA | SDA     |            22 |            23 |
| D5 / SCL | SCL     |            23 |            24 |

Use a 3.3 V-compatible MT6701 breakout. I2C needs pull-up resistors on SDA and SCL; use the breakout's pull-ups or add external pull-ups if it has none. The firmware enables the ESP32 internal pull-ups as a convenience, but they may be too weak for reliable operation at 400 kHz on some wiring.

## Local configuration and build

Both environments share one ignored local configuration for Wi-Fi credentials and the UDP server. From `firmware`, copy the template once:

```shell
cp sdkconfig.example.local sdkconfig.local
```

Edit the `CONFIG_SENSOR_*` entries in `sdkconfig.local` to set Wi-Fi credentials and the UDP server IPv4 address/port. I2C pins and clock are fixed by each PlatformIO environment's compile flags: C6 uses GPIO22/23 at 400 kHz, and C5 uses GPIO23/24 at 400 kHz. CMake selects the C5 board's 8 MB flash setting when building for `esp32c5`; it does not need to be in the local sdkconfig file. `sdkconfig.local` is ignored by Git. A WPA2 passphrase must be 8-63 bytes, or a 64-digit hexadecimal pre-shared key; an empty password selects an open network.

The Seeed C5 board definition selects `default_8MB.csv`; this project includes that partition table in the `firmware` directory. It provides two OTA app slots, SPIFFS storage, and a coredump partition.

The local file is an `SDKCONFIG_DEFAULTS` fragment, not the generated full ESP-IDF configuration. Both PlatformIO environments use it; PlatformIO writes the generated C6 configuration under `.pio/build/seeed_xiao_esp32c6/` and the C5 configuration under `.pio/build/seeed-xiao-esp32-c5/`. Clean and rebuild the matching environment after editing the shared local file so ESP-IDF applies the new values:

```shell
pio run -e seeed_xiao_esp32c6 -t clean
pio run -e seeed_xiao_esp32c6
pio run -e seeed_xiao_esp32c6 -t upload
pio device monitor -e seeed_xiao_esp32c6
```

Use `seeed-xiao-esp32-c5` instead of `seeed_xiao_esp32c6` for the C5. `pio run -t menuconfig` edits the generated full configuration; keep local overrides in the corresponding ignored file. With an empty SSID, the node still samples and logs but does not start Wi-Fi or send UDP packets.

If Wi-Fi associates with the AP but repeatedly disconnects with reason `15`, ESP-IDF reports a four-way handshake timeout. Check that the local WPA2 passphrase is correct. Editing a local config file does not update an existing generated configuration or an already-flashed image; clean, rebuild, and upload after changing Wi-Fi credentials. The disconnect log includes a short reason description and never prints the passphrase.

In ESP-IDF Wi-Fi logs, `authmode:0x3` means WPA2-PSK. The cipher value `0x3` means TKIP, which is deprecated; configure the AP for WPA2-Personal with AES/CCMP where possible.

At startup, the firmware prints the compiled UDP destination. It sends a four-timestamp clock-sync request about once per second and includes the estimated modular server clock offset and sync round-trip time in each version 2 sample. The periodic `send errors` count is for sample `sendto` failures; `sync send errors` counts time-sync request failures. Neither confirms remote delivery. Use server sequence gaps to observe samples not received by the server.

The serial log prints the station MAC. Add that MAC to the Rust server's device mapping configuration to assign the node a logical role. The firmware does not contain a per-node numeric ID and does not receive configuration from the server. If the station MAC cannot be read, Wi-Fi/UDP are disabled so the node cannot send samples with an invalid all-zero device identity; sensor sampling can still run.

## Implementation notes

- The ESP high-resolution timer notifies a dedicated sampling task every 1,000 us. The task reads MT6701 registers `0x03` then `0x04` over I2C.
- A bounded queue separates acquisition from the nonblocking UDP sender. If Wi-Fi is down, the queue is cleared and acquisition continues. Sequence numbers therefore expose outages and local transmit drops to the receiver.
- The sample timestamp is taken immediately before the MT6701 I2C read. Latency starts at this timestamp; firmware does not model or subtract MT6701 internal measurement delay. Clock offset is refreshed through the server's UDP time-sync exchange.
- The sampler reports an estimated rate and counts for I2C errors, missed timer slots, full-queue drops, and samples taken while Wi-Fi is unavailable every five seconds.
- Sampling is a 1 kHz target. Wi-Fi scheduling, I2C errors, and queue pressure can reduce the delivered rate; UDP does not retransmit lost packets.
- The wire format is documented in [`protocol/protocol.md`](../protocol/protocol.md).
