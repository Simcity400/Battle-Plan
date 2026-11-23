use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    #[default]
    Low,
    High,
}

impl Compression {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "low" => Some(Self::Low),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    pub fn level(self) -> i32 {
        self.block_settings().level
    }

    pub fn block_settings(self) -> Settings {
        match self {
            Self::Low => Settings::LOW,
            Self::High => Settings::HIGH,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub level: i32,
}

impl Settings {
    pub const HIGH: Self = Self { level: 19 };
    pub const LOW: Self = Self { level: 7 };
}
