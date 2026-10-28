//! Effect values and their nested field grammars.

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

// Reader identities select grammar; authored data still comes from the content stack.
mod reader_identity {
    pub(super) const RANDOM_TRAIT_VALUE: u64 = 0x140308FB0;
    pub(super) const REMOVE_UNIT_LEADER_VALUE: u64 = 0x140308FD0;
    pub(super) const SWAP_COUNTRY_TRAITS: u64 = 0x1403092A0;
    pub(super) const CAPTURE_GENERAL_KEYS: u64 = 0x1403110D0;
    pub(super) const RELEASE_GENERAL_KEYS: u64 = 0x140311110;
    pub(super) const TRAIT_TOOLTIP_KEYS: u64 = 0x140319EB0;
    pub(super) const POLICY_TOOLTIP_KEYS: u64 = 0x140319EC0;
    pub(super) const RESISTANCE_VALUE: u64 = 0x140333BA0;
    pub(super) const RESISTANCE_KEYS: u64 = 0x140333EB0;
    pub(super) const REMOVE_RESISTANCE_KEYS: u64 = 0x140334590;
    pub(super) const RANDOMIZE_VALUE: u64 = 0x1403C1580;
    pub(super) const ADVISOR_VALUE: u64 = 0x1403C1D90;
    pub(super) const MISSION_TOOLTIP: u64 = 0x1403C1E50;
    pub(super) const COUNTRY_TRAIT: u64 = 0x1403C20B0;
    pub(super) const DIVISION_TEMPLATE_VALUE: u64 = 0x1403C2360;
    pub(super) const IDEAS_VALUE: u64 = 0x1403C23B0;
    pub(super) const SCRIPT_VALUE: u64 = 0x1403C26E0;
    pub(super) const FOCUS_VALUE: u64 = 0x1403C26F0;
    pub(super) const AGENCY_VALUE: u64 = 0x1403C2740;
    pub(super) const CREATE_OPERATIVE_VALUE: u64 = 0x1403C2750;
    pub(super) const DECISION_VALUE: u64 = 0x1403C27C0;
    pub(super) const PUPPET: u64 = 0x1403C2810;
    pub(super) const MISSION: u64 = 0x1403C2A60;
    pub(super) const OPERATIVE_VALUE: u64 = 0x1403C2CA0;
    pub(super) const PROMOTION: u64 = 0x1403C2F60;
    pub(super) const RECRUIT_CHARACTER_VALUE: u64 = 0x1403C35F0;
    pub(super) const REMOVE_ROLE: u64 = 0x1403C37A0;
    pub(super) const RETIRE_CHARACTER_VALUE: u64 = 0x1403C3B40;
    pub(super) const OOB_VALUE: u64 = 0x1403C3BE0;
    pub(super) const FOCUS_TREE_VALUE: u64 = 0x1403C3C20;
    pub(super) const CHARACTER_NAME: u64 = 0x1403C3E60;
    pub(super) const CHARACTER_NATIONALITY: u64 = 0x1403C4130;
    pub(super) const IDEAS_TOOLTIP_VALUE: u64 = 0x1403C44A0;
    pub(super) const LEADERS_TOOLTIP_VALUE: u64 = 0x1403C48E0;
    pub(super) const PEACE_CONFERENCE: u64 = 0x1403C4990;
    pub(super) const SWAP_IDEAS: u64 = 0x1403C4C70;
    pub(super) const SWAP_RULER_TRAITS: u64 = 0x1403C51F0;
    pub(super) const DECISION_CATEGORY_VALUE: u64 = 0x1403C5820;
    pub(super) const DECISION_TOOLTIP: u64 = 0x1403C5870;
    pub(super) const WHITE_PEACE: u64 = 0x1403C5AE0;
    pub(super) const RANDOMIZE_KEYS: u64 = 0x1403C6A30;
    pub(super) const POPULARITY_KEYS: u64 = 0x1403C7BE0;
    pub(super) const ADD_TEMPLATE_UNITS_KEYS: u64 = 0x1403C8270;
    pub(super) const CAPTURE_OPERATIVE_KEYS: u64 = 0x1403C96F0;
    pub(super) const COMPLETE_FOCUS_KEYS: u64 = 0x1403C9C10;
    pub(super) const COLONIAL_TEMPLATE_KEYS: u64 = 0x1403C9C60;
    pub(super) const AGENCY_KEYS: u64 = 0x1403CA050;
    pub(super) const CREATE_OPERATIVE_KEYS: u64 = 0x1403CA090;
    pub(super) const CREATE_WARGOAL_KEYS: u64 = 0x1403CA830;
    pub(super) const DECLARE_WAR_KEYS: u64 = 0x1403CABB0;
    pub(super) const TEMPLATE_LOCK_KEYS: u64 = 0x1403CC0A0;
    pub(super) const OPERATIVE_COUNTRY_KEYS: u64 = 0x1403CC470;
    pub(super) const RELEASE_COUNTRY_KEYS: u64 = 0x1403CD060;
    pub(super) const SCIENTIST_ROLE_KEYS: u64 = 0x1403CD5A0;
    pub(super) const REMOVE_WARGOAL_KEYS: u64 = 0x1403CD6E0;
    pub(super) const AUTONOMY_KEYS: u64 = 0x1403CD8A0;
    pub(super) const FOCUS_TREE_KEYS: u64 = 0x1403CDB80;
    pub(super) const TRANSFER_LEADERS_KEYS: u64 = 0x1403CEE80;
    pub(super) const HIGHEST_TEMP_VALUE: u64 = 0x14049C680;
    pub(super) const HIGHEST_VALUE: u64 = 0x14049CAD0;
    pub(super) const SORTED_TEMP_VALUE: u64 = 0x14049CF20;
    pub(super) const SORTED_VALUE: u64 = 0x14049D370;
    pub(super) const SUPPLY_VALUE: u64 = 0x14049D7C0;
    pub(super) const MODULO_TEMP_VALUE: u64 = 0x14049E060;
    pub(super) const MODULO_VALUE: u64 = 0x14049E4B0;
    pub(super) const RANDOM_TEMP_VALUE: u64 = 0x14049E900;
    pub(super) const RANDOM_VARIABLE_VALUE: u64 = 0x14049ed80;
    pub(super) const SCORED_VARIABLE_KEYS: u64 = 0x1404A08E0;
    pub(super) const SCORED_ARRAY_KEYS: u64 = 0x1404A0920;
    pub(super) const SUPPLY_KEYS: u64 = 0x1404A0970;
    pub(super) const MODULO_KEYS: u64 = 0x1404A09C0;
    pub(super) const RANDOM_VARIABLE_KEYS: u64 = 0x1404A09D0;
    pub(super) const SCIENTIST_TRAIT_VALUE: u64 = 0x1404B5840;
    pub(super) const INJURE_SCIENTIST_VALUE: u64 = 0x1404B5AF0;
    pub(super) const PROJECT_COMPLETION_KEYS: u64 = 0x1404b5f50;
    pub(super) const EQUIPMENT_AMOUNT_KEYS: u64 = 0x1404BE080;
    pub(super) const TARGET_VALUE_KEYS: u64 = 0x1404BE250;
    pub(super) const FACTION_COLOR_KEYS: u64 = 0x1404D0CF0;
    pub(super) const STRATEGIC_LOCATION: u64 = 0x14056F210;
    pub(super) const DOCTRINE_KEYS: u64 = 0x140572330;
    pub(super) const DIVISION_TEMPLATE_KEYS: u64 = 0x140BF5CF0;
    pub(super) const LOCALIZATION_VALUE: u64 = 0x141435C10;
    pub(super) const EVENT_VALUE: u64 = 0x141435C20;
    pub(super) const FLAG_VALUE: u64 = 0x141435C40;
    pub(super) const MODIFY_FLAG_VALUE: u64 = 0x141435F20;
    pub(super) const ASSIGNMENT_VALUE: u64 = 0x141436430;
    pub(super) const OPERATIVE_EVENT_KEYS: u64 = 0x141436450;
    pub(super) const EVENT_KEYS: u64 = 0x1414366A0;
    pub(super) const FLAG_KEYS: u64 = 0x1414366D0;
    pub(super) const MODIFY_FLAG_KEYS: u64 = 0x141436B40;
    pub(super) const EQUIPMENT_VARIANT_KEYS: u64 = 0x1414C4420;
    pub(super) const BOUND_LOCALIZATION_READ: u64 = 0x1424260C0;
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
            // This grammar enters its field loop even after a scalar or stale value.
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
            // Raw lists resolve tokens without replacing the retained value slot.
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
                        kind: crate::DiagnosticKind::Misread,
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
            kind: crate::DiagnosticKind::Misread,
            report: format!(
                "effect `{}` requires a block",
                String::from_utf8_lossy(field.key.c_str())
            ),
            read: Box::new(Shape::Skip),
        }),
    }
}

fn string_scalar(field: &Field<'_>) -> ScalarRead {
    // Bare names can already be dynamically interned by the content stack.
    // The executable vocabulary cannot settle that distinction or stale input.
    if !field.fresh {
        ScalarRead::Deferred
    } else if field.value.kind == Kind::Quoted {
        ScalarRead::One
    } else if field.value.kind == Kind::Bare
        && !field.value.is_number()
        && engine_executable::executable_tables::get()
            .keywords
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
            // Traits enter the raw loop even after a scalar; flags skip only an opening block.
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

/// Project nested effects using the same field grammar as parsing.
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
                // Anonymous blocks skip; assignments retain one token, including an empty key.
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
                // Gender reads the current token; operative state reads the retained value.
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
                        kind: crate::DiagnosticKind::Misread,
                        report: report.into(),
                        read: Box::new(Shape::Skip),
                    }
                }
            }
            OwnValue::Deferred => Shape::Reported {
                kind: crate::DiagnosticKind::Unsupported,
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
                    kind: crate::DiagnosticKind::Misread,
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
        } else if field.keyword_is("type") {
            self.value(OwnValue::InvalidBlock, field)
        } else if field.keyword_is("target") || (expire && field.keyword_is("expire")) {
            Shape::One
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
        use crate::unit_leader::SubUnit;
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
                // Missing widths affect consumption only when rejection could skip a block.
                if !field.skips_block() {
                    return Shape::One;
                }
                let widths = self.names.and_then(|names| names.template_columns.as_ref());
                let width = match widths {
                    Some(Ok(widths)) => widths[group.column_index()] as u32,
                    Some(Err(reason)) => return Shape::Unsettled(reason.clone()),
                    None => return Shape::Unsettled("add_units_to_division_template: recorded NMilitary column-width defines unavailable".into()),
                };
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
    loop {
        // Test the peek before resolving it, since a resolved closing token does not stop here.
        let ends = reader.peek().ends_block();
        let token = take(reader);
        if ends {
            break;
        }
        if token.is_number() {
            // Keep lexical values for the point consumer's conversion.
            items.push(item(None, token));
            component += 1;
            continue;
        }
        let Some(expected) = template_point_component(component) else {
            // Extra numeric components still consume; an unavailable named label refuses.
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
    pub(super) const TRAIT_KEYS: u64 = 0x140309AF0;
    pub(super) const EVERY_CHARACTER_KEYS: u64 = 0x14030F800;
    pub(super) const RANDOM_CHARACTER_KEYS: u64 = 0x14030F960;
    pub(super) const EVERY_CONTRACT_KEYS: u64 = 0x140320530;
    pub(super) const RANDOM_CONTRACT_KEYS: u64 = 0x140320690;
    pub(super) const EQUIPMENT_SUBSIDY_KEYS: u64 = 0x1403207b0;
    pub(super) const CREATE_CONTRACT_KEYS: u64 = 0x140320830;
    pub(super) const EVERY_STATE_KEYS: u64 = 0x140333D20;
    pub(super) const DOCTRINE_COST_KEYS: u64 = 0x14033BFE0;
    pub(super) const TECH_BONUS_KEYS: u64 = 0x14033c5b0;
    pub(super) const STEAL_TECH_KEYS: u64 = 0x14033CA60;
    pub(super) const ARRAY_EXTREMUM_KEYS: u64 = 0x1403C6600;
    pub(super) const EVERY_COUNTRY_KEYS: u64 = 0x1403C68B0;
    pub(super) const RANDOM_STATE_KEYS: u64 = 0x1403C6A10;
    pub(super) const RANDOM_COUNTRY_KEYS: u64 = 0x1403C71F0;
    pub(super) const ADD_ADVISOR_KEYS: u64 = 0x1403C7640;
    pub(super) const ADD_LEADER_ROLE_KEYS: u64 = 0x1403C77C0;
    pub(super) const INTEL_KEYS: u64 = 0x1403C7970;
    pub(super) const RAILWAY_KEYS: u64 = 0x1403C9530;
    pub(super) const DYNAMIC_COUNTRY_KEYS: u64 = 0x1403C9CF0;
    pub(super) const DESTROY_SHIPS_KEYS: u64 = 0x1403CAEE0;
    pub(super) const DIPLOMATIC_RELATION_KEYS: u64 = 0x1403CAF60;
    pub(super) const EVERY_ORIGINAL_TAG_KEYS: u64 = 0x1403CB210;
    pub(super) const ARRAY_LOOP_KEYS: u64 = 0x1403CB480;
    pub(super) const SCOPE_LOOP_KEYS: u64 = 0x1403CB740;
    pub(super) const INDEX_LOOP_KEYS: u64 = 0x1403CBA00;
    pub(super) const COUNTRY_LEADER_KEYS: u64 = 0x1403CBFB0;
    pub(super) const RANDOM_ORIGINAL_TAG_KEYS: u64 = 0x1403CC560;
    pub(super) const ARRAY_SCOPE_KEYS: u64 = 0x1403CC6A0;
    pub(super) const REMOVE_ADVISOR_KEYS: u64 = 0x1403CD300;
    pub(super) const REMOVE_LEADER_ROLE_KEYS: u64 = 0x1403CD3B0;
    pub(super) const ADVISOR_DISMISSAL_KEYS: u64 = 0x1403CDA60;
    pub(super) const CIVIL_WAR_KEYS: u64 = 0x1403CE820;
    pub(super) const TIMED_IDEA_KEYS: u64 = 0x1403CEAB0;
    pub(super) const WHILE_KEYS: u64 = 0x1403CF560;
    pub(super) const EVERY_DIVISION_KEYS: u64 = 0x14047e430;
    pub(super) const RANDOM_DIVISION_KEYS: u64 = 0x14047e590;
    pub(super) const UNIT_TRAIT_KEYS: u64 = 0x14047e700;
    pub(super) const FACTION_GOAL_SLOT_KEYS: u64 = 0x1404D0CA0;
    pub(super) const DAILY_MASTERY_KEYS: u64 = 0x1405721C0;
    pub(super) const MASTERY_BONUS_KEYS: u64 = 0x140572210;
    pub(super) const MASTERY_KEYS: u64 = 0x140572260;
    pub(super) const CONDITIONAL_KEYS: u64 = 0x141436860;
    pub(super) const EVERY_LEADER_KEYS: u64 = 0x14143CB20;
    pub(super) const RANDOM_LEADER_KEYS: u64 = 0x14143CCA0;
    pub(super) const EVERY_ORGANISATION_KEYS: u64 = 0x14144BC70;
    pub(super) const RANDOM_ORGANISATION_KEYS: u64 = 0x14144BDF0;
}

// Only grammars that forward effects supply a known body scope.
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

/// Scope targets replace context only after effect registry lookup fails.
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
