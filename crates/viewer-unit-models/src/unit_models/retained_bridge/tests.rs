use super::*;
use replay_data::rows::*;
use replay_frame::frame::{Metrics, SharedView, TableView};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    set(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 101,
                canonical_slot: 2,
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::Countries(vec![
            CountryRow {
                id: 100,
                canonical_slot: 1,
                tag: b"ONE".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 101,
                canonical_slot: 2,
                tag: b"TWO".to_vec(),
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::Provinces(
            (1..=12)
                .map(|id| ProvinceRow {
                    id,
                    owner: 100,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::CountryAppearance(
            [100, 101]
                .map(|country| CountryAppearanceRow {
                    country,
                    graphical_culture: b"culture".to_vec(),
                    ..Default::default()
                })
                .to_vec(),
        ),
    );
    set(
        &mut frame,
        TableRows::Politics(
            [100, 101]
                .map(|country| PoliticsRow {
                    country,
                    ..Default::default()
                })
                .to_vec(),
        ),
    );
    set(
        &mut frame,
        TableRows::LexicalTokens(
            [b"infantry".as_slice(), b"rifles", b"rifles_1"]
                .into_iter()
                .enumerate()
                .map(|(i, literal)| LexicalTokenRow {
                    token: i as u32 + 1,
                    literal: literal.to_vec(),
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::VariantDefinitions(
            (0..3)
                .map(|i| VariantDefinitionRow {
                    id: ((i + 1) << 32) | 70,
                    equipment_token: 3,
                    archetype_token: 2,
                    creator_link: 100,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::VariantUpgrades(vec![
            VariantUpgradeRow {
                variant: (1 << 32) | 70,
                level: 255,
                ..Default::default()
            },
            VariantUpgradeRow {
                variant: (1 << 32) | 70,
                level: 4,
                ..Default::default()
            },
        ]),
    );
    set(
        &mut frame,
        TableRows::AvailableEquipmentVariants(
            [100, 101]
                .into_iter()
                .flat_map(|country| {
                    (0..3).flat_map(move |roster_kind| {
                        (0..4).map(move |ordinal| AvailableEquipmentVariantRow {
                            country,
                            roster_kind,
                            ordinal,
                            variant: ((u64::from(ordinal) % 3 + 1) << 32) | 70,
                        })
                    })
                })
                .collect(),
        ),
    );
    set(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow::default()]),
    );
    set(
        &mut frame,
        TableRows::DivisionTemplateIcons(vec![DivisionTemplateIconRow {
            template: 50,
            country: 100,
            representative: 1,
            subunits: vec![TemplateSubunit {
                token: 1,
                local: 0,
                count: 2,
            }],
            ..Default::default()
        }]),
    );
    set(
        &mut frame,
        TableRows::Clock(vec![ClockRow {
            speed: 4,
            ..Default::default()
        }]),
    );
    frame
}

fn content() -> model_selection::Content {
    model_selection::Content::from_test_documents(&[br"sub_units = { infantry = { type = infantry need = { rifles = 100 } } } equipments = { rifles = { is_archetype = yes type = infantry } rifles_1 = { archetype = rifles } }"])
}

fn assets() -> UnitModelAssets {
    let names = [
        "ONE_infantry_1_entity",
        "TWO_infantry_1_entity",
        "infantry_entity",
        "test_object_entity",
        "transport_entity",
        "destroyer_entity",
    ];
    let descriptors = mesh::parse_descriptors(
        names
            .iter()
            .map(|name| format!("entity = {{ name = {name} state = {{ name = idle }} }}"))
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
        "synthetic.asset",
    )
    .unwrap();
    UnitModelAssets {
        sub_units: content(),
        ships: ships::ShipAssets::test_formation(),
        registered: names.into_iter().map(str::to_owned).collect(),
        roster: Arc::new(mesh::Roster {
            descriptors,
            ..Default::default()
        }),
        bounds: Arc::new(
            (1..=12)
                .map(|id| (id, [id as f32, 0., id as f32 + 1., 1.]))
                .collect(),
        ),
        gates: Gates {
            units: 120.,
            ships: 240.,
            arrows: 500.,
            ..Gates::OFF
        },
        disabled: None,
        ..UnitModelAssets::off(String::new())
    }
}

fn prepare(assets: &UnitModelAssets, frame: &DecodedFrame) -> PreparedModels {
    equipment_graphics::test_support::with_world(frame, &[], |limits| {
        let motions =
            viewer_unit_reads::task_force_visual::TaskForceMotions::read(frame).unwrap();
        let missions =
            viewer_unit_reads::task_force_visual::MissionSkips::read(frame).unwrap();
        assets
            .prepare(PrepareInputs {
                frame,
                limits,
                vision: &Vision::default(),
                etas: &Etas::default(),
                anchor: &|province, _| Some([province as f32, 0., province as f32 * 0.25]),
                world: None,
                terrain: &BTreeMap::new(), routes: None,
                ships: &ShipPlaces {
                    motions: &motions,
                    movement: None,
                    land: &|_| false,
                    missions: &missions,
                    region: &|_| Some(76),
                    scale: &|_, _| 1.,
                },
                localisation: &Localisation::default(),
            })
            .unwrap()
    })
}

fn targets_equal(left: &[Target], right: &[Target]) {
    assert_eq!(left.len(), right.len());
    for (left, right) in left.iter().zip(right) {
        assert_eq!(
            (
                left.province,
                left.identity,
                left.part,
                left.order,
                left.occupant,
                left.kind,
                left.mesh
            ),
            (
                right.province,
                right.identity,
                right.part,
                right.order,
                right.occupant,
                right.kind,
                right.mesh
            )
        );
        assert_eq!(
            left.origin.map(f32::to_bits),
            right.origin.map(f32::to_bits)
        );
        assert_eq!(
            bytemuck::bytes_of(&left.instance),
            bytemuck::bytes_of(&right.instance)
        );
    }
}

fn naval_equal(left: &NavalSources, right: &NavalSources) {
    assert_eq!(left.candidates.len(), right.candidates.len());
    for (a, b) in left.candidates.iter().zip(&right.candidates) {
        assert_eq!(
            (
                a.province,
                a.identity,
                a.ordinal,
                a.slot,
                a.priority,
                a.ours,
                a.class,
                a.land_eligible,
                a.sample_count,
                a.arrival_allowed,
                a.day,
                &a.route
            ),
            (
                b.province,
                b.identity,
                b.ordinal,
                b.slot,
                b.priority,
                b.ours,
                b.class,
                b.land_eligible,
                b.sample_count,
                b.arrival_allowed,
                b.day,
                &b.route
            ),
        );
        assert_eq!(
            [a.next_arc, a.speed, a.min_arc].map(f32::to_bits),
            [b.next_arc, b.speed, b.min_arc].map(f32::to_bits)
        );
    }
    assert_eq!(left.formations.len(), right.formations.len());
    for (a, b) in left.formations.iter().zip(&right.formations) {
        assert_eq!(
            (a.identity, a.province, a.slot),
            (b.identity, b.province, b.slot)
        );
        assert_eq!(a.roster.len(), b.roster.len());
        for (a, b) in a.roster.iter().zip(&b.roster) {
            assert_eq!((a.flags, &a.graphics), (b.flags, &b.graphics));
            for (a, b) in a.targets.iter().zip(&b.targets) {
                match (a, b) {
                    (Some(a), Some(b)) => {
                        targets_equal(std::slice::from_ref(a), std::slice::from_ref(b))
                    }
                    (None, None) => {}
                    _ => panic!("hull attachment membership differs"),
                }
            }
        }
    }
    assert!(left.combats.battles.is_empty() && right.combats.battles.is_empty());
    assert_eq!(left.combats.links, right.combats.links);
    assert_eq!(
        left.combats.roots.keys().collect::<Vec<_>>(),
        right.combats.roots.keys().collect::<Vec<_>>()
    );
    for (a, b) in left
        .combats
        .roots
        .values()
        .zip(right.combats.roots.values())
    {
        assert_eq!(bytemuck::bytes_of(a), bytemuck::bytes_of(b));
    }
}

fn naval_rows(frame: &mut DecodedFrame, hour: i32) {
    let forces: Vec<_> = (1..=4_u64)
        .filter(|id| !(id + hour as u64).is_multiple_of(5))
        .map(|id| TaskForceRow {
            id: (id << 32) | 51,
            province: ((id + hour as u64) % 12 + 1) as u32,
            logical_owner: 100,
            roster_owner: if (id + hour as u64).is_multiple_of(3) {
                101
            } else {
                100
            },
            mission: viewer_foundation::unit_rules::PATROL,
            ..Default::default()
        })
        .collect();
    let hulls: Vec<_> = forces
        .iter()
        .flat_map(|force| {
            (0..3_u64).map(move |at| ShipRow {
                id: force.id + at * 256,
                task_force: force.id,
                ..Default::default()
            })
        })
        .filter(|ship| !(ship.id + hour as u64).is_multiple_of(7))
        .collect();
    set(
        frame,
        TableRows::ShipModels(
            hulls
                .iter()
                .enumerate()
                .map(|(at, ship)| ShipModelRow {
                    ship: ship.id,
                    fallback_sprite: if (hour as usize + at).is_multiple_of(11) {
                        b"missing".to_vec()
                    } else {
                        b"destroyer".to_vec()
                    },
                    formation_flags: if at.is_multiple_of(3) { 1 << 6 } else { 1 << 8 },
                    counter: Some(ShipCounterInputs {
                        definition_valid: 1,
                        roster_ordinal: (at as u32 + hour as u32) % 3,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(frame, TableRows::Ships(hulls));
    set(
        frame,
        TableRows::TaskForceMapMotion(
            forces
                .iter()
                .map(|force| TaskForceMapMotionRow {
                    task_force: force.id,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        frame,
        TableRows::TaskForceCounterInputs(
            forces
                .iter()
                .map(|force| TaskForceCounterInputRow {
                    task_force: force.id,
                    is_in_regions: 1,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        frame,
        TableRows::TaskForceMissions(
            forces
                .iter()
                .map(|force| TaskForceMissionRow {
                    task_force: force.id,
                    current_region: if hour % 13 == 0 { 76 } else { 79 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        frame,
        TableRows::ProvinceUnitOrder(
            (1..=12)
                .map(|province| {
                    let ids: Vec<_> = forces
                        .iter()
                        .filter(|f| f.province == province)
                        .map(|f| f.id)
                        .collect();
                    ProvinceUnitOrderRow {
                        province,
                        mixed: ids.clone(),
                        naval: ids,
                    }
                })
                .collect(),
        ),
    );
    set(frame, TableRows::TaskForces(forces));
}

#[test]
fn retained_bridge_keeps_reference_census_and_cold_hour_records_across_live_changes() {
    content_reader_script::test_game::enter();
    let mut reference = assets();
    let new = assets();
    let base = frame();
    let mut held = base.clone();
    let mut random = 0x19a7_u64;
    let mut admitted = 0;
    let mut naval_admitted = 0;
    let mut all_errors = BTreeMap::new();
    for hour in 0..360_i32 {
        let mut frame = held.clone();
        frame.hour = hour;
        frame.branch = 1 + u64::from(hour > 290);
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        if hour % 11 == 0 {
            let id = INPUTS[(hour as usize / 11) % INPUTS.len()];
            frame.tables[id.index()] = SharedView::from((*frame.tables[id.index()]).clone());
        }
        if hour % 17 == 0 {
            set(
                &mut frame,
                TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                    variant: (1 << 32) | 70,
                    level: (hour % 256) as u8,
                    ..Default::default()
                }]),
            );
        }
        if hour % 23 == 0 {
            held = frame.clone();
        }
        if hour % 29 == 3 {
            frame.tables[TableId::AvailableEquipmentVariants.index()] = TableView::Unavailable {
                reason: format!("hour {hour} unavailable"),
            }
            .into();
        }
        if hour % 31 == 4 {
            let rows = base
                .complete::<tables::VariantDefinitions>()
                .unwrap()
                .to_vec();
            frame.tables[TableId::VariantDefinitions.index()] = TableView::Truncated {
                rows: TableRows::VariantDefinitions(rows),
                missing: 1,
            }
            .into();
        }
        if hour % 37 == 5 {
            let mut rows = base
                .complete::<tables::VariantDefinitions>()
                .unwrap()
                .to_vec();
            rows.push(rows[0].clone());
            set(&mut frame, TableRows::VariantDefinitions(rows));
        }
        if hour % 41 == 6 {
            let mut rows = base
                .complete::<tables::VariantDefinitions>()
                .unwrap()
                .to_vec();
            rows[1].equipment_token = 999;
            set(&mut frame, TableRows::VariantDefinitions(rows));
        }
        if hour % 43 == 7 {
            set(
                &mut frame,
                TableRows::AvailableEquipmentVariants(vec![AvailableEquipmentVariantRow {
                    country: 100,
                    roster_kind: 0,
                    ordinal: 0,
                    variant: 999,
                }]),
            );
        }
        reference.sub_units = content();
        reference.conventional = RetainedBridge::default();
        let direct = bridge::reference::read(&frame, &reference.sub_units)
            .map(|value| value.comparison_state());
        let retained = new
            .conventional
            .read(&frame, &new.sub_units)
            .map(|value| value.prepared().comparison_state());
        assert_eq!(direct, retained, "census hour {hour}");
        let divisions: Vec<_> = (0..24_u64)
            .filter(|id| !(id + random).is_multiple_of(7))
            .map(|id| DivisionRow {
                id: id + 1,
                province: ((id + random) % 12 + 1) as u32,
                template: 50,
                logical_owner: 100,
                roster_owner: if (id + random).is_multiple_of(3) {
                    101
                } else {
                    100
                },
                ..Default::default()
            })
            .collect();
        set(&mut frame, TableRows::Divisions(divisions.clone()));
        set(
            &mut frame,
            TableRows::DivisionEquipment(
                divisions
                    .iter()
                    .map(|row| DivisionEquipmentRow {
                        division: row.id,
                        variant_handle: ((row.id % 3 + 1) << 32) | 70,
                        amount_raw: 100_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            &mut frame,
            TableRows::DivisionStatus(
                divisions
                    .iter()
                    .map(|row| DivisionStatusRow {
                        division: row.id,
                        camouflage: Recorded::Value(Some(DivisionCamouflage { category: None })),
                        exercising: (hour % 5 == 0) as u8,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        if hour % 7 == 0 {
            set(
                &mut frame,
                TableRows::NewDivisionEquipment(vec![DivisionEquipmentRow {
                    division: 1,
                    variant_handle: (3 << 32) | 70,
                    amount_raw: 200_000,
                    ..Default::default()
                }]),
            );
        }
        naval_rows(&mut frame, hour);
        let left = prepare(&reference, &frame);
        let right = prepare(&new, &frame);
        assert_eq!(left.report, right.report, "report hour {hour}");
        for (reason, count) in &left.report.selection_errors {
            *all_errors.entry(reason.clone()).or_insert(0) += count;
        }
        assert_eq!(left.owned, right.owned);
        naval_equal(&left.ships, &right.ships);
        naval_admitted += left
            .ships
            .formations
            .iter()
            .map(|f| f.targets().count())
            .sum::<usize>();
        let left = left.field(&reference);
        let right = right.field(&new);
        targets_equal(&left.targets, &right.targets);
        admitted += left.targets.len();
        let left_library = reference.library();
        let right_library = new.library();
        assert_eq!(
            left_library.index.keys().collect::<Vec<_>>(),
            right_library.index.keys().collect::<Vec<_>>()
        );
        for name in left_library.index.keys() {
            let left_slot = left_library.slot(name).unwrap();
            let right_slot = right_library.slot(name).unwrap();
            assert_eq!(
                left_library.meshes[left_slot],
                right_library.meshes[right_slot]
            );
            assert_eq!(
                left_library.rig_source(left_slot).1,
                right_library.rig_source(right_slot).1
            );
        }
    }
    assert!(
        admitted > 100,
        "must exercise actual placed figures: {all_errors:?}"
    );
    assert!(
        naval_admitted > 100,
        "must exercise real hull attachments: {all_errors:?}"
    );
}

#[test]
fn coarse_selection_scopes_keep_hour_records_and_lazy_join_order() {
    content_reader_script::test_game::enter();
    let plain = assets();
    let profiled = assets();
    let base = frame();
    let mut reached = BTreeSet::new();
    let mut admitted = 0;
    for hour in 0_u32..16 {
        let mut frame = base.clone();
        frame.hour = hour as i32;
        let divisions: Vec<_> = (1..=8_u64)
            .filter(|id| !(id + u64::from(hour)).is_multiple_of(5))
            .map(|id| DivisionRow {
                id,
                province: ((id + u64::from(hour)) % 12 + 1) as u32,
                template: 50,
                logical_owner: 100,
                roster_owner: if id.is_multiple_of(3) { 101 } else { 100 },
                ..Default::default()
            })
            .collect();
        set(
            &mut frame,
            TableRows::DivisionEquipment(
                divisions
                    .iter()
                    .filter(|row| !(row.id + u64::from(hour)).is_multiple_of(3))
                    .map(|row| DivisionEquipmentRow {
                        division: row.id,
                        variant_handle: ((row.id % 3 + 1) << 32) | 70,
                        amount_raw: 100_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(
            &mut frame,
            TableRows::DivisionStatus(
                divisions
                    .iter()
                    .map(|row| DivisionStatusRow {
                        division: row.id,
                        camouflage: Recorded::Value(Some(DivisionCamouflage { category: None })),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        set(&mut frame, TableRows::Divisions(divisions));
        let ordinary = prepare(&plain, &frame);
        let mut spans = BuilderSpans::new(true);
        let measured = equipment_graphics::test_support::with_world(&frame, &[], |limits| {
            profiled.prepare_with_spans(PrepareInputs {
                frame: &frame, limits, vision: &Vision::default(), etas: &Etas::default(),
                anchor: &|province, _| Some([province as f32, 0., province as f32 * 0.25]),
                world: None, terrain: &BTreeMap::new(), routes: None,
                ships: &ShipPlaces {
                    motions: &viewer_unit_reads::task_force_visual::TaskForceMotions::default(),
                    movement: None, land: &|_| false, missions: &Default::default(),
                    region: &|_| None, scale: &|_, _| 1.,
                },
                localisation: &Localisation::default(),
            }, &mut |_| {}, &mut spans).unwrap()
        });
        assert_eq!(ordinary.report, measured.report, "hour {hour}");
        assert_eq!(ordinary.owned, measured.owned);
        let a = ordinary.field(&plain);
        let b = measured.field(&profiled);
        targets_equal(&a.targets, &b.targets);
        admitted += a.targets.len();
        let records = spans.into_records();
        let selection: Vec<_> = records
            .iter()
            .filter(|record| {
                record.name.starts_with("units.selection.")
                    && !matches!(
                        record.name,
                        "units.selection.bridge_read" | "units.selection.geography_read"
                    )
            })
            .map(|record| record.name)
            .collect();
        assert_eq!(
            selection.len(),
            selection.iter().collect::<BTreeSet<_>>().len(),
            "at most one record per stage"
        );
        assert!(selection.len() <= 13);
        reached.extend(selection);
        let bridge = records
            .iter()
            .position(|record| record.name == "units.selection.bridge_read");
        let graphics = records
            .iter()
            .position(|record| record.name == "units.selection.graphics");
        if let (Some(bridge), Some(graphics)) = (bridge, graphics) {
            assert!(bridge < graphics);
        }
    }
    assert!(admitted > 0, "fixture must place real synthetic figures");
    for stage in [
        "units.selection.pair_classify",
        "units.selection.pair_sorts",
        "units.selection.pair_rank",
        "units.selection.graphics.rules",
        "units.selection.graphics.resolve",
        "units.selection.fallback",
        "units.selection.camouflage",
    ] {
        assert!(reached.contains(stage), "must reach {stage}: {reached:?}");
    }
}

#[path = "model_facts_tests.rs"]
mod model_facts;
