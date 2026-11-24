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
