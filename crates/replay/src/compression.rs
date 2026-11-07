use serde::{Deserialize, Serialize};

/// How hard the recorder compresses what it writes. It never runs in the game: Low makes
/// larger files with less of the recorder's CPU, High the smallest files with more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    Low,
    #[default]
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
        match self {
            Self::Low => 3,
            Self::High => 19,
        }
    }

    /// The columnar writer's settings for blocks and restarts.
    pub fn block_settings(self) -> crate::columnar_file::Settings {
        match self {
            Self::Low => crate::columnar_file::Settings::LOW,
            Self::High => crate::columnar_file::Settings::HIGH,
        }
    }

    pub(crate) fn selection_level(self) -> i32 {
        match self {
            Self::Low => 3,
            Self::High => 9,
        }
    }
}
