
use crate::playhead_host::{Frame, MapSide, units};
use playhead::{
    Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    state::Kind,
    store::Store,
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

pub fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap()
}

pub fn record(name: &str, hours: u32, campaign: impl FnMut(u32, &mut Store)) -> units::Units {
    record_naming(name, hours, &[], campaign)
}

pub fn record_naming(name: &str, hours: u32, texts: &[&[u8]], mut campaign: impl FnMut(u32, &mut Store)) -> units::Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        store.begin_hour(at as i32, if at == 0 { texts } else { &[] }).unwrap();
        campaign(at, &mut store);
        store.end_hour(&mut memory).unwrap();
    }
    assert!(hours < BLOCK);
    store.end_run(&mut memory).unwrap();
    let run = Run::new(0, memory.pages).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(Recording { bytes: memory.bytes, runs: vec![run], kinds: kinds.clone() });
    let timeline = Timeline::whole(&*pages).unwrap();
    units::Units { name: name.to_owned(), pages, timeline, kinds: Arc::new(kinds) }
}

pub fn frames_until(side: &mut MapSide, done: impl FnMut(&MapSide, &Frame) -> bool) {
    frames_until_named(side, |_| "what the test waits for".into(), done);
}

pub fn frames_until_named(side: &mut MapSide, what: impl Fn(&MapSide) -> String, mut done: impl FnMut(&MapSide, &Frame) -> bool) {
    let last = side.entry().playhead().timeline().last();
    let mut at_end = 0u64;
    loop {
        assert!(side.entry().playhead().error().is_none(), "the playhead stopped while waiting for {}: {:?}", what(side), side.entry().playhead().error());
        reads_landed(side);
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            if !side.composing() && side.applied().is_some_and(|applied| applied >= last) {
                let quiet = side.parts().is_some_and(|(_, layers)| {
                    layers.iter().all(|layer| matches!(layer.published(), crate::layers::Published::Current) && layer.carried().entities == 0)
                });
                at_end += 1;
                let limit = if quiet { 64 } else { 200_000 };
                assert!(at_end < limit, "the map side reached the last recorded hour ({last}) without reaching {}: {}", what(side), seen(side));
            } else {
                at_end = 0;
            }
            std::thread::yield_now();
        } else {
            at_end = 0;
        }
    }
}

pub fn reads_landed(side: &MapSide) {
    let playhead = side.entry().playhead();
    playhead
        .wait_for(|| {
            let (declared, counters) = (playhead.declarations(), playhead.held().2);
            counters.declarations_applied == declared && counters.joins_open == 0
        })
        .unwrap_or_else(|error| panic!("the playhead stopped while landing declarations: {error}; {}", seen(side)));
}

pub fn seen(side: &MapSide) -> String {
    let frame = side.last_frame();
    let layers: Vec<String> = side.parts().map_or_else(Vec::new, |(_, layers)| {
        layers.iter().zip(&frame.layers).map(|(layer, part)| format!("{}: carried {:?}, published {:?}, shown {:?}", layer.name(), layer.carried(), layer.published(), part.shown)).collect()
    });
    format!("applied {:?}, newest {:?}, composing {}; {layers:?}", side.applied(), side.entry().playhead().newest(), side.composing())
}

pub fn released_unit_by_unit(side: &mut MapSide, settles: crate::playhead_host::SettlesHeld) -> u64 {
    let carried = |side: &MapSide| side.parts().map_or(Vec::new(), |(_, layers)| layers.iter().map(|layer| layer.carried().entities).collect::<Vec<_>>());
    assert!(carried(side).iter().any(|&entities| entities > 0), "with the settles held, the steps' marks are carried: {}", seen(side));
    drop(settles);
    let hour = side.applied().expect("an hour applied");
    let frame = side.frame(hour).unwrap();
    assert_eq!(frame.steps, 0, "the head held at hour {hour}, the frame took no step: {}", seen(side));
    assert!(carried(side).iter().all(|&entities| entities == 0), "one frame settles every mark carried at hour {hour}: {}", seen(side));
    1
}

pub fn through(side: &mut MapSide, hour: i32) {
    frames_until(side, |_, frame| frame.applied.is_some_and(|applied| applied >= hour));
}
