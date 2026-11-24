use super::*;
use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
use replay::campaign::Replay;
use replay_data::rows::{CountryLinkRow, CountryRow, DivisionRow, DivisionTemplateIconRow, LexicalTokenRow, TableRows, TemplateSubunit};
use replay_frame::frame::{DecodedFrame, TableView};

const FIRST: i32 = 1_000;
const LAST: i32 = 2_600;
const UNREAD: (i32, i32) = (1_300, 1_340);
const TOKENS: [(u32, &str); 4] = [(10, "infantry"), (11, "artillery_brigade"), (12, "engineer"), (13, "recon")];

fn unit(token: u32, count: i32) -> TemplateSubunit {
    TemplateSubunit { token, local: 0, count }
}

fn template(handle: u64, units: Vec<TemplateSubunit>) -> DivisionTemplateIconRow {
    DivisionTemplateIconRow { template: handle, subunits: units, ..Default::default() }
}

fn division(id: u64, roster: u32, expeditionary: u32, template: u64) -> DivisionRow {
    DivisionRow { id, roster_owner: roster, logical_owner: roster, expeditionary_owner: expeditionary, template, hit_points: 100_000, ..Default::default() }
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = source(1, hour, Some(1));
    if let TableView::Complete(TableRows::Countries(rows)) = &mut *frame.tables[TableId::Countries.index()] {
        rows.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ITA".to_vec(), ..CountryRow::default() });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) = &mut *frame.tables[TableId::CountryLinks.index()] {
        rows.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
    }
    let mut divisions = vec![division(1, 9001, 0, 1)];
    if (1_100..1_900).contains(&hour) {
        divisions.push(division(2, 9001, 0, 2));
    }
    if hour >= 1_200 {
        divisions.push(division(3, 9002, 0, 2));
    }
    if hour >= 1_700 {
        divisions.push(division(4, 9002, 9001, 1));
    }
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
    let second = if hour < 1_500 { vec![unit(10, 1), unit(12, 3)] } else { vec![unit(10, 1), unit(12, 4), unit(13, 1)] };
    *frame.tables[TableId::DivisionTemplateIcons.index()] = if (UNREAD.0..UNREAD.1).contains(&hour) {
        TableView::Unavailable { reason: "templates unread in the fixture".into() }
    } else {
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![template(1, vec![unit(10, 2), unit(11, 1)]), template(2, second)]))
    };
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(TOKENS.iter().map(|&(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).collect()));
    frame
}

fn recording() -> Source {
    let frames = (FIRST..=LAST).map(frame).collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(dashboard_base::SourceKey { source_id: "army".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &AtomicBool::new(false)).unwrap()
}

fn walked(recording: &Recording, attribution: u128, parts: &BTreeMap<Country, Vec<Part>>) -> (HashMap<(u32, i32, i32, u32), MixRuns>, HashMap<u32, String>) {
    let battalions = dashboard_declare::field(TableId::ArmyTotals, if attribution == 0 { "owner_battalions" } else { "host_battalions" }).unwrap();
    let mut by_stretch: BTreeMap<(u32, i32, i32), BTreeSet<u32>> = BTreeMap::new();
    for part in parts.values().flatten() {
        by_stretch.entry((part.run, part.first, part.last)).or_default().insert(part.slot);
    }
    let mut slot_runs: HashMap<(u32, i32, i32, u32), Vec<(i32, i32, Option<BTreeMap<u32, i64>>)>> = HashMap::new();
    let mut names: HashMap<u32, String> = HashMap::new();
    let tables: [(TableId, Option<&[usize]>); 3] = [(TableId::ArmyMix, None), (TableId::ArmyTotals, Some(&[battalions])), (TableId::LexicalTokens, None)];
    for (&(run, first, last), slots) in &by_stretch {
        let mut held: HashMap<u32, Option<BTreeMap<u32, i64>>> = HashMap::new();
        let mut fresh = true;
        read::walk(recording, run, &tables, first, last, |hour, readers| {
            let (mix, totals, tokens) = (&readers[0], &readers[1], &readers[2]);
            let moved = totals.changes();
            let battalions_moved = moved.anew || moved.status_changed || !moved.inserted.is_empty() || !moved.deleted.is_empty() || moved.written.iter().any(|written| !written.is_empty());
            if fresh || !mix.changes().quiet() || battalions_moved {
                fresh = false;
                let (totals_view, mix_view) = (totals.view(), mix.view());
                let available = totals.available();
                let mut now: HashMap<u32, Option<BTreeMap<u32, i64>>> = slots
                    .iter()
                    .map(|&slot| {
                        let known = available && totals_view.slot(u128::from(slot)).is_some_and(|at| totals_view.recorded(battalions, at));
                        (slot, known.then(BTreeMap::new))
                    })
                    .collect();
                for at in mix_view.rows() {
                    let Some(key) = mix_view.key(at) else { continue };
                    if (key >> 32) & 0xff != attribution {
                        continue;
                    }
                    let Some(Some(parts)) = now.get_mut(&((key >> 40) as u32)) else { continue };
                    if let Some(count) = mix_view.number(0, at) {
                        let token = key as u32;
                        parts.insert(token, count);
                        names.entry(token).or_insert_with(|| {
                            let view = tokens.view();
                            token_name(&view, view.slot(u128::from(token)), token)
                        });
                    }
                }
                held = now;
            }
            for (&slot, parts) in &held {
                let runs = slot_runs.entry((run, first, last, slot)).or_default();
                match runs.last_mut() {
                    Some(previous) if previous.1 + 1 == hour && previous.2 == *parts => previous.1 = hour,
                    _ => runs.push((hour, hour, parts.clone())),
                }
            }
            Ok(())
        })
        .unwrap();
    }
    let known = slot_runs.into_iter().map(|(at, runs)| (at, runs.into_iter().filter_map(|(first, last, held)| Some((first, last, held?))).collect())).collect();
    (known, names)
}

#[test]
fn the_army_composition_from_keyed_runs_equals_the_hour_by_hour_walk() {
    let source = recording();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let recording = query.replay.recording();
    let (mut compared, mut changes, mut gaps) = (0, 0, 0);
    for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
        let side = u128::from(attribution == ArmyAttribution::Host);
        let line = line(&query, ArmyMeasure::Battalions, attribution).unwrap();
        for (first, last) in [(FIRST, LAST), (FIRST + 300, FIRST + 900), (UNREAD.0 - 10, UNREAD.1 + 10), (1_495, 1_505), (1_719, 1_721)] {
            let parts: BTreeMap<Country, Vec<Part>> = ["GER", "ITA"].map(|tag| (Country::from(tag), line.parts(tag, first, last))).into();
            assert!(parts.values().all(|held| !held.is_empty()), "both countries are held through {first}..={last}");
            let found = totals_runs(&query, &line, &parts).unwrap();
            let (ours, names) = mix_runs(recording, side, &parts, &found, |run| dashboard_query::cost::lifetimes(recording, run, TableId::ArmyMix).map(Arc::new), &AtomicBool::new(false)).unwrap();
            let (theirs, their_names) = walked(recording, side, &parts);
            assert_eq!(ours, theirs, "{attribution:?} through {first}..={last}");
            for (token, name) in &their_names {
                assert_eq!(names.get(token), Some(name), "token {token}");
            }
            for runs in ours.values() {
                compared += runs.len();
                changes += runs.len().saturating_sub(1);
                gaps += runs.windows(2).filter(|pair| pair[0].1 + 1 < pair[1].0).count();
            }
        }
    }
    assert!(compared > 20 && changes > 10 && gaps > 0, "{compared} runs, {changes} changes, {gaps} unknown stretches");
    let host = line(&query, ArmyMeasure::Battalions, ArmyAttribution::Host).unwrap();
    let parts: BTreeMap<Country, Vec<Part>> = BTreeMap::from([(Country::from("ITA"), host.parts("ITA", 1_800, 1_800))]);
    let found = totals_runs(&query, &host, &parts).unwrap();
    let (ours, names) = mix_runs(recording, 1, &parts, &found, |run| dashboard_query::cost::lifetimes(recording, run, TableId::ArmyMix).map(Arc::new), &AtomicBool::new(false)).unwrap();
    let held: Vec<&MixRuns> = ours.values().collect();
    assert_eq!(held.len(), 1);
    assert_eq!(held[0], &vec![(1_800, 1_800, BTreeMap::from([(10, 3), (11, 1), (12, 4), (13, 1)]))], "ITA hosts its own division and GER's sent one");
    assert_eq!(names.get(&13).map(String::as_str), Some("recon"));
}

#[test]
fn unit_group_titles_use_group_title_localisation_key_with_bare_fallback() {
    let (cat, _folder) = dashboard_base::catalogue::tests::folder_catalogue(&[
        (
            "common/units/test_units.txt",
            "sub_units = { infantry = { group = infantry } mod_tank = { group = mod_armor } }",
        ),
        (
            "localisation/english/unit_l_english.yml",
            "\u{feff}l_english:\n group_infantry_title:0 \"Infantry\"\n mod_armor:0 \"Mod Armor\"\n",
        ),
    ]);
    let units = content_reader_key_names::unit_leader::sub_units(&cat.content.layers).unwrap();
    let (key, info, warning) = group_info(Some(&units), Some(&cat), "infantry");
    assert_eq!(key, "group:infantry");
    assert_eq!(info.name, "Infantry");
    assert_eq!(warning, None);

    let (mod_key, mod_info, mod_warning) = group_info(Some(&units), Some(&cat), "mod_tank");
    assert_eq!(mod_key, "group:mod_armor");
    assert_eq!(mod_info.name, "Mod Armor");
    assert_eq!(mod_warning, None);
}

#[test]
fn army_at_ranks_and_splits_battalion_groups_at_an_hour() {
    let source = recording();
    let asked = serde_json::to_value(ArmyAtParams { branch_id: "1".into(), countries: Vec::new(), top: Some(2), group: None, count_for: ArmyAttribution::Host }).unwrap();
    let panel = dashboard_base::products::at_hour::exact_panel::<ArmyAt>(&source, &LoadContext { content: None }, &asked).unwrap();
    let reply = (panel.exact().unwrap().cold)(1_800, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply["missing"], serde_json::Value::Null);
    let rows = reply["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["tag"], "GER");
    assert_eq!(rows[0]["total"], 9);
    assert_eq!(rows[1]["tag"], "ITA");
    assert_eq!(rows[1]["total"], 9);
    assert_eq!(rows[1]["partsMissing"], serde_json::Value::Null);
    let sum: i64 = rows[1]["values"].as_array().unwrap().iter().map(|v| v.as_i64().unwrap_or(0)).sum();
    assert_eq!(sum, 9);

    let after = (panel.exact().unwrap().cold)(1_900, &AtomicBool::new(false)).unwrap();
    let after_rows = after["rows"].as_array().unwrap();
    assert_eq!(after_rows.len(), 2);
    assert_eq!(after_rows[0]["tag"], "ITA");
    assert_eq!(after_rows[0]["total"], 9);
    assert_eq!(after_rows[1]["tag"], "GER");
    assert_eq!(after_rows[1]["total"], 3);
}

fn breakdown_fixture() -> Source {
    let tokens = [(10u32, "unit_alpha"), (11, "unit_beta"), (12, "unit_gamma")];
    let frames: Vec<DecodedFrame> = (100..=140)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            let mut countries = vec![CountryRow {
                id: 9001,
                canonical_slot: 7,
                tag: if hour >= 120 { b"ZZZ".to_vec() } else { b"AAA".to_vec() },
                ..CountryRow::default()
            }];
            let mut links = vec![CountryLinkRow { id: 9001, canonical_slot: 7 }];
            if hour >= 120 {
                countries.push(CountryRow { id: 9009, canonical_slot: 9, tag: b"AAA".to_vec(), ..CountryRow::default() });
                links.push(CountryLinkRow { id: 9009, canonical_slot: 9 });
            }
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(links));
            let mut divisions = vec![division(1, 9001, 0, 1)];
            if hour >= 120 {
                divisions.push(division(2, 9001, 0, 2));
                divisions.push(division(3, 9009, 0, 3));
            }
            *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(divisions));
            *frame.tables[TableId::DivisionTemplateIcons.index()] = if (125..=129).contains(&hour) {
                TableView::Unavailable { reason: "templates unread in test".into() }
            } else {
                TableView::Complete(TableRows::DivisionTemplateIcons(vec![
                    template(1, vec![unit(10, 2), unit(11, 1)]),
                    template(2, vec![unit(12, 4)]),
                    template(3, vec![unit(12, 5)]),
                ]))
            };
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
                tokens.iter().map(|&(token, literal)| LexicalTokenRow { token, literal: literal.as_bytes().to_vec() }).collect(),
            ));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    Source::from_replay(
        dashboard_base::SourceKey { source_id: "army-breakdown-test".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn army_breakdown_excludes_records_first_seen_after_hour_reports_next_change_and_keeps_slot_separate_on_reused_tag() {
    let src = breakdown_fixture();
    let ctx = LoadContext { content: None };

    let at119 = ArmyBreakdown::load(
        &src,
        &ArmyBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), count_for: ArmyAttribution::Owner, hour: 119 },
        &ctx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(at119.hour, 119);
    assert_eq!(at119.next_change, Some(120));
    assert!(at119.available);
    assert_eq!(at119.types.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["unknown:unit_alpha", "unknown:unit_beta"]);
    assert!(at119.runs.iter().all(|r| r.last_hour <= 119 && r.raw == "3"));
    assert!(at119.mix.iter().all(|m| m.last_hour <= 119));
    assert!(at119.warnings.iter().all(|w| !w.contains("unit_gamma")), "future token warning must not leak before hour 120");

    let at120 = ArmyBreakdown::load(
        &src,
        &ArmyBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), count_for: ArmyAttribution::Owner, hour: 120 },
        &ctx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(at120.hour, 120);
    assert_eq!(at120.next_change, Some(125));
    assert_eq!(at120.types.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["unknown:unit_alpha", "unknown:unit_beta", "unknown:unit_gamma"]);
    assert_eq!(at120.runs.last().unwrap().raw, "7");

    let at135 = ArmyBreakdown::load(
        &src,
        &ArmyBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), count_for: ArmyAttribution::Owner, hour: 135 },
        &ctx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(at135.next_change, None);
    assert!(at135.missing.iter().any(|m| m.first_hour == 125 && m.last_hour == 129), "unread template window produces a missing span rather than zero");
    assert!(at135.runs.iter().all(|r| r.last_hour < 125 || r.first_hour > 129));
    assert!(at135.mix.iter().all(|m| m.last_hour < 125 || m.first_hour > 129));

    let slot9 = ArmyBreakdown::load(
        &src,
        &ArmyBreakdownParams { branch_id: "1".into(), country: Country::Slot(9), count_for: ArmyAttribution::Owner, hour: 135 },
        &ctx,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(slot9.types.iter().map(|t| t.key.as_str()).collect::<Vec<_>>(), vec!["unknown:unit_gamma"], "reused tag AAA on slot 9 must not inherit slot 7 history");
    assert_eq!(slot9.runs.first().unwrap().first_hour, 120);
    assert_eq!(slot9.runs.first().unwrap().raw, "5");

    let asked = serde_json::to_value(ArmyAtParams { branch_id: "1".into(), countries: vec![Country::Slot(7)], top: None, group: None, count_for: ArmyAttribution::Owner }).unwrap();
    let panel = dashboard_base::products::at_hour::exact_panel::<ArmyAt>(&src, &ctx, &asked).unwrap();
    let hover115 = (panel.exact().unwrap().cold)(115, &AtomicBool::new(false)).unwrap();
    assert_eq!(hover115["rows"][0]["total"], 3, "earlier hover reads hour 115 rather than playhead total 7");
    assert_eq!(hover115["parts"].as_array().unwrap().len(), 2);
}

#[test]
fn army_breakdown_keeps_witnessed_totals_and_withholds_mix_when_army_mix_table_is_unavailable() {
    use replay_data::rows::{ArmyMixRow, ArmyTotalRow, Recorded, TagHolderRow, TagNameRow};
    let frames: Vec<DecodedFrame> = (200..=210)
        .map(|hour| {
            let mut frame = source(1, hour, Some(1));
            *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
                CountryRow { id: 9001, canonical_slot: 7, tag: b"AAA".to_vec(), ..CountryRow::default() },
            ]));
            *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow { id: 9001, canonical_slot: 7 },
            ]));
            *frame.tables[TableId::TagNames.index()] = TableView::Complete(TableRows::TagNames(vec![
                TagNameRow { number: 1, tag: b"AAA".to_vec() },
            ]));
            *frame.tables[TableId::TagHolders.index()] = TableView::Complete(TableRows::TagHolders(vec![
                TagHolderRow { number: 1, epoch: 0, slot: 7, id: 9001, tag: b"AAA".to_vec() },
            ]));
            *frame.tables[TableId::ArmyTotals.index()] = TableView::Complete(TableRows::ArmyTotals(vec![
                ArmyTotalRow { slot: 7, owner_battalions: Recorded::Value(6), ..Default::default() },
            ]));
            *frame.tables[TableId::ArmyMix.index()] = if (204..=206).contains(&hour) {
                TableView::Unavailable { reason: "mix table unread".into() }
            } else {
                TableView::Complete(TableRows::ArmyMix(vec![
                    ArmyMixRow { slot: 7, attribution: 0, token: 10, battalions: 6 },
                ]))
            };
            *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow { token: 10, literal: b"unit_alpha".to_vec() },
            ]));
            frame
        })
        .collect();
    let replay = Replay::from_records(recorder_derive_framework::framework::Writers::new(Vec::new()), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
    let src = Source::from_replay(
        dashboard_base::SourceKey { source_id: "army-mix-gap".into(), revision: "1".into() },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ArmyBreakdown::load(
        &src,
        &ArmyBreakdownParams { branch_id: "1".into(), country: Country::Slot(7), count_for: ArmyAttribution::Owner, hour: 210 },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.runs, vec![ArmyRun { first_hour: 200, last_hour: 210, total: Some(6.0), raw: "6".into() }], "witnessed totals stay intact across unread mix hours");
    assert_eq!(reply.mix.iter().map(|m| (m.first_hour, m.last_hour)).collect::<Vec<_>>(), vec![(200, 203), (207, 210)], "mix is not fabricated across unread mix hours");
    assert_eq!(reply.missing.len(), 1);
    assert_eq!((reply.missing[0].first_hour, reply.missing[0].last_hour), (204, 206));
}
