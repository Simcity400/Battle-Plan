use crate::test_fixtures::{CAMPAIGN, records};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::sync::atomic::AtomicBool;

use super::test_fixtures::{experience_frame, renamed_frame, slot_base};

#[test]
fn banked_reasks_share_first_read_observations_across_measures_and_exact_block_folds() {
    use super::{SAMPLE_TABLES, SAMPLED_MEASURES, Sampler};
    use crate::prepared::{
        coverage::{Coverage, SCHEMA},
        query::{observations, progressive},
    };
    use crate::test_fixtures::{continuation_records, file_source, replay};
    let mut recording = replay(vec![
        experience_frame(0, true, 9001),
        experience_frame(1, true, 9001),
    ]);
    for block in 1..16 {
        recording = recording
            .append(&[continuation_records(
                &recording,
                1,
                vec![
                    experience_frame(block * 2, true, 9001),
                    experience_frame(block * 2 + 1, true, 9001),
                ],
            )])
            .unwrap();
    }
    let (source, root) = file_source(recording);
    let flag = AtomicBool::new(false);
    let key = serde_json::to_string(&("opaque!tag", 7_u32, 9001_u32)).unwrap();
    let mut cold = None;
    for last in [None, Some(1), Some(3)] {
        let coverage = last.map(|last| Coverage {
            schema: SCHEMA,
            first_hour: slot_base(),
            last_hour: slot_base() + last,
            complete: false,
        });
        for measure in SAMPLED_MEASURES {
            let requested = vec![
                (key.clone(), measure.to_owned()),
                (
                    key.clone(),
                    crate::prepared_measure::query::READ_HOUR.to_owned(),
                ),
            ];
            let sampler = Sampler::new(source.replay_ref().format);
            let ask = || {
                progressive::tail_first_read(
                    &source,
                    "country-experience:exact",
                    1,
                    coverage.as_ref(),
                    SAMPLE_TABLES,
                    replay::cadence::STAGGER_PERIOD,
                    &requested,
                    &Sampler::cache_keys(&requested),
                    (slot_base(), slot_base() + 31),
                    &|frame, _, requested| sampler.sample(frame, requested),
                    &|block| Sampler::first_block(&source, block, &requested, &flag),
                    &flag,
                )
            };
            let (reply, refused) = if cold.is_some() {
                progressive::without_block_decodes(ask)
            } else {
                (ask(), false)
            };
            assert!(!refused, "held first reads stay on the prepared lane");
            let reply = reply.unwrap();
            assert!(!reply.1.is_empty());
            for run in &reply.0[&(key.clone(), measure.to_owned())] {
                let sampled = (run.first_hour - slot_base()) / 2 * 2;
                let expected = match measure {
                    "army-banked" => i128::from(sampled) * 32_768,
                    "navy-banked" => -i128::from(sampled),
                    _ => 0,
                };
                assert_eq!(run.value, expected);
            }
            if measure == "army-banked" && last.is_none() {
                cold = Some(reply);
            }
        }
    }
    assert!(cold.is_some());
    let (bytes, bound, reads) = observations::usage(&source, "country-experience:first-read");
    assert_eq!(
        reads, 16,
        "all three measures and the shrinking tails use one pass"
    );
    assert!(bytes <= bound);
    let reached = slot_base() + 31;
    for measure in SAMPLED_MEASURES {
        let requested = vec![
            (key.clone(), measure.to_owned()),
            (
                key.clone(),
                crate::prepared_measure::query::READ_HOUR.into(),
            ),
        ];
        let mut previous = None;
        for _ in 0..5 {
            let sampler = Sampler::new(source.replay_ref().format);
            let reply = progressive::state_tail_recorded(
                &source,
                "country-experience:exact",
                1,
                None,
                SAMPLE_TABLES,
                replay::cadence::STAGGER_PERIOD,
                &requested,
                &Sampler::cache_keys(&requested),
                &[reached],
                &|frame, _, requested| sampler.sample(frame, requested),
                &flag,
            )
            .unwrap();
            let expected = match measure {
                "army-banked" => 31 * 32_768,
                "navy-banked" => -31,
                _ => 0,
            };
            assert_eq!(
                reply.0[&(key.clone(), measure.to_owned())][0].value,
                expected
            );
            if let Some(previous) = &previous {
                assert_eq!(&reply, previous);
            }
            previous = Some(reply);
        }
    }
    assert_eq!(
        observations::usage(&source, "country-experience:exact").2,
        1
    );
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(feature = "slow-tests")]
#[test]
fn first_read_observations_scale_to_five_thousand_native_holders() {
    use super::{SAMPLED_MEASURES, Sampler};
    use replay::rows::CountryExperienceRow;
    let mut frame = experience_frame(0, true, 9001);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (0..5000)
            .map(|i| CountryRow {
                id: 10000 + i,
                canonical_slot: i + 1,
                tag: format!("holder {i}").into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (0..5000)
            .map(|i| CountryLinkRow {
                id: 10000 + i,
                canonical_slot: i + 1,
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryExperience.index()] =
        TableView::Complete(TableRows::CountryExperience(
            (0..5000)
                .map(|i| CountryExperienceRow {
                    country: 10000 + i,
                    army_raw: i64::from(i),
                    navy_raw: -i64::from(i),
                    air_raw: 2 * i64::from(i),
                    ..Default::default()
                })
                .collect(),
        ));
    let (source, root) =
        crate::test_fixtures::file_source(crate::test_fixtures::replay(vec![frame]));
    let flag = AtomicBool::new(false);
    for measure in SAMPLED_MEASURES {
        let requested: Vec<_> = (0..5000)
            .map(|i| {
                (
                    serde_json::to_string(&(format!("holder {i}"), i + 1, 10000 + i)).unwrap(),
                    measure.to_owned(),
                )
            })
            .collect();
        let sample =
            Sampler::first_block(&source, &source.replay_ref().index[0], &requested, &flag)
                .unwrap();
        for (i, key) in requested.iter().enumerate() {
            let expected = match measure {
                "army-banked" => i as i128,
                "navy-banked" => -(i as i128),
                _ => 2 * i as i128,
            };
            assert_eq!(
                sample[key],
                (Some((i as u32 + 1, 10000 + i as u32)), Some(expected))
            );
        }
    }
    assert_eq!(
        crate::prepared::query::observations::usage(&source, "country-experience:first-read").2,
        1
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn experience_reads_hold_and_change_requires_a_read_exactly_one_day_earlier() {
    use crate::products::country_experience::scan;
    let read_hours = [-3, 0, 7, 21, 24, 31, 48, 52];
    let frames = (-3..=53)
        .map(|hour| experience_frame(hour, read_hours.contains(&hour), 9001))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(
        &replay,
        1,
        &["opaque!tag", "missing"],
        &AtomicBool::new(false),
    )
    .unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert!(scanned["missing"].reads.is_empty());
    assert_eq!(
        reads
            .iter()
            .map(|read| (
                read.first_hour - slot_base(),
                read.last_hour - slot_base(),
                read.starts_segment,
                read.change.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            (-3, -1, true, false),
            (0, 6, false, false),
            (7, 20, false, false),
            (21, 23, false, true),
            (24, 30, false, true),
            (31, 47, false, true),
            (48, 51, false, true),
            (52, 53, false, false)
        ]
    );
    assert_eq!(reads[2].army_raw, (7 * 32_768).to_string());
    for read in reads.iter().filter(|read| read.change.is_some()) {
        let change = read.change.as_ref().unwrap();
        assert_eq!(change.army_raw, (24 * 32_768).to_string());
        assert_eq!(change.navy_raw, "-24");
        assert_eq!(change.air_raw, "0");
        assert!(read.change_unknown.is_none());
    }
    assert!(reads[7].change_unknown.is_some());
}

#[test]
fn experience_gaps_and_tag_holder_changes_break_carry_and_the_next_change() {
    use crate::products::country_experience::scan;
    let frames = (0..=97)
        .filter(|hour| *hour != 12)
        .map(|hour| experience_frame(hour, hour % 24 == 0, if hour < 49 { 9001 } else { 9002 }))
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert_eq!(
        reads
            .iter()
            .map(|read| (
                read.first_hour - slot_base(),
                read.last_hour - slot_base(),
                read.starts_segment,
                read.change.is_some()
            ))
            .collect::<Vec<_>>(),
        [
            (0, 11, true, false),
            (24, 47, true, false),
            (48, 48, false, true),
            (72, 95, true, false),
            (96, 97, false, true)
        ]
    );
}

#[test]
fn experience_incomplete_tables_unresolved_links_and_missing_rows_are_not_zero() {
    use crate::products::country_experience::scan;
    for failure in 0..4 {
        let frames = (0..=49)
            .map(|hour| {
                let mut frame = experience_frame(hour, hour % 24 == 0, 9001);
                if hour == 12 {
                    let table = match failure {
                        0 => TableId::CountryExperience,
                        1 => TableId::Countries,
                        _ => TableId::CountryLinks,
                    };
                    *frame.tables[table.index()] = if failure == 3 {
                        TableView::Complete(TableRows::CountryLinks(Vec::new()))
                    } else {
                        TableView::Unavailable {
                            reason: "fixture".into(),
                        }
                    };
                }
                if hour == 24 {
                    *frame.tables[TableId::CountryExperience.index()] =
                        TableView::Complete(TableRows::CountryExperience(Vec::new()));
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
        let reads = &scanned["opaque!tag"].reads;
        assert_eq!(
            reads
                .iter()
                .map(|read| (
                    read.first_hour - slot_base(),
                    read.last_hour - slot_base(),
                    read.starts_segment
                ))
                .collect::<Vec<_>>(),
            [(0, 11, true), (48, 49, true)]
        );
        assert!(reads[1].change.is_none());
    }
}

#[test]
fn experience_change_widens_signed_raws_before_subtracting_and_requests_share_limits() {
    use crate::products::{
        Product,
        country_experience::{CountryExperience, ExperienceParams, scan},
    };
    let frames = (0..=24)
        .map(|hour| {
            let mut frame = experience_frame(hour, hour % 24 == 0, 9001);
            if let TableView::Complete(TableRows::CountryExperience(rows)) =
                &mut *frame.tables[TableId::CountryExperience.index()]
            {
                rows[0].army_raw = if hour == 0 { i64::MIN } else { i64::MAX };
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned["opaque!tag"].reads[1]
            .change
            .as_ref()
            .unwrap()
            .army_raw,
        "18446744073709551615"
    );
    assert!(scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(true)).is_err());
    let params = |branch: &str, tags| ExperienceParams {
        branch_id: branch.into(),
        tags,
    };
    assert!(CountryExperience::validate(&params("1", vec!["opaque!tag".into()])).is_ok());
    assert!(CountryExperience::validate(&params("1", vec![])).is_err());
    assert!(CountryExperience::validate(&params("bad", vec!["x".into()])).is_err());
    assert!(CountryExperience::validate(&params("1", vec!["x\0".into()])).is_err());
}

#[test]
fn ambiguous_experience_aliases_break_carry_even_between_reads() {
    use crate::products::country_experience::scan;
    for ambiguous_hour in [12, 24] {
        let frames = (0..=49)
            .map(|hour| {
                let mut frame = experience_frame(hour, hour % 24 == 0, 9001);
                if hour == ambiguous_hour
                    && let TableView::Complete(TableRows::CountryExperience(rows)) =
                        &mut *frame.tables[TableId::CountryExperience.index()]
                {
                    let mut duplicate = rows[0].clone();
                    duplicate.country = 9001;
                    duplicate.army_raw = 999;
                    rows.push(duplicate);
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
        let reads = &scanned["opaque!tag"].reads;
        assert_eq!(reads[0].last_hour, slot_base() + ambiguous_hour - 1);
        let resumed = reads
            .iter()
            .find(|read| read.first_hour > slot_base() + ambiguous_hour)
            .unwrap();
        assert!(resumed.starts_segment);
        assert!(resumed.change.is_none());
    }
}

#[test]
fn experience_comparisons_and_cache_keep_every_requested_country() {
    use crate::products::{
        LoadContext, Product,
        country_experience::{CountryExperience, ExperienceParams},
    };
    use replay::rows::CountryExperienceRow;
    use std::sync::Arc;

    let tags: Vec<_> = (0..5000)
        .map(|index| format!("mod country {index}"))
        .collect();
    let mut frame = experience_frame(0, true, 9001);
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        tags.iter()
            .enumerate()
            .map(|(index, tag)| CountryRow {
                id: index as u32 + 100,
                canonical_slot: index as u32 + 1,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        tags.iter()
            .enumerate()
            .map(|(index, _)| CountryLinkRow {
                id: index as u32 + 100,
                canonical_slot: index as u32 + 1,
            })
            .collect(),
    ));
    *frame.tables[TableId::CountryExperience.index()] =
        TableView::Complete(TableRows::CountryExperience(
            tags.iter()
                .enumerate()
                .map(|(index, _)| CountryExperienceRow {
                    country: index as u32 + 100,
                    army_raw: -(index as i64),
                    ..Default::default()
                })
                .collect(),
        ));
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "experience".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = ExperienceParams {
        branch_id: "1".into(),
        tags,
    };
    assert!(CountryExperience::validate(&params).is_ok());
    let context = LoadContext { content: None };
    let first =
        CountryExperience::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(first.series.len(), params.tags.len());
    for (index, series) in first.series.iter().enumerate() {
        assert_eq!(series.tag, params.tags[index]);
        assert_eq!(series.reads[0].army_raw, (-(index as i64)).to_string());
        assert_eq!(series.reads[0].read_hour, slot_base());
        assert_eq!(
            series.identities.as_ref().unwrap()[0].country_id,
            index as u32 + 100
        );
    }
    // A cancelled scan would refuse. A fully cached comparison needs no new scan.
    let cached =
        CountryExperience::load(&source, &params, &context, &AtomicBool::new(true)).unwrap();
    assert_eq!(cached.series, first.series);
}

#[test]
fn daily_gain_is_positive_awards_even_when_banked_change_is_negative_or_capped() {
    use crate::products::country_experience::scan;
    use replay::rows::CountryExperienceGainRow;
    let frames = (0..=48)
        .map(|hour| {
            let mut frame = experience_frame(hour, false, 9001);
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(vec![
                    CountryExperienceGainRow {
                        country: 11,
                        instance: 77,
                        complete: u8::from(hour > 0),
                        army_raw: if hour == 17 { 16 * 32_768 } else { 0 },
                        navy_raw: 32_768,
                        air_raw: i64::MAX,
                    },
                ]));
            if let TableView::Complete(TableRows::CountryExperience(rows)) =
                &mut *frame.tables[TableId::CountryExperience.index()]
            {
                rows[0].army_raw = if hour == 0 { 20 * 32_768 } else { 19 * 32_768 };
                rows[0].navy_raw = 500 * 32_768; // Cap: gain occurs without any bank increase.
            }
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert!(reads[0].gain.is_none());
    let first = reads[1].gain.as_ref().unwrap();
    assert_eq!(first.army_raw, (16 * 32_768).to_string());
    assert_eq!(first.navy_raw, (24 * 32_768).to_string());
    assert_eq!(first.air_raw, (24_i128 * i128::from(i64::MAX)).to_string());
    assert_eq!(
        reads[1].change.as_ref().unwrap().army_raw,
        (-32_768).to_string()
    );
    assert_eq!(reads[1].change.as_ref().unwrap().navy_raw, "0");
    assert_eq!(reads[2].gain.as_ref().unwrap().army_raw, "0");
}

#[test]
fn gross_gain_needs_all_hourly_intervals_and_one_native_instance() {
    use crate::products::country_experience::scan;
    use replay::rows::CountryExperienceGainRow;
    for failure in 0..4 {
        let frames = (0..=72)
            .map(|hour| {
                let mut frame = experience_frame(hour, false, 9001);
                *frame.tables[TableId::CountryExperienceGains.index()] =
                    TableView::Complete(TableRows::CountryExperienceGains(vec![
                        CountryExperienceGainRow {
                            country: 11,
                            instance: if failure == 2 && hour >= 12 { 88 } else { 77 },
                            complete: u8::from(!(hour == 12 && failure == 3)),
                            army_raw: 1,
                            ..Default::default()
                        },
                    ]));
                if hour == 12 && failure < 2 {
                    *frame.tables[TableId::CountryExperienceGains.index()] = if failure == 0 {
                        TableView::Unavailable {
                            reason: "synthetic missed observation".into(),
                        }
                    } else {
                        TableView::Complete(TableRows::CountryExperienceGains(Vec::new()))
                    };
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
        let reads = &scanned["opaque!tag"].reads;
        assert!(reads[1].gain.is_none());
        assert!(reads[1].gain_unknown.is_some());
        if failure == 2 {
            assert!(reads[1].starts_segment);
            assert!(reads[1].change.is_none());
            assert_eq!(reads[0].last_hour, slot_base() + 11);
        }
        assert_eq!(reads[2].gain.as_ref().unwrap().army_raw, "24");
        assert_eq!(reads[3].gain.as_ref().unwrap().army_raw, "24");
    }
}

#[test]
fn a_births_partial_interval_cannot_complete_a_native_day_of_gain() {
    use crate::products::country_experience::scan;
    use replay::rows::CountryExperienceGainRow;
    let frames = (0..=25)
        .map(|hour| {
            let mut frame = experience_frame(hour, hour >= 24, 9001);
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(vec![
                    CountryExperienceGainRow {
                        country: 11,
                        instance: 77,
                        complete: u8::from(hour >= 2),
                        army_raw: 1,
                        ..Default::default()
                    },
                ]));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["opaque!tag"], &AtomicBool::new(false)).unwrap();
    let reads = &scanned["opaque!tag"].reads;
    assert!(
        reads
            .iter()
            .find(|read| read.first_hour == slot_base() + 24)
            .unwrap()
            .gain
            .is_none()
    );
    assert_eq!(
        reads
            .iter()
            .find(|read| read.first_hour == slot_base() + 25)
            .unwrap()
            .gain
            .as_ref()
            .unwrap()
            .army_raw,
        "24"
    );
}

#[test]
fn renaming_keeps_the_daily_read_net_change_and_native_gain_day() {
    use crate::products::country_experience::scan;
    let frames = (0..=48).map(renamed_frame).collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(
        &replay,
        1,
        &["opaque!tag", "renamed"],
        &AtomicBool::new(false),
    )
    .unwrap();
    let carried = &scanned["renamed"].reads[0];
    assert_eq!(carried.first_hour, slot_base() + 12);
    assert_eq!(carried.last_hour, slot_base() + 23);
    assert_eq!(carried.read_hour, slot_base());
    assert_eq!(carried.army_raw, "0");
    let read = &scanned["renamed"].reads[1];
    assert_eq!(read.read_hour, slot_base() + 24);
    assert!(!read.starts_segment);
    assert_eq!(
        read.change.as_ref().unwrap().army_raw,
        (24 * 32_768).to_string()
    );
    assert_eq!(read.gain.as_ref().unwrap().army_raw, "24");
    assert_eq!(scanned["opaque!tag"].reads[0].last_hour, slot_base() + 11);
    let replacement = &scanned["opaque!tag"].reads[1];
    assert!(replacement.starts_segment);
    assert!(replacement.change.is_none());
    assert_eq!(replacement.army_raw, (999 * 32_768).to_string());
}

#[test]
fn alias_renames_still_break_the_gain_day_and_carry_at_an_identity_gap() {
    use crate::products::country_experience::scan;
    let frames = (0..=48)
        .filter(|hour| *hour != 16)
        .map(renamed_frame)
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let scanned = scan(&replay, 1, &["renamed"], &AtomicBool::new(false)).unwrap();
    let reads = &scanned["renamed"].reads;
    assert_eq!(reads[0].last_hour, slot_base() + 15);
    assert_eq!(reads[1].first_hour, slot_base() + 24);
    assert!(reads[1].starts_segment);
    assert!(reads[1].change.is_none());
    assert!(reads[1].gain.is_none());
    assert_eq!(reads[2].gain.as_ref().unwrap().army_raw, "24");
}

#[test]
fn manual_alias_projection_keeps_selected_holders_and_excludes_unrelated_alias_reuse() {
    use crate::products::{
        LoadContext, Product,
        country_experience::{CountryExperience, ExperienceParams},
    };
    use std::sync::Arc;
    let frames = (0..=96).map(renamed_frame).collect();
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap());
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "experience-aliases".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = ExperienceParams {
        branch_id: "1".into(),
        tags: vec!["opaque!tag".into()],
    };
    let context = LoadContext { content: None };
    let reply =
        CountryExperience::load(&source, &params, &context, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| series.tag.as_str())
            .collect::<Vec<_>>(),
        ["opaque!tag", "renamed", "third"]
    );
    let renamed = &reply.series[1];
    assert_eq!(renamed.reads.last().unwrap().last_hour, slot_base() + 48);
    assert!(
        renamed
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .all(|identity| identity.country_id == 9001)
    );
    assert_eq!(
        reply.series[0]
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .map(|identity| identity.country_id)
            .collect::<Vec<_>>(),
        [9001, 9002]
    );
    let third = &reply.series[2].reads[0];
    assert_eq!(third.first_hour, slot_base() + 49);
    assert_eq!(third.read_hour, slot_base() + 48);
    assert_eq!(third.gain.as_ref().unwrap().army_raw, "24");
    let reused = CountryExperience::load(
        &source,
        &ExperienceParams {
            branch_id: "1".into(),
            tags: vec!["renamed".into()],
        },
        &context,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reused_alias = reused
        .series
        .iter()
        .find(|series| series.tag == "renamed")
        .unwrap();
    assert_eq!(
        reused_alias.reads.last().unwrap().last_hour,
        slot_base() + 96
    );
    assert!(
        reused_alias
            .identities
            .as_ref()
            .unwrap()
            .iter()
            .any(|identity| identity.country_id == 9003)
    );
}

#[test]
fn exact_banked_reads_find_a_supplier_several_short_blocks_back_and_reuse_it() {
    use super::{SAMPLE_TABLES, Sampler};
    use crate::prepared::query::{observations, progressive};
    use crate::test_fixtures::{continuation_records, file_source, replay};
    let frames = |first| {
        (first..first + 6)
            .map(|hour| {
                // Shift the daily slot to offset 17, several six-hour blocks before the target.
                let mut frame = experience_frame(hour, hour == 17, 9001);
                frame.hour = slot_base() + 7 + hour;
                frame
            })
            .collect()
    };
    let mut recording = replay(frames(0));
    for first in [6, 12, 18, 24, 30] {
        recording = recording
            .append(&[continuation_records(&recording, 1, frames(first))])
            .unwrap();
    }
    let (source, root) = file_source(recording);
    let flag = AtomicBool::new(false);
    let key = serde_json::to_string(&("opaque!tag", 7_u32, 9001_u32)).unwrap();
    let requested = vec![(key.clone(), "army-banked".into())];
    let ask = |hour| {
        let sampler = Sampler::new(source.replay_ref().format);
        progressive::state_tail_recorded(
            &source,
            "country-experience:exact",
            1,
            None,
            SAMPLE_TABLES,
            replay::cadence::STAGGER_PERIOD,
            &requested,
            &Sampler::cache_keys(&requested),
            &[hour],
            &|frame, _, requested| sampler.sample(frame, requested),
            &flag,
        )
        .unwrap()
    };
    let cold = ask(slot_base() + 37);
    assert_eq!(
        cold.0[&(key.clone(), "army-banked".into())][0].value,
        17 * 32_768
    );
    assert_eq!(
        ask(slot_base() + 38).0[&(key, "army-banked".into())][0].value,
        17 * 32_768
    );
    assert_eq!(
        observations::usage(&source, "country-experience:exact").2,
        1
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gain_runs_visit_only_changed_countries_and_banked_reads() {
    use super::Scan;
    use crate::{prepared::frame_changes::FrameChangeTracker, test_fixtures::scale_frame};
    use replay::rows::{CountryExperienceGainRow, CountryExperienceRow};
    use std::{collections::VecDeque, sync::Arc};
    const COUNT: u32 = 32;
    const HOURS: i32 = 73;
    let frames = (0..HOURS)
        .map(|offset| {
            let mut frame = scale_frame(1, slot_base() + offset, COUNT);
            *frame.tables[TableId::CountryExperience.index()] =
                TableView::Complete(TableRows::CountryExperience(
                    (0..COUNT)
                        .map(|index| CountryExperienceRow {
                            country: 100_000 + index,
                            army_raw: 100,
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(
                    (0..COUNT)
                        .map(|index| CountryExperienceGainRow {
                            country: 100_000 + index,
                            instance: if index == 0 && offset >= 45 { 2 } else { 1 },
                            complete: u8::from(index != 0 || offset != 37),
                            army_raw: if index == 0 {
                                1 + i64::from(offset % 2)
                            } else {
                                7
                            },
                            air_raw: i64::MAX,
                            ..Default::default()
                        })
                        .collect(),
                ));
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    let mut reference: Vec<VecDeque<(i32, u32, [i64; 3])>> = vec![VecDeque::new(); COUNT as usize];
    let mut reads = 0;
    replay
        .read_block_alone(
            &replay.index[0],
            &TableId::ALL,
            |raw, frame| {
                // Independent hourly queues authenticate every interval, including the
                // partial interval and the change of native instance.
                for (index, row) in frame
                    .complete::<replay::rows::tables::CountryExperienceGains>()
                    .unwrap()
                    .iter()
                    .enumerate()
                {
                    let queue = &mut reference[index];
                    if row.complete != 1
                        || queue
                            .back()
                            .is_some_and(|(_, instance, _)| *instance != row.instance)
                    {
                        queue.clear();
                    }
                    if row.complete == 1 {
                        queue.push_back((
                            frame.hour,
                            row.instance,
                            [row.army_raw, row.navy_raw, row.air_raw],
                        ));
                        while queue
                            .front()
                            .is_some_and(|(hour, _, _)| frame.hour - *hour >= 24)
                        {
                            queue.pop_front();
                        }
                    }
                }
                let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                scan.step(frame, replay.format, Some(&changes), |_, identity, read| {
                    if read.read_hour == frame.hour {
                        let queue = &reference[identity.0 as usize];
                        let expected = (queue.len() == 24).then(|| {
                            queue.iter().fold([0_i128; 3], |mut sum, (_, _, values)| {
                                for index in 0..3 {
                                    sum[index] += i128::from(values[index]);
                                }
                                sum
                            })
                        });
                        assert_eq!(
                            read.gain, expected,
                            "country {identity:?} at {}",
                            frame.hour
                        );
                        reads += 1;
                    }
                    Ok(())
                })
            },
            || Ok(()),
        )
        .unwrap();
    assert_eq!(reads, COUNT as usize * 4);
    assert!(
        scan.gain_updates() <= COUNT as usize * 4 + HOURS as usize,
        "unchanged gain owners were revisited: {} updates",
        scan.gain_updates()
    );
    assert!(
        scan.seen
            .iter()
            .filter(|(identity, _)| identity.0 != 0)
            .all(|(_, state)| state.gains.len() == 1)
    );
}

#[test]
fn quiet_gain_runs_advance_without_visiting_countries_and_resume_at_a_daily_read() {
    use super::Scan;
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay::rows::CountryExperienceGainRow;
    let mut first = experience_frame(0, false, 9001);
    *first.tables[TableId::CountryExperienceGains.index()] =
        TableView::Complete(TableRows::CountryExperienceGains(vec![
            CountryExperienceGainRow {
                country: 11,
                instance: 77,
                complete: 1,
                army_raw: 3,
                ..Default::default()
            },
        ]));
    let format = replay::format::FormatVersion::CURRENT;
    let mut hourly = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    let changes = tracker.observe(&first, format);
    hourly
        .step(&first, format, Some(&changes), |_, _, _| Ok(()))
        .unwrap();
    let mut quiet = Scan::default();
    quiet
        .step(&first, format, Some(&changes), |_, _, _| Ok(()))
        .unwrap();
    // Runtime joins are rebuilt at the first hour after loading.
    let mut resumed: Scan = serde_json::from_slice(&serde_json::to_vec(&hourly).unwrap()).unwrap();
    let mut resumed_tracker = FrameChangeTracker::new();
    let mut next = first.clone();
    next.hour += 1;
    let changes = resumed_tracker.observe(&next, format);
    resumed
        .step(&next, format, Some(&changes), |_, _, _| Ok(()))
        .unwrap();
    let ticks = quiet.gain_updates();
    assert!(quiet.advance_quiet(first.hour + 1, first.hour + 23));
    assert_eq!(
        quiet.gain_updates(),
        ticks,
        "a quiet span visits no gain queue"
    );
    for offset in 1..24 {
        let mut next = first.clone();
        next.hour += offset;
        let changes = tracker.observe(&next, format);
        hourly
            .step(&next, format, Some(&changes), |_, _, _| Ok(()))
            .unwrap();
    }
    assert_eq!(
        serde_json::to_value(&quiet).unwrap(),
        serde_json::to_value(&hourly).unwrap()
    );
    assert!(resumed.advance_quiet(first.hour + 2, first.hour + 23));
    let mut daily = first.clone();
    daily.hour += 24;
    let mut found = Vec::new();
    for scan in [&mut hourly, &mut quiet, &mut resumed] {
        scan.step(&daily, format, None, |_, _, read| {
            found.push(read.gain);
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(found, vec![Some([72, 0, 0]); 3]);
    assert_eq!(
        serde_json::to_value(&resumed).unwrap(),
        serde_json::to_value(&hourly).unwrap()
    );
}
