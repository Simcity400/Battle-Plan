//! Domain 24 (`weather_live`) payload grammar — the single structural walk.
//!
//! # Why this crate exists
//!
//! Owner ruling 2026-08-15: one shared decoder per domain, because the same
//! grammar walked at several sites goes stale at some of them. D24's payload was
//! walked at four: the producer
//! (`attach_log.rs::compact_weather_data_payload`),
//! `world_fold::fold_weather`,
//! `domain_data_authority::decode_compact_weather_data_payload` and
//! `wire_format::d24`. The two block widths, the eight region flags, the ten
//! province flags and the header's field offsets were restated at each — and the
//! header words at `+40`/`+44` recently STOPPED being reserved zeros and became
//! the producer's two discovered precipitation ceilings, which is exactly the
//! kind of change that leaves a reader behind.
//!
//! # What is grammar here, and what is not
//!
//! Grammar (this crate): the header's width and every header word's offset, the
//! two row widths, where the region table ends and the province table begins,
//! each row field's offset within its row, which flag bit is which, and the
//! payload's exact length.
//!
//! Semantics (each site): every range check, the id/duplicate rules, the
//! flag-consistency rules (mud, map recolor, dynamic LUT, the activity
//! witnesses), the mutually-exclusive named-weather rule, the delta/baseline
//! rules, the latched observed-row counts, the precipitation-ceiling presence
//! pair and what an absent pair means for the hour, the scale word, and the
//! merge. None of that moved, and none of it should: the two readers disagree
//! about the FATALITY of most of it (the fold returns on the first violation,
//! the validator records each one and keeps walking), and a leaf that took a
//! position on that would delete a domain instead of correcting it.
//!
//! # Traversal is LAZY, and that is a requirement
//!
//! Both readers interleave semantics with the walk — the fold validates a
//! region row's flags and ranges before it reads the next row, and counts active
//! regions as it goes. A whole-payload layout would reorder which diagnostic a
//! malformed capture carries. So the row walk is a PULL cursor.
//!
//! The header split is separate from the body for the same reason it is in
//! `wire_format::d24`: the fold returns on a failure status and on
//! delta-before-baseline WITHOUT validating the body at all, so [`read_header`]
//! must not look past the header, and [`body_layout`] is a second call the
//! caller makes after its own status gates.
//!
//! # Violations are data, never text
//!
//! The sites spell the same refusal differently and always have: the fold says
//! `"weather len 100 != 112"`, the validator says `"offset 900: weather data
//! payload is 100 byte(s), expected 112"`. [`Violation`] carries the operands
//! and no message; each site owns one exhaustive formatter.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod encode;

#[cfg(test)]
mod tests;

// --- Fixed widths ------------------------------------------------------------

/// The payload header.
pub const HEADER_LEN: usize = 48;

/// One strategic-region row.
pub const REGION_ROW_LEN: usize = 32;

/// One province row.
pub const PROVINCE_ROW_LEN: usize = 32;

// --- The header table --------------------------------------------------------

/// One word of the payload header.
///
/// **This enum is half the drift boundary** (the two flag tables are the other
/// half). Every header offset is generated from [`HeaderWord::offset`] by an
/// exhaustive `match` with no wildcard arm, and every site reads its header
/// through [`Header`], so a new header word is a compile error at each site
/// until it has decided what the word means.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum HeaderWord {
    RegionCount,
    ChangedRows,
    StatusFlags,
    ProvinceCount,
    ObservedRegionRows,
    ObservedProvinceRows,
    ScaleRaw,
    ActiveRegionCount,
    /// Engine precipitation ceiling, `CWeatherManager + 0x1a8`. Was a reserved
    /// zero until the producer started discovering it.
    MaxWaterRaw,
    /// Engine precipitation ceiling, `CWeatherManager + 0x1f8`. Same.
    MaxSnowRaw,
}

impl HeaderWord {
    pub const ALL: [HeaderWord; 10] = [
        HeaderWord::RegionCount,
        HeaderWord::ChangedRows,
        HeaderWord::StatusFlags,
        HeaderWord::ProvinceCount,
        HeaderWord::ObservedRegionRows,
        HeaderWord::ObservedProvinceRows,
        HeaderWord::ScaleRaw,
        HeaderWord::ActiveRegionCount,
        HeaderWord::MaxWaterRaw,
        HeaderWord::MaxSnowRaw,
    ];

    /// This word's byte offset in the payload.
    ///
    /// `+0..+8` is `elapsed_wall_ms`, which no reader reads and which
    /// [`Header`] therefore does not carry.
    pub const fn offset(self) -> usize {
        match self {
            HeaderWord::RegionCount => 8,
            HeaderWord::ChangedRows => 12,
            HeaderWord::StatusFlags => 16,
            HeaderWord::ProvinceCount => 20,
            HeaderWord::ObservedRegionRows => 24,
            HeaderWord::ObservedProvinceRows => 28,
            HeaderWord::ScaleRaw => 32,
            HeaderWord::ActiveRegionCount => 36,
            HeaderWord::MaxWaterRaw => 40,
            HeaderWord::MaxSnowRaw => 44,
        }
    }
}

/// Every header word occupies four distinct bytes inside the header, checked at
/// BUILD time so a new word cannot silently overlap an existing one.
const _: () = {
    let mut i = 0;
    while i < HeaderWord::ALL.len() {
        let a = HeaderWord::ALL[i].offset();
        if a + 4 > HEADER_LEN {
            panic!("d24_grammar: a header word runs past the header");
        }
        if a < 8 {
            panic!("d24_grammar: a header word overlaps elapsed_wall_ms");
        }
        let mut j = i + 1;
        while j < HeaderWord::ALL.len() {
            if HeaderWord::ALL[j].offset() == a {
                panic!("d24_grammar: two header words share an offset");
            }
            j += 1;
        }
        i += 1;
    }
};

// --- The flag tables ---------------------------------------------------------

/// One bit of a region row's flag word.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RegionFlag {
    RainLight,
    RainHeavy,
    Snow,
    Blizzard,
    ArcticWater,
    ActiveSignal,
    PhenomenonActive,
    Sandstorm,
}

impl RegionFlag {
    pub const ALL: [RegionFlag; 8] = [
        RegionFlag::RainLight,
        RegionFlag::RainHeavy,
        RegionFlag::Snow,
        RegionFlag::Blizzard,
        RegionFlag::ArcticWater,
        RegionFlag::ActiveSignal,
        RegionFlag::PhenomenonActive,
        RegionFlag::Sandstorm,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            RegionFlag::RainLight => 1,
            RegionFlag::RainHeavy => 1 << 1,
            RegionFlag::Snow => 1 << 2,
            RegionFlag::Blizzard => 1 << 3,
            RegionFlag::ArcticWater => 1 << 4,
            RegionFlag::ActiveSignal => 1 << 5,
            RegionFlag::PhenomenonActive => 1 << 6,
            RegionFlag::Sandstorm => 1 << 7,
        }
    }

    pub const fn is_set(self, flags: u32) -> bool {
        flags & self.bit() != 0
    }
}

/// One bit of a province row's flag word.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ProvinceFlag {
    TempPresent,
    TempChangePresent,
    WaterPresent,
    SnowPresent,
    MudPresent,
    MudActive,
    CustomModifierCountPresent,
    StructValidated,
    MapRecolor,
    DynamicLut,
}

impl ProvinceFlag {
    pub const ALL: [ProvinceFlag; 10] = [
        ProvinceFlag::TempPresent,
        ProvinceFlag::TempChangePresent,
        ProvinceFlag::WaterPresent,
        ProvinceFlag::SnowPresent,
        ProvinceFlag::MudPresent,
        ProvinceFlag::MudActive,
        ProvinceFlag::CustomModifierCountPresent,
        ProvinceFlag::StructValidated,
        ProvinceFlag::MapRecolor,
        ProvinceFlag::DynamicLut,
    ];

    pub const fn bit(self) -> u32 {
        match self {
            ProvinceFlag::TempPresent => 1,
            ProvinceFlag::TempChangePresent => 1 << 1,
            ProvinceFlag::WaterPresent => 1 << 2,
            ProvinceFlag::SnowPresent => 1 << 3,
            ProvinceFlag::MudPresent => 1 << 4,
            ProvinceFlag::MudActive => 1 << 5,
            ProvinceFlag::CustomModifierCountPresent => 1 << 6,
            ProvinceFlag::StructValidated => 1 << 7,
            ProvinceFlag::MapRecolor => 1 << 8,
            ProvinceFlag::DynamicLut => 1 << 9,
        }
    }

    pub const fn is_set(self, flags: u32) -> bool {
        flags & self.bit() != 0
    }
}

/// Every assigned region-row flag bit.
pub const REGION_KNOWN_FLAGS: u32 = region_known_flags();

const fn region_known_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < RegionFlag::ALL.len() {
        mask |= RegionFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

/// Every assigned province-row flag bit.
pub const PROVINCE_KNOWN_FLAGS: u32 = province_known_flags();

const fn province_known_flags() -> u32 {
    let mut mask = 0;
    let mut i = 0;
    while i < ProvinceFlag::ALL.len() {
        mask |= ProvinceFlag::ALL[i].bit();
        i += 1;
    }
    mask
}

// --- The row field tables ----------------------------------------------------

/// One field of a strategic-region row.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RegionField {
    RegionId,
    Flags,
    TemperatureRaw,
    Phenomenon,
    NextWeatherChangeRaw,
    ActiveModifierCount,
    Intensity,
    Reserved,
}

impl RegionField {
    pub const ALL: [RegionField; 8] = [
        RegionField::RegionId,
        RegionField::Flags,
        RegionField::TemperatureRaw,
        RegionField::Phenomenon,
        RegionField::NextWeatherChangeRaw,
        RegionField::ActiveModifierCount,
        RegionField::Intensity,
        RegionField::Reserved,
    ];

    /// This field's offset within its row.
    pub const fn offset(self) -> usize {
        match self {
            RegionField::RegionId => 0,
            RegionField::Flags => 4,
            RegionField::TemperatureRaw => 8,
            RegionField::Phenomenon => 12,
            RegionField::NextWeatherChangeRaw => 16,
            RegionField::ActiveModifierCount => 20,
            RegionField::Intensity => 24,
            RegionField::Reserved => 28,
        }
    }
}

/// One field of a province row.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ProvinceField {
    ProvinceId,
    Flags,
    TemperatureRaw,
    TemperatureChangeRaw,
    WaterRaw,
    SnowRaw,
    CustomModifierCount,
    Reserved,
}

impl ProvinceField {
    pub const ALL: [ProvinceField; 8] = [
        ProvinceField::ProvinceId,
        ProvinceField::Flags,
        ProvinceField::TemperatureRaw,
        ProvinceField::TemperatureChangeRaw,
        ProvinceField::WaterRaw,
        ProvinceField::SnowRaw,
        ProvinceField::CustomModifierCount,
        ProvinceField::Reserved,
    ];

    pub const fn offset(self) -> usize {
        match self {
            ProvinceField::ProvinceId => 0,
            ProvinceField::Flags => 4,
            ProvinceField::TemperatureRaw => 8,
            ProvinceField::TemperatureChangeRaw => 12,
            ProvinceField::WaterRaw => 16,
            ProvinceField::SnowRaw => 20,
            ProvinceField::CustomModifierCount => 24,
            ProvinceField::Reserved => 28,
        }
    }
}

/// Both row tables tile their row exactly, checked at BUILD time: eight
/// four-byte fields at distinct offsets covering the whole 32-byte row. A new
/// field that overlaps an existing one, or a row width changed at one table and
/// not the other, stops the build.
const _: () = {
    let mut i = 0;
    while i < RegionField::ALL.len() {
        if RegionField::ALL[i].offset() != i * 4 {
            panic!("d24_grammar: RegionField::ALL is not in offset order");
        }
        i += 1;
    }
    if RegionField::ALL.len() * 4 != REGION_ROW_LEN {
        panic!("d24_grammar: the region fields do not tile the region row");
    }
    let mut i = 0;
    while i < ProvinceField::ALL.len() {
        if ProvinceField::ALL[i].offset() != i * 4 {
            panic!("d24_grammar: ProvinceField::ALL is not in offset order");
        }
        i += 1;
    }
    if ProvinceField::ALL.len() * 4 != PROVINCE_ROW_LEN {
        panic!("d24_grammar: the province fields do not tile the province row");
    }
};

// --- Violations --------------------------------------------------------------

/// Which part of the walk ran out of bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Header,
    /// The whole body: the two row tables together.
    Body,
    RegionRow,
    ProvinceRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    /// The payload is shorter than the header.
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    /// The payload's length is not exactly what the header's counts imply.
    ///
    /// Both readers require EQUALITY here, not "at least": a weather payload
    /// with residual bytes is a torn payload.
    LengthMismatch { expected: usize, available: usize },
    /// A count multiplied by a row width does not fit a `usize`.
    LengthOverflow {
        section: Section,
        count: usize,
        unit: usize,
    },
}

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

// --- Byte readers ------------------------------------------------------------
//
// Reached only after a length check, so the slices are in bounds -- and they
// PANIC on a short slice, which is why they are crate-private and every public
// path bounds-checks before indexing.

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
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

// --- The walk ----------------------------------------------------------------

/// The payload header, read as bytes and interpreted as nothing.
///
/// It keeps the header bytes rather than a decoded struct so that a word is
/// always read at [`HeaderWord::offset`] — there is no second table mapping a
/// word to an array slot that could drift out of step with the first, and no
/// lookup that can fall through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    bytes: [u8; HEADER_LEN],
}

impl Header {
    /// `+0..+8`. No reader reads it; the producer writes it.
    pub fn elapsed_wall_ms(&self) -> u64 {
        u64_at(&self.bytes, 0)
    }

    /// The raw word this header field occupies.
    pub fn word(&self, which: HeaderWord) -> u32 {
        u32_at(&self.bytes, which.offset())
    }

    /// The same word as a `usize`, for the count words.
    pub fn count(&self, which: HeaderWord) -> usize {
        self.word(which) as usize
    }

    /// Every header word, in [`HeaderWord::ALL`] order.
    ///
    /// **This is what makes a new header word a compile error at the READERS.**
    /// `word(HeaderWord::X)` alone does not: a site that never asks for a new
    /// word keeps compiling and keeps ignoring it, which is precisely how the
    /// two precipitation ceilings at `+40`/`+44` could have been added to the
    /// producer and missed by a reader. The three readers destructure this
    /// array, so its LENGTH changing stops their build.
    pub fn words(&self) -> [u32; HeaderWord::ALL.len()] {
        let mut out = [0u32; HeaderWord::ALL.len()];
        let mut i = 0;
        while i < HeaderWord::ALL.len() {
            out[i] = u32_at(&self.bytes, HeaderWord::ALL[i].offset());
            i += 1;
        }
        out
    }

    /// The header's bytes.
    pub fn bytes(&self) -> &[u8; HEADER_LEN] {
        &self.bytes
    }
}

/// Read the 48-byte header, and check NOTHING else.
///
/// The fold's failure-status and delta-before-baseline exits both happen after
/// exactly this check and before any body validation, so this must not look
/// past the header.
pub fn read_header(payload: &[u8]) -> Result<Header, Violation> {
    if payload.len() < HEADER_LEN {
        return Err(Violation::new(
            0,
            ViolationKind::Need {
                section: Section::Header,
                needed: HEADER_LEN,
                available: payload.len(),
            },
        ));
    }
    let mut bytes = [0u8; HEADER_LEN];
    bytes.copy_from_slice(&payload[..HEADER_LEN]);
    Ok(Header { bytes })
}

/// Where the two row tables are.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyLayout {
    pub region_count: usize,
    pub province_count: usize,
    /// The payload offset the region table starts at.
    pub regions_at: usize,
    /// The payload offset the province table starts at.
    pub provinces_at: usize,
    /// The payload offset the province table ends at, which is the payload's
    /// length: this grammar has no trailing section.
    pub end: usize,
}

/// Delimit the two row tables and require the payload to be exactly that long.
pub fn body_layout(payload: &[u8], header: &Header) -> Result<BodyLayout, Violation> {
    let region_count = header.count(HeaderWord::RegionCount);
    let province_count = header.count(HeaderWord::ProvinceCount);
    let region_bytes = region_count.checked_mul(REGION_ROW_LEN).ok_or(
        Violation::new(
            HEADER_LEN,
            ViolationKind::LengthOverflow {
                section: Section::RegionRow,
                count: region_count,
                unit: REGION_ROW_LEN,
            },
        ),
    )?;
    let province_bytes = province_count.checked_mul(PROVINCE_ROW_LEN).ok_or(
        Violation::new(
            HEADER_LEN,
            ViolationKind::LengthOverflow {
                section: Section::ProvinceRow,
                count: province_count,
                unit: PROVINCE_ROW_LEN,
            },
        ),
    )?;
    let provinces_at = HEADER_LEN.checked_add(region_bytes).ok_or(Violation::new(
        HEADER_LEN,
        ViolationKind::LengthOverflow {
            section: Section::Body,
            count: region_count,
            unit: REGION_ROW_LEN,
        },
    ))?;
    let end = provinces_at.checked_add(province_bytes).ok_or(Violation::new(
        provinces_at,
        ViolationKind::LengthOverflow {
            section: Section::Body,
            count: province_count,
            unit: PROVINCE_ROW_LEN,
        },
    ))?;
    if payload.len() != end {
        return Err(Violation::new(
            HEADER_LEN,
            ViolationKind::LengthMismatch {
                expected: end,
                available: payload.len(),
            },
        ));
    }
    Ok(BodyLayout {
        region_count,
        province_count,
        regions_at: HEADER_LEN,
        provinces_at,
        end,
    })
}

/// A delimited region row. Fixed offset arithmetic that validates nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionRowView<'a> {
    bytes: &'a [u8],
}

impl<'a> RegionRowView<'a> {
    /// A view over ONE already-delimited row.
    ///
    /// For a caller that holds a row slice rather than a cursor -- the wire
    /// port maps one row at a time out of a table it was handed. It is
    /// length-checked, so the accessors below still cannot index out of
    /// bounds, and it takes the width from the same table the cursors do.
    pub fn from_row(row: &'a [u8]) -> Result<Self, Violation> {
        if row.len() != REGION_ROW_LEN {
            return Err(Violation::new(
                0,
                ViolationKind::Need {
                    section: Section::RegionRow,
                    needed: REGION_ROW_LEN,
                    available: row.len(),
                },
            ));
        }
        Ok(RegionRowView { bytes: row })
    }

    /// The raw `u32` this field occupies.
    pub fn u32(&self, field: RegionField) -> u32 {
        u32_at(self.bytes, field.offset())
    }

    /// The same word read as a signed `i32`.
    pub fn i32(&self, field: RegionField) -> i32 {
        self.u32(field) as i32
    }

    /// Every field of the row, in `RegionField::ALL` order.
    ///
    /// **This is what makes a new row field a compile error at the
    /// READERS.** `u32(RegionField::X)` alone does not: a site that never asks
    /// for a new field keeps compiling and keeps ignoring it. The readers
    /// destructure this array, so its LENGTH changing stops their build.
    pub fn words(&self) -> [u32; RegionField::ALL.len()] {
        let mut out = [0u32; RegionField::ALL.len()];
        let mut i = 0;
        while i < RegionField::ALL.len() {
            out[i] = u32_at(self.bytes, RegionField::ALL[i].offset());
            i += 1;
        }
        out
    }

    /// The row's bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// A delimited province row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvinceRowView<'a> {
    bytes: &'a [u8],
}

impl<'a> ProvinceRowView<'a> {
    /// A view over ONE already-delimited row.
    ///
    /// For a caller that holds a row slice rather than a cursor -- the wire
    /// port maps one row at a time out of a table it was handed. It is
    /// length-checked, so the accessors below still cannot index out of
    /// bounds, and it takes the width from the same table the cursors do.
    pub fn from_row(row: &'a [u8]) -> Result<Self, Violation> {
        if row.len() != PROVINCE_ROW_LEN {
            return Err(Violation::new(
                0,
                ViolationKind::Need {
                    section: Section::ProvinceRow,
                    needed: PROVINCE_ROW_LEN,
                    available: row.len(),
                },
            ));
        }
        Ok(ProvinceRowView { bytes: row })
    }

    pub fn u32(&self, field: ProvinceField) -> u32 {
        u32_at(self.bytes, field.offset())
    }

    pub fn i32(&self, field: ProvinceField) -> i32 {
        self.u32(field) as i32
    }

    /// Every field of the row, in `ProvinceField::ALL` order.
    ///
    /// **This is what makes a new row field a compile error at the
    /// READERS.** `u32(ProvinceField::X)` alone does not: a site that never asks
    /// for a new field keeps compiling and keeps ignoring it. The readers
    /// destructure this array, so its LENGTH changing stops their build.
    pub fn words(&self) -> [u32; ProvinceField::ALL.len()] {
        let mut out = [0u32; ProvinceField::ALL.len()];
        let mut i = 0;
        while i < ProvinceField::ALL.len() {
            out[i] = u32_at(self.bytes, ProvinceField::ALL[i].offset());
            i += 1;
        }
        out
    }

    /// The row's bytes.
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// One step of a row walk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowStep<V> {
    Row { index: usize, view: V },
    /// Every row has been delimited; the payload offset the table ends at.
    End(usize),
}

/// The pull cursor over the region table.
#[derive(Clone, Copy, Debug)]
pub struct RegionCursor {
    at: usize,
    next_index: usize,
    count: usize,
}

/// The pull cursor over the province table.
#[derive(Clone, Copy, Debug)]
pub struct ProvinceCursor {
    at: usize,
    next_index: usize,
    count: usize,
}

macro_rules! row_cursor {
    ($cursor:ty, $view:ident, $row_len:expr, $section:expr) => {
        impl $cursor {
            /// The payload offset the cursor has reached.
            pub const fn offset(&self) -> usize {
                self.at
            }

            /// Delimit the next row.
            pub fn next<'a>(
                &mut self,
                payload: &'a [u8],
            ) -> Result<RowStep<$view<'a>>, Violation> {
                if self.next_index >= self.count {
                    return Ok(RowStep::End(self.at));
                }
                let end = self.at.checked_add($row_len).ok_or(Violation::new(
                    self.at,
                    ViolationKind::LengthOverflow {
                        section: $section,
                        count: 1,
                        unit: $row_len,
                    },
                ))?;
                if end > payload.len() {
                    return Err(Violation::new(
                        self.at,
                        ViolationKind::Need {
                            section: $section,
                            needed: end,
                            available: payload.len(),
                        },
                    ));
                }
                let view = $view {
                    bytes: &payload[self.at..end],
                };
                let index = self.next_index;
                self.at = end;
                self.next_index += 1;
                Ok(RowStep::Row { index, view })
            }
        }
    };
}

row_cursor!(RegionCursor, RegionRowView, REGION_ROW_LEN, Section::RegionRow);
row_cursor!(
    ProvinceCursor,
    ProvinceRowView,
    PROVINCE_ROW_LEN,
    Section::ProvinceRow
);

impl RegionCursor {
    /// Start at the first region row.
    pub const fn new(layout: &BodyLayout) -> Self {
        RegionCursor {
            at: layout.regions_at,
            next_index: 0,
            count: layout.region_count,
        }
    }
}

impl ProvinceCursor {
    /// Start at the first province row.
    pub const fn new(layout: &BodyLayout) -> Self {
        ProvinceCursor {
            at: layout.provinces_at,
            next_index: 0,
            count: layout.province_count,
        }
    }
}

/// The two tables must account for every byte behind the header.
///
/// [`body_layout`] already enforced this against the header's counts; this is
/// the second half of the same claim, made against the cursor that actually
/// walked, so a cursor that stopped early cannot be mistaken for a complete
/// walk.
pub fn exact_consumption(consumed: usize, payload_len: usize) -> Result<(), Violation> {
    if consumed != payload_len {
        return Err(Violation::new(
            consumed,
            ViolationKind::LengthMismatch {
                expected: consumed,
                available: payload_len,
            },
        ));
    }
    Ok(())
}
