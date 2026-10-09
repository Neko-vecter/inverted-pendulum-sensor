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

Each ESP32-C6 runs the same firmware. Its Wi-Fi station MAC is included in each sample packet and identifies the node.

The firmware is responsible for sensor sampling, packet encoding, Wi-Fi connectivity, and UDP transmission.

```mermaid
flowchart TD
    Sensor["MT6701 Sensor"]
    Driver["Sensor Driver"]
    Sampler["1 kHz Sampling Task"]
    Queue["Bounded Sample Queue"]
    Encoder["26-byte Packet Encoder"]
    Sender["Nonblocking UDP Sender"]
    WiFi["Wi-Fi STA"]

    Sensor --> Driver
    Driver --> Sampler
    Sampler --> Queue
    Queue --> Encoder
    Encoder --> Sender
    Sender --> WiFi
```

### Responsibilities

- **Sensor Driver:** Reads the raw 14-bit angle value from the MT6701 over I²C.
- **Sampling Task:** Uses a 1 ms high-resolution timer to target 1000 samples per second.
- **Sample Queue:** Decouples sensor reads from network transmission; samples are dropped when the bounded queue is full or Wi-Fi is disconnected.
- **Packet Encoder:** Encodes each sample into the fixed 26-byte binary format in [`protocol/protocol.md`](../protocol/protocol.md), including the Wi-Fi station MAC.
- **UDP Sender:** Sends one nonblocking UDP datagram per queued sample.
- **Wi-Fi Manager:** Establishes and maintains network connectivity, including reconnection.

## Rust Sensor Server Architecture

The Rust Sensor Server receives UDP packets from all three ESP32-C6 nodes and maintains their latest sensor states independently.

```mermaid
flowchart TD
    UDP["UDP Socket :5000"]
    Decoder["Protocol Decoder"]
    Validate{"Valid Packet?"}
    Router["MAC-to-Logical-Device Mapping"]

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
- **Packet Validator:** Validates packet length, magic number, protocol version, packet type, and status fields.
- **Device Mapping:** Maps each packet's MAC address to a configured logical sensor name or role.
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
| Packet Size | 26 bytes |
| Byte Order | Big Endian |
| Retransmission | Disabled |

### Packet Format

| Offset | Field | Type | Size |
|---|---|---|---|
| 0 | `magic` | uint16 | 2 bytes |
| 2 | `version` | uint8 | 1 byte |
| 3 | `packet_type` | uint8 | 1 byte |
| 4 | `device_mac` | byte[6] | 6 bytes |
| 10 | `sequence` | uint32 | 4 bytes |
| 14 | `timestamp_us` | uint64 | 8 bytes |
| 22 | `raw_angle` | uint16 | 2 bytes |
| 24 | `status` | uint8 | 1 byte |
| 25 | `reserved` | uint8 | 1 byte |

**Total packet size: 26 bytes.**

The `timestamp_us` field represents the ESP32's local monotonic time in microseconds. The server maps the embedded Wi-Fi station MAC to a logical sensor using its configuration file. Clocks are not synchronized across nodes in the MVP. See [`protocol/protocol.md`](../protocol/protocol.md) for field semantics and status bits.

## Device Identification

All nodes use the same firmware and include their Wi-Fi station MAC in each sample packet. The server configuration maps each observed MAC address to a logical device name or sensor role. The firmware does not carry a per-node numeric ID; adding or remapping a node does not require rebuilding its image.

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
