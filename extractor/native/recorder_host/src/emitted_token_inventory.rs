use crate::domain_data_authority::{
    collect_static_join_tokens, DomainDataContext, StaticJoinToken,
};
use compact_contract::stream_validator::{
    AcceptedRecordKind, StatefulStreamValidator, StreamIdentityMode,
};
use serde_json::json;
use snapshot::sha256::{to_hex, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const EMITTED_TOKEN_INVENTORY_SCHEMA: &str = "extractor.emitted_token_inventory.v1";
const MAX_RECORD_LEN: usize = 256 * 1024 * 1024;

/// How many named reconstruction refusals are carried out of a scan verbatim.
///
/// One lost record refuses every hour of its domain until the 64-hour keyframe
/// cadence resynchronises the consumer, so a badly damaged spool produces
/// refusals in proportion to its damage. The COUNT is exact and unbounded; the
/// reasons are capped, because an unbounded string list built from a damaged
/// file is the same malformed-input amplification this codec closes everywhere
/// else.
const MAX_NAMED_RECONSTRUCTION_REFUSALS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedEmittedTokenInventory {
    pub path: PathBuf,
    pub sha256: String,
    pub token_count: usize,
    pub decoded_stream_bytes: u64,
    /// Records whose `PSD1` envelope could not be reconstructed, named. Empty on
    /// an undamaged capture. See [`ScannedStaticJoinTokens`].
    pub unreconstructed_records: Vec<String>,
    /// The exact number of them, which may exceed `unreconstructed_records.len()`
    /// once [`MAX_NAMED_RECONSTRUCTION_REFUSALS`] reasons have been kept.
    pub unreconstructed_record_count: u64,
}

/// What one walk of the spool produced.
///
/// `unreconstructed_records` is the reason this is a struct rather than a tuple.
/// A record whose envelope names a baseline this scan never saw -- because the
/// record carrying that baseline was dropped or damaged -- CANNOT contribute its
/// tokens, and that is a fact about the capture, not an error in reading it. It
/// is excluded and named here, and the scan continues; see the refusal arm in
/// `scan_stream` for why aborting instead is the wrong trade.
struct ScannedStaticJoinTokens {
    tokens: BTreeSet<StaticJoinToken>,
    decoded_stream_sha256: String,
    decoded_stream_bytes: u64,
    unreconstructed_records: Vec<String>,
    unreconstructed_record_count: u64,
}

pub fn build_and_write(
    stream_path: &Path,
    output_path: &Path,
    capture_id: &str,
    session_id: &str,
    expected_stream_sha256: &str,
    identity_mode: StreamIdentityMode,
) -> Result<VerifiedEmittedTokenInventory, String> {
    require_nonempty(capture_id, "capture_id")?;
    require_nonempty(session_id, "session_id")?;
    validate_sha256(expected_stream_sha256, "expected stream SHA-256")?;
    if output_path.exists() {
        return Err(format!(
            "emitted token inventory output already exists: {}",
            output_path.display()
        ));
    }

    let ScannedStaticJoinTokens {
        tokens,
        decoded_stream_sha256,
        decoded_stream_bytes,
        unreconstructed_records,
        unreconstructed_record_count,
    } = scan_stream(stream_path, identity_mode)?;
    if decoded_stream_sha256 != expected_stream_sha256 {
        return Err(format!(
            "decoded stream SHA-256 {} does not match capture {}",
            decoded_stream_sha256, expected_stream_sha256
        ));
    }
    if tokens.is_empty() {
        return Err(
            "completed capture emitted no static-join string tokens; refusing stale/empty inventory"
                .to_string(),
        );
    }

    let rows = tokens
        .iter()
        .map(|row| json!({"kind": row.kind, "token": row.token}))
        .collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec_pretty(&json!({
        "schema": EMITTED_TOKEN_INVENTORY_SCHEMA,
        "provenance": {
            "capture_id": capture_id,
            "session_id": session_id,
            "stream_sha256": decoded_stream_sha256,
        },
        "tokens": rows,
    }))
    .map_err(|error| format!("serialize emitted token inventory failed: {error}"))?;
    bytes.push(b'\n');
    write_create_new_durable(output_path, &bytes)?;
    Ok(VerifiedEmittedTokenInventory {
        path: output_path.to_path_buf(),
        sha256: snapshot::sha256::Sha256::hex(&bytes),
        token_count: tokens.len(),
        decoded_stream_bytes,
        unreconstructed_records,
        unreconstructed_record_count,
    })
}

fn scan_stream(
    stream_path: &Path,
    identity_mode: StreamIdentityMode,
) -> Result<ScannedStaticJoinTokens, String> {
    let mut reader = compact_stream::open(stream_path)
        .map_err(|error| format!("open emitted-token stream failed: {error}"))?;
    let mut validator = match identity_mode {
        StreamIdentityMode::Legacy => StatefulStreamValidator::legacy(),
        StreamIdentityMode::IdentityV1 => StatefulStreamValidator::identity_v1(),
    };
    let mut hasher = Sha256::new();
    let mut decoded_bytes = 0u64;
    let mut stream_offset = 0usize;
    let mut tokens = BTreeSet::new();
    // This scan is a WALKER OF THE SAME H4CR STREAM and reconstructs `PSD1`
    // envelopes for itself, like every other one. It has no `WorldFold` to borrow
    // baselines from, so it keeps its own store; it sees every record in stream
    // order, which is all the chain needs.
    //
    // Getting this wrong is not a degraded number, it is a lost capture:
    // `collect_static_join_tokens` decodes the domain grammar before deciding
    // whether the domain contributes tokens, so the first wrapped record of any
    // delta-enabled domain (d43/d47/d53 today) fails to decode, the `?` below
    // turns that into an error, and
    // `finalize_static_bundle` propagates it — no emitted-token inventory, no
    // static bundle, no replay publication. (Codex review round 8, finding 1: the
    // tenth walker, after the eight this lane found and the two round 7 did.)
    let mut span_delta_consumer =
        compact_contract::payload_span_delta::PayloadSpanDeltaConsumer::new();
    let mut unreconstructed_records: Vec<String> = Vec::new();
    let mut unreconstructed_record_count = 0u64;
    loop {
        let mut header_bytes = [0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        let count = read_up_to(&mut reader, &mut header_bytes)
            .map_err(|error| format!("read emitted-token H4CR header failed: {error}"))?;
        if count == 0 {
            break;
        }
        if count != header_bytes.len() {
            return Err(format!(
                "emitted-token stream has truncated H4CR header at offset {stream_offset}"
            ));
        }
        let header = compact_contract::parse_compact_record_header(&header_bytes)
            .map_err(|error| format!("H4CR header at offset {stream_offset} failed: {error}"))?;
        let record_len = header.record_len();
        if record_len > MAX_RECORD_LEN {
            return Err(format!(
                "H4CR record at offset {stream_offset} exceeds {MAX_RECORD_LEN} byte bound"
            ));
        }
        let mut payload = vec![0u8; header.payload_len];
        let payload_count = read_up_to(&mut reader, &mut payload).map_err(|error| {
            format!("read H4CR payload at offset {stream_offset} failed: {error}")
        })?;
        if payload_count != payload.len() {
            return Err(format!(
                "emitted-token stream has truncated H4CR payload at offset {stream_offset}"
            ));
        }
        hasher.update(&header_bytes);
        hasher.update(&payload);
        decoded_bytes = decoded_bytes.saturating_add(record_len as u64);

        match validator
            .validate_record(&header, &payload)
            .map_err(|error| format!("H4CR validation at offset {stream_offset} failed: {error}"))?
        {
            AcceptedRecordKind::CampaignIdentity => {}
            AcceptedRecordKind::Ordinary
                if header.record_type == compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA =>
            {
                let reconstructed =
                    compact_contract::payload_span_delta::reconstruct_if_enveloped(
                        &mut span_delta_consumer,
                        header.record_flags,
                        compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                            domain_id: header.domain_id,
                            source_sequence: header.sequence,
                            game_hour_raw: header.game_hour_raw,
                        },
                        &payload,
                    );
                let reconstructed = match reconstructed {
                    Ok(reconstructed) => reconstructed,
                    Err(reason) => {
                        // PARTIAL DATA STILL SEALS. A refusal here is NOT a
                        // corrupt file: it is the ordinary consequence of a
                        // record being dropped or damaged, which makes every
                        // later record of that domain refuse until the 64-hour
                        // keyframe cadence resynchronises the consumer. The first
                        // spelling of this fix propagated the refusal with `?`,
                        // and because `finalize_static_bundle` propagates in
                        // turn, ONE damaged wrapped record destroyed the
                        // emitted-token inventory, the static bundle and the
                        // replay publication for the WHOLE capture -- and stopped
                        // the walk before the cadence could recover it. (Codex
                        // review round 9, the blocker.)
                        //
                        // So the record is EXCLUDED AND NAMED, exactly as the
                        // fold walkers name it, and the scan continues. What that
                        // costs is bounded and visible. The inventory is the set
                        // of static-join tokens the capture emitted, and
                        // `account_emitted_tokens` in
                        // `extractor/game_defs/static_bundle.py` uses it to check
                        // that every emitted token resolves in the catalogs -- so
                        // a skipped record means the join gate checked LESS, never
                        // that it checked something false. d47 DOES have a
                        // token-collecting arm in `collect_static_join_tokens`
                        // (`collect_domain47_advisor_tokens`, lane QL), so a
                        // skipped d47 record costs its advisor tokens for that
                        // hour; the count and the reasons ride out to
                        // the session manifest either way, because "exclude and
                        // account" is not satisfied by a number nobody can see.
                        unreconstructed_record_count =
                            unreconstructed_record_count.saturating_add(1);
                        if unreconstructed_records.len() < MAX_NAMED_RECONSTRUCTION_REFUSALS {
                            unreconstructed_records.push(format!(
                                "domain {} hour {} sequence {} at offset {stream_offset}: {reason}",
                                header.domain_id, header.game_hour_raw, header.sequence
                            ));
                        }
                        stream_offset = stream_offset
                            .checked_add(record_len)
                            .ok_or_else(|| "decoded stream offset overflow".to_string())?;
                        continue;
                    }
                };
                // The flag describes the payload being handed over, and after the
                // reconstruction that payload is the domain's own grammar.
                let was_enveloped = reconstructed.is_some();
                let record_flags = if was_enveloped {
                    header.record_flags
                        & !compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA
                } else {
                    header.record_flags
                };
                let payload = reconstructed.as_deref().unwrap_or(&payload);
                match collect_static_join_tokens(
                    header.domain_id,
                    payload,
                    DomainDataContext {
                        stream_offset,
                        record_flags,
                        sequence: header.sequence,
                        header_hour_raw: header.game_hour_raw,
                        session_header_identity: None,
                        // Token collection only; no changed-row count is read.
                        xmap07_snow_mud_previous_grid_sha256: None,
                    },
                ) {
                    Ok(rows) => tokens.extend(rows),
                    Err(error) if was_enveloped => {
                        // SAME LAW AS THE RECONSTRUCTION REFUSAL ABOVE, one
                        // layer down: an ENVELOPED record can reconstruct
                        // faithfully yet still fail its grammar, because the
                        // codec's digest excludes the declared volatile prefix
                        // and for d81 those eight bytes are the payload's own
                        // RRL1 magic/version/length. The first spelling of the
                        // r9 fix stopped at the reconstruction arm, so this
                        // shape propagated with `?` and
                        // `finalize_static_bundle` destroyed the inventory, the
                        // static bundle and replay publication for the whole
                        // capture anyway. Excluded and named, exactly like the
                        // arm above; a RAW record with corrupt grammar keeps
                        // its pre-delta hard failure.
                        unreconstructed_record_count =
                            unreconstructed_record_count.saturating_add(1);
                        if unreconstructed_records.len() < MAX_NAMED_RECONSTRUCTION_REFUSALS {
                            unreconstructed_records.push(format!(
                                "domain {} hour {} sequence {} at offset {stream_offset}: \
                                 reconstructed envelope rejected by its grammar: {error}",
                                header.domain_id, header.game_hour_raw, header.sequence
                            ));
                        }
                    }
                    Err(error) => {
                        return Err(format!(
                            "static-join DATA decode failed at offset {stream_offset}: {error}"
                        ));
                    }
                }
            }
            AcceptedRecordKind::Ordinary => {}
        }
        stream_offset = stream_offset
            .checked_add(record_len)
            .ok_or_else(|| "decoded stream offset overflow".to_string())?;
    }
    Ok(ScannedStaticJoinTokens {
        tokens,
        decoded_stream_sha256: to_hex(&hasher.finalize()),
        decoded_stream_bytes: decoded_bytes,
        unreconstructed_records,
        unreconstructed_record_count,
    })
}

fn read_up_to(reader: &mut impl Read, output: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0usize;
    while filled < output.len() {
        match reader.read(&mut output[filled..])? {
            0 => break,
            count => filled += count,
        }
    }
    Ok(filled)
}

fn write_create_new_durable(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create inventory directory failed: {error}"))?;
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("create emitted token inventory failed: {error}"))?;
    let result = (|| {
        file.write_all(bytes)
            .map_err(|error| format!("write emitted token inventory failed: {error}"))?;
        file.flush()
            .map_err(|error| format!("flush emitted token inventory failed: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("sync emitted token inventory failed: {error}"))
    })();
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(path);
    }
    result
}

fn require_nonempty(value: &str, label: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!("{label} is not canonical lowercase SHA-256"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_provenance_rejects_empty_or_noncanonical_inputs_before_io() {
        let path = Path::new("missing-stream.bin");
        let out = Path::new("unused-inventory.json");
        assert!(build_and_write(
            path,
            out,
            "",
            "session",
            &"a".repeat(64),
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("capture_id"));
        assert!(build_and_write(
            path,
            out,
            "capture",
            "session",
            "AA",
            StreamIdentityMode::Legacy
        )
        .unwrap_err()
        .contains("canonical lowercase"));
    }

    /// One valid, EMPTY d53 payload: 72 bytes of header with every section count
    /// at zero. Built by hand from the lane's own public constants rather than
    /// from a fixture, so it is what `diplomacy53::compact::decode_payload`
    /// accepts and nothing more.
    fn d53_payload(elapsed_wall_ms: u64) -> Vec<u8> {
        use diplomacy_intel_operatives_live::compact as d53;
        let mut payload = vec![0u8; d53::HEADER_LEN];
        payload[0..8].copy_from_slice(&elapsed_wall_ms.to_le_bytes());
        payload[20..24].copy_from_slice(&d53::SCHEMA_VERSION.to_le_bytes());
        payload[24..28].copy_from_slice(&d53::FIXED_SCALE_RAW.to_le_bytes());
        payload
    }

    fn append_record(
        stream: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) {
        let mut header = vec![0u8; compact_contract::COMPACT_RECORD_HEADER_LEN];
        header[0..4].copy_from_slice(compact_contract::COMPACT_RECORD_MAGIC);
        header[4..6].copy_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        header[6..8].copy_from_slice(
            &(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
        );
        header[8..16].copy_from_slice(&sequence.to_le_bytes());
        header[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        header[20..22].copy_from_slice(&domain_id.to_le_bytes());
        header[22..24].copy_from_slice(&record_type.to_le_bytes());
        header[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        header[28..32].copy_from_slice(&record_flags.to_le_bytes());
        stream.extend_from_slice(&header);
        stream.extend_from_slice(payload);
    }

    const DOMAIN53: u16 = 53;
    const FIRST_HOUR: i32 = 60_830_824;

    struct BuiltStreams {
        raw: Vec<u8>,
        wrapped: Vec<u8>,
        envelopes: Vec<Vec<u8>>,
        /// Byte length of the FIRST wrapped record, header included, so a test
        /// can remove exactly that record and nothing else.
        first_wrapped_record_len: usize,
    }

    /// `hours` d53 hours, once raw and once wrapped, from one producer.
    fn build_streams(hours: i32) -> BuiltStreams {
        use compact_contract::payload_span_delta as psd;
        let mut producer = psd::PayloadSpanDeltaProducer::new();
        let mut raw = Vec::new();
        let mut wrapped = Vec::new();
        let mut envelopes: Vec<Vec<u8>> = Vec::new();
        let mut sequence = 1u64;
        for hour in 0..hours {
            let game_hour_raw = FIRST_HOUR + hour;
            let payload = d53_payload(hour as u64 * 1_000);
            append_record(
                &mut raw,
                sequence,
                game_hour_raw,
                DOMAIN53,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            let envelope = producer
                .wrap(
                    psd::PayloadSpanDeltaIdentity {
                        domain_id: DOMAIN53,
                        source_sequence: sequence,
                        game_hour_raw,
                    },
                    &payload,
                )
                .expect("wrap a valid d53 payload");
            append_record(
                &mut wrapped,
                sequence,
                game_hour_raw,
                DOMAIN53,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                &envelope,
            );
            envelopes.push(envelope);
            producer.commit();
            sequence += 1;
        }
        let first_wrapped_record_len =
            compact_contract::COMPACT_RECORD_HEADER_LEN + envelopes[0].len();
        BuiltStreams {
            raw,
            wrapped,
            envelopes,
            first_wrapped_record_len,
        }
    }

    /// A scratch directory that removes itself, so a failing assertion does not
    /// leave a stream file behind in the system temp directory.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "{label}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_nanos())
                    .unwrap_or_default()
            ));
            fs::create_dir_all(&root).expect("create scratch dir");
            Self(root)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).expect("write scratch stream");
            path
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_static_join_scan_reads_a_wrapped_stream_exactly_as_it_reads_the_raw_one() {
        // kills: `scan_stream` handing a `PSD1` envelope to
        // `collect_static_join_tokens` without reconstructing it (Codex review
        // round 8, finding 1). That function calls
        // `summarize_domain_data_payload` FIRST, for every domain, before the
        // match that decides d53 contributes no tokens -- so envelope bytes are
        // read as d53 grammar, the schema_version check fails, and the `?` here
        // turns it into an `Err`. `finalize_static_bundle` propagates that, so
        // the cost is not a missing token: it is no inventory, no static bundle,
        // and no replay publication for any capture containing a wrapped record.
        //
        // The vacuity control at the bottom is the whole point: it asserts that
        // the unreconstructed envelope really does fail to decode, so the
        // equality above is a statement about the reconstruction and not about
        // two payloads that would both have parsed anyway.
        use compact_contract::payload_span_delta as psd;

        let BuiltStreams {
            raw,
            wrapped,
            envelopes,
            ..
        } = build_streams(6);
        // kills: a producer that emitted six keyframes and therefore never
        // exercised a body that NEEDS the consumer's baseline. Only the first
        // record stands alone; the rest are header-only `NoChange` bodies, which
        // is what an unchanged empty payload should cost on the wire.
        // 112-byte envelope + the 64-byte BASIS: the 8-byte volatile header is
        // carried in the envelope itself, not a second time in the body.
        assert_eq!(envelopes[0].len(), psd::HEADER_LEN as usize + (72 - 8));
        for envelope in &envelopes[1..] {
            assert_eq!(envelope.len(), psd::HEADER_LEN as usize);
        }

        let scratch = ScratchDir::new("ps_static_join_scan");
        let raw_path = scratch.write("raw.bin", &raw);
        let wrapped_path = scratch.write("wrapped.bin", &wrapped);

        let raw_scan = scan_stream(&raw_path, StreamIdentityMode::Legacy).expect("raw stream scans");
        let wrapped_scan = scan_stream(&wrapped_path, StreamIdentityMode::Legacy)
            .expect("wrapped stream scans -- if this is an Err the walker did not unwrap");
        let raw_bytes = raw_scan.decoded_stream_bytes;
        let wrapped_bytes = wrapped_scan.decoded_stream_bytes;

        assert_eq!(
            raw_scan.tokens, wrapped_scan.tokens,
            "the static-join token set must not depend on the wire encoding"
        );
        // kills: a walker that "succeeded" by refusing every wrapped record and
        // reporting the refusals instead of reconstructing them. An intact stream
        // must produce no refusals at all.
        assert_eq!(wrapped_scan.unreconstructed_record_count, 0);
        assert!(wrapped_scan.unreconstructed_records.is_empty());
        // The two scans really did read different bytes, so the equality above
        // is not two identical files being compared to themselves.
        assert_ne!(raw, wrapped, "the wrapped stream must differ on the wire");
        // This synthetic payload is 72 bytes, the degenerate case for a 112-byte
        // envelope, and the wrapped stream is LARGER here. That is the codec's
        // worst case and it is bounded: `choose_wire_body` falls back to a
        // keyframe whenever the delta program would not beat the basis, so no
        // record ever exceeds `payload_len + (HEADER_LEN - VOLATILE_HEADER_LEN)`.
        // The size win is a claim about real d43/d53 payloads and is measured on
        // real spools in `extractor/native/runs/ps_wire_delta/`, not here.
        let records = envelopes.len() as u64;
        assert!(
            wrapped_bytes <= raw_bytes + records * psd::ENVELOPE_OVERHEAD as u64,
            "wrapped stream decoded {wrapped_bytes} B against raw {raw_bytes} B over              {records} records, past the bounded per-record envelope overhead"
        );

        // Vacuity control, and the reason the equality above means anything.
        let unreconstructed = collect_static_join_tokens(
            DOMAIN53,
            &envelopes[0],
            DomainDataContext {
                stream_offset: 0,
                record_flags: compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                sequence: 1,
                header_hour_raw: FIRST_HOUR,
                session_header_identity: None,
                xmap07_snow_mud_previous_grid_sha256: None,
            },
        );
        assert!(
            unreconstructed.is_err(),
            "envelope bytes parsed as d53 grammar, so this test could not tell \
             a reconstructing walker from one that skipped it"
        );
    }

    #[test]
    fn a_lost_record_costs_its_own_hours_and_not_the_capture() {
        // kills: propagating a PSD1 reconstruction refusal out of `scan_stream`
        // (Codex review round 9, the blocker). A refusal here is not a corrupt
        // file -- it is what a dropped or damaged record does to every later
        // record of its domain until the keyframe cadence recovers it. With the
        // `?` this fix originally used, `finalize_static_bundle` propagated it
        // and ONE lost record cost the capture its emitted-token inventory, its
        // static bundle and its replay publication; the walk also stopped before
        // reaching the cadence keyframe that would have recovered.
        //
        // Sixty-six hours, so the run spans a full 64-hour cadence, and the
        // FIRST record -- the only keyframe before it -- is removed.
        const HOURS: i32 = 66;
        const CADENCE: u64 = 64;

        // Modelled the way the collector really loses a record: the producer
        // wraps every hour, so the baseline chain advances, but the frame's drop
        // pass removes one record and RESEQUENCES the rest, which is why
        // `stamp_source_sequence` exists. Simply deleting the bytes would leave a
        // sequence hole the stateful validator rejects before reconstruction is
        // ever reached, and would prove nothing about this walker.
        let BuiltStreams { envelopes, .. } = build_streams(HOURS);
        let mut damaged = Vec::new();
        for (index, envelope) in envelopes.iter().enumerate().skip(1) {
            let sequence = index as u64;
            let mut envelope = envelope.clone();
            compact_contract::payload_span_delta::stamp_source_sequence(&mut envelope, sequence)
                .expect("stamp the post-drop sequence");
            append_record(
                &mut damaged,
                sequence,
                FIRST_HOUR + index as i32,
                DOMAIN53,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                &envelope,
            );
        }

        let scratch = ScratchDir::new("ps_static_join_lost_record");
        let path = scratch.write("damaged.bin", &damaged);
        let scan = scan_stream(&path, StreamIdentityMode::Legacy)
            .expect("a lost record must not end the walk");

        // 65 records survive the drop. The first 64 name the baseline the
        // removed record carried and refuse; the 65th is the cadence keyframe,
        // which stands alone. So the loss costs exactly the hours between the two
        // keyframes and then stops costing anything -- which is the whole reason
        // the cadence exists, and it is only observable because the walk did not
        // abort at the first refusal.
        assert_eq!(
            scan.unreconstructed_record_count,
            CADENCE,
            "expected exactly the hours between the lost keyframe and the cadence one"
        );
        // kills: a cadence that never fires, leaving the domain refused forever.
        assert_eq!((HOURS as u64) - 1 - scan.unreconstructed_record_count, 1);
        // kills: a walk that stopped early and merely reported fewer refusals.
        // Every remaining byte must have been decoded.
        assert_eq!(scan.decoded_stream_bytes, damaged.len() as u64);
        // kills: counting refusals without naming them.
        assert_eq!(
            scan.unreconstructed_records.len(),
            MAX_NAMED_RECONSTRUCTION_REFUSALS.min(CADENCE as usize)
        );
        assert!(
            scan.unreconstructed_records[0].contains("domain 53")
                && scan.unreconstructed_records[0].contains(&FIRST_HOUR.saturating_add(1).to_string()),
            "a refusal must name the domain and the hour it cost: {}",
            scan.unreconstructed_records[0]
        );
    }

    #[test]
    fn a_wrapped_d47_stream_yields_the_same_advisor_tokens_as_the_raw_one() {
        // kills: `scan_stream` skipping reconstruction for a newly enabled
        // domain (lane QL put 47 on `DELTA_ENABLED_DOMAINS`). d47 is NOT d53:
        // it has a token-collecting arm in `collect_static_join_tokens`
        // (`collect_domain47_advisor_tokens`), so its wrapped records must not
        // merely avoid breaking the walk -- they must contribute exactly the
        // tokens their raw bytes contribute. An earlier wording of this file's
        // inventory comment claimed no enabled domain collects tokens; that was
        // true for {43, 53} and false from the hour 47 joined.
        //
        // The vacuity controls are the point, as in the d53 test above: the
        // token set is asserted NON-EMPTY with a named member (an empty-set
        // equality proves nothing), the raw and wrapped streams are asserted to
        // be different bytes, and the unreconstructed envelope is asserted to
        // fail d47 grammar outright -- so the equality below can only pass if
        // the walker really did reconstruct.
        use characters_leaders_cabinet_live_lane as domain47;
        use compact_contract::payload_span_delta as psd;

        const DOMAIN47: u16 = 47;
        const ADVISOR_IDEA_TOKEN: &str = "POR_political_advisor_idea";

        let advisor_payload = |elapsed_wall_ms: u64| -> Vec<u8> {
            let rows = vec![
                domain47::CharacterLaneRow::Character(domain47::CharacterRow {
                    country_tag_db_id: 47,
                    char_id: 1001,
                    name: Some("Antonio de Oliveira Salazar"),
                    token: Some("POR_salazar"),
                }),
                domain47::CharacterLaneRow::AdvisorRoster(domain47::AdvisorRow {
                    country_tag_db_id: 47,
                    char_id: 1001,
                    slot: Some("political_advisor"),
                    ledger: Some("political_advisor"),
                    idea_token: ADVISOR_IDEA_TOKEN,
                }),
            ];
            domain47::encode_payload(elapsed_wall_ms, 0, domain47::SCHEMA_ROOTS_VALIDATED, &rows)
        };

        let mut producer = psd::PayloadSpanDeltaProducer::new();
        let mut raw = Vec::new();
        let mut wrapped = Vec::new();
        let mut envelopes = Vec::new();
        let mut sequence = 1u64;
        for hour in 0..3i32 {
            // Same engine content every hour, a different recorder clock: the
            // exact shape that makes a NoChange body correct and makes a
            // reconstruction that took ANY byte from the wrong place visible.
            let payload = advisor_payload(1_000 + hour as u64 * 1_000);
            append_record(
                &mut raw,
                sequence,
                FIRST_HOUR + hour,
                DOMAIN47,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
            let envelope = producer
                .wrap(
                    psd::PayloadSpanDeltaIdentity {
                        domain_id: DOMAIN47,
                        source_sequence: sequence,
                        game_hour_raw: FIRST_HOUR + hour,
                    },
                    &payload,
                )
                .expect("wrap a valid d47 payload");
            append_record(
                &mut wrapped,
                sequence,
                FIRST_HOUR + hour,
                DOMAIN47,
                compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA,
                compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                &envelope,
            );
            envelopes.push(envelope);
            producer.commit();
            sequence += 1;
        }
        assert_eq!(envelopes[0].len(), psd::HEADER_LEN + advisor_payload(0).len() - 8);
        assert_eq!(envelopes[1].len(), psd::HEADER_LEN, "hour 2 is a NoChange body");
        assert_eq!(envelopes[2].len(), psd::HEADER_LEN, "hour 3 is a NoChange body");

        let scratch = ScratchDir::new("ql_static_join_d47");
        let raw_path = scratch.write("raw.bin", &raw);
        let wrapped_path = scratch.write("wrapped.bin", &wrapped);
        let raw_scan = scan_stream(&raw_path, StreamIdentityMode::Legacy).expect("raw stream scans");
        let wrapped_scan = scan_stream(&wrapped_path, StreamIdentityMode::Legacy)
            .expect("wrapped stream scans -- an Err here means the walker did not unwrap");

        let expected = BTreeSet::from([StaticJoinToken {
            kind: "idea",
            token: ADVISOR_IDEA_TOKEN.to_string(),
        }]);
        assert_eq!(raw_scan.tokens, expected, "the fixture must actually collect the advisor token");
        assert_eq!(
            raw_scan.tokens, wrapped_scan.tokens,
            "the static-join token set must not depend on the wire encoding"
        );
        assert_eq!(wrapped_scan.unreconstructed_record_count, 0);
        assert_ne!(raw, wrapped);

        let unreconstructed = collect_static_join_tokens(
            DOMAIN47,
            &envelopes[0],
            DomainDataContext {
                stream_offset: 0,
                record_flags: compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
                sequence: 1,
                header_hour_raw: FIRST_HOUR,
                session_header_identity: None,
                xmap07_snow_mud_previous_grid_sha256: None,
            },
        );
        assert!(
            unreconstructed.is_err(),
            "envelope bytes parsed as d47 grammar, so this test could not tell \
             a reconstructing walker from one that skipped it"
        );
    }
}
