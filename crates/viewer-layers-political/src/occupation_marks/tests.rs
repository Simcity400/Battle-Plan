
use super::{OccupationMarks, Source, drawn};
use crate::{left_to_layers::LeftToLayers, resource_icons::tests::{Memory, Recording, strings_of, write}};
use viewer_layer_core::{layers::{Inputs, Layer, View}, playhead_host::{MapSide, open, shared_maps_tests::Reads, units::Units}};
use content_reader_politics::occupation_modifiers::Definitions;
use playhead::{Settings, Timeline, pages::{Pages, Run}};
use replay_columnar::{manifest::Status, store::Store, unit::{Encoder, Settings as Writing}};
use replay_data::rows::{CountryLinkRow, CountryRow, Recorded, StateResistanceRow, StateRow, TableId, TableRows, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::sync::Arc;
use viewer_runtime::resource_origins::fixture::put;
use viewer_map_modes::map_mode::{ColorList, MapMode, Stock};
use viewer_map_icons::occupation_icons::{self as icons, IconDefines, hour_marks};

const TABLES: [TableId; 4] = [TableId::States, TableId::CountryLinks, TableId::Countries, TableId::StateResistance];

struct Content(icons::Content);

impl Source for Content {
    fn occupation(&self) -> Option<&icons::Content> {
        Some(&self.0)
    }
}

fn content() -> Arc<dyn Source> {
    Arc::new(Content(icons::Content {
        definitions: Definitions::default(),
        defines: IconDefines { cutoff: 900., badge_cutoff: 700., reenable: 2_000_000, priority: 30, resistance_colors: ColorList(vec![0., 1., 0., 0., 0., 100., 0., 1., 0., 0.75]), compliance_colors: ColorList(vec![0., 0., 0., 1., 0., 100., 1., 1., 1., 1.]) },
    }))
}

fn resistance(state: u32, level: i64) -> StateResistanceRow {
    StateResistanceRow { state, country: 30, level, speed: -5, compliance: 1_050_000, map: Recorded::NotRecorded, ..Default::default() }
}

fn campaign(name: &str) -> (Units, Vec<DecodedFrame>) {
    let mut first = DecodedFrame { branch: 0, hour: 0, metrics: Default::default(), tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()) };
    put(&mut first, TableRows::States(vec![StateRow { id: 1, controller: 20, ..Default::default() }, StateRow { id: 2, controller: 30, ..Default::default() }]));
    put(&mut first, TableRows::CountryLinks(vec![CountryLinkRow { id: 20, canonical_slot: 1 }, CountryLinkRow { id: 30, canonical_slot: 2 }]));
    put(&mut first, TableRows::Countries(vec![CountryRow { id: 20, canonical_slot: 1, ..Default::default() }, CountryRow { id: 30, canonical_slot: 2, ..Default::default() }]));
    put(&mut first, TableRows::StateResistance(vec![resistance(1, 3_000_000), resistance(2, 0)]));
    let strings = strings_of(&first, &TABLES);
    let mut frames = vec![first];
    let kinds = replay_recording::schema::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for hour in 0..40u32 {
        let names: Vec<&[u8]> = strings.iter().map(Vec::as_slice).collect();
        store.begin_hour(hour as i32, if hour == 0 { &names[..] } else { &[] }).unwrap();
        let mut frame = frames.last().unwrap().clone();
        frame.hour = hour as i32;
        match hour {
            0 => {
                for table in TABLES {
                    let TableView::Complete(rows) = &*frame.tables[table.index()] else { unreachable!() };
                    write(&mut store, &strings, rows);
                }
            }
            10 => {
                let rows = TableRows::StateResistance(vec![resistance(1, 4_500_000), resistance(2, 0)]);
                write(&mut store, &strings, &TableRows::StateResistance(vec![resistance(1, 4_500_000)]));
                put(&mut frame, rows);
            }
            20 => {
                write(&mut store, &strings, &TableRows::States(vec![StateRow { id: 2, controller: 20, ..Default::default() }]));
                put(&mut frame, TableRows::States(vec![StateRow { id: 1, controller: 20, ..Default::default() }, StateRow { id: 2, controller: 20, ..Default::default() }]));
            }
            _ => {}
        }
        for table in TABLES {
            store.status(table.index(), Status::Complete, true, Some(hour as i32), true).unwrap();
        }
        frames.push(frame);
        store.end_hour(&mut memory).unwrap();
    }
    store.end_run(&mut memory).unwrap();
    frames.remove(0);
    let run = Run::new(0, memory.pages).unwrap();
    let pages: Arc<dyn Pages> = Arc::new(Recording { bytes: memory.bytes, runs: vec![run], kinds: kinds.clone() });
    let timeline = Timeline::whole(&*pages).unwrap();
    (Units { name: name.into(), pages, timeline, kinds: Arc::new(kinds) }, frames)
}

fn resistance_mode() -> View {
    View { map_mode: MapMode::Stock(Stock::Resistance), ..View::default() }
}

fn layer(side: &MapSide) -> &OccupationMarks {
    side.parts().expect("composed").1[0].any().downcast_ref().expect("the layer")
}

#[test]
fn the_layer_equals_todays_marks_and_itself_composed_and_a_change_marks_only_its_state() {
    let (units, frames) = campaign("occupation marks, today's");
    let content = content();
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(replay_data::scope::CaptureMode::Full);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(OccupationMarks::new(Arc::clone(&content)))], resistance_mode()).unwrap();
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    let occupation = content.occupation().unwrap();
    let mut marked = layer(&side).marked();
    for hour in 1..26 {
        side.catch_up(hour).unwrap();
        let (mirrors, _) = side.parts().expect("composed");
        let ours = layer(&side).marks();
        let applied = hour as usize;
        assert_eq!(ours, hour_marks(&frames[applied], occupation), "today's at hour {hour}");
        let mut fresh = layer(&side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(ours, fresh.marks(), "composed at hour {hour}");
        let now = layer(&side).marked();
        assert_eq!(now - marked, u64::from(hour == 10 || hour == 20), "states marked by the step to hour {hour}");
        marked = now;
    }
    assert!(frames[3].complete::<tables::StateResistance>().is_ok());
}

#[test]
fn a_view_that_hides_the_layer_holds_and_carries_nothing_and_shown_again_composes() {
    let (units, frames) = campaign("occupation marks, hidden");
    let content = content();
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(replay_data::scope::CaptureMode::Full);
    let (states, _) = Reads::new(&[TableId::States]);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(OccupationMarks::new(Arc::clone(&content))), states], resistance_mode()).unwrap();
    side.anchor(0, false, 0.0, true);
    side.catch_up(5).unwrap();
    assert!(layer(&side).marks().is_ok_and(|marks| !marks.marks.is_empty()), "drawn in the resistance mode");
    side.set_view(View::default()).unwrap();
    side.catch_up(12).unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though a step changed its tables");
    assert_eq!(layer(&side).held(), OccupationMarks::new(Arc::clone(&content)).held(), "and holds what an empty layer holds");
    assert!(layer(&side).marks.is_empty() && layer(&side).viewers.is_empty(), "no mark is kept while hidden");
    let (mirrors, _) = side.parts().expect("composed");
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(&side).marks(), fresh.marks(), "composed anew, it is hidden too");
    side.set_view(resistance_mode()).unwrap();
    side.catch_up(22).unwrap();
    assert_eq!(layer(&side).marks(), hour_marks(&frames[22], content.occupation().unwrap()), "shown again, today's");
    let (mirrors, _) = side.parts().expect("composed");
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(&side).marks(), fresh.marks(), "shown again, itself composed");
}

#[test]
fn the_window_takes_the_layers_marks_where_it_draws_them_and_todays_where_it_does_not() {
    let (units, _) = campaign("occupation marks, drawn");
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(replay_data::scope::CaptureMode::Full);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(OccupationMarks::new(content()))], resistance_mode()).unwrap();
    assert!(drawn(side.layer_parts()).is_none(), "not composed yet: today's makes the marks");
    side.anchor(0, false, 0.0, true);
    side.catch_up(5).unwrap();
    let left = LeftToLayers::of(Some(side.layer_parts()));
    assert_eq!(left, LeftToLayers { resource_icons: false, occupation_marks: true, radar: left.radar }, "the layer draws the marks in the resistance mode");
    let version = drawn(side.layer_parts()).expect("drawn").version();
    side.catch_up(8).unwrap();
    assert_eq!(drawn(side.layer_parts()).expect("drawn").version(), version, "no step to hour 8 changed a mark: the window lays nothing out again");
    side.catch_up(12).unwrap();
    assert!(drawn(side.layer_parts()).expect("drawn").version() > version, "the step to hour 10 changed state 1's mark: the window lays it out again");
    side.set_view(View::default()).unwrap();
    side.catch_up(14).unwrap();
    let left = LeftToLayers::of(Some(side.layer_parts()));
    assert_eq!(left, LeftToLayers::default(), "in another mode the layer draws nothing");
    assert_eq!(LeftToLayers::of(None), LeftToLayers::default(), "the playhead off: today's makes both");
}

