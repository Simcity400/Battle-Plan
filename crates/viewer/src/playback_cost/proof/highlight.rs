//! The pointer's faction highlight over the political layer's picture, against the one over today's
//! political part of the same hour: for 20 pointed provinces spread over the strategic regions
//! (each pointing its region) and the plain plans a cleared highlight restores, the border class
//! plan, the demilitarised strips and the hover strips; and the frame's copy of the picture, timed.

use super::{Compared, political, political_fresh, political_marked};
use crate::{
    app::faction_highlight::Picture,
    document::Document,
    layers::{Layer, political::Political},
    playhead_host::mirror::Mirrors,
};
use map_display_data::landscape::Regions;
use serde_json::json;
use std::{collections::BTreeSet, io, sync::Arc, time::Instant};
use viewer_foundation::map::perspective;
use viewer_world::map::{
    border_pass::ClassPlan,
    demilitarized::{Zone, ZonePlan},
    map_mode::MapMode,
};

/// The cycle counter's rate, timed once against the wall clock.
fn cycle_hz() -> f64 {
    static HZ: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *HZ.get_or_init(|| {
        let (wall, cycles) = (Instant::now(), replay_frame::cpu::stamp());
        std::thread::sleep(std::time::Duration::from_millis(100));
        (replay_frame::cpu::stamp() - cycles) as f64 / wall.elapsed().as_secs_f64()
    })
}

/// Pointed provinces a sample compares.
const POINTED: usize = 20;

pub(super) fn make(document: &Document) -> Box<dyn Layer> {
    Box::new(Political::new(Arc::new(crate::layers::political::Content::new(Arc::clone(&document.content.map), &document.border_meshes))))
}

/// The three plans a highlight of `provinces` installs, as the scene's highlight makes them.
fn plans(meshes: &viewer_world::map::borders::BorderMeshes, regions: &Regions, zone: &Zone, provinces: &BTreeSet<u32>) -> (ClassPlan, ZonePlan, ZonePlan) {
    let highlight = |a, b| provinces.contains(&a) != provinces.contains(&b);
    (
        ClassPlan::matching(meshes, regions, |a, b| !zone.splits(a, b) && !highlight(a, b)),
        ZonePlan::matching(meshes, regions, |a, b| zone.splits(a, b) && !highlight(a, b)),
        ZonePlan::custom_highlight(meshes, regions, highlight),
    )
}

pub(super) fn today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer: &Political = political(layer);
    let mut compared = Compared::default();
    let note = |compared: &mut Compared, what: String| {
        compared.differing += 1;
        compared.first.get_or_insert(what);
    };
    let product = match document.hour()?.political_product(&MapMode::default(), &perspective::Choice::default(), None, None) {
        Ok(product) => product,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            return Ok(compared);
        }
    };
    let (copied, cycles) = (Instant::now(), replay_frame::cpu::thread_cycles());
    let picture = Picture::of(layer);
    let copy_cycles = replay_frame::cpu::thread_cycles().saturating_sub(cycles);
    let copy_ns = copied.elapsed().as_nanos() as u64;
    let (regions, zone) = picture.regions_and_zone(&document.content.map);
    let meshes = &document.border_meshes;
    let ids: Vec<u32> = document.content.strategic_regions.keys().copied().collect();
    let mut selections: Vec<(Option<u32>, BTreeSet<u32>)> = vec![(None, BTreeSet::new())];
    for at in 0..POINTED.min(ids.len()) {
        let region = &document.content.strategic_regions[&ids[at * ids.len() / POINTED]];
        selections.push((region.provinces.first().copied(), region.provinces.iter().copied().collect()));
    }
    for (pointed, provinces) in &selections {
        compared.entries += 3;
        let (ours, theirs) = (plans(meshes, &regions, &zone, provinces), plans(meshes, &product.layers.regions, &product.zone, provinces));
        for (name, same) in [("border classes", ours.0 == theirs.0), ("demilitarised strips", ours.1 == theirs.1), ("hover strips", ours.2 == theirs.2)] {
            if !same {
                let what = pointed.map_or("the restored plain plans".to_owned(), |province| format!("province {province} pointed"));
                note(&mut compared, format!("{what}: the {name} differ"));
            }
        }
    }
    compared.notes = json!({"pointed": selections.len() - 1, "copy_ns": copy_ns, "copy_cpu_ns": copy_cycles as f64 / cycle_hz() * 1e9, "refused": layer.refused()});
    Ok(compared)
}

pub(super) fn fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &crate::layers::Upstream<'_>) -> io::Result<Compared> {
    political_fresh(layer, mirrors, upstream)
}

pub(super) fn marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    political_marked(layer)
}
