//! Content-keyed fold cache: one certified fold per (spool digest, fold-source
//! fingerprint, fold parameters), validated by digests on every adoption.
//!
//! Trust boundary: the receipt self-digest and the artifact digests detect
//! corruption and torn writes, not a hostile writer with access to the cache
//! root. The recorder therefore only PUBLISHES into the cache; the sealed
//! container is always built from the recorder's own fold. Adoption serves
//! tools and gate tests that re-decode a recording they already trust.

use std::fs;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use compact_contract::stream_validator::StreamIdentityMode;
use recorder_host::hour_coverage::HourCoverageMap;
use recorder_host::stream_fold::ExcludedDomainDataRecord;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::CertifiedReplayArtifacts;

pub const SCHEMA: &str = "hoi4.fold-cache-entry.v2";
pub const RECEIPT_FILE_NAME: &str = "FOLD_CACHE.json";
const STAGING_DIR: &str = ".staging";
const CACHE_STALE_AFTER_SECS: u64 = 24 * 60 * 60;
const STAGING_STALE_AFTER_SECS: u64 = 60 * 60;
const COPY_CHUNK: usize = 1 << 20;
/// Distinguishes staging directories created by one process within a second.
static STAGING_SEQ: AtomicU64 = AtomicU64::new(0);

pub const FOLD_SOURCE_FINGERPRINT: &str = env!("FOLD_SOURCE_FINGERPRINT");

pub fn identity_mode_name(mode: StreamIdentityMode) -> &'static str {
    match mode {
        StreamIdentityMode::IdentityV1 => "identity_v1",
        StreamIdentityMode::Legacy => "legacy",
    }
}

fn index_identity_mode_name(mode: snapshot::CacheIdentityMode) -> &'static str {
    match mode {
        snapshot::CacheIdentityMode::IdentityV1 => "identity_v1",
        snapshot::CacheIdentityMode::Legacy => "legacy",
    }
}

/// The hour index names the game id and identity mode it was folded under;
/// a receipt or a set of parameters that says otherwise is refused.
fn check_index_identity(
    index: &snapshot::ReplayHourIndex,
    game_id: &str,
    identity_mode: &str,
    checkpoint_interval_hours: u32,
) -> Result<(), String> {
    if index.checkpoint_interval_hours() != checkpoint_interval_hours {
        return Err(format!(
            "hour index checkpoints every {} hours, the receipt or parameters say {checkpoint_interval_hours}",
            index.checkpoint_interval_hours()
        ));
    }
    if index.game_id() != game_id {
        return Err(format!(
            "hour index was folded for game id {:?}, the receipt or parameters say {game_id:?}",
            index.game_id()
        ));
    }
    let index_mode = index_identity_mode_name(index.identity_mode());
    if index_mode != identity_mode {
        return Err(format!(
            "hour index was folded in identity mode {index_mode:?}, the receipt or parameters say {identity_mode:?}"
        ));
    }
    Ok(())
}

/// Everything besides the spool bytes and the fold source that shapes the
/// artifacts. Two folds of one spool with different parameters are different
/// cache entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoldCacheParams {
    pub game_id: String,
    pub identity_mode: StreamIdentityMode,
    pub checkpoint_interval_hours: u32,
}

impl FoldCacheParams {
    pub fn new(
        game_id: &str,
        identity_mode: StreamIdentityMode,
        checkpoint_interval_hours: u32,
    ) -> Self {
        Self {
            game_id: game_id.to_string(),
            identity_mode,
            checkpoint_interval_hours,
        }
    }

    fn key(&self) -> String {
        key_from_parts(
            &self.game_id,
            identity_mode_name(self.identity_mode),
            self.checkpoint_interval_hours,
        )
    }

    fn matches(&self, receipt: &FoldCacheReceipt) -> Result<(), String> {
        if receipt.game_id != self.game_id {
            return Err(format!(
                "receipt is for game id {:?}, this fold is {:?}",
                receipt.game_id, self.game_id
            ));
        }
        let mode = identity_mode_name(self.identity_mode);
        if receipt.identity_mode != mode {
            return Err(format!(
                "receipt was folded in identity mode {:?}, this fold is {mode:?}",
                receipt.identity_mode
            ));
        }
        if receipt.checkpoint_interval_hours != self.checkpoint_interval_hours {
            return Err(format!(
                "receipt checkpoints every {} hours, this fold every {}",
                receipt.checkpoint_interval_hours, self.checkpoint_interval_hours
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FoldCacheReceipt {
    pub schema: String,
    pub fold_source_fingerprint: String,
    pub source_len: u64,
    pub source_sha256: String,
    pub game_id: String,
    pub identity_mode: String,
    pub created_unix: u64,
    pub producer_exe_sha256: Option<String>,
    pub hour_count: u64,
    pub checkpoint_count: u64,
    pub checkpoint_interval_hours: u32,
    pub max_seek_deltas: u32,
    pub hour_index_len: u64,
    pub hour_index_sha256: String,
    pub checkpoint_cache_len: u64,
    pub checkpoint_cache_sha256: String,
    pub published_coverage_len: u64,
    pub published_coverage_sha256: String,
    pub legacy_json_bytes_avoided: u64,
    pub production_render_blob_count: u64,
    pub production_render_blob_bytes: u64,
    /// Every render blob by file name, length and digest, in name order.
    pub production_render_blobs: Vec<BlobReceipt>,
    pub excluded_domain_data_records: Vec<ExcludedDomainDataRecord>,
    pub hour_coverage: serde_json::Value,
    pub campaign_identity_manifest: Option<serde_json::Value>,
    pub durable_decoded_end_offset: u64,
    /// SHA-256 of this receipt serialized with this field empty.
    pub receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobReceipt {
    pub name: String,
    pub len: u64,
    pub sha256: String,
}

impl FoldCacheReceipt {
    fn self_digest(&self) -> Result<String, String> {
        let mut unsigned = self.clone();
        unsigned.receipt_sha256 = String::new();
        let bytes = serde_json::to_vec(&unsigned)
            .map_err(|error| format!("serialize fold cache receipt: {error}"))?;
        Ok(format!("{:x}", Sha256::digest(&bytes)))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FoldCacheOutcome {
    pub mode: String,
    pub cause: Option<String>,
    pub entry_dir: Option<String>,
    pub fold_source_fingerprint: String,
    pub validate_ms: u64,
    pub copy_ms: u64,
}

pub fn entry_dir(cache_root: &Path, source_sha256: &str, params: &FoldCacheParams) -> PathBuf {
    cache_root.join(entry_home(source_sha256, FOLD_SOURCE_FINGERPRINT, &params.key()))
}

/// The entry path relative to the cache root: `<spool sha>/<fingerprint>-<key>`.
fn entry_home(source_sha256: &str, fingerprint: &str, key: &str) -> String {
    format!("{source_sha256}/{fingerprint}-{key}")
}

/// The parameter key as a receipt records its parts, so a receipt alone
/// names the entry path its directory belongs at.
fn key_from_parts(game_id: &str, identity_mode: &str, checkpoint_interval_hours: u32) -> String {
    let mut hasher = Sha256::new();
    hasher.update(game_id.as_bytes());
    hasher.update([0u8]);
    hasher.update(identity_mode.as_bytes());
    hasher.update([0u8]);
    hasher.update(checkpoint_interval_hours.to_le_bytes());
    let digest = format!("{:x}", hasher.finalize());
    digest[..24].to_string()
}

fn receipt_home(receipt: &FoldCacheReceipt) -> String {
    entry_home(
        &receipt.source_sha256,
        &receipt.fold_source_fingerprint,
        &key_from_parts(
            &receipt.game_id,
            &receipt.identity_mode,
            receipt.checkpoint_interval_hours,
        ),
    )
}

fn tick(bytes: usize) {
    recorder_host::finalize_progress::tick(bytes as u64);
}

fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; COPY_CHUNK];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
        tick(read);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

/// Copies `from` to `to` (created or truncated), hashing the bytes on the way;
/// returns the length and digest of what was written.
fn copy_file(from: &Path, to: &Path) -> Result<(u64, String), String> {
    let target =
        fs::File::create(to).map_err(|error| format!("create {}: {error}", to.display()))?;
    copy_into(from, target, to)
}

/// Creates `to` exclusively: a file already there (put by another actor after
/// the caller looked) is refused, never truncated.
fn create_exclusive(to: &Path) -> Result<fs::File, String> {
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(|error| format!("create {} exclusively: {error}", to.display()))
}

fn copy_into(from: &Path, mut target: fs::File, to: &Path) -> Result<(u64, String), String> {
    let mut source =
        fs::File::open(from).map_err(|error| format!("open {}: {error}", from.display()))?;
    let mut buffer = vec![0u8; COPY_CHUNK];
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        let read = source
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", from.display()))?;
        if read == 0 {
            break;
        }
        target
            .write_all(&buffer[..read])
            .map_err(|error| format!("write {}: {error}", to.display()))?;
        hasher.update(&buffer[..read]);
        total += read as u64;
        tick(read);
    }
    target
        .sync_all()
        .map_err(|error| format!("sync {}: {error}", to.display()))?;
    Ok((total, format!("{:x}", hasher.finalize())))
}

/// Render blobs of a directory as receipts, in name order, hashing each file.
fn blob_receipts(dir: &Path) -> Result<Vec<BlobReceipt>, String> {
    let mut blobs = Vec::new();
    if !dir.is_dir() {
        return Ok(blobs);
    }
    for item in fs::read_dir(dir).map_err(|error| format!("list {}: {error}", dir.display()))? {
        let item = item.map_err(|error| format!("list {}: {error}", dir.display()))?;
        let name = item.file_name().to_string_lossy().into_owned();
        let (len, sha256) = hash_file(&item.path())
            .map_err(|error| format!("hash blob {}: {error}", item.path().display()))?;
        // Render blobs are content-addressed (`<sha256 of the file bytes>.base64`);
        // a blob whose bytes no longer hash to its name is refused rather than
        // blessed by a receipt.
        if name != format!("{sha256}.base64") {
            return Err(format!(
                "render blob {} hashes to {sha256}, which is not its content address",
                item.path().display()
            ));
        }
        blobs.push(BlobReceipt { name, len, sha256 });
    }
    blobs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(blobs)
}

fn check_blob_receipts(actual: &[BlobReceipt], receipt: &FoldCacheReceipt) -> Result<(), String> {
    if actual != receipt.production_render_blobs.as_slice() {
        return Err(format!(
            "render blobs ({} files) do not match the receipt ({} files) by name, length and digest",
            actual.len(),
            receipt.production_render_blobs.len()
        ));
    }
    let bytes: u64 = actual.iter().map(|blob| blob.len).sum();
    if actual.len() as u64 != receipt.production_render_blob_count
        || bytes != receipt.production_render_blob_bytes
    {
        return Err(format!(
            "render blobs are {} files / {bytes} bytes, the receipt says {} / {}",
            actual.len(),
            receipt.production_render_blob_count,
            receipt.production_render_blob_bytes
        ));
    }
    Ok(())
}

/// Removes exactly the paths a failed adoption created (files first, then the
/// directories it made, newest first) so the spill is empty for the fold that
/// replaces it; anything else in the spill is left alone. Returns the paths
/// that could not be removed.
fn remove_created(created: &[PathBuf]) -> Vec<String> {
    let mut failed = Vec::new();
    for path in created.iter().rev() {
        let result = if path.is_dir() {
            fs::remove_dir(path)
        } else {
            fs::remove_file(path)
        };
        if let Err(error) = result {
            if error.kind() != std::io::ErrorKind::NotFound {
                failed.push(format!("{}: {error}", path.display()));
            }
        }
    }
    failed
}

/// `create_dir_all` that records every directory it actually creates.
fn create_dirs_tracked(dir: &Path, created: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut missing = Vec::new();
    let mut probe = dir.to_path_buf();
    while !probe.exists() {
        missing.push(probe.clone());
        match probe.parent() {
            Some(parent) => probe = parent.to_path_buf(),
            None => break,
        }
    }
    for path in missing.iter().rev() {
        fs::create_dir(path).map_err(|error| format!("create {}: {error}", path.display()))?;
        created.push(path.clone());
    }
    Ok(())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn read_receipt(entry: &Path) -> Result<Option<FoldCacheReceipt>, String> {
    let receipt_path = entry.join(RECEIPT_FILE_NAME);
    let bytes = match fs::read(&receipt_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {}: {error}", receipt_path.display())),
    };
    let receipt: FoldCacheReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse {}: {error}", receipt_path.display()))?;
    if receipt.schema != SCHEMA {
        return Err(format!(
            "{} carries schema {:?}, expected {SCHEMA:?}",
            receipt_path.display(),
            receipt.schema
        ));
    }
    let digest = receipt.self_digest()?;
    if digest != receipt.receipt_sha256 {
        return Err(format!(
            "{} hashes to {digest}, it claims {}",
            receipt_path.display(),
            receipt.receipt_sha256
        ));
    }
    Ok(Some(receipt))
}

fn check_receipt(
    receipt: &FoldCacheReceipt,
    source_len: u64,
    source_sha256: &str,
    params: &FoldCacheParams,
) -> Result<(), String> {
    if receipt.fold_source_fingerprint != FOLD_SOURCE_FINGERPRINT {
        return Err(format!(
            "receipt was folded by source fingerprint {}, this binary is {}",
            receipt.fold_source_fingerprint, FOLD_SOURCE_FINGERPRINT
        ));
    }
    if receipt.source_len != source_len || receipt.source_sha256 != source_sha256 {
        return Err(format!(
            "receipt describes spool {}:{}, not {source_len}:{source_sha256}",
            receipt.source_len, receipt.source_sha256
        ));
    }
    params.matches(receipt)
}

struct VerifiedEntry {
    index_len: u64,
    index_sha256: String,
    cache_len: u64,
    cache_sha256: String,
}

/// Hashes every file of an entry against its receipt and parses the hour
/// index, checking that it describes the receipt and the source.
fn verify_entry(
    entry: &Path,
    receipt: &FoldCacheReceipt,
    source_len: u64,
    source_sha256: &str,
) -> Result<VerifiedEntry, String> {
    let (index_len, index_sha256, cache_len, cache_sha256) = verify_artifact_files(entry, receipt)?;
    let index_bytes = fs::read(entry.join(snapshot::HOUR_INDEX_FILE))
        .map_err(|error| format!("read cached index: {error}"))?;
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes)
        .map_err(|error| format!("cached hour index does not parse: {error}"))?;
    if index.source_len() != source_len
        || index.source_sha256() != source_sha256
        || index.cache_len() != cache_len
        || index.cache_sha256() != cache_sha256
        || index.hours().len() as u64 != receipt.hour_count
    {
        return Err(
            "cached hour index disagrees with the receipt or the cached cache bytes".into(),
        );
    }
    check_index_identity(
        &index,
        &receipt.game_id,
        &receipt.identity_mode,
        receipt.checkpoint_interval_hours,
    )?;
    check_blob_receipts(&blob_receipts(&entry.join("production_render_blobs"))?, receipt)?;
    Ok(VerifiedEntry {
        index_len,
        index_sha256,
        cache_len,
        cache_sha256,
    })
}

fn verify_artifact_files(
    entry: &Path,
    receipt: &FoldCacheReceipt,
) -> Result<(u64, String, u64, String), String> {
    let (index_len, index_sha256) = hash_file(&entry.join(snapshot::HOUR_INDEX_FILE))
        .map_err(|error| format!("hash cached hour index: {error}"))?;
    if index_len != receipt.hour_index_len || index_sha256 != receipt.hour_index_sha256 {
        return Err(format!(
            "cached hour index is {index_len} bytes sha256 {index_sha256}, the receipt says {} bytes sha256 {}",
            receipt.hour_index_len, receipt.hour_index_sha256
        ));
    }
    let (cache_len, cache_sha256) = hash_file(&entry.join(snapshot::CHECKPOINT_CACHE_FILE))
        .map_err(|error| format!("hash cached checkpoint cache: {error}"))?;
    if cache_len != receipt.checkpoint_cache_len || cache_sha256 != receipt.checkpoint_cache_sha256
    {
        return Err(format!(
            "cached checkpoint cache is {cache_len} bytes sha256 {cache_sha256}, the receipt says {} bytes sha256 {}",
            receipt.checkpoint_cache_len, receipt.checkpoint_cache_sha256
        ));
    }
    let (coverage_len, coverage_sha256) =
        hash_file(&entry.join(snapshot::PUBLISHED_COVERAGE_FILE))
            .map_err(|error| format!("hash cached published coverage: {error}"))?;
    if coverage_len != receipt.published_coverage_len
        || coverage_sha256 != receipt.published_coverage_sha256
    {
        return Err(format!(
            "cached published coverage is {coverage_len} bytes sha256 {coverage_sha256}, the receipt says {} bytes sha256 {}",
            receipt.published_coverage_len, receipt.published_coverage_sha256
        ));
    }
    Ok((index_len, index_sha256, cache_len, cache_sha256))
}

/// Copies a validated entry into `spill_dir` and returns the artifacts, or
/// `Ok(None)` when the cache holds no entry for this spool and fold.
pub fn adopt(
    cache_root: &Path,
    source_len: u64,
    source_sha256: &str,
    params: &FoldCacheParams,
    spill_dir: &Path,
) -> Result<Option<CertifiedReplayArtifacts>, String> {
    Lifecycle::new(cache_root, &now_unix, &self_verdict).sweep_staging();
    let entry = entry_dir(cache_root, source_sha256, params);
    let Some(receipt) = read_receipt(&entry)? else {
        return Ok(None);
    };
    check_receipt(&receipt, source_len, source_sha256, params)?;
    let VerifiedEntry {
        index_len,
        index_sha256,
        cache_len,
        cache_sha256,
    } = verify_entry(&entry, &receipt, source_len, source_sha256)?;
    let hour_coverage = HourCoverageMap::from_json(&receipt.hour_coverage)
        .map_err(|error| format!("cached hour coverage does not parse: {error}"))?;

    if fs::read_dir(spill_dir)
        .map_err(|error| format!("inspect spill {}: {error}", spill_dir.display()))?
        .next()
        .is_some()
    {
        return Err(format!(
            "fold cache adoption needs an empty spill directory: {}",
            spill_dir.display()
        ));
    }
    let index_path = spill_dir.join(snapshot::HOUR_INDEX_FILE);
    let cache_path = spill_dir.join(snapshot::CHECKPOINT_CACHE_FILE);
    let coverage_path = spill_dir.join(snapshot::PUBLISHED_COVERAGE_FILE);
    let blob_dir = spill_dir.join("production_render_blobs");
    let mut created: Vec<PathBuf> = Vec::new();
    let copied = (|| -> Result<(u64, u64), String> {
        if let Some(parent) = index_path.parent() {
            create_dirs_tracked(parent, &mut created)?;
        }
        for (from, to, len, sha256) in [
            (
                entry.join(snapshot::HOUR_INDEX_FILE),
                &index_path,
                receipt.hour_index_len,
                &receipt.hour_index_sha256,
            ),
            (
                entry.join(snapshot::CHECKPOINT_CACHE_FILE),
                &cache_path,
                receipt.checkpoint_cache_len,
                &receipt.checkpoint_cache_sha256,
            ),
            (
                entry.join(snapshot::PUBLISHED_COVERAGE_FILE),
                &coverage_path,
                receipt.published_coverage_len,
                &receipt.published_coverage_sha256,
            ),
        ] {
            let target = create_exclusive(to)?;
            created.push(to.clone());
            let (copied_len, copied_sha256) = copy_into(&from, target, to)?;
            if copied_len != len || copied_sha256 != *sha256 {
                return Err(format!(
                    "{} changed while being copied: {copied_len} bytes sha256 {copied_sha256}, the receipt says {len} bytes sha256 {sha256}",
                    from.display()
                ));
            }
        }
        let mut blob_count = 0u64;
        let mut blob_bytes = 0u64;
        if !receipt.production_render_blobs.is_empty() {
            create_dirs_tracked(&blob_dir, &mut created)?;
            let cached_blobs = entry.join("production_render_blobs");
            for blob in &receipt.production_render_blobs {
                let to = blob_dir.join(&blob.name);
                let target = create_exclusive(&to)?;
                created.push(to.clone());
                let (copied_len, copied_sha256) =
                    copy_into(&cached_blobs.join(&blob.name), target, &to)?;
                if copied_len != blob.len || copied_sha256 != blob.sha256 {
                    return Err(format!(
                        "render blob {} changed while being copied: {copied_len} bytes sha256 {copied_sha256}, the receipt says {} bytes sha256 {}",
                        blob.name, blob.len, blob.sha256
                    ));
                }
                blob_count += 1;
                blob_bytes += copied_len;
            }
        }
        Ok((blob_count, blob_bytes))
    })();
    let (blob_count, blob_bytes) = match copied {
        Ok(counts) => counts,
        Err(error) => {
            let failed = remove_created(&created);
            if failed.is_empty() {
                return Err(error);
            }
            return Err(format!(
                "{error}; the spill could not be restored, remove these before folding: {}",
                failed.join(", ")
            ));
        }
    };
    Ok(Some(CertifiedReplayArtifacts {
        hour_index_path: index_path,
        checkpoint_cache_path: cache_path,
        hour_count: receipt.hour_count,
        checkpoint_count: receipt.checkpoint_count,
        checkpoint_interval_hours: receipt.checkpoint_interval_hours,
        max_seek_deltas: receipt.max_seek_deltas,
        source_len,
        source_sha256: source_sha256.to_string(),
        hour_index_len: index_len,
        hour_index_sha256: index_sha256,
        checkpoint_cache_len: cache_len,
        checkpoint_cache_sha256: cache_sha256,
        legacy_json_bytes_avoided: receipt.legacy_json_bytes_avoided,
        production_render_blob_dir: blob_dir,
        production_render_blob_count: blob_count,
        production_render_blob_bytes: blob_bytes,
        excluded_domain_data_records: receipt.excluded_domain_data_records,
        hour_coverage,
        campaign_identity_manifest: receipt.campaign_identity_manifest,
        durable_decoded_end_offset: receipt.durable_decoded_end_offset,
        published_coverage_path: coverage_path,
        live_fold: None,
        live_seal_refusal: None,
    }))
}

// ---- entry lifecycle -------------------------------------------------------
//
// Every directory the cache owns is in exactly one state, named by where it
// lives, and leaves it only by an atomic rename made after a verdict on its
// contents:
//
//   Staged     .staging/<pid>-<seq>-<unix>        one publisher's private copy
//                                                 (exclusively created)
//   Published  <spool>/<fingerprint>-<key>        reached only by
//                                                 rename(Staged -> entry) after
//                                                 the staged copy is accepted
//   Aside      .staging/<pid>-<seq>-aside-<unix>  reached only by
//                                                 rename(path -> aside), a
//                                                 claim; settled by a verdict:
//                                                 restored to its home, kept,
//                                                 or removed
//   Removed                                       reached only from Aside, or
//                                                 from a publisher's own Staged
//                                                 copy
//
// A claim is one rename and nothing else. The home of a directory is derived
// from its own receipt (spool, fingerprint, parameters), never from a side
// record, so there is no state between "published" and "claimed" and no clock
// decides where a verifying directory belongs. Nothing at a published path is
// deleted in place, and no removal is decided by a timestamp alone: the
// claimed contents are verified first.

/// What a verifier says about a directory that may be an entry.
pub(crate) enum Verdict {
    /// A verifying entry: when it was made and, relative to the cache root,
    /// the entry path its receipt says it belongs at.
    Valid { created_unix: u64, home: String },
    Corrupt(String),
}

pub(crate) struct Lifecycle<'a> {
    cache_root: &'a Path,
    /// Read at every naming and staleness decision, never captured.
    clock: &'a dyn Fn() -> u64,
    /// The self-consistency verdict used for every settle decision.
    verify: &'a dyn Fn(&Path) -> Verdict,
}

/// A publisher's private copy: only its creator, or a sweep once it is stale
/// (through a claim), touches it.
pub(crate) struct Staged {
    dir: PathBuf,
}

impl Staged {
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }

    /// Drops a copy that was never published.
    pub(crate) fn discard(self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A claimed directory, moved out of its path by one rename. `origin` is the
/// path the claimant took it from, known only within that claimant; a sweep
/// derives the home from the contents instead.
pub(crate) struct Aside {
    dir: PathBuf,
    origin: Option<PathBuf>,
}

pub(crate) enum Settled {
    Restored,
    Kept,
    Removed,
}

fn remove_path(path: &Path) {
    let _ = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
}

/// A home is a non-empty relative path of plain components: no root, no
/// prefix, no `..`, so it cannot name anything outside the cache.
fn home_is_inside(home: &str) -> bool {
    !home.is_empty()
        && Path::new(home)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
}

impl<'a> Lifecycle<'a> {
    pub(crate) fn new(
        cache_root: &'a Path,
        clock: &'a dyn Fn() -> u64,
        verify: &'a dyn Fn(&Path) -> Verdict,
    ) -> Self {
        Self {
            cache_root,
            clock,
            verify,
        }
    }

    fn staging_root(&self) -> Result<PathBuf, String> {
        let root = self.cache_root.join(STAGING_DIR);
        fs::create_dir_all(&root).map_err(|error| format!("create {}: {error}", root.display()))?;
        Ok(root)
    }

    /// `<pid>-<seq><kind>-<unix>`: unique within a process by the sequence,
    /// across processes by the pid; the unix second, read now, stays the
    /// last field.
    fn unique_name(&self, kind: &str) -> String {
        format!(
            "{}-{}{kind}-{}",
            std::process::id(),
            STAGING_SEQ.fetch_add(1, Ordering::Relaxed),
            (self.clock)()
        )
    }

    /// -> Staged: an exclusively created directory with its replay_decoder
    /// subdirectory; a name collision is an error, never a shared directory.
    pub(crate) fn stage(&self) -> Result<Staged, String> {
        let dir = self.staging_root()?.join(self.unique_name(""));
        fs::create_dir(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
        fs::create_dir(dir.join("replay_decoder"))
            .map_err(|error| format!("create {}: {error}", dir.display()))?;
        Ok(Staged { dir })
    }

    /// -> Aside for whatever is at `path`: one rename. Ok(None) when nothing
    /// is there; an error when something is there that cannot be moved (it
    /// stays put).
    pub(crate) fn claim(&self, path: &Path) -> Result<Option<Aside>, String> {
        let aside = self.staging_root()?.join(self.unique_name("-aside"));
        match fs::rename(path, &aside) {
            Ok(()) => Ok(Some(Aside {
                dir: aside,
                origin: Some(path.to_path_buf()),
            })),
            Err(error) => {
                if fs::symlink_metadata(path).is_err() {
                    Ok(None)
                } else {
                    Err(format!("claim {}: {error}", path.display()))
                }
            }
        }
    }

    /// The path under this cache root that a verdict's home names, when it
    /// names one inside the cache.
    fn home_path(&self, home: &str) -> Option<PathBuf> {
        home_is_inside(home).then(|| self.cache_root.join(home))
    }

    /// Aside -> Published (back where it belongs, when `restore` and that
    /// path is free) / Kept / Removed, by the verdict on the aside's contents:
    /// a verifying aside younger than the cache's age-out is never removed.
    /// It goes back to the claimant's origin when known, else to the home its
    /// own receipt names.
    pub(crate) fn settle(&self, aside: Aside, restore: bool) -> Settled {
        match (self.verify)(&aside.dir) {
            Verdict::Valid { created_unix, home }
                if (self.clock)().saturating_sub(created_unix) <= CACHE_STALE_AFTER_SECS =>
            {
                if restore {
                    let target = aside.origin.clone().or_else(|| self.home_path(&home));
                    if let Some(target) = target {
                        if let Some(parent) = target.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        if fs::symlink_metadata(&target).is_err()
                            && fs::rename(&aside.dir, &target).is_ok()
                        {
                            return Settled::Restored;
                        }
                    }
                }
                Settled::Kept
            }
            Verdict::Valid { .. } | Verdict::Corrupt(_) => {
                remove_path(&aside.dir);
                Settled::Removed
            }
        }
    }

    /// Staged -> Published: the staged copy must be accepted, then one rename
    /// into the entry path. Something already there is kept when accepted;
    /// otherwise it is claimed, and an accepted entry that appeared between
    /// the look and the claim is put back and reported instead of this copy,
    /// while anything else is settled without restoring (a self-consistent
    /// entry of another binary is kept aside until it ages out).
    pub(crate) fn install(
        &self,
        staged: Staged,
        entry: &Path,
        accept: &dyn Fn(&Path) -> Verdict,
    ) -> Result<PathBuf, String> {
        if let Verdict::Corrupt(reason) = accept(&staged.dir) {
            staged.discard();
            return Err(format!("staged entry does not verify, not published: {reason}"));
        }
        if let Some(parent) = entry.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                staged.discard();
                return Err(format!("create {}: {error}", parent.display()));
            }
        }
        let mut last_error = String::new();
        for _attempt in 0..3 {
            match fs::rename(&staged.dir, entry) {
                Ok(()) => {
                    self.sweep_siblings(entry);
                    return Ok(entry.to_path_buf());
                }
                Err(error) => last_error = error.to_string(),
            }
            if let Verdict::Valid { .. } = accept(entry) {
                staged.discard();
                return Ok(entry.to_path_buf());
            }
            let aside = match self.claim(entry) {
                Ok(Some(aside)) => aside,
                Ok(None) => continue,
                Err(error) => {
                    staged.discard();
                    return Err(error);
                }
            };
            if let Verdict::Valid { .. } = accept(&aside.dir) {
                if fs::symlink_metadata(entry).is_err() && fs::rename(&aside.dir, entry).is_ok() {
                    staged.discard();
                    return Ok(entry.to_path_buf());
                }
                // Its path was re-occupied meanwhile: the aside stays (it ages
                // out or a sweep restores it) and the next attempt looks at
                // the entry path again.
                continue;
            }
            self.settle(aside, false);
        }
        staged.discard();
        Err(format!(
            "publish fold cache entry {}: {last_error}",
            entry.display()
        ))
    }

    /// Sweeps .staging: a stale (over an hour by its name's last field)
    /// Staged directory is claimed and settled, a stale Aside is settled;
    /// both with restoration, so a verifying one goes back to the home its
    /// receipt names (a finished copy whose publisher died is published by
    /// the sweep) or is kept until it ages out. Non-stale items are not
    /// touched.
    pub(crate) fn sweep_staging(&self) {
        let Ok(children) = fs::read_dir(self.cache_root.join(STAGING_DIR)) else {
            return;
        };
        let now = (self.clock)();
        for child in children.flatten() {
            let path = child.path();
            let Some(name) = child.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let made = name
                .rsplit('-')
                .next()
                .and_then(|unix| unix.parse::<u64>().ok());
            let stale = made.map_or(true, |made| {
                now.saturating_sub(made) > STAGING_STALE_AFTER_SECS
            });
            if !stale {
                continue;
            }
            let aside = if name.contains("-aside-") {
                Aside {
                    dir: path,
                    origin: None,
                }
            } else {
                match self.claim(&path) {
                    Ok(Some(mut aside)) => {
                        aside.origin = None;
                        aside
                    }
                    _ => continue,
                }
            };
            self.settle(aside, true);
        }
    }

    /// Sweeps the siblings of `entry` (other parameter keys of one spool): a
    /// sibling whose receipt says it is older than the cache's age-out is
    /// claimed and settled, so it is removed only if the claimed contents
    /// still say so (one replaced meanwhile is restored). A sibling without a
    /// readable receipt belongs to a binary that replaces it itself.
    pub(crate) fn sweep_siblings(&self, entry: &Path) {
        let Some(spool_dir) = entry.parent() else {
            return;
        };
        let Ok(siblings) = fs::read_dir(spool_dir) else {
            return;
        };
        let now = (self.clock)();
        for sibling in siblings.flatten() {
            let path = sibling.path();
            if path == entry || !path.is_dir() {
                continue;
            }
            let aged = fs::read(path.join(RECEIPT_FILE_NAME))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .and_then(|receipt| receipt.get("created_unix")?.as_u64())
                .map(|created| now.saturating_sub(created) > CACHE_STALE_AFTER_SECS)
                .unwrap_or(false);
            if !aged {
                continue;
            }
            if let Ok(Some(aside)) = self.claim(&path) {
                self.settle(aside, true);
            }
        }
    }
}

/// The self-consistency verdict: the receipt reads back and every file, the
/// parsed index and the blob receipts agree with it and with the spool it
/// names. Enough to decide that a directory must not be destroyed, and where
/// it belongs.
fn self_verdict(dir: &Path) -> Verdict {
    let receipt = match read_receipt(dir) {
        Ok(Some(receipt)) => receipt,
        Ok(None) => return Verdict::Corrupt(format!("{} has no receipt", dir.display())),
        Err(reason) => return Verdict::Corrupt(reason),
    };
    match verify_entry(dir, &receipt, receipt.source_len, &receipt.source_sha256) {
        Ok(_) => Verdict::Valid {
            created_unix: receipt.created_unix,
            home: receipt_home(&receipt),
        },
        Err(reason) => Verdict::Corrupt(reason),
    }
}

/// The publisher's verdict: self-consistent and describing these artifacts'
/// spool under these parameters, the checks adoption applies.
fn accept_verdict(
    dir: &Path,
    artifacts: &CertifiedReplayArtifacts,
    params: &FoldCacheParams,
) -> Verdict {
    if fs::symlink_metadata(dir).is_err() {
        return Verdict::Corrupt(format!("{} is absent", dir.display()));
    }
    let receipt = match read_receipt(dir) {
        Ok(Some(receipt)) => receipt,
        Ok(None) => return Verdict::Corrupt(format!("{} has no receipt", dir.display())),
        Err(reason) => return Verdict::Corrupt(reason),
    };
    if let Err(reason) = check_receipt(&receipt, artifacts.source_len, &artifacts.source_sha256, params) {
        return Verdict::Corrupt(reason);
    }
    match verify_entry(dir, &receipt, artifacts.source_len, &artifacts.source_sha256) {
        Ok(_) => Verdict::Valid {
            created_unix: receipt.created_unix,
            home: receipt_home(&receipt),
        },
        Err(reason) => Verdict::Corrupt(reason),
    }
}

/// Publishes `artifacts` as the entry for their spool under `params`. Copies
/// into a staging directory first and renames it into place, so a reader never
/// sees a half-written entry. An existing entry is kept.
pub fn publish(
    cache_root: &Path,
    params: &FoldCacheParams,
    artifacts: &CertifiedReplayArtifacts,
    producer_exe_sha256: Option<String>,
) -> Result<PathBuf, String> {
    let lifecycle = Lifecycle::new(cache_root, &now_unix, &self_verdict);
    lifecycle.sweep_staging();
    if artifacts.checkpoint_interval_hours != params.checkpoint_interval_hours {
        return Err(format!(
            "artifacts checkpoint every {} hours but the cache parameters say {}",
            artifacts.checkpoint_interval_hours, params.checkpoint_interval_hours
        ));
    }
    let index_bytes = fs::read(&artifacts.hour_index_path)
        .map_err(|error| format!("read hour index to publish: {error}"))?;
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes)
        .map_err(|error| format!("hour index to publish does not parse: {error}"))?;
    check_index_identity(
        &index,
        &params.game_id,
        identity_mode_name(params.identity_mode),
        params.checkpoint_interval_hours,
    )?;
    let entry = entry_dir(cache_root, &artifacts.source_sha256, params);
    let accept = |dir: &Path| accept_verdict(dir, artifacts, params);
    if let Verdict::Valid { .. } = accept(&entry) {
        return Ok(entry);
    }
    let staged = lifecycle.stage()?;
    let staging = staged.dir().to_path_buf();
    let result = (|| -> Result<(), String> {
        for (from, to, expected) in [
            (
                &artifacts.hour_index_path,
                staging.join(snapshot::HOUR_INDEX_FILE),
                Some((artifacts.hour_index_len, &artifacts.hour_index_sha256)),
            ),
            (
                &artifacts.checkpoint_cache_path,
                staging.join(snapshot::CHECKPOINT_CACHE_FILE),
                Some((
                    artifacts.checkpoint_cache_len,
                    &artifacts.checkpoint_cache_sha256,
                )),
            ),
            (
                &artifacts.published_coverage_path,
                staging.join(snapshot::PUBLISHED_COVERAGE_FILE),
                None,
            ),
        ] {
            let (copied_len, copied_sha256) = copy_file(from, &to)?;
            if let Some((len, sha256)) = expected {
                if copied_len != len || copied_sha256 != *sha256 {
                    return Err(format!(
                        "{} is {copied_len} bytes sha256 {copied_sha256} but the certified artifacts say {len} bytes sha256 {sha256}; not published",
                        from.display()
                    ));
                }
            }
        }
        if artifacts.production_render_blob_dir.is_dir() {
            let blobs = staging.join("production_render_blobs");
            fs::create_dir_all(&blobs)
                .map_err(|error| format!("create {}: {error}", blobs.display()))?;
            for item in fs::read_dir(&artifacts.production_render_blob_dir)
                .map_err(|error| format!("list render blobs: {error}"))?
            {
                let item = item.map_err(|error| format!("list render blobs: {error}"))?;
                copy_file(&item.path(), &blobs.join(item.file_name()))?;
            }
        }
        let (published_coverage_len, published_coverage_sha256) =
            hash_file(&staging.join(snapshot::PUBLISHED_COVERAGE_FILE))
                .map_err(|error| format!("hash staged published coverage: {error}"))?;
        let production_render_blobs = blob_receipts(&staging.join("production_render_blobs"))?;
        let blob_bytes: u64 = production_render_blobs.iter().map(|blob| blob.len).sum();
        if production_render_blobs.len() as u64 != artifacts.production_render_blob_count
            || blob_bytes != artifacts.production_render_blob_bytes
        {
            return Err(format!(
                "staged render blobs are {} files / {blob_bytes} bytes, the artifacts say {} / {}",
                production_render_blobs.len(),
                artifacts.production_render_blob_count,
                artifacts.production_render_blob_bytes
            ));
        }
        let mut receipt = FoldCacheReceipt {
            schema: SCHEMA.to_string(),
            fold_source_fingerprint: FOLD_SOURCE_FINGERPRINT.to_string(),
            source_len: artifacts.source_len,
            source_sha256: artifacts.source_sha256.clone(),
            game_id: params.game_id.clone(),
            identity_mode: identity_mode_name(params.identity_mode).to_string(),
            created_unix: now_unix(),
            producer_exe_sha256,
            hour_count: artifacts.hour_count,
            checkpoint_count: artifacts.checkpoint_count,
            checkpoint_interval_hours: artifacts.checkpoint_interval_hours,
            max_seek_deltas: artifacts.max_seek_deltas,
            hour_index_len: artifacts.hour_index_len,
            hour_index_sha256: artifacts.hour_index_sha256.clone(),
            checkpoint_cache_len: artifacts.checkpoint_cache_len,
            checkpoint_cache_sha256: artifacts.checkpoint_cache_sha256.clone(),
            published_coverage_len,
            published_coverage_sha256,
            legacy_json_bytes_avoided: artifacts.legacy_json_bytes_avoided,
            production_render_blob_count: artifacts.production_render_blob_count,
            production_render_blob_bytes: artifacts.production_render_blob_bytes,
            production_render_blobs,
            excluded_domain_data_records: artifacts.excluded_domain_data_records.clone(),
            hour_coverage: artifacts.hour_coverage.to_json(),
            campaign_identity_manifest: artifacts.campaign_identity_manifest.clone(),
            durable_decoded_end_offset: artifacts.durable_decoded_end_offset,
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = receipt.self_digest()?;
        let bytes = serde_json::to_vec_pretty(&receipt)
            .map_err(|error| format!("serialize fold cache receipt: {error}"))?;
        let receipt_path = staging.join(RECEIPT_FILE_NAME);
        let mut file = fs::File::create(&receipt_path)
            .map_err(|error| format!("create {}: {error}", receipt_path.display()))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("write {}: {error}", receipt_path.display()))?;
        Ok(())
    })();
    if let Err(error) = result {
        staged.discard();
        return Err(error);
    }
    // The staged copy is accepted (receipt read back, every file against it)
    // inside install, before the rename into place.
    lifecycle.install(staged, &entry, &accept)
}

/// Opens the cached fold in place (no copy) for tools that read hours.
pub fn open_reader(
    cache_root: &Path,
    source_len: u64,
    source_sha256: &str,
    params: &FoldCacheParams,
) -> Result<Option<(snapshot::CertifiedReplayCache, Vec<i64>)>, String> {
    let entry = entry_dir(cache_root, source_sha256, params);
    let Some(receipt) = read_receipt(&entry)? else {
        return Ok(None);
    };
    check_receipt(&receipt, source_len, source_sha256, params)?;
    verify_entry(&entry, &receipt, source_len, source_sha256)?;
    let index_bytes = fs::read(entry.join(snapshot::HOUR_INDEX_FILE))
        .map_err(|error| format!("read cached hour index: {error}"))?;
    let cache_path = entry.join(snapshot::CHECKPOINT_CACHE_FILE);
    let file =
        fs::File::open(&cache_path).map_err(|error| format!("open cached cache: {error}"))?;
    let reader =
        snapshot::CertifiedReplayCache::open_file(&index_bytes, file, source_len, source_sha256)
            .map_err(|error| format!("admit cached fold: {error}"))?;
    let hours = reader
        .index()
        .hours()
        .iter()
        .map(|hour| hour.hour_index())
        .collect();
    Ok(Some((reader, hours)))
}

#[cfg(test)]
mod lifecycle_tests {
    //! One test per lifecycle transition. Each carries a `kills:` line naming
    //! the production change that makes it fail; the verifiers are fakes that
    //! read a VERDICT file ("valid <unix> [home]", "foreign <unix> [home]" =
    //! self-consistent but another binary's, anything else = corrupt; home
    //! defaults to spool/key), so the tests exercise the rename/claim/settle
    //! mechanics, not the artifact hashing.
    use super::*;
    use std::cell::RefCell;

    const NOW: u64 = 10_000_000;

    fn fixed_now() -> u64 {
        NOW
    }

    struct Sandbox {
        root: PathBuf,
    }

    impl Sandbox {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "fold_cache_lifecycle_{tag}_{}_{}",
                std::process::id(),
                STAGING_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn entry(&self) -> PathBuf {
            self.root.join("spool").join("key")
        }

        fn staging(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.root.join(STAGING_DIR))
                .map(|children| {
                    children
                        .flatten()
                        .map(|child| child.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn write_dir(dir: &Path, verdict: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("VERDICT"), verdict).unwrap();
    }

    fn content(dir: &Path) -> String {
        fs::read_to_string(dir.join("VERDICT")).unwrap_or_default()
    }

    fn verdict_of(dir: &Path, accepting: bool) -> Verdict {
        let Ok(text) = fs::read_to_string(dir.join("VERDICT")) else {
            return Verdict::Corrupt(format!("no VERDICT at {}", dir.display()));
        };
        let mut words = text.split_whitespace();
        let kind = words.next().unwrap_or("");
        let created_unix = words.next().and_then(|word| word.parse().ok()).unwrap_or(NOW);
        let home = words.next().unwrap_or("spool/key").to_string();
        match kind {
            "valid" => Verdict::Valid { created_unix, home },
            "foreign" if !accepting => Verdict::Valid { created_unix, home },
            _ => Verdict::Corrupt(format!("{kind} at {}", dir.display())),
        }
    }

    fn self_fake(dir: &Path) -> Verdict {
        verdict_of(dir, false)
    }

    fn accept_fake(dir: &Path) -> Verdict {
        verdict_of(dir, true)
    }

    fn staged_with(lifecycle: &Lifecycle, verdict: &str) -> Staged {
        let staged = lifecycle.stage().unwrap();
        fs::write(staged.dir().join("VERDICT"), verdict).unwrap();
        staged
    }

    fn write_aside(sandbox: &Sandbox, made: u64, verdict: &str) -> PathBuf {
        let aside = sandbox
            .root
            .join(STAGING_DIR)
            .join(format!("1-{}-aside-{made}", STAGING_SEQ.fetch_add(1, Ordering::Relaxed)));
        write_dir(&aside, verdict);
        aside
    }

    #[test]
    fn stage_makes_a_distinct_exclusive_directory_each_time() {
        // kills: unique_name: replacing STAGING_SEQ.fetch_add with a constant
        // (the second create_dir then fails with AlreadyExists).
        let sandbox = Sandbox::new("stage");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let first = lifecycle.stage().unwrap();
        let second = lifecycle.stage().unwrap();
        assert_ne!(first.dir(), second.dir());
        assert!(first.dir().join("replay_decoder").is_dir());
        assert!(second.dir().join("replay_decoder").is_dir());
        assert_eq!(sandbox.staging().len(), 2);
    }

    #[test]
    fn install_refuses_a_staged_copy_that_is_not_accepted() {
        // kills: install: dropping the accept(&staged.dir) check (the corrupt
        // copy is then renamed into the entry path).
        let sandbox = Sandbox::new("refuse");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let staged = staged_with(&lifecycle, "corrupt");
        let error = lifecycle.install(staged, &sandbox.entry(), &accept_fake).unwrap_err();
        assert!(error.contains("not published"), "{error}");
        assert!(fs::symlink_metadata(sandbox.entry()).is_err());
        assert!(sandbox.staging().is_empty());
    }

    #[test]
    fn install_publishes_an_accepted_copy_and_ages_out_siblings_through_a_claim() {
        // kills: install: dropping self.sweep_siblings(entry) after the rename
        // (the aged sibling then remains).
        let sandbox = Sandbox::new("publish");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let aged = sandbox.root.join("spool").join("aged");
        write_dir(&aged, &format!("valid {} spool/aged", NOW - CACHE_STALE_AFTER_SECS - 1));
        fs::write(
            aged.join(RECEIPT_FILE_NAME),
            format!("{{\"created_unix\": {}}}", NOW - CACHE_STALE_AFTER_SECS - 1),
        )
        .unwrap();
        let young = sandbox.root.join("spool").join("young");
        write_dir(&young, &format!("valid {} spool/young", NOW - 5));
        fs::write(
            young.join(RECEIPT_FILE_NAME),
            format!("{{\"created_unix\": {}}}", NOW - 5),
        )
        .unwrap();
        let staged = staged_with(&lifecycle, "valid 1");
        let published = lifecycle.install(staged, &sandbox.entry(), &accept_fake).unwrap();
        assert_eq!(published, sandbox.entry());
        assert_eq!(content(&sandbox.entry()), "valid 1");
        assert!(fs::symlink_metadata(&aged).is_err(), "aged sibling removed");
        assert_eq!(content(&young), format!("valid {} spool/young", NOW - 5));
        assert!(sandbox.staging().is_empty());
    }

    #[test]
    fn install_keeps_an_accepted_entry_already_published() {
        // kills: install: dropping the accept(entry) Valid arm after a failed
        // rename (the entry is then claimed, so the accept verdict is asked
        // about an aside path).
        let sandbox = Sandbox::new("keep");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        write_dir(&sandbox.entry(), "valid 5");
        let asked = RefCell::new(Vec::<PathBuf>::new());
        let accept = |dir: &Path| {
            asked.borrow_mut().push(dir.to_path_buf());
            accept_fake(dir)
        };
        let staged = staged_with(&lifecycle, "valid 6");
        let published = lifecycle.install(staged, &sandbox.entry(), &accept).unwrap();
        assert_eq!(published, sandbox.entry());
        assert_eq!(content(&sandbox.entry()), "valid 5");
        assert!(sandbox.staging().is_empty());
        let asked = asked.borrow();
        assert!(
            asked.iter().all(|path| !path.to_string_lossy().contains("-aside-")),
            "the published entry was never claimed: {asked:?}"
        );
    }

    #[test]
    fn install_claims_and_replaces_a_corrupt_entry() {
        // kills: install: dropping self.settle(aside, false) after the claim
        // (the corrupt aside then remains under .staging).
        let sandbox = Sandbox::new("replace");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        write_dir(&sandbox.entry(), "corrupt");
        let staged = staged_with(&lifecycle, "valid 7");
        lifecycle.install(staged, &sandbox.entry(), &accept_fake).unwrap();
        assert_eq!(content(&sandbox.entry()), "valid 7");
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
    }

    #[test]
    fn install_restores_an_accepted_entry_that_appeared_between_the_look_and_the_claim() {
        // kills: install: dropping the accept(&aside.dir) Valid arm (the other
        // publisher's entry is then kept aside and this copy installed, so the
        // entry reads "valid 7" instead of "valid 9").
        let sandbox = Sandbox::new("race");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        write_dir(&sandbox.entry(), "corrupt");
        let entry = sandbox.entry();
        let replaced = RefCell::new(false);
        let accept = |dir: &Path| {
            let verdict = accept_fake(dir);
            if dir == entry && !*replaced.borrow() {
                // The other publisher installs its accepted entry right after
                // this one looked and found the corrupt one.
                *replaced.borrow_mut() = true;
                write_dir(&entry, "valid 9");
            }
            verdict
        };
        let staged = staged_with(&lifecycle, "valid 7");
        let published = lifecycle.install(staged, &entry, &accept).unwrap();
        assert_eq!(published, entry);
        assert_eq!(content(&entry), "valid 9");
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
    }

    #[test]
    fn install_keeps_a_self_consistent_foreign_entry_aside_instead_of_removing_it() {
        // kills: settle: the young-Valid arm falling through to the removal
        // block (remove_path on the aside) instead of returning Kept.
        let sandbox = Sandbox::new("foreign");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let foreign = format!("foreign {}", NOW - 100);
        write_dir(&sandbox.entry(), &foreign);
        let staged = staged_with(&lifecycle, "valid 11");
        lifecycle.install(staged, &sandbox.entry(), &accept_fake).unwrap();
        assert_eq!(content(&sandbox.entry()), "valid 11");
        let asides: Vec<String> = sandbox
            .staging()
            .into_iter()
            .filter(|name| name.contains("-aside-"))
            .collect();
        assert_eq!(asides.len(), 1, "{:?}", sandbox.staging());
        assert_eq!(
            content(&sandbox.root.join(STAGING_DIR).join(&asides[0])),
            foreign
        );
    }

    #[test]
    fn claim_names_its_aside_with_the_time_of_the_claim_not_the_lifecycle_start() {
        // kills: Lifecycle::new capturing (clock)() once and unique_name using
        // that instead of reading the clock (the aside is then named two hours
        // stale, and the sweep at NOW settles it: corrupt, removed).
        let sandbox = Sandbox::new("claim_time");
        let calls = RefCell::new(0u32);
        let clock = || {
            *calls.borrow_mut() += 1;
            if *calls.borrow() == 1 {
                NOW - 2 * STAGING_STALE_AFTER_SECS
            } else {
                NOW
            }
        };
        let lifecycle = Lifecycle::new(&sandbox.root, &clock, &self_fake);
        assert_eq!(
            clock(),
            NOW - 2 * STAGING_STALE_AFTER_SECS,
            "the lifecycle started two hours ago"
        );
        write_dir(&sandbox.entry(), "corrupt");
        let aside = lifecycle.claim(&sandbox.entry()).unwrap().unwrap();
        let name = aside.dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(&format!("-{NOW}")), "{name}");
        Lifecycle::new(&sandbox.root, &fixed_now, &self_fake).sweep_staging();
        assert_eq!(content(&aside.dir), "corrupt", "a fresh aside survives a sweep");
    }

    #[test]
    fn sweep_restores_a_stale_aside_to_the_home_its_contents_name() {
        // kills: sweep_staging: settling an aside with restore = false (the
        // aside is then kept and the home stays absent).
        let sandbox = Sandbox::new("restore");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let young = format!("valid {} spool/key", NOW - 100);
        write_aside(&sandbox, NOW - STAGING_STALE_AFTER_SECS - 1, &young);
        lifecycle.sweep_staging();
        assert_eq!(content(&sandbox.entry()), young);
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
    }

    #[test]
    fn sweep_keeps_a_stale_aside_whose_home_is_occupied_until_it_ages_out() {
        // kills: settle: dropping the created_unix age guard (the aged aside is
        // then kept too).
        let sandbox = Sandbox::new("occupied");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        write_dir(&sandbox.entry(), "valid 1");
        let young = write_aside(
            &sandbox,
            NOW - STAGING_STALE_AFTER_SECS - 1,
            &format!("valid {}", NOW - 100),
        );
        let aged = write_aside(
            &sandbox,
            NOW - STAGING_STALE_AFTER_SECS - 1,
            &format!("valid {}", NOW - CACHE_STALE_AFTER_SECS - 1),
        );
        lifecycle.sweep_staging();
        assert_eq!(content(&sandbox.entry()), "valid 1");
        assert_eq!(content(&young), format!("valid {}", NOW - 100));
        assert!(fs::symlink_metadata(&aged).is_err(), "aged aside removed");
    }

    #[test]
    fn sweep_removes_only_a_stale_aside_that_does_not_verify() {
        // kills: sweep_staging: dropping the staleness check (the fresh aside
        // is then removed too).
        let sandbox = Sandbox::new("corrupt_aside");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let stale = write_aside(&sandbox, NOW - STAGING_STALE_AFTER_SECS - 1, "corrupt");
        let fresh = write_aside(&sandbox, NOW - 10, "corrupt");
        lifecycle.sweep_staging();
        assert!(fs::symlink_metadata(&stale).is_err());
        assert_eq!(content(&fresh), "corrupt");
    }

    #[test]
    fn sweep_never_restores_an_aside_outside_the_cache_root() {
        // kills: home_path: dropping the home_is_inside check (the aside is
        // then renamed to <root>/../escaped).
        let sandbox = Sandbox::new("escape");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let young = format!("valid {} ../escaped", NOW - 100);
        let aside = write_aside(&sandbox, NOW - STAGING_STALE_AFTER_SECS - 1, &young);
        let escaped = sandbox.root.join("..").join("escaped");
        lifecycle.sweep_staging();
        assert_eq!(content(&aside), young, "the aside is kept, not moved");
        assert!(fs::symlink_metadata(&escaped).is_err());
    }

    #[test]
    fn sweep_claims_a_stale_staged_copy_before_deciding_on_it() {
        // kills: sweep_staging: removing a stale staged directory in place
        // instead of through claim + settle (the verdict is then never asked
        // about an aside path).
        let sandbox = Sandbox::new("staged_sweep");
        let asked = RefCell::new(Vec::<PathBuf>::new());
        let verify = |dir: &Path| {
            asked.borrow_mut().push(dir.to_path_buf());
            self_fake(dir)
        };
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &verify);
        let stale = sandbox
            .root
            .join(STAGING_DIR)
            .join(format!("1-0-{}", NOW - STAGING_STALE_AFTER_SECS - 1));
        write_dir(&stale, "corrupt");
        lifecycle.sweep_staging();
        assert!(fs::symlink_metadata(&stale).is_err());
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
        let asked = asked.borrow();
        assert!(
            asked.iter().all(|path| path.to_string_lossy().contains("-aside-")),
            "{asked:?}"
        );
        assert!(!asked.is_empty());
    }

    #[test]
    fn sweep_publishes_a_finished_staged_copy_whose_publisher_died() {
        // kills: sweep_staging: clearing the claimed aside's home by settling
        // with restore = false (the finished copy is then kept aside instead
        // of published at the home its contents name).
        let sandbox = Sandbox::new("orphan_staged");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let stale = sandbox
            .root
            .join(STAGING_DIR)
            .join(format!("1-0-{}", NOW - STAGING_STALE_AFTER_SECS - 1));
        let finished = format!("valid {} spool/key", NOW - 100);
        write_dir(&stale, &finished);
        lifecycle.sweep_staging();
        assert_eq!(content(&sandbox.entry()), finished);
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
    }

    #[test]
    fn sweep_siblings_restores_a_sibling_replaced_between_the_peek_and_the_claim() {
        // kills: sweep_siblings: removing an aged sibling by its receipt peek
        // alone instead of through claim + settle (the sibling is then gone).
        let sandbox = Sandbox::new("sibling");
        let lifecycle = Lifecycle::new(&sandbox.root, &fixed_now, &self_fake);
        let sibling = sandbox.root.join("spool").join("other");
        // The receipt peek says aged; the claimed contents say young (a
        // publisher replaced it meanwhile).
        write_dir(&sibling, &format!("valid {} spool/other", NOW - 1));
        fs::write(
            sibling.join(RECEIPT_FILE_NAME),
            format!("{{\"created_unix\": {}}}", NOW - CACHE_STALE_AFTER_SECS - 1),
        )
        .unwrap();
        write_dir(&sandbox.entry(), "valid 2");
        lifecycle.sweep_siblings(&sandbox.entry());
        assert_eq!(content(&sibling), format!("valid {} spool/other", NOW - 1));
        assert!(sandbox.staging().is_empty(), "{:?}", sandbox.staging());
    }
}
