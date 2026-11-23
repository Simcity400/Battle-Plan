use super::*;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    ScriptedEquipmentFraction = 25,
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

impl From<game_model::equipment_destruction::Cause> for EquipmentDestructionCause {
    fn from(cause: game_model::equipment_destruction::Cause) -> Self {
        use game_model::equipment_destruction::Cause;
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
