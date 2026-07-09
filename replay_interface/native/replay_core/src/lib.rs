//! Replay ingest and scrubbable world timeline.
//!
//! The only place recording bytes enter the replay layer. Everything
//! downstream (replay_app, tools) consumes [`WorldTimeline`] /
//! [`WorldFrame`] and never touches container or schema internals.
//! See `replay_interface/documents/REPLAY_ARCHITECTURE.md`.

pub mod source;
pub mod world;

pub use source::{open_recording, open_recording_game, RawFrame, ReplaySource, SourceError};
pub use world::{
    AirSuperiorityRow, BuildingStatus, ChannelCoverage, CombatRow, ConvoyLossRow, CountryCore,
    CountryDiplomacy, CountryIdentity, DesignsView, DiplomacyView, DiplomaticPairState,
    DivisionRow, DivisionTemplateRow, DivisionUnitRow, EquipmentAmount, EquipmentStock,
    EquipmentVariantRow, FleetUnitRow, FocusView, IntelView, Leaf, LeaderRosterView, LeaderRow,
    MapLayerSource, NavalAirCombatView, NavalCombatRow, NavalSupremacyRow, OccupationRow,
    OperationRow, OperativeRow, PlayerCountry, PoliticsDetailView, PovVisibilityView,
    PowerBalanceRow, PowerBalanceView, ProductionLineRow, ProductionView, ProvenanceView,
    ProvinceVisibility, RaidStateView, ResearchProgressRow, ResearchView, ScriptedGuiView,
    SessionMetadataView, ShipCompositionRow, SnowMudCell, SnowMudView, SpeedEvent,
    SpottedUnitsView, StateBuildings, TimelineConfig, TimelineLookup, TopbarAlerts, TopbarRatios,
    TopbarScalars, TradeContractRow, TradeView, ViewStatus, WarRelationRow, WeatherRegionRow,
    WingUnitRow, WorldError, WorldFrame, WorldTimeline,
};
