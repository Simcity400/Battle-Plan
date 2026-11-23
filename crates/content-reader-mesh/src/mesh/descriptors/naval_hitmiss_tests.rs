use super::{
    Descriptors, Entities, NavalHitMissEffect, Roster, parse_descriptors,
    state_naval_hitmiss_effects,
};
use content_reader_parser::parser::{Item, Kind, Operator, Value};

fn read(bytes: &[u8]) -> Descriptors {
    parse_descriptors(bytes, "gfx/entities/naval_test.asset").unwrap()
}

#[test]
fn naval_hitmiss_keeps_declared_rows_separate_from_events_and_entity_data() {
    let descriptors = read(
        br#"entity = { name = ship
            game_data = { texture_anim_speed = 2 atlas_size = { 3 4 } }
            state = { name = attack
                event = { time = 9 particle = muzzle }
                game_data = {
                    hitmiss_effect = { time = 0.6 type = big }
                    hitmiss_effect = { time = 0.1 type = small }
                    hitmiss_effect = { time = 0.1 type = small }
                }
                game_data = { hitmiss_effect = { type = big } }
            }
        }"#,
    );
    let entity = &descriptors.entities["ship"];
    let state = &entity.states[0];
    assert_eq!(
        state.naval_hitmiss_effects,
        [
            NavalHitMissEffect { time: 0.6, kind: 1 },
            NavalHitMissEffect { time: 0.1, kind: 0 },
            NavalHitMissEffect { time: 0.1, kind: 0 },
            NavalHitMissEffect { time: 0., kind: 1 },
        ]
    );
    assert_eq!(state.naval_hitmiss_error, None);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].time, 9.);
    assert_eq!(state.events[0].particle.as_deref(), Some("muzzle"));
    let data = entity.game_data.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(data.texture_anim_speed, 2.);
    assert_eq!(data.atlas_size, [3, 4]);
}

#[test]
fn naval_hitmiss_duplicate_fields_follow_order_crt_and_whole_type_strings() {
    let descriptors = read(
        br#"entity = { name = ship state = { name = attack game_data = {
            HITMISS_EFFECT = { TIME > "0.125tail" time = "0.5tail"
                TYPE = "BIG" type = unknown type = " small " }
            hitmiss_effect = { type = big type = SMALL type = unknown }
            hitmiss_effect = { type = unknown "type" = big }
        } } }"#,
    );
    let state = &descriptors.entities["ship"].states[0];
    assert_eq!(
        state.naval_hitmiss_effects,
        [
            NavalHitMissEffect { time: 0.5, kind: 1 },
            NavalHitMissEffect { time: 0., kind: 0 },
            NavalHitMissEffect { time: 0., kind: 0 },
        ]
    );
    assert_eq!(state.naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_state_carriers_fold_keywords_but_preserve_quoted_key_noops() {
    let descriptors = read(
        br#"entity = { name = ship
            STATE = { NAME = previous name = attack "name" = ignored
                ANIMATION = previous animation = firing "animation" = ignored
                GAME_DATA = { HITMISS_EFFECT = { TIME = 0.2 TYPE = BIG } }
                "game_data" = { hitmiss_effect = { time = 9 type = small } }
            }
            "state" = { name = quoted game_data = { hitmiss_effect = { time = 3 } } }
        }"#,
    );
    let states = &descriptors.entities["ship"].states;
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].name, "attack");
    assert_eq!(states[0].animation, "firing");
    assert_eq!(
        states[0].naval_hitmiss_effects,
        [NavalHitMissEffect { time: 0.2, kind: 1 }]
    );
    assert_eq!(states[0].naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_scalar_and_tagged_state_carriers_refuse_later_entities_in_the_file() {
    for carrier in [
        "scalar",
        "rgb { name = attack game_data = { hitmiss_effect = {} } }",
    ] {
        let text = format!(
            "entity = {{ name = before state = {carrier} }}
             entity = {{ name = after
                state = {{ name = attack game_data = {{ hitmiss_effect = {{ time = 0.4 }} }} }}
             }}"
        );
        let error = parse_descriptors(text.as_bytes(), "gfx/entities/refused.asset").unwrap_err();
        assert!(error.to_string().contains("gfx/entities/refused.asset"));
        assert!(
            error
                .to_string()
                .contains("scalar/tagged entity state carrier")
        );
    }
}

#[test]
fn naval_hitmiss_layered_roster_refuses_later_entity_names_from_an_unsettled_file() {
    for (index, (carrier, reason)) in [
        ("scalar", "scalar/tagged entity state carrier"),
        (
            "rgb { name = attack game_data = { hitmiss_effect = {} } }",
            "scalar/tagged entity state carrier",
        ),
        (
            "{ name = { nested = attack } }",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "{ name = attack animation = rgb { nested = firing } }",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "{ name = attack game_data = { hitmiss_effect = { time = { 1 } } } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "{ name = attack game_data = { hitmiss_effect = { type = { big } } } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "{ name = attack game_data = { hitmiss_effect = { @unknown = scalar } } }",
            "hitmiss_effect member has unsettled native consumption",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let text = format!(
            "entity = {{ name = before state = {carrier} }}
             entity = {{ name = after
                state = {{ name = attack game_data = {{ hitmiss_effect = {{ time = 0.4 }} }} }}
             }}"
        );
        let roster = crate::mesh::tests::roster_of(
            &format!("naval-hitmiss-carrier-{index}"),
            &[("gfx/entities/refused.asset", text.as_bytes())],
            &[],
        );
        assert!(roster.descriptors.entities.is_empty());
        assert!(roster.unreadable.iter().any(|message| {
            message.contains("gfx/entities/refused.asset") && message.contains(reason)
        }));
        for name in ["before", "after"] {
            assert_eq!(
                roster.unreadable_names.get(name).map(String::as_str),
                Some("gfx/entities/refused.asset")
            );
        }
    }
}

#[test]
fn naval_hitmiss_preflight_ignores_unrelated_nested_state_dsl_data() {
    let descriptors = read(
        br#"entity = { name = ship
            state = { name = attack game_data = { hitmiss_effect = { time = 0.2 } } }
            unrelated = { state = scalar }
        }
        unrelated = { state = rgb { name = attack } }"#,
    );
    assert_eq!(
        descriptors.entities["ship"].states[0].naval_hitmiss_effects,
        [NavalHitMissEffect { time: 0.2, kind: 0 }]
    );
}

#[test]
fn naval_hitmiss_state_text_keeps_whitespace_and_255_byte_truncation() {
    let name = "n".repeat(256);
    let animation = "a".repeat(256);
    let text = format!(
        "entity = {{ name = ship
            state = {{ name = \" attack \" animation = \" firing \" }}
            state = {{ name = \"{name}\" animation = \"{animation}\" }}
        }}"
    );
    let descriptors = read(text.as_bytes());
    let states = &descriptors.entities["ship"].states;
    assert_eq!(states[0].name, " attack ");
    assert_eq!(states[0].animation, " firing ");
    assert_eq!(states[1].name, "n".repeat(255));
    assert_eq!(states[1].animation, "a".repeat(255));
    assert_eq!(states[1].naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_unsettled_state_text_refuses_later_entities_in_the_file() {
    for (body, reason) in [
        (
            "name = @unknown",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "name = attack animation = @unknown",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "name = { nested = attack }",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "name = attack animation = { nested = firing }",
            "state name/animation input has unsettled native consumption",
        ),
        (
            "name = rgb { nested = attack }",
            "state name/animation input has unsettled native consumption",
        ),
    ] {
        let text = format!(
            "entity = {{ name = before state = {{ {body} }} }}
             entity = {{ name = after state = {{ name = attack game_data = {{
                hitmiss_effect = {{ time = 0.4 }}
             }} }} }}"
        );
        let error = parse_descriptors(text.as_bytes(), "gfx/entities/refused.asset").unwrap_err();
        assert!(error.to_string().contains(reason), "{body}: {error}");
        assert!(error.to_string().contains("descriptor file refused"));
    }
}

#[test]
fn naval_hitmiss_final_state_text_encoding_refusal_is_local_and_logged() {
    let name = format!("{}\u{e9}", "a".repeat(254));
    let text = format!(
        "entity = {{ name = before state = {{ name = \"{name}\" }} }}
         entity = {{ name = after state = {{ name = attack game_data = {{
            hitmiss_effect = {{ time = 0.4 }}
         }} }} }}"
    );
    let descriptors = read(text.as_bytes());
    let state = &descriptors.entities["before"].states[0];
    assert!(state.name.is_empty());
    assert!(
        state
            .naval_hitmiss_error
            .as_ref()
            .unwrap()
            .contains("255-byte text is not UTF-8")
    );
    let after = &descriptors.entities["after"].states[0];
    assert_eq!(after.name, "attack");
    assert_eq!(
        after.naval_hitmiss_effects,
        [NavalHitMissEffect { time: 0.4, kind: 0 }]
    );
    assert_eq!(after.naval_hitmiss_error, None);

    let document = content_reader_parser::parser::parse(text.as_bytes()).unwrap();
    let mut registry = Entities::default();
    let mut roster = Roster::default();
    registry
        .declare(
            &document.items[0].value,
            &content_reader_layers::script_files::ScriptPath::from("gfx/entities/encoding.asset"),
            &mut roster,
        )
        .unwrap();
    assert!(roster.logged.iter().any(|message| {
        message.contains("before in gfx/entities/encoding.asset")
            && message.contains("255-byte text is not UTF-8")
    }));
}

#[test]
fn naval_hitmiss_later_state_text_writes_recover_earlier_unrepresentable_truncation() {
    let truncated = format!("{}\u{e9}", "a".repeat(254));
    let text = format!(
        "entity = {{ name = ship state = {{
            name = \"{truncated}\" name = attack
            animation = \"{truncated}\" animation = firing
            game_data = {{ hitmiss_effect = {{ time = 0.4 type = big }} }}
        }} }}"
    );
    let descriptors = read(text.as_bytes());
    let state = &descriptors.entities["ship"].states[0];
    assert_eq!(state.name, "attack");
    assert_eq!(state.animation, "firing");
    assert_eq!(
        state.naval_hitmiss_effects,
        [NavalHitMissEffect { time: 0.4, kind: 1 }]
    );
    assert_eq!(state.naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_empty_records_append_zero_defaults_and_absence_is_empty() {
    let descriptors = read(
        br#"entity = { name = ship
            state = { name = idle }
            state = { name = move game_data = {} }
            state = { name = attack game_data = { hitmiss_effect = {} hitmiss_effect = {} } }
        }"#,
    );
    let states = &descriptors.entities["ship"].states;
    for state in &states[..2] {
        assert!(state.naval_hitmiss_effects.is_empty());
        assert_eq!(state.naval_hitmiss_error, None);
    }
    assert_eq!(
        states[2].naval_hitmiss_effects,
        [NavalHitMissEffect::default(); 2]
    );
    assert_eq!(states[2].naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_float_words_keep_infinity_and_nan_bits() {
    let descriptors = read(
        br#"entity = { name = ship state = { name = attack game_data = {
            hitmiss_effect = { time = "1e39" }
            hitmiss_effect = { time = "-nan(snan)" }
        } } }"#,
    );
    let state = &descriptors.entities["ship"].states[0];
    assert_eq!(state.naval_hitmiss_effects[0].time, f32::INFINITY);
    assert_eq!(state.naval_hitmiss_effects[1].time.to_bits(), 0xff80_0001);
    assert_eq!(state.naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_string_read_stops_at_nul_before_comparison() {
    let member = |key: &[u8], bytes: &[u8]| Item {
        key: Some(key.to_vec()),
        key_quoted: false,
        operator: Operator::Equals,
        value: Value::Scalar(bytes.to_vec()),
        kind: Kind::Quoted,
    };
    let block = |key: &[u8], fields| Item {
        key: Some(key.to_vec()),
        key_quoted: false,
        operator: Operator::Equals,
        value: Value::Block(fields),
        kind: Kind::Open,
    };
    let data = block(
        b"game_data",
        vec![block(
            b"hitmiss_effect",
            vec![
                member(b"time", b"0.25\0tail"),
                member(b"type", b"BIG\0ignored\nsmall"),
            ],
        )],
    );
    let (effects, error) = state_naval_hitmiss_effects(&[data]);
    assert_eq!(
        effects,
        [NavalHitMissEffect {
            time: 0.25,
            kind: 1
        }]
    );
    assert_eq!(error, None);
}

#[test]
fn naval_hitmiss_nul_descriptor_input_is_refused_by_the_tree_lexer() {
    let error = parse_descriptors(
        b"entity = { name = ship state = { name = attack game_data = {
            hitmiss_effect = { type = \"BIG\0ignored\" }
        } } }",
        "gfx/entities/naval_test.asset",
    )
    .unwrap_err();
    assert!(error.to_string().contains("NUL byte in quoted string"));
}

#[test]
fn naval_hitmiss_type_line_feed_keeps_previous_kind_without_poisoning_timings() {
    let descriptors = read(
        br#"entity = { name = ship state = { name = attack game_data = {
            hitmiss_effect = { type = big type = "small
            " }
        } } }"#,
    );
    let state = &descriptors.entities["ship"].states[0];
    assert_eq!(
        state.naval_hitmiss_effects,
        [NavalHitMissEffect { time: 0., kind: 1 }]
    );
    assert_eq!(state.naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_unsettled_data_forms_refuse_the_file_and_later_entities() {
    for (body, reason) in [
        (
            "game_data = scalar",
            "state game_data carrier is not a native block",
        ),
        (
            "game_data = rgb {}",
            "state game_data carrier is not a native block",
        ),
        (
            "game_data = { hitmiss_effect = scalar }",
            "hitmiss_effect carrier is not a native block",
        ),
        (
            "game_data = { hitmiss_effect = rgb {} }",
            "hitmiss_effect carrier is not a native block",
        ),
        (
            "game_data = { hitmiss_effect = { time = { 1 } } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { type = { big } } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { time = rgb {} } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { type = @size } }",
            "hitmiss_effect time/type input has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { ignored = { time = 1 } } }",
            "hitmiss_effect member has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { @unknown = scalar } }",
            "hitmiss_effect member has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { ignored = @unknown } }",
            "hitmiss_effect member has unsettled native consumption",
        ),
        (
            "game_data = { hitmiss_effect = { 1 } }",
            "hitmiss_effect member has unsettled native consumption",
        ),
        (
            "game_data = { ignored = { hitmiss_effect = {} } }",
            "state game_data member has unsettled native skip",
        ),
        (
            "game_data = { ignored = @unknown }",
            "state game_data member has unsettled native skip",
        ),
        (
            "game_data = { @unknown = scalar }",
            "state game_data member has unsettled native skip",
        ),
        (
            "game_data = { \"hitmiss_effect\" = {} }",
            "state game_data member has unsettled native skip",
        ),
    ] {
        let text = format!(
            "entity = {{ name = before state = {{ name = attack {body} }} }}
             entity = {{ name = after state = {{ name = attack game_data = {{
                hitmiss_effect = {{ time = 0.4 }}
             }} }} }}"
        );
        let error = parse_descriptors(text.as_bytes(), "gfx/entities/refused.asset").unwrap_err();
        assert!(error.to_string().contains(reason), "{body}: {error}");
        assert!(error.to_string().contains("descriptor file refused"));
    }
}

#[test]
fn naval_hitmiss_failed_literals_and_unknown_outer_scalars_preserve_usable_rows() {
    let descriptors = read(
        br#"entity = { name = ship state = { name = attack game_data = {
            ignored = scalar ignored = "@literal" "hitmiss_effect" = ignored
            hitmiss_effect = { time = 0.2 time = invalid time = "" type = big }
            hitmiss_effect = { time = 0.4 ignored = scalar "@literal" = scalar
                ignored = "@literal" type = small }
        } } }"#,
    );
    let state = &descriptors.entities["ship"].states[0];
    assert_eq!(
        state.naval_hitmiss_effects,
        [
            NavalHitMissEffect { time: 0.2, kind: 1 },
            NavalHitMissEffect { time: 0.4, kind: 0 },
        ]
    );
    assert_eq!(state.naval_hitmiss_error, None);
}

#[test]
fn naval_hitmiss_state_inheritance_keeps_parent_variants_and_replaces_declared_states() {
    let descriptors = read(
        br#"entity = { name = base
            state = { name = attack game_data = { hitmiss_effect = { time = 1 type = big } } }
            state = { name = defend game_data = { hitmiss_effect = { time = 2 type = small } } }
        }
        entity = { name = inherited clone = base }
        entity = { name = override clone = base
            state = { name = attack game_data = { hitmiss_effect = { time = 3 type = small } } }
        }"#,
    );
    let inherited = descriptors.inherit("inherited").unwrap();
    assert_eq!(
        inherited.states["attack"][0].naval_hitmiss_effects,
        [NavalHitMissEffect { time: 1., kind: 1 }]
    );
    let overridden = descriptors.inherit("override").unwrap();
    assert_eq!(
        overridden.states["attack"][0].naval_hitmiss_effects,
        [NavalHitMissEffect { time: 3., kind: 0 }]
    );
    assert_eq!(
        overridden.states["defend"][0].naval_hitmiss_effects,
        [NavalHitMissEffect { time: 2., kind: 0 }]
    );
}
