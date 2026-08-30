//! **Charge before you allocate, release after you drop.** A counter that is
//! moment the number matters (the peak), and a counter released before the
//! is an RAII guard so the release half cannot be forgotten on an early return
//! or an unwind; prefer it to bare [`charge`]/[`release`].
//!     live-heap difference across the `finalize.before_fold_release` /
//!     `finalize.after_fold_release` mark pair, which brackets the call that
//! only at phase boundaries (tens of calls per process lifetime).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(usize)]
pub enum Component {
    SpoolWriter = 0,

    DecoderFraming = 1,
    DecoderHourIndex = 2,
    InflightSnapshots = 3,
    CacheDeltaBase = 4,
    CacheHourEntries = 5,
    ManifestGapLedger = 6,
    ManifestDocument = 7,
    H4dgIndex = 8,
    /// The capture drain's single read buffer, allocated once for the whole
    /// capture window (`CAPTURE_READ_BUFFER_BYTES`). It was a stack array while
    /// it was 64 KiB and went unaccounted; at a mebibyte it is a real fixed
    /// cost and the manifest may not understate it (lane PN, Codex r1 #3).
    CapturePipeReadBuffer = 9,
}

compact_contract::wire_coded_variants! {
    dense Component: usize, all = pub ALL, count = pub COMPONENT_COUNT,
    index = pub(crate) index, label = pub key;

    SpoolWriter = 0 => "spool_writer",
    DecoderFraming = 1 => "decoder_framing",
    DecoderHourIndex = 2 => "decoder_hour_index",
    InflightSnapshots = 3 => "inflight_snapshots",
    CacheDeltaBase = 4 => "cache_delta_base",
    CacheHourEntries = 5 => "cache_hour_entries",
    ManifestGapLedger = 6 => "manifest_gap_ledger",
    ManifestDocument = 7 => "manifest_document",
    H4dgIndex = 8 => "h4dg_index",
    CapturePipeReadBuffer = 9 => "capture_pipe_read_buffer",
}

impl Component {
    pub const fn scaling(self) -> Scaling {
        match self {
            Component::SpoolWriter => Scaling::Fixed,
            Component::DecoderFraming => Scaling::Fixed,
            Component::DecoderHourIndex => Scaling::SessionLength,
            Component::InflightSnapshots => Scaling::GameSize,
            Component::CacheDeltaBase => Scaling::GameSize,
            Component::CacheHourEntries => Scaling::SessionLength,
            Component::ManifestGapLedger => Scaling::SessionLength,
            Component::ManifestDocument => Scaling::SessionLength,
            Component::H4dgIndex => Scaling::SessionLength,
            Component::CapturePipeReadBuffer => Scaling::Fixed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scaling {
    Fixed,
    GameSize,
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

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);

static HELD: [AtomicU64; COMPONENT_COUNT] = [ZERO; COMPONENT_COUNT];
static PEAK: [AtomicU64; COMPONENT_COUNT] = [ZERO; COMPONENT_COUNT];
/// Set when a release would have driven a counter below zero. That can only
/// happen if a release is not paired with its charge, which makes every later
static UNDERFLOWED: [AtomicBool; COMPONENT_COUNT] =
    [const { AtomicBool::new(false) }; COMPONENT_COUNT];

pub fn charge(component: Component, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let index = component.index();
    let after = HELD[index].fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK[index].fetch_max(after, Ordering::Relaxed);
}

/// Release `bytes` from `component`. Call this AFTER the owner is dropped.
pub fn release(component: Component, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let index = component.index();
    // `fetch_update` rather than `fetch_sub`: an unpaired release must be
    let outcome = HELD[index].fetch_update(Ordering::Relaxed, Ordering::Relaxed, |held| {
        Some(held.saturating_sub(bytes))
    });
    if let Ok(previous) = outcome {
        if previous < bytes {
            UNDERFLOWED[index].store(true, Ordering::Relaxed);
        }
    }
}

pub fn resize(component: Component, from_bytes: u64, to_bytes: u64) {
    match to_bytes.cmp(&from_bytes) {
        std::cmp::Ordering::Greater => charge(component, to_bytes - from_bytes),
        std::cmp::Ordering::Less => release(component, from_bytes - to_bytes),
        std::cmp::Ordering::Equal => {}
    }
}

pub fn held(component: Component) -> u64 {
    HELD[component.index()].load(Ordering::Relaxed)
}

pub fn peak(component: Component) -> u64 {
    PEAK[component.index()].load(Ordering::Relaxed)
}

pub fn underflowed(component: Component) -> bool {
    UNDERFLOWED[component.index()].load(Ordering::Relaxed)
}

pub fn total_held() -> u64 {
    Component::ALL
        .iter()
        .map(|component| held(*component))
        .fold(0u64, u64::saturating_add)
}

pub fn json_resident_bytes(value: &serde_json::Value) -> u64 {
    const VALUE: u64 = std::mem::size_of::<serde_json::Value>() as u64;
    const STRING: u64 = std::mem::size_of::<String>() as u64;
    VALUE.saturating_add(json_owned_bytes(value, VALUE, STRING))
}

fn json_owned_bytes(value: &serde_json::Value, value_size: u64, string_size: u64) -> u64 {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 0,
        serde_json::Value::String(text) => text.capacity() as u64,
        serde_json::Value::Array(items) => {
            let slots = (items.capacity() as u64).saturating_mul(value_size);
            items.iter().fold(slots, |total, item| {
                total.saturating_add(json_owned_bytes(item, value_size, string_size))
            })
        }
        serde_json::Value::Object(entries) => entries.iter().fold(0u64, |total, (key, item)| {
            total
                .saturating_add(string_size)
                .saturating_add(key.capacity() as u64)
                .saturating_add(value_size)
                .saturating_add(json_owned_bytes(item, value_size, string_size))
        }),
    }
}

/// An RAII charge: the bytes are charged now and released when this is dropped.
/// same scope -- so the release cannot be skipped by an early return or unwind.
#[derive(Debug)]
pub struct Charge {
    component: Component,
    bytes: u64,
}

impl Charge {
    pub fn new(component: Component, bytes: u64) -> Self {
        charge(component, bytes);
        Self { component, bytes }
    }

    pub fn set(&mut self, bytes: u64) {
        resize(self.component, self.bytes, bytes);
        self.bytes = bytes;
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        release(self.component, self.bytes);
    }
}

pub const MAX_MARKS: usize = 64;

#[derive(Debug, Clone)]
pub struct Mark {
    pub label: &'static str,
    pub private_bytes: Option<u64>,
    pub live_heap_bytes: Option<u64>,
    pub components: [u64; COMPONENT_COUNT],
}

static MARKS: std::sync::Mutex<Vec<Mark>> = std::sync::Mutex::new(Vec::new());
static MARKS_DROPPED: AtomicU64 = AtomicU64::new(0);

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

pub fn marks() -> Vec<Mark> {
    MARKS.lock().map(|marks| marks.clone()).unwrap_or_default()
}

pub fn marks_dropped() -> u64 {
    MARKS_DROPPED.load(Ordering::Relaxed)
}

pub fn report_json() -> serde_json::Value {
    let private_bytes = crate::proc_mem::current_private_bytes();
    let total = total_held();
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

#[cfg(feature = "mem-alloc-trace")]
mod tracking {
    use std::alloc::{GlobalAlloc, Layout};
    use std::sync::atomic::{AtomicI64, Ordering};

    pub(super) static LIVE: AtomicI64 = AtomicI64::new(0);
    pub(super) static PEAK: AtomicI64 = AtomicI64::new(0);

    #[inline]
    fn add(delta: i64) {
        let live = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
        if delta > 0 {
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
    }

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

#[cfg(feature = "mem-alloc-trace")]
pub fn live_heap_bytes() -> Option<u64> {
    Some(tracking::LIVE.load(Ordering::Relaxed).max(0) as u64)
}

#[cfg(feature = "mem-alloc-trace")]
pub fn peak_live_heap_bytes() -> Option<u64> {
    Some(tracking::PEAK.load(Ordering::Relaxed).max(0) as u64)
}

/// A peak is process-lifetime by design, which is what makes it useful and also
#[cfg(feature = "mem-alloc-trace")]
pub fn reset_peak_live_heap() {
    let live = tracking::LIVE.load(Ordering::Relaxed);
    tracking::PEAK.store(live, Ordering::Relaxed);
}

#[cfg(not(feature = "mem-alloc-trace"))]
pub fn reset_peak_live_heap() {}

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
    #[test]
    fn unpaired_release_saturates_and_flags_rather_than_wrapping() {
        let component = Component::CacheHourEntries;
        assert!(!underflowed(component));
        release(component, u64::MAX);
        assert_eq!(held(component), 0);
        assert!(underflowed(component));
        UNDERFLOWED[component.index()].store(false, Ordering::Relaxed);
    }

    #[test]
    fn component_keys_are_unique_and_all_are_enumerated() {
        let mut keys: Vec<&str> = Component::ALL.iter().map(|c| c.key()).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "duplicate component key");
        assert_eq!(count, COMPONENT_COUNT);
        for (index, component) in Component::ALL.iter().enumerate() {
            assert_eq!(*component as usize, index);
        }
    }

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

    #[test]
    fn json_resident_bytes_measures_the_nodes_and_not_the_serialized_length() {
        let mut small = serde_json::Map::new();
        for index in 0..64u32 {
            small.insert(format!("k{index}"), serde_json::json!(index));
        }
        let scalars = serde_json::Value::Object(small);
        let serialized = serde_json::to_vec(&scalars).expect("serialize").len() as u64;
        let resident = json_resident_bytes(&scalars);
        assert!(
            resident > serialized * 2,
            "a table of small scalars is several times larger resident than \
             serialized: resident {resident}, serialized {serialized}"
        );

        let mut reserved: Vec<serde_json::Value> = Vec::with_capacity(1024);
        reserved.push(serde_json::json!(1));
        let reserved_bytes = json_resident_bytes(&serde_json::Value::Array(reserved));
        assert!(
            reserved_bytes >= 1024 * std::mem::size_of::<serde_json::Value>() as u64,
            "array slots are charged by capacity: {reserved_bytes}"
        );

        let payload = "x".repeat(4096);
        let buried = serde_json::json!({ "a": { "b": [{ "c": payload }] } });
        assert!(
            json_resident_bytes(&buried) > 4096,
            "a string buried three levels down is still resident: {}",
            json_resident_bytes(&buried)
        );
        let hollow = serde_json::json!({ "a": { "b": [{ "c": "" }] } });
        assert!(
            json_resident_bytes(&buried) - json_resident_bytes(&hollow) >= 4096,
            "and the whole difference between the two trees is that string"
        );
    }

    #[test]
    fn the_manifest_document_component_is_declared_scaling_and_reachable() {
        assert_eq!(Component::ManifestDocument.key(), "manifest_document");
        assert_eq!(
            Component::ManifestDocument.scaling(),
            Scaling::SessionLength,
            "the manifest grows with the recording; anything else understates \
             the class this component belongs to"
        );
        assert!(
            Component::ALL.contains(&Component::ManifestDocument),
            "a component missing from ALL is missing from every published report"
        );
        assert!(
            report_json()["components"]["manifest_document"]["peak_bytes"].is_u64(),
            "the report must carry a slot for it even before a rewrite has run"
        );
        let base = peak(Component::ManifestDocument);
        {
            let _guard = Charge::new(Component::ManifestDocument, 1 << 20);
            assert!(held(Component::ManifestDocument) >= 1 << 20);
        }
        assert!(peak(Component::ManifestDocument) >= base.max(1 << 20));
    }
}
