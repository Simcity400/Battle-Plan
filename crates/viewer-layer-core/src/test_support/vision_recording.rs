
use crate::{layers::{Layer, View}, vision};
use crate::playhead_host::{MapSide, mirror::Mirrors, open, units::Units};
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
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

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

pub fn column(table: TableId, name: &str) -> usize {
    (0..64).find(|&at| table.field_name(at) == Some(name)).unwrap_or_else(|| panic!("{table:?}.{name}"))
}

pub fn record(hours: u32, tables: &[TableId], mut hour: impl FnMut(u32, &mut Store) -> Vec<(TableId, Status)>) -> Units {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        let text = format!("text {at}");
        store.begin_hour(at as i32, &[text.as_bytes()]).unwrap();
        let statuses: BTreeMap<TableId, Status> = hour(at, &mut store).into_iter().collect();
        for &table in tables {
            let status = statuses.get(&table).copied().unwrap_or(Status::Complete);
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let run = Run::new(0, memory.pages).unwrap();
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![run], kinds: kinds.clone() });
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    static MADE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    Units { name: format!("p6 test {}", MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) }
}

pub fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

pub fn play(units: Units, layer: Box<dyn Layer>, last: i32, mut each: impl FnMut(i32, &Mirrors, &dyn Layer)) {
    play_all(units, vec![layer], last, |hour, mirrors, layers| each(hour, mirrors, &*layers[0]));
}

pub fn play_all(units: Units, layers: Vec<Box<dyn Layer>>, last: i32, mut each: impl FnMut(i32, &Mirrors, &[Box<dyn Layer>])) {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().seek(0);
    let mut seen = BTreeSet::new();
    let mut applied: Option<i32> = None;
    for _ in 0..50_000_000u64 {
        let newest = entry.playhead().newest().unwrap_or(0);
        let head = applied.map_or(newest, |hour| newest.min(hour + 4));
        let frame = side.frame(head).unwrap();
        if let Some(hour) = frame.applied {
            applied = Some(hour);
        }
        if frame.steps > 0 && !side.composing() {
            side.settle_all().unwrap();
            let (mirrors, layers) = side.parts().unwrap();
            let hour = frame.applied.unwrap();
            if seen.insert(hour) {
                each(hour, mirrors, layers);
            }
            if hour >= last {
                assert!(seen.len() >= 4, "steps seen: {seen:?}");
                return;
            }
        } else {
            std::thread::yield_now();
        }
    }
    panic!("the playhead never reached hour {last}");
}

pub fn vision_world() -> Units {
    vision_world_with(&[])
}

pub fn vision_world_with(extra: &[TableId]) -> Units {
    vision_world_and(extra, |_, _| Vec::new())
}

pub fn vision_world_and(extra: &[TableId], mut more: impl FnMut(u32, &mut Store) -> Vec<(TableId, Status)>) -> Units {
    let empty = [
        TableId::FactionMembers,
        TableId::WarRelations,
        TableId::RelationLinks,
        TableId::Subjects,
        TableId::UnitIntel,
        TableId::CountryVisibility,
        TableId::CountryIntel,
        TableId::CountryCounterInputs,
        TableId::ReservedCountryNaming,
        TableId::DivisionMovement,
        TableId::RadarCoverage,
        TableId::SimulationFeatures,
        TableId::StateVisibility,
        TableId::States,
        TableId::Fleets,
        TableId::TaskForces,
        TableId::StationedUnits,
        TableId::LandTheatres,
        TableId::TheatreAreas,
        TableId::VolunteerHosts,
    ];
    let mut tables = vec![TableId::CountryLinks, TableId::Countries, TableId::LocalPlayer, TableId::Provinces, TableId::Divisions];
    tables.extend(empty);
    let extra: Vec<TableId> = extra.iter().copied().filter(|table| !tables.contains(table)).collect();
    tables.extend(extra);
    assert!(viewer_foundation::perspective::VISION_TABLES.iter().all(|table| tables.contains(table)));
    record(44, &tables, |at, store| {
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
        if at == 0 {
            for (slot, raw) in [101u32, 102].into_iter().enumerate() {
                store.table(TableId::CountryLinks.index()).unwrap().insert_at(slot as u32, u128::from(raw)).unwrap();
                write(store, TableId::CountryLinks, slot as u32, "canonical_slot", Cell::U32(slot as u32 + 1));
                store.table(TableId::Countries.index()).unwrap().insert_at(slot as u32, u128::from(raw)).unwrap();
                write(store, TableId::Countries, slot as u32, "canonical_slot", Cell::U32(slot as u32 + 1));
                write(store, TableId::Countries, slot as u32, "tag", Cell::Bytes(0));
            }
            store.table(TableId::LocalPlayer.index()).unwrap().insert_at(0, 0).unwrap();
            write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
            for id in 1..=6u32 {
                store.table(TableId::Provinces.index()).unwrap().insert_at(id - 1, u128::from(id)).unwrap();
                write(store, TableId::Provinces, id - 1, "controller", Cell::U32(if id <= 3 { 101 } else { 102 }));
                write(store, TableId::Provinces, id - 1, "state", Cell::U32(0));
            }
            store.table(TableId::Provinces.index()).unwrap().insert_at(6, 7).unwrap();
            write(store, TableId::Provinces, 6, "controller", Cell::U32(0));
            write(store, TableId::Provinces, 6, "state", Cell::U32(0));
            store.table(TableId::Divisions.index()).unwrap().insert_at(0, 900).unwrap();
            for (field, value) in [("roster_owner", 101), ("logical_owner", 101), ("province", 3)] {
                write(store, TableId::Divisions, 0, field, Cell::U32(value));
            }
            write(store, TableId::Divisions, 0, "reveal_skip", Cell::U8(0));
            store.table(TableId::Divisions.index()).unwrap().insert_at(1, 901).unwrap();
            for (field, value) in [("roster_owner", 101), ("logical_owner", 101), ("province", 7)] {
                write(store, TableId::Divisions, 1, field, Cell::U32(value));
            }
            write(store, TableId::Divisions, 1, "reveal_skip", Cell::U8(1));
            store.table(TableId::DivisionMovement.index()).unwrap().insert_at(0, 901).unwrap();
            write(store, TableId::DivisionMovement, 0, "path", Cell::U32List(&[6]));
            store.table(TableId::WarRelations.index()).unwrap().insert_at(0, 101 << 32 | 102).unwrap();
        }
        match at {
            1 => write(store, TableId::Countries, 1, "tag", Cell::Bytes(1)),
            5 => write(store, TableId::Divisions, 0, "province", Cell::U32(4)),
            9 => write(store, TableId::Divisions, 0, "province", Cell::U32(5)),
            13 => write(store, TableId::Provinces, 3, "controller", Cell::U32(101)),
            26 => write(store, TableId::Provinces, 5, "controller", Cell::U32(101)),
            _ => {}
        }
        let mut statuses = more(at, store);
        match at {
            20..=23 => statuses.push((TableId::UnitIntel, Status::NotRecorded)),
            25.. => statuses.push((TableId::CountryIntel, Status::Truncated { missing: at })),
            _ => {}
        }
        statuses
    })
}

pub fn vision_content() -> Arc<vision::Content> {
    use viewer_foundation::visibility::{FogContent, Topology, test_defines};
    let land: BTreeSet<u32> = (1..=6).collect();
    let neighbors = (1..=7u32).map(|id| (id, [id - 1, id + 1].into_iter().filter(|next| (1..=7).contains(next)).collect())).collect();
    Arc::new(vision::Content::new(Ok(FogContent { topology: Topology { land, sea: BTreeSet::from([7]), neighbors }, defines: test_defines() })))
}
