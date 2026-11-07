//! Recorder-owned preferences shared with the replay app.

use replay_data::scope::CaptureMode;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

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
}

impl CampaignChoice {
    fn is_empty(&self) -> bool {
        self.record.is_none()
            && self.next == NextLoad::Continue
            && self.folder.is_none()
            && self.flag.is_none()
    }
}

fn enabled() -> bool {
    true
}

fn full() -> CaptureMode {
    CaptureMode::Full
}

/// A [`CaptureMode`] by its name.
mod capture_mode {
    use replay_data::scope::CaptureMode;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(mode: &CaptureMode, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(mode.name())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<CaptureMode, D::Error> {
        let name = String::deserialize(deserializer)?;
        CaptureMode::from_name(&name)
            .ok_or_else(|| D::Error::custom(format!("unknown capture mode {name:?}")))
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecorderSettings {
    /// Saved as `blockCompression`: a choice saved under `compression` was for the format before
    /// blocks, and loads as the default, High.
    #[serde(default, rename = "blockCompression")]
    pub compression: crate::compression::Compression,
    /// Auto record: campaigns record when they load, unless their own choice says otherwise.
    #[serde(default = "enabled")]
    pub record_every_campaign: bool,
    /// Auto save: otherwise recordings wait for Save or Discard.
    #[serde(default = "enabled")]
    pub auto_save: bool,
    /// Where recordings are saved; absent is the recorder's default folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_folder: Option<PathBuf>,
    /// Folders saved into before the save folder whose sessions may hold hours waiting to be
    /// saved: listed and saved from until none waits there ([`Self::choose_save_folder`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub earlier_save_folders: Vec<PathBuf>,
    /// The Paradox user folder; absent finds it automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_folder: Option<PathBuf>,
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default = "enabled")]
    pub start_with_replay: bool,
    /// A campaign recorded in another mode continues in a new file.
    #[serde(default = "full", with = "capture_mode")]
    pub capture_mode: CaptureMode,
    #[serde(default)]
    pub campaigns: BTreeMap<String, CampaignChoice>,
    /// Recordings whose name the user chose; later saves keep it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chosen_names: Vec<RecordingId>,
}

/// A recording: its folder and the campaign its header names, whatever its file is called.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RecordingId {
    pub folder: PathBuf,
    pub campaign: String,
}

impl RecordingId {
    pub fn is(&self, folder: &Path, campaign: &[u8]) -> bool {
        same_path(&self.folder, folder) && self.campaign.as_bytes().eq_ignore_ascii_case(campaign)
    }
}

/// The form Windows compares paths in: one separator, no verbatim prefix, any case.
fn path_key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    if let Some(share) = text.strip_prefix(r"\\?\unc\") {
        return format!(r"\\{share}");
    }
    text.strip_prefix(r"\\?\")
        .map_or_else(|| text.clone(), str::to_owned)
}

pub fn same_path(first: &Path, second: &Path) -> bool {
    path_key(first) == path_key(second)
}

impl Default for RecorderSettings {
    fn default() -> Self {
        Self {
            compression: crate::compression::Compression::High,
            record_every_campaign: true,
            auto_save: true,
            save_folder: None,
            earlier_save_folders: Vec::new(),
            user_folder: None,
            start_with_windows: false,
            start_with_replay: true,
            capture_mode: CaptureMode::Full,
            campaigns: BTreeMap::new(),
            chosen_names: Vec::new(),
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

/// Where the viewer keeps its choices of how each campaign's histories join.
pub fn histories_folder() -> PathBuf {
    local_data().join("histories")
}

/// Where recordings are saved and listed when no folder was chosen.
pub fn default_save_folder() -> PathBuf {
    local_data().join("captures")
}

impl RecorderSettings {
    pub fn choose_compression(&mut self, compression: crate::compression::Compression) {
        self.compression = compression;
    }

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

    /// Save into `folder` from now on. The folder saved into until now stays among the session
    /// roots, since its sessions may hold hours waiting to be saved, until it is forgotten.
    pub fn choose_save_folder(&mut self, folder: PathBuf) {
        let before = self.effective_save_folder();
        if !same_path(&before, &folder)
            && !self
                .earlier_save_folders
                .iter()
                .any(|earlier| same_path(earlier, &before))
        {
            self.earlier_save_folders.push(before);
        }
        self.earlier_save_folders
            .retain(|earlier| !same_path(earlier, &folder));
        self.save_folder = Some(folder);
    }

    /// Every folder whose sessions may hold hours waiting to be saved: the save folder, then each
    /// earlier one.
    pub fn session_roots(&self) -> Vec<PathBuf> {
        let current = self.effective_save_folder();
        let mut roots = Vec::with_capacity(1 + self.earlier_save_folders.len());
        roots.extend(
            self.earlier_save_folders
                .iter()
                .filter(|earlier| !same_path(earlier, &current))
                .cloned(),
        );
        roots.insert(0, current);
        roots
    }

    /// Keep `root`, a folder a session records in, among the session roots; whether it was not.
    pub fn keep_session_root(&mut self, root: PathBuf) -> bool {
        if self
            .session_roots()
            .iter()
            .any(|known| same_path(known, &root))
        {
            return false;
        }
        self.earlier_save_folders.push(root);
        true
    }

    /// Forget `folder` as an earlier save folder, none of its sessions waiting to be saved now.
    pub fn forget_save_folder(&mut self, folder: &Path) {
        self.earlier_save_folders
            .retain(|earlier| !same_path(earlier, folder));
    }

    pub fn name_chosen(&self, folder: &Path, campaign: &[u8]) -> bool {
        self.chosen_names.iter().any(|id| id.is(folder, campaign))
    }

    /// Whether the change was needed.
    pub fn choose_name(&mut self, folder: &Path, campaign: &str, chosen: bool) -> bool {
        let before = self.chosen_names.len();
        self.chosen_names
            .retain(|id| !id.is(folder, campaign.as_bytes()));
        if chosen {
            self.chosen_names.push(RecordingId {
                folder: folder.to_path_buf(),
                campaign: campaign.to_owned(),
            });
        }
        chosen || self.chosen_names.len() != before
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
        #[cfg(windows)]
        let replaced = crate::publication::replace_file(&temporary, path).map_err(io::Error::from);
        #[cfg(not(windows))]
        let replaced = fs::rename(&temporary, path);
        replaced
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
    fn first_run_uses_the_documented_defaults() {
        let root = scratch("defaults");
        let path = root.join("settings.json");
        let settings = RecorderSettings::load_from(&path).unwrap();
        assert!(settings.record_every_campaign && settings.auto_save);
        assert!(settings.start_with_replay && !settings.start_with_windows);
        assert_eq!(settings.capture_mode, CaptureMode::Full);
        assert!(settings.save_folder.is_none() && settings.user_folder.is_none());
        assert!(settings.campaigns.is_empty() && settings.chosen_names.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn campaign_choices_override_the_global_default() {
        let root = scratch("campaigns");
        let path = root.join("settings.json");
        fs::write(
            &path,
            br#"{"recordEveryCampaign":false,"campaigns":{"a":{"record":true},"b":{"next":"newFile","folder":"b (2)"}}}"#,
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
        assert_eq!(RecorderSettings::load_from(&path).unwrap(), settings);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_chosen_name_belongs_to_a_recording_by_folder_and_campaign() {
        let campaign = "00000000-0000-4000-8000-000000000000";
        let mut settings = RecorderSettings::default();
        assert!(!settings.choose_name(Path::new(r"C:\r"), campaign, false));
        assert!(settings.choose_name(Path::new(r"\\?\C:\R"), campaign, true));
        assert!(settings.name_chosen(Path::new("c:/r"), campaign.to_uppercase().as_bytes()));
        assert!(!settings.name_chosen(Path::new(r"C:\r\Germany (2)"), campaign.as_bytes()));
        assert!(!settings.name_chosen(Path::new(r"C:\r"), b"00000000-0000-4000-8000-000000000001"));
        // Chosen again, it is kept once.
        settings.choose_name(Path::new(r"C:\r"), campaign, true);
        assert_eq!(settings.chosen_names.len(), 1);
        assert!(settings.choose_name(Path::new("C:/r"), campaign, false));
        assert!(settings.chosen_names.is_empty());
        assert!(same_path(
            Path::new(r"\\?\UNC\server\share\a.battleplan"),
            Path::new("//server/share/A.battleplan")
        ));
    }

    /// A folder saved into before stays a session root after another is chosen, once, until it
    /// is forgotten or chosen again.
    #[test]
    fn earlier_save_folders_stay_session_roots_until_forgotten() {
        let root = scratch("save-folders");
        let path = root.join("settings.json");
        let default = default_save_folder();
        let (settings, ()) = RecorderSettings::update_at(&path, |settings| {
            settings.choose_save_folder(PathBuf::from(r"C:\first"));
        })
        .unwrap();
        assert_eq!(
            settings.session_roots(),
            [PathBuf::from(r"C:\first"), default.clone()]
        );
        let (settings, ()) = RecorderSettings::update_at(&path, |settings| {
            settings.choose_save_folder(PathBuf::from(r"C:\second"));
            settings.choose_save_folder(PathBuf::from(r"c:/SECOND"));
        })
        .unwrap();
        assert_eq!(
            settings.session_roots(),
            [
                PathBuf::from("c:/SECOND"),
                default.clone(),
                PathBuf::from(r"C:\first")
            ]
        );
        // Chosen again, a folder is the save folder alone.
        let mut chosen = settings.clone();
        chosen.choose_save_folder(PathBuf::from(r"C:\FIRST"));
        assert_eq!(
            chosen.session_roots(),
            [
                PathBuf::from(r"C:\FIRST"),
                default.clone(),
                PathBuf::from("c:/SECOND")
            ]
        );
        let (settings, ()) = RecorderSettings::update_at(&path, |settings| {
            settings.forget_save_folder(Path::new(r"c:\First"));
            settings.forget_save_folder(&default);
        })
        .unwrap();
        assert_eq!(settings.session_roots(), [PathBuf::from("c:/SECOND")]);
        assert_eq!(RecorderSettings::load_from(&path).unwrap(), settings);
        // A session's folder forgotten before the session began is kept again, once.
        let mut kept = settings.clone();
        assert!(!kept.keep_session_root(PathBuf::from(r"\\?\C:\Second")));
        assert!(kept.keep_session_root(PathBuf::from(r"C:\first")));
        assert!(!kept.keep_session_root(PathBuf::from("c:/first")));
        assert_eq!(
            kept.session_roots(),
            [PathBuf::from("c:/SECOND"), PathBuf::from(r"C:\first")]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn capture_mode_is_saved_by_name_and_unknown_names_are_refused() {
        let root = scratch("capture-mode");
        let path = root.join("settings.json");
        RecorderSettings::update_at(&path, |settings| settings.capture_mode = CaptureMode::Map)
            .unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(r#""captureMode": "map""#), "{saved}");
        assert_eq!(
            RecorderSettings::load_from(&path).unwrap().capture_mode,
            CaptureMode::Map
        );
        fs::write(&path, br#"{"captureMode":"light"}"#).unwrap();
        assert!(RecorderSettings::load_from(&path).is_err());
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
                                settings.choose_name(
                                    Path::new(&format!("c:/{index}")),
                                    "00000000-0000-4000-8000-000000000000",
                                    true,
                                );
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
        assert_eq!(settings.chosen_names.len(), 25);
        assert_eq!(settings.campaigns.len(), 25);
        fs::remove_dir_all(root).unwrap();
    }
}
