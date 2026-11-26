use super::*;

fn key(kind: u8, identity: &[u8]) -> StabilityPartKeyRow {
    StabilityPartKeyRow {
        handle: 7,
        kind,
        identity: identity.to_vec(),
        identity_id: 0,
        secondary: 0,
        script: Recorded::NotRecorded,
        saved: Recorded::NotRecorded,
        template_present: Recorded::NotRecorded,
        template_tag: Recorded::NotRecorded,
        template_file: Recorded::NotRecorded,
        template_line: Recorded::NotRecorded,
        template_generated: Recorded::NotRecorded,
    }
}

const COUNTRY: IdeaCountry<'static> = IdeaCountry { tag: "AAA", slot: 1, look: None };

#[test]
fn a_country_breakdown_subtracts_only_its_own_lines_for_a_shared_modifier() {
    use replay_data::rows::{SliceReadRow, TableRows};
    use replay_frame::frame::TableView;

    let mut frame = dashboard_base::test_fixtures::source(1, 100, None);
    let owners = [(5, 40, 9), (7, 90, 14)];
    let inputs = owners.map(|(country, total, _)| {
        let mut keys = vec![u32::MAX; CountryStabilityInputRow::KEY_NAMES.len()];
        keys[0] = 3;
        CountryStabilityInputRow { country, stability_factor: Some(total), keys, ..Default::default() }
    });
    let lines = owners.map(|(country, _, value)| CountryStabilityLineRow {
        country, key: 3, value, kind: CountryStabilityLineRow::STATIC, ..Default::default()
    });
    let reads = owners.map(|(owner, _, _)| SliceReadRow {
        owner, table: TableId::CountryStabilityInputs.index() as u32, sample_hour: 100,
    });
    for (table, rows) in [
        (TableId::CountryStabilityInputs, TableRows::CountryStabilityInputs(inputs.into())),
        (TableId::CountryStabilityLines, TableRows::CountryStabilityLines(lines.into())),
        (TableId::SliceReads, TableRows::SliceReads(reads.into())),
    ] {
        *frame.tables[table.index()] = TableView::Complete(rows);
    }
    for (id, total, own) in owners {
        let reply = breakdown_at(&frame, Owner { id, slot: id }, "AAA", &None, &mut Vec::new()).unwrap();
        let modifier = &reply.modifiers[0];
        assert_eq!(modifier.lines.iter().map(|line| &line.value_raw).collect::<Vec<_>>(), [&own.to_string()]);
        assert_eq!(modifier.remainder_raw, Some((total - own).to_string()));
    }
}

#[test]
fn a_part_is_asked_by_its_measure_and_handle_and_nothing_past_war_support() {
    assert_eq!(part_of("7"), Some((StabilityPartRow::STABILITY, 7)));
    assert_eq!(part_of(&((1u64 << 32) | 9).to_string()), Some((StabilityPartRow::WAR_SUPPORT, 9)));
    assert_eq!(part_of(&(2u64 << 32).to_string()), None);
    assert_eq!(part_of("-1"), None);
    assert_eq!(part_of("x"), None);
}

#[test]
fn stability_panels_hold_quiet_hours_and_seed_the_requested_country_at_each_seek() {
    use dashboard_base::{SourceKey, products::at_hour::frame_panel, test_fixtures::{CAMPAIGN, materialized_frame, played, records, source}};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryAppearanceRow, CountryCommandPowerRow, ModifierDefinitionRow, SliceReadRow, TableRows};
    use replay_frame::frame::TableView;
    use serde_json::{Value, json};

    let frames: Vec<_> = [1, 2].map(|branch| {
        let hours = (100..=115).map(|hour| {
            let mut frame = source(branch, hour, Some(i64::from(hour)));
            let mut keys = vec![u32::MAX; 10];
            keys[0] = 3;
            let row = CountryStabilityInputRow {
                country: 9001, stability_raw: 30_000, max_stability: 100_000, max_war_support: 100_000,
                stability_factor: Some(40 + branch as i64), recruitment_ratio: Recorded::Value(Some(i64::from(hour))), keys, ..Default::default()
            };
            let line = |country, value| CountryStabilityLineRow {
                country, key: 3, value, kind: CountryStabilityLineRow::IDEA,
                identity: format!("idea_{branch}_{}", if hour < 107 { 1 } else { 2 }).into_bytes(), ..Default::default()
            };
            let slices = [TableId::CountryStabilityInputs, TableId::CountryStabilityLines].map(|table| SliceReadRow {
                owner: 9001, table: table.index() as u32, sample_hour: if hour >= 107 { 107 } else { 100 },
            });
            for (table, rows) in [
                (TableId::CountryStabilityInputs, TableRows::CountryStabilityInputs(vec![row])),
                (TableId::CountryStabilityLines, TableRows::CountryStabilityLines(vec![line(44, i64::from(hour)), line(9001, 9 + i64::from(hour >= 105))])),
                (TableId::CountryAppearance, TableRows::CountryAppearance(vec![CountryAppearanceRow { country: 9001, ..Default::default() }])),
                (TableId::CountryCommandPower, TableRows::CountryCommandPower(vec![CountryCommandPowerRow { canonical_slot: 7, ..Default::default() }])),
                (TableId::ModifierDefinitions, TableRows::ModifierDefinitions(vec![ModifierDefinitionRow { index: 3, flags: if hour >= 109 { GOOD_FLAG } else { 0 }, ..Default::default() }])),
                (TableId::Characters, TableRows::Characters(Vec::new())),
                (TableId::LexicalTokens, TableRows::LexicalTokens(Vec::new())),
                (TableId::SliceReads, TableRows::SliceReads(slices.into())),
            ] { *frame.tables[table.index()] = TableView::Complete(rows); }
            if (111..114).contains(&hour) {
                *frame.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable { reason: "damaged stability lines".into() };
            }
            frame
        }).collect();
        records(branch, hours)
    }).into();
    let load = LoadContext { content: None };
    for branch in [1, 2] {
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &frames[..branch as usize]).unwrap().into_timelines(&Default::default()).unwrap();
        let source = Arc::new(Source::from_replay(SourceKey { source_id: "stability".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap());
        for parts in [false, true] {
            let mut request = json!({"branchId":branch.to_string(), "country":"GER"});
            let (registration, mut original) = if parts {
                request["parts"] = json!((0..12).map(|key| key.to_string()).collect::<Vec<_>>());
                (crate::registrations::STABILITY_STABILITYPARTNAMES, frame_panel::<StabilityPartNames>(&source, &load, &request).unwrap())
            } else {
                (crate::registrations::STABILITY_COUNTRYSTABILITYBREAKDOWN, frame_panel::<CountryStabilityBreakdown>(&source, &load, &request).unwrap())
            };
            let panel = (registration.panel.unwrap().open)(&source, &load, &request).unwrap();
            let exact = panel.exact().unwrap();
            if branch == 1 {
                let mut previous = None;
                for step in played(source.replay_ref(), exact, 100, 115) {
                    let mut expected = materialized_frame(&mut *original, source.replay_ref(), branch, step.hour).unwrap();
                    let mut materialized = step.value.as_ref().as_ref().unwrap().clone();
                    let date = |reply: &mut Value, hour| {
                        reply["hour"] = Value::from(hour);
                        if let Some(inner) = reply.get_mut("breakdown").filter(|v| v.is_object()) { inner["hour"] = Value::from(hour); }
                    };
                    if !step.fresh { date(&mut materialized, step.hour); }
                    assert_eq!(materialized, expected, "{} at {}", registration.key, step.hour);
                    date(&mut expected, 0);
                    if let Some(prior) = &previous { assert_eq!(step.product_calls, u64::from(prior != &expected), "{} at {}", registration.key, step.hour); }
                    previous = Some(expected);
                }
            }
            for hour in [114, 104, 106, 112] {
                let read = (exact.advance)(hour, true, &false.into(), None);
                assert!(read.fresh);
                assert_eq!(*read.value, materialized_frame(&mut *original, source.replay_ref(), branch, hour), "{} branch {branch} at {hour}", registration.key);
            }
        }
    }
}

#[test]
fn a_dynamic_idea_name_stops_using_the_civil_war_look_when_the_recorded_relation_ends() {
    use replay_data::rows::{CountryAppearanceRow, CountryCivilWarRow, CountryCounterInputRow, CountryLinkRow, CountryRow, TableRows, WarRelationRow};
    use replay_frame::frame::TableView;
    let owner = Owner { slot: 7, id: 9001 };
    let mut frame = dashboard_base::test_fixtures::source(1, 100, None);
    for (table, rows) in [
        (TableId::Countries, TableRows::Countries(vec![
            CountryRow { id: owner.id, canonical_slot: owner.slot, ..Default::default() },
            CountryRow { id: 9002, canonical_slot: 8, ..Default::default() },
        ])),
        (TableId::CountryAppearance, TableRows::CountryAppearance(vec![CountryAppearanceRow { country: owner.id, name_tag: b"D01".to_vec(), ..Default::default() }])),
        (TableId::CountryLinks, TableRows::CountryLinks(vec![CountryLinkRow { id: owner.id, canonical_slot: owner.slot }, CountryLinkRow { id: 9002, canonical_slot: 8 }])),
        (TableId::CountryCivilWars, TableRows::CountryCivilWars(vec![CountryCivilWarRow { country: 9002, target: owner.id, ..Default::default() }])),
        (TableId::CountryCounterInputs, TableRows::CountryCounterInputs(vec![CountryCounterInputRow { country: 9002, owned_state_count: 1, ..Default::default() }])),
        (TableId::CountryWars, TableRows::CountryWars(Vec::new())),
        (TableId::WarRelations, TableRows::WarRelations(vec![WarRelationRow { first_country: owner.id, second_country: 9002, start_hour: 95, end_hour: 105, ..Default::default() }])),
    ] { *frame.tables[table.index()] = TableView::Complete(rows); }
    assert_eq!(look(&frame, owner).unwrap().civil_war, Ok(true));
    assert_eq!(naming_through(&frame, Some(owner)), 104);
    frame.hour = 105;
    assert_eq!(look(&frame, owner).unwrap().civil_war, Ok(false));
    assert_eq!(naming_through(&frame, Some(owner)), i32::MAX);
}

#[test]
fn a_parts_id_is_its_source_as_main_named_it_whatever_its_handle() {
    let term = named_part("7", 0, &key(StabilityPartKeyRow::TERM, b"base"), None, COUNTRY);
    assert_eq!((term.kind.as_str(), term.id.as_deref(), term.term.as_deref()), ("term", Some("term:base"), Some("base")));
    for (kind, id) in [(StabilityPartKeyRow::REMAINDER, "remainder"), (StabilityPartKeyRow::CLAMP, "clamp"), (StabilityPartKeyRow::TOTAL, "total")] {
        let part = named_part("7", 1, &key(kind, b""), None, COUNTRY);
        assert_eq!((part.id.as_deref(), part.measure.as_str(), part.line.is_none()), (Some(id), "warSupport", true));
    }
    let unpaired = named_part("7", 0, &key(CountryStabilityLineRow::UNPAIRED, b"Territorial Integrity"), None, COUNTRY);
    assert_eq!(unpaired.id.as_deref(), Some("unpaired:Territorial Integrity"));
    let line = unpaired.line.unwrap();
    assert_eq!((line.kind.as_str(), line.text.as_deref(), line.identity.as_str()), ("unpaired", Some("Territorial Integrity"), ""));
    let refused = named_part("7", 0, &key(StabilityPartKeyRow::REFUSED_LABEL, b"a\tb"), None, COUNTRY);
    assert_eq!(refused.id.as_deref(), Some("unpaired-refused:a\\tb"));
    assert_eq!(refused.line.unwrap().text, None);
    let mut idea = key(CountryStabilityLineRow::IDEA, b"some_idea");
    idea.secondary = -3;
    let idea = named_part("7", 0, &idea, None, COUNTRY);
    assert_eq!(idea.id.as_deref(), Some("line:idea:some_idea:0:-3"));
    assert_eq!(idea.line.unwrap().label, None, "no content names nothing, never a guess");
}

#[test]
fn a_character_line_carries_its_recorded_character_and_a_character_not_known_its_base() {
    let mut advisor = key(CountryStabilityLineRow::ADVISOR, b"adv_idea");
    advisor.identity_id = 41;
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0"));
    advisor.saved = Recorded::Value(b"SAVED".to_vec());
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0::SAVED"));
    advisor.script = Recorded::Value(b"script_token".to_vec());
    advisor.template_present = Recorded::Value(1);
    advisor.template_tag = Recorded::Value(b"TAG".to_vec());
    advisor.template_file = Recorded::Value(b"file.txt".to_vec());
    advisor.template_line = Recorded::Value(12);
    advisor.template_generated = Recorded::Value(0);
    assert_eq!(named_part("7", 0, &advisor, None, COUNTRY).id.as_deref(), Some("line:advisor:adv_idea:41:0:script_token:SAVED"));
    let character = part_character(&advisor).unwrap();
    let template = character.template.unwrap();
    assert_eq!((template.tag.as_str(), template.file.as_slice(), template.line, template.generated), ("TAG", b"file.txt".as_slice(), 12, false));
    advisor.template_present = Recorded::Value(0);
    assert!(part_character(&advisor).unwrap().template.is_err(), "a character with no template");
    advisor.template_present = Recorded::NotRecorded;
    assert!(part_character(&advisor).unwrap().template.is_err(), "a template not recorded is not known");
}

#[test]
fn a_keys_lines_add_to_its_total_and_what_they_leave_is_the_remainder() {
    let line = |line: u32, key: u32, value: i64| CountryStabilityLineRow { country: 5, line, key, kind: CountryStabilityLineRow::STATIC, identity: b"s".to_vec(), value, ..Default::default() };
    let lines = [line(0, 3, 10), line(1, 3, 15), line(2, 4, 99)];
    let mut refused = dashboard_text::catalogue::Refusals::default();
    let good = |_: u32| Some(true);
    let modifier = modifier((b"stability_factor", 3), Some(40), Some(&lines), &good, &mut refused, "AAA");
    assert_eq!(modifier.lines.len(), 2);
    assert_eq!(modifier.remainder_raw.as_deref(), Some("15"));
    let whole = super::modifier((b"stability_factor", 3), Some(25), Some(&lines), &good, &mut refused, "AAA");
    assert_eq!(whole.remainder_raw, None, "lines that add up leave no remainder");
    let unread = super::modifier((b"stability_factor", 3), Some(25), None, &good, &mut refused, "AAA");
    assert_eq!((unread.lines.len(), unread.remainder_raw), (0, None), "lines not read with the inputs explain nothing");
    let absent = super::modifier((b"stability_factor", u32::MAX), None, Some(&lines), &good, &mut refused, "AAA");
    assert_eq!((absent.index, absent.total_raw, absent.lines.len()), (None, None, 0));
}
