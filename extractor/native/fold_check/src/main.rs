//! `fold_check` — the fold-equivalence gate (G1) for the extraction pipeline.
//!
//! Given a collector compact record stream (`collector_stream.bin`) or a
//! `.battleplan` container, it decodes every record and drives the **existing
//! production fold** ([`recorder_host::world_fold::WorldFold`]) to produce the
//! per-game-hour world snapshots, then emits a canonical SHA-256 per hour.
//!
//! There is exactly ONE decode+fold in the codebase: this tool reuses it via
//! [`WorldFold::ingest_record`] / [`WorldFold::take_finished`]. Only the 32-byte
//! record envelope (`H4CR` header — a stable wire contract in `compact_contract`)
//! is framed here, so the stream can be walked incrementally without buffering a
//! multi-GB file in memory. That is what lets the tool certify that an optimized
//! collector folds byte-identically to the current one (Extraction Optimization
//! Plan, gate G1).
//!
//! Usage:
//!   fold_check hash    <input> [--out report.json]
//!   fold_check compare <a> <b> [--out report.json] [--sample N]
//!
//! `<input>` is auto-detected: a stream begins with the `H4CR` magic; a container
//! begins with the ZIP local-file magic and its `collector_compact_stream` entry
//! is extracted and folded.
//!
//! `hash` prints a deterministic JSON report (per-hour hash + per-domain record
//! counts + totals + a chain hash) to stdout. `compare` exits 0 iff both inputs
//! fold to the identical hour range with identical per-hour hashes; on divergence
//! it reports the first divergent hour with a bounded per-section drill-down.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use compact_contract::{
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
};
use recorder_host::world_fold::WorldFold;
use serde_json::{json, Map, Value};
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::{to_hex, Sha256};

/// Fixed synthetic game id so the folded snapshot's `game_id` field (and thus the
/// hash) depends only on the record stream, never on the file path. Two files
/// with identical stream bytes therefore hash identically — the whole point.
const FOLD_CHECK_GAME_ID: &str = "fold_check";

/// Streaming read granularity for the raw-stream path.
const CHUNK: usize = 8 * 1024 * 1024;

/// ZIP local-file header magic (`PK\x03\x04`).
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

/// Manifest role of the compact stream inside a `.battleplan` container.
const STREAM_ROLE: &str = "collector_compact_stream";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let rc = match args.get(1).map(String::as_str) {
        Some("hash") => cmd_hash(&args[2..]),
        Some("compare") => cmd_compare(&args[2..]),
        Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(0)
        }
        Some(other) => {
            eprintln!("fold_check: unknown subcommand {other:?}\n");
            print_usage();
            Ok(2)
        }
    };
    match rc {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("fold_check: error: {e}");
            ExitCode::from(1)
        }
    }
}

fn print_usage() {
    eprintln!(
        "fold_check — fold-equivalence gate (G1)\n\
         \n\
         USAGE:\n\
         \x20 fold_check hash    <input> [--out report.json]\n\
         \x20 fold_check compare <a> <b> [--out report.json] [--sample N]\n\
         \n\
         <input> is a collector_stream.bin OR a .battleplan container.\n\
         hash    prints a deterministic per-hour hash report.\n\
         compare exits 0 iff the two inputs fold identically."
    );
}

// ---------------------------------------------------------------------------
// Shared: fold-run accounting
// ---------------------------------------------------------------------------

/// One closed game-hour: the canonical hash of its folded snapshot plus the
/// per-domain DOMAIN_DATA record counts that fed it.
struct HourReport {
    hour_index: i64,
    date: String,
    snapshot_hash: String,
    snapshot_bytes: usize,
    domain_data_counts: BTreeMap<u16, u64>,
}

#[derive(Default)]
struct Totals {
    stream_bytes: u64,
    bytes_folded: u64,
    truncated_tail_bytes: u64,
    records_total: u64,
    data_records: u64,
    frame_end_records: u64,
    perf_records: u64,
    other_records: u64,
    hours_emitted: u64,
    per_domain_data_records: BTreeMap<u16, u64>,
    stop_reason: Option<String>,
}

struct FoldRun {
    hours: Vec<HourReport>,
    totals: Totals,
}

/// Drive the production fold over a record stream, invoking `on_hour` as each
/// game-hour closes so the caller can hash-and-release without holding every
/// snapshot. `total_len` is the full decoded-stream length (file size, or the
/// container entry's uncompressed size) so the truncated tail can be reported.
fn drive_fold<R: Read>(
    mut reader: R,
    total_len: u64,
    mut on_hour: impl FnMut(i64, &str, &[u8], &BTreeMap<u16, u64>),
) -> io::Result<Totals> {
    let mut fold = WorldFold::new(FOLD_CHECK_GAME_ID);
    let mut totals = Totals {
        stream_bytes: total_len,
        ..Totals::default()
    };
    // DOMAIN_DATA counts since the last FRAME_END (the domains feeding this hour).
    let mut cur_counts: BTreeMap<u16, u64> = BTreeMap::new();
    let mut carry: Vec<u8> = Vec::with_capacity(CHUNK * 2);
    let mut chunk = vec![0u8; CHUNK];
    let mut processed: u64 = 0;

    'read: loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..n]);

        // Frame every whole record currently buffered at the front of `carry`.
        let mut off = 0usize;
        let mut hard_stop = false;
        loop {
            let remaining = carry.len() - off;
            if remaining < COMPACT_RECORD_HEADER_LEN {
                break; // need more bytes (or EOF tail)
            }
            let hdr = &carry[off..off + COMPACT_RECORD_HEADER_LEN];
            if &hdr[0..4] != COMPACT_RECORD_MAGIC {
                totals.stop_reason = Some(format!(
                    "invalid record magic at byte {}",
                    processed + off as u64
                ));
                hard_stop = true;
                break;
            }
            let version = u16::from_le_bytes([hdr[4], hdr[5]]);
            if version != COMPACT_RECORD_VERSION {
                totals.stop_reason = Some(format!(
                    "unsupported record version {version} at byte {}",
                    processed + off as u64
                ));
                hard_stop = true;
                break;
            }
            let header_len = u16::from_le_bytes([hdr[6], hdr[7]]) as usize;
            if header_len != COMPACT_RECORD_HEADER_LEN {
                totals.stop_reason = Some(format!(
                    "unsupported header length {header_len} at byte {}",
                    processed + off as u64
                ));
                hard_stop = true;
                break;
            }
            let game_hour_raw = i32::from_le_bytes([hdr[16], hdr[17], hdr[18], hdr[19]]);
            let domain_id = u16::from_le_bytes([hdr[20], hdr[21]]);
            let record_type = u16::from_le_bytes([hdr[22], hdr[23]]);
            let payload_len = u32::from_le_bytes([hdr[24], hdr[25], hdr[26], hdr[27]]) as usize;
            let record_flags = u32::from_le_bytes([hdr[28], hdr[29], hdr[30], hdr[31]]);
            let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
            if record_len > remaining {
                break; // payload not fully buffered yet
            }
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];

            totals.records_total += 1;
            match record_type {
                COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                    totals.data_records += 1;
                    *totals.per_domain_data_records.entry(domain_id).or_default() += 1;
                    *cur_counts.entry(domain_id).or_default() += 1;
                }
                COMPACT_RECORD_TYPE_FRAME_END => totals.frame_end_records += 1,
                COMPACT_RECORD_TYPE_DOMAIN_PERF => totals.perf_records += 1,
                _ => totals.other_records += 1,
            }

            // The single production fold. PERF / probe records carry no folded
            // state and are ignored inside `ingest_record`, exactly as the
            // whole-buffer `ingest_stream` ignores them.
            fold.ingest_record(game_hour_raw, domain_id, record_type, payload, record_flags);

            if record_type == COMPACT_RECORD_TYPE_FRAME_END {
                for (h, date, bytes) in fold.take_finished() {
                    on_hour(h, &date, &bytes, &cur_counts);
                    totals.hours_emitted += 1;
                }
                cur_counts.clear();
            }
            off += record_len;
        }

        processed += off as u64;
        carry.drain(0..off);
        if hard_stop {
            break 'read;
        }
    }

    totals.bytes_folded = processed;
    totals.truncated_tail_bytes = total_len.saturating_sub(processed);
    Ok(totals)
}

/// Read a compact stream from disk, auto-detecting a raw stream vs a container,
/// and fold it. The raw path streams the file; the container path extracts the
/// verified `collector_compact_stream` entry through `snapshot::Container` and
/// folds it from memory. That container path intentionally inherits the in-RAM
/// reader's 2 GiB per-entry cap; use a raw `collector_stream.bin` input for
/// multi-GiB fold checks until fold_check grows a direct streaming container
/// entry reader.
fn fold_input(
    path: &Path,
    on_hour: impl FnMut(i64, &str, &[u8], &BTreeMap<u16, u64>),
) -> Result<(Totals, &'static str), String> {
    let kind = detect_kind(path)?;
    match kind {
        InputKind::RawStream => {
            let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
            let total_len = file
                .metadata()
                .map_err(|e| format!("stat {}: {e}", path.display()))?
                .len();
            let reader = BufReader::with_capacity(CHUNK, file);
            let totals = drive_fold(reader, total_len, on_hour)
                .map_err(|e| format!("fold {}: {e}", path.display()))?;
            Ok((totals, "raw_stream"))
        }
        InputKind::Container => {
            let bytes = extract_container_stream(path)?;
            let total_len = bytes.len() as u64;
            let totals = drive_fold(Cursor::new(bytes), total_len, on_hour)
                .map_err(|e| format!("fold {}: {e}", path.display()))?;
            Ok((totals, "container"))
        }
    }
}

enum InputKind {
    RawStream,
    Container,
}

/// Detect the input by its leading bytes, independent of filename: a compact
/// stream starts with `H4CR`; a `.battleplan` container is a zip (`PK\x03\x04`).
fn detect_kind(path: &Path) -> Result<InputKind, String> {
    let mut f = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut magic = [0u8; 4];
    let read =
        read_up_to(&mut f, &mut magic).map_err(|e| format!("read {}: {e}", path.display()))?;
    if read >= 4 && &magic == COMPACT_RECORD_MAGIC {
        Ok(InputKind::RawStream)
    } else if read >= 4 && &magic == ZIP_MAGIC {
        Ok(InputKind::Container)
    } else {
        Err(format!(
            "{}: unrecognized input (leading bytes {:02x?}); expected an H4CR stream or a .battleplan container",
            path.display(),
            &magic[..read.min(4)]
        ))
    }
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// Open a `.battleplan`, verify it, and return the raw bytes of the single
/// `collector_compact_stream` entry (integrity-checked against the manifest).
fn extract_container_stream(path: &Path) -> Result<Vec<u8>, String> {
    let container = Container::open(path, &ExpectedIdentity::default())
        .map_err(|e| format!("open container {}: {e}", path.display()))?;
    let entries = container.entries_by_role(STREAM_ROLE);
    match entries.len() {
        0 => Err(format!(
            "container {} has no entry with role {STREAM_ROLE:?}",
            path.display()
        )),
        1 => container
            .read_entry_bytes(&entries[0].path)
            .map_err(|e| format!("read {} from {}: {e}", entries[0].path, path.display())),
        n => Err(format!(
            "container {} has {n} entries with role {STREAM_ROLE:?}; expected exactly one",
            path.display()
        )),
    }
}

/// Record one closed hour into the running `hours` list. The fold's output is
/// canonical by construction — every accumulator is a BTreeMap/BTreeSet and
/// serde_json (no `preserve_order`) writes object keys sorted, so identical
/// input yields byte-identical snapshot bytes; we hash those shipped bytes.
fn push_hour(
    hours: &mut Vec<HourReport>,
    hour_index: i64,
    date: &str,
    bytes: &[u8],
    counts: &BTreeMap<u16, u64>,
) {
    hours.push(HourReport {
        hour_index,
        date: date.to_string(),
        snapshot_hash: to_hex(&Sha256::digest(bytes)),
        snapshot_bytes: bytes.len(),
        domain_data_counts: counts.clone(),
    });
}

/// Fold an input file into an in-memory [`FoldRun`] (per-hour hashes + totals).
fn run_input(path: &Path) -> Result<(FoldRun, &'static str), String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let (totals, kind) = fold_input(path, |hour_index, date, bytes, counts| {
        push_hour(&mut hours, hour_index, date, bytes, counts)
    })?;
    Ok((FoldRun { hours, totals }, kind))
}

/// Fold an already-opened reader into a [`FoldRun`]. Used by the unit tests,
/// which fold an in-memory `Cursor` without going through a file.
#[cfg(test)]
fn collect_run<R: Read>(reader: R, total_len: u64) -> Result<FoldRun, String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let totals = drive_fold(reader, total_len, |hour_index, date, bytes, counts| {
        push_hour(&mut hours, hour_index, date, bytes, counts)
    })
    .map_err(|e| e.to_string())?;
    Ok(FoldRun { hours, totals })
}

/// Chain hash over the ordered per-hour hashes: a single anchor whose equality
/// implies identical hour range and identical per-hour folded state.
fn chain_hash(hours: &[HourReport]) -> String {
    let mut h = Sha256::new();
    for hr in hours {
        h.update(hr.hour_index.to_le_bytes().as_slice());
        h.update(hr.snapshot_hash.as_bytes());
        h.update(b"\n");
    }
    to_hex(&h.finalize())
}

// ---------------------------------------------------------------------------
// hash
// ---------------------------------------------------------------------------

fn cmd_hash(args: &[String]) -> Result<u8, String> {
    let (positional, out) = parse_common(args)?;
    if positional.len() != 1 {
        return Err("hash takes exactly one <input>".into());
    }
    let path = Path::new(&positional[0]);
    let (run, kind) = run_input(path)?;
    let report = hash_report(path, kind, &run);
    emit(&report, out.as_deref())?;
    Ok(0)
}

fn hash_report(path: &Path, kind: &str, run: &FoldRun) -> Value {
    let mut per_domain = Map::new();
    for (dom, n) in &run.totals.per_domain_data_records {
        per_domain.insert(dom.to_string(), json!({ "data_records": n }));
    }
    let hours: Vec<Value> = run
        .hours
        .iter()
        .map(|hr| {
            let mut doms = Map::new();
            for (d, c) in &hr.domain_data_counts {
                doms.insert(d.to_string(), json!(c));
            }
            json!({
                "hour_index": hr.hour_index,
                "date": hr.date,
                "snapshot_hash": hr.snapshot_hash,
                "snapshot_bytes": hr.snapshot_bytes,
                "domain_data_counts": Value::Object(doms),
            })
        })
        .collect();
    let range = hour_range(&run.hours);
    json!({
        "tool": "fold_check",
        "mode": "hash",
        "report_version": 1,
        "input": path.display().to_string(),
        "input_kind": kind,
        "game_id": FOLD_CHECK_GAME_ID,
        "totals": totals_json(&run.totals),
        "hour_range": range,
        "per_domain": Value::Object(per_domain),
        "hours": hours,
        "chain_hash": chain_hash(&run.hours),
    })
}

fn totals_json(t: &Totals) -> Value {
    let mut v = json!({
        "stream_bytes": t.stream_bytes,
        "bytes_folded": t.bytes_folded,
        "truncated_tail_bytes": t.truncated_tail_bytes,
        "records_total": t.records_total,
        "data_records": t.data_records,
        "frame_end_records": t.frame_end_records,
        "perf_records_ignored": t.perf_records,
        "other_records_ignored": t.other_records,
        "hours_emitted": t.hours_emitted,
    });
    if let Some(reason) = &t.stop_reason {
        v.as_object_mut()
            .unwrap()
            .insert("framing_stopped".into(), json!(reason));
    }
    v
}

fn hour_range(hours: &[HourReport]) -> Value {
    match (hours.first(), hours.last()) {
        (Some(f), Some(l)) => json!([f.hour_index, l.hour_index]),
        _ => Value::Null,
    }
}

// ---------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------

fn cmd_compare(args: &[String]) -> Result<u8, String> {
    let (positional, out) = parse_common(args)?;
    let sample = parse_sample(args)?.unwrap_or(24);
    if positional.len() != 2 {
        return Err("compare takes exactly two inputs <a> <b>".into());
    }
    let path_a = Path::new(&positional[0]);
    let path_b = Path::new(&positional[1]);
    let (run_a, kind_a) = run_input(path_a)?;
    let (run_b, kind_b) = run_input(path_b)?;

    let map_a: BTreeMap<i64, &HourReport> = run_a.hours.iter().map(|h| (h.hour_index, h)).collect();
    let map_b: BTreeMap<i64, &HourReport> = run_b.hours.iter().map(|h| (h.hour_index, h)).collect();

    // First divergent hour across the union of hour indices, in order.
    let mut all_hours: Vec<i64> = map_a.keys().chain(map_b.keys()).copied().collect();
    all_hours.sort_unstable();
    all_hours.dedup();

    let mut first_divergence: Option<Value> = None;
    for h in &all_hours {
        match (map_a.get(h), map_b.get(h)) {
            (Some(a), Some(b)) => {
                if a.snapshot_hash != b.snapshot_hash {
                    first_divergence =
                        Some(divergence_hash_mismatch(*h, a, b, path_a, path_b, sample)?);
                    break;
                }
            }
            (Some(a), None) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_b",
                    "a_hash": a.snapshot_hash,
                    "b_hash": Value::Null,
                }));
                break;
            }
            (None, Some(b)) => {
                first_divergence = Some(json!({
                    "hour_index": h,
                    "kind": "missing_in_a",
                    "a_hash": Value::Null,
                    "b_hash": b.snapshot_hash,
                }));
                break;
            }
            (None, None) => {}
        }
    }

    let identical = first_divergence.is_none();
    let report = json!({
        "tool": "fold_check",
        "mode": "compare",
        "report_version": 1,
        "identical": identical,
        "a": side_summary(path_a, kind_a, &run_a),
        "b": side_summary(path_b, kind_b, &run_b),
        "hour_count_match": run_a.hours.len() == run_b.hours.len(),
        "first_divergence": first_divergence.unwrap_or(Value::Null),
    });
    emit(&report, out.as_deref())?;
    Ok(if identical { 0 } else { 1 })
}

fn side_summary(path: &Path, kind: &str, run: &FoldRun) -> Value {
    json!({
        "input": path.display().to_string(),
        "input_kind": kind,
        "hours": run.hours.len(),
        "hour_range": hour_range(&run.hours),
        "chain_hash": chain_hash(&run.hours),
        "totals": totals_json(&run.totals),
    })
}

/// Build the drill-down for a hash-mismatched hour: re-fold BOTH inputs, capture
/// only that hour's snapshot bytes, and diff the two folded Values.
fn divergence_hash_mismatch(
    hour: i64,
    a: &HourReport,
    b: &HourReport,
    path_a: &Path,
    path_b: &Path,
    sample: usize,
) -> Result<Value, String> {
    let bytes_a = capture_hour(path_a, hour)?;
    let bytes_b = capture_hour(path_b, hour)?;
    let (section_diffs, sections) = match (bytes_a, bytes_b) {
        (Some(ba), Some(bb)) => {
            let va: Value = serde_json::from_slice(&ba)
                .map_err(|e| format!("parse snapshot a hour {hour}: {e}"))?;
            let vb: Value = serde_json::from_slice(&bb)
                .map_err(|e| format!("parse snapshot b hour {hour}: {e}"))?;
            let mut diffs: Vec<Value> = Vec::new();
            diff_values(&va, &vb, "", &mut diffs, sample);
            (Value::Array(diffs), section_summary(&va, &vb))
        }
        _ => (Value::Null, Value::Null),
    };

    let mut domain_diffs = Map::new();
    let doms: std::collections::BTreeSet<u16> = a
        .domain_data_counts
        .keys()
        .chain(b.domain_data_counts.keys())
        .copied()
        .collect();
    for d in doms {
        let ca = a.domain_data_counts.get(&d).copied().unwrap_or(0);
        let cb = b.domain_data_counts.get(&d).copied().unwrap_or(0);
        if ca != cb {
            domain_diffs.insert(d.to_string(), json!({ "a": ca, "b": cb }));
        }
    }

    Ok(json!({
        "hour_index": hour,
        "kind": "hash_mismatch",
        "date": a.date,
        "a_hash": a.snapshot_hash,
        "b_hash": b.snapshot_hash,
        "a_snapshot_bytes": a.snapshot_bytes,
        "b_snapshot_bytes": b.snapshot_bytes,
        "domain_data_count_diffs": Value::Object(domain_diffs),
        "sections": sections,
        "section_diffs_truncated_at": sample,
        "section_diffs": section_diffs,
    }))
}

/// Re-fold `path`, keeping only the snapshot bytes for `target` hour.
fn capture_hour(path: &Path, target: i64) -> Result<Option<Vec<u8>>, String> {
    let mut captured: Option<Vec<u8>> = None;
    fold_input(path, |hour_index, _date, bytes, _counts| {
        if hour_index == target && captured.is_none() {
            captured = Some(bytes.to_vec());
        }
    })?;
    Ok(captured)
}

/// Per top-level snapshot section (the fold's output shape): the "row count" in
/// each side and whether they are equal. A concise map of where the divergence
/// lives before the fine-grained `section_diffs`.
fn section_summary(a: &Value, b: &Value) -> Value {
    let empty = Map::new();
    let am = a.as_object().unwrap_or(&empty);
    let bm = b.as_object().unwrap_or(&empty);
    let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
    let mut out = Map::new();
    for k in keys {
        let av = am.get(k);
        let bv = bm.get(k);
        let equal = av == bv;
        if !equal {
            out.insert(
                k.clone(),
                json!({
                    "a_rows": value_len(av),
                    "b_rows": value_len(bv),
                    "equal": false,
                }),
            );
        }
    }
    Value::Object(out)
}

/// "Row count" of a value: array/object length, 0 for null/absent, 1 for a scalar.
fn value_len(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => json!(0),
        Some(Value::Array(a)) => json!(a.len()),
        Some(Value::Object(o)) => json!(o.len()),
        Some(_) => json!(1),
    }
}

/// Recursive bounded structural diff. Collects at most `limit` differing leaves,
/// each with its path and a short preview of both sides.
fn diff_values(a: &Value, b: &Value, path: &str, out: &mut Vec<Value>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            let keys: std::collections::BTreeSet<&String> = am.keys().chain(bm.keys()).collect();
            for k in keys {
                if out.len() >= limit {
                    return;
                }
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}/{k}")
                };
                match (am.get(k), bm.get(k)) {
                    (Some(x), Some(y)) => diff_values(x, y, &child, out, limit),
                    (Some(x), None) => out.push(json!({
                        "path": child, "kind": "only_in_a", "a": preview(x), "b": Value::Null,
                    })),
                    (None, Some(y)) => out.push(json!({
                        "path": child, "kind": "only_in_b", "a": Value::Null, "b": preview(y),
                    })),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            if aa.len() != ba.len() {
                out.push(json!({
                    "path": path, "kind": "array_len", "a": aa.len(), "b": ba.len(),
                }));
            }
            let n = aa.len().min(ba.len());
            for i in 0..n {
                if out.len() >= limit {
                    return;
                }
                diff_values(&aa[i], &ba[i], &format!("{path}[{i}]"), out, limit);
            }
        }
        _ => {
            if a != b {
                out.push(json!({
                    "path": path, "kind": "value", "a": preview(a), "b": preview(b),
                }));
            }
        }
    }
}

/// A bounded preview of a value for the drill-down (scalars pass through; large
/// containers/strings are summarized so the report stays small).
fn preview(v: &Value) -> Value {
    match v {
        Value::Array(a) => json!(format!("<array len {}>", a.len())),
        Value::Object(o) => json!(format!("<object len {}>", o.len())),
        Value::String(s) if s.len() > 120 => json!(format!("{}…({} chars)", &s[..117], s.len())),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// arg parsing + output
// ---------------------------------------------------------------------------

/// Extract positional args and an optional `--out <file>`.
fn parse_common(args: &[String]) -> Result<(Vec<String>, Option<String>), String> {
    let mut positional = Vec::new();
    let mut out = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = Some(args.get(i).ok_or("--out requires a file path")?.clone());
            }
            "--sample" => {
                i += 1; // value validated in parse_sample; skip here
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other}"));
            }
            other => positional.push(other.to_string()),
        }
        i += 1;
    }
    Ok((positional, out))
}

fn parse_sample(args: &[String]) -> Result<Option<usize>, String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--sample" {
            let v = args.get(i + 1).ok_or("--sample requires a count")?;
            return v
                .parse::<usize>()
                .map(Some)
                .map_err(|_| format!("--sample expects a non-negative integer, got {v:?}"));
        }
        i += 1;
    }
    Ok(None)
}

/// Write a report as deterministic pretty JSON to a file or stdout.
fn emit(report: &Value, out: Option<&str>) -> Result<(), String> {
    let mut text =
        serde_json::to_string_pretty(report).map_err(|e| format!("serialize report: {e}"))?;
    text.push('\n');
    match out {
        Some(path) => {
            File::create(path)
                .and_then(|mut f| f.write_all(text.as_bytes()))
                .map_err(|e| format!("write {path}: {e}"))?;
        }
        None => {
            io::stdout()
                .write_all(text.as_bytes())
                .map_err(|e| format!("write stdout: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
