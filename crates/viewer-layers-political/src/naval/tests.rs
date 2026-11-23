
use super::{Naval, reads};
use viewer_layer_core::{layers::{Layer, View}, test_support::recorded, vision, playhead_host::{MapSide, mirror::Mirrors, open}};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use map_display_data::landscape::Attributes;
use viewer_map_modes::{
    map_mode::{MapMode, Stock},
    naval_dominance::{Content, Fact, Relations, Sides, Supremacy},
};

fn content() -> Arc<Content> {
    Arc::new(Content::for_tests(&[(10, true, 100_000), (11, true, 100_000), (12, false, 100_000)], 60_000))
}

fn vision_content() -> Arc<vision::Content> {
    use viewer_foundation::visibility::{FogContent, Topology, test_defines};
    let neighbors = BTreeMap::from([(1, vec![2]), (2, vec![1])]);
    Arc::new(vision::Content::new(Ok(FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors }, defines: test_defines() })))
}

struct Oracle {
    links: BTreeMap<u32, u32>,
    factions: BTreeMap<u32, u64>,
    wars: BTreeSet<(u32, u32)>,
}

impl Relations for Oracle {
    fn links(&self) -> Result<&BTreeMap<u32, u32>, String> {
        Ok(&self.links)
    }

    fn family(&self, _links: &BTreeMap<u32, u32>, source: u32, viewer: u32) -> Fact<bool> {
        Ok(source == viewer || self.factions.get(&source).is_some_and(|faction| self.factions.get(&viewer) == Some(faction)))
    }

    fn war(&self, source: u32, viewer: u32) -> Fact<bool> {
        Ok(self.wars.contains(&(self.links[&source], viewer)))
    }

    fn blockades(&self) -> Fact<&[(u32, u32)]> {
        Ok(&[])
    }
}

#[derive(Clone, Default)]
struct Truth {
    rows: BTreeMap<(u32, u32), (i64, i64)>,
    factions: BTreeMap<u32, u64>,
    recorded: bool,
}

impl Truth {
    fn expected(&self, content: &Content) -> BTreeMap<u32, Attributes> {
        let oracle = Oracle {
            links: BTreeMap::from([(101, 1), (102, 2), (103, 3)]),
            factions: self.factions.iter().map(|(&raw, &faction)| (raw - 100, faction)).collect(),
            wars: BTreeSet::from([(2, 1), (1, 2)]),
        };
        let mut sides = Sides::default();
        content
            .ids()
            .map(|id| {
                let values: Vec<Supremacy> = self.rows.range((id, 0)..=(id, u32::MAX)).map(|(&(_, country), &(current, previous))| Supremacy { country, current, previous }).collect();
                let read = if self.recorded { Ok(&values[..]) } else { Err("not recorded".to_owned()) };
                (id + 1, content.region(id, &Ok(()), read, &oracle, 1, &mut sides).0)
            })
            .collect()
    }
}

fn record_world(history: &mut BTreeMap<i32, Truth>) -> viewer_layer_core::playhead_host::units::Units {
    let mut tables: Vec<TableId> = viewer_foundation::perspective::VISION_TABLES.to_vec();
    tables.extend(reads::TABLES);
    tables.sort_unstable();
    tables.dedup();
    let mut truth = Truth { recorded: true, ..Truth::default() };
    let column = recorded::column;
    static RECORDED: AtomicUsize = AtomicUsize::new(0);
    let name = format!("naval test {}", RECORDED.fetch_add(1, Ordering::Relaxed));
    recorded::record(&name, 40, |at, store| {
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
        let supremacy = |store: &mut Store, truth: &mut Truth, slot: u32, key: (u32, u32), current: i64, previous: i64| {
            write(store, TableId::NavalSupremacy, slot, "current", Cell::I64(current));
            write(store, TableId::NavalSupremacy, slot, "previous", Cell::I64(previous));
            truth.rows.insert(key, (current, previous));
        };
        let insert = |store: &mut Store, table: TableId, slot: u32, key: u128| store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
        let key = |(region, country): (u32, u32)| u128::from(region) << 32 | u128::from(country);
        match at {
            0 => {
                for (slot, raw) in [101u32, 102, 103].into_iter().enumerate() {
                    insert(store, TableId::CountryLinks, slot as u32, u128::from(raw));
                    write(store, TableId::CountryLinks, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                    insert(store, TableId::Countries, slot as u32, u128::from(raw));
                    write(store, TableId::Countries, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                }
                insert(store, TableId::LocalPlayer, 0, 0);
                write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
                insert(store, TableId::WarRelations, 0, 101 << 32 | 102);
                for (slot, raw) in [101u32, 103].into_iter().enumerate() {
                    insert(store, TableId::FactionMembers, slot as u32, u128::from(raw));
                    write(store, TableId::FactionMembers, slot as u32, "faction", Cell::U64(7));
                    truth.factions.insert(raw, 7);
                }
                for (slot, (at, current, previous)) in [((10, 101), 900_000, 900_000), ((10, 102), 100_000, 50_000), ((11, 102), 500_000, 400_000), ((11, 103), 450_000, 450_000)].into_iter().enumerate() {
                    insert(store, TableId::NavalSupremacy, slot as u32, key(at));
                    supremacy(store, &mut truth, slot as u32, at, current, previous);
                }
            }
            5 => supremacy(store, &mut truth, 1, (10, 102), 2_000_000, 100_000),
            6 => supremacy(store, &mut truth, 2, (11, 102), 100_000, 500_000),
            7 => supremacy(store, &mut truth, 2, (11, 102), 700_000, 100_000),
            9 => {
                store.table(TableId::NavalSupremacy.index()).unwrap().delete(0).unwrap();
                truth.rows.remove(&(10, 101));
                insert(store, TableId::NavalSupremacy, 4, key((10, 103)));
                supremacy(store, &mut truth, 4, (10, 103), 3_000_000, 1);
            }
            13 => {
                store.table(TableId::FactionMembers.index()).unwrap().delete(1).unwrap();
                truth.factions.remove(&103);
            }
            _ => {}
        }
        truth.recorded = !(18..22).contains(&at);
        for &table in &tables {
            let status = if table == TableId::NavalSupremacy && !truth.recorded { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        history.insert(at as i32, truth.clone());
    })
}

fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

fn play(last: i32, each: impl FnMut(i32, &mut MapSide, &BTreeMap<i32, Truth>)) {
    play_with(vec![Box::new(vision::Vision::new(vision_content())), Box::new(Naval::new(content()))], last, each);
}

fn play_with(layers: Vec<Box<dyn Layer>>, last: i32, mut each: impl FnMut(i32, &mut MapSide, &BTreeMap<i32, Truth>)) {
    let mut history = BTreeMap::new();
    let units = record_world(&mut history);
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    let (mut seen, mut applied) = (Vec::new(), None::<i32>);
    for _ in 0..50_000_000u64 {
        let head = match applied {
            None => 0,
            Some(hour) if side.composing() => hour,
            Some(hour) => entry.playhead().newest().unwrap_or(hour).min(hour + 4),
        };
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        if (frame.steps > 0 || frame.swapped.is_some()) && !side.composing()
            && let Some(hour) = applied
        {
            side.settle_all().unwrap();
            seen.push(hour);
            each(hour, &mut side, &history);
            if hour >= last {
                return;
            }
        } else {
            std::thread::yield_now();
        }
    }
    let (report, _, counters) = entry.playhead().held();
    panic!("hour {last} was not reached: hours seen {seen:?}, newest {:?}, applied {applied:?}, composing {}, preparing {}, {counters:?}", entry.playhead().newest(), side.composing(), report.preparing);
}

fn naval(side: &MapSide) -> (&Mirrors, &Naval) {
    let (mirrors, layers) = side.parts().unwrap();
    (mirrors, layers.iter().find_map(|layer| layer.any().downcast_ref::<Naval>()).unwrap())
}

#[test]
fn naval_regions_follow_their_rows_and_relations_settle_to_the_newest_and_equal_composed() {
    let content = content();
    let (mut checked, mut withheld) = (0, 0);
    let (mut marked_before, mut previous) = ([0; 3], None);
    play(32, |hour, side, history| {
        let (mirrors, layer) = naval(side);
        let fresh = layer.composed(mirrors);
        assert_eq!(layer.attributes(), fresh.attributes(), "stepped equals composed at hour {hour}");
        assert_eq!(layer.warnings(), fresh.warnings(), "stepped warnings equal composed at hour {hour}");
        assert_eq!(layer.viewer(), Some(1));
        let truth = &history[&hour];
        assert_eq!(**layer.attributes(), truth.expected(&content), "the classification of hour {hour}");
        if !truth.recorded {
            withheld += 1;
            assert!(layer.warnings()[0].contains("withheld for 2 strategic regions"), "{:?}", layer.warnings());
        }
        if hour == 8 && previous == Some(4) {
            assert_eq!(layer.marked()[0] - marked_before[0], 2, "regions 10 and 11, each once");
        }
        (marked_before, previous) = (layer.marked(), Some(hour));
        checked += 1;
    });
    assert!(checked >= 5 && withheld >= 1, "checked {checked}, withheld {withheld}");
}

#[test]
fn another_map_mode_declares_nothing_and_the_default_composes_again() {
    let content = content();
    let ring = quick().ring as i32;
    let (mut hidden_at, mut shown_at, mut back, mut drawn_again) = (None, None, None, 0);
    let mut hours = Vec::new();
    play(39, |hour, side, history| {
        hours.push(hour);
        let Some(at) = hidden_at else {
            if hour >= 8 {
                side.set_view(View { map_mode: MapMode::Stock(Stock::Diplomacy), ..View::default() }).unwrap();
                hidden_at = Some(hour);
            }
            return;
        };
        let Some(shown) = shown_at else {
            if hour > at {
                let (_, layer) = naval(side);
                assert!(layer.attributes().is_empty(), "nothing is drawn at another map mode (hour {hour})");
                assert_eq!(layer.carried().entities, 0, "a layer that declared nothing carries nothing: the map's first picture waits on what is carried (hour {hour})");
                if hour >= at + 8 {
                    side.set_view(View::default()).unwrap();
                    shown_at = Some(hour);
                }
            }
            return;
        };
        let by = (shown + 4 * (ring + 1)).min(39);
        let (mirrors, layer) = naval(side);
        let heads: Vec<(TableId, Option<i32>)> = mirrors.iter().map(|mirror| (mirror.table(), mirror.head())).collect();
        let whole = side.draws("naval") && mirrors.get(TableId::NavalSupremacy).is_some() && heads.iter().all(|&(_, head)| head == Some(hour));
        if !whole {
            let (_, _, counters) = side.entry().playhead().held();
            assert!(back.is_some() || hour <= by, "the naval tables, declared again at hour {shown}, were not back by hour {by}: at hour {hour} naval drawn {}, mirrors at {heads:?}; hours seen {hours:?}; the playhead's {counters:?}", side.draws("naval"));
            return;
        }
        assert_eq!(**layer.attributes(), history[&hour].expected(&content), "drawn again at hour {hour}");
        assert_eq!(layer.attributes(), layer.composed(mirrors).attributes(), "composed again at hour {hour}");
        back = back.or(Some(hour));
        drawn_again += 1;
    });
    assert!(back.is_some() && drawn_again > 0, "the layer was not drawn again at the default mode (hidden at {hidden_at:?}, shown at {shown_at:?}): hours seen {hours:?}");
}

fn political_content() -> Arc<crate::political::Content> {
    use content_reader_map::map::{MapData, ProvinceDefinition, ProvinceKind};
    use viewer_borders::borders::{BorderClass, BorderMesh, BorderMeshes};
    let definition = |id, kind| (id, ProvinceDefinition { id, rgb: [0; 3], kind, continent: 0 });
    let map = MapData { width: 2, height: 1, province_ids: vec![1, 2], definitions: [definition(1, ProvinceKind::Land), definition(2, ProvinceKind::Sea)].into_iter().collect(), country_colors: Default::default() };
    let empty = |class| BorderMesh { class, chunks: vec![], coarse_chunks: vec![], draws: vec![] };
    let meshes = BorderMeshes { meshes: BorderClass::ALL.iter().map(|&class| empty(class)).collect(), hidden: empty(BorderClass::Province), indices: vec![], one_pixel_provinces: vec![] };
    Arc::new(crate::political::Content::new(Arc::new(map), std::sync::Arc::new(meshes)))
}

#[test]
fn the_limitation_lines_while_the_political_layer_draws_are_the_political_and_naval_layers_own() {
    let layers: Vec<Box<dyn Layer>> = vec![
        Box::new(crate::political::Political::new(political_content())),
        Box::new(vision::Vision::new(vision_content())),
        Box::new(Naval::new(content())),
    ];
    let mut withheld = 0;
    play_with(layers, 32, |hour, side, history| {
        assert!(side.draws("political"), "the political layer draws at hour {hour}");
        let (_, layers) = side.parts().unwrap();
        let political = layers.iter().find_map(|layer| layer.any().downcast_ref::<crate::political::Political>()).unwrap();
        let (_, naval) = naval(side);
        let mut expected = political.warnings();
        expected.extend(naval.warnings());
        let lines = crate::political_lines::political_lines(side);
        assert_eq!(lines, expected, "the political and naval layers' lines at hour {hour}");
        if !history[&hour].recorded {
            assert!(lines.iter().any(|line| line.starts_with("Naval dominance FX withheld for 2 strategic regions")), "the naval layer's withheld regions at hour {hour}: {lines:?}");
            withheld += 1;
        }
    });
    assert!(withheld > 0, "a frame inside the hours NavalSupremacy is not recorded");
}
