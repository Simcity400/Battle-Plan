//! per-interval bookkeeping (ownership, annotation, scheduling, blanket
//!    it below today's, since the executor put allocation and ownership

use std::cell::Cell;

use crate::snapshot_recipes::SnapshotMemory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum D81Family {
    RailHeader = 0,
    PairBlob = 1,
    MapRecordPtr = 2,
    MapAnchors = 3,
    RecordKey = 4,
    SourceProvince = 5,
    SourceSlotClose = 6,
    MapSlotClose = 7,
    ActiveClose = 8,
}

pub const D81_FAMILY_COUNT: usize = 9;

#[derive(Clone, Copy, Debug, Default)]
struct FamilyImages {
    opening: u8,
    closing: u8,
}

const fn family_images(family: D81Family) -> FamilyImages {
    match family {
        D81Family::RailHeader => FamilyImages {
            opening: 2,
            closing: 2,
        },
        D81Family::PairBlob => FamilyImages {
            opening: 2,
            closing: 0,
        },
        D81Family::MapRecordPtr => FamilyImages {
            opening: 2,
            closing: 2,
        },
        D81Family::MapAnchors => FamilyImages {
            opening: 2,
            closing: 0,
        },
        D81Family::RecordKey => FamilyImages {
            opening: 2,
            closing: 0,
        },
        D81Family::SourceProvince => FamilyImages {
            opening: 2,
            closing: 0,
        },
        D81Family::SourceSlotClose => FamilyImages {
            opening: 0,
            closing: 2,
        },
        D81Family::MapSlotClose => FamilyImages {
            opening: 0,
            closing: 2,
        },
        D81Family::ActiveClose => FamilyImages {
            opening: 0,
            closing: 3,
        },
    }
}

const ST_O: u8 = 1 << 0;
const ST_S1: u8 = 1 << 1;
const ST_C: u8 = 1 << 2;
const ST_CS1: u8 = 1 << 3;
const ST_CS2: u8 = 1 << 4;
const ST_DEAD: u8 = 1 << 7;

#[derive(Default)]
pub struct FamilyStore {
    starts: Vec<u64>,
    lens: Vec<u32>,
    offsets: Vec<u32>,
    status: Vec<u8>,
    slabs: [Vec<u8>; 3],
    images: FamilyImages,
}

impl FamilyStore {
    fn new(images: FamilyImages) -> Self {
        Self {
            images,
            ..Self::default()
        }
    }

    fn slab(&self, ordinal: usize) -> &[u8] {
        &self.slabs[ordinal]
    }

    pub fn window_count(&self) -> usize {
        self.starts.len()
    }

    pub fn total_bytes(&self) -> u64 {
        let per_image = self.offsets.last().copied().unwrap_or(0) as u64;
        let image_count = (self.images.opening as u64).saturating_add(self.images.closing as u64);
        per_image.saturating_mul(image_count)
    }

    /// ownership map deduplicated), and sizes the slabs.
    fn push(&mut self, start: u64, len: u32) {
        self.starts.push(start);
        self.lens.push(len);
    }

    fn seal(&mut self) {
        let mut pairs: Vec<(u64, u32)> = self
            .starts
            .iter()
            .copied()
            .zip(self.lens.iter().copied())
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        self.starts = pairs.iter().map(|&(start, _)| start).collect();
        self.lens = pairs.iter().map(|&(_, len)| len).collect();
        self.offsets = Vec::with_capacity(self.starts.len() + 1);
        let mut offset = 0u32;
        self.offsets.push(0);
        for &len in &self.lens {
            offset = offset.saturating_add(len);
            self.offsets.push(offset);
        }
        self.status = vec![0; self.starts.len()];
        let side_images = if self.images.opening > 0 {
            self.images.opening
        } else {
            self.images.closing
        };
        for slab in self.slabs.iter_mut().take(usize::from(side_images)) {
            *slab = vec![0; offset as usize];
        }
    }

    fn copy_side(
        &mut self,
        image_count: usize,
        first_bit: u8,
        copy: &mut dyn FnMut(u64, &mut [u8]) -> bool,
        calls: &Cell<u64>,
    ) {
        for index in 0..self.starts.len() {
            let start = self.starts[index];
            let from = self.offsets[index] as usize;
            let to = from + self.lens[index] as usize;
            for image in 0..image_count {
                let slab = &mut self.slabs[image];
                calls.set(calls.get().saturating_add(1));
                if copy(start, &mut slab[from..to]) {
                    self.status[index] |= first_bit << image;
                } else if image == 0 {
                    break;
                }
            }
        }
    }

    #[cfg(test)]
    fn locate_with_dead(&self, addr: u64, len: u32) -> (Option<usize>, bool) {
        let position = self.starts.partition_point(|&start| start <= addr);
        let mut dead = false;
        let mut live = None;
        for index in (0..position).rev() {
            let start = self.starts[index];
            let width = self.lens[index] as u64;
            let contains =
                addr >= start && addr.saturating_add(u64::from(len)) <= start.saturating_add(width);
            if !contains {
                continue;
            }
            if self.status[index] & ST_DEAD != 0 {
                dead = true;
            } else if live.is_none() {
                live = Some(index);
            }
        }
        (live, dead)
    }

    fn retire_intersecting(&mut self, spans: &[(u64, u64)]) -> u32 {
        let mut retired = 0u32;
        for index in 0..self.starts.len() {
            let start = self.starts[index];
            let end = start.saturating_add(u64::from(self.lens[index]));
            if spans
                .iter()
                .any(|&(span_start, span_end)| start < span_end && span_start < end)
            {
                self.status[index] |= ST_DEAD;
                retired = retired.saturating_add(1);
            }
        }
        retired
    }

    fn serve(&self, index: usize, addr: u64, len: u32, ordinal: usize) -> &[u8] {
        let from = self.offsets[index] as usize + (addr - self.starts[index]) as usize;
        &self.slab(ordinal)[from..from + len as usize]
    }
}

const STORE_HEADER_CLOSE: u8 = D81_FAMILY_COUNT as u8;
const STORE_RECORD_PTR_CLOSE: u8 = D81_FAMILY_COUNT as u8 + 1;
const STORE_COUNT: usize = D81_FAMILY_COUNT + 2;

/// WHICH FAMILY EACH STORE BELONGS TO.
///
/// The last two stores are not families of their own: they hold the CLOSING
/// side of `RailHeader` and of `MapRecordPtr`, at the very same addresses as
/// those families' opening windows. A read that lands on `header_close` and on
/// `RailHeader` has therefore found one family twice, not two families — and
/// that distinction is the whole difference between a served read and an
/// `Aliased` refusal, which is why it is stated once here rather than spelled
/// out again at each site that needs it.
const STORE_FAMILY: [u8; STORE_COUNT] = [
    D81Family::RailHeader as u8,
    D81Family::PairBlob as u8,
    D81Family::MapRecordPtr as u8,
    D81Family::MapAnchors as u8,
    D81Family::RecordKey as u8,
    D81Family::SourceProvince as u8,
    D81Family::SourceSlotClose as u8,
    D81Family::MapSlotClose as u8,
    D81Family::ActiveClose as u8,
    D81Family::RailHeader as u8,
    D81Family::MapRecordPtr as u8,
];

/// WHICH WINDOWS ANSWER ONE READ, decided before the image is chosen.
///
/// [`D81VectorCapture::read`] used to decide this inline, as an
/// `[Option<usize>; STORE_COUNT]` filled by the merged-index scan and then
/// folded by a nine-arm family loop. Naming the verdict makes it CACHEABLE,
/// which is what this type is for: the walk asks the same `(addr, len)` twice
/// for all but a handful of its reads — every `_stable` pair is two reads of
/// one address at two images, and so is every `header()` window and every
/// doubled pair-blob read — and the second ask is a pure repeat of a decision
/// the first already took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolution {
    NotCovered,
    Aliased,
    Hit {
        family: u8,
        /// Index into the family's opening store, or [`NO_WINDOW`].
        opening: u32,
        /// Index into that family's closing store, or [`NO_WINDOW`].
        closing: u32,
    },
}

const NO_WINDOW: u32 = u32::MAX;

/// One remembered GEOMETRY: which entries of the merged index contain a given
/// span. Not the verdict — the verdict is recomputed from these on every read.
///
/// THE KEY IS THE EXACT `(addr, len)` PAIR and not a containing window,
/// because the covering set is a function of the whole asked span: two
/// addresses inside one window can still be covered by different sets when a
/// second window ends between them, and a memo keyed on the window would
/// answer the second read with the first read's covering set.
///
/// WHAT IT DELIBERATELY DOES NOT REMEMBER, and why the type holds positions
/// rather than a `Resolution`. A window can be RETIRED after a read has
/// already been served from it — `a_window_retired_after_the_first_read_
/// poisons_from_the_next_read` pins exactly that — and a retirement turns a
/// served span into `Aliased`. Geometry cannot change after `seal`; status
/// can. So the cache holds the half that is frozen and pays a status load per
/// containing window, which is one or two byte loads, for the half that is
/// not.
#[derive(Clone, Copy, Debug)]
struct GeometryMemo {
    addr: u64,
    len: u32,
    /// How many of `positions` are filled, or [`TOO_MANY_VISITS`] when the
    /// span was contained by more windows than this entry can hold. Such a
    /// span is resolved by the full scan every time rather than partly from a
    /// stale record.
    visits: u8,
    positions: [u32; MEMO_VISITS],
}

/// Containing windows one memo entry can hold. One is the common case and two
/// is the most a family with a closing store can produce, so three is the
/// first count that is not a bet on the shape.
const MEMO_VISITS: usize = 3;

const TOO_MANY_VISITS: u8 = u8::MAX;

/// Ways in the direct-mapped resolution cache. Direct-mapped rather than
/// associative so a MISS costs one comparison; four ways rather than one so
/// that the walk's interleaved reads — a header window, then the pair blob it
/// names, then the header window again for the closing image — do not evict
/// each other.
const MEMO_WAYS: usize = 4;

/// Which resolution form a test wants. Production has no switch: `cfg(test)`
/// deletes this and every branch that reads it, so the shipped read path is
/// the fast form and nothing else.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arm {
    Fast,
    Legacy,
    /// Take both and assert they agree. The differential proof.
    Compare,
}

#[cfg(test)]
thread_local! {
    static RESOLUTION_ARM: Cell<Arm> = const { Cell::new(Arm::Fast) };
}

#[cfg(test)]
fn resolution_arm() -> Arm {
    RESOLUTION_ARM.with(|arm| arm.get())
}

/// Select the arm for this thread and return the one that was set, so a caller
/// can restore it. Thread-local rather than global because the test harness
/// runs tests in parallel and an arm is a property of one walk.
#[cfg(test)]
pub fn set_resolution_arm(arm: Arm) -> Arm {
    RESOLUTION_ARM.with(|slot| slot.replace(arm))
}

struct MergedWindowIndex {
    starts: Vec<u64>,
    ends: Vec<u64>,
    prefix_max_end: Vec<u64>,
    store: Vec<u8>,
    index: Vec<u32>,
}

impl MergedWindowIndex {
    fn build(stores: [&FamilyStore; STORE_COUNT]) -> Self {
        let total: usize = stores.iter().map(|store| store.starts.len()).sum();
        let mut entries: Vec<(u64, u8, u32)> = Vec::with_capacity(total);
        for (store_id, store) in stores.iter().enumerate() {
            for (index, &start) in store.starts.iter().enumerate() {
                entries.push((start, store_id as u8, index as u32));
            }
        }
        entries.sort_unstable();
        let mut merged = Self {
            starts: Vec::with_capacity(total),
            ends: Vec::with_capacity(total),
            prefix_max_end: Vec::with_capacity(total),
            store: Vec::with_capacity(total),
            index: Vec::with_capacity(total),
        };
        let mut running_max = 0u64;
        for (start, store_id, index) in entries {
            let len = stores[usize::from(store_id)].lens[index as usize];
            let end = start.saturating_add(u64::from(len));
            running_max = running_max.max(end);
            merged.starts.push(start);
            merged.ends.push(end);
            merged.prefix_max_end.push(running_max);
            merged.store.push(store_id);
            merged.index.push(index);
        }
        merged
    }

    fn for_each_containing(&self, addr: u64, len: u32, mut visit: impl FnMut(u8, usize)) {
        self.for_each_containing_position(addr, len, |position| {
            visit(self.store[position], self.index[position] as usize)
        });
    }

    /// The same scan, reporting the POSITION in this index rather than the
    /// window it names.
    ///
    /// A position is the one part of a resolution that cannot go stale: the
    /// index is built once, from `starts` and `lens` that `seal` has already
    /// frozen, and nothing afterwards inserts, removes or moves an entry. That
    /// is what makes positions safe to remember when the window's STATUS —
    /// which retirement does change, and after reads have begun — is not.
    fn for_each_containing_position(&self, addr: u64, len: u32, mut visit: impl FnMut(usize)) {
        let want_end = addr.saturating_add(u64::from(len));
        let mut position = self.starts.partition_point(|&start| start <= addr);
        while position > 0 {
            position -= 1;
            if self.prefix_max_end[position] < addr {
                break;
            }
            if self.ends[position] < want_end {
                continue;
            }
            visit(position);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorAnswer<'a> {
    Served(&'a [u8]),
    NotCovered,
    Unreadable,
    SampleAbsent,
    NeverObserved,
    Aliased,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorImage {
    Opening,
    Sample(usize),
    Closing,
    ClosingSample(usize),
}

pub struct D81VectorCapture {
    pub hour: crate::snapshot_recipes::HourStamp,
    opening: [FamilyStore; D81_FAMILY_COUNT],
    header_close: FamilyStore,
    record_ptr_close: FamilyStore,
    pub copies: Cell<u64>,
    pub pointers_rejected: u32,
    pub voids: u32,
    pub aliases_retired: u32,
    pub closing_tail_start: u64,
    merged: std::sync::OnceLock<MergedWindowIndex>,
    /// THE GEOMETRY CACHE. It remembers where a span's windows ARE, never what
    /// they currently say; see [`GeometryMemo`] for why that line is drawn
    /// where it is. Sound against any later retirement, and against any later
    /// `copy_side`, because both write `status` and neither moves an entry.
    memo: [Cell<Option<GeometryMemo>>; MEMO_WAYS],
}

impl D81VectorCapture {
    fn stores(&self) -> [&FamilyStore; STORE_COUNT] {
        [
            &self.opening[0],
            &self.opening[1],
            &self.opening[2],
            &self.opening[3],
            &self.opening[4],
            &self.opening[5],
            &self.opening[6],
            &self.opening[7],
            &self.opening[8],
            &self.header_close,
            &self.record_ptr_close,
        ]
    }

    fn merged(&self) -> &MergedWindowIndex {
        self.merged
            .get_or_init(|| MergedWindowIndex::build(self.stores()))
    }

    pub fn owned_window_count(&self) -> usize {
        let mut owned: Vec<(u64, u32)> = Vec::new();
        for store in &self.opening {
            if store.images.opening == 0 {
                continue;
            }
            for index in 0..store.starts.len() {
                if store.status[index] & ST_DEAD == 0 && store.status[index] & ST_O != 0 {
                    owned.push((store.starts[index], store.lens[index]));
                }
            }
        }
        owned.sort_unstable();
        owned.dedup();
        owned.len()
    }

    pub fn family_window_count(&self, family: D81Family) -> usize {
        self.opening[family as usize].window_count()
    }

    pub fn closing_window_counts(&self) -> (usize, usize) {
        (
            self.header_close.window_count(),
            self.record_ptr_close.window_count(),
        )
    }

    pub fn window_count(&self) -> usize {
        let mut total = self.header_close.window_count() + self.record_ptr_close.window_count();
        for store in &self.opening {
            total += store.window_count();
        }
        total
    }

    pub fn slab_bytes(&self) -> u64 {
        let mut total = self
            .header_close
            .total_bytes()
            .saturating_add(self.record_ptr_close.total_bytes());
        for store in &self.opening {
            total = total.saturating_add(store.total_bytes());
        }
        total
    }

    /// Which way of the direct-mapped cache an address falls in.
    ///
    /// Windows are 4 to 64 bytes and mostly 8, so the low three bits carry
    /// almost no information and are shifted out; the second term folds the
    /// allocation-granularity bits back in so that two objects a page apart do
    /// not collide by construction.
    #[inline]
    fn memo_way(addr: u64) -> usize {
        (((addr >> 3) ^ (addr >> 9)) as usize) & (MEMO_WAYS - 1)
    }

    fn resolve(&self, addr: u64, len: u32) -> Resolution {
        // THE ARM SWITCH IS TEST-ONLY, and it is here rather than in the
        // profile because the two things it exists for both need it AT the
        // read: `Arm::Compare` proves the fast form and the form it replaced
        // decide every read of a whole walk identically, and `Arm::Legacy`
        // lets one process time both arms against each other. A cross-build
        // comparison could do neither on this box -- the same binary measured
        // three times in a row varied 25-35% at the production shape while a
        // game ran beside it, which is wider than the change being measured.
        #[cfg(test)]
        match resolution_arm() {
            Arm::Fast => {}
            Arm::Legacy => return self.resolve_legacy(addr, len),
            Arm::Compare => {
                let legacy = self.resolve_legacy(addr, len);
                let fast = self.resolve_fast(addr, len);
                assert_eq!(
                    fast, legacy,
                    "the resolution cache and the array-and-loop form it \
                     replaced must decide {addr:#x}+{len} identically",
                );
                return fast;
            }
        }
        self.resolve_fast(addr, len)
    }

    fn resolve_fast(&self, addr: u64, len: u32) -> Resolution {
        let way = Self::memo_way(addr);
        if let Some(memo) = self.memo[way].get() {
            if memo.addr == addr && memo.len == len && memo.visits != TOO_MANY_VISITS {
                return self.verdict_of(
                    memo.positions[..usize::from(memo.visits)]
                        .iter()
                        .map(|&position| position as usize),
                );
            }
        }
        // THE SCAN, ONCE, RECORDING WHERE IT LOOKED. The positions are what
        // gets remembered; the verdict is taken from them by the same
        // `verdict_of` the memo hit uses, so the two paths cannot drift.
        let mut positions = [0u32; MEMO_VISITS];
        let mut visits = 0usize;
        self.merged()
            .for_each_containing_position(addr, len, |position| {
                if visits < MEMO_VISITS {
                    positions[visits] = position as u32;
                }
                visits = visits.saturating_add(1);
            });
        self.memo[way].set(Some(GeometryMemo {
            addr,
            len,
            visits: if visits > MEMO_VISITS {
                TOO_MANY_VISITS
            } else {
                visits as u8
            },
            positions,
        }));
        if visits > MEMO_VISITS {
            // Too many to record, so take the verdict straight off a fresh
            // scan rather than off a partial record.
            return self.verdict_scanning(addr, len);
        }
        self.verdict_of(
            positions[..visits]
                .iter()
                .map(|&position| position as usize),
        )
    }

    /// The verdict over a span's containing windows, named by merged-index
    /// position. Both the memo hit and the memo miss come through here.
    fn verdict_of(&self, positions: impl Iterator<Item = usize>) -> Resolution {
        let merged = self.merged();
        let stores = self.stores();
        let mut family: Option<u8> = None;
        let mut opening = NO_WINDOW;
        let mut closing = NO_WINDOW;
        for position in positions {
            let store_id = usize::from(merged.store[position]);
            let index = merged.index[position] as usize;
            let of_family = STORE_FAMILY[store_id];
            match family {
                None => family = Some(of_family),
                Some(seen) if seen == of_family => {}
                // Two families over one span: neither can claim it, exactly as
                // the `hit.is_some()` arm refused before.
                Some(_) => return Resolution::Aliased,
            }
            // A retired window POISONS its family whether or not a live window
            // of the same family also covers the span. The old form spelled
            // this as `poisoned |= opening_dead || closing_dead`, tested after
            // the fold and therefore after any live hit had already been
            // recorded. THIS LOAD IS WHY THE MEMO HOLDS POSITIONS AND NOT A
            // VERDICT: it is re-taken on every read, so a retirement between
            // two reads of one span is seen by the second.
            if stores[store_id].status[index] & ST_DEAD != 0 {
                return Resolution::Aliased;
            }
            if store_id >= D81_FAMILY_COUNT {
                if closing == NO_WINDOW {
                    closing = index as u32;
                }
            } else if opening == NO_WINDOW {
                opening = index as u32;
            }
        }
        match family {
            Some(family) if opening != NO_WINDOW || closing != NO_WINDOW => Resolution::Hit {
                family,
                opening,
                closing,
            },
            // A family is only ever recorded by a visit, and a visit either
            // refuses above or fills one of the two slots, so this arm is the
            // `hit.is_none()` one: nothing contained the span.
            _ => Resolution::NotCovered,
        }
    }

    /// [`Self::verdict_of`] straight off the index, for the span that has more
    /// containing windows than a memo entry holds.
    fn verdict_scanning(&self, addr: u64, len: u32) -> Resolution {
        let mut positions = Vec::new();
        self.merged()
            .for_each_containing_position(addr, len, |position| positions.push(position));
        self.verdict_of(positions.into_iter())
    }

    /// THE FORM THE CACHE REPLACED, kept verbatim so the difference between
    /// the two is a fact a test can check rather than a claim a comment makes.
    ///
    /// It is a transcription and not a paraphrase: the `[Option<usize>;
    /// STORE_COUNT]` pair, the nine-family fold, the `poisoned` accumulation
    /// before the `continue`, and the second-family early return all stand
    /// exactly where they stood. Only the return type changed, from the
    /// `(family, opening, closing)` triple it fell through to into the
    /// [`Resolution`] that names the same three outcomes.
    #[cfg(test)]
    fn resolve_legacy(&self, addr: u64, len: u32) -> Resolution {
        let stores = self.stores();
        let mut live: [Option<usize>; STORE_COUNT] = [None; STORE_COUNT];
        let mut dead: [bool; STORE_COUNT] = [false; STORE_COUNT];
        self.merged()
            .for_each_containing(addr, len, |store_id, index| {
                let store_id = usize::from(store_id);
                if stores[store_id].status[index] & ST_DEAD != 0 {
                    dead[store_id] = true;
                } else if live[store_id].is_none() {
                    live[store_id] = Some(index);
                }
            });

        let mut hit: Option<(usize, Option<usize>, Option<usize>)> = None;
        let mut poisoned = false;
        for family in 0..D81_FAMILY_COUNT {
            let (opening_hit, opening_dead) = (live[family], dead[family]);
            let (closing_hit, closing_dead) = match family {
                f if f == D81Family::RailHeader as usize => (
                    live[usize::from(STORE_HEADER_CLOSE)],
                    dead[usize::from(STORE_HEADER_CLOSE)],
                ),
                f if f == D81Family::MapRecordPtr as usize => (
                    live[usize::from(STORE_RECORD_PTR_CLOSE)],
                    dead[usize::from(STORE_RECORD_PTR_CLOSE)],
                ),
                _ => (None, false),
            };
            poisoned |= opening_dead || closing_dead;
            if opening_hit.is_none() && closing_hit.is_none() && !opening_dead && !closing_dead {
                continue;
            }
            if hit.is_some() {
                return Resolution::Aliased;
            }
            hit = Some((family, opening_hit, closing_hit));
        }
        if poisoned {
            return Resolution::Aliased;
        }
        let Some((family, opening_hit, closing_hit)) = hit else {
            return Resolution::NotCovered;
        };
        Resolution::Hit {
            family: family as u8,
            opening: opening_hit.map_or(NO_WINDOW, |index| index as u32),
            closing: closing_hit.map_or(NO_WINDOW, |index| index as u32),
        }
    }

    pub fn read(&self, addr: u64, len: u32, image: VectorImage) -> VectorAnswer<'_> {
        let (family, opening_hit, closing_hit) = match self.resolve(addr, len) {
            Resolution::NotCovered => return VectorAnswer::NotCovered,
            Resolution::Aliased => return VectorAnswer::Aliased,
            Resolution::Hit {
                family,
                opening,
                closing,
            } => (
                usize::from(family),
                (opening != NO_WINDOW).then_some(opening as usize),
                (closing != NO_WINDOW).then_some(closing as usize),
            ),
        };
        let store = &self.opening[family];
        match image {
            VectorImage::Opening | VectorImage::Sample(_) => {
                let ordinal = match image {
                    VectorImage::Sample(sample) => sample,
                    _ => 0,
                };
                if store.images.opening == 0 {
                    return VectorAnswer::NotCovered;
                }
                let Some(index) = opening_hit else {
                    return VectorAnswer::NotCovered;
                };
                if ordinal >= usize::from(store.images.opening) {
                    return VectorAnswer::NeverObserved;
                }
                Self::serve_side(store, index, addr, len, ordinal, ST_O)
            }
            VectorImage::Closing | VectorImage::ClosingSample(_) => {
                let ordinal = match image {
                    VectorImage::ClosingSample(sample) => sample,
                    _ => 0,
                };
                if let Some(index) = closing_hit {
                    let close_store = if family == D81Family::RailHeader as usize {
                        &self.header_close
                    } else {
                        &self.record_ptr_close
                    };
                    return Self::serve_side(close_store, index, addr, len, ordinal, ST_O);
                }
                if store.images.closing == 0 {
                    return VectorAnswer::NeverObserved;
                }
                let Some(index) = opening_hit else {
                    return VectorAnswer::NotCovered;
                };
                if ordinal >= usize::from(store.images.closing) {
                    return VectorAnswer::NeverObserved;
                }
                Self::serve_side(store, index, addr, len, ordinal, ST_O)
            }
        }
    }

    fn serve_side<'a>(
        store: &'a FamilyStore,
        index: usize,
        addr: u64,
        len: u32,
        ordinal: usize,
        first_bit: u8,
    ) -> VectorAnswer<'a> {
        let bit = first_bit << ordinal;
        if store.status[index] & bit != 0 {
            return VectorAnswer::Served(store.serve(index, addr, len, ordinal));
        }
        if store.status[index] & first_bit == 0 {
            VectorAnswer::Unreadable
        } else {
            VectorAnswer::SampleAbsent
        }
    }
}

pub struct D81VectorRoots {
    pub rail_table: u64,
    pub rail_slots: u32,
    pub map_array: u64,
    pub map_count: u32,
    pub source_array: u64,
    pub source_count: u32,
    pub active_table: u64,
    pub bulk_spans: Vec<(u64, u64)>,
}

pub trait BulkOpeningRead {
    fn u64_at(&self, addr: u64) -> Option<u64>;
    fn u32_at(&self, addr: u64) -> Option<u32>;
    fn u8_at(&self, addr: u64) -> Option<u8>;
}

pub struct D81VectorOffsets {
    pub rail_header_window_off: u64,
    pub rail_header_window_len: u32,
    pub pairs_ptr_in_window: usize,
    pub pair_count_in_window: usize,
    pub pair_stride: u32,
    pub max_neighbors: u32,
    pub map_record_ptr_off: u64,
    pub map_anchors_off: u64,
    pub map_anchors_len: u32,
    pub record_key_off: u64,
    pub cprovince_window_len: u32,
    pub active_entry_stride: u64,
    pub active_window_off: u64,
    pub active_window_len: u32,
}

pub struct D81VectorMemory<'a> {
    pub copy: &'a mut dyn FnMut(u64, &mut [u8]) -> bool,
    pub admit: &'a dyn Fn(u64) -> bool,
}

pub fn d81_vector_observe(
    hour: crate::snapshot_recipes::HourStamp,
    roots: &D81VectorRoots,
    offsets: &D81VectorOffsets,
    bulk: &dyn BulkOpeningRead,
    memory: &mut D81VectorMemory<'_>,
) -> D81VectorCapture {
    let mut capture = D81VectorCapture {
        hour,
        opening: [
            FamilyStore::new(family_images(D81Family::RailHeader)),
            FamilyStore::new(family_images(D81Family::PairBlob)),
            FamilyStore::new(family_images(D81Family::MapRecordPtr)),
            FamilyStore::new(family_images(D81Family::MapAnchors)),
            FamilyStore::new(family_images(D81Family::RecordKey)),
            FamilyStore::new(family_images(D81Family::SourceProvince)),
            FamilyStore::new(family_images(D81Family::SourceSlotClose)),
            FamilyStore::new(family_images(D81Family::MapSlotClose)),
            FamilyStore::new(family_images(D81Family::ActiveClose)),
        ],
        header_close: FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        }),
        record_ptr_close: FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        }),
        copies: Cell::new(0),
        pointers_rejected: 0,
        voids: 0,
        aliases_retired: 0,
        closing_tail_start: 0,
        merged: std::sync::OnceLock::new(),
        memo: Default::default(),
    };

    let mut railed: Vec<(u32, u64)> = Vec::new();
    for id in 0..roots.rail_slots {
        let slot = roots.rail_table + u64::from(id) * 8;
        let Some(pointer) = bulk.u64_at(slot) else {
            continue;
        };
        if pointer == 0 {
            continue;
        }
        if !(memory.admit)(pointer) {
            capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
            continue;
        }
        railed.push((id, pointer));
    }

    {
        let store = &mut capture.opening[D81Family::RailHeader as usize];
        for &(_, pointer) in &railed {
            store.push(
                pointer + offsets.rail_header_window_off,
                offsets.rail_header_window_len,
            );
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    let mut blobs: Vec<(u64, u32)> = Vec::new();
    {
        let store = &capture.opening[D81Family::RailHeader as usize];
        for index in 0..store.window_count() {
            if store.status[index] & ST_O == 0 {
                capture.voids = capture.voids.saturating_add(1);
                continue;
            }
            let window = store.serve(
                index,
                store.starts[index],
                offsets.rail_header_window_len,
                0,
            );
            let ptr = u64::from_le_bytes(
                window[offsets.pairs_ptr_in_window..offsets.pairs_ptr_in_window + 8]
                    .try_into()
                    .expect("eight bytes"),
            );
            let count = u32::from_le_bytes(
                window[offsets.pair_count_in_window..offsets.pair_count_in_window + 4]
                    .try_into()
                    .expect("four bytes"),
            );
            if count == 0 || count > offsets.max_neighbors {
                continue;
            }
            if !(memory.admit)(ptr) {
                capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
                continue;
            }
            blobs.push((ptr, count.saturating_mul(offsets.pair_stride)));
        }
    }

    {
        let opening = &capture.opening[D81Family::RailHeader as usize];
        for index in 0..opening.window_count() {
            capture
                .header_close
                .push(opening.starts[index], opening.lens[index]);
        }
        capture.header_close.seal();
        capture
            .header_close
            .copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    {
        let store = &mut capture.opening[D81Family::PairBlob as usize];
        for &(ptr, len) in &blobs {
            store.push(ptr, len);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    let mut map_provinces: Vec<(u32, u64)> = Vec::new();
    for &(id, _) in &railed {
        if id >= roots.map_count {
            continue;
        }
        let slot = roots.map_array + u64::from(id) * 8;
        let Some(pointer) = bulk.u64_at(slot) else {
            continue;
        };
        if pointer == 0 {
            continue;
        }
        if !(memory.admit)(pointer) {
            capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
            continue;
        }
        map_provinces.push((id, pointer));
    }

    {
        let record = &mut capture.opening[D81Family::MapRecordPtr as usize];
        for &(_, map_province) in &map_provinces {
            record.push(map_province + offsets.map_record_ptr_off, 8);
        }
        record.seal();
        record.copy_side(2, ST_O, memory.copy, &capture.copies);
    }
    {
        let anchors = &mut capture.opening[D81Family::MapAnchors as usize];
        for &(_, map_province) in &map_provinces {
            anchors.push(
                map_province + offsets.map_anchors_off,
                offsets.map_anchors_len,
            );
        }
        anchors.seal();
        anchors.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    let mut records: Vec<u64> = Vec::new();
    {
        let store = &capture.opening[D81Family::MapRecordPtr as usize];
        for index in 0..store.window_count() {
            if store.status[index] & ST_O == 0 {
                capture.voids = capture.voids.saturating_add(1);
                continue;
            }
            let bytes = store.serve(index, store.starts[index], 8, 0);
            let pointer = u64::from_le_bytes(bytes.try_into().expect("eight bytes"));
            if pointer == 0 {
                continue;
            }
            if !(memory.admit)(pointer) {
                capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
                continue;
            }
            records.push(pointer);
        }
    }
    {
        let store = &mut capture.opening[D81Family::RecordKey as usize];
        for &record in &records {
            store.push(record + offsets.record_key_off, 4);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    {
        let opening = &capture.opening[D81Family::MapRecordPtr as usize];
        for index in 0..opening.window_count() {
            capture
                .record_ptr_close
                .push(opening.starts[index], opening.lens[index]);
        }
        capture.record_ptr_close.seal();
        capture
            .record_ptr_close
            .copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    let mut cprovinces: Vec<u64> = Vec::new();
    for &(id, _) in &railed {
        if id >= roots.source_count {
            continue;
        }
        let slot = roots.source_array + u64::from(id) * 8;
        if let Some(pointer) = bulk.u64_at(slot) {
            if pointer == 0 {
                continue;
            }
            if !(memory.admit)(pointer) {
                capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
                continue;
            }
            cprovinces.push(pointer);
        }
    }
    let mut selector_keys: Vec<u32> = Vec::new();
    {
        let store = &capture.opening[D81Family::RecordKey as usize];
        for index in 0..store.window_count() {
            if store.status[index] & ST_O == 0 {
                capture.voids = capture.voids.saturating_add(1);
                continue;
            }
            let bytes = store.serve(index, store.starts[index], 4, 0);
            let key = u32::from_le_bytes(bytes.try_into().expect("four bytes"));
            if key < roots.source_count {
                selector_keys.push(key);
                let slot = roots.source_array + u64::from(key) * 8;
                if let Some(pointer) = bulk.u64_at(slot) {
                    if pointer == 0 {
                        continue;
                    }
                    if !(memory.admit)(pointer) {
                        capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
                        continue;
                    }
                    cprovinces.push(pointer);
                }
            }
        }
    }
    {
        let store = &mut capture.opening[D81Family::SourceProvince as usize];
        for &pointer in &cprovinces {
            store.push(pointer, offsets.cprovince_window_len);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    capture.closing_tail_start = capture.copies.get();
    {
        let store = &mut capture.opening[D81Family::SourceSlotClose as usize];
        for &(id, _) in &railed {
            if id < roots.source_count {
                store.push(roots.source_array + u64::from(id) * 8, 8);
            }
        }
        for &key in &selector_keys {
            store.push(roots.source_array + u64::from(key) * 8, 8);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }
    {
        let store = &mut capture.opening[D81Family::MapSlotClose as usize];
        for &(id, _) in &map_provinces {
            store.push(roots.map_array + u64::from(id) * 8, 8);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }
    {
        let store = &mut capture.opening[D81Family::ActiveClose as usize];
        for &(id, _) in &railed {
            let entry = roots.active_table + u64::from(id) * offsets.active_entry_stride;
            store.push(entry + offsets.active_window_off, offsets.active_window_len);
        }
        store.seal();
        store.copy_side(3, ST_O, memory.copy, &capture.copies);
    }

    for store in capture
        .opening
        .iter_mut()
        .take(D81Family::SourceSlotClose as usize)
    {
        capture.aliases_retired = capture
            .aliases_retired
            .saturating_add(store.retire_intersecting(&roots.bulk_spans));
    }
    capture.aliases_retired = capture
        .aliases_retired
        .saturating_add(capture.header_close.retire_intersecting(&roots.bulk_spans));
    capture.aliases_retired = capture.aliases_retired.saturating_add(
        capture
            .record_ptr_close
            .retire_intersecting(&roots.bulk_spans),
    );

    let mut voids = 0u32;
    for store in capture.opening.iter() {
        if store.images.opening == 0 {
            continue;
        }
        for &status in &store.status {
            if status & (ST_O | ST_DEAD) == 0 {
                voids = voids.saturating_add(1);
            }
        }
    }
    capture.voids = voids;

    capture
}

#[cfg(test)]
mod shape_tests {
    use super::*;

    fn copy_from(world: &[u8], base: u64) -> impl FnMut(u64, &mut [u8]) -> bool + '_ {
        move |addr, out: &mut [u8]| {
            let Some(offset) = addr.checked_sub(base) else {
                return false;
            };
            let offset = offset as usize;
            if offset + out.len() > world.len() {
                return false;
            }
            out.copy_from_slice(&world[offset..offset + out.len()]);
            true
        }
    }

    #[test]
    fn seal_sorts_dedups_and_sizes() {
        let mut store = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        store.push(0x3000, 16);
        store.push(0x1000, 16);
        store.push(0x3000, 16);
        store.push(0x2000, 16);
        store.seal();
        assert_eq!(store.starts, vec![0x1000, 0x2000, 0x3000]);
        assert_eq!(store.offsets, vec![0, 16, 32, 48]);
        assert_eq!(store.slabs[0].len(), 48);
        assert_eq!(store.slabs[1].len(), 48);
        assert_eq!(store.slabs[2].len(), 0);
    }

    #[test]
    fn copy_side_interleaves_per_window() {
        let mut store = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        store.push(0x1000, 4);
        store.push(0x2000, 4);
        store.seal();
        let mut order: Vec<u64> = Vec::new();
        let calls = Cell::new(0);
        let mut copy = |addr: u64, out: &mut [u8]| {
            order.push(addr);
            out.fill(0xAB);
            true
        };
        store.copy_side(2, ST_O, &mut copy, &calls);
        assert_eq!(order, vec![0x1000, 0x1000, 0x2000, 0x2000]);
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn failed_first_observation_ends_the_window_and_serves_nothing() {
        let mut store = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        store.push(0x1000, 4);
        store.seal();
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, _out: &mut [u8]| false;
        store.copy_side(2, ST_O, &mut copy, &calls);
        assert_eq!(calls.get(), 1);
        assert_eq!(store.status[0], 0);
    }

    #[test]
    fn failed_sample_is_absent_not_unreadable() {
        let mut store = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        store.push(0x1000, 4);
        store.seal();
        let calls = Cell::new(0);
        let mut first = true;
        let mut copy = |_addr: u64, out: &mut [u8]| {
            let ok = first;
            first = false;
            if ok {
                out.fill(0xCD);
            }
            ok
        };
        store.copy_side(2, ST_O, &mut copy, &calls);
        assert_eq!(calls.get(), 2);
        assert_eq!(store.status[0], ST_O);
        let capture = capture_with_store(store);
        assert!(matches!(
            capture.read(0x1000, 4, VectorImage::Opening),
            VectorAnswer::Served(&[0xCD, 0xCD, 0xCD, 0xCD])
        ));
        assert_eq!(
            capture.read(0x1000, 4, VectorImage::Sample(1)),
            VectorAnswer::SampleAbsent
        );
    }

    fn capture_with_store(store: FamilyStore) -> D81VectorCapture {
        let mut capture = empty_capture();
        capture.opening[D81Family::RailHeader as usize] = store;
        capture
    }

    fn empty_capture() -> D81VectorCapture {
        let mut opening: Vec<FamilyStore> = Vec::new();
        for family in [
            D81Family::RailHeader,
            D81Family::PairBlob,
            D81Family::MapRecordPtr,
            D81Family::MapAnchors,
            D81Family::RecordKey,
            D81Family::SourceProvince,
            D81Family::SourceSlotClose,
            D81Family::MapSlotClose,
            D81Family::ActiveClose,
        ] {
            opening.push(FamilyStore::new(family_images(family)));
        }
        let opening: [FamilyStore; D81_FAMILY_COUNT] =
            opening.try_into().map_err(|_| ()).expect("nine families");
        D81VectorCapture {
            hour: crate::snapshot_recipes::HourStamp(1),
            opening,
            header_close: FamilyStore::new(FamilyImages {
                opening: 2,
                closing: 0,
            }),
            record_ptr_close: FamilyStore::new(FamilyImages {
                opening: 2,
                closing: 0,
            }),
            copies: Cell::new(0),
            pointers_rejected: 0,
            voids: 0,
            aliases_retired: 0,
            closing_tail_start: 0,
            merged: std::sync::OnceLock::new(),
            memo: Default::default(),
        }
    }

    #[test]
    fn image_demand_is_enforced_per_family() {
        let mut blob = FamilyStore::new(family_images(D81Family::PairBlob));
        blob.push(0x4000, 8);
        blob.seal();
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(1);
            true
        };
        blob.copy_side(2, ST_O, &mut copy, &calls);
        let mut capture = empty_capture();
        capture.opening[D81Family::PairBlob as usize] = blob;
        assert!(matches!(
            capture.read(0x4000, 8, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
        assert_eq!(
            capture.read(0x4000, 8, VectorImage::Closing),
            VectorAnswer::NeverObserved
        );

        let mut slots = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        slots.push(0x8000, 8);
        slots.seal();
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(2);
            true
        };
        slots.copy_side(2, ST_O, &mut copy, &calls);
        let mut capture = empty_capture();
        capture.opening[D81Family::SourceSlotClose as usize] = slots;
        assert_eq!(
            capture.read(0x8000, 8, VectorImage::Opening),
            VectorAnswer::NotCovered
        );
        assert!(matches!(
            capture.read(0x8000, 8, VectorImage::Closing),
            VectorAnswer::Served(_)
        ));
        assert!(matches!(
            capture.read(0x8000, 8, VectorImage::ClosingSample(1)),
            VectorAnswer::Served(_)
        ));
    }

    #[test]
    fn a_span_two_families_contain_is_refused_not_served() {
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(7);
            true
        };
        let mut capture = empty_capture();
        let mut header = FamilyStore::new(family_images(D81Family::RailHeader));
        header.push(0x1000, 16);
        header.seal();
        header.copy_side(2, ST_O, &mut copy, &calls);
        capture.opening[D81Family::RailHeader as usize] = header;
        let mut record = FamilyStore::new(family_images(D81Family::MapRecordPtr));
        record.push(0x1004, 8);
        record.seal();
        record.copy_side(2, ST_O, &mut copy, &calls);
        capture.opening[D81Family::MapRecordPtr as usize] = record;
        assert_eq!(
            capture.read(0x1004, 8, VectorImage::Opening),
            VectorAnswer::Aliased
        );
        assert_eq!(
            capture.read(0x1004, 8, VectorImage::Closing),
            VectorAnswer::Aliased
        );
        assert!(matches!(
            capture.read(0x1000, 4, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
    }

    #[test]
    fn a_retired_windows_span_is_poisoned_not_invisible() {
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(9);
            true
        };
        let mut capture = empty_capture();
        let mut record = FamilyStore::new(family_images(D81Family::MapRecordPtr));
        record.push(0x2000, 8);
        record.seal();
        record.copy_side(2, ST_O, &mut copy, &calls);
        assert_eq!(record.retire_intersecting(&[(0x1FFC, 0x2010)]), 1);
        capture.opening[D81Family::MapRecordPtr as usize] = record;
        let mut slots = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        slots.push(0x2000, 8);
        slots.seal();
        let mut copy2 = |_addr: u64, out: &mut [u8]| {
            out.fill(3);
            true
        };
        slots.copy_side(2, ST_O, &mut copy2, &calls);
        capture.opening[D81Family::SourceSlotClose as usize] = slots;
        for image in [
            VectorImage::Opening,
            VectorImage::Sample(1),
            VectorImage::Closing,
            VectorImage::ClosingSample(1),
        ] {
            assert_eq!(
                capture.read(0x2000, 8, image),
                VectorAnswer::Aliased,
                "a poisoned span must refuse every image, found {image:?}",
            );
        }
    }

    /// EVERY REFUSING SHAPE, BOTH ARMS, ASSERTED EQUAL (lane PC, 2026-08-21).
    ///
    /// `the_resolution_cache_decides_every_read_the_old_form_decided` runs the
    /// differential over a whole staged walk, which is the realistic
    /// population -- and that is exactly its weakness: a clean world retires
    /// nothing and aliases nothing, so it proves the two forms agree on reads
    /// that SUCCEED and never on the reads that refuse. The refusing shapes
    /// have to be built by hand, and they are the ones where the two forms
    /// could most easily diverge: the fast form decides `Aliased` DURING the
    /// scan, where the old form decided it after a nine-family fold.
    ///
    /// Each case below is checked before and after the retirement that changes
    /// its answer, so the cache is also asked to answer a span it has already
    /// answered differently.
    #[test]
    fn both_resolution_arms_agree_on_the_refusing_shapes() {
        let calls = Cell::new(0);
        let mut fill = |_addr: u64, out: &mut [u8]| {
            out.fill(7);
            true
        };

        // Two FAMILIES over one span: an alias, and never a served read.
        let mut aliased = empty_capture();
        let mut record = FamilyStore::new(family_images(D81Family::MapRecordPtr));
        record.push(0x2000, 8);
        record.seal();
        record.copy_side(2, ST_O, &mut fill, &calls);
        aliased.opening[D81Family::MapRecordPtr as usize] = record;
        let mut anchors = FamilyStore::new(family_images(D81Family::MapAnchors));
        anchors.push(0x2000, 8);
        anchors.seal();
        anchors.copy_side(2, ST_O, &mut fill, &calls);
        aliased.opening[D81Family::MapAnchors as usize] = anchors;

        // ONE family over one span, with its closing store at the same
        // address: the case `STORE_FAMILY` exists for, which must serve.
        let mut paired = empty_capture();
        let mut header = FamilyStore::new(family_images(D81Family::RailHeader));
        header.push(0x3000, 16);
        header.seal();
        header.copy_side(2, ST_O, &mut fill, &calls);
        paired.opening[D81Family::RailHeader as usize] = header;
        let mut header_close = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        header_close.push(0x3000, 16);
        header_close.seal();
        header_close.copy_side(2, ST_O, &mut fill, &calls);
        paired.header_close = header_close;

        let probes = [
            (0x2000u64, 8u32),
            (0x2000, 4),
            (0x2004, 4),
            (0x2000, 16),
            (0x3000, 16),
            (0x3000, 8),
            (0x3008, 8),
            (0x3000, 32),
            (0x1000, 8),
            (0x9999, 1),
        ];
        let images = [
            VectorImage::Opening,
            VectorImage::Sample(1),
            VectorImage::Sample(2),
            VectorImage::Closing,
            VectorImage::ClosingSample(1),
        ];

        // THE BEFORE HALF OF A BEFORE-AND-AFTER. Asserting only that the span
        // is `Aliased` at the end would pass on a fixture that had refused it
        // all along; this pins that the very same read SERVED first, so the
        // retirement below is what changed the answer.
        let restore = set_resolution_arm(Arm::Fast);
        let before = paired.read(0x3000, 16, VectorImage::Opening);
        set_resolution_arm(restore);
        assert!(
            matches!(before, VectorAnswer::Served(_)),
            "the paired fixture must serve its span before anything is retired, found {before:?}",
        );

        for capture in [&mut aliased, &mut paired] {
            for retire in [false, true] {
                if retire {
                    // Retire AFTER the reads above have run, so the geometry
                    // cache is already populated when the status changes.
                    //
                    // NOT EVERY PROBE IS STILL RESIDENT, and claiming otherwise
                    // would be the kind of overstatement this test exists to
                    // avoid. `memo_way` puts 0x2000, 0x2004, 0x3000 and 0x1000
                    // all in way 0, where they evict each other; 0x3008 lands
                    // in way 1 and 0x9999 in way 3, so those two are the probes
                    // that genuinely reach a WARM entry after the retirement.
                    // 0x3008+8 is the one that matters: it is covered by the
                    // retired header window, so it is the probe that proves a
                    // cached geometry does not carry a stale verdict with it.
                    for family in [D81Family::MapRecordPtr, D81Family::RailHeader] {
                        capture.opening[family as usize]
                            .retire_intersecting(&[(0x2000, 0x2010), (0x3000, 0x3010)]);
                    }
                }
                for &(addr, len) in &probes {
                    for image in images {
                        let restore = set_resolution_arm(Arm::Legacy);
                        let legacy = capture.read(addr, len, image);
                        set_resolution_arm(Arm::Fast);
                        let fast = capture.read(addr, len, image);
                        set_resolution_arm(restore);
                        assert_eq!(
                            fast, legacy,
                            "arms disagreed at {addr:#x}+{len} {image:?}, retired={retire}",
                        );
                        // And the compare arm must reach the same answer
                        // rather than merely not panicking.
                        //
                        // A disagreement panics INSIDE `read`, before the
                        // restore below, so the thread-local is left on
                        // `Compare` for whatever runs next on this thread.
                        // That is deliberate: the only way to reach it is a
                        // test that is already failing, and unwinding through
                        // a guard to tidy up a failed run would buy nothing.
                        set_resolution_arm(Arm::Compare);
                        let compared = capture.read(addr, len, image);
                        set_resolution_arm(restore);
                        assert_eq!(compared, legacy, "compare arm diverged");
                    }
                }
            }
        }

        // The retirements must actually have changed something, or the second
        // half of this test proved nothing.
        let restore = set_resolution_arm(Arm::Fast);
        assert_eq!(
            paired.read(0x3000, 16, VectorImage::Opening),
            VectorAnswer::Aliased,
            "the retired header window must poison its span",
        );
        set_resolution_arm(restore);
    }

    #[test]
    fn a_window_retired_after_the_first_read_poisons_from_the_next_read() {
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(5);
            true
        };
        let mut capture = empty_capture();
        let mut record = FamilyStore::new(family_images(D81Family::MapRecordPtr));
        record.push(0x2000, 8);
        record.seal();
        record.copy_side(2, ST_O, &mut copy, &calls);
        capture.opening[D81Family::MapRecordPtr as usize] = record;
        assert!(matches!(
            capture.read(0x2000, 8, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
        assert_eq!(
            capture.opening[D81Family::MapRecordPtr as usize]
                .retire_intersecting(&[(0x1FFC, 0x2010)]),
            1
        );
        assert_eq!(
            capture.read(0x2000, 8, VectorImage::Opening),
            VectorAnswer::Aliased
        );
    }

    #[test]
    fn a_containing_window_beyond_eight_predecessors_is_found() {
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, out: &mut [u8]| {
            out.fill(6);
            true
        };
        let mut capture = empty_capture();
        let mut blob = FamilyStore::new(family_images(D81Family::PairBlob));
        blob.push(0x1000, 0x200);
        for k in 0..9u64 {
            blob.push(0x1010 + k * 8, 8);
        }
        blob.seal();
        blob.copy_side(2, ST_O, &mut copy, &calls);
        capture.opening[D81Family::PairBlob as usize] = blob;
        assert!(matches!(
            capture.read(0x1100, 8, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
        let mut anchors = FamilyStore::new(family_images(D81Family::MapAnchors));
        anchors.push(0x1100, 24);
        anchors.seal();
        anchors.copy_side(2, ST_O, &mut copy, &calls);
        let mut capture2 = empty_capture();
        capture2.opening[D81Family::PairBlob as usize] = std::mem::replace(
            &mut capture.opening[D81Family::PairBlob as usize],
            FamilyStore::default(),
        );
        capture2.opening[D81Family::MapAnchors as usize] = anchors;
        assert_eq!(
            capture2.read(0x1100, 8, VectorImage::Opening),
            VectorAnswer::Aliased
        );
    }

    #[test]
    fn merged_index_agrees_with_per_store_scans() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        let calls = Cell::new(0);
        let mut capture = empty_capture();
        let widths: [u32; STORE_COUNT] = [16, 8, 8, 24, 4, 0xA8, 8, 8, 7, 16, 8];
        let images: [usize; STORE_COUNT] = [2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 2];
        for store_id in 0..STORE_COUNT {
            let store = match store_id {
                s if s < D81_FAMILY_COUNT => &mut capture.opening[s],
                s if s == usize::from(STORE_HEADER_CLOSE) => &mut capture.header_close,
                _ => &mut capture.record_ptr_close,
            };
            for _ in 0..200 {
                let start = 0x10_0000 + (next() % 0x1_0000);
                let width = if store_id == D81Family::PairBlob as usize {
                    8 * (1 + (next() % 64) as u32)
                } else {
                    widths[store_id]
                };
                store.push(start, width);
            }
            store.seal();
            let mut copy = |addr: u64, out: &mut [u8]| {
                let fail = (addr / 8) % 9 == 0;
                let drop_second = (addr / 8) % 13 == 0;
                if fail {
                    return false;
                }
                let image_ordinal = calls.get() % 2;
                if drop_second && image_ordinal == 1 {
                    return false;
                }
                for (i, byte) in out.iter_mut().enumerate() {
                    *byte = (addr as u8).wrapping_add(i as u8);
                }
                true
            };
            store.copy_side(images[store_id], ST_O, &mut copy, &calls);
            let mut spans = Vec::new();
            for index in (0..store.starts.len()).step_by(17) {
                spans.push((store.starts[index], store.starts[index] + 1));
            }
            store.retire_intersecting(&spans);
        }

        fn reference<'a>(
            capture: &'a D81VectorCapture,
            addr: u64,
            len: u32,
            image: VectorImage,
        ) -> VectorAnswer<'a> {
            let mut hit: Option<(usize, Option<usize>, Option<usize>)> = None;
            let mut poisoned = false;
            for family in 0..D81_FAMILY_COUNT {
                let (opening_hit, opening_dead) =
                    capture.opening[family].locate_with_dead(addr, len);
                let (closing_hit, closing_dead) = match family {
                    f if f == D81Family::RailHeader as usize => {
                        capture.header_close.locate_with_dead(addr, len)
                    }
                    f if f == D81Family::MapRecordPtr as usize => {
                        capture.record_ptr_close.locate_with_dead(addr, len)
                    }
                    _ => (None, false),
                };
                poisoned |= opening_dead || closing_dead;
                if opening_hit.is_none() && closing_hit.is_none() && !opening_dead && !closing_dead
                {
                    continue;
                }
                if hit.is_some() {
                    return VectorAnswer::Aliased;
                }
                hit = Some((family, opening_hit, closing_hit));
            }
            if poisoned {
                return VectorAnswer::Aliased;
            }
            let Some((family, opening_hit, closing_hit)) = hit else {
                return VectorAnswer::NotCovered;
            };
            let store = &capture.opening[family];
            match image {
                VectorImage::Opening | VectorImage::Sample(_) => {
                    let ordinal = match image {
                        VectorImage::Sample(sample) => sample,
                        _ => 0,
                    };
                    if store.images.opening == 0 {
                        return VectorAnswer::NotCovered;
                    }
                    let Some(index) = opening_hit else {
                        return VectorAnswer::NotCovered;
                    };
                    if ordinal >= usize::from(store.images.opening) {
                        return VectorAnswer::NeverObserved;
                    }
                    D81VectorCapture::serve_side(store, index, addr, len, ordinal, ST_O)
                }
                VectorImage::Closing | VectorImage::ClosingSample(_) => {
                    let ordinal = match image {
                        VectorImage::ClosingSample(sample) => sample,
                        _ => 0,
                    };
                    if let Some(index) = closing_hit {
                        let close_store = if family == D81Family::RailHeader as usize {
                            &capture.header_close
                        } else {
                            &capture.record_ptr_close
                        };
                        return D81VectorCapture::serve_side(
                            close_store,
                            index,
                            addr,
                            len,
                            ordinal,
                            ST_O,
                        );
                    }
                    if store.images.closing == 0 {
                        return VectorAnswer::NeverObserved;
                    }
                    let Some(index) = opening_hit else {
                        return VectorAnswer::NotCovered;
                    };
                    if ordinal >= usize::from(store.images.closing) {
                        return VectorAnswer::NeverObserved;
                    }
                    D81VectorCapture::serve_side(store, index, addr, len, ordinal, ST_O)
                }
            }
        }

        let mut probes: Vec<(u64, u32)> = Vec::new();
        for store in capture.stores() {
            for index in 0..store.starts.len() {
                let start = store.starts[index];
                let width = store.lens[index];
                probes.push((start, width));
                probes.push((start, 1));
                probes.push((start + u64::from(width) / 2, 1));
                probes.push((start + u64::from(width) - 1, 1));
                probes.push((start + u64::from(width), 0));
                probes.push((start.wrapping_sub(3), 8));
            }
        }
        for _ in 0..2000 {
            probes.push((0x10_0000 + (next() % 0x1_0100), 1 + (next() % 16) as u32));
        }
        let mut agreed = 0usize;
        let mut aliased = 0usize;
        let mut served = 0usize;
        for &(addr, len) in &probes {
            for image in [
                VectorImage::Opening,
                VectorImage::Sample(1),
                VectorImage::Closing,
                VectorImage::ClosingSample(1),
                VectorImage::ClosingSample(2),
            ] {
                let want = reference(&capture, addr, len, image);
                let got = capture.read(addr, len, image);
                assert_eq!(
                    got, want,
                    "merged index disagrees at addr {addr:#x} len {len} {image:?}"
                );
                agreed += 1;
                match got {
                    VectorAnswer::Aliased => aliased += 1,
                    VectorAnswer::Served(_) => served += 1,
                    _ => {}
                }
            }
        }
        assert!(agreed > 10_000, "{agreed}");
        assert!(aliased > 100, "aliased {aliased}");
        assert!(served > 100, "served {served}");
    }

    #[test]
    fn pipeline_order_headers_close_between_blob_decode_and_blob_copy() {
        let base = 0x10_0000u64;
        let mut world = vec![0u8; 0x10000];
        let rail_table = base + 0x100;
        let rail_a = base + 0x1000;
        let rail_b = base + 0x2000;
        let blob_a = base + 0x3000;
        let blob_b = base + 0x4000;
        let put_u64 = |world: &mut [u8], addr: u64, value: u64| {
            let offset = (addr - base) as usize;
            world[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        };
        let put_u32 = |world: &mut [u8], addr: u64, value: u32| {
            let offset = (addr - base) as usize;
            world[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put_u64(&mut world, rail_table, rail_a);
        put_u64(&mut world, rail_table + 8, rail_b);
        put_u64(&mut world, rail_a + 0x38, blob_a);
        put_u32(&mut world, rail_a + 0x44, 1);
        put_u64(&mut world, rail_b + 0x38, blob_b);
        put_u32(&mut world, rail_b + 0x44, 2);

        let roots = D81VectorRoots {
            rail_table,
            rail_slots: 2,
            map_array: base + 0x200,
            map_count: 0,
            source_array: base + 0x300,
            source_count: 0,
            active_table: base + 0x5000,
            bulk_spans: Vec::new(),
        };
        let offsets = D81VectorOffsets {
            rail_header_window_off: 0x38,
            rail_header_window_len: 0x10,
            pairs_ptr_in_window: 0,
            pair_count_in_window: 0xC,
            pair_stride: 8,
            max_neighbors: 64,
            map_record_ptr_off: 0x11B8,
            map_anchors_off: 0x14E4,
            map_anchors_len: 24,
            record_key_off: 0xC4,
            cprovince_window_len: 0xA8,
            active_entry_stride: 0x20,
            active_window_off: 0x12,
            active_window_len: 7,
        };
        struct Bulk<'a> {
            world: &'a [u8],
            base: u64,
        }
        impl BulkOpeningRead for Bulk<'_> {
            fn u64_at(&self, addr: u64) -> Option<u64> {
                let offset = addr.checked_sub(self.base)? as usize;
                let bytes = self.world.get(offset..offset + 8)?;
                Some(u64::from_le_bytes(bytes.try_into().ok()?))
            }
            fn u32_at(&self, addr: u64) -> Option<u32> {
                let offset = addr.checked_sub(self.base)? as usize;
                let bytes = self.world.get(offset..offset + 4)?;
                Some(u32::from_le_bytes(bytes.try_into().ok()?))
            }
            fn u8_at(&self, addr: u64) -> Option<u8> {
                let offset = addr.checked_sub(self.base)? as usize;
                self.world.get(offset).copied()
            }
        }

        let mut log: Vec<u64> = Vec::new();
        let mut inner = copy_from(&world, base);
        let mut copy = |addr: u64, out: &mut [u8]| {
            log.push(addr);
            inner(addr, out)
        };
        let admit = |pointer: u64| pointer >= base && pointer < base + 0x10000;
        let mut memory = D81VectorMemory {
            copy: &mut copy,
            admit: &admit,
        };
        let bulk = Bulk {
            world: &world,
            base,
        };
        let capture = d81_vector_observe(
            crate::snapshot_recipes::HourStamp(7),
            &roots,
            &offsets,
            &bulk,
            &mut memory,
        );

        let header_a = rail_a + 0x38;
        let header_b = rail_b + 0x38;
        let active_a = roots.active_table + 0x12;
        let active_b = roots.active_table + 0x20 + 0x12;
        assert_eq!(
            log,
            vec![
                header_a, header_a, header_b, header_b, header_a, header_a, header_b, header_b,
                blob_a, blob_a, blob_b, blob_b, active_a, active_a, active_a, active_b, active_b,
                active_b,
            ]
        );
        assert_eq!(capture.copies.get(), 18);
        assert_eq!(capture.pointers_rejected, 0);
        assert_eq!(capture.voids, 0);
        assert!(matches!(
            capture.read(header_a, 0x10, VectorImage::Closing),
            VectorAnswer::Served(_)
        ));
        assert!(matches!(
            capture.read(blob_a, 8, VectorImage::Closing),
            VectorAnswer::NeverObserved
        ));
    }
}
