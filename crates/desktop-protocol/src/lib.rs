//! Small control messages between the desktop host and its native viewer.
//!
//! Map pixels and replay tables never travel through this pipe. Handles and
//! branch identifiers use decimal strings so every bit survives JavaScript.

use serde::{Deserialize, Serialize};
use std::{io, path::Path, time::UNIX_EPOCH};

pub const EVENT_PREFIX: &str = "BATTLE_PLAN_DESKTOP ";
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeCommand {
    Visibility {
        visible: bool,
    },
    Bounds {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Open {
        path: String,
        revision: String,
    },
    Seek {
        path: String,
        revision: String,
        branch: String,
        hour: i32,
    },
    Close,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeEvent {
    Ready {
        hwnd: String,
    },
    State {
        path: Option<String>,
        branch: Option<String>,
        hour: Option<i32>,
        revision: Option<String>,
        error: Option<String>,
    },
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ViewerState {
    pub path: Option<String>,
    pub branch: Option<String>,
    pub hour: Option<i32>,
    pub revision: Option<String>,
}

/// A cheap file-change marker shared by both processes. This is length and
/// modification time, not a cryptographic content identity or asset witness.
pub fn replay_revision(path: impl AsRef<Path>) -> io::Result<String> {
    let metadata = path.as_ref().metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Recording is not a file",
        ));
    }
    let modified = metadata.modified()?;
    let stamp = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => format!("{}:{}", duration.as_secs(), duration.subsec_nanos()),
        Err(error) => {
            let duration = error.duration();
            format!("-{}:{}", duration.as_secs(), duration.subsec_nanos())
        }
    };
    Ok(format!("{}:{stamp}", metadata.len()))
}

/// The frontend measures its content region in physical client pixels.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ViewBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ViewBounds {
    pub fn validate(self) -> Result<Self, &'static str> {
        if self.x < 0 || self.y < 0 || self.width == 0 || self.height == 0 {
            return Err("The map area must have a positive size inside the window");
        }
        if self.x > 32_768 || self.y > 32_768 || self.width > 32_768 || self.height > 32_768 {
            return Err("The map area exceeds supported window bounds");
        }
        Ok(self)
    }

    pub fn command(self) -> NativeCommand {
        NativeCommand::Bounds {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_branch_survives_the_wire() {
        let command = NativeCommand::Seek {
            path: "campaign.battleplan".into(),
            revision: "snapshot".into(),
            branch: u64::MAX.to_string(),
            hour: 60_832_633,
        };
        let json = serde_json::to_string(&command).unwrap();
        assert!(json.contains("\"18446744073709551615\""));
        let NativeCommand::Seek {
            path,
            revision,
            branch,
            hour,
        } = serde_json::from_str(&json).unwrap()
        else {
            panic!("wrong command");
        };
        assert_eq!(path, "campaign.battleplan");
        assert_eq!(revision, "snapshot");
        assert_eq!(branch.parse::<u64>().unwrap(), u64::MAX);
        assert_eq!(hour, 60_832_633);
    }

    #[test]
    fn invalid_geometry_is_rejected_before_native_calls() {
        for bounds in [
            ViewBounds {
                x: -1,
                y: 0,
                width: 100,
                height: 100,
            },
            ViewBounds {
                x: 80,
                y: 0,
                width: 0,
                height: 100,
            },
            ViewBounds {
                x: 80,
                y: 0,
                width: u32::MAX,
                height: 100,
            },
        ] {
            assert!(bounds.validate().is_err());
        }
        assert!(
            ViewBounds {
                x: 100,
                y: 0,
                width: 1500,
                height: 900
            }
            .validate()
            .is_ok()
        );
    }
}
