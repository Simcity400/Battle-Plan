use super::*;
use replay_data::rows::Recorded;
use viewer_runtime::diagnostics_panel::test_support::assert_diagnostics;
use viewer_native_gui::test_support::gui as tests;
use viewer_foundation::tooltip_text::NoIcons;

#[test]
fn naval_trait_hovers_refuse_each_shown_entry_by_name_and_clear_authored_seeds() {
    content_reader_script::test_game::enter();
    use viewer_native_gui::test_support::gui::windows;
    let windows = windows(
        br#"guiTypes={
        containerWindowType={name=entry buttonType={name=Background pdx_tooltip=SEED}
            iconType={name=mod_child pdx_tooltip=OWN}}
        containerWindowType={name=exile iconType={name=flag pdx_tooltip=SEED}
            iconType={name=mod_child pdx_tooltip_delayed=SEED}}}"#,
    );
    let traits: Vec<_> = [
        ("repeat", 1, 1),
        ("hidden", 0, 1),
        ("repeat", 1, 1),
        ("irrelevant", 1, 0),
        ("exile", 6, 1),
    ]
    .into_iter()
    .map(|(name, kind, relevant)| LandCombatCommanderTrait {
        token: name.as_bytes().to_vec(),
        kind,
        relevant,
        ..Default::default()
    })
    .collect();
    let mut entry = Values::new();
    entry.insert(
        "Background".into(),
        super::super::Widget {
            tooltip: Some("Seed".into()),
            tooltip_description: Some("Body".into()),
            ..Default::default()
        },
    );
    entry.insert(
        "mod_child".into(),
        super::super::Widget {
            tooltip: Some("Own".into()),
            ..Default::default()
        },
    );
    let mut out = Bindings::default();
    out.values.insert(
        "traits".into(),
        super::super::Widget {
            entry_template: Some("entry".into()),
            entry_templates: [(2, "exile".into())].into_iter().collect(),
            entries: vec![entry.clone(), entry, Values::new()],
            ..Default::default()
        },
    );
    let content = Content {
        leaders: Arc::new(Leaders::from_files(Vec::new(), &|_| None)),
        terrain: Vec::new(),
        settled: Mutex::new(None),
        advisor_ranks: Err("unused".into()),
        proximity: Err("unused".into()),
        sub_units: Err("unused".into()),
        leader_traits: Err("unused".into()),
    };
    let loc = Localisation::default();
    let definitions = Definitions::from_parts(&[], &[], None);
    let context = EffectContext {
        icons: &NoIcons,
        loc: &loc,
        definitions: Ok(&definitions),
        traits: &content.leaders.traits,
        proximity: Err("unused"),
        viewing_tag: Ok("SYN"),
        sub_units: Err("unused"),
    };
    let timed = Recorded::<Vec<LandCombatCommanderTimedTrait>>::NotRecorded;
    let bind = |out: &mut Bindings, key: &str| {
        assigned(
            out,
            &content,
            &windows,
            key,
            AssignedCommander {
                traits: &traits,
                context: 2,
                timed_traits: timed
                    .require("NavalCommanderInputs.timed_traits")
                    .map(Vec::as_slice),
                country_tag: Ok("SYN"),
                advisor_feature: Some(false),
                skill: None,
                advisor_roles: Err("unused".into()),
            },
            &context,
        );
    };
    bind(&mut out, "traits");
    let entries = &out.values["traits"].entries;
    for entry in &entries[..2] {
        assert!(entry["Background"].hover_owned);
        assert!(entry["Background"].tooltip.is_none());
        assert!(entry["Background"].tooltip_description.is_none());
        assert_eq!(entry["mod_child"].tooltip.as_deref(), Some("Own"));
        assert!(!entry["mod_child"].hover_owned);
    }
    assert!(entries[2].contains_key("mod_child"));
    assert!(entries[2].values().all(|value| {
        value.hover_owned && value.tooltip.is_none() && value.tooltip_description.is_none()
    }));
    let reason = |name| {
        format!(
            "Combat trait hover traits: the hover of trait {name}: NavalCommanderInputs.timed_traits not recorded is omitted"
        )
    };
    assert_eq!(out.refused().len(), 2);
    assert_eq!(out.refused()[&reason("repeat")], 2);
    assert_eq!(out.refused()[&reason("exile")], 1);
    assert!(out.warnings.is_empty());
    bind(&mut out, "missing");
    out.values.get_mut("traits").unwrap().entries.truncate(2);
    bind(&mut out, "traits");
    assert_eq!(out.warnings.len(), 2);
    assert!(out.warnings[0].contains("the trait grid is not bound"));
    assert!(out.warnings[1].contains("the grid has 2 entries for 3 shown traits"));

    let scope = super::super::Scope {
        branch: 0,
        hour: 10,
        selection: Selection::Naval(7),
        perspective: Default::default(),
        viewer: 1,
        playing: false,
        nato: false,
    };
    let mut window = super::super::Window {
        selection: Some(scope.selection),
        cached: Some(super::super::Cached {
            at: scope.clone(),
            covered: true,
            contents: Some((scope, out)),
            shown: true,
            held: None,
            generation: 1,
            gui_assets: None,
            of: None,
        }),
        ..Default::default()
    };
    let warnings = window.warnings(&[]);
    assert!(warnings.contains(&format!("{} (4 times)", reason("repeat"))));
    assert!(warnings.contains(&reason("exile")));
    assert_eq!(window.warnings(&[]), warnings);
    assert_diagnostics(|| window.warnings(&[]), &warnings);
    window.close();
    assert!(window.warnings(&[]).is_empty());
}

#[test]
fn naval_component_skill_icons_and_text_share_native_tooltips() {
    let text = ["attack", "defense", "maneuvering", "coordination"]
        .into_iter()
        .map(|name| {
            format!(
                "leader_{name}_skills = {{ 3 = {{ type = navy modifier = {{ naval_damage_factor = 0.15 hidden_modifier = {{ navy_org = 0.9 }} custom_modifier_tooltip = SYN_AUTHORED }} }} }}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let read = Leaders::from_files(vec![("skills.txt".into(), text.into_bytes())], &|_| None);
    let loc = [
        ("LEADER_SKILL_DESC", "Skill: $S|H$"),
        ("SYN_DAMAGE", "Synthetic naval damage"),
        ("SYN_HIDDEN", "Synthetic hidden effect"),
        ("SYN_AUTHORED", "Authored skill text"),
        ("SYN_EXTRA", "Synthetic skill text"),
        ("SYN_LAST", "Recorded last line"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let rows = [
        replay_data::rows::ModifierDefinitionRow {
            index: 1,
            name_key: b"SYN_DAMAGE".to_vec(),
            flags: 0b11,
            category_mask: 1,
            precision: 2,
            ..Default::default()
        },
        replay_data::rows::ModifierDefinitionRow {
            index: 2,
            name_key: b"SYN_HIDDEN".to_vec(),
            flags: 0b11,
            category_mask: 1,
            precision: 2,
            ..Default::default()
        },
    ];
    let definitions = Definitions::from_parts(
        &rows,
        &[(b"naval_damage_factor", 1), (b"navy_org", 2)],
        None,
    );
    let mut commander = NavalCommanderInputs {
        commander: 17,
        skills: [4, 7, 7, 7, 7],
        ..Default::default()
    };
    for index in 1..=4 {
        commander.skill_definitions[index] = replay_data::rows::CombatCommanderSkillDefinition {
            present: 1,
            valid: 1,
            level: 3,
            category: 2,
            ..Default::default()
        };
        for (id, value) in [(1, 20_000), (2, 90_000)] {
            commander
                .modifiers
                .push(replay_data::rows::LandCombatCommanderPortraitModifier {
                    source: index as u8,
                    id,
                    value,
                    ..Default::default()
                });
        }
        for (bucket, key) in [(8, "SYN_LAST"), (1, "SYN_EXTRA")] {
            commander
                .tooltips
                .push(replay_data::rows::LandCombatCommanderPortraitTooltip {
                    source: index as u8,
                    bucket,
                    text: key.as_bytes().to_vec(),
                    ..Default::default()
                });
        }
    }
    commander.timed_traits = Recorded::NotRecorded;
    let prefix = "stats";
    let mut out = Bindings::default();
    naval_skills(
        &mut out,
        &NoIcons,
        &loc,
        &read.skills,
        Ok(&definitions),
        &commander,
        prefix,
    );
    assert!(out.refused().is_empty(), "{:?}", out.refused());
    for stat in ["attack", "defense", "maneuvering", "coordination"] {
        for kind in ["icon", "text"] {
            let widget = &out.values[&format!("{prefix}/leader_stat_{stat}_{kind}")];
            assert_eq!(
                widget.tooltip.as_deref(),
                Some(concat!(
                    "Skill: \u{11}H7\u{11}!\n",
                    "Synthetic naval damage: \u{11}G+20.00%\u{11}!\n",
                    "Synthetic skill text\nRecorded last line"
                ))
            );
            assert!(widget.tooltip_description.is_none());
        }
    }
    commander.skill_definitions[1].valid = 0;
    naval_skills(
        &mut out,
        &NoIcons,
        &Localisation::default(),
        &read.skills,
        Ok(&definitions),
        &commander,
        prefix,
    );
    assert_eq!(
        out.values["stats/leader_stat_attack_icon"]
            .tooltip
            .as_deref(),
        Some("LEADER_SKILL_DESC")
    );
    assert!(out.refused().is_empty(), "{:?}", out.refused());
}

fn modifiers(entries: &[(&str, &str)]) -> content_reader_script::modifiers::Modifiers {
    let items: Vec<_> = entries
        .iter()
        .map(|(key, value)| content_reader_parser::parser::Item {
            key: Some(key.as_bytes().to_vec()),
            key_quoted: false,
            operator: content_reader_parser::parser::Operator::Equals,
            value: content_reader_parser::parser::Value::Scalar(value.as_bytes().to_vec()),
            kind: content_reader_parser::parser::Kind::Bare,
        })
        .collect();
    content_reader_script::modifiers::values(&items)
}

fn exile_localisation() -> Localisation {
    [
        ("HEADER", "$KEY|H$\n"),
        ("displaced_officer", "Exiled Leader"),
        (
            "displaced_officer_desc",
            "A displaced officer keeps fighting.",
        ),
        ("EXILE_ATTACK", "Exiled Divisions Attack"),
        ("OWN_EXILE_ATTACK", "Own Exiled Divisions Attack"),
        ("EXILE_DEFENSE", "Exiled Divisions Defense"),
        ("OWN_EXILE_DEFENSE", "Own Exiled Divisions Defense"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect()
}

fn exile_assigned(
    files: Vec<(String, Vec<u8>)>,
    loc: &Localisation,
    windows: &NativeWindows,
) -> Bindings {
    exile_assigned_timed(files, loc, windows, Ok(&[]))
}

fn exile_assigned_timed(
    files: Vec<(String, Vec<u8>)>,
    loc: &Localisation,
    windows: &NativeWindows,
    timed: Result<&[LandCombatCommanderTimedTrait], String>,
) -> Bindings {
    let leaders = Leaders::from_files(files, &|_| None);
    let content = Content {
        leaders: Arc::new(leaders),
        terrain: Vec::new(),
        settled: Mutex::new(None),
        advisor_ranks: Err("unused".into()),
        proximity: Err("unused".into()),
        sub_units: Err("unused".into()),
        leader_traits: Err("unused".into()),
    };
    let names = [
        (b"exile_attack".as_slice(), "EXILE_ATTACK"),
        (b"own_exile_attack", "OWN_EXILE_ATTACK"),
        (b"exile_defense", "EXILE_DEFENSE"),
        (b"own_exile_defense", "OWN_EXILE_DEFENSE"),
    ];
    let rows: Vec<_> = names
        .iter()
        .enumerate()
        .map(
            |(index, (_, key))| replay_data::rows::ModifierDefinitionRow {
                index: index as u32 + 1,
                token: index as u32 + 1,
                name_key: key.as_bytes().to_vec(),
                flags: 0b11,
                category_mask: 1,
                precision: 2,
                ..Default::default()
            },
        )
        .collect();
    let names: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, (name, _))| (*name, index as u32 + 1))
        .collect();
    let definitions = Definitions::from_parts(&rows, &names, None);
    let context = EffectContext {
        icons: &NoIcons,
        loc,
        definitions: Ok(&definitions),
        traits: &content.leaders.traits,
        proximity: Err("unused"),
        viewing_tag: Ok("SYN"),
        sub_units: Err("unused"),
    };
    let traits = [LandCombatCommanderTrait {
        kind: 6,
        relevant: 1,
        token: b"displaced_officer".to_vec(),
        ..Default::default()
    }];
    let mut out = Bindings::default();
    out.values.insert(
        "traits".into(),
        crate::Widget {
            entry_template: Some("unit_leader_trait_entry".into()),
            entry_templates: [(0, "unit_leader_exile_trait_entry".into())].into(),
            entries: vec![Values::new()],
            ..Default::default()
        },
    );
    assigned(
        &mut out,
        &content,
        windows,
        "traits",
        AssignedCommander {
            traits: &traits,
            context: 4,
            timed_traits: timed,
            country_tag: Ok("SYN"),
            advisor_feature: Some(false),
            skill: None,
            advisor_roles: Err("unused".into()),
        },
        &context,
    );
    out
}

fn exile_files() -> Vec<(String, Vec<u8>)> {
    vec![(
        "base.txt".into(),
        b"leader_traits = { displaced_officer = { trait_type = exile modifier = {
            own_exile_defense = 0.05 exile_defense = 0.1
            own_exile_attack = 0.05 exile_attack = 0.1
        } } }"
            .to_vec(),
    )]
}

fn exile_windows() -> NativeWindows {
    tests::windows(
        br#"guiTypes = { containerWindowType = {
            name = unit_leader_exile_trait_entry size = { width = 24 height = 32 }
            iconType = { name = exile_flag spriteType = GFX_even }
            iconType = { name = exile_flag_frame spriteType = GFX_even alwaystransparent = yes }
            containerWindowType = { name = mod_overlay size = { width = 10 height = 10 }
                iconType = { name = mark spriteType = GFX_odd }
            }
        } }"#,
    )
}

#[test]
fn exile_badge_hover_uses_mod_values_and_counts_named_refusals() {
    content_reader_script::test_game::enter();
    let windows = exile_windows();
    let mut files = exile_files();
    files.push((
        "mod.txt".into(),
        b"leader_traits = { displaced_officer = { modifier = {
        exile_attack = 0.175 own_exile_defense = -0.035
    } } }"
            .to_vec(),
    ));
    let loc: Localisation = exile_localisation()
        .iter()
        .chain([
            ("SYN_displaced_officer", "Displaced Captain"),
            ("SYN_displaced_officer_desc", "Mod description."),
        ])
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    let out = exile_assigned(files.clone(), &loc, &windows);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let tooltip = &out.values["traits"].entries[0]["exile_flag"];
    let text = tooltip.tooltip.as_deref().unwrap();
    assert!(text.starts_with("\u{11}HDisplaced Captain\u{11}!\n\n"));
    assert!(text.contains("Exiled Divisions Attack: \u{11}G+17.50%\u{11}!"));
    assert!(text.contains("Own Exiled Divisions Defense: \u{11}R-3.50%\u{11}!"));
    assert_eq!(
        tooltip.tooltip_description.as_deref(),
        Some("Mod description.")
    );

    let missing: Localisation = loc
        .iter()
        .filter(|(key, _)| *key != "HEADER")
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    let out = exile_assigned(files, &missing, &windows);
    assert!(
        out.values["traits"].entries[0]
            .values()
            .all(|widget| widget.hover_owned
                && widget.tooltip.is_none()
                && widget.tooltip_description.is_none())
    );
    assert_eq!(out.refused().len(), 1, "{:?}", out.refused());
    assert!(
        out.refused()
            .iter()
            .any(|(reason, count)| reason.contains("HEADER") && *count == 1),
        "{:?}",
        out.refused()
    );

    let out = exile_assigned(
        exile_files(),
        &exile_localisation(),
        &tests::windows(b"guiTypes = {}"),
    );
    assert_eq!(out.refused().len(), 1, "{:?}", out.refused());
    assert!(
        out.refused().iter().any(|(reason, count)| {
            reason.contains("unit_leader_exile_trait_entry") && *count == 1
        }),
        "{:?}",
        out.refused()
    );
}

#[test]
fn the_settled_cache_holds_the_index_it_was_read_with() {
    let text = b"leader_traits = { t = { modifier = { army_org = { y = 0.2 } x = 0.1 } } }";
    let leaders = Leaders::from_files(vec![("a.txt".into(), text.to_vec())], &|_| None);
    assert!(leaders.conditional());
    let content = Content {
        leaders: Arc::new(leaders),
        terrain: Vec::new(),
        settled: Mutex::new(None),
        advisor_ranks: Err("unused".into()),
        proximity: Err("unused".into()),
        sub_units: Err("unused".into()),
        leader_traits: Err("unused".into()),
    };
    let rows = [replay_data::rows::ModifierDefinitionRow::default()];
    let first = Definitions::from_parts(&rows, &[(&b"army_org"[..], 1)], None);
    let settled = content.settled(Ok(&first));
    assert!(!settled.conditional());
    assert_eq!(Arc::strong_count(&first.identity()), 3);
    assert!(Arc::ptr_eq(&settled, &content.settled(Ok(&first))));
    let second = Definitions::from_parts(&rows, &[(&b"other"[..], 1)], None);
    let other = content.settled(Ok(&second));
    assert!(!Arc::ptr_eq(&settled, &other));
    drop(second);
    let again = content.settled(Ok(&first));
    assert!(!Arc::ptr_eq(&again, &other));
    assert!(!again.conditional());
}

#[test]
fn learning_ratio_preserves_native_sentinel_and_integer_arithmetic() {
    assert_eq!(ratio(5_000_000, 100), Some(50_000));
    assert_eq!(ratio(5_000_001, 100), Some(50_000));
    assert_eq!(ratio(0, 0), Some(0xffff_ffff));
    assert_eq!(ratio(i64::MAX, 1), Some(0xffff_ffff));
}

#[test]
fn effects_follow_the_native_block_adjuster_skill_and_factor_order() {
    content_reader_script::test_game::enter();
    let loc: Localisation = [
        ("MODIFIER_ATTACK", "Division Attack"),
        ("forest", "Forest"),
        ("STAT_ADJUSTER_MOVEMENT", "Movement: "),
        ("STAT_ADJUSTER_MOVEMENT_DIFF", "$VALUE|=+1%$"),
        ("STAT_ADJUSTER_ATTACK", "Attack: "),
        ("STAT_ADJUSTER_ATTACK_DIFF", "$VALUE|=+1%$"),
        ("trait_bonus_attack", "Attack: $VAL|+=0$"),
        ("LEADER_PROXIMITY_CLOSE_TITLE", "Close"),
        (
            "trait_bonus_leader_default_proximity_offset",
            "Starting Proximity: $NAME$",
        ),
        ("infantry_leader", "Infantry Leader"),
        ("cruiser", "Cruiser"),
        ("destroyer", "Destroyer"),
        ("trait_xp_factor", "$NAME|H$ experience factor: $VAL|H%.0$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let rows = [
        replay_data::rows::ModifierDefinitionRow::default(),
        replay_data::rows::ModifierDefinitionRow {
            index: 1,
            token: 9,
            name_key: b"MODIFIER_ATTACK".to_vec(),
            flags: 0b11,
            category_mask: 1,
            precision: 1,
            ..Default::default()
        },
    ];
    let definitions = Definitions::from_parts(&rows, &[(&b"army_attack_factor"[..], 1)], None);
    let mut order = SubUnits::default();
    order.read(b"sub_units = { cruiser = {} destroyer = {} }", "units.txt");
    let traits = Traits::default();
    let context = EffectContext {
        icons: &NoIcons,
        loc: &loc,
        definitions: Ok(&definitions),
        traits: &traits,
        proximity: Ok([2, 3, 5]),
        viewing_tag: Ok("GER"),
        sub_units: Ok(&order),
    };
    let mut definition = TraitEffects {
        skills: [1, 0, 0, 0, 0, 0],
        proximity_offset: 1,
        experience_factors: vec![("infantry_leader".into(), 100_000)],
        adjusters: vec![Adjuster {
            key: "forest".into(),
            attack: 10_000,
            movement: 5_000,
            ..Adjuster::default()
        }],
        ..TraitEffects::default()
    };
    definition.blocks[1] = modifiers(&[
        ("army_attack_factor", "0.05"),
        ("unknown_modifier", "0.00001"),
    ]);
    let mut dropped = Vec::new();
    assert_eq!(
        effects(&context, &definition, &mut dropped).unwrap(),
        "Division Attack: \u{11}G+5.0%\u{11}!\n\u{11}HForest\u{11}!\nMovement: \u{11}G+5.0%\u{11}!\nAttack: \u{11}G+10.0%\u{11}!\nAttack: \u{11}G+1\u{11}!\nStarting Proximity: Close\n\u{11}HInfantry Leader\u{11}! experience factor: \u{11}H100%\u{11}!\n"
    );
    assert_eq!(dropped, [Dropped::Modifier("unknown_modifier".into())]);
    definition.override_tooltips = vec!["forest".into()];
    assert_eq!(
        effects(&context, &definition, &mut dropped).unwrap(),
        "Forest\n"
    );
    definition.override_tooltips.clear();
    definition.unsupported = vec!["a script value".into()];
    assert!(effects(&context, &definition, &mut dropped).is_err());
    definition.unsupported.clear();
    let condition = |modifier: &str| content_reader_key_names::unit_leader::Conditional {
        modifier: modifier.into(),
        place: "file.txt trait t modifier".into(),
    };
    definition.conditions = vec![condition("no_such_modifier")];
    assert!(effects(&context, &definition, &mut dropped).is_ok());
    definition.conditions = vec![condition("ARMY_ATTACK_FACTOR")];
    let refused = effects(&context, &definition, &mut dropped).unwrap_err();
    assert!(
        matches!(&refused, EffectRefusal::OutOfStep(reason) if reason.ends_with("and ARMY_ATTACK_FACTOR is a recorded modifier definition")),
        "{refused:?}"
    );
    definition.conditions.clear();
    let sub_unit = |key: &str, attack: i64, entries: &[(&str, &str)]| {
        content_reader_key_names::unit_leader::SubUnitModifiers {
            adjuster: Adjuster {
                key: key.into(),
                attack,
                ..Adjuster::default()
            },
            block: modifiers(entries),
        }
    };
    let only_sub_units = TraitEffects {
        sub_units: vec![
            sub_unit("destroyer", 0, &[("army_attack_factor", "0.05")]),
            sub_unit("cruiser", 10_000, &[]),
            sub_unit("CRUISER", 20_000, &[]),
            sub_unit("submarine", 10_000, &[]),
        ],
        ..TraitEffects::default()
    };
    dropped.clear();
    assert_eq!(
        effects(&context, &only_sub_units, &mut dropped).unwrap(),
        "\u{11}HCruiser\u{11}!\nAttack: \u{11}G+10.0%\u{11}!\n\u{11}HDestroyer\u{11}!\n  Division Attack: \u{11}G+5.0%\u{11}!\n"
    );
    assert_eq!(
        dropped,
        [
            Dropped::Repeat("CRUISER".into()),
            Dropped::SubUnit("submarine".into())
        ]
    );
    let context = EffectContext {
        sub_units: Err("common/units unreadable"),
        ..context
    };
    assert!(matches!(
        effects(&context, &only_sub_units, &mut dropped),
        Err(EffectRefusal::SubUnitOrder(_))
    ));
}

#[test]
fn the_portrait_hover_names_what_it_lacks() {
    let loc: Localisation = [
        ("HEADER", "$KEY$\n"),
        ("rommel", "Rommel"),
        ("rommel_desc", "Desert fox"),
        ("cc_leader", "General"),
        ("FLAGGED", "@GER"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(
        portrait(
            &NoIcons,
            &loc,
            b"rommel",
            b"rommel_desc",
            (1, 4),
            None,
            false
        ),
        Ok(("General Rommel\n".into(), "Desert fox".into()))
    );
    assert_eq!(
        portrait(
            &NoIcons,
            &loc,
            b"rommel",
            b"rommel_desc",
            (1, 4),
            None,
            true
        ),
        Ok((
            "General Rommel\n\nLEADER_HAS_INITIATIVE".into(),
            "Desert fox".into()
        ))
    );
    assert!(
        portrait(
            &NoIcons,
            &loc,
            b"FLAGGED",
            b"rommel_desc",
            (1, 4),
            None,
            false
        )
        .unwrap_err()
        .starts_with("the commander's name: localisation FLAGGED holds the inline flag 0x12")
    );
    let learning = [LandCombatCommanderLearningTrait {
        token: b"desert_fox".to_vec(),
        experience: 90,
        required_experience: 1,
        ..Default::default()
    }];
    assert_eq!(
            portrait(
                &NoIcons,
                &loc,
                b"rommel",
                b"rommel_desc",
                (1, 4),
                Some((Learning::Raw(&learning, 0), Err("the viewing country has no Countries row"))),
                false
            ),
            Err("desert_fox's name needs its country's TAG_desert_fox key, but the viewing country has no Countries row".into())
        );
}

#[test]
fn the_land_portrait_lists_the_recorded_rows_and_refuses_only_without_them() {
    let loc: Localisation = [
        ("HEADER", "$KEY$\n"),
        ("rommel", "Rommel"),
        ("cc_leader", "General"),
        ("trait_engineer", "Engineer"),
        ("LEARNING_TRAITS_TITLE", "Learning Traits:"),
        ("LEARNING_TRAITS", "$NAME$ ($PERC|%0H$ done)"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let shown = [LandCombatLearningProgress {
        token: b"trait_engineer".to_vec(),
        progress: 57_000,
        ..Default::default()
    }];
    let tag: Result<&str, &str> = Ok("GER");
    let portrait = |own: Result<bool, String>,
                    learning: Result<&[LandCombatLearningProgress], &str>| {
        land_portrait(&NoIcons, &loc, b"rommel", (1, 4), own, learning, tag, false)
    };
    assert_eq!(
        portrait(Ok(false), Ok(&shown[..])),
        Ok("General Rommel\n".into())
    );
    assert_eq!(
        portrait(Ok(true), Ok(&shown[..])),
        Ok("General Rommel\n\nLearning Traits:\nEngineer (\u{11}H57%\u{11}! done)\n".into())
    );
    assert_eq!(
        portrait(Ok(true), Ok(&[][..])),
        Ok("General Rommel\n".into())
    );
    assert_eq!(
        portrait(Err("no link".into()), Ok(&[][..])),
        Ok("General Rommel\n".into())
    );
    assert!(
        portrait(Err("no link".into()), Ok(&shown[..]))
            .unwrap_err()
            .ends_with("is unknown: no link")
    );
    for reason in [LAND_LEARNING_UNRECORDED, LAND_LEARNING_UNREAD] {
        assert_eq!(portrait(Ok(true), Err(reason)), Err(reason.to_owned()));
        assert_eq!(
            portrait(Ok(false), Err(reason)),
            Ok("General Rommel\n".into())
        );
    }
}

#[test]
fn commander_heading_uses_localised_native_roles_and_operative_fallback() {
    let loc: Localisation = [
        ("HEADER", "$KEY$\n"),
        ("fm_leader", "Senior Commander"),
        ("cc_leader", "Army Commander"),
        ("navy_leader", "Fleet Commander"),
        ("operative_skill_7", "Special Agent"),
        ("OPERATIVE", "Agent"),
        ("synthetic_name", "Example Person"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    for (category, skill, title) in [
        (0, 7, "Senior Commander"),
        (1, 7, "Army Commander"),
        (2, 7, "Fleet Commander"),
        (3, 7, "Special Agent"),
        (3, 8, "Agent"),
        (4, 7, ""),
    ] {
        let (heading, _) = portrait(
            &NoIcons,
            &loc,
            b"synthetic_name",
            b"",
            (category, skill),
            None,
            false,
        )
        .unwrap();
        assert_eq!(heading, format!("{title} Example Person\n"));
    }
}

#[test]
fn timed_trait_uses_the_first_matching_entry_and_nonnegative_days() {
    let item = LandCombatCommanderTrait {
        token_id: 7,
        definition_ordinal: 15,
        ..Default::default()
    };
    let entry = |definition_ordinal, remaining_days| LandCombatCommanderTimedTrait {
        token_id: 7,
        definition_ordinal,
        remaining_days,
    };
    assert_eq!(timed_days(&[entry(15, -1), entry(15, 9)], &item), None);
    assert_eq!(timed_days(&[entry(15, 0), entry(15, 9)], &item), Some(0));
    assert_eq!(timed_days(&[entry(16, 3)], &item), None);
}

#[test]
fn naval_timed_trait_hover_shows_zero_days_and_refuses_old_format_absence() {
    content_reader_script::test_game::enter();
    let loc: Localisation = exile_localisation()
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([("TRAIT_WILL_EXPIRE".into(), "Expires in $DAYS$ days.".into())])
        .collect();
    let windows = exile_windows();
    let mut commander = replay_data::rows::NavalCommanderInputs {
        timed_traits: Recorded::Value(vec![
            LandCombatCommanderTimedTrait {
                remaining_days: 0,
                ..Default::default()
            },
            LandCombatCommanderTimedTrait {
                remaining_days: 19,
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let out = exile_assigned_timed(
        exile_files(),
        &loc,
        &windows,
        commander
            .timed_traits
            .require("NavalCommanderInputs.timed_traits")
            .map(Vec::as_slice),
    );
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    assert_eq!(
        out.values["traits"].entries[0]["exile_flag"]
            .tooltip_description
            .as_deref(),
        Some("A displaced officer keeps fighting.\nExpires in 0 days.")
    );
    commander
        .timed_traits
        .require_mut("NavalCommanderInputs.timed_traits")
        .unwrap()[0]
        .remaining_days = -1;
    let out = exile_assigned_timed(
        exile_files(),
        &loc,
        &windows,
        commander
            .timed_traits
            .require("NavalCommanderInputs.timed_traits")
            .map(Vec::as_slice),
    );
    assert_eq!(
        out.values["traits"].entries[0]["exile_flag"]
            .tooltip_description
            .as_deref(),
        Some("A displaced officer keeps fighting.")
    );
    commander.timed_traits = Recorded::NotRecorded;
    let out = exile_assigned_timed(
        exile_files(),
        &loc,
        &windows,
        commander
            .timed_traits
            .require("NavalCommanderInputs.timed_traits")
            .map(Vec::as_slice),
    );
    assert!(
        out.values["traits"].entries[0]["exile_flag"]
            .tooltip
            .is_none()
    );
    assert!(out.refused().iter().any(|(reason, count)| {
        reason.contains("NavalCommanderInputs.timed_traits not recorded") && *count == 1
    }));
}

#[test]
fn naval_bind_uses_live_and_result_grid_paths_and_skips_null_commanders() {
    content_reader_script::test_game::enter();
    use replay_data::{
        rows::CountryLinkRow, rows::NavalCombatDetailsRow, rows::NavalCombatResultDetailsRow,
        rows::NavalCombatSideDetails, rows::NavalResultSideDetails, rows::SimulationFeatureRow,
        rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    let loc: Localisation = exile_localisation()
        .iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .chain([
            ("TRAIT_WILL_EXPIRE".into(), "Expires in $DAYS$ days.".into()),
            (
                "TRAIT_NOT_ACTIVE_WHEN_COMMANDING_AN_ARMY".into(),
                "Inactive army trait.".into(),
            ),
        ])
        .collect();
    let windows = exile_windows();
    let commanders = Content {
        leaders: Arc::new(Leaders::from_files(
            vec![(
                "synthetic.txt".into(),
                b"leader_traits = { displaced_officer = { type = field_marshal } }".to_vec(),
            )],
            &|_| None,
        )),
        terrain: Vec::new(),
        settled: Mutex::new(None),
        advisor_ranks: Err("unused".into()),
        proximity: Err("unused".into()),
        sub_units: Err("unused".into()),
        leader_traits: Err("unused".into()),
    };
    let cache = super::super::modifier_text::name_index_cache();
    let keys = [
        "header_container/traits_grid_attacker",
        "header_container/traits_grid_defender",
        "attacker_leader/traits_grid",
        "defender_leader/traits_grid",
    ];
    for (null, timed_traits) in [
        (
            false,
            Recorded::Value(vec![LandCombatCommanderTimedTrait::default()]),
        ),
        (false, Recorded::Value(Vec::new())),
        (false, Recorded::NotRecorded),
        (true, Recorded::NotRecorded),
    ] {
        let historical = matches!(timed_traits, Recorded::NotRecorded);
        let expires = timed_traits
            .require("NavalCommanderInputs.timed_traits")
            .is_ok_and(|entries| !entries.is_empty());
        let commander = replay_data::rows::NavalCommanderInputs {
            commander: if null { 0 } else { (7 << 32) | 4713 },
            country: 1,
            traits: vec![LandCombatCommanderTrait {
                kind: 1,
                relevant: 1,
                token: b"displaced_officer".to_vec(),
                ..Default::default()
            }],
            timed_traits,
            ..Default::default()
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            }]));
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![
                SimulationFeatureRow::default(),
            ]));
        *frame.tables[TableId::NavalCombatDetails.index()] =
            TableView::Complete(TableRows::NavalCombatDetails(vec![NavalCombatDetailsRow {
                combat: 7,
                sides: std::array::from_fn(|_| NavalCombatSideDetails {
                    commander_inputs: Some(commander.clone()),
                    ..Default::default()
                }),
                ..Default::default()
            }]));
        *frame.tables[TableId::NavalCombatResultDetails.index()] =
            TableView::Complete(TableRows::NavalCombatResultDetails(vec![
                NavalCombatResultDetailsRow {
                    result: 9,
                    sides: std::array::from_fn(|_| NavalResultSideDetails {
                        commander_context: Some(commander.clone()),
                        ..Default::default()
                    }),
                },
            ]));
        for (selection, selected) in [
            (Selection::Naval(7), 0..2),
            (Selection::NavalResult(9), 2..4),
        ] {
            let mut out = Bindings::default();
            for key in keys {
                out.values.insert(
                    key.into(),
                    crate::Widget {
                        entry_template: Some("unit_leader_trait_entry".into()),
                        entries: vec![Values::from([(
                            "Background".into(),
                            crate::Widget {
                                tooltip_description: Some("untouched".into()),
                                ..Default::default()
                            },
                        )])],
                        ..Default::default()
                    },
                );
            }
            bind(
                &mut out,
                BindContent {
                    icons: &NoIcons,
                    loc: &loc,
                    commanders: &commanders,
                    windows: &windows,
                    modifier_names: &cache,
                },
                &frame,
                selection,
                &Vision::sharing(1, [1]),
            )
            .unwrap();
            assert!(out.warnings.is_empty(), "{:?}", out.warnings);
            if historical && !null {
                assert_eq!(out.refused().len(), 2);
                assert!(out.refused().iter().all(|(reason, count)| {
                    reason.contains("NavalCommanderInputs.timed_traits not recorded") && *count == 1
                }));
            } else {
                assert!(out.refused().is_empty(), "{:?}", out.refused());
            }
            for (index, key) in keys.iter().enumerate() {
                let description = out.values[*key].entries[0]["Background"]
                    .tooltip_description
                    .as_deref();
                if selected.contains(&index) && !null {
                    let text = out.values[*key].entries[0]["Background"].tooltip.as_deref();
                    if historical {
                        assert!(text.is_none());
                    } else {
                        assert_eq!(
                            text,
                            Some("\u{11}HExiled Leader\u{11}!\n\nInactive army trait.")
                        );
                    }
                    let expected = if historical {
                        None
                    } else if expires {
                        Some("A displaced officer keeps fighting.\nExpires in 0 days.")
                    } else {
                        Some("A displaced officer keeps fighting.")
                    };
                    assert_eq!(description, expected);
                } else {
                    assert_eq!(description, Some("untouched"));
                }
            }
        }
    }
}

#[test]
fn advisor_traits_resolve_through_the_country_leader_traits() {
    let root = std::env::temp_dir().join(format!(
        "viewer_advisor_traits_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let file = root.join("common/country_leader/traits.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
            &file,
            "leader_traits = { valid_specialist = { } valid_genius_name = { } valid_expert = { } valid_genius_name = { } }",
        )
        .unwrap();
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let (traits, _) = content_reader_military::leader_traits::read(&layers).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let leaders = Leaders::from_files(
            vec![(
                "common/unit_leader/t.txt".into(),
                br#"leader_traits = {
                    quoted = { slot = army_chief specialist_advisor_trait = valid_specialist genius_advisor_trait = "valid_genius_name" }
                    bare = { slot = army_chief Specialist_Advisor_Trait = valid_specialist expert_advisor_trait = valid_expert genius_advisor_trait = valid_genius_name }
                    missing = { slot = army_chief specialist_advisor_trait = no_such_trait }
                    folded = { slot = army_chief specialist_advisor_trait = VALID_Specialist }
                    none = { slot = army_chief }
                }"#
                .to_vec(),
            )],
            &|_| None,
        );
    let ranks = [4, 6, 8];
    let traits = Ok(&traits);
    let get = |name| leaders.traits.get(name).unwrap();
    assert_eq!(
        selected_advisor_trait(get("quoted"), 8, ranks, traits, Err("unavailable roles")),
        Ok(("valid_specialist", None))
    );
    assert_eq!(
        selected_advisor_trait(get("bare"), 8, ranks, traits, Err("unavailable roles")),
        Ok(("valid_genius_name", None))
    );
    assert_eq!(
        selected_advisor_trait(get("bare"), 3, ranks, traits, Err("unavailable roles")),
        Ok(("valid_specialist", Some(4)))
    );
    assert_eq!(
        selected_advisor_trait(get("bare"), 4, ranks, traits, Ok(&[])),
        Ok(("valid_specialist", None))
    );
    assert!(
        selected_advisor_trait(get("missing"), 3, ranks, traits, Ok(&[]))
            .unwrap_err()
            .starts_with("the trait's specialist_advisor_trait is null")
    );
    assert_eq!(
        selected_advisor_trait(get("folded"), 3, ranks, traits, Ok(&[])),
        Ok(("valid_specialist", Some(4)))
    );
    assert!(
        selected_advisor_trait(get("none"), 8, ranks, traits, Ok(&[]))
            .unwrap_err()
            .starts_with("the trait's specialist_advisor_trait is null")
    );
    assert!(
        selected_advisor_trait(get("bare"), 3, ranks, Err("unreadable"), Ok(&[]))
            .unwrap_err()
            .contains("unavailable (unreadable)")
    );

    let role = |role, definitions: &[u32]| CharacterRoleTraitRow {
        character: 7,
        kind: CharacterRoleTraitRow::ADVISOR,
        role,
        tokens: vec![99; definitions.len()],
        definitions: definitions.to_vec(),
    };
    let expert = role(1, &[3]);
    let genius = role(5, &[2]);
    let ordered = [&expert, &genius];
    for (skill, expected) in [
        (4, "valid_expert"),
        (5, "valid_expert"),
        (6, "valid_genius_name"),
        (7, "valid_genius_name"),
    ] {
        assert_eq!(
            selected_advisor_trait(get("bare"), skill, ranks, traits, Ok(&ordered)),
            Ok((expected, None))
        );
    }
    let both = role(0, &[3, 2]);
    assert_eq!(
        selected_advisor_trait(get("bare"), 4, ranks, traits, Ok(&[&both])),
        Ok(("valid_genius_name", None))
    );
    let repeat = role(0, &[4]);
    for (skill, expected) in [(4, "valid_specialist"), (6, "valid_expert")] {
        assert_eq!(
            selected_advisor_trait(get("bare"), skill, ranks, traits, Ok(&[&repeat])),
            Ok((expected, None))
        );
    }
    assert_eq!(
        selected_advisor_trait(get("quoted"), 6, ranks, traits, Ok(&[&genius])),
        Ok(("valid_specialist", None))
    );
    assert_eq!(
        selected_advisor_trait(
            get("bare"),
            4,
            ranks,
            traits,
            Err("CharacterRoleTraits is truncated")
        ),
        Err("CharacterRoleTraits is truncated".into())
    );
    assert_eq!(
        selected_advisor_trait(get("bare"), 8, [10, 6, 8], traits, Err("unavailable roles")),
        Ok(("valid_genius_name", Some(10)))
    );

    let loc: Localisation = [
        ("army_chief", "Chief of Army"),
        ("valid_expert", "Expert"),
        ("valid_specialist", "Specialist"),
        (
            "TRAIT_UNLOCKS_ADVISOR_ROLE",
            "Unlocks $ADVISORSLOT$: $ADVISORNAME$",
        ),
        ("TRAIT_UNLOCKED_BY_MIN_RANK", " (rank $RANK$)"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    assert_eq!(
        advisor_line(&NoIcons, &loc, get("bare"), "valid_expert", None),
        Ok("Unlocks Chief of Army: Expert".into())
    );
    assert_eq!(
        advisor_line(&NoIcons, &loc, get("bare"), "valid_specialist", Some(4)),
        Ok("Unlocks Chief of Army: Specialist (rank 4)\n".into())
    );
}

#[test]
fn advisor_roles_join_the_commander_character_in_native_map_order() {
    let leader = UnitLeaderRow {
        id: 3,
        reference_type: 4,
        character_type: 1,
        character_id: 7,
        ..Default::default()
    };
    let characters = [CharacterRow {
        id: 7,
        ..Default::default()
    }];
    let row = |character, kind, role| CharacterRoleTraitRow {
        character,
        kind,
        role,
        tokens: vec![10],
        definitions: vec![1],
    };
    let rows = [
        row(7, CharacterRoleTraitRow::ADVISOR, 5),
        row(8, CharacterRoleTraitRow::ADVISOR, 0),
        row(7, CharacterRoleTraitRow::COUNTRY_LEADER, 0),
        row(7, CharacterRoleTraitRow::ADVISOR, 1),
    ];
    let roles = advisor_roles(Some(&leader), Ok(&characters), Ok(&rows)).unwrap();
    assert_eq!(roles.iter().map(|row| row.role).collect::<Vec<_>>(), [1, 5]);
    assert!(
        advisor_roles(Some(&leader), Ok(&characters), Ok(&[]))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        advisor_roles(
            Some(&leader),
            Ok(&characters),
            Err("CharacterRoleTraits unavailable".into())
        ),
        Err("CharacterRoleTraits unavailable".into())
    );
    assert!(
        advisor_roles(Some(&leader), Ok(&[]), Ok(&rows))
            .unwrap_err()
            .contains("has no Characters row")
    );
    assert!(
        advisor_roles(None, Ok(&characters), Ok(&rows))
            .unwrap_err()
            .contains("UnitLeaders")
    );
    assert!(
        advisor_roles(Some(&UnitLeaderRow::default()), Ok(&characters), Ok(&rows))
            .unwrap_err()
            .contains("reference is null")
    );
}
