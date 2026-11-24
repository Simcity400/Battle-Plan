use std::sync::Arc;
use super::Sounds;
use viewer_layer_core::test_support::sound_recording::{complete, play, put, quick, record, sought, units};
use viewer_layer_core::{layers::{Layer, View}, playhead_host::{MapSide, open}};
use content_reader_sound::combat_sounds::{BattleDefines, CombatSound};
use replay_columnar::{
    manifest::Status,
    state::Cell,
    store::Store,
    unit::BLOCK,
};
use replay_data::rows::{Field, NativeValue, Opaque, TableId, TemplateGrids, TemplateSubunit};
#[cfg(test)]
use viewer_layer_core::layers::Inputs;
use viewer_audio::battle::Battles;

fn audio(battle: bool) -> Arc<viewer_audio::Content> {
    let mut content = viewer_audio::Content::new(Default::default(), None);
    if battle {
        content.battle = Some(BattleDefines { effect: "distant".into(), falloff_default: 100.0, min_unit_count: 1 });
        content.layers.sounds.push(CombatSound { name: "rifles".into(), effect: "rifles".into(), units: vec!["name 0".into()], min: 1, max: None, source: String::new() });
    }
    Arc::new(content)
}

fn grids(token: u32, count: i32) -> Vec<u8> {
    let grids = TemplateGrids { regiments: vec![TemplateSubunit { token, local: 0, count }], support: Vec::new(), regimental_support: Vec::new() };
    let Field::Opaque(Opaque::Native(value)) = grids.field() else { panic!("grids are opaque") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn u64s(items: &[u64]) -> Vec<u64> {
    items.to_vec()
}

const SOUND_TABLES: [TableId; 5] = [TableId::BattleSounds, TableId::LandCombats, TableId::Divisions, TableId::DivisionTemplateIcons, TableId::LexicalTokens];

fn campaign(at: u32, store: &mut Store) {
    let unrecorded = (15..18).contains(&at);
    complete(store, at, &SOUND_TABLES);
    if unrecorded {
        store.status(TableId::LandCombats.index(), Status::NotRecorded, true, Some(at as i32), true).unwrap();
    }
    let template = grids(7, 3);
    match at {
        0 => {
            put(store, TableId::LexicalTokens, 0, Some(7), &[("literal", Cell::Bytes(0))]);
            put(store, TableId::DivisionTemplateIcons, 0, Some(50), &[("model_unreadable", Cell::U8(0)), ("grids", Cell::Opaque(&template))]);
            put(store, TableId::Divisions, 0, Some(1001), &[("template", Cell::U64(50))]);
            put(store, TableId::Divisions, 1, Some(1002), &[("template", Cell::U64(50))]);
            put(store, TableId::LandCombats, 0, Some(900), &[("province", Cell::U32(10)), ("attackers", Cell::U64List(&u64s(&[1001]))), ("attacker_classes", Cell::U32List(&[0])), ("defenders", Cell::U64List(&[1002])), ("defender_classes", Cell::U32List(&[0]))]);
            put(store, TableId::BattleSounds, 0, Some(0), &[("center_x", Cell::U32(1.5f32.to_bits())), ("center_z", Cell::U32(2.5f32.to_bits())), ("radius", Cell::U32(4.0f32.to_bits())), ("provinces", Cell::U32List(&[10, 11, 10])), ("object", Cell::U64(77))]);
        }
        5 => put(store, TableId::LandCombats, 0, None, &[("attackers", Cell::U64List(&[1001, 1003])), ("attacker_classes", Cell::U32List(&[0, 0]))]),
        6 => put(store, TableId::Divisions, 1, None, &[("template", Cell::U64(51))]),
        9 => put(store, TableId::LandCombats, 1, Some(901), &[("province", Cell::U32(11)), ("attackers", Cell::U64List(&[])), ("attacker_classes", Cell::U32List(&[])), ("defenders", Cell::U64List(&[1001])), ("defender_classes", Cell::U32List(&[0]))]),
        13 => store.table(TableId::LandCombats.index()).unwrap().delete(0).unwrap(),
        14 => put(store, TableId::BattleSounds, 0, None, &[("provinces", Cell::U32List(&[11]))]),
        _ => {}
    }
}

fn layer(side: &MapSide) -> &Sounds {
    side.parts().unwrap().1[0].any().downcast_ref::<Sounds>().unwrap()
}

fn composed(side: &MapSide) -> Option<Battles> {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    fresh.product()
}

fn product(side: &MapSide) -> Option<Battles> {
    layer(side).product()
}

#[test]
fn a_combat_changed_twice_in_a_step_is_marked_once_and_settles_to_the_composed_battles() {
    let entry = open(units(record(24, campaign), "sounds"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    sought(&mut side, 0);
    let first = product(&side).unwrap();
    assert_eq!(first.battles.len(), 1);
    assert_eq!((first.battles[0].key, first.battles[0].divisions), (77, 4), "province 10 counted as often as the battle lists it");
    assert_eq!(first.battles[0].sub_units["name 0"], 12);
    assert_eq!(first.refusals, Vec::<String>::new());
    let mut applied = 0;
    let mut checked = Vec::new();
    play(&mut side, 0, |side, frame| {
        let hour = frame.applied.unwrap();
        let before = applied;
        applied = hour;
        let marked = layer(side).marked();
        assert_eq!(product(side), composed(side), "settled equals composed at hour {hour}");
        checked.push((before, hour, marked));
        hour >= 23
    });
    let (_, _, end) = *checked.last().unwrap();
    assert!(end[0] >= 3, "the combats' changes were marked: {end:?}");
    let last = product(&side).unwrap();
    assert_eq!(last.battles[0].provinces, [11]);
    assert_eq!((last.battles[0].divisions, last.battles[0].sub_units["name 0"]), (1, 3));
    assert_eq!(last.refusals, Vec::<String>::new());
}

#[test]
fn the_newest_values_of_a_combat_changed_twice_show_once_settled() {
    let entry = open(units(record(24, campaign), "sounds twice"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap();
    sought(&mut side, 0);
    let mut seen = None;
    let mut marked_before = layer(&side).marked();
    play(&mut side, 0, |side, frame| {
        let hour = frame.applied.unwrap();
        let marked = layer(side).marked();
        if (6..9).contains(&hour) && seen.is_none() {
            seen = Some((marked[0] - marked_before[0], product(side).unwrap()));
        }
        marked_before = marked;
        hour >= 8
    });
    let (combats, battles) = seen.expect("a step covered hours 5 and 6");
    assert_eq!(combats, 1, "combat 900 changed in hour 5 and through its division in hour 6, marked once");
    let battle = &battles.battles[0];
    assert_eq!(battle.divisions, 6, "three units, province 10 listed twice");
    assert_eq!(battle.sub_units["name 0"], 6, "only 1001 has a composition after hour 6");
    assert_eq!(battles.refusals, ["4 combat divisions have no recorded template composition or sub-unit name, so their sub-units do not count toward battle layers"]);
}

#[test]
fn a_table_not_recorded_is_drawn_as_today_and_a_held_settle_catches_up_with_the_newest() {
    let entry = open(units(record(24, campaign), "sounds gap"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap();
    sought(&mut side, 0);
    let mut gap = None;
    play(&mut side, 0, |side, frame| {
        let hour = frame.applied.unwrap();
        if (15..18).contains(&hour) {
            gap = product(side);
        }
        hour >= 15
    });
    let gap = gap.expect("a step ended inside the stretch");
    assert_eq!(gap.refusals, ["the recorded LandCombats table is unavailable (not recorded at this hour)"]);
    assert_eq!(gap.battles[0].divisions, 0, "no combats are read from a table not recorded, never the last rows");
    let settles = side.hold_settles();
    let before = product(&side).unwrap().battles;
    let at = side.parts().unwrap().0.get(TableId::LandCombats).unwrap().head().unwrap();
    play(&mut side, at, |_, frame| frame.layers[0].dirty > 0);
    assert_eq!(product(&side).unwrap().battles, before, "nothing is made while the settles are held");
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    let at = side.parts().unwrap().0.get(TableId::LandCombats).unwrap().head().unwrap();
    play(&mut side, at, |_, frame| frame.layers[0].dirty == 0);
    assert_eq!(product(&side), composed(&side));
    assert_eq!(product(&side).unwrap().battles[0].divisions, 1);
}

#[test]
fn content_without_battle_defines_or_a_silent_view_declares_nothing() {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut silent = Sounds::new(Some(audio(false)));
    let mut inputs = Inputs::new(&kinds, "sounds");
    silent.inputs(&View::default(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty());
    let mut none = Sounds::new(None);
    let mut inputs = Inputs::new(&kinds, "sounds");
    none.inputs(&View::default(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty());
    let mut muted = Sounds::new(Some(audio(true)));
    let mut inputs = Inputs::new(&kinds, "sounds");
    muted.inputs(&View { sound: false, ..View::default() }, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "battle ambience off declares nothing");
    let mut drawn = Sounds::new(Some(audio(true)));
    let mut inputs = Inputs::new(&kinds, "sounds");
    drawn.inputs(&View::default(), &mut inputs).unwrap();
    let tables = inputs.into_tables();
    assert_eq!(tables.keys().copied().collect::<Vec<_>>(), {
        let mut wanted: Vec<usize> = SOUND_TABLES.iter().map(|table| table.index()).collect();
        wanted.sort_unstable();
        wanted
    });
}

fn stepped(hours: u32, name: &str) -> MapSide {
    let entry = open(units(record(hours, campaign), name), viewer_layer_core::playhead_host::settings()).unwrap();
    MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap()
}

#[test]
fn a_stepped_head_crosses_a_block_end_at_24_hour_steps() {
    let first = BLOCK as i32 - 100;
    let mut side = stepped(BLOCK + 60, "stepped block end");
    side.anchor(first, false, 0.0, true);
    side.catch_up(first).unwrap();
    for hour in (first..BLOCK as i32 + 48).step_by(24).skip(1) {
        side.catch_up(hour).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
    }
}

fn renumbered(at: u32, store: &mut Store) {
    complete(store, at, &SOUND_TABLES);
    let row = |object: u64, province: u32| -> [(&str, Cell<'static>); 4] {
        let provinces: &'static [u32] = if province == 10 { &[10] } else { &[11] };
        [("center_x", Cell::U32(province)), ("radius", Cell::U32(2.0f32.to_bits())), ("provinces", Cell::U32List(provinces)), ("object", Cell::U64(object))]
    };
    match at {
        0 => {
            put(store, TableId::BattleSounds, 0, Some(0), &row(77, 10));
            put(store, TableId::BattleSounds, 1, Some(1), &row(88, 11));
        }
        5 => {
            put(store, TableId::BattleSounds, 0, None, &row(88, 11));
            store.table(TableId::BattleSounds.index()).unwrap().delete(1).unwrap();
        }
        _ => {}
    }
}

#[test]
fn a_battle_the_list_renumbers_keeps_its_battle_and_is_not_made_again() {
    let entry = open(units(record(12, renumbered), "sounds renumbered"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(super::SoundsFed::new(Sounds::new(Some(audio(true)))))], View::default()).unwrap();
    let composed = |side: &MapSide| {
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = fed(side).fresh();
        fresh.compose(mirrors).unwrap();
        fresh.product()
    };
    sought(&mut side, 0);
    let made = fed(&side).made();
    assert_eq!(made, 2);
    assert_eq!(super::drawn(&side).flatten().as_deref(), composed(&side).as_ref(), "a reset publishes the sought hour's mix at once");
    play(&mut side, 0, |_, frame| frame.applied.unwrap() >= 8);
    side.settle_all().unwrap();
    let product = fed(&side).battles().map(|battles| (**battles).clone()).unwrap();
    assert_eq!(product.battles.iter().map(|battle| (battle.key, battle.provinces.clone())).collect::<Vec<_>>(), [(88, vec![11])]);
    assert_eq!(fed(&side).made(), made, "battle 88 moved to ordinal 0 without being made again");
    assert_eq!(Some(product.clone()), composed(&side));
    assert_eq!(super::drawn(&side).flatten().as_deref(), Some(&product), "the worker's mix reaches the head's battles");
}

fn template_only(at: u32, store: &mut Store) {
    if at != 5 {
        campaign(at, store);
    } else {
        complete(store, at, &SOUND_TABLES);
    }
}

#[test]
fn a_division_whose_template_changes_alone_changes_its_front() {
    let entry = open(units(record(12, template_only), "sounds template"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap();
    sought(&mut side, 0);
    assert_eq!(product(&side).unwrap().battles[0].sub_units["name 0"], 12);
    play(&mut side, 0, |_, frame| frame.applied.unwrap() >= 8);
    let battles = product(&side).unwrap();
    assert_eq!(battles.battles[0].sub_units["name 0"], 6, "1002 lost its composition at hour 6: province 10, listed twice, counts 1001's three");
    assert_eq!(Some(battles), composed(&side));
    let layer = side.parts().unwrap().1[0].any().downcast_ref::<Sounds>().unwrap();
    assert_eq!(layer.applied_reads(), 0, "its applies only mark: the division whose template moved is read at the settle");
}

fn moved(at: u32, store: &mut Store) {
    complete(store, at, &SOUND_TABLES);
    let template = grids(7, 3);
    let row = |object: u64, provinces: &'static [u32]| -> [(&str, Cell<'static>); 4] { [("center_x", Cell::U32(object as u32)), ("radius", Cell::U32(2.0f32.to_bits())), ("provinces", Cell::U32List(provinces)), ("object", Cell::U64(object))] };
    match at {
        0 => {
            put(store, TableId::LexicalTokens, 0, Some(7), &[("literal", Cell::Bytes(0))]);
            put(store, TableId::DivisionTemplateIcons, 0, Some(50), &[("model_unreadable", Cell::U8(0)), ("grids", Cell::Opaque(&template))]);
            for (slot, division) in [1001u128, 1002, 1004].into_iter().enumerate() {
                put(store, TableId::Divisions, slot as u32, Some(division), &[("template", Cell::U64(50))]);
            }
            put(store, TableId::LandCombats, 0, Some(900), &[("province", Cell::U32(10)), ("attackers", Cell::U64List(&[1001])), ("attacker_classes", Cell::U32List(&[0])), ("defenders", Cell::U64List(&[1002])), ("defender_classes", Cell::U32List(&[0]))]);
            put(store, TableId::BattleSounds, 0, Some(0), &row(77, &[10]));
        }
        3 => {
            put(store, TableId::BattleSounds, 0, None, &row(88, &[11]));
            put(store, TableId::BattleSounds, 1, Some(1), &row(77, &[10]));
        }
        6 => put(store, TableId::LandCombats, 0, None, &[("attackers", Cell::U64List(&[1001, 1004])), ("attacker_classes", Cell::U32List(&[0, 0]))]),
        _ => {}
    }
}

#[test]
fn a_battle_carried_between_slots_while_its_front_changes_is_made_again() {
    let entry = open(units(record(12, moved), "sounds moved"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Sounds::new(Some(audio(true))))], View::default()).unwrap();
    sought(&mut side, 0);
    let mut stepped = {
        let (mirrors, _) = side.parts().unwrap();
        let mut stepped = layer(&side).fresh();
        stepped.compose(mirrors).unwrap();
        stepped
    };
    assert_eq!(stepped.product().unwrap().battles.iter().map(|battle| (battle.key, battle.divisions)).collect::<Vec<_>>(), [(77, 2)]);
    play(&mut side, 0, |_, frame| frame.applied.unwrap() >= 4);
    stepped.read_battle(0, side.parts().unwrap().0);
    play(&mut side, 4, |_, frame| frame.applied.unwrap() >= 8);
    let (mirrors, _) = side.parts().unwrap();
    stepped.mark_combat(0);
    stepped.mark_battle(1);
    stepped.settle_dirty(mirrors);
    let battles = stepped.product().unwrap();
    assert_eq!(battles.battles.iter().map(|battle| (battle.key, battle.divisions)).collect::<Vec<_>>(), [(88, 0), (77, 3)], "battle 77 counts the division that joined while it moved");
    assert_eq!(battles.battles[1].sub_units["name 0"], 9);
    assert_eq!(Some(battles), composed(&side));
}

fn fed(side: &MapSide) -> &super::SoundsFed {
    side.parts().unwrap().1[0].any().downcast_ref::<super::SoundsFed>().unwrap()
}

const LAST: replay_data::Hour = 23;

#[test]
fn on_their_worker_the_battles_equal_composed_at_each_hour() {
    let entry = open(units(record(24, campaign), "sounds fed"), quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(super::SoundsFed::new(Sounds::new(Some(audio(true)))))], View::default()).unwrap();
    sought(&mut side, 0);
    side.entry().playhead().play(4.0);
    let mut checked = Vec::new();
    for _ in 0..50_000_000u64 {
        viewer_layer_core::test_support::recorded::reads_landed(&side);
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        let Some(hour) = frame.applied.filter(|_| frame.steps > 0 && !side.composing()) else {
            if frame.steps == 0 && frame.swapped.is_none() {
                std::thread::yield_now();
            }
            continue;
        };
        checked.push(hour);
        side.settle_all().unwrap();
        assert_eq!(Layer::published(fed(&side)), viewer_layer_core::layers::Published::Current, "hour {hour}");
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = fed(&side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(fed(&side).battles().map(Arc::as_ref), fresh.product().as_ref(), "hour {hour}: the worker's battles against those composed from the frame's mirrors");
        if hour >= LAST {
            break;
        }
    }
    assert_eq!(checked.last(), Some(&LAST), "the map side never applied the fixture's last hour; hours checked {checked:?}");
    let last = fed(&side).battles().expect("the battles sound");
    assert_eq!(last.battles[0].provinces, [11]);
    assert_eq!(last.battles[0].divisions, 1);
    assert!(fed(&side).marked()[0] >= 3, "the combats' changes were marked on the worker: {:?}", fed(&side).marked());
}
