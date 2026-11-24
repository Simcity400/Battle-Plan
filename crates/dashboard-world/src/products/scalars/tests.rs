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
        Step::Stretch { first: 10, last: 19, listed: vec![(a.clone(), 500, Some(1001))] },
        row(12, ThreatEventRow::LISTED, &b, 300),
        row(13, ThreatEventRow::LISTED, &twin, 500),
        row(14, ThreatEventRow::CHANGED, &a, 700),
        row(15, ThreatEventRow::DROPPED, &b, 300),
        Step::Stretch { first: 30, last: 39, listed: vec![(a.clone(), 700, Some(1001))] },
        row(31, ThreatEventRow::DROPPED, &source(9, "never listed", 0), 0),
    ]);
    assert_eq!(unmatched, 1);
    assert_eq!(
        events,
        [
            Joined { identity: a, slot: Some(1001), added: vec![(10, 500), (14, 700)], first_read: 10, last_read: None },
            Joined { identity: b, slot: Some(1002), added: vec![(12, 300)], first_read: 12, last_read: Some(14) },
            Joined { identity: twin, slot: Some(1001), added: vec![(13, 500)], first_read: 13, last_read: Some(19) },
        ]
    );
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
    nuclear_arsenal::rerank(&mut rank, &changed, 1, "GER").unwrap();
    assert_eq!(rank, serde_json::to_value(nuclear_arsenal::countries(&changed, tag).0).unwrap());
    let (sentinel, _, _) = nuclear_arsenal::countries(&[NukeRow { country: 1, nuke_type: 0, amount_raw: i64::MAX, ready: 0 }, nuke(1, 1, 1)], tag);
    assert_eq!(sentinel[0].total, 0xa7c5 + 1, "the top bar adds the game's sentinel as it shows it");
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
    let input = StackInput { tension: &tension, sum: &sum, slots: &slots, aggregates_complete: &complete, missing: &[] };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("JAP".to_owned(), false)),
        3 => Some(("ITA".to_owned(), false)),
        _ => None,
    };
    let (end_hour, capped, hours, total, bands, other, next_change) =
        compute_stack(&input, 10, 25, 2, TensionValueMode::Share, None, 200, tag_of);
    assert_eq!(end_hour, Some(25));
    assert_eq!(capped, Some(100.0));
    assert_eq!(hours, [10, 20]);
    assert_eq!(total, [Some(25.0), Some(100.0)]);
    assert_eq!(bands.len(), 2);
    assert_eq!(bands[0].tag.as_deref(), Some("GER"));
    assert_eq!(bands[1].tag.as_deref(), Some("JAP"));
    assert_eq!(bands[0].end_value, Some(50.0));
    assert!((bands[1].end_value.unwrap() - 100.0 / 3.0).abs() < 1e-9);
    let other = other.unwrap();
    assert_eq!(other.count, 1);
    assert!((other.end_value.unwrap() - 100.0 / 6.0).abs() < 1e-9);
    assert_eq!(next_change, Some(30));

    let (_, _, _, rec_total, rec_bands, rec_other, _) =
        compute_stack(&input, 10, 25, 2, TensionValueMode::Recorded, None, 200, tag_of);
    assert_eq!(rec_total, [Some(25.0), Some(120.0)]);
    assert_eq!(rec_bands[0].end_value, Some(60.0));
    assert_eq!(rec_bands[1].end_value, Some(40.0));
    assert_eq!(rec_other.unwrap().end_value, Some(20.0));
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
    let input = StackInput { tension: &tension, sum: &sum, slots: &slots, aggregates_complete: &complete, missing: &missing };
    let tag_of = |slot: u32, _: i32| match slot {
        1 => Some(("GER".to_owned(), false)),
        2 => Some(("JAP".to_owned(), false)),
        7 => Some(("ITA".to_owned(), false)),
        _ => None,
    };

    let (_, _, _, total, bands, other, _) = compute_stack(&input, 10, 19, 5, TensionValueMode::Recorded, None, 200, tag_of);
    assert_eq!(total, [Some(6.5)]);
    assert_eq!(bands.len(), 3);
    assert_eq!(bands[0].end_value, Some(4.0));
    assert_eq!(bands[1].end_value, Some(3.0));
    assert_eq!(bands[2].end_value, Some(-0.5));
    assert_eq!(bands[2].end_raw.as_deref(), Some("-50000"));
    assert!(other.is_none());

    let (_, _, _, total2, bands2, other2, _) = compute_stack(&input, 10, 19, 2, TensionValueMode::Recorded, None, 200, tag_of);
    assert_eq!(total2, [Some(6.5)]);
    assert_eq!(bands2.len(), 2);
    let other2 = other2.unwrap();
    assert_eq!(other2.count, 1);
    assert_eq!(other2.end_value, Some(-0.5));

    let (_, _, _, share_total, share_bands, share_other, _) = compute_stack(&input, 10, 19, 2, TensionValueMode::Share, None, 200, tag_of);
    assert_eq!(share_total, [Some(6.5)]);
    assert!((share_bands[0].end_value.unwrap() - 4.0).abs() < 1e-9);
    assert!((share_bands[1].end_value.unwrap() - 3.0).abs() < 1e-9);
    assert!((share_other.unwrap().end_value.unwrap() - (-0.5)).abs() < 1e-9);

    let (end_at_25, capped_at_25, _, rec_total_25, rec_bands_25, _, _) =
        compute_stack(&input, 10, 25, 5, TensionValueMode::Recorded, None, 200, tag_of);
    assert_eq!(end_at_25, Some(25));
    assert_eq!(capped_at_25, None);
    assert_eq!(rec_total_25.last().copied().flatten(), None);
    assert_eq!(rec_bands_25[0].end_value, Some(4.0));
    assert_eq!(rec_bands_25[1].end_value, None);
    assert_eq!(rec_bands_25[1].end_raw, None);
    assert_eq!(rec_bands_25[2].end_value, None);

    let (end_gap, capped_gap, _, gap_total, gap_bands, gap_other, _) =
        compute_stack(&input, 10, 32, 2, TensionValueMode::Recorded, None, 200, tag_of);
    assert_eq!(end_gap, None);
    assert_eq!(capped_gap, None);
    assert_eq!(gap_total.last().copied().flatten(), None);
    assert!(gap_bands.iter().all(|b| b.end_value.is_none() && b.end_raw.is_none()));
    assert_eq!(gap_other.unwrap().end_value, None);
}