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

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Saved {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<Channel>,
}

fn path() -> Result<PathBuf, String> {
    let local = std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .ok_or("LOCALAPPDATA isn’t set, so the update channel has nowhere to be saved.")?;
    Ok(Path::new(&local).join("BattlePlan").join("updates.json"))
}

pub fn load() -> Result<Option<Channel>, String> {
    let path = path()?;
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Saved>(&bytes)
            .map(|saved| saved.channel)
            .map_err(|error| format!("{} couldn’t be read: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{} couldn’t be read: {error}", path.display())),
    }
}

pub fn save(channel: Channel, write: impl FnOnce(&Path, &[u8]) -> io::Result<Option<String>>) -> Result<Option<String>, String> {
    save_to(&path()?, channel, write)
}

fn save_to(path: &Path, channel: Channel, write: impl FnOnce(&Path, &[u8]) -> io::Result<Option<String>>) -> Result<Option<String>, String> {
    serde_json::to_vec_pretty(&Saved { channel: Some(channel) })
        .map_err(io::Error::from)
        .and_then(|bytes| write(path, &bytes))
        .map_err(|error| format!("{} couldn’t be saved: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_is_saved_only_through_the_hosts_admitted_write() {
        let path = Path::new("updates.json");
        let refused = save_to(path, Channel::Nightly, |_, _| Err(io::Error::new(io::ErrorKind::Interrupted, "BattlePlan is closing, so nothing more is written.")));
        assert_eq!(refused, Err("updates.json couldn’t be saved: BattlePlan is closing, so nothing more is written.".to_owned()), "a channel chosen once BattlePlan closes is refused by name");
        let mut written = None;
        let warning = save_to(path, Channel::Nightly, |to, bytes| {
            written = Some((to.to_path_buf(), bytes.to_vec()));
            Ok(None)
        })
        .unwrap();
        assert_eq!(warning, None);
        let (to, bytes) = written.unwrap();
        assert_eq!(to, path);
        assert_eq!(serde_json::from_slice::<Saved>(&bytes).unwrap().channel, Some(Channel::Nightly));
    }
}
