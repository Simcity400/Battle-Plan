//! A recording's change-run index ([`replay::columnar_runs`]), prepared from its native columnar
//! recordings, one per capture, in a folder of its prepared store's, which Clear removes with
//! the store: the viewer's map reads its hours from it in place of walking the recording, and
//! an hour's product reader the tables its row frames leave to the native capture
//! ([`dashboard_base::prepared::query::native`]).

use crate::Source;
use replay::{
    columnar_file,
    columnar_frames::{Limits, Recording},
    columnar_runs::{self, IndexFile, Layout},
    container::Replay,
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

/// The tables read from the index at query time: the map's, and those the hour readers of
/// division careers, division templates, the training queue, army losses, war goals, faction
/// details, the politics details, regional weather and doctrine descriptions read, in table
/// order.
pub fn tables() -> &'static [TableId] {
    static TABLES: std::sync::LazyLock<Vec<TableId>> = std::sync::LazyLock::new(|| {
        use dashboard_army::products::{division_careers, division_templates, training_queue};
        use dashboard_weather::products::regional_weather;
        let map = dashboard_base::map_tables::selected_tables();
        let queue = training_queue::read_tables();
        let read: [&[TableId]; 14] = [
            &division_careers::TABLES,
            &division_templates::TABLES,
            queue.as_slice(),
            &dashboard_land_battles::products::army_losses::TABLES,
            &dashboard_base::products::equipment::DESIGN_TABLES,
            &dashboard_base::products::place_names::NAME_TABLES,
            dashboard_wars::products::wargoals::TABLES,
            &installed_content_country_names::country_names::TABLES,
            dashboard_base::products::faction_details::TABLES,
            dashboard_politics::products::politics_state::EXILE_TABLES,
            dashboard_politics::products::politics_state::BALANCE_TABLES,
            regional_weather::PROVINCE_TABLES,
            regional_weather::CHOICE_TABLES,
            dashboard_doctrine_mastery::products::doctrine_mastery::description_tables(),
        ];
        TableId::ALL
            .into_iter()
            .filter(|table| map.contains(table) || read.iter().any(|tables| tables.contains(table)))
            .collect()
    });
    &TABLES
}

/// The folder of `source`'s prepared store holding its index, or why its native recordings were
/// refused, placed from the recording it loaded, which is not read again.
pub fn folder(source: &Source) -> io::Result<PathBuf> {
    dashboard_base::prepared::query::native::index_folder(source)
}

/// Where `source`'s index is kept ([`folder`]).
pub fn path(source: &Source) -> io::Result<PathBuf> {
    dashboard_base::prepared::query::native::index_path(source)
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

/// The native columnar recordings of `campaign` published beside the replay at `path`, in the
/// campaign's folder there ([`columnar_file::campaign_folder`]), by session dir and then
/// capture: every capture of the campaign, whichever of its replays holds the session.
pub fn native_files(path: &Path, campaign: &[u8]) -> io::Result<Vec<PathBuf>> {
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    let folder = columnar_file::campaign_folder(folder, campaign);
    // A campaign with none published has no folder.
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if let Some((session, capture)) = columnar_file::published_of(&name) {
            files.push((session.to_owned(), capture, entry.path()));
        }
    }
    files.sort_unstable();
    Ok(files.into_iter().map(|(_, _, path)| path).collect())
}

/// A stamp of the native recordings of the replay at `path` ([`native_files`] of the campaign
/// its header names): each one's name, length and last write, so one landing, replaced or gone
/// changes it. Empty with none, as for a replay whose header names no campaign.
pub fn native_revision(path: &Path) -> io::Result<String> {
    let campaign = match Replay::read_campaign_id(&mut File::open(path)?) {
        Ok(campaign) => campaign,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ) =>
        {
            return Ok(String::new());
        }
        Err(error) => return Err(error),
    };
    let mut stamp = String::new();
    for file in native_files(path, &campaign)? {
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
/// none or not: one of another campaign, or holding no hour the replay records, is another
/// recording's or one whose save has not landed ([`records_any`]); any other must be one
/// session's, read whole, and together they must hold every hour the replay records of its
/// ended sessions. Why not, naming the file, the session or the hours at fault.
pub fn natives(source: &Source) -> io::Result<Result<columnar_runs::Whole, String>> {
    let replay = source.replay();
    let mut found = Vec::new();
    for file in native_files(source.path(), &replay.campaign_id)? {
        let name = file.display().to_string();
        let recording = match Recording::open_path(&file, Limits::default()) {
            Ok(recording) => recording,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(Err(format!("{name} cannot be read: {error}")));
            }
            Err(error) => return Err(error),
        };
        if recording.campaign() != replay.campaign_id {
            continue;
        }
        // One that cannot be read whole is the replay's to refuse by name.
        let foreign = columnar_runs::recorded_runs(&recording)?
            .is_some_and(|runs| !runs.is_empty() && !records_any(&replay, &runs));
        if !foreign {
            found.push((name, recording));
        }
    }
    columnar_runs::whole(found, &replay)
}

/// Whether `replay` records an hour of any of `runs` (branch, first hour, ordinals): a capture
/// holding none, even of a branch it holds, is one a save has not added to it yet.
fn records_any(replay: &Replay, runs: &[(u64, i32, u32)]) -> bool {
    runs.iter().any(|&(branch, first, ordinals)| {
        let (first, last) = (i64::from(first), i64::from(first) + i64::from(ordinals) - 1);
        replay.index.iter().any(|block| {
            block.branch == branch
                && i64::from(block.first_hour) <= last
                && i64::from(block.last_hour) >= first
        })
    })
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
