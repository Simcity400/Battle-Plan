//! The owned walk. The parent cursor is unavailable until every declared block
//! was walked or explicitly discarded.

use crate::{
    pcv_country_row_at, BaseHeader, BaseLayout, FixedRows, PcvCountryRow, PcvHeader, PdqHeader,
    PlicHeader, Span, Trailer, Violation,
};

// =============================================================================
// Caller defects and generic blocks
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BlockOwner {
    Base,
    Trailer(Trailer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DeclaredBlock {
    Stockpiles,
    Lines,
    Construction,
    Rows,
    Countries,
    Items,
    Variants,
    Reservations,
    InnerPayload,
}

impl DeclaredBlock {
    pub const ALL: [DeclaredBlock; 9] = [
        DeclaredBlock::Stockpiles,
        DeclaredBlock::Lines,
        DeclaredBlock::Construction,
        DeclaredBlock::Rows,
        DeclaredBlock::Countries,
        DeclaredBlock::Items,
        DeclaredBlock::Variants,
        DeclaredBlock::Reservations,
        DeclaredBlock::InnerPayload,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            DeclaredBlock::Stockpiles => "stockpiles",
            DeclaredBlock::Lines => "lines",
            DeclaredBlock::Construction => "construction",
            DeclaredBlock::Rows => "rows",
            DeclaredBlock::Countries => "countries",
            DeclaredBlock::Items => "items",
            DeclaredBlock::Variants => "variants",
            DeclaredBlock::Reservations => "reservations",
            DeclaredBlock::InnerPayload => "inner payload",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallerDefect {
    BodyNotDelimited,
    BlockNotConsumed {
        owner: BlockOwner,
        block: DeclaredBlock,
        remaining: usize,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowStep {
    Row { index: usize, at: usize },
    End(usize),
}

#[derive(Debug)]
#[must_use]
struct RowCursor {
    at: usize,
    index: usize,
    count: usize,
    stride: usize,
}

impl RowCursor {
    const fn from_span(span: Span, count: usize, stride: usize) -> Self {
        RowCursor {
            at: span.start,
            index: 0,
            count,
            stride,
        }
    }

    const fn remaining(&self) -> usize {
        self.count - self.index
    }

    const fn offset(&self) -> usize {
        self.at
    }

    fn next(&mut self) -> RowStep {
        let step = self.peek();
        if matches!(step, RowStep::Row { .. }) {
            self.advance();
        }
        step
    }

    fn peek(&self) -> RowStep {
        if self.index >= self.count {
            return RowStep::End(self.at);
        }
        RowStep::Row {
            index: self.index,
            at: self.at,
        }
    }

    fn advance(&mut self) {
        if self.index >= self.count {
            return;
        }
        self.at += self.stride;
        self.index += 1;
    }

    fn discard_all(&mut self) {
        self.at += self.remaining() * self.stride;
        self.index = self.count;
    }
}

#[derive(Debug)]
#[must_use]
pub struct BlockEnd<N> {
    pub at: usize,
    pub next: N,
}

#[derive(Debug)]
#[must_use]
pub struct BlockCursor<N> {
    rows: RowCursor,
    next: N,
    owner: BlockOwner,
    block: DeclaredBlock,
}

impl<N> BlockCursor<N> {
    fn new(
        span: Span,
        count: usize,
        stride: usize,
        next: N,
        owner: BlockOwner,
        block: DeclaredBlock,
    ) -> Self {
        BlockCursor {
            rows: RowCursor::from_span(span, count, stride),
            next,
            owner,
            block,
        }
    }

    pub const fn owner(&self) -> BlockOwner {
        self.owner
    }

    pub const fn block(&self) -> DeclaredBlock {
        self.block
    }

    pub const fn remaining(&self) -> usize {
        self.rows.remaining()
    }

    pub const fn offset(&self) -> usize {
        self.rows.offset()
    }

    pub fn next(&mut self) -> RowStep {
        self.rows.next()
    }

    pub fn into_end(self) -> Result<BlockEnd<N>, Violation> {
        let remaining = self.rows.remaining();
        if remaining != 0 {
            return Err(Violation::caller(
                self.rows.offset(),
                CallerDefect::BlockNotConsumed {
                    owner: self.owner,
                    block: self.block,
                    remaining,
                },
            ));
        }
        Ok(BlockEnd {
            at: self.rows.offset(),
            next: self.next,
        })
    }

    pub fn discard_all(mut self) -> BlockEnd<N> {
        self.rows.discard_all();
        BlockEnd {
            at: self.rows.offset(),
            next: self.next,
        }
    }
}

// =============================================================================
// Base blocks
// =============================================================================

#[derive(Debug)]
#[must_use]
pub struct BaseCursor {
    layout: BaseLayout,
}

#[derive(Debug)]
#[must_use]
pub struct BaseLinesState {
    layout: BaseLayout,
}

#[derive(Debug)]
#[must_use]
pub struct BaseConstructionState {
    layout: BaseLayout,
}

/// Value-only iterators over an already-delimited base layout.  These do not
/// decide boundaries: [`BaseCursor`] has already done that.  They exist for
/// callers whose diagnostic order requires walking trailers before applying
/// base-row semantics.
pub struct StockpileRows<'a> {
    payload: &'a [u8],
    rows: RowCursor,
}

pub struct LineRows<'a> {
    payload: &'a [u8],
    rows: RowCursor,
}

pub struct ConstructionRows<'a> {
    payload: &'a [u8],
    rows: RowCursor,
}

pub struct PcvVariantRows<'a> {
    payload: &'a [u8],
    rows: RowCursor,
}

pub struct PcvReservationRows<'a> {
    payload: &'a [u8],
    rows: RowCursor,
}

// Written out rather than generated by a `macro_rules!`: a leaf that defines
// one is unsealable, because the code that runs expands somewhere the leaf's
// attested digest does not cover (`verify_leaf_is_sealed`).

impl Iterator for StockpileRows<'_> {
    type Item = Result<(usize, crate::StockpileRow), crate::Violation>;

    fn next(&mut self) -> Option<Self::Item> {
        let RowStep::Row { index, at } = self.rows.next() else {
            return None;
        };
        Some(crate::stockpile_row_at(self.payload, at).map(|row| (index, row)))
    }
}

impl Iterator for LineRows<'_> {
    type Item = Result<(usize, crate::LineRow), crate::Violation>;

    fn next(&mut self) -> Option<Self::Item> {
        let RowStep::Row { index, at } = self.rows.next() else {
            return None;
        };
        Some(crate::line_row_at(self.payload, at).map(|row| (index, row)))
    }
}

impl Iterator for ConstructionRows<'_> {
    type Item = Result<(usize, crate::ConstructionRow), crate::Violation>;

    fn next(&mut self) -> Option<Self::Item> {
        let RowStep::Row { index, at } = self.rows.next() else {
            return None;
        };
        Some(crate::construction_row_at(self.payload, at).map(|row| (index, row)))
    }
}

impl Iterator for PcvVariantRows<'_> {
    type Item = Result<(usize, crate::PcvVariantRow), crate::Violation>;

    fn next(&mut self) -> Option<Self::Item> {
        let RowStep::Row { index, at } = self.rows.next() else {
            return None;
        };
        Some(crate::pcv_variant_row_at(self.payload, at).map(|row| (index, row)))
    }
}

impl Iterator for PcvReservationRows<'_> {
    type Item = Result<(usize, crate::PcvReservationRow), crate::Violation>;

    fn next(&mut self) -> Option<Self::Item> {
        let RowStep::Row { index, at } = self.rows.next() else {
            return None;
        };
        Some(crate::pcv_reservation_row_at(self.payload, at).map(|row| (index, row)))
    }
}


impl BaseLayout {
    pub fn stockpile_rows<'a>(&self, payload: &'a [u8]) -> StockpileRows<'a> {
        StockpileRows {
            payload,
            rows: RowCursor::from_span(
                self.stockpiles,
                self.stockpiles.len / crate::STOCKPILE_ROW_LEN,
                crate::STOCKPILE_ROW_LEN,
            ),
        }
    }

    pub fn line_rows<'a>(&self, payload: &'a [u8]) -> LineRows<'a> {
        LineRows {
            payload,
            rows: RowCursor::from_span(
                self.lines,
                self.lines.len / crate::LINE_ROW_LEN,
                crate::LINE_ROW_LEN,
            ),
        }
    }

    pub fn construction_rows<'a>(&self, payload: &'a [u8]) -> ConstructionRows<'a> {
        ConstructionRows {
            payload,
            rows: RowCursor::from_span(
                self.construction,
                self.construction.len / crate::CONSTRUCTION_ROW_LEN,
                crate::CONSTRUCTION_ROW_LEN,
            ),
        }
    }
}

impl crate::PcvHeader {
    pub fn variant_rows<'a>(&self, payload: &'a [u8]) -> PcvVariantRows<'a> {
        PcvVariantRows {
            payload,
            rows: RowCursor::from_span(
                self.variants,
                self.variant_count,
                crate::PCV_VARIANT_ROW_LEN,
            ),
        }
    }

    pub fn reservation_rows<'a>(&self, payload: &'a [u8]) -> PcvReservationRows<'a> {
        PcvReservationRows {
            payload,
            rows: RowCursor::from_span(
                self.reservations,
                self.reservation_count,
                crate::PCV_RESERVATION_ROW_LEN,
            ),
        }
    }
}

impl BaseCursor {
    pub fn new(payload: &[u8], header: &BaseHeader) -> Result<Self, Violation> {
        Ok(BaseCursor {
            layout: crate::base_layout(payload, header)?,
        })
    }

    pub const fn layout(&self) -> &BaseLayout {
        &self.layout
    }

    pub fn stockpiles(self) -> BlockCursor<BaseLinesState> {
        let layout = self.layout;
        BlockCursor::new(
            layout.stockpiles,
            layout.stockpiles.len / crate::STOCKPILE_ROW_LEN,
            crate::STOCKPILE_ROW_LEN,
            BaseLinesState { layout },
            BlockOwner::Base,
            DeclaredBlock::Stockpiles,
        )
    }
}

impl BaseLinesState {
    pub fn lines(self) -> BlockCursor<BaseConstructionState> {
        let layout = self.layout;
        BlockCursor::new(
            layout.lines,
            layout.lines.len / crate::LINE_ROW_LEN,
            crate::LINE_ROW_LEN,
            BaseConstructionState { layout },
            BlockOwner::Base,
            DeclaredBlock::Lines,
        )
    }
}

impl BaseConstructionState {
    pub fn construction(self) -> BlockCursor<crate::TrailerCursor> {
        let layout = self.layout;
        BlockCursor::new(
            layout.construction,
            layout.construction.len / crate::CONSTRUCTION_ROW_LEN,
            crate::CONSTRUCTION_ROW_LEN,
            crate::TrailerCursor::new(layout.base_len),
            BlockOwner::Base,
            DeclaredBlock::Construction,
        )
    }
}

// =============================================================================
// Trailer continuation and family cursors
// =============================================================================

#[derive(Debug)]
#[must_use]
pub struct TrailerResume {
    cursor: crate::TrailerCursor,
    trailer: Trailer,
    end: usize,
}

impl TrailerResume {
    fn new(cursor: crate::TrailerCursor, trailer: Trailer, end: usize) -> Self {
        TrailerResume {
            cursor,
            trailer,
            end,
        }
    }

    pub const fn trailer(&self) -> Trailer {
        self.trailer
    }

    pub const fn end(&self) -> usize {
        self.end
    }

    pub fn resume(self) -> crate::TrailerCursor {
        let TrailerResume {
            mut cursor,
            trailer,
            end,
        } = self;
        cursor.advance_past(trailer, end);
        cursor
    }
}

#[derive(Debug)]
#[must_use]
pub enum TrailerBody {
    Plic(PlicCursor),
    Pvj1(FixedCursor),
    Pfr1(FixedCursor),
    Ppr1(FixedCursor),
    Pls1(FixedCursor),
    Pef1(FixedCursor),
    Pmi1(FixedCursor),
    Pcp1(FixedCursor),
    Pcv1(PcvCursor),
    Pdq1(PdqCursor),
}

impl TrailerBody {
    pub const fn trailer(&self) -> Trailer {
        match self {
            TrailerBody::Plic(_) => Trailer::Plic,
            TrailerBody::Pvj1(_) => Trailer::Pvj1,
            TrailerBody::Pfr1(_) => Trailer::Pfr1,
            TrailerBody::Ppr1(_) => Trailer::Ppr1,
            TrailerBody::Pls1(_) => Trailer::Pls1,
            TrailerBody::Pef1(_) => Trailer::Pef1,
            TrailerBody::Pmi1(_) => Trailer::Pmi1,
            TrailerBody::Pcp1(_) => Trailer::Pcp1,
            TrailerBody::Pcv1(_) => Trailer::Pcv1,
            TrailerBody::Pdq1(_) => Trailer::Pdq1,
        }
    }
}

#[derive(Debug)]
#[must_use]
pub struct FixedCursor {
    header: FixedRows,
    parent: crate::TrailerCursor,
}

impl FixedCursor {
    pub(crate) fn new(header: FixedRows, parent: crate::TrailerCursor) -> Self {
        FixedCursor { header, parent }
    }

    pub const fn header(&self) -> &FixedRows {
        &self.header
    }

    pub fn rows(self) -> BlockCursor<TrailerResume> {
        let FixedCursor { header, parent } = self;
        let resume = TrailerResume::new(parent, header.trailer, header.extent.end);
        BlockCursor::new(
            header.rows,
            header.row_count,
            crate::fixed_row_len(header.trailer),
            resume,
            BlockOwner::Trailer(header.trailer),
            DeclaredBlock::Rows,
        )
    }
}

#[derive(Debug)]
#[must_use]
pub struct PlicCursor {
    header: PlicHeader,
    parent: crate::TrailerCursor,
}

#[derive(Debug)]
#[must_use]
pub struct PlicItemsState {
    header: PlicHeader,
    parent: crate::TrailerCursor,
}

impl PlicCursor {
    pub(crate) fn new(header: PlicHeader, parent: crate::TrailerCursor) -> Self {
        PlicCursor { header, parent }
    }

    pub const fn header(&self) -> &PlicHeader {
        &self.header
    }

    pub fn countries(self) -> BlockCursor<PlicItemsState> {
        let PlicCursor { header, parent } = self;
        BlockCursor::new(
            header.countries,
            header.country_count,
            crate::PLIC_COUNTRY_ROW_LEN,
            PlicItemsState { header, parent },
            BlockOwner::Trailer(Trailer::Plic),
            DeclaredBlock::Countries,
        )
    }
}

impl PlicItemsState {
    pub const fn header(&self) -> &PlicHeader {
        &self.header
    }

    pub fn items(self) -> BlockCursor<TrailerResume> {
        let PlicItemsState { header, parent } = self;
        let resume = TrailerResume::new(parent, Trailer::Plic, header.extent.end);
        BlockCursor::new(
            header.items,
            header.item_count,
            header.item_len,
            resume,
            BlockOwner::Trailer(Trailer::Plic),
            DeclaredBlock::Items,
        )
    }
}

// =============================================================================
// PCV1 recoverable canonical ranges
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvRangeStatus {
    Canonical {
        start: u32,
        len: u32,
        end: u32,
    },
    NonCanonical {
        expected_start: u32,
        declared_start: u32,
        len: u32,
        end: u32,
    },
    Overflow {
        expected_start: u32,
        declared_start: u32,
        len: u32,
    },
    OutOfBounds {
        expected_start: u32,
        declared_start: u32,
        len: u32,
        end: u32,
        declared_count: u32,
    },
    PriorOverflow {
        declared_start: u32,
        len: u32,
    },
}

impl PcvRangeStatus {
    pub const fn is_canonical(self) -> bool {
        match self {
            PcvRangeStatus::Canonical { .. } => true,
            PcvRangeStatus::NonCanonical { .. }
            | PcvRangeStatus::Overflow { .. }
            | PcvRangeStatus::OutOfBounds { .. }
            | PcvRangeStatus::PriorOverflow { .. } => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvCountryRanges {
    pub variants: PcvRangeStatus,
    pub reservations: PcvRangeStatus,
}

impl PcvCountryRanges {
    pub const fn is_canonical(self) -> bool {
        self.variants.is_canonical() && self.reservations.is_canonical()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvFinalCoverage {
    pub covered_variants: Option<u32>,
    pub declared_variants: u32,
    pub covered_reservations: Option<u32>,
    pub declared_reservations: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvCountryStep {
    Row {
        index: usize,
        at: usize,
        row: PcvCountryRow,
        ranges: PcvCountryRanges,
    },
    RecoverableRow {
        index: usize,
        at: usize,
        row: PcvCountryRow,
        ranges: PcvCountryRanges,
    },
    RecoverableFinal {
        at: usize,
        defect: PcvFinalCoverage,
    },
    End(usize),
}

#[derive(Debug)]
#[must_use]
pub enum PcvCountryEnd {
    Complete(BlockEnd<PcvVariantsState>),
    Recoverable {
        at: usize,
        defect: PcvFinalCoverage,
        end: BlockEnd<PcvVariantsState>,
    },
}

#[derive(Debug)]
#[must_use]
pub struct PcvCursor {
    header: PcvHeader,
    parent: crate::TrailerCursor,
}

#[derive(Debug)]
#[must_use]
pub struct PcvVariantsState {
    header: PcvHeader,
    parent: crate::TrailerCursor,
}

#[derive(Debug)]
#[must_use]
pub struct PcvReservationsState {
    header: PcvHeader,
    parent: crate::TrailerCursor,
}

#[derive(Debug)]
#[must_use]
pub struct PcvCountryCursor {
    block: BlockCursor<PcvVariantsState>,
    next_variant: Option<u32>,
    next_reservation: Option<u32>,
    final_reported: bool,
}

fn range_status(
    expected: Option<u32>,
    declared_count: u32,
    start: u32,
    len: u32,
) -> (PcvRangeStatus, Option<u32>) {
    let Some(expected_start) = expected else {
        return (
            PcvRangeStatus::PriorOverflow {
                declared_start: start,
                len,
            },
            None,
        );
    };
    let Some(next_expected) = expected_start.checked_add(len) else {
        return (
            PcvRangeStatus::Overflow {
                expected_start,
                declared_start: start,
                len,
            },
            None,
        );
    };
    let Some(end) = start.checked_add(len) else {
        return (
            PcvRangeStatus::Overflow {
                expected_start,
                declared_start: start,
                len,
            },
            Some(next_expected),
        );
    };
    if end > declared_count {
        return (
            PcvRangeStatus::OutOfBounds {
                expected_start,
                declared_start: start,
                len,
                end,
                declared_count,
            },
            Some(next_expected),
        );
    }
    if start != expected_start {
        return (
            PcvRangeStatus::NonCanonical {
                expected_start,
                declared_start: start,
                len,
                end,
            },
            Some(next_expected),
        );
    }
    (
        PcvRangeStatus::Canonical { start, len, end },
        Some(next_expected),
    )
}

impl PcvCursor {
    pub(crate) fn new(header: PcvHeader, parent: crate::TrailerCursor) -> Self {
        PcvCursor { header, parent }
    }

    pub const fn header(&self) -> &PcvHeader {
        &self.header
    }

    pub fn countries(self) -> PcvCountryCursor {
        let PcvCursor { header, parent } = self;
        let block = BlockCursor::new(
            header.countries,
            header.country_count,
            crate::PCV_COUNTRY_ROW_LEN,
            PcvVariantsState { header, parent },
            BlockOwner::Trailer(Trailer::Pcv1),
            DeclaredBlock::Countries,
        );
        PcvCountryCursor {
            block,
            next_variant: Some(0),
            next_reservation: Some(0),
            final_reported: false,
        }
    }
}

impl PcvCountryCursor {
    pub const fn remaining(&self) -> usize {
        self.block.remaining()
    }

    pub const fn offset(&self) -> usize {
        self.block.offset()
    }

    pub fn next(&mut self, payload: &[u8]) -> Result<PcvCountryStep, Violation> {
        match self.block.rows.peek() {
            RowStep::Row { index, at } => {
                let row = pcv_country_row_at(payload, at)?;
                let variant_count = self.block.next.header.variant_count as u32;
                let reservation_count = self.block.next.header.reservation_count as u32;
                let (variants, next_variant) = range_status(
                    self.next_variant,
                    variant_count,
                    row.variant_start,
                    row.variant_len,
                );
                let (reservations, next_reservation) = range_status(
                    self.next_reservation,
                    reservation_count,
                    row.reservation_start,
                    row.reservation_len,
                );
                self.next_variant = next_variant;
                self.next_reservation = next_reservation;
                self.block.rows.advance();
                let ranges = PcvCountryRanges {
                    variants,
                    reservations,
                };
                if ranges.is_canonical() {
                    Ok(PcvCountryStep::Row {
                        index,
                        at,
                        row,
                        ranges,
                    })
                } else {
                    Ok(PcvCountryStep::RecoverableRow {
                        index,
                        at,
                        row,
                        ranges,
                    })
                }
            }
            RowStep::End(at) => {
                if !self.final_reported {
                    let defect = self.final_coverage();
                    if let Some(defect) = defect {
                        self.final_reported = true;
                        return Ok(PcvCountryStep::RecoverableFinal { at, defect });
                    }
                }
                Ok(PcvCountryStep::End(at))
            }
        }
    }

    fn final_coverage(&self) -> Option<PcvFinalCoverage> {
        let expected_variants = self.block.next.header.variant_count as u32;
        let expected_reservations = self.block.next.header.reservation_count as u32;
        if self.next_variant != Some(expected_variants)
            || self.next_reservation != Some(expected_reservations)
        {
            Some(PcvFinalCoverage {
                covered_variants: self.next_variant,
                declared_variants: expected_variants,
                covered_reservations: self.next_reservation,
                declared_reservations: expected_reservations,
            })
        } else {
            None
        }
    }

    pub fn into_end(self) -> Result<PcvCountryEnd, Violation> {
        let defect = if self.final_reported {
            None
        } else {
            self.final_coverage()
        };
        let at = self.block.offset();
        let end = self.block.into_end()?;
        match defect {
            Some(defect) => Ok(PcvCountryEnd::Recoverable { at, defect, end }),
            None => Ok(PcvCountryEnd::Complete(end)),
        }
    }
}

impl PcvVariantsState {
    pub const fn header(&self) -> &PcvHeader {
        &self.header
    }

    pub fn variants(self) -> BlockCursor<PcvReservationsState> {
        let PcvVariantsState { header, parent } = self;
        BlockCursor::new(
            header.variants,
            header.variant_count,
            crate::PCV_VARIANT_ROW_LEN,
            PcvReservationsState { header, parent },
            BlockOwner::Trailer(Trailer::Pcv1),
            DeclaredBlock::Variants,
        )
    }
}

impl PcvReservationsState {
    pub const fn header(&self) -> &PcvHeader {
        &self.header
    }

    pub fn reservations(self) -> BlockCursor<TrailerResume> {
        let PcvReservationsState { header, parent } = self;
        let resume = TrailerResume::new(parent, Trailer::Pcv1, header.extent.end);
        BlockCursor::new(
            header.reservations,
            header.reservation_count,
            crate::PCV_RESERVATION_ROW_LEN,
            resume,
            BlockOwner::Trailer(Trailer::Pcv1),
            DeclaredBlock::Reservations,
        )
    }
}

// =============================================================================
// PDQ1 opaque inner span
// =============================================================================

#[derive(Debug)]
#[must_use]
pub struct PdqCursor {
    header: PdqHeader,
    parent: crate::TrailerCursor,
}

impl PdqCursor {
    pub(crate) fn new(header: PdqHeader, parent: crate::TrailerCursor) -> Self {
        PdqCursor { header, parent }
    }

    pub const fn header(&self) -> &PdqHeader {
        &self.header
    }

    pub const fn inner_payload_span(&self) -> Span {
        self.header.inner_payload_span()
    }

    pub fn inner_payload(self) -> BlockCursor<TrailerResume> {
        let PdqCursor { header, parent } = self;
        let span = header.inner_payload_span();
        let resume = TrailerResume::new(parent, Trailer::Pdq1, header.extent.end);
        BlockCursor::new(
            span,
            1,
            span.len,
            resume,
            BlockOwner::Trailer(Trailer::Pdq1),
            DeclaredBlock::InnerPayload,
        )
    }
}
