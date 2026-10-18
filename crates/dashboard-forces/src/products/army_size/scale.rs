//! Synthetic scale reports. Wire samples cover separate windows across nine
//! years; fixture construction and encoding stay outside the measured interval.

use super::*;
use replay::spool::Record;
use replay_data::{
    identity::Identity as ReplayIdentity, identity::Roster, identity::SampleBoundary,
    identity::SessionStart, identity::Verification, rows::CampaignRow, rows::CountryRow,
    rows::DivisionCensusRow, rows::DivisionLandInputsRow, rows::DivisionManpowerContributor,
    rows::DivisionTemplateIconRow, rows::LexicalTokenRow, rows::TableRows, rows::TemplateSubunit,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Parentage, frame::DecodedFrame,
    frame::FrameEncoder, frame::Metrics, frame::TableView,
};
use std::{hint::black_box, time::Instant};

const COUNTRIES: u32 = 5_000;
const DIVISIONS: usize = 50_000;
const MAJORS: u32 = 100;
const MAJOR_DIVISIONS: usize = 44_650;
const YEARS: i32 = 9;
const HOURS_PER_YEAR: i32 = 365 * 24;
const WINDOW_HOURS: i32 = 48;
const FIRST_HOUR: i32 = 60_000_000;
const CAMPAIGN: &[u8] = b"synthetic-army-scale";

fn synthetic_tag(country: u32) -> String {
    format!("Synthetic|{country:05}")
}

fn session() -> SessionStart {
    SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 10_000_000,
        sample_boundary: SampleBoundary::EndOfHour,
        identity: ReplayIdentity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: vec![],
            mods: vec![],
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    }
}

fn base_divisions(hour: i32) -> Vec<DivisionRow> {
    (0..DIVISIONS)
        .map(|index| {
            let country = if index < MAJOR_DIVISIONS {
                1 + index as u32 % MAJORS
            } else {
                1 + MAJORS + (index - MAJOR_DIVISIONS) as u32 % (COUNTRIES - MAJORS)
            };
            let expeditionary = index % 20 == 0;
            let host = if expeditionary {
                1 + (country + hour as u32 / 6) % MAJORS
            } else {
                country
            };
            DivisionRow {
                id: ((index as u64 + 1) << 32) | 7,
                roster_owner: host,
                logical_owner: host,
                expeditionary_owner: if expeditionary { country } else { u32::MAX },
                template: 100 + u64::from(country - 1) * 3 + index as u64 % 3,
                hit_points: (200 + index as i64 % 80) * 100_000,
                orders_group: if index % 4 == 0 { 0 } else { 99 },
                ..Default::default()
            }
        })
        .collect()
}

fn update_divisions(rows: &mut [DivisionRow], hour: i32) {
    // One percent has combat/reinforcement HP changes each hour. Keep the
    // active countries concentrated as in a campaign with many small states.
    for row in rows.iter_mut().skip(hour as usize % 101).step_by(101) {
        row.hit_points = (180 + (hour as i64 + (row.id >> 32) as i64) % 100) * 100_000;
    }
    if hour % 6 == 0 {
        for row in rows
            .iter_mut()
            .filter(|row| row.expeditionary_owner != u32::MAX)
            .skip(hour as usize / 6 % 50)
            .step_by(50)
        {
            row.roster_owner = 1 + (row.expeditionary_owner + hour as u32 / 6) % MAJORS;
            row.logical_owner = row.roster_owner;
        }
    }
    if hour % 24 == 0 {
        // Daily 0.1% roster churn: removed handles are replaced and their
        // countries/templates change. The complete inventory remains 50,000.
        let offset = hour as usize / 24 % (MAJOR_DIVISIONS - 50);
        for (index, row) in rows.iter_mut().skip(offset).take(50).enumerate() {
            row.id = ((DIVISIONS as u64 + hour as u64 * 50 + index as u64) << 32) | 7;
            row.roster_owner = 1 + (hour as u32 / 24 + index as u32) % MAJORS;
            row.logical_owner = row.roster_owner;
            row.expeditionary_owner = u32::MAX;
            row.template =
                100 + u64::from(row.roster_owner - 1) * 3 + (hour as u64 / 24 + index as u64) % 3;
        }
    }
}

fn wire_window(year: i32) -> Replay {
    let first = FIRST_HOUR + year * HOURS_PER_YEAR + (year * 193) % (HOURS_PER_YEAR - WINDOW_HOURS);
    let mut rows = base_divisions(first);
    let mut views: [TableView; TableId::ALL.len()] =
        std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside synthetic army fixture".into(),
        });
    views[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow {
            id: CAMPAIGN.into(),
        }]));
    views[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=COUNTRIES)
            .map(|country| CountryRow {
                id: country,
                canonical_slot: country + 10_000,
                tag: synthetic_tag(country).into_bytes(),
                ..Default::default()
            })
            .collect(),
    ));
    views[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        (1..=COUNTRIES)
            .map(|country| CountryLinkRow {
                id: country,
                canonical_slot: country + 10_000,
            })
            .collect(),
    ));
    views[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(
            (100..100 + u64::from(COUNTRIES) * 3)
                .map(|template| DivisionTemplateIconRow {
                    template,
                    subunits: vec![
                        TemplateSubunit {
                            token: 10,
                            local: 0,
                            count: 8,
                        },
                        TemplateSubunit {
                            token: 20,
                            local: 1,
                            count: 2,
                        },
                    ],
                    ..Default::default()
                })
                .collect(),
        ));
    views[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(vec![
        LexicalTokenRow {
            token: 10,
            literal: b"synthetic_line".to_vec(),
        },
        LexicalTokenRow {
            token: 20,
            literal: b"synthetic_support".to_vec(),
        },
    ]));
    let mut frame = DecodedFrame {
        branch: 1,
        hour: first,
        tables: views.map(Into::into),
        metrics: Metrics {
            table_ns: [1; TableId::ALL.len()],
            ..Default::default()
        },
    };
    let mut records = vec![
        Record::SessionStart(session()),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: None,
            first_hour: first,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for hour in first..first + WINDOW_HOURS {
        update_divisions(&mut rows, hour);
        if hour % 24 == 0
            && let TableView::Complete(TableRows::DivisionTemplateIcons(templates)) =
                &mut *frame.tables[TableId::DivisionTemplateIcons.index()]
        {
            for change in 0..templates.len() / 1_000 {
                let index = (hour as usize / 24 + change) % templates.len();
                templates[index].subunits[0].count = 8 + hour / 24 % 3;
            }
        }
        rows.sort_unstable_by_key(|row| row.id);
        let mut census: Vec<_> = (1..=COUNTRIES)
            .map(|country| DivisionCensusRow {
                country,
                ..Default::default()
            })
            .collect();
        for row in &rows {
            let count = &mut census[owner(row) as usize - 1];
            if row.orders_group == 0 {
                count.ungrouped += 1;
            } else {
                count.grouped += 1;
            }
        }
        frame.hour = hour;
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(rows.clone()));
        *frame.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(
                rows.iter()
                    .map(|row| DivisionLandInputsRow {
                        division: row.id,
                        manpower: vec![
                            DivisionManpowerContributor {
                                country: owner(row),
                                amount: 1200 + hour.rem_euclid(7),
                            },
                            DivisionManpowerContributor {
                                country: owner(row),
                                amount: -200,
                            },
                        ],
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(census));
        records.push(Record::Frame(encoder.encode(frame.clone(), false).unwrap()));
    }
    Replay::from_records(CAMPAIGN.into(), &[records]).unwrap()
}

fn percentile(values: &[f64], percentile: usize) -> f64 {
    values[(values.len() * percentile).div_ceil(100).saturating_sub(1)]
}

#[test]
#[ignore = "synthetic performance report; run army_size_scale with --ignored --nocapture"]
fn army_size_scale_including_wire_decode() {
    let mut timing: [Vec<f64>; 2] = std::array::from_fn(|_| Vec::new());
    let mut stages: [[Vec<f64>; 4]; 2] =
        std::array::from_fn(|_| std::array::from_fn(|_| Vec::new()));
    let mut work = [0usize; 2];
    let mut runs = 0;
    for year in 0..YEARS {
        let replay = wire_window(year);
        for detailed in [false, true] {
            for _ in 0..3 {
                let selected = scan_tables(detailed);
                let mut wire_selected = selected.clone();
                wire_selected.extend([
                    TableId::Divisions,
                    TableId::DivisionTemplateIcons,
                    TableId::DivisionLandInputs,
                ]);
                let mut built = Scan::default();
                let mut state = ScanState::new(detailed);
                for block in &replay.index {
                    let mut decoder = FrameDecoder::for_tables(&selected);
                    let wire_start = Instant::now();
                    let frames = replay.read_block_selected(block, &wire_selected).unwrap();
                    let wire_ms = wire_start.elapsed().as_secs_f64() * 1000.0 / frames.len() as f64;
                    for frame in frames {
                        let start = Instant::now();
                        let decoded = decoder.advance(&frame).unwrap();
                        validate_campaign(decoded, &replay).unwrap();
                        let decode = wire_ms + start.elapsed().as_secs_f64() * 1000.0;
                        let start = Instant::now();
                        state.apply_changes(&frame, decoded);
                        let apply = start.elapsed().as_secs_f64() * 1000.0;
                        let start = Instant::now();
                        state.compose(decoded);
                        let composition = start.elapsed().as_secs_f64() * 1000.0;
                        let start = Instant::now();
                        observe_totals(
                            &mut built,
                            decoded,
                            &frame,
                            replay.format,
                            false,
                            &mut state,
                            None,
                        );
                        let countries = start.elapsed().as_secs_f64() * 1000.0;
                        let index = usize::from(detailed);
                        for (stage, value) in [decode, apply, countries, composition]
                            .into_iter()
                            .enumerate()
                        {
                            stages[index][stage].push(value);
                        }
                        timing[index].push(decode + apply + countries + composition);
                    }
                }
                assert_eq!(built.tracks.len(), COUNTRIES as usize);
                assert!(built.divisions);
                if detailed {
                    assert!(built.strength);
                    assert!(built.manpower);
                }
                assert_eq!(built.unreadable, 0);
                assert_eq!(
                    built.tracks[&synthetic_tag(COUNTRIES)].runs[0].first,
                    replay.index[0].first_hour
                );
                for hour in [
                    replay.index[0].first_hour,
                    replay.index.last().unwrap().last_hour,
                ] {
                    let total = |channel: usize| {
                        built
                            .tracks
                            .values()
                            .map(|track| {
                                track
                                    .runs
                                    .iter()
                                    .find(|run| run.first <= hour && hour <= run.last)
                                    .unwrap()
                                    .totals
                                    .values[channel]
                                    .unwrap()
                            })
                            .sum::<i128>()
                    };
                    assert_eq!(total(0), DIVISIONS as i128);
                    if detailed {
                        assert_eq!(total(2), DIVISIONS as i128);
                        assert_eq!(
                            total(6),
                            DIVISIONS as i128 * i128::from(1000 + hour.rem_euclid(7))
                        );
                        assert_eq!(total(7), total(6));
                    }
                }
                if detailed {
                    runs = runs.max(
                        built
                            .tracks
                            .values()
                            .map(|track| track.runs.len())
                            .sum::<usize>(),
                    );
                }
                if let Some(inventory) = state.inventory.as_ref() {
                    work[0] += inventory.applied_divisions;
                    work[1] += inventory.compiled_templates;
                }
                black_box(built);
            }
        }
    }
    for (index, values) in timing.iter_mut().enumerate() {
        values.sort_by(f64::total_cmp);
        println!(
            "army_size_scale countries={COUNTRIES} divisions={DIVISIONS} years={YEARS} sampled_hours={} windows={YEARS} repetitions=3 detailed={} p50_ms_per_built_hour={:.6} p95_ms_per_built_hour={:.6} (individual built hours; block decompression/wire decode amortized across its hours; includes reconstruction, identity projection and history building; excludes fixture encoding)",
            YEARS * WINDOW_HOURS,
            index == 1,
            percentile(values, 50),
            percentile(values, 95)
        );
    }
    for (detailed, stage_values) in stages.iter_mut().enumerate() {
        for (stage, values) in stage_values.iter_mut().enumerate() {
            values.sort_by(f64::total_cmp);
            println!(
                "army_size_scale stage={} detailed={} p50_ms={:.6} p95_ms={:.6}",
                [
                    "decode",
                    "apply_changes",
                    "country_totals_history",
                    "composition"
                ][stage],
                detailed == 1,
                percentile(values, 50),
                percentile(values, 95)
            );
        }
    }
    println!(
        "army_size_scale sparse_work applied_divisions={} compiled_templates={}",
        work[0], work[1]
    );
    println!(
        "army_size_scale peak_sample_runs={runs}; hourly HP=1%, daily roster churn=0.1%, \
        expeditionaries=5% initially, host transfers approximately 0.1% per 6h, \
        3 country-local templates per country, approximately 0.1% templates edited daily; \
        sampled windows are not a full nine-year scan"
    );
}

/// History outside this recorded window is compressed; every active country's
/// HP changes hourly inside it. Reply cost must depend only on this window.
fn window_history(range: &HourSpan) -> Scan {
    let last = FIRST_HOUR + YEARS * HOURS_PER_YEAR - 1;
    let mut scanned = Scan {
        divisions: true,
        strength: true,
        battalions: true,
        ..Default::default()
    };
    for country in 1..=COUNTRIES {
        let mut track = Track {
            segments: vec![HourSpan {
                first_hour: FIRST_HOUR,
                last_hour: last,
            }],
            ..Default::default()
        };
        let active = country <= MAJORS;
        let value = |hour: i32| {
            let day = hour / 24;
            let count = if active {
                397 + (i64::from(country - 1) + i64::from(day)) % i64::from(MAJORS)
            } else {
                1 + i64::from(country - MAJORS <= 450)
            };
            let host = if active {
                397 + (i64::from(country) + i64::from(day)) % i64::from(MAJORS)
            } else {
                count
            };
            Totals {
                values: [
                    Some(i128::from(count)),
                    Some(i128::from(
                        count * 240 * 100_000 + i64::from(hour % 997) * 137,
                    )),
                    Some(i128::from(host)),
                    Some(i128::from(
                        host * 240 * 100_000 + i64::from(hour % 991) * 139,
                    )),
                    Some(i128::from(count * 10)),
                    Some(i128::from(host * 10)),
                    None,
                    None,
                ],
            }
        };
        if active {
            if FIRST_HOUR < range.first_hour {
                track.runs.push(RawRun {
                    first: FIRST_HOUR,
                    last: range.first_hour - 1,
                    starts: true,
                    totals: value(0),
                });
            }
            for hour in range.first_hour..=range.last_hour {
                track.runs.push(RawRun {
                    first: hour,
                    last: hour,
                    starts: hour == FIRST_HOUR,
                    totals: value(hour - FIRST_HOUR),
                });
            }
            if range.last_hour < last {
                track.runs.push(RawRun {
                    first: range.last_hour + 1,
                    last,
                    starts: false,
                    totals: value(0),
                });
            }
        } else {
            track.runs.push(RawRun {
                first: FIRST_HOUR,
                last,
                starts: true,
                totals: value(0),
            });
        }
        for day in range.first_hour / 24..=range.last_hour / 24 {
            let totals = value(day * 24 - FIRST_HOUR);
            let parts = |count: i64| {
                Some(BTreeMap::from([
                    ("synthetic_line".into(), count * 8),
                    ("synthetic_support".into(), count * 2),
                ]))
            };
            track.mix.push(RawMixRun {
                first: (day * 24).max(range.first_hour),
                last: (day * 24 + 23).min(range.last_hour),
                parts: [
                    parts(i64::try_from(totals.values[0].unwrap()).unwrap()),
                    parts(i64::try_from(totals.values[2].unwrap()).unwrap()),
                ],
            });
        }
        scanned.tracks.insert(synthetic_tag(country), track);
    }
    scanned
}

#[test]
#[ignore = "synthetic window reply report; run army_size_scale with --ignored --nocapture"]
fn army_size_scale_window_reply_sizes() {
    // The window is authored by the recording's keyframes, not this product.
    let last = FIRST_HOUR + YEARS * HOURS_PER_YEAR - 1;
    let first = last - replay_frame::KEYFRAME_INTERVAL as i32 + 1;
    let mut observations = vec![super::test_fixtures::frame(FIRST_HOUR)];
    observations.extend((first..=last).map(super::test_fixtures::frame));
    let source = super::tests::opened(observations);
    let block = &source.replay_ref().index[1];
    let range = HourSpan {
        first_hour: block.first_hour,
        last_hour: block.last_hour,
    };
    let scanned = window_history(&range);
    let cancelled = AtomicBool::new(false);
    for measure in [ArmyMeasure::Divisions, ArmyMeasure::Strength] {
        for attribution in [ArmyAttribution::Owner, ArmyAttribution::Host] {
            for top in [10, 30] {
                let params = ArmySizeParams {
                    branch_id: "1".into(),
                    tags: vec![],
                    top: Some(top),
                    measure,
                    count_for: attribution,
                    by_type: false,
                    window: Some(1),
                };
                let (series, series_bytes) =
                    reply_series(black_box(&scanned), &params, &range, None, &cancelled, None)
                        .unwrap();
                let reply = ArmySizeReply {
                    branch_id: "1".into(),
                    available: true,
                    windows: vec![],
                    window: Some(1),
                    interval: Some(range),
                    measures: vec![
                        ArmyMeasure::Divisions,
                        ArmyMeasure::Strength,
                        ArmyMeasure::Battalions,
                    ],
                    types: vec![],
                    series,
                    warnings: vec![],
                };
                for hour in [range.first_hour, range.first_hour + 25, range.last_hour] {
                    assert_eq!(
                        reply
                            .series
                            .iter()
                            .filter(|series| series
                                .runs
                                .iter()
                                .any(|run| run.first_hour <= hour && hour <= run.last_hour))
                            .count(),
                        top
                    );
                }
                assert!(
                    reply
                        .series
                        .iter()
                        .flat_map(|series| &series.runs)
                        .all(|run| range.first_hour <= run.first_hour
                            && run.last_hour <= range.last_hour)
                );
                let json_bytes = dashboard_core::transport::json_len(&reply, usize::MAX).unwrap();
                assert!(json_bytes > series_bytes);
                assert!(json_bytes <= dashboard_base::products::MAX_PRODUCT_REPLY_BYTES);
                println!(
                    "army_size_scale window_reply countries={COUNTRIES} divisions={DIVISIONS} \
                    campaign_years={YEARS} window_hours={} measure={measure:?} attribution={attribution:?} \
                    top={top} json_bytes={json_bytes}; active HP hourly, counts daily",
                    range.last_hour - range.first_hour + 1
                );
                black_box(reply);
            }
        }
    }
    let params = ArmySizeParams {
        branch_id: "1".into(),
        tags: vec![synthetic_tag(1)],
        top: None,
        measure: ArmyMeasure::Battalions,
        count_for: ArmyAttribution::Owner,
        by_type: true,
        window: Some(1),
    };
    let (mut series, _) = reply_series(&scanned, &params, &range, None, &cancelled, None).unwrap();
    let mut warnings = vec![];
    let types = composition_reply(
        &source,
        &LoadContext { content: None },
        &scanned,
        &params,
        &range,
        &mut series,
        &mut warnings,
        &cancelled,
    )
    .unwrap();
    let reply = ArmySizeReply {
        branch_id: "1".into(),
        available: true,
        windows: vec![],
        window: Some(1),
        interval: Some(range),
        measures: vec![
            ArmyMeasure::Divisions,
            ArmyMeasure::Strength,
            ArmyMeasure::Battalions,
        ],
        types,
        series,
        warnings,
    };
    let bytes = dashboard_core::transport::json_len(&reply, usize::MAX).unwrap();
    assert_eq!(reply.types.len(), 2);
    assert_eq!(reply.series.len(), 1);
    assert_eq!(
        reply.series[0]
            .mix
            .as_ref()
            .unwrap()
            .last()
            .unwrap()
            .last_hour,
        range.last_hour
    );
    assert!(bytes <= dashboard_base::products::MAX_PRODUCT_REPLY_BYTES);
    println!(
        "army_size_scale window_battalion_mix groups={} window_hours={} json_bytes={bytes}; \
        daily composition changes, 8 line battalions and 2 support per division",
        reply.types.len(),
        range.last_hour - range.first_hour + 1
    );
}
