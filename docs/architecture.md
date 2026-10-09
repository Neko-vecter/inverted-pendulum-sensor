# Sensor System Architecture

## System Overview

The system consists of three ESP32-C6 sensor nodes, each connected to an MT6701 magnetic angle sensor. Each node samples raw angle data at a target frequency of 1000 Hz and transmits it to a centralized Rust Sensor Server over Wi-Fi using UDP.

```mermaid
flowchart TB
    subgraph Sensors["Sensor Nodes"]
        direction LR
        M1["MT6701 #1"] --> E1["ESP32-C6 #1"]
        M2["MT6701 #2"] --> E2["ESP32-C6 #2"]
        M3["MT6701 #3"] --> E3["ESP32-C6 #3"]
    end

    subgraph Network["Wi-Fi Network / UDP"]
        AP["Wi-Fi Access Point"]
    end

    subgraph Server["Rust Sensor Server"]
        UDP["UDP Receiver :5000"]
        Decoder["Packet Decoder"]
        Manager["Device State Manager"]
        Cache["Latest Sample Cache"]
        Stats["Packet & Frequency Statistics"]

        UDP --> Decoder
        Decoder --> Manager
        Manager --> Cache
        Manager --> Stats
    end

    E1 -->|"UDP / 1000 Hz"| AP
    E2 -->|"UDP / 1000 Hz"| AP
    E3 -->|"UDP / 1000 Hz"| AP

    AP --> UDP
```

## ESP32-C6 Firmware Architecture

Each ESP32-C6 runs the same firmware, with a unique `device_id` identifying its sensor node.

The firmware is responsible for sensor sampling, packet encoding, Wi-Fi connectivity, and UDP transmission.

```mermaid
flowchart TD
    Sensor["MT6701 Sensor"]
    Driver["Sensor Driver"]
    Sampler["1 kHz Sampling Task"]
    Encoder["16-byte Packet Encoder"]
    Sender["UDP Sender"]
    WiFi["Wi-Fi STA"]

    Sensor --> Driver
    Driver --> Sampler
    Sampler --> Encoder
    Encoder --> Sender
    Sender --> WiFi
```

### Responsibilities

- **Sensor Driver:** Reads the raw 14-bit angle value from the MT6701.
- **Sampling Task:** Performs periodic sampling at a target rate of 1000 Hz.
- **Packet Encoder:** Encodes each sample into a fixed 16-byte binary packet.
- **UDP Sender:** Sends one UDP datagram per sample.
- **Wi-Fi Manager:** Establishes and maintains network connectivity, including reconnection.

## Rust Sensor Server Architecture

The Rust Sensor Server receives UDP packets from all three ESP32-C6 nodes and maintains their latest sensor states independently.

```mermaid
flowchart TD
    UDP["UDP Socket :5000"]
    Decoder["Protocol Decoder"]
    Validate{"Valid Packet?"}
    Router["Device ID Router"]

    S1["Sensor #1 State"]
    S2["Sensor #2 State"]
    S3["Sensor #3 State"]

    Stats["Reception Statistics"]

    UDP --> Decoder
    Decoder --> Validate

    Validate -->|Yes| Router
    Validate -->|No| Drop["Discard Packet"]

    Router --> S1
    Router --> S2
    Router --> S3

    S1 --> Stats
    S2 --> Stats
    S3 --> Stats
```

### Responsibilities

- **UDP Receiver:** Listens on UDP port `5000`.
- **Protocol Decoder:** Parses the fixed-length binary packets.
- **Packet Validator:** Validates packet length, magic number, protocol version, and device ID.
- **Device State Manager:** Maintains the latest sample for each sensor node.
- **Statistics:** Tracks packet reception rate, sequence gaps, and packet loss.

## Communication Protocol

### Transport Configuration

| Parameter | Value |
|---|---|
| Transport | UDP / IPv4 |
| Network | Wi-Fi |
| Server Port | 5000 |
| Sampling Rate | 1000 Hz per node (target) |
| Transmission Mode | One packet per sample |
| Packet Size | 16 bytes |
| Byte Order | Big Endian |
| Retransmission | Disabled |

### Packet Format

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | `magic` | uint16 | 2 bytes |
| 2 | `version` | uint8 | 1 byte |
| 3 | `device_id` | uint8 | 1 byte |
| 4 | `sequence` | uint32 | 4 bytes |
| 8 | `timestamp_us` | uint32 | 4 bytes |
| 12 | `raw_angle` | uint16 | 2 bytes |
| 14 | `status` | uint8 | 1 byte |
| 15 | `reserved` | uint8 | 1 byte |

**Total packet size: 16 bytes.**

The `timestamp_us` field represents the ESP32's local monotonic time in microseconds. Clocks are not synchronized across nodes in the MVP.

## Device Identification

| Device | Device ID | Sensor |
|---|---|---|
| ESP32-C6 #1 | 1 | MT6701 #1 |
| ESP32-C6 #2 | 2 | MT6701 #2 |
| ESP32-C6 #3 | 3 | MT6701 #3 |

All three devices share the same firmware codebase, with different device configurations.

## 6. MVP Scope

### Included

- MT6701 raw angle acquisition.
- 1000 Hz target sampling rate per node.
- Wi-Fi connectivity and automatic reconnection.
- UDP packet transmission.
- Rust UDP packet reception and validation.
- Independent state management for three sensor nodes.
- Packet reception statistics and sequence-based loss detection.

### Excluded

- Remote device configuration.
- Remote start/stop commands.
- Cross-device clock synchronization.
- Historical data storage.
- Motor control algorithms.
- Unix Domain Socket (UDS) interface.

## Future Integration

The Sensor Server is designed as an independent process, separate from the motion controller.

In a future development phase, the server will expose sensor data through a Unix Domain Socket (UDS), allowing a Python, Rust, or C++ motion controller to consume the latest sensor states.

```mermaid
flowchart LR
    ESP["3 x ESP32-C6"] -->|"Wi-Fi / UDP"| Server["Rust Sensor Service"]
    Server -.->|"Future: UDS"| Controller["Motion Controller"]
    Controller -.-> Planner["Motion Planning"]
    Planner -.-> Motor["Motor Actuation"]
```

The separation ensures that sensor acquisition and motion control can evolve independently.

**MVP Acceptance Criteria:** The Rust Sensor Server can continuously receive raw angle measurements from three physical ESP32-C6 nodes, each targeting 1000 samples per second, while reporting per-device reception rates and packet loss statistics.
