//! Tooltip-only fields. Readers: `CRaidType::ReadToken@140A8FA30`,
//! `CRaidSuccessFactors::vf4@14157F860`, formula `141578090/14157F4D0`
//! and modifier `14157F800/14157F2A0`. Repeated formulas append modifiers;
//! repeated scalar fields replace the previous value.
//!
//! 14157F4D0 picks a term's class by its key's token: `base` is the
//! formula's base, a standard name ([`STANDARD`]) a
//! CRaidSuccessChanceStandardModifier holding that token, and any other key,
//! a quoted one included, a CRaidSuccessChanceCustomModifier. That is kept
//! only when IsConfigured (14157E390) finds a modifier in its `formula` (the
//! CMeanTimeToHappen at +0x38 counts them at +0x2C); otherwise the game
//! records an error (1424ACD40) and drops it.

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

/// The keys 14157F4D0 gives a standard modifier class, by token (names
/// from register_authored_keys 140082CB0): strategic_bomber 0x2FC5,
/// air_defence 0x2EB6, strength 0x28A6, interception 0x2B32, experience
/// 0x2E9A, organisation 0x2ECB, anti_air 0x2F86, recon 0x2FA4, intel
/// 0x30EC, radar 0x2FCD, air_agility 0x2FCE, air_superiority 0x302F,
/// reliability 0x317C, resistance 0x32E1, enemy_units 0x40D7.
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
        // Constructor in `14157F4D0`, both standard and custom classes.
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
            // ReadScriptSignedInt32 and ReadFixed5 keep a slot whose
            // conversion fails.
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
                                "success formula {name:?}: a custom modifier whose formula has no modifier is recorded as an error and dropped (14157E390)"
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
                                // 14157F800 reads only the four weights.
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
    fn repeated_formulas_preserve_modifier_order_and_native_defaults() {
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
        let doc = super::super::reader::document(&text, &|_| Some(false), &mut Vec::new()).unwrap();
        let raid = doc.items[0].value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        read(raid)
    }
}
