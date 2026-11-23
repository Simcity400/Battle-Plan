use super::test_support::Stub;
use super::{Rails, Strips, Target};
use viewer_layer_core::test_support::rail_recording::{frames_until, recorded, side_of};
use viewer_layer_core::{layers::{Layer, View}, playhead_host::{MapSide, mirror::Mirror}};
use playhead::pages::Pages;
use replay_columnar::{manifest::Status, state::Cell};
use replay_data::rows::TableId;
use std::{
    collections::BTreeMap,
    io,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_rail::railways::{Bridge, RailMesh};

const EDGES: [(u32, u32, i32); 8] = [(1, 2, 1), (2, 1, 1), (2, 3, 3), (3, 2, 3), (3, 4, 2), (4, 3, 2), (1, 3, 1), (3, 1, 1)];
const NOT_RECORDED: std::ops::Range<i32> = 40..48;

fn key(from: u32, to: u32) -> u128 {
    u128::from(from) << 32 | u128::from(to)
}




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

pub(crate) fn settle_at(side: &mut MapSide, hour: i32) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        viewer_layer_core::test_support::recorded::reads_landed(side);
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
        assert_eq!(layer.carried().entities, 0, "a 3 ms settling settles a step of this world");
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
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "rails");
    let mut probe = Rails::new(Arc::new(Stub));
    probe.inputs(&View { camera_height: 1000.0, ..View::default() }, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "hidden, the layer declares nothing");
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
    fn drawn(&self) -> Vec<[f32; 6]> {
        self.runs.iter().flat_map(|run| self.buffer[run.start as usize..run.end as usize].iter().copied()).collect()
    }
}

fn whole(layer: &Rails) -> Vec<[f32; 6]> {
    RailMesh::assemble(layer.runs().values(), layer.bridges().values().copied()).unwrap().native_vertices
}

const CHAIN: u32 = 200;

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

fn sent_at(side: &mut MapSide, hour: i32, scene: &mut Counted) -> Rails {
    settle_at(side, hour);
    let (mirrors, _) = side.parts().unwrap();
    let mut layer = rails(side).fresh();
    layer.compose(mirrors).unwrap();
    layer.push(scene).unwrap();
    assert!(layer.uploaded(), "an unlimited upload sends everything");
    layer
}

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
    layer.settle_dirty(mirrors).unwrap();
    assert_ne!(layer.runs()[&100].vertices, before, "the level is drawn in province 100's strip");
    let (writes, written, bridges) = (scene.writes, scene.written, scene.bridges);
    assert!(!layer.uploaded(), "the redrawn strip is owed to the scene");
    layer.push(&mut scene).unwrap();
    assert_eq!(
        (scene.writes - writes, scene.written - written, scene.bridges - bridges),
        (1, layer.runs()[&100].vertices.len(), 0),
        "one strip written, of {} in view, and no bridge placed again",
        CHAIN
    );
    assert_eq!(scene.drawn(), whole(&layer), "the scene draws today's mesh after the change");
}

#[test]
fn rails_upload_sends_every_piece_at_once() {
    let mut side = side_of(chain(), Box::new(Rails::new(Arc::new(Stub))), "rails-upload-settling");
    settle_at(&mut side, 4);
    let (mirrors, _) = side.parts().unwrap();
    let mut layer = rails(&side).fresh();
    layer.compose(mirrors).unwrap();
    let mut scene = Counted::default();
    layer.push(&mut scene).unwrap();
    assert!(layer.uploaded(), "one upload sends it all");
    assert_eq!(scene.writes + scene.bridges, CHAIN as usize + 1, "the bridges and a strip a chain link");
    assert_eq!(scene.drawn(), whole(&layer), "the pieces draw today's mesh");
}

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

#[test]
fn rails_mark_no_supply_row_for_a_write_that_leaves_its_source_as_it_was() {
    let mut side = side_of(sources(), Box::new(Rails::new(Arc::new(Stub))), "rails-sources");
    settle_at(&mut side, 4);
    let before = rails(&side).marked()[1];
    let settles = side.hold_settles();
    applied_at(&mut side, 8);
    assert_eq!(rails(&side).marked()[1], before, "no supply row marked by the writes at hour 6");
    assert_eq!(rails(&side).carried().entities, 0, "nothing carried");
    applied_at(&mut side, 12);
    assert_eq!(rails(&side).marked()[1], before + 1, "province 3's first flags bit marks its row");
    assert_eq!(rails(&side).carried().entities, 1, "held, the row is carried");
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    let hour = side.applied().unwrap();
    settle_at(&mut side, hour);
    let sources: Vec<u32> = rails(&side).network().sources.iter().copied().collect();
    assert_eq!(sources, [3, 4], "province 3 became a source, 4 stayed one");
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = rails(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(rails(&side).differ(&fresh), (0, None), "settled, the railway equals one composed");
}

#[test]
fn rail_strip_indices_are_written_where_a_run_moved_and_read_nowhere_else() {
    let whole = |runs: &[Range<u32>]| -> Vec<u32> { runs.iter().flat_map(|run| run.clone().chain(std::iter::once(u32::MAX))).collect() };
    let mut runs: Vec<Range<u32>> = (0..50u32).map(|at| at * 10..at * 10 + 8).collect();
    let mut strips = Strips::default();
    let (length, owed) = strips.laid(&runs).unwrap();
    assert_eq!((length, owed.clone()), (whole(&runs).len(), Some(0..whole(&runs).len())), "nothing stands yet, so the whole list is owed");
    strips.sent = length;
    assert_eq!(strips.held[..length], whole(&runs)[..], "the list laid out whole");
    assert_eq!(strips.laid(&runs).unwrap(), (length, None), "the same runs owe nothing");

    runs[20] = 1_000..1_008;
    let (length, owed) = strips.laid(&runs).unwrap();
    assert_eq!(owed, Some(20 * 9..20 * 9 + 8), "the run's entries, not its separator, which did not move");
    assert_eq!(strips.held[..length], whole(&runs)[..], "and the list holds what a whole layout holds");
    strips.sent = length;

    runs[10] = 2_000..2_004;
    let (length, owed) = strips.laid(&runs).unwrap();
    assert_eq!(owed.map(|owed| owed.start), Some(10 * 9), "from the run that changed length");
    assert_eq!(strips.held[..length], whole(&runs)[..], "the shifted list");
    strips.sent = length;

    runs.remove(30);
    let (length, _) = strips.laid(&runs).unwrap();
    assert_eq!(strips.held[..length], whole(&runs)[..]);
    strips.sent = length;
    runs.push(5_000..5_016);
    let (length, owed) = strips.laid(&runs).unwrap();
    assert_eq!(owed.map(|owed| owed.start), Some(strips.sent), "only the new tail");
    assert_eq!(strips.held[..length], whole(&runs)[..]);
}
