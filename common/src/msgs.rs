use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct RTMsg {
    pub seq: i64,
    pub is_audio: bool,

    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub enum KeyEvent {
    Key { letter: char, state: bool },
    Mouse { x: f64, y: f64 },
    Click { button: i32, state: bool },
}
