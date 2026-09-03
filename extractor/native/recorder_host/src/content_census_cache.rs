//! Digest memo for the attach-time content census.
//!
//! Every row the census publishes is a payload digest over a file the install
//! holds. The install does not change between captures, so the digests do not
//! either; what changes is which files exist and what their enumeration facts
//! are. A cache row is therefore keyed by the file's exact local path and
//! reused only when the enumeration reports the same length and modification
//! instant; an archive member is keyed by the archive digest the mount already
//! verified. A row is never adopted on the cache's own word alone: the census
//! re-hashes a bounded random sample of the rows it reuses on every run, and
//! one disagreement refuses the whole cache. The document carries a digest over
//! its rows and is replaced atomically, so a torn or edited file is refused,
//! not partially believed.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use portable_content_manifest::effective_vfs::EffectiveVfsEntryFacts;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CACHE_FILE_NAME: &str = "content_census_cache_v1.json";
pub const SCHEMA: &str = "hoi4.content-census-cache.v1";

/// One memo row. `len`, `modified_unix_secs` and `modified_nanos` are the
/// enumeration facts the row was recorded under (zero for an archive member,
/// whose identity is the archive digest in its key); `payload_bytes` is the
/// length of the bytes `payload_sha256` digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    pub key: String,
    pub len: u64,
    pub modified_unix_secs: i64,
    pub modified_nanos: u32,
    pub payload_bytes: u64,
    pub payload_sha256: String,
}

#[derive(Serialize, Deserialize)]
struct CacheDocument {
    schema: String,
    entries: Vec<CacheEntry>,
    entries_sha256: String,
}

pub enum CacheLoad {
    Absent,
    Refused(String),
    Loaded(BTreeMap<String, CacheEntry>),
}

pub fn cache_path(dir: &Path) -> PathBuf {
    dir.join(CACHE_FILE_NAME)
}

/// The memo key and the enumeration facts a cached row must repeat for the
/// listing entry to reuse it.
pub fn entry_identity(facts: &EffectiveVfsEntryFacts) -> (String, u64, i64, u32) {
    match facts {
        EffectiveVfsEntryFacts::File {
            local_path,
            len,
            modified_unix_secs,
            modified_nanos,
        } => (
            format!("file:{}", local_path.display()),
            *len,
            *modified_unix_secs,
            *modified_nanos,
        ),
        EffectiveVfsEntryFacts::ArchiveMember {
            archive_sha256,
            entry_name,
        } => (format!("zip:{archive_sha256}:{entry_name}"), 0, 0, 0),
    }
}

fn entries_sha256(entries: &[CacheEntry]) -> Result<String, String> {
    let bytes = serde_json::to_vec(entries)
        .map_err(|error| format!("serialize content census cache rows failed: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

pub fn load(dir: &Path) -> CacheLoad {
    let path = cache_path(dir);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return CacheLoad::Absent,
        Err(error) => return CacheLoad::Refused(format!("read {} failed: {error}", path.display())),
    };
    let document: CacheDocument = match serde_json::from_slice(&bytes) {
        Ok(document) => document,
        Err(error) => {
            return CacheLoad::Refused(format!("parse {} failed: {error}", path.display()))
        }
    };
    if document.schema != SCHEMA {
        return CacheLoad::Refused(format!(
            "{} carries schema {:?}, expected {SCHEMA:?}",
            path.display(),
            document.schema
        ));
    }
    match entries_sha256(&document.entries) {
        Ok(actual) if actual == document.entries_sha256 => {}
        Ok(actual) => {
            return CacheLoad::Refused(format!(
                "{} rows hash to {actual}, the document claims {}",
                path.display(),
                document.entries_sha256
            ))
        }
        Err(error) => return CacheLoad::Refused(error),
    }
    let mut entries = BTreeMap::new();
    for entry in document.entries {
        if entry.payload_sha256.len() != 64
            || !entry
                .payload_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return CacheLoad::Refused(format!(
                "{} row {:?} carries a malformed digest",
                path.display(),
                entry.key
            ));
        }
        if entries.insert(entry.key.clone(), entry).is_some() {
            return CacheLoad::Refused(format!("{} repeats a row key", path.display()));
        }
    }
    CacheLoad::Loaded(entries)
}

/// Replace the cache document atomically: written to a sibling temp file,
/// synced, then renamed over the published name.
pub fn store(dir: &Path, entries: &BTreeMap<String, CacheEntry>) -> Result<PathBuf, String> {
    fs::create_dir_all(dir)
        .map_err(|error| format!("create content census cache dir {}: {error}", dir.display()))?;
    let rows: Vec<CacheEntry> = entries.values().cloned().collect();
    let document = CacheDocument {
        schema: SCHEMA.to_string(),
        entries_sha256: entries_sha256(&rows)?,
        entries: rows,
    };
    let bytes = serde_json::to_vec(&document)
        .map_err(|error| format!("serialize content census cache failed: {error}"))?;
    let path = cache_path(dir);
    let temp = dir.join(format!(".{CACHE_FILE_NAME}.{}.tmp", std::process::id()));
    let write = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &path)
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temp);
        return Err(format!(
            "publish content census cache {} failed: {error}",
            path.display()
        ));
    }
    Ok(path)
}
