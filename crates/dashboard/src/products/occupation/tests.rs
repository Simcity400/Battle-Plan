use super::law::{GroupLaw, LawInputs, law_for};

fn inputs<'a>(state: Option<&'a [u8]>, group: GroupLaw<'a>, default: Option<(bool, &'a [u8])>) -> LawInputs<'a> {
    LawInputs { state, key_resolved: true, overrides_complete: true, group, default, fallback: Ok("fallback".as_bytes()) }
}

fn never(_: &str) -> Result<bool, String> {
    panic!("only the occupier's default is validated")
}

#[test]
fn a_state_override_wins_then_the_group_law_then_an_admitted_default_else_the_fallback() {
    assert_eq!(law_for(inputs(Some("state".as_bytes()), GroupLaw::Law(Some("group".as_bytes())), Some((true, "default".as_bytes()))), never), Ok("state".into()));
    assert_eq!(law_for(inputs(None, GroupLaw::Law(Some("group".as_bytes())), Some((true, "default".as_bytes()))), never), Ok("group".into()));
    assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |law| Ok(law == "default")), Ok("default".into()));
    assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |_| Ok(false)), Ok("fallback".into()));
    // An unset default validates the fallback itself.
    assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((false, "default".as_bytes()))), |law| Ok(law == "fallback")), Ok("fallback".into()));
    // No group inventory: the database fallback, before any override and unvalidated.
    assert_eq!(law_for(inputs(Some("state".as_bytes()), GroupLaw::Missing, Some((true, "default".as_bytes()))), never), Ok("fallback".into()));
    assert!(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))).reaches_default());
    assert!(!inputs(Some("state".as_bytes()), GroupLaw::Law(None), Some((true, "default".as_bytes()))).reaches_default());
}

#[test]
fn a_law_not_established_says_why_and_is_never_a_guess() {
    let unresolved = LawInputs { key_resolved: false, ..inputs(Some("state".as_bytes()), GroupLaw::Law(None), None) };
    assert!(law_for(unresolved, never).unwrap_err().contains("unresolved"));
    let incomplete = LawInputs { overrides_complete: false, ..inputs(None, GroupLaw::Law(Some("group".as_bytes())), None) };
    assert!(law_for(incomplete, never).unwrap_err().contains("overrides are incomplete"));
    assert_eq!(law_for(inputs(None, GroupLaw::Unknown("not recorded".into()), None), never), Err("not recorded".into()));
    assert!(law_for(inputs(None, GroupLaw::Law(None), None), never).unwrap_err().contains("default law is missing"));
    let no_fallback = LawInputs { fallback: Err("no fallback".into()), ..inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))) };
    assert_eq!(law_for(no_fallback, |_| Ok(false)), Err("no fallback".into()));
    assert_eq!(law_for(inputs(None, GroupLaw::Law(None), Some((true, "default".as_bytes()))), |_| Err("trigger undecided".into())), Err("trigger undecided".into()));
}

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
        Content { catalogue: none(), laws: none(), equipment: none::<std::sync::Arc<installed_content_core::equipment::Database>>().into(), red: None, warnings: Vec::new(), names: Mutex::default() }
    }

    fn resistance(state: u32, country: u32, active: u8, level: i64) -> StateResistanceRow {
        StateResistanceRow { state, country, active, level, compliance: level / 10, speed: -7, compliance_speed: 3, ..Default::default() }
    }

    /// GER (raw 1) occupies POL's states 10 and 11; raw 9 is an alias of POL. State 12 is not occupied, and state 99's
    /// resistance names no States row. State 10's garrison, read with no equipment operand rows, is derived at 62,500.
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

    /// A strength the recorder could not derive says why by its row's reason, and one whose table is not read says the
    /// table's reason; neither names a cell, so nothing paints a zero.
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
    fn occupied_states_not_recorded_at_the_hour_say_why_and_show_no_state() {
        let mut frame = frame();
        *frame.tables[TableId::StateResistance.index()] = TableView::Unavailable { reason: "the read failed".into() };
        let cells = Cells::default();
        let reply = states::reply(&frame, &content(), &cells).unwrap();
        assert!(reply.states.is_empty());
        assert!(reply.missing.unwrap().contains("the read failed"));
        assert!(cells.finish().is_empty());
    }
}
