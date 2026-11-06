use super::test_fixtures::{casualty_frame, chosen, opened, research_replay};
use super::*;
use std::path::PathBuf;
fn params(count: u32) -> CasualtyParams {
    CasualtyParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        top: Some(CasualtyTop { count }),
    }
}

#[test]
fn supplying_pair_keys_hold_through_counter_changes_and_rebuild_at_a_new_lifetime() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::TableRows;
    let replay = research_replay(vec![casualty_frame(100, Some((10, 3)), 0)]);
    let mut builder = ScanFrames::new(&replay, 1);
    let mut oracle = ScanFrames::new(&replay, 1);
    let mut tracker = FrameChangeTracker::new();
    let mut held = std::ptr::null();
    for hour in 100..106 {
        let mut frame = casualty_frame(hour, Some((10 + i64::from(hour - 100), 3)), 0);
        if hour >= 103
            && let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
        {
            rows[0].start_hour = 101;
        }
        let changes = tracker.observe(&frame, replay.format);
        builder.frame_changes(&frame, &replay, &changes);
        // A builder with its join rebuilt follows the same hourly proof.
        oracle.supplying_keys_current = false;
        oracle.frame_changes(&frame, &replay, &changes);
        let current = builder.supplying_pairs.as_ref().unwrap().first().unwrap() as *const PairKey;
        if hour == 101 || hour == 102 || hour == 104 || hour == 105 {
            assert_eq!(
                current, held,
                "unchanged supplying keys were allocated again"
            );
        }
        held = current;
        assert_eq!(
            serde_json::to_vec(&builder).unwrap(),
            serde_json::to_vec(&oracle).unwrap()
        );
        assert_eq!(builder.changed_pairs, oracle.changed_pairs);
        assert_eq!(builder.changed_tags, oracle.changed_tags);
    }
}

#[test]
fn shared_country_run_walk_matches_point_reads_at_gaps_and_native_endpoints() {
    let wide = i128::from(i64::MAX) * 2;
    let mut track = Track::default();
    for (first, last, confirmed, identity) in [
        (-2, 25, 23, (1, 10)),
        (26, 26, 26, (1, 11)),
        (28, 40, 28, (1, 11)),
        (i32::MAX - 2, i32::MAX, i32::MAX - 1, (1, 11)),
    ] {
        track.flush();
        track.tail = Some(Sample {
            first,
            last,
            confirmed,
            identity,
            dealt: wide,
            taken: wide + 1,
            unknown: None,
            at_least: true,
        });
    }
    let scanned = ScannedCasualties {
        tracks: vec![("opaque holder".into(), track)],
        observed: vec![
            HourSpan {
                first_hour: -2,
                last_hour: 26,
            },
            HourSpan {
                first_hour: 28,
                last_hour: 40,
            },
            HourSpan {
                first_hour: i32::MAX - 2,
                last_hour: i32::MAX,
            },
        ],
        incomplete: true,
        windows: Vec::new(),
        names: Vec::new(),
        holders: BTreeMap::new(),
        pairs: pairs::PairHistory::default(),
    };
    let cancelled = AtomicBool::new(false);
    let mut runs = Vec::new();
    scanned
        .for_each_country_run("opaque holder", &cancelled, |run| {
            runs.push(run);
            Ok(())
        })
        .unwrap();
    assert_eq!(runs.iter().filter(|run| run.starts_segment).count(), 4);
    for hour in (-3..=41).chain((i32::MAX - 2)..=i32::MAX) {
        let from_run = runs
            .iter()
            .find(|run| run.first_hour <= hour && hour <= run.last_hour)
            .map(|run| run.value.clone());
        assert_eq!(
            from_run,
            scanned.country_at("opaque holder", hour),
            "hour {hour}"
        );
    }
    assert_eq!(runs.last().unwrap().last_hour, i32::MAX);
    assert_eq!(runs.last().unwrap().value.read_hour, i32::MAX - 1);
    assert!(
        scanned
            .for_each_country_run("not recorded", &cancelled, |_| {
                panic!("An unrecorded country cannot supply zero-valued runs")
            })
            .is_ok()
    );
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        scanned
            .for_each_country_run("opaque holder", &cancelled, |_| Ok(()))
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted
    );
}

#[test]
fn manual_casualties_follow_renames_but_clip_other_holders_and_gaps() {
    let mut scan = Scan::new();
    let holder = scan.country((1, 10));
    let replacement = scan.country((1, 11));
    let enemy = scan.country((2, 20));
    for (hour, tag, country, losses) in [
        (100, "peaceful original", holder, None),
        (101, "renamed | country", holder, Some((12, -4))),
        (102, "renamed | country", replacement, Some((100, 200))),
        (104, "renamed again", holder, Some((-2, 5))),
        (105, "peaceful original", replacement, Some((101, 201))),
    ] {
        let live = BTreeMap::from([(tag.into(), country), ("enemy".into(), enemy)]);
        scan.begin_hour(hour, &live, true, true);
        if let Some((taken, dealt)) = losses {
            scan.relation(country, enemy, 90, [taken, dealt, 0, 0]);
        }
        scan.end_hour(true);
    }
    let cancelled = AtomicBool::new(false);
    let scanned = scan.finish(&cancelled).unwrap();
    let request = |tag: &str| CasualtyParams {
        branch_id: "1".into(),
        tags: vec![tag.into()],
        top: None,
    };
    // The original tag was peaceful at first, but every native holder ever
    // bearing that explicit pick remains selected, including its later reuse.
    let original = scanned
        .reply(
            &request("peaceful original"),
            &cancelled,
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    assert!(
        original
            .warnings
            .iter()
            .all(|warning| !warning.contains("no sampled"))
    );
    assert_eq!(original.series.len(), 4);
    let renamed = original
        .series
        .iter()
        .find(|series| series.tag == "renamed | country" && series.identities[0].id == "1:10")
        .unwrap();
    assert_eq!(
        (reads(renamed), taken(renamed), value(&renamed.dealt, 0)),
        (vec![101], vec![12], -4)
    );
    assert_eq!(renamed.ends, [(0, 101, None)]);
    assert!(
        original
            .observed
            .iter()
            .all(|span| !(span.first_hour <= 103 && 103 <= span.last_hour))
    );

    // A different explicit alias selects only the holder observed under that
    // alias. The unrelated reuse of another alias never leaks into its history.
    let selected = scanned
        .reply(
            &request("renamed again"),
            &cancelled,
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(selected.series.len(), 2);
    assert!(
        selected
            .series
            .iter()
            .all(|series| series.identities[0].id == "1:10")
    );
    let first = selected
        .series
        .iter()
        .find(|series| series.tag == "renamed | country")
        .unwrap();
    assert_eq!(
        first.membership.as_ref().unwrap(),
        &[HourSpan {
            first_hour: 101,
            last_hour: 101
        }]
    );
    let later = selected
        .series
        .iter()
        .find(|series| series.tag == "renamed again")
        .unwrap();
    assert_eq!((reads(later), taken(later)), (vec![104], vec![-2]));
    assert_eq!(value(&later.dealt, 0), 5);
    assert!(later.unknown.is_none());
}

#[test]
fn a_gap_alias_projection_clips_an_unrelated_later_holder() {
    let mut scan = Scan::new();
    let holder = scan.country((1, 10));
    let replacement = scan.country((1, 11));
    let enemy = scan.country((2, 20));
    for (hour, tag, country, losses) in [
        (100, "original", holder, (0, 0)),
        (101, "original", holder, (0, 0)),
        (102, "reused alias", holder, (-3, 5)),
        (103, "reused alias", holder, (-3, 5)),
        (106, "reused alias", replacement, (7, 8)),
        (107, "reused alias", replacement, (7, 8)),
    ] {
        let live = BTreeMap::from([(tag.into(), country), ("enemy".into(), enemy)]);
        scan.begin_hour(hour, &live, true, true);
        scan.relation(country, enemy, 90, [losses.0, losses.1, 0, 0]);
        scan.end_hour(true);
    }
    let cancelled = AtomicBool::new(false);
    let scanned = scan.finish(&cancelled).unwrap();
    let reply = scanned
        .reply(
            &CasualtyParams {
                branch_id: "1".into(),
                tags: vec!["original".into()],
                top: None,
            },
            &cancelled,
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(
        reply.observed,
        [
            HourSpan {
                first_hour: 100,
                last_hour: 103
            },
            HourSpan {
                first_hour: 106,
                last_hour: 107
            }
        ]
    );
    assert_eq!(reply.series.len(), 2);
    assert!(
        reply
            .series
            .iter()
            .all(|series| series.identities[0].id == "1:10")
    );
    let original = reply
        .series
        .iter()
        .find(|series| series.tag == "original")
        .unwrap();
    assert!(original.membership.is_none());
    let alias = reply
        .series
        .iter()
        .find(|series| series.tag == "reused alias")
        .unwrap();
    assert_eq!(
        alias.membership.as_ref().unwrap(),
        &[HourSpan {
            first_hour: 102,
            last_hour: 103
        }]
    );
    assert_eq!((reads(alias), taken(alias)), (vec![102], vec![-3]));
    assert_eq!(alias.ends, [(0, 103, None)]);
}

#[test]
fn a_peaceful_manual_casualty_pick_uses_its_renamed_samples_and_existing_scan() {
    let mut before = casualty_frame(100, None, 0);
    let mut after = casualty_frame(101, Some((12, 0)), 0);
    for (frame, tag) in [(&mut before, "peaceful original"), (&mut after, "renamed")] {
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!();
        };
        rows[0].tag = tag.as_bytes().to_vec();
    }
    let source = opened(research_replay(vec![before, after]));
    let cancelled = AtomicBool::new(false);
    let scanned = scanned_branch(&source, 1, &cancelled).unwrap();
    let product = WarCasualties::load(
        &source,
        &CasualtyParams {
            branch_id: "1".into(),
            tags: vec!["peaceful original".into()],
            top: None,
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(product.series.len(), 1);
    assert_eq!(product.series[0].tag, "renamed");
    assert_eq!(
        (reads(&product.series[0]), taken(&product.series[0])),
        (vec![101], vec![12])
    );
    assert!(
        product
            .warnings
            .iter()
            .all(|warning| !warning.contains("no sampled"))
    );
    assert!(Arc::ptr_eq(
        &scanned,
        &scanned_branch(&source, 1, &cancelled).unwrap()
    ));
}
fn reply(frames: Vec<DecodedFrame>, request: &CasualtyParams) -> CasualtyReply {
    let source = opened(research_replay(frames));
    WarCasualties::load(
        &source,
        request,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn value(values: &CasualtyValues, index: usize) -> i128 {
    let mut value = values.0.parse::<i128>().unwrap();
    let mut left = index;
    for delta in &values.1 {
        let (count, delta) = match delta {
            CasualtyDelta::Small(n) => (1, i128::from(*n)),
            CasualtyDelta::Wide(n) => (1, n.parse::<i128>().unwrap()),
            CasualtyDelta::Repeated(count, n) => (*count as usize, n.parse::<i128>().unwrap()),
        };
        let count = count.min(left);
        value += delta * count as i128;
        left -= count;
        if left == 0 {
            break;
        }
    }
    assert_eq!(left, 0, "missing delta at {index}");
    value
}
fn reads(series: &CasualtySeries) -> Vec<i32> {
    let mut read = 0;
    series
        .reads
        .iter()
        .enumerate()
        .map(|(i, n)| {
            read = if i == 0 { *n } else { read + *n };
            i32::try_from(read).unwrap()
        })
        .collect()
}
fn taken(series: &CasualtySeries) -> Vec<i128> {
    (0..series.reads.len())
        .map(|i| value(&series.taken, i))
        .collect()
}
fn members(reply: &CasualtyReply, hour: i32) -> Vec<&str> {
    reply
        .series
        .iter()
        .filter(|s| {
            s.membership.as_ref().is_some_and(|spans| {
                spans
                    .iter()
                    .any(|s| s.first_hour <= hour && hour <= s.last_hour)
            })
        })
        .map(|s| s.tag.as_str())
        .collect()
}

#[test]
fn war_casualties_shared_measure_agrees_with_product_reads_and_gaps() {
    let frames = (100..=145)
        .filter(|h| !(127..130).contains(h))
        .map(|h| {
            casualty_frame(
                h,
                (h < 130).then_some((if h < 120 { 10 } else { 30 }, 0)),
                0,
            )
        })
        .collect();
    let source = opened(research_replay(frames));
    let request = chosen();
    let cancelled = AtomicBool::new(false);
    let product = WarCasualties::load(
        &source,
        &request,
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    let cached = scanned_branch(&source, 1, &cancelled).unwrap();
    for h in [99, 100, 101, 119, 126, 128, 130, 143, 144, 145, 146] {
        let from_reply = product
            .observed
            .iter()
            .find(|span| span.first_hour <= h && h <= span.last_hour)
            .and_then(|_| {
                product.series.iter().find_map(|series| {
                    let dates = reads(series);
                    let index = dates.partition_point(|read| *read <= h).checked_sub(1)?;
                    let endpoint = series.ends.iter().find(|end| end.0 as usize == index);
                    let last = endpoint.map_or_else(|| dates[index + 1] - 1, |end| end.1);
                    if h > last {
                        return None;
                    }
                    Some(CasualtyMeasure {
                        read_hour: h.min(endpoint.and_then(|end| end.2).unwrap_or(h)),
                        dealt: value(&series.dealt, index),
                        taken: value(&series.taken, index),
                        unknown: series
                            .unknown
                            .as_ref()
                            .filter(|_| series.unknown_until.is_none_or(|until| h < until))
                            .map(|values| i64::try_from(value(values, index)).unwrap()),
                        at_least: series.at_least_from.is_some_and(|from| from <= h),
                    })
                })
            });
        assert_eq!(
            cached.country_at(&request.tags[0], h),
            from_reply,
            "hour {h}"
        );
    }
    assert_eq!(
        cached.country_at(&request.tags[0], 101).unwrap().read_hour,
        101
    );
    assert!(cached.country_at(&request.tags[0], 128).is_none());
    let after_gap = cached.country_at(&request.tags[0], 130).unwrap();
    assert_eq!(
        (
            after_gap.dealt,
            after_gap.taken,
            after_gap.unknown,
            after_gap.at_least
        ),
        (60, 30, None, true)
    );
    assert!(cached.country_at("missing opaque tag", 130).is_none());
    assert_eq!(cached.has_gap(), product.has_gap);
    assert!(cached.has_gap());
    assert!(Arc::ptr_eq(
        &cached,
        &scanned_branch(&source, 1, &cancelled).unwrap()
    ));
}

#[test]
fn packed_country_lookup_seeks_across_checkpoints_identities_and_holes() {
    let mut track = Track::default();
    let mut country = Country::default();
    for day in 0..600 {
        country.taken = i128::from(i64::MAX) + i128::from(day);
        country.dealt = -country.taken;
        country.unknown.total = (day < 300).then_some(i64::from(day));
        country.at_least = day >= 300;
        let first = -500 + day * 24;
        track.sample(first, (1, if day < 300 { 100 } else { 101 }), &country);
        track.tail.as_mut().unwrap().last = first + 22;
    }
    assert_eq!(track.checkpoints.len(), 3);
    for day in [0, 1, 255, 256, 257, 299, 300, 511, 512, 599] {
        for offset in [-1, 0, 10, 22, 23, 24] {
            let h = -500 + day * 24 + offset;
            let expected = track.iter().find(|run| run.first <= h && h <= run.last);
            let actual = track.at(h);
            assert_eq!(
                actual.as_ref().map(|run| (
                    run.first,
                    run.last,
                    run.identity,
                    run.dealt,
                    run.taken,
                    run.unknown,
                    run.at_least
                )),
                expected.as_ref().map(|run| (
                    run.first,
                    run.last,
                    run.identity,
                    run.dealt,
                    run.taken,
                    run.unknown,
                    run.at_least
                )),
                "hour {h}"
            );
        }
    }
}

use dashboard_base::test_fixtures::{CAMPAIGN, records};
use replay::container::Replay;
use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};

#[test]
fn selection_validation_and_exact_escaped_json_budget() {
    assert!(WarCasualties::validate(&params(0)).is_err());
    for count in [31, 40, u32::MAX] {
        assert!(WarCasualties::validate(&params(count)).is_ok());
    }
    let mut request = params(10);
    request.tags.push("mod tag".into());
    assert!(WarCasualties::validate(&request).is_err());
    let text = "mod tag with spaces";
    assert_eq!(
        json_bytes(&text).unwrap(),
        serde_json::to_vec(&text).unwrap().len()
    );
    assert!(check_reply_bytes(MAX_PRODUCT_REPLY_BYTES + 1).is_err());
}
#[test]
fn requests_keep_more_than_thirty_chosen_top_and_all_countries() {
    let source = opened(research_replay((1..=6).map(scale_frame).collect()));
    let cancelled = AtomicBool::new(false);
    let scanned = scanned_branch(&source, 1, &cancelled).unwrap();
    for count in [31, 40] {
        let top = WarCasualties::load(
            &source,
            &params(count),
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(members(&top, 1).len(), count as usize);
        let request = CasualtyParams {
            branch_id: "1".into(),
            tags: top.series.iter().map(|series| series.tag.clone()).collect(),
            top: None,
        };
        let chosen = WarCasualties::load(
            &source,
            &request,
            &LoadContext { content: None },
            &cancelled,
        )
        .unwrap();
        assert_eq!(chosen.series.len(), count as usize);
        for series in &top.series {
            assert_eq!(reads(series), [1, 2, 3, 4, 5, 6]);
            assert!(matches!(
                series.taken.1.as_slice(),
                [CasualtyDelta::Repeated(5, _)]
            ));
            assert_eq!(
                value(&series.taken, 0),
                scanned.country_at(&series.tag, 1).unwrap().taken
            );
            assert_eq!(
                value(&series.taken, 5),
                scanned.country_at(&series.tag, 6).unwrap().taken
            );
        }
        assert!(json_bytes(&top).unwrap() <= MAX_PRODUCT_REPLY_BYTES);
    }
    let all = WarCasualties::load(
        &source,
        &CasualtyParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: None,
        },
        &LoadContext { content: None },
        &cancelled,
    )
    .unwrap();
    assert_eq!(all.series.len(), 40);
    assert!(json_bytes(&all).unwrap() <= MAX_PRODUCT_REPLY_BYTES);
    assert!(Arc::ptr_eq(
        &scanned,
        &scanned_branch(&source, 1, &cancelled).unwrap()
    ));
}

#[test]
fn top_stream_skips_a_later_holder_that_never_enters_the_ranking() {
    let first = casualty_frame(100, Some((10, 0)), 0);
    let mut later = casualty_frame(101, Some((0, 0)), 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *later.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *later.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut().find(|row| row.id == 100).unwrap().id = 101;
    }
    let product = reply(vec![first, later], &params(1));
    let former = product
        .series
        .iter()
        .find(|series| series.tag == chosen().tags[0])
        .unwrap();
    assert_eq!(former.identities[0].id, "1:100");
    assert_eq!(members(&product, 100), [chosen().tags[0].as_str()]);
    assert_eq!(members(&product, 101), ["mod country 2| with spaces"]);
    assert!(product.series.iter().all(|series| !series.reads.is_empty()));
}

#[test]
fn all_countries_includes_a_country_created_after_the_first_catalogue() {
    let mut first = casualty_frame(100, Some((10, 0)), 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first.tables[TableId::Countries.index()]
    {
        rows.pop();
    }
    let mut later = casualty_frame(101, Some((10, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *later.tables[TableId::WarRelations.index()]
    {
        rows.push(replay_data::rows::WarRelationRow {
            first_country: 2,
            second_country: 3,
            first_casualties: 4,
            ..Default::default()
        });
    }
    let product = reply(
        vec![first, later],
        &CasualtyParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            top: None,
        },
    );
    assert_eq!(product.series.len(), 3);
    let created = product
        .series
        .iter()
        .find(|series| series.tag == "mod country 3| with spaces")
        .unwrap();
    assert_eq!(reads(created), [101]);
    assert_eq!(value(&created.dealt, 0), 4);
}

#[test]
fn complete_reply_refuses_measured_bytes_including_headers_and_escaped_labels() {
    let source = opened(research_replay(vec![casualty_frame(100, Some((10, 0)), 0)]));
    let cancelled = AtomicBool::new(false);
    let scanned = scanned_branch(&source, 1, &cancelled).unwrap();
    let empty = opened(research_replay(vec![casualty_frame(100, None, 0)]));
    let empty_scan = scanned_branch(&empty, 1, &cancelled).unwrap();
    let header = "x".repeat(MAX_PRODUCT_REPLY_BYTES);
    let refused = empty_scan
        .reply(&chosen(), &cancelled, None, &header, Vec::new())
        .unwrap_err();
    assert!(refused.to_string().contains("more than one reply holds"));
    let labels = CampaignLabels::from([(
        chosen().tags[0].clone(),
        vec!["\\".repeat(MAX_PRODUCT_REPLY_BYTES / 2)],
    )]);
    let refused = scanned
        .reply(
            &chosen(),
            &cancelled,
            Some(&labels),
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap_err();
    assert!(refused.to_string().contains("more than one reply holds"));
    assert!(check_reply_bytes(MAX_PRODUCT_REPLY_BYTES).is_ok());
}

#[test]
fn casualties_hourly_points_keep_ended_disappeared_totals() {
    let r = reply(
        (100..=149)
            .map(|h| casualty_frame(h, (h <= 122).then_some((i64::from(h), 0)), 122))
            .collect(),
        &chosen(),
    );
    assert_eq!(reads(&r.series[0]), (100..=122).collect::<Vec<_>>());
    assert_eq!(taken(&r.series[0]), (100..=122).collect::<Vec<i128>>());
    assert_eq!(r.series[0].ends, [(22, 149, None)]);
    assert!(r.series[0].membership.is_none());
}
#[test]
fn a_pair_lost_in_a_gap_marks_both_endpoints_as_lower_bounds_permanently() {
    let mut request = chosen();
    request.tags.push("mod country 2| with spaces".into());
    let r = reply(
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            casualty_frame(103, None, 0),
            casualty_frame(104, Some((20, 0)), 0),
        ],
        &request,
    );
    assert!(r.has_gap);
    assert_eq!(r.observed.len(), 2);
    for series in &r.series {
        assert_eq!(series.at_least_from, Some(103));
        assert_eq!(series.unknown_until, Some(103));
    }
    assert_eq!(taken(&r.series[0]), [10, 10, 20]);
}
#[test]
fn resumed_pairs_and_pairs_already_gone_before_a_gap_are_not_lower_bounds() {
    for frames in [
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            casualty_frame(103, Some((20, 0)), 0),
        ],
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            casualty_frame(101, None, 0),
            casualty_frame(104, None, 0),
        ],
    ] {
        let r = reply(frames, &chosen());
        assert!(r.has_gap);
        assert!(r.series[0].at_least_from.is_none());
    }
}
#[test]
fn a_war_entirely_in_a_gap_still_has_a_widget_note() {
    let r = reply(
        vec![casualty_frame(100, None, 0), casualty_frame(103, None, 0)],
        &params(10),
    );
    assert!(r.series.is_empty());
    assert!(r.has_gap);
}
#[test]
fn casualties_unreadable_hours_flush_endpoints_and_never_bridge_gaps() {
    let mut missing = casualty_frame(101, None, 0);
    *missing.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "failed walk".into(),
    };
    let r = reply(
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            missing,
            casualty_frame(102, Some((12, 0)), 0),
        ],
        &chosen(),
    );
    assert_eq!(reads(&r.series[0]), [100, 102]);
    assert_eq!(r.series[0].ends, [(0, 100, None), (1, 102, None)]);
    assert_eq!(r.series[0].unknown_until, Some(102));
}
#[test]
fn casualties_unknown_refuses_resets_disagreement_and_disjoint_relations() {
    for mode in 0..3 {
        let before = casualty_frame(117, Some((10, 0)), 0);
        let middle = casualty_frame(118, Some((10, 5)), 0);
        let mut after = casualty_frame(119, Some((10, if mode == 0 { 3 } else { 9 })), 0);
        if let TableView::Complete(TableRows::WarRelations(rows)) =
            &mut *after.tables[TableId::WarRelations.index()]
        {
            if mode == 1 {
                rows.push(replay_data::rows::WarRelationRow {
                    second_country: 3,
                    first_unknown_casualties: 99,
                    ..rows[0].clone()
                });
            }
            if mode == 2 {
                rows[0].second_country = 3;
            }
        }
        let r = reply(vec![before, middle, after], &chosen());
        assert_eq!(r.series[0].unknown_until, Some(119));
    }
}
#[test]
fn casualties_unknown_is_deduplicated_at_every_hourly_read() {
    let frames = [0, 5, 9]
        .into_iter()
        .enumerate()
        .map(|(i, unknown)| {
            let mut f = casualty_frame(117 + i as i32, Some((10, unknown)), 0);
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *f.tables[TableId::WarRelations.index()]
            {
                rows.push(replay_data::rows::WarRelationRow {
                    second_country: 3,
                    ..rows[0].clone()
                });
            }
            f
        })
        .collect();
    let r = reply(frames, &chosen());
    assert_eq!(reads(&r.series[0]), [117, 118, 119]);
    assert_eq!(taken(&r.series[0]), [20, 20, 20]);
    for (index, unknown) in [0, 5, 9].into_iter().enumerate() {
        assert_eq!(value(r.series[0].unknown.as_ref().unwrap(), index), unknown);
    }
    assert!(
        reply(vec![casualty_frame(119, Some((10, 5)), 0)], &chosen()).series[0]
            .unknown
            .is_none()
    );
}
#[test]
fn changed_relation_keys_refresh_joins_while_unchanged_keys_keep_reading_counters() {
    let mut moved = casualty_frame(101, Some((3, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *moved.tables[TableId::WarRelations.index()]
    {
        rows[0].second_country = 3;
    }
    let mut later = moved.clone();
    later.hour = 102;
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *later.tables[TableId::WarRelations.index()]
    {
        rows[0].first_casualties = 4;
    }
    let r = reply(
        vec![casualty_frame(100, Some((10, 0)), 0), moved, later],
        &chosen(),
    );
    assert_eq!(taken(&r.series[0]), [10, 13, 14]);
}

#[test]
fn repeated_alias_duplicates_are_never_reused_as_valid_endpoint_joins() {
    let mut duplicate = casualty_frame(101, Some((11, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *duplicate.tables[TableId::WarRelations.index()]
    {
        rows.push(replay_data::rows::WarRelationRow {
            first_country: 100,
            second_country: 200,
            ..rows[0].clone()
        });
    }
    let mut repeated = duplicate.clone();
    repeated.hour = 102;
    let r = reply(
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            duplicate,
            repeated,
            casualty_frame(103, Some((12, 0)), 0),
        ],
        &chosen(),
    );
    assert_eq!(
        r.observed
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (103, 103)]
    );
    assert_eq!(taken(&r.series[0]), [10, 12]);
}

#[test]
fn top_identity_endpoint_and_gap_keep_membership_bounded_and_samples_dated() {
    let frames = (0..=49)
        .filter(|h| *h != 25)
        .map(|h| {
            let mut f = casualty_frame(h, Some((i64::from(h), 0)), 0);
            if h >= 24 {
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *f.tables[TableId::Countries.index()]
                {
                    rows[0].id = 101;
                }
                if let TableView::Complete(TableRows::CountryLinks(rows)) =
                    &mut *f.tables[TableId::CountryLinks.index()]
                {
                    rows.iter_mut().find(|r| r.id == 100).unwrap().id = 101;
                }
            }
            f
        })
        .collect();
    let r = reply(frames, &params(2));
    assert_eq!(r.observed.len(), 2);
    assert!(
        r.series
            .iter()
            .any(|series| series.identities[0].id == "1:101" && reads(series).contains(&24))
    );
    for h in 0..=49 {
        assert!(members(&r, h).len() <= 2);
    }
    assert!(members(&r, 25).is_empty());
}
#[test]
fn holder_changes_are_sampled_at_the_first_hourly_read() {
    let mut changed = casualty_frame(101, Some((3, 0)), 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut().find(|r| r.id == 100).unwrap().id = 101;
    }
    let r = reply(
        vec![
            casualty_frame(100, Some((10, 0)), 0),
            changed,
            casualty_frame(102, None, 0),
        ],
        &chosen(),
    );
    assert_eq!(reads(&r.series[0]), [100]);
    assert_eq!(reads(&r.series[1])[0], 101);
    assert_eq!(taken(&r.series[1])[0], 3);
}
#[test]
fn top_ranks_every_country_by_known_totals_and_ignores_unknown() {
    for unknown in [0, 1000] {
        let frames = (0..24)
            .map(|h| {
                let mut f = casualty_frame(h, Some((10, if h == 0 { 0 } else { unknown })), 0);
                if let TableView::Complete(TableRows::WarRelations(rows)) =
                    &mut *f.tables[TableId::WarRelations.index()]
                {
                    rows.push(replay_data::rows::WarRelationRow {
                        first_country: 2,
                        second_country: 3,
                        first_casualties: 1,
                        second_casualties: 1,
                        ..Default::default()
                    });
                }
                f
            })
            .collect();
        let r = reply(frames, &params(1));
        assert_eq!(r.series.len(), 1);
        assert_eq!(r.series[0].tag, "mod country 2| with spaces");
    }
}

#[test]
fn a_holder_without_relations_does_not_extend_the_former_holders_row() {
    let mut changed = casualty_frame(101, None, 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut().find(|r| r.id == 100).unwrap().id = 101;
    }
    let mut later = changed.clone();
    later.hour = 102;
    let frames = vec![casualty_frame(100, Some((10, 0)), 0), changed, later];
    let r = reply(frames.clone(), &chosen());
    assert_eq!(r.series.len(), 1);
    assert_eq!(r.series[0].ends, [(0, 100, None)]);
    let top = reply(frames, &params(1));
    assert_eq!(members(&top, 100), ["mod country 1| with spaces"]);
    assert_eq!(members(&top, 101), ["mod country 2| with spaces"]);
    assert_eq!(members(&top, 102), ["mod country 2| with spaces"]);
}

#[test]
fn a_holder_change_during_an_unresolved_read_resumes_with_the_new_identity() {
    let first = casualty_frame(100, Some((10, 0)), 0);
    let mut changed = casualty_frame(101, Some((3, 0)), 0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 101;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut().find(|r| r.id == 100).unwrap().id = 101;
    }
    let mut later = changed.clone();
    later.hour = 102;
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *changed.tables[TableId::WarRelations.index()]
    {
        rows[0].first_country = u32::MAX;
    }
    let r = reply(vec![first, changed, later], &chosen());
    assert_eq!(
        r.observed
            .iter()
            .map(|s| (s.first_hour, s.last_hour))
            .collect::<Vec<_>>(),
        [(100, 100), (102, 102)]
    );
    assert_eq!(reads(&r.series[0]), [100]);
    assert_eq!(r.series[0].ends, [(0, 100, None)]);
    assert_eq!(reads(&r.series[1]), [102]);
    assert_eq!(r.series[1].identities[0].id, "1:101");
}
#[test]
fn hourly_confirmation_survives_top_membership_clipping() {
    let frames = (0..=23)
        .map(|hour| {
            let mut frame = casualty_frame(hour, Some((if hour < 23 { 50 } else { 0 }, 0)), 0);
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            {
                rows.push(replay_data::rows::WarRelationRow {
                    first_country: 2,
                    second_country: 3,
                    first_casualties: if hour < 23 { 0 } else { 100 },
                    second_casualties: if hour < 23 { 0 } else { 100 },
                    ..Default::default()
                });
            }
            frame
        })
        .collect();
    let r = reply(frames, &params(1));
    let former = r.series.iter().find(|s| s.tag == chosen().tags[0]).unwrap();
    assert_eq!(former.reads, [0]);
    assert_eq!(former.ends, [(0, 22, None)]);
    let branch_end = reply(
        (0..=22)
            .map(|h| casualty_frame(h, Some((50, 0)), 0))
            .collect(),
        &chosen(),
    );
    assert_eq!(branch_end.series[0].ends, [(0, 22, None)]);
}

#[test]
fn top_membership_tracks_every_entrant_and_smaller_n_is_a_cached_prefix() {
    let frames = (0..72)
        .map(|h| {
            let mut f = casualty_frame(h, Some((if h < 24 { 50 } else { 0 }, 0)), 0);
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *f.tables[TableId::WarRelations.index()]
            {
                rows.push(replay_data::rows::WarRelationRow {
                    first_country: 2,
                    second_country: 3,
                    start_hour: 0,
                    first_casualties: if h < 24 { 0 } else { 100 },
                    second_casualties: if h < 24 { 0 } else { 100 },
                    ..Default::default()
                });
            }
            f
        })
        .collect();
    let source = opened(research_replay(frames));
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    let r = scanned
        .reply(
            &params(1),
            &AtomicBool::new(false),
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(members(&r, 0), ["mod country 1| with spaces"]);
    assert_eq!(members(&r, 47), ["mod country 2| with spaces"]);
    let larger = scanned
        .reply(
            &params(2),
            &AtomicBool::new(false),
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    for h in 0..72 {
        for tag in members(&r, h) {
            assert!(members(&larger, h).contains(&tag));
        }
    }
    assert!(Arc::ptr_eq(
        &scanned,
        &scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap()
    ));
}

#[test]
fn top_membership_changes_hourly_and_never_spans_a_missing_hour() {
    let frames = (100..=105)
        .filter(|hour| *hour != 103)
        .map(|hour| {
            let mut frame = casualty_frame(hour, Some((if hour == 100 { 50 } else { 0 }, 0)), 0);
            if let TableView::Complete(TableRows::WarRelations(rows)) =
                &mut *frame.tables[TableId::WarRelations.index()]
            {
                rows.push(replay_data::rows::WarRelationRow {
                    first_country: 2,
                    second_country: 3,
                    first_casualties: if hour == 101 { 100 } else { 0 },
                    second_casualties: if hour == 101 { 100 } else { 0 },
                    ..Default::default()
                });
            }
            frame
        })
        .collect();
    let r = reply(frames, &params(1));
    assert_eq!(members(&r, 100), ["mod country 1| with spaces"]);
    assert_eq!(members(&r, 101), ["mod country 2| with spaces"]);
    assert_eq!(members(&r, 102), ["mod country 1| with spaces"]);
    assert!(members(&r, 103).is_empty());
    assert_eq!(members(&r, 104), ["mod country 1| with spaces"]);
}
#[test]
fn equal_hourly_values_compress_and_keep_wide_signed_totals() {
    let mut f = casualty_frame(-3, Some((0, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *f.tables[TableId::WarRelations.index()]
    {
        rows[0].first_casualties = i64::MAX;
        rows.push(replay_data::rows::WarRelationRow {
            second_country: 3,
            ..rows[0].clone()
        });
    }
    let frames = (-3..=300)
        .map(|h| {
            let mut f = f.clone();
            f.hour = h;
            f
        })
        .collect();
    let r = reply(frames, &chosen());
    assert_eq!(r.series[0].reads, [-3]);
    assert_eq!(value(&r.series[0].taken, 0), i128::from(i64::MAX) * 2);
    assert_eq!(r.series[0].ends, [(0, 300, None)]);
    assert!(
        check_cadence(Cadence::Daily(0))
            .unwrap_err()
            .to_string()
            .contains("Unsupported WarRelations cadence")
    );
    assert!(matches!(
        delta(i128::from(i64::MAX)),
        CasualtyDelta::Wide(_)
    ));
}
fn scale_frame(hour: i32) -> DecodedFrame {
    let mut f = casualty_frame(hour, None, 0);
    *f.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=40)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("mod_country{id:04}").into_bytes(),
                political_power_raw: Some(i64::from(hour)),
                stability_raw: i64::from(hour),
                manpower_residual: hour,
                ..Default::default()
            })
            .collect(),
    ));
    *f.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=40)
            .map(|id| CountryLinkRow {
                id,
                canonical_slot: id,
            })
            .collect(),
    ));
    *f.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (1..=40)
            .map(|id| replay_data::rows::WarRelationRow {
                first_country: id,
                second_country: id % 40 + 1,
                start_hour: 0,
                first_casualties: i64::from(hour) * i64::from(id),
                second_casualties: i64::from(hour) * i64::from(id % 40 + 1),
                first_unknown_casualties: i64::from(hour),
                second_unknown_casualties: i64::from(hour),
                ..Default::default()
            })
            .collect(),
    ));
    // One holder changes each year; volatile balances still change every hour.
    let holder = 1 + 40 * (hour / (365 * 24)) as u32;
    if holder != 1 {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *f.tables[TableId::Countries.index()]
        {
            rows[0].id = holder;
            rows.sort_by_key(|r| r.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *f.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: holder,
                canonical_slot: 1,
            });
        }
    }
    *f.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            (1..=40)
                .map(|country| replay_data::rows::CountryAppearanceRow {
                    country: if country == 1 { holder } else { country },
                    ..Default::default()
                })
                .collect(),
        ));
    *f.tables[TableId::Politics.index()] = TableView::Complete(TableRows::Politics(
        (1..=40)
            .map(|country| replay_data::rows::PoliticsRow {
                country: if country == 1 { holder } else { country },
                ..Default::default()
            })
            .collect(),
    ));
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *f.tables[TableId::CountryAppearance.index()]
    {
        rows.sort_by_key(|r| r.country);
    }
    if let TableView::Complete(TableRows::Politics(rows)) =
        &mut *f.tables[TableId::Politics.index()]
    {
        rows.sort_by_key(|r| r.country);
    }
    *f.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(Vec::new()));
    *f.tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    f
}
struct TestFolder(PathBuf);

impl TestFolder {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4-casualties-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

#[test]
fn war_casualties_all_new_zero_relations_do_not_prove_a_spanning_unknown_increment() {
    let before = casualty_frame(117, Some((10, 0)), 0);
    let mut replacement = casualty_frame(118, Some((0, 0)), 0);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *replacement.tables[TableId::WarRelations.index()]
    {
        rows[0].second_country = 3;
        rows[0].start_hour = 118;
    }
    let source = opened(research_replay(vec![before, replacement]));
    let scanned = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.country_at(&chosen().tags[0], 117).unwrap().unknown,
        Some(0)
    );
    assert_eq!(
        scanned.country_at(&chosen().tags[0], 118).unwrap().unknown,
        None
    );
    let spanning = reply(
        vec![
            casualty_frame(117, Some((10, 0)), 0),
            casualty_frame(118, Some((10, 0)), 0),
        ],
        &chosen(),
    );
    assert!(spanning.series[0].unknown.is_some());
    assert!(spanning.series[0].unknown_until.is_none());
}

impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn loaded_content(source: &Source) -> (LoadContext, TestFolder) {
    let folder = TestFolder::new();
    let root = &folder.0;
    std::fs::create_dir_all(root.join("localisation/english")).unwrap();
    let mut text = String::from("﻿l_english:\n CASUALTIES_UNKNOWN:0 \"Fixture unknown losses\"\n");
    for id in 1..=40 {
        text.push_str(&format!(" mod_country{id:04}:0 \"Fixture country {id}\"\n"));
    }
    std::fs::write(
        root.join("localisation/english/countries_l_english.yml"),
        text,
    )
    .unwrap();
    let catalogue = Arc::new(
        dashboard_text::catalogue::Catalogue::of_layers(
            installed_content_core::ResolvedLayers::from_folders(&[(root, &[])]).unwrap(),
            "english",
        )
        .unwrap(),
    );
    let options = dashboard_core::config::ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: root.clone(),
            user_dir: root.clone(),
            cache_dir: root.join("cache"),
        },
        allow_unverified: true,
        presentation: Default::default(),
    };
    dashboard_base::catalogue::set_test_catalogue(source, &options, catalogue);
    (
        LoadContext {
            content: Some(options),
        },
        folder,
    )
}
#[test]
fn casualty_titles_preserve_empty_and_key_equal_localisations() {
    for (authored, expected) in [
        (Some(""), ""),
        (Some("   "), "   "),
        (Some(UNKNOWN_LABEL), UNKNOWN_LABEL),
        (None, "Unknown attacker"),
    ] {
        let source = opened(research_replay(vec![casualty_frame(100, Some((10, 0)), 0)]));
        let text = authored.map_or_else(
            || "\u{feff}l_english:\n".to_owned(),
            |label| format!("\u{feff}l_english:\n {UNKNOWN_LABEL}:0 \"{label}\"\n"),
        );
        let (catalogue, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "localisation/english/casualties_l_english.yml",
            &text,
        )]);
        let options = dashboard_core::config::ContentOptions {
            options: installed_content_core::ResolveOptions {
                game_dir: folder.0.clone(),
                user_dir: folder.0.clone(),
                cache_dir: folder.0.join("cache"),
            },
            allow_unverified: true,
            presentation: Default::default(),
        };
        dashboard_base::catalogue::set_test_catalogue(&source, &options, Arc::new(catalogue));
        let reply = WarCasualties::load(
            &source,
            &params(1),
            &LoadContext {
                content: Some(options),
            },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(reply.unknown_label, expected);
        assert!(reply.series.iter().all(|series| {
            series
                .labels
                .iter()
                .all(|label| label == "Country name unavailable")
        }));
    }
    let without_content = reply(vec![casualty_frame(100, Some((10, 0)), 0)], &params(1));
    assert_eq!(without_content.unknown_label, "Unknown attacker");
    assert!(
        without_content
            .series
            .iter()
            .all(|series| series.labels == ["Country name unavailable"])
    );
}

#[test]
fn changing_selections_reuses_one_branch_scan_with_content_loaded() {
    let source = opened(research_replay((0..35).map(scale_frame).collect()));
    let (context, _content_folder) = loaded_content(&source);
    let cached = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    for n in [1, 30, 10] {
        let r =
            WarCasualties::load(&source, &params(n), &context, &AtomicBool::new(false)).unwrap();
        assert_eq!(members(&r, 34).len(), n as usize);
        assert!(
            r.series
                .iter()
                .all(|s| s.labels.iter().all(|l| l.starts_with("Fixture country")))
        );
        assert_eq!(r.unknown_label, "Fixture unknown losses");
    }
    assert!(Arc::ptr_eq(
        &cached,
        &scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap()
    ));
    // Unchanged naming operands are kept once per tag.
    assert_eq!(cached.names.len(), 40);
    assert!(!cached.has_gap());
}

#[test]
fn switching_branches_keeps_one_scan_per_branch() {
    let mut second = casualty_frame(0, Some((0, 0)), 0);
    second.branch = 2;
    let mut third = casualty_frame(0, Some((0, 0)), 0);
    third.branch = 3;
    let source = opened(
        Replay::from_records(
            CAMPAIGN.into(),
            &[
                records(1, vec![casualty_frame(0, Some((0, 0)), 0)]),
                records(2, vec![second]),
                records(3, vec![third]),
            ],
        )
        .unwrap(),
    );
    let (context, _content_folder) = loaded_content(&source);
    WarCasualties::load(&source, &chosen(), &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        source.state::<LabelScans>().lock().unwrap().entries.len(),
        1
    );
    let scanned = scanned_branch(&source, 2, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        source.state::<LabelScans>().lock().unwrap().entries.len(),
        1
    );
    let first = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert_eq!(first.country_at(&chosen().tags[0], 0).unwrap().taken, 0);
    assert_eq!(source.state::<Scans>().lock().unwrap().entries.len(), 2);
    assert!(Arc::ptr_eq(
        &scanned,
        &scanned_branch(&source, 2, &AtomicBool::new(false)).unwrap()
    ));
    let mut second_request = chosen();
    second_request.branch_id = "2".into();
    WarCasualties::load(&source, &second_request, &context, &AtomicBool::new(false)).unwrap();
    let mut third_request = chosen();
    third_request.branch_id = "3".into();
    WarCasualties::load(&source, &third_request, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(source.state::<Scans>().lock().unwrap().entries.len(), 3);
    assert_eq!(
        source.state::<LabelScans>().lock().unwrap().entries.len(),
        3
    );
    let kept = scanned_branch(&source, 1, &AtomicBool::new(false)).unwrap();
    assert!(Arc::ptr_eq(&first, &kept));
    assert!(scanned_branch(&source, 999, &AtomicBool::new(false)).is_err());
}

#[test]
fn war_casualties_waiter_can_cancel_without_blocking_another_branch() {
    let mut second = casualty_frame(100, Some((10, 0)), 0);
    second.branch = 2;
    let source = opened(
        Replay::from_records(
            CAMPAIGN.into(),
            &[
                records(1, vec![casualty_frame(100, Some((10, 0)), 0)]),
                records(2, vec![second]),
            ],
        )
        .unwrap(),
    );
    let flight = Arc::new(InFlight::default());
    source
        .state::<Scans>()
        .lock()
        .unwrap()
        .in_flight
        .insert(1, Arc::clone(&flight));
    let cancelled = AtomicBool::new(false);
    let (started, begun) = std::sync::mpsc::channel();
    let (finished, done) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            started.send(()).unwrap();
            let result = scanned_branch(&source, 1, &cancelled);
            finished.send(result.err().unwrap().kind()).unwrap();
        });
        begun
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let other = scanned_branch(&source, 2, &AtomicBool::new(false)).unwrap();
        assert!(other.country_at(&chosen().tags[0], 100).is_some());
        cancelled.store(true, Ordering::Release);
        assert_eq!(
            done.recv_timeout(std::time::Duration::from_secs(1))
                .unwrap(),
            io::ErrorKind::Interrupted
        );
    });
    assert!(flight.result.lock().unwrap().is_none());
}

#[test]
fn war_casualties_stretch_start_waits_for_read_and_nonread_endpoints_do_not_confirm() {
    for gap in [false, true] {
        let mut scan = Scan::new();
        let first = scan.country((1, 100));
        let second = scan.country((2, 200));
        let live = BTreeMap::from([
            ("opaque first".into(), first),
            ("opaque second".into(), second),
        ]);
        for hour in 100..=145 {
            let read = hour == 102 || hour == 119;
            scan.begin_hour(hour, &live, hour == 100, read);
            if read {
                scan.relation(first, second, 90, [10, 20, 0, 0]);
            }
            scan.end_hour(read);
            if hour == 101 {
                assert!(scan.tracks.is_empty());
            }
        }
        // A non-read endpoint can extend a hold but cannot create a read date.
        if gap {
            scan.gap();
        }
        let cached = scan.finish(&AtomicBool::new(false)).unwrap();
        assert!(cached.country_at("opaque first", 101).is_none());
        assert_eq!(
            cached.country_at("opaque first", 102).unwrap().read_hour,
            102
        );
        assert_eq!(
            cached.country_at("opaque first", 118).unwrap().read_hour,
            102
        );
        assert_eq!(
            cached.country_at("opaque first", 145).unwrap().read_hour,
            119
        );
        let product = cached
            .reply(
                &CasualtyParams {
                    branch_id: "1".into(),
                    tags: vec!["opaque first".into()],
                    top: None,
                },
                &AtomicBool::new(false),
                None,
                UNKNOWN_LABEL,
                Vec::new(),
            )
            .unwrap();
        assert_eq!(reads(&product.series[0]), [102, 119]);
        assert_eq!(
            product.series[0].ends,
            [(0, 118, Some(102)), (1, 145, Some(119))]
        );
    }
}

#[test]
fn war_casualties_returning_holder_waits_for_its_own_read() {
    let mut scan = Scan::new();
    let former = scan.country((1, 100));
    let replacement = scan.country((1, 101));
    let opponent = scan.country((2, 200));
    for (hour, holder, read) in [
        (100, former, true),
        (101, replacement, true),
        (102, former, false),
    ] {
        let live = BTreeMap::from([
            ("opaque first".into(), holder),
            ("opaque second".into(), opponent),
        ]);
        scan.begin_hour(hour, &live, true, read);
        if read {
            scan.relation(holder, opponent, hour, [10, 20, 0, 0]);
        }
        scan.end_hour(read);
    }
    let cached = scan.finish(&AtomicBool::new(false)).unwrap();
    assert!(cached.country_at("opaque first", 102).is_none());
    let request = CasualtyParams {
        branch_id: "1".into(),
        tags: vec!["opaque first".into()],
        top: None,
    };
    let product = cached
        .reply(
            &request,
            &AtomicBool::new(false),
            None,
            UNKNOWN_LABEL,
            Vec::new(),
        )
        .unwrap();
    assert_eq!(product.series.len(), 2);
    assert_eq!(product.series[0].ends[0].1, 100);
    assert_eq!(product.series[1].ends[0].1, 101);
}

#[test]
fn casualty_pair_reply_keeps_every_native_country_without_a_selection_cap() {
    let source = opened(research_replay(vec![scale_frame(1)]));
    let reply = pairs::CasualtyPairs::load(
        &source,
        &pairs::CasualtyPairsParams {
            branch_id: "1".into(),
            tags: Vec::new(),
            between: false,
            window: Some(0),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(reply.pairs.len(), 40);
    let countries: HashSet<_> = reply
        .pairs
        .iter()
        .flat_map(|pair| [&pair.first_country, &pair.second_country])
        .collect();
    assert_eq!(countries.len(), 40);
    let total: i128 = reply
        .pairs
        .iter()
        .map(|pair| {
            let run = &pair.runs[0];
            run.first_taken.parse::<i128>().unwrap() + run.second_taken.parse::<i128>().unwrap()
        })
        .sum();
    assert_eq!(total, 40 * 41);
}
