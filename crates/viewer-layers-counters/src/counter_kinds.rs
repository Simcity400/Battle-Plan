
use viewer_layer_core::layers::Layer;
use super::{air_mission_counters::{AirMissionCounters, Passes}, base_counters::BaseCountersFed, gun_counters::GunCounters, land_markers::LandMarkers, naval_markers::NavalMarkers};
use viewer_layer_core::playhead_host::{COUNTER_LAYERS, LayerParts};
use replay_data::BranchId;
use std::sync::Arc;
use viewer_counter_kit::Kinds;
use viewer_counters::counters::{HourTables, layered::Layered};
use viewer_foundation::perspective::{Choice, Vision};

pub fn draws(side: LayerParts<'_>) -> Kinds {
    COUNTER_LAYERS.iter().filter(|(name, _)| side.draws(name)).fold(Kinds::default(), |kinds, &(_, kind)| kinds.with(kind))
}

fn find<T: 'static>(layers: &[Box<dyn Layer>]) -> Option<&T> {
    layers.iter().find_map(|layer| layer.any().downcast_ref::<T>())
}

pub fn published(side: LayerParts<'_>) -> Layered {
    let Some((_, layers)) = side.parts() else { return Layered::default() };
    let drawn = |name: &str| side.draws(name);
    let viewer = side.viewer();
    let shows = |held: Option<(Choice, Arc<Vision>)>| viewer.is_some_and(|viewer| held.is_some_and(|(_, vision)| vision.ours() == viewer));
    Layered {
        land: crate::counters::published(side).filter(|_| find::<super::counters::fed::Fed>(layers).is_some_and(|layer| shows(layer.perspective()))),
        land_markers: drawn("land_markers").then(|| find::<LandMarkers>(layers).filter(|layer| shows(layer.perspective())).and_then(LandMarkers::published).cloned()).flatten(),
        naval_markers: drawn("naval_markers").then(|| find::<NavalMarkers>(layers).filter(|layer| shows(layer.perspective())).and_then(NavalMarkers::published).cloned()).flatten(),
        bases: drawn("base_counters").then(|| find::<BaseCountersFed>(layers).filter(|layer| shows(layer.perspective())).and_then(BaseCountersFed::published).cloned()).flatten(),
        guns: drawn("gun_counters").then(|| find::<GunCounters>(layers).filter(|layer| shows(layer.perspective())).and_then(GunCounters::published).cloned()).flatten(),
        air: drawn("air_mission_counters").then(|| find::<AirMissionCounters>(layers).filter(|layer| shows(layer.perspective())).and_then(AirMissionCounters::published).cloned()).flatten(),
    }
}

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
