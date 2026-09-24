//! Focus presentation definitions, using the shared Clausewitz and layer readers.
use crate::{
    ResolvedLayers,
    parser::{self, Item},
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FocusDefinition {
    pub localisation: String,
    pub description: String,
    /// Authored points before saved progress or country-specific modifiers.
    pub cost: Option<f64>,
    /// Conditional icon scripts need game evaluation and remain unavailable.
    pub icon: Option<String>,
}

fn scalar<'a>(items: &'a [Item], key: &[u8]) -> Option<&'a str> {
    items
        .iter()
        .rev()
        .find(|i| i.key.as_deref() == Some(key))?
        .value
        .scalar_str()
}

pub fn parse(input: &[u8]) -> io::Result<BTreeMap<String, FocusDefinition>> {
    fn collect(items: &[Item], output: &mut BTreeMap<String, FocusDefinition>) {
        for item in items {
            let Some(children) = item.value.items() else {
                continue;
            };
            match item.key.as_deref() {
                Some(b"focus" | b"shared_focus" | b"joint_focus" | b"continuous_focus") => {
                    if let Some(id) = scalar(children, b"id") {
                        let localisation = scalar(children, b"text").unwrap_or(id);
                        output.insert(
                            id.to_owned(),
                            FocusDefinition {
                                localisation: localisation.to_owned(),
                                description: scalar(children, b"desc")
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| format!("{localisation}_desc")),
                                cost: scalar(children, b"cost").and_then(nonnegative_number),
                                icon: scalar(children, b"icon").map(str::to_owned),
                            },
                        );
                    }
                }
                Some(b"focus_tree" | b"continuous_focus_palette") => collect(children, output),
                _ => {}
            }
        }
    }
    let mut result = BTreeMap::new();
    collect(&parser::parse(input)?.items, &mut result);
    Ok(result)
}

fn nonnegative_number(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.0)
}

/// `NDefines.NFocus.FOCUS_POINT_DAYS` from the evaluated defines. Content
/// the defines reader refuses, or a missing or non-positive value, leaves the
/// days unavailable; file errors still fail.
pub fn point_days(layers: &ResolvedLayers) -> io::Result<Option<f64>> {
    let defines = match crate::defines::read(layers) {
        Ok(defines) => defines,
        Err(error) if error.kind() == io::ErrorKind::InvalidData => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(defines
        .number("NFocus", "FOCUS_POINT_DAYS")
        .ok()
        .filter(|days| days.is_finite() && *days > 0.0))
}

pub fn read(
    layers: &ResolvedLayers,
) -> io::Result<(BTreeMap<String, FocusDefinition>, Vec<String>)> {
    let mut paths = layers
        .paths()
        .filter(|p| {
            (p.starts_with("common/national_focus/") || p.starts_with("common/continuous_focus/"))
                && p.ends_with(".txt")
        })
        .map(|p| (layers.layer_index(p).unwrap_or(0), p.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    let mut result = BTreeMap::new();
    let mut warnings = Vec::new();
    for (_, path) in paths {
        match parse(&layers.read(&path)?) {
            Ok(definitions) => result.extend(definitions),
            Err(error) => {
                warnings.push(format!("Focus definitions unavailable in {path}: {error}"))
            }
        }
    }
    Ok((result, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authored_text_shared_and_conditional_icons() {
        let defs = parse(br#"focus_tree={focus={id=A icon=GFX_goal_a text=TITLE} focus={id=B icon={value=GFX_goal_b}}} shared_focus={id=C icon=GFX_goal_c}"#).unwrap();
        assert_eq!(defs["A"].localisation, "TITLE");
        assert_eq!(defs["A"].description, "TITLE_desc");
        assert_eq!(defs["A"].icon.as_deref(), Some("GFX_goal_a"));
        assert_eq!(defs["B"].icon, None);
        assert_eq!(defs["C"].localisation, "C");
    }

    #[test]
    fn explicit_description_overrides_the_authored_text_key() {
        let defs = parse(br#"focus_tree={focus={id=A text=TITLE desc=DESCRIPTION}}"#).unwrap();
        assert_eq!(defs["A"].description, "DESCRIPTION");
    }

    #[test]
    fn authored_costs_and_scripted_costs() {
        let defs = parse(br#"focus_tree={focus={id=A cost=10} focus={id=B cost=2.5} focus={id=C cost=@scripted} focus={id=D cost=-1}} continuous_focus={id=E}"#).unwrap();
        assert_eq!(defs["A"].cost, Some(10.0));
        assert_eq!(defs["B"].cost, Some(2.5));
        for id in ["C", "D", "E"] {
            assert_eq!(defs[id].cost, None);
        }
    }
}
