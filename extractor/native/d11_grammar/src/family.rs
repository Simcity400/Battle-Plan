use crate::{
    checked_add, checked_span, checked_sub, i32_at, i64_at, need, need_row, tiles, u16_at, u32_at,
    Section, Span, Trailer, Violation, ViolationKind, Width,
};

pub const FIXED_HEADER_LEN: usize = 20;
pub const PCV_HEADER_LEN: usize = 28;
pub const LINE_KEY_LEN: usize = 12;
pub const PLIC_COUNTRY_ROW_LEN: usize = 8;
pub const PLIC_V1_ITEM_LEN: usize = 4;
pub const PLIC_V2_ITEM_LEN: usize = 16;
pub const PCV_COUNTRY_ROW_LEN: usize = 40;
pub const PCV_VARIANT_ROW_LEN: usize = 16;
pub const PCV_RESERVATION_ROW_LEN: usize = 8;

pub const KNOWN_PCP_FLAGS: u32 = 0b111;
pub const KNOWN_PCV_FLAGS: u32 = 0b1_1111;
pub const KNOWN_FIXED_DECLARED_WORDS: u32 = 0b1;
pub const KNOWN_PLIC_DECLARED_WORDS: u32 = 0b11;
pub const KNOWN_PCV_DECLARED_WORDS: u32 = 0b111;
pub const KNOWN_PDQ_DECLARED_WORDS: u32 = 0b1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedHeaderField {
    Magic,
    Version,
    HeaderLen,
    SegmentLen,
    RowCount,
    Reserved,
}

impl FixedHeaderField {
    pub const ALL: [FixedHeaderField; 6] = [
        FixedHeaderField::Magic,
        FixedHeaderField::Version,
        FixedHeaderField::HeaderLen,
        FixedHeaderField::SegmentLen,
        FixedHeaderField::RowCount,
        FixedHeaderField::Reserved,
    ];
    pub const fn offset(self) -> usize {
        match self {
            FixedHeaderField::Magic => 0,
            FixedHeaderField::Version => 4,
            FixedHeaderField::HeaderLen => 6,
            FixedHeaderField::SegmentLen => 8,
            FixedHeaderField::RowCount => 12,
            FixedHeaderField::Reserved => 16,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            FixedHeaderField::Magic => Width::Bytes4,
            FixedHeaderField::Version | FixedHeaderField::HeaderLen => Width::U16,
            FixedHeaderField::SegmentLen
            | FixedHeaderField::RowCount
            | FixedHeaderField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlicHeaderField {
    Magic,
    Version,
    HeaderLen,
    CountryRows,
    IdRows,
    Reserved,
}

impl PlicHeaderField {
    pub const ALL: [PlicHeaderField; 6] = [
        PlicHeaderField::Magic,
        PlicHeaderField::Version,
        PlicHeaderField::HeaderLen,
        PlicHeaderField::CountryRows,
        PlicHeaderField::IdRows,
        PlicHeaderField::Reserved,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PlicHeaderField::Magic => 0,
            PlicHeaderField::Version => 4,
            PlicHeaderField::HeaderLen => 6,
            PlicHeaderField::CountryRows => 8,
            PlicHeaderField::IdRows => 12,
            PlicHeaderField::Reserved => 16,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PlicHeaderField::Magic => Width::Bytes4,
            PlicHeaderField::Version | PlicHeaderField::HeaderLen => Width::U16,
            PlicHeaderField::CountryRows | PlicHeaderField::IdRows | PlicHeaderField::Reserved => {
                Width::U32
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvHeaderField {
    Magic,
    Version,
    HeaderLen,
    SegmentLen,
    CountryCount,
    VariantCount,
    ReservationCount,
    Reserved,
}

impl PcvHeaderField {
    pub const ALL: [PcvHeaderField; 8] = [
        PcvHeaderField::Magic,
        PcvHeaderField::Version,
        PcvHeaderField::HeaderLen,
        PcvHeaderField::SegmentLen,
        PcvHeaderField::CountryCount,
        PcvHeaderField::VariantCount,
        PcvHeaderField::ReservationCount,
        PcvHeaderField::Reserved,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PcvHeaderField::Magic => 0,
            PcvHeaderField::Version => 4,
            PcvHeaderField::HeaderLen => 6,
            PcvHeaderField::SegmentLen => 8,
            PcvHeaderField::CountryCount => 12,
            PcvHeaderField::VariantCount => 16,
            PcvHeaderField::ReservationCount => 20,
            PcvHeaderField::Reserved => 24,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PcvHeaderField::Magic => Width::Bytes4,
            PcvHeaderField::Version | PcvHeaderField::HeaderLen => Width::U16,
            PcvHeaderField::SegmentLen
            | PcvHeaderField::CountryCount
            | PcvHeaderField::VariantCount
            | PcvHeaderField::ReservationCount
            | PcvHeaderField::Reserved => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PdqHeaderField {
    Magic,
    Version,
    HeaderLen,
    SegmentLen,
    CodecPayloadLen,
    Reserved,
}

impl PdqHeaderField {
    pub const ALL: [PdqHeaderField; 6] = [
        PdqHeaderField::Magic,
        PdqHeaderField::Version,
        PdqHeaderField::HeaderLen,
        PdqHeaderField::SegmentLen,
        PdqHeaderField::CodecPayloadLen,
        PdqHeaderField::Reserved,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PdqHeaderField::Magic => 0,
            PdqHeaderField::Version => 4,
            PdqHeaderField::HeaderLen => 6,
            PdqHeaderField::SegmentLen => 8,
            PdqHeaderField::CodecPayloadLen => 12,
            PdqHeaderField::Reserved => 16,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PdqHeaderField::Magic => Width::Bytes4,
            PdqHeaderField::Version | PdqHeaderField::HeaderLen => Width::U16,
            PdqHeaderField::SegmentLen
            | PdqHeaderField::CodecPayloadLen
            | PdqHeaderField::Reserved => Width::U32,
        }
    }
}

const _: () = {
    let mut offsets = [0usize; 6];
    let mut widths = [Width::U32; 6];
    let mut i = 0;
    while i < FixedHeaderField::ALL.len() {
        offsets[i] = FixedHeaderField::ALL[i].offset();
        widths[i] = FixedHeaderField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, FIXED_HEADER_LEN) {
        panic!("d11_grammar: FixedHeaderField::ALL does not tile 20 bytes");
    }
    let mut offsets = [0usize; 6];
    let mut widths = [Width::U32; 6];
    let mut i = 0;
    while i < PlicHeaderField::ALL.len() {
        offsets[i] = PlicHeaderField::ALL[i].offset();
        widths[i] = PlicHeaderField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, FIXED_HEADER_LEN) {
        panic!("d11_grammar: PlicHeaderField::ALL does not tile 20 bytes");
    }
    let mut offsets = [0usize; 8];
    let mut widths = [Width::U32; 8];
    let mut i = 0;
    while i < PcvHeaderField::ALL.len() {
        offsets[i] = PcvHeaderField::ALL[i].offset();
        widths[i] = PcvHeaderField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PCV_HEADER_LEN) {
        panic!("d11_grammar: PcvHeaderField::ALL does not tile 28 bytes");
    }
    let mut offsets = [0usize; 6];
    let mut widths = [Width::U32; 6];
    let mut i = 0;
    while i < PdqHeaderField::ALL.len() {
        offsets[i] = PdqHeaderField::ALL[i].offset();
        widths[i] = PdqHeaderField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, FIXED_HEADER_LEN) {
        panic!("d11_grammar: PdqHeaderField::ALL does not tile 20 bytes");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineKeyField {
    TagDbId,
    Kind,
    LineId,
}

impl LineKeyField {
    pub const ALL: [LineKeyField; 3] = [
        LineKeyField::TagDbId,
        LineKeyField::Kind,
        LineKeyField::LineId,
    ];
    pub const fn offset(self) -> usize {
        match self {
            LineKeyField::TagDbId => 0,
            LineKeyField::Kind => 4,
            LineKeyField::LineId => 8,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            LineKeyField::TagDbId | LineKeyField::Kind => Width::U32,
            LineKeyField::LineId => Width::I32,
        }
    }
}

// key. They are written out rather than generated: a leaf may not DEFINE a

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PvjField {
    EquipmentVariantId,
}

impl PvjField {
    pub const ALL: [PvjField; 1] = [PvjField::EquipmentVariantId];

    pub const fn offset(self) -> usize {
        match self {
            PvjField::EquipmentVariantId => LINE_KEY_LEN,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            PvjField::EquipmentVariantId => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PfrField {
    RequestedFactories,
}

impl PfrField {
    pub const ALL: [PfrField; 1] = [PfrField::RequestedFactories];

    pub const fn offset(self) -> usize {
        match self {
            PfrField::RequestedFactories => LINE_KEY_LEN,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            PfrField::RequestedFactories => Width::I32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PprField {
    Priority,
}

impl PprField {
    pub const ALL: [PprField; 1] = [PprField::Priority];

    pub const fn offset(self) -> usize {
        match self {
            PprField::Priority => LINE_KEY_LEN,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            PprField::Priority => Width::I32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlsField {
    Subtype,
}

impl PlsField {
    pub const ALL: [PlsField; 1] = [PlsField::Subtype];

    pub const fn offset(self) -> usize {
        match self {
            PlsField::Subtype => LINE_KEY_LEN,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            PlsField::Subtype => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PmiField {
    MioId,
}

impl PmiField {
    pub const ALL: [PmiField; 1] = [PmiField::MioId];

    pub const fn offset(self) -> usize {
        match self {
            PmiField::MioId => LINE_KEY_LEN,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            PmiField::MioId => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PefField {
    EfficiencyRaw,
    EfficiencyCapRaw,
    EfficiencyGrowthRaw,
}

impl PefField {
    pub const ALL: [PefField; 3] = [
        PefField::EfficiencyRaw,
        PefField::EfficiencyCapRaw,
        PefField::EfficiencyGrowthRaw,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PefField::EfficiencyRaw => 12,
            PefField::EfficiencyCapRaw => 20,
            PefField::EfficiencyGrowthRaw => 28,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PefField::EfficiencyRaw
            | PefField::EfficiencyCapRaw
            | PefField::EfficiencyGrowthRaw => Width::I64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcpField {
    TagDbId,
    Flags,
}

impl PcpField {
    pub const ALL: [PcpField; 2] = [PcpField::TagDbId, PcpField::Flags];
    pub const fn offset(self) -> usize {
        match self {
            PcpField::TagDbId => 0,
            PcpField::Flags => 4,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PcpField::TagDbId | PcpField::Flags => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlicCountryField {
    TagDbId,
    Count,
}

impl PlicCountryField {
    pub const ALL: [PlicCountryField; 2] = [PlicCountryField::TagDbId, PlicCountryField::Count];
    pub const fn offset(self) -> usize {
        match self {
            PlicCountryField::TagDbId => 0,
            PlicCountryField::Count => 4,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PlicCountryField::TagDbId | PlicCountryField::Count => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlicV1ItemField {
    VariantId,
}

impl PlicV1ItemField {
    pub const ALL: [PlicV1ItemField; 1] = [PlicV1ItemField::VariantId];
    pub const fn offset(self) -> usize {
        match self {
            PlicV1ItemField::VariantId => 0,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PlicV1ItemField::VariantId => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlicV2ItemField {
    VariantId,
    VisibleOrder,
    CreatorTagDbId,
    OriginTagDbId,
}

impl PlicV2ItemField {
    pub const ALL: [PlicV2ItemField; 4] = [
        PlicV2ItemField::VariantId,
        PlicV2ItemField::VisibleOrder,
        PlicV2ItemField::CreatorTagDbId,
        PlicV2ItemField::OriginTagDbId,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PlicV2ItemField::VariantId => 0,
            PlicV2ItemField::VisibleOrder => 4,
            PlicV2ItemField::CreatorTagDbId => 8,
            PlicV2ItemField::OriginTagDbId => 12,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PlicV2ItemField::VariantId
            | PlicV2ItemField::VisibleOrder
            | PlicV2ItemField::CreatorTagDbId
            | PlicV2ItemField::OriginTagDbId => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvCountryField {
    TagDbId,
    Flags,
    OwnedRaw,
    Assigned,
    Available,
    VariantStart,
    VariantLen,
    ReservationStart,
    ReservationLen,
}

impl PcvCountryField {
    pub const ALL: [PcvCountryField; 9] = [
        PcvCountryField::TagDbId,
        PcvCountryField::Flags,
        PcvCountryField::OwnedRaw,
        PcvCountryField::Assigned,
        PcvCountryField::Available,
        PcvCountryField::VariantStart,
        PcvCountryField::VariantLen,
        PcvCountryField::ReservationStart,
        PcvCountryField::ReservationLen,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PcvCountryField::TagDbId => 0,
            PcvCountryField::Flags => 4,
            PcvCountryField::OwnedRaw => 8,
            PcvCountryField::Assigned => 16,
            PcvCountryField::Available => 20,
            PcvCountryField::VariantStart => 24,
            PcvCountryField::VariantLen => 28,
            PcvCountryField::ReservationStart => 32,
            PcvCountryField::ReservationLen => 36,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PcvCountryField::OwnedRaw => Width::I64,
            PcvCountryField::Assigned | PcvCountryField::Available => Width::I32,
            PcvCountryField::TagDbId
            | PcvCountryField::Flags
            | PcvCountryField::VariantStart
            | PcvCountryField::VariantLen
            | PcvCountryField::ReservationStart
            | PcvCountryField::ReservationLen => Width::U32,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvVariantField {
    VariantId,
    EquipType,
    AmountRaw,
}

impl PcvVariantField {
    pub const ALL: [PcvVariantField; 3] = [
        PcvVariantField::VariantId,
        PcvVariantField::EquipType,
        PcvVariantField::AmountRaw,
    ];
    pub const fn offset(self) -> usize {
        match self {
            PcvVariantField::VariantId => 0,
            PcvVariantField::EquipType => 4,
            PcvVariantField::AmountRaw => 8,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PcvVariantField::VariantId | PcvVariantField::EquipType => Width::U32,
            PcvVariantField::AmountRaw => Width::I64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcvReservationField {
    Priority,
    Count,
}

impl PcvReservationField {
    pub const ALL: [PcvReservationField; 2] =
        [PcvReservationField::Priority, PcvReservationField::Count];
    pub const fn offset(self) -> usize {
        match self {
            PcvReservationField::Priority => 0,
            PcvReservationField::Count => 4,
        }
    }
    pub const fn width(self) -> Width {
        match self {
            PcvReservationField::Priority | PcvReservationField::Count => Width::I32,
        }
    }
}

const _: () = {
    let mut offsets = [0usize; 3];
    let mut widths = [Width::U32; 3];
    let mut i = 0;
    while i < LineKeyField::ALL.len() {
        offsets[i] = LineKeyField::ALL[i].offset();
        widths[i] = LineKeyField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, LINE_KEY_LEN) {
        panic!("d11_grammar: LineKeyField::ALL does not tile 12 bytes");
    }
    if PvjField::ALL[0].offset() + PvjField::ALL[0].width().bytes() != 16
        || PfrField::ALL[0].offset() + PfrField::ALL[0].width().bytes() != 16
        || PprField::ALL[0].offset() + PprField::ALL[0].width().bytes() != 16
        || PlsField::ALL[0].offset() + PlsField::ALL[0].width().bytes() != 16
        || PmiField::ALL[0].offset() + PmiField::ALL[0].width().bytes() != 16
    {
        panic!("d11_grammar: a 16-byte line-key trailer does not tile its row");
    }
    let mut expected = LINE_KEY_LEN;
    let mut i = 0;
    while i < PefField::ALL.len() {
        if PefField::ALL[i].offset() != expected {
            panic!("d11_grammar: PefField::ALL does not tile its unaligned tail");
        }
        expected += PefField::ALL[i].width().bytes();
        i += 1;
    }
    if expected != 36 {
        panic!("d11_grammar: PEF1 row is not 36 bytes");
    }
};

const _: () = {
    let mut offsets = [0usize; 2];
    let mut widths = [Width::U32; 2];
    let mut i = 0;
    while i < PcpField::ALL.len() {
        offsets[i] = PcpField::ALL[i].offset();
        widths[i] = PcpField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, 8) {
        panic!("d11_grammar: PcpField::ALL does not tile 8 bytes");
    }
    let mut offsets = [0usize; 2];
    let mut widths = [Width::U32; 2];
    let mut i = 0;
    while i < PlicCountryField::ALL.len() {
        offsets[i] = PlicCountryField::ALL[i].offset();
        widths[i] = PlicCountryField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PLIC_COUNTRY_ROW_LEN) {
        panic!("d11_grammar: PLIC country fields do not tile");
    }
    if PlicV1ItemField::ALL[0].offset() + PlicV1ItemField::ALL[0].width().bytes()
        != PLIC_V1_ITEM_LEN
    {
        panic!("d11_grammar: PLIC v1 item fields do not tile");
    }
    let mut offsets = [0usize; 4];
    let mut widths = [Width::U32; 4];
    let mut i = 0;
    while i < PlicV2ItemField::ALL.len() {
        offsets[i] = PlicV2ItemField::ALL[i].offset();
        widths[i] = PlicV2ItemField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PLIC_V2_ITEM_LEN) {
        panic!("d11_grammar: PLIC v2 item fields do not tile");
    }
};

const _: () = {
    let mut offsets = [0usize; 9];
    let mut widths = [Width::U32; 9];
    let mut i = 0;
    while i < PcvCountryField::ALL.len() {
        offsets[i] = PcvCountryField::ALL[i].offset();
        widths[i] = PcvCountryField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PCV_COUNTRY_ROW_LEN) {
        panic!("d11_grammar: PCV1 country fields do not tile");
    }
    let mut offsets = [0usize; 3];
    let mut widths = [Width::U32; 3];
    let mut i = 0;
    while i < PcvVariantField::ALL.len() {
        offsets[i] = PcvVariantField::ALL[i].offset();
        widths[i] = PcvVariantField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PCV_VARIANT_ROW_LEN) {
        panic!("d11_grammar: PCV1 variant fields do not tile");
    }
    let mut offsets = [0usize; 2];
    let mut widths = [Width::I32; 2];
    let mut i = 0;
    while i < PcvReservationField::ALL.len() {
        offsets[i] = PcvReservationField::ALL[i].offset();
        widths[i] = PcvReservationField::ALL[i].width();
        i += 1;
    }
    if !tiles(&offsets, &widths, PCV_RESERVATION_ROW_LEN) {
        panic!("d11_grammar: PCV1 reservation fields do not tile");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum PlicVersion {
    V1,
    V2,
}

impl PlicVersion {
    pub const ALL: [PlicVersion; 2] = [PlicVersion::V1, PlicVersion::V2];
    pub const fn wire(self) -> u16 {
        match self {
            PlicVersion::V1 => 1,
            PlicVersion::V2 => 2,
        }
    }
    pub const fn item_len(self) -> usize {
        match self {
            PlicVersion::V1 => PLIC_V1_ITEM_LEN,
            PlicVersion::V2 => PLIC_V2_ITEM_LEN,
        }
    }
    /// Read through `wire`, not beside it: a literal decoder was a second
    /// copy of the same mapping, so moving a version's emitted code without
    /// moving the decoder arm emitted a code this decoder refuses (lane TV,
    /// row R2).
    const fn from_wire(value: u16) -> Option<PlicVersion> {
        let mut i = 0;
        while i < PlicVersion::ALL.len() {
            if PlicVersion::ALL[i].wire() == value {
                return Some(PlicVersion::ALL[i]);
            }
            i += 1;
        }
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DerivedExtent {
    pub computed_len: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeclaredExtent {
    pub declared_len: usize,
    pub computed_len: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedRows {
    pub trailer: Trailer,
    pub version: u16,
    pub extent: DeclaredExtent,
    pub row_count: usize,
    pub rows: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicHeader {
    pub version: PlicVersion,
    pub extent: DerivedExtent,
    pub country_count: usize,
    pub item_count: usize,
    pub summed_item_count: usize,
    pub item_len: usize,
    pub countries: Span,
    pub items: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvHeader {
    pub extent: DeclaredExtent,
    pub country_count: usize,
    pub variant_count: usize,
    pub reservation_count: usize,
    pub countries: Span,
    pub variants: Span,
    pub reservations: Span,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PdqHeader {
    pub extent: DeclaredExtent,
    codec_payload: Span,
}

impl PdqHeader {
    pub const fn inner_payload_span(&self) -> Span {
        self.codec_payload
    }

    pub const fn ends_at_payload_end(&self, payload_len: usize) -> bool {
        self.extent.end == payload_len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HeaderPreview {
    pub trailer: Trailer,
    pub at: usize,
    pub version: u16,
    pub header_len: u16,
    pub words: [u32; 5],
    pub word_count: usize,
}

impl HeaderPreview {
    pub fn computed_len(&self) -> usize {
        match self.trailer {
            Trailer::Plic => {
                let item_len = match self.version {
                    1 => PLIC_V1_ITEM_LEN,
                    2 => PLIC_V2_ITEM_LEN,
                    _ => 0,
                };
                FIXED_HEADER_LEN
                    .saturating_add((self.words[0] as usize).saturating_mul(PLIC_COUNTRY_ROW_LEN))
                    .saturating_add((self.words[1] as usize).saturating_mul(item_len))
            }
            Trailer::Pcv1 => PCV_HEADER_LEN
                .saturating_add((self.words[1] as usize).saturating_mul(PCV_COUNTRY_ROW_LEN))
                .saturating_add((self.words[2] as usize).saturating_mul(PCV_VARIANT_ROW_LEN))
                .saturating_add((self.words[3] as usize).saturating_mul(PCV_RESERVATION_ROW_LEN)),
            Trailer::Pdq1 => FIXED_HEADER_LEN.saturating_add(self.words[1] as usize),
            Trailer::Pvj1
            | Trailer::Pfr1
            | Trailer::Ppr1
            | Trailer::Pls1
            | Trailer::Pef1
            | Trailer::Pmi1
            | Trailer::Pcp1 => FIXED_HEADER_LEN.saturating_add(
                (self.words[1] as usize).saturating_mul(fixed_row_len(self.trailer)),
            ),
        }
    }

    pub const fn declared_len(&self) -> Option<usize> {
        match self.trailer {
            Trailer::Plic => None,
            Trailer::Pvj1
            | Trailer::Pfr1
            | Trailer::Ppr1
            | Trailer::Pls1
            | Trailer::Pef1
            | Trailer::Pmi1
            | Trailer::Pcp1
            | Trailer::Pcv1
            | Trailer::Pdq1 => Some(self.words[0] as usize),
        }
    }

    pub const fn reserved(&self) -> u32 {
        self.words[self.word_count - 1]
    }
}

pub(crate) enum TrailerHeaders {
    Plic(PlicHeader),
    Pvj1(FixedRows),
    Pfr1(FixedRows),
    Ppr1(FixedRows),
    Pls1(FixedRows),
    Pef1(FixedRows),
    Pmi1(FixedRows),
    Pcp1(FixedRows),
    Pcv1(PcvHeader),
    Pdq1(PdqHeader),
}

pub(crate) fn read_header_preview(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
) -> Result<HeaderPreview, Violation> {
    let end = checked_add(
        Section::TrailerHeader(trailer),
        at,
        at,
        trailer.header_len(),
    )?;
    need(payload.len(), at, Section::TrailerHeader(trailer), end)?;
    let word_count = match trailer {
        Trailer::Plic
        | Trailer::Pvj1
        | Trailer::Pfr1
        | Trailer::Ppr1
        | Trailer::Pls1
        | Trailer::Pef1
        | Trailer::Pmi1
        | Trailer::Pcp1
        | Trailer::Pdq1 => 3,
        Trailer::Pcv1 => 5,
    };
    let mut words = [0u32; 5];
    let mut i = 0;
    while i < word_count {
        words[i] = u32_at(
            payload,
            at + FixedHeaderField::SegmentLen.offset() + i * Width::U32.bytes(),
        );
        i += 1;
    }
    Ok(HeaderPreview {
        trailer,
        at,
        version: u16_at(payload, at + FixedHeaderField::Version.offset()),
        header_len: u16_at(payload, at + FixedHeaderField::HeaderLen.offset()),
        words,
        word_count,
    })
}

fn check_common(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
    accepted_versions: &[u16],
) -> Result<HeaderPreview, Violation> {
    let preview = read_header_preview(payload, at, trailer)?;
    if !accepted_versions.contains(&preview.version) {
        return Err(Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: preview.version,
            },
        ));
    }
    let expected = trailer.header_len();
    if preview.header_len as usize != expected {
        return Err(Violation::new(
            at,
            ViolationKind::HeaderLen {
                trailer,
                declared: preview.header_len as usize,
                expected,
            },
        ));
    }
    Ok(preview)
}

fn check_reserved(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
    offset: usize,
) -> Result<(), Violation> {
    let value = u32_at(payload, at + offset);
    if value != 0 {
        return Err(Violation::new(
            at,
            ViolationKind::ReservedWord {
                trailer,
                at: at + offset,
                value,
            },
        ));
    }
    Ok(())
}

fn declared_extent(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
    declared_len: usize,
    computed_end: usize,
) -> Result<DeclaredExtent, Violation> {
    let computed_len = checked_sub(Section::TrailerBody(trailer), at, computed_end, at)?;
    if declared_len != computed_len {
        return Err(Violation::new(
            at,
            ViolationKind::DeclaredExtent {
                trailer,
                declared_len,
                computed_len,
            },
        ));
    }
    need(
        payload.len(),
        at,
        Section::TrailerBody(trailer),
        computed_end,
    )?;
    Ok(DeclaredExtent {
        declared_len,
        computed_len,
        end: computed_end,
    })
}

pub const fn fixed_row_len(trailer: Trailer) -> usize {
    match trailer {
        Trailer::Pvj1 | Trailer::Pfr1 | Trailer::Ppr1 | Trailer::Pls1 | Trailer::Pmi1 => 16,
        Trailer::Pef1 => 36,
        Trailer::Pcp1 => 8,
        Trailer::Plic | Trailer::Pcv1 | Trailer::Pdq1 => 0,
    }
}

fn read_fixed(payload: &[u8], at: usize, trailer: Trailer) -> Result<FixedRows, Violation> {
    check_common(payload, at, trailer, &[1])?;
    check_reserved(payload, at, trailer, FixedHeaderField::Reserved.offset())?;
    let declared_len = u32_at(payload, at + FixedHeaderField::SegmentLen.offset()) as usize;
    let row_count = u32_at(payload, at + FixedHeaderField::RowCount.offset()) as usize;
    let rows_start = checked_add(Section::TrailerBody(trailer), at, at, FIXED_HEADER_LEN)?;
    let rows = checked_span(
        Section::TrailerBody(trailer),
        at,
        rows_start,
        row_count,
        fixed_row_len(trailer),
    )?;
    let extent = declared_extent(payload, at, trailer, declared_len, rows.end)?;
    Ok(FixedRows {
        trailer,
        version: 1,
        extent,
        row_count,
        rows,
    })
}

fn read_plic(payload: &[u8], at: usize) -> Result<PlicHeader, Violation> {
    let trailer = Trailer::Plic;
    let preview = check_common(payload, at, trailer, &[1, 2])?;
    check_reserved(payload, at, trailer, PlicHeaderField::Reserved.offset())?;
    let version = PlicVersion::from_wire(preview.version).ok_or_else(|| {
        Violation::new(
            at,
            ViolationKind::UnknownVersion {
                trailer,
                version: preview.version,
            },
        )
    })?;
    let country_count = u32_at(payload, at + PlicHeaderField::CountryRows.offset()) as usize;
    let item_count_u32 = u32_at(payload, at + PlicHeaderField::IdRows.offset());
    let item_count = item_count_u32 as usize;
    let countries_start = checked_add(Section::TrailerBody(trailer), at, at, FIXED_HEADER_LEN)?;
    let countries = checked_span(
        Section::TrailerBody(trailer),
        at,
        countries_start,
        country_count,
        PLIC_COUNTRY_ROW_LEN,
    )?;
    need(
        payload.len(),
        at,
        Section::TrailerBody(trailer),
        countries.end,
    )?;
    let items = checked_span(
        Section::TrailerBody(trailer),
        at,
        countries.end,
        item_count,
        version.item_len(),
    )?;
    let mut summed_item_count = 0usize;
    let mut row_at = countries.start;
    let mut i = 0;
    while i < country_count {
        let count = u32_at(payload, row_at + PlicCountryField::Count.offset()) as usize;
        summed_item_count = summed_item_count.saturating_add(count);
        row_at += PLIC_COUNTRY_ROW_LEN;
        i += 1;
    }
    need(payload.len(), at, Section::TrailerBody(trailer), items.end)?;
    Ok(PlicHeader {
        version,
        extent: DerivedExtent {
            computed_len: checked_sub(Section::TrailerBody(trailer), at, items.end, at)?,
            end: items.end,
        },
        country_count,
        item_count,
        summed_item_count,
        item_len: version.item_len(),
        countries,
        items,
    })
}

fn read_pcv(payload: &[u8], at: usize) -> Result<PcvHeader, Violation> {
    let trailer = Trailer::Pcv1;
    check_common(payload, at, trailer, &[1])?;
    check_reserved(payload, at, trailer, PcvHeaderField::Reserved.offset())?;
    let declared_len = u32_at(payload, at + PcvHeaderField::SegmentLen.offset()) as usize;
    let country_count = u32_at(payload, at + PcvHeaderField::CountryCount.offset()) as usize;
    let variant_count = u32_at(payload, at + PcvHeaderField::VariantCount.offset()) as usize;
    let reservation_count =
        u32_at(payload, at + PcvHeaderField::ReservationCount.offset()) as usize;
    let countries_start = checked_add(Section::TrailerBody(trailer), at, at, PCV_HEADER_LEN)?;
    let countries = checked_span(
        Section::TrailerBody(trailer),
        at,
        countries_start,
        country_count,
        PCV_COUNTRY_ROW_LEN,
    )?;
    let variants = checked_span(
        Section::TrailerBody(trailer),
        at,
        countries.end,
        variant_count,
        PCV_VARIANT_ROW_LEN,
    )?;
    let reservations = checked_span(
        Section::TrailerBody(trailer),
        at,
        variants.end,
        reservation_count,
        PCV_RESERVATION_ROW_LEN,
    )?;
    let extent = declared_extent(payload, at, trailer, declared_len, reservations.end)?;
    Ok(PcvHeader {
        extent,
        country_count,
        variant_count,
        reservation_count,
        countries,
        variants,
        reservations,
    })
}

fn read_pdq(payload: &[u8], at: usize) -> Result<PdqHeader, Violation> {
    let trailer = Trailer::Pdq1;
    check_common(payload, at, trailer, &[1])?;
    check_reserved(payload, at, trailer, PdqHeaderField::Reserved.offset())?;
    let declared_len = u32_at(payload, at + PdqHeaderField::SegmentLen.offset()) as usize;
    let codec_len = u32_at(payload, at + PdqHeaderField::CodecPayloadLen.offset()) as usize;
    let codec_start = checked_add(Section::TrailerBody(trailer), at, at, FIXED_HEADER_LEN)?;
    let codec_payload = checked_span(Section::TrailerBody(trailer), at, codec_start, 1, codec_len)?;
    let extent = declared_extent(payload, at, trailer, declared_len, codec_payload.end)?;
    Ok(PdqHeader {
        extent,
        codec_payload,
    })
}

pub(crate) fn read_trailer_headers(
    payload: &[u8],
    at: usize,
    trailer: Trailer,
) -> Result<TrailerHeaders, Violation> {
    match trailer {
        Trailer::Plic => Ok(TrailerHeaders::Plic(read_plic(payload, at)?)),
        Trailer::Pvj1 => Ok(TrailerHeaders::Pvj1(read_fixed(payload, at, trailer)?)),
        Trailer::Pfr1 => Ok(TrailerHeaders::Pfr1(read_fixed(payload, at, trailer)?)),
        Trailer::Ppr1 => Ok(TrailerHeaders::Ppr1(read_fixed(payload, at, trailer)?)),
        Trailer::Pls1 => Ok(TrailerHeaders::Pls1(read_fixed(payload, at, trailer)?)),
        Trailer::Pef1 => Ok(TrailerHeaders::Pef1(read_fixed(payload, at, trailer)?)),
        Trailer::Pmi1 => Ok(TrailerHeaders::Pmi1(read_fixed(payload, at, trailer)?)),
        Trailer::Pcp1 => Ok(TrailerHeaders::Pcp1(read_fixed(payload, at, trailer)?)),
        Trailer::Pcv1 => Ok(TrailerHeaders::Pcv1(read_pcv(payload, at)?)),
        Trailer::Pdq1 => Ok(TrailerHeaders::Pdq1(read_pdq(payload, at)?)),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LineKey {
    pub tag_db_id: u32,
    pub kind: u32,
    pub line_id: i32,
}

fn line_key_at(payload: &[u8], at: usize) -> LineKey {
    LineKey {
        tag_db_id: u32_at(payload, at + LineKeyField::TagDbId.offset()),
        kind: u32_at(payload, at + LineKeyField::Kind.offset()),
        line_id: i32_at(payload, at + LineKeyField::LineId.offset()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PvjRow {
    pub key: LineKey,
    pub equipment_variant_id: u32,
}
pub fn pvj_row_at(payload: &[u8], at: usize) -> Result<PvjRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pvj1), 16)?;
    Ok(PvjRow {
        key: line_key_at(payload, at),
        equipment_variant_id: u32_at(payload, at + PvjField::EquipmentVariantId.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PfrRow {
    pub key: LineKey,
    pub requested_factories: i32,
}
pub fn pfr_row_at(payload: &[u8], at: usize) -> Result<PfrRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pfr1), 16)?;
    Ok(PfrRow {
        key: line_key_at(payload, at),
        requested_factories: i32_at(payload, at + PfrField::RequestedFactories.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PprRow {
    pub key: LineKey,
    pub priority: i32,
}
pub fn ppr_row_at(payload: &[u8], at: usize) -> Result<PprRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Ppr1), 16)?;
    Ok(PprRow {
        key: line_key_at(payload, at),
        priority: i32_at(payload, at + PprField::Priority.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlsRow {
    pub key: LineKey,
    pub subtype: u32,
}
pub fn pls_row_at(payload: &[u8], at: usize) -> Result<PlsRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pls1), 16)?;
    Ok(PlsRow {
        key: line_key_at(payload, at),
        subtype: u32_at(payload, at + PlsField::Subtype.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PmiRow {
    pub key: LineKey,
    pub mio_id: u32,
}
pub fn pmi_row_at(payload: &[u8], at: usize) -> Result<PmiRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pmi1), 16)?;
    Ok(PmiRow {
        key: line_key_at(payload, at),
        mio_id: u32_at(payload, at + PmiField::MioId.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PefRow {
    pub key: LineKey,
    pub efficiency_raw: i64,
    pub efficiency_cap_raw: i64,
    pub efficiency_growth_raw: i64,
}
pub fn pef_row_at(payload: &[u8], at: usize) -> Result<PefRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pef1), 36)?;
    Ok(PefRow {
        key: line_key_at(payload, at),
        efficiency_raw: i64_at(payload, at + PefField::EfficiencyRaw.offset()),
        efficiency_cap_raw: i64_at(payload, at + PefField::EfficiencyCapRaw.offset()),
        efficiency_growth_raw: i64_at(payload, at + PefField::EfficiencyGrowthRaw.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcpRow {
    pub tag_db_id: u32,
    pub flags: u32,
}
pub fn pcp_row_at(payload: &[u8], at: usize) -> Result<PcpRow, Violation> {
    need_row(payload, at, Section::TrailerBody(Trailer::Pcp1), 8)?;
    Ok(PcpRow {
        tag_db_id: u32_at(payload, at + PcpField::TagDbId.offset()),
        flags: u32_at(payload, at + PcpField::Flags.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicCountryRow {
    pub tag_db_id: u32,
    pub count: u32,
}
pub fn plic_country_row_at(payload: &[u8], at: usize) -> Result<PlicCountryRow, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Plic),
        PLIC_COUNTRY_ROW_LEN,
    )?;
    Ok(PlicCountryRow {
        tag_db_id: u32_at(payload, at + PlicCountryField::TagDbId.offset()),
        count: u32_at(payload, at + PlicCountryField::Count.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicV1Item {
    pub variant_id: u32,
}
pub fn plic_v1_item_at(payload: &[u8], at: usize) -> Result<PlicV1Item, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Plic),
        PLIC_V1_ITEM_LEN,
    )?;
    Ok(PlicV1Item {
        variant_id: u32_at(payload, at + PlicV1ItemField::VariantId.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicV2Item {
    pub variant_id: u32,
    pub visible_order: u32,
    pub creator_tag_db_id: u32,
    pub origin_tag_db_id: u32,
}
pub fn plic_v2_item_at(payload: &[u8], at: usize) -> Result<PlicV2Item, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Plic),
        PLIC_V2_ITEM_LEN,
    )?;
    Ok(PlicV2Item {
        variant_id: u32_at(payload, at + PlicV2ItemField::VariantId.offset()),
        visible_order: u32_at(payload, at + PlicV2ItemField::VisibleOrder.offset()),
        creator_tag_db_id: u32_at(payload, at + PlicV2ItemField::CreatorTagDbId.offset()),
        origin_tag_db_id: u32_at(payload, at + PlicV2ItemField::OriginTagDbId.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvCountryRow {
    pub tag_db_id: u32,
    pub flags: u32,
    pub owned_raw: i64,
    pub assigned: i32,
    pub available: i32,
    pub variant_start: u32,
    pub variant_len: u32,
    pub reservation_start: u32,
    pub reservation_len: u32,
}
pub fn pcv_country_row_at(payload: &[u8], at: usize) -> Result<PcvCountryRow, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Pcv1),
        PCV_COUNTRY_ROW_LEN,
    )?;
    Ok(PcvCountryRow {
        tag_db_id: u32_at(payload, at + PcvCountryField::TagDbId.offset()),
        flags: u32_at(payload, at + PcvCountryField::Flags.offset()),
        owned_raw: i64_at(payload, at + PcvCountryField::OwnedRaw.offset()),
        assigned: i32_at(payload, at + PcvCountryField::Assigned.offset()),
        available: i32_at(payload, at + PcvCountryField::Available.offset()),
        variant_start: u32_at(payload, at + PcvCountryField::VariantStart.offset()),
        variant_len: u32_at(payload, at + PcvCountryField::VariantLen.offset()),
        reservation_start: u32_at(payload, at + PcvCountryField::ReservationStart.offset()),
        reservation_len: u32_at(payload, at + PcvCountryField::ReservationLen.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvVariantRow {
    pub variant_id: u32,
    pub equip_type: u32,
    pub amount_raw: i64,
}
pub fn pcv_variant_row_at(payload: &[u8], at: usize) -> Result<PcvVariantRow, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Pcv1),
        PCV_VARIANT_ROW_LEN,
    )?;
    Ok(PcvVariantRow {
        variant_id: u32_at(payload, at + PcvVariantField::VariantId.offset()),
        equip_type: u32_at(payload, at + PcvVariantField::EquipType.offset()),
        amount_raw: i64_at(payload, at + PcvVariantField::AmountRaw.offset()),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvReservationRow {
    pub priority: i32,
    pub count: i32,
}
pub fn pcv_reservation_row_at(payload: &[u8], at: usize) -> Result<PcvReservationRow, Violation> {
    need_row(
        payload,
        at,
        Section::TrailerBody(Trailer::Pcv1),
        PCV_RESERVATION_ROW_LEN,
    )?;
    Ok(PcvReservationRow {
        priority: i32_at(payload, at + PcvReservationField::Priority.offset()),
        count: i32_at(payload, at + PcvReservationField::Count.offset()),
    })
}
