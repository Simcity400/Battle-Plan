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
        match self {
            Self::Low => 3,
            Self::High => 19,
        }
    }

    pub(crate) fn selection_level(self) -> i32 {
        match self {
            Self::Low => 3,
            Self::High => 9,
        }
    }
}
