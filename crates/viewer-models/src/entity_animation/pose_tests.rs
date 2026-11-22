use super::*;
use super::pose_fixture::rig;
use crate::mesh_pose;

fn pose_bits(pose: &[Transform]) -> Vec<([[u32; 3]; 3], [u32; 3])> {
    pose.iter()
        .map(|p| {
            (
                p.basis.map(|c| c.map(f32::to_bits)),
                p.translation.map(f32::to_bits),
            )
        })
        .collect()
}

fn events_equal(a: &[Event], b: &[Event]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(b) {
        assert_eq!(a.definition, b.definition);
        assert_eq!(pose_bits(&[a.transform]), pose_bits(&[b.transform]));
        assert_eq!(a.age.to_bits(), b.age.to_bits());
        assert_eq!(
            (
                a.seed,
                a.key.clone(),
                a.emission_end.map(f32::to_bits),
                a.hidden,
                a.state
            ),
            (
                b.seed,
                b.key.clone(),
                b.emission_end.map(f32::to_bits),
                b.hidden,
                b.state
            )
        );
    }
}

#[test]
fn retained_pose_joins_and_native_only_evaluation_match_full_evaluation_at_all_clocks() {
    for (scale, attached, native) in [(1., false, false), (0.3, true, false), (1.7, true, true)] {
        let rig = rig(scale, attached, native);
        let (mut full, mut direct) = (Player::new(&rig), Player::new(&rig));
        let mut palette = vec![PaletteJoint::IDENTITY; rig.end];
        for (time, state) in [
            (0., IDLE),
            (0.13, IDLE),
            (0.7, MOVE),
            (0.71, MOVE),
            (1.2, MOVE),
            (1.7, ATTACK),
            (2.1, ATTACK),
            (4.9, ATTACK),
            (0.2, IDLE),
            (1.3, MOVE),
        ] {
            let instance = ModelInstance {
                scale: 1.,
                animation: state,
                animation_seed: 7,
                ..Default::default()
            };
            let (mut a, mut b) = (Vec::new(), Vec::new());
            full.evaluate(&rig, instance, time, &mut palette, &mut a);
            direct.evaluate(&rig, instance, time, &mut [], &mut b);
            events_equal(&a, &b);
            assert_eq!(full.root_entry(), direct.root_entry());
            assert_eq!(
                full.root_play_position(&rig),
                direct.root_play_position(&rig)
            );
            for (index, node) in rig.nodes.iter().enumerate() {
                let Some(playing) = full.native_samples[index] else {
                    continue;
                };
                let clip = &node.clips[playing.clip];
                let Some(animation) = &clip.animation else {
                    continue;
                };
                let mut locals = animation.sample(
                    playing.age * clip.state.speed + clip.state.start,
                    clip.state.looping,
                );
                if let Some((old, age, weight)) = playing.previous {
                    let old_clip = &node.clips[old];
                    let old_animation = old_clip.animation.as_ref().unwrap();
                    let old = old_animation.sample(
                        age * old_clip.state.speed + old_clip.state.start,
                        old_clip.state.looping,
                    );
                    let old: BTreeMap<_, _> = old_animation
                        .joints
                        .iter()
                        .zip(old)
                        .map(|(j, p)| (j.name.as_str(), p))
                        .collect();
                    for (joint, pose) in animation.joints.iter().zip(&mut locals) {
                        if let Some(old) = old.get(joint.name.as_str()) {
                            *pose = old.blend(*pose, weight);
                        }
                    }
                }
                assert_eq!(
                    pose_bits(&full.scratch.worlds[index]),
                    pose_bits(&node.mesh.sampled_pose(animation, &locals))
                );
            }
            if native {
                let cached = mesh_pose::actors(&rig, Some(&direct), false);
                let uncached = mesh_pose::actors_uncached(&rig, Some(&full), false);
                for (cached, uncached) in cached.iter().zip(&uncached) {
                    let matrix =
                        |value: mesh_pose::Matrix| value.map(|column| column.map(f32::to_bits));
                    assert_eq!(
                        cached.world.as_ref().map(|world| matrix(*world)),
                        uncached.world.as_ref().map(|world| matrix(*world)),
                    );
                    let pose = |pose: &mesh_pose::Pose| {
                        (
                            pose.joints.iter().copied().map(matrix).collect::<Vec<_>>(),
                            pose.bones.iter().copied().map(matrix).collect::<Vec<_>>(),
                        )
                    };
                    assert_eq!(
                        cached.pose.as_ref().map(pose),
                        uncached.pose.as_ref().map(pose)
                    );
                }
                assert!(Arc::ptr_eq(
                    &cached,
                    &mesh_pose::actors(&rig, Some(&direct), false)
                ));
            }
        }
    }
}

#[test]
fn native_only_callback_sampling_keeps_phase_finishes_and_events() {
    let rig = rig(0.7, true, true);
    let (mut full, mut direct) = (Player::new(&rig), Player::new(&rig));
    let instance = ModelInstance {
        scale: 1.,
        animation_seed: 23,
        ..Default::default()
    };
    let requests = Arc::new(vec![
        Request {
            time: 0.,
            phase_time: Some(0.),
            state: "idle".into(),
        },
        Request {
            time: 0.3,
            phase_time: Some(1.1),
            state: "move".into(),
        },
        Request {
            time: 0.7,
            phase_time: Some(2.1),
            state: "attack".into(),
        },
    ]);
    for (time, phase) in [
        (0., 0.),
        (0.1, 0.7),
        (0.3, 1.1),
        (0.31, 1.2),
        (0.8, 2.3),
        (1.2, 5.4),
    ] {
        let clock = Clock {
            time,
            phase: Some(Phase {
                time: phase,
                root_advances_hidden: true,
                instance,
            }),
            requests: requests.clone(),
        };
        let (mut a, mut b) = (Vec::new(), Vec::new());
        full.evaluate_clock(
            &rig,
            instance,
            &clock,
            &mut vec![PaletteJoint::IDENTITY; rig.end],
            &mut a,
        );
        direct.evaluate_clock(&rig, instance, &clock, &mut [], &mut b);
        events_equal(&a, &b);
        assert_eq!(full.root_entry(), direct.root_entry());
        assert_eq!(full.root_clip_phase(&rig), direct.root_clip_phase(&rig));
        assert_eq!(full.event_refusals(), direct.event_refusals());
    }
}

#[test]
fn event_seed_keeps_fixed_ordinary_and_direct_inputs() {
    let ordinary = ModelInstance {
        animation_seed: 0x1234_5678,
        animation_epoch: 7,
        ..Default::default()
    };
    let playing = Playing {
        requested: "idle".into(),
        clip: 2,
        start: 0.,
        event_time: 1.5,
        phase_offset: 0.,
        previous: None,
        admitted_kept: BTreeMap::new(),
        propagate: false,
        random: Random::new(0),
        cycle: 0,
        completed: false,
    };
    assert_eq!(event_seed(ordinary, 3, 5, Some(&playing)), 0x1234_557d);
    let direct = ModelInstance {
        animation: DIRECT,
        ..ordinary
    };
    assert_eq!(event_seed(direct, 3, 5, Some(&playing)), 0xec36_6d7c);
    let changed_identity = ModelInstance {
        animation_identity: 999,
        ..direct
    };
    assert_eq!(
        event_seed(changed_identity, 3, 5, Some(&playing)),
        0xec36_6d7c
    );
}
