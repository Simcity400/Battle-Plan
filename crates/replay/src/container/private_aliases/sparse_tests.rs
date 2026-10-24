use super::*;

type Sample = (TableView, Vec<(TableId, SharedView)>);

fn column(table: TableId, views: &[TableView]) -> Vec<TableUpdate> {
    views
        .iter()
        .enumerate()
        .map(|(at, view)| {
            if let TableView::NotRecorded { mode } = view {
                return TableUpdate::NotRecorded { mode: *mode };
            }
            let previous = at
                .checked_sub(1)
                .map(|before| (before as i32 + 1, &views[before]));
            update(
                table,
                &plain(view, previous, at == 0 || at == 9).unwrap(),
                at == 0,
            )
            .unwrap()
        })
        .collect()
}

// The reference uses the original whole-table inversion, transform, diff and wire decoder.
fn every_hour(table: TableId, samples: Vec<Sample>) {
    let hours: Vec<_> = (1..=samples.len() as i32).collect();
    let default = default_bits(table);
    let (mut key_marks, mut later_marks, mut previous_marks) = (Vec::new(), Vec::new(), Vec::new());
    put_varint(&mut key_marks, default as u128);
    let mut stored = Vec::new();
    for (at, (view, sources)) in samples.iter().enumerate() {
        let mut marks = Vec::new();
        stored.push((*transform(view, sources, &mut marks, true, default).unwrap()).clone());
        let bytes = encode_marks(&marks, &previous_marks).unwrap();
        if at == 0 {
            key_marks.extend(bytes);
        } else {
            later_marks.extend(bytes);
        }
        previous_marks = marks;
    }
    let raw = column(table, &stored);
    let sources: Vec<_> = dependencies(table)
        .iter()
        .map(|&source| {
            let views: Vec<_> = samples
                .iter()
                .map(|(_, sources)| {
                    (*sources
                        .iter()
                        .find(|(table, _)| *table == source)
                        .unwrap()
                        .1)
                        .clone()
                })
                .collect();
            (source, column(source, &views))
        })
        .collect();
    let run = |candidates| {
        let mut resolution =
            TableResolution::new(table, hours.len(), &key_marks, Some(&later_marks)).unwrap();
        resolution.candidates = candidates;
        resolution.verify = false;
        let mut supply = Own {
            load: |source, _: &mut Check<'_>| {
                Updates::new(
                    sources
                        .iter()
                        .find(|(id, _)| *id == source)
                        .unwrap()
                        .1
                        .clone(),
                )
            },
            loaded: Vec::new(),
        };
        let mut public = None;
        let mut result = Vec::new();
        for (at, raw) in raw.iter().enumerate() {
            let resolved =
                resolution.hour(&hours, at, raw.clone(), &mut supply, &mut Check::disabled());
            if let Err(error) = resolved {
                result.push(Err((error.kind(), error.to_string())));
                break;
            }
            let update = resolution.updates().last().unwrap().clone();
            let view =
                advance_table(table, &update, public.take(), hours[at], at % 3 != 0).unwrap();
            let payload = plain(&view, None, true).unwrap();
            result.push(Ok((update, payload, view.sampled_at(), resolution.bytes)));
            public = matches!(&*view, TableView::Complete(_)).then_some((hours[at], view));
            resolution.release();
        }
        (result, resolution.sparse.hours)
    };
    let (reference, reference_candidates) = run(false);
    let (actual, actual_candidates) = run(true);
    assert_eq!(reference_candidates, 0);
    assert!(
        actual_candidates != 0,
        "{table:?}: sparse path was never exercised"
    );
    assert_eq!(
        actual.len(),
        samples.len(),
        "{table:?}: unexpected early refusal {actual:?}"
    );
    for (at, (actual, reference)) in actual.into_iter().zip(reference).enumerate() {
        assert_eq!(actual, reference, "{table:?} hour {}", hours[at]);
    }
}

fn source(rows: TableRows) -> (TableId, SharedView) {
    (rows.table(), TableView::Complete(rows).into())
}
fn absent(table: TableId) -> (TableId, SharedView) {
    (
        table,
        TableView::Unavailable {
            reason: "independent source refusal".into(),
        }
        .into(),
    )
}

#[test]
fn province_candidates_equal_the_original_every_hour_across_occurrence_and_availability_edits() {
    let samples = (0..18)
        .map(|hour| {
            let (left, right) = match hour {
                0..=1 => (vec![7, 8], vec![]),
                2 => (vec![7, 7, 8], vec![]),
                3 => (vec![7, 8], vec![]),
                4 => (vec![7, 8], vec![7]),
                _ => (vec![8], vec![7, 9]),
            };
            let rows = TableRows::ProvinceUnitOrder(vec![
                rows::ProvinceUnitOrderRow {
                    province: 10,
                    mixed: left,
                    naval: vec![hour],
                },
                rows::ProvinceUnitOrderRow {
                    province: 20,
                    mixed: right,
                    naval: vec![hour + 1],
                },
            ]);
            let province = match hour {
                6 => (
                    TableId::ProvinceUnitOrder,
                    TableView::Truncated { rows, missing: 3 }.into(),
                ),
                7 => absent(TableId::ProvinceUnitOrder),
                8 => (
                    TableId::ProvinceUnitOrder,
                    TableView::NotRecorded {
                        mode: crate::scope::CaptureMode::Map,
                    }
                    .into(),
                ),
                _ => source(rows),
            };
            let location = match hour {
                0..=1 | 3 => 10,
                2 | 4 | 6..=8 => 999,
                _ => 20,
            };
            let rows = TableRows::Divisions(
                [7, 8, 9]
                    .into_iter()
                    .filter(|id| !(*id == 9 && hour < 10) && !(*id == 7 && hour == 12))
                    .map(|id| rows::DivisionRow {
                        id,
                        province: if id == 7 {
                            location
                        } else if id == 8 {
                            if (6..=8).contains(&hour) { 998 } else { 10 }
                        } else {
                            20
                        },
                        organisation: 70_000,
                        ..Default::default()
                    })
                    .collect(),
            );
            let target = match hour {
                14 => TableView::Truncated { rows, missing: 2 },
                15 => TableView::Unavailable {
                    reason: "target read refusal".into(),
                },
                _ => TableView::Complete(rows),
            };
            (target, vec![province, absent(TableId::DivisionStatus)])
        })
        .collect();
    every_hour(TableId::Divisions, samples);
}

#[test]
fn irrelevant_source_fields_and_unmarked_keys_match_the_original_every_hour() {
    for table in [
        TableId::DivisionStatus,
        TableId::DivisionMovement,
        TableId::Divisions,
    ] {
        let samples = (0..18)
            .map(|hour| {
                let panels: Vec<_> = (1..=8)
                    .map(|division| {
                        let mut row = rows::DivisionPanelStatsRow {
                            division,
                            ..Default::default()
                        };
                        row.panel_stats[0] =
                            Some(100_000 + if division == 1 { hour / 3 * 10_000 } else { 0 });
                        row.panel_stats[1] =
                            Some(200_000 + if division == 2 { hour / 4 * 10_000 } else { 0 });
                        row.panel_stats[2] = Some(hour);
                        row.panel_stats[21] =
                            Some(300_000 + if division == 3 { hour / 2 * 10_000 } else { 0 });
                        row
                    })
                    .collect();
                let statuses: Vec<_> = (1..=8)
                    .map(|division| rows::DivisionStatusRow {
                        division,
                        entrenchment: hour,
                        organisation_exact: Some(
                            400_000 + if division == 4 { hour / 5 * 10_000 } else { 0 },
                        )
                        .into(),
                        ..Default::default()
                    })
                    .collect();
                let target = match table {
                    TableId::DivisionStatus => TableRows::DivisionStatus(
                        panels
                            .iter()
                            .map(|panel| rows::DivisionStatusRow {
                                division: panel.division,
                                maximum_hit_points: if panel.division == 1 {
                                    1
                                } else {
                                    panel.panel_stats[0].unwrap()
                                },
                                maximum_organisation: if panel.division == 3 {
                                    1
                                } else {
                                    panel.panel_stats[1].unwrap()
                                },
                                ..Default::default()
                            })
                            .collect(),
                    ),
                    TableId::DivisionMovement => TableRows::DivisionMovement(
                        panels
                            .iter()
                            .map(|panel| rows::DivisionMovementRow {
                                division: panel.division,
                                base_speed: if panel.division == 3 {
                                    None
                                } else {
                                    panel.panel_stats[21]
                                }
                                .into(),
                                ..Default::default()
                            })
                            .collect(),
                    ),
                    _ => TableRows::Divisions(
                        statuses
                            .iter()
                            .map(|status| rows::DivisionRow {
                                id: status.division,
                                province: 20,
                                organisation: if status.division == 4 {
                                    1
                                } else {
                                    exact(status.organisation_exact).unwrap()
                                },
                                ..Default::default()
                            })
                            .collect(),
                    ),
                };
                let sources = match table {
                    TableId::DivisionStatus => vec![
                        source(TableRows::DivisionPanelStats(panels)),
                        absent(TableId::SimulationFeatures),
                    ],
                    TableId::DivisionMovement => {
                        vec![source(TableRows::DivisionPanelStats(panels))]
                    }
                    _ => vec![
                        source(TableRows::DivisionStatus(statuses)),
                        source(TableRows::ProvinceUnitOrder(vec![
                            rows::ProvinceUnitOrderRow {
                                province: 20,
                                mixed: (1..=8).collect(),
                                ..Default::default()
                            },
                        ])),
                    ],
                };
                (TableView::Complete(target), sources)
            })
            .collect();
        every_hour(table, samples);
    }
}

#[test]
fn combat_candidates_project_all_references_and_identity_edits_at_every_hour() {
    let samples = (0..18)
        .map(|hour| {
            let mut panel = rows::DivisionPanelStatsRow {
                division: 7,
                panel_stats: [Some(100_000); 22],
                ..Default::default()
            };
            panel.panel_stats[2] = Some(hour); // Unread field.
            panel.panel_stats[11] = Some(100_000 + hour / 3 * 10_000);
            let status = rows::DivisionStatusRow {
                division: 7,
                strength_factor: 200_000 + hour / 4 * 10_000,
                entrenchment: hour,
                movement_stats: Some(rows::DivisionMovementStats {
                    air_defense: 300_000 + hour / 5 * 10_000,
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            };
            let movement = rows::DivisionMovementRow {
                division: 7,
                source: hour as u32,
                fuel: Some(400_000 + hour / 6 * 10_000).into(),
                ..Default::default()
            };
            let members = (0..8)
                .filter(|ordinal| !(*ordinal == 2 && (5..8).contains(&hour)))
                .map(|ordinal| {
                    let division = if ordinal == 1 && hour >= 4 { 8 } else { 7 };
                    let mut member = LandCombatDivisionRow {
                        combat: if ordinal < 4 { u64::MAX } else { u64::MAX - 1 },
                        side: (ordinal % 2) as u8,
                        ordinal,
                        division,
                        armour: 100_000,
                        piercing: 100_000,
                        reconnaissance: 100_000,
                        initiative: 100_000,
                        strength_combat_factor: if division == 7 {
                            status.strength_factor
                        } else {
                            500_000
                        },
                        air_attack: if division == 7 {
                            300_000 + hour / 5 * 10_000
                        } else {
                            600_000
                        },
                        fuel_inputs: [
                            if division == 7 {
                                400_000 + hour / 6 * 10_000
                            } else {
                                700_000
                            }
                            .into(),
                            0.into(),
                        ],
                        ..Default::default()
                    };
                    member.stat_values = [100_000.into(); 5];
                    if division == 7 {
                        member.stat_values[0] = panel.panel_stats[11].unwrap().into();
                    }
                    if ordinal == 3 {
                        member.strength_combat_factor = 123;
                    }
                    member
                });
            let mut members: Vec<_> = members.collect();
            members.sort_by_key(LandCombatDivisionRow::key);
            let rows = TableRows::LandCombatDivisions(members);
            let target = match hour {
                14 => TableView::Truncated { rows, missing: 2 },
                15 => TableView::Unavailable {
                    reason: "combat read refusal".into(),
                },
                _ => TableView::Complete(rows),
            };
            let status = match hour {
                6 => absent(TableId::DivisionStatus),
                7 => (
                    TableId::DivisionStatus,
                    TableView::NotRecorded {
                        mode: crate::scope::CaptureMode::Map,
                    }
                    .into(),
                ),
                _ => source(TableRows::DivisionStatus(vec![status])),
            };
            (
                target,
                vec![
                    source(TableRows::DivisionPanelStats(vec![panel])),
                    status,
                    source(TableRows::DivisionMovement(vec![movement])),
                ],
            )
        })
        .collect();
    every_hour(TableId::LandCombatDivisions, samples);
}

#[test]
fn relevant_source_masks_exclude_unread_fields_but_include_historical_resets() {
    let rows = TableRows::Divisions(vec![rows::DivisionRow {
        id: 7,
        ..Default::default()
    }]);
    let sparse = Sparse::default();
    let updates = [TableUpdate::Delta {
        baseline: 1,
        changed: vec![rows::DeltaRow {
            key: 7,
            mask: 1,
            values: vec![],
            format: crate::FORMAT_VERSION,
            recorded_fields: 45,
        }],
        deleted: vec![8],
    }];
    let relevant = |key, mask| {
        sparse.relevant(
            TableId::Divisions,
            TableId::DivisionStatus,
            &rows,
            &[],
            3,
            key,
            mask,
        )
    };
    let mut keys = Vec::new();
    assert!(
        source_keys(
            &updates[..],
            0,
            0,
            &relevant,
            &mut keys,
            &mut Check::disabled()
        )
        .unwrap()
    );
    assert!(keys.is_empty());
    let mut historical = updates.clone();
    let TableUpdate::Delta { changed, .. } = &mut historical[0] else {
        unreachable!()
    };
    changed[0].recorded_fields = 42;
    source_keys(
        &historical[..],
        0,
        0,
        &relevant,
        &mut keys,
        &mut Check::disabled(),
    )
    .unwrap();
    assert_eq!(keys.iter().map(|&(key, _)| key).collect::<Vec<_>>(), [7]);
    let TableUpdate::Delta { changed, .. } = &mut historical[0] else {
        unreachable!()
    };
    changed[0].recorded_fields = 65;
    keys.clear();
    source_keys(
        &historical[..],
        0,
        0,
        &relevant,
        &mut keys,
        &mut Check::disabled(),
    )
    .unwrap();
    assert_eq!(keys, [(7, u64::MAX)]);
}

#[test]
fn sparse_alias_refusals_have_the_original_prefix_and_error_precedence() {
    for table in [TableId::Divisions, TableId::LandCombatDivisions] {
        for nonzero in [false, true] {
            let default = if table == TableId::Divisions {
                1
            } else {
                1 << 9
            };
            let mut key_marks = Vec::new();
            put_varint(&mut key_marks, default);
            key_marks.extend(encode_marks(&[], &[]).unwrap());
            let later_marks = [
                encode_marks(&[], &[]).unwrap(),
                encode_marks(&[], &[]).unwrap(),
            ]
            .concat();
            let raw_rows = if table == TableId::Divisions {
                TableRows::Divisions(vec![rows::DivisionRow {
                    id: 7,
                    ..Default::default()
                }])
            } else {
                TableRows::LandCombatDivisions(vec![LandCombatDivisionRow {
                    combat: 99,
                    division: 7,
                    ..Default::default()
                }])
            };
            let source_table = if table == TableId::Divisions {
                TableId::ProvinceUnitOrder
            } else {
                TableId::DivisionStatus
            };
            let first_source = if table == TableId::Divisions {
                TableRows::ProvinceUnitOrder(vec![rows::ProvinceUnitOrderRow {
                    province: 10,
                    mixed: vec![7],
                    ..Default::default()
                }])
            } else {
                TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
                    division: 7,
                    strength_factor: 100_000,
                    ..Default::default()
                }])
            };
            let sources = [
                TableUpdate::Snapshot(first_source),
                TableUpdate::Unchanged { baseline: 1 },
                TableUpdate::Unavailable {
                    reason: "source refusal".into(),
                },
            ];
            let last = if nonzero {
                let mut bad = raw_rows.clone();
                match &mut bad {
                    TableRows::Divisions(rows) => rows[0].province = 1,
                    TableRows::LandCombatDivisions(rows) => rows[0].strength_combat_factor = 1,
                    _ => unreachable!(),
                }
                TableUpdate::Snapshot(bad)
            } else {
                TableUpdate::Unchanged { baseline: 2 }
            };
            let raw = [
                TableUpdate::Snapshot(raw_rows),
                TableUpdate::Unchanged { baseline: 1 },
                last,
            ];
            let run = |candidates| {
                let mut resolution =
                    TableResolution::new(table, 3, &key_marks, Some(&later_marks)).unwrap();
                resolution.candidates = candidates;
                resolution.verify = false;
                let mut supply = Own {
                    load: |source, _: &mut Check<'_>| {
                        assert_eq!(source, source_table);
                        Updates::new(sources.to_vec())
                    },
                    loaded: Vec::new(),
                };
                for (at, raw) in raw.iter().enumerate() {
                    if let Err(error) = resolution.hour(
                        &[1, 2, 3],
                        at,
                        raw.clone(),
                        &mut supply,
                        &mut Check::disabled(),
                    ) {
                        return (resolution.out, at, error.kind(), error.to_string());
                    }
                }
                panic!("marked missing source must refuse")
            };
            let actual = run(true);
            let reference = run(false);
            assert_eq!(actual, reference);
            assert_eq!(actual.1, 2);
            assert!(
                actual.3.contains(if nonzero {
                    "nonzero"
                } else {
                    "source unavailable"
                }),
                "{}",
                actual.3
            );
        }
    }
}

#[test]
fn combat_projection_keeps_each_references_own_marks_and_changed_source_fields() {
    let rows = vec![
        LandCombatDivisionRow {
            combat: 99,
            ordinal: 0,
            division: 7,
            ..Default::default()
        },
        LandCombatDivisionRow {
            combat: 99,
            ordinal: 1,
            division: 7,
            ..Default::default()
        },
    ];
    let (first, second) = (rows[0].key(), rows[1].key());
    let view = TableView::Complete(TableRows::LandCombatDivisions(rows));
    let mut sparse = Sparse::default();
    let TableView::Complete(rows) = &view else {
        unreachable!()
    };
    sparse
        .combat
        .advance(&view, &TableUpdate::Snapshot(rows.clone()))
        .unwrap();
    let marks = [(second, 1)];
    let mut keys = Vec::new();
    sparse
        .combat
        .project(
            &[(7, 1 << 11)],
            TableId::DivisionPanelStats,
            &marks,
            MEMBER_MASK,
            &mut keys,
        )
        .unwrap();
    assert_eq!(keys, [first]);
    keys.clear();
    sparse
        .combat
        .project(
            &[(7, 1 << 12)],
            TableId::DivisionPanelStats,
            &marks,
            MEMBER_MASK,
            &mut keys,
        )
        .unwrap();
    assert_eq!(keys, [first, second]);
    keys.clear();
    sparse
        .combat
        .project(
            &[(7, 1 << 2)],
            TableId::DivisionPanelStats,
            &marks,
            MEMBER_MASK,
            &mut keys,
        )
        .unwrap();
    assert!(keys.is_empty());
}

#[test]
fn production_candidates_match_the_original_every_hour_across_context_and_prefix_edits() {
    let samples = (0..18)
        .map(|hour| {
            let sources: Vec<_> = (1..=3)
                .map(|line| rows::ProductionLineModifiersRow {
                    line,
                    country: (hour / 7) as u32,
                    variant: (hour / 5) as u64,
                    organisation: (hour / 6) as u64,
                    kind: (hour / 9) as u8,
                    active_factories: 2,
                    resource_penalty_raw: hour,
                    organisation_dirty: hour % 2 == 0,
                    factory_efficiencies: vec![100_000 + hour / 3 * 1000, 200_000],
                    ..Default::default()
                })
                .collect();
            let targets = sources
                .iter()
                .map(|source| rows::ProductionLineRow {
                    handle: source.line,
                    country: source.country,
                    kind: source.kind,
                    variant: source.variant,
                    mio: source.organisation,
                    active_factories: 2,
                    max_factories: 2,
                    efficiency_raw: if source.line == 1 {
                        123
                    } else {
                        (source.factory_efficiencies[0] + source.factory_efficiencies[1]) / 2
                    },
                    ..Default::default()
                })
                .collect();
            (
                TableView::Complete(TableRows::ProductionLines(targets)),
                vec![source(TableRows::ProductionLineModifiers(sources))],
            )
        })
        .collect();
    every_hour(TableId::ProductionLines, samples);
}

#[test]
fn modifier_candidates_match_the_original_every_hour_when_only_identity_is_relevant() {
    let samples = (0..18)
        .map(|hour| {
            let sources: Vec<_> = (0..3)
                .map(|ordinal| LandCombatDivisionRow {
                    combat: u64::MAX,
                    ordinal,
                    division: 7 + (hour / 4) as u64,
                    strength_combat_factor: hour,
                    ..Default::default()
                })
                .collect();
            let targets = sources
                .iter()
                .map(|source| rows::LandCombatDivisionModifierRow {
                    combat: source.combat,
                    ordinal: source.ordinal,
                    division: if source.ordinal == 1 {
                        999
                    } else {
                        source.division
                    },
                    ..Default::default()
                })
                .collect();
            (
                TableView::Complete(TableRows::LandCombatDivisionModifiers(targets)),
                vec![source(TableRows::LandCombatDivisions(sources))],
            )
        })
        .collect();
    every_hour(TableId::LandCombatDivisionModifiers, samples);
}

#[test]
fn conditional_grace_candidates_match_the_original_every_hour_with_missing_operands() {
    let samples = (0..18)
        .map(|hour| {
            let define = 144 + hour / 3;
            let features = match hour {
                6 => absent(TableId::SimulationFeatures),
                7 => (
                    TableId::SimulationFeatures,
                    TableView::NotRecorded {
                        mode: crate::scope::CaptureMode::Map,
                    }
                    .into(),
                ),
                _ => source(TableRows::SimulationFeatures(vec![
                    rows::SimulationFeatureRow {
                        dlc_mask: hour as u32,
                        movement_defines: rows::MovementDefines {
                            supply_grace: Some(define),
                            ..Default::default()
                        }
                        .into(),
                        ..Default::default()
                    },
                ])),
            };
            let rows = (1..=3)
                .map(|division| rows::DivisionStatusRow {
                    division,
                    supply_grace_exact: if division == 2 { None } else { Some(123_456) }.into(),
                    supply_grace_remaining: if division != 1 || (6..=7).contains(&hour) {
                        719_000
                    } else {
                        rows::DivisionStatusRow::displayed_grace(123_456, Some(define as i32))
                    }
                    .into(),
                    ..Default::default()
                })
                .collect();
            (
                TableView::Complete(TableRows::DivisionStatus(rows)),
                vec![absent(TableId::DivisionPanelStats), features],
            )
        })
        .collect();
    every_hour(TableId::DivisionStatus, samples);
}

#[test]
fn incremental_province_occurrences_match_original_inversion_at_every_hour() {
    let views: Vec<_> = (0..32)
        .map(|hour| {
            let (left, right) = match hour % 6 {
                0 | 5 => (vec![7, 8], vec![]),
                1 => (vec![7, 7, 8], vec![]),
                2 => (vec![7, 8], vec![7]),
                3 => (vec![8], vec![7]),
                _ => (vec![8], vec![7, 7]),
            };
            let mut rows = vec![rows::ProvinceUnitOrderRow {
                province: 10,
                mixed: left,
                naval: vec![hour],
            }];
            if hour % 9 != 0 {
                rows.push(rows::ProvinceUnitOrderRow {
                    province: 20,
                    mixed: right,
                    naval: vec![],
                });
            }
            let rows = TableRows::ProvinceUnitOrder(rows);
            match hour {
                12 => TableView::Truncated { rows, missing: 1 },
                13 => TableView::Unavailable {
                    reason: "source refused".into(),
                },
                14 => TableView::NotRecorded {
                    mode: crate::scope::CaptureMode::Map,
                },
                _ => TableView::Complete(rows),
            }
        })
        .collect();
    let changes = column(TableId::ProvinceUnitOrder, &views);
    let mut inverse = sparse::Provinces::default();
    for (at, view) in views.iter().enumerate() {
        let sources = [(TableId::ProvinceUnitOrder, view.clone().into())];
        match &changes[at] {
            TableUpdate::Delta {
                changed, deleted, ..
            } => {
                let mut keys: Vec<_> = changed
                    .iter()
                    .filter(|row| row.mask & 1 != 0)
                    .map(|row| row.key)
                    .collect();
                keys.extend_from_slice(deleted);
                keys.sort_unstable();
                keys.dedup();
                inverse.remove(&views[at - 1], &keys);
                inverse.add(view, &keys).unwrap();
            }
            TableUpdate::Unchanged { .. } => {}
            _ => inverse.reset(&sources).unwrap(),
        }
        if at == 18 {
            inverse.reset(&[]).unwrap();
            inverse.ensure(&sources).unwrap();
        }
        let reference = province_members(&sources).unwrap();
        for id in [7, 8, 9, u64::MAX] {
            assert_eq!(
                inverse.unique(id),
                unique_province(&reference, id),
                "hour {at}, handle {id}"
            );
        }
    }
}

#[test]
fn relevant_field_counters_intersect_mixed_changes_with_receiver_marks() {
    let table = TableId::DivisionStatus;
    let source = TableId::DivisionPanelStats;
    let rows = TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
        division: 7,
        ..Default::default()
    }]);
    let sparse = Sparse::default();
    for (exception, changed, expected) in [(14, 3, 1_u64), (13, 3, 2), (15, 3, 0), (14, 2, 0)] {
        let (relevant, operations) = measurement::capture(|| {
            sparse.relevant(table, source, &rows, &[(7, exception)], 15, 7, changed)
        });
        assert_eq!(relevant, expected != 0);
        assert_eq!(
            operations.counter(table, Counter::RelevantSourceRows),
            u64::from(expected != 0)
        );
        assert_eq!(
            operations.counter(table, Counter::RelevantSourceFields),
            u64::from(expected.count_ones())
        );
        assert_eq!(operations.source_mask(table, source), expected);
    }
}

#[test]
fn combat_relevant_field_counters_union_only_demanded_reference_lanes() {
    let table = TableId::LandCombatDivisions;
    let source = TableId::DivisionPanelStats;
    let members: Vec<_> = [7, 7, 8]
        .into_iter()
        .enumerate()
        .map(|(ordinal, division)| LandCombatDivisionRow {
            combat: u64::MAX,
            ordinal: ordinal as u32,
            division,
            ..Default::default()
        })
        .collect();
    let keys: Vec<_> = members.iter().map(row_key).collect();
    let rows = TableRows::LandCombatDivisions(members);
    let view = TableView::Complete(rows.clone());
    let mut sparse = Sparse::default();
    sparse
        .combat
        .advance(&view, &TableUpdate::Snapshot(rows.clone()))
        .unwrap();
    for (second, expected) in [(0, 1_u64 << 11), (1, 1 << 11), (2, (1 << 11) | (1 << 12))] {
        let marks = [
            (keys[0], MEMBER_MASK ^ 1),
            (keys[1], MEMBER_MASK ^ second),
            (keys[2], MEMBER_MASK ^ 4),
        ];
        let (relevant, operations) = measurement::capture(|| {
            sparse.relevant(
                table,
                source,
                &rows,
                &marks,
                MEMBER_MASK,
                7,
                (1 << 11) | (1 << 12) | (1 << 20) | (1 << 2),
            )
        });
        assert!(relevant);
        assert_eq!(operations.counter(table, Counter::RelevantSourceRows), 1);
        assert_eq!(
            operations.counter(table, Counter::RelevantSourceFields),
            u64::from(expected.count_ones())
        );
        assert_eq!(operations.source_mask(table, source), expected);
    }
}

#[test]
fn province_relevant_counters_exclude_index_only_edits_and_include_old_and_new_receivers() {
    let table = TableId::Divisions;
    let source = TableId::ProvinceUnitOrder;
    let source_views: Vec<_> = [
        (vec![7], vec![8]),
        (vec![7, 7], vec![8]),
        (vec![7, 7, 8], vec![]),
    ]
    .into_iter()
    .map(|(left, right)| {
        TableView::Complete(TableRows::ProvinceUnitOrder(vec![
            rows::ProvinceUnitOrderRow {
                province: 10,
                mixed: left,
                ..Default::default()
            },
            rows::ProvinceUnitOrderRow {
                province: 20,
                mixed: right,
                ..Default::default()
            },
        ]))
    })
    .collect();
    let sources = column(source, &source_views);
    let mut supply = Own {
        load: |loaded, _: &mut Check<'_>| {
            assert_eq!(loaded, source);
            Updates::new(sources.clone())
        },
        loaded: Vec::new(),
    };
    let marks = [(7, 1)];
    let mut key_marks = Vec::new();
    put_varint(&mut key_marks, 1);
    key_marks.extend(encode_marks(&marks, &[]).unwrap());
    let later_marks = [
        encode_marks(&marks, &marks).unwrap(),
        encode_marks(&marks, &marks).unwrap(),
    ]
    .concat();
    let mut resolution = TableResolution::new(table, 3, &key_marks, Some(&later_marks)).unwrap();
    resolution.candidates = true;
    resolution.verify = false;
    for at in 0..3 {
        let raw = if at == 0 {
            TableUpdate::Snapshot(TableRows::Divisions(vec![
                rows::DivisionRow {
                    id: 7,
                    province: 99,
                    ..Default::default()
                },
                rows::DivisionRow {
                    id: 8,
                    ..Default::default()
                },
            ]))
        } else {
            TableUpdate::Unchanged {
                baseline: at as i32,
            }
        };
        let (result, operations) = measurement::capture(|| {
            resolution.hour(&[1, 2, 3], at, raw, &mut supply, &mut Check::disabled())
        });
        result.unwrap();
        let expected = if at == 2 { 2 } else { 0 };
        assert_eq!(
            operations.counter(table, Counter::RelevantSourceRows),
            expected
        );
        assert_eq!(
            operations.counter(table, Counter::RelevantSourceFields),
            expected
        );
        assert_eq!(operations.source_mask(table, source), u64::from(at == 2));
        resolution.release();
    }
    assert_eq!(resolution.sparse.hours, 2);
}
