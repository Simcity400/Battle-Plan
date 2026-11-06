use super::*;
use dashboard_core::test_fixtures::{CAMPAIGN, scale_frame, scale_tag, session};
use replay::spool::Record;
use replay_data::{
    rows::AirWingRow, rows::FactionMemberRow, rows::FactionRow, rows::RaidAirScoreRow,
    rows::TableRows, rows::WarRelationRow,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Gap, branches::Parentage,
    frame::FrameEncoder, frame::TableView,
};
use std::sync::Arc;

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
    let (_, folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
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
        installed_content_core::ResolvedLayers::from_folders(&[(&base, &[]), (&modification, &[])])
            .unwrap();
    let defines = installed_content_core::defines::read(&layers).unwrap();
    let content_regions = installed_content_regions::regions::read(&layers)
        .unwrap()
        .regions
        .len();
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
                        friendly_missions: 0,
                        enemy_missions: 0,
                        air_spotting_scale: Some(0),
                        menu: replay_data::rows::Recorded::Value(None),
                    });
                }
            }
            rows.push(RaidAirScoreRow {
                country: 100_000 + country as u32,
                region: region_id(scenario.regions - 1),
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
                menu: replay_data::rows::Recorded::Value(None),
            });
        }
        for index in 0..neutral_count {
            rows.push(RaidAirScoreRow {
                country: 100_000 + neutral_start + index,
                region: region_id(scenario.regions - 1),
                friendly: 0,
                enemy: 0,
                impact: Default::default(),
                friendly_missions: 0,
                enemy_missions: 0,
                air_spotting_scale: Some(0),
                menu: replay_data::rows::Recorded::Value(None),
            });
        }
        rows.sort_by_key(|row| (row.country, row.region));
        let truncated = scenario.sparse && hour % 100 == 37;
        if truncated {
            truncated_hours += 1;
            rows.truncate(rows.len() - 2);
        }
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
            use replay_data::{
                rows::AirTheatreGroupRow, rows::AirTheatreMemberRow, rows::AirTheatreRow,
            };
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
    Fixture {
        replay,
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
    let groups =
        dashboard_base::products::country_groups::scan_branch(&fixture.replay, 1, &cancelled)
            .unwrap();
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
