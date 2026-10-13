//! The recording's modifier definitions by script name, with the native rules playback
//! applies to modifier entries the content stores: the eligibility shared by the
//! emptiness test and effective-value getter, and the getter's clamp. Each definition
//! joins its lexical token's literal (ModifierDefinitions, LexicalTokens); the
//! eligibility callback's flags are the hour's `SimulationFeatures.dlc_mask`.

use crate::{
    frame::DecodedFrame,
    rows::{LexicalTokenRow, ModifierDefinitionRow, tables},
};
use std::collections::BTreeMap;

/// `engine::modifier_definition::CLAMP_MAXIMUM`: the getter clamps an eligible value to
/// [0, 100000].
const CLAMP_MAXIMUM: u32 = engine::modifier_definition::CLAMP_MAXIMUM;

pub struct Registry<'a> {
    by_name: BTreeMap<Vec<u8>, &'a ModifierDefinitionRow>,
    by_index: BTreeMap<u32, &'a ModifierDefinitionRow>,
    /// `SimulationFeatures.dlc_mask`: flags returned by the authenticated eligibility
    /// callback; `None` when the hour did not record it.
    dlc_mask: Option<u32>,
}

impl<'a> Registry<'a> {
    /// Joins each definition to its lexical token's literal. A definition
    /// whose token the recording does not hold is refused by name.
    pub fn new(
        definitions: &'a [ModifierDefinitionRow],
        tokens: &[LexicalTokenRow],
        dlc_mask: Option<u32>,
    ) -> Result<Self, String> {
        let literals: BTreeMap<u32, &[u8]> = tokens
            .iter()
            .map(|row| (row.token, row.literal.as_slice()))
            .collect();
        let mut by_name = BTreeMap::new();
        let mut by_index = BTreeMap::new();
        for definition in definitions {
            by_index.insert(definition.index, definition);
            if definition.token == 0 {
                continue;
            }
            let literal = literals.get(&definition.token).ok_or_else(|| {
                format!(
                    "modifier definition {} has no lexical token {}",
                    definition.index, definition.token
                )
            })?;
            by_name.insert(literal.to_ascii_lowercase(), definition);
        }
        Ok(Self {
            by_name,
            by_index,
            dlc_mask,
        })
    }

    /// The frame's complete ModifierDefinitions and LexicalTokens, and its
    /// SimulationFeatures flags when recorded.
    pub fn from_frame(frame: &'a DecodedFrame) -> Result<Self, String> {
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| e.to_string())?;
        let tokens = frame
            .complete::<tables::LexicalTokens>()
            .map_err(|e| e.to_string())?;
        let dlc_mask = frame
            .complete::<tables::SimulationFeatures>()
            .ok()
            .and_then(|rows| rows.first())
            .map(|row| row.dlc_mask);
        Self::new(definitions, tokens, dlc_mask)
    }

    /// The definition a content-stored entry names (its script name, which
    /// the lexer folds).
    pub fn named(&self, name: &str) -> Result<&'a ModifierDefinitionRow, String> {
        self.by_name
            .get(name.as_bytes().to_ascii_lowercase().as_slice())
            .copied()
            .ok_or_else(|| format!("modifier {name} has no recorded definition"))
    }

    pub fn indexed(&self, index: u32) -> Result<&'a ModifierDefinitionRow, String> {
        self.by_index
            .get(&index)
            .copied()
            .ok_or_else(|| format!("modifier definition {index} is missing"))
    }

    /// Eligibility used by the emptiness test and effective-value getter: a zero feature selector, or a
    /// selector the callback's flags meet. A null callback (`eligibility`
    /// 0) makes every selected feature ineligible.
    pub fn eligible(&self, definition: &ModifierDefinitionRow) -> Result<bool, String> {
        if definition.feature_selector == 0 {
            return Ok(true);
        }
        if definition.eligibility == 0 {
            return Ok(false);
        }
        let flags = self.dlc_mask.ok_or_else(|| {
            format!(
                "modifier {} is feature-gated and this hour's SimulationFeatures.dlc_mask is unavailable",
                definition.index
            )
        })?;
        Ok(definition.feature_selector & flags != 0)
    }

    /// Effective value of a present entry: zero for an ineligible key, the stored value
    /// clamped to [0, 100000] under `CLAMP_MAXIMUM`, else the stored value. An absent
    /// entry is zero without consulting the definition.
    pub fn effective(&self, definition: &ModifierDefinitionRow, raw: i64) -> Result<i64, String> {
        Ok(if !self.eligible(definition)? {
            0
        } else if definition.flags & CLAMP_MAXIMUM != 0 {
            raw.clamp(0, 100_000)
        } else {
            raw
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_and_clamp_follow_the_getter() {
        let definitions = [
            ModifierDefinitionRow {
                index: 1,
                token: 10,
                flags: CLAMP_MAXIMUM,
                ..Default::default()
            },
            ModifierDefinitionRow {
                index: 2,
                token: 20,
                feature_selector: 4,
                eligibility: 1,
                ..Default::default()
            },
            ModifierDefinitionRow {
                index: 3,
                token: 30,
                feature_selector: 4,
                eligibility: 0,
                ..Default::default()
            },
        ];
        let tokens = [
            LexicalTokenRow {
                token: 10,
                literal: b"Clamped".to_vec(),
            },
            LexicalTokenRow {
                token: 20,
                literal: b"gated".to_vec(),
            },
            LexicalTokenRow {
                token: 30,
                literal: b"null_callback".to_vec(),
            },
        ];
        let met = Registry::new(&definitions, &tokens, Some(4)).unwrap();
        let clamped = met.named("clamped").unwrap();
        assert_eq!(met.effective(clamped, 250_000), Ok(100_000));
        assert_eq!(met.effective(clamped, -5), Ok(0));
        assert_eq!(met.effective(met.named("gated").unwrap(), -5), Ok(-5));
        assert_eq!(met.effective(met.named("null_callback").unwrap(), 7), Ok(0));
        let unmet = Registry::new(&definitions, &tokens, Some(1)).unwrap();
        assert_eq!(unmet.effective(unmet.named("gated").unwrap(), 7), Ok(0));
        let unknown = Registry::new(&definitions, &tokens, None).unwrap();
        assert!(
            unknown
                .effective(unknown.named("gated").unwrap(), 7)
                .is_err()
        );
        assert!(unknown.named("absent").is_err());
        assert!(Registry::new(&definitions, &tokens[..1], None).is_err());
    }
}
