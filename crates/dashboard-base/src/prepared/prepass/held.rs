//! What a value an extract keeps holds ([`Held`]), counted into
//! [`HeldBytes`] for [`super::Extract::bytes`].

use super::HeldBytes;
use crate::prepared::{
    frame_changes::FrameChanges,
    series::{Accumulator, Recording},
};
use replay_data::rows::RowCodec;
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::ViewIdentity};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    sync::Arc,
};

/// A value an extract keeps, counting what its own allocations hold: each
/// collection at its capacity, each B-tree by the nodes its entries take,
/// each shared allocation once however many values share it, and what their
/// elements hold in turn. Its inline size is counted by whatever holds it
/// ([`HeldBytes::value`] for a value held on its own).
pub trait Held {
    fn held(&self, bytes: &mut HeldBytes);
}

/// Entries a B-tree node holds at most.
const TREE_NODE: usize = 11;

/// What a B-tree of `len` entries of `entry` bytes holds, about: nodes split
/// in half when full, so a tree keeps about two of every eleven slots per
/// entry, and never less than one node; each node also holds its parent link,
/// position and length, and the internal nodes above the leaves about a
/// sixth more of each.
fn tree_bytes(len: usize, entry: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let leaves = (2 * len).div_ceil(TREE_NODE).max(1);
    let node = 2 * size_of::<usize>() + TREE_NODE * entry;
    leaves * (node + node.div_ceil(6) + size_of::<usize>() * 2)
}

/// What a hash table with room for `capacity` entries of `entry` bytes holds:
/// its buckets (a power of two at most seven eighths full), a control byte
/// each and one group of control bytes more.
fn table_bytes(capacity: usize, entry: usize) -> usize {
    if capacity == 0 {
        return 0;
    }
    let buckets = (capacity * 8 / 7).max(capacity + 1).next_power_of_two();
    buckets * (entry + 1) + 16
}

impl HeldBytes {
    /// What `value`'s own allocations hold ([`Held`]).
    pub fn heap<T: Held + ?Sized>(&mut self, value: &T) -> &mut Self {
        value.held(self);
        self
    }

    /// `value` held on its own: its inline size and its allocations.
    pub fn value<T: Held>(&mut self, value: &T) -> &mut Self {
        self.add(size_of::<T>());
        self.heap(value)
    }

    /// Whether the allocation at `shared` is not yet counted; counts it so.
    pub fn once<T: ?Sized>(&mut self, shared: *const T) -> bool {
        let address = shared as *const () as usize;
        if self.last == Some(address) {
            return false;
        }
        self.last = Some(address);
        self.seen.insert(address)
    }

    /// The nodes of a B-tree of `len` entries of keys `K` and values `V`.
    pub fn tree<K, V>(&mut self, len: usize) -> &mut Self {
        self.add(tree_bytes(len, size_of::<K>() + size_of::<V>()))
    }

    /// The buckets of a hash table with room for `capacity` entries of keys
    /// `K` and values `V`.
    pub fn table<K, V>(&mut self, capacity: usize) -> &mut Self {
        self.add(table_bytes(capacity, size_of::<K>() + size_of::<V>()))
    }
}

/// Values that own no allocation.
macro_rules! inline {
    ($($type:ty),* $(,)?) => {
        $(impl Held for $type {
            fn held(&self, _: &mut HeldBytes) {}
        })*
    };
}

inline!(
    (),
    bool,
    char,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    str,
    std::time::Duration,
    ViewIdentity,
    dashboard_text::metadata::IdentityTrack,
    crate::products::country_groups::HourSpan,
);

/// Recorded rows: their strings, lists and nested values.
macro_rules! rows {
    ($($row:ty),* $(,)?) => {
        $(impl Held for $row {
            fn held(&self, bytes: &mut HeldBytes) {
                bytes.add(RowCodec::heap_bytes(self));
            }
        })*
    };
}

rows!(
    replay_data::rows::CountryRow,
    replay_data::rows::PoliticsCaptureCoverageRow,
    replay_data::rows::WarRelationRow,
    replay_data::rows::WarOverviewCountryRow,
    replay_data::rows::WarOwnedRelationRow,
);

/// A borrow owns nothing.
impl<T: ?Sized> Held for &T {
    fn held(&self, _: &mut HeldBytes) {}
}

impl Held for String {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.capacity());
    }
}

impl<T: Held> Held for [T] {
    fn held(&self, bytes: &mut HeldBytes) {
        for value in self {
            value.held(bytes);
        }
    }
}

impl<T: Held, const N: usize> Held for [T; N] {
    fn held(&self, bytes: &mut HeldBytes) {
        self.as_slice().held(bytes);
    }
}

impl<T: Held> Held for Vec<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.capacity() * size_of::<T>());
        self.as_slice().held(bytes);
    }
}

impl<T: Held> Held for VecDeque<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.capacity() * size_of::<T>());
        for value in self {
            value.held(bytes);
        }
    }
}

impl<T: Held + ?Sized> Held for Box<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(size_of_val(&**self));
        (**self).held(bytes);
    }
}

/// Shared: its allocation (counts and value) once however many hold it.
impl<T: Held + ?Sized> Held for Arc<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        if bytes.once(Arc::as_ptr(self)) {
            bytes.add(2 * size_of::<usize>() + size_of_val(&**self));
            (**self).held(bytes);
        }
    }
}

impl<T: Held> Held for Option<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        if let Some(value) = self {
            value.held(bytes);
        }
    }
}

impl<T: Held, E: Held> Held for Result<T, E> {
    fn held(&self, bytes: &mut HeldBytes) {
        match self {
            Ok(value) => value.held(bytes),
            Err(error) => error.held(bytes),
        }
    }
}

macro_rules! tuples {
    ($(($($name:ident),+)),* $(,)?) => {
        $(impl<$($name: Held),+> Held for ($($name,)+) {
            #[allow(non_snake_case)]
            fn held(&self, bytes: &mut HeldBytes) {
                let ($($name,)+) = self;
                $($name.held(bytes);)+
            }
        })*
    };
}

tuples!(
    (A),
    (A, B),
    (A, B, C),
    (A, B, C, D),
    (A, B, C, D, E),
    (A, B, C, D, E, F),
);

impl<K: Held, V: Held> Held for BTreeMap<K, V> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.tree::<K, V>(self.len());
        for (key, value) in self {
            key.held(bytes);
            value.held(bytes);
        }
    }
}

impl<T: Held> Held for BTreeSet<T> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.tree::<T, ()>(self.len());
        for value in self {
            value.held(bytes);
        }
    }
}

impl<K: Held, V: Held, S> Held for HashMap<K, V, S> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.table::<K, V>(self.capacity());
        for (key, value) in self {
            key.held(bytes);
            value.held(bytes);
        }
    }
}

impl<T: Held, S> Held for HashSet<T, S> {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.table::<T, ()>(self.capacity());
        for value in self {
            value.held(bytes);
        }
    }
}

/// A table view, once per allocation however many frames share it.
impl Held for SharedView {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.view(self);
    }
}

/// A frame's every table view not yet counted.
impl Held for DecodedFrame {
    fn held(&self, bytes: &mut HeldBytes) {
        for view in &self.tables {
            bytes.view(view);
        }
    }
}

/// Change facts' recorded updates, once.
impl Held for FrameChanges {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.recorded(self);
    }
}

impl Held for Recording {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.bytes());
    }
}

/// An accumulator's batch held for its next flush.
impl Held for Accumulator {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.bytes());
    }
}

impl Held for dashboard_text::metadata::CountryKeys {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.heap_bytes());
    }
}

impl Held for dashboard_text::metadata::LinkTags {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.heap_bytes());
    }
}

impl Held for replay_frame::frame::RecordedFrame {
    fn held(&self, bytes: &mut HeldBytes) {
        self.allocations(|pointer, size| {
            bytes.shared(pointer, size);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_size_shared_views_once_and_keep_their_own_totals() {
        use replay_data::{rows::CountryRow, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut view: SharedView = TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: 1,
            tag: b"SYN".to_vec(),
            ..Default::default()
        }]))
        .into();
        let expected = crate::prepared::query::frames::view_bytes(&view) as u64;
        let before = super::super::sized_views();
        super::super::share_view_sizes(|| {
            for _ in 0..3 {
                let mut held = HeldBytes::default();
                held.view(&view).view(&view);
                assert_eq!(held.total(), expected);
            }
            assert_eq!(super::super::sized_views() - before, 1);
            // Mutating even an otherwise unshared view gives it a distinct
            // allocation while the sizing scope holds its weak identity.
            let TableView::Complete(TableRows::Countries(rows)) = &mut *view else {
                unreachable!()
            };
            rows[0].tag = b"OTHER".to_vec();
            let mut held = HeldBytes::default();
            held.view(&view);
            assert_eq!(
                held.total(),
                crate::prepared::query::frames::view_bytes(&view) as u64
            );
            assert_eq!(super::super::sized_views() - before, 2);
        });
        let mut held = HeldBytes::default();
        held.view(&view);
        assert_eq!(
            super::super::sized_views() - before,
            3,
            "the scope releases its sizes"
        );
    }

    #[test]
    fn a_map_holds_more_with_every_entry_and_its_strings() {
        let held = |entries: usize, length: usize| {
            let map: BTreeMap<String, i64> = (0..entries)
                .map(|entry| (format!("{entry:0length$}"), entry as i64))
                .collect();
            let mut bytes = HeldBytes::default();
            bytes.heap(&map);
            bytes.total()
        };
        assert_eq!(held(0, 8), 0);
        // One entry takes a whole node.
        assert!(held(1, 8) >= (TREE_NODE * size_of::<(String, i64)>()) as u64);
        // Many entries take about two slots each, and their strings.
        assert!(held(1_000, 8) >= 1_000 * (2 * size_of::<(String, i64)>() + 8) as u64);
        assert!(held(1_000, 64) >= held(1_000, 8) + 1_000 * 40);
    }

    #[test]
    fn a_shared_allocation_counts_once() {
        let shared: Arc<[String]> = (0..100).map(|tag| format!("{tag:08}")).collect();
        let one = {
            let mut bytes = HeldBytes::default();
            bytes.heap(&shared);
            bytes.total()
        };
        let many = {
            let mut bytes = HeldBytes::default();
            bytes.heap(&vec![Arc::clone(&shared); 50]);
            bytes.total()
        };
        assert!(one >= 100 * (size_of::<String>() + 8) as u64);
        assert_eq!(many, one + (50 * size_of::<Arc<[String]>>()) as u64);
        // Another allocation of equal strings is counted again.
        let copy: Arc<[String]> = shared.iter().cloned().collect();
        let mut bytes = HeldBytes::default();
        bytes.heap(&shared).heap(&copy);
        assert!(bytes.total() >= one + (100 * (size_of::<String>() + 8)) as u64);
    }

    #[test]
    fn a_hash_table_holds_its_buckets() {
        let mut map = HashMap::<u32, String>::with_capacity(1_000);
        map.insert(1, "x".repeat(100));
        let mut bytes = HeldBytes::default();
        bytes.heap(&map);
        assert!(bytes.total() >= (1_000 * size_of::<(u32, String)>() + 100) as u64);
    }

    /// An hour an extract keeps in a field nothing else counts: its frame and
    /// change facts count their inline size once beside what they own.
    #[test]
    fn a_frame_and_change_facts_held_on_their_own_count_their_inline_size_once() {
        use replay_frame::frame::{FrameDecoder, FrameEncoder};
        let format = replay_data::format::FormatVersion::CURRENT;
        let (mut encoder, mut decoder) = (FrameEncoder::new(), FrameDecoder::new());
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
        let mut kept = None;
        for hour in 100..102 {
            let wire = encoder
                .encode(crate::test_fixtures::scale_frame(1, hour, 50), hour == 100)
                .unwrap();
            let frame = decoder.advance(&wire).unwrap().clone();
            let changes = tracker.observe_updates(&frame, Arc::new(wire), format);
            kept = Some((frame, changes));
        }
        let (frame, changes) = kept.as_ref().unwrap();
        let mut owned = HeldBytes::default();
        for view in &frame.tables {
            owned.view(view);
        }
        owned.heap(changes.recorded().expect("the hour continues the one before"));
        let mut alone = HeldBytes::default();
        alone.value(frame).value(changes);
        assert_eq!(
            alone.total(),
            owned.total() + (size_of::<DecodedFrame>() + size_of::<FrameChanges>()) as u64
        );
        let mut inline = HeldBytes::default();
        inline.frame(frame).changes(changes);
        assert_eq!(inline.total(), owned.total(), "a slot holding them counts the rest");
    }
}
