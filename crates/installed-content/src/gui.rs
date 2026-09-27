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

/// Finds an element of a kind by its `name` at any depth **below** `items`.
///
/// A `.gui` file reuses child names across windows — `mapicons.gui` has an
/// `icon` and a `text` in both label windows — so a caller that already found
/// the parent has to search inside it rather than the whole document.
pub fn find_child<'a>(items: &'a [Item], kinds: &[&str], name: &str) -> Option<&'a [Item]> {
    kinds.iter().find_map(|kind| search(items, kind, name))
}

/// A plain numeric field such as `maxWidth`, which is not inside a block.
pub fn number(items: &[Item], key: &str) -> io::Result<Option<f64>> {
    let Some(value) = last(items, key.as_bytes()) else {
        return Ok(None);
    };
    let scalar = text(value, key)?;
    scalar
        .parse()
        .map(Some)
        .map_err(|_| invalid(format!("{key} is not a number: {scalar:?}")))
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

/// A size independent of its parent, for map-icon windows. CSize::evaluate
/// (1416B9BE0) applies each minimum before its maximum, then preserves aspect
/// if requested. CSize::construct (1422EE9F0) leaves both bounds unspecified;
/// CSize::parse (1422EF100 / 1422EFE10) permits a bound on just one axis.
/// Parent-dependent lengths are refused instead of using a guessed viewport.
pub fn fixed_size(items: &[Item]) -> io::Result<Option<[i32; 2]>> {
    let Some((width, height)) = size(items)? else {
        return Ok(None);
    };
    let fixed = |length: Length, what: &str| match length {
        Length::Pixels(value) if value >= 0 => Ok(value),
        _ => Err(invalid(format!(
            "{what} depends on the parent size or position"
        ))),
    };
    let original = [fixed(width, "size width")?, fixed(height, "size height")?];
    let fields = last(items, b"size")
        .and_then(Value::items)
        .unwrap_or_default();
    let bounds = |key: &[u8]| -> io::Result<[Option<i32>; 2]> {
        let mut result = [None; 2];
        for item in fields
            .iter()
            .filter(|item| item.key.as_deref() == Some(key))
        {
            let what = format!("size {}", String::from_utf8_lossy(key));
            let fields = crate::fields::block(&item.value, &what)?;
            for (axis, name) in [b"width".as_slice(), b"height"].into_iter().enumerate() {
                if let Some(value) = last(fields, name) {
                    result[axis] = Some(fixed(Length::parse(value, &what)?, &what)?);
                }
            }
        }
        Ok(result)
    };
    let min = bounds(b"min")?;
    let max = bounds(b"max")?;
    let mut result = original;
    for axis in 0..2 {
        if let Some(min) = min[axis] {
            result[axis] = result[axis].max(min);
        }
        if let Some(max) = max[axis] {
            result[axis] = result[axis].min(max);
        }
    }
    let preserve = last(fields, b"preserve_aspect_ratio")
        .map(|value| crate::fields::boolean(value, "size preserve_aspect_ratio"))
        .transpose()?
        .unwrap_or(false);
    if preserve && original.into_iter().chain(result).all(|value| value > 0) {
        // 1416B9F89 / 1416B9F9D: add 0.5f, floorf, then CVTTSS2SI.
        // Refuse an out-of-range aspect result rather than saturating it.
        let rounded = |value: f32| -> io::Result<i32> {
            let value = (value + 0.5).floor();
            if value >= -(i32::MIN as f32) {
                return Err(invalid(
                    "size aspect-ratio result exceeds the native integer range",
                ));
            }
            Ok(value as i32)
        };
        let ratio = original[0] as f32 / original[1] as f32;
        let actual = result[0] as f32 / result[1] as f32;
        if actual > ratio {
            if min[0] == Some(result[0]) {
                result[1] = rounded(result[0] as f32 / ratio)?;
                if let Some(max) = max[1] {
                    result[1] = result[1].min(max);
                }
            } else {
                result[0] = rounded(result[1] as f32 * ratio)?;
                if let Some(min) = min[0] {
                    result[0] = result[0].max(min);
                }
            }
        } else if actual < ratio {
            if min[1] == Some(result[1]) {
                result[0] = rounded(result[1] as f32 * ratio)?;
                if let Some(max) = max[0] {
                    result[0] = result[0].min(max);
                }
            } else {
                result[1] = rounded(result[0] as f32 / ratio)?;
                if let Some(min) = min[1] {
                    result[1] = result[1].max(min);
                }
            }
        }
    }
    Ok(Some(result))
}

// counters
/// `slotsize = { width = .. height = .. }`, a `gridBoxType`'s cell step.
pub fn slot_size(items: &[Item]) -> io::Result<Option<(Length, Length)>> {
    pair(items, b"slotsize", b"width", b"height")
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
    fn fixed_sizes_apply_partial_minima_then_maxima() {
        for (bounds, expected) in [
            ("", [20, 30]),
            ("min={width=25}", [25, 30]),
            ("max={height=15}", [20, 15]),
            (
                "min={width=40 height=50} max={width=25 height=35}",
                [25, 35],
            ),
            ("min={width=40} min={height=50}", [40, 50]),
            ("max={width=0 height=0}", [0, 0]),
        ] {
            let document =
                parser::parse(format!("size={{width=20 height=30 {bounds}}}").as_bytes()).unwrap();
            assert_eq!(
                fixed_size(&document.items).unwrap(),
                Some(expected),
                "{bounds}"
            );
        }
    }

    #[test]
    fn fixed_sizes_preserve_aspect_after_bounds_using_native_ties() {
        for (bounds, expected) in [
            ("min={width=40}", [40, 20]),
            ("min={height=20}", [40, 20]),
            ("max={width=15}", [15, 8]),
            ("max={height=6}", [12, 6]),
            ("min={width=40} max={height=15}", [40, 15]),
            ("min={height=20} max={width=30}", [30, 20]),
        ] {
            let document = parser::parse(
                format!("size={{width=20 height=10 preserve_aspect_ratio=yes {bounds}}}")
                    .as_bytes(),
            )
            .unwrap();
            assert_eq!(
                fixed_size(&document.items).unwrap(),
                Some(expected),
                "{bounds}"
            );
        }
    }

    #[test]
    fn fixed_sizes_refuse_parent_dependent_or_malformed_bounds() {
        for fields in [
            "width=100% height=20",
            "width=-1 height=20",
            "width=20 height=30 min={width=100%}",
            "width=20 height=30 max={height=-1}",
            "width=20 height=30 min=no",
            "width=20 height=30 max={width=no}",
            "width=20 height=30 preserve_aspect_ratio=maybe",
        ] {
            let document = parser::parse(format!("size={{{fields}}}").as_bytes()).unwrap();
            assert!(fixed_size(&document.items).is_err(), "{fields}");
        }
    }

    #[test]
    fn a_child_is_searched_for_inside_its_own_parent_and_plain_numbers_are_read() {
        let document = document();
        let container = find(&document, "containerWindowType", "unit_counter").unwrap();
        let count = find_child(container, &["instantTextboxType"], "count").unwrap();
        assert_eq!(
            position(count).unwrap(),
            Some((Length::Pixels(36), Length::Pixels(8)))
        );
        // `maxWidth` sits beside the blocks, not inside `size`.
        assert_eq!(number(count, "maxWidth").unwrap(), Some(60.0));
        assert_eq!(number(count, "maxHeight").unwrap(), None);
        // The first window has no `count`, so searching inside it finds none.
        let other = find(&document, "containerWindowType", "mapicons_container").unwrap();
        assert!(find_child(other, &["instantTextboxType"], "count").is_none());
        // A non-numeric value is a named error rather than a silent zero.
        assert!(number(count, "font").is_err());
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
