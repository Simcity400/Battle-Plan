use super::*;
use std::collections::BTreeSet;

#[test]
fn weather_parent_acceptance_is_a_fresh_ordered_prefix_with_plane_contact() {
    let outside = [[0., 0., 0., 2.]; 6];
    let contact = [[0., 0., 0., 1.]; 6];
    let views = vec![([0.; 3], outside), ([0.; 3], contact), ([0.; 3], outside)];
    assert_eq!(
        parent_admission([0.; 3], 1., &views),
        native::ViewMask::Some(vec![false, true, true])
    );
    //The previous call does not leave a stamp in a later frame.
    assert!(parent_admission([0.; 3], 1., &[([0.; 3], outside); 3]).is_empty());
    assert!(parent_admission([0.; 3], -1., &[([0.; 3], [[0.; 4]; 6])]).is_empty());
    assert!(parent_admission([f32::MAX, 0., 0.], 1., &[([0.; 3], contact)]).is_empty());
    // A free camera's frame can hold more views than eight: the mask has one per view.
    let mut many = vec![([0.; 3], outside); 8];
    many.extend([([0.; 3], contact), ([0.; 3], outside)]);
    let mask = parent_admission([0.; 3], 1., &many);
    assert!((0..8).all(|view| !mask.contains(view)));
    assert!(mask.contains(8) && mask.contains(9));
}
use viewer_foundation::map::presentation::Continuity;

#[test]
fn native_tiles_consume_two_hashes_and_survive_particle_motion() {
    let mut seed = 0x1234_5678_0000_0001;
    assert_eq!(sample_tile(&mut seed, [257, 521]), [108, 110, 0, 0]);
    assert_eq!(seed, 0x1234_5678_0000_0003);
    let mut seed = 0x1234_5678_0000_0001;
    assert_eq!(sample_tile(&mut seed, [-257, 521]), [108, 110, 0, 0]);
    let mut emitter = activation_test_emitter();
    emitter.render.tiles = [7, 5];
    let mut sim = Simulation::cold(&emitter, 12345, [0.; 3], 0.);
    sim.tick(&emitter, 1. / 60.);
    assert_eq!(sim.particles.len(), 1);
    let tile = sim.particles[0].tile;
    emitter.emission = 0.;
    sim.tick(&emitter, 0.25);
    assert_eq!(sim.particles[0].tile, tile);
}

#[test]
fn descriptor_selection_preserves_small_fallback_and_native_draw_count() {
    let empty_pair = || WeatherPair {
        members: [Vec::new(), Vec::new()],
        radii: [None; 2],
    };
    let systems = BTreeMap::from([
        ((0, false), vec![empty_pair(), empty_pair(), empty_pair()]),
        ((0, true), vec![empty_pair()]),
        ((4, false), vec![empty_pair()]),
    ]);
    let mut seed = 0x1234_5678_0000_0001;
    let mut expected = seed;
    let index = random_bits(&mut expected) as usize % 3;
    assert!(std::ptr::eq(
        select_pair(&systems, 0, false, &mut seed).unwrap(),
        &systems[&(0, false)][index]
    ));
    assert_eq!(seed, expected);
    assert!(std::ptr::eq(
        select_pair(&systems, 0, true, &mut seed).unwrap(),
        &systems[&(0, true)][0]
    ));
    assert!(std::ptr::eq(
        select_pair(&systems, 4, true, &mut seed).unwrap(),
        &systems[&(4, false)][0]
    ));
    let before = seed;
    assert!(select_pair(&systems, 2, true, &mut seed).is_none());
    assert_eq!(seed, before, "an absent bank consumes no random draw");
    let only_small = BTreeMap::from([((0, true), vec![empty_pair()])]);
    assert!(select_pair(&only_small, 0, false, &mut seed).is_none());
}

#[test]
fn native_curve_bake_smooths_knots_and_reads_sixty_four_intervals() {
    let ramp = Curve::new(&[[0.0, 0.0], [1.0, 1.0]], 0.0, 1.0, 0.0, 1.0);
    assert!((ramp.sample(0.5) - 32.0 / 63.0).abs() < 1e-6);
    let plateau = Curve::new(
        &[[0.0, 0.0], [0.25, 1.0], [0.75, 1.0], [1.0, 0.0]],
        0.0,
        1.0,
        0.0,
        1.0,
    );
    assert!(plateau.sample(0.5) > 1.12);
}

#[test]
fn native_emitter_basis_preserves_rain_direction_and_rotates_spawn_plane() {
    let local = [
        30.0_f32.to_radians().cos(),
        -1.0,
        30.0_f32.to_radians().sin(),
    ];
    let v = transform(emitter_basis(0.0, 0.0), local);
    assert!((v[0] - 0.5).abs() < 1e-6);
    assert!((v[1] + 1.0).abs() < 1e-6);
    assert!((v[2] + 0.8660254).abs() < 1e-6);
    let v = transform(emitter_basis(0.0, 90.0), [1.0, 0.0, 0.0]);
    assert!((v[1] - 1.0).abs() < 1e-6);
}

fn reference_defines() -> CameraDefines {
    CameraDefines {
        distance_cutoff: 1500.0,
        distance_fade_length: 400.0,
        zoom_in_cutoff: 358.0,
        zoom_in_fade_length: 220.0,
        zoom_in_fade_factor: 0.0,
        playback_rate: 0.15,
        playback_rate_cutoff: 500.0,
        playback_rate_length: 200.0,
    }
}

fn gates(height: f32) -> (f32, f32, f32) {
    super::gates(height, &reference_defines())
}

#[test]
fn merging_keeps_retained_runtimes_and_retires_the_rest() {
    // Anchors 1 and 2 sit in kept regions, 3 and 4 in changed ones.
    let kept = |anchor: &u32| *anchor <= 2;
    let mut current = BTreeMap::from([(1, "old 1"), (3, "old 3"), (4, "old 4")]);
    let prepared = BTreeMap::from([(1, "new 1"), (2, "new 2"), (3, "new 3"), (5, "new 5")]);
    let mut retired = merge_runtimes(&mut current, prepared, kept);
    retired.sort_unstable();
    // Anchor 2's region was kept but had no runtime; its prepared one is
    // let go too, since a kept region continues exactly as it was.
    assert_eq!(
        current,
        BTreeMap::from([(1, "old 1"), (3, "new 3"), (5, "new 5")])
    );
    assert_eq!(retired, ["new 1", "new 2", "old 3", "old 4"]);
}

#[test]
fn viewer_history_updates_only_changed_regions() {
    let first = Field {
        branch: 1,
        hour: 20,
        kinds: BTreeMap::from([(1, 0), (2, 2)]),
    };
    let mut presentation = Presentation::default();
    presentation.transition(&Field::default(), &first, Continuity::Continue);
    let next = Field {
        branch: 1,
        hour: 21,
        kinds: BTreeMap::from([(1, 0), (2, 3)]),
    };
    let retained = presentation.transition(&first, &next, Continuity::Continue);
    assert_eq!(retained, BTreeSet::from([1]));
    assert_eq!(presentation.0[&1].seed_hour, 20);
    assert_eq!(presentation.0[&2].seed_hour, 21);
    let seek = Field {
        hour: 20,
        kinds: first.kinds.clone(),
        ..first
    };
    assert!(
        presentation
            .transition(&next, &seek, Continuity::Continue)
            .is_empty()
    );
    let branch = Field {
        branch: 2,
        hour: 21,
        kinds: seek.kinds.clone(),
    };
    assert!(
        presentation
            .transition(&seek, &branch, Continuity::Continue)
            .is_empty()
    );
}

#[test]
fn playback_skips_keep_live_rain_and_snow_runtimes_advancing() {
    let first = Field {
        branch: 1,
        hour: 20,
        kinds: BTreeMap::from([(1, 0), (2, 2)]),
    };
    let next = Field {
        hour: 200,
        kinds: first.kinds.clone(),
        ..first
    };
    let mut presentation = Presentation::default();
    presentation.transition(&Field::default(), &first, Continuity::Reset);
    let emitter = activation_test_emitter();
    let cold = |seed| Simulation::cold(&emitter, seed, [10.0, 0.0, 20.0], 0.0);
    let mut current = BTreeMap::from([(1, cold(12345)), (2, cold(54321))]);
    let mut uninterrupted = BTreeMap::from([(1, cold(12345)), (2, cold(54321))]);
    for simulation in current.values_mut().chain(uninterrupted.values_mut()) {
        simulation.advance(&emitter, 0.2, 1.0);
        assert!(!simulation.particles.is_empty());
    }
    let retained = presentation.transition(&first, &next, Continuity::Continue);
    assert_eq!(retained, BTreeSet::from([1, 2]));
    assert_eq!(presentation.0[&1].seed_hour, first.hour);
    assert_eq!(presentation.0[&2].seed_hour, first.hour);
    let retired = merge_runtimes(
        &mut current,
        BTreeMap::from([(1, cold(999)), (2, cold(888))]),
        |region| retained.contains(region),
    );
    assert!(
        retired
            .iter()
            .all(|simulation| simulation.particles.is_empty())
    );
    for region in [1, 2] {
        let simulation = current.get_mut(&region).unwrap();
        let expected = uninterrupted.get_mut(&region).unwrap();
        assert_same_state(simulation, expected);
        simulation.advance(&emitter, 0.4, 1.0);
        expected.advance(&emitter, 0.4, 1.0);
        assert_same_state(simulation, expected);
        assert!(simulation.age > 0.2);
    }
    // An explicit same-hour seek (also used for gaps/country changes)
    // replaces populated runtimes, even though no weather kind changed.
    let retained = presentation.transition(&next, &next, Continuity::Reset);
    assert!(retained.is_empty());
    let retired = merge_runtimes(
        &mut current,
        BTreeMap::from([(1, cold(999)), (2, cold(888))]),
        |region| retained.contains(region),
    );
    assert!(
        current
            .values()
            .all(|simulation| simulation.particles.is_empty())
    );
    assert!(
        retired
            .iter()
            .all(|simulation| !simulation.particles.is_empty())
    );
    assert_eq!(presentation.0[&1].seed_hour, next.hour);
}

#[test]
fn changed_content_controls_both_camera_bands_and_the_playback_floor() {
    let defines = CameraDefines {
        distance_cutoff: 2000.0,
        distance_fade_length: 600.0,
        zoom_in_cutoff: 100.0,
        zoom_in_fade_length: 200.0,
        zoom_in_fade_factor: 0.2,
        playback_rate: 0.4,
        playback_rate_cutoff: 800.0,
        playback_rate_length: 400.0,
    };
    assert_eq!(super::gates(50.0, &defines), (0.2, 0.0, 1.0));
    assert!((super::gates(200.0, &defines).0 - 0.6).abs() < 1e-6);
    assert_eq!(super::gates(300.0, &defines).0, 1.0);
    assert!((super::gates(600.0, &defines).2 - 0.7).abs() < 1e-6);
    assert_eq!(super::gates(1400.0, &defines), (1.0, 0.0, 0.4));
    assert_eq!(super::gates(1700.0, &defines), (0.5, 0.5, 0.4));
    assert_eq!(super::gates(2000.0, &defines), (0.0, 0.0, 0.4));
}

#[test]
fn the_entity_fades_with_zoom_and_the_fading_entity_draws_only_far() {
    // `WEATHER_ZOOM_IN_FADE_FACTOR` is 0, so nothing draws below the cutoff.
    assert_eq!(gates(50.0).0, 0.0);
    assert_eq!(gates(310.131).0, 0.0);
    assert_eq!(gates(357.999).0, 0.0);
    // The entity's zoom-in fade runs from 358 to 578.
    assert_eq!(gates(358.0).0, 0.0);
    assert!((gates(431.4655).0 - 0.333_934).abs() < 1e-5);
    assert!((gates(468.0).0 - 0.5).abs() < 1e-6);
    assert_eq!(gates(597.5275).0, 1.0);
    assert_eq!(gates(1100.0).0, 1.0);
    // The far fade takes both from 1100 to 1500, and nothing draws above it.
    assert_eq!(gates(1300.0).0, 0.5);
    assert!((gates(1167.101).0 - 0.832_25).abs() < 1e-5);
    assert_eq!(gates(1500.0).0, 0.0);
    assert_eq!(gates(1600.0).0, 0.0);
    // The fading entity draws only in the far band, at the same alpha.
    for h in [50.0, 310.131, 358.0, 431.4655, 597.5275, 1100.0] {
        assert_eq!(gates(h).1, 0.0, "companion at {h}");
    }
    for h in [1167.101, 1300.0] {
        assert_eq!(gates(h).1, gates(h).0, "companion at {h}");
    }
}

#[test]
fn the_playback_rate_holds_at_one_below_300_and_at_its_floor_above_500() {
    assert_eq!(gates(50.0).2, 1.0);
    assert_eq!(gates(300.0).2, 1.0);
    assert!((gates(400.0).2 - 0.575).abs() < 1e-6);
    assert_eq!(gates(500.0).2, 0.15);
    assert_eq!(gates(1500.0).2, 0.15);
}

#[test]
fn zoom_changes_playback_rate_without_teleporting_particle_ages() {
    let mut clock = Clock::default();
    let defines = reference_defines();
    assert_eq!(clock.advance(20.0, 50.0, &defines), 0.0);
    assert_eq!(clock.advance(21.0, 50.0, &defines), 1.0);
    assert_eq!(clock.advance(21.0, 600.0, &defines), 1.0);
    assert!((clock.advance(22.0, 600.0, &defines) - 1.15).abs() < 1e-6);
    assert_eq!(clock.advance(0.0, 50.0, &defines), 0.0);
}

#[test]
fn scalar_and_block_curve_parameters_both_preserve_the_reference() {
    let doc = parser::parse(br#"alpha=60,fade size={ 6,fade 1 }"#).unwrap();
    let curves = BTreeMap::from([(
        "fade".to_owned(),
        Curve::new(&[[0.0, 0.0], [0.5, 1.0], [1.0, 0.0]], 0.0, 1.0, 0.0, 1.0),
    )]);
    let alpha = Parameter::read(&doc.items, "alpha", 255.0, &curves);
    let size = Parameter::read(&doc.items, "size", 0.0, &curves);
    assert!((alpha.sample(0.5, 0.5) - 60.0).abs() < 0.1);
    assert_eq!(alpha.sample(0.5, 1.0), 0.0);
    assert!((size.sample(1.0, 0.5) - 7.0).abs() < 0.01);
    assert!((size.sample(0.0, 0.5) - 5.0).abs() < 0.01);
}

/// [`activation_test_emitter`] with particles that live ten seconds.
pub(crate) fn long_lived_test_emitter() -> Emitter {
    let mut emitter = activation_test_emitter();
    emitter.life.base = 10.;
    emitter
}

pub(crate) fn activation_test_emitter() -> Emitter {
    let scalar = |base: f32| Parameter {
        base,
        spread: 0.0,
        curve: None,
    };
    Emitter {
        scale: 1.0,
        page: 0,
        registration: 0,
        actor: 0,
        radius: scalar(0.0),
        sphere_pitch: scalar(0.0),
        sphere_yaw: scalar(0.0),
        emitter_yaw: scalar(0.0),
        emitter_pitch: scalar(0.0),
        position: [0.0, 0.0, 0.0],
        velocity: scalar(10.0),
        pitch: scalar(0.0),
        yaw: scalar(0.0),
        life: scalar(3.0),
        mass: scalar(1.0),
        start: scalar(0.0),
        duration: -1.0,
        size: scalar(5.0),
        rotation: [scalar(0.0), scalar(0.0), scalar(0.0)],
        spin: [scalar(0.0), scalar(0.0), scalar(0.0)],
        color: [scalar(200.0), scalar(200.0), scalar(200.0), scalar(255.0)],
        emission: 60.0,
        max: 100,
        acceleration: [0.0, -9.0, 0.0],
        turbulence: 0.0,
        noise_division: 1,
        local_space: false,
        render: installed_content::particles::RenderDescriptor::read(&[]).unwrap(),
        system_local_space: false,
    }
}

fn assert_same_state(left: &Simulation, right: &Simulation) {
    assert_eq!(left.prepared, right.prepared);
    assert_eq!(left.particles.len(), right.particles.len());
    assert_eq!(left.age, right.age);
    assert_eq!(left.accumulator, right.accumulator);
    assert_eq!(left.time, right.time);
    assert_eq!(left.seed, right.seed);
    assert_eq!(left.anchor, right.anchor);
    for (left, right) in left.particles.iter().zip(right.particles.iter()) {
        assert_eq!(left.age, right.age);
        assert_eq!(left.life, right.life);
        assert_eq!(left.mass, right.mass);
        assert_eq!(left.position, right.position);
        assert_eq!(left.velocity, right.velocity);
        assert_eq!(left.size, right.size);
        assert_eq!(left.rotation, right.rotation);
        assert_eq!(left.spin, right.spin);
        assert_eq!(left.color, right.color);
        assert_eq!(left.tile, right.tile);
    }
}

#[test]
fn a_finished_entity_emitter_stops_births_and_keeps_live_particles() {
    let mut emitter = activation_test_emitter();
    emitter.life.base = 2.;
    emitter.life.spread = 0.;
    let mut sim = Simulation::cold(&emitter, 7, [0.; 3], 0.);
    sim.advance_event(&emitter, 0.1, None, None);
    assert!(!sim.particles.is_empty());
    sim.advance_event(&emitter, 0.3, Some(0.2), None);
    let count = sim.particles.len();
    assert!(count > 0 && !sim.emitting);
    sim.advance_event(&emitter, 0.5, Some(0.2), None);
    assert_eq!(sim.particles.len(), count);
    sim.advance_event(&emitter, 3., Some(0.2), None);
    assert!(sim.particles.is_empty());
}

#[test]
fn fresh_emitter_accumulates_births_only_after_frame_updates() {
    let emitter = activation_test_emitter();
    let mut simulation = Simulation::cold(&emitter, 12345, [10.0, 0.0, 20.0], 0.0);
    assert!(simulation.particles.is_empty());
    simulation.advance(&emitter, 0.0, 1.0);
    assert!(simulation.particles.is_empty());
    simulation.advance(&emitter, 1.0 / 60.0, 1.0);
    assert_eq!(simulation.particles.len(), 1);
    simulation.advance(&emitter, 2.0 / 60.0, 1.0);
    assert_eq!(simulation.particles.len(), 2);
}

#[test]
fn native_births_interpolate_motion_without_retransforming_survivors() {
    use installed_content::mesh::Transform;
    let mut emitter = activation_test_emitter();
    emitter.emission = 2.;
    emitter.velocity.base = 0.;
    emitter.acceleration = [0.; 3];
    emitter.position = [1., 2., 3.];
    emitter.system_local_space = true;
    let world = Transform::from_trs([10., 0., 0.], [0., 0., 0., 1.], 2.);
    let mut sim = Simulation::cold(&emitter, 7, world.translation, 0.);
    sim.tick_space(
        &emitter,
        1.,
        Some(motion::Space::new(false, true, 3., world)),
    );
    assert_eq!(
        sim.particles.iter().map(|p| p.position).collect::<Vec<_>>(),
        [[11., 2., 3.]; 2]
    );
    let moved = Transform {
        translation: [20., 0., 0.],
        ..world
    };
    sim.tick_space(
        &emitter,
        1.,
        Some(motion::Space::new(false, true, 3., moved)),
    );
    assert_eq!(
        sim.particles.iter().map(|p| p.position).collect::<Vec<_>>(),
        [[11., 2., 3.], [11., 2., 3.], [11., 2., 3.], [16., 2., 3.]]
    );
    assert_eq!(sim.previous_translation, Some([20., 0., 0.]));
}

#[test]
fn expiry_preserves_native_packet_order_and_ages_each_survivor_once() {
    for (trail, expected) in [(false, vec![5, 1, 3]), (true, vec![1, 3, 5])] {
        let mut emitter = activation_test_emitter();
        emitter.emission = 6.;
        emitter.acceleration = [0.; 3];
        emitter.render.trail = trail;
        let mut simulation = Simulation::cold(&emitter, 12345, [0.; 3], 0.);
        simulation.tick(&emitter, 1.);
        assert_eq!(simulation.particles.len(), 6);
        for (index, (particle, (age, life))) in simulation
            .particles
            .iter_mut()
            .zip([
                (2.5, 2.),
                (0.5, 1.),
                (-1., 2.),
                (0.25, 2.),
                (2., 2.),
                (0., 3.),
            ])
            .enumerate()
        {
            particle.tile[0] = index as u8;
            particle.age = age;
            particle.life = life;
            particle.velocity = [index as f32, 0., 0.];
            particle.position = [0.; 3];
        }
        emitter.emission = 0.;
        simulation.tick(&emitter, 0.5);
        assert_eq!(
            simulation
                .particles
                .iter()
                .map(|p| p.tile[0])
                .collect::<Vec<_>>(),
            expected
        );
        for particle in &simulation.particles {
            assert_eq!(
                particle.position,
                [f32::from(particle.tile[0]) * 0.5, 0., 0.]
            );
            assert_eq!(
                particle.age,
                match particle.tile[0] {
                    1 => 1.,
                    3 => 0.75,
                    5 => 0.5,
                    _ => unreachable!(),
                }
            );
        }
        //New births append after native compaction, without aging this tick.
        emitter.emission = 2.;
        simulation.tick(&emitter, 0.5);
        assert_eq!(simulation.particles.last().unwrap().age, 0.);
    }
}

#[test]
fn expiry_keeps_unordered_age_and_lifetime_like_native_comiss() {
    let mut emitter = activation_test_emitter();
    emitter.emission = 2.;
    let mut simulation = Simulation::cold(&emitter, 12345, [0.; 3], 0.);
    simulation.tick(&emitter, 1.);
    simulation.particles[0].age = f32::NAN;
    simulation.particles[1].life = f32::NAN;
    emitter.emission = 0.;
    simulation.tick(&emitter, 0.5);
    assert_eq!(simulation.particles.len(), 2);
    assert!(simulation.particles[0].age.is_nan());
    assert!(simulation.particles[1].life.is_nan());
}

#[test]
fn prepared_emitter_starts_at_the_first_render_clock() {
    let emitter = activation_test_emitter();
    let anchor = [10.0, 0.0, 20.0];
    let mut prepared = Simulation::cold(&emitter, 12345, anchor, 0.0);
    prepared.prepared = true;
    let mut direct = Simulation::cold(&emitter, 12345, anchor, 8.0);
    prepared.advance(&emitter, 8.0, 1.0);
    assert_same_state(&prepared, &direct);
    prepared.advance(&emitter, 8.25, 1.0);
    direct.advance(&emitter, 8.25, 1.0);
    assert_same_state(&prepared, &direct);
    assert!(!prepared.particles.is_empty());
}

#[test]
fn suspension_retains_population_instead_of_inventing_steady_state() {
    let emitter = activation_test_emitter();
    let mut simulation = Simulation::cold(&emitter, 12345, [10.0, 0.0, 20.0], 0.0);
    simulation.advance(&emitter, 0.2, 1.0);
    let count = simulation.particles.len();
    let age = simulation.age;
    assert!(count > 0);
    simulation.advance(&emitter, 20.0, 1.0);
    assert_eq!(simulation.particles.len(), count);
    assert_eq!(simulation.age, age);
    assert_eq!(simulation.time, 20.0);
}

/// Simulations move in and out of the pass's map on every hour change, so
/// the large noise table stays behind a box.
#[test]
fn a_simulation_moves_without_its_noise_table() {
    assert!(std::mem::size_of::<Simulation>() < 256);
    assert!(std::mem::size_of::<Noise>() > 8 * 1024);
}
