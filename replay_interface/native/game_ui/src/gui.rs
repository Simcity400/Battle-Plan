use crate::paradox_text::{first_scalar_any, parse_file, Document, Value};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutDocument {
    pub source_file: String,
    pub roots: Vec<GuiElement>,
    pub recoveries: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GuiElement {
    pub element_type: String,
    pub name: String,
    pub position: Option<MeasurePair>,
    pub size: Option<MeasurePair>,
    pub orientation: Option<String>,
    pub sprite_refs: Vec<String>,
    pub font: Option<String>,
    pub text: Option<String>,
    pub format: Option<String>,
    pub children: Vec<GuiElement>,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasurePair {
    pub x: Measure,
    pub y: Measure,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measure {
    pub raw: String,
    pub value: Option<String>,
    pub is_percent: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuiParseStats {
    pub file_count: usize,
    pub window_count: usize,
    pub element_count: usize,
    pub parse_failures: usize,
    pub recovery_count: usize,
    pub unknown_element_histogram: BTreeMap<String, usize>,
    pub files_with_recoveries: Vec<String>,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutSet {
    pub documents: Vec<LayoutDocument>,
    pub stats: GuiParseStats,
}

impl LayoutSet {
    pub fn parse_file(&mut self, path: &Path, rel: &str) -> Result<()> {
        let doc = parse_file(path)?;
        self.add_document(rel, &doc);
        Ok(())
    }

    pub fn add_document(&mut self, source_file: &str, doc: &Document) {
        let roots = parse_gui_document(doc);
        let mut stats = DocumentStats::default();
        for root in &roots {
            collect_stats(root, &mut stats);
        }
        self.stats.file_count += 1;
        self.stats.window_count += stats.window_count;
        self.stats.element_count += stats.element_count;
        self.stats.recovery_count += doc.recoveries.len();
        if !doc.recoveries.is_empty() {
            self.stats
                .files_with_recoveries
                .push(source_file.to_string());
        }
        for (kind, count) in stats.unknown_element_histogram {
            *self
                .stats
                .unknown_element_histogram
                .entry(kind)
                .or_insert(0) += count;
        }
        self.documents.push(LayoutDocument {
            source_file: source_file.to_string(),
            roots,
            recoveries: doc
                .recoveries
                .iter()
                .map(|recovery| {
                    format!("{}:{} {}", recovery.line, recovery.column, recovery.message)
                })
                .collect(),
        });
    }

    pub fn sprite_refs(&self) -> BTreeSet<String> {
        let mut refs = BTreeSet::new();
        for doc in &self.documents {
            for root in &doc.roots {
                root.collect_sprite_refs(&mut refs);
            }
        }
        refs
    }

    pub fn find_by_name<'a>(&'a self, name: &str) -> Vec<&'a GuiElement> {
        let mut out = Vec::new();
        for doc in &self.documents {
            for root in &doc.roots {
                root.find_by_name(name, &mut out);
            }
        }
        out
    }
}

impl GuiElement {
    pub fn collect_sprite_refs(&self, refs: &mut BTreeSet<String>) {
        for sprite in &self.sprite_refs {
            refs.insert(sprite.clone());
        }
        for child in &self.children {
            child.collect_sprite_refs(refs);
        }
    }

    pub fn find_by_name<'a>(&'a self, name: &str, out: &mut Vec<&'a GuiElement>) {
        if self.name.eq_ignore_ascii_case(name) {
            out.push(self);
        }
        for child in &self.children {
            child.find_by_name(name, out);
        }
    }
}

pub fn parse_gui_document(doc: &Document) -> Vec<GuiElement> {
    let mut roots = Vec::new();
    for gui_types in doc.root.values_ci("guiTypes") {
        for entry in gui_types.fields() {
            let Some(kind) = entry.key.as_deref() else {
                continue;
            };
            if let Some(element) = parse_element(kind, &entry.value) {
                roots.push(element);
            }
        }
    }
    roots
}

pub fn parse_element(element_type: &str, value: &Value) -> Option<GuiElement> {
    value.as_object()?;
    let name = value
        .first_scalar_ci("name")
        .map(str::to_string)
        .unwrap_or_else(|| element_type.to_string());
    let position = value
        .first_value_ci("position")
        .and_then(parse_measure_pair);
    let size = value.first_value_ci("size").and_then(parse_measure_pair);
    let orientation = value.first_scalar_ci("Orientation").map(str::to_string);
    let font = value.first_scalar_ci("font").map(str::to_string);
    let text = value.first_scalar_ci("text").map(str::to_string);
    let format = value.first_scalar_ci("format").map(str::to_string);
    let sprite_refs = sprite_refs(value);
    let mut children = Vec::new();
    for entry in value.fields() {
        let Some(child_type) = entry.key.as_deref() else {
            continue;
        };
        if !entry
            .value
            .as_object()
            .is_some_and(|_| is_child_block(child_type))
        {
            continue;
        }
        if let Some(child) = parse_element(child_type, &entry.value) {
            children.push(child);
        }
    }
    Some(GuiElement {
        element_type: element_type.to_string(),
        name,
        position,
        size,
        orientation,
        sprite_refs,
        font,
        text,
        format,
        children,
        raw: value.clone(),
    })
}

pub fn parse_measure_pair(value: &Value) -> Option<MeasurePair> {
    let x = first_scalar_any(value, &["x", "width"])?;
    let y = first_scalar_any(value, &["y", "height"])?;
    Some(MeasurePair {
        x: parse_measure(x),
        y: parse_measure(y),
    })
}

pub fn parse_measure(raw: &str) -> Measure {
    let trimmed = raw.trim();
    let is_percent = trimmed.contains('%');
    let value = trimmed
        .trim_end_matches('%')
        .trim_end_matches('%')
        .parse::<f32>()
        .ok()
        .map(|value| value.to_string());
    Measure {
        raw: trimmed.to_string(),
        value,
        is_percent,
    }
}

pub fn sprite_refs(value: &Value) -> Vec<String> {
    let mut refs = Vec::new();
    for entry in value.fields() {
        let Some(key) = entry.key.as_deref() else {
            continue;
        };
        let folded = key.to_ascii_lowercase();
        if folded != "spritetype"
            && folded != "quadtexturesprite"
            && !folded.ends_with("spritetype")
        {
            continue;
        }
        let Some(sprite) = entry.value.as_scalar() else {
            continue;
        };
        if !sprite.trim().is_empty() {
            refs.push(sprite.to_string());
        }
    }
    refs
}

fn is_child_block(key: &str) -> bool {
    !matches!(
        key.to_ascii_lowercase().as_str(),
        "position"
            | "show_position"
            | "hide_position"
            | "size"
            | "minimumsize"
            | "maximumsize"
            | "border"
            | "margin"
            | "padding"
            | "color"
            | "colortwo"
            | "text_color_code"
            | "offset"
            | "scale"
            | "pdx_tooltip_delayed"
    )
}

fn is_known_element_type(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "windowtype"
            | "containerwindowtype"
            | "icontype"
            | "buttontype"
            | "extendedscrollbartype"
            | "instanttextboxtype"
            | "smoothlistboxtype"
            | "gridboxtype"
            | "guibuttontype"
            | "decreasebutton"
            | "increasebutton"
            | "listboxtype"
            | "slider"
            | "scrollbartype"
            | "track"
    )
}

fn is_window_type(kind: &str) -> bool {
    matches!(
        kind.to_ascii_lowercase().as_str(),
        "windowtype" | "containerwindowtype"
    )
}

#[derive(Default)]
struct DocumentStats {
    window_count: usize,
    element_count: usize,
    unknown_element_histogram: BTreeMap<String, usize>,
}

fn collect_stats(element: &GuiElement, stats: &mut DocumentStats) {
    stats.element_count += 1;
    if is_window_type(&element.element_type) {
        stats.window_count += 1;
    }
    if !is_known_element_type(&element.element_type) {
        *stats
            .unknown_element_histogram
            .entry(element.element_type.clone())
            .or_insert(0) += 1;
    }
    for child in &element.children {
        collect_stats(child, stats);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paradox_text::parse_str;

    #[test]
    fn parses_structured_fields_and_preserves_unknown_children() {
        let doc = parse_str(
            r#"
            guiTypes = {
              containerWindowType = {
                name = "root"
                position = { x = -4 y = 100%% }
                background = {
                  name = "bg"
                  quadTextureSprite = "GFX_bg"
                }
                buttonType = {
                  name = "ok"
                  spriteType = "GFX_ok"
                }
              }
            }
            "#,
        );
        let roots = parse_gui_document(&doc);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].position.as_ref().unwrap().x.raw, "-4");
        assert!(roots[0].position.as_ref().unwrap().y.is_percent);
        assert_eq!(roots[0].children.len(), 2);
        assert_eq!(roots[0].children[0].element_type, "background");
        let mut refs = BTreeSet::new();
        roots[0].collect_sprite_refs(&mut refs);
        assert!(refs.contains("GFX_bg"));
        assert!(refs.contains("GFX_ok"));
    }
}
