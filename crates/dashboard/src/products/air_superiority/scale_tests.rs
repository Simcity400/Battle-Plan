use super::*;
use crate::test_fixtures::{CAMPAIGN, scale_frame, scale_tag, session};
use replay::{
    branches::{BranchReason, BranchStart, Gap, Parentage},
    frame::{FrameEncoder, TableView},
    rows::{AirWingRow, FactionMemberRow, FactionRow, RaidAirScoreRow, TableRows, WarRelationRow},
    spool::Record,
};
use std::{sync::Arc, time::Instant};

struct Scenario {
    countries: u32,
    members: Vec<u32>,
    co_belligerents: u32,
    regions: u32,
    active_regions: u32,
    hours: i32,
    sparse: bool,
    gaps: Vec<(i32, i32)>,
}

struct Fixture {
    replay: Arc<Replay>,
    countries: u32,
    owners: usize,
    score_rows: usize,
    neutral_owners: u32,
    war_pairs: usize,
    gap_hours: usize,
    recorded_hours: usize,
    truncated_hours: usize,
}

fn region_id(index: u32) -> u32 {
    70_000 + index * 3
}

fn fixture_content(regions: u32) -> (i32, usize) {
    // Synthetic content deliberately overrides this define through the stack.
    let base_regions: String = (0..regions - 1)
        .map(|index| {
            format!(
                "strategic_region={{id={} name=fixture_region_{index} provinces={{{}}}}}\n",
                region_id(index),
                index + 1
            )
        })
        .collect();
    let added_region = format!(
        "strategic_region={{id={} name=fixture_mod_region provinces={{{regions}}}}}",
        region_id(regions - 1)
    );
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "base/common/defines/00_fixture.lua",
            "NDefines = { NAir = { HOURS_DELAY_AFTER_EACH_COMBAT = 2 } }",
        ),
        (
            "mod/common/defines/zz_fixture_mod.lua",
            "NDefines.NAir.HOURS_DELAY_AFTER_EACH_COMBAT = 3",
        ),
        ("base/map/strategicregions/00_fixture.txt", &base_regions),
        ("mod/map/strategicregions/zz_fixture_mod.txt", &added_region),
    ]);
    let base = folder.0.join("base");
    let modification = folder.0.join("mod");
    let layers =
        installed_content::ResolvedLayers::from_folders(&[(&base, &[]), (&modification, &[])]).unwrap();
    let defines = installed_content::defines::read(&layers).unwrap();
    let content_regions = installed_content::regions::read(&layers).unwrap().regions.len();
    (
        2 * defines
            .int32("NAir", "HOURS_DELAY_AFTER_EACH_COMBAT")
            .unwrap(),
        content_regions,
    )
}

fn fixture(scenario: Scenario) -> Fixture {
    assert!(scenario.members.len() >= 2 && scenario.members.len().is_multiple_of(2));
    let many_factions = scenario.members.len() > 4;
    assert!(scenario.members.iter().all(|count| *count > 0));
    assert!(scenario.regions > scenario.active_regions);
    let (period, content_regions) = fixture_content(scenario.regions);
    assert_eq!(content_regions, scenario.regions as usize);
    let mut template = scale_frame(1, 0, scenario.countries);
    // State rows are unrelated to either air request, so do not inflate the
    // encoded fixture with that helper's ownership workload.
    *template.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![]));
    let mut owner_sides = Vec::new();
    let mut members = Vec::new();
    let mut leaders = Vec::new();
    for (side, &count) in scenario.members.iter().enumerate() {
        leaders.push(owner_sides.len() as u32);
        for ordinal in 0..count {
            let country = owner_sides.len() as u32;
            owner_sides.push(side);
            members.push(FactionMemberRow {
                country: 100_000 + country,
                faction: 900 + side as u64 * 7,
                ordinal,
            });
        }
    }
    let faction_members = owner_sides.len() as u32;
    for country in 0..scenario.co_belligerents {
        owner_sides.push((country as usize % 2) * 2);
    }
    let combat_owners = owner_sides.len();
    assert!(combat_owners < scenario.countries as usize);
    let neutral_count = if scenario.sparse {
        (scenario.countries - combat_owners as u32).div_ceil(20)
    } else {
        0
    };
    let neutral_start = combat_owners as u32;
    let mut factions: Vec<_> = scenario
        .members
        .iter()
        .enumerate()
        .map(|(side, _)| FactionRow {
            id: 900 + side as u64 * 7,
            name: format!("Synthetic mod faction {side}").into_bytes(),
            ..Default::default()
        })
        .collect();
    factions.sort_by_key(|row| row.id);
    *template.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(factions));
    *template.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    let mut wars = Vec::new();
    for (country, &side) in owner_sides.iter().enumerate() {
        let opposite = side ^ 1;
        let other = leaders[opposite];
        let first = (country as u32).min(other);
        let second = (country as u32).max(other);
        wars.push(WarRelationRow {
            first_country: 100_000 + first,
            second_country: 100_000 + second,
            start_hour: 0,
            end_hour: -1,
            ..Default::default()
        });
    }
    if many_factions {
        wars.clear();
        let hub = *leaders.last().unwrap();
        for &country in &leaders[..leaders.len() - 1] {
            wars.push(WarRelationRow {
                first_country: 100_000 + country,
                second_country: 100_000 + hub,
                start_hour: 0,
                end_hour: -1,
                ..Default::default()
            });
        }
        // Shared enemy plus mutually hostile pairs: successful and blocked joins.
        for pair in leaders[..leaders.len() - 2].chunks_exact(2) {
            wars.push(WarRelationRow {
                first_country: 100_000 + pair[0],
                second_country: 100_000 + pair[1],
                start_hour: 0,
                end_hour: -1,
                ..Default::default()
            });
        }
        for country in faction_members..combat_owners as u32 {
            wars.push(WarRelationRow {
                first_country: 100_000 + country,
                second_country: 100_000 + hub,
                start_hour: 0,
                end_hour: -1,
                ..Default::default()
            });
        }
    }
    wars.sort_by_key(|row| (row.first_country, row.second_country));
    wars.dedup_by_key(|row| (row.first_country, row.second_country));
    let war_pairs = wars.len();
    *template.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(wars));
    *template.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(vec![]));
    *template.tables[TableId::Capitulation.index()] =
        TableView::Complete(TableRows::Capitulation(vec![]));
    let mut records = vec![
        Record::SessionStart(session(1)),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: None,
            first_hour: 0,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    let mut scores = vec![vec![0_i64; scenario.members.len()]; scenario.active_regions as usize];
    let mut active = vec![false; scenario.active_regions as usize];
    let mut random = 0x48_4f_49_34_u64;
    let mut row_count = 0_usize;
    let mut gap_hours = 0;
    let mut recorded_hours = 0;
    let mut truncated_hours = 0;
    for hour in 0..scenario.hours {
        for region in 0..scenario.active_regions as usize {
            if hour == 0 || hour.rem_euclid(period) == region as i32 % period {
                let count = if scenario.sparse {
                    40 + ((hour / (14 * 24)) % 5) * 20
                } else {
                    scenario.active_regions as i32
                };
                let rotation = if scenario.sparse {
                    (hour / (14 * 24)) * 17
                } else {
                    0
                };
                active[region] =
                    (region as i32 - rotation).rem_euclid(scenario.active_regions as i32) < count;
                for (side, value) in scores[region].iter_mut().enumerate() {
                    let war_region =
                        many_factions || scenario.members.len() == 2 || region % 2 == side / 2;
                    if !active[region] || !war_region {
                        *value = 0;
                    } else {
                        random = random
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1);
                        let step = ((random >> 32) % 200_001) as i64 - 100_000;
                        *value = (*value + step).clamp(100_000, 20_000_000);
                    }
                }
            }
        }
        if let Some(&(first, last)) = scenario
            .gaps
            .iter()
            .find(|&&(first, last)| hour >= first && hour <= last)
        {
            gap_hours += 1;
            if hour == first {
                records.push(Record::Gap(Gap {
                    branch: 1,
                    first_hour: first,
                    last_hour: last,
                    reason: "synthetic recording gap".into(),
                }));
            }
            continue;
        }
        let mut frame = template.clone();
        frame.hour = hour;
        if many_factions
            && hour >= 15 * 24
            && let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
        {
            // A:X becomes A:Y while another native holder takes X.
            countries[0].tag = b"Renamed participant".to_vec();
            countries.last_mut().unwrap().tag = scale_tag(0);
        }
        if many_factions {
            // Repeated faction transfers grow history independently of the war graph.
            let members = (0..leaders.len())
                .map(|index| FactionMemberRow {
                    country: 100_000 + leaders[index],
                    faction: 900 + ((index + hour as usize / 24) % leaders.len()) as u64 * 7,
                    ordinal: 0,
                })
                .collect();
            *frame.tables[TableId::FactionMembers.index()] =
                TableView::Complete(TableRows::FactionMembers(members));
        }
        let mut rows = Vec::new();
        for (country, &side) in owner_sides.iter().enumerate() {
            for (region, values) in scores.iter().enumerate() {
                if values[side] != 0 || values[side ^ 1] != 0 {
                    rows.push(RaidAirScoreRow {
                        country: 100_000 + country as u32,
                        region: region_id(region as u32),
                        friendly: values[side],
                        enemy: values[side ^ 1],
                        impact: Default::default(),
                    });
                }
            }
            rows.push(RaidAirScoreRow {
                country: 100_000 + country as u32,
                region: region_id(scenario.regions - 1),
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
            });
        }
        for index in 0..neutral_count {
            rows.push(RaidAirScoreRow {
                country: 100_000 + neutral_start + index,
                region: region_id(scenario.regions - 1),
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
            });
        }
        rows.sort_by_key(|row| (row.country, row.region));
        let truncated = scenario.sparse && hour % 100 == 37;
        if truncated {
            truncated_hours += 1;
            rows.truncate(rows.len() - 2);
        }
        row_count += rows.len();
        *frame.tables[TableId::RaidAirScores.index()] = if truncated {
            TableView::Truncated {
                rows: TableRows::RaidAirScores(rows),
                missing: 2,
            }
        } else {
            TableView::Complete(TableRows::RaidAirScores(rows))
        };
        let mut wings = Vec::new();
        for (side, &leader) in leaders.iter().enumerate() {
            for (region, _) in scores
                .iter()
                .enumerate()
                .filter(|(_, values)| values[side] > 0)
            {
                wings.push(AirWingRow {
                    id: 1 + (side as u64 * scenario.regions as u64 + region as u64),
                    owner: 100_000 + leader,
                    planes: 10,
                    mission_region: Some(region_id(region as u32)),
                    ..Default::default()
                });
            }
        }
        for index in 0..neutral_count {
            for assignment in 0..1 + index % 5 {
                wings.push(AirWingRow {
                    id: 1_000_000 + u64::from(index) * 5 + u64::from(assignment),
                    owner: 100_000 + neutral_start + index,
                    planes: 10,
                    mission_region: Some(region_id(
                        (index * 7 + assignment) % scenario.active_regions,
                    )),
                    ..Default::default()
                });
            }
        }
        wings.sort_by_key(|wing| wing.id);
        if many_factions {
            use replay::rows::{AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow};
            *frame.tables[TableId::AirTheatres.index()] =
                TableView::Complete(TableRows::AirTheatres(
                    (0..leaders.len())
                        .map(|index| AirTheatreRow {
                            id: 800_000 + index as u64,
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::AirTheatreGroups.index()] =
                TableView::Complete(TableRows::AirTheatreGroups(
                    (0..leaders.len())
                        .map(|index| AirTheatreGroupRow {
                            id: 810_000 + index as u64,
                            theatre: 800_000 + index as u64,
                            ..Default::default()
                        })
                        .collect(),
                ));
            *frame.tables[TableId::AirTheatreMembers.index()] =
                TableView::Complete(TableRows::AirTheatreMembers(
                    wings
                        .iter()
                        .filter(|wing| wing.id < 1_000_000)
                        .map(|wing| AirTheatreMemberRow {
                            wing: wing.id,
                            group: 810_000 + (wing.owner - 100_000) as u64,
                            ..Default::default()
                        })
                        .collect(),
                ));
        }
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(wings));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
        recorded_hours += 1;
    }
    let replay = Arc::new(Replay::from_records(CAMPAIGN.into(), &[records]).unwrap());
    println!(
        "air_fixture countries={} factions={} co_belligerents={} faction_members={faction_members} air_object_owners={} neutral_wing_owners={neutral_count} war_pairs={war_pairs} content_regions={content_regions} hours={} recorded_hours={recorded_hours} gap_hours={gap_hours} truncated_hours={truncated_hours} score_rows_mean={:.1} period={period} replay_bytes={}",
        scenario.countries,
        scenario.members.len(),
        scenario.co_belligerents,
        combat_owners + neutral_count as usize,
        scenario.hours,
        row_count as f64 / recorded_hours as f64,
        replay.bytes().len()
    );
    Fixture {
        replay,
        countries: scenario.countries,
        owners: combat_owners + neutral_count as usize,
        score_rows: row_count,
        neutral_owners: neutral_count,
        war_pairs,
        gap_hours,
        recorded_hours,
        truncated_hours,
    }
}

#[test]
fn air_superiority_sparse_fixture_preserves_a_whole_missing_day() {
    let fixture = fixture(Scenario {
        countries: 40,
        members: vec![3, 3, 4, 4],
        co_belligerents: 2,
        regions: 300,
        active_regions: 120,
        hours: 72,
        sparse: true,
        gaps: vec![(24, 47)],
    });
    assert_eq!(fixture.recorded_hours, 48);
    assert_eq!(fixture.gap_hours, 24);
    assert_eq!(fixture.truncated_hours, 0); // The would-be partial hour is in the gap.
    let cancelled = AtomicBool::new(false);
    let groups = super::super::country_groups::scan_branch(&fixture.replay, 1, &cancelled).unwrap();
    let cache = scan_days(&fixture.replay, 1, &groups, None, &cancelled).unwrap();
    assert_eq!(cache.built_hours().len(), fixture.recorded_hours);
    let mut params: AirParams = serde_json::from_value(serde_json::json!({
        "branchId": "1", "view": "timeline", "firstHour": 0,
        "lastHour": 71, "mode": "sides", "top": 10,
    }))
    .unwrap();
    let reply = timeline(&cache, &params).unwrap();
    assert_eq!(reply.days, [Some(23), None, Some(71)]);
    assert_eq!(reply.rows.len(), 10);
    for row in &reply.rows {
        let cells: Vec<_> = row
            .cells
            .iter()
            .flat_map(|(repeat, cell)| std::iter::repeat_n(cell, *repeat))
            .collect();
        assert_eq!(cells.len(), 3);
        assert_eq!(
            *cells[1],
            AirCell(None),
            "a missing day remains null, never a zero score"
        );
    }
    params.view = AirView::Moment;
    let moment = scan_moment(&fixture.replay, 1, &groups, &params, &cancelled).unwrap();
    assert_eq!(
        moment.recorded,
        [
            AirSpan {
                first_hour: 0,
                last_hour: 23
            },
            AirSpan {
                first_hour: 48,
                last_hour: 71
            }
        ]
    );
    assert!(moment.regions.iter().all(|region| {
        region
            .runs
            .iter()
            .all(|(first, last, _)| *last < 24 || *first > 47)
    }));
    assert!(moment.regions.iter().all(|region| {
        region
            .wings
            .iter()
            .all(|(first, last, _)| *last < 24 || *first > 47)
    }));
    assert!(
        moment.sides.iter().any(|side| side.co_belligerents == 1),
        "recorded war relations attach both outside allies to their faction sides"
    );
}

fn measure(scenario: Scenario, last_hour: i32) {
    let enforce_budget = scenario.sparse;
    let many_factions = scenario.members.len() > 4;
    let selected_theatres: Vec<_> = (0..scenario.members.len())
        .map(|index| (800_000 + index).to_string())
        .collect();
    let fixture = fixture(scenario);
    let cancelled = AtomicBool::new(false);
    let started = Instant::now();
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "air-scale".into(),
            revision: "1".into(),
        },
        "synthetic-air.battleplan".into(),
        Arc::clone(&fixture.replay),
        &cancelled,
    )
    .unwrap();
    let source_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let groups = super::super::country_groups::cached_scan(&source, 1, &cancelled).unwrap();
    let groups_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(Arc::ptr_eq(
        &groups,
        &super::super::country_groups::cached_scan(&source, 1, &cancelled).unwrap()
    ));
    let mut cache = DayCache {
        measurements: Some(Default::default()),
        ..Default::default()
    };
    let started = Instant::now();
    extend_days(&mut cache, &fixture.replay, 1, &groups, &cancelled).unwrap();
    let scan_time = started.elapsed();
    let stages = cache.measurements.as_ref().unwrap();
    let product_time = scan_time.saturating_sub(stages.wire + stages.decode);
    let ms_per_hour = |duration: std::time::Duration| {
        duration.as_secs_f64() * 1000.0 / fixture.recorded_hours as f64
    };
    assert_eq!(cache.built_hours().len(), fixture.recorded_hours);
    let mut params: AirParams = serde_json::from_value(serde_json::json!({
        "branchId": "1", "view": "timeline", "firstHour": 0,
        "lastHour": last_hour, "mode": "sides", "top": 10,
    }))
    .unwrap();
    if many_factions {
        params.theatres = Some(selected_theatres);
    }
    let started = Instant::now();
    let reply = timeline(&cache, &params).unwrap();
    let timeline_ms = started.elapsed().as_secs_f64() * 1000.0;
    let day_count = (last_hour / DAY_HOURS + 1) as usize;
    assert_eq!(reply.days.len(), day_count);
    assert!(
        reply
            .rows
            .iter()
            .all(|row| row.cells.iter().map(|(repeat, _)| repeat).sum::<usize>() == day_count)
    );
    let started = Instant::now();
    let timeline_bytes = check_reply_size(&AirReply::Timeline(reply)).unwrap();
    let timeline_json_ms = started.elapsed().as_secs_f64() * 1000.0;
    params.view = AirView::Moment;
    params.first_hour = Some((last_hour - 167).max(0));
    let mut moment_stages = scan::ScanMeasurements::default();
    let started = Instant::now();
    let moment = scan_moment_measured(
        &fixture.replay,
        1,
        &groups,
        &params,
        &cache.anchors,
        &cancelled,
        Some(&mut moment_stages),
    )
    .unwrap();
    let moment_time = started.elapsed();
    let moment_ms = moment_time.as_secs_f64() * 1000.0;
    let moment_product = moment_time.saturating_sub(moment_stages.wire + moment_stages.decode);
    let moment_hours = (last_hour - params.first_hour.unwrap() + 1) as f64;
    let moment_product_ms_per_hour = moment_product.as_secs_f64() * 1000.0 / moment_hours;
    let started = Instant::now();
    let moment_bytes = check_reply_size(&AirReply::Moment(moment)).unwrap();
    let moment_json_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut viewpoint = DayCache {
        viewpoint: Some(String::from_utf8(scale_tag(0)).unwrap()),
        viewpoint_country: true,
        measurements: Some(Default::default()),
        ..Default::default()
    };
    let started = Instant::now();
    extend_days_shared(
        &mut viewpoint,
        &fixture.replay,
        1,
        &groups,
        Some(&cache),
        &cancelled,
    )
    .unwrap();
    let view_time = started.elapsed();
    let view_stages = viewpoint.measurements.as_ref().unwrap();
    let view_product = view_time.saturating_sub(view_stages.wire + view_stages.decode);
    println!(
        "air_measure countries={} owners={} neutral_owners={} war_pairs={} recorded_hours={} gap_hours={} truncated_hours={} score_rows={} source_ms={source_ms:.3} groups_cold_ms={groups_ms:.3} wire_ms_per_hour={:.3} decode_ms_per_hour={:.3} product_ms_per_hour={:.3} scan_ms_per_hour={:.3} viewpoint_product_ms_per_hour={:.3} timeline_ms={timeline_ms:.3} timeline_json_ms={timeline_json_ms:.3} timeline_bytes={timeline_bytes} moment_ms={moment_ms:.3} moment_product_ms_per_hour={moment_product_ms_per_hour:.3} moment_json_ms={moment_json_ms:.3} moment_bytes={moment_bytes}",
        fixture.countries,
        fixture.owners,
        fixture.neutral_owners,
        fixture.war_pairs,
        fixture.recorded_hours,
        fixture.gap_hours,
        fixture.truncated_hours,
        fixture.score_rows,
        ms_per_hour(stages.wire),
        ms_per_hour(stages.decode),
        ms_per_hour(product_time),
        ms_per_hour(scan_time),
        ms_per_hour(view_product),
    );
    if enforce_budget {
        assert!(
            moment_product_ms_per_hour <= 5.5,
            "air moment exceeds 5.5 ms/game hour: {moment_product_ms_per_hour:.3}"
        );
        assert!(
            ms_per_hour(product_time) <= 5.5,
            "air product exceeds 5.5 ms/game hour: {:.3}",
            ms_per_hour(product_time)
        );
        assert!(
            ms_per_hour(view_product) <= 5.5,
            "air viewpoint exceeds 5.5 ms/game hour: {:.3}",
            ms_per_hour(view_product)
        );
    }
}

fn realistic_scenario(countries: u32, hours: i32) -> Scenario {
    Scenario {
        countries,
        members: [20, 80, 160, 300]
            .map(|count| count * countries / 1500)
            .into(),
        co_belligerents: 50 * countries / 1500,
        regions: 300,
        active_regions: 120,
        hours,
        sparse: true,
        gaps: vec![
            (73 * 24 + 12, 73 * 24 + 17),
            (177 * 24, 177 * 24 + 23),
            (281 * 24 + 3, 281 * 24 + 6),
        ],
    }
}

#[test]
#[ignore = "one synthetic game year; explicit measurement outside routine tests"]
fn air_superiority_realistic_year_scale() {
    measure(realistic_scenario(1500, 8760), 8759);
}

#[test]
#[ignore = "same realistic change pattern at 5,000 countries; explicit measurement"]
fn air_superiority_realistic_5000_scale() {
    // Ninety days cover all five fourteen-day activity levels and a gap.
    measure(realistic_scenario(5000, 90 * 24), 90 * 24 - 1);
}

fn stress_scenario(countries: u32) -> Scenario {
    Scenario {
        countries,
        members: vec![countries / 5, countries / 5],
        co_belligerents: 0,
        regions: 151,
        active_regions: 150,
        hours: 168,
        sparse: false,
        gaps: vec![],
    }
}

#[test]
#[ignore = "same dense change pattern at 1,500 countries; explicit measurement"]
fn air_superiority_stress_1500_scale() {
    measure(stress_scenario(1500), 167);
}

#[test]
#[ignore = "300,000 active pairs per hour; explicit stress measurement"]
fn air_superiority_stress_5000_scale() {
    // The 151st content region carries zero witnesses; 150 regions are active.
    measure(stress_scenario(5000), 167);
}

fn many_faction_scenario(countries: u32) -> Scenario {
    Scenario {
        countries,
        members: vec![1; (countries / 3 / 2 * 2) as usize],
        co_belligerents: countries / 30,
        regions: 33,
        active_regions: 32,
        hours: 30 * 24,
        sparse: true,
        gaps: vec![],
    }
}

#[test]
#[ignore = "many factions, blocked alliances, transfers and selected theatres; controlled measurement"]
fn air_superiority_many_factions_scale() {
    for countries in [1500, 5000] {
        measure(many_faction_scenario(countries), 30 * 24 - 1);
    }
}
