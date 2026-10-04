use super::*;
use crate::parser::Operator;

/// 141395B30 dispatches both bindings to 142390E40. Its block reader
/// 1423912A0 reads argument keys recursively, consuming definitions;
/// argument names that also name triggers are never conditions.
#[test]
fn custom_tooltip_bindings_read_localisation_before_conditions() {
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
    // A quoted key is not one of the binding keywords (1424A7A90).
    assert!(conditions[1].key_quoted);
    assert_eq!(
        conditions[1].key.as_deref(),
        Some(b"not_tooltip".as_slice())
    );
}

/// CFormattedLocalization::vf3 (142393E90) reads only the current
/// value. A block on localization_key is not skipped: the next key is
/// still the enclosing binding's, including a definition there.
#[test]
fn a_localisation_key_block_keeps_its_native_parse_boundary() {
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

/// LoadBody (1405431C0) hands LoadKey every key 1424AC5E0 reads, one
/// read without an operator included, with the value slot as the last
/// read left it: `OR {` takes `{` as its operator and keeps the slot's
/// `{` (from `trigger = {`), so the group reads the tokens after it as
/// its body. Likewise a modifier's `factor` goes to 140545820.
#[test]
fn a_key_without_an_operator_goes_to_its_trigger_reader() {
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

/// CPersistent::Read (1424A9730) reads a `name = value` tactic's fields
/// from the tokens after it, up to the next `}`; a block on a field
/// 1406B3CB0 reads as one value is read as `{`, its contents as the
/// tactic's fields and its `}` ends the tactic. An unknown key's block
/// is skipped (1424AD270).
#[test]
fn a_single_value_tactic_reads_the_rest_of_the_file() {
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
    // `first`'s `}` after `is_attacker = yes` ends it; `unknown` is a
    // tactic whose block is read as its fields, and the stray `}` a key.
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

/// `phases` and both `no` sentinels are compared by their text, exactly
/// (1406B3875, 1406B3D9C, 14047389A); field and trigger keys are
/// keywords and fold.
#[test]
fn phases_and_no_match_their_exact_text_while_keys_fold() {
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

/// ReadStringList (1401426E0) reads every token of `phases` to `}`,
/// `=` included; a variable name that is not UTF-8 is a name like any
/// other (a 0xFF byte would end the text: the lexer's `char` -1). A value
/// without a key at the top level is a key to 1424AC5E0, which consumes
/// the next token as its operator.
#[test]
fn unreadable_entries_are_named_and_mark_later_ids_unproven() {
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
    // `loose` takes `third` as its operator and the stale value.
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

/// A file variable takes no tactic ID and is no field; references read
/// its value. A second definition of a taken name reads nothing more
/// (1424AC830): its `=` goes to the next key.
#[test]
fn file_variables_take_no_id_and_resolve_in_fields() {
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
    // A picture named by a variable reads its value.
    let pictures = parse(
        b"@pic = attack\ntactic_a = { picture = @pic }",
        &KeyNames::default(),
    );
    let fields = &pictures.tactics[0].fields;
    assert_eq!(fields[0].value.scalar(), Some(b"attack".as_slice()));
}

/// A phase list entry naming a variable reads its value, so a tactic's
/// `phase = @p` and the list agree on the phase's name.
#[test]
fn phase_lists_resolve_file_variables() {
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

/// Phase fields resolve against the phases read before the tactic, `no`
/// is its own sentinel even when a phase is named `no`, and an absent or
/// unmatched field is unknown.
#[test]
fn phase_fields_resolve_in_read_order() {
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

/// Field keys match without case; a phase trigger stores the ID of a
/// phase registered before it, and -1 for `no` or a later phase.
#[test]
fn phase_keys_fold_and_triggers_resolve_in_read_order() {
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

/// The six effects as 1406B3CB0 reads them: keys fold, the last one
/// wins, `attacker_movement_speed` scans `%lli` (`010` is eight) and
/// the others read any text as 1424B8600 does ('e' is a digit to it, six
/// places are taken unscaled). A repeated name is a second tactic with
/// its own values; a quoted key is no keyword.
#[test]
fn effects_follow_each_fields_native_reader() {
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
    // An undefined reference is its own text, which reads zero.
    assert_eq!(effects(2)[4], 0);
    assert_eq!((effects(4)[5], effects(4)[4]), (203_300, 1));
    // A quoted "@bonus" is text, never a reference (1424AB540).
    assert_eq!(effects(5), [0; 6]);
    assert!(
        database.logged.iter().any(|line| line
            == "tactic tactic_b's defender names the variable @missing, which is not defined before it, so the game reads that text as the value"),
        "{:?}",
        database.logged
    );
    // `PHASES` is tactic 6; its bare names are keys without operators.
    assert_eq!(database.tactics[5].token, b"PHASES");
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

/// ReadFixed5 (1424ABB10) reports a value that does not scan and leaves
/// the field as it was, so an earlier assignment stands.
#[test]
fn an_unscannable_movement_value_keeps_the_previous_one() {
    let database = parse(
        b"t = { attacker_movement_speed = 0.5 ATTACKER_MOVEMENT_SPEED = fast }",
        &KeyNames::default(),
    );
    assert_eq!(database.tactics[0].effects[1], 50_000);
    assert_eq!(
        database.logged,
        [
            "tactic t's ATTACKER_MOVEMENT_SPEED = fast does not scan (1424B0480): the game reports it (1424ACEE0) and the field keeps its previous value"
        ]
    );
}

/// The file's one reader and its variables: a chain resolves when
/// defined and names keep their case (1424AC830); a definition in a
/// tactic body holds for the rest of the file; `phases` is a string list
/// (1401426E0) that defines nothing; `@(1+1)` is the reference `@` and
/// then `(`, `1+1` and `)` read as keys, the last consuming the tactic's
/// `}`.
#[test]
fn tactic_variables_follow_the_game_reader() {
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
    // `after` is read as a field of `expression`.
    assert_eq!(database.tactics.len(), 5);
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

/// The tactic's `trigger` (a CAndTrigger, 1405431C0) and `base` (a
/// CMeanTimeToHappen, 140545850) are read by key on the file's reader,
/// so their definitions register in document order and their values
/// resolve; an unknown field is skipped (1424ACF90 -> 1424AD270) and
/// defines nothing. A comparison keeps its operator (+0x78).
#[test]
fn definitions_inside_trigger_and_base_register() {
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

/// LoadBody (1405431C0) hands `x`, read without an operator (it
/// consumes `@y`), the stale `{`: what LoadKey makes of `x` decides what
/// it reads. Naming nothing, it is RecordError, whose skip tests the
/// `@y` it consumed, so nothing more is read and `@y` is never defined;
/// a scope reads the tokens after it as its body. Where the names cannot
/// tell, the tactic being read is unproven by name and no later tactic
/// is read.
#[test]
fn a_key_without_an_operator_is_read_as_what_it_names() {
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
        reason.contains("field `x`: LoadKey (1405434F0) reads it"),
        "{reason}"
    );
    assert_eq!(database.unsupported, [reason]);
}

/// 1405431C0 calls slot 6 after 1424AC5E0 even without an operator.
/// Bodies use the stale value; 1424AD270 skips on the current token.
/// Exercise both tactics' dispatch and content readers' Shape::Trigger.
#[test]
fn missing_operators_dispatch_scopes_classes_and_invalid_keys_in_both_readers() {
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

/// Embedded collection filters use the same CReader and LoadKey
/// (1405434F0), including its content names, as the enclosing body.
#[test]
fn collection_filters_keep_names_in_tactics_and_scripted_triggers() {
    let condition = "collection_size = { input = { input = collection:territories operators = { limit = { CONTROLLER = { always = yes } } } name = filtered } value = 0 }";
    let names = KeyNames::default();
    let tactics = parse(
        format!("t = {{ trigger = {{ {condition} }} }}").as_bytes(),
        &names,
    );
    assert!(tactics.unsupported.is_empty(), "{tactics:?}");
    assert!(tactics.logged.is_empty(), "{tactics:?}");
    let text = format!("check = {{ {condition} }}");
    let triggers = crate::scripted_triggers::ScriptedTriggers::parse(
        &[("collection.txt", text.as_bytes())],
        &names,
    );
    assert!(triggers.logged.is_empty(), "{triggers:?}");
    assert!(triggers.templates[b"check".as_slice()].body.is_ok());
}

/// Each nested reader consumes as its native reader does: the weight's
/// one-value fields and unknown keys never skip a block (140545850), so
/// a block there ends the weight early; a block under a trigger the
/// viewer evaluates is read as the enclosing trigger's children
/// (141395A40 / 141394DF0 -> 1424ACD40, no skip) and its `}` ends that
/// trigger. The reading goes on as the game's does and is noted; later
/// tactics are read.
#[test]
fn nested_readers_follow_their_own_consumption() {
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
    // Under IS_ATTACKER the block's `skill > 2` is the trigger's own.
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

/// A weight modifier's `factor` and `add` go to the CMTTHModifier's own
/// key reader (140545820), a scoped variable taking one value token
/// (140539440 -> 140539B50) with no block skipped: a block there is read
/// as `{`, its contents as the modifier's conditions, and its `}` ends
/// the modifier, so what follows is the weight's again.
#[test]
fn a_modifier_factor_block_ends_the_modifier() {
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
    // Later tactics are read as the game reads them.
    assert_eq!(database.tactics.len(), 2, "{:?}", database.tactics);
    assert!(database.tactics.iter().all(|t| t.unproven.is_none()));
}

/// A scalar `trigger` is logged by CTrigger::LoadBody (1405431C0 ->
/// 1424ACF90), which consumes nothing: its tactic keeps its other
/// fields. A scalar `modifier` is a modifier without a condition, and
/// the weight logs an unknown command. A quoted key is no keyword
/// (token 0xF): its block is skipped and defines nothing.
#[test]
fn a_scalar_trigger_is_a_local_error() {
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
            "combat_tactics.txt line 1: field `Trigger` is `yes` where its trigger reads a block: \"Non assign trigger is not enclosed in {}\" is reported (1405431C0) and nothing is read; read as the game reads it",
            "combat_tactics.txt line 1: field `modifier` is `yes` where its trigger reads a block: \"Non assign trigger is not enclosed in {}\" is reported (1405431C0) and nothing is read; read as the game reads it",
            "tactic a's Trigger = yes is not a block: the game's trigger reader (1405431C0) logs an error (1424ACF90) and reads no condition from it",
            "tactic a's base's modifier = yes is not a block: the game's modifier reader (1405431C0) logs an error (1424ACF90) and keeps a modifier with no condition, factor 1 and add 0",
            "tactic a's base's note = 1: the weight's reader (140545850) logs an unknown command and reads on",
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

/// 1424AC5E0 consumes a definition before handing the next key to the
/// trigger's reader (1405431C0), at any depth: none is a condition. A
/// comparison's operand, spaced or compact, is resolved as a phase
/// (140473850 -> 1406B35C0) and through the file's variables.
#[test]
fn consumed_definitions_are_no_conditions_and_comparisons_resolve() {
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

/// `meta_trigger`'s `text` is copied raw (141408C80): nothing in it
/// defines. A key that names nothing is "Invalid trigger" (1405434F0):
/// RecordError skips its block (1424ACF90 -> 1424AD270), nothing in it
/// read or defined.
#[test]
fn meta_text_is_raw_and_an_invalid_key_skips_its_block() {
    let database = parse(
        b"t = { trigger = { meta_trigger = { text = { @hidden = 1 } x = y } \
                any_unit = { infantry @skipped = 1 } } attacker = @hidden defender = @skipped }",
        &KeyNames::default(),
    );
    let trigger = database.tactics[0].fields[0].value.items().unwrap();
    assert_eq!(
        trigger[0].value.items().unwrap()[0].value,
        Value::Block(Vec::new())
    );
    assert_eq!(trigger[1].value, Value::Block(Vec::new()));
    assert_eq!(database.tactics[0].effects[2], 0);
    assert_eq!(database.tactics[0].effects[4], 0);
}

/// A group given a scalar is the empty group LoadKey keeps (1405434F0,
/// its LoadBody reading no child), at any depth of a tactic trigger or a
/// weight modifier; keys fold.
#[test]
fn scalar_groups_are_the_empty_groups_the_game_keeps() {
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
    // A quoted key is token 0xF, no group.
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

/// ReadStringList stores each phase through ReadStringInto (1401426E0
/// -> 1424ABB50): the text to its first NUL, and a text holding a line
/// feed as `Unreadable String` (reported, 1424ACEE0).
#[test]
fn phase_names_are_stored_as_read_string_into_stores_them() {
    let database = parse(
        b"phases = { open\x00ing \"line\nfeed\" }",
        &KeyNames::default(),
    );
    assert_eq!(
        database.phases,
        [b"open".to_vec(), crate::parser::UNREADABLE_STRING.to_vec()]
    );
}

/// The tactic file's reader dispatches through `Field::keyword`
/// (1424A7A90: a quoted key is token 0xF, a bare one folds); its one
/// text compare, `phases` (1406B3875), is an exact byte match.
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
    let code = include_str!("../combat_tactics.rs")
        .split("#[cfg(test)]")
        .next()
        .unwrap_or_default();
    let raw: Vec<_> = code
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//") && RAW.iter().any(|pattern| line.contains(pattern)))
        .collect();
    assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
}
