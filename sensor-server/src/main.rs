mod config;
mod protocol;
mod state;

use crate::protocol::decode_sample;
use crate::state::{DeviceRegistry, DeviceState, ServerMetrics, format_rate};
use std::env;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

const UDP_BUFFER_SIZE: usize = 2048;
const SOCKET_READ_TIMEOUT: Duration = Duration::from_millis(100);

fn main() {
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

    if let Err(error) = run(&config_path) {
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

fn run(config_path: &std::path::Path) -> Result<(), String> {
    let config = config::load(config_path)?;
    let socket = UdpSocket::bind(config.bind_address)
        .map_err(|error| format!("cannot bind UDP {}: {error}", config.bind_address))?;
    socket
        .set_read_timeout(Some(SOCKET_READ_TIMEOUT))
        .map_err(|error| format!("cannot set UDP receive timeout: {error}"))?;

    let local_address = socket
        .local_addr()
        .map_err(|error| format!("cannot read bound UDP address: {error}"))?;
    println!(
        "sensor-server listening on {local_address}; protocol v1, {} configured device(s)",
        config.devices_by_mac.len()
    );
    println!("stats interval: {}s", config.stats_interval.as_secs());
    println!("press Ctrl+C to stop");

    let mut devices = DeviceRegistry::new(config.devices_by_mac);
    let mut metrics = ServerMetrics::default();
    let mut buffer = vec![0_u8; UDP_BUFFER_SIZE];
    let mut interval_started = Instant::now();

    loop {
        match socket.recv_from(&mut buffer) {
            Ok((length, _sender)) => {
                handle_datagram(&buffer[..length], &mut devices, &mut metrics);
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

fn handle_datagram(packet: &[u8], devices: &mut DeviceRegistry, metrics: &mut ServerMetrics) {
    let sample = match decode_sample(packet) {
        Ok(sample) => sample,
        Err(error) => {
            metrics.record_malformed(error);
            return;
        }
    };

    metrics.record_decoded();
    if devices.record(sample, Instant::now()) {
        metrics.record_mapped();
    } else {
        metrics.record_unknown(sample.device_mac);
    }
}

fn print_stats(
    elapsed: Duration,
    now: Instant,
    devices: &mut DeviceRegistry,
    metrics: &mut ServerMetrics,
) {
    let interval = metrics.take_interval();
    println!(
        "\n[{:.3}s] datagrams={} mapped={} unknown={} malformed={} receive_errors_total={}",
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
        "  {} ({}) rx={rate:.1}/s total={} seq_gaps={} lost_est={} loss={} duplicates={} out_of_order={} restarts={} i2c_errors={} {latest}",
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
