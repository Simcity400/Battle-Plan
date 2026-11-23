
use crate::playhead_host::{Frame, MapSide, units};
use playhead::{
    Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    state::Kind,
    store::Store,
    unit::{BLOCK, BlockPage, TableEntry, Encoder, PartSink, Settings as Writing, reader::Extents},
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
    let kinds = replay_recording::schema::schema().unwrap();
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

pub fn frames_until<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, done: impl FnMut(&MapSide<P>, &Frame) -> bool) {
    frames_until_named(side, |_| "what the test waits for".into(), done);
}

pub fn frames_until_named<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, what: impl Fn(&MapSide<P>) -> String, mut done: impl FnMut(&MapSide<P>, &Frame) -> bool) {
    let last = side.entry().playhead().timeline().last();
    let carried = |side: &MapSide<P>| side.parts().map_or(0, |(_, layers)| layers.iter().map(|layer| layer.carried().entities).sum::<usize>());
    loop {
        assert!(side.entry().playhead().error().is_none(), "the playhead stopped while waiting for {}: {:?}", what(side), side.entry().playhead().error());
        reads_landed(side);
        let before = side.entry().playhead().step_state();
        let carried_before = carried(side);
        let head = before.0.unwrap_or(0);
        let frame = if side.composing() { side.frame_waiting(head) } else { side.frame_synced(head) }.unwrap();
        if done(side, &frame) {
            return;
        }
        if side.composing() || frame.steps > 0 || frame.swapped.is_some() || frame.superseded || frame.exact || carried(side) < carried_before {
            continue;
        }
        let more = side
            .wait_step_after(before, last)
            .unwrap_or_else(|error| panic!("the playhead stopped while waiting for {}: {error}; {}", what(side), seen(side)));
        assert!(more, "the map side reached the last recorded hour ({last}) without reaching {}: {}", what(side), seen(side));
    }
}

pub fn frames_at<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, hour: replay_data::Hour, what: &str, done: impl FnMut(&MapSide<P>, &Frame) -> bool) {
    frames_at_named(side, hour, |_| what.to_owned(), done);
}

pub fn frames_at_named<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, hour: replay_data::Hour, what: impl Fn(&MapSide<P>) -> String, mut done: impl FnMut(&MapSide<P>, &Frame) -> bool) {
    let carried = |side: &MapSide<P>| side.parts().map_or(0, |(_, layers)| layers.iter().map(|layer| layer.carried().entities).sum::<usize>());
    loop {
        assert!(side.entry().playhead().error().is_none(), "the playhead stopped while waiting for {}: {:?}", what(side), side.entry().playhead().error());
        reads_landed(side);
        let before = side.entry().playhead().step_state();
        let carried_before = carried(side);
        let frame = if side.composing() { side.frame_waiting(hour) } else { side.frame_synced(hour) }.unwrap();
        if done(side, &frame) {
            return;
        }
        if side.composing() || frame.steps > 0 || frame.swapped.is_some() || frame.superseded || frame.exact || carried(side) < carried_before {
            continue;
        }
        let more = side
            .wait_step_after(before, hour)
            .unwrap_or_else(|error| panic!("the playhead stopped while waiting for {}: {error}; {}", what(side), seen(side)));
        assert!(more, "the map side reached hour {hour} without reaching {}: {}", what(side), seen(side));
    }
}

pub fn reads_landed<P: crate::playhead_host::PlaybackProducts>(side: &MapSide<P>) {
    side.entry()
        .playhead()
        .wait_declarations()
        .unwrap_or_else(|error| panic!("the playhead stopped while landing declarations: {error}; {}", seen(side)));
}

pub fn seen<P: crate::playhead_host::PlaybackProducts>(side: &MapSide<P>) -> String {
    let frame = side.last_frame();
    let layers: Vec<String> = side.parts().map_or_else(Vec::new, |(_, layers)| {
        layers.iter().zip(&frame.layers).map(|(layer, part)| format!("{}: carried {:?}, published {:?}, shown {:?}", layer.name(), layer.carried(), layer.published(), part.shown)).collect()
    });
    format!("applied {:?}, newest {:?}, composing {}; {layers:?}", side.applied(), side.entry().playhead().newest(), side.composing())
}

pub fn released_unit_by_unit<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, settles: crate::playhead_host::SettlesHeld) -> u64 {
    let carried = |side: &MapSide<P>| side.parts().map_or(Vec::new(), |(_, layers)| layers.iter().map(|layer| layer.carried().entities).collect::<Vec<_>>());
    assert!(carried(side).iter().any(|&entities| entities > 0), "with the settles held, the steps' marks are carried: {}", seen(side));
    drop(settles);
    let hour = side.applied().expect("an hour applied");
    let frame = side.frame(hour).unwrap();
    assert_eq!(frame.steps, 0, "the head held at hour {hour}, the frame took no step: {}", seen(side));
    assert!(carried(side).iter().all(|&entities| entities == 0), "one frame settles every mark carried at hour {hour}: {}", seen(side));
    1
}

pub fn through<P: crate::playhead_host::PlaybackProducts>(side: &mut MapSide<P>, hour: i32) {
    frames_until(side, |_, frame| frame.applied.is_some_and(|applied| applied >= hour));
}

pub fn without(units: units::Units, table: TableId) -> units::Units {
    struct Without {
        inner: Arc<dyn Pages>,
        runs: Vec<Run>,
    }
    impl Pages for Without {
        fn extents(&self, run: usize) -> &dyn Extents {
            self.inner.extents(run)
        }
        fn runs(&self) -> &[Run] {
            &self.runs
        }
        fn kinds(&self, table: usize) -> Option<&[Kind]> {
            self.inner.kinds(table)
        }
    }
    let runs = units
        .pages
        .runs()
        .iter()
        .map(|run| {
            let mut pages = run.pages.clone();
            for page in &mut pages {
                if let Some(entry) = page.tables.get_mut(table.index()) {
                    *entry = TableEntry::default();
                }
            }
            Run::new(run.first_hour, pages).unwrap()
        })
        .collect();
    let pages: Arc<dyn Pages> = Arc::new(Without { inner: units.pages, runs });
    let timeline = Timeline::whole(&*pages).unwrap();
    units::Units { name: units.name, pages, timeline, kinds: units.kinds }
}
