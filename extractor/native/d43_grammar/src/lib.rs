//! Domain 43 (`market_trade_live`) payload grammar — the single structural walk.
//!
//! # Why this crate exists
//!
//! D43's DATA payload grammar was transcribed at FOUR shipping sites: the
//! collector's packer wrote it
//! (`attach_log.rs::compact_market_trade_data_payload` and its seven
//! `append_compact_market_trade_*_trailer` helpers),
//! `recorder_host::world_fold::fold_market_trade` folded it through eight
//! `parse_market_trade_*` helpers,
//! `recorder_host::domain_data_authority::decode_compact_market_trade_*` gated
//! it, and `market_trade_live_lane::decode_payload` walked it for the gate
//! tool and wrote it back out through that crate's `encode_*` functions.
//!
//! The survey counted **122 independently duplicated layout facts** across
//! those sites and 16 trailer-magic identifiers for nine magics — the largest
//! drift surface in the tree (`runs/ko_domain_grammars/survey.md`). That is the
//! shape that failed for d13 on 2026-08-14: a lane added a tail at the producer
//! and one reader, two other readers never learned it, and one of them refused
//! 59 of 59 live records of the domain.
//!
//! # What is grammar here, and what is not
//!
//! Grammar (this crate): every magic, every legal version of every family, the
//! width and field offsets of the core header and the core row, each trailer
//! header's width and field offsets, the per-version row width, the presence and
//! length of MTRR's identity block and MTRD's witness and incomplete-owner
//! blocks, MTS1's two row segments, the flag/kind/direction/route vocabularies,
//! and exact consumption.
//!
//! Payload-only semantics are also single-source here: [`decode::decode_record`]
//! evaluates the shared id ranges, row invariants, counted-header checks,
//! identity-entry contract, oil-row model, deal half-join, and cross-family
//! witness rules without letting any of them influence cursor advancement.
//! Findings are data, and fatality plus fold-compatible policy order are stated
//! once in [`decode`]. Sites still own text formatting, non-payload state
//! application, and projection policy. In particular, mapping resource slots
//! onto a game-file-derived resource-name table is not a payload fact and does
//! not belong here.
//!
//! ## A version word is STRUCTURAL here, unlike d68
//!
//! d68's grammar had one shape. D43's version words change widths:
//!
//! * MTRF v1/v2 rows are a fixed 72 bytes; v3 rows are `16 + 8 * resource_count`
//!   and the count is read from the header word at `+24`, which in v1/v2 is a
//!   seven-bit resource MASK instead.
//! * MTRR v3/v4 carry a resource-identity block between the header and the rows;
//!   v1/v2 do not, and hold that header word at zero.
//! * MTRD v3/v4 carry an incomplete-owner block behind the witnesses; v1/v2 do
//!   not, and must hold that count at zero. v4 also spells the deal identity as
//!   three full-width words where v1–v3 pack it into one `u64`, inside the same
//!   96-byte row.
//!
//! So [`FlowVersion`], [`ResourceRowVersion`] and [`DealVersion`] are part of
//! this crate's tables, an unknown version is a structural refusal (a payload
//! whose width is undecidable), and adding a version is a compile error at every
//! site that matches on one.
//!
//! # Caller-owned budgets this crate does not own
//!
//! An MTRF v3 row's WIDTH is `16 + 8 * resource_count`, and MTRR's identity
//! block is `resource_count * stride` bytes. The bound on `resource_count` and
//! the stride are declared in `compact_contract::build_profile`
//! (`MAX_COMPACT_STRATEGIC_RESOURCE_COUNT`,
//! `COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE`) as PRICED BUDGETS, with the
//! owner's rule that raising either is a contract change. This crate has no
//! dependencies — that is what lets all four sites share it — so it cannot
//! import them, and restating them here would create the seventh copy of a
//! number this programme exists to have one of.
//!
//! They are therefore taken as an argument: [`Limits`], which every site builds
//! from those two `compact_contract` constants. The eager decoder additionally
//! takes [`decode::DecodeContext`] for all payload-policy ceilings. The values
//! stay single-source in `compact_contract`; layout arithmetic and payload-rule
//! evaluation are single-source here, and neither argument has a default.
//!
//! # One eager decoder, enforced by lazy owned cursors
//!
//! The low-level trailer walk remains a two-step PULL cursor:
//!
//! 1. [`TrailerCursor::next`] delimits and NAMES the next magic and reads
//!    nothing else;
//! 2. [`TrailerCursor::read_body`] reads that family's header and computes its
//!    extent, and only then advances.
//!
//! Two steps rather than one because the fold and the authority refuse an
//! unaccepted magic BEFORE any header is parsed (the fold accepts seven of the
//! nine magics this grammar knows; the lane gate accepts all nine), and a cursor
//! that parsed the header first would move that refusal behind a header
//! diagnostic the site never emitted. `End` carries the finishing offset rather
//! than a separate `finish()`, which would need a misuse state.
//!
//! [`decode::decode_record`] is the one eager structural driver layered on top
//! of those cursors. It consumes every declared block to obtain each
//! [`TrailerResume`], retains all semantic findings already available, and
//! stops only when a later byte boundary is unknowable. Its report therefore
//! supports partial decode. [`decode::select_first_fatal`] selects the record
//! verdict by explicit site-compatible policy order rather than discovery or
//! wire order; sites may apply their non-payload gates at the documented staged
//! boundary without restating a payload rule.
//!
//! # Violations are data, never text
//!
//! The four sites spell the same violation four ways and always have: the fold
//! says `"market trade MTRF len 40 < 72"`, the validator says `"offset 900:
//! market trade MTRF len 40 < 72"`, the lane says `"offset 900: market trade
//! MTRF trailer is truncated"`. [`Violation`] carries the operands and no
//! message; each site owns one exhaustive formatter. An `ErrorStyle` knob inside
//! this crate would make a supposedly neutral leaf know its consumers by name.
//!
//! # Row readers are DESTRUCTURED at every site, on purpose
//!
//! [`CoreRow`], [`DetailRow`], [`FlowRow`], [`FactoryRow`], [`ResourceRow`],
//! [`DealRow`], [`StockpileCountryRow`] and [`StockpileItemRow`] have public
//! named fields and no private ones, and every consuming site destructures them
//! with a complete pattern and no `..`. That is the mechanism, exactly as
//! `d68_grammar::ScalarBlock::words()`'s fixed-size array is: adding a field to
//! a row here is a compile error at all four sites until each has decided what
//! it means.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod decode;
pub mod encode;

#[cfg(test)]
mod tests;

// =============================================================================
// The ceiling this crate does not own
// =============================================================================

/// The two `compact_contract::build_profile` budgets that decide a WIDTH.
///
/// See the module docs. Built by each site from
/// `MAX_COMPACT_STRATEGIC_RESOURCE_COUNT` and
/// `COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE`; this crate never invents a
/// default for either, because a default is exactly the second copy the
/// argument exists to avoid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    max_resource_count: u32,
    identity_stride: usize,
}

impl Limits {
    /// State the two budgets. There is deliberately no `Default`.
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

// =============================================================================
// The core payload: a 48-byte header and 48-byte purchase-contract rows
// =============================================================================

/// The core header: `elapsed_wall_ms` (u64) and ten `u32` words.
pub const CORE_HEADER_LEN: usize = 48;

/// One equipment purchase-contract row.
pub const CORE_ROW_LEN: usize = 48;

/// A `u32` word of the core header.
///
/// **This enum is a drift boundary.** Every offset is generated from it by an
/// exhaustive `match` with no wildcard arm, and [`CoreHeader::words`] hands back
/// a fixed-size array the sites destructure, so an eleventh word is a compile
/// error at all four of them.
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
    /// this.
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

    /// This word's byte offset from the start of the payload.
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

    /// The name every site already spells this word in its diagnostics.
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

/// The core header's words tile `8..CORE_HEADER_LEN` exactly, in ascending
/// order, four bytes each.
///
/// A `panic!` in a CONST context: this fails the BUILD, and there is no runtime
/// path through it. A leaf under a recorder that must fail closed does not get
/// to panic at runtime.
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

/// A field of one core purchase-contract row.
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

/// How wide a field is on the wire.
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

/// The core row's fields tile `0..CORE_ROW_LEN` exactly, with no gap and no
/// overlap. This is the tiling drift-killer for the core row, at build time.
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

/// A presence bit of a core row's `flags` word.
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

/// Every assigned core-row presence bit.
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

/// The row flags are distinct single bits — the drift-killer for the flag table.
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

/// The `contract_type` word every core row carries.
///
/// One value, and it is a wire constant rather than a semantic gate: it is the
/// engine's `market_trade` contract discriminant, and a row with any other value
/// is not this grammar's row.
pub const CORE_CONTRACT_TYPE: u32 = 81;

/// The fixed-point denominator every scaled `i64` on this wire is expressed in.
pub const FIXED_SCALE_RAW: u32 = 100_000;

/// The largest `amount_raw` this grammar can express, in fixed-point units.
pub const MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;

// =============================================================================
// Violations
// =============================================================================

/// Which part of the walk ran out of bytes, or refused a declared width.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    /// The 48-byte core header.
    CoreHeader,
    /// The core purchase-contract rows.
    CoreRows,
    /// The four magic bytes that name the next trailer.
    TrailerMagic,
    /// A named trailer's fixed header.
    TrailerHeader(Trailer),
    /// A named trailer's body: rows, and any block in front of them.
    TrailerBody(Trailer),
}

/// A structural refusal: the bytes needed to find the next boundary are absent,
/// or a declared width is one this grammar cannot express.
///
/// Everything here is fatal TO THE CURSOR at every site — the walk cannot find
/// the next boundary — which is the same split
/// `decode_compact_market_trade_data_payload` already draws: it accumulates
/// row-semantic diagnostics and keeps going, and stops the walk exactly when a
/// trailer can no longer be delimited. Every rule the four sites disagree about
/// the fatality of is a SEMANTIC rule and stayed with its site, so unlike
/// `d13_grammar` there is no `Recoverable` step here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    /// The payload is shorter than `needed` bytes.
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    /// The bytes at the cursor are not a magic this grammar knows.
    UnknownTrailerMagic { at: usize },
    /// A magic this grammar knows appeared twice in one payload.
    ///
    /// Structural, not stylistic: every family states a width, a count or an
    /// identity for the whole payload, so a second one describes a second game
    /// and there is no rule that says which of the two the rest of the payload
    /// is numbered against.
    DuplicateTrailer { trailer: Trailer, at: usize },
    /// A trailer declares a version this grammar cannot decide a width for.
    UnknownVersion { trailer: Trailer, version: u16 },
    /// A trailer's declared header width is not the one its version defines.
    HeaderLen {
        trailer: Trailer,
        declared: usize,
        expected: usize,
    },
    /// A trailer's declared row width is not the one its version and counts
    /// define.
    RowLen {
        trailer: Trailer,
        declared: usize,
        expected: usize,
    },
    /// An MTRF v3 or MTRR trailer declares a resource count outside
    /// `1..=`[`Limits::max_resource_count`], so no row width follows from it.
    ResourceCount {
        trailer: Trailer,
        declared: u32,
        max: u32,
    },
    /// An identity-bearing `MTRR` was given a stride an entry cannot fit.
    ///
    /// An entry is a `u16` declared length followed by that many bytes, so a
    /// stride below two cannot hold the length word and every slot in the block
    /// is unreadable. The stride is a caller-stated budget
    /// ([`Limits::identity_stride`]), so this refuses a CALLER's number rather
    /// than the payload's -- but it is refused here, at the one place the block
    /// is sized, because the alternative is a reader that computes `stride - 2`
    /// and underflows. A leaf under a recorder that must fail closed does not
    /// get to panic.
    IdentityStride { trailer: Trailer, stride: usize },
    /// A version that defines no block behind the header declared one anyway:
    /// MTRR v1/v2 with a non-zero identity stride, MTRD v1/v2 with a non-zero
    /// incomplete-owner count. The word means something else in the version
    /// that has it, so a decoder that ignored this would size the trailer under
    /// the wrong grammar and walk into the rows.
    AbsentBlockDeclared { trailer: Trailer, declared: u32 },
    /// An offset plus a length does not fit a `usize`.
    ///
    /// A variant rather than a `saturating_add` because a saturating walk
    /// reports a wrong boundary as a truncation, and a reader that must fail
    /// closed should say what actually happened.
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    /// The walk did not account for every byte of the payload.
    TrailingBytes { consumed: usize, available: usize },
    /// The CALLER walked wrong. Not a statement about the payload at all — see
    /// [`CallerDefect`]. It is an arm of this enum rather than a separate error
    /// type because every site already owns one exhaustive formatter over this
    /// enum, and a second channel would be a second thing to remember.
    CallerDefect(CallerDefect),
}

/// A violation and where in the payload it was found.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Violation {
    /// The absolute payload offset the walk had reached.
    pub at: usize,
    pub kind: ViolationKind,
}

impl Violation {
    const fn new(at: usize, kind: ViolationKind) -> Self {
        Violation { at, kind }
    }
}

// =============================================================================
// Byte readers
// =============================================================================
//
// Every one of these is reached only after a length check, so the slices are in
// bounds -- and they PANIC on a short slice, which is why they are crate-private
// and every public path into this crate bounds-checks before indexing.

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

// =============================================================================
// The core walk
// =============================================================================

/// The core header, read as bytes and interpreted as nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreHeader {
    pub elapsed_wall_ms: u64,
    words: [u32; CoreWord::ALL.len()],
}

impl CoreHeader {
    /// The raw word this header field occupies.
    pub fn word(&self, word: CoreWord) -> u32 {
        self.words[(word.offset() - 8) / 4]
    }

    /// Every core header word in wire order.
    ///
    /// Callers destructure this — `let [row_count, changed_rows, ...] =
    /// header.words();` — so an eleventh word is a compile error at every site,
    /// which is the point of the whole crate.
    pub fn words(&self) -> [u32; CoreWord::ALL.len()] {
        self.words
    }

    /// The row count, which decides where the trailers start.
    pub fn row_count(&self) -> u32 {
        self.word(CoreWord::RowCount)
    }
}

/// Read the 48-byte core header, and check NOTHING else.
///
/// Not the scale word, not the counted-header cross-checks, not the row bound:
/// all three are rules the four sites apply at four different points relative to
/// their own diagnostics, and two of them are non-fatal at the authority.
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

/// One core purchase-contract row.
///
/// Destructured with a complete pattern at every site; see the module docs.
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

/// Where the trailers begin: the core header plus `row_count` core rows.
///
/// The bound check is here rather than in the cursor because all four sites
/// perform it as one statement about the whole core, before they read a single
/// row, and two of them return from it.
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

/// One step of the core row walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoreRowStep {
    Row {
        index: usize,
        row: CoreRow,
    },
    /// Every core row has been delimited; the offset the walk finished at, which
    /// is where the trailers begin.
    End(usize),
}

/// The pull cursor over the core purchase-contract rows.
#[derive(Clone, Copy, Debug)]
pub struct CoreRowCursor {
    at: usize,
    index: usize,
    row_count: usize,
}

impl CoreRowCursor {
    /// Start at the first core row. The caller has already called
    /// [`core_end`], so every row this cursor yields is in bounds.
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

    /// Delimit the next core row.
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

// =============================================================================
// The trailer alphabet
// =============================================================================

/// Every trailer magic this grammar knows.
///
/// **This enum is the drift boundary for the payload's tail.** The magics, the
/// labels, the per-family header readers and every site's dispatch are all
/// generated from [`Trailer::ALL`] by exhaustive `match`es with no wildcard arm,
/// so an eighth family is a compile error in the collector's packer, in
/// `fold_market_trade`, in `decode_compact_market_trade_data_payload` and in the
/// lane's `decode_optional_trailer` until each has decided what it means.
///
/// NINE magics, not seven: [`Trailer::Residual`] (`MTRB`) and
/// [`Trailer::LendLease`] (`MTLL`) are trailers only the lane gate accepts. The
/// alphabet of magics belongs to the grammar; WHICH of them a site accepts is
/// that site's own decision, taken on the [`Trailer`] this cursor hands back —
/// which is why [`TrailerCursor::next`] names the magic and reads nothing else.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Trailer {
    /// `MTD1` — purchase-contract detail rows.
    Detail,
    /// `MTRF` — per-country strategic-resource flow pools.
    ResourceFlow,
    /// `MTRH` — per-country civilian factory header.
    FactoryHeader,
    /// `MTRR` — the resource set, its identity block, and per-country rows.
    ResourceRow,
    /// `MTRD` — classic resource-trade deals, with witnesses.
    ClassicDeal,
    /// `MTRA` — the trade-view activation record. Header only.
    Activation,
    /// `MTS1` — per-country equipment stockpile segments.
    Stockpile,
    /// `MTRB` — residual badge rows. Lane gate only.
    Residual,
    /// `MTLL` — lend-lease mapping rows. Lane gate only.
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

    /// The four bytes that name this family on the wire.
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

    /// The four-character label every site already spells in its diagnostics.
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

    /// This family's fixed header width, in bytes.
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

/// Every magic is distinct, and `label()` spells `magic()`. Two families sharing
/// a magic would make the dispatch order decide which one a payload is, which is
/// exactly the kind of fact that must not be discoverable only at runtime.
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
    // `seen_bit` packs the alphabet into a `u16`.
    if Trailer::ALL.len() > 16 {
        panic!("d43_grammar: the trailer alphabet no longer fits TrailerCursor's seen mask");
    }
};

// =============================================================================
// The trailer walk
// =============================================================================

/// One step of the trailer walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrailerStep {
    /// A magic was delimited and NAMED. Nothing behind it has been read: the
    /// caller decides whether it accepts this family here, and only then calls
    /// [`TrailerCursor::read_body`].
    Magic { at: usize, trailer: Trailer },
    /// The payload ended on a trailer boundary; the offset the walk finished at.
    End(usize),
}

/// The pull cursor over the payload's trailers.
///
/// Two steps per trailer, deliberately: see the module docs. `End` carries the
/// finishing offset rather than a separate `finish()`, which would need a misuse
/// state.
///
/// **Neither `Clone` nor `Copy`, and that is the whole enforcement.**
/// [`TrailerCursor::read_body`] takes it by value and the only route back is
/// [`TrailerResume::resume`], reachable only from a family's final
/// [`BlockEnd`] — so a walk that skipped a declared block has no cursor to
/// carry on with. See [`crate::walk`].
#[derive(Debug)]
#[must_use]
pub struct TrailerCursor {
    at: usize,
    pending: Option<(usize, Trailer)>,
    seen: u16,
}

impl TrailerCursor {
    /// Start at `core_end`.
    pub const fn new(at: usize) -> Self {
        TrailerCursor {
            at,
            pending: None,
            seen: 0,
        }
    }

    /// The payload offset the cursor has reached.
    pub const fn offset(&self) -> usize {
        self.at
    }

    /// Whether this payload has already carried `trailer`.
    pub const fn seen(&self, trailer: Trailer) -> bool {
        self.seen & trailer.seen_bit() != 0
    }

    /// Delimit and NAME the next magic. Reads nothing behind it.
    ///
    /// A magic this grammar does not know, and a magic this payload has already
    /// carried, are both refused here — before any header is parsed, which is
    /// the order all four sites already refuse them in.
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

    /// Read the shared header prefix and this family's own header words, apply
    /// NO family rule, and do not advance.
    ///
    /// This exists for one measured reason and it is diagnostic ORDER. The
    /// shared eager driver preserves the historical MTRF value-before-shape
    /// order. [`TrailerCursor::read_body`] refuses a bad declared `header_len`
    /// before it returns anything, so the driver previews that header first.
    ///
    /// It reads nothing outside the family's own declared header, and it is
    /// callable only after [`TrailerCursor::next`] named a magic — so a site
    /// that refuses an unaccepted magic still refuses it before any header word
    /// is read.
    pub(crate) fn peek_header(&self, payload: &[u8]) -> Result<HeaderPreview, Violation> {
        let Some((at, trailer)) = self.pending else {
            return Err(Violation::caller(self.at, CallerDefect::BodyNotDelimited));
        };
        read_header_preview(payload, at, trailer)
    }

    /// Read the named family's header, compute its extent, and hand back the
    /// walk that has to follow it.
    ///
    /// **Consumes the cursor.** The parent does not exist again until this
    /// family's final [`BlockEnd`] yields a [`TrailerResume`], which is what
    /// makes forgetting a declared block unrepresentable rather than invisible.
    ///
    /// Call only after [`TrailerCursor::next`] returned [`TrailerStep::Magic`];
    /// calling it otherwise is [`CallerDefect::BodyNotDelimited`], reported
    /// rather than panicked, because a leaf under a recorder that must fail
    /// closed does not get to panic.
    ///
    /// # The parent is GONE until the family's last block ends
    ///
    /// This route compiles, and it is the only one:
    ///
    /// ```
    /// use d43_grammar::*;
    /// fn walk(flow: ResourceFlowCursor) -> Result<TrailerCursor, Violation> {
    ///     let mut rows = flow.rows();
    ///     while let RowStep::Row { .. } = rows.next() {}
    ///     Ok(rows.into_end()?.next.resume())
    /// }
    /// ```
    ///
    /// Using the cursor after `read_body` took it does not:
    ///
    /// ```compile_fail
    /// use d43_grammar::*;
    /// fn after_move(mut cursor: TrailerCursor, payload: &[u8], limits: Limits) {
    ///     let _body = cursor.read_body(payload, limits);
    ///     let _ = cursor.next(payload);
    /// }
    /// ```
    ///
    /// Neither does keeping a second copy of the parent to walk on with:
    ///
    /// ```compile_fail
    /// use d43_grammar::*;
    /// fn two_parents(cursor: TrailerCursor) -> (TrailerCursor, TrailerCursor) {
    ///     let spare: TrailerCursor = cursor.clone();
    ///     (cursor, spare)
    /// }
    /// ```
    ///
    /// And a family cursor has no route to the parent that does not go through
    /// every one of its declared blocks -- neither by skipping one:
    ///
    /// ```compile_fail
    /// use d43_grammar::*;
    /// fn skip_the_identities(mtrr: ResourceRowCursor) -> BlockCursor<TrailerResume> {
    ///     mtrr.rows()
    /// }
    /// ```
    ///
    /// nor by resuming straight out of the family:
    ///
    /// ```compile_fail
    /// use d43_grammar::*;
    /// fn escape_the_flow(flow: ResourceFlowCursor) -> TrailerCursor {
    ///     flow.resume()
    /// }
    /// ```
    ///
    /// nor by taking the family's `End` without its block:
    ///
    /// ```compile_fail
    /// use d43_grammar::*;
    /// fn escape_the_stockpile(mts1: StockpileCursor) -> BlockEnd<TrailerResume> {
    ///     mts1.end()
    /// }
    /// ```
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

    /// Move past a consumed trailer. Reachable only from
    /// [`TrailerResume::resume`].
    pub(crate) fn advance_past(&mut self, trailer: Trailer, end: usize) {
        self.at = end;
        self.pending = None;
        self.seen |= trailer.seen_bit();
    }
}

/// The walk must account for every byte of the payload.
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
