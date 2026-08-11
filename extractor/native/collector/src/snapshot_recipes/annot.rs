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
#[derive(Clone, Debug, Default)]
pub struct Annotations {
    per_interval: BTreeMap<IntervalId, IntervalAnnot>,
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
        self.per_interval.is_empty()
    }

    /// One interval's annotations, opened once for a whole request's worth of
    /// writes. See [`IntervalEntry`].
    pub fn entry(&mut self, interval: IntervalId) -> IntervalEntry<'_> {
        IntervalEntry {
            slot: self.per_interval.entry(interval).or_default(),
        }
    }

    /// One interval's annotations, or `None` when nothing has been recorded
    /// against it.
    fn slot(&self, interval: IntervalId) -> Option<&IntervalAnnot> {
        self.per_interval.get(&interval)
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
        self.per_interval
            .entry(interval)
            .or_default()
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
        self.per_interval.entry(interval).or_default().samples = Some(images);
    }

    pub fn samples(&self, interval: IntervalId) -> Option<&SampleImages> {
        self.slot(interval).and_then(|slot| slot.samples.as_ref())
    }

    /// ASCENDING BY INTERVAL ID, which is what the map-keyed spelling produced
    /// and what `close_witnesses` walks in.
    pub fn sampled_intervals(&self) -> impl Iterator<Item = (&IntervalId, &SampleImages)> {
        self.per_interval
            .iter()
            .filter_map(|(id, slot)| slot.samples.as_ref().map(|images| (id, images)))
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

    /// Modelled footprint. Approximate in the same way
    /// [`super::arena::hash_table_bytes`] is: node overhead is modelled, not
    /// measured, because a `BTreeMap`'s allocation is not observable.
    /// Modelled footprint.
    ///
    /// # Modelled from `size_of`, not from a constant somebody guessed
    ///
    /// A `BTreeMap` entry costs the KEY, the VALUE INLINE, and the node's own
    /// per-entry overhead. The earlier version charged one flat 48 bytes per
    /// entry, which is less than a `Consumption` alone: that value holds two
    /// `Vec` headers at 24 bytes each. At a hundred thousand masked intervals
    /// the ledger under-reported by more than a megabyte, and an under-model is
    /// custody the ceiling never sees — the one direction the accounting must
    /// not fail in.
    ///
    /// # A NODE is allocated whole, so the model charges nodes, not entries
    ///
    /// A `BTreeMap` leaf holds fixed arrays for [`BTREE_CAPACITY`] keys and the
    /// same number of values and is allocated at that size whatever its current
    /// length. Charging one key plus one initialised value per entry therefore
    /// under-reports every sparse node — and an under-model is custody the
    /// ceiling never sees, which is the one direction this accounting must not
    /// fail in.
    ///
    /// So the charge is `nodes * node_size`, with the node count bounded from
    /// above: every leaf except the root holds at least [`BTREE_MIN_FILL`]
    /// entries, so `1 + len / BTREE_MIN_FILL` is an upper bound on the leaves,
    /// and internal nodes are strictly fewer again and are covered by the same
    /// `+1` on small maps and by the slack on large ones. The heap each inline
    /// `Vec` owns is then added per entry, because that part is real and is not
    /// part of the node.
    ///
    /// It over-charges, deliberately. Over-charging costs an observation
    /// headroom it did not need; under-charging costs the ceiling its meaning.
    pub fn capacity_bytes(&self) -> u64 {
        /// Entries a `BTreeMap` node reserves slots for, whatever its length.
        const BTREE_CAPACITY: u64 = 11;
        /// Entries every non-root leaf holds at least.
        const BTREE_MIN_FILL: u64 = 5;
        /// A node's own header: parent pointer, parent index, length, and the
        /// edge array on an internal node.
        const NODE_HEADER: u64 = 128;

        let table = |len: usize, value: usize, heap: u64| -> u64 {
            if len == 0 {
                return 0;
            }
            let node = NODE_HEADER
                + BTREE_CAPACITY
                    * (std::mem::size_of::<IntervalId>() as u64 + value as u64);
            let nodes = 1 + len as u64 / BTREE_MIN_FILL;
            nodes * node + heap
        };

        // ONE TABLE, because there is one map. The five separate charges this
        // replaces summed five node arrays for an interval that appeared in all
        // five; the merged entry is one node array holding one value that
        // carries all five fields, and `size_of::<IntervalAnnot>()` is that
        // value. The heap each of its `Vec`s owns is added per entry exactly as
        // before, because that part is real and is not part of the node.
        //
        // IT IS NOT UNIFORMLY CHEAPER OR DEARER THAN THE FIVE, and that is
        // worth saying because this figure decides when an observation at the
        // ceiling refuses: an interval that carried all five is charged one node
        // slot where it was charged five, and an interval that carried only a
        // stage is charged a whole `IntervalAnnot` where it was charged a
        // `Vec<u16>`. The model still tracks the structure that exists, which is
        // the only property it has to have.
        let mut heap = 0u64;
        for slot in self.per_interval.values() {
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

        table(
            self.per_interval.len(),
            std::mem::size_of::<IntervalAnnot>(),
            heap,
        )
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
