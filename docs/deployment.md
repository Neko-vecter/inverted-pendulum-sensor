# Deployment

## ESP-NOW default setup

Build and upload each project from its own directory. The sensor defaults to `CONFIG_SENSOR_ESPNOW=y` and channel 1. Build `esp-now-gateway` with `pio run -e seeed_xiao_esp32c6`, upload it, and connect its native USB port to the host. Match the gateway and sensor channels. Keep binary USB data separate from the UART0 console; the gateway defaults disable the secondary USB console.

Set the server configuration:

```toml
[server]
mode = "espnow"
stats_interval_secs = 1

[gateway]
serial = "/dev/ttyACM0" # Prefer /dev/serial/by-id/...; Windows: "COM7".
```

Keep the `[devices]` sensor MAC mappings from the example. For direct Wi-Fi / UDP, set `server.mode = "udp"`, configure `server.bind_address`, and disable `CONFIG_SENSOR_ESPNOW` in the sensor sdkconfig before setting Wi-Fi credentials.

## server

### dev

copy example config

```sh
cp config.example.toml config.toml
```

run server

```sh
cargo run --release -- --config config.toml
```

### build

```sh
cargo build
```

## ESP32 wireless sensor

### Require Package

```sh
python -m pip install platformio
```

### Config sdkconfig

copy example sdkconfig

```shell
cp sdkconfig.example.local sdkconfig.local
```

### Build

#### xiao esp32c6

```sh
pio run -e seeed_xiao_esp32c6
```

#### xiao esp32c3

```sh
pio run -e seeed_xiao_esp32c3
```

#### xiao esp32c5

```sh
pio run -e seeed_xiao_esp32c5
```
