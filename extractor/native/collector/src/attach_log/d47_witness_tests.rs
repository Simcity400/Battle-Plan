//! d47 discovery witness (lane MO): the raw-byte record of one adopted
//! discovery, replayed per frame instead of re-censusing.
//!
//! What every test here asks is "what production change would make this
//! fail?": a seal that merges disagreeing overlaps, a replay that passes a
//! changed byte, a cluster that swallows a refused read, or an eligibility
//! rule that replays a poisoned cache would each turn a byte the engine no
//! longer holds into a published row.

use super::*;
use std::collections::HashMap;

fn record(addr: usize, bytes: &[u8]) -> (usize, Option<Vec<u8>>) {
    (addr, Some(bytes.to_vec()))
}

fn log_of(reads: &[(usize, Option<Vec<u8>>)], failed_len: usize) -> Domain47WitnessLog {
    let mut log = Domain47WitnessLog::default();
    for (addr, bytes) in reads {
        match bytes {
            Some(bytes) => log.note(*addr, bytes.len(), Some(bytes)),
            None => log.note(*addr, failed_len, None),
        }
    }
    log
}

fn manager_snapshot() -> Domain47ManagerRosterSnapshot {
    let vector = |data: usize, entries: Vec<usize>| Domain47ManagerVectorSnapshot {
        data,
        capacity: entries.len() as u32,
        size: entries.len() as u32,
        entries,
    };
    Domain47ManagerRosterSnapshot {
        characters: Vec::new(),
        historical_count: 2,
        dynamic_count: 1,
        name_index_count: 3,
        distinct_name_token_count: 3,
        manager_root: 0x1000,
        manager: 0x2000,
        manager_vtable: 0x1402_92F0_00,
        historical: vector(0x3000, vec![0x10_0000, 0x10_0100]),
        dynamic: vector(0x4000, vec![0x10_0200]),
    }
}

fn seal(log: &Domain47WitnessLog) -> Result<Domain47DiscoveryWitness, Domain47WitnessSealRefusal> {
    domain47_witness_seal(log, 0xAAAA, 0x1_4000_0000, &manager_snapshot())
}

/// A memory the replay reads: address -> byte, plus a set of ranges that
/// refuse (a region boundary, an unreadable page).
#[derive(Default)]
struct FakeMemory {
    bytes: HashMap<usize, u8>,
    refuse: Vec<(usize, usize)>,
}

impl FakeMemory {
    fn from_log(log: &Domain47WitnessLog) -> Self {
        let mut mem = Self::default();
        for record in &log.records {
            if record.off == DOMAIN47_WITNESS_FAILED {
                mem.refuse.push((record.addr, record.len as usize));
                continue;
            }
            let src = &log.bytes[record.off as usize..record.off as usize + record.len as usize];
            for (i, byte) in src.iter().enumerate() {
                mem.bytes.insert(record.addr + i, *byte);
            }
        }
        mem
    }
    fn fill(&self, addr: usize, buf: &mut [u8]) -> bool {
        let end = addr + buf.len();
        if self
            .refuse
            .iter()
            .any(|(start, len)| addr < start + len && *start < end)
        {
            return false;
        }
        for (i, slot) in buf.iter_mut().enumerate() {
            match self.bytes.get(&(addr + i)) {
                Some(byte) => *slot = *byte,
                // A byte nobody recorded is still readable memory: zero, as a
                // gap between two pieces of one cluster would be.
                None => *slot = 0,
            }
        }
        true
    }
}

fn replay(witness: &Domain47DiscoveryWitness, mem: &FakeMemory) -> Result<Domain47WitnessReplayStats, Domain47WitnessMiss> {
    let mut scratch = Vec::new();
    domain47_witness_replay(witness, &mut scratch, |addr, buf| mem.fill(addr, buf))
}

// kills: a seal that keeps the FIRST reading of an overlap and drops the
// second would return Ok here and cache a value one of discovery's own reads
// contradicted.
#[test]
fn seal_refuses_two_reads_of_one_byte_that_disagree() {
    let log = log_of(
        &[record(0x100, &[1, 2, 3, 4]), record(0x102, &[9, 4, 5])],
        0,
    );
    assert_eq!(seal(&log), Err(Domain47WitnessSealRefusal::OverlapDisagreed));
}

// kills: a merge that did not extend the piece over the tail of a later
// overlapping read would leave those bytes un-witnessed.
#[test]
fn seal_merges_agreeing_overlaps_and_adjacent_reads_into_one_piece() {
    let log = log_of(
        &[
            record(0x100, &[1, 2, 3, 4]),
            record(0x102, &[3, 4, 5]),
            record(0x105, &[6]),
            record(0x100, &[1, 2]),
            record(0x200, &[7]),
        ],
        0,
    );
    let witness = seal(&log).expect("sealed");
    assert_eq!(witness.pieces.len(), 2);
    assert_eq!((witness.pieces[0].addr, witness.pieces[0].len), (0x100, 6));
    assert_eq!(&witness.bytes[..6], &[1, 2, 3, 4, 5, 6]);
    assert_eq!((witness.pieces[1].addr, witness.pieces[1].len), (0x200, 1));
    assert_eq!(witness.raw_record_count, 5);
}

// kills: a cluster gap wider than DOMAIN47_WITNESS_CLUSTER_GAP being merged
// would read unbounded holes; a narrower one being split would cost a
// guarded copy per piece.
#[test]
fn seal_clusters_pieces_within_the_gap_and_splits_beyond_it() {
    let log = log_of(
        &[
            record(0x1000, &[1; 16]),
            record(0x1000 + 16 + DOMAIN47_WITNESS_CLUSTER_GAP, &[2; 8]),
            record(0x1000 + 16 + DOMAIN47_WITNESS_CLUSTER_GAP + 8 + DOMAIN47_WITNESS_CLUSTER_GAP + 1, &[3; 8]),
        ],
        0,
    );
    let witness = seal(&log).expect("sealed");
    assert_eq!(witness.spans.len(), 2);
    assert_eq!(witness.spans[0].piece_count, 2);
    assert_eq!(witness.spans[0].len as usize, 16 + DOMAIN47_WITNESS_CLUSTER_GAP + 8);
    assert_eq!(witness.spans[1].piece_count, 1);
}

// kills: a replay that compared only the span's first piece, or that skipped
// the failed-read probes, would pass a changed byte / a newly readable
// object through to the cached roots.
#[test]
fn replay_passes_identical_memory_and_refuses_one_changed_byte_in_any_piece() {
    let log = log_of(
        &[
            record(0x1000, &[1, 2, 3, 4]),
            record(0x1010, &[5, 6, 7, 8]),
            record(0x9000, &[9]),
            (0x7000, None),
        ],
        8,
    );
    let witness = seal(&log).expect("sealed");
    let mem = FakeMemory::from_log(&log);
    let stats = replay(&witness, &mem).expect("identical memory replays");
    assert_eq!(stats.spans, 2);
    assert_eq!(stats.pieces, 3);
    assert_eq!(stats.span_fallbacks, 0);

    let mut changed = FakeMemory::from_log(&log);
    changed.bytes.insert(0x1012, 0xEE);
    assert_eq!(replay(&witness, &changed), Err(Domain47WitnessMiss::SpanChanged(0)));

    let mut changed_last = FakeMemory::from_log(&log);
    changed_last.bytes.insert(0x9000, 0);
    assert_eq!(replay(&witness, &changed_last), Err(Domain47WitnessMiss::SpanChanged(1)));

    let mut now_readable = FakeMemory::from_log(&log);
    now_readable.refuse.clear();
    assert_eq!(
        replay(&witness, &now_readable),
        Err(Domain47WitnessMiss::FailedReadNowReadable(0))
    );
}

// kills: a cluster whose one-copy read refused (a region boundary in the gap)
// being reported as a miss would force a full discovery on every frame for a
// world whose objects straddle regions; a fallback that skipped the compare
// would pass a changed byte.
#[test]
fn replay_falls_back_to_pieces_when_the_cluster_copy_refuses() {
    let log = log_of(&[record(0x1000, &[1, 2]), record(0x1100, &[3, 4])], 0);
    let witness = seal(&log).expect("sealed");
    assert_eq!(witness.spans.len(), 1);
    let mut mem = FakeMemory::from_log(&log);
    // The gap between the two pieces is unreadable, the pieces are not.
    mem.refuse.push((0x1050, 0x10));
    let stats = replay(&witness, &mem).expect("pieces still compare");
    assert_eq!(stats.span_fallbacks, 1);
    mem.bytes.insert(0x1101, 0);
    assert_eq!(replay(&witness, &mem), Err(Domain47WitnessMiss::SpanChanged(0)));
    // A piece itself unreadable is a miss, not a pass.
    let mut piece_gone = FakeMemory::from_log(&log);
    piece_gone.refuse.push((0x1050, 0x10));
    piece_gone.refuse.push((0x1100, 2));
    assert_eq!(replay(&witness, &piece_gone), Err(Domain47WitnessMiss::SpanUnreadable(0)));
}

// kills: a log that silently truncated past its caps would seal a witness
// covering only part of what discovery decoded.
#[test]
fn an_overflowed_log_refuses_to_seal() {
    let mut log = Domain47WitnessLog::default();
    log.note(0x10, DOMAIN47_WITNESS_MAX_READ_LEN + 1, None);
    assert!(log.overflowed);
    assert_eq!(seal(&log), Err(Domain47WitnessSealRefusal::Overflowed));
    assert_eq!(seal(&Domain47WitnessLog::default()), Err(Domain47WitnessSealRefusal::Empty));
}

// kills: replaying a poisoned, invalidated, cold or witness-less cache.
#[test]
fn reuse_is_eligible_only_for_a_clean_adopted_cache_with_a_witness() {
    assert!(domain47_witness_reuse_eligible(true, 0, true, true));
    assert!(!domain47_witness_reuse_eligible(false, 0, true, true));
    assert!(!domain47_witness_reuse_eligible(true, 1, true, true));
    assert!(!domain47_witness_reuse_eligible(true, 0, false, true));
    assert!(!domain47_witness_reuse_eligible(true, 0, true, false));
}

// kills: a tap that recorded reads while disarmed (cost on every read of
// every domain) or dropped reads while armed (a hole in the witness); a
// capture whose drop left the tap armed.
#[test]
fn the_tap_records_only_between_start_and_finish() {
    domain47_witness_note(0x1, 1, Some(&[0]));
    let capture = Domain47WitnessCapture::start(Domain47WitnessLog::default());
    domain47_witness_note(0x10, 2, Some(&[1, 2]));
    domain47_witness_note(0x20, 4, None);
    let log = capture.finish();
    assert_eq!(log.records.len(), 2);
    assert_eq!(log.records[1].off, DOMAIN47_WITNESS_FAILED);
    assert_eq!(&log.bytes, &[1, 2]);
    domain47_witness_note(0x30, 1, Some(&[3]));
    let capture = Domain47WitnessCapture::start(log);
    domain47_witness_note(0x40, 1, Some(&[4]));
    let log = capture.finish();
    // `start` resets the scratch it is handed.
    assert_eq!(log.records.len(), 1);
    assert_eq!(log.records[0].addr, 0x40);
    {
        let _dropped = Domain47WitnessCapture::start(Domain47WitnessLog::default());
        assert_eq!(DOMAIN47_WITNESS_ARMED.load(Ordering::SeqCst), 1);
    }
    assert_eq!(DOMAIN47_WITNESS_ARMED.load(Ordering::SeqCst), 0);
    domain47_witness_note(0x50, 1, Some(&[5]));
    let none = DOMAIN47_WITNESS_TAP.with(|tap| tap.borrow().is_none());
    assert!(none, "a dropped capture leaves no log behind");
}

// kills: the miss codes are the wire's vocabulary; a collision would make two
// causes indistinguishable in a capture.
#[test]
fn miss_codes_are_distinct_and_carry_their_ordinal() {
    let all = [
        Domain47WitnessMiss::StateChanged,
        Domain47WitnessMiss::ManagerRootChanged,
        Domain47WitnessMiss::VectorOpenUnreadable,
        Domain47WitnessMiss::VectorOpenChanged,
        Domain47WitnessMiss::SpanChanged(3),
        Domain47WitnessMiss::SpanUnreadable(4),
        Domain47WitnessMiss::FailedReadNowReadable(5),
        Domain47WitnessMiss::VectorCloseUnreadable,
        Domain47WitnessMiss::VectorCloseChanged,
    ];
    let mut codes: Vec<u32> = all.iter().map(|m| m.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), all.len());
    assert!(codes.iter().all(|c| *c != 0 && *c < 10));
    assert_eq!(Domain47WitnessMiss::SpanChanged(3).ordinal(), 3);
    assert_eq!(Domain47WitnessMiss::VectorCloseChanged.ordinal(), 0);
}
