//! Game and user folders chosen once and stored beside the native executable.

use crate::invalid;
use installed_content::ResolveOptions;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Config {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    /// The standalone viewer's map interface scale; absent matches the game on
    /// screen. The desktop host keeps its own and sends it to the viewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<f32>,
    /// Standalone viewer-local day/night display. An absent choice enables it;
    /// embedded maps use the persisted campaign/widget graphics preferences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_night: Option<bool>,
    /// Standalone viewer-local allied-plan choice. Absent keeps it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_allied_plans: Option<bool>,
    /// Whether the standalone viewer shows its technical stats and data
    /// limitations over the map. Off unless the user turns it on; the desktop
    /// host keeps its own choice and sends it to the viewer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub diagnostics: bool,
}

impl Config {
    pub fn path() -> io::Result<PathBuf> {
        Ok(std::env::current_exe()?
            .parent()
            .ok_or_else(|| invalid("executable has no parent directory"))?
            .join("viewer.json"))
    }

    pub fn load() -> io::Result<Self> {
        Self::load_from(&Self::path()?)
    }

    fn load_from(path: &Path) -> io::Result<Self> {
        match fs::read(path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    /// Stores the folders, keeping the saved standalone display choices.
    pub fn save(&self) -> io::Result<()> {
        self.resolve_options()?;
        self.save_at(&Self::path()?)
    }

    fn save_at(&self, path: &Path) -> io::Result<()> {
        let stored = Self::load_from(path).unwrap_or_default();
        let config = Self {
            diagnostics: stored.diagnostics,
            day_night: stored.day_night,
            show_allied_plans: stored.show_allied_plans,
            ..self.clone()
        };
        fs::write(path, serde_json::to_vec_pretty(&config)?)
    }

    /// Stores only the diagnostics choice, keeping the saved folders as they
    /// are even when they are not chosen yet.
    pub fn save_diagnostics(enabled: bool) -> io::Result<()> {
        Self::save_diagnostics_at(&Self::path()?, enabled)
    }

    fn save_diagnostics_at(path: &Path, enabled: bool) -> io::Result<()> {
        let mut config = Self::load_from(path)?;
        config.diagnostics = enabled;
        fs::write(path, serde_json::to_vec_pretty(&config)?)
    }

    /// Stores only the interface scale, keeping the saved folders as they are
    /// even when they are not chosen yet.
    pub fn save_ui_scale(scale: Option<f32>) -> io::Result<()> {
        if scale.is_some_and(|scale| !crate::protocol::MAP_UI_SCALE_RANGE.contains(&scale)) {
            return Err(invalid("Invalid map UI scale"));
        }
        let mut config = Self::load()?;
        config.ui_scale = scale;
        fs::write(Self::path()?, serde_json::to_vec_pretty(&config)?)
    }

    /// Stores only the day/night choice, preserving the saved folders and scale.
    pub fn save_day_night(enabled: bool) -> io::Result<()> {
        Self::save_day_night_at(&Self::path()?, enabled)
    }

    fn save_day_night_at(path: &Path, enabled: bool) -> io::Result<()> {
        let mut config = Self::load_from(path)?;
        config.day_night = Some(enabled);
        fs::write(path, serde_json::to_vec_pretty(&config)?)
    }

    /// Stores only allied-plan visibility, preserving folders and other choices.
    pub fn save_allied_plans(enabled: bool) -> io::Result<()> {
        let path = Self::path()?;
        let mut config = Self::load_from(&path)?;
        config.show_allied_plans = Some(enabled);
        fs::write(path, serde_json::to_vec_pretty(&config)?)
    }

    pub fn resolve_options(&self) -> io::Result<ResolveOptions> {
        if !self.game_dir.join("hoi4.exe").is_file() || !self.user_dir.is_dir() {
            return Err(invalid(
                "Choose game folder and the Paradox user folder.",
            ));
        }
        Ok(ResolveOptions {
            game_dir: self.game_dir.clone(),
            user_dir: self.user_dir.clone(),
            cache_dir: PathBuf::from(
                std::env::var_os("LOCALAPPDATA")
                    .ok_or_else(|| invalid("LOCALAPPDATA is not set"))?,
            )
            .join("HOI4Replay")
            .join("viewer"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::path::Path;

    #[test]
    fn day_night_choice_survives_folder_saves_and_old_files_enable_it() {
        let folder =
            std::env::temp_dir().join(format!("hoi4-day-night-config-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("viewer.json");
        std::fs::write(&path, br#"{"game_dir":"G","user_dir":"U","ui_scale":1.25}"#).unwrap();
        assert!(Config::load_from(&path).unwrap().day_night.unwrap_or(true));
        Config::save_day_night_at(&path, false).unwrap();
        let changed = Config::load_from(&path).unwrap();
        assert_eq!(changed.ui_scale, Some(1.25));
        assert_eq!(changed.game_dir, Path::new("G"));
        assert_eq!(changed.user_dir, Path::new("U"));
        let stale = Config {
            game_dir: "G2".into(),
            user_dir: "U2".into(),
            ..Default::default()
        };
        stale.save_at(&path).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert_eq!(saved.day_night, Some(false));
        assert_eq!(saved.user_dir, Path::new("U2"));
        Config::save_day_night_at(&path, true).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert_eq!(saved.day_night, Some(true));
        assert_eq!(saved.game_dir, Path::new("G2"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn saving_folders_keeps_the_diagnostics_choice_and_the_choice_keeps_folders() {
        let folder = std::env::temp_dir().join(format!("hoi4-config-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("viewer.json");
        let _ = std::fs::remove_file(&path);
        // Off unless saved: a missing file and an older file without the key.
        assert!(!Config::load_from(&path).unwrap().diagnostics);
        std::fs::write(&path, br#"{"game_dir":"G","user_dir":"U"}"#).unwrap();
        assert!(!Config::load_from(&path).unwrap().diagnostics);
        Config::save_diagnostics_at(&path, true).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert!(saved.diagnostics && saved.game_dir == Path::new("G"));
        // A folder save from a copy that still says off keeps the saved choice.
        let folders = Config {
            game_dir: "G2".into(),
            user_dir: "U2".into(),
            ..Config::default()
        };
        folders.save_at(&path).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert!(saved.diagnostics && saved.user_dir == Path::new("U2"));
        // Turning it off removes the key rather than writing false.
        Config::save_diagnostics_at(&path, false).unwrap();
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains("diagnostics")
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
