use super::*;
use crate::prepared::series::{Accumulator, Reader, tests::Fixture};
use crate::products::country_history::{CountryIdentityRun, RecordedCountry};

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan {
        first_hour,
        last_hour,
    }
}

#[test]
fn faction_prepared_dense_empty_roster_zero_does_not_require_country_reads() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut values = Reader::new(&fixture.store, &fixture.key, "no-values".into(), &flag).unwrap();
    let mut present =
        Reader::new(&fixture.store, &fixture.key, "no-presence".into(), &flag).unwrap();
    let mut rosters = Reader::new(
        &fixture.store,
        &fixture.key,
        "no-country-read".into(),
        &flag,
    )
    .unwrap();
    let native = NativeMembers::new(
        &History::default(),
        &RecordedHistories::default(),
        FactionMembership::Hour,
        &flag,
    )
    .unwrap();
    let result = dense(
        &mut values,
        &BTreeMap::new(),
        &mut present,
        &BTreeMap::new(),
        &mut rosters,
        &native,
        &BTreeMap::new(),
        &[span(0, 99)],
        &[],
        (0.0, 99.0, 2),
        &flag,
    )
    .unwrap();
    assert!(result.runs.is_none());
    assert!(result.buckets.iter().all(|bucket| {
        bucket.is_some_and(|bucket| {
            bucket.min_value == 0.0 && bucket.max_value == 0.0 && !bucket.has_break
        })
    }));
    let unknown = dense(
        &mut values,
        &BTreeMap::new(),
        &mut present,
        &BTreeMap::new(),
        &mut rosters,
        &native,
        &BTreeMap::new(),
        &[],
        &[span(0, 99)],
        (0.0, 99.0, 2),
        &flag,
    )
    .unwrap();
    assert!(unknown.buckets.iter().all(Option::is_none));
}

#[test]
fn faction_prepared_navigation_skips_cancelling_changes_and_keeps_membership_edges() {
    let flag = AtomicBool::new(false);
    let fixture = Fixture::new();
    let mut values = Accumulator::default();
    for hour in 0..=10 {
        values.observe(
            1,
            hour,
            Some(if hour < 5 {
                10
            } else if hour < 7 {
                20
            } else {
                22
            }),
            false,
        );
        values.observe(2, hour, Some(if hour < 5 { 20 } else { 10 }), false);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut history = History {
        factions: BTreeMap::from([(
            "401".into(),
            vec![
                RosterRun {
                    first_hour: 0,
                    last_hour: 8,
                    name: None,
                    members: vec!["A".into(), "B".into()],
                    complete: true,
                },
                RosterRun {
                    first_hour: 9,
                    last_hour: 10,
                    name: None,
                    members: vec!["A".into()],
                    complete: true,
                },
            ],
        )]),
        faction_reads: vec![span(0, 10)],
        last_hour: 10,
        end_recorded: true,
        ..History::default()
    };
    history.country_reads = vec![span(0, 10)];
    history.country_presence = HashMap::from([
        ("A".into(), vec![span(0, 10)]),
        ("B".into(), vec![span(0, 10)]),
    ]);
    let mut identities = RecordedHistories::default();
    identities.identity_reads = vec![span(0, 10)];
    for (tag, slot) in [("A", 1), ("B", 2)] {
        let mut country = RecordedCountry::default();
        country.identities = Arc::new(vec![CountryIdentityRun {
            first_hour: 0,
            last_hour: 10,
            slot,
            country_id: slot,
        }]);
        identities.countries.insert(tag.into(), country);
    }
    let params = FactionTotalsParams {
        branch_id: "1".into(),
        source: FactionMeasure::Territory {
            measure: TerritoryMeasure::States,
            rule: TerritoryRule::Owned,
        },
        membership: FactionMembership::Hour,
        top: 1,
        faction_id: Some("401".into()),
        parts: None,
        inventory: None,
    };
    let members = NativeMembers::new(&history, &identities, params.membership, &flag).unwrap();
    let keys = BTreeMap::from([("A".into(), 1), ("B".into(), 2)]);
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    for (key, from, first_hour, expected) in [
        (series::MeasureKey::Home, None, 0.0, 0.0),
        (series::MeasureKey::ArrowRight, Some(0.0), 0.0, 7.0),
        (series::MeasureKey::ArrowLeft, Some(7.0), 0.0, 0.0),
        (series::MeasureKey::ArrowRight, Some(7.0), 0.0, 9.0),
        (series::MeasureKey::End, None, 0.0, 9.0),
        (series::MeasureKey::Home, None, 1.5, 1.5),
        (series::MeasureKey::ArrowRight, Some(9.0), 0.0, 9.0),
    ] {
        let navigation = series::Navigation {
            key,
            from,
            first_hour,
            last_hour: 10.0,
        };
        assert_eq!(
            navigate(
                &mut values,
                &keys,
                &history,
                &members,
                &params,
                &navigation,
                &flag
            )
            .unwrap(),
            Some(expected),
            "{key:?} {from:?}"
        );
    }
}

#[test]
fn faction_native_counted_reused_alias_is_dated_before_merging_dense_bindings() {
    let flag = AtomicBool::new(false);
    let history = History {
        factions: BTreeMap::from([(
            "401".into(),
            vec![RosterRun {
                first_hour: 0,
                last_hour: 3,
                name: None,
                members: vec!["shared".into()],
                complete: true,
            }],
        )]),
        faction_reads: vec![span(0, 3)],
        last_hour: 3,
        end_recorded: true,
        ..History::default()
    };
    let mut identities = RecordedHistories::default();
    identities.identity_reads = vec![span(-1, 3)];
    for (tag, runs) in [
        ("AA future", vec![(0, 1, 2, 20)]),
        ("shared", vec![(0, 1, 1, 10), (2, 3, 2, 20)]),
        ("Z old", vec![(-1, -1, 1, 10)]),
    ] {
        let mut country = RecordedCountry::default();
        country.identities = Arc::new(
            runs.into_iter()
                .map(
                    |(first_hour, last_hour, slot, country_id)| CountryIdentityRun {
                        first_hour,
                        last_hour,
                        slot,
                        country_id,
                    },
                )
                .collect(),
        );
        identities.countries.insert(tag.into(), country);
    }
    let members =
        NativeMembers::new(&history, &identities, FactionMembership::Ever, &flag).unwrap();
    let shared: Vec<_> = members.counted["401"]
        .iter()
        .filter(|run| run.tag == "shared")
        .map(|run| span(run.first_hour, run.last_hour))
        .collect();
    assert_eq!(
        shared,
        [span(0, 3)],
        "native census order cannot discard an earlier holder of an alias"
    );
    let (native, _, _) = bindings(&members, "401", &flag).unwrap();
    assert_eq!(native.len(), 2);
    assert!(native.values().all(|spans| *spans == [span(0, 3)]));
    let fixture = Fixture::new();
    let keys = BTreeMap::from([
        ("AA future".into(), 1),
        ("shared".into(), 2),
        ("Z old".into(), 3),
    ]);
    let mut values = Accumulator::default();
    let mut present = Accumulator::default();
    let mut rosters = Accumulator::default();
    for hour in 0..=3 {
        values.observe(1, hour, (hour < 2).then_some(20), false);
        values.observe(2, hour, Some(if hour < 2 { 10 } else { 20 }), hour == 2);
        present.observe(1, hour, (hour < 2).then_some(1), false);
        present.observe(2, hour, Some(1), hour == 2);
        rosters.observe(0, hour, Some(1), false);
    }
    let mut writer = fixture.writer();
    values.flush(&mut writer, "values", 0, &flag).unwrap();
    present.flush(&mut writer, "presence", 0, &flag).unwrap();
    rosters.flush(&mut writer, "rosters", 0, &flag).unwrap();
    writer.commit().unwrap();
    let mut values = Reader::new(&fixture.store, &fixture.key, "values".into(), &flag).unwrap();
    let mut present = Reader::new(&fixture.store, &fixture.key, "presence".into(), &flag).unwrap();
    let mut rosters = Reader::new(&fixture.store, &fixture.key, "rosters".into(), &flag).unwrap();
    let (native, known, unknown) = bindings(&members, "401", &flag).unwrap();
    let summary = dense(
        &mut values,
        &keys,
        &mut present,
        &keys,
        &mut rosters,
        &members,
        &native,
        &known,
        &unknown,
        (0.0, 3.0, 1),
        &flag,
    )
    .unwrap();
    let bucket = summary.buckets[0].unwrap();
    assert_eq!(
        (
            bucket.first_value,
            bucket.last_value,
            bucket.min_value,
            bucket.max_value
        ),
        (30.0, 20.0, 20.0, 30.0),
        "aliases contribute once per native holder; proved absence contributes zero"
    );
}

fn total(first_hour: i32, last_hour: i32, value: i64) -> FactionTotalRun {
    FactionTotalRun {
        first_hour,
        last_hour,
        starts_segment: false,
        value,
    }
}

#[test]
fn faction_dense_top_reads_exact_ranks_only_in_a_crossing_column() {
    let flag = AtomicBool::new(false);
    let summary = |buckets: Vec<Option<ChartBucket>>| ChartSummary {
        approximate: Vec::new(),
        from: 0.0,
        to: 7.0,
        width: 2,
        buckets,
        runs: None,
    };
    let column = |first: i32, values: &[f64]| {
        values
            .iter()
            .zip(first..)
            .fold(None, |bucket, (&value, hour)| {
                let part = Some(ChartBucket::hour(hour, value, false));
                if bucket.is_some() {
                    merge(bucket, part)
                } else {
                    part
                }
            })
    };
    // A leads the first column throughout; in the second it leaves Top 1 after two hours.
    let mut summaries = BTreeMap::from([
        (
            "A".to_owned(),
            summary(vec![
                column(0, &[10.0, 12.0, 11.0, 10.0]),
                column(4, &[12.0, 12.0, 2.0, 2.0]),
            ]),
        ),
        (
            "B".to_owned(),
            summary(vec![column(0, &[3.0, 5.0, 5.0, 4.0]), column(4, &[5.0; 4])]),
        ),
        (
            "C".to_owned(),
            summary(vec![column(0, &[-2.0; 4]), column(4, &[-2.0; 4])]),
        ),
    ]);
    let mut windows = Vec::new();
    let ranked = retain_dense_top(
        &mut summaries,
        &BTreeMap::new(),
        1,
        (0.0, 7.0, 2),
        &mut |hours| {
            windows.push(*hours);
            Ok(BTreeMap::from([
                ("A".to_owned(), vec![total(4, 5, 12), total(6, 7, 2)]),
                ("B".to_owned(), vec![total(4, 7, 5)]),
                ("C".to_owned(), vec![total(4, 7, -2)]),
            ]))
        },
        &flag,
    )
    .unwrap();
    assert_eq!(
        windows,
        [span(4, 7)],
        "a settled column reads no exact sums"
    );
    assert_eq!(
        summaries.keys().map(String::as_str).collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(
        summaries["A"].buckets[0],
        column(0, &[10.0, 12.0, 11.0, 10.0]),
        "a column led throughout keeps its bucket"
    );
    let left = summaries["A"].buckets[1].unwrap();
    assert_eq!((left.first_hour, left.last_hour), (4, 5));
    assert_eq!(
        (
            left.first_value,
            left.last_value,
            left.min_value,
            left.max_value
        ),
        (12.0, 12.0, 12.0, 12.0),
        "hours after A leaves Top 1 supply neither its end nor its extent"
    );
    assert!(summaries["B"].buckets[0].is_none());
    let entered = summaries["B"].buckets[1].unwrap();
    assert_eq!(
        (entered.first_hour, entered.last_hour, entered.first_value),
        (6, 7, 5.0)
    );
    assert_eq!(
        ranked,
        BTreeMap::from([
            ("A".to_owned(), vec![span(0, 5)]),
            ("B".to_owned(), vec![span(6, 7)]),
        ])
    );
}

#[test]
fn faction_dense_top_ties_in_faction_order_and_never_ranks_a_missing_column() {
    let flag = AtomicBool::new(false);
    let summary = |buckets: Vec<Option<ChartBucket>>| ChartSummary {
        approximate: Vec::new(),
        from: 0.0,
        to: 3.0,
        width: 2,
        buckets,
        runs: None,
    };
    let column = |hour, first, last| {
        merge(
            Some(ChartBucket::hour(hour, first, false)),
            Some(ChartBucket::hour(hour + 1, last, false)),
        )
    };
    let original = BTreeMap::from([
        ("A".into(), summary(vec![column(0, -2.0, -2.0), None])),
        (
            "B".into(),
            summary(vec![column(0, -10.0, -2.0), column(2, 3.0, 3.0)]),
        ),
        ("C".into(), summary(vec![column(0, -10.0, -3.0), None])),
        ("unknown".into(), summary(vec![None, None])),
    ]);
    // Unknown has no bucket in the second column, but holds a total at its last hour.
    let valued = BTreeMap::from([("unknown".to_owned(), vec![span(3, 3)])]);
    let mut windows = Vec::new();
    let mut top = original.clone();
    let ranked = retain_dense_top(
        &mut top,
        &valued,
        1,
        (0.0, 3.0, 2),
        &mut |hours| {
            windows.push(*hours);
            Ok(BTreeMap::from([
                ("A".to_owned(), Vec::new()),
                ("B".to_owned(), vec![total(2, 3, 3)]),
                ("C".to_owned(), Vec::new()),
                ("unknown".to_owned(), vec![total(3, 3, 9)]),
            ]))
        },
        &flag,
    )
    .unwrap();
    assert_eq!(windows, [span(2, 3)]);
    assert_eq!(
        top.keys().map(String::as_str).collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert_eq!(
        top["A"], original["A"],
        "a missing column never becomes zero"
    );
    assert!(
        top["B"].buckets[0].is_none(),
        "B's maximum equals A's minimum, and A ranks first at equal totals"
    );
    let second = top["B"].buckets[1].unwrap();
    assert_eq!(
        (second.first_hour, second.last_hour),
        (2, 2),
        "a total without a column bucket still takes its exact rank"
    );
    assert_eq!(
        ranked,
        BTreeMap::from([
            ("A".to_owned(), vec![span(0, 1)]),
            ("B".to_owned(), vec![span(2, 2)]),
        ])
    );
    let mut all = original.clone();
    retain_dense_top(
        &mut all,
        &valued,
        u64::MAX,
        (0.0, 3.0, 2),
        &mut |_| Err(std::io::Error::other("no column needs exact sums")),
        &flag,
    )
    .unwrap();
    assert_eq!(
        all.len(),
        3,
        "fewer known candidates than Top N preserves every known column"
    );
    for id in ["A", "B", "C"] {
        assert_eq!(all[id], original[id]);
    }
}

#[test]
fn a_faction_s_view_folds_its_exact_runs_into_the_view_s_bins() {
    let started = |run: FactionTotalRun| FactionTotalRun {
        starts_segment: true,
        ..run
    };
    let runs = [
        started(total(0, 99, 5)),
        total(100, 149, 7),
        started(total(200, 299, 3)),
    ];
    let fold = |width| {
        folded(&runs, (0.0, 399.0, width))
            .unwrap()
            .buckets
            .iter()
            .map(|bucket| {
                bucket.map(|bucket| {
                    (
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_value,
                        bucket.max_value,
                        bucket.last_value,
                        bucket.first_break,
                        bucket.has_break,
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        fold(4),
        [
            Some((0, 99, 5.0, 5.0, 5.0, true, false)),
            Some((100, 149, 7.0, 7.0, 7.0, false, true)),
            Some((200, 299, 3.0, 3.0, 3.0, true, false)),
            None,
        ]
    );
    // A wider bin holds both runs it covers, and its hours left unknown break it.
    assert_eq!(
        fold(2),
        [
            Some((0, 149, 5.0, 7.0, 7.0, true, true)),
            Some((200, 299, 3.0, 3.0, 3.0, true, true))
        ]
    );
    let summary = folded(&runs, (0.0, 399.0, 4)).unwrap();
    assert_eq!(summary.buckets[0].unwrap().absolute_total, 500);
    assert!(summary.runs.is_none(), "a wide view lists no runs");
    // An hourly view lists the runs it shows, whole.
    let hourly = folded(&runs, (90.0, 109.0, 40)).unwrap();
    assert_eq!(
        hourly
            .runs
            .unwrap()
            .iter()
            .map(|run| (run.first_hour, run.last_hour))
            .collect::<Vec<_>>(),
        [(0, 99), (100, 149)]
    );
}

#[test]
fn a_slice_counts_only_its_own_overflowed_faction_hours() {
    let branch = Branch::new(
        Vec::new(),
        Sums {
            // Two factions overflow, across both edges of the window and inside it.
            overflow: vec![span(90, 104), span(108, 130), span(101, 102)],
            ..Sums::default()
        },
    );
    assert_eq!(slice_sums(&branch, &span(100, 119)).overflowed, 5 + 12 + 2);
    assert_eq!(slice_sums(&branch, &span(105, 107)).overflowed, 0);
    assert_eq!(slice_sums(&branch, &span(0, 200)).overflowed, 15 + 23 + 2);
}

#[test]
fn a_kept_branch_holds_a_reply_only_until_a_total_it_is_cut_from_changes() {
    let run = |first_hour, last_hour, value| MeasureRun {
        first_hour,
        last_hour,
        starts_segment: true,
        value: Some(value),
    };
    let branch = Branch::new(
        vec![
            CountryTrack {
                tag: "GER".into(),
                runs: vec![run(0, 120, 7)],
            },
            // A member whose runs overlap still changes only where one starts or ends.
            CountryTrack {
                tag: "ITA".into(),
                runs: vec![run(0, 140, 5), run(100, 110, 0)],
            },
        ],
        Sums {
            series: BTreeMap::from([
                (
                    "allies".to_owned(),
                    vec![total(0, 49, 10), total(50, 200, 12)],
                ),
                ("axis".to_owned(), vec![total(0, 79, 3), total(90, 200, 4)]),
            ]),
            recorded: vec![span(0, 200)],
            // Overflow spans of different factions overlap.
            overflow: vec![span(150, 170), span(155, 160)],
            ..Sums::default()
        },
    );
    let axis = Some(&"axis".to_owned());
    // Every faction: the first total to change after the hour ends the hold.
    assert_eq!(branch.held_through(10, None, []), 49);
    // An unknown stretch holds until the next total is known.
    assert_eq!(branch.held_through(85, axis, []), 89);
    // One faction is held by its own total, the overflow and only the members it shows.
    assert_eq!(branch.held_through(60, Some(&"allies".to_owned()), []), 149);
    assert_eq!(branch.held_through(60, axis, []), 79);
    assert_eq!(branch.held_through(91, axis, []), 149);
    assert_eq!(branch.held_through(91, axis, ["GER"]), 120);
    assert_eq!(branch.held_through(91, axis, ["ITA"]), 99);
    assert_eq!(branch.held_through(105, axis, ["ITA"]), 110);
    assert_eq!(branch.held_through(152, axis, []), 154);
    assert_eq!(branch.held_through(157, axis, []), 160);
    assert_eq!(branch.held_through(165, axis, []), 170);
}

#[test]
fn a_rewritten_or_emptied_range_anywhere_in_a_section_is_a_new_publication() {
    let fixture = Fixture::new();
    let publish = |ranges: &[(u64, &[u8])]| {
        let mut writer = fixture.writer();
        for &(range, bytes) in ranges {
            writer.write_at("values", range, bytes).unwrap();
        }
        writer.commit().unwrap();
        crate::prepared::query::observations::published(
            &fixture.store,
            &fixture.key,
            &["groups", "values"],
        )
        .unwrap()
    };
    let first = publish(&[(0, b"a"), (1, b"b"), (2, b"c")]);
    assert_eq!(
        crate::prepared::query::observations::published(
            &fixture.store,
            &fixture.key,
            &["groups", "values"]
        )
        .unwrap(),
        first,
        "an unchanged section keeps its publication"
    );
    // The middle range alone, then the last emptied as a catalogue retires its patches.
    let middle = publish(&[(1, b"d")]);
    assert_ne!(middle, first);
    let emptied = publish(&[(2, b"")]);
    assert_ne!(emptied, middle);
    assert_ne!(
        publish(&[(1, b"e")]),
        emptied,
        "a rewrite behind an emptied tail"
    );
}
