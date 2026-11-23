use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;
use selection::Names;
mod reference;
use reference::Reference;

fn fixture(step: usize) -> (DecodedFrame, UnitModelAssets) {
    let (mut frame, mut assets) = formation_fixture(123, 5, [1 << 8; 12], 0);
    frame.hour = 60_828_000 + [0, 0, 5, 1, 33, -4][step % 6];
    frame.branch = if step.is_multiple_of(7) { 2 } else { 1 };
    for (i, table) in frame.tables.iter_mut().enumerate() {
        if matches!(&**table, TableView::Unavailable { .. }) {
            **table = TableView::Complete(TableRows::empty(TableId::ALL[i]));
        }
    }
    let creator = if step % 4 == 2 { 200 } else { 100 };
    let variant = 7 | if step % 5 == 3 { 1 << 40 } else { 0 };
    let explicit = if step % 13 == 9 {
        b"saved\0\xff".to_vec()
    } else {
        Vec::new()
    };
    replace_table(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 100,
                canonical_slot: 0,
            },
            CountryLinkRow {
                id: 200,
                canonical_slot: 1,
            },
        ]),
    );
    replace_table(
        &mut frame,
        TableRows::Countries(vec![
            CountryRow {
                id: 100,
                canonical_slot: 0,
                tag: b"TEST".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 200,
                canonical_slot: 1,
                tag: b"MOD".to_vec(),
                ..Default::default()
            },
        ]),
    );
    replace_table(
        &mut frame,
        TableRows::CountryAppearance(
            [100, 200]
                .into_iter()
                .map(|country| CountryAppearanceRow {
                    country,
                    graphical_culture: b"synthetic".to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::Politics(
            [100, 200]
                .into_iter()
                .map(|country| PoliticsRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::LexicalTokens(
            [b"ship".as_slice(), b"hull", b"alternate"]
                .into_iter()
                .enumerate()
                .map(|(i, name)| LexicalTokenRow {
                    token: i as u32 + 1,
                    literal: name.to_vec(),
                })
                .collect(),
        ),
    );
    replace_table(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow::default()]),
    );
    replace_table(
        &mut frame,
        TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: variant,
            creator_link: creator,
            equipment_token: 2,
            archetype_token: 2,
            explicit_entity: explicit,
            ..Default::default()
        }]),
    );
    let mut models = rows::<tables::ShipModels>(&frame).unwrap().rows().to_vec();
    for (i, row) in models.iter_mut().enumerate() {
        row.variant = variant;
        row.fallback_sprite = b"shape".to_vec();
        row.formation_flags = if i == 0 { 1 << 6 } else { 1 << 8 };
        if step % 10 == 3 {
            row.counter.as_mut().unwrap().roster_ordinal = (i / 2) as u32;
        }
        if i == 3 {
            row.counter.as_mut().unwrap().definition_valid = 0;
        }
        if i == 5 && step % 11 == 7 {
            row.variant = variant + 1;
        }
        if i == 6 {
            row.variant = 0;
        }
    }
    if step % 10 == 3 {
        models.reverse();
    }
    replace_table(&mut frame, TableRows::ShipModels(models));
    let mut ships = rows::<tables::Ships>(&frame).unwrap().rows().to_vec();
    for (index, ship) in ships.iter_mut().enumerate() {
        ship.class_id = if index == 8 && step % 9 == 6 { 3 } else { 1 };
    }
    if step % 10 == 3 {
        ships.reverse();
    }
    replace_table(&mut frame, TableRows::Ships(ships));
    let mut forces = rows::<tables::TaskForces>(&frame).unwrap().rows().to_vec();
    for force in &mut forces {
        force.roster_owner = if step % 4 == 1 { 200 } else { 100 };
        if step % 11 == 2 {
            force.expeditionary_owner = 999;
        }
    }
    replace_table(&mut frame, TableRows::TaskForces(forces));
    if step == 39 {
        let ids: Vec<u32> = [100, 200].into_iter().chain(201..5199).collect();
        assert_eq!(ids.len(), 5000);
        replace_table(
            &mut frame,
            TableRows::CountryLinks(
                ids.iter()
                    .enumerate()
                    .map(|(slot, &id)| CountryLinkRow {
                        id,
                        canonical_slot: slot as u32,
                    })
                    .collect(),
            ),
        );
        replace_table(
            &mut frame,
            TableRows::Countries(
                ids.iter()
                    .enumerate()
                    .map(|(slot, &id)| CountryRow {
                        id,
                        canonical_slot: slot as u32,
                        tag: format!("SYNTH_{id}").into_bytes(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        replace_table(
            &mut frame,
            TableRows::Politics(
                ids.iter()
                    .map(|&country| PoliticsRow {
                        country,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        replace_table(
            &mut frame,
            TableRows::CountryAppearance(
                ids.iter()
                    .map(|&country| CountryAppearanceRow {
                        country,
                        graphical_culture: b"synthetic".to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
    }
    if step % 6 == 4 {
        replace_table(
            &mut frame,
            TableRows::CountryLocalSubunits(vec![CountryLocalSubunitRow {
                country: 100,
                token: 1,
                requirements: vec![
                    LocalSubunitRequirement {
                        archetype_token: 3,
                        amount_raw: 0,
                    },
                    LocalSubunitRequirement {
                        archetype_token: 2,
                        amount_raw: -1,
                    },
                ],
            }]),
        );
    }
    if step % 17 == 8 {
        *frame.tables[TableId::CountryLocalSubunits.index()] = TableView::Unavailable {
            reason: "synthetic requirements gap".into(),
        };
    }
    if step % 11 == 5 {
        *frame.tables[TableId::AvailableEquipmentVariants.index()] = TableView::Unavailable {
            reason: "late conventional census gap".into(),
        };
    }
    assets.sub_units=model_selection::Content::from_test_documents(&[br"sub_units={ ship={type=infantry need={hull=1}} } equipments={ hull={is_archetype=yes sprite=shape visual_level=1} alternate={is_archetype=yes} }"]);
    let pool = if step % 8 < 4 { "pool_a" } else { "pool_b" };
    let limit = if step % 13 == 11 {
        "synthetic_unknown_limit=yes"
    } else {
        "has_dlc=synthetic_models"
    };
    assets.graphics = content_reader_equipment_graphics::equipment_graphics::Content::from_texts(
        &[format!(
            "default = {{ hull = {{ pool = {{ models_weight = 1000000 models = {{ {pool} }} limit = {{ {limit} }} }} }} }}"
        )
        .as_bytes()],
        &content_reader_equipment_graphics::equipment_graphics::Names {
            countries: Default::default(),
            continents: Default::default(),
        },
    )
    .unwrap();
    assets.registered = [
        "pool_a",
        "pool_b",
        "TEST_shape_1_entity",
        "MOD_shape_1_entity",
        "shape_entity",
        "saved",
        PLACEHOLDER_ENTITY,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if step % 17 == 6 {
        assets
            .registered
            .retain(|name| name == PLACEHOLDER_ENTITY || name == "saved");
    }
    if step % 13 == 7 {
        assets.registered.remove(pool);
    }
    if step % 19 == 10 {
        Arc::make_mut(&mut assets.roster)
            .unreadable_names
            .insert(pool.into(), "synthetic declaration refusal".into());
    }
    assets.gates.scale_up = 4.0;
    assets.gates.ship_scale_down = 2.0;
    assets.gates.naval_movement_speed = 0.75;
    assets.library.write().unwrap().index = assets
        .registered
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .enumerate()
        .map(|(slot, name)| (name, slot))
        .collect();
    (frame, assets)
}

fn target_bytes(out: &mut Vec<u8>, target: &Target) {
    out.extend_from_slice(&target.province.to_le_bytes());
    out.extend_from_slice(&target.identity.to_le_bytes());
    out.extend_from_slice(&target.part.to_le_bytes());
    out.extend_from_slice(&target.order.to_le_bytes());
    out.extend_from_slice(&target.occupant.to_le_bytes());
    out.push(u8::from(
        target.kind == viewer_unit_history::unit_history::Kind::Ship,
    ));
    out.extend_from_slice(&(target.mesh as u64).to_le_bytes());
    out.extend_from_slice(bytemuck::bytes_of(&target.instance));
    for value in target.origin {
        out.extend_from_slice(&value.to_bits().to_le_bytes());
    }
}
fn blob(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    out.extend_from_slice(bytes);
}
fn records(sources: &Sources) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(sources.candidates.len() as u64).to_le_bytes());
    for c in &sources.candidates {
        for value in [
            u64::from(c.province),
            c.identity,
            u64::from(c.ordinal),
            u64::from(c.slot),
            u64::from(c.priority),
            u64::from(c.ours),
            u64::from(c.land_eligible),
            c.sample_count as u64,
            u64::from(c.arrival_allowed),
        ] {
            out.extend_from_slice(&value.to_le_bytes());
        }
        blob(&mut out, format!("{:?}", c.class).as_bytes());
        for value in [c.next_arc, c.speed, c.min_arc] {
            out.extend_from_slice(&value.to_bits().to_le_bytes());
        }
        out.extend_from_slice(&c.day.to_le_bytes());
        out.extend_from_slice(&(c.route.len() as u64).to_le_bytes());
        for value in &c.route {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&(sources.formations.len() as u64).to_le_bytes());
    for formation in &sources.formations {
        out.extend_from_slice(&formation.identity.to_le_bytes());
        out.extend_from_slice(&formation.province.to_le_bytes());
        out.extend_from_slice(&formation.slot.to_le_bytes());
        out.extend_from_slice(&(formation.roster.len() as u64).to_le_bytes());
        for hull in &formation.roster {
            out.extend_from_slice(&hull.flags.to_le_bytes());
            blob(&mut out, &hull.graphics);
            for target in &hull.targets {
                out.push(u8::from(target.is_some()));
                if let Some(target) = target {
                    target_bytes(&mut out, target);
                }
            }
        }
    }
    assert!(
        sources.combats.battles.is_empty()
            && sources.combats.links.is_empty()
            && sources.combats.roots.is_empty()
    );
    out
}

type ChosenNames = Vec<Result<Option<String>, String>>;
type NameTrace = (ChosenNames, Vec<u64>);

fn names(
    frame: &DecodedFrame,
    assets: &UnitModelAssets,
    dlcs: &[&str],
    reference: bool,
) -> NameTrace {
    equipment_graphics::test_support::with_world(frame, dlcs, |limits| {
        let terrain = BTreeMap::new();
        let localisation = Localisation::default();
        let hour = Hour::read(assets, frame, &terrain, limits, &localisation).unwrap();
        let models = rows::<tables::ShipModels>(frame).unwrap();
        let variants = rows::<tables::VariantDefinitions>(frame).unwrap();
        let ships = frame.complete::<tables::Ships>().unwrap();
        let forces = frame.complete::<tables::TaskForces>().unwrap();
        let force = &forces[0];
        let links = frame.complete::<tables::CountryLinks>().unwrap();
        let countries = frame.complete::<tables::Countries>().unwrap();
        let mut pass = selection::Pass::new(assets, frame, Some(&hour));
        let mut direct = Reference::new(assets, Some(&hour));
        let mut names = Vec::new();
        let mut tags = Vec::new();
        let mut ordered: Vec<_> = ships
            .iter()
            .filter(|ship| ship.task_force == force.id)
            .filter_map(|ship| models.rows().iter().find(|model| model.ship == ship.id))
            .collect();
        ordered.sort_by_key(|model| model.counter.as_ref().unwrap().roster_ordinal);
        for row in ordered {
            let graphics = variants
                .rows()
                .iter()
                .find(|variant| variant.id == row.variant)
                .map_or(&[][..], |variant| variant.explicit_entity.as_slice());
            let definition = ships
                .iter()
                .find(|ship| ship.id == row.ship)
                .map(|ship| (force.roster_owner, ship.class_id as u32));
            let tag = || {
                tags.push(row.ship);
                let owner = model_owner(force.roster_owner, force.expeditionary_owner, true);
                let country = links
                    .iter()
                    .find(|link| link.id == owner)
                    .and_then(|link| {
                        countries
                            .iter()
                            .find(|country| country.canonical_slot == link.canonical_slot)
                    })
                    .ok_or("naval_formation_model_country: native model owner is unavailable")?;
                content_reader_parser::parser::text(content_reader_parser::parser::c_string(&country.tag))
                    .map_err(|error| format!("naval_formation_model_country: {error}"))
            };
            names.push(if reference {
                direct.name(tag, row, graphics, definition)
            } else {
                pass.name(tag, row, graphics, definition)
            });
        }
        (names, tags)
    })
}

#[test]
fn names_and_fallback_demands_keep_fixed_roster_order() {
    viewer_scene_base::test_game::enter();
    for (step, expected) in [
        (
            0,
            [
                "pool_a",
                "pool_a",
                "pool_a",
                PLACEHOLDER_ENTITY,
                "pool_a",
                "pool_a",
                "shape_entity",
                "pool_a",
                "pool_a",
                "pool_a",
                "pool_a",
                "pool_a",
            ],
        ),
        (
            3,
            [
                "pool_a",
                "pool_a",
                PLACEHOLDER_ENTITY,
                "pool_a",
                "pool_a",
                "pool_a",
                "pool_a",
                "shape_entity",
                "pool_a",
                "pool_a",
                "pool_a",
                "pool_a",
            ],
        ),
    ] {
        let (frame, assets) = fixture(step);
        let actual = names(&frame, &assets, &["synthetic_models"], false);
        assert_eq!(actual.0, expected.map(|name| Ok(Some(name.to_owned()))));
        assert_eq!(actual.1, [(7_u64 << 32) | 51]);
    }
}

#[test]
fn naval_name_pass_multihour_world_owner_limits_and_every_record_match_direct() {
    viewer_scene_base::test_game::enter();
    let mut saw_success = false;
    let mut saw_refusal = false;
    let mut saw_tag = false;
    let mut chosen = BTreeSet::new();
    let mut histories = [
        viewer_unit_history::unit_history::History::default(),
        viewer_unit_history::unit_history::History::default(),
    ];
    let mut admitted = 0;
    for step in 0_usize..40 {
        let mut arms = Vec::new();
        for (arm, reference) in [true, false].into_iter().enumerate() {
            let (frame, assets) = fixture(step);
            let dlcs = if step.is_multiple_of(2) {
                vec!["synthetic_models"]
            } else {
                Vec::new()
            };
            let result = equipment_graphics::test_support::with_world(&frame, &dlcs, |limits| {
                let terrain = BTreeMap::new();
                let localisation = Localisation::default();
                let hour = Hour::read(&assets, &frame, &terrain, limits, &localisation).unwrap();
                let motions = TaskForceMotions::read(&frame).unwrap();
                let missions = MissionSkips::read(&frame).unwrap();
                let mut report = ModelReport::default();
                let sources = if reference {
                    assets
                        .prepare_naval_with_names(
                            &frame,
                            &Vision::default(),
                            &|_, _| Some([2.0, 0.0, 3.0]),
                            None,
                            &ShipPlaces {
                                motions: &motions,
                                movement: None,
                                land: &|_| false,
                                missions: &missions,
                                region: &|_| Some(1),
                                scale: &|_, _| 1.0,
                            },
                            Some(&hour),
                            &mut report,
                            &mut BuilderSpans::new(false),
                            &mut Reference::new(&assets, Some(&hour)),
                        )
                        .unwrap()
                } else {
                    assets
                        .prepare_naval_with_spans(
                            &frame,
                            &Vision::default(),
                            &|_, _| Some([2.0, 0.0, 3.0]),
                            None,
                            &ShipPlaces {
                                motions: &motions,
                                movement: None,
                                land: &|_| false,
                                missions: &missions,
                                region: &|_| Some(1),
                                scale: &|_, _| 1.0,
                            },
                            Some(&hour),
                            &mut report,
                            &mut BuilderSpans::new(false),
                        )
                        .unwrap()
                };
                assert!(
                    assets.bakes.lock().unwrap().is_empty(),
                    "warm fixture must not introduce bake demand"
                );
                let source_bytes = records(&sources);
                let history = &mut histories[arm];
                history.set(Vec::new(), Arc::clone(&assets.bounds), false);
                history.set_naval(sources);
                let mut frame_bytes = Vec::new();
                for subframe in 0..4 {
                    let shown = [true, !(step % 7 == 3 && subframe >= 2)];
                    let units = history.step(
                        step as f32 * 0.25 + subframe as f32 * 0.025,
                        assets.gates,
                        shown,
                        true,
                        |_| true,
                    );
                    frame_bytes.extend_from_slice(&(units.len() as u64).to_le_bytes());
                    admitted += units.len();
                    for (slot, instance) in units {
                        frame_bytes.extend_from_slice(&(slot as u64).to_le_bytes());
                        frame_bytes.extend_from_slice(bytemuck::bytes_of(&instance));
                    }
                }
                (report, source_bytes, frame_bytes)
            });
            let trace = names(&frame, &assets, &dlcs, reference);
            arms.push((trace, result));
        }
        assert_eq!(
            arms[0], arms[1],
            "step {step}: chosen names, tag demand, exact refusals, all naval sources and retained unit instance bytes"
        );
        chosen.extend(
            arms[0]
                .0
                .0
                .iter()
                .filter_map(|name| name.as_ref().ok().and_then(Option::as_ref))
                .cloned(),
        );
        if step == 16 {
            assert!(
                arms[0].0.0.iter().any(|name| name
                    .as_ref()
                    .is_err_and(|error| error.contains("AvailableEquipmentVariants"))),
                "a winning graphics candidate must still reach the late conventional refusal"
            );
        }
        saw_success |= arms[0].0.0.iter().any(|name| matches!(name, Ok(Some(_))));
        saw_refusal |= arms[0].0.0.iter().any(Result::is_err);
        saw_tag |= !arms[0].0.1.is_empty();
        assert!(!arms[0].1.1.is_empty());
    }
    assert!(saw_success && saw_refusal && saw_tag);
    for expected in ["pool_a", "pool_b", "saved", PLACEHOLDER_ENTITY] {
        assert!(
            chosen.contains(expected),
            "fixture never selected {expected}: {chosen:?}"
        );
    }
    assert!(
        admitted > 100,
        "must exercise actual retained hull records, not only names"
    );
}

#[test]
fn repeated_generated_winner_keeps_each_bake_refusal_and_never_tries_a_later_hull() {
    viewer_scene_base::test_game::enter();
    let (frame, assets) = fixture(0);
    assets.library.write().unwrap().index.remove("pool_a");
    assets
        .bakes
        .lock()
        .unwrap()
        .insert("pool_a".into(), Arc::new(std::sync::Mutex::new(true)));
    equipment_graphics::test_support::with_world(&frame, &["synthetic_models"], |limits| {
        let terrain = BTreeMap::new();
        let localisation = Localisation::default();
        let hour = Hour::read(&assets, &frame, &terrain, limits, &localisation).unwrap();
        let motions = TaskForceMotions::read(&frame).unwrap();
        let missions = MissionSkips::read(&frame).unwrap();
        let mut report = ModelReport::default();
        let sources = assets
            .prepare_naval_with_spans(
                &frame,
                &Vision::default(),
                &|_, _| Some([2.0, 0.0, 3.0]),
                None,
                &ShipPlaces {
                    motions: &motions,
                    movement: None,
                    land: &|_| false,
                    missions: &missions,
                    region: &|_| Some(1),
                    scale: &|_, _| 1.0,
                },
                Some(&hour),
                &mut report,
                &mut BuilderSpans::new(false),
            )
            .unwrap();
        assert_eq!(
            report
                .selection_errors
                .get("naval_entity: selected registered entity pool_a cannot be baked"),
            Some(&10)
        );
        assert_eq!(report.by_entity.get(PLACEHOLDER_ENTITY), Some(&1));
        assert_eq!(report.by_entity.get("shape_entity"), Some(&1));
        assert!(!report.by_entity.contains_key("pool_a"));
        assert!(assets.library().slot("pool_a").is_none());
        assert!(*assets.bakes.lock().unwrap()["pool_a"].lock().unwrap());
        assert_eq!(sources.formations.len(), 1);
        assert_eq!(sources.formations[0].roster.len(), 12);
    });
}
