//! Whole-recording product replies kept on disk between sessions, so reopening a
//! campaign answers its widgets without reading the recording again.
//!
//! A reply is a pure function of the recording, this build, the request, and
//! the game content the recording pins by hash in the user's game language. An
//! entry is keyed by all of them. A reply that read content outside the
//! recorded coverage depends on unverified local files: it is never stored, and
//! nothing is served while such reads are being reported. The folder is
//! disposable: an unreadable entry is a miss, and the least recently used
//! entries go once the folder passes [`LIMIT`].

use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

use crate::{Source, products::LoadContext};

/// The folder's size limit; pruning keeps three quarters of it.
pub const LIMIT: u64 = 1 << 30;
const MAGIC: &[u8; 4] = b"HRC2";
const ENTRY_BUDGET: usize = crate::transport::MAX_REPLY_BYTES;

/// Bytes stored since the folder was last measured, so pruning walks it rarely.
static WRITTEN: AtomicU64 = AtomicU64::new(LIMIT);

/// The history cache folder beside the content hash cache.
pub fn folder(options: &installed_content::ResolveOptions) -> PathBuf {
    options.cache_dir.join("history")
}

pub(crate) struct Entry {
    path: PathBuf,
    key: String,
}

impl Entry {
    /// The entry for one request, or none when its reply cannot be kept.
    pub(crate) fn new(
        source: &Source,
        product: &str,
        content_dependent: bool,
        params: &serde_json::Value,
        context: &LoadContext,
    ) -> Option<Self> {
        let content = context.content.as_ref()?;
        if content_dependent {
            crate::content::check_replay(
                source.replay_ref(),
                content.allow_unverified,
                &mut Vec::new(),
            )
            .ok()?;
            source.content(&content.options).ok()?;
        }
        let settings = installed_content::settings::read_keys(&content.options.user_dir).ok()?;
        let language = settings.language().ok()?;
        let decode = settings.texture_decode();
        let key = format!(
            "{}\n{}\n{product}\n{params}\n{}\n{}\n{}\n{language}\n{decode:?}",
            String::from_utf8_lossy(MAGIC),
            build()?,
            content.options.game_dir.display(),
            content.options.user_dir.display(),
            content.allow_unverified,
        );
        let name = hex(&Sha256::digest(key.as_bytes())[..16]);
        let path = folder(&content.options)
            .join(recording(source))
            .join(format!("{name}.zst"));
        Some(Self { path, key })
    }

    /// The stored reply JSON, or none.
    pub(crate) fn get(&self) -> Option<String> {
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .ok()?
            .take(ENTRY_BUDGET as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() > ENTRY_BUDGET {
            return None;
        }
        let rest = bytes.strip_prefix(MAGIC)?;
        let (length, rest) = rest.split_first_chunk::<4>()?;
        let (key, body) = rest.split_at_checked(u32::from_le_bytes(*length) as usize)?;
        if key != self.key.as_bytes() {
            return None;
        }
        let mut decoder = zstd::stream::read::Decoder::new(body).ok()?;
        decoder.window_log_max(ENTRY_BUDGET.ilog2()).ok()?;
        let mut json = Vec::new();
        decoder
            .take(ENTRY_BUDGET as u64 + 1)
            .read_to_end(&mut json)
            .ok()?;
        if json.len() > ENTRY_BUDGET {
            return None;
        }
        let json = String::from_utf8(json).ok()?;
        // Pruning removes the least recently used entries first.
        let _ = fs::File::options()
            .write(true)
            .open(&self.path)
            .and_then(|file| file.set_modified(SystemTime::now()));
        Some(json)
    }

    /// Store a reply; a failure leaves only a miss.
    pub(crate) fn put(&self, json: &str) {
        if let Ok(stored) = self.store(json)
            && WRITTEN.fetch_add(stored, Ordering::Relaxed) + stored >= LIMIT / 8
        {
            WRITTEN.store(0, Ordering::Relaxed);
            if let Some(root) = self.path.parent().and_then(Path::parent) {
                let _ = prune(root, LIMIT);
            }
        }
    }

    fn store(&self, json: &str) -> io::Result<u64> {
        if json.len() > ENTRY_BUDGET {
            return Err(io::Error::other(
                "history reply exceeds the transport byte budget",
            ));
        }
        let mut bytes = MAGIC.to_vec();
        let length = u32::try_from(self.key.len()).map_err(io::Error::other)?;
        bytes.extend(length.to_le_bytes());
        bytes.extend(self.key.as_bytes());
        zstd::stream::copy_encode(json.as_bytes(), &mut bytes, 3)?;
        if bytes.len() > ENTRY_BUDGET {
            return Err(io::Error::other(
                "history entry exceeds the transport byte budget",
            ));
        }
        let folder = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("no folder"))?;
        fs::create_dir_all(folder)?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary = self.path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&temporary, &bytes)
            .and_then(|()| fs::rename(&temporary, &self.path))
            .inspect_err(|_| {
                let _ = fs::remove_file(&temporary);
            })?;
        Ok(bytes.len() as u64)
    }
}

/// This build: an entry from another build may have computed a reply differently.
fn build() -> Option<&'static str> {
    static BUILD: OnceLock<Option<String>> = OnceLock::new();
    BUILD
        .get_or_init(|| {
            let metadata = std::env::current_exe().ok()?.metadata().ok()?;
            let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
            Some(format!("{}:{}", metadata.len(), modified.as_nanos()))
        })
        .as_deref()
}

/// Held files use the canonical path and opened metadata revision. Synthetic
/// resident recordings retain their byte identity without a recording file read.
fn recording(source: &Source) -> String {
    #[derive(Default)]
    struct Fingerprint(Option<String>);
    let state = source.state::<Fingerprint>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let replay = source.replay_ref();
    state
        .0
        .get_or_insert_with(|| {
            if replay.is_file_backed() {
                let path = std::fs::canonicalize(source.path())
                    .unwrap_or_else(|_| source.path().to_owned());
                let mut hash = blake3::Hasher::new();
                hash.update(path.as_os_str().as_encoded_bytes());
                hash.update(&[0]);
                hash.update(source.key().revision.as_bytes());
                hash.finalize().to_hex().to_string()
            } else {
                blake3::hash(replay.bytes()).to_hex().to_string()
            }
        })
        .clone()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Bytes the folder holds.
pub fn size(root: &Path) -> u64 {
    entries(root).iter().map(|entry| entry.1).sum()
}

/// Remove every entry.
pub fn clear(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Remove the least recently used entries until the folder holds three quarters
/// of `limit`, once it holds more than `limit`.
fn prune(root: &Path, limit: u64) -> io::Result<()> {
    let mut entries = entries(root);
    let mut total: u64 = entries.iter().map(|entry| entry.1).sum();
    if total <= limit {
        return Ok(());
    }
    entries.sort_unstable_by_key(|entry| entry.0);
    for (_, length, path) in entries {
        if total <= limit / 4 * 3 {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            total -= length;
            if let Some(folder) = path.parent() {
                let _ = fs::remove_dir(folder);
            }
        }
    }
    Ok(())
}

fn entries(root: &Path) -> Vec<(SystemTime, u64, PathBuf)> {
    let mut entries = Vec::new();
    for folder in fs::read_dir(root).into_iter().flatten().flatten() {
        for file in fs::read_dir(folder.path()).into_iter().flatten().flatten() {
            if let Ok(metadata) = file.metadata() {
                let used = metadata.modified().unwrap_or(UNIX_EPOCH);
                entries.push((used, metadata.len(), file.path()));
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests;
