use super::*;
use dashboard_base::{Source, SourceKey, products::{LoadContext, Registered, at_hour::{AtHour, frame_panel}}, test_fixtures::{content_identity, materialized_frame, played, records, source, visual_mods::Machine, with_facts}};
use replay_capture_spool::spool::Record;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::Arc;

fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = source(branch, hour, Some(i64::from(hour)));
    for table in TableId::ALL {
        if !table.is_derived() && table.key_bits() != 0 && matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    let strength = 100 * branch as i64 + i64::from(hour >= 106);
    let mut set = |table: TableId, rows| *frame.tables[table.index()] = TableView::Complete(rows);
    set(TableId::NavalRegionalMines, TableRows::NavalRegionalMines(vec![
        NavalRegionalMineRow { country: 9001, kind: 0, region: 30, amount: 0 },
        NavalRegionalMineRow { country: 9001, kind: 1, region: 7, amount: strength },
    ]));
    set(TableId::NavalBases, TableRows::NavalBases(vec![NavalBaseRow { province: 9, country: 11, ..Default::default() }]));
    set(TableId::NavalRepairBases, TableRows::NavalRepairBases(vec![NavalRepairBaseRow { province: 9, priority: 2, allocated_dockyards: 4 }]));
    set(TableId::NavalRepairQueue, TableRows::NavalRepairQueue(vec![NavalRepairQueueRow { province: 9, ordinal: 0, ship: 42 }]));
    let mut ships = vec![ShipRow { id: 42, task_force: 5, strength, ..Default::default() }];
    if hour >= 108 { ships.push(ShipRow { id: 63, task_force: 600, ..Default::default() }); }
    set(TableId::Ships, TableRows::Ships(ships));
    set(TableId::ShipModels, TableRows::ShipModels(vec![ShipModelRow { ship: 42, variant: 27, ..Default::default() }]));
    set(TableId::TaskForces, TableRows::TaskForces(vec![TaskForceRow { id: 5, roster_owner: 9001, ..Default::default() }]));
    set(TableId::NavalRefitProgress, TableRows::NavalRefitProgress(vec![
        NavalRefitProgressRow { line: 8, progress_raw: strength },
        NavalRefitProgressRow { line: 19, progress_raw: i64::from(hour) },
    ]));
    set(TableId::NavalRefitShips, TableRows::NavalRefitShips(vec![NavalRefitShipRow { ship: 42, line: 8 }]));
    set(TableId::ProductionLines, TableRows::ProductionLines(vec![
        ProductionLineRow { handle: 8, country: 9001, cost_raw: 500, active_factories: 2, ..Default::default() },
        ProductionLineRow { handle: 19, country: 42, ..Default::default() },
    ]));
    set(TableId::NavalMissionLedgers, TableRows::NavalMissionLedgers(vec![NavalMissionLedgerRow { owner: 9001, region: 7, task_forces: vec![5, 600], ..Default::default() }]));
    for (table, from) in [(TableId::NavalRegionalMines,110),(TableId::NavalBases,113),(TableId::NavalRefitProgress,116),(TableId::NavalRepairQueue,119),(TableId::VariantDefinitions,103)] {
        if (from..from+2).contains(&hour) { *frame.tables[table.index()] = TableView::Unavailable { reason: "damaged fleet record".into() }; }
    }
    frame
}

fn recording(name: &str, branches: &[u64]) -> (Machine, Arc<Source>, LoadContext) {
    let machine = Machine::new(name);
    machine.write("hoi4.exe", b"test");
    machine.write("user/settings.txt", b"language=l_english force_pow2_textures=no");
    machine.write("user/dlc_load.json", br#"{"enabled_mods":[],"disabled_dlcs":[]}"#);
    let (options, identity) = content_identity(&machine.root);
    let mut input = Vec::new();
    for &branch in branches {
        let mut part = records(branch, (100..=122).map(|hour| frame(branch, hour)).collect());
        let Record::SessionStart(session) = &mut part[0] else { unreachable!() };
        session.identity = identity.clone();
        input.extend(part);
    }
    let replay = with_facts(&options, &identity, input).into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: name.into(), revision: "1".into() }, "naval.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
    (machine, Arc::new(source), LoadContext { content: Some(options) })
}

fn follows<P: AtHour>(registration: Registered, mut asked: Value) {
    let (_content, source, load) = recording(P::KEY, &[1, 2]);
    asked["panel"] = P::KEY.into();
    let mut seeds = Vec::new();
    for branch in [1,2] {
        asked["branchId"] = branch.to_string().into();
        let panel = (registration.panel.unwrap().open)(&source, &load, &asked).unwrap();
        let exact = panel.exact().expect("registered exact reader");
        let mut original = frame_panel::<P>(&source, &load, &asked).unwrap();
        for (hour, seed) in (100..=122).map(|hour| (hour, hour == 100)).chain([(106,true),(100,true)]) {
            let old = materialized_frame(&mut *original, source.replay_ref(), branch, hour);
            let step = (exact.advance)(hour, seed, &false.into(), None);
            if seed {
                assert!(step.fresh);
                let groups = step.value.as_ref().as_ref().unwrap()["live"]["groups"].as_array().unwrap();
                let values: Vec<_> = groups.iter().flat_map(|group| group["values"].as_array().unwrap()).filter_map(Value::as_i64).collect();
                assert!(!values.is_empty());
                seeds.push(values);
            }
            assert_eq!(*(exact.advance)(hour, true, &false.into(), None).value, old, "{} branch {branch} at {hour}", P::KEY);
        }
    }
    assert_eq!(seeds[0], seeds[2]);
    assert_ne!(seeds[0], seeds[1]);
    assert_ne!(seeds[2], seeds[3]);
    let (_single_content, source, load) = recording(&format!("{}-steps", P::KEY), &[1]);
    asked["branchId"] = "1".into();
    let panel = (registration.panel.unwrap().open)(&source, &load, &asked).unwrap();
    let reads = played(source.replay_ref(), panel.exact().unwrap(), 100, 122);
    let mut original = frame_panel::<P>(&source, &load, &asked).unwrap();
    let mut previous = None;
    let streamed = dashboard_base::test_fixtures::streamed;
    for read in reads {
        let value = materialized_frame(&mut *original, source.replay_ref(), 1, read.hour);
        assert_eq!(streamed(&read.value, read.fresh), streamed(&value, read.fresh), "{} played at {}", P::KEY, read.hour);
        if let Some(previous) = &previous {
            assert_eq!(read.product_calls, u64::from(previous != &value), "{} at {}", P::KEY, read.hour);
        }
        previous = Some(value);
    }
}

#[test]
fn minefields_follow_the_shown_countries_and_reseed_on_seek_and_branch_change() {
    follows::<crate::products::minefields::Minefields>(MINEFIELDS_MINEFIELDS, json!({"pinnedRegions":[7]}));
}

#[test]
fn repairs_follow_the_selected_ports_and_reseed_on_seek_and_branch_change() {
    follows::<crate::products::naval_yards::NavalRepairs>(NAVAL_YARDS_NAVALREPAIRS, json!({"country":"GER"}));
}

#[test]
fn refits_follow_the_selected_lines_and_reseed_on_seek_and_branch_change() {
    follows::<crate::products::naval_yards::NavalRefits>(NAVAL_YARDS_NAVALREFITS, json!({"country":"GER"}));
}

#[test]
fn management_follows_the_selected_fleets_and_reseeds_on_seek_and_branch_change() {
    follows::<crate::products::naval_management::NavalManagement>(NAVAL_MANAGEMENT_NAVALMANAGEMENT, json!({"country":"GER"}));
}

#[test]
fn current_day_supremacy_follows_control_and_only_the_ships_in_the_shown_regions() {
    use crate::products::naval_supremacy::NavalSupremacy;
    let (_content, source, load) = recording("current-naval-day", &[1, 2]);
    for branch in [1, 2] {
        for regions in [json!({"regions":[7]}), json!({"top":0})] {
            let mut request = regions;
            request["branchId"] = branch.to_string().into();
            request["country"] = "GER".into();
            request["panel"] = "naval-supremacy".into();
            let panel = (NAVAL_SUPREMACY_NAVALSUPREMACY.panel.unwrap().open)(&source, &load, &request).unwrap();
            let prior = dashboard_base::products::at_hour::day_panel::<NavalSupremacy>(&source, &load, &request, "firstHour", "lastHour", |reply| reply.valid_through).unwrap();
            let reader = panel.exact().unwrap();
            let old = prior.exact().unwrap();
            for (hour, seed) in (100..=125).map(|h| (h, h == 100)).chain([(108, true), (100, true)]) {
                let actual = (reader.advance)(hour, seed, &false.into(), None);
                let expected = (old.read)(hour, &false.into());
                assert_eq!(*actual.value, expected, "{request} at {hour}");
                if hour > 100 && hour < 110 && !seed {
                    let shown = request["regions"].is_array();
                    assert_eq!(actual.fresh, shown && hour == 108, "{request} at {hour}");
                }
            }
        }
    }
}
