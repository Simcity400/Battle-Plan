//! Releasing capture-spool storage that the checkpoint cache already owns.
//!
//! # The invariant
//!
//! At every instant, every captured hour is either durably in the checkpoint
//! cache or still present in the spool. A crash of the fold or of the host
//! never loses a recording.
//!
//! Nothing here releases a byte on the strength of an in-memory belief, and
//! nothing takes a caller's word for how much is durable. A trim re-reads
//! `checkpoints.h4cp` and `hour_index.h4ix` from disk and parses them with the
//! same `snapshot::ReplayCacheReader` production replay uses, which binds the
//! index to the decoded stream's length and SHA-256. The hour count and the
//! decoded watermark are then read OUT of that parsed index, so a caller can
//! name a spool but never name a watermark. The ordering is:
//!
//! 1. verify the artifacts (re-read, parsed, bound to this decoded stream,
//!    directory synced) and derive the watermark from them,
//! 2. pick the greatest storage frame boundary whose decoded offset is still
//!    inside that derived range,
//! 3. write the trim journal and fsync it — the journal is what a later reader
//!    consults, so it becomes true *before* the bytes stop being there,
//! 4. release the storage prefix.
//!
//! Every crash point is safe by construction. Before (3) the spool is whole.
//! Between (3) and (4), or part-way through (4), the journal already names the
//! live offset, so a reader starts at the first live frame and the released
//! prefix is never interpreted; the worst outcome is storage that was
//! journalled as released but is still allocated, which costs disk and never
//! costs data. The forbidden state — bytes gone while the journal still claims
//! the spool is whole — is unreachable because (4) never runs before (3) is
//! durable.
//!
//! # Why a released prefix and not per-hour segment files
//!
//! The spool has one path, one length and one decoded-offset space, and about
//! forty call sites in the recorder host depend on all three (the certified
//! hour index literally stores decoded offsets into it, and the container
//! packer streams it as a single entry). Splitting it into per-hour files
//! would rewrite every one of those and change the sealed artifacts' notion of
//! where an hour lives. Releasing a storage *prefix* instead keeps the path,
//! the logical length and every offset exactly as they are, while still
//! handing the clusters back to the filesystem: on NTFS the released range is
//! deallocated, which `reclaimed_stored_bytes` measures as the drop in the
//! file's size on disk rather than assuming. The unit of release is the
//! storage frame, because a frame is the smallest span that can be dropped
//! while leaving the remainder independently decodable — see
//! `compact_stream::StreamWriter::create_with_frames`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use snapshot::sha256::Sha256;

use crate::duplex_session::rollover_fs::publish_immutable_descriptor;

/// Sidecar naming the live region of a spool whose prefix was released.
///
/// The suffix is owned by `compact_stream`, because that crate's zero-offset
/// entry points fail closed on its mere presence: a released prefix reads as
/// zeros, and zeros are not the zstd magic, so a storage-blind reader would
/// otherwise classify a compressed spool as raw and ingest the hole as
/// content.
pub const TRIM_JOURNAL_SUFFIX: &str = compact_stream::RELEASED_PREFIX_SIDECAR_SUFFIX;
pub const TRIM_JOURNAL_SCHEMA: &str = "hoi4-spool-trim-journal-v1";

/// Which artifacts to check, and which decoded stream they must belong to.
///
/// Note what this does NOT contain: how many hours the cache holds, or how far
/// into the stream they reach. Those are DERIVED from the artifacts themselves
/// by `verify_durable_receipt`, so a caller cannot authorise a release by
/// asserting a watermark. All a caller supplies is the identity of the decoded
/// stream - the capture's own rolling length and SHA-256 - and the artifacts
/// must prove they were built from exactly that stream.
#[derive(Debug, Clone)]
pub struct DurableHourReceipt {
    pub checkpoint_cache_path: PathBuf,
    pub hour_index_path: PathBuf,
    /// Decoded length of the spool, as the capture counted it.
    pub decoded_len: u64,
    /// Decoded SHA-256 of the spool, as the capture rolled it.
    pub decoded_sha256: String,
    /// The certifier's own published-coverage receipt, written beside the
    /// artifacts (`snapshot::PUBLISHED_COVERAGE_FILE`).
    pub published_coverage_path: PathBuf,
    /// The same coverage, as the fold reported it IN THIS PROCESS.
    ///
    /// Both are required, and they must agree. Two reviews rejected each of
    /// them alone, for opposite reasons, and the pair is what answers both:
    ///
    /// * the in-memory value alone is a caller's claim -- round 2's objection.
    ///   `min(index_ceiling, claim)` does not bound it either, because the
    ///   ceiling IS the decoded length on exactly the stream shape that needs
    ///   bounding (a terminal `FRAME_REFUSED`), so the minimum degenerates to
    ///   the claim;
    /// * the on-disk receipt alone is forgeable -- round 6's objection. Every
    ///   anchor it carries (source identity, both artifact digests, hour count)
    ///   is readable from the artifacts themselves, so a hybrid document with
    ///   correct anchors and an invented in-range offset passes every check.
    ///
    /// Requiring both closes the live capture path, which is the only path that
    /// releases anything: the in-memory value comes from the cursor in the same
    /// process that is about to do the release, so writing files into the
    /// artifact directory no longer moves the watermark.
    ///
    /// RESIDUAL, stated rather than implied, and stated no larger than it is.
    /// Write access to the artifact directory alone is NOT enough: the trim
    /// recomputes both artifact digests from the files it just read and
    /// requires the sidecar to carry those same digests, so replacing the
    /// cache and index leaves the old sidecar failing its binding, and
    /// replacing all three still leaves the offset having to equal what the
    /// fold reported in this process. What remains uncovered is an actor who
    /// controls this process's memory AND the directory together; nothing here
    /// defends against that, and neither does anything else in the recorder --
    /// it is the same boundary the capture-pipe authentication already
    /// declares out of threat model. Closing it would mean authenticating the
    /// artifact directory as a signed unit, which is a different design.
    pub fold_covered_decoded_end: u64,
}

/// What the artifacts on disk actually prove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedDurableHours {
    pub hour_count: u64,
    /// Greatest `h4cr_end_offset` in the parsed index. The ceiling, not the
    /// watermark: see `durable_decoded_end_offset`.
    pub indexed_decoded_end_offset: u64,
    /// Decoded H4CR offset the release is actually allowed to reach: the
    /// MINIMUM of the index's ceiling and the fold's own published coverage.
    /// Storage holding decoded bytes at or after this offset is never
    /// released.
    pub durable_decoded_end_offset: u64,
    pub checkpoint_cache_len: u64,
    pub checkpoint_cache_sha256: String,
    pub hour_index_len: u64,
    pub hour_index_sha256: String,
}

/// Storage layout of a spool: where each independently-decodable frame starts.
#[derive(Debug, Clone)]
pub struct SpoolFrames {
    pub decoded_len: u64,
    pub stored_len: u64,
    pub compression: compact_stream::StreamCompression,
    pub boundaries: Vec<compact_stream::FrameBoundary>,
}

impl SpoolFrames {
    pub fn from_receipt(receipt: &compact_stream::StreamWriteReceipt) -> Self {
        Self {
            decoded_len: receipt.decoded_len,
            stored_len: receipt.stored_len,
            compression: receipt.compression,
            boundaries: receipt.frame_boundaries.clone(),
        }
    }

    /// The greatest frame start whose whole span is already durable.
    ///
    /// A boundary is only releasable if the decoded bytes BEFORE it are all
    /// covered by the receipt, so the comparison is against the boundary's own
    /// decoded offset. The first boundary is `{0, 0}`, which releases nothing.
    pub fn releasable_boundary(
        &self,
        durable_decoded_end_offset: u64,
    ) -> Option<compact_stream::FrameBoundary> {
        self.boundaries
            .iter()
            .copied()
            .filter(|boundary| boundary.decoded_offset <= durable_decoded_end_offset)
            .filter(|boundary| boundary.stored_offset > 0)
            .max_by_key(|boundary| boundary.stored_offset)
    }
}

/// What the spool looks like after a trim, and what it cost the filesystem.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrimJournal {
    pub schema: String,
    pub stream_file: String,
    /// Storage encoding, so a reader opens the live region the same way the
    /// writer stored it instead of sniffing a released prefix's magic.
    pub compression: String,
    pub decoded_len: u64,
    pub stored_len: u64,
    /// First storage byte still live. A reader MUST start here.
    pub live_stored_offset: u64,
    /// Decoded offset that `live_stored_offset` corresponds to.
    pub live_decoded_offset: u64,
    /// Storage bytes the trim released.
    pub released_stored_bytes: u64,
    pub durable_hour_count: u64,
    pub durable_decoded_end_offset: u64,
    pub checkpoint_cache_sha256: String,
    pub hour_index_sha256: String,
}

impl TrimJournal {
    /// Where the surviving storage starts, and how to decode it.
    ///
    /// Fails rather than guessing. Defaulting an unrecognised codec to raw
    /// would hand a caller the still-COMPRESSED suffix as if it were H4CR --
    /// the same class of silent misread the released-prefix guard exists to
    /// prevent, arrived at from the other direction.
    pub fn live_region(&self) -> Result<(u64, compact_stream::StreamCompression), String> {
        let compression = compact_stream::StreamCompression::parse(&self.compression)
            .ok_or_else(|| {
                format!(
                    "trim journal for {} records an unknown storage encoding {:?}; the surviving \
                     region cannot be decoded",
                    self.stream_file, self.compression
                )
            })?;
        Ok((self.live_stored_offset, compression))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrimOutcome {
    /// Nothing was durable enough to release, or no frame boundary fell inside
    /// the certified range. The spool is untouched.
    NothingReleasable { reason: String },
    Released {
        journal: TrimJournal,
        /// Storage bytes the filesystem actually gave back, measured after the
        /// release from its own allocated-range map. Zero means the volume did
        /// not deallocate; that is reported, never rounded up to success.
        reclaimed_stored_bytes: u64,
    },
}

/// Where a trim publishes its journal for this spelling of the spool path.
pub fn journal_path(stream_path: &Path) -> PathBuf {
    compact_stream::released_prefix_sidecar(stream_path)
}

/// Read the trim journal, if this spool has one.
pub fn read_journal(stream_path: &Path) -> Result<Option<TrimJournal>, String> {
    // Every CANONICALISABLE spelling of this storage file, not just the one
    // the caller used: an alias must not be able to look past the journal and
    // then follow the alias to the trimmed bytes. Same rule, same helper, as
    // the codec's own guard -- including the same documented residual, which is
    // that an NTFS hard link is a second directory entry rather than a
    // canonicalisable alias and is therefore NOT covered. See
    // `compact_stream::released_prefix_sidecars`.
    for path in compact_stream::released_prefix_sidecars(stream_path) {
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("read trim journal {}: {error}", path.display())),
        };
        let journal: TrimJournal = serde_json::from_slice(&bytes)
            .map_err(|error| format!("decode trim journal {}: {error}", path.display()))?;
        if journal.schema != TRIM_JOURNAL_SCHEMA {
            return Err(format!(
                "trim journal {} has unsupported schema {:?}",
                path.display(),
                journal.schema
            ));
        }
        // Validate here too, so a corrupt codec is a load error rather than a
        // surprise at whatever later point someone asks for the live region.
        journal
            .live_region()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        return Ok(Some(journal));
    }
    Ok(None)
}

/// Fail-closed guard for every consumer that reads the spool from offset zero.
///
/// A trimmed spool's leading bytes are gone; reading them back would silently
/// produce zeros where stream content used to be. Any code path that has not
/// been taught to start at the journal's live offset must call this first and
/// refuse with the reason instead of decoding the hole.
pub fn require_whole_spool(stream_path: &Path, consumer: &str) -> Result<(), String> {
    match read_journal(stream_path)? {
        None => Ok(()),
        Some(journal) if journal.live_stored_offset == 0 => Ok(()),
        Some(journal) => Err(format!(
            "{consumer} needs the whole spool but {} was trimmed: storage bytes [0,{}) were \
             released after hours through decoded offset {} were durably checkpointed \
             (checkpoints.h4cp sha256={}); read the sealed replay container instead",
            stream_path.display(),
            journal.live_stored_offset,
            journal.durable_decoded_end_offset,
            journal.checkpoint_cache_sha256
        )),
    }
}

/// Establish, from the artifacts on disk, which hours really are durable.
///
/// This is the whole authority for a release, so it never takes a caller's
/// word for anything:
///
/// * both artifacts are read back from disk and parsed by the SAME reader
///   production replay uses, `snapshot::ReplayCacheReader`, which binds the
///   index to the decoded source length and SHA-256 and re-hashes the cache
///   against the index. Fabricated bytes, or a real artifact pair from a
///   different capture, cannot pass that binding;
/// * the hour count and the decoded watermark are read OUT of the parsed
///   index, never supplied. A caller therefore cannot name a watermark;
/// * the artifacts' directory entries are synced, so a crash cannot reach a
///   state where the cache file is unreachable while the spool prefix has
///   already been released. (On Windows that helper is a documented no-op:
///   the product path is write-through.)
pub fn verify_durable_receipt(
    receipt: &DurableHourReceipt,
) -> Result<VerifiedDurableHours, String> {
    if receipt.decoded_len == 0 || receipt.decoded_sha256.len() != 64 {
        return Err(format!(
            "durable receipt names an implausible decoded stream: {} bytes, sha256 {:?}",
            receipt.decoded_len, receipt.decoded_sha256
        ));
    }
    let mut read = Vec::new();
    for (label, path) in [
        ("checkpoints.h4cp", &receipt.checkpoint_cache_path),
        ("hour_index.h4ix", &receipt.hour_index_path),
    ] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| format!("stat durable {label} {}: {error}", path.display()))?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "durable {label} is not a regular file: {}",
                path.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("read durable {label} {}: {error}", path.display()))?;
        if bytes.len() as u64 != metadata.len() {
            return Err(format!(
                "durable {label} changed length while being read: {}",
                path.display()
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| format!("durable {label} has no parent directory"))?;
        crate::duplex_session::rollover_fs::sync_directory(parent)
            .map_err(|error| format!("durable {label}: {error}"))?;
        read.push(bytes);
    }
    let cache_bytes = read.remove(0);
    let index_bytes = read.remove(0);

    let reader = snapshot::ReplayCacheReader::open(
        &index_bytes,
        &cache_bytes,
        receipt.decoded_len,
        &receipt.decoded_sha256,
    )
    .map_err(|error| {
        format!(
            "durable artifacts are not bound to this {}-byte spool (sha256 {}): {error}",
            receipt.decoded_len, receipt.decoded_sha256
        )
    })?;
    let hours = reader.index().hours();
    if hours.is_empty() {
        return Err("durable checkpoint cache holds no hours".to_string());
    }
    let indexed_decoded_end_offset = hours
        .iter()
        .map(snapshot::ReplayHourEntry::h4cr_end_offset)
        .max()
        .unwrap_or(0);
    if indexed_decoded_end_offset == 0 {
        return Err("durable checkpoint cache covers zero decoded bytes".to_string());
    }
    if indexed_decoded_end_offset > receipt.decoded_len {
        return Err(format!(
            "durable checkpoint cache reaches decoded offset {indexed_decoded_end_offset}, past the {}-byte spool it claims to describe",
            receipt.decoded_len
        ));
    }
    // The index's last hour is stretched to the decoded length so a seek can
    // replay it, which makes it a CEILING on what may be released and not a
    // statement that the cache holds those bytes. A stream ending in a
    // terminal FRAME_REFUSED is the case that separates the two: the refused
    // hour published no world state, so its bytes live only in the spool. The
    // certifier's own coverage receipt is what closes that gap, and it is read
    // from disk and re-bound here rather than accepted from the caller.
    let checkpoint_cache_sha256 = Sha256::hex(&cache_bytes);
    let hour_index_sha256 = Sha256::hex(&index_bytes);
    let published_decoded_end_offset = read_published_coverage(
        &receipt.published_coverage_path,
        receipt,
        &checkpoint_cache_sha256,
        &hour_index_sha256,
        hours.len() as u64,
    )?;
    let durable_decoded_end_offset = indexed_decoded_end_offset.min(published_decoded_end_offset);
    if durable_decoded_end_offset == 0 {
        return Err(format!(
            "the fold published no hour coverage ({published_decoded_end_offset} of an indexed \
             {indexed_decoded_end_offset} decoded bytes), so nothing is durable"
        ));
    }
    Ok(VerifiedDurableHours {
        hour_count: hours.len() as u64,
        indexed_decoded_end_offset,
        durable_decoded_end_offset,
        checkpoint_cache_len: cache_bytes.len() as u64,
        checkpoint_cache_sha256,
        hour_index_len: index_bytes.len() as u64,
        hour_index_sha256,
    })
}

/// Read the certifier's published-coverage receipt and prove it belongs here.
///
/// What this establishes, precisely: the document names THIS capture (the
/// decoded identity comes from the capture's own rolling hash, not from any
/// file in this directory), and THESE artifacts (both digests were computed a
/// few lines ago from the files just read), and it agrees with what the fold
/// reported in this process. What it does NOT establish is authorship -- the
/// document is not signed, and every anchor in it can be copied from the
/// artifacts. That is why `fold_covered_decoded_end` is required alongside it;
/// see its documentation for the exact bound and the residual.
fn read_published_coverage(
    path: &Path,
    receipt: &DurableHourReceipt,
    checkpoint_cache_sha256: &str,
    hour_index_sha256: &str,
    hour_count: u64,
) -> Result<u64, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("read published coverage {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("decode published coverage {}: {error}", path.display()))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("published coverage {} is not an object", path.display()))?;

    let string = |key: &str| -> Result<String, String> {
        object
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("published coverage {} has no {key}", path.display()))
    };
    let number = |key: &str| -> Result<u64, String> {
        object
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| format!("published coverage {} has no {key}", path.display()))
    };

    if string("schema")? != snapshot::PUBLISHED_COVERAGE_SCHEMA {
        return Err(format!(
            "published coverage {} has an unsupported schema",
            path.display()
        ));
    }
    for (label, actual, expected) in [
        ("source_sha256", string("source_sha256")?, receipt.decoded_sha256.clone()),
        (
            "checkpoint_cache_sha256",
            string("checkpoint_cache_sha256")?,
            checkpoint_cache_sha256.to_string(),
        ),
        (
            "hour_index_sha256",
            string("hour_index_sha256")?,
            hour_index_sha256.to_string(),
        ),
    ] {
        if actual != expected {
            return Err(format!(
                "published coverage {} records {label} {actual}, these artifacts have {expected}",
                path.display()
            ));
        }
    }
    for (label, actual, expected) in [
        ("source_len", number("source_len")?, receipt.decoded_len),
        ("hour_count", number("hour_count")?, hour_count),
    ] {
        if actual != expected {
            return Err(format!(
                "published coverage {} records {label} {actual}, these artifacts have {expected}",
                path.display()
            ));
        }
    }

    let published = number("published_decoded_end_offset")?;
    if published > receipt.decoded_len {
        return Err(format!(
            "published coverage {} reaches {published}, past the {}-byte spool",
            path.display(),
            receipt.decoded_len
        ));
    }
    // The durable document and the in-process fold must say the same thing. A
    // document edited to raise the offset while keeping every anchor correct
    // dies here, because the anchors are all copyable from the artifacts and
    // this number is not.
    if published != receipt.fold_covered_decoded_end {
        return Err(format!(
            "published coverage {} records {published} decoded bytes, the fold reported {} in \
             this process; a coverage receipt that disagrees with its own fold authorises nothing",
            path.display(),
            receipt.fold_covered_decoded_end
        ));
    }
    Ok(published)
}

/// Release the spool storage that `receipt` proves is already durable.
///
/// Returns without touching the spool when nothing is releasable. Never
/// releases storage holding decoded bytes at or past
/// `receipt.durable_decoded_end_offset`.
pub fn trim_spool_prefix(
    stream_path: &Path,
    frames: &SpoolFrames,
    receipt: &DurableHourReceipt,
) -> Result<TrimOutcome, String> {
    trim_spool_prefix_with_release(stream_path, frames, receipt, release_storage_prefix)
}

/// The trim, with the storage release injected.
///
/// The seam exists so a test can fail the release and observe what a crash at
/// that instant leaves behind: the journal must already be durable. Production
/// always passes `release_storage_prefix`.
pub fn trim_spool_prefix_with_release(
    stream_path: &Path,
    frames: &SpoolFrames,
    receipt: &DurableHourReceipt,
    release: impl FnOnce(&Path, u64) -> Result<u64, String>,
) -> Result<TrimOutcome, String> {
    let verified = verify_durable_receipt(receipt)?;
    if receipt.decoded_len != frames.decoded_len {
        return Err(format!(
            "durable receipt describes a {}-byte decoded stream, the spool write receipt              recorded {}",
            receipt.decoded_len, frames.decoded_len
        ));
    }
    if let Some(existing) = read_journal(stream_path)? {
        if existing.live_stored_offset > 0 {
            return Ok(TrimOutcome::NothingReleasable {
                reason: format!(
                    "spool was already trimmed to storage offset {}",
                    existing.live_stored_offset
                ),
            });
        }
    }
    let boundary = match frames.releasable_boundary(verified.durable_decoded_end_offset) {
        Some(boundary) => boundary,
        None => {
            return Ok(TrimOutcome::NothingReleasable {
                reason: format!(
                    "no storage frame boundary falls inside the durable decoded range [0,{}) \
                     ({} boundary/boundaries recorded)",
                    verified.durable_decoded_end_offset,
                    frames.boundaries.len()
                ),
            })
        }
    };
    let stored_len = fs::symlink_metadata(stream_path)
        .map_err(|error| format!("stat spool {}: {error}", stream_path.display()))?
        .len();
    if stored_len != frames.stored_len {
        return Err(format!(
            "spool {} is {stored_len} bytes on disk, the write receipt recorded {}",
            stream_path.display(),
            frames.stored_len
        ));
    }
    if boundary.stored_offset > stored_len {
        return Err(format!(
            "frame boundary at storage offset {} is past the {stored_len}-byte spool",
            boundary.stored_offset
        ));
    }

    let journal = TrimJournal {
        schema: TRIM_JOURNAL_SCHEMA.to_string(),
        stream_file: stream_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        compression: frames.compression.as_str().to_string(),
        decoded_len: frames.decoded_len,
        stored_len,
        live_stored_offset: boundary.stored_offset,
        live_decoded_offset: boundary.decoded_offset,
        released_stored_bytes: boundary.stored_offset,
        durable_hour_count: verified.hour_count,
        durable_decoded_end_offset: verified.durable_decoded_end_offset,
        checkpoint_cache_sha256: verified.checkpoint_cache_sha256.clone(),
        hour_index_sha256: verified.hour_index_sha256.clone(),
    };

    // (3) The journal becomes true before the bytes stop being there. It is
    // published immutably: the live offset a reader depends on is never
    // rewritten, so no later step can move it out from under a reader.
    write_journal_durably(stream_path, &journal)?;

    // (4) Now, and only now, hand the clusters back.
    let reclaimed_stored_bytes = release(stream_path, boundary.stored_offset).map_err(|error| {
        format!(
            "release spool prefix [0,{}) of {}: {error}",
            boundary.stored_offset,
            stream_path.display()
        )
    })?;
    Ok(TrimOutcome::Released {
        journal,
        reclaimed_stored_bytes,
    })
}

fn write_journal_durably(stream_path: &Path, journal: &TrimJournal) -> Result<(), String> {
    let path = journal_path(stream_path);
    let bytes = serde_json::to_vec_pretty(&json!(journal))
        .map_err(|error| format!("encode trim journal: {error}"))?;
    let root = path
        .parent()
        .ok_or_else(|| "trim journal has no parent directory".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "trim journal has no file name".to_string())?;
    publish_immutable_descriptor(root, name, &bytes)
}

/// Deallocate `[0, stored_offset)` of `path` and report the bytes the
/// filesystem actually gave back.
#[cfg(windows)]
fn release_storage_prefix(path: &Path, stored_offset: u64) -> Result<u64, String> {
    use std::os::windows::io::AsRawHandle;

    if stored_offset == 0 {
        return Ok(0);
    }

    const FSCTL_SET_SPARSE: u32 = 0x0009_00C4;
    const FSCTL_SET_ZERO_DATA: u32 = 0x0009_80C8;
    const INVALID_FILE_SIZE: u32 = u32::MAX;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct FileZeroDataInformation {
        file_offset: i64,
        beyond_final_zero: i64,
    }

    extern "system" {
        fn DeviceIoControl(
            device: *mut std::ffi::c_void,
            control_code: u32,
            in_buffer: *mut std::ffi::c_void,
            in_buffer_size: u32,
            out_buffer: *mut std::ffi::c_void,
            out_buffer_size: u32,
            bytes_returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
        fn GetCompressedFileSizeW(file_name: *const u16, high: *mut u32) -> u32;
        fn GetLastError() -> u32;
    }

    /// Bytes the volume actually has allocated for this file. For a sparse
    /// file this drops as ranges are deallocated, which is what makes the
    /// reclamation a measurement rather than an assumption.
    fn size_on_disk(path: &Path) -> Result<u64, String> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut high = 0u32;
        let low = unsafe { GetCompressedFileSizeW(wide.as_ptr(), &mut high) };
        if low == INVALID_FILE_SIZE {
            let error = unsafe { GetLastError() };
            if error != 0 {
                return Err(format!("GetCompressedFileSizeW failed with OS error {error}"));
            }
        }
        Ok((u64::from(high) << 32) | u64::from(low))
    }

    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("open for release: {error}"))?;
    let handle = file.as_raw_handle() as *mut std::ffi::c_void;
    // Flush first. Dirty cached pages left over from the capture writer would
    // otherwise be written back AFTER the punch and silently re-allocate the
    // range that was just released, which is how this reclaimed nothing on the
    // first measurement.
    file.sync_all()
        .map_err(|error| format!("flush spool before release: {error}"))?;

    let mut returned = 0u32;
    // A non-sparse file cannot deallocate a range; SET_SPARSE is what makes the
    // punch reclaim instead of merely zero. A volume that refuses it is
    // reported, never silently treated as success.
    let sparse = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if sparse == 0 {
        return Err(format!(
            "FSCTL_SET_SPARSE failed with OS error {} (volume cannot deallocate ranges)",
            unsafe { GetLastError() }
        ));
    }

    let allocated_before = size_on_disk(path)?;

    let mut zero = FileZeroDataInformation {
        file_offset: 0,
        beyond_final_zero: i64::try_from(stored_offset)
            .map_err(|_| "release offset exceeds i64".to_string())?,
    };
    let punched = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_ZERO_DATA,
            &mut zero as *mut _ as *mut std::ffi::c_void,
            std::mem::size_of::<FileZeroDataInformation>() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if punched == 0 {
        return Err(format!("FSCTL_SET_ZERO_DATA failed with OS error {}", unsafe {
            GetLastError()
        }));
    }
    file.sync_all()
        .map_err(|error| format!("sync released spool: {error}"))?;

    drop(file);

    let allocated_after = size_on_disk(path)?;
    if std::env::var_os("BATTLE_PLAN_SPOOL_TRIM_DEBUG").or_else(|| std::env::var_os("HOI4_SPOOL_TRIM_DEBUG")).is_some() {
        eprintln!(
            "spool_trim size_on_disk_before={allocated_before} \
             size_on_disk_after={allocated_after} released={stored_offset}"
        );
    }
    // Deliberately conservative. `allocated_before` is read from a file that is
    // not yet sparse, so it reports the logical length rather than the
    // cluster-rounded allocation, and NTFS only deallocates whole 64 KiB sparse
    // allocation units. Both effects make this UNDER-report reclamation; it
    // never claims more than the volume gave back.
    Ok(allocated_before.saturating_sub(allocated_after))
}

#[cfg(not(windows))]
fn release_storage_prefix(_path: &Path, _stored_offset: u64) -> Result<u64, String> {
    Err("spool trimming needs a filesystem that can deallocate a byte range; \
         this platform has no implementation"
        .to_string())
}

/// Convenience accessor used by the manifest writer.
pub fn trim_summary_json(outcome: &TrimOutcome) -> serde_json::Value {
    match outcome {
        TrimOutcome::NothingReleasable { reason } => json!({
            "released": false,
            "reason": reason,
        }),
        TrimOutcome::Released {
            journal,
            reclaimed_stored_bytes,
        } => json!({
            "released": true,
            "journal": journal_path(Path::new(&journal.stream_file))
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            "live_stored_offset": journal.live_stored_offset,
            "live_decoded_offset": journal.live_decoded_offset,
            "released_stored_bytes": journal.released_stored_bytes,
            "reclaimed_stored_bytes": reclaimed_stored_bytes,
            "durable_hour_count": journal.durable_hour_count,
            "durable_decoded_end_offset": journal.durable_decoded_end_offset,
            "checkpoint_cache_sha256": journal.checkpoint_cache_sha256,
            "hour_index_sha256": journal.hour_index_sha256,
        }),
    }
}

#[cfg(test)]
mod tests;
