//! Prepared ranges in append-only pack files, listed by an atomically
//! replaced, flat manifest.
#![cfg(windows)]

use replay::publication::{posix_delete, posix_replace, sync_file};
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
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

const MANIFEST: &str = "manifest";
const LOCK: &str = "writer.lock";
/// Manifest entries name a range's pack, offset and length. Manifests of an
/// older layout (one file per range) read as absent.
const VERSION: u32 = 3;
/// A manifest entry: pack id, offset and length, little-endian.
const ENTRY_BYTES: u64 = 24;
/// A pack table record: pack id, listed (live) bytes and pack length.
const PACK_BYTES: u64 = 24;
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
                scope.spawn(move || {
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
    pub fn recording_size(&self) -> u64 {
        self.recording.size
    }

    pub fn capture(recording: &Path, content: &[PathBuf]) -> io::Result<Self> {
        Ok(Self {
            recording: Stamp::capture(recording)?,
            content: Arc::new(stamps(content, |path| Stamp::capture(path))?),
            binding: Arc::default(),
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

    /// Refresh recording-only metadata without validating the content binding.
    pub fn refresh_recording(&self) -> io::Result<Self> {
        let recording = Stamp::read(self.recording.path.clone())?;
        if recording == self.recording {
            return Ok(self.clone());
        }
        Ok(Self {
            recording,
            content: Arc::clone(&self.content),
            binding: Arc::default(),
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
    pub fn local(campaign: Uuid) -> io::Result<Self> {
        let root = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| invalid("LOCALAPPDATA is unavailable"))?;
        Ok(Self {
            directory: PathBuf::from(root)
                .join("HOI4Replay/prepared")
                .join(campaign.to_string()),
        })
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
        if key.recording.path != previous.recording.path
            || key.recording.size <= previous.recording.size
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
        let lock = self.lock()?;
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
    }

    /// Keeps the lock, the manifest, its source stamps and the packs its table
    /// lists; removes every other name in one directory pass.
    fn cleanup(&self, catalog: Option<&Catalog>) -> io::Result<()> {
        let listed: HashSet<u64> = match catalog {
            Some(catalog) => catalog.packs()?.into_iter().map(|pack| pack.id).collect(),
            None => HashSet::new(),
        };
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let keep = name == LOCK
                || catalog.is_some_and(|catalog| {
                    name == MANIFEST || name == source_name(catalog.header.source).as_str()
                })
                || name
                    .to_str()
                    .and_then(parse_pack)
                    .is_some_and(|id| listed.contains(&id));
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
    }

    impl std::ops::Sub for Counters {
        type Output = Self;
        fn sub(self, earlier: Self) -> Self {
            Self {
                stats: self.stats - earlier.stats,
                manifest_reads: self.manifest_reads - earlier.manifest_reads,
                pack_opens: self.pack_opens - earlier.pack_opens,
            }
        }
    }

    thread_local! {
        static COUNTS: Cell<Counters> = const {
            Cell::new(Counters {
                stats: 0,
                manifest_reads: 0,
                pack_opens: 0,
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

fn prefix_inputs(previous: &SourceKey, current: &SourceKey) -> io::Result<bool> {
    Ok(current.recording.path == previous.recording.path
        && current.recording.size >= previous.recording.size
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

/// A read-only view of a published manifest, or a copy of one, for
/// measurement tools. Manifests of an older layout read as absent.
pub struct Listing(Catalog);

impl Listing {
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
}

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    source: Uuid,
    sections: Vec<Section>,
}

/// Read first, so an older layout is recognised before its header is parsed.
#[derive(Deserialize)]
struct Layout {
    version: u32,
}

#[derive(Serialize, Deserialize)]
struct Section {
    name: String,
    count: u64,
}

/// A pack the manifest lists, with its listed bytes and length.
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
}

fn words(bytes: &[u8]) -> [u64; 3] {
    [0, 8, 16].map(|at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()))
}

struct Catalog {
    file: File,
    header: Header,
    offsets: Vec<u64>,
    /// Where the pack table starts, after every section's entries.
    table: u64,
    packs: u64,
}

impl Catalog {
    fn open(path: &Path) -> io::Result<Option<Self>> {
        let mut file = match read_file(path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let size = file.metadata()?.len();
        let mut bytes = [0; 8];
        file.read_exact(&mut bytes)?;
        let header_size = u64::from_le_bytes(bytes);
        let mut offset = 8u64
            .checked_add(header_size)
            .filter(|end| *end <= size)
            .ok_or_else(|| invalid("invalid manifest header length"))?;
        let mut bytes = Vec::new();
        (&mut file).take(header_size).read_to_end(&mut bytes)?;
        let layout: Layout = serde_json::from_slice(&bytes)?;
        if layout.version < VERSION {
            // An older layout is never read: it is a cache miss, producers
            // prepare again, and the next writer open removes its files.
            return Ok(None);
        }
        if layout.version != VERSION {
            return Err(invalid("unsupported prepared manifest version"));
        }
        let header: Header = serde_json::from_slice(&bytes)?;
        let mut offsets = Vec::new();
        let mut names = HashSet::new();
        for section in &header.sections {
            if section.name.is_empty() || !names.insert(&section.name) {
                return Err(invalid("empty or repeated manifest section"));
            }
            offsets.push(offset);
            offset = section
                .count
                .checked_mul(ENTRY_BYTES)
                .and_then(|bytes| offset.checked_add(bytes))
                .ok_or_else(|| invalid("manifest range count overflow"))?;
        }
        let table = size
            .checked_sub(offset)
            .filter(|bytes| bytes % PACK_BYTES == 0)
            .ok_or_else(|| invalid("manifest length does not match its ranges"))?;
        Ok(Some(Self {
            file,
            header,
            offsets,
            table: offset,
            packs: table / PACK_BYTES,
        }))
    }

    /// Entries are read at their offsets, never through the handle's cursor, so
    /// a writer and its forks share one catalog.
    fn entry(&self, section: usize, range: u64) -> io::Result<Option<RangeId>> {
        let Some(s) = self.header.sections.get(section) else {
            return Ok(None);
        };
        if range >= s.count {
            return Ok(None);
        }
        let mut bytes = [0; ENTRY_BYTES as usize];
        let offset = self.offsets[section] + range * ENTRY_BYTES;
        if read_at(&self.file, &mut bytes, offset)? != bytes.len() {
            return Err(invalid("truncated prepared manifest"));
        }
        RangeId::decode(&bytes).map(Some)
    }

    /// The pack table: every pack an entry names, with its listed bytes.
    fn packs(&self) -> io::Result<Vec<PackRecord>> {
        let mut bytes = vec![
            0;
            usize::try_from(self.packs * PACK_BYTES)
                .map_err(|_| invalid("pack table too large"))?
        ];
        if read_at(&self.file, &mut bytes, self.table)? != bytes.len() {
            return Err(invalid("truncated prepared pack table"));
        }
        Ok(bytes
            .chunks_exact(PACK_BYTES as usize)
            .map(|record| {
                let [id, live, size] = words(record);
                PackRecord { id, live, size }
            })
            .collect())
    }

    /// Streams `count` entries from `offset`. With a mover, entries in the packs
    /// it empties are rewritten to their moved bytes.
    fn copy(
        &self,
        offset: u64,
        count: u64,
        into: &mut impl Write,
        buffer: &mut [u8],
        mut mover: Option<&mut Mover>,
    ) -> io::Result<()> {
        let size = count
            .checked_mul(ENTRY_BYTES)
            .ok_or_else(|| invalid("manifest range count overflow"))?;
        let mut done = 0;
        while done < size {
            let want = buffer
                .len()
                .min(usize::try_from(size - done).unwrap_or(usize::MAX));
            let read = read_at(&self.file, &mut buffer[..want], offset + done)?;
            if read != want {
                return Err(invalid("truncated prepared prefix"));
            }
            match mover.as_deref_mut() {
                None => into.write_all(&buffer[..read])?,
                Some(mover) => {
                    for entry in buffer[..read].chunks_exact(ENTRY_BYTES as usize) {
                        into.write_all(&mover.place(RangeId::decode(entry)?)?.encode())?;
                    }
                }
            }
            done += read as u64;
        }
        Ok(())
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

    /// Lists `id` at `range`, which is at most the current count.
    fn place(&mut self, range: u64, id: RangeId) {
        if range < self.old_count {
            self.replaced.insert(range, id);
        } else {
            let index = (range - self.old_count) as usize;
            match self.added.get_mut(index) {
                Some(slot) => *slot = id,
                None => self.added.push(id),
            }
        }
    }
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
#[derive(Debug)]
enum SectionRef {
    Parent(usize),
    Named(String),
}

/// One step of a fork, replayed and re-checked against its parent by `adopt`.
#[derive(Debug)]
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

/// One batch of a forked writer's work, for [`Writer::adopt`] into its parent.
/// Dropping it leaves its ranges unlisted; the next writer open removes a pack
/// that holds no listed range.
#[derive(Debug)]
pub struct Staged {
    transaction: Uuid,
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
            incomplete: false,
        }
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
        if self.fork.is_some() {
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
            incomplete: false,
        })
    }

    /// Closes a fork's current batch, even if it is empty; later work forms the
    /// next batch. Does nothing on an ordinary writer.
    pub fn end_batch(&mut self) {
        if let Some(fork) = &mut self.fork {
            let operations = std::mem::take(&mut fork.open);
            fork.batches.push(Staged {
                transaction: self.transaction,
                operations,
                pack: None,
            });
        }
    }

    /// A fork's closed batches in order, then its open batch if it holds work.
    /// Writes out the fork's pack.
    pub fn into_staged(self) -> io::Result<Vec<Staged>> {
        let Writer {
            fork,
            pack,
            transaction,
            ..
        } = self;
        let Some(mut fork) = fork else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "only a forked prepared writer has staged batches",
            ));
        };
        if !fork.open.is_empty() {
            fork.batches.push(Staged {
                transaction,
                operations: fork.open,
                pack: None,
            });
        }
        if let Some(mut pack) = pack {
            let shared = pack.finish()?;
            for batch in &mut fork.batches {
                batch.pack = Some((pack.id, shared.clone()));
            }
        }
        Ok(fork.batches)
    }

    /// Lists a fork's batch in this transaction exactly as if its operations had
    /// been made here, in order: the same section numbers, ranges and byte
    /// counts. Every recorded read and replaced range must still be what the
    /// fork saw; otherwise the batch is refused (`ErrorKind::Other`), and so
    /// is a later commit of this transaction.
    pub fn adopt(&mut self, staged: Staged) -> io::Result<()> {
        if self.fork.is_some() || staged.transaction != self.transaction {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staged prepared ranges belong to another transaction",
            ));
        }
        if let Some((id, shared)) = staged.pack {
            self.packs.insert(id, shared);
        }
        let result = self.replay(staged.operations);
        self.incomplete |= result.is_err();
        result
    }

    fn replay(&mut self, operations: Vec<Operation>) -> io::Result<()> {
        for operation in operations {
            match operation {
                Operation::Create(name) => {
                    if self.position(&name).is_some() {
                        return Err(overlap(&name));
                    }
                    self.sections.push(PendingSection::new(&name));
                }
                Operation::Read {
                    section,
                    range,
                    slot,
                } => {
                    let current = self
                        .resolve(&section)
                        .map_or(Slot::Missing, |index| self.sections[index].slot(range));
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
                    if entry.slot(range) != old
                        || (old == Slot::Missing && range != entry.count()?)
                    {
                        return Err(overlap(&entry.name));
                    }
                    let written = self.bytes_written.checked_add(bytes);
                    let displaced = self.bytes_replaced.checked_add(replaced);
                    let (Some(written), Some(displaced)) = (written, displaced) else {
                        return Err(invalid("prepared byte count overflow"));
                    };
                    self.sections[index].place(range, id);
                    self.bytes_written = written;
                    self.bytes_replaced = displaced;
                }
            }
        }
        Ok(())
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
            pack => pack.insert(Pack::create(&self.directory)?),
        };
        let id = pack.append(data)?;
        Ok(if id.len == 0 { RangeId::EMPTY } else { id })
    }

    /// Opens the transaction's latest range, including unpublished tail pages.
    /// Producers can evict a page buffer and continue it before commit. A fork
    /// reads its own ranges, else its parent's as they stood when forked.
    pub fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<Range>> {
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
        let Some(id) = self.slot_id(index, range, slot)? else {
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

    /// Publishes the manifest last, streaming old entries without loading them.
    /// Each pack the transaction lists is flushed once first; packs under half
    /// live are emptied into one new pack. A post-rename flush error can leave
    /// the complete new manifest visible. A fork publishes only through its
    /// parent's [`Writer::adopt`].
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
        if !matches!(&valid, Ok(true))
            && let Some((_, _, accepted)) = self.key.binding.lock().unwrap().as_mut()
        {
            *accepted = false;
        }
        if !valid? {
            return Err(invalid("sources changed during preparation"));
        }
        let mut fresh = std::mem::take(&mut self.packs);
        if let Some(pack) = &mut self.pack {
            fresh.insert(pack.id, pack.finish()?);
        }
        let mut table = self.pack_table(&fresh)?;
        // Packs under half live are emptied into one new pack, so every pack
        // the manifest lists holds at least as many listed bytes as dead ones.
        let compact: HashSet<u64> = table
            .iter()
            .filter(|(_, (live, size))| u128::from(*live) * 2 < u128::from(*size))
            .map(|(&id, _)| id)
            .collect();
        table.retain(|id, _| !compact.contains(id));
        let moves_published = compact.iter().any(|id| !fresh.contains_key(id));
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
        let header = Header {
            version: VERSION,
            source,
            sections: self
                .sections
                .iter()
                .map(|s| Section {
                    name: s.name.to_string(),
                    count: s.old_count + s.added.len() as u64,
                })
                .collect(),
        };
        publish(&self.directory.join(MANIFEST), |file| {
            let mut out = io::BufWriter::with_capacity(1 << 16, &mut *file);
            let bytes = serde_json::to_vec(&header)?;
            out.write_all(&(bytes.len() as u64).to_le_bytes())?;
            out.write_all(&bytes)?;
            let mut buffer = vec![0; ENTRY_BYTES as usize * 4096];
            for (index, s) in self.sections.iter().enumerate() {
                if s.old_count != 0 {
                    let base = self
                        .base
                        .as_ref()
                        .ok_or_else(|| invalid("missing prefix"))?;
                    let start = base.offsets[index];
                    let mut from = 0;
                    for (&range, id) in &s.replaced {
                        base.copy(
                            start + from * ENTRY_BYTES,
                            range - from,
                            &mut out,
                            &mut buffer,
                            moves_published.then_some(&mut mover),
                        )?;
                        out.write_all(&mover.place(*id)?.encode())?;
                        from = range + 1;
                    }
                    base.copy(
                        start + from * ENTRY_BYTES,
                        s.old_count - from,
                        &mut out,
                        &mut buffer,
                        moves_published.then_some(&mut mover),
                    )?;
                }
                for id in &s.added {
                    out.write_all(&mover.place(*id)?.encode())?;
                }
            }
            // The packs this transaction lists, flushed before the manifest
            // that names them; publication then flushes the directory.
            let mut durable: Vec<_> = table
                .keys()
                .filter_map(|id| fresh.get(id).map(|shared| Arc::clone(&shared.file)))
                .collect();
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
            for record in &records {
                out.write_all(&record.encode())?;
            }
            out.flush()?;
            drop(out);
            sync_packs(&durable)
        })?;
        *self.key.binding.lock().unwrap() = Some((self.directory.clone(), source, true));
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
            table.insert(id, (0, shared.size));
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
    std::thread::scope(|scope| {
        let tasks: Vec<_> = files
            .chunks(files.len().div_ceil(workers))
            .map(|part| scope.spawn(move || part.iter().try_for_each(|file| file.sync_all())))
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
