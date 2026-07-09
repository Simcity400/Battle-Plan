//! `fold_check` — the fold-equivalence gate (G1) for the extraction pipeline.
//!
//! Given a collector compact record stream (`collector_stream.bin`) or a
//! `.battleplan` container, it decodes every record and drives the **existing
//! production fold** ([`recorder_host::world_fold::WorldFold`]) to produce the
//! per-game-hour world snapshots, then emits a canonical SHA-256 per hour.
//!
//! There is exactly ONE decode+fold in the codebase. This tool now reuses it
//! through the shared [`replay_decoder`] facade: `replay_decoder` frames the
//! `H4CR` records (via the single `compact_contract::parse_compact_record_header`
//! primitive), drives the production `recorder_host::world_fold::WorldFold`, and
//! yields each game-hour's canonical snapshot bytes in bounded memory. `fold_check`
//! just hashes those bytes. Folding under the fixed [`FOLD_CHECK_GAME_ID`] makes a
//! raw stream and the same stream packed in a container hash identically, so an
//! optimized collector can be certified byte-identical (Extraction Optimization
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
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use replay_decoder::{DecodeStats, OpenOptions, Source};
use serde_json::{json, Map, Value};
use snapshot::sha256::{to_hex, Sha256};

/// Fixed synthetic game id so the folded snapshot's `game_id` field (and thus the
/// hash) depends only on the record stream, never on the file path. Two files
/// with identical stream bytes therefore hash identically — the whole point.
const FOLD_CHECK_GAME_ID: &str = "fold_check";

/// The decode options every `fold_check` run uses: fold under the fixed
/// [`FOLD_CHECK_GAME_ID`] regardless of source, so a raw stream and the same
/// stream packed in a container hash identically.
fn fold_check_opts() -> OpenOptions {
    OpenOptions {
        game_id: Some(FOLD_CHECK_GAME_ID.to_string()),
    }
}

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

/// Convert a [`replay_decoder`] pass's totals into the report `Totals`. The two
/// share the same accounting shape (the decoder mirrors what this tool used to
/// count inline); this is a plain field copy.
fn totals_from_stats(s: &DecodeStats) -> Totals {
    Totals {
        stream_bytes: s.stream_bytes,
        bytes_folded: s.bytes_folded,
        truncated_tail_bytes: s.truncated_tail_bytes,
        records_total: s.records_total,
        data_records: s.data_records,
        frame_end_records: s.frame_end_records,
        perf_records: s.perf_records,
        other_records: s.other_records,
        hours_emitted: s.hours_emitted,
        per_domain_data_records: s.per_domain_data_records.clone(),
        stop_reason: s.stop_reason.clone(),
    }
}

/// Fold an input file through the shared [`replay_decoder`] cursor (auto-detecting
/// a raw `collector_stream.bin` vs a `.battleplan` container), invoking `on_hour`
/// as each game-hour closes so the caller can hash-and-release. The decoder folds
/// under [`FOLD_CHECK_GAME_ID`] regardless of source. The container path inherits
/// the in-RAM entry reader's per-entry cap; use a raw stream for multi-GiB checks.
fn fold_input(
    path: &Path,
    mut on_hour: impl FnMut(i64, &str, &[u8], &BTreeMap<u16, u64>),
) -> Result<(Totals, &'static str), String> {
    let source = Source::detect(path).map_err(|e| e.to_string())?;
    let kind = match source {
        Source::StreamFile(_) => "raw_stream",
        Source::Container(_) => "container",
    };
    let mut cursor = source
        .open_cursor(&fold_check_opts())
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| format!("fold {}: {e}", path.display()))?;
        on_hour(
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
        );
    }
    Ok((totals_from_stats(cursor.stats()), kind))
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

/// Fold an already-opened in-memory reader into a [`FoldRun`] through the shared
/// [`replay_decoder`] cursor. Used by the unit tests, which fold an in-memory
/// `Cursor` (under [`FOLD_CHECK_GAME_ID`]) without going through a file.
#[cfg(test)]
fn collect_run(reader: std::io::Cursor<Vec<u8>>, total_len: u64) -> Result<FoldRun, String> {
    let mut hours: Vec<HourReport> = Vec::new();
    let mut cursor = replay_decoder::open_reader(reader, total_len, FOLD_CHECK_GAME_ID);
    while let Some(hour) = cursor.next_hour() {
        let hour = hour.map_err(|e| e.to_string())?;
        push_hour(
            &mut hours,
            hour.hour_index,
            &hour.date,
            &hour.snapshot,
            &hour.domain_data_counts,
        );
    }
    Ok(FoldRun {
        hours,
        totals: totals_from_stats(cursor.stats()),
    })
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
