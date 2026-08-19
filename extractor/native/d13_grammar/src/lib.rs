#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use core::ops::Range;

pub mod encode;

#[cfg(test)]
mod tests;

pub const HEADER_LEN: usize = 32;
pub const COUNTRY_ROW_LEN: usize = 16;
pub const ACTIVE_ROW_FIXED_LEN: usize = 40;
pub const AHEAD_QUARTET_LEN: usize = 32;
pub const A29_PROVENANCE_LEN: usize = 176;
pub const A29_DERIVED_AUTHORITY_LEN: usize = 16;

pub const XUI09_TAIL_MAGIC: u32 = u32::from_le_bytes(*b"XU09");
pub const XUI09_TAIL_HEADER_LEN: usize = 24;
pub const XUI09_DICTIONARY_INDEX_LEN: usize = 4;
pub const XUI09_DICTIONARY_ENTRY_MIN_LEN: usize = XUI09_DICTIONARY_INDEX_LEN + 2;
pub const XUI09_MEASURED_ENTRY_LEN: usize = 4;
pub const XUI09_REFUSAL_ENTRY_LEN: usize = 8;
pub const XUI09_VECTOR_HEADER_LEN: usize = 16;
pub const XUI09_DENSE_STATE_LEN: usize = 1;
pub const XUI09_SPARSE_STATE_LEN: usize = 5;
pub const XUI09_COMPLETED_ENTRY_LEN: usize = 4;

pub const COUNTRY_FLAG_COMPLETED_TECH_COUNT: u32 = 1;
pub const COUNTRY_FLAG_RESEARCH_SLOT_COUNT: u32 = 1 << 1;
pub const COUNTRY_FLAG_ACTIVE_RESEARCH: u32 = 1 << 2;
pub const COUNTRY_KNOWN_FLAGS: u32 = COUNTRY_FLAG_COMPLETED_TECH_COUNT
    | COUNTRY_FLAG_RESEARCH_SLOT_COUNT
    | COUNTRY_FLAG_ACTIVE_RESEARCH;
pub const COUNTRY_COUNT_ABSENT_SENTINEL: u32 = u32::MAX;

pub const ACTIVE_FLAG_LIVE_PROGRESS: u32 = 1;
pub const ACTIVE_FLAG_BASE_REQUIRED_POINTS: u32 = 1 << 1;
pub const ACTIVE_VALUE_FLAGS: u32 = ACTIVE_FLAG_LIVE_PROGRESS | ACTIVE_FLAG_BASE_REQUIRED_POINTS;

/// **This is the declaration the rest of the crate is generated from.** Every
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ActiveTail {
    TechName,
    AheadQuartet,
    ObservedAuthority,
    DerivedAuthority,
}

impl ActiveTail {
    pub const ALL: [ActiveTail; 4] = [
        ActiveTail::TechName,
        ActiveTail::AheadQuartet,
        ActiveTail::ObservedAuthority,
        ActiveTail::DerivedAuthority,
    ];

    pub const fn flag_bit(self) -> u32 {
        match self {
            ActiveTail::TechName => 1 << 2,
            ActiveTail::AheadQuartet => 1 << 3,
            ActiveTail::ObservedAuthority => 1 << 4,
            ActiveTail::DerivedAuthority => 1 << 5,
        }
    }

    pub const fn fixed_len(self) -> Option<usize> {
        match self {
            ActiveTail::TechName => None,
            ActiveTail::AheadQuartet => Some(AHEAD_QUARTET_LEN),
            ActiveTail::ObservedAuthority => Some(A29_PROVENANCE_LEN),
            ActiveTail::DerivedAuthority => Some(A29_DERIVED_AUTHORITY_LEN),
        }
    }

    const fn section(self) -> Section {
        match self {
            ActiveTail::TechName => Section::TechNameBytes,
            ActiveTail::AheadQuartet => Section::AheadQuartet,
            ActiveTail::ObservedAuthority => Section::ObservedAuthority,
            ActiveTail::DerivedAuthority => Section::DerivedAuthority,
        }
    }
}

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

pub const XUI09_FLAG_PATCH: u32 = 1;
pub const XUI09_KNOWN_FLAGS: u32 = XUI09_FLAG_PATCH;
pub const XUI09_COUNTRY_FLAG_DENSE: u32 = 1;
pub const XUI09_COUNTRY_KNOWN_FLAGS: u32 = XUI09_COUNTRY_FLAG_DENSE;

pub const TECH_PROGRESS_SCALE_RAW: u32 = 1000;
pub const TECH_LIVE_SCALE_RAW: u32 = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Header,
    PayloadMinimum,
    CountryTable,
    CountryRow,
    ActiveFixed,
    TechNamePrefix,
    TechNameBytes,
    AheadQuartet,
    ObservedAuthority,
    DerivedAuthority,
    Xui09TailHeader,
    Xui09DictionaryIndex,
    Xui09DictionaryEntry,
    Xui09DictionaryIdPrefix,
    Xui09DictionaryIdBytes,
    Xui09MeasuredTag,
    Xui09Refusal,
    Xui09MeasuredAndRefusalSets,
    Xui09VectorHeader,
    Xui09VectorRows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlagWord {
    CountryRow,
    ActiveRow,
    Xui09Tail,
    Xui09CountryVector,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuleViolation {
    ObservedAuthorityWithoutQuartet,
    DerivedAuthorityWithoutQuartet,
    BothAuthorities,
    QuartetWithoutAuthority,
}

/// The coarser bounds check a length-only reader would have made at this point,
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AggregateNeed {
    pub section: Section,
    pub at: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    Need {
        section: Section,
        at: usize,
        len: usize,
        available: usize,
        aggregate: Option<AggregateNeed>,
    },
    LengthOverflow {
        section: Section,
        count: usize,
        unit: usize,
    },
    OffsetOverflow {
        section: Section,
        at: usize,
        len: usize,
    },
    BadMagic {
        found: u32,
        expected: u32,
    },
    UnknownFlags {
        word: FlagWord,
        value: u32,
        unknown: u32,
    },
    Rule {
        rule: RuleViolation,
        flags: u32,
    },
    TrailingBytes {
        consumed: usize,
        total: usize,
    },
    WalkOutOfOrder,
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

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn i32_at(bytes: &[u8], at: usize) -> i32 {
    u32_at(bytes, at) as i32
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
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

fn end_of(section: Section, at: usize, len: usize) -> Result<usize, Violation> {
    at.checked_add(len).ok_or(Violation::new(
        at,
        ViolationKind::OffsetOverflow { section, at, len },
    ))
}

fn span_len(section: Section, at: usize, count: usize, unit: usize) -> Result<usize, Violation> {
    count
        .checked_mul(unit)
        .ok_or_else(|| overflow(section, at, count, unit))
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BodyLayout {
    pub countries: Range<usize>,
    pub active_start: usize,
    pub minimum_len: usize,
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CountryRow {
    pub tag_db_id: u32,
    pub flags: u32,
    pub completed_tech_count: u32,
    pub research_slot_count: u32,
    pub span: Range<usize>,
}

impl CountryRow {
    pub const fn completed_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_COMPLETED_TECH_COUNT != 0
    }
    pub const fn slots_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_RESEARCH_SLOT_COUNT != 0
    }
    pub const fn active_research_captured(&self) -> bool {
        self.flags & COUNTRY_FLAG_ACTIVE_RESEARCH != 0
    }
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !COUNTRY_KNOWN_FLAGS
    }
}

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveFixed {
    pub tag_db_id: u32,
    pub slot_index: u32,
    pub tech_handle_raw: u32,
    pub tech_def_index_raw: u32,
    pub progress_raw: u32,
    pub flags: u32,
    pub live_progress_word: i64,
    pub base_required_points_word: i64,
    pub span: Range<usize>,
}

impl ActiveFixed {
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !ACTIVE_KNOWN_FLAGS
    }
    pub const fn carries(&self, tail: ActiveTail) -> bool {
        self.flags & tail.flag_bit() != 0
    }
    pub const fn live_progress_captured(&self) -> bool {
        self.flags & ACTIVE_FLAG_LIVE_PROGRESS != 0
    }
    pub const fn base_required_points_captured(&self) -> bool {
        self.flags & ACTIVE_FLAG_BASE_REQUIRED_POINTS != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveSection {
    pub tail: ActiveTail,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActiveStep {
    Section(ActiveSection),
    Recoverable(Violation),
    End(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActiveStage {
    Tail { index: usize, rule: usize },
    Closing,
    Ended,
}

const ACTIVE_TAIL_MAX_RULES: usize = 2;

#[derive(Clone, Debug)]
pub struct ActiveRowCursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    fixed: ActiveFixed,
    stage: ActiveStage,
}

impl<'a> ActiveRowCursor<'a> {
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

    pub fn fixed(&self) -> &ActiveFixed {
        &self.fixed
    }

    pub fn flags(&self) -> u32 {
        self.fixed.flags
    }

    pub fn offset(&self) -> usize {
        self.cursor
    }

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

#[derive(Clone, Copy, Debug)]
pub struct AheadQuartetView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> AheadQuartetView<'a> {
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
    pub fn tuple(&self) -> (i64, i64, i64, i64) {
        (
            self.years_raw(),
            self.penalty_raw(),
            self.reduction_years_raw(),
            self.reduced_penalty_raw(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct A29ProvenanceView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> A29ProvenanceView<'a> {
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

#[derive(Clone, Copy, Debug)]
pub struct DerivedAuthorityView<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> DerivedAuthorityView<'a> {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct Xui09Header {
    pub flags: u32,
    pub dictionary_count: u32,
    pub measured_count: u32,
    pub refused_count: u32,
    pub vector_count: u32,
    pub at: usize,
}

impl Xui09Header {
    pub const fn patch(&self) -> bool {
        self.flags & XUI09_FLAG_PATCH != 0
    }
    pub const fn unknown_flags(&self) -> u32 {
        self.flags & !XUI09_KNOWN_FLAGS
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Xui09Step {
    DictionaryEntry {
        def_index: u32,
        id_bytes: Range<usize>,
    },
    MeasuredTag {
        tag_db_id: u32,
    },
    Refusal {
        tag_db_id: u32,
        reason: u32,
    },
    CountryVectorHeader {
        tag_db_id: u32,
        flags: u32,
        dense: bool,
        state_count: u32,
        completed_count: u32,
    },
    CountryVectorRows {
        states: Range<usize>,
        completed: Range<usize>,
    },
    Recoverable(Violation),
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

#[derive(Clone, Debug)]
pub struct Xui09Cursor<'a> {
    payload: &'a [u8],
    cursor: usize,
    header: Xui09Header,
    sets_start: usize,
    sets_len: usize,
    stage: Xui09Stage,
}

impl<'a> Xui09Cursor<'a> {
    #[allow(clippy::type_complexity)]
    pub fn begin(
        payload: &'a [u8],
        start: usize,
    ) -> Result<Option<(Self, Option<Violation>)>, Violation> {
        if start == payload.len() {
            return Ok(None);
        }
        let after_header = need(
            payload,
            Section::Xui09TailHeader,
            start,
            XUI09_TAIL_HEADER_LEN,
        )?;
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

    pub fn header(&self) -> &Xui09Header {
        &self.header
    }

    pub fn offset(&self) -> usize {
        self.cursor
    }

    pub fn next_step(&mut self) -> Result<Xui09Step, Violation> {
        loop {
            match self.stage {
                Xui09Stage::Dictionary(0) => {
                    // The measured and refusal sets are one bounds check to the
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
                    self.sets_len = measured.checked_add(refused).ok_or(Violation::new(
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

pub fn xui09_tail_len(payload: &[u8], start: usize) -> Result<usize, Violation> {
    let Some((mut cursor, _unknown_tail_flags)) = Xui09Cursor::begin(payload, start)? else {
        return Ok(0);
    };
    loop {
        match cursor.next_step()? {
            Xui09Step::DictionaryEntry { .. }
            | Xui09Step::MeasuredTag { .. }
            | Xui09Step::Refusal { .. }
            | Xui09Step::CountryVectorHeader { .. }
            | Xui09Step::CountryVectorRows { .. } => {}
            Xui09Step::Recoverable(_) => {}
            Xui09Step::End(end) => return Ok(end - start),
        }
    }
}
