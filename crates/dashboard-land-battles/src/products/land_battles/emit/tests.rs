//! The census kept from projected hours held to the row census on the same hours, written as
//! the native recorder writes them, across segment starts, a resume, a gap and refused tables.

use super::*;
use crate::products::land_battles::facts::Projector;
use replay::{
    columnar_frames::synthetic::{Input, Sample, Synthetic},
    segments::{self, Operator},
};
use replay_data::{
    rows::{
        CountryLinkRow, CountryRow, LandCombatProgressRow, LandCombatResultsRow, LandCombatRow,
        LandEquipmentLoss, LandEventCoverageRow, LandResultSide, LandSideLosses, Recorded,
        RowCodec, TableId,
    },
    scope::CaptureMode,
};
use serde_json::Value;

const FORMAT: FormatVersion = FormatVersion::CURRENT;
const REFUSAL: &str = "synthetic refusal";

/// One hour as the recorder writes it; None where a table is unavailable.
struct Tables {
    hour: i32,
    countries: Vec<CountryRow>,
    links: Vec<CountryLinkRow>,
    combats: Option<Vec<LandCombatRow>>,
    sides: Vec<LandCombatSideRow>,
    progress: Option<Vec<LandCombatProgressRow>>,
    results: Vec<LandCombatResultsRow>,
    coverage: Vec<LandEventCoverageRow>,
}

fn unit(country: u64, index: u64) -> u64 {
    country << 32 | index
}

/// Hours 100 to 150 without 125 and 126. Combat 1 holds its counter at 105 and 106, gains an
/// attacker at 110 and is finalized at 118; combat 2 runs from 103 through the gap, gains a
/// secondary attacker at 112 and moves province at 135; combat 3 runs from 128 to 133 with no
/// participant row; combat 5 counts one of two repeated defenders from 141. LandCombats is
/// unavailable at 122 and LandCombatProgress at 145; a country and its link join at 140. A
/// delayed finalizer of combat 2 in another province, which ends none of its battles, lands at
/// 138 while combat 2 steps its counter.
fn scenario() -> Vec<Tables> {
    (100..=150)
        .filter(|hour| !(125..=126).contains(hour))
        .map(|hour: i32| {
            let mut countries = vec![(1, 1, "AAA"), (2, 2, "BBB"), (3, 3, "CCC")];
            let mut links = vec![(1, 1), (2, 2), (3, 3), (11, 1)];
            if hour >= 140 {
                countries.push((4, 4, "DDD"));
                links.push((4, 4));
            }
            let mut combats = Vec::new();
            let mut sides = Vec::new();
            let mut progress = Vec::new();
            if (100..=117).contains(&hour) {
                let held = if hour >= 106 { hour - 1 } else { hour };
                let mut attackers = vec![unit(1, 1), unit(1, 2)];
                if hour >= 110 {
                    attackers.push(unit(1, 3));
                }
                combats.push(LandCombatRow {
                    id: 1,
                    province: 10,
                    duration: held - 95,
                    attackers,
                    defenders: vec![unit(2, 1)],
                    ..Default::default()
                });
                sides.push(LandCombatSideRow {
                    combat: 1,
                    attacker_primary: vec![1],
                    defender_primary: vec![2],
                    ..Default::default()
                });
                progress.push(LandCombatProgressRow {
                    combat: 1,
                    attacker_manpower_loss: Recorded::Value(hour * 10),
                    defender_manpower_loss: Recorded::NotRecorded,
                    attacker_equipment_losses: Recorded::Value(vec![LandEquipmentLoss {
                        variant: 7,
                        amount_raw: i64::from(hour) * 1000,
                    }]),
                    defender_equipment_losses: Recorded::NotRecorded,
                    ..Default::default()
                });
            }
            if hour >= 103 {
                combats.push(LandCombatRow {
                    id: 2,
                    province: if hour >= 135 { 30 } else { 20 },
                    duration: hour - 100,
                    attackers: vec![unit(2, 5)],
                    defenders: vec![unit(3, 1), unit(3, 2)],
                    ..Default::default()
                });
                sides.push(LandCombatSideRow {
                    combat: 2,
                    attacker_primary: vec![2],
                    attacker_secondary: if hour >= 112 { vec![1] } else { Vec::new() },
                    defender_primary: vec![3, 99],
                    ..Default::default()
                });
                if hour <= 115 {
                    progress.push(LandCombatProgressRow {
                        combat: 2,
                        attacker_manpower_loss: Recorded::Value(hour.min(110)),
                        defender_manpower_loss: Recorded::Value(3),
                        attacker_equipment_losses: Recorded::Value(Vec::new()),
                        defender_equipment_losses: Recorded::Value(vec![LandEquipmentLoss {
                            variant: 9,
                            amount_raw: 5,
                        }]),
                        ..Default::default()
                    });
                }
            }
            if (128..=133).contains(&hour) {
                combats.push(LandCombatRow {
                    id: 3,
                    province: 40,
                    duration: hour - 128,
                    attackers: vec![unit(1, 9)],
                    defenders: vec![unit(3, 9)],
                    ..Default::default()
                });
            }
            if hour >= 141 {
                combats.push(LandCombatRow {
                    id: 5,
                    province: 50,
                    duration: hour - 141,
                    attackers: vec![unit(4, 1)],
                    defenders: vec![unit(3, 4), unit(3, 4)],
                    ..Default::default()
                });
                sides.push(LandCombatSideRow {
                    combat: 5,
                    attacker_primary: vec![11],
                    defender_primary: vec![3],
                    ..Default::default()
                });
            }
            let delayed = (hour == 138).then(|| LandCombatResultsRow {
                id: 901,
                combat: 2,
                province: 99,
                duration: 5,
                end_hour: 138,
                winners: 2,
                ..Default::default()
            });
            let results = if hour == 118 {
                vec![LandCombatResultsRow {
                    id: 900,
                    combat: 1,
                    province: 10,
                    duration: 22,
                    end_hour: 118,
                    sides: [
                        LandResultSide {
                            countries: vec![1],
                            ..Default::default()
                        },
                        LandResultSide {
                            countries: vec![2],
                            ..Default::default()
                        },
                    ],
                    winners: 1,
                    losses: [
                        LandSideLosses {
                            manpower: 1200,
                            equipment: vec![LandEquipmentLoss {
                                variant: 7,
                                amount_raw: 119_000,
                            }],
                        },
                        LandSideLosses {
                            manpower: 4,
                            equipment: Vec::new(),
                        },
                    ],
                    ..Default::default()
                }]
            } else {
                delayed.into_iter().collect()
            };
            let country_rows = countries
                .into_iter()
                .map(|(id, canonical_slot, tag): (u32, u32, &str)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            let mut links: Vec<_> = links
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect();
            links.sort_unstable_by_key(|link| link.id);
            Tables {
                hour,
                countries: country_rows,
                links,
                combats: (hour != 122).then_some(combats),
                sides,
                progress: (hour != 145).then_some(progress),
                results,
                coverage: vec![LandEventCoverageRow {
                    id: 0,
                    epoch: 1,
                    from_hour: hour - 1,
                    to_hour: hour,
                    complete: true,
                    lost_events: 0,
                }],
            }
        })
        .collect()
}

fn write<R: RowCodec>(walk: &mut Synthetic, table: TableId, rows: Option<&[R]>) {
    let (read, sample) = (true, Sample::FromReadBit);
    let input = match rows {
        Some(rows) => Input::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Input::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// What the census holds that its publications read, the runs `stepping` holds extended.
fn held(census: &Census, stepping: Option<&Stepping>) -> Value {
    let mut open: Vec<_> = census
        .open
        .iter()
        .map(|(&combat, &at)| (combat, at))
        .collect();
    open.sort_unstable();
    let mut latest: Vec<_> = census
        .latest
        .iter()
        .map(|(&combat, &at)| (combat, at))
        .collect();
    latest.sort_unstable();
    let mut episodes = serde_json::to_value(&census.episodes).unwrap();
    for ordinal in stepping.iter().flat_map(|stepping| &stepping.members) {
        let runs = episodes[ordinal.to_string()]["runs"]
            .as_array_mut()
            .unwrap();
        runs.last_mut().unwrap()["lastHour"] = stepping.unwrap().through.into();
    }
    let mut dirty: Vec<_> = census.dirty.iter().copied().collect();
    dirty.sort_unstable();
    serde_json::json!({
        "episodes": episodes,
        "dirty": dirty,
        "created": census.created,
        "firstStart": census.first_start,
        "open": open,
        "latest": latest,
        "observed": census.observed,
        "warnings": serde_json::to_value(&census.warnings).unwrap(),
        "liveWarnings": census.live_warnings,
        "deferred": census.deferred_through,
        "available": census.available,
        "designs": serde_json::to_value(&census.designs.values).unwrap(),
    })
}

/// The scenario's hours kept by the row census and, projected by an operator starting cold at
/// each of `starts` and after the gap, by the census an emit keeps, saved and taken up again
/// after `resume`: both hold the same after every hour.
fn compare(starts: &[i32], resume: Option<i32>) {
    let hours = scenario();
    let mut oracle = Census::new(FORMAT);
    let mut census = Census::new(FORMAT);
    census.configured = true;
    let mut mirror = Mirror::default();
    let mut projector = Projector::new(FORMAT).unwrap();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut last: Option<i32> = None;
    for (ordinal, tables) in (0..).zip(&hours) {
        let gap = last.is_some_and(|last| last + 1 != tables.hour);
        last = Some(tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(facts::TABLES, 1, tables.hour).unwrap());
        }
        let walk = walk.as_mut().unwrap();
        write(walk, TableId::Countries, Some(&tables.countries[..]));
        write(walk, TableId::CountryLinks, Some(&tables.links[..]));
        write(walk, TableId::LandCombats, tables.combats.as_deref());
        write(walk, TableId::LandCombatSides, Some(&tables.sides[..]));
        write(
            walk,
            TableId::LandCombatProgress,
            tables.progress.as_deref(),
        );
        write(walk, TableId::LandCombatResults, Some(&tables.results[..]));
        write(walk, TableId::LandEventCoverage, Some(&tables.coverage[..]));
        let cold = starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(FORMAT).unwrap();
        }
        let mut out = Vec::new();
        walk.end(|state, manifest, decoded, first| {
            oracle.observe(decoded).unwrap();
            let starts = first || cold;
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &facts::needs(),
                (FORMAT, CaptureMode::Full),
                state,
                manifest,
            )
            .unwrap();
            projector
                .hour(
                    &segments::Hour {
                        branch: 1,
                        hour: decoded.hour,
                        run: 0,
                        ordinal,
                        starts,
                        opens: starts,
                        state,
                        manifest,
                        world,
                    },
                    &mut out,
                )
                .unwrap();
        })
        .unwrap();
        for hour in out {
            census.observe_projected(1, hour, &mut mirror).unwrap();
        }
        assert_eq!(
            held(&census, Some(&mirror.stepping)),
            held(&oracle, None),
            "after hour {}",
            tables.hour
        );
        if resume == Some(tables.hour) {
            let saved = save(&mut census, &mut mirror, 0).unwrap();
            census = load(FORMAT, None, &saved).unwrap().0;
            assert_eq!(
                held(&census, None),
                held(&oracle, None),
                "resumed at {}",
                tables.hour
            );
            mirror = Mirror::default();
            projector = Projector::new(FORMAT).unwrap();
        }
    }
}

#[test]
fn projected_census_holds_the_row_census_walked_whole() {
    compare(&[], None);
}

#[test]
fn projected_census_holds_the_row_census_across_segment_starts_and_a_resume() {
    compare(&[105, 106, 112, 118, 128, 134, 141, 146], Some(113));
}

#[test]
fn projected_census_holds_the_row_census_starting_every_hour() {
    let starts: Vec<i32> = (100..=150).collect();
    compare(&starts, None);
}
