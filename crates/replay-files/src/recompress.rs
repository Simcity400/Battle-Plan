
use replay_settings::publication;
use replay_data::rows::TABLE_COUNT;
use engine_allocation::zstd::{self, Compressor, Decompressor, Holder};
use replay_columnar::{
    container::{self, ContainerWriter, Open, PAGE_RAW_BYTES, ReadAt, Root, Saved, Section, SectionKey, Tags, schema},
    envelope::{self, Extent, FOOTER_BYTES, HEADER_BYTES, Kind, RowPlace, SPAN_BLOCKS, SpanPages},
    pages::{Block, BlockHead, Entry, RunDirectory, SpanDirectory, SpanRow},
    summary,
    unit::{self, FrameEntry, TableEntry, get_varint},
};
use std::{
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    fmt,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
};
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

#[cfg(test)]
mod tests;
#[cfg(test)]
use std::collections::BTreeSet;

#[derive(Debug)]
pub enum Kept {
    Recording,
    OpenElsewhere,
    Changed,
    Stopped,
    Failed(String),
}

impl fmt::Display for Kept {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Recording => formatter.write_str("still being written by BattlePlan Recorder"),
            Self::OpenElsewhere => formatter.write_str("open in the desktop app or another program"),
            Self::Changed => formatter.write_str("it changed while it was being compressed"),
            Self::Stopped => formatter.write_str("stopped before the smaller file was whole"),
            Self::Failed(why) => formatter.write_str(why),
        }
    }
}

impl From<io::Error> for Kept {
    fn from(error: io::Error) -> Self {
        Self::Failed(error.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Replaced { before: u64, after: u64, unflushed: Option<String> },
    AlreadyAt(i32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub units: u64,
    pub total: u64,
    pub read: u64,
    pub written: u64,
}

pub fn partials(path: &Path) -> io::Result<Vec<PathBuf>> {
    let digest = staging_digest(path);
    let folder = folder_of(path);
    Ok(leftovers(folder)?.into_iter().filter(|(_, held)| *held == digest).map(|(staged, _)| staged).collect())
}

fn folder_of(path: &Path) -> &Path {
    path.parent().filter(|folder| !folder.as_os_str().is_empty()).unwrap_or(Path::new("."))
}

pub fn leftovers(folder: &Path) -> io::Result<Vec<(PathBuf, String)>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(digest) = name.to_str().and_then(staged_name) {
            found.push((entry.path(), digest.to_owned()));
        }
    }
    found.sort();
    Ok(found)
}

pub fn staging_digest(path: &Path) -> String {
    let stem = publication::temporary_beside(path, "recompress");
    let name = stem.file_name().unwrap_or_default().to_string_lossy();
    name.split('.').next().unwrap_or_default().to_owned()
}

fn staged_name(name: &str) -> Option<&str> {
    let (digest, rest) = name.split_once(".recompress.")?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let attempt = rest == "tmp" || rest.strip_suffix(".tmp").and_then(|attempt| attempt.split_once('-')).is_some_and(|(process, attempt)| digits(process) && digits(attempt));
    (digest.len() == 32 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) && attempt).then_some(digest)
}

#[derive(Debug)]
struct Staged {
    path: PathBuf,
    file: Option<File>,
}

impl Staged {
    fn beside(path: &Path) -> Result<Self, Kept> {
        static ATTEMPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        loop {
            let attempt = ATTEMPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let staged = publication::temporary_beside(path, &format!("recompress.{}-{attempt}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .access_mode(GENERIC_WRITE | DELETE)
                .share_mode(0)
                .open(&staged)
            {
                Ok(file) => return Ok(Self { path: staged, file: Some(file) }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(Kept::Failed(format!("{}: {error}", staged.display()))),
            }
        }
    }

    fn file(&self) -> &File {
        self.file.as_ref().expect("a staged file is held until it is published or dropped")
    }

    fn keep(mut self) {
        self.file = None;
    }
}

impl std::borrow::Borrow<File> for Staged {
    fn borrow(&self) -> &File {
        self.file()
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub fn level_of(path: &Path) -> io::Result<Option<i32>> {
    let file = shared(path)?;
    let Some((root, _)) = container::locate(&file, Open::Saved)? else { return Ok(None) };
    Ok(container::read_root(&file, &root)?.level)
}

pub fn recompress_file(
    path: &Path,
    level: i32,
    below_normal: bool,
    progress: impl FnMut(&Progress) -> bool,
) -> Result<Outcome, Kept> {
    if below_normal {
        enter_below_normal()?;
    }
    let stage = || {
        for partial in partials(path)? {
            let _ = std::fs::remove_file(partial);
        }
        Staged::beside(path)
    };
    match write_paced(path, stage, level, below_normal, progress, Pace::here())? {
        Written::Already(level) => {
            for partial in partials(path)? {
                let _ = std::fs::remove_file(partial);
            }
            Ok(Outcome::AlreadyAt(level))
        }
        Written::Whole(original, staged) => publish(path, staged, original),
    }
}

#[derive(Debug)]
enum Written<S> {
    Already(i32),
    Whole(Original, S),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Original {
    length: u64,
    footer: [u8; FOOTER_BYTES],
}

impl Original {
    fn of(file: &File) -> io::Result<Self> {
        let length = file.metadata()?.len();
        let mut footer = [0; FOOTER_BYTES];
        if length >= (HEADER_BYTES + FOOTER_BYTES) as u64 {
            ReadAt::read_at(file, length - FOOTER_BYTES as u64, &mut footer)?;
        }
        Ok(Self { length, footer })
    }
}

fn write_paced<S: std::borrow::Borrow<File>>(path: &Path, stage: impl FnOnce() -> Result<S, Kept>, level: i32, below_normal: bool, mut progress: impl FnMut(&Progress) -> bool, pace: Pace) -> Result<Written<S>, Kept> {
    refuse_writer(path)?;
    let file = shared(path)?;
    let original = Original::of(&file)?;
    let (extent, through) = container::locate(&file, Open::Newest)?.ok_or(Kept::Recording)?;
    let root = container::read_root(&file, &extent)?;
    if through != original.length || root.saved != Saved::Yes || root.state.is_some() {
        return Err(Kept::Recording);
    }
    if root.level == Some(level) {
        return Ok(Written::Already(level));
    }
    let source = Source { file: &file, length: through };
    let staged = stage()?;
    let tracked = Tracked { file: BufWriter::with_capacity(1 << 20, std::borrow::Borrow::<File>::borrow(&staged)), at: 0 };
    let mut writer = ContainerWriter::create_after(tracked, root.identity.clone(), level, root.revision, None)?;
    for section in &root.sections {
        writer.section(section.key, section.version)?;
    }
    rewrite_world_log(&source, &root, &mut writer, level, below_normal, &mut progress, pace)?;
    rewrite_sections(&source, &root, &mut writer, level)?;
    writer.commit(None, true, Tracked::sync)?;
    writer.into_inner().finish()?;
    Ok(Written::Whole(original, staged))
}

fn publish(path: &Path, staged: Staged, original: Original) -> Result<Outcome, Kept> {
    let folder = folder_of(path);
    let _names = crate::recording_files::NamesLock::wait(folder)?;
    refuse_writer(path)?;
    let held = exclusive(path)?;
    if Original::of(&held)? != original {
        return Err(Kept::Changed);
    }
    let after = staged.file().metadata()?.len();
    let unflushed = match publication::replace_held(staged.file(), path, held) {
        Ok(()) => None,
        Err(publication::ReplaceError::Unflushed(error)) => Some(error.to_string()),
        Err(publication::ReplaceError::Unrenamed(error)) => return Err(Kept::Failed(format!("{}: {error}", path.display()))),
    };
    staged.keep();
    Ok(Outcome::Replaced { before: original.length, after, unflushed })
}

fn refuse_writer(path: &Path) -> Result<(), Kept> {
    match OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(error) if error.raw_os_error() == Some(32) => Err(Kept::Recording),
        Err(error) => Err(Kept::Failed(format!("{}: {error}", path.display()))),
    }
}

fn exclusive(path: &Path) -> Result<File, Kept> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_DELETE)
        .open(path)
        .map_err(|error| match error.raw_os_error() {
            Some(32) => Kept::OpenElsewhere,
            _ => Kept::Failed(format!("{}: {error}", path.display())),
        })
}

fn shared(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

pub fn enter_below_normal() -> io::Result<()> {
    use windows_sys::Win32::System::Threading::{BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
    if unsafe { SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) } == 0 {
        return Err(io::Error::last_os_error());
    }
    below_normal_thread()
}

pub fn below_normal_thread() -> io::Result<()> {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn priority() -> (u32, i32) {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetPriorityClass, GetThreadPriority};
    unsafe { (GetPriorityClass(GetCurrentProcess()), GetThreadPriority(GetCurrentThread())) }
}

struct Tracked<'f> {
    file: BufWriter<&'f File>,
    at: u64,
}

impl Write for Tracked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.at += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl Seek for Tracked<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match to {
            SeekFrom::Current(0) => Ok(self.at),
            SeekFrom::Start(at) if at == self.at => Ok(at),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "A recompressed recording is written straight through")),
        }
    }
}

impl Tracked<'_> {
    fn sync(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.get_ref().sync_data()
    }

    fn finish(self) -> io::Result<()> {
        let file = self.file.into_inner().map_err(io::IntoInnerError::into_error)?;
        file.sync_all()
    }
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

struct Source<'f> {
    file: &'f File,
    length: u64,
}

impl Source<'_> {
    fn read(&self, offset: u64, stored: u64, out: &mut Vec<u8>) -> io::Result<()> {
        let inside = offset >= HEADER_BYTES as u64 && offset.checked_add(stored).is_some_and(|end| end <= self.length - FOOTER_BYTES as u64);
        if !inside {
            return Err(invalid("A recording names bytes outside its extents"));
        }
        out.clear();
        engine_allocation::reserve_exact(out, usize::try_from(stored).map_err(|_| engine_allocation::oom())?)?;
        out.resize(stored as usize, 0);
        ReadAt::read_at(self.file, offset, out)
    }

    fn stored(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        self.read(extent.offset, extent.stored, &mut out)?;
        if envelope::digest(&out) != extent.digest {
            return Err(extent.damaged("differs from its digest"));
        }
        Ok(out)
    }

    fn raw(&self, extent: &Extent) -> io::Result<Vec<u8>> {
        container::decompress(&self.stored(extent)?, Some(extent.raw)).map_err(|error| extent.damaged(&error.to_string()))
    }

    fn frame(&self, frame: &FrameEntry, out: &mut Vec<u8>) -> io::Result<()> {
        self.read(frame.offset, u64::from(frame.stored), out)?;
        if envelope::digest(out) != frame.digest {
            return Err(invalid("A unit frame differs from its digest"));
        }
        Ok(())
    }

    fn section(&self, section: &Section) -> io::Result<Vec<Extent>> {
        let mut out = Vec::new();
        for (level, held) in section.frozen.levels().iter().enumerate().rev() {
            for extent in held {
                match level {
                    0 => {
                        container::check_follows(section.key, out.last(), extent, self.length)?;
                        out.push(*extent);
                    }
                    _ => self.expand(section.key, extent, &mut out)?,
                }
            }
        }
        out.extend_from_slice(&section.open);
        for (at, extent) in (0_u64..).zip(&out) {
            container::check_owner(section.key, at, extent)?;
        }
        Ok(out)
    }

    fn expand(&self, key: SectionKey, page: &Extent, out: &mut Vec<Extent>) -> io::Result<()> {
        if page.raw > PAGE_RAW_BYTES {
            return Err(page.damaged(&format!("is larger than any index page of {key}")));
        }
        for entry in container::read_page(&self.raw(page)?, page, key)? {
            if page.stream == 1 {
                container::check_follows(key, out.last(), &entry, self.length)?;
                out.push(entry);
            } else {
                self.expand(key, &entry, out)?;
            }
        }
        Ok(())
    }
}

static RECOMPRESS: Holder = Holder::new("zstd: recompress");

const WINDOW_PER_WORKER: u64 = 8 << 20;

struct Pace {
    workers: usize,
    window: u64,
    #[cfg(test)]
    hold: Option<Arc<tests::Hold>>,
    #[cfg(test)]
    parts: Option<Arc<tests::PartsHold>>,
}

impl Pace {
    fn here() -> Self {
        let workers = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        Self {
            workers,
            window: workers as u64 * WINDOW_PER_WORKER,
            #[cfg(test)]
            hold: None,
            #[cfg(test)]
            parts: None,
        }
    }
}

struct Codec {
    compressor: Compressor,
    decompressor: Decompressor,
}

impl Codec {
    fn new(level: i32) -> io::Result<Self> {
        Ok(Self { compressor: frame_compressor(level)?, decompressor: Decompressor::new(&RECOMPRESS)? })
    }

    fn ready(slot: &mut Option<Self>, level: i32) -> io::Result<&mut Self> {
        if slot.is_none() {
            *slot = Some(Self::new(level)?);
        }
        Ok(slot.as_mut().expect("made"))
    }

    fn decode(&mut self, stored: &[u8], raw: usize, prefix: Option<&[u8]>) -> io::Result<Vec<u8>> {
        let mut out = engine_allocation::vec(raw)?;
        let decoded = match prefix {
            Some(prefix) => self.decompressor.decompress_append_prefixed(prefix, stored, &mut out),
            None => self.decompressor.decompress_append(stored, &mut out),
        };
        decoded.map_err(|error| error.refused(|refusal| invalid(format!("A frame does not decode: {refusal}"))))?;
        if out.len() != raw {
            return Err(invalid("A frame decodes to another length than its entry's"));
        }
        Ok(out)
    }

    fn compress(&mut self, bytes: &[u8], prefix: Option<&[u8]>) -> io::Result<Vec<u8>> {
        let mut frame = engine_allocation::vec(zstd::bound(bytes.len()))?;
        let compressed = match prefix {
            Some(prefix) => self.compressor.compress_append_prefixed(prefix, bytes, &mut frame),
            None => self.compressor.compress_append(bytes, &mut frame),
        };
        compressed.map_err(zstd::Error::invalid)?;
        if self.decode(&frame, bytes.len(), prefix)? != bytes {
            return Err(invalid("A frame compressed again decodes to other bytes"));
        }
        Ok(frame)
    }
}

fn frame_compressor(level: i32) -> io::Result<Compressor> {
    let mut compressor = Compressor::new(&RECOMPRESS, level)?;
    compressor.cap_window(level, unit::Settings::at(level).window_log)?;
    Ok(compressor)
}

#[derive(Clone)]
struct Slice {
    bytes: Arc<Vec<u8>>,
    start: usize,
    end: usize,
}

impl Slice {
    fn get(&self) -> &[u8] {
        &self.bytes[self.start..self.end]
    }
}

#[derive(Default)]
struct Decoded {
    frames: Vec<Arc<Vec<u8>>>,
    streams: Vec<(u32, usize, usize, usize)>,
}

impl Decoded {
    fn stream(&self, id: u32) -> Option<Slice> {
        self.streams
            .iter()
            .find(|held| held.0 == id)
            .map(|&(_, frame, start, end)| Slice { bytes: Arc::clone(&self.frames[frame]), start, end })
    }

    fn push(&mut self, frame: &FrameEntry, raw: Arc<Vec<u8>>) -> io::Result<()> {
        let index = self.frames.len();
        if !frame.small {
            if let Some(&id) = frame.streams.first() {
                self.streams.push((id, index, 0, raw.len()));
            }
        } else {
            let mut at = 0;
            let count = get_varint(&raw, &mut at)? as usize;
            if count > raw.len() {
                return Err(invalid("A frame of small streams counts more than it holds"));
            }
            let mut heads = Vec::with_capacity(count);
            for _ in 0..count {
                heads.push((get_varint(&raw, &mut at)? as u32, get_varint(&raw, &mut at)?));
            }
            for (id, length) in heads {
                let end = (at as u64).checked_add(length).filter(|&end| end <= raw.len() as u64).ok_or_else(|| invalid("A small stream passes its frame"))? as usize;
                self.streams.push((id, index, at, end));
                at = end;
            }
        }
        self.frames.push(raw);
        Ok(())
    }
}

enum Item {
    Unit { run: u32, entry: TableEntry, again: Vec<bool>, summary: Option<Extent> },
    Head { run: u32, head: BlockHead },
    Block { old: Box<Block>, page: Option<Extent> },
    Span { pages: SpanPages, chain: [u8; 32], pieces: Vec<Vec<(u8, Extent)>> },
    Run { index: u32, extent: Extent, directory: RunDirectory },
}

enum Task {
    Parts(Vec<Vec<FrameEntry>>),
    Compress { bytes: Arc<Vec<u8>>, prefix: Option<Slice> },
    Frame { frame: FrameEntry, prefix: Option<FrameEntry> },
    Piece(Extent),
}

struct Job {
    seq: u64,
    slot: u32,
    weight: u64,
    task: Task,
    #[cfg(test)]
    unit: bool,
}

impl Job {
    fn key(&self) -> (u64, u32) {
        (self.seq, self.slot)
    }
}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.key().cmp(&self.key())
    }
}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Job {}

enum Message {
    Done {
        seq: u64,
        slot: Option<u32>,
        weight: u64,
        read: u64,
        out: Vec<u8>,
        #[cfg(test)]
        unit: bool,
    },
    Failed(io::Error),
}

struct Plan {
    seq: u64,
    #[cfg(test)]
    unit: bool,
    jobs: Vec<Job>,
    slots: u32,
    missing: usize,
    weight: u64,
}

impl Plan {
    fn of(seq: u64, item: &Item) -> io::Result<Self> {
        let mut plan = Self {
            seq,
            #[cfg(test)]
            unit: matches!(item, Item::Unit { .. }),
            jobs: Vec::new(),
            slots: 0,
            missing: 0,
            weight: 0,
        };
        match item {
            Item::Unit { entry, again, summary, .. } => {
                let parts: Vec<Vec<FrameEntry>> = entry.parts.iter().map(|part| part.frames.clone()).collect();
                let frames = parts.iter().flatten().count();
                if frames > 0 {
                    plan.weight += parts.iter().flatten().map(|frame| u64::from(frame.raw)).sum::<u64>();
                    plan.push(0, Task::Parts(parts));
                    plan.slots = u32::try_from(frames).map_err(|_| invalid("A unit names more than 2^32 frames"))?;
                    plan.missing = frames + 1;
                }
                let mut named = again.iter();
                for restore in [&entry.first, &entry.restore].into_iter().flatten() {
                    for chunk in restore.streams.iter().flat_map(|(_, chunks)| chunks) {
                        if *named.next().ok_or_else(|| invalid("A unit's restore chunks are not all classed"))? {
                            plan.slots += 1;
                            continue;
                        }
                        let weight = u64::from(chunk.frame.raw) + chunk.prefix.as_ref().map_or(0, |prefix| u64::from(prefix.raw));
                        plan.job(weight, Task::Frame { frame: chunk.frame.clone(), prefix: chunk.prefix.clone() });
                    }
                }
                if let Some(summary) = summary {
                    plan.job(summary.stored, Task::Piece(*summary));
                }
            }
            Item::Head { head, .. } => {
                for frame in std::iter::once(&head.clock).chain(head.strings.iter().map(|chunk| &chunk.frame)) {
                    plan.job(u64::from(frame.raw), Task::Frame { frame: frame.clone(), prefix: None });
                }
            }
            Item::Span { pieces, .. } => {
                for (_, extent) in pieces.iter().flatten() {
                    plan.job(extent.stored, Task::Piece(*extent));
                }
            }
            Item::Run { directory, .. } => {
                for (_, extent) in &directory.tail {
                    plan.job(extent.stored, Task::Piece(*extent));
                }
            }
            Item::Block { .. } => {}
        }
        Ok(plan)
    }

    fn push(&mut self, weight: u64, task: Task) {
        self.jobs.push(Job {
            seq: self.seq,
            slot: self.slots,
            weight,
            task,
            #[cfg(test)]
            unit: self.unit,
        });
    }

    fn job(&mut self, weight: u64, task: Task) {
        self.push(weight, task);
        self.slots += 1;
        self.missing += 1;
        self.weight += weight;
    }
}

struct Queue {
    state: Mutex<State>,
    work: Condvar,
    given: Condvar,
    #[cfg(test)]
    workers: usize,
    #[cfg(test)]
    hold: Option<Arc<tests::Hold>>,
    #[cfg(test)]
    parts: Option<Arc<tests::PartsHold>>,
}

#[derive(Default)]
struct State {
    tasks: BinaryHeap<Job>,
    given: Vec<Message>,
    closed: bool,
    #[cfg(test)]
    idle: usize,
    #[cfg(test)]
    waiting: bool,
    #[cfg(test)]
    units: BTreeSet<u64>,
    #[cfg(test)]
    applied: u64,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn push(&self, jobs: Vec<Job>) {
        let count = jobs.len();
        if count == 0 {
            return;
        }
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.tasks.extend(jobs);
        drop(state);
        if count == 1 {
            self.work.notify_one();
        } else {
            self.work.notify_all();
        }
    }

    fn next(&self) -> Option<Job> {
        let mut state = self.lock();
        loop {
            if state.closed {
                return None;
            }
            if let Some(job) = state.tasks.pop() {
                return Some(job);
            }
            #[cfg(test)]
            {
                state.idle += 1;
                self.given.notify_all();
            }
            state = self.work.wait(state).unwrap_or_else(PoisonError::into_inner);
            #[cfg(test)]
            {
                state.idle -= 1;
            }
        }
    }

    fn give(&self, message: Message) {
        let mut state = self.lock();
        #[cfg(test)]
        state.gave(&message);
        state.given.push(message);
        drop(state);
        self.given.notify_all();
    }

    fn take(&self) -> Vec<Message> {
        let mut state = self.lock();
        while state.given.is_empty() {
            #[cfg(test)]
            {
                state.waiting = true;
                self.given.notify_all();
            }
            state = self.given.wait(state).unwrap_or_else(PoisonError::into_inner);
            #[cfg(test)]
            {
                state.waiting = false;
            }
        }
        std::mem::take(&mut state.given)
    }

    fn close(&self) {
        self.lock().closed = true;
        self.work.notify_all();
        self.given.notify_all();
    }
}

struct Closing<'q>(&'q Queue);

impl Drop for Closing<'_> {
    fn drop(&mut self) {
        self.0.close();
    }
}

fn worker(source: &Source<'_>, queue: &Queue, level: i32, below_normal: bool) {
    if below_normal {
        let _ = below_normal_thread();
    }
    let mut codec = None;
    while let Some(job) = queue.next() {
        queue.give(Codec::ready(&mut codec, level).and_then(|codec| run(source, codec, queue, job, level)).unwrap_or_else(Message::Failed));
    }
}

fn run(source: &Source<'_>, codec: &mut Codec, queue: &Queue, job: Job, level: i32) -> io::Result<Message> {
    let mut read = 0_u64;
    let out = match job.task {
        Task::Parts(parts) => {
            let mut stored = Vec::new();
            let mut previous = Decoded::default();
            let mut slot = job.slot;
            for part in &parts {
                let mut current = Decoded::default();
                for frame in part {
                    source.frame(frame, &mut stored)?;
                    read += stored.len() as u64;
                    let prefix = if frame.chained {
                        Some(
                            frame
                                .streams
                                .first()
                                .and_then(|&id| previous.stream(id))
                                .ok_or_else(|| invalid("A chained frame's stream is in no part before it in its block"))?,
                        )
                    } else {
                        None
                    };
                    let bytes = Arc::new(codec.decode(&stored, frame.raw as usize, prefix.as_ref().map(Slice::get))?);
                    queue.push(vec![Job {
                        seq: job.seq,
                        slot,
                        weight: u64::from(frame.raw),
                        task: Task::Compress { bytes: Arc::clone(&bytes), prefix },
                        #[cfg(test)]
                        unit: job.unit,
                    }]);
                    slot += 1;
                    current.push(frame, bytes)?;
                }
                previous = current;
            }
            #[cfg(test)]
            queue.hold_parts(job.seq);
            return Ok(Message::Done {
                seq: job.seq,
                slot: None,
                weight: 0,
                read,
                out: Vec::new(),
                #[cfg(test)]
                unit: job.unit,
            });
        }
        Task::Compress { bytes, prefix } => {
            #[cfg(test)]
            queue.hold(job.seq, bytes.len());
            codec.compress(&bytes, prefix.as_ref().map(Slice::get))?
        }
        Task::Frame { frame, prefix } => {
            let mut stored = Vec::new();
            source.frame(&frame, &mut stored)?;
            read += stored.len() as u64;
            let prefix = match prefix {
                Some(prefix) => {
                    let mut held = Vec::new();
                    source.frame(&prefix, &mut held)?;
                    Some(codec.decode(&held, prefix.raw as usize, None)?)
                }
                None => None,
            };
            let bytes = codec.decode(&stored, frame.raw as usize, prefix.as_deref())?;
            #[cfg(test)]
            queue.hold(job.seq, bytes.len());
            codec.compress(&bytes, prefix.as_deref())?
        }
        Task::Piece(extent) => {
            read += extent.stored;
            summary::recompress(&source.stored(&extent)?, level)?
        }
    };
    Ok(Message::Done {
        seq: job.seq,
        slot: Some(job.slot),
        weight: job.weight,
        read,
        out,
        #[cfg(test)]
        unit: job.unit,
    })
}

struct Walk<'s, 'f> {
    source: &'s Source<'f>,
    runs: Vec<(Extent, RunDirectory)>,
    run: usize,
    block: u32,
    span: Option<(u32, SpanDirectory, Vec<SpanRow>)>,
    seen: Vec<HashSet<u64>>,
}

impl Walk<'_, '_> {
    fn next(&mut self) -> io::Result<Vec<Item>> {
        let mut items = Vec::new();
        let Some((extent, directory)) = self.runs.get(self.run) else { return Ok(items) };
        let index = self.run as u32;
        if self.block < directory.blocks() {
            let block = self.block;
            let span = block / SPAN_BLOCKS;
            let closed = (span as usize) < directory.spans.len();
            let (old, page) = if closed {
                if self.span.as_ref().is_none_or(|(held, ..)| *held != span) {
                    self.span = Some(read_span(self.source, index, &directory.spans[span as usize])?);
                }
                let (_, held, rows) = self.span.as_ref().expect("read");
                let at = (block % SPAN_BLOCKS) as usize;
                let tables = rows.iter().map(|row| row.blocks[at].clone()).collect();
                (Block { run: index, block, head: held.blocks[at].clone(), tables }, None)
            } else {
                let page = directory.open[(block - directory.spans.len() as u32 * SPAN_BLOCKS) as usize];
                (Block::read(&self.source.raw(&page)?, &page, TABLE_COUNT)?, Some(page))
            };
            for (table, entry) in old.tables.iter().enumerate() {
                let again = classify(&mut self.seen[table], &entry.unit);
                items.push(Item::Unit { run: index, entry: entry.unit.clone(), again, summary: entry.summary });
            }
            items.push(Item::Head { run: index, head: old.head.clone() });
            items.push(Item::Block { old: Box::new(old), page });
            self.block += 1;
            if closed && self.block % SPAN_BLOCKS == 0 {
                let (_, held, rows) = self.span.take().expect("read");
                let pieces = rows.into_iter().map(|row| row.summaries).collect();
                items.push(Item::Span { pages: directory.spans[span as usize], chain: held.chain, pieces });
            }
        }
        if self.block == directory.blocks() {
            items.push(Item::Run { index, extent: *extent, directory: directory.clone() });
            self.run += 1;
            self.block = 0;
        }
        Ok(items)
    }
}

fn read_span(source: &Source<'_>, run: u32, pages: &SpanPages) -> io::Result<(u32, SpanDirectory, Vec<SpanRow>)> {
    let directory = SpanDirectory::read(&source.raw(&pages.directory)?, &pages.directory, TABLE_COUNT)?;
    if pages.index.stored != pages.index.raw {
        return Err(pages.index.damaged("is stored compressed"));
    }
    let index = source.stored(&pages.index)?;
    let mut rows = Vec::with_capacity(TABLE_COUNT);
    for place in &directory.rows {
        let bytes = usize::try_from(place.offset)
            .ok()
            .and_then(|start| index.get(start..start.checked_add(place.length as usize)?))
            .ok_or_else(|| pages.index.damaged("names a row past its bytes"))?;
        if envelope::digest(bytes) != place.digest {
            return Err(pages.index.damaged("holds a row that differs from its digest"));
        }
        rows.push(SpanRow::read(bytes, run, place.table, SPAN_BLOCKS)?);
    }
    Ok((directory.first_block / SPAN_BLOCKS, directory, rows))
}

fn classify(seen: &mut HashSet<u64>, entry: &TableEntry) -> Vec<bool> {
    let mut again = Vec::new();
    for restore in [&entry.first, &entry.restore].into_iter().flatten() {
        let mut whole = HashSet::new();
        for (_, chunks) in &restore.streams {
            for chunk in chunks {
                again.push(seen.contains(&chunk.frame.offset));
                if restore.whole {
                    whole.insert(chunk.frame.offset);
                }
            }
        }
        if restore.whole {
            *seen = whole;
        }
    }
    again
}

#[derive(Default)]
struct RunOut {
    chain: [u8; 32],
    read_chain: [u8; 32],
    spans: Vec<SpanPages>,
    open: Vec<Extent>,
    blocks: Vec<Block>,
    tables: Vec<Entry>,
    head: Option<BlockHead>,
}

fn tags(extent: &Extent) -> Tags {
    Tags { run: extent.run, stream: extent.stream, first: extent.first, hours: extent.hours }
}

struct Out<'w, W: Write + Seek> {
    writer: &'w mut ContainerWriter<W>,
    pages: Compressor,
    run: Option<RunOut>,
    runs: Vec<Extent>,
    wholes: Vec<HashMap<u64, FrameEntry>>,
}

impl<W: Write + Seek> Out<'_, W> {
    fn run(&mut self) -> &mut RunOut {
        self.run.get_or_insert_with(RunOut::default)
    }

    fn frame(&mut self, run: u32, old: &FrameEntry, stored: &[u8]) -> io::Result<FrameEntry> {
        let extent = self.writer.put(Kind::Block, Tags { run, ..Tags::default() }, 0, stored)?;
        Ok(FrameEntry {
            streams: old.streams.clone(),
            offset: extent.offset,
            stored: u32::try_from(stored.len()).map_err(|_| invalid("A frame passes 2^32 stored bytes"))?,
            raw: old.raw,
            digest: extent.digest,
            chained: old.chained,
            small: old.small,
        })
    }

    fn piece(&mut self, old: &Extent, piece: &[u8]) -> io::Result<Extent> {
        self.writer.put(Kind::Summary, tags(old), piece.len() as u64, piece)
    }

    fn page(&mut self, kind: Kind, tags: Tags, raw: &[u8]) -> io::Result<Extent> {
        let mut stored = engine_allocation::vec(zstd::bound(raw.len()))?;
        self.pages.compress_append(raw, &mut stored).map_err(zstd::Error::invalid)?;
        self.writer.put(kind, tags, raw.len() as u64, &stored)
    }

    fn apply(&mut self, item: Item, slots: Vec<Option<Vec<u8>>>) -> io::Result<()> {
        let mut slots = slots.into_iter();
        match item {
            Item::Unit { run, mut entry, summary, .. } => {
                let table = self.run().tables.len();
                let mut next = || slots.next().ok_or_else(|| invalid("A unit's frames came back short"));
                for part in &mut entry.parts {
                    for frame in &mut part.frames {
                        let stored = next()?.ok_or_else(|| invalid("A part's frame is named again"))?;
                        *frame = self.frame(run, frame, &stored)?;
                    }
                }
                let wholes = self.wholes.get_mut(table).ok_or_else(|| invalid("A block names more tables than the schema"))?;
                let mut written = std::mem::take(wholes);
                for restore in [&mut entry.first, &mut entry.restore].into_iter().flatten() {
                    let mut whole = HashMap::new();
                    for (_, chunks) in &mut restore.streams {
                        for chunk in chunks {
                            let read = chunk.frame.offset;
                            let new = match next()? {
                                Some(stored) => self.frame(run, &chunk.frame, &stored)?,
                                None => written.get(&read).cloned().ok_or_else(|| invalid("A restore names again a frame of no whole restore before it"))?,
                            };
                            if let Some(prefix) = &mut chunk.prefix {
                                *prefix = written.get(&prefix.offset).cloned().ok_or_else(|| invalid("A restore chunk's prefix is no whole restore's frame"))?;
                            }
                            if restore.whole {
                                whole.insert(read, new.clone());
                            }
                            chunk.frame = new;
                        }
                    }
                    if restore.whole {
                        written = whole;
                    }
                }
                self.wholes[table] = written;
                let summary = match summary {
                    Some(old) => {
                        let piece = next()?.ok_or_else(|| invalid("A unit's summary piece came back otherwise"))?;
                        Some(self.piece(&old, &piece)?)
                    }
                    None => None,
                };
                if slots.next().is_some() {
                    return Err(invalid("A unit's frames came back long"));
                }
                self.run().tables.push(Entry { unit: entry, summary });
            }
            Item::Head { run, mut head } => {
                let mut given = slots.map(|slot| slot.ok_or_else(|| invalid("A block's head came back short")));
                let clock = given.next().ok_or_else(|| invalid("A block's clock came back short"))??;
                head.clock = self.frame(run, &head.clock, &clock)?;
                let strings = given.collect::<io::Result<Vec<_>>>()?;
                if strings.len() != head.strings.len() {
                    return Err(invalid("A block's strings came back otherwise"));
                }
                for (chunk, stored) in head.strings.iter_mut().zip(strings) {
                    chunk.frame = self.frame(run, &chunk.frame, &stored)?;
                }
                self.run().head = Some(head);
            }
            Item::Block { old, page } => {
                if slots.next().is_some() {
                    return Err(invalid("A block's end came back with frames"));
                }
                let held = self.run();
                if Block::chain_after(&held.read_chain, &old.body()?) != old.head.chain {
                    return Err(invalid(format!("Run {} breaks its chain at block {}", old.run, old.block)));
                }
                held.read_chain = old.head.chain;
                let tables = std::mem::take(&mut held.tables);
                if tables.len() != TABLE_COUNT {
                    return Err(invalid("A block came back without every table"));
                }
                let head = held.head.take().ok_or_else(|| invalid("A block came back without its head"))?;
                let mut block = Block { run: old.run, block: old.block, head, tables };
                block.head.chain = Block::chain_after(&held.chain, &block.body()?);
                held.chain = block.head.chain;
                match page {
                    Some(page) => {
                        let extent = self.page(Kind::BlockPage, tags(&page), &block.write()?)?;
                        self.run().open.push(extent);
                    }
                    None => held.blocks.push(block),
                }
            }
            Item::Span { pages, chain, pieces } => {
                let written = pieces_given(slots)?;
                if chain != self.run().read_chain {
                    return Err(pages.directory.damaged("names another chain than its blocks'"));
                }
                let mut written = written.into_iter();
                let mut summaries = Vec::with_capacity(pieces.len());
                for held in pieces {
                    let mut table = Vec::with_capacity(held.len());
                    for (level, old) in held {
                        let piece = written.next().ok_or_else(|| invalid("A span's pieces came back short"))?;
                        table.push((level, self.piece(&old, &piece)?));
                    }
                    summaries.push(table);
                }
                let blocks = std::mem::take(&mut self.run().blocks);
                if blocks.len() != SPAN_BLOCKS as usize || summaries.len() != TABLE_COUNT {
                    return Err(pages.directory.damaged("closes a span of other blocks than its run's"));
                }
                let mut raw = Vec::new();
                let mut rows = Vec::with_capacity(TABLE_COUNT);
                for (table, summaries) in summaries.into_iter().enumerate() {
                    let row = SpanRow { blocks: blocks.iter().map(|block| block.tables[table].clone()).collect(), summaries };
                    let start = raw.len();
                    row.write(&mut raw)?;
                    let bytes = &raw[start..];
                    rows.push(RowPlace {
                        table: table as u32,
                        offset: start as u64,
                        length: u32::try_from(bytes.len()).map_err(|_| invalid("A span row passes 2^32 bytes"))?,
                        digest: envelope::digest(bytes),
                    });
                }
                let index = self.writer.put(Kind::SpanIndex, tags(&pages.index), raw.len() as u64, &raw)?;
                let directory = SpanDirectory {
                    run: blocks[0].run,
                    first_block: blocks[0].block,
                    blocks: blocks.into_iter().map(|block| block.head).collect(),
                    chain: self.run().chain,
                    rows,
                }
                .write();
                let directory = self.page(Kind::SpanDirectory, tags(&pages.directory), &directory)?;
                self.run().spans.push(SpanPages { directory, index });
            }
            Item::Run { index, extent, directory } => {
                let written = pieces_given(slots)?;
                let held = self.run.take().unwrap_or_default();
                if directory.chain != held.read_chain || !held.blocks.is_empty() || !held.tables.is_empty() || written.len() != directory.tail.len() {
                    return Err(extent.damaged("ends a run of other blocks than it names"));
                }
                let mut tail = Vec::with_capacity(written.len());
                for ((table, old), piece) in directory.tail.iter().zip(written) {
                    tail.push((*table, self.piece(old, &piece)?));
                }
                let bytes = RunDirectory { chain: held.chain, spans: held.spans, open: held.open, tail, ..directory }.write(index)?;
                let written = self.page(Kind::RunDirectory, tags(&extent), &bytes)?;
                self.runs.push(written);
            }
        }
        Ok(())
    }
}

fn pieces_given(slots: impl Iterator<Item = Option<Vec<u8>>>) -> io::Result<Vec<Vec<u8>>> {
    slots.map(|slot| slot.ok_or_else(|| invalid("A summary piece came back short"))).collect()
}

struct Pending {
    item: Item,
    slots: Vec<Option<Vec<u8>>>,
    missing: usize,
    read: u64,
}

fn rewrite_world_log<W: Write + Seek>(
    source: &Source<'_>,
    root: &Root,
    writer: &mut ContainerWriter<W>,
    level: i32,
    below_normal: bool,
    progress: &mut dyn FnMut(&Progress) -> bool,
    pace: Pace,
) -> Result<(), Kept> {
    let Some(section) = root.open(SectionKey::WORLD_LOG, schema::WORLD_LOG)? else { return Ok(()) };
    if !section.frozen.is_empty() {
        return Err(invalid("A columnar file's world log names its runs in its root").into());
    }
    let mut runs = Vec::with_capacity(section.open.len());
    for (index, extent) in (0..).zip(&section.open) {
        runs.push((*extent, RunDirectory::read(&source.raw(extent)?, extent, index)?));
    }
    let mut counted = Progress {
        total: runs.iter().map(|(_, directory)| u64::from(directory.blocks()) * TABLE_COUNT as u64).sum(),
        ..Progress::default()
    };
    let mut walk = Walk { source, runs, run: 0, block: 0, span: None, seen: vec![HashSet::new(); TABLE_COUNT] };
    let mut out = Out { writer, pages: frame_compressor(level)?, run: None, runs: Vec::new(), wholes: vec![HashMap::new(); TABLE_COUNT] };
    let queue = Queue {
        state: Mutex::default(),
        work: Condvar::new(),
        given: Condvar::new(),
        #[cfg(test)]
        workers: pace.workers,
        #[cfg(test)]
        hold: pace.hold,
        #[cfg(test)]
        parts: pace.parts,
    };
    std::thread::scope(|scope| -> Result<(), Kept> {
        let _closing = Closing(&queue);
        for _ in 0..pace.workers.max(1) {
            let queue = &queue;
            std::thread::Builder::new().name("recompress".into()).spawn_scoped(scope, move || worker(source, queue, level, below_normal))?;
        }
        let mut pending = VecDeque::<Pending>::new();
        let mut applied = 0_u64;
        let (mut raw, mut held) = (0_u64, 0_u64);
        let mut queued = VecDeque::new();
        let mut walking = true;
        loop {
            while walking && (pending.is_empty() || raw + held < pace.window) {
                if queued.is_empty() {
                    queued.extend(walk.next()?);
                    if queued.is_empty() {
                        walking = false;
                        break;
                    }
                }
                let item = queued.pop_front().expect("queued");
                let plan = Plan::of(applied + pending.len() as u64, &item)?;
                raw += plan.weight;
                pending.push_back(Pending { item, slots: vec![None; plan.slots as usize], missing: plan.missing, read: 0 });
                queue.push(plan.jobs);
            }
            let mut wrote = false;
            while pending.front().is_some_and(|front| front.missing == 0) {
                let Pending { item, slots, read, .. } = pending.pop_front().expect("ready");
                held -= slots.iter().flatten().map(|given| given.len() as u64).sum::<u64>();
                let unit = matches!(item, Item::Unit { .. });
                out.apply(item, slots)?;
                applied += 1;
                #[cfg(test)]
                queue.applied(applied);
                wrote = true;
                counted.read += read;
                if unit {
                    counted.units += 1;
                    counted.written = out.writer.position()?;
                    if !progress(&counted) {
                        return Err(Kept::Stopped);
                    }
                }
            }
            if pending.is_empty() && !walking {
                break;
            }
            if wrote {
                continue;
            }
            for message in queue.take() {
                let (seq, slot, weight, read, given) = match message {
                    Message::Done { seq, slot, weight, read, out, .. } => (seq, slot, weight, read, out),
                    Message::Failed(error) => return Err(error.into()),
                };
                let item = seq.checked_sub(applied).and_then(|at| pending.get_mut(at as usize)).ok_or_else(|| invalid("A worker gave back work of no item in flight"))?;
                raw -= weight;
                item.read += read;
                if let Some(slot) = slot {
                    let place = item.slots.get_mut(slot as usize).filter(|place| place.is_none()).ok_or_else(|| invalid("A worker gave back work of no slot in flight"))?;
                    held += given.len() as u64;
                    *place = Some(given);
                }
                item.missing = item.missing.checked_sub(1).ok_or_else(|| invalid("A worker gave back more work than its item handed out"))?;
            }
        }
        Ok(())
    })?;
    let runs = std::mem::take(&mut out.runs);
    out.writer.set_open(SectionKey::WORLD_LOG, &runs)?;
    Ok(())
}

fn rewrite_sections<W: Write + Seek>(source: &Source<'_>, root: &Root, writer: &mut ContainerWriter<W>, level: i32) -> Result<(), Kept> {
    for section in root.sections.iter().filter(|section| section.key != SectionKey::WORLD_LOG) {
        let extents = source.section(section)?;
        let frozen = section.frozen.len();
        let mut open = Vec::new();
        for (at, extent) in (0_u64..).zip(&extents) {
            let written = match extent.kind {
                Kind::Facts | Kind::Stored => writer.put(extent.kind, tags(extent), extent.raw, &source.stored(extent)?)?,
                Kind::Controls | Kind::Data => writer.put_zstd(extent.kind, tags(extent), &source.raw(extent)?, level)?,
                other => return Err(invalid(format!("{} holds a {other:?} extent, which a recompress does not know", section.key)).into()),
            };
            if at < frozen {
                writer.append(section.key, written)?;
            } else {
                open.push(written);
            }
        }
        if !open.is_empty() {
            writer.set_open(section.key, &open)?;
        }
    }
    Ok(())
}
