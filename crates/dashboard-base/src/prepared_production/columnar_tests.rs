//! The native preparation held to the row walk's on the same hours, written as the native
//! recorder writes them: the facts and spent series both publish, and every endpoint both
//! complete, across segment starts, a gap, an hour without countries, flushes and a resume.

use super::*;
use crate::prepared::{
    frame_changes::FrameChangeTracker,
    series::tests::{Fixture, assert_payloads_equal},
};
use columnar::{Branch, Projector};
use replay::{
    columnar_frames::synthetic::{Input as Written, Sample, Synthetic},
    segments::{self, Item, Operator},
};
use replay_data::{
    rows::{
        CountryLinkRow, CountryRow, LexicalTokenRow, OutputPurpose, ProductionLineOutputRow,
        ProductionLineRow, RocketProductionOutputRow, RowCodec, VariantChangeRow,
        VariantDefinitionRow,
    },
    scope::CaptureMode,
};

const REFUSAL: &str = "synthetic refusal";

/// One hour as the recorder writes it.
struct Tables {
    hour: i32,
    countries: Option<Vec<CountryRow>>,
    links: Vec<CountryLinkRow>,
    lines: Vec<ProductionLineRow>,
    /// None where the table is unavailable.
    output: Option<Vec<ProductionLineOutputRow>>,
    rocket: Vec<RocketProductionOutputRow>,
    definitions: Vec<VariantDefinitionRow>,
    /// The accepted design edits of the hour.
    changes: Vec<VariantChangeRow>,
    tokens: Vec<LexicalTokenRow>,
}

/// The first hour of the scenario's first day.
fn start() -> i32 {
    crate::test_fixtures::slot_hour(TableId::ProductionLineOutput)
        - crate::test_fixtures::slot_hour(TableId::ProductionLineOutput) % 24
}

/// A line's output row: `days` of delivery and input since the observation began.
fn output_row(
    line: u64,
    country: u32,
    variant: u64,
    days: i64,
    since: i32,
) -> ProductionLineOutputRow {
    ProductionLineOutputRow {
        line,
        country,
        variant,
        units: days,
        since_hour: since,
        exact: true,
        spent_raw: Recorded::Value(Some(days * 250_000)),
        purpose: Recorded::Value(OutputPurpose::New.raw()),
        ..Default::default()
    }
}

/// Five days of three countries (a fourth from day three) producing two designs: AAA's line 1
/// all through, BBB's line 2 until it ends on day two, and DDD's line 3 from day three, with a
/// Rocket design of AAA's. The countries are unavailable for one hour, the output for one read,
/// BBB's tag holder changes id on day four, and hours 61 to 63 are not recorded. Design 2 is
/// renamed at hour 40, between censuses, and again at 44 with an accepted edit naming it.
fn scenario() -> Vec<Tables> {
    let first = start();
    (0..=120)
        .filter(|offset| !(61..=63).contains(offset))
        .map(|offset| {
            let hour = first + offset;
            // Output is published at each day's read: it counts the reads before and at it.
            let read = crate::test_fixtures::slot_hour(TableId::ProductionLineOutput) % 24;
            let days = i64::from((offset - read).div_euclid(24) + 1).max(0);
            let b = if offset >= 72 { 20 } else { 2 };
            let mut countries = vec![(1, 1, "AAA"), (b, 2, "BBB"), (3, 3, "CCC")];
            if offset >= 48 {
                countries.push((4, 4, "DDD"));
            }
            let countries: Vec<_> = countries
                .into_iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect();
            let mut links: Vec<_> = countries
                .iter()
                .map(|country| CountryLinkRow {
                    id: country.id,
                    canonical_slot: country.canonical_slot,
                })
                .collect();
            links.sort_unstable_by_key(|link| link.id);
            let mut lines = vec![ProductionLineRow {
                handle: 1,
                country: 1,
                variant: 1,
                active_factories: i32::try_from(days).unwrap(),
                ..Default::default()
            }];
            let mut output = vec![output_row(1, 1, 1, days, first)];
            if days <= 2 {
                lines.push(ProductionLineRow {
                    handle: 2,
                    country: b,
                    variant: 2,
                    ..Default::default()
                });
                let mut row = output_row(2, b, 2, days, first);
                row.ended = days == 2;
                output.push(row);
            }
            if days >= 3 {
                lines.push(ProductionLineRow {
                    handle: 3,
                    country: 4,
                    variant: 1,
                    ..Default::default()
                });
                output.push(output_row(3, 4, 1, days - 2, first));
            }
            let rocket_days = u64::try_from(days).unwrap();
            let mut rocket = vec![RocketProductionOutputRow {
                since_hour: first,
                exact: true,
                units_exact: true,
                ..Default::default()
            }];
            if rocket_days > 0 {
                rocket.push(RocketProductionOutputRow {
                    country: 1,
                    variant: 7,
                    spent_raw: i64::try_from(rocket_days).unwrap() * 500_000,
                    since_hour: first,
                    exact: true,
                    ticks: rocket_days,
                    units: i64::try_from(rocket_days / 2).unwrap(),
                    units_exact: true,
                    deliveries: rocket_days / 2,
                    ..Default::default()
                });
            }
            let second: &[u8] = match offset {
                ..40 => b"second design",
                40..44 => b"renamed design",
                _ => b"edited design",
            };
            let definitions = [(1, b"first design".as_slice()), (2, second)]
                .into_iter()
                .map(|(id, name)| VariantDefinitionRow {
                    id,
                    equipment_token: 1,
                    creator_link: 1,
                    display_name: name.to_vec(),
                    ..Default::default()
                })
                .collect();
            let changes = (offset == 44)
                .then(|| VariantChangeRow {
                    sequence: 1,
                    hour,
                    variant: 2,
                    flags: Some([1, 1, 1]),
                    ..Default::default()
                })
                .into_iter()
                .collect();
            let tokens = vec![LexicalTokenRow {
                token: 1,
                literal: b"mod_equipment_a".to_vec(),
            }];
            Tables {
                hour,
                countries: (offset != 30).then_some(countries),
                links,
                lines,
                output: (offset != 56).then_some(output),
                rocket,
                definitions,
                changes,
                tokens,
            }
        })
        .collect()
}

/// Writes `table`'s rows at `hour`, read where its cadence reads it.
fn write<R: RowCodec>(walk: &mut Synthetic, hour: i32, table: TableId, rows: Option<&[R]>) {
    let read = replay_data::cadence::is_refresh(table, hour);
    let sample = Sample::FromReadBit;
    let input = match rows {
        Some(rows) => Written::Snapshot {
            rows,
            partial: None,
            read,
            sample,
        },
        None => Written::Unavailable {
            reason: REFUSAL,
            read,
            sample,
        },
    };
    walk.table(table, input).unwrap();
}

/// Both preparations of `hours`: the row walk's accumulator fed each hour's row frame, and the
/// native branch fed the projected hours of a projector starting cold at each of `starts` and
/// at each walk's first hour (after a gap the walk starts again). Each flushes after each of
/// `cuts`, and at `resume` both go on from what they saved.
fn prepare(
    hours: &[Tables],
    starts: &[i32],
    cuts: &[i32],
    resume: Option<i32>,
) -> (Vec<(EndpointPage, Option<i32>)>, Fixture) {
    prepare_resumes(hours, starts, cuts, &resume.into_iter().collect::<Vec<_>>())
}

fn prepare_resumes(
    hours: &[Tables],
    starts: &[i32],
    cuts: &[i32],
    resumes: &[i32],
) -> (Vec<(EndpointPage, Option<i32>)>, Fixture) {
    let format = FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let fixtures = [Fixture::new(), Fixture::new()];
    let first = hours[0].hour;
    let mut row = ProductionAccumulator::new(format.version(), 1, first);
    let mut native = Branch::new(format, 1, first);
    let mut tracker = FrameChangeTracker::new();
    let mut walk: Option<Synthetic> = None;
    let mut world = None;
    let mut projector = Projector::new(format).unwrap();
    let mut last: Option<i32> = None;
    let mut cold = false;
    for (ordinal, tables) in (0..).zip(hours) {
        let gap = last.is_some_and(|last| last + 1 != tables.hour);
        if walk.is_none() || gap {
            walk = Some(Synthetic::new(columnar::TABLES, 1, tables.hour).unwrap());
        }
        last = Some(tables.hour);
        let walk = walk.as_mut().unwrap();
        write(
            walk,
            tables.hour,
            TableId::Countries,
            tables.countries.as_deref(),
        );
        write(
            walk,
            tables.hour,
            TableId::CountryLinks,
            Some(&tables.links[..]),
        );
        write(
            walk,
            tables.hour,
            TableId::ProductionLines,
            Some(&tables.lines[..]),
        );
        write(
            walk,
            tables.hour,
            TableId::ProductionLineOutput,
            tables.output.as_deref(),
        );
        write(
            walk,
            tables.hour,
            TableId::RocketProductionOutput,
            Some(&tables.rocket[..]),
        );
        write(
            walk,
            tables.hour,
            TableId::VariantDefinitions,
            Some(&tables.definitions[..]),
        );
        write::<replay_data::rows::VariantModuleRow>(
            walk,
            tables.hour,
            TableId::VariantModules,
            Some(&[]),
        );
        write::<replay_data::rows::VariantUpgradeRow>(
            walk,
            tables.hour,
            TableId::VariantUpgrades,
            Some(&[]),
        );
        write(
            walk,
            tables.hour,
            TableId::VariantChanges,
            Some(&tables.changes[..]),
        );
        write(
            walk,
            tables.hour,
            TableId::LexicalTokens,
            Some(&tables.tokens[..]),
        );
        cold |= starts.contains(&tables.hour);
        if cold {
            projector = Projector::new(format).unwrap();
        }
        let mut out = Vec::new();
        walk.end(|state, manifest, decoded, opened| {
            let starts = opened || std::mem::take(&mut cold);
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &columnar::needs(),
                (format, CaptureMode::Full),
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
            row.ingest_changed(format, decoded, &flag, &tracker.observe(decoded, format))
                .unwrap();
        })
        .unwrap();
        for hour in out {
            native.observe(hour).unwrap();
        }
        if cuts.contains(&tables.hour) || std::ptr::eq(tables, hours.last().unwrap()) {
            for (index, fixture) in fixtures.iter().enumerate() {
                let mut writer = fixture.writer();
                if index == 0 {
                    row.flush(&mut writer, &flag).unwrap();
                } else {
                    native.flush(&mut writer, &flag).unwrap();
                }
                writer.commit().unwrap();
            }
            let label = format!("published through {}", tables.hour);
            assert_payloads_equal(&fixtures[1].payloads(), &fixtures[0].payloads(), &label);
            assert_eq!(native.accumulator().completed, row.completed, "{label}");
            if resumes.contains(&tables.hour) {
                let saved = |state: &ProductionAccumulator| -> ProductionAccumulator {
                    serde_json::from_slice(&serde_json::to_vec(state).unwrap()).unwrap()
                };
                row = saved(&row);
                native = Branch::resumed(Box::new(saved(native.accumulator())));
                cold = true;
                tracker.invalidate();
            }
        }
    }
    (row.completed, fixtures.into_iter().next().unwrap())
}

#[test]
fn design_witness_keeps_the_empty_census_country_run_first_hour() {
    let census = start() + 8;
    let edit = census + 5;
    let mut hours: Vec<_> = scenario()
        .into_iter()
        .filter(|tables| (census..census + 14).contains(&tables.hour))
        .collect();
    for tables in &mut hours {
        tables.changes = (tables.hour == edit)
            .then(|| VariantChangeRow {
                sequence: 1,
                hour: edit,
                variant: 1,
                flags: Some([1, 1, 1]),
                ..Default::default()
            })
            .into_iter()
            .collect();
        if tables.hour >= edit {
            tables.definitions[0].obsolete = 1;
            tables.definitions.push(VariantDefinitionRow {
                id: 3,
                equipment_token: 1,
                creator_link: 1,
                display_name: b"new design".to_vec(),
                ..Default::default()
            });
        }
    }
    let (completed, fixture) = prepare(&hours, &[], &[], None);
    assert!(!completed.iter().any(|(page, _)| page.first_hour == edit));
    let page = &completed.last().unwrap().0;
    assert_eq!(page.last_hour, census + 13);
    let empty = page.countries.iter().find(|row| row.tag == "CCC").unwrap();
    let equipment = empty.equipment.as_ref().unwrap();
    assert_eq!(equipment.read_hour, census);
    assert_eq!(equipment.present, Some(false));
    assert!(equipment.amounts.is_empty());
    assert_eq!(equipment.spending_since, census);
    let data = spending(&fixture, &hours, "CCC", Some(edit + 1));
    for field in ["total", "equipmentTotal", "production"] {
        assert_eq!(data[field][0]["firstHour"], census, "{field}");
    }
    assert_eq!(data["production"][0]["emptyCensusHour"], census);
}

fn spending(
    fixture: &Fixture,
    hours: &[Tables],
    tag: &str,
    from: Option<i32>,
) -> serde_json::Value {
    use crate::products::production_spending::{
        Purposes,
        prepared::{Params, read_pages},
    };
    let index = read::read_index(&fixture.store, &fixture.key, 1)
        .unwrap()
        .unwrap();
    let classes = read::Classes::new(&index, Named::default());
    let last = hours.last().unwrap().hour;
    read_pages(
        None,
        &index,
        &classes,
        &fixture.store,
        &fixture.key,
        (hours[0].hour, last),
        &Params {
            branch_id: "1".into(),
            tag: tag.into(),
            refits: false,
            from,
            hour: Some(last),
            end: Some(last),
        },
        last,
        last,
        Purposes::Split,
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn own_spending_change_starts_a_new_country_run() {
    let census = start() + 8;
    let next = census + 24;
    let hours: Vec<_> = scenario()
        .into_iter()
        .filter(|tables| (census..next + 5).contains(&tables.hour))
        .collect();
    for resume in [None, Some(next + 1)] {
        let (completed, fixture) = prepare(&hours, &[], &[next + 1], resume);
        let source = completed
            .last()
            .unwrap()
            .0
            .countries
            .iter()
            .find(|row| row.tag == "AAA")
            .unwrap()
            .equipment
            .as_ref()
            .unwrap();
        assert_eq!(source.spending_since, next);
        let data = spending(&fixture, &hours, "AAA", Some(next + 2));
        assert_eq!(data["equipmentTotal"][0]["firstHour"], next);
    }
}

#[test]
fn spending_stretches_match_across_repeated_flushes_and_resumes() {
    let census = start() + 8;
    let hours: Vec<_> = scenario()
        .into_iter()
        .filter(|tables| (census..census + 29).contains(&tables.hour))
        .collect();
    let (expected, uninterrupted) = prepare(&hours, &[], &[], None);
    let cuts = [census + 3, census + 7, census + 13, census + 25, census + 27];
    let (actual, resumed) = prepare_resumes(&hours, &[], &cuts, &cuts);
    for (page, _) in &actual {
        let expected = &expected
            .iter()
            .find(|(held, _)| (held.first_hour..=held.last_hour).contains(&page.last_hour))
            .unwrap()
            .0;
        for row in &page.countries {
            let held = expected
                .countries
                .iter()
                .find(|held| held.tag == row.tag)
                .unwrap();
            assert_eq!(row.absent_since, held.absent_since);
            for (source, held) in [
                (&row.equipment, &held.equipment),
                (&row.rocket, &held.rocket),
            ] {
                assert_eq!(
                    source.as_ref().map(|source| source.spending_since),
                    held.as_ref().map(|source| source.spending_since),
                    "{} at {}",
                    row.tag,
                    page.last_hour,
                );
            }
        }
    }
    for tag in ["AAA", "BBB", "CCC"] {
        for from in [None, Some(census + 14), Some(census + 26)] {
            assert_eq!(
                spending(&resumed, &hours, tag, from),
                spending(&uninterrupted, &hours, tag, from),
                "{tag} from {from:?}",
            );
        }
    }
}

#[test]
fn whole_branch_rocket_run_survives_an_unrelated_cut() {
    let census = start() + 8;
    let cut = census + 5;
    let mut hours: Vec<_> = scenario()
        .into_iter()
        .filter(|tables| (census..census + 14).contains(&tables.hour))
        .collect();
    for tables in &mut hours {
        if tables.hour >= cut {
            tables.definitions[0].display_name = b"unrelated name".to_vec();
        }
        tables.changes = (tables.hour == cut)
            .then(|| VariantChangeRow {
                sequence: 1,
                hour: cut,
                variant: 1,
                flags: Some([1, 1, 1]),
                ..Default::default()
            })
            .into_iter()
            .collect();
    }
    for resume in [None, Some(cut + 1)] {
        let (completed, fixture) = prepare(&hours, &[], &[cut + 1], resume);
        assert!(completed.iter().any(|(page, _)| page.first_hour == cut));
        let rocket = completed
            .last()
            .unwrap()
            .0
            .countries
            .iter()
            .find(|row| row.tag == "CCC")
            .unwrap()
            .rocket
            .as_ref()
            .unwrap();
        let read = crate::test_fixtures::slot_hour(TableId::RocketProductionOutput);
        let expected = census + (read - census).rem_euclid(24);
        assert!(expected < cut);
        assert_eq!(rocket.read_hour, expected);
        assert_eq!(rocket.spending_since, expected);
        let data = spending(&fixture, &hours, "CCC", None);
        assert_eq!(data["rocketTotal"][0]["firstHour"], expected);
        assert_eq!(data["rocketTotal"][0]["totalRaw"], "0");
        assert_eq!(data["rocketReads"], serde_json::json!([expected]));
    }
}

#[test]
fn native_production_publishes_what_the_row_walk_publishes() {
    let hours = scenario();
    let first = start();
    prepare(&hours, &[], &[], None);
    // Segments starting before, at and after a read, an accepted edit, the countries' and the
    // output's gaps.
    prepare(
        &hours,
        &[first + 8, first + 30, first + 44, first + 57, first + 90],
        &[],
        None,
    );
    prepare(
        &hours,
        &[first + 33, first + 81],
        &[first + 20, first + 45, first + 100],
        Some(first + 45),
    );
}

/// A quiet hour hands its branch nothing but its flags, and the branch goes on over a run of
/// them at once.
#[test]
fn quiet_hours_carry_nothing_but_their_flags() {
    let hours = scenario();
    let format = FormatVersion::CURRENT;
    let mut walk = Synthetic::new(columnar::TABLES, 1, hours[0].hour).unwrap();
    let mut world = None;
    let mut projector = Projector::new(format).unwrap();
    let mut out = Vec::new();
    for (ordinal, tables) in (0..).zip(hours.iter().take(30)) {
        write(
            &mut walk,
            tables.hour,
            TableId::Countries,
            tables.countries.as_deref(),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::CountryLinks,
            Some(&tables.links[..]),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::ProductionLines,
            Some(&tables.lines[..]),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::ProductionLineOutput,
            tables.output.as_deref(),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::RocketProductionOutput,
            Some(&tables.rocket[..]),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::VariantDefinitions,
            Some(&tables.definitions[..]),
        );
        write::<replay_data::rows::VariantModuleRow>(
            &mut walk,
            tables.hour,
            TableId::VariantModules,
            Some(&[]),
        );
        write::<replay_data::rows::VariantUpgradeRow>(
            &mut walk,
            tables.hour,
            TableId::VariantUpgrades,
            Some(&[]),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::VariantChanges,
            Some(&tables.changes[..]),
        );
        write(
            &mut walk,
            tables.hour,
            TableId::LexicalTokens,
            Some(&tables.tokens[..]),
        );
        walk.end(|state, manifest, decoded, starts| {
            let world = dashboard_world::World::at(
                &mut world,
                starts,
                &columnar::needs(),
                (format, CaptureMode::Full),
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
    }
    let read = crate::test_fixtures::slot_hour(TableId::ProductionLineOutput) % 24;
    let variants = crate::test_fixtures::slot_hour(TableId::VariantDefinitions) % 24;
    for (offset, hour) in (0..).zip(&out) {
        let dated = offset == 0 || offset == read || offset == variants;
        assert_eq!(
            hour.heap() != 0,
            dated,
            "hour {offset} carries facts only where it is read or starts the walk"
        );
    }
}

#[test]
fn naming_mutations_only_date_changed_selected_operands() {
    let census = crate::test_fixtures::slot_hour(TableId::VariantDefinitions);
    let edit = census + 2;
    let cold = edit + 1;
    let next = census + 24;
    for (selected, rename) in [(false, false), (false, true), (true, true)] {
        let mut hours: Vec<_> = scenario()
            .into_iter()
            .filter(|tables| (census..=next).contains(&tables.hour))
            .collect();
        let baseline = &hours[0];
        let countries = baseline.countries.clone();
        let links = baseline.links.clone();
        let lines = baseline.lines.clone();
        let output = baseline.output.clone();
        let rocket = baseline.rocket.clone();
        let definitions = baseline.definitions.clone();
        for tables in &mut hours {
            tables.countries = countries.clone();
            tables.links = links.clone();
            tables.lines = lines.clone();
            tables.output = output.clone();
            tables.rocket = rocket.clone();
            tables.definitions = definitions.clone();
            tables.definitions.push(VariantDefinitionRow {
                id: 99,
                equipment_token: 1,
                creator_link: 1,
                display_name: b"unselected".to_vec(),
                ..Default::default()
            });
            let variant = if selected { 1 } else { 99 };
            if rename && tables.hour >= edit {
                tables
                    .definitions
                    .iter_mut()
                    .find(|row| row.id == variant)
                    .unwrap()
                    .display_name = b"changed name".to_vec();
            }
            tables.changes = (tables.hour == edit || tables.hour == cold)
                .then(|| VariantChangeRow {
                    sequence: 1,
                    hour: tables.hour,
                    variant,
                    flags: (tables.hour == edit && rename).then_some([1, 1, 1]),
                    ..Default::default()
                })
                .into_iter()
                .collect();
        }
        let (pages, _) = prepare(&hours, &[cold], &[], None);
        assert_eq!(
            pages.iter().any(|(page, _)| page.first_hour == edit),
            selected
        );
        assert!(!pages.iter().any(|(page, _)| page.first_hour == cold));
        for (page, _) in &pages {
            let date = if page.first_hour >= next {
                next
            } else {
                census
            };
            for (&variant, kind) in &page.classifications {
                let expected =
                    if selected && variant == 1 && (edit..next).contains(&page.first_hour) {
                        edit
                    } else {
                        date
                    };
                assert!(
                    kind.names.iter().all(|name| name.first_hour == expected),
                    "selected={selected}, variant={variant}, page={}, expected={expected}",
                    page.first_hour
                );
            }
        }
        assert!(pages.iter().any(|(page, _)| page.first_hour == next));
    }
}

#[test]
fn combined_absence_survives_unrelated_cuts_flushes_and_resume() {
    let census = start() + 8;
    let absent = census + 24;
    let cut = absent + 3;
    let mut hours: Vec<_> = scenario()
        .into_iter()
        .filter(|tables| (census..absent + 10).contains(&tables.hour))
        .collect();
    let countries = hours[0].countries.clone().unwrap();
    // Rocket holds unchanged, so only equipment's absence sets the combined start.
    let rocket = hours[0].rocket.clone();
    for tables in &mut hours {
        if tables.countries.is_none() {
            tables.countries = Some(countries.clone());
        }
        tables.rocket = rocket.clone();
        if tables.hour >= absent {
            tables.output = None;
        }
        if tables.hour >= cut {
            tables.definitions[0].display_name = b"unrelated name".to_vec();
        }
        tables.changes = (tables.hour == cut)
            .then(|| VariantChangeRow {
                sequence: 1,
                hour: cut,
                variant: 1,
                flags: Some([1, 1, 1]),
                ..Default::default()
            })
            .into_iter()
            .collect();
    }
    for resume in [None, Some(absent + 1)] {
        let (pages, fixture) = prepare(&hours, &[], &[absent + 1], resume);
        for (page, _) in &pages {
            if page.last_hour >= absent {
                let row = page.countries.iter().find(|row| row.tag == "AAA").unwrap();
                assert!(row.equipment.is_none());
                assert_eq!(row.absent_since[0], Some(absent));
            }
        }
        // From absent + 2 a query reads the endpoint the flush ends at absent + 1 and a later one.
        for from in [None, Some(census + 2), Some(absent + 2), Some(absent + 5)] {
            let data = spending(&fixture, &hours, "AAA", from);
            let missing = data["total"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| run["reason"] == "notRecorded")
                .unwrap();
            assert_eq!(missing["firstHour"], absent);
            // Presence combines both sources from the same start, whichever endpoints a query reads.
            let presence = data["production"].as_array().unwrap();
            for pair in presence.windows(2) {
                assert!(pair[0]["lastHour"].as_i64() < pair[1]["firstHour"].as_i64());
            }
            if from.is_some_and(|from| from > absent) {
                assert_eq!(presence.len(), 1);
                assert_eq!(presence[0]["firstHour"], absent);
            }
        }
    }
}
