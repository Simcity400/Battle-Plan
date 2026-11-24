
use super::{ResourceIcons, Source, drawn};
use crate::left_to_layers::LeftToLayers;
use viewer_layer_core::{layers::{Inputs, Layer, View}, playhead_host::{MapSide, open, shared_maps_tests::Reads, units::Units}};
use content_reader_localisation::localisation::Localisation;
use playhead::{
    Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, Value},
    unit::{BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::{Field, List, Opaque, ResourceRightRow, RowCodec, RowsVisit, StateResourceRow, TableId, TableRows, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{io, sync::Arc};
use viewer_foundation::test_support::localisation::loaded;
use viewer_runtime::resource_origins::{self, fixture};
use viewer_map_modes::map_mode::{MapMode, Stock};
use viewer_map_icons::resource_icons::hour_icons;

struct Content {
    origins: resource_origins::Content,
    localisation: Localisation,
}

impl Source for Content {
    fn origins(&self) -> Option<&resource_origins::Content> {
        Some(&self.origins)
    }

    fn localisation(&self) -> &Localisation {
        &self.localisation
    }
}

fn content() -> Arc<dyn Source> {
    Arc::new(Content {
        origins: fixture::content(),
        localisation: loaded(concat!(
            "l_english:\n",
            " MAP_ICON_NAME:0 \"$NAME$: $VAL$\"\n",
            " MAP_RESOURCE_ICON_VAL:0 \"$VAL$ of $MAX$\"\n",
            " PRODUCTION_MATERIALS_OIL:0 \"Fuel\"\n",
            " PRODUCTION_MATERIALS_STEEL:0 \"Metal\"\n",
            " PRODUCTION_MATERIALS_RUBBER:0 \"Gum\"\n",
        )),
    })
}

const TABLES: [TableId; 14] = [
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::ResourceOrigins,
    TableId::CountryOilFactors,
    TableId::StateBuildings,
    TableId::TradeRoutes,
    TableId::States,
    TableId::ResourceRights,
    TableId::StateTemporaryResources,
    TableId::StateResourceInputs,
    TableId::StateResources,
    TableId::StateBaseResources,
    TableId::CountryBuildingResources,
];

#[derive(Default)]
pub struct Memory {
    pub bytes: Vec<u8>,
    pub pages: Vec<BlockPage>,
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
    pub bytes: Vec<u8>,
    pub runs: Vec<Run>,
    pub kinds: Vec<Vec<Kind>>,
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

pub fn strings_of(frame: &DecodedFrame, tables: &[TableId]) -> Vec<Vec<u8>> {
    struct Texts<'a>(&'a mut Vec<Vec<u8>>);
    impl RowsVisit for Texts<'_> {
        type Output = ();
        fn rows<R: RowCodec>(self, rows: &[R]) {
            for row in rows {
                let _ = row.visit_wire(&mut |_, field| {
                    if let Field::Bytes(bytes) = field
                        && !self.0.iter().any(|held| held == bytes)
                    {
                        self.0.push(bytes.to_vec());
                    }
                    Ok(())
                });
            }
        }
    }
    let mut out = vec![Vec::new()];
    for &table in tables {
        if let TableView::Complete(rows) = &*frame.tables[table.index()] {
            rows.visit(Texts(&mut out));
        }
    }
    out
}

pub fn write(store: &mut Store, strings: &[Vec<u8>], rows: &TableRows) {
    struct Put<'a> {
        store: &'a mut Store,
        strings: &'a [Vec<u8>],
        table: TableId,
    }
    impl RowsVisit for Put<'_> {
        type Output = ();
        fn rows<R: RowCodec>(self, rows: &[R]) {
            let mut changes = self.store.table(self.table.index()).unwrap();
            let mut next = 0;
            for row in rows {
                let slot = match changes.slot(row.key()) {
                    Some(slot) => slot,
                    None => {
                        changes.insert_at(next, row.key()).unwrap();
                        next += 1;
                        next - 1
                    }
                };
                row.visit_wire(&mut |column, field| {
                    let cell = match field {
                        Field::U8(value) => Cell::U8(value),
                        Field::I8(value) => Cell::I8(value),
                        Field::U16(value) => Cell::U16(value),
                        Field::I16(value) => Cell::I16(value),
                        Field::U32(value) => Cell::U32(value),
                        Field::I32(value) => Cell::I32(value),
                        Field::U64(value) => Cell::U64(value),
                        Field::I64(value) => Cell::I64(value),
                        Field::Bool(value) => Cell::Bool(value),
                        Field::OptionalI32(value) => Cell::OptionalI32(value),
                        Field::OptionalU32(value) => Cell::OptionalU32(value),
                        Field::OptionalI64(value) => Cell::OptionalI64(value),
                        Field::OptionalU64(value) => Cell::OptionalU64(value),
                        Field::Bytes(bytes) => Cell::Bytes(self.strings.iter().position(|held| held == bytes).expect("a fixture text") as u32),
                        Field::U32List(List::Native(items)) => Cell::U32List(items),
                        Field::U64List(List::Native(items)) => Cell::U64List(items),
                        Field::Opaque(Opaque::Encoded(bytes, _)) => Cell::Opaque(bytes),
                        _ => return changes.put_value(slot, column, Value::Unrecorded),
                    };
                    changes.write(slot, column, cell)
                })
                .unwrap();
            }
        }
    }
    rows.visit(Put { store, strings, table: rows.table() });
}

fn rows_of(frame: &DecodedFrame, table: TableId) -> &TableRows {
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) => rows,
        _ => panic!("the fixture holds {table:?} whole"),
    }
}

fn campaign(name: &str, refuse: bool) -> (Units, Vec<DecodedFrame>) {
    let first = fixture::frame();
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
                    write(&mut store, &strings, rows_of(&frame, table));
                }
            }
            10 => {
                let mut stored = frame.complete::<tables::StateResources>().unwrap().to_vec();
                let raised = stored.iter_mut().find(|row| row.state == fixture::MIXED && row.resource == fixture::RUBBER).expect("MIXED's rubber");
                raised.pool += 300_000;
                let raised: StateResourceRow = raised.clone();
                write(&mut store, &strings, &TableRows::StateResources(vec![raised]));
                fixture::put(&mut frame, TableRows::StateResources(stored));
            }
            20 => {
                let mut rights: Vec<ResourceRightRow> = frame.complete::<tables::ResourceRights>().unwrap().to_vec();
                let at = rights.iter().position(|row| row.state == fixture::MIXED).expect("MIXED's rights");
                let gone = rights.remove(at);
                let mut changes = store.table(TableId::ResourceRights.index()).unwrap();
                let slot = changes.slot(gone.key()).unwrap();
                changes.delete(slot).unwrap();
                fixture::put(&mut frame, TableRows::ResourceRights(rights));
            }
            _ => {}
        }
        for table in TABLES {
            let status = if refuse && hour >= 30 && table == TableId::ResourceOrigins { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(hour as i32), true).unwrap();
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

fn resources() -> View {
    View { map_mode: MapMode::Stock(Stock::Resources), ..View::default() }
}

fn side(units: Units, content: Arc<dyn Source>) -> MapSide {
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(replay_data::scope::CaptureMode::Full);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(ResourceIcons::new(content))], resources()).unwrap();
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    side
}

fn layer(side: &MapSide) -> &ResourceIcons {
    side.parts().expect("composed").1[0].any().downcast_ref().expect("the layer")
}

#[test]
fn the_layer_equals_todays_icons_and_itself_composed_and_a_change_marks_only_its_state() {
    let (units, frames) = campaign("resource icons, today's", false);
    let content = content();
    let mut side = side(units, Arc::clone(&content));
    let origins = content.origins().unwrap();
    let mut marked = layer(&side).marked();
    for hour in 1..26 {
        side.catch_up(hour).unwrap();
        let (mirrors, _) = side.parts().expect("composed");
        let ours = layer(&side).icons();
        let applied = hour as usize;
        assert_eq!(ours, hour_icons(&frames[applied], origins, content.localisation()), "today's at hour {hour}");
        let mut fresh = layer(&side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(ours, fresh.icons(), "composed at hour {hour}");
        let now = layer(&side).marked();
        assert_eq!(now - marked, u64::from(hour == 10 || hour == 20), "states marked by the step to hour {hour}");
        marked = now;
    }
    let before = hour_icons(&frames[15], origins, content.localisation()).unwrap();
    let after = hour_icons(&frames[25], origins, content.localisation()).unwrap();
    assert_ne!(before.get(&fixture::MIXED), after.get(&fixture::MIXED), "the campaign changes MIXED's icon");
}

#[test]
fn a_table_not_recorded_refuses_every_icon_by_name() {
    let (units, _) = campaign("resource icons, refused", true);
    let mut side = side(units, content());
    for hour in 1..36 {
        side.catch_up(hour).unwrap();
    }
    side.settle_all().unwrap();
    let status = side.parts().expect("composed").0.get(TableId::ResourceOrigins).and_then(viewer_layer_core::playhead_host::mirror::Mirror::status);
    let refused = layer(&side).icons().map(|icons| icons.len()).expect_err(&format!("ResourceOrigins at the head: {status:?}"));
    assert!(refused.contains("ResourceOrigins is not recorded"), "{refused}");
}

#[test]
fn hidden_while_another_layer_holds_its_states_it_holds_and_carries_nothing_and_shown_again_composes() {
    let (units, frames) = campaign("resource icons, hidden", false);
    let content = content();
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(replay_data::scope::CaptureMode::Full);
    let (states, _) = Reads::new(&[TableId::States]);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(ResourceIcons::new(Arc::clone(&content))), states], View::default()).unwrap();
    side.anchor(0, false, 0.0, true);
    side.catch_up(12).unwrap();
    assert!(side.parts().expect("composed").0.get(TableId::States).is_some(), "another layer keeps States in the mirrors");
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though a step changed its tables");
    assert_eq!(layer(&side).held(), ResourceIcons::new(Arc::clone(&content)).held(), "and holds what an empty layer holds");
    assert!(layer(&side).icons.is_empty(), "no icon is kept while hidden");
    let (mirrors, _) = side.parts().expect("composed");
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(&side).icons(), fresh.icons(), "composed anew, it is hidden too");
    assert_eq!(layer(&side).icons(), Err(super::NOT_DRAWN.to_owned()));
    side.set_view(resources()).unwrap();
    side.catch_up(22).unwrap();
    let origins = content.origins().unwrap();
    assert_eq!(layer(&side).icons(), hour_icons(&frames[22], origins, content.localisation()), "shown again, today's");
    let (mirrors, _) = side.parts().expect("composed");
    let mut fresh = layer(&side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(&side).icons(), fresh.icons(), "shown again, itself composed");
}

#[test]
fn the_window_takes_the_layers_icons_in_the_resources_mode_and_todays_where_it_does_not_draw() {
    let (units, _) = campaign("resource icons, drawn", false);
    let mut side = side(units, content());
    side.catch_up(5).unwrap();
    let left = LeftToLayers::of(Some(side.layer_parts()));
    assert_eq!(left, LeftToLayers { resource_icons: true, occupation_marks: false, radar: left.radar }, "the layer draws the icons in the resources mode");
    let version = drawn(side.layer_parts()).expect("drawn").version();
    side.catch_up(8).unwrap();
    assert_eq!(drawn(side.layer_parts()).expect("drawn").version(), version, "no step to hour 8 changed an icon: the window lays nothing out again");
    side.catch_up(22).unwrap();
    assert!(drawn(side.layer_parts()).expect("drawn").version() > version, "the step to hour 20 changed MIXED's icon: the window lays it out again");
    side.set_view(View::default()).unwrap();
    side.catch_up(24).unwrap();
    let left = LeftToLayers::of(Some(side.layer_parts()));
    assert_eq!(left, LeftToLayers::default(), "in another mode the layer draws nothing");
}

