use super::*;

#[test]
fn day_history_follows_midnight_and_preserves_unrecorded_days() {
    use dashboard_base::{SourceKey, test_fixtures::{replay, source}};
    use serde_json::json;

    let frames = [10, 23, 48, 71, 72].map(|hour| source(1, hour, None));
    let replay = replay(frames.into()).into_timelines(&Default::default()).unwrap();
    let cancelled = false.into();
    let source = Arc::new(Source::from_replay(
        SourceKey { source_id: "day-history".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled,
    ).unwrap());
    let load = LoadContext { content: None };
    let unknown = json!({"branchId": "1", "panel": "unregistered-widget"});
    assert_eq!(open(&source, &load, &unknown).err().unwrap(), "No panel \"unregistered-widget\"");
    let params = json!({"branchId":"1", "panel":"convoy-health", "view":"regions", "country":"GER", "regions":[17], "from":10, "to":71});
    let panel = open(&source, &load, &params).unwrap();
    let exact = panel.exact().unwrap();
    for (hour, last) in [(10, None), (23, None), (24, Some(23)), (48, Some(47)), (71, Some(47)), (72, Some(71)), (10, None)] {
        let read = (exact.advance)(hour, hour == 10, &cancelled, None);
        let expected = if let Some(last) = last {
            let request = json!({"branchId":"1", "view":"regions", "country":"GER", "regions":[17], "from":10, "to":last});
            crate::answer::answer(&source, "convoy-health", &request, &load, &cancelled).unwrap()
        } else { Value::Null };
        assert_eq!(read.value.as_ref().as_ref().unwrap(), &expected, "history at {hour}");
        if hour == 72 {
            assert!(expected["gaps"].as_array().unwrap().iter().any(|gap| gap["firstHour"] == 24 && gap["lastHour"] == 47));
        }
    }
}

#[test]
fn dated_products_seed_advance_confirm_and_reseed_across_a_recording_gap() {
    use dashboard_base::test_fixtures::{source, source_with_content};
    use dashboard_follow::panel::{dated, product::Delivery};
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    use serde_json::json;

    let root = std::env::temp_dir().join(format!("dated-products-{}", uuid::Uuid::new_v4()));
    let defines = root.join("common/defines");
    std::fs::create_dir_all(&defines).unwrap();
    std::fs::write(defines.join("test.lua"), "NDefines={NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NGame={MAX_SCRIPTED_LOC_RECURSION=23},NSupply={MAX_RAILWAY_LEVEL=7}}").unwrap();
    let frames = [100, 101, 102, 103, 110, 111].map(|hour| {
        let mut frame = source(1, hour, None);
        for table in TableId::ALL {
            if !table.is_derived() && table.key_bits() != 0 && matches!(frame.tables[table.index()].held(), TableView::Unavailable { .. }) {
                *frame.tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        let changed = i32::from(hour >= 102);
        let set = |frame: &mut replay_frame::frame::DecodedFrame, table: TableId, rows| {
            *frame.tables[table.index()] = TableView::Complete(rows);
        };
        set(&mut frame, TableId::IntelligenceAgencies, TableRows::IntelligenceAgencies(vec![IntelligenceAgencyRow { country: 9001, created: 1, active_count: changed, ..Default::default() }]));
        set(&mut frame, TableId::CountryGameScoreInputs, TableRows::CountryGameScoreInputs(vec![CountryGameScoreInputRow {
            country: 9001, army_count: Some(changed), navy_count: Some(0), land_count: Some(0),
            air_state: CountryGameScoreInputRow::AIR_ABSENT, ..Default::default()
        }]));
        set(&mut frame, TableId::GameScoreMultipliers, TableRows::GameScoreMultipliers(vec![GameScoreMultiplierRow {
            army_raw: Some(1000), navy_raw: Some(0), land_raw: Some(0), air_raw: Some(0),
            industry_raw: Some(0), total_raw: Some(100_000), ..Default::default()
        }]));
        set(&mut frame, TableId::CountryOccupationReserveManpower, TableRows::CountryOccupationReserveManpower(vec![CountryOccupationReserveManpowerRow { country: 9001, source_country: 11, amount: changed, ..Default::default() }]));
        set(&mut frame, TableId::DoctrineFolders, TableRows::DoctrineFolders(vec![DoctrineFolderRow { slot: 7, folder: 1, ..Default::default() }]));
        set(&mut frame, TableId::DoctrineTracks, TableRows::DoctrineTracks(vec![DoctrineTrackRow { slot: 7, mastery: changed.into(), ..Default::default() }]));
        let mut lines = vec![ProductionLineRow { country: 9001, handle: 1, active_factories: changed, ..Default::default() }];
        if hour >= 102 { lines.push(ProductionLineRow { country: 9001, handle: 2, ..Default::default() }); }
        set(&mut frame, TableId::ProductionLines, TableRows::ProductionLines(lines));
        let states = (1..=12).map(|id| StateRow {
            id, owner: if id == 1 && hour >= 102 { 9001 } else { 0 }, ..Default::default()
        }).collect();
        set(&mut frame, TableId::States, TableRows::States(states));
        set(&mut frame, TableId::ResearchInputs, TableRows::ResearchInputs(vec![ResearchInputRow { country: 9001, banked_days_raw: changed.into(), ..Default::default() }]));
        let mut project = ProjectRow { owner: 9001, local_id: 1, started: 1, ..Default::default() };
        project.state.prototype_progress = changed;
        set(&mut frame, TableId::Projects, TableRows::Projects(vec![project]));
        let mut samples: Vec<_> = [TableId::ResearchInputs, TableId::Projects, TableId::ResearchModifiers, TableId::ResearchAppliedBonuses].map(|table| SliceReadRow { table: table.index() as u32, owner: 9001, sample_hour: 100 + changed * 2 }).into();
        samples.sort_by_key(|row| row.table);
        set(&mut frame, TableId::SliceReads, TableRows::SliceReads(samples));
        frame
    });
    let (source, load) = source_with_content(frames.into(), &root);
    let source = Arc::new(source);
    for name in ["special-project-details", "special-projects", "doctrine-mastery", "research-slots", "production-lines-at", "equipment-designs", "industrial-organisations", "market-lendlease", "nuclear-arsenal", "game-score", "occupation-reserve-manpower", "agency-comparison", "territory-share", "lend-lease-balance", "occupied-states"] {
        let mut params = json!({"branchId": "1", "panel": name});
        match name {
            "nuclear-arsenal" | "game-score" | "occupation-reserve-manpower" | "lend-lease-balance" | "occupied-states" => {}
            "agency-comparison" => params["includeUncreated"] = true.into(),
            "market-lendlease" => params["countries"] = json!(["GER"]),
            "territory-share" => params["measure"] = "ownedStates".into(),
            _ => params["country"] = "GER".into(),
        }
        let panel = open(&source, &load, &params).unwrap_or_else(|error| panic!("{name}: {error}"));
        let exact = panel.exact().unwrap();
        let mut delivery = Delivery::default();
        let mut previous = None;
        for (hour, seek) in [(100, true), (101, false), (102, false), (102, false), (104, false), (106, false), (110, false), (100, true)] {
            let read = (exact.advance)(hour, seek, &false.into(), None);
            let cold = (exact.cold)(hour, &false.into());
            let shown = |data: &dashboard_follow::panel::PanelData, values: bool| data.clone().map(|mut reply| {
                for group in reply.pointer_mut("/live/groups").and_then(Value::as_array_mut).into_iter().flatten() {
                    if !values && let Some(group) = group.as_object_mut() { group.remove("values"); }
                }
                reply
            });
            assert!(read.fresh || !seek, "{name}: a seed reads its live values at its hour");
            let packet = delivery.packet((0, 1, 1), 1, hour, seek, &read.value);
            let mut expected = shown(&cold, read.fresh);
            if name == "special-projects" && let Ok(data) = &*read.value {
                let through = i32::try_from(data["shown"]["through"].as_i64().unwrap()).unwrap();
                assert!(through >= hour);
                expected = (exact.cold)(through, &false.into());
            }
            if name == "special-project-details" && !read.fresh && let (Ok(data), Ok(reply)) = (&*read.value, &mut expected) {
                reply["hour"] = data["hour"].clone();
            }
            if name == "territory-share" && hour == 101 {
                assert!(!packet.changed, "unchanged ownership confirms the hour 100 reply");
                let reply = expected.as_mut().unwrap();
                assert_eq!(reply["hour"], hour);
                reply["hour"] = 100.into();
            }
            assert_eq!(shown(&read.value, read.fresh), expected, "{name} at {hour}");
            if (104..110).contains(&hour) {
                assert!(cold.as_ref().unwrap_err().contains("hour is not recorded"), "{name} gap at {hour}");
            } else {
                assert!(cold.is_ok(), "{name} at {hour}: {cold:?}");
            }
            assert_eq!(packet.changed, seek || previous.as_ref() != Some(&*read.value), "{name} at {hour}");
            let body = match &*read.value { Ok(v) => rmp_serde::to_vec(v).unwrap(), Err(e) => e.as_bytes().to_vec() };
            assert_eq!(delivery.body(), Some(body.as_slice()), "{name} wire record at {hour}");
            assert_eq!(dated::records(&packet.bytes).unwrap()[0].hour, hour);
            previous = Some((*read.value).clone());
        }
    }
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn each_ui_panel_key_has_one_registered_reader() {
    let folder = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let contract = std::fs::read_to_string(folder.join("../../ui/src/generated/dashboard.ts")).unwrap();
    let keys = contract.lines().find_map(|line| line.strip_prefix("export const PANEL_KEYS = ")).unwrap();
    let keys: Vec<String> = serde_json::from_str(keys.strip_suffix(" as const;").unwrap()).unwrap();
    assert_eq!(keys, READERS.keys().copied().collect::<Vec<_>>());
}

#[test]
fn readers_that_can_rebuild_at_each_hour_are_named() {
    let allowed = [
        "autonomy-levels", "casualty-matrix",
        "country-faction-goals", "country-fuel",
        "decryption",
        "faction-details", "factory-parts",
        "occupation-at", "operative-matrix", "operatives-roster",
        "resource-balance",
        "territory-breakdown", "trade-flow",
        "volunteer-flow", "war-contributions",
        "wargoals", "world-tension",
    ];
    let actual: Vec<_> = READERS.iter().filter_map(|(&key, reader)| {
        matches!(reader.kind, PanelKind::WholeFrame | PanelKind::DayThroughHour | PanelKind::ExactHourly).then_some(key)
    }).collect();
    let conditional: Vec<_> = READERS.iter().filter_map(|(&key, reader)| match reader.kind {
        PanelKind::ExactHourlyWhen(conditions) => Some((key, conditions)),
        _ => None,
    }).collect();
    assert_eq!(conditional, [
        ("occupied-states", ["occupation law reads a timed or undeclared world condition"].as_slice()),
    ]);
    assert_eq!(actual, allowed, "Remove a name when its reader no longer needs whole frames or hourly refreshes");
}
