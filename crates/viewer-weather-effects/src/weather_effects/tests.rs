use super::test_support::{activation_test_emitter, long_lived_test_emitter, test_force};
use super::*;
use std::collections::BTreeSet;

#[test]
fn weather_parent_acceptance_is_a_fresh_ordered_prefix_with_plane_contact() {
    let outside = [[0., 0., 0., 2.]; 6];
    let contact = [[0., 0., 0., 1.]; 6];
    let views = vec![([0.; 3], outside), ([0.; 3], contact), ([0.; 3], outside)];
    assert_eq!(
        parent_admission([0.; 3], 1., &views),
        Some(native::ViewMask::Some(vec![false, true, true]))
    );
    assert!(parent_admission([0.; 3], 1., &[([0.; 3], outside); 3]).is_none());
    assert!(parent_admission([0.; 3], -1., &[([0.; 3], [[0.; 4]; 6])]).is_none());
    assert!(parent_admission([f32::MAX, 0., 0.], 1., &[([0.; 3], contact)]).is_none());
    let mut many = vec![([0.; 3], outside); 8];
    many.extend([([0.; 3], contact), ([0.; 3], outside)]);
    let mask = parent_admission([0.; 3], 1., &many).expect("a view accepts it");
    assert!((0..8).all(|view| !mask.contains(view)));
    assert!(mask.contains(8) && mask.contains(9));
}
use viewer_runtime::presentation::Continuity;

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
    emitter.emission.base = 0.;
    sim.tick(&emitter, 0.25);
    assert_eq!(sim.particles[0].tile, tile);
}

#[test]
fn descriptor_selection_preserves_small_fallback_and_native_draw_count() {
    let empty_pair = || WeatherPair {
        members: [Vec::new(), Vec::new()],
        radii: [None; 2],
        sounds: [Vec::new(), Vec::new()],
    };
    let systems = BTreeMap::from([
        ((0, false), vec![empty_pair(), empty_pair(), empty_pair()]),
        ((0, true), vec![empty_pair()]),
        ((4, false), vec![empty_pair()]),
    ]);
    let mut seed = 0x1234_5678_0000_0001;
    let mut expected = seed;
    let index = random_bits(&mut expected) as usize % 3;
    assert_eq!(
        select_pair_at(&systems, 0, false, &mut seed),
        Some(((0, false), index))
    );
    assert_eq!(seed, expected);
    assert_eq!(
        select_pair_at(&systems, 0, true, &mut seed),
        Some(((0, true), 0))
    );
    assert_eq!(
        select_pair_at(&systems, 4, true, &mut seed),
        Some(((4, false), 0))
    );
    let before = seed;
    assert!(select_pair_at(&systems, 2, true, &mut seed).is_none());
    assert_eq!(seed, before, "an absent bank consumes no random draw");
    let only_small = BTreeMap::from([((0, true), vec![empty_pair()])]);
    assert!(select_pair_at(&only_small, 0, false, &mut seed).is_none());
}

#[test]
fn shown_weather_entities_present_their_sound_events_while_their_region_keeps_its_weather() {
    let sound = |effect: &str, time: f32| content_reader_weather::weather::WeatherSound {
        effect: effect.into(),
        time,
    };
    let pair = WeatherPair {
        members: [Vec::new(), Vec::new()],
        radii: [Some(1.0), Some(1.0)],
        sounds: [
            vec![sound("rain_loop", 0.0), sound("thunder", 5.0)],
            vec![sound("far_wind", 0.0)],
        ],
    };
    let systems = BTreeMap::from([((0, false), vec![pair])]);
    let anchors = [Anchor {
        region: 7,
        position: [10.0, 5.0, 20.0],
        small: false,
    }];
    let field = Field {
        branch: 1,
        hour: 20,
        kinds: BTreeMap::from([(7, 0)]).into(),
    };
    let mut presentation = Presentation::default();
    presentation.transition(&Field::default(), &field, Continuity::Continue);
    let graphics = reference_defines();
    let accepting = [(
        [0.0; 3],
        [viewer_gpu::province_admission::OPEN; 6],
    )];
    let mut starts = SoundStarts::new();
    let mut fades = fade::Fades::default();
    let mut drawn = Drawn::default();
    let mut changes = 0;
    let mut collect = |height: f32, time: f32, field: &Field, presentation: &Presentation| {
        let mut names = viewer_models::sound_events::Names::default();
        let mut out = Vec::new();
        fades.produce(presentation.0.keys().copied(), height, &graphics, |_| true);
        let kept = drawn
            .for_field(&anchors, &systems, field, presentation, changes)
            .to_vec();
        changes += 1;
        let again = drawn
            .for_field(&anchors, &systems, field, presentation, changes)
            .to_vec();
        assert_eq!(kept, again, "a kept draw is the draw a walk makes");
        sound_events(
            SoundInputs {
                anchors: &anchors,
                systems: &systems,
                fades: &fades,
                presentation,
                drawn: &kept,
                starts: &mut starts,
                time,
            },
            &accepting,
            &mut names,
            &mut out,
        );
        out
    };
    let effects = |events: &[viewer_models::sound_events::SoundEvent]| {
        events
            .iter()
            .map(|event| (event.effect.to_string(), event.age))
            .collect::<Vec<_>>()
    };
    let named = |pairs: &[(&str, f32)]| {
        pairs
            .iter()
            .map(|&(effect, age)| (effect.to_owned(), age))
            .collect::<Vec<_>>()
    };
    let near = collect(800.0, 10.0, &field, &presentation);
    assert_eq!(effects(&near), named(&[("rain_loop", 0.0), ("far_wind", 0.0)]));
    assert_eq!(near[0].position, [10.0, 5.0, 20.0]);
    let far = collect(1300.0, 16.0, &field, &presentation);
    assert_eq!(
        effects(&far),
        named(&[("rain_loop", 6.0), ("thunder", 1.0), ("far_wind", 6.0)])
    );
    assert_eq!(far[0].key, near[0].key, "the region kept its weather");
    assert_eq!(
        effects(&collect(300.0, 17.0, &field, &presentation)),
        named(&[("rain_loop", 7.0), ("thunder", 2.0)])
    );
    assert!(collect(1600.0, 17.5, &field, &presentation).is_empty());
    assert!(collect(300.0, 17.75, &field, &presentation).is_empty());
    let clear = Field {
        hour: 21,
        kinds: BTreeMap::new().into(),
        ..field
    };
    assert!(collect(800.0, 18.0, &clear, &Presentation::default()).is_empty());
}

#[test]
fn an_unlisted_region_keeps_its_pairs_through_weather_that_comes_back_and_swaps_when_listed() {
    let defines = reference_defines();
    let field = |hour: i32, kinds: &[(u32, usize)]| Field {
        branch: 1,
        hour,
        kinds: kinds.iter().copied().collect::<BTreeMap<_, _>>().into(),
    };
    let region_of = |anchor: usize| if anchor < 2 { 7 } else { 8 };
    let mut wanted = Presentation::default();
    let mut previous = field(10, &[(7, 0), (8, 0)]);
    wanted.transition(&Field::default(), &previous, Continuity::Continue);
    let mut held = Presentation::default();
    let mut fades = fade::Fades::default();
    let mut runtimes = BTreeMap::new();
    let mut pending = BTreeMap::from([
        ((0, false, 0), "rain 0"),
        ((1, true, 0), "rain 1"),
        ((2, false, 0), "rain 2"),
    ]);
    let mut run = |listed: &[u32], wanted: &Presentation, pending: &mut BTreeMap<_, _>| {
        let swap = swap_listed(listed, wanted, &mut held, &mut fades, (&mut runtimes, pending), region_of);
        fades.produce(listed.iter().copied().filter(|region| held.0.contains_key(region)), 1300.0, &defines, |_| true);
        (swap, held.clone(), fades.members(7), runtimes.clone())
    };
    let ((swapped, let_go), first, members, made) = run(&[7, 8], &wanted, &mut pending);
    assert!(swapped && let_go.is_empty() && pending.is_empty());
    assert_eq!(first.0[&7], RegionPresentation { kind: 0, seed_hour: 10 });
    assert_eq!(made.values().copied().collect::<Vec<_>>(), ["rain 0", "rain 1", "rain 2"]);
    assert!(members.iter().all(|member| member.active));
    for (hour, kinds) in [(11, &[(8, 0)][..]), (12, &[(7, 0), (8, 0)][..])] {
        let next = field(hour, kinds);
        wanted.transition(&previous, &next, Continuity::Continue);
        previous = next;
        let ((swapped, let_go), _, _, _) = run(&[], &wanted, &mut BTreeMap::new());
        assert!(!swapped && let_go.is_empty());
    }
    assert_eq!(wanted.0[&7].seed_hour, 12);
    let mut prepared = BTreeMap::from([((0, false, 0), "new rain 0")]);
    let ((swapped, let_go), kept, kept_members, kept_runtimes) = run(&[7], &wanted, &mut prepared);
    assert!(!swapped && let_go.is_empty());
    assert_eq!(kept.0[&7], first.0[&7]);
    assert_eq!(kept_members, members);
    assert_eq!(kept_runtimes, made);
    assert_eq!(prepared.len(), 1);
    let next = field(13, &[(7, 2), (8, 0)]);
    wanted.transition(&previous, &next, Continuity::Continue);
    let mut prepared = BTreeMap::from([((0, false, 0), "snow 0")]);
    let ((swapped, _), unlisted, _, _) = run(&[8], &wanted, &mut prepared);
    assert!(!swapped && unlisted.0[&7] == first.0[&7]);
    let ((swapped, mut let_go), snow, snow_members, snow_runtimes) = run(&[7, 8], &wanted, &mut prepared);
    assert!(swapped);
    assert_eq!(snow.0[&7], RegionPresentation { kind: 2, seed_hour: 13 });
    let_go.sort_unstable();
    assert_eq!(let_go, ["rain 0", "rain 1"]);
    assert_eq!(snow_runtimes.values().copied().collect::<Vec<_>>(), ["snow 0", "rain 2"]);
    assert_eq!(snow_members, fade::created(1300.0, &defines), "the new pairs start fresh");
    let clear = field(14, &[(8, 0)]);
    wanted.transition(&next, &clear, Continuity::Continue);
    let ((swapped, let_go), none, _, rest) = run(&[7], &wanted, &mut BTreeMap::new());
    assert!(swapped && let_go == ["snow 0"] && !none.0.contains_key(&7));
    assert_eq!(rest.values().copied().collect::<Vec<_>>(), ["rain 2"]);
    let off = Presentation::default();
    let held_8 = |held: &Presentation| held.0.get(&8).copied();
    let rain_8 = Some(RegionPresentation { kind: 0, seed_hour: 10 });
    let ((swapped, _), kept, _, kept_runtimes) = run(&[7], &off, &mut BTreeMap::new());
    assert!(!swapped && held_8(&kept) == rain_8 && kept_runtimes.len() == 1);
    let mut storm = BTreeMap::from([((2, true, 0), "storm 2")]);
    let ((swapped, let_go), released, _, rest) = run(&[8], &off, &mut storm);
    assert!(swapped && let_go == ["rain 2", "storm 2"] && held_8(&released).is_none());
    assert!(rest.is_empty() && storm.is_empty());
    let ((swapped, _), unlisted, _, _) = run(&[], &wanted, &mut BTreeMap::new());
    assert!(!swapped && held_8(&unlisted).is_none());
    let ((swapped, _), back, _, _) = run(&[8], &wanted, &mut BTreeMap::new());
    assert!(swapped && held_8(&back) == rain_8);
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
fn particle_life_curves_apply_operation_and_repeat_boundaries() {
    let mut add = Curve::new(&[[0.0, 2.0], [1.0, 2.0]], 0.0, 1.0, 0.0, 1.0);
    add.op = Op::Add;
    add.time = TimeMode::Life;
    let added = Parameter {
        base: 5.0,
        spread: 0.0,
        curve: Some(add),
    };
    assert!((added.animate(5.0, 0.5) - 7.0).abs() < 1e-5);

    let mut abs = Curve::new(&[[0.0, 3.0], [1.0, 3.0]], 0.0, 1.0, 0.0, 1.0);
    abs.op = Op::Abs;
    abs.time = TimeMode::Life;
    let replaced = Parameter {
        base: 5.0,
        spread: 0.0,
        curve: Some(abs),
    };
    assert!((replaced.animate(5.0, 0.5) - 3.0).abs() < 1e-5);

    let mut rep = Curve::new(&[[0.0, 0.0], [1.0, 1.0]], 1.0, 2.0, 0.0, 1.0);
    rep.repeat = true;
    rep.time = TimeMode::Life;
    let repeating = Parameter {
        base: 10.0,
        spread: 0.0,
        curve: Some(rep),
    };
    assert!((repeating.animate(10.0, 3.5) - 0.0).abs() < 1e-5);
}

#[test]
fn particle_life_abs_curve_evaluates_elapsed_seconds_and_malformed_curves_are_refused() {
    let malformed = Curve::parse(
        &parser::parse(b"name = broken op = mul curve = { 0 0 0.5 bad 1 0 }")
            .unwrap()
            .items,
    );
    assert!(malformed.is_err(), "malformed token must be refused");
    let odd = Curve::parse(
        &parser::parse(b"name = odd op = mul curve = { 0 0 1 }")
            .unwrap()
            .items,
    );
    assert!(odd.is_err(), "odd curve count must be refused");
    let nested = Curve::parse(
        &parser::parse(b"name = nested op = mul curve = { 0 0 { bad } 1 1 }")
            .unwrap()
            .items,
    )
    .unwrap_err()
    .to_string();
    assert!(
        nested.contains("nested") && nested.contains("curve"),
        "{nested}"
    );
    let nan_max = Curve::parse(
        &parser::parse(b"name = nan_max op = mul maxValue = NaN curve = { 0 0 1 1 }")
            .unwrap()
            .items,
    )
    .unwrap_err()
    .to_string();
    assert!(
        nan_max.contains("nan_max") && nan_max.contains("maxValue"),
        "{nan_max}"
    );
    let bad_start = Curve::parse(
        &parser::parse(b"name = bad_start op = mul start = nope curve = { 0 0 1 1 }")
            .unwrap()
            .items,
    )
    .unwrap_err()
    .to_string();
    assert!(
        bad_start.contains("bad_start") && bad_start.contains("start"),
        "{bad_start}"
    );
    let upper_time = Curve::parse(
        &parser::parse(
            b"name = upper_time op = \"ADD\" time = \"LIFE_ABS\" start = 0 duration = 1 \
              minValue = 0 maxValue = 1 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    assert_eq!(upper_time.op, Op::Add);
    assert_eq!(upper_time.time, TimeMode::Spawn);

    let life_abs = Curve::parse(
        &parser::parse(
            b"name = abs_ramp start = 0 duration = 1 op = mul time = life_abs \
              minValue = 0 maxValue = 1 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut emitter = activation_test_emitter();
    emitter.size = Parameter {
        base: 1.0,
        spread: 0.0,
        curve: Some(life_abs),
    };
    let mut sim = Simulation::cold(&emitter, 1, [0.0; 3], 0.0);
    let mut particle = moving_particle([0.0; 3]);
    particle.age = 0.5;
    particle.life = 2.0;
    particle.size = 1.0;
    sim.particles.push(particle);
    let packet = native::DrawPacket::weather(&emitter, &sim, [0.0; 3], 1.0);
    let drawn_size = packet.instances[0].position_size[3];
    assert!(
        (drawn_size - 32.0 / 63.0).abs() < 1e-4,
        "expected ~0.5079 for life_abs at age=0.5, got {drawn_size}"
    );
}

#[test]
fn spawn_curves_freeze_birth_values_at_subsystem_age() {
    let ramp = Curve::parse(
        &parser::parse(
            b"name = spawn_ramp start = 0 duration = 1 op = abs time = spawn \
              minValue = 1 maxValue = 5 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut emitter = activation_test_emitter();
    emitter.emission = Parameter {
        base: 2.0,
        spread: 0.0,
        curve: None,
    };
    emitter.size = Parameter {
        base: 1.0,
        spread: 0.0,
        curve: Some(ramp.clone()),
    };
    emitter.life = Parameter {
        base: 1.0,
        spread: 0.0,
        curve: Some(ramp),
    };
    emitter.forces.clear();
    let mut sim = Simulation::cold(&emitter, 7, [0.0; 3], 0.0);
    sim.tick(&emitter, 0.5);
    assert_eq!(sim.particles.len(), 1);
    sim.tick(&emitter, 0.5);
    assert_eq!(sim.particles.len(), 2);

    let expected_second = 1.0 + 4.0 * (32.0 / 63.0);
    let packet = native::DrawPacket::weather(&emitter, &sim, [0.0; 3], 1.0);
    let first_size = packet.instances[0].position_size[3];
    let second_size = packet.instances[1].position_size[3];
    assert!(
        (first_size - 1.0).abs() < 1e-4,
        "first particle must keep its birth size 1.0, got {first_size}"
    );
    assert!(
        (second_size - expected_second).abs() < 1e-3,
        "second particle must take birth size ~{expected_second}, got {second_size}"
    );
    assert!(
        (sim.particles[0].life - 1.0).abs() < 1e-4,
        "first particle must keep birth life 1.0, got {}",
        sim.particles[0].life
    );
    assert!(
        (sim.particles[1].life - expected_second).abs() < 1e-3,
        "second particle must take birth life ~{expected_second}, got {}",
        sim.particles[1].life
    );
}

#[test]
fn non_spawn_life_curve_does_not_extend_lifetime_and_rotation_and_system_clock_match_native() {
    let const_five_life = Curve::parse(
        &parser::parse(
            b"name = five start = 0 duration = 1 op = abs time = life \
              minValue = 0 maxValue = 5 curve = { 0 1  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut life_emitter = activation_test_emitter();
    life_emitter.emission = Parameter {
        base: 2.0,
        spread: 0.0,
        curve: None,
    };
    life_emitter.life = Parameter {
        base: 2.0,
        spread: 0.0,
        curve: Some(const_five_life),
    };
    life_emitter.forces.clear();
    let mut life_sim = Simulation::cold(&life_emitter, 7, [0.0; 3], 0.0);
    life_sim.tick(&life_emitter, 0.5);
    assert_eq!(life_sim.particles.len(), 1);
    life_emitter.emission.base = 0.0;
    for _ in 0..3 {
        life_sim.tick(&life_emitter, 1.0);
    }
    assert!(
        life_sim.particles.is_empty(),
        "non-spawn life curve must not extend 2s base lifetime past 3s"
    );

    let rot_ramp = Curve::parse(
        &parser::parse(
            b"name = rot_ramp start = 0 duration = 1 op = abs time = life_abs \
              minValue = 0 maxValue = 90 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let spin_ramp = Curve::parse(
        &parser::parse(
            b"name = spin_ramp start = 0 duration = 1 op = mul time = life_abs \
              minValue = 0 maxValue = 2 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut rot_emitter = activation_test_emitter();
    rot_emitter.emission = Parameter {
        base: 2.0,
        spread: 0.0,
        curve: None,
    };
    rot_emitter.life = Parameter {
        base: 10.0,
        spread: 0.0,
        curve: None,
    };
    rot_emitter.rotation[2] = Parameter {
        base: 0.0,
        spread: 0.0,
        curve: Some(rot_ramp),
    };
    rot_emitter.spin[0] = Parameter {
        base: 10.0,
        spread: 0.0,
        curve: Some(spin_ramp),
    };
    rot_emitter.forces.clear();
    let mut rot_sim = Simulation::cold(&rot_emitter, 7, [0.0; 3], 0.0);
    rot_sim.tick(&rot_emitter, 0.5);
    assert_eq!(rot_sim.particles.len(), 1);
    let first_rot = native::DrawPacket::weather(&rot_emitter, &rot_sim, [0.0; 3], 1.0).instances[0]
        .rotation;
    assert!(first_rot[2].abs() < 1e-4, "birth rotation should start at 0, got {first_rot:?}");
    rot_sim.tick(&rot_emitter, 0.5);
    assert_eq!(rot_sim.particles.len(), 2);
    let rot_packet = native::DrawPacket::weather(&rot_emitter, &rot_sim, [0.0; 3], 1.0);
    let older_rot = rot_packet.instances[0].rotation;
    let newborn_rot = rot_packet.instances[1].rotation;
    let expected_angle = 90.0 * (32.0 / 63.0);
    assert!(
        (older_rot[2] - expected_angle).abs() < 1e-3,
        "older particle life_abs rotation curve should advance to ~{expected_angle}, got {}",
        older_rot[2]
    );
    assert!(
        newborn_rot[2].abs() < 1e-4,
        "newborn particle life_abs rotation curve should remain at 0, got {}",
        newborn_rot[2]
    );
    let expected_spin_angle = 10.0 * (2.0 * 32.0 / 63.0) * 0.5;
    assert!(
        (older_rot[0] - expected_spin_angle).abs() < 1e-3,
        "older particle should receive first particle spin increment ~{expected_spin_angle}, got {}",
        older_rot[0]
    );
    assert!(
        (newborn_rot[0] - expected_spin_angle).abs() < 1e-3,
        "newborn particle should receive first particle spin increment ~{expected_spin_angle}, got {}",
        newborn_rot[0]
    );

    let sys_size = Curve::parse(
        &parser::parse(
            b"name = sys_size start = 0 duration = 1 op = abs time = system \
              minValue = 1 maxValue = 5 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let sys_red = Curve::parse(
        &parser::parse(
            b"name = sys_red start = 0 duration = 1 op = abs time = system \
              minValue = 50 maxValue = 250 curve = { 0 0  1 1 }",
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let mut sys_emitter = activation_test_emitter();
    sys_emitter.emission = Parameter {
        base: 2.0,
        spread: 0.0,
        curve: None,
    };
    sys_emitter.life = Parameter {
        base: 10.0,
        spread: 0.0,
        curve: None,
    };
    sys_emitter.size = Parameter {
        base: 1.0,
        spread: 0.0,
        curve: Some(sys_size),
    };
    sys_emitter.color[0] = Parameter {
        base: 100.0,
        spread: 0.0,
        curve: Some(sys_red),
    };
    sys_emitter.forces.clear();
    let mut sys_sim = Simulation::cold(&sys_emitter, 7, [0.0; 3], 0.0);
    sys_sim.tick(&sys_emitter, 0.5);
    assert_eq!(sys_sim.particles.len(), 1);
    let first_sys = native::DrawPacket::weather(&sys_emitter, &sys_sim, [0.0; 3], 1.0);
    let first_size = first_sys.instances[0].position_size[3];
    let first_red = first_sys.instances[0].color[0];
    assert!(
        (first_size - 1.0).abs() < 1e-4,
        "first tick at pre-increment age 0.0 must draw size 1.0, got {first_size}"
    );
    assert_eq!(
        first_red, 50,
        "first tick at pre-increment age 0.0 must draw red 50, got {first_red}"
    );
    sys_emitter.emission.base = 0.0;
    sys_sim.tick(&sys_emitter, 0.5);
    let second_sys = native::DrawPacket::weather(&sys_emitter, &sys_sim, [0.0; 3], 1.0);
    let expected_mid_size = 1.0 + 4.0 * (32.0 / 63.0);
    let expected_mid_red = (50.0_f32 + 200.0 * (32.0 / 63.0)).trunc() as u8;
    assert!(
        (second_sys.instances[0].position_size[3] - expected_mid_size).abs() < 1e-3,
        "second tick at pre-increment age 0.5 must draw size ~{expected_mid_size}, got {}",
        second_sys.instances[0].position_size[3]
    );
    assert_eq!(
        second_sys.instances[0].color[0], expected_mid_red,
        "second tick at pre-increment age 0.5 must draw red {expected_mid_red}"
    );

    let mut entity_emitter = sys_emitter.clone();
    entity_emitter.emission.base = 60.0;
    let mut event = viewer_models::order_fixture::events(
        viewer_models::order_fixture::ORDERS[0],
        false,
    )
    .remove(0);
    let name = event.definition.particle.clone().expect("a particle event");
    let entities = BTreeMap::from([(name, (1.0, vec![entity_emitter]))]);
    let mut simulations = EntitySimulations::default();
    event.age = 0.0;
    simulations
        .prepare(&entities, std::slice::from_ref(&event))
        .unwrap();
    event.age = 1.0 / 60.0;
    simulations
        .prepare(&entities, std::slice::from_ref(&event))
        .unwrap();
    let mut packets = Vec::new();
    simulations.packets(&entities, std::slice::from_ref(&event), &mut packets);
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].instances.len(), 1);
    assert!(
        (packets[0].instances[0].position_size[3] - 1.0).abs() < 1e-4,
        "entity packet after first tick must draw size 1.0 at pre-increment age 0.0, got {}",
        packets[0].instances[0].position_size[3]
    );
    assert_eq!(
        packets[0].instances[0].color[0], 50,
        "entity packet after first tick must draw red 50 at pre-increment age 0.0"
    );
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

pub(super) fn reference_defines() -> CameraDefines {
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

fn gates(height: f32) -> (f32, bool, f32) {
    super::gates(height, &reference_defines())
}

#[test]
fn merging_keeps_retained_runtimes_and_retires_the_rest() {
    let kept = |anchor: &u32| *anchor <= 2;
    let mut current = BTreeMap::from([(1, "old 1"), (3, "old 3"), (4, "old 4")]);
    let prepared = BTreeMap::from([(1, "new 1"), (2, "new 2"), (3, "new 3"), (5, "new 5")]);
    let mut retired = merge_runtimes(&mut current, prepared, kept);
    retired.sort_unstable();
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
        kinds: BTreeMap::from([(1, 0), (2, 2)]).into(),
    };
    let mut presentation = Presentation::default();
    presentation.transition(&Field::default(), &first, Continuity::Continue);
    let next = Field {
        branch: 1,
        hour: 21,
        kinds: BTreeMap::from([(1, 0), (2, 3)]).into(),
    };
    let retained = presentation.transition(&first, &next, Continuity::Continue);
    assert_eq!(retained, BTreeSet::from([1]));
    assert_eq!(presentation.0[&1].seed_hour, 20);
    assert_eq!(presentation.0[&2].seed_hour, 21);
    let seek = Field {
        hour: 20,
        kinds: first.kinds.clone().into(),
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
        kinds: seek.kinds.clone().into(),
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
        kinds: BTreeMap::from([(1, 0), (2, 2)]).into(),
    };
    let next = Field {
        hour: 200,
        kinds: first.kinds.clone().into(),
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
    assert_eq!(super::gates(50.0, &defines), (0.2, false, 1.0));
    assert!((super::gates(200.0, &defines).0 - 0.6).abs() < 1e-6);
    assert_eq!(super::gates(300.0, &defines).0, 1.0);
    assert!((super::gates(600.0, &defines).2 - 0.7).abs() < 1e-6);
    assert_eq!(super::gates(1400.0, &defines), (1.0, false, 0.4));
    assert_eq!(super::gates(1700.0, &defines), (0.5, true, 0.4));
    assert_eq!(super::gates(2000.0, &defines), (0.0, true, 0.4));
    assert_eq!(super::gates(2100.0, &defines), (0.0, true, 0.4));
}

#[test]
fn the_camera_fade_runs_from_the_zoom_in_cutoff_and_the_far_band_starts_at_1100() {
    assert_eq!(gates(50.0).0, 0.0);
    assert_eq!(gates(310.131).0, 0.0);
    assert_eq!(gates(357.999).0, 0.0);
    assert_eq!(gates(358.0).0, 0.0);
    assert!((gates(431.4655).0 - 0.333_934).abs() < 1e-5);
    assert!((gates(468.0).0 - 0.5).abs() < 1e-6);
    assert_eq!(gates(597.5275).0, 1.0);
    assert_eq!(gates(1100.0).0, 1.0);
    assert_eq!(gates(1300.0).0, 0.5);
    assert!((gates(1167.101).0 - 0.832_25).abs() < 1e-5);
    assert_eq!(gates(1500.0).0, 0.0);
    assert_eq!(gates(1600.0).0, 0.0);
    for h in [50.0, 310.131, 358.0, 431.4655, 597.5275, 1100.0] {
        assert!(!gates(h).1, "far at {h}");
    }
    for h in [1100.001, 1167.101, 1300.0, 1500.0, 1600.0] {
        assert!(gates(h).1, "near at {h}");
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
    let doc = parser::parse(br#"rotation={ 0.000,0.000 0.000,180.000 }"#).unwrap();
    let rotation = Parameter::read(&doc.items, "rotation", 0.0, &curves);
    assert_eq!((rotation.base, rotation.spread), (0.0, 180.0));
}

fn moving_particle(position: [f32; 3]) -> LiveParticle {
    LiveParticle {
        age: 0.0,
        life: 10.0,
        mass: 1.0,
        position,
        velocity: [0.0; 3],
        size: 1.0,
        rotation: [0.0; 3],
        rotation_base: [0.0; 3],
        spin: [0.0; 3],
        color: [255.0; 4],
        tile: [0; 4],
    }
}

fn assert_near(left: [f32; 3], right: [f32; 3]) {
    assert!(
        left.iter().zip(right).all(|(l, r)| (l - r).abs() < 1e-5),
        "{left:?} != {right:?}"
    );
}

#[test]
fn spin_turns_a_particle_by_its_amount_each_update() {
    let mut emitter = activation_test_emitter();
    emitter.emission.base = 0.;
    emitter.forces = vec![test_force(
        ForceKind::Spin,
        [0.0; 3],
        [0.0, 2.0, 0.0],
        std::f32::consts::FRAC_PI_2,
    )];
    let mut sim = Simulation::cold(&emitter, 7, [0.; 3], 0.);
    sim.particles.push(moving_particle([1.0, 0.0, 0.0]));
    sim.tick(&emitter, 1.0);
    assert_near(sim.particles[0].position, [0.0, 0.0, 1.0]);
    assert_near(sim.particles[0].velocity, [-1.0, 0.0, 1.0]);
    sim.tick(&emitter, 1.0);
    assert_near(sim.particles[0].position, [-1.0, 0.0, 0.0]);
    assert_near(sim.particles[0].velocity, [-1.0, 0.0, -1.0]);
}

#[test]
fn vortex_pushes_around_its_axis_by_its_yaw() {
    let quarter = 90f32 * std::f32::consts::PI / 180.0;
    for (yaw, force) in [
        ([quarter.sin(), quarter.cos()], [2.0 * quarter.cos(), 0.0, 2.0]),
        ([0.0; 2], [2.0, 0.0, 0.0]),
    ] {
        let mut emitter = activation_test_emitter();
        emitter.emission.base = 0.;
        let mut vortex = test_force(ForceKind::Vortex, [0.0; 3], [0.0, 1.0, 0.0], 2.0);
        vortex.yaw = yaw;
        emitter.forces = vec![vortex];
        let mut sim = Simulation::cold(&emitter, 7, [0.; 3], 0.);
        sim.particles.push(moving_particle([2.0, 5.0, 0.0]));
        sim.tick(&emitter, 0.5);
        let velocity = force.map(|f| f * 0.5);
        assert_near(sim.particles[0].velocity, velocity);
        assert_near(
            sim.particles[0].position,
            [2.0 + velocity[0] * 0.5, 5.0, velocity[2] * 0.5],
        );
    }
}

#[test]
fn a_system_draws_its_subsystems_then_its_turbulence_from_one_stream() {
    let mut leader = activation_test_emitter();
    leader.noise_divisions = vec![16];
    leader.position[0].spread = 4.0;
    let mut follower = activation_test_emitter();
    follower.leads = false;
    follower.duration = Parameter {
        base: 3.0,
        spread: 1.0,
        curve: None,
    };
    let mut a = Simulation::cold(&leader, 7, [0.; 3], 0.);
    let mut b = Simulation::cold(&follower, 8, [0.; 3], 0.);
    advance_system(
        &mut [
            Member { sim: &mut a, emitter: &leader, space: None },
            Member { sim: &mut b, emitter: &follower, space: None },
        ],
        0.0,
        1.0,
    );
    let mut stream = 7u64;
    let x = random(&mut stream) * (4.0 + 4.0) + (0.0 - 4.0);
    let duration = random(&mut stream) * (1.0 + 1.0) + (3.0 - 1.0);
    let fields = Noise::new(&mut stream, 16);
    assert_eq!((a.origin[0], b.duration), (x, duration));
    assert_eq!(a.noise[0].permutation, fields.permutation);
    assert_eq!((a.seed, b.seed), (stream, 8));
}

#[test]
fn subsystems_of_one_system_share_and_advance_its_turbulence_fields() {
    let mut leader = activation_test_emitter();
    leader.noise_divisions = vec![16];
    leader.forces = vec![test_force(ForceKind::Turbulence, [0.0; 3], [0.0; 3], 10.0)];
    let mut follower = leader.clone();
    follower.leads = false;
    let mut a = Simulation::cold(&leader, 7, [0.; 3], 0.);
    let mut b = Simulation::cold(&follower, 8, [0.; 3], 0.);
    advance_system(
        &mut [
            Member { sim: &mut a, emitter: &leader, space: None },
            Member { sim: &mut b, emitter: &follower, space: None },
        ],
        0.05,
        1.0,
    );
    assert!(!a.particles.is_empty() && !b.particles.is_empty());
    assert_eq!((a.noise.len(), b.noise.len()), (1, 0));
    assert_eq!(a.noise[0].slice, 2);
    let mut alone = Simulation::cold(&follower, 8, [0.; 3], 0.);
    alone.advance(&follower, 0.05, 1.0);
    assert_ne!(b.particles[0].velocity, alone.particles[0].velocity);
}

#[test]
fn a_delayed_system_waits_for_its_event_before_it_starts() {
    let mut emitter = activation_test_emitter();
    emitter.delay = 2.0;
    emitter.start.base = -3.0;
    let mut sim = Simulation::cold(&emitter, 7, [0.; 3], 0.);
    sim.advance(&emitter, 1.0, 1.0);
    assert!(sim.waiting() && sim.particles.is_empty());
    sim.advance(&emitter, 2.0, 1.0);
    assert!(sim.waiting() && sim.particles.is_empty());
    sim.advance(&emitter, 2.25, 1.0);
    assert!(!sim.waiting() && !sim.particles.is_empty());
    assert!((sim.age - 3.25).abs() < 1e-5, "{}", sim.age);
    let mut culled = Simulation::cold(&emitter, 7, [0.; 3], 0.);
    culled.advance(&emitter, 1.0, 1.0);
    culled.time = 5.0;
    culled.advance(&emitter, 5.25, 1.0);
    assert!(!culled.waiting() && !culled.particles.is_empty());
}

#[test]
fn a_producer_s_pairs_keep_the_clock_they_were_created_on() {
    let defines = reference_defines();
    let mut fades = fade::Fades::default();
    fades.produce([7], 800.0, &defines, |_| true);
    assert_eq!(fades.born(7), None);
    fades.stamp(5.0);
    fades.produce([7], 800.0, &defines, |_| true);
    fades.stamp(9.0);
    assert_eq!(fades.born(7), Some(5.0));
    fades.stamp(1.0);
    assert_eq!(fades.born(7), Some(1.0));
    fades.remove(7);
    assert_eq!(fades.born(7), None);
}

#[test]
fn an_event_s_finished_subsystem_retires_with_its_system() {
    let mut short = activation_test_emitter();
    short.life.base = 0.05;
    short.leads = false;
    let mut long = long_lived_test_emitter();
    long.leads = true;
    let mut event = viewer_models::order_fixture::events(
        viewer_models::order_fixture::ORDERS[0],
        false,
    )
    .remove(0);
    let name = event.definition.particle.clone().expect("a particle event");
    let entities = BTreeMap::from([(name, (1.0, vec![long, short]))]);
    let mut simulations = EntitySimulations::default();
    for (age, end) in [(0.1, None), (0.3, Some(0.2)), (0.5, Some(0.2)), (0.7, Some(0.2))] {
        event.age = age;
        event.emission_end = end;
        simulations.prepare(&entities, std::slice::from_ref(&event)).unwrap();
    }
    let state = simulations.presentation_state();
    let lines: Vec<_> = state.lines().collect();
    assert_eq!(lines.len(), 2, "{state}");
    assert!(lines.iter().all(|line| line.contains("time=0.7 ")), "{state}");
    assert!(lines[1].contains("emitting=false") && lines[1].contains("particles=[]"), "{state}");
}

#[test]
fn a_ranged_value_draws_around_its_animated_base() {
    let half = Curve::new(&[[0.0, 0.5], [1.0, 0.5]], 0.0, 1.0, 0.0, 1.0);
    let base = half.apply(10.0, 0.0);
    assert!((base - 5.0).abs() < 1e-5);
    let value = Parameter {
        base: 10.0,
        spread: 2.0,
        curve: Some(half),
    };
    let mut seed = 0x1234_5678_0000_0001;
    let mut expected = seed;
    let drawn = value.at(&mut seed, 0.0);
    assert_eq!(drawn, random(&mut expected) * 4.0 + (base - 2.0));
    assert!((3.0..=7.0).contains(&drawn));
}

#[test]
fn a_ranged_emission_draws_a_new_rate_every_update() {
    let mut emitter = activation_test_emitter();
    emitter.emission = Parameter {
        base: 0.1,
        spread: 2.0,
        curve: None,
    };
    let mut sim = Simulation::cold(&emitter, 12345, [0.; 3], 0.);
    let mut accumulator = 0.0f32;
    let mut rates = Vec::new();
    for _ in 0..12 {
        let mut seed = sim.seed;
        let rate = random(&mut seed) * (2.0 + 2.0) + (0.1 - 2.0);
        accumulator += rate.max(0.0) * 0.5;
        accumulator -= (accumulator as usize) as f32;
        sim.tick(&emitter, 0.5);
        assert_eq!(sim.accumulator, accumulator);
        rates.push(rate);
    }
    assert!(rates.iter().all(|rate| (-1.9..=2.1).contains(rate)));
    assert!(rates.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn particle_blocks_read_forces_ranges_and_refuse_what_the_map_cannot_present() {
    let doc = parser::parse(
        br#"particle = { name = storm
            subsystem = { name = cloud max_amount = 4 force = spin,vortex,missing,1,spin
                emission = { 0.1 2.0 } position = { x = { 0 4 } y = 15 } duration = { 3 1 } }
            force = { type = "VORTEX" name = "vortex" position = { 0 25 0 } direction = { 1 1 0 }
                local_force = yes yaw = 90 amount = 4 }
            force = { type = "spin" name = "spin" direction = { 0 1 0 } amount = 0.1 }
            force = { type = "magnet" name = "odd" }
        }"#,
    )
    .unwrap();
    let root = particle_root(&doc, "fixture", "storm", Pick::Last).unwrap();
    let [cloud] = &parse_emitters(root).unwrap()[..] else {
        panic!("one subsystem");
    };
    assert_eq!(
        cloud.forces.iter().map(|f| f.kind).collect::<Vec<_>>(),
        [ForceKind::Spin, ForceKind::Vortex, ForceKind::Spin]
    );
    let vortex = &cloud.forces[1];
    let quarter = 90f32 * std::f32::consts::PI / 180.0;
    assert_eq!(vortex.yaw, [quarter.sin(), quarter.cos()]);
    assert_eq!((vortex.position, vortex.direction), ([0., 25., 0.], [1., 1., 0.]));
    assert!(vortex.local && !cloud.forces[0].local);
    assert_eq!(vortex.amount.base, 4.0);
    assert_eq!((cloud.emission.base, cloud.emission.spread), (0.1, 2.0));
    assert_eq!((cloud.position[0].spread, cloud.position[1].base), (4.0, 15.0));
    assert_eq!((cloud.duration.base, cloud.duration.spread), (3.0, 1.0));
    for (source, why) in [
        (
            &br#"particle = { name = storm subsystem = { name = cloud max_amount = 1 force = odd }
                force = { type = "magnet" name = "odd" } }"#[..],
            "particle storm subsystem cloud: force odd has unknown type \"magnet\"",
        ),
        (
            br#"particle = { name = storm subsystem = { name = outer max_amount = 4
                subsystem = { name = inner max_amount = 2 } } }"#,
            "particle storm subsystem outer: nests particle subsystems",
        ),
    ] {
        let doc = parser::parse(source).unwrap();
        let root = particle_root(&doc, "fixture", "storm", Pick::Last).unwrap();
        let error = parse_emitters(root).unwrap_err().to_string();
        assert!(error.contains(why), "{error}");
    }
    let doc = parser::parse(
        br#"particle = { name = storm subsystem = { name = outer hide = yes max_amount = 4
            subsystem = { name = inner max_amount = 2 } } }"#,
    )
    .unwrap();
    let root = particle_root(&doc, "fixture", "storm", Pick::Last).unwrap();
    assert!(parse_emitters(root).unwrap().is_empty());
}

pub(super) fn assert_same_state(left: &Simulation, right: &Simulation) {
    assert_eq!(left.prepared, right.prepared);
    assert_eq!(left.particles.len(), right.particles.len());
    assert_eq!(left.age, right.age);
    assert_eq!(left.update_age, right.update_age);
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
        assert_eq!(left.rotation_base, right.rotation_base);
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
    use content_reader_mesh::mesh::Transform;
    let mut emitter = activation_test_emitter();
    emitter.emission.base = 2.;
    emitter.velocity.base = 0.;
    emitter.forces.clear();
    for (axis, base) in emitter.position.iter_mut().zip([1., 2., 3.]) {
        axis.base = base;
    }
    emitter.system_local_space = true;
    let world = Transform::from_trs([10., 0., 0.], [0., 0., 0., 1.], 2.);
    let mut sim = Simulation::cold(&emitter, 7, world.translation, 0.);
    sim.tick_space(
        &emitter,
        1.,
        Some(motion::Space::new(false, true, 3., world)),
        &mut Vec::new(),
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
        &mut Vec::new(),
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
        emitter.emission.base = 6.;
        emitter.forces.clear();
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
        emitter.emission.base = 0.;
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
        emitter.emission.base = 2.;
        simulation.tick(&emitter, 0.5);
        assert_eq!(simulation.particles.last().unwrap().age, 0.);
    }
}

#[test]
fn expiry_keeps_unordered_age_and_lifetime_like_native_comiss() {
    let mut emitter = activation_test_emitter();
    emitter.emission.base = 2.;
    let mut simulation = Simulation::cold(&emitter, 12345, [0.; 3], 0.);
    simulation.tick(&emitter, 1.);
    simulation.particles[0].age = f32::NAN;
    simulation.particles[1].life = f32::NAN;
    emitter.emission.base = 0.;
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
    direct.advance(&emitter, 8.0, 1.0);
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

#[test]
fn a_simulation_moves_without_its_noise_table() {
    assert!(std::mem::size_of::<Simulation>() < 256);
    assert!(std::mem::size_of::<Noise>() > 8 * 1024);
}

#[test]
fn a_kept_event_leaves_its_player_once_none_of_its_particles_lives() {
    use viewer_models::entity_animation::{Clock, Event, Player, Request, stage_rig};
    use viewer_models::{ModelInstance, PaletteJoint};
    use content_reader_mesh::mesh;
    use std::{collections::BTreeMap, sync::Arc};

    let mut rig = stage_rig(true);
    rig.nodes[0].clips[2].state.events.push(mesh::EntityEvent {
        time: 0.1,
        particle: Some("drop".into()),
        keep_particle: true,
        ..Default::default()
    });
    let instance = ModelInstance {
        scale: 1.,
        animation_seed: 7,
        ..Default::default()
    };
    let requests = Arc::new(vec![
        Request {
            time: 1.,
            phase_time: None,
            state: "dive_intro".into(),
        },
        Request {
            time: 4.,
            phase_time: None,
            state: "dive".into(),
        },
    ]);
    let evaluate = |player: &mut Player, time: f32| {
        let mut events = Vec::new();
        let clock = Clock {
            time,
            phase: None,
            requests: Arc::clone(&requests),
        };
        player.evaluate_clock(
            &rig,
            instance,
            &clock,
            &mut [PaletteJoint::IDENTITY],
            &mut events,
        );
        events
    };
    let kept = |events: &[Event]| events.iter().filter(|event| event.kept.is_some()).count();
    let mut player = Player::new(&rig);
    let events = evaluate(&mut player, 4.2);
    assert_eq!(kept(&events), 1);
    assert_eq!(kept(&evaluate(&mut player, 4.3)), 1);
    crate::weather_effects::advance_entity_effects(
        &mut Default::default(),
        &BTreeMap::new(),
        &crate::entity_effects::Lights::default(),
        events,
    )
    .unwrap();
    assert_eq!(kept(&evaluate(&mut player, 4.4)), 0);
    assert!(player.kept_events_empty());
}
