//! A reference `.battleplan` PACKER (writer side).
//!
//! NOTE ON OWNERSHIP: this Rust packer is both the production streaming writer
//! used by `recorder_host` finalization and the reference/test fixture builder.
//! It is loss-LESS by
//! construction: payload files are stored as RAW BYTES, never re-serialized, so
//! the Absent/null/0 trichotomy, the rollback filenames, and compact-JSON
//! formatting all survive unchanged (spec §5).
//!
//! It also backs a tiny CLI (`src/bin/pack_replay.rs`) so a directory store can be
//! packed/round-tripped from the command line for verification.

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::zip::{
    portable_path_key, validate_payload_path, MAX_CONTAINER_ENTRY_BYTES,
    STREAMING_MAX_CONTAINER_ENTRY_BYTES, ZIP32_MAX_ENTRY_COUNT, ZIP32_MAX_FIELD_VALUE,
    ZIP_METHOD_DEFLATE, ZIP_METHOD_STORE, ZIP_METHOD_ZSTD, ZSTD_WINDOW_LOG,
};
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// One file to pack: a container path + its RAW bytes + a role label.
pub struct PackEntry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub role: &'static str, // "index" | "snapshot" | "timeline"
    pub hour_index: Option<i64>,
}

/// One borrowed file to pack. This is the production-friendly variant used when
/// the caller already owns large immutable payload buffers and should not clone
/// them just to build the container.
pub struct BorrowedPackEntry<'a> {
    pub path: String,
    pub bytes: &'a [u8],
    pub role: &'static str,
    pub hour_index: Option<i64>,
}

/// One file-backed entry to pack. The container stores the file's exact bytes,
/// but the packer does not require the caller to keep those bytes resident.
pub struct FileBackedPackEntry<'a> {
    pub path: String,
    pub file_path: &'a Path,
    pub role: &'static str,
    pub hour_index: Option<i64>,
}

/// One STREAMING file-backed entry — like [`FileBackedPackEntry`] but the bytes
/// are compressed and written in bounded-memory chunks (see
/// [`write_zip_file_streaming`]) instead of the whole file being read into RAM.
/// This is what lets the recorder append a multi-GB raw compact stream to the
/// container without ever materializing it. `sha256`, when supplied, is the
/// caller-computed digest of the decoded bytes (e.g. the capture-time rolling
/// hash of the pipe stream). It seeds the manifest without a separate hash
/// pass, but the exact decoded bytes emitted into the ZIP are hashed in that
/// same write pass and must match before the manifest is written. When `None`,
/// the packer streams the file once to establish the expected digest first and
/// again to write-and-verify it.
pub struct StreamingFileEntry<'a> {
    pub path: String,
    pub file_path: &'a Path,
    /// Storage encoding of file_path. The container entry always receives the
    /// decoded bytes, so its payload and manifest hash remain canonical H4CR.
    pub source_compression: compact_stream::StreamCompression,
    /// Exact decoded byte length when source_compression is zstd. Supplying this
    /// avoids a separate decompression pass merely to size the manifest entry.
    pub decoded_len: Option<u64>,
    pub role: &'static str,
    pub hour_index: Option<i64>,
    pub sha256: Option<String>,
}

/// One authenticated large entry supplied through a caller-owned retained
/// source. `source_id` is opaque to the packer; the ordered streaming callback
/// must copy that source into the unpublished ZIP sink and the packer proves
/// the exact declared length and SHA-256 during the same write pass.
pub struct VerifiedPackEntry {
    pub path: String,
    pub source_id: usize,
    pub expected_len: u64,
    pub expected_sha256: String,
    pub role: &'static str,
    pub hour_index: Option<i64>,
}

/// One entry in a caller-defined global ZIP order. Small canonical metadata can
/// remain borrowed while large payloads stay file-backed; unlike the older
/// grouped mixed packers, this representation preserves one exact ordering
/// across both source kinds.
pub enum OrderedStreamingPackEntry<'a> {
    Borrowed(BorrowedPackEntry<'a>),
    File(StreamingFileEntry<'a>),
    Verified(VerifiedPackEntry),
}

/// Identity recorded into the manifest (the values the capture read at session
/// start — all already redacted of personal paths upstream).
#[derive(Clone, Default)]
pub struct PackIdentity {
    pub schema_version: Option<String>,
    pub build_exe_sha256: Option<String>,
    pub build_pinned: bool,
    /// the whole provenance.mod object (or None = mod-set not read)
    pub mod_identity: Option<Value>,
    pub created_utc: Option<String>,
    pub content_manifest: Option<portable_content_manifest::ContentManifest>,
}

#[derive(Debug)]
pub enum PackError {
    BadPath(String),
    ContentManifest(String),
    Io(std::io::Error),
}
impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::BadPath(p) => write!(f, "pack: unsafe path {p}"),
            PackError::ContentManifest(message) => {
                write!(f, "pack: invalid portable content manifest: {message}")
            }
            PackError::Io(e) => write!(f, "pack io: {e}"),
        }
    }
}
impl std::error::Error for PackError {}
impl From<std::io::Error> for PackError {
    fn from(e: std::io::Error) -> Self {
        PackError::Io(e)
    }
}

fn validate_distinct_paths<'a>(paths: impl IntoIterator<Item = &'a str>) -> Result<(), PackError> {
    let mut seen = HashSet::new();
    for path in paths {
        let key = portable_path_key(path).map_err(|_| PackError::BadPath(path.to_string()))?;
        if !seen.insert(key) {
            return Err(PackError::BadPath(path.to_string()));
        }
    }
    Ok(())
}

const PORTABLE_CONTENT_FEATURE: &str = "portable_content_manifest_v2";

fn portable_content_manifest_value(identity: &PackIdentity) -> Result<Value, PackError> {
    let Some(manifest) = &identity.content_manifest else {
        return Ok(Value::Null);
    };
    manifest.validate().map_err(PackError::ContentManifest)?;
    if manifest.manifest_digest_sha256.is_none() {
        return Err(PackError::ContentManifest(
            "manifest_digest_sha256 is missing; finalize the content manifest before packing"
                .into(),
        ));
    }
    serde_json::to_value(manifest).map_err(|err| PackError::ContentManifest(err.to_string()))
}

fn portable_content_required_features(
    identity: &PackIdentity,
    mut features: Vec<String>,
) -> Vec<String> {
    if identity.content_manifest.is_some()
        && !features
            .iter()
            .any(|feature| feature == PORTABLE_CONTENT_FEATURE)
    {
        features.push(PORTABLE_CONTENT_FEATURE.into());
    }
    features.sort();
    features.dedup();
    features
}

/// Knobs for building deliberately-malformed containers in tests (never used by
/// the production/CLI path). All default to "well-formed".
#[derive(Clone, Default)]
pub struct PackOptions {
    /// omit `manifest.json` entirely (-> the recovery-only / NoManifest case)
    pub omit_manifest: bool,
    /// inject a `required_features` the reader will not know (-> forward-compat reject)
    pub inject_required_feature: Option<String>,
    /// REPLACE the whole `required_features` field with an arbitrary JSON value
    /// (e.g. a STRING instead of an array) to exercise the reader's malformed-type
    /// fail-closed path. Takes precedence over `inject_required_feature`.
    pub required_features_raw: Option<Value>,
    /// append a zip entry that is NOT listed in the manifest (-> stowaway reject).
    /// `(path, bytes)`; the manifest digest stays valid because the entry never
    /// enters the manifest — exactly the tamper case the reader must catch.
    pub inject_stowaway: Option<(String, Vec<u8>)>,
    /// Duplicate the first raw role member with the same value after the
    /// semantic manifest and digest are complete. Ordinary last-wins decoding
    /// would preserve the semantic value and valid digest.
    pub inject_duplicate_role_member: bool,
    /// Replace the first manifest entry's role with an arbitrary raw value.
    pub first_role_raw: Option<Value>,
    /// PRODUCTION: the `container_version` to declare (default `None` -> 1). A
    /// stitched (reload/relaunch) container sets this to 2 (`STITCHING_DESIGN.md`
    /// §9). Not a negative-test knob.
    pub container_version: Option<u64>,
    /// PRODUCTION: the `required_features` to declare (default `None` -> `[]`). A
    /// stitched container sets `["timeline_segments_v1"]`. Distinct from the
    /// negative-test injectors above; `required_features_raw` and
    /// `inject_required_feature` still take precedence for malformed-input tests.
    pub required_features: Option<Vec<String>>,
}

/// Default zstd compression level for production `.battleplan` payload entries.
///
/// Level 3 keeps the window-27 decoder contract while avoiding the excessive
/// single-core finalization time observed when packaging full-roster, per-hour
/// snapshots at level 19. The earlier B3 corpus result remains useful as an
/// opt-in size tradeoff: level 19 packed 9.65 GB decoded to 42.37 MB, 28.2%
/// smaller than level 3, but production now prioritizes prompt finalization.
pub const DEFAULT_ZSTD_COMPRESSION_LEVEL: i32 = 3;
/// Backward-compatible alias for callers/tests that referenced the old constant.
pub const ZSTD_COMPRESSION_LEVEL: i32 = DEFAULT_ZSTD_COMPRESSION_LEVEL;
/// The smallest zstd window log accepted by libzstd.
pub const MIN_ZSTD_WINDOW_LOG: u32 = 10;
/// Entries smaller than this stay STORE even under the zstd policy.
///
/// The policy is a pure size floor: at or above it EVERY entry is compressed.
/// It used to additionally consult a three-name allowlist (`*.snapshot.json`,
/// `/collector_raw/`, `/production_render_blobs/`), which left every other
/// large payload STORED at full size. Measured on the sealed t26 recording the
/// entries that allowlist excluded were 160,247,978 bytes of pure JSON — the
/// recorder manifest, all six static catalogs, `index.json` and `timeline.json`
/// — with no already-compressed image or archive data among them; at this
/// profile's level they pack to 10,711,580 bytes (93.3% smaller). The exclusion
/// was an oversight, not an incompressibility finding, and it also silently
/// covered the `replay_decoder` hour index and checkpoint cache, which are the
/// artifacts that replace the per-hour JSON entirely.
///
/// Below the floor a zstd frame's own header and checksum can cost more than
/// the payload saves, and the container carries many tiny descriptors.
/// Genuinely incompressible bytes need no special case: zstd emits stored
/// blocks for them, bounding the worst case at a small per-frame overhead
/// rather than expansion.
pub const ZSTD_MIN_ENTRY_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionProfile {
    pub zstd_level: i32,
    pub zstd_window_log: u32,
}

impl Default for CompressionProfile {
    fn default() -> Self {
        CompressionProfile {
            zstd_level: DEFAULT_ZSTD_COMPRESSION_LEVEL,
            zstd_window_log: ZSTD_WINDOW_LOG,
        }
    }
}

impl CompressionProfile {
    pub fn validate(self) -> std::io::Result<()> {
        if self.zstd_window_log < MIN_ZSTD_WINDOW_LOG {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "zstd window log {} is below libzstd minimum {}",
                    self.zstd_window_log, MIN_ZSTD_WINDOW_LOG
                ),
            ));
        }
        if self.zstd_window_log > ZSTD_WINDOW_LOG {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "zstd window log {} exceeds reader compatibility max {}",
                    self.zstd_window_log, ZSTD_WINDOW_LOG
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackCompression {
    Store,
    Deflate,
    ZstdLarge,
}

impl PackCompression {
    fn from_deflate_flag(deflate: bool) -> PackCompression {
        if deflate {
            PackCompression::Deflate
        } else {
            PackCompression::Store
        }
    }

    fn method_for_entry(self, len: u64) -> ZipCompression {
        match self {
            PackCompression::Store => ZipCompression::Store,
            PackCompression::Deflate => ZipCompression::Deflate,
            PackCompression::ZstdLarge => {
                if len >= ZSTD_MIN_ENTRY_BYTES {
                    ZipCompression::Zstd
                } else {
                    ZipCompression::Store
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZipCompression {
    Store,
    Deflate,
    Zstd,
}

impl ZipCompression {
    fn method(self) -> u16 {
        match self {
            ZipCompression::Store => ZIP_METHOD_STORE,
            ZipCompression::Deflate => ZIP_METHOD_DEFLATE,
            ZipCompression::Zstd => ZIP_METHOD_ZSTD,
        }
    }
}

/// Build a complete `.battleplan` (manifest written last) from the given entries.
/// `game_id` must be the game the entries belong to (their paths are
/// `payload/<game_id>/...`). `deflate` toggles per-entry compression (the reader
/// handles both; STORE keeps the test fully deterministic).
pub fn pack(
    game_id: &str,
    entries: &[PackEntry],
    identity: &PackIdentity,
    deflate: bool,
) -> Result<Vec<u8>, PackError> {
    pack_with(game_id, entries, identity, deflate, &PackOptions::default())
}

/// Borrowing equivalent of [`pack`]. The generated bytes and manifest are
/// identical; only the packer's ownership model changes.
pub fn pack_borrowed(
    game_id: &str,
    entries: &[BorrowedPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
) -> Result<Vec<u8>, PackError> {
    pack_borrowed_with(game_id, entries, identity, deflate, &PackOptions::default())
}

/// Streaming equivalent of [`pack_borrowed`]. It writes the completed container
/// to `writer` instead of first materializing the whole `.battleplan` as one
/// contiguous `Vec<u8>`.
pub fn pack_borrowed_to_writer<W: Write>(
    game_id: &str,
    entries: &[BorrowedPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    writer: &mut W,
) -> Result<(), PackError> {
    pack_borrowed_with_writer(
        game_id,
        entries,
        identity,
        deflate,
        &PackOptions::default(),
        writer,
    )
}

/// Mixed memory/file-backed variant for production paths with large entries
/// already spilled to disk. File-backed entries are hashed by streaming from
/// disk and are read one-at-a-time when written to the container.
pub fn pack_mixed_to_writer<W: Write>(
    game_id: &str,
    borrowed_entries: &[BorrowedPackEntry<'_>],
    file_entries: &[FileBackedPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    writer: &mut W,
) -> Result<(), PackError> {
    planned_zip_file_count(&[borrowed_entries.len(), file_entries.len()], 1)?;
    validate_distinct_paths(
        borrowed_entries
            .iter()
            .map(|entry| entry.path.as_str())
            .chain(file_entries.iter().map(|entry| entry.path.as_str())),
    )?;

    let mut manifest_entries: Vec<Value> =
        Vec::with_capacity(borrowed_entries.len() + file_entries.len());
    let mut file_expectations = Vec::with_capacity(file_entries.len());

    let mut counts_snapshots = 0u64;
    for e in borrowed_entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let sha = to_hex(&Sha256::digest(e.bytes));
        let mut ent = json!({
            "path": e.path,
            "sha256": sha,
            "bytes": e.bytes.len() as u64,
            "role": e.role,
        });
        if let Some(h) = e.hour_index {
            ent.as_object_mut()
                .unwrap()
                .insert("hour_index".into(), json!(h));
        }
        if e.role == "snapshot" {
            counts_snapshots += 1;
        }
        manifest_entries.push(ent);
    }
    for e in file_entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let (sha, len) = sha256_file(e.file_path)?;
        file_expectations.push((sha.clone(), len));
        let mut ent = json!({
            "path": e.path,
            "sha256": sha,
            "bytes": len,
            "role": e.role,
        });
        if let Some(h) = e.hour_index {
            ent.as_object_mut()
                .unwrap()
                .insert("hour_index".into(), json!(h));
        }
        if e.role == "snapshot" {
            counts_snapshots += 1;
        }
        manifest_entries.push(ent);
    }
    manifest_entries.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or("")
            .cmp(b["path"].as_str().unwrap_or(""))
    });
    let content_manifest = portable_content_manifest_value(identity)?;
    let required_features = portable_content_required_features(identity, Vec::new());
    let mut manifest = json!({
        "format": "hoi4replay",
        "container_version": 1,
        "required_features": required_features,
        "optional_features": [],
        "schema_version": identity.schema_version,
        "game_id": game_id,
        "build": {
            "exe_sha256": identity.build_exe_sha256,
            "pinned": identity.build_pinned,
        },
        "mod_identity": identity.mod_identity.clone().unwrap_or(Value::Null),
        "content_manifest": content_manifest,
        "created_utc": identity.created_utc,
        "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
        "counts": { "snapshots": counts_snapshots },
        "entries": manifest_entries,
        "manifest_digest": Value::Null,
        "signature": Value::Null,
    });
    let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    manifest
        .as_object_mut()
        .unwrap()
        .insert("manifest_digest".into(), json!(digest));

    let mut central = Vec::new();
    let mut written = 0u64;
    let mut zip64_used = false;
    let mut file_count = 0usize;
    let compression = PackCompression::from_deflate_flag(deflate);
    for e in borrowed_entries {
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e.bytes,
            compression.method_for_entry(e.bytes.len() as u64),
            CompressionProfile::default(),
        )?;
        file_count += 1;
    }
    for (e, (expected_sha256, expected_len)) in
        file_entries.iter().zip(file_expectations.iter())
    {
        write_zip_file_from_path(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e.file_path,
            compression,
            CompressionProfile::default(),
            expected_sha256,
            *expected_len,
        )?;
        file_count += 1;
    }

    let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
    write_zip_file(
        writer,
        &mut central,
        &mut written,
        &mut zip64_used,
        "manifest.json",
        &manifest_bytes,
        ZipCompression::Store,
        CompressionProfile::default(),
    )?;
    file_count += 1;
    finish_zip(writer, &mut central, written, file_count, zip64_used)?;
    Ok(())
}

/// Streaming production packer: like [`pack_mixed_to_writer`] but the large
/// `streaming_entries` (the raw compact stream) are compressed and appended in
/// bounded-memory chunks, so peak memory stays O(one chunk) regardless of stream
/// length. `borrowed_entries` (index/timeline/manifest/static) are small and
/// stay in RAM; `file_entries` (per-hour snapshots) are read one-at-a-time.
///
/// The written ZIP uses a trailing data descriptor for each streaming entry (the
/// crc/sizes are unknown until the whole entry is written), and records the true
/// crc/sizes in the central directory, which is the authority our reader
/// ([`crate::zip`]) trusts. The legacy `deflate` flag keeps the old STORE vs
/// DEFLATE writer behavior for fixtures and old tooling.
pub fn pack_streaming_to_writer<W: Write>(
    game_id: &str,
    borrowed_entries: &[BorrowedPackEntry<'_>],
    file_entries: &[FileBackedPackEntry<'_>],
    streaming_entries: &[StreamingFileEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    writer: &mut W,
) -> Result<(), PackError> {
    pack_streaming_to_writer_with_compression(
        game_id,
        borrowed_entries,
        file_entries,
        streaming_entries,
        identity,
        PackCompression::from_deflate_flag(deflate),
        CompressionProfile::default(),
        writer,
    )
}

/// Options-aware streaming writer whose entry slice is already in the exact
/// desired ZIP order. This is the production seam for stitched containers:
/// canonical metadata stays borrowed, raw streams and snapshots are consumed
/// from files in bounded chunks, and the v2 feature/options contract is kept
/// identical to the in-memory packer.
pub fn pack_ordered_streaming_to_writer_with_options<W, F>(
    game_id: &str,
    entries: &[OrderedStreamingPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    opts: &PackOptions,
    writer: &mut W,
    mut copy_verified_source: F,
) -> Result<(), PackError>
where
    W: Write,
    F: FnMut(usize, &mut dyn Write) -> std::io::Result<()>,
{
    if deflate
        && entries
            .iter()
            .any(|entry| matches!(entry, OrderedStreamingPackEntry::Verified(_)))
    {
        return Err(PackError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "verified callback entries currently require deterministic STORE",
        )));
    }
    planned_zip_file_count(
        &[entries.len()],
        (if opts.inject_stowaway.is_some() { 1 } else { 0 })
            + (if !opts.omit_manifest { 1 } else { 0 }),
    )?;
    validate_distinct_paths(entries.iter().map(|entry| match entry {
        OrderedStreamingPackEntry::Borrowed(entry) => entry.path.as_str(),
        OrderedStreamingPackEntry::File(entry) => entry.path.as_str(),
        OrderedStreamingPackEntry::Verified(entry) => entry.path.as_str(),
    }))?;
    let mut previous_path: Option<&str> = None;
    for path in entries.iter().map(|entry| match entry {
        OrderedStreamingPackEntry::Borrowed(entry) => entry.path.as_str(),
        OrderedStreamingPackEntry::File(entry) => entry.path.as_str(),
        OrderedStreamingPackEntry::Verified(entry) => entry.path.as_str(),
    }) {
        if previous_path.is_some_and(|previous| previous >= path) {
            return Err(PackError::BadPath(format!(
                "ordered entry path {path:?} does not follow {previous_path:?}"
            )));
        }
        previous_path = Some(path);
    }

    let mut manifest_entries = Vec::with_capacity(entries.len());
    let mut expectations: Vec<Option<(String, u64)>> = Vec::with_capacity(entries.len());
    let mut counts_snapshots = 0u64;
    for entry in entries {
        let (path, sha256, bytes, role, hour_index, expectation) = match entry {
            OrderedStreamingPackEntry::Borrowed(entry) => {
                validate_payload_path(&entry.path)
                    .map_err(|_| PackError::BadPath(entry.path.clone()))?;
                (
                    entry.path.as_str(),
                    to_hex(&Sha256::digest(entry.bytes)),
                    entry.bytes.len() as u64,
                    entry.role,
                    entry.hour_index,
                    None,
                )
            }
            OrderedStreamingPackEntry::File(entry) => {
                validate_payload_path(&entry.path)
                    .map_err(|_| PackError::BadPath(entry.path.clone()))?;
                let (sha256, bytes) = match &entry.sha256 {
                    Some(sha256) => (sha256.clone(), streaming_entry_decoded_len(entry)?),
                    None => sha256_streaming_entry(entry)?,
                };
                (
                    entry.path.as_str(),
                    sha256.clone(),
                    bytes,
                    entry.role,
                    entry.hour_index,
                    Some((sha256, bytes)),
                )
            }
            OrderedStreamingPackEntry::Verified(entry) => {
                validate_payload_path(&entry.path)
                    .map_err(|_| PackError::BadPath(entry.path.clone()))?;
                (
                    entry.path.as_str(),
                    entry.expected_sha256.clone(),
                    entry.expected_len,
                    entry.role,
                    entry.hour_index,
                    Some((entry.expected_sha256.clone(), entry.expected_len)),
                )
            }
        };
        let mut manifest_entry = json!({
            "path": path,
            "sha256": sha256,
            "bytes": bytes,
            "role": role,
        });
        if let Some(hour_index) = hour_index {
            manifest_entry
                .as_object_mut()
                .expect("manifest entry is an object")
                .insert("hour_index".into(), json!(hour_index));
        }
        if role == "snapshot" {
            counts_snapshots = counts_snapshots.saturating_add(1);
        }
        manifest_entries.push(manifest_entry);
        expectations.push(expectation);
    }
    manifest_entries.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or("")
            .cmp(right["path"].as_str().unwrap_or(""))
    });
    if let (Some(raw), Some(first)) = (&opts.first_role_raw, manifest_entries.first_mut()) {
        first
            .as_object_mut()
            .expect("manifest entry object")
            .insert("role".into(), raw.clone());
    }

    let required_features = match (&opts.required_features_raw, &opts.inject_required_feature) {
        (Some(raw), _) => raw.clone(),
        (None, Some(feature)) => json!([feature]),
        (None, None) => json!(portable_content_required_features(
            identity,
            opts.required_features.clone().unwrap_or_default(),
        )),
    };
    let content_manifest = portable_content_manifest_value(identity)?;
    let mut manifest = json!({
        "format": "hoi4replay",
        "container_version": opts.container_version.unwrap_or(1),
        "required_features": required_features,
        "optional_features": [],
        "schema_version": identity.schema_version,
        "game_id": game_id,
        "build": {
            "exe_sha256": identity.build_exe_sha256,
            "pinned": identity.build_pinned,
        },
        "mod_identity": identity.mod_identity.clone().unwrap_or(Value::Null),
        "content_manifest": content_manifest,
        "created_utc": identity.created_utc,
        "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
        "counts": { "snapshots": counts_snapshots },
        "entries": manifest_entries,
        "manifest_digest": Value::Null,
        "signature": Value::Null,
    });
    let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    manifest
        .as_object_mut()
        .expect("manifest is an object")
        .insert("manifest_digest".into(), json!(digest));

    let compression = PackCompression::from_deflate_flag(deflate);
    let compression_profile = CompressionProfile::default();
    let mut central = Vec::new();
    let mut written = 0u64;
    let mut zip64_used = false;
    let mut file_count = 0usize;
    for (entry, expectation) in entries.iter().zip(expectations.iter()) {
        match entry {
            OrderedStreamingPackEntry::Borrowed(entry) => write_zip_file(
                writer,
                &mut central,
                &mut written,
                &mut zip64_used,
                &entry.path,
                entry.bytes,
                compression.method_for_entry(entry.bytes.len() as u64),
                compression_profile,
            )?,
            OrderedStreamingPackEntry::File(entry) => {
                let (expected_sha256, expected_len) = expectation
                    .as_ref()
                    .expect("file-backed entry has an expectation");
                write_zip_file_streaming(
                    writer,
                    &mut central,
                    &mut written,
                    &mut zip64_used,
                    &entry.path,
                    entry,
                    compression,
                    compression_profile,
                    expected_sha256,
                    *expected_len,
                )?;
            }
            OrderedStreamingPackEntry::Verified(entry) => {
                let (expected_sha256, expected_len) = expectation
                    .as_ref()
                    .expect("verified entry has an expectation");
                write_zip_file_from_callback(
                    writer,
                    &mut central,
                    &mut written,
                    &mut zip64_used,
                    &entry.path,
                    expected_sha256,
                    *expected_len,
                    |sink| copy_verified_source(entry.source_id, sink),
                )?;
            }
        }
        file_count += 1;
    }
    if let Some((path, data)) = &opts.inject_stowaway {
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            path,
            data,
            ZipCompression::Store,
            compression_profile,
        )?;
        file_count += 1;
    }
    if !opts.omit_manifest {
        let mut manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
        if opts.inject_duplicate_role_member {
            inject_duplicate_role_member(&mut manifest_bytes);
        }
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            "manifest.json",
            &manifest_bytes,
            ZipCompression::Store,
            compression_profile,
        )?;
        file_count += 1;
    }
    finish_zip(writer, &mut central, written, file_count, zip64_used)?;
    Ok(())
}

/// Production streaming writer: STORE tiny entries and Zstandard-compress large
/// compact-stream / snapshot JSON entries using method 93.
pub fn pack_streaming_zstd_to_writer<W: Write>(
    game_id: &str,
    borrowed_entries: &[BorrowedPackEntry<'_>],
    file_entries: &[FileBackedPackEntry<'_>],
    streaming_entries: &[StreamingFileEntry<'_>],
    identity: &PackIdentity,
    writer: &mut W,
) -> Result<(), PackError> {
    pack_streaming_zstd_to_writer_with_profile(
        game_id,
        borrowed_entries,
        file_entries,
        streaming_entries,
        identity,
        CompressionProfile::default(),
        writer,
    )
}

/// Production streaming writer with an explicit zstd compression profile.
pub fn pack_streaming_zstd_to_writer_with_profile<W: Write>(
    game_id: &str,
    borrowed_entries: &[BorrowedPackEntry<'_>],
    file_entries: &[FileBackedPackEntry<'_>],
    streaming_entries: &[StreamingFileEntry<'_>],
    identity: &PackIdentity,
    compression_profile: CompressionProfile,
    writer: &mut W,
) -> Result<(), PackError> {
    pack_streaming_to_writer_with_compression(
        game_id,
        borrowed_entries,
        file_entries,
        streaming_entries,
        identity,
        PackCompression::ZstdLarge,
        compression_profile,
        writer,
    )
}

fn pack_streaming_to_writer_with_compression<W: Write>(
    game_id: &str,
    borrowed_entries: &[BorrowedPackEntry<'_>],
    file_entries: &[FileBackedPackEntry<'_>],
    streaming_entries: &[StreamingFileEntry<'_>],
    identity: &PackIdentity,
    compression: PackCompression,
    compression_profile: CompressionProfile,
    writer: &mut W,
) -> Result<(), PackError> {
    if compression == PackCompression::ZstdLarge {
        compression_profile.validate()?;
    }
    planned_zip_file_count(
        &[
            borrowed_entries.len(),
            file_entries.len(),
            streaming_entries.len(),
        ],
        1,
    )?;
    validate_distinct_paths(
        borrowed_entries
            .iter()
            .map(|entry| entry.path.as_str())
            .chain(file_entries.iter().map(|entry| entry.path.as_str()))
            .chain(streaming_entries.iter().map(|entry| entry.path.as_str())),
    )?;

    let mut manifest_entries: Vec<Value> =
        Vec::with_capacity(borrowed_entries.len() + file_entries.len() + streaming_entries.len());
    let mut counts_snapshots = 0u64;
    let mut file_expectations = Vec::with_capacity(file_entries.len());
    let mut streaming_expectations = Vec::with_capacity(streaming_entries.len());

    let mut push_manifest_entry =
        |path: &str, sha: String, bytes: u64, role: &str, hour_index: Option<i64>| {
            let mut ent = json!({
                "path": path,
                "sha256": sha,
                "bytes": bytes,
                "role": role,
            });
            if let Some(h) = hour_index {
                ent.as_object_mut()
                    .unwrap()
                    .insert("hour_index".into(), json!(h));
            }
            if role == "snapshot" {
                counts_snapshots += 1;
            }
            manifest_entries.push(ent);
        };

    for e in borrowed_entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let sha = to_hex(&Sha256::digest(e.bytes));
        push_manifest_entry(&e.path, sha, e.bytes.len() as u64, e.role, e.hour_index);
    }
    for e in file_entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let (sha, len) = sha256_file(e.file_path)?;
        file_expectations.push((sha.clone(), len));
        push_manifest_entry(&e.path, sha, len, e.role, e.hour_index);
    }
    for e in streaming_entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let (sha, len) = match &e.sha256 {
            Some(sha) => (sha.clone(), streaming_entry_decoded_len(e)?),
            None => sha256_streaming_entry(e)?,
        };
        streaming_expectations.push((sha.clone(), len));
        push_manifest_entry(&e.path, sha, len, e.role, e.hour_index);
    }

    manifest_entries.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or("")
            .cmp(b["path"].as_str().unwrap_or(""))
    });

    let content_manifest = portable_content_manifest_value(identity)?;
    let required_features = portable_content_required_features(identity, Vec::new());
    let mut manifest = json!({
        "format": "hoi4replay",
        "container_version": 1,
        "required_features": required_features,
        "optional_features": [],
        "schema_version": identity.schema_version,
        "game_id": game_id,
        "build": {
            "exe_sha256": identity.build_exe_sha256,
            "pinned": identity.build_pinned,
        },
        "mod_identity": identity.mod_identity.clone().unwrap_or(Value::Null),
        "content_manifest": content_manifest,
        "created_utc": identity.created_utc,
        "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
        "counts": { "snapshots": counts_snapshots },
        "entries": manifest_entries,
        "manifest_digest": Value::Null,
        "signature": Value::Null,
    });
    let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    manifest
        .as_object_mut()
        .unwrap()
        .insert("manifest_digest".into(), json!(digest));

    let mut central = Vec::new();
    let mut written = 0u64;
    let mut zip64_used = false;
    let mut file_count = 0usize;
    for e in borrowed_entries {
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e.bytes,
            compression.method_for_entry(e.bytes.len() as u64),
            compression_profile,
        )?;
        file_count += 1;
    }
    for (e, (expected_sha256, expected_len)) in
        file_entries.iter().zip(file_expectations.iter())
    {
        write_zip_file_from_path(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e.file_path,
            compression,
            compression_profile,
            expected_sha256,
            *expected_len,
        )?;
        file_count += 1;
    }
    for (e, (expected_sha256, expected_len)) in streaming_entries
        .iter()
        .zip(streaming_expectations.iter())
    {
        write_zip_file_streaming(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e,
            compression,
            compression_profile,
            expected_sha256,
            *expected_len,
        )?;
        file_count += 1;
    }

    let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
    write_zip_file(
        writer,
        &mut central,
        &mut written,
        &mut zip64_used,
        "manifest.json",
        &manifest_bytes,
        ZipCompression::Store,
        compression_profile,
    )?;
    file_count += 1;
    finish_zip(writer, &mut central, written, file_count, zip64_used)?;
    Ok(())
}

/// As [`pack`] but with [`PackOptions`] for negative-test fixtures.
pub fn pack_with(
    game_id: &str,
    entries: &[PackEntry],
    identity: &PackIdentity,
    deflate: bool,
    opts: &PackOptions,
) -> Result<Vec<u8>, PackError> {
    let borrowed_entries = entries
        .iter()
        .map(|entry| BorrowedPackEntry {
            path: entry.path.clone(),
            bytes: &entry.bytes,
            role: entry.role,
            hour_index: entry.hour_index,
        })
        .collect::<Vec<_>>();
    pack_borrowed_with(game_id, &borrowed_entries, identity, deflate, opts)
}

/// As [`pack_borrowed`] but with [`PackOptions`] for negative-test fixtures.
pub fn pack_borrowed_with(
    game_id: &str,
    entries: &[BorrowedPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    opts: &PackOptions,
) -> Result<Vec<u8>, PackError> {
    let mut out = Vec::new();
    pack_borrowed_with_writer(game_id, entries, identity, deflate, opts, &mut out)?;
    Ok(out)
}

/// As [`pack_borrowed_to_writer`] but with [`PackOptions`] for negative-test
/// fixtures.
pub fn pack_borrowed_with_writer<W: Write>(
    game_id: &str,
    entries: &[BorrowedPackEntry<'_>],
    identity: &PackIdentity,
    deflate: bool,
    opts: &PackOptions,
    writer: &mut W,
) -> Result<(), PackError> {
    planned_zip_file_count(
        &[entries.len()],
        (if opts.inject_stowaway.is_some() { 1 } else { 0 })
            + (if !opts.omit_manifest { 1 } else { 0 }),
    )?;
    validate_distinct_paths(entries.iter().map(|entry| entry.path.as_str()))?;

    // --- manifest entries (sha + bytes), in canonical path order ---
    let mut manifest_entries: Vec<Value> = Vec::with_capacity(entries.len());

    let mut counts_snapshots = 0u64;
    for e in entries {
        validate_payload_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let sha = to_hex(&Sha256::digest(&e.bytes));
        let mut ent = json!({
            "path": e.path,
            "sha256": sha,
            "bytes": e.bytes.len() as u64,
            "role": e.role,
        });
        if let Some(h) = e.hour_index {
            ent.as_object_mut()
                .unwrap()
                .insert("hour_index".into(), json!(h));
        }
        if e.role == "snapshot" {
            counts_snapshots += 1;
        }
        manifest_entries.push(ent);
    }
    // canonical path order so the manifest entry list is stable
    manifest_entries.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or("")
            .cmp(b["path"].as_str().unwrap_or(""))
    });
    if let (Some(raw), Some(first)) = (&opts.first_role_raw, manifest_entries.first_mut()) {
        first
            .as_object_mut()
            .expect("manifest entry object")
            .insert("role".into(), raw.clone());
    }

    // --- the manifest object (digest filled in below) ---
    let required_features = match (&opts.required_features_raw, &opts.inject_required_feature) {
        (Some(raw), _) => raw.clone(),
        (None, Some(f)) => json!([f]),
        (None, None) => json!(portable_content_required_features(
            identity,
            opts.required_features.clone().unwrap_or_default(),
        )),
    };
    let container_version = opts.container_version.unwrap_or(1);
    let content_manifest = portable_content_manifest_value(identity)?;
    let mut manifest = json!({
        "format": "hoi4replay",
        "container_version": container_version,
        "required_features": required_features,
        "optional_features": [],
        "schema_version": identity.schema_version,
        "game_id": game_id,
        "build": {
            "exe_sha256": identity.build_exe_sha256,
            "pinned": identity.build_pinned,
        },
        "mod_identity": identity.mod_identity.clone().unwrap_or(Value::Null),
        "content_manifest": content_manifest,
        "created_utc": identity.created_utc,
        "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
        "counts": { "snapshots": counts_snapshots },
        "entries": manifest_entries,
        "manifest_digest": Value::Null,
        "signature": Value::Null,
    });
    let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    manifest
        .as_object_mut()
        .unwrap()
        .insert("manifest_digest".into(), json!(digest));

    let mut central = Vec::new();
    let mut written = 0u64;
    let mut zip64_used = false;
    let mut file_count = 0usize;
    let compression = PackCompression::from_deflate_flag(deflate);
    for e in entries {
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            &e.path,
            e.bytes,
            compression.method_for_entry(e.bytes.len() as u64),
            CompressionProfile::default(),
        )?;
        file_count += 1;
    }

    // inject a stowaway (a zip entry the manifest does not list) BEFORE the
    // manifest, so the reader's coverage check sees an uncovered entry.
    if let Some((path, data)) = &opts.inject_stowaway {
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            path,
            data,
            ZipCompression::Store,
            CompressionProfile::default(),
        )?;
        file_count += 1;
    }

    // manifest.json is written LAST (spec §6). The omit-manifest test fixture
    // produces a recovery-only file (no manifest) to exercise the NoManifest path.
    if !opts.omit_manifest {
        let mut manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
        if opts.inject_duplicate_role_member {
            inject_duplicate_role_member(&mut manifest_bytes);
        }
        write_zip_file(
            writer,
            &mut central,
            &mut written,
            &mut zip64_used,
            "manifest.json",
            &manifest_bytes,
            ZipCompression::Store,
            CompressionProfile::default(),
        )?;
        file_count += 1;
    }

    finish_zip(writer, &mut central, written, file_count, zip64_used)?;
    Ok(())
}

fn inject_duplicate_role_member(bytes: &mut Vec<u8>) {
    let marker = br#""role":""#;
    let start = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("test fixture manifest has a role member")
        + marker.len();
    let end = bytes[start..]
        .iter()
        .position(|byte| *byte == b'"')
        .map(|offset| start + offset)
        .expect("test fixture role string terminates");
    let role = bytes[start..end].to_vec();
    let mut duplicate = br#","role":""#.to_vec();
    duplicate.extend_from_slice(&role);
    duplicate.push(b'"');
    bytes.splice(end + 1..end + 1, duplicate);
}

// --------------------------------------------------------------------------
// minimal ZIP writer (STORE, DEFLATE, or ZSTD; CRC32; Zip64; deterministic)
// --------------------------------------------------------------------------
fn zip_limit_err(detail: impl AsRef<str>) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("zip container limit exceeded for {}", detail.as_ref()),
    )
}

fn checked_zip_u16_len(value: usize, field: &str, name: &str) -> std::io::Result<u16> {
    if value > usize::from(u16::MAX) {
        return Err(zip_limit_err(format!(
            "{field} for {name} ({value} > {})",
            u16::MAX
        )));
    }
    Ok(value as u16)
}

fn ensure_entry_size_within_limit(
    name: &str,
    field: &str,
    value: u64,
    limit: u64,
) -> std::io::Result<()> {
    if value > limit {
        return Err(zip_limit_err(format!(
            "entry {name} {field} ({value} > per-entry reader cap {limit})"
        )));
    }
    Ok(())
}

fn planned_zip_file_count(parts: &[usize], extra: usize) -> std::io::Result<usize> {
    let mut total = extra;
    for part in parts {
        total = total
            .checked_add(*part)
            .ok_or_else(|| zip_limit_err("entry count overflow"))?;
    }
    Ok(total)
}

fn streaming_entry_decoded_len(entry: &StreamingFileEntry<'_>) -> std::io::Result<u64> {
    if let Some(len) = entry.decoded_len {
        return Ok(len);
    }
    if entry.source_compression == compact_stream::StreamCompression::None {
        return Ok(fs::metadata(entry.file_path)?.len());
    }
    let mut reader =
        compact_stream::open_with_compression(entry.file_path, entry.source_compression)?;
    std::io::copy(&mut reader, &mut std::io::sink())
}

fn sha256_streaming_entry(entry: &StreamingFileEntry<'_>) -> std::io::Result<(String, u64)> {
    let mut reader =
        compact_stream::open_with_compression(entry.file_path, entry.source_compression)?;
    let mut hasher = Sha256::new();
    let mut len = 0u64;
    let mut buf = vec![0u8; STREAM_CHUNK];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len = len.saturating_add(n as u64);
    }
    Ok((to_hex(&hasher.finalize()), len))
}

fn sha256_file(path: &Path) -> Result<(String, u64), PackError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut len = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len = len.saturating_add(n as u64);
    }
    Ok((to_hex(&hasher.finalize()), len))
}

fn write_zip_file_from_path<W: Write>(
    out: &mut W,
    central: &mut Vec<u8>,
    written: &mut u64,
    zip64_used: &mut bool,
    name: &str,
    path: &Path,
    compression: PackCompression,
    compression_profile: CompressionProfile,
    expected_sha256: &str,
    expected_len: u64,
) -> std::io::Result<()> {
    let entry = StreamingFileEntry {
        path: name.to_string(),
        file_path: path,
        source_compression: compact_stream::StreamCompression::None,
        decoded_len: None,
        role: "",
        hour_index: None,
        sha256: None,
    };
    write_zip_file_streaming_with_entry_limit(
        out,
        central,
        written,
        zip64_used,
        name,
        &entry,
        compression,
        compression_profile,
        MAX_CONTAINER_ENTRY_BYTES,
        expected_sha256,
        expected_len,
    )
}

/// A `Write` adapter that counts the bytes passed through it. Used to measure the
/// compressed size of a streamed entry without buffering it, since the compressor
/// encoder emits directly to the output file.
struct CountingWriter<'a, W: Write> {
    inner: &'a mut W,
    count: u64,
    name: &'a str,
    limit: u64,
}
impl<W: Write> Write for CountingWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.count.checked_add(buf.len() as u64).ok_or_else(|| {
            zip_limit_err(format!("streaming entry {} compressed size", self.name))
        })?;
        ensure_entry_size_within_limit(self.name, "compressed size", next, self.limit)?;
        let n = self.inner.write(buf)?;
        self.count = self.count.saturating_add(n as u64);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Chunk size for streaming a large file entry into the container.
const STREAM_CHUNK: usize = 4 * 1024 * 1024;

struct VerifiedStoreWriter<'a, W: Write> {
    out: &'a mut W,
    crc32: crc32fast::Hasher,
    sha256: Sha256,
    len: u64,
    name: &'a str,
}

impl<W: Write> Write for VerifiedStoreWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .len
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| zip_limit_err(format!("verified entry {} size", self.name)))?;
        ensure_entry_size_within_limit(
            self.name,
            "uncompressed size",
            next,
            STREAMING_MAX_CONTAINER_ENTRY_BYTES,
        )?;
        self.out.write_all(bytes)?;
        self.crc32.update(bytes);
        self.sha256.update(bytes);
        self.len = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// Append one authenticated callback source as STORE without opening a path or
/// materializing its bytes. The callback writes only into the unpublished ZIP
/// sink; length and digest are checked before the manifest/central directory is
/// finalized, so any retained-source drift aborts publication.
fn write_zip_file_from_callback<W, F>(
    out: &mut W,
    central: &mut Vec<u8>,
    written: &mut u64,
    zip64_used: &mut bool,
    name: &str,
    expected_sha256: &str,
    expected_len: u64,
    copy_source: F,
) -> std::io::Result<()>
where
    W: Write,
    F: FnOnce(&mut dyn Write) -> std::io::Result<()>,
{
    ensure_entry_size_within_limit(
        name,
        "uncompressed size",
        expected_len,
        STREAMING_MAX_CONTAINER_ENTRY_BYTES,
    )?;
    let local_header_offset = *written;
    let method = ZIP_METHOD_STORE;
    let flags = 0x0008u16;
    let name_len = checked_zip_u16_len(name.len(), "entry name length", name)?;
    let version = version_needed(
        method,
        zip64_field_needed(local_header_offset) || zip64_field_needed(expected_len),
    );

    write_all_counted(out, written, &0x0403_4b50u32.to_le_bytes())?;
    write_all_counted(out, written, &version.to_le_bytes())?;
    write_all_counted(out, written, &flags.to_le_bytes())?;
    write_all_counted(out, written, &method.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?;
    write_all_counted(out, written, &0u32.to_le_bytes())?;
    write_all_counted(out, written, &0u32.to_le_bytes())?;
    write_all_counted(out, written, &0u32.to_le_bytes())?;
    write_all_counted(out, written, &name_len.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?;
    write_all_counted(out, written, name.as_bytes())?;

    let mut sink = VerifiedStoreWriter {
        out,
        crc32: crc32fast::Hasher::new(),
        sha256: Sha256::new(),
        len: 0,
        name,
    };
    copy_source(&mut sink)?;
    let VerifiedStoreWriter {
        out: _,
        crc32,
        sha256,
        len: data_len,
        name: _,
    } = sink;
    let actual_sha256 = to_hex(&sha256.finalize());
    let crc32 = crc32.finalize();
    if data_len != expected_len || actual_sha256 != expected_sha256 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "entry {name} changed while packing: expected {expected_len} bytes sha256 {expected_sha256}, wrote {data_len} bytes sha256 {actual_sha256}"
            ),
        ));
    }
    *written = written
        .checked_add(data_len)
        .ok_or_else(|| zip_limit_err(format!("verified entry {name} size overflow")))?;
    write_all_counted(out, written, &0x0807_4b50u32.to_le_bytes())?;
    write_all_counted(out, written, &crc32.to_le_bytes())?;
    if zip64_field_needed(data_len) {
        write_all_counted(out, written, &data_len.to_le_bytes())?;
        write_all_counted(out, written, &data_len.to_le_bytes())?;
    } else {
        write_all_counted(out, written, &(data_len as u32).to_le_bytes())?;
        write_all_counted(out, written, &(data_len as u32).to_le_bytes())?;
    }
    *zip64_used |= append_central_directory_record(
        central,
        name,
        method,
        flags,
        crc32,
        data_len,
        data_len,
        local_header_offset,
        version,
    )?;
    Ok(())
}

/// Append a file to the zip in bounded memory: the local header is written with a
/// data-descriptor flag (crc/sizes deferred), the payload is streamed through the
/// configured encoder (or STORE) directly to `out` while the crc32 and lengths
/// are computed incrementally, a trailing data descriptor is written, and the
/// central directory record carries the TRUE crc/sizes (the authority our reader
/// trusts). Streaming entries use the larger streaming cap; the in-RAM reader
/// still rejects them if they exceed 2 GiB.
fn write_zip_file_streaming<W: Write>(
    out: &mut W,
    central: &mut Vec<u8>,
    written: &mut u64,
    zip64_used: &mut bool,
    name: &str,
    entry: &StreamingFileEntry<'_>,
    compression: PackCompression,
    compression_profile: CompressionProfile,
    expected_sha256: &str,
    expected_len: u64,
) -> std::io::Result<()> {
    write_zip_file_streaming_with_entry_limit(
        out,
        central,
        written,
        zip64_used,
        name,
        entry,
        compression,
        compression_profile,
        STREAMING_MAX_CONTAINER_ENTRY_BYTES,
        expected_sha256,
        expected_len,
    )
}

fn write_zip_file_streaming_with_entry_limit<W: Write>(
    out: &mut W,
    central: &mut Vec<u8>,
    written: &mut u64,
    zip64_used: &mut bool,
    name: &str,
    entry: &StreamingFileEntry<'_>,
    compression: PackCompression,
    compression_profile: CompressionProfile,
    entry_limit: u64,
    expected_sha256: &str,
    expected_len: u64,
) -> std::io::Result<()> {
    let lho = *written;
    // The manifest identity was derived before this second/open-and-write
    // pass. Use that expected length for compression decisions and then prove
    // the exact decoded bytes written still have the same length and digest.
    // This closes the mutable-path window without retaining whole entries.
    let file_len = expected_len;
    let zip_compression = compression.method_for_entry(file_len);
    let method = zip_compression.method();
    // general-purpose bit 3: crc-32 and sizes are in a trailing data descriptor.
    let flags: u16 = 0x0008;
    let name_len = checked_zip_u16_len(name.len(), "entry name length", name)?;
    // Compressed size is not known until after the local header is emitted.
    // Advertise ZIP64 conservatively for compressed streaming entries; STORE
    // can decide exactly from its decoded/stored length.
    let version = version_needed(
        method,
        zip64_field_needed(lho)
            || zip64_field_needed(expected_len)
            || !matches!(zip_compression, ZipCompression::Store),
    );

    // --- local file header (crc + sizes zeroed; real values in descriptor + CD) ---
    write_all_counted(out, written, &0x0403_4b50u32.to_le_bytes())?; // sig
    write_all_counted(out, written, &version.to_le_bytes())?; // version needed
    write_all_counted(out, written, &flags.to_le_bytes())?;
    write_all_counted(out, written, &method.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?; // mod time
    write_all_counted(out, written, &0u16.to_le_bytes())?; // mod date
    write_all_counted(out, written, &0u32.to_le_bytes())?; // crc (deferred)
    write_all_counted(out, written, &0u32.to_le_bytes())?; // comp size (deferred)
    write_all_counted(out, written, &0u32.to_le_bytes())?; // uncomp size (deferred)
    write_all_counted(out, written, &name_len.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?; // extra len
    write_all_counted(out, written, name.as_bytes())?;

    // --- stream the payload, hashing + measuring as we go ---
    let mut file =
        compact_stream::open_with_compression(entry.file_path, entry.source_compression)?;
    let mut hasher = crc32fast::Hasher::new();
    let mut sha256 = Sha256::new();
    let mut data_len: u64 = 0;
    let mut buf = vec![0u8; STREAM_CHUNK];
    let comp_len: u64 = match zip_compression {
        ZipCompression::Store => {
            let mut n_total: u64 = 0;
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let next_len = data_len.checked_add(n as u64).ok_or_else(|| {
                    zip_limit_err(format!("streaming entry {name} uncompressed size"))
                })?;
                ensure_entry_size_within_limit(name, "uncompressed size", next_len, entry_limit)?;
                hasher.update(&buf[..n]);
                sha256.update(&buf[..n]);
                data_len = next_len;
                ensure_entry_size_within_limit(name, "compressed size", data_len, entry_limit)?;
                out.write_all(&buf[..n])?;
                n_total = data_len;
            }
            n_total
        }
        ZipCompression::Deflate => {
            let mut cw = CountingWriter {
                inner: out,
                count: 0,
                name,
                limit: entry_limit,
            };
            {
                let mut enc = DeflateEncoder::new(&mut cw, Compression::new(6));
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    let next_len = data_len.checked_add(n as u64).ok_or_else(|| {
                        zip_limit_err(format!("streaming entry {name} uncompressed size"))
                    })?;
                    ensure_entry_size_within_limit(
                        name,
                        "uncompressed size",
                        next_len,
                        entry_limit,
                    )?;
                    hasher.update(&buf[..n]);
                    sha256.update(&buf[..n]);
                    data_len = next_len;
                    enc.write_all(&buf[..n])?;
                }
                enc.finish()?;
            }
            cw.count
        }
        ZipCompression::Zstd => {
            let mut cw = CountingWriter {
                inner: out,
                count: 0,
                name,
                limit: entry_limit,
            };
            {
                let mut enc = configured_zstd_encoder(&mut cw, compression_profile)?;
                enc.set_pledged_src_size(Some(file_len))?;
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    let next_len = data_len.checked_add(n as u64).ok_or_else(|| {
                        zip_limit_err(format!("streaming entry {name} uncompressed size"))
                    })?;
                    ensure_entry_size_within_limit(
                        name,
                        "uncompressed size",
                        next_len,
                        entry_limit,
                    )?;
                    hasher.update(&buf[..n]);
                    sha256.update(&buf[..n]);
                    data_len = next_len;
                    enc.write_all(&buf[..n])?;
                }
                enc.finish()?;
            }
            cw.count
        }
    };
    let actual_sha256 = to_hex(&sha256.finalize());
    if data_len != expected_len || actual_sha256.as_str() != expected_sha256 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "entry {name} changed while packing: expected {expected_len} bytes sha256 {expected_sha256}, wrote {data_len} bytes sha256 {actual_sha256}"
            ),
        ));
    }
    *written = written
        .checked_add(comp_len)
        .ok_or_else(|| zip_limit_err(format!("streaming entry {name} compressed size overflow")))?;
    let crc = hasher.finalize();

    // --- data descriptor (with signature, per APPNOTE 4.3.9.3) ---
    write_all_counted(out, written, &0x0807_4b50u32.to_le_bytes())?; // dd sig
    write_all_counted(out, written, &crc.to_le_bytes())?;
    if zip64_field_needed(comp_len) || zip64_field_needed(data_len) {
        write_all_counted(out, written, &comp_len.to_le_bytes())?;
        write_all_counted(out, written, &data_len.to_le_bytes())?;
    } else {
        write_all_counted(out, written, &(comp_len as u32).to_le_bytes())?;
        write_all_counted(out, written, &(data_len as u32).to_le_bytes())?;
    }

    // --- central directory record (true crc/sizes: the reader's authority) ---
    let entry_zip64 = append_central_directory_record(
        central, name, method, flags, crc, comp_len, data_len, lho, version,
    )?;
    *zip64_used |= entry_zip64;
    Ok(())
}

fn write_zip_file<W: Write>(
    out: &mut W,
    central: &mut Vec<u8>,
    written: &mut u64,
    zip64_used: &mut bool,
    name: &str,
    data: &[u8],
    zip_compression: ZipCompression,
    compression_profile: CompressionProfile,
) -> std::io::Result<()> {
    let lho = *written;
    let data_len = data.len() as u64;
    ensure_entry_size_within_limit(
        name,
        "uncompressed size",
        data_len,
        MAX_CONTAINER_ENTRY_BYTES,
    )?;
    let name_len = checked_zip_u16_len(name.len(), "entry name length", name)?;
    let crc = crc32fast::hash(data);
    let (method, comp): (u16, Cow<'_, [u8]>) = match zip_compression {
        ZipCompression::Store => (ZIP_METHOD_STORE, Cow::Borrowed(data)),
        ZipCompression::Deflate => {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
            enc.write_all(data)?;
            (ZIP_METHOD_DEFLATE, Cow::Owned(enc.finish()?))
        }
        ZipCompression::Zstd => {
            let mut enc = configured_zstd_encoder(Vec::new(), compression_profile)?;
            enc.set_pledged_src_size(Some(data_len))?;
            enc.write_all(data)?;
            (ZIP_METHOD_ZSTD, Cow::Owned(enc.finish()?))
        }
    };
    let comp_len = comp.len() as u64;
    ensure_entry_size_within_limit(name, "compressed size", comp_len, MAX_CONTAINER_ENTRY_BYTES)?;
    let entry_zip64 =
        zip64_field_needed(data_len) || zip64_field_needed(comp_len) || zip64_field_needed(lho);
    let local_extra = zip64_extra(
        zip64_field_needed(data_len).then_some(data_len),
        zip64_field_needed(comp_len).then_some(comp_len),
        None,
    )?;
    let version = version_needed(method, entry_zip64);

    write_all_counted(out, written, &0x0403_4b50u32.to_le_bytes())?; // sig
    write_all_counted(out, written, &version.to_le_bytes())?; // version needed
    write_all_counted(out, written, &0u16.to_le_bytes())?; // flags
    write_all_counted(out, written, &method.to_le_bytes())?;
    write_all_counted(out, written, &0u16.to_le_bytes())?; // mod time
    write_all_counted(out, written, &0u16.to_le_bytes())?; // mod date
    write_all_counted(out, written, &crc.to_le_bytes())?;
    write_all_counted(out, written, &zip32_field(comp_len).to_le_bytes())?;
    write_all_counted(out, written, &zip32_field(data_len).to_le_bytes())?;
    write_all_counted(out, written, &name_len.to_le_bytes())?;
    write_all_counted(
        out,
        written,
        &checked_zip_u16_len(local_extra.len(), "local extra length", name)?.to_le_bytes(),
    )?;
    write_all_counted(out, written, name.as_bytes())?;
    write_all_counted(out, written, &local_extra)?;
    write_all_counted(out, written, &comp)?;

    *zip64_used |= append_central_directory_record(
        central, name, method, 0, crc, comp_len, data_len, lho, version,
    )?;
    Ok(())
}

fn finish_zip<W: Write>(
    out: &mut W,
    central: &mut Vec<u8>,
    mut written: u64,
    file_count: usize,
    zip64_used: bool,
) -> std::io::Result<()> {
    let cd_off = written;
    let cd_size = central.len() as u64;
    let needs_zip64 = zip64_used
        || file_count > ZIP32_MAX_ENTRY_COUNT
        || zip64_field_needed(cd_off)
        || zip64_field_needed(cd_size);
    write_all_counted(out, &mut written, central)?;

    if needs_zip64 {
        let zip64_eocd_off = written;
        write_all_counted(out, &mut written, &0x0606_4b50u32.to_le_bytes())?;
        write_all_counted(out, &mut written, &44u64.to_le_bytes())?;
        write_all_counted(out, &mut written, &45u16.to_le_bytes())?; // version made by
        write_all_counted(out, &mut written, &45u16.to_le_bytes())?; // version needed
        write_all_counted(out, &mut written, &0u32.to_le_bytes())?; // disk
        write_all_counted(out, &mut written, &0u32.to_le_bytes())?; // central dir disk
        write_all_counted(out, &mut written, &(file_count as u64).to_le_bytes())?;
        write_all_counted(out, &mut written, &(file_count as u64).to_le_bytes())?;
        write_all_counted(out, &mut written, &cd_size.to_le_bytes())?;
        write_all_counted(out, &mut written, &cd_off.to_le_bytes())?;

        write_all_counted(out, &mut written, &0x0706_4b50u32.to_le_bytes())?;
        write_all_counted(out, &mut written, &0u32.to_le_bytes())?; // zip64 eocd disk
        write_all_counted(out, &mut written, &zip64_eocd_off.to_le_bytes())?;
        write_all_counted(out, &mut written, &1u32.to_le_bytes())?; // total disks
    }

    // end of central directory
    write_all_counted(out, &mut written, &0x0605_4b50u32.to_le_bytes())?;
    write_all_counted(out, &mut written, &0u16.to_le_bytes())?; // disk
    write_all_counted(out, &mut written, &0u16.to_le_bytes())?; // cd disk
    let count16 = if file_count > ZIP32_MAX_ENTRY_COUNT {
        u16::MAX
    } else {
        file_count as u16
    };
    write_all_counted(out, &mut written, &count16.to_le_bytes())?;
    write_all_counted(out, &mut written, &count16.to_le_bytes())?;
    write_all_counted(out, &mut written, &zip32_field(cd_size).to_le_bytes())?;
    write_all_counted(out, &mut written, &zip32_field(cd_off).to_le_bytes())?;
    write_all_counted(out, &mut written, &0u16.to_le_bytes())?; // comment len
    Ok(())
}

fn configured_zstd_encoder<W: Write>(
    writer: W,
    profile: CompressionProfile,
) -> std::io::Result<zstd::stream::write::Encoder<'static, W>> {
    profile.validate()?;
    let mut enc = zstd::stream::write::Encoder::new(writer, profile.zstd_level)?;
    enc.long_distance_matching(true)?;
    enc.window_log(profile.zstd_window_log)?;
    enc.include_checksum(true)?;
    Ok(enc)
}

fn zip64_field_needed(value: u64) -> bool {
    value > ZIP32_MAX_FIELD_VALUE
}

fn zip32_field(value: u64) -> u32 {
    if zip64_field_needed(value) {
        u32::MAX
    } else {
        value as u32
    }
}

fn version_needed(method: u16, uses_zip64: bool) -> u16 {
    if method == ZIP_METHOD_ZSTD {
        63
    } else if uses_zip64 {
        45
    } else {
        20
    }
}

fn zip64_extra(
    uncomp_size: Option<u64>,
    comp_size: Option<u64>,
    local_header_off: Option<u64>,
) -> std::io::Result<Vec<u8>> {
    if uncomp_size.is_none() && comp_size.is_none() && local_header_off.is_none() {
        return Ok(Vec::new());
    }
    let mut payload = Vec::new();
    if let Some(v) = uncomp_size {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = comp_size {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    if let Some(v) = local_header_off {
        payload.extend_from_slice(&v.to_le_bytes());
    }
    let payload_len = checked_zip_u16_len(payload.len(), "zip64 extra length", "zip64 extra")?;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&0x0001u16.to_le_bytes());
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn append_central_directory_record(
    central: &mut Vec<u8>,
    name: &str,
    method: u16,
    flags: u16,
    crc: u32,
    comp_len: u64,
    data_len: u64,
    local_header_off: u64,
    minimum_version: u16,
) -> std::io::Result<bool> {
    let name_len = checked_zip_u16_len(name.len(), "entry name length", name)?;
    let need_uncomp = zip64_field_needed(data_len);
    let need_comp = zip64_field_needed(comp_len);
    let need_lho = zip64_field_needed(local_header_off);
    let extra = zip64_extra(
        need_uncomp.then_some(data_len),
        need_comp.then_some(comp_len),
        need_lho.then_some(local_header_off),
    )?;
    let extra_len = checked_zip_u16_len(extra.len(), "central extra length", name)?;
    let uses_zip64 = need_uncomp || need_comp || need_lho;
    let version = version_needed(method, uses_zip64).max(minimum_version);

    central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    central.extend_from_slice(&version.to_le_bytes()); // version made by
    central.extend_from_slice(&version.to_le_bytes()); // version needed
    central.extend_from_slice(&flags.to_le_bytes());
    central.extend_from_slice(&method.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes()); // time
    central.extend_from_slice(&0u16.to_le_bytes()); // date
    central.extend_from_slice(&crc.to_le_bytes());
    central.extend_from_slice(&zip32_field(comp_len).to_le_bytes());
    central.extend_from_slice(&zip32_field(data_len).to_le_bytes());
    central.extend_from_slice(&name_len.to_le_bytes());
    central.extend_from_slice(&extra_len.to_le_bytes());
    central.extend_from_slice(&0u16.to_le_bytes()); // comment
    central.extend_from_slice(&0u16.to_le_bytes()); // disk no
    central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
    central.extend_from_slice(&zip32_field(local_header_off).to_le_bytes());
    central.extend_from_slice(name.as_bytes());
    central.extend_from_slice(&extra);
    Ok(uses_zip64)
}

fn write_all_counted<W: Write>(
    out: &mut W,
    written: &mut u64,
    bytes: &[u8],
) -> std::io::Result<()> {
    out.write_all(bytes)?;
    *written = written
        .checked_add(bytes.len() as u64)
        .ok_or_else(|| zip_limit_err("archive byte count overflow"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::{
        ContentLayerKind, ContentManifest, DlcSelection, LayerStorageKind, RecordedContentLayer,
        RequiredContent, RequiredContentKind, SourceGameIdentity, SourceProfileIdentity,
        UsedContentKey,
    };
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "hoi4_pack_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct ChunkGuardWriter {
        bytes: Vec<u8>,
        max_write: usize,
    }
    impl ChunkGuardWriter {
        fn new() -> ChunkGuardWriter {
            ChunkGuardWriter {
                bytes: Vec::new(),
                max_write: 0,
            }
        }
    }
    impl Write for ChunkGuardWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.max_write = self.max_write.max(buf.len());
            if buf.len() > STREAM_CHUNK {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("write larger than STREAM_CHUNK: {}", buf.len()),
                ));
            }
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len())
            .position(|window| window == needle)
    }

    #[test]
    fn portable_content_manifest_is_embedded_and_open_validated() {
        let hash = |ch: char| ch.to_string().repeat(64);
        let mut content = ContentManifest {
            schema: portable_content_manifest::SCHEMA.into(),
            product_modset_sha256: hash('9'),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: hash('a'),
                build_profile: "a729-870ebf".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "capture-profile-label".into(),
                playset_id: "capture-playset-label".into(),
                selection_sha256: hash('0'),
            },
            effective_vfs_manifest_sha256: hash('f'),
            ordered_layers: vec![RecordedContentLayer {
                ordinal: 0,
                layer_kind: ContentLayerKind::Base,
                layer_id: "base".into(),
                display_name: "the game base".into(),
                descriptor_sha256: None,
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: vec![],
                required_for_playback: true,
            }],
            dlc_selection: Vec::<DlcSelection>::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            required_content: vec![RequiredContent {
                content_key: "particle-shader".into(),
                kind: RequiredContentKind::ShaderSource,
                content: UsedContentKey {
                    effective_vfs_manifest_sha256: hash('f'),
                    normalized_game_relative_path: "gfx/fx/particle.shader".into(),
                    winning_layer_ordinal: 0,
                    layer_kind: ContentLayerKind::Base,
                    layer_id: "base".into(),
                    source_sha256: hash('b'),
                    payload_sha256: hash('b'),
                    payload_bytes: 17,
                },
                observation_count: 1,
                consumers: vec!["production-render".into()],
            }],
            manifest_digest_sha256: None,
        };
        content.source_profile.selection_sha256 = content.computed_selection_sha256().unwrap();
        content = content.with_computed_digest().unwrap();

        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"date":"1936.1.1.0","file":"snapshots/000000000000.snapshot.json"}]}"#;
        let snapshot = br#"{"schema_version":"1","game_id":"g","hour_index":0}"#;
        let entries = [
            BorrowedPackEntry {
                path: "payload/g/index.json".into(),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            BorrowedPackEntry {
                path: "payload/g/snapshots/000000000000.snapshot.json".into(),
                bytes: snapshot,
                role: "snapshot",
                hour_index: Some(0),
            },
        ];
        let mut bytes = Vec::new();
        pack_borrowed_to_writer(
            "g",
            &entries,
            &PackIdentity {
                schema_version: Some("1".into()),
                content_manifest: Some(content.clone()),
                ..PackIdentity::default()
            },
            false,
            &mut bytes,
        )
        .unwrap();
        let dir = TmpDir::new("content_manifest");
        let path = dir.join("portable.battleplan");
        fs::write(&path, bytes).unwrap();
        let opened = crate::Container::open(&path, &crate::ExpectedIdentity::default()).unwrap();
        assert_eq!(opened.content_manifest(), Some(&content));
        assert!(opened
            .manifest()
            .get("required_features")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .any(|value| value.as_str() == Some(PORTABLE_CONTENT_FEATURE)));
    }

    #[test]
    fn file_backed_entries_use_streaming_data_descriptor_path() {
        let dir = TmpDir::new("file_backed_streaming");
        let snapshot_path = dir.join("large.snapshot.json");
        let snapshot_bytes = format!(
            r#"{{"schema_version":"1","game_id":"g","hour_index":0,"countries":{{"GER":{{"blob":"{}"}}}}}}"#,
            "x".repeat(STREAM_CHUNK + 8192)
        )
        .into_bytes();
        fs::write(&snapshot_path, &snapshot_bytes).unwrap();

        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"date":"1936.1.1.0","file":"snapshots/000000000000.snapshot.json"}]}"#.to_vec();
        let borrowed = vec![BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        }];
        let files = vec![FileBackedPackEntry {
            path: "payload/g/snapshots/000000000000.snapshot.json".into(),
            file_path: snapshot_path.as_path(),
            role: "snapshot",
            hour_index: Some(0),
        }];
        let mut out = ChunkGuardWriter::new();
        pack_mixed_to_writer(
            "g",
            &borrowed,
            &files,
            &PackIdentity::default(),
            false,
            &mut out,
        )
        .expect("file-backed pack streams chunks");
        assert!(out.max_write <= STREAM_CHUNK);

        let name = b"payload/g/snapshots/000000000000.snapshot.json";
        let name_pos = find_bytes(&out.bytes, name).expect("snapshot local header name");
        let header = name_pos.checked_sub(30).expect("local header prefix");
        assert_eq!(&out.bytes[header..header + 4], b"PK\x03\x04");
        let flags = u16::from_le_bytes([out.bytes[header + 6], out.bytes[header + 7]]);
        assert_ne!(flags & 0x0008, 0, "file-backed entry uses data descriptor");

        let container = crate::container::Container::from_bytes(
            out.bytes,
            &crate::container::ExpectedIdentity::default(),
        )
        .expect("container opens streamed file-backed entry");
        let got = container
            .read_entry_bytes("payload/g/snapshots/000000000000.snapshot.json")
            .expect("read file-backed entry");
        assert_eq!(got, snapshot_bytes);
    }

    #[test]
    fn compression_profile_defaults_and_bounds_window() {
        let default = CompressionProfile::default();
        assert_eq!(default.zstd_level, DEFAULT_ZSTD_COMPRESSION_LEVEL);
        assert_eq!(default.zstd_window_log, ZSTD_WINDOW_LOG);

        let too_large = CompressionProfile {
            zstd_level: default.zstd_level,
            zstd_window_log: ZSTD_WINDOW_LOG + 1,
        };
        let err = too_large
            .validate()
            .expect_err("writer must not create unreadable windows");
        assert!(err.to_string().contains("reader compatibility max"));
    }

    /// The zstd policy is a pure size floor over EVERY entry.
    ///
    /// It used to also consult a three-name allowlist (`*.snapshot.json`,
    /// `/collector_raw/`, `/production_render_blobs/`). Measured on the sealed
    /// t26 recording that left 160,247,978 bytes of pure JSON STORED at full
    /// size — the recorder manifest, all six static catalogs, `index.json` and
    /// `timeline.json` — none of it already-compressed data, and it also covered
    /// the `replay_decoder` hour index and checkpoint cache, which are the
    /// artifacts that replace the per-hour JSON series entirely.
    #[test]
    fn zstd_policy_compresses_every_entry_over_the_size_floor() {
        // Compressible far beyond any plausible zstd framing overhead, so a
        // STORED result can only mean the policy declined the entry.
        let big = vec![b'{'; ZSTD_MIN_ENTRY_BYTES as usize * 2];
        let small = vec![b'{'; ZSTD_MIN_ENTRY_BYTES as usize - 1];
        let over_floor = [
            ("payload/g/recorder/native_session_manifest.json", "recorder_manifest"),
            (
                "payload/g/static/localisation_join/localisation_catalog.json",
                "static_artifact",
            ),
            ("payload/g/replay_decoder/checkpoints.h4cp", crate::replay_cache::CHECKPOINT_CACHE_ROLE),
            ("payload/g/replay_decoder/hour_index.h4ix", crate::replay_cache::HOUR_INDEX_ROLE),
            ("payload/g/index.json", "index"),
        ];
        let mut entries = over_floor
            .iter()
            .map(|(path, role)| BorrowedPackEntry {
                path: (*path).to_string(),
                bytes: &big,
                role,
                hour_index: None,
            })
            .collect::<Vec<_>>();
        entries.push(BorrowedPackEntry {
            path: "payload/g/timeline.json".to_string(),
            bytes: &small,
            role: "timeline",
            hour_index: None,
        });

        let mut out = Vec::new();
        pack_streaming_zstd_to_writer_with_profile(
            "g",
            &entries,
            &[],
            &[],
            &PackIdentity::default(),
            CompressionProfile::default(),
            &mut out,
        )
        .expect("pack the container under the production zstd profile");

        let archive = crate::zip::ZipArchive::parse(out).expect("parse the packed container");
        for (path, _) in over_floor {
            let entry = archive
                .entries()
                .iter()
                .find(|entry| entry.name == path)
                .unwrap_or_else(|| panic!("packed container is missing {path}"));
            assert_eq!(
                entry.method, ZIP_METHOD_ZSTD,
                "{path} is at/over the size floor and must be compressed"
            );
            assert!(
                entry.comp_size < entry.uncomp_size,
                "{path} compressed to {} from {}",
                entry.comp_size,
                entry.uncomp_size
            );
        }

        // Below the floor a zstd frame's own header and checksum can cost more
        // than the payload saves, and the container carries many tiny
        // descriptors.
        let tiny = archive
            .entries()
            .iter()
            .find(|entry| entry.name == "payload/g/timeline.json")
            .expect("packed container is missing the sub-floor entry");
        assert_eq!(tiny.method, ZIP_METHOD_STORE);
    }

    #[test]
    fn streaming_entry_crossing_reader_cap_aborts_without_finalizing() {
        let dir = TmpDir::new("entry_cap");
        let stream_path = dir.join("stream.bin");
        fs::write(&stream_path, b"0123456789abcdef").unwrap();

        let entry = StreamingFileEntry {
            path: "payload/g/collector_raw/stream.bin".to_string(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: None,
        };
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut written = 0u64;
        let mut zip64_used = false;
        let err = write_zip_file_streaming_with_entry_limit(
            &mut out,
            &mut central,
            &mut written,
            &mut zip64_used,
            "payload/g/collector_raw/stream.bin",
            &entry,
            PackCompression::Store,
            CompressionProfile::default(),
            8,
            &to_hex(&Sha256::digest(b"0123456789abcdef")),
            16,
        )
        .expect_err("streaming cap must abort");

        let msg = err.to_string();
        assert!(msg.contains("per-entry reader cap"), "{msg}");
        assert!(central.is_empty());
        assert!(!out.windows(4).any(|w| w == b"PK\x05\x06"));
    }

    #[test]
    fn streaming_entry_identity_drift_aborts_before_manifest_or_central_directory() {
        let dir = TmpDir::new("entry_identity_drift");
        let stream_path = dir.join("stream.bin");
        fs::write(&stream_path, b"actual bytes").unwrap();

        let entry = StreamingFileEntry {
            path: "payload/g/collector_raw/stream.bin".to_string(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: Some(12),
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(to_hex(&Sha256::digest(b"other bytes!"))),
        };
        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut written = 0u64;
        let mut zip64_used = false;
        let err = write_zip_file_streaming_with_entry_limit(
            &mut out,
            &mut central,
            &mut written,
            &mut zip64_used,
            &entry.path,
            &entry,
            PackCompression::Store,
            CompressionProfile::default(),
            MAX_CONTAINER_ENTRY_BYTES,
            entry.sha256.as_deref().unwrap(),
            entry.decoded_len.unwrap(),
        )
        .expect_err("digest drift must refuse the partially written archive");

        assert!(err.to_string().contains("changed while packing"));
        assert!(central.is_empty());
        assert!(!out.windows(13).any(|window| window == b"manifest.json"));
        assert!(!out.windows(4).any(|window| window == b"PK\x05\x06"));
    }

    #[test]
    fn entry_count_crossing_zip32_writes_zip64() {
        let empty: &[u8] = &[];
        let entries = (0..ZIP32_MAX_ENTRY_COUNT)
            .map(|i| BorrowedPackEntry {
                path: format!("payload/g/e{i}"),
                bytes: empty,
                role: "snapshot",
                hour_index: Some(i as i64),
            })
            .collect::<Vec<_>>();
        let mut out = Vec::new();
        pack_borrowed_to_writer("g", &entries, &PackIdentity::default(), false, &mut out)
            .expect("manifest entry pushes count into Zip64");

        assert!(out.windows(4).any(|w| w == b"PK\x06\x06"));
        assert!(out.windows(4).any(|w| w == b"PK\x06\x07"));
        assert!(out.windows(2).any(|w| w == u16::MAX.to_le_bytes()));
    }

    #[test]
    fn zip32_sentinel_fields_do_not_truncate() {
        assert_eq!(
            zip32_field(ZIP32_MAX_FIELD_VALUE),
            ZIP32_MAX_FIELD_VALUE as u32
        );
        assert_eq!(zip32_field(ZIP32_MAX_FIELD_VALUE + 1), u32::MAX);
        assert!(checked_zip_u16_len(usize::from(u16::MAX) + 1, "entry count", "EOCD").is_err());

        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut written = ZIP32_MAX_FIELD_VALUE + 1;
        let mut zip64_used = false;
        write_zip_file(
            &mut out,
            &mut central,
            &mut written,
            &mut zip64_used,
            "payload/g/index.json",
            b"{}",
            ZipCompression::Store,
            CompressionProfile::default(),
        )
        .expect("large local header offset uses Zip64 central extra");
        assert!(zip64_used);

        let mut out = Vec::new();
        let mut central = Vec::new();
        let mut written = 0u64;
        let mut zip64_used = false;
        let long_name = format!("payload/g/{}", "x".repeat(usize::from(u16::MAX)));
        let err = write_zip_file(
            &mut out,
            &mut central,
            &mut written,
            &mut zip64_used,
            &long_name,
            b"{}",
            ZipCompression::Store,
            CompressionProfile::default(),
        )
        .expect_err("entry name length must not truncate");
        assert!(err.to_string().contains("entry name length"));
        assert!(out.is_empty());
    }

    #[test]
    fn packer_refuses_unicode_case_collisions_and_non_nfc_paths() {
        let entries = vec![
            PackEntry {
                path: "payload/g/CAF\u{c9}.json".into(),
                bytes: b"one".to_vec(),
                role: "other",
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/caf\u{e9}.json".into(),
                bytes: b"two".to_vec(),
                role: "other",
                hour_index: None,
            },
        ];
        assert!(matches!(
            pack("g", &entries, &PackIdentity::default(), false),
            Err(PackError::BadPath(_))
        ));

        let non_nfc = [PackEntry {
            path: "payload/g/cafe\u{301}.json".into(),
            bytes: Vec::new(),
            role: "other",
            hour_index: None,
        }];
        assert!(matches!(
            pack("g", &non_nfc, &PackIdentity::default(), false),
            Err(PackError::BadPath(_))
        ));

        let full_fold_collision = [
            PackEntry {
                path: "payload/g/Stra\u{df}e.json".into(),
                bytes: b"one".to_vec(),
                role: "other",
                hour_index: None,
            },
            PackEntry {
                path: "payload/g/STRASSE.json".into(),
                bytes: b"two".to_vec(),
                role: "other",
                hour_index: None,
            },
        ];
        assert!(matches!(
            pack("g", &full_fold_collision, &PackIdentity::default(), false),
            Err(PackError::BadPath(_))
        ));

        let top_level_payload = [PackEntry {
            path: "outside-payload.json".into(),
            bytes: Vec::new(),
            role: "other",
            hour_index: None,
        }];
        assert!(matches!(
            pack("g", &top_level_payload, &PackIdentity::default(), false),
            Err(PackError::BadPath(_))
        ));
    }

    #[test]
    fn ordered_verified_packer_keeps_large_sources_chunk_bounded_and_deterministic() {
        struct TrackingWriter {
            bytes: Vec<u8>,
            largest_write: usize,
        }
        impl Write for TrackingWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.largest_write = self.largest_write.max(bytes.len());
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        const SOURCE_CHUNK: usize = 256 * 1024;
        const SOURCE_CHUNKS: usize = 48;
        let chunk = vec![0x5au8; SOURCE_CHUNK];
        let mut digest = Sha256::new();
        for _ in 0..SOURCE_CHUNKS {
            digest.update(&chunk);
        }
        let expected_sha256 = to_hex(&digest.finalize());
        let expected_len = (SOURCE_CHUNK * SOURCE_CHUNKS) as u64;
        let control = br#"{"index_schema":"fixture"}"#;
        let entries = [
            OrderedStreamingPackEntry::Borrowed(BorrowedPackEntry {
                path: "payload/g/index.json".into(),
                bytes: control,
                role: "index",
                hour_index: None,
            }),
            OrderedStreamingPackEntry::Verified(VerifiedPackEntry {
                path: "payload/g/raw/source.h4cr".into(),
                source_id: 7,
                expected_len,
                expected_sha256,
                role: "raw_source",
                hour_index: None,
            }),
        ];

        let mut first = TrackingWriter {
            bytes: Vec::new(),
            largest_write: 0,
        };
        pack_ordered_streaming_to_writer_with_options(
            "g",
            &entries,
            &PackIdentity::default(),
            false,
            &PackOptions::default(),
            &mut first,
            |source_id, writer| {
                assert_eq!(source_id, 7);
                for _ in 0..SOURCE_CHUNKS {
                    writer.write_all(&chunk)?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert!(
            first.largest_write <= SOURCE_CHUNK,
            "source write expanded to {} bytes",
            first.largest_write
        );

        let mut second = TrackingWriter {
            bytes: Vec::new(),
            largest_write: 0,
        };
        pack_ordered_streaming_to_writer_with_options(
            "g",
            &entries,
            &PackIdentity::default(),
            false,
            &PackOptions::default(),
            &mut second,
            |_, writer| {
                for _ in 0..SOURCE_CHUNKS {
                    writer.write_all(&chunk)?;
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(second.bytes, first.bytes);
        let archive = crate::zip::ZipArchive::parse(first.bytes).unwrap();
        let names = archive
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "payload/g/index.json",
                "payload/g/raw/source.h4cr",
                "manifest.json"
            ]
        );
    }
}
