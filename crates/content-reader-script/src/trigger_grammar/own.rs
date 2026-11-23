
use super::{Node, Shaper};
use crate::native::{Field, Shape};

impl Shaper<'_> {
    pub(super) fn own_value(&self, token: u32, address: u64, field: &Field<'_>) -> Shape<Node> {
        match address {
            0x1404540c0 | 0x141446060 | 0x14049F250 | 0x14049F6A0 | 0x14049FAF0 | 0x14049FF40
            | 0x1404A0390 => Shape::Body(Node::Class(token)),
            0x140454140 | 0x140454290 | 0x140454870 | 0x1414461e0 | 0x14046ACF0 | 0x14046ADF0 => {
                Shape::ObjectIfOpen(Node::Class(token))
            }
            0x140490D60 => Shape::RawList,
            0x14056E240 => Shape::Expression,
            0x140454190 | 0x1404545b0 | 0x140454860 | 0x1404C1810 => Shape::One,
            0x140490D00 => Shape::Current,
            0x1404540d0 if field.opens() => Shape::SkipIfOpen,
            0x1404540d0 => Shape::One,
            0x140477CA0
                if matches!(
                    field.operator,
                    content_reader_parser::parser::Operator::Less | content_reader_parser::parser::Operator::Greater
                ) =>
            {
                Shape::One
            }
            0x140477CA0 => Shape::Body(Node::Class(token)),
            0x140477CC0 if field.opens() => Shape::Body(Node::Class(token)),
            0x140477CC0 => Shape::One,
            0x140497D20 if !field.opens() => Shape::One,
            _ => Self::refuse_reader(address, field.key.c_str()),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn register_non_damaged_building_level() {
    crate::test_game::with(|game| {
        game.trigger_at(
            0x3BE4,
            "non_damaged_building_level",
            0,
            crate::trigger_keys::Reader::Own(0x1404540c0),
            0x140491A40,
        )
    });
}

#[cfg(any(test, feature = "test-support"))]
pub fn register_owns_any_state_of() {
    crate::test_game::with(|game| {
        game.trigger(
            "owns_any_state_of",
            0,
            crate::trigger_keys::Reader::Own(0x140490D60),
            0,
        )
    });
}

#[cfg(test)]
mod tests {
    use crate::{key_names::KeyNames, native::{self, Shape}, trigger_keys::Reader};

    fn own(rows: &[(&str, u64, u64)]) {
        crate::test_game::with(|game| {
            rows.iter().fold(game, |game, &(name, address, key)| {
                game.trigger(name, 0, Reader::Own(address), key)
            })
        });
    }

    #[test]
    fn non_damaged_building_level_rejects_provincial_buildings() {
        super::register_non_damaged_building_level();
        let mut names = KeyNames::default();
        for (name, provincial) in [(b"factory".as_slice(), false), (b"port".as_slice(), true)] {
            names.counts.buildings.insert(name.to_vec());
            names
                .province_buildings
                .insert(name.to_vec(), Some(provincial));
        }
        let accepted = crate::triggers::read(
            b"non_damaged_building_level = { building = factory level > 2 }",
            &names,
        );
        assert!(accepted.unsettled.is_none(), "{accepted:?}");
        assert!(accepted.misreads.is_empty(), "{accepted:?}");
        for building in ["port", "missing"] {
            let text = format!(
                "non_damaged_building_level = {{ building = {building} building {{ @lost = 7 level > 0 }} level > 2 }} always = @lost"
            );
            let refused = crate::triggers::read(text.as_bytes(), &names);
            assert!(refused.unsettled.is_none(), "{refused:?}");
            let probe = refused.items.last().unwrap();
            assert!(probe.keyword_is("always"), "{refused:?}");
            assert_eq!(
                probe.value.scalar(),
                Some(b"@lost".as_slice()),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn completed_mio_trait_count_reads_its_assignment_body() {
        own(&[("has_mio_number_of_completed_traits", 0x140477CA0, 0x140477D70)]);
        let result = crate::triggers::read(
            b"has_mio_number_of_completed_traits = { @kept = 2 value > 3 } always = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none(), "{result:?}");
        assert!(result.misreads.is_empty(), "{result:?}");
        assert_eq!(result.items.len(), 2, "{result:?}");
        let value = &result.items[0].value.items().unwrap()[0];
        assert!(value.keyword_is("value"));
        assert_eq!(value.operator, content_reader_parser::parser::Operator::Greater);
        assert_eq!(result.items[1].value.scalar(), Some(b"2".as_slice()));
    }

    #[test]
    fn mio_trait_leaves_read_a_scalar_or_their_block_keys() {
        own(&[
            ("has_mio_trait", 0x140477CC0, 0x140319F20),
            ("is_mio_trait_completed", 0x140477CC0, 0x140319F20),
            ("is_mio_trait_available", 0x140477CC0, 0x140477D90),
        ]);
        for (text, keys) in [
            (
                "has_mio_trait = { trait = a show_modifiers = yes }",
                &["trait", "show_modifiers"][..],
            ),
            ("is_mio_trait_completed = { trait = a }", &["trait"]),
            (
                "is_mio_trait_available = { trait = a check_mio_parent_completed = no check_mio_mutually_exclusive = no }",
                &["trait", "check_mio_parent_completed", "check_mio_mutually_exclusive"],
            ),
            (
                "has_mio_trait = { check_mio_parent_completed = { trait = b } trait = a }",
                &["check_mio_parent_completed", "trait"],
            ),
        ] {
            let result = crate::triggers::read(
                format!("{text} always = yes").as_bytes(),
                &KeyNames::default(),
            );
            assert!(result.unsettled.is_none(), "{text}: {result:?}");
            assert!(result.misreads.is_empty(), "{text}: {result:?}");
            assert_eq!(result.items.len(), 2, "{text}: {result:?}");
            let block = result.items[0].value.items().unwrap();
            assert_eq!(block.len(), keys.len(), "{text}: {result:?}");
            for (field, key) in block.iter().zip(keys) {
                assert!(field.keyword_is(key), "{text}: {result:?}");
                if let Some(skipped) = field.value.items() {
                    assert!(skipped.is_empty(), "{text}: {result:?}");
                }
            }
        }
        let result = crate::triggers::read(b"has_mio_trait = a", &KeyNames::default());
        assert!(result.unsettled.is_none(), "{result:?}");
        assert_eq!(result.items[0].value.scalar(), Some(b"a".as_slice()));
    }

    fn read(text: &[u8]) -> native::Read {
        native::read(
            text,
            native::Mode::KeepMark,
            native::Top::File(()),
            Some(&KeyNames::default()),
            &mut |_, field| {
                if field.keyword_is("body") {
                    Shape::Trigger
                } else {
                    Shape::One
                }
            },
        )
    }

    #[test]
    fn own_key_loops_skip_unknown_blocks_without_registering_definitions() {
        let rows = [
            ("synthetic_flag", 0x1414461e0, 0x1405972d0, "flag"),
            ("synthetic_target_value", 0x1404540c0, 0x141446BF0, "target"),
            ("synthetic_target_type", 0x140454870, 0x1405972d0, "type"),
            ("synthetic_scorer", 0x14049F250, 0x1404A0950, "var"),
            ("synthetic_array_scorer", 0x14049F6A0, 0x1404A0990, "array"),
            ("synthetic_supply", 0x14049FAF0, 0x1404A09E0, "type"),
            ("synthetic_random", 0x1404A0390, 0x1404A0A40, "min"),
            ("synthetic_preparation", 0x140454290, 0x1405972d0, "state"),
            ("synthetic_modifier_scope", 0x140454140, 0x140457B50, "modifier"),
            ("synthetic_character", 0x14046ACF0, 0x14046AE20, "character"),
            ("synthetic_province", 0x14046ADF0, 0x14046AE50, "province"),
        ];
        own(&rows.map(|(name, address, key, _)| (name, address, key)));
        for (class, key) in rows.map(|(name, _, _, key)| (name, key)) {
            let text = format!(
                "body = {{ {class} = {{ {key} = x invalid = {{ @lost = 2 }} @kept = 3 {key} = @kept }} }} lost = @lost kept = @kept"
            );
            let result = read(text.as_bytes());
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            assert_eq!(
                result.items[1].value.scalar(),
                Some(&b"@lost"[..]),
                "{class}"
            );
            assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]), "{class}");
        }
    }

    #[test]
    fn own_one_value_keys_leave_blocks_in_the_enclosing_loop() {
        let rows = [
            ("synthetic_target_value", 0x1404540c0, 0x141446BF0),
            ("synthetic_flag_value", 0x1414461e0, 0x1405972d0),
        ];
        own(&rows);
        for class in rows.map(|(name, _, _)| name) {
            let result = read(format!("body = {{ {class} = {{ value = {{ @kept = 7 value = @kept }} }} }} kept = @kept").as_bytes());
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            let class_body = result.items[0].value.items().unwrap()[0]
                .value
                .items()
                .unwrap();
            assert_eq!(class_body[0].value.scalar(), Some(&b"{"[..]), "{class}");
            assert!(!result.misreads.is_empty(), "{class}");
        }
    }

    #[test]
    fn owns_any_state_of_is_a_raw_list_even_with_a_definition() {
        super::register_owns_any_state_of();
        let result = read(b"body = { owns_any_state_of = { 1 @lost = 2 } } lost = @lost");
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
        let entries = result.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().all(|item| item.key.is_none()));
    }

    #[test]
    fn custom_override_tooltip_uses_bound_localisation_and_trigger_dispatch() {
        own(&[("custom_override_tooltip", 0x141446060, 0x141446C20)]);
        let result = read(b"body = { custom_override_tooltip = { tooltip = { arg = { @kept = 4 localization_key = x } } invalid = { @lost = 2 } always = yes } } kept = @kept lost = @lost");
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"4"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"@lost"[..]));
    }

    #[test]
    fn math_errors_skip_the_tail_without_registering_definitions() {
        own(&[("check_expr", 0x14056E240, 0)]);
        let result = read(b"body = { check_expr = { value = { value = 1 @kept = 4 add = @kept } invalid = 0 ignored = { @lost = 2 } @tail = 8 add = 1 } } kept = @kept lost = @lost tail = @tail");
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"4"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"@lost"[..]));
        assert_eq!(result.items[3].value.scalar(), Some(&b"@tail"[..]));
    }

    #[test]
    fn math_fixed_argument_and_conditional_loops_share_the_reader() {
        own(&[("check_expr", 0x14056E240, 0)]);
        let result = read(b"body = { check_expr = { value = 1 clamp = { min = { value = 0 @a = 2 add = @a } max = 10 } if = { limit = 1 @b = 3 add = @b } else_if = { limit = 0 add = 4 } else = { @c = 5 add = @c } add = 6 } } a = @a b = @b c = @c");
        assert!(result.unsettled.is_none());
        assert!(result.misreads.is_empty(), "{:?}", result.misreads);
        let values: Vec<_> = result.items[1..]
            .iter()
            .map(|item| item.value.scalar())
            .collect();
        assert_eq!(values, [Some(&b"2"[..]), Some(&b"3"[..]), Some(&b"5"[..])]);
    }
}
