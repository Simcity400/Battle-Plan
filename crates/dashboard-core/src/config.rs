
use crate::invalid;
use content_reader_layers::ResolveOptions;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Config {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub older_game_dirs: Vec<PathBuf>,
}

impl Config {
    pub fn path() -> io::Result<PathBuf> {
        Ok(
            PathBuf::from(
                std::env::var_os("APPDATA").ok_or_else(|| invalid("APPDATA is not set"))?,
            )
            .join("BattlePlan")
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

    pub fn save(&self) -> io::Result<()> {
        self.resolve_options()?;
        self.save_folders_at(&Self::path()?)
    }

    fn save_folders_at(&self, path: &Path) -> io::Result<()> {
        locked(path, || {
            replace(path, |stored| {
                stored.game_dir = self.game_dir.clone();
                stored.user_dir = self.user_dir.clone();
            })
        })
    }

    pub fn resolve_options(&self) -> io::Result<ResolveOptions> {
        if !self.game_dir.join("hoi4.exe").is_file() {
            return Err(invalid("Choose the game folder that contains hoi4.exe."));
        }
        if !self.user_dir.is_dir() {
            return Err(invalid("This Paradox user folder doesn’t exist."));
        }
        Ok(ResolveOptions {
            game_dir: self.game_dir.clone(),
            user_dir: self.user_dir.clone(),
            cache_dir: Self::content_cache_dir()?,
        })
    }

    pub fn content_cache_dir() -> io::Result<PathBuf> {
        Ok(PathBuf::from(
            std::env::var_os("LOCALAPPDATA").ok_or_else(|| invalid("LOCALAPPDATA is not set"))?,
        )
        .join("BattlePlan")
        .join("viewer"))
    }
}

fn locked<T>(path: &Path, work: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
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
    work()
}

fn replace(path: &Path, change: impl FnOnce(&mut Config)) -> io::Result<()> {
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

#[derive(Clone)]
pub struct ContentOptions {
    pub options: ResolveOptions,
    pub allow_unverified: bool,
    pub presentation: crate::presentation::Admitted,
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
    fn a_folder_save_keeps_the_other_preferences() {
        let folder = folder();
        let path = folder.join("BattlePlan").join("content.json");
        assert_eq!(Config::load_from(&path).unwrap(), Config::default());
        for round in 0..2 {
            Config {
                game_dir: format!("G{round}").into(),
                user_dir: format!("U{round}").into(),
                older_game_dirs: vec!["old".into()],
            }
            .save_folders_at(&path)
            .unwrap();
            let saved = Config::load_from(&path).unwrap();
            assert_eq!(saved.game_dir, Path::new(&format!("G{round}")));
            assert_eq!(saved.user_dir, Path::new(&format!("U{round}")));
        }
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_folder_refusal_names_the_folder_that_is_wrong() {
        let folder = folder();
        let game = folder.join("game");
        std::fs::create_dir(&game).unwrap();
        let refusal = |game_dir: &Path, user_dir: &Path| {
            Config {
                game_dir: game_dir.into(),
                user_dir: user_dir.into(),
                older_game_dirs: Vec::new(),
            }
            .resolve_options()
            .unwrap_err()
            .to_string()
        };
        assert_eq!(
            refusal(&folder, &folder),
            "Choose the game folder that contains hoi4.exe."
        );
        std::fs::write(game.join("hoi4.exe"), b"").unwrap();
        assert_eq!(
            refusal(&game, &folder.join("missing")),
            "This Paradox user folder doesn’t exist."
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_preferences_live_in_the_hosts_folder() {
        let path = Config::path().unwrap();
        assert_eq!(path.file_name().unwrap(), "content.json");
        assert_eq!(path.parent().unwrap().file_name().unwrap(), "BattlePlan");
        assert_ne!(
            path.parent(),
            std::env::current_exe().unwrap().parent(),
            "the preferences are the host's, not beside an executable"
        );
    }
}
