use crate::{ModelLibrary, ModelMesh, ModelPass, PaletteJoint};
use crate::animation_dispatch::*;
use std::collections::BTreeSet;

fn append_rig(library: &mut ModelLibrary, slot: usize) {
    let mut rig =
        crate::entity_animation::pose_rig(0.6 + slot as f32 * 0.1, true, true);
    if slot == 2 {
        for node in &mut rig.nodes {
            node.clips.push(node.clips[0].clone());
        }
    }
    if slot == 3 {
        rig.nodes[0].default = "unregistered_default".into();
    }
    if slot == 4 {
        rig.nodes[1].native = None;
    }
    if slot == 5 {
        for node in &mut rig.nodes {
            node.clips.clear();
        }
    }
    let start = library.palette.len();
    rig.start += start;
    rig.end += start;
    for node in &mut rig.nodes {
        node.rigid += start as u32;
        node.skin_base = node.skin_base.map(|base| base + start as u32);
    }
    library.palette.resize(rig.end, PaletteJoint::IDENTITY);
    library.rigs.push(Arc::new(rig));
    library.meshes.push(ModelMesh {
        asset: crate::AssetKey::synthetic(&format!("synthetic_{slot}")),
        name: format!("synthetic_{slot}"),
        path: String::new(),
        scale: 1.,
        lod_distances: Vec::new(),
        native: Vec::new(),
        radius: 1.,
        height: 1.,
        visual: slot == 0,
        own_radius: 1.,
        bound: 1.,
        convoy_radius: None,
        attachments: 1,
        cull_distance_squared: content_reader_mesh::mesh::DEFAULT_CULL_DISTANCE_SQUARED,
    });
}

fn assert_frame(
    pass: &ModelPass,
    before: &ModelField,
    appeared: &BTreeMap<Key, f32>,
    time: f32,
    shown: &dyn Fn(&ModelGroup) -> bool,
    frame: u32,
) {
    let early = time <= 0.0 && before.clocks.values().all(|clock| clock.phase.is_none());
    let mut instances = before.instances.clone();
    for instance in &mut instances {
        instance.palette_offset = 0;
    }
    if early {
        assert!(pass.players.is_empty() && pass.events.is_empty());
        assert_eq!(pass.field.default_states, before.default_states);
        assert_eq!(
            pass.default_appeared
                .iter()
                .map(|(key, start)| (*key, start.to_bits()))
                .collect::<Vec<_>>(),
            appeared
                .iter()
                .map(|(key, start)| (*key, start.to_bits()))
                .collect::<Vec<_>>()
        );
    } else {
        let mut defaults = BTreeMap::new();
        let mut starts = BTreeMap::new();
        let mut clocks = before.clocks.clone();
        let mut refused = 0;
        for group in &before.groups {
            if pass.rigs.get(group.mesh).is_none() {
                continue;
            }
            for instance in &before.instances[group.range.start as usize..group.range.end as usize]
            {
                if instance.animation != entity_animation::DEFAULT_STATE {
                    continue;
                }
                if group.mesh == 2 {
                    refused += 1;
                    continue;
                }
                if matches!(group.mesh, 3 | 5) {
                    continue;
                }
                let key = key(group.mesh, *instance);
                let start = appeared
                    .get(&key)
                    .copied()
                    .map_or(time, |start| start.min(time));
                defaults.insert(key, entity_animation::named_request("idle"));
                starts.insert(key, start);
                clocks.insert(
                    key,
                    entity_animation::Clock {
                        time: time - start,
                        phase: None,
                        requests: Arc::new(vec![entity_animation::Request {
                            time: 0.0,
                            phase_time: None,
                            state: "idle".into(),
                        }]),
                    },
                );
            }
        }
        assert_eq!(
            pass.field.default_states, defaults,
            "frame {frame}: defaults"
        );
        assert_eq!(
            pass.default_appeared
                .iter()
                .map(|(key, start)| (*key, start.to_bits()))
                .collect::<Vec<_>>(),
            starts
                .iter()
                .map(|(key, start)| (*key, start.to_bits()))
                .collect::<Vec<_>>()
        );
        let reason = "entity synthetic_2 reaches state idle of 2 variants, which the game draws from a per-entity random counter no recording holds; it stays still and silent";
        let expected_refusals = if refused == 0 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(reason.to_owned(), refused)])
        };
        assert_eq!(
            pass.default_state_refusals, expected_refusals,
            "frame {frame}: exact default refusal"
        );
        assert_eq!(
            pass.field.clocks.keys().collect::<Vec<_>>(),
            clocks.keys().collect::<Vec<_>>()
        );
        for (key, clock) in &clocks {
            let actual = &pass.field.clocks[key];
            assert_eq!(actual.time.to_bits(), clock.time.to_bits());
            let phase = |value: Option<entity_animation::Phase>| {
                value.map(|value| {
                    (
                        value.time.to_bits(),
                        value.root_advances_hidden,
                        bytemuck::bytes_of(&value.instance).to_vec(),
                    )
                })
            };
            assert_eq!(phase(actual.phase), phase(clock.phase));
            let requests = |value: &entity_animation::Clock| {
                value
                    .requests
                    .iter()
                    .map(|request| {
                        (
                            request.time.to_bits(),
                            request.phase_time.map(f32::to_bits),
                            request.state.clone(),
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(requests(actual), requests(clock));
        }
        let phase_only = time <= 0.0 && clocks.values().any(|clock| clock.phase.is_some());
        let mut alive = BTreeSet::new();
        let mut sources = Vec::new();
        let mut source_calls = Vec::new();
        let mut cursor = pass.default_palette.len();
        let mut supported = true;
        for (group_at, index) in before.source_order() {
            let group = &before.groups[group_at];
            let Some(rig) = pass.rigs.get(group.mesh) else {
                continue;
            };
            if rig.nodes.iter().all(|node| node.clips.is_empty()) {
                continue;
            }
            supported &= rig.nodes.iter().all(|node| node.native.is_some());
            let mut instance = before.instances[index];
            let key = key(group.mesh, instance);
            let class = match instance.animation {
                entity_animation::STOPPED => 0,
                entity_animation::DEFAULT_STATE => 1,
                _ => 2,
            };
            sources.push((index as usize, key, class));
            let active = shown(group)
                && class != 0
                && (!phase_only || clocks.get(&key).is_some_and(|clock| clock.phase.is_some()))
                && (class != 1 || defaults.contains_key(&key));
            source_calls.push(if active {
                instance.palette_offset = (cursor - rig.start) as u32;
                instances[index as usize].palette_offset = instance.palette_offset;
                cursor += rig.end - rig.start;
                instance.animation = defaults.get(&key).copied().unwrap_or(instance.animation);
                alive.insert(key);
                Some(instance)
            } else {
                None
            });
        }
        let mut players = alive.clone();
        players.extend(clocks.iter().filter_map(|(key, clock)| {
            (clock.phase.is_some() && pass.rigs.get(key.0).is_some()).then_some(*key)
        }));
        assert_eq!(
            pass.players.keys().copied().collect::<BTreeSet<_>>(),
            players,
            "frame {frame}: live/hidden/retired keys"
        );
        let layout = pass.animation_dispatch.layout.as_ref().unwrap();
        assert!(layout.matches(&pass.rigs, &pass.field));
        assert_eq!(
            layout.supported, supported,
            "frame {frame}: whole-pass palette eligibility"
        );
        assert_eq!(
            layout
                .sources
                .iter()
                .map(|source| (source.index, source.key, source.kind))
                .collect::<Vec<_>>(),
            sources
        );
        if supported {
            let expected_tasks: BTreeSet<_> = sources
                .iter()
                .filter_map(|(_, key, class)| (*class != 0).then_some(*key))
                .collect();
            assert_eq!(
                layout
                    .tasks
                    .iter()
                    .map(|task| task.key)
                    .collect::<BTreeSet<_>>(),
                expected_tasks
            );
            let mut ordinals = BTreeMap::<Key, usize>::new();
            for ((source, (_, key, class)), expected) in
                layout.sources.iter().zip(&sources).zip(source_calls)
            {
                if *class == 0 {
                    assert!(source.call.is_none());
                    continue;
                }
                let task_at = layout
                    .tasks
                    .binary_search_by_key(key, |task| task.key)
                    .unwrap();
                let ordinal = ordinals.entry(*key).or_default();
                assert_eq!(source.call, Some((task_at, *ordinal)));
                let task = &layout.tasks[task_at];
                let call = &task.calls[*ordinal];
                *ordinal += 1;
                assert!(Arc::ptr_eq(&task.rig, &pass.rigs[key.0]));
                assert_eq!(call.active, expected.is_some());
                assert!(call.events.is_empty(), "source packets commit and drain");
                if let Some(expected) = expected {
                    assert_eq!(
                        bytemuck::bytes_of(&call.instance),
                        bytemuck::bytes_of(&expected),
                        "frame {frame}: call instance bytes"
                    );
                    assert_eq!(call.clocked, clocks.contains_key(key));
                }
                assert_eq!(task.alive, alive.contains(key));
                assert_eq!(
                    task.ordinary,
                    alive.contains(key) && !clocks.contains_key(key)
                );
            }
            for task in &layout.tasks {
                assert_eq!(task.calls.len(), ordinals[&task.key]);
            }
        } else {
            assert!(
                layout.tasks.is_empty(),
                "mixed palettes reject retained dispatch before binding calls"
            );
        }
        for event in &pass.events {
            assert_eq!(event.key.pass, pass.event_namespace);
            assert!(
                alive
                    .iter()
                    .any(|&key| pass.actor_address(key) == event.key.actor()),
                "frame {frame}: event {:?}, actor {:?}, live addresses {:?}, groups {:?}",
                event.key,
                event.key.actor(),
                alive
                    .iter()
                    .map(|&key| pass.actor_address(key))
                    .collect::<Vec<_>>(),
                pass.field
                    .groups
                    .iter()
                    .map(|group| (group.mesh, &group.asset))
                    .collect::<Vec<_>>()
            );
            assert_eq!(event.definition.particle.as_deref(), Some("smoke"));
            assert!(event.age.is_finite());
            assert!(
                event
                    .transform
                    .basis
                    .into_iter()
                    .flatten()
                    .chain(event.transform.translation)
                    .all(f32::is_finite)
            );
        }
    }
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&pass.field.instances),
        bytemuck::cast_slice::<_, u8>(&instances),
        "frame {frame}: complete instance bytes"
    );
    assert_eq!(pass.field.groups.len(), before.groups.len());
    for (actual, expected) in pass.field.groups.iter().zip(&before.groups) {
        assert_eq!(
            (actual.mesh, &actual.range),
            (expected.mesh, &expected.range)
        );
        assert_eq!(
            actual.bounds.map(f32::to_bits),
            expected.bounds.map(f32::to_bits)
        );
        assert_eq!(
            actual.centroid.map(f32::to_bits),
            expected.centroid.map(f32::to_bits)
        );
    }
}

fn actor_bits(player: &entity_animation::Player, rig: &Rig) -> String {
    let matrix = |matrix: crate::mesh_pose::Matrix| {
        matrix.map(|axis| {
            axis.map(|value| {
                assert!(
                    value.is_finite(),
                    "fixture scratch is finite, not a NaN-payload oracle"
                );
                value.to_bits()
            })
        })
    };
    let actors = player.native_actors(rig);
    let bits: Vec<_> = actors
        .iter()
        .map(|actor| {
            (
                actor.world.as_ref().map(|world| matrix(*world)),
                actor.pose.as_ref().map(|pose| {
                    (
                        pose.joints.iter().copied().map(matrix).collect::<Vec<_>>(),
                        pose.bones.iter().copied().map(matrix).collect::<Vec<_>>(),
                    )
                }),
            )
        })
        .collect();
    format!("{bits:?}")
}

#[test]
fn late_mesh_admission_binds_events_without_changing_existing_actor_identities() {
    content_reader_script::test_game::enter();
    for label in ["unit models", "map buildings", "general animation"] {
        for install in [false, true] {
            let mut library = ModelLibrary::default();
            append_rig(&mut library, 0);
            let mut pass = ModelPass::new(&library, label);
            let known = ModelInstance {
                animation: entity_animation::DEFAULT_STATE,
                scale: 1.,
                animation_seed: 31,
                animation_identity: 17,
                animation_epoch: 9,
                ..Default::default()
            };
            let late = ModelInstance {
                animation: entity_animation::named_request("idle"),
                scale: 1.,
                animation_seed: 43,
                animation_identity: 19,
                animation_epoch: 11,
                ..Default::default()
            };
            pass.update_placed(vec![(0, known), (1, late)], 256.);
            pass.animate(0.25, None, &[0.]);
            pass.animate(0.5, None, &[0.]);
            assert_eq!(pass.default_appeared.len(), 1);
            let known_key = key(0, known);
            let late_key = key(1, late);
            let address = pass.actor_address(known_key);
            assert!(pass.events.iter().any(|event| event.key.actor() == address));
            assert!(!pass.players.contains_key(&late_key));
            let player = &raw const pass.players[&known_key];
            let played = pass.root_play_position(known_key);
            let appeared = pass.default_appeared.clone();
            let existing = pass
                .field
                .groups
                .iter()
                .find(|group| group.mesh == 0)
                .unwrap()
                .clone();
            let instances = pass.field.instances.clone();
            let order = pass.field.native_order.clone();
            let producers = pass.field.producer_order.clone();
            let mut stale = pass.field.clone();
            assert!(!stale.bindings.contains_key(&1));
            append_rig(&mut library, 1);
            if install {
                pass.sync(&library);
                pass.replace_field(&library, &mut stale);
            } else {
                pass.sync(&library);
            }
            assert_eq!(pass.actor_address(known_key), address);
            assert_eq!(&raw const pass.players[&known_key], player);
            assert_eq!(pass.root_play_position(known_key), played);
            assert_eq!(pass.default_appeared, appeared);
            assert_eq!(pass.field.instances, instances);
            assert_eq!(pass.field.native_order, order);
            assert_eq!(pass.field.producer_order, producers);
            assert_eq!(
                pass.field.groups.iter().find(|group| group.mesh == 0),
                Some(&existing)
            );
            assert_eq!(pass.field.bindings.get(&1), Some(&library.meshes[1].asset));
            assert_eq!(
                pass.field
                    .groups
                    .iter()
                    .find(|group| group.mesh == 1)
                    .unwrap()
                    .asset,
                library.meshes[1].asset
            );
            pass.animate(0.75, None, &[0.]);
            let live = pass.actor_address(late_key);
            let emitted: Vec<_> = pass
                .events
                .iter()
                .filter(|event| {
                    let actor = event.key.actor();
                    (actor.2, actor.3) == (live.2, live.3)
                })
                .collect();
            assert!(
                !emitted.is_empty(),
                "{label}, install={install}: {:?}",
                pass.events
                    .iter()
                    .map(|event| &event.key)
                    .collect::<Vec<_>>()
            );
            for event in emitted {
                assert_eq!(event.key.model, library.meshes[1].asset);
                assert_eq!(event.key.actor(), live);
            }
            assert_eq!(pass.actor_address(known_key), address);
        }
    }
}

#[test]
fn retained_dispatch_preserves_source_calls_and_outputs_across_900_frames() {
    content_reader_script::test_game::enter();
    let mut library = ModelLibrary::default();
    for slot in 0..6 {
        append_rig(&mut library, slot);
    }
    library.alternatives.insert(0, 1);
    let mut pass = ModelPass::new(&library, "unit models");
    let mut random = 0xf0a7_13c5u32;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        random
    };
    let mut counts = [0usize; 8];
    let mut events = 0;
    let mut refusals = 0;
    let mut warm = 0;
    for frame in 0..900u32 {
        let roll = next();
        pass.label = match frame {
            ..300 => "unit models",
            300..600 => "map buildings",
            _ => "city sprawl",
        };
        if frame == 180 {
            append_rig(&mut library, 6);
            pass.sync(&library);
            counts[0] += 1;
        }
        if frame == 347 {
            let mut rig = (*library.rigs[1]).clone();
            rig.nodes[0].clips[0].state.speed = 1.1;
            library.rigs[1] = Arc::new(rig);
            pass.rigs[1] = Arc::clone(&library.rigs[1]);
            counts[1] += 1;
        }
        if frame == 380 || frame == 425 {
            pass.rigs = if frame == 380 {
                library.rigs[..2].to_vec()
            } else {
                library.rigs.clone()
            };
            counts[1] += 1;
        }
        if frame.is_multiple_of(137) {
            pass.reset_animation();
            counts[2] += 1;
        }
        if frame.is_multiple_of(37) {
            let mut placed = Vec::new();
            let mixed = frame / 37 % 4 == 1;
            for mesh in [2, 6, 0, 3, 1, 5, 4] {
                if mesh == 4 && !mixed {
                    continue;
                }
                for ordinal in 0..12u32 {
                    placed.push((
                        mesh,
                        ModelInstance {
                            position: [ordinal as f32 * 0.6 - 2., 0., (mesh % 3) as f32 * 3.],
                            rotation: ordinal as f32 * 0.11,
                            scale: 0.7,
                            animation: entity_animation::IDLE,
                            animation_seed: ordinal % 7,
                            animation_epoch: frame / 111,
                            animation_identity: if ordinal % 4 == 0 { 17 } else { 0 },
                            ..Default::default()
                        },
                    ));
                }
            }
            pass.update_placed(placed, 4.);
            counts[3] += 1;
        }
        let mut field = pass.field.clone();
        for (at, instance) in field.instances.iter_mut().enumerate() {
            instance.position[1] = ((roll.wrapping_add(at as u32) % 17) as f32 - 8.) * 0.03;
            instance.rotation += 0.017;
            instance.scale = if frame % 2 == 0 { 0.7 } else { 1.1 };
            instance.pitch = (at % 3) as f32 * -0.01;
            instance.native_flags = if at % 5 == 0 { 8 } else { 0 };
            instance.animation_phase = (at % 11) as f32 / 11.;
            instance.animation = match (frame / 9 + at as u32) % 9 {
                0 => entity_animation::STOPPED,
                1..=2 => entity_animation::DEFAULT_STATE,
                3..=4 => entity_animation::IDLE,
                5..=6 => entity_animation::MOVE,
                _ => entity_animation::ATTACK,
            };
            if frame % 53 == 17 && at % 13 == 0 {
                instance.animation_identity = roll % 5 + 1;
                counts[4] += 1;
            }
        }
        for (at, group) in field.groups.iter_mut().enumerate() {
            group.bounds = if (frame / 7 + at as u32).is_multiple_of(4) {
                [20., 20., 21., 21.]
            } else {
                [-3., -3., 3., 3.]
            };
            group.centroid[0] = (roll % 17) as f32;
        }
        if frame % 37 == 13 && field.groups.len() > 2 {
            let duplicate = field.groups[0].clone();
            field.groups.insert(2, duplicate);
            let shift = roll as usize % field.groups.len();
            field.groups.rotate_left(shift);
            field.native_order = field
                .groups
                .iter()
                .enumerate()
                .flat_map(|(at, group)| group.range.clone().map(move |index| (at, index as usize)))
                .collect();
            counts[4] += 1;
        }
        if frame.is_multiple_of(43) {
            pass.replace_field(&library, &mut field);
            counts[3] += 1;
        } else {
            pass.field = field;
        }
        if frame == 173 || frame == 521 {
            pass.falling_back
                .get_mut()
                .insert(0, "synthetic redirect".into());
            counts[5] += 1;
        }
        if frame == 311 {
            pass.redirects.clear();
        }
        let time = match frame % 97 {
            0 => 0.,
            1 => -0.0,
            2 => -0.25,
            _ => (frame % 211) as f32 * 0.071,
        };
        let active = (frame % 200) as f32 * 0.031;
        let phase_time = (frame % 227) as f32 * 0.097;
        let mut clocks = BTreeMap::new();
        for mesh in [2, 0, 1, 6, 99] {
            for seed in [0, 3, 6, 77] {
                let phase = (!frame.is_multiple_of(4) && (seed == 77 || seed == 3)).then_some(
                    entity_animation::Phase {
                        time: phase_time,
                        root_advances_hidden: frame / 17 % 2 == 0,
                        instance: ModelInstance {
                            scale: 0.7,
                            animation_seed: seed,
                            animation_epoch: frame / 111,
                            animation: entity_animation::IDLE,
                            ..Default::default()
                        },
                    },
                );
                clocks.insert(
                    (mesh, seed, frame / 111),
                    entity_animation::Clock {
                        time: active,
                        phase,
                        requests: Arc::new(vec![
                            entity_animation::Request {
                                time: 0.,
                                phase_time: phase.map(|_| 0.),
                                state: "idle".into(),
                            },
                            entity_animation::Request {
                                time: 0.19,
                                phase_time: phase.map(|_| 0.43),
                                state: "move".into(),
                            },
                            entity_animation::Request {
                                time: 0.9,
                                phase_time: phase.map(|_| 1.3),
                                state: if frame / 19 % 2 == 0 {
                                    "attack"
                                } else {
                                    "absent_synthetic_state"
                                }
                                .into(),
                            },
                        ]),
                    },
                );
            }
        }
        pass.set_clocks(clocks);
        pass.set_hidden(BTreeSet::from([(0, 77, frame / 111), (1, 3, frame / 111)]));
        let rectangle = if frame / 11 % 3 == 0 {
            None
        } else {
            Some([-5., -5., 5., 5.])
        };
        let offsets: &[f32] = if frame / 13 % 3 == 0 {
            &[]
        } else {
            &[0., -32., 32.]
        };
        pass.fall_back();
        let input = pass.field.clone();
        let appeared = pass.default_appeared.clone();
        let was_warm = pass
            .animation_dispatch
            .layout
            .as_ref()
            .is_some_and(|layout| layout.matches(&pass.rigs, &pass.field));
        pass.animate(time, rectangle, offsets);
        let shown = |group: &ModelGroup| {
            offsets.iter().any(|offset| {
                rectangle.is_none_or(|[x0, z0, x1, z1]| {
                    let [gx0, gz0, gx1, gz1] = group.bounds;
                    gx0 + offset <= x1 && gx1 + offset >= x0 && gz0 <= z1 && gz1 >= z0
                })
            })
        };
        assert_frame(&pass, &input, &appeared, time, &shown, frame);
        if let Some(layout) = &pass.animation_dispatch.layout {
            counts[if layout.supported { 6 } else { 7 }] += 1;
            if was_warm && layout.supported {
                warm += 1;
            }
        }
        if frame >= 173 {
            assert_eq!(
                pass.from_recording,
                BTreeMap::from([("synthetic_0".into(), "synthetic redirect".into())])
            );
        }
        events += pass.events.len();
        refusals += pass.default_state_refusals.values().sum::<usize>();
        for (key, player) in &pass.players {
            let rig = &pass.rigs[key.0];
            let position = || {
                player
                    .root_play_position(rig)
                    .map(|(time, length)| (time.to_bits(), length.to_bits()))
            };
            let before = position();
            let first = actor_bits(player, rig);
            assert_eq!(
                first,
                actor_bits(player, rig),
                "frame {frame} key={key:?}: read-only pose cache"
            );
            assert_eq!(before, position(), "sampling cannot advance the player");
        }
    }
    assert_eq!(&counts[..4], &[1, 3, 7, 46]);
    assert!(
        counts.into_iter().all(|count| count > 0),
        "coverage {counts:?}"
    );
    assert!(
        events > 0 && refusals > 0 && warm > 50,
        "events={events} refusals={refusals} warm={warm}"
    );
}
