//! A recording's change-run index ([`replay::columnar_runs`]), prepared from its native columnar
//! recordings, one per capture, in a folder of its prepared store's, which Clear removes with
//! the store: the viewer's map reads its hours from it in place of walking the recording, and
//! an hour's product reader the tables its row frames leave to the native capture
//! ([`dashboard_base::prepared::query::native`]).

use crate::Source;
use engine_allocation::labels::{self, Kind, Owner};
use replay::{
    columnar_file,
    columnar_frames::Recording,
    columnar_runs::{self, IndexFile, Layout},
    container::Replay,
    segments::{self, Lane},
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

/// The tables read from the index: the map's, and every one a registered product reads through
/// the native hour reader ([`dashboard_base::products::Product::native_tables`]), in table order.
pub fn tables() -> &'static [TableId] {
    static TABLES: std::sync::LazyLock<Vec<TableId>> = std::sync::LazyLock::new(|| {
        let mut read = [false; replay_data::rows::TABLE_COUNT];
        let products = crate::products::PRODUCTS
            .iter()
            .flat_map(|product| (product.native_tables)());
        for table in dashboard_base::map_tables::selected_tables()
            .iter()
            .copied()
            .chain(products)
        {
            read[table.index()] = true;
        }
        TableId::ALL
            .into_iter()
            .filter(|table| read[table.index()])
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
        let recording = match Recording::open_path(&file) {
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

/// A file of a write's own beside its target, which [`Self::commit`] makes durable and puts
/// in place of the target; one not committed is removed once dropped.
pub(crate) struct Partial {
    path: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Partial {
    /// The file another preparation of the same target never writes, opened for writing.
    fn create(target: &Path) -> io::Result<(Self, File)> {
        static WRITES: AtomicU64 = AtomicU64::new(0);
        let mut name = target.as_os_str().to_owned();
        name.push(format!(
            ".{}-{}.partial",
            std::process::id(),
            WRITES.fetch_add(1, Ordering::Relaxed)
        ));
        let path = PathBuf::from(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let partial = Self {
            path,
            target: target.to_owned(),
            committed: false,
        };
        Ok((partial, file))
    }

    /// Makes `file`, written whole, durable, and puts it in place of the target.
    fn commit(mut self, file: File) -> io::Result<()> {
        let synced = file.sync_all();
        // Closed first: an open file cannot be renamed or removed everywhere.
        drop(file);
        synced?;
        fs::rename(&self.path, &self.target)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.committed {
            // The error says why it was refused; a file no reader opens is removed.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Writes `target` whole through a file of this call's own beside it, which `write` fills and
/// hands back once written: it is made durable, then put in place of `target`. Another
/// preparation of the same index writes its own file; a file not written whole is removed.
pub(crate) fn publish(
    target: &Path,
    write: impl FnOnce(File) -> io::Result<File>,
) -> io::Result<()> {
    let (partial, file) = Partial::create(target)?;
    let file = write(file)?;
    partial.commit(file)
}

/// `error`, met preparing the index, saying so.
fn unprepared(error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("The change-run index could not be prepared: {error}"),
    )
}

/// What `source`'s index needs ([`plan`]).
pub enum Plan {
    /// Its native recordings were refused, for the reason named: no index is kept.
    Refused(String),
    /// Its index indexes them as they are.
    Current,
    /// It is to be prepared of them.
    Due(Due),
}

impl Plan {
    /// Why its native recordings were refused, when they were.
    pub fn refused(&self) -> Option<&str> {
        match self {
            Self::Refused(reason) => Some(reason.as_str()),
            Self::Current | Self::Due(_) => None,
        }
    }

    /// The preparation it is due as a lane of the families' union run over `recording`, the one
    /// they walk, when it rides that ([`Due::rides`]); any other is prepared now, in runs of its
    /// own, until `cancelled` is set, as what it did says.
    pub fn take(
        self,
        recording: Option<&Recording>,
        cancelled: &AtomicBool,
    ) -> io::Result<(Option<Due>, Prepared)> {
        match self {
            Self::Due(due) if recording.is_some_and(|recording| due.rides(recording)) => {
                Ok((Some(due), Prepared::default()))
            }
            Self::Due(due) => Ok((None, due.alone(cancelled)?)),
            Self::Refused(reason) => Ok((
                None,
                Prepared {
                    refused: Some(reason),
                    runs: Vec::new(),
                },
            )),
            Self::Current => Ok((None, Prepared::default())),
        }
    }
}

/// An index to prepare: of `whole`, its native recordings, into `target`, its walks' files in
/// `scratch`.
pub struct Due {
    whole: columnar_runs::Whole,
    target: PathBuf,
    scratch: PathBuf,
}

/// What `source`'s index needs: nothing when its native recordings are refused, which removes
/// any index there and keeps the reason in its place until they change ([`outcome`]), or when
/// its index indexes them; else the preparation it is due.
pub fn plan(source: &Source) -> io::Result<Plan> {
    let (target, refused) = (path(source)?, refusal_path(source)?);
    let scratch = folder(source)?;
    fs::create_dir_all(&scratch)?;
    let whole = match natives(source)? {
        Err(reason) => {
            remove(&target)?;
            if refusal(source)?.as_ref() != Some(&reason) {
                publish(&refused, |mut file| {
                    file.write_all(reason.as_bytes())?;
                    Ok(file)
                })?;
            }
            return Ok(Plan::Refused(reason));
        }
        Ok(whole) => whole,
    };
    remove(&refused)?;
    if IndexFile::summary_of(&target)
        .is_ok_and(|summary| summary.indexes(&whole.recordings, &whole.pending, tables()))
    {
        return Ok(Plan::Current);
    }
    Ok(Plan::Due(Due {
        whole,
        target,
        scratch,
    }))
}

impl Due {
    /// Whether a walk of `recording` walks every hour the index holds: it is the one native
    /// recording the index holds ([`columnar_runs::same`]), so a lane of a run over it can
    /// write the index ([`Self::ride`]).
    pub fn rides(&self, recording: &Recording) -> bool {
        matches!(&self.whole.recordings[..], [held] if columnar_runs::same(held, recording))
    }

    /// Prepares the index in runs of its own, on every core, until `cancelled` is set: written
    /// whole, then put in place of the one there. A preparation stopped or left short publishes
    /// nothing.
    pub fn alone(mut self, cancelled: &AtomicBool) -> io::Result<Prepared> {
        let _memory = labels::enter(Owner::RUN, Kind::Index);
        for recording in &mut self.whole.recordings {
            recording.select(tables());
        }
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        let mut runs = Vec::new();
        publish(&self.target, |file| {
            let (out, stats) = columnar_runs::prepare(
                &self.whole,
                tables(),
                Layout::RECORDING,
                columnar_runs::LEVEL,
                threads,
                &self.scratch,
                cancelled,
                BufWriter::new(file),
            )?;
            runs = stats;
            out.into_inner().map_err(io::IntoInnerError::into_error)
        })
        .map_err(unprepared)?;
        Ok(Prepared {
            refused: None,
            runs,
        })
    }

    /// The index as a lane of a run over its one native recording ([`Self::rides`]), which
    /// [`Riding::finish`] then puts in place.
    pub fn ride(self) -> io::Result<Riding> {
        let _memory = labels::enter(Owner::RUN, Kind::Index);
        let (partial, file) = Partial::create(&self.target)?;
        let preparation = columnar_runs::Preparation::new(
            &self.whole,
            tables(),
            Layout::RECORDING,
            columnar_runs::LEVEL,
            &self.scratch,
            BufWriter::new(file),
        )
        .map_err(unprepared)?;
        Ok(Riding {
            preparation: Some(preparation),
            partial,
        })
    }
}

/// An index written by a lane of another run over its one native recording ([`Due::ride`]).
pub struct Riding {
    // Before the partial file: the file closes before an unfinished one is removed.
    preparation: Option<columnar_runs::Preparation<BufWriter<File>>>,
    partial: Partial,
}

impl Riding {
    /// The tables its lane reads, which the run's walks decode.
    pub fn tables(&self) -> &'static [TableId] {
        tables()
    }

    /// Its lane, whose walks stop once `stop` is set and run for `job`.
    pub fn lane<'a, W: 'a>(
        &'a mut self,
        stop: &'a AtomicBool,
        job: dashboard_base::prepared::priority::JobPriority,
    ) -> io::Result<Lane<'a, W>> {
        self.preparation
            .as_mut()
            .ok_or_else(|| io::Error::other("A change-run index's preparation has ended"))?
            .lane_for(0, stop, job)
    }

    /// Puts the index in place once its lane walked every segment of `recording`, as `ran`
    /// says; publishes nothing when it did not.
    pub fn finish(mut self, recording: &Recording, ran: Rode) -> io::Result<()> {
        let _memory = labels::enter(Owner::RUN, Kind::Index);
        let preparation = self
            .preparation
            .take()
            .ok_or_else(|| io::Error::other("A change-run index's preparation has ended"))?;
        let finished = match ran {
            Rode::Stopped => Err(columnar_runs::stopped()),
            Rode::Ran(outcome) => columnar_runs::indexed(recording, outcome)
                .and_then(|()| preparation.finish(std::slice::from_ref(recording)))
                .and_then(|out| out.into_inner().map_err(io::IntoInnerError::into_error)),
        };
        let file = finished.map_err(unprepared)?;
        let Self { partial, .. } = self;
        partial.commit(file).map_err(unprepared)
    }
}

/// How the run an index's lane rode ended for it ([`Riding::finish`]).
pub enum Rode {
    /// It stopped before the lane settled.
    Stopped,
    Ran(segments::Outcome),
}

/// Prepares `source`'s index from its native columnar recordings when they hold its ended
/// sessions' hours whole and its index does not index them, its sessions still being recorded
/// named pending, on every core, until `cancelled` is set: written whole in its [`folder`],
/// then put in place of the one there. A replay none of whose sessions ended is indexed with
/// every session pending. A preparation stopped or left short publishes nothing. Native
/// recordings that do not hold its ended sessions' hours whole, none among them, are refused:
/// any index there is removed, the reason is kept in its place until they change
/// ([`outcome`]), and returned.
pub fn prepare(source: &Source, cancelled: &AtomicBool) -> io::Result<Prepared> {
    plan(source)?
        .take(None, cancelled)
        .map(|(_, prepared)| prepared)
}

/// What an index preparation did ([`prepare`]).
#[derive(Debug, Default)]
pub struct Prepared {
    /// Why the native recordings were refused, when they were.
    pub refused: Option<String>,
    /// Each recording's run, when the index was prepared.
    pub runs: Vec<segments::Stats>,
}
