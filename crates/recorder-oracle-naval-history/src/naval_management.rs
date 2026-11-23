use super::{Report, complete, invalid, required_country_tag};
use content_reader_save::{Save, naval_management as saved};
use replay_data::rows::CountryRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

fn equal<A: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: A,
    expected: A,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}
fn missing(report: &mut Report, label: impl AsRef<str>) {
    report.difference(label, "absent", "present");
}

fn canonical_objects<T>(
    report: &Report,
    objects: impl IntoIterator<Item = T>,
    owner: impl Fn(&T) -> &[u8],
) -> Vec<T> {
    objects
        .into_iter()
        .filter(|row| {
            report
                .canonical_country_tags
                .as_ref()
                .is_none_or(|tags| tags.contains(owner(row)))
        })
        .collect()
}

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let mut passed = report.attempt("NavalRepairCountries", |report| {
        countries(frame, &save.naval_management, links, report)
    });
    passed &= report.attempt("NavalRepairFleets", |report| {
        fleets(frame, &save.naval_management, report)
    });
    passed &= report.attempt("NavalRepairTaskForces", |report| {
        tasks(frame, &save.naval_management, tokens, report)
    });
    passed &= report.attempt("NavalTaskForceComposition", |report| {
        composition(frame, &save.naval_management, report)
    });
    passed &= report.attempt("NavalRepairBases", |report| {
        bases(frame, &save.naval_management, report)
    });
    passed &= report.attempt("NavalRepairQueue", |report| {
        queues(frame, &save.naval_management, report)
    });
    passed &= report.attempt("NavalRefitShips", |report| {
        ships(frame, &save.naval_management, report)
    });
    passed &= report.attempt("NavalRefitProgress", |report| progress(frame, save, report));
    passed &= report.attempt("NavalRepairChildren", |report| {
        children(frame, &save.naval_management, report)
    });
    if passed {
        Ok(())
    } else {
        Err(invalid("naval management comparisons failed"))
    }
}

fn countries(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::NavalRepairCountries>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert(tag, row).is_some() {
            return Err(invalid("duplicate naval repair country tag"));
        }
    }
    let expected_countries = report.canonical_countries(&saved.countries);
    for &(tag, expected) in &expected_countries {
        let label = format!("naval repair country {}", String::from_utf8_lossy(tag));
        let Some(row) = actual.remove(tag.as_slice()) else {
            missing(report, label);
            continue;
        };
        equal(
            report,
            format!("{label}.max_factories_for_repair"),
            Some(row.max_factories_for_repair),
            expected.max_factories_for_repair,
        );
        equal(
            report,
            format!("{label}.max_allowed_dockyards_to_repair"),
            Some(row.max_allowed_dockyards_to_repair),
            expected.max_allowed_dockyards_to_repair,
        );
        equal(
            report,
            format!("{label}.num_used_dockyards"),
            Some(row.num_used_dockyards),
            expected.num_used_dockyards,
        );
    }
    for tag in actual.keys() {
        report.difference(
            format!("naval repair country {}", String::from_utf8_lossy(tag)),
            "present",
            "absent",
        );
    }
    report.compared("NavalRepairCountries", expected_countries.len());
    Ok(())
}
fn fleets(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let mut actual: BTreeMap<_, _> = complete::<tables::NavalRepairFleets>(frame)?
        .iter()
        .map(|row| (row.fleet, row))
        .collect();
    let expected_fleets = canonical_objects(report, &saved.fleets, |(_, row)| &row.country);
    for &(&fleet, expected) in &expected_fleets {
        let label = format!("fleet {fleet}.automated_homebase");
        let Some(row) = actual.remove(&fleet) else {
            missing(report, label);
            continue;
        };
        equal(
            report,
            label,
            Some(row.automated_homebase != 0),
            expected.automated_homebase,
        );
    }
    for fleet in actual.keys() {
        report.difference(format!("naval repair fleet {fleet}"), "present", "absent");
    }
    report.compared("NavalRepairFleets", expected_fleets.len());
    Ok(())
}
fn tasks(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    tokens: &BTreeMap<u32, &[u8]>,
    report: &mut Report,
) -> io::Result<()> {
    let census = complete::<tables::TaskForces>(frame)?;
    let mut actual: BTreeMap<_, _> = complete::<tables::NavalRepairTaskForces>(frame)?
        .iter()
        .map(|row| (row.task_force, row))
        .collect();
    let expected_tasks = canonical_objects(report, &saved.task_forces, |(_, row)| &row.country);
    for &(&task, expected) in &expected_tasks {
        let label = format!("task force {task}");
        let Some(row) = actual.remove(&task) else {
            missing(report, label);
            continue;
        };
        equal(
            report,
            format!("{label}.repair_mode"),
            Some(row.repair_mode),
            expected.repair_mode,
        );
        equal(
            report,
            format!("{label}.repair_split"),
            row.repair_split != 0,
            expected.repair_split,
        );
        equal(
            report,
            format!("{label}.repair_target"),
            (row.repair_target != 0).then_some(row.repair_target),
            expected.repair_target,
        );
        equal(
            report,
            format!("{label}.auto_reinforcement"),
            row.auto_reinforcement != 0,
            expected.auto_reinforcement,
        );
        equal(
            report,
            format!("{label}.refit_equipment_variant"),
            (row.refit_variant != 0).then_some(row.refit_variant),
            expected.refit_variant,
        );
        equal(
            report,
            format!("{label}.refit_to_variant_after_repair"),
            (row.refit_after_repair != 0).then_some(row.refit_after_repair),
            expected.refit_after_repair,
        );
        equal(
            report,
            format!("{label}.merge_with_after_repair"),
            (row.merge_after_repair != 0
                && census
                    .binary_search_by_key(&row.merge_after_repair, |task| task.id)
                    .is_ok())
            .then_some(row.merge_after_repair),
            expected.merge_after_repair,
        );
        match expected.composition.as_ref() {
            None => missing(report, format!("{label}.ai_taskforce_composition")),
            Some(composition) => {
                let selected = if row.composition_token
                    == engine_native_naval::naval_management::NO_AI_COMPOSITION_TOKEN
                {
                    None
                } else {
                    Some(tokens.get(&row.composition_token).copied().ok_or_else(|| {
                        invalid("naval AI composition token missing from recorded dictionary")
                    })?)
                };
                equal(
                    report,
                    format!("{label}.ai_taskforce_composition.selection"),
                    selected,
                    composition.template.as_deref(),
                );
            }
        }
    }
    for task in actual.keys() {
        report.difference(
            format!("naval repair task force {task}"),
            "present",
            "absent",
        );
    }
    report.compared("NavalRepairTaskForces", expected_tasks.len());
    Ok(())
}
fn composition(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let mut actual: BTreeMap<(u64, u8), Vec<(u32, saved::Entry)>> = BTreeMap::new();
    for row in complete::<tables::NavalTaskForceComposition>(frame)? {
        if row.kind > 1 {
            return Err(invalid(
                "naval composition kind is neither requirements nor fulfillment",
            ));
        }
        actual.entry((row.task_force, row.kind)).or_default().push((
            row.ordinal,
            saved::Entry {
                sub_unit: row.sub_unit,
                role: row.role,
                amount: row.amount,
            },
        ));
    }
    for (&task, expected) in canonical_objects(report, &saved.task_forces, |(_, row)| &row.country)
    {
        let Some(expected) = &expected.composition else {
            continue;
        };
        for (kind, name, entries) in [
            (0, "requirements", &expected.requirements),
            (1, "fulfillment", &expected.fulfillment),
        ] {
            let label = format!("task force {task}.ai_taskforce_composition.{name}");
            let Some(entries) = entries else {
                missing(report, label);
                continue;
            };
            let mut rows = actual.remove(&(task, kind)).unwrap_or_default();
            rows.sort_by_key(|row| row.0);
            let expected: Vec<_> = entries
                .iter()
                .copied()
                .enumerate()
                .map(|(index, entry)| (index as u32, entry))
                .collect();
            equal(report, label, rows, expected);
        }
    }
    for key in actual.keys() {
        report.difference(format!("naval composition {key:?}"), "present", "absent");
    }
    report.compared(
        "NavalTaskForceComposition",
        complete::<tables::NavalTaskForceComposition>(frame)?.len(),
    );
    report.limitation("the composition's runtime definition preference is not serialized; ordered sub-unit, role and amount entries compare independently", "NavalTaskForceComposition.definition");
    Ok(())
}
fn bases(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let actual: BTreeMap<_, _> = complete::<tables::NavalRepairBases>(frame)?
        .iter()
        .map(|row| (row.province, row))
        .collect();
    let expected_bases = canonical_objects(report, &saved.bases, |row| &row.country);
    for base in &expected_bases {
        let label = format!("naval repair base {}.priority", base.province);
        let Some(row) = actual.get(&(base.province as u32)) else {
            missing(report, label);
            continue;
        };
        equal(report, label, row.priority, base.priority);
    }
    report.compared("NavalRepairBases", expected_bases.len());
    report.limitation("per-base allocated repair dockyards are runtime allocation state, not serialized; country dockyard budget and use compare separately", "NavalRepairBases.allocated_dockyards");
    Ok(())
}
fn queues(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let expected_bases = canonical_objects(report, &saved.bases, |row| &row.country);
    let mut provinces: BTreeMap<_, _> = saved
        .bases
        .iter()
        .map(|base| (base.province as u32, false))
        .collect();
    for base in &expected_bases {
        provinces.insert(base.province as u32, true);
    }
    let mut actual: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
    for row in complete::<tables::NavalRepairQueue>(frame)? {
        if provinces.get(&row.province) == Some(&false) {
            continue;
        }
        actual
            .entry(row.province)
            .or_default()
            .push((row.ordinal, row.ship));
    }
    for base in &expected_bases {
        let mut rows = actual
            .get(&(base.province as u32))
            .cloned()
            .unwrap_or_default();
        rows.sort_by_key(|row| row.0);
        let expected = base
            .ships
            .iter()
            .copied()
            .enumerate()
            .map(|(index, ship)| (index as u32, ship))
            .collect::<Vec<_>>();
        equal(
            report,
            format!("naval base {}.ships_in_repair", base.province),
            rows,
            expected,
        );
    }
    for province in actual.keys().filter(|province| {
        !expected_bases
            .iter()
            .any(|base| base.province as u32 == **province)
    }) {
        report.difference(
            format!("naval base {province}.ships_in_repair"),
            "present",
            "absent",
        );
    }
    report.compared(
        "NavalRepairQueue",
        expected_bases.iter().map(|base| base.ships.len()).sum(),
    );
    Ok(())
}
fn ships(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let lines = complete::<tables::NavalRefitProgress>(frame)?;
    let mut actual: BTreeMap<_, _> = complete::<tables::NavalRefitShips>(frame)?
        .iter()
        .map(|row| (row.ship, row.resolve_line(lines).map(|line| line.line)))
        .collect();
    let expected_ships = canonical_objects(report, &saved.ships, |(_, row)| &row.country);
    for &(&ship, expected) in &expected_ships {
        let value = actual.remove(&ship).flatten();
        equal(
            report,
            format!("ship {ship}.refit_production_line"),
            value,
            expected.refit_production_line,
        );
    }
    for key in actual.keys() {
        report.difference(format!("naval refit ship {key}"), "present", "absent");
    }
    report.compared("NavalRefitShips", expected_ships.len());
    Ok(())
}
fn progress(frame: &DecodedFrame, saved: &Save, report: &mut Report) -> io::Result<()> {
    let mut actual: BTreeMap<_, _> = complete::<tables::NavalRefitProgress>(frame)?
        .iter()
        .map(|row| (row.line, row.progress_raw))
        .collect();
    let countries = saved
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("saved production countries absent"))?;
    let mut count = 0;
    for (_, country) in report.canonical_countries(countries) {
        let production = country
            .production
            .as_ref()
            .ok_or_else(|| invalid("saved production wrapper absent"))?;
        for line in production
            .lines
            .iter()
            .filter(|line| line.kind == engine_native::cproduction_status::equipment_line::KIND_SHIP_REFIT)
        {
            let expected = line.base.produced.as_ref().map_or(0, |value| value.raw);
            let label = format!("naval refit line {}.produced", line.base.id);
            let Some(value) = actual.remove(&line.base.id) else {
                missing(report, label);
                continue;
            };
            equal(report, label, value, expected);
            count += 1;
        }
    }
    for line in actual.keys() {
        report.difference(format!("naval refit progress {line}"), "present", "absent");
    }
    report.compared("NavalRefitProgress", count);
    Ok(())
}

fn children(frame: &DecodedFrame, saved: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let mut actual: BTreeMap<u64, Vec<(u32, u64)>> = BTreeMap::new();
    for row in complete::<tables::NavalRepairChildren>(frame)? {
        actual
            .entry(row.task_force)
            .or_default()
            .push((row.ordinal, row.child));
    }
    for (&task, expected) in canonical_objects(report, &saved.task_forces, |(_, row)| &row.country)
    {
        let mut rows = actual.remove(&task).unwrap_or_default();
        rows.sort_by_key(|row| row.0);
        let expected: Vec<_> = expected
            .repair_children
            .iter()
            .enumerate()
            .map(|(ordinal, child)| (ordinal as u32, *child))
            .collect();
        equal(
            report,
            format!("task force {task}.repair_child"),
            rows,
            expected,
        );
    }
    for task in actual.keys() {
        report.difference(
            format!("task force {task}.repair_child"),
            "present",
            "absent",
        );
    }
    report.compared(
        "NavalRepairChildren",
        complete::<tables::NavalRepairChildren>(frame)?.len(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::Metrics;
    use replay_frame::frame::TableView;
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 1,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        }
    }
    fn saved_ship(line: Option<u64>) -> saved::Ship {
        saved::Ship {
            country: b"MOD".to_vec(),
            refit_production_line: line,
        }
    }
    fn pack() -> saved::Pack {
        let mut pack = saved::Pack::default();
        pack.task_forces.insert(
            7,
            saved::TaskForce {
                country: b"MOD".to_vec(),
                repair_mode: Some(-2),
                repair_split: true,
                repair_target: Some(9),
                auto_reinforcement: false,
                refit_variant: Some(11),
                refit_after_repair: Some(12),
                composition: Some(saved::Composition {
                    requirements: Some(vec![
                        saved::Entry {
                            sub_unit: 70,
                            role: -3,
                            amount: 2,
                        },
                        saved::Entry {
                            sub_unit: 70,
                            role: -3,
                            amount: 5,
                        },
                    ]),
                    fulfillment: Some(Vec::new()),
                    template: None,
                }),
                merge_after_repair: None,
                repair_children: vec![17, 19, 17],
            },
        );
        pack.bases.push(saved::Base {
            country: b"MOD".to_vec(),
            province: 9,
            priority: 3,
            ships: vec![4, 2, 4],
        });
        pack.ships.insert(4, saved_ship(Some(13)));
        pack.ships.insert(2, saved_ship(None));
        pack
    }
    #[test]
    fn naval_management_oracle_scopes_countries_and_objects_to_canonical_emissions() {
        use content_reader_save::{ActualCountry, CountryEmissionIdentity, SaveDocument};

        let fleet = 61 | (1_u64 << 32);
        let task = 61 | (2_u64 << 32);
        let child = 61 | (3_u64 << 32);
        let ship = 51 | (4_u64 << 32);
        let refitting_ship = 51 | (5_u64 << 32);
        let line = 71 | (13_u64 << 32);
        let canonical = r#"MOD={
            production={max_factories_for_repair=3
                ship_refit_lines={id={type=71 id=13} cost=1 priority=0 amount=1
                    requested_factories=1 equipment_variant_index={type=70 id=7}}}
            strategic_navy={max_allowed_dockyards_to_repair=4 num_used_dockyards=2
                naval_base={9={priority=2 ships_in_repair={{type=51 id=4}}}}}
            units={fleet={id={type=61 id=1} automated_homebase=yes
                task_force={id={type=61 id=2} logical_country=MOD mission={mission=0}
                    repair_mode=3 repair_split=yes
                    repair_child={type=61 id=3}
                    ai_taskforce_composition={requirements={sub_units={17} roles={-2} amount={3}}
                        fulfillment={}}
                    ship={id={type=51 id=4} strength=1 organisation=1}
                    ship={id={type=51 id=5} strength=1 organisation=1
                        refit_production_line={type=71 id=13}}}}}}
        "#;
        let reserved = r#"RES={
            strategic_navy={naval_base={19={priority=7 ships_in_repair={{type=51 id=10}}}}}
            units={fleet={id={type=61 id=8} automated_homebase=yes
                task_force={id={type=61 id=9} logical_country=RES mission={mission=0}
                    repair_mode=2 repair_split=yes
                    repair_child={type=61 id=11}
                    ai_taskforce_composition={requirements={sub_units={99} roles={-1} amount={5}}
                        fulfillment={}}
                    ship={id={type=51 id=10} strength=1 organisation=1
                        refit_production_line={type=71 id=12}}}}}}
        "#;
        let mut frame = frame();
        let country = CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"MOD".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(country.id, &country)]);
        *frame.tables[TableId::NavalRepairCountries.index()] =
            TableView::Complete(TableRows::NavalRepairCountries(vec![
                NavalRepairCountryRow {
                    country: 1,
                    max_factories_for_repair: 3,
                    max_allowed_dockyards_to_repair: 4,
                    num_used_dockyards: 2,
                },
            ]));
        *frame.tables[TableId::NavalRepairFleets.index()] =
            TableView::Complete(TableRows::NavalRepairFleets(vec![NavalRepairFleetRow {
                fleet,
                automated_homebase: 1,
            }]));
        *frame.tables[TableId::NavalRepairTaskForces.index()] =
            TableView::Complete(TableRows::NavalRepairTaskForces(vec![
                NavalRepairTaskForceRow {
                    task_force: task,
                    repair_mode: 3,
                    repair_split: 1,
                    composition_token: engine_native_naval::naval_management::NO_AI_COMPOSITION_TOKEN,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::NavalTaskForceComposition.index()] =
            TableView::Complete(TableRows::NavalTaskForceComposition(vec![
                NavalTaskForceCompositionRow {
                    task_force: task,
                    sub_unit: 17,
                    role: -2,
                    amount: 3,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::NavalRepairChildren.index()] =
            TableView::Complete(TableRows::NavalRepairChildren(vec![NavalRepairChildRow {
                task_force: task,
                ordinal: 0,
                child,
            }]));
        *frame.tables[TableId::NavalRefitShips.index()] =
            TableView::Complete(TableRows::NavalRefitShips(vec![NavalRefitShipRow {
                ship: refitting_ship,
                line,
            }]));
        *frame.tables[TableId::NavalRefitProgress.index()] =
            TableView::Complete(TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
                line,
                progress_raw: 0,
            }]));
        *frame.tables[TableId::NavalRepairBases.index()] =
            TableView::Complete(TableRows::NavalRepairBases(vec![NavalRepairBaseRow {
                province: 9,
                priority: 2,
                allocated_dockyards: 1,
            }]));
        *frame.tables[TableId::NavalRepairQueue.index()] =
            TableView::Complete(TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                province: 9,
                ordinal: 0,
                ship,
            }]));

        for aliases in [0, 1, 2] {
            let document = SaveDocument::read(format!(
                "HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 \
                 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 \
                 theater_group_index=6 military_deployment_line_index=7 \
                 military_deployment_conveyor_index=8 equipment_variant_index=9 \
                 country_leader_index=10 navy_id=11 land_combat_id=12 \
                 multiplayer_random_count=-13 countries={{{canonical}{}}} states={{}} provinces={{}}",
                reserved.repeat(aliases)
            ).as_bytes()).unwrap();
            let layout: Vec<_> = std::iter::once(CountryEmissionIdentity {
                physical_slot: 1,
                own_id: 1,
                actual: ActualCountry::Canonical(1),
                tag: b"MOD".to_vec(),
            })
            .chain((0..aliases).map(|ordinal| CountryEmissionIdentity {
                physical_slot: 2 + ordinal as u32,
                own_id: 0,
                actual: ActualCountry::Reserved,
                tag: b"RES".to_vec(),
            }))
            .collect();
            let save = document.project(&layout).unwrap().parse().unwrap();
            assert_eq!(
                save.naval_management.fleets.len(),
                1 + usize::from(aliases != 0)
            );
            assert_eq!(
                save.naval_management.task_forces.len(),
                save.naval_management.fleets.len()
            );
            assert_eq!(
                save.naval_management.ships.len(),
                2 + usize::from(aliases != 0)
            );
            if aliases != 0 {
                *frame.tables[TableId::NavalRepairBases.index()] =
                    TableView::Complete(TableRows::NavalRepairBases(vec![
                        NavalRepairBaseRow {
                            province: 9,
                            priority: 2,
                            allocated_dockyards: 1,
                        },
                        NavalRepairBaseRow {
                            province: 19,
                            priority: 7,
                            allocated_dockyards: 1,
                        },
                    ]));
                *frame.tables[TableId::NavalRepairQueue.index()] =
                    TableView::Complete(TableRows::NavalRepairQueue(vec![
                        NavalRepairQueueRow {
                            province: 9,
                            ordinal: 0,
                            ship,
                        },
                        NavalRepairQueueRow {
                            province: 19,
                            ordinal: 0,
                            ship: 51 | (10_u64 << 32),
                        },
                    ]));
                assert_eq!(
                    save.naval_management.fleets[&(61 | (8_u64 << 32))].country,
                    b"RES"
                );
                assert_eq!(
                    save.naval_management.task_forces[&(61 | (9_u64 << 32))].country,
                    b"RES"
                );
                assert_eq!(
                    save.naval_management.ships[&(51 | (10_u64 << 32))].country,
                    b"RES"
                );
            }
            let run = |frame: &DecodedFrame, save: &Save| {
                let mut report = Report::default();
                report.record_country_emissions(save);
                compare(frame, save, &links, &BTreeMap::new(), &mut report).unwrap();
                report
            };
            let mut report = run(&frame, &save);
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(
                report.failed_comparisons.is_empty(),
                "{:?}",
                report.failed_comparisons
            );
            for table in [
                "NavalRepairCountries",
                "NavalRepairFleets",
                "NavalRepairTaskForces",
                "NavalRepairBases",
                "NavalRepairQueue",
            ] {
                assert_eq!(report.compared_tables[table], 1, "{table}");
            }
            assert_eq!(report.compared_tables["NavalRefitShips"], 2);
            report.refuse_uncaptured_reserved_operands();
            assert_eq!(
                report
                    .failed_comparisons
                    .contains_key("reserved country RES uncaptured save operands"),
                aliases != 0
            );

            let mut changed = save.clone();
            let pack = &mut changed.naval_management;
            pack.countries
                .get_mut(b"MOD".as_slice())
                .unwrap()
                .max_factories_for_repair = Some(9);
            pack.fleets.get_mut(&fleet).unwrap().automated_homebase = Some(false);
            let policy = pack.task_forces.get_mut(&task).unwrap();
            policy.repair_mode = Some(9);
            policy
                .composition
                .as_mut()
                .unwrap()
                .requirements
                .as_mut()
                .unwrap()[0]
                .amount = 9;
            policy.repair_children.clear();
            pack.ships.get_mut(&ship).unwrap().refit_production_line = Some(71 | (12_u64 << 32));
            pack.bases
                .iter_mut()
                .find(|base| base.country == b"MOD")
                .unwrap()
                .priority = 9;
            pack.bases
                .iter_mut()
                .find(|base| base.country == b"MOD")
                .unwrap()
                .ships
                .clear();
            let differences = run(&frame, &changed).differences;
            for field in [
                "max_factories_for_repair",
                "automated_homebase",
                "repair_mode",
                "requirements",
                "repair_child",
                "refit_production_line",
                ".priority",
                "ships_in_repair",
            ] {
                assert!(
                    differences
                        .iter()
                        .any(|difference| difference.contains(field)),
                    "{field}: {differences:?}"
                );
            }
            let mut missing = frame.clone();
            *missing.tables[TableId::NavalRefitShips.index()] =
                TableView::Complete(TableRows::NavalRefitShips(Vec::new()));
            assert!(
                run(&missing, &save)
                    .differences
                    .iter()
                    .any(|difference| difference.contains("refit_production_line"))
            );
            let mut unexpected = frame.clone();
            *unexpected.tables[TableId::NavalRefitShips.index()] =
                TableView::Complete(TableRows::NavalRefitShips(vec![
                    NavalRefitShipRow { ship, line },
                    NavalRefitShipRow {
                        ship: refitting_ship,
                        line,
                    },
                ]));
            assert!(
                run(&unexpected, &save)
                    .differences
                    .iter()
                    .any(|difference| difference.contains("refit_production_line"))
            );
            if aliases != 0 {
                let mut unadmitted = Report::default();
                assert!(compare(&frame, &save, &links, &BTreeMap::new(), &mut unadmitted).is_err());
                for field in [
                    "naval repair country RES",
                    "automated_homebase",
                    "task force",
                    "requirements",
                    "repair_child",
                    "refit_production_line",
                ] {
                    assert!(
                        unadmitted
                            .differences
                            .iter()
                            .any(|difference| difference.contains(field)),
                        "{field}: {:?}",
                        unadmitted.differences
                    );
                }
                assert!(
                    !unadmitted
                        .differences
                        .iter()
                        .any(|difference| difference.contains("base 19"))
                );
                let mut bad_queue = frame.clone();
                *bad_queue.tables[TableId::NavalRepairQueue.index()] =
                    TableView::Complete(TableRows::NavalRepairQueue(vec![
                        NavalRepairQueueRow {
                            province: 9,
                            ordinal: 0,
                            ship,
                        },
                        NavalRepairQueueRow {
                            province: 19,
                            ordinal: 0,
                            ship,
                        },
                    ]));
                let mut strict = Report::default();
                assert!(compare(&bad_queue, &save, &links, &BTreeMap::new(), &mut strict).is_err());
                assert!(
                    strict
                        .differences
                        .iter()
                        .any(|difference| difference.contains("naval base 19"))
                );

                let mut shared = save.clone();
                shared
                    .naval_management
                    .bases
                    .iter_mut()
                    .find(|base| base.country == b"RES")
                    .unwrap()
                    .province = 9;
                let mut shared_frame = frame.clone();
                *shared_frame.tables[TableId::NavalRepairQueue.index()] =
                    TableView::Complete(TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                        province: 9,
                        ordinal: 0,
                        ship,
                    }]));
                assert!(run(&shared_frame, &shared).differences.is_empty());
                *shared_frame.tables[TableId::NavalRepairQueue.index()] =
                    TableView::Complete(TableRows::NavalRepairQueue(vec![
                        NavalRepairQueueRow {
                            province: 9,
                            ordinal: 0,
                            ship: refitting_ship,
                        },
                        NavalRepairQueueRow {
                            province: 99,
                            ordinal: 0,
                            ship,
                        },
                    ]));
                let differences = run(&shared_frame, &shared).differences;
                for province in ["naval base 9.", "naval base 99."] {
                    assert!(
                        differences
                            .iter()
                            .any(|difference| difference.contains(province)),
                        "{differences:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn naval_management_oracle_omits_dead_rejoin_after_detachment_changes_fleet() {
        let mut frame = frame();
        let mut saved = pack();
        let destination =
            u64::from(engine_native::ctask_force::DEFAULT_HANDLE_TYPE) | (9_u64 << 32);
        let mut destination_saved = saved.task_forces[&7].clone();
        destination_saved.repair_children = vec![7];
        saved.task_forces.insert(destination, destination_saved);
        saved.task_forces.get_mut(&7).unwrap().merge_after_repair = Some(destination);
        let source = NavalRepairTaskForceRow {
            task_force: 7,
            repair_mode: -2,
            repair_split: 1,
            repair_target: 9,
            auto_reinforcement: 0,
            refit_variant: 11,
            refit_after_repair: 12,
            merge_after_repair: destination,
            composition_token: engine_native_naval::naval_management::NO_AI_COMPOSITION_TOKEN,
        };
        *frame.tables[TableId::NavalRepairTaskForces.index()] =
            TableView::Complete(TableRows::NavalRepairTaskForces(vec![
                source,
                NavalRepairTaskForceRow {
                    task_force: destination,
                    merge_after_repair: 0,
                    ..source
                },
            ]));
        let run = |frame: &DecodedFrame, saved: &saved::Pack| {
            let mut report = Report::default();
            tasks(frame, saved, &BTreeMap::new(), &mut report).unwrap();
            report
        };
        for fleet in [1, 2] {
            frame.hour = fleet as i32;
            *frame.tables[TableId::TaskForces.index()] =
                TableView::Complete(TableRows::TaskForces(vec![
                    TaskForceRow {
                        id: 7,
                        fleet,
                        ..Default::default()
                    },
                    TaskForceRow {
                        id: destination,
                        fleet: 1,
                        ..Default::default()
                    },
                ]));
            if fleet == 2 {
                saved
                    .task_forces
                    .get_mut(&destination)
                    .unwrap()
                    .repair_children
                    .clear();
            }
            assert!(run(&frame, &saved).differences.is_empty());
        }
        frame.hour += 1;
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 7,
                fleet: 2,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalRepairTaskForces.index()] =
            TableView::Complete(TableRows::NavalRepairTaskForces(vec![source]));
        saved.task_forces.remove(&destination);
        saved.task_forces.get_mut(&7).unwrap().merge_after_repair = None;
        assert!(run(&frame, &saved).differences.is_empty());
        let mut stale_save = saved.clone();
        stale_save
            .task_forces
            .get_mut(&7)
            .unwrap()
            .merge_after_repair = Some(destination);
        assert!(
            run(&frame, &stale_save)
                .differences
                .iter()
                .any(|line| line.contains("merge_with_after_repair"))
        );
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 7,
                    fleet: 2,
                    ..Default::default()
                },
                TaskForceRow {
                    id: destination + 1,
                    ..Default::default()
                },
            ]));
        assert!(run(&frame, &saved).differences.is_empty());
        *frame.tables[TableId::TaskForces.index()] = TableView::Unavailable {
            reason: "synthetic task census fault".into(),
        };
        assert!(tasks(&frame, &saved, &BTreeMap::new(), &mut Report::default()).is_err());
    }

    #[test]
    fn naval_management_oracle_resolves_raw_refit_links_after_cancel_or_hourly_prune() {
        let mut frame = frame();
        let mut saved = saved::Pack::default();
        let line = 71 | (9_u64 << 32);
        saved.ships.insert(4, saved_ship(Some(line)));
        *frame.tables[TableId::NavalRefitShips.index()] =
            TableView::Complete(TableRows::NavalRefitShips(vec![NavalRefitShipRow {
                ship: 4,
                line,
            }]));
        *frame.tables[TableId::NavalRefitProgress.index()] =
            TableView::Complete(TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
                line,
                progress_raw: 123,
            }]));
        let run = |frame: &DecodedFrame, saved: &saved::Pack| {
            let mut report = Report::default();
            ships(frame, saved, &mut report).unwrap();
            report
        };
        assert!(run(&frame, &saved).differences.is_empty());
        *frame.tables[TableId::ProductionLines.index()] =
            TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
                handle: line,
                ..Default::default()
            }]));
        saved.ships.insert(4, saved_ship(None));
        frame.hour += 1;
        for lines in [
            vec![],
            vec![NavalRefitProgressRow {
                line: line + 1,
                progress_raw: 456,
            }],
        ] {
            *frame.tables[TableId::NavalRefitProgress.index()] =
                TableView::Complete(TableRows::NavalRefitProgress(lines));
            assert!(run(&frame, &saved).differences.is_empty());
            let mut stale_save = saved.clone();
            stale_save.ships.insert(4, saved_ship(Some(line)));
            assert!(!run(&frame, &stale_save).differences.is_empty());
        }
        *frame.tables[TableId::NavalRefitProgress.index()] = TableView::Unavailable {
            reason: "synthetic line census fault".into(),
        };
        assert!(ships(&frame, &saved, &mut Report::default()).is_err());
    }

    #[test]
    fn naval_management_oracle_compares_controls_independent_designs_order_and_omission() {
        let mut frame = frame();
        let saved = pack();
        *frame.tables[TableId::NavalRepairTaskForces.index()] =
            TableView::Complete(TableRows::NavalRepairTaskForces(vec![
                NavalRepairTaskForceRow {
                    task_force: 7,
                    repair_mode: -2,
                    repair_split: 1,
                    repair_target: 9,
                    auto_reinforcement: 0,
                    refit_variant: 11,
                    refit_after_repair: 12,
                    merge_after_repair: 0,
                    composition_token: engine_native_naval::naval_management::NO_AI_COMPOSITION_TOKEN,
                },
            ]));
        let rows = [2, 5]
            .into_iter()
            .enumerate()
            .map(|(ordinal, amount)| NavalTaskForceCompositionRow {
                task_force: 7,
                kind: 0,
                ordinal: ordinal as u32,
                sub_unit: 70,
                role: -3,
                definition: -1,
                amount,
            })
            .collect();
        *frame.tables[TableId::NavalTaskForceComposition.index()] =
            TableView::Complete(TableRows::NavalTaskForceComposition(rows));
        *frame.tables[TableId::NavalRepairQueue.index()] =
            TableView::Complete(TableRows::NavalRepairQueue(
                [4, 2, 4]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, ship)| NavalRepairQueueRow {
                        province: 9,
                        ordinal: ordinal as u32,
                        ship,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::NavalRefitShips.index()] =
            TableView::Complete(TableRows::NavalRefitShips(vec![NavalRefitShipRow {
                ship: 4,
                line: 13,
            }]));
        *frame.tables[TableId::NavalRefitProgress.index()] =
            TableView::Complete(TableRows::NavalRefitProgress(vec![NavalRefitProgressRow {
                line: 13,
                progress_raw: 0,
            }]));
        *frame.tables[TableId::NavalRepairChildren.index()] =
            TableView::Complete(TableRows::NavalRepairChildren(
                [17, 19, 17]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, child)| NavalRepairChildRow {
                        task_force: 7,
                        ordinal: ordinal as u32,
                        child,
                    })
                    .collect(),
            ));
        let run = |frame: &DecodedFrame, saved: &saved::Pack| {
            let mut report = Report::default();
            tasks(frame, saved, &BTreeMap::new(), &mut report).unwrap();
            composition(frame, saved, &mut report).unwrap();
            queues(frame, saved, &mut report).unwrap();
            ships(frame, saved, &mut report).unwrap();
            children(frame, saved, &mut report).unwrap();
            report
        };
        assert!(run(&frame, &saved).differences.is_empty());
        let mut missing = saved.clone();
        missing
            .task_forces
            .get_mut(&7)
            .unwrap()
            .composition
            .as_mut()
            .unwrap()
            .requirements = None;
        assert!(!run(&frame, &missing).differences.is_empty());
        let mut changed = saved.clone();
        let row = changed.task_forces.get_mut(&7).unwrap();
        row.refit_after_repair = row.refit_variant;
        row.repair_split = false;
        assert!(run(&frame, &changed).differences.len() >= 2);
        let mut reordered = saved.clone();
        reordered.bases[0].ships.swap(0, 1);
        reordered
            .task_forces
            .get_mut(&7)
            .unwrap()
            .repair_children
            .swap(0, 1);
        reordered
            .task_forces
            .get_mut(&7)
            .unwrap()
            .composition
            .as_mut()
            .unwrap()
            .requirements
            .as_mut()
            .unwrap()
            .reverse();
        assert!(run(&frame, &reordered).differences.len() >= 3);
        *frame.tables[TableId::NavalRepairQueue.index()] =
            TableView::Complete(TableRows::NavalRepairQueue(vec![NavalRepairQueueRow {
                province: 99,
                ordinal: 0,
                ship: 4,
            }]));
        assert!(
            run(&frame, &saved)
                .differences
                .iter()
                .any(|line| line.contains("99"))
        );
    }
}
