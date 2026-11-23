
use super::Compared;
use crate::{document::Document, playhead_host::mirror::Mirrors};
use viewer_layer_core::layers::Layer;
use viewer_layers_political::{labels::Labels, names::Names};
use viewer_layers_infrastructure::{rails::Rails, trains::Trains};
use serde_json::{Value, json};
use std::{io, sync::Arc};

fn note(compared: &mut Compared, what: String) {
    compared.differing += 1;
    compared.first.get_or_insert(what);
}

pub(super) fn rails_make(document: &Document) -> Box<dyn Layer> {
    let geography: Arc<dyn viewer_layers_infrastructure::rails::Geography> = Arc::clone(&document.content) as _;
    Box::new(Rails::new(geography))
}

fn rails(layer: &dyn Layer) -> &Rails {
    layer.any().downcast_ref::<Rails>().expect("the rails line's layer")
}

pub(super) fn rails_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [edges, supply, ports] = rails(layer).marked();
    vec![("edges", edges as u64), ("supply provinces", supply as u64), ("harbours", ports as u64)]
}

pub(super) fn rails_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = rails(layer);
    if !layer.shown() {
        return Ok(Compared { notes: json!({"hidden": "the camera is at or above the railway cutoff"}), ..Compared::default() });
    }
    let product = document.hour()?.rail_product()?;
    let (mesh, today) = (&product.0, product.2.display());
    let mut compared = Compared::default();
    let ours = layer.network().display();
    compared.entries += (today.graph.values().map(|neighbours| neighbours.len()).sum::<usize>() + today.sources.len() + today.ports.len()) as u64;
    if ours.graph != today.graph {
        let first = today.graph.iter().zip(ours.graph.iter()).find(|(a, b)| a != b).map(|(a, b)| format!("today {a:?}, the layer {b:?}"));
        note(&mut compared, format!("rail graph: today {} provinces, the layer {}; first {first:?}", today.graph.len(), ours.graph.len()));
    }
    if ours.sources != today.sources {
        note(&mut compared, format!("supply sources: today {}, the layer {}", today.sources.len(), ours.sources.len()));
    }
    if ours.ports != today.ports {
        note(&mut compared, format!("harbours: today {}, the layer {}", today.ports.len(), ours.ports.len()));
    }
    let mut vertices = Vec::with_capacity(mesh.native_vertices.len());
    let mut runs = Vec::new();
    let (mut segments, mut unanchored, mut samples) = (0, 0, 0);
    for run in layer.runs().values() {
        segments += run.segments;
        unanchored += run.unanchored;
        samples += run.samples;
        if !run.vertices.is_empty() {
            let first = vertices.len() as u32;
            vertices.extend_from_slice(&run.vertices);
            runs.push(first..vertices.len() as u32);
        }
    }
    compared.entries += (mesh.native_vertices.len() + mesh.native_runs.len() + 3) as u64;
    if (segments, unanchored, samples) != (mesh.segments, mesh.unanchored, mesh.samples) {
        note(&mut compared, format!("counts: today {:?}, the layer {:?}", (mesh.segments, mesh.unanchored, mesh.samples), (segments, unanchored, samples)));
    }
    if runs != mesh.native_runs {
        note(&mut compared, format!("strip runs: today {}, the layer {}", mesh.native_runs.len(), runs.len()));
    }
    let bits = |values: &[[f32; 6]]| values.as_flattened().iter().map(|value| value.to_bits()).collect::<Vec<_>>();
    let (today_bits, our_bits) = (bits(&mesh.native_vertices), bits(&vertices));
    if today_bits != our_bits {
        let at = today_bits.iter().zip(&our_bits).position(|(a, b)| a != b).unwrap_or(today_bits.len().min(our_bits.len()));
        note(&mut compared, format!("strip vertices: today {}, the layer {}; first difference at float {at}", mesh.native_vertices.len(), vertices.len()));
    }
    let bridges: Vec<_> = layer.bridges().values().collect();
    compared.entries += mesh.bridges.len() as u64;
    let same = bridges.len() == mesh.bridges.len()
        && bridges.iter().zip(&mesh.bridges).all(|(a, b)| a.position.map(f32::to_bits) == b.position.map(f32::to_bits) && a.rotation.to_bits() == b.rotation.to_bits() && a.kind == b.kind);
    if !same {
        note(&mut compared, format!("bridges: today {}, the layer {}", mesh.bridges.len(), bridges.len()));
    }
    compared.notes = json!({"provinces": today.graph.len(), "vertices": mesh.native_vertices.len(), "bridges": mesh.bridges.len(), "sources": today.sources.len(), "ports": today.ports.len()});
    Ok(compared)
}

pub(super) fn rails_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = rails(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: (layer.runs().len() + layer.bridges().len() + 1) as u64, differing, first, notes: Value::Null })
}

pub(super) fn names_make(document: &Document) -> Box<dyn Layer> {
    let atlas: Arc<dyn viewer_layers_political::names::Atlas> = Arc::clone(&document.content) as _;
    Box::new(Names::new(atlas))
}

fn names(layer: &dyn Layer) -> &Names {
    layer.any().downcast_ref::<Names>().expect("the names line's layer")
}

pub(super) fn names_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let layer = names(layer);
    let [provinces, named, placed] = layer.marked();
    let latency = layer.latency();
    let mut marked = vec![("provinces", provinces as u64), ("countries named", named as u64), ("areas placed", placed as u64)];
    let buckets = ["names on screen in 1 frame", "in 2 frames", "in 3-4 frames", "in 5-8 frames", "in 9-16 frames", "in 17-32 frames", "in 33+ frames"];
    marked.extend(buckets.into_iter().zip(latency.buckets));
    marked.extend([("latency worst frames", latency.worst_frames), ("worker areas", latency.worker_areas), ("worker us", latency.worker_us), ("worker worst us", latency.worker_worst_us)]);
    marked
}

pub(super) fn names_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = names(layer);
    let mut compared = Compared::default();
    let today = match document.hour()?.shared_names(&viewer_map_modes::map_mode::MapMode::default()) {
        Ok(today) => today,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            return Ok(compared);
        }
    };
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
        return Ok(compared);
    }
    let ours = layer.placements();
    compared.entries += (today.country.len().max(ours.country.len()) + 1) as u64;
    if today.country.len() != ours.country.len() {
        note(&mut compared, format!("names: today {}, the layer {}", today.country.len(), ours.country.len()));
    }
    for (at, (a, b)) in today.country.iter().zip(&ours.country).enumerate() {
        if a != b {
            note(&mut compared, format!("name {at}: today {} {:?} ({} glyphs), the layer {} {:?} ({} glyphs)", a.key, a.text, a.uvs.len(), b.key, b.text, b.uvs.len()));
        }
    }
    if today.warnings != ours.warnings {
        note(&mut compared, format!("warnings: today {:?}, the layer {:?}", today.warnings, ours.warnings));
    }
    if today.near.is_some() {
        note(&mut compared, "today places near names at the default mode".into());
    }
    compared.notes = json!({"names": today.country.len(), "areas": layer.areas().len(), "warnings": today.warnings.len()});
    Ok(compared)
}

pub(super) fn names_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = names(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: (layer.areas().len() + 1) as u64, differing, first, notes: Value::Null })
}

fn viewing(document: &Document) -> Option<u32> {
    document.hour().ok()?.vision(&viewer_foundation::perspective::Choice::default()).ok().map(|vision| vision.ours())
}

pub(super) fn labels_make(document: &Document) -> Box<dyn Layer> {
    Box::new(Labels::new(Arc::clone(&document.content.names.localisation), viewing(document)))
}

fn labels(layer: &dyn Layer) -> &Labels {
    layer.any().downcast_ref::<Labels>().expect("the labels line's layer")
}

pub(super) fn labels_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [provinces, states, wars] = labels(layer).marked();
    vec![("provinces", provinces as u64), ("states renamed", states as u64), ("war recounts", wars as u64)]
}

pub(super) fn labels_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = labels(layer);
    let mut compared = Compared::default();
    if !layer.shown() {
        compared.notes = json!({"hidden": "the view's overlay draws no labels"});
        return Ok(compared);
    }
    let Some(ours) = layer.ours() else {
        compared.notes = json!({"no_viewing_slot": true});
        return Ok(compared);
    };
    if viewing(document) != Some(ours) {
        compared.notes = json!({"viewing_moved": [ours, viewing(document)]});
        return Ok(compared);
    }
    let today = match document.hour()?.labels(ours) {
        Ok(today) => today,
        Err(error) => {
            compared.entries = 1;
            if layer.refused().is_none() {
                note(&mut compared, format!("today refuses the hour ({error}), the layer draws it"));
            }
            return Ok(compared);
        }
    };
    if let Some(why) = layer.refused() {
        compared.entries = 1;
        note(&mut compared, format!("the layer refuses the hour ({why}), today draws it"));
        return Ok(compared);
    }
    let ours = layer.display();
    compared.entries += (today.candidates.len().max(ours.candidates.len()) + 2) as u64;
    if today.perspective != ours.perspective {
        note(&mut compared, format!("perspective: today {:?}, the layer {:?}", today.perspective, ours.perspective));
    }
    if today.allegiances != ours.allegiances {
        note(&mut compared, format!("sides: today {:?}, the layer {:?}", today.allegiances, ours.allegiances));
    }
    if today.candidates.len() != ours.candidates.len() {
        note(&mut compared, format!("candidates: today {}, the layer {}", today.candidates.len(), ours.candidates.len()));
    }
    for (a, b) in today.candidates.iter().zip(ours.candidates.iter()) {
        if a != b {
            note(&mut compared, format!("candidate: today {a:?}, the layer {b:?}"));
        }
    }
    compared.notes = json!({"candidates": today.candidates.len(), "sides": today.allegiances.len()});
    Ok(compared)
}

pub(super) fn labels_fresh(layer: &dyn Layer, mirrors: &Mirrors, _upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = labels(layer);
    let mut fresh = layer.fresh();
    fresh.compose(mirrors)?;
    let (differing, first) = layer.differ(&fresh);
    Ok(Compared { entries: (layer.candidates() + 2) as u64, differing, first, notes: Value::Null })
}

pub(super) fn trains_make(document: &Document) -> Box<dyn Layer> {
    let yard: Arc<dyn viewer_layers_infrastructure::trains::Yard> = Arc::clone(&document.content) as _;
    Box::new(Trains::new(yard, viewing(document)))
}

fn trains(layer: &dyn Layer) -> &Trains {
    layer.any().downcast_ref::<Trains>().expect("the trains line's layer")
}

pub(super) fn trains_marked(layer: &dyn Layer) -> Vec<(&'static str, u64)> {
    let [built, quiet, curves, weights] = trains(layer).marked();
    let [census, plan, nodes, curving, carry, restated] = trains(layer).timed();
    let why = trains(layer).why();
    vec![
        ("plans built", built as u64),
        ("quiet steps", quiet as u64),
        ("curves made", curves as u64),
        ("weights alone", weights as u64),
        ("census us", census),
        ("plan us", plan),
        ("nodes us", nodes),
        ("curves us", curving),
        ("carry us", carry),
        ("structural: a status restated", restated),
        ("structural: SupplyNodes", why[0]),
        ("structural: Provinces", why[1]),
        ("structural: RailCooldowns", why[2]),
        ("structural: RailEdges", why[3]),
        ("structural: TrainInventory", why[4]),
        ("structural: Countries", why[5]),
        ("structural: CountryAppearance", why[6]),
        ("structural: VariantDefinitions", why[7]),
        ("structural: rails redrawn", why[8]),
    ]
}

pub(super) fn trains_today(layer: &dyn Layer, document: &Document) -> io::Result<Compared> {
    let layer = trains(layer);
    let mut compared = Compared::default();
    if !layer.shown() {
        compared.notes = json!({"hidden": "the camera is above the railway cutoff or trains are off"});
        return Ok(compared);
    }
    let Some(ours) = layer.ours() else {
        compared.notes = json!({"no_viewing_slot": true});
        return Ok(compared);
    };
    if viewing(document) != Some(ours) {
        compared.notes = json!({"viewing_moved": [ours, viewing(document)]});
        return Ok(compared);
    }
    compared.entries = 1;
    let today = document.hour()?.train_plan(&viewer_foundation::perspective::Choice::default());
    match (today, layer.refused()) {
        (Err(error), None) => note(&mut compared, format!("today refuses the hour ({error}), the layer draws it")),
        (Ok(_), Some(why)) => note(&mut compared, format!("the layer refuses the hour ({why}), today draws it")),
        (Err(_), Some(_)) => {}
        (Ok(today), None) => {
            let [routes, nodes, variants] = today.counts();
            compared.entries += (routes + nodes + variants) as u64;
            let differs = layer.plan().differ(&today);
            if let Some(first) = &differs {
                note(&mut compared, first.clone());
            }
            compared.notes = json!({"routes": routes, "weighted_nodes": nodes, "variants": variants});
            if differs.is_some() {
                let ours: std::collections::BTreeSet<&[u32]> = layer.plan().route_paths().collect();
                let theirs: std::collections::BTreeSet<&[u32]> = today.route_paths().collect();
                compared.notes["routes_today_alone"] = json!(theirs.difference(&ours).take(8).collect::<Vec<_>>());
                compared.notes["routes_layer_alone"] = json!(ours.difference(&theirs).take(8).collect::<Vec<_>>());
                compared.notes["published"] = json!(format!("{:?}", layer.published()));
                compared.notes["owed"] = json!(layer.owed());
                compared.notes["made"] = json!(layer.made());
            }
        }
    }
    Ok(compared)
}

pub(super) fn trains_fresh(layer: &dyn Layer, mirrors: &Mirrors, upstream: &viewer_layer_core::layers::Upstream<'_>) -> io::Result<Compared> {
    let layer = trains(layer);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, upstream)?;
    let (differing, first) = layer.differ(&fresh);
    let mut notes = Value::Null;
    if differing > 0 {
        let ours: std::collections::BTreeMap<u128, viewer_layers_infrastructure::trains::ShownNode> = layer.shown_nodes().iter().map(|node| (node.key, *node)).collect();
        let theirs: std::collections::BTreeMap<u128, i64> = fresh.shown_nodes().iter().map(|node| (node.key, node.weight)).collect();
        let keys: std::collections::BTreeSet<u128> = ours.keys().chain(theirs.keys()).copied().collect();
        let nodes: Vec<Value> = keys
            .into_iter()
            .filter(|key| ours.get(key).map(|node| node.weight) != theirs.get(key).copied())
            .take(8)
            .map(|key| {
                let node = ours.get(&key);
                json!({"native": (key >> 32) as u32, "index": key as u32, "layer_weight": node.map(|node| node.weight), "composed_weight": theirs.get(&key), "layer_copied_at": node.and_then(|node| node.copied), "worker_available": node.and_then(|node| node.available), "worker_consumed": node.and_then(|node| node.consumed), "mirrors": layer.node_cells(mirrors, key)})
            })
            .collect();
        notes = json!({"nodes_differing": nodes, "weighted_layer": ours.len(), "weighted_composed": theirs.len(), "made": layer.made()});
    }
    Ok(Compared { entries: 1, differing, first, notes })
}
