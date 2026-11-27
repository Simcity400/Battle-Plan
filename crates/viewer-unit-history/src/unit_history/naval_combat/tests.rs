
use super::*;
use content_reader_mesh::mesh::{Animation, EntityState, Mesh, NavalHitMissEffect};
use viewer_models::entity_animation::{Clip, Node};
use crate::unit_history::{self, Kind};
use viewer_unit_reads::model_admission::Gates;
use viewer_runtime::unit_class::CounterClass;

const COMBAT: u64 = 0x1234_5678_0000_002b;

fn geometry(x: f32) -> ModelInstance {
    ModelInstance {
        position: [x, 0.0, 3.0],
        rotation: x / 100.0,
        scale: 1.0,
        ..Default::default()
    }
}

fn target(id: u64, slot: usize) -> Target {
    Target {
        province: 1,
        identity: id,
        part: slot as u32,
        order: 0,
        occupant: id.wrapping_add(slot as u64),
        kind: Kind::Ship,
        mesh: slot + 1,
        instance: geometry(0.0),
        origin: [0.0; 3],
    }
}

fn battle(id: u64, stamp: i32) -> Battle {
    let rig = Arc::new(Rig {
        nodes: vec![Node {
            mesh: Arc::new(Mesh::default()),
            native: Some(Arc::new(mesh_pose::Source {
                pdxmesh: false,
                skeleton: Vec::new(),
                entity_locators: Vec::new(),
                binary_locators: Vec::new(),
                mesh_scale: 1.0,
                entity_scale: 1.0,
                frozen: None,
                game_data: None,
            })),
            clips: [
                "idle",
                "defend",
                "attack",
                "damaged_defend",
                "damaged_attack",
                "death",
            ]
            .map(|name| Clip {
                state: EntityState {
                    name: name.to_owned(),
                    looping: true,
                    chance: 1.0,
                    speed: 1.0,
                    ..Default::default()
                },
                animation: None,
            })
            .into_iter()
            .collect(),
            default: "idle".to_owned(),
            follows_parent: false,
            parent: None,
            scale: 1.0,
            mesh_scale: 1.0,
            skin_base: None,
            rigid: 0,
            locators: BTreeMap::new(),
        }],
        start: 0,
        end: 1,
    });
    let palette = Arc::new(vec![PaletteJoint::IDENTITY]);
    Battle {
        id,
        stamp,
        root: target(id, 0),
        rig: Arc::clone(&rig),
        palette: Arc::clone(&palette),
        locators: std::array::from_fn(|_| None),
        hulls: std::array::from_fn(|slot| Some(target(id, slot))),
        offered: [true; 8],
        impacts: std::array::from_fn(|slot| {
            Some(Arc::new(impacts::Info::new(
                Arc::clone(&rig),
                Arc::clone(&palette),
                if slot < 4 { "defend" } else { "attack" },
            )))
        }),
        effects: std::array::from_fn(|_| Vec::new()),
        health: [Health {
            capital: 100_000,
            screens: 100_000,
            sunk_fraction: 0.0,
        }; 2],
        death_time: 2.0,
    }
}

fn frame(steady: bool) -> Frame {
    Frame {
        delta: 0.0,
        scale_delta: 0.0,
        gates: Gates::OFF,
        shown: [true, false],
        arrows: false,
        steady,
    }
}

fn visit(
    history: &mut History,
    battle: &Battle,
    province: u32,
    x: f32,
    epoch: &mut u32,
    budget: &mut Budget,
) {
    history.visit(
        province,
        Some(Selection {
            battle,
            first: battle.id,
            root: geometry(x),
        }),
        frame(false),
        false,
        epoch,
        budget,
    );
}

fn advance(history: &mut History, delta: f32) {
    history.begin_frame(delta);
    history.advance(delta);
}

fn hull_identities(history: &History, id: u64) -> [Option<u32>; 8] {
    history.entities[&id]
        .parts
        .each_ref()
        .map(|hull| hull.as_ref().map(|hull| hull.part.identity))
}

fn naval_sources(battle: Battle) -> unit_history::naval::Sources {
    unit_history::naval::Sources {
        candidates: vec![unit_history::naval::Candidate {
            province: 1,
            identity: 7,
            ordinal: 0,
            slot: 9,
            priority: 3,
            ours: true,
            class: CounterClass::TaskForce,
            land_eligible: false,
            sample_count: 0,
            next_arc: 0.0,
            route: Vec::new(),
            speed: 1.0,
            min_arc: 0.0,
            arrival_allowed: true,
            day: 0,
        }],
        formations: Vec::new(),
        combats: Sources {
            links: [(7, [battle.id; 2])].into_iter().collect(),
            battles: [(battle.id, battle)].into_iter().collect(),
            roots: [(1, geometry(10.0))].into_iter().collect(),
        },
        positions: Vec::new(),
    }
}

#[test]
fn last_acquire_sets_shared_geometry_release_deactivates_and_reacquire_retains_hulls() {
    let battle = battle(COMBAT, 100);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(8, false);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    assert_eq!(budget.remaining(), 8, "fresh root does not attach hulls");
    visit(&mut history, &battle, 1, 11.0, &mut epoch, &mut budget);
    let root_identity = history.entities[&COMBAT].template.root.identity;
    let identities = hull_identities(&history, COMBAT);
    assert!(identities.iter().all(Option::is_some));
    advance(&mut history, 2.0);

    visit(&mut history, &battle, 2, 20.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 30.0, &mut epoch, &mut budget);
    assert_eq!(history.entities.len(), 1);
    assert_eq!(
        history.entities[&COMBAT].template.root.target.instance,
        geometry(20.0)
    );
    assert_eq!(history.entities[&COMBAT].template.root.target.province, 2);

    history.leave(2, true, false);
    assert_eq!(history.provinces.get(&1), Some(&COMBAT));
    assert!(!history.provinces.contains_key(&2));
    assert!(!history.entities[&COMBAT].active);
    advance(&mut history, 3.0);
    visit(&mut history, &battle, 1, 40.0, &mut epoch, &mut budget);
    let entity = &history.entities[&COMBAT];
    assert!(
        !entity.active,
        "a cached province does not reactivate a shared wrapper"
    );
    assert_eq!(entity.template.root.target.instance, geometry(20.0));
    assert_eq!(entity.template.clock.time, 2.0);
    assert_eq!(entity.template.clock.phase.unwrap().time, 5.0);
    assert_eq!(entity.parts[0].as_ref().unwrap().clock.time, 2.0);
    assert_eq!(
        entity.parts[0].as_ref().unwrap().clock.phase.unwrap().time,
        5.0
    );
    assert!(
        !entity.parts[0]
            .as_ref()
            .unwrap()
            .tracker
            .phase_advances_hidden()
    );

    let mut later_budget = Budget::new(8, false);
    visit(
        &mut history,
        &battle,
        3,
        50.0,
        &mut epoch,
        &mut later_budget,
    );
    let entity = &history.entities[&COMBAT];
    assert!(entity.active);
    assert_eq!(entity.template.root.identity, root_identity);
    assert_eq!(entity.template.root.target.instance, geometry(50.0));
    assert_eq!(entity.status, [1; 8]);
    assert_eq!(hull_identities(&history, COMBAT), identities);
    assert_eq!(
        entity.parts[0].as_ref().unwrap().clock.phase.unwrap().time,
        5.0
    );
    assert_eq!(later_budget.remaining(), 8);
    advance(&mut history, 1.0);
    assert_eq!(history.entities[&COMBAT].template.clock.time, 3.0);
    assert_eq!(
        history.entities[&COMBAT].parts[0]
            .as_ref()
            .unwrap()
            .clock
            .time,
        3.0
    );
}

#[test]
fn combat_ids_with_equal_low_words_keep_independent_wrappers_and_release() {
    let first = battle(COMBAT, 100);
    let second = battle(COMBAT ^ (1_u64 << 32), 100);
    assert_eq!(first.id as u32, second.id as u32);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(8, false);
    visit(&mut history, &first, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &second, 2, 20.0, &mut epoch, &mut budget);
    assert_eq!(history.entities.len(), 2);
    assert_ne!(
        history.entities[&first.id].template.root.identity,
        history.entities[&second.id].template.root.identity
    );
    history.leave(2, true, false);
    assert!(history.entities[&first.id].active);
    assert!(!history.entities[&second.id].active);
    assert_eq!(history.provinces.get(&1), Some(&first.id));
}

#[test]
fn perspective_reset_only_deactivates_while_full_reset_recreates_the_manager() {
    let mut battle = battle(COMBAT, 100);
    let other = self::battle(COMBAT ^ (1_u64 << 32), 100);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(8, false);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    battle.stamp += 1;
    battle.health[0].capital = 0;
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 2, 20.0, &mut epoch, &mut budget);
    visit(&mut history, &other, 4, 40.0, &mut epoch, &mut budget);
    let identities = hull_identities(&history, COMBAT);
    let root_identity = history.entities[&COMBAT].template.root.identity;
    assert_eq!(history.deaths.len(), 1);

    history.reset(Some(&[1].into_iter().collect()));
    assert_eq!(history.entities.len(), 2);
    assert!(!history.entities[&COMBAT].active);
    assert!(history.entities[&other.id].active);
    assert_eq!(history.provinces.get(&2), Some(&COMBAT));
    assert!(!history.provinces.contains_key(&1));
    assert_eq!(hull_identities(&history, COMBAT), identities);
    assert_eq!(history.entities[&COMBAT].status[0], 3);
    assert_eq!(
        history.deaths.len(),
        1,
        "detached death root survives province reset"
    );

    history.reset(None);
    assert!(history.entities.is_empty());
    assert!(history.provinces.is_empty());
    assert!(history.deaths.is_empty());
    assert!(history.effects.is_empty());
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    assert_ne!(
        history.entities[&COMBAT].template.root.identity,
        root_identity
    );
    assert_eq!(history.entities[&COMBAT].status, [0; 8]);
}

#[test]
fn first_non_naval_reference_reacquires_root_without_cached_health_or_probes() {
    for steady in [false, true] {
        let mut battle = battle(COMBAT, 100);
        battle.health = [Health {
            capital: 0,
            screens: 0,
            sunk_fraction: f32::NAN,
        }; 2];
        battle.locators[0] = Some("unavailable-if-probed".to_owned());
        let mut history = History::default();
        let mut epoch = 0;
        let mut budget = Budget::new(8, steady);
        for x in [10.0, 20.0] {
            history.visit(
                1,
                Some(Selection {
                    battle: &battle,
                    first: COMBAT ^ 1,
                    root: geometry(x),
                }),
                frame(steady),
                false,
                &mut epoch,
                &mut budget,
            );
            let entity = &history.entities[&COMBAT];
            assert!(entity.active);
            assert_eq!(entity.template.root.target.instance, geometry(x));
            assert_eq!(entity.status, [0; 8]);
            assert_eq!(entity.stamp, -1);
            assert!(entity.parts.iter().all(Option::is_none));
            assert!(history.deaths.is_empty());
            assert!(
                history.refusals.is_empty(),
                "a root-only acquisition must not probe locators"
            );
            assert_eq!(epoch, 1, "reacquisition retains the manager's root actor");
        }
        if !steady {
            assert_eq!(budget.remaining(), 8);
        }
    }
}

#[test]
fn budget_held_slots_advance_status_once_and_do_not_retry_after_reacquisition() {
    let mut battle = battle(COMBAT, 100);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(3, false);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    assert_eq!(history.entities[&COMBAT].status, [1; 8]);
    let identities = hull_identities(&history, COMBAT);
    assert!(identities[..3].iter().all(Option::is_some));
    assert!(identities[3..].iter().all(Option::is_none));
    assert_eq!(epoch, 4);
    assert_eq!(budget.remaining(), 0);

    battle.stamp += 1;
    let mut later_budget = Budget::new(8, false);
    visit(
        &mut history,
        &battle,
        1,
        10.0,
        &mut epoch,
        &mut later_budget,
    );
    history.leave(1, true, false);
    visit(
        &mut history,
        &battle,
        2,
        20.0,
        &mut epoch,
        &mut later_budget,
    );
    battle.stamp += 1;
    visit(
        &mut history,
        &battle,
        2,
        20.0,
        &mut epoch,
        &mut later_budget,
    );
    assert_eq!(hull_identities(&history, COMBAT), identities);
    assert_eq!(history.entities[&COMBAT].status, [1; 8]);
    assert_eq!(later_budget.remaining(), 8);
    assert_eq!(epoch, 4);
}

#[test]
fn province_show_bits_commit_after_combat_spends_the_shared_creation_budget() {
    let mut history = unit_history::History::default();
    history.set_naval(naval_sources(battle(COMBAT, 100)));
    let mut budget = Budget::new(0, false);
    let _ = history.step_with(
        0.0,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    assert_eq!(history.provinces[&1].shown, [false; 2]);
    assert_eq!(history.combats.entities[&COMBAT].status, [0; 8]);

    let mut budget = Budget::new(1, false);
    let _ = history.step_with(
        1.0,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    assert_eq!(budget.remaining(), 0);
    assert_eq!(
        history.provinces[&1].shown, [false; 2],
        "combat hull creation must precede the native shown-bit hold"
    );
    assert_eq!(history.combats.entities[&COMBAT].status, [1; 8]);
    assert_eq!(
        history.combats.entities[&COMBAT]
            .parts
            .iter()
            .flatten()
            .count(),
        1
    );

    let mut budget = Budget::new(1, false);
    let _ = history.step_with(
        2.0,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    assert_eq!(budget.remaining(), 1);
    assert_eq!(history.provinces[&1].shown, [true; 2]);
    assert_eq!(
        history.combats.entities[&COMBAT]
            .parts
            .iter()
            .flatten()
            .count(),
        1
    );
}

#[test]
fn duration_gap_clears_actors_only_above_48_and_retains_nonretryable_status() {
    let mut battle = battle(COMBAT, 100);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(8, false);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    let identities = hull_identities(&history, COMBAT);
    battle.stamp = 148;
    let mut later_budget = Budget::new(8, false);
    visit(
        &mut history,
        &battle,
        1,
        10.0,
        &mut epoch,
        &mut later_budget,
    );
    assert_eq!(hull_identities(&history, COMBAT), identities);
    battle.stamp = 197;
    visit(
        &mut history,
        &battle,
        1,
        10.0,
        &mut epoch,
        &mut later_budget,
    );
    assert!(history.entities[&COMBAT].parts.iter().all(Option::is_none));
    assert_eq!(history.entities[&COMBAT].status, [1; 8]);
    assert_eq!(history.entities[&COMBAT].stamp, 197);
    assert_eq!(later_budget.remaining(), 8);
    assert_eq!(epoch, 9);
    history.leave(1, true, false);
    visit(
        &mut history,
        &battle,
        2,
        20.0,
        &mut epoch,
        &mut later_budget,
    );
    assert_eq!(history.entities[&COMBAT].status, [1; 8]);
    assert!(history.entities[&COMBAT].parts.iter().all(Option::is_none));
}

#[test]
fn unordered_screen_fraction_retires_all_screen_actors_and_death_roots_keep_running() {
    let mut battle = battle(COMBAT, 100);
    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(8, false);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    let identities = hull_identities(&history, COMBAT);
    battle.stamp += 1;
    for health in &mut battle.health {
        health.sunk_fraction = f32::NAN;
    }
    visit(&mut history, &battle, 1, 10.0, &mut epoch, &mut budget);
    assert_eq!(history.entities[&COMBAT].status, [1, 3, 3, 3, 1, 3, 3, 3]);
    assert_eq!(history.deaths.len(), 6);
    for death in &history.deaths {
        let slot = death.hull.part.target.part as usize;
        assert_eq!(Some(death.hull.part.identity), identities[slot]);
        assert_eq!(death.hull.clock.requests.last().unwrap().state, "death");
        assert_eq!(death.template.clock.requests[0].state, "idle");
    }
    history.leave(1, true, false);
    advance(&mut history, 2.0);
    assert!(!history.entities[&COMBAT].active);
    assert_eq!(
        history.entities[&COMBAT].parts[0]
            .as_ref()
            .unwrap()
            .clock
            .time,
        0.0
    );
    assert_eq!(
        history.deaths.len(),
        6,
        "death expiry is strict, not inclusive"
    );
    assert!(
        history
            .deaths
            .iter()
            .all(|death| death.hull.clock.time == 2.0)
    );
    advance(&mut history, 0.25);
    assert!(history.deaths.is_empty());
    assert!(
        history.entities.contains_key(&COMBAT),
        "retirement cleanup does not prune wrappers"
    );
}

#[test]
fn creation_damage_and_death_setters_precede_the_single_rendered_frame_update() {
    let mut battle = battle(COMBAT, 100);
    battle.death_time = 0.0;
    battle.offered = [true, false, false, false, false, false, false, false];
    let template = Arc::make_mut(&mut battle.rig);
    template.nodes[0].clips[0].state.looping = false;
    template.nodes[0].clips[0].state.chance_weight = Some(100_000);
    let idle_variant = template.nodes[0].clips[0].clone();
    template.nodes[0].clips.push(idle_variant);
    Arc::make_mut(template.nodes[0].native.as_mut().unwrap())
        .entity_locators
        .push(mesh_pose::Locator {
            name: "hull".to_owned(),
            local: mesh_pose::IDENTITY,
            parent: None,
            error: None,
        });
    battle.locators[0] = Some("hull".to_owned());

    let mut hull_rig = (*battle.rig).clone();
    let root = &mut hull_rig.nodes[0];
    Arc::make_mut(root.native.as_mut().unwrap()).pdxmesh = true;
    let normal = root
        .clips
        .iter_mut()
        .find(|clip| clip.state.name == "defend")
        .unwrap();
    normal.state.looping = false;
    normal.state.next_state = Some("hold".to_owned());
    normal.state.naval_hitmiss_error = Some("fixture copied timing refusal".to_owned());
    let mut animation = Animation::default();
    animation.fps = 1.0;
    animation.samples = 2;
    normal.animation = Some(Arc::new(animation));
    root.clips.push(Clip {
        state: EntityState {
            name: "hold".to_owned(),
            chance_weight: Some(100_000),
            speed: 1.0,
            ..Default::default()
        },
        animation: None,
    });
    for (requested, successor) in [
        ("damaged_defend", "damage_finished"),
        ("death", "death_finished"),
    ] {
        let state = &mut root
            .clips
            .iter_mut()
            .find(|clip| clip.state.name == requested)
            .unwrap()
            .state;
        state.looping = false;
        state.next_state = Some(successor.to_owned());
        root.clips.push(Clip {
            state: EntityState {
                name: successor.to_owned(),
                chance_weight: Some(0),
                speed: 1.0,
                ..Default::default()
            },
            animation: None,
        });
    }
    let hull_rig = Arc::new(hull_rig);
    battle.impacts[0] = Some(Arc::new(impacts::Info::new(
        Arc::clone(&hull_rig),
        Arc::clone(&battle.palette),
        "defend",
    )));
    let target = battle.hulls[0].as_mut().unwrap();
    target.instance.native_flags = 8;
    target.instance.animation_phase = 32.0;

    let mut history = unit_history::History::default();
    history.set_naval(naval_sources(battle.clone()));
    let mut budget = Budget::new(1, false);
    let _ = history.step_with(
        0.0,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    let template = &history.combats.entities[&COMBAT].template;
    let mut rendered = Player::new(&battle.rig);
    rendered.evaluate_clock(
        &battle.rig,
        template.root_instance(),
        &template.clock,
        &mut (*battle.palette).clone(),
        &mut Vec::new(),
    );
    assert_eq!(template.player.root_entry(), rendered.root_entry());
    assert_eq!(rendered.root_entry().unwrap().3, 1);

    let _ = history.step_with(
        1.0,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    let hull = history.combats.entities[&COMBAT].parts[0].as_ref().unwrap();
    assert!(
        history
            .combat_warnings()
            .iter()
            .any(|warning| warning.contains("fixture copied timing refusal")),
        "the post-probe manager update preserves reached refusal diagnostics"
    );
    let mut rendered = Player::new(&hull_rig);
    rendered.evaluate_clock(
        &hull_rig,
        actor_instance(&hull.part),
        &hull.clock,
        &mut (*battle.palette).clone(),
        &mut Vec::new(),
    );
    assert_eq!(rendered.root_playing(&hull_rig), Some("hold"));
    assert_eq!(rendered.root_clip_phase(&hull_rig), 1.0);
    assert_eq!(hull.tracker.root_entry(), rendered.root_entry());
    assert_eq!(rendered.root_entry().unwrap().3, 1);
    let entry = rendered.root_entry();
    let _ = history.combats.placed();
    let _ = history.combats.placed();
    assert_eq!(
        history.combats.entities[&COMBAT].parts[0]
            .as_ref()
            .unwrap()
            .tracker
            .root_entry(),
        entry,
        "creation-frame drawing does not admit a second finish"
    );

    for (time, health, expected) in [
        (1.25, 49_000, "damage_finished"),
        (1.5, 0, "death_finished"),
    ] {
        battle.stamp += 1;
        battle.health[0].capital = health;
        history.set_naval(naval_sources(battle.clone()));
        let _ = history.step_with(
            time,
            Gates::OFF,
            [true; 2],
            false,
            |_| true,
            &mut budget,
            |_, _| {},
        );
        let hull = if health == 0 {
            &history.combats.deaths[0].hull
        } else {
            history.combats.entities[&COMBAT].parts[0].as_ref().unwrap()
        };
        rendered.evaluate_clock(
            &hull_rig,
            actor_instance(&hull.part),
            &hull.clock,
            &mut (*battle.palette).clone(),
            &mut Vec::new(),
        );
        assert_eq!(rendered.root_playing(&hull_rig), Some(expected));
        assert_eq!(hull.tracker.root_entry(), rendered.root_entry());
        assert_eq!(rendered.root_entry().unwrap().3, 1);
    }
    let death = &history.combats.deaths[0];
    assert_eq!(death.birth, history.combats.lifetime_time);
    assert_eq!(death.template.clock.time, 0.25);
    assert_eq!(death.hull.clock.time, 1.5);
    assert_eq!(death.death_time, 0.0);
    assert!(
        !impacts::expired(history.combats.lifetime_time, death.birth, death.death_time),
        "a new death root has lifetime age zero despite its first played delta"
    );
    let _ = history.step_with(
        1.75,
        Gates::OFF,
        [true; 2],
        false,
        |_| true,
        &mut budget,
        |_, _| {},
    );
    assert!(history.combats.deaths.is_empty());
}

#[test]
fn cached_visits_probe_copied_timings_once_while_active_or_inactive_but_placing_does_not() {
    let mut battle = battle(COMBAT, 100);
    battle.offered = [true, false, false, false, true, false, false, false];
    battle.hulls = std::array::from_fn(|slot| matches!(slot, 0 | 4).then(|| target(COMBAT, slot)));
    let template = Arc::make_mut(&mut battle.rig);
    template.nodes[0].clips[0].state.looping = false;
    template.nodes[0].clips[0].state.chance_weight = Some(100_000);
    let idle_variant = template.nodes[0].clips[0].clone();
    template.nodes[0].clips.push(idle_variant);
    let source = Arc::make_mut(template.nodes[0].native.as_mut().unwrap());
    source.entity_locators = [("defender", -4.0), ("attacker", 4.0)]
        .map(|(name, x)| {
            let mut local = mesh_pose::IDENTITY;
            local[3][0] = x;
            mesh_pose::Locator {
                name: name.to_owned(),
                local,
                parent: None,
                error: None,
            }
        })
        .into_iter()
        .collect();
    battle.locators[0] = Some("defender".to_owned());
    battle.locators[4] = Some("attacker".to_owned());

    let mut animation = Animation::default();
    animation.fps = 1.0;
    animation.samples = 11;
    let animation = Arc::new(animation);
    let mut hull_rig = (*battle.rig).clone();
    let root = &mut hull_rig.nodes[0];
    let source = Arc::make_mut(root.native.as_mut().unwrap());
    source.pdxmesh = true;
    source.entity_locators = ["hit_effect_locator1", "miss_effect_locator1"]
        .map(|name| {
            let mut local = mesh_pose::IDENTITY;
            local[3][1] = 2.0;
            mesh_pose::Locator {
                name: name.to_owned(),
                local,
                parent: None,
                error: None,
            }
        })
        .into_iter()
        .collect();
    for clip in &mut root.clips {
        clip.animation = Some(Arc::clone(&animation));
        if clip.state.name == "defend" {
            clip.state.looping = false;
            clip.state.next_state = Some("hold".to_owned());
            clip.state.naval_hitmiss_effects = [1.0, 3.0]
                .map(|time| NavalHitMissEffect { time, kind: 1 })
                .into_iter()
                .collect();
        }
    }
    let hold = Clip {
        state: EntityState {
            name: "hold".to_owned(),
            chance: 1.0,
            chance_weight: Some(100_000),
            speed: 1.0,
            ..Default::default()
        },
        animation: None,
    };
    root.clips.extend([hold.clone(), hold]);
    let hull_rig = Arc::new(hull_rig);
    for (slot, state) in [(0, "defend"), (4, "attack")] {
        battle.impacts[slot] = Some(Arc::new(impacts::Info::new(
            Arc::clone(&hull_rig),
            Arc::clone(&battle.palette),
            state,
        )));
    }
    let mut effect_rig = (*battle.rig).clone();
    effect_rig.nodes[0].default = "impact_fixture_default".to_owned();
    effect_rig.nodes[0].clips[0].state.name = effect_rig.nodes[0].default.clone();
    let effect_animation = entity_animation::named_request(&effect_rig.nodes[0].default);
    battle.effects = [
        vec![Some((99, effect_animation))],
        Vec::new(),
        vec![Some((99, effect_animation))],
        Vec::new(),
    ];

    let mut history = History::default();
    let mut epoch = 0;
    let mut budget = Budget::new(2, false);
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 2, 0.0, &mut epoch, &mut budget);
    assert!(history.effects.is_empty());
    assert!(
        history.entities[&COMBAT].parts[0]
            .as_ref()
            .unwrap()
            .tracker
            .phase_advances_hidden()
    );

    let mut replacement = (*hull_rig).clone();
    for clip in &mut replacement.nodes[0].clips {
        clip.state.naval_hitmiss_effects.clear();
    }
    battle.impacts[0] = Some(Arc::new(impacts::Info::new(
        Arc::new(replacement),
        Arc::clone(&battle.palette),
        "defend",
    )));
    advance(&mut history, 1.5);
    let _ = history.placed();
    let _ = history.placed();
    assert!(
        history.effects.is_empty(),
        "advancing and drawing do not consume hit crossings"
    );
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    assert_eq!(history.entities[&COMBAT].stamp, 100);
    assert_eq!(
        history.effects.len(),
        1,
        "unchanged-duration cached visit probes copied timing"
    );
    let effect = &history.effects[0];
    assert_eq!(effect.part.target.mesh, 99);
    assert_eq!(effect.part.target.instance.animation, effect_animation);
    assert_eq!(effect.part.target.instance.position, [4.0, 2.0, 3.0]);
    assert!(
        effect.clock.requests.is_empty(),
        "effect factory retains its authored default"
    );
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    visit(&mut history, &battle, 2, 0.0, &mut epoch, &mut budget);
    assert_eq!(
        history.effects.len(),
        1,
        "ordered provinces consume the same phase once"
    );

    history.leave(2, true, false);
    advance(&mut history, 2.0);
    let _ = history.placed();
    assert_eq!(history.effects.len(), 1, "inactive placement does not emit");
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    let entity = &history.entities[&COMBAT];
    assert!(
        !entity.active,
        "cached visit does not reactivate the shared wrapper"
    );
    assert_eq!(entity.stamp, 100);
    assert_eq!(entity.parts[0].as_ref().unwrap().clock.time, 1.5);
    assert_eq!(
        entity.parts[0].as_ref().unwrap().clock.phase.unwrap().time,
        3.5
    );
    assert_eq!(
        history.effects.len(),
        2,
        "inactive cached visit reads the advanced played phase"
    );
    visit(&mut history, &battle, 1, 0.0, &mut epoch, &mut budget);
    let _ = history.placed();
    assert_eq!(
        history.effects.len(),
        2,
        "a consumed hidden crossing is not replayed"
    );
    assert!(history.refusals.is_empty());
    assert_eq!(
        budget.remaining(),
        0,
        "impact actors do not refund hull creation budget"
    );

    visit(&mut history, &battle, 3, 0.0, &mut epoch, &mut budget);
    advance(&mut history, 7.0);
    advance(&mut history, 0.25);
    let entry = history.entities[&COMBAT].parts[0]
        .as_ref()
        .unwrap()
        .tracker
        .root_entry()
        .unwrap();
    let root_entry = history.entities[&COMBAT]
        .template
        .player
        .root_entry()
        .unwrap();
    assert_eq!(entry.3, 2, "played finish followed by one no-next finish");
    assert!(entry.2 >= 2, "both hold entries choose a weighted variant");
    assert!(
        root_entry.3 >= 2,
        "the animationless template admits finishes"
    );
    for _ in 0..3 {
        visit(&mut history, &battle, 3, 0.0, &mut epoch, &mut budget);
        let _ = history.placed();
        assert_eq!(
            history.entities[&COMBAT].parts[0]
                .as_ref()
                .unwrap()
                .tracker
                .root_entry(),
            Some(entry),
            "cached phase probes preserve the hull incarnation and RNG"
        );
        assert_eq!(
            history.entities[&COMBAT].template.player.root_entry(),
            Some(root_entry),
            "placing and cached probes preserve the template incarnation"
        );
    }
}

fn province_frame_state(province: &unit_history::Province) -> String {
    let entities: Vec<_> = province
        .entities
        .iter()
        .map(|(slot, entity)| {
            let parts: Vec<_> = entity
                .parts
                .iter()
                .map(|(index, part)| {
                    let target = part.target;
                    (
                        index,
                        part.epoch,
                        part.identity,
                        &part.graphics,
                        (
                            target.province,
                            target.identity,
                            target.part,
                            target.order,
                            target.occupant,
                            target.kind,
                            target.mesh,
                        ),
                        bytemuck::bytes_of(&target.instance).to_vec(),
                        target.origin.map(f32::to_bits),
                    )
                })
                .collect();
            (
                slot,
                entity.kind,
                entity.scale.to_bits(),
                entity.shrinking,
                parts,
            )
        })
        .collect();
    format!(
        "{:?}",
        (
            province.shown,
            province.arrows,
            province.land_cursor,
            &province.land_visits,
            entities
        )
    )
}

fn assert_installed_history(history: &unit_history::History, expected_ids: &BTreeSet<u32>) {
    let dense = &history.provinces;
    assert_eq!(
        dense
            .records
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        expected_ids.iter().copied().collect::<Vec<_>>()
    );
    assert_eq!(
        dense.indices,
        expected_ids
            .iter()
            .enumerate()
            .map(|(index, &id)| (id, index))
            .collect()
    );
    assert_eq!(
        dense.populated.iter().collect::<Vec<_>>(),
        dense
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| (!record.province.entities.is_empty()).then_some(index))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        history.province_bounds,
        expected_ids
            .iter()
            .map(|&id| (id, history.bounds.get(&id).copied()))
            .collect::<Vec<_>>()
    );
    for record in &dense.records {
        match (&record.targets, history.targets.0.provinces.get(&record.id)) {
            (Some(joined), Some(targets)) => {
                assert!(Arc::ptr_eq(joined, targets) || **joined == **targets);
                assert!(Arc::ptr_eq(&record.province.lands, &targets.lands));
            }
            (None, None) => assert!(record.province.lands.is_empty()),
            _ => panic!("stale target join for province {}", record.id),
        }
        assert_eq!(
            record.has_candidates,
            history.naval.has_candidates(record.id)
        );
    }
}

#[test]
fn retained_province_installs_and_frames_preserve_operands_and_instance_bytes() {
    check_retained_province_frames(19, 16_384);
}

#[test]
fn retained_province_frames_preserve_sparse_high_ids() {
    check_retained_province_frames(100_019, u32::MAX);
}

fn check_retained_province_frames(stride: u32, last_id: u32) {
    use unit_history::{Target, test_support::GATES, test_support::target as unit_target};
    use viewer_gpu::{camera::{MapDefines, MapSize, Placement}, province_admission::Admission};
    use viewer_runtime::unit_handle::Unit;
    content_reader_script::test_game::enter();
    let ids: Vec<_> = (0..192u32)
        .map(|index| index * stride + 7)
        .chain([last_id - 2, last_id])
        .collect();
    let boxes = |count: usize, shift: f32| {
        Arc::new(
            ids.iter()
                .take(count)
                .enumerate()
                .map(|(index, &id)| {
                    let x = (index % 16) as f32 * 320.0 + shift;
                    let z = (index / 16) as f32 * 120.0;
                    (id, [x, z, x + 90.0, z + 70.0])
                })
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let mut bounds = boxes(160, 0.0);
    let mut history = unit_history::History::default();
    history.set(Vec::new(), Arc::clone(&bounds), false);
    let mut expected_ids: BTreeSet<_> = bounds.keys().copied().collect();
    let mut expected_bounds = Arc::clone(&bounds);
    let mut random = 0xa57e_1983u32;
    let mut next = || {
        random ^= random << 13;
        random ^= random >> 17;
        random ^= random << 5;
        random
    };
    let mut time = 0.0;
    let mut saw = [false; 5];
    for number in 0..1_200u32 {
        let roll = next();
        let hour = number / 17;
        if number.is_multiple_of(17) {
            if number.is_multiple_of(51) {
                bounds = boxes(160 + hour as usize % 35, (hour % 3) as f32);
            }
            let mut targets = Vec::new();
            for (index, &id) in ids.iter().enumerate() {
                if (index as u32 + hour) % 11 > 1 {
                    continue;
                }
                for slot in 0..3u64 {
                    let kind = if slot < 2 { Kind::Land } else { Kind::Ship };
                    for part in 0..=u32::from(slot == 2) {
                        targets.push(Target {
                            identity: u64::from(id) * 16 + slot,
                            part,
                            order: ((index as u32 + slot as u32) % 38),
                            occupant: u64::from(id) * 32 + slot + u64::from(hour % 2),
                            kind,
                            mesh: (hour as usize + part as usize) % 5,
                            instance: ModelInstance {
                                position: [index as f32 * 3.0, 10.0, part as f32],
                                rotation: -0.0,
                                scale: if slot == 1 { 0.3 } else { 1.0 },
                                animation_seed: id,
                                ..Default::default()
                            },
                            origin: [index as f32, 10.0, 0.0],
                            ..unit_target(id, kind)
                        });
                        if part == 0 && index % 3 == 0 {
                            let mut duplicate = *targets.last().unwrap();
                            duplicate.mesh += 7;
                            targets.push(duplicate);
                        }
                    }
                }
            }
            let mut combat = battle(COMBAT, 100 + hour as i32 * 7);
            combat.health[0].capital = if hour % 3 == 0 { 20_000 } else { 100_000 };
            combat.health[1].sunk_fraction = (hour % 3) as f32 * 0.25;
            let mut sources = naval_sources(combat);
            sources.candidates[0].province = ids[0];
            sources.candidates[0].sample_count = 8;
            sources.candidates[0].route = vec![ids[0], ids[1]];
            sources.candidates[0].next_arc = 0.5;
            sources.candidates[0].speed = 0.25;
            sources.candidates[0].day = hour as i32;
            sources.combats.roots = [(ids[0], geometry(10.0)), (ids[1], geometry(20.0))]
                .into_iter()
                .collect();
            let mut repeated = sources.candidates[0].clone();
            repeated.province = ids[1];
            repeated.ordinal = 2;
            sources.candidates.push(repeated.clone());
            repeated.ordinal = 3;
            sources.candidates.push(repeated);
            if hour % 7 == 3 {
                sources.candidates.clear();
            }
            let formation_id = 810;
            sources.candidates.push(unit_history::naval::Candidate {
                province: ids[4],
                identity: formation_id,
                ordinal: 5,
                slot: 11,
                priority: 3,
                ours: true,
                class: CounterClass::TaskForce,
                land_eligible: false,
                sample_count: 4,
                next_arc: 0.8,
                route: vec![ids[4], ids[5]],
                speed: 0.125,
                min_arc: 0.0,
                arrival_allowed: true,
                day: hour as i32,
            });
            let roster = (0..5u64)
                .map(|index| unit_history::naval::Hull {
                    flags: if index == 0 { 0x400040 } else { 1 << 8 },
                    graphics: format!("synthetic_{}", (index + u64::from(hour)) % 3).into_bytes(),
                    targets: std::array::from_fn(|part| {
                        Some(Target {
                            identity: formation_id,
                            occupant: 900 + index,
                            part: part as u32,
                            mesh: (index as usize + hour as usize) % 6,
                            instance: geometry(index as f32 * 5.0 + part as f32),
                            ..unit_target(ids[4], Kind::Ship)
                        })
                    }),
                })
                .collect();
            sources.formations.push(unit_history::naval::Formation {
                identity: formation_id,
                province: ids[4],
                slot: 11,
                roster,
            });
            let mut land_candidate = sources.candidates.last().unwrap().clone();
            land_candidate.identity = 1_900;
            land_candidate.province = ids[191];
            land_candidate.slot = 3;
            land_candidate.class = CounterClass::Division;
            land_candidate.land_eligible = hour % 2 == 0;
            sources.candidates.push(land_candidate);
            let targets: unit_history::Targets = targets.into();
            let carried: BTreeMap<_, _> = history
                .provinces
                .iter()
                .map(|(&id, province)| (id, province_frame_state(province)))
                .collect();
            let installed_bounds = if hour % 4 == 1 {
                Arc::default()
            } else {
                Arc::clone(&bounds)
            };
            if !installed_bounds.is_empty() {
                expected_bounds = Arc::clone(&installed_bounds);
            }
            expected_ids.extend(installed_bounds.keys().copied());
            expected_ids.extend(targets.0.provinces.keys().copied());
            expected_ids.extend(
                sources
                    .candidates
                    .iter()
                    .map(|candidate| candidate.province),
            );
            if hour % 3 == 0 {
                history.set_naval(sources.clone());
                history.set_prepared(targets.clone(), installed_bounds, false);
            } else if hour % 3 == 1 {
                history.set_prepared(targets.clone(), installed_bounds, false);
                history.set_naval(sources.clone());
            } else {
                history.set_prepared_with_naval(
                    targets.clone(),
                    installed_bounds,
                    false,
                    sources.clone(),
                );
            }
            assert_installed_history(&history, &expected_ids);
            if hour % 5 == 2 {
                let mut only_naval = sources.clone();
                let mut candidate = only_naval.candidates.last().unwrap().clone();
                candidate.province = last_id - 10_000 - hour;
                candidate.ordinal += 100;
                expected_ids.insert(candidate.province);
                only_naval.candidates.push(candidate);
                history.set_naval(only_naval);
                assert_installed_history(&history, &expected_ids);
            }
            if hour % 7 == 3 {
                history.set_prepared(Default::default(), Arc::default(), false);
                assert_installed_history(&history, &expected_ids);
            }
            assert!(Arc::ptr_eq(&history.bounds, &expected_bounds));
            if hour % 11 == 4 {
                history.set_naval(Default::default());
                assert_installed_history(&history, &expected_ids);
                expected_ids.extend(bounds.keys().copied());
                expected_bounds = Arc::clone(&bounds);
                history.set_prepared(targets.clone(), Arc::clone(&bounds), false);
                assert_installed_history(&history, &expected_ids);
            }
            assert!(Arc::ptr_eq(&history.bounds, &expected_bounds));
            for (&id, state) in &carried {
                assert_eq!(
                    *state,
                    province_frame_state(&history.provinces[&id]),
                    "hour {hour} province {id}: install changed carried state"
                );
            }
            history.set_raids(vec![(0, geometry(hour as f32))]);
            assert_installed_history(&history, &expected_ids);
        }
        if number == 617 {
            time = 0.0;
        }
        time += [0.0, 1.0 / 240.0, 0.125, 0.5][(roll as usize >> 3) % 4];
        let camera = Placement::map(
            [
                (number % 20) as f32 * 240.0,
                ((number / 13) % 10) as f32 * 120.0,
            ],
            [50.0, 150.0, 260.0, 350.0][number as usize / 37 % 4],
        )
        .on(MapSize::TEST, MapDefines::test());
        let viewport = if roll & 1 == 0 {
            [1024, 768]
        } else {
            [1600, 900]
        };
        let admission = Admission::new(camera, viewport);
        let steady = number.is_multiple_of(79);
        let mut gates = GATES;
        gates.scale_up = [2.0, 0.0, -0.5, 0.125][number as usize / 43 % 4];
        gates.scale_down = [1.0, 0.0, -0.25][number as usize / 47 % 3];
        let shown = [gates.units(camera.height), gates.ships(camera.height)];
        let limit = [-11, 0, 1, 2, 7, 91][roll as usize % 6];
        history.steady = steady;
        if number.is_multiple_of(173) {
            history.reset(Some(&BTreeSet::from([ids[number as usize % ids.len()]])));
        }
        if number.is_multiple_of(313) {
            history.restart_live();
            history.steady = steady;
        }
        history.select_provinces(if number % 11 < 5 {
            BTreeSet::from([ids[0], ids[1], ids[191]])
        } else {
            BTreeSet::new()
        });
        history.set_selection(&BTreeSet::from([Unit::TaskForce(if number % 5 < 3 {
            7
        } else {
            810
        })]));
        if number.is_multiple_of(61) {
            history.schedule.lock().unwrap().advance(
                ids.iter().copied().chain([last_id - 1]),
                |_| true,
                steady,
            );
        } else if number.is_multiple_of(3) {
            history.advance_schedule(camera, viewport);
        }
        let mut budget = Budget::new(limit, steady);
        let mut trace = Vec::new();
        let output = history.step_with(
            time,
            gates,
            shown,
            camera.height < gates.arrows,
            |id| admission.admits_province(&bounds, id),
            &mut budget,
            |id, budget| {
                if (id ^ roll).is_multiple_of(7) {
                    budget.take();
                }
                trace.push((id, budget.remaining()));
            },
        );
        trace.push((u32::MAX, budget.remaining()));
        let expected_trace: Vec<_> = history
            .updates
            .iter()
            .map(|&(id, _)| id)
            .filter(|id| expected_ids.contains(id) && !history.left_alone.contains(id))
            .chain([u32::MAX, u32::MAX])
            .collect();
        assert_eq!(
            trace.iter().map(|&(id, _)| id).collect::<Vec<_>>(),
            expected_trace,
            "frame {number}: the buildings are reached at every province the walk visits, in its order"
        );
        assert_installed_history(&history, &expected_ids);
        let mut full: Vec<_> = history
            .provinces
            .iter()
            .flat_map(|(_, province)| {
                province
                    .entities
                    .values()
                    .flat_map(unit_history::Entity::placed)
            })
            .collect();
        full.extend(history.combats.placed());
        full.extend(history.raids.iter().map(|&(mesh, instance)| {
            (
                mesh,
                ModelInstance {
                    animation_epoch: history.raid_epoch,
                    ..instance
                },
            )
        }));
        unit_history::same_history::assert_instances(&output, &full);
        let dense = &history.provinces;
        let bound = 18 * (dense.records.len() + 1) + 36;
        let slots = history.schedule.lock().unwrap().storage_slots();
        assert!(
            slots <= bound,
            "frame {number}: schedule storage {slots} exceeded bound {bound}"
        );
        saw[0] |= !output.is_empty();
        saw[1] |= history
            .updates
            .iter()
            .any(|(_, update)| *update == unit_history::Update::OffScreen);
        saw[2] |= !history.queue_trace.is_empty();
        saw[3] |= !history.combats.entities.is_empty();
        saw[4] |= dense.records.iter().any(|record| record.id == last_id);
    }
    assert!(
        saw.into_iter().all(|saw| saw),
        "driver must exercise all lifetime paths"
    );
}
