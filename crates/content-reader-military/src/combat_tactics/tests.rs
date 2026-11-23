use super::*;
use content_reader_parser::parser::Operator;
use content_reader_script::native::Shape;

#[test]
fn custom_tooltip_bindings_read_localisation_before_conditions() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { trigger = { custom_trigger_tooltip = { \
            tooltip = { localization_key = POSITIVE \
                @threshold = 2 OR = { @amount = 0.5 localization_key = ARG } } \
            NOT_TOOLTIP = NEGATIVE \
            skill > @threshold \"not_tooltip\" = yes } } attacker = @amount } \
          later = { }",
        &KeyNames::default(),
    );
    assert!(database.unsupported.is_empty(), "{database:?}");
    assert_eq!(database.tactics.len(), 2, "{database:#?}");
    assert_eq!(database.tactics[1].token, b"later");
    assert_eq!(database.tactics[0].effects[2], 50_000);
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    let conditions = trigger[0].value.items().unwrap();
    assert_eq!(conditions.len(), 2, "{conditions:?}");
    assert!(conditions[0].keyword_is("skill"));
    assert_eq!(conditions[0].value.scalar(), Some(b"2".as_slice()));
    assert!(conditions[1].key_quoted);
    assert_eq!(
        conditions[1].key.as_deref(),
        Some(b"not_tooltip".as_slice())
    );
}

#[test]
fn a_localisation_key_block_keeps_its_parse_boundary() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { trigger = { custom_trigger_tooltip = { \
            tooltip = { @threshold = 2 localization_key = { argument = KEY } \
            skill > @threshold } } attacker = 0.5 } later = { }",
        &KeyNames::default(),
    );
    assert!(database.unsupported.is_empty(), "{database:?}");
    assert_eq!(database.tactics.len(), 2, "{database:#?}");
    assert_eq!(database.tactics[0].effects[2], 50_000);
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    let conditions = trigger[0].value.items().unwrap();
    assert_eq!(conditions.len(), 1);
    assert!(conditions[0].keyword_is("skill"));
    assert_eq!(conditions[0].value.scalar(), Some(b"2".as_slice()));
    assert!(
        database
            .logged
            .iter()
            .any(|reason| reason.contains("localization_key"))
    );
}

#[test]
fn a_key_without_an_operator_goes_to_its_trigger_reader() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { trigger = { Or { is_attacker = yes } } attacker = 0.1 }",
        &KeyNames::default(),
    );
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    assert_eq!(trigger.len(), 1, "{trigger:?}");
    assert_eq!(trigger[0].key.as_deref(), Some(b"Or".as_slice()));
    let body = trigger[0].value.items().unwrap();
    assert_eq!(body[0].key.as_deref(), Some(b"is_attacker".as_slice()));
    assert_eq!(database.tactics[0].effects[2], 10_000);
    assert!(
        database
            .logged
            .iter()
            .any(|line| line.contains("key `Or` has no operator")),
        "{:?}",
        database.logged
    );
}

#[test]
fn ids_follow_every_named_non_phase_entry_in_source_order() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases={ opening breakthrough }\n\
          tactic_alpha={ active=yes }\n\
          tactic_alpha={ }\n\
          phases={ counterattack }\n\
          tactic_beta={ active=maybe }\n\
          Phases={}\n",
        &KeyNames::default(),
    );
    assert_eq!(
        database.phases,
        [
            b"opening".to_vec(),
            b"breakthrough".to_vec(),
            b"counterattack".to_vec()
        ]
    );
    assert_eq!(
        database
            .tactics
            .iter()
            .map(|tactic| (tactic.id, tactic.token.as_slice(), tactic.fields.len()))
            .collect::<Vec<_>>(),
        [
            (1, b"tactic_alpha".as_slice(), 1),
            (2, b"tactic_alpha".as_slice(), 0),
            (3, b"tactic_beta".as_slice(), 1),
            (4, b"Phases".as_slice(), 0),
        ]
    );
    assert!(database.unsupported.is_empty());
    assert!(database.logged.is_empty(), "{:?}", database.logged);
}

#[test]
fn a_single_value_tactic_reads_the_rest_of_the_file() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { opening }\n\
          first = { active = { is_attacker = yes } unknown = { x = 1 } }\n\
          second = value\n\
          phases = { later }\n\
          phase = opening\n\
          third = { active = no }\n",
        &KeyNames::default(),
    );
    assert_eq!(database.phases, [b"opening".to_vec()]);
    let keys = |tactic: &Tactic| -> Vec<Vec<u8>> {
        tactic
            .fields
            .iter()
            .map(|field| field.key.clone().unwrap())
            .collect()
    };
    assert_eq!(
        keys(&database.tactics[0]),
        [b"active".to_vec(), b"is_attacker".to_vec()]
    );
    assert_eq!(database.tactics[1].token, b"unknown");
    assert!(
        database.logged[0].contains("field `active` is a block where its reader takes one value"),
        "{:?}",
        database.logged
    );
    assert!(
        database
            .tactics
            .iter()
            .all(|tactic| tactic.unproven.is_none())
    );
}

#[test]
fn phases_and_no_match_their_exact_text_while_keys_fold() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { NO opening }\n\
          PHASES = { }\n\
          a = { Phase = NO Display_Phase = no \
                TRIGGER = { PHASE = NO Phase = no } }\n",
        &KeyNames::default(),
    );
    assert_eq!(database.phases, [b"NO".to_vec(), b"opening".to_vec()]);
    assert_eq!(
        database
            .tactics
            .iter()
            .map(|tactic| tactic.token.as_slice())
            .collect::<Vec<_>>(),
        [b"PHASES".as_slice(), b"a".as_slice()]
    );
    let a = &database.tactics[1];
    assert_eq!((a.phase, a.display_phase), (0, NO_PHASE));
    let trigger = a.fields[2].value.items().unwrap();
    let operands: Vec<_> = trigger
        .iter()
        .map(|item| item.value.scalar().unwrap())
        .collect();
    assert_eq!(operands, [b"0".as_slice(), b"-1".as_slice()]);
    assert!(database.unsupported.is_empty());
}

#[test]
fn unreadable_entries_are_named_and_mark_later_ids_unproven() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"first = { }\n\
          @\xe9 = 2\n\
          phases = { opening key = value }\n\
          second = { attacker = @\xe9 }\n\
          loose\n\
          third = { }\n",
        &KeyNames::default(),
    );
    assert_eq!(
        database.phases,
        [
            b"opening".to_vec(),
            b"key".to_vec(),
            b"=".to_vec(),
            b"value".to_vec()
        ]
    );
    assert_eq!(database.tactics[1].effects[2], 200_000);
    assert_eq!(database.tactics[2].token, b"loose");
    assert!(
        database
            .logged
            .iter()
            .any(|line| line.contains("key `loose` has no operator")),
        "{:?}",
        database.logged
    );
    assert!(
        database
            .tactics
            .iter()
            .all(|tactic| tactic.unproven.is_none())
    );
}

#[test]
fn file_variables_take_no_id_and_resolve_in_fields() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"@weight = 4\n\
          tactic_example = { base = { factor = @weight } @inner = 2 limit = @inner }\n\
          tactic_other = { base = { factor = @weight } }\n",
        &KeyNames::default(),
    );
    let ids: Vec<_> = database
        .tactics
        .iter()
        .map(|tactic| (tactic.id, tactic.token.as_slice()))
        .collect();
    assert_eq!(
        ids,
        [
            (1, b"tactic_example".as_slice()),
            (2, b"tactic_other".as_slice())
        ]
    );
    let example = &database.tactics[0].fields;
    assert_eq!(example.len(), 2);
    let factor = |fields: &[Item]| {
        fields[0].value.items().unwrap()[0]
            .value
            .scalar()
            .unwrap()
            .to_vec()
    };
    assert_eq!(factor(example), b"4");
    assert_eq!(example[1].value.scalar(), Some(b"2".as_slice()));
    assert_eq!(factor(&database.tactics[1].fields), b"4");
    let taken = parse(
        b"@x = 0.1 @x = 0.2 t = { attacker = @x }",
        &KeyNames::default(),
    );
    assert!(
        taken.logged[0].contains("variable `@x` is already taken"),
        "{:?}",
        taken.logged
    );
    let pictures = parse(
        b"@pic = attack\ntactic_a = { picture = @pic }",
        &KeyNames::default(),
    );
    let fields = &pictures.tactics[0].fields;
    assert_eq!(fields[0].value.scalar(), Some(b"attack".as_slice()));
}

#[test]
fn phase_lists_resolve_file_variables() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"@p = breakthrough
          phases = { opening @p }
          tactic_example = { phase = @p }
",
        &KeyNames::default(),
    );
    assert_eq!(
        database.phases,
        [b"opening".to_vec(), b"breakthrough".to_vec()]
    );
    let fields = &database.tactics[0].fields;
    assert_eq!(fields[0].value.scalar(), Some(b"breakthrough".as_slice()));
}

#[test]
fn phase_fields_resolve_in_read_order() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { opening no }\n\
          early = { phase = opening display_phase = later }\n\
          phases = { later }\n\
          late = { phase = no display_phase = later }\n\
          none = { }\n",
        &KeyNames::default(),
    );
    let ids: Vec<_> = database
        .tactics
        .iter()
        .map(|tactic| (tactic.phase, tactic.display_phase))
        .collect();
    assert_eq!(
        ids,
        [
            (0, UNKNOWN_PHASE),
            (NO_PHASE, 2),
            (UNKNOWN_PHASE, UNKNOWN_PHASE)
        ]
    );
}

#[test]
fn phase_keys_fold_and_triggers_resolve_in_read_order() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { opening }\n\
          early = { PHASE = opening DISPLAY_PHASE = opening \
                    trigger = { phase = later OR = { Phase = opening } } \
                    base = { modifier = { phase = no } } }\n\
          phases = { later }\n\
          late = { trigger = { phase = later } }\n",
        &KeyNames::default(),
    );
    assert_eq!(
        (database.tactics[0].phase, database.tactics[0].display_phase),
        (0, 0)
    );
    let operand = |tactic: usize, path: &[&str]| {
        let mut items = database.tactics[tactic].fields.as_slice();
        let (last, blocks) = path.split_last().unwrap();
        for key in blocks {
            items = items
                .iter()
                .find(|item| item.keyword_is(key))
                .unwrap()
                .value
                .items()
                .unwrap();
        }
        items
            .iter()
            .find(|item| item.keyword_is(last))
            .unwrap()
            .value
            .scalar()
            .unwrap()
            .to_vec()
    };
    assert_eq!(operand(0, &["trigger", "phase"]), b"-1");
    assert_eq!(operand(0, &["trigger", "OR", "Phase"]), b"0");
    assert_eq!(operand(0, &["base", "modifier", "phase"]), b"-1");
    assert_eq!(operand(1, &["trigger", "phase"]), b"1");
}

#[test]
fn raw_name_bytes_and_nested_fields_survive_parsing() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"tactic_\xe9={ allowed={ scripted=value } active=no }",
        &KeyNames::default(),
    );
    assert_eq!(database.tactics[0].id, 1);
    assert_eq!(database.tactics[0].token, b"tactic_\xe9");
    let fields = &database.tactics[0].fields;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].key.as_deref(), Some(b"allowed".as_slice()));
}

#[test]
fn effects_follow_each_fields_reader() {
    content_reader_script::test_game::enter();
    let database = parse(
        br#"@bonus = 0.25
            tactic_a = { ATTACKER = @bonus Combat_Width = 0.1 attacker_movement_speed = 010 }
            tactic_b = { defender = @missing }
            tactic_a = { attacker = 0.5 attacker = 0.75 }
            tactic_c = { defender_org_damage_modifier = 1.5e3 active = yes defender = 0.000001 }
            quoted = { defender = "@bonus" "attacker" = 0.5 }
            PHASES = { assault seize }"#,
        &KeyNames::default(),
    );
    let effects = |id: usize| database.tactics[id - 1].effects;
    assert_eq!(effects(1), [10_000, 800_000, 25_000, 0, 0, 0]);
    assert_eq!(effects(3)[2], 75_000);
    assert_eq!(effects(2)[4], 0);
    assert_eq!((effects(4)[5], effects(4)[4]), (203_300, 1));
    assert_eq!(effects(5), [0; 6]);
    assert!(
        database.logged.iter().any(|line| line
            == "tactic tactic_b's defender names the variable @missing, which is not defined before it, so the game reads that text as the value"),
        "{:?}",
        database.logged
    );
    assert_eq!(database.tactics[5].token, b"PHASES");
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

#[test]
fn an_unscannable_movement_value_keeps_the_previous_one() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { attacker_movement_speed = 0.5 ATTACKER_MOVEMENT_SPEED = fast }",
        &KeyNames::default(),
    );
    assert_eq!(database.tactics[0].effects[1], 50_000);
    assert_eq!(
        database.logged,
        [
            "tactic t's ATTACKER_MOVEMENT_SPEED = fast does not scan: the game reports it and the field keeps its previous value"
        ]
    );
}

#[test]
fn tactic_variables_follow_the_game_reader() {
    content_reader_script::test_game::enter();
    let database = parse(
        br#"@w = 0.1
            @W = 0.2
            @chain = @w
            phases = { @unregistered = 1 }
            a = { combat_width = @chain attacker = @W }
            b = { @inner = 0.3 attacker = @inner }
            c = { attacker = @inner }
            unregistered = { defender = @unregistered }
            expression = { attacker = @(1+1) }
            after = { attacker = 0.1 }"#,
        &KeyNames::default(),
    );
    let effect = |id: usize, slot: usize| database.tactics[id - 1].effects[slot];
    assert_eq!((effect(1, 0), effect(1, 2)), (10_000, 20_000));
    assert_eq!(effect(2, 2), 30_000);
    assert_eq!(effect(3, 2), 30_000);
    assert_eq!(effect(4, 4), 0);
    assert_eq!(
        database.phases,
        [b"@unregistered".to_vec(), b"=".to_vec(), b"1".to_vec()]
    );
    assert_eq!(database.tactics.len(), 5);
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

#[test]
fn definitions_inside_trigger_and_base_register() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { attacker = @late trigger = { OR = { @late = 0.1 always = yes } phase = no } \
          base = { factor = 1 modifier = { @weight = 0.2 factor = 2 } } unknown = { @skipped = 1 x = 1 } } \
          u = { attacker = @late defender = @weight combat_width = @skipped \
                trigger = { skill > @late skill_advantage<@weight } }", &KeyNames::default(),
    );
    assert!(
        database.unsupported.is_empty(),
        "{:?}",
        database.unsupported
    );
    assert_eq!(database.tactics[0].effects[2], 0);
    let u = &database.tactics[1];
    assert_eq!(
        (u.effects[2], u.effects[4], u.effects[0]),
        (10_000, 20_000, 0)
    );
    let trigger = u.fields[3].value.items().unwrap();
    let compared: Vec<_> = trigger
        .iter()
        .map(|item| {
            (
                item.key.clone().unwrap(),
                item.operator,
                item.value.scalar().unwrap().to_vec(),
            )
        })
        .collect();
    assert_eq!(
        compared,
        [
            (b"skill".to_vec(), Operator::Greater, b"0.1".to_vec()),
            (b"skill_advantage".to_vec(), Operator::Less, b"0.2".to_vec()),
        ]
    );
}

#[test]
fn a_key_without_an_operator_is_read_as_what_it_names() {
    content_reader_script::test_game::enter();
    let text = b"s = { attacker = 0.1 } t = { trigger = { x @y = 1 } } u = { attacker = @y }";
    let database = parse(text, &KeyNames::default());
    assert!(
        database.unsupported.is_empty(),
        "{:?}",
        database.unsupported
    );
    assert_eq!(database.tactics.len(), 3);
    let scope = KeyNames {
        characters: [b"x".to_vec()].into(),
        ..KeyNames::default()
    };
    let database = parse(text, &scope);
    assert_eq!(database.tactics.len(), 2);
    let trigger = database.tactics[1].fields[0].value.items().unwrap();
    assert_eq!(trigger[0].value.items().map(<[Item]>::len), Some(1));
    let database = parse(text, &KeyNames::unknown());
    assert_eq!(database.tactics.len(), 2);
    assert_eq!(database.tactics[0].unproven, None);
    let reason = database.tactics[1].unproven.clone().unwrap();
    assert!(
        reason.contains("field `x`: trigger dispatch reads it"),
        "{reason}"
    );
    assert_eq!(database.unsupported, [reason]);
}

#[test]
fn missing_operators_dispatch_scopes_classes_and_invalid_keys_in_both_readers() {
    content_reader_script::test_game::enter();
    let names = KeyNames {
        countries: [b"ABC".to_vec()].into(),
        ..KeyNames::default()
    };
    for (key, body, skipped) in [
        ("ABC", true, false),
        ("OR", true, false),
        ("always", false, false),
        ("invalid_test_key", false, true),
    ] {
        let text = format!(
            "t = {{ trigger = {{ {key} {{ always = yes @seen = 7 }} tag = @seen }} final = @seen"
        );
        let mut shaper = Shaper::new(Some(&names));
        let tactics = native::read(
            text.as_bytes(),
            native::Mode::KeepMark,
            native::Top::File(Node::Top),
            Some(&names),
            &mut |node, field| shaper.shape(node, field),
        );
        let content = native::read(
            text.as_bytes(),
            native::Mode::KeepMark,
            native::Top::File(0_u8),
            Some(&names),
            &mut |node, field| match (node, field.keyword().as_deref()) {
                (0, Some(b"t")) => Shape::Object(1),
                (1, Some(b"trigger")) => Shape::Trigger,
                _ => Shape::One,
            },
        );
        for read in [&tactics, &content] {
            assert!(read.unsettled.is_none(), "{key}: {:?}", read.unsettled);
            let t = read.items[0].value.items().unwrap();
            let children = t[0].value.items().unwrap();
            assert_eq!(children[0].key.as_deref(), Some(key.as_bytes()));
            assert_eq!(children[0].value.items().is_some(), body || skipped);
            if body {
                assert_eq!(
                    children[0].value.items().unwrap()[0].key.as_deref(),
                    Some(b"always".as_slice())
                );
                assert_eq!(children.len(), 2);
                assert_eq!(children[1].value.scalar(), Some(b"7".as_slice()));
            } else if skipped {
                assert!(children[0].value.items().unwrap().is_empty());
                assert_eq!(children.len(), 2);
                assert_eq!(children[1].value.scalar(), Some(b"@seen".as_slice()));
            } else {
                assert_eq!(children[0].value.scalar(), Some(b"{".as_slice()));
                assert_eq!(children[1].key.as_deref(), Some(b"always".as_slice()));
            }
            assert!(
                read.misreads
                    .iter()
                    .any(|note| note.what.contains("has no operator"))
            );
        }
    }
}

#[test]
fn collection_filters_keep_names_in_tactics_and_scripted_triggers() {
    content_reader_script::test_game::enter();
    let condition = "collection_size = { input = { input = collection:territories operators = { limit = { CONTROLLER = { always = yes } } } name = filtered } value = 0 }";
    let names = KeyNames::default();
    let tactics = parse(
        format!("t = {{ trigger = {{ {condition} }} }}").as_bytes(),
        &names,
    );
    assert!(tactics.unsupported.is_empty(), "{tactics:?}");
    assert!(tactics.logged.is_empty(), "{tactics:?}");
    let text = format!("check = {{ {condition} }}");
    let triggers = content_reader_script::scripted_trigger_grammar::ScriptedTriggers::parse(
        &[("collection.txt", text.as_bytes())],
        &names,
    );
    assert!(triggers.logged.is_empty(), "{triggers:?}");
    assert!(triggers.templates[b"check".as_slice()].body.is_ok());
}

#[test]
fn nested_readers_follow_their_own_consumption() {
    content_reader_script::test_game::enter();
    for text in [
        &b"s = { } a = { base = { factor = { factor = 2 } } } b = { }"[..],
        b"s = { } a = { BASE = { Mystery = { x = 1 } } } b = { }",
        b"s = { } a = { base = { Days = { 1 } } } b = { }",
        b"s = { } a = { trigger = { IS_ATTACKER = { skill > 2 } } } b = { }",
        b"s = { } a = { base = { modifier = { OR = { skill > { 1 } } } } } b = { }",
    ] {
        let database = parse(text, &KeyNames::default());
        let shown = String::from_utf8_lossy(text);
        assert!(
            database
                .logged
                .iter()
                .any(|line| line.contains("is a block where its reader takes one value")),
            "{shown}: {:?}",
            database.logged
        );
        assert!(
            database.tactics.iter().all(|t| t.unproven.is_none()),
            "{shown}"
        );
        assert_eq!(database.tactics[0].token, b"s", "{shown}");
    }
    let database = parse(
        b"a = { trigger = { IS_ATTACKER = { skill > 2 } } attacker = 0.1 }",
        &KeyNames::default(),
    );
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    assert_eq!(
        trigger
            .iter()
            .map(|item| item.key.clone().unwrap())
            .collect::<Vec<_>>(),
        [b"IS_ATTACKER".to_vec(), b"skill".to_vec()]
    );
}

#[test]
fn a_modifier_factor_block_ends_the_modifier() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"a = { base = { modifier = { FACTOR = { is_attacker = yes } skill > 2 } attacker = 0.1 }\n\
          b = { base = { modifier = { add = { has_reserves = yes } } }\n", &KeyNames::default(),
    );
    assert!(
        database
            .logged
            .iter()
            .any(|line| line
                .contains("field `FACTOR` is a block where its reader takes one value")),
        "{:?}",
        database.logged
    );
    let base = database.tactics[0].fields[0].value.items().unwrap();
    let keys = |items: &[Item]| -> Vec<Vec<u8>> {
        items.iter().map(|item| item.key.clone().unwrap()).collect()
    };
    assert_eq!(keys(base), [b"modifier".to_vec(), b"skill".to_vec()]);
    assert_eq!(
        keys(&database.tactics[0].fields),
        [b"base".to_vec(), b"attacker".to_vec()]
    );
    let modifier = base[0].value.items().unwrap();
    assert_eq!(
        keys(modifier),
        [b"FACTOR".to_vec(), b"is_attacker".to_vec()]
    );
    assert_eq!(modifier[0].value.scalar(), Some(b"{".as_slice()));
    assert_eq!(database.tactics.len(), 2, "{:?}", database.tactics);
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

#[test]
fn a_scalar_trigger_is_a_local_error() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"a = { trigger = { always = yes } Trigger = yes attacker = 0.1 \
                base = { modifier = yes factor = 2 note = 1 } } \
          b = { attacker = 0.2 \"trigger\" = { @hidden = 0.3 } defender = @hidden }",
        &KeyNames::default(),
    );
    assert!(
        database.unsupported.is_empty(),
        "{:?}",
        database.unsupported
    );
    assert_eq!(
        database.logged,
        [
            "combat_tactics.txt line 1: field `Trigger` is `yes` where its trigger reads a block: the missing block is reported and nothing is read; read as the game reads it",
            "combat_tactics.txt line 1: field `modifier` is `yes` where its trigger reads a block: the missing block is reported and nothing is read; read as the game reads it",
            "tactic a's Trigger = yes is not a block: the trigger reader reports an error and reads no condition from it",
            "tactic a's base's modifier = yes is not a block: the modifier reader reports an error and keeps a modifier with no condition, factor 1 and add 0",
            "tactic a's base's note = 1: the weight reader reports an unknown command and reads on",
            "tactic b's defender names the variable @hidden, which is not defined before it, so the game reads that text as the value",
        ]
    );
    let a = &database.tactics[0];
    let keys: Vec<_> = a
        .fields
        .iter()
        .map(|field| field.key.clone().unwrap())
        .collect();
    assert_eq!(
        keys,
        [b"trigger".to_vec(), b"attacker".to_vec(), b"base".to_vec()]
    );
    let base = a.fields[2].value.items().unwrap();
    assert_eq!(base[0].value, Value::Block(Vec::new()));
    assert_eq!(a.effects[2], 10_000);
    let b = &database.tactics[1];
    assert_eq!((b.effects[2], b.effects[4]), (20_000, 0));
}

#[test]
fn consumed_definitions_are_no_conditions_and_comparisons_resolve() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { 99 opening }\n\
          t = { trigger = { @x = yes is_attacker = @x OR = { @y = 1 phase>99 } } \
                base = { modifier = { @z = 2 factor = @z PHASE < opening } } }",
        &KeyNames::default(),
    );
    assert!(
        database.unsupported.is_empty(),
        "{:?}",
        database.unsupported
    );
    let fields = &database.tactics[0].fields;
    let shown = |items: &[Item]| -> Vec<String> {
        items
            .iter()
            .map(|item| {
                format!(
                    "{}{}{}",
                    String::from_utf8_lossy(item.key.as_deref().unwrap()),
                    String::from_utf8_lossy(item.operator.text()),
                    match &item.value {
                        Value::Scalar(text) => String::from_utf8_lossy(text).into_owned(),
                        _ => "{}".into(),
                    }
                )
            })
            .collect()
    };
    let trigger = fields[0].value.items().unwrap();
    assert_eq!(shown(trigger), ["is_attacker=yes", "OR={}"]);
    assert_eq!(shown(trigger[1].value.items().unwrap()), ["phase>0"]);
    let modifier = fields[1].value.items().unwrap()[0].value.items().unwrap();
    assert_eq!(shown(modifier), ["factor=2", "PHASE<1"]);
}

#[test]
fn meta_text_is_raw_and_an_invalid_key_skips_its_block() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { trigger = { meta_trigger = { text = { @hidden = 1 } x = y } \
                any_unit = { infantry @skipped = 1 } } attacker = @hidden defender = @skipped }",
        &KeyNames::default(),
    );
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    assert_eq!(
        trigger[0].value.items().unwrap()[0].value,
        Value::Scalar(b" @hidden = 1 ".to_vec())
    );
    assert_eq!(trigger[1].value, Value::Block(Vec::new()));
    assert_eq!(database.tactics[0].effects[2], 0);
    assert_eq!(database.tactics[0].effects[4], 0);
}

#[test]
fn scalar_groups_are_the_empty_groups_the_game_keeps() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"t = { trigger = { AND = yes OR = { or = no } Not = yes CUSTOM_TRIGGER_TOOLTIP = x \"and\" = yes } \
                base = { modifier = { or = no } } }", &KeyNames::default(),
    );
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    assert_eq!(trigger[0].value, Value::Block(Vec::new()));
    assert_eq!(
        trigger[1].value.items().unwrap()[0].value,
        Value::Block(Vec::new())
    );
    assert_eq!(trigger[2].value, Value::Block(Vec::new()));
    assert_eq!(trigger[3].value, Value::Block(Vec::new()));
    assert_eq!(trigger[4].value.scalar(), Some(b"yes".as_slice()));
    let modifier = database.tactics[0].fields[1].value.items().unwrap()[0]
        .value
        .items()
        .unwrap();
    assert_eq!(modifier[0].value, Value::Block(Vec::new()));
    assert_eq!(
        database
            .logged
            .iter()
            .filter(|line| line.contains("the game keeps the empty group"))
            .count(),
        5,
        "{:?}",
        database.logged
    );
}

#[test]
fn phase_names_keep_first_nul_text_and_line_feed_fallback() {
    content_reader_script::test_game::enter();
    let database = parse(
        b"phases = { open\x00ing \"line\nfeed\" }",
        &KeyNames::default(),
    );
    assert_eq!(
        database.phases,
        [b"open".to_vec(), content_reader_parser::parser::UNREADABLE_STRING.to_vec()]
    );
}

#[test]
fn the_tactic_reader_asks_the_keyword_helper() {
    const RAW: [&str; 6] = [
        "key.as_deref() ==",
        "key.as_deref() !=",
        "eq_ignore_ascii_case(",
        "to_ascii_lowercase(",
        ".get(b\"",
        "get_all(",
    ];
    let raw: Vec<_> = [
        include_str!("../combat_tactics.rs"),
        include_str!("../../../content-reader-script/src/trigger_grammar.rs"),
    ]
        .into_iter()
        .flat_map(|code| code.split("#[cfg(test)]").next().unwrap_or_default().lines())
        .map(str::trim)
        .filter(|line| !line.starts_with("//") && RAW.iter().any(|pattern| line.contains(pattern)))
        .collect();
    assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
}
