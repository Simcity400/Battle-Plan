use super::*;

#[test]
fn deferred_trait_effect_keeps_unsupported_kind() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = Leaders::<crate::Diagnostic>::read_files(
        vec![(
            "traits.txt".into(),
            b"leader_traits = { test = { on_add = { free_operative = { operative = a } } } }"
                .to_vec(),
        )],
        &|_| None,
        &|_| None,
        Arc::default(),
    );
    assert!(
        leaders.misreads.iter().any(|(kind, message)| {
            *kind == crate::DiagnosticKind::Unsupported
                && message.contains("Deferred effect `free_operative`")
        }),
        "{:?}",
        leaders.misreads
    );
}
use std::collections::BTreeSet;

/// Files read through the loader's two passes, `file0.txt` onwards.
fn leaders(files: &[&[u8]], terrain: &dyn Fn(&str) -> Option<String>) -> Leaders {
    let files: Vec<_> = files
        .iter()
        .enumerate()
        .map(|(index, text)| (format!("file{index}.txt"), text.to_vec()))
        .collect();
    Leaders::read_files(files, terrain, &|_| None, Arc::default())
}

fn skills(files: &[&[u8]]) -> Skills {
    leaders(files, &|_| None).skills
}

/// A block's listed entries (not hidden) as stored: the key as last
/// authored and the fixed5.
fn entries(block: &Modifiers) -> Vec<(String, i64)> {
    block
        .listed()
        .map(|modifier| (modifier.key.clone(), modifier.value))
        .collect()
}

/// The name pass registers every `leader_traits` key across the files,
/// a name in another case being the same trait, kept with its first
/// spelling; a quoted `"leader_traits"` is no keyword.
#[test]
fn registered_traits_keep_their_first_spelling_in_order() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
            &[
                b"Leader_Traits = { Brilliant = { } plain = { } } \"leader_traits\" = { skipped = { } }",
                b"leader_traits = { brilliant = { } late = { } }",
            ],
            &|_| None,
        );
    assert_eq!(
        leaders.traits.spellings().collect::<Vec<_>>(),
        ["Brilliant", "plain", "late"]
    );
}

/// The content's terrain types, found without ASCII case.
fn terrain_named<'a>(names: &'a [&'a str]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        names
            .iter()
            .find(|terrain| terrain.eq_ignore_ascii_case(name))
            .map(|terrain| (*terrain).to_owned())
    }
}

/// One trait `t` with `body`, beside `infantry_leader`, read through the
/// loader with the terrain type `forest`.
fn effects(body: &str) -> TraitEffects {
    let text = format!("leader_traits = {{ infantry_leader = {{}} t = {{ {body} }} }}");
    leaders(&[text.as_bytes()], &terrain_named(&["forest"]))
        .traits
        .get("t")
        .cloned()
        .unwrap()
}

#[test]
fn trait_type_mask_warns_only_for_field_marshal_alone() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for body in [
        "type = field_marshal",
        "type = { field_marshal field_marshal }",
    ] {
        assert!(effects(body).army_only, "{body}");
    }
    for body in [
        "",
        "type = {}",
        "type = navy",
        "type = land",
        "type = all",
        "type = corps_commander",
        "type = operative",
        "type = { field_marshal navy }",
        "type = field_marshal type = unknown",
    ] {
        assert!(!effects(body).army_only, "{body}");
    }
    let mixed = effects("type = { unknown field_marshal }");
    assert!(mixed.army_only);
    assert_eq!(mixed.ignored.len(), 1);
}

#[test]
fn categories_sparse_levels_and_duplicate_definitions_match_lookup() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let first: &[u8] = br#"leader_skills = {
                    1 = { type = navy cost = 100 }
                    2 = { type = navy cost = 200 }
                    2 = { type = corps_commander cost = 300 }
                    2 = { type = field_marshal cost = -7 }
                    2 = { type = operative }
                    5 = { type = navy cost = 500 }
                    0 = { type = navy cost = 999 }
                    2 = { cost = 999 }
                }
                attack_skills = { 2 = { type = navy cost = 999 } }"#;
    let skills = self::skills(&[first]);
    assert_eq!(skills.next_required_experience(1, 2), Some(Some(200)));
    assert_eq!(skills.next_required_experience(1, 1), Some(Some(300)));
    assert_eq!(skills.next_required_experience(1, 0), Some(Some(-7)));
    assert_eq!(skills.next_required_experience(1, 3), Some(Some(0)));
    assert_eq!(skills.next_required_experience(2, 2), Some(None));
    assert_eq!(skills.next_required_experience(5, 2), Some(None));
    assert_eq!(skills.next_required_experience(i32::MAX, 2), Some(None));
    assert_eq!(skills.next_required_experience(-1, 2), Some(None));
    assert_eq!(skills.next_required_experience(1, 4), None);
    let skills = self::skills(&[first, b"leader_skills={ 2={type=navy cost=400} }"]);
    assert_eq!(skills.next_required_experience(1, 2), Some(Some(400)));
}

#[test]
fn component_skills_keep_modifiers_hidden_sets_and_last_definition() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let tables = [
        (Component::Attack, "attack"),
        (Component::Defense, "defense"),
        (Component::Planning, "planning"),
        (Component::Logistics, "logistics"),
        (Component::Maneuvering, "maneuvering"),
        (Component::Coordination, "coordination"),
    ];
    let text = tables
        .iter()
        .map(|(_, name)| {
            format!(
                "leader_{name}_skills = {{ 3 = {{ type = navy modifier = {{\n\
             naval_damage_factor = 0.1 hidden_modifier = {{ naval_defense_factor = 0.2 }}\n\
             custom_modifier_tooltip = SYN_SKILL modifier_unknown = 0.4\n\
             }} modifier = {{ naval_damage_factor = 0.15 }}\n\
             building_module_modifier = {{ naval_damage_factor = 0.9 }} }} }}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let skills = skills(&[text.as_bytes()]);
    for (component, _) in tables {
        let block = skills
            .component_modifiers(component, 3, 2)
            .unwrap()
            .unwrap();
        assert_eq!(
            entries(block),
            [
                ("naval_damage_factor".into(), 15_000),
                ("modifier_unknown".into(), 40_000),
            ]
        );
        assert!(block.hidden.contains("naval_defense_factor"));
        assert_eq!(block.tooltips(), ["SYN_SKILL"]);
        assert!(
            skills
                .component_modifiers(component, -1, 2)
                .unwrap()
                .is_none()
        );
    }
    let replaced = self::skills(&[
        text.as_bytes(),
        b"leader_attack_skills = { 3 = { type = navy modifier = { naval_damage_factor = 0.3 } } }",
    ]);
    let block = replaced
        .component_modifiers(Component::Attack, 3, 2)
        .unwrap()
        .unwrap();
    assert_eq!(entries(block), [("naval_damage_factor".into(), 30_000)]);
    assert!(block.hidden.is_empty());
    assert!(block.tooltips().is_empty());
}

#[test]
fn component_skill_modifier_conditions_settle_with_the_registry() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let unknown = leaders(
        &[
            b"leader_attack_skills = { 3 = { type = navy modifier = { navy_org = { naval_damage_factor = 0.2 } } } }",
        ],
        &|_| None,
    );
    assert_eq!(unknown.skills.component_conditions().len(), 1);
    assert!(unknown.conditional());
    let settled = unknown.reread(&|_| None, &|name| Some(name == b"navy_org"));
    assert!(settled.skills.component_conditions().is_empty());
    let block = settled
        .skills
        .component_modifiers(Component::Attack, 3, 2)
        .unwrap()
        .unwrap();
    assert_eq!(
        entries(block),
        [
            ("navy_org".into(), 0),
            ("naval_damage_factor".into(), 20_000),
        ]
    );
}

/// Signed integer field conversion reports a value that does not scan
/// and keeps the field, so an undefined `@cost` leaves the earlier value,
/// as the game has it.
#[test]
fn an_unscannable_cost_keeps_the_previous_value() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let skills = skills(&[b"leader_skills={ 2={type=navy cost=50 cost=@cost} }"]);
    assert_eq!(skills.next_required_experience(1, 2), Some(Some(50)));
    assert_eq!(
        skills.warnings(),
        [
            "Commander skill levels: file0.txt: leader_skills level 2 cost @cost does not scan as an integer; the game reports it and keeps 50."
        ]
    );
    // Past 32 bits the CRT keeps the low 32 bits.
    let skills = self::skills(&[b"leader_skills={ 2={type=navy cost=4294967297} }"]);
    assert_eq!(skills.next_required_experience(1, 2), Some(Some(1)));
}

#[test]
fn trait_blocks_keep_authored_entries_adjusters_and_sub_units() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let effects = effects(
        r#"type = corps_commander
                enable_ability = force_attack
                attack_skill = 2
                @speed = 0.25
                modifier = {
                    army_attack_factor = 0.05
                    forest = { attack = 0.1 movement = 0.05 }
                    custom_modifier_tooltip = my_tt
                    custom_modifier_tooltip = my_tt
                    hidden_modifier = { army_org_factor = 0.1 }
                    added_modifier = { army_org_factor = 0.1 }
                    hidden_modifier = yes
                }
                non_shared_modifier = { river = { attack = 0.1 } }
                field_marshal_modifier = { army_speed_factor = @speed army_org = @missing }
                sub_unit_modifiers = {
                    destroyer = { units = { attack = 0.1 } naval_speed_factor = 0.1 }
                    cruiser = { custom_modifier_tooltip = b_tt }
                }
                trait_xp_factor = { INFANTRY_LEADER = 1 no_such_trait = { x = 1 } }
                leader_default_proximity_offset = -1
                custom_effect_tooltip = extra_tt
                mystery = yes"#,
    );
    assert_eq!(effects.name, "t");
    assert_eq!(effects.skills, [2, 0, 0, 0, 0, 0]);
    assert_eq!(
        entries(&effects.blocks[0]),
        [("army_attack_factor".to_owned(), 5_000)]
    );
    // One key per text in the modifier set.
    assert_eq!(effects.blocks[0].tooltips(), ["my_tt"]);
    assert_eq!(effects.blocks[0].children.len(), 1);
    // Stored with the others, and hidden.
    assert_eq!(
        effects.blocks[0].hidden,
        BTreeSet::from(["army_org_factor".to_owned()])
    );
    assert_eq!(effects.blocks[0].values[1].value, 10_000);
    // Adjusters in first-appearance order across the blocks.
    assert_eq!(
        effects.adjusters,
        [
            Adjuster {
                key: "forest".into(),
                attack: 10_000,
                movement: 5_000,
                ..Adjuster::default()
            },
            Adjuster {
                key: "river".into(),
                attack: 10_000,
                ..Adjuster::default()
            }
        ]
    );
    // The variable resolves; an unknown one reads as the game's zero.
    assert_eq!(
        entries(&effects.blocks[2]),
        [
            ("army_speed_factor".to_owned(), 25_000),
            ("army_org".to_owned(), 0)
        ]
    );
    assert_eq!(effects.sub_units.len(), 2);
    assert_eq!(effects.sub_units[0].adjuster.key, "destroyer");
    assert_eq!(effects.sub_units[0].adjuster.attack, 10_000);
    assert_eq!(
        entries(&effects.sub_units[0].block),
        [("naval_speed_factor".to_owned(), 10_000)]
    );
    assert_eq!(effects.sub_units[1].block.tooltips(), ["b_tt"]);
    // The trait's registered spelling; a key naming no trait is skipped
    // with its block.
    assert_eq!(
        effects.experience_factors,
        [("infantry_leader".to_owned(), 100_000)]
    );
    assert_eq!(effects.proximity_offset, -1);
    assert_eq!(effects.custom_tooltips, ["extra_tt"]);
    assert!(effects.unsupported.is_empty(), "{:?}", effects.unsupported);
    assert!(effects.conditions.is_empty());
    assert_eq!(
        effects.ignored,
        [
            "modifier.hidden_modifier is not a block; the game's reader records it and reads nothing more",
            "field_marshal_modifier.army_org names the variable @missing, which is not defined before it; the game reads zero",
            "trait_xp_factor no_such_trait names no trait (none matches its token)",
            "unknown trait key mystery"
        ]
    );
}

/// A repeat of an adjuster other than the default token leaves its
/// block unread: the game reads `{` and the block's entries as the
/// modifier block's, whose end its `}` becomes. A new `none` adjuster
/// is read twice: the second read takes what follows as its fields.
/// `Forest` and `forest` are one.
#[test]
fn repeated_adjusters_follow_the_default_token_rule() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let read = |body: &str| {
        let text = format!("leader_traits = {{ t = {{ {body} }} }}");
        let leaders = leaders(&[text.as_bytes()], &terrain_named(&["forest", "none"]));
        (leaders.traits.get("t").cloned().unwrap(), leaders.misreads)
    };
    let (t, misreads) = read(
        "modifier = { forest = { attack = 0.1 } Forest = { defence = 0.2 } x = 1 } attack_skill = 3",
    );
    assert_eq!((t.adjusters[0].attack, t.adjusters[0].defence), (10_000, 0));
    assert_eq!(entries(&t.blocks[0]), [("defence".to_owned(), 20_000)]);
    // `attack_skill = 3` came after the trait's end: never its field.
    assert_eq!(t.skills[0], 0);
    assert!(misreads[0].contains("field `Forest` is a block where its reader takes one value"));
    let (t, _) = read("modifier = { NONE = { attack = 0.1 } x = 0.5 } y = 1");
    assert_eq!(t.adjusters[0].key, "none");
    assert_eq!(t.adjusters[0].attack, 10_000);
    assert_eq!(t.ignored, ["adjuster none has the unknown entry x"]);
    assert_eq!(entries(&t.blocks[0]), [("y".to_owned(), 100_000)]);
}

/// The name pass registers each name once without case, the first spelling
/// kept; every body of that name merges into the first trait. Where a
/// reading leaves the text, the next reader goes on from there: here a
/// repeated adjuster's block ends the modifier block, so b's own `}` closes
/// the trait table early and c is read at the top level, which skips it.
#[test]
fn repeated_trait_keys_merge_and_a_misread_reading_goes_on() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
        &[br#"leader_traits = {
                a = { attack_skill = 1 modifier = { x = 0.1 } }
                A = { defense_skill = 2 modifier = { y = 0.2 } }
                b = { modifier = { forest = {} forest = {} } }
                c = { attack_skill = 3 }
            }"#],
        &terrain_named(&["forest"]),
    );
    let traits = &leaders.traits;
    let a = traits.get("A").unwrap();
    assert_eq!(a.name, "a");
    assert_eq!(a.skills[..2], [1, 2]);
    assert_eq!(
        entries(&a.blocks[0]),
        [("x".to_owned(), 10_000), ("y".to_owned(), 20_000)]
    );
    assert_eq!(traits.get("c").unwrap().skills[0], 0);
    assert!(traits.unsupported.is_empty());
    assert_eq!(leaders.misreads.len(), 2, "{:?}", leaders.misreads);
    assert!(leaders.misreads[0].starts_with("file0.txt line 4: field `forest` is a block"));
    // A scalar body makes the trait's reader take what follows as its
    // fields: c's block is an unknown field of `scalar`, skipped.
    let leaders = self::leaders(
        &[b"leader_traits = { scalar = 5 c = { attack_skill = 3 } }"],
        &|_| None,
    );
    assert_eq!(leaders.traits.get("c").unwrap().skills[0], 0);
    assert_eq!(
        leaders.traits.get("scalar").unwrap().ignored,
        ["unknown trait key c"]
    );
    assert!(leaders.misreads[0].contains("field `scalar` is `5` where its reader reads a block"));
}

/// The file's reader variables through the loader's own read: a chain
/// resolves when it is defined, names keep their case, a quoted token is
/// never a reference (its quoted flag is checked first), a definition
/// anywhere the reader reads by key holds for the rest of the file,
/// `leader_skills` and `leader_traits` share it, and the next file
/// starts empty.
#[test]
fn variables_follow_the_game_reader_through_the_file() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
        &[
            br#"@a = 0.5
                    @A = 0.25
                    @quoted = "@a"
                    leader_skills = { @cost = 100 1 = { type = navy cost = @cost } }
                    leader_traits = {
                        t = {
                            attack_skill = @undefined
                            modifier = {
                                @b = @a
                                x = @b
                                y = @A
                                z = "@a"
                                q = @quoted
                                custom_modifier_tooltip = @tooltip
                                @tooltip = my_tt
                                custom_modifier_tooltip = @tooltip
                                custom_modifier_tooltip = "@tooltip"
                            }
                            mystery = { @skipped = 1 }
                        }
                        u = { modifier = { w = @b s = @skipped } }
                    }"#,
            b"leader_traits = { v = { modifier = { x = @a } } }",
        ],
        &|_| None,
    );
    assert_eq!(
        leaders.skills.next_required_experience(0, 2),
        Some(Some(100))
    );
    let t = leaders.traits.get("t").unwrap();
    assert!(t.unsupported.is_empty(), "{:?}", t.unsupported);
    assert_eq!(
        entries(&t.blocks[0]),
        [
            ("x".to_owned(), 50_000),
            ("y".to_owned(), 25_000),
            ("z".to_owned(), 0),
            ("q".to_owned(), 0)
        ]
    );
    // A reference before its definition, and a quoted one, stay text;
    // the set holds each text once.
    let mut tooltips = t.blocks[0].tooltips();
    tooltips.sort();
    assert_eq!(tooltips, ["@tooltip", "my_tt"]);
    assert_eq!(
        t.ignored,
        [
            "attack_skill value @undefined does not scan as an integer; the game keeps the previous value",
            "modifier.z value @a does not scan as a number; the game reads zero",
            "modifier.q value @a does not scan as a number; the game reads zero",
            "unknown trait key mystery"
        ]
    );
    // `@b` holds for the rest of the file; the skipped block defined nothing.
    let u = leaders.traits.get("u").unwrap();
    assert_eq!(
        entries(&u.blocks[0]),
        [("w".to_owned(), 50_000), ("s".to_owned(), 0)]
    );
    assert_eq!(
        u.ignored,
        [
            "modifier.s names the variable @skipped, which is not defined before it; the game reads zero"
        ]
    );
    let v = leaders.traits.get("v").unwrap();
    assert_eq!(entries(&v.blocks[0]), [("x".to_owned(), 0)]);
    assert!(leaders.traits.unsupported.is_empty());
}

/// The game keeps a quoted `"@a"` as text, and its lexer ends a token at
/// `(`: `@(1+1)` is the reference `@`, then `(`, `1+1` and `)`, read as keys
/// without operators, the last consuming the modifier block's `}`, so
/// e's `}` ends the modifier block and `after` is e's unknown field.
#[test]
fn quoted_references_stay_text_and_expressions_split() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
        &[br#"@a = 0.5
                leader_traits = {
                    t = { modifier = { x = "@a" } }
                    e = { modifier = { y = @(1+1) } }
                    after = { attack_skill = 1 }
                }"#],
        &|_| None,
    );
    let traits = &leaders.traits;
    assert_eq!(
        entries(&traits.get("t").unwrap().blocks[0]),
        [("x".to_owned(), 0)]
    );
    let e = traits.get("e").unwrap();
    // `(` and `)` are punctuation, whose token ids name no modifier:
    // no entries.
    assert_eq!(entries(&e.blocks[0]), [("y".to_owned(), 0)]);
    assert!(e.ignored.contains(&"unknown trait key after".to_owned()));
    assert_eq!(traits.get("after").unwrap().skills[0], 0);
    assert!(traits.unsupported.is_empty());
}

/// Blocks the hovers do not read still define as their readers do, in
/// document order: by key for triggers, effects, mean-time fields, parent
/// traits, added_modifier and a component skill table's level modifier;
/// a token list, a string list, an unknown key and a non-positive level
/// define nothing.
#[test]
fn unread_blocks_define_as_their_readers_do() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
            &[br#"leader_attack_skills = { 1 = { modifier = { @skill = 0.1 x = 1 } } 0 = { modifier = { @zero = 1 x = 1 } } }
                other = { @other = 1 x = 1 }
                leader_traits = {
                    s = {
                        on_add = { hidden_effect = { @effect = 0.2 add_stability = 1 } }
                        allowed = { power < @missing @trigger = 0.3 always = yes }
                        ai_will_do = { factor = 1 modifier = { @weight = 0.4 factor = 2 } }
                        parent = { traits = { a b } @parent = 0.5 num_parents_needed = 1 }
                        type = { land @typed }
                        modifier = { added_modifier = { @added = 0.6 y = 1 } }
                        mystery = { @mystery = 1 z = 1 }
                    }
                    t = { modifier = { a = @skill b = @zero c = @other d = @effect e = @trigger f = @weight g = @parent h = @added i = @mystery } }
                }"#],
            &|_| None,
        );
    assert!(
        leaders.traits.unsupported.is_empty(),
        "{:?}",
        leaders.traits.unsupported
    );
    let entries = entries(&leaders.traits.get("t").unwrap().blocks[0]);
    let entries: Vec<_> = entries
        .iter()
        .map(|(key, value)| (key.as_str(), *value))
        .collect();
    assert_eq!(
        entries,
        [
            ("a", 10_000),
            ("b", 0),
            ("c", 0),
            ("d", 20_000),
            ("e", 30_000),
            ("f", 40_000),
            ("g", 50_000),
            ("h", 60_000),
            ("i", 0),
        ]
    );
}

/// Dispatch `type` on the value token's id, keeping a definition's token,
/// so a quoted category, direct or through a variable, is no keyword:
/// reported, the level keeps category 4 and no shard holds it. A bare
/// one through a variable is a category.
#[test]
fn a_quoted_skill_type_is_no_category_keyword() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for text in [
        &br#"leader_skills = { 1 = { type = "navy" cost = 5 } }"#[..],
        br#"@navy = "navy" leader_skills = { 1 = { type = @navy cost = 5 } }"#,
    ] {
        let leaders = leaders(&[text], &|_| None);
        assert_eq!(leaders.skills.next_required_experience(0, 2), Some(None));
        assert!(
            leaders.skills.warnings()[0]
                .contains("type navy (quoted) is not a leader-category keyword"),
            "{:?}",
            leaders.skills.warnings()
        );
    }
    let leaders = leaders(
        &[br#"@navy = navy leader_skills = { 1 = { type = @navy cost = 5 } }"#],
        &|_| None,
    );
    assert_eq!(leaders.skills.next_required_experience(0, 2), Some(Some(5)));
}

/// Forms the tree cannot hold are read as the game reads them, each
/// named: a taken name (leaving `= value` for the next key), a block value
/// (only its `{` is kept), a taken name inside a block read by key, a block
/// where one value is read, a definition ending its block (the reader takes
/// `}` as a key), and an undefined `@` value (read again as a definition).
/// What reaches `t` follows.
#[test]
fn forms_the_tree_cannot_hold_are_read_as_the_game_reads_them() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for (text, attack, misread) in [
            (
                &b"@a = 0.5 @a = 0.75 leader_traits = { t = { attack_skill = 1 } }"[..],
                1,
                "variable `@a` is already taken",
            ),
            (
                b"@a = { 1 } leader_traits = { t = { attack_skill = 1 } }",
                1,
                "definition `@a` keeps only the `{` of its block value",
            ),
            (
                b"leader_traits = { s = { on_add = { @x = 1 @x = 2 add = 1 } } t = { attack_skill = 1 } }",
                1,
                "variable `@x` is already taken",
            ),
            (
                b"leader_traits = { s = { gui_row = { @x = 1 y = 2 } } t = { attack_skill = 1 } }",
                0,
                "field `gui_row` is a block where its reader takes one value",
            ),
            (
                b"leader_traits = { s = { modifier = { x = 1 @last = 2 } } t = { attack_skill = 1 } }",
                0,
                "key `}` has no operator",
            ),
            (
                b"leader_traits = { s = { @r = @nowhere } t = { attack_skill = 1 } }",
                0,
                "definition `@r` takes `@nowhere`, which the variable reader leaves unread",
            ),
        ] {
            let leaders = leaders(
                &[text, b"leader_skills = { 1 = { type = navy cost = 5 } }"],
                &|_| None,
            );
            let t = leaders.traits.get("t").unwrap();
            assert_eq!(t.skills[0], attack, "{}", String::from_utf8_lossy(text));
            assert!(t.unsupported.is_empty());
            assert!(
                leaders.misreads.iter().any(|what| what.contains(misread)),
                "{misread}: {:?}",
                leaders.misreads
            );
            // The next file has its own reader.
            assert_eq!(
                leaders.skills.next_required_experience(0, 2),
                Some(Some(5))
            );
        }
    let leaders = leaders(
        &[b"@a = 1 @a = 2 leader_skills = { 1 = { type = navy cost = 5 } }"],
        &|_| None,
    );
    assert_eq!(leaders.skills.next_required_experience(0, 2), Some(Some(5)));
}

/// A block where a reader takes one value (none skips it, so the persistent
/// reader reads its contents as the enclosing fields) and a value without
/// a key (consuming the next token as its operator and handing the key
/// the previous value) are read as the game reads them.
#[test]
fn scalar_fields_given_blocks_and_keyless_values_read_on() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    // The block's fields become t's; its `}` ends t, so `after`'s body is
    // read at the table level, and t's own `}` ends the table.
    let leaders = leaders(
            &[b"leader_traits = { t = { attack_skill = { defense_skill = 2 } planning_skill = 4 } after = { attack_skill = 1 } }"],
            &|_| None,
        );
    let t = leaders.traits.get("t").unwrap();
    assert_eq!(t.skills[..4], [0, 2, 0, 0]);
    assert_eq!(leaders.traits.get("after").unwrap().skills[0], 0);
    // `stray` takes the previous value 3 and consumes `slot`; `=` then
    // consumes `x`; the trait goes on. `=` is punctuation (token 1),
    // which the lexer never looks up as a keyword (parser::looked_up).
    let leaders = self::leaders(
        &[b"leader_traits = { t = { attack_skill = 3 stray slot = x Coordination_Skill = 5 } }"],
        &|_| None,
    );
    let t = leaders.traits.get("t").unwrap();
    assert_eq!(t.skills, [3, 0, 0, 0, 0, 5]);
    assert_eq!(t.advisor_slot, None);
    assert_eq!(
        t.ignored,
        [
            "unknown trait key stray",
            "trait key = is not a keyword (quoted, or never looked up by the lexer)"
        ]
    );
    assert_eq!(leaders.misreads.len(), 2);
}

/// Mean-time and parent-trait readers only log a key they do not know,
/// skipping nothing: a block there is read as their fields and its `}`
/// ends them, so the trait's own `}` ends the trait before `defense_skill`.
/// Known keys read on.
#[test]
fn mean_time_and_parent_readers_skip_nothing_for_unknown_keys() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for body in [
        &b"ai_will_do = { odd = { attack_skill = 1 } }"[..],
        b"new_commander_weight = { factor = 1 odd = { attack_skill = 1 } }",
        b"parent = { traits = { a } odd = { attack_skill = 1 } }",
    ] {
        let text = [b"leader_traits = { t = { ", body, b" defense_skill = 2 } }"].concat();
        let leaders = leaders(&[&text], &|_| None);
        let t = leaders.traits.get("t").unwrap();
        assert_eq!(t.skills[..2], [0, 0], "{}", String::from_utf8_lossy(body));
        assert!(
            leaders.misreads[0].contains("field `odd` is a block where its reader takes one value"),
            "{:?}",
            leaders.misreads
        );
    }
    let leaders = leaders(
            &[b"leader_traits = { t = { ai_will_do = { factor = 2 base = 1 days = 3 modifier = { factor = 0 has_war = yes } } parent = { traits = { a b } num_parents_needed = 1 } defense_skill = 2 } }"],
            &|_| None,
        );
    assert_eq!(leaders.traits.get("t").unwrap().skills[1], 2);
    assert!(leaders.misreads.is_empty(), "{:?}", leaders.misreads);
}

/// Text that is not UTF-8 is not a block: an integer scans its bytes, a
/// string field names the encoding, never the block shape.
#[test]
fn text_that_is_not_utf8_is_named_as_such() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let text = b"leader_traits = { t = { attack_skill = 3\xfe slot = x\xfe \
                     modifier = { custom_modifier_tooltip = \"t\xfe\" } } }";
    let traits = leaders(&[text], &|_| None).traits;
    let t = traits.get("t").unwrap();
    assert_eq!(t.skills[0], 3);
    // A modifier block's text through the shared reader's first failure
    // (Modifiers::unheld).
    assert_eq!(
        t.unsupported,
        [
            "slot value x\u{fffd} is not UTF-8",
            "modifier: custom_modifier_tooltip is the stored text `t\\xfe` is not UTF-8, which the port's text cannot hold"
        ]
    );
}

/// The name pass registers a non-UTF-8 name as bytes; the body pass finds
/// its token and reads its body, including file-local definitions. A
/// display refusal must not change the later traits' operands.
#[test]
fn non_utf8_trait_names_still_dispatch_their_bodies() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let text = b"leader_traits = {
            x\xfe = { @n = 3 attack_skill = 1 }
            x\xfd = { @m = 7 attack_skill = 2 }
            readable = { attack_skill = @n defense_skill = @m }
            referenced = { trait_xp_factor = { x\xfe = 0.5 } }
        }";
    let traits = leaders(&[text], &|_| None).traits;
    assert!(traits.get("readable").is_some(), "{traits:#?}");
    assert_eq!(traits.get("readable").unwrap().skills[..2], [3, 7]);
    assert!(traits.get("readable").unwrap().unsupported.is_empty());
    assert_eq!(traits.traits.len(), 4);
    assert_eq!(traits.traits[b"x\xfe".as_slice()].skills[0], 1);
    assert_eq!(traits.traits[b"x\xfd".as_slice()].skills[0], 2);
    assert!(traits.get("x\u{fffd}").is_none());
    assert_eq!(
        traits.spellings().collect::<Vec<_>>(),
        ["readable", "referenced"]
    );
    assert!(
        traits.get("referenced").unwrap().unsupported[0]
            .contains("trait_xp_factor: file0.txt: leader_traits entry")
    );
    assert_eq!(traits.unsupported.len(), 2);
}

/// A quoted key is token 0xF: no keyword, adjuster or `trait_xp_factor`
/// trait, so the readers take their other branch; a quoted modifier name
/// is still found by its text; a quoted `leader_traits` is skipped by
/// both passes.
#[test]
fn quoted_keys_are_no_keywords() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
            &[br#""leader_skills" = { 1 = { type = navy cost = 5 } }
                leader_traits = {
                    infantry_leader = {}
                    t = {
                        "attack_skill" = 3
                        defense_skill = 1
                        modifier = { "forest" = { attack = 1 } "army_org" = 0.5 "hidden_modifier" = 1 }
                        trait_xp_factor = { "infantry_leader" = 1 infantry_leader = 2 }
                    }
                }
                "leader_traits" = { u = { attack_skill = 1 } }"#],
            &terrain_named(&["forest"]),
        );
    assert_eq!(leaders.skills.next_required_experience(0, 2), Some(None));
    let t = leaders.traits.get("t").unwrap();
    assert_eq!(t.skills[..2], [0, 1]);
    assert!(t.adjusters.is_empty());
    assert_eq!(
        entries(&t.blocks[0]),
        [
            ("forest".to_owned(), 0),
            ("army_org".to_owned(), 50_000),
            ("hidden_modifier".to_owned(), 100_000)
        ]
    );
    assert_eq!(
        t.ignored[0],
        "trait key attack_skill is not a keyword (quoted, or never looked up by the lexer)"
    );
    assert!(t.ignored[1].starts_with("modifier.forest is a block"));
    // The quoted `"forest"` block names no adjuster: whether it is a
    // registered modifier decides the rest (a Conditional).
    assert_eq!(t.conditions[0].modifier, "forest");
    assert_eq!(
        t.experience_factors,
        [("infantry_leader".to_owned(), 200_000)]
    );
    assert!(leaders.traits.get("u").is_none());
}

/// A block under a modifier entry leaves the rest of the file depending
/// on the registry: the trait holding it and every later trait and
/// `leader_skills` of that file carry the condition, earlier ones and
/// other files do not; the entry itself is not kept, and nothing inside
/// defines.
#[test]
fn a_block_under_a_modifier_entry_conditions_the_rest_of_the_file() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
            &[
                b"leader_traits = {\nbefore = { attack_skill = 1 }\nt = { modifier = { army_org = { @inside = 1 } x = 0.1 } }\nafter = { attack_skill = 2 modifier = { y = @inside } }\n}\nleader_skills = { 1 = { type = navy cost = 5 } }",
                b"leader_traits = { other = { attack_skill = 3 } }",
            ],
            &|_| None,
        );
    let condition = Conditional {
        modifier: "army_org".into(),
        place: "file0.txt line 3".into(),
    };
    let traits = &leaders.traits;
    assert!(traits.get("before").unwrap().conditions.is_empty());
    let t = traits.get("t").unwrap();
    assert_eq!(t.conditions, std::slice::from_ref(&condition));
    assert_eq!(
        entries(&t.blocks[0]),
        [("army_org".to_owned(), 0), ("x".to_owned(), 10_000)]
    );
    let after = traits.get("after").unwrap();
    assert_eq!(after.conditions, std::slice::from_ref(&condition));
    assert_eq!(entries(&after.blocks[0]), [("y".to_owned(), 0)]);
    assert_eq!(
        leaders.skills.conditions(),
        std::slice::from_ref(&condition)
    );
    assert!(traits.get("other").unwrap().conditions.is_empty());
    assert!(
        condition
            .to_string()
            .starts_with("file0.txt line 3 gives army_org a block value")
    );
}

/// With the registry, a block under a registered modifier is one value,
/// taking `{`: its contents are the modifier block's entries and its `}`
/// ends that block; under a name that is no modifier it is skipped.
/// Neither leaves a condition.
#[test]
fn the_registry_settles_a_block_under_a_modifier_entry() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let text: &[u8] =
            b"leader_traits = { t = { modifier = { army_org = { y = 0.2 } x = 0.1 } attack_skill = 3 } }";
    let unknown = leaders(&[text], &|_| None);
    assert!(unknown.conditional());
    let registered = unknown.reread(&|_| None, &|name| Some(name == b"army_org"));
    assert!(!registered.conditional());
    let t = registered.traits.get("t").unwrap();
    // `x = 0.1` and `attack_skill = 3` fall out of the modifier block.
    assert_eq!(
        entries(&t.blocks[0]),
        [("army_org".to_owned(), 0), ("y".to_owned(), 20_000)]
    );
    assert_eq!(t.skills[0], 0);
    let skipped = unknown.reread(&|_| None, &|_| Some(false));
    let t = skipped.traits.get("t").unwrap();
    // The skipped block is stored as the zero Modifiers keeps for it.
    assert_eq!(
        entries(&t.blocks[0]),
        [("army_org".to_owned(), 0), ("x".to_owned(), 10_000)]
    );
    assert_eq!(t.skills[0], 3);
    assert!(t.conditions.is_empty());
}

#[test]
fn sub_unit_directories_read_modifiers_and_tags_first() {
    assert_eq!(
        SUB_UNIT_DIRECTORIES,
        [
            "common/units/unit_modifiers/",
            "common/unit_tags/",
            "common/units/critical_parts/",
            "common/units/"
        ]
    );
}

fn sub_units(files: &[&[u8]]) -> SubUnits {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
    let mut indices = SubUnits::default();
    for (index, text) in files.iter().enumerate() {
        indices.read(text, &format!("units{index}.txt"));
    }
    indices
}

#[test]
fn sub_unit_groups_follow_resolved_tokens_and_repeated_body_retention() {
    let units = sub_units(&[
        b"@g = Mod_Group sub_units = { foot = { group = @g } support = { group = Mod_Group } \
          orphan = {} kept = { group = previous } cleared = { group = previous } \
          empty = { group = previous } nul = { group = \"head\0tail\" } \
          bad_text = { group = \"\xfe\" } }",
        b"sub_units = { FOOT = {} kept = { \"group\" = ignored } \
          support = { group = first group = second } cleared = { group = -1 } \
          empty = { group = \"\" } }",
    ]);
    assert_eq!(units.group(b"foot"), Ok(Some("Mod_Group")));
    assert_eq!(units.group(b"SUPPORT"), Ok(Some("second")));
    assert_eq!(units.group(b"kept"), Ok(Some("previous")));
    assert_eq!(units.group(b"nul"), Ok(Some("head")));
    for name in [b"orphan".as_slice(), b"cleared", b"empty"] {
        assert_eq!(units.group(name), Ok(None));
    }
    assert!(units.group(b"bad_text").unwrap_err().contains("not UTF-8"));
    assert!(units.group(b"absent").unwrap_err().contains("absent"));
}

#[test]
fn sub_unit_group_gaps_require_an_explicit_assignment_to_recover() {
    let mut units = sub_units(&[
        b"sub_units={a={group=known} kept={group=known}}",
        b"sub_units={bad={weights={x @variable=1}}}",
    ]);
    let refused = units.group(b"a").unwrap_err();
    assert!(refused.contains("variable @variable"), "{refused}");
    assert_eq!(units.group(b"kept"), Err(refused.clone()));
    units.read(
        b"sub_units={A={} kept={group=restored} new={}}",
        "recovered.txt",
    );
    assert_eq!(units.group(b"a"), Err(refused.clone()));
    assert_eq!(units.group(b"kept"), Ok(Some("restored")));
    assert_eq!(units.group(b"new"), Err(refused));
    units.read(b"sub_units={A={group=0}}", "cleared.txt");
    assert_eq!(units.group(b"a"), Ok(None));
}

/// `is_artillery_brigade` as the sub-unit reader reads it: exact texts
/// `yes` and `no`, anything else leaving the value; a repeated name, in
/// any case, is one definition reset before its body; the file's variables;
/// a quoted key is no keyword; a block where the value is read leaves
/// its contents to the definition, whose unknown readers then decide
/// the rest; and a file whose reading cannot be followed leaves every
/// value unknown.
#[test]
fn artillery_brigade_follows_the_sub_unit_reader() {
    let units = sub_units(&[
        &b"@yes = yes SUB_UNITS = { artillery = { IS_ARTILLERY_BRIGADE = @yes } \
               odd = { is_artillery_brigade = YES } infantry = { } \
               reset = { is_artillery_brigade = yes } }"[..],
        b"sub_units = { Reset = { \"is_artillery_brigade\" = yes } }",
    ]);
    assert_eq!(units.artillery(b"artillery"), Ok(true));
    assert_eq!(units.artillery(b"Artillery"), Ok(true));
    assert_eq!(units.artillery(b"infantry"), Ok(false));
    // The repeat is reset and its quoted key sets nothing.
    assert_eq!(units.artillery(b"RESET"), Ok(false));
    // `YES` fails the boolean scan: the default `no` stays.
    assert_eq!(units.artillery(b"odd"), Ok(false));
    assert_eq!(
        units.artillery(b"absent"),
        Err("sub-unit absent is not in the content's sub_units".into())
    );
    // `{` fails too; the block's `yes` is a key that consumes `}` as its
    // operator and is handed the stale `{`, which its reader decides.
    let units = sub_units(&[
            b"sub_units = { bad = { is_artillery_brigade = { yes } } later = { is_artillery_brigade = yes } }",
        ]);
    let refused = units.artillery(b"later").unwrap_err();
    assert!(
        refused
            .contains("key `yes` has no operator and its reader is handed the previous value `{`"),
        "{refused}"
    );
    assert_eq!(units.artillery(b"bad"), Err(refused));
    let units = sub_units(&[
            b"sub_units = { a = { is_artillery_brigade = yes } b = { weights = { x @v = 1 } } c = { } }",
        ]);
    let refused = units.artillery(b"b").unwrap_err();
    assert!(refused.contains("variable @v"), "{refused}");
    assert_eq!(units.artillery(b"a"), Err(refused.clone()));
    assert_eq!(units.artillery(b"c"), Err(refused));
}

/// A file whose reading stops leaves every `is_artillery_brigade` unknown,
/// before the stop or not: its unread tail may define any name again.
/// A later file's appearance settles the name, since the value resets
/// before the body, whatever the body assigns; a later stop makes every
/// value unknown again.
#[test]
fn an_unread_tail_leaves_every_artillery_brigade_unknown_until_defined_again() {
    let mut units = sub_units(&[
            &b"sub_units = { a = { is_artillery_brigade = yes } b = { is_artillery_brigade = yes } \
               kept = { is_artillery_brigade = yes } z = { is_artillery_brigade = yes } }"[..],
            b"sub_units = { c = { is_artillery_brigade = yes } d = { weights = { x @v = 1 } } }",
            b"sub_units = { A = { is_artillery_brigade = no } b = { } C = { is_artillery_brigade = yes } \
              new = { } Z = { is_artillery_brigade = maybe } }",
        ]);
    let first = units.artillery(b"d").unwrap_err();
    assert!(
        first.starts_with("units1.txt") && first.contains("variable @v"),
        "{first}"
    );
    assert_eq!(units.artillery(b"kept"), Err(first.clone()));
    assert_eq!(units.artillery(b"absent"), Err(first.clone()));
    assert_eq!(units.artillery(b"a"), Ok(false));
    assert_eq!(units.artillery(b"b"), Ok(false));
    assert_eq!(units.artillery(b"c"), Ok(true));
    assert_eq!(units.artillery(b"new"), Ok(false));
    assert_eq!(units.artillery(b"z"), Ok(false));
    units.read(
        b"sub_units = { e = { is_artillery_brigade = yes } f = { weights = { x @w = 1 } } }",
        "units3.txt",
    );
    let second = units.artillery(b"f").unwrap_err();
    assert!(
        second.starts_with("units3.txt") && second.contains("variable @w"),
        "{second}"
    );
    for name in [&b"a"[..], b"c", b"new", b"e"] {
        assert_eq!(units.artillery(name), Err(second.clone()));
    }
    assert_eq!(units.artillery(b"absent"), Err(first.clone()));
    let stops: Vec<_> = units
        .warnings()
        .iter()
        .filter(|warning| warning.contains("is_artillery_brigade is unknown"))
        .collect();
    assert_eq!(stops.len(), 2, "{:?}", units.warnings());
    assert!(stops[0].contains(&first) && stops[0].contains("unknown index"));
    assert!(stops[1].contains(&second) && !stops[1].contains("unknown index"));
}

/// The name pass registers every `leader_traits` key whatever its value,
/// on each file's own reader: a name in another case is the same trait,
/// a quoted `"leader_traits"` is no keyword, the skip defines nothing,
/// and a name that is not UTF-8 is named and not kept.
#[test]
fn trait_names_follow_the_append_pass() {
    let mut traits = Traits::default();
    for (path, text) in [
        (
            "a.txt",
            &b"@t = 1 LEADER_TRAITS = { brilliant = { x = @t } scalar = yes later = { } } \
               \"leader_traits\" = { quoted_block = { } } other = { not_a_trait = { } }"[..],
        ),
        (
            "b.txt",
            b"leader_traits = { first = { } second = { x @v = 1 } \xe9 = { } third = { } }",
        ),
        ("c.txt", b"leader_traits = { Brilliant = { } }"),
    ] {
        traits.register(text, path);
    }
    assert_eq!(
        traits.spellings().collect::<Vec<_>>(),
        ["brilliant", "scalar", "later", "first", "second", "third"]
    );
    assert!(
        traits.first_ordinal(b"BRILLIANT") == Ok(Some(1))
            && traits.first_ordinal(b"quoted_block") == Ok(None)
    );
    assert_eq!(traits.unsupported.len(), 1);
    assert!(
        traits.unsupported[0].starts_with("b.txt: leader_traits entry"),
        "{:?}",
        traits.unsupported
    );
}

/// Lookup finds the first definition the name pass appended for a name
/// in any case: its ordinal counts every earlier definition, repeated
/// and non-UTF-8 names included, after the null object. A repeated key
/// is a later definition no lookup finds.
#[test]
fn a_trait_name_finds_its_first_definitions_ordinal() {
    let mut traits = Traits::default();
    for (path, text) in [
        (
            "a.txt",
            &b"leader_traits = { brave = { } \xe9 = { } Brave = { } calm = { } }"[..],
        ),
        ("b.txt", b"leader_traits = { CALM = { } late = { } }"),
    ] {
        traits.register(text, path);
    }
    assert_eq!(traits.first_ordinal(b"BRAVE"), Ok(Some(1)));
    assert_eq!(traits.first_ordinal(b"calm"), Ok(Some(4)));
    assert_eq!(traits.first_ordinal(b"late"), Ok(Some(6)));
    assert_eq!(traits.first_ordinal(b"absent"), Ok(None));
}

/// First appearance takes the next index; lookup folds case and the
/// first spelling names the sub-unit.
#[test]
fn sub_unit_indices_follow_first_appearance_without_case() {
    let indices = sub_units(&[
            &b"sub_units = { Infantry = {} cavalry = {} }"[..],
            // `SUB_UNITS` folds; a key that is not UTF-8 still takes index 3.
            b"SUB_UNITS = { CAVALRY = {} @x = 1 \xfe = {} destroyer = {} } critical_parts = { part = {} }",
            b"\"sub_units\" = { quoted = {} }",
        ]);
    assert_eq!(indices.get("infantry"), SubUnit::Index(1));
    assert_eq!(indices.get("INFANTRY"), SubUnit::Index(1));
    assert_eq!(indices.get("Cavalry"), SubUnit::Index(2));
    assert_eq!(indices.get("destroyer"), SubUnit::Index(4));
    assert_eq!(indices.get("part"), SubUnit::Unknown);
    assert_eq!(indices.get("quoted"), SubUnit::Unknown);
    assert_eq!(indices.name(1), Some(Ok("Infantry")));
    assert!(indices.name(3).unwrap().is_err());
    assert!(indices.warnings().is_empty());
}

/// Byte names are hashed; invalid UTF-8 bytes neither collapse into each
/// other nor alias an authored Unicode replacement character.
#[test]
fn sub_unit_indices_keep_distinct_byte_names() {
    let indices = sub_units(&[b"sub_units = {
            x\xfe = {} x\xfd = {} x\xef\xbf\xbd = {} readable = {}
        }"]);
    assert!(indices.name(1).unwrap().is_err());
    assert!(indices.name(2).unwrap().is_err());
    assert_eq!(indices.get("x\u{fffd}"), SubUnit::Index(3));
    assert_eq!(indices.get("readable"), SubUnit::Index(4));
    assert_eq!(indices.name(3), Some(Ok("x\u{fffd}")));
}

#[test]
fn sub_unit_token_lookup_keeps_first_zero_token_name_and_null_fallback() {
    use crate::tokens::Token;
    let indices = sub_units(&[br#"sub_units = { Destroyer = {} 9later = {} -earlier = {} }"#]);
    assert_eq!(
        indices.token_name(&Token::register(b"DESTROYER")),
        Ok("Destroyer")
    );
    assert_eq!(indices.token_name(&Token::Rejected), Ok("9later"));
    assert_eq!(
        indices.token_name(&Token::register(b"not_a_subunit")),
        Ok("")
    );
    assert_eq!(SubUnits::default().token_name(&Token::Rejected), Ok(""));
    let uncertain = sub_units(&[
        b"sub_units={kept={}}",
        b"sub_units={bad={weights={x @variable=1}}}",
    ]);
    assert_eq!(uncertain.token_name(&Token::register(b"kept")), Ok("kept"));
    assert!(
        uncertain
            .token_name(&Token::register(b"unseen"))
            .unwrap_err()
            .contains("uncertain")
    );
}

/// `sub_units = no`: definition creation reads the tokens after it as
/// its keys, so `sub_units` itself becomes a sub-unit whose body holds
/// infantry and cavalry; a non-block entry's body takes the next entry;
/// a value without a key consumes the next token.
#[test]
fn a_scalar_sub_units_is_read_as_the_game_reads_it() {
    let indices = sub_units(&[
        &b"sub_units = { infantry = {} }"[..],
        b"sub_units = no sub_units = { infantry = {} cavalry = {} }",
        b"sub_units = { destroyer = {} }",
    ]);
    assert_eq!(indices.get("infantry"), SubUnit::Index(1));
    assert_eq!(indices.get("sub_units"), SubUnit::Index(2));
    assert_eq!(indices.get("cavalry"), SubUnit::Unknown);
    assert_eq!(indices.get("destroyer"), SubUnit::Index(3));
    assert!(indices.warnings()[0].contains("field `sub_units` is `no`"));
    let indices = sub_units(&[b"sub_units = { a = {} b = 5 c = {} }"]);
    assert_eq!(indices.get("b"), SubUnit::Index(2));
    assert_eq!(indices.get("c"), SubUnit::Unknown);
    let indices = sub_units(&[b"sub_units = { a = {} b c = {} }"]);
    assert_eq!(indices.get("b"), SubUnit::Index(2));
    assert_eq!(indices.get("c"), SubUnit::Unknown);
}

/// Keywords fold with lexer identity; modifier identifiers keep their
/// text, and a terrain adjuster takes the content's spelling.
#[test]
fn mixed_case_keywords_read_as_the_lowercase_ones() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let traits = leaders(
        &[br#"LEADER_TRAITS = {
                infantry_leader = {}
                Mixed_Trait = {
                    Attack_Skill = 2
                    MODIFIER = {
                        army_attack_factor = 0.05
                        FOREST = { ATTACK = 0.1 Movement = 0.05 }
                        Night = { Defence = 0.2 }
                        Custom_Modifier_Tooltip = my_tt
                        HIDDEN_MODIFIER = { army_org_factor = 0.1 }
                        Added_Modifier = { army_org_factor = 0.1 }
                        NAME = label
                    }
                    Sub_Unit_Modifiers = { destroyer = { UNITS = { Attack = 0.1 } } }
                    Trait_XP_Factor = { Infantry_Leader = 1 }
                    SLOT = army_chief
                    Specialist_Advisor_Trait = chief_a
                    Leader_Default_Proximity_Offset = 1
                    Override_Effect_Tooltip = over_tt
                    Custom_Effect_Tooltip = extra_tt
                    GUI_ROW = 1
                }
            }"#],
        &terrain_named(&["forest"]),
    )
    .traits;
    let effects = traits.get("mixed_trait").unwrap();
    assert_eq!(effects.name, "Mixed_Trait");
    assert!(effects.unsupported.is_empty(), "{:?}", effects.unsupported);
    assert!(effects.ignored.is_empty(), "{:?}", effects.ignored);
    assert_eq!(effects.skills[0], 2);
    assert_eq!(
        entries(&effects.blocks[0]),
        [("army_attack_factor".to_owned(), 5_000)]
    );
    assert_eq!(effects.blocks[0].tooltips(), ["my_tt"]);
    assert_eq!(effects.blocks[0].hidden.len(), 1);
    assert_eq!(effects.blocks[0].children.len(), 1);
    // Adjusters keep the registered spelling of their token.
    assert_eq!(
        effects
            .adjusters
            .iter()
            .map(|adjuster| (adjuster.key.as_str(), adjuster.attack, adjuster.defence))
            .collect::<Vec<_>>(),
        [("forest", 10_000, 0), ("night", 0, 20_000)]
    );
    assert_eq!(effects.adjusters[0].movement, 5_000);
    assert_eq!(effects.sub_units[0].adjuster.attack, 10_000);
    assert_eq!(
        effects.experience_factors,
        [("infantry_leader".to_owned(), 100_000)]
    );
    assert_eq!(effects.advisor_slot.as_deref(), Some("army_chief"));
    assert_eq!(
        effects.advisor_traits[0],
        Some(TraitReference::Name("chief_a".into()))
    );
    assert_eq!(effects.proximity_offset, 1);
    assert_eq!(effects.override_tooltips, ["over_tt"]);
    assert_eq!(effects.custom_tooltips, ["extra_tt"]);
}

/// Mixed-case `leader_skills`, `cost`, `type` and category keywords read
/// as the lowercase ones; what the game reports and goes on from is a
/// note.
#[test]
fn mixed_case_leader_skills_fold_and_notes_name_the_field() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mixed = skills(&[
        b"LEADER_SKILLS = { 1 = { TYPE = NAVY COST = 100 } 2 = { Type = Field_Marshal Cost = 7 } }",
    ]);
    assert_eq!(mixed.next_required_experience(0, 2), Some(Some(100)));
    assert_eq!(mixed.next_required_experience(1, 0), Some(Some(7)));
    assert!(mixed.warnings().is_empty());
    for (text, note) in [
        (
            &b"leader_skills = { x = { type = navy } 1 = { type = navy cost = 3 } }"[..],
            "leader_skills key x reads level 0, not a positive level; the game reports it and skips its block",
        ),
        (
            b"leader_skills = { 1 = { type = navy cost = 3 type = admiral } }",
            "leader_skills level 1 type admiral is not a leader-category keyword; the game reports it and the level keeps its category",
        ),
        (
            b"leader_skills = { 1 = { type = navy cost = 3 cost = lots } }",
            "leader_skills level 1 cost lots does not scan as an integer; the game reports it and keeps 3",
        ),
    ] {
        let skills = skills(&[text]);
        assert_eq!(skills.next_required_experience(0, 2), Some(Some(3)));
        assert_eq!(
            skills.warnings(),
            [format!("Commander skill levels: file0.txt: {note}.")]
        );
    }
    // A level given a scalar reads the next level's tokens as its fields.
    let skills = skills(&[b"leader_skills = { 1 = 5 type = navy cost = 9 }"]);
    assert_eq!(skills.next_required_experience(0, 2), Some(Some(9)));
}

/// A `type` that is no category keyword is reported and a block skipped:
/// the level keeps its category and the cost after the block is its own.
#[test]
fn an_invalid_skill_type_skips_its_block() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    for text in [
            &b"leader_skills = { 1 = { type = corps_commander cost = 5 type = { cost = 99 } cost = 7 } }"[..],
            b"LEADER_SKILLS = { 1 = { Type = Corps_Commander COST = 5 TYPE = { Cost = 99 } Cost = 7 } }",
        ] {
            let skills = skills(&[text]);
            assert_eq!(skills.next_required_experience(0, 1), Some(Some(7)));
        }
}

/// Where a file's reading cannot be followed, its unread rest could
/// hold another body of any registered trait, merging into that trait
/// (the name pass registered every file's names first), or another
/// `leader_skills`: every trait, one read before the gap or in another
/// file included, and the skill table are refused.
#[test]
fn a_gap_leaves_every_trait_and_the_skills_unknown() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let leaders = leaders(
            &[
                b"leader_skills = { 1 = { type = navy cost = 5 } }\nleader_traits = { a = { attack_skill = 1 }\nb = { on_add = { ai_message = { x @y = 1 } } } a = { attack_skill = 5 } }",
                b"leader_traits = { c = { attack_skill = 2 } }",
            ],
            &|_| None,
        );
    for name in ["a", "b", "c"] {
        let effects = leaders.traits.get(name).unwrap();
        assert!(
            effects.unsupported.iter().any(|reason| reason
                .starts_with("the game's reading of file0.txt cannot be followed from line 3")),
            "{name}: {:?}",
            effects.unsupported
        );
    }
    assert_eq!(leaders.skills.next_required_experience(0, 2), None);
}

/// `override_effect_tooltip` and `custom_effect_tooltip` are copied as
/// C strings (to the NUL, no line-feed check); `slot` is the lexer's
/// current token's text to its NUL, so a variable stays its name;
/// `custom_modifier_tooltip` reads one string to the NUL, a line feed
/// read as `Unreadable String`.
#[test]
fn string_fields_convert_as_their_readers_do() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let effects = effects(
        "custom_effect_tooltip = \"tip\0tail\" override_effect_tooltip = \"two\nlines\" \
             slot = \"army\0chief\" modifier = { custom_modifier_tooltip = \"a\nb\" CUSTOM_MODIFIER_TOOLTIP = \"c\0d\" }",
    );
    assert_eq!(effects.custom_tooltips, ["tip"]);
    assert_eq!(effects.override_tooltips, ["two\nlines"]);
    assert_eq!(effects.advisor_slot.as_deref(), Some("army"));
    let mut tooltips = effects.blocks[0].tooltips();
    tooltips.sort();
    assert_eq!(tooltips, ["Unreadable String", "c"]);
    let effects = self::effects("@s = chief slot = @s");
    assert_eq!(effects.advisor_slot.as_deref(), Some("@s"));
}

/// Resolve the lexer's current token: a bare name is looked up, a quoted
/// one is token 0xF and a variable's own token (resolution only updates
/// the value slot) names no trait either.
#[test]
fn advisor_traits_keep_the_current_tokens_kind() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let effects = effects(
        "Specialist_Advisor_Trait = chief expert_advisor_trait = \"quoted\" \
             @g = genius_name genius_advisor_trait = @g",
    );
    assert_eq!(
        effects.advisor_traits,
        [
            Some(TraitReference::Name("chief".into())),
            Some(TraitReference::Null("quoted".into())),
            Some(TraitReference::Null("@g".into())),
        ]
    );
}

/// Leader and sub-unit loaders list each folder's own `.txt` files:
/// a subdirectory's traits, skill costs and sub-units are not read.
#[cfg(feature = "test-support")]
#[test]
fn the_loaders_do_not_read_subdirectories() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
    let root = std::env::temp_dir().join(format!(
        "installed_content_unit_leader_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    for (path, text) in [
        (
            "common/unit_leader/a.txt",
            "leader_traits = { top = { attack_skill = 1 } }",
        ),
        (
            "common/unit_leader/sub/b.txt",
            "leader_traits = { deep = { } } leader_skills = { 1 = { type = navy cost = 5 } }",
        ),
        (
            "common/units/unit_modifiers/sub/c.txt",
            "sub_units = { hidden = { } }",
        ),
        ("common/units/d.txt", "sub_units = { infantry = { } }"),
    ] {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
    let leaders = Leaders::<crate::Diagnostic>::load(&layers, &|_| None).unwrap();
    let sub_units = super::sub_units(&layers).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert!(leaders.traits.get("top").is_some());
    assert!(leaders.traits.get("deep").is_none());
    assert_eq!(leaders.skills.next_required_experience(0, 2), Some(None));
    assert_eq!(sub_units.get("infantry"), SubUnit::Index(1));
    assert_eq!(sub_units.get("hidden"), SubUnit::Unknown);
}

// Direct catalogue coverage supplements effect fixtures.
#[test]
fn support_category_follows_lexer_current_raw_lists_and_redefinition_reset() {
    let units = sub_units(&[
        br#"@category=support sub_units={
            scalar={type=support}
            list={type={infantry support}}
            numeric={type=16777216}
            numeric_list={type={16777216}}
            alias_scalar={type=@category}
            alias_list={type={@category}}
            quoted={type="support"}
            repeated={type=infantry type=support}
            reset={type=support}
        }"#,
        b"sub_units={ReSet={}}",
    ]);
    for name in [b"scalar".as_slice(), b"list", b"alias_list", b"repeated"] {
        assert_eq!(units.support(name), Ok(true), "{:?}", name);
    }
    for name in [
        b"numeric".as_slice(),
        b"numeric_list",
        b"alias_scalar",
        b"quoted",
        b"reset",
    ] {
        assert_eq!(units.support(name), Ok(false), "{:?}", name);
    }
    assert_eq!(units.support(b"SCALAR"), Ok(true));
    assert_eq!(
        units.support(b"unknown"),
        Err("sub-unit unknown is not in the content's sub_units".into())
    );
}

// The category loop starts with an unconditional advance. The first
// `}` is itself a category token; support after it is still in this raw list.
#[test]
fn support_category_empty_list_takes_its_first_close_before_peeking_for_the_end() {
    let units = sub_units(&[b"sub_units={unit={type={} support}}"]);
    assert_eq!(units.support(b"unit"), Ok(true), "{:?}", units.warnings());
}

#[test]
fn an_unread_tail_leaves_support_unknown_until_a_later_definition_resets_it() {
    let mut units = sub_units(&[
        b"sub_units={a={type=support} kept={type=support}}",
        b"sub_units={bad={weights={x @variable=1}}}",
    ]);
    let refused = units.support(b"bad").unwrap_err();
    assert!(refused.contains("variable @variable"), "{refused}");
    assert_eq!(units.support(b"a"), Err(refused.clone()));
    assert_eq!(units.support(b"kept"), Err(refused.clone()));
    assert_eq!(units.support(b"unknown"), Err(refused));
    units.read(b"sub_units={A={} new={type=support}}", "recovered.txt");
    assert_eq!(units.support(b"a"), Ok(false));
    assert_eq!(units.support(b"new"), Ok(true));
    assert!(units.support(b"kept").is_err());
}

#[test]
fn inline_unit_icon_categories_follow_mask_or_and_reset() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let units = sub_units(&[
        br#"sub_units={
            land={type=infantry type={motorized support}}
            ship={type=screen_ship}
            quoted={type="screen_ship"}
            reset={type=screen_ship}
        }"#,
        b"sub_units={RESET={type=infantry}}",
    ]);
    let mask = |name| crate::equipment::category_mask(name).unwrap();
    assert_eq!(
        units.category(b"LAND"),
        Ok(mask("infantry") | mask("motorized") | mask("support"))
    );
    assert_eq!(units.category(b"ship"), Ok(mask("screen_ship")));
    assert_eq!(units.category(b"quoted"), Ok(0));
    assert_eq!(units.category(b"reset"), Ok(mask("infantry")));
    assert!(units.category(b"absent").unwrap_err().contains("absent"));
}

/// `special_forces` reads as `is_artillery_brigade` does: only an exact `yes` or `no`
/// assigns it, each appearance starts at `no`, and the last appearance gives it.
#[test]
fn special_forces_follows_the_last_appearance() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let units = sub_units(&[
        br#"sub_units={
            marine={special_forces=yes}
            line={special_forces=no}
            plain={}
            unclear={special_forces=yes special_forces=maybe}
            redefined={special_forces=yes}
        }"#,
        b"sub_units={REDEFINED={type=infantry}}",
    ]);
    assert_eq!(units.special_forces(b"MARINE"), Ok(true));
    assert_eq!(units.special_forces(b"line"), Ok(false));
    assert_eq!(units.special_forces(b"plain"), Ok(false));
    // A value that is neither `yes` nor `no` leaves the flag as it was.
    assert_eq!(units.special_forces(b"unclear"), Ok(true));
    assert_eq!(units.special_forces(b"redefined"), Ok(false));
    assert!(
        units
            .special_forces(b"absent")
            .unwrap_err()
            .contains("absent")
    );
}
