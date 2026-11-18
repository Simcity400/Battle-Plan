//! A recorded campaign for the layers' tests: real tables written hour by hour into units in
//! memory, read by a playhead as the window's would be.

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

/// The index of `table`'s column `field` in the recording's schema.
pub fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap()
}

/// `hours` hours from game hour 0 in one run, each written by `campaign`, as a playhead's units.
pub fn record(name: &str, hours: u32, campaign: impl FnMut(u32, &mut Store)) -> units::Units {
    record_naming(name, hours, &[], campaign)
}

/// [`record`] with `texts` appended at hour 0, the dictionary ids 0 on that a text cell names.
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

/// Frames at the newest step built until `done` holds, yielding between frames that take nothing.
pub fn frames_until(side: &mut MapSide, done: impl FnMut(&MapSide, &Frame) -> bool) {
    frames_until_named(side, |_| "what the test waits for".into(), done);
}

/// [`frames_until`], its failure naming what it waited for as `what` words it from the map side.
/// Before each frame the playhead's reads have their turn ([`reads_landed`]).
pub fn frames_until_named(side: &mut MapSide, what: impl Fn(&MapSide) -> String, mut done: impl FnMut(&MapSide, &Frame) -> bool) {
    for _ in 0..50_000_000u64 {
        reads_landed(side);
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached {}: {}", what(side), seen(side));
}

/// The playhead's reads have their turn before the head moves on: once every declaration made (a
/// map composed, a view shown again, a mode changed) has reached the stepper and no table is still
/// being read from the file for it (`joins_open`), a frame may move the head. A table new to the
/// playhead is read on another thread, and a head driven to the newest step every frame with no
/// frame's time between them would otherwise pass hours, or the whole recording, before it lands:
/// in a window a frame takes 16 ms and the read lands within a few. Makes no frame, so a frame a
/// test looks for is never taken here. Returns at once when nothing is outstanding; fails by name
/// with the playhead's counters when they never land.
pub fn reads_landed(side: &MapSide) {
    let playhead = side.entry().playhead();
    for _ in 0..50_000_000u64 {
        let (declared, counters) = (playhead.declarations(), playhead.held().2);
        if counters.declarations_applied == declared && counters.joins_open == 0 {
            return;
        }
        std::thread::yield_now();
    }
    let (declared, counters) = (playhead.declarations(), playhead.held().2);
    panic!("the tables declared (declaration {declared}) were never read and landed: {counters:?}; {}", seen(side));
}

/// What a wait saw last: the map's hour, the playhead's newest and a compose, and each layer's
/// carried and published with the hour the last frame showed of it.
pub fn seen(side: &MapSide) -> String {
    let frame = side.last_frame();
    let layers: Vec<String> = side.parts().map_or_else(Vec::new, |(_, layers)| {
        layers.iter().zip(&frame.layers).map(|(layer, part)| format!("{}: carried {:?}, published {:?}, shown {:?}", layer.name(), layer.carried(), layer.published(), part.shown)).collect()
    });
    format!("applied {:?}, newest {:?}, composing {}; {layers:?}", side.applied(), side.entry().playhead().newest(), side.composing())
}

/// Frames until the step applying `hour` is taken; a frame takes every step built by then, so it
/// may pass `hour` by the ring's few steps, which a campaign's events are spaced beyond.
/// The settles held ([`MapSide::hold_settles`]) released at the hour applied, the head held there so
/// no step comes: every layer that settles with work carried is given one unit of it a frame
/// whatever the budget, until no layer carries any. A unit made can mark more of a later phase (a
/// supply row its strips), so what is carried need not fall in a frame: the units given are
/// counted instead. Returns the frames it took. Fails by name.
pub fn released_unit_by_unit(side: &mut MapSide, settles: crate::playhead_host::SettlesHeld) -> u64 {
    let carried = |side: &MapSide| side.parts().map_or(Vec::new(), |(_, layers)| layers.iter().map(|layer| layer.carried().entities).collect::<Vec<_>>());
    assert!(carried(side).iter().any(|&entities| entities > 0), "with the settles held, the steps' marks are carried: {}", seen(side));
    drop(settles);
    let hour = side.applied().expect("an hour applied");
    let mut frames = 0;
    while carried(side).iter().any(|&entities| entities > 0) {
        assert!(frames < 1_000_000, "a unit a frame at hour {hour} never settled the work carried: {}", seen(side));
        let before = carried(side);
        let frame = side.frame(hour).unwrap();
        assert_eq!(frame.steps, 0, "the head held at hour {hour}, the frame took no step: {}", seen(side));
        let after = carried(side);
        let (owed, owing) = (before.iter().sum::<usize>(), after.iter().sum::<usize>());
        for (((part, &entities), &left), name) in frame.layers.iter().zip(&before).zip(&after).zip(side.names()) {
            assert!(entities == 0 || part.given != Some(0) || left < entities, "released with no budget, {name} settled with {entities} carried, was given no unit and carries {left}: {}", seen(side));
        }
        assert!(frame.layers.iter().any(|part| part.given.is_some_and(|given| given > 0)) || owing < owed, "released with no budget, no layer was given a unit at hour {hour} ({owed} carried before, {owing} after): {}", seen(side));
        frames += 1;
    }
    frames
}

pub fn through(side: &mut MapSide, hour: i32) {
    frames_until(side, |_, frame| frame.applied.is_some_and(|applied| applied >= hour));
}
