//! Prepared ranges in append-only pack files, listed by an atomically
//! replaced, flat manifest.
#![cfg(windows)]

use replay::{publication::posix_delete, publication::posix_replace, publication::sync_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{
        ffi::OsStrExt,
        fs::{FileExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{ERROR_SHARING_VIOLATION, GENERIC_WRITE},
    Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

const MANIFEST: &str = "manifest";

/// The folder holding every local store.
pub fn local_root() -> io::Result<PathBuf> {
    let root =
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| invalid("LOCALAPPDATA is unavailable"))?;
    Ok(PathBuf::from(root).join("HOI4Replay/prepared"))
}

/// The free bytes this user may write, and the capacity, of the volume
/// holding the local stores. Reads the volume of the nearest folder that
/// exists, so it answers before the first store is written.
pub fn local_space() -> io::Result<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut folder = local_root()?;
    while !folder.is_dir() {
        if !folder.pop() {
            return Err(invalid("No folder of the local stores exists"));
        }
    }
    let wide: Vec<u16> = folder.as_os_str().encode_wide().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: `wide` is a NUL-terminated path; the outputs are valid u64s.
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((free, total))
}
const LOCK: &str = "writer.lock";

#[derive(Debug)]
struct WriterBusy;

impl std::fmt::Display for WriterBusy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("writer busy")
    }
}

impl std::error::Error for WriterBusy {}

pub fn is_writer_busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        && error.get_ref().is_some_and(|inner| inner.is::<WriterBusy>())
}
/// The manifest is a small root naming each section's runs of entries in
/// append-only entry logs, with the section names in a names file. Manifests
/// of an older layout (one flat file of every entry, or one file per range)
/// read as absent.
const VERSION: u32 = 4;
/// An entry: pack id, offset and length, little-endian.
const ENTRY_BYTES: u64 = 24;
/// A pack or log table record: id, listed (live) bytes and length.
const PACK_BYTES: u64 = 24;
/// A root's section record: range count and run count.
const SECTION_BYTES: u64 = 16;
/// A run: first range, range count, log id and offset of its first entry.
const RUN_BYTES: u64 = 32;
/// Runs a section keeps before its entries are rewritten as one run.
const MAX_RUNS: usize = 32;
/// The log id runs of a commit's own log carry until the log is created.
const NEW_LOG: u64 = 0;
/// Bytes a pack writer buffers before writing them to its file.
const PACK_BUFFER: u64 = 1 << 20;
const SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
/// How long a key's content stamps, once found current, stand for its commits.
/// A content stack holds thousands of files, so checking them all at every
/// commit costs a commit about 0.2 s of every core. Tests set their own.
#[cfg_attr(test, allow(dead_code))]
const CONTENT_RECHECK: Duration = Duration::from_secs(60);

/// Counts file-system work for [`counters`] in test builds.
macro_rules! count {
    ($field:ident) => {
        #[cfg(any(test, feature = "test-support"))]
        counters::add(|counts| counts.$field += 1);
    };
}

/// Ordered inputs from the recording's resolved content stack, including archives.
/// Only paths, sizes and modification times participate in reuse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceKey {
    recording: Stamp,
    content: Arc<Vec<Stamp>>,
    #[serde(skip)]
    binding: Arc<Mutex<Option<(PathBuf, Uuid, bool)>>>,
    /// A commit rejected these inputs, not merely a different saved manifest.
    #[serde(skip)]
    rejected: Arc<AtomicBool>,
    /// When the content stamps were last read and found current, shared by
    /// the key's clones.
    #[serde(skip)]
    checked: Arc<Mutex<Option<Instant>>>,
}

impl PartialEq for SourceKey {
    fn eq(&self, other: &Self) -> bool {
        self.recording == other.recording
            && (Arc::ptr_eq(&self.content, &other.content) || self.content == other.content)
    }
}
impl Eq for SourceKey {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

impl Stamp {
    fn capture(path: &Path) -> io::Result<Self> {
        Self::read(std::path::absolute(path)?)
    }

    fn read(path: PathBuf) -> io::Result<Self> {
        #[cfg(test)]
        tests::INPUT_CHECKS.set(tests::INPUT_CHECKS.get() + 1);
        count!(stats);
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(invalid("source is not a file"));
        }
        Ok(Self {
            path,
            size: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

fn stamps(
    paths: &[PathBuf],
    read: impl Fn(&PathBuf) -> io::Result<Stamp> + Sync,
) -> io::Result<Vec<Stamp>> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(paths.len());
    if workers <= 1 {
        return paths.iter().map(read).collect();
    }
    std::thread::scope(|scope| {
        let read = &read;
        let tasks: Vec<_> = paths
            .chunks(paths.len().div_ceil(workers))
            .map(|part| {
                let priority = job_priority::JobPriority::current();
                scope.spawn(move || {
                    priority.enter(job_priority::Level::Normal);
                    let result = part.iter().map(read).collect::<io::Result<Vec<_>>>();
                    #[cfg(test)]
                    let checked = tests::INPUT_CHECKS.get();
                    #[cfg(not(test))]
                    let checked = ();
                    (result, checked)
                })
            })
            .collect();
        let mut result = Vec::with_capacity(paths.len());
        let mut error = None;
        for task in tasks {
            let (part, _checked) = task
                .join()
                .map_err(|_| io::Error::other("Input metadata worker panicked"))?;
            #[cfg(test)]
            tests::INPUT_CHECKS.set(tests::INPUT_CHECKS.get() + _checked);
            match part {
                Ok(part) => result.extend(part),
                Err(why) => {
                    error.get_or_insert(why);
                }
            }
        }
        error.map_or(Ok(result), Err)
    })
}

impl SourceKey {
    /// Refresh only the saved manifest identity, never a rejected binding.
    pub fn refresh_saved_binding(&self, saved: &Self) {
        if self != saved {
            return;
        }
        let saved = saved.binding.lock().unwrap().clone();
        let mut binding = self.binding.lock().unwrap();
        if !self.rejected.load(Ordering::Acquire) {
            *binding = saved;
        }
    }

    pub fn recording_size(&self) -> u64 {
        self.recording.size
    }

    pub fn capture(recording: &Path, content: &[PathBuf]) -> io::Result<Self> {
        Ok(Self {
            recording: Stamp::capture(recording)?,
            content: Arc::new(stamps(content, |path| Stamp::capture(path))?),
            binding: Arc::default(),
            rejected: Arc::default(),
            checked: Arc::new(Mutex::new(Some(Instant::now()))),
        })
    }

    /// Recapture the same ordered physical input paths without reading payloads.
    pub fn recapture(&self) -> io::Result<Self> {
        let paths: Vec<_> = self
            .content
            .iter()
            .map(|stamp| stamp.path.clone())
            .collect();
        let current = Self {
            recording: Stamp::read(self.recording.path.clone())?,
            content: Arc::new(stamps(&paths, |path| Stamp::read(path.clone()))?),
            binding: Arc::default(),
            rejected: Arc::default(),
            checked: Arc::new(Mutex::new(Some(Instant::now()))),
        };
        Ok(if current == *self {
            self.clone()
        } else {
            current
        })
    }

    /// Bind a freshly resolved file list, retaining the saved binding only when
    /// its ordered paths and every current size/mtime still match.
    pub fn recapture_for(&self, recording: &Path, content: &[PathBuf]) -> io::Result<Self> {
        if self.recording.path == recording
            && self.content.len() == content.len()
            && self
                .content
                .iter()
                .zip(content)
                .all(|(stamp, path)| &stamp.path == path)
        {
            self.recapture()
        } else {
            Self::capture(recording, content)
        }
    }

    /// The same recording under another name: a rename keeps its size, time and content.
    pub fn renames(&self, previous: &SourceKey) -> bool {
        self.recording.path != previous.recording.path
            && self.recording.size == previous.recording.size
            && self.recording.modified == previous.recording.modified
            && self.content == previous.content
    }

    /// Refresh recording-only metadata at its current path, without validating the content binding.
    pub fn refresh_recording(&self, recording: &Path) -> io::Result<Self> {
        let recording = Stamp::capture(recording)?;
        if recording == self.recording {
            return Ok(self.clone());
        }
        Ok(Self {
            recording,
            content: Arc::clone(&self.content),
            binding: Arc::default(),
            rejected: Arc::default(),
            checked: Arc::clone(&self.checked),
        })
    }

    fn recording_current(&self) -> io::Result<bool> {
        Ok(Stamp::read(self.recording.path.clone())? == self.recording)
    }

    fn current(&self) -> io::Result<bool> {
        #[cfg(test)]
        tests::VALIDATIONS.set(tests::VALIDATIONS.get() + 1);
        Ok(self.recapture()? == *self)
    }

    /// Whether a commit may publish under this key: the recording's stamp is
    /// read every time, the content stack's only once [`CONTENT_RECHECK`] has
    /// passed since its stamps were last found current. Content edited within
    /// that interval is refused when the store is next opened, since the
    /// published stamps no longer match it.
    fn current_for_commit(&self) -> io::Result<bool> {
        #[cfg(test)]
        let interval = tests::CONTENT_RECHECK.get();
        #[cfg(not(test))]
        let interval = CONTENT_RECHECK;
        let mut checked = self.checked.lock().unwrap();
        if checked.is_some_and(|at| at.elapsed() < interval) {
            return self.recording_current();
        }
        let current = self.current()?;
        *checked = current.then(Instant::now);
        Ok(current)
    }
}

/// Range readers are independent of this object and keep their bytes until closed.
pub struct Store {
    directory: PathBuf,
}

/// Pack handles one lookup has opened, shared by the ranges it returns.
type Opened = HashMap<u64, Arc<File>>;

impl Store {
    /// The store's folder, for tests that damage its files.
    #[cfg(any(test, feature = "test-support"))]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn local(campaign: Uuid) -> io::Result<Self> {
        Ok(Self {
            directory: local_root()?.join(campaign.to_string()),
        })
    }

    /// The manifest's on-disk identity (its length, last write and creation
    /// times), or none before the first commit. Every commit replaces the
    /// manifest and Clear removes it, so a changed identity means some writer,
    /// in this process or another, changed the store. Reads file metadata only.
    pub fn manifest_identity(&self) -> io::Result<Option<(u64, u64, u64)>> {
        use std::os::windows::fs::MetadataExt;
        match fs::metadata(self.directory.join(MANIFEST)) {
            Ok(metadata) => Ok(Some((
                metadata.len(),
                metadata.last_write_time(),
                metadata.creation_time(),
            ))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reads just the manifest header, never enumerating or opening pack files.
    pub fn is_reusable(&self, key: &SourceKey) -> io::Result<bool> {
        if self.scoped(key).is_some() {
            return Ok(true);
        }
        match self.catalog()? {
            Some(catalog) => self.matches(&catalog, key),
            None => Ok(false),
        }
    }

    /// The published input metadata, also available after an input has changed.
    /// Producers use it to decide whether their verified prefix can be continued.
    pub fn saved_key(&self) -> io::Result<Option<SourceKey>> {
        self.catalog()?
            .map(|catalog| self.source_key(&catalog))
            .transpose()
    }

    /// Reads only the manifest header, independent of the section's range count.
    pub fn range_count(&self, key: &SourceKey, section: &str) -> io::Result<Option<u64>> {
        if let Some(session) = self.scoped(key) {
            return Ok(session.range_count(section));
        }
        let Some(catalog) = self.catalog()? else {
            return Ok(None);
        };
        if !self.matches(&catalog, key)? {
            return Ok(None);
        }
        Ok(catalog
            .header
            .sections
            .into_iter()
            .find(|entry| entry.name == section)
            .map(|entry| entry.count))
    }

    /// A range is its zero-based ordinal within a section, owned by the producer.
    /// Reads the header and one fixed-width manifest entry, then opens its pack.
    pub fn read(&self, key: &SourceKey, section: &str, range: u64) -> io::Result<Option<Range>> {
        if let Some(session) = self.scoped(key) {
            return session.read(section, range);
        }
        if !key.recording_current()? {
            return Ok(None);
        }
        self.read_named(key, section, range)
    }

    /// Opens requested ranges in order with one manifest read.
    #[cfg(test)]
    pub fn read_many(
        &self,
        key: &SourceKey,
        ranges: &[(&str, u64)],
    ) -> io::Result<Vec<Option<Range>>> {
        Ok(self
            .read_many_counted(key, ranges)?
            .into_iter()
            .map(|(_, range)| range)
            .collect())
    }

    /// The section counts and readers come from the same manifest lookup.
    /// Ranges in one pack share its handle.
    pub fn read_many_counted(
        &self,
        key: &SourceKey,
        ranges: &[(&str, u64)],
    ) -> io::Result<Vec<(u64, Option<Range>)>> {
        let empty = || ranges.iter().map(|_| (0, None)).collect();
        if ranges.is_empty() {
            return Ok(empty());
        }
        if let Some(session) = self.scoped(key) {
            return ranges
                .iter()
                .map(|&(section, range)| session.read_counted(section, range))
                .collect();
        }
        let Some(catalog) = self.catalog()? else {
            return Ok(empty());
        };
        if !self.matches(&catalog, key)? {
            return Ok(empty());
        }
        let mut opened = Opened::new();
        ranges
            .iter()
            .map(|&(section, range)| {
                match self.read_catalog(&catalog, section, range, &mut opened) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.read_named_counted(key, section, range)
                    }
                    result => result,
                }
            })
            .collect()
    }

    /// Reads an accepted prepared prefix after recording growth. The producer
    /// first applies its recording-index reuse policy, just as for `append`.
    pub fn read_prefix(
        &self,
        previous: &SourceKey,
        current: &SourceKey,
        section: &str,
        range: u64,
    ) -> io::Result<Option<Range>> {
        if !prefix_inputs(previous, current)? {
            return Ok(None);
        }
        self.read_named(previous, section, range)
    }

    pub fn prefix_range_count(
        &self,
        previous: &SourceKey,
        current: &SourceKey,
        section: &str,
    ) -> io::Result<Option<u64>> {
        if !prefix_inputs(previous, current)? {
            return Ok(None);
        }
        let Some(catalog) = self.catalog()? else {
            return Ok(None);
        };
        if !self.matches_bound(&catalog, previous)? {
            return Ok(None);
        }
        Ok(catalog
            .header
            .sections
            .into_iter()
            .find(|entry| entry.name == section)
            .map(|entry| entry.count))
    }

    fn read_named(&self, key: &SourceKey, section: &str, range: u64) -> io::Result<Option<Range>> {
        Ok(self.read_named_counted(key, section, range)?.1)
    }

    fn read_named_counted(
        &self,
        key: &SourceKey,
        section: &str,
        range: u64,
    ) -> io::Result<(u64, Option<Range>)> {
        // Cleanup/Clear may remove a pack between manifest lookup and open.
        // Retry against the current manifest; returned readers need no coordination.
        for _ in 0..2 {
            let Some(catalog) = self.catalog()? else {
                return Ok((0, None));
            };
            if !self.matches_bound(&catalog, key)? {
                return Ok((0, None));
            }
            match self.read_catalog(&catalog, section, range, &mut Opened::new()) {
                Ok(range) => return Ok(range),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        Ok((0, None))
    }

    fn source_key(&self, catalog: &Catalog) -> io::Result<SourceKey> {
        #[cfg(test)]
        tests::KEY_READS.set(tests::KEY_READS.get() + 1);
        let mut bytes = Vec::new();
        read_file(&self.directory.join(source_name(catalog.header.source)))?
            .read_to_end(&mut bytes)?;
        let key: SourceKey = serde_json::from_slice(&bytes)?;
        *key.binding.lock().unwrap() = Some((self.directory.clone(), catalog.header.source, true));
        Ok(key)
    }

    fn matches(&self, catalog: &Catalog, key: &SourceKey) -> io::Result<bool> {
        Ok(key.recording_current()? && self.matches_bound(catalog, key)?)
    }

    fn matches_bound(&self, catalog: &Catalog, key: &SourceKey) -> io::Result<bool> {
        let mut binding = key.binding.lock().unwrap();
        if let Some((path, id, accepted)) = binding.as_ref()
            && path == &self.directory
        {
            return Ok(*id == catalog.header.source && *accepted);
        }
        let saved = match self.source_key(catalog) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            result => result?,
        };
        let accepted = saved == *key;
        *binding = Some((self.directory.clone(), catalog.header.source, accepted));
        Ok(accepted)
    }

    fn read_catalog(
        &self,
        catalog: &Catalog,
        section: &str,
        range: u64,
        opened: &mut Opened,
    ) -> io::Result<(u64, Option<Range>)> {
        lookup(&self.directory, catalog, section, range, opened)
    }

    /// A verified view of the published manifest for `key`: one recording
    /// metadata check and one manifest read, or None when the key is not the
    /// published one. Its reads repeat neither.
    pub fn session(&self, key: &SourceKey) -> io::Result<Option<Session>> {
        if let Some(session) = self.scoped(key) {
            return Ok(Some(session));
        }
        let Some(catalog) = self.catalog()? else {
            return Ok(None);
        };
        if !self.matches(&catalog, key)? {
            return Ok(None);
        }
        Ok(Some(Session(Arc::new(SessionState {
            directory: self.directory.clone(),
            key: key.clone(),
            catalog: Mutex::new(Arc::new(catalog)),
            opened: Mutex::default(),
        }))))
    }

    /// Until the guard drops, this thread's reads of `key` through any store of
    /// this directory use one [`Session`]: a query verifies its key and reads
    /// the manifest once, and ranges share pack handles. Without a published
    /// manifest for `key`, reads stay unscoped.
    pub fn scope(&self, key: &SourceKey) -> io::Result<Scope> {
        let session = self.session(key)?;
        let installed = session.is_some();
        if let Some(session) = session {
            SCOPES.with(|scopes| scopes.borrow_mut().push(session));
        }
        Ok(Scope {
            installed,
            _thread: std::marker::PhantomData,
        })
    }

    fn scoped(&self, key: &SourceKey) -> Option<Session> {
        SCOPES.with(|scopes| {
            scopes
                .borrow()
                .iter()
                .rev()
                .find(|session| session.0.directory == self.directory && session.0.key == *key)
                .cloned()
        })
    }

    /// Starts a full replacement. The current manifest remains visible until commit.
    pub fn replace(&self, key: SourceKey) -> io::Result<Writer> {
        let lock = self.lock()?;
        let previous = self.catalog()?;
        self.cleanup(previous.as_ref())?;
        let source = match previous.as_ref() {
            Some(catalog) if self.matches_bound(catalog, &key)? => Some(catalog.header.source),
            _ => None,
        };
        // A replacement lists nothing from the old manifest, so none of its
        // packs stays listed.
        Ok(Writer::new(
            self.directory.clone(),
            lock,
            key,
            source,
            None,
            Vec::new(),
        ))
    }

    /// The producer accepts the old prefix through its recording-index policy.
    /// The store checks growth and content metadata, not byte identity. Ordinary
    /// reopen never accepts an unequal key; this operation publishes a new key last.
    pub fn append(&self, previous: &SourceKey, key: SourceKey) -> io::Result<Writer> {
        // The store is its recording's identity, so a renamed recording appends, or rebinds unchanged.
        if (key.recording.size <= previous.recording.size && !key.renames(previous))
            || key.content != previous.content
        {
            return Err(invalid(
                "append requires recording growth and unchanged content",
            ));
        }
        self.continue_from(previous, key)
    }

    /// Adds accepted preparation ranges for the same unchanged recording.
    /// A cancelled or interrupted producer resumes from its published accumulator.
    pub fn extend(&self, key: &SourceKey) -> io::Result<Writer> {
        if !key.recording_current()? {
            return Err(invalid("sources changed before preparation resumed"));
        }
        self.continue_from(key, key.clone())
    }

    fn continue_from(&self, previous: &SourceKey, key: SourceKey) -> io::Result<Writer> {
        self.continue_locked(previous, key, self.lock()?)
    }

    fn continue_locked(
        &self,
        previous: &SourceKey,
        key: SourceKey,
        lock: File,
    ) -> io::Result<Writer> {
        let base = self
            .catalog()?
            .ok_or_else(|| invalid("no prepared prefix"))?;
        if !self.matches_bound(&base, previous)? {
            return Err(invalid("prepared prefix key changed"));
        }
        self.cleanup(Some(&base))?;
        let sections = base
            .header
            .sections
            .iter()
            .map(|s| PendingSection {
                name: Arc::from(s.name.as_str()),
                old_count: s.count,
                added: Vec::new(),
                replaced: BTreeMap::new(),
            })
            .collect();
        let source = (previous == &key).then_some(base.header.source);
        Ok(Writer::new(
            self.directory.clone(),
            lock,
            key,
            source,
            Some(Arc::new(base)),
            sections,
        ))
    }

    /// Removes saved names while held range readers keep their bytes. The empty
    /// directory and zero-byte lock file remain so writers cannot split ownership.
    pub fn clear(&self) -> io::Result<()> {
        let _lock = self.lock()?;
        // Removing the manifest first prevents new readers seeing deleted packs.
        match posix_delete(&self.directory.join(MANIFEST)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
        self.cleanup(None)
    }

    fn catalog(&self) -> io::Result<Option<Catalog>> {
        #[cfg(test)]
        tests::MANIFEST_READS.set(tests::MANIFEST_READS.get() + 1);
        count!(manifest_reads);
        Catalog::open(&self.directory.join(MANIFEST))
    }

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.directory)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(self.directory.join(LOCK))
            .map_err(|error| {
                if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION as i32) {
                    io::Error::new(io::ErrorKind::WouldBlock, WriterBusy)
                } else {
                    error
                }
            })
    }

    /// Keeps the lock, the manifest, its source stamps, its names file and the
    /// packs and entry logs its tables list, plus live stages' files; removes
    /// every other name in one directory pass.
    fn cleanup(&self, catalog: Option<&Catalog>) -> io::Result<()> {
        // Creation and registration of a stage's packs use this same lock, so
        // the directory pass cannot remove a pack before its pin is visible.
        let mut registry = STAGING.lock().unwrap_or_else(|error| error.into_inner());
        let stages = active_stages(&mut registry, &self.directory);
        let (mut listed, mut logs): (HashSet<u64>, HashSet<u64>) = match catalog {
            Some(catalog) => (
                catalog.packs.iter().map(|pack| pack.id).collect(),
                catalog.logs.iter().map(|log| log.id).collect(),
            ),
            None => Default::default(),
        };
        for stage in stages {
            listed.extend(
                stage
                    .packs
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .iter()
                    .copied(),
            );
            logs.extend(&stage.logs);
        }
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let text = name.to_str();
            let keep = name == LOCK
                || catalog.is_some_and(|catalog| {
                    name == MANIFEST
                        || name == source_name(catalog.header.source).as_str()
                        || (catalog.names.0 != 0 && name == names_name(catalog.names.0).as_str())
                })
                || text
                    .and_then(parse_pack)
                    .is_some_and(|id| listed.contains(&id))
                || text
                    .and_then(parse_log)
                    .is_some_and(|id| logs.contains(&id));
            if !keep {
                posix_delete(&entry.path())?;
            }
        }
        Ok(())
    }
}

thread_local! {
    /// Sessions installed on this thread by [`Store::scope`], innermost last.
    static SCOPES: std::cell::RefCell<Vec<Session>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Looks up one listed range and opens its pack, sharing handles in `opened`.
fn lookup(
    directory: &Path,
    catalog: &Catalog,
    section: &str,
    range: u64,
    opened: &mut Opened,
) -> io::Result<(u64, Option<Range>)> {
    let Some(index) = catalog
        .header
        .sections
        .iter()
        .position(|s| s.name == section)
    else {
        return Ok((0, None));
    };
    let count = catalog.header.sections[index].count;
    let Some(id) = catalog.entry(index, range)? else {
        return Ok((count, None));
    };
    if id.len == 0 {
        return Ok((count, Some(Range::empty())));
    }
    let file = match opened.get(&id.pack) {
        Some(file) => Arc::clone(file),
        None => {
            let file = Arc::new(open_pack(directory, id.pack)?);
            opened.insert(id.pack, Arc::clone(&file));
            file
        }
    };
    Ok((count, Some(Range::new(file, id))))
}

fn open_pack(directory: &Path, pack: u64) -> io::Result<File> {
    count!(pack_opens);
    read_file(&directory.join(pack_name(pack)))
}

/// One verified view of a published manifest for one source key. Its reads
/// check neither the recording nor the manifest again, and ranges in one pack
/// share a handle. A pack a later writer removed before this session opened it
/// is read through the then-current manifest, as unscoped reads are.
#[derive(Clone)]
pub struct Session(Arc<SessionState>);

struct SessionState {
    directory: PathBuf,
    key: SourceKey,
    catalog: Mutex<Arc<Catalog>>,
    opened: Mutex<Opened>,
}

impl Session {
    pub fn range_count(&self, section: &str) -> Option<u64> {
        self.0
            .catalog
            .lock()
            .unwrap()
            .header
            .sections
            .iter()
            .find(|entry| entry.name == section)
            .map(|entry| entry.count)
    }

    pub fn read(&self, section: &str, range: u64) -> io::Result<Option<Range>> {
        Ok(self.read_counted(section, range)?.1)
    }

    /// The range and its section's published count.
    pub fn read_counted(&self, section: &str, range: u64) -> io::Result<(u64, Option<Range>)> {
        let state = &self.0;
        for retry in [false, true] {
            let catalog = Arc::clone(&state.catalog.lock().unwrap());
            let mut opened = state.opened.lock().unwrap();
            match lookup(&state.directory, &catalog, section, range, &mut opened) {
                Err(error) if error.kind() == io::ErrorKind::NotFound && !retry => {
                    drop(opened);
                    let store = Store {
                        directory: state.directory.clone(),
                    };
                    let Some(current) = store.catalog()? else {
                        return Ok((0, None));
                    };
                    if !store.matches_bound(&current, &state.key)? {
                        return Ok((0, None));
                    }
                    *state.catalog.lock().unwrap() = Arc::new(current);
                }
                result => return result,
            }
        }
        Ok((0, None))
    }
}

/// Keeps a [`Store::scope`] session installed on this thread until dropped.
#[must_use]
pub struct Scope {
    installed: bool,
    _thread: std::marker::PhantomData<*const ()>,
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.installed {
            SCOPES.with(|scopes| scopes.borrow_mut().pop());
        }
    }
}

/// Per-thread counts of the file-system work store calls did, for tests that
/// bound a query's reads.
#[cfg(any(test, feature = "test-support"))]
pub mod counters {
    use std::cell::Cell;

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Counters {
        /// Input metadata reads (the recording check of an unscoped read).
        pub stats: u64,
        pub manifest_reads: u64,
        pub pack_opens: u64,
        /// Entry logs opened to find a range.
        pub log_opens: u64,
    }

    impl std::ops::Sub for Counters {
        type Output = Self;
        fn sub(self, earlier: Self) -> Self {
            Self {
                stats: self.stats - earlier.stats,
                manifest_reads: self.manifest_reads - earlier.manifest_reads,
                pack_opens: self.pack_opens - earlier.pack_opens,
                log_opens: self.log_opens - earlier.log_opens,
            }
        }
    }

    thread_local! {
        static COUNTS: Cell<Counters> = const {
            Cell::new(Counters {
                stats: 0,
                manifest_reads: 0,
                pack_opens: 0,
                log_opens: 0,
            })
        };
    }

    pub fn now() -> Counters {
        COUNTS.get()
    }

    pub(crate) fn add(change: impl FnOnce(&mut Counters)) {
        let mut counts = COUNTS.get();
        change(&mut counts);
        COUNTS.set(counts);
    }
}

/// A store is its recording's identity, so a renamed recording continues its prefix.
fn prefix_inputs(previous: &SourceKey, current: &SourceKey) -> io::Result<bool> {
    Ok(current.recording.size >= previous.recording.size
        && current.content == previous.content
        && current.recording_current()?)
}

/// Where one range's bytes live: a pack, an offset in it and a length. Equal
/// identities name the same immutable bytes. An empty range lives in no pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RangeId {
    pack: u64,
    offset: u64,
    len: u64,
}

impl RangeId {
    /// Every empty range. No pack has id 0, so no pack can be removed under it.
    const EMPTY: Self = Self {
        pack: 0,
        offset: 0,
        len: 0,
    };

    /// Serializable identity for transient cache keys, without a pack handle.
    pub fn cache_key(&self) -> [u64; 3] {
        [self.pack, self.offset, self.len]
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Opens the range in the prepared directory that lists it.
    pub fn open(&self, directory: &Path) -> io::Result<Range> {
        if self.len == 0 {
            return Ok(Range::empty());
        }
        Ok(Range::new(
            Arc::new(open_pack(directory, self.pack)?),
            *self,
        ))
    }

    fn encode(&self) -> [u8; ENTRY_BYTES as usize] {
        let mut bytes = [0; ENTRY_BYTES as usize];
        bytes[..8].copy_from_slice(&self.pack.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.offset.to_le_bytes());
        bytes[16..].copy_from_slice(&self.len.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> io::Result<Self> {
        let [pack, offset, len] = words(bytes);
        offset
            .checked_add(len)
            .ok_or_else(|| invalid("prepared range exceeds its pack"))?;
        Ok(Self { pack, offset, len })
    }
}

/// One range's bytes: an owned pack handle bounded to the range (none for an
/// empty range). It stays valid while held, through replacement, Clear and
/// later commits.
#[derive(Clone, Debug)]
pub struct Range {
    file: Option<Arc<File>>,
    id: RangeId,
    position: u64,
}

impl Range {
    fn new(file: Arc<File>, id: RangeId) -> Self {
        if id.len == 0 {
            return Self::empty();
        }
        Self {
            file: Some(file),
            id,
            position: 0,
        }
    }

    fn empty() -> Self {
        Self {
            file: None,
            id: RangeId::EMPTY,
            position: 0,
        }
    }

    /// The range's length in bytes.
    pub fn len(&self) -> u64 {
        self.id.len
    }

    pub fn is_empty(&self) -> bool {
        self.id.len == 0
    }

    /// Where these bytes live; replacement gives a non-empty range a new identity.
    pub fn id(&self) -> RangeId {
        self.id
    }
}

/// The most [`Range::read_to_end`] sizes its buffer for in one read.
const READ_TO_END_STEP: u64 = 64 << 20;

impl Read for Range {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let remaining = self.id.len.saturating_sub(self.position);
        let want = buffer
            .len()
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        if want == 0 {
            return Ok(0);
        }
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| invalid("prepared range has no pack"))?;
        let at = self.id.offset + self.position;
        loop {
            match file.seek_read(&mut buffer[..want], at) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "prepared range is truncated",
                    ));
                }
                Ok(read) => {
                    self.position += read as u64;
                    return Ok(read);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// The rest of the range, read straight into a buffer sized for it rather
    /// than one grown through a read per doubling. The buffer grows a bounded
    /// step at a time, so a damaged length claims no more memory than one
    /// step beyond what its pack holds; as by default, an error keeps the
    /// bytes read before it.
    fn read_to_end(&mut self, buffer: &mut Vec<u8>) -> io::Result<usize> {
        let start = buffer.len();
        loop {
            let remaining = self.id.len.saturating_sub(self.position);
            if remaining == 0 {
                return Ok(buffer.len() - start);
            }
            let step = usize::try_from(remaining.min(READ_TO_END_STEP))
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
            buffer
                .try_reserve_exact(step)
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
            let filled = buffer.len();
            buffer.resize(filled + step, 0);
            match self.read(&mut buffer[filled..]) {
                Ok(read) => buffer.truncate(filled + read),
                Err(error) => {
                    buffer.truncate(filled);
                    return Err(error);
                }
            }
        }
    }
}

impl Seek for Range {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let target = match position {
            SeekFrom::Start(at) => i128::from(at),
            SeekFrom::End(delta) => i128::from(self.id.len) + i128::from(delta),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
        };
        self.position = u64::try_from(target).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before the start of a prepared range",
            )
        })?;
        Ok(self.position)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        Ok(self.position)
    }
}

/// A read-only view of a published manifest, or of a snapshot one wrote
/// ([`Listing::write_snapshot`]), for measurement tools. Manifests of an
/// older layout read as absent.
pub struct Listing(Catalog);

impl Listing {
    /// Opens a manifest, whose names file and entry logs are its siblings,
    /// or a snapshot, which holds its entries itself.
    pub fn open(manifest: &Path) -> io::Result<Option<Self>> {
        Ok(Catalog::open(manifest)?.map(Self))
    }

    /// Section names and range counts, in manifest order.
    pub fn sections(&self) -> impl Iterator<Item = (&str, u64)> {
        self.0
            .header
            .sections
            .iter()
            .map(|section| (section.name.as_str(), section.count))
    }

    /// The listed range at a section's index and ordinal.
    pub fn range(&self, section: usize, range: u64) -> io::Result<Option<RangeId>> {
        self.0.entry(section, range)
    }

    /// Writes a self-contained snapshot of the listing: its sections and
    /// every entry, in manifest order, which [`Listing::open`] reads without
    /// the store's names file or logs.
    pub fn write_snapshot(&self, into: &mut impl Write) -> io::Result<()> {
        let head = serde_json::to_vec(&FlatHead {
            version: VERSION,
            flat: true,
            sections: self.0.header.sections.clone(),
        })?;
        into.write_all(&(head.len() as u64).to_le_bytes())?;
        into.write_all(&head)?;
        for (index, section) in self.0.header.sections.iter().enumerate() {
            match &self.0.entries {
                Entries::Flat { file, offsets } => {
                    let mut bytes = vec![0; to_usize(section.count * ENTRY_BYTES)?];
                    if read_at(file, &mut bytes, offsets[index])? != bytes.len() {
                        return Err(invalid("truncated prepared listing"));
                    }
                    into.write_all(&bytes)?;
                }
                Entries::Logs(_) => {
                    for run in &self.0.runs[index] {
                        into.write_all(&self.0.run_entries(run)?)?;
                    }
                }
            }
        }
        into.flush()
    }
}

/// What a catalog lists, whatever holds its entries.
struct Header {
    source: Uuid,
    sections: Vec<Section>,
}

/// Read first, so an older layout is recognised before its header is parsed.
#[derive(Deserialize)]
struct Layout {
    version: u32,
    #[serde(default)]
    flat: bool,
}

/// A root's header: its source stamps, its names file and how many of that
/// file's bytes it lists, and the length of each of its tables (the pack
/// table fills the rest of the root).
#[derive(Serialize, Deserialize)]
struct Head {
    version: u32,
    source: Uuid,
    names: u64,
    names_bytes: u64,
    sections: u64,
    runs: u64,
    logs: u64,
}

/// A snapshot's header: its sections, whose entries follow it.
#[derive(Serialize, Deserialize)]
struct FlatHead {
    version: u32,
    flat: bool,
    sections: Vec<Section>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Section {
    name: String,
    count: u64,
}

/// A pack or entry log the manifest lists, with its listed bytes and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PackRecord {
    id: u64,
    live: u64,
    size: u64,
}

impl PackRecord {
    fn encode(&self) -> [u8; PACK_BYTES as usize] {
        RangeId {
            pack: self.id,
            offset: self.live,
            len: self.size,
        }
        .encode()
    }

    fn decode(bytes: &[u8]) -> Self {
        let [id, live, size] = words(bytes);
        Self { id, live, size }
    }
}

/// Consecutive ranges of one section whose entries lie consecutively in one
/// entry log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Run {
    first: u64,
    count: u64,
    log: u64,
    offset: u64,
}

impl Run {
    fn end(&self) -> u64 {
        self.first + self.count
    }

    fn bytes(&self) -> u64 {
        self.count * ENTRY_BYTES
    }

    fn encode(&self) -> [u8; RUN_BYTES as usize] {
        let mut bytes = [0; RUN_BYTES as usize];
        for (at, word) in [self.first, self.count, self.log, self.offset]
            .into_iter()
            .enumerate()
        {
            bytes[at * 8..at * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    fn decode(bytes: &[u8]) -> Self {
        let word = |at: usize| u64::from_le_bytes(bytes[at * 8..at * 8 + 8].try_into().unwrap());
        Self {
            first: word(0),
            count: word(1),
            log: word(2),
            offset: word(3),
        }
    }
}

fn words(bytes: &[u8]) -> [u64; 3] {
    [0, 8, 16].map(|at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()))
}

fn to_usize(value: u64) -> io::Result<usize> {
    usize::try_from(value).map_err(|_| invalid("prepared manifest too large"))
}

/// Where a catalog's entries are: in entry logs beside its root, opened when
/// first read and shared by its lookups, or in a snapshot itself.
enum Entries {
    Logs(Mutex<HashMap<u64, Arc<File>>>),
    Flat { file: File, offsets: Vec<u64> },
}

struct Catalog {
    /// Where its names file, entry logs and packs are.
    directory: PathBuf,
    header: Header,
    /// The names file and how many of its bytes are this root's.
    names: (u64, u64),
    /// Each section's runs, in range order, covering its ranges.
    runs: Vec<Vec<Run>>,
    logs: Vec<PackRecord>,
    packs: Vec<PackRecord>,
    entries: Entries,
}

/// A root encoded: its header, section and run records, then its log and
/// pack tables.
fn encode_root(
    head: &Head,
    sections: &[(u64, &[Run])],
    logs: &[PackRecord],
    packs: &[PackRecord],
) -> io::Result<Vec<u8>> {
    let header = serde_json::to_vec(head)?;
    let runs: usize = sections.iter().map(|(_, runs)| runs.len()).sum();
    let mut bytes = Vec::with_capacity(
        8 + header.len()
            + sections.len() * SECTION_BYTES as usize
            + runs * RUN_BYTES as usize
            + (logs.len() + packs.len()) * PACK_BYTES as usize,
    );
    bytes.extend_from_slice(&(header.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&header);
    for (count, runs) in sections {
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&(runs.len() as u64).to_le_bytes());
    }
    for (_, runs) in sections {
        for run in *runs {
            bytes.extend_from_slice(&run.encode());
        }
    }
    for record in logs.iter().chain(packs) {
        bytes.extend_from_slice(&record.encode());
    }
    Ok(bytes)
}

/// Section names as a names file lists them: each a little-endian length and
/// its UTF-8 bytes.
fn encode_names<'a>(names: impl Iterator<Item = &'a str>) -> Vec<u8> {
    let mut bytes = Vec::new();
    for name in names {
        bytes.extend_from_slice(&(name.len() as u32).to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
    }
    bytes
}

fn decode_names(bytes: &[u8], count: u64) -> io::Result<Vec<String>> {
    let mut names = Vec::with_capacity(to_usize(count)?);
    let mut at = 0;
    while at < bytes.len() {
        let length = bytes
            .get(at..at + 4)
            .map(|word| u32::from_le_bytes(word.try_into().unwrap()) as usize)
            .ok_or_else(|| invalid("truncated prepared names"))?;
        let name = bytes
            .get(at + 4..at + 4 + length)
            .ok_or_else(|| invalid("truncated prepared names"))?;
        names.push(
            String::from_utf8(name.to_vec()).map_err(|_| invalid("prepared name is not UTF-8"))?,
        );
        at += 4 + length;
    }
    if names.len() as u64 != count {
        return Err(invalid("prepared names do not match the manifest"));
    }
    Ok(names)
}

impl Catalog {
    fn open(path: &Path) -> io::Result<Option<Self>> {
        // A writer that replaced the manifest and a later writer open may
        // remove the names file between reading the root and opening it:
        // read the current root again.
        let mut attempts = 0;
        loop {
            match Self::open_once(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound && attempts < 3 => {
                    attempts += 1;
                }
                result => return result,
            }
        }
    }

    fn open_once(path: &Path) -> io::Result<Option<Self>> {
        let mut file = match read_file(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let directory = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let size = file.metadata()?.len();
        let mut bytes = [0; 8];
        file.read_exact(&mut bytes)?;
        let header_size = u64::from_le_bytes(bytes);
        let start = 8u64
            .checked_add(header_size)
            .filter(|end| *end <= size)
            .ok_or_else(|| invalid("invalid manifest header length"))?;
        let mut header = Vec::new();
        (&mut file).take(header_size).read_to_end(&mut header)?;
        let layout: Layout = serde_json::from_slice(&header)?;
        if layout.version < VERSION {
            // An older layout is never read: it is a cache miss, producers
            // prepare again, and the next writer open removes its files.
            return Ok(None);
        }
        if layout.version != VERSION {
            return Err(invalid("unsupported prepared manifest version"));
        }
        if layout.flat {
            return Self::open_flat(file, directory, &header, start, size).map(Some);
        }
        let head: Head = serde_json::from_slice(&header)?;
        let mut body = Vec::new();
        file.read_to_end(&mut body)?;
        let sections = to_usize(head.sections)?;
        let runs = to_usize(head.runs)?;
        let logs = to_usize(head.logs)?;
        let fixed = sections
            .checked_mul(SECTION_BYTES as usize)
            .and_then(|bytes| bytes.checked_add(runs.checked_mul(RUN_BYTES as usize)?))
            .and_then(|bytes| bytes.checked_add(logs.checked_mul(PACK_BYTES as usize)?))
            .filter(|bytes| *bytes <= body.len())
            .ok_or_else(|| invalid("manifest length does not match its tables"))?;
        if (body.len() - fixed) % PACK_BYTES as usize != 0 {
            return Err(invalid("manifest length does not match its tables"));
        }
        let (records, rest) = body.split_at(sections * SECTION_BYTES as usize);
        let (run_bytes, rest) = rest.split_at(runs * RUN_BYTES as usize);
        let (log_bytes, pack_bytes) = rest.split_at(logs * PACK_BYTES as usize);
        let logs: Vec<_> = log_bytes
            .chunks_exact(PACK_BYTES as usize)
            .map(PackRecord::decode)
            .collect();
        let listed: HashSet<u64> = logs.iter().map(|log| log.id).collect();
        let packs = pack_bytes
            .chunks_exact(PACK_BYTES as usize)
            .map(PackRecord::decode)
            .collect();
        let mut all = run_bytes.chunks_exact(RUN_BYTES as usize).map(Run::decode);
        let mut counts = Vec::with_capacity(sections);
        let mut section_runs = Vec::with_capacity(sections);
        let mismatch = || invalid("manifest runs do not match their sections");
        for record in records.chunks_exact(SECTION_BYTES as usize) {
            let [count, runs] =
                [0, 8].map(|at| u64::from_le_bytes(record[at..at + 8].try_into().unwrap()));
            let mut covered = 0;
            let runs = to_usize(runs)
                .ok()
                .filter(|runs| *runs <= all.len())
                .ok_or_else(mismatch)?;
            let mut own = Vec::with_capacity(runs);
            for _ in 0..runs {
                let run = all.next().ok_or_else(mismatch)?;
                if run.first != covered
                    || run.count == 0
                    || run.first.checked_add(run.count).is_none()
                    || !listed.contains(&run.log)
                    || run
                        .count
                        .checked_mul(ENTRY_BYTES)
                        .and_then(|bytes| run.offset.checked_add(bytes))
                        .is_none()
                {
                    return Err(mismatch());
                }
                covered = run.end();
                own.push(run);
            }
            if covered != count {
                return Err(mismatch());
            }
            counts.push(count);
            section_runs.push(own);
        }
        if all.next().is_some() {
            return Err(mismatch());
        }
        let names = if head.sections == 0 {
            Vec::new()
        } else {
            let file = read_file(&directory.join(names_name(head.names)))?;
            if head.names_bytes > file.metadata()?.len() {
                return Err(invalid("truncated prepared names"));
            }
            let mut bytes = vec![0; to_usize(head.names_bytes)?];
            if read_at(&file, &mut bytes, 0)? != bytes.len() {
                return Err(invalid("truncated prepared names"));
            }
            decode_names(&bytes, head.sections)?
        };
        let mut seen = HashSet::new();
        let sections = names
            .into_iter()
            .zip(counts)
            .map(|(name, count)| {
                if name.is_empty() || !seen.insert(name.clone()) {
                    return Err(invalid("empty or repeated manifest section"));
                }
                Ok(Section { name, count })
            })
            .collect::<io::Result<_>>()?;
        Ok(Some(Self {
            directory,
            header: Header {
                source: head.source,
                sections,
            },
            names: (head.names, head.names_bytes),
            runs: section_runs,
            logs,
            packs,
            entries: Entries::Logs(Mutex::default()),
        }))
    }

    /// A snapshot: its sections' entries follow its header.
    fn open_flat(
        file: File,
        directory: PathBuf,
        header: &[u8],
        start: u64,
        size: u64,
    ) -> io::Result<Self> {
        let head: FlatHead = serde_json::from_slice(header)?;
        let mut offsets = Vec::with_capacity(head.sections.len());
        let mut offset = start;
        for section in &head.sections {
            offsets.push(offset);
            offset = section
                .count
                .checked_mul(ENTRY_BYTES)
                .and_then(|bytes| offset.checked_add(bytes))
                .ok_or_else(|| invalid("manifest range count overflow"))?;
        }
        if offset != size {
            return Err(invalid("prepared listing length does not match its ranges"));
        }
        Ok(Self {
            directory,
            runs: head.sections.iter().map(|_| Vec::new()).collect(),
            header: Header {
                source: Uuid::nil(),
                sections: head.sections,
            },
            names: (0, 0),
            logs: Vec::new(),
            packs: Vec::new(),
            entries: Entries::Flat { file, offsets },
        })
    }

    /// One range's entry: its section's run, then that run's log, read at
    /// the entry's offset. Logs are opened once per catalog.
    fn entry(&self, section: usize, range: u64) -> io::Result<Option<RangeId>> {
        let Some(s) = self.header.sections.get(section) else {
            return Ok(None);
        };
        if range >= s.count {
            return Ok(None);
        }
        let mut bytes = [0; ENTRY_BYTES as usize];
        let read = match &self.entries {
            Entries::Flat { file, offsets } => {
                read_at(file, &mut bytes, offsets[section] + range * ENTRY_BYTES)?
            }
            Entries::Logs(_) => {
                let runs = &self.runs[section];
                let run = runs[runs.partition_point(|run| run.end() <= range)];
                let log = self.log(run.log)?;
                read_at(
                    &log,
                    &mut bytes,
                    run.offset + (range - run.first) * ENTRY_BYTES,
                )?
            }
        };
        if read != bytes.len() {
            return Err(invalid("truncated prepared entry log"));
        }
        RangeId::decode(&bytes).map(Some)
    }

    /// Every entry of one run, in range order.
    fn run_entries(&self, run: &Run) -> io::Result<Vec<u8>> {
        let log = self.log(run.log)?;
        let length = log.metadata()?.len();
        if run
            .offset
            .checked_add(run.bytes())
            .is_none_or(|end| end > length)
        {
            return Err(invalid("truncated prepared entry log"));
        }
        let mut bytes = vec![0; to_usize(run.bytes())?];
        if read_at(&log, &mut bytes, run.offset)? != bytes.len() {
            return Err(invalid("truncated prepared entry log"));
        }
        Ok(bytes)
    }

    fn log(&self, id: u64) -> io::Result<Arc<File>> {
        let Entries::Logs(opened) = &self.entries else {
            return Err(invalid("a prepared listing snapshot has no entry logs"));
        };
        let mut opened = opened.lock().unwrap();
        if let Some(file) = opened.get(&id) {
            return Ok(Arc::clone(file));
        }
        count!(log_opens);
        let file = Arc::new(read_file(&self.directory.join(log_name(id)))?);
        opened.insert(id, Arc::clone(&file));
        Ok(file)
    }

    /// The pack table: every pack an entry names, with its listed bytes.
    fn packs(&self) -> io::Result<Vec<PackRecord>> {
        Ok(self.packs.clone())
    }
}

/// Fills `bytes` from `offset` until the end of the file; returns the count read.
fn read_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut done = 0;
    while done < bytes.len() {
        match file.seek_read(&mut bytes[done..], offset + done as u64) {
            Ok(0) => break,
            Ok(read) => done += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(done)
}

fn write_all_at(file: &File, mut bytes: &[u8], mut offset: u64) -> io::Result<()> {
    while !bytes.is_empty() {
        match file.seek_write(bytes, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => {
                bytes = &bytes[written..];
                offset += written as u64;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The pack file one writer appends its ranges to. Bytes are buffered, then
/// written in order; no range is ever rewritten.
struct Pack {
    id: u64,
    file: Arc<File>,
    /// Bytes written to the file; the buffer holds the bytes after them.
    flushed: u64,
    /// The file's length, which a failed range can leave past `flushed`.
    extent: u64,
    buffer: Vec<u8>,
}

impl Pack {
    fn create(directory: &Path) -> io::Result<Self> {
        Self::create_pinned(directory, None)
    }

    fn create_pinned(directory: &Path, pins: Option<&StageFiles>) -> io::Result<Self> {
        let _registry = pins.map(|_| STAGING.lock().unwrap_or_else(|error| error.into_inner()));
        loop {
            let id = Uuid::new_v4().as_u64_pair().0;
            if id == RangeId::EMPTY.pack {
                continue;
            }
            match OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(SHARING)
                .create_new(true)
                .open(directory.join(pack_name(id)))
            {
                Ok(file) => {
                    if let Some(pins) = pins {
                        pins.packs
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .insert(id);
                    }
                    #[cfg(test)]
                    tests::CREATED.set(tests::CREATED.get() + 1);
                    return Ok(Self {
                        id,
                        file: Arc::new(file),
                        flushed: 0,
                        extent: 0,
                        buffer: Vec::new(),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
    }

    fn end(&self) -> u64 {
        self.flushed + self.buffer.len() as u64
    }

    /// Appends one whole range. A failed range leaves no bytes the next
    /// range would keep.
    fn append<R: Read + ?Sized>(&mut self, data: &mut R) -> io::Result<RangeId> {
        let offset = self.end();
        match self.fill(data) {
            Ok(()) => Ok(RangeId {
                pack: self.id,
                offset,
                len: self.end() - offset,
            }),
            Err(error) => {
                if offset >= self.flushed {
                    self.buffer.truncate((offset - self.flushed) as usize);
                } else {
                    self.buffer.clear();
                    self.flushed = offset;
                }
                Err(error)
            }
        }
    }

    fn fill<R: Read + ?Sized>(&mut self, data: &mut R) -> io::Result<()> {
        loop {
            if self.buffer.len() as u64 >= PACK_BUFFER {
                self.flush()?;
            }
            let room = PACK_BUFFER - self.buffer.len() as u64;
            if io::copy(&mut (&mut *data).take(room), &mut self.buffer)? < room {
                return Ok(());
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        write_all_at(&self.file, &self.buffer, self.flushed)?;
        self.flushed += self.buffer.len() as u64;
        self.extent = self.extent.max(self.flushed);
        self.buffer.clear();
        Ok(())
    }

    /// Writes out every range and drops bytes a failed range left past them.
    fn finish(&mut self) -> io::Result<Shared> {
        self.flush()?;
        if self.extent > self.flushed {
            self.file.set_len(self.flushed)?;
            self.extent = self.flushed;
        }
        Ok(Shared {
            file: Arc::clone(&self.file),
            size: self.flushed,
            durable: false,
        })
    }

    fn reader(&mut self, id: RangeId) -> io::Result<Range> {
        if id.offset + id.len > self.flushed {
            self.flush()?;
        }
        Ok(Range::new(Arc::clone(&self.file), id))
    }
}

/// A finished pack of this transaction that another writer reads or lists.
#[derive(Clone, Debug)]
struct Shared {
    file: Arc<File>,
    size: u64,
    /// Already flushed, by the fork that wrote it, so commit does not.
    durable: bool,
}

/// Empties packs under half live at commit into one new pack, in manifest order.
struct Mover<'a> {
    directory: &'a Path,
    compact: HashSet<u64>,
    fresh: &'a HashMap<u64, Shared>,
    opened: Opened,
    into: Option<Pack>,
}

impl Mover<'_> {
    fn place(&mut self, id: RangeId) -> io::Result<RangeId> {
        if id.len == 0 {
            return Ok(RangeId::EMPTY);
        }
        if !self.compact.contains(&id.pack) {
            return Ok(id);
        }
        let file = match self.fresh.get(&id.pack) {
            Some(shared) => Arc::clone(&shared.file),
            None => match self.opened.get(&id.pack) {
                Some(file) => Arc::clone(file),
                None => {
                    let file = Arc::new(read_file(&self.directory.join(pack_name(id.pack)))?);
                    self.opened.insert(id.pack, Arc::clone(&file));
                    file
                }
            },
        };
        let into = match &mut self.into {
            Some(into) => into,
            into => into.insert(Pack::create(self.directory)?),
        };
        let moved = into.append(&mut Range::new(file, id))?;
        if moved.len != id.len {
            return Err(invalid("truncated prepared range"));
        }
        Ok(moved)
    }
}

#[derive(Clone)]
struct PendingSection {
    name: Arc<str>,
    old_count: u64,
    added: Vec<RangeId>,
    replaced: BTreeMap<u64, RangeId>,
}

impl PendingSection {
    fn new(name: &str) -> Self {
        Self {
            name: Arc::from(name),
            old_count: 0,
            added: Vec::new(),
            replaced: BTreeMap::new(),
        }
    }

    fn count(&self) -> io::Result<u64> {
        self.old_count
            .checked_add(self.added.len() as u64)
            .ok_or_else(|| invalid("range count overflow"))
    }

    /// Where the transaction's current bytes of `range` live.
    fn slot(&self, range: u64) -> Slot {
        if range < self.old_count {
            self.replaced
                .get(&range)
                .map_or(Slot::Published, |&id| Slot::Staged(id))
        } else {
            usize::try_from(range - self.old_count)
                .ok()
                .and_then(|index| self.added.get(index))
                .map_or(Slot::Missing, |&id| Slot::Staged(id))
        }
    }

    /// Lists `id` at `range`, which is at most the current count. Returns the
    /// range it displaced in this transaction: None for a published range not
    /// yet replaced, or for an appended one.
    fn place(&mut self, range: u64, id: RangeId) -> Option<RangeId> {
        if range < self.old_count {
            self.replaced.insert(range, id)
        } else {
            let index = (range - self.old_count) as usize;
            match self.added.get_mut(index) {
                Some(slot) => Some(std::mem::replace(slot, id)),
                None => {
                    self.added.push(id);
                    None
                }
            }
        }
    }

    /// Takes back the latest [`Self::place`] of `range`, which returned `old`.
    fn unplace(&mut self, range: u64, old: Option<RangeId>) {
        if range < self.old_count {
            match old {
                Some(id) => self.replaced.insert(range, id),
                None => self.replaced.remove(&range),
            };
        } else {
            let index = (range - self.old_count) as usize;
            match old {
                Some(id) => self.added[index] = id,
                None => {
                    self.added.pop();
                }
            }
        }
    }
}

/// How to take back one step a refused [`Writer::adopt_all`] replayed.
enum Undo {
    /// The batch created the last section.
    Created,
    /// The batch listed a range at `range` of section `index`, over `old`.
    Placed {
        index: usize,
        range: u64,
        old: Option<RangeId>,
    },
}

/// Where a range's current bytes live within one transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// Not in the section, or no such section.
    Missing,
    /// The published range, not replaced in this transaction.
    Published,
    /// A range this transaction wrote.
    Staged(RangeId),
}

/// A section a fork used: by index when its parent already held it (indices
/// never change within a transaction), otherwise by name.
#[derive(Clone, Debug)]
enum SectionRef {
    Parent(usize),
    Named(String),
}

/// One step of a fork, replayed and re-checked against its parent by `adopt`.
#[derive(Clone, Debug)]
enum Operation {
    /// The fork created a section its parent did not hold.
    Create(String),
    /// The fork resolved `range` and found `slot`.
    Read {
        section: SectionRef,
        range: u64,
        slot: Slot,
    },
    /// The fork wrote range `id` at `range` over `old` (`Missing` appends).
    Write {
        section: SectionRef,
        range: u64,
        id: RangeId,
        old: Slot,
        bytes: u64,
        replaced: u64,
    },
}

struct Fork {
    /// The parent's section count when forked. Adoption finds sections the
    /// parent held by index and the fork's own sections by name.
    parent_sections: usize,
    batches: Vec<Staged>,
    open: Vec<Operation>,
}

// A detached batch keeps its snapshot's files and range identities until it
// joins a later transaction. Ordinary forks retain their transaction-only rules.
static STAGING: Mutex<Vec<Weak<StageFiles>>> = Mutex::new(Vec::new());

#[cfg(test)]
fn staging(directory: &Path) -> bool {
    let mut registry = STAGING.lock().unwrap_or_else(|error| error.into_inner());
    !active_stages(&mut registry, directory).is_empty()
}

fn active_stages(registry: &mut Vec<Weak<StageFiles>>, directory: &Path) -> Vec<Arc<StageFiles>> {
    let mut active = Vec::new();
    registry.retain(|weak| {
        let Some(stage) = weak.upgrade() else {
            return false;
        };
        if stage.directory == directory {
            active.push(stage);
        }
        true
    });
    active
}

#[derive(Debug)]
struct StageFiles {
    directory: PathBuf,
    // Snapshot packs keep their identities; later packs belong only to this
    // stage and its nested forks. Unrelated publications stay reclaimable.
    packs: Mutex<HashSet<u64>>,
    logs: HashSet<u64>,
}

#[derive(Debug)]
struct StageLease {
    _lock: File,
    directory: PathBuf,
    key: SourceKey,
    inherited: HashSet<u64>,
}

#[derive(Debug)]
struct Detached {
    lease: Arc<StageLease>,
    packs: HashMap<u64, Shared>,
}

/// One batch of a forked writer's work, for [`Writer::adopt`] into its parent.
/// Dropping it leaves its ranges unlisted; the next writer open removes a pack
/// that holds no listed range.
#[derive(Debug)]
pub struct Staged {
    transaction: Uuid,
    detached: Option<Arc<Detached>>,
    // Ordinary nested batches also retain the stage's files until adopted.
    _pins: Option<Arc<StageFiles>>,
    operations: Vec<Operation>,
    /// The fork's pack, finished when the fork closed.
    pack: Option<(u64, Shared)>,
}

/// Owns one exclusive-open lock. Dropping it leaves the old manifest intact;
/// packs no manifest lists are removed on the next writer open.
pub struct Writer {
    directory: PathBuf,
    _lock: File,
    key: SourceKey,
    source: Option<Uuid>,
    base: Option<Arc<Catalog>>,
    sections: Vec<PendingSection>,
    bytes_written: u64,
    bytes_replaced: u64,
    transaction: Uuid,
    /// The pack this writer appends to, created by its first write.
    pack: Option<Pack>,
    /// Other packs of this transaction: adopted forks', and on a fork, its
    /// parent's as they stood when forked.
    packs: HashMap<u64, Shared>,
    fork: Option<Fork>,
    detached: Option<Arc<StageLease>>,
    pins: Option<Arc<StageFiles>>,
    /// An adoption failed partway, so the transaction must not be published.
    incomplete: bool,
}

impl Writer {
    fn new(
        directory: PathBuf,
        lock: File,
        key: SourceKey,
        source: Option<Uuid>,
        base: Option<Arc<Catalog>>,
        sections: Vec<PendingSection>,
    ) -> Self {
        Self {
            directory,
            _lock: lock,
            key,
            source,
            base,
            sections,
            bytes_written: 0,
            bytes_replaced: 0,
            transaction: Uuid::new_v4(),
            pack: None,
            packs: HashMap::new(),
            fork: None,
            detached: None,
            pins: None,
            incomplete: false,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending_range_ownership(&self) -> (usize, usize) {
        self.sections
            .iter()
            .fold((0, 0), |(added, replaced), section| {
                (
                    added + section.added.len(),
                    replaced + section.replaced.len(),
                )
            })
    }

    /// New range payload bytes successfully written by this transaction (by
    /// this fork alone, on a fork).
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Change in published range payload bytes, excluding displaced ranges.
    pub fn payload_bytes_change(&self) -> i128 {
        i128::from(self.bytes_written) - i128::from(self.bytes_replaced)
    }

    /// A writer for another thread that reads this transaction as it stands and
    /// writes ranges, into its own pack, that reach this transaction only
    /// through [`Writer::adopt`], batch by batch ([`Writer::end_batch`],
    /// [`Writer::into_staged`]). It holds a duplicate of this writer's lock
    /// handle. Forks must use disjoint sections: adoption refuses a batch whose
    /// reads or writes another adoption changed.
    pub fn fork(&mut self) -> io::Result<Writer> {
        if self.fork.is_some() && self.detached.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "a forked prepared writer cannot fork",
            ));
        }
        let mut packs = self.packs.clone();
        if let Some(pack) = &mut self.pack {
            packs.insert(pack.id, pack.finish()?);
        }
        Ok(Writer {
            directory: self.directory.clone(),
            _lock: self._lock.try_clone()?,
            key: self.key.clone(),
            source: self.source,
            base: self.base.clone(),
            sections: self.sections.clone(),
            bytes_written: 0,
            bytes_replaced: 0,
            transaction: self.transaction,
            pack: None,
            packs,
            fork: Some(Fork {
                parent_sections: self.sections.len(),
                batches: Vec::new(),
                open: Vec::new(),
            }),
            detached: None,
            pins: self.pins.clone(),
            incomplete: false,
        })
    }

    /// Stage a long batch without holding up this writer's commits. Its ranges
    /// may be adopted into a later transaction of this directory and source key;
    /// every read and replacement is checked by immutable range identity.
    /// Nested ordinary forks support parallel writes into this batch.
    pub fn stage(&mut self) -> io::Result<Writer> {
        let lock = self._lock.try_clone()?;
        let mut writer = self.fork()?;
        let mut packs: HashSet<_> = writer.packs.keys().copied().collect();
        let mut logs = HashSet::new();
        if let Some(base) = &writer.base {
            packs.extend(base.packs.iter().map(|pack| pack.id));
            logs.extend(base.logs.iter().map(|log| log.id));
        }
        let pins = Arc::new(StageFiles {
            directory: self.directory.clone(),
            packs: Mutex::new(packs),
            logs,
        });
        {
            let mut registry = STAGING.lock().unwrap_or_else(|error| error.into_inner());
            registry.retain(|stage| stage.strong_count() != 0);
            registry.push(Arc::downgrade(&pins));
        }
        writer.pins = Some(pins);
        writer.detached = Some(Arc::new(StageLease {
            _lock: lock,
            directory: self.directory.clone(),
            key: self.key.clone(),
            inherited: writer.packs.keys().copied().collect(),
        }));
        Ok(writer)
    }

    /// Closes a fork's current batch, even if it is empty; later work forms the
    /// next batch. Does nothing on an ordinary writer.
    pub fn end_batch(&mut self) {
        if let Some(fork) = &mut self.fork {
            let operations = std::mem::take(&mut fork.open);
            fork.batches.push(Staged {
                transaction: self.transaction,
                detached: None,
                _pins: self.pins.clone(),
                operations,
                pack: None,
            });
        }
    }

    /// A fork's closed batches in order, then its open batch if it holds work.
    /// Writes out and flushes the fork's pack, on the calling thread: a flush
    /// failure is this fork's, and forks flush at once rather than in their
    /// parent's commit.
    pub fn into_staged(mut self) -> io::Result<Vec<Staged>> {
        if self.incomplete {
            return Err(invalid(
                "a refused adoption left this prepared transaction incomplete",
            ));
        }
        let Some(mut fork) = self.fork.take() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only a forked prepared writer has staged batches",
            ));
        };
        if !fork.open.is_empty() {
            fork.batches.push(Staged {
                transaction: self.transaction,
                detached: None,
                _pins: self.pins.clone(),
                operations: fork.open,
                pack: None,
            });
        }
        if self.detached.is_some() {
            for batch in &mut fork.batches {
                for operation in &mut batch.operations {
                    self.identify(operation)?;
                }
            }
        }
        if let Some(mut pack) = self.pack.take() {
            let mut shared = pack.finish()?;
            sync_packs(std::slice::from_ref(&shared.file))?;
            shared.durable = true;
            for batch in &mut fork.batches {
                batch.pack = Some((pack.id, shared.clone()));
            }
        }
        if let Some(lease) = self.detached.take() {
            self.packs.retain(|id, _| !lease.inherited.contains(id));
            let detached = Arc::new(Detached {
                lease,
                packs: self.packs,
            });
            for batch in &mut fork.batches {
                batch.detached = Some(Arc::clone(&detached));
            }
        }
        Ok(fork.batches)
    }

    fn identify(&self, operation: &mut Operation) -> io::Result<()> {
        let (section, range, slot) = match operation {
            Operation::Create(_) => return Ok(()),
            Operation::Read {
                section,
                range,
                slot,
            } => (section, *range, slot),
            Operation::Write {
                section,
                range,
                old,
                ..
            } => (section, *range, old),
        };
        let name = self.section_name(section).to_owned();
        if *slot == Slot::Published {
            let index = self.position(&name).ok_or_else(|| overlap(&name))?;
            *slot = self
                .slot_id(index, range, *slot)?
                .map_or(Slot::Missing, Slot::Staged);
        }
        *section = SectionRef::Named(name);
        Ok(())
    }

    fn accepts(&self, staged: &Staged) -> bool {
        match &staged.detached {
            Some(detached) => {
                detached.lease.directory == self.directory && detached.lease.key == self.key
            }
            None => {
                // Nested ordinary forks belong to their stage's snapshot,
                // even when its original publishing transaction has ended.
                staged.transaction == self.transaction
                    && match (&self.pins, &staged._pins) {
                        (None, None) => true,
                        (Some(parent), Some(child)) => Arc::ptr_eq(parent, child),
                        _ => false,
                    }
            }
        }
    }

    // Nested flushes' operations become part of the detached parent's journal.
    fn journal(&self, operations: &[Operation]) -> Vec<Operation> {
        if self.detached.is_none() {
            return Vec::new();
        }
        operations
            .iter()
            .cloned()
            .map(|mut operation| {
                let section = match &mut operation {
                    Operation::Create(_) => return operation,
                    Operation::Read { section, .. } | Operation::Write { section, .. } => section,
                };
                if let SectionRef::Parent(index) = section {
                    *section = self.reference(Some(*index), &self.sections[*index].name);
                }
                operation
            })
            .collect()
    }

    fn pin_adoption(&self, staged: &Staged, added: &mut Vec<u64>) {
        if let Some(pins) = &self.pins {
            let mut packs = pins.packs.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(detached) = &staged.detached {
                for &id in detached.packs.keys() {
                    if packs.insert(id) {
                        added.push(id);
                    }
                }
            }
            if let Some((id, _)) = &staged.pack
                && packs.insert(*id)
            {
                added.push(*id);
            }
        }
    }

    /// Lists a fork's batch in this transaction exactly as if its operations had
    /// been made here, in order: the same section numbers, ranges and byte
    /// counts. Every recorded read and replaced range must still be what the
    /// fork saw; otherwise the batch is refused (`ErrorKind::Other`), and so
    /// is a later commit of this transaction.
    pub fn adopt(&mut self, staged: Staged) -> io::Result<()> {
        if (self.fork.is_some() && self.detached.is_none()) || !self.accepts(&staged) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staged prepared ranges belong to another transaction or stage",
            ));
        }
        let journal = self.journal(&staged.operations);
        self.pin_adoption(&staged, &mut Vec::new());
        if let Some(detached) = &staged.detached {
            self.packs.extend(detached.packs.clone());
        }
        if let Some((id, shared)) = staged.pack {
            self.packs.insert(id, shared);
        }
        let result = self.replay(
            staged.operations,
            &mut Vec::new(),
            staged.detached.is_some(),
        );
        self.incomplete |= result.is_err();
        if result.is_ok()
            && let Some(fork) = &mut self.fork
        {
            fork.open.extend(journal);
        }
        result
    }

    /// Lists every batch, in order, as [`Writer::adopt`] does, or none of them:
    /// a refused batch (`ErrorKind::Other`) takes back the batches before it,
    /// leaving this transaction exactly as it was and still publishable. One
    /// producer's batches thus join a transaction other producers' batches
    /// share whole or not at all, and a refusal stays that producer's.
    pub fn adopt_all(&mut self, batches: Vec<Staged>) -> io::Result<()> {
        if (self.fork.is_some() && self.detached.is_none())
            || batches.iter().any(|staged| !self.accepts(staged))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staged prepared ranges belong to another transaction or stage",
            ));
        }
        let before = (self.bytes_written, self.bytes_replaced, self.sections.len());
        let mut undo = Vec::new();
        let mut packs = Vec::new();
        let mut pinned = Vec::new();
        let journal: Vec<_> = batches
            .iter()
            .flat_map(|staged| self.journal(&staged.operations))
            .collect();
        for staged in batches {
            self.pin_adoption(&staged, &mut pinned);
            if let Some(detached) = &staged.detached {
                for (&id, shared) in &detached.packs {
                    if let std::collections::hash_map::Entry::Vacant(entry) = self.packs.entry(id) {
                        entry.insert(shared.clone());
                        packs.push(id);
                    }
                }
            }
            if let Some((id, shared)) = staged.pack
                && let std::collections::hash_map::Entry::Vacant(entry) = self.packs.entry(id)
            {
                entry.insert(shared);
                packs.push(id);
            }
            if let Err(error) = self.replay(staged.operations, &mut undo, staged.detached.is_some())
            {
                for step in undo.into_iter().rev() {
                    match step {
                        Undo::Created => {
                            self.sections.pop();
                        }
                        Undo::Placed { index, range, old } => {
                            self.sections[index].unplace(range, old);
                        }
                    }
                }
                debug_assert_eq!(self.sections.len(), before.2);
                (self.bytes_written, self.bytes_replaced) = (before.0, before.1);
                for id in packs {
                    self.packs.remove(&id);
                }
                if let Some(pins) = &self.pins {
                    let mut packs = pins.packs.lock().unwrap_or_else(|error| error.into_inner());
                    for id in pinned {
                        packs.remove(&id);
                    }
                }
                return Err(error);
            }
        }
        if let Some(fork) = &mut self.fork {
            fork.open.extend(journal);
        }
        Ok(())
    }

    /// Replays a batch's operations, noting in `undo` how to take back each
    /// step it made.
    fn replay(
        &mut self,
        operations: Vec<Operation>,
        undo: &mut Vec<Undo>,
        detached: bool,
    ) -> io::Result<()> {
        for operation in operations {
            match operation {
                Operation::Create(name) => {
                    if self.position(&name).is_some() {
                        return Err(overlap(&name));
                    }
                    self.sections.push(PendingSection::new(&name));
                    undo.push(Undo::Created);
                }
                Operation::Read {
                    section,
                    range,
                    slot,
                } => {
                    let current = match self.resolve(&section) {
                        Some(index) => self.checked_slot(index, range, detached)?,
                        None => Slot::Missing,
                    };
                    if current != slot {
                        return Err(overlap(self.section_name(&section)));
                    }
                }
                Operation::Write {
                    section,
                    range,
                    id,
                    old,
                    bytes,
                    replaced,
                } => {
                    let index = self
                        .resolve(&section)
                        .ok_or_else(|| overlap(self.section_name(&section)))?;
                    let entry = &self.sections[index];
                    if self.checked_slot(index, range, detached)? != old
                        || (old == Slot::Missing && range != entry.count()?)
                    {
                        return Err(overlap(&entry.name));
                    }
                    let written = self.bytes_written.checked_add(bytes);
                    let displaced = self.bytes_replaced.checked_add(replaced);
                    let (Some(written), Some(displaced)) = (written, displaced) else {
                        return Err(invalid("prepared byte count overflow"));
                    };
                    let old = self.sections[index].place(range, id);
                    undo.push(Undo::Placed { index, range, old });
                    self.bytes_written = written;
                    self.bytes_replaced = displaced;
                }
            }
        }
        Ok(())
    }

    fn checked_slot(&self, index: usize, range: u64, detached: bool) -> io::Result<Slot> {
        let slot = self.sections[index].slot(range);
        if detached {
            Ok(self
                .slot_id(index, range, slot)?
                .map_or(Slot::Missing, Slot::Staged))
        } else {
            Ok(slot)
        }
    }

    fn position(&self, section: &str) -> Option<usize> {
        self.sections
            .iter()
            .position(|entry| *entry.name == *section)
    }

    fn resolve(&self, section: &SectionRef) -> Option<usize> {
        match section {
            SectionRef::Parent(index) => Some(*index),
            SectionRef::Named(name) => self.position(name),
        }
    }

    fn section_name<'a>(&'a self, section: &'a SectionRef) -> &'a str {
        match section {
            SectionRef::Parent(index) => &self.sections[*index].name,
            SectionRef::Named(name) => name,
        }
    }

    /// On a fork, how adoption finds a section: by index if the parent held it.
    fn reference(&self, index: Option<usize>, section: &str) -> SectionRef {
        match (index, &self.fork) {
            (Some(index), Some(fork)) if index < fork.parent_sections => SectionRef::Parent(index),
            _ => SectionRef::Named(section.to_owned()),
        }
    }

    fn record(&mut self, operation: impl FnOnce(&Self) -> Operation) {
        if self.fork.is_some() {
            let operation = operation(self);
            if let Some(fork) = &mut self.fork {
                fork.open.push(operation);
            }
        }
    }

    fn slot_id(&self, index: usize, range: u64, slot: Slot) -> io::Result<Option<RangeId>> {
        match slot {
            Slot::Missing => Ok(None),
            Slot::Staged(id) => Ok(Some(id)),
            Slot::Published => self
                .base
                .as_ref()
                .ok_or_else(|| invalid("missing prefix"))?
                .entry(index, range),
        }
    }

    /// Appends one range to this writer's pack, creating the pack first.
    fn append(&mut self, data: &mut impl Read) -> io::Result<RangeId> {
        let pack = match &mut self.pack {
            Some(pack) => pack,
            pack => pack.insert(Pack::create_pinned(&self.directory, self.pins.as_deref())?),
        };
        let id = pack.append(data)?;
        Ok(if id.len == 0 { RangeId::EMPTY } else { id })
    }

    /// The latest range's immutable identity without opening its pack or
    /// flushing staged bytes. On a fork this records the same dependency as
    /// `read_at`, so adoption checks what an identity-based cache reused.
    pub fn range_id(&mut self, section: &str, range: u64) -> io::Result<Option<RangeId>> {
        let index = self.position(section);
        let slot = index.map_or(Slot::Missing, |index| self.sections[index].slot(range));
        self.record(|writer| Operation::Read {
            section: writer.reference(index, section),
            range,
            slot,
        });
        let Some(index) = index else {
            return Ok(None);
        };
        self.slot_id(index, range, slot)
    }

    /// Opens the transaction's latest range, including unpublished tail pages.
    /// Producers can evict a page buffer and continue it before commit. A fork
    /// reads its own ranges, else its parent's as they stood when forked.
    pub fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Range>> {
        let Some(id) = self.range_id(section, range)? else {
            return Ok(None);
        };
        if let Some(pack) = self.pack.as_mut().filter(|pack| pack.id == id.pack) {
            return pack.reader(id).map(Some);
        }
        if let Some(shared) = self.packs.get(&id.pack) {
            return Ok(Some(Range::new(Arc::clone(&shared.file), id)));
        }
        id.open(&self.directory).map(Some)
    }

    /// Replaces one logical range with a new whole range, or appends the next.
    /// The old bytes and manifest remain untouched until manifest publication.
    pub fn write_at(&mut self, section: &str, range: u64, mut data: impl Read) -> io::Result<()> {
        let Some(index) = self.position(section) else {
            if range != 0 {
                return Err(invalid(&format!(
                    "non-consecutive prepared range {range} in {section}"
                )));
            }
            self.write(section, data)?;
            return Ok(());
        };
        let count = self.sections[index].count()?;
        if range == count {
            self.write(section, data)?;
            return Ok(());
        }
        if range > count {
            return Err(invalid(&format!(
                "non-consecutive prepared range {range} in {section}"
            )));
        }
        let slot = self.sections[index].slot(range);
        let old = self
            .slot_id(index, range, slot)?
            .ok_or_else(|| invalid("missing prepared range"))?;
        let replaced = self
            .bytes_replaced
            .checked_add(old.len)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        let id = self.append(&mut data)?;
        self.bytes_written = self
            .bytes_written
            .checked_add(id.len)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        self.bytes_replaced = replaced;
        self.sections[index].place(range, id);
        self.record(|writer| Operation::Write {
            section: writer.reference(Some(index), section),
            range,
            id,
            old: slot,
            bytes: id.len,
            replaced: old.len,
        });
        Ok(())
    }

    /// Streams a whole, immutable range; no existing range is modified.
    /// Ranges are consecutive per section, with boundaries defined by its producer.
    pub fn write(&mut self, section: &str, mut data: impl Read) -> io::Result<u64> {
        if section.is_empty() {
            return Err(invalid("empty section name"));
        }
        let index = match self.position(section) {
            Some(index) => index,
            None => {
                self.sections.push(PendingSection::new(section));
                self.record(|_| Operation::Create(section.to_owned()));
                self.sections.len() - 1
            }
        };
        let range = self.sections[index].count()?;
        let id = self.append(&mut data)?;
        self.bytes_written = self
            .bytes_written
            .checked_add(id.len)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        self.sections[index].added.push(id);
        self.record(|writer| Operation::Write {
            section: writer.reference(Some(index), section),
            range,
            id,
            old: Slot::Missing,
            bytes: id.len,
            replaced: 0,
        });
        Ok(range)
    }

    /// Publish and reopen under the same exclusive lock. Detached staging
    /// writers keep duplicate handles, so reopening must not acquire a new lock.
    pub fn commit_continue(self) -> io::Result<Writer> {
        let lock = self._lock.try_clone()?;
        let key = self.key.clone();
        let store = Store {
            directory: self.directory.clone(),
        };
        self.commit()?;
        if !key.recording_current()? {
            return Err(invalid("sources changed before preparation resumed"));
        }
        store.continue_locked(&key, key.clone(), lock)
    }

    /// Publishes the root last. The commit's new, replaced and moved entries
    /// go to one new entry log; every section keeps the runs of its unchanged
    /// entries where they are, so the root, the log and the names it appends
    /// grow with what the commit changed, not with what the store holds. Each
    /// pack and log the commit created or appended to is flushed once first
    /// (a fork's pack was flushed by its fork); packs under half live are
    /// emptied into one new pack. A post-rename flush error can leave the
    /// complete new root visible. A fork publishes only through its parent's
    /// [`Writer::adopt`].
    pub fn commit(mut self) -> io::Result<()> {
        if self.fork.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a forked prepared writer publishes through adopt",
            ));
        }
        if self.incomplete {
            return Err(io::Error::other(
                "a refused adoption left this prepared transaction incomplete",
            ));
        }
        let valid = self.key.current_for_commit();
        if !matches!(&valid, Ok(true)) {
            self.key.rejected.store(true, Ordering::Release);
            if let Some((_, _, accepted)) = self.key.binding.lock().unwrap().as_mut() {
                *accepted = false;
            }
        }
        if !valid? {
            return Err(invalid("sources changed during preparation"));
        }
        let mut fresh = std::mem::take(&mut self.packs);
        if let Some(pack) = &mut self.pack {
            fresh.insert(pack.id, pack.finish()?);
        }
        let mut table = self.pack_table(&fresh)?;
        // Packs under half live are emptied into one new pack, so the listed
        // unpinned packs hold at least as many listed bytes as dead ones.
        // Packs supplied by this commit are emptied at once; finding the
        // entries that name another published pack reads every entry,
        // so published packs are emptied together, once the listed packs
        // hold more than a quarter dead bytes.
        let under = |live: u64, size: u64| u128::from(live) * 2 < u128::from(size);
        let pinned: HashSet<u64> = {
            let mut registry = STAGING.lock().unwrap_or_else(|error| error.into_inner());
            active_stages(&mut registry, &self.directory)
                .iter()
                .flat_map(|stage| {
                    stage
                        .packs
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .iter()
                        .copied()
                        .collect::<Vec<_>>()
                })
                .collect()
        };
        let (live, size) = table.iter().filter(|(id, _)| !pinned.contains(*id)).fold(
            (0u128, 0u128),
            |(all_live, all_size), (_, &(live, size))| {
                (all_live + u128::from(live), all_size + u128::from(size))
            },
        );
        let published_too = size.saturating_sub(live) * 4 > live;
        let compact: HashSet<u64> = table
            .iter()
            .filter(|(id, (live, size))| {
                !pinned.contains(*id)
                    && under(*live, *size)
                    && (published_too || fresh.contains_key(*id))
            })
            .map(|(&id, _)| id)
            .collect();
        table.retain(|id, _| !compact.contains(id));
        // A later batch can supply a pack that an earlier batch already
        // published. Its unchanged published ranges must move as well.
        let scan = self
            .base
            .as_ref()
            .is_some_and(|base| base.packs.iter().any(|pack| compact.contains(&pack.id)));
        let mut mover = Mover {
            directory: &self.directory,
            compact,
            fresh: &fresh,
            opened: Opened::new(),
            into: None,
        };
        let source = match self.source {
            Some(source) => source,
            None => {
                let source = Uuid::new_v4();
                publish(&self.directory.join(source_name(source)), |file| {
                    file.write_all(&serde_json::to_vec(&self.key)?)
                })?;
                source
            }
        };
        let base = self.base.clone();
        let base = base.as_deref();
        let base_sections = base.map_or(0, |base| base.header.sections.len());
        // This commit's entries, in the order its runs list them.
        let mut log = Vec::new();
        let mut section_runs = Vec::with_capacity(self.sections.len());
        for (index, section) in self.sections.iter().enumerate() {
            let mut runs = match base {
                Some(base) if index < base_sections => base.runs[index].clone(),
                _ => Vec::new(),
            };
            let mut changes = section.replaced.clone();
            if scan {
                // Published ranges in the packs being emptied move too.
                for run in &runs {
                    let entries = base
                        .ok_or_else(|| invalid("missing prefix"))?
                        .run_entries(run)?;
                    for (at, entry) in entries.chunks_exact(ENTRY_BYTES as usize).enumerate() {
                        let range = run.first + at as u64;
                        let id = RangeId::decode(entry)?;
                        if id.len != 0
                            && mover.compact.contains(&id.pack)
                            && !section.replaced.contains_key(&range)
                        {
                            changes.insert(range, id);
                        }
                    }
                }
            }
            // Changed ranges, a run for each consecutive span, laid over the
            // runs they replace in one ordered pass.
            let mut spans: Vec<Run> = Vec::new();
            for (&range, &id) in &changes {
                let placed = mover.place(id)?;
                match spans.last_mut() {
                    Some(run) if run.end() == range => run.count += 1,
                    _ => spans.push(Run {
                        first: range,
                        count: 1,
                        log: NEW_LOG,
                        offset: log.len() as u64,
                    }),
                }
                log.extend_from_slice(&placed.encode());
            }
            if !spans.is_empty() {
                runs = lay_over(&runs, &spans, base, &mut log)?;
            }
            if !section.added.is_empty() {
                let offset = log.len() as u64;
                for &id in &section.added {
                    log.extend_from_slice(&mover.place(id)?.encode());
                }
                runs.push(Run {
                    first: section.old_count,
                    count: section.added.len() as u64,
                    log: NEW_LOG,
                    offset,
                });
            }
            // A new last run absorbs the run before it while that one is no
            // longer, so an appended section's runs halve in length towards
            // its end: each entry is copied once per doubling of its section.
            while let &[.., before, last] = &runs[..]
                && last.log == NEW_LOG
                && before.count <= last.count
            {
                runs.truncate(runs.len() - 2);
                let offset = log.len() as u64;
                copy_run(&before, base, &mut log)?;
                copy_run(&last, base, &mut log)?;
                runs.push(Run {
                    first: before.first,
                    count: before.count + last.count,
                    log: NEW_LOG,
                    offset,
                });
            }
            if runs.len() > MAX_RUNS {
                let offset = log.len() as u64;
                for run in &runs {
                    copy_run(run, base, &mut log)?;
                }
                runs = vec![Run {
                    first: 0,
                    count: section.count()?,
                    log: NEW_LOG,
                    offset,
                }];
            }
            section_runs.push(runs);
        }
        // Logs under half live are emptied into this commit's, as packs are.
        let base_logs = base.map_or(&[][..], |base| &base.logs[..]);
        let mut live_logs: HashMap<u64, u64> = HashMap::new();
        for run in section_runs.iter().flatten() {
            if run.log != NEW_LOG {
                *live_logs.entry(run.log).or_default() += run.bytes();
            }
        }
        let emptied: HashSet<u64> = base_logs
            .iter()
            .filter(|record| {
                live_logs
                    .get(&record.id)
                    .is_some_and(|&live| under(live, record.size))
            })
            .map(|record| record.id)
            .collect();
        for run in section_runs.iter_mut().flatten() {
            if emptied.contains(&run.log) {
                let offset = log.len() as u64;
                copy_run(run, base, &mut log)?;
                (run.log, run.offset) = (NEW_LOG, offset);
            }
        }
        // Runs merged within this commit left their first copies behind:
        // the log holds each listed run once, in section order.
        let mut packed = Vec::with_capacity(log.len());
        for run in section_runs.iter_mut().flatten() {
            if run.log == NEW_LOG {
                let start = to_usize(run.offset)?;
                run.offset = packed.len() as u64;
                packed.extend_from_slice(&log[start..start + to_usize(run.bytes())?]);
            }
        }
        let log = packed;
        let mut logs: Vec<PackRecord> = base_logs
            .iter()
            .filter(|record| !emptied.contains(&record.id))
            .filter_map(|record| {
                live_logs.get(&record.id).map(|&live| PackRecord {
                    id: record.id,
                    live,
                    size: record.size,
                })
            })
            .collect();
        // The packs this transaction lists that no fork flushed, its log and
        // its names, flushed before the root that names them.
        let mut durable: Vec<_> = table
            .keys()
            .filter_map(|id| fresh.get(id))
            .filter(|shared| !shared.durable)
            .map(|shared| Arc::clone(&shared.file))
            .collect();
        let mut written = 0;
        if !log.is_empty() {
            let (id, file) = create_named(&self.directory, log_name)?;
            write_all_at(&file, &log, 0)?;
            let mut live = 0;
            for run in section_runs.iter_mut().flatten() {
                if run.log == NEW_LOG {
                    run.log = id;
                    live += run.bytes();
                }
            }
            logs.push(PackRecord {
                id,
                live,
                size: log.len() as u64,
            });
            durable.push(Arc::new(file));
            written += log.len();
        }
        logs.sort_by_key(|record| record.id);
        let (mut names, mut names_bytes) = base.map_or((0, 0), |base| base.names);
        if self.sections.len() > base_sections {
            let appended = encode_names(
                self.sections[base_sections..]
                    .iter()
                    .map(|section| &*section.name),
            );
            let file = if names == 0 {
                let (id, file) = create_named(&self.directory, names_name)?;
                names = id;
                file
            } else {
                OpenOptions::new()
                    .write(true)
                    .share_mode(SHARING)
                    .open(self.directory.join(names_name(names)))?
            };
            // Readers read only the bytes their root lists; names an
            // unpublished transaction left past them are written over.
            file.set_len(names_bytes)?;
            write_all_at(&file, &appended, names_bytes)?;
            names_bytes += appended.len() as u64;
            durable.push(Arc::new(file));
            written += appended.len();
        }
        let mut records: Vec<_> = table
            .iter()
            .map(|(&id, &(live, size))| PackRecord { id, live, size })
            .collect();
        if let Some(into) = &mut mover.into {
            let moved = into.finish()?;
            records.push(PackRecord {
                id: into.id,
                live: moved.size,
                size: moved.size,
            });
            durable.push(moved.file);
        }
        records.sort_by_key(|record| record.id);
        sync_packs(&durable)?;
        #[cfg(test)]
        if tests::TERMINATE_BEFORE_ROOT.get() {
            return Err(io::Error::other("simulated termination before the root"));
        }
        let sections: Vec<_> = self
            .sections
            .iter()
            .zip(&section_runs)
            .map(|(section, runs)| Ok((section.count()?, &runs[..])))
            .collect::<io::Result<_>>()?;
        let root = encode_root(
            &Head {
                version: VERSION,
                source,
                names,
                names_bytes,
                sections: sections.len() as u64,
                runs: section_runs.iter().map(|runs| runs.len() as u64).sum(),
                logs: logs.len() as u64,
            },
            &sections,
            &logs,
            &records,
        )?;
        written += root.len();
        #[cfg(test)]
        tests::MANIFEST_BYTES.set(tests::MANIFEST_BYTES.get() + written);
        let _ = written;
        publish(&self.directory.join(MANIFEST), |file| file.write_all(&root))?;
        *self.key.binding.lock().unwrap() = Some((self.directory.clone(), source, true));
        self.key.rejected.store(false, Ordering::Release);
        Ok(())
    }

    /// Listed and total bytes of every pack the new manifest will name.
    fn pack_table(&self, fresh: &HashMap<u64, Shared>) -> io::Result<HashMap<u64, (u64, u64)>> {
        let mut table = HashMap::new();
        if let Some(base) = &self.base {
            for pack in base.packs()? {
                table.insert(pack.id, (pack.live, pack.size));
            }
        }
        for (&id, shared) in fresh {
            // Separate batches of one stage can publish this pack in separate
            // commits. Its earlier ranges are still live in the base.
            table.entry(id).or_insert((0, shared.size));
        }
        let accounting = || invalid("prepared pack accounting does not match its ranges");
        for (index, section) in self.sections.iter().enumerate() {
            for &range in section.replaced.keys() {
                let old = self
                    .slot_id(index, range, Slot::Published)?
                    .ok_or_else(accounting)?;
                if old.len == 0 {
                    continue;
                }
                let (live, _) = table.get_mut(&old.pack).ok_or_else(accounting)?;
                *live = live.checked_sub(old.len).ok_or_else(accounting)?;
            }
            for id in section
                .added
                .iter()
                .chain(section.replaced.values())
                .filter(|id| id.len != 0)
            {
                let (live, _) = table.get_mut(&id.pack).ok_or_else(accounting)?;
                *live = live.checked_add(id.len).ok_or_else(accounting)?;
            }
        }
        table.retain(|_, (live, _)| *live > 0);
        Ok(table)
    }
}

/// Flushes packs on scoped threads bounded by the CPU count. Test builds with
/// `test-support` skip it, as [`durable`] does.
fn sync_packs(files: &[Arc<File>]) -> io::Result<()> {
    #[cfg(test)]
    tests::SYNCS.set(tests::SYNCS.get() + files.len());
    if files.is_empty() || skip_sync() {
        return Ok(());
    }
    let workers = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(files.len());
    if workers <= 1 {
        return files.iter().try_for_each(|file| file.sync_all());
    }
    std::thread::scope(|scope| {
        let tasks: Vec<_> = files
            .chunks(files.len().div_ceil(workers))
            .map(|part| {
                let priority = job_priority::JobPriority::current();
                scope.spawn(move || {
                    priority.enter(job_priority::Level::Normal);
                    part.iter().try_for_each(|file| file.sync_all())
                })
            })
            .collect();
        let mut error = None;
        for task in tasks {
            if let Err(why) = task
                .join()
                .map_err(|_| io::Error::other("Pack flush worker panicked"))?
            {
                error.get_or_insert(why);
            }
        }
        error.map_or(Ok(()), Err)
    })
}

fn publish(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .access_mode(DELETE | GENERIC_WRITE)
        .share_mode(SHARING)
        .create_new(true)
        .open(&temporary)?;
    #[cfg(test)]
    tests::CREATED.set(tests::CREATED.get() + 1);
    write(&mut file)?;
    durable(&temporary)?;
    let target: Vec<_> = std::path::absolute(path)?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if !posix_replace(&file, &target)? {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "prepared storage requires POSIX replacement",
        ));
    }
    drop(file);
    durable(path)
}

/// Flushes a published file and its directory. Test builds with `test-support`
/// skip the flushes: no test can observe them, and they would dominate the
/// prepared pipeline tests, which publish thousands of times.
fn durable(path: &Path) -> io::Result<()> {
    #[cfg(test)]
    tests::SYNCS.set(tests::SYNCS.get() + 1);
    if skip_sync() { Ok(()) } else { sync_file(path) }
}

fn skip_sync() -> bool {
    #[cfg(test)]
    if tests::FORCE_SYNC.get() {
        return false;
    }
    cfg!(feature = "test-support")
}

fn read_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(SHARING).open(path)
}

fn pack_name(id: u64) -> String {
    format!("pack_{id:016x}.bin")
}

fn source_name(id: Uuid) -> String {
    format!("source_{}.json", id.simple())
}

fn parse_pack(name: &str) -> Option<u64> {
    let id = u64::from_str_radix(name.strip_prefix("pack_")?.strip_suffix(".bin")?, 16).ok()?;
    (pack_name(id) == name).then_some(id)
}

fn log_name(id: u64) -> String {
    format!("entries_{id:016x}.bin")
}

fn parse_log(name: &str) -> Option<u64> {
    let id = u64::from_str_radix(name.strip_prefix("entries_")?.strip_suffix(".bin")?, 16).ok()?;
    (log_name(id) == name).then_some(id)
}

fn names_name(id: u64) -> String {
    format!("names_{id:016x}.bin")
}

/// Creates a file named by a new random non-zero id.
fn create_named(directory: &Path, name: fn(u64) -> String) -> io::Result<(u64, File)> {
    loop {
        let id = Uuid::new_v4().as_u64_pair().0;
        if id == NEW_LOG {
            continue;
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(SHARING)
            .create_new(true)
            .open(directory.join(name(id)))
        {
            Ok(file) => {
                #[cfg(test)]
                tests::CREATED.set(tests::CREATED.get() + 1);
                return Ok((id, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
}

/// A section's runs once `spans` (this commit's, sorted and disjoint, within
/// the published ranges) are laid over its `runs`, in one ordered pass that
/// visits each run and span once. Past [`MAX_RUNS`] pieces the section is
/// rewritten as one run of this commit's log in the same pass, reading each
/// published run's entries once.
fn lay_over(
    runs: &[Run],
    spans: &[Run],
    base: Option<&Catalog>,
    log: &mut Vec<u8>,
) -> io::Result<Vec<Run>> {
    let mut layout = Overlaid {
        runs,
        base,
        log,
        pieces: Vec::new(),
        rewrite: None,
        loaded: None,
    };
    let mut next = 0;
    let mut covered = 0;
    for (index, run) in runs.iter().enumerate() {
        let mut at = run.first.max(covered);
        while at < run.end() {
            if let Some(span) = spans.get(next)
                && span.first <= at
            {
                layout.push(*span, None)?;
                (at, covered, next) = (span.end(), span.end(), next + 1);
                continue;
            }
            let stop = spans
                .get(next)
                .map_or(run.end(), |span| span.first.min(run.end()));
            layout.push(
                Run {
                    first: at,
                    count: stop - at,
                    log: run.log,
                    offset: run.offset + (at - run.first) * ENTRY_BYTES,
                },
                Some(index),
            )?;
            (at, covered) = (stop, stop);
        }
    }
    for span in &spans[next..] {
        layout.push(*span, None)?;
    }
    Ok(match layout.rewrite {
        Some(run) => vec![run],
        None => layout.pieces.into_iter().map(|(piece, _)| piece).collect(),
    })
}

/// The pieces [`lay_over`] lists, or the one run it rewrites them into.
struct Overlaid<'a> {
    runs: &'a [Run],
    base: Option<&'a Catalog>,
    log: &'a mut Vec<u8>,
    /// Each piece, and the published run it is part of.
    pieces: Vec<(Run, Option<usize>)>,
    rewrite: Option<Run>,
    /// The published run whose entries were read last, and those entries.
    loaded: Option<(usize, Vec<u8>)>,
}

impl Overlaid<'_> {
    fn push(&mut self, piece: Run, source: Option<usize>) -> io::Result<()> {
        #[cfg(test)]
        tests::RUN_VISITS.set(tests::RUN_VISITS.get() + 1);
        if self.rewrite.is_none() {
            self.pieces.push((piece, source));
            if self.pieces.len() <= MAX_RUNS {
                return Ok(());
            }
            self.rewrite = Some(Run {
                first: self.pieces[0].0.first,
                count: 0,
                log: NEW_LOG,
                offset: self.log.len() as u64,
            });
            for (piece, source) in std::mem::take(&mut self.pieces) {
                self.copy(piece, source)?;
            }
            return Ok(());
        }
        self.copy(piece, source)
    }

    /// Appends a piece's entries to the run being rewritten.
    fn copy(&mut self, piece: Run, source: Option<usize>) -> io::Result<()> {
        match source {
            None => copy_run(&piece, None, self.log)?,
            Some(index) => {
                if self
                    .loaded
                    .as_ref()
                    .is_none_or(|(loaded, _)| *loaded != index)
                {
                    let base = self.base.ok_or_else(|| invalid("missing prefix"))?;
                    self.loaded = Some((index, base.run_entries(&self.runs[index])?));
                }
                let (_, entries) = self.loaded.as_ref().expect("loaded run");
                let start = to_usize(piece.offset - self.runs[index].offset)?;
                self.log
                    .extend_from_slice(&entries[start..start + to_usize(piece.bytes())?]);
            }
        }
        if let Some(rewrite) = &mut self.rewrite {
            rewrite.count += piece.count;
        }
        Ok(())
    }
}

/// Appends a run's entries to this commit's log: from the log itself for a
/// run of this commit, otherwise from the published log that holds them.
fn copy_run(run: &Run, base: Option<&Catalog>, log: &mut Vec<u8>) -> io::Result<()> {
    if run.log == NEW_LOG {
        let start = to_usize(run.offset)?;
        log.extend_from_within(start..start + to_usize(run.bytes())?);
    } else {
        let base = base.ok_or_else(|| invalid("missing prefix"))?;
        log.extend_from_slice(&base.run_entries(run)?);
    }
    Ok(())
}

fn overlap(section: &str) -> io::Error {
    io::Error::other(format!(
        "parallel prepared writers overlapped in section {section}"
    ))
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
