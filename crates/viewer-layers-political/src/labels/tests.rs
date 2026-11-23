use super::Labels;
use viewer_layer_core::{layers::Layer, test_support::rail_recording::{frames_until, recorded, side_of}};
use content_reader_localisation::localisation::Localisation;
use map_display_data::landscape::LabelName;
use replay_columnar::{
    manifest::Status,
    state::Cell,
    store::Store,
};
use replay_data::rows::TableId;
use std::sync::Arc;

const GAP: std::ops::Range<i32> = 40..48;

fn key(high: u32, low: u32) -> u128 {
    u128::from(high) << 32 | u128::from(low)
}

fn side(name: &str) -> viewer_layer_core::playhead_host::MapSide {
    let strings = |at: u32| match at {
        0 => ["AAA", "BBB", "Bigtown"].map(str::to_owned).to_vec(),
        _ => Vec::new(),
    };
    let tables = [TableId::Provinces, TableId::CountryLinks, TableId::Countries, TableId::States, TableId::Politics, TableId::CountryAppearance, TableId::ProvinceNames, TableId::WarRelations];
    let pages = recorded(80, strings, |at, store| {
        let gap = GAP.contains(&(at as i32));
        for table in tables {
            let status = if gap && table == TableId::Countries { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        let points = |store: &mut Store, province: u32, points: i32| store.table(TableId::Provinces.index()).unwrap().write(province - 1, 3, Cell::I32(points)).unwrap();
        match at {
            0 => {
                for (slot, native, canonical, tag) in [(0, 10, 1, 0), (1, 20, 2, 1)] {
                    let mut countries = store.table(TableId::Countries.index()).unwrap();
                    countries.insert_at(slot, u128::from(native)).unwrap();
                    countries.write(slot, 0, Cell::U32(canonical)).unwrap();
                    countries.write(slot, 1, Cell::Bytes(tag)).unwrap();
                    let mut links = store.table(TableId::CountryLinks.index()).unwrap();
                    links.insert_at(slot, u128::from(native)).unwrap();
                    links.write(slot, 0, Cell::U32(canonical)).unwrap();
                    let mut appearance = store.table(TableId::CountryAppearance.index()).unwrap();
                    appearance.insert_at(slot, u128::from(native)).unwrap();
                    appearance.write(slot, 20, Cell::OptionalI32(Some(0))).unwrap();
                    let mut politics = store.table(TableId::Politics.index()).unwrap();
                    politics.insert_at(slot, u128::from(native)).unwrap();
                    politics.write(slot, 3, Cell::I32(if native == 10 { 100 } else { 200 })).unwrap();
                    let mut states = store.table(TableId::States.index()).unwrap();
                    states.insert_at(slot, u128::from(native * 10)).unwrap();
                    states.write(slot, 0, Cell::U32(native)).unwrap();
                    states.write(slot, 1, Cell::U32(native)).unwrap();
                }
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for (province, state, controller, points) in [(1, 100, 10, 5), (2, 100, 10, 0), (3, 200, 20, 3), (4, 200, 20, 3)] {
                    provinces.insert_at(province - 1, u128::from(province)).unwrap();
                    provinces.write(province - 1, 0, Cell::U32(state)).unwrap();
                    provinces.write(province - 1, 2, Cell::U32(controller)).unwrap();
                    provinces.write(province - 1, 3, Cell::I32(points)).unwrap();
                }
            }
            9 => points(store, 2, 7),
            10 => points(store, 2, 1),
            11 => points(store, 2, 9),
            20 => {
                let mut names = store.table(TableId::ProvinceNames.index()).unwrap();
                names.insert_at(0, 3).unwrap();
                names.write(0, 0, Cell::Bytes(2)).unwrap();
            }
            24 => {
                let mut wars = store.table(TableId::WarRelations.index()).unwrap();
                wars.insert_at(0, key(10, 20)).unwrap();
                wars.write(0, 0, Cell::I32(24)).unwrap();
                wars.write(0, 1, Cell::I32(60)).unwrap();
            }
            30 => {
                store.table(TableId::Provinces.index()).unwrap().write(3, 2, Cell::U32(10)).unwrap();
                store.table(TableId::States.index()).unwrap().write(1, 0, Cell::U32(10)).unwrap();
            }
            _ => {}
        }
    });
    let localisation: Localisation = [("AAA_VICTORY_POINTS_1", "Firstburg"), ("BBB_VICTORY_POINTS_4", "Fourton"), ("AAA_VICTORY_POINTS_4", "Vierstadt")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    side_of(pages, Box::new(Labels::new(Arc::new(localisation), Some(1))), name)
}

fn labels(side: &viewer_layer_core::playhead_host::MapSide) -> &Labels {
    side.parts().unwrap().1[0].any().downcast_ref::<Labels>().unwrap()
}

#[test]
fn labels_settled_every_step_equal_composed_and_follow_points_capitals_names_wars_owners_and_gaps() {
    let mut side = side("labels-steps");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut seen = Vec::new();
    let mut made: Option<(Arc<map_display_data::landscape::Labels>, replay_data::Hour)> = None;
    frames_until(&mut side, |side, frame| {
        if frame.steps == 0 || side.composing() {
            return false;
        }
        let (mirrors, _) = side.parts().unwrap();
        let layer = labels(side);
        assert_eq!(layer.carried().entities, 0);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.differ(&fresh), (0, None), "settled at hour {:?} equals composed", frame.applied);
        let hour = frame.applied.unwrap();
        assert_eq!(layer.notes(), fresh.notes(), "the notes settled at hour {hour} equal those composed");
        if GAP.contains(&hour) {
            assert!(layer.refused().is_some() && layer.display().candidates.is_empty(), "Countries not recorded draws no labels at hour {hour}");
            assert!(layer.notes().is_empty(), "a refused layer gives no note of its own at hour {hour}");
            assert_eq!(layer.drawn_hours(), None, "refused labels hold for no hour, so nothing takes their emptiness as hour {hour}'s");
            seen.push(hour);
            return false;
        }
        let hours = layer.drawn_hours().expect("settled labels hold for an hour");
        assert!(hours.contains(&hour), "the labels drawn at hour {hour} hold for it: {hours:?}");
        match made.replace((Arc::clone(&layer.drawn), *hours.start())) {
            Some((before, from)) if Arc::ptr_eq(&before, &layer.drawn) => assert_eq!(*hours.start(), from, "unchanged labels hold from the hour they were made, at hour {hour}"),
            Some(_) => assert_eq!(hours, hour..=hour, "labels made again at hour {hour} hold for no earlier hour"),
            None => {}
        }
        let display = layer.display();
        let candidate = |province: u32| display.candidates.iter().find(|candidate| candidate.province == province).cloned();
        let text = |province: u32| match candidate(province).map(|candidate| candidate.name) {
            Some(LabelName::Value { text }) => text,
            other => panic!("province {province} at hour {hour}: {other:?}"),
        };
        assert_eq!(display.perspective.as_deref(), Some("AAA"));
        let tied = format!("Field Capital stars: capital state 200 carries no star: {}", viewer_map_icons::labels::TIED_VICTORY_POINTS);
        assert_eq!(layer.notes(), [tied], "today's line for the withheld star alone at hour {hour}");
        if hour < 8 {
            assert!(candidate(1).unwrap().capital && candidate(2).is_none(), "the capital star on the most points at hour {hour}");
            assert!(!candidate(3).unwrap().capital && !candidate(4).unwrap().capital, "a tie carries no star at hour {hour}");
            assert_eq!(text(1).as_deref(), Some("Firstburg"));
        }
        if hour >= 12 {
            let moved = candidate(2).unwrap();
            assert!((moved.victory_points, moved.capital) == (9, true) && !candidate(1).unwrap().capital, "points written twice in a step show the newest at hour {hour}");
        }
        if hour >= 24 {
            assert_eq!(text(3).as_deref(), Some("Bigtown"), "a recorded rename at hour {hour}");
        }
        if (28..60).contains(&hour) {
            assert_eq!(display.allegiances.get(&2), Some(&0), "at war at hour {hour}");
        }
        if hour >= 60 {
            assert_eq!(display.allegiances.get(&2), None, "the war ended at hour {hour}");
        }
        if hour >= 32 {
            assert_eq!((candidate(4).unwrap().controller, text(4).as_deref()), (Some(1), Some("Vierstadt")), "a state's new owner renames its provinces at hour {hour}");
        } else if hour >= 4 {
            assert_eq!(text(4).as_deref(), Some("Fourton"));
        }
        seen.push(hour);
        hour >= 76
    });
    assert!(seen.iter().any(|hour| GAP.contains(hour)) && seen.iter().any(|&hour| hour >= 60), "{seen:?}");
}
