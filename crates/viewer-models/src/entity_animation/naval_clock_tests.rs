use super::*;

fn instance() -> ModelInstance {
    ModelInstance {
        scale: 1.0,
        animation_seed: 7,
        ..Default::default()
    }
}

fn clock(active: f32, phase: f32, requests: &[(f32, f32, &str)]) -> Clock {
    Clock {
        time: active,
        phase: Some(Phase {
            time: phase,
            root_advances_hidden: false,
            instance: instance(),
        }),
        requests: Arc::new(
            requests
                .iter()
                .map(|&(time, phase_time, state)| Request {
                    time,
                    phase_time: Some(phase_time),
                    state: state.to_owned(),
                })
                .collect(),
        ),
    }
}

fn admitted(player: &mut Player, rig: &Rig, clock: &Clock) -> Vec<Event> {
    let mut palette = vec![PaletteJoint::IDENTITY; rig.end - rig.start];
    let mut events = Vec::new();
    player.evaluate_clock(
        rig,
        clock.phase.unwrap().instance,
        clock,
        &mut palette,
        &mut events,
    );
    events
}

fn chain() -> Rig {
    let mut rig = stage_rig(true);
    rig.nodes[0].default = "normal".into();
    rig.nodes[0].clips[0].state.name = "normal".into();
    rig.nodes[0].clips[0].state.looping = false;
    rig.nodes[0].clips[0].state.next_state = Some("damage".into());
    rig.nodes[0].clips[1].state.name = "damage".into();
    rig.nodes[0].clips[1].state.looping = false;
    rig.nodes[0].clips[1].state.next_state = Some("death".into());
    rig.nodes[0].clips[2].state.name = "death".into();
    rig.nodes[0].clips[2].state.looping = true;
    rig.nodes[0].clips[2].state.next_state = None;
    rig
}

#[test]
fn hidden_finish_waits_for_one_active_visit_and_preserves_old_events() {
    let mut rig = chain();
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 1.0,
        particle: Some("impact".into()),
        trigger_once: true,
        ..Default::default()
    });
    let mut player = Player::new(&rig);
    let clock = clock(0.0, 8.0, &[(0.0, 0.0, "normal")]);
    player.advance_hidden(&rig, &clock);
    assert_eq!(player.root_playing(&rig), Some("normal"));
    assert_eq!(player.root_clip_phase(&rig), 2.0);
    let events = admitted(&mut player, &rig, &clock);
    assert_eq!(
        events.len(),
        1,
        "old-state events precede its admitted finish"
    );
    assert_eq!(player.root_playing(&rig), Some("damage"));
    assert_eq!(player.root_clip_phase(&rig), 0.0);
    admitted(&mut player, &rig, &clock);
    assert_eq!(
        player.root_playing(&rig),
        Some("damage"),
        "no hidden-time successor chain"
    );
}

#[test]
fn cold_hidden_callbacks_use_each_phase_timestamp_even_with_frozen_active_age() {
    let rig = chain();
    let mut player = Player::new(&rig);
    player.advance_hidden(
        &rig,
        &clock(
            0.0,
            5.0,
            &[
                (0.0, 0.0, "normal"),
                (0.0, 2.0, "damage"),
                (0.0, 4.0, "death"),
            ],
        ),
    );
    assert_eq!(player.root_playing(&rig), Some("death"));
    assert_eq!(player.root_clip_phase(&rig), 1.0);
    let playing = player.nodes[0].as_ref().unwrap();
    assert_eq!(
        playing.previous,
        Some((1, 2.0)),
        "the prior damage clip advanced before death"
    );
}

#[test]
fn initial_same_default_request_reenters_and_uses_native_positive_range() {
    let rig = stage_rig(false);
    let mut player = Player::new(&rig);
    let mut clock = clock(0.0, 0.0, &[(0.0, 0.0, "cruise")]);
    clock.phase.as_mut().unwrap().instance.native_flags = 8;
    clock.phase.as_mut().unwrap().instance.animation_phase = 1.25;
    player.advance_hidden(&rig, &clock);
    let playing = player.nodes[0].as_ref().unwrap();
    assert_eq!(
        playing.random.index, 1,
        "single variants draw no RNG; only the positive requested range draws phase"
    );
    assert!(playing.phase_offset > 0.0 && playing.phase_offset < 1.25);
}

#[test]
fn nonpositive_request_range_falls_back_to_positive_authored_range() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.random_start = 0.75;
    let mut phases = Vec::new();
    for range in [-1.0, 0.0] {
        let mut player = Player::new(&rig);
        let mut clock = clock(0.0, 0.0, &[(0.0, 0.0, "cruise")]);
        clock.phase.as_mut().unwrap().instance.native_flags = 8;
        clock.phase.as_mut().unwrap().instance.animation_phase = range;
        player.advance_hidden(&rig, &clock);
        phases.push(player.root_clip_phase(&rig));
    }
    assert_eq!(phases[0], phases[1]);
    assert!(phases[0] > 0.0 && phases[0] < 0.75);
}

#[test]
fn hidden_loop_wraps_but_active_elapsed_guards_once_events() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        particle: Some("once".into()),
        trigger_once: true,
        ..Default::default()
    });
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 4.75, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.root_clip_phase(&rig), 0.75);
    assert_eq!(
        admitted(
            &mut player,
            &rig,
            &clock(0.0, 4.75, &[(0.0, 0.0, "cruise")])
        )
        .len(),
        1
    );
    assert!(
        admitted(
            &mut player,
            &rig,
            &clock(3.0, 7.75, &[(0.0, 0.0, "cruise")])
        )
        .is_empty()
    );
}

#[test]
fn phase_rewind_reinitializes_requests_and_never_backfills_hidden_events() {
    let rig = stage_rig(false);
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 3.0, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.root_clip_phase(&rig), 1.0);
    player.advance_hidden(&rig, &clock(0.0, 0.25, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.root_clip_phase(&rig), 0.25);
    assert_eq!(player.requested, 1);
    assert!(player.kept.is_empty());
}

fn native_source() -> super::super::mesh_pose::Source {
    super::super::mesh_pose::Source {
        pdxmesh: true,
        skeleton: Vec::new(),
        entity_locators: Vec::new(),
        binary_locators: Vec::new(),
        mesh_scale: 1.0,
        entity_scale: 2.0,
        frozen: None,
        game_data: None,
    }
}

#[test]
fn dummy_entity_animation_is_not_a_played_pdxmesh_hit_probe() {
    let mut rig = stage_rig(false);
    let mut source = native_source();
    source.pdxmesh = false;
    rig.nodes[0].native = Some(Arc::new(source));
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]));
    assert!(!player.phase_advances_hidden(&rig));
    assert_eq!(player.root_clip_phase(&rig), 0.0);
}

#[test]
fn zero_joint_pdxmesh_can_still_have_a_played_animation() {
    let mut rig = stage_rig(false);
    rig.nodes[0].native = Some(Arc::new(native_source()));
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]));
    assert!(player.phase_advances_hidden(&rig));
    assert_eq!(player.root_clip_phase(&rig), 0.75);
}

#[test]
fn single_sample_clip_probe_has_native_one_frame_duration() {
    let mut rig = stage_rig(false);
    Arc::make_mut(rig.nodes[0].clips[0].animation.as_mut().unwrap()).samples = 1;
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.root_clip_phase(&rig), 0.75);
}

#[test]
fn hidden_parent_advances_played_child_but_freezes_dummy_child_phase() {
    let mut rig = stage_rig(false);
    let mut played = rig.nodes[0].clone();
    played.parent = Some((0, "absent".into(), "played".into()));
    played.follows_parent = true;
    played.native = Some(Arc::new(native_source()));
    played.rigid = 1;
    let mut dummy = played.clone();
    dummy.parent = Some((0, "absent".into(), "dummy".into()));
    dummy.rigid = 2;
    Arc::make_mut(dummy.native.as_mut().unwrap()).pdxmesh = false;
    rig.nodes.extend([played, dummy]);
    rig.end = 3;
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 1.0, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.native_samples[0].unwrap().age, 1.0);
    assert_eq!(player.native_samples[1].unwrap().age, 1.0);
    assert_eq!(player.native_samples[2].unwrap().age, 0.0);
}

#[test]
fn hidden_attached_dummy_root_freezes_without_freezing_its_played_child() {
    let mut rig = stage_rig(false);
    let mut source = native_source();
    source.pdxmesh = false;
    rig.nodes[0].native = Some(Arc::new(source));
    let mut child = rig.nodes[0].clone();
    child.native = Some(Arc::new(native_source()));
    child.parent = Some((0, "absent".into(), "played".into()));
    child.follows_parent = true;
    child.rigid = 1;
    rig.nodes.push(child);
    rig.end = 2;
    let mut player = Player::new(&rig);
    let mut clock = clock(0.0, 1.0, &[(0.0, 0.0, "cruise")]);
    player.advance_hidden(&rig, &clock);
    assert_eq!(player.native_samples[0].unwrap().age, 0.0);
    assert_eq!(player.native_samples[1].unwrap().age, 1.0);
    clock.phase.as_mut().unwrap().root_advances_hidden = true;
    clock.phase.as_mut().unwrap().time = 2.0;
    player.advance_hidden(&rig, &clock);
    assert_eq!(
        player.native_samples[0].unwrap().age,
        1.0,
        "manager root is visited even while inactive"
    );
    assert_eq!(player.native_samples[1].unwrap().age, 2.0);
}

#[test]
fn unplaced_phase_actor_is_initialized_retained_and_retired_with_its_clock() {
    use crate::{ModelField, animate_players};
    let rigs = vec![Arc::new(stage_rig(false))];
    let palette = vec![PaletteJoint::IDENTITY];
    let mut players = BTreeMap::new();
    let mut field = ModelField::default();
    field
        .clocks
        .insert((0, 7, 0), clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]));
    let mut events = Vec::new();
    animate_players(
        &mut players,
        &rigs,
        &palette,
        &mut field,
        1.0,
        &|_| true,
        &mut events,
    );
    assert_eq!(players[&(0, 7, 0)].root_clip_phase(&rigs[0]), 0.75);
    assert!(
        events.is_empty(),
        "an unplaced actor only advances hidden requests and phase"
    );
    field
        .clocks
        .get_mut(&(0, 7, 0))
        .unwrap()
        .phase
        .as_mut()
        .unwrap()
        .time = 1.25;
    animate_players(
        &mut players,
        &rigs,
        &palette,
        &mut field,
        2.0,
        &|_| true,
        &mut events,
    );
    assert_eq!(players[&(0, 7, 0)].root_clip_phase(&rigs[0]), 1.25);
    field.clocks.get_mut(&(0, 7, 0)).unwrap().phase = None;
    animate_players(
        &mut players,
        &rigs,
        &palette,
        &mut field,
        3.0,
        &|_| true,
        &mut events,
    );
    assert!(
        players.is_empty(),
        "ordinary clocks do not retain unplaced players"
    );
}

#[test]
fn first_zero_time_frame_samples_phase_actors_and_leaves_ordinary_poses_frozen() {
    use crate::{ModelField, ModelLibrary, animate_players};
    let rigs = vec![Arc::new(stage_rig(false))];
    let palette = vec![PaletteJoint::IDENTITY];
    let mut players = BTreeMap::from([((0, 8, 0), Player::new(&rigs[0]))]);
    let mut field = ModelField::build(
        &ModelLibrary::default(),
        vec![
            (0, instance()),
            (
                0,
                ModelInstance {
                    animation_seed: 8,
                    ..instance()
                },
            ),
        ],
        256.0,
    );
    field
        .clocks
        .insert((0, 7, 0), clock(0.0, 0.0, &[(0.0, 0.0, "cruise")]));
    animate_players(
        &mut players,
        &rigs,
        &palette,
        &mut field,
        0.0,
        &|_| true,
        &mut Vec::new(),
    );
    assert_eq!(players.len(), 1);
    assert_eq!(players[&(0, 7, 0)].root_playing(&rigs[0]), Some("cruise"));
    assert!(
        field
            .instances
            .iter()
            .find(|i| i.animation_seed == 7)
            .unwrap()
            .palette_offset
            > 0
    );
    assert_eq!(
        field
            .instances
            .iter()
            .find(|i| i.animation_seed == 8)
            .unwrap()
            .palette_offset,
        0
    );
}

#[test]
fn same_state_hidden_callbacks_have_distinct_event_incarnations() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        particle: Some("impact".into()),
        ..Default::default()
    });
    let mut player = Player::new(&rig);
    let first = admitted(
        &mut player,
        &rig,
        &clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]),
    );
    let second = admitted(
        &mut player,
        &rig,
        &clock(0.0, 1.75, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "cruise")]),
    );
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_ne!(
        first[0].key, second[0].key,
        "active-age equality cannot merge distinct callbacks' particles"
    );
}

#[test]
fn hidden_crossfade_keeps_both_played_phases_and_expires_on_phase_age() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[1].state.blend = 2.0;
    let mut player = Player::new(&rig);
    player.advance_hidden(
        &rig,
        &clock(0.0, 1.5, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    let sample = player.native_samples[0].unwrap();
    assert_eq!(sample.age, 0.5);
    assert_eq!(sample.previous, Some((0, 1.5, 0.25)));
    player.advance_hidden(
        &rig,
        &clock(0.0, 3.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    assert!(player.native_samples[0].unwrap().previous.is_none());
}

#[test]
fn hidden_explicit_transition_keeps_only_previously_admitted_particles() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        particle: Some("kept".into()),
        keep_particle: true,
        ..Default::default()
    });
    let mut player = Player::new(&rig);
    let emitted = admitted(
        &mut player,
        &rig,
        &clock(0.0, 0.75, &[(0.0, 0.0, "cruise")]),
    );
    assert_eq!(emitted.len(), 1);
    player.advance_hidden(
        &rig,
        &clock(0.0, 1.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    let retained = admitted(
        &mut player,
        &rig,
        &clock(0.0, 3.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].key, emitted[0].key);
    assert_eq!(retained[0].emission_end, Some(0.5));
    assert_eq!(
        retained[0].age, 2.5,
        "retained particles continue on global phase age"
    );
}

#[test]
fn never_admitted_hidden_trigger_is_not_synthesized_as_a_kept_particle() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        particle: Some("kept".into()),
        keep_particle: true,
        ..Default::default()
    });
    let mut player = Player::new(&rig);
    player.advance_hidden(
        &rig,
        &clock(0.0, 3.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    assert!(player.kept.is_empty());
    assert!(
        admitted(
            &mut player,
            &rig,
            &clock(0.0, 3.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive"),])
        )
        .is_empty()
    );
}

#[test]
fn state_events_use_native_binary_precedence_and_name_reached_locator_refusals() {
    use super::super::mesh_pose::{IDENTITY, Locator};
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        node: "muzzle".into(),
        particle: Some("impact".into()),
        ..Default::default()
    });
    let locator = |x| {
        let mut local = IDENTITY;
        local[3][0] = x;
        Locator {
            name: "muzzle".into(),
            local,
            parent: None,
            error: None,
        }
    };
    let mut source = native_source();
    source.entity_locators.push(locator(9.0));
    source.binary_locators.push(locator(3.0));
    rig.nodes[0].native = Some(Arc::new(source));
    let mut player = Player::new(&rig);
    let clock = clock(0.75, 0.75, &[(0.0, 0.0, "cruise")]);
    let events = admitted(&mut player, &rig, &clock);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].transform.translation, [6.0, 0.0, 0.0]);
    Arc::make_mut(rig.nodes[0].native.as_mut().unwrap()).binary_locators[0].error =
        Some("fixture origin ordinal unavailable".into());
    assert!(admitted(&mut player, &rig, &clock).is_empty());
    assert_eq!(player.event_refusals().values().sum::<usize>(), 1);
    assert!(
        player
            .event_refusals()
            .keys()
            .next()
            .unwrap()
            .contains("muzzle")
    );
}

#[test]
fn native_entity_locator_particle_survives_zero_mesh_scale() {
    use super::super::mesh_pose::{IDENTITY, Locator};
    let mut rig = stage_rig(false);
    rig.nodes[0].scale = 0.0;
    rig.nodes[0].mesh_scale = 0.0;
    rig.nodes[0].clips[0].state.events.push(mesh::EntityEvent {
        time: 0.5,
        node: "muzzle".into(),
        particle: Some("impact".into()),
        ..Default::default()
    });
    let mut source = native_source();
    source.mesh_scale = 0.0;
    source.entity_scale = 1.0;
    source.entity_locators.push(Locator {
        name: "muzzle".into(),
        local: IDENTITY,
        parent: None,
        error: None,
    });
    rig.nodes[0].native = Some(Arc::new(source));
    let mut player = Player::new(&rig);
    let events = admitted(
        &mut player,
        &rig,
        &clock(0.75, 0.75, &[(0.0, 0.0, "cruise")]),
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].transform.uniform_scale(), 1.0);
}

#[test]
fn animationless_state_retains_played_phase_and_new_animation_blends_from_it() {
    let mut rig = stage_rig(false);
    rig.nodes[0].native = Some(Arc::new(native_source()));
    Arc::make_mut(rig.nodes[0].clips[0].animation.as_mut().unwrap())
        .joints
        .push(mesh::AnimationJoint {
            name: "probe".into(),
            translation: [2.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
            ..Default::default()
        });
    rig.nodes[0].clips[1].animation = None;
    let mut after = rig.nodes[0].clips[0].clone();
    after.state.name = "after".into();
    after.state.blend = 1.0;
    Arc::make_mut(after.animation.as_mut().unwrap()).joints[0].translation = [6.0, 0.0, 0.0];
    rig.nodes[0].clips.push(after);
    let mut player = Player::new(&rig);
    player.advance_hidden(
        &rig,
        &clock(0.0, 2.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    assert_eq!(player.root_playing(&rig), Some("dive"));
    assert_eq!(player.native_samples[0].unwrap().clip, 0);
    assert_eq!(player.native_samples[0].unwrap().age, 2.0);
    assert!(player.phase_advances_hidden(&rig));
    assert_eq!(
        player.native_locals(&rig, 0).unwrap().unwrap().poses[0].translation,
        [2.0, 0.0, 0.0],
        "the prior actual played pose remains"
    );
    player.advance_hidden(
        &rig,
        &clock(
            0.0,
            3.0,
            &[
                (0.0, 0.0, "cruise"),
                (0.0, 1.0, "dive"),
                (0.0, 2.5, "after"),
            ],
        ),
    );
    assert_eq!(player.root_playing(&rig), Some("after"));
    assert_eq!(player.native_samples[0].unwrap().age, 0.5);
    assert_eq!(
        player.native_samples[0].unwrap().previous,
        Some((0, 3.0, 0.5)),
        "blend source is the actual old clip, not the animationless entity state"
    );
    assert_eq!(
        player.native_locals(&rig, 0).unwrap().unwrap().poses[0].translation,
        [4.0, 0.0, 0.0],
        "the newly played pose blends from the retained actual source"
    );
}

fn with_child(mut rig: Rig) -> Rig {
    let mut child = rig.nodes[0].clone();
    child.native = Some(Arc::new(native_source()));
    child.parent = Some((0, "absent".into(), "child".into()));
    child.follows_parent = true;
    child.rigid = 1;
    rig.nodes.push(child);
    rig.end = 2;
    rig
}

#[test]
fn played_child_uses_own_phase_while_unplayed_child_copies_raw_parent_phase() {
    let mut rig = with_child(stage_rig(false));
    rig.nodes[0].clips[0].state.speed = 2.0;
    rig.nodes[0].clips[0].state.start = 0.25;
    rig.nodes[1].clips[0].state.speed = 0.5;
    rig.nodes[1].clips[0].state.start = 0.1;
    let mut dummy = rig.nodes[1].clone();
    dummy.parent = Some((0, "absent".into(), "dummy".into()));
    dummy.rigid = 2;
    Arc::make_mut(dummy.native.as_mut().unwrap()).pdxmesh = false;
    rig.nodes.push(dummy);
    rig.end = 3;
    let mut player = Player::new(&rig);
    admitted(&mut player, &rig, &clock(0.5, 0.5, &[(0.0, 0.0, "cruise")]));
    assert_eq!(
        player.native_samples[1].unwrap().age,
        0.5,
        "played child keeps its own AnimInstance time"
    );
    let dummy = player.native_samples[2].unwrap();
    let state = &rig.nodes[2].clips[dummy.clip].state;
    assert!(
        (dummy.age * state.speed + state.start - 1.25).abs() < 1e-5,
        "unplayed child copies raw parent phase, independent of its authored speed/start"
    );
    player.advance_hidden(&rig, &clock(0.5, 1.0, &[(0.0, 0.0, "cruise")]));
    assert_eq!(player.native_samples[1].unwrap().age, 1.0);
    assert_eq!(
        player.native_samples[2].unwrap().age,
        dummy.age,
        "hidden unplayed child freezes"
    );
}

#[test]
fn propagated_missing_child_state_is_noop_for_ordinary_native_nodes() {
    content_reader_script::test_game::enter();
    let mut rig = with_child(stage_rig(false));
    rig.nodes[0].clips[0]
        .state
        .propagate
        .push(("child".into(), "dive".into()));
    rig.nodes[0].clips[1]
        .state
        .propagate
        .push(("child".into(), "missing".into()));
    let mut player = Player::new(&rig);
    let mut palette = vec![PaletteJoint::IDENTITY; 2];
    let mut unit = instance();
    unit.animation = named_request("cruise");
    player.evaluate(&rig, unit, 0.0, &mut palette, &mut Vec::new());
    let old = player.nodes[1].as_ref().unwrap().start;
    unit.animation = named_request("dive");
    player.evaluate(&rig, unit, 1.0, &mut palette, &mut Vec::new());
    let child = player.nodes[1].as_ref().unwrap();
    assert_eq!(rig.nodes[1].clips[child.clip].state.name, "dive");
    assert_eq!(
        child.start, old,
        "an absent propagated state must not reset the existing child default"
    );
}

#[test]
fn naval_parent_entry_forces_same_child_while_ordinary_packet_suppresses_it() {
    content_reader_script::test_game::enter();
    let mut rig = with_child(stage_rig(false));
    rig.nodes[0].native = Some(Arc::new(native_source()));
    rig.nodes[0].clips[1]
        .state
        .propagate
        .push(("child".into(), "cruise".into()));
    let mut player = Player::new(&rig);
    let mut palette = vec![PaletteJoint::IDENTITY; 2];
    let mut unit = instance();
    unit.animation = named_request("cruise");
    player.evaluate(&rig, unit, 0.0, &mut palette, &mut Vec::new());
    player.evaluate(&rig, unit, 0.5, &mut palette, &mut Vec::new());
    assert_eq!(player.nodes[1].as_ref().unwrap().start, 0.0);
    unit.animation = named_request("dive");
    player.evaluate(&rig, unit, 1.0, &mut palette, &mut Vec::new());
    assert_eq!(
        player.nodes[1].as_ref().unwrap().start,
        0.0,
        "the ordinary packet's same-state suppression is also passed to its child"
    );
    let mut player = Player::new(&rig);
    player.advance_hidden(
        &rig,
        &clock(0.0, 1.0, &[(0.0, 0.0, "cruise"), (0.0, 1.0, "dive")]),
    );
    assert_eq!(
        player.nodes[1].as_ref().unwrap().event_time,
        1.0,
        "the forced naval parent packet reenters the same valid child state"
    );
}

#[test]
fn played_child_finishes_before_parent_and_parent_propagation_runs_last() {
    let mut rig = with_child(chain());
    Arc::make_mut(rig.nodes[1].clips[0].animation.as_mut().unwrap()).samples = 2;
    rig.nodes[0].clips[1]
        .state
        .propagate
        .push(("child".into(), "death".into()));
    let mut player = Player::new(&rig);
    player.advance_hidden(&rig, &clock(0.0, 1.5, &[(0.0, 0.0, "normal")]));
    admitted(&mut player, &rig, &clock(0.0, 1.5, &[(0.0, 0.0, "normal")]));
    assert_eq!(player.root_playing(&rig), Some("normal"));
    assert_eq!(
        rig.nodes[1].clips[player.nodes[1].as_ref().unwrap().clip]
            .state
            .name,
        "damage",
        "a following child has its own admitted automatic finish"
    );
    admitted(&mut player, &rig, &clock(0.5, 2.0, &[(0.0, 0.0, "normal")]));
    assert_eq!(player.root_playing(&rig), Some("damage"));
    assert_eq!(
        rig.nodes[1].clips[player.nodes[1].as_ref().unwrap().clip]
            .state
            .name,
        "death",
        "the later parent setter overwrites the child's earlier automatic state"
    );
}

#[test]
fn native_zero_weight_variants_choose_uniformly_with_one_draw() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.chance_weight = Some(0);
    let mut variant = rig.nodes[0].clips[0].clone();
    variant.state.chance_weight = Some(i32::MIN);
    rig.nodes[0].clips.push(variant);
    let mut random = Random::new(7);
    let mut witness = random.clone();
    let expected = [0, 2][(f64::from(witness.next_word()) / 2_147_483_648.0 * 2.0) as usize];
    assert_eq!(
        pick(&rig.nodes[0], "cruise", &mut random, true),
        Some(expected)
    );
    assert_eq!(
        random.index, 1,
        "zero-total fallback still consumes the one native variant draw"
    );
    let mut single = Random::new(7);
    assert_eq!(pick(&rig.nodes[0], "dive", &mut single, true), Some(1));
    assert_eq!(
        single.index, 0,
        "a single variant never consumes a weighted draw"
    );
}

#[test]
fn native_variant_weights_use_exact_signed_positive_values() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.chance = 100.0;
    rig.nodes[0].clips[0].state.chance_weight = Some(0);
    let mut variant = rig.nodes[0].clips[0].clone();
    variant.state.chance = 0.0;
    variant.state.chance_weight = Some(1);
    rig.nodes[0].clips.push(variant);
    assert_eq!(
        pick(&rig.nodes[0], "cruise", &mut Random::new(7), true),
        Some(2)
    );
    assert_eq!(
        pick(&rig.nodes[0], "cruise", &mut Random::new(7), false),
        Some(0),
        "ordinary float selection remains independent of the native weight"
    );
}

#[test]
fn native_zero_chance_without_next_state_holds_after_clip_finish() {
    let mut rig = stage_rig(false);
    rig.nodes[0].clips[0].state.looping = false;
    rig.nodes[0].clips[0].state.chance_weight = Some(0);
    let mut player = Player::new(&rig);
    admitted(&mut player, &rig, &clock(0.0, 3.0, &[(0.0, 0.0, "cruise")]));
    let playing = player.nodes[0].as_ref().unwrap();
    assert_eq!(
        playing.cycle, 0,
        "native omitted/zero chance forbids automatic same-state entry"
    );
    assert_eq!(playing.event_time, 0.0);
    assert_eq!(player.root_clip_phase(&rig), 2.0);
}

#[test]
fn native_positive_chance_without_next_repeats_current_successor() {
    let mut rig = chain();
    rig.nodes[0].clips[1].state.next_state = None;
    rig.nodes[0].clips[1].state.chance_weight = Some(50_000);
    let mut player = Player::new(&rig);
    admitted(&mut player, &rig, &clock(2.0, 2.0, &[(0.0, 0.0, "normal")]));
    assert_eq!(player.root_playing(&rig), Some("damage"));
    admitted(&mut player, &rig, &clock(4.0, 4.0, &[(0.0, 0.0, "normal")]));
    assert_eq!(
        player.root_playing(&rig),
        Some("damage"),
        "fallback repeats the current state's hash"
    );
    assert_eq!(player.root_clip_phase(&rig), 0.0);
}
