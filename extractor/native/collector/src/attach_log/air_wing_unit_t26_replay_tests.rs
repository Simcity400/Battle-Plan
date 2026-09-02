use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;

use compact_contract::record_header::{parse_compact_record_header, COMPACT_RECORD_HEADER_LEN};
use compact_contract::record_semantics::{
    COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_FRAME_END,
};

use super::*;

const D30: u16 = COMPACT_DOMAIN_AIR_WINGS;
const D32: u16 = COMPACT_DOMAIN_AIR_WING_LOADOUTS;
const D42: u16 = COMPACT_DOMAIN_AIR_COMBAT_LOSSES_LIVE;

const D30_HEADER_LEN: usize = 36;
const D30_ROW_LEN: usize = 40;
const D32_HEADER_LEN: usize = 44;
const D32_ROW_LEN: usize = 32;
const D32_ENTRY_LEN: usize = 16;

type AirWingKey = (u32, u32, u64);

fn corpus_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("BATTLE_PLAN_T26_COLLECTOR_STREAM").or_else(|_| std::env::var("HOI4_T26_COLLECTOR_STREAM")) {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("runs/live_unlock/t26/collector_stream.bin");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn read_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

fn read_u64(buf: &[u8], at: usize) -> u64 {
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&buf[at..at + 8]);
    u64::from_le_bytes(bytes)
}

fn read_i64(buf: &[u8], at: usize) -> i64 {
    read_u64(buf, at) as i64
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FrameOutcome<T> {
    Data(Vec<T>),
    Gapped,
    Absent,
}

#[derive(Debug, Clone)]
struct WireWingRow {
    owner_tag_db_id: u32,
    wing_id: u32,
    deleted: bool,
    count: u32,
    mission_mask: u32,
    mission_region: Option<u32>,
    effective_planes: u32,
}

#[derive(Debug, Clone)]
struct WireLoadoutRow {
    owner_tag_db_id: u32,
    wing_id: u32,
    deleted: bool,
    air_base_id: u32,
    equipment: Vec<CompactAirWingLoadoutEntry>,
}

fn decode_d30(payload: &[u8]) -> Vec<WireWingRow> {
    let row_count = read_u32(payload, 8) as usize;
    (0..row_count)
        .map(|index| {
            let base = D30_HEADER_LEN + index * D30_ROW_LEN;
            let flags = read_u32(payload, base + 16);
            WireWingRow {
                owner_tag_db_id: read_u32(payload, base),
                wing_id: read_u32(payload, base + 4),
                deleted: flags & COMPACT_AIR_WING_ROW_FLAG_DELETED != 0,
                count: read_u32(payload, base + 20),
                mission_mask: read_u32(payload, base + 24),
                mission_region: (flags & COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION != 0)
                    .then(|| read_u32(payload, base + 28)),
                effective_planes: read_u32(payload, base + 32),
            }
        })
        .collect()
}

fn decode_d32(payload: &[u8]) -> Vec<WireLoadoutRow> {
    let row_count = read_u32(payload, 8) as usize;
    let entries_at = D32_HEADER_LEN + row_count * D32_ROW_LEN;
    (0..row_count)
        .map(|index| {
            let base = D32_HEADER_LEN + index * D32_ROW_LEN;
            let flags = read_u32(payload, base + 16);
            let deleted = flags & COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED != 0;
            let entry_first = read_u32(payload, base + 24) as usize;
            let entry_count = read_u32(payload, base + 28) as usize;
            let equipment = if deleted {
                Vec::new()
            } else {
                (0..entry_count)
                    .map(|entry| {
                        let at = entries_at + (entry_first + entry) * D32_ENTRY_LEN;
                        CompactAirWingLoadoutEntry {
                            variant_id: read_u32(payload, at),
                            amount_raw: read_i64(payload, at + 8),
                        }
                    })
                    .collect()
            };
            WireLoadoutRow {
                owner_tag_db_id: read_u32(payload, base),
                wing_id: read_u32(payload, base + 4),
                deleted,
                air_base_id: read_u32(payload, base + 20),
                equipment,
            }
        })
        .collect()
}

struct Corpus {
    d30: Vec<FrameOutcome<WireWingRow>>,
    d32: Vec<FrameOutcome<WireLoadoutRow>>,
    d42_gapped: Vec<bool>,
    total_records: u64,
}

fn walk_corpus(path: &std::path::Path) -> Corpus {
    let file = std::fs::File::open(path).expect("open the t26 collector stream");
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut header = [0u8; COMPACT_RECORD_HEADER_LEN];
    let mut payload = Vec::new();

    let mut corpus = Corpus {
        d30: Vec::new(),
        d32: Vec::new(),
        d42_gapped: Vec::new(),
        total_records: 0,
    };
    let mut frame_d30 = FrameOutcome::Absent;
    let mut frame_d32 = FrameOutcome::Absent;
    let mut frame_d42_gapped = false;

    loop {
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => panic!("reading the t26 stream: {error}"),
        }
        let parsed = parse_compact_record_header(&header).expect("a well-formed H4CR header");
        payload.resize(parsed.payload_len, 0);
        reader
            .read_exact(&mut payload)
            .expect("a complete H4CR payload");
        corpus.total_records = corpus.total_records.saturating_add(1);

        match (parsed.domain_id, parsed.record_type) {
            (D30, COMPACT_RECORD_TYPE_DOMAIN_DATA) => {
                frame_d30 = FrameOutcome::Data(decode_d30(&payload));
            }
            (D30, COMPACT_RECORD_TYPE_DOMAIN_GAP) => frame_d30 = FrameOutcome::Gapped,
            (D32, COMPACT_RECORD_TYPE_DOMAIN_DATA) => {
                frame_d32 = FrameOutcome::Data(decode_d32(&payload));
            }
            (D32, COMPACT_RECORD_TYPE_DOMAIN_GAP) => frame_d32 = FrameOutcome::Gapped,
            (D42, COMPACT_RECORD_TYPE_DOMAIN_GAP) => frame_d42_gapped = true,
            _ => {}
        }
        if parsed.record_type == COMPACT_RECORD_TYPE_FRAME_END {
            corpus
                .d30
                .push(std::mem::replace(&mut frame_d30, FrameOutcome::Absent));
            corpus
                .d32
                .push(std::mem::replace(&mut frame_d32, FrameOutcome::Absent));
            corpus.d42_gapped.push(frame_d42_gapped);
            frame_d42_gapped = false;
        }
    }
    corpus
}

#[derive(Default)]
struct BodyAddrs {
    by_identity: BTreeMap<(u32, u32), usize>,
}

impl BodyAddrs {
    fn addr_of(&mut self, owner_tag_db_id: u32, wing_id: u32) -> usize {
        let next = self.by_identity.len().saturating_add(1) * 0x100;
        *self
            .by_identity
            .entry((owner_tag_db_id, wing_id))
            .or_insert(next)
    }
}

#[derive(Default)]
struct ObservedWings {
    live: BTreeMap<(u32, u32), WireWingRow>,
}

impl ObservedWings {
    fn apply(&mut self, rows: &[WireWingRow], whole: bool) {
        if whole {
            self.live.clear();
        }
        for row in rows {
            let identity = (row.owner_tag_db_id, row.wing_id);
            if row.deleted {
                self.live.remove(&identity);
            } else {
                self.live.insert(identity, row.clone());
            }
        }
    }
}

#[derive(Default)]
struct ObservedLoadouts {
    live: BTreeMap<(u32, u32), WireLoadoutRow>,
}

impl ObservedLoadouts {
    fn apply(&mut self, rows: &[WireLoadoutRow], whole: bool) {
        if whole {
            self.live.clear();
        }
        for row in rows {
            let identity = (row.owner_tag_db_id, row.wing_id);
            if row.deleted {
                self.live.remove(&identity);
            } else {
                self.live.insert(identity, row.clone());
            }
        }
    }
}

#[derive(Default)]
struct FoldModel {
    keys: BTreeSet<AirWingKey>,
    seen_first_record: bool,
    tombstones_applied: u64,
}

impl FoldModel {
    fn apply<K: Iterator<Item = (AirWingKey, bool)>>(&mut self, rows: K) {
        if !self.seen_first_record {
            self.keys.clear();
            self.seen_first_record = true;
        }
        for (key, deleted) in rows {
            if deleted {
                if self.keys.remove(&key) {
                    self.tombstones_applied = self.tombstones_applied.saturating_add(1);
                }
            } else {
                self.keys.insert(key);
            }
        }
    }
}

struct ReplayResult {
    frames: usize,
    d30_records: usize,
    d32_records: usize,
    d30_gapped_frames: usize,
    d32_gapped_frames: usize,
    unit_rejected_frames: usize,
    d30_tombstone_rows: u64,
    d32_tombstone_rows: u64,
    final_air_wing_keys: usize,
    final_loadout_keys: usize,
    orphan_loadout_keys: usize,
    max_generation: u64,
    distinct_wing_identities: usize,
}

fn replay(corpus: &Corpus) -> ReplayResult {
    let mut trackers = CompactPerfGateTrackers::default();
    let mut bodies = BodyAddrs::default();
    let mut observed_wings = ObservedWings::default();
    let mut observed_loadouts = ObservedLoadouts::default();
    let mut wing_fold = FoldModel::default();
    let mut loadout_fold = FoldModel::default();

    let mut result = ReplayResult {
        frames: corpus.d30.len(),
        d30_records: 0,
        d32_records: 0,
        d30_gapped_frames: 0,
        d32_gapped_frames: 0,
        unit_rejected_frames: 0,
        d30_tombstone_rows: 0,
        d32_tombstone_rows: 0,
        final_air_wing_keys: 0,
        final_loadout_keys: 0,
        orphan_loadout_keys: 0,
        max_generation: 0,
        distinct_wing_identities: 0,
    };

    let mut d30_previous_was_gap = true;
    let mut d32_previous_was_gap = true;

    for frame in 0..corpus.d30.len() {
        let mut rejected: BTreeSet<u16> = BTreeSet::new();
        if matches!(corpus.d30[frame], FrameOutcome::Gapped) {
            rejected.insert(D30);
            result.d30_gapped_frames += 1;
        }
        if matches!(corpus.d32[frame], FrameOutcome::Gapped) {
            rejected.insert(D32);
            result.d32_gapped_frames += 1;
        }
        if corpus.d42_gapped[frame] {
            rejected.insert(D42);
        }
        if !rejected.is_empty() {
            result.unit_rejected_frames += 1;
        }

        if let FrameOutcome::Data(rows) = &corpus.d30[frame] {
            result.d30_records += 1;
            observed_wings.apply(rows, d30_previous_was_gap);
            d30_previous_was_gap = false;
            let current: Vec<CompactAirWingRow> = observed_wings
                .live
                .values()
                .map(|row| CompactAirWingRow {
                    owner_tag_db_id: row.owner_tag_db_id,
                    wing_id: row.wing_id,
                    generation: 0,
                    body_addr: bodies.addr_of(row.owner_tag_db_id, row.wing_id),
                    deleted: false,
                    count: row.count,
                    mission_mask: row.mission_mask,
                    mission_region: row.mission_region,
                    effective_planes: row.effective_planes,
                })
                .collect();
            let generations = &trackers.air_wing_generations;
            let (emitted, ..) =
                compact_air_wing_delta_rows(&mut trackers.air_wings, generations, current, 0);
            result.d30_tombstone_rows = result
                .d30_tombstone_rows
                .saturating_add(emitted.iter().filter(|row| row.deleted).count() as u64);
            for row in &emitted {
                result.max_generation = result.max_generation.max(row.generation);
            }
            wing_fold.apply(emitted.iter().map(|row| {
                (
                    (row.owner_tag_db_id, row.wing_id, row.generation),
                    row.deleted,
                )
            }));
        } else if matches!(corpus.d30[frame], FrameOutcome::Gapped) {
            d30_previous_was_gap = true;
        }

        if let FrameOutcome::Data(rows) = &corpus.d32[frame] {
            result.d32_records += 1;
            observed_loadouts.apply(rows, d32_previous_was_gap);
            d32_previous_was_gap = false;
            let current: Vec<CompactAirWingLoadoutRow> = observed_loadouts
                .live
                .values()
                .map(|row| CompactAirWingLoadoutRow {
                    owner_tag_db_id: row.owner_tag_db_id,
                    wing_id: row.wing_id,
                    generation: 0,
                    body_addr: bodies.addr_of(row.owner_tag_db_id, row.wing_id),
                    deleted: false,
                    air_base_id: row.air_base_id,
                    equipment: row.equipment.clone(),
                })
                .collect();
            let generations = &trackers.air_wing_generations;
            let (emitted, ..) = compact_air_wing_loadout_delta_rows(
                &mut trackers.air_wing_loadouts,
                generations,
                current,
                0,
            );
            result.d32_tombstone_rows = result
                .d32_tombstone_rows
                .saturating_add(emitted.iter().filter(|row| row.deleted).count() as u64);
            loadout_fold.apply(emitted.iter().map(|row| {
                (
                    (row.owner_tag_db_id, row.wing_id, row.generation),
                    row.deleted,
                )
            }));
        } else if matches!(corpus.d32[frame], FrameOutcome::Gapped) {
            d32_previous_was_gap = true;
        }

        if !rejected.is_empty() {
            reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
        }
        commit_delta_pin_transactions(&mut trackers, &mut Vec::new());
    }

    result.final_air_wing_keys = wing_fold.keys.len();
    result.final_loadout_keys = loadout_fold.keys.len();
    result.orphan_loadout_keys = loadout_fold.keys.difference(&wing_fold.keys).count();
    result.distinct_wing_identities = bodies.by_identity.len();
    result
}

#[test]
#[ignore = "needs the 2.6 GB t26 capture; set BATTLE_PLAN_T26_COLLECTOR_STREAM"]
fn t26_air_wing_unit_replay() {
    let Some(path) = corpus_path() else {
        panic!(
            "the t26 collector stream was not found. Set BATTLE_PLAN_T26_COLLECTOR_STREAM to \
             <checkout>/extractor/native/runs/live_unlock/t26/collector_stream.bin"
        );
    };
    let corpus = walk_corpus(&path);
    let result = replay(&corpus);

    println!("t26 air-wing unit replay, corpus {}", path.display());
    println!("  records walked ............. {}", corpus.total_records);
    println!("  frames ..................... {}", result.frames);
    println!("  d30 DOMAIN_DATA ............ {}", result.d30_records);
    println!(
        "  d30 gapped frames .......... {}",
        result.d30_gapped_frames
    );
    println!("  d32 DOMAIN_DATA ............ {}", result.d32_records);
    println!(
        "  d32 gapped frames .......... {}",
        result.d32_gapped_frames
    );
    println!(
        "  frames rejecting the unit .. {}",
        result.unit_rejected_frames
    );
    println!(
        "  d30 tombstone rows ......... {}",
        result.d30_tombstone_rows
    );
    println!(
        "  d32 tombstone rows ......... {}",
        result.d32_tombstone_rows
    );
    println!(
        "  fold air_wings keys ........ {}",
        result.final_air_wing_keys
    );
    println!(
        "  fold loadout keys .......... {}",
        result.final_loadout_keys
    );
    println!(
        "  orphan loadout keys ........ {}",
        result.orphan_loadout_keys
    );
    println!("  max generation ............. {}", result.max_generation);
    println!(
        "  distinct (owner, wing_id) .. {}",
        result.distinct_wing_identities
    );

    assert_eq!(
        result.final_air_wing_keys as u64 + result.d30_tombstone_rows,
        result.distinct_wing_identities as u64,
        "the fold must end holding exactly the wing identities that were never tombstoned. The \
         shipped encoder ended with 2,310 keys for 657 identities because every post-gap whole \
         snapshot renumbered densely from zero and the fold, which only removes a tombstoned \
         key, merged a freshly-numbered copy of every wing on top of the old ones: {} live + {} \
         tombstoned vs {} identities",
        result.final_air_wing_keys,
        result.d30_tombstone_rows,
        result.distinct_wing_identities,
    );
    assert!(
        result.final_air_wing_keys < 1_000,
        "the shipped encoder left 2,310 keys behind for a 657-wing air force; anything near \
         that number means the superseded copies are back: {}",
        result.final_air_wing_keys
    );
    assert_eq!(
        result.orphan_loadout_keys, 0,
        "no d32 key may be left without a d30 counterpart, or \
         `world_fold::prune_air_wing_loadout_orphans_for_current_hour` deletes it"
    );
    assert!(
        result.d30_tombstone_rows > 2,
        "the shipped encoder emitted 2 d30 tombstone rows over the whole run because the \
         post-own-gap branch had no baseline left to sweep. The rollback restores that \
         baseline, so every wing that vanished across a gap must now be tombstoned: got {}",
        result.d30_tombstone_rows
    );
    assert!(
        result.d32_tombstone_rows > 2,
        "the same claim for d32, which is the domain the ruling names: got {}",
        result.d32_tombstone_rows
    );
    assert!(
        result.max_generation >= result.distinct_wing_identities as u64 - 1,
        "a stable body -> generation map hands out one number per identity, so the maximum \
         must reach the identity count rather than the per-stretch maximum of 652"
    );
}

#[test]
#[ignore = "needs the 2.6 GB t26 capture; set BATTLE_PLAN_T26_COLLECTOR_STREAM"]
fn the_wing_identity_stand_in_is_one_to_one_in_this_corpus() {
    let Some(path) = corpus_path() else {
        panic!("set BATTLE_PLAN_T26_COLLECTOR_STREAM");
    };
    let corpus = walk_corpus(&path);
    let mut offenders = Vec::new();
    for (frame, outcome) in corpus.d30.iter().enumerate() {
        if let FrameOutcome::Data(rows) = outcome {
            let mut seen = BTreeSet::new();
            for row in rows {
                if !seen.insert((row.owner_tag_db_id, row.wing_id, row.deleted)) {
                    offenders.push((frame, row.owner_tag_db_id, row.wing_id));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a d30 record carries the same wing identity twice, so `(owner, wing_id)` cannot stand \
         in for `body_addr` on this corpus: {offenders:?}"
    );
}

#[test]
fn the_fold_model_matches_the_shipped_worlds_rules() {
    let source = include_str!("../../../recorder_host/src/world_fold.rs").replace("\r\n", "\n");
    let d30 = source
        .split("fn fold_air_wings(")
        .nth(1)
        .expect("world_fold declares fold_air_wings")
        .split("\n    fn ")
        .next()
        .expect("the function body is bounded by the next fn");
    assert!(
        d30.contains(
            "let sparse = self.replace_or_sparse_after_first(AIR_WINGS_DOMAIN_ID, status_flags);"
        ) && d30.contains("if !sparse {\n            self.air_wings.clear();\n        }"),
        "d30's fold no longer clears only on the not-sparse (first-record) path; the model in \
         this module assumes exactly that"
    );
    assert!(
        d30.contains("self.air_wings.remove(&key);"),
        "d30's fold no longer removes a key on the deleted flag; tombstone-only removal is the \
         other rule this model assumes"
    );
}

#[test]
fn the_air_wing_bank_producer_bounds_match_the_consumers_refusal_rule() {
    assert_eq!(
        super::offsets::MAX_DIVISION_EQUIPMENT_VARIANT_ID,
        i32::MAX as u32,
        "portable_admission::AIR_WING_BANK_MAX_VARIANT_ID mirrors this"
    );
    assert_eq!(
        super::offsets::MAX_AIR_WING_EQUIPMENT_AMOUNT_RAW,
        1_000_000_000_000_000,
        "portable_admission::AIR_WING_BANK_MAX_AMOUNT_RAW mirrors this"
    );
    assert_eq!(
        super::offsets::MAX_AIR_WING_EQUIPMENT_BANK_ROWS,
        512,
        "portable_admission::AIR_WING_BANK_MAX_ROWS mirrors this"
    );
}
