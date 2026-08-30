//!    the declared counts, with checked arithmetic and one bound check against

use crate::{
    add, i32_at, i64_at, mul, u16_at, u32_at, u64_at, Limits, Section, Trailer, Violation,
    ViolationKind, Width,
};

pub const TRAILER_MAGIC_OFFSET: usize = 0;
pub const TRAILER_VERSION_OFFSET: usize = 4;
pub const TRAILER_HEADER_LEN_OFFSET: usize = 6;
pub const TRAILER_ROW_LEN_OFFSET: usize = 8;
pub const TRAILER_RESERVED0_OFFSET: usize = 10;
pub const TRAILER_ROW_COUNT_OFFSET: usize = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum FlowVersion {
    Legacy,
    Fixed7,
    Variable,
}

impl FlowVersion {
    pub const ALL: [FlowVersion; 3] = [
        FlowVersion::Legacy,
        FlowVersion::Fixed7,
        FlowVersion::Variable,
    ];

    pub const fn wire(self) -> u16 {
        match self {
            FlowVersion::Legacy => 1,
            FlowVersion::Fixed7 => 2,
            FlowVersion::Variable => 3,
        }
    }

    pub const CURRENT: FlowVersion = FlowVersion::Variable;

    pub const fn states_resource_count(self) -> bool {
        match self {
            FlowVersion::Variable => true,
            FlowVersion::Legacy | FlowVersion::Fixed7 => false,
        }
    }

    fn from_wire(version: u16) -> Option<FlowVersion> {
        let mut i = 0;
        while i < FlowVersion::ALL.len() {
            if FlowVersion::ALL[i].wire() == version {
                return Some(FlowVersion::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ResourceRowVersion {
    Legacy,
    CountOnly,
    Identity,
    NeededUnavailable,
}

impl ResourceRowVersion {
    pub const ALL: [ResourceRowVersion; 4] = [
        ResourceRowVersion::Legacy,
        ResourceRowVersion::CountOnly,
        ResourceRowVersion::Identity,
        ResourceRowVersion::NeededUnavailable,
    ];

    pub const fn wire(self) -> u16 {
        match self {
            ResourceRowVersion::Legacy => 1,
            ResourceRowVersion::CountOnly => 2,
            ResourceRowVersion::Identity => 3,
            ResourceRowVersion::NeededUnavailable => 4,
        }
    }

    pub const fn carries_identity(self) -> bool {
        match self {
            ResourceRowVersion::Identity | ResourceRowVersion::NeededUnavailable => true,
            ResourceRowVersion::Legacy | ResourceRowVersion::CountOnly => false,
        }
    }

    pub const fn pins_fixed7(self) -> bool {
        match self {
            ResourceRowVersion::Legacy => true,
            ResourceRowVersion::CountOnly
            | ResourceRowVersion::Identity
            | ResourceRowVersion::NeededUnavailable => false,
        }
    }

    pub const fn admits_needed_unavailable(self) -> bool {
        match self {
            ResourceRowVersion::NeededUnavailable => true,
            ResourceRowVersion::Legacy
            | ResourceRowVersion::CountOnly
            | ResourceRowVersion::Identity => false,
        }
    }

    fn from_wire(version: u16) -> Option<ResourceRowVersion> {
        let mut i = 0;
        while i < ResourceRowVersion::ALL.len() {
            if ResourceRowVersion::ALL[i].wire() == version {
                return Some(ResourceRowVersion::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DealVersion {
    Legacy,
    Statuses,
    PerOwner,
    WideIdentity,
}

impl DealVersion {
    pub const ALL: [DealVersion; 4] = [
        DealVersion::Legacy,
        DealVersion::Statuses,
        DealVersion::PerOwner,
        DealVersion::WideIdentity,
    ];

    pub const fn wire(self) -> u16 {
        match self {
            DealVersion::Legacy => 1,
            DealVersion::Statuses => 2,
            DealVersion::PerOwner => 3,
            DealVersion::WideIdentity => 4,
        }
    }

    pub const CURRENT: DealVersion = DealVersion::WideIdentity;

    pub const fn carries_incomplete_owners(self) -> bool {
        match self {
            DealVersion::PerOwner | DealVersion::WideIdentity => true,
            DealVersion::Legacy | DealVersion::Statuses => false,
        }
    }

    pub const fn carries_route_status(self) -> bool {
        match self {
            DealVersion::Statuses | DealVersion::PerOwner | DealVersion::WideIdentity => true,
            DealVersion::Legacy => false,
        }
    }

    pub const fn identity_shape(self) -> DealIdentityShape {
        match self {
            DealVersion::WideIdentity => DealIdentityShape::Wide,
            DealVersion::Legacy | DealVersion::Statuses | DealVersion::PerOwner => {
                DealIdentityShape::Packed
            }
        }
    }

    fn from_wire(version: u16) -> Option<DealVersion> {
        let mut i = 0;
        while i < DealVersion::ALL.len() {
            if DealVersion::ALL[i].wire() == version {
                return Some(DealVersion::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DealIdentityShape {
    Packed,
    Wide,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SingleVersion {
    V1,
}

impl SingleVersion {
    pub const ALL: [SingleVersion; 1] = [SingleVersion::V1];

    pub const fn wire(self) -> u16 {
        match self {
            SingleVersion::V1 => 1,
        }
    }

    fn from_wire(version: u16) -> Option<SingleVersion> {
        let mut i = 0;
        while i < SingleVersion::ALL.len() {
            if SingleVersion::ALL[i].wire() == version {
                return Some(SingleVersion::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

const _: () = {
    let mut i = 0;
    while i < FlowVersion::ALL.len() {
        if FlowVersion::ALL[i].wire() as usize != i + 1 {
            panic!("d43_grammar: FlowVersion::ALL is not 1..=n in order");
        }
        i += 1;
    }
    let mut i = 0;
    while i < ResourceRowVersion::ALL.len() {
        if ResourceRowVersion::ALL[i].wire() as usize != i + 1 {
            panic!("d43_grammar: ResourceRowVersion::ALL is not 1..=n in order");
        }
        i += 1;
    }
    let mut i = 0;
    while i < DealVersion::ALL.len() {
        if DealVersion::ALL[i].wire() as usize != i + 1 {
            panic!("d43_grammar: DealVersion::ALL is not 1..=n in order");
        }
        i += 1;
    }
    let mut i = 0;
    while i < SingleVersion::ALL.len() {
        if SingleVersion::ALL[i].wire() as usize != i + 1 {
            panic!("d43_grammar: SingleVersion::ALL is not 1..=n in order");
        }
        i += 1;
    }
    if DealVersion::CURRENT.wire() == DealVersion::Statuses.wire() {
        panic!("d43_grammar: DealVersion::CURRENT collapsed onto DealVersion::Statuses");
    }
};

pub const FLOW_FIXED7_ROW_LEN: usize = 72;

pub const FLOW_ROW_HEADER_LEN: usize = 16;

pub const FIXED7_RESOURCE_COUNT: u32 = 7;

pub const FLOW_RESOURCE_MASK: u32 = 0x7F;

pub const FLOW_KIND_MASK: u32 = 0x3F;

pub fn flow_row_len(resource_count: u32) -> usize {
    FLOW_ROW_HEADER_LEN.saturating_add((resource_count as usize).saturating_mul(8))
}

const _: () = {
    if FLOW_ROW_HEADER_LEN + (FIXED7_RESOURCE_COUNT as usize) * 8 != FLOW_FIXED7_ROW_LEN {
        panic!("d43_grammar: FLOW_FIXED7_ROW_LEN is not the fixed-seven row width");
    }
    if FLOW_KIND_MASK != (1u32 << 6) - 1 {
        panic!("d43_grammar: FLOW_KIND_MASK is not the low six bits");
    }
    if FLOW_RESOURCE_MASK != (1u32 << 7) - 1 {
        panic!("d43_grammar: FLOW_RESOURCE_MASK is not the low seven bits");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum FlowKind {
    Produced,
    Imported,
    Exported,
    Used,
    ToExport,
    BaseExport,
}

impl FlowKind {
    pub const ALL: [FlowKind; 6] = [
        FlowKind::Produced,
        FlowKind::Imported,
        FlowKind::Exported,
        FlowKind::Used,
        FlowKind::ToExport,
        FlowKind::BaseExport,
    ];

    pub const fn wire(self) -> u32 {
        match self {
            FlowKind::Produced => 1,
            FlowKind::Imported => 2,
            FlowKind::Exported => 3,
            FlowKind::Used => 4,
            FlowKind::ToExport => 5,
            FlowKind::BaseExport => 6,
        }
    }

    pub fn from_wire(kind: u32) -> Option<FlowKind> {
        let mut i = 0;
        while i < FlowKind::ALL.len() {
            if FlowKind::ALL[i].wire() == kind {
                return Some(FlowKind::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

const _: () = {
    let mut i = 0;
    while i < FlowKind::ALL.len() {
        if FlowKind::ALL[i].wire() as usize != i + 1 {
            panic!("d43_grammar: FlowKind::ALL is not 1..=n in order");
        }
        i += 1;
    }
    if FlowKind::ALL.len() != FLOW_KIND_MASK.count_ones() as usize {
        panic!("d43_grammar: FlowKind::ALL and FLOW_KIND_MASK disagree about the matrix width");
    }
};

/// sees. The difference is ownership: this is a plain parsed header a caller can
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TrailerHeaders {
    Detail(DetailHeader),
    ResourceFlow(FlowTrailer),
    FactoryHeader(FactoryHeader),
    ResourceRow(ResourceRowTrailer),
    ClassicDeal(DealTrailer),
    Activation(ActivationTrailer),
    Stockpile(StockpileTrailer),
    Residual(ResidualHeader),
    LendLease(LendLeaseHeader),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SizedTrailer {
    pub trailer: Trailer,
    pub at: usize,
    pub version: SingleVersion,
    pub header_len: usize,
    pub row_len: usize,
    pub row_count: usize,
    pub rows_at: usize,
    pub end: usize,
}

impl SizedTrailer {
    pub(crate) const fn rows(&self) -> RowCursor {
        RowCursor::new(self.rows_at, self.row_count, self.row_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetailHeader {
    pub sized: SizedTrailer,
    pub price_valid_rows: u32,
    pub speed_valid_rows: u32,
    pub delivery_route_valid_rows: u32,
    pub progress_valid_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactoryHeader {
    pub sized: SizedTrailer,
    pub country_count: usize,
    pub reserved1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidualHeader {
    pub sized: SizedTrailer,
    pub source_blocked_rows: u32,
    pub schema_blocked_rows: u32,
    pub owned_elsewhere_rows: u32,
    pub shared_patch_blocked_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LendLeaseHeader {
    pub sized: SizedTrailer,
    pub outgoing_rows: u32,
    pub incoming_rows: u32,
    pub amount_valid_rows: u32,
    pub price_valid_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlowTrailer {
    pub at: usize,
    pub version: FlowVersion,
    pub header_len: usize,
    pub row_len: usize,
    pub row_count: usize,
    pub country_count_or_exported: usize,
    pub scale_raw: u32,
    pub mask_or_count: u32,
    pub kind_mask: u32,
    pub resource_count: u32,
    pub rows_at: usize,
    pub end: usize,
}

impl FlowTrailer {
    pub(crate) const fn rows(&self) -> RowCursor {
        RowCursor::new(self.rows_at, self.row_count, self.row_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRowTrailer {
    pub at: usize,
    pub version: ResourceRowVersion,
    pub header_len: usize,
    pub row_len: usize,
    pub row_count: usize,
    pub country_count: usize,
    pub resource_count: u32,
    pub display_scale: u32,
    pub identity_stride_or_reserved: u32,
    pub identity_stride: usize,
    pub identity_at: usize,
    pub identity_len: usize,
    pub rows_at: usize,
    pub end: usize,
}

impl ResourceRowTrailer {
    pub(crate) fn identities(&self) -> RowCursor {
        let count = if self.version.carries_identity() {
            self.resource_count as usize
        } else {
            0
        };
        RowCursor::new(self.identity_at, count, self.identity_stride)
    }

    pub(crate) const fn rows(&self) -> RowCursor {
        RowCursor::new(self.rows_at, self.row_count, self.row_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DealTrailer {
    pub at: usize,
    pub version: DealVersion,
    pub header_len: usize,
    pub row_len: usize,
    pub row_count: usize,
    pub country_count: usize,
    pub scale_raw: u32,
    pub witness_count: usize,
    pub incomplete_count: usize,
    pub witnesses_at: usize,
    pub incomplete_at: usize,
    pub rows_at: usize,
    pub end: usize,
}

impl DealTrailer {
    pub(crate) const fn witnesses(&self) -> RowCursor {
        RowCursor::new(self.witnesses_at, self.witness_count, 4)
    }

    pub(crate) const fn incomplete_owners(&self) -> RowCursor {
        RowCursor::new(self.incomplete_at, self.incomplete_count, 4)
    }

    pub(crate) const fn rows(&self) -> RowCursor {
        RowCursor::new(self.rows_at, self.row_count, self.row_len)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationTrailer {
    pub at: usize,
    pub version: SingleVersion,
    pub header_len: usize,
    pub flags: u32,
    pub country_tag_db_id: u32,
    pub observation_sequence: u64,
    pub root_kind: u32,
    pub open_rva: u32,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StockpileTrailer {
    pub at: usize,
    pub version: SingleVersion,
    pub header_len: usize,
    pub country_row_len: usize,
    pub item_row_len: usize,
    pub declared_segment_len: usize,
    pub country_count: usize,
    pub item_count: usize,
    pub expected_country_count: usize,
    pub scale_raw: u32,
    pub flags: u32,
    pub reserved: u32,
    pub countries_at: usize,
    pub items_at: usize,
    pub end: usize,
}

impl StockpileTrailer {
    pub(crate) const fn countries(&self) -> RowCursor {
        RowCursor::new(self.countries_at, self.country_count, self.country_row_len)
    }

    pub(crate) const fn items(&self) -> RowCursor {
        RowCursor::new(self.items_at, self.item_count, self.item_row_len)
    }

    pub const fn computed_segment_len(&self) -> usize {
        self.end - self.at
    }
}

pub const fn fixed_row_len(trailer: Trailer) -> usize {
    match trailer {
        Trailer::Detail => 64,
        Trailer::FactoryHeader => 16,
        Trailer::ResourceRow => 64,
        Trailer::ClassicDeal => 96,
        Trailer::Residual => 48,
        Trailer::LendLease => 48,
        Trailer::ResourceFlow | Trailer::Activation | Trailer::Stockpile => 0,
    }
}

pub(crate) fn read_trailer_body(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
    limits: Limits,
) -> Result<TrailerHeaders, Violation> {
    let header_len = trailer.header_len();
    let header_end = add(Section::TrailerHeader(trailer), at, header_len)?;
    if header_end > payload.len() {
        return Err(Violation::new(
            at,
            ViolationKind::Need {
                section: Section::TrailerHeader(trailer),
                needed: header_end,
                available: payload.len(),
            },
        ));
    }
    match trailer {
        Trailer::Detail => {
            let sized = read_sized_trailer(payload, at, trailer)?;
            Ok(TrailerHeaders::Detail(DetailHeader {
                sized,
                price_valid_rows: u32_at(payload, at + 16),
                speed_valid_rows: u32_at(payload, at + 20),
                delivery_route_valid_rows: u32_at(payload, at + 24),
                progress_valid_rows: u32_at(payload, at + 28),
            }))
        }
        Trailer::FactoryHeader => {
            let sized = read_sized_trailer(payload, at, trailer)?;
            Ok(TrailerHeaders::FactoryHeader(FactoryHeader {
                sized,
                country_count: u32_at(payload, at + 16) as usize,
                reserved1: u32_at(payload, at + 20),
            }))
        }
        Trailer::Residual => {
            let sized = read_sized_trailer(payload, at, trailer)?;
            Ok(TrailerHeaders::Residual(ResidualHeader {
                sized,
                source_blocked_rows: u32_at(payload, at + 16),
                schema_blocked_rows: u32_at(payload, at + 20),
                owned_elsewhere_rows: u32_at(payload, at + 24),
                shared_patch_blocked_rows: u32_at(payload, at + 28),
            }))
        }
        Trailer::LendLease => {
            let sized = read_sized_trailer(payload, at, trailer)?;
            Ok(TrailerHeaders::LendLease(LendLeaseHeader {
                sized,
                outgoing_rows: u32_at(payload, at + 16),
                incoming_rows: u32_at(payload, at + 20),
                amount_valid_rows: u32_at(payload, at + 24),
                price_valid_rows: u32_at(payload, at + 28),
            }))
        }
        Trailer::ResourceFlow => Ok(TrailerHeaders::ResourceFlow(read_flow_trailer(
            payload, at, limits,
        )?)),
        Trailer::ResourceRow => Ok(TrailerHeaders::ResourceRow(read_resource_row_trailer(
            payload, at, limits,
        )?)),
        Trailer::ClassicDeal => Ok(TrailerHeaders::ClassicDeal(read_deal_trailer(payload, at)?)),
        Trailer::Activation => Ok(TrailerHeaders::Activation(read_activation_trailer(
            payload, at,
        )?)),
        Trailer::Stockpile => Ok(TrailerHeaders::Stockpile(read_stockpile_trailer(
            payload, at,
        )?)),
    }
}

fn declared_header_len(payload: &[u8], at: usize, trailer: Trailer) -> Result<usize, Violation> {
    let declared = u16_at(payload, at + TRAILER_HEADER_LEN_OFFSET) as usize;
    let expected = trailer.header_len();
    if declared != expected {
        return Err(Violation::new(
            at,
            ViolationKind::HeaderLen {
                trailer,
                declared,
                expected,
            },
        ));
    }
    Ok(expected)
}

fn declared_row_len(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
    expected: usize,
) -> Result<usize, Violation> {
    let declared = u16_at(payload, at + TRAILER_ROW_LEN_OFFSET) as usize;
    if declared != expected {
        return Err(Violation::new(
            at,
            ViolationKind::RowLen {
                trailer,
                declared,
                expected,
            },
        ));
    }
    Ok(expected)
}

fn extent(
    trailer: Trailer,
    at: usize,
    parts: &[(usize, usize)],
    payload_len: usize,
) -> Result<usize, Violation> {
    let section = Section::TrailerBody(trailer);
    let mut end = at;
    let mut i = 0;
    while i < parts.len() {
        let (count, stride) = parts[i];
        let bytes = mul(section, end, count, stride)?;
        end = add(section, end, bytes)?;
        i += 1;
    }
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
    Ok(end)
}

fn read_sized_trailer(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
) -> Result<SizedTrailer, Violation> {
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = SingleVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let row_len = declared_row_len(payload, at, trailer, fixed_row_len(trailer))?;
    let row_count = u32_at(payload, at + TRAILER_ROW_COUNT_OFFSET) as usize;
    let rows_at = at + header_len;
    let end = extent(
        trailer,
        at,
        &[(1, header_len), (row_count, row_len)],
        payload.len(),
    )?;
    Ok(SizedTrailer {
        trailer,
        at,
        version,
        header_len,
        row_len,
        row_count,
        rows_at,
        end,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowStep {
    Row { index: usize, at: usize },
    End(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct RowCursor {
    at: usize,
    index: usize,
    count: usize,
    stride: usize,
}

impl RowCursor {
    pub(crate) const fn new(at: usize, count: usize, stride: usize) -> Self {
        RowCursor {
            at,
            index: 0,
            count,
            stride,
        }
    }

    pub const fn offset(&self) -> usize {
        self.at
    }

    pub const fn remaining(&self) -> usize {
        self.count - self.index
    }

    pub fn next(&mut self) -> RowStep {
        if self.index >= self.count {
            return RowStep::End(self.at);
        }
        let step = RowStep::Row {
            index: self.index,
            at: self.at,
        };
        self.at += self.stride;
        self.index += 1;
        step
    }
}

fn read_flow_trailer(payload: &[u8], at: usize, limits: Limits) -> Result<FlowTrailer, Violation> {
    let trailer = Trailer::ResourceFlow;
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = FlowVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let row_count = u32_at(payload, at + TRAILER_ROW_COUNT_OFFSET) as usize;
    let country_count_or_exported = u32_at(payload, at + 16) as usize;
    let scale_raw = u32_at(payload, at + 20);
    let mask_or_count = u32_at(payload, at + 24);
    let kind_mask = u32_at(payload, at + 28);

    let resource_count = if version.states_resource_count() {
        if mask_or_count == 0 || mask_or_count > limits.max_resource_count() {
            return Err(Violation::new(
                at,
                ViolationKind::ResourceCount {
                    trailer,
                    declared: mask_or_count,
                    max: limits.max_resource_count(),
                },
            ));
        }
        mask_or_count
    } else {
        FIXED7_RESOURCE_COUNT
    };
    let expected_row_len = if version.states_resource_count() {
        flow_row_len(resource_count)
    } else {
        FLOW_FIXED7_ROW_LEN
    };
    let row_len = declared_row_len(payload, at, trailer, expected_row_len)?;
    let rows_at = at + header_len;
    let end = extent(
        trailer,
        at,
        &[(1, header_len), (row_count, row_len)],
        payload.len(),
    )?;
    Ok(FlowTrailer {
        at,
        version,
        header_len,
        row_len,
        row_count,
        country_count_or_exported,
        scale_raw,
        mask_or_count,
        kind_mask,
        resource_count,
        rows_at,
        end,
    })
}

fn read_resource_row_trailer(
    payload: &[u8],
    at: usize,
    limits: Limits,
) -> Result<ResourceRowTrailer, Violation> {
    let trailer = Trailer::ResourceRow;
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = ResourceRowVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let row_len = declared_row_len(payload, at, trailer, fixed_row_len(trailer))?;
    let row_count = u32_at(payload, at + TRAILER_ROW_COUNT_OFFSET) as usize;
    let country_count = u32_at(payload, at + 16) as usize;
    let resource_count = u32_at(payload, at + 20);
    let display_scale = u32_at(payload, at + 24);
    let identity_stride_or_reserved = u32_at(payload, at + 28);

    let admitted = if version.pins_fixed7() {
        resource_count == FIXED7_RESOURCE_COUNT
    } else {
        resource_count >= 1 && resource_count <= limits.max_resource_count()
    };
    if !admitted {
        return Err(Violation::new(
            at,
            ViolationKind::ResourceCount {
                trailer,
                declared: resource_count,
                max: if version.pins_fixed7() {
                    FIXED7_RESOURCE_COUNT
                } else {
                    limits.max_resource_count()
                },
            },
        ));
    }
    let identity_stride = if version.carries_identity() {
        if limits.identity_stride() < 2 {
            return Err(Violation::new(
                at,
                ViolationKind::IdentityStride {
                    trailer,
                    stride: limits.identity_stride(),
                },
            ));
        }
        limits.identity_stride()
    } else {
        0
    };
    let identity_len = if version.carries_identity() {
        if identity_stride_or_reserved != limits.identity_stride() as u32 {
            return Err(Violation::new(
                at,
                ViolationKind::AbsentBlockDeclared {
                    trailer,
                    declared: identity_stride_or_reserved,
                },
            ));
        }
        mul(
            Section::TrailerBody(trailer),
            at,
            resource_count as usize,
            limits.identity_stride(),
        )?
    } else {
        if identity_stride_or_reserved != 0 {
            return Err(Violation::new(
                at,
                ViolationKind::AbsentBlockDeclared {
                    trailer,
                    declared: identity_stride_or_reserved,
                },
            ));
        }
        0
    };
    let identity_at = at + header_len;
    let rows_at = add(Section::TrailerBody(trailer), identity_at, identity_len)?;
    let end = extent(
        trailer,
        at,
        &[(1, header_len), (1, identity_len), (row_count, row_len)],
        payload.len(),
    )?;
    Ok(ResourceRowTrailer {
        at,
        version,
        header_len,
        row_len,
        row_count,
        country_count,
        resource_count,
        display_scale,
        identity_stride_or_reserved,
        identity_stride,
        identity_at,
        identity_len,
        rows_at,
        end,
    })
}

fn read_deal_trailer(payload: &[u8], at: usize) -> Result<DealTrailer, Violation> {
    let trailer = Trailer::ClassicDeal;
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = DealVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let row_len = declared_row_len(payload, at, trailer, fixed_row_len(trailer))?;
    let row_count = u32_at(payload, at + TRAILER_ROW_COUNT_OFFSET) as usize;
    let country_count = u32_at(payload, at + 16) as usize;
    let scale_raw = u32_at(payload, at + 20);
    let witness_count = u32_at(payload, at + 24) as usize;
    let incomplete_word = u32_at(payload, at + 28);
    if !version.carries_incomplete_owners() && incomplete_word != 0 {
        return Err(Violation::new(
            at,
            ViolationKind::AbsentBlockDeclared {
                trailer,
                declared: incomplete_word,
            },
        ));
    }
    let incomplete_count = incomplete_word as usize;
    let witnesses_at = at + header_len;
    let witness_bytes = mul(
        Section::TrailerBody(trailer),
        witnesses_at,
        witness_count,
        4,
    )?;
    let incomplete_at = add(Section::TrailerBody(trailer), witnesses_at, witness_bytes)?;
    let incomplete_bytes = mul(
        Section::TrailerBody(trailer),
        incomplete_at,
        incomplete_count,
        4,
    )?;
    let rows_at = add(
        Section::TrailerBody(trailer),
        incomplete_at,
        incomplete_bytes,
    )?;
    let end = extent(
        trailer,
        at,
        &[
            (1, header_len),
            (witness_count, 4),
            (incomplete_count, 4),
            (row_count, row_len),
        ],
        payload.len(),
    )?;
    Ok(DealTrailer {
        at,
        version,
        header_len,
        row_len,
        row_count,
        country_count,
        scale_raw,
        witness_count,
        incomplete_count,
        witnesses_at,
        incomplete_at,
        rows_at,
        end,
    })
}

fn read_activation_trailer(payload: &[u8], at: usize) -> Result<ActivationTrailer, Violation> {
    let trailer = Trailer::Activation;
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = SingleVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let end = extent(trailer, at, &[(1, header_len)], payload.len())?;
    Ok(ActivationTrailer {
        at,
        version,
        header_len,
        flags: u32_at(payload, at + 8),
        country_tag_db_id: u32_at(payload, at + 12),
        observation_sequence: u64_at(payload, at + 16),
        root_kind: u32_at(payload, at + 24),
        open_rva: u32_at(payload, at + 28),
        end,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ActivationFlag {
    Active,
    RootExact,
}

impl ActivationFlag {
    pub const ALL: [ActivationFlag; 2] = [ActivationFlag::Active, ActivationFlag::RootExact];

    pub const fn bit(self) -> u32 {
        match self {
            ActivationFlag::Active => 1 << 0,
            ActivationFlag::RootExact => 1 << 1,
        }
    }
}

pub const ACTIVATION_FLAG_ACTIVE: u32 = ActivationFlag::Active.bit();
pub const ACTIVATION_FLAG_ROOT_EXACT: u32 = ActivationFlag::RootExact.bit();

pub const KNOWN_ACTIVATION_FLAGS: u32 = known_activation_flags();

const fn known_activation_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < ActivationFlag::ALL.len() {
        mask |= ActivationFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

pub const ACTIVATION_ROOT_COUNTRYTRADEVIEW: u32 = 1;

pub const ACTIVATION_OPEN_RVA: u32 = 0x015e_c520;

pub const STOCKPILE_COUNTRY_ROW_LEN: usize = 20;
pub const STOCKPILE_ITEM_ROW_LEN: usize = 24;

fn read_stockpile_trailer(payload: &[u8], at: usize) -> Result<StockpileTrailer, Violation> {
    let trailer = Trailer::Stockpile;
    let version_word = u16_at(payload, at + TRAILER_VERSION_OFFSET);
    let Some(version) = SingleVersion::from_wire(version_word) else {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: version_word,
            },
        ));
    };
    let header_len = declared_header_len(payload, at, trailer)?;
    let country_row_len = u16_at(payload, at + TRAILER_ROW_LEN_OFFSET) as usize;
    if country_row_len != STOCKPILE_COUNTRY_ROW_LEN {
        return Err(Violation::new(
            at,
            ViolationKind::RowLen {
                trailer,
                declared: country_row_len,
                expected: STOCKPILE_COUNTRY_ROW_LEN,
            },
        ));
    }
    let item_row_len = u16_at(payload, at + TRAILER_RESERVED0_OFFSET) as usize;
    if item_row_len != STOCKPILE_ITEM_ROW_LEN {
        return Err(Violation::new(
            at,
            ViolationKind::RowLen {
                trailer,
                declared: item_row_len,
                expected: STOCKPILE_ITEM_ROW_LEN,
            },
        ));
    }
    let declared_segment_len = u32_at(payload, at + 12) as usize;
    let country_count = u32_at(payload, at + 16) as usize;
    let item_count = u32_at(payload, at + 20) as usize;
    let expected_country_count = u32_at(payload, at + 24) as usize;
    let scale_raw = u32_at(payload, at + 28);
    let flags = u32_at(payload, at + 32);
    let reserved = u32_at(payload, at + 36);
    let countries_at = at + header_len;
    let country_bytes = mul(
        Section::TrailerBody(trailer),
        countries_at,
        country_count,
        country_row_len,
    )?;
    let items_at = add(Section::TrailerBody(trailer), countries_at, country_bytes)?;
    let end = extent(
        trailer,
        at,
        &[
            (1, header_len),
            (country_count, country_row_len),
            (item_count, item_row_len),
        ],
        payload.len(),
    )?;
    Ok(StockpileTrailer {
        at,
        version,
        header_len,
        country_row_len,
        item_row_len,
        declared_segment_len,
        country_count,
        item_count,
        expected_country_count,
        scale_raw,
        flags,
        reserved,
        countries_at,
        items_at,
        end,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetailRowField {
    ContractId,
    Flags,
    PriceRaw,
    AmountRaw,
    SpeedRaw,
    CompletedRaw,
    Days,
    DeliveryCountryTagDbId,
    ConvoysAssigned,
    ConvoysTotal,
    EfficiencyRaw,
    FactoryCicProgressRaw,
    EquipmentTransferProgressRaw,
    Reserved,
}

impl DetailRowField {
    pub const ALL: [DetailRowField; 14] = [
        DetailRowField::ContractId,
        DetailRowField::Flags,
        DetailRowField::PriceRaw,
        DetailRowField::AmountRaw,
        DetailRowField::SpeedRaw,
        DetailRowField::CompletedRaw,
        DetailRowField::Days,
        DetailRowField::DeliveryCountryTagDbId,
        DetailRowField::ConvoysAssigned,
        DetailRowField::ConvoysTotal,
        DetailRowField::EfficiencyRaw,
        DetailRowField::FactoryCicProgressRaw,
        DetailRowField::EquipmentTransferProgressRaw,
        DetailRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            DetailRowField::ContractId => 0,
            DetailRowField::Flags => 4,
            DetailRowField::PriceRaw => 8,
            DetailRowField::AmountRaw => 16,
            DetailRowField::SpeedRaw => 24,
            DetailRowField::CompletedRaw => 28,
            DetailRowField::Days => 32,
            DetailRowField::DeliveryCountryTagDbId => 36,
            DetailRowField::ConvoysAssigned => 40,
            DetailRowField::ConvoysTotal => 44,
            DetailRowField::EfficiencyRaw => 48,
            DetailRowField::FactoryCicProgressRaw => 52,
            DetailRowField::EquipmentTransferProgressRaw => 56,
            DetailRowField::Reserved => 60,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            DetailRowField::PriceRaw | DetailRowField::AmountRaw => Width::I64,
            DetailRowField::ContractId
            | DetailRowField::Flags
            | DetailRowField::DeliveryCountryTagDbId
            | DetailRowField::Reserved => Width::U32,
            DetailRowField::SpeedRaw
            | DetailRowField::CompletedRaw
            | DetailRowField::Days
            | DetailRowField::ConvoysAssigned
            | DetailRowField::ConvoysTotal
            | DetailRowField::EfficiencyRaw
            | DetailRowField::FactoryCicProgressRaw
            | DetailRowField::EquipmentTransferProgressRaw => Width::I32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DetailFlag {
    PriceValid,
    AmountValid,
    SpeedValid,
    CompletedValid,
    DaysValid,
    DeliveryCountryValid,
    ConvoysValid,
    EfficiencyValid,
    FactoryProgressValid,
    EquipmentProgressValid,
}

impl DetailFlag {
    pub const ALL: [DetailFlag; 10] = [
        DetailFlag::PriceValid,
        DetailFlag::AmountValid,
        DetailFlag::SpeedValid,
        DetailFlag::CompletedValid,
        DetailFlag::DaysValid,
        DetailFlag::DeliveryCountryValid,
        DetailFlag::ConvoysValid,
        DetailFlag::EfficiencyValid,
        DetailFlag::FactoryProgressValid,
        DetailFlag::EquipmentProgressValid,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            DetailFlag::PriceValid => 1 << 0,
            DetailFlag::AmountValid => 1 << 1,
            DetailFlag::SpeedValid => 1 << 2,
            DetailFlag::CompletedValid => 1 << 3,
            DetailFlag::DaysValid => 1 << 4,
            DetailFlag::DeliveryCountryValid => 1 << 5,
            DetailFlag::ConvoysValid => 1 << 6,
            DetailFlag::EfficiencyValid => 1 << 7,
            DetailFlag::FactoryProgressValid => 1 << 8,
            DetailFlag::EquipmentProgressValid => 1 << 9,
        }
    }

    pub const fn is_set(self, flags: u32) -> bool {
        flags & self.bit() != 0
    }
}

pub const KNOWN_DETAIL_FLAGS: u32 = known_detail_flags();

const fn known_detail_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < DetailFlag::ALL.len() {
        mask |= DetailFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetailRow {
    pub contract_id: u32,
    pub flags: u32,
    pub price_raw: i64,
    pub amount_raw: i64,
    pub speed_raw: i32,
    pub completed_raw: i32,
    pub days: i32,
    pub delivery_country_tag_db_id: u32,
    pub convoys_assigned: i32,
    pub convoys_total: i32,
    pub efficiency_raw: i32,
    pub factory_cic_progress_raw: i32,
    pub equipment_transfer_progress_raw: i32,
    pub reserved: u32,
}

pub fn detail_row_at(payload: &[u8], at: usize) -> DetailRow {
    DetailRow {
        contract_id: u32_at(payload, at + DetailRowField::ContractId.offset()),
        flags: u32_at(payload, at + DetailRowField::Flags.offset()),
        price_raw: i64_at(payload, at + DetailRowField::PriceRaw.offset()),
        amount_raw: i64_at(payload, at + DetailRowField::AmountRaw.offset()),
        speed_raw: i32_at(payload, at + DetailRowField::SpeedRaw.offset()),
        completed_raw: i32_at(payload, at + DetailRowField::CompletedRaw.offset()),
        days: i32_at(payload, at + DetailRowField::Days.offset()),
        delivery_country_tag_db_id: u32_at(
            payload,
            at + DetailRowField::DeliveryCountryTagDbId.offset(),
        ),
        convoys_assigned: i32_at(payload, at + DetailRowField::ConvoysAssigned.offset()),
        convoys_total: i32_at(payload, at + DetailRowField::ConvoysTotal.offset()),
        efficiency_raw: i32_at(payload, at + DetailRowField::EfficiencyRaw.offset()),
        factory_cic_progress_raw: i32_at(
            payload,
            at + DetailRowField::FactoryCicProgressRaw.offset(),
        ),
        equipment_transfer_progress_raw: i32_at(
            payload,
            at + DetailRowField::EquipmentTransferProgressRaw.offset(),
        ),
        reserved: u32_at(payload, at + DetailRowField::Reserved.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FactoryRowField {
    TagDbId,
    Total,
    Used,
    Available,
}

impl FactoryRowField {
    pub const ALL: [FactoryRowField; 4] = [
        FactoryRowField::TagDbId,
        FactoryRowField::Total,
        FactoryRowField::Used,
        FactoryRowField::Available,
    ];

    pub const fn offset(self) -> usize {
        match self {
            FactoryRowField::TagDbId => 0,
            FactoryRowField::Total => 4,
            FactoryRowField::Used => 8,
            FactoryRowField::Available => 12,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            FactoryRowField::TagDbId => Width::U32,
            FactoryRowField::Total | FactoryRowField::Used | FactoryRowField::Available => {
                Width::I32
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactoryRow {
    pub tag_db_id: u32,
    pub total: i32,
    pub used: i32,
    pub available: i32,
}

pub fn factory_row_at(payload: &[u8], at: usize) -> FactoryRow {
    FactoryRow {
        tag_db_id: u32_at(payload, at + FactoryRowField::TagDbId.offset()),
        total: i32_at(payload, at + FactoryRowField::Total.offset()),
        used: i32_at(payload, at + FactoryRowField::Used.offset()),
        available: i32_at(payload, at + FactoryRowField::Available.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlowRowField {
    TagDbId,
    FlowKind,
    MaskOrCount,
    Reserved,
}

impl FlowRowField {
    pub const ALL: [FlowRowField; 4] = [
        FlowRowField::TagDbId,
        FlowRowField::FlowKind,
        FlowRowField::MaskOrCount,
        FlowRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            FlowRowField::TagDbId => 0,
            FlowRowField::FlowKind => 4,
            FlowRowField::MaskOrCount => 8,
            FlowRowField::Reserved => 12,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            FlowRowField::TagDbId
            | FlowRowField::FlowKind
            | FlowRowField::MaskOrCount
            | FlowRowField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlowRow {
    pub tag_db_id: u32,
    pub flow_kind: u32,
    pub mask_or_count: u32,
    pub reserved: u32,
}

pub fn flow_row_at(payload: &[u8], at: usize) -> FlowRow {
    FlowRow {
        tag_db_id: u32_at(payload, at + FlowRowField::TagDbId.offset()),
        flow_kind: u32_at(payload, at + FlowRowField::FlowKind.offset()),
        mask_or_count: u32_at(payload, at + FlowRowField::MaskOrCount.offset()),
        reserved: u32_at(payload, at + FlowRowField::Reserved.offset()),
    }
}

pub fn flow_value_at(payload: &[u8], row_at: usize, slot: usize) -> i64 {
    i64_at(payload, row_at + FLOW_ROW_HEADER_LEN + slot * 8)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceRowField {
    TagDbId,
    ResourceIndex,
    VisibleOrder,
    NeededType,
    Balance,
    Produced,
    Imported,
    Exported,
    SpecialProject,
    ProductionLine,
    Needed,
    NeededCurrent,
    NeededMax,
    Flags,
    DescriptorDbIndex,
    Reserved,
}

impl ResourceRowField {
    pub const ALL: [ResourceRowField; 16] = [
        ResourceRowField::TagDbId,
        ResourceRowField::ResourceIndex,
        ResourceRowField::VisibleOrder,
        ResourceRowField::NeededType,
        ResourceRowField::Balance,
        ResourceRowField::Produced,
        ResourceRowField::Imported,
        ResourceRowField::Exported,
        ResourceRowField::SpecialProject,
        ResourceRowField::ProductionLine,
        ResourceRowField::Needed,
        ResourceRowField::NeededCurrent,
        ResourceRowField::NeededMax,
        ResourceRowField::Flags,
        ResourceRowField::DescriptorDbIndex,
        ResourceRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            ResourceRowField::TagDbId => 0,
            ResourceRowField::ResourceIndex => 4,
            ResourceRowField::VisibleOrder => 8,
            ResourceRowField::NeededType => 12,
            ResourceRowField::Balance => 16,
            ResourceRowField::Produced => 20,
            ResourceRowField::Imported => 24,
            ResourceRowField::Exported => 28,
            ResourceRowField::SpecialProject => 32,
            ResourceRowField::ProductionLine => 36,
            ResourceRowField::Needed => 40,
            ResourceRowField::NeededCurrent => 44,
            ResourceRowField::NeededMax => 48,
            ResourceRowField::Flags => 52,
            ResourceRowField::DescriptorDbIndex => 56,
            ResourceRowField::Reserved => 60,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            ResourceRowField::TagDbId
            | ResourceRowField::ResourceIndex
            | ResourceRowField::VisibleOrder
            | ResourceRowField::NeededType
            | ResourceRowField::Flags
            | ResourceRowField::DescriptorDbIndex
            | ResourceRowField::Reserved => Width::U32,
            ResourceRowField::Balance
            | ResourceRowField::Produced
            | ResourceRowField::Imported
            | ResourceRowField::Exported
            | ResourceRowField::SpecialProject
            | ResourceRowField::ProductionLine
            | ResourceRowField::Needed
            | ResourceRowField::NeededCurrent
            | ResourceRowField::NeededMax => Width::I32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum NeededType {
    Surplus,
    Deficit,
    CurrentMax,
    Unavailable,
}

impl NeededType {
    pub const ALL: [NeededType; 4] = [
        NeededType::Surplus,
        NeededType::Deficit,
        NeededType::CurrentMax,
        NeededType::Unavailable,
    ];

    pub const fn wire(self) -> u32 {
        match self {
            NeededType::Surplus => 1,
            NeededType::Deficit => 2,
            NeededType::CurrentMax => 3,
            NeededType::Unavailable => 4,
        }
    }

    pub fn from_wire(value: u32) -> Option<NeededType> {
        let mut i = 0;
        while i < NeededType::ALL.len() {
            if NeededType::ALL[i].wire() == value {
                return Some(NeededType::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ResourceFlag {
    NeededCurrent,
    NeededMax,
    NeededUnavailable,
}

impl ResourceFlag {
    pub const ALL: [ResourceFlag; 3] = [
        ResourceFlag::NeededCurrent,
        ResourceFlag::NeededMax,
        ResourceFlag::NeededUnavailable,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            ResourceFlag::NeededCurrent => 1 << 0,
            ResourceFlag::NeededMax => 1 << 1,
            ResourceFlag::NeededUnavailable => 1 << 2,
        }
    }
}

pub const RESOURCE_FLAGS_OIL: u32 =
    ResourceFlag::NeededCurrent.bit() | ResourceFlag::NeededMax.bit();

pub const KNOWN_RESOURCE_FLAGS: u32 = known_resource_flags();

const fn known_resource_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < ResourceFlag::ALL.len() {
        mask |= ResourceFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRow {
    pub tag_db_id: u32,
    pub resource_index: u32,
    pub visible_order: u32,
    pub needed_type: u32,
    pub balance: i32,
    pub produced: i32,
    pub imported: i32,
    pub exported: i32,
    pub special_project: i32,
    pub production_line: i32,
    pub needed: i32,
    pub needed_current: i32,
    pub needed_max: i32,
    pub flags: u32,
    pub descriptor_db_index: u32,
    pub reserved: u32,
}

pub fn resource_row_at(payload: &[u8], at: usize) -> ResourceRow {
    ResourceRow {
        tag_db_id: u32_at(payload, at + ResourceRowField::TagDbId.offset()),
        resource_index: u32_at(payload, at + ResourceRowField::ResourceIndex.offset()),
        visible_order: u32_at(payload, at + ResourceRowField::VisibleOrder.offset()),
        needed_type: u32_at(payload, at + ResourceRowField::NeededType.offset()),
        balance: i32_at(payload, at + ResourceRowField::Balance.offset()),
        produced: i32_at(payload, at + ResourceRowField::Produced.offset()),
        imported: i32_at(payload, at + ResourceRowField::Imported.offset()),
        exported: i32_at(payload, at + ResourceRowField::Exported.offset()),
        special_project: i32_at(payload, at + ResourceRowField::SpecialProject.offset()),
        production_line: i32_at(payload, at + ResourceRowField::ProductionLine.offset()),
        needed: i32_at(payload, at + ResourceRowField::Needed.offset()),
        needed_current: i32_at(payload, at + ResourceRowField::NeededCurrent.offset()),
        needed_max: i32_at(payload, at + ResourceRowField::NeededMax.offset()),
        flags: u32_at(payload, at + ResourceRowField::Flags.offset()),
        descriptor_db_index: u32_at(payload, at + ResourceRowField::DescriptorDbIndex.offset()),
        reserved: u32_at(payload, at + ResourceRowField::Reserved.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentityEntry<'a> {
    pub declared_len: usize,
    pub token: Option<&'a [u8]>,
    pub padding: Option<&'a [u8]>,
}

pub fn identity_entry_at<'a>(
    payload: &'a [u8],
    at: usize,
    stride: usize,
) -> Option<IdentityEntry<'a>> {
    // EVERY CHECK BELOW IS A RELEASE CHECK. `read_resource_row_trailer` refuses
    if stride < 2 {
        return None;
    }
    let entry_end = at.checked_add(stride)?;
    if entry_end > payload.len() {
        return None;
    }
    let declared_len = u16_at(payload, at) as usize;
    let max = stride - 2;
    if declared_len > max {
        return Some(IdentityEntry {
            declared_len,
            token: None,
            padding: None,
        });
    }
    Some(IdentityEntry {
        declared_len,
        token: Some(&payload[at + 2..at + 2 + declared_len]),
        padding: Some(&payload[at + 2 + declared_len..entry_end]),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DealRowField {
    OwnerTagDbId,
    CounterpartTagDbId,
    ResourceIndex,
    VisibleOrder,
    Direction,
    RouteKind,
    Flags,
    RouteStatusOrReserved,
    IdentityLow,
    IdentityHigh,
    RequestedRaw,
    DeliveredRaw,
    AvailableExportRaw,
    FactoryCostRaw,
    EfficiencyRaw,
    CivilianFactories,
    ConvoysAssigned,
    ConvoysRequired,
    OrdinalOrReserved,
}

impl DealRowField {
    pub const ALL: [DealRowField; 19] = [
        DealRowField::OwnerTagDbId,
        DealRowField::CounterpartTagDbId,
        DealRowField::ResourceIndex,
        DealRowField::VisibleOrder,
        DealRowField::Direction,
        DealRowField::RouteKind,
        DealRowField::Flags,
        DealRowField::RouteStatusOrReserved,
        DealRowField::IdentityLow,
        DealRowField::IdentityHigh,
        DealRowField::RequestedRaw,
        DealRowField::DeliveredRaw,
        DealRowField::AvailableExportRaw,
        DealRowField::FactoryCostRaw,
        DealRowField::EfficiencyRaw,
        DealRowField::CivilianFactories,
        DealRowField::ConvoysAssigned,
        DealRowField::ConvoysRequired,
        DealRowField::OrdinalOrReserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            DealRowField::OwnerTagDbId => 0,
            DealRowField::CounterpartTagDbId => 4,
            DealRowField::ResourceIndex => 8,
            DealRowField::VisibleOrder => 12,
            DealRowField::Direction => 16,
            DealRowField::RouteKind => 20,
            DealRowField::Flags => 24,
            DealRowField::RouteStatusOrReserved => 28,
            DealRowField::IdentityLow => 32,
            DealRowField::IdentityHigh => 36,
            DealRowField::RequestedRaw => 40,
            DealRowField::DeliveredRaw => 48,
            DealRowField::AvailableExportRaw => 56,
            DealRowField::FactoryCostRaw => 64,
            DealRowField::EfficiencyRaw => 72,
            DealRowField::CivilianFactories => 80,
            DealRowField::ConvoysAssigned => 84,
            DealRowField::ConvoysRequired => 88,
            DealRowField::OrdinalOrReserved => 92,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            DealRowField::RequestedRaw
            | DealRowField::DeliveredRaw
            | DealRowField::AvailableExportRaw
            | DealRowField::FactoryCostRaw
            | DealRowField::EfficiencyRaw => Width::I64,
            DealRowField::CivilianFactories
            | DealRowField::ConvoysAssigned
            | DealRowField::ConvoysRequired => Width::I32,
            DealRowField::OwnerTagDbId
            | DealRowField::CounterpartTagDbId
            | DealRowField::ResourceIndex
            | DealRowField::VisibleOrder
            | DealRowField::Direction
            | DealRowField::RouteKind
            | DealRowField::Flags
            | DealRowField::RouteStatusOrReserved
            | DealRowField::IdentityLow
            | DealRowField::IdentityHigh
            | DealRowField::OrdinalOrReserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DealDirection {
    Import,
    Export,
}

impl DealDirection {
    pub const ALL: [DealDirection; 2] = [DealDirection::Import, DealDirection::Export];

    pub const fn wire(self) -> u32 {
        match self {
            DealDirection::Import => 1,
            DealDirection::Export => 2,
        }
    }

    pub fn from_wire(value: u32) -> Option<DealDirection> {
        let mut i = 0;
        while i < DealDirection::ALL.len() {
            if DealDirection::ALL[i].wire() == value {
                return Some(DealDirection::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DealRoute {
    Land,
    Sea,
    Locked,
}

impl DealRoute {
    pub const ALL: [DealRoute; 3] = [DealRoute::Land, DealRoute::Sea, DealRoute::Locked];

    pub const fn wire(self) -> u32 {
        match self {
            DealRoute::Land => 1,
            DealRoute::Sea => 2,
            DealRoute::Locked => 3,
        }
    }

    pub fn from_wire(value: u32) -> Option<DealRoute> {
        let mut i = 0;
        while i < DealRoute::ALL.len() {
            if DealRoute::ALL[i].wire() == value {
                return Some(DealRoute::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

pub const DEAL_FLAG_EXACT: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DealStatus {
    NoTradeInfluence,
    NoResources,
}

impl DealStatus {
    pub const ALL: [DealStatus; 2] = [DealStatus::NoTradeInfluence, DealStatus::NoResources];

    pub const fn bit(self) -> u32 {
        match self {
            DealStatus::NoTradeInfluence => 1 << 0,
            DealStatus::NoResources => 1 << 1,
        }
    }
}

pub const KNOWN_DEAL_STATUS: u32 = known_deal_status();

const fn known_deal_status() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < DealStatus::ALL.len() {
        mask |= DealStatus::ALL[i].bit();
        i += 1;
    }
    mask
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DealRow {
    pub owner_tag_db_id: u32,
    pub counterpart_tag_db_id: u32,
    pub resource_index: u32,
    pub visible_order: u32,
    pub direction: u32,
    pub route_kind: u32,
    pub flags: u32,
    pub route_status_or_reserved: u32,
    pub identity_exporter_tag_db_id: u32,
    pub identity_importer_tag_db_id: u32,
    pub identity_ordinal: u32,
    pub packed_identity: Option<u64>,
    pub reserved_ordinal_word: Option<u32>,
    pub requested_raw: i64,
    pub delivered_raw: i64,
    pub available_export_raw: i64,
    pub factory_cost_raw: i64,
    pub efficiency_raw: i64,
    pub civilian_factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}

pub fn deal_row_at(payload: &[u8], at: usize, version: DealVersion) -> DealRow {
    let (exporter, importer, ordinal, packed, reserved_ordinal) = match version.identity_shape() {
        DealIdentityShape::Wide => (
            u32_at(payload, at + DealRowField::IdentityLow.offset()),
            u32_at(payload, at + DealRowField::IdentityHigh.offset()),
            u32_at(payload, at + DealRowField::OrdinalOrReserved.offset()),
            None,
            None,
        ),
        DealIdentityShape::Packed => {
            let packed = u64_at(payload, at + DealRowField::IdentityLow.offset());
            (
                ((packed >> 32) & 0xFFFF) as u32,
                ((packed >> 16) & 0xFFFF) as u32,
                (packed & 0xFF) as u32,
                Some(packed),
                Some(u32_at(
                    payload,
                    at + DealRowField::OrdinalOrReserved.offset(),
                )),
            )
        }
    };
    DealRow {
        owner_tag_db_id: u32_at(payload, at + DealRowField::OwnerTagDbId.offset()),
        counterpart_tag_db_id: u32_at(payload, at + DealRowField::CounterpartTagDbId.offset()),
        resource_index: u32_at(payload, at + DealRowField::ResourceIndex.offset()),
        visible_order: u32_at(payload, at + DealRowField::VisibleOrder.offset()),
        direction: u32_at(payload, at + DealRowField::Direction.offset()),
        route_kind: u32_at(payload, at + DealRowField::RouteKind.offset()),
        flags: u32_at(payload, at + DealRowField::Flags.offset()),
        route_status_or_reserved: u32_at(
            payload,
            at + DealRowField::RouteStatusOrReserved.offset(),
        ),
        identity_exporter_tag_db_id: exporter,
        identity_importer_tag_db_id: importer,
        identity_ordinal: ordinal,
        packed_identity: packed,
        reserved_ordinal_word: reserved_ordinal,
        requested_raw: i64_at(payload, at + DealRowField::RequestedRaw.offset()),
        delivered_raw: i64_at(payload, at + DealRowField::DeliveredRaw.offset()),
        available_export_raw: i64_at(payload, at + DealRowField::AvailableExportRaw.offset()),
        factory_cost_raw: i64_at(payload, at + DealRowField::FactoryCostRaw.offset()),
        efficiency_raw: i64_at(payload, at + DealRowField::EfficiencyRaw.offset()),
        civilian_factories: i32_at(payload, at + DealRowField::CivilianFactories.offset()),
        convoys_assigned: i32_at(payload, at + DealRowField::ConvoysAssigned.offset()),
        convoys_required: i32_at(payload, at + DealRowField::ConvoysRequired.offset()),
    }
}

pub fn packed_identity_resource(packed: u64) -> u32 {
    ((packed >> 8) & 0xFF) as u32
}

pub fn packed_identity_reserved(packed: u64) -> u64 {
    packed >> 48
}

pub fn owner_word_at(payload: &[u8], at: usize) -> u32 {
    u32_at(payload, at)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StockpileCountryField {
    TagDbId,
    ItemFirst,
    ItemCount,
    Flags,
    Reserved,
}

impl StockpileCountryField {
    pub const ALL: [StockpileCountryField; 5] = [
        StockpileCountryField::TagDbId,
        StockpileCountryField::ItemFirst,
        StockpileCountryField::ItemCount,
        StockpileCountryField::Flags,
        StockpileCountryField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            StockpileCountryField::TagDbId => 0,
            StockpileCountryField::ItemFirst => 4,
            StockpileCountryField::ItemCount => 8,
            StockpileCountryField::Flags => 12,
            StockpileCountryField::Reserved => 16,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            StockpileCountryField::TagDbId
            | StockpileCountryField::ItemFirst
            | StockpileCountryField::ItemCount
            | StockpileCountryField::Flags
            | StockpileCountryField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StockpileItemField {
    EquipmentVariantId,
    VariantType,
    Ordinal,
    Reserved,
    AmountRaw,
}

impl StockpileItemField {
    pub const ALL: [StockpileItemField; 5] = [
        StockpileItemField::EquipmentVariantId,
        StockpileItemField::VariantType,
        StockpileItemField::Ordinal,
        StockpileItemField::Reserved,
        StockpileItemField::AmountRaw,
    ];

    pub const fn offset(self) -> usize {
        match self {
            StockpileItemField::EquipmentVariantId => 0,
            StockpileItemField::VariantType => 4,
            StockpileItemField::Ordinal => 8,
            StockpileItemField::Reserved => 12,
            StockpileItemField::AmountRaw => 16,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            StockpileItemField::AmountRaw => Width::I64,
            StockpileItemField::EquipmentVariantId
            | StockpileItemField::VariantType
            | StockpileItemField::Ordinal
            | StockpileItemField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum StockpileGlobalFlag {
    OwnerIteratorComplete,
    NestedIteratorsComplete,
}

impl StockpileGlobalFlag {
    pub const ALL: [StockpileGlobalFlag; 2] = [
        StockpileGlobalFlag::OwnerIteratorComplete,
        StockpileGlobalFlag::NestedIteratorsComplete,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            StockpileGlobalFlag::OwnerIteratorComplete => 1 << 0,
            StockpileGlobalFlag::NestedIteratorsComplete => 1 << 1,
        }
    }
}

pub const STOCKPILE_GLOBAL_FLAGS: u32 = known_stockpile_global_flags();

pub const KNOWN_STOCKPILE_GLOBAL_FLAGS: u32 = known_stockpile_global_flags();

const fn known_stockpile_global_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < StockpileGlobalFlag::ALL.len() {
        mask |= StockpileGlobalFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

pub const STOCKPILE_COUNTRY_COMPLETE: u32 = 1;

pub const STOCKPILE_EQUIPMENT_TYPE: u32 = 70;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StockpileCountryRow {
    pub tag_db_id: u32,
    pub item_first: usize,
    pub item_count: usize,
    pub flags: u32,
    pub reserved: u32,
}

pub fn stockpile_country_row_at(payload: &[u8], at: usize) -> StockpileCountryRow {
    StockpileCountryRow {
        tag_db_id: u32_at(payload, at + StockpileCountryField::TagDbId.offset()),
        item_first: u32_at(payload, at + StockpileCountryField::ItemFirst.offset()) as usize,
        item_count: u32_at(payload, at + StockpileCountryField::ItemCount.offset()) as usize,
        flags: u32_at(payload, at + StockpileCountryField::Flags.offset()),
        reserved: u32_at(payload, at + StockpileCountryField::Reserved.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StockpileItemRow {
    pub equipment_variant_id: u32,
    pub variant_type: u32,
    pub ordinal: u32,
    pub reserved: u32,
    pub amount_raw: i64,
}

pub fn stockpile_item_row_at(payload: &[u8], at: usize) -> StockpileItemRow {
    StockpileItemRow {
        equipment_variant_id: u32_at(
            payload,
            at + StockpileItemField::EquipmentVariantId.offset(),
        ),
        variant_type: u32_at(payload, at + StockpileItemField::VariantType.offset()),
        ordinal: u32_at(payload, at + StockpileItemField::Ordinal.offset()),
        reserved: u32_at(payload, at + StockpileItemField::Reserved.offset()),
        amount_raw: i64_at(payload, at + StockpileItemField::AmountRaw.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResidualRowField {
    GapId,
    Status,
    EvidenceMask,
    CurrentSupport,
    CurrentDomainId,
    ProposedRecordKind,
    CurrentRowLen,
    SourceRefId,
    OwnerBoundary,
    RootStatus,
    ProofStatus,
    Reserved,
}

impl ResidualRowField {
    pub const ALL: [ResidualRowField; 12] = [
        ResidualRowField::GapId,
        ResidualRowField::Status,
        ResidualRowField::EvidenceMask,
        ResidualRowField::CurrentSupport,
        ResidualRowField::CurrentDomainId,
        ResidualRowField::ProposedRecordKind,
        ResidualRowField::CurrentRowLen,
        ResidualRowField::SourceRefId,
        ResidualRowField::OwnerBoundary,
        ResidualRowField::RootStatus,
        ResidualRowField::ProofStatus,
        ResidualRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            ResidualRowField::GapId => 0,
            ResidualRowField::Status => 4,
            ResidualRowField::EvidenceMask => 8,
            ResidualRowField::CurrentSupport => 12,
            ResidualRowField::CurrentDomainId => 16,
            ResidualRowField::ProposedRecordKind => 20,
            ResidualRowField::CurrentRowLen => 24,
            ResidualRowField::SourceRefId => 28,
            ResidualRowField::OwnerBoundary => 32,
            ResidualRowField::RootStatus => 36,
            ResidualRowField::ProofStatus => 40,
            ResidualRowField::Reserved => 44,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            ResidualRowField::GapId
            | ResidualRowField::Status
            | ResidualRowField::EvidenceMask
            | ResidualRowField::CurrentSupport
            | ResidualRowField::CurrentDomainId
            | ResidualRowField::ProposedRecordKind
            | ResidualRowField::CurrentRowLen
            | ResidualRowField::SourceRefId
            | ResidualRowField::OwnerBoundary
            | ResidualRowField::RootStatus
            | ResidualRowField::ProofStatus
            | ResidualRowField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LendLeaseRowField {
    ContractId,
    PerspectiveTagDbId,
    CounterpartTagDbId,
    Direction,
    EquipmentVariantId,
    Flags,
    AmountRaw,
    PriceRaw,
    SourceRecordKind,
    Reserved,
}

impl LendLeaseRowField {
    pub const ALL: [LendLeaseRowField; 10] = [
        LendLeaseRowField::ContractId,
        LendLeaseRowField::PerspectiveTagDbId,
        LendLeaseRowField::CounterpartTagDbId,
        LendLeaseRowField::Direction,
        LendLeaseRowField::EquipmentVariantId,
        LendLeaseRowField::Flags,
        LendLeaseRowField::AmountRaw,
        LendLeaseRowField::PriceRaw,
        LendLeaseRowField::SourceRecordKind,
        LendLeaseRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            LendLeaseRowField::ContractId => 0,
            LendLeaseRowField::PerspectiveTagDbId => 4,
            LendLeaseRowField::CounterpartTagDbId => 8,
            LendLeaseRowField::Direction => 12,
            LendLeaseRowField::EquipmentVariantId => 16,
            LendLeaseRowField::Flags => 20,
            LendLeaseRowField::AmountRaw => 24,
            LendLeaseRowField::PriceRaw => 32,
            LendLeaseRowField::SourceRecordKind => 40,
            LendLeaseRowField::Reserved => 44,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            LendLeaseRowField::AmountRaw | LendLeaseRowField::PriceRaw => Width::I64,
            LendLeaseRowField::ContractId
            | LendLeaseRowField::PerspectiveTagDbId
            | LendLeaseRowField::CounterpartTagDbId
            | LendLeaseRowField::Direction
            | LendLeaseRowField::EquipmentVariantId
            | LendLeaseRowField::Flags
            | LendLeaseRowField::SourceRecordKind
            | LendLeaseRowField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidualRow {
    pub gap_id: u32,
    pub status: u32,
    pub evidence_mask: u32,
    pub current_support: u32,
    pub current_domain_id: u32,
    pub proposed_record_kind: u32,
    pub current_row_len: u32,
    pub source_ref_id: u32,
    pub owner_boundary: u32,
    pub root_status: u32,
    pub proof_status: u32,
    pub reserved: u32,
}

pub fn residual_row_at(payload: &[u8], at: usize) -> ResidualRow {
    ResidualRow {
        gap_id: u32_at(payload, at + ResidualRowField::GapId.offset()),
        status: u32_at(payload, at + ResidualRowField::Status.offset()),
        evidence_mask: u32_at(payload, at + ResidualRowField::EvidenceMask.offset()),
        current_support: u32_at(payload, at + ResidualRowField::CurrentSupport.offset()),
        current_domain_id: u32_at(payload, at + ResidualRowField::CurrentDomainId.offset()),
        proposed_record_kind: u32_at(payload, at + ResidualRowField::ProposedRecordKind.offset()),
        current_row_len: u32_at(payload, at + ResidualRowField::CurrentRowLen.offset()),
        source_ref_id: u32_at(payload, at + ResidualRowField::SourceRefId.offset()),
        owner_boundary: u32_at(payload, at + ResidualRowField::OwnerBoundary.offset()),
        root_status: u32_at(payload, at + ResidualRowField::RootStatus.offset()),
        proof_status: u32_at(payload, at + ResidualRowField::ProofStatus.offset()),
        reserved: u32_at(payload, at + ResidualRowField::Reserved.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LendLeaseRow {
    pub contract_id: u32,
    pub perspective_tag_db_id: u32,
    pub counterpart_tag_db_id: u32,
    pub direction: u32,
    pub equipment_variant_id: u32,
    pub flags: u32,
    pub amount_raw: i64,
    pub price_raw: i64,
    pub source_record_kind: u32,
    pub reserved: u32,
}

pub fn lend_lease_row_at(payload: &[u8], at: usize) -> LendLeaseRow {
    LendLeaseRow {
        contract_id: u32_at(payload, at + LendLeaseRowField::ContractId.offset()),
        perspective_tag_db_id: u32_at(payload, at + LendLeaseRowField::PerspectiveTagDbId.offset()),
        counterpart_tag_db_id: u32_at(payload, at + LendLeaseRowField::CounterpartTagDbId.offset()),
        direction: u32_at(payload, at + LendLeaseRowField::Direction.offset()),
        equipment_variant_id: u32_at(payload, at + LendLeaseRowField::EquipmentVariantId.offset()),
        flags: u32_at(payload, at + LendLeaseRowField::Flags.offset()),
        amount_raw: i64_at(payload, at + LendLeaseRowField::AmountRaw.offset()),
        price_raw: i64_at(payload, at + LendLeaseRowField::PriceRaw.offset()),
        source_record_kind: u32_at(payload, at + LendLeaseRowField::SourceRecordKind.offset()),
        reserved: u32_at(payload, at + LendLeaseRowField::Reserved.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum LendLeaseDirection {
    Outgoing,
    Incoming,
}

impl LendLeaseDirection {
    pub const ALL: [LendLeaseDirection; 2] =
        [LendLeaseDirection::Outgoing, LendLeaseDirection::Incoming];

    pub const fn wire(self) -> u32 {
        match self {
            LendLeaseDirection::Outgoing => 1,
            LendLeaseDirection::Incoming => 2,
        }
    }

    /// This taxonomy had no decoder, so `decode.rs` matched the raw `1`/`2` a
    /// third time and a moved code left the counters on the other half of the
    /// contract (lane TV, row R3).
    pub(crate) const fn from_wire(value: u32) -> Option<LendLeaseDirection> {
        let mut i = 0;
        while i < LendLeaseDirection::ALL.len() {
            if LendLeaseDirection::ALL[i].wire() == value {
                return Some(LendLeaseDirection::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum LendLeaseFlag {
    AmountValid,
    PriceValid,
}

impl LendLeaseFlag {
    pub const ALL: [LendLeaseFlag; 2] = [LendLeaseFlag::AmountValid, LendLeaseFlag::PriceValid];

    pub const fn bit(self) -> u32 {
        match self {
            LendLeaseFlag::AmountValid => 1 << 0,
            LendLeaseFlag::PriceValid => 1 << 1,
        }
    }
}

pub const KNOWN_LEND_LEASE_FLAGS: u32 = known_lend_lease_flags();

const fn known_lend_lease_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < LendLeaseFlag::ALL.len() {
        mask |= LendLeaseFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

const fn tiles(offsets: &[usize], widths: &[Width], row_len: usize) -> bool {
    let mut expected = 0;
    let mut i = 0;
    while i < offsets.len() {
        if offsets[i] != expected {
            return false;
        }
        expected += widths[i].bytes();
        i += 1;
    }
    expected == row_len
}

const _: () = {
    let mut offsets = [0usize; 14];
    let mut widths = [Width::U32; 14];
    let mut i = 0;
    while i < DetailRowField::ALL.len() {
        offsets[i] = DetailRowField::ALL[i].offset();
        widths[i] = DetailRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::Detail)) {
        panic!("d43_grammar: DetailRowField::ALL does not tile the MTD1 row");
    }
};

const _: () = {
    let mut offsets = [0usize; 4];
    let mut widths = [Width::U32; 4];
    let mut i = 0;
    while i < FactoryRowField::ALL.len() {
        offsets[i] = FactoryRowField::ALL[i].offset();
        widths[i] = FactoryRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::FactoryHeader)) {
        panic!("d43_grammar: FactoryRowField::ALL does not tile the MTRH row");
    }
};

const _: () = {
    let mut offsets = [0usize; 4];
    let mut widths = [Width::U32; 4];
    let mut i = 0;
    while i < FlowRowField::ALL.len() {
        offsets[i] = FlowRowField::ALL[i].offset();
        widths[i] = FlowRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, FLOW_ROW_HEADER_LEN) {
        panic!("d43_grammar: FlowRowField::ALL does not tile the MTRF row header");
    }
};

const _: () = {
    let mut offsets = [0usize; 16];
    let mut widths = [Width::U32; 16];
    let mut i = 0;
    while i < ResourceRowField::ALL.len() {
        offsets[i] = ResourceRowField::ALL[i].offset();
        widths[i] = ResourceRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::ResourceRow)) {
        panic!("d43_grammar: ResourceRowField::ALL does not tile the MTRR row");
    }
};

const _: () = {
    let mut offsets = [0usize; 19];
    let mut widths = [Width::U32; 19];
    let mut i = 0;
    while i < DealRowField::ALL.len() {
        offsets[i] = DealRowField::ALL[i].offset();
        widths[i] = DealRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::ClassicDeal)) {
        panic!("d43_grammar: DealRowField::ALL does not tile the MTRD row");
    }
};

const _: () = {
    let mut offsets = [0usize; 5];
    let mut widths = [Width::U32; 5];
    let mut i = 0;
    while i < StockpileCountryField::ALL.len() {
        offsets[i] = StockpileCountryField::ALL[i].offset();
        widths[i] = StockpileCountryField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, STOCKPILE_COUNTRY_ROW_LEN) {
        panic!("d43_grammar: StockpileCountryField::ALL does not tile the MTS1 country row");
    }
    let mut offsets = [0usize; 5];
    let mut widths = [Width::U32; 5];
    let mut i = 0;
    while i < StockpileItemField::ALL.len() {
        offsets[i] = StockpileItemField::ALL[i].offset();
        widths[i] = StockpileItemField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, STOCKPILE_ITEM_ROW_LEN) {
        panic!("d43_grammar: StockpileItemField::ALL does not tile the MTS1 item row");
    }
};

const _: () = {
    let mut offsets = [0usize; 12];
    let mut widths = [Width::U32; 12];
    let mut i = 0;
    while i < ResidualRowField::ALL.len() {
        offsets[i] = ResidualRowField::ALL[i].offset();
        widths[i] = ResidualRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::Residual)) {
        panic!("d43_grammar: ResidualRowField::ALL does not tile the MTRB row");
    }
};

const _: () = {
    let mut offsets = [0usize; 10];
    let mut widths = [Width::U32; 10];
    let mut i = 0;
    while i < LendLeaseRowField::ALL.len() {
        offsets[i] = LendLeaseRowField::ALL[i].offset();
        widths[i] = LendLeaseRowField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, fixed_row_len(Trailer::LendLease)) {
        panic!("d43_grammar: LendLeaseRowField::ALL does not tile the MTLL row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderPrefix {
    pub at: usize,
    pub trailer: Trailer,
    pub version: u16,
    pub header_len: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SizedHeaderWords {
    pub prefix: HeaderPrefix,
    pub row_len: u16,
    pub reserved0: u16,
    pub row_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetailHeaderWords {
    pub sized: SizedHeaderWords,
    pub price_valid_rows: u32,
    pub speed_valid_rows: u32,
    pub delivery_route_valid_rows: u32,
    pub progress_valid_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlowHeaderWords {
    pub sized: SizedHeaderWords,
    pub country_count_or_exported: u32,
    pub scale_raw: u32,
    pub mask_or_count: u32,
    pub kind_mask: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FactoryHeaderWords {
    pub sized: SizedHeaderWords,
    pub country_count: u32,
    pub reserved1: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRowHeaderWords {
    pub sized: SizedHeaderWords,
    pub country_count: u32,
    pub resource_count: u32,
    pub display_scale: u32,
    pub identity_stride_or_reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DealHeaderWords {
    pub sized: SizedHeaderWords,
    pub country_count: u32,
    pub scale_raw: u32,
    pub witness_count: u32,
    pub incomplete_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivationHeaderWords {
    pub prefix: HeaderPrefix,
    pub flags: u32,
    pub country_tag_db_id: u32,
    pub observation_sequence: u64,
    pub root_kind: u32,
    pub open_rva: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StockpileHeaderWords {
    pub prefix: HeaderPrefix,
    pub country_row_len: u16,
    pub item_row_len: u16,
    pub declared_segment_len: u32,
    pub country_count: u32,
    pub item_count: u32,
    pub expected_country_count: u32,
    pub scale_raw: u32,
    pub flags: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidualHeaderWords {
    pub sized: SizedHeaderWords,
    pub source_blocked_rows: u32,
    pub schema_blocked_rows: u32,
    pub owned_elsewhere_rows: u32,
    pub shared_patch_blocked_rows: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LendLeaseHeaderWords {
    pub sized: SizedHeaderWords,
    pub outgoing_rows: u32,
    pub incoming_rows: u32,
    pub amount_valid_rows: u32,
    pub price_valid_rows: u32,
}

/// Generated from [`Trailer::ALL`] by an exhaustive `match` with no wildcard
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderPreview {
    Detail(DetailHeaderWords),
    ResourceFlow(FlowHeaderWords),
    FactoryHeader(FactoryHeaderWords),
    ResourceRow(ResourceRowHeaderWords),
    ClassicDeal(DealHeaderWords),
    Activation(ActivationHeaderWords),
    Stockpile(StockpileHeaderWords),
    Residual(ResidualHeaderWords),
    LendLease(LendLeaseHeaderWords),
}

impl HeaderPreview {
    pub const fn prefix(&self) -> HeaderPrefix {
        match self {
            HeaderPreview::Detail(w) => w.sized.prefix,
            HeaderPreview::ResourceFlow(w) => w.sized.prefix,
            HeaderPreview::FactoryHeader(w) => w.sized.prefix,
            HeaderPreview::ResourceRow(w) => w.sized.prefix,
            HeaderPreview::ClassicDeal(w) => w.sized.prefix,
            HeaderPreview::Activation(w) => w.prefix,
            HeaderPreview::Stockpile(w) => w.prefix,
            HeaderPreview::Residual(w) => w.sized.prefix,
            HeaderPreview::LendLease(w) => w.sized.prefix,
        }
    }
}

fn header_prefix(payload: &[u8], at: usize, trailer: Trailer) -> HeaderPrefix {
    HeaderPrefix {
        at,
        trailer,
        version: u16_at(payload, at + TRAILER_VERSION_OFFSET),
        header_len: u16_at(payload, at + TRAILER_HEADER_LEN_OFFSET),
    }
}

fn sized_header_words(payload: &[u8], at: usize, trailer: Trailer) -> SizedHeaderWords {
    SizedHeaderWords {
        prefix: header_prefix(payload, at, trailer),
        row_len: u16_at(payload, at + TRAILER_ROW_LEN_OFFSET),
        reserved0: u16_at(payload, at + TRAILER_RESERVED0_OFFSET),
        row_count: u32_at(payload, at + TRAILER_ROW_COUNT_OFFSET),
    }
}

pub(crate) fn read_header_preview(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
) -> Result<HeaderPreview, Violation> {
    // The SAME bounds check `read_trailer_body` opens with, and the same
    let header_end = add(Section::TrailerHeader(trailer), at, trailer.header_len())?;
    if header_end > payload.len() {
        return Err(Violation::new(
            at,
            ViolationKind::Need {
                section: Section::TrailerHeader(trailer),
                needed: header_end,
                available: payload.len(),
            },
        ));
    }
    Ok(match trailer {
        Trailer::Detail => HeaderPreview::Detail(DetailHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            price_valid_rows: u32_at(payload, at + 16),
            speed_valid_rows: u32_at(payload, at + 20),
            delivery_route_valid_rows: u32_at(payload, at + 24),
            progress_valid_rows: u32_at(payload, at + 28),
        }),
        Trailer::ResourceFlow => HeaderPreview::ResourceFlow(FlowHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            country_count_or_exported: u32_at(payload, at + 16),
            scale_raw: u32_at(payload, at + 20),
            mask_or_count: u32_at(payload, at + 24),
            kind_mask: u32_at(payload, at + 28),
        }),
        Trailer::FactoryHeader => HeaderPreview::FactoryHeader(FactoryHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            country_count: u32_at(payload, at + 16),
            reserved1: u32_at(payload, at + 20),
        }),
        Trailer::ResourceRow => HeaderPreview::ResourceRow(ResourceRowHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            country_count: u32_at(payload, at + 16),
            resource_count: u32_at(payload, at + 20),
            display_scale: u32_at(payload, at + 24),
            identity_stride_or_reserved: u32_at(payload, at + 28),
        }),
        Trailer::ClassicDeal => HeaderPreview::ClassicDeal(DealHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            country_count: u32_at(payload, at + 16),
            scale_raw: u32_at(payload, at + 20),
            witness_count: u32_at(payload, at + 24),
            incomplete_count: u32_at(payload, at + 28),
        }),
        Trailer::Activation => HeaderPreview::Activation(ActivationHeaderWords {
            prefix: header_prefix(payload, at, trailer),
            flags: u32_at(payload, at + 8),
            country_tag_db_id: u32_at(payload, at + 12),
            observation_sequence: u64_at(payload, at + 16),
            root_kind: u32_at(payload, at + 24),
            open_rva: u32_at(payload, at + 28),
        }),
        Trailer::Stockpile => HeaderPreview::Stockpile(StockpileHeaderWords {
            prefix: header_prefix(payload, at, trailer),
            country_row_len: u16_at(payload, at + TRAILER_ROW_LEN_OFFSET),
            item_row_len: u16_at(payload, at + TRAILER_RESERVED0_OFFSET),
            declared_segment_len: u32_at(payload, at + 12),
            country_count: u32_at(payload, at + 16),
            item_count: u32_at(payload, at + 20),
            expected_country_count: u32_at(payload, at + 24),
            scale_raw: u32_at(payload, at + 28),
            flags: u32_at(payload, at + 32),
            reserved: u32_at(payload, at + 36),
        }),
        Trailer::Residual => HeaderPreview::Residual(ResidualHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            source_blocked_rows: u32_at(payload, at + 16),
            schema_blocked_rows: u32_at(payload, at + 20),
            owned_elsewhere_rows: u32_at(payload, at + 24),
            shared_patch_blocked_rows: u32_at(payload, at + 28),
        }),
        Trailer::LendLease => HeaderPreview::LendLease(LendLeaseHeaderWords {
            sized: sized_header_words(payload, at, trailer),
            outgoing_rows: u32_at(payload, at + 16),
            incoming_rows: u32_at(payload, at + 20),
            amount_valid_rows: u32_at(payload, at + 24),
            price_valid_rows: u32_at(payload, at + 28),
        }),
    })
}
