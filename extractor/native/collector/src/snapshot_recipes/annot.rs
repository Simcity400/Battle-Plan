//! Per-interval annotations: what a consumer CONSUMES, what was sampled twice,
//! and which stage of a reader's walk produced it.
//!
//! # Why these are a side table and not fields on the record
//!
//! [`super::dedup::IntervalRecord`] is 32 bytes, two per cache line, and the
//! sweep walks the index end to end once per wave. Every field added to it is
//! paid by every interval of every observation, including the overwhelming
//! majority that declare no mask, take one sample and carry no stage. The three
//! things this module holds are the opposite shape: they exist only for the ops
//! that ask for them, and an observation whose recipes ask for none of them
//! allocates nothing here at all.
//!
//! The cost of the side table is a lookup where a field would have been a load.
//! It is paid once per (request, interval) in the dependency attach and once
//! per witnessed interval at closure — never inside a copy — and only when the
//! observation has any annotation at all ([`Annotations::armed`]).
//!
//! # Consumption, and why the default is "all of it"
//!
//! Ruling A (2026-08-09) scopes an over-read's failure to the rows whose
//! consumed bytes intersect it, and ruling B compares only consumed bytes at
//! closure so engine padding cannot manufacture a torn gap. Both need the same
//! fact — which bytes of a copied object a reader actually looks at — so both
//! read it from here.
//!
//! The default is [`Consumption::full`]: an interval nobody has said anything
//! about is treated as consumed end to end, which is exactly the behaviour the
//! substrate had before this module existed. Narrowing is opt-in and it is
//! MONOTONE IN THE SAFE DIRECTION: one unmasked contributor makes the whole
//! interval consumed again and no later mask can un-say it. A mask that
//! narrowed an interval another recipe reads whole would hide a real torn read.

use std::collections::BTreeMap;

use super::arena::{vec_bytes, CustodyLedger};
use super::{CustodyRefusal, IntervalId, Span};

/// The consumed windows of a DECODED EXTENT, stated once for the whole array.
///
/// A mask on a container's elements is per-element by nature — "the first four
/// bytes of every 0x20-byte active-table entry" — and materialising it as one
/// span per element would be 16,384 spans for one of d81's tables, at the
/// moment custody is fullest. So it is stored as the arithmetic it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StridedMask {
    pub base: u64,
    pub stride: u32,
    pub count: u32,
    /// `(offset within the element, length)`.
    pub windows: Vec<(u32, u32)>,
}

impl StridedMask {
    fn for_each_window(&self, mut visit: impl FnMut(Span) -> bool) -> bool {
        for index in 0..u64::from(self.count) {
            let element = self.base + index * u64::from(self.stride);
            for &(offset, len) in &self.windows {
                if !visit(Span::new(element + u64::from(offset), u64::from(len))) {
                    return false;
                }
            }
        }
        true
    }
}

/// Which bytes of one interval are consumed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consumption {
    /// Every byte. Set by any contributor that declared no mask, and never
    /// unset.
    pub full: bool,
    /// Absolute spans, when `full` is false. Unsorted and possibly
    /// overlapping: every consumer of this type is a membership test or a
    /// comparison, never an ordered walk.
    pub spans: Vec<Span>,
    /// Per-element windows of decoded extents covering this interval.
    pub strided: Vec<StridedMask>,
}

impl Default for Consumption {
    fn default() -> Self {
        Self {
            full: true,
            spans: Vec::new(),
            strided: Vec::new(),
        }
    }
}

impl Consumption {
    /// True when any byte of `want` is consumed.
    pub fn touches(&self, want: &Span) -> bool {
        if self.full {
            return true;
        }
        if self.spans.iter().any(|span| span.overlaps(want)) {
            return true;
        }
        for mask in &self.strided {
            let mut hit = false;
            mask.for_each_window(|span| {
                if span.overlaps(want) {
                    hit = true;
                    return false;
                }
                true
            });
            if hit {
                return true;
            }
        }
        false
    }

    /// Compare the consumed bytes of `interval` between two images.
    ///
    /// `opening`/`closing` are the interval's two images, both `interval.len`
    /// long. A window that falls outside the interval is clipped, never
    /// skipped-and-certified: clipping compares the part that is there, and the
    /// part that is not was never in this interval to change.
    pub fn images_agree(&self, interval: Span, opening: &[u8], closing: &[u8]) -> bool {
        if self.full {
            return opening == closing;
        }
        if self.spans.is_empty() && self.strided.is_empty() {
            // A narrowed consumption with nothing in it compares nothing, and
            // "nothing compared" must never read as "agreed": that is a witness
            // certifying an object it did not look at. The executor does not
            // build this state — an empty mask falls back to `consume_all` — so
            // this is the second line of defence and it fails CLOSED.
            return false;
        }
        let compare = |window: Span| -> bool {
            let start = window.start.max(interval.start);
            let end = window.end_excl().min(interval.end_excl());
            if u128::from(start) >= end {
                return true;
            }
            let from = (start - interval.start) as usize;
            let to = from + (end - u128::from(start)) as usize;
            opening[from..to] == closing[from..to]
        };
        for span in &self.spans {
            if !compare(*span) {
                return false;
            }
        }
        for mask in &self.strided {
            if !mask.for_each_window(compare) {
                return false;
            }
        }
        true
    }
}

/// One consumer's standing request for certification of an interval.
///
/// The REQUEST is per consumer and the IMAGES are shared, and conflating the
/// two is a real defect in both directions: a recipe that asked for three
/// images and got two must be gapped, while a recipe that asked for two and got
/// them must not be — even though they are looking at the same interval. The
/// same goes for the policy: one recipe electing to resolve a disagreement
/// itself cannot be overruled by a neighbour that would rather be gapped, and a
/// neighbour that wants to be gapped cannot be silently downgraded either.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleAsk {
    pub consumer: super::ConsumerId,
    /// Images this consumer asked for, the opening one included.
    pub wanted: u8,
    /// Whether a DISAGREEMENT gaps this consumer or is merely recorded for it.
    pub gap_on_disagree: bool,
}

/// The back-to-back images of one interval, and who asked for them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleImages {
    /// Arena offsets of images 2..=n, in the order they were taken. The first
    /// image is the record's own opening offset.
    pub extra: Vec<u32>,
    /// Bit `k` set when image `k` (0-based) was taken.
    pub success_mask: u8,
    /// Every consumer's request. A consumer appears at most once.
    pub asks: Vec<SampleAsk>,
}

impl SampleImages {
    /// Images actually held, the opening one included.
    pub fn held(&self) -> usize {
        1 + self.extra.len()
    }

    /// Record one request. Identical requests collapse; DIFFERENT ones do not.
    ///
    /// # Why the count and the policy are not merged
    ///
    /// They belong to the same request and merging them across requests invents
    /// one nobody made. Take one consumer reached by two recipes: recipe A asks
    /// for two images and to be gapped on a disagreement, and gets its two;
    /// recipe B asks for three and says it will resolve a disagreement itself.
    /// Merging by `max` and OR stores "three images, gap me" — a request with
    /// A's policy and B's count — and the consumer is then gapped for a third
    /// image that only the non-gapping request ever wanted.
    ///
    /// Each ask therefore stands on its own and is judged on its own.
    pub fn ask(&mut self, consumer: super::ConsumerId, wanted: u8, gap_on_disagree: bool) {
        let request = SampleAsk {
            consumer,
            wanted,
            gap_on_disagree,
        };
        if !self.asks.contains(&request) {
            self.asks.push(request);
        }
    }

    /// The consumers at least one of whose own gapping requests went unmet.
    pub fn unmet(&self) -> impl Iterator<Item = super::ConsumerId> + '_ {
        let held = self.held();
        self.asks
            .iter()
            .filter(move |ask| ask.gap_on_disagree && held < ask.wanted as usize)
            .map(|ask| ask.consumer)
    }

    /// True when `consumer` elected to resolve a disagreement itself.
    ///
    /// It asks whether EVERY one of that consumer's requests said `Record`: a
    /// consumer reached by two recipes, one of which wants to be gapped, is not
    /// opting out. A consumer that never asked has not opted out either — it
    /// never expressed a preference, and a torn image is bad for it too.
    pub fn opted_out_of_gapping(&self, consumer: super::ConsumerId) -> bool {
        let mut asked = false;
        for ask in self.asks.iter().filter(|ask| ask.consumer == consumer) {
            asked = true;
            if ask.gap_on_disagree {
                return false;
            }
        }
        asked
    }

    /// True when at least one consumer wants a disagreement to gap it.
    pub fn anyone_gaps(&self) -> bool {
        self.asks.iter().any(|ask| ask.gap_on_disagree)
    }
}

/// The SECOND and later closing images of one interval.
///
/// # Why the closing wave takes more than one
///
/// A reader that renders its own torn verdict compares two adjacent observations
/// of a field. It does that at the OPENING — which is what [`SampleImages`] is
/// for — and it does it again at every CLOSING re-read, because a closing read is
/// an observation like any other and the live reader it replaces took two of
/// them. With one closing image a converted reader's closing A/B compares that
/// image with itself and can never tear: it publishes where the live reader
/// refused `Torn`, and it publishes where the live reader's second closing read
/// went unreadable.
///
/// So the closing wave takes as many images as the interval's own
/// [`super::executor::SampleSpec`] asked for. An interval nobody asked about
/// takes one, which is the closing wave every recipe had before this existed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosingImages {
    /// Arena offsets of images 2..=n, in the order taken. Image 1 is the
    /// record's own `closing_offset`.
    pub extra: Vec<u32>,
}

impl ClosingImages {
    /// Images held, the first one included.
    pub fn held(&self) -> usize {
        1 + self.extra.len()
    }
}

/// One consumer's policy for the CLOSING witness of one interval.
///
/// The shape mirrors [`SampleAsk`] because the question is the same one: a
/// policy belongs to a REQUEST, not to an interval, and two recipes reaching one
/// interval may want different things. Merging them would invent a request
/// nobody made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseAsk {
    pub consumer: super::ConsumerId,
    /// Whether a CHANGED closing image gaps this consumer, or is left for it to
    /// judge from the image itself.
    pub gap_on_change: bool,
}

/// Everything the annotations hold about ONE interval.
///
/// # Why one entry and not five maps
///
/// This was five `BTreeMap<IntervalId, _>` — `consumed`, `samples`, `closing`,
/// `closes` and `stages` — and the attach pass wrote three of them and read a
/// fourth for EVERY request against EVERY interval it covered. Lane GL measured
/// that on d81's production shape: 35,084 requests per observation, five
/// ordered-map descents apiece, about a quarter of the whole observation spent
/// finding entries rather than filling them.
///
/// The five maps were always keyed identically and always written together, so
/// they are one map now and one descent serves the whole request. Nothing about
/// what is stored changed: each of the five is still independently ABSENT (an
/// interval with a stage and no consumption records no consumption, which is
/// the difference between "reads all of it" and "reads a mask of it"), which is
/// why three of these fields are `Option` rather than a default value.
#[derive(Clone, Debug, Default)]
struct IntervalAnnot {
    consumed: Option<Consumption>,
    samples: Option<SampleImages>,
    /// Arena offsets of the SECOND and later closing images, in the order the
    /// closing wave took them. The FIRST closing image is the record's own
    /// `closing_offset`, exactly as the first opening image is its
    /// `opening_offset` — see [`ClosingImages`].
    closing: Option<ClosingImages>,
    closes: Vec<CloseAsk>,
    stages: Vec<u16>,
}

/// Everything the annotations hold for one observation.
///
/// # Why an index and not an ordered map
///
/// Lane GL turned five `BTreeMap<IntervalId, _>` into one; lane HL (task #185)
/// removed the descent itself. `IntervalId` is a DENSE `u32` handed out in
/// order by the ownership map — `dedup.rs` mints it as `records.len()` — so the
/// key already indexes an array and an ordered map was searching for a number
/// it had been given. At d81's production shape that is a descent through
/// 21,056 entries, taken twice per request across 35,084 requests, to reach a
/// slot whose position was known.
///
/// [`index`] maps interval id to `slot + 1`, with `0` for "never annotated", and
/// [`slots`] holds only the intervals that were. That pair, rather than one
/// `Vec<IntervalAnnot>` indexed directly, because the direct spelling would
/// materialise an entry for every id BELOW the highest annotated one: on d81
/// that is nearly free, and on a recipe with a million intervals and ten
/// annotations it would charge the custody ledger for a million
/// `IntervalAnnot`s. The index costs four bytes per id and the slots cost what
/// they hold.
///
/// ASCENDING ITERATION IS PRESERVED BY CONSTRUCTION, not by a sort:
/// [`Self::sampled_intervals`] walks `index` in id order, which is the order
/// the map produced and the order `close_witnesses` depends on.
///
/// [`index`]: Self::index
/// [`slots`]: Self::slots
#[derive(Clone, Debug, Default)]
pub struct Annotations {
    /// Interval id -> `slots` position PLUS ONE; `0` means no entry exists.
    /// Plus one rather than a sentinel `u32::MAX` so that the vector's own
    /// zero-initialised growth is already the absent state.
    index: Vec<u32>,
    /// Interval ids too far past the dense index to grow it to — see
    /// [`Self::place`], which bounds the dense vector at `slots + slack` so
    /// that a caller passing a distant id cannot allocate the gap. Empty on
    /// every path production takes, because the ownership map mints ids
    /// densely from zero.
    sparse: BTreeMap<IntervalId, u32>,
    /// The annotated intervals, in first-annotation order.
    slots: Vec<IntervalAnnot>,
    /// True once any recipe in the observation declares any annotation. While
    /// false every entry point below is a single predictable branch.
    armed: bool,
    /// Modelled footprint, reconciled into the custody ledger by the executor.
    charged: u64,
}

/// One interval's annotations, held open across a request's whole set of
/// writes.
///
/// THE POINT IS THE DESCENT, not the borrow: the attach pass tags a stage,
/// records a closing policy, records a consumption mask and records a sampling
/// request against the same interval, one after the other, and each of those
/// used to find the interval again. Holding the entry is what makes the four
/// writes one lookup. Every method here does exactly what the identically named
/// method on [`Annotations`] does.
pub struct IntervalEntry<'a> {
    slot: &'a mut IntervalAnnot,
}

impl IntervalEntry<'_> {
    /// See [`Annotations::tag_stage`].
    pub fn tag_stage(&mut self, stage: u16) {
        if stage == 0 {
            return;
        }
        if !self.slot.stages.contains(&stage) {
            self.slot.stages.push(stage);
        }
    }

    /// See [`Annotations::ask_close`]. The armed check is the caller's, exactly
    /// as it is on the map-level spelling.
    pub fn ask_close(&mut self, consumer: super::ConsumerId, gap_on_change: bool) {
        let request = CloseAsk {
            consumer,
            gap_on_change,
        };
        if !self.slot.closes.contains(&request) {
            self.slot.closes.push(request);
        }
    }

    /// See [`Annotations::consume_all`].
    pub fn consume_all(&mut self) {
        let entry = self.slot.consumed.get_or_insert_with(Consumption::default);
        entry.full = true;
        entry.spans.clear();
        entry.strided.clear();
    }

    /// See [`Annotations::consume_spans`].
    pub fn consume_spans(&mut self, spans: &[Span]) {
        let entry = self.slot.consumed.get_or_insert_with(|| Consumption {
            full: false,
            spans: Vec::new(),
            strided: Vec::new(),
        });
        if entry.full {
            return;
        }
        entry.spans.extend_from_slice(spans);
    }

    /// See [`Annotations::consume_strided`].
    pub fn consume_strided(&mut self, mask: StridedMask) {
        let entry = self.slot.consumed.get_or_insert_with(|| Consumption {
            full: false,
            spans: Vec::new(),
            strided: Vec::new(),
        });
        if entry.full {
            return;
        }
        entry.strided.push(mask);
    }

    /// Record one consumer's SAMPLING request against this interval.
    ///
    /// IN PLACE. The map-keyed path cloned the whole [`SampleImages`] — two
    /// `Vec`s — out of the map, appended one ask to the clone, and inserted the
    /// clone back, which is two descents and two allocations to append one
    /// element. The ask that lands is the same ask.
    ///
    /// A missing record means no ADJACENT sample was ever taken, and the
    /// opening image still exists, so the default's `success_mask` is 1 — which
    /// is exactly what the cloning path substituted for an absent entry.
    pub fn ask_samples(&mut self, consumer: super::ConsumerId, wanted: u8, gap_on_disagree: bool) {
        let images = self.slot.samples.get_or_insert_with(|| SampleImages {
            extra: Vec::new(),
            success_mask: 1,
            asks: Vec::new(),
        });
        images.ask(consumer, wanted, gap_on_disagree);
    }
}

impl Annotations {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turn the machinery on. Called once per observation, from the executor,
    /// when any recipe declares a mask, a stage, a sample count or a closing
    /// program.
    pub fn arm(&mut self) {
        self.armed = true;
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// One interval's annotations, opened once for a whole request's worth of
    /// writes. See [`IntervalEntry`].
    ///
    /// Creates the entry if this is the first annotation against `interval`,
    /// exactly as the map's `entry(..).or_default()` did.
    pub fn entry(&mut self, interval: IntervalId) -> IntervalEntry<'_> {
        IntervalEntry {
            slot: self.slot_mut(interval),
        }
    }

    /// The slot for `interval`, created empty if it does not exist yet.
    fn slot_mut(&mut self, interval: IntervalId) -> &mut IntervalAnnot {
        let at = self.place(interval);
        &mut self.slots[at - 1]
    }

    /// This interval's slot position PLUS ONE, creating the slot if needed.
    ///
    /// # The dense index is grown only where growing it is bounded
    ///
    /// `IntervalId` is `pub` and `entry` is `pub`, so "the ownership map mints
    /// ids densely from zero" is a fact about today's callers and not a
    /// property of this type. A dense index sized to `id + 1` would turn
    /// `entry(IntervalId(u32::MAX))` into a 16 GiB allocation taken BEFORE the
    /// custody ledger ever sees it — an unbudgeted allocation where the
    /// `BTreeMap` this replaced would have taken one node (Codex round 2,
    /// finding 3).
    ///
    /// So the index grows only to ids within [`DENSE_INDEX_SLACK`] of the
    /// entries that already exist, which makes the dense term at most
    /// `slots + SLACK` however the ids arrive, and anything beyond it goes into
    /// an ordered map that costs one entry per id exactly as the old table did.
    /// The dense path is what every production caller takes; the sparse path
    /// exists so that the ones that do not cannot allocate a gap.
    ///
    /// BOTH PATHS STORE THE SAME THING — position plus one — and every reader
    /// goes through [`Self::slot`], which consults both.
    ///
    /// # THE INVARIANT, because it is what keeps ascending iteration free
    ///
    /// `index.len() <= every key in sparse`. The dense vector is never grown
    /// past an id the sparse map already holds, so "dense ids, then sparse ids"
    /// IS ascending id order and [`Self::sampled_intervals`] can simply chain
    /// the two rather than merge them.
    fn place(&mut self, interval: IntervalId) -> usize {
        /// How far past the last dense slot an id may sit and still be indexed
        /// densely. The dense vector is therefore bounded by
        /// `slots.len() + this`, whatever order the ids arrive in.
        const DENSE_INDEX_SLACK: usize = 1024;

        let id = interval.0 as usize;
        if let Some(&at) = self.index.get(id) {
            if at != 0 {
                return at as usize;
            }
        }
        // Only ever non-empty on the sparse path, so the common case pays one
        // branch on a length rather than a descent.
        if !self.sparse.is_empty() {
            if let Some(&at) = self.sparse.get(&interval) {
                return at as usize;
            }
        }
        if id >= self.index.len()
            && id <= self.slots.len().saturating_add(DENSE_INDEX_SLACK)
            // The invariant: never grow past an id the sparse map holds.
            && self.sparse.keys().next().is_none_or(|low| id < low.0 as usize)
        {
            // GROW TO THE ID AND NO FURTHER. `resize` zero-fills, and zero is
            // the absent state, so there is no separate initialisation pass.
            self.index.resize(id + 1, 0);
        }
        self.slots.push(IntervalAnnot::default());
        // `slots.len()` is the position PLUS ONE by construction, which is
        // exactly what both tables store.
        let at = self.slots.len() as u32;
        if id < self.index.len() {
            self.index[id] = at;
        } else {
            self.sparse.insert(interval, at);
        }
        at as usize
    }

    /// One interval's annotations, or `None` when nothing has been recorded
    /// against it.
    fn slot(&self, interval: IntervalId) -> Option<&IntervalAnnot> {
        let at = match self.index.get(interval.0 as usize).copied() {
            Some(at) if at != 0 => at,
            // Absent from the dense index, or present as the zero that means
            // "no entry" — either way the sparse tail is the other place it
            // could be. See `place`.
            _ => *self.sparse.get(&interval)?,
        };
        self.slots.get(at as usize - 1)
    }

    /// How many CLOSING images this interval's consumers asked for.
    ///
    /// The same number their [`SampleSpec`] asked for at the opening, because it
    /// is the same question: a reader that compares two adjacent observations of
    /// a field needs two of them at every instant it compares, not only at the
    /// first. One when nobody asked, which is the closing wave every recipe had
    /// before this existed.
    ///
    /// [`SampleSpec`]: super::executor::SampleSpec
    pub fn closing_images_wanted(&self, interval: IntervalId) -> u8 {
        self.slot(interval)
            .and_then(|slot| slot.samples.as_ref())
            .map(|images| images.asks.iter().map(|ask| ask.wanted).max().unwrap_or(1))
            .unwrap_or(1)
            .max(1)
    }

    /// Record one extra closing image's arena offset.
    pub fn push_closing_image(&mut self, interval: IntervalId, offset: u32) {
        self.slot_mut(interval)
            .closing
            .get_or_insert_with(ClosingImages::default)
            .extra
            .push(offset);
    }

    /// The extra closing images of `interval`, the first one excluded.
    pub fn closing_images(&self, interval: IntervalId) -> Option<&ClosingImages> {
        self.slot(interval).and_then(|slot| slot.closing.as_ref())
    }

    /// Record one consumer's closing-witness policy for one interval.
    ///
    /// Identical requests collapse; different ones stand side by side, and the
    /// strictest wins at closure. The table is work-scaled and is charged
    /// through [`Self::capacity_bytes`] like every other one here.
    ///
    /// EVERY REQUEST IS STORED, including a `Gap` one on an interval nobody has
    /// asked about (Codex 019feb7d, finding 5). An earlier version skipped those
    /// on the grounds that `Gap` is the answer [`Self::close_gaps`] already
    /// gives — which is true of an interval with NO asks and false the moment
    /// one arrives: a consumer whose `Gap` request reached a coalesced interval
    /// first, and whose `Record` request reached it second, would have had only
    /// the `Record` stored and would then have opted out of a gap it asked for.
    /// The strictest request must win whatever order the requests arrive in, and
    /// that is only expressible if every one of them is recorded.
    pub fn ask_close(
        &mut self,
        interval: IntervalId,
        consumer: super::ConsumerId,
        gap_on_change: bool,
    ) {
        if !self.armed {
            return;
        }
        self.entry(interval).ask_close(consumer, gap_on_change);
    }

    /// Whether a CHANGED closing image of `interval` gaps `consumer`.
    ///
    /// TRUE WHEN NOBODY ASKED, which is the substrate's own default and the safe
    /// direction: a consumer that never expressed a preference is one that takes
    /// the substrate's verdict. False only when EVERY one of this consumer's
    /// requests elected to judge the image itself — a consumer reached by two
    /// recipes, one of which wants the default, is not opting out.
    pub fn close_gaps(&self, interval: IntervalId, consumer: super::ConsumerId) -> bool {
        let Some(asks) = self.slot(interval).map(|slot| &slot.closes) else {
            return true;
        };
        let mut asked = false;
        for ask in asks.iter().filter(|ask| ask.consumer == consumer) {
            asked = true;
            if ask.gap_on_change {
                return true;
            }
        }
        !asked
    }

    /// Record that every byte of `interval` a contributor touched is consumed.
    /// Once set this cannot be narrowed again.
    pub fn consume_all(&mut self, interval: IntervalId) {
        self.entry(interval).consume_all();
    }

    /// Record `spans` of `interval` as consumed. Ignored once the interval is
    /// fully consumed.
    pub fn consume_spans(&mut self, interval: IntervalId, spans: &[Span]) {
        self.entry(interval).consume_spans(spans);
    }

    /// Record a per-element mask over `interval`.
    pub fn consume_strided(&mut self, interval: IntervalId, mask: StridedMask) {
        self.entry(interval).consume_strided(mask);
    }

    /// The consumption of `interval`. Absent is [`Consumption::full`].
    pub fn consumption(&self, interval: IntervalId) -> Option<&Consumption> {
        self.slot(interval).and_then(|slot| slot.consumed.as_ref())
    }

    /// True when `want` intersects bytes some consumer reads out of
    /// `interval`.
    pub fn consumed_touches(&self, interval: IntervalId, want: &Span) -> bool {
        match self.consumption(interval) {
            Some(consumption) => consumption.touches(want),
            None => true,
        }
    }

    pub fn set_samples(&mut self, interval: IntervalId, images: SampleImages) {
        self.slot_mut(interval).samples = Some(images);
    }

    pub fn samples(&self, interval: IntervalId) -> Option<&SampleImages> {
        self.slot(interval).and_then(|slot| slot.samples.as_ref())
    }

    /// ASCENDING BY INTERVAL ID, which is what the map-keyed spelling produced
    /// and what `close_witnesses` walks in.
    ///
    /// The index is walked, not the slots: the slots are in first-annotation
    /// order and the index is in id order, so this is where the guarantee in
    /// the sentence above actually comes from. The id is yielded BY VALUE
    /// because there is no longer a stored key to borrow — `IntervalId` is a
    /// `Copy` newtype over a `u32`, so nothing is copied that was not already.
    pub fn sampled_intervals(&self) -> impl Iterator<Item = (IntervalId, &SampleImages)> {
        let dense = self.index.iter().enumerate().filter_map(move |(id, at)| {
            let slot = self.slots.get((*at as usize).checked_sub(1)?)?;
            slot.samples
                .as_ref()
                .map(|images| (IntervalId(id as u32), images))
        });
        // CHAINED, NOT MERGED, and `place`'s invariant is the whole licence for
        // that: every sparse key is at least `index.len()`, so every dense id
        // precedes every sparse one and both halves are already ascending.
        let sparse = self.sparse.iter().filter_map(move |(id, at)| {
            let slot = self.slots.get((*at as usize).checked_sub(1)?)?;
            slot.samples.as_ref().map(|images| (*id, images))
        });
        dense.chain(sparse)
    }

    /// Tag `interval` with the stage of an op that reached it. Stages are a set:
    /// an interval a deduplicated copy serves for two stages carries both, and
    /// that is exactly what closing elision keys on.
    pub fn tag_stage(&mut self, interval: IntervalId, stage: u16) {
        if stage == 0 {
            return;
        }
        self.entry(interval).tag_stage(stage);
    }

    pub fn stages_of(&self, interval: IntervalId) -> &[u16] {
        self.slot(interval)
            .map(|slot| slot.stages.as_slice())
            .unwrap_or(&[])
    }

    /// Modelled footprint.
    ///
    /// # It models the structure that exists, and that structure changed
    ///
    /// This charged `BTreeMap` NODES — fixed arrays allocated whole whatever
    /// their fill — because that is what the table was. It is two vectors now
    /// (lane HL), and a `Vec`'s allocation, unlike a node's, IS observable:
    /// `capacity()` is the number the allocator was asked for. So the model no
    /// longer estimates anything about the container itself. The one estimate
    /// that survives is `Vec::capacity` on the inline `Vec`s each slot owns,
    /// which is likewise exact.
    ///
    /// CAPACITY AND NOT LENGTH, in both vectors. Both grow by doubling, so a
    /// table holding 21,056 slots may hold 32,768 slots' worth of allocation,
    /// and charging the length would under-report by a third at the worst
    /// moment. An under-model is custody the ceiling never sees — the one
    /// direction this accounting must not fail in.
    ///
    /// THE INDEX IS CHARGED TOO, and it is the term that did not exist before:
    /// four bytes for every interval id up to the highest annotated one,
    /// whether or not that id was annotated. On a dense recipe it is noise
    /// against the slots; on a sparse one it is the whole charge, which is
    /// correct — it is the whole allocation.
    ///
    /// It over-charges, deliberately, wherever it estimates at all.
    /// Over-charging costs an observation headroom it did not need;
    /// under-charging costs the ceiling its meaning.
    pub fn capacity_bytes(&self) -> u64 {
        let mut heap = 0u64;
        for slot in self.slots.iter() {
            if let Some(consumption) = &slot.consumed {
                heap += vec_bytes(&consumption.spans) + vec_bytes(&consumption.strided);
                for mask in &consumption.strided {
                    heap += vec_bytes(&mask.windows);
                }
            }
            if let Some(images) = &slot.samples {
                heap += vec_bytes(&images.extra) + vec_bytes(&images.asks);
            }
            if let Some(images) = &slot.closing {
                heap += vec_bytes(&images.extra);
            }
            heap += vec_bytes(&slot.closes);
            heap += vec_bytes(&slot.stages);
        }

        // The same model `vec_bytes` applies to every inline `Vec` above,
        // spelled out here because these two are the CONTAINER and a reader
        // checking the ledger against the allocator should see the two terms
        // named: `capacity`, not `len`, times the element size.
        let index = self.index.capacity() as u64 * std::mem::size_of::<u32>() as u64;
        let slots = self.slots.capacity() as u64 * std::mem::size_of::<IntervalAnnot>() as u64;
        // THE SPARSE TAIL IS STILL A `BTreeMap` AND IS STILL CHARGED AS NODES,
        // for the reason the whole table used to be: a node is allocated whole
        // whatever its fill, so charging entries under-reports every sparse
        // node, and under-charging is the direction this must not fail in.
        // Every non-root leaf holds at least `BTREE_MIN_FILL`, so
        // `1 + len / BTREE_MIN_FILL` bounds the leaves from above and internal
        // nodes are strictly fewer again. Zero on every production path,
        // because `place` never reaches the sparse table there.
        const BTREE_CAPACITY: u64 = 11;
        const BTREE_MIN_FILL: u64 = 5;
        const NODE_HEADER: u64 = 128;
        let sparse = if self.sparse.is_empty() {
            0
        } else {
            let node = NODE_HEADER
                + BTREE_CAPACITY
                    * (std::mem::size_of::<IntervalId>() as u64
                        + std::mem::size_of::<u32>() as u64);
            (1 + self.sparse.len() as u64 / BTREE_MIN_FILL) * node
        };
        index + slots + sparse + heap
    }

    /// Bring the ledger up to this table's real footprint. Reconciled rather
    /// than admitted for the reason the executor's own tables are: the growth
    /// points are one insert deep inside the attach pass.
    pub fn reconcile(&mut self, ledger: &CustodyLedger) -> Result<(), CustodyRefusal> {
        let bytes = self.capacity_bytes();
        if bytes <= self.charged {
            return Ok(());
        }
        let delta = bytes - self.charged;
        self.charged = bytes;
        ledger.charge_tables_overshoot(delta)
    }
}
