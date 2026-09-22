//! Game and user folders chosen once and stored beside the native executable.

use crate::invalid;
use installed_content::ResolveOptions;
use serde::{Deserialize, Serialize};
use std::{fs, io, path::PathBuf};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Config {
    pub game_dir: PathBuf,
    pub user_dir: PathBuf,
}

impl Config {
    pub fn path() -> io::Result<PathBuf> {
        Ok(std::env::current_exe()?
            .parent()
            .ok_or_else(|| invalid("executable has no parent directory"))?
            .join("viewer.json"))
    }

    pub fn load() -> io::Result<Self> {
        match fs::read(Self::path()?) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        self.resolve_options()?;
        fs::write(Self::path()?, serde_json::to_vec_pretty(self)?)
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
