use super::*;

pub(super) fn compare(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalDominanceTaskForceRanges>()?;
    let forces = frame.complete::<tables::TaskForces>()?;
    let forces: BTreeMap<_, _> = forces.iter().map(|row| (row.id, row)).collect();
    let registered: BTreeSet<_> = frame
        .complete::<tables::NavalMissionLedgers>()?
        .iter()
        .flat_map(|ledger| ledger.task_forces.iter().copied())
        .collect();
    let mut fleets = BTreeSet::new();
    let mut saved_fleets = BTreeSet::new();
    for handle in registered {
        let force = forces.get(&handle).ok_or_else(|| {
            invalid("dominance range registration lacks an authenticated task force")
        })?;
        if force.mission == 2 {
            fleets.insert(force.fleet);
        }
        if let Some(force) = save.naval_missions.task_forces.get(&handle) {
            if force.mission == 2 {
                saved_fleets.insert(force.fleet);
            }
        } else {
            r.difference(
                format!("NavalDominanceTaskForceRanges registration {handle}"),
                "present",
                "absent",
            );
        }
    }
    let actual: BTreeSet<_> = rows.iter().map(|row| row.task_force).collect();
    eq(
        r,
        "NavalDominanceTaskForceRanges TaskForces demand census",
        actual.clone(),
        forces
            .values()
            .filter(|force| {
                fleets.contains(&force.fleet) && !MISSION_KINDS.contains(&force.mission)
            })
            .map(|force| force.id)
            .collect(),
    );
    eq(
        r,
        "NavalDominanceTaskForceRanges saved demand census",
        actual,
        save.naval_missions
            .task_forces
            .iter()
            .filter(|(_, force)| {
                saved_fleets.contains(&force.fleet)
                    && !MISSION_KINDS.contains(&(force.mission as u32))
            })
            .map(|(&id, _)| id)
            .collect(),
    );
    let mut unsaved_leaders = 0;
    let mut negative_convoys = 0;
    for row in rows {
        let label = format!("NavalDominanceTaskForceRanges {}", row.task_force);
        let Some(expected) = save.naval_missions.task_forces.get(&row.task_force) else {
            r.difference(label, "present", "absent");
            continue;
        };
        let (unsaved, negative) =
            super::compare_saved_row(row, expected, &save.naval_missions, r, &label);
        unsaved_leaders += unsaved;
        negative_convoys += negative;
    }
    r.compared("NavalDominanceTaskForceRanges", rows.len());
    if negative_convoys != 0 {
        r.limitation("the game omits every nonpositive convoy count; its exact negative value is not represented by the save",
            format!("NavalDominanceTaskForceRanges.convoys: {negative_convoys} negative rows"));
    }
    if unsaved_leaders != 0 {
        r.limitation("the fleet save writer omits its runtime invalid leader reference, whose value the save does not state",
            format!("NavalDominanceTaskForceRanges.leader: {unsaved_leaders} rows"));
    }
    r.limitation("the text save omits regional mission registration; demand is checked against the complete recorded ledgers and independently against saved task-force kinds and fleet membership. Registration itself requires matched native observation",
        "NavalDominanceTaskForceRanges registered strike fleet demand");
    r.limitation("the text save stores no stat-block range or raiding factor, replenishment ratio, local or weather modifier entries, headquarters commander characters, mission owner or mission region range; it stores only the compared headquarters count, fuel pair, underway predicate, positive convoys and fleet leader",
        "NavalDominanceTaskForceRanges range_base, raiding_factor, replenishment, replenishment_limit, local_*, weather_*, mission_owner, headquarters characters, range_fleet, range_begin, range_end");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_save::naval_missions::{Pack, TaskForce};
    use replay_data::rows::{NavalMissionLedgerRow, NavalMissionTaskForceRow, TaskForceRow};

    fn fixture() -> (DecodedFrame, Save) {
        let (mut frame, mut save, _, _) = fixtures::fixture();
        let force_specs = [
            (11, 100, 2),
            (12, 100, 0),
            (13, 100, 4),
            (14, 100, 1),
            (21, 200, 2),
            (22, 200, 0),
        ];
        *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
            force_specs
                .iter()
                .map(|&(id, fleet, mission)| TaskForceRow {
                    id,
                    fleet,
                    mission,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::NavalMissionLedgers.index()] =
            TableView::Complete(TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 1,
                    region: 7,
                    task_forces: vec![14, 11, 11],
                    ..Default::default()
                },
            ]));
        let rows = vec![12, 14]
            .into_iter()
            .map(|task_force| NavalMissionTaskForceRow {
                task_force,
                underway: 1,
                fuel: 123,
                fuel_requested: -17,
                convoys: 2,
                leader: 5,
                headquarters: vec![0].into(),
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::NavalDominanceTaskForceRanges.index()] =
            TableView::Complete(TableRows::NavalDominanceTaskForceRanges(rows));
        save.naval_missions = Pack {
            task_forces: force_specs
                .into_iter()
                .map(|(id, fleet, mission)| {
                    (
                        id,
                        TaskForce {
                            fleet,
                            mission: mission as i32,
                            underway_replenishment: Some(true),
                            fuel: Some(Decimal {
                                raw: 123,
                                fractional_digits: 5,
                                lexeme: Vec::new(),
                            }),
                            requested: Some(Decimal {
                                raw: -17,
                                fractional_digits: 5,
                                lexeme: Vec::new(),
                            }),
                            headquarters: 1,
                            convoys: Some(2),
                        },
                    )
                })
                .collect(),
            fleet_leaders: [(100, Some(5)), (200, Some(5))].into(),
            ..Default::default()
        };
        (frame, save)
    }

    #[test]
    fn exact_demand_keeps_patrol_idle_siblings_and_excludes_strict_or_unregistered_fleets() {
        let (frame, save) = fixture();
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "{:?}",
            report.difference_lines()
        );
        let mut changed = save;
        changed
            .naval_missions
            .task_forces
            .get_mut(&22)
            .unwrap()
            .fleet = 100;
        let mut report = Report::default();
        compare(&frame, &changed, &mut report).unwrap();
        assert!(
            report
                .difference_lines()
                .iter()
                .any(|line| line.contains("saved demand census"))
        );
    }

    #[test]
    fn every_saved_operand_difference_is_reported_and_unsaved_range_is_named() {
        let (frame, mut save) = fixture();
        let task = save.naval_missions.task_forces.get_mut(&12).unwrap();
        task.fuel.as_mut().unwrap().raw += 1;
        task.requested.as_mut().unwrap().raw -= 1;
        task.underway_replenishment = Some(false);
        task.headquarters = 2;
        task.convoys = Some(3);
        save.naval_missions.fleet_leaders.insert(100, Some(6));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        let lines = report.difference_lines();
        for field in [
            ".fuel:",
            ".fuel_requested:",
            ".underway:",
            ".headquarters entries:",
            ".convoys:",
            ".leader:",
        ] {
            assert!(
                lines.iter().any(|line| line.contains(field)),
                "missing {field}: {lines:?}"
            );
        }
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|line| line.contains("range_begin"))
        );
    }

    #[test]
    fn missing_registration_witness_never_proves_an_empty_demand() {
        let (mut frame, save) = fixture();
        *frame.tables[TableId::NavalMissionLedgers.index()] = TableView::Unavailable {
            reason: "registration witness lost".into(),
        };
        assert!(compare(&frame, &save, &mut Report::default()).is_err());
    }
}
