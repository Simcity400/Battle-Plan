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

/// THE LARGEST SPAN ONE GUARDED CALL MAY ASK FOR when windows are merged.
///
/// This is the TEAR BOUND, not a memory bound. `d81_copy_bracket`'s
/// `u64_stable_at` reads an address at image 0 and again at image 1 and refuses
/// the value when the two disagree; the interval it judges is the wall time
/// between those two observations. Today that interval is one guarded call of
/// the window, and under a merged run it is still one guarded call — of the
/// hull. Capping the hull at a page keeps that call's payload at ~400 ns of
/// copy against a guarded call's ~122 ns of fixed cost, so the judged interval
/// grows by a few hundred nanoseconds while the number of calls it spans stays
/// at one. Raising this cap widens the tear detector's window; it is not a
/// tuning knob.
const MERGE_MAX_HULL_BYTES: usize = 4096;

/// THE LARGEST HOLE a run may swallow between two consecutive windows.
///
/// Bounds the wasted bytes independently of [`MERGE_MAX_HULL_BYTES`], so a
/// sparse stretch of a family's array breaks the run instead of reading a whole
/// page to collect a handful of slots. At the shapes this applies to — 8-byte
/// slots at stride 8, and 7-byte windows at stride 0x20 — 128 bytes spans a
/// skip of sixteen source slots or four active entries.
const MERGE_MAX_GAP: u64 = 128;

/// WHETHER A FAMILY'S COPY PROGRAM MAY MERGE ITS WINDOWS, and over what.
///
/// `None` is today's program exactly: one guarded call per window per image.
///
/// `Some(span)` permits [`FamilyStore::copy_side`] to collect a run of sorted,
/// non-overlapping windows into ONE hull read. The span is the memory the
/// caller has certified the frame ALREADY reads in full, and a hull may never
/// leave it — which is what makes reading the gaps between windows sound. Only
/// [`merge_domain`] constructs a non-empty one, and only after checking the
/// span against `roots.bulk_spans`.
#[derive(Clone, Copy, Debug, Default)]
struct MergePolicy {
    domain: Option<(u64, u64)>,
}

const ST_O: u8 = 1 << 0;
const ST_S1: u8 = 1 << 1;
const ST_C: u8 = 1 << 2;
const ST_CS1: u8 = 1 << 3;
const ST_CS2: u8 = 1 << 4;
const ST_DEAD: u8 = 1 << 7;

/// CLONE IS THE RAIL WRITE-TAP'S REUSE (lane SN), and it is the only reason
/// this type is `Clone`. A store is cloned exactly twice per remembered rail
/// copy: once into [`D81RailReuse`] when a full copy is taken, and once back
/// out of it on a frame whose witness is still current.
///
/// The three rail stores together are of the order of half a megabyte on
/// `captures/rc_vanilla5`'s shape, against a modelled 2.0 - 3.0 ms of guarded
/// calls they replace. Both are MODELS: no counter publishes a per-family
/// window count, and the wall figure is a whole-phase quotient, so read
/// `runs/sn_rail_write_tap/MEASUREMENT.md` section 3 for what they rest on
/// rather than either number on its own (Codex r5 finding 1).
#[derive(Clone, Default)]
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

    /// Hand every byte of this store's CONTENT to `sink`, in a fixed order.
    ///
    /// Content is the window geometry (`starts`, `lens`, `offsets`), the
    /// per-window `status` that decides whether a window may be read at all,
    /// the three slabs the reads come out of, and the image counts that say how
    /// many of those slabs are live. Nothing else in the store is content:
    /// `merged` and the geometry memo are derived indices, rebuilt from exactly
    /// these bytes.
    ///
    /// Used by [`super::d81_geometry_gate`] to prove two captures are the same
    /// bytes. It is deliberately a SUPERSET of what any one walk reads — a
    /// digest narrowed to the fields a reader touches is a second copy of that
    /// reader's read set, and the two drift.
    /// `store` is this store's index in [`D81VectorCapture::stores`], handed to
    /// the sink beside every run so a consumer can attribute a difference to
    /// the family it belongs to. It is a LABEL and nothing more: it is not fed
    /// into the byte stream, so the image this produces is byte-identical to
    /// the one produced before the label existed.
    fn feed_content(
        &self,
        store: usize,
        sink: &mut dyn FnMut(usize, &[u8]),
        reduce_active_windows: bool,
    ) {
        sink(store, &(self.starts.len() as u64).to_le_bytes());
        for value in &self.starts {
            sink(store, &value.to_le_bytes());
        }
        sink(store, &(self.lens.len() as u64).to_le_bytes());
        for value in &self.lens {
            sink(store, &value.to_le_bytes());
        }
        sink(store, &(self.offsets.len() as u64).to_le_bytes());
        for value in &self.offsets {
            sink(store, &value.to_le_bytes());
        }
        sink(store, &(self.status.len() as u64).to_le_bytes());
        sink(store, &self.status);
        for slab in &self.slabs {
            sink(store, &(slab.len() as u64).to_le_bytes());
            if reduce_active_windows {
                feed_active_windows(store, slab, sink);
            } else {
                sink(store, slab);
            }
        }
        sink(store, &[self.images.opening, self.images.closing]);
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

    /// ONE WINDOW, EVERY IMAGE — the whole of what `copy_side` used to be.
    ///
    /// Kept as its own function because it is now reached from three places:
    /// the unmerged program, the singleton run, and the fallback for a run
    /// whose hull declined. All three must set `status` identically, and the
    /// only way to make that a fact rather than a claim is for there to be one
    /// copy of the code that sets it.
    ///
    /// Note the asymmetry, which is load-bearing and predates this lane: an
    /// image-0 failure ends the window, while a later image's failure leaves
    /// the bit clear and lets the next image proceed.
    fn copy_window(
        &mut self,
        index: usize,
        image_count: usize,
        first_bit: u8,
        copy: &mut dyn FnMut(u64, &mut [u8]) -> bool,
        calls: &Cell<u64>,
    ) {
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

    /// HOW FAR A RUN STARTING AT `first` REACHES, as an exclusive end index.
    ///
    /// A run is a maximal stretch of sorted, strictly non-overlapping windows
    /// whose hull stays inside `domain`, whose holes are each at most
    /// [`MERGE_MAX_GAP`], and whose total span is at most
    /// [`MERGE_MAX_HULL_BYTES`]. `first + 1` means "no run": the caller copies
    /// that window by itself, on today's path.
    ///
    /// OVERLAPPING AND DUPLICATE-START WINDOWS BREAK THE RUN. `seal`
    /// deduplicates exact `(start, len)` pairs but two windows may still share
    /// a start with different lengths, or overlap. The scatter below would in
    /// fact place them correctly, but refusing them keeps the run a partition
    /// of the packed slab and keeps this function's contract to one sentence.
    fn run_end(&self, first: usize, domain: (u64, u64)) -> usize {
        let hull_start = self.starts[first];
        let mut hull_end = hull_start.saturating_add(u64::from(self.lens[first]));
        if hull_start < domain.0 || hull_end > domain.1 {
            return first + 1;
        }
        let mut end = first + 1;
        while end < self.starts.len() {
            let start = self.starts[end];
            if start < hull_end || start.saturating_sub(hull_end) > MERGE_MAX_GAP {
                break;
            }
            let next_end = start.saturating_add(u64::from(self.lens[end]));
            if next_end > domain.1
                || next_end.saturating_sub(hull_start) > MERGE_MAX_HULL_BYTES as u64
            {
                break;
            }
            hull_end = next_end;
            end += 1;
        }
        end
    }

    /// ONE RUN, EVERY IMAGE, ONE GUARDED CALL PER IMAGE.
    ///
    /// A RUN BEHAVES EXACTLY AS ONE OVERSIZED WINDOW DOES. The loop below is
    /// `copy_window`'s, with the hull in place of the window and a scatter
    /// after each successful image: an image-0 decline ends the run with no bit
    /// set, a later image's decline leaves that image's bit clear and takes the
    /// next image. That is the whole failure contract, and it is the same
    /// sentence `copy_window` already obeys.
    ///
    /// THERE IS NO FALLBACK, and its absence is the point (Codex r1 finding 1,
    /// r2 findings 1 and 2). Two earlier forms re-read the members after a hull
    /// declined — first on any image, then on image 0 only — and BOTH heal a
    /// one-shot refusal: the engine declines one read, the re-read succeeds,
    /// and a window the unmerged program would have left unreadable comes back
    /// present. That direction manufactures data, and no accounting of which
    /// span was asked makes it safe, because a transient fault is indexed by
    /// WHEN a call happens and not by WHAT it asks for.
    ///
    /// What makes the absence affordable is [`merge_domain`]'s second
    /// condition: a run only exists inside a span the frozen snapshot already
    /// holds contiguously and void-free, which is a span this frame has already
    /// read in full and whose region `range_confirmed_good` has already cached.
    /// A hull inside it fails only if the mapping changes mid-frame — the same
    /// event that would fail a member's own read.
    ///
    /// The residual divergence is stated rather than hidden: when that does
    /// happen the whole run loses the image, where the unmerged program would
    /// have lost only the member that faulted. That is strictly MORE
    /// conservative, never less, and it is the direction the recorder's
    /// fail-closed law asks for.
    ///
    /// THE GAP BYTES GO NOWHERE. The hull lands in `scratch` and only member
    /// spans are scattered out of it, so nothing between two windows reaches a
    /// slab, `feed_content`, the geometry gate or the wire.
    fn copy_run(
        &mut self,
        first: usize,
        end: usize,
        image_count: usize,
        first_bit: u8,
        copy: &mut dyn FnMut(u64, &mut [u8]) -> bool,
        calls: &Cell<u64>,
        scratch: &mut Vec<u8>,
    ) {
        let Self {
            starts,
            lens,
            offsets,
            status,
            slabs,
            ..
        } = self;
        let hull_start = starts[first];
        let hull_end = starts[end - 1].saturating_add(u64::from(lens[end - 1]));
        let hull_len = (hull_end - hull_start) as usize;
        if scratch.len() < hull_len {
            scratch.resize(hull_len, 0);
        }
        for image in 0..image_count {
            calls.set(calls.get().saturating_add(1));
            if !copy(hull_start, &mut scratch[..hull_len]) {
                if image == 0 {
                    // `copy_window`'s `break`: the run ends, no bit set.
                    return;
                }
                // `copy_window`'s fall-through: this image's bit stays clear
                // and the next image is taken. No re-read, ever.
                continue;
            }
            let slab = &mut slabs[image];
            for member in first..end {
                let at = (starts[member] - hull_start) as usize;
                let len = lens[member] as usize;
                let to = offsets[member] as usize;
                slab[to..to + len].copy_from_slice(&scratch[at..at + len]);
                status[member] |= first_bit << image;
            }
        }
    }

    fn copy_side(
        &mut self,
        image_count: usize,
        first_bit: u8,
        copy: &mut dyn FnMut(u64, &mut [u8]) -> bool,
        calls: &Cell<u64>,
        policy: MergePolicy,
    ) {
        let Some(domain) = policy.domain else {
            for index in 0..self.starts.len() {
                self.copy_window(index, image_count, first_bit, copy, calls);
            }
            return;
        };
        // Grows to at most `MERGE_MAX_HULL_BYTES` and is reused by every run of
        // this store, so a family costs at most one 4 KiB allocation for the
        // whole side -- and none at all when no run forms.
        let mut scratch: Vec<u8> = Vec::new();
        let mut index = 0usize;
        while index < self.starts.len() {
            let end = self.run_end(index, domain);
            if end - index < 2 {
                self.copy_window(index, image_count, first_bit, copy, calls);
                index = end;
                continue;
            }
            self.copy_run(
                index,
                end,
                image_count,
                first_bit,
                copy,
                calls,
                &mut scratch,
            );
            index = end;
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

    /// Did EVERY window of this store obtain EVERY image, with none retired?
    ///
    /// The rail write-tap's remember-side precondition (lane SN). A store with
    /// a window that lost an image is a store whose reuse would hand the walk
    /// an `Unreadable` where the engine had bytes, and a retired window is one
    /// the alias check already poisoned; neither may be carried into another
    /// hour, so both refuse the memory and re-owe the baseline.
    fn every_window_fully_imaged(&self, image_count: usize) -> bool {
        let wanted = match image_count {
            0 => return self.starts.is_empty(),
            n => ((1u16 << n) - 1) as u8,
        };
        self.status
            .iter()
            .all(|&status| status & ST_DEAD == 0 && status & wanted == wanted)
    }

    /// How many guarded calls an unmerged side of this store would issue.
    /// Used only to price what a reuse avoided.
    fn unmerged_call_count(&self, image_count: usize) -> u64 {
        (self.starts.len() as u64).saturating_mul(image_count as u64)
    }
}

const STORE_HEADER_CLOSE: u8 = D81_FAMILY_COUNT as u8;
const STORE_RECORD_PTR_CLOSE: u8 = D81_FAMILY_COUNT as u8 + 1;
const STORE_COUNT: usize = D81_FAMILY_COUNT + 2;

/// The number of stores [`D81VectorCapture::feed_content`] labels its runs
/// with, and the label it uses for content that belongs to no store.
///
/// Lane TD. `rc_vanilla4` put 125 of the geometry gate's 159 misses in the
/// `Vector` stage, which is every one of these stores fed through one bin, so
/// the manifest could not say which family churned. These names give the miss
/// an owner; `D81_VECTOR_STORE_NAMES` is the wire spelling, in `stores()`
/// order, with the unattributed bin last.
pub const D81_VECTOR_STORE_COUNT: usize = STORE_COUNT;
/// The label [`D81VectorCapture::feed_content`] uses for the capture's own
/// trailing framing. Deliberately `usize::MAX` rather than `STORE_COUNT`, so a
/// consumer that forgets to map it lands out of range instead of silently
/// blaming the last store.
pub const D81_VECTOR_STORE_UNATTRIBUTED: usize = usize::MAX;
/// One bin per store, plus one for [`D81_VECTOR_STORE_UNATTRIBUTED`].
///
/// Taken from the NAME LIST rather than written as `STORE_COUNT + 1`, so the
/// list is what the histogram is sized by: a bin nobody named cannot exist, and
/// the array's own `[&str; STORE_COUNT + 1]` type is what makes forgetting a
/// name a compile error rather than an unnamed column in a manifest.
pub const D81_VECTOR_STORE_BINS: usize = D81_VECTOR_STORE_NAMES.len();
/// Wire names for the bins, in `stores()` order. The last two stores carry the
/// CLOSING images of `RailHeader` and `MapRecordPtr` (see `STORE_FAMILY`), so
/// they are named for that rather than for a family of their own.
pub const D81_VECTOR_STORE_NAMES: [&str; STORE_COUNT + 1] = [
    "rail_header",
    "pair_blob",
    "map_record_ptr",
    "map_anchors",
    "record_key",
    "source_province",
    "source_slot_close",
    "map_slot_close",
    "active_close",
    "rail_header_close",
    "map_record_ptr_close",
    "framing",
];

/// Which bin a [`D81VectorCapture::feed_content`] label falls in.
///
/// Total, so a label this module does not recognise is accounted to the
/// `framing` bin rather than dropped: a miss that vanishes from the histogram
/// is worse than one attributed to the wrong bin, because the histogram's sum
/// is what says the split is complete.
pub fn d81_vector_store_bin(label: usize) -> usize {
    if label < D81_VECTOR_STORE_COUNT {
        label
    } else {
        D81_VECTOR_STORE_COUNT
    }
}

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
    /// HOW MUCH A WIDER MERGE COULD STILL REACH, over the families that do NOT
    /// have a [`MergePolicy`] (lane RY3).
    ///
    /// Those eight are the whole of what the VECTOR PIPELINE still issues --
    /// `d81_plan_copies_issued` counts the substrate's bulk `observe` calls
    /// beside them, about 35 a frame -- and whether extending the merge to them
    /// is worth anything is a question
    /// about the ENGINE'S HEAP LAYOUT that no fixture can answer -- their
    /// windows are small offsets inside separately allocated province objects.
    /// So it is measured instead of argued.
    ///
    ///  * `adjacent_pairs` -- consecutive windows with NO hole between them.
    ///    This is what a merge needs when it may not cross a hole, which is the
    ///    only kind these families could ever have: `merge_domain` certifies a
    ///    gap allowance against a span the frozen snapshot holds whole, and no
    ///    bulk span holds a set of scattered heap objects. A hull over exactly
    ///    adjacent windows reads no byte a member did not already ask for, so
    ///    it needs no certificate at all.
    ///  * `near_pairs` -- consecutive windows within [`MERGE_MAX_GAP`]. Not
    ///    reachable under today's law; recorded so the size of the prize is
    ///    known if a domain for these families is ever established.
    ///
    /// THEY ARE GEOMETRY, NOT A PROMISE (Codex r2 finding 3). A merge also
    /// imports `copy_run`'s failure semantics -- a declined hull costs its
    /// WHOLE RUN that image -- and that trade is not the same for these
    /// families as for the three already merged. Those three read slots of an
    /// array the frame has already read whole and certified, so an individual
    /// window of one cannot be unreadable on its own. These eight read offsets
    /// inside separately allocated province objects, where exactly that CAN
    /// happen: one unreadable object costs one window's image today, and under
    /// a merge would cost every window sharing its run. A large count here is
    /// an argument to look, not an argument to merge.
    ///
    /// Both are WORK-DONE style counters, not content: they are a pure function
    /// of `starts` and `lens`, which `feed_content` already feeds, so the gate
    /// gains nothing by seeing them and is not shown them.
    pub adjacent_pairs: u32,
    pub near_pairs: u32,
    pub closing_tail_start: u64,
    merged: std::sync::OnceLock<MergedWindowIndex>,
    /// THE GEOMETRY CACHE. It remembers where a span's windows ARE, never what
    /// they currently say; see [`GeometryMemo`] for why that line is drawn
    /// where it is. Sound against any later retirement, and against any later
    /// `copy_side`, because both write `status` and neither moves an entry.
    memo: [Cell<Option<GeometryMemo>>; MEMO_WAYS],
    /// WHY THIS FRAME DID OR DID NOT REUSE THE RAIL STORES (lane SN). One of
    /// the `D81_RAIL_TAP_*` ordinals, published as a histogram so a capture can
    /// tell "the rails never moved" from "the detour never fired" from "the
    /// remembered copy did not match".
    pub rail_tap_outcome: u8,
    /// The guarded calls the reuse avoided on this frame, `0` on any frame
    /// that copied. Summed over a capture this is the whole prize, in the same
    /// unit as `d81_plan_copies_issued`, so the two divide.
    pub rail_tap_copies_avoided: u64,
}

/// Feed a slab of back-to-back active windows as their SEMANTIC state.
///
/// The slab is `n * D81_ACTIVE_WINDOW_LEN` bytes by construction (every push in
/// the `ActiveClose` family uses that exact length). A trailing partial window
/// cannot occur, but is fed verbatim if it somehow does: unexplained bytes are
/// compared, never dropped.
fn feed_active_windows(store: usize, slab: &[u8], sink: &mut dyn FnMut(usize, &[u8])) {
    let len = super::D81_ACTIVE_WINDOW_LEN;
    let mut at = 0usize;
    while at + len <= slab.len() {
        let mut window = [0u8; super::D81_ACTIVE_WINDOW_LEN];
        window.copy_from_slice(&slab[at..at + len]);
        let state = super::d81_active_state_from_window(window);
        sink(
            store,
            &[
                state.active_a,
                state.active_b,
                state.active_flags,
                (state.active_u16 & 0xFF) as u8,
                (state.active_u16 >> 8) as u8,
            ],
        );
        at += len;
    }
    if at < slab.len() {
        sink(store, &slab[at..]);
    }
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

    /// Hand every byte of this capture's CONTENT to `sink`, in a fixed order.
    ///
    /// The eleven stores in `stores()` order, then `closing_tail_start`, which
    /// participates in deciding which image a read resolves against.
    ///
    /// DELIBERATELY EXCLUDED, each for a stated reason:
    ///  * `hour` — a different value every hour by construction. Including it
    ///    would make the gate below never fire, which is not a safety property,
    ///    it is the gate not existing. The hour is checked where it belongs, by
    ///    `view_for`, before this capture is used at all.
    ///  * `copies` — a counter of work done, not a byte anything reads.
    ///  * `merged`, `memo` — indices derived from the stores, rebuilt on demand.
    ///  * `pointers_rejected`, `voids`, `aliases_retired` — DEFECT COUNTS. They
    ///    are not content, and the gate does not compare them: it REFUSES
    ///    outright when any of them is nonzero, which is strictly stronger than
    ///    comparing them (two captures with the same defect count need not have
    ///    the same defects).
    ///
    /// The sink's first argument is the index in [`Self::stores`] of the store
    /// the run belongs to, or [`D81_VECTOR_STORE_UNATTRIBUTED`] for the
    /// capture's own trailing framing, which belongs to no family. Nothing
    /// about the BYTES changed when that label was added (lane TD): the label
    /// is not serialized, so an image built before and after this change is the
    /// same image, and the label exists only so a consumer that finds a
    /// difference can say which family it was in.
    pub fn feed_content(&self, sink: &mut dyn FnMut(usize, &[u8])) {
        for (index, store) in self.stores().into_iter().enumerate() {
            // THE ACTIVE-CLOSE FAMILY IS REDUCED, not fed raw (lane PW).
            //
            // Its slabs are a concatenation of `active_window_len` windows, one
            // per railed province, taken at `entry + active_window_off` -- the
            // SAME seven bytes the snapshot's active-table extent carries and
            // the same seven the walk decodes. `d81_active_state_from_window`
            // reduces three of their four fields to zero/nonzero because that
            // is the producer's contract, so an entry whose counter ticked is
            // the same state to every reader.
            //
            // Feeding it raw here would undo the reduction the gate applies to
            // the snapshot side and put the volatile counter straight back into
            // the comparison, which is exactly the miss lane PW measured.
            let reduce = index == D81Family::ActiveClose as usize;
            store.feed_content(index, sink, reduce);
        }
        sink(
            D81_VECTOR_STORE_UNATTRIBUTED,
            &self.closing_tail_start.to_le_bytes(),
        );
    }

    /// Did this capture take every byte it planned to?
    ///
    /// The gate may only reason about a capture with no rejected pointer, no
    /// void and no retired alias; see `feed_content` for why these are refused
    /// rather than compared.
    pub fn is_defect_free(&self) -> bool {
        self.pointers_rejected == 0 && self.voids == 0 && self.aliases_retired == 0
    }

    pub fn owned_window_count(&self) -> usize {
        // PRE-SIZED (lane RY, 2026-08-24). This runs inside d81's clock bracket
        // and exists to produce ONE wire counter, and on `captures/rc_vanilla`
        // it collects ~21,000 windows: an unreserved `Vec` pays a dozen
        // reallocations and ~336 KB of memcpy for them before the sort below
        // even begins. The capacity is an exact upper bound -- the number of
        // windows the six opening stores hold -- and the live filter can only
        // take fewer, so nothing here grows.
        //
        // THE SORT IS STILL THE PHASE'S COST and this does not touch it. Every
        // `FamilyStore` was already sorted and deduplicated by `seal`, so the
        // union's cardinality is a k-way merge over six ordered lists rather
        // than a sort of their concatenation; that is a larger change than a
        // capacity and is named in `runs/ry_d81_worker/MEASUREMENT.md` section 5.
        let capacity: usize = self
            .opening
            .iter()
            .filter(|store| store.images.opening != 0)
            .map(|store| store.starts.len())
            .sum();
        let mut owned: Vec<(u64, u32)> = Vec::with_capacity(capacity);
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

    /// How many LIVE windows of `family` are missing the image at `ordinal`.
    ///
    /// THE DIRECT FORM OF "every closing window was observed at every image".
    /// `tests/d81_authoritative.rs` used to infer that from the closing tail's
    /// CALL COUNT — `tail >= 2 * slot_windows + active_windows` — which was
    /// sound only while the copy program was one call per window per image.
    /// Lane RY2 merges adjacent windows into hull reads, so a run of 512 slots
    /// is two calls rather than 1,024, and the inference no longer holds. The
    /// property it was after is this one: it is about `status`, not about how
    /// many calls set it, and asserting it directly is strictly stronger than
    /// the inequality (which a family of FAILED calls could also satisfy).
    pub fn windows_missing_image(&self, family: D81Family, ordinal: usize) -> usize {
        self.opening[family as usize]
            .status
            .iter()
            .filter(|&&status| status & ST_DEAD == 0 && status & (ST_O << ordinal) == 0)
            .count()
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

    /// Does the FROZEN BULK SNAPSHOT already hold every byte of
    /// `[addr, addr + len)` — contiguously, with no void over it, owned by this
    /// consumer?
    ///
    /// The production implementation is `SnapshotView::bytes(..).is_ok()`,
    /// which copies nothing (it hands back a borrowed slice) and refuses on
    /// exactly the three conditions that matter here: a void intersecting the
    /// span, the span not lying inside ONE captured interval, and the interval
    /// not reaching this consumer.
    ///
    /// WHY THE MERGE NEEDS IT, and why the span list alone was not enough
    /// (Codex r2 finding 3): `roots.bulk_spans` is what the recipe ASKED FOR,
    /// built unconditionally from the roots. It says nothing about whether
    /// those bytes were obtained. A span the recipe requested and then narrowed
    /// to a scoped void is still in that list, and certifying a merge domain
    /// against it would let a hull read across a hole the frame never proved
    /// readable. This asks the copy instead of the request.
    fn covers(&self, addr: u64, len: usize) -> bool;
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

/// HOW MANY OF `store`'S WINDOWS WOULD JOIN A RUN ALREADY IN PROGRESS, at a
/// hole allowance of zero and at [`MERGE_MAX_GAP`], as `(adjacent, near)`.
///
/// Each count is the number of guarded calls a merge at that allowance would
/// remove PER IMAGE when every read succeeds, which is what makes it a prize
/// rather than a shape statistic: the first window of every run still costs its
/// call, and every later member of that run does not.
///
/// IT OBEYS THE HULL CAP, and that is the whole reason it is a function rather
/// than a hole test (Codex r1 finding 5). Counting holes alone reports 512
/// joins for 513 consecutive 8-byte windows, when [`MERGE_MAX_HULL_BYTES`]
/// forces `run_end` to break after 4,096 bytes and only 511 calls actually
/// disappear. The run bookkeeping below is `run_end`'s, so the two agree.
///
/// BOTH ALLOWANCES IN ONE TRAVERSAL. Two calls would touch `starts` and `lens`
/// twice for a diagnostic that runs inside the clock bracket (Codex r2 finding
/// 4); the allowances differ only in the hole test, so they share the walk and
/// carry a hull base each.
///
/// WHAT THE COUNTS ARE NOT: a promise that those calls could be removed with
/// the recording unchanged. They are geometry. Merging a family also imports
/// the failure semantics `copy_run` documents -- a declined hull costs its
/// whole run that image -- and for these families that is a real exposure; see
/// [`D81VectorCapture::adjacent_pairs`].
fn windows_joining_a_run(store: &FamilyStore) -> (u32, u32) {
    const ALLOWANCES: [u64; 2] = [0, MERGE_MAX_GAP];
    let mut joins = [0u32; 2];
    let mut hull_start = [None::<u64>; 2];
    for index in 0..store.starts.len() {
        let start = store.starts[index];
        let end = start.saturating_add(u64::from(store.lens[index]));
        let previous_end = if index == 0 {
            start
        } else {
            store.starts[index - 1].saturating_add(u64::from(store.lens[index - 1]))
        };
        for slot in 0..ALLOWANCES.len() {
            let Some(base) = hull_start[slot] else {
                hull_start[slot] = Some(start);
                continue;
            };
            let joins_the_run = index > 0
                && start >= previous_end
                && start.saturating_sub(previous_end) <= ALLOWANCES[slot]
                && end.saturating_sub(base) <= MERGE_MAX_HULL_BYTES as u64;
            if joins_the_run {
                joins[slot] = joins[slot].saturating_add(1);
            } else {
                hull_start[slot] = Some(start);
            }
        }
    }
    (joins[0], joins[1])
}

/// MAY THIS FAMILY HULL ACROSS THE HOLES IN `span`?
///
/// Only when the frame ALREADY reads all of `span`. A hull that crosses a hole
/// reads bytes no window asked for, and a read of an unmapped page is a fault
/// this recorder must not take; `bulk_spans` is the list of extents the same
/// frame's bulk recipe copies wholesale, so a span contained in one of them is
/// memory this frame has already proved readable.
///
/// THE CHECK IS THE GUARANTEE, not the derivation. The three call sites derive
/// their span from `roots` and the engine offsets, and the derivation could be
/// wrong -- a stride could change, a count could be the wrong one. Testing the
/// derived span here means a wrong derivation DISABLES the merge instead of
/// widening a read.
///
/// TWO CONDITIONS, and they answer different questions:
///
///  * `bulk_spans` contains the span -- the span is one the RECIPE OWNS, so a
///    hull over it is reading bytes inside the plan's own territory rather
///    than somewhere the plan never reasoned about.
///  * `bulk.covers` -- the frozen snapshot ACTUALLY HOLDS those bytes: no
///    void, one contiguous captured interval, owned by this consumer. This is
///    the readability proof, and the first condition cannot supply it because
///    `bulk_spans` is built from the roots unconditionally and lists what was
///    requested (Codex r2 finding 3).
fn merge_domain(
    span: (u64, u64),
    bulk_spans: &[(u64, u64)],
    bulk: &dyn BulkOpeningRead,
) -> MergePolicy {
    let owned = span.0 < span.1
        && bulk_spans
            .iter()
            .any(|&(start, end)| start <= span.0 && span.1 <= end);
    let held = owned && usize::try_from(span.1 - span.0).is_ok_and(|len| bulk.covers(span.0, len));
    MergePolicy {
        domain: held.then_some(span),
    }
}

// ===================== THE RAIL WRITE-TAP (lane SN) =====================
//
// WHAT IT IS. Three of these eleven stores are the rails: `RailHeader` (the
// sixteen bytes at `CProvinceRailwayInfo + 0x38`, i.e. the pair-vector data
// pointer and count), its closing image `header_close`, and `PairBlob` (the
// sorted `(neighbour_province_id, railway_level)` entries). On
// `captures/rc_vanilla5` those three issue between 14,056 and 21,084 of the
// frame's 58,523 guarded copies -- 24% to 36% -- the range being the
// `PairBlob` window count, which no counter in that capture publishes. Times
// the capture's own `d81_plan_us / d81_plan_copies_issued`, which is a
// WHOLE-PHASE QUOTIENT and not a marginal per-call cost, that models 2.0 to
// 3.0 ms of the 8.4 ms `plan` phase, every frame, on a network that almost
// never moves. Both figures are models; `runs/sn_rail_write_tap/MEASUREMENT.md`
// section 3 states what they rest on and `d81_plan_rail_tap_copies_avoided` is
// what measures the call half exactly (Codex r2 finding 3).
//
// WHY A HOOK RATHER THAN A COMPARISON. Lane TD searched the whole image for an
// observable change signal for this data and found none
// (`runs/td_rail_change_signal/DOSSIER.md`): the level is written in place at a
// stable address inside a stable allocation (§4 M2a), the engine's own consumer
// re-compares the vector element by element rather than reading a version word
// (§6.1), and the one rail dirty bit the engine keeps is cleared by its own
// producer inside a single update (§6.2). Its §6.6 states what that search
// could NOT establish -- it is not a proof that no signal exists, and its
// writer census is not closed. That bounded negative is what the minimise-hooks
// ruling asks for, and `lanes/d81_rail_change_gate` is the tap it authorises.
//
// WHAT REUSE DOES AND DOES NOT CHANGE. It removes guarded COPIES and nothing
// else. The bulk recipe still runs, the other eight families are still copied,
// `feed_content` still hands the gate every byte of all eleven stores, and the
// unchanged-geometry gate's verdict is computed exactly as before. What the
// walk reads out of the three rail stores on a reused frame are the bytes the
// last full copy took, which the tap proves are still the bytes in the engine.
//
// THE TWO CONDITIONS, and the second is not the tap. Reuse requires
//
//   1. the tap: no hooked writer has run since the last full rail copy, and
//   2. the GEOMETRY: this frame's own `(province_id, info_pointer)` list --
//      re-derived from the rail table the bulk recipe copies anyway -- is
//      identical to the list the remembered copy was taken from, and every
//      root and offset the rail program depends on is unchanged.
//
// Condition 2 is what makes the manager's table-slot writers
// (`GetOrCreateProvinceInfo`, `CRailwayManager::LoadToken`, the constructor)
// need no hook of their own: their whole mutation is a store into `table[id]`,
// which this list is derived from, once a frame, for free. That is the
// minimise-hooks ruling applied rather than quoted.

/// The reuse outcome for one frame, published as a histogram bin. `0` is the
/// reuse arm; every other value names why the frame copied in full.
pub const D81_RAIL_TAP_REUSED: u8 = 0;
/// The detours are not installed (or the build has none). The pre-tap program.
pub const D81_RAIL_TAP_HOOKS_NOT_INSTALLED: u8 = 1;
/// No full rail copy has been remembered since the tap was reset.
pub const D81_RAIL_TAP_BASELINE_PENDING: u8 = 2;
/// A hooked writer ran; the bytes may have moved.
pub const D81_RAIL_TAP_RAIL_WRITTEN: u8 = 3;
/// The tap was clean but this process holds no remembered copy to reuse.
pub const D81_RAIL_TAP_NO_MEMORY: u8 = 4;
/// The tap was clean but condition 2 failed: a table slot, a root or an offset
/// moved, so the remembered windows are not this frame's windows.
pub const D81_RAIL_TAP_GEOMETRY_CHANGED: u8 = 5;

/// Wire names for the bins, by ordinal.
pub const D81_RAIL_TAP_OUTCOME_NAMES: [&str; 6] = [
    "reused",
    "hooks_not_installed",
    "baseline_pending",
    "rail_written",
    "no_memory",
    "geometry_changed",
];

pub const D81_RAIL_TAP_OUTCOME_COUNT: usize = D81_RAIL_TAP_OUTCOME_NAMES.len();

/// ONE READING OF THE TAP, carried opaquely.
///
/// The copy program stores it beside the remembered stores and hands it back;
/// it never reasons about the numbers. `epoch` counts the detours' entry and
/// exit bumps and `quiet` says no detour was executing. See the lane crate's
/// `gate` module for why a reading pair either side of the copy is what makes
/// reuse sound, and why the dirty bit it replaced was not (Codex r2 finding 1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct D81RailWitness {
    pub epoch: u64,
    pub quiet: bool,
}

/// What the copy bracket asks the write-tap, and tells it.
///
/// A TRAIT RATHER THAN A DIRECT CALL into the lane crate, for one reason: the
/// reuse decision has to be exercised in both directions by a fixture, and a
/// fixture cannot patch a detour into anything.
pub trait D81RailTap {
    /// Called ONCE per copy bracket, before anything else. Returns the outcome
    /// ordinal a GLOBAL condition forces -- [`D81_RAIL_TAP_REUSED`] when none
    /// does. The per-copy currency question is [`Self::still_current`].
    fn open_window(&self) -> u8;
    /// Read the tap right now.
    fn witness(&self) -> D81RailWitness;
    /// May a copy taken between these two readings be remembered? True only
    /// when no hooked writer overlapped the interval between them.
    fn copy_was_quiet(&self, before: D81RailWitness, after: D81RailWitness) -> bool;
    /// Is a copy remembered with `remembered` still the engine's current state?
    fn still_current(&self, remembered: D81RailWitness) -> bool;
    /// A rail copy was remembered. Discharges the baseline obligation.
    fn note_memory_seeded(&self);
    /// The remembered copy is gone. Re-owes the baseline by name.
    fn note_memory_lost(&self);
}

/// The last full rail copy, and everything that decides whether it is still
/// this frame's copy.
///
/// EVERY FIELD IS A PRECONDITION, not a convenience. `railed` is compared
/// element by element rather than by length or by digest: the whole soundness
/// of reuse rests on "the same provinces point at the same objects", and a
/// count or a hash would answer "probably" where the gate this feeds refuses
/// to accept "probably" anywhere else.
pub struct D81RailReuse {
    /// The tap reading taken AFTER the copy that produced these stores, and the
    /// only thing that says they are still the engine's bytes.
    witness: D81RailWitness,
    railed: Vec<(u32, u64)>,
    rail_table: u64,
    rail_slots: u32,
    header_off: u64,
    header_len: u32,
    pairs_ptr_in_window: usize,
    pair_count_in_window: usize,
    pair_stride: u32,
    max_neighbors: u32,
    header: FamilyStore,
    header_close: FamilyStore,
    pairs: FamilyStore,
}

impl D81RailReuse {
    fn matches(
        &self,
        roots: &D81VectorRoots,
        offsets: &D81VectorOffsets,
        railed: &[(u32, u64)],
    ) -> bool {
        self.rail_table == roots.rail_table
            && self.rail_slots == roots.rail_slots
            && self.header_off == offsets.rail_header_window_off
            && self.header_len == offsets.rail_header_window_len
            && self.pairs_ptr_in_window == offsets.pairs_ptr_in_window
            && self.pair_count_in_window == offsets.pair_count_in_window
            && self.pair_stride == offsets.pair_stride
            && self.max_neighbors == offsets.max_neighbors
            && self.railed == railed
    }

    /// The guarded calls a reuse of this memory avoids: every window of the
    /// three rail stores, at every image their unmerged program would take.
    fn avoided_calls(&self) -> u64 {
        self.header
            .unmerged_call_count(2)
            .saturating_add(self.header_close.unmerged_call_count(2))
            .saturating_add(self.pairs.unmerged_call_count(2))
    }
}

/// THE REMEMBERED COPY, held for one frame.
///
/// A process global rather than a field of the domain tracker, deliberately:
/// `d81_vector_observe` is the only reader and the only writer, one d81 copy
/// bracket runs at a time, and threading a carrier through
/// `d81_observe_copy_plan_with` would have added a parameter to twenty fixture
/// call sites that have no tap and no memory.
///
/// BEING A PROCESS GLOBAL IS ALSO ITS ONE HAZARD, and it cost a BLOCKING
/// finding (Codex round 4): every reuse precondition is ADDRESS-shaped, so a
/// boundary that replaces the world WITHOUT changing the addresses -- a
/// campaign rollover, a same-identity forward reload -- left a memory that
/// `still_current` and `matches` both accepted for a different campaign. The
/// two call sites of [`d81_rail_reuse_forget`] are what close that, and
/// `the_campaign_reset_forgets_the_rail_memory_and_re_owes_the_baseline` in
/// `attach_log/tests.rs` is what keeps them there.
static RAIL_REUSE: std::sync::Mutex<Option<D81RailReuse>> = std::sync::Mutex::new(None);

/// Drop the remembered rail copy.
///
/// TWO CALLERS, and both are boundaries where the same addresses may belong to
/// a different world: `reset_campaign_scoped_capture_state!` in `attach_log.rs`
/// (campaign rollover and same-identity forward reload), and the
/// defective-copy arm of [`d81_vector_observe`]. Each pairs it with the lane
/// gate's `note_memory_lost`, so the next bracket copies under a NAMED reason
/// rather than reporting an unexplained `no_memory`.
pub fn d81_rail_reuse_forget() {
    if let Ok(mut slot) = RAIL_REUSE.lock() {
        *slot = None;
    }
}

/// Test-facing: is a copy remembered right now?
#[cfg(test)]
fn d81_rail_reuse_present() -> bool {
    RAIL_REUSE
        .lock()
        .map(|slot| slot.is_some())
        .unwrap_or(false)
}

pub fn d81_vector_observe(
    hour: crate::snapshot_recipes::HourStamp,
    roots: &D81VectorRoots,
    offsets: &D81VectorOffsets,
    bulk: &dyn BulkOpeningRead,
    memory: &mut D81VectorMemory<'_>,
    tap: &dyn D81RailTap,
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
        adjacent_pairs: 0,
        near_pairs: 0,
        closing_tail_start: 0,
        merged: std::sync::OnceLock::new(),
        memo: Default::default(),
        rail_tap_outcome: D81_RAIL_TAP_HOOKS_NOT_INSTALLED,
        rail_tap_copies_avoided: 0,
    };

    // THE BRACKET OPENS HERE, once, before anything is read. This asks only
    // the GLOBAL questions -- are the detours installed, is a baseline owed --
    // because whether the remembered bytes are still the engine's is a question
    // about that memory's own witness, and it is asked below.
    let tap_reason = tap.open_window();
    capture.rail_tap_outcome = tap_reason;

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

    // CONDITION 1 AND CONDITION 2, in that order, and both fail closed.
    //
    // A geometry mismatch DROPS the memory and re-owes the baseline rather
    // than silently copying: the remembered bytes describe a set of objects
    // this frame does not have, so nothing about them may survive into the
    // next decision. A stale WITNESS does not drop it -- the bytes still
    // describe the right objects, they are simply out of date, and the copy
    // this frame is about to take will replace them.
    let mut reused = false;
    if tap_reason == D81_RAIL_TAP_REUSED {
        match RAIL_REUSE.lock() {
            Ok(mut slot) => match slot.as_ref() {
                None => capture.rail_tap_outcome = D81_RAIL_TAP_NO_MEMORY,
                Some(remembered) if !tap.still_current(remembered.witness) => {
                    capture.rail_tap_outcome = D81_RAIL_TAP_RAIL_WRITTEN;
                }
                Some(remembered) if remembered.matches(roots, offsets, &railed) => {
                    capture.rail_tap_copies_avoided = remembered.avoided_calls();
                    capture.opening[D81Family::RailHeader as usize] = remembered.header.clone();
                    capture.header_close = remembered.header_close.clone();
                    capture.opening[D81Family::PairBlob as usize] = remembered.pairs.clone();
                    capture.rail_tap_outcome = D81_RAIL_TAP_REUSED;
                    reused = true;
                }
                Some(_) => {
                    *slot = None;
                    tap.note_memory_lost();
                    capture.rail_tap_outcome = D81_RAIL_TAP_GEOMETRY_CHANGED;
                }
            },
            Err(_) => capture.rail_tap_outcome = D81_RAIL_TAP_NO_MEMORY,
        }
    }

    // THE WITNESS PAIR BRACKETS THE RAIL COPY AND NOTHING ELSE. Everything
    // between these two readings is the three rail stores' guarded calls; the
    // other families are copied after, and a writer running during THEM cannot
    // have touched the bytes this pair is vouching for.
    let witness_before = tap.witness();
    if !reused {
        {
            let store = &mut capture.opening[D81Family::RailHeader as usize];
            for &(_, pointer) in &railed {
                store.push(
                    pointer + offsets.rail_header_window_off,
                    offsets.rail_header_window_len,
                );
            }
            store.seal();
            store.copy_side(
                2,
                ST_O,
                memory.copy,
                &capture.copies,
                MergePolicy::default(),
            );
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
            capture.header_close.copy_side(
                2,
                ST_O,
                memory.copy,
                &capture.copies,
                MergePolicy::default(),
            );
        }

        {
            let store = &mut capture.opening[D81Family::PairBlob as usize];
            for &(ptr, len) in &blobs {
                store.push(ptr, len);
            }
            store.seal();
            store.copy_side(
                2,
                ST_O,
                memory.copy,
                &capture.copies,
                MergePolicy::default(),
            );
        }
    }
    // THE CLOSING HALF OF THE SEQLOCK PAIR. Taken here, immediately after the
    // last rail call and before any other family is touched, so the interval it
    // closes is exactly the rail copy.
    let witness_after = tap.witness();

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
        record.copy_side(
            2,
            ST_O,
            memory.copy,
            &capture.copies,
            MergePolicy::default(),
        );
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
        anchors.copy_side(
            2,
            ST_O,
            memory.copy,
            &capture.copies,
            MergePolicy::default(),
        );
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
        store.copy_side(
            2,
            ST_O,
            memory.copy,
            &capture.copies,
            MergePolicy::default(),
        );
    }

    {
        let opening = &capture.opening[D81Family::MapRecordPtr as usize];
        for index in 0..opening.window_count() {
            capture
                .record_ptr_close
                .push(opening.starts[index], opening.lens[index]);
        }
        capture.record_ptr_close.seal();
        capture.record_ptr_close.copy_side(
            2,
            ST_O,
            memory.copy,
            &capture.copies,
            MergePolicy::default(),
        );
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
        store.copy_side(
            2,
            ST_O,
            memory.copy,
            &capture.copies,
            MergePolicy::default(),
        );
    }

    // THE STAMP IS COMPENSATED FOR A REUSE, and it has to be (lane SN).
    //
    // `closing_tail_start` is a copy-call ORDINAL and `feed_content` hands it
    // to the unchanged-geometry gate, so it is one of the bytes two
    // observations are compared on. A reused frame issues none of the three
    // rail families' calls, so the raw counter here would be up to 21,084 lower
    // than
    // the frame the memory was taken from -- and every frame after a reuse
    // would MISS on that one number alone, buying 3 ms of copy and giving back
    // 20 ms of walk. Adding the avoided calls back restores exactly the value a
    // full copy would have stamped: reuse is only permitted from a memory whose
    // every rail window obtained every image, so that frame issued exactly
    // `windows * images` calls for those three stores, which is what
    // `avoided_calls` counts.
    //
    // `rail_tap_copies_avoided` is zero on every frame that copied, so nothing
    // about the stamp moves on the pre-tap path, and `d81_plan_copies_issued`
    // is deliberately NOT compensated -- that counter is the prize and must
    // show the reduction.
    capture.closing_tail_start = capture
        .copies
        .get()
        .saturating_add(capture.rail_tap_copies_avoided);

    // THE THREE MERGING FAMILIES, and the only three.
    //
    // Each of these closes against SLOTS OF ONE CONTIGUOUS ENGINE ARRAY --
    // `source_array[id]`, `map_array[id]`, `active_table[id]` -- rather than
    // against an offset inside a separately allocated heap object, which is
    // what the other eight families read. That is the whole difference: 3,508
    // windows of one array can be one guarded call, 3,508 windows of 3,508
    // objects cannot.
    //
    // Each span is checked against `roots.bulk_spans` by `merge_domain`, so a
    // wrong derivation here disables the merge rather than widening a read.
    // `attach_log.rs` puts all three in that list: `span(source_array,
    // source_count * 8)`, `span(map_array, map_count * 8)` and
    // `span(active_table, province_slots * D81_ACTIVE_ENTRY_STRIDE)`.
    let source_slot_merge = merge_domain(
        (
            roots.source_array,
            roots
                .source_array
                .saturating_add(u64::from(roots.source_count).saturating_mul(8)),
        ),
        &roots.bulk_spans,
        bulk,
    );
    let map_slot_merge = merge_domain(
        (
            roots.map_array,
            roots
                .map_array
                .saturating_add(u64::from(roots.map_count).saturating_mul(8)),
        ),
        &roots.bulk_spans,
        bulk,
    );
    // The active windows sit at `active_window_off` inside a `active_entry_stride`
    // entry, and `D81_ACTIVE_WINDOW_OFFSET + D81_ACTIVE_WINDOW_LEN` (0x12 + 7) is
    // inside `D81_ACTIVE_ENTRY_STRIDE` (0x20), so every window this family pushes
    // is inside the table extent below.
    let active_merge = merge_domain(
        (
            roots.active_table,
            roots.active_table.saturating_add(
                u64::from(roots.rail_slots).saturating_mul(offsets.active_entry_stride),
            ),
        ),
        &roots.bulk_spans,
        bulk,
    );

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
        store.copy_side(2, ST_O, memory.copy, &capture.copies, source_slot_merge);
    }
    {
        let store = &mut capture.opening[D81Family::MapSlotClose as usize];
        for &(id, _) in &map_provinces {
            store.push(roots.map_array + u64::from(id) * 8, 8);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies, map_slot_merge);
    }
    {
        let store = &mut capture.opening[D81Family::ActiveClose as usize];
        for &(id, _) in &railed {
            let entry = roots.active_table + u64::from(id) * offsets.active_entry_stride;
            store.push(entry + offsets.active_window_off, offsets.active_window_len);
        }
        store.seal();
        store.copy_side(3, ST_O, memory.copy, &capture.copies, active_merge);
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

    // WHAT A WIDER MERGE COULD STILL REACH (lane RY3). One pass over the
    // windows of the families that have no `MergePolicy` -- the eight whose
    // windows are offsets inside separately allocated heap objects, and which
    // are the whole of what `d81_plan_copies_issued` still counts. See the
    // fields for why this is measured rather than argued. `starts` is sorted by
    // `seal`, so consecutive entries are the only pairs that can be adjacent.
    let (mut adjacent, mut near) = (0u32, 0u32);
    let mut measure = |store: &FamilyStore| {
        let (store_adjacent, store_near) = windows_joining_a_run(store);
        adjacent = adjacent.saturating_add(store_adjacent);
        near = near.saturating_add(store_near);
    };
    for (index, store) in capture.opening.iter().enumerate() {
        // The three merged families already collapse; only the rest are open.
        if index == D81Family::SourceSlotClose as usize
            || index == D81Family::MapSlotClose as usize
            || index == D81Family::ActiveClose as usize
        {
            continue;
        }
        measure(store);
    }
    measure(&capture.header_close);
    measure(&capture.record_ptr_close);
    capture.adjacent_pairs = adjacent;
    capture.near_pairs = near;

    // REMEMBER, and only from a copy that is complete in every direction.
    //
    // Placed at the very end because the two defect words this depends on are
    // only final here: `voids` is summed over every family above, and
    // `aliases_retired` is set by the retirement pass. A capture the gate
    // would refuse (`is_defect_free`) is a capture whose rail bytes may be
    // missing, and reusing those next hour would publish an `Unreadable` where
    // the engine had data. Anything short of complete therefore DROPS the
    // memory and re-owes the baseline by name, which costs the next frame a
    // full copy and nothing else.
    // REMEMBERED ONLY WHERE IT COULD EVER BE READ. With the detours absent the
    // tap can never answer current, so remembering would clone the three rail
    // stores -- of the order of half a megabyte on `rc_vanilla5`'s shape, the
    // slabs plus each store's `starts`, `lens`, `offsets` and `status`, and a
    // MODEL either way because no counter publishes a per-family window count
    // (Codex r6 finding 1) -- every frame for a decision nothing will make.
    // And in the offline fixtures,
    // which are exactly the builds with no detours, it would leave one
    // fixture's rail bytes in a process global another fixture then has to
    // step over.
    //
    // AND ONLY FROM A COPY NO WRITER OVERLAPPED (Codex r2 finding 1).
    // `copy_was_quiet` is the seqlock check: the epoch must not have moved
    // across the rail copy and no detour may have been executing at either
    // end. A copy taken while a writer was inside the engine is bytes that may
    // be half-written, and remembering it would republish them for as long as
    // the epoch stayed put.
    if !reused && capture.rail_tap_outcome != D81_RAIL_TAP_HOOKS_NOT_INSTALLED {
        let rail_complete = capture.opening[D81Family::RailHeader as usize]
            .every_window_fully_imaged(2)
            && capture.header_close.every_window_fully_imaged(2)
            && capture.opening[D81Family::PairBlob as usize].every_window_fully_imaged(2);
        let quiet = tap.copy_was_quiet(witness_before, witness_after);
        if capture.is_defect_free() && rail_complete && quiet {
            if let Ok(mut slot) = RAIL_REUSE.lock() {
                *slot = Some(D81RailReuse {
                    witness: witness_after,
                    railed,
                    rail_table: roots.rail_table,
                    rail_slots: roots.rail_slots,
                    header_off: offsets.rail_header_window_off,
                    header_len: offsets.rail_header_window_len,
                    pairs_ptr_in_window: offsets.pairs_ptr_in_window,
                    pair_count_in_window: offsets.pair_count_in_window,
                    pair_stride: offsets.pair_stride,
                    max_neighbors: offsets.max_neighbors,
                    header: capture.opening[D81Family::RailHeader as usize].clone(),
                    header_close: capture.header_close.clone(),
                    pairs: capture.opening[D81Family::PairBlob as usize].clone(),
                });
                // ONLY HERE, and only after the memory is actually held: this
                // is what discharges the baseline obligation, and a baseline
                // discharged against a memory nobody stored would open the
                // gate over nothing.
                tap.note_memory_seeded();
            }
        } else {
            d81_rail_reuse_forget();
            tap.note_memory_lost();
        }
    }

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
        store.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
        assert_eq!(order, vec![0x1000, 0x1000, 0x2000, 0x2000]);
        assert_eq!(calls.get(), 4);
    }

    /// THE STORE LABELS AND THEIR WIRE NAMES ARE ONE LIST (lane TD).
    ///
    /// `feed_content` labels each run with a `stores()` index and the manifest
    /// publishes one named unit per bin, so the two orderings have to be the
    /// same list or a capture reports the churn of one family under another
    /// family's name — which is precisely the reading this histogram exists to
    /// make. Nothing else in the tree ties them together.
    ///
    /// One half is already a compile error rather than an assertion:
    /// `D81_VECTOR_STORE_NAMES` is typed `[&str; STORE_COUNT + 1]`, so a family
    /// added without a name does not build. What is NOT compiler-enforced, and
    /// is what this test carries, is that the label-to-bin mapping is total and
    /// that the names are distinct.
    ///
    /// kills: mapping the unattributed label onto a real store (the `framing`
    /// assertions fail); making `d81_vector_store_bin` partial so an unknown
    /// label silently vanishes from the histogram; giving two bins one name so
    /// the manifest sums them together.
    #[test]
    fn every_store_has_a_bin_and_a_name() {
        assert_eq!(D81_VECTOR_STORE_BINS, D81_VECTOR_STORE_COUNT + 1);
        for index in 0..D81_VECTOR_STORE_COUNT {
            assert_eq!(
                d81_vector_store_bin(index),
                index,
                "a real store must bin to itself"
            );
        }
        assert_eq!(
            d81_vector_store_bin(D81_VECTOR_STORE_UNATTRIBUTED),
            D81_VECTOR_STORE_COUNT,
            "the capture's own framing belongs in the trailing bin"
        );
        assert_eq!(
            D81_VECTOR_STORE_NAMES[D81_VECTOR_STORE_COUNT], "framing",
            "the trailing bin is the one that is not a store"
        );
        // AND EVERY NAME IS DISTINCT: two bins sharing a name would be summed
        // together by the manifest, which aggregates by name.
        let mut seen = D81_VECTOR_STORE_NAMES;
        seen.sort_unstable();
        for pair in seen.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate store name {}", pair[0]);
        }
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
        store.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
        store.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
            adjacent_pairs: 0,
            near_pairs: 0,
            closing_tail_start: 0,
            merged: std::sync::OnceLock::new(),
            memo: Default::default(),
            rail_tap_outcome: D81_RAIL_TAP_HOOKS_NOT_INSTALLED,
            rail_tap_copies_avoided: 0,
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
        blob.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
        slots.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
        header.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
        capture.opening[D81Family::RailHeader as usize] = header;
        let mut record = FamilyStore::new(family_images(D81Family::MapRecordPtr));
        record.push(0x1004, 8);
        record.seal();
        record.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
        record.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
        assert_eq!(record.retire_intersecting(&[(0x1FFC, 0x2010)]), 1);
        capture.opening[D81Family::MapRecordPtr as usize] = record;
        let mut slots = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        slots.push(0x2000, 8);
        slots.seal();
        let mut copy2 = |_addr: u64, out: &mut [u8]| {
            out.fill(3);
            true
        };
        slots.copy_side(2, ST_O, &mut copy2, &calls, MergePolicy::default());
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
        record.copy_side(2, ST_O, &mut fill, &calls, MergePolicy::default());
        aliased.opening[D81Family::MapRecordPtr as usize] = record;
        let mut anchors = FamilyStore::new(family_images(D81Family::MapAnchors));
        anchors.push(0x2000, 8);
        anchors.seal();
        anchors.copy_side(2, ST_O, &mut fill, &calls, MergePolicy::default());
        aliased.opening[D81Family::MapAnchors as usize] = anchors;

        // ONE family over one span, with its closing store at the same
        // address: the case `STORE_FAMILY` exists for, which must serve.
        let mut paired = empty_capture();
        let mut header = FamilyStore::new(family_images(D81Family::RailHeader));
        header.push(0x3000, 16);
        header.seal();
        header.copy_side(2, ST_O, &mut fill, &calls, MergePolicy::default());
        paired.opening[D81Family::RailHeader as usize] = header;
        let mut header_close = FamilyStore::new(FamilyImages {
            opening: 2,
            closing: 0,
        });
        header_close.push(0x3000, 16);
        header_close.seal();
        header_close.copy_side(2, ST_O, &mut fill, &calls, MergePolicy::default());
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
        record.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
        blob.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
        capture.opening[D81Family::PairBlob as usize] = blob;
        assert!(matches!(
            capture.read(0x1100, 8, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
        let mut anchors = FamilyStore::new(family_images(D81Family::MapAnchors));
        anchors.push(0x1100, 24);
        anchors.seal();
        anchors.copy_side(2, ST_O, &mut copy, &calls, MergePolicy::default());
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
            store.copy_side(
                images[store_id],
                ST_O,
                &mut copy,
                &calls,
                MergePolicy::default(),
            );
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
            fn covers(&self, addr: u64, len: usize) -> bool {
                addr.checked_sub(self.base)
                    .and_then(|offset| usize::try_from(offset).ok())
                    .and_then(|offset| offset.checked_add(len))
                    .is_some_and(|end| end <= self.world.len())
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
            // The tap a build with no detours gets: the pre-tap program.
            &ScriptedTap::new(D81_RAIL_TAP_HOOKS_NOT_INSTALLED, true),
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

    // ==============================================================
    // THE RAIL WRITE-TAP'S REUSE (lane SN)
    //
    // The tap's own seqlock rules are unit-tested where they live
    // (`lanes/d81_rail_change_gate/src/gate.rs`). What is tested here is the
    // half that lives in the copy program: that a current witness removes
    // exactly the rail copies and changes NOTHING the gate can see, that every
    // other answer copies in full, and that a memory is only ever remembered
    // from a copy that was complete AND that no writer overlapped.
    // ==============================================================

    /// Serialises the tests below: [`RAIL_REUSE`] is a process global and the
    /// harness runs tests concurrently. Nothing else in the crate populates it
    /// -- every other caller reaches `d81_vector_observe` through
    /// `D81RailTapLive`, which answers `HooksNotInstalled` off-detour and is
    /// therefore refused the remember step -- so this lock is the whole of the
    /// exclusion these need.
    static RAIL_TAP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn rail_tap_lock() -> std::sync::MutexGuard<'static, ()> {
        RAIL_TAP_TEST_LOCK
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// A tap whose answers the test writes down, so the copy program's own
    /// behaviour is what is under test rather than the lane gate's. The gate's
    /// own seqlock rules are unit-tested where they live.
    ///
    /// `epoch` is what every witness this tap hands out reports, and a test
    /// bumps it to stand for a hooked writer having run. `noisy` makes every
    /// witness non-quiet, standing for a writer that is still inside the
    /// engine.
    struct ScriptedTap {
        reason: u8,
        epoch: Cell<u64>,
        noisy: Cell<bool>,
        /// Bumped by the fixture's own copy closure when the rail copy runs,
        /// so the epoch moves BETWEEN the two witness readings rather than on
        /// every reading. That is what makes the mid-copy test sensitive to
        /// WHERE the readings are taken (Codex r5 finding 2).
        epoch_bumps: Cell<u32>,
        seeded: Cell<u32>,
        lost: Cell<u32>,
    }

    impl ScriptedTap {
        /// Stand for a hooked writer arriving right now. Called from the
        /// fixture's copy closure, so the epoch moves at a KNOWN point inside
        /// the copy program rather than on a witness reading.
        fn bump_epoch(&self) {
            self.epoch.set(self.epoch.get().wrapping_add(1));
            self.epoch_bumps.set(self.epoch_bumps.get() + 1);
        }

        /// A tap that will let the copy program reuse, given a matching memory.
        fn new(reason: u8, current: bool) -> Self {
            let tap = Self {
                reason,
                epoch: Cell::new(1),
                noisy: Cell::new(false),
                epoch_bumps: Cell::new(0),
                seeded: Cell::new(0),
                lost: Cell::new(0),
            };
            if !current {
                // A different epoch from the one any memory was seeded under.
                tap.epoch.set(u64::MAX);
            }
            tap
        }
    }

    impl D81RailTap for ScriptedTap {
        fn open_window(&self) -> u8 {
            self.reason
        }
        fn witness(&self) -> D81RailWitness {
            D81RailWitness {
                epoch: self.epoch.get(),
                quiet: !self.noisy.get(),
            }
        }
        fn copy_was_quiet(&self, before: D81RailWitness, after: D81RailWitness) -> bool {
            before.quiet && after.quiet && before.epoch == after.epoch
        }
        fn still_current(&self, remembered: D81RailWitness) -> bool {
            let now = self.witness();
            now.quiet && now.epoch == remembered.epoch
        }
        fn note_memory_seeded(&self) {
            self.seeded.set(self.seeded.get() + 1);
        }
        fn note_memory_lost(&self) {
            self.lost.set(self.lost.get() + 1);
        }
    }

    const RAIL_BASE: u64 = 0x10_0000;
    const RAIL_WORLD_LEN: usize = 0x10000;

    struct RailBulk<'a> {
        world: &'a [u8],
    }

    impl BulkOpeningRead for RailBulk<'_> {
        fn u64_at(&self, addr: u64) -> Option<u64> {
            let offset = addr.checked_sub(RAIL_BASE)? as usize;
            let bytes = self.world.get(offset..offset + 8)?;
            Some(u64::from_le_bytes(bytes.try_into().ok()?))
        }
        fn u32_at(&self, addr: u64) -> Option<u32> {
            let offset = addr.checked_sub(RAIL_BASE)? as usize;
            let bytes = self.world.get(offset..offset + 4)?;
            Some(u32::from_le_bytes(bytes.try_into().ok()?))
        }
        fn u8_at(&self, addr: u64) -> Option<u8> {
            let offset = addr.checked_sub(RAIL_BASE)? as usize;
            self.world.get(offset).copied()
        }
        fn covers(&self, addr: u64, len: usize) -> bool {
            addr.checked_sub(RAIL_BASE)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| offset.checked_add(len))
                .is_some_and(|end| end <= self.world.len())
        }
    }

    fn rail_world() -> Vec<u8> {
        let mut world = vec![0u8; RAIL_WORLD_LEN];
        let put_u64 = |world: &mut [u8], addr: u64, value: u64| {
            let offset = (addr - RAIL_BASE) as usize;
            world[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        };
        let put_u32 = |world: &mut [u8], addr: u64, value: u32| {
            let offset = (addr - RAIL_BASE) as usize;
            world[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put_u64(&mut world, RAIL_BASE + 0x100, RAIL_BASE + 0x1000);
        put_u64(&mut world, RAIL_BASE + 0x108, RAIL_BASE + 0x2000);
        put_u64(&mut world, RAIL_BASE + 0x1038, RAIL_BASE + 0x3000);
        put_u32(&mut world, RAIL_BASE + 0x1044, 1);
        put_u64(&mut world, RAIL_BASE + 0x2038, RAIL_BASE + 0x4000);
        put_u32(&mut world, RAIL_BASE + 0x2044, 2);
        // Distinguishable pair bytes, so a reuse that served the wrong slab
        // would be visible rather than merely equal to zero.
        for (index, byte) in world[0x3000..0x3018].iter_mut().enumerate() {
            *byte = 0xA0 + index as u8;
        }
        world
    }

    fn rail_roots() -> D81VectorRoots {
        D81VectorRoots {
            rail_table: RAIL_BASE + 0x100,
            rail_slots: 2,
            map_array: RAIL_BASE + 0x200,
            map_count: 0,
            source_array: RAIL_BASE + 0x300,
            source_count: 0,
            active_table: RAIL_BASE + 0x5000,
            bulk_spans: Vec::new(),
        }
    }

    fn rail_offsets() -> D81VectorOffsets {
        D81VectorOffsets {
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
        }
    }

    /// One frame. Returns the capture and the addresses the guarded copies
    /// asked for, in order.
    ///
    /// `refuse` is handed the address AND HOW MANY TIMES THAT ADDRESS HAS BEEN
    /// ASKED FOR ALREADY, so a test can fail exactly one IMAGE of one window
    /// (Codex r1 finding 5). Failing every image of a window is a `voids`
    /// defect that `is_defect_free` already catches; failing only image 1 is
    /// not, and it is the case `every_window_fully_imaged` exists for.
    fn rail_frame(
        world: &[u8],
        tap: &ScriptedTap,
        refuse: impl FnMut(u64, usize) -> bool,
    ) -> (D81VectorCapture, Vec<u64>) {
        let (capture, log, _refusals) = rail_frame_counting(world, tap, refuse);
        (capture, log)
    }

    /// [`rail_frame`] over a caller-supplied `roots`, so one test can give the
    /// plan a NON-EMPTY non-rail family. Every other test uses
    /// [`rail_roots`], whose map and source counts are zero.
    fn rail_frame_over(
        world: &[u8],
        roots: &D81VectorRoots,
        tap: &ScriptedTap,
        mut refuse: impl FnMut(u64, usize) -> bool,
    ) -> (D81VectorCapture, Vec<u64>) {
        let offsets = rail_offsets();
        let mut log: Vec<u64> = Vec::new();
        let capture = {
            let mut inner = copy_from(world, RAIL_BASE);
            let mut copy = |addr: u64, out: &mut [u8]| {
                let seen = log.iter().filter(|&&past| past == addr).count();
                log.push(addr);
                if refuse(addr, seen) {
                    return false;
                }
                inner(addr, out)
            };
            let admit =
                |pointer: u64| pointer >= RAIL_BASE && pointer < RAIL_BASE + RAIL_WORLD_LEN as u64;
            let mut memory = D81VectorMemory {
                copy: &mut copy,
                admit: &admit,
            };
            let bulk = RailBulk { world };
            d81_vector_observe(
                crate::snapshot_recipes::HourStamp(7),
                roots,
                &offsets,
                &bulk,
                &mut memory,
                tap,
            )
        };
        (capture, log)
    }

    /// [`rail_frame`], and HOW MANY CALLS THE CLOSURE ACTUALLY REFUSED.
    ///
    /// A test that scripts a refusal and never checks it fired proves nothing
    /// when the call it aimed at stops being issued (Codex r2 finding 5): the
    /// expected "not remembered" would then come from the missing call rather
    /// than from the refusal.
    fn rail_frame_counting(
        world: &[u8],
        tap: &ScriptedTap,
        mut refuse: impl FnMut(u64, usize) -> bool,
    ) -> (D81VectorCapture, Vec<u64>, usize) {
        let mut refusals = 0usize;
        let roots = rail_roots();
        let offsets = rail_offsets();
        let mut log: Vec<u64> = Vec::new();
        let capture = {
            let mut inner = copy_from(world, RAIL_BASE);
            let mut copy = |addr: u64, out: &mut [u8]| {
                let seen = log.iter().filter(|&&past| past == addr).count();
                log.push(addr);
                if refuse(addr, seen) {
                    refusals += 1;
                    return false;
                }
                inner(addr, out)
            };
            let admit =
                |pointer: u64| pointer >= RAIL_BASE && pointer < RAIL_BASE + RAIL_WORLD_LEN as u64;
            let mut memory = D81VectorMemory {
                copy: &mut copy,
                admit: &admit,
            };
            let bulk = RailBulk { world };
            d81_vector_observe(
                crate::snapshot_recipes::HourStamp(7),
                &roots,
                &offsets,
                &bulk,
                &mut memory,
                tap,
            )
        };
        (capture, log, refusals)
    }

    fn rail_image(capture: &D81VectorCapture) -> Vec<u8> {
        let mut image = Vec::new();
        capture.feed_content(&mut |_store, bytes| image.extend_from_slice(bytes));
        image
    }

    /// The addresses only the three rail stores ever ask for, with the number
    /// of guarded calls a FULL copy issues for each: four for a header window
    /// (two opening images plus two closing) and two for a pair blob.
    ///
    /// Exact counts rather than "was it copied at all" (Codex r2 finding 5): a
    /// fallback that copied image 0 and dropped image 1 passes a `contains`
    /// check and is a half-observed window.
    fn rail_call_counts() -> [(u64, usize); 4] {
        [
            (RAIL_BASE + 0x1038, 4),
            (RAIL_BASE + 0x2038, 4),
            (RAIL_BASE + 0x3000, 2),
            (RAIL_BASE + 0x4000, 2),
        ]
    }

    fn calls_for(log: &[u64], address: u64) -> usize {
        log.iter().filter(|&&seen| seen == address).count()
    }

    /// Seed a remembered copy and return its image.
    fn seed_memory(world: &[u8]) -> Vec<u8> {
        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_BASELINE_PENDING, true);
        let (capture, _log) = rail_frame(world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_BASELINE_PENDING);
        assert_eq!(tap.seeded.get(), 1, "a complete copy is remembered");
        assert!(d81_rail_reuse_present());
        rail_image(&capture)
    }

    #[test]
    fn a_clean_tap_reuses_the_rails_and_the_gate_cannot_tell() {
        // kills: a reuse that changes ANY byte the unchanged-geometry gate
        // compares. The gate keys on `feed_content` over all eleven stores, so
        // if the reused image differed from the copied one by a single byte,
        // every gated frame after a reuse would MISS -- the tap would buy back
        // some of the copy and give back the whole walk, which on
        // `rc_vanilla5`'s shape is the larger of the two by an order of
        // magnitude (runs/sn_rail_write_tap/MEASUREMENT.md section 3).
        let _g = rail_tap_lock();
        let world = rail_world();
        let copied = seed_memory(&world);

        let tap = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
        let (capture, log) = rail_frame(&world, &tap, |_, _| false);

        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_REUSED);
        assert_eq!(rail_image(&capture), copied, "the image must be identical");
        for (address, _) in rail_call_counts() {
            assert_eq!(
                calls_for(&log, address),
                0,
                "{address:#x} was copied on a reused frame"
            );
        }
        // THE ONE FED VALUE A REUSE COULD HAVE MOVED, asserted directly as
        // well as through the image above: `closing_tail_start` is a copy-call
        // ordinal, the three rail stores issue twelve of this fixture's calls,
        // and without the compensation at the stamp it would read 0 here and
        // 12 on the copied frame -- one number, and every gated frame after a
        // reuse would miss on it.
        assert_eq!(capture.closing_tail_start, 12);
        // Two header windows, two closing images of them, two pair blobs, at
        // two images each.
        assert_eq!(capture.rail_tap_copies_avoided, 12);
        assert_eq!(capture.copies.get(), 6, "only the active table remains");
        assert_eq!(
            tap.seeded.get(),
            0,
            "a reuse copies nothing, so it consumes no marks"
        );
        assert_eq!(tap.lost.get(), 0);
    }

    #[test]
    fn a_reused_frame_serves_the_remembered_bytes_to_the_walk() {
        // kills: a reuse that carries the geometry but not the slabs, which
        // would answer `Unreadable` where the copy answered `Served` and refuse
        // every rail read of the hour.
        let _g = rail_tap_lock();
        let world = rail_world();
        seed_memory(&world);

        let tap = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
        let (capture, _log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_REUSED);

        let blob = RAIL_BASE + 0x3000;
        match capture.read(blob, 8, VectorImage::Opening) {
            VectorAnswer::Served(bytes) => {
                assert_eq!(bytes, &world[0x3000..0x3008], "the engine's own bytes");
            }
            other => panic!("reused pair blob answered {other:?}"),
        }
        assert!(matches!(
            capture.read(RAIL_BASE + 0x1038, 0x10, VectorImage::Closing),
            VectorAnswer::Served(_)
        ));
    }

    #[test]
    fn every_answer_but_clean_copies_the_rails_in_full() {
        // kills: reusing on any arm the tap did not clear. Each of these is a
        // fail-closed reason and each must cost the guarded calls.
        let _g = rail_tap_lock();
        let world = rail_world();
        for (reason, clean) in [
            (D81_RAIL_TAP_HOOKS_NOT_INSTALLED, true),
            (D81_RAIL_TAP_BASELINE_PENDING, true),
            (D81_RAIL_TAP_RAIL_WRITTEN, true),
        ] {
            seed_memory(&world);
            let tap = ScriptedTap::new(reason, clean);
            let (capture, log) = rail_frame(&world, &tap, |_, _| false);
            assert_eq!(capture.rail_tap_outcome, reason, "reason {reason}");
            assert_eq!(capture.rail_tap_copies_avoided, 0, "reason {reason}");
            for (address, calls) in rail_call_counts() {
                assert_eq!(
                    calls_for(&log, address),
                    calls,
                    "reason {reason}: {address:#x} must be copied at EVERY image",
                );
            }
        }
    }

    #[test]
    fn a_write_since_the_remembered_copy_is_seen_by_the_reuse_decision() {
        // kills: opening the bracket, finding no GLOBAL reason to copy, and
        // reusing without ever asking whether the remembered bytes are still
        // the engine's. `open_window` answers "nothing global forbids it"; only
        // `still_current` answers "and nothing has been written since".
        let _g = rail_tap_lock();
        let world = rail_world();
        seed_memory(&world);
        let tap = ScriptedTap::new(D81_RAIL_TAP_REUSED, false);
        let (capture, log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_RAIL_WRITTEN);
        assert_eq!(capture.rail_tap_copies_avoided, 0);
        assert_eq!(calls_for(&log, RAIL_BASE + 0x3000), 2);
        // The memory is NOT dropped by a stale witness -- it describes the
        // right objects and this frame's copy is about to replace it -- and the
        // frame that copied re-seeds it.
        assert_eq!(tap.lost.get(), 0);
        assert_eq!(tap.seeded.get(), 1);
    }

    #[test]
    fn a_copy_taken_while_a_writer_is_inside_the_engine_is_never_remembered() {
        // kills: THE round-2 defect, at the copy program's own boundary. The
        // dirty-bit model this replaced could not see a writer that was STILL
        // RUNNING when the copy ran: its mark had already been consumed by an
        // earlier bracket, so the copy looked clean and was remembered, and
        // the frame after it republished half-written bytes.
        //
        // The witness pair sees it: `before` is not quiet.
        let _g = rail_tap_lock();
        let world = rail_world();
        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_BASELINE_PENDING, true);
        tap.noisy.set(true);
        let (capture, _log) = rail_frame(&world, &tap, |_, _| false);
        assert!(capture.is_defect_free(), "the copy itself succeeded");
        assert_eq!(tap.seeded.get(), 0, "and must still not be remembered");
        assert_eq!(tap.lost.get(), 1, "the baseline is re-owed by name");
        assert!(!d81_rail_reuse_present());
    }

    #[test]
    fn a_writer_that_starts_during_the_copy_is_never_remembered() {
        // kills: taking one witness instead of two, AND taking both on the
        // same side of the rail copy. A writer that enters between the first
        // reading and the last rail call leaves both readings QUIET -- the
        // copy program cannot see it in a single snapshot -- and only the
        // epoch moving ACROSS the pair catches it.
        //
        // THE EPOCH MOVES FROM INSIDE THE COPY, not on a witness read (Codex
        // r5 finding 2). An earlier form bumped on every reading, which would
        // have passed just as well with both witnesses moved after the copy --
        // the mutation this test's name is about. Here the bump happens when
        // the first rail header window is asked for, so it lies strictly
        // between a correctly placed pair and outside a misplaced one.
        let _g = rail_tap_lock();
        let world = rail_world();
        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_BASELINE_PENDING, true);
        let (capture, _log) = rail_frame(&world, &tap, |addr, seen| {
            if addr == RAIL_BASE + 0x1038 && seen == 0 {
                tap.bump_epoch();
            }
            false
        });
        assert_eq!(tap.epoch_bumps.get(), 1, "the fixture must have bumped once");
        assert!(capture.is_defect_free(), "the copy itself succeeded");
        assert_eq!(tap.seeded.get(), 0, "the epoch moved across the copy");
        assert_eq!(tap.lost.get(), 1);
        assert!(!d81_rail_reuse_present());
    }

    #[test]
    fn a_writer_arriving_after_the_rail_copy_does_not_stop_it_being_remembered() {
        // kills: a closing witness placed too WIDE. A writer that arrives once
        // the rail copy is done cannot have touched the rail bytes, and a pair
        // that spanned it would refuse a perfectly good memory on every busy
        // frame.
        //
        // THE BUMP IS ON THE VERY FIRST NON-RAIL CALL, which is why this
        // fixture gives the plan a non-empty `map_count` (Codex r6 finding 2).
        // An earlier form bumped during the ACTIVE TABLE -- the last family --
        // so `witness_after` could have been moved past five whole families and
        // still passed. `MapRecordPtr` is the first store the copy program
        // touches after `PairBlob`, so the closing witness is now pinned to
        // within one family on this side, and to within none on the other by
        // `a_writer_that_starts_during_the_copy_is_never_remembered`.
        let _g = rail_tap_lock();
        let mut world = rail_world();
        // Two map provinces, so `MapRecordPtr` has windows to copy. Their
        // record pointers stay zero, which the copy program simply skips.
        let map_a = RAIL_BASE + 0x7000;
        let map_b = RAIL_BASE + 0x8000;
        world[0x200..0x208].copy_from_slice(&map_a.to_le_bytes());
        world[0x208..0x210].copy_from_slice(&map_b.to_le_bytes());
        let roots = D81VectorRoots {
            map_count: 2,
            ..rail_roots()
        };
        let first_non_rail = map_a + 0x11B8;

        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_BASELINE_PENDING, true);
        let (capture, log) = rail_frame_over(&world, &roots, &tap, |addr, seen| {
            if addr == first_non_rail && seen == 0 {
                tap.bump_epoch();
            }
            false
        });
        assert_eq!(
            tap.epoch_bumps.get(),
            1,
            "the fixture must have reached the first non-rail window",
        );
        assert_eq!(
            calls_for(&log, first_non_rail),
            4,
            "MapRecordPtr takes two images and `record_ptr_close` two more",
        );
        assert!(capture.is_defect_free());
        assert_eq!(tap.seeded.get(), 1, "the rail copy was still quiet");
        assert_eq!(tap.lost.get(), 0);
        assert!(d81_rail_reuse_present());
    }

    #[test]
    fn a_moved_table_slot_refuses_reuse_and_drops_the_memory() {
        // kills: THE table-slot writer argument. `GetOrCreateProvinceInfo` and
        // `CRailwayManager::LoadToken` are deliberately NOT hooked because
        // their whole mutation is a store into `table[id]`, which this frame
        // re-derives from the bulk copy. If that comparison did not refuse the
        // reuse, those two writers would be unobserved and the minimise-hooks
        // argument for leaving them alone would be false.
        let _g = rail_tap_lock();
        let mut world = rail_world();
        seed_memory(&world);

        // The engine hands province 1 a NEW CProvinceRailwayInfo.
        let moved = RAIL_BASE + 0x6000;
        world[0x108..0x110].copy_from_slice(&moved.to_le_bytes());
        world[0x6038..0x6040].copy_from_slice(&(RAIL_BASE + 0x4000).to_le_bytes());
        world[0x6044..0x6048].copy_from_slice(&2u32.to_le_bytes());

        let tap = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
        let (capture, log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_GEOMETRY_CHANGED);
        assert_eq!(tap.lost.get(), 1, "the memory is dropped by name");
        // EXACT COUNTS over the MOVED geometry (Codex r5 finding 3): the new
        // header object at every image, the old one at none, and both blobs.
        for (address, calls) in [
            (RAIL_BASE + 0x1038, 4usize),
            (RAIL_BASE + 0x6038, 4),
            (RAIL_BASE + 0x2038, 0),
            (RAIL_BASE + 0x3000, 2),
            (RAIL_BASE + 0x4000, 2),
        ] {
            assert_eq!(calls_for(&log, address), calls, "{address:#x}");
        }
        // The frame that refused is itself a complete copy, so it re-seeds.
        assert_eq!(tap.seeded.get(), 1);
        assert!(d81_rail_reuse_present());
    }

    #[test]
    fn a_clean_tap_with_nothing_remembered_copies_and_says_so() {
        // kills: treating an empty memory as a reuse, which would publish an
        // empty rail network for the hour.
        let _g = rail_tap_lock();
        let world = rail_world();
        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
        let (capture, log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_NO_MEMORY);
        assert_eq!(capture.rail_tap_copies_avoided, 0);
        // EXACT COUNTS, like the other fail-closed arms (Codex r5 finding 3):
        // "was it copied at all" would pass for a partial copy.
        for (address, calls) in rail_call_counts() {
            assert_eq!(calls_for(&log, address), calls, "{address:#x}");
        }
    }

    #[test]
    fn an_incomplete_rail_copy_is_never_remembered_whichever_store_lost_it() {
        // kills: remembering a copy with a hole. Reusing a window that lost an
        // image would answer `Unreadable` or `SampleAbsent` for that province
        // every hour until the next rail write, which is data loss the tap
        // invented.
        //
        // AND IT COVERS ALL THREE STORES AND BOTH IMAGES, because they are not
        // caught by the same thing (Codex r1 finding 5). `capture.voids` and
        // `is_defect_free` only ever test IMAGE 0: `d81_vector_observe`'s void
        // loop reads `status & (ST_O | ST_DEAD)`, which is the image-0 bit. A
        // window that took image 0 and lost image 1 therefore produces a
        // DEFECT-FREE capture with a half-observed window, and only
        // `every_window_fully_imaged` refuses it. The `header_close` and
        // `PairBlob` rows are the ones that fail if the remember-side check is
        // narrowed to the opening `RailHeader` store.
        //
        // The occurrence numbers follow the copy program's own order, which
        // `pipeline_order_headers_close_between_blob_decode_and_blob_copy`
        // pins: header opening images 0 and 1, then `header_close` images 0
        // and 1 over the SAME addresses, then the pair blobs.
        let _g = rail_tap_lock();
        let world = rail_world();
        let header_b = RAIL_BASE + 0x2038;
        let blob_b = RAIL_BASE + 0x4000;
        // `expect_calls` pins `copy_window`'s documented asymmetry: an IMAGE-0
        // refusal ends that window (the later images are never asked for),
        // while a later image's refusal leaves the bit clear and the next image
        // still runs. The header rows also cover `header_close`, which re-asks
        // for the SAME two addresses after the opening pair.
        for (label, target, occurrence, expect_void, expect_calls) in [
            // Opening image 0 lost: the window is abandoned (1 call), the blob
            // derived from it is never pushed, and `header_close` still asks
            // for its own two.
            ("RailHeader image 0", header_b, 0usize, true, 3usize),
            ("RailHeader image 1", header_b, 1, false, 4),
            ("header_close image 0", header_b, 2, false, 3),
            ("header_close image 1", header_b, 3, false, 4),
            // A PairBlob window that lost image 0 IS a void: the final void
            // loop walks every `opening` family, and `PairBlob` is one.
            ("PairBlob image 0", blob_b, 0, true, 1),
            ("PairBlob image 1", blob_b, 1, false, 2),
        ] {
            d81_rail_reuse_forget();
            let tap = ScriptedTap::new(D81_RAIL_TAP_BASELINE_PENDING, true);
            let (capture, log, refusals) = rail_frame_counting(&world, &tap, |addr, seen| {
                addr == target && seen == occurrence
            });
            // THE REFUSAL MUST HAVE FIRED (Codex r2 finding 5). If the call
            // this case aims at ever stops being issued, the "not remembered"
            // below would come from the MISSING call rather than from the lost
            // image, and the case would pass while testing nothing.
            assert_eq!(refusals, 1, "{label}: the scripted refusal did not fire");
            assert_eq!(
                calls_for(&log, target),
                expect_calls,
                "{label}: the copy program issued the wrong number of calls, so                  the refusal did not land where this case says it did",
            );
            assert_eq!(
                capture.voids > 0,
                expect_void,
                "{label}: only an image-0 loss is a void, which is exactly why \
                 the remember side cannot lean on `is_defect_free` alone",
            );
            assert_eq!(tap.seeded.get(), 0, "{label}: nothing may be remembered");
            assert_eq!(tap.lost.get(), 1, "{label}: the baseline is re-owed by name");
            assert!(!d81_rail_reuse_present(), "{label}");

            // The next clean bracket therefore copies, under a name.
            let next = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
            let (capture, log) = rail_frame(&world, &next, |_, _| false);
            assert_eq!(
                capture.rail_tap_outcome, D81_RAIL_TAP_NO_MEMORY,
                "{label}: the next bracket must find no memory",
            );
            assert!(log.contains(&target), "{label}");
        }
    }

    #[test]
    fn a_frame_with_no_tap_at_all_leaves_the_memory_exactly_as_it_found_it() {
        // kills BOTH directions (Codex r1 finding 5). A detour-less build must
        // not SEED the process global -- every fixture in this crate is such a
        // build, and one fixture's rail bytes sitting in a global is something
        // the next has to step over -- and it must not CLEAR one either, which
        // an earlier form of this test could not have seen because it started
        // from an empty memory.
        let _g = rail_tap_lock();
        let world = rail_world();

        // Direction one: nothing remembered stays nothing remembered.
        d81_rail_reuse_forget();
        let tap = ScriptedTap::new(D81_RAIL_TAP_HOOKS_NOT_INSTALLED, true);
        let (capture, _log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_HOOKS_NOT_INSTALLED);
        assert_eq!(tap.seeded.get(), 0);
        assert_eq!(tap.lost.get(), 0);
        assert!(!d81_rail_reuse_present(), "a detour-less frame must not seed");

        // Direction two: a memory that IS held survives untouched, and is
        // still the memory a later clean bracket reuses.
        let seeded = seed_memory(&world);
        let tap = ScriptedTap::new(D81_RAIL_TAP_HOOKS_NOT_INSTALLED, true);
        let (capture, _log) = rail_frame(&world, &tap, |_, _| false);
        assert_eq!(capture.rail_tap_outcome, D81_RAIL_TAP_HOOKS_NOT_INSTALLED);
        assert_eq!(tap.lost.get(), 0, "a detour-less frame must not clear");
        assert!(d81_rail_reuse_present());
        let clean = ScriptedTap::new(D81_RAIL_TAP_REUSED, true);
        let (reused, _log) = rail_frame(&world, &clean, |_, _| false);
        assert_eq!(reused.rail_tap_outcome, D81_RAIL_TAP_REUSED);
        assert_eq!(rail_image(&reused), seeded);
    }

    #[test]
    fn every_outcome_ordinal_has_exactly_one_name() {
        // kills: a bin published under the wrong name, or an ordinal past the
        // end of the histogram.
        let ordinals = [
            D81_RAIL_TAP_REUSED,
            D81_RAIL_TAP_HOOKS_NOT_INSTALLED,
            D81_RAIL_TAP_BASELINE_PENDING,
            D81_RAIL_TAP_RAIL_WRITTEN,
            D81_RAIL_TAP_NO_MEMORY,
            D81_RAIL_TAP_GEOMETRY_CHANGED,
        ];
        assert_eq!(ordinals.len(), D81_RAIL_TAP_OUTCOME_COUNT);
        let mut seen = Vec::new();
        for (index, ordinal) in ordinals.iter().enumerate() {
            assert_eq!(usize::from(*ordinal), index, "ordinals must be dense");
            let name = D81_RAIL_TAP_OUTCOME_NAMES[index];
            assert!(!name.is_empty());
            assert!(!seen.contains(&name), "{name} is used twice");
            seen.push(name);
        }
    }
    // ==============================================================
    // THE COPY-WINDOW MERGE (lane RY2)
    //
    // Every test below compares the merged copy program against the
    // UNMERGED one on the same fixture and the same closure. That is the
    // shape the proof needs: the merge is only allowed to change how many
    // guarded calls are issued, never a byte of a slab and never a bit of
    // `status`.
    // ==============================================================

    const SLOT_BASE: u64 = 0x40_0000;
    const WORLD_LEN: usize = 0x4000;

    /// A world whose every byte is derived from its own ADDRESS, so a byte
    /// that came from the wrong place is identifiable rather than merely
    /// wrong. A constant fill would let a scatter that copied the hull into
    /// the slab pass.
    fn addressed_world() -> Vec<u8> {
        (0..WORLD_LEN)
            .map(|off| (off as u64).wrapping_mul(31).wrapping_add(7) as u8)
            .collect()
    }

    /// Ids carrying all three run shapes in one fixture:
    ///  * `0..300` -- one long consecutive run, the case the lane exists for
    ///  * `308..311` -- a 64-byte hole, inside [`MERGE_MAX_GAP`], so the run
    ///    continues across it
    ///  * `900..902` -- a 4,712-byte hole, far outside the allowance, so the
    ///    run breaks and a second one starts
    fn slot_ids() -> Vec<u32> {
        let mut ids: Vec<u32> = (0..300).collect();
        ids.extend([308, 309, 310]);
        ids.extend([900, 901]);
        ids
    }

    fn slot_store(ids: &[u32]) -> FamilyStore {
        let mut store = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        for &id in ids {
            store.push(SLOT_BASE + u64::from(id) * 8, 8);
        }
        store.seal();
        store
    }

    /// A bulk reader that certifies `[SLOT_BASE, SLOT_BASE + held)` and
    /// nothing else -- the fixture's stand-in for the frozen snapshot.
    struct HeldBulk {
        held: usize,
    }

    impl BulkOpeningRead for HeldBulk {
        fn u64_at(&self, _addr: u64) -> Option<u64> {
            None
        }
        fn u32_at(&self, _addr: u64) -> Option<u32> {
            None
        }
        fn u8_at(&self, _addr: u64) -> Option<u8> {
            None
        }
        fn covers(&self, addr: u64, len: usize) -> bool {
            addr.checked_sub(SLOT_BASE)
                .and_then(|offset| usize::try_from(offset).ok())
                .and_then(|offset| offset.checked_add(len))
                .is_some_and(|end| end <= self.held)
        }
    }

    /// The policy the three production families get: a span the recipe owns
    /// AND that the frozen snapshot actually holds -- exactly the two
    /// conditions `d81_vector_observe` puts on it.
    fn slot_policy() -> MergePolicy {
        merge_domain(
            (SLOT_BASE, SLOT_BASE + 1000 * 8),
            &[(SLOT_BASE, SLOT_BASE + WORLD_LEN as u64)],
            &HeldBulk { held: WORLD_LEN },
        )
    }

    /// Run one store's side and return the guarded calls it issued, as
    /// `(addr, len)` in order.
    fn run_side(
        store: &mut FamilyStore,
        images: usize,
        policy: MergePolicy,
        world: &[u8],
        mut refuse: impl FnMut(u64, usize) -> bool,
    ) -> Vec<(u64, usize)> {
        let mut log: Vec<(u64, usize)> = Vec::new();
        let calls = Cell::new(0);
        {
            let mut inner = copy_from(world, SLOT_BASE);
            let mut copy = |addr: u64, out: &mut [u8]| {
                log.push((addr, out.len()));
                if refuse(addr, out.len()) {
                    return false;
                }
                inner(addr, out)
            };
            store.copy_side(images, ST_O, &mut copy, &calls, policy);
        }
        assert_eq!(
            calls.get() as usize,
            log.len(),
            "every guarded call must be counted exactly once",
        );
        log
    }

    fn assert_same_capture_state(merged: &FamilyStore, plain: &FamilyStore, what: &str) {
        assert_eq!(merged.starts, plain.starts, "{what}: starts moved");
        assert_eq!(merged.lens, plain.lens, "{what}: lens moved");
        assert_eq!(merged.offsets, plain.offsets, "{what}: offsets moved");
        assert_eq!(merged.status, plain.status, "{what}: status differs");
        for image in 0..3 {
            assert_eq!(
                merged.slabs[image], plain.slabs[image],
                "{what}: slab {image} differs",
            );
        }
    }

    /// THE BYTE-IDENTITY PROOF. Same fixture, same closure, policy on and off.
    ///
    /// kills: any error in the scatter's source or destination offset, any
    /// status bit the hull path sets or misses, any slab byte it places
    /// wrongly. All four of `seal`'s outputs are compared too, because the
    /// whole safety argument for `Aliased` and `retire_intersecting` is that
    /// they do not move.
    #[test]
    fn merged_and_unmerged_copy_programs_agree_byte_for_byte() {
        let world = addressed_world();
        let ids = slot_ids();

        let mut plain = slot_store(&ids);
        let plain_log = run_side(&mut plain, 2, MergePolicy::default(), &world, |_, _| false);

        let mut merged = slot_store(&ids);
        let merged_log = run_side(&mut merged, 2, slot_policy(), &world, |_, _| false);

        assert_same_capture_state(&merged, &plain, "clean merge");

        // NOT VACUOUS: the fixture must actually merge, or this test proves
        // only that two identical programs agree.
        assert_eq!(
            plain_log.len(),
            ids.len() * 2,
            "the unmerged program is one call per window per image",
        );
        assert_eq!(
            merged_log.len(),
            4,
            "two runs at two images: ids 0..300 and 308..311 hull into one run \
             across their 64-byte hole, ids 900..902 into a second, so the \
             610-call program becomes four calls",
        );

        // And the served bytes are the world's, at every window.
        let capture = capture_with_slots(merged);
        for &id in &ids {
            let addr = SLOT_BASE + u64::from(id) * 8;
            let at = (addr - SLOT_BASE) as usize;
            for image in [VectorImage::Closing, VectorImage::ClosingSample(1)] {
                assert_eq!(
                    capture.read(addr, 8, image),
                    VectorAnswer::Served(&world[at..at + 8]),
                    "slot {id} at {image:?} must serve the world's own bytes",
                );
            }
        }
    }

    fn capture_with_slots(store: FamilyStore) -> D81VectorCapture {
        let mut capture = empty_capture();
        capture.opening[D81Family::SourceSlotClose as usize] = store;
        capture
    }

    /// HAZARD 1 -- failure semantics, and THE ABSENCE OF A FALLBACK.
    ///
    /// A declining hull ends its run exactly as a declining window ends itself
    /// in `copy_window`: image 0 sets no bit and stops, and no narrower read is
    /// attempted. The run is one oversized window and nothing more.
    ///
    /// kills: any re-read after a guarded decline. Both earlier forms of this
    /// lane had one -- on every image, then on image 0 only -- and both HEAL a
    /// one-shot refusal into data the unmerged program would have refused
    /// (Codex r1 finding 1, r2 finding 1).
    #[test]
    fn a_declining_hull_ends_its_run_and_no_narrower_read_is_attempted() {
        let world = addressed_world();
        let ids = slot_ids();

        // Every hull declines; every individual window would have succeeded.
        let mut merged = slot_store(&ids);
        let merged_log = run_side(&mut merged, 2, slot_policy(), &world, |_, len| len > 8);

        assert_eq!(
            merged_log.len(),
            2,
            "two runs, one declined image-0 hull each, and NOTHING else: any \
             further call is the re-read that manufactures data",
        );
        for &(_, len) in &merged_log {
            assert!(len > 8, "only hulls were attempted");
        }
        assert!(
            merged.status.iter().all(|&status| status == 0),
            "a run whose image 0 declined carries no image at all",
        );
        assert!(
            merged.slabs[0].iter().all(|&byte| byte == 0),
            "and wrote no slab byte",
        );

        // NOT VACUOUS: the same windows read individually all succeed, so the
        // zeroes above are the hull declining and not an unreadable fixture.
        let mut plain = slot_store(&ids);
        run_side(&mut plain, 2, MergePolicy::default(), &world, |_, _| false);
        assert!(
            plain.status.iter().all(|&status| status == (ST_O | ST_S1)),
            "every window of this fixture IS individually readable",
        );
    }

    /// HAZARD 1, the later-image case, and the finding that shaped it.
    ///
    /// Codex r1 finding 1: an earlier form of the fallback cleared the run and
    /// redid it from image 0 whenever ANY hull image declined. Against a
    /// one-shot refusal -- the engine declining a single read, which is what
    /// `tests/d81_authoritative.rs`'s `RefusingAtMemory` stages and what a
    /// decommitted page does in production -- that re-read SUCCEEDS and the
    /// refusal is healed. An absent image becomes a present one, and the frame
    /// seals data the unmerged program would have refused.
    ///
    /// kills: any fallback that re-reads a span the guarded read declined.
    #[test]
    fn a_one_shot_refusal_of_a_later_hull_image_is_not_healed_by_a_retry() {
        let world = addressed_world();
        let ids: Vec<u32> = (0..8).collect();

        let mut refusals = 0usize;
        let mut merged = slot_store(&ids);
        let log = run_side(&mut merged, 2, slot_policy(), &world, |_, len| {
            // Refuse exactly ONCE, at the second hull call -- the run's image 1.
            if len > 8 {
                refusals += 1;
                return refusals == 2;
            }
            false
        });

        assert_eq!(refusals, 2, "the fixture must reach the second hull image");
        assert_eq!(
            log.len(),
            2,
            "two hull calls and NOTHING else: a third call is the retry that \
             would heal the refusal",
        );
        for (index, &status) in merged.status.iter().enumerate() {
            assert_eq!(
                status, ST_O,
                "window {index} must carry image 0 and NOT image 1: the engine \
                 refused image 1 and a refusal is not re-asked",
            );
        }
        // Image 0's bytes are the world's; image 1's slab was never written.
        assert_eq!(merged.slabs[0], world[..ids.len() * 8]);
        assert_eq!(merged.slabs[1], vec![0u8; ids.len() * 8]);

        // And the refusal is visible to a reader as an ABSENT SAMPLE, which is
        // exactly what the unmerged program produces for a window whose later
        // image the engine declined.
        let capture = capture_with_slots(merged);
        assert_eq!(
            capture.read(SLOT_BASE, 8, VectorImage::ClosingSample(1)),
            VectorAnswer::SampleAbsent,
        );
        assert!(matches!(
            capture.read(SLOT_BASE, 8, VectorImage::Closing),
            VectorAnswer::Served(_)
        ));
    }

    /// The image-0 half of the same contract, against a ONE-SHOT refusal --
    /// the shape a re-read would silently repair.
    ///
    /// kills: a `copy_run` that retries after image 0 declines. Under a
    /// transient fault the retry succeeds and the run comes back fully
    /// present, where the unmerged program would have left the faulting window
    /// unreadable. A fault is indexed by WHEN a call happens, not by WHAT it
    /// asks for, so no argument about which span was asked makes the retry
    /// safe.
    #[test]
    fn a_one_shot_refusal_of_the_image_0_hull_is_not_repaired_by_a_retry() {
        let world = addressed_world();
        let ids: Vec<u32> = (0..8).collect();

        let mut refusals = 0usize;
        let mut merged = slot_store(&ids);
        let log = run_side(&mut merged, 2, slot_policy(), &world, |_, len| {
            if len > 8 {
                refusals += 1;
                return refusals == 1;
            }
            false
        });

        assert_eq!(refusals, 1, "exactly one hull call was refused");
        assert_eq!(
            log.len(),
            1,
            "one declined image-0 hull and NOTHING else: a second call is the \
             retry, and it would succeed",
        );
        assert!(
            merged.status.iter().all(|&status| status == 0),
            "the run carries no image",
        );
    }

    /// kills: a scatter that copies the hull into the slab, or that reads the
    /// hull at the member's PACKED offset instead of its address offset. Both
    /// would put a gap byte where a member byte belongs.
    #[test]
    fn gap_bytes_never_reach_a_slab() {
        let world = addressed_world();
        // Two windows with a 56-byte hole between them, inside the allowance.
        let ids = vec![0u32, 8];
        let mut store = slot_store(&ids);
        let log = run_side(&mut store, 2, slot_policy(), &world, |_, _| false);

        assert_eq!(log, vec![(SLOT_BASE, 72), (SLOT_BASE, 72)]);
        // The slab is 16 bytes for two 8-byte windows -- the hole is not in it.
        assert_eq!(store.slabs[0].len(), 16);
        let mut want = Vec::new();
        want.extend_from_slice(&world[0..8]);
        want.extend_from_slice(&world[64..72]);
        assert_eq!(store.slabs[0], want, "slab 0 holds the two members only");
        assert_eq!(store.slabs[1], want, "slab 1 holds the two members only");
        assert!(
            world[8..64].iter().any(|&byte| !want.contains(&byte)),
            "the fixture's hole must carry bytes the members do not, or this \
             test cannot tell a gap byte from a member byte",
        );
    }

    /// HAZARD 3's sibling -- the read-widening guard.
    ///
    /// kills: a run that hulls past the end of the array it was certified
    /// against, which is a read of memory the frame never proved readable.
    #[test]
    fn no_copy_call_leaves_the_certified_domain() {
        let world = addressed_world();
        // Ids that reach the domain's last slot, so a run ending exactly at
        // the boundary is exercised rather than avoided.
        let ids: Vec<u32> = (995..1000).collect();
        let policy = slot_policy();
        let (domain_start, domain_end) = policy.domain.expect("the fixture certifies a domain");
        let mut store = slot_store(&ids);
        let log = run_side(&mut store, 2, policy, &world, |_, _| false);

        assert!(!log.is_empty());
        for &(addr, len) in &log {
            assert!(
                addr >= domain_start && addr.saturating_add(len as u64) <= domain_end,
                "a guarded call for {addr:#x}+{len} left the certified domain \
                 {domain_start:#x}..{domain_end:#x}",
            );
        }
        assert_eq!(
            log.len(),
            2,
            "the five slots must still have merged, or the boundary check is \
             passing by refusing to merge at all",
        );
    }

    /// HAZARD 3 -- the tear bound.
    ///
    /// kills: a run allowed to grow without limit, which widens the interval
    /// `u64_stable_at` judges between a window's two images.
    #[test]
    fn no_copy_call_exceeds_the_hull_cap() {
        let world = addressed_world();
        let ids: Vec<u32> = (0..1000).collect();
        let mut store = slot_store(&ids);
        let log = run_side(&mut store, 2, slot_policy(), &world, |_, _| false);

        for &(_, len) in &log {
            assert!(
                len <= MERGE_MAX_HULL_BYTES,
                "a guarded call asked for {len} bytes against a cap of \
                 {MERGE_MAX_HULL_BYTES}",
            );
        }
        // 1,000 slots of 8 bytes is 8,000 bytes, so the cap must have split
        // them into more than one run per image.
        assert_eq!(
            log.len(),
            2 * 2,
            "8,000 bytes at a 4,096-byte cap is two runs, taken at two images",
        );

        let mut plain = slot_store(&ids);
        run_side(&mut plain, 2, MergePolicy::default(), &world, |_, _| false);
        assert_same_capture_state(&store, &plain, "capped runs");
    }

    /// kills: a gap allowance that swallows an arbitrarily sparse stretch,
    /// which would read a page to collect two slots.
    #[test]
    fn a_hole_wider_than_the_allowance_breaks_the_run() {
        let world = addressed_world();
        // MERGE_MAX_GAP is 128 bytes: id 17 is 128 bytes past id 0's end and
        // merges; id 18 is 136 and does not.
        for (id, runs) in [(17u32, 1usize), (18, 2)] {
            let ids = vec![0u32, id];
            let mut store = slot_store(&ids);
            let log = run_side(&mut store, 2, slot_policy(), &world, |_, _| false);
            assert_eq!(
                log.len(),
                runs * 2,
                "slots 0 and {id} must form {runs} run(s) at a {MERGE_MAX_GAP}-byte allowance",
            );
            let mut plain = slot_store(&ids);
            run_side(&mut plain, 2, MergePolicy::default(), &world, |_, _| false);
            assert_same_capture_state(&store, &plain, "gap allowance");
        }
    }

    /// THE OPT-IN. kills: a merge that leaks onto the eight heap-scattered
    /// families, whose windows are offsets inside separately allocated
    /// objects and whose adjacency is not a fact this lane can prove.
    #[test]
    fn a_family_without_a_certified_domain_is_not_merged() {
        let world = addressed_world();
        let ids = slot_ids();

        // The same span, certified against a bulk list that does not contain
        // it, is no policy at all.
        let uncertified = merge_domain(
            (SLOT_BASE, SLOT_BASE + 1000 * 8),
            &[(SLOT_BASE + 0x1000, SLOT_BASE + 0x2000)],
            &HeldBulk { held: WORLD_LEN },
        );
        assert_eq!(uncertified.domain, None);

        let mut store = slot_store(&ids);
        let log = run_side(&mut store, 2, uncertified, &world, |_, _| false);
        assert_eq!(
            log.len(),
            ids.len() * 2,
            "an uncertified family must issue today's program exactly",
        );
        for &(_, len) in &log {
            assert_eq!(len, 8, "and every call must be one window wide");
        }
    }

    /// kills: the section-3 guard being dropped, which is the one check
    /// standing between a gap-spanning hull and a read of memory the frame
    /// never proved readable.
    #[test]
    fn the_merge_domain_must_be_contained_by_a_span_the_frame_already_copies() {
        let span = (SLOT_BASE + 0x1000, SLOT_BASE + 0x2000);
        let holding = HeldBulk { held: 0x4000 };
        let cases: [(&str, Vec<(u64, u64)>, bool); 6] = [
            ("exactly the span", vec![span], true),
            (
                "a strictly larger span",
                vec![(SLOT_BASE + 0x800, SLOT_BASE + 0x2800)],
                true,
            ),
            ("no spans at all", vec![], false),
            (
                "overlapping the head only",
                vec![(SLOT_BASE + 0x800, SLOT_BASE + 0x1800)],
                false,
            ),
            (
                "overlapping the tail only",
                vec![(SLOT_BASE + 0x1800, SLOT_BASE + 0x2800)],
                false,
            ),
            (
                "two spans that cover it only together",
                vec![
                    (SLOT_BASE + 0x1000, SLOT_BASE + 0x1800),
                    (SLOT_BASE + 0x1800, SLOT_BASE + 0x2000),
                ],
                false,
            ),
        ];
        for (what, spans, want) in cases {
            assert_eq!(
                merge_domain(span, &spans, &holding).domain.is_some(),
                want,
                "{what}",
            );
        }
        assert_eq!(
            merge_domain(
                (SLOT_BASE + 0x1000, SLOT_BASE + 0x1000),
                &[(0, u64::MAX)],
                &holding
            )
            .domain,
            None,
            "an empty span certifies nothing",
        );
    }

    /// THE SECOND CONDITION ALONE (Codex r2 finding 3).
    ///
    /// `roots.bulk_spans` is what the recipe ASKED FOR, built unconditionally
    /// from the roots. A span it lists but the snapshot does not hold -- one
    /// narrowed to a scoped void, or never captured -- must not certify a
    /// merge, because a hull over it would read across a hole the frame never
    /// proved readable.
    ///
    /// kills: dropping the `bulk.covers` half of `merge_domain` and trusting
    /// the span list, which no geometry test can catch.
    #[test]
    fn a_span_the_recipe_asked_for_but_did_not_get_certifies_nothing() {
        let span = (SLOT_BASE, SLOT_BASE + 0x2000);
        // The recipe owns it either way; only the snapshot's answer differs.
        let owned = [(SLOT_BASE, SLOT_BASE + 0x4000)];

        assert!(
            merge_domain(span, &owned, &HeldBulk { held: 0x2000 })
                .domain
                .is_some(),
            "the snapshot holds the whole span, so it certifies",
        );
        assert_eq!(
            merge_domain(span, &owned, &HeldBulk { held: 0x1FFF }).domain,
            None,
            "one byte short is not held, and a partially held span must not \
             certify a merge",
        );
        assert_eq!(
            merge_domain(span, &owned, &HeldBulk { held: 0 }).domain,
            None,
            "a span the snapshot does not hold at all certifies nothing",
        );
    }

    /// The active family's windows sit INSIDE a strided entry rather than
    /// filling it, so its runs cross a hole on every step. kills: a run
    /// builder that only ever merges exactly-adjacent windows, which would
    /// leave d81's largest closing family on today's program.
    #[test]
    fn the_strided_active_family_merges_across_its_intra_entry_holes() {
        let world = addressed_world();
        let stride = 0x20u64;
        let window_off = 0x12u64;
        let window_len = 7u32;
        let table = SLOT_BASE;
        let mut store = FamilyStore::new(family_images(D81Family::ActiveClose));
        for id in 0..64u64 {
            store.push(table + id * stride + window_off, window_len);
        }
        store.seal();
        let policy = merge_domain(
            (table, table + 64 * stride),
            &[(table, table + WORLD_LEN as u64)],
            &HeldBulk { held: WORLD_LEN },
        );
        let log = run_side(&mut store, 3, policy, &world, |_, _| false);
        assert_eq!(
            log.len(),
            3,
            "64 entries at stride 0x20 span 2,048 bytes, inside the hull cap, \
             and each 25-byte hole is inside the gap allowance: one run at \
             three images",
        );

        let mut plain = FamilyStore::new(family_images(D81Family::ActiveClose));
        for id in 0..64u64 {
            plain.push(table + id * stride + window_off, window_len);
        }
        plain.seal();
        run_side(&mut plain, 3, MergePolicy::default(), &world, |_, _| false);
        assert_same_capture_state(&store, &plain, "strided family");
    }

    /// kills: a run builder that merges windows sharing a start, or
    /// overlapping ones, either of which would stop the run being a partition
    /// of the packed slab.
    #[test]
    fn duplicate_starts_and_overlaps_break_the_run() {
        let world = addressed_world();
        let mut store = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        store.push(SLOT_BASE, 8);
        store.push(SLOT_BASE, 16); // same start, longer -- not adjacent
        store.push(SLOT_BASE + 8, 8); // overlaps the 16-byte window
        store.seal();
        assert_eq!(store.starts.len(), 3, "seal keeps all three");

        let log = run_side(&mut store, 2, slot_policy(), &world, |_, _| false);
        assert_eq!(
            log.len(),
            6,
            "three windows at two images, none of them merged",
        );

        let mut plain = FamilyStore::new(family_images(D81Family::SourceSlotClose));
        plain.push(SLOT_BASE, 8);
        plain.push(SLOT_BASE, 16);
        plain.push(SLOT_BASE + 8, 8);
        plain.seal();
        run_side(&mut plain, 2, MergePolicy::default(), &world, |_, _| false);
        assert_same_capture_state(&store, &plain, "duplicate and overlap");
    }
}
