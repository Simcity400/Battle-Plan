//! Tooltip-only fields. Readers: `CRaidType::ReadToken@140A8FA30`,
//! `CRaidSuccessFactors::vf4@14157F860`, formula `141578090/14157F4D0`
//! and modifier `14157F800/14157F2A0`. Repeated formulas append modifiers;
//! repeated scalar fields replace the previous value.

use crate::{fields, parser::Item};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Definition {
    pub days_to_prepare: i32,
    pub target_loc_key: String,
    pub success: Formula,
    pub refused: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Formula {
    pub base: i64,
    pub modifiers: Vec<Modifier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modifier {
    pub name: String,
    pub weight: i64,
    pub reference: i64,
    pub start_weight: i64,
    pub start_reference: i64,
    /// Custom expression fields are retained, not treated as standard
    /// modifiers with a zero value. Their scopes belong to the evaluator.
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
        match item.key.as_deref() {
            Some(b"days_to_prepare") => {
                out.days_to_prepare = fields::integer(&item.value, "days_to_prepare")?
            }
            Some(b"target_loc_key") => {
                out.target_loc_key = fields::text(&item.value, "target_loc_key")?.to_owned()
            }
            Some(b"success_factors") => {
                for success in
                    fields::all(fields::block(&item.value, "success_factors")?, b"success")
                {
                    for term in fields::block(success, "success_factors.success")? {
                        let name = fields::key_str(term, "success formula")?;
                        if name == "base" {
                            out.success.base = super::fixed(&term.value, "success base")?;
                            continue;
                        }
                        let mut modifier = Modifier::new(name.to_owned());
                        for field in fields::block(&term.value, name)? {
                            let slot = match field.key.as_deref() {
                                Some(b"weight") => &mut modifier.weight,
                                Some(b"reference") => &mut modifier.reference,
                                Some(b"start_weight") => &mut modifier.start_weight,
                                Some(b"start_reference") => &mut modifier.start_reference,
                                _ => {
                                    modifier.custom.push(field.clone());
                                    continue;
                                }
                            };
                            *slot = super::fixed(&field.value, name)?;
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
        let doc = crate::parser::parse(
            br#"
            days_to_prepare = 4 target_loc_key = "some_target"
            success_factors = { success = { base = 0.4 alpha = { weight = -0.1 } } }
            success_factors = { success = { base = 0.7 beta = { formula = { base = 3 } } } }
        "#,
        )
        .unwrap();
        let got = read(&doc.items);
        assert_eq!(got.refused, None);
        assert_eq!(got.days_to_prepare, 4);
        assert_eq!(got.target_loc_key, "some_target");
        assert_eq!(got.success.base, 70_000);
        assert_eq!(
            got.success
                .modifiers
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "beta"]
        );
        assert_eq!(got.success.modifiers[0].reference, 100_000);
        assert_eq!(got.success.modifiers[0].weight, -10_000);
        assert_eq!(got.success.modifiers[1].custom.len(), 1);
    }
}
