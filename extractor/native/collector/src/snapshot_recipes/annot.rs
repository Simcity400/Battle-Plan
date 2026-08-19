use std::collections::BTreeMap;

use super::arena::{vec_bytes, CustodyLedger};
use super::{CustodyRefusal, IntervalId, Span};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StridedMask {
    pub base: u64,
    pub stride: u32,
    pub count: u32,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Consumption {
    pub full: bool,
    pub spans: Vec<Span>,
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

    pub fn images_agree(&self, interval: Span, opening: &[u8], closing: &[u8]) -> bool {
        if self.full {
            return opening == closing;
        }
        if self.spans.is_empty() && self.strided.is_empty() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleAsk {
    pub consumer: super::ConsumerId,
    pub wanted: u8,
    pub gap_on_disagree: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleImages {
    pub extra: Vec<u32>,
    pub success_mask: u8,
    pub asks: Vec<SampleAsk>,
}

impl SampleImages {
    pub fn held(&self) -> usize {
        1 + self.extra.len()
    }

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

    pub fn unmet(&self) -> impl Iterator<Item = super::ConsumerId> + '_ {
        let held = self.held();
        self.asks
            .iter()
            .filter(move |ask| ask.gap_on_disagree && held < ask.wanted as usize)
            .map(|ask| ask.consumer)
    }

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

    pub fn anyone_gaps(&self) -> bool {
        self.asks.iter().any(|ask| ask.gap_on_disagree)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClosingImages {
    pub extra: Vec<u32>,
}

impl ClosingImages {
    pub fn held(&self) -> usize {
        1 + self.extra.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseAsk {
    pub consumer: super::ConsumerId,
    pub gap_on_change: bool,
}

#[derive(Clone, Debug, Default)]
struct IntervalAnnot {
    consumed: Option<Consumption>,
    samples: Option<SampleImages>,
    closing: Option<ClosingImages>,
    closes: Vec<CloseAsk>,
    stages: Vec<u16>,
}

/// order by the ownership map — `dedup.rs` mints it as `records.len()` — so the
#[derive(Clone, Debug, Default)]
pub struct Annotations {
    index: Vec<u32>,
    /// every path production takes, because the ownership map mints ids
    sparse: BTreeMap<IntervalId, u32>,
    slots: Vec<IntervalAnnot>,
    armed: bool,
    charged: u64,
}

pub struct IntervalEntry<'a> {
    slot: &'a mut IntervalAnnot,
}

impl IntervalEntry<'_> {
    pub fn tag_stage(&mut self, stage: u16) {
        if stage == 0 {
            return;
        }
        if !self.slot.stages.contains(&stage) {
            self.slot.stages.push(stage);
        }
    }

    pub fn ask_close(&mut self, consumer: super::ConsumerId, gap_on_change: bool) {
        let request = CloseAsk {
            consumer,
            gap_on_change,
        };
        if !self.slot.closes.contains(&request) {
            self.slot.closes.push(request);
        }
    }

    pub fn consume_all(&mut self) {
        let entry = self.slot.consumed.get_or_insert_with(Consumption::default);
        entry.full = true;
        entry.spans.clear();
        entry.strided.clear();
    }

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

    pub fn arm(&mut self) {
        self.armed = true;
    }

    pub fn armed(&self) -> bool {
        self.armed
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn entry(&mut self, interval: IntervalId) -> IntervalEntry<'_> {
        IntervalEntry {
            slot: self.slot_mut(interval),
        }
    }

    fn slot_mut(&mut self, interval: IntervalId) -> &mut IntervalAnnot {
        let at = self.place(interval);
        &mut self.slots[at - 1]
    }

    /// `IntervalId` is `pub` and `entry` is `pub`, so "the ownership map mints
    fn place(&mut self, interval: IntervalId) -> usize {
        const DENSE_INDEX_SLACK: usize = 1024;

        let id = interval.0 as usize;
        if let Some(&at) = self.index.get(id) {
            if at != 0 {
                return at as usize;
            }
        }
        if !self.sparse.is_empty() {
            if let Some(&at) = self.sparse.get(&interval) {
                return at as usize;
            }
        }
        if id >= self.index.len()
            && id <= self.slots.len().saturating_add(DENSE_INDEX_SLACK)
            && self
                .sparse
                .keys()
                .next()
                .is_none_or(|low| id < low.0 as usize)
        {
            self.index.resize(id + 1, 0);
        }
        self.slots.push(IntervalAnnot::default());
        let at = self.slots.len() as u32;
        if id < self.index.len() {
            self.index[id] = at;
        } else {
            self.sparse.insert(interval, at);
        }
        at as usize
    }

    fn slot(&self, interval: IntervalId) -> Option<&IntervalAnnot> {
        let at = match self.index.get(interval.0 as usize).copied() {
            Some(at) if at != 0 => at,
            _ => *self.sparse.get(&interval)?,
        };
        self.slots.get(at as usize - 1)
    }

    pub fn closing_images_wanted(&self, interval: IntervalId) -> u8 {
        self.slot(interval)
            .and_then(|slot| slot.samples.as_ref())
            .map(|images| images.asks.iter().map(|ask| ask.wanted).max().unwrap_or(1))
            .unwrap_or(1)
            .max(1)
    }

    pub fn push_closing_image(&mut self, interval: IntervalId, offset: u32) {
        self.slot_mut(interval)
            .closing
            .get_or_insert_with(ClosingImages::default)
            .extra
            .push(offset);
    }

    pub fn closing_images(&self, interval: IntervalId) -> Option<&ClosingImages> {
        self.slot(interval).and_then(|slot| slot.closing.as_ref())
    }

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

    pub fn consume_all(&mut self, interval: IntervalId) {
        self.entry(interval).consume_all();
    }

    pub fn consume_spans(&mut self, interval: IntervalId, spans: &[Span]) {
        self.entry(interval).consume_spans(spans);
    }

    pub fn consume_strided(&mut self, interval: IntervalId, mask: StridedMask) {
        self.entry(interval).consume_strided(mask);
    }

    pub fn consumption(&self, interval: IntervalId) -> Option<&Consumption> {
        self.slot(interval).and_then(|slot| slot.consumed.as_ref())
    }

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

    pub fn sampled_intervals(&self) -> impl Iterator<Item = (IntervalId, &SampleImages)> {
        let dense = self.index.iter().enumerate().filter_map(move |(id, at)| {
            let slot = self.slots.get((*at as usize).checked_sub(1)?)?;
            slot.samples
                .as_ref()
                .map(|images| (IntervalId(id as u32), images))
        });
        let sparse = self.sparse.iter().filter_map(move |(id, at)| {
            let slot = self.slots.get((*at as usize).checked_sub(1)?)?;
            slot.samples.as_ref().map(|images| (*id, images))
        });
        dense.chain(sparse)
    }

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

        let index = self.index.capacity() as u64 * std::mem::size_of::<u32>() as u64;
        let slots = self.slots.capacity() as u64 * std::mem::size_of::<IntervalAnnot>() as u64;
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
