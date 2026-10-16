use super::*;
use installed_content_core::test_support::Stack;

#[test]
fn asset_files_load_recursively_and_resolve_after_every_file() {
    let stack = Stack::new("sound_catalog_order");
    let base = stack.base();
    // `category.asset` sorts before the effects it names; references wait for every file.
    stack.write(
        &base,
        "sound/category.asset",
        r#"category = { name = "Effects" soundeffects = { boom hum missing_effect }
             compressor = { pregain = 20 ratio = 0 threshold = -10 attacktime = 0.03 } }
           master_compressor = { postgain = -20 enabled = no }"#,
    );
    stack.write(
        &base,
        "sound/effects.asset",
        r#"soundeffect = { name = boom falloff = "near" volume = 0.5 is3d = yes
             sounds = { sound = boom_01 weighted_sound = { sound = boom_02 weight = 3 } sound = gone }
             volume_random_offset = { 0.0 -0.15 } playbackrate_random_offset = { -5 5 }
             max_audible = 3 max_audible_behaviour = fail loop = no }
           soundeffect = { name = hum sounds = { sound = boom_01 } loop = yes falloff = absent
             playbackrate = 50 random_sound_when_looping = no }"#,
    );
    stack.write(
        &base,
        "sound/units/sounds.asset",
        r#"sound = { name = "boom_01" file = "boom1.wav" volume = 0.25 }
           sound = { name = boom_02 file = "deep/boom2.wav" }
           falloff = { name = near min_distance = 10 max_distance = 30 height_scale = 0.5 type = linear }"#,
    );
    let catalog = Catalog::read(&stack.layers(&[])).unwrap();
    assert_eq!(
        catalog.files,
        [
            "sound/category.asset",
            "sound/effects.asset",
            "sound/units/sounds.asset"
        ]
    );
    let boom = &catalog.effects[catalog.effect("boom").unwrap()];
    assert_eq!(boom.volume, 0.5);
    assert!(boom.positional && !boom.looping);
    assert_eq!(boom.overflow, Overflow::Fail);
    assert_eq!(boom.max_audible, 3);
    assert_eq!(
        boom.volume_offset,
        Range {
            first: 0.0,
            second: -0.15
        }
    );
    assert_eq!(
        boom.playback_rate_offset,
        Range {
            first: -0.05,
            second: 0.05
        }
    );
    let sounds: Vec<_> = boom
        .choices
        .iter()
        .map(|choice| (catalog.sounds[choice.sound].name.as_str(), choice.weight))
        .collect();
    assert_eq!(sounds, [("boom_01", 1), ("boom_02", 3)]);
    // Files are relative to their own asset's folder.
    assert_eq!(catalog.sounds[0].path, "sound/units/boom1.wav");
    assert_eq!(catalog.sounds[0].volume, 0.25);
    assert_eq!(catalog.sounds[1].path, "sound/units/deep/boom2.wav");
    let near = &catalog.falloffs[boom.falloff.unwrap()];
    assert_eq!(
        (
            near.min_distance,
            near.max_distance,
            near.height_scale,
            near.curve
        ),
        (10.0, 30.0, 0.5, Curve::Linear)
    );
    let hum = &catalog.effects[catalog.effect("hum").unwrap()];
    assert_eq!(hum.falloff, None);
    assert_eq!(hum.playback_rate, 0.5);
    assert!(hum.looping && !hum.random_sound_when_looping);
    assert_eq!(hum.category, boom.category);
    let effects = &catalog.categories[boom.category.unwrap()];
    assert_eq!(effects.name, "Effects");
    let compressor = effects.compressor.unwrap();
    assert!((compressor.pregain - 10.0).abs() < 1e-5);
    assert_eq!(
        (compressor.ratio, compressor.threshold, compressor.attack),
        (1.0, -10.0, 0.03)
    );
    let master = catalog.master_compressor.unwrap();
    assert!(!master.enabled);
    assert!((master.postgain - 0.1).abs() < 1e-6);
    let notes = catalog.notes.lines();
    assert!(
        notes
            .contains(&"sound/effects.asset: soundeffect boom names missing sound gone".to_owned())
    );
    assert!(
        notes
            .iter()
            .any(|note| note.contains("hum names missing falloff absent"))
    );
    assert!(
        notes.contains(
            &"sound/category.asset: category Effects names missing soundeffect missing_effect"
                .to_owned()
        )
    );
}

#[test]
fn first_declarations_win_and_later_categories_take_effects_over() {
    let stack = Stack::new("sound_catalog_duplicates");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(
        &base,
        "sound/a.asset",
        r#"sound = { name = s file = "base.wav" }
           soundeffect = { name = e sounds = { sound = s } volume = 0.2
               delay_random_offset = { 1 } volume_random_offset = { 0 x } }
           falloff = { name = f max_distance = 40 }
           category = { name = One soundeffects = { e } }"#,
    );
    stack.write(
        &addon,
        "sound/b.asset",
        r#"sound = { name = s file = "mod.wav" }
           soundeffect = { name = e sounds = { sound = s } volume = 0.9 }
           falloff = { name = f max_distance = 90 }
           category = { name = Two soundeffects = { e } }
           unknown_block = { }"#,
    );
    let catalog = Catalog::read(&stack.layers(&[])).unwrap();
    assert_eq!(catalog.sounds.len(), 1);
    assert_eq!(catalog.sounds[0].path, "sound/base.wav");
    assert_eq!(catalog.effects[0].volume, 0.2);
    assert_eq!(catalog.falloffs[0].max_distance, 40.0);
    assert_eq!(
        catalog.categories[catalog.effects[0].category.unwrap()].name,
        "Two"
    );
    let notes = catalog.notes.lines();
    for expected in [
        "sound/b.asset: refused repeated sound s",
        "sound/b.asset: refused repeated soundeffect e",
        "sound/b.asset: refused repeated falloff f",
        "sound/b.asset: soundeffect e moves from category One to Two",
        "sound/b.asset: refused top-level block unknown_block",
        "sound/a.asset: refused malformed delay_random_offset",
        "sound/a.asset: refused malformed volume_random_offset",
    ] {
        assert!(
            notes.contains(&expected.to_owned()),
            "{expected}: {notes:?}"
        );
    }
}

#[test]
fn a_repeated_category_takes_the_later_compressor_and_sounds_read_their_priority() {
    let stack = Stack::new("sound_catalog_category_repeat");
    let base = stack.base();
    stack.write(
        &base,
        "sound/a.asset",
        r#"sound = { name = s file = "a.wav" priority = 3 }
           sound = { name = t file = "b.wav" }
           soundeffect = { name = e sounds = { sound = s } }
           category = { name = One compressor = { ratio = 2 } soundeffects = { e } }
           category = { name = One soundeffects = { } }
           category = { name = One compressor = { ratio = 8 } }"#,
    );
    let catalog = Catalog::read(&stack.layers(&[])).unwrap();
    assert_eq!(catalog.categories.len(), 1);
    // A repeat without a compressor keeps the earlier one; a later compressor
    // replaces it.
    assert_eq!(catalog.categories[0].compressor.unwrap().ratio, 8.0);
    assert_eq!(catalog.sounds[0].priority, 3.0);
    assert_eq!(catalog.sounds[1].priority, 1.0);
    assert!(catalog.notes.is_empty(), "{:?}", catalog.notes.lines());
}

#[test]
fn a_mod_replacing_the_sound_folder_hides_the_base_definitions() {
    let stack = Stack::new("sound_catalog_replace");
    let (base, addon) = (stack.base(), stack.addon());
    stack.write(&base, "sound/a.asset", "sound = { name = s file = x.wav }");
    stack.write(&addon, "sound/b.asset", "sound = { name = t file = y.wav }");
    let catalog = Catalog::read(&stack.layers(&["sound"])).unwrap();
    assert_eq!(catalog.sound("s"), None);
    assert_eq!(catalog.sound("t"), Some(0));
}

#[test]
fn defaults_follow_the_engine_reader() {
    let falloff = Falloff::default();
    assert_eq!(
        (
            falloff.min_distance,
            falloff.max_distance,
            falloff.height_scale,
            falloff.curve
        ),
        (0.0, 100.0, 1.0, Curve::Logarithmic)
    );
    let effect = Effect::new("e".into(), String::new());
    assert_eq!(effect.max_audible, -1);
    assert_eq!(effect.overflow, Overflow::Silent);
    assert!(effect.random_sound_when_looping);
    assert_eq!(
        (effect.volume, effect.playback_rate, effect.polyphony),
        (1.0, 1.0, 1)
    );
}

#[test]
fn height_volume_clamps_altitude_and_interpolates() {
    let defines = Defines {
        height_category: "Atmosphere".into(),
        height_min_altitude: 100.0,
        height_max_altitude: 2000.0,
        height_min_volume: 0.2,
        height_max_volume: 1.0,
        voice_category: "Voices".into(),
    };
    assert_eq!(defines.height_volume(10.0), 0.2);
    assert_eq!(defines.height_volume(5000.0), 1.0);
    assert!((defines.height_volume(1050.0) - 0.6).abs() < 1e-6);
}

/// Reads the installation's own sound definitions and decodes every sample an
/// effect can choose. Set `BATTLE_PLAN_GAME_DIR`; DLC folders are not mounted here.
#[test]
#[ignore = "reads the installation named by BATTLE_PLAN_GAME_DIR"]
fn installed_sounds_resolve_and_decode() {
    let game = std::path::PathBuf::from(std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("BATTLE_PLAN_GAME_DIR"));
    let layers = ResolvedLayers::from_folders(&[(&game, &[])]).unwrap();
    let catalog = Catalog::read(&layers).unwrap();
    let mut decoded = 0;
    let mut refused = Notes::default();
    for effect in &catalog.effects {
        for choice in &effect.choices {
            let sound = &catalog.sounds[choice.sound];
            match layers.read_uncovered(&sound.path) {
                Ok(bytes) => match crate::wav::decode(&bytes) {
                    Ok(_) => decoded += 1,
                    Err(why) => refused.note(format!("{}: {why}", sound.path)),
                },
                Err(error) => refused.note(format!("{}: {error}", sound.path)),
            }
        }
    }
    eprintln!(
        "{} files, {} sounds, {} effects, {} falloffs, {} categories; {decoded} decoded",
        catalog.files.len(),
        catalog.sounds.len(),
        catalog.effects.len(),
        catalog.falloffs.len(),
        catalog.categories.len()
    );
    for line in catalog.notes.lines().iter().chain(&refused.lines()) {
        eprintln!("  {line}");
    }
    assert!(decoded > 0);
}
