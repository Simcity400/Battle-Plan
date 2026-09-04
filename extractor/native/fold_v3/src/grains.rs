//! The grain store: dense slots addressed by a hash index, with the canonical
//! order kept beside them as arrays of slot numbers.
//!
//! # Why this is not a `BTreeMap`
//!
//! The state used to be a `BTreeMap<Path, Grain>` and the per-mutation constant
//! was made of its descents. Lane FB measured them: on the corpus's largest
//! section a descent is ~28 key comparisons over a 105,000-entry map, every one
//! of them a pointer chase into a path's own allocation, and the mutation path
//! took one in [`crate::SectionState::install`] and another in
//! [`crate::HourTxn::commit`]. Interning the member names made each comparison
//! cheap; it could not make them fewer, because a comparison-ordered container
//! has no other way to find an address.
//!
//! Almost every mutation on the corpus lands on an address that ALREADY STANDS
//! -- a division's stats move, its identity does not -- and for those the order
//! is not consulted at all. So the two questions are separated:
//!
//! * **Where is this address?** answered by a hash index, in one probe, whatever
//!   the section holds. This is the question every mutation asks.
//! * **What order do the addresses come out in?** answered by
//!   [`GrainStore::order`], a sorted array of slot numbers. This is the question
//!   the projection, the delta encoding and `check_path`'s neighbour tests ask,
//!   and only a mutation that CHANGES THE ADDRESS SET has to maintain it.
//!
//! # The order is exactly the `BTreeMap`'s
//!
//! `order` is sorted by `<[Seg]>::cmp` -- the same total order the map was keyed
//! on, over the same segments. Nothing else decides where an entry goes. The
//! hash index is an accelerator and never an authority: every probe confirms its
//! candidate with the same `==` (which is `Seg::cmp(..).is_eq()`, i.e. ADDRESS
//! equality) that the map used, so a hash collision costs a comparison and
//! cannot produce a wrong answer.
//!
//! # Insertions, removals, and why there are two arrays
//!
//! Maintaining one sorted array by memmove would be O(n) per inserted address.
//! That is free when a section is loaded in ascending order -- which is how a
//! whole-section assignment arrives, so the first hour costs nothing -- and it
//! is a 420 KB memmove per scattered insertion on a 105,000-grain section.
//!
//! So new addresses land in a second sorted array, [`GrainStore::pending`], and
//! the two are merged when it grows past a fraction of the state. Every ordered
//! read consults both, and because the merge is O(n) against a `pending` that is
//! O(n/16) the amortized cost of an insertion is a handful of `u32` copies.
//!
//! A removal cannot simply vanish from an array without the same memmove, so the
//! slot is TOMBSTONED: its grain goes, its path stays, and the entry keeps the
//! array sorted until the next merge drops it. Ordered reads skip tombstones.
//!
//! # Slot reuse
//!
//! A tombstoned slot is NOT recycled while an array still names it -- it would
//! come back holding a different path, and the array it sits in would no longer
//! be sorted. Slots return to [`GrainStore::free`] only inside
//! [`GrainStore::compact`], which is also where the arrays stop naming them. A
//! grain removed and reinstalled inside one hour therefore gets a DIFFERENT slot
//! number, and nothing outside this module holds a slot number across a call.

use crate::{Grain, Path, Seg};

/// Bucket value for a bucket that has never held anything. Slot numbers are
/// stored biased by one so that zero can mean this.
const EMPTY: u32 = 0;
/// Bucket value for a bucket whose entry was removed. A linear probe must walk
/// through it, so it cannot be reset to [`EMPTY`] until the table is rebuilt.
const TOMB: u32 = u32::MAX;

/// One dense slot: an address, its hash, and the grain standing there.
///
/// `grain` is `None` in two different situations that this module deliberately
/// keeps apart by which list names the slot: a TOMBSTONE is still named by
/// `order` or `pending` and keeps its `path` so those stay sorted; a FREE slot
/// is named only by `free` and holds an empty path.
struct SlotEntry {
    path: Path,
    /// [`hash_path`] of the address, cached so a rehash and a removal do not
    /// have to walk the path again.
    hash: u64,
    grain: Option<Grain>,
}

pub(crate) struct GrainStore {
    slots: Vec<SlotEntry>,
    /// Slots named by nothing, ready to be reused. See the module docs on why a
    /// tombstone does not go straight here.
    free: Vec<u32>,
    /// The canonical order, as of the last [`Self::compact`], plus tombstones.
    order: Vec<u32>,
    /// Addresses installed since then, in the same order, plus tombstones.
    pending: Vec<u32>,
    /// Open-addressed, linearly probed, power-of-two sized. Holds `slot + 1`,
    /// [`EMPTY`] or [`TOMB`]. Empty until the first insertion.
    buckets: Vec<u32>,
    /// Buckets that are not [`EMPTY`] -- live entries and index tombstones
    /// together, which is what the probe length depends on and therefore what
    /// the growth test reads.
    filled: usize,
    /// Grains the store holds. This is `len()`.
    live: usize,
    /// Tombstoned slots still named by `order` or `pending`.
    dead: usize,
}

/// The first term of both merge thresholds: below this many pending or dead
/// entries a section is small enough that neither an ordered read nor a merge
/// is worth thinking about.
const MERGE_FLOOR: usize = 64;

impl GrainStore {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            order: Vec::new(),
            pending: Vec::new(),
            buckets: Vec::new(),
            filled: 0,
            live: 0,
            dead: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.live
    }

    /// `(rows, live, tombstoned, free)`.
    ///
    /// Test-only, and it exists because nothing else can see a reclamation
    /// leak: a `compact` that stopped returning its tombstoned slots to `free`
    /// would leave the order, the projection, the state root and the replica
    /// all correct, and only these numbers would show it.
    ///
    /// The four are published together because the fact worth asserting is the
    /// IDENTITY between them -- every row is live, tombstoned or free, and
    /// nothing else. A row that is none of the three is orphaned: it can never
    /// be handed out again and it holds its slot forever. A test that watched
    /// only the row count would miss a leak that stops after one cohort, which
    /// this cannot.
    #[cfg(test)]
    pub(crate) fn accounting(&self) -> (usize, usize, usize, usize) {
        (self.slots.len(), self.live, self.dead, self.free.len())
    }

    /// Drop everything, releasing the memory. Used by the poison path, which
    /// must leave nothing to serve.
    pub(crate) fn clear(&mut self) {
        self.slots = Vec::new();
        self.free = Vec::new();
        self.order = Vec::new();
        self.pending = Vec::new();
        self.buckets = Vec::new();
        self.filled = 0;
        self.live = 0;
        self.dead = 0;
    }

    fn path_at(&self, slot: u32) -> &[Seg] {
        &self.slots[slot as usize].path
    }

    fn is_live(&self, slot: u32) -> bool {
        self.slots[slot as usize].grain.is_some()
    }

    pub(crate) fn path_of(&self, slot: u32) -> &Path {
        &self.slots[slot as usize].path
    }

    pub(crate) fn grain_of(&self, slot: u32) -> &Grain {
        self.slots[slot as usize]
            .grain
            .as_ref()
            .expect("a slot handed out by the index holds a grain")
    }

    // -- the hash index ------------------------------------------------------

    /// The slot standing at `path`, or `None`.
    ///
    /// One probe sequence. The candidate is confirmed by the SAME comparison the
    /// `BTreeMap` used -- `[Seg]`'s `PartialEq`, which is `Seg::cmp(..).is_eq()`
    /// and therefore address equality, not spelling equality -- so an address
    /// stored under a different escaped spelling is still found here, exactly as
    /// `BTreeMap::get_key_value` found it.
    pub(crate) fn get_slot(&self, path: &[Seg]) -> Option<u32> {
        if self.buckets.is_empty() {
            return None;
        }
        let hash = hash_path(path);
        let mask = self.buckets.len() - 1;
        let mut probe = (hash as usize) & mask;
        loop {
            let bucket = self.buckets[probe];
            if bucket == EMPTY {
                return None;
            }
            if bucket != TOMB {
                let slot = bucket - 1;
                let entry = &self.slots[slot as usize];
                // The cached hash first: a mismatch rejects a candidate without
                // touching its path at all, which is the whole reason it is
                // cached on the slot.
                if entry.hash == hash && entry.path.as_slice() == path {
                    return Some(slot);
                }
            }
            probe = (probe + 1) & mask;
        }
    }

    fn index_insert(&mut self, hash: u64, slot: u32) {
        // A table that is never allowed to fill has a probe that always
        // terminates: the growth test below keeps at least a quarter of the
        // buckets EMPTY.
        if (self.filled + 1) * 4 >= self.buckets.len() * 3 {
            self.rehash();
        }
        let mask = self.buckets.len() - 1;
        let mut probe = (hash as usize) & mask;
        let mut first_tomb: Option<usize> = None;
        loop {
            let bucket = self.buckets[probe];
            if bucket == EMPTY {
                match first_tomb {
                    // A tombstone reused is not a new non-empty bucket.
                    Some(at) => self.buckets[at] = slot + 1,
                    None => {
                        self.buckets[probe] = slot + 1;
                        self.filled += 1;
                    }
                }
                return;
            }
            if bucket == TOMB && first_tomb.is_none() {
                first_tomb = Some(probe);
            }
            probe = (probe + 1) & mask;
        }
    }

    fn index_remove(&mut self, hash: u64, slot: u32) {
        let mask = self.buckets.len() - 1;
        let mut probe = (hash as usize) & mask;
        loop {
            let bucket = self.buckets[probe];
            assert!(bucket != EMPTY, "an indexed slot is not in the index");
            if bucket == slot + 1 {
                // Not EMPTY: a probe for some other address may have walked
                // through this bucket, and emptying it would strand that
                // address behind a gap it does not expect.
                self.buckets[probe] = TOMB;
                return;
            }
            probe = (probe + 1) & mask;
        }
    }

    /// Rebuild the index from the live slots, which is also what drops its
    /// tombstones.
    fn rehash(&mut self) {
        let wanted = ((self.live + 1) * 4).next_power_of_two().max(16);
        self.buckets.clear();
        self.buckets.resize(wanted, EMPTY);
        self.filled = 0;
        let mask = wanted - 1;
        for index in 0..self.slots.len() {
            if self.slots[index].grain.is_none() {
                continue;
            }
            let hash = self.slots[index].hash;
            let mut probe = (hash as usize) & mask;
            while self.buckets[probe] != EMPTY {
                probe = (probe + 1) & mask;
            }
            self.buckets[probe] = index as u32 + 1;
            self.filled += 1;
        }
    }

    // -- mutation ------------------------------------------------------------

    /// Install a grain at an address the store does NOT hold.
    ///
    /// The caller has already probed: `SectionState::install` looks the address
    /// up once and everything below is decided from that one probe, so this
    /// must not probe again.
    pub(crate) fn insert(&mut self, path: Path, grain: Grain) {
        debug_assert!(
            self.get_slot(&path).is_none(),
            "insert was handed an address the store already holds"
        );
        let hash = hash_path(&path);
        // Where the new address goes in `pending`, computed BEFORE the path is
        // moved into its slot. Ascending arrivals -- which is how a whole
        // section assignment presents its grains -- land at the end, so a bulk
        // load moves nothing.
        let at = self.pending_position(&path);
        // The slot is prepared EMPTY and filled after it is indexed.
        //
        // `index_insert` may grow the table, and growing rebuilds it from the
        // live slots. A slot that already held its grain would be indexed by
        // that rebuild and then indexed a second time by the insertion that
        // triggered it -- two buckets for one slot, of which a later removal
        // clears one. The bucket left behind names a slot with no grain in it,
        // and the next probe that walked into it would hand that slot out.
        let slot = match self.free.pop() {
            Some(slot) => {
                let entry = &mut self.slots[slot as usize];
                entry.path = path;
                entry.hash = hash;
                debug_assert!(entry.grain.is_none(), "a free slot still holds a grain");
                slot
            }
            None => {
                let slot = u32::try_from(self.slots.len())
                    .expect("a section cannot hold four billion grains");
                assert!(slot < TOMB - 1, "slot number space exhausted");
                self.slots.push(SlotEntry {
                    path,
                    hash,
                    grain: None,
                });
                slot
            }
        };
        self.live += 1;
        self.index_insert(hash, slot);
        self.slots[slot as usize].grain = Some(grain);
        self.pending.insert(at, slot);
        self.maybe_compact();
    }

    /// Replace the grain standing in `slot`, and optionally the SPELLING of the
    /// address it stands at.
    ///
    /// A respelling does not move the entry and does not change its hash: both
    /// are functions of the address, and a respelling is by definition the same
    /// address written differently. That is why this can overwrite in place
    /// where the `BTreeMap` had to remove and reinsert -- `BTreeMap::insert`
    /// keeps the ORIGINAL key when the new one compares equal, and this store
    /// has no such rule to work around.
    pub(crate) fn replace(&mut self, slot: u32, respelling: Option<Path>, grain: Grain) {
        let entry = &mut self.slots[slot as usize];
        debug_assert!(entry.grain.is_some(), "replace was handed a dead slot");
        if let Some(path) = respelling {
            debug_assert_eq!(
                hash_path(&path),
                entry.hash,
                "a respelling changed the address hash, so it was not a respelling"
            );
            debug_assert!(
                path.as_slice() == entry.path.as_slice(),
                "a respelling changed the address, so it was not a respelling"
            );
            entry.path = path;
        }
        entry.grain = Some(grain);
    }

    /// Retire the grain in `slot`. See the module docs on why the slot is not
    /// recycled here.
    pub(crate) fn remove(&mut self, slot: u32) {
        let hash = self.slots[slot as usize].hash;
        debug_assert!(
            self.slots[slot as usize].grain.is_some(),
            "remove was handed a dead slot"
        );
        self.index_remove(hash, slot);
        self.slots[slot as usize].grain = None;
        self.live -= 1;
        self.dead += 1;
        self.maybe_compact();
    }

    fn pending_position(&self, key: &[Seg]) -> usize {
        self.pending
            .partition_point(|&slot| self.path_at(slot) < key)
    }

    fn maybe_compact(&mut self) {
        // Both thresholds are proportional to the state, so a merge -- which
        // costs one path comparison per live entry -- is amortized to a handful
        // of `u32` copies per address-set change. The measured churn is what
        // sets the constant: on capture t30 the busiest section installs and
        // retires ~1,925 addresses an hour against 58,961 live, so a sixteenth
        // is roughly one merge every two game hours.
        if self.pending.len() >= MERGE_FLOOR + self.live / 16
            || self.dead >= MERGE_FLOOR + self.live / 16
        {
            self.compact();
        }
    }

    /// Merge `pending` into `order`, drop the tombstones, and release their
    /// slots.
    ///
    /// The result is the live addresses in ascending order, which is what
    /// `order` alone means when `pending` is empty.
    pub(crate) fn compact(&mut self) {
        if self.pending.is_empty() && self.dead == 0 {
            return;
        }
        let order = std::mem::take(&mut self.order);
        let mut pending = std::mem::take(&mut self.pending);
        let mut merged: Vec<u32> = Vec::with_capacity(self.live);
        let mut left = 0usize;
        let mut right = 0usize;
        loop {
            while left < order.len() && !self.is_live(order[left]) {
                left += 1;
            }
            while right < pending.len() && !self.is_live(pending[right]) {
                right += 1;
            }
            let taken = match (left < order.len(), right < pending.len()) {
                (true, true) => {
                    // The two live sets are disjoint -- an address stands in one
                    // slot and one array -- so this never has to break a tie.
                    if self.path_at(order[left]) <= self.path_at(pending[right]) {
                        let slot = order[left];
                        left += 1;
                        slot
                    } else {
                        let slot = pending[right];
                        right += 1;
                        slot
                    }
                }
                (true, false) => {
                    let slot = order[left];
                    left += 1;
                    slot
                }
                (false, true) => {
                    let slot = pending[right];
                    right += 1;
                    slot
                }
                (false, false) => break,
            };
            merged.push(taken);
        }
        debug_assert_eq!(merged.len(), self.live, "the merge lost or gained a grain");
        // Now that no array names them, the tombstoned slots can be recycled --
        // and their paths released, because nothing orders by them any more.
        // Each slot is named by exactly one of the two arrays, so no slot is
        // freed twice.
        for &slot in order.iter().chain(pending.iter()) {
            if !self.is_live(slot) {
                self.slots[slot as usize].path = Path::new();
                self.free.push(slot);
            }
        }
        self.order = merged;
        pending.clear();
        self.pending = pending;
        self.dead = 0;
    }

    // -- ordered reads -------------------------------------------------------

    /// Every live entry, ascending. The order `BTreeMap`'s iterator produced.
    pub(crate) fn iter(&self) -> StoreIter<'_> {
        StoreIter {
            store: self,
            left: 0,
            right: 0,
        }
    }

    /// The live addresses either side of `path`: the last strictly BEFORE it and
    /// the first AT OR AFTER it -- `range((Unbounded, Excluded(path))).next_back()`
    /// and `range((Included(path), Unbounded)).next()`, together.
    ///
    /// Together because they are the same question. Both are decided by ONE
    /// boundary per array -- the index the address would occupy -- and
    /// `check_path` needs both of them plus the second one again for its
    /// descendant test, which as three separate calls was six binary searches
    /// per newly addressed grain. Each of those searches is a chain of random
    /// reads into the slot table, which is the expensive kind, and
    /// `check_path` runs on exactly the mutations that a churning section is
    /// made of.
    pub(crate) fn neighbours(&self, path: &[Seg]) -> (Option<&Path>, Option<&Path>) {
        let (order_before, order_after) = self.split_at(&self.order, path);
        let (pending_before, pending_after) = self.split_at(&self.pending, path);
        let before = match (order_before, pending_before) {
            (Some(left), Some(right)) => Some(if self.path_at(left) >= self.path_at(right) {
                left
            } else {
                right
            }),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        };
        let after = match (order_after, pending_after) {
            (Some(left), Some(right)) => Some(if self.path_at(left) <= self.path_at(right) {
                left
            } else {
                right
            }),
            (Some(only), None) | (None, Some(only)) => Some(only),
            (None, None) => None,
        };
        (
            before.map(|slot| self.path_of(slot)),
            after.map(|slot| self.path_of(slot)),
        )
    }

    /// The first live address at or after `path`.
    pub(crate) fn first_at_or_after(&self, path: &[Seg]) -> Option<&Path> {
        self.neighbours(path).1
    }

    /// The last live address strictly before `path`.
    #[cfg(test)]
    pub(crate) fn last_before(&self, path: &[Seg]) -> Option<&Path> {
        self.neighbours(path).0
    }

    /// One binary search, then a scan each way past the tombstones it lands
    /// among.
    ///
    /// `partition_point` is monotone over an array that holds tombstones,
    /// because a tombstone keeps its path until the next merge -- that is what
    /// it is for. The scans skip them, and how far they can run is what the
    /// merge threshold bounds.
    fn split_at(&self, array: &[u32], path: &[Seg]) -> (Option<u32>, Option<u32>) {
        let at = array.partition_point(|&slot| self.path_at(slot) < path);
        let mut back = at;
        let mut before = None;
        while back > 0 {
            back -= 1;
            if self.is_live(array[back]) {
                before = Some(array[back]);
                break;
            }
        }
        let mut forward = at;
        let mut after = None;
        while forward < array.len() {
            if self.is_live(array[forward]) {
                after = Some(array[forward]);
                break;
            }
            forward += 1;
        }
        (before, after)
    }

    // -- accounting ----------------------------------------------------------

    /// The paths' own segment arrays, for every slot that still holds one --
    /// which is every live grain and every tombstone waiting for the next merge.
    pub(crate) fn retained_path_bytes(&self) -> u64 {
        self.slots
            .iter()
            .map(|entry| (entry.path.len() * std::mem::size_of::<Seg>()) as u64)
            .sum()
    }

    /// What the store holds BESIDE the grains and their paths: the index, the
    /// two order arrays, the free list, and the slack in the slot table.
    ///
    /// The slot table's OCCUPIED rows are not counted here. A live row is what
    /// the driver's per-grain overhead constant charges per grain -- a `Grain`,
    /// a `Path` header and the cached hash -- and counting it twice would
    /// invent memory. What is counted is every row the state is holding that no
    /// grain is standing in: reserved-but-unused capacity, tombstones and free
    /// slots. That is real retained memory and nothing else reports it.
    pub(crate) fn retained_overhead_bytes(&self) -> u64 {
        // 128 bytes on a 64-bit target: a `Path` header, the cached hash and a
        // `Grain`.
        const _: () = assert!(std::mem::size_of::<SlotEntry>() == 128);
        let row = std::mem::size_of::<SlotEntry>();
        ((self.buckets.capacity() + self.order.capacity() + self.pending.capacity()
            + self.free.capacity())
            * std::mem::size_of::<u32>()
            + self.slots.capacity().saturating_sub(self.live) * row) as u64
    }
}

/// The live entries of `order` and `pending`, merged.
pub(crate) struct StoreIter<'a> {
    store: &'a GrainStore,
    left: usize,
    right: usize,
}

impl<'a> Iterator for StoreIter<'a> {
    type Item = (&'a Path, &'a Grain);

    fn next(&mut self) -> Option<Self::Item> {
        let store = self.store;
        while self.left < store.order.len() && !store.is_live(store.order[self.left]) {
            self.left += 1;
        }
        while self.right < store.pending.len() && !store.is_live(store.pending[self.right]) {
            self.right += 1;
        }
        let slot = match (
            self.left < store.order.len(),
            self.right < store.pending.len(),
        ) {
            (true, true) => {
                if store.path_at(store.order[self.left]) <= store.path_at(store.pending[self.right])
                {
                    let slot = store.order[self.left];
                    self.left += 1;
                    slot
                } else {
                    let slot = store.pending[self.right];
                    self.right += 1;
                    slot
                }
            }
            (true, false) => {
                let slot = store.order[self.left];
                self.left += 1;
                slot
            }
            (false, true) => {
                let slot = store.pending[self.right];
                self.right += 1;
                slot
            }
            (false, false) => return None,
        };
        Some((store.path_of(slot), store.grain_of(slot)))
    }
}

/// A hash of the ADDRESS `path` denotes, not of the way it is spelled.
///
/// This is the one property the index rests on: two paths that compare EQUAL
/// must hash equal, or an address stored under one escaped spelling would not
/// be found under another and the store would hold it twice.
///
/// It holds because a segment's hash is taken over
/// [`crate::intern::KeyRef::order_hash`] -- the hash of the name's ORDERING
/// TEXT -- and `KeyRef::cmp_key` says two names are equal exactly when their
/// ordering texts are equal:
///
/// * two plain names, or two escaped names that decode, or one of each, are
///   compared by ordering text directly;
/// * where either name's escape does not decode the comparison falls back to the
///   raw bodies -- and an undecodable name's ordering text IS its body, as is a
///   plain name's, so equal bodies are equal ordering texts there too. (Equal
///   bodies are in fact the same interned entry, so this case is equality by
///   pointer.)
///
/// The converse -- equal ordering texts implying equal addresses -- is NOT
/// claimed and is not needed: a collision costs the probe one `==`, which is the
/// authority.
fn hash_path(path: &[Seg]) -> u64 {
    // FNV-1a's constants over a per-segment word. The mixing has to be
    // `wrapping_*`: this workspace builds release with `overflow-checks = true`.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    hash ^= path.len() as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    for seg in path {
        let word = match seg {
            Seg::Key(name) => name.order_hash(),
            // Domain-separated from a name's hash so that an ordinal and a name
            // do not systematically share buckets. They can never compare equal
            // -- `Seg::cmp` puts every name before every ordinal -- so this is
            // about probe length and nothing else.
            Seg::Index(index) => (u64::from(*index) ^ 0x9e37_79b9_7f4a_7c15)
                .wrapping_mul(0xd6e8_feb8_6659_fd93),
        };
        hash ^= word;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        hash ^= hash >> 29;
    }
    hash
}
