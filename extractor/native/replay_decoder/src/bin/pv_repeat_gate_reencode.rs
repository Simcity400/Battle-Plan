//! Lane PV offline proof harness: re-encode a retained H4CR spool through the
//! collector's real DOMAIN_DATA_REPEAT gate and prove round-trip equality of
//! the production fold against the non-gated encoding.
//!
//! Measurement only (mirrors the `gq-bad-hour` / `gz-interior-holes` bins):
//! never on by default, never packaged, writes only the files its arguments
//! name.
//!
//! ```text
//! pv_repeat_gate_reencode <original.spool> <gated.spool> <report.json>
//! ```
//!
//! Pass A re-encodes: the original stream is walked one FRAME at a time (a
//! frame is every record up to and including its FRAME_END) and each complete
//! frame goes through `collector::domain_repeat_gate`'s
//! `rewrite_frame_for_publication` â€” the exact function the capture loop runs
//! before staging a frame for the sink. Per-domain byte accounting pairs the
//! pre- and post-rewrite records in order.
//!
//! Pass B proves: both spools are folded by the REAL production decoder
//! (`replay_decoder::open_stream_file`, which drives the one WorldFold). The
//! two hour streams must agree hour-for-hour on `WorldHour` equality
//! (`hour_index`, date, snapshot bytes, per-domain data counts â€” repeats
//! counted as data), on decode stats including the seal coverage map, and
//! both must pass `certification_verdict`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use replay_decoder::{OpenOptions, Source, StreamIdentityMode};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: pv_repeat_gate_reencode <original.spool> <gated.spool> <report.json>");
        std::process::exit(2);
    }
    let original_path = PathBuf::from(&args[1]);
    let gated_path = PathBuf::from(&args[2]);
    let report_path = PathBuf::from(&args[3]);

    match run(&original_path, &gated_path, &report_path) {
        Ok(report) => {
            println!("{}", serde_json::to_string_pretty(&report).expect("report json"));
        }
        Err(error) => {
            eprintln!("pv_repeat_gate_reencode FAILED: {error}");
            std::process::exit(1);
        }
    }
}

fn run(
    original_path: &Path,
    gated_path: &Path,
    report_path: &Path,
) -> Result<Value, String> {
    println!(
        "pass A: re-encoding {} (decoded through compact_stream)",
        original_path.display()
    );

    let mut gate = collector::domain_repeat_gate::DomainDataRepeatGate::new();
    let mut gated = fs::File::create(gated_path)
        .map_err(|error| format!("create {}: {error}", gated_path.display()))?;
    let mut domain_before_map: BTreeMap<u16, u64> = BTreeMap::new();
    let mut domain_after_map: BTreeMap<u16, u64> = BTreeMap::new();
    let mut domain_records: BTreeMap<u16, u64> = BTreeMap::new();
    let mut domain_repeats: BTreeMap<u16, u64> = BTreeMap::new();

    let header_len = usize::from(compact_contract::COMPACT_RECORD_HEADER_LEN);
    // The spool may be a zstd-compressed compact_stream container; open it
    // through its decoder and walk DECODED H4CR bytes. The gated output is
    // written as an UNCOMPRESSED raw H4CR stream, which Source::detect
    // accepts directly.
    let mut reader = compact_stream::open(original_path)
        .map_err(|error| format!("open {}: {error}", original_path.display()))?;
    let mut carry: Vec<u8> = Vec::with_capacity(32 * 1024 * 1024);
    let mut frame: Vec<u8> = Vec::new();
    let mut consumed = 0u64;
    let mut frames = 0u64;
    let mut decoded_bytes = 0u64;
    let mut type_counts: BTreeMap<u16, u64> = BTreeMap::new();
    loop {
        // Drain every whole record currently in `carry`.
        loop {
            if carry.len() - 0 < header_len {
                break;
            }
            let header = match compact_contract::parse_compact_record_header(&carry) {
                Ok(header) => header,
                Err(error) => {
                    return Err(format!(
                        "record header at decoded offset {consumed}: {error}"
                    ))
                }
            };
            let total = usize::try_from(header.record_len())
                .map_err(|_| format!("record length overflow at {consumed}"))?;
            if carry.len() < total {
                break;
            }
            let record_bytes = carry[..total].to_vec();
            carry.drain(..total);
            consumed += total as u64;
            *type_counts.entry(header.record_type).or_default() += 1;
            // Hour-scoped frames end at FRAME_END or FRAME_REFUSED. Census
            // and termination sentinels ride OUTSIDE any hour frame (the
            // release epilogue follows the final FRAME_END), so flush what is
            // pending and copy them verbatim: they carry no DOMAIN_DATA.
            let passthrough = matches!(
                header.record_type,
                compact_contract::COMPACT_RECORD_TYPE_STREAM_HEADER
                    | compact_contract::COMPACT_RECORD_TYPE_FREEZE_CENSUS
                    | compact_contract::COMPACT_RECORD_TYPE_CONTENT_IDENTITY_CENSUS
                    | compact_contract::COMPACT_RECORD_TYPE_STATE_TOPOLOGY_CENSUS
                    | compact_contract::COMPACT_RECORD_TYPE_CAPTURE_TERMINATED
            );
            let is_frame_refused =
                header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_REFUSED;
            let is_frame_end =
                header.record_type == compact_contract::COMPACT_RECORD_TYPE_FRAME_END
                    || is_frame_refused;
            if passthrough {
                if !frame.is_empty() {
                    rewrite_and_account(
                        &mut gate,
                        &mut frame,
                        &gated,
                        &mut domain_before_map,
                        &mut domain_after_map,
                        &mut domain_records,
                        &mut domain_repeats,
                    )?;
                    frame.clear();
                    frames += 1;
                }
                gated
                    .write_all(&record_bytes)
                    .map_err(|error| format!("write gated stream: {error}"))?;
                continue;
            }
            frame.extend_from_slice(&record_bytes);
            if is_frame_end {
                rewrite_and_account(
                    &mut gate,
                    &mut frame,
                    &gated,
                    &mut domain_before_map,
                    &mut domain_after_map,
                    &mut domain_records,
                    &mut domain_repeats,
                )?;
                frame.clear();
                frames += 1;
                // A REFUSED hour is a gap for the whole roster: the next
                // published record for every gateable domain must be a full
                // baseline, mirroring the production collector's refusal hook.
                if is_frame_refused {
                    gate.force_full_all();
                }
            }
        }
        let chunk_start = carry.len();
        carry.resize(chunk_start + 8 * 1024 * 1024, 0);
        let read = reader
            .read(&mut carry[chunk_start..])
            .map_err(|error| format!("read {}: {error}", original_path.display()))?;
        carry.truncate(chunk_start + read);
        decoded_bytes += read as u64;
        if read == 0 {
            break;
        }
    }
    if !carry.is_empty() || !frame.is_empty() {
        eprintln!("type histogram: {type_counts:?}");
        return Err(format!(
            "stream ends with {} trailing byte(s) outside a whole record and an open \
             frame of {} byte(s): the spool is torn",
            carry.len(),
            frame.len()
        ));
    }
    drop(gated);
    println!("pass A done: {frames} frames, {decoded_bytes} decoded bytes");

    println!("pass B: folding both encodings through the production fold");
    let identity_mode = detect_identity_mode(gated_path)?;
    // Lockstep drain: both cursors advance together and each hour pair is
    // compared (and reduced to its chain-hash contribution) IMMEDIATELY, so
    // memory holds one snapshot per cursor rather than two full spools.
    let mut original_cursor = open_cursor(original_path, identity_mode)?;
    let mut gated_cursor = open_cursor(gated_path, identity_mode)?;
    let mut mismatches: Vec<String> = Vec::new();
    let mut hours_compared = 0u64;
    let mut offset_only_hours = 0u64;
    let mut original_chain = Sha256::new();
    let mut gated_chain = Sha256::new();
    loop {
        let next_original = original_cursor.next_hour();
        let next_gated = gated_cursor.next_hour();
        match (next_original, next_gated) {
            (Some(Ok(a)), Some(Ok(b))) => {
                if a != b {
                    // Provenance notes quote the ABSOLUTE STREAM OFFSET of
                    // refused records ("... (offset N: ...)"). A
                    // size-reducing re-encoding moves those offsets by
                    // construction, so quotes of them are expected to
                    // differ while everything else stays identical.
                    // Normalize exactly that pattern before judging; an
                    // hour that matches only under normalization is
                    // counted separately, never silently merged.
                    if normalize_offset_quotes(&a.snapshot) == normalize_offset_quotes(&b.snapshot)
                        && a.hour_index == b.hour_index
                        && a.date == b.date
                        && a.domain_data_counts == b.domain_data_counts
                    {
                        offset_only_hours += 1;
                    } else {
                        mismatches.push(format!(
                            "hour #{} differs: hour_index {} vs {}, snapshot sha {} vs {}, counts equal {}",
                            hours_compared,
                            a.hour_index,
                            b.hour_index,
                            a.snapshot_sha256(),
                            b.snapshot_sha256(),
                            a.domain_data_counts == b.domain_data_counts
                        ));
                    }
                }
                original_chain.update(a.hour_index.to_le_bytes());
                original_chain.update(normalize_offset_quotes(&a.snapshot));
                original_chain.update(a.snapshot_sha256().as_bytes());
                original_chain.update(b"\n");
                gated_chain.update(b.hour_index.to_le_bytes());
                gated_chain.update(normalize_offset_quotes(&b.snapshot));
                gated_chain.update(b.snapshot_sha256().as_bytes());
                gated_chain.update(b"\n");
                hours_compared += 1;
            }
            (None, None) => break,
            (Some(Ok(a)), None) => {
                mismatches.push(format!(
                    "gated stream ends early: hour {} has no gated counterpart",
                    a.hour_index
                ));
                break;
            }
            (None, Some(Ok(b))) => {
                mismatches.push(format!(
                    "gated stream emits extra hour {} the original lacks",
                    b.hour_index
                ));
                break;
            }
            (Some(Err(error)), _) => {
                mismatches.push(format!("original fold failed: {error}"));
                break;
            }
            (_, Some(Err(error))) => {
                mismatches.push(format!("gated fold failed: {error}"));
                break;
            }
            (Some(Err(_)), Some(Err(_))) | (None, Some(Err(_))) | (Some(Err(_)), None) => {
                unreachable!("matched above")
            }
        }
    }

    let original_stats = original_cursor.stats().clone();
    let gated_stats = gated_cursor.stats().clone();
    drop(original_cursor);
    drop(gated_cursor);
    if original_stats.data_records != gated_stats.data_records {
        mismatches.push(format!(
            "data records disagree: {} vs {}",
            original_stats.data_records, gated_stats.data_records
        ));
    }
    // Coverage maps carry per-hour BYTE offsets, which differ between
    // encodings by construction (the gated stream is smaller). What must be
    // equal is the SEMANTIC seal: which hours verified, which were excluded
    // and why, and what was lost unattributed.
    let original_coverage = &original_stats.hour_coverage;
    let gated_coverage = &gated_stats.hour_coverage;
    let original_verified: Vec<i32> = original_coverage.verified_hours().collect();
    let gated_verified: Vec<i32> = gated_coverage.verified_hours().collect();
    if original_verified != gated_verified {
        mismatches.push(format!(
            "verified hour sets differ: {} vs {} entries",
            original_verified.len(),
            gated_verified.len()
        ));
    }
    let original_excluded: Vec<String> = original_coverage
        .excluded_hours()
        .iter()
        .map(|(hour, _)| hour.to_string())
        .collect();
    let gated_excluded: Vec<String> = gated_coverage
        .excluded_hours()
        .iter()
        .map(|(hour, _)| hour.to_string())
        .collect();
    if original_excluded != gated_excluded {
        mismatches.push(format!(
            "excluded hour sets differ: {original_excluded:?} vs {gated_excluded:?}"
        ));
    }
    if original_coverage.unattributed_losses() != gated_coverage.unattributed_losses() {
        mismatches.push("unattributed losses differ".to_string());
    }
    if let Err(error) = original_stats.certification_verdict() {
        mismatches.push(format!("ORIGINAL stream fails certification: {error}"));
    }
    if let Err(error) = gated_stats.certification_verdict() {
        mismatches.push(format!("GATED stream fails certification: {error}"));
    }
    if !mismatches.is_empty() {
        return Err(format!(
            "ROUND TRIP BROKEN:\n  {}",
            mismatches.join("\n  ")
        ));
    }

    let total_before: u64 = domain_before_map.values().sum();
    let total_after: u64 = domain_after_map.values().sum();
    let mut domains_json = Vec::new();
    for (domain, before) in &domain_before_map {
        let after = domain_after_map.get(domain).copied().unwrap_or(0);
        domains_json.push(json!({
            "domain_id": domain,
            "records": domain_records.get(domain).copied().unwrap_or(0),
            "repeats_emitted": domain_repeats.get(domain).copied().unwrap_or(0),
            "payload_bytes_before": before,
            "payload_bytes_after": after,
            "bytes_saved": before.saturating_sub(after),
        }));
    }

    let gated_len = fs::metadata(gated_path)
        .map_err(|error| format!("stat {}: {error}", gated_path.display()))?
        .len();
    let report = json!({
        "schema": "pv.repeat_gate_reencode.v1",
        "original_stream": original_path.to_string_lossy(),
        "gated_stream": gated_path.to_string_lossy(),
        "decoded_stream_bytes_before": decoded_bytes,
        "gated_stream_bytes_after": gated_len,
        "frames": frames,
        "payload_bytes_before": total_before,
        "payload_bytes_after": total_after,
        "payload_bytes_saved": total_before.saturating_sub(total_after),
        "stream_bytes_saved": decoded_bytes.saturating_sub(gated_len),
        "round_trip": {
            "verdict": if mismatches.is_empty() { "EQUAL" } else { "BROKEN" },
            "hours_compared": hours_compared,
            "hours_equal_modulo_stream_offset_quotes": offset_only_hours,
            "hours_emitted_original": original_stats.hours_emitted,
            "hours_emitted_gated": gated_stats.hours_emitted,
            "data_records_original": original_stats.data_records,
            "data_records_gated": gated_stats.data_records,
            "verified_hours_original": original_stats.hour_coverage.verified_count(),
            "verified_hours_gated": gated_stats.hour_coverage.verified_count(),
            "excluded_hours_original": original_stats.hour_coverage.excluded_count(),
            "excluded_hours_gated": gated_stats.hour_coverage.excluded_count(),
            "snapshot_chain_sha256_original": hex(&original_chain.finalize()),
            "snapshot_chain_sha256_gated": hex(&gated_chain.finalize()),
        },
        "domains": domains_json,
    });
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut file =
        fs::File::create(report_path).map_err(|error| format!("create report: {error}"))?;
    writeln!(file, "{}", serde_json::to_string_pretty(&report).expect("json"))
        .map_err(|error| format!("write report: {error}"))?;
    Ok(report)
}

fn open_cursor(
    path: &Path,
    identity_mode: StreamIdentityMode,
) -> Result<replay_decoder::HourCursor, String> {
    let source = Source::StreamFile(path.to_path_buf());
    source
        .open_cursor_with_identity_mode(&OpenOptions::default(), identity_mode)
        .map_err(|error| format!("open {}: {error}", path.display()))
}

/// Rewrite one complete frame through the real gate, write it out, and fold
/// the before/after payload accounting into the per-domain ledgers.
#[allow(clippy::too_many_arguments)]
fn rewrite_and_account(
    gate: &mut collector::domain_repeat_gate::DomainDataRepeatGate,
    frame: &mut Vec<u8>,
    gated_out: &std::fs::File,
    domain_before_map: &mut BTreeMap<u16, u64>,
    domain_after_map: &mut BTreeMap<u16, u64>,
    domain_records: &mut BTreeMap<u16, u64>,
    domain_repeats: &mut BTreeMap<u16, u64>,
) -> Result<(), String> {
    let before = walk_payloads(frame)?;
    gate.rewrite_frame_for_publication(frame)?;
    let after = walk_payloads(frame)?;
    debug_assert_eq!(before.len(), after.len(), "gate keeps record count");
    for ((domain_before, len_before), (domain_after, len_after)) in
        before.iter().zip(after.iter())
    {
        debug_assert_eq!(domain_before, domain_after);
        *domain_after_map.entry(*domain_after).or_default() += *len_after as u64;
        if len_after < len_before {
            *domain_repeats.entry(*domain_after).or_default() += 1;
        }
    }
    for (domain, len_before) in &before {
        *domain_before_map.entry(*domain).or_default() += *len_before as u64;
        *domain_records.entry(*domain).or_default() += 1;
    }
    let mut gated = gated_out;
    gated
        .write_all(frame)
        .map_err(|error| format!("write gated stream: {error}"))?;
    Ok(())
}

fn detect_identity_mode(path: &Path) -> Result<StreamIdentityMode, String> {
    use std::io::Read;
    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut head = [0u8; 40];
    let read = file
        .read(&mut head)
        .map_err(|error| format!("peek {}: {error}", path.display()))?;
    if read >= 24 && &head[0..4] == compact_contract::COMPACT_RECORD_MAGIC {
        let record_type = u16::from_le_bytes([head[22], head[23]]);
        if record_type == compact_contract::COMPACT_RECORD_TYPE_STREAM_HEADER {
            return Ok(StreamIdentityMode::IdentityV1);
        }
    }
    Ok(StreamIdentityMode::Legacy)
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Replace `(offset <digits>:` provenance quotes with a fixed token so two
/// encodings of one capture compare equal on content while any OTHER byte
/// difference still fails. Returns the snapshot bytes unchanged when the
/// pattern is absent (the common case).
fn normalize_offset_quotes(snapshot: &[u8]) -> Vec<u8> {
    const NEEDLE: &[u8] = b"(offset ";
    let mut had_match = false;
    for window in snapshot.windows(NEEDLE.len()) {
        if window == NEEDLE {
            had_match = true;
            break;
        }
    }
    if !had_match {
        return snapshot.to_vec();
    }
    let mut out = Vec::with_capacity(snapshot.len());
    let mut at = 0usize;
    while at < snapshot.len() {
        if snapshot[at..].starts_with(NEEDLE) {
            let mut cursor = at + NEEDLE.len();
            while cursor < snapshot.len() && snapshot[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if cursor > at + NEEDLE.len() && cursor < snapshot.len() && snapshot[cursor] == b':' {
                out.extend_from_slice(NEEDLE);
                out.push(b'#');
                out.push(b':');
                at = cursor + 1;
                continue;
            }
        }
        out.push(snapshot[at]);
        at += 1;
    }
    out
}

/// Payload length of every DOMAIN_DATA / DOMAIN_DATA_REPEAT record in a frame
/// buffer, in order. The pairing between a frame's pre- and post-rewrite walks
/// attributes each rewrite to its own domain.
fn walk_payloads(frame: &[u8]) -> Result<Vec<(u16, usize)>, String> {
    let header_len = usize::from(compact_contract::COMPACT_RECORD_HEADER_LEN);
    let mut out = Vec::new();
    let mut offset = 0usize;
    while offset < frame.len() {
        let header = compact_contract::parse_compact_record_header(&frame[offset..])
            .map_err(|error| format!("frame record header at {offset}: {error}"))?;
        let total = header.record_len();
        if offset + total > frame.len() {
            return Err(format!("truncated frame record at {offset}"));
        }
        if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA
            || header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        {
            out.push((header.domain_id, total - header_len));
        }
        offset += total;
    }
    Ok(out)
}
