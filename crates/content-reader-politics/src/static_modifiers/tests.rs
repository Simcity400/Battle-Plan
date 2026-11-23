use super::*;
use std::{fs, path::Path};

#[test]
fn descriptions_cache_lookup_precedence_absence_and_refusals() {
    content_reader_script::test_game::enter();
    let descriptions = Descriptions::default();
    let mut names = KeyNames::default();
    names.effects.modifiers.insert(b"attack".to_vec());
    assert!(
        descriptions
            .files
            .set(Ok((
                names,
                vec![
                    (
                        "a.txt".into(),
                        read_file(b"SYN={attack=0.1} RENAMED={key=other}", None)
                    ),
                    ("b.txt".into(), read_file(b"SYN={attack=0.2}", None)),
                ]
            )))
            .is_ok()
    );
    let layers = ResolvedLayers::from_folders(&[]).unwrap();
    let (name, block) = descriptions.get(&layers, "SYN").unwrap();
    assert_eq!(name.as_deref(), Some("SYN"));
    assert_eq!(block.values[0].value, 20_000);
    assert_eq!(
        descriptions.get(&layers, "missing").unwrap(),
        (None, modifiers::Modifiers::default())
    );
    assert!(
        descriptions
            .get(&layers, "RENAMED")
            .unwrap_err()
            .contains("renames")
    );
    assert_eq!(descriptions.get(&layers, "SYN").unwrap().1, block);
    assert_eq!(descriptions.values.lock().unwrap().len(), 3);
}

#[test]
fn description_registry_omits_unknown_values_and_refuses_uncertain_tokens() {
    content_reader_script::test_game::enter();
    let names = KeyNames::default();
    let registry = |key: &[u8]| modifiers::registered(key, Some(&names));
    let file = read_file_with_registry(
        b"SYN={army_attack_factor=1 unknown=2 unknown_block={nested=3}}",
        Some(&names),
        &registry,
    );
    assert!(file.stop.is_none());
    let mut block = stored_block(file.definitions[0].value.items().unwrap()).unwrap();
    description_values(&mut block, &names).unwrap();
    assert_eq!(
        block
            .values
            .iter()
            .map(|value| value.name.as_str())
            .collect::<Vec<_>>(),
        ["army_attack_factor"]
    );
    let mut names = names;
    names.effects.modifiers_gap = Some("synthetic unread registry".into());
    let mut block = modifiers::values(&parser::parse(b"unknown=2").unwrap().items);
    assert!(
        description_values(&mut block, &names)
            .unwrap_err()
            .contains("registry is unavailable")
    );
}

#[test]
fn tooltip_identifiers_use_presence_precedence_and_rename_refusal() {
    content_reader_script::test_game::enter();
    let names = ["custom_cold", "absent"];
    let resolve = |files: &[(&str, &str)]| {
        resolve_with(
            files.iter().map(|(path, text)| (*path, text.as_bytes())),
            &names,
            None,
            None,
            |name, items| {
                stored_block(items)?;
                Ok(Some(name.to_owned()))
            },
        )
        .0
    };
    assert_eq!(
        resolve(&[("a.txt", "custom_cold={}")]),
        [Ok(Some("custom_cold".into())), Ok(None)]
    );
    let renamed = resolve(&[("a.txt", "custom_cold={key=changed}")]);
    assert!(renamed[0].as_ref().unwrap_err().reason.contains("renames"));
    let recovered = resolve(&[
        ("a.txt", "custom_cold={key=changed}"),
        ("b.txt", "custom_cold={}"),
    ]);
    assert_eq!(recovered[0], Ok(Some("custom_cold".into())));
}

const SCREENING: [Request<'static>; 2] = [
    Request {
        modifier: "screening_bonus",
        key: "sortie_efficiency",
    },
    Request {
        modifier: "capital_screening_bonus",
        key: "fighter_sortie_efficiency",
    },
];

fn resolved(files: &[(&str, &str)]) -> Vec<Resolved> {
    resolve(
        files.iter().map(|(path, text)| (*path, text.as_bytes())),
        &SCREENING,
        None,
    )
    .values
}

fn first(text: &str) -> Resolved {
    resolved(&[("common/modifiers/a.txt", text)])
        .into_iter()
        .next()
        .unwrap()
}

fn blocks(files: &[(&str, &str)]) -> Blocks {
    resolve_blocks(
        files.iter().map(|(path, text)| (*path, text.as_bytes())),
        &["weather_rain_light", "weather_snow"],
        None,
    )
}

#[test]
fn full_blocks_keep_last_definition_hidden_values_and_custom_tooltips() {
    content_reader_script::test_game::enter();
    let read = blocks(&[
        (
            "common/modifiers/a.txt",
            "weather_rain_light = { attack = 0.1 }",
        ),
        (
            "common/modifiers/b.txt",
            "weather_rain_light = {
                attack = 0.2 hidden_modifier = { defence = -0.3 }
                custom_modifier_tooltip = weather_help
                attack = 0.4 added_modifier = { speed = 0.9 }
            }",
        ),
    ]);
    let rain = read.blocks[0].as_ref().unwrap();
    assert_eq!(
        rain.values
            .iter()
            .map(|value| (value.name.as_str(), value.value))
            .collect::<Vec<_>>(),
        [("attack", 40_000), ("defence", -30_000)]
    );
    assert!(rain.hidden.contains("defence"));
    assert_eq!(rain.tooltips(), ["weather_help"]);
    assert_eq!(rain.children.len(), 1);
    assert_eq!(read.blocks[1], Ok(modifiers::Modifiers::default()));
}

#[test]
fn full_blocks_refuse_stopped_files_until_a_later_definition_replaces_them() {
    content_reader_script::test_game::enter();
    let stopped = blocks(&[(
        "common/modifiers/a.txt",
        "weather_rain_light = { attack = 0.1 }
         other = { attack = { unresolved = yes } }
         weather_snow = { defence = 0.2 }",
    )]);
    for (block, name) in stopped
        .blocks
        .iter()
        .zip(["weather_rain_light", "weather_snow"])
    {
        let refused = block.as_ref().unwrap_err();
        assert_eq!(refused.file, "common/modifiers/a.txt");
        assert_eq!(refused.modifier, name);
        assert_eq!(refused.key, "<whole block>");
        assert!(refused.reason.contains("the rest of the file"));
    }
    let replaced = blocks(&[
        (
            "common/modifiers/a.txt",
            "other = { attack = { unresolved = yes } }",
        ),
        (
            "common/modifiers/b.txt",
            "weather_rain_light = { attack = 0.5 }",
        ),
    ]);
    assert_eq!(replaced.blocks[0].as_ref().unwrap().values[0].value, 50_000);
    assert!(replaced.blocks[1].is_err());
}

#[test]
fn the_last_definition_and_the_last_value_win_and_absence_is_zero() {
    content_reader_script::test_game::enter();
    assert_eq!(resolved(&[]), [Ok(0), Ok(0)]);
    let values = resolved(&[
        (
            "common/modifiers/a.txt",
            "screening_bonus = { sortie_efficiency = 0.5 }
             capital_screening_bonus = { fighter_sortie_efficiency = 0.25 }",
        ),
        (
            "common/modifiers/b.txt",
            "capital_screening_bonus = { sortie_efficiency = 1 }
             screening_bonus = { sortie_efficiency = 0.1 sortie_efficiency = -0.02 }
             Screening_Bonus = { sortie_efficiency = 9 }",
        ),
    ]);
    assert_eq!(values, [Ok(-2_000), Ok(0)]);
}

#[test]
fn a_variable_reads_its_definitions_value() {
    content_reader_script::test_game::enter();
    assert_eq!(
        first("@bonus = 0.5 screening_bonus = { sortie_efficiency = @bonus }"),
        Ok(50_000)
    );
    assert_eq!(
        first("screening_bonus = { @b = 0x10 sortie_efficiency = @b }"),
        Ok(1_600_000)
    );
    let values = resolved(&[
        ("common/modifiers/a.txt", "@bonus = 0.5"),
        (
            "common/modifiers/b.txt",
            "screening_bonus = { sortie_efficiency = @bonus }",
        ),
    ]);
    assert_eq!(values[0], Ok(0));
}

#[test]
fn a_modifier_key_is_found_whatever_its_case() {
    content_reader_script::test_game::enter();
    assert_eq!(
        first("screening_bonus = { Sortie_Efficiency = 0.1 }"),
        Ok(10_000)
    );
    assert_eq!(
        first("screening_bonus = { sortie_efficiency = 0.1 \"SORTIE_EFFICIENCY\" = 0.2 }"),
        Ok(20_000)
    );
}

#[test]
fn a_value_that_does_not_convert_stores_zero() {
    content_reader_script::test_game::enter();
    for text in ["yes", ".5", "@undefined", "[?x]"] {
        assert_eq!(
            first(&format!(
                "screening_bonus = {{ sortie_efficiency = {text} }}"
            )),
            Ok(0),
            "{text}"
        );
    }
    assert_eq!(
        first("screening_bonus = { sortie_efficiency = 0.5 sortie_efficiency = yes }"),
        Ok(0)
    );
}

#[test]
fn hidden_values_count_and_a_child_modifier_does_not() {
    content_reader_script::test_game::enter();
    assert_eq!(
        first("screening_bonus = { hidden_modifier = { sortie_efficiency = 0.3 } }"),
        Ok(30_000)
    );
    assert_eq!(
        first("screening_bonus = { added_modifier = { sortie_efficiency = 0.3 } }"),
        Ok(0)
    );
}

#[test]
fn an_unfollowed_reading_or_a_rename_is_refused_by_file_modifier_and_key() {
    content_reader_script::test_game::enter();
    let values = resolved(&[(
        "common/modifiers/x.txt",
        "capital_screening_bonus = { fighter_sortie_efficiency = 0.2 }
         screening_bonus = { sortie_efficiency = { value = 1 } }",
    )]);
    for (value, request) in values.iter().zip(&SCREENING) {
        let refusal = value.as_ref().unwrap_err();
        assert_eq!(
            (
                refusal.file.as_str(),
                refusal.modifier.as_str(),
                refusal.key.as_str()
            ),
            ("common/modifiers/x.txt", request.modifier, request.key)
        );
        assert!(
            refusal
                .reason
                .contains("sortie_efficiency has a block value")
                && refusal
                    .reason
                    .ends_with("the rest of the file makes is unknown"),
            "{}",
            refusal.reason
        );
    }
    let values = resolved(&[
        (
            "common/modifiers/x.txt",
            "screening_bonus = { sortie_efficiency = { value = 1 } }",
        ),
        (
            "common/modifiers/y.txt",
            "screening_bonus = { sortie_efficiency = 0.1 }",
        ),
    ]);
    assert_eq!(values[0], Ok(10_000));
    for text in [
        "screening_bonus = { key = other sortie_efficiency = 0.1 }",
        "other = { tag = screening_bonus sortie_efficiency = 0.1 }",
    ] {
        let refusal = first(text).unwrap_err();
        assert!(refusal.reason.contains("renames it"), "{text}");
    }
}

#[test]
fn values_convert_with_the_fixed5_reader() {
    content_reader_script::test_game::enter();
    for (text, raw) in [
        ("0", 0),
        ("1", 100_000),
        ("+0.5", 50_000),
        ("-0.5", -50_000),
        ("0.00001", 1),
        ("-1.25", -125_000),
        ("0.010", 1_000),
        ("1.", 100_000),
        ("010", 800_000),
        ("-010.5", -850_000),
        ("0x10", 1_600_000),
        ("0.123456", 12_345),
        ("-0.000019", -1),
    ] {
        let text = format!("screening_bonus = {{ sortie_efficiency = {text} }}");
        assert_eq!(first(&text), Ok(raw), "{text}");
    }
}

fn write(root: &Path, files: &[(&str, &str)]) {
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

#[test]
fn static_modifier_relation_scopes_keep_weather_and_screening_blocks() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_static_modifier_relation_scopes_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    write(
        &root,
        &[(
            "common/modifiers/a.txt",
            concat!(
                "weather_rain_light={naval_strike=-0.2 naval_retreat_speed=0.05}\n",
                "screening_bonus={sortie_efficiency=0.1}\n",
                "capital_screening_bonus={fighter_sortie_efficiency=0.2}\n",
                "relation={valid_relation_trigger={FROM={NOT={has_war_with=ROOT}}}}\n",
                "weather_snow={naval_speed_factor=-0.1}\n",
            ),
        )],
    );
    let layers = ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
    let blocks = read_blocks(&layers, &["weather_rain_light", "weather_snow"]).unwrap();
    assert!(blocks.notes.is_empty(), "{:?}", blocks.notes);
    assert_eq!(
        blocks.blocks[0]
            .as_ref()
            .unwrap()
            .values
            .iter()
            .map(|value| (value.name.as_str(), value.value))
            .collect::<Vec<_>>(),
        [("naval_strike", -20_000), ("naval_retreat_speed", 5_000)]
    );
    assert_eq!(blocks.blocks[1].as_ref().unwrap().values[0].value, -10_000);
    assert_eq!(
        read(&layers, &SCREENING).unwrap().values,
        [Ok(10_000), Ok(20_000)]
    );
    assert_eq!(
        read_identifiers(&layers, &["weather_rain_light"])
            .unwrap()
            .identifiers,
        [Ok(Some("weather_rain_light".into()))]
    );
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_stack_is_read_without_nested_files_and_honours_replacement() {
    content_reader_script::test_game::enter();
    let root = std::env::temp_dir().join(format!(
        "hoi4_content_reader_static_modifiers_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&root);
    let (base, over, replacing) = (root.join("base"), root.join("over"), root.join("rep"));
    write(
        &base,
        &[
            (
                "common/modifiers/00_static_modifiers.txt",
                "screening_bonus = { sortie_efficiency = 0.1 }
                 capital_screening_bonus = { fighter_sortie_efficiency = 0.2 }",
            ),
            (
                "common/modifiers/nested/zz.txt",
                "screening_bonus = { sortie_efficiency = 0.9 }",
            ),
        ],
    );
    write(
        &over,
        &[
            (
                "common/modifiers/00_static_modifiers.txt",
                "capital_screening_bonus = { fighter_sortie_efficiency = 0.25 }",
            ),
            (
                "common/modifiers/zz_mod.txt",
                "screening_bonus = { sortie_efficiency = 0.3 }",
            ),
            (
                "common/modifiers/zz_mod/zzz.txt",
                "screening_bonus = { sortie_efficiency = 0.7 }",
            ),
        ],
    );
    write(
        &replacing,
        &[
            (
                "common/modifiers/only.txt",
                "capital_screening_bonus = { fighter_sortie_efficiency = 0.4 }",
            ),
            (
                "common/modifiers/deeper/zz.txt",
                "screening_bonus = { sortie_efficiency = 0.8 }",
            ),
        ],
    );
    let stack = |layers: &[(&Path, &[&str])]| ResolvedLayers::from_folders(layers).unwrap();
    let none: &[&str] = &[];
    let replaced: &[&str] = &["common/modifiers"];
    let values = |layers: &ResolvedLayers| read(layers, &SCREENING).unwrap().values;
    let layers = stack(&[(&base, none)]);
    assert_eq!(values(&layers), [Ok(10_000), Ok(20_000)]);
    let layers = stack(&[(&base, none), (&over, none)]);
    assert_eq!(values(&layers), [Ok(30_000), Ok(25_000)]);
    let layers = stack(&[(&base, none), (&over, none), (&replacing, replaced)]);
    assert_eq!(values(&layers), [Ok(0), Ok(40_000)]);
    fs::remove_dir_all(&root).unwrap();
}
