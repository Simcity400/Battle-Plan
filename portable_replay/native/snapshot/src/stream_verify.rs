//! STREAMING `.battleplan` verifier (bounded memory).
//!
//! Path-based [`crate::container::Container`] opens retain a bounded file-backed
//! archive; byte-buffer opens remain available for fixtures. The recorder must
//! still self-verify a container whose raw-stream entry can be many GiB without
//! ever materializing it. This module re-reads the finished container
//! FROM DISK, one entry at a time, seeking to each local header and streaming its
//! compressed bytes through the decompressor while hashing incrementally. Peak
//! memory is O(one chunk + retained entry metadata + the manifest JSON), not
//! O(archive): central-directory records are parsed incrementally. It performs
//! the same integrity checks the in-RAM reader does
//! (manifest digest, per-entry sha256, manifest↔zip coverage, declared-size
//! agreement) and, for the collector stream entry, returns its decompressed
//! sha256 so the caller can compare it to the capture-time hash (the streaming
//! replacement for the old in-RAM "packed stream bytes == pipe bytes" compare).

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::zip::{
    legacy_path_key, portable_path_key, validate_path, validate_path_legacy_v1,
    validate_payload_path, validate_payload_path_legacy_v1, INITIAL_ENTRY_RESERVE,
    MAX_CONTAINER_ENTRY_BYTES, MAX_READER_ENTRIES, STREAMING_MAX_CONTAINER_ENTRY_BYTES,
    ZIP32_MAX_FIELD_VALUE, ZIP_METHOD_DEFLATE, ZIP_METHOD_STORE, ZIP_METHOD_ZSTD, ZSTD_WINDOW_LOG,
};
use flate2::read::DeflateDecoder;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const EOCD_MIN_LEN: u64 = 22;
const ZIP64_LOCATOR_LEN: u64 = 20;
const ZIP64_EOCD_MIN_LEN: u64 = 56;
const VERIFY_CHUNK: usize = 4 * 1024 * 1024;
const STITCHED_CONTAINER_VERSION: u64 = 2;
const FEATURE_TIMELINE_SEGMENTS_V1: &str = "timeline_segments_v1";
const FEATURE_PORTABLE_CONTENT_MANIFEST_V2: &str = "portable_content_manifest_v2";
const KNOWN_FEATURES: &[&str] = &[
    FEATURE_TIMELINE_SEGMENTS_V1,
    FEATURE_PORTABLE_CONTENT_MANIFEST_V2,
];

/// What a successful streaming verification observed.
#[derive(Debug, Clone, Default)]
pub struct StreamVerifyReport {
    /// Number of manifest-listed entries whose bytes were re-read and hash-matched.
    pub entries_verified: usize,
    /// Decompressed sha256 of the unique canonical entry whose role matched
    /// the requested stream role, if found.
    pub stream_entry_sha256: Option<String>,
    /// Decompressed byte length of that stream entry.
    pub stream_entry_bytes: u64,
    /// Sum of every entry's uncompressed size (integrity accounting only).
    pub total_uncompressed: u64,
}

/// A central-directory record we serve.
#[derive(Debug)]
pub(crate) struct CdEntry {
    pub(crate) name: String,
    pub(crate) version_needed: u16,
    pub(crate) flags: u16,
    pub(crate) method: u16,
    pub(crate) mod_time: u16,
    pub(crate) mod_date: u16,
    pub(crate) crc32: u32,
    pub(crate) comp_size: u64,
    pub(crate) uncomp_size: u64,
    pub(crate) local_header_off: u64,
}

/// File-backed archive used by the product reader. It retains one pinned file
/// handle for central-directory parsing and lazy authenticated payload reads;
/// the complete container never resides in a `Vec<u8>`.
pub(crate) struct FileBackedArchive {
    file: Mutex<File>,
    file_len: u64,
    central_directory_off: u64,
    entries: Vec<CdEntry>,
}

impl FileBackedArchive {
    /// Hash a retained container handle before any ZIP parsing begins. The
    /// caller must move this same handle into [`FileBackedArchive::open`] and
    /// compare the result with [`Self::whole_file_receipt`] after parsing.
    pub(crate) fn whole_file_receipt_before_open(file: &mut File) -> Result<(u64, String), String> {
        let file_len = file
            .metadata()
            .map_err(|error| format!("stat retained container for receipt: {error}"))?
            .len();
        whole_file_receipt_from_handle(file, file_len)
    }

    pub(crate) fn open(mut file: File, max_entries: u64) -> Result<Self, String> {
        let file_len = file
            .metadata()
            .map_err(|error| format!("stat container: {error}"))?
            .len();
        let (entries, central_directory_off) =
            read_central_directory_with_limit_policy(&mut file, file_len, max_entries, false)?;
        Ok(Self {
            file: Mutex::new(file),
            file_len,
            central_directory_off,
            entries,
        })
    }

    pub(crate) fn entries(&self) -> &[CdEntry] {
        &self.entries
    }

    /// Derive a whole-container receipt through the exact retained handle.
    /// The pathname is never resolved or reopened, and length is checked both
    /// before and after hashing so a persistent concurrent resize fails closed.
    pub(crate) fn whole_file_receipt(&self) -> Result<(u64, String), String> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| "container file lock poisoned".to_string())?;
        whole_file_receipt_from_handle(&mut file, self.file_len)
    }

    pub(crate) fn validate_local_headers(&self) -> Result<(), String> {
        let mut file = self
            .file
            .lock()
            .map_err(|_| "container file lock poisoned".to_string())?;
        validate_local_headers_file(
            &mut file,
            &self.entries,
            self.file_len,
            self.central_directory_off,
        )
    }

    pub(crate) fn read_entry(
        &self,
        index: usize,
        calculate_sha256: bool,
    ) -> Result<(Vec<u8>, Option<String>), String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("container entry index {index} is out of range"))?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| "container file lock poisoned".to_string())?;
        validate_file_length(&file, self.file_len)?;
        let mut bytes = Vec::with_capacity(entry.uncomp_size.min(1 << 20) as usize);
        let (sha256, _) = entry_integrity_inner(
            &mut file,
            entry,
            Some(&mut bytes),
            calculate_sha256,
            MAX_CONTAINER_ENTRY_BYTES,
        )?;
        validate_file_length(&file, self.file_len)?;
        Ok((bytes, sha256))
    }

    /// Decompress one entry directly into a caller-owned writer while deriving
    /// the same CRC/size/SHA evidence as an ordinary authenticated read. This
    /// path is intentionally available only on the retained file-backed
    /// archive: it never materializes the complete entry in memory.
    pub(crate) fn copy_entry_to_writer<W: Write>(
        &self,
        index: usize,
        writer: &mut W,
        calculate_sha256: bool,
        maximum_uncompressed_bytes: u64,
    ) -> Result<(Option<String>, u64), String> {
        let entry = self
            .entries
            .get(index)
            .ok_or_else(|| format!("container entry index {index} is out of range"))?;
        let mut file = self
            .file
            .lock()
            .map_err(|_| "container file lock poisoned".to_string())?;
        validate_file_length(&file, self.file_len)?;
        let result = entry_integrity_inner(
            &mut file,
            entry,
            Some(writer),
            calculate_sha256,
            maximum_uncompressed_bytes.min(STREAMING_MAX_CONTAINER_ENTRY_BYTES),
        )?;
        validate_file_length(&file, self.file_len)?;
        Ok(result)
    }
}

fn validate_file_length(file: &File, expected: u64) -> Result<(), String> {
    let observed = file
        .metadata()
        .map_err(|error| format!("stat retained container handle: {error}"))?
        .len();
    if observed != expected {
        return Err(format!(
            "container file length changed while opening (observed {expected}, now {observed})"
        ));
    }
    Ok(())
}

fn whole_file_receipt_from_handle(
    file: &mut File,
    expected_len: u64,
) -> Result<(u64, String), String> {
    validate_file_length(file, expected_len)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek retained container for receipt: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 128 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read retained container for receipt: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    validate_file_length(file, expected_len)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewind retained container after receipt: {error}"))?;
    Ok((expected_len, to_hex(&digest.finalize())))
}

#[derive(Clone, Copy)]
struct FileEntryRegion {
    header_start: u64,
    region_end: u64,
}

fn rd_u16(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(off)?, *b.get(off + 1)?]))
}
fn rd_u32(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(off)?,
        *b.get(off + 1)?,
        *b.get(off + 2)?,
        *b.get(off + 3)?,
    ]))
}
fn rd_u64(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes([
        *b.get(off)?,
        *b.get(off + 1)?,
        *b.get(off + 2)?,
        *b.get(off + 3)?,
        *b.get(off + 4)?,
        *b.get(off + 5)?,
        *b.get(off + 6)?,
        *b.get(off + 7)?,
    ]))
}

/// Verify a finished `.battleplan` on disk with bounded memory. `game_id` and
/// `stream_role` identify the collector raw-stream entry whose decompressed
/// sha256 is returned for the capture-hash compare. Every entry is hash-checked
/// against the manifest; any mismatch, coverage gap, or digest failure is a hard
/// error (fail-closed), mirroring [`crate::container::Container::open`].
pub fn verify_replay_container_streaming(
    path: &Path,
    game_id: &str,
    stream_role: &str,
) -> Result<StreamVerifyReport, String> {
    let mut file =
        File::open(path).map_err(|e| format!("open container {}: {e}", path.display()))?;
    verify_replay_container_streaming_file(&mut file, game_id, stream_role)
}

/// Verify through a caller-owned open file handle. The caller controls the
/// handle's access and sharing policy and can keep this exact object open after
/// verification (for example, through an atomic rename-by-handle promotion).
/// The verifier never re-opens or resolves a pathname.
pub fn verify_replay_container_streaming_file(
    file: &mut File,
    game_id: &str,
    stream_role: &str,
) -> Result<StreamVerifyReport, String> {
    verify_replay_container_streaming_file_at_path(
        file,
        game_id,
        stream_role,
        "collector_raw/collector_stream.bin",
    )
}

/// Verify through a caller-owned handle while binding the requested stream role
/// to an explicit game-relative payload path.
pub fn verify_replay_container_streaming_file_at_path(
    mut file: &mut File,
    game_id: &str,
    stream_role: &str,
    stream_payload_path: &str,
) -> Result<StreamVerifyReport, String> {
    let file_len = file
        .metadata()
        .map_err(|e| format!("stat container: {e}"))?
        .len();

    // The path-policy selector is authenticated inside manifest.json, so the
    // structural pass must begin with the historical grammar. It rejects
    // traversal/absolute/device paths but deliberately does not impose NFC or
    // full Unicode-caseless identity before the manifest version is known.
    let (entries, central_directory_off) =
        read_central_directory_with_limit_policy(&mut file, file_len, MAX_READER_ENTRIES, false)?;
    validate_local_headers_file(&mut file, &entries, file_len, central_directory_off)?;

    // --- manifest.json (read + parse + digest) ---
    let man_entry = entries
        .iter()
        .find(|e| e.name == "manifest.json")
        .ok_or_else(|| "container has no manifest.json (unfinalized/recovery-only)".to_string())?;
    if man_entry.uncomp_size > MAX_CONTAINER_ENTRY_BYTES
        || man_entry.comp_size > MAX_CONTAINER_ENTRY_BYTES
    {
        return Err(format!(
            "entry manifest.json exceeds per-entry reader cap (compressed {}, uncompressed {}, cap {})",
            man_entry.comp_size, man_entry.uncomp_size, MAX_CONTAINER_ENTRY_BYTES
        ));
    }
    let man_bytes = read_entry_bytes(&mut file, man_entry)?;
    let manifest: Value = crate::strict_json::from_slice(&man_bytes)
        .map_err(|e| format!("strict parse manifest.json: {e}"))?;
    if manifest.get("format").and_then(Value::as_str) != Some("hoi4replay") {
        return Err("manifest.format != \"hoi4replay\"".to_string());
    }
    let unicode_paths = manifest_uses_unicode_paths(&manifest)?;
    validate_archive_paths(&entries, unicode_paths)?;
    if unicode_paths {
        validate_path(stream_payload_path)
    } else {
        validate_path_legacy_v1(stream_payload_path)
    }
    .map_err(|e| format!("invalid canonical stream payload path: {e}"))?;

    let declared_digest = manifest
        .get("manifest_digest")
        .and_then(Value::as_str)
        .ok_or_else(|| "manifest.manifest_digest missing".to_string())?
        .to_string();
    let recomputed = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    if recomputed != declared_digest {
        return Err(format!(
            "manifest_digest mismatch (recomputed {recomputed}, declared {declared_digest})"
        ));
    }

    // --- manifest entry table: path -> (sha, bytes, role) ---
    let man_entries = manifest
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "manifest.entries missing".to_string())?;
    struct ManRec {
        sha: String,
        bytes: u64,
        role: Option<String>,
    }
    let mut by_path: HashMap<String, ManRec> = HashMap::new();
    let mut manifest_path_keys = HashSet::with_capacity(man_entries.len());
    for ent in man_entries {
        let p = ent
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "entry.path missing".to_string())?;
        let collision_key = versioned_path_key(p, true, unicode_paths)
            .map_err(|e| format!("invalid manifest entry path {p}: {e}"))?;
        if !manifest_path_keys.insert(collision_key) {
            return Err(format!(
                "duplicate or portable-path-colliding manifest entry {p}"
            ));
        }
        let sha = ent
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("entry {p}: sha256 missing"))?;
        portable_content_manifest::hud_role::validate_sha256(sha, "manifest entry sha256")
            .map_err(|error| format!("entry {p}: {error}"))?;
        let bytes = ent
            .get("bytes")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("entry {p}: bytes missing"))?;
        let role = ent.get("role").and_then(Value::as_str).map(str::to_string);
        if by_path
            .insert(
                p.to_string(),
                ManRec {
                    sha: sha.to_string(),
                    bytes,
                    role,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate manifest entry {p}"));
        }
    }

    // The capture hash binds to one object, not merely to a role label. A
    // second role-bearing decoy must not be able to overwrite the report after
    // the canonical stream has been hashed, and a lone role at any other path
    // is not the captured stream for this game.
    let canonical_stream_path = format!("payload/{game_id}/{stream_payload_path}");
    let mut stream_role_paths = by_path
        .iter()
        .filter_map(|(path, rec)| {
            (rec.role.as_deref() == Some(stream_role)).then_some(path.as_str())
        })
        .collect::<Vec<_>>();
    stream_role_paths.sort_unstable();
    match stream_role_paths.as_slice() {
        [path] if *path == canonical_stream_path => {}
        [path] => {
            return Err(format!(
                "stream role {stream_role:?} must occur at canonical path {canonical_stream_path:?}, found {path:?}"
            ))
        }
        paths => {
            return Err(format!(
                "stream role {stream_role:?} must occur exactly one time at {canonical_stream_path:?}, found {} entries: {paths:?}",
                paths.len()
            ))
        }
    }

    // Coverage: every zip payload entry is in the manifest with the declared size,
    // and every manifest entry exists in the zip (no stowaways, no phantoms).
    let mut zip_names: HashMap<&str, &CdEntry> = HashMap::new();
    for e in &entries {
        if e.name == "manifest.json" {
            continue;
        }
        if e.uncomp_size > STREAMING_MAX_CONTAINER_ENTRY_BYTES
            || e.comp_size > STREAMING_MAX_CONTAINER_ENTRY_BYTES
        {
            return Err(format!(
                "entry {} exceeds streaming per-entry cap (compressed {}, uncompressed {}, cap {})",
                e.name, e.comp_size, e.uncomp_size, STREAMING_MAX_CONTAINER_ENTRY_BYTES
            ));
        }
        zip_names.insert(e.name.as_str(), e);
        match by_path.get(&e.name) {
            None => {
                return Err(format!(
                    "zip entry {} is not listed in the manifest (tamper signal)",
                    e.name
                ))
            }
            Some(rec) => {
                if rec.bytes != e.uncomp_size {
                    return Err(format!(
                        "entry {}: manifest bytes {} != zip uncompressed {}",
                        e.name, rec.bytes, e.uncomp_size
                    ));
                }
            }
        }
    }
    for p in by_path.keys() {
        if !zip_names.contains_key(p.as_str()) {
            return Err(format!("manifest entry {p} not present in zip"));
        }
    }

    // --- hash EVERY payload entry from disk (bounded), tracking only the
    // unique canonical stream object established above ---
    let mut report = StreamVerifyReport::default();
    for e in &entries {
        if e.name == "manifest.json" {
            continue;
        }
        let rec = by_path.get(&e.name).ok_or_else(|| {
            format!(
                "internal coverage invariant lost manifest record for entry {}",
                e.name
            )
        })?;
        let (sha, uncomp) = hash_entry_streaming(&mut file, e)?;
        if sha != rec.sha {
            return Err(format!(
                "entry {}: sha256 {sha} != manifest {}",
                e.name, rec.sha
            ));
        }
        report.entries_verified += 1;
        report.total_uncompressed = report.total_uncompressed.saturating_add(uncomp);
        if e.name == canonical_stream_path {
            report.stream_entry_sha256 = Some(sha);
            report.stream_entry_bytes = uncomp;
        }
    }
    if report.stream_entry_sha256.is_none() {
        return Err(format!(
            "canonical stream entry {canonical_stream_path:?} was not verified"
        ));
    }
    Ok(report)
}

/// Select the same versioned path namespace as `Container::open`. This is kept
/// next to the streaming verifier because it must make the decision before any
/// archive/manifest path join is trusted.
fn manifest_uses_unicode_paths(manifest: &Value) -> Result<bool, String> {
    let container_version = manifest
        .get("container_version")
        .and_then(Value::as_u64)
        .ok_or_else(|| "manifest.container_version missing".to_string())?;
    if container_version > crate::container::MAX_CONTAINER_VERSION {
        return Err(format!(
            "container_version {container_version} > reader max {}",
            crate::container::MAX_CONTAINER_VERSION
        ));
    }

    let mut declared_features = Vec::new();
    match manifest.get("required_features") {
        None | Some(Value::Null) => {}
        Some(Value::Array(features)) => {
            for feature in features {
                let feature = feature.as_str().ok_or_else(|| {
                    "manifest.required_features has a non-string element (malformed)".to_string()
                })?;
                if !KNOWN_FEATURES.contains(&feature) {
                    return Err(format!(
                        "unknown required feature {feature:?} in required_features"
                    ));
                }
                declared_features.push(feature);
            }
        }
        Some(_) => {
            return Err(
                "manifest.required_features must be an array (malformed; fail-closed)".into(),
            )
        }
    }

    let has_timeline_segments = declared_features.contains(&FEATURE_TIMELINE_SEGMENTS_V1);
    if container_version >= STITCHED_CONTAINER_VERSION && !has_timeline_segments {
        return Err(format!(
            "container_version {container_version} must declare required feature {FEATURE_TIMELINE_SEGMENTS_V1:?}"
        ));
    }
    if container_version < STITCHED_CONTAINER_VERSION && has_timeline_segments {
        return Err(format!(
            "required feature {FEATURE_TIMELINE_SEGMENTS_V1:?} requires container_version {STITCHED_CONTAINER_VERSION}"
        ));
    }

    Ok(container_version >= STITCHED_CONTAINER_VERSION
        || declared_features.contains(&FEATURE_PORTABLE_CONTENT_MANIFEST_V2))
}

fn validate_archive_paths(entries: &[CdEntry], unicode_paths: bool) -> Result<(), String> {
    let mut collision_keys = HashSet::with_capacity(entries.len());
    for entry in entries {
        let key = versioned_path_key(&entry.name, entry.name != "manifest.json", unicode_paths)
            .map_err(|e| format!("unsafe entry path {}: {e}", entry.name))?;
        if !collision_keys.insert(key) {
            return Err(format!(
                "duplicate entry name {} (portable path collision)",
                entry.name
            ));
        }
    }
    Ok(())
}

fn versioned_path_key(name: &str, payload: bool, unicode_paths: bool) -> Result<String, String> {
    if unicode_paths {
        if payload {
            validate_payload_path(name)
        } else {
            validate_path(name)
        }
        .map_err(|e| e.to_string())?;
        portable_path_key(name).map_err(|e| e.to_string())
    } else {
        if payload {
            validate_payload_path_legacy_v1(name)
        } else {
            validate_path_legacy_v1(name)
        }
        .map_err(|e| e.to_string())?;
        legacy_path_key(name).map_err(|e| e.to_string())
    }
}

/// Locate the End-Of-Central-Directory and parse central-directory records
/// incrementally, retaining only the metadata needed for later reads.
#[cfg(test)]
fn read_central_directory(file: &mut File, file_len: u64) -> Result<Vec<CdEntry>, String> {
    read_central_directory_with_limit(file, file_len, MAX_READER_ENTRIES)
        .map(|(entries, _)| entries)
}

fn read_central_directory_with_limit(
    file: &mut File,
    file_len: u64,
    max_entries: u64,
) -> Result<(Vec<CdEntry>, u64), String> {
    read_central_directory_with_limit_policy(file, file_len, max_entries, true)
}

fn read_central_directory_with_limit_policy(
    file: &mut File,
    file_len: u64,
    max_entries: u64,
    unicode_v2: bool,
) -> Result<(Vec<CdEntry>, u64), String> {
    if file_len < 22 {
        return Err("container too small for an EOCD".to_string());
    }
    // The EOCD sits within the last 22 + 0xFFFF bytes (comment cap). Include
    // the Zip64 locator + minimum Zip64 EOCD before it so the locator is always
    // inside the scanned tail; the Zip64 EOCD record itself is then reached by
    // FOLLOWING the locator's declared offset (parse_zip64_eocd), because a
    // record with an extensible data sector can validly start before any fixed
    // tail window.
    let tail_len = (EOCD_MIN_LEN + 0xFFFF + ZIP64_LOCATOR_LEN + ZIP64_EOCD_MIN_LEN).min(file_len);
    let tail_start = file_len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|e| format!("seek eocd: {e}"))?;
    file.read_exact(&mut tail)
        .map_err(|e| format!("read eocd tail: {e}"))?;

    let eocd = find_eocd(&tail).ok_or_else(|| "no EOCD record found".to_string())?;
    // Mirror the in-RAM reader's EOCD strictness (zip.rs): multi-disk and split
    // central directories are refused, and ordinary EOCD fields must agree with
    // their Zip64 counterparts — a container the in-RAM reader refuses must
    // never be promoted by the recorder-side verifier.
    let disk_no = rd_u16(&tail, eocd + 4).ok_or("truncated eocd")?;
    let cd_disk = rd_u16(&tail, eocd + 6).ok_or("truncated eocd")?;
    if disk_no != 0 || cd_disk != 0 {
        return Err("multi-disk ZIP archives are not supported".to_string());
    }
    let entries_on_disk16 = rd_u16(&tail, eocd + 8).ok_or("truncated eocd")?;
    let total16 = rd_u16(&tail, eocd + 10).ok_or("truncated eocd")?;
    let cd_size32 = rd_u32(&tail, eocd + 12).ok_or("truncated eocd")?;
    let cd_off32 = rd_u32(&tail, eocd + 16).ok_or("truncated eocd")?;
    let locator = find_zip64_locator(&tail, eocd);
    let (total_u64, cd_size, cd_off, expected_cd_end) = if entries_on_disk16 == u16::MAX
        || total16 == u16::MAX
        || cd_size32 == u32::MAX
        || cd_off32 == u32::MAX
        || locator.is_some()
    {
        let locator = locator.ok_or_else(|| "Zip64 locator missing".to_string())?;
        let (zip64_total, zip64_cd_size, zip64_cd_off, zip64_eocd_abs) =
            parse_zip64_eocd(file, &tail, tail_start, locator)?;
        crate::zip::validate_ordinary_zip64_field(
            "entries-on-disk count",
            entries_on_disk16 as u64,
            u16::MAX as u64,
            zip64_total,
        )
        .map_err(|e| e.to_string())?;
        crate::zip::validate_ordinary_zip64_field(
            "total-entry count",
            total16 as u64,
            u16::MAX as u64,
            zip64_total,
        )
        .map_err(|e| e.to_string())?;
        crate::zip::validate_ordinary_zip64_field(
            "central-directory size",
            cd_size32 as u64,
            u32::MAX as u64,
            zip64_cd_size,
        )
        .map_err(|e| e.to_string())?;
        crate::zip::validate_ordinary_zip64_field(
            "central-directory offset",
            cd_off32 as u64,
            u32::MAX as u64,
            zip64_cd_off,
        )
        .map_err(|e| e.to_string())?;
        (zip64_total, zip64_cd_size, zip64_cd_off, zip64_eocd_abs)
    } else {
        if entries_on_disk16 != total16 {
            return Err("split ZIP central directories are not supported".to_string());
        }
        (
            total16 as u64,
            cd_size32 as u64,
            cd_off32 as u64,
            tail_start + eocd as u64,
        )
    };
    // The central directory must end exactly at its end record (same anchor the
    // in-RAM reader enforces): no unaccounted bytes between CD and (Zip64) EOCD.
    let actual_cd_end = cd_off
        .checked_add(cd_size)
        .ok_or_else(|| "central directory range overflows".to_string())?;
    if actual_cd_end != expected_cd_end {
        return Err(format!(
            "central directory does not end at its end record: {actual_cd_end} != {expected_cd_end}"
        ));
    }
    if total_u64 > max_entries {
        return Err(format!(
            "central directory entry count {total_u64} exceeds reader cap {max_entries}"
        ));
    }
    let total = usize::try_from(total_u64)
        .map_err(|_| "central directory entry count too large".to_string())?;
    if cd_off.saturating_add(cd_size) > file_len {
        return Err("central directory out of range".to_string());
    }

    file.seek(SeekFrom::Start(cd_off))
        .map_err(|e| format!("seek central dir: {e}"))?;
    let mut entries = Vec::with_capacity(total.min(INITIAL_ENTRY_RESERVE));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut seen_ci: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut consumed = 0u64;
    for record_index in 0..total {
        let fixed_end = consumed
            .checked_add(46)
            .ok_or("central-directory record offset overflow")?;
        if fixed_end > cd_size {
            return Err(format!(
                "central directory record {record_index} is truncated"
            ));
        }
        let mut cdh = [0u8; 46];
        file.read_exact(&mut cdh)
            .map_err(|e| format!("read central directory record {record_index}: {e}"))?;
        let sig = rd_u32(&cdh, 0).ok_or("truncated cdh")?;
        if sig != CDH_SIG {
            return Err(format!(
                "bad central-directory header signature at record {record_index}"
            ));
        }
        let version_needed = rd_u16(&cdh, 6).ok_or("truncated cdh")?;
        let flags = rd_u16(&cdh, 8).ok_or("truncated cdh")?;
        let method = rd_u16(&cdh, 10).ok_or("truncated cdh")?;
        let mod_time = rd_u16(&cdh, 12).ok_or("truncated cdh")?;
        let mod_date = rd_u16(&cdh, 14).ok_or("truncated cdh")?;
        let crc32 = rd_u32(&cdh, 16).ok_or("truncated cdh")?;
        if flags & !0x0008 != 0 {
            return Err(format!(
                "entry has unsupported zip general-purpose flags 0x{flags:04x}"
            ));
        }
        let comp_size32 = rd_u32(&cdh, 20).ok_or("truncated cdh")?;
        let uncomp_size32 = rd_u32(&cdh, 24).ok_or("truncated cdh")?;
        let name_len = rd_u16(&cdh, 28).ok_or("truncated cdh")? as usize;
        let extra_len = rd_u16(&cdh, 30).ok_or("truncated cdh")? as usize;
        let comment_len = rd_u16(&cdh, 32).ok_or("truncated cdh")? as usize;
        let disk_start = rd_u16(&cdh, 34).ok_or("truncated cdh")?;
        if disk_start != 0 {
            return Err(format!(
                "entry at central-directory record {record_index} starts on disk {disk_start}; multi-disk ZIP archives are not supported"
            ));
        }
        let lho32 = rd_u32(&cdh, 42).ok_or("truncated cdh")?;
        let variable_len = (name_len as u64)
            .checked_add(extra_len as u64)
            .and_then(|value| value.checked_add(comment_len as u64))
            .ok_or("central-directory variable fields overflow")?;
        let record_end = fixed_end
            .checked_add(variable_len)
            .ok_or("central-directory record end overflow")?;
        if record_end > cd_size {
            return Err(format!(
                "central directory record {record_index} extends past declared size"
            ));
        }
        let mut name_bytes = vec![0u8; name_len];
        file.read_exact(&mut name_bytes)
            .map_err(|e| format!("read central-directory name {record_index}: {e}"))?;
        let name = std::str::from_utf8(&name_bytes)
            .map_err(|_| "non-utf8 entry name".to_string())?
            .to_string();
        let mut extra = vec![0u8; extra_len];
        file.read_exact(&mut extra)
            .map_err(|e| format!("read central-directory extra {record_index}: {e}"))?;
        let need_uncomp = uncomp_size32 == u32::MAX;
        let need_comp = comp_size32 == u32::MAX;
        let need_lho = lho32 == u32::MAX;
        let zip64 = parse_zip64_extra(&extra, need_uncomp, need_comp, need_lho)?;
        let uncomp_size = if need_uncomp {
            zip64
                .uncomp_size
                .ok_or_else(|| format!("entry {name}: Zip64 uncompressed size missing"))?
        } else {
            uncomp_size32 as u64
        };
        let comp_size = if need_comp {
            zip64
                .comp_size
                .ok_or_else(|| format!("entry {name}: Zip64 compressed size missing"))?
        } else {
            comp_size32 as u64
        };
        let lho = if need_lho {
            zip64
                .local_header_off
                .ok_or_else(|| format!("entry {name}: Zip64 local header offset missing"))?
        } else {
            lho32 as u64
        };
        if unicode_v2 {
            validate_path(&name)
        } else {
            validate_path_legacy_v1(&name)
        }
        .map_err(|e| format!("unsafe entry path {name}: {e}"))?;
        if name != "manifest.json" {
            if unicode_v2 {
                validate_payload_path(&name)
            } else {
                validate_payload_path_legacy_v1(&name)
            }
            .map_err(|e| format!("entry outside payload root {name}: {e}"))?;
        }
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate entry name {name}"));
        }
        // case-insensitive collision (would clobber on Windows/macOS unpack);
        // same rule as the in-RAM reader.
        let collision_key = if unicode_v2 {
            portable_path_key(&name)
        } else {
            legacy_path_key(&name)
        }
        .map_err(|e| format!("unsafe entry path {name}: {e}"))?;
        if !seen_ci.insert(collision_key) {
            return Err(format!("duplicate entry name {name} (case-insensitive)"));
        }
        entries.push(CdEntry {
            name,
            version_needed,
            flags,
            method,
            mod_time,
            mod_date,
            crc32,
            comp_size,
            uncomp_size,
            local_header_off: lho,
        });
        consumed = record_end;
        let next_record = cd_off
            .checked_add(consumed)
            .ok_or("central-directory absolute offset overflow")?;
        file.seek(SeekFrom::Start(next_record))
            .map_err(|e| format!("seek past central-directory comment {record_index}: {e}"))?;
    }
    if entries.len() != total {
        return Err(format!(
            "central directory entry count mismatch (parsed {}, EOCD {total})",
            entries.len()
        ));
    }
    if consumed != cd_size {
        return Err("central directory trailing bytes".to_string());
    }
    Ok((entries, cd_off))
}

fn find_zip64_locator(tail: &[u8], eocd: usize) -> Option<usize> {
    if eocd < 20 {
        return None;
    }
    let off = eocd - 20;
    (rd_u32(tail, off) == Some(ZIP64_LOCATOR_SIG)).then_some(off)
}

/// Bytes allowed between a Zip64 EOCD record's start and its locator: the
/// 56-byte fixed prefix plus the record's zip64 extensible data sector. Real
/// sectors are tiny; the cap only bounds the verifier's window fail-closed.
const ZIP64_EOCD_MAX_SPAN: u64 = 1024 * 1024;

/// Returns `(total_entries, cd_size, cd_off, zip64_eocd_abs)` — the last is the
/// ABSOLUTE file offset of the Zip64 EOCD record, i.e. where the central
/// directory must end. The record is located by FOLLOWING the locator's
/// declared offset (exactly like the in-RAM reader) instead of requiring it to
/// start inside the fixed scanned tail: a valid extensible data sector plus a
/// max-size EOCD comment (or a sector larger than 65,535 bytes) pushes the
/// record before the tail, and the streaming verifier must accept every Zip64
/// EOCD the in-memory parser accepts. Only the fixed 56-byte prefix is ever
/// materialized; a record spanning more than [`ZIP64_EOCD_MAX_SPAN`] bytes
/// before its locator is a typed refusal.
fn parse_zip64_eocd(
    file: &mut File,
    tail: &[u8],
    tail_start: u64,
    locator: usize,
) -> Result<(u64, u64, u64, u64), String> {
    if rd_u32(tail, locator + 4).ok_or("truncated zip64 locator")? != 0 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    let zip64_eocd_abs = rd_u64(tail, locator + 8).ok_or("truncated zip64 locator")?;
    let total_disks = rd_u32(tail, locator + 16).ok_or("truncated zip64 locator")?;
    if total_disks != 1 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    let locator_abs = tail_start + locator as u64;
    if zip64_eocd_abs >= locator_abs {
        return Err(format!(
            "Zip64 EOCD offset {zip64_eocd_abs} does not precede its locator at {locator_abs}"
        ));
    }
    let span = locator_abs - zip64_eocd_abs;
    if span < ZIP64_EOCD_MIN_LEN {
        return Err("truncated Zip64 EOCD".to_string());
    }
    if span > ZIP64_EOCD_MAX_SPAN {
        return Err(format!(
            "Zip64 EOCD record spans {span} bytes before its locator, beyond the verifier cap {ZIP64_EOCD_MAX_SPAN}"
        ));
    }
    let mut record = [0u8; ZIP64_EOCD_MIN_LEN as usize];
    if zip64_eocd_abs >= tail_start {
        // Fast path: the record's fixed prefix already sits inside the scanned
        // tail (span >= 56 guarantees the slice ends at/before the locator).
        let off = usize::try_from(zip64_eocd_abs - tail_start)
            .map_err(|_| "Zip64 EOCD offset too large".to_string())?;
        let record_len = record.len();
        record.copy_from_slice(&tail[off..off + record_len]);
    } else {
        file.seek(SeekFrom::Start(zip64_eocd_abs))
            .map_err(|e| format!("seek zip64 eocd: {e}"))?;
        file.read_exact(&mut record)
            .map_err(|e| format!("read zip64 eocd: {e}"))?;
    }
    if rd_u32(&record, 0) != Some(ZIP64_EOCD_SIG) {
        return Err("bad Zip64 EOCD signature".to_string());
    }
    let record_size = rd_u64(&record, 4).ok_or("truncated zip64 eocd")?;
    if record_size < 44 {
        return Err("truncated Zip64 EOCD size".to_string());
    }
    let disk_no = rd_u32(&record, 16).ok_or("truncated zip64 eocd")?;
    let cd_disk = rd_u32(&record, 20).ok_or("truncated zip64 eocd")?;
    if disk_no != 0 || cd_disk != 0 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    let entries_on_disk = rd_u64(&record, 24).ok_or("truncated zip64 eocd")?;
    let total_entries = rd_u64(&record, 32).ok_or("truncated zip64 eocd")?;
    if entries_on_disk != total_entries {
        return Err("split Zip64 central directories are not supported".to_string());
    }
    let cd_size = rd_u64(&record, 40).ok_or("truncated zip64 eocd")?;
    let cd_off = rd_u64(&record, 48).ok_or("truncated zip64 eocd")?;
    Ok((total_entries, cd_size, cd_off, zip64_eocd_abs))
}

/// Zip64 extra-field parsing is shared with the in-RAM reader (one parser, one
/// strictness: duplicate 0x0001 fields and malformed extra buffers are refused
/// identically by both readers).
fn parse_zip64_extra(
    extra: &[u8],
    need_uncomp: bool,
    need_comp: bool,
    need_lho: bool,
) -> Result<crate::zip::Zip64Extra, String> {
    crate::zip::parse_zip64_extra(extra, need_uncomp, need_comp, need_lho)
        .map_err(|e| e.to_string())
}

/// Validate every local header and complete physical entry region without
/// decoding payload bytes, including central-directory order and overlap.
fn validate_local_headers_file(
    file: &mut File,
    entries: &[CdEntry],
    file_len: u64,
    central_directory_off: u64,
) -> Result<(), String> {
    let mut previous_end = 0u64;
    for entry in entries {
        let region = validate_entry_structure(file, entry, file_len, central_directory_off)?;
        if region.header_start < previous_end {
            return Err(format!(
                "entry regions overlap or are out of order: {}",
                entry.name
            ));
        }
        previous_end = region.region_end;
    }
    Ok(())
}

fn validate_entry_structure(
    file: &mut File,
    e: &CdEntry,
    file_len: u64,
    central_directory_off: u64,
) -> Result<FileEntryRegion, String> {
    let mut lfh = [0u8; 30];
    file.seek(SeekFrom::Start(e.local_header_off))
        .map_err(|error| format!("seek local header {}: {error}", e.name))?;
    file.read_exact(&mut lfh)
        .map_err(|error| format!("read local header {}: {error}", e.name))?;
    if rd_u32(&lfh, 0) != Some(LFH_SIG) {
        return Err(format!("entry {}: bad local header signature", e.name));
    }
    let version_needed = rd_u16(&lfh, 4).ok_or("truncated local version")?;
    let flags = rd_u16(&lfh, 6).ok_or("truncated local flags")?;
    let method = rd_u16(&lfh, 8).ok_or("truncated local method")?;
    let mod_time = rd_u16(&lfh, 10).ok_or("truncated local time")?;
    let mod_date = rd_u16(&lfh, 12).ok_or("truncated local date")?;
    let crc32 = rd_u32(&lfh, 14).ok_or("truncated local crc")?;
    let comp_size32 = rd_u32(&lfh, 18).ok_or("truncated local compressed size")?;
    let uncomp_size32 = rd_u32(&lfh, 22).ok_or("truncated local uncompressed size")?;
    if version_needed != e.version_needed
        || flags != e.flags
        || method != e.method
        || mod_time != e.mod_time
        || mod_date != e.mod_date
    {
        return Err(format!("entry {}: local/central header mismatch", e.name));
    }
    if e.method == ZIP_METHOD_STORE && e.comp_size != e.uncomp_size {
        return Err(format!(
            "entry {}: STORE compressed size {} != uncompressed size {}",
            e.name, e.comp_size, e.uncomp_size
        ));
    }
    let name_len = rd_u16(&lfh, 26).ok_or("truncated local name length")? as u64;
    let extra_len = rd_u16(&lfh, 28).ok_or("truncated local extra length")? as u64;
    let name_start = e
        .local_header_off
        .checked_add(30)
        .ok_or_else(|| format!("entry {}: local name start overflow", e.name))?;
    let data_start = name_start
        .checked_add(name_len)
        .and_then(|value| value.checked_add(extra_len))
        .ok_or_else(|| format!("entry {}: data start overflow", e.name))?;
    let data_end = data_start
        .checked_add(e.comp_size)
        .ok_or_else(|| format!("entry {}: data end overflow", e.name))?;
    if data_end > file_len || data_end > central_directory_off {
        return Err(format!(
            "entry {}: data range ends past payload area",
            e.name
        ));
    }

    file.seek(SeekFrom::Start(name_start))
        .map_err(|error| format!("seek local name {}: {error}", e.name))?;
    let mut local_name = vec![0u8; name_len as usize];
    file.read_exact(&mut local_name)
        .map_err(|error| format!("read local name {}: {error}", e.name))?;
    let mut local_extra = vec![0u8; extra_len as usize];
    file.read_exact(&mut local_extra)
        .map_err(|error| format!("read local extra {}: {error}", e.name))?;
    if local_name != e.name.as_bytes() {
        return Err(format!("entry {}: local/central name mismatch", e.name));
    }

    if e.flags & 0x0008 == 0 {
        let need_uncomp = uncomp_size32 == u32::MAX;
        let need_comp = comp_size32 == u32::MAX;
        let zip64 = parse_zip64_extra(&local_extra, need_uncomp, need_comp, false)?;
        let local_uncomp_size = if need_uncomp {
            zip64
                .uncomp_size
                .ok_or_else(|| format!("entry {}: local Zip64 uncompressed size missing", e.name))?
        } else {
            uncomp_size32 as u64
        };
        let local_comp_size = if need_comp {
            zip64
                .comp_size
                .ok_or_else(|| format!("entry {}: local Zip64 compressed size missing", e.name))?
        } else {
            comp_size32 as u64
        };
        if crc32 != e.crc32 || local_comp_size != e.comp_size || local_uncomp_size != e.uncomp_size
        {
            return Err(format!(
                "entry {}: local/central crc or size mismatch",
                e.name
            ));
        }
    } else if crc32 != 0 || comp_size32 != 0 || uncomp_size32 != 0 {
        return Err(format!(
            "entry {}: data-descriptor local crc/size fields must be zero",
            e.name
        ));
    }

    let region_end = if e.flags & 0x0008 != 0 {
        data_descriptor_end_file(file, e, data_end)?
    } else {
        data_end
    };
    if e.local_header_off >= central_directory_off || region_end > central_directory_off {
        return Err(format!(
            "entry {} region {}..{} overlaps central directory at {}",
            e.name, e.local_header_off, region_end, central_directory_off
        ));
    }
    Ok(FileEntryRegion {
        header_start: e.local_header_off,
        region_end,
    })
}

/// Read + decompress one entry FULLY into RAM. Only used for small entries
/// (manifest.json); large entries go through [`hash_entry_streaming`].
fn read_entry_bytes(file: &mut File, e: &CdEntry) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    entry_integrity_inner(file, e, Some(&mut out), false, MAX_CONTAINER_ENTRY_BYTES)?;
    Ok(out)
}

/// Stream + decompress one entry, returning `(sha256_hex, uncompressed_len)`
/// without holding the decompressed bytes.
fn hash_entry_streaming(file: &mut File, e: &CdEntry) -> Result<(String, u64), String> {
    let (digest, len) =
        entry_integrity_inner(file, e, None, true, STREAMING_MAX_CONTAINER_ENTRY_BYTES)?;
    Ok((
        digest.ok_or_else(|| format!("entry {}: internal SHA-256 state missing", e.name))?,
        len,
    ))
}

/// Shared core: seek to the entry's data and stream it through the decompressor,
/// always deriving CRC-32 and optionally deriving SHA-256 and/or collecting the
/// bytes. Enforces the declared uncompressed size.
fn entry_integrity_inner(
    file: &mut File,
    e: &CdEntry,
    output: Option<&mut dyn Write>,
    calculate_sha256: bool,
    output_cap: u64,
) -> Result<(Option<String>, u64), String> {
    if e.uncomp_size > output_cap || e.comp_size > STREAMING_MAX_CONTAINER_ENTRY_BYTES {
        return Err(format!(
            "entry {} exceeds verifier cap (compressed {}, uncompressed {}, output cap {})",
            e.name, e.comp_size, e.uncomp_size, output_cap
        ));
    }
    // local file header: read fixed 30 bytes, then name+extra len to find data.
    let mut lfh = [0u8; 30];
    file.seek(SeekFrom::Start(e.local_header_off))
        .map_err(|err| format!("seek local header {}: {err}", e.name))?;
    file.read_exact(&mut lfh)
        .map_err(|err| format!("read local header {}: {err}", e.name))?;
    if rd_u32(&lfh, 0) != Some(LFH_SIG) {
        return Err(format!("entry {}: bad local header signature", e.name));
    }
    let lh_version_needed = rd_u16(&lfh, 4).unwrap();
    let lh_flags = rd_u16(&lfh, 6).unwrap();
    let lh_method = rd_u16(&lfh, 8).unwrap();
    let lh_mod_time = rd_u16(&lfh, 10).unwrap();
    let lh_mod_date = rd_u16(&lfh, 12).unwrap();
    let lh_crc32 = rd_u32(&lfh, 14).unwrap();
    let lh_comp_size = rd_u32(&lfh, 18).unwrap() as u64;
    let lh_uncomp_size = rd_u32(&lfh, 22).unwrap() as u64;
    // Mirror the in-RAM reader's local/central agreement checks
    // (zip.rs validate_local_entry) exactly: a container the product reader
    // refuses must never pass the recorder-side streaming verifier.
    if lh_version_needed != e.version_needed {
        return Err(format!(
            "entry {}: local/central version-needed mismatch (local {lh_version_needed}, central {})",
            e.name, e.version_needed
        ));
    }
    if lh_flags != e.flags {
        return Err(format!(
            "entry {}: local/central flags mismatch (local 0x{lh_flags:04x}, central 0x{:04x})",
            e.name, e.flags
        ));
    }
    if lh_method != e.method {
        return Err(format!("entry {}: local/central method mismatch", e.name));
    }
    if lh_mod_time != e.mod_time || lh_mod_date != e.mod_date {
        return Err(format!(
            "entry {}: local/central timestamp mismatch",
            e.name
        ));
    }
    let lh_name_len = rd_u16(&lfh, 26).unwrap() as u64;
    let lh_extra_len = rd_u16(&lfh, 28).unwrap() as u64;
    let data_start = e
        .local_header_off
        .checked_add(30)
        .and_then(|v| v.checked_add(lh_name_len))
        .and_then(|v| v.checked_add(lh_extra_len))
        .ok_or_else(|| format!("entry {}: data start overflow", e.name))?;
    let name_start = e
        .local_header_off
        .checked_add(30)
        .ok_or_else(|| format!("entry {}: local name start overflow", e.name))?;
    file.seek(SeekFrom::Start(name_start))
        .map_err(|err| format!("seek local name {}: {err}", e.name))?;
    let mut local_name = vec![0u8; lh_name_len as usize];
    file.read_exact(&mut local_name)
        .map_err(|err| format!("read local name {}: {err}", e.name))?;
    let mut local_extra = vec![0u8; lh_extra_len as usize];
    file.read_exact(&mut local_extra)
        .map_err(|err| format!("read local extra {}: {err}", e.name))?;
    if local_name != e.name.as_bytes() {
        return Err(format!("entry {}: local/central name mismatch", e.name));
    }
    if e.flags & 0x0008 == 0 {
        let need_uncomp = lh_uncomp_size == u32::MAX as u64;
        let need_comp = lh_comp_size == u32::MAX as u64;
        let zip64 = parse_zip64_extra(&local_extra, need_uncomp, need_comp, false)?;
        let local_uncomp_size = if need_uncomp {
            zip64
                .uncomp_size
                .ok_or_else(|| format!("entry {}: local Zip64 size missing", e.name))?
        } else {
            lh_uncomp_size
        };
        let local_comp_size = if need_comp {
            zip64
                .comp_size
                .ok_or_else(|| format!("entry {}: local Zip64 size missing", e.name))?
        } else {
            lh_comp_size
        };
        if lh_crc32 != e.crc32
            || local_comp_size != e.comp_size
            || local_uncomp_size != e.uncomp_size
        {
            return Err(format!(
                "entry {}: local/central crc or size mismatch",
                e.name
            ));
        }
    } else if lh_crc32 != 0 || lh_comp_size != 0 || lh_uncomp_size != 0 {
        // Same rule as the in-RAM reader: with general-purpose bit 3 the local
        // crc/size fields carry no authority and MUST be zero.
        return Err(format!(
            "entry {}: data-descriptor local crc/size fields must be zero",
            e.name
        ));
    }
    file.seek(SeekFrom::Start(data_start))
        .map_err(|err| format!("seek entry data {}: {err}", e.name))?;

    // A sink that owns the hasher and optionally collects decompressed output.
    let sink = HashSink {
        hasher: calculate_sha256.then(Sha256::new),
        crc32: crc32fast::Hasher::new(),
        output,
        len: 0,
        limit: e.uncomp_size.saturating_add(1),
    };

    let final_sink = match e.method {
        ZIP_METHOD_STORE => {
            // STORE: comp_size == uncomp_size; stream raw bytes through the sink.
            let mut s = sink;
            stream_n(file, e.comp_size, |chunk| {
                s.write_all(chunk).map_err(|e| e.to_string())
            })?;
            s
        }
        ZIP_METHOD_DEFLATE => {
            let mut s = sink;
            let compressed = std::io::Read::by_ref(file).take(e.comp_size);
            let mut dec = DeflateDecoder::new(compressed);
            let mut out = vec![0u8; VERIFY_CHUNK];
            loop {
                let n = dec
                    .read(&mut out)
                    .map_err(|error| format!("entry {}: inflate failed: {error}", e.name))?;
                if n == 0 {
                    break;
                }
                s.write_all(&out[..n]).map_err(|error| error.to_string())?;
            }
            // Match the in-memory reader: every byte inside the declared
            // compressed region must belong to the deflate stream.
            if dec.total_in() != e.comp_size {
                return Err(format!(
                    "entry {}: trailing bytes after deflate stream",
                    e.name
                ));
            }
            drop(dec);
            file.seek(SeekFrom::Start(
                data_start
                    .checked_add(e.comp_size)
                    .ok_or_else(|| format!("entry {}: data end overflow", e.name))?,
            ))
            .map_err(|error| format!("entry {}: seek after deflate data: {error}", e.name))?;
            s
        }
        ZIP_METHOD_ZSTD => {
            let mut s = sink;
            {
                let limited = std::io::Read::by_ref(file).take(e.comp_size);
                let mut dec = zstd::stream::read::Decoder::new(limited)
                    .map_err(|err| format!("entry {}: zstd decoder: {err}", e.name))?;
                dec.window_log_max(ZSTD_WINDOW_LOG)
                    .map_err(|err| format!("entry {}: zstd decoder: {err}", e.name))?;
                let mut out = vec![0u8; VERIFY_CHUNK];
                loop {
                    let n = dec
                        .read(&mut out)
                        .map_err(|err| format!("entry {}: zstd decode failed: {err}", e.name))?;
                    if n == 0 {
                        break;
                    }
                    s.write_all(&out[..n]).map_err(|err| err.to_string())?;
                }
            }
            file.seek(SeekFrom::Start(
                data_start
                    .checked_add(e.comp_size)
                    .ok_or_else(|| format!("entry {}: data end overflow", e.name))?,
            ))
            .map_err(|err| format!("entry {}: seek after zstd data: {err}", e.name))?;
            s
        }
        m => return Err(format!("entry {}: unsupported method {m}", e.name)),
    };

    let uncomp_len = final_sink.len;
    if uncomp_len != e.uncomp_size {
        return Err(format!(
            "entry {}: decompressed {uncomp_len} bytes != declared {}",
            e.name, e.uncomp_size
        ));
    }
    let crc32 = final_sink.crc32.finalize();
    if crc32 != e.crc32 {
        // Keep the historical "crc32 mismatch" phrase — the negative-fixture
        // manifest and replay_app's fixture table pin it as the stable
        // substring for this refusal class.
        return Err(format!(
            "entry {}: crc32 mismatch: {crc32:08x} != central {:08x}",
            e.name, e.crc32
        ));
    }
    if e.flags & 0x0008 != 0 {
        let descriptor_start = data_start
            .checked_add(e.comp_size)
            .ok_or_else(|| format!("entry {}: data descriptor offset overflow", e.name))?;
        data_descriptor_end_file(file, e, descriptor_start)?;
    }
    Ok((
        final_sink.hasher.map(|hasher| to_hex(&hasher.finalize())),
        uncomp_len,
    ))
}

fn data_descriptor_end_file(file: &mut File, e: &CdEntry, start: u64) -> Result<u64, String> {
    let signed = data_descriptor_candidate_file(file, e, start, true)?;
    let unsigned = data_descriptor_candidate_file(file, e, start, false)?;
    match (signed, unsigned) {
        (Some(end), None) | (None, Some(end)) => {
            file.seek(SeekFrom::Start(end)).map_err(|error| {
                format!("entry {}: seek after data descriptor: {error}", e.name)
            })?;
            Ok(end)
        }
        (Some(_), Some(_)) => Err(format!(
            "entry {}: ambiguous signed/unsigned data descriptor",
            e.name
        )),
        (None, None) => Err(format!(
            "entry {}: data descriptor missing, truncated, or inconsistent with central directory",
            e.name
        )),
    }
}

fn data_descriptor_candidate_file(
    file: &mut File,
    e: &CdEntry,
    start: u64,
    signed: bool,
) -> Result<Option<u64>, String> {
    let zip64 = e.comp_size > ZIP32_MAX_FIELD_VALUE || e.uncomp_size > ZIP32_MAX_FIELD_VALUE;
    let length = (if signed { 4 } else { 0 }) + 4 + if zip64 { 16 } else { 8 };
    let mut data = vec![0u8; length];
    file.seek(SeekFrom::Start(start))
        .map_err(|error| format!("entry {}: seek data descriptor: {error}", e.name))?;
    if let Err(error) = file.read_exact(&mut data) {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(format!("entry {}: read data descriptor: {error}", e.name));
    }

    let mut p = 0usize;
    if signed {
        if rd_u32(&data, p) != Some(0x0807_4b50) {
            return Ok(None);
        }
        p += 4;
    }
    let crc32 = rd_u32(&data, p).ok_or("truncated data descriptor")?;
    p += 4;
    let (comp_size, uncomp_size) = if zip64 {
        let comp_size = rd_u64(&data, p).ok_or("truncated data descriptor")?;
        p += 8;
        let uncomp_size = rd_u64(&data, p).ok_or("truncated data descriptor")?;
        (comp_size, uncomp_size)
    } else {
        let comp_size = rd_u32(&data, p).ok_or("truncated data descriptor")? as u64;
        p += 4;
        let uncomp_size = rd_u32(&data, p).ok_or("truncated data descriptor")? as u64;
        (comp_size, uncomp_size)
    };
    if crc32 != e.crc32 || comp_size != e.comp_size || uncomp_size != e.uncomp_size {
        return Ok(None);
    }
    start
        .checked_add(length as u64)
        .map(Some)
        .ok_or_else(|| format!("entry {}: data descriptor end overflow", e.name))
}

/// Read exactly `n` bytes from `file` in chunks, handing each chunk to `f`.
fn stream_n(
    file: &mut File,
    n: u64,
    mut f: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut remaining = n;
    let mut buf = vec![0u8; VERIFY_CHUNK];
    while remaining > 0 {
        let want = remaining.min(buf.len() as u64) as usize;
        file.read_exact(&mut buf[..want])
            .map_err(|e| format!("read entry data: {e}"))?;
        f(&buf[..want])?;
        remaining -= want as u64;
    }
    Ok(())
}

/// A `Write` sink that always derives CRC-32 and optionally hashes/copies.
struct HashSink<'a> {
    hasher: Option<Sha256>,
    crc32: crc32fast::Hasher,
    output: Option<&'a mut dyn Write>,
    len: u64,
    limit: u64,
}
impl Write for HashSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let next = self.len.checked_add(buf.len() as u64).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "entry length overflow")
        })?;
        if next > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "entry exceeded declared uncompressed size cap {}",
                    self.limit
                ),
            ));
        }
        if let Some(hasher) = &mut self.hasher {
            hasher.update(buf);
        }
        self.crc32.update(buf);
        if let Some(output) = self.output.as_deref_mut() {
            output.write_all(buf)?;
        }
        self.len = next;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Backwards scan for the EOCD signature within a tail buffer.
fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    let mut i = tail.len() - 22;
    loop {
        if rd_u32(tail, i) == Some(EOCD_SIG) && eocd_comment_ends_at_eof(tail, i) {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn eocd_comment_ends_at_eof(tail: &[u8], off: usize) -> bool {
    let comment_len = match rd_u16(tail, off + 20) {
        Some(len) => len as usize,
        None => return false,
    };
    off.checked_add(22)
        .and_then(|end| end.checked_add(comment_len))
        == Some(tail.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{Container, ExpectedIdentity};
    use crate::pack::{
        pack, pack_streaming_to_writer, pack_streaming_zstd_to_writer,
        pack_streaming_zstd_to_writer_with_profile, BorrowedPackEntry, CompressionProfile,
        FileBackedPackEntry, PackEntry, PackIdentity, StreamingFileEntry,
        DEFAULT_ZSTD_COMPRESSION_LEVEL,
    };
    use std::fs;
    use std::io::{Read, Seek, SeekFrom};
    use std::path::Path;
    use std::path::PathBuf;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let mut p = std::env::temp_dir();
            let uniq = format!(
                "hoi4_stream_verify_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            p.push(uniq);
            fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
        fn join(&self, n: &str) -> PathBuf {
            self.0.join(n)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn manifest_selects_container_versioned_path_policy() {
        let v1 = serde_json::json!({
            "container_version": 1,
            "required_features": []
        });
        assert!(!manifest_uses_unicode_paths(&v1).unwrap());

        let portable_v1 = serde_json::json!({
            "container_version": 1,
            "required_features": ["portable_content_manifest_v2"]
        });
        assert!(manifest_uses_unicode_paths(&portable_v1).unwrap());

        let v2 = serde_json::json!({
            "container_version": 2,
            "required_features": ["timeline_segments_v1"]
        });
        assert!(manifest_uses_unicode_paths(&v2).unwrap());

        let malformed_v2 = serde_json::json!({
            "container_version": 2,
            "required_features": []
        });
        assert!(manifest_uses_unicode_paths(&malformed_v2).is_err());
    }

    #[test]
    fn archive_and_manifest_joins_share_the_versioned_unicode_namespace() {
        let decomposed = "payload/g/CAFE\u{301}.json";
        assert!(versioned_path_key(decomposed, true, false).is_ok());
        assert!(versioned_path_key(decomposed, true, true).is_err());

        let sharp_s = "payload/g/Stra\u{df}e.json";
        let expanded = "payload/g/STRASSE.json";
        assert_ne!(
            versioned_path_key(sharp_s, true, false).unwrap(),
            versioned_path_key(expanded, true, false).unwrap()
        );
        assert_eq!(
            versioned_path_key(sharp_s, true, true).unwrap(),
            versioned_path_key(expanded, true, true).unwrap()
        );

        let entries = [
            dummy_cd_entry("manifest.json"),
            dummy_cd_entry(sharp_s),
            dummy_cd_entry(expanded),
        ];
        assert!(validate_archive_paths(&entries, false).is_ok());
        assert!(validate_archive_paths(&entries, true)
            .unwrap_err()
            .contains("portable path collision"));
    }

    fn dummy_cd_entry(name: &str) -> CdEntry {
        CdEntry {
            name: name.to_string(),
            version_needed: 20,
            flags: 0,
            method: ZIP_METHOD_STORE,
            mod_time: 0,
            mod_date: 0,
            crc32: 0,
            comp_size: 0,
            uncomp_size: 0,
            local_header_off: 0,
        }
    }

    fn zip64_empty_with_entry_count(total_entries: u64, comment_len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let zip64_eocd_off = out.len() as u64;
        out.extend_from_slice(&ZIP64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&44u64.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&total_entries.to_le_bytes());
        out.extend_from_slice(&total_entries.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());

        out.extend_from_slice(&ZIP64_LOCATOR_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&zip64_eocd_off.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());

        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&(comment_len as u16).to_le_bytes());
        out.extend(std::iter::repeat(b'z').take(comment_len));
        out
    }

    fn add_comment_with_fake_eocd(mut data: Vec<u8>) -> Vec<u8> {
        let eocd = find_eocd(&data).expect("base archive eocd");
        assert_eq!(rd_u16(&data, eocd + 20), Some(0));
        let mut comment = vec![b'x'; 40];
        comment[8..12].copy_from_slice(&EOCD_SIG.to_le_bytes());
        comment[28..30].copy_from_slice(&0u16.to_le_bytes());
        data[eocd + 20..eocd + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        data.extend_from_slice(&comment);
        data
    }

    fn build_store_streaming_container(dir: &TmpDir, tag: &str) -> (PathBuf, Vec<u8>, String) {
        let stream_path = dir.join(&format!("{tag}.bin"));
        let stream_bytes = (0..4096u32)
            .map(|i| (i.wrapping_mul(37) >> 3) as u8)
            .collect::<Vec<_>>();
        fs::write(&stream_path, &stream_bytes).unwrap();
        let stream_sha = to_hex(&Sha256::digest(&stream_bytes));
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec();
        let borrowed = vec![BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        }];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];
        let container_path = dir.join(&format!("{tag}.battleplan"));
        let mut out = fs::File::create(&container_path).unwrap();
        pack_streaming_to_writer(
            "g",
            &borrowed,
            &[],
            &streaming,
            &PackIdentity::default(),
            false,
            &mut out,
        )
        .unwrap();
        (container_path, stream_bytes, stream_sha)
    }

    fn entry_data_range(path: &Path, name: &str) -> (u64, u64) {
        let mut file = fs::File::open(path).unwrap();
        let file_len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, file_len).unwrap();
        let entry = entries.iter().find(|entry| entry.name == name).unwrap();
        let mut lfh = [0u8; 30];
        file.seek(SeekFrom::Start(entry.local_header_off)).unwrap();
        file.read_exact(&mut lfh).unwrap();
        assert_eq!(rd_u32(&lfh, 0), Some(LFH_SIG));
        let name_len = rd_u16(&lfh, 26).unwrap() as u64;
        let extra_len = rd_u16(&lfh, 28).unwrap() as u64;
        let data_start = entry.local_header_off + 30 + name_len + extra_len;
        (data_start, entry.comp_size)
    }

    fn write_with_duplicate_stream_role_manifest(source: &Path, output: &Path) {
        let bytes = fs::read(source).unwrap();
        let eocd = find_eocd(&bytes).expect("fixture EOCD");
        assert!(
            find_zip64_locator(&bytes, eocd).is_none(),
            "small fixture unexpectedly uses Zip64"
        );
        let cd_offset = rd_u32(&bytes, eocd + 16).unwrap() as usize;
        let cd_size = rd_u32(&bytes, eocd + 12).unwrap() as usize;
        let mut cursor = cd_offset;
        let cd_end = cd_offset + cd_size;
        let mut manifest_cdh = None;
        while cursor < cd_end {
            assert_eq!(rd_u32(&bytes, cursor), Some(CDH_SIG));
            let name_len = rd_u16(&bytes, cursor + 28).unwrap() as usize;
            let extra_len = rd_u16(&bytes, cursor + 30).unwrap() as usize;
            let comment_len = rd_u16(&bytes, cursor + 32).unwrap() as usize;
            let name_start = cursor + 46;
            let name_end = name_start + name_len;
            if &bytes[name_start..name_end] == b"manifest.json" {
                manifest_cdh = Some(cursor);
            }
            cursor = name_end + extra_len + comment_len;
        }
        let manifest_cdh = manifest_cdh.expect("manifest central-directory record");
        assert_eq!(rd_u16(&bytes, manifest_cdh + 8), Some(0));
        assert_eq!(rd_u16(&bytes, manifest_cdh + 10), Some(ZIP_METHOD_STORE));
        let old_len = rd_u32(&bytes, manifest_cdh + 20).unwrap() as usize;
        assert_eq!(rd_u32(&bytes, manifest_cdh + 24), Some(old_len as u32));
        let local_header = rd_u32(&bytes, manifest_cdh + 42).unwrap() as usize;
        assert_eq!(rd_u32(&bytes, local_header), Some(LFH_SIG));
        let local_name_len = rd_u16(&bytes, local_header + 26).unwrap() as usize;
        let local_extra_len = rd_u16(&bytes, local_header + 28).unwrap() as usize;
        let data_start = local_header + 30 + local_name_len + local_extra_len;
        let original = &bytes[data_start..data_start + old_len];
        let needle = br#""role":"collector_compact_stream""#;
        let replacement = br#""role":"collector_compact_stream","role":"collector_compact_stream""#;
        let positions = original
            .windows(needle.len())
            .enumerate()
            .filter_map(|(index, window)| (window == needle).then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(positions.len(), 1, "fixture has one stream role");
        let role_offset = positions[0];
        let mut manifest = Vec::with_capacity(original.len() + replacement.len() - needle.len());
        manifest.extend_from_slice(&original[..role_offset]);
        manifest.extend_from_slice(replacement);
        manifest.extend_from_slice(&original[role_offset + needle.len()..]);

        let delta = manifest.len() - old_len;
        let new_len = u32::try_from(manifest.len()).unwrap();
        let crc = crc32fast::hash(&manifest);
        let mut tampered = Vec::with_capacity(bytes.len() + delta);
        tampered.extend_from_slice(&bytes[..data_start]);
        tampered.extend_from_slice(&manifest);
        tampered.extend_from_slice(&bytes[data_start + old_len..]);

        fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }

        put_u32(&mut tampered, local_header + 14, crc);
        put_u32(&mut tampered, local_header + 18, new_len);
        put_u32(&mut tampered, local_header + 22, new_len);
        let new_manifest_cdh = manifest_cdh + delta;
        put_u32(&mut tampered, new_manifest_cdh + 16, crc);
        put_u32(&mut tampered, new_manifest_cdh + 20, new_len);
        put_u32(&mut tampered, new_manifest_cdh + 24, new_len);
        let new_eocd = eocd + delta;
        put_u32(
            &mut tampered,
            new_eocd + 16,
            u32::try_from(cd_offset + delta).unwrap(),
        );
        fs::write(output, tampered).unwrap();
    }

    #[test]
    fn streaming_verifier_rejects_duplicate_role_in_raw_manifest() {
        let dir = TmpDir::new("duplicate_role");
        let (container_path, _, _) = build_store_streaming_container(&dir, "duplicate_role");
        let bad_path = dir.join("duplicate_role_bad.battleplan");
        write_with_duplicate_stream_role_manifest(&container_path, &bad_path);

        let error = verify_replay_container_streaming(&bad_path, "g", "collector_compact_stream")
            .expect_err("raw duplicate manifest members must block recorder promotion");
        assert!(error.contains("duplicate JSON member"), "{error}");
        assert!(error.contains("role"), "{error}");
    }

    #[test]
    fn streaming_verifier_rejects_two_role_entries_even_when_decoy_hash_is_last() {
        let dir = TmpDir::new("two_stream_role_entries");
        let captured = b"genuine captured stream";
        let container = pack(
            "g",
            &[
                PackEntry {
                    path: "payload/g/collector_raw/collector_stream.bin".into(),
                    bytes: b"substituted canonical stream".to_vec(),
                    role: "collector_compact_stream",
                    hour_index: None,
                },
                PackEntry {
                    path: "payload/g/decoy/captured_stream.bin".into(),
                    bytes: captured.to_vec(),
                    role: "collector_compact_stream",
                    hour_index: None,
                },
            ],
            &PackIdentity::default(),
            false,
        )
        .unwrap();
        let path = dir.join("two_stream_role_entries.battleplan");
        fs::write(&path, container).unwrap();

        let error = verify_replay_container_streaming(&path, "g", "collector_compact_stream")
            .expect_err("a last-ordered decoy role must never bind the capture hash");
        assert!(error.contains("exactly one"), "{error}");
        assert!(error.contains("collector_compact_stream"), "{error}");
    }

    #[test]
    fn streaming_decoder_refuses_deflate_trailing_bytes() {
        // The same [valid deflate stream][garbage] fixture the in-RAM reader
        // refuses: the write-side decoder must also fail (the two readers must
        // never disagree about the same container).
        let dir = TmpDir::new("deflate_trailing");
        let clean_path = dir.join("clean.zip");
        fs::write(&clean_path, crate::zip::test_deflate_zip_with_trailing(0)).unwrap();
        let mut file = fs::File::open(&clean_path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).expect("clean CD parses");
        hash_entry_streaming(&mut file, &entries[0]).expect("clean deflate entry hashes");

        let dirty_path = dir.join("dirty.zip");
        fs::write(&dirty_path, crate::zip::test_deflate_zip_with_trailing(16)).unwrap();
        let mut file = fs::File::open(&dirty_path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).expect("structure still parses");
        let err = hash_entry_streaming(&mut file, &entries[0])
            .expect_err("trailing bytes after the deflate stream must be refused");
        // Since the 2026-07-19 strictness port the streaming path raises the
        // in-RAM reader's canonical trailing-bytes refusal directly; the older
        // incidental short-write/inflate refusals remain acceptable spellings.
        assert!(
            err.contains("trailing bytes after deflate stream")
                || err.contains("failed to write whole buffer")
                || err.contains("inflate failed"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn streaming_reader_rejects_zip64_entry_count_above_reader_cap() {
        let dir = TmpDir::new("entry_count_cap");
        let path = dir.join("too_many_entries.battleplan");
        fs::write(
            &path,
            zip64_empty_with_entry_count(MAX_READER_ENTRIES + 1, 0),
        )
        .unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();

        let err = match read_central_directory(&mut file, len) {
            Ok(_) => panic!("entry-count cap should reject hostile Zip64 EOCD"),
            Err(err) => err,
        };
        assert!(
            err.contains("entry count") && err.contains("reader cap"),
            "{err}"
        );
    }

    #[test]
    fn streaming_tail_includes_zip64_locator_with_max_eocd_comment() {
        let dir = TmpDir::new("zip64_tail");
        let path = dir.join("zip64_max_comment.battleplan");
        fs::write(&path, zip64_empty_with_entry_count(0, 0xFFFF)).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();

        let entries = read_central_directory(&mut file, len)
            .expect("extended tail includes locator and Zip64 EOCD");
        assert!(entries.is_empty());
    }

    /// Empty Zip64 archive whose EOCD record carries a `sector_len`-byte
    /// zip64 extensible data sector (record size declares it) plus an EOCD
    /// comment — the layouts that push the record before a fixed scanned tail.
    fn zip64_empty_with_extensible_sector(sector_len: usize, comment_len: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let zip64_eocd_off = out.len() as u64;
        out.extend_from_slice(&ZIP64_EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&(44u64 + sector_len as u64).to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&45u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // cd size
        out.extend_from_slice(&0u64.to_le_bytes()); // cd offset
        out.extend(std::iter::repeat(0xEEu8).take(sector_len));

        out.extend_from_slice(&ZIP64_LOCATOR_SIG.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&zip64_eocd_off.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());

        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u16::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&u32::MAX.to_le_bytes());
        out.extend_from_slice(&(comment_len as u16).to_le_bytes());
        out.extend(std::iter::repeat(b'z').take(comment_len));
        out
    }

    #[test]
    fn streaming_zip64_extensible_data_sector_before_tail_is_accepted() {
        let dir = TmpDir::new("zip64_sector");

        // Nonempty extensible sector + max-size EOCD comment: the record
        // starts before the fixed scanned tail and must be reached through
        // the locator, exactly as the in-memory parser does.
        let path = dir.join("sector_max_comment.battleplan");
        fs::write(&path, zip64_empty_with_extensible_sector(4096, 0xFFFF)).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len)
            .expect("Zip64 EOCD with extensible data sector and max comment parses");
        assert!(entries.is_empty());

        // A sector larger than 65,535 bytes alone pushes the record out of
        // any comment-sized tail window.
        let path = dir.join("large_sector.battleplan");
        fs::write(&path, zip64_empty_with_extensible_sector(70_000, 0)).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len)
            .expect("Zip64 EOCD with >64KiB extensible data sector parses");
        assert!(entries.is_empty());
    }

    #[test]
    fn streaming_zip64_record_span_beyond_cap_is_refused() {
        let dir = TmpDir::new("zip64_span_cap");
        let path = dir.join("oversized_sector.battleplan");
        fs::write(
            &path,
            zip64_empty_with_extensible_sector(2 * 1024 * 1024, 0),
        )
        .unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        let err = read_central_directory(&mut file, len)
            .expect_err("a record spanning past the sane cap is a typed refusal");
        assert!(err.contains("verifier cap"), "{err}");
    }

    #[test]
    fn streaming_verifier_rejects_local_header_identity_drift() {
        // The in-RAM reader (zip.rs validate_local_entry) refuses local
        // headers whose version-needed or modification timestamp disagree
        // with the central directory; the streaming verifier must refuse the
        // same bytes (the two readers must never disagree about a container).
        let dir = TmpDir::new("local_header_drift");
        let base = crate::zip::test_deflate_zip_with_trailing(0);

        // Control: the untampered entry hashes cleanly.
        let clean_path = dir.join("clean.zip");
        fs::write(&clean_path, &base).unwrap();
        let mut file = fs::File::open(&clean_path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).unwrap();
        hash_entry_streaming(&mut file, &entries[0]).expect("clean entry hashes");

        // Local version-needed drifts from the central directory.
        let mut tampered = base.clone();
        tampered[4] = tampered[4].wrapping_add(1); // LFH at 0; version-needed at +4
        let path = dir.join("version_drift.zip");
        fs::write(&path, &tampered).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).unwrap();
        let err = hash_entry_streaming(&mut file, &entries[0])
            .expect_err("local/central version-needed drift must be refused");
        assert!(err.contains("version-needed mismatch"), "{err}");

        // Local modification time drifts from the central directory.
        let mut tampered = base.clone();
        tampered[10] = tampered[10].wrapping_add(1); // LFH mod-time at +10
        let path = dir.join("timestamp_drift.zip");
        fs::write(&path, &tampered).unwrap();
        let mut file = fs::File::open(&path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).unwrap();
        let err = hash_entry_streaming(&mut file, &entries[0])
            .expect_err("local/central timestamp drift must be refused");
        assert!(err.contains("timestamp mismatch"), "{err}");
    }

    #[test]
    fn streaming_verifier_requires_zero_local_fields_with_data_descriptor() {
        // With general-purpose bit 3 the in-RAM reader requires the local
        // crc/size fields to be ZERO (zip.rs); the streaming verifier must
        // refuse the same nonzero-field container instead of promoting it.
        let dir = TmpDir::new("dd_zero_fields");
        let (container_path, _, _) = build_store_streaming_container(&dir, "dd_zero_fields");
        let mut bytes = fs::read(&container_path).unwrap();
        let mut file = fs::File::open(&container_path).unwrap();
        let len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, len).unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.flags & 0x0008 != 0)
            .expect("streaming container has a data-descriptor entry");
        let crc_off = usize::try_from(entry.local_header_off).unwrap() + 14;
        assert_eq!(
            rd_u32(&bytes, crc_off),
            Some(0),
            "packer writes zero local crc under bit 3"
        );
        bytes[crc_off..crc_off + 4].copy_from_slice(&0xDEAD_BEEFu32.to_le_bytes());
        let tampered_path = dir.join("dd_zero_fields_tampered.battleplan");
        fs::write(&tampered_path, &bytes).unwrap();

        let mut file = fs::File::open(&tampered_path).unwrap();
        let entries = read_central_directory(&mut file, len).unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.flags & 0x0008 != 0)
            .expect("tampered container still lists the data-descriptor entry");
        let err = hash_entry_streaming(&mut file, entry)
            .expect_err("nonzero local crc under a data descriptor must be refused");
        assert!(err.contains("must be zero"), "{err}");
    }

    #[test]
    fn streaming_eocd_collision_in_comment_is_ignored() {
        let dir = TmpDir::new("eocd_collision");
        let path = dir.join("collision.battleplan");
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec();
        let borrowed = vec![
            BorrowedPackEntry {
                path: "payload/g/index.json".into(),
                bytes: &index,
                role: "index",
                hour_index: None,
            },
            BorrowedPackEntry {
                path: "payload/g/collector_raw/collector_stream.bin".into(),
                bytes: b"",
                role: "collector_compact_stream",
                hour_index: None,
            },
        ];
        let mut base = Vec::new();
        crate::pack::pack_borrowed_to_writer(
            "g",
            &borrowed,
            &PackIdentity::default(),
            false,
            &mut base,
        )
        .unwrap();
        fs::write(&path, add_comment_with_fake_eocd(base)).unwrap();

        let report = verify_replay_container_streaming(&path, "g", "collector_compact_stream")
            .expect("collision inside EOCD comment does not hide real EOCD");
        assert_eq!(report.entries_verified, 2);
    }

    fn build_zstd_streaming_container(dir: &TmpDir, tag: &str) -> (PathBuf, Vec<u8>, String) {
        let stream_path = dir.join(&format!("{tag}.bin"));
        let stream_bytes = (0..(256 * 1024u32))
            .map(|i| (i.wrapping_mul(97) >> 5) as u8)
            .collect::<Vec<_>>();
        fs::write(&stream_path, &stream_bytes).unwrap();
        let stream_sha = to_hex(&Sha256::digest(&stream_bytes));
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":0,"date":"1936.1.1.1","file":"snapshots/000000000000.snapshot.json"}]}"#.to_vec();
        let borrowed = vec![BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        }];
        let snap_path = dir.join(&format!("{tag}.snapshot.json"));
        let snap = format!(
            r#"{{"game_id":"g","schema_version":"1","hour_index":0,"date":"1936.1.1.1","countries":{{"GER":{{"blob":"{}"}}}}}}"#,
            "x".repeat(64 * 1024)
        );
        fs::write(&snap_path, snap.as_bytes()).unwrap();
        let files = vec![FileBackedPackEntry {
            path: "payload/g/snapshots/000000000000.snapshot.json".into(),
            file_path: snap_path.as_path(),
            role: "snapshot",
            hour_index: Some(0),
        }];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];
        let container_path = dir.join(&format!("{tag}.battleplan"));
        let mut out = fs::File::create(&container_path).unwrap();
        pack_streaming_zstd_to_writer(
            "g",
            &borrowed,
            &files,
            &streaming,
            &PackIdentity::default(),
            &mut out,
        )
        .unwrap();
        (container_path, stream_bytes, stream_sha)
    }

    /// Build a `.battleplan` via the STREAMING packer (a large streamed stream
    /// entry + two file-backed snapshot entries + one small in-RAM index entry),
    /// then confirm: (1) the streaming verifier hash-matches every entry and
    /// returns the stream role's sha; (2) the EXISTING in-RAM `Container` reader
    /// still opens it and can read each entry back individually (backward-compat
    /// of the data-descriptor zip); (3) a one-byte tamper of the stream on-disk is
    /// caught by the streaming verifier.
    #[test]
    fn streaming_pack_roundtrips_and_verifies() {
        let dir = TmpDir::new("roundtrip");
        // A stream large enough to exercise multi-chunk streaming compression.
        let stream_path = dir.join("collector_stream.bin");
        let mut stream_bytes = Vec::new();
        for i in 0..(5 * 1024 * 1024u32) {
            stream_bytes.push((i.wrapping_mul(2654435761) >> 13) as u8);
        }
        fs::write(&stream_path, &stream_bytes).unwrap();
        let stream_sha = to_hex(&Sha256::digest(&stream_bytes));

        let snap_a = dir.join("a.json");
        let snap_b = dir.join("b.json");
        fs::write(&snap_a, br#"{"hour_index":0,"x":1}"#).unwrap();
        fs::write(&snap_b, br#"{"hour_index":1,"y":2}"#).unwrap();

        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec();
        let borrowed = vec![BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        }];
        let files = vec![
            FileBackedPackEntry {
                path: "payload/g/snapshots/000000000000.snapshot.json".into(),
                file_path: &snap_a,
                role: "snapshot",
                hour_index: Some(0),
            },
            FileBackedPackEntry {
                path: "payload/g/snapshots/000000000001.snapshot.json".into(),
                file_path: &snap_b,
                role: "snapshot",
                hour_index: Some(1),
            },
        ];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];

        let container_path = dir.join("out.battleplan");
        {
            let mut out = fs::File::create(&container_path).unwrap();
            pack_streaming_to_writer(
                "g",
                &borrowed,
                &files,
                &streaming,
                &PackIdentity::default(),
                true,
                &mut out,
            )
            .unwrap();
        }

        // (1) streaming verify
        let report =
            verify_replay_container_streaming(&container_path, "g", "collector_compact_stream")
                .expect("streaming verify passes");
        assert_eq!(report.entries_verified, 4); // index + 2 snapshots + stream
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
        assert_eq!(report.stream_entry_bytes, stream_bytes.len() as u64);

        // (2) product file-backed reader opens the streamed container and reads
        // each entry back individually (data-descriptor ZIP is compatible).
        let container = Container::open(&container_path, &ExpectedIdentity::default())
            .expect("file-backed Container opens streamed container");
        let got_stream = container
            .read_entry_bytes("payload/g/collector_raw/collector_stream.bin")
            .expect("read stream entry");
        assert_eq!(got_stream, stream_bytes);
        let got_a = container
            .read_entry_bytes("payload/g/snapshots/000000000000.snapshot.json")
            .unwrap();
        assert_eq!(got_a, br#"{"hour_index":0,"x":1}"#);

        // (3) tamper the stream on disk -> streaming verify must reject.
        let bad_path = dir.join("bad.battleplan");
        let mut bytes = fs::read(&container_path).unwrap();
        // flip a byte in the compressed stream region (well before the central dir).
        bytes[200] ^= 0xff;
        fs::write(&bad_path, &bytes).unwrap();
        let tampered =
            verify_replay_container_streaming(&bad_path, "g", "collector_compact_stream");
        assert!(tampered.is_err(), "tampered container must fail verify");
    }

    #[test]
    fn zstd_streaming_pack_roundtrips_both_readers() {
        let dir = TmpDir::new("zstd_roundtrip");
        let (container_path, stream_bytes, stream_sha) =
            build_zstd_streaming_container(&dir, "zstd_roundtrip");

        let mut file = fs::File::open(&container_path).unwrap();
        let file_len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, file_len).unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "payload/g/collector_raw/collector_stream.bin")
                .unwrap()
                .method,
            ZIP_METHOD_ZSTD
        );
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "payload/g/snapshots/000000000000.snapshot.json")
                .unwrap()
                .method,
            ZIP_METHOD_ZSTD
        );

        let report =
            verify_replay_container_streaming(&container_path, "g", "collector_compact_stream")
                .expect("streaming verify zstd container");
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
        assert_eq!(report.stream_entry_bytes, stream_bytes.len() as u64);

        let container = Container::open(&container_path, &ExpectedIdentity::default())
            .expect("in-RAM reader opens zstd container");
        let got_stream = container
            .read_entry_bytes("payload/g/collector_raw/collector_stream.bin")
            .expect("read zstd stream entry");
        assert_eq!(got_stream, stream_bytes);
        let snap = container.load(0).expect("load zstd snapshot");
        assert_eq!(snap.hour_index, Some(0));
    }

    #[test]
    fn zstd_source_is_decoded_before_replay_packing() {
        let dir = TmpDir::new("zstd_source_pack");
        let raw_path = dir.join("raw.bin");
        let encoded_path = dir.join("collector_stream.bin");
        let replay_path = dir.join("encoded_source.battleplan");
        let raw = vec![0x5au8; 256 * 1024];
        fs::write(&raw_path, &raw).unwrap();
        compact_stream::encode_file(
            &raw_path,
            &encoded_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        let stream_sha = to_hex(&Sha256::digest(&raw));
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#;
        let timeline = br#"{}"#;
        let borrowed = vec![
            BorrowedPackEntry {
                path: "payload/g/index.json".into(),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            BorrowedPackEntry {
                path: "payload/g/timeline.json".into(),
                bytes: timeline,
                role: "timeline",
                hour_index: None,
            },
        ];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &encoded_path,
            source_compression: compact_stream::StreamCompression::Zstd,
            decoded_len: Some(raw.len() as u64),
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];
        let mut out = fs::File::create(&replay_path).unwrap();
        pack_streaming_zstd_to_writer(
            "g",
            &borrowed,
            &[],
            &streaming,
            &PackIdentity::default(),
            &mut out,
        )
        .unwrap();
        drop(out);

        let report =
            verify_replay_container_streaming(&replay_path, "g", "collector_compact_stream")
                .unwrap();
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
        assert_eq!(report.stream_entry_bytes, raw.len() as u64);
        let container =
            crate::Container::open(&replay_path, &crate::ExpectedIdentity::default()).unwrap();
        assert_eq!(
            container
                .read_entry_bytes("payload/g/collector_raw/collector_stream.bin")
                .unwrap(),
            raw
        );
    }

    /// The opt-in high-compression profile (level 19 at window-27 + LDM) MUST
    /// remain losslessly reversible, and the container it
    /// produces MUST decode under the production window cap (the weaker-PC
    /// decode bound). This is the zero-data-loss proof for the level flip:
    /// pack a realistic mixed payload (long repetitive runs to exercise LDM at
    /// window 27, interleaved with incompressible random blocks) through the
    /// exact production packing entry point at an EXPLICIT {19, 27} profile,
    /// then read every byte back and assert identity.
    #[test]
    fn level19_window27_roundtrip_is_byte_lossless() {
        // The production default prioritizes prompt finalization. Pin it so a
        // future silent change trips this test while retaining the explicit
        // level-19 roundtrip below.
        assert_eq!(
            DEFAULT_ZSTD_COMPRESSION_LEVEL, 3,
            "production default zstd level must remain level 3"
        );
        let dir = TmpDir::new("level19_w27_roundtrip");
        let raw_path = dir.join("raw.bin");
        let encoded_path = dir.join("collector_stream.bin");
        let replay_path = dir.join("level19.battleplan");

        // ~1.75 MiB: repetitive runs (LDM-matchable across a wide window) spliced
        // with random blocks that cannot be compressed, so a lossy round-trip
        // would diverge somewhere.
        let mut raw = Vec::with_capacity(1_835_008);
        let mut state: u32 = 0x1234_5678;
        for block in 0..112u32 {
            // 8 KiB repetitive
            raw.extend(std::iter::repeat((block & 0xff) as u8).take(8 * 1024));
            // 8 KiB pseudo-random (xorshift; no Math.random needed, deterministic)
            for _ in 0..(8 * 1024) {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                raw.push((state & 0xff) as u8);
            }
        }
        fs::write(&raw_path, &raw).unwrap();
        compact_stream::encode_file(
            &raw_path,
            &encoded_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        let stream_sha = to_hex(&Sha256::digest(&raw));
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#;
        let timeline = br#"{}"#;
        let borrowed = vec![
            BorrowedPackEntry {
                path: "payload/g/index.json".into(),
                bytes: index,
                role: "index",
                hour_index: None,
            },
            BorrowedPackEntry {
                path: "payload/g/timeline.json".into(),
                bytes: timeline,
                role: "timeline",
                hour_index: None,
            },
        ];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &encoded_path,
            source_compression: compact_stream::StreamCompression::Zstd,
            decoded_len: Some(raw.len() as u64),
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];

        // Explicit opt-in profile: level 19, window 27. validate() rejects an
        // out-of-bound window, so this also guards the window cap.
        let profile = CompressionProfile {
            zstd_level: 19,
            zstd_window_log: crate::zip::ZSTD_WINDOW_LOG,
        };
        assert_eq!(profile.zstd_window_log, 27);
        {
            let mut out = fs::File::create(&replay_path).unwrap();
            pack_streaming_zstd_to_writer_with_profile(
                "g",
                &borrowed,
                &[],
                &streaming,
                &PackIdentity::default(),
                profile,
                &mut out,
            )
            .unwrap();
        }

        // (1) streaming verify decodes under the production window cap and the
        // capture-time sha256 matches the decoded stream.
        let report =
            verify_replay_container_streaming(&replay_path, "g", "collector_compact_stream")
                .expect("level-19/window-27 container verifies under the production window cap");
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
        assert_eq!(report.stream_entry_bytes, raw.len() as u64);

        // (2) the entry really is zstd (a silent STORE would hide a level bug).
        let mut file = fs::File::open(&replay_path).unwrap();
        let file_len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, file_len).unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|e| e.name == "payload/g/collector_raw/collector_stream.bin")
                .unwrap()
                .method,
            ZIP_METHOD_ZSTD
        );

        // (3) full byte-identity through the production reader = zero data loss.
        let container =
            crate::Container::open(&replay_path, &crate::ExpectedIdentity::default()).unwrap();
        let got = container
            .read_entry_bytes("payload/g/collector_raw/collector_stream.bin")
            .unwrap();
        assert_eq!(got.len(), raw.len());
        assert!(got == raw, "level-19 round-trip must be byte-identical");
    }

    #[test]
    fn corrupted_zstd_entry_is_rejected() {
        let dir = TmpDir::new("zstd_corrupt");
        let (container_path, _, _) = build_zstd_streaming_container(&dir, "zstd_corrupt");
        let (data_start, comp_size) = entry_data_range(
            &container_path,
            "payload/g/collector_raw/collector_stream.bin",
        );
        assert!(comp_size > 16);

        let bad_path = dir.join("zstd_corrupt_bad.battleplan");
        let mut bytes = fs::read(&container_path).unwrap();
        bytes[(data_start + comp_size / 2) as usize] ^= 0x40;
        fs::write(&bad_path, &bytes).unwrap();

        let err = verify_replay_container_streaming(&bad_path, "g", "collector_compact_stream")
            .expect_err("zstd payload corruption must fail verification");
        assert!(
            err.contains("zstd") || err.contains("crc32") || err.contains("sha256"),
            "{err}"
        );
    }

    #[test]
    fn zstd_zip64_entry_count_crossing_is_accepted_by_both_readers() {
        let dir = TmpDir::new("zip64_count");
        let empty: &[u8] = &[];
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec();
        let stream_path = dir.join("collector_stream.bin");
        let stream_bytes = vec![42u8; 128 * 1024];
        fs::write(&stream_path, &stream_bytes).unwrap();
        let stream_sha = to_hex(&Sha256::digest(&stream_bytes));
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: stream_path.as_path(),
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: Some(stream_sha.clone()),
        }];
        let mut borrowed = Vec::with_capacity(crate::zip::ZIP32_MAX_ENTRY_COUNT - 1);
        borrowed.push(BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        });
        for i in 0..(crate::zip::ZIP32_MAX_ENTRY_COUNT - 2) {
            borrowed.push(BorrowedPackEntry {
                path: format!("payload/g/padding/{i:05}.bin"),
                bytes: empty,
                role: "padding",
                hour_index: None,
            });
        }

        let container_path = dir.join("zip64_count.battleplan");
        {
            let mut out = fs::File::create(&container_path).unwrap();
            pack_streaming_zstd_to_writer(
                "g",
                &borrowed,
                &[],
                &streaming,
                &PackIdentity::default(),
                &mut out,
            )
            .unwrap();
        }
        let bytes = fs::read(&container_path).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"PK\x06\x06"));
        assert!(bytes.windows(4).any(|w| w == b"PK\x06\x07"));
        let mut file = fs::File::open(&container_path).unwrap();
        let file_len = file.metadata().unwrap().len();
        let entries = read_central_directory(&mut file, file_len).unwrap();
        assert_eq!(
            entries
                .iter()
                .find(|entry| entry.name == "payload/g/collector_raw/collector_stream.bin")
                .unwrap()
                .method,
            ZIP_METHOD_ZSTD
        );

        let container = Container::open(&container_path, &ExpectedIdentity::default())
            .expect("file-backed reader accepts zip64 entry count");
        assert_eq!(container.entries().len(), borrowed.len() + 1);
        assert_eq!(
            container
                .read_entry_bytes("payload/g/collector_raw/collector_stream.bin")
                .unwrap(),
            stream_bytes
        );
        let report =
            verify_replay_container_streaming(&container_path, "g", "collector_compact_stream")
                .expect("streaming verifier accepts zip64 entry count");
        assert_eq!(report.entries_verified, borrowed.len() + 1);
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
    }

    #[test]
    #[ignore = "multi-GiB streaming verifier stress; run explicitly for Phase 3"]
    fn streaming_verifier_accepts_multi_gib_zip64_zstd_entry() {
        let dir = TmpDir::new("multi_gib_zstd");
        let container_path = dir.join("multi_gib_zstd.battleplan");
        let chunk = vec![0u8; 8 * 1024 * 1024];
        let repeat = 257u64;
        let uncomp_size = chunk.len() as u64 * repeat;
        assert!(uncomp_size > MAX_CONTAINER_ENTRY_BYTES);

        let frame = zstd::stream::encode_all(&chunk[..], 1).unwrap();
        let mut sha = Sha256::new();
        let mut crc = crc32fast::Hasher::new();
        for _ in 0..repeat {
            sha.update(&chunk);
            crc.update(&chunk);
        }
        let stream_sha = to_hex(&sha.finalize());
        let stream_crc = crc.finalize();
        write_repeated_zstd_frame_container(
            &container_path,
            &frame,
            repeat,
            uncomp_size,
            stream_crc,
            &stream_sha,
        );

        let report =
            verify_replay_container_streaming(&container_path, "g", "collector_compact_stream")
                .expect("streaming verifier accepts >2 GiB zstd entry");
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
        assert_eq!(report.stream_entry_bytes, uncomp_size);

        let err = match Container::open(&container_path, &ExpectedIdentity::default()) {
            Ok(_) => panic!("materializing product reader keeps 2 GiB per-entry cap"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("cap") || err.to_string().contains("too large"),
            "{err}"
        );
    }

    fn write_repeated_zstd_frame_container(
        path: &Path,
        frame: &[u8],
        repeat: u64,
        uncomp_size: u64,
        stream_crc: u32,
        stream_sha: &str,
    ) {
        use crate::canon::manifest_canonical_bytes;
        use serde_json::json;

        let stream_name = "payload/g/collector_raw/collector_stream.bin";
        let comp_size = frame.len() as u64 * repeat;
        let mut manifest = json!({
            "format": "hoi4replay",
            "container_version": 1,
            "required_features": [],
            "optional_features": [],
            "schema_version": null,
            "game_id": "g",
            "build": { "exe_sha256": null, "pinned": false },
            "mod_identity": null,
            "created_utc": null,
            "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
            "counts": { "snapshots": 0 },
            "entries": [{
                "path": stream_name,
                "sha256": stream_sha,
                "bytes": uncomp_size,
                "role": "collector_compact_stream"
            }],
            "manifest_digest": null,
            "signature": null,
        });
        let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
        manifest
            .as_object_mut()
            .unwrap()
            .insert("manifest_digest".into(), json!(digest));
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_crc = crc32fast::hash(&manifest_bytes);

        struct Cd {
            name: &'static str,
            method: u16,
            crc: u32,
            comp_size: u64,
            uncomp_size: u64,
            local_off: u64,
            zip64_uncomp: bool,
        }
        let mut out = Vec::new();
        let mut cd = Vec::new();

        let stream_off = out.len() as u64;
        write_local_header(
            &mut out,
            stream_name,
            ZIP_METHOD_ZSTD,
            stream_crc,
            comp_size,
            uncomp_size,
            true,
        );
        for _ in 0..repeat {
            out.extend_from_slice(frame);
        }
        append_cd(
            &mut cd,
            Cd {
                name: stream_name,
                method: ZIP_METHOD_ZSTD,
                crc: stream_crc,
                comp_size,
                uncomp_size,
                local_off: stream_off,
                zip64_uncomp: true,
            },
        );

        let manifest_off = out.len() as u64;
        write_local_header(
            &mut out,
            "manifest.json",
            ZIP_METHOD_STORE,
            manifest_crc,
            manifest_bytes.len() as u64,
            manifest_bytes.len() as u64,
            false,
        );
        out.extend_from_slice(&manifest_bytes);
        append_cd(
            &mut cd,
            Cd {
                name: "manifest.json",
                method: ZIP_METHOD_STORE,
                crc: manifest_crc,
                comp_size: manifest_bytes.len() as u64,
                uncomp_size: manifest_bytes.len() as u64,
                local_off: manifest_off,
                zip64_uncomp: false,
            },
        );

        let cd_off = out.len() as u64;
        let cd_size = cd.len() as u64;
        out.extend_from_slice(&cd);
        let zip64_eocd_off = out.len() as u64;
        push_u32(&mut out, 0x0606_4b50);
        push_u64(&mut out, 44);
        push_u16(&mut out, 63);
        push_u16(&mut out, 45);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        push_u64(&mut out, 2);
        push_u64(&mut out, 2);
        push_u64(&mut out, cd_size);
        push_u64(&mut out, cd_off);
        push_u32(&mut out, 0x0706_4b50);
        push_u32(&mut out, 0);
        push_u64(&mut out, zip64_eocd_off);
        push_u32(&mut out, 1);
        push_u32(&mut out, 0x0605_4b50);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 2);
        push_u16(&mut out, 2);
        push_u32(&mut out, cd_size as u32);
        push_u32(&mut out, cd_off as u32);
        push_u16(&mut out, 0);
        fs::write(path, out).unwrap();

        fn write_local_header(
            out: &mut Vec<u8>,
            name: &str,
            method: u16,
            crc: u32,
            comp_size: u64,
            uncomp_size: u64,
            zip64_uncomp: bool,
        ) {
            let extra = if zip64_uncomp {
                zip64_extra(uncomp_size)
            } else {
                Vec::new()
            };
            push_u32(out, 0x0403_4b50);
            push_u16(out, if method == ZIP_METHOD_ZSTD { 63 } else { 20 });
            push_u16(out, 0);
            push_u16(out, method);
            push_u16(out, 0);
            push_u16(out, 0);
            push_u32(out, crc);
            push_u32(out, comp_size as u32);
            push_u32(
                out,
                if zip64_uncomp {
                    u32::MAX
                } else {
                    uncomp_size as u32
                },
            );
            push_u16(out, name.len() as u16);
            push_u16(out, extra.len() as u16);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&extra);
        }

        fn append_cd(cd: &mut Vec<u8>, entry: Cd) {
            let extra = if entry.zip64_uncomp {
                zip64_extra(entry.uncomp_size)
            } else {
                Vec::new()
            };
            push_u32(cd, 0x0201_4b50);
            push_u16(
                cd,
                if entry.method == ZIP_METHOD_ZSTD {
                    63
                } else {
                    20
                },
            );
            push_u16(
                cd,
                if entry.method == ZIP_METHOD_ZSTD {
                    63
                } else {
                    20
                },
            );
            push_u16(cd, 0);
            push_u16(cd, entry.method);
            push_u16(cd, 0);
            push_u16(cd, 0);
            push_u32(cd, entry.crc);
            push_u32(cd, entry.comp_size as u32);
            push_u32(
                cd,
                if entry.zip64_uncomp {
                    u32::MAX
                } else {
                    entry.uncomp_size as u32
                },
            );
            push_u16(cd, entry.name.len() as u16);
            push_u16(cd, extra.len() as u16);
            push_u16(cd, 0);
            push_u16(cd, 0);
            push_u16(cd, 0);
            push_u32(cd, 0);
            push_u32(cd, entry.local_off as u32);
            cd.extend_from_slice(entry.name.as_bytes());
            cd.extend_from_slice(&extra);
        }

        fn zip64_extra(uncomp_size: u64) -> Vec<u8> {
            let mut extra = Vec::new();
            push_u16(&mut extra, 0x0001);
            push_u16(&mut extra, 8);
            push_u64(&mut extra, uncomp_size);
            extra
        }

        fn push_u16(out: &mut Vec<u8>, value: u16) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        fn push_u32(out: &mut Vec<u8>, value: u32) {
            out.extend_from_slice(&value.to_le_bytes());
        }
        fn push_u64(out: &mut Vec<u8>, value: u64) {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }

    /// STORE (uncompressed) streaming entries verify too.
    #[test]
    fn streaming_pack_store_method_verifies() {
        let dir = TmpDir::new("store");
        let stream_path = dir.join("s.bin");
        let stream_bytes = vec![7u8; 100_000];
        fs::write(&stream_path, &stream_bytes).unwrap();
        let stream_sha = to_hex(&Sha256::digest(&stream_bytes));
        let index = br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec();
        let borrowed = vec![BorrowedPackEntry {
            path: "payload/g/index.json".into(),
            bytes: &index,
            role: "index",
            hour_index: None,
        }];
        let streaming = vec![StreamingFileEntry {
            path: "payload/g/collector_raw/collector_stream.bin".into(),
            file_path: &stream_path,
            source_compression: compact_stream::StreamCompression::None,
            decoded_len: None,
            role: "collector_compact_stream",
            hour_index: None,
            sha256: None, // exercise the pack-side hash path
        }];
        let container_path = dir.join("store.battleplan");
        {
            let mut out = fs::File::create(&container_path).unwrap();
            pack_streaming_to_writer(
                "g",
                &borrowed,
                &[],
                &streaming,
                &PackIdentity::default(),
                false, // STORE
                &mut out,
            )
            .unwrap();
        }
        let report =
            verify_replay_container_streaming(&container_path, "g", "collector_compact_stream")
                .expect("verify STORE container");
        assert_eq!(
            report.stream_entry_sha256.as_deref(),
            Some(stream_sha.as_str())
        );
    }

    #[test]
    fn crc_verification_catches_single_store_payload_byte_corruption() {
        let dir = TmpDir::new("crc");
        let (container_path, _, _) = build_store_streaming_container(&dir, "crc");
        let (data_start, _) = entry_data_range(
            &container_path,
            "payload/g/collector_raw/collector_stream.bin",
        );

        let bad_path = dir.join("crc_bad.battleplan");
        let mut bytes = fs::read(&container_path).unwrap();
        bytes[data_start as usize] ^= 0xff;
        fs::write(&bad_path, &bytes).unwrap();

        let err = verify_replay_container_streaming(&bad_path, "g", "collector_compact_stream")
            .expect_err("payload corruption must fail CRC verification");
        assert!(err.contains("crc32"), "{err}");
    }

    #[test]
    fn data_descriptor_validation_catches_tampered_descriptor() {
        let dir = TmpDir::new("descriptor");
        let (container_path, _, _) = build_store_streaming_container(&dir, "descriptor");
        let (data_start, comp_size) = entry_data_range(
            &container_path,
            "payload/g/collector_raw/collector_stream.bin",
        );
        let descriptor_crc_offset = data_start + comp_size + 4;

        let bad_path = dir.join("descriptor_bad.battleplan");
        let mut bytes = fs::read(&container_path).unwrap();
        bytes[descriptor_crc_offset as usize] ^= 0x01;
        fs::write(&bad_path, &bytes).unwrap();

        let err = verify_replay_container_streaming(&bad_path, "g", "collector_compact_stream")
            .expect_err("descriptor tamper must fail verification");
        assert!(err.contains("data descriptor"), "{err}");
    }
}
