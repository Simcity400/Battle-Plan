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

/// Read literal focus-day defines from the recorded stack. Lua expressions are
/// deliberately unavailable: executing mod code is outside this reader's scope.
pub fn point_days(layers: &ResolvedLayers) -> io::Result<Option<f64>> {
    let mut paths = layers
        .paths()
        .filter(|p| p.starts_with("common/defines/") && p.ends_with(".lua"))
        .map(|p| (layers.layer_index(p).unwrap_or(0), p.to_owned()))
        .collect::<Vec<_>>();
    paths.sort();
    let mut days = None;
    for (_, path) in paths {
        let bytes = layers.read(&path)?;
        apply_point_days(&String::from_utf8_lossy(&bytes), &mut days);
    }
    Ok(days)
}

fn apply_point_days(text: &str, days: &mut Option<f64>) {
    for line in lua_code(text).lines() {
        let code = line.trim();
        if !code.contains("FOCUS_POINT_DAYS") {
            continue;
        }
        let Some((key, value)) = code.split_once('=') else {
            *days = None;
            continue;
        };
        let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
        *days = match key.as_str() {
            "FOCUS_POINT_DAYS" | "NDefines.NFocus.FOCUS_POINT_DAYS" => {
                nonnegative_number(value.trim().trim_end_matches([',', ';']).trim())
                    .filter(|n| *n > 0.0)
            }
            _ => None,
        };
    }
}

// Ignore Lua comments and string contents before looking for literal defines.
// This is not a Lua evaluator: expressions still produce an unavailable value.
fn lua_code(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let comment = bytes[at..].starts_with(b"--");
        let start = at + if comment { 2 } else { 0 };
        let long = bytes.get(start) == Some(&b'[');
        let mut delimiter = start + 1;
        if long {
            while bytes.get(delimiter) == Some(&b'=') {
                delimiter += 1;
            }
        }
        if long && bytes.get(delimiter) == Some(&b'[') {
            let mut close = vec![b']'];
            close.extend(std::iter::repeat_n(b'=', delimiter - start - 1));
            close.push(b']');
            at = delimiter + 1;
            while at < bytes.len() && !bytes[at..].starts_with(&close) {
                if bytes[at] == b'\n' {
                    output.push(b'\n');
                }
                at += 1;
            }
            at = (at + close.len()).min(bytes.len());
            output.push(b' ');
        } else if comment {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if matches!(bytes[at], b'\'' | b'"') {
            let quote = bytes[at];
            output.push(quote);
            at += 1;
            while at < bytes.len() && bytes[at] != quote {
                if bytes[at] == b'\\' {
                    at += 1;
                }
                at += 1;
            }
            if at < bytes.len() {
                output.push(quote);
                at += 1;
            }
        } else {
            output.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
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
    fn authored_costs_and_define_overrides() {
        let defs = parse(br#"focus_tree={focus={id=A cost=10} focus={id=B cost=2.5} focus={id=C cost=@scripted} focus={id=D cost=-1}} continuous_focus={id=E}"#).unwrap();
        assert_eq!(defs["A"].cost, Some(10.0));
        assert_eq!(defs["B"].cost, Some(2.5));
        for id in ["C", "D", "E"] {
            assert_eq!(defs[id].cost, None);
        }
        let mut days = None;
        apply_point_days(
            "NFocus = {\n FOCUS_POINT_DAYS = 7, -- each point\n}",
            &mut days,
        );
        assert_eq!(days, Some(7.0));
        apply_point_days(
            "-- FOCUS_POINT_DAYS = 100\nNDefines.NFocus.FOCUS_POINT_DAYS = 5;",
            &mut days,
        );
        assert_eq!(days, Some(5.0));
        apply_point_days("NDefines.NFocus.FOCUS_POINT_DAYS = 7 * 2", &mut days);
        assert_eq!(days, None);
        apply_point_days("NDefines.NFocus.FOCUS_POINT_DAYS = 0", &mut days);
        assert_eq!(days, None);
    }

    #[test]
    fn comments_and_strings_cannot_override_focus_days() {
        let mut days = Some(7.0);
        apply_point_days(
            "--[[\nFOCUS_POINT_DAYS = 99\n]]\n--[=[\nFOCUS_POINT_DAYS = 88\n]=]\nmessage = 'FOCUS_POINT_DAYS = 77'\nmessage = [=[\nFOCUS_POINT_DAYS = 66\n]=]",
            &mut days,
        );
        assert_eq!(days, Some(7.0));
        apply_point_days(
            "--[=[ ignored ]=] NDefines.NFocus.FOCUS_POINT_DAYS = 5 -- override\n-- FOCUS_POINT_DAYS = 4",
            &mut days,
        );
        assert_eq!(days, Some(5.0));
        apply_point_days("NDefines.NFocus.FOCUS_POINT_DAYS = '7'", &mut days);
        assert_eq!(days, None);
    }
}
