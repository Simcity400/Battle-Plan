//! Synthetic content-layer fixtures for reader tests.

use crate::ResolvedLayers;
use std::path::{Path, PathBuf};

/// A base game folder and a mod folder under a fresh temporary root.
pub struct Stack {
    root: PathBuf,
}

impl Stack {
    pub fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "hoi4_mod_layers_{name}_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        Self { root }
    }

    pub fn base(&self) -> PathBuf {
        self.root.join("base")
    }

    pub fn addon(&self) -> PathBuf {
        self.root.join("mod")
    }

    pub fn write(&self, layer: &Path, path: &str, text: &str) {
        let path = layer.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn layers(&self, replace_paths: &[&str]) -> ResolvedLayers {
        let (base, addon) = (self.base(), self.addon());
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&addon).unwrap();
        ResolvedLayers::from_folders(&[(&base, &[]), (&addon, replace_paths)]).unwrap()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
