//! Recorder-owned preferences shared with the replay app.

use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingCompression {
    #[default]
    Fast,
    Small,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct RecorderSettings {
    #[serde(default)]
    pub compression: RecordingCompression,
    // Settings clients update only their owned fields; future recorder GUI
    // preferences survive saves from an older replayer.
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}

pub fn directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HOI4Replay")
        .join("recorder")
}

impl RecorderSettings {
    pub fn path() -> PathBuf {
        directory().join("settings.json")
    }
    pub fn load() -> io::Result<Self> {
        Self::load_from(&Self::path())
    }
    pub fn load_from(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }
    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::path())
    }
    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, &serde_json::to_vec_pretty(self)?)
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("Missing settings directory"))?,
    )?;
    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos()
    ));
    let temporary = PathBuf::from(name);
    // If create_new refuses an existing temporary file, it belongs to another
    // write or an earlier interruption; never remove it as our own cleanup.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        crate::container::replace_file(&temporary, path)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_defaults_fast_and_persists_small_without_losing_old_fields() {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join("settings.json");
        assert_eq!(
            RecorderSettings::load_from(&path).unwrap().compression,
            RecordingCompression::Fast
        );
        fs::create_dir_all(&root).unwrap();
        fs::write(&path, br#"{"futureRecorderSetting":42}"#).unwrap();
        let mut settings = RecorderSettings::load_from(&path).unwrap();
        assert_eq!(settings.compression, RecordingCompression::Fast);
        settings.compression = RecordingCompression::Small;
        settings.save_to(&path).unwrap();
        let restored = RecorderSettings::load_from(&path).unwrap();
        assert_eq!(restored.compression, RecordingCompression::Small);
        assert_eq!(restored.other["futureRecorderSetting"], 42);
        settings.compression = RecordingCompression::Fast;
        settings.save_to(&path).unwrap();
        assert_eq!(
            RecorderSettings::load_from(&path).unwrap().compression,
            RecordingCompression::Fast
        );
        fs::remove_dir_all(root).unwrap();
    }
}
