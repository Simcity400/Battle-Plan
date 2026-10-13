//! Immutable prepared ranges with an atomically replaced, flat manifest.
#![cfg(windows)]

use replay::publication::{posix_delete, posix_replace, sync_file};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, BufReader, Read, Seek, SeekFrom, Write},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
    path::{Path, PathBuf},
    time::SystemTime,
};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::GENERIC_WRITE,
    Storage::FileSystem::{DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE},
};

const MANIFEST: &str = "manifest";
const LOCK: &str = "writer.lock";
const VERSION: u32 = 1;
const ID_BYTES: u64 = 16;
const SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;

/// Ordered inputs from the recording's resolved content stack, including archives.
/// Only paths, sizes and modification times participate in reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceKey {
    recording: Stamp,
    content: Vec<Stamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

impl Stamp {
    fn capture(path: &Path) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
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

impl SourceKey {
    pub fn capture(recording: &Path, content: &[PathBuf]) -> io::Result<Self> {
        Ok(Self {
            recording: Stamp::capture(recording)?,
            content: content
                .iter()
                .map(|path| Stamp::capture(path))
                .collect::<io::Result<_>>()?,
        })
    }

    fn current(&self) -> io::Result<bool> {
        let paths: Vec<_> = self
            .content
            .iter()
            .map(|stamp| stamp.path.clone())
            .collect();
        Ok(Self::capture(&self.recording.path, &paths)? == *self)
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
        Ok(key.current()? && self.catalog()?.is_some_and(|c| c.header.key == *key))
    }

    /// A range is its zero-based ordinal within a section, owned by the producer.
    /// Reads the header and one fixed-width manifest entry, then opens that file.
    pub fn read(&self, key: &SourceKey, section: &str, range: u64) -> io::Result<Option<File>> {
        if !key.current()? {
            return Ok(None);
        }
        // Cleanup/Clear may remove a name between manifest lookup and file open.
        // Retry against the current manifest; returned handles need no coordination.
        for _ in 0..2 {
            let Some(mut catalog) = self.catalog()? else {
                return Ok(None);
            };
            if catalog.header.key != *key {
                return Ok(None);
            }
            let Some(index) = catalog
                .header
                .sections
                .iter()
                .position(|s| s.name == section)
            else {
                return Ok(None);
            };
            let Some(id) = catalog.id(index, range)? else {
                return Ok(None);
            };
            match read_file(&self.directory.join(chunk_name(index, range, id))) {
                Ok(file) => return Ok(Some(file)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    /// Starts a full replacement. The current manifest remains visible until commit.
    pub fn replace(&self, key: SourceKey) -> io::Result<Writer> {
        let lock = self.lock()?;
        let mut previous = self.catalog()?;
        self.cleanup(previous.as_mut())?;
        Ok(Writer {
            directory: self.directory.clone(),
            _lock: lock,
            key,
            base: None,
            sections: Vec::new(),
        })
    }

    /// The producer must know that the old recording prefix is unchanged.
    /// Metadata alone cannot establish append-only growth. Ordinary reopen never
    /// accepts an unequal key; this explicit operation publishes a new key last.
    pub fn append(&self, previous: &SourceKey, key: SourceKey) -> io::Result<Writer> {
        if key.recording.path != previous.recording.path
            || key.recording.size <= previous.recording.size
            || key.content != previous.content
        {
            return Err(invalid(
                "append requires recording growth and unchanged content",
            ));
        }
        let lock = self.lock()?;
        let mut base = self
            .catalog()?
            .ok_or_else(|| invalid("no prepared prefix"))?;
        if base.header.key != *previous {
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
            })
            .collect();
        Ok(Writer {
            directory: self.directory.clone(),
            _lock: lock,
            key,
            base: Some(base),
            sections,
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
            if name == LOCK || (catalog.is_some() && name == MANIFEST) {
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

#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    key: SourceKey,
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
        let header: Header =
            serde_json::from_reader(BufReader::new((&mut file).take(header_size)))?;
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
}

/// Owns one exclusive-open lock. Dropping it leaves the old manifest intact;
/// unlisted files are removed on the next writer open.
pub struct Writer {
    directory: PathBuf,
    _lock: File,
    key: SourceKey,
    base: Option<Catalog>,
    sections: Vec<PendingSection>,
}

impl Writer {
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
        publish(&self.directory.join(chunk_name(index, range, id)), |file| {
            io::copy(&mut data, file)?;
            Ok(())
        })?;
        s.added.push(id);
        Ok(range)
    }

    /// Publishes the manifest last, streaming old file entries without loading them.
    /// A post-rename flush error can leave the complete new manifest visible.
    pub fn commit(mut self) -> io::Result<()> {
        if !self.key.current()? {
            return Err(invalid("sources changed during preparation"));
        }
        let header = Header {
            version: VERSION,
            key: self.key,
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
                    let size = s.old_count * ID_BYTES;
                    if io::copy(&mut (&mut base.file).take(size), file)? != size {
                        return Err(invalid("truncated prepared prefix"));
                    }
                }
                for id in &s.added {
                    file.write_all(id.as_bytes())?;
                }
            }
            Ok(())
        })
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
    sync_file(&temporary)?;
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
    sync_file(path)
}

fn read_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(SHARING).open(path)
}

fn chunk_name(section: usize, range: u64, id: Uuid) -> String {
    format!("{section}_{range}_{}.bin", id.simple())
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
