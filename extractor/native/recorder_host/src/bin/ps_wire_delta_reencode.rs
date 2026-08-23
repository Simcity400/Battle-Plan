//! Offline re-encode of a sealed H4CR spool through `compact_contract::payload_span_delta`.
//!
//! What it does, per DOMAIN_DATA record of every delta-enabled domain, in stream
//! order:
//!
//!   1. wraps the payload as a `PSD1` record against the producer's baseline,
//!   2. unwraps it through an INDEPENDENT consumer baseline store,
//!   3. asserts the unwrapped bytes are byte-identical to the original payload,
//!   4. accounts the before/after wire bytes and the body kind.
//!
//! Step 3 is the round-trip proof, run against every hour of a real capture
//! rather than against a fixture: the producer and consumer here are the same
//! two objects the collector and the fold use, driven in the same order, so a
//! drift between them is a failure of this tool.
//!
//! It also prices the whole spool: bytes for the enabled domains before and
//! after, bytes for everything else unchanged, and the resulting total. And it
//! times the producer, because the encoder runs on the collector thread after
//! the reader fan-out joins and its cost has to be weighed against the sink time
//! it removes.
//!
//! `--rewrite <path>` additionally writes a WHOLE re-encoded spool: every record
//! of the original, in order, with the enabled domains' payloads replaced by
//! their `PSD1` envelopes and the envelope flag set on those records' H4CR
//! headers. Folding that file and the original and comparing the snapshot chain
//! hash is the end-to-end equality gate: the fold must produce bit-identical
//! hours either way, because the payloads it folds are bit-identical either way.
//!
//! Usage:
//!   ps_wire_delta_reencode <collector_stream.bin> [--json out.json]
//!                          [--domains 43,47,53] [--rewrite out.bin]

use std::collections::BTreeMap;
use std::env;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::time::Instant;

use compact_contract::payload_span_delta as psd;
use compact_contract::{
    parse_compact_record_header, COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_DOMAIN_DATA,
};
use serde::Serialize;

/// The shipped enabled set, restated so `--domains` can still price a
/// candidate domain that is not enabled yet.
const DEFAULT_DOMAINS: [u16; 4] = [43, 47, 53, 81];

/// The `ICCP` companion carrier's magic
/// (`diplomacy_intel_operatives_live::a30_companion_carrier::MAGIC`).
///
/// # Why this tool has to know about it
///
/// d53's DOMAIN_DATA records are not one grammar. Production wraps only the
/// records the main emit path appends;
/// `attach_log::insert_a30_payloads_before_frame_end` splices companion rows into
/// an already-built frame and those ride UNWRAPPED, because their first bytes are
/// this magic and not the `elapsed_wall_ms` the delta basis is defined against.
/// Production makes that distinction by CALL SITE, which a tool reading a sealed
/// spool cannot see.
///
/// So this tool makes it by content, and reports the count
/// (`companion_records_passed_through`) rather than assuming it is zero. Wrapping
/// them would advance the d53 baseline through payloads the shipping producer
/// never wraps, and every size below that record would then measure something the
/// recorder does not do — its round trips would still pass, which is exactly why
/// it has to be stated rather than trusted. (Codex review round 3, finding 2.)
const A30_COMPANION_MAGIC: &[u8; 4] = b"ICCP";

fn main() {
    if let Err(error) = run() {
        eprintln!("ps_wire_delta_reencode: {error}");
        std::process::exit(2);
    }
}

#[derive(Default, Serialize)]
struct DomainRow {
    domain_id: u16,
    data_records: u64,
    payload_bytes_before: u64,
    payload_bytes_after: u64,
    keyframes: u64,
    no_change: u64,
    deltas: u64,
    keyframe_bytes: u64,
    no_change_bytes: u64,
    delta_bytes: u64,
    /// Largest single wrapped record, in bytes.
    max_wrapped_bytes: u64,
    /// Producer wall time, microseconds, summed over every record.
    encode_us: u64,
    /// Consumer wall time, microseconds, summed over every record.
    decode_us: u64,
    round_trip_mismatches: u64,
    /// Records this tool deliberately did NOT wrap because production does not:
    /// d53's `ICCP` companion rows. See [`A30_COMPANION_MAGIC`].
    companion_records_passed_through: u64,
    /// Selected-domain records that arrived ALREADY enveloped. This tool prices
    /// raw spools; such records are passed through verbatim (and decoded, so a
    /// damaged envelope is a mismatch) rather than wrapped a second time.
    already_enveloped_records: u64,
    encode_errors: Vec<String>,
    decode_errors: Vec<String>,
}

impl DomainRow {
    fn ratio(&self) -> f64 {
        if self.payload_bytes_after == 0 {
            0.0
        } else {
            self.payload_bytes_before as f64 / self.payload_bytes_after as f64
        }
    }
}

#[derive(Serialize)]
struct Report {
    schema: &'static str,
    stream: String,
    enabled_domains: Vec<u16>,
    records_total: u64,
    /// Every DOMAIN_DATA payload byte in the spool, enabled or not.
    spool_domain_data_bytes_before: u64,
    spool_domain_data_bytes_after: u64,
    /// Every record byte in the spool, H4CR headers included.
    spool_record_bytes_before: u64,
    spool_record_bytes_after: u64,
    domains: Vec<DomainRow>,
    round_trip_records: u64,
    round_trip_mismatches: u64,
    /// Total records passed through unwrapped because production does not wrap
    /// them. Zero on both committed captures.
    companion_records_passed_through: u64,
    /// The producer's own `retained_bytes` at its highest, sampled just before
    /// each frame's commit: every enabled domain's committed baseline plus what
    /// the frame has staged. It does NOT include the reconstruction buffers a
    /// consumer holds, nor any transient the collector's other work allocates.
    peak_retained_bytes: u64,
}

fn run() -> Result<(), String> {
    let (input, json_output, domains, rewrite) = parse_args()?;
    let mut rewritten = match rewrite.as_ref() {
        Some(path) => Some(BufWriter::with_capacity(
            1 << 22,
            File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?,
        )),
        None => None,
    };
    let mut reader = compact_stream::open(&input)
        .map_err(|error| format!("open {}: {error}", input.display()))?;

    let mut producer = psd::PayloadSpanDeltaProducer::new();
    let mut consumer = psd::PayloadSpanDeltaConsumer::new();
    let mut rows: BTreeMap<u16, DomainRow> = BTreeMap::new();

    let mut records_total = 0u64;
    let mut data_bytes_before = 0u64;
    let mut data_bytes_after = 0u64;
    let mut record_bytes_before = 0u64;
    let mut record_bytes_after = 0u64;
    let mut round_trip_records = 0u64;
    let mut round_trip_mismatches = 0u64;
    let mut peak_retained_bytes = 0u64;
    let mut open_hour: Option<i32> = None;

    let mut carry: Vec<u8> = Vec::with_capacity(1 << 22);
    let mut chunk = vec![0u8; 1 << 22];
    let mut consumed = 0usize;
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| format!("read {}: {error}", input.display()))?;
        if read == 0 {
            break;
        }
        carry.extend_from_slice(&chunk[..read]);
        let mut off = 0usize;
        while carry.len() - off >= COMPACT_RECORD_HEADER_LEN {
            let header = match parse_compact_record_header(&carry[off..]) {
                Ok(header) => header,
                Err(error) => return Err(format!("byte {}: {error}", consumed + off)),
            };
            let record_len = header.record_len();
            if carry.len() - off < record_len {
                break;
            }
            // THE PRODUCER'S TRANSACTION CLOSES WITH THE FRAME. A frame is one
            // game hour of records, so the hour changing is the frame seal the
            // collector performs in `commit_delta_pin_transactions`.
            if open_hour.is_some_and(|hour| hour != header.game_hour_raw) {
                producer.commit();
            }
            open_hour = Some(header.game_hour_raw);
            records_total += 1;
            record_bytes_before += record_len as u64;
            let payload = &carry[off + COMPACT_RECORD_HEADER_LEN..off + record_len];

            let mut wire_len = payload.len() as u64;
            let mut rewritten_payload: Option<Vec<u8>> = None;
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
                data_bytes_before += payload.len() as u64;
                if domains.contains(&header.domain_id) {
                    let row = rows.entry(header.domain_id).or_insert_with(|| DomainRow {
                        domain_id: header.domain_id,
                        ..DomainRow::default()
                    });
                    row.data_records += 1;

                    // THIS HARNESS PRICES RAW SPOOLS. A record that arrives
                    // already enveloped is passed through VERBATIM — wrapping it
                    // again would measure a double envelope no collector ever
                    // produces, and its round trip would pass against the INNER
                    // bytes, which is exactly the kind of vacuous success this
                    // tool exists to prevent. The existing envelope is still
                    // decoded, so a damaged one is counted as a mismatch rather
                    // than silently carried. (Codex review round 6.)
                    if header.record_flags
                        & compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                        != 0
                    {
                        row.already_enveloped_records += 1;
                        // BEFORE == AFTER: the record rides verbatim, so its
                        // contribution to the per-domain ratio is 1.0x and the
                        // ratio stays a ratio (a missing before-entry would
                        // understate it toward zero).
                        row.payload_bytes_before += payload.len() as u64;
                        row.payload_bytes_after += payload.len() as u64;
                        let started = Instant::now();
                        let unwrapped = psd::reconstruct_if_enveloped(
                            &mut consumer,
                            header.record_flags,
                            psd::PayloadSpanDeltaIdentity {
                                domain_id: header.domain_id,
                                source_sequence: header.sequence,
                                game_hour_raw: header.game_hour_raw,
                            },
                            payload,
                        );
                        row.decode_us += started.elapsed().as_micros() as u64;
                        round_trip_records += 1;
                        match unwrapped {
                            Ok(Some(_)) => {}
                            Ok(None) | Err(_) => {
                                row.round_trip_mismatches += 1;
                                round_trip_mismatches += 1;
                                if row.decode_errors.len() < 8 {
                                    row.decode_errors.push(format!(
                                        "already-enveloped record at sequence {} did not reconstruct",
                                        header.sequence
                                    ));
                                }
                            }
                        }
                        data_bytes_after += wire_len;
                        record_bytes_after +=
                            (COMPACT_RECORD_HEADER_LEN as u64) + wire_len;
                        if let Some(writer) = rewritten.as_mut() {
                            writer
                                .write_all(&carry[off..off + record_len])
                                .map_err(|e| format!("write rewritten record: {e}"))?;
                        }
                        off += record_len;
                        continue;
                    }

                    row.payload_bytes_before += payload.len() as u64;
                    if payload.len() >= 4 && &payload[..4] == A30_COMPANION_MAGIC {
                        row.companion_records_passed_through += 1;
                        row.payload_bytes_after += payload.len() as u64;
                        data_bytes_after += payload.len() as u64;
                        record_bytes_after +=
                            (COMPACT_RECORD_HEADER_LEN as u64) + payload.len() as u64;
                        if let Some(writer) = rewritten.as_mut() {
                            writer
                                .write_all(&carry[off..off + record_len])
                                .map_err(|e| format!("write rewritten record: {e}"))?;
                        }
                        off += record_len;
                        continue;
                    }

                    // The envelope carries no domain semantics, so the offline
                    // producer is exactly the one the collector runs: same
                    // object, same order, same inputs.
                    let identity = psd::PayloadSpanDeltaIdentity {
                        domain_id: header.domain_id,
                        source_sequence: header.sequence,
                        game_hour_raw: header.game_hour_raw,
                    };
                    let started = Instant::now();
                    let wrapped = producer.wrap(identity, payload);
                    row.encode_us += started.elapsed().as_micros() as u64;
                    // SAMPLED BEFORE THE FRAME'S COMMIT, and the commit happens
                    // at the HOUR BOUNDARY below rather than after every record,
                    // because that is where the collector's own
                    // `commit_delta_pin_transactions` runs. Committing per record
                    // would never leave two domains staged at once and would
                    // report a peak the collector does not have (Codex rounds 2
                    // and 4). What this counts is every enabled domain's
                    // committed baseline plus whatever this frame has staged so
                    // far -- the producer's own `retained_bytes`.
                    peak_retained_bytes = peak_retained_bytes.max(producer.retained_bytes() as u64);
                    match wrapped {
                        Ok(wrapped) => {
                            row.payload_bytes_after += wrapped.len() as u64;
                            row.max_wrapped_bytes = row.max_wrapped_bytes.max(wrapped.len() as u64);
                            wire_len = wrapped.len() as u64;
                            rewritten_payload = Some(wrapped.clone());
                            match psd::decode_payload(&wrapped).map(|record| record.body_kind()) {
                                Ok(psd::BODY_KIND_KEYFRAME) => {
                                    row.keyframes += 1;
                                    row.keyframe_bytes += wrapped.len() as u64;
                                }
                                Ok(psd::BODY_KIND_NO_CHANGE) => {
                                    row.no_change += 1;
                                    row.no_change_bytes += wrapped.len() as u64;
                                }
                                Ok(psd::BODY_KIND_DELTA) => {
                                    row.deltas += 1;
                                    row.delta_bytes += wrapped.len() as u64;
                                }
                                Ok(other) => row
                                    .decode_errors
                                    .push(format!("unassigned body kind {other}")),
                                Err(error) => row.decode_errors.push(error),
                            }

                            let started = Instant::now();
                            let unwrapped = psd::reconstruct_if_enveloped(
                                &mut consumer,
                                compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                                identity,
                                &wrapped,
                            );
                            row.decode_us += started.elapsed().as_micros() as u64;
                            round_trip_records += 1;
                            match unwrapped {
                                Ok(Some(bytes)) => {
                                    if bytes != payload {
                                        row.round_trip_mismatches += 1;
                                        round_trip_mismatches += 1;
                                        if row.decode_errors.len() < 8 {
                                            row.decode_errors.push(format!(
                                                "round trip differs at sequence {}",
                                                header.sequence
                                            ));
                                        }
                                    }
                                }
                                Ok(None) => {
                                    row.round_trip_mismatches += 1;
                                    round_trip_mismatches += 1;
                                    if row.decode_errors.len() < 8 {
                                        row.decode_errors
                                            .push("wrapped payload was not recognised".into());
                                    }
                                }
                                Err(error) => {
                                    row.round_trip_mismatches += 1;
                                    round_trip_mismatches += 1;
                                    if row.decode_errors.len() < 8 {
                                        row.decode_errors.push(error);
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            // The producer refused to wrap: the hour keeps the
                            // bytes it always had. That is the shipped fallback,
                            // and it is priced here as such.
                            row.payload_bytes_after += payload.len() as u64;
                            if row.encode_errors.len() < 8 {
                                row.encode_errors.push(error);
                            }
                        }
                    }
                }
                data_bytes_after += wire_len;
            }
            record_bytes_after += (COMPACT_RECORD_HEADER_LEN as u64) + wire_len;
            if let Some(writer) = rewritten.as_mut() {
                match rewritten_payload {
                    Some(wrapped) => {
                        let mut rebuilt =
                            carry[off..off + COMPACT_RECORD_HEADER_LEN].to_vec();
                        rebuilt[24..28].copy_from_slice(
                            &u32::try_from(wrapped.len())
                                .map_err(|_| "envelope longer than a record may carry")?
                                .to_le_bytes(),
                        );
                        let flags = header.record_flags
                            | compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA;
                        rebuilt[28..32].copy_from_slice(&flags.to_le_bytes());
                        writer
                            .write_all(&rebuilt)
                            .and_then(|_| writer.write_all(&wrapped))
                            .map_err(|e| format!("write rewritten record: {e}"))?;
                    }
                    None => writer
                        .write_all(&carry[off..off + record_len])
                        .map_err(|e| format!("write rewritten record: {e}"))?,
                }
            }
            off += record_len;
        }
        consumed += off;
        carry.drain(..off);
    }
    producer.commit();
    if let Some(mut writer) = rewritten {
        writer
            .flush()
            .map_err(|e| format!("flush rewritten spool: {e}"))?;
    }
    if !carry.is_empty() {
        return Err(format!("{} trailing byte(s) after the last record", carry.len()));
    }

    let companions;
    let report = Report {
        schema: "ps.wire_delta_reencode.v1",
        stream: input.display().to_string(),
        enabled_domains: domains.clone(),
        records_total,
        spool_domain_data_bytes_before: data_bytes_before,
        spool_domain_data_bytes_after: data_bytes_after,
        spool_record_bytes_before: record_bytes_before,
        spool_record_bytes_after: record_bytes_after,
        domains: {
            companions = rows
                .values()
                .map(|row| row.companion_records_passed_through)
                .sum();
            rows.into_values().collect()
        },
        round_trip_records,
        round_trip_mismatches,
        companion_records_passed_through: companions,
        peak_retained_bytes,
    };
    print_report(&report);
    if let Some(path) = json_output {
        let file =
            File::create(&path).map_err(|error| format!("create {}: {error}", path.display()))?;
        let mut writer = BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer, &report)
            .map_err(|error| format!("encode {}: {error}", path.display()))?;
        writer
            .write_all(b"\n")
            .and_then(|_| writer.flush())
            .map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    if report.round_trip_mismatches != 0 {
        return Err(format!(
            "{} record(s) failed the round trip",
            report.round_trip_mismatches
        ));
    }
    Ok(())
}

fn print_report(report: &Report) {
    println!("stream {}", report.stream);
    println!(
        "records {} | DOMAIN_DATA payload {} -> {} B | whole spool {} -> {} B ({:.3}x)",
        report.records_total,
        report.spool_domain_data_bytes_before,
        report.spool_domain_data_bytes_after,
        report.spool_record_bytes_before,
        report.spool_record_bytes_after,
        if report.spool_record_bytes_after == 0 {
            0.0
        } else {
            report.spool_record_bytes_before as f64 / report.spool_record_bytes_after as f64
        }
    );
    println!(
        "round trip {} record(s), {} mismatch(es) | {} companion record(s) passed through |          peak retained {} B",
        report.round_trip_records,
        report.round_trip_mismatches,
        report.companion_records_passed_through,
        report.peak_retained_bytes
    );
    println!(
        "{:>4} {:>7} {:>14} {:>13} {:>8} {:>5} {:>4} {:>6} {:>10} {:>10}",
        "dom", "records", "before B", "after B", "ratio", "kf", "nc", "delta", "enc ms", "dec ms"
    );
    for row in &report.domains {
        println!(
            "{:>4} {:>7} {:>14} {:>13} {:>7.2}x {:>5} {:>4} {:>6} {:>10.1} {:>10.1}",
            row.domain_id,
            row.data_records,
            row.payload_bytes_before,
            row.payload_bytes_after,
            row.ratio(),
            row.keyframes,
            row.no_change,
            row.deltas,
            row.encode_us as f64 / 1000.0,
            row.decode_us as f64 / 1000.0,
        );
        for error in row.encode_errors.iter().chain(row.decode_errors.iter()) {
            println!("     ! {error}");
        }
    }
}

type Args = (PathBuf, Option<PathBuf>, Vec<u16>, Option<PathBuf>);

fn parse_args() -> Result<Args, String> {
    let mut input: Option<PathBuf> = None;
    let mut json_output: Option<PathBuf> = None;
    let mut domains: Option<Vec<u16>> = None;
    let mut rewrite: Option<PathBuf> = None;
    let mut args = env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy().to_string();
        match text.as_str() {
            "--json" => {
                json_output = Some(PathBuf::from(
                    args.next().ok_or("--json needs a path")?,
                ));
            }
            "--rewrite" => {
                rewrite = Some(PathBuf::from(
                    args.next().ok_or("--rewrite needs a path")?,
                ));
            }
            "--domains" => {
                let raw = args
                    .next()
                    .ok_or("--domains needs a comma-separated list")?
                    .to_string_lossy()
                    .to_string();
                let mut parsed = Vec::new();
                for part in raw.split(',') {
                    parsed.push(
                        part.trim()
                            .parse::<u16>()
                            .map_err(|_| format!("domain id {part:?} is not a u16"))?,
                    );
                }
                domains = Some(parsed);
            }
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }
    Ok((
        input.ok_or(
            "usage: ps_wire_delta_reencode <collector_stream.bin> [--json out] \
             [--domains 43,47,53] [--rewrite out.bin]",
        )?,
        json_output,
        domains.unwrap_or_else(|| DEFAULT_DOMAINS.to_vec()),
        rewrite,
    ))
}
