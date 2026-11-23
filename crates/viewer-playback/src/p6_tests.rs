
use viewer_layer_core::{layers::{Layer, View}, vision::Vision};
use viewer_layers_infrastructure::weather::Weather;
use viewer_layer_core::test_support::vision_recording::{column, play, play_all, quick, record, vision_content, vision_world, vision_world_and};
use viewer_layer_core::playhead_host::{MapSide, mirror::Mirrors, open, units::Units};
use replay_columnar::{
    manifest::Status,
    state::Cell,
    store::Store,
};
use replay_data::rows::TableId;
use viewer_gpu::camera::{MapDefines, MapSize};
use std::{
    collections::BTreeMap,
    io,
    sync::Arc,
    time::{Duration, Instant},
};

fn kind(flags: [u8; 5]) -> Option<usize> {
    flags.iter().position(|&flag| flag != 0)
}

#[test]
fn weather_regions_are_marked_once_settle_to_their_newest_flags_and_none_draw_while_not_recorded() {
    let table = TableId::WeatherRegions;
    let names = ["light_rain", "heavy_rain", "snow", "blizzard", "sandstorm"];
    let columns: Vec<usize> = names.iter().map(|name| column(table, name)).collect();
    let mut truth: BTreeMap<u32, [u8; 5]> = BTreeMap::new();
    let mut history: BTreeMap<u32, BTreeMap<u32, [u8; 5]>> = BTreeMap::new();
    let mut gone = None;
    let units = record(40, &[table], |at, store| {
        let mut changes = store.table(table.index()).unwrap();
        let mut put = |changes: &mut replay_columnar::store::TableChanges<'_>, slot: u32, id: u32, flags: [u8; 5]| {
            for (&column, &flag) in columns.iter().zip(&flags) {
                changes.write(slot, column, Cell::U8(flag)).unwrap();
            }
            truth.insert(id, flags);
        };
        match at {
            0 => {
                for (slot, id) in (1..=6u32).enumerate() {
                    changes.insert_at(slot as u32, u128::from(id)).unwrap();
                    let mut flags = [0; 5];
                    flags[id as usize % 5] = 1;
                    put(&mut changes, slot as u32, id, flags);
                }
            }
            5 => put(&mut changes, 1, 2, [0, 0, 0, 1, 0]),
            6 => put(&mut changes, 2, 3, [1, 0, 0, 0, 0]),
            7 => put(&mut changes, 2, 3, [0, 0, 1, 1, 0]),
            9 => {
                changes.delete(3).unwrap();
                gone = Some(4);
                changes.insert_at(6, 7).unwrap();
                put(&mut changes, 6, 7, [0, 1, 0, 0, 0]);
            }
            _ => {}
        }
        if let Some(id) = gone.take() {
            truth.remove(&id);
        }
        history.insert(at, truth.clone());
        if (16..24).contains(&at) { vec![(table, Status::NotRecorded)] } else { Vec::new() }
    });
    let mut checked = 0;
    let (mut marked_before, mut previous) = (0, None);
    play(units, Box::new(Weather::new()), 36, |hour, mirrors, layer| {
        let layer = layer.any().downcast_ref::<Weather>().unwrap();
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.kinds(), fresh.kinds(), "stepped equals composed at hour {hour}");
        let expected: BTreeMap<u32, usize> =
            if (16..24).contains(&hour) { BTreeMap::new() } else { history[&(hour as u32)].iter().filter_map(|(&id, &flags)| kind(flags).map(|kind| (id, kind))).collect() };
        assert_eq!(layer.kinds(), expected, "today's kinds at hour {hour}");
        if hour == 7 && previous == Some(3) {
            assert_eq!(layer.marked() - marked_before, 2, "regions 2 and 3, each once");
        }
        (marked_before, previous) = (layer.marked(), Some(hour));
        checked += 1;
    });
    assert!(checked >= 5);
}

#[test]
fn a_moving_division_and_a_province_changing_hands_refold_only_what_they_touch_and_equal_the_composed_vision() {
    let mut checked = 0;
    let mut refolded = BTreeMap::new();
    let mut published = BTreeMap::new();
    play(vision_world(), Box::new(Vision::new(vision_content())), 40, |hour, mirrors, layer| {
        let changed = layer.changed().expect("vision publishes its changes").read();
        published.insert(hour, match changed {
            viewer_layer_core::layers::Change::Whole => (true, Vec::new()),
            viewer_layer_core::layers::Change::Marked(marked) => (false, marked.of("province").to_vec()),
        });
        let layer = layer.any().downcast_ref::<Vision>().unwrap();
        refolded.insert(hour, layer.marked()[1]);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        let (ours, composed) = (layer.vision().expect("the layer draws"), fresh.vision().expect("composed draws"));
        assert_eq!(ours.differences(&composed), (0, Vec::new()), "stepped equals composed at hour {hour}");
        assert_eq!(ours.ours(), 1);
        let notes = layer.notes().expect("notes while it draws");
        assert_eq!(Some(notes), fresh.notes(), "the notes stepped equal those composed at hour {hour}");
        let fog = if (20..24).contains(&hour) { "off, the game's fow console switch" } else { "on, so a stack text 0 has no intelligence" };
        assert!(notes.perspective().len() == 1 && notes.perspective()[0].starts_with(&format!("Viewing country text 0: the recording client's stored viewpoint; fog of war {fog}")), "the viewing country's line at hour {hour}: {:?}", notes.perspective());
        let mut truncated = viewer_foundation::tables::Limitations::default();
        for table in ours.truncated() {
            map_products::note_truncated_perspective(&mut truncated, table);
        }
        assert_eq!(notes.truncated(), truncated.into_lines(), "today's lines of the tables truncated at hour {hour}");
        assert_eq!(notes.truncated().is_empty(), hour < 25, "CountryIntel truncated from hour 25, stated with fog on, at hour {hour}");
        assert_eq!(notes.fog_drawn(), ours.shared_province_fog().is_some(), "the fog of war's line owed at hour {hour}");
        for owner in 0..=4 {
            assert_eq!(layer.shares_vision_proven(owner), Some(ours.shares_vision_proven(owner)), "shared vision of {owner} at hour {hour}");
            for unit in [0, 900, 901, 902] {
                for province in 1..=7 {
                    assert_eq!(layer.sees_land(unit, owner, province), Some(ours.sees_land(unit, owner, province)), "unit {unit} of {owner} in {province} at hour {hour}");
                }
            }
        }
        if (20..24).contains(&hour) {
            assert!(!ours.fog() && ours.province_fog().is_none(), "a fog input not recorded draws no fog");
            assert_eq!(ours.undecided(), None, "with fog off nothing is undecided, as today");
            checked += 1;
            return;
        }
        assert_eq!(ours.undecided(), None);
        let bytes: Vec<u8> = (1..=7).map(|id| ours.province_fog().expect("fog drawn").get(&id).copied().unwrap_or(0)).collect();
        let expected: &[u8] = match hour {
            0..=4 => &[255, 255, 255, 250, 0, 0, 0],
            5..=8 => &[255, 255, 255, 255, 0, 0, 0],
            9..=12 => &[255, 255, 255, 150, 255, 0, 0],
            13..=25 => &[255, 255, 255, 255, 255, 0, 0],
            _ => &[255, 255, 255, 255, 255, 255, 0],
        };
        assert_eq!(bytes, expected, "province bytes at hour {hour}");
        checked += 1;
    });
    assert!(checked >= 4);
    let quiet: Vec<(i32, i32)> = refolded.keys().zip(refolded.keys().skip(1)).map(|(&a, &b)| (a, b)).filter(|&(a, _)| a >= 26).collect();
    assert!(!quiet.is_empty(), "no step after hour 26 was seen: {refolded:?}");
    for &(a, b) in &quiet {
        assert_eq!(refolded[&b], refolded[&a], "hours {a} to {b}: provinces refolded {} then {}; a truncated census whose missing count moves refolds nothing: {refolded:?}", refolded[&a], refolded[&b]);
        assert_eq!(published[&b], (false, Vec::new()), "nothing is published after the last change: {published:?}");
    }
    let (&hour, (all, provinces)) = published.range(13..).next().expect("a step at hour 13 or after");
    assert!(!all && provinces.contains(&4), "hour {hour} publishes province 4 alone: {published:?}");
}

#[test]
fn vision_applies_mark_their_intel_rows_reading_none_and_settle_to_the_composed_vision() {
    let units = vision_world_and(&[], |at, store| {
        let write = |store: &mut Store, table: TableId, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(0, column(table, field), cell).unwrap();
        match at {
            2 => {
                store.table(TableId::UnitIntel.index()).unwrap().insert_at(0, 900 << 32).unwrap();
                write(store, TableId::UnitIntel, "viewer", Cell::U32(102));
                write(store, TableId::UnitIntel, "level", Cell::I8(3));
            }
            3 => {
                store.table(TableId::CountryIntel.index()).unwrap().insert_at(0, 101 << 32 | 2).unwrap();
                write(store, TableId::CountryIntel, "air", Cell::I64(5));
                write(store, TableId::CountryIntel, "navy", Cell::I64(6));
            }
            6 => write(store, TableId::UnitIntel, "level", Cell::I8(1)),
            7 => write(store, TableId::CountryIntel, "air", Cell::I64(7)),
            10 => store.table(TableId::UnitIntel.index()).unwrap().delete(0).unwrap(),
            _ => {}
        }
        Vec::new()
    });
    let mut checked = 0;
    play(units, Box::new(Vision::new(vision_content())), 14, |hour, mirrors, layer| {
        let layer = layer.any().downcast_ref::<Vision>().unwrap();
        assert_eq!(layer.applied_reads(), 0, "the applies to hour {hour} read no row");
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        let (ours, composed) = (layer.vision().expect("the layer draws"), fresh.vision().expect("composed draws"));
        assert_eq!(ours.differences(&composed), (0, Vec::new()), "stepped equals composed at hour {hour}");
        for owner in [101, 102] {
            for unit in [900, 901] {
                for province in 1..=7 {
                    assert_eq!(layer.sees_land(unit, owner, province), fresh.sees_land(unit, owner, province), "unit {unit} of {owner} in {province} at hour {hour}");
                }
            }
        }
        checked += 1;
    });
    assert!(checked >= 4);
}

#[test]
fn snow_cells_follow_their_provinces_and_go_unknown_where_a_truncated_or_unrecorded_table_cannot_say() {
    use viewer_layers_infrastructure::snow::{self, Snow};
    use replay_data::rows::{TableRows, WeatherProvinceRow};
    use replay_frame::frame::{DecodedFrame, TableView};
    let ids: Vec<u32> = (0..144u32).map(|at| match at / 48 {
        0 => 30,
        1 if at % 12 >= 8 => 21,
        1 => 20,
        _ => 10,
    }).collect();
    let map = content_reader_map::map::MapData { width: 12, height: 12, province_ids: ids, definitions: Default::default(), country_colors: Default::default() };
    let cells = map_snow::SnowCells::new(&map, &[false; 144]).unwrap();
    let winter = map_snow::winter::Winter { defines: map_snow::winter::SnowDefines { per_province: [0.0, 1.0], total: [0.0, 1.0] }, records: vec![vec![map_snow::winter::SnowRecord { province: 20, weight: 1.0 }]] };
    let table = TableId::WeatherProvinces;
    let [snow_column, display_snow, display_mud] = ["snow", "display_snow", "display_mud"].map(|name| column(table, name));
    let status = |at: u32| match at {
        12..16 => Status::Truncated { missing: 1 },
        16..20 => Status::NotRecorded,
        _ => Status::Complete,
    };
    let mut truth: BTreeMap<u32, (i64, u8, u8)> = BTreeMap::new();
    let mut history = BTreeMap::new();
    let units = record(28, &[table], |at, store| {
        let mut changes = store.table(table.index()).unwrap();
        let put = |changes: &mut replay_columnar::store::TableChanges<'_>, truth: &mut BTreeMap<u32, (i64, u8, u8)>, slot: u32, id: u32, row: (i64, u8, u8)| {
            changes.write(slot, snow_column, Cell::I64(row.0)).unwrap();
            changes.write(slot, display_snow, Cell::U8(row.1)).unwrap();
            changes.write(slot, display_mud, Cell::U8(row.2)).unwrap();
            truth.insert(id, row);
        };
        match at {
            0 => {
                for (slot, id) in [10u32, 20, 21, 30].into_iter().enumerate() {
                    changes.insert_at(slot as u32, u128::from(id)).unwrap();
                    put(&mut changes, &mut truth, slot as u32, id, (i64::from(id) * 1000, id as u8 * 5, id as u8));
                }
            }
            5 => put(&mut changes, &mut truth, 1, 20, (90_000, 240, 7)),
            6 => put(&mut changes, &mut truth, 2, 21, (1, 2, 3)),
            7 => put(&mut changes, &mut truth, 2, 21, (4, 200, 6)),
            13 => {
                changes.delete(1).unwrap();
                truth.remove(&20);
            }
            22 => {
                changes.insert_at(1, 20).unwrap();
                put(&mut changes, &mut truth, 1, 20, (50_000, 100, 9));
            }
            _ => {}
        }
        history.insert(at, truth.clone());
        vec![(table, status(at))]
    });
    let content = Arc::new(snow::Content::new(cells.clone(), winter.clone(), MapSize { width: 64, height: 64 }, MapDefines::test()));
    let mut checked = 0;
    play(units, Box::new(Snow::new(Ok(content))), 26, |hour, mirrors, layer| {
        let layer = layer.any().downcast_ref::<Snow>().unwrap();
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!((layer.texels(), layer.unknown(), layer.summary()), (fresh.texels(), fresh.unknown(), fresh.summary()), "stepped equals composed at hour {hour}");
        let rows: Vec<WeatherProvinceRow> = history[&(hour as u32)].iter().map(|(&id, &(snow, ds, dm))| WeatherProvinceRow { id, snow, display_snow: ds, display_mud: dm, ..Default::default() }).collect();
        let view = match status(hour as u32) {
            Status::Complete => TableView::Complete(TableRows::WeatherProvinces(rows)),
            Status::Truncated { missing } => TableView::Truncated { rows: TableRows::WeatherProvinces(rows), missing },
            _ => TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full },
        };
        let mut tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|_| TableView::Unavailable { reason: "not needed".into() });
        tables[table.index()] = view;
        let frame = DecodedFrame { branch: 1, hour, tables: tables.map(Into::into), metrics: Default::default() };
        let today = map_snow::build(&cells, &frame, &winter).unwrap();
        let texels: Vec<u8> = layer.texels().iter().flatten().copied().collect();
        assert_eq!(texels, today.texels.to_vec(), "texels as today at hour {hour}");
        assert_eq!(layer.unknown(), &today.unknown.to_vec()[..], "unknown cells as today at hour {hour}");
        assert_eq!(layer.summary(), Some(today.summary), "summary as today at hour {hour}");
        assert_eq!(layer.province_snow().collect::<Vec<_>>(), today.province_snow.to_vec(), "snow words as today at hour {hour}");
        assert_eq!(layer.factors().iter().map(|factor| factor.map(f32::to_bits)).collect::<Vec<_>>(), today.post_factors.unwrap().iter().map(|factor| factor.map(f32::to_bits)).collect::<Vec<_>>());
        checked += 1;
    });
    assert!(checked >= 4);
}

#[test]
fn the_intel_texture_steps_with_the_vision_it_reads_and_equals_itself_composed() {
    use viewer_layers_infrastructure::intel::{self, Intel};
    use content_reader_map::map::{MapData, ProvinceDefinition, ProvinceKind};
    let ids: Vec<u32> = (0..42 * 6u32).map(|at| at % 42 / 6 + 1).collect();
    let definitions = (1..=7u32).map(|id| (id, ProvinceDefinition { id, rgb: [0; 3], kind: if id == 7 { ProvinceKind::Sea } else { ProvinceKind::Land }, continent: 0 })).collect();
    let map = MapData { width: 42, height: 6, province_ids: ids, definitions, country_colors: Default::default() };
    let cells = viewer_gpu::textures::IntelCells::new(&map).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Vision::new(vision_content())), Box::new(Intel::new(Ok(Arc::new(intel::Content::new(cells, 7)))))];
    let mut checked = 0;
    play_all(vision_world(), layers, 40, |hour, _mirrors, layers| {
        let vision = layers[0].any().downcast_ref::<Vision>().unwrap();
        let layer = layers[1].any().downcast_ref::<Intel>().unwrap();
        let composed = layer.composed();
        assert_eq!(layer.map(), composed.map(), "stepped equals composed at hour {hour}");
        let (texels, drawn, _) = layer.map();
        assert_eq!(drawn, vision.fog_drawn(), "a fog is drawn as the vision says at hour {hour}");
        let expected: Vec<u8> = (1..=7u32).map(|province| if !drawn || province == 7 { 0 } else { u8::MAX - vision.intel_byte(province) }).collect();
        assert_eq!(texels, &expected[..], "each cell the complement of its province's byte at hour {hour}");
        checked += 1;
    });
    assert!(checked >= 4);
}

struct Asked(std::sync::Mutex<Vec<u32>>);

impl viewer_layers_infrastructure::trains::Yard for Asked {
    fn cutoff(&self) -> f32 {
        300.0
    }

    fn plan(&self, _rows: &viewer_layers_infrastructure::trains::PlanRows<'_>, ours: u32, _curve: &mut dyn FnMut(&[u32]) -> io::Result<viewer_rail::trains::Curve>, _cache: &mut viewer_rail::trains::PlanCache) -> io::Result<viewer_rail::trains::TrainPlan> {
        self.0.lock().unwrap().push(ours);
        Ok(viewer_rail::trains::TrainPlan::default())
    }

    fn curve(&self, _path: &[u32], _network: &viewer_rail::railways::RailNetwork) -> viewer_rail::trains::Curve {
        None
    }
}

fn viewing_world(asked: &Arc<Asked>) -> (Arc<viewer_layer_core::playhead_host::Entry>, Vec<Box<dyn Layer>>) {
    viewing_world_and(asked, |_, _| Vec::new())
}

fn viewing_world_and(asked: &Arc<Asked>, more: impl FnMut(u32, &mut Store) -> Vec<(TableId, Status)>) -> (Arc<viewer_layer_core::playhead_host::Entry>, Vec<Box<dyn Layer>>) {
    use viewer_layer_core::engagements::Engagements;
    use viewer_layers_political::labels::Labels;
    use viewer_layers_infrastructure::{rails::{Rails, test_support::Stub}, trains::Trains};
    let extra = [
        TableId::Politics, TableId::CountryAppearance, TableId::ProvinceNames, TableId::LexicalTokens, TableId::SupplyNodes, TableId::TrainInventory,
        TableId::VariantDefinitions, TableId::RailEdges, TableId::RailCooldowns, TableId::ProvinceSupply, TableId::NavalBases,
    ];
    let layers: Vec<Box<dyn Layer>> = vec![
        Box::new(Vision::new(vision_content())),
        Box::new(Engagements::new(None, None).reading_vision()),
        Box::new(Rails::new(Arc::new(Stub))),
        Box::new(Trains::new(Arc::clone(asked) as Arc<dyn viewer_layers_infrastructure::trains::Yard>, None).reading_vision()),
        Box::new(Labels::new(Arc::new(content_reader_localisation::localisation::Localisation::default()), None).reading_vision()),
    ];
    (open(vision_world_and(&extra, more), quick()).unwrap(), layers)
}

#[test]
fn a_viewing_country_change_reaches_engagements_trains_and_labels_through_vision_as_a_reader_opened_on_it() {
    use viewer_layer_core::{layers::Upstream, engagements::Engagements};
    use viewer_layers_political::labels::Labels;
    use viewer_layers_infrastructure::trains::Trains;
    use viewer_foundation::perspective::{Choice, Country};
    const HOUR: i32 = 8;
    let asked = Arc::new(Asked(std::sync::Mutex::new(Vec::new())));
    let (entry, layers) = viewing_world(&asked);
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    side.anchor(HOUR, false, 0.0, true);
    let shows = |side: &MapSide, slot: u32| {
        side.applied() == Some(HOUR) && !side.composing() && side.parts().is_some_and(|(_, layers)| {
            layers.iter().all(|layer| layer.carried().entities == 0)
                && layers[0].any().downcast_ref::<Vision>().unwrap().ours() == Some(slot)
                && layers[3].any().downcast_ref::<Trains>().unwrap().ours() == Some(slot)
                && layers[4].any().downcast_ref::<Labels>().unwrap().ours() == Some(slot)
                && layers[1].any().downcast_ref::<Engagements>().unwrap().viewer() == Some(slot)
        })
    };
    let settled = |side: &mut MapSide, slot: u32, what: &str| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            let frame = side.frame(HOUR).unwrap();
            if shows(side, slot) {
                return;
            }
            assert!(std::time::Instant::now() < deadline, "{what} within 120 s: applied {:?}, composing {}", side.applied(), side.composing());
            if frame.steps == 0 && frame.swapped.is_none() {
                std::thread::yield_now();
            }
        }
    };
    let opened_on = |side: &MapSide, slot: u32, tag: &str| {
        let (mirrors, layers) = side.parts().unwrap();
        let trains = layers[3].any().downcast_ref::<Trains>().unwrap();
        let mut fresh = trains.fresh();
        fresh.compose_with(mirrors, &Upstream::new(&layers[..3])).unwrap();
        assert_eq!(trains.differ(&fresh), (0, None), "the trains equal a reader opened on slot {slot}");
        assert_eq!(asked.0.lock().unwrap().last(), Some(&slot), "the trains' last plan is slot {slot}'s");
        let labels = layers[4].any().downcast_ref::<Labels>().unwrap();
        let mut fresh = labels.fresh();
        fresh.compose_with(mirrors, &Upstream::new(&layers[..4])).unwrap();
        assert_eq!(labels.differ(&fresh), (0, None), "the labels equal a reader opened on slot {slot}");
        assert_eq!(labels.display().perspective.as_deref(), Some(tag), "the labels' perspective is slot {slot}'s tag");
    };
    settled(&mut side, 1, "the recorded viewer, slot 1, reaching every layer");
    opened_on(&side, 1, "text 0");
    side.set_view(View { perspective: Choice { country: Country::Tag("text 1".into()), ..Choice::default() }, ..View::default() }).unwrap();
    settled(&mut side, 2, "the chosen country, slot 2, reaching every layer");
    opened_on(&side, 2, "text 1");
}

#[test]
fn a_compose_answers_the_windows_vision_questions_without_its_layers() {
    const HOUR: i32 = 8;
    const SOUGHT: i32 = 30;
    let asked = Arc::new(Asked(std::sync::Mutex::new(Vec::new())));
    let (entry, layers) = viewing_world(&asked);
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    side.anchor(HOUR, false, 0.0, true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !(side.applied() == Some(HOUR) && !side.composing() && side.viewer() == Some(1)) {
        side.frame(HOUR).unwrap();
        assert!(std::time::Instant::now() < deadline, "the recorded viewer, slot 1, within 120 s");
        std::thread::yield_now();
    }
    side.anchor(SOUGHT, false, 0.0, true);
    let mut asked_while_away = 0;
    while !(asked_while_away > 0 && side.applied() == Some(SOUGHT) && !side.composing()) {
        side.frame(SOUGHT).unwrap();
        if side.composing() {
            asked_while_away += 1;
            assert_eq!(side.viewer(), None, "no viewer while the layers are away");
            assert_eq!(side.vision_refused(), None, "no refusal while the layers are away");
            assert!(!side.take_overlay_withdrawal(), "no layout let go while the layers are away");
        }
        assert!(std::time::Instant::now() < deadline, "the seek's compose within 120 s");
        std::thread::yield_now();
    }
    assert_eq!(side.viewer(), Some(1), "the viewer answered again once the layers are home");
}

#[test]
fn a_new_viewing_country_is_never_handed_over_with_parts_of_the_country_before() {
    use crate::map_open::MapSide;
    use viewer_layer_core::playhead_host::Head;
    use crate::side::SideThread;
    use viewer_foundation::perspective::{Choice, Country};
    const HOUR: i32 = 8;
    const REFUSED: i32 = 16;
    let asked = Arc::new(Asked(std::sync::Mutex::new(Vec::new())));
    let (entry, layers) = viewing_world_and(&asked, |at, _| if at == REFUSED as u32 { vec![(TableId::Countries, Status::NotRecorded)] } else { Vec::new() });
    let mut side = SideThread::new(crate::map_open::MapSide::with_products(entry, layers, View::default()).unwrap()).unwrap();
    side.anchor(HOUR, false, 0.0, true);
    let chosen = View { perspective: Choice { country: Country::Tag("text 1".into()), ..Choice::default() }, ..View::default() };
    let refused = View { perspective: Choice { country: Country::Tag("text 0".into()), ..Choice::default() }, ..View::default() };
    let slot = |side: &MapSide| {
        let view = side.view_shown();
        if view.perspective == chosen.perspective {
            Some(2)
        } else if view.perspective == refused.perspective && side.applied() == Some(REFUSED) {
            None
        } else {
            Some(1)
        }
    };
    let viewers = |side: &MapSide| -> (Option<u32>, Vec<Option<u32>>) {
        let mut layers: Vec<Option<u32>> = side.caught_up().iter().map(|layer| layer.viewer).collect();
        if side.draws("engagements") {
            layers.extend(side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<viewer_layer_core::engagements::Engagements>())).map(viewer_layer_core::engagements::Engagements::viewer));
        }
        (side.vision().ok().map(|vision| vision.ours()), layers)
    };
    let wholly = |side: &MapSide, slot: u32| {
        let (shown, layers) = viewers(side);
        side.applied() == Some(HOUR) && !side.composing() && shown == Some(slot) && layers.iter().all(|held| held.is_none_or(|held| held == slot))
    };
    let never_mixed = |side: &MapSide| {
        let want = slot(side);
        let (shown, layers) = viewers(side);
        let other = |held: &Option<u32>| held.is_some_and(|held| Some(held) != want);
        assert!(!other(&shown) && !layers.iter().any(other), "a part made for another viewing country handed with {:?}: {:?}; {}", side.view_shown().perspective, viewers(side), side.shown_states());
    };
    let windows = |side: &mut SideThread, what: &str, done: &dyn Fn(&MapSide) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut handed = 0;
        loop {
            if let Some(mut window) = side.window() {
                never_mixed(window.side());
                let finished = done(window.side());
                window.take(Default::default());
                handed += 1;
                if finished {
                    return handed;
                }
            }
            assert!(Instant::now() < deadline, "{what} within 120 s, {handed} windows handed");
            std::thread::yield_now();
        }
    };
    side.ask(View::default(), Head::Paced(HOUR), None, None);
    windows(&mut side, "the recorded viewer, slot 1, wholly", &|side| wholly(side, 1));
    let held = side.with_side(MapSide::hold_settles);
    side.ask(chosen.clone(), Head::Paced(HOUR), None, None);
    windows(&mut side, "the chosen view handed while its vision is not made", &|side| side.view_shown().perspective == chosen.perspective);
    assert_eq!(side.view().perspective, chosen.perspective, "the frame draws the chosen view while its vision is made");
    assert!(side.products().overlay_coming(), "the frame keeps its picture while the vision is made");
    windows(&mut side, "a further window handed while the vision is not made", &|_| true);
    drop(held);
    windows(&mut side, "the chosen country, slot 2, wholly", &|side| wholly(side, 2));
    side.anchor(REFUSED - 4, false, 0.0, true);
    side.ask(chosen.clone(), Head::Paced(REFUSED - 4), None, None);
    windows(&mut side, "slot 2 at the hour before the refusal", &|side| side.applied() == Some(REFUSED - 4) && !side.composing() && side.viewer() == Some(2));
    side.ask(refused.clone(), Head::Paced(REFUSED), None, None);
    windows(&mut side, "the refused country handed over, its refusal named", &|side| {
        side.view_shown().perspective == refused.perspective && side.applied() == Some(REFUSED) && side.vision_refused().is_some()
    });
    assert_eq!(side.view().perspective, refused.perspective, "the refused country is handed over, not held off");
    assert!(side.products().vision.is_err(), "the frame's vision is withheld: {:?}", side.products().vision.as_ref().map(|vision| vision.ours()));
    assert!(side.products().vision_refused.as_deref().is_some_and(|why| !why.is_empty()), "the frame is told why the vision is withheld");
    assert!(!side.products().overlay_coming(), "no overlay is laid out without a vision: the frame draws without one");
    assert_eq!(side.with_side(|side| side.parts().map(|(_, layers)| layers[0].carried().entities)), Some(0), "a settled refusal is the vision's whole answer: it carries nothing");
    let caught = |side: &MapSide| {
        let layers = side.caught_up();
        ["vision", "engagements", "labels"].iter().all(|name| layers.iter().any(|layer| layer.name == *name && layer.caught))
    };
    windows(&mut side, "the refused view caught up, so a paused map waits on nothing of it", &|side| side.view_shown().perspective == refused.perspective && caught(side));
    side.with_side_mut(|side| {
        side.hand_viewer(refused.perspective.clone(), 1);
        assert!(side.take_overlay_withdrawal(), "a refused vision never keeps the layout of the hour before");
    });
    side.ask(refused.clone(), Head::Paced(REFUSED + 1), None, None);
    windows(&mut side, "slot 1 again once the countries are recorded", &|side| side.applied() > Some(REFUSED) && !side.composing() && side.viewer() == Some(1) && caught(side));
    side.with_side(|side| {
        let (mirrors, layers) = side.parts().unwrap();
        let labels = layers[4].any().downcast_ref::<viewer_layers_political::labels::Labels>().unwrap();
        assert_eq!(labels.ours(), Some(1), "the labels' viewer is slot 1 again");
        let mut fresh = labels.fresh();
        fresh.compose_with(mirrors, &viewer_layer_core::layers::Upstream::new(&layers[..4])).unwrap();
        assert_eq!(labels.differ(&fresh), (0, None), "the labels equal a reader opened after the refusal");
    });
}

fn play_ahead(units: Units, layers: Vec<Box<dyn Layer>>, last: i32, made: impl Fn(&dyn Layer, i32) -> bool, mut each: impl FnMut(&viewer_layer_core::playhead_host::Frame, &mut MapSide)) {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().seek(0);
    for _ in 0..50_000_000u64 {
        let frame = side.frame(side.applied().unwrap_or(0)).unwrap();
        if frame.swapped.is_some() && !side.composing() {
            break;
        }
        std::thread::yield_now();
    }
    let mut seen = 0;
    while side.applied().unwrap() < last {
        let next = side.applied().unwrap() + 4;
        for _ in 0..50_000_000u64 {
            let applied = side.applied().unwrap();
            side.frame(applied).unwrap();
            if entry.playhead().newest() >= Some(next) && made(&*side.parts().unwrap().1[0], next) {
                break;
            }
            std::thread::yield_now();
        }
        let mut stepped = None;
        for _ in 0..50_000_000u64 {
            let frame = side.frame(next).unwrap();
            if frame.steps > 0 {
                stepped = Some(frame);
                break;
            }
            std::thread::yield_now();
        }
        let frame = stepped.unwrap_or_else(|| panic!("no frame took the step to hour {next}"));
        assert_eq!((frame.applied, frame.steps), (Some(next), 1), "a step of four hours a frame");
        each(&frame, &mut side);
        seen += 1;
    }
    assert!(seen >= 4, "steps seen: {seen}");
}

#[derive(Default)]
struct Listens {
    stepped: Option<i32>,
    heard: Vec<(Option<i32>, Vec<u64>)>,
}

impl Layer for Listens {
    fn name(&self) -> &'static str {
        "listens"
    }
    fn inputs(&mut self, _view: &View, inputs: &mut viewer_layer_core::layers::Inputs<'_>) -> io::Result<()> {
        inputs.after("vision");
        Ok(())
    }
    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        Ok(())
    }
    fn apply(&mut self, step: &playhead::Step, _mirrors: &Mirrors) {
        self.stepped = Some(step.last());
    }
    fn settle(&mut self, _mirrors: &Mirrors, _settling: viewer_layer_core::layers::Settling) -> io::Result<()> {
        Ok(())
    }
    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &viewer_layer_core::layers::Upstream<'_>, _settling: viewer_layer_core::layers::Settling) -> io::Result<()> {
        let named = match upstream.changed::<Vision>() {
            viewer_layer_core::layers::Change::Whole => Vec::new(),
            viewer_layer_core::layers::Change::Marked(marked) => marked.of("province").to_vec(),
        };
        self.heard.push((self.stepped.take(), named));
        Ok(())
    }
    fn upload(&mut self, _gpu: &mut viewer_layer_core::layers::Gpu<'_>) -> io::Result<()> {
        Ok(())
    }
    fn carried(&self) -> viewer_layer_core::layers::Carried {
        viewer_layer_core::layers::Carried::default()
    }
    fn held(&self) -> usize {
        0
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

#[test]
fn vision_folded_ahead_shows_the_hour_applied_in_its_frame_and_its_readers_hear_it_there() {
    let mut heard_four = None;
    play_ahead(
        vision_world(),
        vec![Box::new(Vision::new(vision_content())), Box::new(Listens::default())],
        40,
        |layer, hour| layer.any().downcast_ref::<Vision>().unwrap().made_through() >= Some(hour),
        |frame, side| {
            let hour = frame.applied.unwrap();
            assert_eq!(frame.layers[0].ready_at_hour, Some(true), "vision's product of hour {hour} was ready in the frame that applied it");
            assert_eq!(side.parts().unwrap().1[0].published(), viewer_layer_core::layers::Published::Current, "vision shows hour {hour}");
            let listens = side.parts().unwrap().1[1].any().downcast_ref::<Listens>().unwrap();
            let (stepped, provinces) = listens.heard.last().unwrap();
            assert_eq!(*stepped, Some(hour), "the reader settled in the frame that applied {hour}");
            if (hour - 3..=hour).contains(&13) {
                heard_four = Some(provinces.contains(&4));
            }
            side.settle_all().unwrap();
            let (mirrors, layers) = side.parts().unwrap();
            let layer = layers[0].any().downcast_ref::<Vision>().unwrap();
            let mut fresh = layer.fresh();
            fresh.compose(mirrors).unwrap();
            assert_eq!(layer.vision().expect("the layer draws").differences(&fresh.vision().expect("composed draws")), (0, Vec::new()), "stepped equals composed at hour {hour}");
        },
    );
    assert_eq!(heard_four, Some(true), "province 4 changing hands at hour 13 reached the reader in the frame of its step");
}

#[test]
fn snow_folded_ahead_shows_the_hour_applied_in_its_frame() {
    use viewer_layers_infrastructure::snow::{self, Snow};
    let ids: Vec<u32> = (0..144u32).map(|at| match at / 48 {
        0 => 30,
        1 => 20,
        _ => 10,
    }).collect();
    let map = content_reader_map::map::MapData { width: 12, height: 12, province_ids: ids, definitions: Default::default(), country_colors: Default::default() };
    let cells = map_snow::SnowCells::new(&map, &[false; 144]).unwrap();
    let winter = map_snow::winter::Winter { defines: map_snow::winter::SnowDefines { per_province: [0.0, 1.0], total: [0.0, 1.0] }, records: vec![vec![map_snow::winter::SnowRecord { province: 20, weight: 1.0 }]] };
    let table = TableId::WeatherProvinces;
    let [snow_column, display_snow, display_mud] = ["snow", "display_snow", "display_mud"].map(|name| column(table, name));
    let units = record(40, &[table], |at, store| {
        let mut changes = store.table(table.index()).unwrap();
        if at == 0 {
            for (slot, id) in [10u32, 20, 30].into_iter().enumerate() {
                changes.insert_at(slot as u32, u128::from(id)).unwrap();
            }
        }
        for slot in 0..3u32 {
            changes.write(slot, snow_column, Cell::I64(i64::from(at) * 1000 + i64::from(slot))).unwrap();
            changes.write(slot, display_snow, Cell::U8((at * 7 + slot * 40) as u8)).unwrap();
            changes.write(slot, display_mud, Cell::U8((at * 3 + slot) as u8)).unwrap();
        }
        Vec::new()
    });
    play_ahead(
        units,
        vec![Box::new(Snow::new(Ok(Arc::new(snow::Content::new(cells, winter, MapSize { width: 64, height: 64 }, MapDefines::test())))))],
        32,
        |layer, hour| layer.any().downcast_ref::<Snow>().unwrap().made_through() >= Some(hour),
        |frame, side| {
            let hour = frame.applied.unwrap();
            assert_eq!(frame.layers[0].ready_at_hour, Some(true), "snow's grid of hour {hour} was ready in the frame that applied it");
            assert_eq!(side.parts().unwrap().1[0].published(), viewer_layer_core::layers::Published::Current, "snow shows hour {hour}");
            let (mirrors, layers) = side.parts().unwrap();
            let layer = layers[0].any().downcast_ref::<Snow>().unwrap();
            let mut fresh = layer.fresh();
            fresh.compose(mirrors).unwrap();
            assert_eq!((layer.texels(), layer.unknown(), layer.summary()), (fresh.texels(), fresh.unknown(), fresh.summary()), "stepped equals composed at hour {hour}");
        },
    );
}

fn ahead_then_hourly(units: Units, layers: Vec<Box<dyn Layer>>, from: i32, last: i32, made: impl Fn(&dyn Layer, i32) -> bool, mut each: impl FnMut(i32, &mut MapSide)) -> u32 {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().seek(0);
    let deadline = Instant::now() + Duration::from_secs(120);
    let frame = |side: &mut MapSide, head: i32, what: &str| {
        assert!(Instant::now() < deadline, "{what} within 120 s (applied {:?}, newest {:?})", side.applied(), side.entry().playhead().newest());
        let frame = side.frame(head).unwrap();
        if frame.steps == 0 {
            std::thread::yield_now();
        }
        frame
    };
    while !(frame(&mut side, 0, "the first compose").swapped.is_some() && !side.composing()) {}
    while side.applied().unwrap() < from {
        let next = side.applied().unwrap() + 4;
        while entry.playhead().newest() < Some(next) {
            let applied = side.applied().unwrap();
            frame(&mut side, applied, "the next four-hour step");
        }
        frame(&mut side, next, "the next four-hour step");
    }
    let applied = side.applied().unwrap();
    while !made(&*side.parts().unwrap().1[0], applied + 8) {
        frame(&mut side, applied, "the worker folding ahead");
    }
    entry.playhead().pace(1.0);
    let mut voided = 0;
    while entry.playhead().held().2.resteps == 0 {
        voided += frame(&mut side, applied, "the steps ahead built again").voided;
    }
    for hour in applied + 1..=last {
        loop {
            let done = frame(&mut side, hour, "a step of one hour");
            voided += done.voided;
            if done.applied == Some(hour) {
                assert_eq!(done.steps, 1, "one step of one hour to hour {hour}");
                break;
            }
        }
        side.settle_all().unwrap();
        each(hour, &mut side);
    }
    voided
}

#[test]
fn vision_after_a_slower_pace_starts_over_and_equals_the_composed_vision_at_each_hour() {
    let voided = ahead_then_hourly(
        vision_world(),
        vec![Box::new(Vision::new(vision_content()))],
        8,
        24,
        |layer, hour| layer.any().downcast_ref::<Vision>().unwrap().made_through() >= Some(hour),
        |hour, side| {
            let (mirrors, layers) = side.parts().unwrap();
            assert_eq!(layers[0].published(), viewer_layer_core::layers::Published::Current, "vision shows hour {hour}");
            let layer = layers[0].any().downcast_ref::<Vision>().unwrap();
            let mut fresh = layer.fresh();
            fresh.compose(mirrors).unwrap();
            assert_eq!(layer.vision().expect("the layer draws").differences(&fresh.vision().expect("composed draws")), (0, Vec::new()), "equals composed at hour {hour}");
        },
    );
    assert!(voided >= 1, "the steps vision folded ahead were voided and it started over");
}

#[test]
fn snow_after_a_slower_pace_starts_over_and_equals_the_composed_grid_at_each_hour() {
    use viewer_layers_infrastructure::snow::{self, Snow};
    let ids: Vec<u32> = (0..144u32).map(|at| match at / 48 {
        0 => 30,
        1 => 20,
        _ => 10,
    }).collect();
    let map = content_reader_map::map::MapData { width: 12, height: 12, province_ids: ids, definitions: Default::default(), country_colors: Default::default() };
    let cells = map_snow::SnowCells::new(&map, &[false; 144]).unwrap();
    let winter = map_snow::winter::Winter { defines: map_snow::winter::SnowDefines { per_province: [0.0, 1.0], total: [0.0, 1.0] }, records: vec![vec![map_snow::winter::SnowRecord { province: 20, weight: 1.0 }]] };
    let table = TableId::WeatherProvinces;
    let [snow_column, display_snow, display_mud] = ["snow", "display_snow", "display_mud"].map(|name| column(table, name));
    let units = record(40, &[table], |at, store| {
        let mut changes = store.table(table.index()).unwrap();
        if at == 0 {
            for (slot, id) in [10u32, 20, 30].into_iter().enumerate() {
                changes.insert_at(slot as u32, u128::from(id)).unwrap();
            }
        }
        for slot in 0..3u32 {
            changes.write(slot, snow_column, Cell::I64(i64::from(at) * 1000 + i64::from(slot))).unwrap();
            changes.write(slot, display_snow, Cell::U8((at * 7 + slot * 40) as u8)).unwrap();
            changes.write(slot, display_mud, Cell::U8((at * 3 + slot) as u8)).unwrap();
        }
        Vec::new()
    });
    let voided = ahead_then_hourly(
        units,
        vec![Box::new(Snow::new(Ok(Arc::new(snow::Content::new(cells, winter, MapSize { width: 64, height: 64 }, MapDefines::test())))))],
        8,
        24,
        |layer, hour| layer.any().downcast_ref::<Snow>().unwrap().made_through() >= Some(hour),
        |hour, side| {
            let (mirrors, layers) = side.parts().unwrap();
            assert_eq!(layers[0].published(), viewer_layer_core::layers::Published::Current, "snow shows hour {hour}");
            let layer = layers[0].any().downcast_ref::<Snow>().unwrap();
            let mut fresh = layer.fresh();
            fresh.compose(mirrors).unwrap();
            assert_eq!((layer.texels(), layer.unknown(), layer.summary()), (fresh.texels(), fresh.unknown(), fresh.summary()), "equals composed at hour {hour}");
        },
    );
    assert!(voided >= 1, "the grids snow built ahead were voided and it started over");
}
