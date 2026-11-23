use super::*;
use content_reader_save::{SaveDocument, naval_missions::PatrolMissionFields};
use replay_data::rows::NavalPatrolMissionInputRow;

pub(super) fn bind(
    frame: &DecodedFrame,
    document: &SaveDocument,
    save: &mut Save,
) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalPatrolMissionInputs>()?;
    if rows.is_empty() {
        return Ok(());
    }
    let lexical = frame.complete::<tables::LexicalTokens>()?;
    let keys = |tokens: &[u32]| -> io::Result<Vec<&[u8]>> {
        tokens
            .iter()
            .map(|token| {
                let mut matches = lexical.iter().filter(|row| row.token == *token);
                let key = matches
                    .next()
                    .filter(|row| !row.literal.is_empty())
                    .ok_or_else(|| invalid("patrol mission save token unavailable"))?;
                if matches.next().is_some() {
                    return Err(invalid("patrol mission save token duplicated"));
                }
                Ok(key.literal.as_slice())
            })
            .collect()
    };
    let patrol = keys(&engine_native_naval::naval_patrol_inputs::SAVE_TOKENS)?;
    let patrol = [patrol[0], patrol[1], patrol[2]];
    save.naval_missions.patrol_inputs = Some(match country_emission_layout(frame) {
        Ok(Some(layout)) => document.project(&layout)?.mission_patrol_inputs(patrol)?,
        _ => document.mission_patrol_inputs(patrol)?,
    });
    if rows
        .iter()
        .any(|row| row.radar_cache.is_some() || row.air_cache.is_some())
    {
        let caches = keys(&engine_native_naval::naval_patrol_inputs::CACHE_SAVE_TOKENS)?;
        let caches = [caches[0], caches[1]];
        save.naval_missions.spotting_caches = Some(match country_emission_layout(frame) {
            Ok(Some(layout)) => document.project(&layout)?.mission_spotting_caches(caches)?,
            _ => document.mission_spotting_caches(caches)?,
        });
    }
    Ok(())
}

fn required<T>(value: Option<T>, label: &str, field: &str) -> io::Result<T> {
    value.ok_or_else(|| invalid(format!("{label}.{field}: demanded native read unavailable")))
}

fn saved_words(
    row: &NavalPatrolMissionInputRow,
    expected: &PatrolMissionFields,
    convoy: bool,
    r: &mut Report,
) -> io::Result<u64> {
    let label = format!("NavalPatrolMissionInputs {}", row.task_force);
    let target = required(row.target, &label, "target")?;
    eq(
        r,
        &format!("{label}.target"),
        target,
        expected.target.unwrap_or(0),
    );
    let live = required(row.target_live, &label, "target_live")?;
    if target == 0 {
        eq(r, &format!("{label}.null target liveness"), live, 0);
    }
    if target != 0 || convoy {
        eq(
            r,
            &format!("{label}.progress"),
            required(row.progress, &label, "progress")?,
            expected.progress.as_ref().map_or(0, |value| value.raw),
        );
        eq(
            r,
            &format!("{label}.chance"),
            required(row.chance, &label, "chance")?,
            expected.chance.as_ref().map_or(0, |value| value.raw),
        );
    }
    Ok(target)
}

pub(super) fn compare(frame: &DecodedFrame, save: &Save, r: &mut Report) -> io::Result<()> {
    let rows = frame.complete::<tables::NavalPatrolMissionInputs>()?;
    let demanded: BTreeSet<_> = frame
        .complete::<tables::NavalMissionLedgers>()?
        .iter()
        .flat_map(|row| row.task_forces.iter().copied())
        .collect();
    let actual: BTreeSet<_> = rows.iter().map(|row| row.task_force).collect();
    if actual.len() != rows.len() {
        return Err(invalid(
            "duplicate full task force in patrol mission inputs",
        ));
    }
    eq(
        r,
        "NavalPatrolMissionInputs distinct actual ledger census",
        actual,
        demanded,
    );
    let tasks: BTreeMap<_, _> = frame
        .complete::<tables::TaskForces>()?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let missions: BTreeMap<_, _> = frame
        .complete::<tables::NavalMissionTaskForces>()?
        .iter()
        .map(|row| (row.task_force, row))
        .collect();
    for row in rows {
        let label = format!("NavalPatrolMissionInputs {}", row.task_force);
        let Some(task) = tasks.get(&row.task_force) else {
            r.difference(format!("{label} TaskForces identity"), "present", "absent");
            continue;
        };
        let convoy = task.mission == 3
            && missions
                .get(&row.task_force)
                .and_then(|mission| mission.raiding_targets)
                .is_some_and(|targets| {
                    targets[0] == Some(true)
                        || targets[0] == Some(false) && targets[1] == Some(true)
                });
        let saved = save.naval_missions.patrol_inputs.as_ref().ok_or_else(|| {
            invalid("patrol mission save fields were not bound to recorded lexical keys")
        })?;
        let Some(expected) = saved.get(&row.task_force) else {
            r.difference(format!("{label} saved identity"), "present", "absent");
            continue;
        };
        let target = saved_words(row, expected, convoy, r)?;
        eq(
            r,
            &format!("{label}.target_live current task-force census"),
            row.target_live,
            Some(u32::from(target != 0 && tasks.contains_key(&target))),
        );
        for (field, value) in [
            ("surface_detection", row.surface_detection),
            ("carrier_detection", row.carrier_detection),
            ("submarine_detection", row.submarine_detection),
            (
                "carrier_submarine_detection",
                row.carrier_submarine_detection,
            ),
        ] {
            required(value, &label, field)?;
        }
        if convoy {
            required(row.surface_visibility, &label, "surface_visibility")?;
            required(row.submarine_visibility, &label, "submarine_visibility")?;
            required(row.formation_class, &label, "formation_class")?;
            let cached = save
                .naval_missions
                .spotting_caches
                .as_ref()
                .and_then(|cache| cache.get(&row.task_force))
                .ok_or_else(|| {
                    invalid(format!(
                        "{label}: retained spotting cache save fields unavailable"
                    ))
                })?;
            for ((field, value), expected) in [
                ("radar_cache", row.radar_cache),
                ("air_cache", row.air_cache),
            ]
            .into_iter()
            .zip(cached)
            {
                let value = required(value, &label, field)?;
                eq(
                    r,
                    &format!("{label}.{field} positive serializer projection"),
                    (value > 0).then_some(value),
                    expected.as_ref().map(|value| value.raw),
                );
                if value <= 0 {
                    r.limitation("the mission writer omits every nonpositive cached value; exact nonpositive values require matched native observation",format!("{label}.{field}"));
                }
            }
        }
    }
    r.compared("NavalPatrolMissionInputs", rows.len());
    r.limitation("the save has no regional mission registration ledger or its order; the distinct census is checked against recorded actual ledger members, and requires matched native observation for independent membership","NavalPatrolMissionInputs.task_force census");
    r.limitation("the save retains full target references without registry resolution; liveness is checked against the complete current hourly task-force census, not inferred from save presence","NavalPatrolMissionInputs.target_live independent saved representation");
    r.limitation("the save stores no current task-force statistic cache; requires matched native observation","NavalPatrolMissionInputs.surface_detection, carrier_detection, submarine_detection, carrier_submarine_detection, surface_visibility, submarine_visibility, formation_class");
    r.limitation("modifier accumulators, contextual read certificates and source-character joins are not serialized; requires matched native observation","NavalPatrolMissionInputs.local_detection_known, local_detection, weather_detection_known, weather_detection, leader, headquarters_known, headquarters, mission_owner, targeted_spotting_known, targeted_spotting");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_save::naval_missions::{PatrolMissionFields, PatrolMissionSave};
    use replay_data::rows::{NavalMissionLedgerRow, NavalMissionTaskForceRow, TaskForceRow};
    fn fixture() -> (DecodedFrame, Save) {
        let (mut frame, mut save, _, _) = fixtures::fixture();
        let row = NavalPatrolMissionInputRow {
            task_force: 11,
            target: Some(91),
            target_live: Some(0),
            progress: Some(-17),
            chance: Some(0),
            surface_detection: Some(3),
            carrier_detection: Some(4),
            submarine_detection: Some(5),
            carrier_submarine_detection: Some(6),
            ..Default::default()
        };
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row]));
        *frame.tables[TableId::NavalMissionLedgers.index()] =
            TableView::Complete(TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 1,
                    region: 2,
                    task_forces: vec![11, 11],
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 11,
                mission: 1,
                ..Default::default()
            }]));
        save.naval_missions.patrol_inputs = Some(PatrolMissionSave::from([(
            11,
            PatrolMissionFields {
                target: Some(91),
                progress: Some(Decimal::parse(b"-0.00017").unwrap()),
                chance: None,
            },
        )]));
        (frame, save)
    }
    #[test]
    fn stale_full_reference_and_signed_words_compare_and_corruption_differs() {
        let (frame, mut save) = fixture();
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let words = save
            .naval_missions
            .patrol_inputs
            .as_mut()
            .unwrap()
            .get_mut(&11)
            .unwrap();
        words.target = Some((1u64 << 32) | 91);
        words.progress = Some(Decimal::parse(b"0.00017").unwrap());
        words.chance = Some(Decimal::parse(b"0.00001").unwrap());
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
    }
    #[test]
    fn target_null_skips_unreached_words_but_a_demanded_fault_fails_coverage() {
        let (mut frame, save) = fixture();
        let mut row = frame
            .complete::<tables::NavalPatrolMissionInputs>()
            .unwrap()[0]
            .clone();
        row.target = Some(0);
        row.target_live = Some(0);
        row.progress = None;
        row.chance = None;
        let mut expected = save.naval_missions.patrol_inputs.as_ref().unwrap()[&11].clone();
        expected.target = None;
        let mut report = Report::default();
        saved_words(&row, &expected, false, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(saved_words(&row, &expected, true, &mut report).is_err());
        row.target = Some(91);
        row.target_live = Some(0);
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row]));
        let mut report = Report::default();
        assert!(
            !report.attempt_table(TableId::NavalPatrolMissionInputs, |report| compare(
                &frame, &save, report
            ))
        );
        assert!(report.outcome().is_err());
    }
    #[test]
    fn ledger_repeats_demand_one_row_and_distinct_missing_member_is_a_difference() {
        let (mut frame, save) = fixture();
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        *frame.tables[TableId::NavalMissionLedgers.index()] =
            TableView::Complete(TableRows::NavalMissionLedgers(vec![
                NavalMissionLedgerRow {
                    owner: 1,
                    region: 2,
                    task_forces: vec![11, 11, 22],
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn liveness_uses_current_full_task_force_census_independently_of_saved_target() {
        let (mut frame, save) = fixture();
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 11,
                    mission: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: 91,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("target_live"));
        let mut row = frame
            .complete::<tables::NavalPatrolMissionInputs>()
            .unwrap()[0]
            .clone();
        row.target_live = Some(1);
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![
                TaskForceRow {
                    id: 11,
                    mission: 1,
                    ..Default::default()
                },
                TaskForceRow {
                    id: (1u64 << 32) | 91,
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
    #[test]
    fn convoy_caches_compare_positive_projection_and_require_each_demanded_word() {
        let (mut frame, mut save) = fixture();
        let mut row = frame
            .complete::<tables::NavalPatrolMissionInputs>()
            .unwrap()[0]
            .clone();
        row.surface_visibility = Some(1);
        row.submarine_visibility = Some(2);
        row.formation_class = Some(0);
        row.radar_cache = Some(-1);
        row.air_cache = Some(700_000);
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 11,
                mission: 3,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![
                NavalMissionTaskForceRow {
                    task_force: 11,
                    raiding_targets: Some([Some(false), Some(true)]),
                    ..Default::default()
                },
            ]));
        save.naval_missions.spotting_caches =
            Some([(11, [None, Some(Decimal::parse(b"7").unwrap())])].into());
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row.clone()]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitation_lines()
                .any(|line| line.contains("radar_cache"))
        );
        row.air_cache = Some(800_000);
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row.clone()]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        row.air_cache = None;
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row]));
        assert!(compare(&frame, &save, &mut Report::default()).is_err());
    }

    #[test]
    fn unknown_priority_trade_target_does_not_demand_transport_payload() {
        let (mut frame, save) = fixture();
        *frame.tables[TableId::TaskForces.index()] =
            TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
                id: 11,
                mission: 3,
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalMissionTaskForces.index()] =
            TableView::Complete(TableRows::NavalMissionTaskForces(vec![
                NavalMissionTaskForceRow {
                    task_force: 11,
                    raiding_targets: Some([None, Some(true)]),
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty());
    }

    #[test]
    fn save_binding_uses_recorded_tokens_and_refuses_ambiguous_dictionary() {
        use replay_data::rows::LexicalTokenRow;
        let (mut frame, mut save) = fixture();
        let mut row = frame
            .complete::<tables::NavalPatrolMissionInputs>()
            .unwrap()[0]
            .clone();
        row.radar_cache = Some(300_000);
        *frame.tables[TableId::NavalPatrolMissionInputs.index()] =
            TableView::Complete(TableRows::NavalPatrolMissionInputs(vec![row]));
        let tokens = engine_native_naval::naval_patrol_inputs::SAVE_TOKENS
            .into_iter()
            .chain(engine_native_naval::naval_patrol_inputs::CACHE_SAVE_TOKENS);
        let mut lexical: Vec<_> = tokens
            .zip([
                b"custom_target".as_slice(),
                b"custom_progress",
                b"custom_chance",
                b"custom_radar",
                b"custom_air",
            ])
            .map(|(token, key)| LexicalTokenRow {
                token,
                literal: key.to_vec(),
            })
            .collect();
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(lexical.clone()));
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![]));
        let document = SaveDocument::read(b"HOI4txt date=1936.1.1.1 game_unique_id=c version=b countries={X={units={fleet={id={id=1 type=61} task_force={id={id=2 type=61} mission={custom_target={id=91 type=61} custom_progress=-0.00017 custom_chance=0.00001 custom_radar=3 custom_air=2}}}}}}").unwrap();
        bind(&frame, &document, &mut save).unwrap();
        let key = (2u64 << 32) | 61;
        let parsed = &save.naval_missions.patrol_inputs.as_ref().unwrap()[&key];
        assert_eq!(parsed.target, Some((91u64 << 32) | 61));
        assert_eq!(parsed.progress.as_ref().unwrap().raw, -17);
        assert_eq!(
            save.naval_missions.spotting_caches.as_ref().unwrap()[&key][0]
                .as_ref()
                .unwrap()
                .raw,
            300_000
        );
        lexical.push(lexical[0].clone());
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(lexical));
        assert!(bind(&frame, &document, &mut save).is_err());
    }
}
