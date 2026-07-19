//! Crate-internal helpers shared by the scene builders (`trade_state`,
//! `major_views`, `hud`, `selected_present`). These were previously duplicated
//! verbatim per module; a single copy keeps the shared laws (named-element
//! loading, greedy word-wrap) from drifting between consumers.

use crate::assets::AssetResolver;
use crate::gui::{parse_measure, parse_measure_pair, GuiElement, LayoutSet, MeasurePair};
use crate::paradox_text::parse_file;
use crate::{GameUiError, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// Parse one `.gui` source file into a fresh [`LayoutSet`].
pub(crate) fn parse_layout_set(path: &Path, rel: &str) -> Result<LayoutSet> {
    let doc = parse_file(path)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(rel, &doc);
    Ok(layouts)
}

/// Find a named element in an already-parsed layout set, with the same typed
/// missing-element refusal the per-module loaders produced.
pub(crate) fn find_named_element(
    layouts: &LayoutSet,
    source_label: &str,
    name: &str,
) -> Result<GuiElement> {
    layouts
        .find_by_name(name)
        .into_iter()
        .next()
        .cloned()
        .ok_or_else(|| GameUiError::missing(format!("{source_label}: element {name} not found")))
}

/// Load a named element from an already-resolved file path.
pub(crate) fn load_named_element_from_file(
    path: &Path,
    rel: &str,
    name: &str,
) -> Result<GuiElement> {
    find_named_element(&parse_layout_set(path, rel)?, rel, name)
}

/// Load a named element through the resolver, caching the parsed
/// [`LayoutSet`] per source file so a scene that references the same `.gui`
/// document many times (root + per-row templates) resolves and parses it once.
/// The first resolve/parse failure for a file returns `Err` exactly as the
/// uncached loaders did; absent named elements keep their typed refusal per
/// lookup.
pub(crate) fn load_named_element(
    cache: &mut BTreeMap<String, LayoutSet>,
    resolver: &AssetResolver,
    file: &str,
    name: &str,
) -> Result<GuiElement> {
    if !cache.contains_key(file) {
        let resolved = resolver.resolve_file(file)?;
        let layouts = parse_layout_set(&resolved.path, &resolved.rel_path)?;
        cache.insert(file.to_string(), layouts);
    }
    find_named_element(cache.get(file).expect("cache populated"), file, name)
}

/// Promote an authored `show_position` field to the element's position.
pub(crate) fn promote_show_position(element: &mut GuiElement) {
    if let Some(position) = element
        .raw
        .first_value_ci("show_position")
        .and_then(parse_measure_pair)
    {
        element.position = Some(position);
    }
}

/// Overwrite the element position, leaving the authored orientation intact.
pub(crate) fn set_position(element: &mut GuiElement, x: i32, y: i32) {
    element.position = Some(pair(x, y));
}

/// Overwrite the element position and force `UPPER_LEFT` orientation (the
/// trade-scene variant; `major_views` sets orientation explicitly instead).
pub(crate) fn set_position_upper_left(element: &mut GuiElement, x: i32, y: i32) {
    set_position(element, x, y);
    element.orientation = Some("UPPER_LEFT".to_string());
}

/// Recursively prefix every element name in the subtree.
pub(crate) fn prefix_element_names(element: &mut GuiElement, prefix: &str) {
    element.name = format!("{prefix}{}", element.name);
    for child in &mut element.children {
        prefix_element_names(child, prefix);
    }
}

pub(crate) fn pair(x: i32, y: i32) -> MeasurePair {
    MeasurePair {
        x: parse_measure(&x.to_string()),
        y: parse_measure(&y.to_string()),
    }
}

/// Greedy word-wrap shared by the HUD tooltip compositor and the
/// authored-reproducible selected-Present text law. The two consumers must
/// agree byte-for-byte, so there is exactly one copy of this law.
pub(crate) fn wrap_text_greedy<F>(text: &str, max_width: i32, measure: F) -> String
where
    F: Fn(&str) -> i32,
{
    if max_width <= 0 {
        return text.to_string();
    }
    let mut out = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure(&candidate) > max_width {
                out.push(current);
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        out.push(current);
    }
    out.join("\n")
}
