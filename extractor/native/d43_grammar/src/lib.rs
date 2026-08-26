#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod decode;
pub mod encode;
pub mod oil_trio;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    max_resource_count: u32,
    identity_stride: usize,
}

impl Limits {
    pub const fn new(max_resource_count: u32, identity_stride: usize) -> Self {
        Limits {
            max_resource_count,
            identity_stride,
        }
    }

    pub const fn max_resource_count(self) -> u32 {
        self.max_resource_count
    }

    pub const fn identity_stride(self) -> usize {
        self.identity_stride
    }
}

pub const CORE_HEADER_LEN: usize = 48;

pub const CORE_ROW_LEN: usize = 48;

/// **This enum is a drift boundary.** Every offset is generated from it by an
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CoreWord {
    RowCount,
    ChangedRows,
    StatusFlags,
    ObservedContractRows,
    EmittedContractRows,
    BuyerPtrValidRows,
    VariantValidRows,
    AmountValidRows,
    PartialFieldRows,
    ScaleRaw,
}

impl CoreWord {
    /// Every core header word, in wire order. The whole block is generated from
    pub const ALL: [CoreWord; 10] = [
        CoreWord::RowCount,
        CoreWord::ChangedRows,
        CoreWord::StatusFlags,
        CoreWord::ObservedContractRows,
        CoreWord::EmittedContractRows,
        CoreWord::BuyerPtrValidRows,
        CoreWord::VariantValidRows,
        CoreWord::AmountValidRows,
        CoreWord::PartialFieldRows,
        CoreWord::ScaleRaw,
    ];

    pub const fn offset(self) -> usize {
        match self {
            CoreWord::RowCount => 8,
            CoreWord::ChangedRows => 12,
            CoreWord::StatusFlags => 16,
            CoreWord::ObservedContractRows => 20,
            CoreWord::EmittedContractRows => 24,
            CoreWord::BuyerPtrValidRows => 28,
            CoreWord::VariantValidRows => 32,
            CoreWord::AmountValidRows => 36,
            CoreWord::PartialFieldRows => 40,
            CoreWord::ScaleRaw => 44,
        }
    }

    pub const fn diagnostic_name(self) -> &'static str {
        match self {
            CoreWord::RowCount => "row_count",
            CoreWord::ChangedRows => "changed_rows",
            CoreWord::StatusFlags => "status_flags",
            CoreWord::ObservedContractRows => "observed_contract_rows",
            CoreWord::EmittedContractRows => "emitted_contract_rows",
            CoreWord::BuyerPtrValidRows => "buyer_ptr_valid_rows",
            CoreWord::VariantValidRows => "variant_valid_rows",
            CoreWord::AmountValidRows => "amount_valid_rows",
            CoreWord::PartialFieldRows => "partial_field_rows",
            CoreWord::ScaleRaw => "scale_raw",
        }
    }
}

const _: () = {
    let mut i = 0;
    while i < CoreWord::ALL.len() {
        if CoreWord::ALL[i].offset() != 8 + i * 4 {
            panic!("d43_grammar: CoreWord::ALL does not tile the core header");
        }
        i += 1;
    }
    if 8 + CoreWord::ALL.len() * 4 != CORE_HEADER_LEN {
        panic!("d43_grammar: CoreWord::ALL does not fill the core header");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum CoreRowField {
    ContractId,
    ContractType,
    Flags,
    SellerTagDbId,
    BuyerTagDbId,
    EquipmentVariantId,
    AmountRaw,
    EquipmentSubbufStatus,
    DeliverySubbufStatus,
    BuyerPtrStatus,
    Reserved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Width {
    U32,
    I32,
    I64,
    U64,
}

impl Width {
    pub const fn bytes(self) -> usize {
        match self {
            Width::U32 | Width::I32 => 4,
            Width::I64 | Width::U64 => 8,
        }
    }
}

impl CoreRowField {
    pub const ALL: [CoreRowField; 11] = [
        CoreRowField::ContractId,
        CoreRowField::ContractType,
        CoreRowField::Flags,
        CoreRowField::SellerTagDbId,
        CoreRowField::BuyerTagDbId,
        CoreRowField::EquipmentVariantId,
        CoreRowField::AmountRaw,
        CoreRowField::EquipmentSubbufStatus,
        CoreRowField::DeliverySubbufStatus,
        CoreRowField::BuyerPtrStatus,
        CoreRowField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            CoreRowField::ContractId => 0,
            CoreRowField::ContractType => 4,
            CoreRowField::Flags => 8,
            CoreRowField::SellerTagDbId => 12,
            CoreRowField::BuyerTagDbId => 16,
            CoreRowField::EquipmentVariantId => 20,
            CoreRowField::AmountRaw => 24,
            CoreRowField::EquipmentSubbufStatus => 32,
            CoreRowField::DeliverySubbufStatus => 36,
            CoreRowField::BuyerPtrStatus => 40,
            CoreRowField::Reserved => 44,
        }
    }

    pub const fn width(self) -> Width {
        match self {
            CoreRowField::AmountRaw => Width::I64,
            CoreRowField::ContractId
            | CoreRowField::ContractType
            | CoreRowField::Flags
            | CoreRowField::SellerTagDbId
            | CoreRowField::BuyerTagDbId
            | CoreRowField::EquipmentVariantId
            | CoreRowField::EquipmentSubbufStatus
            | CoreRowField::DeliverySubbufStatus
            | CoreRowField::BuyerPtrStatus
            | CoreRowField::Reserved => Width::U32,
        }
    }
}

const _: () = {
    let mut expected = 0;
    let mut i = 0;
    while i < CoreRowField::ALL.len() {
        if CoreRowField::ALL[i].offset() != expected {
            panic!("d43_grammar: CoreRowField::ALL does not tile the core row");
        }
        expected += CoreRowField::ALL[i].width().bytes();
        i += 1;
    }
    if expected != CORE_ROW_LEN {
        panic!("d43_grammar: CoreRowField::ALL does not fill the core row");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RowFlag {
    BuyerPtrValid,
    EquipmentSubbufValid,
    DeliverySubbufValid,
    VariantValid,
    AmountValid,
}

impl RowFlag {
    pub const ALL: [RowFlag; 5] = [
        RowFlag::BuyerPtrValid,
        RowFlag::EquipmentSubbufValid,
        RowFlag::DeliverySubbufValid,
        RowFlag::VariantValid,
        RowFlag::AmountValid,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            RowFlag::BuyerPtrValid => 1 << 0,
            RowFlag::EquipmentSubbufValid => 1 << 1,
            RowFlag::DeliverySubbufValid => 1 << 2,
            RowFlag::VariantValid => 1 << 3,
            RowFlag::AmountValid => 1 << 4,
        }
    }

    pub const fn is_set(self, flags: u32) -> bool {
        flags & self.bit() != 0
    }
}

pub const KNOWN_ROW_FLAGS: u32 = known_row_flags();

const fn known_row_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < RowFlag::ALL.len() {
        mask |= RowFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

const _: () = {
    let mut i = 0;
    let mut seen = 0u32;
    while i < RowFlag::ALL.len() {
        let bit = RowFlag::ALL[i].bit();
        if bit == 0 || bit & (bit - 1) != 0 {
            panic!("d43_grammar: a RowFlag is not a single bit");
        }
        if seen & bit != 0 {
            panic!("d43_grammar: two RowFlags share a bit");
        }
        seen |= bit;
        i += 1;
    }
};

pub const CORE_CONTRACT_TYPE: u32 = 81;

pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    CoreHeader,
    CoreRows,
    TrailerMagic,
    TrailerHeader(Trailer),
    TrailerBody(Trailer),
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
    RowLen {
        trailer: Trailer,
        declared: usize,
        expected: usize,
    },
    ResourceCount {
        trailer: Trailer,
        declared: u32,
        max: u32,
    },
    IdentityStride {
        trailer: Trailer,
        stride: usize,
    },
    AbsentBlockDeclared {
        trailer: Trailer,
        declared: u32,
    },
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    TrailingBytes {
        consumed: usize,
        available: usize,
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
}

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

fn add(section: Section, at: usize, len: usize) -> Result<usize, Violation> {
    at.checked_add(len)
        .ok_or_else(|| Violation::new(at, ViolationKind::OffsetOverflow { section, at, len }))
}

fn mul(section: Section, at: usize, count: usize, stride: usize) -> Result<usize, Violation> {
    count.checked_mul(stride).ok_or_else(|| {
        Violation::new(
            at,
            ViolationKind::OffsetOverflow {
                section,
                at,
                len: count,
            },
        )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreHeader {
    pub elapsed_wall_ms: u64,
    words: [u32; CoreWord::ALL.len()],
}

impl CoreHeader {
    pub fn word(&self, word: CoreWord) -> u32 {
        self.words[(word.offset() - 8) / 4]
    }

    pub fn words(&self) -> [u32; CoreWord::ALL.len()] {
        self.words
    }

    pub fn row_count(&self) -> u32 {
        self.word(CoreWord::RowCount)
    }
}

pub fn read_core_header(payload: &[u8]) -> Result<CoreHeader, Violation> {
    if payload.len() < CORE_HEADER_LEN {
        return Err(Violation::new(
            0,
            ViolationKind::Need {
                section: Section::CoreHeader,
                needed: CORE_HEADER_LEN,
                available: payload.len(),
            },
        ));
    }
    let mut words = [0u32; CoreWord::ALL.len()];
    let mut i = 0;
    while i < CoreWord::ALL.len() {
        words[i] = u32_at(payload, CoreWord::ALL[i].offset());
        i += 1;
    }
    Ok(CoreHeader {
        elapsed_wall_ms: u64_at(payload, 0),
        words,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreRow {
    pub contract_id: u32,
    pub contract_type: u32,
    pub flags: u32,
    pub seller_tag_db_id: u32,
    pub buyer_tag_db_id: u32,
    pub equipment_variant_id: u32,
    pub amount_raw: i64,
    pub equipment_subbuf_status: u32,
    pub delivery_subbuf_status: u32,
    pub buyer_ptr_status: u32,
    pub reserved: u32,
}

fn core_row_at(payload: &[u8], at: usize) -> CoreRow {
    CoreRow {
        contract_id: u32_at(payload, at + CoreRowField::ContractId.offset()),
        contract_type: u32_at(payload, at + CoreRowField::ContractType.offset()),
        flags: u32_at(payload, at + CoreRowField::Flags.offset()),
        seller_tag_db_id: u32_at(payload, at + CoreRowField::SellerTagDbId.offset()),
        buyer_tag_db_id: u32_at(payload, at + CoreRowField::BuyerTagDbId.offset()),
        equipment_variant_id: u32_at(payload, at + CoreRowField::EquipmentVariantId.offset()),
        amount_raw: i64_at(payload, at + CoreRowField::AmountRaw.offset()),
        equipment_subbuf_status: u32_at(payload, at + CoreRowField::EquipmentSubbufStatus.offset()),
        delivery_subbuf_status: u32_at(payload, at + CoreRowField::DeliverySubbufStatus.offset()),
        buyer_ptr_status: u32_at(payload, at + CoreRowField::BuyerPtrStatus.offset()),
        reserved: u32_at(payload, at + CoreRowField::Reserved.offset()),
    }
}

/// The bound check is here rather than in the cursor because all four sites
pub fn core_end(payload: &[u8], row_count: u32) -> Result<usize, Violation> {
    let rows = mul(
        Section::CoreRows,
        CORE_HEADER_LEN,
        row_count as usize,
        CORE_ROW_LEN,
    )?;
    let end = add(Section::CoreRows, CORE_HEADER_LEN, rows)?;
    if payload.len() < end {
        return Err(Violation::new(
            CORE_HEADER_LEN,
            ViolationKind::Need {
                section: Section::CoreRows,
                needed: end,
                available: payload.len(),
            },
        ));
    }
    Ok(end)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreRowStep {
    Row { index: usize, row: CoreRow },
    End(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct CoreRowCursor {
    at: usize,
    index: usize,
    row_count: usize,
}

impl CoreRowCursor {
    pub const fn new(row_count: u32) -> Self {
        CoreRowCursor {
            at: CORE_HEADER_LEN,
            index: 0,
            row_count: row_count as usize,
        }
    }

    pub const fn offset(&self) -> usize {
        self.at
    }

    pub fn next(&mut self, payload: &[u8]) -> CoreRowStep {
        if self.index >= self.row_count {
            return CoreRowStep::End(self.at);
        }
        let row = core_row_at(payload, self.at);
        let step = CoreRowStep::Row {
            index: self.index,
            row,
        };
        self.at += CORE_ROW_LEN;
        self.index += 1;
        step
    }
}

/// generated from [`Trailer::ALL`] by exhaustive `match`es with no wildcard arm,
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Trailer {
    Detail,
    ResourceFlow,
    FactoryHeader,
    ResourceRow,
    ClassicDeal,
    Activation,
    Stockpile,
    Residual,
    LendLease,
}

impl Trailer {
    pub const ALL: [Trailer; 9] = [
        Trailer::Detail,
        Trailer::ResourceFlow,
        Trailer::FactoryHeader,
        Trailer::ResourceRow,
        Trailer::ClassicDeal,
        Trailer::Activation,
        Trailer::Stockpile,
        Trailer::Residual,
        Trailer::LendLease,
    ];

    pub const fn magic(self) -> &'static [u8; 4] {
        match self {
            Trailer::Detail => b"MTD1",
            Trailer::ResourceFlow => b"MTRF",
            Trailer::FactoryHeader => b"MTRH",
            Trailer::ResourceRow => b"MTRR",
            Trailer::ClassicDeal => b"MTRD",
            Trailer::Activation => b"MTRA",
            Trailer::Stockpile => b"MTS1",
            Trailer::Residual => b"MTRB",
            Trailer::LendLease => b"MTLL",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Trailer::Detail => "MTD1",
            Trailer::ResourceFlow => "MTRF",
            Trailer::FactoryHeader => "MTRH",
            Trailer::ResourceRow => "MTRR",
            Trailer::ClassicDeal => "MTRD",
            Trailer::Activation => "MTRA",
            Trailer::Stockpile => "MTS1",
            Trailer::Residual => "MTRB",
            Trailer::LendLease => "MTLL",
        }
    }

    pub const fn header_len(self) -> usize {
        match self {
            Trailer::Detail => 32,
            Trailer::ResourceFlow => 32,
            Trailer::FactoryHeader => 24,
            Trailer::ResourceRow => 32,
            Trailer::ClassicDeal => 32,
            Trailer::Activation => 32,
            Trailer::Stockpile => 40,
            Trailer::Residual => 32,
            Trailer::LendLease => 32,
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
        panic!("d43_grammar: a Trailer is not in Trailer::ALL")
    }
}

const _: () = {
    let mut i = 0;
    while i < Trailer::ALL.len() {
        let a = Trailer::ALL[i].magic();
        let label = Trailer::ALL[i].label().as_bytes();
        if label.len() != 4
            || label[0] != a[0]
            || label[1] != a[1]
            || label[2] != a[2]
            || label[3] != a[3]
        {
            panic!("d43_grammar: a Trailer's label does not spell its magic");
        }
        let mut j = i + 1;
        while j < Trailer::ALL.len() {
            let b = Trailer::ALL[j].magic();
            if a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3] {
                panic!("d43_grammar: two Trailers share a magic");
            }
            j += 1;
        }
        i += 1;
    }
    if Trailer::ALL.len() > 16 {
        panic!("d43_grammar: the trailer alphabet no longer fits TrailerCursor's seen mask");
    }
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrailerStep {
    Magic { at: usize, trailer: Trailer },
    End(usize),
}

#[derive(Debug)]
#[must_use]
pub struct TrailerCursor {
    at: usize,
    pending: Option<(usize, Trailer)>,
    seen: u16,
}

impl TrailerCursor {
    pub const fn new(at: usize) -> Self {
        TrailerCursor {
            at,
            pending: None,
            seen: 0,
        }
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
        let end = add(Section::TrailerMagic, self.at, 4)?;
        if end > payload.len() {
            return Err(Violation::new(
                self.at,
                ViolationKind::Need {
                    section: Section::TrailerMagic,
                    needed: end,
                    available: payload.len(),
                },
            ));
        }
        let Some(trailer) = Trailer::from_magic(&payload[self.at..end]) else {
            return Err(Violation::new(
                self.at,
                ViolationKind::UnknownTrailerMagic { at: self.at },
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

    pub(crate) fn peek_header(&self, payload: &[u8]) -> Result<HeaderPreview, Violation> {
        let Some((at, trailer)) = self.pending else {
            return Err(Violation::caller(self.at, CallerDefect::BodyNotDelimited));
        };
        read_header_preview(payload, at, trailer)
    }

    pub fn read_body(self, payload: &[u8], limits: Limits) -> Result<TrailerBody, Violation> {
        let Some((at, trailer)) = self.pending else {
            return Err(Violation::caller(self.at, CallerDefect::BodyNotDelimited));
        };
        let headers = read_trailer_body(payload, at, trailer, limits)?;
        Ok(match headers {
            TrailerHeaders::Detail(h) => TrailerBody::Detail(DetailCursor::new(h, self)),
            TrailerHeaders::ResourceFlow(h) => {
                TrailerBody::ResourceFlow(ResourceFlowCursor::new(h, self))
            }
            TrailerHeaders::FactoryHeader(h) => {
                TrailerBody::FactoryHeader(FactoryHeaderCursor::new(h, self))
            }
            TrailerHeaders::ResourceRow(h) => {
                TrailerBody::ResourceRow(ResourceRowCursor::new(h, self))
            }
            TrailerHeaders::ClassicDeal(h) => {
                TrailerBody::ClassicDeal(ClassicDealCursor::new(h, self))
            }
            TrailerHeaders::Activation(h) => {
                TrailerBody::Activation(ActivationCursor::new(h, self))
            }
            TrailerHeaders::Stockpile(h) => TrailerBody::Stockpile(StockpileCursor::new(h, self)),
            TrailerHeaders::Residual(h) => TrailerBody::Residual(ResidualCursor::new(h, self)),
            TrailerHeaders::LendLease(h) => TrailerBody::LendLease(LendLeaseCursor::new(h, self)),
        })
    }

    pub(crate) fn advance_past(&mut self, trailer: Trailer, end: usize) {
        self.at = end;
        self.pending = None;
        self.seen |= trailer.seen_bit();
    }
}

pub fn exact_consumption(consumed: usize, payload_len: usize) -> Result<(), Violation> {
    if consumed != payload_len {
        return Err(Violation::new(
            consumed,
            ViolationKind::TrailingBytes {
                consumed,
                available: payload_len,
            },
        ));
    }
    Ok(())
}

mod family;
pub use family::*;

mod walk;
pub use walk::*;
