//! Native slot-6 dispatch for the classes whose jump tables were untraced.

use super::{Node, Shaper};
use crate::parser::native::{Field, Shape};

impl Shaper<'_> {
    /// Value readers of the static keys in KEY_READERS. The remaining
    /// accepted keys call scalar/string/scoped-variable readers only.
    pub(super) fn body_value(&mut self, token: u32, field: &Field<'_>) -> Shape<Node> {
        let key = field.keyword();
        match (token, key.as_deref()) {
            // 1404A7960 -> 1404A5D30 -> 1404A6170. The closing token
            // resolves into the lexer only; each entry sets the value slot.
            (0x4967 | 0x49DA | 0x49DB | 0x49DC | 0x47B9, Some(b"target")) if field.opens() => {
                Shape::ValueList
            }
            // CTargetOfTrigger::vf6@140471740 selects the state array on
            // `{` (14045FDC0): entries set the value slot; the close does not.
            (0x4363, Some(b"target")) if field.opens() => Shape::ValueList,
            // Collections are the same embedded CCollection reader everywhere.
            (0x27FA | 0x474B | 0x474C | 0x40DF, Some(b"collection")) => Self::collection(field),
            // 1405148F0's countries uses 1404A5D30.
            (0x40DF, Some(b"countries")) if field.opens() => Shape::ValueList,
            // ReadParserStringWithCaseInsensitiveHash@140606100 takes the
            // current lexer token, not the reader's retained value slot.
            // Power-balance readers use the same path (140489B20).
            (0x3CBD | 0x3CBE | 0x3CCE | 0x3CCF | 0x3CC0 | 0x3CC1 | 0x3E53, Some(b"id"))
            | (0x3CBD, Some(b"range"))
            | (0x3CCE, Some(b"side")) => Shape::Current,
            // ReadScriptInt32Array@1401F91B0 / ReadTokenIdList@1403B7180.
            (0x39CE, Some(b"regions"))
            | (0x3164, Some(b"resources"))
            | (0x4D7E | 0x4D7F, Some(b"states" | b"types" | b"exclude")) => Shape::RawList,
            // The navy's weight map reads any key with ReadKeyOperatorValue,
            // then one fixed5 token (14043A2D0).
            (0x3C75, Some(b"sub_unit_def_weights")) => Shape::Object(Node::ScalarFields),
            // 140435760's idea / ignore loops are lists only on `{`.
            (0x3366 | 0x4CBB, Some(b"idea" | b"ignore")) if field.opens() => Shape::RawList,
            // 140436180's authored categories and slots are scalar or list.
            (0x37F7, Some(b"categories" | b"slots")) if field.opens() => Shape::CheckedList {
                stores_value: false,
            },
            (0x3BE4, Some(b"building")) => self.building(field, false, 0x140472210),
            // 1404378C0 / 140436AB0 reject an opening type value.
            (0x39E7 | 0x3A2B, Some(b"type")) if field.opens() => Shape::Skip,
            // 140440150 rejects an invalid comparison operator.
            (0x3E81 | 0x3F14, Some(b"compare")) if !comparison(field) => Shape::Skip,
            (0x37DC, Some(b"province")) => Shape::Object(Node::KeyReader(0x140471B20)),
            (0x37DC, Some(b"building")) => self.building(field, true, 0x140471900),
            (0x37DC, Some(b"level"))
                if !matches!(
                    field.operator,
                    crate::parser::Operator::Less | crate::parser::Operator::Greater
                ) =>
            {
                Shape::Skip
            }
            // CHasLicenseTrigger embeds CEquipmentVariantReference at +0xA8.
            (0x37B9, Some(b"equipment")) => Shape::Object(Node::KeyReader(0x141411210)),
            (0x2828 | 0x2938, Some(b"path")) => Shape::RawList,
            (0x2828 | 0x2938, Some(b"controller_priority")) => Shape::Object(Node::Mtth),
            (0x4C9D, Some(b"input")) | (0x31C7, Some(b"collection")) => Self::collection(field),
            (0x31C7, Some(b"buildings")) => Shape::RawList,
            (0x31C7, Some(b"unit_category")) if field.opens() => {
                Shape::CheckedList { stores_value: true }
            }
            (0x31C7, Some(b"unit")) if field.opens() => Shape::Skip,
            _ => Shape::One,
        }
    }

    /// Bespoke loops and embedded readers with no coded-trigger token.
    pub(super) fn native_key(&mut self, address: u64, field: &Field<'_>) -> Shape<Node> {
        let key = field.keyword();
        let key = key.as_deref();
        match address {
            // CVariableTrigger selects an expression for an opening value,
            // except for var/tooltip; unknown names remain scoped variables.
            0x14043B2C0 if !matches!(key, Some(b"tooltip" | b"var")) && field.opens() => {
                Shape::Expression
            }
            0x14043B2C0 | 0x140437640 | 0x140437F90 | 0x14043AE40 | 0x140440460
            | 0x1404723E0 | 0x1404386A0 | 0x1403B04D0 | 0x140438210
            // These own loops return for unknown keys without skipping.
            | 0x1404377F0 | 0x140438100 | 0x14043A8A0 | 0x140480E00
            | 0x140458240 => Shape::One,
            0x140437400 | 0x1404401E0 if key == Some(b"compare") && !comparison(field) => {
                Shape::Skip
            }
            0x140437400 | 0x1404401E0 => Shape::One,
            0x1403B38F0 if key == Some(b"var_list") => Shape::List,
            0x1403B38F0 => Shape::One,
            0x1403B34F0 if key == Some(b"text") => Shape::RawText,
            0x1403B34F0 => Shape::One,
            // CProvinceBuildingFilter and CEquipmentVariantReference are
            // embedded objects, not registered trigger classes.
            0x140471B20
                if matches!(
                    key,
                    Some(
                        b"id"
                            | b"all_provinces"
                            | b"limit_to_coastal"
                            | b"limit_to_border"
                            | b"limit_to_naval_base"
                            | b"limit_to_victory_point"
                    )
                ) =>
            {
                Shape::One
            }
            0x141411210
                if matches!(
                    key,
                    Some(
                        b"type"
                            | b"version"
                            | b"creator"
                            | b"create_if_missing"
                            | b"version_name"
                            | b"license_creator"
                    )
                ) =>
            {
                Shape::One
            }
            0x140471B20 | 0x141411210 => Shape::Skip,
            // CCollection::vf4@140A0FC90: its three authored fields.
            0x140A0FC90 if key == Some(b"name") => Shape::ObjectIfOpen(Node::BoundLocalization),
            0x140A0FC90 if key == Some(b"operators") && field.opens() => {
                Shape::Statements(Node::CollectionOperator)
            }
            0x140A0FC90 if key == Some(b"input") => Shape::One,
            0x140A0FC90 => Shape::Skip,
            _ => Self::refuse_reader(address, field.key.c_str()),
        }
    }

    pub(super) fn collection_operator(&mut self, field: &Field<'_>) -> Shape<Node> {
        // SOperatorReader@142923D00: bare values (140A10410) consume no
        // further token; anonymous blocks (140496090) are RecordError.
        if field.key.text.is_empty() {
            return if field.opens() {
                Shape::Skip
            } else {
                Shape::One
            };
        }
        // Assignment callback 140A10660: only `limit` reads a CAndTrigger
        // (140A0E020). The three bare-only operators use raw SkipBlock after
        // TryHandleOrRecordError; every other key uses RecordError's skip.
        match field.keyword().as_deref() {
            Some(b"limit") => Shape::Body(Node::Trigger),
            Some(b"country_and_all_subjects" | b"controlled_states" | b"faction_members") => {
                Shape::RawSkip
            }
            _ => Shape::Skip,
        }
    }

    fn collection(field: &Field<'_>) -> Shape<Node> {
        // NScript::CCollection::vf3@140A0F440: 140A0FC90 on an opening
        // value, 140A0FFB0 (one token) otherwise. Post-read checks do not skip.
        if field.opens() {
            Shape::Object(Node::KeyReader(0x140A0FC90))
        } else {
            Shape::One
        }
    }

    fn building(&self, field: &Field<'_>, provincial: bool, address: u64) -> Shape<Node> {
        let Some(names) = self.names else {
            return Self::refuse_reader(address, field.key.c_str());
        };
        let name = field.value.lookup().map(|name| name.to_ascii_lowercase());
        match name.and_then(|name| names.province_buildings.get(&name).copied()) {
            Some(Some(value)) if value == provincial => Shape::One,
            Some(None) => Shape::Unsettled(format!(
                "ambiguous provincial building in {} ({address:X})",
                String::from_utf8_lossy(field.key.c_str())
            )),
            _ => Shape::Skip,
        }
    }
}

fn comparison(field: &Field<'_>) -> bool {
    matches!(
        field.value_keyword().as_deref(),
        Some(
            b"greater_than"
                | b"less_than"
                | b"equals"
                | b"not_equals"
                | b"greater_than_or_equals"
                | b"less_than_or_equals"
        )
    )
}

#[cfg(test)]
mod tests {
    use crate::{
        key_names::KeyNames,
        parser::native::{self, Shape},
    };

    fn read(text: &str, names: &KeyNames) -> native::Read {
        native::read(
            text.as_bytes(),
            native::Mode::KeepMark,
            native::Top::File(()),
            Some(names),
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
    fn jump_table_key_readers_skip_unknown_blocks() {
        for class in [
            "has_railway_level",
            "free_building_slots",
            "has_built",
            "has_volunteers_amount_from",
            "has_resources_amount",
            "army_manpower_in_state",
            "divisions_in_border_state",
            "has_mines",
            "has_operation_token",
            "war_length_with",
            "compare_ideology_with_faction",
            "fighting_army_strength_ratio",
            "network_strength",
            "career_profile_check_ratio",
            "career_profile_check_playthrough_ratio",
            "count_in_collection",
        ] {
            let result = read(
                &format!(
                    "body = {{ {class} = {{ invalid = {{ @lost = 2 }} @kept = 3 marker = @kept }} }} lost = @lost kept = @kept"
                ),
                &KeyNames::default(),
            );
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            assert!(result.items.len() >= 3, "{class}: {result:?}");
            assert_eq!(
                result.items[1].value.scalar(),
                Some(&b"@lost"[..]),
                "{class}"
            );
            assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]), "{class}");
        }
    }

    #[test]
    fn scoped_variable_keys_consume_one_value_instead_of_a_body() {
        for class in [
            "check_variable",
            "clamp_temp_variable",
            "has_border_war_between",
            "resize_temp_array",
            "career_profile_set_temp_variable",
            "has_game_rule",
            "is_in_array",
        ] {
            let result = read(
                &format!(
                    "body = {{ {class} = {{ unknown = {{ @kept = 6 value = @kept }} }} }} kept = @kept"
                ),
                &KeyNames::default(),
            );
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            let entries = result.items[0].value.items().unwrap()[0]
                .value
                .items()
                .unwrap();
            assert_eq!(entries[0].value.scalar(), Some(&b"{"[..]), "{class}");
            assert!(!result.misreads.is_empty(), "{class}");
        }
    }

    #[test]
    fn province_filter_and_building_lookup_use_their_own_readers() {
        let names = KeyNames {
            province_buildings: [(b"native_building".to_vec(), Some(true))].into(),
            ..KeyNames::default()
        };
        let result = read(
            "body = { any_province_building_level = { province = { id = 1 invalid = { @lost = 2 } @kept = 4 id = @kept } building = native_building level > 0 } } lost = @lost kept = @kept",
            &names,
        );
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"4"[..]));
    }

    #[test]
    fn variable_blocks_use_the_expression_reader() {
        for class in [
            "set_temp_variable",
            "add_to_temp_variable",
            "subtract_from_temp_variable",
            "multiply_temp_variable",
            "divide_temp_variable",
        ] {
            let result = read(
                &format!(
                    "body = {{ {class} = {{ x = {{ value = 1 @kept = 3 invalid = 0 @lost = 4 add = 1 }} }} }} kept = @kept lost = @lost"
                ),
                &KeyNames::default(),
            );
            assert!(
                result.unsettled.is_none(),
                "{class}: {:?}",
                result.unsettled
            );
            assert_eq!(result.items[1].value.scalar(), Some(&b"3"[..]), "{class}");
            assert_eq!(
                result.items[2].value.scalar(),
                Some(&b"@lost"[..]),
                "{class}"
            );
        }
    }

    #[test]
    fn collection_statements_define_only_inside_the_limit_trigger() {
        let result = read(
            "body = { collection_size = { input = { input = scope:root operators = { country_and_all_subjects @lost = 2 limit = { invalid = { @hidden = 3 } @kept = 4 always = yes } } } value > 0 } } lost = @lost hidden = @hidden kept = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none(), "{:?}", result.unsettled);
        assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"@hidden"[..]));
        assert_eq!(result.items[3].value.scalar(), Some(&b"4"[..]));
    }

    #[test]
    fn railway_and_country_lists_do_not_register_definitions() {
        for (class, key) in [
            ("can_build_railway", "path"),
            ("has_railway_connection", "path"),
            ("any_country_of", "target"),
            ("any_country_with_original_tag_of", "target"),
        ] {
            let result = read(
                &format!("body = {{ {class} = {{ {key} = {{ 1 @lost = 2 }} }} }} lost = @lost"),
                &KeyNames::default(),
            );
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
        }
    }

    #[test]
    fn license_equipment_reference_skips_invalid_child_blocks() {
        let result = read(
            "body = { has_license = { equipment = { type = native_type invalid = { @lost = 2 } @kept = 3 version_name = @kept } } } lost = @lost kept = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"@lost"[..]));
        assert_eq!(result.items[2].value.scalar(), Some(&b"3"[..]));
    }

    #[test]
    fn validated_lists_skip_invalid_opening_entries_without_defining() {
        for (class, key) in [
            ("amount_taken_ideas", "categories"),
            ("amount_taken_ideas", "slots"),
            ("count_in_collection", "unit_category"),
        ] {
            let result = read(
                &format!(
                    "body = {{ {class} = {{ {key} = {{ {{ @lost = 2 }} token }} @kept = 3 marker = @kept }} }} lost = @lost kept = @kept"
                ),
                &KeyNames::default(),
            );
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
    fn unreached_key_readers_refuse_by_parent_class_name() {
        for class in [
            "career_profile_check_points",
            "career_profile_check_medal",
            "is_licensing_to",
            "faction_goal_fulfillment",
        ] {
            let result = read(
                &format!("body = {{ {class} = {{ value = 1 }} }}"),
                &KeyNames::default(),
            );
            assert!(result.unsettled.unwrap().what.contains(class), "{class}");
        }
    }

    #[test]
    fn invalid_meta_parameters_use_record_errors_skip_after_the_raw_read() {
        let result = read(
            // RecordError consumes one closing delimiter; the native loops
            // then need their own delimiters before the file resumes.
            "body = { meta_trigger = { text = { ] } @lost = 2 debug = yes } } } @kept = 3 marker = @kept lost = @lost kept = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none());
        assert_eq!(
            result
                .items
                .iter()
                .find(|item| item.keyword_is("lost"))
                .unwrap()
                .value
                .scalar(),
            Some(&b"@lost"[..])
        );
        assert_eq!(
            result
                .items
                .iter()
                .find(|item| item.keyword_is("kept"))
                .unwrap()
                .value
                .scalar(),
            Some(&b"3"[..])
        );
        assert!(
            result
                .misreads
                .iter()
                .any(|note| note.what.contains("raw meta text"))
        );
    }

    #[test]
    fn embedded_nul_in_meta_text_is_refused_by_name() {
        let text = String::from_utf8(
            b"body = { meta_trigger = { text = { "
                .iter()
                .copied()
                .chain([0])
                .chain(b" } } }".iter().copied())
                .collect(),
        )
        .unwrap();
        let result = read(&text, &KeyNames::default());
        assert!(
            result
                .unsettled
                .unwrap()
                .what
                .contains("raw meta text (141408C80): embedded NUL")
        );
    }

    #[test]
    fn meta_text_counts_braces_in_comments_as_raw_characters() {
        let result = read(
            "body = { meta_trigger = { text = { # }\n @kept = 7 debug = yes } } kept = @kept",
            &KeyNames::default(),
        );
        assert!(result.unsettled.is_none());
        assert_eq!(result.items[1].value.scalar(), Some(&b"7"[..]));
    }

    #[test]
    fn any_state_of_reads_target_arrays_without_defining_their_tokens() {
        for operator in ["=", ""] {
            let text = format!(
                "@first = 11 body = {{ any_state_of = {{ target {operator} {{ @first 2 @lost = 4 }} @kept = 7 always = yes }} }} kept = @kept lost = @lost"
            );
            let read = native::read(
                text.as_bytes(),
                native::Mode::KeepMark,
                native::Top::File(()),
                Some(&KeyNames::default()),
                &mut |(), field| {
                    if field.keyword_is("body") {
                        Shape::Trigger
                    } else {
                        Shape::One
                    }
                },
            );
            assert!(read.unsettled.is_none(), "{read:?}");
            assert_eq!(read.items[1].value.scalar(), Some(b"7".as_slice()));
            assert_eq!(read.items[2].value.scalar(), Some(b"@lost".as_slice()));
            let state = read.items[0].value.items().unwrap()[0]
                .value
                .items()
                .unwrap();
            assert_eq!(state.len(), 2, "{state:?}");
            assert!(state[1].keyword_is("always"));
            let target = state[0].value.items().unwrap();
            assert_eq!(target.len(), 5);
            assert!(target.iter().all(|item| item.key.is_none()));
            assert_eq!(target[0].value.scalar(), Some(b"11".as_slice()));
        }
        let read = crate::triggers::read(
            b"any_state_of = { target = { 1 2 } always extra = 8 }",
            &KeyNames::default(),
        );
        assert!(read.unsettled.is_none(), "{read:?}");
        let fields = read.items[0].value.items().unwrap();
        assert!(fields[1].keyword_is("always"));
        assert_eq!(fields[1].value.scalar(), Some(b"2".as_slice()));
    }
}
