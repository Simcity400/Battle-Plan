//! Specialized key dispatch and embedded helper objects.

use super::{Node, Shaper};
use crate::parser::native::{Field, Shape};

impl Shaper<'_> {
    /// Value readers of the static keys in KEY_READERS. The remaining
    /// accepted keys call scalar/string/scoped-variable readers only.
    pub(super) fn body_value(&mut self, token: u32, field: &Field<'_>) -> Shape<Node> {
        let key = field.keyword();
        match (token, key.as_deref()) {
            // Each entry sets the value slot; the closing token resolves only
            // into the lexer.
            (0x4967 | 0x49DA | 0x49DB | 0x49DC | 0x47B9, Some(b"target")) if field.opens() => {
                Shape::ValueList
            }
            // An opening brace selects the state array. Entries set the value
            // slot, while the close does not.
            (0x4363, Some(b"target")) if field.opens() => Shape::ValueList,
            // Collections are the same embedded collection reader everywhere.
            (0x27FA | 0x474B | 0x474C | 0x40DF, Some(b"collection")) => Self::collection(field),
            // Country entries follow the same array read.
            (0x40DF, Some(b"countries")) if field.opens() => Shape::ValueList,
            // Read the current lexer token, preserving the reader's retained value
            // slot. Power-balance readers use the same path.
            (0x3CBD | 0x3CBE | 0x3CCE | 0x3CCF | 0x3CC0 | 0x3CC1 | 0x3E53, Some(b"id"))
            | (0x3CBD, Some(b"range"))
            | (0x3CCE, Some(b"side")) => Shape::Current,
            // Integer and token lists consume resolved tokens.
            (0x39CE, Some(b"regions"))
            | (0x3164, Some(b"resources"))
            | (0x4D7E | 0x4D7F, Some(b"states" | b"types" | b"exclude")) => Shape::RawList,
            // The navy's weight map reads any key, then one fixed5 token.
            (0x3C75, Some(b"sub_unit_def_weights")) => Shape::Object(Node::ScalarFields),
            // Idea and ignore fields are lists only when the value opens a block.
            (0x3366 | 0x4CBB, Some(b"idea" | b"ignore")) if field.opens() => Shape::RawList,
            // Categories and slots accept a scalar or list.
            (0x37F7, Some(b"categories" | b"slots")) if field.opens() => Shape::CheckedList {
                stores_value: false,
            },
            (0x3BE4, Some(b"building")) => self.building(field, false, 0x140472210),
            // Type fields reject an opening value.
            (0x39E7 | 0x3A2B, Some(b"type")) if field.opens() => Shape::Skip,
            // Invalid comparison operators are rejected.
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
            // License conditions contain an equipment variant reference.
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
            // Variable triggers select an expression for an opening value,
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
            // Province-building filters and equipment variant references are
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
            // The three authored collection fields.
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
        // Bare values consume no further token; anonymous blocks use error recovery.
        if field.key.text.is_empty() {
            return if field.opens() {
                Shape::Skip
            } else {
                Shape::One
            };
        }
        // Only `limit` reads an AND trigger. The four bare-only operators use
        // raw block skipping after a scan error; every other key uses error recovery.
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
        // An opening value reads collection fields; any other value reads
        // one token. Post-read checks do not skip.
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
