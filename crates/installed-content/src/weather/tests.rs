use super::*;

#[test]
fn thermal_pairs_keep_native_order_fixed5_and_repeated_group_values() {
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
    // Native accepts inverted/overlapping intervals; consumers decide matches.
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
fn native_variables_keyword_tokens_and_error_recovery() {
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
        entity = { name = base scale = 2 default_state = idle
            state = { name = idle state_time = 2 looping = no
                event = { time = 0 particle = drops keep_particle = yes }
                event = { trigger_once = yes sound = { soundeffect = rain } }
            }
        }
        entity = { name = custom clone = base }
        entity = { name = timed state = { name = idle event = { time = 1 particle = drops keep_particle = yes } } }
        entity = { name = orphan clone = absent }
        "#,
        "gfx/entities/weather.asset",
    )
    .unwrap();
    let mut catalog = Catalog::default();
    catalog.add_test_particle("drops", "unrelated_id", 0.5, "gfx/particles/x/file.asset");
    assert_eq!(
        entity_particles("custom", &entities, &catalog).unwrap(),
        [Particle {
            path: "gfx/particles/x/file.asset".into(),
            name: "unrelated_id".into(),
            scale: 1.0,
            registration: 0,
        }]
    );
    for name in ["timed", "orphan", "absent"] {
        assert!(
            entity_particles(name, &entities, &catalog).is_err(),
            "{name}"
        );
    }
}

#[test]
fn descriptors_come_from_files_that_name_the_entities_and_follow_clone_parents() {
    let paths = ["a.asset", "units.asset", "storm.asset", "late.asset"].map(String::from);
    let files = BTreeMap::from([
        ("a.asset", &b"entity = { name = base scale = 2 }"[..]),
        // Unrelated files may be malformed: they are never parsed.
        ("units.asset", b"entity = { name = stormtrooper"),
        ("storm.asset", b"entity = { name = storm clone = base }"),
        ("late.asset", b"entity = { name = base scale = 3 }"),
    ]);
    let entities =
        weather_descriptors(
            &paths,
            vec!["storm".into()],
            |path| Ok(files[path].to_vec()),
        )
        .unwrap();
    assert_eq!(entities.entities.len(), 2);
    assert_eq!(entities.inherit("storm").unwrap().scale, Some(3.0));
    assert!(
        weather_descriptors(&paths[1..2], vec!["stormtrooper".into()], |path| {
            Ok(files[path].to_vec())
        })
        .is_err()
    );
}

#[test]
fn particle_consumer_refuses_native_loader_abort_and_thermal_notes_survive() {
    let root = std::env::temp_dir().join(format!("hoi4-weather-consumer-{}", std::process::id()));
    std::fs::create_dir_all(root.join("common/modifiers")).unwrap();
    std::fs::write(
        root.join(PATH),
        b"weather_effects={rain_light={weather_effect={}}}",
    )
    .unwrap();
    // The identifier reader keeps this native misread, even when later
    // definitions make all requested lookups resolvable.
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
