//! The desktop host's content preferences: the game and user folders and
//! background preparation. The host and its development bridge write them;
//! the native viewer receives them from the host, and headless tools read them
//! as defaults for their own folder options.

use crate::invalid;
use installed_content_core::ResolveOptions;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Config {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    /// Prepare opened recordings in the background unless explicitly disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_preparation: Option<bool>,
}

impl Config {
    /// `content.json` in the host's preference folder, beside its video settings.
    pub fn path() -> io::Result<PathBuf> {
        Ok(
            PathBuf::from(
                std::env::var_os("APPDATA").ok_or_else(|| invalid("APPDATA is not set"))?,
            )
            .join("battle-plan")
            .join("content.json"),
        )
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

    /// Stores the folders once they name a usable installation, keeping the
    /// saved preparation choice.
    pub fn save(&self) -> io::Result<()> {
        self.resolve_options()?;
        engine::executable_tables::initialize(&self.game_dir)?;
        self.save_folders_at(&Self::path()?)
    }

    fn save_folders_at(&self, path: &Path) -> io::Result<()> {
        update(path, |stored| {
            stored.game_dir = self.game_dir.clone();
            stored.user_dir = self.user_dir.clone();
        })
    }

    pub fn background_preparation_enabled(&self) -> bool {
        self.background_preparation.unwrap_or(true)
    }

    /// Store only preparation admission, preserving the folders.
    pub fn save_background_preparation(enabled: bool) -> io::Result<()> {
        Self::save_background_preparation_at(&Self::path()?, enabled)
    }

    fn save_background_preparation_at(path: &Path, enabled: bool) -> io::Result<()> {
        update(path, |stored| stored.background_preparation = Some(enabled))
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

/// Reads, changes and replaces the preferences as one transaction. The desktop
/// host's folder Apply, its data service's preparation choice and the
/// development bridge all write the same file, so an exclusive lock on a
/// sibling file spans the whole read-modify-write, across threads and
/// processes alike; each writer also stages its bytes in a file of its own.
fn update(path: &Path, change: impl FnOnce(&mut Config)) -> io::Result<()> {
    let folder = path
        .parent()
        .ok_or_else(|| invalid("content preferences have no folder"))?;
    fs::create_dir_all(folder)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut config = Config::load_from(path)?;
    change(&mut config);
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4().simple()));
    let written = fs::write(&temporary, serde_json::to_vec_pretty(&config)?)
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Game and user folders chosen in Settings, needed by products that read
/// artwork or localisation.
#[derive(Clone)]
pub struct ContentOptions {
    pub options: ResolveOptions,
    pub allow_unverified: bool,
}

pub struct LoadContext {
    pub content: Option<ContentOptions>,
}

impl LoadContext {
    pub fn content(&self) -> io::Result<&ContentOptions> {
        self.content
            .as_ref()
            .ok_or_else(|| crate::invalid("Choose the game and user folders in Settings first"))
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use std::path::Path;

    fn folder() -> std::path::PathBuf {
        let folder =
            std::env::temp_dir().join(format!("hoi4-content-config-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&folder).unwrap();
        folder
    }

    #[test]
    fn preparation_defaults_on_and_each_save_keeps_the_other_preference() {
        let folder = folder();
        // The host's preference folder need not exist before the first save.
        let path = folder.join("battle-plan").join("content.json");
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
        assert!(
            Config::load_from(&path)
                .unwrap()
                .background_preparation_enabled()
        );
        Config::save_background_preparation_at(&path, false).unwrap();
        assert!(
            !Config::load_from(&path)
                .unwrap()
                .background_preparation_enabled()
        );
        // A folder save from a copy read before the choice keeps the saved choice.
        Config {
            game_dir: "G".into(),
            user_dir: "U".into(),
            background_preparation: None,
        }
        .save_folders_at(&path)
        .unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert_eq!(saved.game_dir, Path::new("G"));
        assert_eq!(saved.user_dir, Path::new("U"));
        assert!(!saved.background_preparation_enabled());
        Config::save_background_preparation_at(&path, true).unwrap();
        let saved = Config::load_from(&path).unwrap();
        assert!(saved.background_preparation_enabled());
        assert_eq!(saved.game_dir, Path::new("G"));
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// Folder Apply and preparation saves overlap in the host; neither may
    /// lose the other's acknowledged choice.
    #[test]
    fn overlapping_saves_keep_every_acknowledged_preference() {
        let folder = folder();
        let path = folder.join("battle-plan").join("content.json");
        for round in 0..20 {
            let enabled = round % 2 == 0;
            std::thread::scope(|scope| {
                scope.spawn(|| Config::save_background_preparation_at(&path, enabled).unwrap());
                scope.spawn(|| {
                    Config {
                        game_dir: format!("G{round}").into(),
                        user_dir: format!("U{round}").into(),
                        background_preparation: None,
                    }
                    .save_folders_at(&path)
                    .unwrap()
                });
            });
            let saved = Config::load_from(&path).unwrap();
            assert_eq!(saved.background_preparation, Some(enabled), "round {round}");
            assert_eq!(
                saved.game_dir,
                Path::new(&format!("G{round}")),
                "round {round}"
            );
        }
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(left.is_empty(), "staged files left behind: {left:?}");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_preferences_live_in_the_hosts_folder() {
        let path = Config::path().unwrap();
        assert_eq!(path.file_name().unwrap(), "content.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "battle-plan");
        assert_ne!(
            path.parent(),
            std::env::current_exe().unwrap().parent(),
            "the preferences are the host's, not beside an executable"
        );
    }
}
