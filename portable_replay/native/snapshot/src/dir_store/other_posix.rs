use super::{DirAccessError, RootEntry};
use std::path::Path;

#[derive(Debug)]
pub(super) struct PlatformRoot;

impl PlatformRoot {
    pub(super) fn open(_path: &Path) -> Result<Self, DirAccessError> {
        unimplemented!("fail closed: exact non-Linux POSIX root binding is unavailable")
    }

    pub(super) fn root_entries(&self) -> Result<Vec<RootEntry>, DirAccessError> {
        unimplemented!("fail closed: exact non-Linux POSIX root binding is unavailable")
    }

    pub(super) fn read_file(&self, _components: &[&str]) -> Result<Vec<u8>, DirAccessError> {
        unimplemented!("fail closed: exact non-Linux POSIX root binding is unavailable")
    }
}
