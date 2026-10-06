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
        // (140A0E020). The four bare-only operators use raw SkipBlock after
        // TryHandleOrRecordError; every other key uses RecordError's skip.
        match field.keyword().as_deref() {
            Some(b"limit") => Shape::Body(Node::Trigger),
            Some(
                b"country_and_all_subjects"
                | b"owned_states"
                | b"controlled_states"
                | b"faction_members",
            ) => Shape::RawSkip,
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
mod tests;
