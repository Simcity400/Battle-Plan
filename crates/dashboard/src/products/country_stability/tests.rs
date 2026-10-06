use crate::test_fixtures::{CAMPAIGN, records, stability_frame, stability_inputs};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn stability_formula_follows_the_engine_getters() {
    use crate::products::country_stability::formula::*;
    // Q15 conversions: rounding half up on the fraction, truncation back.
    assert_eq!(q15_to_fixed5(32_768), 100_000);
    assert_eq!(q15_to_fixed5(16_384), 50_000);
    assert_eq!(q15_to_fixed5(-3277), -10_001);
    // Whole parts beyond the engine's bound saturate.
    assert_eq!(q15_to_fixed5(0x36f9_bfb2 << 15), 0x53e2_d61f_d320);
    assert_eq!(q15_to_fixed5(-0x36f9_bfb2 << 15), -0x53e2_d61f_d321);
    assert_eq!(fixed5_to_q15(-20_000), -6553);
    assert_eq!(fixed5_to_q15(100_000), 32_768);
    // Popularity: two truncating divisions, toward zero for a negative factor.
    assert_eq!(popularity_term(15_000, 4_500_000), 6_750);
    assert_eq!(popularity_term(15_000, 3_333_333), 4_999);
    assert_eq!(popularity_term(-15_000, 3_333_333), -4_999);
    // At an offensive war: base, war term, popularity, propaganda, factor, coast.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.offensive_war = 1;
    let war = stability(&row);
    assert_eq!(
        war.unclamped,
        Some(50_000 - 25_000 + 6_750 - 10_001 + 30_000 + 2_500)
    );
    assert_eq!(war.value, war.unclamped);
    // An offensive war without IsAtWar(mode 1) adds nothing, as at peace.
    row.at_war = 0;
    assert_eq!(stability(&row).value, Some(79_249));
    // Clamped to the defines.
    row.stability_factor = Some(90_000);
    let clamped = stability(&row);
    assert_eq!(
        (clamped.unclamped, clamped.value),
        (Some(139_249), Some(100_000))
    );
    row.stability_raw = -500_000;
    assert_eq!(stability(&row).value, Some(0));
    // An unregistered key needed at war leaves the value unknown.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.defensive_war = 1;
    row.defensive_war_stability_factor = None;
    assert_eq!(stability(&row).value, None);
    // War support: war define, tension, interception scaled and clamped.
    let mut row = stability_inputs();
    row.at_war = 1;
    row.offensive_war = 1;
    assert_eq!(interception_term(&row), 4_999);
    assert_eq!(
        war_support(&row).value,
        Some(40_000 - 20_000 + 20_000 + 4_999)
    );
    row.interception_sum_raw = -10_000_000;
    assert_eq!(interception_term(&row), -30_000);
    // Maximum command power: (base + flat) * (1 + mult), with the modifier
    // getter values and the allocated power from CountryCommandPower.
    assert_eq!(
        max_command_power(&row, Some(&launch(0))).value,
        Some(9_900_000)
    );
    // The game shows and clamps to the maximum minus the allocated command
    // power, which can be negative.
    assert_eq!(
        max_command_power(&row, Some(&launch(400_000))).value,
        Some(9_500_000)
    );
    assert_eq!(
        max_command_power(&row, Some(&launch(-10_600_000))).value,
        Some(20_500_000)
    );
    // No CountryCommandPower row: unknown, never a zero allocation.
    assert_eq!(max_command_power(&row, None).value, None);
    let mut unregistered = launch(0);
    unregistered.max_command_power_factor = None;
    assert_eq!(max_command_power(&row, Some(&unregistered)).value, None);
}

/// The country's CountryCommandPower row: the capacity modifiers' getter
/// values and `allocated` command power.
fn launch(allocated: i64) -> replay::rows::CountryCommandPowerRow {
    replay::rows::CountryCommandPowerRow {
        canonical_slot: 7,
        allocated_command_power_raw: allocated,
        max_command_power: Some(1_000_000),
        max_command_power_factor: Some(10_000),
        ..Default::default()
    }
}

/// The maximum command power takes the allocated power from
/// CountryCommandPower, joined through CountryLinks by canonical slot at the
/// same frame; an hour without that row has no value, never a zero one.
#[test]
fn stability_product_takes_the_allocated_command_power_from_raid_country_inputs() {
    use crate::products::country_stability::scan;
    let with_allocation = |hour, allocated: Option<i64>| {
        let mut frame = stability_frame(hour, stability_inputs(), hour, vec![]);
        *frame.tables[TableId::CountryCommandPower.index()] = TableView::Complete(
            TableRows::CountryCommandPower(allocated.map(launch).into_iter().collect()),
        );
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                with_allocation(100, Some(400_000)),
                with_allocation(101, None),
            ],
        )],
    )
    .unwrap();
    let (result, _) = scan(&replay, 1, &["GER"], Some(101), &AtomicBool::new(false)).unwrap();
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .max_command_power
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
        .collect();
    assert_eq!(runs, [(100, 100, "9500000")]);
    let breakdown = germany.breakdown.as_ref().unwrap();
    assert_eq!(breakdown.max_command_power.value_raw, None);
    let allocated = breakdown
        .max_command_power
        .terms
        .iter()
        .find(|term| term.term == "allocatedCommandPower")
        .unwrap();
    assert_eq!(allocated.value_raw, None);
}

/// A maximum computed from a carried stability read and fresh command power
/// names only the stability read: CountryCommandPower is read every hour.
#[test]
fn stability_product_names_only_the_stability_read_of_the_maximum() {
    use crate::products::country_stability::scan;
    let frame = |hour, stability_read, allocated| {
        let mut frame = stability_frame(hour, stability_inputs(), stability_read, vec![]);
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(vec![launch(allocated)]));
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                frame(100, 100, 400_000),
                // Stability carried from 100; the allocation read fresh.
                frame(101, 100, 500_000),
                frame(102, 102, 500_000),
            ],
        )],
    )
    .unwrap();
    let (result, _) = scan(&replay, 1, &["GER"], None, &AtomicBool::new(false)).unwrap();
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .max_command_power
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
        .collect();
    assert_eq!(runs, [(100, 100, "9500000"), (101, 102, "9400000")]);
    let reads: Vec<_> = germany
        .carried
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.read_hour))
        .collect();
    assert_eq!(reads, [(101, 101, 100)]);
}

/// The stability product's bytes are the same as before the command power
/// values moved to CountryCommandPower: the maximum command power breakdown
/// and the modifier totals, built from the same values the old way (all on
/// the stability row) and the new way, serialise identically.
#[test]
fn stability_output_is_byte_identical_with_the_values_on_country_command_power() {
    use crate::products::country_stability::{
        breakdown, formula, modifiers, modifiers_with_totals,
    };
    // The old formula, over values the stability row used to hold.
    fn old_max_command_power(
        base: i64,
        max: Option<i64>,
        mult: Option<i64>,
        allocated: i64,
    ) -> formula::Terms {
        let value = (|| {
            let base = base.wrapping_add(max?);
            Some((mult?.wrapping_add(100_000).wrapping_mul(base) / 100_000).wrapping_sub(allocated))
        })();
        formula::Terms {
            terms: vec![
                ("baseMaxCommandPower", Some(base)),
                ("maxCommandPower", max),
                ("maxCommandPowerMult", mult),
                ("allocatedCommandPower", Some(allocated)),
            ],
            unclamped: value,
            value,
        }
    }
    let row = stability_inputs();
    for (max, mult, allocated) in [
        (Some(1_000_000), Some(10_000), 400_000),
        (Some(0), Some(0), -10_600_000),
        (None, Some(10_000), 0),
        (Some(5), None, 7),
    ] {
        let launch = replay::rows::CountryCommandPowerRow {
            canonical_slot: 7,
            allocated_command_power_raw: allocated,
            max_command_power: max,
            max_command_power_factor: mult,
            ..Default::default()
        };
        let old = breakdown(old_max_command_power(
            row.base_max_command_power,
            max,
            mult,
            allocated,
        ));
        let new = breakdown(formula::max_command_power(&row, Some(&launch)));
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
        // The old totals: all ten on the stability row.
        let stability = row.totals();
        let old_totals: [Option<i64>; 10] = std::array::from_fn(|index| match index {
            8 => max,
            9 => mult,
            index => stability[index],
        });
        let mut refused = crate::catalogue::Refusals::default();
        let old =
            modifiers_with_totals(&row, old_totals, None, |_| Some(true), "FRA", &mut refused);
        let new = modifiers(
            &row,
            Some(&launch),
            None,
            |_| Some(true),
            "FRA",
            &mut refused,
        );
        assert_eq!(
            serde_json::to_vec(&old).unwrap(),
            serde_json::to_vec(&new).unwrap()
        );
    }
}

#[test]
fn stability_product_runs_carry_reads_and_break_down_one_hour() {
    use crate::products::country_stability::scan;
    use replay::rows::CountryStabilityLineRow as Line;
    let line = |line, kind, identity: &[u8], value, text: &[u8]| Line {
        country: 9001,
        line,
        key: 0x8d,
        kind,
        identity: identity.to_vec(),
        value,
        text: text.to_vec(),
        ..Default::default()
    };
    let lines = vec![
        line(0, Line::IDEA, b"idea_a", 10_000, b""),
        line(2, Line::UNPAIRED, b"", 15_000, b"Decision: X"),
    ];
    let mut changed = stability_inputs();
    changed.stability_raw = 60_000;
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(100, stability_inputs(), 100, lines.clone()),
                // Carried from the hour-100 read.
                stability_frame(101, stability_inputs(), 100, lines),
                stability_frame(102, changed, 102, vec![]),
                // Hour 103 is missing: the next hour starts a new segment.
                stability_frame(104, stability_inputs(), 104, vec![]),
            ],
        )],
    )
    .unwrap();
    let (result, warnings) = scan(
        &replay,
        1,
        &["GER", "ITA"],
        Some(101),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let germany = &result["GER"];
    let runs: Vec<_> = germany
        .stability
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.value_raw.as_str(),
                run.starts_segment,
            )
        })
        .collect();
    assert_eq!(
        runs,
        [
            (100, 101, "79249", true),
            (102, 102, "89249", false),
            (104, 104, "79249", true)
        ]
    );
    let carried: Vec<_> = germany
        .carried
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.read_hour))
        .collect();
    // Only hour 101 carries an earlier (hour-100) read; the others were read then.
    assert_eq!(carried, [(101, 101, 100)]);
    assert!(germany.recorded);
    let breakdown = germany.breakdown.as_ref().unwrap();
    assert_eq!((breakdown.hour, breakdown.read_hour), (101, 100));
    assert_eq!(breakdown.language, "l_english");
    assert_eq!(breakdown.stability.value_raw.as_deref(), Some("79249"));
    assert!(breakdown.lines_available);
    let factor = &breakdown.modifiers[0];
    assert_eq!(factor.key, "stability_factor");
    assert_eq!(factor.good, Some(true));
    assert_eq!(factor.lines.len(), 2);
    assert_eq!(factor.lines[0].identity, "idea_a");
    assert_eq!(factor.lines[1].kind, "unpaired");
    assert_eq!(factor.lines[1].text.as_deref(), Some("Decision: X"));
    // 30000 recorded, 25000 in lines: the rest is shown, not hidden.
    assert_eq!(factor.remainder_raw.as_deref(), Some("5000"));
    assert!(result["ITA"].stability.is_empty());
    // A breakdown at an hour whose lines came from another read says so.
    let (result, warnings) =
        scan(&replay, 1, &["GER"], Some(102), &AtomicBool::new(false)).unwrap();
    let breakdown = result["GER"].breakdown.as_ref().unwrap();
    assert!(breakdown.lines_available && breakdown.modifiers[0].lines.is_empty());
    assert!(warnings.is_empty());
}

#[test]
fn a_stability_breakdown_needs_the_same_country_identity_as_its_history() {
    use crate::products::{
        LoadContext, Product,
        country_stability::{CountryStability, CountryStabilityParams},
    };
    let mut unlinked = stability_frame(102, stability_inputs(), 102, vec![]);
    *unlinked.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![]));
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    stability_frame(100, stability_inputs(), 100, vec![]),
                    stability_frame(101, stability_inputs(), 100, vec![]),
                    unlinked,
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let load_with = |hour, breakdown_only| {
        let params = CountryStabilityParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            hour: Some(hour),
            breakdown_only,
        };
        CountryStability::load(
            &source,
            &params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let load = |hour| load_with(hour, false);
    let mut sampled = load(100);
    assert!(sampled.series[0].breakdown.is_some());
    // A carried read has the same reply except for its displayed hour.
    sampled.series[0].breakdown.as_mut().unwrap().hour = 101;
    assert_eq!(
        serde_json::to_vec(&load(101)).unwrap(),
        serde_json::to_vec(&sampled).unwrap()
    );
    // A breakdown alone leaves the histories out.
    let alone = load_with(100, true);
    assert!(alone.series[0].breakdown.is_some());
    assert!(alone.series[0].stability.is_empty() && !alone.series[0].recorded);
    assert!(alone.warnings.is_empty(), "{:?}", alone.warnings);
    // CountryLinks no longer maps GER: the history leaves the hour out, and
    // so does the breakdown.
    assert!(load(102).series[0].breakdown.is_none());
    let cached = super::histories(&source, 1, &["GER".into()], &AtomicBool::new(false)).unwrap();
    // Naming an hour-specific breakdown never mutates the shared history.
    assert!(cached["GER"].breakdown.is_none());
}

#[test]
fn stability_histories_keep_large_sets_shared_after_a_new_widget_tag() {
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![stability_frame(100, stability_inputs(), 100, vec![])],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability-cache".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut tags: Vec<String> = (0..4999).map(|index| format!("C{index}")).collect();
    tags.push("GER".into());
    let before = super::histories(&source, 1, &tags, &AtomicBool::new(false)).unwrap();
    assert_eq!(before.len(), 5000);
    assert!(before["GER"].recorded);
    let small = super::histories(
        &source,
        1,
        &["new-widget-tag".into()],
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!small["new-widget-tag"].recorded);
    // A cancelled token would reject any attempted scan. Cached histories need
    // no new scan, and retain the exact same allocation for every country.
    let after = super::histories(&source, 1, &tags, &AtomicBool::new(true)).unwrap();
    for tag in &tags {
        assert!(Arc::ptr_eq(&before[tag], &after[tag]), "{tag}");
    }
    let state = source.state::<super::Cache>();
    let cache = state.lock().unwrap();
    assert_eq!(cache.entries[&1].len(), 5001);
}

#[test]
fn stability_breakdown_index_preserves_first_tag_match_and_link_validation() {
    use crate::products::{LoadContext, Product};
    let mut frame = stability_frame(100, stability_inputs(), 100, vec![]);
    let mut countries = frame
        .complete::<replay::rows::tables::Countries>()
        .unwrap()
        .to_vec();
    // The first GER row has no link. A later valid row with the same tag cannot
    // supply its identity.
    countries.insert(
        0,
        replay::rows::CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..Default::default()
        },
    );
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability-breakdown-index".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = super::CountryStability::load(
        &source,
        &super::CountryStabilityParams {
            branch_id: "1".into(),
            tags: vec!["GER".into()],
            hour: Some(100),
            breakdown_only: true,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(reply.series[0].breakdown.is_none());
    assert!(reply.warnings.is_empty());
}

#[test]
fn stability_full_selection_answers_five_thousand_countries_on_each_branch() {
    use super::{CountryStability, CountryStabilityParams};
    use crate::products::{LoadContext, Product};
    use crate::test_fixtures::{scale_frame, scale_tag};
    let frame = |branch| {
        let mut frame = scale_frame(branch, 100, 5_000);
        *frame.tables[TableId::CountryStabilityInputs.index()] =
            TableView::Complete(TableRows::CountryStabilityInputs(
                (0..5_000)
                    .map(|index| replay::rows::CountryStabilityInputRow {
                        country: 100_000 + index,
                        stability_raw: 50_000 + i64::from(index),
                        ..stability_inputs()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::CountryCommandPower.index()] =
            TableView::Complete(TableRows::CountryCommandPower(
                (0..5_000)
                    .map(|index| replay::rows::CountryCommandPowerRow {
                        canonical_slot: index,
                        ..launch(400_000)
                    })
                    .collect(),
            ));
        *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
            (0..5_000)
                .map(|index| replay::rows::SliceReadRow {
                    table: TableId::CountryStabilityInputs.index() as u32,
                    owner: 100_000 + index,
                    sample_hour: 100,
                })
                .collect(),
        ));
        frame
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![frame(1)]), records(2, vec![frame(2)])],
        )
        .unwrap(),
    );
    let cancelled = AtomicBool::new(false);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability-scale".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &cancelled,
    )
    .unwrap();
    let tags: Vec<String> = (0..5_000)
        .map(|index| String::from_utf8(scale_tag(index)).unwrap())
        .collect();
    let load = |branch: u64, tags: Vec<String>| {
        let params = CountryStabilityParams {
            branch_id: branch.to_string(),
            tags,
            hour: None,
            breakdown_only: false,
        };
        CountryStability::validate(&params).unwrap();
        CountryStability::load(&source, &params, &LoadContext { content: None }, &cancelled)
            .unwrap()
    };
    let reply = load(2, tags.clone());
    assert_eq!(reply.series.len(), tags.len());
    for (index, series) in reply.series.iter().enumerate() {
        assert_eq!(series.tag, tags[index]);
        assert!(series.recorded);
        assert_eq!(series.stability[0].value_raw, (79_249 + index).to_string());
        assert_eq!(series.stability[0].first_hour, 100);
    }
    let restored = load(1, tags);
    assert_eq!(restored.series, reply.series);
}

#[test]
fn stability_carried_reads_have_no_entry_budget() {
    let mut carried = Vec::new();
    for hour in 1..=400_001 {
        super::note_carried(&mut carried, hour, hour - 1);
    }
    assert_eq!(carried.len(), 400_001);
    assert_eq!(carried.last().unwrap().read_hour, 400_000);
}

#[test]
fn stability_oversized_breakdown_refuses_the_complete_reply_by_name() {
    use super::{CountryStability, CountryStabilityParams};
    use crate::products::{LoadContext, MAX_PRODUCT_REPLY_BYTES, Product};
    let line = replay::rows::CountryStabilityLineRow {
        country: 9001,
        key: 0x8d,
        kind: replay::rows::CountryStabilityLineRow::UNPAIRED,
        text: vec![b'x'; MAX_PRODUCT_REPLY_BYTES],
        ..Default::default()
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![stability_frame(100, stability_inputs(), 100, vec![line])],
            )],
        )
        .unwrap(),
    );
    let cancelled = AtomicBool::new(false);
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "stability-large-breakdown".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &cancelled,
    )
    .unwrap();
    let params = CountryStabilityParams {
        branch_id: "1".into(),
        tags: vec!["GER".into()],
        hour: Some(100),
        breakdown_only: true,
    };
    let error =
        CountryStability::load(&source, &params, &LoadContext { content: None }, &cancelled)
            .unwrap_err();
    assert!(
        error.to_string().starts_with("StabilityReplyTooLarge:"),
        "{error}"
    );
}
