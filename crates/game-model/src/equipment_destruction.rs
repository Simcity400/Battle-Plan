/// Semantic native classification; replay owns the stable encoded values.
/// An unrecognized caller still has an exact removal and is recorded as
/// Unknown, rather than discarded or attributed to a guessed cause.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Cause {
    #[default]
    Unknown,
    LandCombat,
    NavalTransport,
    Immediate,
    Attrition,
    Garrison,
    AirNuclear,
    AirAccident,
    AirStrategic,
    AirMissionAa,
    AirLandCombatCas,
    AirRaid,
    AirNavalAa,
    DivisionDestroyed,
    DivisionOverrun,
    DivisionDeleted,
    DivisionDisbanded,
    DivisionCapitulation,
    DivisionScripted,
    DivisionTemplateRemoved,
    ParadropDisruption,
    DivisionMerged,
    ExileEquipment,
    VolunteerReturnEquipment,
    CapitulationEscapeEquipment,
    ScriptedEquipmentFraction,
    ScriptedStockpileFraction,
}
