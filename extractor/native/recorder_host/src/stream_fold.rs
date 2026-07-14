//! Bounded-memory streaming fold over an on-disk compact record stream.
//!
//! This is the production replacement for `WorldFold::ingest_stream(&whole_buffer)`:
//! instead of holding the entire (multi-GB) compact stream in RAM, it reads the
//! stream file in fixed chunks, frames each 32-byte `H4CR` record against a small
//! carry buffer, and drives the SAME single production fold
//! ([`WorldFold::ingest_record`]). Because the fold is constructed file-backed
//! ([`WorldFold::new_file_backed`]), each per-game-hour snapshot is serialized and
//! spilled to disk the moment its `FRAME_END` closes and then released — so peak
//! memory is O(current world-fold state + one hour's snapshot), independent of the
//! stream's length.
//!
//! The framing loop is byte-for-byte the same contract the `fold_check`
//! fold-equivalence gate (G1) walks, so a stream folded here produces per-hour
//! snapshots hash-identical to `fold_check`'s reference fold. This module is used
//! by the recorder host during finalization and by the `stream_fold` offline
//! tool (which also re-folds old recordings and measures peak memory).

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use compact_contract::{
    parse_compact_record_header, RecordFramingError, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END,
};
use snapshot::sha256::{to_hex, Sha256};

use crate::world_fold::{FoldedSnapshotFile, WorldFold};

/// Read granularity for the streaming fold. Bounded and independent of stream size.
const CHUNK: usize = 8 * 1024 * 1024;

/// Accounting for one streaming fold pass (mirrors the `fold_check` totals so the
/// two can be cross-checked). Never grows with stream length.
#[derive(Debug, Default, Clone)]
pub struct FoldStreamStats {
    /// Total decoded H4CR bytes (independent of raw vs zstd on-disk storage).
    pub stream_bytes: u64,
    /// Bytes consumed as whole, well-framed records.
    pub bytes_folded: u64,
    /// Trailing bytes that did not form a complete record (mid-record EOF).
    pub truncated_tail_bytes: u64,
    pub records_total: u64,
    pub data_records: u64,
    pub frame_end_records: u64,
    pub perf_records: u64,
    pub other_records: u64,
    pub hours_emitted: u64,
    /// Set when framing stopped early on a malformed record (bad magic/version).
    pub stop_reason: Option<String>,
    /// True when the stream ends at a CLEAN CLOSED boundary per the shared
    /// BoundaryTracker (no residual bytes, no open hour, no hour-interleave /
    /// mismatched-FRAME_END / sequence-gap violation, >=1 closed hour). A raw
    /// crash tail (tolerated on read) makes this false; no seal path may pack
    /// such a source as a clean compressed container.
    pub ends_at_frame_end: bool,
    /// SHA-256 of the decoded stream bytes — parallel workers must agree on
    /// CONTENT, not just shape (round-3 finding 3); sha256 (round 4) so even
    /// deliberately-collided cheap-hash source versions cannot merge into a
    /// mixed fold. Hex, empty when never computed.
    pub decoded_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldedSnapshotReportRow {
    pub hour_index: i64,
    pub date: String,
    pub snapshot_hash: String,
    pub snapshot_bytes: usize,
}

/// Fold an on-disk compact stream file into per-game-hour snapshot files under
/// `snapshot_dir`, in bounded memory. Returns the spilled snapshot descriptors
/// (in emit order) plus the pass accounting.
pub fn fold_compact_stream_file(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
) -> io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)> {
    let reader = compact_stream::open(stream_path)?;
    let compressed = reader.compression() == compact_stream::StreamCompression::Zstd;
    let total_len = if compressed { 0 } else { reader.encoded_len() };
    let mut fold = WorldFold::new_file_backed(game_id, snapshot_dir)?;
    let stats = drive_fold_compressed(reader, total_len, &mut fold, compressed)?;
    let files = fold
        .finish_file_backed()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok((files, stats))
}

/// ONE shared logical-boundary state machine (container gate round 3: two
/// copies of this invariant are a permanent drift hazard — the fold driver
/// and the seal-time walker now both drive THIS). Legal production streams
/// are strictly hour-sequential: every record of hour h, then FRAME_END(h).
/// Anything else is corruption and must never be silently normalized:
/// - DOMAIN_DATA opens its hour; a DATA for a DIFFERENT hour while one is
///   open is a VIOLATION (round-3 finding 1: a single-slot tracker that just
///   overwrites forgets the earlier open hour).
/// - Only a FRAME_END whose hour MATCHES the open hour closes it; a
///   mismatched FRAME_END is a VIOLATION.
/// - Record sequence must be CONSECUTIVE (+1) — an inflated-but-under-cap
///   payload_len that swallows whole records leaves a sequence gap (round-3
///   finding 2); framing alone cannot see the swallowed bytes, the counter
///   can.
/// - PERF/OTHER records neither open nor close hours (but do consume
///   sequence numbers).
#[derive(Default)]
struct BoundaryTracker {
    open_hour: Option<i32>,
    frame_end_seen: bool,
    prev_sequence: Option<u64>,
    violation: Option<String>,
}

impl BoundaryTracker {
    fn observe(&mut self, header: &compact_contract::CompactRecordHeader, at_byte: u64) {
        if self.violation.is_some() {
            return; // first violation is the diagnostic; state is already dirty
        }
        if let Some(prev) = self.prev_sequence {
            if header.sequence != prev.wrapping_add(1) {
                self.violation = Some(format!(
                    "sequence discontinuity at byte {at_byte}: {prev} -> {} \
                     (a record was swallowed or reordered)",
                    header.sequence
                ));
                return;
            }
        }
        self.prev_sequence = Some(header.sequence);
        // Per-type payload bounds (round-4 finding 1: a TERMINAL record with a
        // forged under-cap payload_len can swallow everything to EOF, leaving
        // no later header to expose the sequence gap). DOMAIN_DATA is the only
        // type that legitimately needs large payloads, and a terminal
        // swallowing DATA always leaves its own hour OPEN at EOF (already a
        // violation) — so tight bounds on every other type close the class:
        // FRAME_END is EXACTLY 32 bytes (collector layout; corpus-verified on
        // 126k records: min=max=32); PERF diagnostics observed <=2.5KB, bound
        // 1 MiB; unknown/session types bound 16 MiB.
        let payload_bound_violation = match header.record_type {
            COMPACT_RECORD_TYPE_FRAME_END => header.payload_len != 32,
            COMPACT_RECORD_TYPE_DOMAIN_DATA => false, // contract cap (256 MiB) applies
            COMPACT_RECORD_TYPE_DOMAIN_PERF => header.payload_len > (1 << 20),
            _ => header.payload_len > (16 << 20),
        };
        if payload_bound_violation {
            self.violation = Some(format!(
                "record type {} at byte {at_byte} carries an implausible payload \
                 length {} (per-type bound violated)",
                header.record_type, header.payload_len
            ));
            return;
        }
        match header.record_type {
            COMPACT_RECORD_TYPE_DOMAIN_DATA => match self.open_hour {
                Some(open) if open != header.game_hour_raw => {
                    self.violation = Some(format!(
                        "hour interleaving at byte {at_byte}: DOMAIN_DATA for hour {} \
                         while hour {open} is still open",
                        header.game_hour_raw
                    ));
                }
                _ => self.open_hour = Some(header.game_hour_raw),
            },
            COMPACT_RECORD_TYPE_FRAME_END => {
                self.frame_end_seen = true;
                match self.open_hour {
                    Some(open) if open == header.game_hour_raw => self.open_hour = None,
                    Some(open) => {
                        self.violation = Some(format!(
                            "mismatched FRAME_END at byte {at_byte}: closes hour {} \
                             while hour {open} is open",
                            header.game_hour_raw
                        ));
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Ok(()) when the stream ended at a CLEAN closed boundary; Err(reason).
    fn eof_check(&self, residual_carry: usize) -> Result<(), String> {
        if let Some(v) = &self.violation {
            return Err(v.clone());
        }
        if residual_carry != 0 {
            return Err(format!(
                "stream ends mid-record ({residual_carry} residual bytes; partial \
                 header or incomplete payload)"
            ));
        }
        if let Some(open) = self.open_hour {
            return Err(format!("game hour {open} left open at EOF"));
        }
        if !self.frame_end_seen {
            return Err("stream closes no game hour (no FRAME_END records)".to_string());
        }
        Ok(())
    }
}

/// Framing-only logical-boundary check of an on-disk compact stream — the
/// SEAL-TIME gate for containers packed WITHOUT a host fold (skip-fold /
/// fast-turnaround), so the production writer can never publish a container
/// that is guaranteed unreadable under the sealed-source strictness
/// (container re-gate round 2, finding 3). Runs the same hour-aware state
/// machine as `drive_fold_compressed` but folds nothing: hard-errors on any
/// framing error, residual tail bytes, an hour left open at EOF, or a stream
/// that closes no hour.
pub fn verify_stream_logical_boundary(stream_path: &Path) -> io::Result<()> {
    let mut reader = compact_stream::open(stream_path)?;
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;
    let mut tracker = BoundaryTracker::default();
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..n]);
        let mut off = 0usize;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            let header = parse_compact_record_header(&carry[off..]).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("seal-time boundary check: {e} at byte {}", processed + off as u64),
                )
            })?;
            let record_len = header.record_len();
            if record_len > remaining {
                break;
            }
            tracker.observe(&header, processed + off as u64);
            off += record_len;
        }
        processed += off as u64;
        carry.drain(0..off);
    }
    tracker.eof_check(carry.len()).map_err(|reason| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("seal-time boundary check: {reason}"),
        )
    })
}

/// Default worker count for the parallel post-capture fold. The fold is OFFLINE
/// batch work (post-capture finalization / re-fold tooling), where the
/// weaker-PC law's "don't lean on cores" constraint does not apply — that law
/// governs CAPTURE-time per-frame work.
///
/// cores/2, capped at 8: measured on the deep-war 512 MiB prefix (8-core rig),
/// 4 workers (11.15 MB/s) beats 8 workers (10.2 MB/s) — the fold is
/// memory-bandwidth/allocation-bound, and each fold worker also owns an emit
/// worker thread, so cores/2 fold workers already saturates the machine. The
/// cap also bounds peak memory (each worker holds a full fold-state copy).
pub fn default_fold_workers() -> usize {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    (cores / 2).clamp(1, 8)
}

/// Parallel bounded-memory fold of an on-disk compact stream: `workers`
/// independent streaming folds each walk the WHOLE stream (fold state is
/// carried across hours, so every worker must apply every record — record
/// application is ~10% of fold cost), but worker `k` BUILDS + spills only the
/// hours whose emission ordinal is `≡ k (mod workers)`. Because
/// `WorldFold::build_snapshot` is `&self`-pure, skipping it on non-owned hours
/// cannot perturb any worker's state, so the union of the workers' outputs is
/// byte-identical to the sequential fold's output — same files, same names,
/// same canonical bytes, same hashes (G1-preserving by construction).
///
/// Returns the merged snapshot descriptors sorted by `hour_index` (identical
/// ordering to the sequential fold) and the pass accounting from worker 0
/// (every worker walks identical bytes, so their `FoldStreamStats` must agree;
/// divergence fails loudly). `workers <= 1` delegates to the sequential fold.
pub fn fold_compact_stream_file_parallel(
    stream_path: &Path,
    game_id: &str,
    snapshot_dir: &Path,
    workers: usize,
) -> io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)> {
    if workers <= 1 {
        return fold_compact_stream_file(stream_path, game_id, snapshot_dir);
    }
    // A worker that ends up owning zero hours is only wasted CPU, never a
    // correctness problem, so no up-front knowledge of the hour count is needed.
    let results: Vec<io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)>> =
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..workers)
                .map(|k| {
                    scope.spawn(
                        move || -> io::Result<(Vec<FoldedSnapshotFile>, FoldStreamStats)> {
                            let reader = compact_stream::open(stream_path)?;
                            let compressed =
                                reader.compression() == compact_stream::StreamCompression::Zstd;
                            let total_len = if compressed { 0 } else { reader.encoded_len() };
                            let mut fold = WorldFold::new_file_backed_strided(
                                game_id,
                                snapshot_dir,
                                workers as u64,
                                k as u64,
                            )?;
                            let stats =
                                drive_fold_compressed(reader, total_len, &mut fold, compressed)?;
                            let files = fold
                                .finish_file_backed()
                                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                            Ok((files, stats))
                        },
                    )
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| {
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            "parallel fold worker panicked",
                        ))
                    })
                })
                .collect()
        });

    let mut merged: Vec<FoldedSnapshotFile> = Vec::new();
    let mut stats: Option<FoldStreamStats> = None;
    for result in results {
        let (files, worker_stats) = result?;
        // Every worker framed identical bytes; any accounting divergence means a
        // non-deterministic walk and MUST fail loudly rather than merge.
        if let Some(reference) = &stats {
            let agree = reference.stream_bytes == worker_stats.stream_bytes
                && reference.bytes_folded == worker_stats.bytes_folded
                && reference.records_total == worker_stats.records_total
                && reference.hours_emitted == worker_stats.hours_emitted
                && reference.stop_reason == worker_stats.stop_reason
                // Boundary facts join the agreement predicate (re-gate round 2
                // finding 4): a worker seeing a different file view must fail
                // the merge, not silently supply the stored stats.
                && reference.truncated_tail_bytes == worker_stats.truncated_tail_bytes
                && reference.ends_at_frame_end == worker_stats.ends_at_frame_end
                // Content digest: same-shape-different-payload source views
                // (e.g. file replaced between worker opens) must fail the
                // merge, not silently mix snapshots from two stream versions.
                && reference.decoded_sha256 == worker_stats.decoded_sha256;
            if !agree {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "parallel fold workers disagree on stream accounting: \
                         {reference:?} vs {worker_stats:?}"
                    ),
                ));
            }
        } else {
            stats = Some(worker_stats);
        }
        merged.extend(files);
    }
    merged.sort_by_key(|f| f.hour_index);
    let stats = stats.expect("workers >= 2 always yields stats");
    if merged.len() as u64 != stats.hours_emitted {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!(
                "parallel fold merged {} snapshot files but the stream closed {} hours",
                merged.len(),
                stats.hours_emitted
            ),
        ));
    }
    Ok((merged, stats))
}

/// Hash spilled snapshot files for the offline report. When `discard_snapshots`
/// is true, each file is removed only after its hash and byte count are captured.
pub fn hash_folded_snapshot_files(
    files: &[FoldedSnapshotFile],
    discard_snapshots: bool,
) -> io::Result<Vec<FoldedSnapshotReportRow>> {
    let mut rows = Vec::with_capacity(files.len());
    for f in files {
        // Prefer the hash the emit worker already computed over the exact bytes it
        // wrote — avoids re-reading (and re-hashing) every multi-MB snapshot from
        // disk. Fall back to a fresh read only for snapshots without a stored hash.
        let snapshot_hash = match &f.sha256 {
            Some(hash) => hash.clone(),
            None => sha256_file(&f.path)
                .map_err(|e| io::Error::new(e.kind(), format!("hash {}: {e}", f.path.display())))?,
        };
        rows.push(FoldedSnapshotReportRow {
            hour_index: f.hour_index,
            date: f.date.clone(),
            snapshot_hash,
            snapshot_bytes: f.byte_len,
        });
        if discard_snapshots {
            std::fs::remove_file(&f.path).map_err(|e| {
                io::Error::new(e.kind(), format!("remove {}: {e}", f.path.display()))
            })?;
        }
    }
    Ok(rows)
}

pub fn snapshot_chain_hash(rows: &[FoldedSnapshotReportRow]) -> String {
    let mut chain = Sha256::new();
    for row in rows {
        chain.update(row.hour_index.to_le_bytes().as_slice());
        chain.update(row.snapshot_hash.as_bytes());
        chain.update(b"\n");
    }
    to_hex(&chain.finalize())
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// Validate one DOMAIN_DATA payload through the canonical per-domain decoder's
/// structural-only entrypoint. The record header's declared payload length is
/// not trusted: exact consumption by the domain grammar is the independent
/// delimiter. No payload bytes or status bits are modified, and missing fold
/// baselines are deliberately irrelevant to this boundary verdict.
fn validate_domain_data_payload(
    domain_id: u16,
    payload: &[u8],
    record_flags: u32,
) -> Result<(), String> {
    WorldFold::validate_domain_data_payload_exact(domain_id, payload, record_flags)
}

/// Drive the production fold over a record stream with a bounded carry buffer.
/// Every `DOMAIN_DATA`/`FRAME_END` record is handed to [`WorldFold::ingest_record`]
/// exactly as `WorldFold::ingest_stream` would; `DOMAIN_PERF`/probe records carry
/// no folded state and are counted but ignored. A file-backed fold spills each
/// closed hour to disk inside `ingest_record`, so nothing accumulates here.
pub fn drive_fold<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
) -> io::Result<FoldStreamStats> {
    drive_fold_compressed(reader.by_ref(), total_len, fold, false)
}

/// `compressed` = the reader is a checksummed zstd stream. On such a source an
/// H4CR framing stop before physical EOF drains the reader to EOF (forcing the
/// zstd footer + content-checksum check) and then hard-errors — corruption of
/// the decoded content fails the checksum, and an intact-storage framing stop
/// is itself garbage. Raw sources keep the tolerant torn-tail behavior the
/// deep-war corpus relies on.
pub fn drive_fold_compressed<R: Read>(
    mut reader: R,
    total_len: u64,
    fold: &mut WorldFold,
    compressed: bool,
) -> io::Result<FoldStreamStats> {
    let mut stats = FoldStreamStats {
        stream_bytes: total_len,
        ..FoldStreamStats::default()
    };
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;
    let mut decoded_read: u64 = 0;
    // Shared hour-aware closure state machine (see BoundaryTracker: hour
    // interleaving, mismatched FRAME_END, and sequence discontinuities are
    // sticky violations; only a clean tracker at EOF is a closed boundary).
    let mut tracker = BoundaryTracker::default();
    let mut decoded_hasher = Sha256::new();

    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        decoded_read = decoded_read.saturating_add(n as u64);
        decoded_hasher.update(&chunk[..n]);
        carry.extend_from_slice(&chunk[..n]);

        let mut off = 0usize;
        let mut hard_stop = false;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break;
            }
            // The single H4CR framing primitive (compact_contract); malformed
            // headers stop the walk with the same diagnostic strings as before.
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(h) => h,
                Err(e) => {
                    let at = processed + off as u64;
                    stats.stop_reason = Some(match e {
                        RecordFramingError::BadMagic => {
                            format!("invalid record magic at byte {at}")
                        }
                        RecordFramingError::UnsupportedVersion(v) => {
                            format!("unsupported record version {v} at byte {at}")
                        }
                        RecordFramingError::UnsupportedHeaderLen(l) => {
                            format!("unsupported header length {l} at byte {at}")
                        }
                        RecordFramingError::PayloadTooLarge(l) => {
                            format!("payload length {l} exceeds the record bound at byte {at}")
                        }
                    });
                    hard_stop = true;
                    break;
                }
            };
            let record_len = header.record_len();
            if record_len > remaining {
                break; // payload not fully buffered yet
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];

            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                validate_domain_data_payload(header.domain_id, payload, header.record_flags)
                    .map_err(|reason| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "invalid DOMAIN_DATA payload for domain {} at byte {}: {reason}",
                                header.domain_id,
                                processed + off as u64
                            ),
                        )
                    })?;
            }

            stats.records_total += 1;
            match header.record_type {
                COMPACT_RECORD_TYPE_DOMAIN_DATA => stats.data_records += 1,
                COMPACT_RECORD_TYPE_FRAME_END => stats.frame_end_records += 1,
                COMPACT_RECORD_TYPE_DOMAIN_PERF => stats.perf_records += 1,
                _ => stats.other_records += 1,
            }
            tracker.observe(&header, processed + off as u64);

            fold.ingest_record(
                header.game_hour_raw,
                header.domain_id,
                header.record_type,
                payload,
                header.record_flags,
            );
            off += record_len;
        }

        processed += off as u64;
        carry.drain(0..off);
        if hard_stop {
            if compressed {
                // Force zstd footer + content-checksum validation before
                // trusting this stop; a corrupt frame errors here rather than
                // reading as a clean short fold.
                let mut sink = [0u8; 64 * 1024];
                loop {
                    match reader.read(&mut sink)? {
                        0 => break,
                        _ => continue,
                    }
                }
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "H4CR framing stopped before physical EOF on a compressed \
                         stream at byte {processed} (storage checksum valid, so the \
                         decoded content itself is corrupt)"
                    ),
                ));
            }
            break 'read;
        }
    }

    // A file-backed fold spills as it goes; `emitted_hours` is the authoritative
    // count of closed game-hours in either mode.
    if total_len == 0 {
        stats.stream_bytes = decoded_read;
    }
    stats.bytes_folded = processed;
    stats.truncated_tail_bytes = stats.stream_bytes.saturating_sub(processed);
    stats.hours_emitted = fold.emitted_hours_count() as u64;
    stats.ends_at_frame_end = tracker.eof_check(carry.len()).is_ok();
    stats.decoded_sha256 = to_hex(&decoded_hasher.finalize());
    if compressed {
        // A SEALED (checksummed zstd) container must end at a valid LOGICAL
        // boundary, not merely a valid storage boundary: no residual carry,
        // no open hour, no boundary violation (hour interleave / mismatched
        // FRAME_END / sequence gap), at least one closed hour. RAW sources
        // keep the tolerant torn-tail behavior byte-identically.
        if let Err(reason) = tracker.eof_check(carry.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("compressed stream fails the closed-boundary law: {reason}"),
            ));
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use compact_contract::{
        COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
        COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_PARTIAL_DATA,
        COMPACT_STATUS_SOURCE_UNAVAILABLE,
    };
    use snapshot::sha256::{to_hex, Sha256};
    use std::io::Cursor;
    use std::path::PathBuf;

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "hoi4_stream_fold_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn raw_for(hour_index: i64) -> i32 {
        DATE_EPOCH_CONST + hour_index as i32
    }

    fn push_record(
        out: &mut Vec<u8>,
        seq: u64,
        game_hour_raw: i32,
        domain: u16,
        ty: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain.to_le_bytes());
        out.extend_from_slice(&ty.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    /// domain-18 country_identity payload (same wire layout the collector emits).
    fn country_identity_payload(rows: &[(u32, &str)]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&0u64.to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        p.extend_from_slice(&0u32.to_le_bytes());
        for (id, tag) in rows {
            p.extend_from_slice(&id.to_le_bytes());
            p.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            p.extend_from_slice(tag.as_bytes());
            p.push(1);
        }
        p
    }

    fn append_complete_domain_record(payload: &mut Vec<u8>, hour: i32) {
        let mut swallowed = Vec::new();
        push_record(
            &mut swallowed,
            99,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );
        payload.extend_from_slice(&swallowed);
    }

    fn expect_domain_boundary_rejection(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("gid");
        let err = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect_err("a DATA payload that swallows a complete record must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "domain {domain}");
    }

    fn expect_malformed_domain_invalid_data(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        let mut fold = WorldFold::new("malformed_boundary");
        let result = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold);
        let err = result.expect_err("a malformed DATA payload must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData, "domain {domain}");
        assert!(
            err.to_string().contains(&format!(
                "invalid DOMAIN_DATA payload for domain {domain} at byte 0"
            )),
            "unexpected boundary attribution: {err}"
        );
    }

    fn expect_domain_boundary_acceptance(domain: u16, payload: &[u8]) {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            domain,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("production_boundary_acceptance");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .unwrap_or_else(|err| panic!("valid production d{domain} payload was rejected: {err}"));
        assert_eq!(stats.data_records, 1, "domain {domain}");
        assert_eq!(stats.bytes_folded, stream.len() as u64, "domain {domain}");
        assert!(stats.stop_reason.is_none(), "domain {domain}");
    }

    const HOUR_A: i64 = 16_959_360;
    const HOUR_B: i64 = 16_959_361;
    const HOUR_C: i64 = 16_959_362;

    #[test]
    fn d12_truncated_country_row_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 28];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_invalid_data(12, &payload);
    }

    #[test]
    fn d14_truncated_popularity_row_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 40];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_invalid_data(14, &payload);
    }

    #[test]
    fn d15_truncated_advisor_detail_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 24];
        payload[20..24].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_invalid_data(15, &payload);
    }

    #[test]
    fn d12_truncated_completed_tag_read_u32_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 31];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        expect_malformed_domain_invalid_data(12, &payload);
    }

    #[test]
    fn d14_truncated_popularity_value_returns_invalid_data_not_panic() {
        let mut payload = vec![0u8; 40];
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        expect_malformed_domain_invalid_data(14, &payload);
    }

    #[test]
    fn d14_full_popularity_row_remains_boundary_valid() {
        let mut payload = vec![0u8; 40];
        payload[8..12].copy_from_slice(&1u32.to_le_bytes());
        payload[24..28].copy_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&[0u8; 24]);
        for _ in 0..4 {
            payload.extend_from_slice(&0u16.to_le_bytes());
        }
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.push(b'd');
        payload.extend_from_slice(&50_000i32.to_le_bytes());
        expect_domain_boundary_acceptance(14, &payload);
    }

    #[test]
    fn d12_zero_row_payload_cannot_swallow_complete_record() {
        let mut payload = vec![0u8; 28];
        append_complete_domain_record(&mut payload, raw_for(HOUR_A));
        expect_domain_boundary_rejection(12, &payload);
    }

    #[test]
    fn d14_through_d17_trailing_record_residue_is_rejected() {
        for (domain, header_len) in [(14u16, 40usize), (15, 24), (16, 24), (17, 24)] {
            let mut payload = vec![0u8; header_len];
            append_complete_domain_record(&mut payload, raw_for(HOUR_A));
            expect_domain_boundary_rejection(domain, &payload);
        }
    }

    #[test]
    fn d51_delta_and_failure_status_cannot_skip_boundary_validation() {
        for status in [COMPACT_STATUS_DELTA_DATA, COMPACT_STATUS_SOURCE_UNAVAILABLE] {
            let mut payload = vec![0u8; 64];
            payload[20..24].copy_from_slice(&status.to_le_bytes());
            payload[48..52].copy_from_slice(&100_000u32.to_le_bytes());
            payload[52..56].copy_from_slice(&2u32.to_le_bytes());
            append_complete_domain_record(&mut payload, raw_for(HOUR_A));
            expect_domain_boundary_rejection(51, &payload);
        }
    }

    #[test]
    fn d69_failure_status_at_byte_12_cannot_skip_boundary_validation() {
        let mut payload = vec![0u8; 16];
        payload[12..16].copy_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        append_complete_domain_record(&mut payload, raw_for(HOUR_A));
        expect_domain_boundary_rejection(69, &payload);

        // Refutation attempt: make the swallowed neighbor exactly 64 bytes so
        // d69's member-blob remainder is u32-aligned. Zero group windows still
        // cannot cover those orphan u32s, so exact coverage must reject it.
        let mut aligned_payload = vec![0u8; 16];
        aligned_payload[12..16].copy_from_slice(&COMPACT_STATUS_SOURCE_UNAVAILABLE.to_le_bytes());
        push_record(
            &mut aligned_payload,
            99,
            raw_for(HOUR_A),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        expect_domain_boundary_rejection(69, &aligned_payload);
    }

    #[test]
    fn d51_and_d69_production_failure_payloads_pass_stream_boundary() {
        use scripted_state_flags_variables_live_lane::scripted_state_domain51_gate::compact as d51c;

        let d51 = d51c::encode_payload(
            &d51c::ScriptedStateHeader {
                elapsed_wall_ms: 1,
                status_flags: d51c::STATUS_SOURCE_UNAVAILABLE,
                changed_rows: 0,
                observed_scopes: 0,
                measured_scopes: 0,
                rejected_scopes: 0,
            },
            &[],
        );
        expect_domain_boundary_acceptance(51, &d51);

        let d69 = compact_contract::tech_sharing_groups::encode_payload(
            &[],
            1,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
        );
        expect_domain_boundary_acceptance(69, &d69);
    }

    #[test]
    fn d53_plain_delta_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_DELTA_DATA,
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            ..Default::default()
        });
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_plain_partial_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
            ..Default::default()
        });
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_facs_v1_absent_production_payload_passes_stream_boundary() {
        use diplomacy_intel_operatives_live::compact as d53c;

        let payload = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            factions: Vec::new(),
            ..Default::default()
        });
        assert!(
            !payload.windows(4).any(|window| window == b"FACS"),
            "the production encoder must omit FACS v1 for an absent faction set"
        );
        expect_domain_boundary_acceptance(53, &payload);
    }

    #[test]
    fn d53_delta_partial_passes_boundary_validation_and_normal_fold() {
        use diplomacy_intel_operatives_live::compact as d53c;

        // This is the real recorder's empty-first-frame sequence: a tolerated
        // partial full frame seeds an empty baseline, then the next partial read
        // emits a DELTA|PARTIAL diff against that baseline.
        let baseline = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_PARTIAL_DATA,
            ..Default::default()
        });
        let delta = d53c::encode_payload(&d53c::DiplomacyIntelSnapshot {
            status_flags: COMPACT_STATUS_DELTA_DATA | COMPACT_STATUS_PARTIAL_DATA,
            opinions: vec![d53c::OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 2,
                opinion: 5,
            }],
            ..Default::default()
        });

        validate_domain_data_payload(53, &delta, 0)
            .expect("a structurally valid d53 DELTA|PARTIAL must pass boundary validation");

        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            raw_for(HOUR_A),
            53,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &baseline,
        );
        push_record(
            &mut stream,
            2,
            raw_for(HOUR_A),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        push_record(
            &mut stream,
            3,
            raw_for(HOUR_B),
            53,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &delta,
        );
        push_record(
            &mut stream,
            4,
            raw_for(HOUR_B),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("the normal pipeline must fold a valid d53 DELTA|PARTIAL after its baseline");
        assert_eq!(stats.hours_emitted, 2);
    }

    fn three_hour_stream() -> Vec<u8> {
        let mut s = Vec::new();
        let mut seq = 1u64;
        for (h, rows) in [
            (HOUR_A, vec![(47u32, "GER")]),
            (HOUR_B, vec![(47, "GER")]),
            (HOUR_C, vec![(47, "GER"), (48, "FRA")]),
        ] {
            push_record(
                &mut s,
                seq,
                raw_for(h),
                18,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &country_identity_payload(&rows),
            );
            seq += 1;
            push_record(
                &mut s,
                seq,
                raw_for(h),
                0,
                COMPACT_RECORD_TYPE_FRAME_END,
                &[0u8; 32], // production FRAME_END payload is exactly 32 bytes
            );
            seq += 1;
        }
        s
    }

    /// Reproduce the audited boundary construction exactly: after the forged
    /// DATA length consumes a complete adjacent record, the only visible H4CR
    /// headers remain PERF/DATA/FRAME_END with consecutive sequence numbers.
    /// Coverage counts and the stream hash are intentionally derived from the
    /// already-corrupt bytes, so neither can reveal the swallowed record.
    #[test]
    fn data_payload_cannot_swallow_complete_adjacent_record() {
        let hour = raw_for(HOUR_A);
        let mut swallowed = Vec::new();
        push_record(
            &mut swallowed,
            3,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(48, "FRA")]),
        );

        let mut forged_payload = country_identity_payload(&[(47, "GER")]);
        forged_payload.extend_from_slice(&swallowed);

        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0u8; 40],
        );
        push_record(
            &mut stream,
            2,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &forged_payload,
        );
        push_record(
            &mut stream,
            3,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );

        let regenerated_manifest_hash = Sha256::digest(&stream);
        assert_eq!(Sha256::digest(&stream), regenerated_manifest_hash);

        let mut fold = WorldFold::new("gid");
        let err = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect_err("a DATA payload that swallows a complete H4CR record must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string().contains("invalid DOMAIN_DATA payload"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn well_formed_domain_data_payload_passes_boundary_validation() {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &[0u8; 40],
        );
        push_record(
            &mut stream,
            2,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        push_record(
            &mut stream,
            3,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect("well-formed DOMAIN_DATA must remain accepted");
        assert_eq!(stats.stream_bytes, stream.len() as u64);
        assert_eq!(stats.bytes_folded, stream.len() as u64);
        assert_eq!(stats.records_total, 3);
        assert_eq!(stats.perf_records, 1);
        assert_eq!(stats.data_records, 1);
        assert_eq!(stats.frame_end_records, 1);
        assert_eq!(stats.hours_emitted, 1);
        assert_eq!(stats.truncated_tail_bytes, 0);
        assert!(stats.stop_reason.is_none());
    }

    fn one_data_hour(payload: &[u8]) -> Vec<u8> {
        let hour = raw_for(HOUR_A);
        let mut stream = Vec::new();
        push_record(
            &mut stream,
            1,
            hour,
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            payload,
        );
        push_record(
            &mut stream,
            2,
            hour,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        stream
    }

    #[test]
    fn domain_data_payload_residue_fails_closed() {
        let mut payload = country_identity_payload(&[(47, "GER")]);
        payload.push(0xA5);
        let stream = one_data_hour(&payload);
        let mut fold = WorldFold::new("gid");
        let err = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect_err("unconsumed DATA residue must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn domain_data_payload_declared_row_overrun_fails_closed() {
        let mut payload = country_identity_payload(&[(47, "GER")]);
        payload[8..12].copy_from_slice(&2u32.to_le_bytes());
        let stream = one_data_hour(&payload);
        let mut fold = WorldFold::new("gid");
        let err = drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold)
            .expect_err("DATA internal row overrun must fail closed");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// The streaming fold spills one snapshot file per hour and produces bytes
    /// byte-identical to the in-memory batch fold (`ingest_stream` + `finish`).
    /// This is the unit-level G1 equivalence: streaming path == batch path.
    #[test]
    fn streaming_fold_matches_batch_fold() {
        let dir = TmpDir::new("equiv");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();

        let (files, stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(stats.hours_emitted, 3);
        assert_eq!(stats.truncated_tail_bytes, 0);
        assert!(stats.stop_reason.is_none());

        // Batch fold the same bytes in memory.
        let mut batch = WorldFold::new("gid");
        batch.ingest_stream(&stream);
        let batch_out = batch.finish();
        assert_eq!(batch_out.len(), files.len());
        for (f, (hour, _date, bytes)) in files.iter().zip(batch_out.iter()) {
            assert_eq!(f.hour_index, *hour);
            let spilled = std::fs::read(&f.path).unwrap();
            assert_eq!(
                to_hex(&Sha256::digest(&spilled)),
                to_hex(&Sha256::digest(bytes)),
                "streaming hour {} snapshot != batch",
                hour
            );
        }
    }

    /// Net-gate finding 1: a sealed (compressed) stream whose decoded bytes
    /// end mid-record must HARD-ERROR, never return Ok-with-stats. Raw stays
    /// tolerant (separately asserted by the incomplete-tail test).
    #[test]
    fn compressed_stream_ending_mid_record_hard_errors() {
        let mut stream = three_hour_stream();
        // Append 16 bytes of a would-be next header (valid magic, then cut).
        stream.extend_from_slice(COMPACT_RECORD_MAGIC);
        stream.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        stream.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        stream.extend_from_slice(&7u64.to_le_bytes());
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true)
            .unwrap_err();
        assert!(err.to_string().contains("ends mid-record"), "{err}");

        // Same bytes, RAW source: tolerated with the residue accounted.
        let mut fold = WorldFold::new("gid");
        let stats = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false)
            .unwrap();
        assert_eq!(stats.truncated_tail_bytes, 16);
        assert!(!stats.ends_at_frame_end);
    }

    /// Net-gate finding 2: a sealed stream whose last record is NOT FRAME_END
    /// ends inside an open game hour (a crash tail) and must HARD-ERROR; the
    /// deferred repack independently refuses to seal such a source.
    #[test]
    fn compressed_stream_ending_in_open_hour_hard_errors() {
        let mut stream = three_hour_stream();
        push_record(
            &mut stream,
            7,
            raw_for(HOUR_C + 1),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true)
            .unwrap_err();
        assert!(err.to_string().contains("left open"), "{err}");

        // RAW: tolerated; the boundary fact is exposed for the repack guard.
        let mut fold = WorldFold::new("gid");
        let stats = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false)
            .unwrap();
        assert!(!stats.ends_at_frame_end);
        // A clean stream reports the closed boundary.
        let mut fold = WorldFold::new("gid");
        let clean = drive_fold_compressed(
            std::io::Cursor::new(three_hour_stream()),
            0,
            &mut fold,
            false,
        )
        .unwrap();
        assert!(clean.ends_at_frame_end);
    }

    /// Re-gate round 2, finding 1: closure is HOUR-AWARE — `DATA(N+1),
    /// FRAME_END(N), EOF` has a trailing FRAME_END but hour N+1 was never
    /// closed; it must hard-error on sealed sources and report an unclosed
    /// boundary on raw ones.
    #[test]
    fn trailing_frame_end_for_the_wrong_hour_does_not_close_the_stream() {
        let mut stream = three_hour_stream();
        push_record(
            &mut stream,
            7,
            raw_for(HOUR_C + 1),
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &country_identity_payload(&[(47, "GER")]),
        );
        // FRAME_END for the WRONG (already closed) hour.
        push_record(
            &mut stream,
            8,
            raw_for(HOUR_C),
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[0u8; 32],
        );
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true)
            .unwrap_err();
        assert!(err.to_string().contains("mismatched FRAME_END"), "{err}");

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false)
            .unwrap();
        assert!(!stats.ends_at_frame_end);

        // The seal-time walker enforces the same law for skip-fold packs.
        let dir = TmpDir::new("boundary");
        let p = dir.0.join("torn.bin");
        std::fs::write(&p, &stream).unwrap();
        let err = verify_stream_logical_boundary(&p).unwrap_err();
        assert!(err.to_string().contains("mismatched FRAME_END"), "{err}");
        let clean = dir.0.join("clean.bin");
        std::fs::write(&clean, three_hour_stream()).unwrap();
        verify_stream_logical_boundary(&clean).unwrap();
    }

    /// Net-gate finding 3: a forged/corrupt payload_len beyond the record
    /// bound is a framing error — hard on compressed, loud stop_reason on raw
    /// — instead of a multi-hundred-MB carry that can swallow later records.
    #[test]
    fn oversized_payload_len_is_a_framing_stop() {
        let mut stream = three_hour_stream();
        stream.extend_from_slice(COMPACT_RECORD_MAGIC);
        stream.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
        stream.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
        stream.extend_from_slice(&7u64.to_le_bytes());
        stream.extend_from_slice(&raw_for(HOUR_C).to_le_bytes());
        stream.extend_from_slice(&18u16.to_le_bytes());
        stream.extend_from_slice(&COMPACT_RECORD_TYPE_DOMAIN_DATA.to_le_bytes());
        stream.extend_from_slice(&0x2000_0000u32.to_le_bytes()); // 512 MiB > bound
        stream.extend_from_slice(&0u32.to_le_bytes());

        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, true)
            .unwrap_err();
        assert!(err.to_string().contains("framing stopped"), "{err}");

        let mut fold = WorldFold::new("gid");
        let stats = drive_fold_compressed(std::io::Cursor::new(&stream), 0, &mut fold, false)
            .unwrap();
        assert!(
            stats
                .stop_reason
                .as_deref()
                .is_some_and(|r| r.contains("exceeds the record bound")),
            "{:?}",
            stats.stop_reason
        );
        assert_eq!(stats.hours_emitted, 3, "records before the stop still fold");
    }

    /// Hour-boundary release: a file-backed fold NEVER retains a closed hour's
    /// snapshot in memory — `take_finished()` stays empty while every hour is
    /// accounted, so peak memory does not grow with the hour count.
    #[test]
    fn file_backed_fold_releases_each_hour() {
        let dir = TmpDir::new("release");
        let stream = three_hour_stream();
        let mut fold = WorldFold::new_file_backed("gid", &dir.0.join("snaps")).unwrap();
        let stats =
            drive_fold(Cursor::new(stream.clone()), stream.len() as u64, &mut fold).unwrap();
        assert_eq!(stats.hours_emitted, 3);
        // Nothing accumulated in the in-memory sink: each hour spilled + released.
        assert!(
            fold.take_finished().is_empty(),
            "file-backed fold must not retain snapshots in memory"
        );
        assert_eq!(fold.emitted_hours_count(), 3);
        let files = fold.finish_file_backed().unwrap();
        assert_eq!(files.len(), 3);
        for f in &files {
            assert!(
                f.path.exists(),
                "hour {} snapshot spilled to disk",
                f.hour_index
            );
        }
    }

    /// A stream that ends mid-record yields only the complete hours and reports the
    /// truncated tail (no partial snapshot, no panic).
    #[test]
    fn truncated_tail_emits_complete_hours_only() {
        let dir = TmpDir::new("trunc");
        let mut stream = three_hour_stream();
        stream.truncate(stream.len() - 5); // chop hour C's frame_end
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();
        let (files, stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 2, "only complete hours emit");
        assert_eq!(files[0].hour_index, HOUR_A);
        assert_eq!(files[1].hour_index, HOUR_B);
        assert!(stats.truncated_tail_bytes > 0, "incomplete tail reported");
    }

    #[test]
    fn torn_zstd_frame_at_complete_record_boundary_fails_fold() {
        let dir = TmpDir::new("zstd_trunc");
        let stream = three_hour_stream();
        let raw_path = dir.0.join("raw.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&raw_path, &stream).unwrap();
        compact_stream::encode_file(
            &raw_path,
            &zstd_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        let mut stored = std::fs::read(&zstd_path).unwrap();
        stored.pop().expect("checksummed zstd frame is non-empty");
        std::fs::write(&zstd_path, stored).unwrap();

        let err = match fold_compact_stream_file(&zstd_path, "gid", &dir.0.join("snaps")) {
            Ok(_) => panic!("stream_fold must reject a torn zstd frame"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(err.to_string().contains("incomplete zstd frame"));
    }

    // Review finding (fix round 2): corruption that mangles a record HEADER
    // makes framing stop gracefully BEFORE the zstd footer, bypassing the
    // content checksum. On a compressed source that must now hard-error; on a
    // raw source the tolerant stop-with-reason behavior is preserved exactly.
    #[test]
    fn header_corruption_in_compressed_stream_fails_fold_loudly() {
        let dir = TmpDir::new("zstd_hdr_flip");
        let clean = three_hour_stream();
        let mut corrupted = clean.clone();
        // First record starts at 0; walk one record forward with the public
        // framing primitive and clobber the SECOND record's magic byte, so
        // framing stops mid-stream inside checksummed-valid zstd bytes.
        let h = parse_compact_record_header(&clean).unwrap();
        let second = h.record_len();
        corrupted[second] ^= 0xFF;

        let raw_path = dir.0.join("raw.bin");
        let zstd_src = dir.0.join("src.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&raw_path, &corrupted).unwrap();
        std::fs::write(&zstd_src, &corrupted).unwrap();
        compact_stream::encode_file(
            &zstd_src,
            &zstd_path,
            compact_stream::StreamCompression::Zstd,
        )
        .unwrap();

        // Compressed: hard error, never a clean short fold.
        let err = match fold_compact_stream_file(&zstd_path, "gid", &dir.0.join("zs")) {
            Ok(_) => panic!("compressed header corruption must fail the fold"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            err.to_string()
                .contains("framing stopped before physical EOF"),
            "unexpected error: {err}"
        );

        // Raw: byte-identical tolerant behavior (stop reason preserved; the
        // corruption sits before hour A's FRAME_END, so no hour completes).
        let (files, stats) =
            fold_compact_stream_file(&raw_path, "gid", &dir.0.join("raws")).unwrap();
        assert!(
            files.is_empty(),
            "corruption before the first FRAME_END emits no hours"
        );
        assert!(stats.stop_reason.is_some(), "raw stop keeps its diagnostic");
    }

    /// Scripted reader discriminating the post-framing-stop drain: serves a
    /// prepared chunk, then either errors (checksum found during drain) or
    /// EOFs (intact storage ⇒ the framing stop is the hard error).
    struct ScriptedReader {
        prefix: Vec<u8>,
        served: bool,
        tail_error: Option<(io::ErrorKind, &'static str)>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if !self.served {
                self.served = true;
                let n = self.prefix.len().min(buf.len());
                buf[..n].copy_from_slice(&self.prefix[..n]);
                assert_eq!(n, self.prefix.len(), "test prefix must fit one chunk");
                return Ok(n);
            }
            match self.tail_error {
                Some((kind, msg)) => Err(io::Error::new(kind, msg)),
                None => Ok(0),
            }
        }
    }

    fn one_record_then_garbage() -> Vec<u8> {
        let clean = three_hour_stream();
        let first_len = parse_compact_record_header(&clean).unwrap().record_len();
        let mut bytes = clean[..first_len].to_vec();
        bytes.extend_from_slice(b"XXXXGARBAGEHEADERXXXXGARBAGEHEADER");
        bytes
    }

    #[test]
    fn fold_drain_after_framing_stop_propagates_the_storage_root_error() {
        let reader = ScriptedReader {
            prefix: one_record_then_garbage(),
            served: false,
            tail_error: Some((
                io::ErrorKind::InvalidData,
                "simulated zstd checksum mismatch",
            )),
        };
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(reader, 0, &mut fold, true)
            .expect_err("checksum error during drain must surface");
        assert!(
            err.to_string().contains("simulated zstd checksum mismatch"),
            "root error must propagate, got: {err}"
        );
    }

    #[test]
    fn fold_framing_stop_with_intact_storage_is_itself_the_hard_error() {
        let reader = ScriptedReader {
            prefix: one_record_then_garbage(),
            served: false,
            tail_error: None,
        };
        let mut fold = WorldFold::new("gid");
        let err = drive_fold_compressed(reader, 0, &mut fold, true)
            .expect_err("compressed framing stop must not end cleanly");
        assert!(
            err.to_string()
                .contains("framing stopped before physical EOF"),
            "unexpected error: {err}"
        );
    }

    // The PARALLEL fold call site must also thread compressed=true (a review
    // finding: only raw fixtures exercised the parallel path, so flipping its
    // flag to false went undetected).
    #[test]
    fn parallel_fold_rejects_compressed_header_corruption() {
        let dir = TmpDir::new("zstd_par_hdr");
        let clean = three_hour_stream();
        let mut corrupted = clean.clone();
        let first_len = parse_compact_record_header(&clean).unwrap().record_len();
        corrupted[first_len] ^= 0xFF;
        let src = dir.0.join("src.bin");
        let zstd_path = dir.0.join("compressed.bin");
        std::fs::write(&src, &corrupted).unwrap();
        compact_stream::encode_file(&src, &zstd_path, compact_stream::StreamCompression::Zstd)
            .unwrap();
        let err =
            match fold_compact_stream_file_parallel(&zstd_path, "gid", &dir.0.join("snaps"), 2) {
                Ok(_) => panic!("parallel fold must reject compressed header corruption"),
                Err(err) => err,
            };
        assert!(
            err.to_string()
                .contains("framing stopped before physical EOF"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn discard_snapshot_reports_match_keep_mode_and_remove_files() {
        let dir = TmpDir::new("discard");
        let stream = three_hour_stream();

        let keep_path = dir.0.join("keep.bin");
        std::fs::write(&keep_path, &stream).unwrap();
        let (keep_files, keep_stats) =
            fold_compact_stream_file(&keep_path, "gid", &dir.0.join("keep_snaps")).unwrap();
        assert_eq!(keep_stats.hours_emitted, 3);
        let keep_rows = hash_folded_snapshot_files(&keep_files, false).unwrap();
        let keep_chain_hash = snapshot_chain_hash(&keep_rows);
        assert_eq!(keep_rows.len(), 3);
        for f in &keep_files {
            assert!(
                f.path.exists(),
                "keep mode should retain {}",
                f.path.display()
            );
        }

        let discard_path = dir.0.join("discard.bin");
        std::fs::write(&discard_path, &stream).unwrap();
        let (discard_files, discard_stats) =
            fold_compact_stream_file(&discard_path, "gid", &dir.0.join("discard_snaps")).unwrap();
        assert_eq!(discard_stats.hours_emitted, 3);
        let discard_rows = hash_folded_snapshot_files(&discard_files, true).unwrap();
        let discard_chain_hash = snapshot_chain_hash(&discard_rows);

        assert_eq!(discard_rows, keep_rows);
        assert_eq!(discard_chain_hash, keep_chain_hash);
        for f in &discard_files {
            assert!(
                !f.path.exists(),
                "discard mode should remove {}",
                f.path.display()
            );
        }
        assert!(
            std::fs::read_dir(dir.0.join("discard_snaps"))
                .unwrap()
                .next()
                .is_none(),
            "discard snapshot directory should be empty"
        );
    }

    /// The emit worker's streamed SHA-256 equals a fresh hash of the exact bytes
    /// it wrote (tee-writer correctness), for every spilled hour.
    #[test]
    fn stored_snapshot_sha_matches_file_bytes() {
        let dir = TmpDir::new("teesha");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();
        let (files, _) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("snaps")).unwrap();
        assert_eq!(files.len(), 3);
        for f in &files {
            let bytes = std::fs::read(&f.path).unwrap();
            assert_eq!(bytes.len(), f.byte_len);
            assert_eq!(
                f.sha256.as_deref(),
                Some(to_hex(&Sha256::digest(&bytes)).as_str()),
                "stored sha for hour {} must match spilled bytes",
                f.hour_index
            );
        }
    }

    /// G1 at the unit level for the parallel fold: for several worker counts
    /// (including more workers than hours), the merged strided output is
    /// byte-identical to the sequential fold — same hour set, same per-hour
    /// hashes, same chain hash, same stats accounting.
    #[test]
    fn parallel_fold_matches_sequential_for_all_worker_counts() {
        let dir = TmpDir::new("par");
        let stream = three_hour_stream();
        let stream_path = dir.0.join("s.bin");
        std::fs::write(&stream_path, &stream).unwrap();

        let (seq_files, seq_stats) =
            fold_compact_stream_file(&stream_path, "gid", &dir.0.join("seq")).unwrap();
        let seq_rows = hash_folded_snapshot_files(&seq_files, false).unwrap();
        let seq_chain = snapshot_chain_hash(&seq_rows);
        assert_eq!(seq_stats.hours_emitted, 3);

        for workers in [2usize, 3, 5] {
            let out = dir.0.join(format!("par{workers}"));
            let (par_files, par_stats) =
                fold_compact_stream_file_parallel(&stream_path, "gid", &out, workers).unwrap();
            assert_eq!(
                par_stats.hours_emitted, seq_stats.hours_emitted,
                "workers={workers}"
            );
            assert_eq!(par_stats.records_total, seq_stats.records_total);
            assert_eq!(par_stats.bytes_folded, seq_stats.bytes_folded);
            let par_rows = hash_folded_snapshot_files(&par_files, false).unwrap();
            assert_eq!(par_rows, seq_rows, "workers={workers} rows diverge");
            assert_eq!(
                snapshot_chain_hash(&par_rows),
                seq_chain,
                "workers={workers} chain diverges"
            );
            // Ordering contract: merged descriptors ascend by hour_index exactly
            // like the sequential emit order.
            let hours: Vec<i64> = par_files.iter().map(|f| f.hour_index).collect();
            assert_eq!(hours, vec![HOUR_A, HOUR_B, HOUR_C]);
        }
    }
}
