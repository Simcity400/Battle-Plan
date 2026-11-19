//! The demilitarised zone moving reclassifies the edges of the states that entered or left it and
//! patches the class plan's chunks, instead of reclassifying every edge and building every plan.

use super::{Content, Installer, LEFT_OUT, Ordering, Political, Row, Set, Whole};
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
    Arc::new(Content::new(Arc::new(map), Arc::new(meshes)))
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
    let _ = marked.settle(|edge| {
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

/// The hand-over between the side's thread and the display frame: what the side sends as it dates
/// a step's changes, and the frame's own installer, which writes the hours its head has reached
/// without taking the side at all.
fn handover() -> (Arc<super::Handover>, Installer) {
    let taken = Arc::new(super::Handover::default());
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    (Arc::clone(&taken), Installer::new(taken, Arc::default(), Arc::default(), live, Arc::new(std::sync::atomic::AtomicU64::new(1))))
}

/// One hour's change set: province `province` took fill key `key` at `hour`.
fn set(hour: i32, composed: u64, province: u32, key: u32) -> Set {
    Set { hour, from: hour, composed, whole: None, provinces: vec![(province, key)], keys: Vec::new(), naval: Vec::new(), plans: None, built_ns: 0, digest: None }
}

/// The whole fill at `hour`, as the side hands it over after a compose or a refused write.
fn whole(hour: i32, composed: u64, keys_of: Vec<u32>) -> Set {
    let field = |values: &[u32]| map_display_data::province_field::ProvinceField(Arc::new(map_display_data::province_field::Storage::Dense(Arc::new(values.to_vec()))));
    let regions = Regions { country_of: field(&keys_of), owner_of: field(&keys_of), state_of: field(&keys_of), country_colors: Default::default(), warnings: Vec::new(), strategic_attributes: Default::default(), mode: None };
    let whole = Whole { regions: Arc::new(regions), keys_of, colours: Vec::new(), naval: std::collections::BTreeMap::new() };
    Set { hour, from: hour, composed, whole: Some(whole), provinces: Vec::new(), keys: Vec::new(), naval: Vec::new(), plans: None, built_ns: 0, digest: None }
}

/// A display frame writes the change sets of the hours its own head has reached and leaves the
/// rest to the frames that reach them, so the hours advance with the frames and not with the
/// publications. The sets are handed over to the frame, so a frame that cannot take the side
/// still writes the ones it reached.
#[test]
fn a_frame_writes_the_hours_its_head_has_reached_and_leaves_the_rest() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(25));
    assert_eq!(picked.hour, Some(22), "the newest hour this head has reached");
    assert_eq!(picked.provinces, [10, 22], "and nothing of hour 34");
    assert_eq!(installer.pick(Some(25)).hour, None, "the same head writes nothing twice");
    assert_eq!(installer.pick(Some(40)).provinces, [34]);
    assert_eq!(installer.pick(Some(60)).provinces, [46]);
    assert!(installer.pick(Some(72)).hour.is_none(), "nothing is left");
}

/// A pause in the middle of a publication's queue: the picture a watcher is left looking at is
/// whole, so the frame writes every set handed over, whatever its head.
#[test]
fn a_pause_writes_every_hour_the_queue_holds() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(None);
    assert_eq!(picked.hour, Some(46));
    assert_eq!(picked.provinces, [10, 22, 34, 46]);
    assert!(!installer.owed, "nothing is left unwritten");
}

/// A seek in the middle of a publication's queue: its compose replaces the picture every set was
/// dated over, so nothing of those hours is written after it, and what the frame writes is the
/// whole fill the compose hands over.
#[test]
fn a_seek_writes_nothing_of_the_hours_it_left() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    installer.generation.store(2, Ordering::Release);
    let picked = installer.pick(Some(60));
    assert_eq!(picked.hour, None, "no hour of the picture the seek left");
    assert!(picked.provinces.is_empty());
    sets.push(whole(900, 2, vec![0, 7]));
    let picked = installer.pick(Some(900));
    assert_eq!(picked.hour, Some(900));
    assert!(picked.whole.is_some(), "the seek's own picture");
    assert_eq!(installer.keys_of, [0, 7], "and what is on the GPU is that picture's");
}

/// A speed change moves the head at another rate and seeks nothing: the hours already dated stand
/// and are written as the frames reach them, however fast that now is.
#[test]
fn a_speed_change_leaves_the_dated_hours_standing() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    assert_eq!(installer.pick(Some(12)).provinces, [10]);
    for head in [13, 14, 15] {
        assert!(installer.pick(Some(head)).hour.is_none(), "a slower head reaches no new hour");
    }
    assert!(installer.due(Some(15)).is_none(), "and none is owed at that head");
    assert_eq!(installer.pick(Some(46)).provinces, [22, 34, 46], "and the rest when it gets there");
}

/// A reversal: the head stands behind hours the side has already dated. Nothing of them is written
/// again and the picture does not go backwards; the seek that turned the clock around hands over
/// the whole fill of the hour it landed at, and that is what the next frame writes.
#[test]
fn a_head_that_went_backwards_writes_nothing_of_the_hours_it_has_left() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    assert_eq!(installer.pick(Some(46)).hour, Some(46));
    assert!(installer.pick(Some(20)).hour.is_none(), "nothing is written for a head that went back");
    assert!(installer.due(Some(20)).is_none(), "and nothing of those hours is owed it, so the picture stands where it was");
    installer.generation.store(2, Ordering::Release);
    sets.push(whole(20, 2, vec![0, 3]));
    let picked = installer.pick(Some(20));
    assert_eq!(picked.hour, Some(20));
    assert!(picked.whole.is_some());
}

/// The fill refused a write of what changed (its key table is full): the frame writes nothing more
/// until the whole fill the side is asked for arrives, since a write over a refused fill would
/// show neither hour. Until then the frame says it is owed one.
#[test]
fn a_fill_that_refused_a_changed_write_waits_for_the_whole_one() {
    let (sets, mut installer) = handover();
    installer.refused = true;
    sets.push(set(22, 1, 22, 22));
    let picked = installer.pick(Some(30));
    assert_eq!(picked.hour, None);
    assert!(picked.provinces.is_empty());
    sets.push(whole(34, 1, vec![0, 1]));
    let picked = installer.pick(Some(40));
    assert_eq!(picked.hour, Some(34), "the whole fill voids the set it refused");
    assert!(!installer.refused);
}

/// A whole fill dated ahead of the head (a refusal starting or ending as the side reads ahead) waits
/// for its hour, and the sets before it are written as the head reaches them. Before, it was
/// installed at once and the map showed an hour the head had not reached.
#[test]
fn a_whole_fill_ahead_of_the_head_waits_for_its_hour() {
    let (sets, mut installer) = handover();
    sets.push(set(102, 1, 2, 2));
    sets.push(set(104, 1, 4, 4));
    sets.push(whole(148, 1, vec![0, 9]));
    let picked = installer.pick(Some(103));
    assert!(picked.whole.is_none(), "the whole fill of hour 148 is not this head's");
    assert_eq!((picked.hour, picked.provinces), (Some(102), vec![2]));
    assert_eq!(installer.pick(Some(147)).provinces, [4]);
    let picked = installer.pick(Some(148));
    assert_eq!(picked.hour, Some(148));
    assert!(picked.whole.is_some());
    assert_eq!(installer.keys_of, [0, 9]);
}

/// A frame whose head lands inside hundreds of changed hours writes exactly the hours it reached:
/// the hand-over keeps every hour a publication dates apart. Before, the hand-over folded every set
/// past its 64th into the newest, so a head inside them missed the folded changes it had reached.
#[test]
fn a_head_inside_a_long_run_of_changes_writes_the_hours_it_reached() {
    let (sets, mut installer) = handover();
    for hour in 1..=1440 {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(700));
    assert_eq!(picked.hour, Some(700));
    assert_eq!(picked.provinces, (1..=700).collect::<Vec<usize>>());
}

/// Frames that stopped taking the sets leave the hand-over holding more than it keeps apart: the
/// oldest are folded into one another rather than dropped, dated at the newer hour so no change is
/// shown before its own, and every change is written once the head reaches them.
#[test]
fn a_hand_over_past_what_it_keeps_apart_folds_its_oldest_hours_instead_of_losing_them() {
    let (sets, mut installer) = handover();
    let hours = super::SETS_HELD as i32 + 40;
    for hour in 1..=hours {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(40));
    assert!(picked.hour.is_none(), "the folded hours 1 to 41 are not shown before hour 41");
    let picked = installer.pick(None);
    assert_eq!(picked.hour, Some(hours), "the newest hour dated");
    assert_eq!(picked.provinces.len(), hours as usize, "and every province of those hours");
    assert_eq!(installer.keys_of[hours as usize], hours as u32, "with the newest hour's key");
    // One contested province changing hands every hour: the folded set holds its newest write
    // alone. Before, it held one write an hour for as long as the frames took none.
    let (sets, _installer) = handover();
    for hour in 1..=hours {
        sets.push(set(hour, 1, 7, hour as u32));
    }
    let held = sets.0.lock().unwrap();
    assert_eq!(held.front().map(|set| set.provinces.clone()), Some(vec![(7, 41)]), "the folded hours 1 to 41, as hour 41 left province 7");
}

/// A publication whose provinces changed hands at different game hours leaves one change set an
/// hour, each holding the key that hour gave them, and a province that moved twice is in both with
/// the key of its own hour: what the display frames write as their heads reach those hours. Before,
/// everything a publication changed stood at the publication's own last hour.
#[test]
fn the_hours_of_a_publication_are_dated_one_change_set_an_hour() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0), (8, 1)].into_iter().collect();
    layer.natives = [(0, 70), (1, 80)].into_iter().collect();
    layer.resize();
    layer.provinces.clear();
    let row = |owner, controller| Row { state: Some(5), owner: Some(owner), controller: Some(controller) };
    for id in 1..=3 {
        layer.paint_from(id, Some(row(7, 7)));
    }
    assert_eq!(layer.country_of[1..=3], [1, 1, 1], "every province is country link 7's, whose slot is 0");
    layer.pending_provinces.clear();

    layer.hour_rows = vec![(100, vec![(1, row(7, 8))]), (104, vec![(2, row(8, 8))]), (108, vec![(1, row(8, 8)), (3, row(7, 8))])];
    layer.paint_hours();
    assert_eq!(layer.hour_sets, vec![(100, vec![(1, 2)]), (104, vec![(2, 2)]), (108, vec![(1, 2), (3, 2)])]);
    assert!(layer.hour_rows.is_empty() && layer.pending_provinces.is_empty(), "every hour is dated and nothing is left for the publication's own");
    assert_eq!(layer.interior_keys, [2].into(), "the key the hours painted with is coloured whatever holds it at the end");
    assert_eq!(layer.country_of[1..=3], [2, 2, 2], "and the arrays stand where the publication's last hour left them");
    assert_eq!(layer.recorded[2], Some(row(8, 8)), "the cells each province was painted from are kept for the hours that write only some of them");
}

/// A country whose appearance changed inside a publication shows its old colour before the hour it
/// changed and the new one from that hour, and a province that took its key earlier in the
/// publication shows the colour of its own hour. A colour a subject's change gave stands from the
/// hour the relation changed. Before, every hour of a publication carried the colour the
/// publication ended with.
#[test]
fn a_colour_changed_inside_a_publication_shows_from_the_hour_it_changed() {
    use std::collections::BTreeMap;
    let mut layer = Political::new(content());
    let (old, new) = ([10, 20, 30], [255, 0, 0]);
    layer.colours = vec![None, Some(new)];
    layer.colour_native = vec![super::NONE, 70];
    let channel = |value: f32| Some(value.to_bits());
    let appearance = vec![(104, vec![(70, 0, channel(1.0)), (70, 1, channel(0.0)), (70, 2, channel(0.0))])];
    let painted = || vec![(102, vec![(3, 1)]), (106, vec![(4, 1)])];
    let dated = layer.dated_hours(painted(), &BTreeMap::from([(1, (Some(old), 70))]), &appearance, None, 110).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (104, vec![], vec![(1, Some(new))]), (106, vec![(4, 1)], vec![(1, Some(new))])]);
    let dated = layer.dated_hours(painted(), &BTreeMap::from([(1, (Some(old), 80))]), &[], Some(105), 110).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (105, vec![], vec![(1, Some(new))]), (106, vec![(4, 1)], vec![(1, Some(new))])]);
    // A subject takes its overlord's colour at 105 and the overlord's appearance changes at 135:
    // the subject shows the overlord's colour of 105 until 135. Before, the colour the publication
    // ended with stood from 105.
    let overlord = [0, 0, 255];
    layer.colours = vec![None, Some(new), Some(new)];
    layer.colour_native = vec![super::NONE, 70, 70];
    let later = vec![(135, vec![(70, 0, channel(1.0)), (70, 2, channel(0.0))])];
    let before = BTreeMap::from([(1, (Some(old), 80)), (2, (Some(overlord), 70))]);
    let dated = layer.dated_hours(vec![(102, vec![(3, 1)])], &before, &later, Some(105), 148).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (105, vec![], vec![(1, Some(overlord))]), (135, vec![], vec![(1, Some(new)), (2, Some(new))])]);
    // A country holding a province only inside the publication is coloured at its hours, though no
    // province holds its key at the end. Before, its province was drawn black.
    layer.colours = vec![None, None, Some(overlord)];
    let dated = layer.dated_hours(vec![(105, vec![(5, 2)]), (120, vec![(5, 1)])], &BTreeMap::from([(2, (None, super::NONE))]), &[], None, 148).0;
    assert_eq!(dated[0], (105, vec![(5, 2)], vec![(2, Some(overlord))]));
}

/// A province changing owner inside a publication dates the border plans it changed at its own
/// hour, and a zone flagged later in the publication is dated at the flag's hour, after it. Before,
/// every interior hour carried the publication's last plans, and the later zone was applied before
/// the earlier transfer.
#[test]
fn borders_and_zones_are_dated_at_the_hours_they_changed() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0), (8, 1)].into_iter().collect();
    layer.natives = [(0, 70), (1, 80)].into_iter().collect();
    layer.resize();
    let row = |owner, state| Row { state: Some(state), owner: Some(owner), controller: Some(owner) };
    for id in 1..=PROVINCES {
        layer.paint_from(id, Some(row(7, if id >= 30 { 9 } else { 5 })));
    }
    let edges = layer.content.edges.len();
    layer.class = vec![LEFT_OUT; edges];
    (0..edges as u32).for_each(|edge| layer.classify(edge));
    layer.prepare();
    assert!(layer.classes.is_some() && !layer.edges_changed, "the plans stand before the publication");
    (layer.edges, layer.pending_chunks, layer.hour_plans) = (Default::default(), Default::default(), Vec::new());
    layer.pending_provinces.clear();

    layer.hour_rows = vec![(104, vec![(2, row(8, 5))])];
    layer.pending_zone = Some((140, HashSet::from([9])));
    layer.hour = 148;
    layer.paint_hours();
    let hours: Vec<replay_data::Hour> = layer.hour_plans.iter().map(|(at, _)| *at).collect();
    assert_eq!(hours, [104, 140], "the transfer's plans at 104, the zone's at 140");
    assert_eq!(layer.class[0], BorderClass::Country.bucket() as u8, "the boundary 1-2 is a country border");
    assert_ne!(layer.hour_plans[0].1.1, layer.hour_plans[1].1.1, "the zone's plan changes at 140, not at 104");
    assert_eq!(layer.hour_plans[0].1.1, ZonePlan::default(), "and no zone stands at 104");
}

/// A whole fill folded with a later write keeps the write: the frame writes the whole first and then
/// what came after it. Before, the whole was taken and the later transfer left out.
#[test]
fn a_whole_folded_with_a_later_write_keeps_the_write() {
    let (_sets, mut installer) = handover();
    let mut front = whole(100, 1, vec![0, 4, 4]);
    front.fold(set(101, 1, 2, 9));
    installer.sets.push_back(front);
    let picked = installer.pick(Some(101));
    assert!(picked.whole.is_some());
    assert_eq!(installer.keys_of[2], 9, "province 2 took key 9 at 101, after the whole fill of 100");
    assert_eq!(picked.provinces, [2]);
}

/// Frames whose head stands still take every set handed over and write none past it: the sets they
/// hold are folded as the hand-over folds them, however long the head stands.
#[test]
fn a_head_held_back_holds_a_bounded_queue() {
    let (sets, mut installer) = handover();
    for hour in 1..=(super::SETS_HELD as i32 * 2) {
        sets.push(set(100 + hour, 1, 7, hour as u32));
        installer.pick(Some(100));
    }
    assert!(installer.sets.len() <= super::SETS_HELD, "{} sets held", installer.sets.len());
    let picked = installer.pick(None);
    assert_eq!(installer.keys_of[7], super::SETS_HELD as u32 * 2, "and every change is written once the head moves");
    assert_eq!(picked.provinces, [7]);
}

/// An inner hour that refuses a province's row is not one the hours can show: no set is dated from
/// it on. Before, the province was drawn with no fill and no refusal at that hour.
#[test]
fn an_inner_hour_refusing_a_province_is_not_dated() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0)].into_iter().collect();
    layer.natives = [(0, 70)].into_iter().collect();
    layer.resize();
    let row = |controller: Option<u32>| Row { state: Some(5), owner: Some(7), controller };
    layer.paint_from(3, Some(row(Some(7))));
    layer.pending_provinces.clear();
    layer.hour_rows = vec![(105, vec![(3, row(None))]), (120, vec![(3, row(Some(7)))])];
    layer.paint_hours();
    assert_eq!(layer.undated_from, Some(105), "the hours from 105 on wait for the publication's own");
}

/// A colour the hours cannot follow stops the dated hours at its first write: a key starting grey
/// (the fallback, no channels known) whose country's appearance is written, and a key whose former
/// source was written to before the change.
#[test]
fn a_colour_that_cannot_be_followed_is_not_dated() {
    use std::collections::BTreeMap;
    let mut layer = Political::new(content());
    layer.colours = vec![None, Some([255, 0, 0])];
    layer.colour_native = vec![super::NONE, 70];
    let channel = |value: f32| Some(value.to_bits());
    let red = vec![(105, vec![(70, 0, channel(1.0))])];
    let (_, undated) = layer.colour_hours(&BTreeMap::from([(1, (Some(super::UNAVAILABLE_COLOR), 70))]), &red, None);
    assert_eq!(undated, Some(105), "a grey start has no channels to follow");
    let earlier = vec![(103, vec![(80, 0, channel(1.0))])];
    let (_, undated) = layer.colour_hours(&BTreeMap::from([(1, (Some([0, 0, 255]), 80))]), &earlier, Some(105));
    assert_eq!(undated, Some(103), "the former source moved before the change");
}

/// Sea region 10's naval control turns from the viewer (slot 1) to its enemy (slot 2) at hour 9;
/// the political core tables are recorded, with no rows.
fn naval_turn(name: &str) -> crate::playhead_host::units::Units {
    use replay_columnar::{manifest::Status, state::Cell, store::Store};
    use replay_data::rows::TableId;
    let column = crate::layers::recorded::column;
    let mut tables: Vec<TableId> = viewer_foundation::map::perspective::VISION_TABLES.to_vec();
    tables.extend([TableId::NavalSupremacy, TableId::RelationTuples, TableId::Provinces, TableId::CountryAppearance, TableId::Autonomy, TableId::StateFlags, TableId::States]);
    tables.sort_unstable();
    tables.dedup();
    crate::layers::recorded::record(name, 24, |at, store| {
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
        let insert = |store: &mut Store, table: TableId, slot: u32, key: u128| store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
        let supremacy = |store: &mut Store, slot: u32, current: i64, previous: i64| {
            write(store, TableId::NavalSupremacy, slot, "current", Cell::I64(current));
            write(store, TableId::NavalSupremacy, slot, "previous", Cell::I64(previous));
        };
        match at {
            0 => {
                for (slot, raw) in [101u32, 102].into_iter().enumerate() {
                    insert(store, TableId::CountryLinks, slot as u32, u128::from(raw));
                    write(store, TableId::CountryLinks, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                    insert(store, TableId::Countries, slot as u32, u128::from(raw));
                    write(store, TableId::Countries, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                }
                insert(store, TableId::LocalPlayer, 0, 0);
                write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
                insert(store, TableId::WarRelations, 0, 101 << 32 | 102);
                for (slot, country) in [101u128, 102].into_iter().enumerate() {
                    insert(store, TableId::NavalSupremacy, slot as u32, 10 << 32 | country);
                }
                supremacy(store, 0, 900_000, 900_000);
                supremacy(store, 1, 100_000, 50_000);
            }
            9 => supremacy(store, 1, 2_000_000, 100_000),
            _ => {}
        }
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    })
}

/// Plays `name` four hours a step to hour 20 with the political, vision and naval layers in the
/// order the map registers them, the political layer's installer taken as the side's thread takes
/// it. After each frame that took a step or swapped a composed picture in (then a whole settle when
/// `whole`), a frame writes through the step's hour from the hand-over, and the sea regions'
/// attributes it then holds must be the naval layer's at that hour. Returns the hours whose naval
/// attributes changed from the hour checked before.
fn naval_dated_with_its_hour(name: &str, whole: bool) -> Vec<replay_data::Hour> {
    use crate::{
        layers::{Layer, View, naval::Naval, vision},
        playhead_host::{MapSide, open},
    };
    use std::collections::{BTreeMap, BTreeSet};
    use viewer_foundation::map::visibility::{FogContent, Topology, test_defines};
    let definition = |id, kind| (id, ProvinceDefinition { id, rgb: [0; 3], kind, continent: 0 });
    let map = MapData { width: 2, height: 1, province_ids: vec![1, 2], definitions: [definition(1, ProvinceKind::Land), definition(2, ProvinceKind::Sea)].into_iter().collect(), country_colors: Default::default() };
    let empty = |class| BorderMesh { class, chunks: vec![], draws: vec![] };
    let meshes = BorderMeshes { meshes: BorderClass::ALL.iter().map(|&class| empty(class)).collect(), hidden: empty(BorderClass::Province), indices: vec![], one_pixel_provinces: vec![] };
    let fog = FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors: BTreeMap::from([(1, vec![2]), (2, vec![1])]) }, defines: test_defines() };
    let layers: Vec<Box<dyn Layer>> = vec![
        Box::new(Political::new(Arc::new(Content::new(Arc::new(map), Arc::new(meshes))))),
        Box::new(vision::Vision::new(Arc::new(vision::Content::new(Ok(fog))))),
        Box::new(Naval::new(Arc::new(viewer_world::map::naval_dominance::Content::for_tests(&[(10, true, 100_000)], 60_000)))),
    ];
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..playhead::Settings::default() };
    let entry = open(naval_turn(name), settings).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    drop(side.take_installs());
    let mut installer = {
        let political = side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Political>()).unwrap();
        Installer::new(Arc::clone(political.sets.as_ref().expect("the installer was taken")), Arc::default(), Arc::default(), Arc::clone(&political.live), Arc::clone(&political.generation))
    };
    // Held, an ordinary step settles the timed layers alone, so what the naval layer settles
    // reaches the fill through the whole settle only.
    let _held = whole.then(|| side.hold_settles());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    let (mut applied, mut before, mut turned) = (None::<replay_data::Hour>, None, Vec::new());
    for _ in 0..50_000_000u64 {
        let head = match applied {
            None => 0,
            Some(hour) if side.composing() => hour,
            Some(hour) => entry.playhead().newest().unwrap_or(hour).min(hour + 4),
        };
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        let Some(hour) = applied.filter(|_| (frame.steps > 0 || frame.swapped.is_some()) && !side.composing()) else {
            std::thread::yield_now();
            continue;
        };
        if whole {
            side.settle_all().unwrap();
        }
        if side.draws("political") && side.draws("naval") {
            installer.pick(Some(hour));
            let naval = side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Naval>()).unwrap();
            let attributes = (**naval.attributes()).clone();
            assert_eq!(installer.naval, attributes, "the political sets dated through hour {hour} carry the naval attributes of hour {hour}");
            if before.as_ref().is_some_and(|before| *before != attributes) {
                turned.push(hour);
            }
            before = Some(attributes);
        }
        if hour >= 20 {
            return turned;
        }
    }
    panic!("hour 20 was not reached: applied {applied:?}, composing {}", side.composing());
}

/// An ordinary step dates the sea regions' naval attributes in the political set of its own hour.
/// Before, the naval layer handed them to the fill at the display frame's upload, after the side
/// had dated the hour, so the set of an hour carried the attributes of the step before it.
#[test]
fn a_step_dates_naval_attributes_at_their_own_hour() {
    let turned = naval_dated_with_its_hour("political naval step", false);
    assert!(turned.iter().any(|&hour| (9..=16).contains(&hour)), "region 10 turned at hour 9, seen at {turned:?}");
}

/// A whole settle hands the naval attributes over before the political layer dates its hour, with
/// the ordinary steps settling no naval change.
#[test]
fn a_whole_settle_dates_naval_attributes_at_their_own_hour() {
    let turned = naval_dated_with_its_hour("political naval whole", true);
    assert!(turned.iter().any(|&hour| (9..=16).contains(&hour)), "region 10 turned at hour 9, seen at {turned:?}");
}

/// A scene that refuses the country fill's write of what changed and records the border plans it is
/// given, as two views sharing one fill refuse it.
#[derive(Default)]
struct Refusing {
    plans: usize,
}

impl super::Sink for Refusing {
    fn set_political(&mut self, _regions: &Arc<Regions>) -> std::io::Result<()> {
        Ok(())
    }

    fn update_naval(&mut self, _attributes: &dyn Fn(u32) -> super::Attributes, _regions: &[u32]) -> std::io::Result<bool> {
        Ok(true)
    }

    fn update_political(&mut self, _keys_of: &[u32], _colour: &dyn Fn(u32) -> [u8; 3], _provinces: &[usize], _keys: &[u32]) -> std::io::Result<bool> {
        Ok(false)
    }

    fn set_border_plans(&mut self, _classes: &viewer_world::map::border_pass::ClassPlan, _zone: &ZonePlan) -> std::io::Result<()> {
        self.plans += 1;
        Ok(())
    }
}

/// A fill that refuses a set's write keeps its hour and the borders it has: the set's border plans
/// are not written over the older fill, and the whole fill is asked for. Before, the new borders
/// were drawn around the old territory until the whole fill came.
#[test]
fn a_refused_fill_write_keeps_its_hour_and_its_borders() {
    let (sets, mut installer) = handover();
    sets.push(whole(100, 1, vec![0, 4, 4]));
    let mut moved = set(101, 1, 2, 9);
    moved.plans = Some((Default::default(), ZonePlan::default()));
    sets.push(moved);
    let mut scene = Refusing::default();
    let mut phases = [0; 4];
    let picked = installer.pick(Some(100));
    installer.write(&mut scene, picked, Some(100), &mut phases).unwrap();
    assert_eq!(installer.at, Some(100));
    let picked = installer.pick(Some(101));
    installer.write(&mut scene, picked, Some(101), &mut phases).unwrap();
    assert_eq!(installer.at, Some(100), "the refused hour is not on screen");
    assert_eq!(scene.plans, 0, "and its borders are not written");
    assert!(installer.refused && installer.asks_whole.load(Ordering::Acquire), "the whole fill is asked for");
}

/// The game hours a step of [`political_side`] covers: steps 1 to 12, 13 to 24, 25 to 36 after the
/// seek to hour 0, so a test's events at 15 and 19 are inner hours of one publication.
const STEP: replay_data::Hour = 12;

/// Countries by raw link, canonical slot and native id, each with its appearance colour.
const NATIONS: [(u32, u32, u32, [f32; 3]); 3] = [(7, 0, 70, [0.2, 0.2, 0.2]), (8, 1, 80, [0.4, 0.4, 0.4]), (9, 2, 90, [0.6, 0.6, 0.6])];

/// The fill colour an appearance's channels give.
fn shade(channels: [f32; 3]) -> Option<[u8; 3]> {
    Some(channels.map(|value| (value * 255.0) as u8))
}

fn write_cell(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32, field: &str, cell: replay_columnar::state::Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, crate::layers::recorded::column(table, field), cell).unwrap();
}

fn insert_row(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn delete_row(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32) {
    store.table(table.index()).unwrap().delete(slot).unwrap();
}

/// Province `id`'s owner and controller, as raw country links.
fn hold(store: &mut replay_columnar::store::Store, id: u32, owner: u32, controller: u32) {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    write_cell(store, TableId::Provinces, id - 1, "owner", Cell::U32(owner));
    write_cell(store, TableId::Provinces, id - 1, "controller", Cell::U32(controller));
}

/// The appearance row at `slot` coloured `channels`.
fn paint_appearance(store: &mut replay_columnar::store::Store, slot: u32, channels: [f32; 3]) {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    for (field, value) in ["red", "green", "blue"].into_iter().zip(channels) {
        write_cell(store, TableId::CountryAppearance, slot, field, Cell::U32(value.to_bits()));
    }
}

/// 48 hours of a recorded political map over [`world`]: [`NATIONS`] linked, counted and coloured,
/// every province land of link 7 in state 5 (30 on in state 9), state 9 demilitarised, and every
/// table the layer reads wholly recorded each hour but Provinces in `provinces_out`. `campaign`
/// writes each hour after that. The texts are dictionary ids 0 ("puppet") and 1 ("outage").
fn politics(name: &str, provinces_out: std::ops::RangeInclusive<u32>, mut campaign: impl FnMut(u32, &mut replay_columnar::store::Store)) -> crate::playhead_host::units::Units {
    use replay_columnar::{manifest::Status, state::Cell};
    use replay_data::rows::TableId;
    let tables = [TableId::Provinces, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::Subjects, TableId::Autonomy, TableId::StateFlags, TableId::States];
    crate::layers::recorded::record_naming(name, 48, &[b"puppet", b"outage"], |at, store| {
        if at == 0 {
            for (slot, &(raw, canonical, native, channels)) in NATIONS.iter().enumerate() {
                let slot = slot as u32;
                insert_row(store, TableId::CountryLinks, slot, u128::from(raw));
                write_cell(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(canonical));
                insert_row(store, TableId::Countries, slot, u128::from(native));
                write_cell(store, TableId::Countries, slot, "canonical_slot", Cell::U32(canonical));
                insert_row(store, TableId::CountryAppearance, slot, u128::from(native));
                paint_appearance(store, slot, channels);
            }
            for id in 1..=PROVINCES {
                insert_row(store, TableId::Provinces, id - 1, u128::from(id));
                write_cell(store, TableId::Provinces, id - 1, "state", Cell::U32(if id >= 30 { 9 } else { 5 }));
                hold(store, id, 7, 7);
            }
            for (slot, state) in [5u128, 9].into_iter().enumerate() {
                insert_row(store, TableId::States, slot as u32, state);
            }
            insert_row(store, TableId::StateFlags, 0, 9);
            write_cell(store, TableId::StateFlags, 0, "demilitarized", Cell::U8(1));
        }
        campaign(at, store);
        for table in tables {
            let status = if table == TableId::Provinces && provinces_out.contains(&at) { Status::Unavailable { reason: 1 } } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
    })
}

/// A map side drawing the political layer alone over `units`, its installer taken as the side's
/// thread takes it, playing [`STEP`] hours a step.
fn political_side(units: crate::playhead_host::units::Units, content: Arc<Content>) -> crate::playhead_host::MapSide {
    use crate::{
        layers::{Layer, View},
        playhead_host::{MapSide, open},
    };
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Political::new(content))];
    let speed = f64::from(STEP);
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 8, speed, rate: 1.0, ..playhead::Settings::default() };
    let entry = open(units, settings).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    drop(side.take_installs());
    entry.playhead().play(speed);
    entry.playhead().seek(0);
    side
}

fn political(side: &crate::playhead_host::MapSide) -> &Political {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Political>()).unwrap()
}

/// Frames until the step ending at `target` is applied, each frame given the hour it applied as its
/// head until the steps through `target` are built, then `target`: every step built by then is
/// applied in that one frame, one publication. The sets the layer hands over are added to `sets`.
fn frame_to(side: &mut crate::playhead_host::MapSide, sets: &mut Vec<Set>, target: replay_data::Hour) {
    let handover = Arc::clone(political(side).sets.as_ref().expect("the installer was taken"));
    for _ in 0..50_000_000u64 {
        crate::layers::recorded::reads_landed(side);
        let built = side.entry().playhead().newest().is_some_and(|newest| newest >= target);
        let ready = built && side.applied().is_some() && !side.composing();
        let head = if ready { target } else { side.applied().unwrap_or(0) };
        let frame = side.frame(head).unwrap();
        sets.extend(handover.take());
        if side.applied().is_some_and(|applied| applied >= target) && !side.composing() {
            assert_eq!(side.applied(), Some(target), "the steps end at the hours a test names: {}", crate::layers::recorded::seen(side));
            return;
        }
        assert!(!ready || frame.steps > 0, "the frame at {target} took the steps built through it: {}", crate::layers::recorded::seen(side));
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("hour {target} was not applied: {}", crate::layers::recorded::seen(side));
}

/// What the scene was last given: whether the fill was blank (refused), and the border plans.
#[derive(Default)]
struct Screen {
    refuse: bool,
    blank: bool,
    plans: Option<(viewer_world::map::border_pass::ClassPlan, ZonePlan)>,
    plans_written: usize,
}

impl super::Sink for Screen {
    fn set_political(&mut self, regions: &Arc<Regions>) -> std::io::Result<()> {
        self.blank = (1..=PROVINCES as usize).all(|id| regions.country_of.get(id) == Some(&0));
        Ok(())
    }

    fn update_naval(&mut self, _attributes: &dyn Fn(u32) -> super::Attributes, _regions: &[u32]) -> std::io::Result<bool> {
        Ok(!self.refuse)
    }

    fn update_political(&mut self, _keys_of: &[u32], _colour: &dyn Fn(u32) -> [u8; 3], _provinces: &[usize], _keys: &[u32]) -> std::io::Result<bool> {
        Ok(!self.refuse)
    }

    fn set_border_plans(&mut self, classes: &viewer_world::map::border_pass::ClassPlan, zone: &ZonePlan) -> std::io::Result<()> {
        self.plans = Some((classes.clone(), zone.clone()));
        self.plans_written += 1;
        Ok(())
    }
}

/// Plays `units` through hour 36 in the publications `targets` end at, then writes the sets the
/// layer handed over through the display's own installer, one frame an hour from 0 to 36. At every
/// hour the installer says is on screen (its `at`), `truth` names what is wrong with the picture
/// it holds, if anything: the hours a publication certifies must show what those hours were.
fn certified(name: &str, units: crate::playhead_host::units::Units, content: Arc<Content>, targets: &[replay_data::Hour], mut truth: impl FnMut(replay_data::Hour, &Installer, &Screen) -> Option<String>) {
    let mut side = political_side(units, content);
    let mut sets = Vec::new();
    for &target in targets {
        frame_to(&mut side, &mut sets, target);
    }
    let generation = Arc::clone(&political(&side).generation);
    let taken = Arc::new(super::Handover::default());
    for set in sets {
        taken.push(set);
    }
    let mut installer = Installer::new(taken, Arc::default(), Arc::default(), Arc::new(std::sync::atomic::AtomicBool::new(true)), generation);
    let (mut screen, mut phases) = (Screen::default(), [0; 4]);
    let mut wrong = Vec::new();
    for hour in 0..=36 {
        let picked = installer.pick(Some(hour));
        installer.write(&mut screen, picked, Some(hour), &mut phases).unwrap();
        if installer.at == Some(hour)
            && let Some(why) = truth(hour, &installer, &screen)
        {
            wrong.push(format!("hour {hour}: {why}"));
        }
    }
    assert!(wrong.is_empty(), "{name}: the installer said these hours were on screen with another hour's picture: {wrong:#?}");
}

/// Provinces is unavailable from hour 15 through 26, across two publications: a direct compose of
/// those hours refuses the map, so neither publication certifies an inner hour with the picture of
/// its other side. Before, the refused fill of hour 24 stood from 24 and the recorded one of 36
/// from 36, so hours 15 to 23 were said shown recorded and 27 to 35 refused.
#[test]
fn a_core_table_unavailable_inside_publications_certifies_none_of_its_hours() {
    let units = politics("political core outage", 15..=26, |_, _| {});
    certified("core outage", units, content(), &[12, 24, 36], |hour, installer, _| {
        let recorded = !(15..=26).contains(&hour);
        let key = installer.keys_of.get(1).copied().unwrap_or(0);
        (recorded != (key == 1)).then(|| format!("province 1 shows key {key}, the map {} recorded", if recorded { "being" } else { "not" }))
    });
}

/// State 9's demilitarised row is deleted at hour 15: the zone ends at 15, inside the publication.
/// Before, a row gone was no flag write, so the new zone stood at the publication's hour 24 and
/// hours 15 to 23 were certified with the old zone's borders.
#[test]
fn a_demilitarised_row_gone_inside_a_publication_ends_the_zone_at_its_hour() {
    use replay_data::rows::TableId;
    let units = politics("political zone row gone", 1..=0, |at, store| {
        if at == 15 {
            delete_row(store, TableId::StateFlags, 0);
        }
    });
    certified("zone row gone", units, content(), &[12, 24, 36], |hour, _, screen| {
        let zoned = hour < 15;
        let shown = screen.plans.as_ref().is_some_and(|(_, zone)| *zone != ZonePlan::default());
        (zoned != shown).then(|| format!("the borders show {} zone", if shown { "a" } else { "no" }))
    });
}

/// Province 2 passes to link 8 at hour 15, and link 8 moves from slot 1 to slot 2 at 19, inside
/// the same publication: the province is key 2 from 15 and key 3 from 19. Before, hour 15 was
/// painted through the links as the publication leaves them and dated key 3 from 15.
#[test]
fn a_link_written_inside_a_publication_undates_the_provinces_painted_before_it() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political link rewritten", 1..=0, |at, store| match at {
        15 => hold(store, 2, 8, 8),
        19 => write_cell(store, TableId::CountryLinks, 1, "canonical_slot", Cell::U32(2)),
        _ => {}
    });
    certified("link rewritten", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..15 => 1,
            15..19 => 2,
            _ => 3,
        };
        let key = installer.keys_of.get(2).copied().unwrap_or(0);
        (key != truth).then(|| format!("province 2 shows key {key}, not {truth}"))
    });
}

/// Provinces is unavailable for hours 15 to 17 alone, inside one step that ends recorded as it
/// began: the step reports no status, only the hour the status left. Before, hours 15 to 17 were
/// certified with the recorded map.
#[test]
fn a_core_table_unavailable_and_back_inside_one_step_certifies_none_of_its_hours() {
    let units = politics("political core blink", 15..=17, |_, _| {});
    certified("core blink", units, content(), &[12, 24, 36], |hour, installer, _| {
        let recorded = !(15..=17).contains(&hour);
        let key = installer.keys_of.get(1).copied().unwrap_or(0);
        (recorded != (key == 1)).then(|| format!("province 1 shows key {key}, the map {} recorded", if recorded { "being" } else { "not" }))
    });
}

/// Province 2 passes to link 8 at hour 5, a publication before link 8 moves from slot 1 to slot 2
/// at 19: the province, painted in no step of that publication, is key 3 from 19. Before, hours 19
/// to 23 were certified with key 2, the slot the link left.
#[test]
fn a_link_written_inside_a_publication_undates_the_provinces_already_on_it() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political link under a province", 1..=0, |at, store| match at {
        5 => hold(store, 2, 8, 8),
        19 => write_cell(store, TableId::CountryLinks, 1, "canonical_slot", Cell::U32(2)),
        _ => {}
    });
    certified("link under a province", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..5 => 1,
            5..19 => 2,
            _ => 3,
        };
        let key = installer.keys_of.get(2).copied().unwrap_or(0);
        (key != truth).then(|| format!("province 2 shows key {key}, not {truth}"))
    });
}

/// Native 80's appearance turns red at hour 15 in one step; in the next, of the same publication,
/// its row and native 90's swap slots, so the publication's last slots name 90 where 80 was
/// written. Before, the write of 15 was read as 90's and dated, and 80's own change waited for the
/// publication's hour 36.
#[test]
fn appearance_rows_moved_between_slots_undate_the_appearance_writes() {
    use replay_data::rows::TableId;
    let red = [1.0, 0.0, 0.0];
    let units = politics("political appearance reslotted", 1..=0, |at, store| match at {
        0 => {
            hold(store, 10, 8, 8);
            hold(store, 20, 9, 9);
        }
        15 => paint_appearance(store, 1, red),
        27 => {
            delete_row(store, TableId::CountryAppearance, 1);
            delete_row(store, TableId::CountryAppearance, 2);
            insert_row(store, TableId::CountryAppearance, 1, 90);
            paint_appearance(store, 1, NATIONS[2].3);
            insert_row(store, TableId::CountryAppearance, 2, 80);
            paint_appearance(store, 2, red);
        }
        _ => {}
    });
    // The steps 13 to 24 and 25 to 36 are one publication.
    certified("appearance reslotted", units, content(), &[12, 36], |hour, installer, _| {
        let colour = |key: usize| installer.colours.get(key).copied().flatten();
        let eighty = if hour < 15 { shade(NATIONS[1].3) } else { shade(red) };
        let ninety = shade(NATIONS[2].3);
        (colour(2) != eighty || colour(3) != ninety).then(|| format!("keys 2 and 3 are {:?} and {:?}, not {eighty:?} and {ninety:?}", colour(2), colour(3)))
    });
}

/// Province 3 is held by link 9 for hours 15 and 16 alone. Link 9 is a puppet taking its colour
/// from link 6, whose slot no country holds: key 3's colour is refused, but only at those inner
/// hours. The publication's end at 24 holds no province of key 3 and is drawn. Before, the inner
/// key's refusal was still held when the end's fill was made, which was blank from 24 to 35.
#[test]
fn an_inner_keys_colour_refusal_does_not_blank_the_publications_end() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political inner colour refusal", 1..=0, |at, store| match at {
        0 => {
            insert_row(store, TableId::CountryLinks, 3, 6);
            write_cell(store, TableId::CountryLinks, 3, "canonical_slot", Cell::U32(5));
            insert_row(store, TableId::Subjects, 0, 6 << 32 | 9);
            write_cell(store, TableId::Subjects, 0, "start_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "end_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "cancel", Cell::U8(0));
            insert_row(store, TableId::Autonomy, 0, 9);
            write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0));
        }
        15 => hold(store, 3, 7, 9),
        17 => hold(store, 3, 7, 7),
        _ => {}
    });
    let (mut map, meshes) = world();
    map.country_colors.use_overlord_color.insert(b"puppet".to_vec(), true);
    let content = Arc::new(Content::new(Arc::new(map), Arc::new(meshes)));
    certified("inner colour refusal", units, content, &[12, 24, 36], |hour, _, screen| {
        let refused = (15..=16).contains(&hour);
        (screen.blank != refused).then(|| format!("the fill is {}", if screen.blank { "blank" } else { "drawn" }))
    });
}

/// Link 9, holding province 20, is a puppet of link 8 and takes its colour; its autonomy leaves
/// puppet at hour 15 and comes back at 19, inside one publication, so key 3 has its own colour from
/// 15 to 18 alone. The relations read at the publication's end are those of its start. Before,
/// hours 15 to 18 were certified with the overlord's colour.
#[test]
fn a_subject_relation_undone_inside_a_publication_dates_none_of_the_hours_between() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political relation undone", 1..=0, |at, store| match at {
        0 => {
            hold(store, 20, 9, 9);
            insert_row(store, TableId::Subjects, 0, 8 << 32 | 9);
            write_cell(store, TableId::Subjects, 0, "start_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "end_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "cancel", Cell::U8(0));
            insert_row(store, TableId::Autonomy, 0, 9);
            write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0));
        }
        15 => write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(1)),
        19 => write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0)),
        _ => {}
    });
    let (mut map, meshes) = world();
    map.country_colors.use_overlord_color.insert(b"puppet".to_vec(), true);
    let content = Arc::new(Content::new(Arc::new(map), Arc::new(meshes)));
    certified("relation undone", units, content, &[12, 24, 36], |hour, installer, _| {
        let colour = installer.colours.get(3).copied().flatten();
        let truth = if (15..19).contains(&hour) { shade(NATIONS[2].3) } else { shade(NATIONS[1].3) };
        (colour != truth).then(|| format!("key 3 is {colour:?}, not {truth:?}"))
    });
}

/// Key 2's colour source moves from native 80 to native 85 at hour 19 (a Countries row, which no
/// hour dates), after 80's appearance was written at 15: the new source cannot be followed, and
/// the old one's write caps the hours. Before, the old source capped them only when the new one
/// was followed, so 80's colour of before stood from 13 to 23.
#[test]
fn an_old_colour_source_written_caps_the_hours_when_the_new_one_is_not_followed() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let (red, blue) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let units = politics("political old colour source", 1..=0, |at, store| match at {
        0 => {
            hold(store, 10, 8, 8);
            insert_row(store, TableId::CountryAppearance, 3, 85);
            paint_appearance(store, 3, blue);
        }
        15 => paint_appearance(store, 1, red),
        19 => {
            insert_row(store, TableId::Countries, 3, 85);
            write_cell(store, TableId::Countries, 3, "canonical_slot", Cell::U32(1));
        }
        _ => {}
    });
    certified("old colour source", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..15 => shade(NATIONS[1].3),
            15..19 => shade(red),
            _ => shade(blue),
        };
        let colour = installer.colours.get(2).copied().flatten();
        (colour != truth).then(|| format!("key 2 is {colour:?}, not {truth:?}"))
    });
}

/// The display's fill refuses the write of hour 24's sets (province 2's transfer at 15 and its
/// borders): the whole fill the side then hands over carries the borders as the end leaves them,
/// and writing it puts them on screen. Before, a whole fill carried plans only when they were
/// built again, so the recovery left the borders the refused write never replaced.
#[test]
fn a_recovery_whole_after_a_refused_write_carries_the_ends_borders() {
    let units = politics("political refused recovery", 1..=0, |at, store| {
        if at == 15 {
            hold(store, 2, 8, 8);
        }
    });
    let mut side = political_side(units, content());
    let mut sets = Vec::new();
    frame_to(&mut side, &mut sets, 12);
    let layer = political(&side);
    let mut installer = Installer::new(Arc::clone(layer.sets.as_ref().unwrap()), Arc::default(), Arc::clone(&layer.asks_whole), Arc::new(std::sync::atomic::AtomicBool::new(true)), Arc::clone(&layer.generation));
    let (mut screen, mut phases) = (Screen::default(), [0; 4]);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    let picked = installer.pick(Some(12));
    installer.write(&mut screen, picked, Some(12), &mut phases).unwrap();
    assert!(screen.plans.is_some(), "the first whole fill carries its borders");
    let before = screen.plans.clone();

    frame_to(&mut side, &mut sets, 24);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    screen.refuse = true;
    let picked = installer.pick(Some(24));
    assert!(picked.plans.is_some(), "the transfer at 15 moved a country border");
    installer.write(&mut screen, picked, Some(24), &mut phases).unwrap();
    assert!(installer.refused && political(&side).asks_whole.load(Ordering::Acquire), "the refused write asks the side for the whole fill");
    assert_eq!(screen.plans, before, "and the refused hour's borders are not written");

    screen.refuse = false;
    frame_to(&mut side, &mut sets, 36);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    let picked = installer.pick(Some(36));
    assert!(picked.whole.is_some(), "the side answered with the whole fill");
    let written = screen.plans_written;
    installer.write(&mut screen, picked, Some(36), &mut phases).unwrap();
    let layer = political(&side);
    let end = layer.classes.clone().zip(layer.zone_plan.clone());
    assert!(screen.plans_written > written, "the recovery whole writes borders");
    assert_eq!(screen.plans, end, "and they are the borders the end leaves");
    assert_ne!(screen.plans, before, "which the transfer at 15 changed");
}
