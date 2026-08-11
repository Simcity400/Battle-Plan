//! The frozen snapshot and the views readers get over it.
//!
//! # A view is not a pointer
//!
//! Post-bracket readers are handed a [`SnapshotView`]. Its whole API takes
//! ORIGINAL addresses — the addresses the reader's offsets are expressed in —
//! and returns copied bytes or a translated arena offset. There is no method
//! on this type that returns a live address, a raw pointer, or anything that
//! can be dereferenced against the observed process. A reader converted onto
//! this substrate cannot accidentally keep reading the game.
//!
//! To be precise about what that does and does not claim, because the
//! difference matters:
//!
//!  * The view's own HANDLES are never addresses. [`SnapshotView::translate`]
//!    returns an offset into a buffer this process owns.
//!  * The view's CONTENT is whatever the engine had. If a copied field holds a
//!    pointer, reading it yields that pointer's value, which is an address in
//!    the observed process. That is inherent to copying memory, not something
//!    an API can prevent, and the recipe engine depends on it: `DecodePointer`
//!    exists precisely to follow those values.
//!  * What makes the value harmless is that nothing reachable from a view can
//!    dereference it. A [`SnapshotView`] borrows a frozen byte buffer and
//!    holds no [`super::SnapshotMemory`], no handle, and no capability to read
//!    the observed process at all.
//!  * [`IntervalRecord::span`] likewise carries original addresses. It is the
//!    domain of the translation, so it could not be anything else.
//!
//! # A view is hour-locked
//!
//! [`Snapshot::view_for`] refuses a hour stamp that is not the snapshot's own.
//! That is the structural half of the hour-misattribution rule: a reader
//! cannot present hour H+1 work against hour H bytes even by mistake, because
//! the only handle it has carries H and checks it.
//!
//! # A view is gap-aware
//!
//! A consumer whose recipe refused, or whose bytes a closing witness found
//! changed, has no view at all: `view_for` returns the [`GapReason`]. Per
//! [`super::dedup`] that decision is per consumer, so a shared extent going
//! bad gaps its dependents and nobody else.

use std::collections::BTreeMap;

use super::dedup::{IntervalRecord, Links};
use super::{ConsumerId, GapReason, HourStamp, Span};

/// Bits of address per index bucket. 4 KiB pages: the same granularity the
/// address space is actually managed at.
pub const PAGE_SHIFT: u32 = 12;

/// Page/radix index over the frozen intervals.
///
/// `first[page]` is the index into the snapshot's address-ordered interval
/// table of the first interval STARTING in that page. A lookup takes the last
/// bucket at or before the wanted page and scans forward; because intervals
/// are disjoint and sorted, the scan stops at the first interval starting past
/// the address.
///
/// # The bucket entry is one position too far, and the lookup steps back
///
/// A bucket names the first interval STARTING in its page, but an interval
/// that started in an EARLIER page can still cover the address. Concretely,
/// with intervals `[0x1000, 0x2800)` and `[0x2900, 0x2908)` the buckets are
/// `{page 1 -> 0, page 2 -> 1}`; looking up `0x2400` lands on page 2, whose
/// entry is position 1, and position 1 starts past the address. Scanning from
/// there would report the address as never captured when position 0 holds it.
///
/// Stepping back one position is not a heuristic and is exactly sufficient.
/// Let `X` be the interval covering `addr` and `pos` the bucket entry. Any
/// interval `Z` strictly between `X` and `pos` would have to start after `X`
/// ends — i.e. after `addr` — and before the page whose bucket was taken,
/// which is at or before `addr`. No such `Z` exists, so `X` is at `pos - 1` or
/// at `pos` or later. `page_index_agrees_with_the_ordered_search` pins it
/// against a plain binary search over page-spanning intervals.
///
/// # Why an array rather than a map
///
/// The buckets are built once, from an already address-ordered table, and are
/// then read-only. That makes an ordered map pure overhead: the build inserts
/// one node per occupied page — up to one per interval on a scattered image —
/// where an array append is a push, and the lookup's `range(..=page)` is a
/// tree walk where `partition_point` is a bisection over contiguous memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageIndex {
    /// `(page, first position)`, ascending by page, one entry per page that
    /// has an interval starting in it.
    first: Vec<(u64, u32)>,
}

impl PageIndex {
    /// Build over an ALREADY address-ordered interval table, charging the
    /// buckets to the observation's custody ledger.
    ///
    /// The bucket list is work-scaled — up to one entry per interval on a
    /// scattered image — and it is built at the moment custody is fullest,
    /// with the arena and every ownership table still live. Charging it is
    /// what keeps "held" the whole footprint rather than the part that was
    /// convenient to count.
    pub fn build(
        ordered: &[IntervalRecord],
        ledger: &super::arena::CustodyLedger,
    ) -> Result<Self, super::CustodyRefusal> {
        let mut first: Vec<(u64, u32)> = Vec::new();
        for (position, record) in ordered.iter().enumerate() {
            let page = record.start() >> PAGE_SHIFT;
            // Ascending input, so a new page is always a new last bucket and
            // a repeated page already has its first position recorded.
            if first.last().map(|&(held, _)| held) != Some(page) {
                super::arena::charged_push(&mut first, ledger, (page, position as u32))?;
            }
        }
        Ok(Self { first })
    }

    /// Position in `order` to start scanning from for `addr`.
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

/// An immutable, hour-stamped image of everything one observation copied.
#[derive(Debug)]
pub struct Snapshot {
    hour: HourStamp,
    bytes: Vec<u8>,
    /// Address-ordered.
    intervals: Vec<IntervalRecord>,
    /// The dependency lists and discovery edges every record's chain heads
    /// index into. Frozen with the records, and the reason a record is 56
    /// bytes rather than 96 plus two heap allocations.
    links: Links,
    index: PageIndex,
    gapped: BTreeMap<ConsumerId, GapReason>,
    /// Per-interval consumption, samples and stage tags.
    annot: super::annot::Annotations,
    /// RULING A. Spans a speculative copy could not read. Empty on every
    /// observation whose recipes all declare `Tolerance::Required`, which is
    /// what makes the void check on the read path a single predictable branch.
    voids: Vec<super::Void>,
    /// Pointers decoded, rejected on admissibility, and not followed.
    rejections: Vec<super::PointerRejection>,
}

impl Snapshot {
    /// Freeze an ALREADY address-ordered record table.
    ///
    /// The ordering is the caller's to establish, and
    /// [`super::dedup::OwnershipMap::into_frozen`] is what establishes it: the
    /// ownership index already holds the permutation, so asking for it there
    /// is a pass over 24-byte index entries instead of a comparison sort over
    /// the record table. Sorting here as well would pay for the same order
    /// twice, once per observation, on the largest table the substrate
    /// builds.
    ///
    /// A record's position in the frozen table is therefore NOT its
    /// [`IntervalId`]; the id stays in the record, which is what the witness
    /// comparison keys its edges on.
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
            records.windows(2).all(|pair| pair[0].start() <= pair[1].start()),
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

    /// The dependency lists and discovery edges of the frozen intervals.
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

    /// RULING A. The spans a speculative copy could not read.
    pub fn voids(&self) -> &[super::Void] {
        &self.voids
    }

    /// Pointers a recipe decoded, rejected on admissibility, and did not
    /// follow. Each carries the witness a converted reader needs to raise its
    /// own typed failure.
    pub fn pointer_rejections(&self) -> &[super::PointerRejection] {
        &self.rejections
    }

    /// Rejections belonging to one consumer.
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

    /// The first void `want` intersects, if any.
    ///
    /// The empty case is the overwhelming majority and it is one `is_empty`
    /// test. A non-empty list is short by construction: it holds one entry per
    /// unreadable hole of a speculative copy, and an observation with many of
    /// those is an observation reading a region that has gone away.
    fn void_at(&self, want: &Span) -> Option<&super::Void> {
        if self.voids.is_empty() {
            return None;
        }
        self.voids.iter().find(|void| void.span.overlaps(want))
    }

    pub(super) fn gap_consumer(&mut self, consumer: ConsumerId, reason: GapReason) {
        self.gapped.entry(consumer).or_insert(reason);
    }

    /// Untranslated arena bytes. `pub(super)` because only the witness
    /// comparison has a legitimate reason to look at the arena by offset — a
    /// reader must go through [`SnapshotView`], which checks the hour, the
    /// gap set and the consumer's ownership.
    pub(super) fn raw(&self, offset: usize, len: usize) -> &[u8] {
        &self.bytes[offset..offset + len]
    }

    /// A view for one consumer at one hour, or the reason it has none.
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

    /// Address-ordered position of the interval containing `addr`.
    ///
    /// Goes through the page index and falls back to scanning from the front
    /// when the index has no bucket at or before the address. This and
    /// [`Self::locate_by_search`] answer the same question;
    /// `page_index_agrees_with_the_ordered_search` pins that.
    fn locate(&self, addr: u64) -> Option<usize> {
        // The step back is load-bearing, not defensive: see `PageIndex`.
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

/// Why a reader has no view, or no bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewRefusal {
    /// The reader asked for an hour this snapshot is not.
    ForeignHour { snapshot: HourStamp, asked: HourStamp },
    /// This consumer's hour is gapped.
    ConsumerGapped(GapReason),
    /// The observation never copied these bytes.
    NotCaptured { span: Span },
    /// The bytes were copied, but as separate transactions, so no single
    /// arena slice holds them. [`SnapshotView::read_into`] stitches this case;
    /// [`SnapshotView::bytes`] cannot and says so.
    NotContiguous { span: Span },
    /// The consumer does not depend on these bytes. A reader may only see what
    /// its own recipe asked for: sharing an extent is an implementation
    /// detail of the copier, never a widening of what a reader may look at.
    NotOwned { span: Span },
    /// RULING A. The observation asked for these bytes as part of a
    /// speculative copy and the engine could not read them.
    ///
    /// Distinct from [`ViewRefusal::NotCaptured`] on purpose, and the
    /// distinction is the whole ruling: `NotCaptured` is a recipe that
    /// under-asked, which is our defect, while this is a span the recipe DID
    /// ask for and the engine refused. Only the reads that reach it fail; every
    /// other byte of the same copy is served normally, so a row whose consumed
    /// bytes never touch the void publishes.
    Unreadable { span: Span, void: Span },
}

/// A reader's handle on the frozen arena. Immutable, hour-locked, own-hour
/// only, and translated: every entry point takes an ORIGINAL address.
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

    /// Translate an original address to its offset in the frozen arena.
    ///
    /// This is an offset into a byte buffer this process owns. It is not an
    /// address in the observed process and cannot be turned back into one.
    pub fn translate(&self, addr: u64) -> Result<usize, ViewRefusal> {
        let want = Span::new(addr, 1);
        // Same order as `bytes`: a void has no interval behind it, so without
        // this the reader would be told its recipe under-asked for a byte its
        // recipe did ask for.
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

    /// Borrow `len` bytes at `addr` without copying.
    pub fn bytes(&self, addr: u64, len: usize) -> Result<&'a [u8], ViewRefusal> {
        let want = Span::new(addr, len as u64);
        // The void check comes FIRST, because a void is exactly a span with no
        // interval behind it: without this the reader would be told its recipe
        // under-asked for bytes its recipe did ask for.
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
        // Contiguity FIRST. A request spanning two transactions is never
        // inside one interval's reach, so checking ownership first would
        // report every stitchable read as `NotOwned` and hide the real reason.
        if !record.span().contains_span(&want) {
            // Either the tail was never copied, or it was copied by another
            // transaction. Distinguish, because they are different bugs: the
            // first is a recipe that under-asked, the second is a reader that
            // wants a stitch.
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

    /// Fill `out` from `addr`, stitching across as many owned intervals as it
    /// takes. Refuses if any byte of the request was not copied, or if any
    /// covering interval is not this consumer's.
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
            out[written..written + take].copy_from_slice(&self.snapshot.bytes[offset..offset + take]);
            written += take;
            // CHECKED, not wrapping: two owned intervals at the top and the
            // bottom of the address space are not adjacent, and stitching
            // across the wrap would return bytes from address 0 for a request
            // whose second byte is at 2^64.
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

    /// Borrow `len` bytes at `addr` from one of the images the copier took.
    ///
    /// `image` 0 is the opening copy — the same bytes [`Self::bytes`] returns.
    /// Image `k` for `k >= 1` is the `k`-th ADJACENT sample, taken back to back
    /// with the opening one; [`Self::closing_bytes`] is the separate closing
    /// image. This is what lets a converted reader run its own A/B comparison
    /// instead of taking the substrate's byte-wise verdict.
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

    /// How many images of the bytes at `addr` the copier holds, the closing one
    /// excluded.
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

    /// Borrow `len` bytes at `addr` from the CLOSING image.
    ///
    /// RULING B leaves the substrate's own comparison in place — it is what
    /// gaps a consumer whose consumed bytes moved — and adds this so a reader
    /// that compares DECODED FIELDS can do so itself. Refuses
    /// [`ViewRefusal::NotCaptured`] when no closing copy was taken, because an
    /// absent closing image certifies nothing and must never read as one that
    /// happens to be equal.
    pub fn closing_bytes(&self, addr: u64, len: usize) -> Result<&'a [u8], ViewRefusal> {
        let want = Span::new(addr, len as u64);
        let (record, skew) = self.owned_record(want)?;
        let closing = record
            .closing_offset()
            .ok_or(ViewRefusal::NotCaptured { span: want })?;
        let offset = closing + skew;
        Ok(&self.snapshot.bytes[offset..offset + len])
    }

    /// Borrow `len` bytes at `addr` from the `image`-th CLOSING image.
    ///
    /// Image 0 is [`Self::closing_bytes`]. Image `k >= 1` is the `k`-th adjacent
    /// closing observation, taken back to back with it — the closing side of
    /// [`Self::sample_bytes`], and the instrument a converted reader renders its
    /// own closing torn verdict from.
    ///
    /// # The two ways an image can be absent are DIFFERENT ANSWERS
    ///
    /// [`ViewRefusal::Unreadable`] when the interval's consumers ASKED for this
    /// image and the closing wave could not take it: the engine refused an
    /// observation the reader needs, which is the live reader's own second
    /// closing read going unreadable and must refuse with that reason.
    /// [`ViewRefusal::NotCaptured`] when nobody asked for it, which is a reader
    /// wanting an observation its own plan never requested — its defect, not the
    /// engine's.
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

    /// The interval that wholly holds `want` and this consumer may read, with
    /// the offset of `want` inside it. Shared by the image accessors, which all
    /// need the same three checks in the same order.
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

    /// True when every byte of `span` is owned by some interval.
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
    /// Test-only accessor for the binary-search path, so the index and the
    /// search can be pinned against each other.
    pub(super) fn locate_both(&self, addr: u64) -> (Option<usize>, Option<usize>) {
        (self.locate(addr), self.locate_by_search(addr))
    }
}

#[cfg(not(test))]
impl Snapshot {
    /// Kept compiled outside tests so the fallback is not dead code drifting
    /// away from the indexed path it backs up.
    pub fn locate_fallback(&self, addr: u64) -> Option<usize> {
        self.locate_by_search(addr)
    }
}
