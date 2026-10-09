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
        TimeSync["Time Sync Handler"]
        Decoder["Packet Decoder"]
        Manager["Device State Manager"]
        Cache["Latest Sample Cache"]
        Stats["Packet, Rate & Latency Statistics"]

        UDP --> TimeSync
        TimeSync -->|"UDP sync response"| AP
        UDP --> Decoder
        Decoder --> Manager
        Manager --> Cache
        Manager --> Stats
    end

    E1 -->|"UDP / 1000 Hz"| AP
    E2 -->|"UDP / 1000 Hz"| AP
    E3 -->|"UDP / 1000 Hz"| AP

    AP --> UDP
    AP -->|"UDP sync response"| E1
    AP -->|"UDP sync response"| E2
    AP -->|"UDP sync response"| E3
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
    Encoder["46-byte v2 Packet Encoder"]
    Sender["Nonblocking UDP Sender"]
    Sync["UDP Clock Sync Client"]
    WiFi["Wi-Fi STA"]

    Sensor --> Driver
    Driver --> Sampler
    Sampler --> Queue
    Queue --> Encoder
    Encoder --> Sender
    Sender --> WiFi
    Sync <--> WiFi
    Sync --> Encoder
```

### Responsibilities

- **Sensor Driver:** Reads the raw 14-bit angle value from the MT6701 over I²C.
- **Sampling Task:** Uses a 1 ms high-resolution timer to target 1000 samples per second.
- **Sample Queue:** Decouples sensor reads from network transmission; samples are dropped when the bounded queue is full or Wi-Fi is disconnected.
- **Packet Encoder:** Encodes each sample into the fixed 46-byte version 2 binary format in [`protocol/protocol.md`](../protocol/protocol.md), including the Wi-Fi station MAC and clock-sync metadata.
- **UDP Sender:** Sends one nonblocking UDP datagram per queued sample.
- **Clock Sync Client:** Exchanges monotonic timestamps with the server over UDP about once per second and attaches the estimated clock offset and round-trip time to samples.
- **Wi-Fi Manager:** Establishes and maintains network connectivity, including reconnection.

## Rust Sensor Server Architecture

The Rust Sensor Server receives UDP packets from all three ESP32-C6 nodes and maintains their latest sensor states independently.

```mermaid
flowchart TD
    UDP["UDP Socket :5000"]
    Decoder["Protocol Decoder"]
    Validate{"Valid Packet?"}
    Router["MAC-to-Logical-Device Mapping"]
    Sync["Clock Sync Response"]

    S1["Sensor #1 State"]
    S2["Sensor #2 State"]
    S3["Sensor #3 State"]

    Stats["Reception Statistics"]

    UDP --> Decoder
    UDP --> Sync
    Sync -->|"UDP response to node"| UDP
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
- **Clock Sync Handler:** Replies to configured nodes with receive/send timestamps and the current server-session ID.
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
| Server Listen Port | 5000 (default) |
| Firmware UDP Destination | Configurable per node; must match the server listener |
| Sampling Rate | 1000 Hz per node (target) |
| Transmission Mode | One packet per sample |
| Packet Size | 46-byte version 2 sample; 26-byte version 1 samples remain accepted |
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
| 26 | `sync_session_id` | uint64 | 8 bytes |
| 34 | `clock_offset_us` | int64 | 8 bytes |
| 42 | `sync_round_trip_us` | uint32 | 4 bytes |

**Version 2 sample size: 46 bytes.**

The `timestamp_us` field is the ESP32's local monotonic time at the start of the MT6701 I2C read. A UDP time-sync exchange estimates each node's offset from the server's monotonic clock. The server uses the offset and matching session ID to calculate per-device sample-to-receive latency percentiles. The metric does not model or subtract MT6701 internal measurement delay. Version 1 sample packets remain accepted but do not provide latency estimates. See [`protocol/protocol.md`](../protocol/protocol.md) for field semantics and status bits.

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
- Device-to-server clock synchronization and per-device P50/P75/P90/P99 latency estimates.

### Excluded

- Remote device configuration.
- Remote start/stop commands.
- Hardware-level clock synchronization guarantees or a hard real-time latency guarantee.
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

**MVP Acceptance Criteria:** The Rust Sensor Server can continuously receive raw angle measurements from three physical ESP32-C6 nodes, each targeting 1000 samples per second, while reporting per-device reception rates, packet loss, and synchronized sample-to-receive latency percentiles.
