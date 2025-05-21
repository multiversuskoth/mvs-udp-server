#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClientMessageType {
    PlayerConnection = 1,
    PlayerInput = 2,
    PlayerInputAck = 3,
    MatchResult = 4,
    Pong = 5,
    Disconnecting = 6,
    PlayerDisconnectedAck = 7,
    ReadyForMatch = 8,
    MVSI_HOLE_PUNCH = 13,
}

impl From<u8> for ClientMessageType {
    fn from(value: u8) -> Self {
        match value {
            1 => ClientMessageType::PlayerConnection,
            2 => ClientMessageType::PlayerInput,
            3 => ClientMessageType::PlayerInputAck,
            4 => ClientMessageType::MatchResult,
            5 => ClientMessageType::Pong,
            6 => ClientMessageType::Disconnecting,
            7 => ClientMessageType::PlayerDisconnectedAck,
            8 => ClientMessageType::ReadyForMatch,
            13 => ClientMessageType::MVSI_HOLE_PUNCH,
            _ => panic!("Unknown client message type: {}", value),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClientHeader {
    pub type_: ClientMessageType,
    pub sequence: u32,
}

#[derive(Debug, Clone)]
pub struct PlayerData {
    pub team_id: u16,
    pub player_index: u16,
}

#[derive(Debug, Clone)]
pub struct GameMatchData {
    pub match_id: String,       // up to 25 chars, zero‑terminated
    pub key: String,            // up to 45 chars
    pub environment_id: String, // up to 25 chars
}

#[derive(Debug, Clone)]
pub struct PlayerConnectionPaylod {
    pub message_version: u16,
    pub player_data: PlayerData,
    pub match_data: GameMatchData,
}

#[derive(Debug, Clone)]
pub struct PlayerInputPayload {
    pub start_frame: u32,
    pub client_frame: u32,
    pub num_frames: u8,
    pub num_checksums: u8,
    pub input_per_frame: Vec<u32>,
    pub checksum_per_frame: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct PlayerInputAckPayload {
    pub num_players: u8,
    pub ack_frame: Vec<u32>,
    pub server_message_sequence_number: u32,
}

#[derive(Debug, Clone)]
pub struct MatchResultPayload {
    pub num_players: u8,
    pub last_frame_checksum: u32,
    pub winning_team_index: u8,
}

#[derive(Debug, Clone)]
pub struct PongPayload {
    pub server_message_sequence_number: u32,
}

#[derive(Debug, Clone)]
pub struct DisconnectingPayload {
    pub reason: u8,
}

#[derive(Debug, Clone)]
pub struct PlayerDisconnectedAckPayload {
    pub player_disconnected_array_index: u8,
}

#[derive(Debug, Clone)]
pub struct ReadyForMatchPayload {
    pub ready: u8,
}

#[derive(Debug, Clone)]
pub enum ClientPayload {
    PlayerConnectionPaylod(PlayerConnectionPaylod),
    PlayerInputPayload(PlayerInputPayload),
    PlayerInputAckPayload(PlayerInputAckPayload),
    MatchResultPayload(MatchResultPayload),
    PongPayload(PongPayload),
    DisconnectingPayload(DisconnectingPayload),
    PlayerDisconnectedAckPayload(PlayerDisconnectedAckPayload),
    ReadyForMatchPayload(ReadyForMatchPayload),
    MVSI_HOLE_PUNCH()
}

#[derive(Debug, Clone)]
pub struct UdpClientMessage {
    pub header: ClientHeader,
    pub payload: ClientPayload,
}

// Constants
pub const CLIENT_HEADER_SIZE: usize = 5; // 1 byte type + 4 bytes sequence
