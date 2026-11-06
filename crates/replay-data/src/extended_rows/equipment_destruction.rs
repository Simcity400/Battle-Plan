use super::*;

/// Stable recording values, independent of the supported executable's addresses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[derive(replay_spool::Spooled)]
pub enum EquipmentDestructionCause {
    #[default]
    Unknown = 0,
    LandCombat = 1,
    NavalTransport = 2,
    Immediate = 3,
    Attrition = 4,
    Garrison = 5,
    AirNuclear = 6,
    AirAccident = 7,
    AirStrategic = 8,
    AirMissionAa = 9,
    AirLandCombatCas = 10,
    AirRaid = 11,
    AirNavalAa = 12,
    DivisionDestroyed = 13,
    DivisionOverrun = 14,
    DivisionDeleted = 15,
    DivisionDisbanded = 16,
    DivisionCapitulation = 17,
    DivisionScripted = 18,
    DivisionTemplateRemoved = 19,
    ParadropDisruption = 20,
    DivisionMerged = 21,
    ExileEquipment = 22,
    VolunteerReturnEquipment = 23,
    CapitulationEscapeEquipment = 24,
    /// Division pools only, labelled "Scripted equipment reductions".
    ScriptedEquipmentFraction = 25,
    /// National stockpile, labelled "Scripted stockpile reductions".
    ScriptedStockpileFraction = 26,
}

impl EquipmentDestructionCause {
    pub const ALL: [Self; 27] = [
        Self::Unknown,
        Self::LandCombat,
        Self::NavalTransport,
        Self::Immediate,
        Self::Attrition,
        Self::Garrison,
        Self::AirNuclear,
        Self::AirAccident,
        Self::AirStrategic,
        Self::AirMissionAa,
        Self::AirLandCombatCas,
        Self::AirRaid,
        Self::AirNavalAa,
        Self::DivisionDestroyed,
        Self::DivisionOverrun,
        Self::DivisionDeleted,
        Self::DivisionDisbanded,
        Self::DivisionCapitulation,
        Self::DivisionScripted,
        Self::DivisionTemplateRemoved,
        Self::ParadropDisruption,
        Self::DivisionMerged,
        Self::ExileEquipment,
        Self::VolunteerReturnEquipment,
        Self::CapitulationEscapeEquipment,
        Self::ScriptedEquipmentFraction,
        Self::ScriptedStockpileFraction,
    ];

    pub const fn is_air(self) -> bool {
        matches!(
            self,
            Self::AirNuclear
                | Self::AirAccident
                | Self::AirStrategic
                | Self::AirMissionAa
                | Self::AirLandCombatCas
                | Self::AirRaid
                | Self::AirNavalAa
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::LandCombat => "land_combat",
            Self::NavalTransport => "naval_transport",
            Self::Immediate => "immediate",
            Self::Attrition => "attrition",
            Self::Garrison => "garrison",
            Self::AirNuclear => "air_nuclear",
            Self::AirAccident => "air_accident",
            Self::AirStrategic => "air_strategic",
            Self::AirMissionAa => "air_mission_aa",
            Self::AirLandCombatCas => "air_land_combat_cas",
            Self::AirRaid => "air_raid",
            Self::AirNavalAa => "air_naval_aa",
            Self::DivisionDestroyed => "division_destroyed",
            Self::DivisionOverrun => "division_overrun",
            Self::DivisionDeleted => "division_deleted",
            Self::DivisionDisbanded => "division_disbanded",
            Self::DivisionCapitulation => "division_capitulation",
            Self::DivisionScripted => "division_scripted",
            Self::DivisionTemplateRemoved => "division_template_removed",
            Self::ParadropDisruption => "paradrop_disruption",
            Self::DivisionMerged => "division_merged",
            Self::ExileEquipment => "exile_equipment",
            Self::VolunteerReturnEquipment => "volunteer_return_equipment",
            Self::CapitulationEscapeEquipment => "capitulation_escape_equipment",
            Self::ScriptedEquipmentFraction => "scripted_equipment_fraction",
            Self::ScriptedStockpileFraction => "scripted_stockpile_fraction",
        }
    }

    pub const fn is_division_deletion(self) -> bool {
        matches!(
            self,
            Self::Unknown
                | Self::DivisionDestroyed
                | Self::DivisionOverrun
                | Self::DivisionDeleted
                | Self::DivisionDisbanded
                | Self::DivisionCapitulation
                | Self::DivisionScripted
                | Self::DivisionTemplateRemoved
        )
    }
}

/// A drain before its branch's first complete interval. Consumers begin
/// counting at the first subsequent complete drain instead of claiming an
/// unknown campaign total for this deliberately uncounted startup interval.
/// Frozen recording string: stored in table availability and matched exactly
/// by consumers. Rewording silently classifies old startup intervals as withheld.
pub const EQUIPMENT_DESTRUCTION_PARTIAL_REASON: &str =
    "equipment destruction first/branch interval is partial";

impl TryFrom<u8> for EquipmentDestructionCause {
    type Error = io::Error;

    fn try_from(value: u8) -> io::Result<Self> {
        Ok(match value {
            0 => Self::Unknown,
            1 => Self::LandCombat,
            2 => Self::NavalTransport,
            3 => Self::Immediate,
            4 => Self::Attrition,
            5 => Self::Garrison,
            6 => Self::AirNuclear,
            7 => Self::AirAccident,
            8 => Self::AirStrategic,
            9 => Self::AirMissionAa,
            10 => Self::AirLandCombatCas,
            11 => Self::AirRaid,
            12 => Self::AirNavalAa,
            13 => Self::DivisionDestroyed,
            14 => Self::DivisionOverrun,
            15 => Self::DivisionDeleted,
            16 => Self::DivisionDisbanded,
            17 => Self::DivisionCapitulation,
            18 => Self::DivisionScripted,
            19 => Self::DivisionTemplateRemoved,
            20 => Self::ParadropDisruption,
            21 => Self::DivisionMerged,
            22 => Self::ExileEquipment,
            23 => Self::VolunteerReturnEquipment,
            24 => Self::CapitulationEscapeEquipment,
            25 => Self::ScriptedEquipmentFraction,
            26 => Self::ScriptedStockpileFraction,
            _ => return Err(invalid("unsupported equipment destruction cause")),
        })
    }
}

impl NativeValue for EquipmentDestructionCause {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(value) => Self::try_from(value),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

impl From<engine_events::equipment_destruction::Cause> for EquipmentDestructionCause {
    fn from(cause: engine_events::equipment_destruction::Cause) -> Self {
        use engine_events::equipment_destruction::Cause;
        match cause {
            Cause::Unknown => Self::Unknown,
            Cause::LandCombat => Self::LandCombat,
            Cause::NavalTransport => Self::NavalTransport,
            Cause::Immediate => Self::Immediate,
            Cause::Attrition => Self::Attrition,
            Cause::Garrison => Self::Garrison,
            Cause::AirNuclear => Self::AirNuclear,
            Cause::AirAccident => Self::AirAccident,
            Cause::AirStrategic => Self::AirStrategic,
            Cause::AirMissionAa => Self::AirMissionAa,
            Cause::AirLandCombatCas => Self::AirLandCombatCas,
            Cause::AirRaid => Self::AirRaid,
            Cause::AirNavalAa => Self::AirNavalAa,
            Cause::DivisionDestroyed => Self::DivisionDestroyed,
            Cause::DivisionOverrun => Self::DivisionOverrun,
            Cause::DivisionDeleted => Self::DivisionDeleted,
            Cause::DivisionDisbanded => Self::DivisionDisbanded,
            Cause::DivisionCapitulation => Self::DivisionCapitulation,
            Cause::DivisionScripted => Self::DivisionScripted,
            Cause::DivisionTemplateRemoved => Self::DivisionTemplateRemoved,
            Cause::ParadropDisruption => Self::ParadropDisruption,
            Cause::DivisionMerged => Self::DivisionMerged,
            Cause::ExileEquipment => Self::ExileEquipment,
            Cause::VolunteerReturnEquipment => Self::VolunteerReturnEquipment,
            Cause::CapitulationEscapeEquipment => Self::CapitulationEscapeEquipment,
            Cause::ScriptedEquipmentFraction => Self::ScriptedEquipmentFraction,
            Cause::ScriptedStockpileFraction => Self::ScriptedStockpileFraction,
        }
    }
}

/// Coverage of the supplemental deletion, transfer and live-pool writer group.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[derive(replay_spool::Spooled)]
pub enum EquipmentDestructionCoverage {
    #[default]
    Complete = 0,
    SupplementalUnavailable = 1,
}

impl NativeValue for EquipmentDestructionCoverage {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(0) => Ok(Self::Complete),
            Field::U8(1) => Ok(Self::SupplementalUnavailable),
            _ => Err(invalid("unsupported equipment destruction coverage")),
        }
    }
}

/// Signed net equipment removal since the preceding consecutive hourly drain,
/// dated by this frame's drain read; negative amounts are native gains (corrections).
/// Historical positive amounts retain their identity and amount when the appended
/// coverage/reason columns are NotRecorded; that absence never certifies coverage.
/// Nonzero (country,cause,design) rows use Complete coverage and an empty
/// unavailable_reason. If the optional hook
/// group failed, one reserved key-zero marker has all identity/amount fields
/// zero, SupplementalUnavailable coverage and its nonempty reason.
/// Consumers must read the marker even when there are no amount rows: net
/// removals are incomplete because deletion, paradrop, merge, exile, volunteer-return,
/// capitulation-escape and scripted division/stockpile causes are unavailable.
/// `cause` is a stable value; engine_events::equipment_destruction owns the executable
/// caller map and evidence. An unrecognized caller records Unknown, counted in
/// collector status, without guessing a known cause or discarding its removals.
/// This table is not the Combat Log and does not cover unhooked convoy-pool or
/// lend-lease losses, scrapping, or equipment captured/transferred between pools.
/// A complete empty hour means zero net removal through the hooked writers, not
/// complete loss coverage for any equipment domain.
/// Variant identity is the same full handle as VariantDefinitions.id.
/// ScriptedStockpileFraction has no fixed domain and is not a land cause.
/// Resolve its variant's equipment_token through recorded LexicalTokens and
/// installed_content_core::equipment::Database from the recorded content stack, then classify
/// that equipment type's effective category with
/// engine_native::cequipment_type::{PLANE_MASK, SHIP_MASK}: air first, naval next,
/// otherwise land. Convoy (bit 0) is naval; train and railway gun are land.
/// Unresolved tokens/content leave the domain unknown.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct EquipmentDestructionRow {
    pub country: u32,
    pub cause: EquipmentDestructionCause,
    pub variant: u64,
    pub amount_raw: i64,
    pub coverage: Recorded<EquipmentDestructionCoverage>,
    pub unavailable_reason: Recorded<String>,
}
typed_codec!(EquipmentDestructionRow, 104,
|row| (row.country as u128) << 72 | (row.cause as u128) << 64 | row.variant as u128,
|key| Self { country: (key >> 72) as u32, cause: EquipmentDestructionCause::try_from((key >> 64) as u8)?, variant: key as u64, ..Default::default() },
{ 0 => (amount_raw): I64, 1 => (coverage): U8, 2 => (unavailable_reason): Bytes }, |row| {
    match (&row.coverage, &row.unavailable_reason) {
        (Recorded::NotRecorded, Recorded::NotRecorded) if row.amount_raw > 0 => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::Complete), Recorded::Value(reason)) if row.key() != 0 && row.amount_raw != 0 && reason.is_empty() => Ok(()),
        (Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable), Recorded::Value(reason)) if row.key() == 0 && row.amount_raw == 0 && !reason.is_empty() => Ok(()),
        _ => Err(invalid("invalid equipment destruction amount or availability marker")),
    }
});

/// Stable disposition of a gameplay deletion or temporary roster removal.
/// Its field is independent of the optional destruction cause.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[derive(replay_spool::Spooled)]
pub enum DivisionDeletionDisposition {
    #[default]
    Loss = 0,
    Returned = 1,
    Merged = 2,
    HqTransferred = 3,
    VolunteerTransferred = 4,
    ExileTransferred = 5,
}

impl DivisionDeletionDisposition {
    pub const fn is_temporary_transfer(self) -> bool {
        matches!(self, Self::VolunteerTransferred | Self::ExileTransferred)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loss => "loss",
            Self::Returned => "returned",
            Self::Merged => "merged",
            Self::HqTransferred => "hq_transferred",
            Self::VolunteerTransferred => "volunteer_transferred",
            Self::ExileTransferred => "exile_transferred",
        }
    }
}

impl NativeValue for DivisionDeletionDisposition {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(*self as u8)
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(0) => Ok(Self::Loss),
            Field::U8(1) => Ok(Self::Returned),
            Field::U8(2) => Ok(Self::Merged),
            Field::U8(3) => Ok(Self::HqTransferred),
            Field::U8(4) => Ok(Self::VolunteerTransferred),
            Field::U8(5) => Ok(Self::ExileTransferred),
            Field::U8(_) => Err(invalid("unsupported division deletion disposition")),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

impl NativeValue for Option<EquipmentDestructionCause> {
    fn heap_bytes(&self) -> usize {
        0
    }
    fn try_clone(&self) -> io::Result<Self> {
        Ok(*self)
    }

    fn field(&self) -> Field<'_> {
        Field::U8(self.map_or(u8::MAX, |cause| cause as u8))
    }

    fn from_field(field: Field<'_>) -> io::Result<Self> {
        match field {
            Field::U8(u8::MAX) => Ok(None),
            Field::U8(value) => EquipmentDestructionCause::try_from(value).map(Some),
            _ => Err(invalid("field type differs from schema")),
        }
    }
}

/// Every gameplay division deletion or volunteer/exile roster removal per drain,
/// including empty pools, returned equipment, merged sources and HQ transfers.
/// Signed net amounts are aggregated separately in EquipmentDestruction. Rejected
/// spawns, fake-intel units and load/reset/teardown cleanup have no witness.
/// Temporary transfer witnesses stay active until census reappearance or deletion.
/// A witness proves its event, not presence at an earlier census. A complete table
/// certifies that no censused division disappeared without an event witness.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct DivisionEquipmentDeletionRow {
    /// Full native handle, joining Divisions.id.
    pub division_handle: u64,
    pub country: u32,
    /// Raw expeditionary-owner reference captured before DeleteUnit clears it.
    pub expeditionary_owner: u32,
    pub disposition: DivisionDeletionDisposition,
    /// Present only for Loss. Temporary roster transfers have no loss cause.
    pub cause: Option<EquipmentDestructionCause>,
}

impl Default for DivisionEquipmentDeletionRow {
    fn default() -> Self {
        Self {
            division_handle: 0,
            country: 0,
            expeditionary_owner: 0,
            disposition: DivisionDeletionDisposition::Loss,
            cause: Some(EquipmentDestructionCause::Unknown),
        }
    }
}

impl DivisionEquipmentDeletionRow {
    pub const fn valid_disposition(&self) -> bool {
        match (self.disposition, self.cause) {
            (DivisionDeletionDisposition::Loss, Some(cause)) => cause.is_division_deletion(),
            (DivisionDeletionDisposition::Loss, None) => false,
            (_, None) => true,
            (_, Some(_)) => false,
        }
    }

    pub const fn event_name(&self) -> &'static str {
        match self.cause {
            Some(cause) => cause.as_str(),
            None => self.disposition.as_str(),
        }
    }
}
typed_codec!(DivisionEquipmentDeletionRow, 64,
|row| row.division_handle as u128,
|key| Self { division_handle: key as u64, ..Default::default() },
{ 0 => (country): U32, 1 => (expeditionary_owner): U32, 2 => (disposition): U8, 3 => (cause): U8 }, |row| {
    if row.division_handle >> 32 == 0 {
        Err(invalid("deleted division handle has a null id"))
    } else if !row.valid_disposition() {
        Err(invalid("invalid division deletion disposition"))
    } else {
        Ok(())
    }
});

#[cfg(test)]
#[path = "equipment_destruction/tests.rs"]
mod tests;
