//! ReadScript's slot-4 readers. Static own-key lists and their fallback
//! come from captures/astra/effect-key-readers/port_data.py, using hoi4re
//! dumps from extract.py and the factory/vtable inventory in
//! captures/astra/key-validity/classes.py. Addresses accompany the rows.

use super::{READ_EFFECT, READ_SCRIPT};

/// The `else` and `else_if` keyword tokens ReadEffect (1405345D0) tests
/// before the registry; both read with CIfEffect's key reader.
const ELSE_TOKEN: u32 = 0x36D3;
const ELSE_IF_TOKEN: u32 = 0x38ED;
#[cfg(test)]
mod tests;
use crate::{
    combat_tactics,
    key_names::{self, KeyNames, Load},
    parser::{
        Operator, c_string,
        native::{Field, Shape},
    },
};

#[derive(Clone, Copy)]
enum ValueReader {
    One,
    ValueList,
    Localization,
    Limit,
    Province,
}

#[derive(Clone, Copy)]
enum Fallback {
    /// CEffect::ReadEffect (1405345D0).
    Effect,
    /// ReportUnexpectedToken (1424AD100 -> 1424ACF90), with its skip.
    Only,
    /// Dynamic name assignment or a report without a skip (1424ACD40).
    One,
}

#[derive(Clone, Copy)]
enum KeyReader {
    Keys(&'static [(&'static str, ValueReader)], Fallback),
    Variable,
    DivideVariable,
    Conditional,
}

// BEGIN EXTRACTED TABLE
const READER_1403ADCF0: KeyReader = KeyReader::Keys(
    &[
        ("tooltip", ValueReader::Localization),
        ("limit", ValueReader::Limit),
        ("visible_when_empty", ValueReader::One),
        ("display_individual_scopes", ValueReader::One),
        ("random_select_amount", ValueReader::One),
    ],
    Fallback::Effect,
);
const READER_1403AEEE0: KeyReader = KeyReader::Keys(
    &[
        ("target", ValueReader::One),
        ("modifier", ValueReader::One),
        ("days", ValueReader::One),
    ],
    Fallback::Only,
);
const READER_140323D80: KeyReader = KeyReader::Keys(&[("popup", ValueReader::One)], Fallback::One);
const READER_14031D4D0: KeyReader = KeyReader::Keys(
    &[
        ("type", ValueReader::One),
        ("province", ValueReader::Province),
        ("level", ValueReader::One),
        ("instant_build", ValueReader::One),
    ],
    Fallback::Only,
);
const READER_1403ADE50: KeyReader = KeyReader::Keys(
    &[
        ("tooltip", ValueReader::Localization),
        ("limit", ValueReader::Limit),
        ("prioritize", ValueReader::ValueList),
        ("visible_when_empty", ValueReader::One),
    ],
    Fallback::Effect,
);
const READER_140323AC0: KeyReader = KeyReader::Keys(
    &[
        ("name", ValueReader::One),
        ("category", ValueReader::One),
        ("technology", ValueReader::One),
        ("bonus", ValueReader::One),
        ("ahead_reduction", ValueReader::One),
        ("uses", ValueReader::One),
    ],
    Fallback::Effect,
);
const READER_1403B5EB0: KeyReader = KeyReader::Keys(
    &[
        ("idea", ValueReader::One),
        ("months", ValueReader::One),
        ("days", ValueReader::One),
        ("years", ValueReader::One),
    ],
    Fallback::Effect,
);
const READER_1403B5280: KeyReader = KeyReader::Keys(
    &[
        ("name", ValueReader::One),
        ("ideology", ValueReader::One),
        ("long_name", ValueReader::One),
    ],
    Fallback::Only,
);
const READER_14049E3D0: KeyReader = KeyReader::Keys(
    &[
        ("type", ValueReader::One),
        ("amount", ValueReader::One),
        ("producer", ValueReader::One),
        ("variant_name", ValueReader::One),
    ],
    Fallback::Only,
);
const READER_1403B0B70: KeyReader = KeyReader::Keys(
    &[
        ("min", ValueReader::One),
        ("max", ValueReader::One),
        ("var", ValueReader::One),
    ],
    Fallback::One,
);
const READER_1403B04D0: KeyReader = KeyReader::Keys(
    &[
        ("size", ValueReader::One),
        ("index", ValueReader::One),
        ("value", ValueReader::One),
        ("array", ValueReader::One),
    ],
    Fallback::One,
);
const READER_1403AD9F0: KeyReader = KeyReader::Keys(
    &[
        ("modifier", ValueReader::One),
        ("days", ValueReader::One),
        ("scope", ValueReader::One),
    ],
    Fallback::Only,
);
const KEY_READERS: &[(u32, KeyReader)] = &[
    (0x275C, READER_1403ADCF0),              // every_country, 1403ADCF0
    (0x2769, READER_1403ADCF0),              // every_neighbor_country, 1403ADCF0
    (0x277C, READER_1403ADCF0),              // every_enemy_country, 1403ADCF0
    (0x27B3, KeyReader::Variable),           // set_variable, 1403B6640
    (0x27C3, KeyReader::Conditional),        // if, 141385BA0
    (0x27C8, READER_1403AEEE0),              // add_opinion_modifier, 1403AEEE0
    (0x28DB, READER_1403ADCF0),              // every_subject_country, 1403ADCF0
    (0x292A, READER_1403ADCF0),              // every_faction_member, 1403ADCF0
    (0x3028, READER_140323D80),              // set_technology, 140323D80
    (0x319F, READER_14031D4D0),              // add_building_construction, 14031D4D0
    (0x31C9, READER_1403ADE50),              // random_state, 1403ADE50
    (0x31CA, READER_1403ADE50),              // random_owned_state, 1403ADE50
    (0x31CB, READER_1403ADE50),              // random_neighbor_state, 1403ADE50
    (0x31CD, READER_1403ADCF0),              // every_state, 14031B220
    (0x31CE, READER_1403ADCF0),              // every_owned_state, 14031B220
    (0x31CF, READER_1403ADCF0),              // every_neighbor_state, 14031B220
    (0x33E2, READER_140323AC0),              // add_tech_bonus, 140323AC0
    (0x3462, READER_1403B5EB0),              // add_timed_idea, 1403B5EB0
    (0x34EA, READER_1403B5280),              // set_party_name, 1403B5280
    (0x357F, READER_1403AEEE0),              // reverse_add_opinion_modifier, 1403AEEE0
    (0x360C, READER_1403ADE50),              // random_owned_controlled_state, 1403ADE50
    (0x3678, READER_1403ADCF0),              // every_other_country, 1403ADCF0
    (0x36C2, READER_14049E3D0),              // add_equipment_to_stockpile, 14049E3D0
    (ELSE_TOKEN, KeyReader::Conditional),    // else, 141385BA0
    (0x37DB, READER_14031D4D0),              // set_building_level, 14031D4D0
    (ELSE_IF_TOKEN, KeyReader::Conditional), // else_if, 141385BA0
    (0x38F8, KeyReader::Variable),           // add_to_variable, 1403B6640
    (0x38F9, KeyReader::Variable),           // subtract_from_variable, 1403B6640
    (0x38FA, KeyReader::Variable),           // multiply_variable, 1403B6640
    (0x38FB, KeyReader::Variable),           // divide_variable, 1403B6640
    (0x391A, READER_14031D4D0),              // add_offsite_building, 14031D4D0
    (0x3977, KeyReader::Variable),           // add_to_temp_variable, 1403B6640
    (0x3978, KeyReader::Variable),           // subtract_from_temp_variable, 1403B6640
    (0x3979, KeyReader::Variable),           // multiply_temp_variable, 1403B6640
    (0x397A, KeyReader::DivideVariable),     // divide_temp_variable, 1403B2580
    (0x397B, KeyReader::Variable),           // set_temp_variable, 1403B6640
    (0x397E, READER_1403B0B70),              // clamp_variable, 1403B0DD0
    (0x397F, READER_1403B0B70),              // clamp_temp_variable, 1403B0B70
    (0x39D7, READER_1403B5EB0),              // modify_timed_idea, 1403B5EB0
    (0x3BA1, READER_1403B04D0),              // add_to_array, 1403B04D0
    (0x3BA2, READER_1403B04D0),              // remove_from_array, 1403B04D0
    (0x3BA7, READER_1403B04D0),              // add_to_temp_array, 1403B04D0
    (0x3BA8, READER_1403B04D0),              // remove_from_temp_array, 1403B04D0
    (0x3C02, READER_1403AD9F0),              // remove_dynamic_modifier, 1403AD9F0
    (0x3C03, READER_1403AD9F0),              // add_dynamic_modifier, 1403AD9F0
    (0x4094, READER_1403ADCF0),              // every_allied_country, 1403ADCF0
    (0x4A61, READER_1403ADCF0),              // every_controlled_state, 14031B220
    (0x4A8E, READER_1403ADCF0),              // every_occupied_country, 1403ADCF0
    (0x4B5F, READER_1403ADE50),              // random_controlled_state, 1403ADE50
    (0x4DF8, READER_1403ADCF0),              // every_core_state, 14031B220
    (0x4DF9, READER_1403ADE50),              // random_core_state, 1403ADE50
    (0x4E0C, READER_1403ADCF0),              // every_possible_country, 1403ADCF0
];
// END EXTRACTED TABLE

fn reader(token: u32) -> Option<KeyReader> {
    KEY_READERS
        .binary_search_by_key(&token, |(id, _)| *id)
        .ok()
        .map(|index| KEY_READERS[index].1)
}

pub(super) fn shape(address: u64, field: &Field<'_>) -> Shape<()> {
    if address == READ_EFFECT {
        return Shape::Effect;
    }
    let keyword = field.keyword().unwrap_or_default();
    let token = match keyword.as_slice() {
        b"else" => Some(ELSE_TOKEN),
        b"else_if" => Some(ELSE_IF_TOKEN),
        _ => super::registered(&keyword),
    };
    if let Some(token) = token
        && reader(token).is_some()
    {
        return Shape::EffectKeys(token);
    }
    Shape::Reported {
        report: format!(
            "Deferred effect `{}`: value reader {READ_SCRIPT:#X}, key reader {address:#X}; by-key reading retained",
            String::from_utf8_lossy(&keyword)
        ),
        read: Box::new(Shape::EffectByKey {
            trigger_limits: false,
        }),
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Node {
    Keys,
    Limit,
    Province,
    Trigger(combat_tactics::Node),
}

enum Limit {
    Empty,
    Children,
    Unknown(String),
}

pub(crate) struct Shaper<'n> {
    token: u32,
    names: Option<&'n KeyNames>,
    triggers: combat_tactics::Shaper<'n>,
    limit: Limit,
    expression: bool,
}

impl<'n> Shaper<'n> {
    pub(crate) fn new(names: Option<&'n KeyNames>, token: u32) -> Self {
        Self {
            token,
            names,
            triggers: combat_tactics::Shaper::new(names),
            limit: Limit::Empty,
            expression: false,
        }
    }

    pub(crate) fn shape(&mut self, node: Node, field: &Field<'_>) -> Shape<Node> {
        let keyword = field.keyword();
        let key = keyword.as_deref();
        match node {
            Node::Trigger(node) => self.triggers.shape(node, field).map(Node::Trigger),
            Node::Limit => {
                // CAndTrigger +0x14: LoadKey adds a child as the trigger
                // dispatch decides, including malformed registered values.
                let load = key_names::load_key(self.names, field);
                if !matches!(self.limit, Limit::Children) {
                    match load {
                        _ if load.adds_child() == Some(true) => self.limit = Limit::Children,
                        Load::Unknown(why) if matches!(self.limit, Limit::Empty) => {
                            self.limit = Limit::Unknown(why)
                        }
                        _ => {}
                    }
                }
                self.triggers
                    .shape(combat_tactics::Node::Trigger, field)
                    .map(Node::Trigger)
            }
            // CBuildingConstructionEffect@14031D4D0's province loop:
            // own keys only, invalid comparison operators RecordError.
            Node::Province => match key {
                Some(b"level") if !matches!(field.operator, Operator::Less | Operator::Greater) => {
                    Shape::Skip
                }
                Some(b"limit_to_victory_point")
                    if c_string(&field.value.text) != b"yes"
                        && !matches!(field.operator, Operator::Less | Operator::Greater) =>
                {
                    Shape::Skip
                }
                Some(
                    b"id"
                    | b"level"
                    | b"first"
                    | b"all_provinces"
                    | b"limit_to_coastal"
                    | b"limit_to_border"
                    | b"limit_to_naval_base"
                    | b"limit_to_victory_point"
                    | b"limit_to_border_country"
                    | b"limit_to_supply_node"
                    | b"limit_to_coastal_bunker",
                ) => Shape::One,
                _ => Shape::Skip,
            },
            Node::Keys => match reader(self.token).expect("ported effect key reader") {
                KeyReader::Conditional => match key {
                    // CIfEffect@141385BA0 rejects a limit on else, and a
                    // second limit while the embedded CAnd has children.
                    Some(b"limit") if self.token == ELSE_TOKEN => Shape::Skip,
                    Some(b"limit") => self.limit(),
                    _ => super::shape(self.names, field).map(|()| Node::Keys),
                },
                // CVariableEffect@1403B6640: var and tooltip are scalar;
                // value and shorthand variable names select CExpression
                // (14032B3D0 -> 141405ED0) on an opening value.
                KeyReader::Variable | KeyReader::DivideVariable => {
                    if matches!(key, Some(b"var" | b"tooltip"))
                        || (matches!(reader(self.token), Some(KeyReader::DivideVariable))
                            && key == Some(b"if_zero"))
                    {
                        return Shape::One;
                    }
                    if field.opens() {
                        self.expression = true;
                        Shape::Expression
                    } else if self.expression {
                        Shape::Unsettled("CVariableEffect 1403B6640: scalar value after an expression calls std::_Throw_bad_variant_access (1403BA3C0)".into())
                    } else {
                        Shape::One
                    }
                }
                KeyReader::Keys(keys, fallback) => {
                    if let Some((_, value)) =
                        keys.iter().find(|(name, _)| key == Some(name.as_bytes()))
                    {
                        return match value {
                            ValueReader::One => Shape::One,
                            ValueReader::ValueList => Shape::ValueList,
                            ValueReader::Localization => Shape::ObjectIfOpen(Node::Trigger(
                                combat_tactics::Node::BoundLocalization,
                            )),
                            ValueReader::Limit => self.limit(),
                            ValueReader::Province => Shape::ObjectIfOpen(Node::Province),
                        };
                    }
                    match fallback {
                        Fallback::Effect => super::shape(self.names, field).map(|()| Node::Keys),
                        Fallback::Only => Shape::Skip,
                        Fallback::One => Shape::One,
                    }
                }
            },
        }
    }

    fn limit(&self) -> Shape<Node> {
        match &self.limit {
            Limit::Empty => Shape::Body(Node::Limit),
            Limit::Children => Shape::Skip,
            Limit::Unknown(why) => Shape::Unsettled(format!(
                "effect limit: native key reader reads a later limit only into an empty CAndTrigger; whether the first holds a child is unknown: {why}"
            )),
        }
    }
}
