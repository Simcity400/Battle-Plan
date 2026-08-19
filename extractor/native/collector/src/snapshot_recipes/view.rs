use std::collections::BTreeMap;

use super::dedup::{IntervalRecord, Links};
use super::{ConsumerId, GapReason, HourStamp, Span};

pub const PAGE_SHIFT: u32 = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageIndex {
    first: Vec<(u64, u32)>,
}

impl PageIndex {
    /// with the arena and every ownership table still live. Charging it is
    pub fn build(
        ordered: &[IntervalRecord],
        ledger: &super::arena::CustodyLedger,
    ) -> Result<Self, super::CustodyRefusal> {
        let mut first: Vec<(u64, u32)> = Vec::new();
        for (position, record) in ordered.iter().enumerate() {
            let page = record.start() >> PAGE_SHIFT;
            if first.last().map(|&(held, _)| held) != Some(page) {
                super::arena::charged_push(&mut first, ledger, (page, position as u32))?;
            }
        }
        Ok(Self { first })
    }

    fn entry_for(&self, addr: u64) -> Option<u32> {
        let page = addr >> PAGE_SHIFT;
        let at = self.first.partition_point(|&(held, _)| held <= page);
        if at == 0 {
            return None;
        }
        Some(self.first[at - 1].1)
    }

    pub fn buckets(&self) -> usize {
        self.first.len()
    }
}

#[derive(Debug)]
pub struct Snapshot {
    hour: HourStamp,
    bytes: Vec<u8>,
    intervals: Vec<IntervalRecord>,
    links: Links,
    index: PageIndex,
    gapped: BTreeMap<ConsumerId, GapReason>,
    annot: super::annot::Annotations,
    voids: Vec<super::Void>,
    rejections: Vec<super::PointerRejection>,
}

impl Snapshot {
    /// [`super::dedup::OwnershipMap::into_frozen`] is what establishes it: the
    /// ownership index already holds the permutation, so asking for it there
    #[allow(clippy::too_many_arguments)]
    pub(super) fn freeze(
        hour: HourStamp,
        bytes: Vec<u8>,
        records: Vec<IntervalRecord>,
        links: Links,
        gapped: BTreeMap<ConsumerId, GapReason>,
        annot: super::annot::Annotations,
        voids: Vec<super::Void>,
        rejections: Vec<super::PointerRejection>,
        ledger: &super::arena::CustodyLedger,
    ) -> Result<Self, super::CustodyRefusal> {
        debug_assert!(
            records
                .windows(2)
                .all(|pair| pair[0].start() <= pair[1].start()),
            "freeze requires an address-ordered record table",
        );
        let index = PageIndex::build(&records, ledger)?;
        Ok(Self {
            hour,
            bytes,
            intervals: records,
            links,
            index,
            gapped,
            annot,
            voids,
            rejections,
        })
    }

    pub fn hour(&self) -> HourStamp {
        self.hour
    }

    pub fn custody_bytes(&self) -> u64 {
        self.bytes.len() as u64
    }

    pub fn interval_count(&self) -> usize {
        self.intervals.len()
    }

    pub fn intervals(&self) -> &[IntervalRecord] {
        &self.intervals
    }

    pub fn links(&self) -> &Links {
        &self.links
    }

    pub fn index(&self) -> &PageIndex {
        &self.index
    }

    pub fn gapped(&self) -> &BTreeMap<ConsumerId, GapReason> {
        &self.gapped
    }

    pub fn gap_reason(&self, consumer: ConsumerId) -> Option<GapReason> {
        self.gapped.get(&consumer).copied()
    }

    pub fn voids(&self) -> &[super::Void] {
        &self.voids
    }

    pub fn pointer_rejections(&self) -> &[super::PointerRejection] {
        &self.rejections
    }

    pub fn rejections_for(
        &self,
        consumer: ConsumerId,
    ) -> impl Iterator<Item = &super::PointerRejection> {
        self.rejections
            .iter()
            .filter(move |rejection| rejection.consumer == consumer)
    }

    pub fn annotations(&self) -> &super::annot::Annotations {
        &self.annot
    }

    fn void_at(&self, want: &Span) -> Option<&super::Void> {
        if self.voids.is_empty() {
            return None;
        }
        self.voids.iter().find(|void| void.span.overlaps(want))
    }

    pub(super) fn gap_consumer(&mut self, consumer: ConsumerId, reason: GapReason) {
        self.gapped.entry(consumer).or_insert(reason);
    }

    /// gap set and the consumer's ownership.
    pub(super) fn raw(&self, offset: usize, len: usize) -> &[u8] {
        &self.bytes[offset..offset + len]
    }

    pub fn view_for(
        &self,
        consumer: ConsumerId,
        hour: HourStamp,
    ) -> Result<SnapshotView<'_>, ViewRefusal> {
        if hour != self.hour {
            return Err(ViewRefusal::ForeignHour {
                snapshot: self.hour,
                asked: hour,
            });
        }
        if let Some(&reason) = self.gapped.get(&consumer) {
            return Err(ViewRefusal::ConsumerGapped(reason));
        }
        Ok(SnapshotView {
            snapshot: self,
            consumer,
        })
    }

    fn locate(&self, addr: u64) -> Option<usize> {
        let start = self
            .index
            .entry_for(addr)
            .map(|position| position.saturating_sub(1) as usize)
            .unwrap_or(0);
        let mut position = start;
        while position < self.intervals.len() {
            let span = self.intervals[position].span();
            if span.contains_addr(addr) {
                return Some(position);
            }
            if span.start > addr {
                return None;
            }
            position += 1;
        }
        None
    }

    fn locate_by_search(&self, addr: u64) -> Option<usize> {
        let position = self
            .intervals
            .partition_point(|record| record.start() <= addr);
        if position == 0 {
            return None;
        }
        let candidate = position - 1;
        if self.intervals[candidate].span().contains_addr(addr) {
            Some(candidate)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRefusal {
    ForeignHour {
        snapshot: HourStamp,
        asked: HourStamp,
    },
    ConsumerGapped(GapReason),
    NotCaptured {
        span: Span,
    },
    NotContiguous {
        span: Span,
    },
    NotOwned {
        span: Span,
    },
    Unreadable {
        span: Span,
        void: Span,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct SnapshotView<'a> {
    snapshot: &'a Snapshot,
    consumer: ConsumerId,
}

impl<'a> SnapshotView<'a> {
    pub fn hour(&self) -> HourStamp {
        self.snapshot.hour
    }

    pub fn consumer(&self) -> ConsumerId {
        self.consumer
    }

    pub fn translate(&self, addr: u64) -> Result<usize, ViewRefusal> {
        let want = Span::new(addr, 1);
        if let Some(void) = self.snapshot.void_at(&want) {
            return Err(ViewRefusal::Unreadable {
                span: want,
                void: void.span,
            });
        }
        let position = self
            .snapshot
            .locate(addr)
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let record = &self.snapshot.intervals[position];
        if !self.snapshot.links.reaches(record, self.consumer, &want) {
            return Err(ViewRefusal::NotOwned { span: want });
        }
        Ok(record.opening_offset() + (addr - record.start()) as usize)
    }

    pub fn bytes(&self, addr: u64, len: usize) -> Result<&'a [u8], ViewRefusal> {
        let want = Span::new(addr, len as u64);
        if let Some(void) = self.snapshot.void_at(&want) {
            return Err(ViewRefusal::Unreadable {
                span: want,
                void: void.span,
            });
        }
        let position = self
            .snapshot
            .locate(addr)
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let record = &self.snapshot.intervals[position];
        // inside one interval's reach, so checking ownership first would
        if !record.span().contains_span(&want) {
            return Err(if self.covered(want) {
                ViewRefusal::NotContiguous { span: want }
            } else {
                ViewRefusal::NotCaptured { span: want }
            });
        }
        if !self.snapshot.links.reaches(record, self.consumer, &want) {
            return Err(ViewRefusal::NotOwned { span: want });
        }
        let skew = (addr - record.start()) as usize;
        let offset = record.opening_offset() + skew;
        Ok(&self.snapshot.bytes[offset..offset + len])
    }

    pub fn read_into(&self, addr: u64, out: &mut [u8]) -> Result<(), ViewRefusal> {
        let want = Span::new(addr, out.len() as u64);
        if out.is_empty() {
            return Ok(());
        }
        if let Some(void) = self.snapshot.void_at(&want) {
            return Err(ViewRefusal::Unreadable {
                span: want,
                void: void.span,
            });
        }
        let mut cursor = addr;
        let mut written = 0usize;
        while written < out.len() {
            let position = self
                .snapshot
                .locate(cursor)
                .ok_or(ViewRefusal::NotCaptured { span: want })?;
            let record = &self.snapshot.intervals[position];
            let skew = (cursor - record.start()) as usize;
            let available = record.len() as usize - skew;
            let take = available.min(out.len() - written);
            if !self
                .snapshot
                .links
                .reaches(record, self.consumer, &Span::new(cursor, take as u64))
            {
                return Err(ViewRefusal::NotOwned { span: want });
            }
            let offset = record.opening_offset() + skew;
            out[written..written + take]
                .copy_from_slice(&self.snapshot.bytes[offset..offset + take]);
            written += take;
            cursor = match cursor.checked_add(take as u64) {
                Some(next) => next,
                None => {
                    if written == out.len() {
                        break;
                    }
                    return Err(ViewRefusal::NotCaptured { span: want });
                }
            };
        }
        Ok(())
    }

    pub fn sample_bytes(
        &self,
        addr: u64,
        len: usize,
        image: usize,
    ) -> Result<&'a [u8], ViewRefusal> {
        if image == 0 {
            return self.bytes(addr, len);
        }
        let want = Span::new(addr, len as u64);
        let (record, skew) = self.owned_record(want)?;
        let images = self
            .snapshot
            .annot
            .samples(record.id())
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let offset = *images
            .extra
            .get(image - 1)
            .ok_or(ViewRefusal::NotCaptured { span: want })? as usize
            + skew;
        Ok(&self.snapshot.bytes[offset..offset + len])
    }

    pub fn sample_count(&self, addr: u64) -> usize {
        match self.snapshot.locate(addr) {
            Some(position) => {
                let record = &self.snapshot.intervals[position];
                1 + self
                    .snapshot
                    .annot
                    .samples(record.id())
                    .map(|images| images.extra.len())
                    .unwrap_or(0)
            }
            None => 0,
        }
    }

    pub fn closing_bytes(&self, addr: u64, len: usize) -> Result<&'a [u8], ViewRefusal> {
        let want = Span::new(addr, len as u64);
        let (record, skew) = self.owned_record(want)?;
        let closing = record
            .closing_offset()
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let offset = closing + skew;
        Ok(&self.snapshot.bytes[offset..offset + len])
    }

    pub fn closing_sample_bytes(
        &self,
        addr: u64,
        len: usize,
        image: usize,
    ) -> Result<&'a [u8], ViewRefusal> {
        if image == 0 {
            return self.closing_bytes(addr, len);
        }
        let want = Span::new(addr, len as u64);
        let (record, skew) = self.owned_record(want)?;
        let held = self.snapshot.annot.closing_images(record.id());
        let offset = held.and_then(|images| images.extra.get(image - 1).copied());
        match offset {
            Some(offset) => {
                let at = offset as usize + skew;
                Ok(&self.snapshot.bytes[at..at + len])
            }
            None => {
                let wanted = self.snapshot.annot.closing_images_wanted(record.id()) as usize;
                Err(if image < wanted {
                    ViewRefusal::Unreadable {
                        span: want,
                        void: record.span(),
                    }
                } else {
                    ViewRefusal::NotCaptured { span: want }
                })
            }
        }
    }

    fn owned_record(&self, want: Span) -> Result<(&'a IntervalRecord, usize), ViewRefusal> {
        if let Some(void) = self.snapshot.void_at(&want) {
            return Err(ViewRefusal::Unreadable {
                span: want,
                void: void.span,
            });
        }
        let position = self
            .snapshot
            .locate(want.start)
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let record = &self.snapshot.intervals[position];
        if !record.span().contains_span(&want) {
            return Err(if self.covered(want) {
                ViewRefusal::NotContiguous { span: want }
            } else {
                ViewRefusal::NotCaptured { span: want }
            });
        }
        if !self.snapshot.links.reaches(record, self.consumer, &want) {
            return Err(ViewRefusal::NotOwned { span: want });
        }
        Ok((record, (want.start - record.start()) as usize))
    }

    pub fn u8_at(&self, addr: u64) -> Result<u8, ViewRefusal> {
        Ok(self.bytes(addr, 1)?[0])
    }

    pub fn u32_le(&self, addr: u64) -> Result<u32, ViewRefusal> {
        let mut raw = [0u8; 4];
        self.read_into(addr, &mut raw)?;
        Ok(u32::from_le_bytes(raw))
    }

    pub fn u64_le(&self, addr: u64) -> Result<u64, ViewRefusal> {
        let mut raw = [0u8; 8];
        self.read_into(addr, &mut raw)?;
        Ok(u64::from_le_bytes(raw))
    }

    fn covered(&self, span: Span) -> bool {
        let mut cursor = u128::from(span.start);
        let end = span.end_excl();
        while cursor < end {
            let addr = match u64::try_from(cursor) {
                Ok(addr) => addr,
                Err(_) => return false,
            };
            match self.snapshot.locate(addr) {
                Some(position) => cursor = self.snapshot.intervals[position].span().end_excl(),
                None => return false,
            }
        }
        true
    }
}

#[cfg(test)]
impl Snapshot {
    pub(super) fn locate_both(&self, addr: u64) -> (Option<usize>, Option<usize>) {
        (self.locate(addr), self.locate_by_search(addr))
    }
}

#[cfg(not(test))]
impl Snapshot {
    pub fn locate_fallback(&self, addr: u64) -> Option<usize> {
        self.locate_by_search(addr)
    }
}
