# Deployment

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
