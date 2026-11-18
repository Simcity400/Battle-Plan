//! The demilitarised zone moving reclassifies the edges of the states that entered or left it and
//! patches the class plan's chunks, instead of reclassifying every edge and building every plan.

use super::{Content, LEFT_OUT, Political};
use installed_content_core::map::{MapData, ProvinceDefinition, ProvinceKind};
use std::{collections::HashSet, sync::Arc};
use viewer_world::map::{
    borders::{BorderClass, BorderDraw, BorderMesh, BorderMeshes, Vertex},
    demilitarized::{Zone, ZonePlan},
    political::Regions,
};

const PROVINCES: u32 = 40;

/// Land provinces 1 to `PROVINCES` in a row, each boundary one draw of the province mesh, ten
/// boundaries a vertex chunk.
fn content() -> Arc<Content> {
    let (map, meshes) = world();
    Arc::new(Content::new(Arc::new(map), &meshes))
}

fn world() -> (MapData, BorderMeshes) {
    let definition = |id| (id, ProvinceDefinition { id, rgb: [0; 3], kind: ProvinceKind::Land, continent: 0 });
    let map = MapData { width: PROVINCES, height: 1, province_ids: (1..=PROVINCES).collect(), definitions: (1..=PROVINCES).map(definition).collect(), country_colors: Default::default() };
    let vertex = Vertex { position: [0.0; 3], uv: [0; 2] };
    let draws: Vec<BorderDraw> = (1..PROVINCES).map(|a| BorderDraw { province_a: a, province_b: a + 1, chunk: (a - 1) / 10, base_vertex: (a - 1) % 10 * 4, vertex_count: 4, index_count: 6 }).collect();
    let chunks: Vec<Vec<Vertex>> = (0..PROVINCES.div_ceil(10)).map(|_| vec![vertex; 40]).collect();
    let mesh = |class| BorderMesh { class, chunks: if class == BorderClass::Province { chunks.clone() } else { Vec::new() }, draws: if class == BorderClass::Province { draws.clone() } else { Vec::new() } };
    let meshes = BorderMeshes { meshes: BorderClass::ALL.iter().map(|&class| mesh(class)).collect(), hidden: BorderMesh { class: BorderClass::Province, chunks: Vec::new(), draws: Vec::new() }, indices: vec![0, 2, 1, 2, 3, 1], one_pixel_provinces: vec![] };
    (map, meshes)
}

/// State 20 joins the zone: the four boundaries of its provinces are marked and classified, the
/// two it now splits leave the land classes through their chunks, and nothing is made whole.
/// Before, a zone change marked every boundary and set the whole plan to be built again.
#[test]
fn a_zone_change_reclassifies_the_moved_states_edges_and_patches_their_chunks() {
    let mut layer = Political::new(content());
    let edges = layer.content.edges.len();
    assert_eq!(edges, PROVINCES as usize - 1);
    layer.state_of = (0..=PROVINCES).map(|province| if (21..=23).contains(&province) { 20 } else { 10 }).collect();
    layer.owner_of = vec![1; PROVINCES as usize + 1];
    layer.class = vec![LEFT_OUT; edges];
    (0..edges as u32).for_each(|edge| layer.classify(edge));
    (layer.edges_changed, layer.zone_plan_stale) = (false, false);
    layer.pending_chunks.clear();
    assert!(layer.class.iter().all(|&class| class != LEFT_OUT), "no zone, no boundary left out");

    layer.rezone(HashSet::from([20]));
    assert_eq!(layer.edges.len(), 4, "the boundaries of provinces 21 to 23 alone, of {edges}");
    assert!(layer.zone_plan_stale && !layer.edges_changed);
    let mut marked = std::mem::take(&mut layer.edges);
    let _ = marked.settle(&mut crate::layers::Budget::unlimited(), |edge| {
        layer.classify(edge);
        Ok(())
    });
    let left: Vec<usize> = (0..edges).filter(|&edge| layer.class[edge] == LEFT_OUT).collect();
    assert_eq!(left, [19, 22], "the boundaries 20-21 and 23-24 are split by the zone");
    assert!(!layer.edges_changed, "split boundaries in the land chunks patch the plan, not rebuild it");
    assert_eq!(layer.pending_chunks.take(), [1, 2], "the chunks holding them");

    // The zone's plan from the zone's own borders equals the one made from every draw.
    let (map, meshes) = world();
    let field = |values: &[u32]| map_display_data::province_field::ProvinceField(Arc::new(map_display_data::province_field::Storage::Dense(Arc::new(values.to_vec()))));
    let regions = Regions { country_of: field(&layer.owner_of), owner_of: field(&layer.owner_of), state_of: field(&layer.state_of), country_colors: Default::default(), warnings: Vec::new(), strategic_attributes: Default::default(), mode: None };
    let whole = ZonePlan::new(&meshes, &regions, &Zone::new(&map, &regions, &std::collections::BTreeSet::from([20])));
    assert_eq!(layer.zone_plan_of_its_borders(&meshes), whole);
    assert_ne!(whole, ZonePlan::default(), "the two split boundaries are drawn by the zone");
}
