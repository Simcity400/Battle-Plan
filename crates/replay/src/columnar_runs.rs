//! A columnar recording's change-run index ([`replay_columnar::runs`]): prepared on the segment
//! executor and read as the row decoder's frames, so the viewer and products read any hour's
//! tables without walking the recording.
//!
//! One index holds a recording's native recordings, one per session ([`whole`]), their runs in
//! turn, and names the sessions still being recorded, whose hours it holds none of yet.
//! [`Preparation`] is one lane of a segment run of each: every segment's walk writes its
//! strips' pages, packed with zstd on the walk's worker, to a file of its own in the
//! preparation's folder, its open pages' bytes to another until they close; the lane's emit
//! takes each segment's directory in segment order, from the spool in that folder, and
//! appends its file to one file, removing it: a magic,
//! the pages, the index's directory packed, its [`Summary`], and a footer naming both. The summary names every native recording whole: its
//! identity, its capture and each of its runs' hour digest chain. [`IndexFile`] reads pages by
//! position, each checked against the pages' region and its packed frame before its bytes are
//! held, and a run's dictionary chunks only when a cell names one of their strings, through a
//! cache its readers share.
//! [`Snapshots`] holds a cursor of some tables and the row decoder's views of them: a move
//! takes the cursor to an hour and notes what differs into the views, and a build makes the
//! hour's frame from every move since the last. [`IndexFile::rows_at`] reads one table's rows
//! at a few keys.

use crate::{
    branches::BranchStart,
    columnar_file::Capture,
    columnar_frames::{Damage, Recording, Segment, Views, metrics, schema, wire},
    container::{BlockIndex, Parallel, Replay},
    format::FormatVersion,
    frame::{DecodedFrame, Metrics, TableView},
    identity::{Identity, IdentityCodec, SessionStart},
    rows::TableId,
    scope::CaptureMode,
    segments::{
        self, Cancellation, Chunk, Ending, Hour, Lane, Loss, Operator, Settings, Sink, Start,
    },
    spool::ControlRecord,
};
use engine_allocation::labels::{self, Kind as Label};
use job_priority::SpawnCarried as _;
pub use replay_columnar::runs::Layout;
use replay_columnar::{
    leaf::{Int, Reader, Writer},
    manifest::{Manifest, Section, Status},
    runs::{self, Builder, Cursor, Disk, Index, Location, PAGE_BYTES, Pack, Pages, Piece, Store},
    state::{Column, Kind},
    walk::{Chunks, Dictionary, Packed},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc,
    },
};
use zstd::bulk::{Compressor, Decompressor};

#[cfg(test)]
mod tests;

const MAGIC: &[u8; 8] = b"H4RUNS07";
/// The footer: the directory's offset, stored and raw lengths, the summary's offset and length,
/// then the magic.
const FOOTER: usize = 8 * 5 + MAGIC.len();
/// The zstd level pages are packed at, on the walks' workers.
pub const LEVEL: i32 = 3;
/// What each of a walk's files gathers before it goes to disk, and what the copier copies at once.
const LANDING_BYTES: usize = 256 << 10;
const COPY_BYTES: usize = 1 << 20;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Pages packed with zstd when that is smaller.
pub struct Zstd(Compressor<'static>);

impl Zstd {
    pub fn new(level: i32) -> io::Result<Self> {
        Ok(Self(Compressor::new(level)?))
    }
}

impl Pack for Zstd {
    fn pack(&mut self, raw: &[u8], out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        engine_allocation::reserve(out, zstd::zstd_safe::compress_bound(raw.len()))?;
        self.0.compress_to_buffer(raw, out)?;
        if out.len() >= raw.len() {
            out.clear();
            out.extend_from_slice(raw);
        }
        Ok(())
    }
}

/// Fills `into` with `file`'s bytes from `offset`.
#[cfg(windows)]
fn read_at(file: &File, offset: u64, into: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < into.len() {
        let read = file.seek_read(&mut into[done..], offset + done as u64)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += read;
    }
    Ok(())
}

#[cfg(unix)]
fn read_at(file: &File, offset: u64, into: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(into, offset)
}

#[cfg(not(any(windows, unix)))]
fn read_at(_: &File, _: u64, _: &mut [u8]) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// Writes `bytes` to `file` from `offset`.
#[cfg(windows)]
fn write_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < bytes.len() {
        let wrote = file.seek_write(&bytes[done..], offset + done as u64)?;
        if wrote == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        done += wrote;
    }
    Ok(())
}

#[cfg(unix)]
fn write_at(file: &File, offset: u64, bytes: &[u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(bytes, offset)
}

#[cfg(not(any(windows, unix)))]
fn write_at(_: &File, _: u64, _: &[u8]) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

/// A preparation's own folder, in the folder it was given: its walks' files, and its segment
/// runs' spool. Removed with what it holds once dropped, which is what a stopped or failed
/// preparation left there.
struct Folder {
    path: PathBuf,
    next: AtomicU64,
}

impl Folder {
    fn new(within: &Path) -> io::Result<Self> {
        static FOLDERS: AtomicU64 = AtomicU64::new(0);
        let path = within.join(format!(
            ".change-runs-{}-{}.scratch",
            std::process::id(),
            FOLDERS.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(within)?;
        fs::create_dir(&path)?;
        Ok(Self {
            path,
            next: AtomicU64::new(0),
        })
    }

    /// Where its segment runs spool their items: each run empties it as it starts and removes
    /// it once done.
    fn spool(&self) -> PathBuf {
        self.path.join("spool")
    }

    /// Walk `at`'s file of the index's bytes.
    fn pages(&self, at: u64) -> PathBuf {
        self.path.join(format!("seg-{at}"))
    }

    /// A segment's walk's files: what it writes of the index, and its open pages' bytes.
    fn landing(&self) -> io::Result<Landing> {
        let at = self.next.fetch_add(1, Ordering::Relaxed);
        Ok(Landing {
            at,
            pages: Spill::new(Temp::create(self.pages(at))?),
            open: Spill::new(Temp::create(self.path.join(format!("seg-{at}.open")))?),
        })
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// A file of a preparation's folder, removed once dropped unless [`Self::keep`] left it there.
struct Temp(Option<(File, PathBuf)>);

impl Temp {
    fn create(path: PathBuf) -> io::Result<Self> {
        let file =
            segments::temporary(OpenOptions::new().read(true).write(true).create_new(true))
                .open(&path)?;
        Ok(Self(Some((file, path))))
    }

    fn file(&self) -> io::Result<&File> {
        self.0
            .as_ref()
            .map(|(file, _)| file)
            .ok_or_else(|| io::Error::other("A preparation's file is closed"))
    }

    /// Closes it and leaves it to its folder.
    fn keep(mut self) {
        self.0 = None;
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        if let Some((file, path)) = self.0.take() {
            // Closed first: an open file cannot be removed everywhere.
            drop(file);
            let _ = fs::remove_file(path);
        }
    }
}

/// One of a walk's files written end to end: `landed` bytes on disk, then those it gathered,
/// which go to disk [`LANDING_BYTES`] at a time.
struct Spill {
    file: Temp,
    gathered: Vec<u8>,
    landed: u64,
}

impl Spill {
    fn new(file: Temp) -> Self {
        Self {
            file,
            gathered: Vec::new(),
            landed: 0,
        }
    }

    /// The bytes it holds, on disk and gathered.
    fn len(&self) -> u64 {
        self.landed + self.gathered.len() as u64
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.gathered.is_empty() {
            write_at(self.file.file()?, self.landed, &self.gathered)?;
            self.landed += self.gathered.len() as u64;
            self.gathered.clear();
        }
        Ok(())
    }

    /// Appends `bytes`: where they lie.
    fn append(&mut self, bytes: &[u8]) -> io::Result<u64> {
        let offset = self.len();
        if self.gathered.len() + bytes.len() > LANDING_BYTES {
            self.flush()?;
        }
        if bytes.len() > LANDING_BYTES {
            write_at(self.file.file()?, self.landed, bytes)?;
            self.landed += bytes.len() as u64;
        } else {
            let _memory = labels::kind(Label::IndexSpill);
            engine_allocation::reserve(&mut self.gathered, bytes.len())?;
            self.gathered.extend_from_slice(bytes);
        }
        Ok(offset)
    }

    /// Fills `into` with its bytes from `at`, which it holds: those on disk read, the rest
    /// copied from those gathered.
    fn read(&self, at: u64, into: &mut [u8]) -> io::Result<()> {
        let on_disk = self.landed.saturating_sub(at).min(into.len() as u64) as usize;
        let (disk, gathered) = into.split_at_mut(on_disk);
        if !disk.is_empty() {
            read_at(self.file.file()?, at, disk)?;
        }
        if !gathered.is_empty() {
            let from = (at + on_disk as u64 - self.landed) as usize;
            gathered.copy_from_slice(&self.gathered[from..][..gathered.len()]);
        }
        Ok(())
    }
}

/// Walk `at`'s files: its pages, views and dictionary chunks as they will lie in the index;
/// and its open pages' bytes, read back once as the walk ends.
struct Landing {
    at: u64,
    pages: Spill,
    open: Spill,
}

impl Landing {
    /// The segment's walk wrote `piece`: its file whole, left to the folder for the emit, its
    /// open pages' bytes let go.
    fn land(mut self, piece: Piece) -> io::Result<Landed> {
        self.pages.flush()?;
        let Self { at, pages, open } = self;
        drop(open);
        let length = pages.landed;
        pages.file.keep();
        Ok(Landed {
            piece,
            pages: at,
            length,
        })
    }
}

impl Store for Landing {
    fn put(&mut self, stored: &[u8], raw: u32) -> io::Result<Location> {
        let length = u32::try_from(stored.len()).map_err(|_| invalid("A page passes 4 GiB"))?;
        Ok(Location {
            offset: self.pages.append(stored)?,
            stored: length,
            raw,
        })
    }
}

impl Disk for Landing {
    fn keep(&mut self, bytes: &[u8]) -> io::Result<u64> {
        self.open.append(bytes)
    }

    fn kept(&mut self, at: u64, into: &mut [u8]) -> io::Result<()> {
        if at
            .checked_add(into.len() as u64)
            .is_none_or(|end| end > self.open.len())
        {
            return Err(invalid("An open page's bytes lie past those kept"));
        }
        self.open.read(at, into)
    }
}

/// A segment's directory, and which walk's file of the preparation's folder holds the `length`
/// bytes its locations name: the folder holds that file until the emit appends and removes it.
#[derive(replay_spool::Spooled)]
struct Landed {
    piece: Piece,
    pages: u64,
    length: u64,
}

/// One segment's walk writing its pages, its native recording's runs from the index's run
/// `first`, to its files, until `stop` is set.
struct Walk<'s> {
    builder: Builder,
    first: u32,
    stop: &'s AtomicBool,
    landing: Option<Landing>,
}

impl<W> Operator<W> for Walk<'_> {
    type Output = Landed;

    fn start(&mut self, start: Start<'_>, _branch: u64, _first: i32) -> io::Result<()> {
        self.builder.start(match start {
            Start::Restart(state) => state.dictionary().len(),
            Start::Run { .. } => 0,
        })
    }

    fn hour(&mut self, hour: &Hour<'_, W>, _out: &mut dyn Sink<Landed>) -> io::Result<()> {
        if self.stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        let landing = self
            .landing
            .as_mut()
            .ok_or_else(|| invalid("A change-run index's walk has ended"))?;
        self.builder.hour(
            self.first + hour.run,
            hour.ordinal,
            hour.state,
            hour.manifest,
            landing,
        )
    }

    fn end(&mut self, out: &mut dyn Sink<Landed>) -> io::Result<()> {
        let mut landing = self
            .landing
            .take()
            .ok_or_else(|| invalid("A change-run index's walk has ended"))?;
        let piece = self.builder.finish(&mut landing)?;
        out.push(landing.land(piece)?)
    }
}

/// Segments' files appended one after another to `out` by a copier thread of its own, in the
/// order the emit places them: the emit only names where each lands, so copying the index's
/// bytes is never its serial work.
struct Appended<W> {
    /// Where the next segment's bytes land.
    offset: u64,
    queue: Option<mpsc::Sender<(PathBuf, u64)>>,
    copier: Option<std::thread::JoinHandle<io::Result<W>>>,
    /// The copier's failure, which the next placement meets; its own result says it again.
    failed: Arc<Mutex<Option<io::Error>>>,
    /// Set as it is dropped unfinished: the copier copies nothing more.
    abandoned: Arc<AtomicBool>,
}

impl<W: Write + Send + 'static> Appended<W> {
    /// Copies into `out`, which stands at `offset`, the files placed.
    fn new(out: W, offset: u64) -> io::Result<Self> {
        let (queue, placed) = mpsc::channel::<(PathBuf, u64)>();
        let failed = Arc::new(Mutex::new(None));
        let abandoned = Arc::new(AtomicBool::new(false));
        let (failing, stopping) = (Arc::clone(&failed), Arc::clone(&abandoned));
        let label = labels::current();
        let copier = std::thread::Builder::new()
            .name("change-run copier".to_owned())
            .spawn_carried(move || {
                let _memory = labels::resume(label);
                let mut out = out;
                let mut buffer = Vec::new();
                let mut failure = None;
                for (path, length) in placed {
                    if failure.is_none() && !stopping.load(Ordering::Relaxed) {
                        failure = copy(&path, length, &mut out, &mut buffer).err();
                        if let Some(error) = &failure {
                            *failing.lock().unwrap_or_else(PoisonError::into_inner) =
                                Some(io::Error::new(error.kind(), error.to_string()));
                        }
                    }
                    let _ = fs::remove_file(&path);
                }
                match failure {
                    Some(error) => Err(error),
                    None => Ok(out),
                }
            })?;
        Ok(Self {
            offset,
            queue: Some(queue),
            copier: Some(copier),
            failed,
            abandoned,
        })
    }

    /// Places the `length` bytes of the file at `path` next: where they land.
    fn place(&mut self, path: PathBuf, length: u64) -> io::Result<u64> {
        if let Some(error) = self
            .failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        let at = self.offset;
        let queue = self
            .queue
            .as_ref()
            .ok_or_else(|| io::Error::other("A change-run index's copier has ended"))?;
        queue
            .send((path, length))
            .map_err(|_| io::Error::other("A change-run index's copier has ended"))?;
        self.offset = at
            .checked_add(length)
            .ok_or_else(|| invalid("A change-run index passes 2^64 bytes"))?;
        Ok(at)
    }

    /// Waits for every file placed to be copied: `out`, and where the next bytes land.
    fn finish(mut self) -> io::Result<(W, u64)> {
        drop(self.queue.take());
        let copier = self
            .copier
            .take()
            .ok_or_else(|| io::Error::other("A change-run index's copier has ended"))?;
        let out = copier
            .join()
            .map_err(|_| io::Error::other("A change-run index's copier panicked"))??;
        Ok((out, self.offset))
    }
}

impl<W> Drop for Appended<W> {
    fn drop(&mut self) {
        self.abandoned.store(true, Ordering::Relaxed);
        drop(self.queue.take());
        if let Some(copier) = self.copier.take() {
            let _ = copier.join();
        }
    }
}

/// Appends the `length` bytes of the file at `path` to `out`, through `buffer`.
fn copy(path: &Path, length: u64, out: &mut impl Write, buffer: &mut Vec<u8>) -> io::Result<()> {
    let file = File::open(path)?;
    if buffer.is_empty() {
        let _memory = labels::kind(Label::IndexSpill);
        *buffer = engine_allocation::filled(COPY_BYTES, 0_u8)?;
    }
    let mut at = 0;
    while at < length {
        let count = (length - at).min(buffer.len() as u64) as usize;
        read_at(&file, at, &mut buffer[..count])?;
        out.write_all(&buffer[..count])?;
        at += count as u64;
    }
    Ok(())
}

/// Why `runs` (branch, first hour, ordinals) do not hold exactly the hours `index`'s blocks
/// record, naming the first hours at fault; none when they do: every block's first and last
/// hours lie in a run of its branch, every run's hours lie within its branch's blocks, and no
/// two runs hold one hour. An hour inside a block that no run holds was dropped, as a native
/// run ends at every dropped hour; the row recording skips it within the block. No runs hold
/// no blocks.
pub fn uncovered<'b, I>(index: I, runs: &[(u64, i32, u32)]) -> Option<String>
where
    I: IntoIterator<Item = &'b BlockIndex>,
    I::IntoIter: Clone,
{
    let index = index.into_iter();
    let span = |first: i32, ordinals: u32| {
        let first = i64::from(first);
        (first, first + i64::from(ordinals) - 1)
    };
    let held = |branch: u64, hour: i32| {
        runs.iter().any(|&(run, first, ordinals)| {
            let (first, last) = span(first, ordinals);
            run == branch && (first..=last).contains(&i64::from(hour))
        })
    };
    for block in index.clone() {
        if let Some(hour) = [block.first_hour, block.last_hour]
            .into_iter()
            .find(|&hour| !held(block.branch, hour))
        {
            return Some(format!(
                "No native run holds hour {hour} of branch {}",
                block.branch
            ));
        }
    }
    for (at, &(branch, first, ordinals)) in runs.iter().enumerate() {
        let (start, last) = span(first, ordinals);
        // A run's hours, block after block of its branch, with no hour outside one.
        let mut next = start;
        let mut blocks: Vec<&BlockIndex> = index
            .clone()
            .filter(|block| {
                block.branch == branch
                    && i64::from(block.last_hour) >= start
                    && i64::from(block.first_hour) <= last
            })
            .collect();
        blocks.sort_by_key(|block| block.first_hour);
        for block in blocks {
            if i64::from(block.first_hour) > next {
                break;
            }
            next = next.max(i64::from(block.last_hour) + 1);
        }
        if ordinals != 0 && next <= last {
            return Some(format!(
                "Branch {branch}'s native run from hour {first} holds hour {next}, which the \
                 recording does not"
            ));
        }
        let overlaps = runs[..at].iter().any(|&(other, earlier, count)| {
            let (from, to) = span(earlier, count);
            other == branch && from <= last && start <= to
        });
        if overlaps {
            return Some(format!(
                "Two native runs of branch {branch} hold hour {first} or hours after it"
            ));
        }
    }
    None
}

/// Whether `runs` hold exactly the hours `index`'s blocks record ([`uncovered`]).
pub fn covers(index: &[BlockIndex], runs: &[(u64, i32, u32)]) -> bool {
    uncovered(index, runs).is_none()
}

/// The value `bytes` record in format `format`, when they hold exactly one.
fn recorded<T: IdentityCodec>(bytes: &[u8], format: u16) -> Option<T> {
    let mut decoder = crate::codec::Decoder::new(bytes);
    decoder.format = format;
    let value = T::decode(&mut decoder).ok()?;
    decoder.finish().ok()?;
    Some(value)
}

/// `value` in its recorded form.
fn encoded(value: &impl IdentityCodec) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    value.encode(&mut out).ok().map(|()| out)
}

/// What of a replay its native recordings are held against: its campaign and capture mode, its
/// sessions, the session of each branch, the hours its blocks record, and its control records,
/// which record where each session's capture ended.
#[derive(Clone, Copy)]
struct Sessions<'r> {
    campaign: &'r [u8],
    capture_mode: u8,
    sessions: &'r [SessionStart],
    branches: &'r [BranchStart],
    index: &'r [BlockIndex],
    controls: &'r [ControlRecord],
}

impl<'r> Sessions<'r> {
    fn of(replay: &'r Replay) -> Self {
        Self {
            campaign: &replay.campaign_id,
            capture_mode: replay.capture_mode.byte(),
            sessions: &replay.sessions,
            branches: &replay.branches,
            index: &replay.index,
            controls: &replay.controls,
        }
    }

    /// Whether it records that session `id`'s capture ended: the last of the session's control
    /// records is its detach, or its capture's failure. A capture that went on after a detach
    /// records its reattachment after it.
    fn ended(self, id: u64) -> bool {
        self.controls
            .iter()
            .rev()
            .find(|control| control.session() == id)
            .is_some_and(|control| {
                matches!(
                    control,
                    ControlRecord::Detach { .. } | ControlRecord::Partial { .. }
                )
            })
    }

    /// Whether `branch` is of a session whose capture it does not record the end of.
    fn open(self, branch: u64) -> bool {
        self.of_branch(branch)
            .is_some_and(|at| !self.ended(self.sessions[at].id))
    }

    /// The starts of its sessions whose capture it does not record the end of, encoded as it
    /// records them, in its sessions' order.
    fn pending(self) -> io::Result<Vec<Vec<u8>>> {
        let mut pending = Vec::new();
        for session in self.sessions {
            if !self.ended(session.id) {
                let start = encoded(session)
                    .ok_or_else(|| invalid("A session's start cannot be encoded"))?;
                engine_allocation::reserve(&mut pending, 1)?;
                pending.push(start);
            }
        }
        Ok(pending)
    }

    /// The branches of the sessions whose starts, as it encodes them, `starts` holds, each
    /// with its session.
    fn branches_of(self, starts: &[Vec<u8>]) -> Vec<(u64, u64)> {
        self.branches
            .iter()
            .filter(|branch| {
                self.sessions
                    .iter()
                    .find(|session| session.id == branch.session)
                    .and_then(encoded)
                    .is_some_and(|start| starts.contains(&start))
            })
            .map(|branch| (branch.id, branch.session))
            .collect()
    }

    /// The place among its sessions of the session `branch` belongs to.
    fn of_branch(self, branch: u64) -> Option<usize> {
        let session = self.branches.iter().find(|held| held.id == branch)?.session;
        self.sessions.iter().position(|held| held.id == session)
    }

    /// Whether `capture`, the capture of a recording of `runs` (branch, first hour, ordinals),
    /// is its: its campaign and capture mode, and for every run the session its branch belongs
    /// to, with that session's content identity. The capture holds them in the row recording's
    /// format, so they are compared as read, not as this release would encode them.
    fn captured(self, capture: &Capture, runs: &[(u64, i32, u32)]) -> bool {
        let content = recorded::<Identity>(&capture.content, capture.format);
        capture.campaign_id == self.campaign
            && capture.capture_mode == self.capture_mode
            && capture.run_sessions.len() == runs.len()
            && runs
                .iter()
                .zip(&capture.run_sessions)
                .all(|(&(branch, _, _), &session)| {
                    self.of_branch(branch).is_some_and(|at| {
                        let held = &self.sessions[at];
                        capture
                            .sessions
                            .get(session as usize)
                            .and_then(|start| recorded::<SessionStart>(start, capture.format))
                            .as_ref()
                            == Some(held)
                            && content.as_ref() == Some(&held.identity)
                    })
                })
    }
}

/// Whether `capture`, the capture of a recording of `runs` (branch, first hour, ordinals), is
/// `replay`'s: its campaign and capture mode, and for every run the session its branch
/// belongs to in `replay`, with that session's content identity.
pub fn captures(capture: &Capture, runs: &[(u64, i32, u32)], replay: &Replay) -> bool {
    Sessions::of(replay).captured(capture, runs)
}

/// What one index of a replay holds: its campaign, the native recordings of its sessions whose
/// capture ended, in its sessions' order, and the starts of its sessions still being recorded,
/// encoded as it records them, none of whose hours the index holds yet. A replay none of whose
/// sessions ended has an index of no native recording.
pub struct Whole {
    pub campaign: Vec<u8>,
    pub recordings: Vec<Recording>,
    pub pending: Vec<Vec<u8>>,
}

/// `recordings`, each with its name, as the native recordings one index of `replay` holds:
/// each undamaged and naming its capture, which is the capture of one of `replay`'s sessions
/// ([`captures`]), ordered as `replay`'s sessions are and a session's several as given, and
/// together holding exactly the hours `replay` records of the sessions whose capture it records
/// the end of, gaps and all ([`uncovered`]), none or not; a session's several hold its hours
/// together, none twice and none left between them. A session it records no end of is still being
/// recorded: its hours are pending, and a native recording of it is left for when it ends. Why
/// not, naming the recording, the session or the hours at fault.
pub fn whole(
    recordings: Vec<(String, Recording)>,
    replay: &Replay,
) -> io::Result<Result<Whole, String>> {
    whole_of(recordings, Sessions::of(replay))
}

/// [`whole`], against `replay`'s sessions and hours.
fn whole_of(
    recordings: Vec<(String, Recording)>,
    replay: Sessions<'_>,
) -> io::Result<Result<Whole, String>> {
    let mut placed: Vec<(usize, String, Recording, Vec<(u64, i32, u32)>)> = Vec::new();
    for (name, recording) in recordings {
        if let Some(damage) = recording.damage().first() {
            return Ok(Err(format!("{name} cannot be read whole: {damage}")));
        }
        let Some(held) = recorded_runs(&recording)? else {
            return Ok(Err(format!("{name} cannot be read whole")));
        };
        let capture = match recording.capture() {
            Ok(Some(capture)) => capture,
            Ok(None) => return Ok(Err(format!("{name} names no capture"))),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                return Ok(Err(format!("{name}'s capture cannot be read: {error}")));
            }
            Err(error) => return Err(error),
        };
        let session = held
            .first()
            .and_then(|&(branch, _, _)| replay.of_branch(branch));
        let Some(session) = session.filter(|_| {
            recording.campaign() == replay.campaign && replay.captured(&capture, &held)
        }) else {
            return Ok(Err(format!(
                "{name} is not the native recording of a session of this recording"
            )));
        };
        if !replay.ended(replay.sessions[session].id) {
            continue;
        }
        engine_allocation::reserve(&mut placed, 1)?;
        placed.push((session, name, recording, held));
    }
    // Stable: a session's captures keep the order given, the order they were published in.
    placed.sort_by_key(|&(session, _, _, _)| session);
    for captures in placed.chunk_by(|a, b| a.0 == b.0) {
        let [(session, ..), _, ..] = captures else {
            continue;
        };
        let mut runs = Vec::new();
        for (_, _, _, held) in captures {
            engine_allocation::reserve(&mut runs, held.len())?;
            runs.extend_from_slice(held);
        }
        let session = *session;
        let blocks = replay.index.iter().filter(move |block| {
            !replay.open(block.branch) && replay.of_branch(block.branch) == Some(session)
        });
        if let Some(reason) = uncovered(blocks, &runs) {
            let names: Vec<&str> = captures.iter().map(|(_, name, _, _)| name.as_str()).collect();
            return Ok(Err(format!(
                "{} do not hold session {}'s hours together: {reason}",
                names.join(", "),
                replay.sessions[session].id
            )));
        }
    }
    let mut runs = Vec::new();
    for (_, _, _, held) in &placed {
        engine_allocation::reserve(&mut runs, held.len())?;
        runs.extend_from_slice(held);
    }
    let finished = replay
        .index
        .iter()
        .filter(|block| !replay.open(block.branch));
    if let Some(reason) = uncovered(finished, &runs) {
        // Ended sessions whose branches the recording holds hours of, with no native recording.
        let missing: Vec<String> = replay
            .sessions
            .iter()
            .enumerate()
            .filter(|&(at, session)| {
                !placed.iter().any(|&(held, _, _, _)| held == at)
                    && replay.ended(session.id)
                    && replay.branches.iter().any(|branch| {
                        branch.session == session.id
                            && replay.index.iter().any(|block| block.branch == branch.id)
                    })
            })
            .map(|(_, session)| session.id.to_string())
            .collect();
        return Ok(Err(match missing.as_slice() {
            [] => reason,
            missing => format!(
                "No native recording holds the hours of session {}: {reason}",
                missing.join(", ")
            ),
        }));
    }
    Ok(Ok(Whole {
        campaign: engine_allocation::copy_vec(replay.campaign)?,
        recordings: placed
            .into_iter()
            .map(|(_, _, recording, _)| recording)
            .collect(),
        pending: replay.pending()?,
    }))
}

/// Each readable run of `recording` (branch, first hour, ordinals); none when any cannot be
/// read.
pub fn recorded_runs(recording: &Recording) -> io::Result<Option<Vec<(u64, i32, u32)>>> {
    if !recording.damage().is_empty() {
        return Ok(None);
    }
    let mut runs = Vec::new();
    for run in 0..recording.run_count() {
        let held = recording.run(run)?;
        engine_allocation::reserve(&mut runs, 1)?;
        runs.push((held.branch, held.first_hour, held.ordinals));
    }
    Ok(Some(runs))
}

/// A copy of `withheld` ([`Recording::withheld`]).
fn copy_withheld(withheld: &[(usize, String)]) -> io::Result<Vec<(usize, String)>> {
    let mut copy = engine_allocation::vec(withheld.len())?;
    for (table, reason) in withheld {
        copy.push((*table, engine_allocation::string(reason)?));
    }
    Ok(copy)
}

/// The schema tables of `tables`, ascending, and each one's kinds.
fn table_kinds(tables: &[TableId]) -> io::Result<(Vec<usize>, Vec<Vec<Kind>>)> {
    let schema = schema()?;
    let mut indexed: Vec<usize> = tables.iter().map(|table| table.index()).collect();
    indexed.sort_unstable();
    indexed.dedup();
    let kinds = indexed.iter().map(|&table| schema[table].clone()).collect();
    Ok((indexed, kinds))
}

/// An index of native recordings being written to `out`: one lane of a segment run walking each
/// recording in turn, every table it indexes applied.
pub struct Preparation<W> {
    index: Index,
    campaign: Vec<u8>,
    /// Each native recording's identity and capture, its chains once finished, and its first
    /// run among the index's.
    natives: Vec<Native>,
    firsts: Vec<u32>,
    /// The tables each native recording's lane reads: those indexed it does not withhold.
    reads: Vec<Vec<TableId>>,
    pending: Vec<Vec<u8>>,
    tables: Vec<usize>,
    /// The same tables, as its lanes read them.
    ids: Vec<TableId>,
    kinds: Vec<Vec<Kind>>,
    level: i32,
    /// Before its folder: the copier stops before the files it reads go.
    store: Appended<W>,
    pack: Zstd,
    folder: Folder,
}

impl<W: Write + Send + 'static> Preparation<W> {
    /// The index of `tables` of `whole`'s native recordings, of its campaign, their runs in
    /// turn, naming its pending sessions, laid out as `layout`, its pages packed at zstd
    /// `level`, into `out`, its walks' files in a folder of its own in `scratch`. Every
    /// readable run is indexed whole; an unreadable one holds no hours.
    pub fn new(
        whole: &Whole,
        tables: &[TableId],
        layout: Layout,
        level: i32,
        scratch: &Path,
        out: W,
    ) -> io::Result<Self> {
        let (recordings, campaign) = (&whole.recordings, &whole.campaign);
        if campaign.is_empty() {
            return Err(invalid("A change-run index is of a campaign"));
        }
        if recordings
            .iter()
            .any(|recording| recording.campaign() != campaign.as_slice())
        {
            return Err(invalid(
                "A change-run index's native recordings are of one campaign",
            ));
        }
        let (tables, kinds) = table_kinds(tables)?;
        let mut shapes = Vec::new();
        let mut natives = engine_allocation::vec(recordings.len())?;
        let mut firsts = engine_allocation::vec(recordings.len())?;
        let mut reads = engine_allocation::vec(recordings.len())?;
        for recording in recordings {
            firsts.push(u32::try_from(shapes.len()).map_err(|_| invalid("Runs pass 2^32"))?);
            for run in 0..recording.run_count() {
                engine_allocation::reserve(&mut shapes, 1)?;
                shapes.push(match recording.run(run) {
                    Ok(held) => (held.branch, held.first_hour),
                    Err(error) if Damage::of(&error).is_some() => (0, 0),
                    Err(error) => return Err(error),
                });
            }
            natives.push(Native {
                identity: engine_allocation::copy_vec(recording.identity())?,
                capture: recording.capture()?,
                chains: Vec::new(),
                withheld: copy_withheld(recording.withheld())?,
            });
            let held = engine_allocation::collect_vec(
                tables
                    .iter()
                    .filter(|&&table| recording.withheld().iter().all(|&(gone, _)| gone != table))
                    .map(|&table| TableId::ALL[table]),
            )?;
            engine_allocation::reserve(&mut reads, 1)?;
            reads.push(held);
        }
        let mut out = out;
        out.write_all(MAGIC)?;
        let folder = Folder::new(scratch)?;
        let store = Appended::new(out, MAGIC.len() as u64)?;
        let ids = engine_allocation::collect_vec(
            TableId::ALL
                .into_iter()
                .filter(|table| tables.binary_search(&table.index()).is_ok()),
        )?;
        Ok(Self {
            index: Index::new(layout, &tables, kinds.clone(), &shapes)?,
            ids,
            campaign: engine_allocation::copy_vec(campaign)?,
            natives,
            firsts,
            reads,
            pending: whole
                .pending
                .iter()
                .map(|start| engine_allocation::copy_vec(start))
                .collect::<io::Result<_>>()?,
            tables,
            kinds,
            level,
            store,
            pack: Zstd::new(level)?,
            folder,
        })
    }

    /// The lane writing the pages of its native recording `native`, its walks stopping once
    /// `stop` is set and running for the job of the thread that makes the lane.
    pub fn lane<'a>(&'a mut self, native: usize, stop: &'a AtomicBool) -> io::Result<Lane<'a>> {
        self.lane_for(native, stop, job_priority::JobPriority::current())
    }

    /// [`Self::lane`], its walks running for `job`, in a run whose walks share the world `V`
    /// with other lanes: it reads only the tables it indexes, and none of the world.
    pub fn lane_for<'a, V: 'a>(
        &'a mut self,
        native: usize,
        stop: &'a AtomicBool,
        job: job_priority::JobPriority,
    ) -> io::Result<Lane<'a, V>> {
        let first = *self
            .firsts
            .get(native)
            .ok_or_else(|| invalid("A change-run index holds no such native recording"))?;
        let Self {
            index,
            tables,
            kinds,
            reads,
            level,
            store,
            folder,
            ..
        } = self;
        let (layout, level, tables, kinds) = (index.layout, *level, &*tables, &*kinds);
        let reads: &'a [Vec<TableId>] = reads;
        let folder: &'a Folder = folder;
        let make = move |_: &Segment| -> io::Result<Walk<'a>> {
            // Walks run on the executor's workers, which start at no job's priority.
            job.clone().enter(job_priority::Level::Normal);
            Ok(Walk {
                builder: Builder::new(layout, tables, kinds.clone(), Box::new(Zstd::new(level)?))?,
                first,
                stop,
                landing: Some(folder.landing()?),
            })
        };
        // Only the directories are taken in, in segment order: each segment's bytes are its
        // file's, appended whole by the copier while the emit goes on.
        let emit = move |chunk: Chunk<'_, Landed>| -> io::Result<()> {
            for landed in chunk.items {
                let at = store.place(folder.pages(landed.pages), landed.length)?;
                index.append(landed.piece, at)?;
            }
            Ok(())
        };
        Ok(Lane::new("change runs", make, emit).reads(&reads[native]))
    }

    /// The tables it indexes, in table order.
    pub fn tables(&self) -> &[TableId] {
        &self.ids
    }

    /// The finished file, once a lane walked every segment of each of `recordings`.
    pub fn finish(mut self, recordings: &[Recording]) -> io::Result<W> {
        if recordings.len() != self.natives.len() {
            return Err(invalid(
                "A change-run index finishes with other native recordings",
            ));
        }
        let mut held = self.index.runs.iter();
        for (native, recording) in self.natives.iter_mut().zip(recordings) {
            native.chains = engine_allocation::vec(recording.run_count() as usize)?;
            for run in 0..recording.run_count() {
                let indexed = held
                    .next()
                    .ok_or_else(|| invalid("A change-run index holds fewer runs"))?;
                let chain = match recording.run(run) {
                    Ok(expected) if indexed.hours.len() != expected.ordinals as usize => {
                        return Err(invalid(format!(
                            "Run {run}'s index holds {} of its {} hours",
                            indexed.hours.len(),
                            expected.ordinals
                        )));
                    }
                    Ok(expected) => expected.chain,
                    // An unreadable run holds no hours, and no chain names them.
                    Err(_) => [0; 32],
                };
                native.chains.push(chain);
            }
        }
        let directory = self.index.write()?;
        let mut stored = Vec::new();
        self.pack.pack(&directory, &mut stored)?;
        let summary = Summary {
            campaign: self.campaign,
            tables: self.index.tables.clone(),
            runs: runs_of(&self.index),
            natives: self.natives,
            pending: self.pending,
        }
        .write()?;
        let (mut out, directory_at) = self.store.finish()?;
        let summary_at = directory_at + stored.len() as u64;
        out.write_all(&stored)?;
        out.write_all(&summary)?;
        for word in [
            directory_at,
            stored.len() as u64,
            directory.len() as u64,
            summary_at,
            summary.len() as u64,
        ] {
            out.write_all(&word.to_le_bytes())?;
        }
        out.write_all(MAGIC)?;
        out.flush()?;
        Ok(out)
    }
}

/// The index of `tables` of `whole` ([`whole`]): its native recordings, of one campaign, laid
/// out as `layout` and packed at `level`, written to `out` by a segment run of each in turn on
/// `threads` workers, its walks' files and the runs' spool in a folder of its own in `scratch`,
/// stopping once `stop` is set, and its pending sessions named: every recording applies every
/// table indexed. Each run's statistics. An index stopped, or any segment of which reached its
/// emit short or not at all, is refused and never finished.
#[allow(clippy::too_many_arguments)]
pub fn prepare<W: Write + Send + 'static>(
    whole: &Whole,
    tables: &[TableId],
    layout: Layout,
    level: i32,
    threads: usize,
    scratch: &Path,
    stop: &AtomicBool,
    out: W,
) -> io::Result<(W, Vec<segments::Stats>)> {
    let recordings = &whole.recordings;
    let mut preparation = Preparation::new(whole, tables, layout, level, scratch, out)?;
    let settings = Settings::new(threads);
    let spool = preparation.folder.spool();
    let mut stats = engine_allocation::vec(recordings.len())?;
    for (native, recording) in recordings.iter().enumerate() {
        let ran = segments::run(
            recording,
            &settings,
            &spool,
            &Cancellation::default(),
            vec![preparation.lane(native, stop)?],
        )?;
        if ran.cancelled || stop.load(Ordering::Relaxed) {
            return Err(stopped());
        }
        for outcome in ran.outcomes {
            indexed(recording, outcome)?;
        }
        stats.push(ran.stats);
    }
    Ok((preparation.finish(recordings)?, stats))
}

/// Why a preparation was stopped.
pub fn stopped() -> io::Error {
    io::Error::new(
        io::ErrorKind::Interrupted,
        "A change-run index's preparation was stopped",
    )
}

/// Whether a run's lane of [`Preparation::lane`] wrote every segment of `recording`, as `outcome`
/// says: why not, when it did not.
pub fn indexed(recording: &Recording, outcome: segments::Outcome) -> io::Result<()> {
    match short(outcome, recording.segments().len()) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// Whether `a` and `b` are one recording: of one campaign, identity and capture, holding the
/// same runs, each with the same branch, hours and hour digest chain, in the same segments.
/// Walks of either then see the same hours.
pub fn same(a: &Recording, b: &Recording) -> bool {
    let runs = (0..a.run_count()).all(|run| match (a.run(run), b.run(run)) {
        (Ok(a), Ok(b)) => {
            a.branch == b.branch
                && a.first_hour == b.first_hour
                && a.ordinals == b.ordinals
                && a.chain == b.chain
        }
        _ => false,
    });
    let captured = match (a.capture(), b.capture()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    a.campaign() == b.campaign()
        && a.identity() == b.identity()
        && a.run_count() == b.run_count()
        && runs
        && captured
        && a.segments() == b.segments()
        && a.damage().is_empty()
        && b.damage().is_empty()
}

/// Why a preparation's run of `segments` segments left its index short: its family's failure,
/// else the first segment that reached its emit short, else the first it never went past.
fn short(outcome: segments::Outcome, segments: usize) -> Option<io::Error> {
    if let Some(failure) = outcome.failure {
        let how = if failure.panicked {
            "panicked"
        } else {
            "failed"
        };
        return Some(io::Error::new(
            failure.error.kind(),
            format!(
                "Segment {}'s {:?} {how}: {}",
                failure.at, failure.operation, failure.error
            ),
        ));
    }
    let Some(ended) = outcome.ended.into_iter().next() else {
        return (outcome.settled < segments).then(|| {
            io::Error::new(
                io::ErrorKind::Interrupted,
                format!(
                    "Segment {} of {segments} was never indexed",
                    outcome.settled
                ),
            )
        });
    };
    Some(match ended.ending {
        Ending::Refused { hour, error } => io::Error::new(
            error.kind(),
            format!(
                "Segment {} refused its branch from hour {hour}: {error}",
                ended.at
            ),
        ),
        Ending::Lost(Loss::Damaged(damage)) => io::Error::new(
            damage.kind(),
            format!("Segment {} cannot be read: {damage}", ended.at),
        ),
        Ending::Lost(Loss::After(before)) => invalid(format!(
            "Segment {} goes on from segment {before}, which was lost",
            ended.at
        )),
        Ending::Lost(Loss::Earlier) => invalid(format!(
            "Segment {} goes on from hours an earlier run lost",
            ended.at
        )),
    })
}

enum Source {
    File(File),
    Memory(Vec<u8>),
}

impl Source {
    fn len(&self) -> io::Result<u64> {
        match self {
            Self::File(file) => Ok(file.metadata()?.len()),
            Self::Memory(bytes) => Ok(bytes.len() as u64),
        }
    }

    fn read_at(&self, offset: u64, into: &mut [u8]) -> io::Result<()> {
        match self {
            Self::Memory(bytes) => {
                let start = usize::try_from(offset).map_err(|_| invalid("Past the index"))?;
                let held = start
                    .checked_add(into.len())
                    .and_then(|end| bytes.get(start..end))
                    .ok_or_else(|| invalid("A page lies past the index's end"))?;
                into.copy_from_slice(held);
                Ok(())
            }
            Self::File(file) => read_at(file, offset, into),
        }
    }
}

thread_local! {
    static UNPACK: RefCell<Option<Decompressor<'static>>> = const { RefCell::new(None) };
}

/// Each run of `index` (branch, first hour, ordinals).
fn runs_of(index: &Index) -> Vec<(u64, i32, u32)> {
    index
        .runs
        .iter()
        .map(|run| (run.branch, run.first_hour, run.hours.len() as u32))
        .collect()
}

/// One native recording an index holds: its identity block ([`Recording::identity`]), the
/// capture its root names, none for one naming none, and each of its runs' hour digest chain
/// after its last hour, which digests every manifest and section the run holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Native {
    pub identity: Vec<u8>,
    pub capture: Option<Capture>,
    pub chains: Vec<[u8; 32]>,
    /// The tables it withholds ([`Recording::withheld`]): its runs index none of their rows.
    pub withheld: Vec<(usize, String)>,
}

/// What an index indexes, read without its directory: its campaign, its tables, its runs
/// (branch, first hour, ordinals), the native recordings it holds, their runs in turn, and the
/// sessions still being recorded when it was prepared, none of whose hours it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub campaign: Vec<u8>,
    /// Schema tables, ascending.
    pub tables: Vec<usize>,
    pub runs: Vec<(u64, i32, u32)>,
    pub natives: Vec<Native>,
    /// Each pending session's start, encoded as its recording records it.
    pub pending: Vec<Vec<u8>>,
}

impl Summary {
    /// The tables the native recording holding run `run` withholds, each with why.
    pub fn withheld_in(&self, run: u32) -> &[(usize, String)] {
        let mut first = 0_usize;
        for native in &self.natives {
            first += native.chains.len();
            if (run as usize) < first {
                return &native.withheld;
            }
        }
        &[]
    }

    fn write(&self) -> io::Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.varint(self.campaign.len() as u64)?;
        writer.raw(&self.campaign)?;
        writer.varint(self.tables.len() as u64)?;
        for &table in &self.tables {
            writer.varint(table as u64)?;
        }
        writer.varint(self.runs.len() as u64)?;
        for &(branch, first_hour, ordinals) in &self.runs {
            writer.varint(branch)?;
            writer.varint(first_hour.wire())?;
            writer.varint(u64::from(ordinals))?;
        }
        if self
            .natives
            .iter()
            .map(|native| native.chains.len())
            .sum::<usize>()
            != self.runs.len()
        {
            return Err(invalid("A summary's native recordings hold other runs"));
        }
        writer.varint(self.natives.len() as u64)?;
        for native in &self.natives {
            writer.varint(native.identity.len() as u64)?;
            writer.raw(&native.identity)?;
            match &native.capture {
                None => writer.varint(0)?,
                Some(capture) => {
                    let bytes = capture.write()?;
                    writer.varint(bytes.len() as u64 + 1)?;
                    writer.raw(&bytes)?;
                }
            }
            writer.varint(native.chains.len() as u64)?;
            for chain in &native.chains {
                writer.raw(chain)?;
            }
            writer.varint(native.withheld.len() as u64)?;
            for (table, reason) in &native.withheld {
                writer.varint(*table as u64)?;
                writer.varint(reason.len() as u64)?;
                writer.raw(reason.as_bytes())?;
            }
        }
        writer.varint(self.pending.len() as u64)?;
        for start in &self.pending {
            writer.varint(start.len() as u64)?;
            writer.raw(start)?;
        }
        Ok(writer.into_bytes())
    }

    fn read(bytes: &[u8]) -> io::Result<Self> {
        let mut reader = Reader::new(bytes);
        let owned = |reader: &mut Reader<'_>| -> io::Result<Vec<u8>> {
            let length = reader.count()?;
            engine_allocation::copy_vec(reader.raw(length)?)
        };
        let campaign = owned(&mut reader)?;
        let count = reader.count()?;
        let mut tables = engine_allocation::vec(count.min(bytes.len()))?;
        for _ in 0..count {
            tables.push(reader.count()?);
        }
        let count = reader.count()?;
        let mut runs = engine_allocation::vec(count.min(bytes.len()))?;
        for _ in 0..count {
            runs.push((
                reader.varint()?,
                i32::unwire(reader.varint()?)?,
                reader.varint_u32()?,
            ));
        }
        let count = reader.count()?;
        let mut natives = engine_allocation::vec(count.min(bytes.len()))?;
        for _ in 0..count {
            let identity = owned(&mut reader)?;
            let capture = match reader.count()? {
                0 => None,
                length => Some(Capture::read(reader.raw(length - 1)?)?),
            };
            let count = reader.count()?;
            let mut chains = engine_allocation::vec(count.min(bytes.len() / 32))?;
            for _ in 0..count {
                let mut chain = [0; 32];
                chain.copy_from_slice(reader.raw(32)?);
                chains.push(chain);
            }
            let count = reader.count()?;
            let mut withheld = engine_allocation::vec(count.min(bytes.len()))?;
            for _ in 0..count {
                let table = reader.count()?;
                let reason = String::from_utf8(owned(&mut reader)?)
                    .map_err(|_| invalid("A withheld table's reason is not text"))?;
                withheld.push((table, reason));
            }
            natives.push(Native {
                identity,
                capture,
                chains,
                withheld,
            });
        }
        let count = reader.count()?;
        let mut pending = engine_allocation::vec(count.min(bytes.len()))?;
        for _ in 0..count {
            pending.push(owned(&mut reader)?);
        }
        reader.finish()?;
        if natives
            .iter()
            .map(|native| native.chains.len())
            .sum::<usize>()
            != runs.len()
        {
            return Err(invalid("A summary's native recordings hold other runs"));
        }
        Ok(Self {
            campaign,
            tables,
            runs,
            natives,
            pending,
        })
    }

    /// Whether it indexes exactly `tables` of `recordings`, in order, with `pending` the
    /// sessions still being recorded: of their campaign, each with the same identity and
    /// capture, and every run of the same branch, first hour and hours, holding what it holds.
    /// Another recording in place of one, one more or one fewer, or a session pending or no
    /// longer, is not indexed.
    pub fn indexes(
        &self,
        recordings: &[Recording],
        pending: &[Vec<u8>],
        tables: &[TableId],
    ) -> bool {
        let mut expected: Vec<usize> = tables.iter().map(|table| table.index()).collect();
        expected.sort_unstable();
        expected.dedup();
        let mut runs = self.runs.iter();
        let natives = recordings
            .iter()
            .zip(&self.natives)
            .all(|(recording, native)| {
                recording.campaign() == self.campaign
                    && recording.identity() == native.identity
                    && recording.withheld() == native.withheld.as_slice()
                    && recording
                        .capture()
                        .is_ok_and(|capture| capture == native.capture)
                    && native.chains.len() == recording.run_count() as usize
                    && (0..).zip(&native.chains).all(|(run, chain)| {
                        runs.next().zip(recording.run(run).ok()).is_some_and(
                            |(&(branch, first_hour, ordinals), held)| {
                                held.branch == branch
                                    && held.first_hour == first_hour
                                    && held.ordinals == ordinals
                                    && held.chain == *chain
                            },
                        )
                    })
            });
        self.tables == expected
            && recordings.len() == self.natives.len()
            && natives
            && runs.next().is_none()
            && self.pending == pending
    }

    /// Whether it holds exactly the hours `replay` records, gaps and all, but those of its
    /// pending sessions, of its campaign and sessions: every native recording's capture is one
    /// of `replay`'s sessions ([`captures`]), its pending sessions are exactly those `replay`
    /// records no end of, and together the native recordings' runs hold the hours of the
    /// others ([`uncovered`]). An index of a recording naming no capture serves no replay.
    pub fn records(&self, replay: &Replay) -> bool {
        self.held_by(Sessions::of(replay))
    }

    /// [`Self::records`], against `replay`'s sessions and hours.
    fn held_by(&self, replay: Sessions<'_>) -> bool {
        let mut runs = &self.runs[..];
        let captured = self.natives.iter().all(|native| {
            let all = runs;
            let (held, rest) = all.split_at(native.chains.len().min(all.len()));
            runs = rest;
            native
                .capture
                .as_ref()
                .is_some_and(|capture| replay.captured(capture, held))
        });
        let held = replay
            .index
            .iter()
            .filter(|block| !replay.open(block.branch));
        self.campaign == replay.campaign
            && captured
            && replay
                .pending()
                .is_ok_and(|pending| pending == self.pending)
            && uncovered(held, &self.runs).is_none()
    }

    /// The branches of `replay` whose session it holds pending, each with its session: their
    /// hours are not indexed yet.
    pub fn pending(&self, replay: &Replay) -> Vec<(u64, u64)> {
        Sessions::of(replay).branches_of(&self.pending)
    }

    /// Whether it holds `tables`.
    pub fn holds(&self, tables: &[TableId]) -> bool {
        tables
            .iter()
            .all(|table| self.tables.binary_search(&table.index()).is_ok())
    }
}

/// An index file's length, where its directory lies, and its summary.
fn footer(source: &Source) -> io::Result<(u64, Location, Summary)> {
    let length = source.len()?;
    let short = || invalid("Not a change-run index");
    if length < (MAGIC.len() + FOOTER) as u64 {
        return Err(short());
    }
    let (mut head, mut footer) = ([0; MAGIC.len()], [0; FOOTER]);
    source.read_at(0, &mut head)?;
    source.read_at(length - FOOTER as u64, &mut footer)?;
    if &head != MAGIC || &footer[FOOTER - MAGIC.len()..] != MAGIC {
        return Err(short());
    }
    let word = |at: usize| u64::from_le_bytes(footer[at * 8..at * 8 + 8].try_into().expect("8"));
    let (offset, stored, raw) = (word(0), word(1), word(2));
    let (summary_at, summary_length) = (word(3), word(4));
    if offset.checked_add(stored) != Some(summary_at)
        || summary_at.checked_add(summary_length) != Some(length - FOOTER as u64)
    {
        return Err(invalid("A change-run index's footer names no directory"));
    }
    let location = Location {
        offset,
        stored: u32::try_from(stored).map_err(|_| invalid("A run index's directory"))?,
        raw: u32::try_from(raw).map_err(|_| invalid("A run index's directory"))?,
    };
    let mut summary =
        engine_allocation::filled(usize::try_from(summary_length).map_err(|_| short())?, 0_u8)?;
    source.read_at(summary_at, &mut summary)?;
    Ok((length, location, Summary::read(&summary)?))
}

/// A file's bytes, `start..end` of them pages, read by position.
struct Shared {
    source: Source,
    pages: (u64, u64),
    /// Dictionary chunks readers hold, by their offset: a chunk goes with its last reader.
    strings: Mutex<HashMap<u64, Weak<Packed>>>,
    /// Pages and dictionary chunks read.
    page_reads: AtomicUsize,
    chunk_reads: AtomicUsize,
}

/// The raw bytes stored at `location`, into `into`, emptied first: it lies within `region`,
/// raw it holds at most `limit` bytes, and packed it is one frame naming its raw length, all
/// checked before its raw bytes are held.
fn unpack(
    source: &Source,
    location: Location,
    (start, end): (u64, u64),
    limit: u32,
    into: &mut Vec<u8>,
) -> io::Result<()> {
    into.clear();
    let stop = location
        .offset
        .checked_add(u64::from(location.stored))
        .filter(|&stop| location.offset >= start && stop <= end)
        .ok_or_else(|| invalid("A change-run index's page lies outside its pages"))?;
    if location.raw > limit || location.stored > location.raw || stop == location.offset {
        return Err(invalid("A change-run index's page passes its length"));
    }
    if location.stored == location.raw {
        engine_allocation::resize(into, location.raw as usize, 0)?;
        return source.read_at(location.offset, into);
    }
    let mut stored = engine_allocation::filled(location.stored as usize, 0_u8)?;
    source.read_at(location.offset, &mut stored)?;
    if zstd::zstd_safe::find_frame_compressed_size(&stored).ok() != Some(stored.len())
        || zstd::zstd_safe::get_frame_content_size(&stored)
            .ok()
            .flatten()
            != Some(u64::from(location.raw))
    {
        return Err(invalid(
            "A change-run index's page is not one frame of its length",
        ));
    }
    engine_allocation::reserve(into, location.raw as usize)?;
    UNPACK.with(|unpack| -> io::Result<()> {
        let mut unpack = unpack.borrow_mut();
        if unpack.is_none() {
            *unpack = Some(Decompressor::new()?);
        }
        let decompressor = unpack
            .as_mut()
            .ok_or_else(|| invalid("A page's decompressor is not held"))?;
        decompressor.decompress_to_buffer(&stored, into)?;
        Ok(())
    })?;
    if into.len() != location.raw as usize {
        return Err(invalid(
            "A change-run index's page unpacks to another length",
        ));
    }
    Ok(())
}

impl Shared {
    fn read(&self, location: Location, into: &mut Vec<u8>) -> io::Result<()> {
        unpack(&self.source, location, self.pages, PAGE_BYTES, into)?;
        self.page_reads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn strings(&self, chunk: &runs::Chunk) -> io::Result<Arc<Packed>> {
        let key = chunk.location.offset;
        if let Some(held) = self
            .strings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .and_then(Weak::upgrade)
        {
            return Ok(held);
        }
        let mut raw = Vec::new();
        unpack(
            &self.source,
            chunk.location,
            self.pages,
            PAGE_BYTES,
            &mut raw,
        )?;
        self.chunk_reads.fetch_add(1, Ordering::Relaxed);
        let strings = Packed::read(chunk.first as usize, raw)?;
        if strings.len() != chunk.count as usize {
            return Err(invalid("A dictionary chunk holds other than its strings"));
        }
        let mut held = self.strings.lock().unwrap_or_else(PoisonError::into_inner);
        // Read outside the lock: a reader that raced this one may hold the chunk already.
        if let Some(raced) = held.get(&key).and_then(Weak::upgrade) {
            return Ok(raced);
        }
        held.retain(|_, strings| strings.strong_count() > 0);
        engine_allocation::reserve_map(&mut *held, 1)?;
        let strings = Arc::new(strings);
        held.insert(key, Arc::downgrade(&strings));
        Ok(strings)
    }
}

/// A run's dictionary chunks read through a file's shared cache when a cell names them.
struct RunChunks {
    shared: Arc<Shared>,
    chunks: Vec<runs::Chunk>,
}

impl Chunks for RunChunks {
    fn chunk(&self, id: u32) -> io::Result<Arc<Packed>> {
        let at = self.chunks.partition_point(|chunk| chunk.first <= id);
        let chunk = at
            .checked_sub(1)
            .map(|at| &self.chunks[at])
            .filter(|chunk| id - chunk.first < chunk.count)
            .ok_or_else(|| invalid(format!("No dictionary chunk holds string {id}")))?;
        self.shared.strings(chunk)
    }
}

/// A prepared change-run index, its directory held and its pages read by position, shared by
/// every reader of its recording.
pub struct IndexFile {
    pub index: Index,
    summary: Summary,
    length: u64,
    shared: Arc<Shared>,
}

impl IndexFile {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::load(Source::File(File::open(path)?))
    }

    pub fn from_bytes(bytes: Vec<u8>) -> io::Result<Self> {
        Self::load(Source::Memory(bytes))
    }

    /// The summary of the index at `path`, its directory not read.
    pub fn summary_of(path: impl AsRef<Path>) -> io::Result<Summary> {
        footer(&Source::File(File::open(path)?)).map(|(_, _, summary)| summary)
    }

    fn load(source: Source) -> io::Result<Self> {
        let (length, location, summary) = footer(&source)?;
        let directory_end = location.offset + u64::from(location.stored);
        let mut directory = Vec::new();
        unpack(
            &source,
            location,
            (location.offset, directory_end),
            u32::MAX,
            &mut directory,
        )?;
        let index = Index::read(&directory, &schema()?)?;
        if index.tables != summary.tables || runs_of(&index) != summary.runs {
            return Err(invalid(
                "A change-run index's summary differs from its directory",
            ));
        }
        Ok(Self {
            index,
            summary,
            length,
            shared: Arc::new(Shared {
                source,
                pages: (MAGIC.len() as u64, location.offset),
                strings: Mutex::default(),
                page_reads: AtomicUsize::new(0),
                chunk_reads: AtomicUsize::new(0),
            }),
        })
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }

    /// The campaign of the recording it indexes.
    pub fn campaign(&self) -> &[u8] {
        &self.summary.campaign
    }

    /// Its bytes, pages and directory with them.
    pub fn len(&self) -> u64 {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }

    /// Each run it holds (branch, first hour, ordinals).
    pub fn runs(&self) -> &[(u64, i32, u32)] {
        &self.summary.runs
    }

    /// Whether it holds `tables`.
    pub fn holds(&self, tables: &[TableId]) -> bool {
        self.summary.holds(tables)
    }

    /// Whether it indexes exactly `tables` of `recordings`, with `pending` the sessions still
    /// being recorded ([`Summary::indexes`]).
    pub fn indexes(
        &self,
        recordings: &[Recording],
        pending: &[Vec<u8>],
        tables: &[TableId],
    ) -> bool {
        self.summary.indexes(recordings, pending, tables)
    }

    /// The branches of `replay` whose session it holds pending, each with its session
    /// ([`Summary::pending`]).
    pub fn pending(&self, replay: &Replay) -> Vec<(u64, u64)> {
        self.summary.pending(replay)
    }

    /// Whether it holds exactly the hours `replay` records ([`Summary::records`]).
    pub fn records(&self, replay: &Replay) -> bool {
        self.summary.records(replay)
    }

    /// The pages and the dictionary chunks its readers read so far.
    pub fn reads(&self) -> (usize, usize) {
        (
            self.shared.page_reads.load(Ordering::Relaxed),
            self.shared.chunk_reads.load(Ordering::Relaxed),
        )
    }

    /// `table`'s rows at `keys` at `hour` of `branch`, as the recording held them: a view of
    /// them as a frame holds its table, opaque cells encoded at `format`, and the table's
    /// sample hour; none when no run holds the hour. Only the dictionary chunks holding the
    /// strings those rows name are read.
    pub fn rows_at(
        &self,
        branch: u64,
        hour: i32,
        table: TableId,
        keys: &[u128],
        format: FormatVersion,
        mode: CaptureMode,
    ) -> io::Result<Option<(TableView, Option<i32>)>> {
        let Some((run, ordinal)) = self.index.find(branch, hour) else {
            return Ok(None);
        };
        let withheld = self.summary.withheld_in(run);
        if let Some((_, reason)) = withheld.iter().find(|&&(held, _)| held == table.index()) {
            let reason = engine_allocation::string(reason)?;
            return Ok(Some((TableView::Unavailable { reason }, None)));
        }
        let keyed = runs::rows_at(&self.index, self, run, ordinal, table.index(), keys)?;
        let held = &self.index.runs[run as usize];
        let strings = || -> io::Result<Dictionary> {
            Ok(Dictionary::run(
                self.chunks(&held.chunks)?,
                held.strings as usize,
            ))
        };
        let view = match keyed.status {
            Status::NotRecorded => TableView::NotRecorded { mode },
            Status::Unavailable { reason } => {
                let mut dictionary = strings()?;
                dictionary.hold(reason)?;
                let reason = dictionary
                    .get(reason)
                    .ok_or_else(|| invalid("A reason names a missing dictionary entry"))?;
                TableView::Unavailable {
                    reason: engine_allocation::string(
                        std::str::from_utf8(reason)
                            .map_err(|_| invalid("A table's reason is not text"))?,
                    )?,
                }
            }
            status => {
                let found = &keyed.table;
                let mut dictionary = Dictionary::default();
                for column in 0..found.kinds().count() {
                    let Some(Column::Bytes(ids)) = found.column(column) else {
                        continue;
                    };
                    for slot in found.live().ones() {
                        if found.recorded(column, slot)
                            && let Some(&id) = ids.get(slot as usize)
                        {
                            if dictionary.is_empty() {
                                dictionary = strings()?;
                            }
                            dictionary.hold(id)?;
                        }
                    }
                }
                let keys = engine_allocation::collect_vec(
                    (0..found.slots() as u32).filter_map(|slot| found.key(slot)),
                )?;
                let format = format.version();
                let rows = table.rows_from_wire(&keys, &mut |row, column| {
                    wire(found, &dictionary, row as u32, column, format)
                })?;
                match status {
                    Status::Truncated { missing } => TableView::Truncated { rows, missing },
                    _ => TableView::Complete(rows),
                }
            }
        };
        Ok(Some((view, keyed.sampled)))
    }
}

impl Pages for IndexFile {
    fn read(&self, location: Location, into: &mut Vec<u8>) -> io::Result<()> {
        self.shared.read(location, into)
    }

    fn strings(&self, chunk: &runs::Chunk) -> io::Result<Arc<Packed>> {
        self.shared.strings(chunk)
    }

    fn chunks(&self, chunks: &[runs::Chunk]) -> io::Result<Arc<dyn Chunks>> {
        Ok(Arc::new(RunChunks {
            shared: Arc::clone(&self.shared),
            chunks: engine_allocation::copy_vec(chunks)?,
        }))
    }
}

/// One table's rows a frame changed since the frame before, by key, ascending: inserted (or
/// built anew), gone, and written with the columns written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyChanges {
    pub inserted: Vec<u128>,
    pub deleted: Vec<u128>,
    pub written: Vec<(u128, u64)>,
}

/// Some tables' frames at any hour, as the row decoder makes them, read from a change-run
/// index: moves take the cursor to an hour and note what differs into the views; a build makes
/// the frame of the hour moved to last from every move since the last build.
pub struct Snapshots {
    file: Arc<IndexFile>,
    cursor: Cursor,
    views: Views,
    /// The hour moved to last, and its metrics.
    at: Option<(u64, i32, Metrics)>,
    /// The views were left part noted or built: the next move notes them anew.
    broken: bool,
    /// The moves and builds made so far.
    moves: usize,
    builds: usize,
}

impl Snapshots {
    /// Frames of `tables` and Campaign, opaque cells encoded at `format`, as a recording of
    /// `mode` holds them, each move on up to `workers` threads.
    pub fn new(
        file: Arc<IndexFile>,
        tables: &[TableId],
        format: FormatVersion,
        mode: CaptureMode,
        workers: usize,
    ) -> io::Result<Self> {
        let views = Views::new(tables, format, mode)?;
        let held: Vec<usize> = views.selected().iter().map(|table| table.index()).collect();
        Ok(Self {
            cursor: Cursor::new(&file.index, schema()?, &held, workers)?,
            views,
            file,
            at: None,
            broken: false,
            moves: 0,
            builds: 0,
        })
    }

    /// Complete views carrying change facts and field stamps, as a decoder with change facts
    /// makes them.
    pub fn with_change_facts(mut self) -> Self {
        self.views = self.views.with_change_facts();
        self
    }

    pub fn set_workers(&mut self, workers: usize) {
        self.cursor.set_workers(workers);
    }

    pub fn file(&self) -> &Arc<IndexFile> {
        &self.file
    }

    /// The tables its frames hold, Campaign among them, in table order.
    pub fn tables(&self) -> &[TableId] {
        self.views.selected()
    }

    /// The pages the last move read, and their raw bytes.
    pub fn read(&self) -> (usize, usize) {
        self.cursor.read()
    }

    /// The moves and the frame builds made so far.
    pub fn counts(&self) -> (usize, usize) {
        (self.moves, self.builds)
    }

    /// The bytes the cursor's tables and strings hold.
    pub fn held_bytes(&self) -> usize {
        self.cursor.state().held_bytes() + self.views.heap_bytes()
    }

    /// Moves to `hour` of `branch`, building no frame, its cursor's workers each first running
    /// `on_worker`: false when no run holds the hour, which is then the recording's gap.
    pub fn move_to(
        &mut self,
        branch: u64,
        hour: i32,
        on_worker: &(dyn Fn() + Sync),
    ) -> io::Result<bool> {
        let Some((run, ordinal)) = self.file.index.find(branch, hour) else {
            return Ok(false);
        };
        self.at = None;
        self.moves += 1;
        let file = &*self.file;
        if let Err(error) = self
            .cursor
            .seek(&file.index, file, run, ordinal, on_worker)
        {
            self.broken = true;
            return Err(error);
        }
        // A run of a native recording withholding a table reads as that recording does.
        let withheld = file.summary.withheld_in(run);
        for table in self.views.selected() {
            let reason = withheld
                .iter()
                .find(|&&(held, _)| held == table.index())
                .map(|(_, reason)| reason.as_str());
            if let Err(error) = self.cursor.withhold(table.index(), reason) {
                self.broken = true;
                return Err(error);
            }
        }
        let held = &file.index.runs[run as usize];
        let state = self.cursor.state();
        let mut sections = Vec::new();
        for table in self.views.selected() {
            let changes = state
                .changes(table.index())
                .ok_or_else(|| invalid("The cursor holds no such table"))?;
            if !(changes.changed.is_empty()
                && changes.deleted.is_empty()
                && changes.inserted.is_empty())
            {
                sections.push(Section {
                    table: table.index(),
                    bytes: 0,
                    columns: Vec::new(),
                    ops: 0,
                });
            }
        }
        let recorded = held.metrics[ordinal as usize];
        let manifest = Manifest {
            branch,
            hour,
            flags: 0,
            total_ns: recorded.total_ns,
            pool_bytes: recorded.pool_bytes,
            dropped_frames: recorded.dropped_frames,
            appends: Vec::new(),
            statuses: Vec::new(),
            read: held
                .read_bits(ordinal)
                .ok_or_else(|| invalid("A run index holds no read bits at the ordinal"))?
                .to_vec(),
            samples: Vec::new(),
            sections,
        };
        let restart = self.cursor.fresh() || self.broken;
        self.broken = true;
        self.views.note(state, &manifest, restart)?;
        self.broken = false;
        self.at = Some((branch, hour, metrics(&manifest)));
        Ok(true)
    }

    /// The frame of the hour moved to last, as walking the recording would have built it,
    /// views built anew on `parallel`'s workers when given.
    pub fn build(&mut self, parallel: Option<&Parallel<'_>>) -> io::Result<DecodedFrame> {
        let (branch, hour, metrics) = self
            .at
            .clone()
            .ok_or_else(|| invalid("A frame is built before any move"))?;
        self.builds += 1;
        let built = self
            .views
            .build(self.cursor.state(), (branch, hour), metrics, parallel);
        if built.is_err() {
            self.broken = true;
            self.at = None;
        }
        built
    }

    /// The tables' frame at `hour` of `branch`; none when no run holds the hour.
    pub fn frame(
        &mut self,
        branch: u64,
        hour: i32,
        parallel: Option<&Parallel<'_>>,
    ) -> io::Result<Option<DecodedFrame>> {
        let alone = || {};
        let on_worker =
            parallel.map_or(&alone as &(dyn Fn() + Sync), |parallel| parallel.on_worker);
        if !self.move_to(branch, hour, on_worker)? {
            return Ok(None);
        }
        self.build(parallel).map(Some)
    }

    /// `table`'s rows the last move changed since the one before it: every row of a move from
    /// nothing. None for a table these frames do not hold.
    pub fn changed(&self, table: TableId) -> Option<KeyChanges> {
        if !self.views.selected().contains(&table) {
            return None;
        }
        let state = self.cursor.state();
        let (held, changes) = (state.table(table.index())?, state.changes(table.index())?);
        let mut inserted: Vec<u128> = changes
            .inserted
            .iter()
            .filter_map(|&slot| held.key(slot))
            .collect();
        inserted.sort_unstable();
        // A key that moved to another slot is built anew, not gone.
        let mut deleted: Vec<u128> = changes
            .deleted_keys
            .iter()
            .copied()
            .filter(|key| inserted.binary_search(key).is_err())
            .collect();
        deleted.sort_unstable();
        let mut written: Vec<(u128, u64)> = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .filter_map(|(&slot, &mask)| Some((held.key(slot)?, mask)))
            .collect();
        written.sort_unstable();
        Some(KeyChanges {
            inserted,
            deleted,
            written,
        })
    }
}
