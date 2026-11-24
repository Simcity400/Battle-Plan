
use replay_data::scope::CaptureMode;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io,
    path::{Path, PathBuf},
};

pub const STARTED_WITH_REPLAY: &str = "--with-replay";

pub const RECORDER_INSTANCE: &str = "Local\\BattlePlan.Recorder";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NextLoad {
    #[default]
    Continue,
    NewFile,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CampaignChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<bool>,
    #[serde(default)]
    pub next: NextLoad,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
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
    #[serde(default, rename = "liveCompression")]
    pub compression: crate::compression::Compression,
    #[serde(default = "enabled")]
    pub auto_compress: bool,
    #[serde(default = "enabled")]
    pub record_every_campaign: bool,
    #[serde(default = "enabled")]
    pub auto_save: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_folder: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub earlier_save_folders: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_folder: Option<PathBuf>,
    #[serde(default = "enabled")]
    pub start_with_replay: bool,
    #[serde(default = "full", with = "capture_mode")]
    pub capture_mode: CaptureMode,
    #[serde(default)]
    pub campaigns: BTreeMap<String, CampaignChoice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chosen_names: Vec<RecordingId>,
}

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
            compression: crate::compression::Compression::default(),
            auto_compress: true,
            record_every_campaign: true,
            auto_save: true,
            save_folder: None,
            earlier_save_folders: Vec::new(),
            user_folder: None,
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
        .join("BattlePlan")
}

pub fn directory() -> PathBuf {
    local_data().join("recorder")
}

pub fn histories_folder() -> PathBuf {
    local_data().join("histories")
}

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
    fn write_under(&self, receipt: &crate::exit::Receipt<'_>, path: &Path) -> io::Result<crate::exit::Applied> {
        receipt.replace(path, |out| serde_json::to_writer_pretty(out, self).map_err(io::Error::from))
    }

    pub fn update<T>(change: impl FnOnce(&mut Self) -> T) -> io::Result<crate::exit::Applied<Self, T>> {
        Self::update_at(&Self::path(), change)
    }
    pub fn update_at<T>(path: &Path, change: impl FnOnce(&mut Self) -> T) -> io::Result<crate::exit::Applied<Self, T>> {
        Self::hold_at(path, |held| held.update(change))
    }
    pub fn hold_at<T>(path: &Path, work: impl FnOnce(&Held<'_>) -> io::Result<T>) -> io::Result<T> {
        Self::hold_under(&crate::exit::EXIT, path, work)
    }
    pub fn hold_under<T>(exit: &crate::exit::Exit, path: &Path, work: impl FnOnce(&Held<'_>) -> io::Result<T>) -> io::Result<T> {
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".lock");
        exit.under_lock(Path::new(&lock_path), crate::exit::What::Change, |receipt| work(&Held { path, receipt }))
    }

    pub fn effective_save_folder(&self) -> PathBuf {
        self.save_folder.clone().unwrap_or_else(default_save_folder)
    }

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

    pub fn forget_save_folder(&mut self, folder: &Path) {
        self.earlier_save_folders
            .retain(|earlier| !same_path(earlier, folder));
    }

    pub fn name_chosen(&self, folder: &Path, campaign: &[u8]) -> bool {
        self.chosen_names.iter().any(|id| id.is(folder, campaign))
    }

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

    pub fn records_campaign(&self, campaign: &str) -> bool {
        self.campaigns
            .get(campaign)
            .and_then(|choice| choice.record)
            .unwrap_or(self.record_every_campaign)
    }
}

pub struct Held<'a> {
    path: &'a Path,
    receipt: &'a crate::exit::Receipt<'a>,
}

impl Held<'_> {
    pub fn update<T>(&self, change: impl FnOnce(&mut RecorderSettings) -> T) -> io::Result<crate::exit::Applied<RecorderSettings, T>> {
        let mut settings = RecorderSettings::load_from(self.path)?;
        let result = change(&mut settings);
        settings.campaigns.retain(|_, choice| !choice.is_empty());
        let applied = settings.write_under(self.receipt, self.path)?;
        Ok(applied.with(settings, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battleplan-recorder-settings-{name}-{}-{}",
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
    fn campaign_choices_override_the_global_default() {
        let root = scratch("campaigns");
        let path = root.join("settings.json");
        fs::write(
            &path,
            br#"{"recordEveryCampaign":false,"campaigns":{"a":{"record":true},"b":{"next":"newFile","folder":"b (2)"}}}"#,
        )
        .unwrap();
        let settings = RecorderSettings::update_at(&path, |settings| {
            settings.campaigns.entry("c".into()).or_default();
        })
        .unwrap()
        .value;
        assert!(settings.records_campaign("a"));
        assert!(!settings.records_campaign("b"));
        assert!(!settings.records_campaign("unknown"));
        assert_eq!(settings.campaigns["b"].next, NextLoad::NewFile);
        assert_eq!(settings.campaigns["b"].folder.as_deref(), Some("b (2)"));
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
        settings.choose_name(Path::new(r"C:\r"), campaign, true);
        assert_eq!(settings.chosen_names.len(), 1);
        assert!(settings.choose_name(Path::new("C:/r"), campaign, false));
        assert!(settings.chosen_names.is_empty());
        assert!(same_path(
            Path::new(r"\\?\UNC\server\share\a.battleplan"),
            Path::new("//server/share/A.battleplan")
        ));
    }

    #[test]
    fn earlier_save_folders_stay_session_roots_until_forgotten() {
        let root = scratch("save-folders");
        let path = root.join("settings.json");
        let default = default_save_folder();
        let settings = RecorderSettings::update_at(&path, |settings| {
            settings.choose_save_folder(PathBuf::from(r"C:\first"));
        })
        .unwrap()
        .value;
        assert_eq!(
            settings.session_roots(),
            [PathBuf::from(r"C:\first"), default.clone()]
        );
        let settings = RecorderSettings::update_at(&path, |settings| {
            settings.choose_save_folder(PathBuf::from(r"C:\second"));
            settings.choose_save_folder(PathBuf::from(r"c:/SECOND"));
        })
        .unwrap()
        .value;
        assert_eq!(
            settings.session_roots(),
            [
                PathBuf::from("c:/SECOND"),
                default.clone(),
                PathBuf::from(r"C:\first")
            ]
        );
        let mut chosen = settings;
        chosen.choose_save_folder(PathBuf::from(r"C:\FIRST"));
        assert_eq!(
            chosen.session_roots(),
            [
                PathBuf::from(r"C:\FIRST"),
                default.clone(),
                PathBuf::from("c:/SECOND")
            ]
        );
        let settings = RecorderSettings::update_at(&path, |settings| {
            settings.forget_save_folder(Path::new(r"c:\First"));
            settings.forget_save_folder(&default);
        })
        .unwrap()
        .value;
        assert_eq!(settings.session_roots(), [PathBuf::from("c:/SECOND")]);
        assert_eq!(RecorderSettings::load_from(&path).unwrap(), settings);
        let mut kept = settings;
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
        let _ = RecorderSettings::update_at(&path, |settings| settings.capture_mode = CaptureMode::Map)
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
    fn old_compression_keys_are_ignored_and_auto_compress_is_on_until_turned_off() {
        use crate::compression::Compression;
        let root = scratch("compression");
        let path = root.join("settings.json");
        for old in [&br#"{"blockCompression":"high"}"#[..], br#"{"compression":"high"}"#] {
            fs::write(&path, old).unwrap();
            let loaded = RecorderSettings::load_from(&path).unwrap();
            assert_eq!((loaded.compression, loaded.auto_compress), (Compression::Low, true));
        }
        let _ = RecorderSettings::update_at(&path, |settings| {
            settings.choose_compression(Compression::High);
            settings.auto_compress = false;
        })
        .unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(r#""liveCompression": "high""#) && saved.contains(r#""autoCompress": false"#), "{saved}");
        let loaded = RecorderSettings::load_from(&path).unwrap();
        assert_eq!((loaded.compression, loaded.auto_compress), (Compression::High, false));
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
                        let _ = RecorderSettings::update_at(&path, |settings| {
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

    #[test]
    fn an_operation_admitted_before_battleplan_closes_writes_all_its_steps_on_its_one_receipt() {
        let root = scratch("admitted");
        let path = root.join("settings.json");
        let exit = crate::exit::Exit::new();
        let written = RecorderSettings::hold_under(&exit, &path, |held| {
            assert!(exit.seal(), "BattlePlan starts closing while the operation holds its receipt");
            let _ = held.update(|settings| settings.auto_compress = false)?;
            held.update(|settings| settings.record_every_campaign = false)
        });
        assert!(written.is_ok(), "neither the save nor its compensating write asks for a second permit: {written:?}");
        let saved = RecorderSettings::load_from(&path).unwrap();
        assert!(!saved.auto_compress && !saved.record_every_campaign);
        let refused = RecorderSettings::hold_under(&exit, &path, |held| held.update(|_| ())).unwrap_err();
        assert_eq!(refused.to_string(), crate::exit::CLOSED, "an operation that begins after the close is refused whole");
        fs::remove_dir_all(root).unwrap();
    }
}
