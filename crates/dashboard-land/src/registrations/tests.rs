use super::*;
use dashboard_base::{Source, SourceKey, products::{LoadContext, Registered, at_hour::{AtHour, frame_panel}}, test_fixtures::{CAMPAIGN, materialized_frame, ownership_frame, played, records, streamed}};
use replay::campaign::Replay;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::sync::Arc;

fn frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = ownership_frame(branch, hour, 9001);
    for table in TableId::ALL {
        if !table.is_derived() && table.key_bits() != 0 && matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
            *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
        }
    }
    let change = i32::from(hour >= 107);
    if [100, 107, 115].contains(&hour) { frame.metrics.table_ns[TableId::DeploymentLines.index()] = 1; }
    let amount = hour + branch as i32 * 100;
    let mut set = |table: TableId, rows| {
        *frame.tables[table.index()] = if (113..115).contains(&hour) {
            TableView::Unavailable { reason: "damaged land table".into() }
        } else { TableView::Complete(rows) };
    };
    set(TableId::CombatLogGroups, TableRows::CombatLogGroups(vec![CombatLogGroupRow { country: 9001, group: 1 << 32, ..Default::default() }]));
    set(TableId::OrdersGroups, TableRows::OrdersGroups(vec![OrdersGroupRow { handle: 1 << 32, country: 9001, ..Default::default() }]));
    set(TableId::CombatLogManpower, TableRows::CombatLogManpower(vec![
        CombatLogManpowerRow { country: 44, group: 1, losses_0: hour, ..Default::default() },
        CombatLogManpowerRow { country: 9001, group: 1, date: 43_800_000, losses_0: amount, losses_1: change, ..Default::default() },
    ]));
    let leader = UnitLeaderRow { id: 1, owner: 9001, kind: crate::products::leaders::KIND_ARMY, skill_valid: 1,
        skill_level: 3 + change, experience: i64::from(amount), ..Default::default() };
    let mut leaders = vec![leader.clone()];
    if branch == 2 { leaders.push(UnitLeaderRow { id: 2, skill_level: 8, ..leader }); }
    set(TableId::UnitLeaders, TableRows::UnitLeaders(leaders));
    set(TableId::DivisionTemplates, TableRows::DivisionTemplates(vec![
        DivisionTemplateRow { template: 1, owner: 9001, ..Default::default() },
        DivisionTemplateRow { template: 2, owner: 44, ..Default::default() },
    ]));
    set(TableId::DivisionTemplateIcons, TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
        template: 1, settings: Recorded::Value(Some(Arc::new(TemplateSettings { parent: 2, priority: change, ..Default::default() }))),
        base_width: Recorded::Value(Some(10 + i64::from(change))), ..Default::default()
    }, DivisionTemplateIconRow { template: 2, ..Default::default() }]));
    set(TableId::DeploymentConveyors, TableRows::DeploymentConveyors(vec![
        DeploymentConveyorRow { handle: 1, country: 9001, template: 1, amount: 1 + change, ..Default::default() },
        DeploymentConveyorRow { handle: 2, country: 44, ..Default::default() },
    ]));
    set(TableId::DeploymentLines, TableRows::DeploymentLines(vec![
        DeploymentLineRow { handle: 7, conveyor: 1, ..Default::default() },
        DeploymentLineRow { handle: 8, conveyor: 2, ..Default::default() },
    ]));
    set(TableId::DeploymentManpower, TableRows::DeploymentManpower(vec![
        DeploymentManpowerRow { line: 7, source_country: 9001, amount: 50 + change, ..Default::default() },
        DeploymentManpowerRow { line: 8, source_country: 44, amount: hour, ..Default::default() },
    ]));
    set(TableId::DoctrineTracks, TableRows::DoctrineTracks(vec![
        DoctrineTrackRow { slot: 7, sub_doctrine: (change != 0).then_some(120), mastery: i64::from(hour), ..Default::default() },
        DoctrineTrackRow { slot: 8, mastery: i64::from(hour), ..Default::default() },
    ]));
    set(TableId::LexicalTokens, TableRows::LexicalTokens(vec![LexicalTokenRow { token: 120, literal: b"mod_sub".to_vec() }]));
    frame
}

fn recording(branches: &[u64]) -> Arc<Source> {
    let records: Vec<_> = branches.iter().map(|&branch| records(branch, (100..=120).map(|hour| frame(branch, hour)).collect())).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &records).unwrap().into_timelines(&Default::default()).unwrap();
    Arc::new(Source::from_replay(SourceKey { source_id: "land".into(), revision: "1".into() }, "land.battleplan".into(), Arc::new(replay), &false.into()).unwrap())
}

fn played_structure(key: &str, data: &dashboard_follow::panel::PanelData, fresh: bool) -> dashboard_follow::panel::PanelData {
    let mut data = streamed(data, fresh);
    if key == "army-losses" && !fresh && let Ok(reply) = &mut data
        && reply.pointer("/rollup/months/0/liveRow").is_some_and(|row| row == "r:0")
    {
        assert_eq!(reply["live"]["rows"], json!(["r:0"]));
        assert_eq!(reply["live"]["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect::<Vec<_>>(), ["p0", "p1", "p2", "total"]);
        let bar = &mut reply["rollup"]["months"][0];
        bar["values"] = Value::Null;
        bar["total"] = Value::Null;
    }
    data
}

fn follows<P: AtHour>(registration: Registered, mut request: Value, live: bool) {
    let load = LoadContext { content: None };
    let source = recording(&[1, 2]);
    let mut seeds = Vec::new();
    for branch in [1, 2] {
        request["branchId"] = branch.to_string().into();
        let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
        let mut original = frame_panel::<P>(&source, &load, &request).unwrap();
        let exact = panel.exact().unwrap();
        for hour in [100, 107, 114, 100] {
            let read = (exact.advance)(hour, true, &false.into(), None);
            assert!(read.fresh || read.value.is_err());
            assert_eq!(*read.value, materialized_frame(&mut *original, source.replay_ref(), branch, hour), "{} branch {branch} at {hour}", P::KEY);
            if live && hour != 114 {
                let values: Vec<_> = read.value.as_ref().as_ref().unwrap()["live"]["groups"].as_array().unwrap().iter()
                    .flat_map(|group| group["values"].as_array().unwrap()).collect();
                assert!(!values.is_empty());
                seeds.push(serde_json::to_value(values).unwrap());
            }
        }
    }
    if live {
        assert_eq!(seeds[0], seeds[2], "seek restores the first seed");
        assert_ne!(seeds[0], seeds[1], "seek seeds the new hour");
        assert_ne!(seeds[2], seeds[3], "the other branch seeds its own values");
    }
    let source = recording(&[1]);
    request["branchId"] = "1".into();
    played_replies::<P>(registration, &source, &load, &request);
}

fn played_replies<P: AtHour>(registration: Registered, source: &Arc<Source>, load: &LoadContext, request: &Value) {
    let panel = (registration.panel.unwrap().open)(source, load, request).unwrap();
    let mut original = frame_panel::<P>(source, load, request).unwrap();
    let mut previous = None;
    for read in played(source.replay_ref(), panel.exact().unwrap(), 100, 120) {
        let expected = materialized_frame(&mut *original, source.replay_ref(), 1, read.hour);
        assert_eq!(played_structure(P::KEY, &read.value, read.fresh), played_structure(P::KEY, &expected, read.fresh), "{} at {}", P::KEY, read.hour);
        if P::KEY == "army-losses" && let Ok(reply) = &expected
            && let Some(values) = reply.pointer("/live/groups/0/values").and_then(Value::as_array)
        {
            let bar = &reply["rollup"]["months"][0];
            let resolved: Vec<_> = values.iter().map(|v| v.as_i64().unwrap()).collect();
            assert_eq!(bar["values"], json!(resolved.iter().map(ToString::to_string).collect::<Vec<_>>()));
            assert_eq!(bar["total"], resolved.iter().sum::<i64>().to_string());
        }
        let structure = played_structure(P::KEY, &expected, false);
        if read.hour > 100 && let Some(previous) = &previous {
            assert_eq!(read.product_calls, u64::from(previous != &structure), "{} calls at {}: previous {previous:?}, current {structure:?}", P::KEY, read.hour);
        }
        previous = Some(structure);
    }
}

#[test]
fn army_losses_follows_the_selected_country_and_seeds_live_losses_on_seek_or_branch_change() {
    follows::<crate::products::army_losses::ArmyLosses>(ARMY_LOSSES_ARMYLOSSES, json!({"country":"GER", "view":{"kind":"manpower"}}), true);
}

#[test]
fn leader_experience_seeds_the_selected_hour_and_branch() {
    follows::<crate::products::leaders::career::LeaderHour>(LEADERS_CAREER_LEADERHOUR, json!({"reference":"4294967296"}), true);
    let source = recording(&[1, 2]);
    let load = LoadContext { content: None };
    for (branch, winner) in [(1, "4294967296"), (2, "8589934592"), (1, "4294967296")] {
        for (registration, mut request, reference) in [
            (LEADERS_CAREER_LEADERHOUR, json!({"selection":{"kind":"all"}}), "/reference"),
            (LEADERS_UNITLEADERS, json!({"selection":{"kind":"all"}, "leaders":"land"}), "/leaders/0/reference"),
        ] {
            request["branchId"] = branch.to_string().into();
            let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
            let reader = panel.exact().unwrap();
            for hour in [107, 114, 100] {
                let painted = (reader.advance)(hour, true, &false.into(), None);
                assert_eq!(*painted.value, (reader.cold)(hour, &false.into()), "{request} at {hour}");
                if hour != 114 {
                    assert_eq!(painted.value.as_ref().as_ref().unwrap().pointer(reference).and_then(Value::as_str), Some(winner));
                }
            }
        }
    }
}

#[test]
fn division_templates_follows_the_selected_countrys_template_fields() {
    follows::<crate::products::divisions::templates::DivisionTemplates>(DIVISIONS_TEMPLATES_DIVISIONTEMPLATES, json!({"country":"GER", "countFor":"owner"}), false);
}

#[test]
fn division_templates_recompute_for_their_own_grid_tokens_and_ignore_other_templates() {
    let frames = (100..=120).map(|hour| {
        let mut frame = frame(1, hour);
        if let TableView::Complete(TableRows::DivisionTemplateIcons(icons)) = &mut *frame.tables[TableId::DivisionTemplateIcons.index()] {
            let grid = |token| Recorded::Value(TemplateGrids {
                regimental_support: vec![TemplateSubunit { token, count: 1, local: 0 }],
                ..Default::default()
            });
            icons[0].grids = grid(if hour < 110 { 120 } else { 121 });
            icons[1].grids = grid(121);
        }
        if let TableView::Complete(TableRows::DivisionTemplates(templates)) = &mut *frame.tables[TableId::DivisionTemplates.index()] {
            templates[0].unit_type = i32::from(hour >= 103);
            templates[1].name = if hour < 105 { b"Other".to_vec() } else { b"Renamed".to_vec() };
        }
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow { token: 120, literal: b"unit_one".to_vec() },
            LexicalTokenRow { token: 121, literal: match hour {
                ..104 => b"foreign_one".to_vec(),
                ..111 => b"foreign_two".to_vec(),
                _ => b"selected_unit".to_vec(),
            } },
        ]));
        frame
    }).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Arc::new(Source::from_replay(SourceKey { source_id: "template-names".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
    let request = json!({"branchId":"1", "country":"GER", "countFor":"owner"});
    let panel = (DIVISIONS_TEMPLATES_DIVISIONTEMPLATES.panel.unwrap().open)(&source, &LoadContext { content: None }, &request).unwrap();
    let reads = played(source.replay_ref(), panel.exact().unwrap(), 100, 112);
    let at = |hour| reads.iter().find(|read| read.hour == hour).unwrap();
    assert_eq!(at(103).product_calls, 0, "the template's unit type is outside the reply");
    assert_eq!(at(104).product_calls, 0, "a token used by another owner's grid is outside the reply");
    assert_eq!(at(105).product_calls, 0, "a foreign template name is outside the reply");
    assert_eq!(at(110).product_calls, 1, "a displayed subunit token changes the reply");
    assert_eq!(at(110).value.as_ref().as_ref().unwrap()["templates"][0]["units"][0]["key"], "foreign_two");
    assert_eq!(at(111).product_calls, 1, "the selected grid now needs this token");
    assert_eq!(at(111).value.as_ref().as_ref().unwrap()["templates"][0]["units"][0]["key"], "selected_unit");
}

#[test]
fn training_queue_tracks_owned_orders_lines_and_manpower() {
    follows::<crate::products::training_queue::TrainingQueue>(TRAINING_QUEUE_TRAININGQUEUE, json!({"country":"GER"}), false);
}

#[test]
fn division_equipment_tracks_the_selected_countrys_equipment() {
    for view in ["types", "templates"] {
        follows::<crate::products::divisions::equipment::DivisionEquipment>(DIVISIONS_EQUIPMENT_DIVISIONEQUIPMENT,
            json!({"country":"GER", "countFor":"owner", "view":view}), false);
    }
}

#[test]
fn roster_and_rewards_follow_the_selected_template_settings_and_doctrine_track() {
    use dashboard_base::test_fixtures::{source_with_content, visual_mods::Machine};
    let machine = Machine::new("land_panel_inputs");
    machine.write("common/defines/test.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write("common/doctrines/subdoctrines/test.txt", b"mod_sub={rewards={tier={mastery=2}}}");
    let (source, load) = source_with_content((100..=120).map(|hour| frame(1, hour)).collect(), &machine.root);
    let source = Arc::new(source);
    played_replies::<crate::products::army::TemplateRoster>(ARMY_TEMPLATEROSTER, &source, &load, &json!({"branchId":"1", "country":"GER"}));
    played_replies::<crate::products::doctrine_mastery::rewards::DoctrineRewards>(DOCTRINE_MASTERY_REWARDS_DOCTRINEREWARDS, &source, &load,
        &json!({"branchId":"1", "country":"GER", "folder":0, "track":0}));
}

#[test]
fn training_locations_follow_only_the_state_of_each_queued_province() {
    use dashboard_base::test_fixtures::{source_with_content, visual_mods::Machine};
    let machine = Machine::new("land_queue_locations");
    machine.write("common/defines/test.lua", b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}");
    machine.write("interface/queue_fixture.gui", br#"
        guiTypes = { containerWindowType = {
            name = "military_deployment_conveyor_view"
            buttonType = { name = "low_prio" pdx_tooltip = "queue_fixture_priority" }
        } }
    "#);
    machine.write("localisation/queue_fixture_l_english.yml", "\u{feff}l_english:\n queue_fixture_priority:0 \"Wait your turn\"\n".as_bytes());
    let frames = (100..=120).map(|hour| {
        let mut frame = frame(1, hour);
        if let TableView::Complete(TableRows::DeploymentConveyors(orders)) = &mut *frame.tables[TableId::DeploymentConveyors.index()] {
            orders[0].location = if hour < 108 { 1 } else { 3 };
        }
        *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
            ProvinceRow { id: 1, state: 1, controller: hour as u32, ..Default::default() },
            ProvinceRow { id: 2, state: hour as u32, controller: hour as u32, ..Default::default() },
            ProvinceRow { id: 3, state: if hour < 110 { 2 } else { 1 }, ..Default::default() },
        ]));
        let second_name = match hour {
            100 => "Old second camp",
            101..=108 => "Second camp",
            _ => "Renamed camp",
        };
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
            StateRow { id: 1, custom_name: b"First camp".to_vec(), ..Default::default() },
            StateRow { id: 2, custom_name: second_name.as_bytes().to_vec(), ..Default::default() },
        ]));
        frame
    }).collect();
    let (source, load) = source_with_content(frames, &machine.root);
    let source = Arc::new(source);
    let request = json!({"branchId":"1", "country":"GER"});
    let panel = (TRAINING_QUEUE_TRAININGQUEUE.panel.unwrap().open)(&source, &load, &request).unwrap();
    let exact = &panel.exact().unwrap().cold;
    let (_, reads) = replay_recording::cells::count_reads(|| {
        for (hour, name) in [(100, "First camp"), (108, "Second camp")] {
            let reply = exact(hour, &false.into()).unwrap();
            assert_eq!(reply["orders"][0]["locationName"], name);
            assert_eq!(reply["orders"][0]["priorityName"], "Wait your turn");
        }
    });
    assert_eq!(reads.get(&TableId::ActiveIdeas).map_or(0, |read| read.opened), 0, "plain deployment names do not open the scripted world on either read");
    assert_eq!(exact(110, &false.into()).unwrap()["orders"][0]["locationName"], "First camp");
    played_replies::<crate::products::training_queue::TrainingQueue>(TRAINING_QUEUE_TRAININGQUEUE, &source, &load, &request);
}
