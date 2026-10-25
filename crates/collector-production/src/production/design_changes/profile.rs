use serde::{
    Serialize, Serializer,
    ser::{SerializeMap, SerializeSeq},
};
use std::{
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};

const CAPACITY: usize = 64;
const PHASES: usize = 12;
pub(super) const CACHE_FIELDS: usize = 15;
const WIDTH: usize = 4 + PHASES * 3 + CACHE_FIELDS;
const PHASE_NAMES: [&str; PHASES] = [
    "drain",
    "decode_batch_shared",
    "cache_full_decode",
    "liveness_refresh_context",
    "held_apply_shared",
    "sources_prune_interests",
    "bonus_snapshots",
    "design_context",
    "design_context_parts",
    "registry_walk",
    "variants_walk",
    "variant_parts",
];
const CACHE_NAMES: [&str; CACHE_FIELDS] = [
    "full_copy_hits",
    "read_witness_hits",
    "full_decodes",
    "no_recent_entry",
    "span_count_differs",
    "span_address_differs",
    "span_metadata_differs",
    "read_bytes_differ",
    "overlap_inconsistent",
    "context_not_reusable",
    "insert_skipped_by_allocation",
    "read_witness_unavailable",
    "unreadable_copy",
    "witness_read_failed",
    "insert_overlap_inconsistent",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Outcome {
    FullCopyHit,
    ReadWitnessHit,
    FullDecode,
    NoRecentEntry,
    SpanCount,
    SpanAddress,
    SpanMetadata,
    ReadBytes,
    Overlap,
    ContextNotReusable,
    AllocationSkipped,
    WitnessUnavailable,
    UnreadableCopy,
    WitnessReadFailed,
    InsertOverlap,
}
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Phase {
    Drain,
    DecodeBatch,
    FullDecode,
    Liveness,
    Held,
    Prune,
    Bonuses,
    Context,
    ContextParts,
    Registry,
    Variants,
    VariantParts,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub(super) struct Cost {
    wall_ns: u64,
    growths: u64,
    calls: u64,
}
impl Cost {
    pub(super) fn add(&mut self, other: Self) {
        self.wall_ns += other.wall_ns;
        self.growths += other.growths;
        self.calls += other.calls;
    }
}
pub(super) struct Stamp {
    begin: Instant,
    growths: u64,
}
impl Stamp {
    pub(super) fn new() -> Self {
        Self {
            begin: Instant::now(),
            growths: engine_allocation::growths(),
        }
    }
    pub(super) fn finish(self) -> Cost {
        Cost {
            wall_ns: self.begin.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            growths: engine_allocation::growths().saturating_sub(self.growths),
            calls: 1,
        }
    }
}
// Each hourly Variants parent drains its children before the next parent starts.
static ACTIVE: AtomicBool = AtomicBool::new(false);
static WORK: [AtomicU64; PHASES * 3 + CACHE_FIELDS] =
    [const { AtomicU64::new(0) }; PHASES * 3 + CACHE_FIELDS];
static LATEST: Slot = Slot::new();
static SLOWEST: [Slot; CAPACITY] = [const { Slot::new() }; CAPACITY];

pub struct Timer {
    phase: Phase,
    stamp: Option<Stamp>,
}
impl Timer {
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            stamp: ACTIVE.load(Ordering::Relaxed).then(Stamp::new),
        }
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(stamp) = self.stamp.take() {
            add_cost(self.phase, stamp.finish());
        }
    }
}
pub(super) fn add_cost(phase: Phase, cost: Cost) {
    if ACTIVE.load(Ordering::Relaxed) {
        for (field, value) in
            WORK[phase as usize * 3..][..3]
                .iter()
                .zip([cost.wall_ns, cost.growths, cost.calls])
        {
            field.fetch_add(value, Ordering::Relaxed);
        }
    }
}
pub(super) fn add_cache(counts: [u64; CACHE_FIELDS]) {
    if ACTIVE.load(Ordering::Relaxed) {
        for (field, value) in WORK[PHASES * 3..].iter().zip(counts) {
            field.fetch_add(value, Ordering::Relaxed);
        }
    }
}
pub struct Job {
    hour: i32,
    qpc: i64,
    stamp: Option<Stamp>,
}
impl Job {
    pub fn begin(hour: i32) -> Self {
        for field in &WORK {
            field.store(0, Ordering::Relaxed);
        }
        ACTIVE.store(true, Ordering::Release);
        Self {
            hour,
            qpc: crate::clock::qpc(),
            stamp: Some(Stamp::new()),
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        let cost = self.stamp.take().unwrap().finish();
        ACTIVE.store(false, Ordering::Release);
        let mut fields = [0; WIDTH];
        fields[..4].copy_from_slice(&[
            self.hour as u32 as u64,
            self.qpc as u64,
            cost.wall_ns,
            cost.growths,
        ]);
        for (field, source) in fields[4..].iter_mut().zip(&WORK) {
            *field = source.load(Ordering::Acquire);
        }
        LATEST.publish(fields);
        retain(&SLOWEST, fields);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hour {
    fields: [u64; WIDTH],
}
impl Default for Hour {
    fn default() -> Self {
        Self { fields: [0; WIDTH] }
    }
}
struct Phases(Hour);
impl Serialize for Phases {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(PHASES))?;
        for (name, fields) in PHASE_NAMES.iter().zip(self.0.fields[4..].chunks_exact(3)) {
            map.serialize_entry(
                name,
                &Cost {
                    wall_ns: fields[0],
                    growths: fields[1],
                    calls: fields[2],
                },
            )?;
        }
        map.end()
    }
}
struct Outcomes(Hour);
impl Serialize for Outcomes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(CACHE_FIELDS))?;
        for (name, count) in CACHE_NAMES.iter().zip(&self.0.fields[4 + PHASES * 3..]) {
            map.serialize_entry(name, count)?;
        }
        map.end()
    }
}
impl Serialize for Hour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct View {
            hour: i32,
            qpc: i64,
            wall_ns: u64,
            parent_growths: u64,
            phases: Phases,
            cache: Outcomes,
        }
        View {
            hour: self.fields[0] as u32 as i32,
            qpc: self.fields[1] as i64,
            wall_ns: self.fields[2],
            parent_growths: self.fields[3],
            phases: Phases(*self),
            cache: Outcomes(*self),
        }
        .serialize(serializer)
    }
}
struct Slot {
    sequence: AtomicU64,
    wall_ns: AtomicU64,
    fields: [AtomicU64; WIDTH],
}
impl Slot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            wall_ns: AtomicU64::new(0),
            fields: [const { AtomicU64::new(0) }; WIDTH],
        }
    }
    fn publish(&self, fields: [u64; WIDTH]) {
        self.sequence.fetch_add(1, Ordering::SeqCst);
        for (field, value) in self.fields.iter().zip(fields) {
            field.store(value, Ordering::SeqCst);
        }
        self.wall_ns.store(fields[2], Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::SeqCst);
    }
    fn read(&self) -> Option<Hour> {
        let sequence = self.sequence.load(Ordering::SeqCst);
        if sequence == 0 || sequence & 1 != 0 {
            return None;
        }
        let fields = self
            .fields
            .each_ref()
            .map(|field| field.load(Ordering::SeqCst));
        (self.sequence.load(Ordering::SeqCst) == sequence).then_some(Hour { fields })
    }
}
fn retain(slots: &[Slot], fields: [u64; WIDTH]) {
    let slot = slots
        .iter()
        .min_by_key(|slot| {
            (
                slot.wall_ns.load(Ordering::Relaxed),
                std::cmp::Reverse(slot.fields[1].load(Ordering::Relaxed)),
            )
        })
        .unwrap();
    if fields[2] > slot.wall_ns.load(Ordering::Relaxed) {
        slot.publish(fields);
    }
}

pub fn latest(hour: i32, callback_qpc: i64) -> Option<Hour> {
    LATEST.read().filter(|entry| {
        entry.fields[0] as u32 as i32 == hour && entry.fields[1] as i64 >= callback_qpc
    })
}
pub(super) struct Status;
impl Serialize for Status {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Tail;
        impl Serialize for Tail {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(None)?;
                for slot in &SLOWEST {
                    if let Some(hour) = slot.read() {
                        seq.serialize_element(&hour)?;
                    }
                }
                seq.end()
            }
        }
        #[derive(Serialize)]
        struct View {
            capacity: usize,
            scope: &'static str,
            slowest: Tail,
        }
        View {
            capacity: CAPACITY,
            scope: "Slowest instrumented Variants work intervals since attach, unordered, first on ties; wall_ns excludes diagnostic setup/publication. All values are status-only. Phase walls and thread-local checked allocation growths are inclusive and overlap: drain contains decode_batch_shared, cache_full_decode, liveness, held and prune; design_context and variants_walk contain their parallel parts. Parts sum body wall across lanes, not elapsed wall. Parent phase growths exclude other threads; part growths can include same-thread work already charged to the parent. Cache hit counts are per snapshot. Full-decode miss reasons use the most recent pointer entry; read_witness_unavailable means its witness was absent, not changed bytes. NoRecentEntry/span/read-bytes/overlap/witness-unavailable reasons partition full_decodes against the most recent entry. Context, insert_overlap_inconsistent, unreadable_copy, witness_read_failed and allocation skips are independent insertion/fallback counts; allocation skips count failed optional stages, not misses. Do not add them to full_decodes. Pending or concurrently replaced slots are omitted, never partially read.",
            slowest: Tail,
        }.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tail_evicts_the_latest_minimum_and_never_displaces_an_equal_arrival() {
        let slots = [Slot::new(), Slot::new()];
        let sample = |qpc, wall| {
            let mut fields = [0; WIDTH];
            fields[1] = qpc;
            fields[2] = wall;
            fields
        };
        retain(&slots, sample(1, 10));
        retain(&slots, sample(2, 10));
        retain(&slots, sample(3, 11));
        retain(&slots, sample(4, 10));
        let mut retained = slots.each_ref().map(|slot| slot.read().unwrap().fields[1]);
        retained.sort_unstable();
        assert_eq!(retained, [1, 3]);
    }

    #[test]
    fn fixed_slot_round_trip_never_publishes_pending_fields() {
        let slot = Slot::new();
        assert!(slot.read().is_none());
        let mut fields = std::array::from_fn(|at| at as u64 + 1);
        fields[0] = i32::MIN as u32 as u64;
        slot.publish(fields);
        assert_eq!(slot.read().unwrap().fields, fields);
        assert_eq!(slot.read().unwrap().fields[4], 5);
        assert_eq!(slot.read().unwrap().fields[39], 40);
        assert_eq!(slot.read().unwrap().fields[40], 41);
        slot.sequence.fetch_add(1, Ordering::SeqCst);
        slot.fields[2].store(999, Ordering::SeqCst);
        assert!(slot.read().is_none());
    }
}
