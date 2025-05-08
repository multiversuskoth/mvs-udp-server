use std::{collections::HashMap, net::SocketAddr, time::Instant};

use log::{debug, error, warn};
use serde_json::json;
use tokio::sync::MutexGuard;

use crate::{
    message_types::{
        client_messages::{PlayerConnectionPaylod, PlayerInputAckPayload, PlayerInputPayload, PongPayload, ReadyForMatchPayload},
        server_messages::{PlayerConnection, RequestPing, ServerMessagePayload, ServerMessageType},
    },
    models::{game_match::GameMatch, player::Player},
    P2PRollbackServer,
};

pub trait MessageHandler {
    async fn handle_new_connection(&self, payload: PlayerConnectionPaylod, src: SocketAddr) -> anyhow::Result<()>;

    async fn ping_players(&self) -> anyhow::Result<()>;
    async fn handle_player_pong_response(&self, payload: PongPayload, src: SocketAddr) -> anyhow::Result<()>;
    async fn handle_player_ready(&self, payload: ReadyForMatchPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn handle_player_input_ack(&self, payload: PlayerInputAckPayload, src: SocketAddr) -> anyhow::Result<()>;

    async fn register_match(&self, payload: &PlayerConnectionPaylod, current_match: &mut MutexGuard<'_, GameMatch>);
    async fn fetch_and_register_players(
        &self,
        payload: &PlayerConnectionPaylod,
        src: SocketAddr,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    );
}

impl MessageHandler for P2PRollbackServer {
    async fn register_match(&self, payload: &PlayerConnectionPaylod, current_match: &mut MutexGuard<'_, GameMatch>) {
        if !current_match.ready {
            let response = self
                .http_client
                .post(format!("{}/register", self.http_endpoint))
                .json(&json!({
                    "match_id": payload.match_data.match_id,
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
                        current_match.match_duration = match_data["match_duration"].as_u64().unwrap_or(0) as u32;
                        current_match.num_players = max_players as u8;
                    }
                }
                Err(e) => {
                    error!("Failed to register match: {}", e);
                }
            }
        }
    }

    async fn fetch_and_register_players(
        &self,
        payload: &PlayerConnectionPaylod,
        src: SocketAddr,
        players: &mut MutexGuard<'_, Vec<Player>>,
        current_match: &mut MutexGuard<'_, GameMatch>,
    ) {
        let response = self
            .http_client
            .post(&self.http_endpoint)
            .json(&json!({
                "match_id": payload.match_data.match_id,
                "key": payload.match_data.key,
                "player_index": payload.player_data.player_index,
            }))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if let Ok(player_data) = resp.json::<serde_json::Value>().await {
                    if let (Some(index), Some(ip), Some(port), Some(is_host)) = (
                        player_data["player_index"].as_u64(),
                        player_data["ip"].as_str(),
                        player_data["port"].as_u64(),
                        player_data["isHost"].as_bool(),
                    ) {
                        if players.iter().any(|p| p.index == index as u16 && p.ip == ip && p.port == port as u16) {
                            debug!("Player already exists: index={}, ip={}, port={}", index, ip, port);
                            return;
                        }

                        let socket = if src.ip() == std::net::IpAddr::V4("127.0.0.1".parse().unwrap()) {
                            src
                        } else {
                            SocketAddr::new(ip.parse().unwrap(), port as u16)
                        };

                        let player = Player {
                            index: index as u16,
                            team_index: payload.player_data.team_id,
                            socket,
                            pending_pings: HashMap::new(),
                            replied_pings: 0,
                            ready: false,
                            connected: true,
                            ping: 0,
                            last_client_frame: 0,
                            rift: 0.0,
                            ip: ip.to_string(),
                            port: port as u16,
                            acked_frames: vec![],
                            inputs: HashMap::new(),
                            missed_inputs: 0,
                            is_host,
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
                        self.send_message(ServerMessageType::PlayerConnection, msg, &player_socket).await;
                    }
                }
            }
            Err(e) => {
                error!("Failed to fetch players from HTTP endpoint: {}", e);
            }
        }
    }

    async fn handle_new_connection(&self, payload: PlayerConnectionPaylod, src: SocketAddr) -> anyhow::Result<()> {
        let mut current_match = self.current_state.current_match.lock().await;
        let mut players = self.current_state.players.lock().await;
        self.register_match(&payload, &mut current_match).await;
        self.fetch_and_register_players(&payload, src, &mut players, &mut current_match).await;

        if current_match.ready {
            let all_connected = players.iter().filter(|p| p.connected).count() == current_match.num_players as usize;
            if all_connected {
                self.ping_players().await?;
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

    async fn ping_players(&self) -> anyhow::Result<()> {
        let max_pings = 10;
        loop {
            {
                let (current_match, mut players) = tokio::join!(self.current_state.current_match.lock(), self.current_state.players.lock());

                // check if all players have been pinged the max_pings times
                let all_pinged = players.iter().all(|player| player.replied_pings >= max_pings);
                if all_pinged {
                    break;
                }

                let sequence_number = current_match.sequence_number.clone();
                for player in players.iter_mut() {
                    let msg = ServerMessagePayload::RequestPing(RequestPing {
                        ping: player.ping as i16,
                        packets_loss_percent: 0,
                    });
                    player.pending_pings.insert(sequence_number, Instant::now());
                    self.send_message(ServerMessageType::PlayerConnection, msg, &player.socket).await;
                }
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        self.send_players_get_ready().await?;

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
            drop(players); // Drop the lock before sending the message
            self.send_game_start().await?;
        }

        Ok(())
    }

    async fn handle_player_input(&self, payload: PlayerInputPayload, src: SocketAddr) -> anyhow::Result<()> {
        let (current_match, mut players) = tokio::join!(self.current_state.current_match.lock(), self.current_state.players.lock());

        let max_players = current_match.num_players;
        let max_ping = players.iter().map(|p| p.ping).max().unwrap_or(0);

        {
            for player in players.iter_mut() {
                // Only update host's ping if max_players == 2
                // TODO: What should the host ping be if max_players > 2?
                if player.is_host {
                    if max_players == 2 {
                        player.ping = max_ping;
                    }
                }

                if player.socket == src {
                    player.last_client_frame = payload.client_frame;

                    for (i, &input) in payload.input_per_frame.iter().enumerate() {
                        let frame = payload.start_frame + i as u32;
                        player.inputs.insert(frame, input);
                    }

                    self.send_player_inputs().await?;

                    // TODO: UPDATE PLAYER RIFT
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
