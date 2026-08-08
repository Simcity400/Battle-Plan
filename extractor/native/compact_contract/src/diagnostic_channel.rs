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
    /// The record never said. Only a lifted version 1 record produces this:
    /// v1 carried no authority, and the lift must state that absence rather
    /// than hand a reader a default that looks like a declaration. Its kind
    /// code is 0 — the value an uninitialised slot holds — and the decoder
    /// keeps rejecting 0, so this can exist in memory but never on the wire.
    Undeclared,
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
    /// The record never accounted. Only a lifted version 1 record produces
    /// this: v1 had no truncation accounting, and claiming `complete` would
    /// assert a fact no v1 producer stated. Code 0 — the uninitialised-slot
    /// value — and `from_code` keeps rejecting it, so it cannot reach the
    /// wire.
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

    /// The account a lifted version 1 record carries: v1 never accounted, and
    /// this says so instead of claiming completeness.
    pub const fn undeclared() -> Self {
        Self {
            kind: TruncationKind::Undeclared,
            units_offered: 0,
            units_emitted: 0,
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
            // No counts: rendering `0/0` here would state an accounting the
            // record never made.
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

// ---------------------------------------------------------------------------
// Accumulation
// ---------------------------------------------------------------------------

/// How ONE frame's value for a named unit composes into the whole-capture
/// figure the manifest publishes.
///
/// # Why this exists
///
/// [`UnitRole`] says what a unit is FOR and [`UnitPresence`] says whether its
/// value is a measurement. Neither says how two frames' values combine, and
/// the recorder host had no answer either: it summed everything except two
/// hard-coded name shapes (`*_sample`, `d69_*_mask`). That default is correct
/// only for a unit whose value describes the frame it rode in.
///
/// It is not correct for a unit whose producer keeps a running total and
/// reports its ABSOLUTE value every frame. Summing an absolute running total
/// over `n` frames yields roughly `n^2/2` times the truth. Measured, not
/// theorised: in `runs/live_unlock/t15`, domain 90's
/// `convoy_transport_attack_dropped_events` was published as **19,470**. The
/// stream's 396 CTA2 batch headers carry `callbacks_seen` running 1, 2, ...,
/// 107 — the real population is **107**, and 19,470 is the triangular sum of
/// the same 107 events counted again on every frame that followed. The figure
/// was quoted to the project as an event count twice.
///
/// The same shape was live in the `rs_*` scheduler probe, where the producer's
/// own doc comment already said "counters are cumulative (monotonic); the host
/// aggregates them by MAX" while the host in fact summed them.
///
/// # The rule
///
/// A unit's class is a property of its PRODUCER, and this enum is where the
/// producer's answer is written down. [`unit_accumulation`] is the single
/// authority; every aggregation site consults it and no aggregation site is
/// allowed its own private rule. [`Accumulation::fold`] is the only place a
/// value is combined.
///
/// # Undeclared is not a default
///
/// [`Accumulation::Undeclared`] exists so that "nobody classified this" is a
/// visible state rather than a silent SUM. It folds like [`Self::PerFrame`] —
/// changing the arithmetic for an unknown unit would silently move published
/// figures — but the recorder counts it, the manifest publishes it, and the
/// promotion gate refuses a capture that contains one. A new unit in a family
/// whose values are known to be cumulative therefore cannot join the summed
/// side quietly: it lands in `Undeclared` and says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Accumulation {
    /// The value describes THIS frame and nothing else: a count of rows
    /// decoded from this frame's payload, an incidence flag for this frame, a
    /// per-frame drain of a registry that is reset by the drain. Whole-capture
    /// figure is the SUM.
    PerFrame = 1,
    /// The producer maintains the value across frames and reports its absolute
    /// state every frame: a monotonic counter, a running maximum, a latching
    /// boolean, or a session constant. Whole-capture figure is the MAX, which
    /// for a monotonic series is the final value and for a constant is the
    /// constant. Summing is what produced the 19,470.
    Cumulative = 2,
    /// An instantaneous level rather than a total: a queue depth, an in-flight
    /// count, the size of the publication currently in force. Whole-capture
    /// figure is the PEAK, so it folds by MAX like [`Self::Cumulative`], but
    /// the two are kept apart because a peak and a final total are different
    /// claims and a reader must be able to tell which one a figure is.
    Gauge = 3,
    /// A raw word sampled this frame — a hash, an address, an index, a
    /// percentile over a window the producer already accumulated. Arithmetic
    /// destroys it. Whole-capture figure is the LAST observed value.
    LastValue = 4,
    /// A bitmask of slots. Two frames failing slot 0 must read as mask 1, not
    /// as 2 — which would name slot 1, a slot that never failed. Whole-capture
    /// figure is the bitwise OR.
    BitwiseOr = 5,
    /// No producer has declared a class for this name. Folds as
    /// [`Self::PerFrame`] so that adding the declaration later cannot change a
    /// figure by accident, but is counted, published, and gated.
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

    /// Every class, for census and test enumeration.
    pub const ALL: [Self; 6] = [
        Self::PerFrame,
        Self::Cumulative,
        Self::Gauge,
        Self::LastValue,
        Self::BitwiseOr,
        Self::Undeclared,
    ];

    /// Combine one frame's `incoming` value into the running figure.
    ///
    /// `running` is `None` for the first observation of the unit. This is the
    /// ONLY place a diagnostic unit's values are combined; an aggregation site
    /// that reimplements the arithmetic is the defect this type exists to
    /// remove.
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

    /// Does this class need every frame's value to arrive to be correct?
    ///
    /// `false` for the classes whose figure survives a lost frame, which is
    /// why a hard kill costs a cumulative unit only the in-flight tail.
    pub const fn requires_every_frame(self) -> bool {
        matches!(self, Self::PerFrame | Self::Undeclared)
    }
}

/// Name prefixes whose units come from a producer that keeps state across
/// frames, so an unregistered name in the family is [`Accumulation::Undeclared`]
/// rather than silently per-frame.
///
/// This is the mechanism that stops a new unit joining the summed side by
/// accident. Everywhere else the default is `PerFrame` — which is what the
/// ~460 domain data units genuinely are, because their values are computed
/// from the payload of the frame they ride in. Inside these prefixes it is not
/// a safe default, because everything the family has ever emitted has been
/// producer-retained state.
pub const STATEFUL_UNIT_PREFIXES: &[&str] = &[
    // Read-only scheduler + sink accounting (`SchedulerTelemetry`,
    // `SinkDeliveryState`), neither reset within a capture.
    "rs_",
    // Domain 90's convoy terminal hook, whose drain field is named
    // `cumulative_telemetry`.
    "convoy_transport_attack_",
    // Domain 61 reads the engine's 89-field per-country AGGREGATE air-loss
    // history record; every channel is an engine lifetime total.
    "air_loss_",
    // Domain 75's fired-event ledger is append-only for the life of the
    // runtime: `snapshot_rows` clones the whole ledger every frame and rows are
    // removed only by an install abort.
    "fired_event_popups_",
    "d75_",
    // The heap sweep's counters and generation are process-global statics.
    "heap_sweep_",
    // Domain 86's `current_complete_snapshot` clones the complete writer
    // history every frame.
    "sunk_ship_",
    // The two A40 observers. EVERY unit either domain publishes is read out
    // of a process-global `HookState` that survives the frame -- callback
    // totals, refresh sequences, latching install flags, the ordinal of a
    // latched failure -- so a new sibling here is producer-retained until
    // somebody says otherwise, and `PerFrame` would silently sum it. The
    // suffix rules still run first, which is what keeps
    // `d85_error_labelling_ordinal`-shaped names on the `LastValue` side and
    // `*_mask` names on the bitwise side without needing an entry each.
    "d85_",
    "d97_",
    // D95's spotted-unit observer, for the same reason. Its per-arm census
    // lives in the process-global `LatestFrames` and its site-entry counts in
    // `SITE_ENTRIES` statics; neither is reset between frames, so an
    // unregistered sibling here must be `Undeclared` rather than summed.
    "d95_",
];

/// Name suffixes whose value is a raw OBSERVATION rather than a quantity: a
/// hash, a fingerprint, a pointer, an address, a vtable, an RVA, an engine
/// identifier, an ordinal, or a failure code.
///
/// This is the third category, distinct from per-frame quantities and from
/// cumulative counters. Arithmetic on any of these is not merely inaccurate,
/// it is meaningless — and it has already produced published lies, all of them
/// read out of `runs/live_unlock/t15`:
///
/// * `h_convoy_resolver_error_hash = 18446744073709551615` — `u64::MAX`,
///   the saturation of one hash added to itself once per frame. Also
///   `d93_fail_reason_hash` and `d64_first_rejected_timed_effect_class_fnv1a64`.
/// * `d43_resource_failure_resource_index = 1550483193495` — exactly
///   `u32::MAX * 361`, i.e. the producer's "never written" reset sentinel on
///   every one of the domain's 361 frames.
/// * `d44_country_supply_slot_index` and `d44_country_supply_tag_db_id` —
///   `u32::MAX * 7` apiece.
/// * `d64_first_rejected_timed_effect_vtable = 1547721722735848` — one vtable
///   pointer, added up 396 times.
/// * `d41_lifecycle_failure_stage = 180063` — stage code 513, summed over 351
///   frames.
///
/// The `_sample` convention was already here and already respected by one of
/// the three aggregation sites. The rest of this list is the same convention
/// applied to the units that predate it: a suffix rule is a NAMING rule, so
/// units named before the rule existed were silently on the wrong side. The
/// registry above is what makes membership explicit rather than orthographic;
/// these suffixes cover the families where the spelling genuinely does name
/// the value's kind, and the sweep in
/// `tests/diagnostic_unit_accumulation_sweep.rs` is what stops a new unit
/// relying on a spelling nobody enforced.
pub const RAW_WORD_SUFFIXES: &[&str] = &[
    "_sample",
    // Digests and identities.
    "_hash",
    "_fnv1a64",
    "_fingerprint",
    // Addresses.
    "_ptr",
    "_rva",
    "_address",
    "_vtable",
    // Engine identifiers.
    "_id",
    // Positions within a container. `_ordinal` names the same thing as
    // `_index` and was missed on the first pass, which summed
    // `air_strength_first_invalid_ordinal` and
    // `domain47_manager_failure_ordinal`.
    "_index",
    "_ordinal",
    // The width of the read that failed.
    "_fail_width",
    // Latched failure codes: which check refused, in which branch, on what
    // kind of object. A code is a name written as a number.
    "_stage",
    "_branch",
    "_reason",
    "_kind",
    "_code",
    // NOT `_epoch`: `convoy_transport_attack_dropped_hour_or_epoch` is a
    // cumulative DROP COUNT whose name happens to end that way. The two real
    // epoch words are listed in `LAST_VALUE_UNITS` instead. A suffix rule is
    // only safe when the suffix names the value's KIND rather than the last
    // word of a sentence.
];

/// Name suffixes whose value is a bitmask of reasons, slots, or status bits.
///
/// Generalises the `d69_*_mask` special case the recorder had hard-coded: the
/// same argument applies to `d70_rejected_unknown_status_mask`,
/// `d81_failure_read_success_mask`, the three `domain45_targeted_*_reason_mask`
/// words, `domain53_partial_cause_mask`, and
/// `domain53_reportfail_payload_status_or`. Two frames setting bit 0 must read
/// as 1, never as 2 — which would name a bit that was never set.
pub const BITMASK_SUFFIXES: &[&str] = &["_mask", "_status_or"];

/// Units whose value is an absolute running total, a running maximum, a
/// latching boolean, or a session constant. Folded by MAX.
///
/// `rs_*` come from `read_only_scheduler::SchedulerTelemetry` and the
/// collector's `SinkDeliveryState`; neither is ever reset within a capture, so
/// each probe record carries the accounting AS OF that record.
///
/// `convoy_transport_attack_*` come from `ConvoyTerminalDrain`'s field that is
/// literally named `cumulative_telemetry`, backed by process-global `AtomicU64`
/// counters that `reset_observer_state_for_install` clears only at hook
/// install.
pub const CUMULATIVE_UNITS: &[&str] = &[
    // --- read-only scheduler accounting (SchedulerTelemetry) ---------------
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
    // Running maxima maintained by the scheduler, not per-frame depths.
    // `read_only_scheduler` keeps each with `.max(...)`, never resetting.
    "rs_max_request_depth",
    "rs_untrusted_clock_recovery_max_delta_hours",
    "rs_max_control_depth",
    "rs_max_publishing_depth",
    // Session constants: capacity caps and policy latches. Constant across
    // frames, so MAX is the constant and SUM was "the constant times frames".
    "rs_request_cap",
    "rs_control_cap",
    "rs_zero_mutation_policy",
    "rs_scheduler_active",
    // --- capture-floor stage S3 custody SHADOW ----------------------------
    // `collector::custody_shadow::CustodyShadowSummary`, every field of which
    // is a running total or a running maximum that is never reset within a
    // capture -- the same contract as `SchedulerTelemetry` above, because it
    // lives inside it. These describe what the END-STATE pipeline would have
    // done; none of them describes what the capture published.
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
    // Running maxima, kept with `.max(...)` and never reset.
    "rs_shadow_max_late_dispatch_us",
    "rs_shadow_max_cost_ns",
    // Latching booleans: set once, never cleared.
    "rs_terminal",
    "rs_transport_failed",
    "rs_finalized",
    // --- sink delivery accounting (SinkDeliveryState) ---------------------
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
    // `write_times_us` is push-only for the life of the capture, so its length,
    // total and maximum are all cumulative.
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
    // --- domain 90 convoy transport attack hook (cumulative_telemetry) ----
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
    // The per-site splits of the two aggregates above. Same shape -- process-
    // global counters reset only at hook install, republished absolute every
    // batch -- so they must be MAX'd for exactly the reason the aggregates are.
    "convoy_transport_attack_dropped_pre_hour_unreadable",
    "convoy_transport_attack_dropped_receipt_hour_mismatch",
    "convoy_transport_attack_dropped_post_hour_unreadable",
    "convoy_transport_attack_dropped_post_hour_changed",
    "convoy_transport_attack_dropped_resolver_snapshot_absent",
    "convoy_transport_attack_dropped_resolver_unpublished",
    // Running maxima of a game-hour distance. Cumulative for the same reason:
    // a maximum republished every batch is not a per-frame quantity, and
    // summing it would report a distance no receipt was ever at.
    "convoy_transport_attack_receipt_hour_lag_max",
    "convoy_transport_attack_receipt_hour_lead_max",
    // --- domain 75 fired-event popups (append-only ledger + hook telemetry) --
    // `snapshot_rows` clones `state.rows` in full every frame, and rows are
    // inserted without removal up to the queue capacity, so the published row
    // count is the ledger SIZE, not this frame's fires.
    "fired_event_popups_rows",
    "fired_event_popups_unadmitted_rows",
    "fired_event_popups_dropped_rows",
    "d75_dropped_fire_rows",
    "d75_dropped_resolution_rows",
    "d75_correlation_failures",
    "d75_invalid_definition_joins",
    "d75_fired_ledger_shadow_mismatches",
    // Running maximum: `queue_high_water_mark.max(rows.len())`.
    "d75_queue_high_water_mark",
    // Session constant.
    "d75_queue_capacity",
    // Latching validation booleans: set once as each hook proves itself.
    "d75_runtime_installed",
    "d75_fire_hook_validated",
    "d75_option_hook_validated",
    "d75_materialisation_hook_validated",
    "d75_timeout_hook_validated",
    "d75_all_event_classes_covered",
    "d75_all_resolution_routes_covered",
    "d75_capture_start_baseline_complete",
    "d75_terminal_custody",
    // --- domain 86 sunk-ship writer history --------------------------------
    // `current_complete_snapshot` clones the complete event history, so the
    // count is the history LENGTH.
    "sunk_ship_writer_events",
    // --- process-global sweep state ----------------------------------------
    "heap_sweep_degraded",
    "heap_sweep_degraded_total",
    "heap_sweep_spawn_failures",
    "heap_sweep_generation",
    // Monotonic per-tracker stale-frame counters and absolute generation
    // numbers; the producers say "monotonic per tracker" at the increment site.
    "domain53_sweep_stale_frames",
    "domain53_sweep_generation",
    "domain53_war_validated_generation",
    "capwar60_sweep_stale_frames",
    // Process-global monotonic publication sequence shared by d49/d50.
    "h_convoy_resolver_publication_sequence",
    // --- A40: the two selected-designer / peace-conference observers -------
    // Both hooks keep their counters in process-global `HookState`, reset only
    // at install or at an authenticated campaign boundary, and publish the
    // ABSOLUTE value every frame. This is the exact shape that produced the
    // 19,470: `d85_callback_count` on a 400-frame capture with 10 real
    // callbacks would be published as ~4,000 if it were summed.
    //
    // `*_hook_installed` are latching session booleans -- and in a passive
    // product capture they are latched at ZERO, because the collector gates
    // both A40 installs behind `!passive_product_capture`. MAX is what makes
    // "was this observer ever patched in" answerable from the manifest.
    "d85_callback_count",
    "d85_variant_upgrade_selection_count",
    // Same shape: an observer-lifetime total of division-order registrations,
    // published absolute every frame. Summing it over a 644-frame capture would
    // multiply a handful of real registrations by ~644.
    "d85_division_view_registrations",
    "d85_hook_installed",
    "d97_refresh_observed_total",
    "d97_hook_installed",
    // D95's per-arm census and hook-entry counts. Every one of these is a
    // PROCESS-LIFETIME total read out of `LatestFrames::census` or the
    // `SITE_ENTRIES` statics, published as its absolute value each frame --
    // the same shape as `d85_callback_count` directly above, and summing them
    // over a 316-hour capture would multiply each by ~316.
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
];

/// Domain 61 publishes one unit per pinned engine channel, named
/// `air_loss_{label}_raw_sum`, where the label comes from the lane's
/// `PINNED_CHANNELS` table. The value is the within-frame sum over countries of
/// each country's ENGINE LIFETIME total for that channel, so it is cumulative
/// even though the host built it by adding.
///
/// Matched by shape rather than listed, because the 24 labels live in the lane
/// crate and `compact_contract` is below it in the dependency order. The shape
/// is specific enough not to collide: `air_wing_loadout_amount_raw_sum` is a
/// genuine per-frame sum and is not in the `air_loss_` family.
fn is_air_loss_channel_total(name: &str) -> bool {
    name.starts_with("air_loss_") && name.ends_with("_raw_sum")
}

/// One word of a SHA-256 carried as `<group>_sha256_<n>`, or the `<group>`
/// prefix the producer hands to the helper that builds the four words.
///
/// Matched by shape so that a new digest group is classified the day it is
/// added rather than the day someone notices its words are being added
/// together.
fn is_digest_word_or_group(name: &str) -> bool {
    if let Some((group, index)) = name.rsplit_once("_sha256_") {
        return !group.is_empty()
            && !index.is_empty()
            && index.bytes().all(|b| b.is_ascii_digit());
    }
    matches!(
        name,
        "d75_terminal_payload" | "d64_terminal_envelope" | "d64_terminal_del2"
    )
}

/// Units whose value is an instantaneous level. Folded by MAX, reported as a
/// peak.
pub const GAUGE_UNITS: &[&str] = &[
    // Queue depth and in-flight callback count at the instant of the drain.
    "convoy_transport_attack_remaining_events",
    "convoy_transport_attack_active_callbacks",
    // The size of the d49/d50 resolver publication in force for this batch,
    // not a total over the capture. Summing it multiplied one publication by
    // the frame count.
    "convoy_transport_attack_resolver_member_bindings",
    "convoy_transport_attack_resolver_route_bindings",
    "convoy_transport_attack_resolver_unbound_convoy_clients",
    // How stale the d53 discovery sweep was when this frame read it.
    "domain53_sweep_age_ms",
    // The deferred publication's start and end OFFSETS within the frame. Their
    // producer describes them as interval endpoints to be intersected against
    // the reader spans, so they are positions on a timeline, not durations: a
    // sum of start positions is not any quantity. The host does not route them
    // into the lane-wall aggregate, so before this they were summed into
    // `read_ab_probe`. The peak -- the latest the publication ever started or
    // ended -- is a statistic that can be named.
    "lw_pub_start_us",
    "lw_pub_end_us",
    // The three WIDTHS d43's purchase-contract census states each frame: the
    // country set it was handed, the active-country subset that set was
    // widened from, and the engine's own per-country outer table count. They
    // exist so one capture can tell "our extended country set outgrew the
    // engine's table" from every other refusal, and that comparison is only
    // meaningful frame by frame. Summing them would report a 439-country world
    // as 129,066 countries over 294 frames -- the same defect the resolver
    // bindings above were moved here to fix.
    "d43_contract_scope_countries",
    "d43_contract_snapshot_countries",
    "d43_contract_outer_count",
    // How many peace conferences the engine's own container held at the
    // instant D97 read it. A LEVEL, and one that spends nearly every hour of a
    // campaign at zero: it is what separates D97's measured zero from a
    // conference the window observer missed. Summing it would report a single
    // three-turn conference as hundreds of conferences.
    "d97_conference_count",
    // How many designer families D85 currently holds a committed observation
    // for. Bounded by six, and a level rather than a total: the map is
    // overwritten in place by each new selection.
    "d85_observed_families",
];

/// The OUTSIDE-BUILD frame segments: where the sweep thread spends the wall
/// time that is not inside `build_compact_extraction_gate_records`.
///
/// Nothing measured this before. The frame period was known (119.76 ms in
/// `runs/live_unlock/t19`) and the build's own `frame_time_us` was known
/// (~93 ms), so ~27 ms — 23% of the period, and the period is what decides how
/// many game hours a capture can keep — existed only as the arithmetic
/// difference of two other numbers. These units make it an observation.
///
/// # The two shapes in this table, and why mixing them is not a defect here
///
/// `total` and `wait` PARTITION the interval: `total` is the whole gap between
/// one build's return and the next build's entry, and `wait` is the part of it
/// spent parked on the scheduler's hour latch. `total - wait` is the busy
/// remainder.
///
/// Everything else is a NAMED SUB-SPAN of that busy remainder, and several of
/// them nest inside each other (`complete`, `degnotice`, `deglog`, `openpub`
/// and `a30insert` all run inside `prepub`). They are deliberately not made
/// disjoint: the question each answers is "how much does THIS call cost",
/// which stays answerable when the enclosing span is also reported, and a
/// disjoint carve-up would have to invent a name for every gap between two
/// timed calls. The host publishes them individually and never adds them
/// together.
///
/// `outside_build_segments_do_not_sum_to_total` pins two levels of it: the
/// `outside_build_us` object holds one labelled entry per segment carrying its
/// own observation and no other key, and the enclosing lane-wall aggregate's
/// whole field roster is fixed, so no total can appear beside it either. What
/// it does not reach is a total buried deeper — inside `per_lane`, say. A
/// recursive shape check could reach that; it is not worth the machinery for a
/// field nobody would put there, so the gap is named and left to review rather
/// than pretended away.
///
/// Every entry is a DURATION and folds `PerFrame`, so the host sums them and
/// keeps the distribution. "Per frame" is exact whenever every build publishes
/// a probe, which is the ordinary case. It is not a promise: the accumulator is
/// cleared where it is PUBLISHED, so a build that returns without emitting one
/// leaves its spans to the next probe, and that observation then covers more
/// than one build.
///
/// `lw_frames` counts PROBES, not builds — it is `1` per lane-wall record — so
/// dividing a sum by it gives the mean per PUBLISHED window, not per build.
/// Those two are the same number exactly when every build publishes, and this
/// build emits no count of builds that did not. A consumer that needs a true
/// per-build mean needs a producer change; a consumer that wants to know
/// whether it needs one can compare `lw_frames` against the manifest's
/// scheduler hour accounting.
///
/// The codes are wire-stable: a segment that stops being measured keeps its
/// code and simply stops being emitted.
///
/// # What is NOT reported, stated rather than discovered
///
/// The accumulator is published by the lane-wall probe of the next frame that
/// emits one. So a span opened on a path that TERMINATES the capture — a `?`,
/// a `break 'capture`, a staging-pool refusal — is never reported, because
/// there is no later frame to report it. That is not a leak to be chased: the
/// capture is over, and the frame period these segments describe is a
/// steady-state quantity.
///
/// Two per-iteration prologue paths `continue` without closing `identity`, and
/// both are deliberate: the campaign-rollover arm (see below) and, before it,
/// nothing else — the uncertain-boundary retry DOES close `identity` and books
/// its backoff to `wait`, because a sleeping thread reported as busy would
/// corrupt the one figure `total - wait` exists to give.
pub const OUTSIDE_BUILD_SEGMENTS: &[(u16, &str)] = &[
    // The interval itself: previous build's return -> this build's entry.
    (0, "total"),
    // Parked on the read-only scheduler's hour latch (or the legacy fixed
    // sample sleep). Idle by construction, and the only part of `total` that
    // is not the sweep thread doing work.
    (1, "wait"),
    // The whole per-iteration prologue: the stop-request checks, the
    // capture-ready and early-render session polls, the good-page cache epoch
    // bump, the IdentityV1 re-read, the campaign-root comparison and the
    // campaign state refresh. Named for its dominant term. Runs at the top of
    // EVERY loop iteration, before any sweep decision.
    //
    // NOT closed on the campaign-rollover arm, which `continue`s from inside
    // the identity boundary match. That iteration's cost lands in `total` and
    // in no named segment, which is the honest reading: a rollover resets the
    // campaign and the segment names below describe a steady-state frame.
    (2, "identity"),
    // The A30 evidence/production owner polls and the capture-ready /
    // early-render session polls, also per-iteration.
    (3, "prepoll"),
    // The whole pre-boundary publication block: everything done to the
    // PREVIOUS hour's finished frame before its bytes are staged for the
    // overlapped publisher.
    (4, "prepub"),
    // `compact_frame_completeness_findings` — one walk of the finished frame's
    // records. Inside `prepub`.
    (5, "complete"),
    // `insert_frame_degraded_notice_before_frame_end`. Inside `prepub`. Every
    // frame in t19 was degraded, so this ran on every frame.
    (6, "degnotice"),
    // The degraded-frame line appended to `collector_worker_error.log`: an
    // open/append/close per degraded frame. Inside `prepub`.
    (7, "deglog"),
    // `transport.open_preboundary_publication` — the campaign-identity
    // observation that gates the staging. Inside `prepub`.
    (8, "openpub"),
    // `insert_a30_payloads_before_frame_end`. Inside `prepub`.
    (9, "a30insert"),
    // Hook/UI staging for THIS hour before the build: latched render, A37
    // evidence, resolved tooltips, the decision panel, A38, and the
    // selected-Present finalization.
    (10, "hookstage"),
    // Post-build work in the same iteration: the render/A38/scheduler-probe
    // insertions, the pending-frame assembly, and the legacy synchronous
    // frame write on the scheduler-off path.
    (11, "tailinsert"),
    // The build's OWN untimed tail: the wall time
    // `build_compact_extraction_gate_records` spends after it stamps the
    // `frame_time_us` that the FRAME_END record and every report carry.
    //
    // This is not a rounding error. `frame_time_us` is stamped with the last
    // domain emission still several hundred lines from the end of the
    // function, and what runs after it includes the read-counter probe, the
    // attribution probe, the lane-wall probe, the FRAME_END append, and
    // `replace_rejected_domain_records_with_gaps` — which allocates a second
    // buffer the size of the frame and copies every retained record into it.
    // A capture that reports 93 ms of build was therefore reporting the build
    // up to that stamp and calling the rest "outside".
    //
    // Measured in the CALLER, as the build call's true wall time minus the
    // `frame_time_us` the build returned, and therefore carried in the NEXT
    // frame's probe: this frame's probe was encoded inside the very tail being
    // measured, so the number does not exist yet when the record is written.
    //
    // THE NAME SAYS "PREVIOUS" BECAUSE THE RECORD'S HOUR LABEL IS NOT ITS OWN.
    // A probe emitted for hour N carries `frame_time_us` and `total` belonging
    // to hour N, and this value belonging to an EARLIER build. That
    // displacement is a property of when the number becomes knowable and
    // cannot be removed without a second probe record per frame. It is stated
    // here, and in the manifest key, so that no consumer has to discover it.
    //
    // "An earlier build" and not flatly "hour N-1", because the accumulator is
    // cleared only where it is published. A build that returns without
    // emitting a lane-wall probe — the `state == None` path — carries its tail
    // and its interval forward, so after a run of those the next probe's
    // `buildtail_previous_frame` and `total` are SUMS over several builds while
    // its `frame_time_us` is one build's. In the ordinary case, where every
    // build publishes, it is exactly hour N-1's.
    //
    // The consequence for the identity: the frame period does NOT partition
    // per frame. It partitions over a SESSION, which is the only level the
    // manifest reports at anyway:
    //
    //     sum(frame_time_us) + sum(buildtail_previous_frame) + sum(total)
    //
    // counts each publishing build's own time, its own tail and the interval
    // after it once each. Four things sit outside that sum, named rather than
    // hidden, all of them at the two ends of a capture:
    //
    //   * the first build has no preceding `total`;
    //   * the last build's tail is computed after its own probe, so no probe
    //     reports it;
    //   * the interval AFTER the last build is closed into `total` only when
    //     another build begins, so if the capture ends after a publishing
    //     build that interval is never closed at all — this needs no
    //     non-publishing build and is not the same omission as the tail above;
    //   * a capture that ends on a run of builds that published nothing never
    //     flushes that whole suffix, which is as many builds long as the run.
    //
    // The first three are one build's worth each. The fourth is bounded by the
    // length of the terminal non-publishing run, which is unbounded in
    // principle and empty in every capture measured so far.
    (12, "buildtail_previous_frame"),
];

/// The wire unit name for each [`OUTSIDE_BUILD_SEGMENTS`] code, indexed by that
/// code. A `&'static str` table rather than a `format!`, because a diagnostic
/// unit's name must be `&'static str` and because the host resolves the same
/// table when it labels the manifest.
pub const OUTSIDE_BUILD_UNIT_NAMES: &[&str] = &[
    "lw_o0_us", "lw_o1_us", "lw_o2_us", "lw_o3_us", "lw_o4_us", "lw_o5_us", "lw_o6_us",
    "lw_o7_us", "lw_o8_us", "lw_o9_us", "lw_o10_us", "lw_o11_us", "lw_o12_us",
];

/// How many outside-build segments exist. Producer and host both size their
/// accumulators from this.
pub const OUTSIDE_BUILD_SEGMENT_COUNT: usize = OUTSIDE_BUILD_SEGMENTS.len();

/// The segment name for a wire code, or `None` for a code this build does not
/// know. An unknown code is reported by the host under its raw number rather
/// than dropped, so a newer collector's segment is still visible.
pub fn outside_build_segment_name(code: u16) -> Option<&'static str> {
    OUTSIDE_BUILD_SEGMENTS
        .iter()
        .find(|(segment_code, _)| *segment_code == code)
        .map(|(_, name)| *name)
}

/// Units inside a [`STATEFUL_UNIT_PREFIXES`] family that ARE per-frame, listed
/// explicitly because inside those families silence means undeclared.
pub const STATEFUL_FAMILY_PER_FRAME_UNITS: &[&str] = &[
    // Literally `value: 1` once per decoded batch.
    "convoy_transport_attack_batches",
    // `decoded.rows.len()` — this frame's rows.
    "convoy_transport_attack_rows",
    // `u64::from(bool)` once per decoded batch.
    "convoy_transport_attack_exact_complete_batches",
    "convoy_transport_attack_incomplete_batches",
    "convoy_transport_attack_hook_installed_batches",
    "convoy_transport_attack_queue_drain_complete_batches",
    // `rows.len()` of the drain this batch carried.
    "convoy_transport_attack_drained_rows",
    // Raised once per decoded batch that failed its witness.
    "convoy_transport_attack_incomplete_witness",
    // Domain 61's own row accounting is per-frame: these count the rows of THIS
    // frame's payload, unlike the engine channel totals the rows carry.
    "air_loss_stat_rows",
    "air_loss_binding_verified_rows",
];

/// Units whose value is a raw sample rather than a quantity. Folded by LAST.
pub const LAST_VALUE_UNITS: &[&str] = &[
    // Percentiles over `write_times_us`, which the producer already
    // accumulates for the whole capture; the last emission is the
    // whole-capture percentile. A max over mid-capture percentiles would be a
    // different and unnamed statistic.
    "rs_sink_time_p50_us",
    "rs_sink_time_p99_us",
    // The last OS error, not a count. (`d75_install_failure_stage` needs no
    // entry: the `_stage` suffix already names it a latched failure code.)
    "heap_sweep_spawn_last_os_error",
    // Epoch identifiers: a truncated 128-bit sweep epoch and d75's terminal
    // epoch. Identifiers, not counts.
    "heap_sweep_epoch",
    "d75_terminal_epoch",
];

/// Units that are always zero by construction but must still be classified, so
/// that a future nonzero value is not summed.
pub const ZERO_GAUGE_UNITS: &[&str] = &["rs_sink_write_in_flight"];

/// Raw failure CONTEXT whose names do not carry a suffix that says so.
///
/// Every one of these is a word read out of the engine at the moment a check
/// refused — a side, a vector's size or capacity, an index, an observed value,
/// an expected value, a type word, the raw bits of an `i32`, seven bytes of a
/// reason string. They are the locator half of the diagnostic channel's
/// contract (see [`LocatorLevel`]) and adding them together produces a number
/// that describes nothing.
///
/// Listed rather than matched, because the suffixes that would catch them
/// (`_size`, `_capacity`, `_observed`, `_abs`) each also end the name of a
/// genuine per-frame count somewhere in the tree: `d75_queue_capacity` is a
/// session constant, `domain47_manager_census_observed` is a census count, and
/// `lw_hour_label_skew_abs` feeds a percentile. A suffix rule is only safe when
/// the suffix names the value's KIND everywhere it appears.
pub const RAW_FAILURE_CONTEXT_UNITS: &[&str] = &[
    // d21 border-war walk.
    "d21_failure_side",
    "d21_failure_manager_index_plus_one",
    "d21_failure_vector_capacity",
    "d21_failure_vector_size",
    "d21_failure_vector_element_index_plus_one",
    "d21_border_war_observed_type_word",
    "d21_border_war_type_word_observed",
    // d57 world-tension detail read: the refused value and the raw bytes of
    // the reason string it came with.
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
    // d61 per-country stat read: which country slot and which field.
    "d61_country_read_failure_country_idx",
    "d61_country_read_failure_field_idx",
    // Observed-vs-expected pairs. Summing either half makes the pair
    // meaningless even when the other half survives.
    "d65_first_failure_observed",
    "d65_first_failure_expected",
    "d83_fail_actual",
    "d83_fail_expected",
    // d47 manager census.
    "domain47_manager_failure_vector",
    "domain47_manager_failure_observed_value",
    "domain47_manager_failure_locator_observed",
    // d48 first invalid progress line.
    "domain48_first_invalid_progress_line_type",
    "domain48_first_invalid_progress_cost_raw_i32_bits",
    "domain48_first_invalid_progress_produced_raw_i32_bits",
    // d54: the value is the error code the reject function returned, not a
    // count of rejects.
    "domain54_exact_orders_group_manager_rejected",
    "domain54_orders_group_carrier_rejected",
];

/// The two `lw_*` units whose value is a session CONSTANT rather than a
/// per-frame span.
///
/// The rest of the lane-wall family is genuinely per-frame and the lane-wall
/// aggregate sums it, which agrees with `PerFrame`. These two do not: they
/// record which prefetch-distance configuration the d53 loops actually used,
/// which does not change within a session. The aggregate keeps the first value
/// and raises a `mixed` flag if a later frame disagrees — strictly more
/// information than any single fold, and consistent with `Cumulative` for a
/// constant. Declaring them here stops the registry claiming `PerFrame` for a
/// value the host deliberately does not sum.
pub const LANE_WALL_SESSION_CONSTANTS: &[&str] = &[
    "lw_prefetch_dist_relation",
    "lw_prefetch_dist_candidates",
];

/// EVERY `domain47_prewarm_*` unit, with the class its producer gives it.
///
/// # Why this family is declared as a whole, and FIRST
///
/// The prewarm family mixes three shapes under one prefix — per-frame counts, a
/// categorical CODE, and a family-presence BITMASK — and nothing in the naming
/// separates them. Two units reached `runs/live_unlock/t36` on the `PerFrame`
/// fall-through and published sums of things that cannot be added:
///
/// * `domain47_prewarm_clock_verdict` — the [0..=4] enum code naming which
///   clock check refused the hour, published as **219**: code 1 (`InFrameHour`)
///   added up once per prewarm. It reads as a count, and it is not one.
/// * `domain47_prewarm_known_families` — the eight-bit `domain47::FAMILY_*`
///   presence mask, published as **20469**. A sum of masks names no family:
///   the readable whole-capture question is WHICH families were ever observed,
///   which is the bitwise OR.
///
/// A third was misclassified in the other direction:
/// `domain47_prewarm_families_dark_without_reason` returns a MASK of families
/// (`FAMILY_ALL & !known_families`, minus the explained ones), and the
/// `_reason` entry in [`RAW_WORD_SUFFIXES`] captured it as a raw word — so a
/// frame that violated the invariant anywhere but last would have disappeared
/// from the manifest entirely.
///
/// This table is consulted BEFORE the suffix rules for exactly that reason: a
/// suffix rule is a naming rule, and this family's names do not encode their
/// kinds. The whole family in one place, each entry stating what its producer
/// makes it, is what makes the next unit's class a decision rather than an
/// accident. Every name the collector emits must appear here — the guard in
/// `tests/diagnostic_unit_accumulation_sweep.rs` reads the emit sites and fails
/// the build for one that does not, so a new sibling cannot repeat the miss.
///
/// # RETAINED CAPTURES ARE UNCHANGED
///
/// The manifest aggregate is computed at capture time and written into the
/// capture. t36 and every earlier run keep the summed figures they recorded;
/// nothing rewrites them and this table cannot. It changes what FUTURE captures
/// publish. A reader comparing across the boundary must know which side a
/// figure came from.
///
/// # What an aggregate of a code or a mask can and cannot answer
///
/// `LastValue` on a code answers "what did the last frame say", and `BitwiseOr`
/// on a mask answers "which bits were EVER set". Neither answers the per-frame
/// question — which hour was refused for which reason, which hour lost which
/// family. That answer needs the per-frame record in the stream, and no
/// whole-capture fold of a single number can substitute for it.
pub const DOMAIN47_PREWARM_UNITS: &[(&str, Accumulation)] = &[
    // --- the two miss-class fixes -----------------------------------------
    // `Domain47PrewarmClockVerdict::code()`: 0 NotScheduled, 1 InFrameHour,
    // 2 Straddled, 3 WrongHour, 4 Unreadable. A code is a name written as a
    // number, exactly like the `_stage` and `_reason` words. t36 published
    // 219 and KEEPS it: this changes what later captures publish, not what
    // that one recorded.
    ("domain47_prewarm_clock_verdict", Accumulation::LastValue),
    // The `domain47::FAMILY_*` presence mask the adopted roots support. Bit n
    // is row kind n+1; two frames observing family bit 0 must read as mask 1,
    // never as 2, which would name a family that was never observed. t36
    // published 20469 and KEEPS it; the OR applies to later captures only.
    ("domain47_prewarm_known_families", Accumulation::BitwiseOr),
    // --- the two the same defect class reached from the other side ---------
    // Also a `domain47::FAMILY_*` mask, not a count: the producer returns
    // `FAMILY_ALL & !known_families` with the explained families cleared. It
    // was `LastValue` by the `_reason` suffix, so a violation on any frame but
    // the last was published as zero — for the one unit whose entire purpose
    // is to make a silent family gap loud.
    (
        "domain47_prewarm_families_dark_without_reason",
        Accumulation::BitwiseOr,
    ),
    // `Domain47ShadowWalkState`: 0 NotRun, 1 WalkedOnly, 2 Compared,
    // 3 Disabled. The word that says whether the six counters beside it are a
    // measurement at all; a sum of its codes says nothing about any frame.
    // Added by lane EJ the same day this table was written, so no retained
    // capture carries the summed spelling.
    ("domain47_prewarm_shadow_walk_state", Accumulation::LastValue),
    // --- what the `sweep_only` residual WAS (lane FC) ----------------------
    // `Domain47ShadowWalkRecheckOutcome`: 0 NotChecked, 1 WalkHit (reached now
    // and identity-compatible — consistent with insertion-timing skew, not
    // proof of it), 2 Refused, 3 WalkMissed, 4 OwnerNotCensused.
    //
    // A PER-PREWARM LOCATOR, not the promotion signal. It says what the worst
    // element of ONE prewarm's residual was, so that a capture carrying a
    // structural candidate names it; the whole-capture question is answered by
    // the three tallies below, whose rule is stated there. Summing this code
    // instead would produce a number whose every value is reachable by adding
    // up benign frames, which is why it is `LastValue`.
    (
        "domain47_prewarm_shadow_walk_recheck_outcome",
        Accumulation::LastValue,
    ),
    // The element that code describes: the engine `char_id` and the map it
    // belongs to (`Domain47ShadowWalkMapAxis`: 1 leader map, 2 advisor map,
    // 0 no element). `_char_id` would reach `LastValue` through the `_id`
    // suffix and `_map` would reach `BitwiseOr` through the mask suffixes —
    // an axis code ORed across frames is a number naming no map — so both are
    // stated here, ahead of every suffix rule.
    (
        "domain47_prewarm_shadow_walk_sweep_only_char_id",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_map",
        Accumulation::LastValue,
    ),
    // --- WHERE that element is (lane FS) -----------------------------------
    // Eight words describing the SAME element the two above name, emitted in
    // the same conditional. Every one of them is a RAW OBSERVATION of one
    // element — two addresses, two codes, two engine identifiers, one map
    // size, two examined-counts — so every one is `LastValue`: adding an
    // address to another address, or a `Censused` code to a `NotCensused` one,
    // produces a number with no referent, which is the defect this table
    // exists to end.
    //
    // THE TWO COUNTS ARE `LastValue` AND THEIR NEIGHBOURS `_recheck_*_count`
    // ARE `PerFrame`, and the difference is not an oversight. The tally counts
    // ELEMENTS, one per re-check, so its capture-wide sum is the figure the
    // promotion gate reads. These two describe the ONE latched element — how
    // many bodies the re-check examined for IT — so summing them across
    // prewarms would answer no question at all.
    //
    // `_address` and `_owner_address` would each reach `LastValue` through
    // `RAW_WORD_SUFFIXES`, and `_owner_char_id` through `_id`; they are stated
    // here for the reason the table states everything — a member of this family
    // whose class is decided by a suffix rule is a member whose class lives
    // somewhere else. `_owner_censused`, `_owner_map`, `_owner_map_size`,
    // `_candidate_bodies` and `_candidate_entries` reach NO rule and would land
    // on the summed default without an entry.
    (
        "domain47_prewarm_shadow_walk_sweep_only_address",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_address",
        Accumulation::LastValue,
    ),
    // `Domain47ShadowWalkOwnerBody`: 0 NotDerived, 1 Censused, 2 NotCensused.
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_censused",
        Accumulation::LastValue,
    ),
    (
        "domain47_prewarm_shadow_walk_sweep_only_owner_char_id",
        Accumulation::LastValue,
    ),
    // `Domain47ShadowWalkOwnerMap`: 0 NotAttempted, 1 Holds, 2 CleanWithout,
    // 3 Refused.
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
    // --- raw codes and identifiers, unchanged ------------------------------
    // These four already resolved to `LastValue` through `RAW_WORD_SUFFIXES`
    // (`_reason`, `_kind`, `_id`, `_ordinal`) and are restated here because
    // this table now runs first: an entry omitted would be a member of the
    // family whose class lives somewhere else, which is the state this table
    // exists to end. Each is a `Domain47PrewarmHoldReason` /
    // `Domain47FamilyGapReason` / `Domain47PortraitFailure` code or an engine
    // identifier.
    ("domain47_prewarm_hold_reason", Accumulation::LastValue),
    ("domain47_prewarm_family_gap_reason", Accumulation::LastValue),
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
    // --- the identity axis and its evidence probe (lane FA) ----------------
    // `Domain47PortraitIdentityDetail`: 0 NoIdentityFailure, 1..=9 the residual
    // paths of the identity `std::string` read. A code, exactly like the
    // `_kind` word it sits beside — but `_detail` is in NO suffix list, so
    // without this entry it would land on the `PerFrame` default and be SUMMED,
    // which is precisely the failure this table was built to end. It is the
    // first new member of the family since the table was written and it is
    // therefore also the test of whether the table works.
    (
        "domain47_prewarm_portrait_failure_detail",
        Accumulation::LastValue,
    ),
    // Whether `BATTLE_PLAN_D47_PORTRAIT_PROBE` armed the capture. A per-process
    // constant republished on every frame, so its sum is a frame count wearing
    // a boolean's name — the `shadow_walk_state` argument exactly.
    (
        "domain47_prewarm_portrait_probe_armed",
        Accumulation::LastValue,
    ),
    // The five probe words, emitted only on an armed capture. Every one of them
    // is a RAW OBSERVATION of one `std::string` header — two header words, two
    // outcome codes, one byte — and arithmetic over frames on any of them is
    // meaningless in the way `d43_resource_failure_resource_index` was
    // meaningless at `u32::MAX * 361`. None of the five ends in a suffix any
    // rule recognises, so each needs its entry here.
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
    // `Domain47PortraitReaderReason` (lane FI): WHICH refusal point of the
    // SHARED `std::string` reader refused the non-empty identity — 0 no reader
    // refusal, 1..=13 one point each. A code, like the two beside it.
    //
    // Code 11 (`TextNotDecodable`) CHANGED MEANING on 2026-08-08 (lane FP):
    // the portrait identity is now read under a tolerant decode policy, so 11
    // means the bytes failed BOTH UTF-8 and Windows-1252 rather than "the bytes
    // are not ASCII". Code 13 (`DecodedTextOverMax`) is new and is reachable
    // only through that fallback's expansion. Captures t37..t40b carry 11 with
    // the old meaning.
    //
    // It ends in `_reason`, so `RAW_WORD_SUFFIXES` would land it on `LastValue`
    // anyway — and that is precisely why it needs an entry: this table is
    // consulted FIRST and is the family's authority, so a member of it left out
    // of the table would have its class decided somewhere else, which is the
    // state the table exists to end. The producer-side sweep enforces exactly
    // that membership.
    (
        "domain47_prewarm_portrait_probe_reader_reason",
        Accumulation::LastValue,
    ),
    // --- per-frame quantities, unchanged -----------------------------------
    // Booleans raised for THIS frame: whether the prewarm ran, took the
    // negative-cache branch, was a refresh, and whether that refresh held the
    // previous good roots. Their sums are frame counts and are read as such.
    ("domain47_prewarm_ran", Accumulation::PerFrame),
    ("domain47_prewarm_negative_cache_hit", Accumulation::PerFrame),
    ("domain47_prewarm_refresh", Accumulation::PerFrame),
    (
        "domain47_prewarm_refresh_held_roots",
        Accumulation::PerFrame,
    ),
    // Counts the census produced. `portrait_rows` is a published row count, in
    // the same convention as every `domain47_adapter_*_rows` unit beside it.
    (
        "domain47_prewarm_portrait_failed_characters",
        Accumulation::PerFrame,
    ),
    ("domain47_prewarm_portrait_rows", Accumulation::PerFrame),
    // How many of THIS frame's portrait rows carried an identity the producer
    // decoded through its Windows-1252 fallback (owner ruling 2026-08-08). A
    // published row count exactly like `portrait_rows` above, and summed the
    // same way — the manifest total is "portrait identities that needed the
    // fallback, across the capture", which is the figure that says how much of
    // the world the tolerant decode recovered.
    //
    // NOT a code, despite the `_count` ending: `_count` is in no suffix list,
    // so without this entry it would land on the `PerFrame` default and happen
    // to be right for the wrong reason. Stated here because this table is the
    // family's authority and a member left out of it has its class decided
    // somewhere else.
    (
        "domain47_prewarm_portrait_cp1252_count",
        Accumulation::PerFrame,
    ),
    // WHAT A REFUSED ROLE CONTAINER COST (lane FU, 2026-08-08).
    //
    // `_gap_reason` and `_refused_map` are CODES (a `Domain47FamilyGapReason`
    // and a `Domain47ShadowWalkMapAxis`) and `_refused_body` is an ADDRESS;
    // summing any of the three produces a number with no referent, which is
    // exactly what this table exists to stop. `LastValue` keeps the most recent
    // frame's answer, the same class `domain47_prewarm_clock_verdict` and the
    // `shadow_walk` locator words take.
    //
    // The two per-axis COUNTS are counts and are summed. THE CAPTURE-WIDE FIGURE
    // IS "how many role containers this run could not read THAT COST A FAMILY",
    // and the qualifier is not pedantry: a prewarm whose adoption was blocked
    // zeroes these two with every other family word, because it published no
    // families to lose. `domain47_prewarm_shadow_walk_refused_count` is the raw
    // walk counter and records every refusal, blocked or not, so the two agree
    // on an adopting run and diverge by exactly the blocked frames' refusals.
    // Reading either as the other undercounts or over-attributes (Codex
    // 019fe24e). A nonzero `_gap_reason` says which families the counted ones
    // cost.
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
    // The shadow walk's six counters. Per-walk observations, summed the way
    // the adapter row counters are, and meaningless without
    // `domain47_prewarm_shadow_walk_state` above.
    (
        "domain47_prewarm_shadow_walk_leader_count",
        Accumulation::PerFrame,
    ),
    (
        "domain47_prewarm_shadow_walk_advisor_count",
        Accumulation::PerFrame,
    ),
    // THE POPULATION A/B (lane FU, 2026-08-08): what the container walk adopted
    // and what the heap sweep WOULD have adopted, on one prewarm.
    //
    // FOUR WORDS, ONE EVENT, ONE CLASS. `LastValue`, because these are gauges
    // over a root cache and not per-frame events — summing one side of a
    // comparison and gauging the other makes two IDENTICAL prewarms of ten
    // advisor roots publish 20 against 10 (Codex 019fe214, finding 1). And all
    // four together, in one conditional arm, because folding a `LastValue`
    // production half published every frame against a shadow half published only
    // on compared frames lets a run describe two different prewarms and report a
    // difference that never existed (Codex 019fe238, finding 1).
    //
    // The `_walk_` pair deliberately duplicates
    // `domain47_prewarm_country_leader_role_count` /
    // `_advisor_root_count`, which stay as ordinary every-frame gauges. Two
    // words for one number is the price of the pairing being atomic.
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
    // Sweep hits the shadow's double read found torn. A COUNT and not a gauge,
    // so it is summed: this one is not half of a pair, and the capture-wide
    // figure a reader wants is "how many hits the comparison never saw across
    // the run". A nonzero total disqualifies a zero `sweep_only` read off it.
    (
        "domain47_prewarm_sweep_shadow_torn_hits",
        Accumulation::PerFrame,
    ),
    // The production country-leader root count, the counterpart of
    // `_sweep_shadow_leader_roots`. A gauge over the adopted root cache,
    // exactly like `domain47_prewarm_advisor_root_count` beside it — the two
    // halves of a comparison must accumulate the same way or the comparison is
    // between two different quantities.
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
    // The re-check tally (lane FC). Per-element verdicts, one per sweep-only
    // element this prewarm classified, so the capture-wide figure a reader
    // wants is exactly their sum: "how many residual events across the whole
    // run were reached on re-check, and how many were candidate structural
    // gaps".
    //
    // `miss == 0` IS NOT THE PROMOTION SIGNAL, and reading it as one is the
    // trap this note exists to close: it also holds when every re-check
    // REFUSED, and when the per-prewarm cap of 16 left residual elements
    // unclassified. The figure to read is `hit == sweep_only` over the capture
    // — every residual element classified AND every one of them reached — with
    // `miss` and `refused` both zero. A nonzero `miss` is the sweep's reprieve;
    // a nonzero `refused`, or a `hit + miss + refused` short of `sweep_only`,
    // is a capture that did not answer the question. And a run that clears all
    // of it is EVIDENCE across captures, never proof on one frame: see
    // `Domain47ShadowWalkRecheckOutcome::WalkHit` for the ABA residual no
    // re-check can exclude.
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
    // --- the seven read back out of the RETAINED cache ---------------------
    // `LastValue` by owner ruling (2026-08-08): each is read from
    // `CompactDomain47PrewarmCache` rather than computed from this frame's
    // work, so a hold frame republishes the value the last censusing frame
    // left there, and summing republications produced frame-weighted totals —
    // t36 published `stable_root_count` as 4,849,311, ~22k roots counted
    // again on each of 219 prewarms. The manifest now answers "what was the
    // roster size", the `convoy_transport_attack_resolver_member_bindings`
    // Gauge precedent. RETAINED CAPTURES KEEP THEIR RECORDED SUMS — manifests
    // are folded at finalization and never recomputed — so t36's figures
    // stand as written; only future manifests carry the gauge reading.
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

/// THE authority: how the whole-capture figure for `name` is computed.
///
/// Every aggregation site calls this. The classification is by name because
/// the name is the only thing that survives the wire, and because the two
/// producer families that keep state across frames name their units with a
/// reserved prefix.
///
/// Resolution order, and each step's reason:
///
/// 0. [`DOMAIN47_PREWARM_UNITS`] — the one family declared as a whole, ahead
///    of the suffix rules, because its names do not encode their kinds and two
///    of them reached a manifest as sums of codes and masks.
/// 1. `*_sample` — the established raw-word convention, already relied on by
///    `bt_*_raw_sample` and `so_*_sample`.
/// 2. `d69_failed_slot_mask*` / `d69_stale_slot_mask` — the established
///    bitmask convention.
/// 3. The explicit tables above.
/// 4. Inside a [`STATEFUL_UNIT_PREFIXES`] family, anything left is
///    `Undeclared`, because a default of `PerFrame` there would be a guess
///    about producer-retained state.
/// 5. Everything else is `PerFrame`: a domain data unit's value is computed
///    from the payload of the frame it rode in.
///
/// Step 5 is a DEFAULT and not a declaration. [`declared_unit_accumulation`]
/// is the same resolution with that step withheld, so a guard can tell a unit
/// somebody classified from a unit that merely landed on the summed side.
pub fn unit_accumulation(name: &str) -> Accumulation {
    declared_unit_accumulation(name).unwrap_or(Accumulation::PerFrame)
}

/// The class a RULE gave `name`, or `None` when no rule did and
/// [`unit_accumulation`] fell through to its `PerFrame` default.
///
/// This is the same resolution as `unit_accumulation` — there is still exactly
/// one authority, and this is its body — with the final default withheld so
/// that "declared per-frame" and "defaulted to per-frame" can be told apart.
/// They fold identically and always will; the distinction is for the guards,
/// which is the only way a family can be required to classify its own units.
pub fn declared_unit_accumulation(name: &str) -> Option<Accumulation> {
    // AHEAD of the suffix rules, and the position is the point: this family
    // carries codes and masks under names that say neither, and one of its
    // masks was being taken for a raw word by the `_reason` suffix.
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
    // The recorder's original hard-coded case: the numbered per-slot forms end
    // in a digit, so the `_mask` suffix does not reach them.
    if name.starts_with("d69_failed_slot_mask_") {
        return Some(Accumulation::BitwiseOr);
    }
    // `<prefix>_sha256_0..3`: one SHA-256 carried as four u64 words, and the
    // digest-group prefix the producer passes to build them. d75's terminal
    // payload and d64's terminal envelope/del2 digests are all this shape.
    // Adding digest words together is the `_hash` defect with more steps.
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

/// Is this name inside a family where an unregistered unit is undeclared
/// rather than assumed per-frame?
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

    // -----------------------------------------------------------------------
    // Accumulation
    // -----------------------------------------------------------------------

    /// The measured `runs/live_unlock/t15` series, not a reconstruction.
    ///
    /// Run-length encoding of `callbacks_seen` as read from the 396 `CTA2`
    /// batch headers in that capture's `collector_stream.bin`. The series is
    /// monotonically non-decreasing from 1 to 107 because the counter is a
    /// process-global `AtomicU64` reported at its absolute value every frame.
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

    /// The measured t15 defect, replayed through the authority.
    ///
    /// The t15 manifest published
    /// `convoy_transport_attack_dropped_events = 19470` and
    /// `convoy_transport_attack_callbacks_seen = 19470`. Both are the SUM of
    /// the series below over 396 frames. The real population is 107 — the
    /// counter's final absolute value — and 19,470 was quoted to the project
    /// as an event count.
    ///
    /// MUTATION: classify `convoy_transport_attack_*` telemetry as `PerFrame`,
    /// or make `Accumulation::Cumulative` fold by `saturating_add`, and this
    /// fails on the 19470 vs 107 assertion.
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

        // The figure this replaces, named so the regression is explicit rather
        // than merely absent.
        let summed = series.iter().fold(0u64, |a, b| a + b);
        assert_eq!(summed, 19_470, "the published t15 figure");
        assert_ne!(summed, 107);
    }

    /// Law: the eight CTA2 v3 words are classified, and every one of them is
    /// CUMULATIVE.
    ///
    /// They have the same shape as the aggregates they split: process-global
    /// `AtomicU64`s reset only at hook install, republished at their absolute
    /// value on every batch. An unclassified name falls through to `PerFrame`
    /// and is SUMMED, which is exactly the arithmetic that published t15's 107
    /// drops as 19,470 -- and a unit added to diagnose a loss is the worst
    /// possible place to reintroduce it.
    ///
    /// The two `_max` words matter for a second reason: they are game-hour
    /// DISTANCES. Summing a running maximum over 644 batches reports a
    /// disagreement no receipt was ever at.
    ///
    /// MUTATION RUN: remove
    /// `"convoy_transport_attack_dropped_receipt_hour_mismatch"` from
    /// `CUMULATIVE_UNITS`. OBSERVED: FAILED, `left: PerFrame, right:
    /// Cumulative`.
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

        // And the fold really is last-value, not sum, on the same t15 series
        // the aggregate above is checked against.
        let series = t15_callbacks_seen_series();
        let class = unit_accumulation("convoy_transport_attack_dropped_receipt_hour_mismatch");
        let mut running: Option<u64> = None;
        for value in &series {
            running = Some(class.fold(running, *value));
        }
        assert_eq!(running, Some(107));
    }

    /// A per-frame unit must still be summed. Guards the opposite mistake:
    /// "fix the number by folding everything with MAX".
    ///
    /// MUTATION: fold `PerFrame` by `max` and this fails.
    #[test]
    fn a_per_frame_unit_is_still_summed() {
        let class = unit_accumulation("convoy_transport_attack_rows");
        assert_eq!(class, Accumulation::PerFrame);
        let mut running: Option<u64> = None;
        for value in [3u64, 1, 4, 1, 5] {
            running = Some(class.fold(running, value));
        }
        assert_eq!(running, Some(14));

        // And a name from no declared family at all.
        let generic = unit_accumulation("fuel_rows_decoded");
        assert_eq!(generic, Accumulation::PerFrame);
        assert_eq!(generic.fold(Some(10), 5), 15);
    }

    /// The producer's own doc comment says the `rs_*` counters are cumulative
    /// and that the host aggregates them by MAX. Until now the host summed
    /// them.
    ///
    /// MUTATION: drop any `rs_` name from `CUMULATIVE_UNITS` and it becomes
    /// `Undeclared` (because `rs_` is a stateful prefix), which this rejects.
    #[test]
    fn every_scheduler_probe_unit_is_classified_and_none_of_them_is_summed() {
        // The exact roster the collector emits, transcribed from
        // `append_compact_scheduler_probe_record`.
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

    /// The property that makes the classification enforceable rather than
    /// advisory: inside a family whose producer keeps state across frames, a
    /// name nobody classified is UNDECLARED, not silently summed.
    ///
    /// MUTATION: make `unit_accumulation` fall through to `PerFrame` for
    /// unregistered stateful names and this fails.
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
        // Outside those families the default stays PerFrame, which is what the
        // per-frame payload decoders genuinely produce.
        assert!(!is_stateful_unit_family("weather_rows_total"));
        assert_eq!(
            unit_accumulation("weather_rows_total"),
            Accumulation::PerFrame
        );
    }

    /// `Undeclared` must not change any arithmetic, or classifying a unit
    /// later would silently move a published figure.
    #[test]
    fn undeclared_folds_exactly_like_per_frame() {
        for (running, incoming) in [(None, 7u64), (Some(3), 4), (Some(u64::MAX), 1)] {
            assert_eq!(
                Accumulation::Undeclared.fold(running, incoming),
                Accumulation::PerFrame.fold(running, incoming)
            );
        }
    }

    /// The pre-existing conventions the host had hard-coded are now expressed
    /// in the same authority, so there is one rule rather than three.
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
        // OR, not sum: slot 0 failing twice is mask 1, never mask 2.
        assert_eq!(Accumulation::BitwiseOr.fold(Some(1), 1), 1);
        // LAST, not sum: a raw word survives.
        assert_eq!(Accumulation::LastValue.fold(Some(0xDEAD), 0xBEEF), 0xBEEF);
    }

    /// A gauge is a peak, not a total, and never a sum.
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

    /// D43's contract-census widths are gauges, and the suffix rules alone do
    /// NOT get them there: `_countries` and `_count` match no suffix table, so
    /// without their `GAUGE_UNITS` entries `unit_accumulation` falls through to
    /// `PerFrame` and a capture would publish a 439-country world as a
    /// six-figure country count.
    ///
    /// Pinned as a test rather than left to the table because these three
    /// exist to be COMPARED with each other -- scope against snapshot against
    /// the engine's outer width -- and a comparison between one summed number
    /// and one gauge is not a comparison at all.
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
        // The two failure words next to them are raw codes, reached by suffix.
        assert_eq!(
            unit_accumulation("d43_contract_failure_stage"),
            Accumulation::LastValue
        );
        assert_eq!(
            unit_accumulation("d43_contract_failure_tag_db_id"),
            Accumulation::LastValue
        );
    }

    /// The d47 prewarm family's codes and masks, pinned against the figures
    /// `runs/live_unlock/t36` actually published.
    ///
    /// Both were on the `PerFrame` fall-through, so the manifest carried a sum
    /// of enum codes and a sum of family masks. The arithmetic is reproduced
    /// here from the capture's own numbers so the test states what was wrong
    /// rather than only what is now expected.
    ///
    /// MUTATION: delete either unit's [`DOMAIN47_PREWARM_UNITS`] entry and it
    /// falls back to the summed default, which this rejects — and
    /// `every_d47_prewarm_unit_declares_its_class` names it as undeclared.
    #[test]
    fn the_d47_prewarm_code_and_mask_units_are_not_summed() {
        // The verdict is a code: t36 ran 219 prewarms, every one of them
        // `InFrameHour` (code 1), and published 219 — a figure that reads as a
        // count of something and is a repeated code.
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

        // The presence mask is a mask: OR, never sum. Two frames observing the
        // same seven families must read as those seven families.
        let families = unit_accumulation("domain47_prewarm_known_families");
        assert_eq!(families, Accumulation::BitwiseOr);
        // A series of the shape t36 carried — many frames observing everything
        // but the portrait family (`FAMILY_ALL & !FAMILY_CHARACTER_PORTRAIT`,
        // 0x7F), then one that observed all eight. The exact per-frame
        // composition of that capture is not reconstructed here; what is
        // reproduced is the arithmetic, which is what the class decides.
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
        // The same series under the class it had: a number naming no family.
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

        // The invariant unit is a mask too, and was reached by the `_reason`
        // suffix. Under `LastValue` a violation on any frame but the last is
        // published as zero; under the OR it survives.
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

        // The shadow walk's state word is a code on the same footing as the
        // clock verdict: its sum says nothing about any frame.
        let state = unit_accumulation("domain47_prewarm_shadow_walk_state");
        assert_eq!(state, Accumulation::LastValue);
        assert_eq!(state.fold(Some(2), 2), 2);
    }

    /// The sweep-only classification's six units (lane FC): three raw words
    /// naming ONE element and three per-element tallies.
    ///
    /// Each half is here because the suffix rules would get it WRONG on its own,
    /// which is the whole reason `DOMAIN47_PREWARM_UNITS` runs first:
    ///
    ///  * `_map` ends in a [`BITMASK_SUFFIXES`] spelling, so without its entry
    ///    the axis code would be OR-ed across frames — a leader-map frame (1)
    ///    and an advisor-map frame (2) would publish 3, which names no map.
    ///  * `_outcome` is in NO suffix list, so it would land on the summed
    ///    default: four `WalkHit` frames would publish 4, indistinguishable from
    ///    one `OwnerNotCensused`, and 4 is the code that keeps the sweep.
    ///  * the three counts ARE per-element quantities and their sum over a
    ///    capture is the figure the promotion gate reads, so they must NOT be
    ///    dragged to `LastValue` beside their neighbours.
    ///
    /// MUTATION: delete any of the six entries and this fails on that unit.
    /// The class assertions alone would NOT catch deleting
    /// `..._sweep_only_char_id`, whose `_id` suffix independently resolves to
    /// `LastValue` (Codex 019fde9b, finding 4) — so membership in the table is
    /// asserted directly for all six, which is the property this family's rule
    /// actually demands.
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
                DOMAIN47_PREWARM_UNITS
                    .iter()
                    .any(|(unit, _)| *unit == name),
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

    /// The eight words that say WHERE the `sweep_only` element is (lane FS)
    /// are declared IN the table, and every one of them is `LastValue`.
    ///
    /// They describe ONE element — two addresses, two codes, an engine
    /// identifier, a map size and two examined-counts — and five of the eight
    /// reach NO suffix rule at all, so without their entries they would land on
    /// the summed default. A summed address is a number naming no object; a
    /// summed `Domain47ShadowWalkOwnerMap` code turns two `Holds` frames into
    /// the code for a body that walked cleanly WITHOUT the entry, which is the
    /// opposite conclusion.
    ///
    /// The two `_candidate_*` words are the trap this test exists for: they
    /// count, they sit beside three `_count` units that are correctly
    /// `PerFrame`, and they are NOT per-frame quantities — they describe the
    /// one latched element, so summing them across prewarms answers nothing.
    ///
    /// MUTATION: delete any of the eight entries and this fails on that unit;
    /// class either `_candidate_*` as `PerFrame` and the last assertion fails.
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
        // What the default would have done to the owner-map code: two frames
        // reporting `Holds` (1) publish 2, which is `CleanWithout` — the
        // opposite finding, and the one that would retire the sweep.
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

    /// THE TWO HALVES OF A COMPARISON ACCUMULATE THE SAME WAY.
    ///
    /// d47's sweep shadow publishes what the heap sweep WOULD have adopted
    /// beside what the container walk DID, so a transition capture can compare
    /// populations directly. That comparison is only meaningful if both sides
    /// are the same kind of quantity across a run: summing one and gauging the
    /// other makes two identical prewarms of ten advisor roots publish 20
    /// against 10, and the manifest reports a population change that never
    /// happened (Codex 019fe214, finding 1).
    ///
    /// MUTATION: set either `_sweep_shadow_*_roots` entry to `PerFrame`, or
    /// either production counterpart, and this fails.
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
            // The every-frame gauges the `_walk_` pair duplicates are the same
            // class too, so a reader who compares against them instead is not
            // comparing a sum with a gauge.
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

    /// Every unit in [`DOMAIN47_PREWARM_UNITS`] resolves to the class the table
    /// gives it, and no member of the family is left to the default.
    ///
    /// The producer-side half of this guard — that every name the collector
    /// EMITS is in the table — lives in
    /// `tests/diagnostic_unit_accumulation_sweep.rs`, which reads the emit
    /// sites. This half is the crate-local property: the table is reachable,
    /// self-consistent, and complete over itself.
    ///
    /// MUTATION: delete any entry and the name it named falls through to the
    /// `PerFrame` default, so `declared_unit_accumulation` returns `None` and
    /// the sweep guard fails.
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
        // The table is the family's authority, so a member of it must never be
        // decided by anything else. Four entries END in a raw-word suffix and
        // agree with it; the fifth kind — the masks — deliberately do not, and
        // that disagreement is the defect this table exists to settle.
        assert_eq!(
            declared_unit_accumulation("domain47_prewarm_families_dark_without_reason"),
            Some(Accumulation::BitwiseOr),
            "the table must win over the `_reason` suffix for this name"
        );
    }

    /// `declared_unit_accumulation` must distinguish a classified unit from one
    /// that merely landed on the summed default — the property every
    /// explicit-classification guard rests on.
    ///
    /// MUTATION: return `Some(Accumulation::PerFrame)` from the final arm of
    /// `declared_unit_accumulation` and this fails, as does the sweep guard.
    #[test]
    fn a_defaulted_unit_is_distinguishable_from_a_declared_one() {
        assert_eq!(
            declared_unit_accumulation("weather_rows_total"),
            None,
            "nothing classifies this name; it is summed by default"
        );
        assert_eq!(unit_accumulation("weather_rows_total"), Accumulation::PerFrame);
        assert_eq!(
            declared_unit_accumulation("domain47_prewarm_ran"),
            Some(Accumulation::PerFrame),
            "declared per-frame, which is not the same statement as defaulted"
        );
        // Every name in the tree resolves identically through both entry
        // points; the second only says whether a rule was involved.
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

    /// The tables must not disagree with each other; a name in two of them
    /// would make the classification depend on the order of the `if`s.
    #[test]
    fn the_classification_tables_are_disjoint_and_self_consistent() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for (name, _) in DOMAIN47_PREWARM_UNITS {
            assert!(seen.insert(name), "{name} is classified by more than one table");
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

    /// The raw-observation category, pinned against the figures
    /// `runs/live_unlock/t15` actually published.
    ///
    /// Each case is `(unit, published_sum, frames, true_value)` read out of
    /// that capture's manifest. A raw observation repeated every frame sums to
    /// a number with no referent; the whole-capture figure is the observation.
    ///
    /// MUTATION: drop any of these suffixes from `RAW_WORD_SUFFIXES` and the
    /// unit becomes `PerFrame`, which this rejects.
    #[test]
    fn a_raw_observation_is_never_summed() {
        // (name, what t15 published, frames it rode, the one observed value)
        const T15: &[(&str, u64, u64, u64)] = &[
            // u32::MAX * 361: the producer's "never written" reset sentinel on
            // every frame of the domain.
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
            // Stage code 513, summed over 351 frames.
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

        // The saturating cases: a hash added to itself pins at u64::MAX, so the
        // published number carries no information about the hash at all.
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
        assert_eq!(summed, u64::MAX, "which is what t15 published for three hashes");
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
        // First observation is the value itself for every class, so a unit
        // seen exactly once reads the same whatever it is classified as.
        for class in Accumulation::ALL {
            assert_eq!(class.fold(None, 42), 42);
        }
    }

    /// The outside-build segment tables are ONE table read two ways: the host
    /// resolves a wire unit name back to a segment name by the code embedded in
    /// the name, so `OUTSIDE_BUILD_UNIT_NAMES[code]` must be the name for that
    /// code and nothing else.
    ///
    /// MUTATION: reorder two entries of `OUTSIDE_BUILD_SEGMENTS`, or drop one
    /// name from `OUTSIDE_BUILD_UNIT_NAMES`, and this fails — which is exactly
    /// the shape that would relabel one segment's milliseconds as another's.
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

    /// Every outside-build unit is a per-frame duration, so the registry must
    /// say `PerFrame`. `lw_o` is not inside a stateful family and `_us` is not
    /// a raw-word suffix, so this is what the fall-through gives — pinned
    /// because a later suffix or prefix rule could silently capture the family
    /// and turn a sum of durations into a MAX or a last value.
    ///
    /// MUTATION: add `"_us"` to `RAW_WORD_SUFFIXES` and this fails.
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
}
