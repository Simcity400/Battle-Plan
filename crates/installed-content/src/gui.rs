//! Layout constants from `interface/*.gui`.
//!
//! A `.gui` file is ordinary Clausewitz text, so [`read`] returns the parsed
//! document unchanged. The helpers here find an element by its declared name
//! and read the fields the map passes need, keeping the game's own spellings:
//! `containerWindowType`, `instantTextboxType` and `instantTextBoxType` all
//! occur in `mapicons.gui`.

use std::io;

use crate::{
    ResolvedLayers,
    fields::{invalid, last, text},
    parser::{self, Document, Item, Value},
};

/// A `.gui` length: authored pixels, or a percentage of the parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Pixels(i32),
    /// `100%%` in the file, stored as 100.0.
    Percent(f64),
}

impl Length {
    fn parse(value: &Value, what: &str) -> io::Result<Self> {
        let scalar = text(value, what)?;
        match scalar.trim_end_matches('%') {
            trimmed if trimmed.len() < scalar.len() => trimmed
                .parse()
                .map(Length::Percent)
                .map_err(|_| invalid(format!("{what} is not a percentage: {scalar:?}"))),
            trimmed => trimmed
                .parse()
                .map(Length::Pixels)
                .map_err(|_| invalid(format!("{what} is not a length: {scalar:?}"))),
        }
    }
}

/// Reads a `.gui` file from the resolved layers.
pub fn read(layers: &ResolvedLayers, relative: &str) -> io::Result<Document> {
    parser::parse(&layers.read(relative)?).map_err(|error| invalid(format!("{relative}: {error}")))
}

fn named(item: &Item, kind: &str, name: &str) -> bool {
    item.key
        .as_deref()
        .is_some_and(|key| key.eq_ignore_ascii_case(kind.as_bytes()))
        && item
            .value
            .items()
            .and_then(|items| last(items, b"name"))
            .and_then(Value::scalar_str)
            == Some(name)
}

fn search<'a>(items: &'a [Item], kind: &str, name: &str) -> Option<&'a [Item]> {
    for item in items {
        if named(item, kind, name) {
            return item.value.items();
        }
        if let Some(nested) = item.value.items()
            && let Some(found) = search(nested, kind, name)
        {
            return Some(found);
        }
    }
    None
}

/// Finds an element of a kind by its `name`, at any depth.
///
/// The kind is matched without regard to case, because the shipped files
/// spell the same element `instantTextboxType` and `instantTextBoxType`.
pub fn find<'a>(document: &'a Document, kind: &str, name: &str) -> Option<&'a [Item]> {
    search(&document.items, kind, name)
}

/// Finds an element of any of several kinds, in the order given.
pub fn find_any<'a>(document: &'a Document, kinds: &[&str], name: &str) -> Option<&'a [Item]> {
    kinds.iter().find_map(|kind| find(document, kind, name))
}

fn pair(
    items: &[Item],
    key: &[u8],
    first: &[u8],
    second: &[u8],
) -> io::Result<Option<(Length, Length)>> {
    let Some(value) = last(items, key) else {
        return Ok(None);
    };
    let what = String::from_utf8_lossy(key).into_owned();
    let inner = value
        .items()
        .ok_or_else(|| invalid(format!("{what} is not a block")))?;
    let read = |name: &[u8]| -> io::Result<Length> {
        let field = last(inner, name)
            .ok_or_else(|| invalid(format!("{what} has no {}", String::from_utf8_lossy(name))))?;
        Length::parse(field, &what)
    };
    Ok(Some((read(first)?, read(second)?)))
}

/// `position = { x = .. y = .. }`.
pub fn position(items: &[Item]) -> io::Result<Option<(Length, Length)>> {
    pair(items, b"position", b"x", b"y")
}

/// `size = { width = .. height = .. }`.
pub fn size(items: &[Item]) -> io::Result<Option<(Length, Length)>> {
    pair(items, b"size", b"width", b"height")
}

/// `spriteType`, or the `quadTextureSprite` a button names instead.
pub fn sprite(items: &[Item]) -> Option<&str> {
    last(items, b"spriteType")
        .or_else(|| last(items, b"quadTextureSprite"))
        .and_then(Value::scalar_str)
}

/// `font`, the name of a `bitmapfont` declaration.
pub fn font(items: &[Item]) -> Option<&str> {
    last(items, b"font").and_then(Value::scalar_str)
}

/// `format`, written quoted or bare in the same file.
pub fn format(items: &[Item]) -> Option<&str> {
    last(items, b"format").and_then(Value::scalar_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUI: &[u8] = br#"
guiTypes = {
    containerWindowType = {
        name = "mapicons_container"
        position = { x=0 y=0 }
        size = { width=100%% height=100%% }
    }
    containerWindowType = {
        name = "unit_counter"
        position = { x=0 y=0 }
        size = { width = 76 height = 24 }
        buttonType = {
            name = "count_bg"
            quadTextureSprite = "GFX_onmap_unit_counter"
            position = { x=0 y=0 }
        }
        iconType = {
            name = "group_marker"
            spriteType = "GFX_UV_group_marker"
            position = { x=55 y=-9 }
        }
        instantTextboxType = {
            name = "count"
            position = { x= 36 y = 8 }
            font = "hoi_18mbs"
            format = left
            maxWidth = 60
        }
    }
    positionType = { name = "unit_counter_zoomed_in_offset" position = { x = 0 y = 20 } }
}
"#;

    fn document() -> Document {
        parser::parse(GUI).unwrap()
    }

    #[test]
    fn elements_are_found_by_name_at_any_depth_and_whatever_case_the_kind_uses() {
        let document = document();
        let container = find(&document, "containerWindowType", "unit_counter").unwrap();
        assert_eq!(
            size(container).unwrap(),
            Some((Length::Pixels(76), Length::Pixels(24)))
        );
        let icon = find(&document, "iconType", "group_marker").unwrap();
        assert_eq!(
            position(icon).unwrap(),
            Some((Length::Pixels(55), Length::Pixels(-9)))
        );
        assert_eq!(sprite(icon), Some("GFX_UV_group_marker"));
        // The shipped files spell this element both ways.
        let text = find(&document, "instantTextBoxType", "count").unwrap();
        assert_eq!(font(text), Some("hoi_18mbs"));
        assert_eq!(format(text), Some("left"));
        assert_eq!(sprite(text), None);
        assert_eq!(
            find_any(&document, &["iconType", "buttonType"], "count_bg").map(sprite),
            Some(Some("GFX_onmap_unit_counter"))
        );
        assert!(find(&document, "iconType", "missing").is_none());
        assert!(find(&document, "gridBoxType", "unit_counter").is_none());
    }

    #[test]
    fn percentage_lengths_keep_their_meaning() {
        let document = document();
        let container = find(&document, "containerWindowType", "mapicons_container").unwrap();
        assert_eq!(
            size(container).unwrap(),
            Some((Length::Percent(100.0), Length::Percent(100.0)))
        );
        assert_eq!(
            position(container).unwrap(),
            Some((Length::Pixels(0), Length::Pixels(0)))
        );
    }

    #[test]
    fn missing_and_malformed_layout_fields_are_distinguished() {
        let document = parser::parse(
            br#"guiTypes = { iconType = { name = "a" }
                           iconType = { name = "b" position = { x = 1 } }
                           iconType = { name = "c" size = 4 }
                           iconType = { name = "d" position = { x = wide y = 1 } } }"#,
        )
        .unwrap();
        assert_eq!(
            position(find(&document, "iconType", "a").unwrap()).unwrap(),
            None
        );
        assert!(
            position(find(&document, "iconType", "b").unwrap())
                .unwrap_err()
                .to_string()
                .contains("position has no y")
        );
        assert!(
            size(find(&document, "iconType", "c").unwrap())
                .unwrap_err()
                .to_string()
                .contains("size is not a block")
        );
        assert!(
            position(find(&document, "iconType", "d").unwrap())
                .unwrap_err()
                .to_string()
                .contains("not a length")
        );
    }
}
