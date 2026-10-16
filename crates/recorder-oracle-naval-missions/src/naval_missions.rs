//! Naval mission icon operands: the fields the text save stores, and the stated limitations
//! for the rest.
use super::eq;
use super::*;
use engine::naval_mission_inputs::{MISSION_KINDS, PRESENCE_WINDOW};

pub fn compare(frame: &DecodedFrame, save: &Save, report: &mut Report) {
    report.attempt_table(TableId::NavalMissionTaskForces, |r| {
        task_forces(frame, save, r)
    });
    report.attempt_table(TableId::NavalMissionLedgers, |r| {
        ledger_members(frame, r)?;
        presence(frame, save, r)
    });
    report.limitation(
        "the game saves neither the navy ledger order nor the supply hubs; the save also omits country, fleet-leader, headquarters, task-force and region-weather modifier accumulators and modifier metadata",
        "NavalMissionTaskForces.local_*, weather_*; NavalMissionCountries modifiers and hubs; NavalMissionCommanders modifiers",
    );
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
        eq(
            r,
            &format!("{label}.underway"),
            Some(row.underway != 0),
            expected.underway_replenishment,
        );
        // The game omits fuel and requested at zero.
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
        // The naval-mission writer emits the convoy count only when signed-positive.
        // Omission proves nonpositive, not zero; retain a named limitation
        // for the exact negative value the serializer loses.
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
    }
    r.compared("NavalMissionTaskForces", rows.len());
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

/// The game registers fleet regions independently of the narrower accessible
/// list. The save lacks this membership as well as its order. Check reference
/// integrity only, and state that limitation rather than invent a multiset.
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
    use installed_content_save::naval_missions;
    use replay::rows::{NavalMissionLedgerRow, NavalMissionTaskForceRow, TaskForceRow};

    fn decimal(raw: i64) -> Decimal {
        Decimal {
            raw,
            fractional_digits: 2,
            lexeme: Vec::new(),
        }
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
            headquarters: vec![(5 << 32) | 4713, 0],
            convoys: 210,
            ..Default::default()
        };
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![row.clone()]));
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
        save.naval_missions = naval_missions::Pack {
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
        };
        let mut report = Report::default();
        task_forces(&frame, &save, &mut report).unwrap();
        presence(&frame, &save, &mut report).unwrap();
        assert!(
            report.differences.is_empty(),
            "{:?}",
            report.difference_lines()
        );
        assert!(report.failed_comparisons.is_empty());
        // One convoy fewer is a difference, never waived.
        save.naval_missions
            .task_forces
            .get_mut(&escort)
            .unwrap()
            .convoys = Some(209);
        let mut report = Report::default();
        task_forces(&frame, &save, &mut report).unwrap();
        presence(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
