//! Which counter kinds the map's layers draw, and the products the window's counter layout takes
//! from them: the one table the view ([`viewer_counters::map::counters::Preparation::set_layered`]),
//! the hour's tables (which leave those kinds out) and the window read.

use super::{
    Layer,
    air_mission_counters::{AirMissionCounters, Passes},
    base_counters::BaseCountersFed,
    gun_counters::GunCounters,
    land_markers::LandMarkers,
    naval_markers::NavalMarkers,
};
use crate::playhead_host::MapSide;
use replay_data::BranchId;
use std::sync::Arc;
use viewer_counters::map::counters::{HourTables, Kinds, Layered};
use viewer_foundation::map::perspective::{Choice, Vision};

/// Each counter kind and the layer that draws it.
pub const LAYERS: [(&str, Kinds); 6] = [
    ("counters", Kinds::LAND),
    ("land_markers", Kinds::LAND_MARKERS),
    ("naval_markers", Kinds::NAVAL_MARKERS),
    ("base_counters", Kinds::BASES),
    ("gun_counters", Kinds::GUNS),
    ("air_mission_counters", Kinds::AIR_MISSIONS),
];

/// The kinds whose layer draws (declared, composed, not refused).
pub fn draws(side: &MapSide) -> Kinds {
    LAYERS.iter().filter(|(name, _)| side.draws(name)).fold(Kinds::default(), |kinds, &(_, kind)| kinds.with(kind))
}

fn find<T: 'static>(layers: &[Box<dyn Layer>]) -> Option<&T> {
    layers.iter().find_map(|layer| layer.any().downcast_ref::<T>())
}

/// The product of each kind whose layer draws, as of its last settle; none for the rest, and
/// none at all while the map composes.
pub fn published(side: &MapSide) -> Layered {
    let Some((_, layers)) = side.parts() else { return Layered::default() };
    let drawn = |name: &str| side.draws(name);
    Layered {
        land: crate::layers::counters::published(side),
        land_markers: drawn("land_markers").then(|| find::<LandMarkers>(layers).and_then(LandMarkers::published).cloned()).flatten(),
        naval_markers: drawn("naval_markers").then(|| find::<NavalMarkers>(layers).and_then(NavalMarkers::published).cloned()).flatten(),
        bases: drawn("base_counters").then(|| find::<BaseCountersFed>(layers).and_then(BaseCountersFed::published).cloned()).flatten(),
        guns: drawn("gun_counters").then(|| find::<GunCounters>(layers).and_then(GunCounters::published).cloned()).flatten(),
        air: drawn("air_mission_counters").then(|| find::<AirMissionCounters>(layers).and_then(AirMissionCounters::published).cloned()).flatten(),
    }
}

/// Gives the counter layers in `layers` the vision the map shows at the step's hour, its type-icon
/// relations, its guns' destination hostility and the recording's daily air rosters with the branch
/// played; the rosters only for a branch not given yet, as each `passes()` is a new history. Run on
/// the side's thread inside the step, before those layers settle. Whether any vision was new.
pub fn give(layers: &[Box<dyn Layer>], choice: &Choice, vision: &Arc<Vision>, tables: &HourTables, branch: BranchId, passes: impl FnOnce() -> Passes) -> bool {
    let new = |held: Option<(Choice, Arc<Vision>)>| held.is_none_or(|(_, held)| !Arc::ptr_eq(&held, vision));
    let mut any = false;
    if let Some(layer) = find::<LandMarkers>(layers) {
        any |= new(layer.perspective());
        layer.set_vision(choice.clone(), Arc::clone(vision));
    }
    if let Some(layer) = find::<NavalMarkers>(layers) {
        any |= new(layer.perspective());
        layer.set_vision(choice.clone(), Arc::clone(vision));
    }
    if let Some(layer) = find::<BaseCountersFed>(layers) {
        any |= new(layer.perspective());
        layer.set_vision(choice.clone(), Arc::clone(vision));
    }
    if let Some(layer) = find::<GunCounters>(layers) {
        any |= new(layer.perspective());
        layer.set_vision(choice.clone(), Arc::clone(vision));
        layer.set_relations(Arc::clone(tables.type_tints()));
        layer.set_hostility(Arc::new(tables.gun_hostility()));
    }
    if let Some(layer) = find::<AirMissionCounters>(layers) {
        any |= layer.perspective().is_none_or(|(_, held)| held.ours() != vision.ours());
        layer.set_vision(choice.clone(), Arc::clone(vision));
        if layer.passes_branch() != Some(branch) {
            layer.set_passes(passes(), branch);
        }
    }
    any
}
