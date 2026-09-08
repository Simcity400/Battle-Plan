//! The generic DOMAIN_DATA_REPEAT change gate (record-framing layer).
//!
//! When an hour's DOMAIN_DATA payload, minus the domain's DECLARED volatile
//! prefix ([`compact_contract::domain_declared_volatile_prefix_len`]), is
//! byte-identical to the previous PUBLISHED payload for that domain, the
//! collector emits a `DOMAIN_DATA_REPEAT` record instead: the fresh volatile
//! prefix plus a SHA-256 digest of the unchanged tail. The fold reconstructs
//! by overwriting its stored baseline's prefix and refuses by name unless the
//! digest verifies.
//!
//! Placement laws this module is written against:
//!
//! * CAPTURE-ONLY HOT PATH â€” the comparison runs at publication time, after
//!   every reader clock bracket has closed. The read still happens every
//!   hour; only the WRITE shrinks. Zero in-bracket cost is added.
//! * PUBLISHED-BASELINE SEMANTICS â€” slots advance from the FINAL frame bytes
//!   at the moment they are staged for the sink, never from an in-progress
//!   build, so "previous published payload" means exactly what the fold can
//!   already see on the stream.
//! * FORCE FULL AFTER ANY GAP â€” every scheduler gap range carries
//!   `force_next_baseline`; [`Self::note_gap_domain`] is hooked where those
//!   ranges are staged, and any H4DG record inside a rewritten frame marks
//!   its domain too. The keyframe cadence
//!   ([`compact_contract::COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS`])
//!   forces a full record even with no gap, so a consumer that lost a record
//!   resyncs.
//! * COMPOSITION â€” domains with their own delta encoders (d100 XSM2 and the
//!   later d53/d41/d43 lanes) keep them untouched; this gate sits in FRONT of
//!   them at the framing layer, comparing final wire payloads. A gated domain
//!   simply rarely repeats; nothing about their baselines changes.
//!   D20 bypasses this layer: its semantic delta protocol already owns
//!   unchanged-state records and recovery baselines, so retaining its encoded
//!   payload here would duplicate that state.

use sha2::{Digest, Sha256};

use compact_contract::{
    domain_data_repeat_payload_len, domain_declared_volatile_prefix_len,
    COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
};

/// D20 owns semantic deltas and must not retain a second encoded baseline.
/// The remaining exclusions never carry data records: 0 is reserved, 1000 is
/// the frame envelope, and 250 is the RC attribution probe (PERF-only).
const fn gateable_domain(domain_id: u16) -> bool {
    domain_id != 0
        && domain_id != 20
        && domain_id != 250
        && domain_id != compact_contract::COMPACT_DOMAIN_FRAME
}

const REPEAT_GATE_DOMAIN_SLOTS: usize = 256;

#[derive(Default)]
pub struct RepeatGateFrameStats {
    /// DOMAIN_DATA records examined in the frame.
    pub data_records_considered: u32,
    /// Records rewritten from DOMAIN_DATA to DOMAIN_DATA_REPEAT.
    pub records_rewritten: u32,
    /// Sum of the replaced records' payload lengths.
    pub payload_bytes_before: u64,
    /// Sum of the repeat payloads that replaced them.
    pub payload_bytes_after: u64,
    /// Domains whose baseline slot this pass changed (deduplicated, in first
    /// change order). A caller whose publication outcome can put the frame
    /// BACK â€” the legacy transport's uncertain-boundary retry rebuilds the
    /// frame from scratch â€” forces these domains full next hour so both sides
    /// reconverge on a published full record instead of repeating against
    /// bytes the fold never saw.
    pub decided_domains: Vec<u16>,
}

impl RepeatGateFrameStats {
    pub fn bytes_saved(&self) -> u64 {
        self.payload_bytes_before
            .saturating_sub(self.payload_bytes_after)
    }
}

fn slot(domain_id: u16) -> usize {
    (domain_id as usize).min(REPEAT_GATE_DOMAIN_SLOTS - 1)
}

/// Per-domain repeat-gate state for one capture session.
///
/// `slots[d]` holds the last published full-or-reconstructed payload for
/// domain `d` â€” the exact bytes the fold's own baseline store holds after
/// ingesting the same stream. `last_full_hour_index[d]` anchors the keyframe
/// cadence in elapsed game hours; `force_full[d]` is set by every gap
/// mechanism and consumed by the next published record for that domain.
pub struct DomainDataRepeatGate {
    slots: Box<[Option<Vec<u8>>]>,
    /// Game-hour index of each domain's last published FULL record. The
    /// keyframe cadence is measured in ELAPSED GAME HOURS, not in record
    /// count: a snapshot step coarser than one hour must not stretch the
    /// cadence, and two records for one domain inside one hour must not
    /// advance it.
    last_full_hour_index: Box<[Option<i64>]>,
    force_full: Box<[bool]>,
}

impl DomainDataRepeatGate {
    pub fn new() -> Self {
        Self {
            slots: (0..REPEAT_GATE_DOMAIN_SLOTS).map(|_| None).collect::<Vec<_>>().into_boxed_slice(),
            last_full_hour_index: (0..REPEAT_GATE_DOMAIN_SLOTS)
                .map(|_| None)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            force_full: vec![false; REPEAT_GATE_DOMAIN_SLOTS].into_boxed_slice(),
        }
    }

    /// Drop every baseline: used when the logical stream changes (campaign
    /// rollover / reload boundary), because the successor stream starts a new
    /// fold whose slots begin empty.
    pub fn reset(&mut self) {
        for slot in self.slots.iter_mut() {
            *slot = None;
        }
        for anchor in self.last_full_hour_index.iter_mut() {
            *anchor = None;
        }
        for flag in self.force_full.iter_mut() {
            *flag = false;
        }
    }

    /// A scheduler gap range was staged for `domain_id`: whatever the next
    /// published record for that domain is, it must be a FULL record. The
    /// scheduler sets `force_next_baseline` on every range it mints, so this
    /// is called for each of them.
    pub fn note_gap_domain(&mut self, domain_id: u16) {
        if gateable_domain(domain_id) {
            self.force_full[slot(domain_id)] = true;
        }
    }

    /// A whole FRAME_REFUSED hour is a gap for the ENTIRE scheduled roster:
    /// every gateable domain must publish a full record next. Used by the
    /// refusal publication path (roster known there) and by offline tools
    /// that do not carry the roster.
    pub fn force_full_all(&mut self) {
        for flag in self.force_full.iter_mut() {
            *flag = true;
        }
    }

    /// Rewrite one frame's records in place immediately before publication,
    /// advancing the per-domain slots to exactly the bytes this frame puts on
    /// the wire.
    ///
    /// The frame must be FINAL: every post-build insertion pass has run, so
    /// what lands here is byte-for-byte what reaches the sink. Records only
    /// ever shrink, so a single forward compaction pass cannot overwrite
    /// unread input.
    ///
    /// The sequence numbers, hours, domains, flags and PERF/FRAME_END records
    /// are untouched: a rewritten record differs from its source only in
    /// record type and payload.
    pub fn rewrite_frame_for_publication(
        &mut self,
        records: &mut Vec<u8>,
    ) -> Result<RepeatGateFrameStats, String> {
        let mut stats = RepeatGateFrameStats::default();
        let mut read = 0usize;
        let mut write = 0usize;
        while read < records.len() {
            let header = compact_contract::parse_compact_record_header(&records[read..])
                .map_err(|error| format!("repeat gate parses compact record header: {error}"))?;
            let header_len = usize::from(COMPACT_RECORD_HEADER_LEN);
            let total = usize::try_from(header.record_len())
                .map_err(|_| "repeat gate compact record length overflow".to_string())?;
            if total < header_len || read + total > records.len() {
                return Err(format!(
                    "repeat gate found a truncated or oversized record at offset {read}"
                ));
            }
            let domain_id = header.domain_id;
            if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_GAP && gateable_domain(domain_id)
            {
                // An in-frame gap replacement (rejected bracket, silent
                // reader) is the same claim as a scheduler gap range: the
                // fold sees no data for this domain this hour, so the next
                // record must restate a full baseline.
                self.note_gap_domain(domain_id);
            }
            if header.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
                records.copy_within(read..read + total, write);
                write += total;
                read += total;
                continue;
            }
            stats.data_records_considered = stats.data_records_considered.saturating_add(1);
            let index = slot(domain_id);
            // The cadence is ELAPSED GAME HOURS since this domain's last full
            // record. A missing anchor, a force-full mark, or an hour index
            // that regresses (reload/rollback) all publish FULL; only a
            // monotonic advance under the cadence may repeat.
            let hour_index =
                compact_contract::calendar::hour_index_from_raw(header.game_hour_raw);
            let gateable = gateable_domain(domain_id)
                && !self.force_full[index]
                && match (self.last_full_hour_index[index], hour_index) {
                    (Some(last), Some(current)) => {
                        current >= last && current - last < i64::from(
                            compact_contract::COMPACT_DOMAIN_DATA_REPEAT_KEYFRAME_CADENCE_HOURS,
                        )
                    }
                    _ => false,
                };
            let mut emitted_repeat = false;
            if gateable {
                let span = domain_declared_volatile_prefix_len(domain_id);
                let payload_start = read + header_len;
                let current = &records[payload_start..read + total];
                // A repeat only pays when the unchanged tail is LONGER than
                // the digest that vouches for it; below that bound the
                // rewritten record would be the same size or bigger.
                if let Some(stored) = self.slots[index].as_ref() {
                    if current.len() > span + compact_contract::COMPACT_DOMAIN_DATA_REPEAT_DIGEST_LEN
                        && stored.len() == current.len()
                        && stored.len() > span
                        && current[span..] == stored[span..]
                    {
                        let digest = Sha256::digest(&stored[span..]);
                        let repeat_payload_len = domain_data_repeat_payload_len(domain_id);
                        let repeat_total = header_len + repeat_payload_len;
                        debug_assert!(repeat_total <= total, "a repeat record never grows");
                        // Lift the source bytes out first: the output region
                        // [write..write+repeat_total] may overlap the current
                        // record's own bytes when nothing has been compacted
                        // yet (`write == read`).
                        let mut source_header = [0u8; COMPACT_RECORD_HEADER_LEN];
                        source_header.copy_from_slice(&records[read..read + header_len]);
                        // Lifted too because the write below can clobber this
                        // record's own bytes when nothing has been compacted
                        // yet (`write == read`).
                        let fresh_prefix = current[..span].to_vec();
                        let new_slot = current.to_vec();
                        let current_len = current.len();
                        // `write <= read` and `repeat_total <= total`, so the
                        // output never reaches past this record's end.
                        let (front, _rest) = records.split_at_mut(read + total);
                        let out = &mut front[write..write + repeat_total];
                        // Header: identical fields except record type and
                        // payload length. The flag word is copied from the
                        // replaced DATA record, which is exactly the word
                        // contract validation admits for the domain.
                        out[..header_len].copy_from_slice(&source_header);
                        // magic/version/header-len/sequence/hour/domain are
                        // copied verbatim above; record type sits at 22..24
                        // and the declared payload length at 24..28.
                        out[22..24].copy_from_slice(
                            &(COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT.to_le_bytes()),
                        );
                        out[24..28].copy_from_slice(&(repeat_payload_len as u32).to_le_bytes());
                        out[header_len..header_len + span].copy_from_slice(&fresh_prefix);
                        out[header_len + span..header_len + repeat_payload_len]
                            .copy_from_slice(&digest);
                        // The slot advances to THIS hour's payload: the fresh
                        // prefix over the verified tail.
                        let updated = self.slots[index].as_mut().expect("checked above");
                        updated.clear();
                        updated.extend_from_slice(&new_slot);
                        if !stats.decided_domains.contains(&domain_id) {
                            stats.decided_domains.push(domain_id);
                        }
                        stats.records_rewritten =
                            stats.records_rewritten.saturating_add(1);
                        stats.payload_bytes_before =
                            stats.payload_bytes_before.saturating_add(current_len as u64);
                        stats.payload_bytes_after = stats
                            .payload_bytes_after
                            .saturating_add(repeat_payload_len as u64);
                        emitted_repeat = true;
                    }
                }
            }
            if emitted_repeat {
                write += header_len + domain_data_repeat_payload_len(domain_id);
                read += total;
                continue;
            }
            // Full record: publish as-is, rebase the slot and cadence, and
            // consume any pending force-full mark.
            if gateable_domain(domain_id) {
                let payload = &records[read + header_len..read + total];
                match self.slots[index].as_mut() {
                    Some(stored) => {
                        stored.clear();
                        stored.extend_from_slice(payload);
                    }
                    None => self.slots[index] = Some(payload.to_vec()),
                }
                self.last_full_hour_index[index] = hour_index;
                self.force_full[index] = false;
                if !stats.decided_domains.contains(&domain_id) {
                    stats.decided_domains.push(domain_id);
                }
            }
            records.copy_within(read..read + total, write);
            write += total;
            read += total;
        }
        records.truncate(write);
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compact_contract::COMPACT_RECORD_MAGIC;

    // A VALID raw game hour: hour_index_from_raw admits only raws inside the
    // calendar range, whose floor is the epoch plus 1850 years of hours.
    const HOUR: i32 =
        compact_contract::calendar::DATE_EPOCH_CONST + (1850 * 365 * 24);

    fn append_data_record(out: &mut Vec<u8>, sequence: u64, hour_raw: i32, domain_id: u16, payload: &[u8]) {
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(
            &(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
        );
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_TYPE_DOMAIN_DATA).to_le_bytes());
        out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn append_gap_record(out: &mut Vec<u8>, sequence: u64, hour_raw: i32, domain_id: u16) {
        let payload = vec![0u8; compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN];
        out.extend_from_slice(COMPACT_RECORD_MAGIC);
        out.extend_from_slice(&compact_contract::COMPACT_RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(
            &(compact_contract::COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes(),
        );
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&(COMPACT_RECORD_TYPE_DOMAIN_GAP).to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&payload);
    }

    fn walk<'a>(records: &'a [u8]) -> Vec<(compact_contract::CompactRecordHeader, &'a [u8])> {
        let mut out = Vec::new();
        let mut off = 0usize;
        while off < records.len() {
            let header = compact_contract::parse_compact_record_header(&records[off..]).unwrap();
            let end = off + header.record_len() as usize;
            out.push((
                header,
                &records[off + usize::from(compact_contract::COMPACT_RECORD_HEADER_LEN)..end],
            ));
            off = end;
        }
        out
    }

    fn payload_for(tag: u8, elapsed: u64, body: &[u8]) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&elapsed.to_le_bytes());
        p.push(tag);
        // Long enough that the unchanged tail outlives the digest that
        // vouches for it â€” the gate only pays above that bound.
        let mut padded = Vec::from(body);
        padded.resize(64, b'.');
        p.extend_from_slice(&padded);
        p
    }

    /// kills: a gate that rewrote nothing would leave both records full; one
    /// that rewrote against a stale slot would emit a digest over the wrong
    /// tail and fail the round trip below.
    #[test]
    fn an_unchanged_tail_is_rewritten_with_a_fresh_prefix_and_verified_digest() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 10, &payload_for(7, 100, b"body"));
        append_data_record(&mut frame, 2, HOUR + (2 as i32), 10, &payload_for(7, 200, b"body"));
        let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
        assert_eq!(stats.data_records_considered, 2);
        assert_eq!(stats.records_rewritten, 1);
        assert_eq!(
            stats.bytes_saved(),
            (payload_for(7, 100, b"body").len() - domain_data_repeat_payload_len(10)) as u64
        );

        let walked = walk(&frame);
        assert_eq!(walked.len(), 2);
        assert_eq!(walked[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
        assert_eq!(
            walked[1].0.record_type,
            COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        );
        let repeat_payload = walked[1].1;
        let span = domain_declared_volatile_prefix_len(10);
        assert_eq!(repeat_payload.len(), span + 32);
        assert_eq!(&repeat_payload[..span], &200u64.to_le_bytes());
        // The digest names the tail the fold will hold.
        let expected = Sha256::digest(&payload_for(7, 100, b"body")[span..]);
        assert_eq!(&repeat_payload[span..], expected.as_slice());
    }

    #[test]
    fn division_full_and_repeat_candidates_never_store_a_generic_baseline() {
        let mut gate = DomainDataRepeatGate::new();
        for hour in 1..=3 {
            let mut frame = Vec::new();
            append_data_record(
                &mut frame,
                hour as u64,
                HOUR + hour,
                20,
                &payload_for(7, hour as u64, b"unchanged division membership"),
            );
            let original = frame.clone();
            let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
            assert_eq!(frame, original, "D20 record bytes at hour {hour}");
            assert_eq!(stats.data_records_considered, 1);
            assert_eq!(stats.records_rewritten, 0);
            assert_eq!(stats.payload_bytes_before, 0);
            assert_eq!(stats.payload_bytes_after, 0);
            assert!(stats.decided_domains.is_empty());
            assert!(gate.slots[slot(20)].is_none());
            assert!(gate.last_full_hour_index[slot(20)].is_none());
        }
    }

    #[test]
    fn division_semantic_delta_and_gap_recovery_records_pass_through() {
        let mut gate = DomainDataRepeatGate::new();
        // This framing fixture keeps D20's status at payload +16. Its owned
        // grammar is opaque to the repeat gate, including a rowless delta.
        let mut full_payload = vec![0u8; 32];
        full_payload.extend_from_slice(&[0x5a; 128]);
        let mut empty_delta_payload = vec![0u8; 32];
        empty_delta_payload[16..20]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());
        let mut first = Vec::new();
        append_data_record(&mut first, 1, HOUR + 1, 20, &full_payload);
        append_data_record(&mut first, 2, HOUR + 2, 20, &empty_delta_payload);
        let original = first.clone();
        let stats = gate.rewrite_frame_for_publication(&mut first).unwrap();
        assert_eq!(first, original);
        assert_eq!(stats.records_rewritten, 0);
        assert!(stats.decided_domains.is_empty());
        assert!(gate.slots[slot(20)].is_none());

        gate.note_gap_domain(20);
        assert!(!gate.force_full[slot(20)]);
        let mut gap = Vec::new();
        append_gap_record(&mut gap, 3, HOUR + 3, 20);
        let original_gap = gap.clone();
        gate.rewrite_frame_for_publication(&mut gap).unwrap();
        assert_eq!(gap, original_gap);
        assert!(!gate.force_full[slot(20)]);

        // Whole-frame refusal and cadence expiry must also leave the
        // producer's recovery full record and subsequent delta intact.
        gate.force_full_all();
        let mut recovery = Vec::new();
        append_data_record(&mut recovery, 4, HOUR + 65, 20, &full_payload);
        append_data_record(&mut recovery, 5, HOUR + 66, 20, &empty_delta_payload);
        let original_recovery = recovery.clone();
        let stats = gate.rewrite_frame_for_publication(&mut recovery).unwrap();
        assert_eq!(recovery, original_recovery);
        assert_eq!(stats.records_rewritten, 0);
        assert!(stats.decided_domains.is_empty());
        assert!(gate.slots[slot(20)].is_none());
        assert!(gate.last_full_hour_index[slot(20)].is_none());
    }

    #[test]
    fn division_records_survive_other_domain_repeat_compaction() {
        let mut gate = DomainDataRepeatGate::new();
        let other_payload = payload_for(3, 1, b"fuel state");
        let division_payload = payload_for(7, 1, b"division membership");
        let mut baseline = Vec::new();
        append_data_record(&mut baseline, 1, HOUR + 1, 10, &other_payload);
        append_data_record(&mut baseline, 2, HOUR + 1, 20, &division_payload);
        gate.rewrite_frame_for_publication(&mut baseline).unwrap();

        let mut first_division = Vec::new();
        append_data_record(&mut first_division, 4, HOUR + 2, 20, &division_payload);
        let mut second_division = Vec::new();
        append_data_record(&mut second_division, 6, HOUR + 2, 20, &division_payload);
        let mut frame = Vec::new();
        append_data_record(&mut frame, 3, HOUR + 2, 10, &other_payload);
        frame.extend_from_slice(&first_division);
        append_data_record(&mut frame, 5, HOUR + 2, 10, &other_payload);
        frame.extend_from_slice(&second_division);

        let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
        assert_eq!(stats.data_records_considered, 4);
        assert_eq!(stats.records_rewritten, 2);
        assert_eq!(stats.decided_domains, vec![10]);
        let walked = walk(&frame);
        assert_eq!(walked.len(), 4);
        assert_eq!(walked[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT);
        assert_eq!(walked[2].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT);
        let first_start = walked[0].0.record_len() as usize;
        assert_eq!(&frame[first_start..first_start + first_division.len()], first_division);
        let second_start = first_start + first_division.len() + walked[2].0.record_len() as usize;
        assert_eq!(&frame[second_start..], second_division);
        assert!(gate.slots[slot(20)].is_none());
        assert!(gate.last_full_hour_index[slot(20)].is_none());
        assert_eq!(gate.slots[slot(10)].as_deref(), Some(other_payload.as_slice()));
    }

    /// kills: a changed tail must never be rewritten, and the full record
    /// must rebase the slot so a LATER unchanged hour gates again.
    #[test]
    fn a_changed_tail_publishes_full_and_rebases_the_slot() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 10, &payload_for(7, 100, b"body"));
        append_data_record(&mut frame, 2, HOUR + (2 as i32), 10, &payload_for(7, 101, b"CHANGED"));
        append_data_record(&mut frame, 3, HOUR + (3 as i32), 10, &payload_for(7, 102, b"CHANGED"));
        let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
        assert_eq!(stats.records_rewritten, 1, "only the third hour repeats");
        let walked = walk(&frame);
        assert_eq!(walked[1].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
        assert_eq!(
            walked[2].0.record_type,
            COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT
        );
    }

    /// kills: dropping the cadence check would let a consumer that lost a
    /// record stay desynchronised forever.
    #[test]
    fn the_keyframe_cadence_forces_a_full_record_every_sixty_four_hours() {
        let mut gate = DomainDataRepeatGate::new();
        let mut first = Vec::new();
        append_data_record(&mut first, 1, HOUR + (1 as i32), 10, &payload_for(9, 0, b"static-state"));
        gate.rewrite_frame_for_publication(&mut first).unwrap();
        // Full record anchors the cadence at hour index 1. Hours 2..=64 are
        // deltas 1..=63, all under the 64-hour bound: every one repeats.
        for hour in 2..=64u64 {
            let mut frame = Vec::new();
            append_data_record(&mut frame, hour, HOUR + (hour as i32), 10, &payload_for(9, hour, b"static-state"));
            let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
            assert_eq!(stats.records_rewritten, 1, "hour {hour} should repeat");
        }
        // Hour index 65 is delta 64 — the cadence expires and the full
        // keyframe is forced.
        let mut forced = Vec::new();
        append_data_record(
            &mut forced,
            65,
            HOUR + 65,
            10,
            &payload_for(9, 65, b"static-state"),
        );
        let stats = gate.rewrite_frame_for_publication(&mut forced).unwrap();
        assert_eq!(
            stats.records_rewritten, 0,
            "the sixty-fourth consecutive repeat hour publishes a full record"
        );
        let walked = walk(&forced);
        assert_eq!(walked[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
    }

    /// kills: a gap followed by a repeat would hand the fold a reconstruction
    /// across a discontinuity the owner ruling forbids.
    #[test]
    fn a_scheduler_gap_range_forces_the_next_record_full() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 10, &payload_for(3, 0, b"pre-gap"));
        gate.rewrite_frame_for_publication(&mut frame).unwrap();

        gate.note_gap_domain(10);
        let mut after = Vec::new();
        append_data_record(&mut after, 2, HOUR + (2 as i32), 10, &payload_for(3, 99, b"pre-gap"));
        let stats = gate.rewrite_frame_for_publication(&mut after).unwrap();
        assert_eq!(stats.records_rewritten, 0, "gap forces the full record");
        let walked = walk(&after);
        assert_eq!(walked[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);

        // And the mark is consumed: the following hour may repeat again.
        let mut later = Vec::new();
        append_data_record(&mut later, 3, HOUR + (3 as i32), 10, &payload_for(3, 100, b"pre-gap"));
        let stats = gate.rewrite_frame_for_publication(&mut later).unwrap();
        assert_eq!(stats.records_rewritten, 1);
    }

    /// kills: an H4DG record inside the published frame must mark its domain
    /// just like a scheduler gap range does.
    #[test]
    fn an_in_frame_gap_record_marks_its_domain() {
        let mut gate = DomainDataRepeatGate::new();
        let mut first = Vec::new();
        append_data_record(&mut first, 1, HOUR + (1 as i32), 43, &payload_for(1, 0, b"market"));
        gate.rewrite_frame_for_publication(&mut first).unwrap();

        let mut gapped = Vec::new();
        append_gap_record(&mut gapped, 2, HOUR + (2 as i32), 43);
        gate.rewrite_frame_for_publication(&mut gapped).unwrap();

        let mut after = Vec::new();
        append_data_record(&mut after, 3, HOUR + (3 as i32), 43, &payload_for(1, 50, b"market"));
        let stats = gate.rewrite_frame_for_publication(&mut after).unwrap();
        assert_eq!(stats.records_rewritten, 0, "the gap forced a full record");
    }

    /// kills: a length change between hours must not gate (equal tails are
    /// impossible across different lengths), and a tiny payload no longer
    /// than its declared prefix must never gate either.
    #[test]
    fn length_changes_and_tiny_payloads_never_gate() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 10, &payload_for(7, 100, b"one"));
        append_data_record(&mut frame, 2, HOUR + (2 as i32), 10, &payload_for(7, 100, b"grown-body"));
        append_data_record(&mut frame, 3, HOUR + (3 as i32), 10, &100u64.to_le_bytes());
        let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
        assert_eq!(stats.records_rewritten, 0);

        // A payload EXACTLY the prefix long has no tail to vouch for.
        let mut tiny = Vec::new();
        append_data_record(&mut tiny, 4, HOUR + (4 as i32), 10, &100u64.to_le_bytes());
        let stats = gate.rewrite_frame_for_publication(&mut tiny).unwrap();
        assert_eq!(stats.records_rewritten, 0);
    }

    /// kills: a multi-record hour (d53 snapshot + controller carriers) must
    /// leave both records decodable and the slot on the LAST published
    /// payload, mirroring the fold's single-slot rule.
    #[test]
    fn multiple_records_for_one_domain_follow_the_single_slot_rule() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 53, &payload_for(0xAA, 0, b"snapshot-n1"));
        append_data_record(&mut frame, 2, HOUR + (2 as i32), 53, &payload_for(0xBB, 0, b"controller-n1"));
        let stats = gate.rewrite_frame_for_publication(&mut frame).unwrap();
        assert_eq!(stats.records_rewritten, 0, "first hour: all full");
        let walked = walk(&frame);
        assert_eq!(walked.len(), 2);

        let mut next_hour = Vec::new();
        append_data_record(
            &mut next_hour,
            3,
            HOUR + 2,
            53,
            &payload_for(0xAA, 3600, b"snapshot-n1"),
        );
        append_data_record(
            &mut next_hour,
            4,
            HOUR + 2,
            53,
            &payload_for(0xBB, 3600, b"controller-n1"),
        );
        // The snapshot compares against last hour's CONTROLLER (single slot),
        // so it stays full; the controller then matches the snapshot's slotâ€¦
        // which also differs. Nothing gates this hour; the NEXT hour can
        // repeat whichever carrier landed in the slot last.
        let stats = gate.rewrite_frame_for_publication(&mut next_hour).unwrap();
        assert_eq!(stats.records_rewritten, 0);
        let walked = walk(&next_hour);
        assert_eq!(walked[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
        assert_eq!(walked[1].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);

        let mut third = Vec::new();
        append_data_record(
            &mut third,
            5,
            HOUR + 3,
            53,
            &payload_for(0xBB, 7200, b"controller-n1"),
        );
        let stats = gate.rewrite_frame_for_publication(&mut third).unwrap();
        assert_eq!(stats.records_rewritten, 1);
    }

    /// kills: a reset that left slots behind would put repeats referencing
    /// the old stream's payloads into the successor stream, which its fold
    /// cannot verify.
    #[test]
    fn reset_clears_slots_cadence_and_force_marks() {
        let mut gate = DomainDataRepeatGate::new();
        let mut frame = Vec::new();
        append_data_record(&mut frame, 1, HOUR + (1 as i32), 10, &payload_for(7, 0, b"old-stream"));
        gate.rewrite_frame_for_publication(&mut frame).unwrap();
        gate.note_gap_domain(11);
        gate.reset();
        let mut fresh = Vec::new();
        append_data_record(&mut fresh, 2, HOUR + (2 as i32), 10, &payload_for(7, 1, b"old-stream"));
        let stats = gate.rewrite_frame_for_publication(&mut fresh).unwrap();
        assert_eq!(stats.records_rewritten, 0, "no baseline survives a reset");
    }
}
