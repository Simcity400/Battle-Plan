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

    pub fn block_settings(self) -> crate::columnar_file::Settings {
        match self {
            Self::Low => crate::columnar_file::Settings::LOW,
            Self::High => crate::columnar_file::Settings::HIGH,
        }
    }
}
