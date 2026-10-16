//! Immutable prepared ranges with an atomically replaced, flat manifest.
#![cfg(windows)]

use replay::publication::{posix_delete, posix_replace, sync_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

const MANIFEST: &str = "manifest";
const LOCK: &str = "writer.lock";
const VERSION: u32 = 2;
const ID_BYTES: u64 = 16;
const SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// Ordered inputs from the recording's resolved content stack, including archives.
/// Only paths, sizes and modification times participate in reuse.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceKey {
    recording: Stamp,
    content: Arc<Vec<Stamp>>,
    #[serde(skip)]
    binding: Arc<Mutex<Option<(PathBuf, Uuid, bool)>>>,
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
}

/// File readers are independent of this object and keep their bytes until closed.
pub struct Store {
    directory: PathBuf,
}

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

    /// Reads just the manifest header, never enumerating or opening range files.
    pub fn is_reusable(&self, key: &SourceKey) -> io::Result<bool> {
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
    /// Reads the header and one fixed-width manifest entry, then opens that file.
    pub fn read(&self, key: &SourceKey, section: &str, range: u64) -> io::Result<Option<File>> {
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
    ) -> io::Result<Vec<Option<File>>> {
        Ok(self
            .read_many_counted(key, ranges)?
            .into_iter()
            .map(|(_, file)| file)
            .collect())
    }

    /// The section counts and handles come from the same manifest lookup.
    pub fn read_many_counted(
        &self,
        key: &SourceKey,
        ranges: &[(&str, u64)],
    ) -> io::Result<Vec<(u64, Option<File>)>> {
        let empty = || ranges.iter().map(|_| (0, None)).collect();
        if ranges.is_empty() {
            return Ok(empty());
        }
        let Some(mut catalog) = self.catalog()? else {
            return Ok(empty());
        };
        if !self.matches(&catalog, key)? {
            return Ok(empty());
        }
        ranges
            .iter()
            .map(
                |&(section, range)| match self.read_catalog(&mut catalog, section, range) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.read_named_counted(key, section, range)
                    }
                    result => result,
                },
            )
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
    ) -> io::Result<Option<File>> {
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

    fn read_named(&self, key: &SourceKey, section: &str, range: u64) -> io::Result<Option<File>> {
        Ok(self.read_named_counted(key, section, range)?.1)
    }

    fn read_named_counted(
        &self,
        key: &SourceKey,
        section: &str,
        range: u64,
    ) -> io::Result<(u64, Option<File>)> {
        // Cleanup/Clear may remove a name between manifest lookup and file open.
        // Retry against the current manifest; returned handles need no coordination.
        for _ in 0..2 {
            let Some(mut catalog) = self.catalog()? else {
                return Ok((0, None));
            };
            if !self.matches_bound(&catalog, key)? {
                return Ok((0, None));
            }
            match self.read_catalog(&mut catalog, section, range) {
                Ok(file) => return Ok(file),
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
        catalog: &mut Catalog,
        section: &str,
        range: u64,
    ) -> io::Result<(u64, Option<File>)> {
        let Some(index) = catalog
            .header
            .sections
            .iter()
            .position(|s| s.name == section)
        else {
            return Ok((0, None));
        };
        let count = catalog.header.sections[index].count;
        let Some(id) = catalog.id(index, range)? else {
            return Ok((count, None));
        };
        read_file(&self.directory.join(chunk_name(index, range, id)))
            .map(|file| (count, Some(file)))
    }

    /// Starts a full replacement. The current manifest remains visible until commit.
    pub fn replace(&self, key: SourceKey) -> io::Result<Writer> {
        let lock = self.lock()?;
        let mut previous = self.catalog()?;
        self.cleanup(previous.as_mut())?;
        let source = match previous.as_ref() {
            Some(catalog) if self.matches_bound(catalog, &key)? => Some(catalog.header.source),
            _ => None,
        };
        Ok(Writer {
            directory: self.directory.clone(),
            _lock: lock,
            key,
            source,
            base: None,
            sections: Vec::new(),
            bytes_written: 0,
            bytes_replaced: 0,
        })
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
        let mut base = self
            .catalog()?
            .ok_or_else(|| invalid("no prepared prefix"))?;
        if !self.matches_bound(&base, previous)? {
            return Err(invalid("prepared prefix key changed"));
        }
        self.cleanup(Some(&mut base))?;
        let sections = base
            .header
            .sections
            .iter()
            .map(|s| PendingSection {
                name: s.name.clone(),
                old_count: s.count,
                added: Vec::new(),
                replaced: BTreeMap::new(),
            })
            .collect();
        let source = (previous == &key).then_some(base.header.source);
        Ok(Writer {
            directory: self.directory.clone(),
            _lock: lock,
            key,
            source,
            base: Some(base),
            sections,
            bytes_written: 0,
            bytes_replaced: 0,
        })
    }

    /// Removes saved names while held file readers keep their copy. The empty
    /// directory and zero-byte lock file remain so writers cannot split ownership.
    pub fn clear(&self) -> io::Result<()> {
        let _lock = self.lock()?;
        // Removing the manifest first prevents new readers seeing deleted ranges.
        match posix_delete(&self.directory.join(MANIFEST)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            result => result?,
        }
        self.cleanup(None)
    }

    fn catalog(&self) -> io::Result<Option<Catalog>> {
        #[cfg(test)]
        tests::MANIFEST_READS.set(tests::MANIFEST_READS.get() + 1);
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

    fn cleanup(&self, mut catalog: Option<&mut Catalog>) -> io::Result<()> {
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            if name == LOCK
                || catalog.as_ref().is_some_and(|catalog| {
                    name == MANIFEST || name == source_name(catalog.header.source).as_str()
                })
            {
                continue;
            }
            let keep = match (catalog.as_deref_mut(), name.to_str().and_then(parse_chunk)) {
                (Some(c), Some((section, range, id))) => c.id(section, range)? == Some(id),
                _ => false,
            };
            if !keep {
                posix_delete(&entry.path())?;
            }
        }
        Ok(())
    }
}

fn prefix_inputs(previous: &SourceKey, current: &SourceKey) -> io::Result<bool> {
    Ok(current.recording.path == previous.recording.path
        && current.recording.size >= previous.recording.size
        && current.content == previous.content
        && current.recording_current()?)
}

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    source: Uuid,
    sections: Vec<Section>,
}

#[derive(Serialize, Deserialize)]
struct Section {
    name: String,
    count: u64,
}

struct Catalog {
    file: File,
    header: Header,
    offsets: Vec<u64>,
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
        let header: Header = serde_json::from_slice(&bytes)?;
        if header.version != VERSION {
            return Err(invalid("unsupported prepared manifest version"));
        }
        let mut offsets = Vec::new();
        let mut names = std::collections::HashSet::new();
        for section in &header.sections {
            if section.name.is_empty() || !names.insert(&section.name) {
                return Err(invalid("empty or repeated manifest section"));
            }
            offsets.push(offset);
            offset = section
                .count
                .checked_mul(ID_BYTES)
                .and_then(|bytes| offset.checked_add(bytes))
                .ok_or_else(|| invalid("manifest range count overflow"))?;
        }
        if offset != size {
            return Err(invalid("manifest length does not match its ranges"));
        }
        Ok(Some(Self {
            file,
            header,
            offsets,
        }))
    }

    fn id(&mut self, section: usize, range: u64) -> io::Result<Option<Uuid>> {
        let Some(s) = self.header.sections.get(section) else {
            return Ok(None);
        };
        if range >= s.count {
            return Ok(None);
        }
        self.file
            .seek(SeekFrom::Start(self.offsets[section] + range * ID_BYTES))?;
        let mut bytes = [0; ID_BYTES as usize];
        self.file.read_exact(&mut bytes)?;
        Ok(Some(Uuid::from_bytes(bytes)))
    }
}

struct PendingSection {
    name: String,
    old_count: u64,
    added: Vec<Uuid>,
    replaced: BTreeMap<u64, Uuid>,
}

/// Owns one exclusive-open lock. Dropping it leaves the old manifest intact;
/// unlisted files are removed on the next writer open.
pub struct Writer {
    directory: PathBuf,
    _lock: File,
    key: SourceKey,
    source: Option<Uuid>,
    base: Option<Catalog>,
    sections: Vec<PendingSection>,
    bytes_written: u64,
    bytes_replaced: u64,
}

impl Writer {
    /// New range payload bytes successfully written by this transaction.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Change in published range payload bytes, excluding displaced files.
    pub fn payload_bytes_change(&self) -> i128 {
        i128::from(self.bytes_written) - i128::from(self.bytes_replaced)
    }

    /// Opens the transaction's latest range, including unpublished tail pages.
    /// Producers can evict a page buffer and continue it before commit.
    pub fn read_at(&mut self, section: &str, range: u64) -> io::Result<Option<File>> {
        let Some(index) = self.sections.iter().position(|entry| entry.name == section) else {
            return Ok(None);
        };
        let entry = &self.sections[index];
        let id = if range < entry.old_count {
            match entry.replaced.get(&range).copied() {
                Some(id) => Some(id),
                None => self
                    .base
                    .as_mut()
                    .ok_or_else(|| invalid("missing prefix"))?
                    .id(index, range)?,
            }
        } else {
            usize::try_from(range - entry.old_count)
                .ok()
                .and_then(|index| entry.added.get(index))
                .copied()
        };
        id.map(|id| read_file(&self.directory.join(chunk_name(index, range, id))))
            .transpose()
    }

    /// Replaces one logical range with a new whole file, or appends the next.
    /// The old file and manifest remain untouched until manifest publication.
    pub fn write_at(&mut self, section: &str, range: u64, mut data: impl Read) -> io::Result<()> {
        let Some(index) = self.sections.iter().position(|entry| entry.name == section) else {
            if range != 0 {
                return Err(invalid(&format!(
                    "non-consecutive prepared range {range} in {section}"
                )));
            }
            self.write(section, data)?;
            return Ok(());
        };
        let entry = &self.sections[index];
        let count = entry
            .old_count
            .checked_add(entry.added.len() as u64)
            .ok_or_else(|| invalid("range count overflow"))?;
        if range == count {
            self.write(section, data)?;
            return Ok(());
        }
        if range > count {
            return Err(invalid(&format!(
                "non-consecutive prepared range {range} in {section}"
            )));
        }
        let old = if range < entry.old_count {
            match entry.replaced.get(&range).copied() {
                Some(id) => id,
                None => self
                    .base
                    .as_mut()
                    .ok_or_else(|| invalid("missing prefix"))?
                    .id(index, range)?
                    .ok_or_else(|| invalid("missing prepared range"))?,
            }
        } else {
            entry.added[(range - entry.old_count) as usize]
        };
        let old_bytes = fs::metadata(self.directory.join(chunk_name(index, range, old)))?.len();
        let replaced = self
            .bytes_replaced
            .checked_add(old_bytes)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        let id = Uuid::new_v4();
        let mut bytes = 0;
        publish(&self.directory.join(chunk_name(index, range, id)), |file| {
            bytes = io::copy(&mut data, file)?;
            Ok(())
        })?;
        self.bytes_written = self
            .bytes_written
            .checked_add(bytes)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        self.bytes_replaced = replaced;
        let entry = &mut self.sections[index];
        if range < entry.old_count {
            entry.replaced.insert(range, id);
        } else {
            entry.added[(range - entry.old_count) as usize] = id;
        }
        Ok(())
    }

    /// Streams a whole, immutable range; no existing data file is modified.
    /// Ranges are consecutive per section, with boundaries defined by its producer.
    pub fn write(&mut self, section: &str, mut data: impl Read) -> io::Result<u64> {
        if section.is_empty() {
            return Err(invalid("empty section name"));
        }
        let index = match self.sections.iter().position(|s| s.name == section) {
            Some(index) => index,
            None => {
                self.sections.push(PendingSection {
                    name: section.to_owned(),
                    old_count: 0,
                    added: Vec::new(),
                    replaced: BTreeMap::new(),
                });
                self.sections.len() - 1
            }
        };
        let s = &mut self.sections[index];
        let range = s
            .old_count
            .checked_add(s.added.len() as u64)
            .ok_or_else(|| invalid("range count overflow"))?;
        let id = Uuid::new_v4();
        let mut bytes = 0;
        publish(&self.directory.join(chunk_name(index, range, id)), |file| {
            bytes = io::copy(&mut data, file)?;
            Ok(())
        })?;
        self.bytes_written = self
            .bytes_written
            .checked_add(bytes)
            .ok_or_else(|| invalid("prepared byte count overflow"))?;
        s.added.push(id);
        Ok(range)
    }

    /// Publishes the manifest last, streaming old file entries without loading them.
    /// A post-rename flush error can leave the complete new manifest visible.
    pub fn commit(mut self) -> io::Result<()> {
        let valid = self.key.current();
        if !matches!(&valid, Ok(true))
            && let Some((_, _, accepted)) = self.key.binding.lock().unwrap().as_mut()
        {
            *accepted = false;
        }
        if !valid? {
            return Err(invalid("sources changed during preparation"));
        }
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
                    name: s.name.clone(),
                    count: s.old_count + s.added.len() as u64,
                })
                .collect(),
        };
        publish(&self.directory.join(MANIFEST), |file| {
            let bytes = serde_json::to_vec(&header)?;
            file.write_all(&(bytes.len() as u64).to_le_bytes())?;
            file.write_all(&bytes)?;
            for (index, s) in self.sections.iter().enumerate() {
                if s.old_count != 0 {
                    let base = self
                        .base
                        .as_mut()
                        .ok_or_else(|| invalid("missing prefix"))?;
                    base.file.seek(SeekFrom::Start(base.offsets[index]))?;
                    let mut from = 0;
                    for (&range, id) in &s.replaced {
                        let size = (range - from) * ID_BYTES;
                        if io::copy(&mut (&mut base.file).take(size), file)? != size {
                            return Err(invalid("truncated prepared prefix"));
                        }
                        base.file.seek(SeekFrom::Current(ID_BYTES as i64))?;
                        file.write_all(id.as_bytes())?;
                        from = range + 1;
                    }
                    let size = (s.old_count - from) * ID_BYTES;
                    if io::copy(&mut (&mut base.file).take(size), file)? != size {
                        return Err(invalid("truncated prepared prefix"));
                    }
                }
                for id in &s.added {
                    file.write_all(id.as_bytes())?;
                }
            }
            Ok(())
        })?;
        *self.key.binding.lock().unwrap() = Some((self.directory, source, true));
        Ok(())
    }
}

fn publish(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .access_mode(DELETE | GENERIC_WRITE)
        .share_mode(SHARING)
        .create_new(true)
        .open(&temporary)?;
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
/// prepared pipeline tests, which publish thousands of files.
fn durable(path: &Path) -> io::Result<()> {
    if cfg!(feature = "test-support") {
        Ok(())
    } else {
        sync_file(path)
    }
}

fn read_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(SHARING).open(path)
}

fn chunk_name(section: usize, range: u64, id: Uuid) -> String {
    format!("{section}_{range}_{}.bin", id.simple())
}

fn source_name(id: Uuid) -> String {
    format!("source_{}.json", id.simple())
}

fn parse_chunk(name: &str) -> Option<(usize, u64, Uuid)> {
    let mut parts = name.strip_suffix(".bin")?.split('_');
    let result = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        Uuid::parse_str(parts.next()?).ok()?,
    );
    (parts.next().is_none() && chunk_name(result.0, result.1, result.2) == name).then_some(result)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests;
