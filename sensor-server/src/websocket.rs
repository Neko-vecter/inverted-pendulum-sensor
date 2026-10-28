use crate::state::DeviceRegistry;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io;
use std::net::{SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{Request, Response};
use tungstenite::{Message, WebSocket, accept_hdr};

const CLIENT_QUEUE_CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct WebSocketHub {
    shared: Arc<Shared>,
}

struct Shared {
    server_start: Instant,
    next_client_id: AtomicU64,
    state: Mutex<HubState>,
}

struct HubState {
    sensor_names: BTreeSet<String>,
    latest: BTreeMap<String, SensorValue>,
    clients: HashMap<u64, Client>,
}

struct Client {
    sensors: BTreeSet<String>,
    sender: mpsc::SyncSender<HubUpdate>,
}

#[derive(Clone)]
struct SensorValue {
    angle: Option<f64>,
    received_at: Instant,
}

#[derive(Clone)]
struct SensorUpdate {
    name: String,
    value: SensorValue,
}

#[derive(Clone)]
struct HubUpdate {
    sensors: Vec<SensorUpdate>,
    timestamp: Instant,
}

impl WebSocketHub {
    pub fn new(sensor_names: impl IntoIterator<Item = String>, server_start: Instant) -> Self {
        Self {
            shared: Arc::new(Shared {
                server_start,
                next_client_id: AtomicU64::new(1),
                state: Mutex::new(HubState {
                    sensor_names: sensor_names.into_iter().collect(),
                    latest: BTreeMap::new(),
                    clients: HashMap::new(),
                }),
            }),
        }
    }

    pub fn publish(&self, name: &str, angle: Option<f64>, received_at: Instant) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        let value = SensorValue { angle, received_at };
        state.latest.insert(name.to_owned(), value.clone());
        let update = HubUpdate {
            sensors: vec![SensorUpdate {
                name: name.to_owned(),
                value,
            }],
            timestamp: received_at,
        };
        state.clients.retain(|_, client| {
            if !client.sensors.contains(name) {
                return true;
            }
            match client.sender.try_send(update.clone()) {
                Ok(()) => true,
                Err(mpsc::TrySendError::Full(_) | mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
    }

    pub fn start(&self, bind_address: SocketAddrV4) -> io::Result<()> {
        let listener = TcpListener::bind(bind_address)?;
        listener.set_nonblocking(true)?;
        let hub = self.clone();
        thread::Builder::new()
            .name("sensor-websocket-listener".to_owned())
            .spawn(move || {
                loop {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let hub = hub.clone();
                            let _ = thread::Builder::new()
                                .name("sensor-websocket-client".to_owned())
                                .spawn(move || hub.serve_client(stream));
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(20)),
                    }
                }
            })?;
        Ok(())
    }

    fn serve_client(&self, stream: TcpStream) {
        let Ok(mut socket) = accept_hdr(stream, |request: &Request, response: Response| {
            if request.uri().path() == "/ws" {
                Ok(response)
            } else {
                Err(http::Response::builder()
                    .status(404)
                    .body(Some("WebSocket endpoint is /ws".to_owned()))
                    .expect("valid WebSocket rejection response"))
            }
        }) else {
            return;
        };
        let _ = socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(2)));
        let client_id = self.shared.next_client_id.fetch_add(1, Ordering::Relaxed);
        let mut subscription: Option<mpsc::Receiver<HubUpdate>> = None;
        let mut sequence = 0_u64;

        loop {
            if let Some(receiver) = &subscription {
                match receiver.try_recv() {
                    Ok(update) => {
                        sequence = sequence.wrapping_add(1);
                        if send_update(&mut socket, sequence, update, self.shared.server_start)
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => break,
                    Err(mpsc::TryRecvError::Empty) => {}
                }
            }

            match socket.read() {
                Ok(Message::Text(text)) => match serde_json::from_str::<SubscribeRequest>(&text) {
                    Ok(request) if request.message_type == "subscribe" => {
                        match self.subscribe(client_id, &request.sensors) {
                            Ok((receiver, snapshot)) => {
                                subscription = Some(receiver);
                                sequence = sequence.wrapping_add(1);
                                if send_update(
                                    &mut socket,
                                    sequence,
                                    snapshot,
                                    self.shared.server_start,
                                )
                                .is_err()
                                {
                                    break;
                                }
                            }
                            Err(message) => {
                                let error = json!({
                                    "type": "error",
                                    "code": "unknown_sensor",
                                    "message": message,
                                });
                                if socket.send(Message::Text(error.to_string())).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(_) => {
                        if send_error(
                            &mut socket,
                            "invalid_request",
                            "type must be subscribe and sensors must be a non-empty array",
                        )
                        .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => {
                        if send_error(
                            &mut socket,
                            "invalid_json",
                            "request must be a valid JSON object",
                        )
                        .is_err()
                        {
                            break;
                        }
                    }
                },
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) => {}
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    break;
                }
                Err(_) => break,
            }
        }

        if let Ok(mut state) = self.shared.state.lock() {
            state.clients.remove(&client_id);
        }
    }

    fn subscribe(
        &self,
        client_id: u64,
        requested: &[String],
    ) -> Result<(mpsc::Receiver<HubUpdate>, HubUpdate), String> {
        if requested.is_empty() {
            return Err("sensors must contain at least one configured sensor name".to_owned());
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| "sensor state is unavailable".to_owned())?;
        let sensors: BTreeSet<String> = if requested.iter().any(|name| name == "*") {
            state.sensor_names.clone()
        } else {
            requested.iter().cloned().collect()
        };
        if sensors.is_empty() {
            return Err("no sensors are configured".to_owned());
        }
        if let Some(unknown) = sensors
            .iter()
            .find(|name| !state.sensor_names.contains(*name))
        {
            return Err(format!("unknown sensor: {unknown}"));
        }

        let (sender, receiver) = mpsc::sync_channel(CLIENT_QUEUE_CAPACITY);
        let now = Instant::now();
        let snapshot = HubUpdate {
            sensors: sensors
                .iter()
                .filter_map(|name| {
                    state.latest.get(name).cloned().map(|value| SensorUpdate {
                        name: name.clone(),
                        value,
                    })
                })
                .collect(),
            timestamp: now,
        };
        state.clients.insert(client_id, Client { sensors, sender });
        Ok((receiver, snapshot))
    }
}

#[derive(Deserialize)]
struct SubscribeRequest {
    #[serde(rename = "type")]
    message_type: String,
    sensors: Vec<String>,
}

fn send_update(
    socket: &mut WebSocket<TcpStream>,
    sequence: u64,
    update: HubUpdate,
    server_start: Instant,
) -> Result<(), tungstenite::Error> {
    let now = Instant::now();
    let mut sensors = serde_json::Map::new();
    for sensor in update.sensors {
        sensors.insert(
            sensor.name,
            json!({
                "angle": sensor.value.angle,
                "age_us": now.saturating_duration_since(sensor.value.received_at).as_micros().min(u128::from(u64::MAX)) as u64,
            }),
        );
    }
    let timestamp_us = update
        .timestamp
        .saturating_duration_since(server_start)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64;
    let message = json!({
        "event": "update",
        "type": "sensor_data",
        "seq": sequence,
        "timestamp_us": timestamp_us,
        "sensor": Value::Object(sensors),
    });
    socket.send(Message::Text(message.to_string()))
}

fn send_error(
    socket: &mut WebSocket<TcpStream>,
    code: &str,
    message: &str,
) -> Result<(), tungstenite::Error> {
    socket.send(Message::Text(
        json!({"type": "error", "code": code, "message": message}).to_string(),
    ))
}

pub fn start_from_registry(
    devices: &DeviceRegistry,
    server_start: Instant,
    bind_address: SocketAddrV4,
) -> Result<WebSocketHub, String> {
    let hub = WebSocketHub::new(devices.by_name.keys().cloned(), server_start);
    hub.start(bind_address)
        .map_err(|error| format!("cannot start WebSocket API on {bind_address}: {error}"))?;
    Ok(hub)
}
