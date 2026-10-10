mod config;
mod protocol;
mod state;

use crate::protocol::{DecodedDatagram, decode_datagram, encode_time_sync_response};
use crate::state::{
    DeviceRegistry, DeviceState, ServerMetrics, format_rate, latency_percentiles_us,
};
use std::env;
use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const UDP_BUFFER_SIZE: usize = 2048;
const SOCKET_READ_TIMEOUT: Duration = Duration::from_millis(100);

fn main() {
    let server_start = Instant::now();
    let config_path = match config_path_from_args() {
        Ok(Some(path)) => path,
        Ok(None) => {
            print_usage();
            return;
        }
        Err(error) => {
            eprintln!("error: {error}");
            print_usage();
            process::exit(2);
        }
    };

    if let Err(error) = run(&config_path, server_start) {
        eprintln!("sensor-server: {error}");
        process::exit(1);
    }
}

fn config_path_from_args() -> Result<Option<PathBuf>, String> {
    let mut args = env::args().skip(1);
    let mut config_path = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(None),
            "-c" | "--config" => {
                if config_path.is_some() {
                    return Err("specify --config only once".to_owned());
                }
                let Some(path) = args.next() else {
                    return Err("--config requires a path".to_owned());
                };
                config_path = Some(PathBuf::from(path));
            }
            _ => return Err(format!("unknown argument {argument:?}")),
        }
    }
    Ok(Some(
        config_path.unwrap_or_else(|| PathBuf::from("config.toml")),
    ))
}

fn print_usage() {
    println!("Usage: sensor-server [--config PATH]\n       sensor-server --help");
    println!("Default config path: config.toml in the current directory");
}

fn run(config_path: &std::path::Path, server_start: Instant) -> Result<(), String> {
    let config = config::load(config_path)?;
    let server_session_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))?
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    let server_session_id = server_session_id.max(1);
    let socket = UdpSocket::bind(config.bind_address)
        .map_err(|error| format!("cannot bind UDP {}: {error}", config.bind_address))?;
    socket
        .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
        .map_err(|error| format!("cannot set UDP receive timeout: {error}"))?;

    let local_address = socket
        .local_addr()
        .map_err(|error| format!("cannot read bound UDP address: {error}"))?;
    println!(
        "sensor-server listening on {local_address}; protocol v2, {} configured device(s)",
        config.devices_by_mac.len()
    );
    println!("time-sync session: {server_session_id}");
    println!("stats interval: {}s", config.stats_interval.as_secs());
    println!("press Ctrl+C to stop");

    let mut devices = DeviceRegistry::new(config.devices_by_mac);
    let mut metrics = ServerMetrics::default();
    let mut buffer = vec![0_u8; UDP_BUFFER_SIZE];
    let mut interval_started = Instant::now();

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((length, sender)) => {
                let received_at = Instant::now();
                let server_now_us = monotonic_us(server_start, received_at) as u32;
                handle_datagram(
                    &buffer[..length],
                    sender,
                    received_at,
                    server_now_us,
                    server_start,
                    server_session_id,
                    &socket,
                    &mut devices,
                    &mut metrics,
                );
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                metrics.record_receive_error();
                eprintln!("UDP receive error: {error}");
                thread::sleep(Duration::from_millis(100));
            }
        }

        let now = Instant::now();
        let elapsed = now.duration_since(interval_started);
        if elapsed >= config.stats_interval {
            print_stats(elapsed, now, &mut devices, &mut metrics);
            interval_started = now;
        }
    }
}

fn handle_datagram(
    packet: &[u8],
    sender: SocketAddr,
    received_at: Instant,
    server_now_us: u32,
    server_start: Instant,
    server_session_id: u64,
    socket: &UdpSocket,
    devices: &mut DeviceRegistry,
    metrics: &mut ServerMetrics,
) {
    let datagram = match decode_datagram(packet) {
        Ok(datagram) => datagram,
        Err(error) => {
            metrics.record_malformed(error);
            return;
        }
    };

    match datagram {
        DecodedDatagram::Sample(sample) => {
            metrics.record_decoded();
            if devices.record(
                sample,
                received_at,
                server_now_us,
                server_session_id,
            ) {
                metrics.record_mapped();
            } else {
                metrics.record_unknown(sample.device_mac);
            }
        }
        DecodedDatagram::TimeSyncRequest(request) => {
            if !devices.contains_mac(request.device_mac) {
                return;
            }
            let server_t3_us = monotonic_us(server_start, Instant::now()) as u32;
            let response = encode_time_sync_response(
                request,
                server_now_us,
                server_t3_us,
                server_session_id,
            );
            if let Err(error) = socket.send_to(&response, sender) {
                eprintln!("UDP time-sync response to {sender} failed: {error}");
            }
        }
    }
}

fn monotonic_us(start: Instant, now: Instant) -> u64 {
    now.saturating_duration_since(start)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

fn print_stats(
    elapsed: Duration,
    now: Instant,
    devices: &mut DeviceRegistry,
    metrics: &mut ServerMetrics,
) {
    let interval = metrics.take_interval();
    println!(
        "\n[{:.3}s] samples={} mapped={} unknown={} malformed={} receive_errors_total={}",
        elapsed.as_secs_f64(),
        interval.decoded,
        interval.mapped,
        interval.unknown,
        interval.malformed.values().sum::<u64>(),
        metrics.receive_errors_total
    );
    println!(
        "  totals decoded={} mapped={} unknown={} malformed={}",
        metrics.decoded_total, metrics.mapped_total, metrics.unknown_total, metrics.malformed_total
    );

    if !interval.malformed.is_empty() {
        let details = interval
            .malformed
            .iter()
            .map(|(kind, count)| format!("{}={count}", kind.label()))
            .collect::<Vec<_>>()
            .join(", ");
        println!("  rejected: {details}");
    }
    for (mac, count) in interval.unknown_by_mac {
        println!("  unmapped MAC {mac}: {count} datagram(s)");
    }
    if interval.unknown_unlisted > 0 {
        println!(
            "  additional unmapped datagrams from unlisted MACs: {}",
            interval.unknown_unlisted
        );
    }

    for state in devices.by_name.values_mut() {
        print_device_stats(state, elapsed, now);
    }
}

fn print_device_stats(state: &mut DeviceState, elapsed: Duration, now: Instant) {
    let interval_received = state.take_interval_received();
    let rate = format_rate(interval_received, elapsed);
    let latency = state.take_latency_interval();
    let latency_total_samples = latency.total_samples;
    let percentile_sample_count = latency.samples_us.len();
    let sync_rtt = latency
        .latest_sync_rtt_us
        .map_or_else(|| "n/a".to_owned(), |rtt| rtt.to_string());
    let latency_unsynced = latency.unsynced;
    let latency_clock_ahead = latency.clock_ahead;
    let latency_percentiles = latency_percentiles_us(latency.samples_us);
    let latency_summary = match latency_percentiles {
        Some([p50, p75, p90, p99]) => format!(
            "latency_ms p50={:.3} p75={:.3} p90={:.3} p99={:.3} samples={} percentile_samples={} sync_rtt_us={} unsynced={} clock_ahead={}",
            p50 as f64 / 1000.0,
            p75 as f64 / 1000.0,
            p90 as f64 / 1000.0,
            p99 as f64 / 1000.0,
            latency_total_samples,
            percentile_sample_count,
            sync_rtt,
            latency_unsynced,
            latency_clock_ahead
        ),
        None => format!(
            "latency_ms n/a samples={} sync_rtt_us={} unsynced={} clock_ahead={}",
            latency_total_samples,
            sync_rtt,
            latency_unsynced,
            latency_clock_ahead
        ),
    };
    let loss = state
        .loss_percent()
        .map(|percent| format!("{percent:.3}%"))
        .unwrap_or_else(|| "n/a".to_owned());

    let latest = match state.latest {
        Some(latest) => {
            let age_ms = now
                .saturating_duration_since(latest.received_at)
                .as_millis();
            match latest.sample.angle_degrees() {
                Some(degrees) => format!(
                    "angle={degrees:.3}deg raw={} seq={} ts={}us age={}ms",
                    latest.sample.raw_angle,
                    latest.sample.sequence,
                    latest.sample.timestamp_us,
                    age_ms
                ),
                None => format!(
                    "angle=invalid(i2c_error) seq={} ts={}us age={}ms",
                    latest.sample.sequence, latest.sample.timestamp_us, age_ms
                ),
            }
        }
        None => "latest=none".to_owned(),
    };

    println!(
        "  {} ({}) rx={rate:.1}/s total={} seq_gaps={} lost_est={} loss={} duplicates={} out_of_order={} restarts={} i2c_errors={} {latency_summary} {latest}",
        state.logical_name,
        state.mac,
        state.received_total,
        state.sequence_gaps,
        state.estimated_lost,
        loss,
        state.duplicates,
        state.out_of_order,
        state.restarts,
        state.sensor_errors
    );
}
