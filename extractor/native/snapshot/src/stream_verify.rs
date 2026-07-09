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
use crate::zip::{validate_path, MAX_CONTAINER_ENTRY_BYTES};
use flate2::write::DeflateDecoder;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

const EOCD_SIG: u32 = 0x0605_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;
const VERIFY_CHUNK: usize = 4 * 1024 * 1024;

/// What a successful streaming verification observed.
#[derive(Debug, Clone, Default)]
pub struct StreamVerifyReport {
    /// Number of manifest-listed entries whose bytes were re-read and hash-matched.
    pub entries_verified: usize,
    /// Decompressed sha256 of the entry whose role matched `stream_role`, if found.
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
    let manifest: Value =
        serde_json::from_slice(&man_bytes).map_err(|e| format!("parse manifest.json: {e}"))?;
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

    // Coverage: every zip payload entry is in the manifest with the declared size,
    // and every manifest entry exists in the zip (no stowaways, no phantoms).
    let mut zip_names: HashMap<&str, &CdEntry> = HashMap::new();
    for e in &entries {
        if e.name == "manifest.json" {
            continue;
        }
        if e.uncomp_size > MAX_CONTAINER_ENTRY_BYTES || e.comp_size > MAX_CONTAINER_ENTRY_BYTES {
            return Err(format!(
                "entry {} exceeds per-entry reader cap (compressed {}, uncompressed {}, cap {})",
                e.name, e.comp_size, e.uncomp_size, MAX_CONTAINER_ENTRY_BYTES
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

    // --- hash EVERY payload entry from disk (bounded), tracking the stream role ---
    let _ = game_id; // reserved for future per-game path checks
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
        if rec.role.as_deref() == Some(stream_role) {
            report.stream_entry_sha256 = Some(sha);
            report.stream_entry_bytes = uncomp;
        }
    }
    Ok(report)
}

/// Locate + parse the End-Of-Central-Directory and read the central directory
/// (bounded: the CD is small relative to the payloads).
fn read_central_directory(file: &mut File, file_len: u64) -> Result<Vec<CdEntry>, String> {
    if file_len < 22 {
        return Err("container too small for an EOCD".to_string());
    }
    // The EOCD sits within the last 22 + 0xFFFF bytes (comment cap).
    let tail_len = (22u64 + 0xFFFF).min(file_len);
    let tail_start = file_len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    file.seek(SeekFrom::Start(tail_start))
        .map_err(|e| format!("seek eocd: {e}"))?;
    file.read_exact(&mut tail)
        .map_err(|e| format!("read eocd tail: {e}"))?;

    let eocd = find_eocd(&tail).ok_or_else(|| "no EOCD record found".to_string())?;
    let total = rd_u16(&tail, eocd + 10).ok_or("truncated eocd")? as usize;
    let cd_size = rd_u32(&tail, eocd + 12).ok_or("truncated eocd")? as u64;
    let cd_off = rd_u32(&tail, eocd + 16).ok_or("truncated eocd")? as u64;
    if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF || total == 0xFFFF {
        return Err("Zip64 not supported".to_string());
    }
    if cd_off.saturating_add(cd_size) > file_len {
        return Err("central directory out of range".to_string());
    }

    let mut cd = vec![0u8; cd_size as usize];
    file.seek(SeekFrom::Start(cd_off))
        .map_err(|e| format!("seek central dir: {e}"))?;
    file.read_exact(&mut cd)
        .map_err(|e| format!("read central dir: {e}"))?;

    let mut entries = Vec::with_capacity(total);
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
        let comp_size = rd_u32(&cd, p + 20).ok_or("truncated cdh")? as u64;
        let uncomp_size = rd_u32(&cd, p + 24).ok_or("truncated cdh")? as u64;
        let name_len = rd_u16(&cd, p + 28).ok_or("truncated cdh")? as usize;
        let extra_len = rd_u16(&cd, p + 30).ok_or("truncated cdh")? as usize;
        let comment_len = rd_u16(&cd, p + 32).ok_or("truncated cdh")? as usize;
        let lho = rd_u32(&cd, p + 42).ok_or("truncated cdh")? as u64;
        let name_start = p + 46;
        let name_end = name_start + name_len;
        if name_end > cd.len() {
            return Err("central dir name past end".to_string());
        }
        let name = std::str::from_utf8(&cd[name_start..name_end])
            .map_err(|_| "non-utf8 entry name".to_string())?
            .to_string();
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
    Ok(entries)
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
    if local_name != e.name.as_bytes() {
        return Err(format!("entry {}: local/central name mismatch", e.name));
    }
    if e.flags & 0x0008 == 0 {
        if lh_crc32 != e.crc32 || lh_comp_size != e.comp_size || lh_uncomp_size != e.uncomp_size {
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
    };

    let final_sink = match e.method {
        0 => {
            // STORE: comp_size == uncomp_size; stream raw bytes through the sink.
            let mut s = sink;
            stream_n(file, e.comp_size, |chunk| {
                s.write_all(chunk).map_err(|e| e.to_string())
            })?;
            s
        }
        8 => {
            // DEFLATE: feed compressed bytes into a write-side decoder whose output
            // goes to the hashing sink.
            let mut dec = DeflateDecoder::new(sink);
            stream_n(file, e.comp_size, |chunk| {
                dec.write_all(chunk).map_err(|e| e.to_string())
            })?;
            dec.finish()
                .map_err(|err| format!("entry {}: inflate failed: {err}", e.name))?
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
    let mut dd = [0u8; 16];
    file.read_exact(&mut dd)
        .map_err(|err| format!("entry {}: read data descriptor: {err}", e.name))?;
    if rd_u32(&dd, 0) != Some(0x0807_4b50) {
        return Err(format!(
            "entry {}: data descriptor signature missing",
            e.name
        ));
    }
    let dd_crc32 = rd_u32(&dd, 4).ok_or("truncated data descriptor")?;
    let dd_comp_size = rd_u32(&dd, 8).ok_or("truncated data descriptor")? as u64;
    let dd_uncomp_size = rd_u32(&dd, 12).ok_or("truncated data descriptor")? as u64;
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
}
impl Write for HashSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.hasher.update(buf);
        self.crc32.update(buf);
        if let Some(c) = self.collect.as_deref_mut() {
            c.extend_from_slice(buf);
        }
        self.len = self.len.saturating_add(buf.len() as u64);
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
        if rd_u32(tail, i) == Some(EOCD_SIG) {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{Container, ExpectedIdentity};
    use crate::pack::{
        pack_streaming_to_writer, BorrowedPackEntry, FileBackedPackEntry, PackIdentity,
        StreamingFileEntry,
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
            path: "payload/g/collector_raw/s.bin".into(),
            file_path: &stream_path,
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
