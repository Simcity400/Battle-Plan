use super::Relations;
use viewer_layer_core::{layers::View, playhead_host::{MapSide, open, units::Units}};
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
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use viewer_counter_kit::type_tint::{Index, Side};

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

fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap_or_else(|| panic!("{table:?}.{field}"))
}

fn record(hours: u32, mut hour: impl FnMut(u32, &mut Store)) -> Units {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        let text = format!("text {at}");
        store.begin_hour(at as i32, &[text.as_bytes()]).unwrap();
        for table in super::READ {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        hour(at, &mut store);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds: kinds.clone() });
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    static MADE: AtomicU32 = AtomicU32::new(0);
    Units { name: format!("relations {}", MADE.fetch_add(1, Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) }
}

fn insert(store: &mut Store, table: TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn delete(store: &mut Store, table: TableId, slot: u32) {
    store.table(table.index()).unwrap().delete(slot).unwrap();
}

fn world(store: &mut Store) {
    for (at, raw) in (10..14u32).enumerate() {
        let (slot, canonical) = (at as u32, at as u32 + 1);
        insert(store, TableId::CountryLinks, slot, u128::from(raw));
        store.table(TableId::CountryLinks.index()).unwrap().write(slot, column(TableId::CountryLinks, "canonical_slot"), Cell::U32(canonical)).unwrap();
        insert(store, TableId::Countries, slot, u128::from(raw));
        store.table(TableId::Countries.index()).unwrap().write(slot, column(TableId::Countries, "canonical_slot"), Cell::U32(canonical)).unwrap();
    }
}

fn pair(a: u32, b: u32) -> u128 {
    (u128::from(a) << 32) | u128::from(b)
}

fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

fn reach(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour + 3);
        let frame = side.frame(head).unwrap();
        if !side.composing() && frame.applied.is_some_and(|applied| applied >= hour) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached hour {hour}");
}

fn index(side: &MapSide) -> Arc<Index> {
    Arc::clone(side.parts().unwrap().1[0].any().downcast_ref::<Relations>().unwrap().index())
}

#[test]
fn a_war_a_faction_and_a_subject_each_change_exactly_the_countries_they_name() {
    let units = record(BLOCK, |at, store| match at {
        0 => world(store),
        200 => insert(store, TableId::WarRelations, 0, pair(10, 12)),
        220 => {
            insert(store, TableId::FactionMembers, 0, 10);
            store.table(TableId::FactionMembers.index()).unwrap().write(0, column(TableId::FactionMembers, "faction"), Cell::U64(7)).unwrap();
            insert(store, TableId::FactionMembers, 1, 11);
            store.table(TableId::FactionMembers.index()).unwrap().write(1, column(TableId::FactionMembers, "faction"), Cell::U64(7)).unwrap();
        }
        240 => delete(store, TableId::FactionMembers, 1),
        260 => insert(store, TableId::Subjects, 0, pair(10, 13)),
        280 => delete(store, TableId::Subjects, 0),
        300 => insert(store, TableId::WarRelations, 1, pair(11, 13)),
        _ => {}
    });
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Relations::new().viewed_as(1))], View::default()).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    reach(&mut side, 150);
    let mut before = index(&side);
    assert_eq!(before.of(10, 10), Some(Side::Friend), "the viewer is its own friend");
    assert_eq!(before.of(12, 12), Some(Side::Neutral));
    for (hour, named, side_of_named) in [(204, 12, Side::Enemy), (224, 11, Side::Friend), (244, 11, Side::Neutral), (264, 13, Side::Friend), (284, 13, Side::Neutral)] {
        reach(&mut side, hour);
        let after = index(&side);
        assert_eq!(after.changed_from(&before), vec![named], "at hour {hour}");
        assert_eq!(after.of(named, named), Some(side_of_named), "at hour {hour}");
        before = after;
    }
    reach(&mut side, 304);
    let after = index(&side);
    assert_eq!(after.changed_from(&before), Vec::<u32>::new(), "a war between two other countries changes no country's relation");
    let layer = side.parts().unwrap().1[0].any().downcast_ref::<Relations>().unwrap();
    let wars = layer.wars().unwrap();
    assert_eq!(wars.at_war(11, 13), Ok(true), "but who is at war with whom is read again");
    assert_eq!(wars.at_war(10, 12), Ok(true));
    assert_eq!(wars.at_war(12, 10), Ok(true));
    assert_eq!(wars.at_war(10, 11), Ok(false));
    assert!(matches!(wars.at_war(99, 10), Err(super::WarUnknown::Receiver(_))));
    let (mirrors, _) = side.parts().unwrap();
    assert_eq!(layer.differ(&layer.composed(mirrors)), (0, None), "stepwise equals composed");
}

#[test]
fn a_change_decides_the_countries_it_names_and_no_other() {
    let units = record(BLOCK, |at, store| match at {
        0 => world(store),
        200 => insert(store, TableId::WarRelations, 0, pair(10, 12)),
        300 => insert(store, TableId::WarRelations, 1, pair(11, 13)),
        _ => {}
    });
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Relations::new().viewed_as(1))], View::default()).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    reach(&mut side, 150);
    let decided = |side: &MapSide| side.parts().unwrap().1[0].any().downcast_ref::<Relations>().unwrap().decided();
    let composed = decided(&side);
    reach(&mut side, 180);
    assert_eq!(decided(&side), composed, "thirty hours that changed nothing decided no country");
    reach(&mut side, 204);
    assert_eq!(decided(&side) - composed, 1, "the war names the viewer and country 12: country 12 alone is decided");
    let after = decided(&side);
    reach(&mut side, 304);
    assert_eq!(decided(&side), after, "a war between two other countries decides none");
    let layer = side.parts().unwrap().1[0].any().downcast_ref::<Relations>().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    assert_eq!(layer.differ(&layer.composed(mirrors)), (0, None), "stepwise equals composed");
}
