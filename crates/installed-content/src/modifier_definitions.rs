//! Scripted modifier definitions (`common/modifier_definitions`): how a
//! content-defined modifier is shown (good or bad colour, number or percentage,
//! precision, postfix). Built-in modifiers are defined in the executable, not in
//! these files; their display comes from the recording, never from a list here.

use crate::{ResolvedLayers, parser, script_files};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModifierDefinition {
    pub token: String,
    /// `good`, `bad` or `neutral` as authored.
    pub color_type: Option<String>,
    /// `number`, `percentage`, `percentage_in_hundred` or `yes_no` as authored.
    pub value_type: Option<String>,
    pub precision: Option<u32>,
    /// `none`, `days`, `hours` or `daily` as authored.
    pub postfix: Option<String>,
    /// Every category it belongs to; `category` may be assigned more than once.
    pub categories: Vec<String>,
}

pub fn parse(input: &[u8]) -> io::Result<Vec<ModifierDefinition>> {
    Ok(from_document(&parser::parse_unclosed(input)?.0))
}

fn from_document(document: &parser::Document) -> Vec<ModifierDefinition> {
    document
        .items
        .iter()
        .filter_map(|item| {
            let token = std::str::from_utf8(item.key.as_deref()?).ok()?;
            let items = item.value.items()?;
            let text = |key: &[u8]| {
                items
                    .iter()
                    .rev()
                    .find(|i| i.key.as_deref() == Some(key))
                    .and_then(|i| i.value.scalar_str())
                    .map(str::to_owned)
            };
            Some(ModifierDefinition {
                token: token.to_owned(),
                color_type: text(b"color_type"),
                value_type: text(b"value_type"),
                precision: text(b"precision").and_then(|p| p.parse().ok()),
                postfix: text(b"postfix"),
                categories: items
                    .iter()
                    .filter(|i| i.key.as_deref() == Some(b"category"))
                    .filter_map(|i| i.value.scalar_str().map(str::to_owned))
                    .collect(),
            })
        })
        .collect()
}

/// Every scripted modifier definition, later files replacing earlier ones.
pub fn read(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, ModifierDefinition>, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result = BTreeMap::new();
    for (_, document) in script_files::documents(
        layers,
        "common/modifier_definitions/",
        "Modifier definitions",
        &mut warnings,
    )? {
        result.extend(
            from_document(&document)
                .into_iter()
                .map(|d| (d.token.clone(), d)),
        );
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_keep_their_display_fields() {
        let definitions = parse(b"operation_outcome = { color_type = good value_type = percentage precision = 0 category = intelligence_agency }").unwrap();
        assert_eq!(definitions[0].token, "operation_outcome");
        assert_eq!(definitions[0].value_type.as_deref(), Some("percentage"));
        assert_eq!(definitions[0].precision, Some(0));
        assert_eq!(definitions[0].postfix, None);
    }
}
