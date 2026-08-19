#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod encode;

#[cfg(test)]
mod tests;

pub const HEADER_LEN: usize = 48;

pub const REGION_ROW_LEN: usize = 32;

pub const PROVINCE_ROW_LEN: usize = 32;

/// half). Every header offset is generated from [`HeaderWord::offset`] by an
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
    MaxWaterRaw,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Header,
    Body,
    RegionRow,
    ProvinceRow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViolationKind {
    Need {
        section: Section,
        needed: usize,
        available: usize,
    },
    LengthMismatch {
        expected: usize,
        available: usize,
    },
    LengthOverflow {
        section: Section,
        count: usize,
        unit: usize,
    },
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Header {
    bytes: [u8; HEADER_LEN],
}

impl Header {
    pub fn elapsed_wall_ms(&self) -> u64 {
        u64_at(&self.bytes, 0)
    }

    pub fn word(&self, which: HeaderWord) -> u32 {
        u32_at(&self.bytes, which.offset())
    }

    pub fn count(&self, which: HeaderWord) -> usize {
        self.word(which) as usize
    }

    pub fn words(&self) -> [u32; HeaderWord::ALL.len()] {
        let mut out = [0u32; HeaderWord::ALL.len()];
        let mut i = 0;
        while i < HeaderWord::ALL.len() {
            out[i] = u32_at(&self.bytes, HeaderWord::ALL[i].offset());
            i += 1;
        }
        out
    }

    pub fn bytes(&self) -> &[u8; HEADER_LEN] {
        &self.bytes
    }
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BodyLayout {
    pub region_count: usize,
    pub province_count: usize,
    pub regions_at: usize,
    pub provinces_at: usize,
    pub end: usize,
}

pub fn body_layout(payload: &[u8], header: &Header) -> Result<BodyLayout, Violation> {
    let region_count = header.count(HeaderWord::RegionCount);
    let province_count = header.count(HeaderWord::ProvinceCount);
    let region_bytes = region_count
        .checked_mul(REGION_ROW_LEN)
        .ok_or(Violation::new(
            HEADER_LEN,
            ViolationKind::LengthOverflow {
                section: Section::RegionRow,
                count: region_count,
                unit: REGION_ROW_LEN,
            },
        ))?;
    let province_bytes = province_count
        .checked_mul(PROVINCE_ROW_LEN)
        .ok_or(Violation::new(
            HEADER_LEN,
            ViolationKind::LengthOverflow {
                section: Section::ProvinceRow,
                count: province_count,
                unit: PROVINCE_ROW_LEN,
            },
        ))?;
    let provinces_at = HEADER_LEN.checked_add(region_bytes).ok_or(Violation::new(
        HEADER_LEN,
        ViolationKind::LengthOverflow {
            section: Section::Body,
            count: region_count,
            unit: REGION_ROW_LEN,
        },
    ))?;
    let end = provinces_at
        .checked_add(province_bytes)
        .ok_or(Violation::new(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionRowView<'a> {
    bytes: &'a [u8],
}

impl<'a> RegionRowView<'a> {
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

    pub fn u32(&self, field: RegionField) -> u32 {
        u32_at(self.bytes, field.offset())
    }

    pub fn i32(&self, field: RegionField) -> i32 {
        self.u32(field) as i32
    }

    pub fn words(&self) -> [u32; RegionField::ALL.len()] {
        let mut out = [0u32; RegionField::ALL.len()];
        let mut i = 0;
        while i < RegionField::ALL.len() {
            out[i] = u32_at(self.bytes, RegionField::ALL[i].offset());
            i += 1;
        }
        out
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProvinceRowView<'a> {
    bytes: &'a [u8],
}

impl<'a> ProvinceRowView<'a> {
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

    pub fn words(&self) -> [u32; ProvinceField::ALL.len()] {
        let mut out = [0u32; ProvinceField::ALL.len()];
        let mut i = 0;
        while i < ProvinceField::ALL.len() {
            out[i] = u32_at(self.bytes, ProvinceField::ALL[i].offset());
            i += 1;
        }
        out
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RowStep<V> {
    Row { index: usize, view: V },
    End(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct RegionCursor {
    at: usize,
    next_index: usize,
    count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct ProvinceCursor {
    at: usize,
    next_index: usize,
    count: usize,
}

macro_rules! row_cursor {
    ($cursor:ty, $view:ident, $row_len:expr, $section:expr) => {
        impl $cursor {
            pub const fn offset(&self) -> usize {
                self.at
            }

            pub fn next<'a>(&mut self, payload: &'a [u8]) -> Result<RowStep<$view<'a>>, Violation> {
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

row_cursor!(
    RegionCursor,
    RegionRowView,
    REGION_ROW_LEN,
    Section::RegionRow
);
row_cursor!(
    ProvinceCursor,
    ProvinceRowView,
    PROVINCE_ROW_LEN,
    Section::ProvinceRow
);

impl RegionCursor {
    pub const fn new(layout: &BodyLayout) -> Self {
        RegionCursor {
            at: layout.regions_at,
            next_index: 0,
            count: layout.region_count,
        }
    }
}

impl ProvinceCursor {
    pub const fn new(layout: &BodyLayout) -> Self {
        ProvinceCursor {
            at: layout.provinces_at,
            next_index: 0,
            count: layout.province_count,
        }
    }
}

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
