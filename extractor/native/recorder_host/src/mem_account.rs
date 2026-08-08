//! Always-on resident-memory accounting for the recorder process.
//!
//! # Why this exists
//!
//! The recorder used to report exactly two memory numbers -- `host_finalize.
//! peak_private_bytes` and `peak_working_set_bytes` (see [`crate::proc_mem`]) --
//! and one prose claim, `spool_backpressure.host_memory_bound`, asserting that
//! the capture-time spool path is "one 64 KiB pipe read buffer + a 1 MiB
//! BufWriter + one zstd encoder context; independent of capture length".
//!
//! Both are true and neither is a component table. A process peak says *how
//! much*; it never says *what*, so no one could tell whether a gigabyte was the
//! irreducible size of the world being modelled or a buffer somebody sized by
//! the worst case they had ever seen. This module is the missing middle: named
//! owners charge the bytes they hold, the process publishes the table, and the
//! difference between the table and the OS's own number is published too, so an
//! owner that forgets to charge shows up as a growing residual rather than as
//! silence.
//!
//! # The honesty rule
//!
//! **Charge before you allocate, release after you drop.** A counter that is
//! incremented after a successful allocation under-reports exactly at the
//! moment the number matters (the peak), and a counter released before the
//! owner is actually dropped reports a trough that never happened. [`Charge`]
//! is an RAII guard so the release half cannot be forgotten on an early return
//! or an unwind; prefer it to bare [`charge`]/[`release`].
//!
//! The registry never claims to be complete. [`report_json`] always publishes
//! `unaccounted_bytes` = OS private bytes - sum(components), so an uncharged
//! owner is visible as a number rather than hidden by omission. That residual
//! is expected to be nonzero and fairly large: it also carries the binary's own
//! image, thread stacks, allocator metadata and every small allocation nobody
//! would sensibly name. It is a *bound* on what the table is missing, not a
//! defect count.
//!
//! [`Component`] therefore lists only owners that are ACTUALLY charged. A
//! component declared but never charged reads as "holds zero bytes", which is a
//! stronger and falser claim than saying nothing. Three large owners are known
//! and deliberately absent, because none of them can be priced without either
//! walking a structure per hour or estimating:
//!
//!   * the fold's retained world state — bounded, not measured, by the
//!     live-heap difference across the `finalize.before_fold_release` /
//!     `finalize.after_fold_release` mark pair, which brackets the call that
//!     consumes the cursor. The cache-encode consumer is still running across
//!     those two marks, so the difference also carries whatever it allocated
//!     and freed in between: it is an estimate of the fold's state, not a
//!     measurement of it;
//!   * the game content index and effective VFS — bracketed by the
//!     `capture.window_open` mark, which is taken with the spool charge already
//!     live, so content is what remains after the spool writer is subtracted;
//!   * the session manifest, its read buffer and its `serde_json::Value` trees
//!     at pack time — bracketed by the `finalize.before_manifest_rewrite` mark.
//!     A component for these was written and then withdrawn: the rewrite runs
//!     AFTER [`report_json`] has been taken for that same manifest, so any
//!     counter for it could only ever publish its own previous value. A number
//!     that can never be right is worse than a mark saying where to look.
//!
//! All three land in `unaccounted_bytes`, where they are visible, rather than
//! in a component whose number would be wrong.
//!
//! # Cost
//!
//! Every component is charged where its *capacity* is decided -- once per
//! buffer, once per game hour -- never per allocation. A charge is two relaxed
//! atomic RMWs on a per-component cache line. The finalize path charges at most
//! a few times per hour against hundreds of thousands of allocations per hour,
//! so this is production code that runs in every build, not a diagnostic mode.
//! `mark()` additionally makes one `GetProcessMemoryInfo` call, and is used
//! only at phase boundaries (tens of calls per process lifetime).
//!
//! For the finer-grained question -- "is the registry itself lying?" -- build
//! with the `mem-alloc-trace` feature, which wraps the global allocator in
//! [`TrackingAlloc`] and makes the live Rust-allocator heap readable via
//! [`live_heap_bytes`]. That is a measurement build, not production: it adds an
//! atomic RMW to every allocation on an allocation-bound path.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// A named owner of resident bytes.
///
/// The set is deliberately closed and small: a component is a thing whose size
/// somebody has to be able to argue about, not every allocation site. Adding
/// one means committing to charging it honestly everywhere it is allocated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum Component {
    // ---- capture phase ----
    /// The spool sink's `BufWriter` capacity plus, when the spool is
    /// compressed, the zstd encoder's match WINDOW.
    ///
    /// Priced by `compact_stream::writer_reserved_bytes` from the constants
    /// that module controls, which makes it a FLOOR: libzstd's own match tables
    /// and working buffers live inside its context and no caller can read them,
    /// so they land in `unaccounted_bytes`. The window alone is the dominant
    /// term — `compact_stream::ZSTD_WINDOW_LOG` is 27, so 128 MiB.
    ///
    /// Charged only where this process owns the writer. A rollover capture's
    /// sinks belong to the rollover store and are not charged here.
    SpoolWriter = 0,

    // ---- finalize phase ----
    /// The decoder's framing buffers (`HourCursor::carry` + `chunk`).
    DecoderFraming = 1,
    /// One index row per published game hour (`HourCursor::index_hours`).
    /// A session-length scaler by construction.
    DecoderHourIndex = 2,
    /// Serialized whole-world snapshot bytes in flight between the producer
    /// and the cache-encode consumer: the producer's held-back hour, the
    /// channel's queued hours and the hour the consumer is working on.
    InflightSnapshots = 3,
    /// The previous hour's canonical world tree, retained by the cache encoder
    /// as the delta base.
    CacheDeltaBase = 4,
    /// One cache index row per encoded hour (`ReplayCacheEncoder::entries`).
    /// A session-length scaler by construction.
    CacheHourEntries = 5,
}

impl Component {
    /// Every component, in declaration order. `report_json` walks this.
    pub const ALL: [Component; COMPONENT_COUNT] = [
        Component::SpoolWriter,
        Component::DecoderFraming,
        Component::DecoderHourIndex,
        Component::InflightSnapshots,
        Component::CacheDeltaBase,
        Component::CacheHourEntries,
    ];

    /// The stable manifest key for this component. Changing one of these
    /// renames a published field, so they are treated as wire names.
    pub const fn key(self) -> &'static str {
        match self {
            Component::SpoolWriter => "spool_writer",
            Component::DecoderFraming => "decoder_framing",
            Component::DecoderHourIndex => "decoder_hour_index",
            Component::InflightSnapshots => "inflight_snapshots",
            Component::CacheDeltaBase => "cache_delta_base",
            Component::CacheHourEntries => "cache_hour_entries",
        }
    }

    /// How this component's size is expected to grow.
    ///
    /// This is the classification the production blocker is stated in terms of:
    /// anything answering [`Scaling::SessionLength`] is memory that grows for
    /// as long as the recording runs, and every one of them needs a bound. It
    /// is declared here, next to the counter, so a future component cannot be
    /// added without someone answering the question.
    pub const fn scaling(self) -> Scaling {
        match self {
            Component::SpoolWriter => Scaling::Fixed,
            Component::DecoderFraming => Scaling::Fixed,
            Component::DecoderHourIndex => Scaling::SessionLength,
            Component::InflightSnapshots => Scaling::GameSize,
            Component::CacheDeltaBase => Scaling::GameSize,
            Component::CacheHourEntries => Scaling::SessionLength,
        }
    }
}

/// What determines a component's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaling {
    /// A compile-time constant, or a runtime constant with a hard ceiling.
    Fixed,
    /// Proportional to the size of the world being modelled -- countries,
    /// states, divisions. Large, but it stops growing when the map does.
    GameSize,
    /// Proportional to the number of game hours recorded. This is the class a
    /// long session cannot survive; each instance needs a stated per-hour rate.
    SessionLength,
}

impl Scaling {
    pub const fn key(self) -> &'static str {
        match self {
            Scaling::Fixed => "fixed",
            Scaling::GameSize => "game_size",
            Scaling::SessionLength => "session_length",
        }
    }
}

pub const COMPONENT_COUNT: usize = 6;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);

static HELD: [AtomicU64; COMPONENT_COUNT] = [ZERO; COMPONENT_COUNT];
static PEAK: [AtomicU64; COMPONENT_COUNT] = [ZERO; COMPONENT_COUNT];
/// Set when a release would have driven a counter below zero. That can only
/// happen if a release is not paired with its charge, which makes every later
/// number for that component untrustworthy -- so it is published rather than
/// clamped silently.
static UNDERFLOWED: [AtomicBool; COMPONENT_COUNT] = [const { AtomicBool::new(false) }; COMPONENT_COUNT];

/// Charge `bytes` to `component`. Call this BEFORE the allocation it accounts
/// for, so the peak is never observed low.
pub fn charge(component: Component, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let index = component as usize;
    let after = HELD[index].fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK[index].fetch_max(after, Ordering::Relaxed);
}

/// Release `bytes` from `component`. Call this AFTER the owner is dropped.
pub fn release(component: Component, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let index = component as usize;
    // `fetch_update` rather than `fetch_sub`: an unpaired release must be
    // recorded as an accounting fault, not wrapped around into a 16-exabyte
    // reading that would then be published as a component size.
    let outcome = HELD[index].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
        Some(held.saturating_sub(bytes))
    });
    if let Ok(previous) = outcome {
        if previous < bytes {
            UNDERFLOWED[index].store(true, Ordering::Relaxed);
        }
    }
}

/// Move a component's charge to a new absolute size, for owners that grow in
/// place (a `Vec` that reallocates, a map that gains a row per hour).
pub fn resize(component: Component, from_bytes: u64, to_bytes: u64) {
    match to_bytes.cmp(&from_bytes) {
        std::cmp::Ordering::Greater => charge(component, to_bytes - from_bytes),
        std::cmp::Ordering::Less => release(component, from_bytes - to_bytes),
        std::cmp::Ordering::Equal => {}
    }
}

/// Bytes `component` currently holds.
pub fn held(component: Component) -> u64 {
    HELD[component as usize].load(Ordering::Relaxed)
}

/// The largest value [`held`] ever reached for `component`.
pub fn peak(component: Component) -> u64 {
    PEAK[component as usize].load(Ordering::Relaxed)
}

/// Whether `component`'s counter ever went negative, which invalidates it.
pub fn underflowed(component: Component) -> bool {
    UNDERFLOWED[component as usize].load(Ordering::Relaxed)
}

/// Sum of what every component currently claims to hold.
pub fn total_held() -> u64 {
    Component::ALL
        .iter()
        .map(|component| held(*component))
        .fold(0u64, u64::saturating_add)
}

/// An RAII charge: the bytes are charged now and released when this is dropped.
///
/// Hold it alongside the thing it accounts for -- in the same struct, or in the
/// same scope -- so the release cannot be skipped by an early return or unwind.
#[derive(Debug)]
pub struct Charge {
    component: Component,
    bytes: u64,
}

impl Charge {
    /// Charge `bytes` to `component` for as long as the guard lives.
    pub fn new(component: Component, bytes: u64) -> Self {
        charge(component, bytes);
        Self { component, bytes }
    }

    /// Adjust the charge in place, for an owner whose size changed.
    pub fn set(&mut self, bytes: u64) {
        resize(self.component, self.bytes, bytes);
        self.bytes = bytes;
    }

    /// The bytes currently charged through this guard.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        release(self.component, self.bytes);
    }
}

// `Charge` is deliberately NOT `Clone` and NOT `PartialEq`. Both were once
// implemented so a struct embedding a guard could keep its derives, and both
// were traps: a `Clone` that charges again is right only if the clone really
// duplicated the bytes, and an equality that ignores the guard silently changes
// what equality means for the embedding type. A type that embeds a guard writes
// its own `Clone`/`PartialEq` instead, where the relationship between the
// allocation and the charge is visible.

// ---------------------------------------------------------------------------
// Phase marks
// ---------------------------------------------------------------------------

/// The most phase marks retained. A mark is written at a phase boundary, so the
/// bound is on the number of phases, not on anything the session controls; the
/// cap exists only so a caller in a loop cannot turn this into the very kind of
/// session-length growth the module was written to find.
pub const MAX_MARKS: usize = 64;

#[derive(Debug, Clone)]
pub struct Mark {
    pub label: &'static str,
    /// OS private (commit) bytes at the mark, or `None` if the OS call failed.
    ///
    /// Optional rather than zero: a mark that could not read the process would
    /// otherwise publish a plausible-looking `0`, and a reader comparing marks
    /// across a phase would see a memory cliff that never happened.
    pub private_bytes: Option<u64>,
    /// Live Rust-allocator heap at the mark, when built with `mem-alloc-trace`
    /// (see [`live_heap_bytes`] for what that does and does not cover).
    pub live_heap_bytes: Option<u64>,
    /// Each component's held bytes at the mark.
    pub components: [u64; COMPONENT_COUNT],
}

static MARKS: std::sync::Mutex<Vec<Mark>> = std::sync::Mutex::new(Vec::new());
static MARKS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Record a phase boundary: the OS's private-bytes reading and every
/// component's charge at this instant, under one label.
///
/// Costs one `GetProcessMemoryInfo` call. Intended for phase boundaries; the
/// ring is capped at [`MAX_MARKS`] and surplus marks are counted, not stored.
pub fn mark(label: &'static str) {
    let private_bytes = crate::proc_mem::current_private_bytes();
    let mut components = [0u64; COMPONENT_COUNT];
    for (slot, component) in components.iter_mut().zip(Component::ALL.iter()) {
        *slot = held(*component);
    }
    let entry = Mark {
        label,
        private_bytes,
        live_heap_bytes: live_heap_bytes(),
        components,
    };
    if let Ok(mut marks) = MARKS.lock() {
        if marks.len() >= MAX_MARKS {
            MARKS_DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
        marks.push(entry);
    }
}

/// Every mark recorded so far, in order.
pub fn marks() -> Vec<Mark> {
    MARKS.lock().map(|marks| marks.clone()).unwrap_or_default()
}

/// Marks that were discarded because [`MAX_MARKS`] was already reached.
pub fn marks_dropped() -> u64 {
    MARKS_DROPPED.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------

/// The manifest block: every component's current and peak charge, its declared
/// scaling class, and the residual the table does not explain.
///
/// `unaccounted_bytes` is `private_bytes - total_held` and is expected to be
/// substantial: it carries the executable image, thread stacks, allocator
/// metadata, and every allocation too small to be worth a name. It is published
/// so that a component which stops charging shows up as a residual that grows,
/// which is the only way a reader can tell this table apart from a table that
/// is merely quiet.
///
/// The residual is NOT clamped at zero. If the components ever claim more than
/// the process has committed, that is an over-charge — a double count, or a
/// guard outliving its bytes — and it is published as `overaccounted_bytes`
/// with `accounting_consistent: false`. Saturating it to zero would turn the
/// one piece of arithmetic that can catch an over-charge into silence, which is
/// the failure mode this module exists to prevent.
pub fn report_json() -> serde_json::Value {
    let private_bytes = crate::proc_mem::current_private_bytes();
    let total = total_held();
    // Signed in effect, via two fields: exactly one of them is populated. Both
    // are null when the OS call failed, because then neither is computable.
    let (unaccounted, overaccounted) = match private_bytes {
        Some(private) if private >= total => (Some(private - total), None),
        Some(private) => (None, Some(total - private)),
        None => (None, None),
    };
    let mut components = serde_json::Map::new();
    for component in Component::ALL {
        components.insert(
            component.key().to_string(),
            serde_json::json!({
                "held_bytes": held(component),
                "peak_bytes": peak(component),
                "scaling": component.scaling().key(),
                "accounting_underflow": underflowed(component),
            }),
        );
    }
    let marks: Vec<serde_json::Value> = marks()
        .into_iter()
        .map(|entry| {
            let mut per_component = serde_json::Map::new();
            for (component, bytes) in Component::ALL.iter().zip(entry.components.iter()) {
                per_component.insert(component.key().to_string(), serde_json::json!(bytes));
            }
            serde_json::json!({
                "label": entry.label,
                "private_bytes": entry.private_bytes,
                "live_heap_bytes": entry.live_heap_bytes,
                "components": per_component,
            })
        })
        .collect();
    serde_json::json!({
        "schema": "battle-plan-recorder-memory-account-v1",
        "note": "named owners charge the bytes they hold; unaccounted_bytes bounds what this \
                 table does not name (image, stacks, allocator metadata, unnamed small \
                 allocations), so an owner that stops charging shows up as a growing residual",
        "components": components,
        "total_held_bytes": total,
        "private_bytes": private_bytes,
        "unaccounted_bytes": unaccounted,
        "overaccounted_bytes": overaccounted,
        // `null`, not `true`, when the OS reading failed. Consistency is a
        // claim about evidence; with no `private_bytes` there is nothing to
        // check the components against, and answering "consistent" would be
        // asserting a comparison that was never made.
        "accounting_consistent": private_bytes.map(|_| {
            overaccounted.is_none()
                && Component::ALL
                    .iter()
                    .all(|component| !underflowed(*component))
        }),
        "live_heap_bytes": live_heap_bytes(),
        "marks": marks,
        "marks_dropped": marks_dropped(),
    })
}

// ---------------------------------------------------------------------------
// Optional whole-heap tracking (`mem-alloc-trace`)
// ---------------------------------------------------------------------------

#[cfg(feature = "mem-alloc-trace")]
mod tracking {
    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicI64, Ordering};

    /// Live heap bytes, and the largest value it ever reached.
    ///
    /// ONE counter, not a sharded pair. A sharded allocated/freed pair is
    /// cheaper but cannot be read coherently: summing all the allocation shards
    /// and then all the free shards, while two finalize threads keep working,
    /// observes some blocks on only one side of the subtraction. The number
    /// that came out was an oracle nobody could trust to be a real instant.
    /// One contended atomic costs more per allocation and is worth it, because
    /// this is a measurement build whose whole job is to be right; production
    /// does not compile it.
    ///
    /// Signed, because a block allocated before this counter existed (or by a
    /// path that bypassed it) can still be freed through it, and a wrapped
    /// unsigned counter would then read as sixteen exabytes.
    pub(super) static LIVE: AtomicI64 = AtomicI64::new(0);
    pub(super) static PEAK: AtomicI64 = AtomicI64::new(0);

    #[inline]
    fn add(delta: i64) {
        let live = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
        if delta > 0 {
            // Exact, not sampled: the peak is taken on the allocation that set
            // it, so a spike shorter than a sampling interval is still seen.
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
    }

    /// Wraps another allocator and keeps a running total of live bytes.
    ///
    /// Only compiled under `mem-alloc-trace`: it puts an atomic read-modify-write
    /// on every allocation, on a path that allocates hundreds of thousands of
    /// times per game hour.
    pub struct TrackingAlloc<A>(pub A);

    unsafe impl<A: GlobalAlloc> GlobalAlloc for TrackingAlloc<A> {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = self.0.alloc(layout);
            if !pointer.is_null() {
                add(layout.size() as i64);
            }
            pointer
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            add(-(layout.size() as i64));
            self.0.dealloc(pointer, layout);
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let pointer = self.0.alloc_zeroed(layout);
            if !pointer.is_null() {
                add(layout.size() as i64);
            }
            pointer
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let moved = self.0.realloc(pointer, layout, new_size);
            if !moved.is_null() {
                add(new_size as i64 - layout.size() as i64);
            }
            moved
        }
    }
}

#[cfg(feature = "mem-alloc-trace")]
pub use tracking::TrackingAlloc;

/// Live bytes requested through Rust's global allocator, when built with
/// `mem-alloc-trace`; `None` otherwise.
///
/// This is the oracle the component table is reconciled against: unlike the
/// table it cannot be under-reported by a forgotten charge, and it is one
/// atomic load, so the value is a real instant rather than a stitched-together
/// one.
///
/// It is NOT the whole process heap, and calling it that would be the same kind
/// of overclaim this module was written to remove. It sees exactly what passes
/// through `GlobalAlloc`. Memory a native library allocates for itself — most
/// of all libzstd's compression context, whose match window at
/// `compact_stream::ZSTD_WINDOW_LOG` is the single largest capture-time
/// allocation in the process — never goes through Rust's allocator and is
/// invisible here. Those bytes are still in the OS's `private_bytes`, which is
/// why `report_json` computes its residual against that and not against this.
#[cfg(feature = "mem-alloc-trace")]
pub fn live_heap_bytes() -> Option<u64> {
    Some(tracking::LIVE.load(Ordering::Relaxed).max(0) as u64)
}

/// The largest live heap ever reached, exact rather than sampled.
#[cfg(feature = "mem-alloc-trace")]
pub fn peak_live_heap_bytes() -> Option<u64> {
    Some(tracking::PEAK.load(Ordering::Relaxed).max(0) as u64)
}

#[cfg(not(feature = "mem-alloc-trace"))]
pub fn peak_live_heap_bytes() -> Option<u64> {
    None
}

#[cfg(not(feature = "mem-alloc-trace"))]
pub fn live_heap_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Charges and releases move the counter, and the peak survives the
    /// release. Uses a component the production code does not touch during
    /// tests so the assertion is about this module, not about a neighbour.
    #[test]
    fn charge_release_tracks_current_and_peak() {
        let component = Component::CacheDeltaBase;
        let base_held = held(component);
        let base_peak = peak(component);
        charge(component, 4096);
        assert_eq!(held(component), base_held + 4096);
        assert!(peak(component) >= base_peak.max(base_held + 4096));
        release(component, 4096);
        assert_eq!(held(component), base_held);
        // The peak is the point of the exercise: it must NOT fall back.
        assert!(peak(component) >= base_held + 4096);
    }

    /// The RAII guard releases on drop, including on an early return path.
    #[test]
    fn charge_guard_releases_on_drop() {
        let component = Component::DecoderFraming;
        let base = held(component);
        {
            let _guard = Charge::new(component, 1 << 20);
            assert_eq!(held(component), base + (1 << 20));
        }
        assert_eq!(held(component), base);
    }

    /// Resizing through the guard nets out to the new size, not to the sum.
    #[test]
    fn charge_guard_resize_is_absolute_not_cumulative() {
        let component = Component::InflightSnapshots;
        let base = held(component);
        let mut guard = Charge::new(component, 100);
        guard.set(700);
        assert_eq!(held(component), base + 700);
        guard.set(50);
        assert_eq!(held(component), base + 50);
        drop(guard);
        assert_eq!(held(component), base);
    }

    /// An unpaired release must be recorded as a fault rather than wrapping
    /// the counter to ~2^64, which would then be published as a component size.
    #[test]
    fn unpaired_release_saturates_and_flags_rather_than_wrapping() {
        let component = Component::CacheHourEntries;
        assert!(!underflowed(component));
        release(component, u64::MAX);
        assert_eq!(held(component), 0);
        assert!(underflowed(component));
        // Reset so this test does not poison the shared statics for others.
        UNDERFLOWED[component as usize].store(false, Ordering::Relaxed);
    }

    /// Every component must have a distinct manifest key: a duplicate would
    /// silently drop one component from the published table (a serde_json map
    /// insert overwrites).
    #[test]
    fn component_keys_are_unique_and_all_are_enumerated() {
        let mut keys: Vec<&str> = Component::ALL.iter().map(|c| c.key()).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "duplicate component key");
        assert_eq!(count, COMPONENT_COUNT);
        // ALL must be in discriminant order, because `Mark::components` is
        // indexed positionally and `report_json` zips ALL against it.
        for (index, component) in Component::ALL.iter().enumerate() {
            assert_eq!(*component as usize, index);
        }
    }

    /// Handing a guard on with `std::mem::replace` must TRANSFER the charge,
    /// not duplicate it.
    ///
    /// This is the shape the decoder uses when the hour rows move out of the
    /// cursor and into the stream index. Minting a fresh guard next to the old
    /// one instead would leave both alive until the old owner dropped, and
    /// because `peak` never falls, that transient double count would be
    /// published forever as this component's peak.
    #[test]
    fn replacing_a_guard_transfers_the_charge_without_doubling_the_peak() {
        let component = Component::DecoderFraming;
        let base_peak = peak(component);
        let mut owner = Charge::new(component, 8 << 20);
        let held_before = held(component);

        let handed_on = std::mem::replace(&mut owner, Charge::new(component, 0));
        assert_eq!(
            held(component),
            held_before,
            "a transfer must not change what is held"
        );
        assert!(
            peak(component) <= base_peak.max(held_before),
            "a transfer must not lift the peak above what was ever really held"
        );

        drop(handed_on);
        drop(owner);
        assert_eq!(held(component), held_before - (8 << 20));
    }

    /// The report must publish the residual, not just the table: without it a
    /// reader cannot distinguish "nothing else is held" from "nobody charged".
    #[test]
    fn report_publishes_the_unaccounted_residual() {
        let report = report_json();
        assert!(report.get("unaccounted_bytes").is_some());
        assert!(report["components"]
            .as_object()
            .is_some_and(|map| map.len() == COMPONENT_COUNT));
        for component in Component::ALL {
            assert!(report["components"][component.key()]["scaling"].is_string());
        }
    }
}
