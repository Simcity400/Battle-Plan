
use crate::rows::{LexicalTokenRow, ModifierDefinitionRow, column, mask, tables};
use replay_frame::frame::{Rowset, Tables};
use std::{borrow::Cow, collections::BTreeMap};

pub const CLAMP_MAXIMUM: u32 = engine_native_parts::modifier_definition::CLAMP_MAXIMUM;

pub const DEFINITION: u64 =
    mask::<ModifierDefinitionRow>(&["token", "flags", "feature_selector", "eligibility"]);
const LITERAL: usize = column::<LexicalTokenRow>("literal");
const DLC_MASK: u64 = mask::<crate::rows::SimulationFeatureRow>(&["dlc_mask"]);

pub struct Registry<'a> {
    definitions: Vec<Cow<'a, ModifierDefinitionRow>>,
    by_name: BTreeMap<Vec<u8>, usize>,
    dlc_mask: Option<u32>,
}

impl<'a> Registry<'a> {
    pub fn new<'t>(
        definitions: &'a [ModifierDefinitionRow],
        tokens: impl Into<replay_frame::frame::Rows<'t, LexicalTokenRow>>,
        dlc_mask: Option<u32>,
    ) -> Result<Self, String> {
        let literals: BTreeMap<u32, &[u8]> = tokens
            .into()
            .iter()
            .map(|row| (row.token, row.literal.as_slice()))
            .collect();
        Self::joined(
            definitions.iter().map(|row| Ok(Cow::Borrowed(row))),
            |token| Ok(literals.get(&token).copied()),
            dlc_mask,
        )
    }

    fn joined<'t>(
        rows: impl Iterator<Item = Result<Cow<'a, ModifierDefinitionRow>, String>>,
        literal: impl Fn(u32) -> Result<Option<&'t [u8]>, String>,
        dlc_mask: Option<u32>,
    ) -> Result<Self, String> {
        let mut entries: BTreeMap<u32, Cow<'a, ModifierDefinitionRow>> = BTreeMap::new();
        let mut names = Vec::new();
        for definition in rows {
            let definition = definition?;
            if definition.token != 0 {
                let literal = literal(definition.token)?.ok_or_else(|| {
                    format!(
                        "modifier definition {} has no lexical token {}",
                        definition.index, definition.token
                    )
                })?;
                names.push((literal.to_ascii_lowercase(), definition.index));
            }
            entries.insert(definition.index, definition);
        }
        let places: BTreeMap<u32, usize> =
            entries.keys().enumerate().map(|(at, &index)| (index, at)).collect();
        let by_name = names
            .into_iter()
            .map(|(name, index)| (name, places[&index]))
            .collect();
        Ok(Self {
            definitions: entries.into_values().collect(),
            by_name,
            dlc_mask,
        })
    }

    pub fn from_frame(frame: &'a dyn replay_frame::frame::DecodedSource) -> Result<Self, String> {
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| e.to_string())?;
        let tokens = frame.lexical().map_err(|e| e.to_string())?;
        let dlc_mask = frame
            .complete::<tables::SimulationFeatures>()
            .ok()
            .and_then(|rows| rows.first())
            .map(|row| row.dlc_mask);
        Self::new(definitions, tokens, dlc_mask)
    }

    pub fn from_tables<F: Tables + ?Sized>(frame: &'a F, fields: u64) -> Result<Self, String> {
        let definitions = frame
            .complete_rows::<tables::ModifierDefinitions>(fields | DEFINITION)
            .map_err(|e| e.to_string())?;
        let tokens: Rowset<'a, LexicalTokenRow> = frame
            .complete_rows::<tables::LexicalTokens>(0)
            .map_err(|e| e.to_string())?;
        let dlc_mask = frame
            .complete_rows::<tables::SimulationFeatures>(DLC_MASK)
            .ok()
            .and_then(|rows| rows.iter().next())
            .and_then(Result::ok)
            .map(|row| row.dlc_mask);
        Self::joined(
            definitions.iter().map(|row| row.map_err(|e| e.to_string())),
            |token| {
                tokens
                    .text(u128::from(token), |row| &row.literal, LITERAL)
                    .map_err(|e| e.to_string())
            },
            dlc_mask,
        )
    }

    pub fn named(&self, name: &str) -> Result<&ModifierDefinitionRow, String> {
        self.by_name
            .get(name.as_bytes().to_ascii_lowercase().as_slice())
            .map(|&at| &*self.definitions[at])
            .ok_or_else(|| format!("modifier {name} has no recorded definition"))
    }

    pub fn indexed(&self, index: u32) -> Result<&ModifierDefinitionRow, String> {
        self.definitions
            .binary_search_by_key(&index, |row| row.index)
            .map(|at| &*self.definitions[at])
            .map_err(|_| format!("modifier definition {index} is missing"))
    }

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

    pub fn effective(&self, definition: &ModifierDefinitionRow, raw: i64) -> Result<i64, String> {
        Ok(self.rule(definition)?.of(raw))
    }

    pub fn rule(&self, definition: &ModifierDefinitionRow) -> Result<Effective, String> {
        Ok(if !self.eligible(definition)? {
            Effective::Zero
        } else if definition.flags & CLAMP_MAXIMUM != 0 {
            Effective::Clamped
        } else {
            Effective::Stored
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effective {
    Zero,
    Clamped,
    Stored,
}

impl Effective {
    pub fn of(self, raw: i64) -> i64 {
        match self {
            Self::Zero => 0,
            Self::Clamped => raw.clamp(0, 100_000),
            Self::Stored => raw,
        }
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
