//! Recorder-owned preferences shared with the replay app.

use serde::{Deserialize, Serialize};
use serde_json::Value;
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

/// What the next session of a campaign records into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NextLoad {
    /// Continue the campaign's current recording.
    #[default]
    Continue,
    /// Start a new recording; the earlier one stays as it is.
    NewFile,
}

/// One campaign's choices, keyed in [`RecorderSettings::campaigns`] by its
/// recorded campaign id.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignChoice {
    /// Record automatically; absent follows `record_every_campaign`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<bool>,
    #[serde(default)]
    pub next: NextLoad,
    /// Folder of the recording new sessions continue, relative to the save
    /// folder. Absent is the save folder itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Recorded country tag whose flag stands for the campaign.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl CampaignChoice {
    fn is_empty(&self) -> bool {
        self.record.is_none()
            && self.next == NextLoad::Continue
            && self.folder.is_none()
            && self.flag.is_none()
            && self.other.is_empty()
    }
}

fn enabled() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderSettings {
    #[serde(default)]
    pub compression: RecordingCompression,
    #[serde(default = "enabled")]
    pub record_every_campaign: bool,
    /// Where recordings are saved; absent is the recorder's default folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_folder: Option<PathBuf>,
    /// The Paradox user folder; absent finds it automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_folder: Option<PathBuf>,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default = "enabled")]
    pub start_with_replay: bool,
    #[serde(default = "enabled")]
    pub game_score: bool,
    #[serde(default = "enabled")]
    pub operation_outcomes: bool,
    #[serde(default = "enabled")]
    pub politics: bool,
    #[serde(default)]
    pub campaigns: BTreeMap<String, CampaignChoice>,
    /// Display names by normalized recording path. File names never change.
    #[serde(default)]
    pub recording_names: BTreeMap<String, String>,
    // Settings clients update only their owned fields; preferences a newer
    // program adds survive saves from an older one.
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl Default for RecorderSettings {
    fn default() -> Self {
        Self {
            compression: RecordingCompression::Fast,
            record_every_campaign: true,
            save_folder: None,
            user_folder: None,
            start_with_windows: false,
            start_with_replay: true,
            game_score: true,
            operation_outcomes: true,
            politics: true,
            campaigns: BTreeMap::new(),
            recording_names: BTreeMap::new(),
            other: BTreeMap::new(),
        }
    }
}

fn local_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("HOI4Replay")
}

pub fn directory() -> PathBuf {
    local_data().join("recorder")
}

/// Where recordings are saved and listed when no folder was chosen.
pub fn default_save_folder() -> PathBuf {
    local_data().join("captures")
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

    /// Load the current file, apply `change` and save it, holding a short OS
    /// lock so the recorder and the replay app never undo each other's fields.
    pub fn update<T>(change: impl FnOnce(&mut Self) -> T) -> io::Result<(Self, T)> {
        Self::update_at(&Self::path(), change)
    }
    pub fn update_at<T>(path: &Path, change: impl FnOnce(&mut Self) -> T) -> io::Result<(Self, T)> {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| io::Error::other("Missing settings directory"))?,
        )?;
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(PathBuf::from(lock_path))?;
        lock.lock()?;
        let mut settings = Self::load_from(path)?;
        let result = change(&mut settings);
        settings.campaigns.retain(|_, choice| !choice.is_empty());
        settings.save_to(path)?;
        Ok((settings, result))
    }

    /// The recordings folder both programs use: the recorder saves into it and
    /// the replay app lists it.
    pub fn effective_save_folder(&self) -> PathBuf {
        self.save_folder.clone().unwrap_or_else(default_save_folder)
    }

    /// Whether a campaign records automatically.
    pub fn records_campaign(&self, campaign: &str) -> bool {
        self.campaigns
            .get(campaign)
            .and_then(|choice| choice.record)
            .unwrap_or(self.record_every_campaign)
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

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-settings-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn preference_defaults_fast_and_persists_small_without_losing_old_fields() {
        let root = scratch("compression");
        let path = root.join("settings.json");
        assert_eq!(
            RecorderSettings::load_from(&path).unwrap().compression,
            RecordingCompression::Fast
        );
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

    #[test]
    fn old_files_take_the_documented_defaults() {
        let root = scratch("defaults");
        let path = root.join("settings.json");
        fs::write(&path, br#"{"compression":"small"}"#).unwrap();
        let settings = RecorderSettings::load_from(&path).unwrap();
        assert_eq!(settings.compression, RecordingCompression::Small);
        assert!(settings.record_every_campaign);
        assert!(settings.start_with_replay && !settings.start_with_windows);
        assert!(settings.game_score && settings.operation_outcomes && settings.politics);
        assert!(settings.save_folder.is_none() && settings.user_folder.is_none());
        assert!(settings.campaigns.is_empty() && settings.recording_names.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn campaign_choices_override_the_global_default_and_keep_unknown_fields() {
        let root = scratch("campaigns");
        let path = root.join("settings.json");
        fs::write(
            &path,
            br#"{"recordEveryCampaign":false,"campaigns":{"a":{"record":true,"futureChoice":"kept"},"b":{"next":"newFile","folder":"b (2)"}}}"#,
        )
        .unwrap();
        let (settings, ()) = RecorderSettings::update_at(&path, |settings| {
            settings.campaigns.entry("c".into()).or_default();
        })
        .unwrap();
        assert!(settings.records_campaign("a"));
        assert!(!settings.records_campaign("b"));
        assert!(!settings.records_campaign("unknown"));
        assert_eq!(settings.campaigns["b"].next, NextLoad::NewFile);
        assert_eq!(settings.campaigns["b"].folder.as_deref(), Some("b (2)"));
        // An entry with no choices is not kept.
        assert!(!settings.campaigns.contains_key("c"));
        let restored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored["campaigns"]["a"]["futureChoice"], "kept");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn concurrent_updates_each_keep_the_other_writers_fields() {
        let root = scratch("concurrent");
        let path = root.join("settings.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writers: Vec<_> = (0..2)
            .map(|writer| {
                let path = path.clone();
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    for index in 0..25 {
                        RecorderSettings::update_at(&path, |settings| {
                            if writer == 0 {
                                settings
                                    .recording_names
                                    .insert(format!("c:/{index}.battleplan"), format!("{index}"));
                            } else {
                                settings
                                    .campaigns
                                    .entry(format!("{index}"))
                                    .or_default()
                                    .record = Some(false);
                            }
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let settings = RecorderSettings::load_from(&path).unwrap();
        assert_eq!(settings.recording_names.len(), 25);
        assert_eq!(settings.campaigns.len(), 25);
        fs::remove_dir_all(root).unwrap();
    }
}
