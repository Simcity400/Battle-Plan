use crate::playhead_host::{Frame, MapSide, units::Units};
use playhead::{Settings, Timeline, pages::{Pages, Run}};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, TableChanges},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::TableId;
use std::{io, sync::Arc};

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

pub struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
    format: u16,
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

    fn format(&self) -> u16 {
        self.format
    }
}

pub fn record(hours: u32, mut hour: impl FnMut(u32, &mut Store)) -> Arc<Recording> {
    record_runs(&[(0, hours)], &[], |at, store| hour(at as u32, store))
}

pub fn record_runs(runs: &[(i32, u32)], names: &[&[u8]], hour: impl FnMut(i32, &mut Store)) -> Arc<Recording> {
    record_runs_at(replay_data::FORMAT_VERSION, runs, names, hour)
}

pub fn record_runs_at(format: u16, runs: &[(i32, u32)], names: &[&[u8]], mut hour: impl FnMut(i32, &mut Store)) -> Arc<Recording> {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut written = Vec::new();
    let mut bytes = Vec::new();
    for &(first, hours) in runs {
        let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
        store.declare_logs(&replay_data::rows::logs()).unwrap();
        store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
        let mut memory = Memory { bytes: std::mem::take(&mut bytes), pages: Vec::new() };
        store.begin_run(first);
        for at in 0..hours {
            let text = format!("name {at}");
            let mut appends: Vec<&[u8]> = if at == 0 { names.to_vec() } else { Vec::new() };
            appends.push(text.as_bytes());
            store.begin_hour(first + at as i32, &appends).unwrap();
            hour(first + at as i32, &mut store);
            store.end_hour(&mut memory).unwrap();
            if at % BLOCK == BLOCK - 1 {
                store.end_block();
            }
        }
        if !hours.is_multiple_of(BLOCK) {
            store.end_run(&mut memory).unwrap();
        }
        written.push(Run::new(first, memory.pages).unwrap());
        bytes = memory.bytes;
    }
    Arc::new(Recording { bytes, runs: written, kinds, format })
}

pub fn units(recording: Arc<Recording>, name: &str) -> Units {
    let kinds = Arc::new(recording.kinds.clone());
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: name.to_owned(), pages, timeline, kinds }
}

pub fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

pub fn sought(side: &mut MapSide<impl crate::playhead_host::PlaybackProducts>, hour: replay_data::Hour) {
    side.entry().playhead().seek(hour);
    for _ in 0..50_000_000u64 {
        let frame = side.frame(hour).unwrap();
        if frame.swapped.is_some() && !side.composing() && side.applied() == Some(hour) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the reset at hour {hour} never arrived");
}

pub fn play(side: &mut MapSide, from: replay_data::Hour, mut each: impl FnMut(&MapSide, &Frame) -> bool) {
    side.entry().playhead().play(4.0);
    let mut applied = from;
    for _ in 0..50_000_000u64 {
        let newest = side.entry().playhead().newest().unwrap_or(applied);
        let frame = side.frame(newest.min(applied + 4)).unwrap();
        if let Some(hour) = frame.applied.filter(|_| frame.steps > 0) {
            applied = hour;
            if each(side, &frame) {
                return;
            }
        } else {
            std::thread::yield_now();
        }
    }
    panic!("play never reached what the test waits for");
}

pub fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap_or_else(|| panic!("{table:?}.{field}"))
}

pub fn put(store: &mut Store, table: TableId, slot: u32, key: Option<u128>, cells: &[(&str, Cell<'_>)]) {
    let mut changes: TableChanges<'_> = store.table(table.index()).unwrap();
    if let Some(key) = key {
        changes.insert_at(slot, key).unwrap();
    }
    for &(field, cell) in cells {
        changes.write(slot, column(table, field), cell).unwrap();
    }
}

pub fn complete(store: &mut Store, at: u32, tables: &[TableId]) {
    for table in tables {
        store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    }
}
