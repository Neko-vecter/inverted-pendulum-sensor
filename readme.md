# Inverted Pendulum Sensor

A modular wireless angle sensing system designed for double and triple inverted pendulums.

This project uses **ESP32 + ESP-NOW** to transmit pendulum angle measurements through a USB serial gateway to a Linux-based Single Board Computer (SBC) for sensor data collection and processing.

## Features

- Modular design for 2-stage and 3-stage inverted pendulums
- Wireless angle data transmission over ESP-NOW (default), with optional Wi-Fi / UDP
- ESP32-based sensor nodes
- Linux SBC support for data collection and processing

## Hardware Support

| Hardware | Status    |
| -------- | --------- |
| ESP32-C6 | Supported |
| ESP32-C5 | Supported |

## Project Scope

This project focuses exclusively on **angle sensing, wireless data transmission, and data collection**.

Pendulum stabilization and control algorithms are **not included**.

## Project Status

**Early Development**

This project is in its initial development stage. Features, APIs, and hardware support may change as development progresses.

The sensor firmware, ESP-NOW gateway and Rust server are independently buildable projects. `protocol/` contains their communication contract as documentation only. The default path is sensor -> ESP-NOW -> gateway -> USB serial -> server. The server selects `mode = "espnow"` or `mode = "udp"` under `[server]`; `[gateway] serial = "path"` selects the serial device.

An async Python WebSocket client that keeps the latest readings in `client.sensors` is available in [`lib-sensor/`](lib-sensor/README.md).
