// Client message serialization and deserialization

use std::io::{Cursor, Read};

use anyhow::{anyhow, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use crate::message_types::client_messages::{
    ClientHeader, GameMatchData, ClientMessageType, ClientPayload, PlayerData, DisconnectingPayload, PlayerInputPayload, MatchResultPayload,
    PlayerConnectionPaylod, PlayerDisconnectedAckPayload, PlayerInputAckPayload, PongPayload, ReadyForMatchPayload, UdpClientMessage,
    CLIENT_HEADER_SIZE,
};
use crate::message_types::server_messages::{ServerMessagePayload, UdpServerMessage};

pub fn parse_client_message(buf: &[u8]) -> Result<UdpClientMessage> {
    if buf.len() < CLIENT_HEADER_SIZE {
        return Err(anyhow!(
            "Buffer too small for client header: got {}, need ≥{}",
            buf.len(),
            CLIENT_HEADER_SIZE
        ));
    }

    let mut cursor = Cursor::new(buf);

    // Read header
    let type_byte = cursor.read_u8()?;
    let sequence = cursor.read_u32::<LittleEndian>()?;

    let msg_type = ClientMessageType::from(type_byte);
    let header = ClientHeader { type_: msg_type, sequence };

    // Read payload based on message type
    let payload = match msg_type {
        ClientMessageType::PlayerConnection => {
            let message_version = cursor.read_u16::<LittleEndian>()?;

            // Player config data
            let team_id = cursor.read_u16::<LittleEndian>()?;
            let player_index = cursor.read_u16::<LittleEndian>()?;

            // Read strings as zero-terminated UTF-8
            let read_string = |cursor: &mut Cursor<&[u8]>, max_len: usize| -> Result<String> {
                let mut buffer = vec![0u8; max_len];
                cursor.read_exact(&mut buffer)?;

                // Find the terminating zero byte
                let zero_pos = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
                let string_bytes = &buffer[0..zero_pos];

                Ok(String::from_utf8_lossy(string_bytes).to_string())
            };

            let match_id = read_string(&mut cursor, 25)?;
            let key = read_string(&mut cursor, 45)?;
            let environment_id = read_string(&mut cursor, 25)?;

            ClientPayload::PlayerConnectionPaylod(PlayerConnectionPaylod {
                message_version,
                player_data: PlayerData { team_id, player_index },
                match_data: GameMatchData {
                    match_id,
                    key,
                    environment_id,
                },
            })
        }

        ClientMessageType::PlayerInput => {
            let start_frame = cursor.read_u32::<LittleEndian>()?;
            let client_frame = cursor.read_u32::<LittleEndian>()?;
            let num_frames = cursor.read_u8()?;
            let num_checksums = cursor.read_u8()?;

            let mut input_per_frame = Vec::with_capacity(num_frames as usize);
            for _ in 0..num_frames {
                input_per_frame.push(cursor.read_u32::<LittleEndian>()?);
            }

            let mut checksum_per_frame = Vec::with_capacity(num_checksums as usize);
            for _ in 0..num_checksums {
                checksum_per_frame.push(cursor.read_u32::<LittleEndian>()?);
            }

            ClientPayload::PlayerInputPayload(PlayerInputPayload {
                start_frame,
                client_frame,
                num_frames,
                num_checksums,
                input_per_frame,
                checksum_per_frame,
            })
        }

        ClientMessageType::PlayerInputAck => {
            let num_players = cursor.read_u8()?;

            let mut ack_frame = Vec::with_capacity(num_players as usize);
            for _ in 0..num_players {
                ack_frame.push(cursor.read_u32::<LittleEndian>()?);
            }

            let server_message_sequence_number = cursor.read_u32::<LittleEndian>()?;

            ClientPayload::PlayerInputAckPayload(PlayerInputAckPayload {
                num_players,
                ack_frame,
                server_message_sequence_number,
            })
        }

        ClientMessageType::MatchResult => {
            let num_players = cursor.read_u8()?;
            let last_frame_checksum = cursor.read_u32::<LittleEndian>()?;
            let winning_team_index = cursor.read_u8()?;

            ClientPayload::MatchResultPayload(MatchResultPayload {
                num_players,
                last_frame_checksum,
                winning_team_index,
            })
        }

        ClientMessageType::Pong => {
            let server_message_sequence_number = cursor.read_u32::<LittleEndian>()?;
            ClientPayload::PongPayload(PongPayload {
                server_message_sequence_number,
            })
        }

        ClientMessageType::Disconnecting => {
            let reason = cursor.read_u8()?;
            ClientPayload::DisconnectingPayload(DisconnectingPayload { reason })
        }

        ClientMessageType::PlayerDisconnectedAck => {
            let player_disconnected_array_index = cursor.read_u8()?;
            ClientPayload::PlayerDisconnectedAckPayload(PlayerDisconnectedAckPayload {
                player_disconnected_array_index,
            })
        }

        ClientMessageType::ReadyForMatch => {
            let ready = cursor.read_u8()?;
            ClientPayload::ReadyForMatchPayload(ReadyForMatchPayload { ready })
        }
    };

    Ok(UdpClientMessage { header, payload })
}

pub fn serialize_server_message(message: &UdpServerMessage, max_players: usize) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();

    // Write header
    buffer.write_u8(message.header.type_ as u8)?;
    buffer.write_u32::<LittleEndian>(message.header.sequence)?;

    // Write payload based on message type
    match &message.payload {
        ServerMessagePayload::PlayerConnection(data) => {
            buffer.write_u8(data.success)?;
            buffer.write_u8(data.num_players)?;
            buffer.write_u8(data.player_index)?;
            buffer.write_u32::<LittleEndian>(data.match_duration)?;
            buffer.write_u8(data.unused_0)?;
            buffer.write_u8(data.unused_1)?;
        }

        ServerMessagePayload::PlayerInputs(data) => {
            buffer.write_u8(data.num_players)?;

            // StartFrame[maxPlayers]
            for i in 0..max_players {
                let sf = data.start_frame.get(i).copied().unwrap_or(0);
                buffer.write_u32::<LittleEndian>(sf)?;
            }

            // NumFrames[maxPlayers]
            for i in 0..max_players {
                let nf = data.num_frames.get(i).copied().unwrap_or(0);
                buffer.write_u8(nf)?;
            }

            // Fixed fields
            buffer.write_u16::<LittleEndian>(data.num_predicted_overrides)?;
            buffer.write_u16::<LittleEndian>(data.unused_0)?;
            buffer.write_u16::<LittleEndian>(data.ping)?;
            buffer.write_i16::<LittleEndian>(data.packets_loss_percent)?;

            // Rift (float * 100, rounded)
            let rift_i16 = (data.rift * 100.0).round() as i16;
            buffer.write_i16::<LittleEndian>(rift_i16)?;

            buffer.write_u32::<LittleEndian>(data.unused_1)?;

            // InputPerFrame[player][frame]
            for pi in 0..max_players {
                let default_inputs = vec![];
                let player_inputs = data.input_per_frame.get(pi).unwrap_or(&default_inputs);
                let num_frames = data.num_frames.get(pi).copied().unwrap_or(0);

                for f in 0..num_frames as usize {
                    let input_val = player_inputs.get(f).copied().unwrap_or(0);
                    buffer.write_u32::<LittleEndian>(input_val)?;
                }
            }
        }

        ServerMessagePayload::RequestPing(data) => {
            buffer.write_i16::<LittleEndian>(data.ping)?;
            buffer.write_i16::<LittleEndian>(data.packets_loss_percent)?;
        }

        ServerMessagePayload::Kick(data) => {
            buffer.write_u16::<LittleEndian>(data.reason)?;
            buffer.write_u32::<LittleEndian>(data.param1)?;
        }

        ServerMessagePayload::PlayerGetReady(data) => {
            buffer.write_u8(data.num_players)?;

            // PlayerConfigValues = [0, 257, 512, 769] from TypeScript code
            let player_config_values = [0u16, 257, 512, 769];
            for i in 0..max_players {
                buffer.write_u16::<LittleEndian>(player_config_values[i % player_config_values.len()])?;
            }
        }

        ServerMessagePayload::PlayerDisconnected(data) => {
            buffer.write_u8(data.player_index)?;
            buffer.write_u8(data.should_ai_take_control)?;
            buffer.write_u32::<LittleEndian>(data.ai_take_control_frame)?;
            buffer.write_u16::<LittleEndian>(data.player_disconnected_array_index)?;
        }

        ServerMessagePayload::StartGame(_) => {
            // Empty payload
        }
    };

    Ok(buffer)
}
