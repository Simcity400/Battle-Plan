//! Domain 11 (`production_live`) payload grammar -- one structural walk and
//! one encoder for every site.
//!
//! # Why this crate exists
//!
//! Five shipping sites wrote or walked the same 40-byte base payload and ten
//! additive trailer families. The layout constants were already shared, but
//! the walks were not. A new trailer could therefore be added at one site
//! without making the other four fail to compile. This dependency-free leaf is
//! the common drift boundary: every family is a public exhaustive enum arm,
//! every declared block is an ownership transition, and the encoder lives next
//! to the only structural walk.
//!
//! # Grammar here; semantics at the sites
//!
//! This crate owns byte widths, field offsets, versions, counts that determine
//! boundaries, checked extents, PLIC's country-to-item partition, PCV1's
//! canonical ranges, duplicate trailer detection, and exact block consumption.
//! It does not own tag or id ranges, line `kind` values, the fixed-point scale
//! policy, construction's reserved-word policy, cross-family bijections, exact
//! totals, JSON, counters, or diagnostic strings. Those rules do not decide a
//! byte boundary and the sites do not agree about all of them.
//!
//! # Traversal is lazy and owned
//!
//! The readers interleave semantics with structural checks and therefore care
//! which diagnostic is observed first. [`TrailerCursor::next`] names a magic
//! without reading behind it; [`TrailerCursor::peek_header`] exposes header
//! operands without applying a family rule; [`TrailerCursor::read_body`]
//! consumes the parent. The parent returns only through the final
//! [`TrailerResume`] after every declared block was walked or explicitly
//! discarded. A whole-payload eager layout would change diagnostic order, and
//! advancing the parent at header time would make a forgotten block invisible.
//!
//! # Violations are data
//!
//! [`Violation`] carries operands, never text. Each site owns one exhaustive
//! formatter and decides whether a recoverable PCV1 range defect is fatal.
//! Caller defects are a separate [`ViolationKind`] arm so a broken consumer is
//! never presented as a malformed recording.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod deployment;
pub mod encode;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod deployment_tests;

// =============================================================================
// Shared scalar widths and spans
// =============================================================================

/// A field's exact wire width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Width {
    Bytes4,
    U16,
    U32,
    I32,
    U64,
    I64,
}

impl Width {
    pub const fn bytes(self) -> usize {
        match self {
            Width::U16 => 2,
            Width::Bytes4 | Width::U32 | Width::I32 => 4,
            Width::U64 | Width::I64 => 8,
        }
    }
}

/// A checked half-open byte span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Span {
    pub start: usize,
    pub len: usize,
    pub end: usize,
}

impl Span {
    const fn new(start: usize, len: usize, end: usize) -> Self {
        Span { start, len, end }
    }
}

// =============================================================================
// Base field tables
// =============================================================================

/// Canonical fixed-point scale written in the D11 base header.
pub const PRODUCTION_SCALE_RAW: u32 = 100_000;
pub const BASE_HEADER_LEN: usize = 40;
pub const STOCKPILE_ROW_LEN: usize = 16;
pub const LINE_ROW_LEN: usize = 32;
pub const CONSTRUCTION_ROW_LEN: usize = 32;

/// A field of the 40-byte base header.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum BaseHeaderField {
    ElapsedWallMs,
    CountryCount,
    ChangedRows,
    StatusFlags,
    ScaleRaw,
    StockpileCount,
    MilitaryCount,
    NavalCount,
    ConstructionCount,
}

impl BaseHeaderField {
    pub const ALL: [BaseHeaderField; 9] = [
        BaseHeaderField::ElapsedWallMs,
        BaseHeaderField::CountryCount,
        BaseHeaderField::ChangedRows,
        BaseHeaderField::StatusFlags,
        BaseHeaderField::ScaleRaw,
        BaseHeaderField::StockpileCount,
        BaseHeaderField::MilitaryCount,
        BaseHeaderField::NavalCount,
        BaseHeaderField::ConstructionCount,
    ];

    /// The four fields which structurally size the base blocks.
    pub const STRUCTURAL_COUNTS: [BaseHeaderField; 4] = [
        BaseHeaderField::StockpileCount,
        BaseHeaderField::MilitaryCount,
        BaseHeaderField::NavalCount,
        BaseHeaderField::ConstructionCount,
    ];

    /// Fields that do not move a base-block boundary.
    pub const SEMANTIC: [BaseHeaderField; 5] = [
        BaseHeaderField::ElapsedWallMs,
        BaseHeaderField::CountryCount,
        BaseHeaderField::ChangedRows,
        BaseHeaderField::StatusFlags,
        BaseHeaderField::ScaleRaw,
    ];

    pub const fn offset(self) -> usize {
        match self {
            BaseHeaderField::ElapsedWallMs => 0,
            BaseHeaderField::CountryCount => 8,
            BaseHeaderField::ChangedRows => 12,
            BaseHeaderField::StatusFlags => 16,
            BaseHeaderField::ScaleRaw => 20,
            BaseHeaderField::StockpileCount => 24,
            BaseHeaderField::MilitaryCount => 28,
            BaseHeaderField::NavalCount => 32,
            BaseHeaderField::ConstructionCount => 36,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            BaseHeaderField::ElapsedWallMs => Width::U64,
            BaseHeaderField::CountryCount
            | BaseHeaderField::ChangedRows
            | BaseHeaderField::StatusFlags
            | BaseHeaderField::ScaleRaw
            | BaseHeaderField::StockpileCount
            | BaseHeaderField::MilitaryCount
            | BaseHeaderField::NavalCount
            | BaseHeaderField::ConstructionCount => Width::U32,
        }
    }
}

/// A field of one stockpile row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StockpileField {
    TagDbId,
    VariantId,
    AmountRaw,
}

impl StockpileField {
    pub const ALL: [StockpileField; 3] = [
        StockpileField::TagDbId,
        StockpileField::VariantId,
        StockpileField::AmountRaw,
    ];

    pub const fn offset(self) -> usize {
        match self {
            StockpileField::TagDbId => 0,
            StockpileField::VariantId => 4,
            StockpileField::AmountRaw => 8,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            StockpileField::TagDbId | StockpileField::VariantId => Width::U32,
            StockpileField::AmountRaw => Width::I64,
        }
    }
}

/// A field of one military or naval line row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineField {
    TagDbId,
    Kind,
    LineId,
    ActiveFactories,
    CostRaw,
    SpeedRaw,
    ProducedRaw,
    Amount,
}

impl LineField {
    pub const ALL: [LineField; 8] = [
        LineField::TagDbId,
        LineField::Kind,
        LineField::LineId,
        LineField::ActiveFactories,
        LineField::CostRaw,
        LineField::SpeedRaw,
        LineField::ProducedRaw,
        LineField::Amount,
    ];

    pub const LINE_KEY: [LineField; 3] = [LineField::TagDbId, LineField::Kind, LineField::LineId];
    pub const VALUES: [LineField; 5] = [
        LineField::ActiveFactories,
        LineField::CostRaw,
        LineField::SpeedRaw,
        LineField::ProducedRaw,
        LineField::Amount,
    ];

    pub const fn offset(self) -> usize {
        match self {
            LineField::TagDbId => 0,
            LineField::Kind => 4,
            LineField::LineId => 8,
            LineField::ActiveFactories => 12,
            LineField::CostRaw => 16,
            LineField::SpeedRaw => 20,
            LineField::ProducedRaw => 24,
            LineField::Amount => 28,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            LineField::TagDbId | LineField::Kind => Width::U32,
            LineField::LineId
            | LineField::ActiveFactories
            | LineField::CostRaw
            | LineField::SpeedRaw
            | LineField::ProducedRaw
            | LineField::Amount => Width::I32,
        }
    }
}

/// A field of one construction row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstructionField {
    TagDbId,
    LineId,
    TemplateId,
    ActiveFactories,
    CostRaw,
    ProducedRaw,
    Priority,
    Reserved,
}

impl ConstructionField {
    pub const ALL: [ConstructionField; 8] = [
        ConstructionField::TagDbId,
        ConstructionField::LineId,
        ConstructionField::TemplateId,
        ConstructionField::ActiveFactories,
        ConstructionField::CostRaw,
        ConstructionField::ProducedRaw,
        ConstructionField::Priority,
        ConstructionField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            ConstructionField::TagDbId => 0,
            ConstructionField::LineId => 4,
            ConstructionField::TemplateId => 8,
            ConstructionField::ActiveFactories => 12,
            ConstructionField::CostRaw => 16,
            ConstructionField::ProducedRaw => 20,
            ConstructionField::Priority => 24,
            ConstructionField::Reserved => 28,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            ConstructionField::TagDbId
            | ConstructionField::LineId
            | ConstructionField::TemplateId
            | ConstructionField::Priority
            | ConstructionField::Reserved => Width::U32,
            ConstructionField::ActiveFactories
            | ConstructionField::CostRaw
            | ConstructionField::ProducedRaw => Width::I32,
        }
    }
}

const fn tiles(offsets: &[usize], widths: &[Width], len: usize) -> bool {
    let mut expected = 0;
    let mut i = 0;
    while i < offsets.len() {
        if offsets[i] != expected {
            return false;
        }
        expected += widths[i].bytes();
        i += 1;
    }
    expected == len
}

const _: () = {
    let mut offsets = [0usize; 9];
    let mut widths = [Width::U32; 9];
    let mut i = 0;
    while i < BaseHeaderField::ALL.len() {
        offsets[i] = BaseHeaderField::ALL[i].offset();
        widths[i] = BaseHeaderField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, BASE_HEADER_LEN) {
        panic!("d11_grammar: BaseHeaderField::ALL does not tile the base header");
    }
    let mut seen = 0u16;
    let mut i = 0;
    while i < BaseHeaderField::STRUCTURAL_COUNTS.len() {
        seen |= 1 << (BaseHeaderField::STRUCTURAL_COUNTS[i] as u8);
        i += 1;
    }
    let mut i = 0;
    while i < BaseHeaderField::SEMANTIC.len() {
        let bit = 1 << (BaseHeaderField::SEMANTIC[i] as u8);
        if seen & bit != 0 {
            panic!("d11_grammar: base-header sub-lists overlap");
        }
        seen |= bit;
        i += 1;
    }
    if seen != (1u16 << BaseHeaderField::ALL.len()) - 1 {
        panic!("d11_grammar: base-header sub-lists do not partition ALL");
    }
};

const _: () = {
    let mut offsets = [0usize; 3];
    let mut widths = [Width::U32; 3];
    let mut i = 0;
    while i < StockpileField::ALL.len() {
        offsets[i] = StockpileField::ALL[i].offset();
        widths[i] = StockpileField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, STOCKPILE_ROW_LEN) {
        panic!("d11_grammar: StockpileField::ALL does not tile its row");
    }
};

const _: () = {
    let mut offsets = [0usize; 8];
    let mut widths = [Width::U32; 8];
    let mut i = 0;
    while i < LineField::ALL.len() {
        offsets[i] = LineField::ALL[i].offset();
        widths[i] = LineField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, LINE_ROW_LEN) {
        panic!("d11_grammar: LineField::ALL does not tile its row");
    }
    let mut seen = 0u16;
    let mut i = 0;
    while i < LineField::LINE_KEY.len() {
        seen |= 1 << (LineField::LINE_KEY[i] as u8);
        i += 1;
    }
    let mut i = 0;
    while i < LineField::VALUES.len() {
        let bit = 1 << (LineField::VALUES[i] as u8);
        if seen & bit != 0 {
            panic!("d11_grammar: line sub-lists overlap");
        }
        seen |= bit;
        i += 1;
    }
    if seen != (1u16 << LineField::ALL.len()) - 1 {
        panic!("d11_grammar: line sub-lists do not partition ALL");
    }
};

const _: () = {
    let mut offsets = [0usize; 8];
    let mut widths = [Width::U32; 8];
    let mut i = 0;
    while i < ConstructionField::ALL.len() {
        offsets[i] = ConstructionField::ALL[i].offset();
        widths[i] = ConstructionField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, CONSTRUCTION_ROW_LEN) {
        panic!("d11_grammar: ConstructionField::ALL does not tile its row");
    }
};

// =============================================================================
// Violations and checked arithmetic
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    BaseHeader,
    BaseStockpiles,
    BaseLines,
    BaseConstruction,
    TrailerMagic,
    TrailerHeader(Trailer),
    TrailerBody(Trailer),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArithmeticOp {
    Add,
    Subtract,
    Multiply,
    U32Conversion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    UnknownTrailerMagic {
        at: usize,
        magic: [u8; 4],
    },
    DuplicateTrailer {
        trailer: Trailer,
        at: usize,
    },
    UnknownVersion {
        trailer: Trailer,
        version: u16,
    },
    HeaderLen {
        trailer: Trailer,
        declared: usize,
        expected: usize,
    },
    ReservedWord {
        trailer: Trailer,
        at: usize,
        value: u32,
    },
    DeclaredExtent {
        trailer: Trailer,
        declared_len: usize,
        computed_len: usize,
    },
    PlicItemCount {
        declared_items: usize,
        summed_items: usize,
    },
    ArithmeticOverflow {
        section: Section,
        operation: ArithmeticOp,
        left: usize,
        right: usize,
    },
    CallerDefect(CallerDefect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Violation {
    pub at: usize,
    pub kind: ViolationKind,
}

impl Violation {
    const fn new(at: usize, kind: ViolationKind) -> Self {
        Violation { at, kind }
    }

    pub(crate) const fn caller(at: usize, defect: CallerDefect) -> Self {
        Violation::new(at, ViolationKind::CallerDefect(defect))
    }
}

pub(crate) fn checked_add(
    section: Section,
    at: usize,
    left: usize,
    right: usize,
) -> Result<usize, Violation> {
    left.checked_add(right).ok_or_else(|| {
        Violation::new(
            at,
            ViolationKind::ArithmeticOverflow {
                section,
                operation: ArithmeticOp::Add,
                left,
                right,
            },
        )
    })
}

pub(crate) fn checked_mul(
    section: Section,
    at: usize,
    left: usize,
    right: usize,
) -> Result<usize, Violation> {
    left.checked_mul(right).ok_or_else(|| {
        Violation::new(
            at,
            ViolationKind::ArithmeticOverflow {
                section,
                operation: ArithmeticOp::Multiply,
                left,
                right,
            },
        )
    })
}

pub(crate) fn checked_sub(
    section: Section,
    at: usize,
    left: usize,
    right: usize,
) -> Result<usize, Violation> {
    left.checked_sub(right).ok_or_else(|| {
        Violation::new(
            at,
            ViolationKind::ArithmeticOverflow {
                section,
                operation: ArithmeticOp::Subtract,
                left,
                right,
            },
        )
    })
}

pub(crate) fn checked_span(
    section: Section,
    at: usize,
    start: usize,
    count: usize,
    stride: usize,
) -> Result<Span, Violation> {
    let len = checked_mul(section, at, count, stride)?;
    let end = checked_add(section, at, start, len)?;
    Ok(Span::new(start, len, end))
}

pub(crate) fn need(
    payload_len: usize,
    at: usize,
    section: Section,
    end: usize,
) -> Result<(), Violation> {
    if payload_len < end {
        return Err(Violation::new(
            at,
            ViolationKind::Need {
                section,
                needed: end,
                available: payload_len,
            },
        ));
    }
    Ok(())
}

pub(crate) fn need_row(
    payload: &[u8],
    at: usize,
    section: Section,
    row_len: usize,
) -> Result<(), Violation> {
    let end = checked_add(section, at, at, row_len)?;
    need(payload.len(), at, section, end)
}

// =============================================================================
// Checked byte readers (all public paths bounds-check first)
// =============================================================================

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}

pub(crate) fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

pub(crate) fn i64_at(bytes: &[u8], at: usize) -> i64 {
    u64_at(bytes, at) as i64
}

// =============================================================================
// The base payload
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BaseHeader {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub scale_raw: u32,
    pub stockpile_count: u32,
    pub military_count: u32,
    pub naval_count: u32,
    pub construction_count: u32,
}

pub fn read_base_header(payload: &[u8]) -> Result<BaseHeader, Violation> {
    need(payload.len(), 0, Section::BaseHeader, BASE_HEADER_LEN)?;
    Ok(BaseHeader {
        elapsed_wall_ms: u64_at(payload, BaseHeaderField::ElapsedWallMs.offset()),
        country_count: u32_at(payload, BaseHeaderField::CountryCount.offset()),
        changed_rows: u32_at(payload, BaseHeaderField::ChangedRows.offset()),
        status_flags: u32_at(payload, BaseHeaderField::StatusFlags.offset()),
        scale_raw: u32_at(payload, BaseHeaderField::ScaleRaw.offset()),
        stockpile_count: u32_at(payload, BaseHeaderField::StockpileCount.offset()),
        military_count: u32_at(payload, BaseHeaderField::MilitaryCount.offset()),
        naval_count: u32_at(payload, BaseHeaderField::NavalCount.offset()),
        construction_count: u32_at(payload, BaseHeaderField::ConstructionCount.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BaseLayout {
    pub stockpiles: Span,
    pub lines: Span,
    pub construction: Span,
    pub base_len: usize,
}

/// Compute the three base blocks with checked arithmetic and bounds-check the
/// whole base before any row is yielded.
pub fn derive_base_layout(header: &BaseHeader) -> Result<BaseLayout, Violation> {
    let stockpiles = checked_span(
        Section::BaseStockpiles,
        BASE_HEADER_LEN,
        BASE_HEADER_LEN,
        header.stockpile_count as usize,
        STOCKPILE_ROW_LEN,
    )?;
    let line_count = checked_add(
        Section::BaseLines,
        stockpiles.end,
        header.military_count as usize,
        header.naval_count as usize,
    )?;
    let lines = checked_span(
        Section::BaseLines,
        stockpiles.end,
        stockpiles.end,
        line_count,
        LINE_ROW_LEN,
    )?;
    let construction = checked_span(
        Section::BaseConstruction,
        lines.end,
        lines.end,
        header.construction_count as usize,
        CONSTRUCTION_ROW_LEN,
    )?;
    Ok(BaseLayout {
        stockpiles,
        lines,
        construction,
        base_len: construction.end,
    })
}

pub fn base_layout(payload: &[u8], header: &BaseHeader) -> Result<BaseLayout, Violation> {
    let layout = derive_base_layout(header)?;
    need(
        payload.len(),
        BASE_HEADER_LEN,
        Section::BaseStockpiles,
        layout.stockpiles.end,
    )?;
    need(
        payload.len(),
        layout.stockpiles.end,
        Section::BaseLines,
        layout.lines.end,
    )?;
    need(
        payload.len(),
        layout.lines.end,
        Section::BaseConstruction,
        layout.construction.end,
    )?;
    Ok(layout)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StockpileRow {
    pub tag_db_id: u32,
    pub variant_id: u32,
    pub amount_raw: i64,
}

pub fn stockpile_row_at(payload: &[u8], at: usize) -> Result<StockpileRow, Violation> {
    need_row(payload, at, Section::BaseStockpiles, STOCKPILE_ROW_LEN)?;
    Ok(StockpileRow {
        tag_db_id: u32_at(payload, at + StockpileField::TagDbId.offset()),
        variant_id: u32_at(payload, at + StockpileField::VariantId.offset()),
        amount_raw: i64_at(payload, at + StockpileField::AmountRaw.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineRow {
    pub tag_db_id: u32,
    pub kind: u32,
    pub line_id: i32,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub speed_raw: i32,
    pub produced_raw: i32,
    pub amount: i32,
}

pub fn line_row_at(payload: &[u8], at: usize) -> Result<LineRow, Violation> {
    need_row(payload, at, Section::BaseLines, LINE_ROW_LEN)?;
    Ok(LineRow {
        tag_db_id: u32_at(payload, at + LineField::TagDbId.offset()),
        kind: u32_at(payload, at + LineField::Kind.offset()),
        line_id: i32_at(payload, at + LineField::LineId.offset()),
        active_factories: i32_at(payload, at + LineField::ActiveFactories.offset()),
        cost_raw: i32_at(payload, at + LineField::CostRaw.offset()),
        speed_raw: i32_at(payload, at + LineField::SpeedRaw.offset()),
        produced_raw: i32_at(payload, at + LineField::ProducedRaw.offset()),
        amount: i32_at(payload, at + LineField::Amount.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConstructionRow {
    pub tag_db_id: u32,
    pub line_id: u32,
    pub template_id: u32,
    pub active_factories: i32,
    pub cost_raw: i32,
    pub produced_raw: i32,
    pub priority: u32,
    pub reserved: u32,
}

pub fn construction_row_at(payload: &[u8], at: usize) -> Result<ConstructionRow, Violation> {
    need_row(payload, at, Section::BaseConstruction, CONSTRUCTION_ROW_LEN)?;
    Ok(ConstructionRow {
        tag_db_id: u32_at(payload, at + ConstructionField::TagDbId.offset()),
        line_id: u32_at(payload, at + ConstructionField::LineId.offset()),
        template_id: u32_at(payload, at + ConstructionField::TemplateId.offset()),
        active_factories: i32_at(payload, at + ConstructionField::ActiveFactories.offset()),
        cost_raw: i32_at(payload, at + ConstructionField::CostRaw.offset()),
        produced_raw: i32_at(payload, at + ConstructionField::ProducedRaw.offset()),
        priority: u32_at(payload, at + ConstructionField::Priority.offset()),
        reserved: u32_at(payload, at + ConstructionField::Reserved.offset()),
    })
}

// =============================================================================
// The ten-family trailer alphabet
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Trailer {
    Plic,
    Pvj1,
    Pfr1,
    Ppr1,
    Pls1,
    Pef1,
    Pmi1,
    Pcp1,
    Pcv1,
    Pdq1,
}

impl Trailer {
    /// Wire order; PCV1 is pinned immediately before final PDQ1.
    pub const ALL: [Trailer; 10] = [
        Trailer::Plic,
        Trailer::Pvj1,
        Trailer::Pfr1,
        Trailer::Ppr1,
        Trailer::Pls1,
        Trailer::Pef1,
        Trailer::Pmi1,
        Trailer::Pcp1,
        Trailer::Pcv1,
        Trailer::Pdq1,
    ];

    pub const fn magic(self) -> &'static [u8; 4] {
        match self {
            Trailer::Plic => b"PLIC",
            Trailer::Pvj1 => b"PVJ1",
            Trailer::Pfr1 => b"PFR1",
            Trailer::Ppr1 => b"PPR1",
            Trailer::Pls1 => b"PLS1",
            Trailer::Pef1 => b"PEF1",
            Trailer::Pmi1 => b"PMI1",
            Trailer::Pcp1 => b"PCP1",
            Trailer::Pcv1 => b"PCV1",
            Trailer::Pdq1 => b"PDQ1",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Trailer::Plic => "PLIC",
            Trailer::Pvj1 => "PVJ1",
            Trailer::Pfr1 => "PFR1",
            Trailer::Ppr1 => "PPR1",
            Trailer::Pls1 => "PLS1",
            Trailer::Pef1 => "PEF1",
            Trailer::Pmi1 => "PMI1",
            Trailer::Pcp1 => "PCP1",
            Trailer::Pcv1 => "PCV1",
            Trailer::Pdq1 => "PDQ1",
        }
    }

    pub const fn header_len(self) -> usize {
        match self {
            Trailer::Plic
            | Trailer::Pvj1
            | Trailer::Pfr1
            | Trailer::Ppr1
            | Trailer::Pls1
            | Trailer::Pef1
            | Trailer::Pmi1
            | Trailer::Pcp1
            | Trailer::Pdq1 => 20,
            Trailer::Pcv1 => 28,
        }
    }

    fn from_magic(bytes: &[u8]) -> Option<Trailer> {
        let mut i = 0;
        while i < Trailer::ALL.len() {
            let trailer = Trailer::ALL[i];
            let magic = trailer.magic();
            if bytes[0] == magic[0]
                && bytes[1] == magic[1]
                && bytes[2] == magic[2]
                && bytes[3] == magic[3]
            {
                return Some(trailer);
            }
            i += 1;
        }
        None
    }

    const fn seen_bit(self) -> u16 {
        let mut i = 0;
        while i < Trailer::ALL.len() {
            if Trailer::ALL[i] as u8 == self as u8 {
                return 1 << i;
            }
            i += 1;
        }
        panic!("d11_grammar: Trailer is absent from Trailer::ALL")
    }
}

const _: () = {
    if Trailer::ALL[8] as u8 != Trailer::Pcv1 as u8 || Trailer::ALL[9] as u8 != Trailer::Pdq1 as u8
    {
        panic!("d11_grammar: PCV1/PDQ1 order drifted");
    }
    let mut i = 0;
    while i < Trailer::ALL.len() {
        let a = Trailer::ALL[i].magic();
        let label = Trailer::ALL[i].label().as_bytes();
        if label.len() != 4
            || a[0] != label[0]
            || a[1] != label[1]
            || a[2] != label[2]
            || a[3] != label[3]
        {
            panic!("d11_grammar: trailer label does not spell its magic");
        }
        let mut j = i + 1;
        while j < Trailer::ALL.len() {
            let b = Trailer::ALL[j].magic();
            if a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3] {
                panic!("d11_grammar: duplicate trailer magic");
            }
            j += 1;
        }
        i += 1;
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrailerStep {
    Magic { at: usize, trailer: Trailer },
    End(usize),
}

/// The one parent cursor. Deliberately neither `Clone` nor `Copy`.
#[derive(Debug)]
#[must_use]
pub struct TrailerCursor {
    at: usize,
    pending: Option<(usize, Trailer)>,
    seen: u16,
}

impl TrailerCursor {
    pub(crate) const fn new(at: usize) -> Self {
        TrailerCursor {
            at,
            pending: None,
            seen: 0,
        }
    }

    /// Start a walk over a slice whose first byte is the first trailer magic.
    ///
    /// This is the wire port's boundary: it exposes its historical trailer-only
    /// slice without granting callers an arbitrary continuation offset.
    pub const fn for_trailer_slice() -> Self {
        TrailerCursor::new(0)
    }

    pub const fn offset(&self) -> usize {
        self.at
    }

    pub const fn seen(&self, trailer: Trailer) -> bool {
        self.seen & trailer.seen_bit() != 0
    }

    pub fn next(&mut self, payload: &[u8]) -> Result<TrailerStep, Violation> {
        if let Some((at, trailer)) = self.pending {
            return Ok(TrailerStep::Magic { at, trailer });
        }
        if self.at >= payload.len() {
            return Ok(TrailerStep::End(self.at));
        }
        let end = checked_add(Section::TrailerMagic, self.at, self.at, 4)?;
        need(payload.len(), self.at, Section::TrailerMagic, end)?;
        let magic = [
            payload[self.at],
            payload[self.at + 1],
            payload[self.at + 2],
            payload[self.at + 3],
        ];
        let Some(trailer) = Trailer::from_magic(&magic) else {
            return Err(Violation::new(
                self.at,
                ViolationKind::UnknownTrailerMagic { at: self.at, magic },
            ));
        };
        if self.seen(trailer) {
            return Err(Violation::new(
                self.at,
                ViolationKind::DuplicateTrailer {
                    trailer,
                    at: self.at,
                },
            ));
        }
        self.pending = Some((self.at, trailer));
        Ok(TrailerStep::Magic {
            at: self.at,
            trailer,
        })
    }

    /// Read header operands without applying a family rule or advancing.
    pub fn peek_header(&self, payload: &[u8]) -> Result<HeaderPreview, Violation> {
        let Some((at, trailer)) = self.pending else {
            return Err(Violation::caller(self.at, CallerDefect::BodyNotDelimited));
        };
        family::read_header_preview(payload, at, trailer)
    }

    /// Parse the named family and consume the parent until every block ends.
    ///
    /// The only successful route back is the final [`TrailerResume`].
    ///
    /// ```compile_fail
    /// use d11_grammar::*;
    /// fn use_after_move(mut cursor: TrailerCursor, payload: &[u8]) {
    ///     let _body = cursor.read_body(payload);
    ///     let _ = cursor.next(payload);
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use d11_grammar::*;
    /// fn clone_parent(cursor: TrailerCursor) -> (TrailerCursor, TrailerCursor) {
    ///     let spare = cursor.clone();
    ///     (cursor, spare)
    /// }
    /// ```
    ///
    /// A caller cannot forge a continuation at a published extent either:
    ///
    /// ```compile_fail
    /// use d11_grammar::*;
    /// fn forge_continuation(end: usize) -> TrailerCursor {
    ///     TrailerCursor::new(end)
    /// }
    /// ```
    ///
    /// ```compile_fail
    /// use d11_grammar::*;
    /// fn skip_declared_block(pcv: PcvCursor) -> BlockCursor<TrailerResume> {
    ///     pcv.variants()
    /// }
    /// ```
    pub fn read_body(self, payload: &[u8]) -> Result<TrailerBody, Violation> {
        let at = self.at;
        let body = self.read_body_recoverable(payload)?;
        if let TrailerBody::Plic(cursor) = &body {
            let header = cursor.header();
            if header.summed_item_count != header.item_count {
                return Err(Violation::new(
                    at,
                    ViolationKind::PlicItemCount {
                        declared_items: header.item_count,
                        summed_items: header.summed_item_count,
                    },
                ));
            }
        }
        Ok(body)
    }

    /// Parse a family while leaving caller-policy PLIC item-count mismatch
    /// recoverable after its country rows have been inspected.
    pub fn read_body_recoverable(self, payload: &[u8]) -> Result<TrailerBody, Violation> {
        let Some((at, trailer)) = self.pending else {
            return Err(Violation::caller(self.at, CallerDefect::BodyNotDelimited));
        };
        let parsed = family::read_trailer_headers(payload, at, trailer)?;
        Ok(match parsed {
            family::TrailerHeaders::Plic(header) => {
                TrailerBody::Plic(PlicCursor::new(header, self))
            }
            family::TrailerHeaders::Pvj1(header) => {
                TrailerBody::Pvj1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pfr1(header) => {
                TrailerBody::Pfr1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Ppr1(header) => {
                TrailerBody::Ppr1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pls1(header) => {
                TrailerBody::Pls1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pef1(header) => {
                TrailerBody::Pef1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pmi1(header) => {
                TrailerBody::Pmi1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pcp1(header) => {
                TrailerBody::Pcp1(FixedCursor::new(header, self))
            }
            family::TrailerHeaders::Pcv1(header) => TrailerBody::Pcv1(PcvCursor::new(header, self)),
            family::TrailerHeaders::Pdq1(header) => TrailerBody::Pdq1(PdqCursor::new(header, self)),
        })
    }

    pub(crate) fn advance_past(&mut self, trailer: Trailer, end: usize) {
        self.at = end;
        self.pending = None;
        self.seen |= trailer.seen_bit();
    }
}

mod family;
pub use family::*;

mod walk;
pub use walk::*;
