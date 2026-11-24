mod convoy_targets;
mod dominance_ranges;
mod patrol_inputs;
use super::eq;
use super::*;
use engine_native_naval::naval_mission_inputs::{MISSION_KINDS, PRESENCE_WINDOW};

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::NavalDominanceTaskForceRanges, |r| {
        dominance_ranges::compare(frame, save, r)
    });
    report.attempt_table(TableId::NavalPatrolMissionInputs, |r| {
        patrol_inputs::compare(frame, save, r)
    });
    report.attempt_table(TableId::NavalMissionTaskForces, |r| {
        task_forces(frame, save, r)
    });
    report.attempt_table(TableId::NavalMissionLedgers, |r| {
        ledger_members(frame, r)?;
        presence(frame, save, r)
    });
    report.attempt_table(TableId::NavalMissionCountries, |r| escort_trees(frame, r));
    report.limitation(
        "the game saves neither the navy ledger order nor the supply hubs; the save also omits country, fleet-leader, headquarters, task-force and region-weather modifier accumulators and modifier metadata",
        "NavalMissionTaskForces.local_*, weather_*; NavalMissionCountries modifiers, hubs and escort_tree; NavalMissionCommanders modifiers",
    );
}

pub fn bind_patrol_save_fields(
    frame: &DecodedFrame,
    document: &content_reader_save::SaveDocument,
    save: &mut Save,
) -> io::Result<()> {
    patrol_inputs::bind(frame, document, save)
}

fn escort_trees(frame: &DecodedFrame, r: &mut Report) -> io::Result<()> {
    use replay_data::rows::{Recorded, check_tree};
    let countries = frame.complete::<tables::NavalMissionCountries>()?;
    let mut definitions = None;
    let features = frame
        .complete::<tables::SimulationFeatures>()
        .ok()
        .and_then(|rows| rows.first())
        .map(|row| row.dlc_mask);
    for row in countries {
        let Recorded::Value(Some(nodes)) = &row.escort_tree else {
            continue;
        };
        if !nodes.is_empty() {
            check_tree(nodes)?;
        }
        let definition = if row.escort.is_some() {
            if definitions.is_none() {
                definitions = Some(frame.complete::<tables::ModifierDefinitions>()?);
            }
            definitions
                .unwrap()
                .iter()
                .find(|row| row.index == engine_executable::modifier_keys::CONVOY_ESCORT_EFFICIENCY)
        } else {
            None
        };
        let expected = match definition {
            Some(definition) => {
                replay_derive::naval_mission_derive::effective(definition, features, row.escort)
                    .map_err(invalid)?
            }
            None if row.escort.is_none() => 0,
            None => return Err(invalid("escort source root modifier metadata unavailable")),
        };
        eq(
            r,
            &format!("NavalMissionCountries {} escort_tree root", row.country),
            nodes.first().map_or(0, |node| node.value),
            expected,
        );
    }
    r.limitation("country modifier source membership and display labels are absent from text saves; requires matched native observation","NavalMissionCountries.escort_tree source leaves");
    Ok(())
}

fn task_forces(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalMissionTaskForces>()?;
    let recorded: BTreeSet<u64> = rows.iter().map(|row| row.task_force).collect();
    eq(
        r,
        "NavalMissionTaskForces TaskForces mission census",
        recorded.clone(),
        frame
            .complete::<tables::TaskForces>()?
            .iter()
            .filter(|row| MISSION_KINDS.contains(&row.mission))
            .map(|row| row.id)
            .collect(),
    );
    let saved = &save.naval_missions;
    eq(
        r,
        "NavalMissionTaskForces saved mission census",
        recorded,
        saved
            .task_forces
            .iter()
            .filter(|(_, task)| {
                u32::try_from(task.mission).is_ok_and(|kind| MISSION_KINDS.contains(&kind))
            })
            .map(|(&id, _)| id)
            .collect(),
    );
    let mut unsaved_leaders = 0;
    let mut negative_convoys = 0;
    for row in rows {
        let label = format!("NavalMissionTaskForces {}", row.task_force);
        let Some(expected) = saved.task_forces.get(&row.task_force) else {
            r.difference(label, "present", "absent");
            continue;
        };
        if expected.mission == 3 {
            let counter = save
                .counter_inputs
                .task_forces
                .get(&row.task_force)
                .ok_or_else(|| invalid("raiding grid saved active predicate missing task force"))?;
            let actual = frame
                .complete::<tables::TaskForceCounterInputs>()?
                .iter()
                .find(|counter| counter.task_force == row.task_force)
                .ok_or_else(|| invalid("raiding grid recorded active predicate unavailable"))?;
            let active = actual.is_in_regions != 0;
            eq(
                r,
                &format!("{label}.raiding active"),
                active,
                counter.is_in_regions.unwrap_or(false),
            );
            if active {
                let fields = saved
                    .raiding_grid
                    .as_ref()
                    .and_then(|rows| rows.get(&row.task_force))
                    .ok_or_else(|| {
                        invalid("retained raiding grid fields were not bound to recorded save keys")
                    })?;
                if let Some(spotting) = row.raiding_spotting {
                    eq(
                        r,
                        &format!("{label}.raiding_spotting"),
                        spotting,
                        fields.spotting.as_ref().map_or(0, |value| value.raw),
                    );
                } else {
                    r.incomplete(
                        "NavalMissionTaskForces",
                        format!("{label}.raiding_spotting: active retained spotting read unavailable"),
                    );
                }
                if let Some(region) = row.raiding_region {
                    eq(
                        r,
                        &format!("{label}.raiding_region"),
                        region,
                        fields.region.unwrap_or(0),
                    );
                } else {
                    r.incomplete(
                        "NavalMissionTaskForces",
                        format!("{label}.raiding_region: active retained region read unavailable"),
                    );
                }
            }
        }
        let (unsaved, negative) = compare_saved_row(row, expected, saved, r, &label);
        unsaved_leaders += unsaved;
        negative_convoys += negative;
    }
    convoy_targets::compare(frame, save, r)?;
    r.compared("NavalMissionTaskForces", rows.len());
    if rows.iter().any(|row| row.raiding_targets.is_some()) {
        r.limitation("the save serializes target references but not whether each reference resolves in the live registry; target liveness requires the matched native-observation check", "NavalMissionTaskForces.raiding_targets");
    }
    if negative_convoys != 0 {
        r.limitation("the game omits every nonpositive convoy count; its exact negative value is not represented by the save",
            format!("NavalMissionTaskForces.convoys: {negative_convoys} negative rows"));
    }
    if unsaved_leaders != 0 {
        r.limitation(
            "the fleet save writer writes leader only when its reference differs from the runtime invalid reference, whose value the save does not state; a fleet without a saved leader is not compared",
            format!("NavalMissionTaskForces.leader: {unsaved_leaders} rows"),
        );
    }
    r.limitation(
        "the text save stores no stat-block range or raiding factor, no underway replenishment ratio pair, no task-force modifier accumulator, no headquarters commander character (naval_headquarter lists each entry's own reference), no mission region range; only their counts or presence are compared",
        "NavalMissionTaskForces range_base, raiding_factor, replenishment, replenishment_limit, local_*, mission_owner (unsaved), headquarters characters, range_fleet, range_begin, range_end",
    );
    Ok(())
}

fn compare_saved_row(
    row: &replay_data::rows::NavalMissionTaskForceRow,
    expected: &content_reader_save::naval_missions::TaskForce,
    saved: &content_reader_save::naval_missions::Pack,
    r: &mut Report,
    label: &str,
) -> (usize, usize) {
    let mut unsaved_leaders = 0;
    let mut negative_convoys = 0;
    eq(
        r,
        &format!("{label}.underway"),
        Some(row.underway != 0),
        expected.underway_replenishment,
    );
    eq(
        r,
        &format!("{label}.fuel"),
        row.fuel,
        expected.fuel.as_ref().map_or(0, |fuel| fuel.raw),
    );
    eq(
        r,
        &format!("{label}.fuel_requested"),
        row.fuel_requested,
        expected.requested.as_ref().map_or(0, |fuel| fuel.raw),
    );
    negative_convoys += usize::from(row.convoys < 0);
    eq(
        r,
        &format!("{label}.convoys"),
        (row.convoys > 0).then_some(row.convoys),
        expected.convoys,
    );
    eq(
        r,
        &format!("{label}.headquarters entries"),
        row.headquarters.len(),
        expected.headquarters,
    );
    match saved.fleet_leaders.get(&expected.fleet) {
        Some(Some(leader)) => eq(r, &format!("{label}.leader"), row.leader, *leader),
        Some(None) => unsaved_leaders += 1,
        None => r.difference(format!("{label} fleet"), "present", "absent"),
    }
    (unsaved_leaders, negative_convoys)
}

fn ledger_members(frame: &DecodedFrame, r: &mut Report) -> io::Result<()> {
    let known: BTreeSet<_> = frame
        .complete::<tables::TaskForces>()?
        .iter()
        .map(|row| row.id)
        .collect();
    let mut keys = BTreeSet::new();
    for row in frame.complete::<tables::NavalMissionLedgers>()? {
        if !keys.insert((row.owner, row.region)) {
            return Err(invalid("duplicate naval mission ledger row"));
        }
        for &handle in &row.task_forces {
            eq(
                r,
                &format!(
                    "NavalMissionLedgers {} region {} reference {handle:#x}",
                    row.owner, row.region
                ),
                known.contains(&handle),
                true,
            );
        }
    }
    r.limitation("the game saves neither ledger membership nor order; the game can register regions outside the accessible list, so TaskForceMissions cannot be used as its oracle",
        "NavalMissionLedgers.task_forces (reference integrity only)");
    Ok(())
}

fn presence(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalMissionLedgers>()?;
    let tags: BTreeMap<_, _> = frame
        .complete::<tables::Countries>()?
        .iter()
        .map(|row| (row.id, row.tag.as_slice()))
        .collect();
    for row in rows {
        let tag = tags
            .get(&row.owner)
            .ok_or_else(|| invalid("NavalMissionLedgers owner missing from Countries"))?;
        let label = format!(
            "NavalMissionLedgers {} region {}",
            String::from_utf8_lossy(tag),
            row.region
        );
        let ring = save
            .naval_missions
            .presence
            .get(*tag)
            .and_then(|rings| rings.get(&row.region));
        let expected = ring.map(|ring| {
            let window = &ring[..ring.len().min(PRESENCE_WINDOW)];
            (
                window.iter().filter(|&&value| value > 0).count() as i32,
                window.iter().filter(|&&value| value >= 0).count() as i32,
            )
        });
        eq(
            r,
            &label,
            (row.ring).then_some((row.positive, row.total)),
            expected,
        );
    }
    r.compared("NavalMissionLedgers", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_save::naval_missions;
    use replay_data::{
        rows::NavalMissionLedgerRow, rows::NavalMissionTaskForceRow, rows::TaskForceRow,
    };

    fn decimal(raw: i64) -> Decimal {
        Decimal {
            raw,
            fractional_digits: 2,
            lexeme: Vec::new(),
        }
    }

    #[test]
    fn escort_tree_witness_checks_effective_root_and_rejects_bad_preorder() {
        use replay_data::rows::{
            ModifierBreakdownNode, ModifierDefinitionRow, NavalMissionCountryRow, Recorded,
        };
        let (mut frame, _, _, _) = fixtures::fixture();
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                ModifierDefinitionRow {
                    index: engine_executable::modifier_keys::CONVOY_ESCORT_EFFICIENCY,
                    ..Default::default()
                },
            ]));
        let row = NavalMissionCountryRow {
            country: 7,
            escort: Some(25_000),
            escort_tree: Recorded::Value(Some(vec![ModifierBreakdownNode {
                value: 25_000,
                children: 0,
                name: b"Source".to_vec(),
            }])),
            ..Default::default()
        };
        *frame.tables[TableId::NavalMissionCountries.index()] =
            TableView::Complete(TableRows::NavalMissionCountries(vec![row.clone()]));
        let mut report = Report::default();
        escort_trees(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let mut changed = row.clone();
        changed.escort = Some(30_000);
        *frame.tables[TableId::NavalMissionCountries.index()] =
            TableView::Complete(TableRows::NavalMissionCountries(vec![changed]));
        let mut report = Report::default();
        escort_trees(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut invalid = row;
        if let Recorded::Value(Some(nodes)) = &mut invalid.escort_tree {
            nodes[0].children = 1;
        }
        *frame.tables[TableId::NavalMissionCountries.index()] =
            TableView::Complete(TableRows::NavalMissionCountries(vec![invalid]));
        assert!(escort_trees(&frame, &mut Report::default()).is_err());
    }

    #[test]
    fn convoy_oracle_follows_positive_only_serializer_and_names_negative_loss() {
        for convoys in [0, -17, 23] {
            let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
            let task = (43_u64 << 32) | 61;
            let fleet = (1_u64 << 32) | 61;
            *frame.tables[TableId::TaskForces.index()] =
                TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                    id: task,
                    fleet,
                    mission: 4,
                    ..Default::default()
                }]));
            *frame.tables[TableId::NavalMissionTaskForces.index()] =
                TableView::Complete(TableRows::NavalMissionTaskForces(vec![
                    NavalMissionTaskForceRow {
                        task_force: task,
                        convoys,
                        ..Default::default()
                    },
                ]));
            save.naval_missions = naval_missions::Pack {
                task_forces: [(
                    task,
                    naval_missions::TaskForce {
                        fleet,
                        mission: 4,
                        underway_replenishment: Some(false),
                        convoys: (convoys > 0).then_some(convoys),
                        ..Default::default()
                    },
                )]
                .into(),
                fleet_leaders: [(fleet, Some(0))].into(),
                ..Default::default()
            };
            let mut report = Report::default();
            task_forces(&frame, &save, &mut report).unwrap();
            assert!(
                report.differences.is_empty(),
                "{:?}",
                report.difference_lines()
            );
            assert_eq!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|line| line.contains("negative rows")),
                convoys < 0
            );
            save.naval_missions
                .task_forces
                .get_mut(&task)
                .unwrap()
                .convoys = Some(if convoys > 0 { convoys - 1 } else { 0 });
            let mut report = Report::default();
            task_forces(&frame, &save, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1);
        }
    }

    #[test]
    fn ledger_oracle_names_unsaved_membership_and_checks_repeated_references() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 11,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 22,
                    ..Default::default()
                },
            ]));
        for (members, differences) in [
            (vec![22, 11, 11], 0),
            (vec![11, 22, 11], 0),
            (vec![11, 99], 1),
        ] {
            *frame.tables[TableId::NavalMissionLedgers.index()] =
                TableView::Complete(TableRows::NavalMissionLedgers(vec![
                    NavalMissionLedgerRow {
                        owner: 7,
                        region: 9,
                        task_forces: members,
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            ledger_members(&frame, &mut report).unwrap();
            assert_eq!(report.differences.len(), differences);
        }
    }

    #[test]
    fn saved_fields_compare_and_a_changed_one_is_a_difference() {
        let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
        let escort = (43_u64 << 32) | 61;
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: escort,
                fleet: (1 << 32) | 61,
                mission: 4,
                ..Default::default()
            }]));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 7,
                tag: b"ENG".to_vec(),
                ..Default::default()
            }]));
        let row = NavalMissionTaskForceRow {
            task_force: escort,
            fuel: 1_632_000,
            fuel_requested: 1_650_000,
            leader: (897 << 32) | 4713,
            headquarters: vec![(5 << 32) | 4713, 0].into(),
            convoys: 210,
            ..Default::default()
        };
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![row]));
        *frame.tables[TableId::NavalMissionLedgers.index()] =
            TableView::Complete(TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 7,
                    region: 9,
                    ring: true,
                    task_forces: vec![escort],
                    positive: 1,
                    total: 3,
                },
                NavalMissionLedgerRow {
                    owner: 7,
                    region: 10,
                    ring: false,
                    task_forces: vec![escort],
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![
                replay_data::rows::NavalPatrolMissionInputRow {
                    task_force: escort,
                    target: Some(0),
                    target_live: Some(0),
                    surface_detection: Some(0),
                    carrier_detection: Some(0),
                    submarine_detection: Some(0),
                    carrier_submarine_detection: Some(0),
                    ..Default::default()
                },
            ]));
        save.naval_missions = naval_missions::Pack {
            patrol_inputs: Some([(escort, naval_missions::PatrolMissionFields::default())].into()),
            spotting_caches: None,
            task_forces: [(
                escort,
                naval_missions::TaskForce {
                    fleet: (1 << 32) | 61,
                    mission: 4,
                    underway_replenishment: Some(false),
                    fuel: Some(decimal(1_632_000)),
                    requested: Some(decimal(1_650_000)),
                    headquarters: 2,
                    convoys: Some(210),
                },
            )]
            .into(),
            fleet_leaders: [((1 << 32) | 61, Some((897 << 32) | 4713))].into(),
            presence: [(b"ENG".to_vec(), [(9, vec![-1, 0, 2, 0])].into())].into(),
            raiding_grid: None,
        };
        let mut report = Report::default();
        task_forces(&frame, &save, &mut report).unwrap();
        presence(&frame, &save, &mut report).unwrap();
        patrol_inputs::compare(&frame, &save, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "{:?}",
            report.difference_lines()
        );
        assert!(report.failed_comparisons.is_empty());
        save.naval_missions
            .task_forces
            .get_mut(&escort)
            .unwrap()
            .convoys = Some(209);
        let mut report = Report::default();
        task_forces(&frame, &save, &mut report).unwrap();
        presence(&frame, &save, &mut report).unwrap();
        patrol_inputs::compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
    #[test]
    fn retained_raiding_grid_oracle_compares_active_nonzero_signed_spotting_and_region() {
        let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
        let task = (43_u64 << 32) | 61;
        let fleet = (1_u64 << 32) | 61;
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: task,
                fleet,
                mission: 3,
                ..Default::default()
            }]));
        save.naval_missions = naval_missions::Pack {
            task_forces: [(
                task,
                naval_missions::TaskForce {
                    fleet,
                    mission: 3,
                    underway_replenishment: Some(false),
                    ..Default::default()
                },
            )]
            .into(),
            fleet_leaders: [(fleet, Some(0))].into(),
            raiding_grid: Some(
                [(
                    task,
                    naval_missions::RaidingGridFields {
                        spotting: Some(decimal(-12500)),
                        region: Some(17),
                    },
                )]
                .into(),
            ),
            ..Default::default()
        };
        save.counter_inputs.task_forces.insert(
            task,
            content_reader_save::counter_inputs::TaskForce {
                fleet,
                is_in_regions: Some(true),
                ..Default::default()
            },
        );
        let mut row = NavalMissionTaskForceRow {
            task_force: task,
            raiding_region: Some(17),
            raiding_spotting: Some(-12500),
            raiding_targets: Some([Some(true), Some(false)]),
            ..Default::default()
        };
        let run = |row: NavalMissionTaskForceRow, save: &Save| {
            let mut frame = frame.clone();
            *frame.tables[TableId::NavalMissionTaskForces.index()] =
                TableView::Complete(TableRows::NavalMissionTaskForces(vec![row]));
            *frame.tables[TableId::TaskForceCounterInputs.index()] =
                TableView::Complete(TableRows::TaskForceCounterInputs(vec![
                    replay_data::rows::TaskForceCounterInputRow {
                        task_force: task,
                        is_in_regions: if save.counter_inputs.task_forces[&task]
                            .is_in_regions
                            .unwrap_or(false)
                        {
                            2
                        } else {
                            0
                        },
                        ..Default::default()
                    },
                ]));
            let mut report = Report::default();
            task_forces(&frame, save, &mut report).unwrap();
            report
        };
        let exact = run(row.clone(), &save);
        assert!(
            exact.differences.is_empty(),
            "{:?}",
            exact.difference_lines()
        );
        assert!(
            exact
                .limitations
                .values()
                .flatten()
                .any(|line| line.contains("raiding_targets"))
        );
        assert!(exact.failed_comparisons.is_empty());
        for (region, spotting) in [(None, Some(-12500)), (Some(17), None)] {
            let unread = run(
                NavalMissionTaskForceRow {
                    raiding_region: region,
                    raiding_spotting: spotting,
                    ..row.clone()
                },
                &save,
            );
            assert!(unread.differences.is_empty());
            assert!(
                unread
                    .failed_comparisons
                    .contains_key("NavalMissionTaskForces")
            );
        }
        row.raiding_spotting = row.raiding_spotting.map(|value| value + 1);
        assert_eq!(run(row.clone(), &save).differences.len(), 1);
        row.raiding_region = row.raiding_region.map(|value| value + 1);
        assert_eq!(run(row.clone(), &save).differences.len(), 2);
        row.raiding_region = None;
        row.raiding_spotting = None;
        row.raiding_targets = None;
        save.counter_inputs
            .task_forces
            .get_mut(&task)
            .unwrap()
            .is_in_regions = None;
        save.naval_missions.raiding_grid = None;
        let inactive = run(row, &save);
        assert!(inactive.differences.is_empty() && inactive.failed_comparisons.is_empty());
    }
}
