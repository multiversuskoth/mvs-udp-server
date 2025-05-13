mod compression;
mod ffi;
mod message_handler;
mod message_types;
mod models;
mod serializer;

use std::fs;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};

use anyhow::bail;
use chrono::Local;
use compression::{compress_packet, decompress_packet};
use message_handler::{MVSIPlayer, MessageHandler};

use log::{debug, error, info, warn};
use message_types::{
    client_messages::{ClientMessageType, ClientPayload},
    server_messages::{
        Header, PlayerGetReady, PlayerInputs, ServerMessagePayload, ServerMessageType, UdpServerMessage,
    },
};
use models::{game_match::GameMatch, player::Player};
use reqwest::Client;
use serializer::{parse_client_message, serialize_server_message};
use tokio::sync::MutexGuard;
use tokio::{net::UdpSocket, sync::Mutex};

use std::os::raw::c_uint;

#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(uPeriod: c_uint) -> c_uint;
    fn timeEndPeriod(uPeriod: c_uint) -> c_uint;
}

// Global static variable to hold port 41234
static MVSI_PORT: AtomicU16 = AtomicU16::new(41234);

pub fn get_mvsi_port() -> u16 {
    MVSI_PORT.load(Ordering::SeqCst)
}

fn get_bdomain_from_file() -> String {
    let file = fs::File::open("settings.ini").expect("Failed to open settings.ini");
    let reader = io::BufReader::new(file);

    for line in reader.lines() {
        if let Ok(line) = line {
            if line.starts_with("bDomain=") {
                return line[8..].trim_matches('"').to_string();
            }
        }
    }

    panic!("bDomain not found in settings.ini");
}

use once_cell::sync::Lazy;

static MVS_HTTP_ENDPOINT: Lazy<String> = Lazy::new(|| get_bdomain_from_file());

enum ServerState {
    Idle,
    WaitingForPlayers,
    MatchInProgress,
}

struct SharedState {
    players: Arc<Mutex<Vec<Player>>>,
    current_match: Arc<Mutex<GameMatch>>,
    current_state: Arc<Mutex<ServerState>>,
    passthrough: AtomicBool,
    host_socket: Arc<Mutex<Option<SocketAddr>>>,
    local_socket: Arc<Mutex<Option<SocketAddr>>>,
    players_data: Arc<Mutex<Vec<MVSIPlayer>>>,
}

#[derive(Clone)]
struct P2PRollbackServer {
    socket: Arc<UdpSocket>,
    current_state: Arc<SharedState>,
    http_client: reqwest::Client,
    is_local_player_connected: Arc<AtomicBool>,
    is_host: Arc<AtomicBool>,
}

impl P2PRollbackServer {
    pub async fn new() -> Self {
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", get_mvsi_port()))
            .await
            .expect("Failed to bind socket");
        debug!("UDP Started at {}", socket.local_addr().unwrap());
        let current_state = Arc::new(SharedState {
            players: Arc::new(Mutex::new(Vec::new())),
            current_match: Arc::new(Mutex::new(GameMatch::new())),
            current_state: Arc::new(Mutex::new(ServerState::Idle)),
            passthrough: AtomicBool::new(false),
            host_socket: Arc::new(Mutex::new(None)),
            local_socket: Arc::new(Mutex::new(None)),
            players_data: Arc::new(Mutex::new(vec![])),
        });

        let http_client = Client::new();

        let server = P2PRollbackServer {
            socket: Arc::new(socket),
            current_state,
            http_client,
            is_local_player_connected: Arc::new(AtomicBool::new(false)),
            is_host: Arc::new(AtomicBool::new(false)),
        };
        server
    }

    async fn send_players_get_ready(
        &self,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) -> anyhow::Result<()> {
        let player_count = players.len().clone();
        for player in players.iter() {
            let msg = ServerMessagePayload::PlayerGetReady(PlayerGetReady {
                num_players: player_count as u8,
                raw_data: vec![0u8; 4 * player_count],
            });
            self.send_message(ServerMessageType::PlayerGetReady, msg, &player.socket, current_match)
                .await;
        }

        Ok(())
    }

    async fn send_game_start(
        &self,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) -> anyhow::Result<()> {
        for player in players.iter() {
            let msg = ServerMessagePayload::StartGame {
                0: message_types::server_messages::Empty {},
            };
            self.send_message(ServerMessageType::StartGame, msg, &player.socket, current_match)
                .await;
        }

        Ok(())
    }

    async fn send_udp_hole_punch(&self, target: &SocketAddr, current_match: &mut MutexGuard<'_, GameMatch>) {
        self.send_message(
            ServerMessageType::MVSI_HOLE_PUNCH,
            ServerMessagePayload::Empty(),
            target,
            current_match,
        )
        .await;
    }

    pub async fn send_message(
        &self,
        header_type: ServerMessageType,
        message: ServerMessagePayload,
        target: &SocketAddr,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) {
        let server_msg = UdpServerMessage {
            header: Header {
                type_: header_type,
                sequence: current_match.sequence_number,
            },
            payload: message,
        };
        let serialized_msg = match serialize_server_message(&server_msg, current_match.num_players as usize) {
            Ok(msg) => msg,
            Err(e) => {
                warn!("Failed to serialize server message: {}", e);
                return;
            }
        };
        current_match.sequence_number += 1;
        let compressed = compress_packet(serialized_msg.as_slice())
            .map_err(|e| anyhow::anyhow!("Failed to compress packet: {}", e))
            .unwrap();
        match self.socket.send_to(compressed.as_slice(), target).await {
            Ok(_) => {
                let now = Local::now();
                let formatted = now.format("%H:%M:%S:%3f").to_string();
                debug!("{} Sent {:#?} to {:?}", formatted, server_msg.header.type_, target);
            }
            Err(e) => {
                error!("Failed to send message: {}", e);
            }
        }
    }

    async fn handle_incoming_message(&self, len: usize, buf: &[u8], src: SocketAddr) -> anyhow::Result<()> {
        {
            let host_socket = self.current_state.host_socket.lock().await;
            if let Some(host_socket_real) = *host_socket {
                debug!("Recv from {:?} ", src);
                if src.ip().to_string() == "127.0.0.1" {
                    debug!("Send to {:?} ", host_socket_real);
                    self.socket.send_to(&buf[0..len], host_socket_real).await;
                } else {
                    let local_socket = self.current_state.local_socket.lock().await.unwrap();

                    debug!("Send to {:?} ", local_socket);
                    self.socket.send_to(&buf[0..len], local_socket).await;
                }

                return Ok(());
            }
        }

        let decompressed =
            decompress_packet(&buf, None).map_err(|e| anyhow::anyhow!("Failed to decompress packet: {}", e))?;
        let client_msg = match parse_client_message(decompressed.as_slice()) {
            Ok(msg) => msg,
            Err(e) => {
                bail!("Failed to parse client message: {}", e);
            }
        };
        let now = Local::now();
        let formatted = now.format("%H:%M:%S:%3f").to_string();
        debug!("{} Recv {:#?} from {:?} ", formatted, client_msg.header.type_, src);

        if let ClientMessageType::MVSI_HOLE_PUNCH = client_msg.header.type_ {
            // Do nothing for now
            return Ok(());
        }
        // Register player
        if let ClientMessageType::PlayerConnection = client_msg.header.type_ {
            if let ClientPayload::PlayerConnectionPaylod(payload) = client_msg.payload {
                self.handle_new_connection(payload, src).await?;
                return Ok(());
            } else {
                warn!("Unexpected payload type for NewConnection message");
            }
        }

        {
            let mut players = self.current_state.players.lock().await;
            match players.iter_mut().find(|p| p.socket == src) {
                Some(player) => {
     
                    if client_msg.header.sequence < player.last_seq_received {
                        warn!("Received old message from player: {:?}", src);
                        return Ok(());
                    }
                    player.last_seq_received = client_msg.header.sequence;
                }
                None => {
                    warn!("Player not found for socket: {:?}", src);
                    return Ok(());
                }
            }
        }

        match client_msg.header.type_ {
            ClientMessageType::Pong => {
                if let ClientPayload::PongPayload(payload) = client_msg.payload {
                    self.handle_player_pong_response(payload, src).await?;
                } else {
                    warn!("Unexpected payload type for NewConnection message");
                }
            }
            ClientMessageType::ReadyForMatch => {
                if let ClientPayload::ReadyForMatchPayload(payload) = client_msg.payload {
                    self.handle_player_ready(payload, src).await?;
                } else {
                    warn!("Unexpected payload type for NewConnection message");
                }
            }
            ClientMessageType::PlayerInput => {
                if let ClientPayload::PlayerInputPayload(payload) = client_msg.payload {
                    self.handle_player_input(payload, src).await?;
                } else {
                    warn!("Unexpected payload type for NewConnection message");
                }
            }

            ClientMessageType::PlayerInputAck => {
                if let ClientPayload::PlayerInputAckPayload(payload) = client_msg.payload {
                    self.handle_player_input_ack(payload, src).await?;
                } else {
                    warn!("Unexpected payload type for NewConnection message");
                }
            }
            ClientMessageType::Disconnecting => {
                //self.player_disconnected(&buf[1..size]);
            }
            _ => {
                warn!("Unknown message for {:?} not implemented yet", client_msg.header.type_);
            }
        }
        Ok(())
    }

    async fn send_player_inputs(
        &self,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) -> anyhow::Result<()> {

        let peer_input_data: Vec<_> = players.iter().map(|peer| (peer.index, peer.inputs.clone())).collect();

        for recipient in players.iter_mut() {
            let mut start_frame = Vec::with_capacity(current_match.num_players as usize);
            let mut num_frames = Vec::with_capacity(current_match.num_players as usize);
            let mut input_per_frame = Vec::with_capacity(current_match.num_players as usize);

            recipient.missed_inputs = 0; // Reset miss counter

            // Initialize empty arrays for each player
            for _ in 0..current_match.num_players {
                input_per_frame.push(Vec::new());
            }

            // For each peer, decide what frames to send
            for (peer_idx, hist_map) in &peer_input_data {
                let last_ack = recipient.acked_frames[*peer_idx as usize];

                let next_frame = last_ack + 1;

                // If we have the next real input
                if hist_map.contains_key(&next_frame) {
                    start_frame.push(next_frame);

                    // Send everything we actually have
                    let mut f = next_frame;
                    let mut frames_for_player = 0;
                    while hist_map.contains_key(&f) {
                        input_per_frame[*peer_idx as usize].push(*hist_map.get(&f).unwrap());
                        frames_for_player += 1;
                        f += 1;
                    }
                    num_frames.push(frames_for_player);
                }
            }

            // Send the personalized PlayerInput message
            let msg = ServerMessagePayload::PlayerInputs(PlayerInputs {
                num_players: current_match.num_players,
                start_frame,
                num_frames,
                num_predicted_overrides: 0,
                unused_0: 0,
                ping: recipient.ping,
                packets_loss_percent: 0,
                rift: recipient.rift,
                unused_1: 0,
                input_per_frame,
            });

            //debug!("PlayerInputs {:#?}", msg);
            debug!(
                "PlayerPing-rift PING:{} RIFT:{} {}",
                recipient.ping, recipient.rift, recipient.socket,
            );

            //recipient.last_sent_time = Some(Instant::now());
            recipient
                .pending_pings
                .insert(current_match.sequence_number, Instant::now());

            self.send_message(ServerMessageType::PlayerInputs, msg, &recipient.socket, current_match)
                .await;
        }

        Ok(())
    }

    pub fn calc_rift_variable_tick(&self, server_frame: u32, client_frame: u32, ping: u16) -> f32 {
        let target_frame_time = 16.6667;

        let ping_in_frames = ping as f32 / target_frame_time;

        let rift = (client_frame as f32 + ping_in_frames / 2.0) - server_frame as f32;
        println!("RIFT IS {}", rift);
        // Clamp rift to [-49.0, 49.0]
        rift.clamp(-49.0, 49.0)
    }
}

pub async fn start_rollback_server() -> anyhow::Result<()> {
    info!("Starting MVS P2P Rollback Server");
    unsafe {
        let result = timeBeginPeriod(1);
        if result != 0 {
            eprintln!("timeBeginPeriod failed: {}", result);
        }
    }

    let handler = P2PRollbackServer::new().await;
    loop {
        let mut buf = [0; 1024];
        let (len, addr) = handler.socket.recv_from(&mut buf).await?;
        let handler_clone = handler.clone();
        let now = Local::now();
        let formatted = now.format("%H:%M:%S:%3f").to_string();
        //println!("{} RECEIVED {} ", formatted, addr);
        tokio::spawn(async move { handler_clone.handle_incoming_message(len, &buf, addr).await });
    }
}
