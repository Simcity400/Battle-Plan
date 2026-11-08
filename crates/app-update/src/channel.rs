//! The update channel both apps follow, saved once for the user.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Nightly,
}

impl Channel {
    pub fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }
}

/// Absent fields follow the running build.
#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<Channel>,
}

fn path() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .ok_or("LOCALAPPDATA is not set, so the update channel has nowhere to be saved.")?;
    Ok(Path::new(&local).join("HOI4Replay").join("updates.json"))
}

/// The channel the user chose, if they chose one.
pub fn load() -> Result<Option<Channel>, String> {
    let path = path()?;
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Saved>(&bytes)
            .map(|saved| saved.channel)
            .map_err(|error| format!("{} could not be read: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{} could not be read: {error}", path.display())),
    }
}

pub fn save(channel: Channel) -> Result<(), String> {
    let path = path()?;
    let write = || -> io::Result<()> {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("no parent folder"))?,
        )?;
        let bytes = serde_json::to_vec_pretty(&Saved {
            channel: Some(channel),
        })?;
        let mut temporary = path.clone().into_os_string();
        temporary.push(format!(".{}.tmp", std::process::id()));
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, &path)
    };
    write().map_err(|error| format!("{} could not be saved: {error}", path.display()))
}
