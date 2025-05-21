use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::bail;
use log::{debug, error, info, warn};
use serde_json::json;
use tokio::{
    sync::MutexGuard,
    time::{interval, sleep},
};

use crate::{
    get_mvsi_port,
    message_types::{
        client_messages::{
            PlayerConnectionPaylod, PlayerInputAckPayload, PlayerInputPayload, PongPayload, ReadyForMatchPayload,
        },
        server_messages::{PlayerConnection, RequestPing, ServerMessagePayload, ServerMessageType},
    },
    models::{
        game_match::GameMatch,
        player::{self, Player},
    },
    P2PRollbackServer, MVS_HTTP_ENDPOINT,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]

pub struct MVSIPlayer {
    player_index: u16,
    ip: String,
    is_host: bool,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MVSIMatchConfig {
    max_players: u8,
    match_duration: u32,
    players: Vec<MVSIPlayer>,
}

pub trait MessageHandler {
    async fn handle_new_connection(&self, payload: PlayerConnectionPaylod, src: SocketAddr) -> anyhow::Result<()>;

    async fn ping_players(&self) -> anyhow::Result<()>;
    async fn handle_player_pong_response(&self, payload: PongPayload, src: SocketAddr) -> anyhow::Result<()>;
    async fn handle_player_ready(&self, payload: ReadyForMatchPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input_ack(&self, payload: PlayerInputAckPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn try_register_match(&self, payload: &PlayerConnectionPaylod, current_match: &mut MutexGuard<'_, GameMatch>);
    async fn fetch_player_data(
        &self,
        payload: &PlayerConnectionPaylod,
        src: SocketAddr,
    ) -> anyhow::Result<Vec<MVSIPlayer>>;
}

impl MessageHandler for P2PRollbackServer {
    async fn try_register_match(
        &self,
        payload: &PlayerConnectionPaylod,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) {
        if !current_match.ready {
            let response = self
                .http_client
                .post(format!("{}/mvsi_register", MVS_HTTP_ENDPOINT.as_str()))
                .json(&json!({
                    "matchId": payload.match_data.match_id,
                    "key": payload.match_data.key
                }))
                .send()
                .await;

            match response {
                Ok(resp) => match resp.json::<MVSIMatchConfig>().await {
                    Ok(match_data) => {
                        debug!("MATCH_DATA:{:#?}", match_data);
                        let mut players_data = self.current_state.http_players.lock().await;
                        *players_data = match_data.players;
                        current_match.num_players = match_data.max_players;
                        current_match.match_id = payload.match_data.match_id.clone();
                        current_match.match_key = payload.match_data.key.clone();
                        current_match.ready = true;
                        current_match.match_duration = match_data.match_duration;
                    }
                    Err(e) => {
                        error!("Failed to DECODE JSON: {}", e);
                    }
                },

                Err(e) => {
                    error!("Failed to register match: {}", e);
                }
            }
        }
    }

    async fn fetch_player_data(
        &self,
        payload: &PlayerConnectionPaylod,
        src_socket: SocketAddr,
    ) -> anyhow::Result<Vec<MVSIPlayer>> {
        let response: Result<reqwest::Response, reqwest::Error> = self
            .http_client
            .post(format!("{}/mvsi_match_players", MVS_HTTP_ENDPOINT.as_str()))
            .json(&json!({
                "matchId": payload.match_data.match_id,
                "key": payload.match_data.key,
            }))
            .send()
            .await;
        match response {
            Ok(resp) => match resp.json::<Vec<MVSIPlayer>>().await {
                Ok(players_data) => {
                    Ok(players_data)
                    /* if let Some(player_data) = players_data
                        .iter()
                        .find(|p| p.player_index == payload.player_data.player_index as u64)
                    {
                        let mut players = self.current_state.players.lock().await;
                        let curent_player = players.iter_mut().find(|p| p.socket == src_socket);
                        if let Some(player) = curent_player {
                            if player_data.is_host {
                                player.is_host = player_data.is_host;
                            }
                        }

                        // If we have defined that we are the host then just skip this
                        let is_host = self.is_host.load(Ordering::SeqCst);
                        if !is_host {
                            if player_data.is_host {
                                debug!("Player is host");
                                self.is_host.store(true, Ordering::SeqCst);
                            }
                        }
                    } */
                }
                Err(e) => bail!("Error Parsing JSON {}", e.to_string()),
            },
            Err(e) => {
                bail!("Failed to fetch players from HTTP endpoint: {}", e);
            }
        }
    }

    async fn handle_new_connection(
        &self,
        payload: PlayerConnectionPaylod,
        src_socket: SocketAddr,
    ) -> anyhow::Result<()> {
        {
            let current_player_index = payload.player_data.player_index as u16;
            let mut current_match = self.current_state.current_match.lock().await;

            let is_local_player_connected = self.is_local_player_connected.load(Ordering::SeqCst);
            if !is_local_player_connected {
                // Save the local socket
                // This is the socket that we will use to send messages to the local player
                {
                    let mut local_socket = self.current_state.local_socket.lock().await;
                    *local_socket = Some(src_socket);
                }
                // Register match if its not already
                self.try_register_match(&payload, &mut current_match).await;
                self.is_local_player_connected.store(true, Ordering::SeqCst);
                let http_players_data = self.current_state.http_players.lock().await.clone();

                if let Some(http_player) = http_players_data
                    .iter()
                    .find(|p| p.player_index == payload.player_data.player_index)
                {
                    if http_player.is_host {
                        info!("PLAYER IS HOST");
                        self.is_host.store(true, Ordering::SeqCst);

                        // UDP Hole punch all other players if host
                        for player_data in http_players_data.iter() {
                            if player_data.player_index != payload.player_data.player_index {
                                let mut count = 0;
                                let target = SocketAddr::new(player_data.ip.parse().unwrap(), get_mvsi_port());
                                let s_clone = self.clone();
                                // Spawn a new task to send UDP hole punch packets to everyone else
                                tokio::spawn(async move {
                                    loop {
                                        let mut current_match = s_clone.current_state.current_match.lock().await;
                                        if count > 3 {
                                            break;
                                        }
                                        s_clone.send_udp_hole_punch(&target, &mut current_match).await;
                                        count += 1;
                                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                                    }
                                });
                            }
                        }
                    } else {
                        // If we are not the host then we need to find the host
                        // and set the host socket
                        for player_data in http_players_data.iter() {
                            if player_data.is_host {
                                let host_target = SocketAddr::new(player_data.ip.parse().unwrap(), get_mvsi_port());
                                let mut host_socket = self.current_state.host_socket.lock().await;
                                *host_socket = Some(host_target);
                            }
                        }
                        return Ok(());
                    }
                }
            }

            let mut players = self.current_state.players.lock().await;
            if players.iter().any(|p| p.index == current_player_index) {
                debug!("Player already exists: index={}, {}", current_player_index, src_socket);
                return Ok(());
            }

            {
                // If host socket is set then we don't need to do anything
                // and just return. We will now just forward the packets to the host socket
                let host_socket = self.current_state.host_socket.lock().await;
                if let Some(_) = *host_socket {
                    return Ok(());
                }
            }

            let http_data = self.current_state.http_players.lock().await;

            if let Some(http_player) = http_data.iter().find(|p| p.player_index == current_player_index) {
                let msg = ServerMessagePayload::PlayerConnection(PlayerConnection {
                    success: 0,
                    num_players: current_match.num_players as u8,
                    player_index: current_player_index as u8,
                    match_duration: current_match.match_duration,
                    unused_0: 0,
                    unused_1: 0,
                });

                let player = Player {
                    index: current_player_index,
                    team_index: payload.player_data.team_id,
                    socket: src_socket,
                    pending_pings: HashMap::new(),
                    replied_pings: 0,
                    ready: false,
                    connected: true,
                    ping: 0,
                    last_client_frame: 0,
                    rift: 0.0,
                    acked_frames: vec![0; current_match.num_players as usize],
                    inputs: HashMap::new(),
                    missed_inputs: 0,
                    is_host: http_player.is_host,
                    last_seq_received: 0,
                };

                players.push(player);
                drop(http_data);

                self.send_message(
                    ServerMessageType::PlayerConnection,
                    msg,
                    &src_socket,
                    &mut current_match,
                )
                .await;

                debug!("Player {} connected with {}", current_player_index, src_socket);
            }

            if current_match.ready {
                let all_connected =
                    players.iter().filter(|p| p.connected).count() == current_match.num_players as usize;
                if all_connected {
                    let server_clone = self.clone();
                    tokio::spawn(async move {
                        server_clone.ping_players().await;
                    });
                }
            }
            Ok(())
        }
    }

    async fn handle_player_pong_response(&self, payload: PongPayload, src: SocketAddr) -> anyhow::Result<()> {
        let mut players = self.current_state.players.lock().await;

        // Find the player based on the source address
        if let Some(player) = players.iter_mut().find(|p| p.socket == src) {
            debug!("handle_player_pong_response {}", src);
            if let Some(start_time) = player.pending_pings.remove(&payload.server_message_sequence_number) {
                // Calculate the ping duration in milliseconds
                let duration = start_time.elapsed().as_millis() as u32;
                player.ping = duration as u16;
                player.replied_pings += 1;
                debug!("Updated ping for player {}: {} ms", player.index, player.ping);
            } else {
                warn!(
                    "Sequence number {} not found in pending pings for player {}",
                    payload.server_message_sequence_number, player.index
                );
            }
        } else {
            warn!("Player with socket {:?} not found", src);
        }

        Ok(())
    }

    async fn ping_players(&self) -> anyhow::Result<()> {
        let max_pings = 10;
        let mut current_match = self.current_state.current_match.lock().await;

        loop {
            {
                // check if all players have been pinged the max_pings times
                let mut players = self.current_state.players.lock().await;
                let all_pinged = players.iter().all(|player| player.replied_pings >= max_pings);
                if all_pinged {
                    break;
                }

                for player in players.iter_mut() {
                    let msg = ServerMessagePayload::RequestPing(RequestPing {
                        ping: player.ping as u16,
                        packets_loss_percent: 0,
                    });
                    let sequence_number = current_match.sequence_number;
                    player.pending_pings.insert(sequence_number, Instant::now());
                    self.send_message(ServerMessageType::RequestPing, msg, &player.socket, &mut current_match)
                        .await;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        let mut players = self.current_state.players.lock().await;
        // Sort players by key useful for later
        players.sort_by_key(|p| p.index);
        self.send_players_get_ready(&mut players, &mut current_match).await?;

        Ok(())
    }

    async fn handle_player_ready(&self, payload: ReadyForMatchPayload, src: SocketAddr) -> anyhow::Result<()> {
        let mut players = self.current_state.players.lock().await;

        if let Some(player) = players.iter_mut().find(|p| p.socket == src) {
            player.ready = payload.ready != 0;
            debug!("Player {} is now ready: {}", player.index, player.ready);
        } else {
            warn!("Player with socket {:?} not found", src);
        }

        // Check if all players are ready
        let all_ready = players.iter().all(|p| p.ready);
        if all_ready {
            let mut current_match = self.current_state.current_match.lock().await;
            self.send_game_start(&mut players, &mut current_match).await?;

            let handler_copy = self.clone();
            tokio::spawn(async move {
                let target_interval = Duration::from_millis(16);
                let mut ticker = interval(target_interval);

                let mut last_tick = Instant::now();

                loop {
                    ticker.tick().await;
                    let mut current_match = handler_copy.current_state.current_match.lock().await;
                    let mut players = handler_copy.current_state.players.lock().await;
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_tick);

                    let drift = if elapsed > target_interval {
                        elapsed - target_interval
                    } else {
                        target_interval - elapsed
                    };

                    if players.iter().all(|player| player.inputs.len() >= 5) {
                        handler_copy.send_player_inputs(&mut players, &mut current_match).await;
                    }

                    if elapsed > target_interval {
                        println!("Tick drifted LATE by: {} ms", drift.as_secs_f64() * 1000.0);
                    } else {
                        println!("Tick drifted EARLY by: {} ms", drift.as_secs_f64() * 1000.0);
                    }

                    last_tick = now;
                }
            });
        }

        Ok(())
    }

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()> {
        let (mut current_match, mut players) = tokio::join!(
            self.current_state.current_match.lock(),
            self.current_state.players.lock()
        );
        //debug!("Player INPUT {:#?} socket {}", payload, src);

        let max_players = current_match.num_players;
        let max_ping = players.iter().map(|p| p.ping).max().unwrap_or(0);

        {
            if let Some(player) = players.iter_mut().find(|p| p.socket == src) {
                player.last_client_frame = payload.client_frame;

                for (i, &input) in payload.input_per_frame.iter().enumerate() {
                    let frame = payload.start_frame + i as u32;
                    player.inputs.insert(frame, input);
                }

                // Only update host's ping if max_players == 2
                // TODO: What should the host ping be if max_players > 2?
                if player.is_host {
                    if max_players == 2 {
                        println!("HOST PING:{}", max_ping);
                        player.ping = max_ping;
                    }
                    current_match.current_frame = player.last_client_frame;
                } else {
                    player.rift = self.calc_rift_variable_tick(
                        current_match.current_frame,
                        player.last_client_frame,
                        player.ping,
                    );
                    debug!("NONE");
                }
            }
        }

        Ok(())
    }

    async fn handle_player_input_ack(&self, payload: PlayerInputAckPayload, src: SocketAddr) -> anyhow::Result<()> {
        let mut players = self.current_state.players.lock().await;
        let player = players
            .iter_mut()
            .find(|p| p.socket == src)
            .ok_or_else(|| anyhow::anyhow!("Player with socket {:?} not found", src))?;
        // Update that client's view of acked frames
        for (i, &player_acked_frame) in payload.ack_frame.iter().enumerate() {
            if i < player.acked_frames.len() && player_acked_frame > 0 && player.acked_frames[i] < player_acked_frame {
                debug!("ACKED:{}|{}    --{}", player.acked_frames[i], player_acked_frame, src);
                player.acked_frames[i] = player_acked_frame;
            }
        }

        // Compute ping as RTT
        if let Some(ts) = player.pending_pings.remove(&payload.server_message_sequence_number) {
            player.ping = ts.elapsed().as_millis() as u16;
        }
        Ok(())
    }
}
