use super::*;

#[test]
fn thermal_pairs_keep_order_fixed5_and_repeated_group_values() {
    let ranges = parse_temperature_ranges(
        br#"
        @boundary=-0.00001
        settings={temperature={
            weather_very_hot={20.00001 28.12345}
            weather_extreme_cold={-100 @boundary}
            weather_very_cold={@boundary -4}
        }}
        settings={temperature={weather_very_hot={30 40} weather_extreme_hot={yes 100}}}
    "#,
    )
    .unwrap();
    assert_eq!(
        ranges,
        [
            [-10_000_000, -1],
            [-1, -400_000],
            [3_000_000, 4_000_000],
            [0, 10_000_000]
        ]
    );
    assert_eq!(
        parse_temperature_ranges(b"settings={temperature={}}").unwrap(),
        [[0; 2]; 4]
    );
    assert_eq!(
        parse_temperature_ranges(b"settings={temperature={weather_very_hot={4 2}}}").unwrap()[2],
        [400_000, 200_000]
    );
    for text in [
        "settings={temperature={weather_very_hot={1}}}",
        "settings={temperature={weather_extreme_cold={1 2 3}}}",
        "settings={temperature={unknown={1 2}}}",
    ] {
        assert!(parse_temperature_ranges(text.as_bytes()).is_err(), "{text}");
    }
}

#[test]
fn province_modifier_rules_keep_ranges_ceiling_and_classify_first_and_high() {
    let rules = parse_province_modifier_rules(
        br#"
        settings={
            temperature={weather_extreme_cold={-100 -20} weather_very_cold={-30 -5}}
            snowing={
                weather_ground_snow_medium={1 10}
                weather_ground_snow_high={yes 20}
                snow_gain_max=2.5
                snow_melt_temperature={0 1}
            }
        }
        settings={snowing={weather_ground_snow_high={5.5 20} snow_gain_max=12}}
    "#,
    )
    .unwrap();
    assert_eq!(
        rules,
        ProvinceModifierRules {
            temperature: [
                [-10_000_000, -2_000_000],
                [-3_000_000, -500_000],
                [0, 0],
                [0, 0]
            ],
            ground_snow: [[100_000, 1_000_000], [550_000, 2_000_000]],
            snow_gain_max: 1_200_000,
        }
    );
    assert_eq!(
        rules.temperature_modifier(-2_500_000),
        Some("weather_extreme_cold")
    );
    assert_eq!(
        rules.temperature_modifier(-2_000_000),
        Some("weather_very_cold")
    );
    assert_eq!(rules.temperature_modifier(-500_000), None);
    assert_eq!(rules.temperature_modifier(0), None);
    assert_eq!(
        rules.ground_snow_modifier(300_000, 0),
        Some("weather_ground_snow_medium")
    );
    assert_eq!(
        rules.ground_snow_modifier(500_000, 100_000),
        Some("weather_ground_snow_high")
    );
    assert_eq!(
        rules.ground_snow_modifier(2_000_000, 0),
        Some("weather_ground_snow_high")
    );
    assert_eq!(rules.ground_snow_modifier(50_000, 0), None);
    assert_eq!(
        parse_province_modifier_rules(b"").unwrap(),
        ProvinceModifierRules::default()
    );
    for text in [
        "settings={snowing={weather_ground_snow_medium={1}}}",
        "settings={snowing={weather_ground_snow_high={1 2 3}}}",
        "settings={snowing={unknown=1}}",
    ] {
        assert!(
            parse_province_modifier_rules(text.as_bytes()).is_err(),
            "{text}"
        );
    }
}

#[test]
fn variables_keyword_tokens_and_error_recovery() {
    let names = parse_effect_names(
        br#"@sprite=rain weather_effects={RAIN_LIGHT={
        weather_effect={unknown={entity=hidden} "entity"=ignored ENTITY=@sprite small=yes small=bad}
    }}"#,
    )
    .unwrap();
    assert_eq!(names[&(0, true)][0][0].as_deref(), Some("rain"));
    assert!(
        parse_effect_names(b"settings={temperature={weather_very_hot={1}}}")
            .unwrap_err()
            .to_string()
            .contains("weather_very_hot")
    );
    assert!(
        parse_effect_names(b"settings={unsupported={}} ")
            .unwrap_err()
            .to_string()
            .contains("unsupported")
    );
}

#[test]
fn effects_keep_banks_pair_members_and_authored_order() {
    let names = parse_effect_names(
        br#"weather_effects = {
            rain_light = {
                weather_effect = { entity = rain fading_entity = cloud }
                weather_effect = { entity = other }
                weather_effect = { entity = small_rain small = yes }
            }
            sandstorm = { weather_effect = { fading_entity = dust small = yes } }
            snow = { weather_effect = { } }
        }"#,
    )
    .unwrap();
    let name = |value: &str| Some(value.to_owned());
    assert_eq!(
        names[&(0, false)],
        [[name("rain"), name("cloud")], [name("other"), None]]
    );
    assert_eq!(names[&(0, true)], [[name("small_rain"), None]]);
    assert_eq!(names[&(4, true)], [[None, name("dust")]]);
    assert!(!names.contains_key(&(2, false)));
}

#[test]
fn entities_resolve_through_clones_and_refuse_unsupported_forms() {
    let entities = mesh::parse_descriptors(
        br#"
        entity = { name = base scale = 2 cull_radius = 3 default_state = idle
            state = { name = idle state_time = 2 looping = no
                event = { time = 0 particle = drops keep_particle = yes }
                event = { trigger_once = yes sound = { soundeffect = rain } }
            }
        }
        entity = { name = custom clone = base cull_radius = 99 }
        entity = { name = timed state = { name = idle event = { time = 1 particle = drops keep_particle = yes } } }
        entity = { name = delayed state = { name = idle looping = no event = { time = 1.5 particle = drops keep_particle = yes } } }
        entity = { name = offset state = { name = idle looping = no animation_start_time = 1 event = { time = 2 particle = drops keep_particle = yes } } }
        entity = { name = random state = { name = idle looping = no animation_random_start_time = 1 event = { time = 2 particle = drops keep_particle = yes } } }
        entity = { name = orphan clone = absent }
        "#,
        "gfx/entities/weather.asset",
    )
    .unwrap();
    let mut catalog = Catalog::default();
    catalog.add_test_particle("drops", "unrelated_id", 0.5, "gfx/particles/x/file.asset");
    let drops = |scale: f32, delay: f32| EffectParticle {
        particle: Particle {
            path: "gfx/particles/x/file.asset".into(),
            name: "unrelated_id".into(),
            scale,
            registration: 0,
        },
        delay,
    };
    assert_eq!(
        entity_particles("delayed", &entities, &catalog).unwrap().0,
        [drops(0.5, 1.5)]
    );
    assert_eq!(
        entity_particles("custom", &entities, &catalog).unwrap(),
        (
            vec![drops(1.0, 0.0)],
            EffectParent {
                name: "custom".into(),
                radius: 6.,
                sounds: vec![WeatherSound {
                    effect: "rain".into(),
                    time: 0.0,
                }],
            }
        )
    );
    for name in ["timed", "offset", "random", "orphan", "absent"] {
        assert!(
            entity_particles(name, &entities, &catalog).is_err(),
            "{name}"
        );
    }
}

#[test]
fn cull_radius_scans_byte_prefixes_and_refuses_unported_carriers() {
    let entities = mesh::parse_descriptors(
        b"entity={name=raw cull_radius=\"2.5\x80\" state={name=idle}}",
        "gfx/entities/weather.asset",
    )
    .unwrap();
    let (_, parent) = entity_particles("raw", &entities, &Catalog::default()).unwrap();
    assert_eq!(parent.radius, 2.5);
    let quoted = mesh::parse_descriptors(
        b"entity={name=quoted cull_radius=\"@radius\" state={name=idle}}",
        "gfx/entities/weather.asset",
    )
    .unwrap();
    assert_eq!(
        entity_particles("quoted", &quoted, &Catalog::default())
            .unwrap()
            .1
            .radius,
        1.
    );
    for value in ["@radius", "{}", "rgb {1 2 3}"] {
        let text = format!("entity={{name=raw cull_radius={value} state={{name=idle}}}}");
        let error = mesh::parse_descriptors(text.as_bytes(), "gfx/entities/weather.asset")
            .unwrap_err()
            .to_string();
        assert!(error.contains("entity raw cull_radius"), "{error}");
        assert!(error.contains("unsupported native consumption"), "{error}");
    }
}

#[test]
fn weather_uses_registered_clone_snapshots_versions_and_held_radius_reads() {
    let root = std::env::temp_dir().join(format!("hoi4-weather-registry-{}", std::process::id()));
    std::fs::create_dir_all(root.join("common")).unwrap();
    std::fs::create_dir_all(root.join("gfx/entities")).unwrap();
    std::fs::write(root.join(PATH), b"weather_effects={rain_light={weather_effect={entity=storm fading_entity=base} weather_effect={entity=plain} weather_effect={entity=held_clone} weather_effect={entity=scanned}}}").unwrap();
    std::fs::write(
        root.join("gfx/entities/weather.asset"),
        br#"
        entity={name=base scale=2 cull_radius=3 state={name=idle}}
        entity={name=storm clone=base cull_radius=99}
        entity={name=base scale=3 cull_radius=4 state={name=idle}}
        entity={name=plain cull_radius=7 state={name=idle}}
        entity={name=plain state={name=idle}}
        entity={name=held version=5 cull_radius=8 state={name=idle}}
        entity={name=held version=4 cull_radius=99 state={name=idle}}
        entity={name=held_clone clone=held cull_radius=99}
        entity={name=scanned cull_radius=bad cull_radius=2.5junk cull_radius=bad state={name=idle}}
        entity={name=dropped clone=later}
        entity={name=later state={name=idle}}
    "#,
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let before = layers.descriptor_roster().unwrap();
    let effects = effects(&layers, &Catalog::default()).unwrap();
    let after = layers.descriptor_roster().unwrap();
    assert!(std::sync::Arc::ptr_eq(&before, &after));
    let pairs = &effects[&(0, false)];
    assert_eq!(
        pairs[0]
            .parents
            .each_ref()
            .map(|p| p.as_ref().map(|p| p.radius)),
        [Some(6.), Some(12.)]
    );
    assert_eq!(pairs[1].parents[0].as_ref().unwrap().radius, 1.);
    assert_eq!(pairs[2].parents[0].as_ref().unwrap().radius, 8.);
    assert_eq!(pairs[3].parents[0].as_ref().unwrap().radius, 2.5);
    assert!(
        entity_particles("dropped", before.descriptor_job().view(), &Catalog::default())
            .unwrap_err()
            .to_string()
            .contains("dropped")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn weather_refuses_a_requested_name_from_an_unreadable_later_descriptor() {
    let root = std::env::temp_dir().join(format!("hoi4-weather-uncertain-{}", std::process::id()));
    std::fs::create_dir_all(root.join("common")).unwrap();
    std::fs::create_dir_all(root.join("gfx/entities")).unwrap();
    std::fs::write(
        root.join(PATH),
        b"weather_effects={rain_light={weather_effect={entity=uncertain} weather_effect={entity=plain}}}",
    )
    .unwrap();
    std::fs::write(
        root.join("gfx/entities/a.asset"),
        b"entity={name=uncertain state={name=idle}} entity={name=plain cull_radius=2 state={name=idle}}",
    )
    .unwrap();
    std::fs::write(
        root.join("gfx/entities/b.asset"),
        b"entity={name=uncertain cull_radius=@radius state={name=idle}}",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert!(
        layers
            .descriptor_roster()
            .unwrap()
            .has_entity("uncertain")
    );
    let effects = effects(&layers, &Catalog::default()).unwrap();
    let [uncertain, plain] = &effects[&(0, false)][..] else {
        panic!("both pairs keep their places: {effects:?}");
    };
    let why = uncertain.withheld.as_deref().unwrap();
    assert!(
        why.contains("weather entity uncertain: unreadable descriptor gfx/entities/b.asset"),
        "{why}"
    );
    assert!(uncertain.name.contains("rain_light weather_effect 1 (entity uncertain"));
    assert_eq!(uncertain.parents, [None, None]);
    assert_eq!(plain.withheld, None);
    assert_eq!(plain.parents[0].as_ref().unwrap().radius, 2.);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn particle_consumer_refuses_loader_abort_and_thermal_notes_survive() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!("hoi4-weather-consumer-{}", std::process::id()));
    std::fs::create_dir_all(root.join("common/modifiers")).unwrap();
    std::fs::write(
        root.join(PATH),
        b"weather_effects={rain_light={weather_effect={}}}",
    )
    .unwrap();
    std::fs::write(root.join("common/modifiers/a.txt"), b"other=word").unwrap();
    std::fs::write(
        root.join("common/modifiers/b.txt"),
        b"weather_extreme_cold={} weather_very_cold={} weather_very_hot={} weather_extreme_hot={}",
    )
    .unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let notes = crate::static_modifiers::read_identifiers(&layers, &TEMPERATURE_MODIFIERS)
        .unwrap()
        .notes;
    assert!(!notes.is_empty());
    assert_eq!(temperature_ranges(&layers).unwrap().notes, notes);
    assert!(effects(&layers, &Catalog::default()).unwrap().is_empty());
    std::fs::write(root.join(PATH), b"weather_effects={rain_light={weather_effect={}}} settings={temperature={weather_very_hot={1}}}").unwrap();
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    assert!(
        effects(&layers, &Catalog::default())
            .unwrap_err()
            .to_string()
            .contains("weather_very_hot")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reachable_province_modifiers_follow_the_intervals() {
    let rules = ProvinceModifierRules {
        temperature: [
            [-10_000_000, -2_000_000],
            [-5_000_000, -3_000_000],
            [2_000_000, 3_000_000],
            [3_000_000, 10_000_000],
        ],
        ground_snow: [[100_000, 500_000], [400_000, 2_000_000]],
        snow_gain_max: 300_000,
    };
    assert_eq!(
        rules.reachable_temperature_modifiers(),
        [
            None,
            Some("weather_extreme_cold"),
            Some("weather_very_hot"),
            Some("weather_extreme_hot")
        ]
    );
    assert_eq!(
        rules.reachable_ground_snow_modifiers(),
        [None, Some("weather_ground_snow_medium")]
    );
    let empty = ProvinceModifierRules {
        ground_snow: [[0, 0], [0, 0]],
        ..rules
    };
    assert_eq!(empty.reachable_ground_snow_modifiers(), [None]);
}
