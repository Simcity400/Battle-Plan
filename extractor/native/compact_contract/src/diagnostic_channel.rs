//! Every discriminant here is a **stable ABI**: retained telemetry must stay

use core::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UnitRole {
    Deciding = 1,
    Locator = 2,
    Context = 3,
    Counter = 4,
    Undeclared = 5,
}

impl UnitRole {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Deciding),
            2 => Some(Self::Locator),
            3 => Some(Self::Context),
            4 => Some(Self::Counter),
            5 => Some(Self::Undeclared),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Deciding => "deciding",
            Self::Locator => "locator",
            Self::Context => "context",
            Self::Counter => "counter",
            Self::Undeclared => "undeclared",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UnitPresence {
    Observed = 1,
    Absent = 2,
    ResetSentinel = 3,
    NotApplicable = 4,
    Undeclared = 5,
}

impl UnitPresence {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Observed),
            2 => Some(Self::Absent),
            3 => Some(Self::ResetSentinel),
            4 => Some(Self::NotApplicable),
            5 => Some(Self::Undeclared),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Absent => "absent",
            Self::ResetSentinel => "sentinel",
            Self::NotApplicable => "n/a",
            Self::Undeclared => "undeclared",
        }
    }

    pub const fn value_is_a_measurement(self) -> bool {
        matches!(self, Self::Observed)
    }
}

pub const MAX_UNIT_NAME_LEN: usize = 96;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagUnit {
    pub name: String,
    pub value: u64,
    pub role: UnitRole,
    pub presence: UnitPresence,
}

impl DiagUnit {
    pub fn new(
        name: impl Into<String>,
        value: u64,
        role: UnitRole,
        presence: UnitPresence,
    ) -> Self {
        Self {
            name: name.into(),
            value,
            role,
            presence,
        }
    }

    pub fn deciding(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Deciding, UnitPresence::Observed)
    }

    pub fn locator(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Locator, UnitPresence::Observed)
    }

    pub fn context(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Context, UnitPresence::Observed)
    }

    pub fn counter(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Counter, UnitPresence::Observed)
    }

    pub fn encoded_len(&self) -> usize {
        UNIT_FIXED_LEN + self.name.len()
    }

    pub fn render(&self) -> String {
        match self.presence {
            UnitPresence::Observed => format!("{}={}", self.name, self.value),
            UnitPresence::Absent => format!("{}=absent", self.name),
            UnitPresence::ResetSentinel => format!("{}=sentinel(0x{:x})", self.name, self.value),
            UnitPresence::NotApplicable => format!("{}=n/a", self.name),
            UnitPresence::Undeclared => format!("{}={}", self.name, self.value),
        }
    }

    pub fn undeclared(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Undeclared, UnitPresence::Undeclared)
    }
}

pub const UNIT_FIXED_LEN: usize = 12;

pub const MAX_UNIT_RENDER_LEN: usize = MAX_UNIT_NAME_LEN + 30;

/// bound for the first unit; `debug_assert` states that, and the release path
pub fn render_units_bounded(units: &[DiagUnit], max_len: usize) -> (String, u32) {
    let mut text = String::new();
    let mut named = 0u32;
    for unit in units {
        let rendered = unit.render();
        let separator = usize::from(!text.is_empty());
        if text.len() + separator + rendered.len() > max_len {
            break;
        }
        if separator == 1 {
            text.push(',');
        }
        text.push_str(&rendered);
        named += 1;
    }
    debug_assert!(
        units.is_empty() || named > 0,
        "caller gave render_units_bounded {max_len} bytes, too little for one unit"
    );
    (text, named)
}

pub fn encode_units(units: &[DiagUnit]) -> Result<Vec<u8>, String> {
    if units.len() > u16::MAX as usize {
        return Err(format!("{} diagnostic units exceeds u16", units.len()));
    }
    let mut out = Vec::with_capacity(2 + units.iter().map(DiagUnit::encoded_len).sum::<usize>());
    out.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for unit in units {
        if unit.name.is_empty() {
            return Err("diagnostic unit has an empty name".to_string());
        }
        if unit.name.len() > MAX_UNIT_NAME_LEN {
            return Err(format!(
                "diagnostic unit name {:?} is {} bytes, above the {MAX_UNIT_NAME_LEN} bound",
                unit.name,
                unit.name.len()
            ));
        }
        out.extend_from_slice(&(unit.name.len() as u16).to_le_bytes());
        out.push(unit.role.code());
        out.push(unit.presence.code());
        out.extend_from_slice(&unit.value.to_le_bytes());
        out.extend_from_slice(unit.name.as_bytes());
    }
    Ok(out)
}

pub fn decode_units(bytes: &[u8]) -> Result<(Vec<DiagUnit>, usize), String> {
    if bytes.len() < 2 {
        return Err("diagnostic unit list is shorter than its count field".to_string());
    }
    let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    let mut units = Vec::with_capacity(count.min(1024));
    let mut offset = 2usize;
    for index in 0..count {
        if offset
            .checked_add(UNIT_FIXED_LEN)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(format!("diagnostic unit {index} is truncated"));
        }
        let name_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        let role = UnitRole::from_code(bytes[offset + 2]).ok_or_else(|| {
            format!(
                "diagnostic unit {index} has role code {}",
                bytes[offset + 2]
            )
        })?;
        let presence = UnitPresence::from_code(bytes[offset + 3]).ok_or_else(|| {
            format!(
                "diagnostic unit {index} has presence code {}",
                bytes[offset + 3]
            )
        })?;
        let value = u64::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]);
        if name_len == 0 {
            return Err(format!("diagnostic unit {index} has an empty name"));
        }
        if name_len > MAX_UNIT_NAME_LEN {
            return Err(format!(
                "diagnostic unit {index} name is {name_len} bytes, above the \
                 {MAX_UNIT_NAME_LEN} bound"
            ));
        }
        let start = offset + UNIT_FIXED_LEN;
        let Some(end) = start.checked_add(name_len) else {
            return Err(format!("diagnostic unit {index} name length overflows"));
        };
        if end > bytes.len() {
            return Err(format!("diagnostic unit {index} name runs past the blob"));
        }
        let name = core::str::from_utf8(&bytes[start..end])
            .map_err(|error| format!("diagnostic unit {index} name is not UTF-8: {error}"))?
            .to_string();
        units.push(DiagUnit {
            name,
            value,
            role,
            presence,
        });
        offset = end;
    }
    Ok((units, offset))
}

pub const MAX_LOCATOR_LABEL_LEN: usize = 32;
pub const MAX_LOCATOR_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorLevel {
    pub label: String,
    pub ordinal: u64,
    pub extent: Option<u64>,
    pub address: Option<u64>,
}

impl LocatorLevel {
    pub fn new(label: impl Into<String>, ordinal: u64, extent: Option<u64>) -> Self {
        Self {
            label: label.into(),
            ordinal,
            extent,
            address: None,
        }
    }

    pub fn with_address(mut self, address: u64) -> Self {
        self.address = Some(address);
        self
    }

    pub fn render(&self) -> String {
        let mut text = match self.extent {
            Some(extent) => format!("{} {} of {extent}", self.label, self.ordinal),
            None => format!("{} {} of ?", self.label, self.ordinal),
        };
        if let Some(address) = self.address {
            text.push_str(&format!("@0x{address:x}"));
        }
        text
    }
}

pub const LOCATOR_LEVEL_FIXED_LEN: usize = 28;
const LOCATOR_FLAG_EXTENT_KNOWN: u16 = 1 << 0;
const LOCATOR_FLAG_ADDRESS_KNOWN: u16 = 1 << 1;

pub fn encode_locator(levels: &[LocatorLevel]) -> Result<Vec<u8>, String> {
    if levels.len() > MAX_LOCATOR_DEPTH {
        return Err(format!(
            "locator path is {} deep, above the {MAX_LOCATOR_DEPTH} bound",
            levels.len()
        ));
    }
    let mut out = Vec::with_capacity(2 + levels.len() * (LOCATOR_LEVEL_FIXED_LEN + 16));
    out.extend_from_slice(&(levels.len() as u16).to_le_bytes());
    for level in levels {
        if level.label.is_empty() {
            return Err("locator level has an empty label".to_string());
        }
        if level.label.len() > MAX_LOCATOR_LABEL_LEN {
            return Err(format!(
                "locator label {:?} is {} bytes, above the {MAX_LOCATOR_LABEL_LEN} bound",
                level.label,
                level.label.len()
            ));
        }
        let mut flags = 0u16;
        if level.extent.is_some() {
            flags |= LOCATOR_FLAG_EXTENT_KNOWN;
        }
        if level.address.is_some() {
            flags |= LOCATOR_FLAG_ADDRESS_KNOWN;
        }
        out.extend_from_slice(&(level.label.len() as u16).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&level.ordinal.to_le_bytes());
        out.extend_from_slice(&level.extent.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&level.address.unwrap_or(0).to_le_bytes());
        out.extend_from_slice(level.label.as_bytes());
    }
    Ok(out)
}

pub fn decode_locator(bytes: &[u8]) -> Result<(Vec<LocatorLevel>, usize), String> {
    if bytes.len() < 2 {
        return Err("locator path is shorter than its count field".to_string());
    }
    let count = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
    if count > MAX_LOCATOR_DEPTH {
        return Err(format!(
            "locator path is {count} deep, above the {MAX_LOCATOR_DEPTH} bound"
        ));
    }
    let mut levels = Vec::with_capacity(count);
    let mut offset = 2usize;
    for index in 0..count {
        if offset
            .checked_add(LOCATOR_LEVEL_FIXED_LEN)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(format!("locator level {index} is truncated"));
        }
        let label_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        let flags = u16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]);
        let read_u64 = |at: usize| -> u64 {
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
        };
        let ordinal = read_u64(offset + 4);
        let extent_raw = read_u64(offset + 12);
        let address_raw = read_u64(offset + 20);
        if label_len == 0 {
            return Err(format!("locator level {index} has an empty label"));
        }
        if label_len > MAX_LOCATOR_LABEL_LEN {
            return Err(format!(
                "locator level {index} label is {label_len} bytes, above the \
                 {MAX_LOCATOR_LABEL_LEN} bound"
            ));
        }
        if flags & !(LOCATOR_FLAG_EXTENT_KNOWN | LOCATOR_FLAG_ADDRESS_KNOWN) != 0 {
            return Err(format!(
                "locator level {index} has unknown flags {flags:#06x}"
            ));
        }
        let start = offset + LOCATOR_LEVEL_FIXED_LEN;
        let Some(end) = start.checked_add(label_len) else {
            return Err(format!("locator level {index} label length overflows"));
        };
        if end > bytes.len() {
            return Err(format!("locator level {index} label runs past the blob"));
        }
        let label = core::str::from_utf8(&bytes[start..end])
            .map_err(|error| format!("locator level {index} label is not UTF-8: {error}"))?
            .to_string();
        levels.push(LocatorLevel {
            label,
            ordinal,
            extent: (flags & LOCATOR_FLAG_EXTENT_KNOWN != 0).then_some(extent_raw),
            address: (flags & LOCATOR_FLAG_ADDRESS_KNOWN != 0).then_some(address_raw),
        });
        offset = end;
    }
    Ok((levels, offset))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    SelfDomain,
    OtherDomain(u16),
    Engine { rva: u64 },
    Host,
    OffFrameWorker,
    Undeclared,
}

pub const AUTHORITY_LEN: usize = 12;

impl Authority {
    pub const fn kind_code(self) -> u16 {
        match self {
            Self::SelfDomain => 1,
            Self::OtherDomain(_) => 2,
            Self::Engine { .. } => 3,
            Self::Host => 4,
            Self::OffFrameWorker => 5,
            Self::Undeclared => 0,
        }
    }

    pub fn encode(self) -> [u8; AUTHORITY_LEN] {
        let mut out = [0u8; AUTHORITY_LEN];
        out[0..2].copy_from_slice(&self.kind_code().to_le_bytes());
        let domain = match self {
            Self::OtherDomain(id) => id,
            _ => 0,
        };
        out[2..4].copy_from_slice(&domain.to_le_bytes());
        let rva = match self {
            Self::Engine { rva } => rva,
            _ => 0,
        };
        out[4..12].copy_from_slice(&rva.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < AUTHORITY_LEN {
            return Err("authority is truncated".to_string());
        }
        let kind = u16::from_le_bytes([bytes[0], bytes[1]]);
        let domain = u16::from_le_bytes([bytes[2], bytes[3]]);
        let rva = u64::from_le_bytes([
            bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
        ]);
        match kind {
            1 => Ok(Self::SelfDomain),
            2 => Ok(Self::OtherDomain(domain)),
            3 => Ok(Self::Engine { rva }),
            4 => Ok(Self::Host),
            5 => Ok(Self::OffFrameWorker),
            other => Err(format!("authority kind {other} is not defined")),
        }
    }

    pub fn render(self) -> String {
        match self {
            Self::SelfDomain => "authority=self".to_string(),
            Self::OtherDomain(id) => format!("authority=domain{id}"),
            Self::Engine { rva } => format!("authority=engine@rva=0x{rva:x}"),
            Self::Host => "authority=host".to_string(),
            Self::OffFrameWorker => "authority=off_frame_worker".to_string(),
            Self::Undeclared => "authority=undeclared".to_string(),
        }
    }
}

pub const MAX_PREDICATE_NAME_LEN: usize = 64;

pub const PREDICATE_UNDECLARED: u32 = 0xFFFF_FF01;
pub const PREDICATE_UNDECLARED_NAME: &str = "producer_declared_no_predicate";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Predicate {
    pub code: NonZeroU32,
    pub name: String,
}

impl Predicate {
    pub fn new(code: u32, name: impl Into<String>) -> Result<Self, String> {
        let code = NonZeroU32::new(code)
            .ok_or_else(|| "predicate code 0 is reserved for 'not declared'".to_string())?;
        Ok(Self {
            code,
            name: name.into(),
        })
    }

    pub fn undeclared() -> Self {
        Self {
            code: NonZeroU32::new(PREDICATE_UNDECLARED).expect("reserved code is non-zero"),
            name: PREDICATE_UNDECLARED_NAME.to_string(),
        }
    }

    pub fn is_undeclared(&self) -> bool {
        self.code.get() == PREDICATE_UNDECLARED
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OffFrameTerminal {
    Running = 1,
    CompletedOk = 2,
    CompletedError = 3,
    Panicked = 4,
}

impl OffFrameTerminal {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Running),
            2 => Some(Self::CompletedOk),
            3 => Some(Self::CompletedError),
            4 => Some(Self::Panicked),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::CompletedOk => "completed_ok",
            Self::CompletedError => "completed_error",
            Self::Panicked => "panicked",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffFrameOutcome {
    pub spawn_generation: u64,
    pub elapsed_us: u64,
    pub deadline_us: u64,
    pub terminal: OffFrameTerminal,
}

pub const OFF_FRAME_LEN: usize = 28;

impl OffFrameOutcome {
    pub fn encode(&self) -> [u8; OFF_FRAME_LEN] {
        let mut out = [0u8; OFF_FRAME_LEN];
        out[0..8].copy_from_slice(&self.spawn_generation.to_le_bytes());
        out[8..16].copy_from_slice(&self.elapsed_us.to_le_bytes());
        out[16..24].copy_from_slice(&self.deadline_us.to_le_bytes());
        out[24] = self.terminal.code();
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < OFF_FRAME_LEN {
            return Err("off-frame outcome is truncated".to_string());
        }
        let read = |at: usize| -> u64 {
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
        };
        let terminal = OffFrameTerminal::from_code(bytes[24])
            .ok_or_else(|| format!("off-frame terminal code {} is not defined", bytes[24]))?;
        Ok(Self {
            spawn_generation: read(0),
            elapsed_us: read(8),
            deadline_us: read(16),
            terminal,
        })
    }

    pub fn overdue(&self) -> bool {
        self.deadline_us != 0
            && self.terminal == OffFrameTerminal::Running
            && self.elapsed_us > self.deadline_us
    }

    pub fn render(&self) -> String {
        let deadline = if self.deadline_us == 0 {
            "no_deadline".to_string()
        } else {
            format!("deadline={}us", self.deadline_us)
        };
        format!(
            "off_frame={} gen={} elapsed={}us {deadline}{}",
            self.terminal.label(),
            self.spawn_generation,
            self.elapsed_us,
            if self.overdue() { " OVERDUE" } else { "" }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TruncationKind {
    None = 1,
    BudgetShare = 2,
    UnitTooLarge = 3,
    ProducerOverflow = 4,
    Undeclared = 0,
}

impl TruncationKind {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::None),
            2 => Some(Self::BudgetShare),
            3 => Some(Self::UnitTooLarge),
            4 => Some(Self::ProducerOverflow),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "complete",
            Self::BudgetShare => "budget_share",
            Self::UnitTooLarge => "unit_too_large",
            Self::ProducerOverflow => "producer_overflow",
            Self::Undeclared => "undeclared",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TruncationAccount {
    pub kind: TruncationKind,
    pub units_offered: u32,
    pub units_emitted: u32,
    pub bytes_dropped: u32,
}

impl TruncationAccount {
    pub const fn complete(units: u32) -> Self {
        Self {
            kind: TruncationKind::None,
            units_offered: units,
            units_emitted: units,
            bytes_dropped: 0,
        }
    }

    pub const fn units_dropped(&self) -> u32 {
        self.units_offered.saturating_sub(self.units_emitted)
    }

    pub fn render(&self) -> String {
        match self.kind {
            TruncationKind::None => format!(
                "units={}/{} complete",
                self.units_emitted, self.units_offered
            ),
            TruncationKind::Undeclared => "units=undeclared".to_string(),
            other => format!(
                "units={}/{} dropped={} bytes_dropped={} reason={}",
                self.units_emitted,
                self.units_offered,
                self.units_dropped(),
                self.bytes_dropped,
                other.label()
            ),
        }
    }
}

pub const OFFENDER_FLOOR_BYTES: usize = 384;
pub const MAX_FLOOR_UNITS_PER_OFFENDER: usize = 8;

#[derive(Clone, Debug)]
pub struct BudgetRequest {
    pub domain_id: u16,
    pub units: Vec<DiagUnit>,
}

#[derive(Clone, Debug)]
pub struct BudgetGrant {
    pub domain_id: u16,
    pub units: Vec<DiagUnit>,
    pub account: TruncationAccount,
}

pub fn allocate_unit_budget(
    requests: &[BudgetRequest],
    discretionary_pool: usize,
    max_bytes_per_offender: usize,
) -> Vec<BudgetGrant> {
    let mut ordered: Vec<(usize, Vec<(usize, DiagUnit)>)> = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let mut units: Vec<(usize, DiagUnit)> = request
            .units
            .iter()
            .cloned()
            .enumerate()
            .map(|(order, unit)| (order, unit))
            .collect();
        units.sort_by_key(|(order, unit)| (unit.role, *order));
        ordered.push((index, units));
    }

    let mut kept: Vec<Vec<DiagUnit>> = vec![Vec::new(); requests.len()];
    let mut pending: Vec<Vec<DiagUnit>> = vec![Vec::new(); requests.len()];
    let mut too_large = vec![false; requests.len()];
    let mut floor_bytes = vec![0usize; requests.len()];

    let floor_ceiling = OFFENDER_FLOOR_BYTES.min(max_bytes_per_offender);
    for (index, units) in &ordered {
        let mut floor_used = 0usize;
        let mut floor_units = 0usize;
        for (_, unit) in units {
            let size = unit.encoded_len();
            let floor_eligible =
                floor_units < MAX_FLOOR_UNITS_PER_OFFENDER && floor_used + size <= floor_ceiling;
            if floor_eligible {
                floor_used += size;
                floor_units += 1;
                kept[*index].push(unit.clone());
            } else {
                pending[*index].push(unit.clone());
            }
        }
        floor_bytes[*index] = floor_used;
    }

    let demands: Vec<usize> = pending
        .iter()
        .map(|units| units.iter().map(DiagUnit::encoded_len).sum::<usize>())
        .collect();
    let shares = water_fill(&demands, discretionary_pool, &order_by_domain_id(requests));

    for (index, units) in pending.iter().enumerate() {
        let mut budget =
            shares[index].min(max_bytes_per_offender.saturating_sub(floor_bytes[index]));
        for unit in units {
            let size = unit.encoded_len();
            if size <= budget {
                budget -= size;
                kept[index].push(unit.clone());
            } else if size > floor_ceiling && size > shares[index] {
                too_large[index] = true;
            }
        }
    }

    requests
        .iter()
        .enumerate()
        .map(|(index, request)| {
            let offered = request.units.len();
            let emitted = kept[index].len();
            let bytes_offered: usize = request.units.iter().map(DiagUnit::encoded_len).sum();
            let bytes_emitted: usize = kept[index].iter().map(DiagUnit::encoded_len).sum();
            let kind = if emitted == offered {
                TruncationKind::None
            } else if too_large[index] {
                TruncationKind::UnitTooLarge
            } else {
                TruncationKind::BudgetShare
            };
            let mut units = kept[index].clone();
            units.sort_by_key(|unit| unit.role);
            BudgetGrant {
                domain_id: request.domain_id,
                units,
                account: TruncationAccount {
                    kind,
                    units_offered: offered.min(u32::MAX as usize) as u32,
                    units_emitted: emitted.min(u32::MAX as usize) as u32,
                    bytes_dropped: bytes_offered
                        .saturating_sub(bytes_emitted)
                        .min(u32::MAX as usize) as u32,
                },
            }
        })
        .collect()
}

fn order_by_domain_id(requests: &[BudgetRequest]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..requests.len()).collect();
    order.sort_by_key(|index| (requests[*index].domain_id, *index));
    order
}

fn water_fill(demands: &[usize], pool: usize, visit_order: &[usize]) -> Vec<usize> {
    let mut shares = vec![0usize; demands.len()];
    let mut satisfied = vec![false; demands.len()];
    let mut remaining = pool;
    loop {
        let claimants: Vec<usize> = visit_order
            .iter()
            .copied()
            .filter(|index| !satisfied[*index] && demands[*index] > 0)
            .collect();
        if claimants.is_empty() || remaining == 0 {
            break;
        }
        let equal = remaining / claimants.len();
        if equal == 0 {
            for index in claimants.into_iter().take(remaining) {
                shares[index] += 1;
            }
            break;
        }
        let mut progressed = false;
        for index in claimants {
            let want = demands[index] - shares[index];
            let give = want.min(equal);
            shares[index] += give;
            remaining -= give;
            if give > 0 {
                progressed = true;
            }
            if shares[index] == demands[index] {
                satisfied[index] = true;
            }
        }
        if !progressed {
            break;
        }
    }
    shares
}

#[derive(Debug, Default)]
pub struct FirstFailureLatch<T> {
    first: Option<T>,
    later: u64,
}

impl<T> FirstFailureLatch<T> {
    pub const fn new() -> Self {
        Self {
            first: None,
            later: 0,
        }
    }

    pub fn offer(&mut self, failure: T) -> bool {
        if self.first.is_none() {
            self.first = Some(failure);
            true
        } else {
            self.later = self.later.saturating_add(1);
            false
        }
    }

    pub fn first(&self) -> Option<&T> {
        self.first.as_ref()
    }

    pub const fn later_failures(&self) -> u64 {
        self.later
    }

    pub const fn total(&self) -> u64 {
        match self.first {
            Some(_) => self.later + 1,
            None => 0,
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.first.is_none()
    }

    pub fn reset(&mut self) {
        self.first = None;
        self.later = 0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Accumulation {
    PerFrame = 1,
    Cumulative = 2,
    Gauge = 3,
    LastValue = 4,
    BitwiseOr = 5,
    Undeclared = 6,
}

impl Accumulation {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::PerFrame),
            2 => Some(Self::Cumulative),
            3 => Some(Self::Gauge),
            4 => Some(Self::LastValue),
            5 => Some(Self::BitwiseOr),
            6 => Some(Self::Undeclared),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::PerFrame => "per_frame",
            Self::Cumulative => "cumulative",
            Self::Gauge => "gauge",
            Self::LastValue => "last_value",
            Self::BitwiseOr => "bitwise_or",
            Self::Undeclared => "undeclared",
        }
    }

    pub const ALL: [Self; 6] = [
        Self::PerFrame,
        Self::Cumulative,
        Self::Gauge,
        Self::LastValue,
        Self::BitwiseOr,
        Self::Undeclared,
    ];

    pub fn fold(self, running: Option<u64>, incoming: u64) -> u64 {
        let Some(running) = running else {
            return incoming;
        };
        match self {
            Self::PerFrame | Self::Undeclared => running.saturating_add(incoming),
            Self::Cumulative | Self::Gauge => running.max(incoming),
            Self::LastValue => incoming,
            Self::BitwiseOr => running | incoming,
        }
    }

    pub const fn requires_every_frame(self) -> bool {
        matches!(self, Self::PerFrame | Self::Undeclared)
    }
}

pub const STATEFUL_UNIT_PREFIXES: &[&str] = &[
    "rs_",
    "convoy_transport_attack_",
    // history record; every channel is an engine lifetime total.
    "air_loss_",
    "fired_event_popups_",
    "d75_",
    "heap_sweep_",
    "sunk_ship_",
    "d85_",
    "d97_",
    "d95_",
];

pub const RAW_WORD_SUFFIXES: &[&str] = &[
    "_sample",
    "_hash",
    "_fnv1a64",
    "_fingerprint",
    "_ptr",
    "_rva",
    "_address",
    "_vtable",
    "_id",
    "_index",
    "_ordinal",
    "_fail_width",
    "_stage",
    "_branch",
    "_reason",
    "_kind",
    "_code",
];

pub const BITMASK_SUFFIXES: &[&str] = &["_mask", "_status_or"];

pub const CUMULATIVE_UNITS: &[&str] = &[
    "rs_observations",
    "rs_scheduled_hours",
    "rs_accepted_hours",
    "rs_degraded_hours",
    "rs_refused_hours",
    "rs_evicted_pending_hours",
    "rs_abandoned_hours",
    "rs_flushed_pending_hours",
    "rs_unobserved_overrun_hours",
    "rs_unobserved_clock_unknown_hours",
    "rs_counter_anomaly_events",
    "rs_untrusted_clock_recovery_events",
    "rs_untrusted_clock_recoveries_dropped",
    "rs_post_terminal_observations",
    "rs_gapped_domain_hours",
    "rs_overrun_domain_hours",
    "rs_backpressure_domain_hours",
    "rs_torn_domain_hours",
    "rs_worker_panic_domain_hours",
    "rs_clock_unreadable_events",
    "rs_clock_unreadable_intervals_dropped",
    "rs_control_lane_compactions",
    "rs_reloads",
    "rs_max_request_depth",
    "rs_untrusted_clock_recovery_max_delta_hours",
    "rs_max_control_depth",
    "rs_max_publishing_depth",
    "rs_request_cap",
    "rs_control_cap",
    "rs_zero_mutation_policy",
    "rs_scheduler_active",
    "rs_shadow_active",
    "rs_shadow_events",
    "rs_shadow_hour_edges",
    "rs_shadow_dispatches",
    "rs_shadow_committed_hours",
    "rs_shadow_refused_hours",
    "rs_shadow_processing_backpressure_hours",
    "rs_shadow_observation_deadline_miss_hours",
    "rs_shadow_invalidated_hours",
    "rs_shadow_live_kept_hours_shadow_dropped",
    "rs_shadow_collapsed_publications",
    "rs_shadow_illegal_transitions",
    "rs_shadow_hour_mismatches",
    "rs_shadow_agreed_evictions",
    "rs_shadow_disagreed_evictions",
    "rs_shadow_drop_memory_overflows",
    "rs_shadow_late_dispatch_samples",
    "rs_shadow_late_dispatch_total_us",
    "rs_shadow_cost_total_ns",
    "rs_shadow_max_late_dispatch_us",
    "rs_shadow_max_cost_ns",
    "rs_terminal",
    "rs_transport_failed",
    "rs_finalized",
    "rs_sink_write_failures",
    "rs_sink_open_attempts",
    "rs_sink_open_finishes",
    "rs_sink_requested_count",
    "rs_sink_opened_count",
    "rs_sink_open_failures",
    "rs_sink_write_attempts",
    "rs_sink_write_finishes",
    "rs_sink_write_commits",
    "rs_sink_file_bytes_committed",
    "rs_sink_pipe_bytes_committed",
    "rs_sink_time_sample_count",
    "rs_sink_time_total_us",
    "rs_sink_time_max_us",
    "rs_configured_sink_count",
    "rs_pipe_sink_enabled",
    "rs_file_sink_enabled",
    "rs_pipe_sink_requested",
    "rs_pipe_sink_opened",
    "rs_file_sink_requested",
    "rs_file_sink_opened",
    "convoy_transport_attack_callbacks_seen",
    "convoy_transport_attack_original_calls",
    "convoy_transport_attack_events_emitted",
    "convoy_transport_attack_dropped_events",
    "convoy_transport_attack_dropped_no_original",
    "convoy_transport_attack_dropped_teardown",
    "convoy_transport_attack_dropped_nested",
    "convoy_transport_attack_dropped_capture_busy",
    "convoy_transport_attack_dropped_hour_or_epoch",
    "convoy_transport_attack_dropped_resolver",
    "convoy_transport_attack_dropped_candidate",
    "convoy_transport_attack_dropped_d70_pre",
    "convoy_transport_attack_dropped_d70_post",
    "convoy_transport_attack_dropped_d70_mutation",
    "convoy_transport_attack_dropped_queue_busy",
    "convoy_transport_attack_dropped_queue_full",
    "convoy_transport_attack_dropped_pre_hour_unreadable",
    "convoy_transport_attack_dropped_receipt_hour_mismatch",
    "convoy_transport_attack_dropped_post_hour_unreadable",
    "convoy_transport_attack_dropped_post_hour_changed",
    "convoy_transport_attack_dropped_resolver_snapshot_absent",
    "convoy_transport_attack_dropped_resolver_unpublished",
    "convoy_transport_attack_receipt_hour_lag_max",
    "convoy_transport_attack_receipt_hour_lead_max",
    "convoy_transport_attack_dropped_receipt_hour_ahead",
    "convoy_transport_attack_dropped_receipt_lag_beyond",
    "convoy_transport_attack_dropped_pre_hour_address_overflow",
    "convoy_transport_attack_dropped_candidate_initial_gate",
    "convoy_transport_attack_dropped_candidate_producer_sequence_exhausted",
    "convoy_transport_attack_dropped_candidate_capture_error",
    "convoy_transport_attack_dropped_candidate_context_mismatch",
    "convoy_transport_attack_dropped_candidate_context_address_overflow",
    "convoy_transport_attack_dropped_candidate_member_bytes_unreadable",
    // The `CaptureError` leaves of `..._dropped_candidate_capture_error`,
    // lifetime counters like the aggregate they partition.
    "convoy_transport_attack_dropped_capture_error_null_call_register",
    "convoy_transport_attack_dropped_capture_error_call_abi_mismatch",
    "convoy_transport_attack_dropped_capture_error_address_overflow",
    "convoy_transport_attack_dropped_capture_error_member_unreadable",
    "convoy_transport_attack_dropped_capture_error_member_vtable_mismatch",
    "convoy_transport_attack_dropped_capture_error_invalid_handle",
    "convoy_transport_attack_dropped_capture_error_invalid_owner_tag",
    "convoy_transport_attack_dropped_capture_error_invalid_killer_tag",
    "convoy_transport_attack_dropped_capture_error_invalid_producer_sequence",
    "convoy_transport_attack_dropped_capture_error_invalid_combat_epoch",
    "convoy_transport_attack_dropped_capture_error_invalid_endpoint",
    "convoy_transport_attack_dropped_capture_error_combat_join",
    "convoy_transport_attack_dropped_capture_error_combat_slot_mismatch",
    "convoy_transport_attack_dropped_capture_error_route_join",
    "convoy_transport_attack_dropped_capture_error_route_key_mismatch",
    "fired_event_popups_rows",
    "fired_event_popups_unadmitted_rows",
    "fired_event_popups_dropped_rows",
    "d75_dropped_fire_rows",
    "d75_dropped_resolution_rows",
    "d75_correlation_failures",
    "d75_invalid_definition_joins",
    "d75_fired_ledger_shadow_mismatches",
    "d75_queue_high_water_mark",
    "d75_queue_capacity",
    "d75_runtime_installed",
    "d75_fire_hook_validated",
    "d75_option_hook_validated",
    "d75_materialisation_hook_validated",
    "d75_timeout_hook_validated",
    "d75_all_event_classes_covered",
    "d75_all_resolution_routes_covered",
    "d75_capture_start_baseline_complete",
    "d75_terminal_custody",
    "sunk_ship_writer_events",
    "heap_sweep_degraded",
    "heap_sweep_degraded_total",
    "heap_sweep_spawn_failures",
    "heap_sweep_generation",
    "domain53_sweep_stale_frames",
    "domain53_sweep_generation",
    "domain53_war_validated_generation",
    "capwar60_sweep_stale_frames",
    "h_convoy_resolver_publication_sequence",
    "d85_callback_count",
    "d85_variant_upgrade_selection_count",
    // Same shape: an observer-lifetime total of division-order registrations,
    "d85_division_view_registrations",
    "d85_hook_installed",
    "d97_refresh_observed_total",
    "d97_hook_installed",
    // PROCESS-LIFETIME total read out of `LatestFrames::census` or the
    "d95_hooks_installed",
    "d95_site_entries_intel_getter",
    "d95_site_entries_land_frame",
    "d95_site_entries_land_eligibility",
    "d95_site_entries_taskforce_frame",
    "d95_land_candidates_seen",
    "d95_land_rejected_unreadable_handle",
    "d95_land_rejected_below_intel_threshold",
    "d95_land_rejected_by_eligibility",
    "d95_land_rejected_taskforce_state",
    "d95_land_abandoned_pending",
    "d95_land_admitted",
    "d95_land_frames_without_enumeration",
    "d95_land_frames_producer_gate_closed",
    "d95_taskforce_candidates_seen",
    "d95_taskforce_rejected_unreadable_handle",
    "d95_taskforce_rejected_below_intel_threshold",
    "d95_taskforce_rejected_by_eligibility",
    "d95_taskforce_rejected_taskforce_state",
    "d95_taskforce_abandoned_pending",
    "d95_taskforce_admitted",
    "d95_taskforce_frames_without_enumeration",
    "d95_taskforce_frames_producer_gate_closed",
    "d95_taskforce_frames_source_collection_empty",
    // Lifetime totals of rows this lane admitted into a frame that was then
    "d95_land_admitted_rows_lost_to_replacement",
    "d95_taskforce_admitted_rows_lost_to_replacement",
];

/// each country's ENGINE LIFETIME total for that channel, so it is cumulative
fn is_air_loss_channel_total(name: &str) -> bool {
    name.starts_with("air_loss_") && name.ends_with("_raw_sum")
}

fn is_digest_word_or_group(name: &str) -> bool {
    if let Some((group, index)) = name.rsplit_once("_sha256_") {
        return !group.is_empty() && !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit());
    }
    matches!(
        name,
        "d75_terminal_payload" | "d64_terminal_envelope" | "d64_terminal_del2"
    )
}

pub const GAUGE_UNITS: &[&str] = &[
    "convoy_transport_attack_remaining_events",
    "convoy_transport_attack_active_callbacks",
    "convoy_transport_attack_resolver_member_bindings",
    "convoy_transport_attack_resolver_route_bindings",
    "convoy_transport_attack_resolver_unbound_convoy_clients",
    "domain53_sweep_age_ms",
    "lw_pub_start_us",
    "lw_pub_end_us",
    "d43_contract_scope_countries",
    "d43_contract_snapshot_countries",
    "d43_contract_outer_count",
    "d97_conference_count",
    "d85_observed_families",
];

pub const OUTSIDE_BUILD_SEGMENTS: &[(u16, &str)] = &[
    (0, "total"),
    (1, "wait"),
    (2, "identity"),
    (3, "prepoll"),
    (4, "prepub"),
    (5, "complete"),
    (6, "degnotice"),
    (7, "deglog"),
    (8, "openpub"),
    (9, "a30insert"),
    (10, "hookstage"),
    (11, "tailinsert"),
    (12, "buildtail_previous_frame"),
];

pub const OUTSIDE_BUILD_UNIT_NAMES: &[&str] = &[
    "lw_o0_us",
    "lw_o1_us",
    "lw_o2_us",
    "lw_o3_us",
    "lw_o4_us",
    "lw_o5_us",
    "lw_o6_us",
    "lw_o7_us",
    "lw_o8_us",
    "lw_o9_us",
    "lw_o10_us",
    "lw_o11_us",
    "lw_o12_us",
];

pub const OUTSIDE_BUILD_SEGMENT_COUNT: usize = OUTSIDE_BUILD_SEGMENTS.len();

pub fn outside_build_segment_name(code: u16) -> Option<&'static str> {
    OUTSIDE_BUILD_SEGMENTS
        .iter()
        .find(|(segment_code, _)| *segment_code == code)
        .map(|(_, name)| *name)
}

/// The frame's SERIAL TAIL, by segment.
///
/// `lw_ft_us` (the build's own `frame_time_us`) is
/// `lw_fanout_origin_us + lw_worker_span_us + sum(lw_t*_us)` plus a handful of
/// microseconds of statements between the fan-out close and the first mark.
/// Everything after the fan-out JOIN is serial: the frame is one thread from
/// there to the end of the build, so a millisecond here costs the frame a
/// millisecond, unlike a millisecond inside the fan-out.
///
/// WHY THE SEGMENTS ARE PROGRAM-ORDER RANGES AND NOT PER-DOMAIN TIMERS. The
/// tail is one straight-line sequence of ~90 `if let Some(read) = ..` publish
/// blocks with no common entry point to wrap: each one takes a reader's result
/// and appends that domain's records, with its own early exits and its own
/// borrow of the output buffer. Wrapping all ninety would be ninety edits into
/// the most delicate code in the build for a decomposition no finer than the
/// two heaviest domains, which this already isolates; the seven marks below sit
/// at top-level statement boundaries, cost nine lines between them, and cannot
/// change what any block does. The two segments that name a single domain
/// (`d81_publish`, `d47_publish`) are the two whose ranges contain exactly one
/// block.
///
/// The segments PARTITION the tail: adjacent marks, so they sum to the tail by
/// construction and the check with content is against `lw_ft_us`, which is
/// timed by a different instrument from a different origin. That check's
/// residual is TWO boundary gaps, both inside `lw_ft_us` and inside no segment:
/// the fan-out cycle-counter close between the fan-out span closing and the
/// first mark, and — because the last segment has no following segment to
/// absorb it, unlike every other — the final close's own arithmetic and return
/// plus the frame-time stamp.
pub const FRAME_TAIL_SEGMENTS: &[(u16, &str)] = &[
    (0, "derived_encode"),
    (1, "country_publish"),
    (2, "publish_early"),
    (3, "d81_publish"),
    (4, "publish_units"),
    (5, "d47_publish"),
    (6, "publish_late"),
];

pub const FRAME_TAIL_UNIT_NAMES: &[&str] = &[
    "lw_t0_us", "lw_t1_us", "lw_t2_us", "lw_t3_us", "lw_t4_us", "lw_t5_us", "lw_t6_us",
];

pub const FRAME_TAIL_SEGMENT_COUNT: usize = FRAME_TAIL_SEGMENTS.len();

/// The unit carrying WHICH segments were actually closed on the frame this
/// record describes: one bit per segment code, so a complete tail reads
/// `FRAME_TAIL_MARKS_COMPLETE`.
///
/// WHY A VALUE AND NOT A SOURCE PIN. The seven segment values alone cannot
/// distinguish a filled accumulator from a fresh one -- both are seven numbers,
/// and an accumulator that was never marked publishes seven honest zeros with
/// every count, every total and the host's whole population check intact. Four
/// review rounds tried to close that with pins over the collector's source, and
/// each was defeated by a compile-valid edit the next round found: a decoy
/// binding, a function-pointer alias beside a dead call, a module-level static
/// shadowing the build-local accumulator. Text matching cannot resolve names,
/// and the property is not about text.
///
/// So the frame reports it. Whatever value reaches the emitter, it carries the
/// marks it received; a default, a static, or anything else that was never
/// closed reports zero, the host refuses the decomposition by name, and the
/// capture says so instead of publishing a balanced fiction.
pub const FRAME_TAIL_MARKS_UNIT: &str = "lw_ft_marks";

/// Every segment closed exactly once: the low `FRAME_TAIL_SEGMENT_COUNT` bits.
pub const FRAME_TAIL_MARKS_COMPLETE: u64 = (1u64 << FRAME_TAIL_SEGMENT_COUNT) - 1;

pub fn frame_tail_segment_name(code: u16) -> Option<&'static str> {
    FRAME_TAIL_SEGMENTS
        .iter()
        .find(|(segment_code, _)| *segment_code == code)
        .map(|(_, name)| *name)
}

pub const POLL_GAP_BUCKET_MAX_US: [u64; 7] = [1_000, 2_000, 4_000, 8_000, 16_000, 32_000, 64_000];

pub const POLL_GAP_BUCKET_UNIT_NAMES: [&str; POLL_GAP_BUCKET_MAX_US.len() + 1] = [
    "lw_poll_gap_le1ms",
    "lw_poll_gap_le2ms",
    "lw_poll_gap_le4ms",
    "lw_poll_gap_le8ms",
    "lw_poll_gap_le16ms",
    "lw_poll_gap_le32ms",
    "lw_poll_gap_le64ms",
    "lw_poll_gap_gt64ms",
];

pub const POLL_GAP_BUCKET_COUNT: usize = POLL_GAP_BUCKET_UNIT_NAMES.len();

pub fn poll_gap_bucket(gap_us: u64) -> usize {
    POLL_GAP_BUCKET_MAX_US
        .iter()
        .position(|bound| gap_us <= *bound)
        .unwrap_or(POLL_GAP_BUCKET_COUNT - 1)
}

pub fn poll_gap_bucket_of_unit(name: &str) -> Option<usize> {
    POLL_GAP_BUCKET_UNIT_NAMES
        .iter()
        .position(|bucket| *bucket == name)
}

pub const STATEFUL_FAMILY_PER_FRAME_UNITS: &[&str] = &[
    "convoy_transport_attack_batches",
    "convoy_transport_attack_rows",
    // Sinks RECORDED with their route gap named (lane RX2). Per frame and
    // summed, like every other row count here: each is a count of rows in this
    // frame's batch, not a running state, and `..._rows_route_refused` is the
    // total the per-cause leaves below partition.
    "convoy_transport_attack_rows_route_refused",
    "convoy_transport_attack_row_route_refused_null_module_base",
    "convoy_transport_attack_row_route_refused_address_overflow",
    "convoy_transport_attack_row_route_refused_invalid_client_handle",
    "convoy_transport_attack_row_route_refused_reference_registry_unreadable",
    "convoy_transport_attack_row_route_refused_reference_resolution_failed",
    "convoy_transport_attack_row_route_refused_reference_resolution_unstable",
    "convoy_transport_attack_row_route_refused_object_vtable_unreadable",
    "convoy_transport_attack_row_route_refused_object_vtable_unstable",
    "convoy_transport_attack_row_route_refused_unsupported_producer",
    "convoy_transport_attack_row_route_refused_endpoint_pointer_unreadable",
    "convoy_transport_attack_row_route_refused_endpoint_pointer_unstable",
    "convoy_transport_attack_row_route_refused_endpoint_unreadable",
    "convoy_transport_attack_row_route_refused_endpoint_unstable",
    "convoy_transport_attack_row_route_refused_zero_endpoint",
    "convoy_transport_attack_row_route_refused_authentication",
    "convoy_transport_attack_row_route_refused_binding_lapsed",
    "convoy_transport_attack_row_route_refused_unclassified",
    "convoy_transport_attack_exact_complete_batches",
    "convoy_transport_attack_incomplete_batches",
    "convoy_transport_attack_hook_installed_batches",
    "convoy_transport_attack_queue_drain_complete_batches",
    "convoy_transport_attack_drained_rows",
    "convoy_transport_attack_incomplete_witness",
    "convoy_transport_attack_events_seen_this_drain",
    "convoy_transport_attack_events_emitted_this_drain",
    "convoy_transport_attack_drops_this_drain",
    "air_loss_stat_rows",
    "air_loss_binding_verified_rows",
];

pub const LAST_VALUE_UNITS: &[&str] = &[
    "rs_sink_time_p50_us",
    "rs_sink_time_p99_us",
    "heap_sweep_spawn_last_os_error",
    "heap_sweep_epoch",
    "d75_terminal_epoch",
];

pub const ZERO_GAUGE_UNITS: &[&str] = &["rs_sink_write_in_flight"];

pub const RAW_FAILURE_CONTEXT_UNITS: &[&str] = &[
    "d21_failure_side",
    "d21_failure_manager_index_plus_one",
    "d21_failure_vector_capacity",
    "d21_failure_vector_size",
    "d21_failure_vector_element_index_plus_one",
    "d21_border_war_observed_type_word",
    "d21_border_war_type_word_observed",
    "d57_failure_current_negative",
    "d57_failure_current_abs",
    "d57_failure_original_negative",
    "d57_failure_original_abs",
    "d57_failure_reason_size",
    "d57_failure_reason_capacity",
    "d57_failure_reason_first_qword",
    "d57_failure_reason_bytes_0",
    "d57_failure_reason_bytes_1",
    "d57_failure_reason_bytes_2",
    "d57_failure_reason_bytes_3",
    "d57_failure_reason_bytes_4",
    "d57_failure_reason_bytes_5",
    "d57_failure_reason_bytes_6",
    "d61_country_read_failure_country_idx",
    "d61_country_read_failure_field_idx",
    "d65_first_failure_observed",
    "d65_first_failure_expected",
    "d83_fail_actual",
    "d83_fail_expected",
    "domain47_manager_failure_vector",
    "domain47_manager_failure_observed_value",
    "domain47_manager_failure_locator_observed",
    "domain48_first_invalid_progress_line_type",
    "domain48_first_invalid_progress_cost_raw_i32_bits",
    "domain48_first_invalid_progress_produced_raw_i32_bits",
    "domain54_exact_orders_group_manager_rejected",
    "domain54_orders_group_carrier_rejected",
];

pub const LANE_WALL_SESSION_CONSTANTS: &[&str] =
    &["lw_prefetch_dist_relation", "lw_prefetch_dist_candidates"];

pub const DOMAIN47_PREWARM_UNITS: &[(&str, Accumulation)] = &[
    ("domain47_prewarm_clock_verdict", Accumulation::LastValue),
    ("domain47_prewarm_known_families", Accumulation::BitwiseOr),
    (
        "domain47_prewarm_families_dark_without_reason",
        Accumulation::BitwiseOr,
    ),
    (
        "domain47_prewarm_shadow_walk_state",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_recheck_outcome",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_char_id",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_map",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_address",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_address",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_censused",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_char_id",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_map",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_map_size",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_candidate_bodies",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_candidate_entries",
        Accumulation::LastValue,
    ),
    ("domain47_prewarm_hold_reason", Accumulation::LastValue),
    (
        "domain47_prewarm_family_gap_reason",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_gap_reason",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_failure_kind",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_failure_char_id",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_failure_ordinal",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_failure_detail",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_armed",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_size",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_capacity",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_pointer",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_terminator_outcome",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_terminator_value",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_portrait_probe_reader_reason",
        Accumulation::LastValue,
    ),
    ("domain47_prewarm_ran", Accumulation::PerFrame),
    (
        "domain47_prewarm_negative_cache_hit",
        Accumulation::PerFrame,
    ),
    ("domain47_prewarm_refresh", Accumulation::PerFrame),
    (
        "domain47_prewarm_refresh_held_roots",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_portrait_failed_characters",
        Accumulation::PerFrame,
    ),
    ("domain47_prewarm_portrait_rows", Accumulation::PerFrame),
    (
        "domain47_prewarm_portrait_cp1252_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_role_walk_gap_reason",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_role_walk_refused_leader",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_role_walk_refused_advisor",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_role_walk_refused_map",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_role_walk_refused_body",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_leader_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_advisor_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_population_ab_walk_leader_roots",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_population_ab_walk_advisor_roots",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_population_ab_sweep_leader_roots",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_population_ab_sweep_advisor_roots",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_sweep_shadow_torn_hits",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_country_leader_role_count",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_matched_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_only_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_refused_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_recheck_hit_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_recheck_miss_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_recheck_refused_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_negative_cache_frames_remaining",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_stable_root_count",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_skipped_root_rows",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_excluded_scan_hits",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_advisor_root_count",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_trait_token_count",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_ambiguous_root_rows",
        Accumulation::LastValue,
    ),
];

pub fn unit_accumulation(name: &str) -> Accumulation {
    declared_unit_accumulation(name).unwrap_or(Accumulation::PerFrame)
}

pub fn declared_unit_accumulation(name: &str) -> Option<Accumulation> {
    if let Some((_, class)) = DOMAIN47_PREWARM_UNITS
        .iter()
        .find(|(unit, _)| *unit == name)
    {
        return Some(*class);
    }
    if RAW_WORD_SUFFIXES
        .iter()
        .any(|suffix| name.ends_with(suffix))
    {
        return Some(Accumulation::LastValue);
    }
    if BITMASK_SUFFIXES.iter().any(|suffix| name.ends_with(suffix)) {
        return Some(Accumulation::BitwiseOr);
    }
    if name.starts_with("d69_failed_slot_mask_") {
        return Some(Accumulation::BitwiseOr);
    }
    if is_digest_word_or_group(name) {
        return Some(Accumulation::LastValue);
    }
    if CUMULATIVE_UNITS.contains(&name)
        || LANE_WALL_SESSION_CONSTANTS.contains(&name)
        || is_air_loss_channel_total(name)
    {
        return Some(Accumulation::Cumulative);
    }
    if GAUGE_UNITS.contains(&name) || ZERO_GAUGE_UNITS.contains(&name) {
        return Some(Accumulation::Gauge);
    }
    if LAST_VALUE_UNITS.contains(&name) || RAW_FAILURE_CONTEXT_UNITS.contains(&name) {
        return Some(Accumulation::LastValue);
    }
    if STATEFUL_FAMILY_PER_FRAME_UNITS.contains(&name) {
        return Some(Accumulation::PerFrame);
    }
    if STATEFUL_UNIT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        return Some(Accumulation::Undeclared);
    }
    None
}

pub fn is_stateful_unit_family(name: &str) -> bool {
    STATEFUL_UNIT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_role_and_presence_code_is_non_zero_and_round_trips() {
        for role in [
            UnitRole::Deciding,
            UnitRole::Locator,
            UnitRole::Context,
            UnitRole::Counter,
            UnitRole::Undeclared,
        ] {
            assert_ne!(role.code(), 0, "{role:?} must not use the reserved 0 code");
            assert_eq!(UnitRole::from_code(role.code()), Some(role));
        }
        for presence in [
            UnitPresence::Observed,
            UnitPresence::Absent,
            UnitPresence::ResetSentinel,
            UnitPresence::NotApplicable,
            UnitPresence::Undeclared,
        ] {
            assert_ne!(presence.code(), 0);
            assert_eq!(UnitPresence::from_code(presence.code()), Some(presence));
        }
        for terminal in [
            OffFrameTerminal::Running,
            OffFrameTerminal::CompletedOk,
            OffFrameTerminal::CompletedError,
            OffFrameTerminal::Panicked,
        ] {
            assert_ne!(terminal.code(), 0);
            assert_eq!(OffFrameTerminal::from_code(terminal.code()), Some(terminal));
        }
        for kind in [
            TruncationKind::None,
            TruncationKind::BudgetShare,
            TruncationKind::UnitTooLarge,
            TruncationKind::ProducerOverflow,
        ] {
            assert_ne!(kind.code(), 0);
            assert_eq!(TruncationKind::from_code(kind.code()), Some(kind));
        }
        assert_eq!(UnitRole::from_code(0), None);
        assert_eq!(UnitPresence::from_code(0), None);
        assert_eq!(OffFrameTerminal::from_code(0), None);
        assert_eq!(TruncationKind::from_code(0), None);
    }

    /// The discriminants are an ABI: retained telemetry must stay comparable
    #[test]
    fn discriminants_are_pinned() {
        assert_eq!(
            [
                UnitRole::Deciding.code(),
                UnitRole::Locator.code(),
                UnitRole::Context.code(),
                UnitRole::Counter.code(),
                UnitRole::Undeclared.code()
            ],
            [1, 2, 3, 4, 5]
        );
        assert_eq!(
            [
                UnitPresence::Observed.code(),
                UnitPresence::Absent.code(),
                UnitPresence::ResetSentinel.code(),
                UnitPresence::NotApplicable.code(),
                UnitPresence::Undeclared.code()
            ],
            [1, 2, 3, 4, 5]
        );
        assert_eq!(
            [
                Authority::SelfDomain.kind_code(),
                Authority::OtherDomain(0).kind_code(),
                Authority::Engine { rva: 0 }.kind_code(),
                Authority::Host.kind_code(),
                Authority::OffFrameWorker.kind_code()
            ],
            [1, 2, 3, 4, 5]
        );
        assert_eq!(
            [
                OffFrameTerminal::Running.code(),
                OffFrameTerminal::CompletedOk.code(),
                OffFrameTerminal::CompletedError.code(),
                OffFrameTerminal::Panicked.code()
            ],
            [1, 2, 3, 4]
        );
        assert_eq!(
            [
                TruncationKind::None.code(),
                TruncationKind::BudgetShare.code(),
                TruncationKind::UnitTooLarge.code(),
                TruncationKind::ProducerOverflow.code()
            ],
            [1, 2, 3, 4]
        );
        assert_eq!(PREDICATE_UNDECLARED, 0xFFFF_FF01);
    }

    #[test]
    fn units_round_trip_with_role_and_presence_preserved() {
        let units = vec![
            DiagUnit::new(
                "d44_country_supply_failure_stage",
                0,
                UnitRole::Deciding,
                UnitPresence::Observed,
            ),
            DiagUnit::new(
                "d44_country_supply_slot_index",
                0xFFFF_FFFF,
                UnitRole::Locator,
                UnitPresence::ResetSentinel,
            ),
            DiagUnit::new(
                "d44_country_supply_tag_db_id",
                0,
                UnitRole::Locator,
                UnitPresence::Absent,
            ),
        ];
        let wire = encode_units(&units).unwrap();
        let (back, consumed) = decode_units(&wire).unwrap();
        assert_eq!(consumed, wire.len());
        assert_eq!(back, units);
        assert_eq!(back[0].value, 0);
        assert!(back[0].presence.value_is_a_measurement());
        assert!(!back[2].presence.value_is_a_measurement());
    }

    #[test]
    fn a_unit_with_an_undefined_role_or_presence_fails_closed() {
        let units = vec![DiagUnit::deciding("x", 1)];
        let wire = encode_units(&units).unwrap();
        let mut bad_role = wire.clone();
        bad_role[4] = 0;
        assert!(decode_units(&bad_role).is_err());
        let mut bad_presence = wire.clone();
        bad_presence[5] = 9;
        assert!(decode_units(&bad_presence).is_err());
    }

    #[test]
    fn locator_carries_ordinal_and_extent_at_every_level() {
        let levels = vec![
            LocatorLevel::new("country_container", 3, Some(12)),
            LocatorLevel::new("region", 41, Some(96)).with_address(0x7ff6_0000_1000),
            LocatorLevel::new("record", 7, None),
        ];
        let wire = encode_locator(&levels).unwrap();
        let (back, consumed) = decode_locator(&wire).unwrap();
        assert_eq!(consumed, wire.len());
        assert_eq!(back, levels);
        assert_eq!(back[1].render(), "region 41 of 96@0x7ff600001000");
        assert_eq!(back[2].render(), "record 7 of ?");
        assert_eq!(
            LocatorLevel::new("record", 7, Some(0)).render(),
            "record 7 of 0"
        );
    }

    #[test]
    fn a_locator_deeper_than_the_bound_fails_closed() {
        let deep: Vec<LocatorLevel> = (0..MAX_LOCATOR_DEPTH + 1)
            .map(|index| LocatorLevel::new("level", index as u64, None))
            .collect();
        assert!(encode_locator(&deep).is_err());
        let mut wire = encode_locator(&deep[..MAX_LOCATOR_DEPTH]).unwrap();
        wire[0] = (MAX_LOCATOR_DEPTH + 1) as u8;
        assert!(decode_locator(&wire).is_err());
    }

    #[test]
    fn authority_round_trips_and_an_undefined_kind_fails_closed() {
        for authority in [
            Authority::SelfDomain,
            Authority::OtherDomain(69),
            Authority::Engine { rva: 0x1920D20 },
            Authority::Host,
            Authority::OffFrameWorker,
        ] {
            let wire = authority.encode();
            assert_eq!(Authority::decode(&wire).unwrap(), authority);
        }
        let mut bad = Authority::SelfDomain.encode();
        bad[0] = 0;
        assert!(Authority::decode(&bad).is_err());
        bad[0] = 99;
        assert!(Authority::decode(&bad).is_err());
    }

    #[test]
    fn a_predicate_code_of_zero_is_refused() {
        assert!(Predicate::new(0, "never").is_err());
        assert!(Predicate::new(1, "ok").is_ok());
        assert!(Predicate::undeclared().is_undeclared());
        assert!(!Predicate::new(1, "ok").unwrap().is_undeclared());
    }

    #[test]
    fn off_frame_outcome_separates_running_from_overdue() {
        let running = OffFrameOutcome {
            spawn_generation: 4,
            elapsed_us: 1_000,
            deadline_us: 50_000,
            terminal: OffFrameTerminal::Running,
        };
        assert!(!running.overdue());
        let overdue = OffFrameOutcome {
            elapsed_us: 50_000_000,
            ..running.clone()
        };
        assert!(overdue.overdue());
        assert!(overdue.render().contains("OVERDUE"));
        let done = OffFrameOutcome {
            terminal: OffFrameTerminal::CompletedOk,
            ..overdue.clone()
        };
        assert!(!done.overdue());
        for outcome in [running, overdue, done] {
            assert_eq!(OffFrameOutcome::decode(&outcome.encode()).unwrap(), outcome);
        }
        let mut bad = OffFrameOutcome {
            spawn_generation: 0,
            elapsed_us: 0,
            deadline_us: 0,
            terminal: OffFrameTerminal::Running,
        }
        .encode();
        bad[24] = 0;
        assert!(OffFrameOutcome::decode(&bad).is_err());
    }

    fn unit(name: &str, role: UnitRole) -> DiagUnit {
        DiagUnit::new(name, 1, role, UnitPresence::Observed)
    }

    #[test]
    fn a_zero_valued_deciding_unit_is_never_dropped_for_being_zero() {
        let mut units = vec![DiagUnit::new(
            "d44_country_supply_failure_stage",
            0,
            UnitRole::Deciding,
            UnitPresence::Observed,
        )];
        for index in 0..40 {
            units.push(DiagUnit::counter(format!("d44_filler_{index}"), 99));
        }
        units.rotate_left(1);
        let grants = allocate_unit_budget(
            &[BudgetRequest {
                domain_id: 44,
                units,
            }],
            0,
            usize::MAX,
        );
        let kept = &grants[0].units;
        assert!(
            kept.iter()
                .any(|unit| unit.name == "d44_country_supply_failure_stage" && unit.value == 0),
            "the zero-valued deciding unit must survive an empty pool, got {kept:?}"
        );
        assert_eq!(grants[0].account.kind, TruncationKind::BudgetShare);
        assert!(grants[0].account.units_dropped() > 0);
        assert!(grants[0].account.bytes_dropped > 0);
    }

    #[test]
    fn a_verbose_offender_cannot_take_a_quiet_neighbours_share() {
        let verbose = BudgetRequest {
            domain_id: 47,
            units: (0..200)
                .map(|index| unit(&format!("d47_counter_{index:03}"), UnitRole::Counter))
                .collect(),
        };
        let quiet = BudgetRequest {
            domain_id: 91,
            units: vec![
                unit("d91_exact_read_failure_code", UnitRole::Deciding),
                unit("d91_context_a", UnitRole::Context),
                unit("d91_context_b", UnitRole::Context),
            ],
        };
        let pool = 400usize;
        let grants = allocate_unit_budget(&[verbose.clone(), quiet.clone()], pool, usize::MAX);
        let quiet_grant = grants.iter().find(|g| g.domain_id == 91).unwrap();
        assert_eq!(
            quiet_grant.units.len(),
            3,
            "the quiet offender's three units all fit inside its equal share"
        );
        assert_eq!(quiet_grant.account.kind, TruncationKind::None);
        let alone = allocate_unit_budget(&[quiet], pool, usize::MAX);
        assert_eq!(alone[0].units, quiet_grant.units);
        let verbose_grant = grants.iter().find(|g| g.domain_id == 47).unwrap();
        assert_eq!(verbose_grant.account.kind, TruncationKind::BudgetShare);
    }

    #[test]
    fn every_offender_receives_at_least_its_equal_share_of_the_pool() {
        let requests: Vec<BudgetRequest> = (0..7u16)
            .map(|domain_id| BudgetRequest {
                domain_id,
                units: (0..60)
                    .map(|index| {
                        unit(
                            &format!("d{domain_id}_counter_{index:03}"),
                            UnitRole::Counter,
                        )
                    })
                    .collect(),
            })
            .collect();
        let pool = 3_000usize;
        let grants = allocate_unit_budget(&requests, pool, usize::MAX);
        let equal = pool / requests.len();
        for grant in &grants {
            let bytes: usize = grant.units.iter().map(DiagUnit::encoded_len).sum();
            assert!(
                bytes + UNIT_FIXED_LEN + 32 >= equal,
                "domain {} got {bytes} bytes, below its {equal}-byte equal share",
                grant.domain_id
            );
        }
        let total: usize = grants
            .iter()
            .flat_map(|grant| grant.units.iter())
            .map(DiagUnit::encoded_len)
            .sum();
        let ceiling = requests.len() * OFFENDER_FLOOR_BYTES + pool;
        assert!(
            total <= ceiling,
            "allocation {total} overran the {ceiling}-byte bound"
        );
    }

    #[test]
    fn an_unmigrated_offender_with_no_declared_roles_still_reaches_the_wire() {
        let unmigrated = BudgetRequest {
            domain_id: 65,
            units: (0..30)
                .map(|index| DiagUnit::undeclared(format!("d65_legacy_{index:03}"), index))
                .collect(),
        };
        let hog = BudgetRequest {
            domain_id: 47,
            units: (0..4_000)
                .map(|index| DiagUnit::counter(format!("d47_counter_{index:04}"), index))
                .collect(),
        };
        let grants = allocate_unit_budget(&[hog, unmigrated], 0, usize::MAX);
        let quiet = grants.iter().find(|g| g.domain_id == 65).unwrap();
        assert_eq!(
            quiet.units.len(),
            MAX_FLOOR_UNITS_PER_OFFENDER,
            "an unmigrated offender must still place its floor with an empty pool"
        );
        assert_eq!(quiet.units[0].name, "d65_legacy_000");
        assert_eq!(quiet.units[0].role, UnitRole::Undeclared);
        assert_eq!(quiet.units[0].presence, UnitPresence::Undeclared);
        assert_eq!(quiet.units[0].value, 0);
        assert_eq!(quiet.account.kind, TruncationKind::BudgetShare);
        assert_eq!(quiet.account.units_offered, 30);
        assert_eq!(
            quiet.account.units_emitted,
            MAX_FLOOR_UNITS_PER_OFFENDER as u32
        );
        for grant in &grants {
            assert!(
                !grant.units.is_empty(),
                "domain {} reported nothing",
                grant.domain_id
            );
        }
    }

    #[test]
    fn floor_units_are_capped_so_one_domain_cannot_claim_everything_as_deciding() {
        let greedy = BudgetRequest {
            domain_id: 1,
            units: (0..64)
                .map(|index| unit(&format!("greedy_{index:03}"), UnitRole::Deciding))
                .collect(),
        };
        let grants = allocate_unit_budget(&[greedy], 0, usize::MAX);
        assert_eq!(
            grants[0].units.len(),
            MAX_FLOOR_UNITS_PER_OFFENDER,
            "the floor is capped at {MAX_FLOOR_UNITS_PER_OFFENDER} units"
        );
        let bytes: usize = grants[0].units.iter().map(DiagUnit::encoded_len).sum();
        assert!(bytes <= OFFENDER_FLOOR_BYTES);
    }

    #[test]
    fn allocation_is_deterministic_and_ordered_by_role() {
        let request = BudgetRequest {
            domain_id: 42,
            units: vec![
                unit("counter_a", UnitRole::Counter),
                unit("deciding_a", UnitRole::Deciding),
                unit("context_a", UnitRole::Context),
                unit("locator_a", UnitRole::Locator),
                unit("deciding_b", UnitRole::Deciding),
            ],
        };
        let first = allocate_unit_budget(&[request.clone()], 500, usize::MAX);
        let second = allocate_unit_budget(&[request], 500, usize::MAX);
        assert_eq!(first[0].units, second[0].units);
        let names: Vec<&str> = first[0].units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "deciding_a",
                "deciding_b",
                "locator_a",
                "context_a",
                "counter_a"
            ],
            "wire order must be role order, then producer order within a role"
        );
    }

    #[test]
    fn a_first_failure_survives_three_hundred_and_fifty_later_iterations() {
        let mut latch: FirstFailureLatch<(u32, &str)> = FirstFailureLatch::new();
        assert!(latch.is_empty());
        assert!(latch.offer((3, "country_supply_slot_read")));
        for iteration in 4..=350u32 {
            latch.offer((iteration, "later_and_different"));
        }
        assert_eq!(latch.first(), Some(&(3, "country_supply_slot_read")));
        assert_eq!(latch.later_failures(), 347);
        assert_eq!(latch.total(), 348);
        latch.reset();
        assert!(latch.is_empty());
        assert_eq!(latch.total(), 0);
    }

    #[test]
    fn no_offender_is_granted_more_than_the_container_can_hold() {
        let requests = vec![
            BudgetRequest {
                domain_id: 44,
                units: (0..2_000u64)
                    .map(|index| DiagUnit::counter(format!("d44_counter_{index:04}"), index))
                    .collect(),
            },
            BudgetRequest {
                domain_id: 91,
                units: vec![unit("d91_deciding", UnitRole::Deciding)],
            },
        ];
        const CAP: usize = 512;
        let grants = allocate_unit_budget(&requests, 100_000, CAP);
        for grant in &grants {
            let bytes: usize = grant.units.iter().map(DiagUnit::encoded_len).sum();
            assert!(
                bytes <= CAP,
                "domain {} was granted {bytes} bytes against a {CAP} cap",
                grant.domain_id
            );
        }
        let hog = grants.iter().find(|g| g.domain_id == 44).unwrap();
        assert_ne!(hog.account.kind, TruncationKind::None);
        let quiet = grants.iter().find(|g| g.domain_id == 91).unwrap();
        assert_eq!(quiet.units.len(), 1);
        assert_eq!(quiet.account.kind, TruncationKind::None);

        const TIGHT: usize = 100;
        assert!(
            TIGHT < OFFENDER_FLOOR_BYTES,
            "this case is only meaningful while the cap is below the floor"
        );
        let tight = allocate_unit_budget(&requests, 0, TIGHT);
        for grant in &tight {
            let bytes: usize = grant.units.iter().map(DiagUnit::encoded_len).sum();
            assert!(
                bytes <= TIGHT,
                "domain {} placed {bytes} floor bytes against a {TIGHT} cap",
                grant.domain_id
            );
        }
        assert!(tight.iter().all(|grant| !grant.units.is_empty()));
    }

    #[test]
    fn the_allocation_does_not_depend_on_the_callers_slice_order() {
        let build = |domain_id: u16| BudgetRequest {
            domain_id,
            units: (0..12u64)
                .map(|index| DiagUnit::counter(format!("d{domain_id}_c{index:04}"), index))
                .collect(),
        };
        assert_eq!(build(12).units[0].encoded_len(), 21);
        let forward = vec![build(12), build(53), build(91)];
        let reversed = vec![build(91), build(53), build(12)];
        let pool = 61usize;

        let a = allocate_unit_budget(&forward, pool, usize::MAX);
        let b = allocate_unit_budget(&reversed, pool, usize::MAX);
        for grant in &a {
            let other = b
                .iter()
                .find(|g| g.domain_id == grant.domain_id)
                .expect("same domains either way");
            assert_eq!(
                grant.units, other.units,
                "domain {} allocated differently when the caller reordered its slice",
                grant.domain_id
            );
            assert_eq!(grant.account, other.account);
        }
    }

    #[test]
    fn truncation_account_states_what_was_lost_without_a_literal_ellipsis() {
        let account = TruncationAccount {
            kind: TruncationKind::BudgetShare,
            units_offered: 20,
            units_emitted: 6,
            bytes_dropped: 412,
        };
        assert_eq!(account.units_dropped(), 14);
        let rendered = account.render();
        assert!(!rendered.contains("..."), "got {rendered}");
        assert!(rendered.contains("dropped=14"));
        assert!(rendered.contains("bytes_dropped=412"));
        assert_eq!(TruncationAccount::complete(6).units_dropped(), 0);
    }

    const T15_CALLBACKS_SEEN_RUNS: &[(u64, usize)] = &[
        (1, 9),
        (2, 8),
        (3, 8),
        (4, 3),
        (5, 5),
        (6, 4),
        (7, 1),
        (9, 1),
        (10, 3),
        (11, 9),
        (12, 5),
        (13, 4),
        (14, 9),
        (15, 26),
        (16, 1),
        (17, 2),
        (18, 5),
        (19, 8),
        (20, 8),
        (21, 5),
        (22, 10),
        (23, 5),
        (24, 4),
        (25, 1),
        (26, 10),
        (27, 2),
        (28, 3),
        (29, 2),
        (30, 1),
        (32, 8),
        (33, 2),
        (34, 1),
        (35, 3),
        (36, 2),
        (37, 2),
        (38, 3),
        (39, 2),
        (40, 2),
        (41, 2),
        (42, 4),
        (43, 6),
        (44, 1),
        (45, 3),
        (46, 2),
        (48, 2),
        (49, 4),
        (50, 1),
        (51, 9),
        (52, 1),
        (53, 8),
        (54, 2),
        (55, 1),
        (56, 2),
        (57, 3),
        (58, 2),
        (60, 1),
        (62, 1),
        (63, 2),
        (64, 13),
        (65, 4),
        (66, 1),
        (68, 4),
        (69, 2),
        (70, 4),
        (71, 7),
        (72, 3),
        (73, 2),
        (74, 2),
        (75, 2),
        (76, 1),
        (77, 6),
        (78, 4),
        (81, 1),
        (82, 8),
        (84, 2),
        (85, 2),
        (86, 1),
        (87, 2),
        (89, 1),
        (90, 2),
        (92, 4),
        (93, 5),
        (94, 3),
        (95, 3),
        (96, 1),
        (99, 4),
        (100, 17),
        (102, 9),
        (103, 1),
        (104, 9),
        (105, 2),
        (106, 15),
        (107, 5),
    ];

    fn t15_callbacks_seen_series() -> Vec<u64> {
        T15_CALLBACKS_SEEN_RUNS
            .iter()
            .flat_map(|&(value, count)| std::iter::repeat_n(value, count))
            .collect()
    }

    #[test]
    fn the_t15_cumulative_series_folds_to_its_last_value_not_its_sum() {
        let series = t15_callbacks_seen_series();
        assert_eq!(series.len(), 396, "t15 published 396 d90 batches");
        assert!(
            series.windows(2).all(|w| w[0] <= w[1]),
            "the counter is monotonic, which is what makes it cumulative"
        );

        for name in [
            "convoy_transport_attack_callbacks_seen",
            "convoy_transport_attack_original_calls",
            "convoy_transport_attack_dropped_hour_or_epoch",
            "convoy_transport_attack_dropped_events",
        ] {
            let class = unit_accumulation(name);
            assert_eq!(class, Accumulation::Cumulative, "{name}");
            let mut running: Option<u64> = None;
            for value in &series {
                running = Some(class.fold(running, *value));
            }
            assert_eq!(
                running,
                Some(107),
                "{name}: a cumulative unit's whole-capture figure is its final \
                 absolute value"
            );
        }

        let summed = series.iter().fold(0u64, |a, b| a + b);
        assert_eq!(summed, 19_470, "the published t15 figure");
        assert_ne!(summed, 107);
    }

    #[test]
    fn the_new_d90_hour_split_units_are_classified_and_none_is_summed() {
        for name in [
            "convoy_transport_attack_dropped_pre_hour_unreadable",
            "convoy_transport_attack_dropped_receipt_hour_mismatch",
            "convoy_transport_attack_dropped_post_hour_unreadable",
            "convoy_transport_attack_dropped_post_hour_changed",
            "convoy_transport_attack_dropped_resolver_snapshot_absent",
            "convoy_transport_attack_dropped_resolver_unpublished",
            "convoy_transport_attack_receipt_hour_lag_max",
            "convoy_transport_attack_receipt_hour_lead_max",
        ] {
            assert_eq!(unit_accumulation(name), Accumulation::Cumulative, "{name}");
        }

        let series = t15_callbacks_seen_series();
        let class = unit_accumulation("convoy_transport_attack_dropped_receipt_hour_mismatch");
        let mut running: Option<u64> = None;
        for value in &series {
            running = Some(class.fold(running, *value));
        }
        assert_eq!(running, Some(107));
    }

    #[test]
    fn a_per_frame_unit_is_still_summed() {
        let class = unit_accumulation("convoy_transport_attack_rows");
        assert_eq!(class, Accumulation::PerFrame);
        let mut running: Option<u64> = None;
        for value in [3u64, 1, 4, 1, 5] {
            running = Some(class.fold(running, value));
        }
        assert_eq!(running, Some(14));

        let generic = unit_accumulation("fuel_rows_decoded");
        assert_eq!(generic, Accumulation::PerFrame);
        assert_eq!(generic.fold(Some(10), 5), 15);
    }

    #[test]
    fn every_scheduler_probe_unit_is_classified_and_none_of_them_is_summed() {
        const EMITTED: &[&str] = &[
            "rs_observations",
            "rs_scheduled_hours",
            "rs_accepted_hours",
            "rs_degraded_hours",
            "rs_refused_hours",
            "rs_evicted_pending_hours",
            "rs_abandoned_hours",
            "rs_flushed_pending_hours",
            "rs_unobserved_overrun_hours",
            "rs_unobserved_clock_unknown_hours",
            "rs_counter_anomaly_events",
            "rs_untrusted_clock_recovery_events",
            "rs_untrusted_clock_recoveries_dropped",
            "rs_untrusted_clock_recovery_max_delta_hours",
            "rs_post_terminal_observations",
            "rs_gapped_domain_hours",
            "rs_overrun_domain_hours",
            "rs_backpressure_domain_hours",
            "rs_torn_domain_hours",
            "rs_worker_panic_domain_hours",
            "rs_clock_unreadable_events",
            "rs_clock_unreadable_intervals_dropped",
            "rs_control_lane_compactions",
            "rs_max_request_depth",
            "rs_max_publishing_depth",
            "rs_request_cap",
            "rs_max_control_depth",
            "rs_control_cap",
            "rs_reloads",
            "rs_terminal",
            "rs_zero_mutation_policy",
            "rs_scheduler_active",
            "rs_sink_write_failures",
            "rs_sink_open_attempts",
            "rs_sink_open_finishes",
            "rs_sink_requested_count",
            "rs_sink_opened_count",
            "rs_sink_open_failures",
            "rs_sink_write_attempts",
            "rs_sink_write_finishes",
            "rs_sink_write_commits",
            "rs_sink_write_in_flight",
            "rs_sink_file_bytes_committed",
            "rs_sink_pipe_bytes_committed",
            "rs_sink_time_sample_count",
            "rs_sink_time_total_us",
            "rs_sink_time_p50_us",
            "rs_sink_time_p99_us",
            "rs_sink_time_max_us",
            "rs_transport_failed",
            "rs_finalized",
            "rs_configured_sink_count",
            "rs_pipe_sink_enabled",
            "rs_file_sink_enabled",
            "rs_pipe_sink_requested",
            "rs_pipe_sink_opened",
            "rs_file_sink_requested",
            "rs_file_sink_opened",
        ];
        for name in EMITTED {
            let class = unit_accumulation(name);
            assert_ne!(
                class,
                Accumulation::Undeclared,
                "{name} reaches the manifest with no declared accumulation"
            );
            assert_ne!(
                class,
                Accumulation::PerFrame,
                "{name} comes from SchedulerTelemetry/SinkDeliveryState, neither of \
                 which is reset within a capture, so its per-frame value is absolute"
            );
        }
    }

    #[test]
    fn an_unregistered_unit_in_a_stateful_family_is_undeclared_not_per_frame() {
        for name in [
            "rs_some_counter_added_next_quarter",
            "convoy_transport_attack_something_new",
        ] {
            assert!(is_stateful_unit_family(name));
            assert_eq!(
                unit_accumulation(name),
                Accumulation::Undeclared,
                "{name} must announce itself rather than join the summed side"
            );
        }
        assert!(!is_stateful_unit_family("weather_rows_total"));
        assert_eq!(
            unit_accumulation("weather_rows_total"),
            Accumulation::PerFrame
        );
    }

    #[test]
    fn undeclared_folds_exactly_like_per_frame() {
        for (running, incoming) in [(None, 7u64), (Some(3), 4), (Some(u64::MAX), 1)] {
            assert_eq!(
                Accumulation::Undeclared.fold(running, incoming),
                Accumulation::PerFrame.fold(running, incoming)
            );
        }
    }

    #[test]
    fn the_established_sample_and_mask_conventions_moved_into_the_authority() {
        assert_eq!(
            unit_accumulation("bt_unit_leader_skill_raw_sample"),
            Accumulation::LastValue
        );
        assert_eq!(
            unit_accumulation("so_d45_partial_sample"),
            Accumulation::LastValue
        );
        assert_eq!(
            unit_accumulation("d69_failed_slot_mask"),
            Accumulation::BitwiseOr
        );
        assert_eq!(
            unit_accumulation("d69_failed_slot_mask_7"),
            Accumulation::BitwiseOr
        );
        assert_eq!(
            unit_accumulation("d69_stale_slot_mask"),
            Accumulation::BitwiseOr
        );
        assert_eq!(Accumulation::BitwiseOr.fold(Some(1), 1), 1);
        assert_eq!(Accumulation::LastValue.fold(Some(0xDEAD), 0xBEEF), 0xBEEF);
    }

    #[test]
    fn a_gauge_folds_to_its_peak() {
        let class = unit_accumulation("convoy_transport_attack_resolver_member_bindings");
        assert_eq!(class, Accumulation::Gauge);
        let mut running: Option<u64> = None;
        for value in [9_865u64, 9_865, 9_865] {
            running = Some(class.fold(running, value));
        }
        assert_eq!(
            running,
            Some(9_865),
            "one publication observed three times is one publication"
        );
    }

    #[test]
    fn the_d43_contract_census_widths_are_gauges_not_running_totals() {
        for name in [
            "d43_contract_scope_countries",
            "d43_contract_snapshot_countries",
            "d43_contract_outer_count",
        ] {
            let class = unit_accumulation(name);
            assert_eq!(
                class,
                Accumulation::Gauge,
                "{name} is a width observed each frame, not a total over the capture"
            );
            let mut running: Option<u64> = None;
            for _ in 0..294 {
                running = Some(class.fold(running, 439));
            }
            assert_eq!(
                running,
                Some(439),
                "{name}: 439 countries seen on 294 frames is still 439 countries"
            );
        }
        assert_eq!(
            unit_accumulation("d43_contract_failure_stage"),
            Accumulation::LastValue
        );
        assert_eq!(
            unit_accumulation("d43_contract_failure_tag_db_id"),
            Accumulation::LastValue
        );
    }

    #[test]
    fn the_d47_prewarm_code_and_mask_units_are_not_summed() {
        let verdict = unit_accumulation("domain47_prewarm_clock_verdict");
        assert_eq!(verdict, Accumulation::LastValue);
        let mut running: Option<u64> = None;
        for _ in 0..219 {
            running = Some(verdict.fold(running, 1));
        }
        assert_eq!(
            running,
            Some(1),
            "219 frames agreeing on verdict 1 is verdict 1, not 219"
        );
        assert_eq!(
            Accumulation::PerFrame.fold(Some(218), 1),
            219,
            "the summed spelling t36 published, kept here as the thing being replaced"
        );

        let families = unit_accumulation("domain47_prewarm_known_families");
        assert_eq!(families, Accumulation::BitwiseOr);
        let mut running: Option<u64> = None;
        for _ in 0..161 {
            running = Some(families.fold(running, 0x7F));
        }
        running = Some(families.fold(running, 0xFF));
        assert_eq!(
            running,
            Some(0xFF),
            "the union of the masks observed, which is the readable question"
        );
        let mut summed: Option<u64> = None;
        for _ in 0..161 {
            summed = Some(Accumulation::PerFrame.fold(summed, 0x7F));
        }
        summed = Some(Accumulation::PerFrame.fold(summed, 0xFF));
        assert_eq!(summed, Some(161 * 0x7F + 0xFF));
        assert!(
            summed.unwrap() > 0xFF,
            "a sum of masks leaves the eight-bit vocabulary entirely, which is how a \
             capture publishes a five-figure family mask"
        );

        let dark = unit_accumulation("domain47_prewarm_families_dark_without_reason");
        assert_eq!(dark, Accumulation::BitwiseOr);
        let mut running: Option<u64> = None;
        for value in [0u64, 0, 0b1000_0000, 0, 0] {
            running = Some(dark.fold(running, value));
        }
        assert_eq!(
            running,
            Some(0b1000_0000),
            "one frame's unexplained family must not be erased by the frames after it"
        );
        assert_eq!(
            Accumulation::LastValue.fold(Some(0b1000_0000), 0),
            0,
            "which is what the `_reason` suffix would have published"
        );

        let state = unit_accumulation("domain47_prewarm_shadow_walk_state");
        assert_eq!(state, Accumulation::LastValue);
        assert_eq!(state.fold(Some(2), 2), 2);
    }

    #[test]
    fn the_d47_sweep_only_classification_units_carry_their_own_kinds() {
        for name in [
            "domain47_prewarm_shadow_walk_recheck_outcome",
            "domain47_prewarm_shadow_walk_sweep_only_char_id",
            "domain47_prewarm_shadow_walk_sweep_only_map",
            "domain47_prewarm_shadow_walk_recheck_hit_count",
            "domain47_prewarm_shadow_walk_recheck_miss_count",
            "domain47_prewarm_shadow_walk_recheck_refused_count",
        ] {
            assert!(
                DOMAIN47_PREWARM_UNITS.iter().any(|(unit, _)| *unit == name),
                "{name} must be declared IN the table; a class it happens to \
                 reach through a suffix rule is the accident this family's \
                 table exists to end"
            );
        }
        let outcome = unit_accumulation("domain47_prewarm_shadow_walk_recheck_outcome");
        assert_eq!(outcome, Accumulation::LastValue);
        let mut summed: Option<u64> = None;
        for _ in 0..4 {
            summed = Some(Accumulation::PerFrame.fold(summed, 1));
        }
        assert_eq!(
            summed,
            Some(4),
            "the default would publish four reached-on-recheck frames as the \
             code for an uncensused owner"
        );

        let axis = unit_accumulation("domain47_prewarm_shadow_walk_sweep_only_map");
        assert_eq!(axis, Accumulation::LastValue);
        assert_eq!(
            Accumulation::BitwiseOr.fold(Some(1), 2),
            3,
            "the `_map` suffix would OR the two axis codes into a third that \
             names neither map"
        );

        assert_eq!(
            unit_accumulation("domain47_prewarm_shadow_walk_sweep_only_char_id"),
            Accumulation::LastValue
        );

        for count in [
            "domain47_prewarm_shadow_walk_recheck_hit_count",
            "domain47_prewarm_shadow_walk_recheck_miss_count",
            "domain47_prewarm_shadow_walk_recheck_refused_count",
        ] {
            assert_eq!(
                unit_accumulation(count),
                Accumulation::PerFrame,
                "{count} counts elements, and the capture-wide question is how \
                 many residual events of each kind the run produced"
            );
        }
        assert_eq!(
            Accumulation::PerFrame.fold(Some(2), 3),
            5,
            "which is the arithmetic that answers it"
        );
    }

    #[test]
    fn the_d47_sweep_only_location_units_are_all_last_value() {
        const LOCATION_UNITS: &[&str] = &[
            "domain47_prewarm_shadow_walk_sweep_only_address",
            "domain47_prewarm_shadow_walk_sweep_only_owner_address",
            "domain47_prewarm_shadow_walk_sweep_only_owner_censused",
            "domain47_prewarm_shadow_walk_sweep_only_owner_char_id",
            "domain47_prewarm_shadow_walk_sweep_only_owner_map",
            "domain47_prewarm_shadow_walk_sweep_only_owner_map_size",
            "domain47_prewarm_shadow_walk_sweep_only_candidate_bodies",
            "domain47_prewarm_shadow_walk_sweep_only_candidate_entries",
        ];
        for name in LOCATION_UNITS {
            assert!(
                DOMAIN47_PREWARM_UNITS.iter().any(|(unit, _)| unit == name),
                "{name} must be declared IN the table; five of these eight reach \
                 no suffix rule and would be SUMMED"
            );
            assert_eq!(
                unit_accumulation(name),
                Accumulation::LastValue,
                "{name} describes ONE element and arithmetic over frames on it \
                 has no referent"
            );
        }
        assert_eq!(
            Accumulation::PerFrame.fold(Some(1), 1),
            2,
            "the summed default turns two Holds into a CleanWithout"
        );
        assert_eq!(
            Accumulation::LastValue.fold(Some(1), 1),
            1,
            "which is why the class is LastValue"
        );
    }

    #[test]
    fn the_d47_population_comparison_pairs_share_an_accumulation_class() {
        for (shadow, production) in [
            (
                "domain47_prewarm_population_ab_sweep_leader_roots",
                "domain47_prewarm_population_ab_walk_leader_roots",
            ),
            (
                "domain47_prewarm_population_ab_sweep_advisor_roots",
                "domain47_prewarm_population_ab_walk_advisor_roots",
            ),
            (
                "domain47_prewarm_population_ab_walk_leader_roots",
                "domain47_prewarm_country_leader_role_count",
            ),
            (
                "domain47_prewarm_population_ab_walk_advisor_roots",
                "domain47_prewarm_advisor_root_count",
            ),
        ] {
            let shadow_class = declared_unit_accumulation(shadow);
            assert_eq!(
                shadow_class,
                declared_unit_accumulation(production),
                "{shadow} and {production} are the two halves of one population \
                 comparison and must accumulate identically"
            );
            assert_eq!(
                shadow_class,
                Some(Accumulation::LastValue),
                "{shadow} is a GAUGE over a retained root cache, not a per-frame \
                 event count"
            );
        }
    }

    #[test]
    fn every_d47_prewarm_unit_declares_its_class() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (name, expected) in DOMAIN47_PREWARM_UNITS {
            assert!(
                name.starts_with("domain47_prewarm_"),
                "{name} is not a member of the family this table declares"
            );
            assert!(seen.insert(name), "{name} is listed twice");
            assert_eq!(
                declared_unit_accumulation(name),
                Some(*expected),
                "{name} does not resolve to the class its own table gives it"
            );
        }
        assert_eq!(
            declared_unit_accumulation("domain47_prewarm_families_dark_without_reason"),
            Some(Accumulation::BitwiseOr),
            "the table must win over the `_reason` suffix for this name"
        );
    }

    #[test]
    fn a_defaulted_unit_is_distinguishable_from_a_declared_one() {
        assert_eq!(
            declared_unit_accumulation("weather_rows_total"),
            None,
            "nothing classifies this name; it is summed by default"
        );
        assert_eq!(
            unit_accumulation("weather_rows_total"),
            Accumulation::PerFrame
        );
        assert_eq!(
            declared_unit_accumulation("domain47_prewarm_ran"),
            Some(Accumulation::PerFrame),
            "declared per-frame, which is not the same statement as defaulted"
        );
        for name in [
            "weather_rows_total",
            "domain47_prewarm_ran",
            "domain47_prewarm_known_families",
            "rs_observations",
            "d69_stale_slot_mask",
            "convoy_transport_attack_something_new",
        ] {
            assert_eq!(
                unit_accumulation(name),
                declared_unit_accumulation(name).unwrap_or(Accumulation::PerFrame),
                "{name}"
            );
        }
    }

    #[test]
    fn the_classification_tables_are_disjoint_and_self_consistent() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (name, _) in DOMAIN47_PREWARM_UNITS {
            assert!(
                seen.insert(name),
                "{name} is classified by more than one table"
            );
        }
        for (table, expected) in [
            (CUMULATIVE_UNITS, Accumulation::Cumulative),
            (GAUGE_UNITS, Accumulation::Gauge),
            (ZERO_GAUGE_UNITS, Accumulation::Gauge),
            (LAST_VALUE_UNITS, Accumulation::LastValue),
            (RAW_FAILURE_CONTEXT_UNITS, Accumulation::LastValue),
            (STATEFUL_FAMILY_PER_FRAME_UNITS, Accumulation::PerFrame),
        ] {
            for name in table {
                assert!(
                    seen.insert(name),
                    "{name} is classified by more than one table"
                );
                assert_eq!(
                    unit_accumulation(name),
                    expected,
                    "{name} does not resolve to the table that lists it"
                );
                assert!(
                    !RAW_WORD_SUFFIXES.iter().any(|s| name.ends_with(s))
                        && !BITMASK_SUFFIXES.iter().any(|s| name.ends_with(s)),
                    "{name} would be captured by a suffix rule before its table, so \
                     its table entry is either dead or contradicted"
                );
            }
        }
    }

    #[test]
    fn a_raw_observation_is_never_summed() {
        const T15: &[(&str, u64, u64, u64)] = &[
            (
                "d43_resource_failure_resource_index",
                1_550_483_193_495,
                361,
                0xFFFF_FFFF,
            ),
            (
                "d44_country_supply_slot_index",
                30_064_771_065,
                7,
                0xFFFF_FFFF,
            ),
            (
                "d44_country_supply_tag_db_id",
                30_064_771_065,
                7,
                0xFFFF_FFFF,
            ),
            ("d41_lifecycle_failure_stage", 180_063, 351, 513),
            ("d43_classic_failure_stage", 74_366, 361, 206),
            ("d43_resource_failure_stage", 37_544, 361, 104),
        ];
        for (name, published, frames, value) in T15 {
            let class = unit_accumulation(name);
            assert_eq!(
                class,
                Accumulation::LastValue,
                "{name} is a raw observation and must not be aggregated arithmetically"
            );
            let mut running: Option<u64> = None;
            for _ in 0..*frames {
                running = Some(class.fold(running, *value));
            }
            assert_eq!(running, Some(*value), "{name}");
            assert_eq!(
                value * frames,
                *published,
                "{name}: the published figure must be exactly the sum this replaces"
            );
        }

        for name in [
            "h_convoy_resolver_error_hash",
            "d93_fail_reason_hash",
            "d64_first_rejected_timed_effect_class_fnv1a64",
            "d64_first_rejected_timed_effect_vtable",
            "d64_first_rejected_timed_effect_address",
        ] {
            assert_eq!(unit_accumulation(name), Accumulation::LastValue, "{name}");
        }
        let hash = 0x9E37_79B9_7F4A_7C15u64;
        let mut summed = 0u64;
        for _ in 0..396 {
            summed = summed.saturating_add(hash);
        }
        assert_eq!(
            summed,
            u64::MAX,
            "which is what t15 published for three hashes"
        );
        assert_eq!(Accumulation::LastValue.fold(Some(hash), hash), hash);
    }

    #[test]
    fn accumulation_discriminants_are_pinned_and_round_trip() {
        assert_eq!(
            [
                Accumulation::PerFrame.code(),
                Accumulation::Cumulative.code(),
                Accumulation::Gauge.code(),
                Accumulation::LastValue.code(),
                Accumulation::BitwiseOr.code(),
                Accumulation::Undeclared.code(),
            ],
            [1, 2, 3, 4, 5, 6]
        );
        for class in Accumulation::ALL {
            assert_ne!(class.code(), 0);
            assert_eq!(Accumulation::from_code(class.code()), Some(class));
        }
        assert_eq!(Accumulation::from_code(0), None);
        assert_eq!(Accumulation::from_code(7), None);
        for class in Accumulation::ALL {
            assert_eq!(class.fold(None, 42), 42);
        }
    }

    #[test]
    fn outside_build_segment_codes_index_their_own_unit_names() {
        assert_eq!(
            OUTSIDE_BUILD_SEGMENTS.len(),
            OUTSIDE_BUILD_UNIT_NAMES.len(),
            "every segment needs exactly one wire name"
        );
        assert_eq!(OUTSIDE_BUILD_SEGMENT_COUNT, OUTSIDE_BUILD_SEGMENTS.len());
        let mut seen = BTreeSet::new();
        for (index, (code, name)) in OUTSIDE_BUILD_SEGMENTS.iter().enumerate() {
            assert_eq!(
                usize::from(*code),
                index,
                "segment {name} sits at index {index} but claims code {code}; the                  host indexes the name table by code"
            );
            assert_eq!(
                OUTSIDE_BUILD_UNIT_NAMES[index],
                format!("lw_o{code}_us"),
                "the wire name for segment {name} must be derivable from its code"
            );
            assert!(seen.insert(*name), "segment name {name} is used twice");
            assert_eq!(outside_build_segment_name(*code), Some(*name));
        }
        assert_eq!(
            outside_build_segment_name(OUTSIDE_BUILD_SEGMENT_COUNT as u16),
            None,
            "a code this build does not know must resolve to None, not to a              neighbouring segment's name"
        );
    }

    #[test]
    fn every_outside_build_unit_is_a_per_frame_duration() {
        for name in OUTSIDE_BUILD_UNIT_NAMES {
            assert_eq!(
                unit_accumulation(name),
                Accumulation::PerFrame,
                "{name} is a per-frame duration"
            );
            assert!(
                !is_stateful_unit_family(name),
                "{name} must not fall inside a stateful family, where silence                  means undeclared"
            );
        }
    }

    #[test]
    fn frame_tail_segment_codes_index_their_own_unit_names() {
        assert_eq!(
            FRAME_TAIL_SEGMENTS.len(),
            FRAME_TAIL_UNIT_NAMES.len(),
            "every segment needs exactly one wire name"
        );
        assert_eq!(FRAME_TAIL_SEGMENT_COUNT, FRAME_TAIL_SEGMENTS.len());
        let mut seen = BTreeSet::new();
        for (index, (code, name)) in FRAME_TAIL_SEGMENTS.iter().enumerate() {
            assert_eq!(
                usize::from(*code),
                index,
                "segment {name} sits at index {index} but claims code {code}; the \
                 host indexes the name table by code"
            );
            assert_eq!(
                FRAME_TAIL_UNIT_NAMES[index],
                format!("lw_t{code}_us"),
                "the wire name for segment {name} must be derivable from its code"
            );
            assert!(seen.insert(*name), "segment name {name} is used twice");
            assert_eq!(frame_tail_segment_name(*code), Some(*name));
        }
        assert_eq!(
            frame_tail_segment_name(FRAME_TAIL_SEGMENT_COUNT as u16),
            None,
            "a code this build does not know must resolve to None, not to a \
             neighbouring segment's name"
        );
    }

    #[test]
    fn every_frame_tail_unit_is_a_per_frame_duration() {
        for name in FRAME_TAIL_UNIT_NAMES.iter().chain(["lw_ft_us"].iter()) {
            assert_eq!(
                unit_accumulation(name),
                Accumulation::PerFrame,
                "{name} is a per-frame duration"
            );
            assert!(
                !is_stateful_unit_family(name),
                "{name} must not fall inside a stateful family, where silence \
                 means undeclared"
            );
        }
    }

    /// The frame-tail names must not collide with the outside-build names, and
    /// neither family may absorb the other's codes: both are decoded by the
    /// host from a one-letter prefix and a number, so `lw_t1_us` and `lw_o1_us`
    /// differ by exactly one character and a mistake here would silently fold
    /// tail time into the sweep-loop overhead the frame is measured against.
    #[test]
    fn frame_tail_and_outside_build_wire_names_are_disjoint() {
        let outside: BTreeSet<&str> = OUTSIDE_BUILD_UNIT_NAMES.iter().copied().collect();
        for name in FRAME_TAIL_UNIT_NAMES {
            assert!(
                !outside.contains(name),
                "{name} is claimed by both the frame-tail and outside-build tables"
            );
        }
    }
}
