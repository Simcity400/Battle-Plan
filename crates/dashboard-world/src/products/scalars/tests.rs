use super::{factory_groups, factory_parts, game_score, nuclear_arsenal, world_tension, world_tension_stack};
use replay_data::rows::{CountryFactoriesRow, CountryGameScoreRow, NukeRow, Recorded, ThreatEventRow};
use world_tension_stack::{Identity, Joined, Step, join};

fn source(country: u32, reason: &str, place: u32) -> Identity {
    Identity { country, second: 0, date: 100, reason: reason.as_bytes().to_vec(), place }
}

fn row(hour: i32, kind: u8, identity: &Identity, final_raw: i64) -> Step {
    Step::Row { hour, kind, identity: identity.clone(), final_raw, slot: Some(identity.country + 1000) }
}

#[test]
fn a_threat_source_is_one_event_from_its_listing_to_its_drop_across_a_stretch() {
    let (a, b, twin) = (source(1, "annexed", 0), source(2, "justified", 0), source(1, "annexed", 1));
    let (events, unmatched) = join([
        Step::Stretch { first: 10, last: 19, listed: Some(vec![(a.clone(), 500, Some(1001))]) },
        row(12, ThreatEventRow::LISTED, &b, 300),
        row(13, ThreatEventRow::LISTED, &twin, 500),
        row(14, ThreatEventRow::CHANGED, &a, 700),
        row(15, ThreatEventRow::DROPPED, &b, 300),
        Step::Stretch { first: 20, last: 29, listed: None },
        Step::Stretch { first: 30, last: 39, listed: Some(vec![(a.clone(), 700, Some(1001))]) },
        row(31, ThreatEventRow::DROPPED, &source(9, "never listed", 0), 0),
    ]);
    assert_eq!(unmatched, [31]);
    assert_eq!(
        events,
        [
            Joined { identity: a, slot: Some(1001), added: vec![(10, 500), (14, 700)], first_read: 10, last_read: None, removed_at: None },
            Joined { identity: b, slot: Some(1002), added: vec![(12, 300)], first_read: 12, last_read: Some(14), removed_at: Some(15) },
            Joined { identity: twin, slot: Some(1001), added: vec![(13, 500)], first_read: 13, last_read: Some(19), removed_at: Some(30) },
        ]
    );
}

#[test]
fn unread_threat_source_snapshot_keeps_open_events_and_dates_removals() {
    let a = source(1, "annexed", 0);
    let (events, unmatched) = join([
        Step::Stretch { first: 10, last: 19, listed: Some(vec![(a, 500, Some(1001))]) },
        Step::Stretch { first: 20, last: 29, listed: None },
    ]);
    assert!(unmatched.is_empty());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].last_read, None);
    assert_eq!(events[0].removed_at, None);

    let dated = [(50, "later failure".to_owned())];
    let mut next_change = None;
    let early = world_tension_stack::filter_dated_warnings(&dated, &[60], &[70], 20, &mut next_change);
    assert!(early.is_empty(), "later failures must not appear on earlier dates");
    assert_eq!(next_change, Some(50));
    let mut next_change_late = None;
    let late = world_tension_stack::filter_dated_warnings(&dated, &[60], &[70], 65, &mut next_change_late);
    assert_eq!(late.len(), 2);
    assert_eq!(next_change_late, Some(70));
}

#[test]
fn tension_markers_keep_the_witness_at_their_hour_and_report_an_unread_listing() {
    use dashboard_base::{Source, SourceKey, products::{LoadContext, Product}, test_fixtures};
    use replay_data::rows::{TableId, TableRows, ThreatSourceRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};
    use world_tension_stack::{WorldTensionStack, WorldTensionStackParams};

    let mut damaged = test_fixtures::source(1, 150, None);
    *damaged.tables[TableId::ThreatSources.index()] = TableView::Unavailable { reason: "Fixture block unreadable".into() };
    let (step, missing) = world_tension_stack::listing_snapshot(&damaged, 150, 160);
    assert!(matches!(step, Step::Stretch { listed: None, .. }));
    let missing = missing.unwrap();
    assert_eq!((missing.first_hour, missing.last_hour), (150, 160));
    assert!(missing.reason.contains("Fixture block unreadable"));

    let frames = (100..=103).map(|hour| {
        let mut frame = test_fixtures::source(1, hour, None);
        *frame.tables[TableId::ThreatSources.index()] = if hour == 102 {
            TableView::Unavailable { reason: "Fixture listing unread".into() }
        } else {
            TableView::Complete(TableRows::ThreatSources([0, 1].map(|ordinal| ThreatSourceRow {
                ordinal, country: 9001, reason: b"fixture".to_vec(), date: 100,
                final_raw: match (ordinal, hour) { (0, 100) => 5, (0, _) => 9, (1, 100) => 3, _ => 1 },
                ..Default::default()
            }).to_vec()))
        };
        frame
    }).collect();
    let cancelled = AtomicBool::new(false);
    let replay = test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "tension-marker".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap();
    for hour in 101..=103 {
        let reply = WorldTensionStack::load(&source, &WorldTensionStackParams {
            branch_id: "1".into(), first_hour: 100, last_hour: hour, layers: 1, values: Default::default(), countries: None,
            top: None, group: None, width: None, include_stack: Some(false), include_events: Some(true),
        }, &LoadContext { content: None }, &cancelled).unwrap();
        assert_eq!(reply.events.len(), 2);
        assert_eq!(reply.listing_missing.as_deref(), (hour == 102).then_some("Fixture listing unread"));
        for event in reply.events {
            assert_eq!(event.marker_raw.as_deref(), Some(if event.place == 0 { "5" } else { "3" }));
            assert_eq!(event.added_raw, if event.place == 0 { "9" } else { "1" });
        }
        if hour < 103 { assert_eq!(reply.next_change_hour, Some(hour + 1)); }
    }
}

#[test]
fn world_tension_playback_visits_only_slot_and_event_edits_entering_or_expiring() {
    use dashboard_base::{Source, SourceKey, products::{LoadContext, Product, at_hour::{counted, exact_panel}}, test_fixtures};
    use replay_data::rows::{TableId, TableRows, ThreatCountryAggregateRow, ThreatSourceRow, WorldTensionRow};
    use replay_frame::frame::TableView;
    use std::{collections::BTreeSet, sync::Arc};
    use world_tension_stack::{TensionValueMode, WorldTensionStack, WorldTensionStackParams};

    let amount = |hour: i32| if hour % 2 == 0 { i64::from(hour) } else { 47 };
    let frames = (100..=145).map(|hour| {
        let mut frame = test_fixtures::source(1, hour, None);
        *frame.tables[TableId::WorldTension.index()] = TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow { total_raw: hour.into(), ..Default::default() }]));
        *frame.tables[TableId::ThreatCountryAggregates.index()] = TableView::Complete(TableRows::ThreatCountryAggregates(
            (1..=48).map(|country_slot| ThreatCountryAggregateRow { country_slot, total_raw: if country_slot == 1 { amount(hour) } else { country_slot.into() } }).collect()));
        *frame.tables[TableId::ThreatSources.index()] = TableView::Complete(TableRows::ThreatSources(
            (0..=hour - 100).map(|offset| ThreatSourceRow { ordinal: offset as u32, country: 9001, date: 100 + offset,
                final_raw: if offset == 30 { hour.into() } else { 1 }, reason: b"fixture".to_vec(), ..Default::default() }).collect()));
        frame
    }).collect();
    let cancelled = false.into();
    let replay = test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "tension-played".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap();
    let load = LoadContext { content: None };
    let driver = exact_panel::<world_tension::WorldTension>(&source, &load, &serde_json::json!({ "branchId": "1" })).unwrap();
    let played: BTreeSet<i32> = test_fixtures::played(&source.replay(), &driver.exact().unwrap(), 130, 145)
        .into_iter().map(|step| step.hour).collect();
    assert_eq!(played.len(), 16);
    let mut visits = Vec::new();
    for (stack, layers) in [(true, 2), (false, 2), (true, 1)] {
        let mut total = 0;
        for &hour in &played {
            let (reply, visited) = counted(|| WorldTensionStack::load(&source, &WorldTensionStackParams {
                branch_id: "1".into(), first_hour: hour - 10, last_hour: hour, layers, values: TensionValueMode::Recorded,
                countries: None, top: None, group: None, width: None, include_stack: Some(stack), include_events: Some(!stack),
            }, &load, &cancelled).unwrap());
            if stack {
                let (leader, raw) = if hour % 2 == 0 { (1, i64::from(hour)) } else { (48, 48) };
                assert_eq!(reply.bands[0].slot, leader);
                assert_eq!(reply.bands[0].end_raw, Some(raw.to_string()));
                assert_eq!(reply.hours[0], hour - 10);
            } else {
                assert_eq!((reply.events.first().unwrap().hour, reply.events.last().unwrap().hour), (hour - 10, hour));
                assert_eq!(reply.events.len(), 11);
                if hour <= 140 {
                    let revised = reply.events.iter().find(|event| event.hour == 130).unwrap();
                    assert_eq!(revised.added_raw, hour.to_string());
                    assert_eq!(revised.marker_raw.as_deref(), Some("130"));
                }
            }
            if hour > 130 { total += visited; }
        }
        visits.push(total);
    }
    for hour in [136, 133, 141, 132] {
        let reply = WorldTensionStack::load(&source, &WorldTensionStackParams {
            branch_id: "1".into(), first_hour: hour - 10, last_hour: hour, layers: 2, values: TensionValueMode::Recorded,
            countries: None, top: None, group: None, width: None, include_stack: Some(false), include_events: Some(true),
        }, &load, &cancelled).unwrap();
        assert_eq!(reply.events.iter().map(|e| e.hour).collect::<Vec<_>>(), (hour - 10..=hour).collect::<Vec<_>>());
        assert_eq!(reply.events.iter().find(|e| e.hour == 130).map(|e| &e.added_raw), (hour <= 140).then(|| hour.to_string()).as_ref());
    }
    println!("slot and event edit visits: {visits:?}");
    assert!(visits.iter().all(|&count| count <= 15 * 6), "stack and event visits for fifteen entry and expiry steps: {visits:?}");
}

#[test]
fn a_nuclear_total_adds_the_shown_counts_as_the_top_bar_does_and_ranks_ties_by_tag() {
    let nuke = |country: u32, nuke_type: u32, bombs: i64| NukeRow { country, nuke_type, amount_raw: bombs * 100_000_000, ready: 0 };
    let rows = [nuke(1, 0, 3), nuke(1, 1, 2), nuke(2, 0, 5), nuke(3, 0, 1), nuke(4, 0, 9)];
    let tag = |country: u32| match country {
        1 => Some("GER".to_owned()),
        2 => Some("ENG".to_owned()),
        3 => Some("FRA".to_owned()),
        _ => None,
    };
    let (countries, unheld, unplaced) = nuclear_arsenal::countries(&rows, tag);
    assert_eq!(unheld, 1, "a country no tag holds is counted, not shown");
    assert_eq!(unplaced, [9]);
    let shown: Vec<(&str, i32, usize)> = countries.iter().map(|country| (country.tag.as_str(), country.total, country.counts.len())).collect();
    assert_eq!(shown, [("ENG", 5, 1), ("GER", 5, 2), ("FRA", 1, 1)]);
    let top = nuclear_arsenal::top_countries(countries.clone(), &unplaced, false, 2).unwrap();
    assert_eq!(top.iter().map(|country| country.tag.as_str()).collect::<Vec<_>>(), ["ENG", "GER"]);
    assert!(nuclear_arsenal::top_countries(countries.clone(), &unplaced, true, 2).is_err(), "a contending unresolved holder refuses the top");
    let zero_and_positive = nuclear_arsenal::countries(&[nuke(1, 0, 0), nuke(2, 0, 2)], tag).0;
    assert_eq!(nuclear_arsenal::top_countries(zero_and_positive, &[], false, 5).unwrap().iter().map(|c| c.tag.as_str()).collect::<Vec<_>>(), ["ENG"]);
    let mut rank = serde_json::to_value(&countries).unwrap();
    let mut changed = rows.to_vec();
    changed[0] = nuke(1, 0, 8);
    nuclear_arsenal::rerank(&mut rank, &changed, 1, "GER", None).unwrap();
    assert_eq!(rank, serde_json::to_value(nuclear_arsenal::countries(&changed, tag).0).unwrap());
    let (sentinel, _, _) = nuclear_arsenal::countries(&[NukeRow { country: 1, nuke_type: 0, amount_raw: i64::MAX, ready: 0 }, nuke(1, 1, 1)], tag);
    assert_eq!(sentinel[0].total, 0xa7c5 + 1, "the top bar adds the game's sentinel as it shows it");
    let chosen = nuclear_arsenal::chosen_countries(countries, &[(9, 3, "FRA".to_owned()), (8, 5, "ITA".to_owned()), (7, 1, "GER".to_owned())]);
    assert_eq!(
        chosen.iter().map(|country| (country.tag.as_str(), country.slot, country.total)).collect::<Vec<_>>(),
        [("GER", Some(7), 5), ("FRA", Some(9), 1), ("ITA", Some(8), 0)],
        "each chosen holder is one row keyed by its slot, the product orders them, and one with no stockpile row in a whole table holds none"
    );
}

#[test]
fn world_tensions_moving_numbers_are_live_cells_of_its_rows_and_the_unnamed_part_only_where_there_is_one() {
    let columns = world_tension::Columns { tension: 0, sum: 0, contribution: 0, current: 2 };
    let cells = dashboard_follow::live::Cells::default();
    world_tension::name_cells(&cells, &columns, &[4, 9], &[], &[0, 3]);
    let json = cells.finish().json(&[]);
    assert_eq!(json["rows"], serde_json::json!(["world", "4", "9", "source0", "source3"]));
    let kinds: Vec<&str> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["tension", "sum", "value", "current"], "no unnamed part where every slot is named");
    let cells = dashboard_follow::live::Cells::default();
    world_tension::name_cells(&cells, &columns, &[4], &[7, 8], &[]);
    let json = cells.finish().json(&[]);
    let kinds: Vec<&str> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["tension", "sum", "unnamed", "value"]);
}

#[test]
fn factory_parts_truncate_as_the_game_shows_and_name_an_unavailable_count_withheld_parts_do_not_explain() {
    let row = CountryFactoriesRow {
        military_total_raw: 1_299_999,
        military_controlled_raw: 1_000_000,
        military_owned_raw: 900_000,
        military_damaged_raw: 199_999,
        military_to_overlord: 2,
        military_to_others: 1,
        military_unavailable: 3,
        civilian_total_raw: 3_000_000,
        civilian_unavailable: 99,
        consumer_goods: 5,
        trade_imports: 3,
        ..CountryFactoriesRow::default()
    };
    let (parts, unexplained) = factory_parts::parts(&row).unwrap();
    let value = |group: &str, part: &str| parts.iter().find(|held| held.group == group && held.part == part).map(|held| held.value);
    assert_eq!((value("military", "total"), value("military", "damaged"), value("military", "owned")), (Some(12), Some(1), Some(9)));
    assert_eq!((value("civilian", "consumerGoods"), value("civilian", "tradeImports"), value("naval", "consumerGoods")), (Some(5), Some(3), None));
    assert_eq!(parts.len(), 31);
    assert_eq!(unexplained, ["civilian"]);
    assert!(factory_parts::parts(&CountryFactoriesRow { naval_total_raw: i64::MAX, ..row }).is_none());
}

#[test]
fn a_game_score_part_is_its_value_or_the_named_reason_it_was_not_recorded_never_zero() {
    let score = CountryGameScoreRow {
        country: 7,
        army: Recorded::Value(1_250_000),
        air: Recorded::NotRecorded,
        air_reason: CountryGameScoreRow::NO_WING,
        total: Recorded::NotRecorded,
        total_reason: CountryGameScoreRow::TOTAL_PARTS,
        ..CountryGameScoreRow::default()
    };
    let shown = game_score::row("GER", &score);
    assert_eq!((shown.tag.as_str(), shown.country_id), ("GER", 7));
    assert_eq!(shown.army, game_score::GameScoreCell { missing: None });
    assert_eq!(shown.air.missing.as_deref(), Some(CountryGameScoreRow::REASONS[usize::from(CountryGameScoreRow::NO_WING)]));
    assert_eq!(shown.total.missing.as_deref(), Some(CountryGameScoreRow::REASONS[usize::from(CountryGameScoreRow::TOTAL_PARTS)]));
    assert!(game_score::cell(&Recorded::NotRecorded, CountryGameScoreRow::KNOWN).missing.is_some(), "an unrecorded value with no reason is still not a zero");
    let cells = dashboard_follow::live::Cells::default();
    game_score::name_cells(&cells, "GER", &score, &[5, 0, 1, 2, 3, 4]);
    let live = cells.finish();
    assert_eq!(live.json(&[])["rows"], serde_json::json!(["GER"]));
    assert_eq!(live.json(&[])["kinds"].as_array().map(Vec::len), Some(4), "army, navy, industry and land: air and total are not recorded");
}

#[test]
fn the_game_score_top_is_ranked_in_the_product_and_refused_by_name_while_any_country_lacks_the_score() {
    let scored = |army: Recorded<i64>, army_reason: u8| CountryGameScoreRow { army, army_reason, ..CountryGameScoreRow::default() };
    let (ger, ita, sov, eng) = (scored(Recorded::Value(30), CountryGameScoreRow::KNOWN), scored(Recorded::Value(50), CountryGameScoreRow::KNOWN),
        scored(Recorded::Value(30), CountryGameScoreRow::KNOWN), scored(Recorded::NotRecorded, CountryGameScoreRow::COUNT_UNREAD));
    let ask = |count| game_score::GameScoreTopParams { count, by: "army".into() };
    let complete = [("GER", &ger), ("ITA", &ita), ("SOV", &sov)];
    assert_eq!(game_score::top(&complete, &[], &ask(2)), game_score::GameScoreTop { tags: vec!["ITA".into(), "GER".into()], missing: None }, "highest first, a tie broken by tag");
    assert_eq!(game_score::top(&complete, &[], &ask(9)).tags, ["ITA", "GER", "SOV"]);
    let (low, high) = (scored(Recorded::Value(10), CountryGameScoreRow::KNOWN), scored(Recorded::Value(40), CountryGameScoreRow::KNOWN));
    let zero = scored(Recorded::Value(0), CountryGameScoreRow::KNOWN);
    let with_zero = [("GER", &ger), ("ITA", &ita), ("SOV", &sov), ("ABK", &zero)];
    assert_eq!(game_score::top(&with_zero, &[], &ask(9)).tags, ["ITA", "GER", "SOV"], "zero scores are excluded from a count/total top");
    assert_eq!(game_score::top(&complete, &[&zero], &ask(9)).tags, ["ITA", "GER", "SOV"], "an unresolved zero-score country cannot enter a short top");
    assert_eq!(game_score::top(&complete, &[&low], &ask(2)).tags, ["ITA", "GER"], "a country whose holder is unresolved but scores below the top cannot change it");
    let unproven = game_score::top(&complete, &[&high], &ask(2));
    assert!(unproven.tags.is_empty(), "an unresolved country that could rank in the top makes the top unproven");
    assert_eq!(unproven.missing.as_deref(), Some("The top 2 by army score is not known: 1 scored countries could not be matched to a tag at this hour and could be among them"));
    assert!(game_score::top(&complete, &[&eng], &ask(2)).missing.is_some(), "an unresolved country with no score could be anywhere");
    assert!(game_score::top(&complete, &[&low], &ask(9)).missing.is_some(), "a short top could hold any unresolved country");
    let partial = [("GER", &ger), ("ITA", &ita), ("ENG", &eng)];
    let refused = game_score::top(&partial, &[], &ask(1));
    assert!(refused.tags.is_empty(), "with one unread score the apparent leader is unproven");
    assert_eq!(refused.missing.as_deref(), Some("The top 1 by army score needs every country's army score, and 1 of 3 were not recorded at this hour"));
    let unknown_part = game_score::GameScoreTopParams { count: 1, by: "morale".into() };
    assert!(game_score::top(&complete, &[], &unknown_part).missing.is_some(), "a part the product does not score ranks nothing");
}

#[test]
fn game_score_top_advance_invalidates_only_when_membership_coverage_or_ranking_changes() {
    use dashboard_base::products::at_hour::Edits;
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};
    use std::collections::BTreeMap;

    let score = |country: u32, total: Recorded<i64>, army: Recorded<i64>| CountryGameScoreRow {
        country,
        total,
        army,
        navy: Recorded::Value(0),
        air: Recorded::Value(0),
        industry: Recorded::Value(0),
        land: Recorded::Value(0),
        ..CountryGameScoreRow::default()
    };
    let make_frame = |rows: Vec<CountryGameScoreRow>| {
        let mut tables = std::array::from_fn(|_| not_selected());
        tables[TableId::CountryGameScores.index()] = TableView::Complete(TableRows::CountryGameScores(rows)).into();
        DecodedFrame { branch: 1, hour: 100, tables, metrics: Metrics::default() }
    };
    let holders = Holders::new(
        BTreeMap::from([
            ("GER".to_owned(), vec![HolderRun { first: 0, last: 200, holder: Holder::Found { slot: 1, id: 1 }, starts_segment: true }]),
            ("ITA".to_owned(), vec![HolderRun { first: 0, last: 200, holder: Holder::Found { slot: 2, id: 2 }, starts_segment: true }]),
            ("SOV".to_owned(), vec![HolderRun { first: 0, last: 200, holder: Holder::Found { slot: 3, id: 3 }, starts_segment: true }]),
        ]),
        Vec::new(),
    );
    let columns: [usize; 6] = std::array::from_fn(|i| {
        dashboard_declare::field(TableId::CountryGameScores, ["total", "army", "navy", "air", "industry", "land"][i]).unwrap()
    });
    let base_rows = [
        score(1, Recorded::Value(500), Recorded::Value(100)),
        score(2, Recorded::Value(400), Recorded::Value(200)),
        score(3, Recorded::Value(300), Recorded::Value(50)),
    ];
    let cells = dashboard_follow::live::Cells::default();
    for (tag, row) in [("GER", &base_rows[0]), ("ITA", &base_rows[1]), ("SOV", &base_rows[2])] {
        game_score::name_cells(&cells, tag, row, &columns);
    }
    let live = cells.finish();
    let params = game_score::GameScoreParams {
        branch_id: "1".into(),
        top: Some(game_score::GameScoreTopParams { count: 2, by: "total".into() }),
    };
    let held_top = serde_json::json!({ "tags": ["GER", "ITA"], "missing": null });
    let mask_of = |name: &str| 1u64 << dashboard_declare::field(TableId::CountryGameScores, name).unwrap();
    let edit = |country: u32, mask: u64| Edits {
        tables: BTreeMap::from([(TableId::CountryGameScores, BTreeMap::from([(u128::from(country), mask)]))]),
        ..Edits::default()
    };

    let army_changed = make_frame(vec![
        score(1, Recorded::Value(500), Recorded::Value(999)),
        score(2, Recorded::Value(400), Recorded::Value(200)),
        score(3, Recorded::Value(300), Recorded::Value(50)),
    ]);
    assert!(game_score::advance_keeps(&army_changed, &params, &holders, &edit(1, mask_of("army")), &live, Some(&held_top)).unwrap());

    let same_order = make_frame(vec![
        score(1, Recorded::Value(550), Recorded::Value(100)),
        score(2, Recorded::Value(400), Recorded::Value(200)),
        score(3, Recorded::Value(300), Recorded::Value(50)),
    ]);
    assert!(game_score::advance_keeps(&same_order, &params, &holders, &edit(1, mask_of("total")), &live, Some(&held_top)).unwrap());

    let swapped_order = make_frame(vec![
        score(1, Recorded::Value(500), Recorded::Value(100)),
        score(2, Recorded::Value(600), Recorded::Value(200)),
        score(3, Recorded::Value(300), Recorded::Value(50)),
    ]);
    assert!(!game_score::advance_keeps(&swapped_order, &params, &holders, &edit(2, mask_of("total")), &live, Some(&held_top)).unwrap());

    let membership_changed = make_frame(vec![
        score(1, Recorded::Value(500), Recorded::Value(100)),
        score(2, Recorded::Value(400), Recorded::Value(200)),
        score(3, Recorded::Value(450), Recorded::Value(50)),
    ]);
    assert!(!game_score::advance_keeps(&membership_changed, &params, &holders, &edit(3, mask_of("total")), &live, Some(&held_top)).unwrap());

    let coverage_lost = make_frame(vec![
        score(1, Recorded::Value(500), Recorded::Value(100)),
        score(2, Recorded::Value(400), Recorded::Value(200)),
        score(3, Recorded::NotRecorded, Recorded::Value(50)),
    ]);
    assert!(!game_score::advance_keeps(&coverage_lost, &params, &holders, &edit(3, mask_of("total")), &live, Some(&held_top)).unwrap());
}

#[test]
fn factory_groups_are_named_by_every_type_their_flag_counts_and_an_ambiguous_type_by_none() {
    use content_reader_key_names::buildings::{Buildings, FactoryFlags};
    let flags = |military, naval, general, rocket| FactoryFlags { military_production: military, naval_production: naval, general_production: general, rocket_production: rocket };
    let mut buildings = Buildings::default();
    for (token, held) in [("arms_factory", flags(1, 0, 0, 0)), ("dockyard", flags(0, 1, 0, 0)), ("industrial_complex", flags(0, 0, 1, 0)),
        ("mod_foundry", flags(1, 0, 1, 0)), ("twice", flags(1, 0, 0, 0)), ("rocket_site", flags(0, 0, 0, 1))] {
        buildings.declaration_order.push(token.to_owned());
        buildings.factory_flags.insert(token.to_owned(), held);
    }
    buildings.ambiguous_factory_types.insert("twice".to_owned());
    let (groups, warnings) = factory_groups::groups(&buildings, |token| (token != "mod_foundry").then(|| format!("Name of {token}")));
    let named: Vec<(&str, Vec<&str>)> = groups.iter().map(|group| (group.key.as_str(), group.buildings.iter().map(|building| building.token.as_str()).collect())).collect();
    assert_eq!(named, [("military", vec!["arms_factory", "mod_foundry"]), ("naval", vec!["dockyard"]), ("civilian", vec!["industrial_complex", "mod_foundry"])]);
    assert_eq!(groups[0].buildings[0].name.as_deref(), Some("Name of arms_factory"));
    assert_eq!(groups[0].buildings[1].name, None, "a type the content does not name stays unnamed, never its token");
    assert!(warnings.iter().any(|warning| warning.starts_with("twice is declared more than once")));
    assert!(warnings.iter().any(|warning| warning.starts_with("rocket_site has a rocket production flag")));
    assert!(warnings.iter().any(|warning| warning.starts_with("mod_foundry: no name")));
}

#[test]
fn world_tension_stack_computes_exact_shares_folds_other_and_clips_future() {
    use std::collections::BTreeMap;
    use world_tension_stack::{StackInput, TensionValueMode, compute_stack};

    let tension = vec![(10, 19, 25_000), (20, 29, 100_000), (30, 39, 100_000)];
    let sum = vec![(10, 19, 2_500_000), (20, 29, 12_000_000), (30, 39, 20_000_000)];
    let mut slots = BTreeMap::new();
    slots.insert(1, vec![(10, 19, Some(1_500_000)), (20, 29, Some(6_000_000)), (30, 39, Some(10_000_000))]);
    slots.insert(2, vec![(10, 19, Some(1_000_000)), (20, 29, Some(4_000_000)), (30, 39, Some(6_000_000))]);
    slots.insert(3, vec![(20, 29, Some(2_000_000)), (30, 39, Some(4_000_000))]);
    let complete = [(10, 39)];
    let input = StackInput { tension: &tension, sum: &sum, slots: &slots, aggregates_complete: &complete, tension_missing: &[], aggregates_missing: &[] };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("JAP".to_owned(), false)),
        3 => Some(("ITA".to_owned(), false)),
        _ => None,
    };
    let no_change = |_: u32, _: i32| None;
    let out = compute_stack(&input, 10, 25, 2, TensionValueMode::Share, None, None, None, 200, tag_of, no_change);
    assert_eq!(out.end_hour, Some(25));
    assert_eq!(out.capped_end_tension, Some(100.0));
    assert_eq!(out.capped_end_tension_raw.as_deref(), Some("100000"));
    assert_eq!(out.end_sum_raw.as_deref(), Some("12000000"));
    assert!(!out.downsampled);
    assert_eq!(out.hours, [10, 20]);
    assert_eq!(out.total, [Some(25.0), Some(100.0)]);
    assert_eq!(out.total_raw, [Some("25000".into()), Some("100000".into())]);
    assert_eq!(out.sum_raw, [Some("2500000".into()), Some("12000000".into())]);
    assert_eq!(out.bands.len(), 2);
    assert_eq!(out.bands[0].tag.as_deref(), Some("GER"));
    assert_eq!(out.bands[1].tag.as_deref(), Some("JAP"));
    assert_eq!(out.bands[0].end_value, Some(50.0));
    assert_eq!(out.bands[0].end_raw.as_deref(), Some("6000000"));
    assert_eq!(out.bands[0].values_raw, [Some("1500000".into()), Some("6000000".into())]);
    assert!((out.bands[1].end_value.unwrap() - 100.0 / 3.0).abs() < 1e-9);
    assert_eq!(out.bands[1].end_raw.as_deref(), Some("4000000"));
    let other = out.other.unwrap();
    assert_eq!(other.count, 1);
    assert!((other.end_value.unwrap() - 100.0 / 6.0).abs() < 1e-9);
    assert_eq!(other.end_raw.as_deref(), Some("2000000"));
    assert_eq!(other.values_raw, [Some("0".into()), Some("2000000".into())]);
    assert_eq!(out.next_change_hour, Some(30));

    let rec = compute_stack(&input, 10, 25, 2, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(rec.total, [Some(25.0), Some(100.0)]);
    assert_eq!(rec.total_raw, [Some("25000".into()), Some("100000".into())]);
    assert_eq!(rec.capped_end_tension_raw.as_deref(), Some("100000"));
    assert_eq!(rec.bands[0].end_value, Some(60.0));
    assert_eq!(rec.bands[0].end_raw.as_deref(), Some("6000000"));
    assert_eq!(rec.bands[1].end_value, Some(40.0));
    assert_eq!(rec.bands[1].end_raw.as_deref(), Some("4000000"));
    let rec_other = rec.other.unwrap();
    assert_eq!(rec_other.end_value, Some(20.0));
    assert_eq!(rec_other.end_raw.as_deref(), Some("2000000"));
}

#[test]
fn world_tension_stack_preserves_negative_contributions_independent_coverage_and_gap_end_labels() {
    use dashboard_query::series::MissingSpan;
    use std::collections::BTreeMap;
    use world_tension_stack::{StackInput, TensionValueMode, compute_stack};

    let tension = vec![(10, 19, 6_500)];
    let sum = vec![(10, 19, 650_000)];
    let mut slots = BTreeMap::new();
    slots.insert(1, vec![(10, 19, Some(400_000)), (20, 29, Some(400_000))]);
    slots.insert(2, vec![(10, 19, Some(300_000)), (20, 29, None)]);
    slots.insert(7, vec![(10, 19, Some(-50_000))]);
    let complete = [(10, 19)];
    let missing = [MissingSpan { first_hour: 30, last_hour: 35, reason: "gap".into() }];
    let input = StackInput { tension: &tension, sum: &sum, slots: &slots, aggregates_complete: &complete, tension_missing: &missing, aggregates_missing: &missing };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("JAP".to_owned(), false)),
        7 => Some(("ITA".to_owned(), false)),
        _ => None,
    };

    let no_change = |_: u32, _: i32| None;
    let out = compute_stack(&input, 10, 19, 5, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(out.total, [Some(6.5)]);
    assert_eq!(out.bands.len(), 3);
    assert_eq!(out.bands[0].end_value, Some(4.0));
    assert_eq!(out.bands[1].end_value, Some(3.0));
    assert_eq!(out.bands[2].end_value, Some(-0.5));
    assert_eq!(out.bands[2].end_raw.as_deref(), Some("-50000"));
    assert!(out.other.is_none());

    let out2 = compute_stack(&input, 10, 19, 2, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(out2.total, [Some(6.5)]);
    assert_eq!(out2.bands.len(), 2);
    let other2 = out2.other.unwrap();
    assert_eq!(other2.count, 1);
    assert_eq!(other2.end_value, Some(-0.5));

    let share = compute_stack(&input, 10, 19, 2, TensionValueMode::Share, None, None, None, 200, tag_of, no_change);
    assert_eq!(share.total, [Some(6.5)]);
    assert!((share.bands[0].end_value.unwrap() - 4.0).abs() < 1e-9);
    assert!((share.bands[1].end_value.unwrap() - 3.0).abs() < 1e-9);
    assert!((share.other.unwrap().end_value.unwrap() - (-0.5)).abs() < 1e-9);

    let rec_25 = compute_stack(&input, 10, 25, 5, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(rec_25.end_hour, Some(25));
    assert_eq!(rec_25.capped_end_tension, None);
    assert_eq!(rec_25.total.last().copied().flatten(), None);
    assert_eq!(rec_25.bands[0].end_value, Some(4.0));
    assert_eq!(rec_25.bands[1].end_value, None);
    assert_eq!(rec_25.bands[1].end_raw, None);
    assert_eq!(rec_25.bands[2].end_value, None);

    let gap = compute_stack(&input, 10, 32, 2, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(gap.end_hour, None);
    assert_eq!(gap.capped_end_tension, None);
    assert_eq!(gap.total.last().copied().flatten(), None);
    assert!(gap.bands.iter().all(|b| b.end_value.is_none() && b.end_raw.is_none()));
    assert_eq!(gap.other.unwrap().end_value, None);
}

#[test]
fn world_tension_stack_states_the_recorded_world_scalar_and_each_missing_reason_never_a_sum_or_subtotal() {
    use dashboard_query::series::MissingSpan;
    use std::collections::{BTreeMap, HashSet};
    use world_tension_stack::{StackInput, TensionValueMode, compute_stack};

    let tension = vec![(10, 14, 100_000)];
    let sum = vec![(10, 19, 12_500_000)];
    let mut slots = BTreeMap::new();
    slots.insert(1, vec![(10, 19, Some(10_000_000))]);
    slots.insert(2, vec![(10, 19, Some(2_000_000))]);
    slots.insert(3, vec![(10, 14, Some(500_000))]);
    let complete = [(10, 14)];
    let tension_missing = [MissingSpan { first_hour: 15, last_hour: 19, reason: "World tension withheld".into() }];
    let aggregates_missing = [MissingSpan { first_hour: 15, last_hour: 19, reason: "Contributions truncated".into() }];
    let input = StackInput { tension: &tension, sum: &sum, slots: &slots, aggregates_complete: &complete, tension_missing: &tension_missing, aggregates_missing: &aggregates_missing };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("JAP".to_owned(), false)),
        3 => Some(("ITA".to_owned(), false)),
        _ => None,
    };
    let no_change = |_: u32, _: i32| None;

    let all = compute_stack(&input, 10, 12, 5, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(all.total, [Some(100.0)], "a recorded world tension of 100% is 100%, not the uncapped 125% sum");
    assert_eq!(all.total_raw, [Some("100000".into())]);
    assert_eq!(all.capped_end_tension_raw.as_deref(), Some("100000"));
    assert_eq!(all.end_missing, None);

    let japan: HashSet<&str> = ["JAP"].into_iter().collect();
    let one = compute_stack(&input, 10, 12, 5, TensionValueMode::Recorded, Some(&japan), None, None, 200, tag_of, no_change);
    assert_eq!(one.total, [Some(100.0)], "one selected contributor of 20% does not make the world 20%");
    assert_eq!(one.bands.len(), 1);
    assert_eq!(one.bands[0].end_raw.as_deref(), Some("2000000"));
    let other = one.other.expect("the selected country and the remaining contributions have separate bands");
    assert_eq!(other.count, 2);
    assert_eq!(other.end_raw.as_deref(), Some("10500000"));
    let held = StackInput { aggregates_complete: &[(10, 19)], ..input };
    let after = compute_stack(&held, 10, 21, 5, TensionValueMode::Recorded, Some(&japan), None, None, 200, tag_of, no_change);
    assert_eq!(after.other.unwrap().count, 1, "Other counts nonzero contributors at the witnessed ranking hour");

    let gap = compute_stack(&input, 10, 17, 5, TensionValueMode::Recorded, None, None, None, 200, tag_of, no_change);
    assert_eq!(gap.end_missing.as_deref(), Some("World tension withheld"), "the world end states why it is not known");
    let ita = gap.bands.iter().find(|band| band.slot == 3).unwrap();
    assert_eq!(ita.end_raw, None);
    assert_eq!(ita.end_missing.as_deref(), Some("Contributions truncated"), "an unread contribution states its own reason");
    let ger = gap.bands.iter().find(|band| band.slot == 1).unwrap();
    assert_eq!(ger.end_raw.as_deref(), Some("10000000"), "a witnessed contribution keeps its value beside an unread world scalar");
    assert_eq!(ger.end_missing, None);

    let share_gap = compute_stack(&input, 10, 17, 5, TensionValueMode::Share, None, None, None, 200, tag_of, no_change);
    let ger_share = share_gap.bands.iter().find(|band| band.slot == 1).unwrap();
    assert_eq!(ger_share.end_value, None);
    assert_eq!(ger_share.end_missing.as_deref(), Some("World tension withheld"), "a share of an unread world tension says why");
}

#[test]
fn downsampled_world_tension_stack_preserves_band_min_max_bounds() {
    use std::collections::BTreeMap;
    use world_tension_stack::{StackInput, TensionValueMode, compute_stack};

    let mut runs = Vec::new();
    let vals = [0_i64, 1_000_000, 4_000_000, 2_000_000, 2_500_000, 3_000_000, 3_500_000, 5_000_000];
    for (i, &v) in vals.iter().enumerate() {
        let h = (i as i32) * 10;
        runs.push((h, h + 9, Some(v)));
    }
    let sum: Vec<(i32, i32, i64)> = runs.iter().map(|&(a, b, v)| (a, b, v.unwrap())).collect();
    let mut slots = BTreeMap::new();
    slots.insert(1, runs);
    let complete = [(0, 79)];
    let input = StackInput { tension: &[], sum: &sum, slots: &slots, aggregates_complete: &complete, tension_missing: &[], aggregates_missing: &[] };
    let out = compute_stack(
        &input,
        0,
        79,
        5,
        TensionValueMode::Recorded,
        None,
        None,
        None,
        4,
        |slot, _| (slot == 1).then(|| ("GER".to_owned(), false)),
        |_, _| None,
    );
    assert!(out.downsampled);
    let band = &out.bands[0];
    assert_eq!(band.min_values.len(), band.values.len());
    assert_eq!(band.max_values.len(), band.values.len());
    assert!(
        band.max_values.contains(&Some(40.0)),
        "intermediate 40% spike must survive in max_values when bucketed: {:?}",
        (&out.hours, &band.values, &band.min_values, &band.max_values),
    );
    assert!(
        band.min_values.iter().zip(&band.max_values).any(|(lo, hi)| lo < hi),
        "at least one downsampled bucket carries distinct min/max interval bounds",
    );
    for (at, hour) in out.hours.iter().enumerate() {
        let recorded = vals[(*hour / 10) as usize];
        assert_eq!(band.values_raw[at], Some(recorded.to_string()), "a reduced bin's figure is the one recorded at its own hour {hour}, never a later sample's");
    }
    use dashboard_base::{Source, SourceKey, products::{LoadContext, Product}, test_fixtures};
    use replay_data::rows::{TableId, TableRows, ThreatCountryAggregateRow};
    use replay_frame::frame::TableView;
    use world_tension_stack::{WorldTensionStack, WorldTensionStackParams};

    let frames = (10..380).map(|hour| {
        let mut frame = test_fixtures::source(1, hour, None);
        let rows = vec![ThreatCountryAggregateRow { country_slot: 4, total_raw: i64::from(hour) }];
        *frame.tables[TableId::ThreatCountryAggregates.index()] = TableView::Complete(TableRows::ThreatCountryAggregates(rows));
        frame
    }).collect();
    let cancelled = false.into();
    let replay = test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "rolling-tension".into(), revision: "1".into() },
        "synthetic.battleplan".into(), std::sync::Arc::new(replay), &cancelled).unwrap();
    for (first, last) in [(10, 301), (11, 302), (15, 306), (89, 378), (10, 308), (10, 197)] {
        let out = WorldTensionStack::load(&source, &WorldTensionStackParams {
            branch_id: "1".into(), first_hour: first, last_hour: last, layers: 2, values: TensionValueMode::Recorded,
            countries: None, top: None, group: None, width: Some(120), include_stack: Some(true), include_events: Some(false),
        }, &LoadContext { content: None }, &cancelled).unwrap();
        assert!(out.downsampled);
        assert_eq!(out.hours[0], first);
        assert_eq!(out.hours.last(), Some(&last));
        for (index, &hour) in out.hours.iter().enumerate() {
            let through = out.hours.get(index + 1).map_or(last, |next| next - 1);
            assert_eq!(out.bands[0].values_raw[index], Some(hour.to_string()));
            assert_eq!(out.bands[0].min_values[index], Some(f64::from(hour) / 100000.0));
            assert_eq!(out.bands[0].max_values[index], Some(f64::from(through) / 100000.0));
        }
    }

}

#[test]
fn world_tension_stack_resolves_slot_identity_at_drawn_hour_and_bounds_holder_transitions() {
    use world_tension_stack::SlotNames;
    use dashboard_query::identity::{Holder, HolderRun, Holders};
    use std::collections::BTreeMap;
    use world_tension_stack::{StackInput, TensionValueMode, compute_stack};

    let holders = Holders::new(
        BTreeMap::from([
            ("GER".to_owned(), vec![HolderRun { first: 12, last: 25, holder: Holder::Found { slot: 1, id: 1 }, starts_segment: true }]),
            ("D01".to_owned(), vec![HolderRun { first: 28, last: 50, holder: Holder::Found { slot: 1, id: 2 }, starts_segment: true }]),
        ]),
        Vec::new(),
    );
    let mut slots = BTreeMap::new();
    slots.insert(1, vec![(10, 19, Some(400_000))]);
    let complete = [(10, 19)];
    let input = StackInput { tension: &[], sum: &[(10, 19, 400_000)], slots: &slots, aggregates_complete: &complete, tension_missing: &[], aggregates_missing: &[] };

    let names = SlotNames::new(&holders);
    assert_eq!(names.next_change(1, 20), Some(26));
    let out_20 = compute_stack(
        &input,
        10,
        20,
        5,
        TensionValueMode::Recorded,
        None,
        None,
        None,
        200,
        |slot, hour| names.slot(slot, hour).map(|(tag, rem)| (tag.to_owned(), rem)),
        |slot, hour| names.next_change(slot, hour),
    );
    assert_eq!(out_20.bands[0].tag.as_deref(), Some("GER"));
    assert_eq!(out_20.next_change_hour, Some(26), "holder transition at hour 26 must bound validity");

    let out_30 = compute_stack(
        &input,
        10,
        30,
        5,
        TensionValueMode::Recorded,
        None,
        None,
        None,
        200,
        |slot, hour| names.slot(slot, hour).map(|(tag, rem)| (tag.to_owned(), rem)),
        |slot, hour| names.next_change(slot, hour),
    );
    let history = serde_json::to_value(&out_30.bands[0]).unwrap();
    assert_eq!(history["holders"], serde_json::json!([
        { "tag": "GER", "firstHour": 12, "lastHour": 25, "remembered": false },
        { "tag": "GER", "firstHour": 26, "lastHour": 27, "remembered": true },
        { "tag": "D01", "firstHour": 28, "lastHour": 30, "remembered": false },
    ]));
    assert_eq!(out_30.bands[0].tag.as_deref(), Some("D01"), "slot identity resolves at drawn hour 30, not rank_hour 19");
}

#[test]
fn world_tension_top_matches_in_stack_lines_and_events_and_refuses_an_unknown_value() {
    use dashboard_base::{Source, SourceKey, products::{Product, LoadContext}, test_fixtures};
    use replay_data::rows::{CountryLinkRow, TableId, TableRows, ThreatCountryAggregateRow, ThreatSourceRow};
    use replay_frame::frame::TableView;
    use std::sync::{Arc, atomic::AtomicBool};
    use world_tension_stack::{WorldTensionStack, WorldTensionStackParams, TensionValueMode};

    let frames = (100..=106).map(|hour| {
        let mut frame = test_fixtures::source(1, hour, None);
        if let TableView::Complete(TableRows::CountryLinks(links)) = &mut *frame.tables[TableId::CountryLinks.index()] {
            links.push(CountryLinkRow { id: 9002, canonical_slot: 8 });
        }
        *frame.tables[TableId::ThreatSources.index()] = TableView::Complete(TableRows::ThreatSources(
            [9001, 9002].into_iter().enumerate().map(|(index, country)| ThreatSourceRow {
                ordinal: index as u32, country, date: 100, reason: b"fixture".to_vec(), final_raw: 4, ..Default::default()
            }).collect()
        ));
        *frame.tables[TableId::ThreatCountryAggregates.index()] = if matches!(hour, 102 | 104 | 105) {
            TableView::Unavailable { reason: if hour == 105 { "Fixture read interrupted" } else { "Fixture contributions unread" }.into() }
        } else if hour == 106 {
            TableView::Truncated { rows: TableRows::ThreatCountryAggregates(vec![ThreatCountryAggregateRow { country_slot: 8, total_raw: 6 }]), missing: 1 }
        } else {
            TableView::Complete(TableRows::ThreatCountryAggregates(
                [(7, 3), (8, 6)].into_iter().filter(|&(slot, _)| hour != 101 || slot != 8).map(|(country_slot, peak)| ThreatCountryAggregateRow {
                    country_slot, total_raw: match hour { 100 => peak, 103 if country_slot == 7 => 2, _ => 0 },
                }).collect()
            ))
        };
        frame
    }).collect();
    let cancelled = AtomicBool::new(false);
    let replay = test_fixtures::replay(frames).into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(SourceKey { source_id: "tension-top".into(), revision: "1".into() },
        "synthetic.battleplan".into(), Arc::new(replay), &cancelled).unwrap();
    for (stack, events, mode) in [(true, true, TensionValueMode::Share), (true, false, TensionValueMode::Recorded), (false, true, TensionValueMode::Share)] {
        let read = |last_hour, top| WorldTensionStack::load(&source, &WorldTensionStackParams {
            branch_id: "1".into(), first_hour: 100, last_hour, layers: 1, values: mode, countries: None,
            top, group: None, width: None, include_stack: Some(stack), include_events: Some(events),
        }, &LoadContext { content: None }, &cancelled).unwrap();
        let ranked = read(101, Some(1));
        assert_eq!(ranked.refused, None);
        if stack {
            assert_eq!(ranked.bands.iter().map(|b| (b.slot, &b.tag)).collect::<Vec<_>>(), [(8, &None)]);
        }
        if events {
            assert_eq!(ranked.events.iter().map(|e| e.slot).collect::<Vec<_>>(), [Some(8)]);
        }
        let unread = read(102, Some(1));
        assert_eq!(unread.refused.as_deref(), Some("Fixture contributions unread"));
        assert_eq!(unread.next_change_hour, Some(103));
        assert!(unread.bands.is_empty() && unread.events.is_empty());
        assert_eq!(read(104, Some(1)).next_change_hour, Some(105));
        assert_eq!(read(105, Some(1)).refused.as_deref(), Some("Fixture read interrupted"));
        if stack {
            let all = read(104, None);
            assert_eq!(all.bands[0].slot, 7, "the historical band remains the last witnessed leader through a gap");
            assert_eq!(all.bands[0].end_raw, None);
            let partial = read(106, None);
            assert_eq!(partial.bands[0].slot, 7, "a partial read cannot displace the last fully witnessed leader");
            assert_eq!(partial.bands[0].end_raw, None);
        }
    }
}

#[test]
fn world_tension_stack_applies_dated_group_membership_and_top_limit() {
    use crate::products::groups::MembershipRun;
    use dashboard_query::{identity::{Holder, HolderRun, Holders}, series::ranked::GroupMembership};
    use std::collections::BTreeMap;
    use world_tension_stack::{DatedGroupFilter, StackInput, TensionValueMode, compute_stack};

    let members = [
        MembershipRun { tag: "GER".into(), first_hour: 10, last_hour: 27 },
        MembershipRun { tag: "ITA".into(), first_hour: 25, last_hour: 40 },
    ];
    let holders = Holders::new([("GER", 1), ("ITA", 2)].map(|(tag, slot)| {
        (tag.to_owned(), vec![HolderRun { first: 10, last: 40, holder: Holder::Found { slot, id: slot }, starts_segment: true }])
    }).into(), Vec::new());
    let mut slots = BTreeMap::new();
    slots.insert(1, vec![(10, 40, Some(500_000))]);
    slots.insert(2, vec![(10, 40, Some(300_000))]);
    slots.insert(3, vec![(10, 40, Some(200_000))]);
    let complete = [(10, 40)];
    let input = StackInput {
        tension: &[],
        sum: &[(10, 40, 1_000_000)],
        slots: &slots,
        aggregates_complete: &complete,
        tension_missing: &[],
        aggregates_missing: &[],
    };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("ITA".to_owned(), false)),
        3 => Some(("JAP".to_owned(), false)),
        _ => None,
    };
    let gf = DatedGroupFilter::new(&members, GroupMembership::Hour, &holders);
    let out = compute_stack(&input, 10, 30, 5, TensionValueMode::Recorded, None, Some(&gf), None, 200, tag_of, |_, _| None);
    assert_eq!(out.bands.len(), 2);
    assert_eq!(out.hours, [10, 25, 28]);
    let former = out.bands.iter().find(|b| b.tag.as_deref() == Some("GER")).unwrap();
    assert_eq!(former.values, [Some(5.0), Some(5.0), None]);
    let ita = out.bands.iter().find(|b| b.tag.as_deref() == Some("ITA")).unwrap();
    assert_eq!(ita.values, [None, Some(3.0), Some(3.0)]);
    assert_eq!(out.total, [None, None, None], "the world line is the recorded world scalar, never the members' subtotal");

    let top_out = compute_stack(&input, 10, 30, 5, TensionValueMode::Recorded, None, None, Some(2), 200, tag_of, |_, _| None);
    assert_eq!(top_out.bands.len(), 2);
    assert!(top_out.other.is_none(), "Top-N selection omits Other band");

    let partial_slots = BTreeMap::from([
        (1, vec![(10, 27, Some(950_000))]), (2, vec![(25, 40, Some(900_000))]),
    ]);
    let partial = StackInput { slots: &partial_slots, aggregates_complete: &[(10, 23)], ..input };
    let selection = ["GER", "ITA"].into_iter().collect();
    let named = compute_stack(&partial, 10, 30, 1, TensionValueMode::Recorded, Some(&selection), None, None, 200, tag_of, |_, _| None);
    assert_eq!(named.bands[0].slot, 1, "the selected leader at the last complete read keeps its band");
    for (countries, group) in [(Some(&selection), None), (None, Some(&gf)), (None, None)] {
        let full = compute_stack(&partial, 10, 30, 5, TensionValueMode::Recorded, countries, group, None, 200, tag_of, |_, _| None);
        assert_eq!(full.bands.iter().map(|band| band.slot).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(full.bands[1].end_value, Some(9.0));
    }

    use crate::products::group_members::GroupsAt;
    use dashboard_base::products::at_hour::{At, AtHour};
    use crate::products::groups::{CountryGroupReply, FactionGroup, UnobservedHours};
    use dashboard_query::series::ranked::{GroupSelection, GroupKind};
    use replay_data::rows::{TableId, TableRows, ThreatCountryAggregateRow};
    use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};

    let groups_reply = CountryGroupReply {
        branch_id: "1".into(),
        humans: Vec::new(),
        factions: vec![FactionGroup {
            id: "42".into(),
            name: Some("Example faction".into()),
            name_reason: None,
            names: Vec::new(),
            recorded_name: "Example faction".into(),
            first_hour: 10,
            last_hour: 40,
            members: members.to_vec(),
            leaders: Vec::new(),
            incomplete: Vec::new(),
        }],
        wars: Vec::new(),
        war_starts: Vec::new(),
        capitulations: Vec::new(),
        identity_changes: Vec::new(),
        country_identities: Vec::new(),
        unobserved: UnobservedHours::default(),
        warnings: Vec::new(),
    };
    let sel = GroupSelection { kind: GroupKind::Faction, id: Some("42".into()), tag: None, membership: GroupMembership::Hour };
    let groups = GroupsAt::new(groups_reply.into(), 10);
    let params = world_tension::WorldTensionParams { branch_id: "1".into(), countries: Some(vec![2]), values: Some(TensionValueMode::Recorded), group: Some(sel) };
    for (hour, raw, expected) in [(24, Some(300_000), None), (24, None, None), (25, Some(300_000), Some("300000")), (25, None, Some("0"))] {
        let mut tables = std::array::from_fn(|_| not_selected());
        let rows = raw.into_iter().map(|total_raw| ThreatCountryAggregateRow { country_slot: 2, total_raw }).collect();
        tables[TableId::ThreatCountryAggregates.index()] = TableView::Complete(TableRows::ThreatCountryAggregates(rows)).into();
        let frame = DecodedFrame { branch: 1, hour, tables, metrics: Metrics::default() };
        let at = At { hour, holders: &holders, owner: None, context: &groups, cancelled: &false.into(), live: &Default::default() };
        let reply = world_tension::WorldTension::reply(&frame, &params, &at).unwrap();
        assert_eq!(reply.contributions[0].reading.as_ref().unwrap().as_deref().map_err(String::as_str),
            expected.ok_or("Not a member of the selected group at this hour."));
    }
}

#[test]
fn factory_mix_computes_rust_shares_keeps_witnessed_parts_and_refuses_unreconciled_shares() {
    use super::factory_mix::row_mix;
    use replay_data::rows::CountryFactoriesRow;

    let base = CountryFactoriesRow {
        country: 1,
        military_total_raw: 40 * 100_000,
        military_damaged_raw: 10 * 100_000,
        naval_total_raw: 20 * 100_000,
        civilian_total_raw: 40 * 100_000,
        ..Default::default()
    };
    let (total, values, shares, total_missing, parts_missing) = row_mix(&base, "built", &[0, 1, 2]);
    assert_eq!(total, Some(100));
    assert_eq!(values, vec![Some(30), Some(20), Some(40), Some(10), Some(0), Some(0)]);
    assert_eq!(shares, vec![Some(0.3), Some(0.2), Some(0.4), Some(0.1), Some(0.0), Some(0.0)]);
    assert!(total_missing.is_none());
    assert!(parts_missing.is_none());

    let mut partial = base.clone();
    partial.military_damaged_raw = 50 * 100_000;
    let (total_p, values_p, shares_p, total_missing_p, parts_missing_p) = row_mix(&partial, "built", &[0, 1, 2]);
    assert_eq!(total_p, Some(100));
    assert_eq!(values_p, vec![None, Some(20), Some(40), None, Some(0), Some(0)], "witnessed naval and civilian counts stay");
    assert_eq!(shares_p, vec![None, Some(0.2), Some(0.4), None, Some(0.0), Some(0.0)], "witnessed shares stay beside unreconciled military");
    assert!(total_missing_p.is_none());
    assert!(parts_missing_p.is_some());

    let mut neg_free = base;
    neg_free.consumer_goods = 50;
    let (total_n, values_n, shares_n, total_missing_n, parts_missing_n) = row_mix(&neg_free, "free", &[0, 1, 2]);
    assert_eq!(total_n, Some(40));
    assert_eq!(values_n, vec![Some(30), Some(20), Some(-10)], "witnessed counts stay even when civilian free is negative");
    assert_eq!(shares_n, vec![None, None, None], "shares refused when a part is negative");
    assert!(total_missing_n.is_none());
    assert!(parts_missing_n.is_some());
}
