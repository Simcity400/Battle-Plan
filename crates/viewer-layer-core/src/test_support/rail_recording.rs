use crate::{
    layers::{Layer, View},
    playhead_host::{Frame, MapSide, open, units::Units},
};
use playhead::{Settings, Timeline, pages::{Pages, Run}};
use replay_columnar::{
    state::Kind,
    store::Store,
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::TableId;
use std::{io, sync::Arc};

const RING: usize = 3;

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

pub fn recorded(hours: u32, mut strings: impl FnMut(u32) -> Vec<String>, mut hour: impl FnMut(u32, &mut Store)) -> Arc<dyn Pages> {
    let kinds = replay_recording::schema::schema().unwrap();
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

pub fn side_of(pages: Arc<dyn Pages>, layer: Box<dyn Layer>, name: &str) -> MapSide {
    side_at_pace(pages, layer, name, 4.0)
}

pub fn side_at_pace(pages: Arc<dyn Pages>, layer: Box<dyn Layer>, name: &str, speed: f64) -> MapSide {
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    let units = Units { name: name.to_owned(), pages, timeline, kinds };
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: RING, speed, rate: 1.0, ..Settings::default() };
    let entry = open(units, settings).unwrap();
    let side = MapSide::new(Arc::clone(&entry), vec![layer], View::default()).unwrap();
    entry.playhead().play(speed);
    entry.playhead().seek(0);
    side
}

pub fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &Frame) -> bool) {
    let mut applied: Option<i32> = None;
    for _ in 0..50_000_000u64 {
        crate::test_support::recorded::reads_landed(side);
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
