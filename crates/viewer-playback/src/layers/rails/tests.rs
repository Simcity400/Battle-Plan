use super::{Geography, Rails, Target};
use crate::{
    layers::{Budget, Layer, View},
    playhead_host::{Frame, MapSide, mirror::Mirror, open, units::Units},
};
use playhead::{
    Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::TableId;
use std::{
    collections::BTreeMap,
    io,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_world::map::railways::{Bridge, BridgeKind, RailMesh, RailNetwork, RailRun};

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Recording {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

/// A strip and a bridge that read exactly what today's railway reads of the network: a province's
/// neighbours and their levels, the triangle test, its source flag and every anchor's harbour flag.
pub struct Stub;

impl Geography for Stub {
    fn cutoff(&self) -> f32 {
        300.0
    }

    fn run(&self, network: &RailNetwork, province: u32) -> io::Result<Option<RailRun>> {
        let Some(neighbours) = network.graph.get(&province) else { return Ok(None) };
        let port = |id: u32| f32::from(u8::from(network.ports.contains(&id)));
        let mut vertices = Vec::new();
        for (&a, &level) in neighbours {
            vertices.push([province as f32, a as f32, level as f32, f32::from(u8::from(network.sources.contains(&province))), port(a), port(province)]);
            for &b in neighbours.keys().filter(|&&b| b > a) {
                let triangle = network.graph.get(&b).is_some_and(|other| other.contains_key(&a));
                vertices.push([province as f32, a as f32, b as f32, f32::from(u8::from(triangle)), port(b), -1.0]);
            }
        }
        Ok(Some(RailRun { segments: neighbours.len(), unanchored: 0, samples: vertices.len(), vertices }))
    }

    fn bridge(&self, network: &RailNetwork, key: (u32, u32)) -> Option<Bridge> {
        let port = |id: u32| f32::from(u8::from(network.ports.contains(&id)));
        ((key.0 + key.1) % 2 == 1).then(|| Bridge { position: [key.0 as f32, key.1 as f32, port(key.0) + 2.0 * port(key.1)], rotation: 0.0, kind: BridgeKind::Small })
    }
}

const EDGES: [(u32, u32, i32); 8] = [(1, 2, 1), (2, 1, 1), (2, 3, 3), (3, 2, 3), (3, 4, 2), (4, 3, 2), (1, 3, 1), (3, 1, 1)];
const NOT_RECORDED: std::ops::Range<i32> = 40..48;
/// The steps the playhead builds past the head in [`side_of`].
const RING: usize = 3;

fn key(from: u32, to: u32) -> u128 {
    u128::from(from) << 32 | u128::from(to)
}

/// `hours` hours of the recording's own schema, each hour's strings and changes made by `hour`.
pub fn recorded(hours: u32, mut strings: impl FnMut(u32) -> Vec<String>, mut hour: impl FnMut(u32, &mut Store)) -> Arc<dyn Pages> {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        let named = strings(at);
        let named: Vec<&[u8]> = named.iter().map(String::as_bytes).collect();
        store.begin_hour(at as i32, &named).unwrap();
        hour(at, &mut store);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let run = Run::new(0, memory.pages).unwrap();
    Arc::new(Recording { bytes: memory.bytes, runs: vec![run], kinds })
}

/// A map side reading `pages` with `layer`, sought to hour 0 and playing four hours a step.
pub fn side_of(pages: Arc<dyn Pages>, layer: Box<dyn Layer>, name: &str) -> MapSide {
    side_at_pace(pages, layer, name, 4.0)
}

/// [`side_of`] playing at `speed` game hours a second, so its steps are that many hours long: a
/// playing head stops only at the end of a step, so a test that settles at every hour plays at 1.
pub fn side_at_pace(pages: Arc<dyn Pages>, layer: Box<dyn Layer>, name: &str, speed: f64) -> MapSide {
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    let units = Units { name: name.to_owned(), pages, timeline, kinds };
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: RING, speed, rate: 1.0 };
    let entry = open(units, settings).unwrap();
    let side = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    entry.playhead().play(speed);
    entry.playhead().seek(0);
    side
}

/// RailEdges, ProvinceSupply and NavalBases over `hours`: edges written twice inside one step,
/// deleted and inserted, a supply source appearing, a harbour moving, and NavalBases not recorded
/// for a stretch.
fn record(hours: u32) -> Arc<dyn Pages> {
    let (edges, supply, naval) = (TableId::RailEdges.index(), TableId::ProvinceSupply.index(), TableId::NavalBases.index());
    recorded(hours, |_| Vec::new(), |at, store| {
        let naval_status = if NOT_RECORDED.contains(&(at as i32)) { Status::NotRecorded } else { Status::Complete };
        for (table, status) in [(edges, Status::Complete), (supply, Status::Complete), (naval, naval_status)] {
            store.status(table, status, true, Some(at as i32), true).unwrap();
        }
        let mut changes = store.table(edges).unwrap();
        match at {
            0 => {
                for (slot, &(from, to, level)) in EDGES.iter().enumerate() {
                    changes.insert_at(slot as u32, key(from, to)).unwrap();
                    changes.write(slot as u32, 0, Cell::I32(level)).unwrap();
                }
            }
            9 => changes.write(2, 0, Cell::I32(0)).unwrap(),
            10 => {
                changes.write(2, 0, Cell::I32(5)).unwrap();
                changes.insert_at(8, key(4, 5)).unwrap();
                changes.write(8, 0, Cell::I32(1)).unwrap();
            }
            20 => {
                changes.delete(6).unwrap();
                changes.delete(7).unwrap();
            }
            _ => {}
        }
        let mut changes = store.table(supply).unwrap();
        match at {
            0 => {
                for province in 1..=5u32 {
                    changes.insert_at(province - 1, u128::from(province)).unwrap();
                    for column in 0..5 {
                        let value = if column == 4 && province == 4 { 1 } else { 0 };
                        changes.write(province - 1, column, if column < 2 { Cell::U16(value) } else { Cell::U8(value as u8) }).unwrap();
                    }
                }
            }
            21 => changes.write(1, 2, Cell::U8(1)).unwrap(),
            22 => changes.write(1, 2, Cell::U8(0)).unwrap(),
            23 => changes.write(1, 2, Cell::U8(2)).unwrap(),
            _ => {}
        }
        if naval_status == Status::Complete {
            let mut changes = store.table(naval).unwrap();
            match at {
                0 => {
                    changes.insert_at(0, 3).unwrap();
                }
                30 => {
                    changes.delete(0).unwrap();
                    changes.insert_at(0, 5).unwrap();
                }
                _ => {}
            }
        }
    })
}

fn side(name: &str) -> MapSide {
    side_of(record(80), Box::new(Rails::new(Arc::new(Stub))), name)
}

/// Frames until `done` holds, the head one step past the last applied so a frame takes one step.
pub fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &Frame) -> bool) {
    let mut applied: Option<i32> = None;
    for _ in 0..50_000_000u64 {
        crate::layers::recorded::reads_landed(side);
        let newest = side.entry().playhead().newest().unwrap_or(0);
        let head = applied.map_or(newest, |hour| newest.min(hour + 4));
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached what the test waits for");
}

/// Frames at `hour` until it is applied, nothing composes and no layer carries frame-thread work:
/// one step taken whole, however the frames are paced. Fails by name within 120 s.
pub fn settle_at(side: &mut MapSide, hour: i32) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        crate::layers::recorded::reads_landed(side);
        let frame = side.frame(hour).unwrap();
        let carried: Vec<(&str, usize)> = side.parts().map(|(_, layers)| layers.iter().map(|layer| (layer.name(), layer.carried().entities)).collect()).unwrap_or_default();
        if side.applied() == Some(hour) && !side.composing() && carried.iter().all(|&(_, entities)| entities == 0) {
            return;
        }
        if Instant::now() >= deadline {
            let (_, _, counters) = side.entry().playhead().held();
            panic!("hour {hour} was not applied and settled within 120 s: applied {:?}, newest decoded {:?}, composing {}, carried {carried:?}, the playhead's {counters:?}", side.applied(), side.entry().playhead().newest(), side.composing());
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

/// Frames at `hour` until it is applied, whatever the layers carry: no step past it is taken.
/// Fails by name within 120 s.
fn applied_at(side: &mut MapSide, hour: i32) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let frame = side.frame(hour).unwrap();
        if side.applied() == Some(hour) && !side.composing() {
            return;
        }
        assert!(Instant::now() < deadline, "hour {hour} was not applied within 120 s: applied {:?}, composing {}", side.applied(), side.composing());
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

fn rails(side: &MapSide) -> &Rails {
    side.parts().unwrap().1[0].any().downcast_ref::<Rails>().unwrap()
}

/// The layer settled at every step against itself composed from the mirrors, and the network
/// against the recorded story at the hours it changes.
#[test]
fn rails_settled_every_step_equal_composed_and_follow_writes_deletes_inserts_and_gaps() {
    let mut side = side("rails-steps");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut seen = Vec::new();
    frames_until(&mut side, |side, frame| {
        if frame.steps == 0 || side.composing() {
            return false;
        }
        let (mirrors, _) = side.parts().unwrap();
        let layer = rails(side);
        assert_eq!(layer.carried().entities, 0, "a 3 ms budget settles a step of this world");
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.differ(&fresh), (0, None), "settled at hour {:?} equals composed", frame.applied);
        let hour = frame.applied.unwrap();
        let network = layer.network();
        let level = |from: u32, to: u32| network.graph.get(&from).and_then(|neighbours| neighbours.get(&to)).copied();
        if NOT_RECORDED.contains(&hour) {
            assert!(network.graph.is_empty() && layer.runs().is_empty() && layer.bridges().is_empty(), "NavalBases not recorded draws no railway at hour {hour}");
        } else if hour >= 12 {
            assert_eq!(level(2, 3), Some(5), "an edge written twice in a step shows its newest level at hour {hour}");
            assert_eq!(level(4, 5), Some(1), "an inserted edge is drawn at hour {hour}");
            assert!(layer.runs().contains_key(&4) && layer.bridges().contains_key(&(4, 5)));
            if hour >= 24 {
                assert_eq!((level(1, 3), level(3, 1)), (None, None), "deleted edges are gone at hour {hour}");
                assert!(network.sources.contains(&2) && network.sources.contains(&4), "sources at hour {hour}");
            }
            if hour >= 32 {
                assert_eq!(network.ports.iter().copied().collect::<Vec<_>>(), vec![5], "the harbour moved at hour {hour}");
            }
        }
        seen.push(hour);
        hour >= 76
    });
    assert!(seen.iter().any(|hour| NOT_RECORDED.contains(hour)) && seen.iter().any(|&hour| hour >= 48 && hour < 76), "{seen:?}");
}

#[test]
fn rails_out_of_budget_carry_and_settle_late_to_the_newest_values() {
    let mut side = side("rails-budget");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    // Held, the steps' marks are carried and the host counts the frames; released with no budget, a
    // unit a frame settles them.
    side.set_budget(Duration::ZERO);
    let settles = side.hold_settles();
    let mut late = 0;
    frames_until(&mut side, |side, frame| {
        late = frame.layers[0].late;
        frame.applied.is_some_and(|hour| hour >= 36) && rails(side).carried().entities > 0 && late >= 1
    });
    assert!(late >= 1, "the carry-over is counted in frames: {late}");
    crate::layers::recorded::released_unit_by_unit(&mut side, settles);
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = rails(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(rails(&side).differ(&fresh), (0, None), "settled late, with the newest values");
}

#[test]
fn rails_above_the_cutoff_declare_and_hold_nothing_and_settle_again_when_shown() {
    let mut side = side("rails-view");
    settle_at(&mut side, 0);
    side.set_view(View { camera_height: 1000.0, ..View::default() }).unwrap();
    let shown = 20;
    for hour in (4..=shown).step_by(4) {
        settle_at(&mut side, hour);
    }
    let layer = rails(&side);
    assert!(!layer.shown() && layer.runs().is_empty() && layer.bridges().is_empty() && layer.network().graph.is_empty(), "hidden, the layer holds nothing");
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut inputs = crate::layers::Inputs::new(&kinds, "rails");
    let mut probe = Rails::new(Arc::new(Stub));
    probe.inputs(&View { camera_height: 1000.0, ..View::default() }, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "hidden, the layer declares nothing");
    // Shown again, its tables join at the hour the map's frames are at: with the head held there,
    // they arrive as a reset at that hour and the layer composes, and at every step after it is
    // drawn whole: a railway, but none while NavalBases is not recorded.
    side.set_view(View::default()).unwrap();
    let tables = [TableId::RailEdges, TableId::ProvinceSupply, TableId::NavalBases];
    let heads = |side: &MapSide| side.parts().map(|(mirrors, _)| tables.map(|table| (table, mirrors.get(table).and_then(Mirror::head))).to_vec());
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let frame = side.frame(shown).unwrap();
        let back = heads(&side).is_some_and(|heads| heads.iter().all(|&(_, head)| head.is_some()));
        if back && side.draws("rails") && !side.composing() && rails(&side).carried().entities == 0 {
            break;
        }
        if Instant::now() >= deadline {
            let (_, _, counters) = side.entry().playhead().held();
            panic!("the rails tables, declared again at hour {shown} with the head held there, were not back within 120 s: applied {:?}, composing {}, rails drawn {}, mirrors at {:?}, the playhead's {counters:?}", side.applied(), side.composing(), side.draws("rails"), heads(&side));
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    assert_eq!(heads(&side), Some(tables.map(|table| (table, Some(shown))).to_vec()), "declared at hour {shown}, the tables arrive at that hour");
    let mut railway = 0;
    for hour in (shown..=56).step_by(4) {
        settle_at(&mut side, hour);
        let recorded: Vec<(TableId, Option<i32>)> = heads(&side).unwrap_or_default().into_iter().filter(|&(table, _)| table != TableId::NavalBases || !NOT_RECORDED.contains(&hour)).collect();
        assert!(recorded.len() >= 2 && recorded.iter().all(|&(_, head)| head == Some(hour)), "drawn again, the recorded rails tables at hour {hour}: {recorded:?}");
        let (mirrors, _) = side.parts().unwrap();
        let layer = rails(&side);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert!(side.draws("rails") && layer.shown(), "drawn again at hour {hour}, the layer is shown");
        assert_eq!(layer.differ(&fresh), (0, None), "shown again, settled at hour {hour} equals composed");
        assert_eq!(layer.runs().is_empty(), NOT_RECORDED.contains(&hour), "drawn again at hour {hour}: strips, but none while NavalBases is not recorded");
        railway += usize::from(!layer.runs().is_empty());
    }
    assert!(railway > 0, "shown again at hour {shown}, the railway was drawn again");
}

/// A scene that keeps what it is sent: the strip buffer, the strips it draws, and what each
/// upload wrote.
#[derive(Default)]
struct Counted {
    buffer: Vec<[f32; 6]>,
    runs: Vec<Range<u32>>,
    writes: usize,
    written: usize,
    bridges: usize,
}

impl Target for Counted {
    fn reserve(&mut self, vertices: u32, kept: u32) -> io::Result<()> {
        assert!(kept as usize <= self.buffer.len() && kept <= vertices, "kept {kept} of {vertices}, {} held", self.buffer.len());
        self.buffer.resize(vertices as usize, [0.0; 6]);
        Ok(())
    }

    fn write(&mut self, at: u32, vertices: &[[f32; 6]]) {
        self.buffer[at as usize..at as usize + vertices.len()].copy_from_slice(vertices);
        self.writes += 1;
        self.written += vertices.len();
    }

    fn publish(&mut self, runs: &[Range<u32>]) -> io::Result<()> {
        self.runs = runs.to_vec();
        Ok(())
    }

    fn bridges(&mut self, _bridges: &BTreeMap<(u32, u32), Bridge>) -> io::Result<()> {
        self.bridges += 1;
        Ok(())
    }
}

impl Counted {
    /// The vertices drawn, run after run.
    fn drawn(&self) -> Vec<[f32; 6]> {
        self.runs.iter().flat_map(|run| self.buffer[run.start as usize..run.end as usize].iter().copied()).collect()
    }
}

/// Today's whole mesh of the layer's strips: what the scene must draw.
fn whole(layer: &Rails) -> Vec<[f32; 6]> {
    RailMesh::assemble(layer.runs().values(), layer.bridges().values().copied()).unwrap().native_vertices
}

const CHAIN: u32 = 200;

/// A railway of `CHAIN` provinces in a row, each edge both ways at level 1; at hour 6 one way of the
/// edge 100 to 101 goes to level 3.
fn chain() -> Arc<dyn Pages> {
    let (edges, supply, naval) = (TableId::RailEdges.index(), TableId::ProvinceSupply.index(), TableId::NavalBases.index());
    recorded(12, |_| Vec::new(), |at, store| {
        for table in [edges, supply, naval] {
            store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        }
        let mut changes = store.table(edges).unwrap();
        match at {
            0 => {
                for province in 1..CHAIN {
                    let slot = 2 * (province - 1);
                    changes.insert_at(slot, key(province, province + 1)).unwrap();
                    changes.write(slot, 0, Cell::I32(1)).unwrap();
                    changes.insert_at(slot + 1, key(province + 1, province)).unwrap();
                    changes.write(slot + 1, 0, Cell::I32(1)).unwrap();
                }
            }
            6 => changes.write(2 * 99, 0, Cell::I32(3)).unwrap(),
            _ => {}
        }
    })
}

/// The layer settled at `hour` on its own, every strip and bridge then sent to `scene`.
fn sent_at(side: &mut MapSide, hour: i32, scene: &mut Counted) -> Rails {
    settle_at(side, hour);
    let (mirrors, _) = side.parts().unwrap();
    let mut layer = rails(side).fresh();
    layer.compose(mirrors).unwrap();
    layer.push(scene, &mut Budget::unlimited()).unwrap();
    assert!(layer.uploaded(), "an unlimited upload sends everything");
    layer
}

/// One railway's level changes: the upload writes that railway's strip, not the strips in view,
/// and the scene draws today's whole mesh. Before, an upload rebuilt and wrote the whole mesh.
#[test]
fn rails_upload_writes_only_the_strip_a_level_change_redrew() {
    let mut side = side_of(chain(), Box::new(Rails::new(Arc::new(Stub))), "rails-upload");
    let mut scene = Counted::default();
    let mut layer = sent_at(&mut side, 4, &mut scene);
    assert_eq!(layer.runs().len(), CHAIN as usize, "a strip a province");
    assert_eq!((scene.writes, scene.written), (CHAIN as usize, whole(&layer).len()), "composed, every strip is sent once");
    assert_eq!(scene.drawn(), whole(&layer), "the scene draws today's mesh");
    let before = layer.runs()[&100].vertices.clone();

    settle_at(&mut side, 8);
    let (mirrors, _) = side.parts().unwrap();
    layer.mark_edge(key(100, 101));
    layer.settle_dirty(mirrors, &mut Budget::unlimited()).unwrap();
    assert_ne!(layer.runs()[&100].vertices, before, "the level is drawn in province 100's strip");
    let (writes, written, bridges) = (scene.writes, scene.written, scene.bridges);
    assert!(!layer.uploaded(), "the redrawn strip is owed to the scene");
    layer.push(&mut scene, &mut Budget::unlimited()).unwrap();
    assert_eq!(
        (scene.writes - writes, scene.written - written, scene.bridges - bridges),
        (1, layer.runs()[&100].vertices.len(), 0),
        "one strip written, of {} in view, and no bridge placed again",
        CHAIN
    );
    assert_eq!(scene.drawn(), whole(&layer), "the scene draws today's mesh after the change");
}

/// With its budget spent, each upload sends one piece (the bridges, or one strip) and carries the
/// rest; the pieces together draw today's whole mesh.
#[test]
fn rails_upload_out_of_budget_sends_one_piece_a_frame_and_carries_the_rest() {
    let mut side = side_of(chain(), Box::new(Rails::new(Arc::new(Stub))), "rails-upload-budget");
    settle_at(&mut side, 4);
    let (mirrors, _) = side.parts().unwrap();
    let mut layer = rails(&side).fresh();
    layer.compose(mirrors).unwrap();
    let mut scene = Counted::default();
    let mut uploads = 0;
    while !layer.uploaded() {
        let (writes, bridges) = (scene.writes, scene.bridges);
        layer.push(&mut scene, &mut Budget::new(Duration::ZERO)).unwrap();
        uploads += 1;
        assert_eq!((scene.writes - writes) + (scene.bridges - bridges), 1, "upload {uploads} sends one piece");
        assert!(uploads <= CHAIN as usize + 1, "the upload never finished");
    }
    assert_eq!(uploads, CHAIN as usize + 1, "the bridges, then a strip an upload");
    assert_eq!(scene.drawn(), whole(&layer), "the pieces draw today's mesh");
}

/// Four supply rows, province 4 a source by its node level; at hour 6 province 3's flags gain a
/// bit the railway does not read, province 1's controller moves and province 4's level rises; at
/// hour 10 province 3's flags gain their first bit.
fn sources() -> Arc<dyn Pages> {
    let (edges, supply, naval) = (TableId::RailEdges.index(), TableId::ProvinceSupply.index(), TableId::NavalBases.index());
    recorded(16, |_| Vec::new(), |at, store| {
        for table in [edges, supply, naval] {
            store.status(table, Status::Complete, true, Some(at as i32), true).unwrap();
        }
        if at == 0 {
            let mut changes = store.table(edges).unwrap();
            for (slot, &(from, to, level)) in EDGES.iter().enumerate() {
                changes.insert_at(slot as u32, key(from, to)).unwrap();
                changes.write(slot as u32, 0, Cell::I32(level)).unwrap();
            }
        }
        let mut changes = store.table(supply).unwrap();
        match at {
            0 => {
                for province in 1..=4u32 {
                    changes.insert_at(province - 1, u128::from(province)).unwrap();
                    for column in 0..5 {
                        let value = u8::from(column == 2 && province == 4);
                        changes.write(province - 1, column, if column < 2 { Cell::U16(u16::from(value)) } else { Cell::U8(value) }).unwrap();
                    }
                }
            }
            6 => {
                changes.write(2, 4, Cell::U8(2)).unwrap();
                changes.write(0, 0, Cell::U16(7)).unwrap();
                changes.write(3, 2, Cell::U8(2)).unwrap();
            }
            10 => changes.write(2, 4, Cell::U8(3)).unwrap(),
            _ => {}
        }
    })
}

/// A ProvinceSupply write that leaves every province a source or not as it was marks nothing: a
/// flags bit the railway does not read, a column it does not read, a level that stays above zero.
/// Before, each write of a read column marked its row for the settle.
#[test]
fn rails_mark_no_supply_row_for_a_write_that_leaves_its_source_as_it_was() {
    let mut side = side_of(sources(), Box::new(Rails::new(Arc::new(Stub))), "rails-sources");
    settle_at(&mut side, 4);
    let before = rails(&side).marked()[1];
    side.set_budget(Duration::ZERO);
    let settles = side.hold_settles();
    // Frames at a head of the test's own: the hour 10 write stays out of the first window.
    applied_at(&mut side, 8);
    assert_eq!(rails(&side).marked()[1], before, "no supply row marked by the writes at hour 6");
    assert_eq!(rails(&side).carried().entities, 0, "nothing carried");
    applied_at(&mut side, 12);
    assert_eq!(rails(&side).marked()[1], before + 1, "province 3's first flags bit marks its row");
    assert_eq!(rails(&side).carried().entities, 1, "held, the row is carried");
    crate::layers::recorded::released_unit_by_unit(&mut side, settles);
    side.set_budget(Duration::from_secs(1));
    let hour = side.applied().unwrap();
    settle_at(&mut side, hour);
    let sources: Vec<u32> = rails(&side).network().sources.iter().copied().collect();
    assert_eq!(sources, [3, 4], "province 3 became a source, 4 stayed one");
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = rails(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(rails(&side).differ(&fresh), (0, None), "settled, the railway equals one composed");
}
