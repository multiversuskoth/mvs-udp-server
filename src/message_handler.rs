use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::bail;
use log::{debug, error, info, warn};
use serde_json::json;
use tokio::{sync::MutexGuard, time::sleep};

use crate::{
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
    P2PRollbackServer,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct MVSIPlayer {
    player_index: u64,
    ip: String,
    port: u64,
    is_host: bool,
}

pub trait MessageHandler {
    async fn handle_new_connection(&mut self, payload: PlayerConnectionPaylod, src: SocketAddr) -> anyhow::Result<()>;

    async fn ping_players(
        &self,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) -> anyhow::Result<()>;
    async fn handle_player_pong_response(&self, payload: PongPayload, src: SocketAddr) -> anyhow::Result<()>;
    async fn handle_player_ready(&self, payload: ReadyForMatchPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input_ack(&self, payload: PlayerInputAckPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn register_match(&self, payload: &PlayerConnectionPaylod, current_match: &mut MutexGuard<'_, GameMatch>);
    async fn fetch_and_register_players(&mut self, payload: &PlayerConnectionPaylod, src: SocketAddr);
}

impl MessageHandler for P2PRollbackServer {
    async fn register_match(&self, payload: &PlayerConnectionPaylod, current_match: &mut MutexGuard<'_, GameMatch>) {
        if !current_match.ready {
            let response = self
                .http_client
                .post(format!("{}/mvsi_register", self.http_endpoint))
                .json(&json!({
                    "matchId": payload.match_data.match_id,
                    "key": payload.match_data.key
                }))
                .send()
                .await;

            match response {
                Ok(resp) => {
                    if let Ok(match_data) = resp.json::<serde_json::Value>().await {
                        let max_players = match_data["max_players"].as_u64().unwrap_or(2);
                        current_match.num_players = max_players as u8;
                        current_match.match_id = payload.match_data.match_id.clone();
                        current_match.match_key = payload.match_data.key.clone();
                        current_match.ready = true;
                        current_match.match_duration = match_data["matchDuration"].as_u64().unwrap_or(0) as u32;
                        current_match.num_players = max_players as u8;
                    }
                }
                Err(e) => {
                    error!("Failed to register match: {}", e);
                }
            }
        }
    }

    async fn fetch_and_register_players(&mut self, payload: &PlayerConnectionPaylod, src: SocketAddr) {
        let response = self
            .http_client
            .post(format!("{}/mvsi_match_players", self.http_endpoint))
            .json(&json!({
                "matchId": payload.match_data.match_id,
                "key": payload.match_data.key,
            }))
            .send()
            .await;
        match response {
            Ok(resp) => match resp.json::<Vec<MVSIPlayer>>().await {
                Ok(players_data) => {
                    let mut current_match = self.current_state.current_match.lock().await;
                    let mut players = self.current_state.players.lock().await;

                    let pDataCopy = players_data.clone();

                    if players
                        .iter()
                        .any(|p| p.index == payload.player_data.player_index as u16)
                    {
                        debug!(
                            "Player already exists: index={}, ip={}, port={}",
                            payload.player_data.player_index,
                            src.ip().to_string(),
                            src.port()
                        );
                        return;
                    }

                    if let Some(player_data) = players_data
                        .iter()
                        .find(|p| p.player_index == payload.player_data.player_index as u64)
                    {
                        let socket = if src.ip() == std::net::IpAddr::V4("127.0.0.1".parse().unwrap()) {
                            src
                        } else {
                            SocketAddr::new(player_data.ip.parse().unwrap(), player_data.port as u16)
                        };

                        let player = Player {
                            index: payload.player_data.player_index as u16,
                            team_index: payload.player_data.team_id,
                            socket,
                            pending_pings: HashMap::new(),
                            replied_pings: 0,
                            ready: false,
                            connected: true,
                            ping: 0,
                            last_client_frame: 0,
                            rift: 0.0,
                            ip: player_data.ip.to_string(),
                            port: player_data.port as u16,
                            acked_frames: vec![],
                            inputs: HashMap::new(),
                            missed_inputs: 0,
                            is_host: player_data.is_host,
                            last_seq_received: 0,
                        };
                        let player_socket = player.socket;
                        players.push(player);

                        let msg = ServerMessagePayload::PlayerConnection(PlayerConnection {
                            success: 0,
                            num_players: current_match.num_players as u8,
                            player_index: payload.player_data.player_index as u8,
                            match_duration: current_match.match_duration,
                            unused_0: 0,
                            unused_1: 0,
                        });
                        self.send_message(
                            ServerMessageType::PlayerConnection,
                            msg,
                            &player_socket,
                            &mut current_match,
                        )
                        .await;
                        info!("Player {} connected with {}", payload.player_data.player_index, src);
                        let mut count = 0;
                        if player_data.is_host {
                            loop {
                                if count == 10 {
                                    break;
                                }
                                for player_data_copy in &pDataCopy {
                                    if player_data_copy.player_index != payload.player_data.player_index as u64 {
                                        let target = SocketAddr::new(
                                            player_data_copy.ip.parse().unwrap(),
                                            player_data_copy.port as u16,
                                        );
                                        //info!("UDP Hole Punching {}:{}", player_data_copy.ip, player_data_copy.port);
                                        self.send_udp_hole_punch(&target, &mut current_match).await;
                                        sleep(Duration::from_millis(50)).await;
                                    }
                                }
                                count += 1;
                            }
                        } else {
                            if let Some(host_player_data) = pDataCopy.iter().find(|p| p.is_host) {
                                self.current_state.passthrough.store(true, Ordering::SeqCst);
                                let target =
                                    SocketAddr::new(host_player_data.ip.parse().unwrap(), host_player_data.port as u16);
                                self.current_state.hostSocket = Some(target);
                                loop {
                                    if count == 10 {
                                        break;
                                    }

                                    //info!("UDP Hole Punching {}:{}", host_player_data.ip, host_player_data.port);
                                    self.send_udp_hole_punch(&target, &mut current_match).await;
                                    sleep(Duration::from_millis(50)).await;

                                    count += 1;
                                }
                            }
                        }
                    }
                }
                Err(e) => println!("ERROR {}", e.to_string()),
            },
            Err(e) => {
                error!("Failed to fetch players from HTTP endpoint: {}", e);
            }
        }
    }

    async fn handle_new_connection(&mut self, payload: PlayerConnectionPaylod, src: SocketAddr) -> anyhow::Result<()> {
        {
            let mut current_match = self.current_state.current_match.lock().await;
            self.register_match(&payload, &mut current_match).await;
        }
        {
            self.fetch_and_register_players(&payload, src).await;
        }
        let mut current_match = self.current_state.current_match.lock().await;
        let mut players = self.current_state.players.lock().await;
        if current_match.ready {
            let all_connected = players.iter().filter(|p| p.connected).count() == current_match.num_players as usize;
            if all_connected {
                self.ping_players(&mut players, &mut current_match).await?;
            }
        }
        Ok(())
    }

    async fn handle_player_pong_response(&self, payload: PongPayload, src: SocketAddr) -> anyhow::Result<()> {
        let mut players = self.current_state.players.lock().await;

        // Find the player based on the source address
        if let Some(player) = players.iter_mut().find(|p| p.socket == src) {
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

    async fn ping_players(
        &self,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) -> anyhow::Result<()> {
        let max_pings = 10;
        loop {
            {
                // check if all players have been pinged the max_pings times
                let all_pinged = players.iter().all(|player| player.replied_pings >= max_pings);
                if all_pinged {
                    break;
                }

                let sequence_number = current_match.sequence_number.clone();
                for player in players.iter_mut() {
                    let msg = ServerMessagePayload::RequestPing(RequestPing {
                        ping: player.ping as u16,
                        packets_loss_percent: 0,
                    });
                    player.pending_pings.insert(sequence_number, Instant::now());
                    self.send_message(ServerMessageType::RequestPing, msg, &player.socket, current_match)
                        .await;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        self.send_players_get_ready(players, current_match).await?;

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
        }

        Ok(())
    }

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()> {
        let (mut current_match, mut players) = tokio::join!(
            self.current_state.current_match.lock(),
            self.current_state.players.lock()
        );

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
                        player.ping = max_ping;
                    }
                    current_match.current_frame = player.last_client_frame;
                    self.send_player_inputs(&mut players, &mut current_match).await?;
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
