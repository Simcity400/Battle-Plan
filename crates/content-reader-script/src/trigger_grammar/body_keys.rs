
use super::{Node, Shaper};
use crate::native::{Field, Shape};

impl Shaper<'_> {
    pub(super) fn body_value_needs_names(token: u32, field: &Field<'_>) -> bool {
        matches!(
            (token, field.keyword().as_deref()),
            (0x3BE4 | 0x37DC, Some(b"building"))
        )
    }

    pub(super) fn body_value(&mut self, token: u32, field: &Field<'_>) -> Shape<Node> {
        let key = field.keyword();
        match (token, key.as_deref()) {
            (0x4967 | 0x49DA | 0x49DB | 0x49DC | 0x47B9, Some(b"target")) if field.opens() => {
                Shape::ValueList
            }
            (0x4363, Some(b"target")) if field.opens() => Shape::ValueList,
            (0x27FA | 0x474B | 0x474C | 0x40DF, Some(b"collection")) => Self::collection(field),
            (0x40DF, Some(b"countries")) if field.opens() => Shape::ValueList,
            (0x3CBD | 0x3CBE | 0x3CCE | 0x3CCF | 0x3CC0 | 0x3CC1 | 0x3E53, Some(b"id"))
            | (0x3CBD, Some(b"range"))
            | (0x3CCE, Some(b"side")) => Shape::Current,
            (0x39CE, Some(b"regions"))
            | (0x3164, Some(b"resources"))
            | (0x4D7E | 0x4D7F, Some(b"states" | b"types" | b"exclude")) => Shape::RawList,
            (0x3C75, Some(b"sub_unit_def_weights")) => Shape::Object(Node::ScalarFields),
            (0x3366 | 0x4CBB, Some(b"idea" | b"ignore")) if field.opens() => Shape::RawList,
            (0x37F7, Some(b"categories" | b"slots")) if field.opens() => Shape::CheckedList {
                stores_value: false,
            },
            (0x3BE4, Some(b"building")) => self.building(field, false, 0x140491A40),
            (0x39E7 | 0x3A2B, Some(b"type")) if field.opens() => Shape::Skip,
            (0x3E81 | 0x3F14, Some(b"compare")) if !comparison(field) => Shape::Skip,
            (0x37DC, Some(b"province")) => Shape::Object(Node::KeyReader(0x140491320)),
            (0x37DC, Some(b"building")) => self.building(field, true, 0x140491100),
            (0x37DC, Some(b"level"))
                if !matches!(
                    field.operator,
                    content_reader_parser::parser::Operator::Less | content_reader_parser::parser::Operator::Greater
                ) =>
            {
                Shape::Skip
            }
            (0x37B9, Some(b"equipment")) => Shape::Object(Node::KeyReader(0x1414C4850)),
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

    pub(super) fn native_key(&mut self, address: u64, field: &Field<'_>) -> Shape<Node> {
        let key = field.keyword();
        let key = key.as_deref();
        match address {
            0x14045AC70 if !matches!(key, Some(b"tooltip" | b"var")) && field.opens() => {
                Shape::Expression
            }
            0x14045AC70 | 0x140456ff0 | 0x140457940 | 0x14045A7F0 | 0x14045FF80
            | 0x140491C10 | 0x140458040 | 0x1403C9110 | 0x140457BC0
            | 0x1404571a0 | 0x140457AB0 | 0x14045A250 | 0x1404A0810
            | 0x140477D70 => Shape::One,
            0x140456DB0 | 0x14045FD00 if key == Some(b"compare") && !comparison(field) => {
                Shape::Skip
            }
            0x140456DB0 | 0x14045FD00 => Shape::One,
            0x1403CC550 if key == Some(b"var_list") => Shape::List,
            0x1403CC550 => Shape::One,
            0x1403CC150 if key == Some(b"text") => Shape::RawText,
            0x1403CC150 => Shape::One,
            0x140491320
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
            0x1414C4850
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
            0x140491320 | 0x1414C4850 => Shape::Skip,
            0x140A6A4C0 if key == Some(b"name") => Shape::ObjectIfOpen(Node::BoundLocalization),
            0x140A6A4C0 if key == Some(b"operators") && field.opens() => {
                Shape::Statements(Node::CollectionOperator)
            }
            0x140A6A4C0 if key == Some(b"input") => Shape::One,
            0x140A6A4C0 => Shape::Skip,
            _ => Self::refuse_reader(address, field.key.c_str()),
        }
    }

    pub(super) fn collection_operator(&mut self, field: &Field<'_>) -> Shape<Node> {
        if field.key.text.is_empty() {
            return if field.opens() {
                Shape::Skip
            } else {
                Shape::One
            };
        }
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
        if field.opens() {
            Shape::Object(Node::KeyReader(0x140A6A4C0))
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

#[cfg(any(test, feature = "test-support"))]
pub(super) const VARIABLE_EXPRESSION_FIELDS: u64 = 0x14045AC70;

#[cfg(test)]
mod tests;
