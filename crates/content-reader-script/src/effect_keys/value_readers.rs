
use crate::{key_names::KeyNames, native::{Field, Kind, Reader, Shape, Token}, scope::ScopeKind};
use content_reader_parser::parser::{Item, Operator, Value};

use super::{
    IF_TOKEN,
    key_readers::{ELSE_IF_TOKEN, ELSE_TOKEN},
};

mod reader_identity {
    pub(super) const RANDOM_TRAIT_VALUE: u64 = 0x140309020;
    pub(super) const REMOVE_UNIT_LEADER_VALUE: u64 = 0x140309040;
    pub(super) const SWAP_COUNTRY_TRAITS: u64 = 0x140309310;
    pub(super) const CAPTURE_GENERAL_KEYS: u64 = 0x140311140;
    pub(super) const RELEASE_GENERAL_KEYS: u64 = 0x140311180;
    pub(super) const TRAIT_TOOLTIP_KEYS: u64 = 0x140319F20;
    pub(super) const POLICY_TOOLTIP_KEYS: u64 = 0x140319F30;
    pub(super) const RESISTANCE_VALUE: u64 = 0x140333C10;
    pub(super) const RESISTANCE_KEYS: u64 = 0x140333F20;
    pub(super) const REMOVE_RESISTANCE_KEYS: u64 = 0x140334600;
    pub(super) const RANDOMIZE_VALUE: u64 = 0x1403C15F0;
    pub(super) const ADVISOR_VALUE: u64 = 0x1403C1E00;
    pub(super) const MISSION_TOOLTIP: u64 = 0x1403C1EC0;
    pub(super) const COUNTRY_TRAIT: u64 = 0x1403C2120;
    pub(super) const DIVISION_TEMPLATE_VALUE: u64 = 0x1403C23D0;
    pub(super) const IDEAS_VALUE: u64 = 0x1403C2420;
    pub(super) const SCRIPT_VALUE: u64 = 0x1403C2750;
    pub(super) const FOCUS_VALUE: u64 = 0x1403C2760;
    pub(super) const AGENCY_VALUE: u64 = 0x1403C27B0;
    pub(super) const CREATE_OPERATIVE_VALUE: u64 = 0x1403C27C0;
    pub(super) const DECISION_VALUE: u64 = 0x1403C2830;
    pub(super) const PUPPET: u64 = 0x1403C2880;
    pub(super) const MISSION: u64 = 0x1403C2AD0;
    pub(super) const OPERATIVE_VALUE: u64 = 0x1403C2D10;
    pub(super) const PROMOTION: u64 = 0x1403C2FD0;
    pub(super) const RECRUIT_CHARACTER_VALUE: u64 = 0x1403C3660;
    pub(super) const REMOVE_ROLE: u64 = 0x1403C3810;
    pub(super) const RETIRE_CHARACTER_VALUE: u64 = 0x1403C3BB0;
    pub(super) const OOB_VALUE: u64 = 0x1403C3C50;
    pub(super) const FOCUS_TREE_VALUE: u64 = 0x1403C3C90;
    pub(super) const CHARACTER_NAME: u64 = 0x1403C3ED0;
    pub(super) const CHARACTER_NATIONALITY: u64 = 0x1403C41A0;
    pub(super) const IDEAS_TOOLTIP_VALUE: u64 = 0x1403C4510;
    pub(super) const LEADERS_TOOLTIP_VALUE: u64 = 0x1403C4950;
    pub(super) const PEACE_CONFERENCE: u64 = 0x1403C4A00;
    pub(super) const SWAP_IDEAS: u64 = 0x1403C4CE0;
    pub(super) const SWAP_RULER_TRAITS: u64 = 0x1403C5260;
    pub(super) const DECISION_CATEGORY_VALUE: u64 = 0x1403C5890;
    pub(super) const DECISION_TOOLTIP: u64 = 0x1403C58E0;
    pub(super) const WHITE_PEACE: u64 = 0x1403C5B50;
    pub(super) const RANDOMIZE_KEYS: u64 = 0x1403C6AA0;
    pub(super) const POPULARITY_KEYS: u64 = 0x1403C7C50;
    pub(super) const ADD_TEMPLATE_UNITS_KEYS: u64 = 0x1403C82E0;
    pub(super) const CAPTURE_OPERATIVE_KEYS: u64 = 0x1403C9760;
    pub(super) const COMPLETE_FOCUS_KEYS: u64 = 0x1403C9C80;
    pub(super) const COLONIAL_TEMPLATE_KEYS: u64 = 0x1403C9CD0;
    pub(super) const AGENCY_KEYS: u64 = 0x1403CA0C0;
    pub(super) const CREATE_OPERATIVE_KEYS: u64 = 0x1403CA100;
    pub(super) const CREATE_WARGOAL_KEYS: u64 = 0x1403CA8A0;
    pub(super) const DECLARE_WAR_KEYS: u64 = 0x1403CAC20;
    pub(super) const TEMPLATE_LOCK_KEYS: u64 = 0x1403CC110;
    pub(super) const OPERATIVE_COUNTRY_KEYS: u64 = 0x1403CC4E0;
    pub(super) const RELEASE_COUNTRY_KEYS: u64 = 0x1403CD0D0;
    pub(super) const SCIENTIST_ROLE_KEYS: u64 = 0x1403CD610;
    pub(super) const REMOVE_WARGOAL_KEYS: u64 = 0x1403CD750;
    pub(super) const AUTONOMY_KEYS: u64 = 0x1403CD910;
    pub(super) const FOCUS_TREE_KEYS: u64 = 0x1403CDBF0;
    pub(super) const TRANSFER_LEADERS_KEYS: u64 = 0x1403CEEF0;
    pub(super) const HIGHEST_TEMP_VALUE: u64 = 0x14049C6F0;
    pub(super) const HIGHEST_VALUE: u64 = 0x14049CB40;
    pub(super) const SORTED_TEMP_VALUE: u64 = 0x14049CF90;
    pub(super) const SORTED_VALUE: u64 = 0x14049D3E0;
    pub(super) const SUPPLY_VALUE: u64 = 0x14049D830;
    pub(super) const MODULO_TEMP_VALUE: u64 = 0x14049E0D0;
    pub(super) const MODULO_VALUE: u64 = 0x14049E520;
    pub(super) const RANDOM_TEMP_VALUE: u64 = 0x14049E970;
    pub(super) const RANDOM_VARIABLE_VALUE: u64 = 0x14049edf0;
    pub(super) const SCORED_VARIABLE_KEYS: u64 = 0x1404A0950;
    pub(super) const SCORED_ARRAY_KEYS: u64 = 0x1404A0990;
    pub(super) const SUPPLY_KEYS: u64 = 0x1404A09E0;
    pub(super) const MODULO_KEYS: u64 = 0x1404A0A30;
    pub(super) const RANDOM_VARIABLE_KEYS: u64 = 0x1404A0A40;
    pub(super) const SCIENTIST_TRAIT_VALUE: u64 = 0x1404B58B0;
    pub(super) const INJURE_SCIENTIST_VALUE: u64 = 0x1404B5B60;
    pub(super) const PROJECT_COMPLETION_KEYS: u64 = 0x1404b5fc0;
    pub(super) const EQUIPMENT_AMOUNT_KEYS: u64 = 0x1404BE0F0;
    pub(super) const TARGET_VALUE_KEYS: u64 = 0x1404BE2C0;
    pub(super) const FACTION_COLOR_KEYS: u64 = 0x1404D0D60;
    pub(super) const STRATEGIC_LOCATION: u64 = 0x14056F280;
    pub(super) const DOCTRINE_KEYS: u64 = 0x1405723a0;
    pub(super) const DIVISION_TEMPLATE_KEYS: u64 = 0x140BF5D60;
    pub(super) const LOCALIZATION_VALUE: u64 = 0x141436040;
    pub(super) const EVENT_VALUE: u64 = 0x141436050;
    pub(super) const FLAG_VALUE: u64 = 0x141436070;
    pub(super) const MODIFY_FLAG_VALUE: u64 = 0x141436350;
    pub(super) const ASSIGNMENT_VALUE: u64 = 0x141436860;
    pub(super) const OPERATIVE_EVENT_KEYS: u64 = 0x141436880;
    pub(super) const EVENT_KEYS: u64 = 0x141436AD0;
    pub(super) const FLAG_KEYS: u64 = 0x141436B00;
    pub(super) const MODIFY_FLAG_KEYS: u64 = 0x141436F70;
    pub(super) const EQUIPMENT_VARIANT_KEYS: u64 = 0x1414C4850;
    pub(super) const BOUND_LOCALIZATION_READ: u64 = 0x142426550;
}

use reader_identity::*;

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

fn value_form(reader: u64) -> Option<Form> {
    use LoopSource::{ClassKey, Inline};
    Some(match reader {
        REMOVE_UNIT_LEADER_VALUE => Form::SkipIfOpen,
        OOB_VALUE
        | SCIENTIST_TRAIT_VALUE
        | ADVISOR_VALUE
        | RECRUIT_CHARACTER_VALUE
        | RETIRE_CHARACTER_VALUE
        | INJURE_SCIENTIST_VALUE => Form::One,
        DECISION_VALUE | LEADERS_TOOLTIP_VALUE | DECISION_CATEGORY_VALUE => Form::Current,
        IDEAS_TOOLTIP_VALUE => Form::CurrentOrRaw,
        MISSION_TOOLTIP | MISSION => Form::CurrentOrKeys(Inline),
        RANDOMIZE_VALUE | FOCUS_TREE_VALUE | EVENT_VALUE | FLAG_VALUE | ASSIGNMENT_VALUE
        | RESISTANCE_VALUE | FOCUS_VALUE | AGENCY_VALUE | OPERATIVE_VALUE => {
            Form::ScalarOrKeys(ClassKey)
        }
        PUPPET
        | CHARACTER_NAME
        | CHARACTER_NATIONALITY
        | DECISION_TOOLTIP
        | WHITE_PEACE
        | PROMOTION
        | COUNTRY_TRAIT
        | PEACE_CONFERENCE => Form::ScalarOrKeys(Inline),
        SCRIPT_VALUE
        | HIGHEST_TEMP_VALUE
        | HIGHEST_VALUE
        | SORTED_TEMP_VALUE
        | SORTED_VALUE
        | SUPPLY_VALUE
        | MODULO_TEMP_VALUE
        | MODULO_VALUE
        | MODIFY_FLAG_VALUE
        | CREATE_OPERATIVE_VALUE => Form::Keys(ClassKey),
        SWAP_IDEAS | SWAP_COUNTRY_TRAITS | SWAP_RULER_TRAITS => Form::Keys(Inline),
        RANDOM_TEMP_VALUE | RANDOM_VARIABLE_VALUE => Form::StringOrKeys(ClassKey),
        LOCALIZATION_VALUE => Form::Localization,
        DIVISION_TEMPLATE_VALUE => Form::Persistent(DIVISION_TEMPLATE_KEYS),
        REMOVE_ROLE => Form::RoleKeys(Inline),
        RANDOM_TRAIT_VALUE => Form::List,
        IDEAS_VALUE => Form::Ideas,
        STRATEGIC_LOCATION => Form::AlwaysKeys(Inline),
        _ => return None,
    })
}

pub(super) fn shape(value: u64, key: u64, field: &Field<'_>) -> Option<Shape<()>> {
    let (source, scalar) = match value_form(value)? {
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
            own_reader(reader, |name| field.key.is_keyword(name))?;
            return Some(Shape::EffectValueKeys(reader));
        }
        Form::AlwaysKeys(source) => {
            let reader = source.address(value, key);
            own_reader(reader, |name| field.key.is_keyword(name))?;
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
            return Some(if field.opens() {
                Shape::RawList
            } else {
                Shape::Current
            });
        }
        Form::Localization => return Some(super::bound_localization_shape(field)),
        Form::RoleKeys(source) => match field.effect_scope {
            Some(scope) if scope == ScopeKind::Character.bit() => {
                return Some(if field.value.c_str() == b"yes" {
                    Shape::One
                } else {
                    Shape::Reported {
                        kind: content_reader_parser::DiagnosticKind::Misread,
                        report: "remove_unit_leader_role in character scope requires exact yes"
                            .into(),
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
    own_reader(reader, |name| field.key.is_keyword(name))?;
    if field.opens() {
        return Some(Shape::EffectValueKeys(reader));
    }
    match scalar {
        ScalarRead::One => Some(Shape::One),
        ScalarRead::Deferred => None,
        ScalarRead::Block => Some(Shape::Reported {
            kind: content_reader_parser::DiagnosticKind::Misread,
            report: format!(
                "effect `{}` requires a block",
                String::from_utf8_lossy(field.key.c_str())
            ),
            read: Box::new(Shape::Skip),
        }),
    }
}

fn string_scalar(field: &Field<'_>) -> ScalarRead {
    if !field.fresh {
        ScalarRead::Deferred
    } else if field.value.kind == Kind::Quoted {
        ScalarRead::One
    } else if field.value.kind == Kind::Bare
        && !field.value.is_number()
        && engine_executable::executable_tables::required(
            engine_executable::executable_tables::get()
                .keywords
                .as_ref(),
        )
        .static_keyword(field.value.c_str())
        .is_none()
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
    Gender,
    OperativeState,
    Deferred,
}

#[derive(Clone, Copy)]
enum OwnReader {
    CaptureGeneral,
    ReleaseGeneral,
    ScientistRole,
    Mission,
    MissionTooltip,
    Promotion,
    RemoveRole,
    DivisionTemplate,
    AddTemplateUnits,
    ColonialTemplate,
    EquipmentVariant,
    TraitTooltip,
    PolicyTooltip,
    Puppet,
    CharacterName,
    CharacterNationality,
    SwapIdeas,
    DecisionTooltip,
    WhitePeace,
    Randomize,
    Popularity,
    One,
    ReleaseCountry,
    Autonomy,
    FocusTree,
    ScoredVariable,
    ScoredArray,
    Supply,
    RandomVariable,
    TargetValue,
    Color,
    OperativeEvent,
    Event,
    Flag,
    Localization,
    SwapCountryTraits,
    CountryTrait,
    SwapRulerTraits,
    CaptureOperative,
    KillOperative,
    TurnOperative,
    CreateOperative,
    Wargoal { expire: bool },
    RemoveWargoal,
    EquipmentAmount,
    TransferLeaders,
    Resistance,
    RemoveResistance,
    ProjectCompletion,
    CompleteFocus,
    Agency,
    PeaceConference,
    Doctrine,
}

impl OwnReader {
    fn falls_back_to_effect(self) -> bool {
        matches!(
            self,
            Self::Popularity
                | Self::Resistance
                | Self::RemoveResistance
                | Self::CompleteFocus
                | Self::Agency
                | Self::Doctrine
        )
    }

    fn is_event(self) -> bool {
        matches!(self, Self::OperativeEvent | Self::Event)
    }

    fn token_value(self, key: &Token) -> Option<OwnValue> {
        self.value(&key.lookup()?.to_ascii_lowercase())
    }

    fn item_value(self, item: &Item) -> Option<OwnValue> {
        self.value(&item.keyword()?.folded())
    }

    fn value(self, key: &[u8]) -> Option<OwnValue> {
        use OwnValue::{Object, ObjectIfOpen, One};
        use UnitGroup::{RegimentalSupport, Regiments, Support};
        Some(match (self, key) {
            (Self::Wargoal { .. }, b"type") => OwnValue::InvalidBlock,
            (Self::Wargoal { .. }, b"target") | (Self::Wargoal { expire: true }, b"expire") => One,
            (Self::CaptureGeneral, b"province" | b"country")
            | (Self::ReleaseGeneral, b"target" | b"province")
            | (Self::ScientistRole, b"character")
            | (Self::Mission, b"mission")
            | (Self::MissionTooltip, b"mission" | b"desc") => One,
            (
                Self::DivisionTemplate,
                b"name"
                | b"localization_key"
                | b"priority"
                | b"country"
                | b"obsolete_change_date"
                | b"colonial"
                | b"force_allow_recruiting"
                | b"obsolete"
                | b"template_counter"
                | b"ingame_set_template_counter"
                | b"original_tag"
                | b"allow_new_equipment"
                | b"role"
                | b"division_names_group"
                | b"is_locked"
                | b"foreign_template_tag"
                | b"origin_type"
                | b"override_model"
                | b"is_army_hq"
                | b"is_fake_intel_division"
                | b"equipment_niche"
                | b"allow_foreign_equipment"
                | b"division_limit",
            ) => One,
            (Self::DivisionTemplate, b"parent") => ObjectIfOpen(Node::One),
            (Self::DivisionTemplate, b"inherit") => OwnValue::RawSkipIfOpen,
            (Self::DivisionTemplate, b"support") => Object(Node::Units(Support, Placement::Point)),
            (Self::DivisionTemplate, b"regiments") => {
                Object(Node::Units(Regiments, Placement::Point))
            }
            (Self::DivisionTemplate, b"regimental_support") => {
                Object(Node::Units(RegimentalSupport, Placement::Point))
            }
            (Self::DivisionTemplate, b"not_allowed_equipment") => {
                Object(Node::EquipmentRestrictions)
            }
            (Self::DivisionTemplate, b"override_model_equipment_category_filter") => {
                OwnValue::CurrentOrRaw
            }
            (Self::DivisionTemplate, b"forbidden_equipment_types") => Object(Node::One),
            (Self::AddTemplateUnits, b"template_name") => One,
            (Self::AddTemplateUnits, b"regiments") => {
                Object(Node::Units(Regiments, Placement::Column))
            }
            (Self::AddTemplateUnits, b"support") => Object(Node::Units(Support, Placement::Column)),
            (Self::AddTemplateUnits, b"regimental_support") => {
                Object(Node::Units(RegimentalSupport, Placement::Column))
            }
            (Self::ColonialTemplate, b"subject") => One,
            (Self::ColonialTemplate, b"division_template") => {
                Object(Node::Keys(DIVISION_TEMPLATE_KEYS))
            }
            (
                Self::EquipmentVariant,
                b"type" | b"version" | b"creator" | b"create_if_missing" | b"version_name"
                | b"license_creator",
            )
            | (Self::TraitTooltip, b"trait" | b"show_modifiers")
            | (Self::PolicyTooltip, b"policy" | b"show_modifiers")
            | (Self::Puppet, b"target" | b"end_civil_wars" | b"end_wars")
            | (Self::CharacterName, b"name" | b"character")
            | (Self::CharacterNationality, b"target_country" | b"character")
            | (Self::SwapIdeas, b"days" | b"remove_idea" | b"add_idea" | b"add_days")
            | (Self::DecisionTooltip, b"decision" | b"show_effect_tooltip" | b"show_modifiers")
            | (Self::WhitePeace, b"tooltip" | b"message" | b"tag")
            | (Self::Popularity, b"ideology" | b"popularity") => One,
            (
                Self::ReleaseCountry,
                b"target"
                | b"autonomous_state"
                | b"autonomy_state"
                | b"freedom_level"
                | b"release_non_owned_controlled"
                | b"force_change_controller_for_non_ally_controlled",
            )
            | (
                Self::Autonomy,
                b"target" | b"autonomous_state" | b"autonomy_state" | b"freedom_level"
                | b"end_civil_wars" | b"end_wars" | b"keep_subjects",
            )
            | (Self::FocusTree, b"tree" | b"keep_completed" | b"copy_completed_from")
            | (Self::ScoredVariable, b"var" | b"scorer")
            | (Self::ScoredArray, b"array" | b"scores" | b"scorer")
            | (Self::Supply, b"var" | b"type" | b"need")
            | (Self::RandomVariable, b"min" | b"max" | b"var" | b"integer")
            | (Self::TargetValue, b"target" | b"value") => One,
            (
                Self::OperativeEvent | Self::Event,
                b"id" | b"tooltip" | b"trigger_for" | b"months" | b"days" | b"hours"
                | b"random_hours" | b"random_days" | b"random" | b"set_from_from" | b"originator",
            )
            | (Self::OperativeEvent, b"recipient" | b"set_from" | b"set_root")
            | (Self::Flag, b"flag" | b"value" | b"days")
            | (Self::SwapCountryTraits, b"ideology" | b"remove" | b"add")
            | (Self::CountryTrait, b"ideology" | b"trait")
            | (Self::SwapRulerTraits, b"remove" | b"add")
            | (Self::CaptureOperative, b"captured_by" | b"operative" | b"ignore_death_chance")
            | (Self::KillOperative, b"killed_by" | b"operative")
            | (Self::TurnOperative, b"turned_by" | b"operative") => One,
            (
                Self::CreateOperative,
                b"gfx"
                | b"bypass_recruitment"
                | b"available_to_spy_master"
                | b"portrait_tag_override"
                | b"id"
                | b"name"
                | b"female"
                | b"skill"
                | b"picture"
                | b"portrait_path"
                | b"rank"
                | b"desc"
                | b"experience"
                | b"max_traits"
                | b"custom_cost_text"
                | b"link"
                | b"government_in_exile_tag"
                | b"promoted_from_unit"
                | b"cooldown_reason"
                | b"script_id"
                | b"legacy_id"
                | b"leader_modifier_enable_date"
                | b"leader_cooldown_start_date"
                | b"capture_date"
                | b"captured",
            ) => One,
            (Self::CreateOperative, b"nationalities") => OwnValue::RawListRequired,
            (Self::CreateOperative, b"traits") => OwnValue::RawList,
            (Self::CreateOperative, b"gender") => OwnValue::Gender,
            (Self::CreateOperative, b"state") => OwnValue::OperativeState,
            (Self::CreateOperative, b"flags") => OwnValue::RawSkipIfOpen,
            (Self::CreateOperative, b"operation") => ObjectIfOpen(Node::One),
            (
                Self::CreateOperative,
                b"traits_to_remove"
                | b"in_progress"
                | b"trait_xp_factor"
                | b"codename"
                | b"mission"
                | b"sub_unit_modifiers"
                | b"dynamic_modifier",
            ) => OwnValue::Deferred,
            (Self::RemoveWargoal, b"target") => One,
            (Self::RemoveWargoal, b"type") => OwnValue::InvalidBlock,
            (
                Self::EquipmentAmount,
                b"target" | b"type" | b"equipment" | b"amount" | b"new_prioritised",
            ) => One,
            (Self::EquipmentAmount, b"old_prioritised") => OwnValue::RawSkipIfOpen,
            (
                Self::TransferLeaders,
                b"target"
                | b"size"
                | b"source_organization"
                | b"target_organization"
                | b"stockpile_ratio"
                | b"army_ratio"
                | b"navy_ratio"
                | b"air_ratio",
            ) => One,
            (Self::TransferLeaders, b"keep_unit_leaders") => OwnValue::ValueListRequired,
            (Self::TransferLeaders, b"keep_unit_leaders_trigger") => OwnValue::Trigger,
            (
                Self::Resistance,
                b"id" | b"tooltip" | b"amount" | b"days" | b"occupied" | b"occupier",
            )
            | (Self::RemoveResistance, b"clear" | b"occupied" | b"occupier")
            | (Self::ProjectCompletion, b"state" | b"project" | b"show_modifiers" | b"scientist") => {
                One
            }
            (Self::ProjectCompletion, b"iteration_output") => {
                OwnValue::Statements(Node::IterationOutput)
            }
            (Self::CompleteFocus, b"focus" | b"use_side_message") => One,
            (Self::CompleteFocus, b"originator_name") => OwnValue::BoundLocalization,
            (Self::Agency, b"name" | b"icon")
            | (
                Self::PeaceConference,
                b"tooltip" | b"message" | b"tag" | b"winner_scope" | b"loser_scope"
                | b"score_factor",
            )
            | (Self::Doctrine, b"track" | b"folder" | b"sub_doctrine") => One,
            _ => return None,
        })
    }
}

fn own_reader(address: u64, is_class: impl Fn(&str) -> bool) -> Option<OwnReader> {
    use OwnReader as Grammar;
    Some(match address {
        CAPTURE_GENERAL_KEYS => Grammar::CaptureGeneral,
        RELEASE_GENERAL_KEYS => Grammar::ReleaseGeneral,
        SCIENTIST_ROLE_KEYS => Grammar::ScientistRole,
        MISSION => Grammar::Mission,
        MISSION_TOOLTIP => Grammar::MissionTooltip,
        PROMOTION => Grammar::Promotion,
        REMOVE_ROLE => Grammar::RemoveRole,
        DIVISION_TEMPLATE_KEYS => Grammar::DivisionTemplate,
        ADD_TEMPLATE_UNITS_KEYS => Grammar::AddTemplateUnits,
        COLONIAL_TEMPLATE_KEYS => Grammar::ColonialTemplate,
        EQUIPMENT_VARIANT_KEYS => Grammar::EquipmentVariant,
        TRAIT_TOOLTIP_KEYS => Grammar::TraitTooltip,
        POLICY_TOOLTIP_KEYS => Grammar::PolicyTooltip,
        PUPPET => Grammar::Puppet,
        CHARACTER_NAME => Grammar::CharacterName,
        CHARACTER_NATIONALITY => Grammar::CharacterNationality,
        SWAP_IDEAS => Grammar::SwapIdeas,
        DECISION_TOOLTIP => Grammar::DecisionTooltip,
        WHITE_PEACE => Grammar::WhitePeace,
        RANDOMIZE_KEYS => Grammar::Randomize,
        POPULARITY_KEYS => Grammar::Popularity,
        TEMPLATE_LOCK_KEYS | MODULO_KEYS | STRATEGIC_LOCATION => Grammar::One,
        RELEASE_COUNTRY_KEYS => Grammar::ReleaseCountry,
        AUTONOMY_KEYS => Grammar::Autonomy,
        FOCUS_TREE_KEYS => Grammar::FocusTree,
        SCORED_VARIABLE_KEYS => Grammar::ScoredVariable,
        SCORED_ARRAY_KEYS => Grammar::ScoredArray,
        SUPPLY_KEYS => Grammar::Supply,
        RANDOM_VARIABLE_KEYS => Grammar::RandomVariable,
        TARGET_VALUE_KEYS => Grammar::TargetValue,
        FACTION_COLOR_KEYS => Grammar::Color,
        OPERATIVE_EVENT_KEYS => Grammar::OperativeEvent,
        EVENT_KEYS => Grammar::Event,
        FLAG_KEYS | MODIFY_FLAG_KEYS => Grammar::Flag,
        BOUND_LOCALIZATION_READ => Grammar::Localization,
        SWAP_COUNTRY_TRAITS => Grammar::SwapCountryTraits,
        COUNTRY_TRAIT => Grammar::CountryTrait,
        SWAP_RULER_TRAITS => Grammar::SwapRulerTraits,
        CAPTURE_OPERATIVE_KEYS => Grammar::CaptureOperative,
        OPERATIVE_COUNTRY_KEYS if is_class("kill_operative") => Grammar::KillOperative,
        OPERATIVE_COUNTRY_KEYS if is_class("turn_operative") => Grammar::TurnOperative,
        CREATE_OPERATIVE_KEYS => Grammar::CreateOperative,
        CREATE_WARGOAL_KEYS => Grammar::Wargoal { expire: true },
        DECLARE_WAR_KEYS => Grammar::Wargoal { expire: false },
        REMOVE_WARGOAL_KEYS => Grammar::RemoveWargoal,
        EQUIPMENT_AMOUNT_KEYS => Grammar::EquipmentAmount,
        TRANSFER_LEADERS_KEYS => Grammar::TransferLeaders,
        RESISTANCE_KEYS => Grammar::Resistance,
        REMOVE_RESISTANCE_KEYS => Grammar::RemoveResistance,
        PROJECT_COMPLETION_KEYS => Grammar::ProjectCompletion,
        COMPLETE_FOCUS_KEYS => Grammar::CompleteFocus,
        AGENCY_KEYS => Grammar::Agency,
        PEACE_CONFERENCE => Grammar::PeaceConference,
        DOCTRINE_KEYS => Grammar::Doctrine,
        _ => return None,
    })
}

pub(super) fn is_own_field(value: u64, key: u64, effect: &[u8], field: &[u8]) -> Option<bool> {
    let address = match value_form(value) {
        Some(
            Form::CurrentOrKeys(source)
            | Form::ScalarOrKeys(source)
            | Form::StringOrKeys(source)
            | Form::Keys(source)
            | Form::AlwaysKeys(source)
            | Form::RoleKeys(source),
        ) => source.address(value, key),
        Some(Form::Persistent(address)) => address,
        Some(_) => return Some(false),
        None if value == super::READ_SCRIPT => key,
        None => return None,
    };
    if address == super::READ_EFFECT {
        return Some(false);
    }
    let reader = own_reader(address, |name| effect.eq_ignore_ascii_case(name.as_bytes()))?;
    Some(reader.value(field).is_some())
}

pub(super) fn description_fields(
    value: u64,
    key: u64,
    token: u32,
    item: &Item,
) -> Result<Vec<Item>, String> {
    let form = value_form(value).ok_or_else(|| {
        format!(
            "effect `{}` has no supported value projection",
            String::from_utf8_lossy(item.key.as_deref().unwrap_or_default())
        )
    })?;
    let items = item.value.items().unwrap_or_default();
    let source = match form {
        Form::CurrentOrKeys(source)
        | Form::ScalarOrKeys(source)
        | Form::StringOrKeys(source)
        | Form::Keys(source)
        | Form::AlwaysKeys(source)
        | Form::RoleKeys(source) => source,
        Form::Persistent(address) => return inline_description_fields(address, item, items),
        Form::One
        | Form::SkipIfOpen
        | Form::Current
        | Form::CurrentOrRaw
        | Form::List
        | Form::Ideas
        | Form::Localization => return Ok(Vec::new()),
    };
    match source {
        LoopSource::ClassKey if key == super::READ_EFFECT => Ok(items.to_vec()),
        LoopSource::ClassKey => inline_description_fields(key, item, items)
            .or_else(|_| super::key_readers::description_fields(token, items)),
        LoopSource::Inline => inline_description_fields(value, item, items),
    }
}

fn inline_description_fields(
    address: u64,
    class: &Item,
    items: &[Item],
) -> Result<Vec<Item>, String> {
    let reader = own_reader(address, |name| class.keyword_is(name)).ok_or_else(|| {
        format!(
            "effect `{}` has no supported field projection",
            String::from_utf8_lossy(class.key.as_deref().unwrap_or_default())
        )
    })?;
    Ok(items
        .iter()
        .filter(|item| {
            if reader.is_event() {
                item.keyword_is("random") && item.value.items().is_some()
            } else {
                reader.falls_back_to_effect() && reader.item_value(item).is_none()
            }
        })
        .cloned()
        .collect())
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
            reader: own_reader(address, |name| class.is_keyword(name))
                .expect("supported effect value field loop"),
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
                let reader = own_reader(address, |name| self.class.is_keyword(name))
                    .expect("supported nested value field loop");
                self.field(address, reader, field)
            }
            Node::IterationOutput => {
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
                            kind: content_reader_parser::DiagnosticKind::Unsupported,
                            report: "Deferred effect `promote_character.character`: parse scope unavailable".into(),
                            read: Box::new(Shape::Unsettled("promote_character.character: character scope skips the current block; other scopes read one value".into())),
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
            OwnReader::Wargoal { expire } => self.wargoal(expire, field),
            _ if reader.is_event() && field.keyword_is("random") && field.opens() => {
                super::shape(self.names, field).map(|()| unreachable!())
            }
            _ => {
                if let Some(value) = reader.token_value(field.key) {
                    self.value(value, field)
                } else if reader.falls_back_to_effect() {
                    super::shape(self.names, field).map(|()| unreachable!())
                } else {
                    Shape::Skip
                }
            }
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
                kind: content_reader_parser::DiagnosticKind::Misread,
                report: "invalid war goal: {".into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::InvalidBlock => Shape::One,
            OwnValue::RawSkipIfOpen if field.opens() => Shape::RawSkip,
            OwnValue::RawSkipIfOpen => Shape::One,
            OwnValue::ValueListRequired if field.opens() => Shape::ValueList,
            OwnValue::ValueListRequired => Shape::Reported {
                kind: content_reader_parser::DiagnosticKind::Misread,
                report: "Unexpected token: integer array requires a block".into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::RawListRequired if field.opens() => Shape::RawList,
            OwnValue::RawListRequired => Shape::Reported {
                kind: content_reader_parser::DiagnosticKind::Misread,
                report: "nationalities requires a block".into(),
                read: Box::new(Shape::Skip),
            },
            OwnValue::RawList => Shape::RawList,
            OwnValue::Trigger => Shape::Trigger,
            OwnValue::BoundLocalization if field.opens() => {
                Shape::Object(Node::Keys(BOUND_LOCALIZATION_READ))
            }
            OwnValue::BoundLocalization => Shape::One,
            OwnValue::Statements(node) => Shape::Statements(node),
            OwnValue::Gender | OwnValue::OperativeState => {
                let (token, report) = match value {
                    OwnValue::Gender => (field.current, "invalid gender"),
                    OwnValue::OperativeState => (field.value, "invalid operative state"),
                    _ => unreachable!(),
                };
                let accepted = token.lookup().is_some_and(|text| {
                    let key = text.to_ascii_lowercase();
                    match value {
                        OwnValue::Gender => {
                            matches!(key.as_slice(), b"female" | b"male" | b"undefined")
                        }
                        OwnValue::OperativeState => matches!(
                            key.as_slice(),
                            b"killed"
                                | b"on_mission"
                                | b"on_operation"
                                | b"on_capture"
                                | b"on_cooldown"
                                | b"on_disband"
                        ),
                        _ => unreachable!(),
                    }
                });
                if accepted {
                    Shape::One
                } else {
                    Shape::Reported {
                        kind: content_reader_parser::DiagnosticKind::Misread,
                        report: report.into(),
                        read: Box::new(Shape::Skip),
                    }
                }
            }
            OwnValue::Deferred => Shape::Reported {
                kind: content_reader_parser::DiagnosticKind::Unsupported,
                report: format!(
                    "Deferred effect `{}.{}`: inherited field grammar unavailable; by-key reading retained",
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
                    kind: content_reader_parser::DiagnosticKind::Misread,
                    report: "generator used twice".into(),
                    read: Box::new(Shape::Skip),
                };
            }
            self.generator_seen = true;
            if field.opens() {
                Shape::ValueList
            } else {
                Shape::One
            }
        } else if let Some(value) = (OwnReader::Wargoal { expire }).token_value(field.key) {
            self.value(value, field)
        } else {
            Shape::Skip
        }
    }

    fn randomize(&mut self, field: &Field<'_>) -> Shape<()> {
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
        use crate::sub_units::SubUnit;
        let Some(key) = field.key.lookup() else {
            return Shape::Skip;
        };
        let Some(catalog) = self.names.and_then(|names| names.sub_units.as_ref()) else {
            return Shape::Unsettled(format!(
                "division-template subunit `{}`: content subunit database at this load point is unavailable",
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
                if !field.skips_block() {
                    return Shape::One;
                }
                let widths = self.names.and_then(|names| names.template_columns.as_ref());
                let width = match widths {
                    Some(Ok(widths)) => widths[group.column_index()] as u32,
                    Some(Err(reason)) => return Shape::Unsettled(reason.clone()),
                    None => return Shape::Unsettled("add_units_to_division_template: recorded NMilitary column-width defines unavailable".into()),
                };
                let column = content_reader_parser::fixed5::decimal_i32(field.value.c_str()) as u32;
                if column >= width {
                    Shape::Skip
                } else {
                    Shape::One
                }
            }
        }
    }
}

fn template_point_component(index: usize) -> Option<&'static [u8]> {
    match index {
        0 => Some(b"x"),
        1 => Some(b"y"),
        2 => Some(b"z"),
        3 => Some(b"w"),
        _ => None,
    }
}

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
        error(reader, "division-template point: Expected block".into());
        return Value::Scalar(value);
    }
    let mut items = Vec::new();
    let mut component = 0usize;
    while !reader.is_unsettled() {
        let ends = reader.peek().ends_block();
        let token = take(reader);
        if ends || reader.is_unsettled() {
            break;
        }
        if token.is_number() {
            items.push(item(None, token));
            component += 1;
            continue;
        }
        let Some(expected) = template_point_component(component) else {
            reader.refuse_native(
                token.line,
                format!(
                    "division-template point for {}: nonnumeric component {} beyond the initialized x/y/z/w labels; component label is unavailable",
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
                    "division-template point: Expected component {}",
                    String::from_utf8_lossy(expected)
                ),
            );
            continue;
        }
        let equals = take(reader);
        if reader.is_unsettled() {
            break;
        }
        if equals.kind != Kind::Equals {
            error(
                reader,
                format!(
                    "division-template point: Expected '=' after {}",
                    String::from_utf8_lossy(expected)
                ),
            );
            continue;
        }
        let number = take(reader);
        if reader.is_unsettled() {
            break;
        }
        if !number.is_number() {
            error(
                reader,
                "division-template point: Expected a number after '='".into(),
            );
            continue;
        }
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

mod scope_reader_identity {
    pub(super) const TRAIT_KEYS: u64 = 0x140309B60;
    pub(super) const EVERY_CHARACTER_KEYS: u64 = 0x14030F870;
    pub(super) const RANDOM_CHARACTER_KEYS: u64 = 0x14030F9D0;
    pub(super) const EVERY_CONTRACT_KEYS: u64 = 0x1403205a0;
    pub(super) const RANDOM_CONTRACT_KEYS: u64 = 0x140320700;
    pub(super) const EQUIPMENT_SUBSIDY_KEYS: u64 = 0x140320820;
    pub(super) const CREATE_CONTRACT_KEYS: u64 = 0x1403208a0;
    pub(super) const EVERY_STATE_KEYS: u64 = 0x140333D90;
    pub(super) const DOCTRINE_COST_KEYS: u64 = 0x14033C050;
    pub(super) const TECH_BONUS_KEYS: u64 = 0x14033c620;
    pub(super) const STEAL_TECH_KEYS: u64 = 0x14033CAD0;
    pub(super) const ARRAY_EXTREMUM_KEYS: u64 = 0x1403C6670;
    pub(super) const EVERY_COUNTRY_KEYS: u64 = 0x1403C6920;
    pub(super) const RANDOM_STATE_KEYS: u64 = 0x1403C6A80;
    pub(super) const RANDOM_COUNTRY_KEYS: u64 = 0x1403C7260;
    pub(super) const ADD_ADVISOR_KEYS: u64 = 0x1403C76B0;
    pub(super) const ADD_LEADER_ROLE_KEYS: u64 = 0x1403C7830;
    pub(super) const INTEL_KEYS: u64 = 0x1403C79E0;
    pub(super) const RAILWAY_KEYS: u64 = 0x1403C95A0;
    pub(super) const DYNAMIC_COUNTRY_KEYS: u64 = 0x1403C9D60;
    pub(super) const DESTROY_SHIPS_KEYS: u64 = 0x1403CAF50;
    pub(super) const DIPLOMATIC_RELATION_KEYS: u64 = 0x1403CAFD0;
    pub(super) const EVERY_ORIGINAL_TAG_KEYS: u64 = 0x1403CB280;
    pub(super) const ARRAY_LOOP_KEYS: u64 = 0x1403CB4F0;
    pub(super) const SCOPE_LOOP_KEYS: u64 = 0x1403CB7B0;
    pub(super) const INDEX_LOOP_KEYS: u64 = 0x1403CBA70;
    pub(super) const COUNTRY_LEADER_KEYS: u64 = 0x1403CC020;
    pub(super) const RANDOM_ORIGINAL_TAG_KEYS: u64 = 0x1403CC5D0;
    pub(super) const ARRAY_SCOPE_KEYS: u64 = 0x1403CC710;
    pub(super) const REMOVE_ADVISOR_KEYS: u64 = 0x1403CD370;
    pub(super) const REMOVE_LEADER_ROLE_KEYS: u64 = 0x1403CD420;
    pub(super) const ADVISOR_DISMISSAL_KEYS: u64 = 0x1403CDAD0;
    pub(super) const CIVIL_WAR_KEYS: u64 = 0x1403CE890;
    pub(super) const TIMED_IDEA_KEYS: u64 = 0x1403CEB20;
    pub(super) const WHILE_KEYS: u64 = 0x1403CF5D0;
    pub(super) const EVERY_DIVISION_KEYS: u64 = 0x14047e4a0;
    pub(super) const RANDOM_DIVISION_KEYS: u64 = 0x14047e600;
    pub(super) const UNIT_TRAIT_KEYS: u64 = 0x14047e770;
    pub(super) const FACTION_GOAL_SLOT_KEYS: u64 = 0x1404D0D10;
    pub(super) const DAILY_MASTERY_KEYS: u64 = 0x140572230;
    pub(super) const MASTERY_BONUS_KEYS: u64 = 0x140572280;
    pub(super) const MASTERY_KEYS: u64 = 0x1405722d0;
    pub(super) const CONDITIONAL_KEYS: u64 = 0x141436c90;
    pub(super) const EVERY_LEADER_KEYS: u64 = 0x14143CF50;
    pub(super) const RANDOM_LEADER_KEYS: u64 = 0x14143D0D0;
    pub(super) const EVERY_ORGANISATION_KEYS: u64 = 0x14144C0A0;
    pub(super) const RANDOM_ORGANISATION_KEYS: u64 = 0x14144C220;
}

fn scope_policy(reader: u64) -> ScopePolicy {
    use scope_reader_identity::*;
    match reader {
        EVERY_CHARACTER_KEYS | RANDOM_CHARACTER_KEYS | EVERY_LEADER_KEYS | RANDOM_LEADER_KEYS => {
            ScopePolicy::Set(ScopeKind::Character.bit())
        }
        EVERY_CONTRACT_KEYS | RANDOM_CONTRACT_KEYS => {
            ScopePolicy::Set(ScopeKind::PurchaseContract.bit())
        }
        EQUIPMENT_SUBSIDY_KEYS
        | CREATE_CONTRACT_KEYS
        | EVERY_COUNTRY_KEYS
        | RANDOM_COUNTRY_KEYS
        | DYNAMIC_COUNTRY_KEYS
        | EVERY_ORIGINAL_TAG_KEYS
        | RANDOM_ORIGINAL_TAG_KEYS => ScopePolicy::Set(ScopeKind::Country.bit()),
        EVERY_STATE_KEYS | RANDOM_STATE_KEYS => ScopePolicy::Set(ScopeKind::State.bit()),
        EVERY_DIVISION_KEYS | RANDOM_DIVISION_KEYS => ScopePolicy::Set(ScopeKind::Unit.bit()),
        EVERY_ORGANISATION_KEYS | RANDOM_ORGANISATION_KEYS => {
            ScopePolicy::Set(ScopeKind::IndustrialOrganisation.bit())
        }
        SCOPE_LOOP_KEYS => ScopePolicy::Set(ScopeKind::all_bits()),
        TRAIT_KEYS
        | DOCTRINE_COST_KEYS
        | TECH_BONUS_KEYS
        | STEAL_TECH_KEYS
        | ARRAY_EXTREMUM_KEYS
        | ADD_ADVISOR_KEYS
        | ADD_LEADER_ROLE_KEYS
        | INTEL_KEYS
        | RAILWAY_KEYS
        | DESTROY_SHIPS_KEYS
        | DIPLOMATIC_RELATION_KEYS
        | ARRAY_LOOP_KEYS
        | INDEX_LOOP_KEYS
        | COUNTRY_LEADER_KEYS
        | ARRAY_SCOPE_KEYS
        | REMOVE_ADVISOR_KEYS
        | REMOVE_LEADER_ROLE_KEYS
        | ADVISOR_DISMISSAL_KEYS
        | CIVIL_WAR_KEYS
        | TIMED_IDEA_KEYS
        | WHILE_KEYS
        | UNIT_TRAIT_KEYS
        | FACTION_GOAL_SLOT_KEYS
        | DAILY_MASTERY_KEYS
        | MASTERY_BONUS_KEYS
        | MASTERY_KEYS
        | CONDITIONAL_KEYS
        | super::READ_EFFECT => ScopePolicy::Preserve,
        _ => ScopePolicy::Unknown,
    }
}

pub(crate) fn read_script_scope(token: u32, enclosing: Option<u32>) -> Option<u32> {
    let token = match token {
        ELSE_TOKEN | ELSE_IF_TOKEN => IF_TOKEN,
        _ => token,
    };
    let (_, key_reader) = super::class(token)?;
    match scope_policy(key_reader) {
        ScopePolicy::Set(scope) => Some(scope),
        ScopePolicy::Preserve => enclosing,
        ScopePolicy::Unknown => None,
    }
}

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

#[cfg(any(test, feature = "test-support"))]
impl crate::effect_keys::EffectShape {
    pub const DECLARED_WAR: Self = Self(SCRIPT_VALUE, DECLARE_WAR_KEYS);
    pub const CONFERENCE_TERMS: Self = Self(PEACE_CONFERENCE, 0);
    pub const PEACE_TERMS: Self = Self(WHITE_PEACE, 0);
    pub const END_RESISTANCE: Self = Self(RESISTANCE_VALUE, REMOVE_RESISTANCE_KEYS);
    pub const RESISTANCE_ACTIVITY: Self = Self(RESISTANCE_VALUE, REMOVE_RESISTANCE_KEYS);
    pub const RESISTANCE_TARGET: Self = Self(RESISTANCE_VALUE, RESISTANCE_KEYS);
    pub const FINISH_FOCUS: Self = Self(FOCUS_VALUE, COMPLETE_FOCUS_KEYS);
    pub const POPULARITY_SHIFT: Self = Self(ASSIGNMENT_VALUE, POPULARITY_KEYS);
    pub const FREE_GENERALS: Self = Self(ASSIGNMENT_VALUE, RELEASE_GENERAL_KEYS);
    pub const FOUND_AGENCY: Self = Self(AGENCY_VALUE, AGENCY_KEYS);
    pub const FOCUS_TREE: Self = Self(FOCUS_TREE_VALUE, FOCUS_TREE_KEYS);
    pub const ALLIANCE_TEMPLATE: Self = Self(ASSIGNMENT_VALUE, FACTION_COLOR_KEYS);
    pub const REVOKE_OBJECTIVE: Self = Self(SCRIPT_VALUE, REMOVE_WARGOAL_KEYS);
    pub const SUBJECT_LEVEL: Self = Self(SCRIPT_VALUE, AUTONOMY_KEYS);
    pub const SUBJECT_RELEASE: Self = Self(SCRIPT_VALUE, RELEASE_COUNTRY_KEYS);
    pub const MAKE_SUBJECT: Self = Self(PUPPET, 0);
    pub const WAR_OBJECTIVE: Self = Self(SCRIPT_VALUE, CREATE_WARGOAL_KEYS);
    pub const EQUIPMENT_DELIVERY: Self = Self(SCRIPT_VALUE, EQUIPMENT_AMOUNT_KEYS);
    pub const TEMPLATE_MEMBERS: Self = Self(SCRIPT_VALUE, ADD_TEMPLATE_UNITS_KEYS);
    pub const SEND_SHARE: Self = Self(SCRIPT_VALUE, EQUIPMENT_AMOUNT_KEYS);
    pub const TARGET_VALUE: Self = Self(SCRIPT_VALUE, TARGET_VALUE_KEYS);
    pub const MOVE_SHARE: Self = Self(SCRIPT_VALUE, TRANSFER_LEADERS_KEYS);
    pub const LOCK_COUNTRY_TEMPLATES: Self = Self(ASSIGNMENT_VALUE, TEMPLATE_LOCK_KEYS);
    pub const IDEA_EXCHANGE: Self = Self(SWAP_IDEAS, 0);
    pub const IDEA_DETAILS: Self = Self(IDEAS_TOOLTIP_VALUE, 0);
    pub const IDEAS: Self = Self(IDEAS_VALUE, 0);
    pub const PROJECT: Self = Self(ASSIGNMENT_VALUE, PROJECT_COMPLETION_KEYS);
    pub const DOCTRINE: Self = Self(FOCUS_VALUE, DOCTRINE_KEYS);
    pub const DECISION_DETAILS: Self = Self(DECISION_TOOLTIP, 0);
    pub const DECISION_CATEGORY: Self = Self(DECISION_CATEGORY_VALUE, 0);
    pub const CAPTURED_GENERAL: Self = Self(ASSIGNMENT_VALUE, CAPTURE_GENERAL_KEYS);
    pub const CHARACTER_NATIONALITY: Self = Self(CHARACTER_NATIONALITY, 0);
    pub const CHARACTER_PROMOTION: Self = Self(PROMOTION, 0);
    pub const COUNTRY_TRAIT: Self = Self(COUNTRY_TRAIT, 0);
    pub const EVENT: Self = Self(EVENT_VALUE, EVENT_KEYS);
    pub const HIGHEST_TEMP_COUNTRY: Self = Self(HIGHEST_TEMP_VALUE, SCORED_VARIABLE_KEYS);
    pub const LEADER_TOOLTIP: Self = Self(LEADERS_TOOLTIP_VALUE, 0);
    pub const LOCALISATION: Self = Self(LOCALIZATION_VALUE, 0);
    pub const OPERATIVE: Self = Self(CREATE_OPERATIVE_VALUE, CREATE_OPERATIVE_KEYS);
    pub const OPERATIVE_CAPTURE: Self = Self(OPERATIVE_VALUE, CAPTURE_OPERATIVE_KEYS);
    pub const OPERATIVE_EVENT: Self = Self(EVENT_VALUE, OPERATIVE_EVENT_KEYS);
    pub const ORGANISATION_POLICY: Self = Self(ASSIGNMENT_VALUE, POLICY_TOOLTIP_KEYS);
    pub const ORGANISATION_TRAIT: Self = Self(ASSIGNMENT_VALUE, TRAIT_TOOLTIP_KEYS);
    pub const REMOVE_COMMANDER_ROLE: Self = Self(REMOVE_ROLE, 0);
    pub const SCIENTIST_CHARACTER: Self = Self(ASSIGNMENT_VALUE, SCIENTIST_ROLE_KEYS);
    pub const SORTED_TEMP_COUNTRIES: Self = Self(SORTED_TEMP_VALUE, SCORED_ARRAY_KEYS);
    pub const TRAIT_CHOICE: Self = Self(RANDOM_TRAIT_VALUE, 0);
    pub const TRAIT_EXCHANGE: Self = Self(SWAP_RULER_TRAITS, 0);
    pub const UNIT_LEADER: Self = Self(REMOVE_UNIT_LEADER_VALUE, 0);
}

#[cfg(any(test, feature = "test-support"))]
pub(super) const SYNTHETIC_EFFECTS: &[(&str, u64, u64)] = &[
    ("show_ideas_tooltip", IDEAS_TOOLTIP_VALUE, 0),
    ("unlock_decision_category_tooltip", DECISION_CATEGORY_VALUE, 0),
    ("activate_mission", MISSION, 0),
    ("remove_mission", MISSION, 0),
    ("activate_mission_tooltip", MISSION_TOOLTIP, 0),
    ("retire_character", RETIRE_CHARACTER_VALUE, 0),
    ("show_unit_leaders_tooltip", LEADERS_TOOLTIP_VALUE, 0),
    ("activate_decision", DECISION_VALUE, 0),
    ("remove_decision", DECISION_VALUE, 0),
    ("activate_advisor", ADVISOR_VALUE, 0),
    ("deactivate_advisor", ADVISOR_VALUE, 0),
    ("remove_unit_leader", REMOVE_UNIT_LEADER_VALUE, 0),
    ("recruit_character", RECRUIT_CHARACTER_VALUE, 0),
    ("injure_scientist_for_days", INJURE_SCIENTIST_VALUE, 0),
    ("add_scientist_trait", SCIENTIST_TRAIT_VALUE, 0),
    ("set_oob", OOB_VALUE, 0),
    ("set_naval_oob", OOB_VALUE, 0),
    ("set_air_oob", OOB_VALUE, 0),
    ("capture_by", ASSIGNMENT_VALUE, CAPTURE_GENERAL_KEYS),
    ("release_captured_generals_from", ASSIGNMENT_VALUE, RELEASE_GENERAL_KEYS),
    ("remove_scientist_role", ASSIGNMENT_VALUE, SCIENTIST_ROLE_KEYS),
    ("custom_effect_tooltip", LOCALIZATION_VALUE, 0),
    ("swap_ideas", SWAP_IDEAS, 0),
    ("swap_ruler_traits", SWAP_RULER_TRAITS, 0),
    ("country_lock_all_division_template", ASSIGNMENT_VALUE, TEMPLATE_LOCK_KEYS),
    ("complete_special_project", SCRIPT_VALUE, PROJECT_COMPLETION_KEYS),
    ("complete_national_focus", FOCUS_VALUE, COMPLETE_FOCUS_KEYS),
    ("set_sub_doctrine", FOCUS_VALUE, DOCTRINE_KEYS),
    ("create_intelligence_agency", AGENCY_VALUE, AGENCY_KEYS),
    ("start_peace_conference", PEACE_CONFERENCE, 0),
    ("white_peace", WHITE_PEACE, 0),
    ("puppet", PUPPET, 0),
    ("set_character_name", CHARACTER_NAME, 0),
    ("set_nationality", CHARACTER_NATIONALITY, 0),
    ("unlock_decision_tooltip", DECISION_TOOLTIP, 0),
    ("add_random_trait", RANDOM_TRAIT_VALUE, 0),
    ("create_operative_leader", CREATE_OPERATIVE_VALUE, CREATE_OPERATIVE_KEYS),
    ("promote_character", PROMOTION, 0),
    ("remove_unit_leader_role", REMOVE_ROLE, 0),
    ("transfer_units_fraction", SCRIPT_VALUE, TRANSFER_LEADERS_KEYS),
    ("send_equipment", SCRIPT_VALUE, EQUIPMENT_AMOUNT_KEYS),
    ("send_equipment_fraction", SCRIPT_VALUE, EQUIPMENT_AMOUNT_KEYS),
    ("create_wargoal", SCRIPT_VALUE, CREATE_WARGOAL_KEYS),
    ("declare_war_on", SCRIPT_VALUE, DECLARE_WAR_KEYS),
    ("remove_wargoal", SCRIPT_VALUE, REMOVE_WARGOAL_KEYS),
    ("release_autonomy", SCRIPT_VALUE, RELEASE_COUNTRY_KEYS),
    ("set_autonomy", SCRIPT_VALUE, AUTONOMY_KEYS),
    ("load_focus_tree", FOCUS_TREE_VALUE, FOCUS_TREE_KEYS),
    ("country_event", EVENT_VALUE, EVENT_KEYS),
    ("operative_leader_event", EVENT_VALUE, OPERATIVE_EVENT_KEYS),
    ("set_country_flag", FLAG_VALUE, FLAG_KEYS),
    ("modify_country_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("add_popularity", ASSIGNMENT_VALUE, POPULARITY_KEYS),
    ("get_highest_scored_country", HIGHEST_VALUE, SCORED_VARIABLE_KEYS),
    ("get_highest_scored_country_temp", HIGHEST_TEMP_VALUE, SCORED_VARIABLE_KEYS),
    ("get_sorted_scored_countries", SORTED_VALUE, SCORED_ARRAY_KEYS),
    ("get_sorted_scored_countries_temp", SORTED_TEMP_VALUE, SCORED_ARRAY_KEYS),
    ("get_supply_vehicles_temp", SUPPLY_VALUE, SUPPLY_KEYS),
    ("modulo_variable", MODULO_VALUE, MODULO_KEYS),
    ("modulo_temp_variable", MODULO_TEMP_VALUE, MODULO_KEYS),
    ("set_variable_to_random", RANDOM_VARIABLE_VALUE, RANDOM_VARIABLE_KEYS),
    ("set_temp_variable_to_random", RANDOM_TEMP_VALUE, RANDOM_VARIABLE_KEYS),
    ("add_ideas", IDEAS_VALUE, 0),
    ("add_country_leader_trait", COUNTRY_TRAIT, 0),
    ("force_disable_resistance", RESISTANCE_VALUE, REMOVE_RESISTANCE_KEYS),
    ("division_template", DIVISION_TEMPLATE_VALUE, 0),
    ("news_event", EVENT_VALUE, EVENT_KEYS),
    ("state_event", EVENT_VALUE, EVENT_KEYS),
    ("unit_leader_event", EVENT_VALUE, EVENT_KEYS),
    ("set_global_flag", FLAG_VALUE, FLAG_KEYS),
    ("set_state_flag", FLAG_VALUE, FLAG_KEYS),
    ("set_character_flag", FLAG_VALUE, FLAG_KEYS),
    ("set_unit_leader_flag", FLAG_VALUE, FLAG_KEYS),
    ("set_mio_flag", FLAG_VALUE, FLAG_KEYS),
    ("set_project_flag", FLAG_VALUE, FLAG_KEYS),
    ("modify_global_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("modify_state_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("modify_character_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("modify_unit_leader_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("modify_mio_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("modify_project_flag", MODIFY_FLAG_VALUE, MODIFY_FLAG_KEYS),
    ("remove_ideas", IDEAS_VALUE, 0),
    ("randomize_temp_variable", RANDOMIZE_VALUE, RANDOMIZE_KEYS),
    ("randomize_variable", RANDOMIZE_VALUE, RANDOMIZE_KEYS),
    ("complete_mio_trait", ASSIGNMENT_VALUE, TRAIT_TOOLTIP_KEYS),
    ("unlock_mio_trait_tooltip", ASSIGNMENT_VALUE, TRAIT_TOOLTIP_KEYS),
    ("unlock_mio_policy_tooltip", ASSIGNMENT_VALUE, POLICY_TOOLTIP_KEYS),
    ("create_faction_from_template", ASSIGNMENT_VALUE, FACTION_COLOR_KEYS),
    ("remove_country_leader_trait", COUNTRY_TRAIT, 0),
    ("add_units_to_division_template", SCRIPT_VALUE, ADD_TEMPLATE_UNITS_KEYS),
    ("create_colonial_division_template", SCRIPT_VALUE, COLONIAL_TEMPLATE_KEYS),
    ("capture_operative", OPERATIVE_VALUE, CAPTURE_OPERATIVE_KEYS),
    ("kill_operative", OPERATIVE_VALUE, OPERATIVE_COUNTRY_KEYS),
    ("turn_operative", OPERATIVE_VALUE, OPERATIVE_COUNTRY_KEYS),
    ("add_resistance_target", RESISTANCE_VALUE, RESISTANCE_KEYS),
    ("force_enable_resistance", RESISTANCE_VALUE, REMOVE_RESISTANCE_KEYS),
    ("swap_country_leader_traits", SWAP_COUNTRY_TRAITS, 0),
];

#[cfg(test)]
mod tests;
