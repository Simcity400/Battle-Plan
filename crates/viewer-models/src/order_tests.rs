use crate::*;
use crate::order_fixture::*;

fn packets(events: &[entity_animation::Event]) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            format!(
                "{:?}|{:?}|{:?}|{}|{}|{:?}|{}",
                event.key.without_pass(),
                event.definition,
                PaletteJoint::from_transform(&event.transform),
                event.seed,
                event.age.to_bits(),
                event.emission_end.map(f32::to_bits),
                event.state
            )
        })
        .collect()
}

#[test]
fn opposite_bake_and_merge_orders_keep_groups_native_calls_and_both_animation_paths() {
    let expected_names = ["zeta", "alpha", "middle", "alpha", "zeta", "middle"];
    let mut reference = None;
    for retained in [false, true] {
        for order in ORDERS {
            let library = library(order, true);
            let mut pass =
                ModelPass::new(&library, if retained { "unit models" } else { "general" });
            pass.update_placed(placed(&library), 256.);
            assert_eq!(
                pass.field
                    .groups
                    .iter()
                    .map(|group| library.meshes[group.mesh].name.as_str())
                    .collect::<Vec<_>>(),
                ["alpha", "middle", "zeta"]
            );
            for (at, &(group, instance)) in pass.field.native_order.iter().enumerate() {
                assert_eq!(
                    library.meshes[pass.field.groups[group].mesh].name,
                    expected_names[at]
                );
                assert_eq!(
                    pass.field.instances[instance].animation_seed,
                    70 + at as u32
                );
            }
            let groups: Vec<_> = pass
                .field
                .groups
                .iter()
                .map(|group| {
                    (
                        group.asset.clone(),
                        group.range.clone(),
                        group.bounds.map(f32::to_bits),
                        group.centroid.map(f32::to_bits),
                        pass.field.instances[group.range.start as usize..group.range.end as usize]
                            .to_vec(),
                    )
                })
                .collect();
            let mut frames = Vec::new();
            for time in [1., 1.13, 1.4, 2.1, 0.2, 0.7] {
                pass.animate(time, None, &[0.]);
                assert_eq!(pass.events.len(), 6);
                for (at, event) in pass.events.iter().enumerate() {
                    assert_eq!(
                        event.key.model,
                        library.meshes[library.slot(expected_names[at]).unwrap()].asset
                    );
                    assert_eq!(
                        event.seed,
                        70 + at as u32,
                        "event_seed never reads an asset or binding"
                    );
                }
                frames.push(packets(&pass.events));
            }
            let observed = (groups, frames);
            if let Some(expected) = &reference {
                assert_eq!(&observed, expected);
            } else {
                reference = Some(observed);
            }
        }
    }
}

#[test]
fn repeated_players_keep_serial_source_calls_and_distinct_parts_and_reset_namespaces() {
    for order in ORDERS {
        let library = library(order, true);
        let mut pass = ModelPass::new(&library, "unit models");
        let mesh = library.slot("zeta").unwrap();
        let mut placements = placed(&library);
        let mut repeated = placements[0];
        repeated.1.position[0] = 400.;
        placements.insert(2, repeated);
        pass.update_placed(placements, 256.);
        pass.animate(1., None, &[0.]);
        assert_eq!(
            pass.players.len(),
            6,
            "repeated calls share one player, not an occurrence key"
        );
        assert_eq!(pass.events.len(), 7);
        assert_eq!(pass.events[0].key, pass.events[2].key);
        assert_ne!(pass.events[0].key, pass.events[5].key);
        let actor = pass.actor_address((mesh, 70, 100));
        assert_eq!(pass.events[0].key.player(), actor);
        pass.set_hidden(BTreeSet::from([(mesh, 70, 100)]));
        assert!(pass.hides(actor));
        let old = pass.events[0].key.clone();
        pass.reset_animation();
        pass.animate(1., None, &[0.]);
        assert_ne!(old.pass, pass.events[0].key.pass);
        assert_ne!(old.player(), pass.events[0].key.player());
        assert!(pass.hides(pass.actor_address((mesh, 70, 100))));
        assert_eq!(old.without_pass(), pass.events[0].key.without_pass());
    }
}

#[test]
fn stable_identity_distinguishes_content_contract_and_counterpart_roles() {
    let original = AssetKey::synthetic("same");
    let mut retry = original.clone();
    retry.recorded(false);
    let mut counterpart = original.clone();
    counterpart.recorded(true);
    let padded = AssetKey::selected(
        "same",
        Arc::from([]),
        Decode {
            force_pow2_textures: true,
        },
    );
    let all = BTreeSet::from([original, retry, counterpart, padded]);
    assert_eq!(all.len(), 4);
}

#[test]
fn redirects_rebind_stable_hidden_and_clock_addresses_and_canonical_storage() {
    for order in ORDERS {
        let mut library = library(order, true);
        let from = library.slot("zeta").unwrap();
        let to = library.slot("alpha").unwrap();
        library.meshes[from].visual = true;
        library.alternatives.insert(from, to);
        let mut pass = ModelPass::new(&library, "unit models");
        pass.update_placed(placed(&library), 256.);
        pass.set_hidden(BTreeSet::from([(from, 70, 100)]));
        pass.set_clocks(BTreeMap::from([(
            (from, 70, 100),
            entity_animation::Clock {
                time: 0.5,
                phase: None,
                requests: Arc::new(vec![entity_animation::Request {
                    time: 0.,
                    phase_time: None,
                    state: "idle".into(),
                }]),
            },
        )]));
        pass.falling_back
            .get_mut()
            .insert(from, "synthetic material refusal".into());
        pass.animate(1., None, &[0.]);
        assert!(pass.field.clocks.contains_key(&(to, 70, 100)));
        assert!(pass.hides(pass.actor_address((to, 70, 100))));
        assert!(
            pass.field
                .groups
                .windows(2)
                .all(|pair| pair[0].asset <= pair[1].asset)
        );
        assert_eq!(pass.field.groups.len(), 2);
        assert_eq!(pass.events[0].key.model, library.meshes[to].asset);
        assert!(pass.player(from, &placed(&library)[0].1).is_some());
        assert_eq!(pass.from_recording["zeta"], "synthetic material refusal");
    }
}

#[test]
fn clocked_and_default_callbacks_keep_the_source_spine_in_both_paths() {
    let mut reference = None;
    for retained in [false, true] {
        for order in ORDERS {
            let library = library(order, true);
            let mut pass =
                ModelPass::new(&library, if retained { "unit models" } else { "general" });
            let mut placements = placed(&library);
            placements[2].1.animation = entity_animation::DEFAULT_STATE;
            let mut repeated = placements[0];
            repeated.1.position[0] = 400.;
            placements.insert(3, repeated);
            let clock_keys: Vec<_> = placements
                .iter()
                .map(|(mesh, instance)| {
                    let (seed, identity) = instance.animation_key();
                    (*mesh, seed, identity)
                })
                .collect();
            pass.update_placed(placements, 256.);
            let mut frames = Vec::new();
            for time in [1., 1.2, 1.8] {
                pass.set_clocks(
                    clock_keys
                        .iter()
                        .copied()
                        .map(|key| {
                            (
                                key,
                                entity_animation::Clock {
                                    time: time - 1.,
                                    phase: None,
                                    requests: Arc::new(vec![
                                        entity_animation::Request {
                                            time: 0.,
                                            phase_time: None,
                                            state: "idle".into(),
                                        },
                                        entity_animation::Request {
                                            time: 0.1,
                                            phase_time: None,
                                            state: "move".into(),
                                        },
                                    ]),
                                },
                            )
                        })
                        .collect(),
                );
                pass.animate(time, None, &[0.]);
                assert_eq!(pass.players.len(), 6);
                assert!(!pass.events.is_empty());
                frames.push(packets(&pass.events));
            }
            if let Some(expected) = &reference {
                assert_eq!(&frames, expected);
            } else {
                reference = Some(frames);
            }
        }
    }
}

#[test]
fn redirected_clock_collisions_keep_producer_priority_across_reinstalls() {
    for order in ORDERS {
        let mut library = library(order, true);
        let from = library.slot("zeta").unwrap();
        let to = library.slot("alpha").unwrap();
        library.alternatives.insert(from, to);
        let mut pass = ModelPass::new(&library, "unit models");
        let a = placed(&library)[0].1;
        pass.update_placed(vec![(from, a), (to, a)], 256.);
        pass.redirects.insert(from, to);
        for _ in 0..3 {
            let clock = |time| entity_animation::Clock {
                time,
                phase: None,
                requests: Arc::default(),
            };
            pass.set_clocks(BTreeMap::from([
                ((from, 70, 100), clock(1.)),
                ((to, 70, 100), clock(2.)),
            ]));
            assert_eq!(pass.field.clocks.len(), 1);
            assert_eq!(pass.field.clocks[&(to, 70, 100)].time, 2.);
            assert_eq!(pass.field.clock_priorities[&(to, 70, 100)], 1);
        }
    }
}

#[test]
fn tile_storage_keeps_fixed_membership_and_arithmetic_without_reordering_source_events() {
    for retained in [false, true] {
        for order in ORDERS {
            let library = library(order, true);
            let mut pass =
                ModelPass::new(&library, if retained { "unit models" } else { "general" });
            let mut placements = placed(&library);
            for ((_, instance), position) in placements.iter_mut().zip([
                [300., 0., 2.],
                [20., 0., 3.],
                [-10., 0., 4.],
                [10., 0., 1.],
                [-30., 0., 6.],
                [-20., 0., 2.],
            ]) {
                instance.position = position;
            }
            pass.update_placed(placements, 256.);
            let groups: Vec<_> = pass
                .field
                .groups
                .iter()
                .map(|group| {
                    (
                        library.meshes[group.mesh].name.as_str(),
                        pass.field.instances[group.range.start as usize..group.range.end as usize]
                            .iter()
                            .map(|instance| instance.animation_seed)
                            .collect::<Vec<_>>(),
                        group.bounds,
                        group.centroid,
                    )
                })
                .collect();
            assert_eq!(
                groups,
                vec![
                    ("alpha", vec![71, 73], [10., 1., 20., 3.], [15., 0., 2.]),
                    ("middle", vec![72, 75], [-20., 2., -10., 4.], [-15., 0., 3.]),
                    ("zeta", vec![74], [-30., 6., -30., 6.], [-30., 0., 6.]),
                    ("zeta", vec![70], [300., 2., 300., 2.], [300., 0., 2.]),
                ]
            );
            pass.animate(1., None, &[0.]);
            assert_eq!(
                pass.events
                    .iter()
                    .map(|event| event.seed)
                    .collect::<Vec<_>>(),
                [70, 71, 72, 73, 74, 75]
            );
            assert_eq!(
                pass.events
                    .iter()
                    .map(|event| event.transform.translation)
                    .collect::<Vec<_>>(),
                [
                    [300., 0., 2.],
                    [20., 0., 3.],
                    [-10., 0., 4.],
                    [10., 0., 1.],
                    [-30., 0., 6.],
                    [-20., 0., 2.]
                ]
            );
        }
    }
}

#[test]
fn actor_addresses_keep_epoch_fallback_and_explicit_part_identity() {
    for order in ORDERS {
        let library = library(order, true);
        let mut pass = ModelPass::new(&library, "unit models");
        let mesh = library.slot("zeta").unwrap();
        let instance = |epoch, identity| ModelInstance {
            animation_seed: 70,
            animation_epoch: epoch,
            animation_identity: identity,
            animation: entity_animation::IDLE,
            scale: 1.,
            ..Default::default()
        };
        pass.update_placed(
            vec![
                (mesh, instance(3, 0)),
                (mesh, instance(4, 0)),
                (mesh, instance(3, 101)),
                (mesh, instance(4, 101)),
                (mesh, instance(3, 102)),
            ],
            256.,
        );
        pass.animate(1., None, &[0.]);
        assert_eq!(pass.players.len(), 4);
        assert_eq!(pass.events.len(), 5);
        assert_ne!(pass.events[0].key.player(), pass.events[1].key.player());
        assert_eq!(pass.events[2].key.player(), pass.events[3].key.player());
        assert_ne!(pass.events[2].key.player(), pass.events[4].key.player());
        assert!(pass.events.iter().all(|event| event.seed == 70));
    }
}

#[test]
fn otherwise_unplaced_counterpart_keeps_hidden_phase_callback_priority_after_reinstalls() {
    for retained in [false, true] {
        for (permutation, order) in ORDERS.into_iter().enumerate() {
            let mut library = library(order, true);
            let visual = library.slot("zeta").unwrap();
            let hidden = library.slot("middle").unwrap();
            let recorded = library.slot("alpha").unwrap();
            library.alternatives.insert(visual, recorded);
            let mut pass =
                ModelPass::new(&library, if retained { "unit models" } else { "general" });
            let v = placed(&library)[0].1;
            let h = placed(&library)[1].1;
            pass.update_placed(vec![(visual, v), (hidden, h)], 256.);
            for reinstall in 0..3 {
                let names = [
                    format!("me_a2_r1_recorded_{retained}_{permutation}_{reinstall}"),
                    format!("me_a2_r1_hidden_{retained}_{permutation}_{reinstall}"),
                ];
                for (slot, name) in [(recorded, &names[0]), (hidden, &names[1])] {
                    let rig = Arc::make_mut(&mut pass.rigs[slot]);
                    rig.nodes[0].default.clone_from(name);
                    rig.nodes[0].clips[0].state.name.clone_from(name);
                }
                pass.reset_animation();
                let clock = |instance| entity_animation::Clock {
                    time: 0.5,
                    phase: Some(entity_animation::Phase {
                        time: 0.5,
                        root_advances_hidden: true,
                        instance,
                    }),
                    requests: Arc::default(),
                };
                pass.set_clocks(BTreeMap::from([
                    ((visual, 70, 100), clock(v)),
                    ((hidden, 71, 101), clock(h)),
                ]));
                if reinstall == 0 {
                    pass.falling_back
                        .get_mut()
                        .insert(visual, "synthetic refusal".into());
                }
                pass.animate(1., Some([10_000., 10_000., 10_001., 10_001.]), &[0.]);
                assert!(pass.events.is_empty());
                assert_eq!(
                    pass.field.clock_order().collect::<Vec<_>>(),
                    [(recorded, 70, 100), (hidden, 71, 101),]
                );
                assert_eq!(pass.field.clock_priorities[&(recorded, 70, 100)], 0);
                assert_eq!(pass.field.clock_priorities[&(hidden, 71, 101)], 1);
                assert_eq!(pass.players.len(), 2);
                assert!(
                    entity_animation::named_request(&names[0])
                        < entity_animation::named_request(&names[1]),
                    "hidden callbacks must intern the inherited-first recorded state before the second actor"
                );
                assert_eq!(pass.field.producer_ordinal((visual, 70, 100)), 0);
                assert_eq!(pass.field.producer_ordinal((recorded, 70, 100)), usize::MAX);
            }
        }
    }
}

#[test]
fn clock_metadata_and_execution_order_are_retained_when_only_times_change() {
    for label in ["general", "unit models"] {
        let library = library(ORDERS[0], true);
        let mut pass = ModelPass::new(&library, label);
        let placements = placed(&library);
        let instance = placements[0].1;
        let key = (placements[0].0, 70, 100);
        pass.update_placed(placements.clone(), 256.);
        let clock = |time| entity_animation::Clock {
            time,
            phase: Some(entity_animation::Phase {
                time,
                root_advances_hidden: true,
                instance,
            }),
            requests: Arc::default(),
        };
        pass.set_clocks(BTreeMap::from([(key, clock(0.1))]));
        pass.animate(1., None, &[0.]);
        let builds = (pass.clock_routes.builds, pass.field.clock_order_builds);
        let buffers = (
            pass.clock_routes.routes.as_ptr(),
            pass.field.ordered_clocks.as_ptr(),
        );
        for time in [0.2, 0.3, 0.4] {
            pass.update_placed(placements.clone(), 256.);
            pass.set_clocks(BTreeMap::from([(key, clock(time))]));
            pass.animate(1. + time, None, &[0.]);
            assert_eq!(
                (pass.clock_routes.builds, pass.field.clock_order_builds),
                builds
            );
            assert_eq!(
                (
                    pass.clock_routes.routes.as_ptr(),
                    pass.field.ordered_clocks.as_ptr()
                ),
                buffers
            );
        }
        pass.set_clocks(BTreeMap::new());
        pass.animate(2., None, &[0.]);
        assert_eq!(pass.clock_routes.builds, builds.0 + 1);
        assert_eq!(pass.field.clock_order_builds, builds.1 + 1);
        assert!(pass.field.clock_order().next().is_none());
    }
}

#[test]
fn a_later_redirect_retains_the_first_effective_clocks_inherited_ordinal() {
    for label in ["general", "unit models"] {
        for order in ORDERS {
            let mut library = library(order, true);
            let visual = library.slot("zeta").unwrap();
            let recorded = library.slot("alpha").unwrap();
            let hidden = library.slot("middle").unwrap();
            let other_recorded = library.meshes.len();
            let mut counterpart = library.meshes[hidden].clone();
            counterpart.name = "other_recorded".into();
            counterpart.asset = AssetKey::synthetic("other_recorded");
            library.meshes.push(counterpart);
            library.rigs.push(Arc::clone(&library.rigs[hidden]));
            library
                .alternatives
                .extend([(visual, recorded), (hidden, other_recorded)]);
            let mut pass = ModelPass::new(&library, label);
            let v = placed(&library)[0].1;
            let h = placed(&library)[1].1;
            pass.update_placed(vec![(visual, v), (hidden, h)], 256.);
            let clock = |instance| entity_animation::Clock {
                time: 0.5,
                phase: Some(entity_animation::Phase {
                    time: 0.5,
                    root_advances_hidden: true,
                    instance,
                }),
                requests: Arc::default(),
            };
            pass.set_clocks(BTreeMap::from([
                ((visual, 70, 100), clock(v)),
                ((hidden, 71, 101), clock(h)),
            ]));
            for mesh in [visual, hidden] {
                pass.falling_back
                    .get_mut()
                    .insert(mesh, "synthetic refusal".into());
                pass.animate(1., Some([10_000., 10_000., 10_001., 10_001.]), &[0.]);
                assert_eq!(pass.field.clock_priorities[&(recorded, 70, 100)], 0);
            }
            assert_eq!(
                pass.field.clock_order().collect::<Vec<_>>(),
                [(recorded, 70, 100), (other_recorded, 71, 101),]
            );
        }
    }
}

#[test]
fn redirected_placement_rebuilds_reuse_clock_routes_order_and_producer_scratch() {
    for label in ["general", "unit models"] {
        let mut library = library(ORDERS[0], true);
        let from = library.slot("zeta").unwrap();
        let to = library.slot("alpha").unwrap();
        library.alternatives.insert(from, to);
        let placements = placed(&library);
        let instance = placements[0].1;
        let key = (from, 70, 100);
        let clock = |time| entity_animation::Clock {
            time,
            phase: Some(entity_animation::Phase {
                time,
                root_advances_hidden: true,
                instance,
            }),
            requests: Arc::default(),
        };
        let mut pass = ModelPass::new(&library, label);
        pass.update_placed(placements.clone(), 256.);
        pass.set_clocks(BTreeMap::from([(key, clock(0.1))]));
        pass.falling_back
            .get_mut()
            .insert(from, "synthetic refusal".into());
        pass.animate(1., None, &[0.]);
        pass.set_clocks(BTreeMap::from([(key, clock(0.1))]));
        pass.animate(1.1, None, &[0.]);
        let builds = (pass.clock_routes.builds, pass.field.clock_order_builds);
        let producers = (
            pass.field.producer_order.as_ptr(),
            pass.field.producer_scratch.as_ptr(),
        );
        for time in [0.2, 0.3, 0.4] {
            pass.update_placed(placements.clone(), 256.);
            pass.set_clocks(BTreeMap::from([(key, clock(time))]));
            pass.animate(1. + time, None, &[0.]);
            assert_eq!(
                (pass.clock_routes.builds, pass.field.clock_order_builds),
                builds
            );
            assert_eq!(
                (
                    pass.field.producer_order.as_ptr(),
                    pass.field.producer_scratch.as_ptr()
                ),
                producers
            );
            assert_eq!(pass.field.clock_priorities[&(to, 70, 100)], 0);
            assert_eq!(
                pass.field.clock_order().collect::<Vec<_>>(),
                [(to, 70, 100)]
            );
        }
    }
}

#[test]
fn whole_field_installs_retain_clock_metadata_across_transform_and_time_changes() {
    for label in ["general", "unit models"] {
        for order in ORDERS {
            for redirected in [false, true] {
                for phased in [false, true] {
                    let mut library = library(order, true);
                    let from = library.slot("zeta").unwrap();
                    let to = library.slot("alpha").unwrap();
                    let other = library.slot("middle").unwrap();
                    library.alternatives.insert(from, to);
                    let mut pass = ModelPass::new(&library, label);
                    if redirected {
                        pass.redirects.insert(from, to);
                    }
                    let placements = vec![
                        (from, placed(&library)[0].1),
                        (other, placed(&library)[1].1),
                    ];
                    let mut baseline = None;
                    let mut buffers = None;
                    for frame in 0..4 {
                        let time = 0.1 + frame as f32 * 0.1;
                        let mut placements = placements.clone();
                        for (at, (_, instance)) in placements.iter_mut().enumerate() {
                            instance.position[0] += frame as f32 * (300. - at as f32 * 600.);
                            instance.animation_phase = time;
                        }
                        let mut field = ModelField::build(&library, placements.clone(), 256.);
                        field.clocks = placements
                            .iter()
                            .map(|&(mesh, instance)| {
                                let (seed, identity) = instance.animation_key();
                                (
                                    (mesh, seed, identity),
                                    entity_animation::Clock {
                                        time,
                                        phase: phased.then_some(entity_animation::Phase {
                                            time,
                                            root_advances_hidden: true,
                                            instance,
                                        }),
                                        requests: Arc::default(),
                                    },
                                )
                            })
                            .collect();
                        let producer_sorts = field.producer_sorts;
                        pass.replace_field(&library, &mut field);
                        pass.animate(1. + time, None, &[0.]);
                        let builds = (pass.clock_routes.builds, pass.field.clock_order_builds);
                        let pointers = (
                            pass.clock_routes.routes.as_ptr(),
                            pass.field.ordered_clocks.as_ptr(),
                        );
                        if let Some(baseline) = baseline {
                            assert_eq!(builds, baseline);
                            assert_eq!(Some(pointers), buffers);
                        } else {
                            baseline = Some(builds);
                            buffers = Some(pointers);
                            assert_eq!(builds.1, usize::from(phased));
                        }
                        assert_eq!(pass.field.producer_sorts, producer_sorts);
                        assert_eq!(pass.field.producer_ordinal((from, 70, 100)), 0);
                        assert_eq!(pass.field.producer_ordinal((other, 71, 101)), 1);
                        let target = if redirected { to } else { from };
                        assert_eq!(pass.field.clocks[&(target, 70, 100)].time, time);
                        assert_eq!(
                            pass.field.instances[pass.field.native_order[0].1].position,
                            placements[0].1.position
                        );
                        if redirected {
                            assert_eq!(pass.field.clock_priorities[&(to, 70, 100)], 0);
                        } else {
                            assert!(pass.clock_routes.routes.is_empty());
                            assert!(pass.field.clock_priorities.is_empty());
                            assert_eq!(pass.clock_actor_addresses().count(), 2);
                        }
                        if !phased {
                            assert!(pass.field.ordered_clocks.is_empty());
                        }
                    }
                    let before = baseline.unwrap();
                    let mut placements = placements.clone();
                    placements.swap(0, 1);
                    let mut field = ModelField::build(&library, placements.clone(), 256.);
                    field.clocks = placements
                        .into_iter()
                        .map(|(mesh, instance)| {
                            let (seed, identity) = instance.animation_key();
                            (
                                (mesh, seed, identity),
                                entity_animation::Clock {
                                    time: 0.5,
                                    phase: phased.then_some(entity_animation::Phase {
                                        time: 0.5,
                                        root_advances_hidden: true,
                                        instance,
                                    }),
                                    requests: Arc::default(),
                                },
                            )
                        })
                        .collect();
                    pass.replace_field(&library, &mut field);
                    pass.animate(1.5, None, &[0.]);
                    assert_eq!(pass.clock_routes.builds, before.0 + usize::from(redirected));
                    assert_eq!(
                        pass.field.clock_order_builds,
                        before.1 + usize::from(phased)
                    );
                    assert_eq!(pass.field.producer_ordinal((from, 70, 100)), 1);
                }
            }
        }
    }
}

#[test]
fn whole_field_installs_validate_bindings_keys_and_redirects_without_losing_collision_priority() {
    for order in ORDERS {
        let mut library = library(order, true);
        let from = library.slot("zeta").unwrap();
        let to = library.slot("alpha").unwrap();
        let other = library.slot("middle").unwrap();
        library.alternatives.insert(from, to);
        let instance = placed(&library)[0].1;
        let key = |mesh| (mesh, 70, 100);
        let clock = |time| entity_animation::Clock {
            time,
            phase: Some(entity_animation::Phase {
                time,
                root_advances_hidden: true,
                instance,
            }),
            requests: Arc::default(),
        };
        let mut pass = ModelPass::new(&library, "unit models");
        pass.redirects.insert(from, to);
        let mut placements = vec![(from, instance), (to, instance)];
        let mut clocks = BTreeMap::from([(key(from), clock(1.)), (key(to), clock(2.))]);
        let install =
            |pass: &mut ModelPass, placements: Vec<_>, clocks: BTreeMap<_, _>, binding| {
                let mut field = ModelField::build(&library, placements, 256.);
                if let Some(binding) = binding {
                    field.bindings.insert(from, binding);
                }
                field.clocks = clocks;
                pass.replace_field(&library, &mut field);
                pass.field.prepare_clock_order();
            };
        install(&mut pass, placements.clone(), clocks.clone(), None);
        let mut builds = pass.clock_routes.builds;
        assert_eq!(pass.field.clocks[&key(to)].time, 2.);
        assert_eq!(pass.field.clock_priorities[&key(to)], 1);
        placements.swap(0, 1);
        install(&mut pass, placements.clone(), clocks.clone(), None);
        assert_eq!(pass.clock_routes.builds, builds + 1);
        builds += 1;
        assert_eq!(pass.field.clocks[&key(to)].time, 1.);
        assert_eq!(pass.field.clock_priorities[&key(to)], 1);
        assert_eq!(pass.field.producer_ordinal(key(to)), 0);
        assert_eq!(pass.field.producer_ordinal(key(from)), 1);
        let binding = AssetKey::synthetic("changed_source_binding");
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds + 1);
        builds += 1;
        assert_eq!(pass.field.clocks[&key(to)].time, 1.);
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds);
        clocks.remove(&key(to));
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds + 1);
        builds += 1;
        pass.redirects.insert(from, other);
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds + 1);
        builds += 1;
        assert_eq!(pass.field.clocks[&key(other)].time, 1.);
        assert_eq!(pass.field.clock_priorities[&key(other)], 1);
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds);
        assert_eq!(pass.field.clock_order().collect::<Vec<_>>(), [key(other)]);
        let order_builds = pass.field.clock_order_builds;
        let effective_asset = AssetKey::synthetic("changed_effective_binding");
        pass.meshes[other].asset = effective_asset.clone();
        install(
            &mut pass,
            placements.clone(),
            clocks.clone(),
            Some(binding.clone()),
        );
        assert_eq!(pass.clock_routes.builds, builds);
        assert_eq!(pass.field.clock_order_builds, order_builds + 1);
        assert_eq!(
            pass.field.ordered_clocks[0].2.as_ref(),
            Some(&effective_asset)
        );
        clocks.get_mut(&key(from)).unwrap().phase = None;
        install(&mut pass, placements, clocks, Some(binding));
        assert_eq!(pass.clock_routes.builds, builds);
        assert_eq!(pass.field.clock_order_builds, order_builds + 2);
        assert!(pass.field.ordered_clocks.is_empty());
    }
}

#[test]
fn producer_metadata_sorts_only_when_source_keys_or_first_ordinals_change() {
    let library = library(ORDERS[0], true);
    let first = placed(&library)[0];
    let second = placed(&library)[1];
    let key = |(mesh, instance): (usize, ModelInstance)| {
        let (seed, identity) = instance.animation_key();
        (mesh, seed, identity)
    };
    let mut field = ModelField::default();
    let mut keyed = Vec::new();
    let mut rebuild = |field: &mut ModelField, mut placements: Vec<_>| {
        let expected: Vec<_> = placements.iter().copied().map(key).collect();
        field.rebuild(&library, &mut placements, &mut keyed, 256.);
        assert_eq!(
            field
                .source_order()
                .map(|(group, instance)| key((field.groups[group].mesh, field.instances[instance])))
                .collect::<Vec<_>>(),
            expected
        );
    };
    rebuild(&mut field, vec![first, second, first, second]);
    let sorts = field.producer_sorts;
    assert_eq!(sorts, 1);
    for transform in [false, true] {
        for frame in 1..4 {
            let mut placements = vec![first, second, first, second];
            for (at, (_, instance)) in placements.iter_mut().enumerate() {
                if transform {
                    instance.position[0] += (at as f32 - 2.) * frame as f32 * 300.;
                    instance.scale += frame as f32 * 0.1;
                } else {
                    instance.animation_phase = frame as f32 * 0.1;
                }
            }
            rebuild(&mut field, placements);
            assert_eq!(field.producer_sorts, sorts);
            assert_eq!(field.producer_ordinal(key(first)), 0);
            assert_eq!(field.producer_ordinal(key(second)), 1);
        }
    }
    for placements in [vec![first, second, second, first], vec![first, second]] {
        rebuild(&mut field, placements);
        assert_eq!(field.producer_sorts, sorts);
    }
    rebuild(&mut field, vec![second, first, first, second]);
    assert_eq!(field.producer_sorts, sorts + 1);
    assert_eq!(field.producer_ordinal(key(first)), 1);
    assert_eq!(field.producer_ordinal(key(second)), 0);
    rebuild(&mut field, vec![second, first]);
    assert_eq!(field.producer_sorts, sorts + 1);
    rebuild(&mut field, vec![first, first]);
    assert_eq!(field.producer_sorts, sorts + 2);
    assert_eq!(field.producer_ordinal(key(second)), usize::MAX);
}
