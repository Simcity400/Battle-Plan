mod reply {
    use super::super::{Content, states};
    use dashboard_follow::live::{Cells, GroupState};
    use replay_data::rows::{
        CountryLinkRow, CountryRow, GarrisonEquipmentInputs, GarrisonStrengthRow, LexicalTokenRow, OccupationGroupLaw, OccupationSettingsRow, Recorded, StateAssignedManpowerRow, StateGarrisonRow,
        StateOccupationLawRow, StateResistanceRow, StateRow, TableId, TableRows,
    };
    use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};
    use std::sync::Mutex;

    fn content() -> Content {
        fn none<T>() -> Result<T, String> {
            Err("no content in this test".to_owned())
        }
        Content { catalogue: none(), laws: none(), equipment: none::<std::sync::Arc<content_reader_key_names::equipment::Database>>().into(), red: None, warnings: Vec::new(), names: Mutex::default(), world: Mutex::default(), judged: Mutex::default() }
    }

    fn resistance(state: u32, country: u32, active: u8, level: i64) -> StateResistanceRow {
        StateResistanceRow { state, country, active, level, compliance: level / 10, speed: -7, compliance_speed: 3, ..Default::default() }
    }

    fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        let mut set = |table: TableId, rows: TableRows| *frame.tables[table.index()] = TableView::Complete(rows);
        set(TableId::Countries, TableRows::Countries([(1, b"GER"), (2, b"POL"), (3, b"ITA")].map(|(id, tag)| CountryRow { id, canonical_slot: id, tag: tag.to_vec(), ..Default::default() }).to_vec()));
        set(TableId::CountryLinks, TableRows::CountryLinks([(1, 1), (2, 2), (3, 3), (9, 2)].map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).to_vec()));
        set(TableId::States, TableRows::States(vec![
            StateRow { id: 10, owner: 2, controller: 1, ..Default::default() },
            StateRow { id: 11, owner: 2, controller: 1, ..Default::default() },
            StateRow { id: 12, owner: 3, controller: 3, ..Default::default() },
        ]));
        set(TableId::StateResistance, TableRows::StateResistance(vec![resistance(10, 9, 1, 2_500_000), resistance(11, 2, 1, 4_000_000), resistance(12, 3, 0, 0), resistance(99, 2, 1, 1)]));
        set(TableId::StateGarrisons, TableRows::StateGarrisons(vec![StateGarrisonRow { controller: 1, state: 10, occupied: 2, required_manpower: 100, garrison_required: 250_000, equipment: Recorded::Value(GarrisonEquipmentInputs::default()), ..Default::default() }]));
        set(TableId::GarrisonStrengths, TableRows::GarrisonStrengths(vec![GarrisonStrengthRow { controller: 1, state: 10, strength: Recorded::Value(62_500), reason: GarrisonStrengthRow::KNOWN }]));
        set(TableId::StateAssignedManpower, TableRows::StateAssignedManpower(vec![
            StateAssignedManpowerRow { controller: 1, state: 10, ordinal: 0, occupied: 9, source_country: 1, amount: 30 },
            StateAssignedManpowerRow { controller: 1, state: 10, ordinal: 1, occupied: 2, source_country: 1, amount: 20 },
        ]));
        set(TableId::StateOccupationLaws, TableRows::StateOccupationLaws(vec![StateOccupationLawRow { controller: 1, occupied: 2, state: 11, name: b"harsh".to_vec() }]));
        set(TableId::OccupationSettings, TableRows::OccupationSettings(vec![OccupationSettingsRow {
            country: 1,
            default_law_present: 1,
            law: b"default".to_vec(),
            group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 9, scope_country: Some(2), law: Some(b"group_law".to_vec()) }])),
            fallback_law: Recorded::Value(Some(b"fallback".to_vec())),
            ..Default::default()
        }]));
        set(TableId::LexicalTokens, TableRows::LexicalTokens(Vec::<LexicalTokenRow>::new()));
        frame
    }

    #[test]
    fn shared_defines_give_the_garrison_its_mod_threshold_or_name_the_missing_value() {
        let read = |value| {
            let (_machine, source, load) = dashboard_base::test_fixtures::recorded_mod_defines(
                "NDefines={NInterface={GARRISON_STRENGTH_TO_SHOW_RED=0.4375}}",
                &format!("NDefines.NInterface.GARRISON_STRENGTH_TO_SHOW_RED={value}"),
                &[],
            );
            states::reply(&frame(), &Content::load(&source, &load), &Cells::default()).unwrap()
        };
        assert_eq!(read("0.8125").garrison_red_raw.as_deref(), Some("81250"));
        let refused = read("nil");
        assert!(refused.garrison_red_raw.is_none());
        assert!(refused.warnings.iter().any(|why| why == "Garrison red threshold unavailable: NDefines.NInterface.GARRISON_STRENGTH_TO_SHOW_RED is not a number"), "{:?}", refused.warnings);
    }

    #[test]
    fn occupied_states_join_by_resolved_countries_and_name_their_moving_numbers_as_live_cells() {
        let frame = frame();
        let cells = Cells::default();
        let reply = states::reply(&frame, &content(), &cells).unwrap();
        assert_eq!(reply.states.iter().map(|state| state.state).collect::<Vec<_>>(), [10, 11], "only active resistance against a country; state 99 has no States row");
        assert!(!reply.complete, "a resistance row without its state leaves the inventory incomplete");
        assert!(reply.missing.is_none());
        let (ten, eleven) = (&reply.states[0], &reply.states[1]);
        assert_eq!((ten.owner.as_deref(), ten.occupier.as_deref(), ten.occupied.as_deref()), (Some("POL"), Some("GER"), Some("POL")), "the alias names POL");
        assert_eq!(ten.law.as_deref(), Some("group_law"), "the law the occupier set for POL, joined through the alias");
        assert_eq!(eleven.law.as_deref(), Some("harsh"), "a state's own override comes first");
        assert_eq!((ten.garrison_required_sign, eleven.garrison_required_sign, ten.assigned_known, eleven.assigned_known), (Some(1), None, true, false), "the garrison's numbers are cells; only their sign and whether they are known are structure");
        assert_eq!(ten.garrison_missing, None, "the derived strength is known and its equipment needs no content");
        assert_eq!(reply.garrison_red_raw, None, "no content, no red threshold");
        assert!(eleven.garrison_missing.as_deref().unwrap().contains("No state garrisons row"));
        assert_eq!(ten.name, None);
        assert!(ten.name_missing.as_deref().unwrap().contains("recorded content"));
        assert!(reply.warnings.iter().any(|warning| warning.contains("absent from States")));
        let live = cells.finish();
        let json = live.json(&live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
        assert_eq!(json["rows"], serde_json::json!(["10", "11"]));
        let group = |table: TableId| json["groups"].as_array().unwrap().iter().find(|group| group["table"] == table.index()).unwrap().clone();
        let numbers = |group: &serde_json::Value| -> Vec<i64> {
            let mut values: Vec<i64> = group["values"].as_array().unwrap().iter().map(|value| value.as_i64().unwrap()).collect();
            values.sort_unstable();
            values
        };
        let resistance = group(TableId::StateResistance);
        assert_eq!(resistance["keys"], serde_json::json!(["10", "11"]));
        assert_eq!(numbers(&resistance), [-7, -7, 3, 3, 250_000, 400_000, 2_500_000, 4_000_000], "level, compliance and the two speeds of both states, exact");
        let garrisons = group(TableId::StateGarrisons);
        assert_eq!(garrisons["keys"], serde_json::json!([(1u128 << 32 | 10).to_string()]), "state 10's garrison row by its own key; state 11 has none");
        assert_eq!(numbers(&garrisons), [100, 250_000], "required manpower and amount, exact");
        let assigned = group(TableId::StateAssignedManpower);
        assert_eq!(assigned["keys"], serde_json::json!([(1u128 << 64 | 10 << 32).to_string(), (1u128 << 64 | 10 << 32 | 1).to_string()]), "both rows of POL, through the alias too");
        assert_eq!(numbers(&assigned), [20, 30], "summed where painted");
        let strength = group(TableId::GarrisonStrengths);
        assert_eq!(strength["keys"], serde_json::json!([(1u128 << 32 | 10).to_string()]), "the garrison's derived strength by its own key");
        assert_eq!(numbers(&strength), [62_500], "the strength a live cell, no longer the structure's");
        let kinds: Vec<&str> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
        assert_eq!(kinds.len(), 8, "{kinds:?}");
        assert!(kinds.contains(&"garrisonStrength"));
        assert!(json["kinds"].as_array().unwrap().iter().any(|kind| kind["id"] == "assignedManpower" && kind["op"] == "sum"));
    }

    #[test]
    fn a_garrison_strength_not_derived_says_why_and_names_no_cell() {
        let ten = |frame: &DecodedFrame| {
            let cells = Cells::default();
            let reply = states::reply(frame, &content(), &cells).unwrap();
            let live = cells.finish();
            let json = live.json(&live.values(frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
            let named = json["groups"].as_array().unwrap().iter().any(|group| group["table"] == TableId::GarrisonStrengths.index());
            (reply.states[0].garrison_missing.clone(), named)
        };
        let mut frame = frame();
        *frame.tables[TableId::StateGarrisons.index()] =
            TableView::Complete(TableRows::StateGarrisons(vec![StateGarrisonRow { controller: 1, state: 10, occupied: 2, required_manpower: 100, garrison_required: 250_000, equipment: Recorded::NotRecorded, ..Default::default() }]));
        *frame.tables[TableId::GarrisonStrengths.index()] =
            TableView::Complete(TableRows::GarrisonStrengths(vec![GarrisonStrengthRow { controller: 1, state: 10, strength: Recorded::NotRecorded, reason: GarrisonStrengthRow::OPERANDS }]));
        assert_eq!(ten(&frame), (Some("This recording lacks the garrison equipment strength operands".to_owned()), false));
        *frame.tables[TableId::GarrisonStrengths.index()] = TableView::Unavailable { reason: "define NResistance.GARRISON_STR_POW_MANPOWER is not held".into() };
        let (why, named) = ten(&frame);
        assert!(why.as_deref().unwrap().contains("GARRISON_STR_POW_MANPOWER"), "{why:?}");
        assert!(!named);
    }

    #[test]
    fn the_reply_reads_no_field_its_narrow_projection_leaves_out() {
        use dashboard_base::products::at_hour::{AtHour, Read};
        let params = states::OccupiedStatesParams { branch_id: "1".into() };
        let world = [(vec![Read::all(TableId::States)], i32::MAX), (vec![Read::all(TableId::Provinces)], i32::MAX)];
        let learning = content();
        let projected = |table, plan: &[(Vec<Read>, i32)]| states::OccupiedStates::projection(&params, table, plan, &learning);
        for table in states::TABLES {
            assert_eq!(projected(table, &world), projected(table, &[]), "{table:?}: a world whose triggers read nothing of it");
        }
        assert!(!states::OccupiedStates::clock_dependent(&world, &learning), "occupied states holds to dated boundaries");
        let level = dashboard_declare::field(TableId::StateResistance, "level").unwrap();
        learning.world.lock().unwrap().fields.insert(TableId::StateResistance, 1 << level);
        assert!(projected(TableId::StateResistance, &world).unwrap().contains(&level), "a field the triggers read");
        assert!(!projected(TableId::StateResistance, &[]).unwrap().contains(&level), "and only while they are planned");
        learning.world.lock().unwrap().ends.insert(TableId::WarRelations);
        let mut timed = frame();
        *timed.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(vec![
            replay_data::rows::WarRelationRow { first_country: 1, second_country: 2, start_hour: 400, end_hour: 512, ..Default::default() },
        ]));
        let holders = dashboard_query::identity::Holders::default();
        let live = Cells::default();
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let at = dashboard_base::products::at_hour::At { owner: None, holders: &holders, hour: 500, context: &learning, cancelled: &cancelled, live: &live };
        assert_eq!(states::world_valid_through(&timed, &learning.world.lock().unwrap()), 511, "a timed war relation holds until the hour before it ends");
        assert_eq!(states::OccupiedStates::valid_through(&timed, &params, &at), 500, "an unsettled judgement expires after the hour");
        learning.world.lock().unwrap().ends.clear();
        learning.world.lock().unwrap().undeclared = true;
        assert_eq!(projected(TableId::States, &world), None, "a read declaring nothing reads every field");
        assert_eq!(states::world_valid_through(&timed, &learning.world.lock().unwrap()), 500, "an undeclared read expires after the hour");
        let mut moved = frame();
        fn held(frame: &mut DecodedFrame, table: TableId) -> &mut TableRows {
            match &mut *frame.tables[table.index()] { TableView::Complete(rows) => rows, _ => unreachable!() }
        }
        let TableRows::StateResistance(rows) = held(&mut moved, TableId::StateResistance) else { unreachable!() };
        for row in rows { (row.level, row.speed, row.compliance, row.compliance_speed) = (row.level + 1, 1, 2, 3); }
        let TableRows::StateGarrisons(rows) = held(&mut moved, TableId::StateGarrisons) else { unreachable!() };
        for row in rows { (row.required_manpower, row.strength_ratio) = (row.required_manpower + 1, 7); }
        let TableRows::StateAssignedManpower(rows) = held(&mut moved, TableId::StateAssignedManpower) else { unreachable!() };
        for row in rows { row.source_country = 3; }
        let TableRows::States(rows) = held(&mut moved, TableId::States) else { unreachable!() };
        for row in rows { (row.manpower_available, row.population) = (5, Recorded::Value(6)); }
        let TableRows::Countries(rows) = held(&mut moved, TableId::Countries) else { unreachable!() };
        for row in rows { (row.political_power_raw, row.stability_raw) = (Some(9), 10); }
        let reply = |frame: &DecodedFrame| serde_json::to_value(states::reply(frame, &content(), &Cells::default()).unwrap()).unwrap();
        assert_eq!(reply(&moved), reply(&frame()));
    }

    #[test]
    fn moving_garrison_numbers_keep_the_structure_only_while_what_it_shows_of_them_holds() {
        use dashboard_base::products::at_hour::Edits;
        use replay_data::rows::GarrisonEquipment;
        fn held(frame: &mut DecodedFrame, table: TableId) -> &mut TableRows {
            match &mut *frame.tables[table.index()] { TableView::Complete(rows) => rows, _ => unreachable!() }
        }
        let garrison = 1u128 << 32 | 10;
        let assigned = [1u128 << 64 | 10 << 32, 1u128 << 64 | 10 << 32 | 1];
        let edits = |before: &DecodedFrame, tables: &[(TableId, &[u128], &[&str])]| {
            let mut edits = Edits::default();
            for &(table, keys, fields) in tables {
                edits.previous.insert(table, before.tables[table.index()].clone());
                let mask = dashboard_base::products::at_hour::field_mask(table, fields);
                edits.tables.insert(table, keys.iter().map(|&key| (key, mask)).collect());
            }
            edits
        };
        let moved = |change: &dyn Fn(&mut DecodedFrame)| {
            let mut frame = frame();
            change(&mut frame);
            frame
        };
        let all: [(TableId, &[u128], &[&str]); 3] = [
            (TableId::StateGarrisons, &[garrison], &["garrison_required", "equipment"]),
            (TableId::StateAssignedManpower, &assigned, &["amount"]),
            (TableId::GarrisonStrengths, &[garrison], &["strength"]),
        ];
        let reply = |frame: &DecodedFrame| serde_json::to_value(states::reply(frame, &content(), &Cells::default()).unwrap()).unwrap();
        let token = |equipment_token| GarrisonEquipment { equipment_token, archetype_token: 2, allocated_raw: 5, ..Default::default() };

        let numbers = moved(&|frame| {
            let TableRows::StateGarrisons(rows) = held(frame, TableId::StateGarrisons) else { unreachable!() };
            rows[0].garrison_required = 1;
            let TableRows::StateAssignedManpower(rows) = held(frame, TableId::StateAssignedManpower) else { unreachable!() };
            (rows[0].amount, rows[1].amount) = (7, -5);
            let TableRows::GarrisonStrengths(rows) = held(frame, TableId::GarrisonStrengths) else { unreachable!() };
            rows[0].strength = Recorded::Value(1);
        });
        assert!(states::structure_holds(&numbers, &edits(&frame(), &all)), "every number moved, none crossed what the row shows");
        assert_eq!(reply(&numbers), reply(&frame()), "and a full read agrees");
        assert!(!states::structure_holds(&numbers, &edits(&frame(), &[(TableId::StateGarrisons, &[garrison], &["garrison_required", "serialized_flag"])])), "a column the structure reads");

        let with_equipment = |tokens: &[u32]| moved(&|frame| {
            let TableRows::StateGarrisons(rows) = held(frame, TableId::StateGarrisons) else { unreachable!() };
            rows[0].equipment = Recorded::Value(GarrisonEquipmentInputs { rows: tokens.iter().map(|&t| token(t)).collect(), failure: None });
        });
        let equipment = &all[..1];
        let mut counted = with_equipment(&[3, 4]);
        let TableRows::StateGarrisons(rows) = held(&mut counted, TableId::StateGarrisons) else { unreachable!() };
        rows[0].equipment = Recorded::Value(GarrisonEquipmentInputs { rows: vec![GarrisonEquipment { allocated_raw: 9, ..token(3) }, token(4)], failure: None });
        assert!(states::structure_holds(&counted, &edits(&with_equipment(&[3, 4]), equipment)), "equipment counts moved, its tokens held");
        assert!(!states::structure_holds(&with_equipment(&[3, 5]), &edits(&with_equipment(&[3, 4]), equipment)), "a new equipment token needs its definition");

        for (why, after, tables) in [
            ("the garrison requirement crossed zero", moved(&|frame| {
                let TableRows::StateGarrisons(rows) = held(frame, TableId::StateGarrisons) else { unreachable!() };
                rows[0].garrison_required = -1;
            }), &all[..1]),
            ("the assigned total reached zero", moved(&|frame| {
                let TableRows::StateAssignedManpower(rows) = held(frame, TableId::StateAssignedManpower) else { unreachable!() };
                (rows[0].amount, rows[1].amount) = (5, -5);
            }), &all[1..2]),
            ("the strength stopped being recorded", moved(&|frame| {
                let TableRows::GarrisonStrengths(rows) = held(frame, TableId::GarrisonStrengths) else { unreachable!() };
                rows[0].strength = Recorded::NotRecorded;
            }), &all[2..]),
        ] {
            assert!(!states::structure_holds(&after, &edits(&frame(), tables)), "{why}");
        }
        let mut inserted = edits(&frame(), &all[..1]);
        inserted.tables.get_mut(&TableId::StateGarrisons).unwrap().insert(garrison, u64::MAX);
        assert!(!states::structure_holds(&frame(), &inserted), "a row came or went");
        let mut replaced = edits(&frame(), &all[..1]);
        replaced.replaced.insert(TableId::StateGarrisons);
        assert!(!states::structure_holds(&frame(), &replaced), "a table read again whole");
    }

    #[test]
    fn judged_laws_hold_through_world_edits_only_where_their_triggers_read_nothing_that_moved() {
        use dashboard_base::products::at_hour::{Edits, field_mask};
        use dashboard_text::triggers::inputs;
        let at = |hour| { let mut frame = frame(); frame.hour = hour; frame };
        let start = at(500);
        let (_, read) = inputs::capture(&start, || inputs::fields(&start, TableId::ScriptFlags, &["value"]));
        let mut world_read = super::super::WorldReads::default();
        world_read.note(&read);
        let content = content();
        *content.judged.lock().unwrap() = super::super::Judged { at: Some((start.branch, 500)), inputs_at: 500, stage: 2, through: i32::MAX, read: world_read, decisions: [(("law".to_owned(), 1, 2), (Ok(true), read))].into(), carried: false };
        let edit = |table, fields: &[&str]| {
            let mut edits = Edits::default();
            edits.tables.insert(table, [(7u128, if fields.is_empty() { u64::MAX } else { field_mask(table, fields) })].into());
            edits
        };
        let judged = || content.judged.lock().unwrap().at.map(|(_, hour)| hour);

        assert!(!states::law_stands(&content, &at(500), &edit(TableId::ScriptVariables, &["seed"])), "laws judged afresh at the hour may differ from those the reply shows");
        assert!(states::law_stands(&content, &at(500), &edit(TableId::StateGarrisons, &["garrison_required"])), "an edit to the states alone leaves the laws as they were");
        assert!(states::judged_through(&content, &at(524), Some(500), &edit(TableId::ScriptVariables, &["seed"])), "a table no trigger read");
        assert_eq!(judged(), Some(524), "and the laws now stand at the new hour");
        assert!(states::law_stands(&content, &at(524), &edit(TableId::ScriptVariables, &["seed"])), "carried through the edit, the shown laws stand");
        content.judged.lock().unwrap().read.ends.insert(TableId::WarRelations);
        assert!(states::law_stands(&content, &at(524), &Edits::default()), "a relation that ends, the laws carried through the hours");
        assert!(!states::law_stands(&content, &at(548), &Edits::default()), "a relation that ends, the laws not carried to this hour");
        content.judged.lock().unwrap().read.ends.clear();
        assert!(states::judged_through(&content, &at(524), Some(500), &Edits::default()), "asked again within the hour");
        assert!(!states::judged_through(&content, &at(572), Some(548), &Edits::default()), "an hour the laws were not judged through");
        assert_eq!(judged(), Some(524));
        assert!(!states::judged_through(&content, &at(548), Some(524), &edit(TableId::ScriptFlags, &[])), "a flag came or went");
        assert_eq!(judged(), None, "and nothing stands until they are judged again");
        assert!(!states::judged_through(&content, &at(548), None, &Edits::default()), "a read that starts the stream");
    }

    #[test]
    fn occupied_states_not_recorded_at_the_hour_say_why_and_show_no_state() {
        let mut frame = frame();
        *frame.tables[TableId::StateResistance.index()] = TableView::Unavailable { reason: "the read failed".into() };
        let cells = Cells::default();
        let reply = states::reply(&frame, &content(), &cells).unwrap();
        assert!(reply.states.is_empty());
        assert!(reply.missing.unwrap().contains("the read failed"));
        assert!(cells.finish().is_empty());
    }

    #[test]
    fn unavailable_auxiliary_tables_carry_their_reason_into_state_garrison_and_law_missing() {
        let mut no_garrisons = frame();
        *no_garrisons.tables[TableId::StateGarrisons.index()] = TableView::Unavailable { reason: "garrison table damaged".into() };
        let reply = states::reply(&no_garrisons, &content(), &Cells::default()).unwrap();
        assert!(reply.missing.is_none());
        assert!(reply.states[0].garrison_missing.as_deref().is_some_and(|why| why.contains("garrison table damaged")), "{:?}", reply.states[0].garrison_missing);

        let mut no_assigned = frame();
        *no_assigned.tables[TableId::StateAssignedManpower.index()] = TableView::Unavailable { reason: "manpower table damaged".into() };
        *no_assigned.tables[TableId::GarrisonStrengths.index()] = TableView::Complete(TableRows::GarrisonStrengths(vec![
            GarrisonStrengthRow { controller: 1, state: 10, strength: Recorded::NotRecorded, reason: GarrisonStrengthRow::ASSIGNED },
        ]));
        let reply = states::reply(&no_assigned, &content(), &Cells::default()).unwrap();
        assert!(!reply.states[0].assigned_known);
        assert!(reply.states[0].garrison_missing.as_deref().is_some_and(|why| why.contains("manpower table damaged")), "{:?}", reply.states[0].garrison_missing);

        let mut truncated_strengths = frame();
        *truncated_strengths.tables[TableId::StateGarrisons.index()] = TableView::Complete(TableRows::StateGarrisons(vec![
            StateGarrisonRow { controller: 1, state: 10, occupied: 2, required_manpower: 100, garrison_required: 250_000, equipment: Recorded::Value(GarrisonEquipmentInputs::default()), ..Default::default() },
            StateGarrisonRow { controller: 1, state: 11, occupied: 2, required_manpower: 100, garrison_required: 250_000, equipment: Recorded::Value(GarrisonEquipmentInputs::default()), ..Default::default() },
        ]));
        *truncated_strengths.tables[TableId::GarrisonStrengths.index()] = TableView::Truncated {
            rows: TableRows::GarrisonStrengths(vec![GarrisonStrengthRow { controller: 1, state: 10, strength: Recorded::Value(62_500), reason: GarrisonStrengthRow::KNOWN }]),
            missing: 2,
        };
        let cells = Cells::default();
        let reply = states::reply(&truncated_strengths, &content(), &cells).unwrap();
        assert_eq!(reply.states[0].garrison_missing, None, "a witnessed garrison strength keeps its live cell when another row was truncated");
        assert!(reply.states[1].garrison_missing.as_deref().is_some_and(|why| why.contains("2 rows not read")), "{:?}", reply.states[1].garrison_missing);
        let live = cells.finish();
        let json = live.json(&live.values(&truncated_strengths).into_iter().map(GroupState::Values).collect::<Vec<_>>());
        let strength = json["groups"].as_array().unwrap().iter().find(|group| group["table"] == TableId::GarrisonStrengths.index()).unwrap();
        assert_eq!(strength["values"], serde_json::json!([62_500]));

        let mut truncated_resistance = frame();
        *truncated_resistance.tables[TableId::StateResistance.index()] = TableView::Truncated {
            rows: TableRows::StateResistance(vec![resistance(10, 9, 1, 2_500_000)]),
            missing: 3,
        };
        let reply = states::reply(&truncated_resistance, &content(), &Cells::default()).unwrap();
        assert!(!reply.complete);
        assert_eq!(reply.states.iter().map(|state| state.state).collect::<Vec<_>>(), [10]);
        assert!(reply.warnings.iter().any(|why| why.contains("3 rows not read")), "{:?}", reply.warnings);

        let mut no_overrides = frame();
        *no_overrides.tables[TableId::StateOccupationLaws.index()] = TableView::Unavailable { reason: "law overrides damaged".into() };
        let reply = states::reply(&no_overrides, &content(), &Cells::default()).unwrap();
        assert_eq!(reply.states[1].law, None);
        assert!(reply.states[1].law_missing.as_deref().is_some_and(|why| why.contains("law overrides damaged")), "{:?}", reply.states[1].law_missing);

        let mut no_settings = frame();
        *no_settings.tables[TableId::OccupationSettings.index()] = TableView::Unavailable { reason: "settings damaged".into() };
        let reply = states::reply(&no_settings, &content(), &Cells::default()).unwrap();
        assert_eq!(reply.states[0].law, None);
        assert!(reply.states[0].law_missing.as_deref().is_some_and(|why| why.contains("settings damaged")), "{:?}", reply.states[0].law_missing);

        let mut old_recording = frame();
        *old_recording.tables[TableId::StateGarrisons.index()] = TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full };
        let reply = states::reply(&old_recording, &content(), &Cells::default()).unwrap();
        assert!(reply.missing.is_none());
        assert!(reply.states[0].garrison_missing.as_deref().is_some_and(|why| why.contains("not recorded")), "{:?}", reply.states[0].garrison_missing);
    }

    #[test]
    fn occupied_states_and_occupation_detail_hold_through_moving_numbers_and_refresh_on_structural_changes() {
        use super::super::detail::OccupationDetail;
        use dashboard_base::{
            Source, SourceKey,
            products::{LoadContext, at_hour::{exact_panel, frame_panel}},
            test_fixtures::{CAMPAIGN, materialized_frame, played, records, source, streamed},
        };
        use replay::campaign::Replay;
        use replay_data::rows::CountryCollaborationRow;
        use serde_json::json;
        use std::sync::{Arc, atomic::AtomicBool};

        let make = |branch: u64, hour: i32, damaged: bool| {
            let mut f = source(branch, hour, Some(100));
            let pol_tag = if hour >= 109 { b"PLC".to_vec() } else { b"POL".to_vec() };
            fn set(frame: &mut DecodedFrame, table: TableId, rows: TableRows) {
                *frame.tables[table.index()] = TableView::Complete(rows);
            }
            set(&mut f, TableId::Countries, TableRows::Countries(vec![
                CountryRow { id: 1, canonical_slot: 1, tag: b"GER".to_vec(), political_power_raw: Some(i64::from(hour) * 10), ..Default::default() },
                CountryRow { id: 2, canonical_slot: 2, tag: pol_tag, political_power_raw: Some(i64::from(hour)), ..Default::default() },
                CountryRow { id: 3, canonical_slot: 3, tag: b"ITA".to_vec(), ..Default::default() },
            ]));
            set(&mut f, TableId::CountryLinks, TableRows::CountryLinks([1, 2, 3].map(|id| CountryLinkRow { id, canonical_slot: id }).to_vec()));
            set(&mut f, TableId::States, TableRows::States(vec![
                StateRow { id: 10, owner: 2, controller: 1, manpower_available: hour * 5, ..Default::default() },
                StateRow { id: 11, owner: 2, controller: if hour >= 105 { 3 } else { 1 }, ..Default::default() },
            ]));
            if damaged {
                *f.tables[TableId::StateResistance.index()] = TableView::Unavailable { reason: "resistance read failed".into() };
            } else {
                set(&mut f, TableId::StateResistance, TableRows::StateResistance(vec![
                    resistance(10, 2, 1, 1_000_000 + i64::from(hour) * 1000),
                    resistance(11, 2, 1, 2_000_000 + i64::from(hour) * 1000),
                ]));
            }
            let required = if hour == 104 { -10 } else { 250_000 + i64::from(hour) };
            set(&mut f, TableId::StateGarrisons, TableRows::StateGarrisons(vec![
                StateGarrisonRow { controller: 1, state: 10, occupied: 2, required_manpower: 100 + hour, garrison_required: required, equipment: Recorded::Value(GarrisonEquipmentInputs::default()), ..Default::default() },
            ]));
            set(&mut f, TableId::GarrisonStrengths, TableRows::GarrisonStrengths(vec![
                GarrisonStrengthRow { controller: 1, state: 10, strength: Recorded::Value(50_000 + i64::from(hour)), reason: GarrisonStrengthRow::KNOWN },
            ]));
            set(&mut f, TableId::StateAssignedManpower, TableRows::StateAssignedManpower(vec![
                StateAssignedManpowerRow { controller: 1, state: 10, ordinal: 0, occupied: 2, source_country: 1, amount: 30 + (hour % 5) },
            ]));
            set(&mut f, TableId::StateOccupationLaws, TableRows::StateOccupationLaws(vec![
                StateOccupationLawRow { controller: 1, occupied: 2, state: 10, name: if hour >= 106 { b"brutal".to_vec() } else { b"harsh".to_vec() } },
            ]));
            set(&mut f, TableId::OccupationSettings, TableRows::OccupationSettings(vec![
                OccupationSettingsRow { country: 1, default_law_present: 1, law: b"default".to_vec(), group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: Some(b"group_law".to_vec()) }])), fallback_law: Recorded::Value(Some(b"fallback".to_vec())), ..Default::default() },
                OccupationSettingsRow { country: 3, default_law_present: 1, law: b"default".to_vec(), group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: Some(b"group_law".to_vec()) }])), fallback_law: Recorded::Value(Some(b"fallback".to_vec())), ..Default::default() },
            ]));
            set(&mut f, TableId::LexicalTokens, TableRows::LexicalTokens(Vec::<LexicalTokenRow>::new()));
            set(&mut f, TableId::Provinces, TableRows::Provinces(Vec::new()));
            if damaged {
                *f.tables[TableId::CountryCollaboration.index()] = TableView::Unavailable { reason: "collab read failed".into() };
            } else {
                set(&mut f, TableId::CountryCollaboration, TableRows::CountryCollaboration(vec![
                    CountryCollaborationRow { source: 2, target: 1, progress: if hour >= 108 { 40_000 } else { 10_000 } },
                ]));
            }
            f
        };

        let make_writers = || {
            let current: Vec<usize> = replay_recording::schema::schema().unwrap().iter().map(Vec::len).collect();
            let mut kept = current.clone();
            kept[TableId::GarrisonStrengths.index()] = usize::MAX;
            recorder_derive::writers_within(&kept, &current)
        };
        let main_frames: Vec<_> = (100..=114).map(|hour| make(1, hour, (111..=112).contains(&hour))).collect();
        let replay = Replay::from_records(make_writers(), CAMPAIGN.into(), &[records(1, main_frames.clone())]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Source::from_replay(SourceKey { source_id: "occ-played".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap();
        let load = LoadContext { content: None };

        let occ_params = json!({"branchId": "1"});
        let occ_panel = exact_panel::<states::OccupiedStates>(&source, &load, &occ_params).unwrap();
        let occ_exact = occ_panel.exact().unwrap();
        let mut occ_whole = frame_panel::<states::OccupiedStates>(&source, &load, &occ_params).unwrap();
        let steps = played(source.replay_ref(), occ_exact, 100, 114);
        for step in &steps {
            let expected = streamed(&materialized_frame(&mut *occ_whole, source.replay_ref(), 1, step.hour), step.fresh);
            assert_eq!(streamed(&step.value, step.fresh), expected, "occupied-states at {}", step.hour);
            if matches!(step.hour, 101 | 102 | 103 | 107 | 108 | 110 | 114) {
                assert_eq!(step.product_calls, 0, "quiet/moving hour {} should not rebuild occupied-states", step.hour);
            }
        }
        let back = (occ_exact.advance)(102, true, &false.into(), None);
        assert_eq!(*back.value, materialized_frame(&mut *occ_whole, source.replay_ref(), 1, 102), "occupied-states backward seek");
        let alt_frames: Vec<_> = (105..=108).map(|hour| make(2, hour, false)).collect();
        let branched = Replay::from_records(make_writers(), CAMPAIGN.into(), &[records(1, main_frames), records(2, alt_frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let branch2 = branched.timelines().unwrap().list.iter().find(|t| t.id != 1).unwrap().id;
        let alt_source = Source::from_replay(SourceKey { source_id: "occ-branch".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(branched), &AtomicBool::new(false)).unwrap();
        let alt_panel = exact_panel::<states::OccupiedStates>(&alt_source, &load, &json!({"branchId": branch2.to_string()})).unwrap();
        let alt_read = (alt_panel.exact().unwrap().advance)(106, true, &false.into(), None);
        let mut alt_whole = frame_panel::<states::OccupiedStates>(&alt_source, &load, &json!({"branchId": branch2.to_string()})).unwrap();
        assert_eq!(*alt_read.value, materialized_frame(&mut *alt_whole, alt_source.replay_ref(), branch2, 106), "occupied-states branch switch");

        for det_params in [
            json!({"branchId": "1", "countries": [], "collaboration": true}),
            json!({"branchId": "1", "countries": ["GER"], "collaboration": false}),
        ] {
            let det_panel = exact_panel::<OccupationDetail>(&source, &load, &det_params).unwrap();
            let det_exact = det_panel.exact().unwrap();
            let mut det_whole = frame_panel::<OccupationDetail>(&source, &load, &det_params).unwrap();
            let steps = played(source.replay_ref(), det_exact, 100, 114);
            for step in &steps {
                let expected = streamed(&materialized_frame(&mut *det_whole, source.replay_ref(), 1, step.hour), step.fresh);
                assert_eq!(streamed(&step.value, step.fresh), expected, "occupation-at {det_params} at {}", step.hour);
                if matches!(step.hour, 101 | 102 | 103 | 104 | 106 | 107 | 110 | 114) {
                    assert_eq!(step.product_calls, 0, "quiet hour {} should not rebuild occupation-at", step.hour);
                }
            }
            let back = (det_exact.advance)(103, true, &false.into(), None);
            assert_eq!(*back.value, materialized_frame(&mut *det_whole, source.replay_ref(), 1, 103), "occupation-at backward seek");
            let cold = (det_exact.cold)(100, &false.into());
            let expected_cold = materialized_frame(&mut *det_whole, source.replay_ref(), 1, 100);
            assert_eq!(cold, expected_cold, "occupation-at {det_params} cold path");
            let totals = cold.as_ref().unwrap()["live"]["groups"]
                .as_array()
                .unwrap()
                .iter()
                .find(|group| group["table"] == TableId::OccupierTotals.index())
                .unwrap()["values"]
                .as_array()
                .unwrap();
            assert!(totals.iter().any(|value| value.as_i64().is_some_and(|n| n > 0)), "cold occupation-at must decode nonzero OccupierTotals live cells: {totals:?}");
        }
    }

    #[test]
    fn occupied_states_reacts_to_newly_discovered_world_edits_and_holds_flag_age_thresholds() {
        use dashboard_base::{
            Source, SourceKey,
            products::at_hour::{exact_panel, frame_panel},
            test_fixtures::{CAMPAIGN, content_identity, game, materialized_frame, played, records, source, source_with_content, streamed, visual_mods::Machine},
        };
        use replay::campaign::Replay;
        use replay_data::rows::{CompletedFocusRow, CountryAppearanceRow, ScriptFlagRow};
        use serde_json::json;
        use std::sync::atomic::AtomicBool;

        game::enter();
        let machine = Machine::new("occ_law_exact");
        machine.write(
            "common/defines/00_defines.lua",
            b"NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=7},NInterface={GARRISON_STRENGTH_TO_SHOW_RED=0.5},NResistance={GARRISON_STR_POW_MANPOWER=1,GARRISON_STR_POW_EQUIPMENT=1}}",
        );
        machine.write(
            "common/occupation_laws/test_laws.txt",
            br#"
cosmetic_law = {
    icon = 1
    visible = { has_cosmetic_tag = test_cosmetic }
}
original_law = {
    icon = 1
    visible = { original_tag = ROOT }
}
focus_law = {
    icon = 1
    visible = { has_completed_focus = test_occ_focus }
}
flag_age_law = {
    icon = 1
    visible = { has_country_flag = { flag = test_occ_flag days > 1 } }
}
dynamic_flag_law = {
    icon = 1
    visible = { has_country_flag = { flag = dyn_occ_flag_@ROOT days > 1 } }
}
plain_law = {
    icon = 1
}
fallback_law = {
    icon = 1
    fallback_law = yes
}
"#,
        );

        let h0 = game_model::calendar::HOUR_EPOCH;
        let frames: Vec<_> = (0..=55)
            .map(|offset| {
                let hour = h0 + offset;
                let mut f = source(1, hour, Some(100));
                let mut set = |table: TableId, rows: TableRows| *f.tables[table.index()] = TableView::Complete(rows);
                set(TableId::Countries, TableRows::Countries(vec![
                    CountryRow { id: 1, canonical_slot: 1, tag: b"GER".to_vec(), ..Default::default() },
                    CountryRow { id: 2, canonical_slot: 2, tag: b"POL".to_vec(), ..Default::default() },
                    CountryRow { id: 3, canonical_slot: 3, tag: b"FRA".to_vec(), ..Default::default() },
                ]));
                set(TableId::CountryLinks, TableRows::CountryLinks([1, 2, 3].map(|id| CountryLinkRow { id, canonical_slot: id }).to_vec()));
                set(TableId::States, TableRows::States(vec![
                    StateRow { id: 10, owner: 2, controller: 1, ..Default::default() },
                    StateRow { id: 11, owner: 2, controller: 3, ..Default::default() },
                ]));
                set(TableId::StateResistance, TableRows::StateResistance(vec![
                    resistance(10, 2, 1, 1_000_000),
                    resistance(11, 2, 3, 1_000_000),
                ]));
                set(TableId::StateGarrisons, TableRows::StateGarrisons(Vec::new()));
                set(TableId::GarrisonStrengths, TableRows::GarrisonStrengths(Vec::new()));
                set(TableId::StateAssignedManpower, TableRows::StateAssignedManpower(Vec::new()));
                set(TableId::StateOccupationLaws, TableRows::StateOccupationLaws(Vec::new()));
                let flag_law: &[u8] = if offset <= 52 { b"flag_age_law" } else { b"plain_law" };
                set(TableId::OccupationSettings, TableRows::OccupationSettings(vec![
                    OccupationSettingsRow {
                        country: 1,
                        default_law_present: 1,
                        law: b"cosmetic_law".to_vec(),
                        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                        fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                        ..Default::default()
                    },
                    OccupationSettingsRow {
                        country: 3,
                        default_law_present: 1,
                        law: flag_law.to_vec(),
                        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                        fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                        ..Default::default()
                    },
                ]));
                set(TableId::CountryAppearance, TableRows::CountryAppearance(vec![
                    CountryAppearanceRow {
                        country: 1,
                        cosmetic_tag: if offset == 0 { b"test_cosmetic".to_vec() } else { Vec::new() },
                        original_tag_reference: Recorded::Value(Some(1)),
                        ..Default::default()
                    },
                    CountryAppearanceRow { country: 2, original_tag_reference: Recorded::Value(Some(2)), ..Default::default() },
                    CountryAppearanceRow { country: 3, original_tag_reference: Recorded::Value(Some(3)), ..Default::default() },
                ]));
                set(TableId::LexicalTokens, TableRows::LexicalTokens(vec![
                    LexicalTokenRow { token: 7, literal: b"test_occ_flag".to_vec() },
                    LexicalTokenRow { token: 8, literal: b"test_occ_focus".to_vec() },
                    LexicalTokenRow { token: 9, literal: b"dyn_occ_flag_FRA".to_vec() },
                ]));
                set(TableId::ScriptFlags, TableRows::ScriptFlags(vec![
                    ScriptFlagRow { scope_kind: 1, scope_id: 2, token: 7, set_hour: h0, value: if offset >= 10 { 2 } else { 1 }, remaining_days: 0 },
                    ScriptFlagRow { scope_kind: 1, scope_id: 3, token: 7, set_hour: h0, value: 1, remaining_days: 0 },
                    ScriptFlagRow { scope_kind: 1, scope_id: 3, token: 9, set_hour: h0, value: 1, remaining_days: 0 },
                ]));
                set(TableId::ScriptVariables, TableRows::ScriptVariables(Vec::new()));
                set(TableId::CompletedFocuses, TableRows::CompletedFocuses(vec![CompletedFocusRow { country: 3, ordinal: 0, token: b"test_occ_focus".to_vec() }]));
                set(TableId::Provinces, TableRows::Provinces(Vec::new()));
                f
            })
            .collect();

        let (seeded, load) = source_with_content(vec![frames[0].clone()], &machine.root);
        let pack = seeded.replay_ref().facts(0).unwrap();
        let (_, identity) = content_identity(&machine.root);
        let build_source = |frames: Vec<DecodedFrame>, name: &str| {
            let mut writers = recorder_derive::writers();
            writers.define_from(&recorder_derive_framework::framework::Facts::held(std::sync::Arc::new(pack.clone())), &identity);
            let mut input = records(1, frames);
            if let replay_capture_spool::spool::Record::SessionStart(session) = &mut input[0] {
                session.identity = identity.clone();
            }
            let replay = Replay::from_records(writers, CAMPAIGN.into(), &[input]).unwrap();
            let path = machine.root.join(format!("{name}.battleplan"));
            std::fs::write(&path, replay.bytes()).unwrap();
            replay::campaign::writer::revise(
                &path,
                replay.recording(),
                &replay::campaign::writer::Addition { facts: Some(&pack), ..Default::default() },
                true,
                3,
            )
            .unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let replay = Replay::open(bytes).unwrap().into_timelines(&Default::default()).unwrap();
            Source::from_replay(
                SourceKey { source_id: name.into(), revision: "1".into() },
                path,
                std::sync::Arc::new(replay),
                &AtomicBool::new(false),
            )
            .unwrap()
        };
        let source = build_source(frames.clone(), "occ-law-exact");
        let params = json!({"branchId": "1"});
        let panel = exact_panel::<states::OccupiedStates>(&source, &load, &params).unwrap();
        let exact = panel.exact().unwrap();
        let mut whole = frame_panel::<states::OccupiedStates>(&source, &load, &params).unwrap();
        let steps = played(source.replay_ref(), exact, h0, h0 + 55);
        let law_at = |offset: i32, index: usize| {
            steps[offset as usize].value.as_ref().as_ref().unwrap()["states"][index]["law"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        for step in &steps {
            let expected = streamed(&materialized_frame(&mut *whole, source.replay_ref(), 1, step.hour), step.fresh);
            assert_eq!(streamed(&step.value, step.fresh), expected, "occupied-states at offset {}", step.hour - h0);
        }
        assert_eq!(law_at(0, 0), "cosmetic_law");
        assert_eq!(law_at(1, 0), "fallback_law", "newly discovered stage-2 table flip must take effect on hour 1");
        assert_eq!(law_at(0, 1), "fallback_law");
        assert_eq!(law_at(47, 1), "fallback_law");
        assert_eq!(law_at(48, 1), "flag_age_law", "flag age > 1 day flips at +48h");
        assert_eq!(law_at(52, 1), "flag_age_law");
        assert_eq!(law_at(53, 1), "plain_law");
        for offset in 2..=47 {
            assert_eq!(steps[offset as usize].product_calls, 0, "quiet/unrelated-flag hour before flag-age threshold (offset {offset}) should hold");
        }
        for offset in 49..=52 {
            assert_eq!(steps[offset as usize].product_calls, 0, "quiet hour after flag-age threshold (offset {offset}) should hold");
        }
        for offset in 54..=55 {
            assert_eq!(steps[offset as usize].product_calls, 0, "quiet hour after switching off clock law (offset {offset}) should hold");
        }

        drop(panel);
        let after_cold = exact_panel::<states::OccupiedStates>(&source, &load, &params).unwrap();
        let after_cold_exact = after_cold.exact().unwrap();
        assert_eq!(
            (after_cold_exact.cold)(h0 + 1, &false.into()),
            materialized_frame(&mut *whole, source.replay_ref(), 1, h0 + 1),
            "cold read at offset 1"
        );
        let after_cold_steps = played(source.replay_ref(), after_cold_exact, h0 + 1, h0 + 49);
        for step in &after_cold_steps {
            let expected = streamed(&materialized_frame(&mut *whole, source.replay_ref(), 1, step.hour), step.fresh);
            assert_eq!(streamed(&step.value, step.fresh), expected, "occupied-states after cold read at offset {}", step.hour - h0);
        }
        assert_eq!(after_cold_steps[46].value.as_ref().as_ref().unwrap()["states"][1]["law"], "fallback_law");
        assert_eq!(after_cold_steps[47].value.as_ref().as_ref().unwrap()["states"][1]["law"], "flag_age_law", "cold read before seed must preserve +48h flag-age deadline");
        drop(after_cold);

        let mut dynamic_frames = frames[..=49].to_vec();
        for (offset, frame) in dynamic_frames.iter_mut().enumerate() {
            *frame.tables[TableId::ScriptFlags.index()] = TableView::Complete(TableRows::ScriptFlags(vec![
                ScriptFlagRow { scope_kind: 1, scope_id: 3, token: 7, set_hour: h0, value: 1, remaining_days: 0 },
                ScriptFlagRow { scope_kind: 1, scope_id: 3, token: 9, set_hour: h0, value: 1, remaining_days: 0 },
            ]));
            if offset >= 1 {
                *frame.tables[TableId::OccupationSettings.index()] = TableView::Complete(TableRows::OccupationSettings(vec![
                    OccupationSettingsRow {
                        country: 1,
                        default_law_present: 1,
                        law: b"cosmetic_law".to_vec(),
                        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                        fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                        ..Default::default()
                    },
                    OccupationSettingsRow {
                        country: 3,
                        default_law_present: 1,
                        law: b"dynamic_flag_law".to_vec(),
                        group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                        fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                        ..Default::default()
                    },
                ]));
            }
        }
        let dynamic_source = build_source(dynamic_frames, "occ-law-dynamic-flag");
        let dynamic_panel = exact_panel::<states::OccupiedStates>(&dynamic_source, &load, &params).unwrap();
        let dynamic_exact = dynamic_panel.exact().unwrap();
        let mut dynamic_whole = frame_panel::<states::OccupiedStates>(&dynamic_source, &load, &params).unwrap();
        let dynamic_steps = played(dynamic_source.replay_ref(), dynamic_exact, h0, h0 + 49);
        for step in &dynamic_steps {
            let expected = streamed(&materialized_frame(&mut *dynamic_whole, dynamic_source.replay_ref(), 1, step.hour), step.fresh);
            assert_eq!(streamed(&step.value, step.fresh), expected, "occupied-states dynamic flag at offset {}", step.hour - h0);
        }
        assert_eq!(dynamic_steps[47].value.as_ref().as_ref().unwrap()["states"][1]["law"], "fallback_law");
        assert_eq!(dynamic_steps[48].value.as_ref().as_ref().unwrap()["states"][1]["law"], "dynamic_flag_law", "switching to @ROOT flag law on already-projected Countries.tag must keep +48h deadline");
        for offset in 3..=47 {
            assert_eq!(dynamic_steps[offset].product_calls, 0, "quiet hour after settling dynamic flag law (offset {offset}) should hold");
        }
        drop(dynamic_panel);

        let mut retry_frames = frames[..=1].to_vec();
        *retry_frames[1].tables[TableId::OccupationSettings.index()] = TableView::Complete(TableRows::OccupationSettings(vec![
            OccupationSettingsRow {
                country: 1,
                default_law_present: 1,
                law: b"original_law".to_vec(),
                group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                ..Default::default()
            },
            OccupationSettingsRow {
                country: 3,
                default_law_present: 1,
                law: b"focus_law".to_vec(),
                group_laws: Recorded::Value(Some(vec![OccupationGroupLaw { occupied: 2, scope_country: Some(2), law: None }])),
                fallback_law: Recorded::Value(Some(b"fallback_law".to_vec())),
                ..Default::default()
            },
        ]));
        let retry_source = build_source(retry_frames, "occ-law-retry");
        let retry_panel = exact_panel::<states::OccupiedStates>(&retry_source, &load, &params).unwrap();
        let retry_exact = retry_panel.exact().unwrap();
        let mut retry_whole = frame_panel::<states::OccupiedStates>(&retry_source, &load, &params).unwrap();
        assert_eq!(*(retry_exact.advance)(h0, true, &false.into(), None).value, materialized_frame(&mut *retry_whole, retry_source.replay_ref(), 1, h0));
        let retried = (retry_exact.advance)(h0 + 1, false, &false.into(), None);
        assert_eq!(*retried.value, materialized_frame(&mut *retry_whole, retry_source.replay_ref(), 1, h0 + 1), "projection retry must re-evaluate newly widened columns afresh");
        assert_eq!(retried.value.as_ref().as_ref().unwrap()["states"][0]["law"], "original_law");
        assert_eq!(retried.value.as_ref().as_ref().unwrap()["states"][1]["law"], "focus_law");
    }
}
