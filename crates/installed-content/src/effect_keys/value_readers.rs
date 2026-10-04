//! Slot-3 readers outside the default ReadScript and one-value paths.
//! Addresses name executable grammar, never content from a vanilla roster.

use crate::{
    key_names::KeyNames,
    parser::{
        Item, Operator, Value,
        native::{Field, Kind, Reader, Shape, Token},
    },
    scope::ScopeKind,
};

use super::{
    IF_TOKEN,
    key_readers::{ELSE_IF_TOKEN, ELSE_TOKEN},
};

#[derive(Clone, Copy)]
enum LoopSource {
    Inline,
    ClassKey,
}

impl LoopSource {
    fn address(self, value: u64, key: u64) -> u64 {
        match self {
            Self::Inline => value,
            Self::ClassKey => key,
        }
    }
}

#[derive(Clone, Copy)]
enum Form {
    One,
    SkipIfOpen,
    Current,
    CurrentOrRaw,
    CurrentOrKeys(LoopSource),
    ScalarOrKeys(LoopSource),
    StringOrKeys(LoopSource),
    Keys(LoopSource),
    AlwaysKeys(LoopSource),
    Persistent(u64),
    RoleKeys(LoopSource),
    List,
    Ideas,
    Localization,
}

enum ScalarRead {
    One,
    Block,
    Deferred,
}

// The embedded CBoundLocalization has its own Read, independent of the
// containing effect's slot-4 reader (141384F50 -> 142390E40).
const BOUND_LOCALIZATION_READ: u64 = 0x1423912A0;
const DIVISION_TEMPLATE_KEYS: u64 = 0x140B937A0;
const EQUIPMENT_VARIANT_KEYS: u64 = 0x141411210;

// A loop uses either the value reader itself or the class's slot-4 reader.
const VALUE_READERS: &[(u64, Form)] = &[
    // Slot-3 readers below use the already-held scalar token. Only the
    // tooltip/mission readers enter a loop when that token is `{`.
    (0x1402F0540, Form::SkipIfOpen), // remove unit leader: RecordError on `{`
    (0x1403AB040, Form::One),        // set land/naval/air OOB: value text, no advance
    (0x140495DC0, Form::One),        // add scientist trait: registry lookup of held token
    (0x1403A91F0, Form::One),        // activate/deactivate advisor: value text
    (0x1403A92B0, Form::CurrentOrKeys(LoopSource::Inline)), // mission tooltip
    (0x1403A9C20, Form::Current),    // activate/remove decision
    (0x1403A9EC0, Form::CurrentOrKeys(LoopSource::Inline)), // activate/remove mission
    (0x1403AAA50, Form::One),        // recruit character: retained token
    (0x1403AAFA0, Form::One),        // retire character: retained token
    (0x1403AB900, Form::CurrentOrRaw), // show ideas tooltip
    (0x1403ABD40, Form::Current),    // show unit leaders tooltip
    (0x1403ACC80, Form::Current),    // unlock decision category tooltip
    (0x140496070, Form::One),        // injure scientist: integer from retained token
    (0x1403A89E0, Form::ScalarOrKeys(LoopSource::ClassKey)), // randomize
    (0x1403A9810, Form::Ideas),      // raw resolved tokens, not ReadKeyOperatorValue
    (0x1403A9B40, Form::Keys(LoopSource::ClassKey)), // ReadScript thunk
    (0x1403A9C70, Form::ScalarOrKeys(LoopSource::Inline)), // puppet
    (0x1403AB080, Form::ScalarOrKeys(LoopSource::ClassKey)), // focus tree
    (0x1403AB2C0, Form::ScalarOrKeys(LoopSource::Inline)), // character name
    (0x1403AB590, Form::ScalarOrKeys(LoopSource::Inline)), // nationality
    (0x1403AC0D0, Form::Keys(LoopSource::Inline)), // swap ideas
    (0x1403ACCD0, Form::ScalarOrKeys(LoopSource::Inline)), // decision tooltip
    (0x1403ACF40, Form::ScalarOrKeys(LoopSource::Inline)), // white peace
    (0x14047CCE0, Form::Keys(LoopSource::ClassKey)), // highest scored temp
    (0x14047D130, Form::Keys(LoopSource::ClassKey)), // highest scored
    (0x14047D580, Form::Keys(LoopSource::ClassKey)), // sorted scored temp
    (0x14047D9D0, Form::Keys(LoopSource::ClassKey)), // sorted scored
    (0x14047DE20, Form::Keys(LoopSource::ClassKey)), // supply vehicles temp
    (0x14047E6C0, Form::Keys(LoopSource::ClassKey)), // modulo temp
    (0x14047EB10, Form::Keys(LoopSource::ClassKey)), // modulo
    (0x14047EF60, Form::StringOrKeys(LoopSource::ClassKey)), // random temp
    (0x14047F3E0, Form::StringOrKeys(LoopSource::ClassKey)), // random variable
    (0x141384F50, Form::Localization), // embedded CBoundLocalization
    (0x141384F60, Form::ScalarOrKeys(LoopSource::ClassKey)), // event
    (0x141384F80, Form::ScalarOrKeys(LoopSource::ClassKey)), // flag
    (0x141385260, Form::Keys(LoopSource::ClassKey)), // modify flag
    (0x141385770, Form::ScalarOrKeys(LoopSource::ClassKey)), // assignment
    (0x1403A97C0, Form::Persistent(DIVISION_TEMPLATE_KEYS)),
    (0x1403AA3C0, Form::ScalarOrKeys(LoopSource::Inline)),
    (0x1403AAC00, Form::RoleKeys(LoopSource::Inline)),
    // Round 2 value forms.
    (0x1402F0520, Form::List), // add random trait -> ReadStringList 1401426E0
    (0x1402F0810, Form::Keys(LoopSource::Inline)), // swap country leader traits
    (0x14031B0A0, Form::ScalarOrKeys(LoopSource::ClassKey)), // targeted resistance
    (0x1403A9510, Form::ScalarOrKeys(LoopSource::Inline)), // country leader trait
    (0x1403A9B50, Form::ScalarOrKeys(LoopSource::ClassKey)), // complete focus
    (0x1403A9BA0, Form::ScalarOrKeys(LoopSource::ClassKey)), // create agency
    (0x1403A9BB0, Form::Keys(LoopSource::ClassKey)), // create operative leader
    (0x1403AA100, Form::ScalarOrKeys(LoopSource::ClassKey)), // operative operands
    (0x1403ABDF0, Form::ScalarOrKeys(LoopSource::Inline)), // peace conference
    (0x1403AC650, Form::Keys(LoopSource::Inline)), // swap ruler traits
    (0x14051BBB0, Form::AlwaysKeys(LoopSource::Inline)), // strategic location
];

pub(super) fn shape(value: u64, key: u64, field: &Field<'_>) -> Option<Shape<()>> {
    let form = VALUE_READERS
        .iter()
        .find(|(address, _)| *address == value)?
        .1;
    let (source, scalar) = match form {
        Form::One => return Some(Shape::One),
        Form::SkipIfOpen => return Some(Shape::SkipIfOpen),
        Form::Current => return Some(Shape::Current),
        Form::CurrentOrRaw => {
            return Some(if field.opens() {
                Shape::RawList
            } else {
                Shape::Current
            });
        }
        Form::CurrentOrKeys(source) => {
            if !field.opens() {
                return Some(Shape::Current);
            }
            (source, ScalarRead::One)
        }
        Form::Persistent(reader) => {
            own_reader(reader, field.key)?;
            return Some(Shape::EffectValueKeys(reader));
        }
        Form::AlwaysKeys(source) => {
            let reader = source.address(value, key);
            own_reader(reader, field.key)?;
            // 14051BBB0 begins its loop even when the value is scalar or stale.
            return Some(Shape::EffectValueKeys(reader));
        }
        Form::List => {
            return Some(if field.opens() {
                Shape::List
            } else {
                Shape::One
            });
        }
        Form::Ideas => {
            // The scalar takes the current lexer token; the raw array
            // resolves tokens without writing the retained value slot.
            return Some(if field.opens() {
                Shape::RawList
            } else {
                Shape::Current
            });
        }
        Form::Localization => {
            return Some(if field.opens() {
                Shape::EffectValueKeys(BOUND_LOCALIZATION_READ)
            } else {
                Shape::One
            });
        }
        Form::RoleKeys(source) => match field.effect_scope {
            Some(scope) if scope == ScopeKind::Character.bit() => {
                return Some(if field.value.c_str() == b"yes" {
                    Shape::One
                } else {
                    Shape::Reported {
                        kind: crate::DiagnosticKind::Misread,
                        report: "remove_unit_leader_role in character scope requires exact yes; native ReportUnexpectedToken (1403AAC00 -> 1424AD100)".into(),
                        read: Box::new(Shape::Skip),
                    }
                });
            }
            None if field.opens() || (field.value.c_str() != b"yes" && field.skips_block()) => {
                return None;
            }
            _ => (source, ScalarRead::One),
        },
        Form::ScalarOrKeys(source) => (source, ScalarRead::One),
        Form::StringOrKeys(source) => (source, string_scalar(field)),
        Form::Keys(source) => (source, ScalarRead::Block),
    };
    let reader = source.address(value, key);
    // Shared slots retain named, counted refusals for class grammars
    // absent from the own-key table, on both scalar and block forms.
    own_reader(reader, field.key)?;
    if field.opens() {
        return Some(Shape::EffectValueKeys(reader));
    }
    match scalar {
        ScalarRead::One => Some(Shape::One),
        ScalarRead::Deferred => None,
        ScalarRead::Block => Some(Shape::Reported {
            kind: crate::DiagnosticKind::Misread,
            report: format!(
                "effect value reader {value:#X} requires a block; native RecordError (1424ACF90)"
            ),
            read: Box::new(Shape::Skip),
        }),
    }
}

fn string_scalar(field: &Field<'_>) -> ScalarRead {
    use crate::parser::native::Kind;
    use std::{collections::BTreeSet, sync::LazyLock};
    static KEYWORDS: LazyLock<BTreeSet<Vec<u8>>> = LazyLock::new(|| {
        include_str!("../gui/native/keywords.tsv")
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(_, key)| key.to_ascii_lowercase().into_bytes())
            .collect()
    });
    // Token 0xF selects scalar assignment. Bare names may already be
    // dynamically interned by the recorded content stack; the static
    // catalogue cannot settle that distinction or a stale scalar route.
    if !field.fresh {
        ScalarRead::Deferred
    } else if field.value.kind == Kind::Quoted {
        ScalarRead::One
    } else if field.value.kind == Kind::Bare
        && !field.value.is_number()
        && !KEYWORDS.contains(&field.value.c_str().to_ascii_lowercase())
    {
        ScalarRead::Deferred
    } else {
        ScalarRead::Block
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Root,
    Keys(u64),
    IterationOutput,
    EquipmentRestrictions,
    One,
    Units(UnitGroup, Placement),
}

#[derive(Clone, Copy)]
enum Unknown {
    Skip,
    Effect,
}

#[derive(Clone, Copy)]
enum TokenSource {
    Current,
    Value,
}

#[derive(Clone, Copy)]
enum OwnValue {
    One,
    Object(Node),
    ObjectIfOpen(Node),
    CurrentOrRaw,
    InvalidBlock,
    RawSkipIfOpen,
    ValueListRequired,
    RawListRequired,
    RawList,
    Trigger,
    BoundLocalization,
    Statements(Node),
    Keywords {
        source: TokenSource,
        accepted: &'static [&'static str],
        report: &'static str,
    },
    Deferred(u64),
}

#[derive(Clone, Copy)]
enum OwnReader {
    Keys(&'static [&'static str]),
    EffectFallback(&'static [&'static str]),
    One,
    Color,
    Localization,
    Randomize,
    Event(&'static [&'static str]),
    Fields(&'static [(&'static str, OwnValue)], Unknown),
    // A shared slot's accepted operand is selected by the class constructor.
    ClassKeys(&'static [(&'static str, &'static [&'static str])]),
    Wargoal { expire: bool },
    Promotion,
    RemoveRole,
}

// Ordinary accepted fields take one string/bool/scoped-variable token;
// unknowns use ReportUnexpectedToken/RecordError, including its skip.
const OWN_KEYS: &[(u64, OwnReader)] = &[
    // 1402F8640/680 ReadObjectFromParser 1424ABB40 on these tokens;
    // 1403B49C0 reads character as a scoped variable. Other keys report.
    (0x1402F8640, OwnReader::Keys(&["province", "country"])),
    (0x1402F8680, OwnReader::Keys(&["target", "province"])),
    (0x1403B49C0, OwnReader::Keys(&["character"])),
    // 1403A9EC0 accepts mission (0x2CBA); 1403A92B0 also accepts
    // desc (0x2994). Each accepted entry calls ReadStringInto 1424ABB50;
    // an unknown entry calls ReportUnexpectedToken 1424AD100.
    (0x1403A9EC0, OwnReader::Keys(&["mission"])),
    (0x1403A92B0, OwnReader::Keys(&["mission", "desc"])),
    (0x1403AA3C0, OwnReader::Promotion),
    (0x1403AAC00, OwnReader::RemoveRole),
    (
        DIVISION_TEMPLATE_KEYS,
        OwnReader::Fields(
            &[
                ("name", OwnValue::One),
                ("localization_key", OwnValue::One),
                ("parent", OwnValue::ObjectIfOpen(Node::One)),
                ("priority", OwnValue::One),
                ("inherit", OwnValue::RawSkipIfOpen),
                ("country", OwnValue::One),
                ("obsolete_change_date", OwnValue::One),
                ("colonial", OwnValue::One),
                (
                    "support",
                    OwnValue::Object(Node::Units(UnitGroup::Support, Placement::Point)),
                ),
                (
                    "regiments",
                    OwnValue::Object(Node::Units(UnitGroup::Regiments, Placement::Point)),
                ),
                ("force_allow_recruiting", OwnValue::One),
                ("obsolete", OwnValue::One),
                ("template_counter", OwnValue::One),
                ("ingame_set_template_counter", OwnValue::One),
                ("original_tag", OwnValue::One),
                ("allow_new_equipment", OwnValue::One),
                (
                    "not_allowed_equipment",
                    OwnValue::Object(Node::EquipmentRestrictions),
                ),
                ("role", OwnValue::One),
                ("division_names_group", OwnValue::One),
                ("is_locked", OwnValue::One),
                ("foreign_template_tag", OwnValue::One),
                ("origin_type", OwnValue::One),
                ("override_model", OwnValue::One),
                (
                    "regimental_support",
                    OwnValue::Object(Node::Units(UnitGroup::RegimentalSupport, Placement::Point)),
                ),
                (
                    "override_model_equipment_category_filter",
                    OwnValue::CurrentOrRaw,
                ),
                ("is_army_hq", OwnValue::One),
                ("is_fake_intel_division", OwnValue::One),
                ("equipment_niche", OwnValue::One),
                ("allow_foreign_equipment", OwnValue::One),
                ("forbidden_equipment_types", OwnValue::Object(Node::One)),
                ("division_limit", OwnValue::One),
            ],
            Unknown::Skip,
        ),
    ),
    (
        0x1403AF6B0,
        OwnReader::Fields(
            &[
                ("template_name", OwnValue::One),
                (
                    "regiments",
                    OwnValue::Object(Node::Units(UnitGroup::Regiments, Placement::Column)),
                ),
                (
                    "support",
                    OwnValue::Object(Node::Units(UnitGroup::Support, Placement::Column)),
                ),
                (
                    "regimental_support",
                    OwnValue::Object(Node::Units(UnitGroup::RegimentalSupport, Placement::Column)),
                ),
            ],
            Unknown::Skip,
        ),
    ),
    (
        0x1403B1090,
        OwnReader::Fields(
            &[
                ("subject", OwnValue::One),
                (
                    "division_template",
                    OwnValue::Object(Node::Keys(DIVISION_TEMPLATE_KEYS)),
                ),
            ],
            Unknown::Skip,
        ),
    ),
    (
        EQUIPMENT_VARIANT_KEYS,
        OwnReader::Keys(&[
            "type",
            "version",
            "creator",
            "create_if_missing",
            "version_name",
            "license_creator",
        ]),
    ),
    // Thunks -> 141398390 / 141398350.
    (0x140301200, OwnReader::Keys(&["trait", "show_modifiers"])),
    (0x140301210, OwnReader::Keys(&["policy", "show_modifiers"])),
    (
        0x1403A9C70,
        OwnReader::Keys(&["target", "end_civil_wars", "end_wars"]),
    ),
    (0x1403AB2C0, OwnReader::Keys(&["name", "character"])),
    (
        0x1403AB590,
        OwnReader::Keys(&["target_country", "character"]),
    ),
    (
        0x1403AC0D0,
        OwnReader::Keys(&["days", "remove_idea", "add_idea", "add_days"]),
    ),
    (
        0x1403ACCD0,
        OwnReader::Keys(&["decision", "show_effect_tooltip", "show_modifiers"]),
    ),
    (0x1403ACF40, OwnReader::Keys(&["tooltip", "message", "tag"])),
    // Case-sensitive distribution with mutable allowed fields. Factories
    // 14033A5D0 / 14033A660 choose uniform (+0x130 = 0).
    (0x1403ADE70, OwnReader::Randomize),
    // Remaining keys tail-call CEffect::ReadEffect (1405345D0).
    (
        0x1403AF020,
        OwnReader::EffectFallback(&["ideology", "popularity"]),
    ),
    // Unknown keys return without reporting or skipping.
    (0x1403B34B0, OwnReader::One),
    (
        0x1403B4480,
        OwnReader::Keys(&[
            "target",
            "autonomous_state",
            "autonomy_state",
            "freedom_level",
            "release_non_owned_controlled",
            "force_change_controller_for_non_ally_controlled",
        ]),
    ),
    (
        0x1403B4CC0,
        OwnReader::Keys(&[
            "target",
            "autonomous_state",
            "autonomy_state",
            "freedom_level",
            "end_civil_wars",
            "end_wars",
        ]),
    ),
    (
        0x1403B4F80,
        OwnReader::Keys(&["tree", "keep_completed", "copy_completed_from"]),
    ),
    (0x140480F40, OwnReader::Keys(&["var", "scorer"])),
    (0x140480F80, OwnReader::Keys(&["array", "scores", "scorer"])),
    (0x140480FD0, OwnReader::Keys(&["var", "type", "need"])),
    // -> 140480E00: every field is an operand pair, including shorthand.
    (0x140481020, OwnReader::One),
    (
        0x140481030,
        OwnReader::Keys(&["min", "max", "var", "integer"]),
    ),
    (0x14049E7D0, OwnReader::Keys(&["target", "value"])),
    // Scalar fields or CColor; unknown keys return without an error skip.
    (0x1404B1340, OwnReader::Color),
    // random blocks read a CRandomEffect through the enclosing names.
    (
        0x141385790,
        OwnReader::Event(&[
            "id",
            "tooltip",
            "trigger_for",
            "recipient",
            "months",
            "days",
            "hours",
            "random_hours",
            "random_days",
            "random",
            "set_from",
            "set_root",
            "set_from_from",
            "originator",
        ]),
    ),
    (
        0x1413859E0,
        OwnReader::Event(&[
            "id",
            "tooltip",
            "trigger_for",
            "months",
            "days",
            "hours",
            "random_hours",
            "random_days",
            "random",
            "set_from_from",
            "originator",
        ]),
    ),
    (0x141385A10, OwnReader::Keys(&["flag", "value", "days"])),
    (0x141385E80, OwnReader::Keys(&["flag", "value", "days"])),
    (BOUND_LOCALIZATION_READ, OwnReader::Localization),
    // Ordinary character loops; 140539440 and reference failures take one token.
    (0x1402F0810, OwnReader::Keys(&["ideology", "remove", "add"])),
    (0x1403A9510, OwnReader::Keys(&["ideology", "trait"])),
    (0x1403AC650, OwnReader::Keys(&["remove", "add"])),
    (
        0x1403B0B20,
        OwnReader::Keys(&["captured_by", "operative", "ignore_death_chance"]),
    ),
    (
        0x1403B3880,
        // Constructors 14033E4B0 / 140342130 supply killed_by / turned_by.
        // free_operative shares this slot but has zero reach and is absent.
        OwnReader::ClassKeys(&[
            ("kill_operative", &["killed_by", "operative"]),
            ("turn_operative", &["turned_by", "operative"]),
        ]),
    ),
    (
        0x1403B14C0,
        // Local effect fields, then COperativeLeader::load 140C11620 and
        // CUnitLeader::ReadToken 140C0C950. No runtime execution is inferred.
        OwnReader::Fields(
            &[
                ("gfx", OwnValue::One),
                ("bypass_recruitment", OwnValue::One),
                ("available_to_spy_master", OwnValue::One),
                ("portrait_tag_override", OwnValue::One),
                ("id", OwnValue::One),
                ("nationalities", OwnValue::RawListRequired),
                // Inherited traits is unconditional and leaves the value slot intact.
                ("traits", OwnValue::RawList),
                (
                    "gender",
                    // 1413DAFA0 tests the CURRENT token, including stale fields.
                    OwnValue::Keywords {
                        source: TokenSource::Current,
                        accepted: &["female", "male", "undefined"],
                        report: "invalid gender; native RecordError (1413DAFA0 -> 1424ACF90)",
                    },
                ),
                ("name", OwnValue::One),
                ("female", OwnValue::One),
                ("skill", OwnValue::One),
                ("picture", OwnValue::One),
                ("portrait_path", OwnValue::One),
                ("rank", OwnValue::One),
                ("desc", OwnValue::One),
                ("experience", OwnValue::One),
                ("max_traits", OwnValue::One),
                ("custom_cost_text", OwnValue::One),
                ("link", OwnValue::One),
                ("government_in_exile_tag", OwnValue::One),
                ("promoted_from_unit", OwnValue::One),
                ("cooldown_reason", OwnValue::One),
                ("script_id", OwnValue::One),
                ("legacy_id", OwnValue::One),
                ("leader_modifier_enable_date", OwnValue::One),
                ("leader_cooldown_start_date", OwnValue::One),
                ("capture_date", OwnValue::One),
                ("captured", OwnValue::One),
                (
                    "state",
                    // 140C11620 tests retained token IDs 336C/394C/4A50..4A53.
                    OwnValue::Keywords {
                        source: TokenSource::Value,
                        accepted: &[
                            "killed",
                            "on_mission",
                            "on_operation",
                            "on_capture",
                            "on_cooldown",
                            "on_disband",
                        ],
                        report: "invalid operative state; native RecordError (140C11620 -> 1424ACF90)",
                    },
                ),
                ("flags", OwnValue::RawSkipIfOpen),
                // These inherited nested grammars have zero immediate corpus reach.
                // Retain a named, counted refusal rather than inventing their readers.
                ("traits_to_remove", OwnValue::Deferred(0x140C0C950)),
                ("in_progress", OwnValue::Deferred(0x140C0C950)),
                ("trait_xp_factor", OwnValue::Deferred(0x140C0C950)),
                ("operation", OwnValue::ObjectIfOpen(Node::One)),
                ("codename", OwnValue::Deferred(0x1409BB290)),
                ("mission", OwnValue::Deferred(0x140FAF520)),
                ("sub_unit_modifiers", OwnValue::Deferred(0x1410079E0)),
                ("dynamic_modifier", OwnValue::Deferred(0x140603920)),
            ],
            Unknown::Skip,
        ),
    ),
    // Wargoal constructors zero generator kind; 140A2C820 writes it on every first route.
    (0x1403B1C60, OwnReader::Wargoal { expire: true }),
    (0x1403B1FE0, OwnReader::Wargoal { expire: false }),
    (
        0x1403B4B00,
        OwnReader::Fields(
            &[("target", OwnValue::One), ("type", OwnValue::InvalidBlock)],
            Unknown::Skip,
        ),
    ),
    (
        0x14049E600,
        OwnReader::Fields(
            &[
                ("target", OwnValue::One),
                ("type", OwnValue::One),
                ("equipment", OwnValue::One),
                ("amount", OwnValue::One),
                ("new_prioritised", OwnValue::One),
                ("old_prioritised", OwnValue::RawSkipIfOpen),
            ],
            Unknown::Skip,
        ),
    ),
    (
        0x1403B6280,
        OwnReader::Fields(
            &[
                ("target", OwnValue::One),
                ("size", OwnValue::One),
                ("source_organization", OwnValue::One),
                ("target_organization", OwnValue::One),
                ("stockpile_ratio", OwnValue::One),
                ("army_ratio", OwnValue::One),
                ("navy_ratio", OwnValue::One),
                ("air_ratio", OwnValue::One),
                ("keep_unit_leaders", OwnValue::ValueListRequired),
                ("keep_unit_leaders_trigger", OwnValue::Trigger),
            ],
            Unknown::Skip,
        ),
    ),
    (
        0x14031B3B0,
        OwnReader::EffectFallback(&["id", "tooltip", "amount", "days", "occupied", "occupier"]),
    ),
    (
        0x14031BA90,
        OwnReader::EffectFallback(&["clear", "occupied", "occupier"]),
    ),
    (
        0x1404964D0,
        // 140490F40 constructs project/scientist/state scoped-variable fields.
        OwnReader::Fields(
            &[
                ("state", OwnValue::One),
                ("project", OwnValue::One),
                (
                    "iteration_output",
                    OwnValue::Statements(Node::IterationOutput),
                ),
                ("show_modifiers", OwnValue::One),
                ("scientist", OwnValue::One),
            ],
            Unknown::Skip,
        ),
    ),
    (
        0x1403B1040,
        OwnReader::Fields(
            &[
                ("focus", OwnValue::One),
                ("use_side_message", OwnValue::One),
                // 14033C850 constructs CBoundLocalization at effect+0x80.
                ("originator_name", OwnValue::BoundLocalization),
            ],
            Unknown::Effect,
        ),
    ),
    (
        0x1403B1480,
        OwnReader::Fields(
            &[("name", OwnValue::One), ("icon", OwnValue::One)],
            Unknown::Effect,
        ),
    ),
    (
        0x1403ABDF0,
        OwnReader::Keys(&[
            "tooltip",
            "message",
            "tag",
            "winner_scope",
            "loser_scope",
            "score_factor",
        ]),
    ),
    (0x14051BBB0, OwnReader::One),
    (
        0x14051ECD0,
        OwnReader::EffectFallback(&["track", "folder", "sub_doctrine"]),
    ),
];

fn own_reader(address: u64, class: &Token) -> Option<OwnReader> {
    let reader = OWN_KEYS
        .iter()
        .find(|(reader, _)| *reader == address)
        .map(|(_, reader)| *reader)?;
    match reader {
        OwnReader::ClassKeys(classes) => classes
            .iter()
            .find(|(name, _)| class.is_keyword(name))
            .map(|(_, keys)| OwnReader::Keys(keys)),
        _ => Some(reader),
    }
}

pub(crate) struct Shaper<'n, 'c> {
    address: u64,
    reader: OwnReader,
    class: &'c Token,
    names: Option<&'n KeyNames>,
    distribution: u8,
    generator_seen: bool,
}

impl<'n, 'c> Shaper<'n, 'c> {
    pub(crate) fn new(address: u64, names: Option<&'n KeyNames>, class: &'c Token) -> Self {
        Self {
            address,
            reader: own_reader(address, class).expect("ported effect value key loop"),
            class,
            names,
            distribution: 0,
            generator_seen: false,
        }
    }

    pub(crate) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        match node {
            Node::One => Shape::One,
            Node::EquipmentRestrictions => {
                if field.keyword_is("equipment") {
                    Shape::Object(Node::Keys(EQUIPMENT_VARIANT_KEYS))
                } else if field.keyword_is("equipment_variant_index") {
                    Shape::ObjectIfOpen(Node::One)
                } else {
                    Shape::One
                }
            }
            Node::Units(group, placement) => self.unit(group, placement, field),
            Node::Root => self.field(self.address, self.reader, field),
            Node::Keys(address) => {
                let reader = own_reader(address, self.class).expect("ported nested value key loop");
                self.field(address, reader, field)
            }
            Node::IterationOutput => {
                // 140496850 / 140496090 / 140496970: an anonymous opening
                // block is skipped; an assigned opening brace takes one value.
                // fresh distinguishes the callbacks, even for an empty quoted key.
                if !field.fresh && field.opens() {
                    Shape::Skip
                } else {
                    Shape::One
                }
            }
        }
    }

    fn field(&mut self, address: u64, reader: OwnReader, field: &Field<'_>) -> Shape<Node> {
        match reader {
            OwnReader::Promotion => {
                if field.keyword_is("ideology") {
                    Shape::One
                } else if field.keyword_is("character") {
                    match field.effect_scope {
                        Some(scope) if scope == ScopeKind::Character.bit() => Shape::Skip,
                        Some(_) => Shape::One,
                        None if field.skips_block() => Shape::Reported {
                            kind: crate::DiagnosticKind::Unsupported,
                            report: format!("Deferred effect `promote_character.character`: value reader {address:#X}; parse scope unavailable"),
                            read: Box::new(Shape::Unsettled("promote_character.character: character scope skips the current block; other scopes read one value (1403AA3C0)".into())),
                        },
                        None => Shape::One,
                    }
                } else {
                    Shape::Skip
                }
            }
            OwnReader::RemoveRole => {
                if field.keyword_is("character") {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            OwnReader::One => Shape::One,
            OwnReader::Color => {
                if field.keyword_is("color") {
                    Shape::Color
                } else {
                    Shape::One
                }
            }
            OwnReader::Localization => {
                if !field.keyword_is("localization_key") && field.opens() {
                    Shape::Object(Node::Keys(address))
                } else {
                    Shape::One
                }
            }
            OwnReader::Randomize => self.randomize(field).map(|()| unreachable!()),
            OwnReader::Event(_) if field.keyword_is("random") && field.opens() => {
                super::shape(self.names, field).map(|()| unreachable!())
            }
            OwnReader::Keys(keys) | OwnReader::Event(keys) => {
                if keys.iter().any(|key| field.keyword_is(key)) {
                    Shape::One
                } else {
                    Shape::Skip
                }
            }
            OwnReader::EffectFallback(keys) => {
                if keys.iter().any(|key| field.keyword_is(key)) {
                    Shape::One
                } else {
                    super::shape(self.names, field).map(|()| unreachable!())
                }
            }
            OwnReader::Fields(fields, unknown) => {
                if let Some((_, value)) = fields.iter().find(|(name, _)| field.keyword_is(name)) {
                    self.value(*value, field)
                } else {
                    match unknown {
                        Unknown::Skip => Shape::Skip,
                        Unknown::Effect => super::shape(self.names, field).map(|()| unreachable!()),
                    }
                }
            }
            OwnReader::Wargoal { expire } => self.wargoal(expire, field),
            OwnReader::ClassKeys(_) => unreachable!("constructor class selection"),
        }
    }

    fn value(&self, value: OwnValue, field: &Field<'_>) -> Shape<Node> {
        match value {
            OwnValue::One => Shape::One,
            OwnValue::Object(node) => Shape::Object(node),
            OwnValue::ObjectIfOpen(node) => Shape::ObjectIfOpen(node),
            OwnValue::CurrentOrRaw => {
                if field.opens() {
                    Shape::RawList
                } else {
                    Shape::Current
                }
            }
            OwnValue::InvalidBlock if field.opens() => Shape::Reported {
                kind: crate::DiagnosticKind::Misread,
                report: "invalid war goal: {".into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::InvalidBlock => Shape::One,
            OwnValue::RawSkipIfOpen if field.opens() => Shape::RawSkip,
            OwnValue::RawSkipIfOpen => Shape::One,
            OwnValue::ValueListRequired if field.opens() => Shape::ValueList,
            OwnValue::ValueListRequired => Shape::Reported {
                kind: crate::DiagnosticKind::Misread,
                report: "Unexpected token: integer array requires a block".into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::RawListRequired if field.opens() => Shape::RawList,
            OwnValue::RawListRequired => Shape::Reported {
                kind: crate::DiagnosticKind::Misread,
                report:
                    "nationalities requires a block; native RecordError (1403B14C0 -> 1424ACF90)"
                        .into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::RawList => Shape::RawList,
            OwnValue::Trigger => Shape::Trigger,
            OwnValue::BoundLocalization if field.opens() => {
                Shape::Object(Node::Keys(BOUND_LOCALIZATION_READ))
            }
            OwnValue::BoundLocalization => Shape::One,
            // 1404964D0 calls ReadCustomStatementBlock unconditionally, including a scalar.
            OwnValue::Statements(node) => Shape::Statements(node),
            OwnValue::Keywords {
                source,
                accepted,
                report,
            } => {
                let token = match source {
                    TokenSource::Current => field.current,
                    TokenSource::Value => field.value,
                };
                if accepted.iter().any(|keyword| token.is_keyword(keyword)) {
                    Shape::One
                } else {
                    Shape::Reported {
                        kind: crate::DiagnosticKind::Misread,
                        report: report.into(),
                        read: Box::new(Shape::Skip),
                    }
                }
            }
            OwnValue::Deferred(reader) => Shape::Reported {
                kind: crate::DiagnosticKind::Unsupported,
                report: format!(
                    "Deferred effect `{}.{}`: inherited reader {reader:#X}; by-key reading retained",
                    String::from_utf8_lossy(self.class.c_str()),
                    String::from_utf8_lossy(field.key.c_str()),
                ),
                read: Box::new(Shape::ByKey),
            },
        }
    }

    fn wargoal(&mut self, expire: bool, field: &Field<'_>) -> Shape<Node> {
        if field.keyword_is("generator") {
            if self.generator_seen {
                return Shape::Reported {
                    kind: crate::DiagnosticKind::Misread,
                    report: "generator used twice".into(),
                    read: Box::new(Shape::Skip),
                };
            }
            self.generator_seen = true;
            // 140A2C820 selects from the retained token id, even on stale fields.
            if field.opens() {
                Shape::ValueList
            } else {
                Shape::One
            }
        } else if field.keyword_is("type") {
            self.value(OwnValue::InvalidBlock, field)
        } else if field.keyword_is("target") || (expire && field.keyword_is("expire")) {
            Shape::One
        } else {
            Shape::Skip
        }
    }

    fn randomize(&mut self, field: &Field<'_>) -> Shape<()> {
        // strcmp 14253D9F8; literals at 142766390 / 398 / 3A0.
        if field.keyword_is("distribution") {
            let value = match field.value.c_str() {
                b"uniform" => 0,
                b"poisson" => 1,
                b"binomial" => 2,
                _ => return Shape::Skip,
            };
            self.distribution = value;
            Shape::One
        } else if field.keyword_is("min")
            || field.keyword_is("var")
            || (field.keyword_is("max") && self.distribution != 1)
            || (field.keyword_is("lambda") && self.distribution == 1)
        {
            Shape::One
        } else {
            Shape::Skip
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum UnitGroup {
    Regiments,
    Support,
    RegimentalSupport,
}

impl UnitGroup {
    fn support(self) -> bool {
        !matches!(self, Self::Regiments)
    }
    fn column_index(self) -> usize {
        match self {
            Self::Regiments => 0,
            Self::Support => 1,
            Self::RegimentalSupport => 2,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Placement {
    Point,
    Column,
}

impl Shaper<'_, '_> {
    fn unit(&self, group: UnitGroup, placement: Placement, field: &Field<'_>) -> Shape<Node> {
        use crate::unit_leader::SubUnit;
        let Some(key) = field.key.lookup() else {
            return Shape::Skip;
        };
        let Some(catalog) = self.names.and_then(|names| names.sub_units.as_ref()) else {
            return Shape::Unsettled(format!(
                "division-template subunit `{}`: content subunit database at this load point is unavailable (140B937A0 / 1403AF6B0)",
                String::from_utf8_lossy(key)
            ));
        };
        let Ok(name) = std::str::from_utf8(key) else {
            return Shape::Unsettled(format!(
                "division-template subunit {} is not UTF-8",
                String::from_utf8_lossy(key)
            ));
        };
        match catalog.get(name) {
            SubUnit::Unknown => return Shape::Skip,
            SubUnit::Uncertain(reason) => return Shape::Unsettled(reason.into()),
            SubUnit::Index(_) => {}
        }
        let support = match catalog.support(key) {
            Ok(support) => support,
            Err(reason) => return Shape::Unsettled(reason),
        };
        if support != group.support() {
            return Shape::Skip;
        }
        match placement {
            Placement::Point => Shape::TemplatePoint,
            Placement::Column => {
                // Missing defines matter to consumption only if RecordError
                // could skip an opening current token. Ordinary scalar columns
                // take one token on both valid and invalid native routes.
                if !field.skips_block() {
                    return Shape::One;
                }
                let widths = self.names.and_then(|names| names.template_columns.as_ref());
                let width = match widths {
                    Some(Ok(widths)) => widths[group.column_index()] as u32,
                    Some(Err(reason)) => return Shape::Unsettled(reason.clone()),
                    None => return Shape::Unsettled("add_units_to_division_template: recorded NMilitary column-width defines unavailable (1403AF6B0)".into()),
                };
                // ParseTokenUnsigned1424B00A0 -> decimal atoi14254ADC4.
                let column = crate::fixed5::decimal_i32(field.value.c_str()) as u32;
                if column >= width {
                    Shape::Skip
                } else {
                    Shape::One
                }
            }
        }
    }
}

const TEMPLATE_POINT_COMPONENTS: [&[u8]; 4] = [b"x", b"y", b"z", b"w"];

pub(crate) fn template_point(reader: &mut Reader<'_>) -> Value {
    fn take(reader: &mut Reader<'_>) -> Token {
        reader.advance();
        let token = reader.resolve(reader.current.clone());
        reader.current = token.clone();
        token
    }
    fn error(reader: &mut Reader<'_>, message: String) {
        let line = reader.current.line;
        reader.note_native_error(line, message);
        // RecordError1424ACF90, not TryHandleOrRecordError1424ACD40.
        reader.skip_reported();
    }
    fn item(key: Option<Vec<u8>>, token: Token) -> Item {
        Item {
            key,
            key_quoted: false,
            operator: Operator::Equals,
            kind: token.kind,
            value: Value::Scalar(token.text),
        }
    }
    if reader.value.kind != Kind::Open {
        let value = reader.value.text.clone();
        error(
            reader,
            "division-template point: Expected block (140B847D0)".into(),
        );
        return Value::Scalar(value);
    }
    let mut items = Vec::new();
    let mut component = 0usize;
    loop {
        // Native snapshots end before advancing/resolving: resolving the
        // closing token does not alter this stopping decision.
        let ends = reader.peek().ends_block();
        let token = take(reader);
        if ends {
            break;
        }
        if token.is_number() {
            // 1424B0220 stores unsigned16. The reader replay keeps the
            // consumed token, leaving conversion to the point consumer.
            items.push(item(None, token));
            component += 1;
            continue;
        }
        let Some(expected) = TEMPLATE_POINT_COMPONENTS.get(component) else {
            // Native piVar13 now addresses beyond its four initialized IDs.
            // Do not invent an expected name or a native component bound.
            reader.refuse_native(
                token.line,
                format!(
                    "division-template point for {}: nonnumeric component {} beyond the initialized x/y/z/w labels (140B847D0); native expected-label read is outside its local array",
                    String::from_utf8_lossy(reader.key.c_str()),
                    String::from_utf8_lossy(token.c_str())
                ),
            );
            break;
        };
        if !token
            .lookup()
            .is_some_and(|key| key.eq_ignore_ascii_case(expected))
        {
            error(
                reader,
                format!(
                    "division-template point: Expected component {} (140B847D0)",
                    String::from_utf8_lossy(expected)
                ),
            );
            continue;
        }
        let equals = take(reader);
        if equals.kind != Kind::Equals {
            error(
                reader,
                format!(
                    "division-template point: Expected '=' after {} (140B847D0)",
                    String::from_utf8_lossy(expected)
                ),
            );
            continue;
        }
        let number = take(reader);
        if !number.is_number() {
            error(
                reader,
                "division-template point: Expected a number after '=' (140B847D0)".into(),
            );
            continue;
        }
        // Keep named x/y points compatible with the existing identity
        // harness representation. This is still raw token consumption:
        // neither CReader key/operator/value slots have been changed.
        items.push(item(Some(token.text), number));
        component += 1;
    }
    Value::Block(items)
}

#[derive(Clone, Copy)]
enum ScopePolicy {
    Set(u32),
    Preserve,
    Unknown,
}

// Only proven ReadEffect fallback paths are listed. An untraced slot resets
// context to unknown rather than assuming it preserves the enclosing kind.
const KEY_SCOPE_POLICIES: &[(u64, ScopePolicy)] = &[
    (0x1402F1060, ScopePolicy::Preserve), // add_trait, remove_trait; fallback 1402F109B
    (0x1402F6D70, ScopePolicy::Set(ScopeKind::Character.bit())), // every_scientist, every_active_scientist, every_character, character_list_tooltip; MOV at 1402F6DAA
    (0x1402F6ED0, ScopePolicy::Set(ScopeKind::Character.bit())), // random_scientist, random_active_scientist, random_character, party_leader; MOV at 1402F6EF8
    (
        0x140307920,
        ScopePolicy::Set(ScopeKind::PurchaseContract.bit()),
    ), // every_purchase_contract; MOV at 14030795A
    (
        0x140307A80,
        ScopePolicy::Set(ScopeKind::PurchaseContract.bit()),
    ), // random_purchase_contract; MOV at 140307AA8
    (0x140307BA0, ScopePolicy::Set(ScopeKind::Country.bit())), // add_equipment_subsidy; MOV at 140307BC7
    (0x140307C20, ScopePolicy::Set(ScopeKind::Country.bit())), // create_purchase_contract; MOV at 140307C6E
    (0x14031B220, ScopePolicy::Set(ScopeKind::State.bit())), // every_state, every_owned_state, every_neighbor_state, every_controlled_state, every_core_state; MOV at 14031B25A
    (0x1403234E0, ScopePolicy::Preserve), // add_doctrine_cost_reduction; fallback 14032353F
    (0x140323AC0, ScopePolicy::Preserve), // add_tech_bonus; fallback 140323CF8
    (0x140323F70, ScopePolicy::Preserve), // steal_random_tech_bonus; fallback 140324602
    (0x1403ADA40, ScopePolicy::Preserve), // find_lowest_in_array, find_highest_in_array; fallback 1403ADA8A
    (0x1403ADCF0, ScopePolicy::Set(ScopeKind::Country.bit())), // every_country, every_neighbor_country, every_enemy_country, every_subject_country, every_faction_member, every_other_country, every_allied_country, every_occupied_country, every_possible_country; MOV at 1403ADD2A
    (0x1403ADE50, ScopePolicy::Set(ScopeKind::State.bit())), // random_state, random_owned_state, random_neighbor_state, random_owned_controlled_state, random_controlled_state, random_core_state; MOV at 1403AE778 shared tail
    (0x1403AE630, ScopePolicy::Set(ScopeKind::Country.bit())), // random_country, random_neighbor_country, random_enemy_country, random_subject_country, random_other_country, random_allied_country, random_occupied_country; MOV at 1403AE658
    (0x1403AEA80, ScopePolicy::Preserve), // add_advisor_role; fallback 1403AEAAA
    (0x1403AEC00, ScopePolicy::Preserve), // add_country_leader_role; fallback 1403AEC2E
    (0x1403AEDB0, ScopePolicy::Preserve), // add_intel; fallback 1403AEDF1
    (0x1403B0960, ScopePolicy::Preserve), // build_railway; fallback 1403B0A68
    (0x1403B1120, ScopePolicy::Set(ScopeKind::Country.bit())), // create_dynamic_country; MOV at 1403B1132
    (0x1403B2310, ScopePolicy::Preserve),                      // destroy_ships; fallback 1403B2333
    (0x1403B2390, ScopePolicy::Preserve), // diplomatic_relation; fallback 1403B23C9
    (0x1403B2620, ScopePolicy::Set(ScopeKind::Country.bit())), // every_country_with_original_tag; MOV at 1403B2667
    (0x1403B2890, ScopePolicy::Preserve),                      // for_each_loop; fallback 1403B28E7
    (0x1403B2B50, ScopePolicy::Set(ScopeKind::all_bits())), // for_each_scope_loop; MOV at 1403B2B9A
    (0x1403B2E10, ScopePolicy::Preserve),                   // for_loop_effect; fallback 1403B2E89
    (0x1403B33C0, ScopePolicy::Preserve), // create_country_leader; fallback 1403B3460
    (0x1403B3970, ScopePolicy::Set(ScopeKind::Country.bit())), // random_country_with_original_tag; MOV at 1403B39A5
    (0x1403B3AB0, ScopePolicy::Preserve), // random_scope_in_array; fallback 1403B3AEE
    (0x1403B4720, ScopePolicy::Preserve), // remove_advisor_role; fallback 1403B473E
    (0x1403B47D0, ScopePolicy::Preserve), // remove_country_leader_role; fallback 1403B47F1
    (0x1403B4E60, ScopePolicy::Preserve), // set_can_be_fired_in_advisor_role; fallback 1403B4E8E
    (0x1403B5C20, ScopePolicy::Preserve), // start_civil_war; fallback 1403B5D9A
    (0x1403B5EB0, ScopePolicy::Preserve), // add_timed_idea, modify_timed_idea; fallback 1403B5EE2
    (0x1403B6960, ScopePolicy::Preserve), // while_loop_effect; fallback 1403B6980
    (0x14045E970, ScopePolicy::Set(ScopeKind::Unit.bit())), // every_state_division, every_country_division; MOV at 14045E9AA
    (0x14045EAD0, ScopePolicy::Set(ScopeKind::Unit.bit())), // random_state_division, random_country_division; MOV at 14045EAF8
    (0x14045EC40, ScopePolicy::Preserve), // add_random_valid_trait_from_unit; fallback 14045EC51
    (0x1404B12F0, ScopePolicy::Preserve), // add_faction_goal_slot; fallback 1404B1313
    (0x14051EB60, ScopePolicy::Preserve), // add_daily_mastery; fallback 14051ECA9
    (0x14051EBB0, ScopePolicy::Preserve), // add_mastery_bonus; fallback 14051ECA9
    (0x14051EC00, ScopePolicy::Preserve), // add_mastery; fallback 14051EC42
    (super::READ_EFFECT, ScopePolicy::Preserve), // ReadScript forwards its caller kind (140534441/44E).
    (0x141385BA0, ScopePolicy::Preserve),        // if; fallback 141385BDC
    (0x14138BE60, ScopePolicy::Set(ScopeKind::Character.bit())), // every_unit_leader, every_army_leader, every_navy_leader, global_every_army_leader, every_operative; MOV at 14138BEA7
    (0x14138BFE0, ScopePolicy::Set(ScopeKind::Character.bit())), // random_unit_leader, random_army_leader, random_navy_leader, random_operative; MOV at 14138C015
    (
        0x14139AF50,
        ScopePolicy::Set(ScopeKind::IndustrialOrganisation.bit()),
    ), // every_military_industrial_organization; MOV at 14139AF97
    (
        0x14139B0D0,
        ScopePolicy::Set(ScopeKind::IndustrialOrganisation.bit()),
    ), // random_military_industrial_organization; MOV at 14139B105
];

// ReadEffect's two pre-registry conditional tokens use the if class.
const SCOPE_CLASS_ALIASES: &[(u32, u32)] = &[
    (ELSE_TOKEN, IF_TOKEN),    // else -> if, 1405345D0
    (ELSE_IF_TOKEN, IF_TOKEN), // else_if -> if, 1405345D0
];

pub(crate) fn read_script_scope(token: u32, enclosing: Option<u32>) -> Option<u32> {
    let token = SCOPE_CLASS_ALIASES
        .iter()
        .find(|(alias, _)| *alias == token)
        .map_or(token, |(_, class)| *class);
    let (_, key_reader) = super::class(token)?;
    let policy = KEY_SCOPE_POLICIES
        .iter()
        .find(|(reader, _)| *reader == key_reader)
        .map_or(ScopePolicy::Unknown, |(_, policy)| *policy);
    match policy {
        ScopePolicy::Set(scope) => Some(scope),
        ScopePolicy::Preserve => enclosing,
        ScopePolicy::Unknown => None,
    }
}

// GetEventTargetScopeKind 14146D870 walks to the final nested target.
// Numeric zero is a native result, never an instruction to inherit.
fn target_kind(target: &crate::scope::Scope) -> u32 {
    use crate::scope::{Keyword, Scope};
    match target {
        Scope::Chain { then, .. }
        | Scope::EventTarget {
            then: Some(then), ..
        } => target_kind(then),
        Scope::Country(_) => ScopeKind::Country.bit(),
        Scope::Character(_) => ScopeKind::Character.bit(),
        Scope::State(_) | Scope::ZeroNumber(_) => ScopeKind::State.bit(),
        Scope::Keyword(Keyword::Character) => ScopeKind::Character.bit(),
        Scope::Keyword(
            Keyword::Owner
            | Keyword::Controller
            | Keyword::Overlord
            | Keyword::Occupied
            | Keyword::FactionLeader,
        ) => ScopeKind::Country.bit(),
        Scope::Keyword(Keyword::CapitalScope) => ScopeKind::State.bit(),
        Scope::Keyword(Keyword::Operation) => ScopeKind::Operation.bit(),
        Scope::Keyword(Keyword::Opponent) => ScopeKind::Combat.bit(),
        Scope::Prefixed { prefix, value } => match prefix.as_str() {
            "mio" => ScopeKind::IndustrialOrganisation.bit(),
            "sp" => ScopeKind::Project.bit(),
            "scope" if value.eq_ignore_ascii_case("faction") => ScopeKind::Faction.bit(),
            _ => 0,
        },
        _ => 0,
    }
}

/// ReadEffect1405345D0 replaces context only after registry lookup failed.
pub(crate) fn body_scope(
    names: Option<&KeyNames>,
    key: &Token,
    enclosing: Option<u32>,
) -> Option<u32> {
    if key.lookup().and_then(super::registered).is_some() {
        return enclosing;
    }
    let Some(names) = names else { return enclosing };
    match names.scope(key.c_str(), key.kind == Kind::Quoted) {
        crate::trigger_keys::ScopeStep::Scope(target) => Some(target_kind(&target)),
        _ => enclosing,
    }
}

#[cfg(test)]
mod tests;
