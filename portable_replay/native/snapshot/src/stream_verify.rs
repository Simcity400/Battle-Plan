//! STREAMING `.battleplan` verifier (bounded memory).
//!
//! [`crate::container::Container`] reads the WHOLE archive into RAM (`fs::read`)
//! and caps total size at 64 GiB — fine for a replay a consumer opens, but the
//! recorder must self-verify a container whose raw-stream entry can be many GiB
//! without ever materializing it. This module re-reads the finished container
//! FROM DISK, one entry at a time, seeking to each local header and streaming its
//! compressed bytes through the decompressor while hashing incrementally. Peak
//! memory is O(one chunk + the central directory + the manifest JSON), not
//! O(archive). It performs the same integrity checks the in-RAM reader does
//! (manifest digest, per-entry sha256, manifest↔zip coverage, declared-size
//! agreement) and, for the collector stream entry, returns its decompressed
//! sha256 so the caller can compare it to the capture-time hash (the streaming
//! replacement for the old in-RAM "packed stream bytes == pipe bytes" compare).

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::zip::{
    validate_path, INITIAL_ENTRY_RESERVE, MAX_CONTAINER_ENTRY_BYTES, MAX_READER_ENTRIES,
    STREAMING_MAX_CONTAINER_ENTRY_BYTES, ZIP32_MAX_FIELD_VALUE, ZIP_METHOD_DEFLATE,
    ZIP_METHOD_STORE, ZIP_METHOD_ZSTD, ZSTD_WINDOW_LOG,
};
use flate2::write::DeflateDecoder;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const ZIP64_EXTRA_ID: u16 = 0x0001;
const EOCD_MIN_LEN: u64 = 22;
const ZIP64_LOCATOR_LEN: u64 = 20;
const ZIP64_EOCD_MIN_LEN: u64 = 56;
const VERIFY_CHUNK: usize = 4 * 1024 * 1024;

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
struct CdEntry {
    name: String,
    flags: u16,
    method: u16,
    crc32: u32,
    comp_size: u64,
    uncomp_size: u64,
    local_header_off: u64,
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
    validate_path(stream_payload_path)
        .map_err(|e| format!("invalid canonical stream payload path: {e}"))?;
    let file_len = file
        .metadata()
        .map_err(|e| format!("stat container: {e}"))?
        .len();

    let entries = read_central_directory(&mut file, file_len)?;

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
    for ent in man_entries {
        let p = ent
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| "entry.path missing".to_string())?;
        let sha = ent
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("entry {p}: sha256 missing"))?;
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
        let rec = by_path
            .get(&e.name)
            .expect("coverage check populated by_path");
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

/// Locate + parse the End-Of-Central-Directory and read the central directory
/// (bounded: the CD is small relative to the payloads).
fn read_central_directory(file: &mut File, file_len: u64) -> Result<Vec<CdEntry>, String> {
    if file_len < 22 {
        return Err("container too small for an EOCD".to_string());
    }
    // The EOCD sits within the last 22 + 0xFFFF bytes (comment cap). Include
    // the Zip64 locator + minimum Zip64 EOCD before it, because a max-size EOCD
    // comment otherwise pushes those records just outside the scanned tail.
    let tail_len = (EOCD_MIN_LEN + 0xFFFF + ZIP64_LOCATOR_LEN + ZIP64_EOCD_MIN_LEN).min(file_len);
    let tail_start = file_len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|e| format!("seek eocd: {e}"))?;
    file.read_exact(&mut tail)
        .map_err(|e| format!("read eocd tail: {e}"))?;

    let eocd = find_eocd(&tail).ok_or_else(|| "no EOCD record found".to_string())?;
    let total16 = rd_u16(&tail, eocd + 10).ok_or("truncated eocd")?;
    let cd_size32 = rd_u32(&tail, eocd + 12).ok_or("truncated eocd")?;
    let cd_off32 = rd_u32(&tail, eocd + 16).ok_or("truncated eocd")?;
    let locator = find_zip64_locator(&tail, eocd);
    let (total_u64, cd_size, cd_off) = if total16 == u16::MAX
        || cd_size32 == u32::MAX
        || cd_off32 == u32::MAX
        || locator.is_some()
    {
        let locator = locator.ok_or_else(|| "Zip64 locator missing".to_string())?;
        parse_zip64_eocd(&tail, tail_start, locator)?
    } else {
        (total16 as u64, cd_size32 as u64, cd_off32 as u64)
    };
    if total_u64 > MAX_READER_ENTRIES {
        return Err(format!(
            "central directory entry count {total_u64} exceeds reader cap {MAX_READER_ENTRIES}"
        ));
    }
    let total = usize::try_from(total_u64)
        .map_err(|_| "central directory entry count too large".to_string())?;
    if cd_off.saturating_add(cd_size) > file_len {
        return Err("central directory out of range".to_string());
    }

    let cd_size_usize =
        usize::try_from(cd_size).map_err(|_| "central directory too large".to_string())?;
    let mut cd = vec![0u8; cd_size_usize];
    file.seek(SeekFrom::Start(cd_off))
        .map_err(|e| format!("seek central dir: {e}"))?;
    file.read_exact(&mut cd)
        .map_err(|e| format!("read central dir: {e}"))?;

    let mut entries = Vec::with_capacity(total.min(INITIAL_ENTRY_RESERVE));
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut p = 0usize;
    while p + 46 <= cd.len() {
        let sig = rd_u32(&cd, p).ok_or("truncated cdh")?;
        if sig != CDH_SIG {
            break;
        }
        let flags = rd_u16(&cd, p + 8).ok_or("truncated cdh")?;
        let method = rd_u16(&cd, p + 10).ok_or("truncated cdh")?;
        let crc32 = rd_u32(&cd, p + 16).ok_or("truncated cdh")?;
        if flags & !0x0008 != 0 {
            return Err(format!(
                "entry has unsupported zip general-purpose flags 0x{flags:04x}"
            ));
        }
        let comp_size32 = rd_u32(&cd, p + 20).ok_or("truncated cdh")?;
        let uncomp_size32 = rd_u32(&cd, p + 24).ok_or("truncated cdh")?;
        let name_len = rd_u16(&cd, p + 28).ok_or("truncated cdh")? as usize;
        let extra_len = rd_u16(&cd, p + 30).ok_or("truncated cdh")? as usize;
        let comment_len = rd_u16(&cd, p + 32).ok_or("truncated cdh")? as usize;
        let lho32 = rd_u32(&cd, p + 42).ok_or("truncated cdh")?;
        let name_start = p + 46;
        let name_end = name_start + name_len;
        if name_end > cd.len() {
            return Err("central dir name past end".to_string());
        }
        let name = std::str::from_utf8(&cd[name_start..name_end])
            .map_err(|_| "non-utf8 entry name".to_string())?
            .to_string();
        let extra_start = name_end;
        let extra_end = extra_start
            .checked_add(extra_len)
            .ok_or("cdh extra overflow")?;
        if extra_end > cd.len() {
            return Err("central dir extra past end".to_string());
        }
        let extra = &cd[extra_start..extra_end];
        let need_uncomp = uncomp_size32 == u32::MAX;
        let need_comp = comp_size32 == u32::MAX;
        let need_lho = lho32 == u32::MAX;
        let zip64 = parse_zip64_extra(extra, need_uncomp, need_comp, need_lho)?;
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
        validate_path(&name).map_err(|e| format!("unsafe entry path {name}: {e}"))?;
        if !seen.insert(name.clone()) {
            return Err(format!("duplicate entry name {name}"));
        }
        entries.push(CdEntry {
            name,
            flags,
            method,
            crc32,
            comp_size,
            uncomp_size,
            local_header_off: lho,
        });
        let next = name_end
            .checked_add(extra_len)
            .and_then(|v| v.checked_add(comment_len))
            .ok_or("cdh advance overflow")?;
        if next > cd.len() {
            return Err("cdh extra/comment past central dir".to_string());
        }
        p = next;
    }
    if entries.len() != total {
        return Err(format!(
            "central directory entry count mismatch (parsed {}, EOCD {total})",
            entries.len()
        ));
    }
    if p != cd.len() {
        return Err("central directory trailing bytes".to_string());
    }
    Ok(entries)
}

#[derive(Default)]
struct Zip64Extra {
    uncomp_size: Option<u64>,
    comp_size: Option<u64>,
    local_header_off: Option<u64>,
}

fn find_zip64_locator(tail: &[u8], eocd: usize) -> Option<usize> {
    if eocd < 20 {
        return None;
    }
    let off = eocd - 20;
    (rd_u32(tail, off) == Some(ZIP64_LOCATOR_SIG)).then_some(off)
}

fn parse_zip64_eocd(
    tail: &[u8],
    tail_start: u64,
    locator: usize,
) -> Result<(u64, u64, u64), String> {
    if rd_u32(tail, locator + 4).ok_or("truncated zip64 locator")? != 0 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    let zip64_eocd_abs = rd_u64(tail, locator + 8).ok_or("truncated zip64 locator")?;
    let total_disks = rd_u32(tail, locator + 16).ok_or("truncated zip64 locator")?;
    if total_disks != 1 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    if zip64_eocd_abs < tail_start {
        return Err("Zip64 EOCD outside scanned tail".to_string());
    }
    let off = usize::try_from(zip64_eocd_abs - tail_start)
        .map_err(|_| "Zip64 EOCD offset too large".to_string())?;
    if off.checked_add(56).map_or(true, |end| end > tail.len()) {
        return Err("truncated Zip64 EOCD".to_string());
    }
    if rd_u32(tail, off) != Some(ZIP64_EOCD_SIG) {
        return Err("bad Zip64 EOCD signature".to_string());
    }
    let record_size = rd_u64(tail, off + 4).ok_or("truncated zip64 eocd")?;
    if record_size < 44 {
        return Err("truncated Zip64 EOCD size".to_string());
    }
    let disk_no = rd_u32(tail, off + 16).ok_or("truncated zip64 eocd")?;
    let cd_disk = rd_u32(tail, off + 20).ok_or("truncated zip64 eocd")?;
    if disk_no != 0 || cd_disk != 0 {
        return Err("multi-disk Zip64 archives are not supported".to_string());
    }
    let entries_on_disk = rd_u64(tail, off + 24).ok_or("truncated zip64 eocd")?;
    let total_entries = rd_u64(tail, off + 32).ok_or("truncated zip64 eocd")?;
    if entries_on_disk != total_entries {
        return Err("split Zip64 central directories are not supported".to_string());
    }
    let cd_size = rd_u64(tail, off + 40).ok_or("truncated zip64 eocd")?;
    let cd_off = rd_u64(tail, off + 48).ok_or("truncated zip64 eocd")?;
    Ok((total_entries, cd_size, cd_off))
}

fn parse_zip64_extra(
    extra: &[u8],
    need_uncomp: bool,
    need_comp: bool,
    need_lho: bool,
) -> Result<Zip64Extra, String> {
    let mut p = 0usize;
    while p + 4 <= extra.len() {
        let id = rd_u16(extra, p).ok_or("truncated extra")?;
        let len = rd_u16(extra, p + 2).ok_or("truncated extra")? as usize;
        let start = p + 4;
        let end = start.checked_add(len).ok_or("extra length overflow")?;
        if end > extra.len() {
            return Err("extra field past end".to_string());
        }
        if id == ZIP64_EXTRA_ID {
            let mut q = start;
            let mut z = Zip64Extra::default();
            if need_uncomp {
                z.uncomp_size = Some(rd_u64(extra, q).ok_or("truncated Zip64 extra")?);
                q += 8;
            }
            if need_comp {
                z.comp_size = Some(rd_u64(extra, q).ok_or("truncated Zip64 extra")?);
                q += 8;
            }
            if need_lho {
                z.local_header_off = Some(rd_u64(extra, q).ok_or("truncated Zip64 extra")?);
            }
            return Ok(z);
        }
        p = end;
    }
    if need_uncomp || need_comp || need_lho {
        return Err("Zip64 extra field missing".to_string());
    }
    Ok(Zip64Extra::default())
}

/// Read + decompress one entry FULLY into RAM. Only used for small entries
/// (manifest.json); large entries go through [`hash_entry_streaming`].
fn read_entry_bytes(file: &mut File, e: &CdEntry) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    hash_entry_inner(file, e, Some(&mut out))?;
    Ok(out)
}

/// Stream + decompress one entry, returning `(sha256_hex, uncompressed_len)`
/// without holding the decompressed bytes.
fn hash_entry_streaming(file: &mut File, e: &CdEntry) -> Result<(String, u64), String> {
    hash_entry_inner(file, e, None)
}

/// Shared core: seek to the entry's data, stream it through the decompressor,
/// hashing (always) and optionally collecting the bytes into `collect`. Enforces
/// the declared uncompressed size. Returns `(sha256_hex, uncompressed_len)`.
fn hash_entry_inner(
    file: &mut File,
    e: &CdEntry,
    collect: Option<&mut Vec<u8>>,
) -> Result<(String, u64), String> {
    let output_cap = if collect.is_some() {
        MAX_CONTAINER_ENTRY_BYTES
    } else {
        STREAMING_MAX_CONTAINER_ENTRY_BYTES
    };
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
    let lh_flags = rd_u16(&lfh, 6).unwrap();
    let lh_method = rd_u16(&lfh, 8).unwrap();
    let lh_crc32 = rd_u32(&lfh, 14).unwrap();
    let lh_comp_size = rd_u32(&lfh, 18).unwrap() as u64;
    let lh_uncomp_size = rd_u32(&lfh, 22).unwrap() as u64;
    if lh_flags != e.flags {
        return Err(format!(
            "entry {}: local/central flags mismatch (local 0x{lh_flags:04x}, central 0x{:04x})",
            e.name, e.flags
        ));
    }
    if lh_method != e.method {
        return Err(format!("entry {}: local/central method mismatch", e.name));
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
    }
    file.seek(SeekFrom::Start(data_start))
        .map_err(|err| format!("seek entry data {}: {err}", e.name))?;

    // A sink that owns the hasher and optionally collects decompressed output.
    let sink = HashSink {
        hasher: Sha256::new(),
        crc32: crc32fast::Hasher::new(),
        collect,
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
            // DEFLATE: feed compressed bytes into a write-side decoder whose output
            // goes to the hashing sink.
            let mut dec = DeflateDecoder::new(sink);
            stream_n(file, e.comp_size, |chunk| {
                dec.write_all(chunk).map_err(|e| e.to_string())
            })?;
            dec.finish()
                .map_err(|err| format!("entry {}: inflate failed: {err}", e.name))?
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
        return Err(format!(
            "entry {}: crc32 {crc32:08x} != central {:08x}",
            e.name, e.crc32
        ));
    }
    if e.flags & 0x0008 != 0 {
        read_and_validate_data_descriptor(file, e, crc32, e.comp_size, uncomp_len)?;
    }
    Ok((to_hex(&final_sink.hasher.finalize()), uncomp_len))
}

fn read_and_validate_data_descriptor(
    file: &mut File,
    e: &CdEntry,
    crc32: u32,
    comp_size: u64,
    uncomp_size: u64,
) -> Result<(), String> {
    let zip64_descriptor = comp_size > ZIP32_MAX_FIELD_VALUE || uncomp_size > ZIP32_MAX_FIELD_VALUE;
    let mut dd = vec![0u8; if zip64_descriptor { 24 } else { 16 }];
    file.read_exact(&mut dd)
        .map_err(|err| format!("entry {}: read data descriptor: {err}", e.name))?;
    if rd_u32(&dd, 0) != Some(0x0807_4b50) {
        return Err(format!(
            "entry {}: data descriptor signature missing",
            e.name
        ));
    }
    let dd_crc32 = rd_u32(&dd, 4).ok_or("truncated data descriptor")?;
    let (dd_comp_size, dd_uncomp_size) = if zip64_descriptor {
        (
            rd_u64(&dd, 8).ok_or("truncated data descriptor")?,
            rd_u64(&dd, 16).ok_or("truncated data descriptor")?,
        )
    } else {
        (
            rd_u32(&dd, 8).ok_or("truncated data descriptor")? as u64,
            rd_u32(&dd, 12).ok_or("truncated data descriptor")? as u64,
        )
    };
    if dd_crc32 != crc32 || dd_comp_size != comp_size || dd_uncomp_size != uncomp_size {
        return Err(format!(
            "entry {}: data descriptor mismatch (crc {dd_crc32:08x}/{crc32:08x}, compressed {dd_comp_size}/{comp_size}, uncompressed {dd_uncomp_size}/{uncomp_size})",
            e.name
        ));
    }
    Ok(())
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

/// A `Write` sink that SHA-256-hashes everything and optionally collects it.
struct HashSink<'a> {
    hasher: Sha256,
    crc32: crc32fast::Hasher,
    collect: Option<&'a mut Vec<u8>>,
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
        self.hasher.update(buf);
        self.crc32.update(buf);
        if let Some(c) = self.collect.as_deref_mut() {
            c.extend_from_slice(buf);
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

        // (2) existing in-RAM reader opens the streamed container and reads each
        // entry back individually (data-descriptor zip is backward-compatible).
        let container = Container::open(&container_path, &ExpectedIdentity::default())
            .expect("in-RAM Container opens streamed container");
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

    /// B3 flip gate (2026-07-11): the shipped production default (level 19 at
    /// window-27 + LDM) MUST be losslessly reversible, and the container it
    /// produces MUST decode under the production window cap (the weaker-PC
    /// decode bound). This is the zero-data-loss proof for the level flip:
    /// pack a realistic mixed payload (long repetitive runs to exercise LDM at
    /// window 27, interleaved with incompressible random blocks) through the
    /// exact production packing entry point at an EXPLICIT {19, 27} profile,
    /// then read every byte back and assert identity.
    #[test]
    fn level19_window27_roundtrip_is_byte_lossless() {
        // The shipped default is what production runs use when no --replay-zstd-level
        // is passed; pin it so a future silent change trips this test.
        assert_eq!(
            DEFAULT_ZSTD_COMPRESSION_LEVEL, 19,
            "production default zstd level must be the B3-shipped level 19"
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

        // Explicit production profile: level 19, window 27. validate() rejects an
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
            .expect("in-RAM reader accepts zip64 entry count");
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
            Ok(_) => panic!("in-RAM reader keeps 2 GiB cap"),
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
