use super::{DirAccessError, RootEntry};
use std::path::Path;

#[derive(Debug)]
pub(super) struct PlatformRoot;

impl PlatformRoot {
    pub(super) fn open(_path: &Path) -> Result<Self, DirAccessError> {
        // W12B_DLC_META.md R9 F1/F3: normalization-insensitive POSIX name
        // binding and hosts without a mount-instance primitive are deferred.
        // They must never accept based on a raced readdir/fstatat pair or on
        // equal st_dev alone.
        unimplemented!("fail closed: W12B DLC metadata dossier R9 F1/F3 defers non-Linux POSIX")
    }

    pub(super) fn root_entries(&self) -> Result<Vec<RootEntry>, DirAccessError> {
        unimplemented!("fail closed: W12B DLC metadata dossier R9 F1/F3 defers non-Linux POSIX")
    }

    pub(super) fn read_file(&self, _components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        unimplemented!("fail closed: W12B DLC metadata dossier R9 F1/F3 defers non-Linux POSIX")
    }
}
