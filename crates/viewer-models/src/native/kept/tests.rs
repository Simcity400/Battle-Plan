use crate::native::kept::*;
use viewer_gpu::camera::{MapDefines, MapSize, Pose};

const ALIGNMENT: usize = 256;

fn camera() -> Camera {
    Camera {
        center: [3200., 1408.],
        height: 100.,
        map: MapSize {
            width: 5632,
            height: 2048,
        },
        defines: MapDefines::test(),
        pose: Pose::Map,
    }
}

fn frame(camera: Camera, offsets: &[f32]) -> ModelFrame<'_> {
    ModelFrame {
        camera,
        offsets,
        visible: Some([0., 0., 100., 100.]),
        seen: Some([0., 0., 100., 100.]),
        alpha: 1.,
    }
}

const SCENE: [u8; 416] = [7; 416];

#[test]
fn a_frame_from_the_same_inputs_keeps_the_list_and_each_input_makes_it_again() {
    let offsets = [0., 5632.];
    let reads = Reads { views: 2, ..Reads::default() };
    let made = Made::new(9, &frame(camera(), &offsets), &[&SCENE, &SCENE], reads, 1, 3, 0, &[], None);
    let same = |revision, frame: &ModelFrame<'_>, frames: &[&[u8]], samples, textures| {
        made == Made::new(revision, frame, frames, reads, samples, textures, 0, &[], None)
    };
    assert!(same(9, &frame(camera(), &offsets), &[&SCENE, &SCENE], 1, 3));
    assert!(same(9, &frame(camera(), &offsets), &[&[1; 416], &[2; 416]], 1, 3));

    assert!(!same(10, &frame(camera(), &offsets), &[&SCENE, &SCENE], 1, 3), "revision");
    let mut moved = camera();
    moved.center[0] += 0.5;
    assert!(same(9, &frame(moved, &offsets), &[&SCENE, &SCENE], 1, 3), "camera");
    let mut lower = camera();
    lower.height -= 1.;
    assert!(same(9, &frame(lower, &offsets), &[&SCENE, &SCENE], 1, 3), "height");
    let mut wider = camera();
    wider.map.width += 1;
    assert!(!same(9, &frame(wider, &offsets), &[&SCENE, &SCENE], 1, 3), "map");
    assert!(!same(9, &frame(camera(), &[0.]), &[&SCENE, &SCENE], 1, 3), "views");
    assert!(!same(9, &frame(camera(), &[0., -5632.]), &[&SCENE, &SCENE], 1, 3), "offsets");
    let mut light = frame(camera(), &offsets);
    light.visible = Some([0., 0., 100., 101.]);
    assert!(!same(9, &light, &[&SCENE, &SCENE], 1, 3), "caster rectangle");
    light.visible = None;
    assert!(!same(9, &light, &[&SCENE, &SCENE], 1, 3), "every instance");
    let mut faded = frame(camera(), &offsets);
    faded.alpha = 0.5;
    assert!(!same(9, &faded, &[&SCENE, &SCENE], 1, 3), "alpha");
    assert!(!same(9, &frame(camera(), &offsets), &[&SCENE, &[0; 400]], 1, 3), "scene length");
    assert!(!same(9, &frame(camera(), &offsets), &[&SCENE, &SCENE], 4, 3), "samples");
    assert!(!same(9, &frame(camera(), &offsets), &[&SCENE, &SCENE], 1, 4), "a texture replaced");
    let narrow = crate::GameCull::of(&camera(), [800, 600]);
    let wide = crate::GameCull::of(&camera(), [1600, 600]);
    let culled = |cull| Made::new(9, &frame(camera(), &offsets), &[&SCENE, &SCENE], reads, 1, 3, 0, &[], Some(cull));
    assert_eq!(culled(narrow), culled(narrow));
    assert_ne!(culled(narrow), culled(wide), "the cull");
}

#[test]
fn a_lit_list_is_made_again_by_what_its_walk_reads_beyond_the_casters() {
    let grid = [3u8; 16];
    let lit = Reads { views: 1, grid: true, pixel: false, own_pixel: true };
    let shared: [&[u8]; 2] = [&SCENE, &grid];
    let made = |shared: &[&[u8]], reads, globals, chain: &[u8]| {
        Made::new(0, &frame(camera(), &[0.]), shared, reads, 1, 0, globals, chain, None)
    };
    let kept = made(&shared, lit, 5, &[0; 112]);
    assert_eq!(kept, made(&[&[1; 416], &[9; 16]], lit, 5, &[0; 112]), "the grid's bytes are written");
    assert_ne!(kept, made(&shared, lit, 6, &[0; 112]), "the global textures");
    assert_ne!(kept, made(&shared, lit, 5, &[1; 112]), "the incoming block's tail");
    assert_ne!(kept, made(&[&SCENE, &[0; 32]], lit, 5, &[0; 112]), "the grid's length");
    let carried = Reads { views: 1, grid: true, pixel: true, own_pixel: false };
    let snapshot = [4u8; 208];
    let with = [&SCENE[..], &grid, &snapshot];
    let kept = made(&with, carried, 5, &[2; 144]);
    assert_eq!(kept, made(&[&SCENE, &grid, &[8; 208]], carried, 5, &[2; 144]), "its bytes written");
    assert_ne!(kept, made(&with, carried, 5, &[3; 144]), "the incoming userdata");
    let none = Reads { pixel: false, ..carried };
    assert_ne!(kept, made(&shared, none, 5, &[2; 144]), "no snapshot");
}

#[test]
fn the_walk_records_each_nodes_block_once() {
    let mut recording = Recording::default();
    let world = [1u8; 208];
    let first = recording.place(&world);
    assert_eq!(first, Block { at: 0, length: 208 });
    let palette = [2u8; 128];
    let second = recording.place(&palette);
    assert_eq!(second, Block { at: 208, length: 128 });
    assert_eq!(recording.recorded(first), &world);
    assert_eq!(recording.recorded(second), &palette);
    assert_eq!(recording.bytes.len(), 336, "each block as it stands, the list placing it");
}

#[test]
fn a_walk_bands_where_its_parts_stop_agreeing_and_a_part_with_no_order_stands_alone() {
    let opaque = mesh_shader::Order::for_tests(15, (4, 4));
    let tracks = mesh_shader::Order::for_tests(15, (0, 0));
    let band = |orders: &[Option<mesh_shader::Order>]| {
        let mut bands = Bands::default();
        let held: Vec<u32> = orders.iter().map(|order| bands.next(*order)).collect();
        (held, bands.count())
    };
    assert_eq!(band(&[]), (vec![], 0), "no draw, no band");
    assert_eq!(
        band(&[Some(opaque); 4]),
        (vec![0; 4], 1),
        "parts that agree are one band, whatever their part"
    );
    assert_eq!(
        band(&[Some(opaque), Some(opaque), Some(tracks), Some(opaque), Some(opaque)]),
        (vec![0, 0, 1, 2, 2], 3)
    );
    assert_eq!(
        band(&[Some(opaque), None, None, Some(opaque)]),
        (vec![0, 1, 2, 3], 4)
    );
}

fn key(identity: u32) -> FigureKey {
    (0, 7, identity, 0)
}

fn unit(live: bool, index: usize, vertex: Option<Block>, skin: Option<Block>) -> Unit {
    Unit {
        live,
        view: 0,
        group: 0,
        index,
        row: None,
        descriptor: 0,
        lod: Some(0),
        at: [0.0; 3],
        vertex,
        skin,
        bones: 2,
        mask: 0,
        draws: 0,
        parts: 0,
    }
}

fn still_figure(recording: &mut Recording, position: usize, identity: u32, fill: u8) -> Block {
    recording.begin(0, position, key(identity), false);
    let block = recording.place(&[fill; 208]);
    recording.units.push(unit(false, position, Some(block), None));
    block
}

fn laid_out_made() -> Made {
    let reads = Reads { views: 2, ..Reads::default() };
    Made::new(0, &frame(camera(), &[0., 0.]), &[&SCENE, &SCENE], reads, 1, 0, 0, &[], None)
}

fn kept_block(list: &mut Kept, fill: u8, length: usize) -> Block {
    let at = list.blocks.len();
    list.blocks.extend(std::iter::repeat_n(fill, length));
    Block { at, length }
}

fn listed(nodes: &[(u32, bool, bool, u8)]) -> Kept {
    let mut list = Kept::new(Kind::Casters);
    for (at, &(identity, live, waiting, fill)) in nodes.iter().enumerate() {
        let block = kept_block(&mut list, fill, 208);
        list.units.push(Unit { vertex: Some(block), ..unit(live, at, None, None) });
        list.figures.push(Figure {
            view: 0,
            position: at,
            key: key(identity),
            live,
            waiting,
            units: at..at + 1,
            draws: 0..0,
        });
    }
    list.made = Some(laid_out_made());
    list.read(5);
    list.gathered = vec![0];
    list.index();
    list
}

#[test]
fn a_figure_without_units_is_held_at_its_place() {
    let mut list = listed(&[(1, false, false, 3), (2, false, false, 4), (3, false, false, 5)]);
    list.units.remove(1);
    list.figures[1].units = 1..1;
    list.figures[2].units = 1..2;
    list.index();
    assert!(list.holds(0, 0) && list.holds(0, 1) && list.holds(0, 2));
    assert_eq!(list.units_of(1).count(), 0, "its instance has no units");
    assert!(!list.holds(0, 3) && !list.holds(1, 1));
}

fn rows_list(groups: &[(usize, bool, bool, &[(Supply, usize)])]) -> Kept {
    let mut list = Kept::new(Kind::Lit);
    list.bytes = vec![0; 2 << 20];
    list.live_area = Area::new(1024, 1024, 1 << 20);
    list.still_area = Area::new(1 << 20, 1 << 20, 2 << 20);
    for &(key, live, alone, runs) in groups {
        list.make_group(key, 0, live, 0, alone, true, runs, ALIGNMENT)
            .expect("the areas hold these runs");
    }
    list
}

fn held(list: &Kept, at: usize) -> (Option<usize>, Vec<Run>, usize) {
    let group = list.groups[at].as_ref().expect("a group of the list");
    (group.rows, group.runs.clone(), group.slots.len())
}

#[test]
fn a_groups_runs_stand_at_a_multiple_of_their_stride_with_room_for_what_joins_it() {
    let runs = [(Supply::Vertex, 208), (Supply::Skin, 128)];
    let list = rows_list(&[(3, false, false, &runs)]);
    let (rows, placed, slots) = held(&list, 0);
    assert_eq!(slots, 0, "no instance yet");
    assert_eq!(rows.expect("a rows block") % ALIGNMENT, 0, "a uniform binds at the alignment");
    assert!(
        placed.iter().all(|run| run.at.is_multiple_of(run.stride)),
        "the instance index reaches every row of a run from its base"
    );
    assert_eq!(placed.iter().map(|run| run.stride).collect::<Vec<_>>(), vec![208, 128]);
    assert_eq!(list.groups[0].as_ref().expect("made").capacity, 5, "room for four more");
    let alone = rows_list(&[(3, false, true, &runs)]);
    assert_eq!(alone.groups[0].as_ref().expect("made").capacity, 1);
}

#[test]
fn an_instance_takes_the_next_slot_and_a_full_group_grows_where_its_rows_fit() {
    let mut list = rows_list(&[(3, false, false, &[(Supply::Vertex, 208)])]);
    let first = held(&list, 0).1[0].at;
    for slot in 0..5 {
        assert_eq!(list.take_slot(0), Some((slot, false)), "room for five");
    }
    for slot in 0..5u32 {
        let row = first + slot as usize * 208;
        list.bytes[row..row + 4].copy_from_slice(&slot.to_le_bytes());
    }
    assert_eq!(list.take_slot(0), Some((5, true)), "the sixth grew the run");
    let (_, placed, slots) = held(&list, 0);
    assert_eq!(slots, 6);
    assert!(placed[0].at.is_multiple_of(208));
    assert_ne!(placed[0].at, first, "a longer run elsewhere");
    for slot in 0..5u32 {
        let row = placed[0].at + slot as usize * 208;
        assert_eq!(&list.bytes[row..row + 4], &slot.to_le_bytes(), "its rows moved with it");
    }
}

#[test]
fn a_group_with_no_room_for_its_runs_leaves_none_taken() {
    let mut list = rows_list(&[]);
    let free = list.still_area.clone();
    let long = [(Supply::Vertex, 1 << 22)];
    assert!(list.make_group(1, 0, false, 0, false, true, &long, ALIGNMENT).is_none());
    assert_eq!(list.still_area, free, "the area is as it was");
    let mut list = rows_list(&[(2, false, false, &[(Supply::Vertex, 1 << 17)])]);
    while list.take_slot(0).is_some() {}
    let slots = held(&list, 0).2;
    assert!(slots >= 5, "five rows were placed: {slots}");
    assert_eq!(list.take_slot(0), None, "full");
    assert_eq!(held(&list, 0).2, slots);
}

fn joined(list: &mut Kept, at: usize) -> usize {
    let (slot, _) = list.take_slot(at).expect("room");
    let instance = list.instances.len();
    list.groups[at].as_mut().expect("made").slots[slot] = instance;
    list.instances.push(Instance { group: at, slot });
    slot
}

#[test]
fn an_instance_that_leaves_gives_its_slot_back_and_the_rows_above_it_move_down() {
    let mut list = rows_list(&[(3, false, false, &[(Supply::Vertex, 208)])]);
    let at = held(&list, 0).1[0].at;
    for instance in 0..4u8 {
        let slot = joined(&mut list, 0);
        list.bytes[at + slot * 208] = 10 + instance;
    }
    list.dirty.clear();
    list.release(&[1]);
    assert_eq!(held(&list, 0).2, 3, "three instances left");
    assert_eq!(
        list.instances.iter().map(|instance| instance.slot).collect::<Vec<_>>(),
        vec![0, 1, 1, 2],
        "the instances above the hole the second left moved down, in order"
    );
    let rows: Vec<u8> = (0..3).map(|slot| list.bytes[at + slot * 208]).collect();
    assert_eq!(rows, vec![10, 12, 13], "their rows moved with them");
    assert_eq!(list.dirty_ranges(), vec![at + 208..at + 624], "the still rows it wrote");
    list.release(&[0, 2, 3]);
    assert!(list.groups[0].is_none(), "the group left");
    assert_eq!(list.still_area, Area::new(1 << 20, 1 << 20, 2 << 20), "its area is as it was");
}

#[test]
fn a_node_writes_its_world_and_userdata_into_its_block_and_the_rows_its_draws_read() {
    let runs = [(Supply::Vertex, 96), (Supply::Skin, 128)];
    let mut list = rows_list(&[(3, false, false, &runs)]);
    let (_, placed, _) = held(&list, 0);
    joined(&mut list, 0);
    joined(&mut list, 0);
    let vertex = kept_block(&mut list, 0, 208);
    let skin = kept_block(&mut list, 0, 128);
    list.units.push(Unit {
        vertex: Some(vertex),
        skin: Some(skin),
        draws: 1,
        parts: 1,
        ..unit(false, 0, None, None)
    });
    let (world, userdata, bones) = ([[1.0f32; 4]; 4], [2.0f32; 8], [9u8; 128]);
    let wrote = LiveBytes { world: &world, userdata: Some(userdata), bones: Some(&bones) };
    assert!(list.write_unit(0, wrote));
    let (row, palette) = (placed[0].at + 96, placed[1].at + 128);
    assert_eq!(&list.bytes[row..row + 64], bytemuck::bytes_of(&world), "the second slot's row");
    assert_eq!(&list.bytes[row + 64..row + 96], bytemuck::bytes_of(&userdata));
    assert_eq!(&list.bytes[palette..palette + 128], &bones);
    assert_eq!(&list.vertex_bytes(0).expect("its block")[..64], bytemuck::bytes_of(&world));
    assert_eq!(&list.vertex_bytes(0).expect("its block")[96..], &[0; 112], "the rest as it was");
    assert_eq!(list.skin_bytes(0).expect("its palette"), &bones);
    assert_eq!(
        list.dirty_ranges(),
        vec![row..palette + 128],
        "the still rows it wrote, joined: they stand near each other"
    );
    let moved = LiveBytes { world: &world, userdata: None, bones: Some(&[0; 64]) };
    assert!(!list.write_unit(0, moved));
    let none = LiveBytes { world: &world, userdata: None, bones: None };
    assert!(!list.write_unit(0, none));
}

#[test]
fn a_kept_frame_stages_the_rows_and_shared_blocks_whose_bytes_changed() {
    let mut list = rows_list(&[(3, true, false, &[(Supply::Vertex, 208)])]);
    list.shared = Shared { at: vec![0, 512], reserved: vec![416, 416], reads: Reads { views: 2, ..Reads::default() } };
    let at = held(&list, 0).1[0].at;
    joined(&mut list, 0);
    let vertex = kept_block(&mut list, 0, 208);
    list.units.push(Unit { vertex: Some(vertex), parts: 1, ..unit(true, 0, None, None) });
    let world = [[0.5f32; 4]; 4];
    assert!(list.write_unit(0, LiveBytes { world: &world, userdata: None, bones: None }));
    assert_eq!(list.dirty_ranges(), vec![at..at + 64], "the live row whose bytes changed");
    assert_eq!(&list.bytes[at..at + 64], bytemuck::bytes_of(&world));
    assert!(list.write_unit(0, LiveBytes { world: &world, userdata: None, bones: None }));
    assert!(list.dirty_ranges().is_empty(), "the same pose again stages nothing");
    let moved = [[0.75f32; 4]; 4];
    assert!(list.write_unit(0, LiveBytes { world: &moved, userdata: None, bones: None }));
    assert_eq!(list.dirty_ranges(), vec![at..at + 64], "a row whose node moved");
    list.write_shared(&[&[1; 416], &[2; 416]]);
    assert_eq!(list.dirty_ranges(), vec![0..928], "both blocks");
    list.write_shared(&[&[1; 416], &[2; 416]]);
    assert!(list.dirty_ranges().is_empty(), "the blocks stand");
    list.write_shared(&[&[1; 416], &[3; 416]]);
    assert_eq!(list.dirty_ranges(), vec![512..928], "the one that changed");
    assert_eq!(&list.bytes[..416], &[1; 416]);
    assert_eq!(&list.bytes[512..928], &[3; 416]);
}

#[test]
fn the_judge_of_the_staged_bytes_names_a_row_the_list_did_not_stage() {
    let mut list = rows_list(&[(3, true, false, &[(Supply::Vertex, 208)])]);
    let at = held(&list, 0).1[0].at;
    list.resident = list.bytes.clone();
    assert_eq!(list.staged_difference(), None, "the GPU holds a whole write's bytes");
    list.bytes[at + 8] = 9;
    assert_eq!(list.staged_difference(), Some(at + 8), "a row written and not staged");
    list.resident[at + 8] = 9;
    assert_eq!(list.staged_difference(), None, "staged");
    let past = list.live_area.top + 4;
    list.bytes[past] = 7;
    assert_eq!(list.staged_difference(), None, "nothing past the rows its draws read");
    list.resident = Vec::new();
    list.bytes[at + 8] = 1;
    assert_eq!(list.staged_difference(), None, "no judge keeps no bytes to compare");
}

#[test]
fn near_writes_join_and_far_ones_stay_apart() {
    let mut casters = Kept::default();
    casters.dirty = vec![(5000, 64), (256, 64), (512, 128)];
    assert_eq!(casters.dirty_ranges(), vec![256..640, 5000..5064]);
}

#[test]
fn a_forgotten_list_or_one_that_missed_a_change_of_the_field_is_not_fresh() {
    let mut casters = listed(&[(1, false, false, 3)]);
    let made = laid_out_made();
    assert!(casters.fresh(&made, Some(5)));
    assert!(!casters.fresh(&made, Some(6)), "the field was read in a frame the list was not");
    assert!(!casters.fresh(&made, None));
    assert_eq!(casters.stale(&made, Some(6)), Some(Stale::Whole(Whole::NotFollowed)), "named");
    let other = Made { alpha: made.alpha ^ 1, chain: vec![1], ..made.clone() };
    assert_eq!(casters.stale(&other, Some(5)), Some(Stale::Input(4)), "the first input that differs: alpha");
    casters.forget();
    assert!(!casters.fresh(&made, Some(5)));
    assert_eq!(casters.stale(&made, Some(5)), Some(Stale::Whole(Whole::First)));
    casters.drop_for(Whole::NoRoom);
    assert_eq!(casters.stale(&made, Some(5)), Some(Stale::Whole(Whole::NoRoom)), "what dropped it");
}

#[test]
fn a_walk_of_some_figures_takes_their_routes_alone_and_marks_each() {
    let mut recording = Recording::of(vec![(0, 2), (1, 0)]);
    let taken: Vec<_> = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1)]
        .into_iter()
        .filter(|&(view, position)| recording.takes(view, position))
        .collect();
    assert_eq!(taken, vec![(0, 2), (1, 0)]);
    let mut all = Recording::default();
    assert!(all.takes(0, 0) && all.takes(3, 9), "a full walk takes every route");

    let mut recording = Recording::default();
    still_figure(&mut recording, 0, 1, 1);
    recording.begin(0, 1, key(2), true);
    recording.place(&[2; 208]);
    recording.begin(0, 2, key(3), false);
    recording.units.push(unit(false, 2, None, None));
    still_figure(&mut recording, 3, 4, 4);
    let walked = recording.walked();
    let runs: Vec<_> = walked
        .iter()
        .map(|walk| (walk.mark.key, walk.units.clone(), walk.mark.live))
        .collect();
    assert_eq!(
        runs,
        vec![
            (key(1), 0..1, false),
            (key(2), 1..1, true),
            (key(3), 1..2, false),
            (key(4), 2..3, false),
        ]
    );
}

#[test]
fn an_area_takes_free_runs_first_and_a_run_given_back_joins_its_neighbours() {
    let mut area = Area::new(1024, 1024, 4096);
    assert_eq!(area.take(512, ALIGNMENT), Some(1024));
    assert_eq!(area.take(256, ALIGNMENT), Some(1536));
    assert_eq!(area.take(512, ALIGNMENT), Some(1792));
    assert_eq!(area.take(2048, ALIGNMENT), None, "full");
    area.give(1024..1536);
    assert_eq!(area.take(256, ALIGNMENT), Some(1024), "a free run before the top first");
    assert_eq!(area.free, vec![1280..1536]);
    area.give(1024..1280);
    assert_eq!(area.free, vec![1024..1536], "neighbours join");
    area.give(1792..2304);
    assert_eq!(area.top, 1792, "the run at the top lowers it");
    area.give(1536..1792);
    assert_eq!((area.top, area.free.len()), (1024, 0), "all given back");
    assert_eq!(area.take(3072, ALIGNMENT), Some(1024));
}

#[test]
fn a_run_of_one_stride_starts_at_a_multiple_of_it() {
    let mut area = Area::new(0, 0, 4096);
    assert_eq!(area.take(208, 208), Some(0));
    assert_eq!(area.take(416, 208), Some(208), "the rows after it");
    area.give(0..208);
    assert_eq!(area.take(104, 104), Some(0), "the free run holds these rows");
    assert_eq!(area.take(104, 208), Some(624), "no row of 208 fits below: above the top");
    assert_eq!(area.free, vec![104..208], "what the free run has left");
}

fn target(position: usize, identity: u32, born: bool) -> Target {
    Target {
        view: 0,
        position,
        group: 0,
        index: position,
        row: None,
        key: key(identity),
        born,
    }
}

#[test]
fn a_figure_placed_once_in_both_frames_is_kept_and_every_other_is_walked() {
    let casters = listed(&[(1, false, false, 3), (2, true, false, 4), (3, false, false, 6)]);
    let entered = |targets: &[Target], demoted: &[bool]| {
        let mut entered = [0u64; ENTERED];
        let next = casters.plan_counting(targets, demoted, Some(&mut entered));
        assert_eq!(next, casters.plan(targets, demoted), "counting plans the same");
        assert_eq!(entered.iter().sum::<u64>() as usize, next.iter().filter(|figure| figure.is_none()).count());
        entered
    };
    let only = |why: Entered, n: u64| {
        let mut entered = [0u64; ENTERED];
        entered[why as usize] = n;
        entered
    };
    let targets = [target(0, 3, false), target(1, 1, false), target(2, 4, false)];
    assert_eq!(casters.plan(&targets, &[]), vec![Some(2), Some(0), None]);
    assert_eq!(entered(&targets, &[]), only(Entered::New, 1));
    let born = [target(0, 1, true), target(1, 2, true)];
    assert_eq!(casters.plan(&born, &[]), vec![None, Some(1)]);
    assert_eq!(entered(&born, &[]), only(Entered::Born, 1));
    let all = [target(0, 1, false), target(1, 2, false), target(2, 3, false)];
    assert_eq!(casters.plan(&all, &[false, true, false]), vec![Some(0), None, Some(2)]);
    assert_eq!(entered(&all, &[false, true, false]), only(Entered::Demoted, 1));
    let twice = [target(0, 1, false), target(1, 1, false), target(2, 3, false)];
    assert_eq!(casters.plan(&twice, &[]), vec![None, None, Some(2)]);
    assert_eq!(entered(&twice, &[]), only(Entered::Twice, 2));
    let mut other_view = target(0, 1, false);
    other_view.view = 1;
    assert_eq!(casters.plan(&[other_view], &[]), vec![None], "another view is another figure");
    assert_eq!(entered(&[other_view], &[]), only(Entered::OtherView, 1));
    let mut mesh = target(0, 1, false);
    mesh.key.0 = 9;
    let mut epoch = target(1, 2, false);
    epoch.key.3 = 9;
    let mut keys = only(Entered::KeyMesh, 1);
    keys[Entered::KeyEpoch as usize] = 1;
    assert_eq!(entered(&[mesh, epoch], &[]), keys);
}

#[test]
fn a_player_made_for_an_actor_lists_it_and_raises_nothing() {
    use entity_animation::{named_stage, stage_rig};
    let mut pass = ModelPass::with_rigs_for_tests(vec![Arc::new(stage_rig(true))], "general");
    let figure = |seed: u32| ModelInstance {
        animation: named_stage("cruise", None),
        animation_seed: seed,
        scale: 1.,
        ..Default::default()
    };
    let mut born = Vec::new();
    pass.update_placed(vec![(0, figure(7))], 128.);
    let revision = pass.revision;
    pass.animate(10., None, &[0.]);
    pass.take_born(&mut born);
    assert_eq!(born, vec![(0, 7, 0)]);
    assert_eq!(pass.revision, revision, "the figure given a player is walked alone");
    pass.animate(10.5, None, &[0.]);
    pass.take_born(&mut born);
    assert!(born.is_empty(), "a player made once");
    pass.update_placed(vec![(0, figure(7)), (0, figure(8))], 128.);
    pass.animate(11., None, &[0.]);
    pass.take_born(&mut born);
    assert_eq!(born, vec![(0, 8, 0)]);
    assert_eq!(pass.revision, revision);
}

#[test]
fn a_library_that_only_grew_raises_nothing_and_another_raises() {
    use entity_animation::stage_rig;
    let mut pass = ModelPass::with_rigs_for_tests(vec![Arc::new(stage_rig(true))], "general");
    let mut library = ModelLibrary::default();
    library.meshes = pass.meshes.clone();
    library.rigs = pass.rigs.clone();
    library.palette = pass.default_palette.clone();
    let revision = pass.revision;
    pass.sync(&library);
    assert_eq!(pass.revision, revision, "the same library");
    library.meshes.push(library.meshes[0].clone());
    library.rigs.push(Arc::new(stage_rig(false)));
    library.palette.push(PaletteJoint::IDENTITY);
    pass.sync(&library);
    assert_eq!(pass.revision, revision, "a library that grew: a new entity's figures join");
    library.palette[0] = PaletteJoint::from_transform(&mesh::Transform::from_trs(
        [1., 0., 0.],
        [0., 0., 0., 1.],
        1.,
    ));
    pass.sync(&library);
    assert_eq!(pass.revision, revision + 1, "a palette that changed");
}

fn field(positions: &[[f32; 3]]) -> ModelField {
    let instances: Vec<_> = positions
        .iter()
        .map(|&position| ModelInstance {
            position,
            scale: 1.,
            animation_seed: 7,
            ..Default::default()
        })
        .collect();
    ModelField {
        groups: vec![ModelGroup {
            mesh: 0,
            asset: AssetKey::synthetic("synthetic"),
            range: 0..instances.len() as u32,
            bounds: [0., 0., 10., 10.],
            centroid: [0.; 3],
        }],
        native_order: (0..instances.len()).map(|index| (0, index)).collect(),
        instances,
        ..Default::default()
    }
}

#[test]
fn placements_made_again_unchanged_raise_nothing_and_a_change_is_listed_by_instance() {
    let mut walked = WalkedField::default();
    let mut changed = Changed::default();
    let mut placed = field(&[[1., 0., 1.], [2., 0., 2.], [3., 0., 3.]]);
    assert_eq!(walked.note(&placed, &mut changed), Change::Whole, "the first field");
    assert_eq!(walked.note(&placed, &mut changed), Change::Same);
    placed.instances[1].palette_offset = 40;
    assert_eq!(
        walked.note(&placed, &mut changed),
        Change::Same,
        "animation's palette offset is no input of a caster"
    );
    placed.instances[2].position[0] += 0.25;
    placed.groups[0].bounds[2] = 10.25;
    assert_eq!(walked.note(&placed, &mut changed), Change::Changed);
    assert_eq!(
        (changed.instances.as_slice(), changed.groups.as_slice()),
        (&[2][..], &[0][..])
    );
    changed.clear();
    assert_eq!(walked.note(&placed, &mut changed), Change::Same, "a change is listed once");
    assert!(changed.instances.is_empty() && changed.groups.is_empty());

    placed.instances[0].animation = entity_animation::STOPPED + 1;
    placed.instances[0].native_flags = 3;
    assert_eq!(walked.note(&placed, &mut changed), Change::Changed);
    assert_eq!(changed.instances, vec![0]);
}

#[test]
fn another_actor_count_mesh_or_order_is_a_whole_change() {
    let mut walked = WalkedField::default();
    let mut changed = Changed::default();
    let mut placed = field(&[[1., 0., 1.], [2., 0., 2.], [3., 0., 3.]]);
    walked.note(&placed, &mut changed);
    placed.instances[1].animation_identity = 9;
    assert_eq!(walked.note(&placed, &mut changed), Change::Whole, "another actor");
    let mut grown = field(&[[1., 0., 1.], [2., 0., 2.], [3., 0., 3.], [4., 0., 4.]]);
    assert_eq!(walked.note(&grown, &mut changed), Change::Whole, "another count");
    grown.groups[0].mesh = 1;
    assert_eq!(walked.note(&grown, &mut changed), Change::Whole, "another mesh");
    grown.native_order.reverse();
    assert_eq!(walked.note(&grown, &mut changed), Change::Whole, "another order");
}

fn actors(identities: &[u32]) -> ModelField {
    let mut placed = field(&vec![[0.; 3]; identities.len()]);
    for (instance, &identity) in placed.instances.iter_mut().zip(identities) {
        instance.animation_identity = identity;
    }
    placed
}

#[test]
fn a_whole_change_counts_what_joined_left_and_left_its_order() {
    let mut walked = WalkedField::default();
    let mut changed = Changed::default();
    walked.note(&actors(&[1, 2, 3, 4, 5]), &mut changed);
    assert_eq!(walked.note(&actors(&[1, 5, 3, 4, 6]), &mut changed), Change::Whole);
    assert_eq!(walked.reshaped, [1, 1, 1]);
    assert_eq!(walked.note(&actors(&[1, 5, 3, 4, 6]), &mut changed), Change::Same);
}

#[test]
fn a_field_placed_twice_in_a_frame_is_compared_once_as_drawn() {
    let mut pass = ModelPass::with_rigs_for_tests(Vec::new(), "general");
    let mut changed = Changed::default();
    let placed = || -> Vec<(usize, ModelInstance)> {
        (0..3)
            .map(|at| {
                let instance = ModelInstance {
                    position: [at as f32, 0., 0.],
                    scale: 1.,
                    animation_identity: at + 1,
                    ..Default::default()
                };
                (0, instance)
            })
            .collect()
    };
    pass.update_placed(placed(), 128.);
    assert_eq!(pass.field_change(&mut changed), Change::Whole, "the first placement");
    assert_eq!(pass.field_change(&mut changed), Change::Same, "nothing placed since");
    let mut handed = placed();
    handed.reverse();
    pass.update_placed(handed, 128.);
    pass.update_placed(placed(), 128.);
    assert_eq!(pass.field_change(&mut changed), Change::Same);
    let mut moved = placed();
    moved[1].1.position[2] = 0.5;
    pass.update_placed(moved, 128.);
    assert_eq!(pass.field_change(&mut changed), Change::Changed);
    assert_eq!(changed.instances.len(), 1);
}

#[test]
fn a_whole_change_lists_the_actors_in_both_fields_whose_values_changed() {
    let mut walked = WalkedField::default();
    let mut changed = Changed::default();
    walked.note(&actors(&[1, 2, 3, 4]), &mut changed);
    let mut next = actors(&[3, 1, 5, 4]);
    next.instances[0].position[0] = 2.;
    next.instances[3].animation = entity_animation::STOPPED + 1;
    assert_eq!(walked.note(&next, &mut changed), Change::Whole);
    assert_eq!(changed.instances, vec![0, 3], "actors 3 and 4, at their new places");
    assert!(changed.groups.is_empty());

    let mut walked = WalkedField::default();
    walked.note(&actors(&[1, 2, 3]), &mut changed);
    let mut next = actors(&[1, 2, 3]);
    next.instances[0].position[2] = 1.;
    next.instances[1].animation_identity = 9;
    assert_eq!(walked.note(&next, &mut changed), Change::Whole);
    assert_eq!(changed.instances, vec![0]);
}

fn lit(reads: Reads) -> Kept {
    let mut list = Kept::new(Kind::Lit);
    let a = kept_block(&mut list, 1, 208);
    list.units.push(Unit { mask: 0b10_1111, draws: 0, parts: 1, ..unit(false, 0, Some(a), None) });
    let b = kept_block(&mut list, 2, 208);
    let bones = kept_block(&mut list, 6, 128);
    list.units.push(Unit {
        mask: 0b11_1111,
        draws: 1,
        parts: 2,
        ..unit(false, 1, Some(b), Some(bones))
    });
    let c = kept_block(&mut list, 3, 208);
    list.units.push(Unit { draws: 3, ..unit(false, 1, Some(c), None) });
    for (key, material) in [(3, 5), (4, 6), (5, 7)] {
        list.metas.push(Meta { key, textures: 1, material });
    }
    list.shared = Shared { at: Vec::new(), reserved: vec![416, 16], reads };
    list.index();
    list
}

#[test]
fn a_lit_list_gives_the_constant_state_its_walk_leaves() {
    use crate::native::kept::plans::Source;
    let own = Reads { views: 1, grid: true, pixel: false, own_pixel: true };
    let list = lit(own);
    let replay = list.replay(|key| (u32::from(key == 4), key != 3));
    assert_eq!((replay.drawn, replay.last, replay.material), (3, Some(2), Some(7)));
    assert_eq!(replay.sources.state.part, Some(5), "the last part's program is retained");
    assert_eq!(replay.sources.state.skin, Some(5), "and the last skinned part's palette stream");
    let named = |source| -> &'static [u8] {
        match source {
            Source::Frame(0) => b"frame",
            Source::Grid => b"grid",
            Source::Object(0) => b"first",
            Source::Object(1) => b"second",
            Source::Object(SHARED_PIXEL) => b"incoming",
            Source::Skin(1, 0) => b"palette",
            _ => b"other",
        }
    };
    let binds: Vec<_> = replay.sources.resolve_with(named).into_iter().take(6).collect();
    let expected: [&[u8]; 6] = [b"frame", b"frame", b"second", b"second", b"palette", b"grid"];
    assert_eq!(binds, expected.map(Some), "each slot from the last node that supplied it");
    let carried = lit(Reads { pixel: true, own_pixel: false, ..own });
    let replay = carried.replay(|key| (0, key != 3));
    let pixel = replay.sources.resolve_with(named)[3];
    assert_eq!(pixel, Some(&b"incoming"[..]));
    assert_eq!(carried.vertex_bytes(2), Some(&[3u8; 208][..]), "the next pass's start");
}

#[test]
fn the_field_s_change_is_made_once_a_frame_and_names_the_frame_read_before() {
    let mut pass = ModelPass::with_rigs_for_tests(Vec::new(), "general");
    let figure = |x: f32| {
        let instance = ModelInstance { position: [x, 0., 0.], scale: 1., animation_identity: 1, ..Default::default() };
        vec![(0, instance)]
    };
    pass.update_placed(figure(0.), 128.);
    {
        let change = pass.frame_change(1);
        assert_eq!((change.change, change.since), (Change::Whole, None));
    }
    pass.update_placed(figure(0.5), 128.);
    assert_eq!(pass.frame_change(1).change, Change::Whole, "every list of a frame sees one change");
    {
        let change = pass.frame_change(2);
        assert_eq!((change.change, change.since), (Change::Changed, Some(1)));
        assert_eq!(change.changed.instances, vec![0]);
    }
    let change = pass.frame_change(3);
    assert_eq!((change.change, change.since), (Change::Same, Some(2)));
}

#[test]
fn a_figure_that_waited_on_a_build_is_walked_again_alone_and_the_list_is_kept() {
    let mut recording = Recording::default();
    still_figure(&mut recording, 0, 1, 3);
    still_figure(&mut recording, 1, 2, 4);
    recording.wait();
    still_figure(&mut recording, 2, 3, 5);
    assert!(!recording.incomplete(), "the wait is its figure's");
    let list = listed(&[(1, false, false, 3), (2, false, true, 4), (3, false, false, 5)]);
    let made = laid_out_made();
    assert!(list.fresh(&made, Some(5)), "kept, though one figure waited");
    let waiting = list.waiting().expect("one figure waited");
    assert_eq!(waiting, vec![false, true, false]);
    let all = [target(0, 1, false), target(1, 2, false), target(2, 3, false)];
    assert_eq!(list.plan(&all, &waiting), vec![Some(0), None, Some(2)], "walked again alone");

    let mut outside = Recording::default();
    outside.wait();
    assert!(outside.incomplete(), "a wait outside any figure keeps no list");
}

#[test]
fn part_counts_give_what_instancing_by_run_and_by_part_leaves() {
    let mut list = Kept::new(Kind::Lit);
    for (position, view) in [(0, 0), (1, 0), (2, 1)] {
        let parts = if view == 0 { 2 } else { 1 };
        let draws = list.metas.len();
        list.units.push(Unit { view, draws, parts, ..unit(false, position, None, None) });
        for key in 3..3 + parts {
            list.metas.push(Meta { key, textures: 1, material: 5 });
        }
    }
    let counts = list.part_counts();
    assert_eq!(counts, PartCounts { parts: 2, runs: 5, in_views: 3 }, "3,4,3,4 then 3 in view 1");
}

#[test]
fn with_the_camera_still_a_unit_walking_out_or_in_crosses_the_cull_on_the_frame_it_crosses() {
    use crate::GameCull;
    let wide = 1.0e4;
    let cull = GameCull {
        eye: [0., 100., 0.],
        sides: [[1., 0., 0., wide], [-1., 0., 0., wide], [0., 0., 1., wide], [0., 0., -1., wide]],
        x: 0.,
        pad: 0.,
    };
    let distance = content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED;
    let mut list = Kept::default();
    let mut culled = Vec::new();
    for frame in 0..6 {
        let out = [400. + 10. * frame as f32, 0., 0.];
        let back = [0., 0., 450. - 10. * frame as f32];
        let now = vec![cull.test(out, 1., distance).is_some(), cull.test(back, 1., distance).is_some()];
        culled.push(now);
    }
    assert_eq!(culled.iter().map(|now| now[0]).collect::<Vec<_>>(), [false, false, true, true, true, true], "walking out");
    assert_eq!(culled.iter().map(|now| now[1]).collect::<Vec<_>>(), [true, true, true, true, false, false], "walking in");

    let key = (0, 1, 2, 3);
    list.set_culled(vec![(0, key)]);
    assert!(list.was_culled(0, key));
    assert!(!list.was_culled(1, key), "culled in one view only");
    list.set_culled(Vec::new());
    assert!(!list.was_culled(0, key), "the next splice took it");
}
