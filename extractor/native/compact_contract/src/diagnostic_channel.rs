//! The diagnostic channel: how a domain says WHY it refused, in a form that
//! survives the trip to the stream.
//!
//! # What went wrong with the old channel
//!
//! The channel a domain used to report a refusal was a list of
//! `(&'static str, u64)` pairs on the PERF record, rendered into one comma
//! joined string on the frame refusal record. Five separate investigations
//! were blocked by the same five properties of that shape, all of them
//! measured on `runs/postmerge_recovery_20260731/deflist-0{1,2}`:
//!
//! 1. **Truncation was untyped.** The renderer kept six units and appended the
//!    literal `...`. On the wire, `d43`, `d47`, `d53`, `d55`, `d75` and `d44`
//!    every single frame ended in `,...` -- and nothing said how many units
//!    were lost, or how many bytes, or whether the one that mattered was among
//!    them. `...` is also a legal ending for a real reason, so a consumer could
//!    not even detect the truncation reliably.
//! 2. **Which six survived was emission order.** Not importance. `d44`'s
//!    deciding units were constructed after eleven others and never appeared.
//! 3. **Zero-valued units were dropped as "this did not happen".** A domain
//!    whose failure genuinely is stage `0` therefore reported nothing at all,
//!    and the reader could not tell that from "not emitted". `d44`'s
//!    `d44_country_supply_failure_stage` and `_branch` were absent for exactly
//!    this reason.
//! 4. **Sentinels were bare integers.** `d43` and `d44` both published
//!    `0xFFFFFFFF` meaning "never written"; a lane read it as a real index.
//! 5. **A stage is not a predicate.** `d42` published
//!    `d42_region_walk_stage_sample=16`, and stage 16 covers three distinct
//!    checks (heap, vtable, owner echo); stage 18 covers seven. A refusal that
//!    names a stage cannot distinguish "wrong engine constant" from
//!    "legitimate value refused".
//!
//! # What this module guarantees instead
//!
//! * A unit carries a [`UnitRole`] and a [`UnitPresence`]. Selection under
//!   budget pressure is by ROLE, never by emission order and never by value,
//!   so a zero is carried exactly as faithfully as a 5.
//! * [`UnitPresence`] separates *observed* (the value is a measurement, and 0
//!   is a real 0), *absent* (nothing was written here), *reset sentinel* (the
//!   producer's "never written" marker, so a consumer never reads
//!   `0xFFFFFFFF` as an index), and *not applicable*.
//! * A refusal names a [`Predicate`] by a stable non-zero code, an
//!   [`Authority`] (which component actually refused -- often a DIFFERENT
//!   domain), and a [`Locator`] that is a PATH of `(ordinal, extent)` levels,
//!   so "region 41" is reported as "region 41 of 96 in container 3 of 12"
//!   instead of a bare innermost ordinal with the container overwritten.
//! * Truncation is [`TruncationAccount`]: counted units, counted bytes, and a
//!   typed [`TruncationKind`]. Never a literal `...`.
//! * [`allocate_unit_budget`] gives every offender an unconditional floor that
//!   its deciding and locator units are placed into before any discretionary
//!   byte is handed out, then shares the discretionary pool max-min fairly. A
//!   verbose domain can therefore not delete a quiet neighbour's reason, which
//!   is what happened to five domains in the captures above.
//!
//! # Stability
//!
//! Every discriminant here is a **stable ABI**: retained telemetry must stay
//! comparable across recorder builds. Discriminants are explicit, non-zero,
//! and append-only. Zero is reserved everywhere and always means "the producer
//! did not declare", which is a fact a consumer must be able to see rather
//! than a value it might mistake for a declaration.

use core::num::NonZeroU32;

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

/// What a diagnostic unit is FOR, which is what decides whether it survives a
/// budget squeeze.
///
/// The old channel had no answer to "which of these six matters", so it kept
/// the first six constructed. Roles make the answer explicit and make it the
/// producer's declaration rather than an accident of statement order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UnitRole {
    /// THE fact the refusal turns on. Never dropped: placed from the
    /// per-offender floor before any discretionary byte is allocated.
    Deciding = 1,
    /// Where the deciding fact was observed -- ordinal, extent, address,
    /// generation. Useless without the deciding unit and vice versa, so it
    /// shares the floor.
    Locator = 2,
    /// Supporting evidence that shapes the diagnosis but does not decide it.
    Context = 3,
    /// Volume and throughput counters. First to go.
    Counter = 4,
    /// The producer has not been migrated onto this channel and declared no
    /// role for this unit.
    ///
    /// This is deliberately a VALUE rather than an absence. 71 of the 73
    /// domains still build untyped `(name, value)` pairs, and a consumer that
    /// saw a defaulted `Context` could not tell a considered declaration from
    /// a legacy pair. This variant says which, and one pass over a stream
    /// counts exactly how much of the producer migration is left.
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

/// Whether a unit's value is a measurement, an absence, or a marker.
///
/// This is the field that makes `0` readable. Under the old channel a unit
/// with value 0 was deleted before it reached the wire, so "the failing stage
/// is stage 0" and "no stage was recorded" were the same observation. They are
/// not the same observation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum UnitPresence {
    /// The value is a real measurement. `0` means zero, not "missing".
    Observed = 1,
    /// The producer looked and there was nothing there. `value` carries no
    /// information and a consumer must not read it as a measurement.
    Absent = 2,
    /// The producer's reset marker (`0xFFFFFFFF`, `u64::MAX`, ...) is still in
    /// the slot: this field was never written on this pass. The raw marker is
    /// still carried in `value` so the wire stays comparable with retained
    /// telemetry, but a consumer now knows not to read it as an index.
    ResetSentinel = 3,
    /// The field does not apply to the branch that was taken. Distinct from
    /// `Absent`: nothing was expected here in the first place.
    NotApplicable = 4,
    /// The producer has not been migrated and declared nothing about this
    /// value. A consumer must NOT assume `0` means zero here -- that
    /// ambiguity is precisely what the migration removes, and pretending it is
    /// already resolved would hide the remaining work.
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

    /// Is the numeric value meaningful as a measurement?
    ///
    /// `Undeclared` answers NO on purpose: an unmigrated producer's `0` might
    /// be a measurement or might be an uninitialised slot, and a consumer that
    /// treats it as a measurement is making exactly the mistake that cost D44
    /// its diagnosis.
    pub const fn value_is_a_measurement(self) -> bool {
        matches!(self, Self::Observed)
    }
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

/// Longest unit name accepted onto the wire. Names are `&'static str` literals
/// in the producer; this bounds a pathological one, not an ordinary one.
pub const MAX_UNIT_NAME_LEN: usize = 96;

/// One diagnostic unit.
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

    /// A measured deciding fact.
    pub fn deciding(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Deciding, UnitPresence::Observed)
    }

    /// A measured locator component.
    pub fn locator(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Locator, UnitPresence::Observed)
    }

    /// A measured context fact.
    pub fn context(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Context, UnitPresence::Observed)
    }

    /// A measured counter.
    pub fn counter(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Counter, UnitPresence::Observed)
    }

    /// Encoded size of this unit inside a units blob.
    pub fn encoded_len(&self) -> usize {
        UNIT_FIXED_LEN + self.name.len()
    }

    /// Human rendering that states the presence rather than hiding it. This is
    /// the replacement for `name=value`, which could not say whether `0` was a
    /// measurement.
    pub fn render(&self) -> String {
        match self.presence {
            UnitPresence::Observed => format!("{}={}", self.name, self.value),
            UnitPresence::Absent => format!("{}=absent", self.name),
            UnitPresence::ResetSentinel => format!("{}=sentinel(0x{:x})", self.name, self.value),
            UnitPresence::NotApplicable => format!("{}=n/a", self.name),
            // Byte-for-byte what the legacy channel rendered, so retained
            // telemetry stays comparable, but the presence field beside it
            // says the producer never vouched for it.
            UnitPresence::Undeclared => format!("{}={}", self.name, self.value),
        }
    }

    /// An unmigrated `(name, value)` pair, lifted onto the channel without
    /// inventing a declaration for it.
    pub fn undeclared(name: impl Into<String>, value: u64) -> Self {
        Self::new(name, value, UnitRole::Undeclared, UnitPresence::Undeclared)
    }
}

/// `name_len u16 | role u8 | presence u8 | value u64`
pub const UNIT_FIXED_LEN: usize = 12;

/// Longest text one unit can render to: the longest legal name, `=`, and the
/// longest decimal a `u64` sentinel renders as (`=sentinel(0x` + 16 + `)`).
pub const MAX_UNIT_RENDER_LEN: usize = MAX_UNIT_NAME_LEN + 30;

/// Render units into a string that CANNOT exceed `max_len`, and say how many
/// were named.
///
/// The reason string and the units blob are not redundant, and this is the
/// seam where that gets decided. The units blob is machine-readable and four
/// times larger; the reason is what every existing consumer actually logs --
/// truncating the reason to 192 bytes plus a literal `...` is the defect that
/// blocked five lanes. So the reason must still NAME the deciding fact, and it
/// must do so without ever being able to exceed the bound its record enforces.
///
/// Two properties follow, and both are load-bearing:
///
/// * `units` arrives in role order from [`allocate_unit_budget`], and this
///   walks it in that order, so whatever is named is named deciding-first. The
///   deciding unit survives the reason bound for the same reason it survives
///   the byte budget.
/// * The return is `(text, named)`. `named` is a COUNT, not a marker, and the
///   caller puts it on the wire as a typed field. Nothing here appends `...`,
///   and nothing downstream has to infer a loss from punctuation that a real
///   reason could also end with.
///
/// A caller must leave `MAX_UNIT_RENDER_LEN` of headroom below its record's
/// bound for the first unit; `debug_assert` states that, and the release path
/// simply names zero units rather than producing an over-long string.
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

/// Encode a unit list: `count u16` then each unit.
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

/// Decode a unit list, returning the units and the number of bytes consumed.
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

// ---------------------------------------------------------------------------
// Locator
// ---------------------------------------------------------------------------

/// Longest locator level label.
pub const MAX_LOCATOR_LABEL_LEN: usize = 32;
/// Deepest locator path accepted. The deepest real walk in the tree is
/// country -> region table -> region -> side vector -> record -> damage vector
/// -> damage entry, which is 7.
pub const MAX_LOCATOR_DEPTH: usize = 8;

/// How far along a container the walk got, and how long that container is.
///
/// "Refused at region 41" is not usable. "Refused at region 41 of 96" is,
/// because 41-of-96 and 41-of-41 are different defects: one is a bad object in
/// the middle of a healthy table, the other is a walk that ran off the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocatorLevel {
    /// What is being indexed: `region`, `country_container`, `record`, ...
    pub label: String,
    /// Zero-based index within the container.
    pub ordinal: u64,
    /// How many entries the container declared, when the producer knew.
    /// `None` is a real answer -- an unbounded or not-yet-read container --
    /// and is encoded as such rather than as `0`.
    pub extent: Option<u64>,
    /// Address of the object at `ordinal`, when the producer had one.
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

/// `label_len u16 | flags u16 | ordinal u64 | extent u64 | address u64`
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

// ---------------------------------------------------------------------------
// Authority
// ---------------------------------------------------------------------------

/// WHO refused, as opposed to where the refusal was noticed.
///
/// D91's real cause was a different domain's incomplete roster, and it was
/// reported as a raid-parent defect because the channel had no way to say
/// "the thing that refused is not me". Every refusal now says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authority {
    /// This domain's own predicate refused.
    SelfDomain,
    /// A different domain's published state (roster, bridge, prewarm) was the
    /// authority and it refused or was incomplete.
    OtherDomain(u16),
    /// The engine itself: a read at this RVA did not satisfy the predicate.
    Engine { rva: u64 },
    /// The recorder host / transport refused.
    Host,
    /// An off-frame worker owned the answer and did not deliver it.
    OffFrameWorker,
}

/// `kind u16 | domain_id u16 | rva u64`
pub const AUTHORITY_LEN: usize = 12;

impl Authority {
    pub const fn kind_code(self) -> u16 {
        match self {
            Self::SelfDomain => 1,
            Self::OtherDomain(_) => 2,
            Self::Engine { .. } => 3,
            Self::Host => 4,
            Self::OffFrameWorker => 5,
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
        }
    }
}

// ---------------------------------------------------------------------------
// Predicate
// ---------------------------------------------------------------------------

/// Longest predicate name.
pub const MAX_PREDICATE_NAME_LEN: usize = 64;

/// The reserved predicate a producer that has not been migrated gets stamped
/// with.
///
/// This is what makes "a domain reports nothing" structurally impossible: the
/// encoder REFUSES an entry whose predicate code is zero, and the only way to
/// build an entry without a producer-declared predicate is to be given this
/// code, which says so out loud and is countable in one grep of a stream.
pub const PREDICATE_UNDECLARED: u32 = 0xFFFF_FF01;
/// The predicate name that travels with [`PREDICATE_UNDECLARED`].
pub const PREDICATE_UNDECLARED_NAME: &str = "producer_declared_no_predicate";

/// The specific check that refused.
///
/// A code is per-domain and stable: `(domain_id, code)` identifies the
/// predicate for the life of the telemetry archive. Codes are non-zero for the
/// same reason a reason code is: zero is what an uninitialised slot holds.
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

    /// The reserved stand-in for a producer that has not declared a predicate.
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

// ---------------------------------------------------------------------------
// Off-frame outcome
// ---------------------------------------------------------------------------

/// The four terminal states an off-frame derivation can be in.
///
/// D53 and D60 could report "a sweep is pending" but not "a sweep has been
/// pending for 50 seconds", which is why a worker that never returns is still
/// undiagnosed. Running is a legitimate outcome; running past the deadline is
/// a different one, and the elapsed/deadline pair is what separates them.
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

/// Spawn-to-return accounting for a derivation that does not run on the frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OffFrameOutcome {
    /// The generation the work was spawned under. A result that arrives after
    /// the generation moved on is stale, and this is how a consumer sees that.
    pub spawn_generation: u64,
    /// Microseconds from spawn to the moment this outcome was sampled.
    pub elapsed_us: u64,
    /// The bound the producer promised. `0` means "no deadline declared",
    /// which the render states rather than hiding.
    pub deadline_us: u64,
    pub terminal: OffFrameTerminal,
}

/// `spawn_generation u64 | elapsed_us u64 | deadline_us u64 | terminal u8 | pad [3]`
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

    /// Has this outcome blown the deadline it declared?
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

// ---------------------------------------------------------------------------
// Truncation accounting
// ---------------------------------------------------------------------------

/// Why units were dropped, as a value rather than as three dots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TruncationKind {
    /// Everything the producer offered was carried.
    None = 1,
    /// The offender's discretionary share of the frame budget ran out.
    BudgetShare = 2,
    /// A single unit was itself larger than any share could be.
    UnitTooLarge = 3,
    /// The producer offered more units than the wire can count.
    ProducerOverflow = 4,
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
        }
    }
}

/// Exactly what was lost, so a consumer never has to infer it from a literal
/// `...` that a real reason could also end with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TruncationAccount {
    pub kind: TruncationKind,
    /// How many units the producer offered.
    pub units_offered: u32,
    /// How many reached the wire.
    pub units_emitted: u32,
    /// How many encoded bytes of unit were dropped.
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

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

/// Bytes of unit blob every offender gets unconditionally, before any
/// discretionary byte is handed to anyone.
///
/// Sized so a deciding unit plus a full locator path always fits: eight
/// levels of `MAX_UNIT_NAME_LEN`-named locator units would be
/// 8 * (12 + 96) = 864, which is more headroom than any real producer needs,
/// so the floor is the smaller, realistic figure and the per-offender floor
/// cap below is what actually bounds the record.
pub const OFFENDER_FLOOR_BYTES: usize = 384;
/// Most units one offender may place from its floor. A producer that labels
/// everything `Deciding` cannot thereby seize the whole record: past this
/// count its units fall back to the discretionary pool with everyone else's.
///
/// The floor is NOT restricted to `Deciding`/`Locator` roles. Roles decide
/// WHICH units fill the floor; the floor itself is unconditional, so a domain
/// that has not been migrated -- and therefore has no `Deciding` unit to
/// prioritise -- still cannot be reduced to reporting nothing by a verbose
/// neighbour. "A domain reports nothing" is structurally unreachable for all
/// 73 domains, not only for the migrated ones.
pub const MAX_FLOOR_UNITS_PER_OFFENDER: usize = 8;

/// One offender's demand, as handed to the allocator.
#[derive(Clone, Debug)]
pub struct BudgetRequest {
    /// Stable ordering key; the allocator is deterministic in it.
    pub domain_id: u16,
    pub units: Vec<DiagUnit>,
}

/// One offender's allocation.
#[derive(Clone, Debug)]
pub struct BudgetGrant {
    pub domain_id: u16,
    pub units: Vec<DiagUnit>,
    pub account: TruncationAccount,
}

/// Share a byte pool between offenders so that no offender's carried units can
/// be reduced by another offender's verbosity below its equal share.
///
/// Two-phase, both deterministic:
///
/// * **Floor.** Every offender places its highest-priority units first, up to
///   [`MAX_FLOOR_UNITS_PER_OFFENDER`] units and [`OFFENDER_FLOOR_BYTES`]
///   bytes, whichever binds first. This does not come out of
///   `discretionary_pool` at all, so it cannot be competed for. Because units
///   are ordered by role before the floor is filled, an offender's `Deciding`
///   units are the first candidates; because the floor is unconditional, an
///   offender with no declared roles at all still places its first units.
///
///   The exact guarantee is: *the first up to `MAX_FLOOR_UNITS_PER_OFFENDER`
///   priority-ordered units that fit `OFFENDER_FLOOR_BYTES` are placed
///   regardless of what any other offender emits.* It is deliberately NOT
///   "every deciding unit always rides the floor" -- nine short deciding
///   units, or four with maximum-length names, exceed the floor, and the
///   surplus competes in the pool with everything else. Stating the weaker
///   true guarantee matters: a producer that declares nine deciding units has
///   not prioritised, and the channel should not pretend it has.
/// * **Discretionary pool, max-min fair.** The remainder is water-filled: each
///   round hands `remaining / claimants` to every offender still wanting
///   bytes, offenders whose whole remaining demand fits are satisfied and drop
///   out, and the freed surplus is redistributed. An offender therefore
///   receives `min(its demand, at least pool / n)`: it can be given less than
///   an equal share only when it did not ask for one, and no offender is ever
///   granted more than it demanded.
///
/// `max_bytes_per_offender` caps what any single offender may be granted in
/// total, floor included. The caller passes the bound its own container can
/// actually hold; an allocator that grants more than the record can encode
/// turns a truncated reason into an unpublishable one, which is strictly
/// worse.
///
/// Within an offender, units are placed in role order (`Deciding`, `Locator`,
/// `Context`, `Counter`, `Undeclared`) and, within a role, in the order the
/// producer built them. Value is NEVER an input: a zero-valued unit competes
/// on exactly the same terms as any other, which is the defect that hid D44's
/// failing stage.
pub fn allocate_unit_budget(
    requests: &[BudgetRequest],
    discretionary_pool: usize,
    max_bytes_per_offender: usize,
) -> Vec<BudgetGrant> {
    // Phase 1: the floor.
    let mut ordered: Vec<(usize, Vec<(usize, DiagUnit)>)> = Vec::with_capacity(requests.len());
    for (index, request) in requests.iter().enumerate() {
        let mut units: Vec<(usize, DiagUnit)> = request
            .units
            .iter()
            .cloned()
            .enumerate()
            .map(|(order, unit)| (order, unit))
            .collect();
        // Stable sort by role; producer order is preserved within a role.
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

    // Phase 2: max-min fair water-filling of the discretionary pool.
    let demands: Vec<usize> = pending
        .iter()
        .map(|units| units.iter().map(DiagUnit::encoded_len).sum::<usize>())
        .collect();
    let shares = water_fill(&demands, discretionary_pool, &order_by_domain_id(requests));

    for (index, units) in pending.iter().enumerate() {
        // The per-offender ceiling binds the TOTAL, floor included, so what is
        // granted always fits the container the caller named.
        let mut budget = shares[index].min(max_bytes_per_offender.saturating_sub(floor_bytes[index]));
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
            // Re-sort the kept set into role order so the wire order is the
            // priority order a reader wants, not floor-then-pool order.
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

/// The order the allocator visits offenders in when a remainder has to be
/// broken.
///
/// By `domain_id`, not by position in the slice. The two coincide today
/// because the completeness walk visits records in stream order, but an
/// allocation that depends on the caller's iteration order is one refactor
/// away from becoming irreproducible, and retained telemetry has to be
/// comparable across builds.
fn order_by_domain_id(requests: &[BudgetRequest]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..requests.len()).collect();
    order.sort_by_key(|index| (requests[*index].domain_id, *index));
    order
}

/// Classic max-min fair share. Deterministic; the integer-division remainder
/// goes to the lowest `domain_id` first, so two identical inputs always
/// allocate identically however the caller ordered its slice.
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
            // Fewer bytes than claimants: hand the remainder out one byte at a
            // time in index order rather than dropping it.
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

// ---------------------------------------------------------------------------
// Capture-at-failure
// ---------------------------------------------------------------------------

/// A slot that keeps the FIRST failure and counts the rest.
///
/// D44's country-supply diagnostics are process-global and shared across ~350
/// country iterations, so they reported whichever country ran LAST rather than
/// the one that failed; D66's "first failure kind" collapsed a per-bucket
/// outcome into one global and turned out to be reporting a coin flip. A slot
/// that latches on first write and refuses later ones makes a failure in
/// iteration 3 survive iterations 4..=350, and the `later` count says how many
/// followed so a single sporadic failure is still distinguishable from a
/// systematic one.
///
/// Latching on FIRST rather than LAST is deliberate and is the only choice
/// that is deterministic: last-write-wins depends on iteration order and on
/// what freed memory happened to hold.
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

    /// Offer a failure. The first offer is kept; every later offer only
    /// increments the count. Returns true when this offer was the one kept.
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

    /// How many failures followed the one that was kept.
    pub const fn later_failures(&self) -> u64 {
        self.later
    }

    /// Total failures offered.
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

#[cfg(test)]
mod tests {
    use super::*;

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
    /// across recorder builds, so these numbers are pinned here and a change
    /// has to come through this test.
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
        // The whole point: a zero-valued OBSERVED unit and an ABSENT unit are
        // different on the wire, and neither one vanished.
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
        // "of ?" is a real answer and is distinct from "of 0".
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
        // A completed worker is never "overdue", however long it took.
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
        // Exactly D44's shape: the deciding units are zero and are constructed
        // AFTER a dozen non-zero counters.
        let mut units = vec![DiagUnit::new(
            "d44_country_supply_failure_stage",
            0,
            UnitRole::Deciding,
            UnitPresence::Observed,
        )];
        for index in 0..40 {
            units.push(DiagUnit::counter(format!("d44_filler_{index}"), 99));
        }
        units.rotate_left(1); // deciding unit is now LAST in producer order
        let grants = allocate_unit_budget(
            &[BudgetRequest {
                domain_id: 44,
                units,
            }],
            0, // no discretionary pool at all
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
        // And the same quiet offender alone gets exactly the same units: its
        // outcome does not depend on the neighbour at all.
        let alone = allocate_unit_budget(&[quiet], pool, usize::MAX);
        assert_eq!(alone[0].units, quiet_grant.units);
        // The verbose one is the only one that loses anything.
        let verbose_grant = grants.iter().find(|g| g.domain_id == 47).unwrap();
        assert_eq!(verbose_grant.account.kind, TruncationKind::BudgetShare);
    }

    #[test]
    fn every_offender_receives_at_least_its_equal_share_of_the_pool() {
        // Property check: with N offenders each demanding more than the pool,
        // nobody gets less than pool/N.
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
        // The record stays bounded: the floor is per-offender and outside the
        // pool, so the whole allocation is bounded by
        // n * OFFENDER_FLOOR_BYTES + pool and by nothing the producers choose.
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

    /// The bar is structural, not "the migrated domains are fine". 71 of 73
    /// producers still emit untyped pairs; if the floor only protected
    /// `Deciding` units, every one of them could still be silenced by a
    /// verbose neighbour and the channel would have fixed nothing for them.
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
        // And a legacy zero is carried, not deleted for being zero.
        assert_eq!(quiet.units[0].value, 0);
        assert_eq!(quiet.account.kind, TruncationKind::BudgetShare);
        assert_eq!(quiet.account.units_offered, 30);
        assert_eq!(
            quiet.account.units_emitted,
            MAX_FLOOR_UNITS_PER_OFFENDER as u32
        );
        // Every offender in the frame placed its floor: nobody reports nothing.
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

    /// No offender may be granted more than the caller's container can encode.
    ///
    /// Granting more looks generous and is not: the record then refuses to
    /// encode, and a refusal that cannot be encoded is an hour that publishes
    /// NOTHING. That is strictly worse than a truncated reason, which is the
    /// whole reason `max_bytes_per_offender` exists.
    ///
    /// Production change that makes this fail: ignoring `max_bytes_per_offender`
    /// in either phase -- dropping `floor_ceiling` back to `OFFENDER_FLOOR_BYTES`
    /// or dropping the `.min(...)` that binds the discretionary share.
    #[test]
    fn no_offender_is_granted_more_than_the_container_can_hold() {
        // One hog and one quiet domain; the pool alone would hand the hog far
        // more than the cap.
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
        // The cap binds the hog and not the quiet domain, and the hog says so.
        let hog = grants.iter().find(|g| g.domain_id == 44).unwrap();
        assert_ne!(hog.account.kind, TruncationKind::None);
        let quiet = grants.iter().find(|g| g.domain_id == 91).unwrap();
        assert_eq!(quiet.units.len(), 1);
        assert_eq!(quiet.account.kind, TruncationKind::None);

        // And the cap binds the FLOOR too, which a cap larger than
        // `OFFENDER_FLOOR_BYTES` cannot show. The floor is unconditional --
        // that is its whole point -- so when a caller's container is smaller
        // than the floor, the cap is the only thing standing between the floor
        // and a record that cannot encode.
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
        // Tight is not empty: the floor still places whatever fits.
        assert!(tight.iter().all(|grant| !grant.units.is_empty()));
    }

    /// The same offenders allocate identically however the caller ordered its
    /// slice.
    ///
    /// Retained telemetry has to be comparable across recorder builds, and an
    /// allocation whose remainder depends on the order the completeness walk
    /// happened to visit records in is one refactor away from becoming
    /// irreproducible.
    ///
    /// Production change that makes this fail: water-filling in slice order
    /// instead of `order_by_domain_id` order.
    #[test]
    fn the_allocation_does_not_depend_on_the_callers_slice_order() {
        // The visit order only ever shows in the remainder, so this is built so
        // that the remainder DECIDES A UNIT rather than a byte nobody can
        // spend. A test where the remainder buys nothing cannot see the
        // difference and would certify either implementation.
        //
        // Twelve units per domain, each `12 + 9 = 21` encoded bytes. The
        // unconditional floor takes eight, leaving four pending and a demand of
        // 84. A pool of 61 gives each of the three claimants 20 -- one byte
        // short of a single unit -- and leaves one byte over, which
        // `water_fill` hands to the FIRST claimant in visit order. That
        // claimant, and only that claimant, places a ninth unit.
        //
        // Walking the caller's slice instead of `domain_id` order therefore
        // gives that ninth unit to whichever domain the caller happened to put
        // first, and the two orderings below disagree.
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
}
