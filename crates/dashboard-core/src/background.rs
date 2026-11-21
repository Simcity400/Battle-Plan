
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DASHBOARD: &str = "replay-desktop.exe";

pub const ROLE: &str = "--prepare";

pub fn folder() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| crate::invalid("LOCALAPPDATA is not set"))?,
    )
    .join("BattlePlan")
    .join("preparation"))
}

fn prefix(path: &Path) -> io::Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| crate::invalid("The recording's path is not Unicode"))?;
    Ok(format!("{:016x}-", fnv(text.to_lowercase().as_bytes())))
}

pub fn request(path: &Path) -> io::Result<PathBuf> {
    let path = fs::canonicalize(path)?;
    let text = path
        .to_str()
        .ok_or_else(|| crate::invalid("The recording's path is not Unicode"))?;
    let folder = folder()?;
    fs::create_dir_all(&folder)?;
    let name = folder.join(format!(
        "{}{:x}-{}.request",
        prefix(&path)?,
        now_ms(),
        std::process::id()
    ));
    let staged = name.with_extension("tmp");
    fs::write(&staged, text)?;
    fs::rename(&staged, &name).inspect_err(|_| {
        let _ = fs::remove_file(&staged);
    })?;
    Ok(path)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub recording: PathBuf,
    pub files: Vec<PathBuf>,
}

pub fn requests() -> io::Result<Vec<Request>> {
    let folder = folder()?;
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut asked: BTreeMap<String, Request> = BTreeMap::new();
    for entry in entries {
        let file = entry?.path();
        if file
            .extension()
            .is_none_or(|extension| extension != "request")
        {
            continue;
        }
        let recording = match fs::read_to_string(&file) {
            Ok(recording) => recording,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        asked
            .entry(recording.to_lowercase())
            .or_insert_with(|| Request {
                recording: PathBuf::from(&recording),
                files: Vec::new(),
            })
            .files
            .push(file);
    }
    let mut found = Vec::new();
    for request in asked.into_values() {
        match fs::metadata(&request.recording).and_then(|metadata| metadata.modified()) {
            Ok(modified) => found.push((Some(modified), request)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                eprintln!(
                    "info: {} was deleted or moved; it leaves the preparation queue",
                    request.recording.display()
                );
                forget(&request)?;
            }
            Err(_) => found.push((None, request)),
        }
    }
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.recording.cmp(&b.1.recording))
    });
    Ok(found.into_iter().map(|(_, request)| request).collect())
}

fn asks(path: &Path) -> io::Result<Vec<PathBuf>> {
    let prefix = prefix(path)?;
    let entries = match fs::read_dir(folder()?) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut asks = Vec::new();
    for entry in entries {
        let file = entry?.path();
        let name = file.file_name().and_then(|name| name.to_str());
        if name.is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".request")) {
            asks.push(file);
        }
    }
    Ok(asks)
}

pub fn pending(path: &Path) -> io::Result<bool> {
    Ok(!asks(path)?.is_empty())
}

pub fn forget_recording(path: &Path) -> io::Result<()> {
    let path = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    forget(&Request {
        files: asks(&path)?,
        recording: path,
    })
}

pub fn forget(request: &Request) -> io::Result<()> {
    for file in &request.files {
        match fs::remove_file(file) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    Ok(())
}

pub fn runner() -> io::Result<Option<fs::File>> {
    let folder = folder()?;
    fs::create_dir_all(&folder)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(folder.join("preparer.lock"))?;
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(fs::TryLockError::WouldBlock) => Ok(None),
        Err(fs::TryLockError::Error(error)) => Err(error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Activity {
    Preparing,
    WaitingForGame,
    WaitingForSpace,
    Ready,
    InApp,
    Cleared,
    Off,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub recording: PathBuf,
    pub activity: Activity,
    pub progress: Option<f64>,
    pub error: Option<String>,
    pub at: u64,
}

const VERSION: u32 = 3;

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub version: u32,
    pub running: bool,
    pub pid: u32,
    pub started: u64,
    pub entries: Vec<Entry>,
    pub error: Option<String>,
    pub at: u64,
}

impl Status {
    pub fn read() -> io::Result<Self> {
        #[derive(Deserialize)]
        struct Format {
            version: Option<u32>,
        }
        let path = folder()?.join("status.json");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error),
        };
        let format: Format =
            serde_json::from_slice(&bytes).map_err(|error| damaged(&path, error))?;
        if format.version != Some(VERSION) {
            let found = format
                .version
                .map_or_else(|| "none".to_owned(), |version| version.to_string());
            eprintln!(
                "info: {} is of format {found}, this build's is {VERSION}; it starts again",
                path.display()
            );
            return Ok(Self::default());
        }
        serde_json::from_slice(&bytes).map_err(|error| damaged(&path, error))
    }

    pub fn write(&mut self) -> io::Result<()> {
        self.version = VERSION;
        self.at = now_ms();
        let folder = folder()?;
        fs::create_dir_all(&folder)?;
        let staged = folder.join(format!("status.{}.tmp", std::process::id()));
        fs::write(&staged, serde_json::to_vec(self)?)?;
        fs::rename(&staged, folder.join("status.json"))
    }

    pub fn set(
        &mut self,
        recording: &Path,
        activity: Activity,
        progress: Option<f64>,
        error: Option<String>,
    ) {
        self.entries.retain(|entry| entry.recording != recording);
        self.entries.push(Entry {
            recording: recording.to_owned(),
            activity,
            progress,
            error,
            at: now_ms(),
        });
    }
}

fn damaged(path: &Path, error: serde_json::Error) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{} is damaged: {error}", path.display()),
    )
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_moves_to_the_end_with_what_happened_last() {
        let mut status = Status::default();
        status.set(Path::new("a"), Activity::Preparing, Some(0.5), None);
        status.set(Path::new("b"), Activity::WaitingForGame, None, None);
        status.set(
            Path::new("a"),
            Activity::Failed,
            None,
            Some("damaged".into()),
        );
        let shown: Vec<_> = status
            .entries
            .iter()
            .map(|entry| (entry.recording.to_str().unwrap(), entry.activity))
            .collect();
        assert_eq!(
            shown,
            [("b", Activity::WaitingForGame), ("a", Activity::Failed)]
        );
        assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
    }
}
