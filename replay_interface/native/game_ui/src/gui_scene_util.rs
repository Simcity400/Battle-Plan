
use crate::assets::AssetResolver;
use crate::gui::{parse_measure, parse_measure_pair, GuiElement, LayoutSet, MeasurePair};
use crate::paradox_text::parse_checked_bytes;
use crate::{GameUiError, Result};
use std::collections::BTreeMap;

pub(crate) fn parse_layout_set(bytes: &[u8], rel: &str) -> Result<LayoutSet> {
    let doc = parse_checked_bytes(bytes, rel)?;
    let mut layouts = LayoutSet::default();
    layouts.add_document(rel, &doc);
    Ok(layouts)
}

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

pub(crate) fn load_named_element_from_bytes(
    bytes: &[u8],
    rel: &str,
    name: &str,
) -> Result<GuiElement> {
    find_named_element(&parse_layout_set(bytes, rel)?, rel, name)
}

pub(crate) fn load_last_top_level_named_element_from_bytes(
    bytes: &[u8],
    rel: &str,
    name: &str,
) -> Result<GuiElement> {
    find_last_top_level_named_element(&parse_layout_set(bytes, rel)?, rel, name)
}

pub(crate) fn load_named_element(
    cache: &mut BTreeMap<String, LayoutSet>,
    resolver: &AssetResolver,
    file: &str,
    name: &str,
) -> Result<GuiElement> {
    if !cache.contains_key(file) {
        let resolved = resolver.resolve_file(file)?;
        let layouts = parse_layout_set(&resolver.read_file(&resolved)?, &resolved.rel_path)?;
        cache.insert(file.to_string(), layouts);
    }
    find_named_element(cache.get(file).expect("cache populated"), file, name)
}

pub(crate) fn load_last_top_level_named_element(
    cache: &mut BTreeMap<String, LayoutSet>,
    resolver: &AssetResolver,
    file: &str,
    name: &str,
) -> Result<GuiElement> {
    if !cache.contains_key(file) {
        let resolved = resolver.resolve_file(file)?;
        let layouts = parse_layout_set(&resolver.read_file(&resolved)?, &resolved.rel_path)?;
        cache.insert(file.to_string(), layouts);
    }
    find_last_top_level_named_element(cache.get(file).expect("cache populated"), file, name)
}

pub(crate) fn promote_show_position(element: &mut GuiElement) {
    if let Some(position) = element
        .raw
        .first_value_ci("show_position")
        .and_then(parse_measure_pair)
    {
        element.position = Some(position);
    }
}

pub(crate) fn set_position(element: &mut GuiElement, x: i32, y: i32) {
    element.position = Some(pair(x, y));
}

pub(crate) fn set_position_upper_left(element: &mut GuiElement, x: i32, y: i32) {
    set_position(element, x, y);
    element.orientation = Some("UPPER_LEFT".to_string());
}

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
            load_last_top_level_named_element_from_bytes(&std::fs::read(&path).unwrap(), "interface/test.gui", "shared")
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
