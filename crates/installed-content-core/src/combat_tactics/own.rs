//! Specialized value reads beyond assignment, comparison, and direct bodies.
//! Body paths use the class's one KEY_READERS entry, including inline loops.

use super::{Node, Shaper};
use crate::parser::native::{Field, Shape};

impl Shaper<'_> {
    pub(super) fn own_value(&self, token: u32, address: u64, field: &Field<'_>) -> Shape<Node> {
        match address {
            // Body and variable-trigger readers use the key loop or embedded
            // helper selected by KEY_READERS.
            0x140454050 | 0x141445C30 | 0x14049F1E0 | 0x14049F630 | 0x14049FA80 | 0x14049FED0
            | 0x1404A0320 => Shape::Body(Node::Class(token)),
            // Scalar first unless the value opens the class's key loop.
            0x1404540D0 | 0x140454220 | 0x140454800 | 0x141445DB0 | 0x14046AC80 | 0x14046AD80 => {
                Shape::ObjectIfOpen(Node::Class(token))
            }
            // Integer arrays read resolved tokens without registering definitions.
            0x140490CF0 => Shape::RawList,
            // Read the numeric expression.
            0x14056E1D0 => Shape::Expression,
            0x140454120 | 0x140454540 | 0x1404547F0 | 0x1404C17A0 => Shape::One,
            0x140490C90 => Shape::Current,
            // Opening blocks report errors and skip raw tokens; other values read once.
            0x140454060 if field.opens() => Shape::SkipIfOpen,
            0x140454060 => Shape::One,
            // `<` and `>` select a comparison; every other operator selects a
            // body read, including `=` with an opening value.
            0x140477C30
                if matches!(
                    field.operator,
                    crate::parser::Operator::Less | crate::parser::Operator::Greater
                ) =>
            {
                Shape::One
            }
            0x140477C30 => Shape::Body(Node::Class(token)),
            // A trait leaf reads a scalar token or the class's block-key loop.
            0x140477C50 if field.opens() => Shape::Body(Node::Class(token)),
            0x140477C50 => Shape::One,
            // The weather reader never reaches its block form; a scalar resolves one weather.
            0x140497CB0 if !field.opens() => Shape::One,
            _ => Self::refuse_reader(address, field.key.c_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        key_names::KeyNames,
        parser::native::{self, Shape},
    };

    /// The state-building reader rejects missing or provincial buildings;
    /// the province-building reader requires a provincial building.
    #[test]
    fn non_damaged_building_level_rejects_provincial_buildings() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            // Scalar error recovery reads nothing. A stale invalid building
            // with `{` consumed as its operator must skip that block.
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

    /// Assignment reads a body through its key reader.
    #[test]
    fn completed_mio_trait_count_reads_its_assignment_body() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let result = crate::triggers::read(
            b"has_mio_number_of_completed_traits = { @kept = 2 value > 3 } always = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none(), "{result:?}");
        assert!(result.misreads.is_empty(), "{result:?}");
        assert_eq!(result.items.len(), 2, "{result:?}");
        let value = &result.items[0].value.items().unwrap()[0];
        assert!(value.keyword_is("value"));
        assert_eq!(value.operator, crate::parser::Operator::Greater);
        assert_eq!(result.items[1].value.scalar(), Some(b"2".as_slice()));
    }

    /// The game skips unknown trait-leaf block keys unread.
    #[test]
    fn mio_trait_leaves_read_a_scalar_or_their_block_keys() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // One fixture for every reached skip-dispatch family; a generic
        // by-key reader defines @lost inside the unknown field instead.
        for (class, key) in [
            ("has_dynamic_modifier", "modifier"),
            ("has_naval_invasion_against_state", "state"),
            ("has_wargoal_against", "target"),
            ("has_war_with_wargoal_against", "type"),
            ("has_country_flag", "flag"),
            ("has_state_flag", "flag"),
            ("has_global_flag", "flag"),
            ("has_unit_leader_flag", "flag"),
            ("has_character_flag", "flag"),
            ("decryption_progress", "target"),
            ("has_collaboration", "target"),
            ("network_national_coverage", "target"),
            ("is_running_operation", "operation"),
            ("is_preparing_operation", "operation"),
            ("advisor_can_be_fired", "character"),
            ("is_leading_army_in_province", "province"),
            ("get_highest_scored_country_temp", "var"),
            ("get_sorted_scored_countries_temp", "array"),
            ("get_supply_vehicles_temp", "var"),
            ("set_temp_variable_to_random", "min"),
            ("power_balance_value", "value"),
            ("power_balance_daily_change", "value"),
            ("power_balance_weekly_change", "value"),
            ("non_damaged_building_level", "level"),
        ] {
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        for class in [
            "has_country_flag",
            "core_compliance",
            "core_resistance",
            "has_core_occupation_modifier",
            "num_finished_operations",
            "modulo_temp_variable",
        ] {
            let result = read(format!("body = {{ {class} = {{ value = {{ @kept = 7 value = @kept }} }} }} kept = @kept").as_bytes());
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            // The brace given to `value` consumes no body. Its closing brace
            // ends the class, and the last probe remains readable at top level.
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let result = read(b"body = { custom_override_tooltip = { tooltip = { arg = { @kept = 4 localization_key = x } } invalid = { @lost = 2 } always = yes } } kept = @kept lost = @lost");
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"4"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"@lost"[..]));
    }

    #[test]
    fn math_errors_skip_the_tail_without_registering_definitions() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let result = read(b"body = { check_expr = { value = { value = 1 @kept = 4 add = @kept } invalid = 0 ignored = { @lost = 2 } @tail = 8 add = 1 } } kept = @kept lost = @lost tail = @tail");
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"4"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"@lost"[..]));
        assert_eq!(result.items[3].value.scalar(), Some(&b"@tail"[..]));
    }

    #[test]
    fn math_fixed_argument_and_conditional_loops_share_the_reader() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
