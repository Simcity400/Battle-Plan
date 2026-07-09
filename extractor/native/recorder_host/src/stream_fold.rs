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
use std::io::{self, BufReader, Read};
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
    /// Total bytes of the on-disk stream file.
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
    let file = std::fs::File::open(stream_path)?;
    let total_len = file.metadata()?.len();
    let reader = BufReader::with_capacity(CHUNK, file);
    let mut fold = WorldFold::new_file_backed(game_id, snapshot_dir)?;
    let stats = drive_fold(reader, total_len, &mut fold)?;
    let files = fold
        .finish_file_backed()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    Ok((files, stats))
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
                            let file = std::fs::File::open(stream_path)?;
                            let total_len = file.metadata()?.len();
                            let reader = BufReader::with_capacity(CHUNK, file);
                            let mut fold = WorldFold::new_file_backed_strided(
                                game_id,
                                snapshot_dir,
                                workers as u64,
                                k as u64,
                            )?;
                            let stats = drive_fold(reader, total_len, &mut fold)?;
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
                && reference.stop_reason == worker_stats.stop_reason;
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
    let mut stats = FoldStreamStats {
        stream_bytes: total_len,
        ..FoldStreamStats::default()
    };
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;

    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
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

            stats.records_total += 1;
            match header.record_type {
                COMPACT_RECORD_TYPE_DOMAIN_DATA => stats.data_records += 1,
                COMPACT_RECORD_TYPE_FRAME_END => stats.frame_end_records += 1,
                COMPACT_RECORD_TYPE_DOMAIN_PERF => stats.perf_records += 1,
                _ => stats.other_records += 1,
            }

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
            break 'read;
        }
    }

    // A file-backed fold spills as it goes; `emitted_hours` is the authoritative
    // count of closed game-hours in either mode.
    stats.bytes_folded = processed;
    stats.truncated_tail_bytes = total_len.saturating_sub(processed);
    stats.hours_emitted = fold.emitted_hours_count() as u64;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::calendar::DATE_EPOCH_CONST;
    use compact_contract::{
        COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_FRAME_END,
        COMPACT_RECORD_VERSION,
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

    const HOUR_A: i64 = 16_959_360;
    const HOUR_B: i64 = 16_959_361;
    const HOUR_C: i64 = 16_959_362;

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
                &[],
            );
            seq += 1;
        }
        s
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
