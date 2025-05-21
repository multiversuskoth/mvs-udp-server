use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ServerMessageType {
    PlayerConnection = 1,
    StartGame = 2,
    Unknown3 = 3,
    PlayerInputs = 4,
    RequestPing = 6,
    Unknown = 7,
    Kick = 8,
    Unknown1 = 9,
    PlayerGetReady = 10,
    PlayerDisconnected = 11,
    Unknown2 = 12,
    MVSI_HOLE_PUNCH = 13
}

impl From<u8> for ServerMessageType {
    fn from(value: u8) -> Self {
        match value {
            1 => ServerMessageType::PlayerConnection,
            2 => ServerMessageType::StartGame,
            3 => ServerMessageType::Unknown3,
            4 => ServerMessageType::PlayerInputs,
            6 => ServerMessageType::RequestPing,
            7 => ServerMessageType::Unknown,
            8 => ServerMessageType::Kick,
            9 => ServerMessageType::Unknown1,
            10 => ServerMessageType::PlayerGetReady,
            11 => ServerMessageType::PlayerDisconnected,
            12 => ServerMessageType::Unknown2,
            13 => ServerMessageType::MVSI_HOLE_PUNCH,
            _ => panic!("Unknown message type: {}", value),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Header {
    pub type_: ServerMessageType,
    pub sequence: u32,
}

#[derive(Debug, Clone)]
pub struct PlayerConnection {
    pub success: u8,
    pub num_players: u8,
    pub player_index: u8,
    pub match_duration: u32,
    pub unused_0: u8,
    pub unused_1: u8,
}

#[derive(Debug, Clone)]
pub struct PlayerInputs {
    pub num_players: u8,
    pub start_frame: Vec<u32>, // length max_players
    pub num_frames: Vec<u8>,   // length max_players
    pub num_predicted_overrides: u16,
    pub unused_0: u16,
    pub ping: u16,
    pub packets_loss_percent: i16,
    pub rift: f32,
    pub unused_1: u32,
    pub input_per_frame: Vec<Vec<u32>>, // [player][frame] = uint32
}

#[derive(Debug, Clone)]
pub struct RequestPing {
    pub ping: u16,
    pub packets_loss_percent: u16,
}

#[derive(Debug, Clone)]
pub struct Kick {
    pub reason: u16,
    pub param1: u32,
}

#[derive(Debug, Clone)]
pub struct PlayerGetReady {
    pub num_players: u8,
    pub raw_data: Vec<u8>, // Raw buffer for player config data
}

#[derive(Debug, Clone)]
pub struct PlayerDisconnected {
    pub player_index: u8,
    pub should_ai_take_control: u8,
    pub ai_take_control_frame: u32,
    pub player_disconnected_array_index: u16,
}

#[derive(Debug, Clone)]
pub struct Empty {}

#[derive(Debug, Clone)]
pub enum ServerMessagePayload {
    PlayerConnection(PlayerConnection),
    PlayerInputs(PlayerInputs),
    RequestPing(RequestPing),
    Kick(Kick),
    PlayerGetReady(PlayerGetReady),
    PlayerDisconnected(PlayerDisconnected),
    StartGame(Empty),
    Empty(),
}

#[derive(Debug, Clone)]
pub struct UdpServerMessage {
    pub header: Header,
    pub payload: ServerMessagePayload,
}

// Constants
pub const HEADER_SIZE: usize = 5; // 1 byte type + 4 bytes sequence
