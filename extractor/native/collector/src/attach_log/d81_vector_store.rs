//! d81's scattered per-province observations as VECTOR FAMILIES (lane HS).
//!
//! # Why this module exists
//!
//! The recipe substrate priced d81's plan phase at 41.8 ms per invocation at
//! t50 — 84,305 guarded transactions at ~496 ns of phase wall apiece, with
//! per-interval bookkeeping (ownership, annotation, scheduling, blanket
//! closing comparison) charged on every one of them (`runs/hl_d81_span/`,
//! `runs/hs_d81_cheap/COST_MODEL.txt`). 83,000 of those transactions are the
//! per-railed-province branch: six intervals, four images each, for ~3,513
//! provinces. The owner's 2026-08-13 mandate is to make d81 as cheap as
//! absolutely possible, the four-image structure explicitly included.
//!
//! This module replaces the branch with FIVE-plus window families — flat lists
//! of `(address, length, province)` descriptors copied by a tight loop over
//! the same guarded primitive, into per-family slabs, with per-window,
//! per-image success masks. Bookkeeping is per FAMILY, not per interval.
//!
//! # What is preserved, mechanism by mechanism
//!
//!  * **The A/B tear pair is interleaved PER WINDOW** — `copy A; if A ok,
//!    copy B` before the next window (Codex arch consult, thread
//!    019ff96e-dd04, section 1: chunked double passes were REJECTED because
//!    they widen the pair gap by K-1 foreign operations; interleaving narrows
//!    it below today's, since the executor put allocation and ownership
//!    bookkeeping between the two).
//!  * **The range gate runs before every observation**, inside the loop —
//!    never once per family. PAGE_GUARD pages are rejected by the gate BEFORE
//!    being touched; SEH remains the decommit-race containment only, exactly
//!    as `read_into` has it.
//!  * **Closing images are observed AFTER the reads they vouch for**: the
//!    rail-header closing pair is taken after the pair-blob descriptors are
//!    decoded and before the blobs are copied (the executor's `AfterWave(3)`
//!    timing, reproduced deliberately); the record-pointer closing pair after
//!    the record-key windows; the slot closings and the active-entry closing
//!    triples LAST, after every family that was reached through them. A
//!    closing image taken before the bytes it vouches for would let a
//!    replaced object publish with every check passing, which is the one
//!    thing the closing discipline exists to refuse.
//!  * **Images are demand-driven.** A family holds exactly the images the
//!    walk consumes (the demand table in `COST_MODEL.txt`, confirmed by the
//!    consult): headers and record pointers all four; blobs, anchors, record
//!    keys and `CProvince` windows the opening pair only; slot closings and
//!    active closings closing-side only, because their opening pairs are
//!    served from the bulk arrays the recipe still copies whole.
//!  * **Per-image status dominates lookup.** A window whose copy faulted is
//!    never served — the slab prefix a failed SEH copy may have written is
//!    unreachable, not merely deprecated.
//!  * **Fail-closed coverage.** The store serves ONLY bytes inside captured
//!    windows; an address nobody asked for answers `NotCaptured` and the
//!    walk's misses accounting turns that into the plan-defect class it is.
//!
//! # What is deliberately NOT here
//!
//! No verdicts. The walk renders every tear and closure verdict by comparing
//! images, exactly as it did over the recipe view — this module moves bytes
//! and says which observations exist. No worker threads, no locks, no engine
//! wait: one sequential loop on the worker that owns the bracket.

use std::cell::Cell;

use crate::snapshot_recipes::SnapshotMemory;

/// One window family. The discriminants order the families as the pipeline
/// copies them; the order is documented at each variant and pinned by
/// `d81_vector_pipeline`'s own sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum D81Family {
    /// `[rail_info + 0x38, +0x48)` — the pairs pointer and pair count.
    /// Opening pair, then a closing pair taken after the blob descriptors
    /// are decoded and before the blobs are copied.
    RailHeader = 0,
    /// `pair_count * 8` bytes behind the header's pairs pointer. Opening
    /// pair only: the walk never consumes a blob closing image.
    PairBlob = 1,
    /// `[map_province + 0x11B8, +8)` — the record pointer. All four images;
    /// the closing pair is taken after the record-key family so the key the
    /// walk consumed is bracketed, which is the check's own stated purpose.
    MapRecordPtr = 2,
    /// `[map_province + 0x14E4, +0x14FC)` — both anchor triplets as ONE
    /// 24-byte window (they are adjacent; two windows would be a transaction
    /// for nothing). Opening pair only.
    MapAnchors = 3,
    /// `[record + 0xC4, +4)` — the selector key. Opening pair only.
    RecordKey = 4,
    /// `[cprovince, +0xA8)` — vtable through id. Opening pair only.
    /// Covers the union of railed provinces and decoded selector keys: the
    /// walk resolves the SELECTOR's province, which the recipe plan only
    /// covered when the key was the province's own id.
    SourceProvince = 5,
    /// `[province_array + 8*id, +8)` for every id the walk closes over —
    /// railed ids and decoded selector keys. CLOSING pair only, taken last;
    /// the opening pair is served from the bulk source array.
    SourceSlotClose = 6,
    /// `[map_array + 8*id, +8)` for railed ids. CLOSING pair only, taken
    /// last; the opening pair is served from the bulk map array.
    MapSlotClose = 7,
    /// `[active_entry + 0x12, +7)` for railed ids. CLOSING side only, THREE
    /// images interleaved back-to-back per window — the coherence loop
    /// publishes the first consecutive pair that agrees, so it needs as many
    /// distinct observations closing as it takes live. Opening triples are
    /// served from the bulk active table.
    ActiveClose = 8,
}

pub const D81_FAMILY_COUNT: usize = 9;

/// Which images a family observes. `opening`/`closing` are image COUNTS:
/// 2 means an adjacent pair, 3 the active family's coherence triple.
#[derive(Clone, Copy, Debug, Default)]
struct FamilyImages {
    opening: u8,
    closing: u8,
}

const fn family_images(family: D81Family) -> FamilyImages {
    match family {
        D81Family::RailHeader => FamilyImages { opening: 2, closing: 2 },
        D81Family::PairBlob => FamilyImages { opening: 2, closing: 0 },
        D81Family::MapRecordPtr => FamilyImages { opening: 2, closing: 2 },
        D81Family::MapAnchors => FamilyImages { opening: 2, closing: 0 },
        D81Family::RecordKey => FamilyImages { opening: 2, closing: 0 },
        D81Family::SourceProvince => FamilyImages { opening: 2, closing: 0 },
        D81Family::SourceSlotClose => FamilyImages { opening: 0, closing: 2 },
        D81Family::MapSlotClose => FamilyImages { opening: 0, closing: 2 },
        D81Family::ActiveClose => FamilyImages { opening: 0, closing: 3 },
    }
}

/// Per-window success bits. An image's bit is set only when its guarded copy
/// returned whole; a bit that is clear makes the image unservable whatever
/// the slab holds.
const ST_O: u8 = 1 << 0;
const ST_S1: u8 = 1 << 1;
const ST_C: u8 = 1 << 2;
const ST_CS1: u8 = 1 << 3;
const ST_CS2: u8 = 1 << 4;
/// The window ALIASES a bulk table span and is dead: it is never served, so a
/// read of the aliased address is answered by the bulk view's own images (or
/// refuses `NotCaptured`), never by a scattered window that happens to cover
/// it. See [`D81VectorCapture::retire_bulk_aliases`] — Codex review round 1,
/// finding 1 (BLOCKER): without this, a malformed engine pointer landing a
/// record-pointer window on top of a source-array slot would let BOTH the
/// opening and the closing read of that slot be served from the alias, and a
/// replaced slot could publish with every check passing.
const ST_DEAD: u8 = 1 << 7;

/// One family's windows and images.
///
/// Slabs are laid out window-after-window in descriptor order; `offsets[i]`
/// is window `i`'s start and `offsets[len]` the slab length, so a window's
/// bytes are `slab[offsets[i]..offsets[i] + len_of(i)]` in EVERY image's
/// slab (all images of a window have the window's own length).
#[derive(Default)]
pub struct FamilyStore {
    /// Window start addresses, ascending after `seal`.
    starts: Vec<u64>,
    lens: Vec<u32>,
    /// Prefix offsets into each image slab; `starts.len() + 1` entries.
    offsets: Vec<u32>,
    status: Vec<u8>,
    /// Image slabs, indexed by [`FamilyStore::slab_index`]. Opening images
    /// first, then closing.
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

    /// Slab index for an image ordinal within this family: opening `k` is
    /// slab `k`; closing `k` is slab `k` too when the family has no opening
    /// images, and `2 + 0/1` never occurs because no family holds more than
    /// three images total. Families with both sides (header, record pointer)
    /// hold O/S1 in slabs 0/1 and C/CS1 in a SECOND store created by the
    /// pipeline — see [`D81VectorCapture`], which keeps closing-side windows
    /// of a mixed family in a parallel `FamilyStore` so every store's slabs
    /// are one side only. This keeps the slab arithmetic flat.
    fn slab(&self, ordinal: usize) -> &[u8] {
        &self.slabs[ordinal]
    }

    pub fn window_count(&self) -> usize {
        self.starts.len()
    }

    pub fn total_bytes(&self) -> u64 {
        let per_image = self.offsets.last().copied().unwrap_or(0) as u64;
        let image_count =
            (self.images.opening as u64).saturating_add(self.images.closing as u64);
        per_image.saturating_mul(image_count)
    }

    /// Stage the descriptor list. Descriptors must be pushed before `seal`;
    /// `seal` sorts by start address, DEDUPLICATES identical `(start, len)`
    /// windows (two provinces may legitimately reach one object — the copy
    /// is taken once and serves both readers, exactly as the recipe's
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

    /// Copy one SIDE of this family: `image_count` observations per window,
    /// interleaved back-to-back per window, gate-then-copy each. `first_bit`
    /// is the status bit of the side's first image.
    ///
    /// A first observation that fails ends the window's side — the later
    /// images are not attempted, matching the executor's acquisition (a hole
    /// whose bulk copy failed takes no adjacent samples). A later observation
    /// that fails leaves the earlier ones servable and its own bit clear.
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

    /// The window containing `[addr, addr + len)`, if any.
    ///
    /// Windows are sorted by start after `seal`. The candidate is the last
    /// window starting at or below `addr`; because engine objects can alias
    /// under malformed pointers, up to [`D81_LOOKUP_BACKSCAN`] earlier
    /// windows are also checked for containment rather than assuming
    /// non-overlap. A read no window contains is simply not this family's.
    fn locate(&self, addr: u64, len: u32) -> Option<usize> {
        let position = self.starts.partition_point(|&start| start <= addr);
        if position == 0 {
            return None;
        }
        let lowest = position.saturating_sub(D81_LOOKUP_BACKSCAN);
        for index in (lowest..position).rev() {
            if self.status[index] & ST_DEAD != 0 {
                continue;
            }
            let start = self.starts[index];
            let width = self.lens[index] as u64;
            if addr >= start && addr.saturating_add(u64::from(len)) <= start.saturating_add(width)
            {
                return Some(index);
            }
        }
        None
    }

    /// [`Self::locate`], but a DEAD window's containment is reported too.
    ///
    /// A retired window must POISON its span, never merely vanish (Codex
    /// review round 3): invisible retirement let a dead record-pointer
    /// window's span fall through to the bulk view and to an exempt
    /// closing-only family, bypassing the divergent closing observation the
    /// dead window held. `(live hit, dead containment)`.
    fn locate_with_dead(&self, addr: u64, len: u32) -> (Option<usize>, bool) {
        let position = self.starts.partition_point(|&start| start <= addr);
        let lowest = position.saturating_sub(D81_LOOKUP_BACKSCAN);
        let mut dead = false;
        let mut live = None;
        for index in (lowest..position).rev() {
            let start = self.starts[index];
            let width = self.lens[index] as u64;
            let contains = addr >= start
                && addr.saturating_add(u64::from(len)) <= start.saturating_add(width);
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

    /// Kill every window that intersects one of `spans`. Dead windows are
    /// invisible to [`Self::locate`], so their bytes are unreachable and the
    /// aliased address is answered by whoever legitimately owns it.
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

    /// Serve `len` bytes at `addr` out of image `ordinal` of this side.
    fn serve(&self, index: usize, addr: u64, len: u32, ordinal: usize) -> &[u8] {
        let from = self.offsets[index] as usize + (addr - self.starts[index]) as usize;
        &self.slab(ordinal)[from..from + len as usize]
    }
}

/// How many earlier windows [`FamilyStore::locate`] rechecks for containment.
///
/// Windows within one family are same-shaped (one length per family except
/// the blobs), so a read inside window `i` can only also fall inside a
/// nearby earlier window when engine pointers alias. Blobs are bounded at
/// `MAX_NEIGHBORS_PER_PROVINCE * 8` bytes and every other family's windows
/// are under 0xA8 bytes, so aliased containment beyond a handful of
/// neighbours is impossible by widths alone. Eight is comfortably past that
/// and still O(1).
const D81_LOOKUP_BACKSCAN: usize = 8;

/// What a vector read answered. The walk-facing adapter in
/// `d81_copy_bracket` maps these onto the same miss classes the recipe view
/// produced, so the wire vocabulary does not move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorAnswer<'a> {
    /// The bytes, from the image asked for.
    Served(&'a [u8]),
    /// No window of any family contains the span: nobody asked for these
    /// bytes. The caller falls through to the recipe view (bulk tables) and
    /// the view's own answer stands.
    NotCovered,
    /// The window exists and its governing observation failed: the engine
    /// refused the span. Ruling A's void, at window granularity.
    Unreadable,
    /// The window exists, its opening image was taken, and the requested
    /// ADJACENT sample was refused by the engine — the live second read's
    /// refusal, preserved as its own class.
    SampleAbsent,
    /// The requested image is one this family never observes (a closing
    /// image of a blob, a third sample of a header). The plan did not ask,
    /// so a consumer that reads it has found a plan defect, and it reports
    /// as exactly that.
    NeverObserved,
    /// MORE THAN ONE family's live window contains the span: a malformed
    /// engine pointer has aliased two scattered objects (Codex review round
    /// 2, findings 1 and 2 — serving whichever family matched first let the
    /// family holding the DIVERGENT closing observation go unconsulted, and
    /// a moved pointer could publish). No byte is served from an aliased
    /// span, ever; the walk refuses the province fail-closed.
    Aliased,
}

/// The image a vector read asks for, in the walk's own vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VectorImage {
    Opening,
    Sample(usize),
    Closing,
    ClosingSample(usize),
}

/// The hour's scattered observations: one store per family, plus the
/// closing-side stores of the two mixed families.
///
/// `hour` is stamped at construction from the same bracket-opening stamp the
/// recipe snapshot carries; the walk-facing adapter refuses to serve a
/// capture whose hour is not the frame's own, which keeps hour attribution
/// structural on this path exactly as `view_for` keeps it on the bulk path.
pub struct D81VectorCapture {
    pub hour: crate::snapshot_recipes::HourStamp,
    opening: [FamilyStore; D81_FAMILY_COUNT],
    /// Closing-side stores for the mixed families ([`D81Family::RailHeader`],
    /// [`D81Family::MapRecordPtr`]); the closing-only families keep their
    /// windows in `opening` with closing images in its slabs.
    header_close: FamilyStore,
    record_ptr_close: FamilyStore,
    /// Guarded copy calls issued, successful or not — the same meaning as
    /// `D81PlanCounters::copies_issued`.
    pub copies: Cell<u64>,
    /// Non-null pointers the pipeline declined to follow on the heap-window
    /// predicate. Same meaning as the recipe's `pointers_rejected`.
    pub pointers_rejected: u32,
    /// Windows whose governing observation the engine refused. Same meaning
    /// as the recipe's `scoped_voids`: a recorded void, not a frame refusal.
    pub voids: u32,
    /// Scattered windows retired as bulk-span aliases; see
    /// `d81_vector_observe`'s retirement pass. Diagnostic, not a void and
    /// not a rejection: the refusal the walk renders downstream carries the
    /// wire meaning.
    pub aliases_retired: u32,
    /// How many copies the pipeline had issued when its FINAL closing stages
    /// (the slot and active closings) began — the boundary the offline
    /// mutation gates place writes against, read off the run that took it
    /// exactly as the executor's `final_wave_copies` was.
    pub closing_tail_start: u64,
}

impl D81VectorCapture {
    /// Windows that are their OWN intervals — the scattered opening families'
    /// live windows. Closing-only families sit inside bulk intervals, the
    /// closing stores share their opening stores' spans, and dead aliases own
    /// nothing, so none of those is counted (Codex review round 1, finding
    /// 4: `intervals_closed` must stay a count of distinct owned intervals).
    pub fn owned_window_count(&self) -> usize {
        // Only windows whose opening observation was CAPTURED own an
        // interval, and one (start, len) owns it once however many families
        // minted it (Codex review round 2, finding 4).
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

    /// One family's window count — the executable half of the demand table,
    /// so a gate can pin "how many windows of which family" against a staged
    /// world's own arithmetic instead of against a total that hides drift.
    pub fn family_window_count(&self, family: D81Family) -> usize {
        self.opening[family as usize].window_count()
    }

    /// The closing-side window counts of the two mixed families.
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

    /// Serve a read under the SINGLE-FAMILY rule: the span must be contained
    /// by live windows of exactly ONE family (a mixed family's opening and
    /// closing stores are that one family's two sides). Two families
    /// containing one span is an alias between scattered engine objects, and
    /// nothing is served from it — the family holding a divergent
    /// observation must never be routed around (Codex review round 2,
    /// findings 1 and 2).
    pub fn read(&self, addr: u64, len: u32, image: VectorImage) -> VectorAnswer<'_> {
        // (family id, opening-side store hit, closing-side store hit).
        // The census counts DEAD containment too: a retired window poisons
        // its span rather than vanishing, so a span it contains can neither
        // fall through to the bulk view nor be served by another family
        // (Codex review round 3).
        let mut hit: Option<(usize, Option<usize>, Option<usize>)> = None;
        let mut poisoned = false;
        for family in 0..D81_FAMILY_COUNT {
            let (opening_hit, opening_dead) = self.opening[family].locate_with_dead(addr, len);
            let (closing_hit, closing_dead) = match family {
                f if f == D81Family::RailHeader as usize => {
                    self.header_close.locate_with_dead(addr, len)
                }
                f if f == D81Family::MapRecordPtr as usize => {
                    self.record_ptr_close.locate_with_dead(addr, len)
                }
                _ => (None, false),
            };
            poisoned |= opening_dead || closing_dead;
            if opening_hit.is_none() && closing_hit.is_none() && !opening_dead && !closing_dead {
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
        let store = &self.opening[family];
        match image {
            VectorImage::Opening | VectorImage::Sample(_) => {
                let ordinal = match image {
                    VectorImage::Sample(sample) => sample,
                    _ => 0,
                };
                if store.images.opening == 0 {
                    // A closing-only family's span: the bulk view owns these
                    // bytes' opening images. Falling through is correct, not
                    // lenient.
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
                // Closing-only families keep closing images in their own
                // slabs, bit 0 first.
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
        // The image is absent. If the side's FIRST observation failed the
        // window is a void; if only a later one did, the pair's second
        // observation was refused — a different fact with a different class.
        if store.status[index] & first_bit == 0 {
            VectorAnswer::Unreadable
        } else {
            VectorAnswer::SampleAbsent
        }
    }
}

/// Everything the pipeline needs that the caller already resolved inside the
/// bracket. All addresses are the live reader's own; none is trusted — every
/// pointer decoded along the way re-passes the same heap-window predicate
/// the live reader gates with.
pub struct D81VectorRoots {
    /// The rail table's base and slot count, decoded from the BULK COPY the
    /// recipe took (never re-read live — the pipeline observes the same
    /// table image the walk will).
    pub rail_table: u64,
    pub rail_slots: u32,
    /// The map container's province array and count, from the bulk copy.
    pub map_array: u64,
    pub map_count: u32,
    /// The gamestate province array and count, from the bulk copy.
    pub source_array: u64,
    pub source_count: u32,
    /// The active table's base, from the bulk copy.
    pub active_table: u64,
    /// Every span the BULK recipe owns (the five tables and the fixed
    /// objects). A scattered window intersecting one of these is an ALIAS —
    /// a malformed engine pointer landing inside bulk-owned memory — and is
    /// retired unread, so the bulk images stay the only route to those
    /// bytes (Codex review round 1, finding 1).
    pub bulk_spans: Vec<(u64, u64)>,
}

/// The pipeline's read access to the BULK opening images (rail table slots,
/// array slots) — a closure over the recipe view, so the pipeline decodes
/// from the same bytes the walk will read and never from a second live read.
pub trait BulkOpeningRead {
    /// The opening image's u64 at `addr`, or `None` when the bulk copy does
    /// not hold it (a scoped void in the table; the pipeline skips exactly
    /// the provinces the walk will refuse or skip on the same void).
    fn u64_at(&self, addr: u64) -> Option<u64>;
    fn u32_at(&self, addr: u64) -> Option<u32>;
    fn u8_at(&self, addr: u64) -> Option<u8>;
}

/// Offsets the families are built from — re-exported from the module-scope
/// constants the live reader uses, so the two cannot drift.
pub struct D81VectorOffsets {
    pub rail_header_window_off: u64,
    pub rail_header_window_len: u32,
    /// Pairs pointer / pair count offsets INSIDE the header window.
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

/// The admission predicate and the copy primitive, injected so the offline
/// gates can stage worlds and faults (`D81ExtentPair::copy_with`'s pattern:
/// a tear is the one thing a fixture cannot schedule through a real engine).
pub struct D81VectorMemory<'a> {
    pub copy: &'a mut dyn FnMut(u64, &mut [u8]) -> bool,
    pub admit: &'a dyn Fn(u64) -> bool,
}

/// Run the whole scattered pipeline. See the module header for the order and
/// why each closing side sits where it does.
///
/// Returns the capture; refusal never happens HERE — an inadmissible pointer
/// stops its branch and is counted, an unreadable window records a void, and
/// the WALK renders every frame-level verdict from what was and was not
/// observed, exactly as it did over the recipe view.
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
        header_close: FamilyStore::new(FamilyImages { opening: 2, closing: 0 }),
        record_ptr_close: FamilyStore::new(FamilyImages { opening: 2, closing: 0 }),
        copies: Cell::new(0),
        pointers_rejected: 0,
        voids: 0,
        aliases_retired: 0,
        closing_tail_start: 0,
    };

    // ---- stage 1: rail slots out of the BULK table image -------------------
    //
    // Every non-null admitted slot is followed, exactly as the recipe's
    // element branch was — including provinces the topology walk will skip,
    // whose windows are scoped: unreadable ones cost a void and nothing else.
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

    // ---- stage 2: rail headers, opening pair -------------------------------
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

    // ---- stage 3: pair-blob descriptors from the header OPENING image ------
    //
    // Decoded from the image the walk will decode from; a header whose
    // opening copy failed contributes no blob, and the walk refuses that
    // province at the header, never at a blob it silently lacks.
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
                // Zero is a legitimate empty blob; over-max is the walk's own
                // HeaderInvalid refusal, rendered by the walk from this same
                // image. Neither takes a copy.
                continue;
            }
            if !(memory.admit)(ptr) {
                // The walk renders InvalidPointer/HeaderInvalid from the same
                // image; the pipeline just declines to copy through it.
                capture.pointers_rejected = capture.pointers_rejected.saturating_add(1);
                continue;
            }
            blobs.push((ptr, count.saturating_mul(offsets.pair_stride)));
        }
    }

    // ---- stage 4: rail-header CLOSING pair ---------------------------------
    //
    // After the blob descriptors are decoded, BEFORE the blobs are copied —
    // the executor's `AfterWave(3)` instant, reproduced (arch consult,
    // section 4 risk 1).
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

    // ---- stage 5: pair blobs, opening pair ---------------------------------
    {
        let store = &mut capture.opening[D81Family::PairBlob as usize];
        for &(ptr, len) in &blobs {
            store.push(ptr, len);
        }
        store.seal();
        store.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    // ---- stage 6: map provinces out of the BULK map array ------------------
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

    // ---- stage 7: record pointers + anchors, opening pairs -----------------
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
            anchors.push(map_province + offsets.map_anchors_off, offsets.map_anchors_len);
        }
        anchors.seal();
        anchors.copy_side(2, ST_O, memory.copy, &capture.copies);
    }

    // ---- stage 8: record keys behind the record pointers -------------------
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
                // The record may legitimately be a MODULE static — the walk
                // admits those through its own owner predicate and reads only
                // the key window. The pipeline cannot apply the walk's
                // predicate (it needs the module base and is the walk's own
                // judgement), so it copies the key window for any non-null
                // pointer the HEAP window admits and lets the walk refuse the
                // rest from the same image it always refused them from.
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

    // ---- stage 9: record-pointer CLOSING pair ------------------------------
    //
    // After the key windows: the check's purpose is that the key the walk
    // consumed came from the record the closing observation vouches for.
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

    // ---- stage 10: CProvince windows for railed ids AND selector keys ------
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
                // Counted exactly as the recipe's admitted SOURCE_SLOT_PTR
                // branch counted it (Codex review round 1, finding 2): the
                // walk still refuses the province from the same slot value,
                // and the wire still says a pointer was declined.
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
                        capture.pointers_rejected =
                            capture.pointers_rejected.saturating_add(1);
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

    // ---- stage 11: the CLOSING slots and active entries, LAST --------------
    //
    // Taken after every family that was reached through them, so each pair
    // `(opening image from the bulk array, closing window here)` brackets
    // the whole use of the pointer it carries. Taking these before the
    // families would let an object replaced mid-plan publish with every
    // closure check passing — the regression this order exists to refuse.
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

    // ---- ALIAS RETIREMENT (Codex review round 1, finding 1) ----------------
    //
    // A scattered window intersecting a bulk-owned span is a malformed
    // engine pointer's alias. It dies UNSERVED: the slot's opening images
    // stay the bulk view's, its closing images stay the closing-only
    // families', and the reader that derived the aliased window refuses
    // `NotCaptured` at its own site. The closing-only families are exempt —
    // their windows sit inside the bulk tables BY DESIGN. Cross-scattered
    // aliases (one scattered object inside another) are left to the walk's
    // own comparisons: whichever family serves, the images are genuine
    // observations of that address and a divergence refuses — the
    // publish-evading class is exactly the bulk overlap this retires.
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
    capture.aliases_retired = capture
        .aliases_retired
        .saturating_add(capture.record_ptr_close.retire_intersecting(&roots.bulk_spans));

    // Windows whose OPENING observation failed are scoped voids — the same
    // meaning the recipe's counter had: the plan asked for an opening copy
    // and the engine refused it. Failed CLOSING observations are NOT voids
    // (the recipe's analog was an unreadable close, which reached the wire
    // through the walk's refusal and never through `scoped_voids`), and dead
    // aliases are not voids either (Codex review round 1, finding 3).
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

// kills: any reordering of the pipeline stages above that takes a closing
// side before the reads it vouches for — the staged-world gates in
// `attach_log/tests/d81_authoritative.rs` compare the walk's published rows
// and refusals against the live walk's over worlds staged to change between
// stages, and the transaction-accounting test pins the per-family window
// and image counts.
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

    /// kills: `FamilyStore::seal` losing the sort, the dedup, or the offset
    /// prefix arithmetic — every later lookup depends on all three.
    #[test]
    fn seal_sorts_dedups_and_sizes() {
        let mut store = FamilyStore::new(FamilyImages { opening: 2, closing: 0 });
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

    /// kills: swapping the interleave back to chunked passes — the copy
    /// order here asserts A then B per window before the next window's A.
    #[test]
    fn copy_side_interleaves_per_window() {
        let mut store = FamilyStore::new(FamilyImages { opening: 2, closing: 0 });
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

    /// kills: attempting the adjacent sample after a failed first
    /// observation, or serving any image of such a window.
    #[test]
    fn failed_first_observation_ends_the_window_and_serves_nothing() {
        let mut store = FamilyStore::new(FamilyImages { opening: 2, closing: 0 });
        store.push(0x1000, 4);
        store.seal();
        let calls = Cell::new(0);
        let mut copy = |_addr: u64, _out: &mut [u8]| false;
        store.copy_side(2, ST_O, &mut copy, &calls);
        // One call: the failed A. B was never attempted.
        assert_eq!(calls.get(), 1);
        assert_eq!(store.status[0], 0);
    }

    /// kills: serving a window's later image when only its first succeeded
    /// under any class but the sample-absent one.
    #[test]
    fn failed_sample_is_absent_not_unreadable() {
        let mut store = FamilyStore::new(FamilyImages { opening: 2, closing: 0 });
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
            header_close: FamilyStore::new(FamilyImages { opening: 2, closing: 0 }),
            record_ptr_close: FamilyStore::new(FamilyImages { opening: 2, closing: 0 }),
            copies: Cell::new(0),
            pointers_rejected: 0,
            voids: 0,
            aliases_retired: 0,
            closing_tail_start: 0,
        }
    }

    /// kills: answering a closing image out of an opening-only family, or an
    /// opening image out of a closing-only family, instead of the honest
    /// `NeverObserved` / fall-through classes.
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

        // A closing-only family: opening reads FALL THROUGH (the bulk view
        // owns those bytes' opening images), closing reads serve.
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

    /// kills: serving any byte from a span two families' live windows both
    /// contain — the single-family rule is the whole of the round-2 alias
    /// fix, and this stages the overlap directly.
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
        // The record window sits inside the header window: every image of
        // the overlapped span refuses, in both directions.
        assert_eq!(
            capture.read(0x1004, 8, VectorImage::Opening),
            VectorAnswer::Aliased
        );
        assert_eq!(
            capture.read(0x1004, 8, VectorImage::Closing),
            VectorAnswer::Aliased
        );
        // The header's un-overlapped prefix is still one family's... and is
        // still refused ONLY if the span itself is contained by both; a
        // 4-byte read at the header's start is the header's alone and
        // serves.
        assert!(matches!(
            capture.read(0x1000, 4, VectorImage::Opening),
            VectorAnswer::Served(_)
        ));
    }

    /// kills: a retired window's span falling through to `NotCovered` (and
    /// so to the bulk view) instead of poisoning every read that touches it.
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
        // A closing-only family covering the same span stays exempt from
        // retirement — and must STILL not serve, because the dead record
        // window poisons the span.
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

    /// kills: the pipeline taking the header closing pair after the blob
    /// copies (or before the blob decode) — the copy log's order IS the
    /// soundness claim, and this asserts it end to end at a small shape.
    #[test]
    fn pipeline_order_headers_close_between_blob_decode_and_blob_copy() {
        // A tiny staged world: rail table with two slots, two rail objects
        // whose headers point at two blobs, a map array, a source array.
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

        // The header windows appear twice as an opening pair, then twice as
        // a closing pair, and only THEN the blobs — with each window's two
        // observations adjacent — and the active closing triples LAST (the
        // map and source slot families are empty at this world's zero
        // counts).
        let header_a = rail_a + 0x38;
        let header_b = rail_b + 0x38;
        let active_a = roots.active_table + 0x12;
        let active_b = roots.active_table + 0x20 + 0x12;
        assert_eq!(
            log,
            vec![
                header_a, header_a, header_b, header_b, // opening pairs
                header_a, header_a, header_b, header_b, // closing pairs
                blob_a, blob_a, blob_b, blob_b, // blob opening pairs
                active_a, active_a, active_a, // closing coherence triples
                active_b, active_b, active_b,
            ]
        );
        assert_eq!(capture.copies.get(), 18);
        assert_eq!(capture.pointers_rejected, 0);
        assert_eq!(capture.voids, 0);
        // And the closing images serve as closing images.
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
