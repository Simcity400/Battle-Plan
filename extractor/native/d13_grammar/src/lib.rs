//! Domain 13 (`tech_live`) payload grammar — the single structural walk.
//!
//! # Why this crate exists
//!
//! D13's DATA payload grammar was walked in FOUR places: the collector's packer
//! wrote it, `recorder_host::world_fold::fold_tech` folded it,
//! `recorder_host::domain_data_authority::decode_compact_tech_data_payload`
//! validated it, and `wire_format::d13` ported it. A trailing-section change
//! proved that duplicated structural walks could reject otherwise valid
//! payloads. This crate provides one shared decoder while each site keeps its
//! own semantic layer — what it does with the
//! spans, which violations are fatal, and how it spells its diagnostics. The
//! shared boundary is grammar only, not merged responsibility.
//!
//! # What is grammar here, and what is not
//!
//! Grammar (this crate): where every section starts and ends, which optional
//! tails a flag word claims, the flag-combination rules that decide whether a
//! row *has a length at all*, the magic word, the fixed field offsets inside the
//! two authority tails, and exact consumption.
//!
//! Semantics (each site): what a span MEANS. UTF-8 validity, the A29 twenty-
//! clause authority predicate, the ahead-of-time value ranges, duplicate keys,
//! zero tags, scale-word expectations, the XUI09 measured/refused/vector
//! relationships, and every allocation bound. Nothing here reads a value it does
//! not need in order to find the next byte boundary — except through the
//! read-only views ([`A29ProvenanceView`], [`DerivedAuthorityView`],
//! [`AheadQuartetView`]), which are fixed offset arithmetic and validate
//! nothing.
//!
//! # Traversal is LAZY, and that is a requirement
//!
//! The three readers interleave their semantics with the walk: the fold checks a
//! row's presence words before the name tail and its quartet values before the
//! provenance block, and the validator records a diagnostic and keeps walking
//! where the fold returns. A function that computed a whole-payload layout up
//! front would discover a truncation in row 2 before the fold reported a
//! semantic refusal in row 1, which changes which diagnostic a malformed capture
//! carries. So the walk is a PULL cursor: the caller asks for the next step,
//! does its own work, and asks again.
//!
//! # Violations are data, never text
//!
//! The three readers spell the same violation three different ways and always
//! have (`"tech active row has unknown flags 64"` in the fold, `"offset 900:
//! tech active row 3 has unknown flags 64"` in the validator, `"row has unknown
//! flags 0x40"` in the port). [`Violation`] carries the operands and no message;
//! each site owns one exhaustive formatter. That is the correct compatibility
//! boundary: an `ErrorStyle::Fold | Wire | Authority` knob inside this crate
//! would make a supposedly neutral leaf know its consumers by name.
//!
//! Where two readers bounds-check at different granularities, the finer walk is
//! the one performed (it is the fold's, and it preserves the fold's ordering)
//! and the coarser reader's operands ride along in
//! [`Need::aggregate`], so its message does not change either. See
//! [`AggregateNeed`].
//!
//! # Fatality is the caller's, not this crate's
//!
//! Two classes come back differently:
//!
//! * `Err(Violation)` — TERMINAL. The walk cannot continue: the bytes needed to
//!   find the next boundary are not there, or the length arithmetic overflows.
//! * [`ActiveStep::Recoverable`] / [`Xui09Step::Recoverable`] — the structure is
//!   still walkable. The fold and the port refuse on these; the authority
//!   validator records them and keeps walking, and it must, because it gates the
//!   fold — a rule that is merely STRICTER there deletes the domain rather than
//!   correcting it.
//!
//! # The payload
//!
//! ```text
//! header          32 bytes
//! country rows    country_count * 16 bytes
//! active rows     active_count * (40 + optional tails)
//! XUI09 tail      0 bytes (absent) or 24 + sections
//! ```
//!
//! and the payload must be consumed EXACTLY.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use core::ops::Range;

pub mod encode;

#[cfg(test)]
mod tests;

// --- Lengths -----------------------------------------------------------------

/// The fixed payload header.
pub const HEADER_LEN: usize = 32;
/// One country row. Fixed: d13 has exactly one country row form.
pub const COUNTRY_ROW_LEN: usize = 16;
/// The fixed part of one active-research row, before any tail.
pub const ACTIVE_ROW_FIXED_LEN: usize = 40;
/// The ahead-of-time quartet tail: four `i64`.
pub const AHEAD_QUARTET_LEN: usize = 32;
/// The A29 observed-producer provenance tail.
pub const A29_PROVENANCE_LEN: usize = 176;
/// The A29 derived-authority tail: `authority: u32`, `producer_game_hour_raw:
/// i32`, `penalty_factor_raw: i64`.
pub const A29_DERIVED_AUTHORITY_LEN: usize = 16;

/// `XU09`, the first word of the witness tail.
pub const XUI09_TAIL_MAGIC: u32 = u32::from_le_bytes(*b"XU09");
/// `magic`, `flags`, `dictionary_count`, `measured_count`, `refused_count`,
/// `vector_count`, one `u32` each.
pub const XUI09_TAIL_HEADER_LEN: usize = 24;
/// One dictionary entry's fixed part: `def_index`, then a `u16`-prefixed id.
pub const XUI09_DICTIONARY_INDEX_LEN: usize = 4;
/// The smallest a dictionary entry can be: the index plus an empty id's prefix.
pub const XUI09_DICTIONARY_ENTRY_MIN_LEN: usize = XUI09_DICTIONARY_INDEX_LEN + 2;
/// One measured-country entry: a `tag_db_id`.
pub const XUI09_MEASURED_ENTRY_LEN: usize = 4;
/// One refusal entry: `tag_db_id`, `reason`.
pub const XUI09_REFUSAL_ENTRY_LEN: usize = 8;
/// One country vector's header: `tag_db_id`, `flags`, `state_count`,
/// `completed_count`.
pub const XUI09_VECTOR_HEADER_LEN: usize = 16;
/// A dense vector carries one enum byte per technology and no index.
pub const XUI09_DENSE_STATE_LEN: usize = 1;
/// A sparse vector carries `def_index: u32` beside each enum byte.
pub const XUI09_SPARSE_STATE_LEN: usize = 5;
/// One completed-membership entry: a `def_index`.
pub const XUI09_COMPLETED_ENTRY_LEN: usize = 4;

// --- Flag words --------------------------------------------------------------

/// The country row's `completed_tech_count` was read.
pub const COUNTRY_FLAG_COMPLETED_TECH_COUNT: u32 = 1;
/// The country row's `research_slot_count` was read.
pub const COUNTRY_FLAG_RESEARCH_SLOT_COUNT: u32 = 1 << 1;
/// The country's active-research LIST was read at all — which is not the same as
/// it being non-empty.
pub const COUNTRY_FLAG_ACTIVE_RESEARCH: u32 = 1 << 2;
/// Every assigned country-row flag bit.
///
/// This crate does not decide whether an unassigned bit is fatal, because the
/// three readers disagree today and every disagreement is deliberate: the fold
/// never reads the word at all, the validator records an unknown bit, and the
/// port deliberately accepts a future bit rather than refusing a payload the
/// recorder folds.
pub const COUNTRY_KNOWN_FLAGS: u32 = COUNTRY_FLAG_COMPLETED_TECH_COUNT
    | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
    | COUNTRY_FLAG_ACTIVE_RESEARCH;
/// The word packed in place of a country count the collector could not read.
/// Unreachable as a real reading: the collector's own bounds are 2000 and 32.
pub const COUNTRY_COUNT_ABSENT_SENTINEL: u32 = u32::MAX;

/// `live_progress_raw` at `+24` is a real reading.
pub const ACTIVE_FLAG_LIVE_PROGRESS: u32 = 1;
/// `base_required_points_raw` at `+32` is a real reading.
pub const ACTIVE_FLAG_BASE_REQUIRED_POINTS: u32 = 1 << 1;
/// The two active-row flag bits that gate a FIXED field rather than a tail.
pub const ACTIVE_VALUE_FLAGS: u32 = ACTIVE_FLAG_LIVE_PROGRESS | ACTIVE_FLAG_BASE_REQUIRED_POINTS;

// --- The tail inventory ------------------------------------------------------

/// The active-research row's optional tails, in wire order.
///
/// **This is the declaration the rest of the crate is generated from.** Every
/// `match` on it below is exhaustive and none carries a wildcard arm, so adding
/// a variant here is a compile error until the flag bit, the length, the
/// combination rules, the walk and the encoder have all been taught it — and,
/// because the three readers also match on [`ActiveSection::tail`] without a
/// wildcard, until each of them has decided what the new tail means. That is
/// what the owner's one-decoder ruling buys: the drift that cost a live capture
/// is a build failure now.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ActiveTail {
    /// `u16`-prefixed UTF-8 technology name.
    TechName,
    /// Four `i64` ahead-of-time values.
    AheadQuartet,
    /// The observed producer's 176-byte receipt.
    ObservedAuthority,
    /// The recorder's own derived-quartet inputs.
    DerivedAuthority,
}

impl ActiveTail {
    /// Every tail, in the order the wire carries them.
    pub const ALL: [ActiveTail; 4] = [
        ActiveTail::TechName,
        ActiveTail::AheadQuartet,
        ActiveTail::ObservedAuthority,
        ActiveTail::DerivedAuthority,
    ];

    /// The flag bit that says this tail is present.
    pub const fn flag_bit(self) -> u32 {
        match self {
            ActiveTail::TechName => 1 << 2,
            ActiveTail::AheadQuartet => 1 << 3,
            ActiveTail::ObservedAuthority => 1 << 4,
            ActiveTail::DerivedAuthority => 1 << 5,
        }
    }

    /// The tail's fixed byte length, or `None` for the one tail whose length is
    /// carried in its own bytes.
    pub const fn fixed_len(self) -> Option<usize> {
        match self {
            ActiveTail::TechName => None,
            ActiveTail::AheadQuartet => Some(AHEAD_QUARTET_LEN),
            ActiveTail::ObservedAuthority => Some(A29_PROVENANCE_LEN),
            ActiveTail::DerivedAuthority => Some(A29_DERIVED_AUTHORITY_LEN),
        }
    }

    /// The section a truncation of this tail is reported against.
    const fn section(self) -> Section {
        match self {
            ActiveTail::TechName => Section::TechNameBytes,
            ActiveTail::AheadQuartet => Section::AheadQuartet,
            ActiveTail::ObservedAuthority => Section::ObservedAuthority,
            ActiveTail::DerivedAuthority => Section::DerivedAuthority,
        }
    }
}

/// Every assigned active-row flag bit, folded out of [`ActiveTail::ALL`] rather
/// than written down a second time. `flags & !ACTIVE_KNOWN_FLAGS != 0` is what
/// the fold and the port refuse and what the validator records.
pub const ACTIVE_KNOWN_FLAGS: u32 = ACTIVE_VALUE_FLAGS | active_tail_flag_bits();

const fn active_tail_flag_bits() -> u32 {
    let mut bits = 0u32;
    let mut index = 0usize;
    while index < ActiveTail::ALL.len() {
        bits |= ActiveTail::ALL[index].flag_bit();
        index += 1;
    }
    bits
}

/// The XUI09 tail's country vectors are a PATCH: only the countries whose
/// witness bytes changed are carried.
pub const XUI09_FLAG_PATCH: u32 = 1;
/// Every assigned XUI09 tail flag bit.
pub const XUI09_KNOWN_FLAGS: u32 = XUI09_FLAG_PATCH;
/// This country's classifier vector covers definition indices `0..count`
/// exactly, so the indices are implied and only the enum bytes are carried.
pub const XUI09_COUNTRY_FLAG_DENSE: u32 = 1;
/// Every assigned XUI09 country-vector flag bit.
pub const XUI09_COUNTRY_KNOWN_FLAGS: u32 = XUI09_COUNTRY_FLAG_DENSE;

// --- Scales ------------------------------------------------------------------

/// `progress_raw` divides by this. Carried in the header at `+20`.
pub const TECH_PROGRESS_SCALE_RAW: u32 = 1000;
/// `live_progress_raw`, `base_required_points_raw` and all four quartet values
/// divide by this — **100x the other one, in the same row**. Carried in the
/// header at `+24`.
pub const TECH_LIVE_SCALE_RAW: u32 = 100_000;

// --- Violations --------------------------------------------------------------

/// The part of the payload a violation is about. Each reader maps this onto the
/// label it has always printed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    /// The 32-byte header itself.
    Header,
    /// `HEADER_LEN + country_count*16 + active_count*40`, the structural
    /// minimum.
    PayloadMinimum,
    /// The country table as a whole.
    CountryTable,
    /// One 16-byte country row.
    CountryRow,
    /// The 40 fixed bytes of one active row.
    ActiveFixed,
    /// The technology name's `u16` length prefix.
    TechNamePrefix,
    /// The technology name's UTF-8 bytes.
    TechNameBytes,
    /// The 32-byte ahead-of-time quartet.
    AheadQuartet,
    /// The 176-byte observed-producer receipt.
    ObservedAuthority,
    /// The 16-byte derived-authority tail.
    DerivedAuthority,
    /// The XUI09 tail's 24-byte header.
    Xui09TailHeader,
    /// One dictionary entry's `def_index` word.
    Xui09DictionaryIndex,
    /// One dictionary entry, index and id prefix together — the coarse form the
    /// length-only walkers have always reported.
    Xui09DictionaryEntry,
    /// One dictionary id's `u16` length prefix.
    Xui09DictionaryIdPrefix,
    /// One dictionary id's bytes.
    Xui09DictionaryIdBytes,
    /// One measured-country tag.
    Xui09MeasuredTag,
    /// One refusal entry.
    Xui09Refusal,
    /// The measured and refusal sets together — the coarse form the length-only
    /// walkers have always reported.
    Xui09MeasuredAndRefusalSets,
    /// One country vector's 16-byte header.
    Xui09VectorHeader,
    /// One country vector's state and completed rows.
    Xui09VectorRows,
}

/// The flag word a violation is about.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagWord {
    CountryRow,
    ActiveRow,
    Xui09Tail,
    Xui09CountryVector,
}

/// A broken flag-combination rule. Every one of these leaves the remaining
/// structure walkable, which is why they arrive as a `Recoverable` step and not
/// as an `Err`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleViolation {
    /// `1<<4` without `1<<3`: a receipt for a quartet that is not there.
    ObservedAuthorityWithoutQuartet,
    /// `1<<5` without `1<<3`.
    DerivedAuthorityWithoutQuartet,
    /// `1<<4` and `1<<5` together: a quartet cannot be both observed and
    /// derived.
    BothAuthorities,
    /// `1<<3` with neither `1<<4` nor `1<<5`: an unauthenticated quartet.
    QuartetWithoutAuthority,
}

/// The coarser bounds check a length-only reader would have made at this point,
/// carried so that reader's diagnostic does not change when the walk gets finer.
///
/// The measured and refusal sets are the case this exists for: the fold checks
/// each 4-byte tag and each 8-byte refusal as it reads it, because it validates
/// a refusal's reason as it goes, while both length-only walkers bounds-check
/// `measured_count*4 + refused_count*8` in one step. The walk performs the
/// fold's finer check — it must, to keep the fold's ordering — and the coarse
/// operands ride along here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AggregateNeed {
    pub section: Section,
    pub at: usize,
    pub len: usize,
}

/// What went wrong, with the operands each reader needs to print what it has
/// always printed. No message text lives here; see the module docs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    /// The payload is shorter than the bytes this section needs. `at + len` is
    /// the absolute end offset the section requires; `available` is the length
    /// of the slice the walk was given.
    Need {
        section: Section,
        at: usize,
        len: usize,
        available: usize,
        aggregate: Option<AggregateNeed>,
    },
    /// A count multiplied by a row width does not fit a `usize`.
    LengthOverflow {
        section: Section,
        count: usize,
        unit: usize,
    },
    /// An offset plus a length does not fit a `usize`.
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    /// The XUI09 tail does not start with `XU09`.
    BadMagic { found: u32, expected: u32 },
    /// A flag word carries a bit nobody assigned.
    UnknownFlags {
        word: FlagWord,
        value: u32,
        unknown: u32,
    },
    /// A flag-combination rule is broken.
    Rule { rule: RuleViolation, flags: u32 },
    /// The walk consumed fewer (or more) bytes than the slice holds.
    TrailingBytes { consumed: usize, total: usize },
    /// The caller drove the walk out of order — asked for a second active row
    /// without returning the first, or asked a finished walk for more. It is a
    /// caller defect, not a payload defect, and it is reported rather than
    /// panicked because this crate sits under a recorder that must fail closed
    /// and never abort the process.
    WalkOutOfOrder,
}

/// One violation, and the byte offset the walk had reached.
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

// --- Byte readers ------------------------------------------------------------
//
// Every one of these is reached only after a `need`, so the slices are in
// bounds -- and they PANIC on a short slice, which is why they are crate-private
// and every public path into this crate goes through a cursor or a view that
// bounds-checks first. The readers decode the fields inside a delimited span
// through those views, never through these.

/// The `u32` at `at`.
pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The `i32` at `at`.
pub(crate) fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}

/// The `u16` at `at`.
pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// The `u64` at `at`.
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

/// The `i64` at `at`.
pub(crate) fn i64_at(bytes: &[u8], at: usize) -> i64 {
    u64_at(bytes, at) as i64
}

fn overflow(section: Section, at: usize, count: usize, unit: usize) -> Violation {
    Violation::new(
        at,
        ViolationKind::LengthOverflow {
            section,
            count,
            unit,
        },
    )
}

/// `at + len`, or the section's overflow violation.
fn end_of(section: Section, at: usize, len: usize) -> Result<usize, Violation> {
    at.checked_add(len).ok_or(Violation::new(
        at,
        ViolationKind::OffsetOverflow { section, at, len },
    ))
}

/// `count * unit`, or the section's overflow violation.
fn span_len(section: Section, at: usize, count: usize, unit: usize) -> Result<usize, Violation> {
    count
        .checked_mul(unit)
        .ok_or_else(|| overflow(section, at, count, unit))
}

/// Require `len` bytes at `at`, and return the end offset.
///
/// Checked, never saturating: a saturated product turns an overflow into a
/// plausible-looking shortfall, and the operands a reader wants to print are
/// gone by then. On a 64-bit process a `u32` count times these row widths cannot
/// overflow a `usize`, so this changes no shipping outcome; it changes what a
/// synthetic malformed payload reports, and it keeps the arithmetic honest.
fn need(bytes: &[u8], section: Section, at: usize, len: usize) -> Result<usize, Violation> {
    need_with(bytes, section, at, len, None)
}

fn need_with(
    bytes: &[u8],
    section: Section,
    at: usize,
    len: usize,
    aggregate: Option<AggregateNeed>,
) -> Result<usize, Violation> {
    let end = end_of(section, at, len)?;
    if end > bytes.len() {
        return Err(Violation::new(
            at,
            ViolationKind::Need {
                section,
                at,
                len,
                available: bytes.len(),
                aggregate,
            },
        ));
    }
    Ok(end)
}

/// The exact-consumption rule, in the one place that owns it.
pub fn exact_consumption(consumed: usize, total: usize) -> Result<(), Violation> {
    if consumed == total {
        Ok(())
    } else {
        Err(Violation::new(
            consumed,
            ViolationKind::TrailingBytes { consumed, total },
        ))
    }
}

// --- The payload header ------------------------------------------------------

/// The 32-byte payload header.
///
/// `changed_rows`, `status_flags` and the two scale words are carried because
/// they are captured bytes; this crate acts on none of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct Header {
    pub elapsed_wall_ms: u64,
    pub country_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub progress_scale_raw: u32,
    pub live_scale_raw: u32,
    pub active_count: u32,
}

/// Read the header. Nothing but its presence is checked here — the readers'
/// header policies differ (the fold judges status first, the validator judges
/// the scale words first) and each runs before the body length is known.
pub fn read_header(payload: &[u8]) -> Result<Header, Violation> {
    need(payload, Section::Header, 0, HEADER_LEN)?;
    Ok(Header {
        elapsed_wall_ms: u64_at(payload, 0),
        country_count: u32_at(payload, 8),
        changed_rows: u32_at(payload, 12),
        status_flags: u32_at(payload, 16),
        progress_scale_raw: u32_at(payload, 20),
        live_scale_raw: u32_at(payload, 24),
        active_count: u32_at(payload, 28),
    })
}

/// Where the two tables sit, once the header's counts are known.
///
/// There is no `active_end`: the rows are variable-length and only
/// [`ActiveRowCursor`] can find their boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyLayout {
    /// The country table's span.
    pub countries: Range<usize>,
    /// Where the active table begins.
    pub active_start: usize,
    /// `HEADER_LEN + country_count*16 + active_count*40` — a MINIMUM, because
    /// the active rows carry optional tails.
    pub minimum_len: usize,
}

/// The country table's span and the active table's start, with the structural
/// minimum-length check.
pub fn body_layout(payload: &[u8], header: &Header) -> Result<BodyLayout, Violation> {
    let countries_len = span_len(
        Section::CountryTable,
        HEADER_LEN,
        header.country_count as usize,
        COUNTRY_ROW_LEN,
    )?;
    let countries_end = end_of(Section::CountryTable, HEADER_LEN, countries_len)?;
    let active_min = span_len(
        Section::PayloadMinimum,
        countries_end,
        header.active_count as usize,
        ACTIVE_ROW_FIXED_LEN,
    )?;
    let minimum_len = end_of(Section::PayloadMinimum, countries_end, active_min)?;
    if payload.len() < minimum_len {
        return Err(Violation::new(
            0,
            ViolationKind::Need {
                section: Section::PayloadMinimum,
                at: 0,
                len: minimum_len,
                available: payload.len(),
                aggregate: None,
            },
        ));
    }
    Ok(BodyLayout {
        countries: HEADER_LEN..countries_end,
        active_start: countries_end,
        minimum_len,
    })
}

// --- Country rows ------------------------------------------------------------

/// One country row's four words. Whether a clear flag over a non-sentinel word
/// is a contradiction, and whether an unassigned bit is fatal, are the readers'
/// decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountryRow {
    pub tag_db_id: u32,
    pub flags: u32,
    pub completed_tech_count: u32,
    pub research_slot_count: u32,
    /// This row's span in the payload.
    pub span: Range<usize>,
}

impl CountryRow {
    /// The bit that says `completed_tech_count` is a real reading.
    pub const fn completed_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_COMPLETED_TECH_COUNT != 0
    }
    /// The bit that says `research_slot_count` is a real reading.
    pub const fn slots_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_RESEARCH_SLOT_COUNT != 0
    }
    /// The bit that says the active-research LIST was read.
    pub const fn active_research_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_ACTIVE_RESEARCH != 0
    }
    /// The bits nobody assigned.
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !COUNTRY_KNOWN_FLAGS
    }
}

/// Read the 16-byte country row at `at`.
pub fn read_country_row(payload: &[u8], at: usize) -> Result<CountryRow, Violation> {
    let end = need(payload, Section::CountryRow, at, COUNTRY_ROW_LEN)?;
    Ok(CountryRow {
        tag_db_id: u32_at(payload, at),
        flags: u32_at(payload, at + 4),
        completed_tech_count: u32_at(payload, at + 8),
        research_slot_count: u32_at(payload, at + 12),
        span: at..end,
    })
}

// --- Active-research rows ----------------------------------------------------

/// The fixed 40 bytes of one active-research row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveFixed {
    pub tag_db_id: u32,
    pub slot_index: u32,
    pub tech_handle_raw: u32,
    pub tech_def_index_raw: u32,
    pub progress_raw: u32,
    pub flags: u32,
    /// The word at `+24`. A real reading only when [`ACTIVE_FLAG_LIVE_PROGRESS`]
    /// is set; the readers decide what a non-zero word without its flag means.
    pub live_progress_word: i64,
    /// The word at `+32`, same rule with [`ACTIVE_FLAG_BASE_REQUIRED_POINTS`].
    pub base_required_points_word: i64,
    /// The fixed part's span.
    pub span: Range<usize>,
}

impl ActiveFixed {
    /// The bits nobody assigned.
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !ACTIVE_KNOWN_FLAGS
    }
    /// Whether this row claims the given tail.
    pub const fn carries(&self, tail: ActiveTail) -> bool {
        self.flags & tail.flag_bit() != 0
    }
    /// Whether `live_progress_word` is a real reading.
    pub const fn live_progress_captured(&self) -> bool {
        self.flags & ACTIVE_FLAG_LIVE_PROGRESS != 0
    }
    /// Whether `base_required_points_word` is a real reading.
    pub const fn base_required_points_captured(&self) -> bool {
        self.flags & ACTIVE_FLAG_BASE_REQUIRED_POINTS != 0
    }
}

/// One present tail of an active row, delimited.
///
/// `span` is the tail's CONTENT: for [`ActiveTail::TechName`] that is the UTF-8
/// bytes, with the `u16` length prefix already consumed by the walk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveSection {
    pub tail: ActiveTail,
    pub span: Range<usize>,
}

/// One step of an active-row walk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActiveStep {
    /// The next present tail.
    Section(ActiveSection),
    /// A flag-combination rule is broken. The walk can continue; the caller
    /// decides whether that is a refusal.
    Recoverable(Violation),
    /// The row is fully delimited; the payload offset it ends at.
    End(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveStage {
    /// Consider `ActiveTail::ALL[index]`, having already emitted `rule` of its
    /// rules.
    Tail { index: usize, rule: usize },
    /// Every tail is delimited; the closing rule is left.
    Closing,
    /// Everything is emitted.
    Ended,
}

/// The number of combination rules one tail can break at once.
const ACTIVE_TAIL_MAX_RULES: usize = 2;

/// The pull cursor over ONE active-research row.
///
/// It owns the cursor and hands back one step at a time, having already done
/// that section's `need` and its combination rules at exactly the point in the
/// row the readers do them today.
#[derive(Clone, Debug)]
pub struct ActiveRowCursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    fixed: ActiveFixed,
    stage: ActiveStage,
}

impl<'a> ActiveRowCursor<'a> {
    /// Begin a row at `start`: the 40-byte `need`, then the flag word.
    ///
    /// The returned `Option<Violation>` is the unknown-flag violation, which is
    /// RECOVERABLE by construction — the known bits still describe a walkable
    /// row. The fold and the port refuse on it; the validator records it and
    /// walks on. Nothing else in this crate reports unknown active flags.
    pub fn begin(payload: &'a [u8], start: usize) -> Result<(Self, Option<Violation>), Violation> {
        let end = need(payload, Section::ActiveFixed, start, ACTIVE_ROW_FIXED_LEN)?;
        let fixed = ActiveFixed {
            tag_db_id: u32_at(payload, start),
            slot_index: u32_at(payload, start + 4),
            tech_handle_raw: u32_at(payload, start + 8),
            tech_def_index_raw: u32_at(payload, start + 12),
            progress_raw: u32_at(payload, start + 16),
            flags: u32_at(payload, start + 20),
            live_progress_word: i64_at(payload, start + 24),
            base_required_points_word: i64_at(payload, start + 32),
            span: start..end,
        };
        let unknown = fixed.unknown_flags();
        let violation = if unknown == 0 {
            None
        } else {
            Some(Violation::new(
                start,
                ViolationKind::UnknownFlags {
                    word: FlagWord::ActiveRow,
                    value: fixed.flags,
                    unknown,
                },
            ))
        };
        Ok((
            ActiveRowCursor {
                payload,
                cursor: end,
                fixed,
                stage: ActiveStage::Tail { index: 0, rule: 0 },
            },
            violation,
        ))
    }

    /// The row's fixed words.
    pub fn fixed(&self) -> &ActiveFixed {
        &self.fixed
    }

    /// The row's flag word.
    pub fn flags(&self) -> u32 {
        self.fixed.flags
    }

    /// The offset the walk has reached.
    pub fn offset(&self) -> usize {
        self.cursor
    }

    /// The next rule, section, or the row's end.
    ///
    /// Calling it again after [`ActiveStep::End`] returns `End` again: the row's
    /// end offset is a fact, not a resource.
    pub fn next_step(&mut self) -> Result<ActiveStep, Violation> {
        loop {
            match self.stage {
                ActiveStage::Tail { index, rule } => {
                    if index >= ActiveTail::ALL.len() {
                        self.stage = ActiveStage::Closing;
                        continue;
                    }
                    let tail = ActiveTail::ALL[index];
                    if !self.fixed.carries(tail) {
                        self.stage = ActiveStage::Tail {
                            index: index + 1,
                            rule: 0,
                        };
                        continue;
                    }
                    if rule < ACTIVE_TAIL_MAX_RULES {
                        let rules = self.rules_for(tail);
                        self.stage = ActiveStage::Tail {
                            index,
                            rule: rule + 1,
                        };
                        match rules[rule] {
                            Some(broken) => {
                                return Ok(ActiveStep::Recoverable(Violation::new(
                                    self.cursor,
                                    ViolationKind::Rule {
                                        rule: broken,
                                        flags: self.fixed.flags,
                                    },
                                )))
                            }
                            None => continue,
                        }
                    }
                    self.stage = ActiveStage::Tail {
                        index: index + 1,
                        rule: 0,
                    };
                    return self.measure(tail).map(ActiveStep::Section);
                }
                ActiveStage::Closing => {
                    self.stage = ActiveStage::Ended;
                    if self.fixed.carries(ActiveTail::AheadQuartet)
                        && !self.fixed.carries(ActiveTail::ObservedAuthority)
                        && !self.fixed.carries(ActiveTail::DerivedAuthority)
                    {
                        return Ok(ActiveStep::Recoverable(Violation::new(
                            self.cursor,
                            ViolationKind::Rule {
                                rule: RuleViolation::QuartetWithoutAuthority,
                                flags: self.fixed.flags,
                            },
                        )));
                    }
                    continue;
                }
                ActiveStage::Ended => return Ok(ActiveStep::End(self.cursor)),
            }
        }
    }

    /// The rules this tail's presence breaks, in the order the readers check
    /// them. Exhaustive over [`ActiveTail`]: a new tail must say here whether it
    /// constrains, or is constrained by, the others.
    fn rules_for(&self, tail: ActiveTail) -> [Option<RuleViolation>; ACTIVE_TAIL_MAX_RULES] {
        let quartet = self.fixed.carries(ActiveTail::AheadQuartet);
        match tail {
            ActiveTail::TechName | ActiveTail::AheadQuartet => [None, None],
            ActiveTail::ObservedAuthority => [
                if quartet {
                    None
                } else {
                    Some(RuleViolation::ObservedAuthorityWithoutQuartet)
                },
                None,
            ],
            ActiveTail::DerivedAuthority => [
                if quartet {
                    None
                } else {
                    Some(RuleViolation::DerivedAuthorityWithoutQuartet)
                },
                if self.fixed.carries(ActiveTail::ObservedAuthority) {
                    Some(RuleViolation::BothAuthorities)
                } else {
                    None
                },
            ],
        }
    }

    /// Delimit one present tail and advance. Exhaustive over [`ActiveTail`].
    fn measure(&mut self, tail: ActiveTail) -> Result<ActiveSection, Violation> {
        let span = match tail {
            ActiveTail::TechName => {
                let after_prefix = need(self.payload, Section::TechNamePrefix, self.cursor, 2)?;
                let len = u16_at(self.payload, self.cursor) as usize;
                let end = need(self.payload, Section::TechNameBytes, after_prefix, len)?;
                self.cursor = end;
                after_prefix..end
            }
            ActiveTail::AheadQuartet
            | ActiveTail::ObservedAuthority
            | ActiveTail::DerivedAuthority => {
                // Every tail but the name has a fixed length; `fixed_len` is the
                // same declaration, so the two cannot disagree.
                let len = tail.fixed_len().unwrap_or(0);
                let end = need(self.payload, tail.section(), self.cursor, len)?;
                let span = self.cursor..end;
                self.cursor = end;
                span
            }
        };
        Ok(ActiveSection { tail, span })
    }
}

/// The total length of the active row beginning at `start`, with every rule and
/// unknown-flag violation treated as fatal.
///
/// This is the length-only entry point: it is what a caller that only needs to
/// know where the row ends calls, and it is the one implementation behind
/// `wire_format::d13::active_row_len`.
pub fn active_row_len(payload: &[u8], start: usize) -> Result<usize, Violation> {
    let (mut cursor, unknown) = ActiveRowCursor::begin(payload, start)?;
    if let Some(violation) = unknown {
        return Err(violation);
    }
    loop {
        match cursor.next_step()? {
            ActiveStep::Section(_) => {}
            ActiveStep::Recoverable(violation) => return Err(violation),
            ActiveStep::End(end) => return Ok(end - start),
        }
    }
}

// --- Views over the two authority tails --------------------------------------

/// The four ahead-of-time values, in wire order. Reads only; every range and
/// branch-shape rule is the readers'.
#[derive(Clone, Copy, Debug)]
pub struct AheadQuartetView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> AheadQuartetView<'a> {
    /// Over the span [`ActiveTail::AheadQuartet`] delimited.
    pub fn new(payload: &'a [u8], span: Range<usize>) -> Option<Self> {
        if span.end.checked_sub(span.start) != Some(AHEAD_QUARTET_LEN) || span.end > payload.len() {
            return None;
        }
        Some(AheadQuartetView {
            bytes: payload,
            at: span.start,
        })
    }
    pub fn years_raw(&self) -> i64 {
        i64_at(self.bytes, self.at)
    }
    pub fn penalty_raw(&self) -> i64 {
        i64_at(self.bytes, self.at + 8)
    }
    pub fn reduction_years_raw(&self) -> i64 {
        i64_at(self.bytes, self.at + 16)
    }
    pub fn reduced_penalty_raw(&self) -> i64 {
        i64_at(self.bytes, self.at + 24)
    }
    /// The quartet as the tuple the three readers pass around.
    pub fn tuple(&self) -> (i64, i64, i64, i64) {
        (
            self.years_raw(),
            self.penalty_raw(),
            self.reduction_years_raw(),
            self.reduced_penalty_raw(),
        )
    }
}

/// The 176-byte observed-producer receipt's field offsets.
///
/// **Offsets only.** All twenty clauses of the A29 acceptance predicate stay
/// with the readers, and two of them are not evaluable in `wire_format` at all.
/// This exists because the same offset table was transcribed in three files, one
/// level below the tail inventory, and that is the same defect class.
#[derive(Clone, Copy, Debug)]
pub struct A29ProvenanceView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> A29ProvenanceView<'a> {
    /// Over the span [`ActiveTail::ObservedAuthority`] delimited.
    pub fn new(payload: &'a [u8], span: Range<usize>) -> Option<Self> {
        if span.end.checked_sub(span.start) != Some(A29_PROVENANCE_LEN) || span.end > payload.len()
        {
            return None;
        }
        Some(A29ProvenanceView {
            bytes: payload,
            at: span.start,
        })
    }
    pub fn authority(&self) -> u32 {
        u32_at(self.bytes, self.at)
    }
    pub fn localization_branch(&self) -> u32 {
        u32_at(self.bytes, self.at + 4)
    }
    pub fn callback_sequence(&self) -> u64 {
        u64_at(self.bytes, self.at + 8)
    }
    pub fn callback_thread_id(&self) -> u32 {
        u32_at(self.bytes, self.at + 16)
    }
    pub fn producer_game_hour_raw(&self) -> i32 {
        i32_at(self.bytes, self.at + 20)
    }
    pub fn callback_qpc_ticks(&self) -> u64 {
        u64_at(self.bytes, self.at + 24)
    }
    pub fn qpc_frequency_hz(&self) -> u64 {
        u64_at(self.bytes, self.at + 32)
    }
    pub fn source_record_sha256(&self) -> &'a [u8] {
        &self.bytes[self.at + 40..self.at + 72]
    }
    pub fn ui_action_gate_return_rva(&self) -> u32 {
        u32_at(self.bytes, self.at + 72)
    }
    pub fn selected_present_candidate_count(&self) -> u32 {
        u32_at(self.bytes, self.at + 76)
    }
    pub fn selected_present_ordinal(&self) -> u32 {
        u32_at(self.bytes, self.at + 80)
    }
    pub fn selected_present_game_hour_raw(&self) -> i32 {
        i32_at(self.bytes, self.at + 84)
    }
    pub fn selected_present_candidate_id(&self) -> u64 {
        u64_at(self.bytes, self.at + 88)
    }
    pub fn selected_present_qpc_ticks(&self) -> u64 {
        u64_at(self.bytes, self.at + 96)
    }
    pub fn selected_present_cutoff_qpc_ticks(&self) -> u64 {
        u64_at(self.bytes, self.at + 104)
    }
    pub fn selected_present_candidate_set_sha256(&self) -> &'a [u8] {
        &self.bytes[self.at + 112..self.at + 144]
    }
    pub fn selected_present_receipt_sha256(&self) -> &'a [u8] {
        &self.bytes[self.at + 144..self.at + 176]
    }
}

/// The 16-byte derived-authority tail's field offsets.
#[derive(Clone, Copy, Debug)]
pub struct DerivedAuthorityView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> DerivedAuthorityView<'a> {
    /// Over the span [`ActiveTail::DerivedAuthority`] delimited.
    pub fn new(payload: &'a [u8], span: Range<usize>) -> Option<Self> {
        if span.end.checked_sub(span.start) != Some(A29_DERIVED_AUTHORITY_LEN)
            || span.end > payload.len()
        {
            return None;
        }
        Some(DerivedAuthorityView {
            bytes: payload,
            at: span.start,
        })
    }
    pub fn authority(&self) -> u32 {
        u32_at(self.bytes, self.at)
    }
    pub fn producer_game_hour_raw(&self) -> i32 {
        i32_at(self.bytes, self.at + 4)
    }
    pub fn penalty_factor_raw(&self) -> i64 {
        i64_at(self.bytes, self.at + 8)
    }
}

// --- The XUI09 witness tail --------------------------------------------------

/// The XUI09 tail's 24-byte header.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct Xui09Header {
    pub flags: u32,
    pub dictionary_count: u32,
    pub measured_count: u32,
    pub refused_count: u32,
    pub vector_count: u32,
    /// Where the header's own bytes sit.
    pub at: usize,
}

impl Xui09Header {
    /// The country vectors are a patch rather than the whole witness.
    pub const fn patch(&self) -> bool {
        self.flags & XUI09_FLAG_PATCH != 0
    }
    /// The bits nobody assigned.
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !XUI09_KNOWN_FLAGS
    }
}

/// One step of an XUI09 tail walk.
///
/// The country vector arrives in TWO steps: the header, so the fold can run its
/// unknown-flag, measured-membership and duplicate checks, and then the rows,
/// whose `need` must not happen before those checks.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Xui09Step {
    /// One dictionary entry: a definition index and its id bytes (the `u16`
    /// prefix is already consumed).
    DictionaryEntry {
        def_index: u32,
        id_bytes: Range<usize>,
    },
    /// One measured country.
    MeasuredTag { tag_db_id: u32 },
    /// One refusal, with the producer's reason code. The reason's assignment is
    /// the fold's business.
    Refusal { tag_db_id: u32, reason: u32 },
    /// One country vector's header.
    CountryVectorHeader {
        tag_db_id: u32,
        flags: u32,
        dense: bool,
        state_count: u32,
        completed_count: u32,
    },
    /// The same vector's rows.
    CountryVectorRows {
        /// `state_count` entries of 1 byte (dense) or 5 bytes (sparse).
        states: Range<usize>,
        /// `completed_count` `u32` definition indices.
        completed: Range<usize>,
    },
    /// A flag word carries an unassigned bit. Recoverable: the two length-only
    /// readers have always ignored these, and the fold refuses on them.
    Recoverable(Violation),
    /// The tail is fully delimited; the payload offset it ends at.
    End(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Xui09Stage {
    Dictionary(u32),
    Measured(u32),
    Refusals(u32),
    Vector(u32),
    VectorFlags {
        remaining: u32,
        unknown: u32,
        flags: u32,
        at: usize,
    },
    VectorRows {
        remaining: u32,
        dense: bool,
        state_count: u32,
        completed_count: u32,
    },
    Ended,
}

/// The pull cursor over the XUI09 witness tail.
#[derive(Clone, Debug)]
pub struct Xui09Cursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    header: Xui09Header,
    /// Where the measured set begins — the aggregate operand the length-only
    /// readers print.
    sets_start: usize,
    sets_len: usize,
    stage: Xui09Stage,
}

impl<'a> Xui09Cursor<'a> {
    /// Begin the tail at `start`, or report that there is none.
    ///
    /// `Ok(None)` means `start == payload.len()`: no tail at all, which is NOT
    /// "unchanged" — it is "nothing was measured this frame" — but that reading
    /// belongs to the fold.
    ///
    /// The `Option<Violation>` beside the cursor is the tail flag word's
    /// unknown-bit violation.
    #[allow(clippy::type_complexity)]
    pub fn begin(
        payload: &'a [u8],
        start: usize,
    ) -> Result<Option<(Self, Option<Violation>)>, Violation> {
        if start == payload.len() {
            return Ok(None);
        }
        let after_header = need(payload, Section::Xui09TailHeader, start, XUI09_TAIL_HEADER_LEN)?;
        let found = u32_at(payload, start);
        if found != XUI09_TAIL_MAGIC {
            return Err(Violation::new(
                start,
                ViolationKind::BadMagic {
                    found,
                    expected: XUI09_TAIL_MAGIC,
                },
            ));
        }
        let header = Xui09Header {
            flags: u32_at(payload, start + 4),
            dictionary_count: u32_at(payload, start + 8),
            measured_count: u32_at(payload, start + 12),
            refused_count: u32_at(payload, start + 16),
            vector_count: u32_at(payload, start + 20),
            at: start,
        };
        let unknown = header.unknown_flags();
        let violation = if unknown == 0 {
            None
        } else {
            Some(Violation::new(
                start,
                ViolationKind::UnknownFlags {
                    word: FlagWord::Xui09Tail,
                    value: header.flags,
                    unknown,
                },
            ))
        };
        Ok(Some((
            Xui09Cursor {
                payload,
                cursor: after_header,
                header,
                sets_start: 0,
                sets_len: 0,
                stage: Xui09Stage::Dictionary(header.dictionary_count),
            },
            violation,
        )))
    }

    /// The tail's header.
    pub fn header(&self) -> &Xui09Header {
        &self.header
    }

    /// The offset the walk has reached.
    pub fn offset(&self) -> usize {
        self.cursor
    }

    /// The next section, recoverable violation, or the tail's end.
    pub fn next_step(&mut self) -> Result<Xui09Step, Violation> {
        loop {
            match self.stage {
                Xui09Stage::Dictionary(0) => {
                    // The measured and refusal sets are one bounds check to the
                    // length-only readers; their operands are computed once,
                    // here, and ride on every per-item `Need` below.
                    self.sets_start = self.cursor;
                    let measured = span_len(
                        Section::Xui09MeasuredAndRefusalSets,
                        self.cursor,
                        self.header.measured_count as usize,
                        XUI09_MEASURED_ENTRY_LEN,
                    )?;
                    let refused = span_len(
                        Section::Xui09MeasuredAndRefusalSets,
                        self.cursor,
                        self.header.refused_count as usize,
                        XUI09_REFUSAL_ENTRY_LEN,
                    )?;
                    self.sets_len = measured
                        .checked_add(refused)
                        .ok_or(Violation::new(
                            self.cursor,
                            ViolationKind::OffsetOverflow {
                                section: Section::Xui09MeasuredAndRefusalSets,
                                at: measured,
                                len: refused,
                            },
                        ))?;
                    self.stage = Xui09Stage::Measured(self.header.measured_count);
                }
                Xui09Stage::Dictionary(remaining) => {
                    let at = self.cursor;
                    let aggregate = Some(AggregateNeed {
                        section: Section::Xui09DictionaryEntry,
                        at,
                        len: XUI09_DICTIONARY_ENTRY_MIN_LEN,
                    });
                    let after_index = need_with(
                        self.payload,
                        Section::Xui09DictionaryIndex,
                        at,
                        XUI09_DICTIONARY_INDEX_LEN,
                        aggregate,
                    )?;
                    let def_index = u32_at(self.payload, at);
                    let after_prefix = need_with(
                        self.payload,
                        Section::Xui09DictionaryIdPrefix,
                        after_index,
                        2,
                        aggregate,
                    )?;
                    let len = u16_at(self.payload, after_index) as usize;
                    let end = need(
                        self.payload,
                        Section::Xui09DictionaryIdBytes,
                        after_prefix,
                        len,
                    )?;
                    self.cursor = end;
                    self.stage = Xui09Stage::Dictionary(remaining - 1);
                    return Ok(Xui09Step::DictionaryEntry {
                        def_index,
                        id_bytes: after_prefix..end,
                    });
                }
                Xui09Stage::Measured(0) => {
                    self.stage = Xui09Stage::Refusals(self.header.refused_count);
                }
                Xui09Stage::Measured(remaining) => {
                    let at = self.cursor;
                    let end = need_with(
                        self.payload,
                        Section::Xui09MeasuredTag,
                        at,
                        XUI09_MEASURED_ENTRY_LEN,
                        self.sets_aggregate(),
                    )?;
                    self.cursor = end;
                    self.stage = Xui09Stage::Measured(remaining - 1);
                    return Ok(Xui09Step::MeasuredTag {
                        tag_db_id: u32_at(self.payload, at),
                    });
                }
                Xui09Stage::Refusals(0) => {
                    self.stage = Xui09Stage::Vector(self.header.vector_count);
                }
                Xui09Stage::Refusals(remaining) => {
                    let at = self.cursor;
                    let end = need_with(
                        self.payload,
                        Section::Xui09Refusal,
                        at,
                        XUI09_REFUSAL_ENTRY_LEN,
                        self.sets_aggregate(),
                    )?;
                    self.cursor = end;
                    self.stage = Xui09Stage::Refusals(remaining - 1);
                    return Ok(Xui09Step::Refusal {
                        tag_db_id: u32_at(self.payload, at),
                        reason: u32_at(self.payload, at + 4),
                    });
                }
                Xui09Stage::Vector(0) => {
                    self.stage = Xui09Stage::Ended;
                }
                Xui09Stage::Vector(remaining) => {
                    let at = self.cursor;
                    let end = need(
                        self.payload,
                        Section::Xui09VectorHeader,
                        at,
                        XUI09_VECTOR_HEADER_LEN,
                    )?;
                    let flags = u32_at(self.payload, at + 4);
                    self.cursor = end;
                    let unknown = flags & !XUI09_COUNTRY_KNOWN_FLAGS;
                    self.stage = Xui09Stage::VectorFlags {
                        remaining,
                        unknown,
                        flags,
                        at,
                    };
                    return Ok(Xui09Step::CountryVectorHeader {
                        tag_db_id: u32_at(self.payload, at),
                        flags,
                        dense: flags & XUI09_COUNTRY_FLAG_DENSE != 0,
                        state_count: u32_at(self.payload, at + 8),
                        completed_count: u32_at(self.payload, at + 12),
                    });
                }
                Xui09Stage::VectorFlags {
                    remaining,
                    unknown,
                    flags,
                    at,
                } => {
                    let dense = flags & XUI09_COUNTRY_FLAG_DENSE != 0;
                    self.stage = Xui09Stage::VectorRows {
                        remaining,
                        dense,
                        state_count: u32_at(self.payload, at + 8),
                        completed_count: u32_at(self.payload, at + 12),
                    };
                    if unknown != 0 {
                        return Ok(Xui09Step::Recoverable(Violation::new(
                            at,
                            ViolationKind::UnknownFlags {
                                word: FlagWord::Xui09CountryVector,
                                value: flags,
                                unknown,
                            },
                        )));
                    }
                }
                Xui09Stage::VectorRows {
                    remaining,
                    dense,
                    state_count,
                    completed_count,
                } => {
                    let at = self.cursor;
                    let state_bytes = if dense {
                        XUI09_DENSE_STATE_LEN
                    } else {
                        XUI09_SPARSE_STATE_LEN
                    };
                    let states_len = span_len(
                        Section::Xui09VectorRows,
                        at,
                        state_count as usize,
                        state_bytes,
                    )?;
                    let completed_len = span_len(
                        Section::Xui09VectorRows,
                        at,
                        completed_count as usize,
                        XUI09_COMPLETED_ENTRY_LEN,
                    )?;
                    let rows_len = states_len.checked_add(completed_len).ok_or(Violation::new(
                        at,
                        ViolationKind::OffsetOverflow {
                            section: Section::Xui09VectorRows,
                            at: states_len,
                            len: completed_len,
                        },
                    ))?;
                    let end = need(self.payload, Section::Xui09VectorRows, at, rows_len)?;
                    let states_end = at + states_len;
                    self.cursor = end;
                    self.stage = Xui09Stage::Vector(remaining - 1);
                    return Ok(Xui09Step::CountryVectorRows {
                        states: at..states_end,
                        completed: states_end..end,
                    });
                }
                Xui09Stage::Ended => return Ok(Xui09Step::End(self.cursor)),
            }
        }
    }

    fn sets_aggregate(&self) -> Option<AggregateNeed> {
        Some(AggregateNeed {
            section: Section::Xui09MeasuredAndRefusalSets,
            at: self.sets_start,
            len: self.sets_len,
        })
    }
}

/// The byte length of the XUI09 witness tail beginning at `start`, or `0` when
/// there is none.
///
/// LENGTH ONLY: the recoverable unknown-flag steps are ignored here, exactly as
/// both length-only walkers have always ignored them, because a decoder that
/// only needs to know where the record ends must not refuse a tail the fold
/// would accept. It exists because a decoder that does not know about the tail
/// reports it as trailing bytes and refuses a perfectly good payload — which is
/// not hypothetical: it cost a live capture its entire D13 domain.
pub fn xui09_tail_len(payload: &[u8], start: usize) -> Result<usize, Violation> {
    let Some((mut cursor, _unknown_tail_flags)) = Xui09Cursor::begin(payload, start)? else {
        return Ok(0);
    };
    loop {
        match cursor.next_step()? {
            // Every section is a length this walk has already accounted for.
            Xui09Step::DictionaryEntry { .. }
            | Xui09Step::MeasuredTag { .. }
            | Xui09Step::Refusal { .. }
            | Xui09Step::CountryVectorHeader { .. }
            | Xui09Step::CountryVectorRows { .. } => {}
            // Deliberately ignored; see the doc comment.
            Xui09Step::Recoverable(_) => {}
            Xui09Step::End(end) => return Ok(end - start),
        }
    }
}
