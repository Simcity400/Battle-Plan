//! Ideologies (`common/ideologies`): each ideology group, its colour and its
//! sub-ideologies. The recording names a party by its group token and a leader
//! by a sub-ideology; this joins both to the group and its colour, so a mod's
//! ideologies chart with their own colours.

use crate::{
    ResolvedLayers,
    parser::{self, Item, Value},
    script_files,
};
use std::io;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ideology {
    pub token: String,
    /// sRGB bytes; None when the colour is missing or not three 0–255 numbers.
    pub color: Option<[u8; 3]>,
    /// Sub-ideologies (`types`), in source order.
    pub types: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ideologies {
    /// In load order.
    pub groups: Vec<Ideology>,
}

impl Ideologies {
    pub fn get(&self, token: &str) -> Option<&Ideology> {
        self.groups.iter().find(|group| group.token == token)
    }

    /// The group a sub-ideology belongs to.
    pub fn group_of(&self, sub_ideology: &str) -> Option<&Ideology> {
        self.groups
            .iter()
            .find(|group| group.types.iter().any(|kind| kind == sub_ideology))
    }
}

/// `color = { r g b }` with three 0–255 numbers. An `hsv` colour or anything
/// else is left unavailable rather than converted by guesswork.
fn color(value: &Value) -> Option<[u8; 3]> {
    let items = match value {
        Value::Block(items) => items,
        Value::Tagged { tag, items } if tag.as_slice() == b"rgb" => items,
        _ => return None,
    };
    let numbers: Vec<u8> = items
        .iter()
        .filter(|item| item.key.is_none())
        .map(|item| {
            item.value
                .scalar_str()?
                .parse::<f64>()
                .ok()
                .filter(|n| n.fract() == 0.0 && (0.0..=255.0).contains(n))
                .map(|n| n as u8)
        })
        .collect::<Option<_>>()?;
    <[u8; 3]>::try_from(numbers).ok()
}

fn key(item: &Item) -> Option<&str> {
    std::str::from_utf8(item.key.as_deref()?).ok()
}

pub fn parse(input: &[u8]) -> io::Result<Vec<Ideology>> {
    Ok(from_document(&parser::parse_unclosed(input)?.0))
}

fn from_document(document: &parser::Document) -> Vec<Ideology> {
    let mut groups = Vec::new();
    for root in document.get_all(b"ideologies") {
        for item in root.items().unwrap_or_default() {
            let (Some(token), Some(items)) = (key(item), item.value.items()) else {
                continue;
            };
            let last = |wanted: &[u8]| {
                items
                    .iter()
                    .rev()
                    .find(|i| i.key.as_deref() == Some(wanted))
                    .map(|i| &i.value)
            };
            groups.push(Ideology {
                token: token.to_owned(),
                color: last(b"color").and_then(color),
                types: last(b"types")
                    .and_then(Value::items)
                    .map(|types| types.iter().filter_map(key).map(str::to_owned).collect())
                    .unwrap_or_default(),
            });
        }
    }
    groups
}

/// Every ideology group in load order; a group defined again replaces the earlier one in place.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Ideologies, Vec<String>)> {
    let mut warnings = Vec::new();
    let mut result = Ideologies::default();
    for (_, document) in
        script_files::documents(layers, "common/ideologies/", "Ideologies", &mut warnings)?
    {
        for group in from_document(&document) {
            match result
                .groups
                .iter_mut()
                .find(|existing| existing.token == group.token)
            {
                Some(existing) => *existing = group,
                None => result.groups.push(group),
            }
        }
    }
    let uncoloured: Vec<_> = result
        .groups
        .iter()
        .filter(|group| group.color.is_none())
        .map(|group| group.token.as_str())
        .collect();
    if !uncoloured.is_empty() {
        warnings.push(format!(
            "No usable colour for ideologies: {}.",
            uncoloured.join(", ")
        ));
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_keep_colours_and_sub_ideologies() {
        let groups = parse(
            b"ideologies = { democratic = { types = { conservatism = { } liberalism = { } } color = { 0 0 255 } rules = { a = yes } } \
              fascism = { types = { nazism = { } } color = rgb { 150 75 0 } } odd = { color = hsv { 0.1 0.5 0.5 } } }",
        )
        .unwrap();
        let ideologies = Ideologies { groups };
        assert_eq!(
            ideologies.get("democratic").unwrap().color,
            Some([0, 0, 255])
        );
        assert_eq!(ideologies.get("fascism").unwrap().color, Some([150, 75, 0]));
        assert_eq!(ideologies.get("odd").unwrap().color, None);
        assert_eq!(
            ideologies.group_of("liberalism").unwrap().token,
            "democratic"
        );
        assert!(ideologies.group_of("unknown").is_none());
    }
}
