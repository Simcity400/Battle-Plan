//! Tooltip-only fields. Repeated formulas append modifiers; repeated scalar
//! fields replace the previous value.
//!
//! A term's key selects its class: `base` sets the formula's base, a standard
//! name ([`STANDARD`]) retains its folded token name, and every other key,
//! including a quoted one, creates a custom modifier. A custom modifier is
//! kept only when its `formula` contains a modifier; otherwise the game
//! records an error and drops it.

use crate::{fields, parser::Item};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub days_to_prepare: i32,
    pub target_loc_key: String,
    pub success: Formula,
    pub refused: Option<String>,
    /// Each term the game records an error for and drops.
    pub misreads: Vec<String>,
}

/// The folded bare keys that select standard success modifiers.
pub const STANDARD: [&str; 15] = [
    "strategic_bomber",
    "air_defence",
    "strength",
    "interception",
    "experience",
    "organisation",
    "anti_air",
    "recon",
    "intel",
    "radar",
    "air_agility",
    "air_superiority",
    "reliability",
    "resistance",
    "enemy_units",
];

/// The standard class a folded bare key selects, by its token's name.
pub(super) fn standard(folded: &[u8]) -> Option<&'static str> {
    STANDARD.into_iter().find(|name| name.as_bytes() == folded)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Formula {
    pub base: i64,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifier {
    /// A standard modifier's token name (lower case, as registered), or a
    /// custom modifier's key as authored.
    pub name: String,
    pub weight: i64,
    pub reference: i64,
    pub start_weight: i64,
    pub start_reference: i64,
    /// A custom modifier's own fields (`formula`, `scope`, `enable`, ...),
    /// retained for the evaluator: empty exactly for a standard modifier, as
    /// a kept custom modifier always has a `formula`.
    pub custom: Vec<Item>,
}

impl Modifier {
    fn new(name: String) -> Self {
        // Both standard and custom modifiers share these constructor defaults.
        Self {
            name,
            weight: 0,
            reference: 100_000,
            start_weight: 0,
            start_reference: 0,
            custom: Vec::new(),
        }
    }
}

pub(super) fn read(items: &[Item]) -> Definition {
    let mut out = Definition::default();
    if let Err(error) = read_into(items, &mut out) {
        out.refused = Some(error.to_string());
    }
    out
}

fn read_into(items: &[Item], out: &mut Definition) -> std::io::Result<()> {
    for item in items {
        match item
            .keyword()
            .map(crate::parser::Keyword::folded)
            .as_deref()
        {
            // Integer and fixed5 fields keep their slots when conversion fails.
            Some(b"days_to_prepare") => super::integer(&item.value, &mut out.days_to_prepare),
            Some(b"target_loc_key") => {
                out.target_loc_key = super::text(&item.value, "target_loc_key")?.to_owned()
            }
            Some(b"success_factors") => {
                for success in super::all(fields::block(&item.value, "success_factors")?, "success")
                {
                    for term in fields::block(success, "success_factors.success")? {
                        let name = fields::key_str(term, "success formula")?;
                        if term.keyword_is("base") {
                            super::fixed(&term.value, &mut out.success.base);
                            continue;
                        }
                        let class = term.keyword().and_then(|key| standard(&key.folded()));
                        let body = fields::block(&term.value, name)?;
                        if class.is_none()
                            && !body
                                .iter()
                                .filter(|field| field.keyword_is("formula"))
                                .filter_map(|field| field.value.items())
                                .flatten()
                                .any(|entry| entry.keyword_is("modifier"))
                        {
                            out.misreads.push(format!(
                                "success formula {name:?}: a custom modifier whose formula has no modifier is recorded as an error and dropped"
                            ));
                            continue;
                        }
                        let mut modifier = Modifier::new(class.unwrap_or(name).to_owned());
                        for field in body {
                            let slot = match field
                                .keyword()
                                .map(crate::parser::Keyword::folded)
                                .as_deref()
                            {
                                Some(b"weight") => &mut modifier.weight,
                                Some(b"reference") => &mut modifier.reference,
                                Some(b"start_weight") => &mut modifier.start_weight,
                                Some(b"start_reference") => &mut modifier.start_reference,
                                // A standard modifier reads only the four weights.
                                _ if class.is_some() => continue,
                                _ => {
                                    modifier.custom.push(field.clone());
                                    continue;
                                }
                            };
                            super::fixed(&field.value, slot);
                        }
                        out.success.modifiers.push(modifier);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_formulas_preserve_modifier_order_and_defaults() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        let got = tooltip(
            br#"days_to_prepare = 4 target_loc_key = "some_target"
            success_factors = { success = { base = 0.4 STRENGTH = { weight = -0.1 ignored = 1 } } }
            success_factors = { success = { base = 0.7 Beta = { formula = { base = 3 modifier = { always = yes } } } } }"#,
        );
        assert_eq!(got.refused, None);
        assert_eq!(got.misreads, Vec::<String>::new());
        assert_eq!(got.days_to_prepare, 4);
        assert_eq!(got.target_loc_key, "some_target");
        assert_eq!(got.success.base, 70_000);
        assert_eq!(
            got.success
                .modifiers
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            // A standard class keeps its token; a custom its authored key.
            ["strength", "Beta"]
        );
        assert_eq!(got.success.modifiers[0].reference, 100_000);
        assert_eq!(got.success.modifiers[0].weight, -10_000);
        assert!(got.success.modifiers[0].custom.is_empty());
        assert_eq!(got.success.modifiers[1].custom.len(), 1);
    }

    #[test]
    fn an_unconfigured_custom_modifier_is_dropped() {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
        // A quoted key is looked up as no token: a custom class, even
        // `strength`.
        let got = tooltip(
            br#"success_factors = { success = {
                "strength" = { weight = 0.3 }
                custom = { formula = { base = 1 } weight = 0.2 }
                radar = { weight = 0.1 }
            } }"#,
        );
        assert_eq!(got.refused, None);
        assert_eq!(
            got.success
                .modifiers
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            ["radar"]
        );
        assert_eq!(got.misreads.len(), 2);
        assert!(
            got.misreads[0].contains("\"strength\""),
            "{:?}",
            got.misreads
        );
    }

    fn tooltip(body: &[u8]) -> Definition {
        let text = [&b"types = { t = { "[..], body, b" } }"].concat();
        let doc = super::super::reader::document(
            &text,
            &|_| Some(false),
            &crate::key_names::KeyNames::default(),
            &mut Vec::new(),
        )
        .unwrap();
        let raid = doc.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        read(raid)
    }
}
