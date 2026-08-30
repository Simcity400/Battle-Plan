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

crate::wire_coded_variants! {
    UnitRole: u8, all = ALL, code = code, from_code = from_code, label = label;

    codes {
        Deciding = 1 => "deciding",
        Locator = 2 => "locator",
        Context = 3 => "context",
        Counter = 4 => "counter",
        Undeclared = 5 => "undeclared",
    }

    reserved {
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

crate::wire_coded_variants! {
    UnitPresence: u8, all = ALL, code = code, from_code = from_code, label = label;

    codes {
        Observed = 1 => "observed",
        Absent = 2 => "absent",
        ResetSentinel = 3 => "sentinel",
        NotApplicable = 4 => "n/a",
        Undeclared = 5 => "undeclared",
    }

    reserved {
    }
}

impl UnitPresence {
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

crate::wire_coded_codes! {
    payloads Authority: u16,
    code = pub kind_code,
    Ok = from_kind_code(kind) -> Result<Self, String> {
        Err(format!("authority kind {kind} is not defined"))
    };

    SelfDomain = 1,
    Host = 4,
    OffFrameWorker = 5,
    ; tuple
    OtherDomain(domain: u16) = 2,
    ; named
    Engine { rva: u64 } = 3,
    ; never
    Undeclared = 0,
}

impl Authority {
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
        Self::from_kind_code(kind, domain, rva)
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

crate::wire_coded_variants! {
    OffFrameTerminal: u8, all = ALL, code = code, from_code = from_code, label = label;

    codes {
        Running = 1 => "running",
        CompletedOk = 2 => "completed_ok",
        CompletedError = 3 => "completed_error",
        Panicked = 4 => "panicked",
    }

    reserved {
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

crate::wire_coded_variants! {
    TruncationKind: u8, all = ALL, code = code, from_code = from_code, label = label;

    codes {
        None = 1 => "complete",
        BudgetShare = 2 => "budget_share",
        UnitTooLarge = 3 => "unit_too_large",
        ProducerOverflow = 4 => "producer_overflow",
    }

    reserved {
        Undeclared = 0 => "undeclared",
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

crate::wire_coded_variants! {
    Accumulation: u8, all = ALL, code = code, from_code = from_code, label = label;

    codes {
        PerFrame = 1 => "per_frame",
        Cumulative = 2 => "cumulative",
        Gauge = 3 => "gauge",
        LastValue = 4 => "last_value",
        BitwiseOr = 5 => "bitwise_or",
        Undeclared = 6 => "undeclared",
    }

    reserved {
    }
}

impl Accumulation {

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
    // `wire_counters!` producer families. Declared stateful so a name added
    // beside them is `Undeclared` and refused, rather than defaulting to
    // per-frame and being summed across the hours of a hook lifetime total.
    "d79_hook_",
    "d87_hook_",
    "d72_hook_",
    "d78_hook_",
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
    "convoy_transport_attack_dropped_capture_error_invalid_endpoint",
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
    // The frame-id allocator, monotone for the process lifetime like every
    // other next_* below it.
    "d95_next_sequence",
    // `wire_counters!` producer families: hook-lifetime totals, and the
    // install/epoch state beside them, which is `Cumulative` for the reason
    // `d95_hooks_installed` and `d97_hook_installed` are. Both fold by maximum.
    // The instantaneous readings of the same families sit in `GAUGE_UNITS`.
    "d79_hook_callback_count",
    "d79_hook_rejected_observation_count",
    "d79_hook_next_sequence",
    "d87_hook_next_generation",
    "d78_hook_next_transition_sequence",
    "d87_hook_installed",
    "d87_hook_epoch_open",
    "d87_hook_pending_receipt",
    "d87_hook_authenticated_snapshot",
    "d87_hook_materializer_calls",
    "d87_hook_insertion_calls",
    "d87_hook_text_events",
    "d87_hook_can_take_calls",
    "d87_hook_timeout_calls",
    "d87_hook_current_error",
    "d72_hook_installed",
    "d72_hook_install_epoch",
    "d72_hook_callbacks_open",
    "d72_hook_controller_calls",
    "d72_hook_observation_sequence",
    "d72_hook_owner_generation",
    "d72_hook_has_observed_baseline",
    "d72_hook_poisoned",
    "d78_hook_callback_count",
    "d78_hook_latest_transition_sequence",
    "d78_hook_has_complete_snapshot",
];

/// each country's ENGINE LIFETIME total for that channel, so it is cumulative
const fn is_air_loss_channel_total(name: &str) -> bool {
    const_str_starts_with(name, "air_loss_") && const_str_ends_with(name, "_raw_sum")
}

const fn is_digest_word_or_group(name: &str) -> bool {
    if const_str_eq(name, "d75_terminal_payload")
        || const_str_eq(name, "d64_terminal_envelope")
        || const_str_eq(name, "d64_terminal_del2")
    {
        return true;
    }
    let bytes = name.as_bytes();
    let marker = b"_sha256_";
    // The LAST occurrence, mirroring `rsplit_once`.
    let mut split = usize::MAX;
    let mut start = 0;
    while start + marker.len() <= bytes.len() {
        let mut k = 0;
        let mut hit = true;
        while k < marker.len() {
            if bytes[start + k] != marker[k] {
                hit = false;
                break;
            }
            k += 1;
        }
        if hit {
            split = start;
        }
        start += 1;
    }
    if split == usize::MAX || split == 0 || split + marker.len() == bytes.len() {
        return false;
    }
    let mut i = split + marker.len();
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
        i += 1;
    }
    true
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
    // Depth at the instant the frame read the hook, not a total since install.
    "d79_hook_authenticated_surface_count",
    "d72_hook_in_flight_callbacks",
    "d79_hook_in_flight_callbacks",
    "d87_hook_in_flight_callbacks",
    "d78_hook_in_flight_callbacks",
];

/// The segments of the time the build was NOT running, named.
///
/// The collector charges its accumulator by this type; the discriminant is the
/// wire code and the slot the span is published in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum OutsideBuildSegment {
    Total = 0,
    Wait = 1,
    Identity = 2,
    Prepoll = 3,
    Prepub = 4,
    Complete = 5,
    DegNotice = 6,
    DegLog = 7,
    OpenPub = 8,
    A30Insert = 9,
    HookStage = 10,
    TailInsert = 11,
    BuildTail = 12,
}

crate::cause_tables::ordered_segments! {
    OutsideBuildSegment,
    segments = pub OUTSIDE_BUILD_SEGMENTS,
    unit_names = pub OUTSIDE_BUILD_UNIT_NAMES,
    count = pub OUTSIDE_BUILD_SEGMENT_COUNT,
    unit_prefix = pub OUTSIDE_BUILD_UNIT_PREFIX = "lw_o",
    unit_suffix = pub OUTSIDE_BUILD_UNIT_SUFFIX = "_us";

    Total = 0 => "total",
    Wait = 1 => "wait",
    Identity = 2 => "identity",
    Prepoll = 3 => "prepoll",
    Prepub = 4 => "prepub",
    Complete = 5 => "complete",
    DegNotice = 6 => "degnotice",
    DegLog = 7 => "deglog",
    OpenPub = 8 => "openpub",
    A30Insert = 9 => "a30insert",
    HookStage = 10 => "hookstage",
    TailInsert = 11 => "tailinsert",
    BuildTail = 12 => "buildtail_previous_frame",
}

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
/// The frame tail's segments, named. The discriminant is the wire code, the
/// slot the span is published in, and the bit the collector sets in
/// [`FRAME_TAIL_MARKS_UNIT`] when it closes the segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum FrameTailSegment {
    /// The post-join encodes of the derived and presentation domains, which
    /// take no reader permit and run on the build thread after the fan-out.
    DerivedEncode = 0,
    /// The country bundle's lane attribution plus the drain that publishes its
    /// eight sub-reads.
    CountryPublish = 1,
    /// The publish blocks from D45's depth record to just before D81.
    PublishEarly = 2,
    /// D81 rail-replay-state's publish block, alone in its range.
    D81Publish = 3,
    /// The unit-shaped publish blocks: divisions, orders, ships, air.
    PublishUnits = 4,
    /// D47 characters/leaders/cabinet's publish block, alone in its range.
    D47Publish = 5,
    /// Everything from the air-variant reserve block to the frame-time close.
    PublishLate = 6,
}

crate::cause_tables::ordered_segments! {
    FrameTailSegment,
    segments = pub FRAME_TAIL_SEGMENTS,
    unit_names = pub FRAME_TAIL_UNIT_NAMES,
    count = pub FRAME_TAIL_SEGMENT_COUNT,
    unit_prefix = pub FRAME_TAIL_UNIT_PREFIX = "lw_t",
    unit_suffix = pub FRAME_TAIL_UNIT_SUFFIX = "_us";

    DerivedEncode = 0 => "derived_encode",
    CountryPublish = 1 => "country_publish",
    PublishEarly = 2 => "publish_early",
    D81Publish = 3 => "d81_publish",
    PublishUnits = 4 => "publish_units",
    D47Publish = 5 => "d47_publish",
    PublishLate = 6 => "publish_late",
}

/// EVERY numbered wire-unit family in this namespace, `(prefix, suffix)`.
///
/// The two segment tables were not the only families spelling a name from a
/// prefix, a number and a suffix: the collector builds six more --
/// `lw_l{lane}_wall_us`, `lw_l{lane}_busy_us`, `lw_d{tag}_crit_us`,
/// `lw_d{tag}_inb_us`, `lw_d{tag}_post_us`, `lw_cb{tag}_us` -- and each was
/// spelled from its own literal there. Guarding only the two tables against
/// each other left the rest open: giving the outside-build table the lane
/// family's prefix and suffix compiled, and the unchanged `lw_l3_wall_us`
/// decoded as outside-build code 3 (Codex round 34). Declared once here, so
/// the collector spells its names from these and the assertion below can see
/// the whole namespace.
pub const LANE_WALL_UNIT_PREFIX: &str = "lw_l";
pub const LANE_WALL_UNIT_SUFFIX: &str = "_wall_us";
pub const LANE_BUSY_UNIT_SUFFIX: &str = "_busy_us";
pub const DOMAIN_CRIT_UNIT_PREFIX: &str = "lw_d";
pub const DOMAIN_CRIT_UNIT_SUFFIX: &str = "_crit_us";
pub const DOMAIN_INBOUND_UNIT_SUFFIX: &str = "_inb_us";
pub const DOMAIN_POST_UNIT_SUFFIX: &str = "_post_us";
pub const CALLBACK_UNIT_PREFIX: &str = "lw_cb";
pub const CALLBACK_UNIT_SUFFIX: &str = "_us";

/// Two numbered families ALIAS if one family's spelling can also be read as
/// the other's: equal prefixes, or one prefix extending the other by DIGITS.
///
/// `lw_l` and `lw_o` cannot alias -- the character after `lw_` differs and is
/// not a digit either way. `lw_l` and `lw_l` do, whatever their suffixes,
/// because a number sits between and can absorb the difference.
pub const fn const_families_cannot_alias(families: &[(&str, &str)]) -> bool {
    let mut i = 0;
    while i < families.len() {
        let mut j = i + 1;
        while j < families.len() {
            let (a, b) = (families[i].0, families[j].0);
            if const_str_eq(a, b) {
                return false;
            }
            if const_str_starts_with(b, a) && const_extends_by_digit(b, a) {
                return false;
            }
            if const_str_starts_with(a, b) && const_extends_by_digit(a, b) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// `longer` continues `shorter` with an ASCII digit.
const fn const_extends_by_digit(longer: &str, shorter: &str) -> bool {
    let (l, s) = (longer.as_bytes(), shorter.as_bytes());
    if l.len() <= s.len() {
        return false;
    }
    l[s.len()] >= b'0' && l[s.len()] <= b'9'
}

/// Both tables spell their unit names from a one-letter prefix and a number,
/// so `lw_t1_us` and `lw_o1_us` differ by one character. A `unit_prefix` edit
/// that made them collide would fold tail time into the sweep-loop overhead
/// the frame is measured against, and the record carries no second copy of
/// either to disagree. Refused where the edit is written.
const _: () = assert!(
    const_no_name_is_shared(OUTSIDE_BUILD_UNIT_NAMES, FRAME_TAIL_UNIT_NAMES),
    "the frame-tail and outside-build tables claim the same wire unit name",
);

/// The WHOLE namespace, checked against itself.
pub const WIRE_UNIT_FAMILIES: &[(&str, &str)] = &[
    (OUTSIDE_BUILD_UNIT_PREFIX, OUTSIDE_BUILD_UNIT_SUFFIX),
    (FRAME_TAIL_UNIT_PREFIX, FRAME_TAIL_UNIT_SUFFIX),
    (LANE_WALL_UNIT_PREFIX, LANE_WALL_UNIT_SUFFIX),
    (LANE_WALL_UNIT_PREFIX, LANE_BUSY_UNIT_SUFFIX),
    (DOMAIN_CRIT_UNIT_PREFIX, DOMAIN_CRIT_UNIT_SUFFIX),
    (DOMAIN_CRIT_UNIT_PREFIX, DOMAIN_INBOUND_UNIT_SUFFIX),
    (DOMAIN_CRIT_UNIT_PREFIX, DOMAIN_POST_UNIT_SUFFIX),
    (CALLBACK_UNIT_PREFIX, CALLBACK_UNIT_SUFFIX),
];

const _: () = assert!(
    const_families_cannot_alias(&[
        (OUTSIDE_BUILD_UNIT_PREFIX, OUTSIDE_BUILD_UNIT_SUFFIX),
        (FRAME_TAIL_UNIT_PREFIX, FRAME_TAIL_UNIT_SUFFIX),
        (LANE_WALL_UNIT_PREFIX, LANE_WALL_UNIT_SUFFIX),
        (DOMAIN_CRIT_UNIT_PREFIX, DOMAIN_CRIT_UNIT_SUFFIX),
        (CALLBACK_UNIT_PREFIX, CALLBACK_UNIT_SUFFIX),
    ]),
    "two numbered wire-unit families can spell the same name, so one      family's span would decode as another's code",
);

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

crate::cause_tables::millisecond_buckets! {
    bounds = pub POLL_GAP_BUCKET_MAX_US, names = pub POLL_GAP_BUCKET_UNIT_NAMES,
    unit_prefix = "lw_poll_gap_";

    1, 2, 4, 8, 16, 32, last 64,
}

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

/// Names classified per frame BY NAME rather than by falling through to the
/// per-frame default: a name inside a stateful family, which would otherwise be
/// `Undeclared`, and a name a `wire_counters!` producer publishes, which the
/// macro refuses unless a table here names its class.
pub const EXPLICIT_PER_FRAME_UNITS: &[&str] = &[
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
    "convoy_transport_attack_row_route_refused_delivery_route_unreadable",
    "convoy_transport_attack_row_route_refused_delivery_route_vtable_mismatch",
    "convoy_transport_attack_row_route_refused_delivery_route_absent",
    "convoy_transport_attack_row_route_refused_delivery_route_endpoint_out_of_range",
    "convoy_transport_attack_row_route_refused_delivery_route_unstable",
    // Sinks RECORDED with their COMBAT gap named (lane TS), the same unit and
    // the same partition one attribute over.
    "convoy_transport_attack_rows_combat_refused",
    "convoy_transport_attack_row_combat_refused_member_absent_from_epoch_snapshot",
    "convoy_transport_attack_row_combat_refused_battle_slot_mismatch",
    "convoy_transport_attack_row_combat_refused_invalid_epoch",
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
    // d102's walk counters. Each is a count of what THIS hour's walk dropped,
    // so summing the hours of a run is the reading, exactly as the per-frame
    // default already gave them before the macro required the class be said.
    "fog_derivation_inputs_countries_walked",
    "fog_derivation_inputs_units_without_handle",
    "fog_derivation_inputs_viewer_levels_non_positive",
    "fog_derivation_inputs_units_undispatched_class",
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

pub const fn unit_accumulation(name: &str) -> Accumulation {
    match declared_unit_accumulation(name) {
        Some(class) => class,
        None => Accumulation::PerFrame,
    }
}

/// CONST so a producer can assert its own leaf spellings against it.
///
/// The alternative was a second const-evaluable copy of these rules, which is
/// the shape this lane exists to remove: two readings of one law that can
/// disagree. The body is the same law in the same order, written with index
/// loops because iterators are not available in a const fn.
pub const fn declared_unit_accumulation(name: &str) -> Option<Accumulation> {
    let mut i = 0;
    while i < DOMAIN47_PREWARM_UNITS.len() {
        if const_str_eq(DOMAIN47_PREWARM_UNITS[i].0, name) {
            return Some(DOMAIN47_PREWARM_UNITS[i].1);
        }
        i += 1;
    }
    if const_ends_with_any(name, RAW_WORD_SUFFIXES) {
        return Some(Accumulation::LastValue);
    }
    if const_ends_with_any(name, BITMASK_SUFFIXES) {
        return Some(Accumulation::BitwiseOr);
    }
    if const_str_starts_with(name, "d69_failed_slot_mask_") {
        return Some(Accumulation::BitwiseOr);
    }
    if is_digest_word_or_group(name) {
        return Some(Accumulation::LastValue);
    }
    if const_names_contain(CUMULATIVE_UNITS, name)
        || const_names_contain(LANE_WALL_SESSION_CONSTANTS, name)
        || is_air_loss_channel_total(name)
    {
        return Some(Accumulation::Cumulative);
    }
    if const_names_contain(GAUGE_UNITS, name) || const_names_contain(ZERO_GAUGE_UNITS, name) {
        return Some(Accumulation::Gauge);
    }
    if const_names_contain(LAST_VALUE_UNITS, name)
        || const_names_contain(RAW_FAILURE_CONTEXT_UNITS, name)
    {
        return Some(Accumulation::LastValue);
    }
    if const_names_contain(EXPLICIT_PER_FRAME_UNITS, name) {
        return Some(Accumulation::PerFrame);
    }
    if const_starts_with_any(name, STATEFUL_UNIT_PREFIXES) {
        return Some(Accumulation::Undeclared);
    }
    None
}

/// `declared_unit_accumulation(name) == Some(Accumulation::Cumulative)`.
pub const fn declares_cumulative_unit(name: &str) -> bool {
    matches!(
        declared_unit_accumulation(name),
        Some(Accumulation::Cumulative)
    )
}

/// `unit_accumulation(name) == Accumulation::PerFrame`.
///
/// A name that falls off the end of every class INSIDE a stateful family is
/// `Undeclared` rather than per-frame, and fails the producer sweep; this says
/// so, because the two are the same answer only outside such a family.
pub const fn declares_per_frame_unit(name: &str) -> bool {
    matches!(unit_accumulation(name), Accumulation::PerFrame)
}

/// A table in this module names the class, rather than the caller falling
/// through to the per-frame default.
pub const fn declares_explicit_unit_accumulation(name: &str) -> bool {
    declared_unit_accumulation(name).is_some()
}

pub fn is_stateful_unit_family(name: &str) -> bool {
    STATEFUL_UNIT_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

pub const fn const_str_eq(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0;
    while i < left.len() {
        if left[i] != right[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `const_str_eq(name, concat!(prefix, suffix))` without building the joined
/// string: `concat!` needs literals and a producer's leaf spelling is usually a
/// const it must be checked against.
pub const fn const_str_is_prefixed(name: &str, prefix: &str, suffix: &str) -> bool {
    let (name, prefix, suffix) = (name.as_bytes(), prefix.as_bytes(), suffix.as_bytes());
    if name.len() != prefix.len() + suffix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if name[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    let mut j = 0;
    while j < suffix.len() {
        if name[prefix.len() + j] != suffix[j] {
            return false;
        }
        j += 1;
    }
    true
}

const fn const_str_ends_with(name: &str, suffix: &str) -> bool {
    let (name, suffix) = (name.as_bytes(), suffix.as_bytes());
    if name.len() < suffix.len() {
        return false;
    }
    let offset = name.len() - suffix.len();
    let mut i = 0;
    while i < suffix.len() {
        if name[offset + i] != suffix[i] {
            return false;
        }
        i += 1;
    }
    true
}

pub const fn const_names_contain(names: &[&str], name: &str) -> bool {
    let mut i = 0;
    while i < names.len() {
        if const_str_eq(names[i], name) {
            return true;
        }
        i += 1;
    }
    false
}

const fn const_str_starts_with(name: &str, prefix: &str) -> bool {
    let (name, prefix) = (name.as_bytes(), prefix.as_bytes());
    if name.len() < prefix.len() {
        return false;
    }
    let mut i = 0;
    while i < prefix.len() {
        if name[i] != prefix[i] {
            return false;
        }
        i += 1;
    }
    true
}

const fn const_ends_with_any(name: &str, suffixes: &[&str]) -> bool {
    let mut i = 0;
    while i < suffixes.len() {
        if const_str_ends_with(name, suffixes[i]) {
            return true;
        }
        i += 1;
    }
    false
}

const fn const_starts_with_any(name: &str, prefixes: &[&str]) -> bool {
    let mut i = 0;
    while i < prefixes.len() {
        if const_str_starts_with(name, prefixes[i]) {
            return true;
        }
        i += 1;
    }
    false
}

/// No two entries of `names` are the same string.
///
/// The half a bijection over INDICES cannot see: distinct slots carrying one
/// name are summed together by a manifest that aggregates by name, so one
/// cause's count silently absorbs another's.
pub const fn const_names_are_distinct(names: &[&str]) -> bool {
    let mut i = 0;
    while i < names.len() {
        let mut j = i + 1;
        while j < names.len() {
            if const_str_eq(names[i], names[j]) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// `name` SPELLS `variant`: the same letters in the same order, word breaks
/// written as underscores, and any further word separated by one.
///
/// What binds a published name to the thing it names. A table of codes, names
/// and variants can assert that each code is its own discriminant, that the
/// codes are a bijection and that the names are distinct, and still let two
/// `=> "name"` literals be exchanged: every assertion holds and each segment's
/// span is published under the other's key.
pub const fn const_name_spells_the_variant(name: &str, variant: &str) -> bool {
    let (name, variant) = (name.as_bytes(), variant.as_bytes());
    let (mut i, mut j) = (0, 0);
    while j < variant.len() {
        if i >= name.len() {
            return false;
        }
        if name[i] == b'_' {
            i += 1;
            continue;
        }
        let lowered = if variant[j] >= b'A' && variant[j] <= b'Z' {
            variant[j] + 32
        } else {
            variant[j]
        };
        if name[i] != lowered {
            return false;
        }
        i += 1;
        j += 1;
    }
    i == name.len() || name[i] == b'_'
}

/// `left` and `right` share no entry.
pub const fn const_no_name_is_shared(left: &[&str], right: &[&str]) -> bool {
    let mut i = 0;
    while i < left.len() {
        let mut j = 0;
        while j < right.len() {
            if const_str_eq(left[i], right[j]) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// No spelling in `names` is a prefix of another, itself included.
///
/// What closes the one gap [`const_name_spells_the_variant`] leaves. It admits
/// a name that carries further words, so two names COULD still be exchanged if
/// one variant's spelling were a prefix of the other's -- each would go on
/// spelling its new variant with the remainder read as a further word. Refuse
/// the prefix and the exchange has nowhere to hide.
pub const fn const_no_spelling_is_a_prefix_of_another(names: &[&str]) -> bool {
    let mut i = 0;
    while i < names.len() {
        let mut j = i + 1;
        while j < names.len() {
            if const_str_starts_with(names[i], names[j])
                || const_str_starts_with(names[j], names[i])
            {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// `name` begins with `ordinal` as two zero-padded digits and an underscore.
///
/// Wire spellings that carry their own ordinal are readable in a manifest and
/// unreadable once the two disagree, which nothing else notices: the counter
/// still increments, under a number that names a different cause.
pub const fn const_str_has_ordinal_prefix(name: &str, ordinal: u32) -> bool {
    if ordinal > 99 {
        return false;
    }
    let bytes = name.as_bytes();
    bytes.len() > 3
        && bytes[0] == b'0' + (ordinal / 10) as u8
        && bytes[1] == b'0' + (ordinal % 10) as u8
        && bytes[2] == b'_'
}
/// `name` is `ordinal` in two digits, an underscore, then `tail`.
pub const fn const_str_is_ordinal_then(name: &str, ordinal: u32, tail: &str) -> bool {
    if !const_str_has_ordinal_prefix(name, ordinal) {
        return false;
    }
    let (name, tail) = (name.as_bytes(), tail.as_bytes());
    if name.len() != tail.len() + 3 {
        return false;
    }
    let mut i = 0;
    while i < tail.len() {
        if name[3 + i] != tail[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// `left` and `right` are the same string once their own prefixes are removed.
///
/// For a counter published twice under two primitives: the pair is one list
/// spelled twice, and the two spellings drifting apart renames one arm of an
/// A/B.
pub const fn const_str_tails_match(
    left: &str,
    left_prefix: &str,
    right: &str,
    right_prefix: &str,
) -> bool {
    let (l, lp) = (left.as_bytes(), left_prefix.as_bytes());
    let (r, rp) = (right.as_bytes(), right_prefix.as_bytes());
    if l.len() < lp.len() || r.len() < rp.len() || l.len() - lp.len() != r.len() - rp.len() {
        return false;
    }
    let mut i = 0;
    while i < lp.len() {
        if l[i] != lp[i] {
            return false;
        }
        i += 1;
    }
    let mut i = 0;
    while i < rp.len() {
        if r[i] != rp[i] {
            return false;
        }
        i += 1;
    }
    let mut i = 0;
    while lp.len() + i < l.len() {
        if l[lp.len() + i] != r[rp.len() + i] {
            return false;
        }
        i += 1;
    }
    true
}
/// `name` is `variant` spelled in snake case.
///
/// A wire name and the variant it names are one word in two conventions, and
/// a `match` arm can pair either variant with either word.
pub const fn const_str_is_snake_of(name: &str, variant: &str) -> bool {
    let (name, variant) = (name.as_bytes(), variant.as_bytes());
    let mut i = 0;
    let mut j = 0;
    while i < variant.len() {
        let byte = variant[i];
        if byte.is_ascii_uppercase() {
            if i > 0 {
                if j >= name.len() || name[j] != b'_' {
                    return false;
                }
                j += 1;
            }
            if j >= name.len() || name[j] != byte.to_ascii_lowercase() {
                return false;
            }
        } else {
            if j >= name.len() || name[j] != byte {
                return false;
            }
        }
        i += 1;
        j += 1;
    }
    j == name.len()
}
/// The two strings differ only in ASCII case.
///
/// For a cause whose CONSTANT is upper snake and whose wire leaf is the same
/// word in lower snake: the two are one name, and the compiler cannot lower a
/// `stringify!` to check it any other way.
pub const fn const_str_eq_ignore_ascii_case(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0;
    while i < left.len() {
        if left[i].to_ascii_lowercase() != right[i].to_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    true
}

/// `name` is `prefix`, then `first`, then an underscore, then `second`.
///
/// A cross-tabulated cell's name, checked against the row's cause and the
/// column's class without composing the string, which `concat!` cannot do
/// across two independent macro repetitions.
pub const fn const_str_is_joined(name: &str, prefix: &str, first: &str, second: &str) -> bool {
    let (n, p, a, b) = (
        name.as_bytes(),
        prefix.as_bytes(),
        first.as_bytes(),
        second.as_bytes(),
    );
    if n.len() != p.len() + a.len() + 1 + b.len() {
        return false;
    }
    let mut i = 0;
    while i < p.len() {
        if n[i] != p[i] {
            return false;
        }
        i += 1;
    }
    let mut i = 0;
    while i < a.len() {
        if n[p.len() + i] != a[i] {
            return false;
        }
        i += 1;
    }
    if n[p.len() + a.len()] != b'_' {
        return false;
    }
    let mut i = 0;
    while i < b.len() {
        if n[p.len() + a.len() + 1 + i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
/// The EMITTED table's codes are their own positions.
///
/// Over `&[(u16, &str)]` rather than over the macro's arguments, because the
/// arguments are not what ships. A macro that validates its inputs and emits
/// something else has validated nothing: rewriting the emitted array to swap
/// two names left every argument-side assertion true and published each
/// segment under the other's key.
pub const fn const_segment_codes_are_their_own_positions(
    segments: &[(u16, &str)],
) -> bool {
    let mut i = 0;
    while i < segments.len() {
        if segments[i].0 as usize != i {
            return false;
        }
        i += 1;
    }
    true
}

/// No two entries of the EMITTED table are published under one name.
pub const fn const_segment_names_are_distinct(segments: &[(u16, &str)]) -> bool {
    let mut i = 0;
    while i < segments.len() {
        let mut j = i + 1;
        while j < segments.len() {
            if const_str_eq(segments[i].1, segments[j].1) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// The EMITTED unit name for `code` is `prefix` + the code in decimal +
/// `suffix`, and nothing else.
///
/// The wire name is what the host keys the published span by, and the guards
/// all read `$segments` -- so an emitted `$units` built from the same prefix,
/// code and suffix by any other expression shipped unchallenged.
pub const fn const_unit_is_spelled(
    unit: &str,
    prefix: &str,
    suffix: &str,
    code: u16,
) -> bool {
    let (u, p, s) = (unit.as_bytes(), prefix.as_bytes(), suffix.as_bytes());
    if u.len() < p.len() + s.len() {
        return false;
    }
    let mut i = 0;
    while i < p.len() {
        if u[i] != p[i] {
            return false;
        }
        i += 1;
    }
    let mut j = 0;
    while j < s.len() {
        if u[u.len() - s.len() + j] != s[j] {
            return false;
        }
        j += 1;
    }
    let (start, stop) = (p.len(), u.len() - s.len());
    if start >= stop {
        return false;
    }
    if stop - start > 1 && u[start] == b'0' {
        return false;
    }
    let mut value: u32 = 0;
    let mut k = start;
    while k < stop {
        if u[k] < b'0' || u[k] > b'9' {
            return false;
        }
        value = value * 10 + (u[k] - b'0') as u32;
        k += 1;
    }
    value == code as u32
}

/// The EMITTED table's entry for `code` carries exactly `name`.
///
/// The direct binding between a row as written and the row that ships.
/// Spelling the variant is not enough on its own: `concat!($name, "_wrong")`
/// still spells it, because a published name is allowed to carry further
/// words -- one of them is `buildtail_previous_frame`. This is what refuses an
/// emitted table built from the rows by any expression that alters them.
pub const fn const_segment_name_is(
    segments: &[(u16, &str)],
    code: u16,
    name: &str,
) -> bool {
    let mut i = 0;
    while i < segments.len() {
        if segments[i].0 == code {
            return const_str_eq(segments[i].1, name);
        }
        i += 1;
    }
    false
}

/// The EMITTED table's entry for `code` spells `variant`.
pub const fn const_segment_spells_the_variant(
    segments: &[(u16, &str)],
    code: u16,
    variant: &str,
) -> bool {
    let mut i = 0;
    while i < segments.len() {
        if segments[i].0 == code {
            return const_name_spells_the_variant(segments[i].1, variant);
        }
        i += 1;
    }
    false
}

/// Each index is its OWN POSITION in the list, not merely present in it.
///
/// A bijection is blind to ORDER, and order is what the wire carries here: the
/// unit names are emitted in the order the rows are written and the collector
/// pairs that order with accumulator slots, so exchanging two whole rows --
/// variant, code and name together -- left every other assertion true and
/// published slot 1's span under slot 2's name.
pub const fn indices_are_their_own_positions(indices: &[usize]) -> bool {
    let mut i = 0;
    while i < indices.len() {
        if indices[i] != i {
            return false;
        }
        i += 1;
    }
    true
}

/// Every index in `indices` is below `count` and appears once, so the indices
/// and the slots `0..count` are the same set.
///
/// The compile-time half of a wire taxonomy: a variant given an index another
/// variant already holds is what makes a loss count under another cause's name,
/// and an exhaustive match cannot see it.
pub const fn indices_are_a_bijection(indices: &[usize], count: usize) -> bool {
    if indices.len() != count {
        return false;
    }
    let mut i = 0;
    while i < indices.len() {
        if indices[i] >= count {
            return false;
        }
        let mut j = i + 1;
        while j < indices.len() {
            if indices[i] == indices[j] {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The guard producers const-assert their wire taxonomies with.
    ///
    /// Nothing in the tree exercises its false arms -- a taxonomy that tripped
    /// one would not compile -- so a rubber stamp here would be invisible and
    /// would let a cause count its losses under another cause's name.
    ///
    /// MUTATION: `indices_are_a_bijection` returns `true` unconditionally.
    /// Every taxonomy still compiles and this fails on the first case.
    #[test]
    fn indices_are_a_bijection_refuses_the_collision_and_the_overrun() {
        assert!(indices_are_a_bijection(&[0, 1, 2], 3));
        assert!(indices_are_a_bijection(&[2, 0, 1], 3));
        assert!(indices_are_a_bijection(&[], 0));
        assert!(!indices_are_a_bijection(&[0, 1, 1], 3));
        assert!(!indices_are_a_bijection(&[0, 1, 3], 3));
        assert!(!indices_are_a_bijection(&[0, 1], 3));
        assert!(!indices_are_a_bijection(&[0, 1, 2], 2));
    }

    /// The other three guards producers const-assert their taxonomies with.
    ///
    /// Same reason as the bijection above: a taxonomy that tripped one of
    /// these would not compile, so nothing in the tree exercises their false
    /// arms and a rubber stamp would be invisible (Codex round 4, finding 2).
    ///
    /// MUTATION: any of the three returns `true` unconditionally. Every
    /// taxonomy still compiles -- `declares_cumulative_unit` would then admit
    /// a leaf the manifest folds per-frame, `const_names_are_distinct` two
    /// causes summed under one name, `const_str_is_prefixed` a frozen version
    /// whose slots have moved -- and this fails.
    #[test]
    fn the_taxonomy_guards_refuse_what_they_exist_to_refuse() {
        assert!(declares_cumulative_unit(
            "convoy_transport_attack_dropped_capture_error_route_join"
        ));
        assert!(!declares_cumulative_unit("a_name_no_list_declares"));
        assert!(!declares_cumulative_unit("convoy_transport_attack_remaining_events"));

        assert!(declares_per_frame_unit("a_name_no_list_declares"));
        assert!(!declares_per_frame_unit("rs_observations"));
        assert!(!declares_per_frame_unit("rs_a_stateful_name_nothing_declares"));

        assert!(const_names_are_distinct(&["one", "two", "three"]));
        assert!(const_names_are_distinct(&[]));
        assert!(!const_names_are_distinct(&["one", "two", "one"]));

        assert!(const_str_is_prefixed("d81_plan_rail_tap_reused", "d81_plan_rail_tap_", "reused"));
        assert!(!const_str_is_prefixed("d81_plan_rail_tap_reused", "d81_plan_rail_tap_", "no_memory"));
        assert!(!const_str_is_prefixed("d81_plan_rail_tap_reused_x", "d81_plan_rail_tap_", "reused"));
        assert!(!const_str_is_prefixed("other_prefix_reused", "d81_plan_rail_tap_", "reused"));

        assert!(const_str_has_ordinal_prefix("07_extent_bytes", 7));
        assert!(!const_str_has_ordinal_prefix("07_extent_bytes", 8));
        assert!(!const_str_has_ordinal_prefix("7_extent_bytes", 7));

        assert!(const_str_is_ordinal_then("05_anchor_bits", 5, "anchor_bits"));
        assert!(!const_str_is_ordinal_then("05_anchor_bits", 5, "anchor_variant"));
        assert!(!const_str_is_ordinal_then("06_anchor_bits", 5, "anchor_bits"));
        assert!(!const_str_is_ordinal_then("05_anchor_bits_x", 5, "anchor_bits"));

        assert!(const_str_tails_match("rc_seh_reads", "rc_seh_", "rc_rpm_reads", "rc_rpm_"));
        assert!(!const_str_tails_match(
            "rc_seh_reads",
            "rc_seh_",
            "rc_rpm_region_hits",
            "rc_rpm_"
        ));
        assert!(!const_str_tails_match("rc_seh_reads", "rc_rpm_", "rc_rpm_reads", "rc_rpm_"));

        assert!(const_str_eq_ignore_ascii_case("SO_D48_PARTIAL_INHERITED", "so_d48_partial_inherited"));
        assert!(!const_str_eq_ignore_ascii_case("SO_D48_PARTIAL_INHERITED", "so_d48_partial_roots_err"));

        assert!(const_str_is_snake_of("hour_label_mismatch", "HourLabelMismatch"));
        assert!(!const_str_is_snake_of("hour_label_mismatch", "HourLabelMismatched"));
        assert!(!const_str_is_snake_of("hourlabelmismatch", "HourLabelMismatch"));

        assert!(const_str_is_joined("skip_owner_absent_active", "skip_", "owner_absent", "active"));
        assert!(!const_str_is_joined(
            "skip_owner_absent_active",
            "skip_",
            "owner_absent",
            "pool_a"
        ));
        assert!(!const_str_is_joined(
            "skip_owner_absentactive",
            "skip_",
            "owner_absent",
            "active"
        ));
    }
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
            (EXPLICIT_PER_FRAME_UNITS, Accumulation::PerFrame),
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
