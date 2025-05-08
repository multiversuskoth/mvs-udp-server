pub struct GameMatch {
    pub match_id: String,
    pub match_key: String,
    pub num_players: u8,
    pub sequence_number: u32, // Added sequence number
    pub match_duration: u32,  // Added match duration
    pub current_frame: u32,   // Added current frame
    pub ready: bool,
}

impl GameMatch {
    pub fn new() -> Self {
        GameMatch {
            match_id: "".to_string(),
            match_key: "".to_string(),
            num_players: 0,     // Initialize max players
            sequence_number: 0, // Initialize sequence number
            match_duration: 0,  // Initialize match duration
            current_frame: 0,   // Initialize current frame
            ready: false,       // Initialize ready state
        }
    }
}
