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
    BuildingStatus, ChannelCoverage, CombatRow, CountryCore, CountryDiplomacy, CountryIdentity,
    DiplomacyView, DiplomaticPairState, DivisionRow, DivisionUnitRow, EquipmentAmount,
    FleetUnitRow, Leaf, MapLayerSource, NavalSupremacyRow, PlayerCountry, PovVisibilityView,
    ProvenanceView, ProvinceVisibility, SessionMetadataView, ShipCompositionRow, SpeedEvent,
    SpottedUnitsView, StateBuildings, TimelineLookup, TopbarScalars, ViewStatus, WeatherRegionRow,
    WingUnitRow, WorldError, WorldFrame, WorldTimeline,
};
