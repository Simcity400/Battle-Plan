use engine_executable::build::hash_executable;
use engine_sites::Located;
use crate::windows::{self, Process};
use std::path::Path;

pub(crate) fn locate(path: &Path, base: usize) -> Result<&'static Located, String> {
    let file = std::fs::read(path).map_err(|e| e.to_string())?;
    let exe = hash_executable(file.as_slice()).map_err(|e| e.to_string())?;
    engine_sites::install_file(&file, &exe.sha256, base).map_err(|withheld| withheld.to_string())
}

pub(crate) fn module_base(process: &Process, pid: u32) -> Result<usize, String> {
    let name = process
        .path
        .file_name()
        .ok_or("game executable has no name")?;
    Ok(windows::modules(pid)?
        .into_iter()
        .find(|module| module.path.file_name() == Some(name))
        .ok_or("game module not found")?
        .base)
}
