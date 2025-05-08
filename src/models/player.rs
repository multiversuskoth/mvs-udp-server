use std::{collections::HashMap, net::SocketAddr, time::Instant};

#[derive(Debug, Clone)]
pub struct Player {
    pub index: u16,
    pub team_index: u16,
    pub socket: SocketAddr,
    pub pending_pings: HashMap<u32, Instant>,
    pub replied_pings: u32,
    pub ready: bool,
    pub connected: bool,
    pub ping: u16,
    pub ip: String,    // Added IP address
    pub port: u16,     // Added port
    pub is_host: bool, // Added isHost flag
    pub last_seq_received: u32,

    pub last_client_frame: u32,
    // How many frames of each player this client has acked
    pub acked_frames: Vec<u32>,
    pub rift: f32,
    pub inputs: HashMap<u32, u32>, // One map per player: frame → input
    pub missed_inputs: u32,
}
