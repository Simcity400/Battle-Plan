use alloc::vec::Vec;
use core::convert::TryFrom;

use crate::{Span, Width};

pub const HEADER_LEN: usize = 40;
pub const COUNTRY_ROW_LEN: usize = 16;
pub const CONVEYOR_ROW_LEN: usize = 40;
pub const LINE_ROW_LEN: usize = 72;
pub const EQUIPMENT_ROW_LEN: usize = 24;
pub const STRING_PREFIX_LEN: usize = 4;

pub const CONVEYOR_ID_TYPE: u32 = 64;
pub const LINE_ID_TYPE: u32 = 63;
pub const TEMPLATE_ID_TYPE: u32 = 52;
pub const EQUIPMENT_ID_TYPE: u32 = 70;

// The field and flag tables are written out rather than generated.
// width and mask below is still generated FROM the table by an exhaustive

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum HeaderField {
    ElapsedWallMs,
    CountryCount,
    ChangedCountryRows,
    StatusFlags,
    SchemaVersion,
    DeclaredConveyors,
    DeclaredLines,
    DeclaredEquipment,
    Reserved,
}

impl HeaderField {
    pub const ALL: [HeaderField; 9] = [
        HeaderField::ElapsedWallMs,
        HeaderField::CountryCount,
        HeaderField::ChangedCountryRows,
        HeaderField::StatusFlags,
        HeaderField::SchemaVersion,
        HeaderField::DeclaredConveyors,
        HeaderField::DeclaredLines,
        HeaderField::DeclaredEquipment,
        HeaderField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            HeaderField::ElapsedWallMs => 0,
            HeaderField::CountryCount => 8,
            HeaderField::ChangedCountryRows => 12,
            HeaderField::StatusFlags => 16,
            HeaderField::SchemaVersion => 20,
            HeaderField::DeclaredConveyors => 24,
            HeaderField::DeclaredLines => 28,
            HeaderField::DeclaredEquipment => 32,
            HeaderField::Reserved => 36,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            HeaderField::ElapsedWallMs => Width::U64,
            HeaderField::CountryCount => Width::U32,
            HeaderField::ChangedCountryRows => Width::U32,
            HeaderField::StatusFlags => Width::U32,
            HeaderField::SchemaVersion => Width::U32,
            HeaderField::DeclaredConveyors => Width::U32,
            HeaderField::DeclaredLines => Width::U32,
            HeaderField::DeclaredEquipment => Width::U32,
            HeaderField::Reserved => Width::U32,
        }
    }
}

const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < HeaderField::ALL.len() {
        let field = HeaderField::ALL[i];
        if field.offset() != cursor {
            panic!("d11 deployment: HeaderField has a gap or overlap");
        }
        cursor += field.width().bytes();
        i += 1;
    }
    if cursor != HEADER_LEN {
        panic!("d11 deployment: HeaderField does not tile its row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CountryField {
    TagDbId,
    Flags,
    ConveyorCount,
    Reserved,
}

impl CountryField {
    pub const ALL: [CountryField; 4] = [
        CountryField::TagDbId,
        CountryField::Flags,
        CountryField::ConveyorCount,
        CountryField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            CountryField::TagDbId => 0,
            CountryField::Flags => 4,
            CountryField::ConveyorCount => 8,
            CountryField::Reserved => 12,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            CountryField::TagDbId => Width::U32,
            CountryField::Flags => Width::U32,
            CountryField::ConveyorCount => Width::U32,
            CountryField::Reserved => Width::U32,
        }
    }
}

const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < CountryField::ALL.len() {
        let field = CountryField::ALL[i];
        if field.offset() != cursor {
            panic!("d11 deployment: CountryField has a gap or overlap");
        }
        cursor += field.width().bytes();
        i += 1;
    }
    if cursor != COUNTRY_ROW_LEN {
        panic!("d11 deployment: CountryField does not tile its row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ConveyorField {
    Ordinal,
    ConveyorId,
    IdType,
    TemplateId,
    TemplateIdType,
    Amount,
    Location,
    Priority,
    Flags,
    LineCount,
}

impl ConveyorField {
    pub const ALL: [ConveyorField; 10] = [
        ConveyorField::Ordinal,
        ConveyorField::ConveyorId,
        ConveyorField::IdType,
        ConveyorField::TemplateId,
        ConveyorField::TemplateIdType,
        ConveyorField::Amount,
        ConveyorField::Location,
        ConveyorField::Priority,
        ConveyorField::Flags,
        ConveyorField::LineCount,
    ];

    pub const fn offset(self) -> usize {
        match self {
            ConveyorField::Ordinal => 0,
            ConveyorField::ConveyorId => 4,
            ConveyorField::IdType => 8,
            ConveyorField::TemplateId => 12,
            ConveyorField::TemplateIdType => 16,
            ConveyorField::Amount => 20,
            ConveyorField::Location => 24,
            ConveyorField::Priority => 28,
            ConveyorField::Flags => 32,
            ConveyorField::LineCount => 36,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            ConveyorField::Ordinal => Width::U32,
            ConveyorField::ConveyorId => Width::U32,
            ConveyorField::IdType => Width::U32,
            ConveyorField::TemplateId => Width::U32,
            ConveyorField::TemplateIdType => Width::U32,
            ConveyorField::Amount => Width::I32,
            ConveyorField::Location => Width::U32,
            ConveyorField::Priority => Width::U32,
            ConveyorField::Flags => Width::U32,
            ConveyorField::LineCount => Width::U32,
        }
    }
}

const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < ConveyorField::ALL.len() {
        let field = ConveyorField::ALL[i];
        if field.offset() != cursor {
            panic!("d11 deployment: ConveyorField has a gap or overlap");
        }
        cursor += field.width().bytes();
        i += 1;
    }
    if cursor != CONVEYOR_ROW_LEN {
        panic!("d11 deployment: ConveyorField does not tile its row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DqLineField {
    Ordinal,
    LineId,
    IdType,
    NameOrder,
    Amount,
    Flags,
    TrainingBits,
    MaxTrainingBits,
    ManpowerHave,
    ManpowerNeed,
    MaxManpower,
    EquipmentCount,
    Reserved,
}

impl DqLineField {
    pub const ALL: [DqLineField; 13] = [
        DqLineField::Ordinal,
        DqLineField::LineId,
        DqLineField::IdType,
        DqLineField::NameOrder,
        DqLineField::Amount,
        DqLineField::Flags,
        DqLineField::TrainingBits,
        DqLineField::MaxTrainingBits,
        DqLineField::ManpowerHave,
        DqLineField::ManpowerNeed,
        DqLineField::MaxManpower,
        DqLineField::EquipmentCount,
        DqLineField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            DqLineField::Ordinal => 0,
            DqLineField::LineId => 4,
            DqLineField::IdType => 8,
            DqLineField::NameOrder => 12,
            DqLineField::Amount => 16,
            DqLineField::Flags => 20,
            DqLineField::TrainingBits => 24,
            DqLineField::MaxTrainingBits => 32,
            DqLineField::ManpowerHave => 40,
            DqLineField::ManpowerNeed => 48,
            DqLineField::MaxManpower => 56,
            DqLineField::EquipmentCount => 64,
            DqLineField::Reserved => 68,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            DqLineField::Ordinal => Width::U32,
            DqLineField::LineId => Width::U32,
            DqLineField::IdType => Width::U32,
            DqLineField::NameOrder => Width::I32,
            DqLineField::Amount => Width::I32,
            DqLineField::Flags => Width::U32,
            DqLineField::TrainingBits => Width::U64,
            DqLineField::MaxTrainingBits => Width::U64,
            DqLineField::ManpowerHave => Width::I64,
            DqLineField::ManpowerNeed => Width::I64,
            DqLineField::MaxManpower => Width::I64,
            DqLineField::EquipmentCount => Width::U32,
            DqLineField::Reserved => Width::U32,
        }
    }
}

const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < DqLineField::ALL.len() {
        let field = DqLineField::ALL[i];
        if field.offset() != cursor {
            panic!("d11 deployment: DqLineField has a gap or overlap");
        }
        cursor += field.width().bytes();
        i += 1;
    }
    if cursor != LINE_ROW_LEN {
        panic!("d11 deployment: DqLineField does not tile its row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EquipmentField {
    Ordinal,
    EquipmentId,
    IdType,
    Flags,
    AmountBits,
}

impl EquipmentField {
    pub const ALL: [EquipmentField; 5] = [
        EquipmentField::Ordinal,
        EquipmentField::EquipmentId,
        EquipmentField::IdType,
        EquipmentField::Flags,
        EquipmentField::AmountBits,
    ];

    pub const fn offset(self) -> usize {
        match self {
            EquipmentField::Ordinal => 0,
            EquipmentField::EquipmentId => 4,
            EquipmentField::IdType => 8,
            EquipmentField::Flags => 12,
            EquipmentField::AmountBits => 16,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            EquipmentField::Ordinal => Width::U32,
            EquipmentField::EquipmentId => Width::U32,
            EquipmentField::IdType => Width::U32,
            EquipmentField::Flags => Width::U32,
            EquipmentField::AmountBits => Width::U64,
        }
    }
}

const _: () = {
    let mut cursor = 0usize;
    let mut i = 0usize;
    while i < EquipmentField::ALL.len() {
        let field = EquipmentField::ALL[i];
        if field.offset() != cursor {
            panic!("d11 deployment: EquipmentField has a gap or overlap");
        }
        cursor += field.width().bytes();
        i += 1;
    }
    if cursor != EQUIPMENT_ROW_LEN {
        panic!("d11 deployment: EquipmentField does not tile its row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CountryFlag {
    QueueComplete,
}

impl CountryFlag {
    pub const ALL: [CountryFlag; 1] = [CountryFlag::QueueComplete];

    pub const fn bit(self) -> u32 {
        match self {
            CountryFlag::QueueComplete => 1,
        }
    }
}

pub const COUNTRY_KNOWN_FLAGS: u32 = {
    let mut mask = 0u32;
    let mut i = 0usize;
    while i < CountryFlag::ALL.len() {
        mask |= CountryFlag::ALL[i].bit();
        i += 1;
    }
    mask
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ConveyorFlag {
    Closed,
}

impl ConveyorFlag {
    pub const ALL: [ConveyorFlag; 1] = [ConveyorFlag::Closed];

    pub const fn bit(self) -> u32 {
        match self {
            ConveyorFlag::Closed => 1,
        }
    }
}

pub const CONVEYOR_KNOWN_FLAGS: u32 = {
    let mut mask = 0u32;
    let mut i = 0usize;
    while i < ConveyorFlag::ALL.len() {
        mask |= ConveyorFlag::ALL[i].bit();
        i += 1;
    }
    mask
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum LineFlag {
    Training,
    MaxTraining,
    ManpowerHave,
    ManpowerNeed,
    MaxManpower,
    NameOrder,
}

impl LineFlag {
    pub const ALL: [LineFlag; 6] = [
        LineFlag::Training,
        LineFlag::MaxTraining,
        LineFlag::ManpowerHave,
        LineFlag::ManpowerNeed,
        LineFlag::MaxManpower,
        LineFlag::NameOrder,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            LineFlag::Training => 1,
            LineFlag::MaxTraining => 1 << 1,
            LineFlag::ManpowerHave => 1 << 2,
            LineFlag::ManpowerNeed => 1 << 3,
            LineFlag::MaxManpower => 1 << 4,
            LineFlag::NameOrder => 1 << 5,
        }
    }
}

pub const LINE_KNOWN_FLAGS: u32 = {
    let mut mask = 0u32;
    let mut i = 0usize;
    while i < LineFlag::ALL.len() {
        mask |= LineFlag::ALL[i].bit();
        i += 1;
    }
    mask
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EquipmentFlag {
    Archetype,
}

impl EquipmentFlag {
    pub const ALL: [EquipmentFlag; 1] = [EquipmentFlag::Archetype];

    pub const fn bit(self) -> u32 {
        match self {
            EquipmentFlag::Archetype => 1,
        }
    }
}

pub const EQUIPMENT_KNOWN_FLAGS: u32 = {
    let mut mask = 0u32;
    let mut i = 0usize;
    while i < EquipmentFlag::ALL.len() {
        mask |= EquipmentFlag::ALL[i].bit();
        i += 1;
    }
    mask
};

const _: () = {
    if COUNTRY_KNOWN_FLAGS != 1
        || CONVEYOR_KNOWN_FLAGS != 1
        || LINE_KNOWN_FLAGS != 0b11_1111
        || EQUIPMENT_KNOWN_FLAGS != 1
    {
        panic!("d11 deployment: generated known-flag mask drifted");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum SchemaVersion {
    V1,
    V2,
}

impl SchemaVersion {
    pub const ALL: [SchemaVersion; 2] = [SchemaVersion::V1, SchemaVersion::V2];
    pub const fn wire(self) -> u32 {
        match self {
            SchemaVersion::V1 => 1,
            SchemaVersion::V2 => 2,
        }
    }
    pub const fn from_wire(value: u32) -> Option<Self> {
        match value {
            1 => Some(SchemaVersion::V1),
            2 => Some(SchemaVersion::V2),
            _ => None,
        }
    }
    pub const fn archetype_present(self, equipment_flags: u32) -> bool {
        match self {
            SchemaVersion::V1 => true,
            SchemaVersion::V2 => equipment_flags & EquipmentFlag::Archetype.bit() != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_country_rows: u32,
    pub status_flags: u32,
    pub schema_version: SchemaVersion,
    pub declared_conveyors: u32,
    pub declared_lines: u32,
    pub declared_equipment: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderPreview {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_country_rows: u32,
    pub status_flags: u32,
    pub schema_version: u32,
    pub declared_conveyors: u32,
    pub declared_lines: u32,
    pub declared_equipment: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CountryRow {
    pub tag_db_id: u32,
    pub flags: u32,
    pub conveyor_count: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConveyorRow {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub id_type: u32,
    pub template_id: u32,
    pub template_id_type: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub flags: u32,
    pub line_count: u32,
    pub name: Span,
    pub role: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConveyorFixedRow {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub id_type: u32,
    pub template_id: u32,
    pub template_id_type: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub flags: u32,
    pub line_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineRow {
    pub ordinal: u32,
    pub line_id: u32,
    pub id_type: u32,
    pub name_order: i32,
    pub amount: i32,
    pub flags: u32,
    pub training_bits: u64,
    pub max_training_bits: u64,
    pub manpower_have: i64,
    pub manpower_need: i64,
    pub max_manpower: i64,
    pub equipment_count: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EquipmentRow {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub id_type: u32,
    pub flags: u32,
    pub amount_bits: u64,
    pub archetype: Option<Span>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum DqSection {
    Header,
    Country,
    Conveyor,
    ConveyorName,
    ConveyorRole,
    Line,
    Equipment,
    Archetype,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Block {
    Countries,
    Conveyors,
    Lines,
    Equipment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DqArithmeticOp {
    Add,
    Subtract,
    UsizeConversion,
    U32Conversion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DqCallerDefect {
    BlockNotConsumed { block: Block, remaining: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DqViolationKind {
    Need {
        section: DqSection,
        needed: usize,
        available: usize,
    },
    UnknownSchema {
        version: u32,
        reserved: u32,
    },
    HeaderReserved {
        version: u32,
        value: u32,
    },
    CountCannotFit {
        block: Block,
        count: u32,
        remaining: usize,
        minimum_row_len: usize,
    },
    TrailingBytes {
        consumed: usize,
        available: usize,
    },
    ArithmeticOverflow {
        operation: DqArithmeticOp,
        left: usize,
        right: usize,
    },
    CallerDefect(DqCallerDefect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Violation {
    pub at: usize,
    pub kind: DqViolationKind,
}

fn violation(at: usize, kind: DqViolationKind) -> Violation {
    Violation { at, kind }
}

fn add(at: usize, left: usize, right: usize) -> Result<usize, Violation> {
    left.checked_add(right).ok_or_else(|| {
        violation(
            at,
            DqViolationKind::ArithmeticOverflow {
                operation: DqArithmeticOp::Add,
                left,
                right,
            },
        )
    })
}

fn sub(at: usize, left: usize, right: usize) -> Result<usize, Violation> {
    left.checked_sub(right).ok_or_else(|| {
        violation(
            at,
            DqViolationKind::ArithmeticOverflow {
                operation: DqArithmeticOp::Subtract,
                left,
                right,
            },
        )
    })
}

fn need(payload: &[u8], at: usize, section: DqSection, len: usize) -> Result<(), Violation> {
    let end = add(at, at, len)?;
    if payload.len() < end {
        return Err(violation(
            at,
            DqViolationKind::Need {
                section,
                needed: end,
                available: payload.len(),
            },
        ));
    }
    Ok(())
}

fn count_usize(at: usize, value: u32) -> Result<usize, Violation> {
    usize::try_from(value).map_err(|_| {
        violation(
            at,
            DqViolationKind::ArithmeticOverflow {
                operation: DqArithmeticOp::UsizeConversion,
                left: value as usize,
                right: usize::MAX,
            },
        )
    })
}

fn require_count_fits_remaining(
    payload: &[u8],
    at: usize,
    block: Block,
    count: u32,
    minimum_row_len: usize,
) -> Result<(), Violation> {
    let remaining = sub(at, payload.len(), at)?;
    let count_usize = count_usize(at, count)?;
    if count_usize > remaining / minimum_row_len {
        return Err(violation(
            at,
            DqViolationKind::CountCannotFit {
                block,
                count,
                remaining,
                minimum_row_len,
            },
        ));
    }
    Ok(())
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}
fn u64_at(bytes: &[u8], at: usize) -> u64 {
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
fn i64_at(bytes: &[u8], at: usize) -> i64 {
    u64_at(bytes, at) as i64
}

fn string_span(payload: &[u8], at: usize, section: DqSection) -> Result<Span, Violation> {
    need(payload, at, section, STRING_PREFIX_LEN)?;
    let len = count_usize(at, u32_at(payload, at))?;
    let start = add(at, at, STRING_PREFIX_LEN)?;
    let end = add(at, start, len)?;
    if payload.len() < end {
        return Err(violation(
            at,
            DqViolationKind::Need {
                section,
                needed: end,
                available: payload.len(),
            },
        ));
    }
    Ok(Span { start, len, end })
}

pub fn read_header_preview(payload: &[u8]) -> Result<HeaderPreview, Violation> {
    need(payload, 0, DqSection::Header, HEADER_LEN)?;
    Ok(HeaderPreview {
        elapsed_wall_ms: u64_at(payload, HeaderField::ElapsedWallMs.offset()),
        country_count: u32_at(payload, HeaderField::CountryCount.offset()),
        changed_country_rows: u32_at(payload, HeaderField::ChangedCountryRows.offset()),
        status_flags: u32_at(payload, HeaderField::StatusFlags.offset()),
        schema_version: u32_at(payload, HeaderField::SchemaVersion.offset()),
        declared_conveyors: u32_at(payload, HeaderField::DeclaredConveyors.offset()),
        declared_lines: u32_at(payload, HeaderField::DeclaredLines.offset()),
        declared_equipment: u32_at(payload, HeaderField::DeclaredEquipment.offset()),
        reserved: u32_at(payload, HeaderField::Reserved.offset()),
    })
}

pub fn read_header(payload: &[u8]) -> Result<Header, Violation> {
    let preview = read_header_preview(payload)?;
    let Some(schema_version) = SchemaVersion::from_wire(preview.schema_version) else {
        return Err(violation(
            HeaderField::SchemaVersion.offset(),
            DqViolationKind::UnknownSchema {
                version: preview.schema_version,
                reserved: preview.reserved,
            },
        ));
    };
    if preview.reserved != 0 {
        return Err(violation(
            HeaderField::Reserved.offset(),
            DqViolationKind::HeaderReserved {
                version: preview.schema_version,
                value: preview.reserved,
            },
        ));
    }
    Ok(Header {
        elapsed_wall_ms: preview.elapsed_wall_ms,
        country_count: preview.country_count,
        changed_country_rows: preview.changed_country_rows,
        status_flags: preview.status_flags,
        schema_version,
        declared_conveyors: preview.declared_conveyors,
        declared_lines: preview.declared_lines,
        declared_equipment: preview.declared_equipment,
        reserved: preview.reserved,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DelimitedTotals {
    pub countries: u32,
    pub conveyors: u32,
    pub lines: u32,
    pub equipment: u32,
}

impl DelimitedTotals {
    pub const fn matches_declared(self, header: &Header) -> bool {
        self.conveyors == header.declared_conveyors
            && self.lines == header.declared_lines
            && self.equipment == header.declared_equipment
    }
}

#[derive(Debug)]
#[must_use]
struct Rows {
    at: usize,
    index: usize,
    count: usize,
}

impl Rows {
    fn new(at: usize, count: u32) -> Result<Self, Violation> {
        Ok(Self {
            at,
            index: 0,
            count: count_usize(at, count)?,
        })
    }
    const fn remaining(&self) -> usize {
        self.count - self.index
    }
    fn advance_fixed(&mut self, at: usize) {
        self.at = at;
        self.index += 1;
    }
}

#[derive(Debug)]
#[must_use]
pub struct CountryCursor {
    header: Header,
    rows: Rows,
    totals: DelimitedTotals,
    prechecked: bool,
}

pub fn walk(payload: &[u8]) -> Result<CountryCursor, Violation> {
    let header = read_header(payload)?;
    Ok(CountryCursor {
        header,
        rows: Rows::new(HEADER_LEN, header.country_count)?,
        totals: DelimitedTotals::default(),
        prechecked: false,
    })
}

#[derive(Debug)]
#[must_use]
struct CountryResume {
    header: Header,
    rows: Rows,
    totals: DelimitedTotals,
    prechecked: bool,
}

#[derive(Debug)]
#[must_use]
pub struct CountryConveyors {
    parent: CountryResume,
    at: usize,
    count: u32,
}

#[derive(Debug)]
#[must_use]
pub struct ConveyorCursor {
    parent: CountryResume,
    rows: Rows,
}

#[derive(Debug)]
#[must_use]
struct ConveyorResume {
    parent: CountryResume,
    rows: Rows,
}

#[derive(Debug)]
#[must_use]
pub struct LineCursor {
    parent: ConveyorResume,
    rows: Rows,
}

#[derive(Debug)]
#[must_use]
struct LineResume {
    parent: ConveyorResume,
    rows: Rows,
}

#[derive(Debug)]
#[must_use]
pub struct EquipmentCursor {
    parent: LineResume,
    rows: Rows,
}

#[derive(Debug)]
#[must_use]
pub struct ConveyorNameCursor {
    parent: CountryResume,
    rows: Rows,
    index: usize,
    row: ConveyorFixedRow,
}

#[derive(Debug)]
#[must_use]
pub struct ConveyorRoleCursor {
    parent: CountryResume,
    rows: Rows,
    index: usize,
    row: ConveyorFixedRow,
    name: Span,
}

#[derive(Debug)]
#[must_use]
pub struct ConveyorLinesCursor {
    parent: CountryResume,
    rows: Rows,
    row: ConveyorRow,
}

#[derive(Debug)]
#[must_use]
pub struct LineEquipmentCursor {
    parent: ConveyorResume,
    rows: Rows,
    row: LineRow,
}

#[derive(Debug)]
#[must_use]
pub struct DeploymentEnd {
    pub at: usize,
    pub header: Header,
    pub totals: DelimitedTotals,
}

#[derive(Debug)]
pub enum CountryStep {
    Country {
        index: usize,
        row: CountryRow,
        conveyors: CountryConveyors,
    },
    End(DeploymentEnd),
}

#[derive(Debug)]
pub struct ConveyorFixedItem {
    pub index: usize,
    pub row: ConveyorFixedRow,
    pub name: ConveyorNameCursor,
}

#[derive(Debug)]
pub struct ConveyorNameItem {
    pub index: usize,
    pub row: ConveyorFixedRow,
    pub name: Span,
    pub role: ConveyorRoleCursor,
}

#[derive(Debug)]
pub struct ConveyorRoleItem {
    pub index: usize,
    pub row: ConveyorRow,
    pub lines: ConveyorLinesCursor,
}

#[derive(Debug)]
pub struct LineFixedItem {
    pub index: usize,
    pub row: LineRow,
    pub equipment: LineEquipmentCursor,
}

#[derive(Debug)]
pub struct EquipmentItem {
    pub index: usize,
    pub row: EquipmentRow,
    pub next: EquipmentCursor,
}

impl CountryCursor {
    pub const fn header(&self) -> &Header {
        &self.header
    }
    pub const fn remaining(&self) -> usize {
        self.rows.remaining()
    }
    pub const fn offset(&self) -> usize {
        self.rows.at
    }

    pub fn precheck_declared_counts(mut self, payload: &[u8]) -> Result<Self, Violation> {
        self.precheck(payload)?;
        Ok(self)
    }

    fn precheck(&mut self, payload: &[u8]) -> Result<(), Violation> {
        if !self.prechecked {
            require_count_fits_remaining(
                payload,
                HEADER_LEN,
                Block::Countries,
                self.header.country_count,
                COUNTRY_ROW_LEN,
            )?;
            require_count_fits_remaining(
                payload,
                HEADER_LEN,
                Block::Conveyors,
                self.header.declared_conveyors,
                CONVEYOR_ROW_LEN,
            )?;
            require_count_fits_remaining(
                payload,
                HEADER_LEN,
                Block::Lines,
                self.header.declared_lines,
                LINE_ROW_LEN,
            )?;
            require_count_fits_remaining(
                payload,
                HEADER_LEN,
                Block::Equipment,
                self.header.declared_equipment,
                EQUIPMENT_ROW_LEN,
            )?;
            self.prechecked = true;
        }
        Ok(())
    }

    pub fn next(mut self, payload: &[u8]) -> Result<CountryStep, Violation> {
        self.precheck(payload)?;
        if self.rows.remaining() == 0 {
            if self.rows.at != payload.len() {
                return Err(violation(
                    self.rows.at,
                    DqViolationKind::TrailingBytes {
                        consumed: self.rows.at,
                        available: payload.len(),
                    },
                ));
            }
            return Ok(CountryStep::End(DeploymentEnd {
                at: self.rows.at,
                header: self.header,
                totals: self.totals,
            }));
        }
        let at = self.rows.at;
        need(payload, at, DqSection::Country, COUNTRY_ROW_LEN)?;
        let row = CountryRow {
            tag_db_id: u32_at(payload, at + CountryField::TagDbId.offset()),
            flags: u32_at(payload, at + CountryField::Flags.offset()),
            conveyor_count: u32_at(payload, at + CountryField::ConveyorCount.offset()),
            reserved: u32_at(payload, at + CountryField::Reserved.offset()),
        };
        let next_at = add(at, at, COUNTRY_ROW_LEN)?;
        let index = self.rows.index;
        self.rows.advance_fixed(next_at);
        self.totals.countries = self.totals.countries.checked_add(1).ok_or_else(|| {
            violation(
                at,
                DqViolationKind::ArithmeticOverflow {
                    operation: DqArithmeticOp::U32Conversion,
                    left: self.totals.countries as usize,
                    right: 1,
                },
            )
        })?;
        Ok(CountryStep::Country {
            index,
            row,
            conveyors: CountryConveyors {
                parent: CountryResume {
                    header: self.header,
                    rows: self.rows,
                    totals: self.totals,
                    prechecked: self.prechecked,
                },
                at: next_at,
                count: row.conveyor_count,
            },
        })
    }

    pub fn into_end(mut self, payload: &[u8]) -> Result<DeploymentEnd, Violation> {
        self.precheck(payload)?;
        if self.rows.remaining() != 0 {
            return Err(caller(
                self.rows.at,
                Block::Countries,
                self.rows.remaining(),
            ));
        }
        if self.rows.at != payload.len() {
            return Err(violation(
                self.rows.at,
                DqViolationKind::TrailingBytes {
                    consumed: self.rows.at,
                    available: payload.len(),
                },
            ));
        }
        Ok(DeploymentEnd {
            at: self.rows.at,
            header: self.header,
            totals: self.totals,
        })
    }
}

impl CountryConveyors {
    pub const fn count(&self) -> u32 {
        self.count
    }
    pub const fn offset(&self) -> usize {
        self.at
    }

    pub fn open(self, payload: &[u8]) -> Result<ConveyorCursor, Violation> {
        require_count_fits_remaining(
            payload,
            self.at,
            Block::Conveyors,
            self.count,
            CONVEYOR_ROW_LEN,
        )?;
        Ok(ConveyorCursor {
            parent: self.parent,
            rows: Rows::new(self.at, self.count)?,
        })
    }
}

fn caller(at: usize, block: Block, remaining: usize) -> Violation {
    violation(
        at,
        DqViolationKind::CallerDefect(DqCallerDefect::BlockNotConsumed { block, remaining }),
    )
}

impl ConveyorCursor {
    pub const fn remaining(&self) -> usize {
        self.rows.remaining()
    }
    pub const fn offset(&self) -> usize {
        self.rows.at
    }

    pub fn next(mut self, payload: &[u8]) -> Result<ConveyorFixedItem, Violation> {
        if self.rows.remaining() == 0 {
            return Err(caller(self.rows.at, Block::Conveyors, 0));
        }
        let at = self.rows.at;
        need(payload, at, DqSection::Conveyor, CONVEYOR_ROW_LEN)?;
        let fixed_end = add(at, at, CONVEYOR_ROW_LEN)?;
        let line_count = u32_at(payload, at + ConveyorField::LineCount.offset());
        let row = ConveyorFixedRow {
            ordinal: u32_at(payload, at + ConveyorField::Ordinal.offset()),
            conveyor_id: u32_at(payload, at + ConveyorField::ConveyorId.offset()),
            id_type: u32_at(payload, at + ConveyorField::IdType.offset()),
            template_id: u32_at(payload, at + ConveyorField::TemplateId.offset()),
            template_id_type: u32_at(payload, at + ConveyorField::TemplateIdType.offset()),
            amount: i32_at(payload, at + ConveyorField::Amount.offset()),
            location: u32_at(payload, at + ConveyorField::Location.offset()),
            priority: u32_at(payload, at + ConveyorField::Priority.offset()),
            flags: u32_at(payload, at + ConveyorField::Flags.offset()),
            line_count,
        };
        let index = self.rows.index;
        self.rows.advance_fixed(fixed_end);
        self.parent.totals.conveyors =
            self.parent.totals.conveyors.checked_add(1).ok_or_else(|| {
                violation(
                    at,
                    DqViolationKind::ArithmeticOverflow {
                        operation: DqArithmeticOp::U32Conversion,
                        left: self.parent.totals.conveyors as usize,
                        right: 1,
                    },
                )
            })?;
        Ok(ConveyorFixedItem {
            index,
            row,
            name: ConveyorNameCursor {
                parent: self.parent,
                rows: self.rows,
                index,
                row,
            },
        })
    }

    pub fn into_end(self) -> Result<CountryCursor, Violation> {
        if self.rows.remaining() != 0 {
            return Err(caller(
                self.rows.at,
                Block::Conveyors,
                self.rows.remaining(),
            ));
        }
        let mut parent = self.parent;
        parent.rows.at = self.rows.at;
        Ok(CountryCursor {
            header: parent.header,
            rows: parent.rows,
            totals: parent.totals,
            prechecked: parent.prechecked,
        })
    }

    pub fn discard_all(mut self, payload: &[u8]) -> Result<CountryCursor, Violation> {
        while self.rows.remaining() != 0 {
            let fixed = self.next(payload)?;
            let name = fixed.name.read_name(payload)?;
            let role = name.role.read_role(payload)?;
            self = role.lines.open(payload)?.discard_all(payload)?;
        }
        self.into_end()
    }
}

impl ConveyorNameCursor {
    pub fn read_name(self, payload: &[u8]) -> Result<ConveyorNameItem, Violation> {
        let name = string_span(payload, self.rows.at, DqSection::ConveyorName)?;
        Ok(ConveyorNameItem {
            index: self.index,
            row: self.row,
            name,
            role: ConveyorRoleCursor {
                parent: self.parent,
                rows: self.rows,
                index: self.index,
                row: self.row,
                name,
            },
        })
    }
}

impl ConveyorRoleCursor {
    pub fn read_role(mut self, payload: &[u8]) -> Result<ConveyorRoleItem, Violation> {
        let role = string_span(payload, self.name.end, DqSection::ConveyorRole)?;
        self.rows.at = role.end;
        let row = ConveyorRow {
            ordinal: self.row.ordinal,
            conveyor_id: self.row.conveyor_id,
            id_type: self.row.id_type,
            template_id: self.row.template_id,
            template_id_type: self.row.template_id_type,
            amount: self.row.amount,
            location: self.row.location,
            priority: self.row.priority,
            flags: self.row.flags,
            line_count: self.row.line_count,
            name: self.name,
            role,
        };
        Ok(ConveyorRoleItem {
            index: self.index,
            row,
            lines: ConveyorLinesCursor {
                parent: self.parent,
                rows: self.rows,
                row,
            },
        })
    }
}

impl ConveyorLinesCursor {
    pub fn open(self, payload: &[u8]) -> Result<LineCursor, Violation> {
        require_count_fits_remaining(
            payload,
            self.rows.at,
            Block::Lines,
            self.row.line_count,
            LINE_ROW_LEN,
        )?;
        Ok(LineCursor {
            parent: ConveyorResume {
                parent: self.parent,
                rows: self.rows,
            },
            rows: Rows::new(self.row.role.end, self.row.line_count)?,
        })
    }
}

impl LineCursor {
    pub const fn remaining(&self) -> usize {
        self.rows.remaining()
    }
    pub const fn offset(&self) -> usize {
        self.rows.at
    }

    pub fn next(mut self, payload: &[u8]) -> Result<LineFixedItem, Violation> {
        if self.rows.remaining() == 0 {
            return Err(caller(self.rows.at, Block::Lines, 0));
        }
        let at = self.rows.at;
        need(payload, at, DqSection::Line, LINE_ROW_LEN)?;
        let equipment_count = u32_at(payload, at + DqLineField::EquipmentCount.offset());
        let next_at = add(at, at, LINE_ROW_LEN)?;
        let row = LineRow {
            ordinal: u32_at(payload, at + DqLineField::Ordinal.offset()),
            line_id: u32_at(payload, at + DqLineField::LineId.offset()),
            id_type: u32_at(payload, at + DqLineField::IdType.offset()),
            name_order: i32_at(payload, at + DqLineField::NameOrder.offset()),
            amount: i32_at(payload, at + DqLineField::Amount.offset()),
            flags: u32_at(payload, at + DqLineField::Flags.offset()),
            training_bits: u64_at(payload, at + DqLineField::TrainingBits.offset()),
            max_training_bits: u64_at(payload, at + DqLineField::MaxTrainingBits.offset()),
            manpower_have: i64_at(payload, at + DqLineField::ManpowerHave.offset()),
            manpower_need: i64_at(payload, at + DqLineField::ManpowerNeed.offset()),
            max_manpower: i64_at(payload, at + DqLineField::MaxManpower.offset()),
            equipment_count,
            reserved: u32_at(payload, at + DqLineField::Reserved.offset()),
        };
        let index = self.rows.index;
        self.rows.advance_fixed(next_at);
        self.parent.parent.totals.lines = self
            .parent
            .parent
            .totals
            .lines
            .checked_add(1)
            .ok_or_else(|| {
                violation(
                    at,
                    DqViolationKind::ArithmeticOverflow {
                        operation: DqArithmeticOp::U32Conversion,
                        left: self.parent.parent.totals.lines as usize,
                        right: 1,
                    },
                )
            })?;
        Ok(LineFixedItem {
            index,
            row,
            equipment: LineEquipmentCursor {
                parent: self.parent,
                rows: self.rows,
                row,
            },
        })
    }

    pub fn into_end(self) -> Result<ConveyorCursor, Violation> {
        if self.rows.remaining() != 0 {
            return Err(caller(self.rows.at, Block::Lines, self.rows.remaining()));
        }
        let mut parent = self.parent;
        parent.rows.at = self.rows.at;
        Ok(ConveyorCursor {
            parent: parent.parent,
            rows: parent.rows,
        })
    }

    pub fn discard_all(mut self, payload: &[u8]) -> Result<ConveyorCursor, Violation> {
        while self.rows.remaining() != 0 {
            self = self
                .next(payload)?
                .equipment
                .open(payload)?
                .discard_all(payload)?;
        }
        self.into_end()
    }
}

impl LineEquipmentCursor {
    pub fn open(self, payload: &[u8]) -> Result<EquipmentCursor, Violation> {
        let at = self.rows.at;
        require_count_fits_remaining(
            payload,
            self.rows.at,
            Block::Equipment,
            self.row.equipment_count,
            EQUIPMENT_ROW_LEN,
        )?;
        Ok(EquipmentCursor {
            parent: LineResume {
                parent: self.parent,
                rows: self.rows,
            },
            rows: Rows::new(at, self.row.equipment_count)?,
        })
    }
}

impl EquipmentCursor {
    pub const fn remaining(&self) -> usize {
        self.rows.remaining()
    }
    pub const fn offset(&self) -> usize {
        self.rows.at
    }

    pub fn next(mut self, payload: &[u8]) -> Result<EquipmentItem, Violation> {
        if self.rows.remaining() == 0 {
            return Err(caller(self.rows.at, Block::Equipment, 0));
        }
        let at = self.rows.at;
        need(payload, at, DqSection::Equipment, EQUIPMENT_ROW_LEN)?;
        let flags = u32_at(payload, at + EquipmentField::Flags.offset());
        let fixed_end = add(at, at, EQUIPMENT_ROW_LEN)?;
        let archetype = if self
            .parent
            .parent
            .parent
            .header
            .schema_version
            .archetype_present(flags)
        {
            Some(string_span(payload, fixed_end, DqSection::Archetype)?)
        } else {
            None
        };
        let end = archetype.map_or(fixed_end, |span| span.end);
        let row = EquipmentRow {
            ordinal: u32_at(payload, at + EquipmentField::Ordinal.offset()),
            equipment_id: u32_at(payload, at + EquipmentField::EquipmentId.offset()),
            id_type: u32_at(payload, at + EquipmentField::IdType.offset()),
            flags,
            amount_bits: u64_at(payload, at + EquipmentField::AmountBits.offset()),
            archetype,
        };
        let index = self.rows.index;
        self.rows.advance_fixed(end);
        self.parent.parent.parent.totals.equipment = self
            .parent
            .parent
            .parent
            .totals
            .equipment
            .checked_add(1)
            .ok_or_else(|| {
                violation(
                    at,
                    DqViolationKind::ArithmeticOverflow {
                        operation: DqArithmeticOp::U32Conversion,
                        left: self.parent.parent.parent.totals.equipment as usize,
                        right: 1,
                    },
                )
            })?;
        Ok(EquipmentItem {
            index,
            row,
            next: self,
        })
    }

    pub fn into_end(self) -> Result<LineCursor, Violation> {
        if self.rows.remaining() != 0 {
            return Err(caller(
                self.rows.at,
                Block::Equipment,
                self.rows.remaining(),
            ));
        }
        let mut parent = self.parent;
        parent.rows.at = self.rows.at;
        Ok(LineCursor {
            parent: parent.parent,
            rows: parent.rows,
        })
    }

    pub fn discard_all(mut self, payload: &[u8]) -> Result<LineCursor, Violation> {
        while self.rows.remaining() != 0 {
            self = self.next(payload)?.next;
        }
        self.into_end()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DqEncodeError {
    CountOverflow { count: usize },
    LengthOverflow { left: usize, right: usize },
    LengthDoesNotFitU32 { len: usize },
    Allocation { requested: usize },
    PreflightMismatch { declared: usize, written: usize },
}

#[derive(Clone, Copy, Debug)]
pub struct HeaderWords {
    pub elapsed_wall_ms: u64,
    pub changed_country_rows: u32,
    pub status_flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct CountryInput<'a, E> {
    pub tag_db_id: u32,
    pub flags: u32,
    pub reserved: u32,
    pub conveyors: &'a [ConveyorInput<'a, E>],
}

#[derive(Clone, Copy, Debug)]
pub struct ConveyorInput<'a, E> {
    pub ordinal: u32,
    pub conveyor_id: u32,
    pub id_type: u32,
    pub template_id: u32,
    pub template_id_type: u32,
    pub amount: i32,
    pub location: u32,
    pub priority: u32,
    pub flags: u32,
    pub name: &'a [u8],
    pub role: &'a [u8],
    pub lines: &'a [LineInput<'a, E>],
}

#[derive(Clone, Copy, Debug)]
pub struct LineInput<'a, E> {
    pub ordinal: u32,
    pub line_id: u32,
    pub id_type: u32,
    pub name_order: i32,
    pub amount: i32,
    pub flags: u32,
    pub training_bits: u64,
    pub max_training_bits: u64,
    pub manpower_have: i64,
    pub manpower_need: i64,
    pub max_manpower: i64,
    pub reserved: u32,
    pub equipment: &'a [E],
}

#[derive(Clone, Copy, Debug)]
pub struct EquipmentInputV1<'a> {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub id_type: u32,
    pub flags: u32,
    pub amount_bits: u64,
    pub archetype: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub struct EquipmentInputV2<'a> {
    pub ordinal: u32,
    pub equipment_id: u32,
    pub id_type: u32,
    pub flags_without_archetype: u32,
    pub amount_bits: u64,
    pub archetype: Option<&'a [u8]>,
}

pub trait EquipmentEncoding {
    fn ordinal(&self) -> u32;
    fn equipment_id(&self) -> u32;
    fn id_type(&self) -> u32;
    fn flags(&self) -> u32;
    fn amount_bits(&self) -> u64;
    fn archetype(&self) -> Option<&[u8]>;
}

pub trait LineEncoding {
    type Equipment: EquipmentEncoding;

    fn ordinal(&self) -> u32;
    fn line_id(&self) -> u32;
    fn id_type(&self) -> u32;
    fn name_order(&self) -> i32;
    fn amount(&self) -> i32;
    fn flags(&self) -> u32;
    fn training_bits(&self) -> u64;
    fn max_training_bits(&self) -> u64;
    fn manpower_have(&self) -> i64;
    fn manpower_need(&self) -> i64;
    fn max_manpower(&self) -> i64;
    fn reserved(&self) -> u32;
    fn equipment(&self) -> &[Self::Equipment];
}

pub trait ConveyorEncoding {
    type Line: LineEncoding;

    fn ordinal(&self) -> u32;
    fn conveyor_id(&self) -> u32;
    fn id_type(&self) -> u32;
    fn template_id(&self) -> u32;
    fn template_id_type(&self) -> u32;
    fn amount(&self) -> i32;
    fn location(&self) -> u32;
    fn priority(&self) -> u32;
    fn flags(&self) -> u32;
    fn name(&self) -> &[u8];
    fn role(&self) -> &[u8];
    fn lines(&self) -> &[Self::Line];
}

pub trait CountryEncoding {
    type Conveyor: ConveyorEncoding;

    fn tag_db_id(&self) -> u32;
    fn flags(&self) -> u32;
    fn reserved(&self) -> u32;
    fn conveyors(&self) -> &[Self::Conveyor];
}

impl<E: EquipmentEncoding> LineEncoding for LineInput<'_, E> {
    type Equipment = E;

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn line_id(&self) -> u32 {
        self.line_id
    }
    fn id_type(&self) -> u32 {
        self.id_type
    }
    fn name_order(&self) -> i32 {
        self.name_order
    }
    fn amount(&self) -> i32 {
        self.amount
    }
    fn flags(&self) -> u32 {
        self.flags
    }
    fn training_bits(&self) -> u64 {
        self.training_bits
    }
    fn max_training_bits(&self) -> u64 {
        self.max_training_bits
    }
    fn manpower_have(&self) -> i64 {
        self.manpower_have
    }
    fn manpower_need(&self) -> i64 {
        self.manpower_need
    }
    fn max_manpower(&self) -> i64 {
        self.max_manpower
    }
    fn reserved(&self) -> u32 {
        self.reserved
    }
    fn equipment(&self) -> &[Self::Equipment] {
        self.equipment
    }
}

impl<'a, E: EquipmentEncoding> ConveyorEncoding for ConveyorInput<'a, E> {
    type Line = LineInput<'a, E>;

    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn conveyor_id(&self) -> u32 {
        self.conveyor_id
    }
    fn id_type(&self) -> u32 {
        self.id_type
    }
    fn template_id(&self) -> u32 {
        self.template_id
    }
    fn template_id_type(&self) -> u32 {
        self.template_id_type
    }
    fn amount(&self) -> i32 {
        self.amount
    }
    fn location(&self) -> u32 {
        self.location
    }
    fn priority(&self) -> u32 {
        self.priority
    }
    fn flags(&self) -> u32 {
        self.flags
    }
    fn name(&self) -> &[u8] {
        self.name
    }
    fn role(&self) -> &[u8] {
        self.role
    }
    fn lines(&self) -> &[Self::Line] {
        self.lines
    }
}

impl<'a, E: EquipmentEncoding> CountryEncoding for CountryInput<'a, E> {
    type Conveyor = ConveyorInput<'a, E>;

    fn tag_db_id(&self) -> u32 {
        self.tag_db_id
    }
    fn flags(&self) -> u32 {
        self.flags
    }
    fn reserved(&self) -> u32 {
        self.reserved
    }
    fn conveyors(&self) -> &[Self::Conveyor] {
        self.conveyors
    }
}

impl EquipmentEncoding for EquipmentInputV1<'_> {
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn equipment_id(&self) -> u32 {
        self.equipment_id
    }
    fn id_type(&self) -> u32 {
        self.id_type
    }
    fn flags(&self) -> u32 {
        self.flags
    }
    fn amount_bits(&self) -> u64 {
        self.amount_bits
    }
    fn archetype(&self) -> Option<&[u8]> {
        Some(self.archetype)
    }
}

impl EquipmentEncoding for EquipmentInputV2<'_> {
    fn ordinal(&self) -> u32 {
        self.ordinal
    }
    fn equipment_id(&self) -> u32 {
        self.equipment_id
    }
    fn id_type(&self) -> u32 {
        self.id_type
    }
    fn flags(&self) -> u32 {
        (self.flags_without_archetype & !EquipmentFlag::Archetype.bit())
            | if self.archetype.is_some() {
                EquipmentFlag::Archetype.bit()
            } else {
                0
            }
    }
    fn amount_bits(&self) -> u64 {
        self.amount_bits
    }
    fn archetype(&self) -> Option<&[u8]> {
        self.archetype
    }
}

fn enc_add(left: usize, right: usize) -> Result<usize, DqEncodeError> {
    left.checked_add(right)
        .ok_or(DqEncodeError::LengthOverflow { left, right })
}
fn enc_mul(left: usize, right: usize) -> Result<usize, DqEncodeError> {
    left.checked_mul(right)
        .ok_or(DqEncodeError::LengthOverflow { left, right })
}
fn enc_count(value: usize) -> Result<u32, DqEncodeError> {
    u32::try_from(value).map_err(|_| DqEncodeError::CountOverflow { count: value })
}
fn enc_string_len(value: &[u8]) -> Result<usize, DqEncodeError> {
    let _ = u32::try_from(value.len())
        .map_err(|_| DqEncodeError::LengthDoesNotFitU32 { len: value.len() })?;
    enc_add(STRING_PREFIX_LEN, value.len())
}

#[derive(Clone, Copy)]
struct Preflight {
    len: usize,
    countries: u32,
    conveyors: u32,
    lines: u32,
    equipment: u32,
}

fn preflight<C: CountryEncoding>(countries: &[C]) -> Result<Preflight, DqEncodeError> {
    let mut len = HEADER_LEN;
    let mut conveyor_total = 0usize;
    let mut line_total = 0usize;
    let mut equipment_total = 0usize;
    for country in countries {
        len = enc_add(len, COUNTRY_ROW_LEN)?;
        let conveyors = country.conveyors();
        let _ = enc_count(conveyors.len())?;
        conveyor_total = enc_add(conveyor_total, conveyors.len())?;
        for conveyor in conveyors {
            len = enc_add(len, CONVEYOR_ROW_LEN)?;
            len = enc_add(len, enc_string_len(conveyor.name())?)?;
            len = enc_add(len, enc_string_len(conveyor.role())?)?;
            let lines = conveyor.lines();
            let _ = enc_count(lines.len())?;
            line_total = enc_add(line_total, lines.len())?;
            for line in lines {
                len = enc_add(len, LINE_ROW_LEN)?;
                let equipment_rows = line.equipment();
                let _ = enc_count(equipment_rows.len())?;
                equipment_total = enc_add(equipment_total, equipment_rows.len())?;
                len = enc_add(len, enc_mul(equipment_rows.len(), EQUIPMENT_ROW_LEN)?)?;
                for equipment in equipment_rows {
                    if let Some(archetype) = equipment.archetype() {
                        len = enc_add(len, enc_string_len(archetype)?)?;
                    }
                }
            }
        }
    }
    Ok(Preflight {
        len,
        countries: enc_count(countries.len())?,
        conveyors: enc_count(conveyor_total)?,
        lines: enc_count(line_total)?,
        equipment: enc_count(equipment_total)?,
    })
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_string(out: &mut Vec<u8>, value: &[u8]) -> Result<(), DqEncodeError> {
    let len = u32::try_from(value.len())
        .map_err(|_| DqEncodeError::LengthDoesNotFitU32 { len: value.len() })?;
    push_u32(out, len);
    out.extend_from_slice(value);
    Ok(())
}

fn encode<C: CountryEncoding>(
    schema: SchemaVersion,
    words: HeaderWords,
    countries: &[C],
) -> Result<Vec<u8>, DqEncodeError> {
    let p = preflight(countries)?;
    let mut out = Vec::new();
    out.try_reserve_exact(p.len)
        .map_err(|_| DqEncodeError::Allocation { requested: p.len })?;
    let mut written = Preflight {
        len: 0,
        countries: 0,
        conveyors: 0,
        lines: 0,
        equipment: 0,
    };
    push_u64(&mut out, words.elapsed_wall_ms);
    let country_count_at = out.len();
    push_u32(&mut out, 0);
    push_u32(&mut out, words.changed_country_rows);
    push_u32(&mut out, words.status_flags);
    push_u32(&mut out, schema.wire());
    let conveyor_count_at = out.len();
    push_u32(&mut out, 0);
    let line_count_at = out.len();
    push_u32(&mut out, 0);
    let equipment_count_at = out.len();
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    for country in countries {
        push_u32(&mut out, country.tag_db_id());
        push_u32(&mut out, country.flags());
        written.countries = enc_count(written.countries as usize + 1)?;
        let conveyors = country.conveyors();
        written.conveyors = enc_count(written.conveyors as usize + conveyors.len())?;
        push_u32(&mut out, conveyors.len() as u32);
        push_u32(&mut out, country.reserved());
        for conveyor in conveyors {
            push_u32(&mut out, conveyor.ordinal());
            push_u32(&mut out, conveyor.conveyor_id());
            push_u32(&mut out, conveyor.id_type());
            push_u32(&mut out, conveyor.template_id());
            push_u32(&mut out, conveyor.template_id_type());
            push_i32(&mut out, conveyor.amount());
            push_u32(&mut out, conveyor.location());
            push_u32(&mut out, conveyor.priority());
            push_u32(&mut out, conveyor.flags());
            let lines = conveyor.lines();
            written.lines = enc_count(written.lines as usize + lines.len())?;
            push_u32(&mut out, lines.len() as u32);
            push_string(&mut out, conveyor.name())?;
            push_string(&mut out, conveyor.role())?;
            for line in lines {
                push_u32(&mut out, line.ordinal());
                push_u32(&mut out, line.line_id());
                push_u32(&mut out, line.id_type());
                push_i32(&mut out, line.name_order());
                push_i32(&mut out, line.amount());
                push_u32(&mut out, line.flags());
                push_u64(&mut out, line.training_bits());
                push_u64(&mut out, line.max_training_bits());
                push_i64(&mut out, line.manpower_have());
                push_i64(&mut out, line.manpower_need());
                push_i64(&mut out, line.max_manpower());
                let equipment_rows = line.equipment();
                written.equipment = enc_count(written.equipment as usize + equipment_rows.len())?;
                push_u32(&mut out, equipment_rows.len() as u32);
                push_u32(&mut out, line.reserved());
                for equipment in equipment_rows {
                    let archetype = equipment.archetype();
                    push_u32(&mut out, equipment.ordinal());
                    push_u32(&mut out, equipment.equipment_id());
                    push_u32(&mut out, equipment.id_type());
                    let flags = match schema {
                        SchemaVersion::V1 => equipment.flags(),
                        SchemaVersion::V2 => {
                            (equipment.flags() & !EquipmentFlag::Archetype.bit())
                                | if archetype.is_some() {
                                    EquipmentFlag::Archetype.bit()
                                } else {
                                    0
                                }
                        }
                    };
                    push_u32(&mut out, flags);
                    push_u64(&mut out, equipment.amount_bits());
                    if let Some(archetype) = archetype {
                        push_string(&mut out, archetype)?;
                    }
                }
            }
        }
    }
    out[country_count_at..country_count_at + 4].copy_from_slice(&written.countries.to_le_bytes());
    out[conveyor_count_at..conveyor_count_at + 4].copy_from_slice(&written.conveyors.to_le_bytes());
    out[line_count_at..line_count_at + 4].copy_from_slice(&written.lines.to_le_bytes());
    out[equipment_count_at..equipment_count_at + 4]
        .copy_from_slice(&written.equipment.to_le_bytes());

    if out.len() != p.len
        || written.countries != p.countries
        || written.conveyors != p.conveyors
        || written.lines != p.lines
        || written.equipment != p.equipment
    {
        return Err(DqEncodeError::PreflightMismatch {
            declared: p.len,
            written: out.len(),
        });
    }
    Ok(out)
}

pub fn encode_v1(
    words: HeaderWords,
    countries: &[CountryInput<'_, EquipmentInputV1<'_>>],
) -> Result<Vec<u8>, DqEncodeError> {
    encode(SchemaVersion::V1, words, countries)
}

pub fn encode_v2<C: CountryEncoding>(
    words: HeaderWords,
    countries: &[C],
) -> Result<Vec<u8>, DqEncodeError> {
    encode(SchemaVersion::V2, words, countries)
}
