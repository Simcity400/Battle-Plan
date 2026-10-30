//! A recording's change-run index ([`replay::columnar_runs`]), prepared from its native columnar
//! recordings, one per session, in a folder of its prepared store's, which Clear removes with
//! the store: the viewer's map reads its hours from it in place of walking the recording.

use crate::Source;
use replay::{
    columnar_frames::{Limits, Recording},
    columnar_runs::{self, IndexFile, Layout},
    segments::Settings,
};
use replay_data::rows::TableId;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(test)]
mod tests;

/// The tables read from the index at query time: the map's.
pub fn tables() -> &'static [TableId] {
    dashboard_base::prepared_map::selected_tables()
}

/// The folder of `source`'s prepared store holding its index, or why its native recordings were
/// refused, placed from the recording it loaded, which is not read again.
pub fn folder(source: &Source) -> io::Result<PathBuf> {
    Ok(dashboard_base::prepared::recording_folder(source)?.join("change-runs"))
}

/// Where `source`'s index is kept ([`folder`]).
pub fn path(source: &Source) -> io::Result<PathBuf> {
    Ok(folder(source)?.join("index.h4runs"))
}

/// Where why `source`'s native recordings were refused is kept ([`folder`]).
fn refusal_path(source: &Source) -> io::Result<PathBuf> {
    Ok(folder(source)?.join("refused.txt"))
}

/// Why `source`'s native recordings were refused when its index was last prepared; none when
/// they were not, or it cannot be read.
fn refusal(source: &Source) -> io::Result<Option<String>> {
    match fs::read(refusal_path(source)?) {
        Ok(bytes) => Ok(String::from_utf8(bytes).ok()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// The `<replay stem>.*.h4col` files beside the replay at `path`, by name: its native columnar
/// recordings and those of other replays sharing its stem.
pub fn native_files(path: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(Vec::new());
    };
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let prefix = format!("{stem}.");
    // A replay held in a folder that is gone has nothing beside it.
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".h4col"))
        {
            files.push(entry.path());
        }
    }
    files.sort();
    Ok(files)
}

/// A stamp of the native recordings beside the replay at `path` ([`native_files`]): each one's
/// name, length and last write, so one landing, replaced or gone changes it. Empty with none.
pub fn native_revision(path: &Path) -> io::Result<String> {
    let mut stamp = String::new();
    for file in native_files(path)? {
        let revision = match crate::protocol::replay_revision(&file) {
            Ok(revision) => revision,
            // Gone since its folder was read.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let name = file.file_name().unwrap_or_default().to_string_lossy();
        stamp.push_str(&format!("{}:{name}:{revision}\n", name.len()));
    }
    Ok(stamp)
}

/// `source`'s native columnar recordings as its index holds them ([`columnar_runs::whole`]),
/// none or not: one of another campaign is another recording's; any other must be one
/// session's, read whole, and together they must hold every hour the replay records of its
/// ended sessions. Why not, naming the file, the session or the hours at fault.
pub fn natives(source: &Source) -> io::Result<Result<columnar_runs::Whole, String>> {
    let replay = source.replay();
    let mut found = Vec::new();
    for file in native_files(source.path())? {
        let name = file.display().to_string();
        let recording = match Recording::open_path(&file, Limits::default()) {
            Ok(recording) => recording,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(Err(format!("{name} cannot be read: {error}")));
            }
            Err(error) => return Err(error),
        };
        if recording.campaign() == replay.campaign_id {
            found.push((name, recording));
        }
    }
    columnar_runs::whole(found, &replay)
}

/// What `source`'s last index preparation left, against its native recordings as they are now.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Its index indexes them.
    Indexed,
    /// They were refused, for the reason named, and are refused for it still.
    Refused(String),
    /// Neither: one of them replaced, added or gone, a session it holds pending ended or
    /// another begun, or they hold its hours whole now or are refused for another reason.
    Stale,
}

/// What `source`'s last index preparation left ([`Outcome`]).
pub fn outcome(source: &Source) -> io::Result<Outcome> {
    Ok(match natives(source)? {
        Ok(whole)
            if IndexFile::summary_of(path(source)?).is_ok_and(|summary| {
                summary.indexes(&whole.recordings, &whole.pending, tables())
            }) =>
        {
            Outcome::Indexed
        }
        Ok(_) => Outcome::Stale,
        Err(reason) if refusal(source)?.as_ref() == Some(&reason) => Outcome::Refused(reason),
        Err(_) => Outcome::Stale,
    })
}

/// Removes `target`, which no native recordings hold now.
fn remove(target: &Path) -> io::Result<()> {
    match fs::remove_file(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        removed => removed,
    }
}

/// Writes `target` whole through a file of this call's own beside it, which `write` fills and
/// hands back once written: it is made durable, then put in place of `target`. Another
/// preparation of the same index writes its own file; a file not written whole is removed.
pub(crate) fn publish(
    target: &Path,
    write: impl FnOnce(File) -> io::Result<File>,
) -> io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let mut name = target.as_os_str().to_owned();
    name.push(format!(
        ".{}-{}.partial",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let partial = PathBuf::from(name);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let written = write(file).and_then(|file| file.sync_all());
    match written.and_then(|()| fs::rename(&partial, target)) {
        Ok(()) => Ok(()),
        Err(error) => {
            // The error says why it was refused; a file no reader opens is removed.
            let _ = fs::remove_file(&partial);
            Err(error)
        }
    }
}

/// Prepares `source`'s index from its native columnar recordings when they hold its ended
/// sessions' hours whole and its index does not index them, its sessions still being recorded
/// named pending, on every core, until `cancelled` is set: written whole in its [`folder`],
/// then put in place of the one there. A replay none of whose sessions ended is indexed with
/// every session pending. A preparation stopped or left short publishes nothing. Native
/// recordings that do not hold its ended sessions' hours whole, none among them, are refused:
/// any index there is removed, the reason is kept in its place until they change
/// ([`outcome`]), and returned.
pub fn prepare(source: &Source, cancelled: &AtomicBool) -> io::Result<Option<String>> {
    let (target, refused) = (path(source)?, refusal_path(source)?);
    fs::create_dir_all(folder(source)?)?;
    let mut whole = match natives(source)? {
        Err(reason) => {
            remove(&target)?;
            if refusal(source)?.as_ref() != Some(&reason) {
                publish(&refused, |mut file| {
                    file.write_all(reason.as_bytes())?;
                    Ok(file)
                })?;
            }
            return Ok(Some(reason));
        }
        Ok(whole) => whole,
    };
    remove(&refused)?;
    if IndexFile::summary_of(&target)
        .is_ok_and(|summary| summary.indexes(&whole.recordings, &whole.pending, tables()))
    {
        return Ok(None);
    }
    for recording in &mut whole.recordings {
        recording.select(tables());
    }
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    publish(&target, |file| {
        let (out, _) = columnar_runs::prepare(
            &whole,
            tables(),
            Layout::RECORDING,
            columnar_runs::LEVEL,
            &Settings::new(threads),
            cancelled,
            BufWriter::new(file),
        )?;
        out.into_inner().map_err(io::IntoInnerError::into_error)
    })
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("The change-run index could not be prepared: {error}"),
        )
    })?;
    Ok(None)
}
