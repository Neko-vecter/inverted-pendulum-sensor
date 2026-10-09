# Sensor UDP Protocol, Version 1

This document defines the sample datagrams sent by each ESP32-C6 sensor node.
It is the wire contract between the firmware and the Rust sensor server.

## Transport

| Setting | Value |
|---|---|
| Network | Wi-Fi station |
| Transport | UDP over IPv4 |
| Destination | Server IPv4 and UDP port from firmware configuration |
| Default destination port | `5000` |
| Sample target | 1,000 datagrams per second per node |
| Datagram payload | Exactly 26 bytes |
| Byte order | Big-endian for multi-byte integers |
| Retransmission | None |

Every datagram contains one sample record. UDP delivery is best effort. The
firmware uses a bounded queue between sampling and transmission; queue overflow,
network outages, and UDP loss can produce sequence gaps.

## Sample frame

The packet is encoded field-by-field. It is not a native C structure and has no
compiler-dependent padding.

| Offset | Length | Field | Encoding |
|---:|---:|---|---|
| 0 | 2 | `magic` | Unsigned 16-bit big-endian; value `0x4950` (`"IP"`) |
| 2 | 1 | `version` | Unsigned 8-bit; value `1` |
| 3 | 1 | `packet_type` | Unsigned 8-bit; sample value `0x01` |
| 4 | 6 | `device_mac` | Raw Wi-Fi station MAC octets, in display order |
| 10 | 4 | `sequence` | Unsigned 32-bit big-endian |
| 14 | 8 | `timestamp_us` | Unsigned 64-bit big-endian |
| 22 | 2 | `raw_angle` | Unsigned 16-bit big-endian; valid values are 0 through 16,383 |
| 24 | 1 | `status` | Status bit field, defined below |
| 25 | 1 | `reserved` | Must be zero; receiver should ignore its value in version 1 |

Total payload length is **26 bytes**. A version 1 receiver should reject
datagrams with a different length, unknown magic, unsupported version, or
unknown packet type.

### Device identity and server mapping

`device_mac` is the six-byte Wi-Fi station MAC read from the ESP32-C6. For
example, the displayed MAC `AA:BB:CC:DD:EE:FF` is transmitted as bytes
`AA BB CC DD EE FF`. The MAC is included in the payload because an IP/UDP
receiver cannot use the original Wi-Fi link-layer source MAC to identify a node
after packets pass through an access point and IP network.

The ESP32-C6 firmware does not assign a numeric `device_id`. The server maps
`device_mac` to its configured logical sensor name or role. This mapping is a
server-side configuration-file concern; it does not require a downlink packet
or a different firmware image for each node. The server should treat an
unmapped MAC as an unknown device according to its own policy.

### Sequence

`sequence` starts at zero for the first sampling task wake after boot and
advances for each subsequent 1 ms sampling slot, including slots whose sample
is not transmitted. It wraps modulo 2³². Samples omitted while the network is
disconnected, while the sampling task is late, or when the local transmit queue
is full therefore appear as gaps between received records. A reboot resets the
counter; the receiver can also use the MAC and timestamp to distinguish a
restart from ordinary packet loss.

### Timestamp

`timestamp_us` is the ESP32-C6 monotonic time in microseconds at the start of
the MT6701 read. It is local to one boot and is not synchronized with other
nodes or the server. It is encoded as an unsigned 64-bit value; the server
should not interpret it as Unix time.

### Angle and status

The MT6701 provides a 14-bit absolute angle. The firmware reads register
`0x03` first and `0x04` second, then combines `Angle[13:6]` from the first byte
with `Angle[5:0]` from bits 7 through 2 of the second byte. The transmitted
integer is in the range 0–16,383. Convert it to degrees as:

```text
degrees = raw_angle * 360.0 / 16384.0
```

The `status` bits in version 1 are:

| Bit | Mask | Meaning |
|---:|---:|---|
| 0 | `0x01` | Angle is valid (`1` when both I²C register reads succeeded) |
| 1 | `0x02` | MT6701 I²C read failed |
| 2–7 | — | Reserved; transmitted as zero |

The sender emits exactly `0x01` when both I²C register reads succeed and
exactly `0x02` when either read fails. For `status = 0x01`, `raw_angle` is in
the range 0–16,383. For `status = 0x02`, `raw_angle` is `0xFFFF`. A version 1
receiver should reject any other `status` value, including values with
reserved bits set or both defined bits set. It must use the valid status bit
and must not interpret `0xFFFF` as an angle.

## Example datagram

This sample has MAC `AA:BB:CC:DD:EE:FF`, sequence `0x01020304`, monotonic
timestamp `1,000,000` µs, angle `0x2A10`, and a valid sensor reading:

```text
49 50 01 01 AA BB CC DD EE FF 01 02 03 04
00 00 00 00 00 0F 42 40 2A 10 01 00
```

## Versioning and scope

The protocol version is the byte at offset 2. A future incompatible frame
change must use a new version. Version 1 only defines ESP32-C6-to-server sample
datagrams; it defines no acknowledgements, server commands, remote
configuration, or clock synchronization.
