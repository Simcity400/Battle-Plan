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

fn find_last_top_level_named_element(
    layouts: &LayoutSet,
    source_label: &str,
    name: &str,
) -> Result<GuiElement> {
    layouts
        .documents
        .iter()
        .flat_map(|document| document.roots.iter())
        .rev()
        .find(|element| element.name.eq_ignore_ascii_case(name))
        .cloned()
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "{source_label}: top-level element {name} not found"
            ))
        })
}

/// Load a named element from an already-resolved file path.
pub(crate) fn load_named_element_from_file(
    path: &Path,
    rel: &str,
    name: &str,
) -> Result<GuiElement> {
    find_named_element(&parse_layout_set(path, rel)?, rel, name)
}

/// Load the last top-level declaration with `name` from one parsed file.
///
/// GUI roots are entered into the engine registry in parse order, so a later
/// same-name top-level declaration in the same file replaces the earlier one.
/// Recursive `find_by_name(...).next()` can instead select a descendant or the
/// shadowed first declaration.
pub(crate) fn load_last_top_level_named_element_from_file(
    path: &Path,
    rel: &str,
    name: &str,
) -> Result<GuiElement> {
    find_last_top_level_named_element(&parse_layout_set(path, rel)?, rel, name)
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

/// Load the engine-winning last top-level declaration through the resolver,
/// caching the parsed file for root/template pairs from the same source.
pub(crate) fn load_last_top_level_named_element(
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
    find_last_top_level_named_element(cache.get(file).expect("cache populated"), file, name)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn top_level_loader_uses_same_file_last_parsed_winner() {
        let root = std::env::temp_dir().join(format!(
            "game_ui_last_top_level_{}_{}",
            std::process::id(),
            line!()
        ));
        let interface = root.join("interface");
        fs::create_dir_all(&interface).unwrap();
        let path = interface.join("unitleaderwindow.gui");
        fs::write(
            &path,
            b"guiTypes = {
                containerWindowType = { name = \"shared\" position = { x = 1 y = 2 } }
                containerWindowType = { name = \"SHARED\" position = { x = 9 y = 8 } }
            }",
        )
        .unwrap();
        let winner =
            load_last_top_level_named_element_from_file(&path, "interface/test.gui", "shared")
                .unwrap();
        assert_eq!(winner.name, "SHARED");
        assert_eq!(winner.position.as_ref().unwrap().x.raw, "9");

        let resolver = AssetResolver::new(vec![root.clone()]);
        let mut cache = BTreeMap::new();
        let cached = load_last_top_level_named_element(
            &mut cache,
            &resolver,
            "interface/unitleaderwindow.gui",
            "shared",
        )
        .unwrap();
        assert_eq!(cached.name, "SHARED");
        assert_eq!(cached.position.as_ref().unwrap().x.raw, "9");
        fs::remove_dir_all(root).ok();
    }
}
