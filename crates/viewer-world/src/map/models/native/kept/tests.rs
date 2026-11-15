use super::*;
use viewer_foundation::map::camera::{MapDefines, MapSize, Pose};

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
    // The scene blocks' bytes change every frame (they carry the animation clock): kept.
    assert!(same(9, &frame(camera(), &offsets), &[&[1; 416], &[2; 416]], 1, 3));

    assert!(!same(10, &frame(camera(), &offsets), &[&SCENE, &SCENE], 1, 3), "revision");
    let mut moved = camera();
    moved.center[0] += 0.5;
    assert!(!same(9, &frame(moved, &offsets), &[&SCENE, &SCENE], 1, 3), "camera");
    let mut lower = camera();
    lower.height -= 1.;
    assert!(!same(9, &frame(lower, &offsets), &[&SCENE, &SCENE], 1, 3), "height");
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
    // The viewport widened with the camera still: the cull's side planes moved, and a caster list
    // walked with no rectangle (the map frame's) would otherwise keep the casters the old ones cut.
    let narrow = crate::map::models::GameCull::of(&camera(), [800, 600]);
    let wide = crate::map::models::GameCull::of(&camera(), [1600, 600]);
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
    // A pass whose nodes supply no userdata reads the incoming one and the snapshot.
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
fn blocks_are_placed_as_the_arena_places_them() {
    let mut recording = Recording::default();
    let world = [1u8; 208];
    let first = recording.place(false, &world, 160, ALIGNMENT);
    assert_eq!(first, Block { live: false, at: 0, length: 208 });
    let palette = [2u8; 128];
    let second = recording.place(false, &palette, 512, ALIGNMENT);
    assert_eq!(second.at, 256, "the next block starts at the alignment");
    let region = &recording.regions[1];
    assert_eq!(region.len(), 256 + 512, "a block is zero-filled to the size its program reads");
    assert_eq!(&region[..208], &world);
    assert!(region[208..256].iter().all(|&byte| byte == 0));
    assert_eq!(&region[256..384], &palette);
    assert!(region[384..].iter().all(|&byte| byte == 0));
    let live = recording.place(true, &world, 0, ALIGNMENT);
    let own = Block { live: true, at: 0, length: 208 };
    assert_eq!(live, own, "live blocks have a region of their own");
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
        parts: 0,
    }
}

/// One still figure's walk at `position`: a world block of `fill`.
fn still_figure(recording: &mut Recording, position: usize, identity: u32, fill: u8) -> Block {
    recording.begin(0, position, key(identity), false);
    let block = recording.place(false, &[fill; 208], 0, ALIGNMENT);
    recording.units.push(unit(false, position, Some(block), None));
    block
}

/// A walk of three figures in one view: actor 1 still, actor 2 with a player, actor 3 still.
fn laid_out() -> (Kept, Block, Block, Block) {
    let mut recording = Recording::default();
    let still = still_figure(&mut recording, 0, 1, 3);
    recording.begin(0, 1, key(2), true);
    let world = recording.place(true, &[4; 208], 0, ALIGNMENT);
    let bones = recording.place(true, &[5; 128], 0, ALIGNMENT);
    recording.units.push(unit(true, 1, Some(world), Some(bones)));
    let other = still_figure(&mut recording, 2, 3, 6);
    recording.visited = vec![true];
    let layout = Layout::new(Vec::new(), &mut recording, &[&SCENE, &SCENE], &[512, 416], ALIGNMENT);
    let mut casters = Kept::default();
    let made = laid_out_made();
    let still = layout.absolute(still);
    let other = layout.absolute(other);
    let reserved = vec![512, 416];
    let shared = Shared { at: Vec::new(), reserved, reads: Reads { views: 2, ..Reads::default() } };
    casters.take(layout, recording, DrawList::default(), shared, ALIGNMENT, Some(made));
    casters.read(5);
    let world = casters.units[1].vertex.unwrap();
    (casters, still, other, world)
}

fn laid_out_made() -> Made {
    let reads = Reads { views: 2, ..Reads::default() };
    Made::new(0, &frame(camera(), &[0., 0.]), &[&SCENE, &SCENE], reads, 1, 0, 0, &[], None)
}

/// Where the still area of [`laid_out`] begins: after the scene blocks (1024 bytes) and the live
/// area, which keeps room for live figures to join.
fn still_start() -> usize {
    1024 + capacity(384, ALIGNMENT)
}

#[test]
fn the_layout_puts_scene_and_live_blocks_before_the_still_ones_each_with_room() {
    let (casters, still, other, world) = laid_out();
    assert_eq!(casters.shared.at, vec![0, 512]);
    assert_eq!(world.at, 1024, "live blocks follow the scene blocks");
    assert_eq!(casters.units[1].skin.unwrap().at, 1280);
    assert_eq!(casters.live_area.top, 1536, "a kept frame writes the scene and live blocks only");
    assert!(casters.live_area.end >= 1536 + (64 << 10), "room for live figures to join");
    assert_eq!((still.at, other.at), (still_start(), still_start() + 256), "still blocks after");
    assert_eq!(casters.units[0].vertex, Some(still), "units hold their blocks' final places");
    assert_eq!(&casters.bytes[still.at..still.at + 208], &[3; 208]);
    assert_eq!(&casters.bytes[other.at..other.at + 208], &[6; 208]);
    assert_eq!(casters.bytes.len() % ALIGNMENT, 0);
    assert_eq!(casters.live(), &[1]);
    assert_eq!(casters.units_of(2).collect::<Vec<_>>(), vec![2]);
    assert_eq!(casters.units_of(5).count(), 0);
}

#[test]
fn a_full_walk_gives_each_figure_its_units_and_a_span_of_its_area() {
    let (casters, still, other, world) = laid_out();
    let spans: Vec<_> = casters.figures.iter().map(|figure| figure.span.clone()).collect();
    assert_eq!(
        spans,
        vec![still.at..still.at + 256, world.at..world.at + 512, other.at..other.at + 256]
    );
    let units: Vec<_> = casters.figures.iter().map(|figure| figure.units.clone()).collect();
    assert_eq!(units, vec![0..1, 1..2, 2..3]);
    let keys: Vec<_> = casters.figures.iter().map(|figure| (figure.key, figure.live)).collect();
    assert_eq!(keys, vec![(key(1), false), (key(2), true), (key(3), false)]);
    assert_eq!((casters.figure_of(0), casters.figure_of(1), casters.figure_of(2)), (0, 1, 2));
}

#[test]
fn a_kept_frame_writes_its_scene_and_live_nodes_and_no_still_block() {
    let (mut casters, still, _, world) = laid_out();
    let before = casters.bytes.clone();
    let pose: [[f32; 4]; 4] = [[0.5; 4]; 4];
    let bones = [9u8; 128];
    assert!(casters.write_unit(
        1,
        LiveBytes {
            world: &pose,
            userdata: None,
            bones: Some(&bones)
        }
    ));
    assert!(casters.dirty_ranges().is_empty(), "a live node is in the bytes every frame writes");
    let written = casters.kept_bytes(&[&[1; 416], &[2; 416]]).len();
    assert_eq!(written, 1536, "nothing past the live blocks reaches the GPU");
    let counts = casters.counts();
    assert_eq!((counts.walks, counts.kept), (1, 1), "one walk, then one frame kept with no walk");
    assert_eq!(&casters.bytes[..416], &[1; 416]);
    assert_eq!(&casters.bytes[512..928], &[2; 416]);
    assert_eq!(&casters.bytes[world.at..world.at + 64], bytemuck::bytes_of(&pose));
    assert_eq!(
        &casters.bytes[world.at + 64..world.at + 208],
        &before[world.at + 64..world.at + 208],
        "a caster's world block changes in its matrix only"
    );
    let skin = casters.units[1].skin.unwrap();
    assert_eq!(&casters.bytes[skin.at..skin.at + 128], &bones);
    assert_eq!(&casters.bytes[still.at..], &before[still.at..], "still blocks untouched");
}

#[test]
fn a_moved_instance_writes_its_own_still_block_and_no_other() {
    let (mut casters, still, other, _) = laid_out();
    let before = casters.bytes.clone();
    let pose: [[f32; 4]; 4] = [[2.0; 4]; 4];
    assert!(casters.write_unit(2, LiveBytes { world: &pose, userdata: None, bones: None }));
    assert_eq!(casters.dirty_ranges(), vec![other.at..other.at + 64]);
    assert_eq!(&casters.bytes[other.at..other.at + 64], bytemuck::bytes_of(&pose));
    assert_eq!(&casters.bytes[still.at..other.at], &before[still.at..other.at]);
    assert_eq!(&casters.bytes[other.at + 64..], &before[other.at + 64..]);
    assert!(casters.dirty_ranges().is_empty(), "written once");
}

#[test]
fn near_writes_join_and_far_ones_stay_apart() {
    let mut casters = Kept::default();
    casters.dirty = vec![(5000, 64), (256, 64), (512, 128)];
    assert_eq!(casters.dirty_ranges(), vec![256..640, 5000..5064]);
}

#[test]
fn a_node_whose_palette_changes_length_walks_its_figure_again() {
    let (mut casters, ..) = laid_out();
    let pose = [[0.0; 4]; 4];
    assert!(!casters.write_unit(1, LiveBytes { world: &pose, userdata: None, bones: Some(&[0; 64]) }));
    assert!(!casters.write_unit(1, LiveBytes { world: &pose, userdata: None, bones: None }));
    assert!(!casters.write_unit(0, LiveBytes { world: &pose, userdata: None, bones: Some(&[0; 64]) }));
}

#[test]
fn a_forgotten_list_or_one_that_missed_a_change_of_the_field_is_not_fresh() {
    let (mut casters, ..) = laid_out();
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

    // Two still figures around a live one and one with no block: each is one run of its region.
    let mut recording = Recording::default();
    still_figure(&mut recording, 0, 1, 1);
    recording.begin(0, 1, key(2), true);
    recording.place(true, &[2; 208], 512, ALIGNMENT);
    recording.begin(0, 2, key(3), false);
    recording.units.push(unit(false, 2, None, None));
    still_figure(&mut recording, 3, 4, 4);
    let walked = recording.walked(ALIGNMENT);
    let runs: Vec<_> = walked
        .iter()
        .map(|walk| (walk.mark.key, walk.units.clone(), walk.region.clone()))
        .collect();
    assert_eq!(
        runs,
        vec![
            (key(1), 0..1, 0..208),
            (key(2), 1..1, 0..512),
            (key(3), 1..2, 208..208),
            (key(4), 2..3, 256..464),
        ]
    );
}

#[test]
fn an_area_takes_free_runs_first_and_a_run_given_back_joins_its_neighbours() {
    let mut area = Area::new(1024, 1024, 4096);
    assert_eq!(area.take(512), Some(1024));
    assert_eq!(area.take(256), Some(1536));
    assert_eq!(area.take(512), Some(1792));
    assert_eq!(area.take(2048), None, "full");
    area.give(1024..1536);
    assert_eq!(area.take(256), Some(1024), "a free run before the top first");
    assert_eq!(area.free, vec![1280..1536]);
    area.give(1024..1280);
    assert_eq!(area.free, vec![1024..1536], "neighbours join");
    area.give(1792..2304);
    assert_eq!(area.top, 1792, "the run at the top lowers it");
    area.give(1536..1792);
    assert_eq!((area.top, area.free.len()), (1024, 0), "all given back");
    assert_eq!(area.take(3072), Some(1024));
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
    let (casters, ..) = laid_out();
    // 3 first, 1 kept, 4 joins; 2 left.
    // Each walked target counted once, by why.
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
    // A figure given a player since it was walked still is walked again; one already live, not.
    let born = [target(0, 1, true), target(1, 2, true)];
    assert_eq!(casters.plan(&born, &[]), vec![None, Some(1)]);
    assert_eq!(entered(&born, &[]), only(Entered::Born, 1));
    // A figure whose nodes no longer write in place is walked again.
    let all = [target(0, 1, false), target(1, 2, false), target(2, 3, false)];
    assert_eq!(casters.plan(&all, &[false, true, false]), vec![Some(0), None, Some(2)]);
    assert_eq!(entered(&all, &[false, true, false]), only(Entered::Demoted, 1));
    // An actor placed twice is never matched by guess.
    let twice = [target(0, 1, false), target(1, 1, false), target(2, 3, false)];
    assert_eq!(casters.plan(&twice, &[]), vec![None, None, Some(2)]);
    assert_eq!(entered(&twice, &[]), only(Entered::Twice, 2));
    let mut other_view = target(0, 1, false);
    other_view.view = 1;
    assert_eq!(casters.plan(&[other_view], &[]), vec![None], "another view is another figure");
    assert_eq!(entered(&[other_view], &[]), only(Entered::OtherView, 1));
    // The same actor in the same view under another key: which part of it.
    let mut mesh = target(0, 1, false);
    mesh.key.0 = 9;
    let mut epoch = target(1, 2, false);
    epoch.key.3 = 9;
    let mut keys = only(Entered::KeyMesh, 1);
    keys[Entered::KeyEpoch as usize] = 1;
    assert_eq!(entered(&[mesh, epoch], &[]), keys);
}

#[test]
fn a_splice_keeps_the_figures_that_stay_and_walks_and_places_only_the_one_that_joined() {
    let (mut casters, still, other, _) = laid_out();
    let before = casters.bytes.clone();
    let targets = [target(0, 3, false), target(1, 1, false), target(2, 4, false)];
    let next = casters.plan(&targets, &[]);
    for (target, figure) in targets.iter().zip(&next) {
        if let Some(figure) = *figure {
            casters.follow(figure, target);
        }
    }
    let mut recording = Recording::of(vec![(0, 2)]);
    assert!(recording.takes(0, 2));
    still_figure(&mut recording, 2, 4, 8);
    // The live figure posed with the bytes it holds: written, and nothing in the list changes.
    let same = [[f32::from_bits(0x0404_0404); 4]; 4];
    assert!(casters.write_unit(1, LiveBytes { world: &same, userdata: None, bones: Some(&[5; 128]) }));
    let bind = |_: &[u8],
                _: &Shared,
                _: &Pending,
                _: Block,
                _: Option<Block>|
     -> io::Result<Option<RecordedDraw>> { unreachable!("no figure here draws") };
    assert!(casters.splice(&next, &recording, vec![true], ALIGNMENT, bind).unwrap());

    let keys: Vec<_> = casters.figures.iter().map(|figure| figure.key).collect();
    assert_eq!(keys, vec![key(3), key(1), key(4)], "this frame's order");
    assert_eq!(casters.figures[0].span, other.at..other.at + 256, "kept where it was");
    assert_eq!(casters.figures[1].span, still.at..still.at + 256, "kept where it was");
    let joined = still_start() + 512;
    assert_eq!(casters.figures[2].span, joined..joined + 256, "taken above the still top");
    assert_eq!(&casters.bytes[joined..joined + 208], &[8; 208]);
    assert_eq!(&casters.bytes[..joined], &before[..joined], "nothing else written");
    assert_eq!(casters.dirty_ranges(), vec![joined..joined + 208], "only the joined blocks");
    let units: Vec<_> = casters.units.iter().map(|unit| (unit.index, unit.vertex)).collect();
    assert_eq!(
        units,
        vec![
            (0, Some(other)),
            (1, Some(still)),
            (2, Some(Block { live: false, at: joined, length: 208 })),
        ],
        "kept units follow their instances; the joined one's blocks are at its span"
    );
    assert!(casters.live().is_empty());
    assert_eq!(casters.live_area.top, 1024, "the live figure that left gave its span back");
    assert_eq!(casters.units_of(2).collect::<Vec<_>>(), vec![2]);
    let counts = casters.counts();
    assert_eq!((counts.walks, counts.spliced, counts.figures), (1, 1, 1), "one figure walked");

    // The next figure to join takes the span of one that left.
    let targets = [target(0, 3, false), target(1, 5, false), target(2, 4, false)];
    let next = casters.plan(&targets, &[]);
    assert_eq!(next, vec![Some(0), None, Some(2)]);
    let mut recording = Recording::of(vec![(0, 1)]);
    assert!(recording.takes(0, 1));
    still_figure(&mut recording, 1, 5, 9);
    assert!(casters.splice(&next, &recording, vec![true], ALIGNMENT, bind).unwrap());
    assert_eq!(casters.figures[1].span, still.at..still.at + 256);
    assert_eq!(&casters.bytes[still.at..still.at + 208], &[9; 208]);
    assert_eq!(casters.counts().figures, 2);
}

#[test]
fn a_splice_whose_walk_is_not_the_one_planned_or_has_no_room_makes_the_list_again() {
    let (mut casters, ..) = laid_out();
    let next = vec![Some(0), None];
    let bind = |_: &[u8],
                _: &Shared,
                _: &Pending,
                _: Block,
                _: Option<Block>|
     -> io::Result<Option<RecordedDraw>> { unreachable!("no figure here draws") };
    let empty = Recording::of(vec![(0, 1)]);
    assert!(!casters.splice(&next, &empty, vec![true], ALIGNMENT, bind).unwrap(), "no walk");

    let (mut casters, ..) = laid_out();
    let mut recording = Recording::of(vec![(0, 1)]);
    assert!(recording.takes(0, 1));
    recording.begin(0, 1, key(9), false);
    recording.place(false, &vec![1; 200 << 10], 0, ALIGNMENT);
    assert!(!casters.splice(&next, &recording, vec![true], ALIGNMENT, bind).unwrap(), "full");
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

    // A figure that starts walking keeps its actor: only its own nodes are made again.
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
    // 6 joins, 2 leaves, 5 is out of the order the others keep.
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
    // The layer hands over one field and the history places another the same frame: only the
    // last is compared with what was drawn.
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

    // An actor that changed before another left keeps its change listed.
    let mut walked = WalkedField::default();
    walked.note(&actors(&[1, 2, 3]), &mut changed);
    let mut next = actors(&[1, 2, 3]);
    next.instances[0].position[2] = 1.;
    next.instances[1].animation_identity = 9;
    assert_eq!(walked.note(&next, &mut changed), Change::Whole);
    assert_eq!(changed.instances, vec![0]);
}

#[test]
fn a_lit_node_writes_its_own_userdata_after_its_world() {
    let (mut list, still, ..) = laid_out();
    let before = list.bytes.clone();
    let (world, userdata) = ([[1.0f32; 4]; 4], [2.0f32; 8]);
    let bytes = LiveBytes { world: &world, userdata: Some(userdata), bones: None };
    assert!(list.write_unit(0, bytes));
    assert_eq!(&list.bytes[still.at..still.at + 64], bytemuck::bytes_of(&world));
    assert_eq!(&list.bytes[still.at + 64..still.at + 96], bytemuck::bytes_of(&userdata));
    assert_eq!(&list.bytes[still.at + 96..], &before[still.at + 96..]);
    assert_eq!(list.dirty_ranges(), vec![still.at..still.at + 96]);
}

fn pending(key: usize, vertex: Block, skin: Option<Block>, material: usize) -> Pending {
    Pending {
        key,
        view: 0,
        vertex,
        skin,
        reference: 0,
        vertices: ((48, false), 0),
        indices: 0,
        first: 0,
        count: 3,
        base: 0,
        textures: 1,
        material,
    }
}

/// A lit walk of two figures: a node drawing one part (frame, vertex, pixel and grid), then a
/// skinned node drawing two parts and a node written that draws nothing.
fn lit(reads: Reads) -> Kept {
    let mut recording = Recording::default();
    recording.begin(0, 0, key(1), false);
    let a = recording.place(false, &[1; 208], 0, ALIGNMENT);
    recording.units.push(Unit { mask: 0b10_1111, parts: 1, ..unit(false, 0, Some(a), None) });
    recording.pending.push(pending(3, a, None, 5));
    recording.begin(0, 1, key(2), false);
    let b = recording.place(false, &[2; 208], 0, ALIGNMENT);
    let bones = recording.place(false, &[6; 128], 0, ALIGNMENT);
    recording.units.push(Unit { mask: 0b11_1111, parts: 2, ..unit(false, 1, Some(b), Some(bones)) });
    recording.pending.push(pending(4, b, Some(bones), 6));
    recording.pending.push(pending(5, b, Some(bones), 7));
    let c = recording.place(false, &[3; 208], 0, ALIGNMENT);
    recording.units.push(unit(false, 1, Some(c), None));
    let grid = [0u8; 16];
    let layout = Layout::new(Vec::new(), &mut recording, &[&SCENE, &grid], &[416, 16], ALIGNMENT);
    let mut list = Kept::new(Kind::Lit);
    let shared = Shared { at: Vec::new(), reserved: vec![416, 16], reads };
    list.take(layout, recording, DrawList::default(), shared, ALIGNMENT, None);
    list
}

#[test]
fn a_lit_list_gives_the_constant_state_its_walk_leaves() {
    use super::plans::Source;
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
    // A pass whose nodes carry the incoming userdata keeps the incoming snapshot.
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
fn a_pass_that_places_nothing_keeps_no_lit_list() {
    let pass = ModelPass::with_rigs_for_tests(Vec::new(), "unit models");
    assert!(pass.layers.is_none(), "a library built empty has no content stack");
    for queue in [Queue::Regular, Queue::Alternate, Queue::Caster] {
        assert!(pass.kept_lit(queue).is_none());
    }
    assert!(pass.native.borrow().is_none(), "and no renderer was asked for");
}

#[test]
fn a_walk_of_some_figures_leaves_no_constant_state() {
    assert!(Recording::of(Vec::new()).partial());
    assert!(!Recording::default().partial());
}

#[test]
fn a_figure_that_waited_on_a_build_is_walked_again_alone_and_the_list_is_kept() {
    let mut recording = Recording::default();
    still_figure(&mut recording, 0, 1, 3);
    still_figure(&mut recording, 1, 2, 4);
    recording.wait();
    still_figure(&mut recording, 2, 3, 5);
    assert!(!recording.incomplete(), "the wait is its figure's");
    let shared = [&SCENE[..], &SCENE[..]];
    let layout = Layout::new(Vec::new(), &mut recording, &shared, &[416, 416], ALIGNMENT);
    let mut list = Kept::default();
    let made = laid_out_made();
    let blocks = Shared { at: Vec::new(), reserved: vec![416, 416], reads: Reads { views: 2, ..Reads::default() } };
    list.take(layout, recording, DrawList::default(), blocks, ALIGNMENT, Some(made.clone()));
    list.read(5);
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
fn part_counts_give_what_instancing_by_run_and_by_part_would_leave() {
    // Two figures of parts 3, 4 then 3, 4 in one view, then one of part 3 in another.
    let mut recording = Recording::default();
    for (position, view) in [(0, 0), (1, 0), (2, 1)] {
        recording.begin(view, position, key(position as u32), false);
        let block = recording.place(false, &[1; 208], 0, ALIGNMENT);
        let parts = if view == 0 { 2 } else { 1 };
        recording.units.push(Unit { view, parts, ..unit(false, position, Some(block), None) });
        recording.pending.push(pending(3, block, None, 5));
        if parts == 2 {
            recording.pending.push(pending(4, block, None, 5));
        }
    }
    let grid = [0u8; 16];
    let layout = Layout::new(Vec::new(), &mut recording, &[&SCENE, &SCENE, &grid], &[416, 416, 16], ALIGNMENT);
    let mut list = Kept::new(Kind::Lit);
    let shared = Shared { at: Vec::new(), reserved: vec![416, 416, 16], reads: Reads { views: 2, ..Reads::default() } };
    list.take(layout, recording, DrawList::default(), shared, ALIGNMENT, None);
    let counts = list.part_counts();
    assert_eq!(counts, PartCounts { parts: 2, runs: 5, in_views: 3 }, "3,4,3,4 then 3 in view 1");
}

/// The camera still, one unit walks out past its mesh's cull distance and another walks in: each
/// is culled or drawn on the frame its sphere's centre crosses. A list made with a figure culled
/// lets it join when it comes within the cull.
#[test]
fn with_the_camera_still_a_unit_walking_out_or_in_crosses_the_cull_on_the_frame_it_crosses() {
    use crate::map::models::GameCull;
    let wide = 1.0e4;
    let cull = GameCull {
        eye: [0., 100., 0.],
        sides: [[1., 0., 0., wide], [-1., 0., 0., wide], [0., 0., 1., wide], [0., 0., -1., wide]],
        x: 0.,
    };
    let distance = installed_content_core::mesh::DEFAULT_CULL_DISTANCE_SQUARED;
    let mut list = Kept::default();
    let mut culled = Vec::new();
    // 10 units a frame along x with the eye 100 above: 430 from the eye lies between 410 and 420.
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
