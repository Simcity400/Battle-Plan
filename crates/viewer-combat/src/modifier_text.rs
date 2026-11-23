use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::ModifierDefinitionRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
use viewer_runtime::memo::Keyed;
use dashboard_text::table_access::{Available, rows};
use {
    super::fixed::{ONE, mul},
    viewer_foundation::tooltip_text::{TextIcons, builtin, token, token_with},
};

const CLAMP: u32 = 1 << 4;
const SHOW_ZERO: u32 = 1 << 8;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Refusal {
    Tables(String),
    Ambiguous(String),
    Localisation(String),
    NotText(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Tables(reason) => write!(f, "{reason}"),
            Refusal::Ambiguous(name) => write!(
                f,
                "the recorded lexical tokens name the modifier {name} twice in different cases, which the game's token table never holds (folded names are unique), so the recording is inconsistent"
            ),
            Refusal::Localisation(reason) | Refusal::NotText(reason) => f.write_str(reason),
        }
    }
}

pub(crate) const NAME_INDEX_TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::ModifierDefinitions,
    replay_data::rows::TableId::LexicalTokens,
];

pub type NameIndex = Result<BTreeMap<Vec<u8>, Option<u32>>, Refusal>;

pub type NameIndexCache = Keyed<NameIndex>;

pub fn name_index_cache() -> NameIndexCache {
    Keyed::new(NAME_INDEX_TABLES)
}

fn index_names(frame: &DecodedFrame) -> io::Result<NameIndex> {
    let definitions = rows::<tables::ModifierDefinitions>(frame)?;
    let Available::Complete(definitions) = definitions else {
        return Ok(Err(Refusal::Tables(
            "the recorded modifier definition table is incomplete".into(),
        )));
    };
    let tokens = rows::<tables::LexicalTokens>(frame)?;
    let Available::Complete(tokens) = tokens else {
        return Ok(Err(Refusal::Tables(
            "the recorded lexical dictionary is incomplete".into(),
        )));
    };
    let mut by_token: BTreeMap<u32, u32> = BTreeMap::new();
    for row in definitions.iter().filter(|row| row.token != 0) {
        let index = by_token.entry(row.token).or_insert(row.index);
        *index = (*index).max(row.index);
    }
    let mut names = BTreeMap::new();
    for row in definitions
        .iter()
        .filter(|row| row.token != 0 && by_token.get(&row.token) == Some(&row.index))
    {
        let Ok(found) = tokens.binary_search_by_key(&row.token, |token| token.token) else {
            return Ok(Err(Refusal::Tables(format!(
                "modifier definition {}'s token {} is not in the recorded lexical dictionary",
                row.index, row.token
            ))));
        };
        names
            .entry(tokens[found].literal.to_ascii_lowercase())
            .and_modify(|index| *index = None)
            .or_insert(Some(row.index));
    }
    Ok(Ok(names))
}

pub(super) struct Definitions<'frame> {
    rows: &'frame [ModifierDefinitionRow],
    names: Arc<NameIndex>,
    features: Option<u32>,
}

impl<'frame> Definitions<'frame> {
    pub(super) fn read(
        frame: &'frame DecodedFrame,
        cache: &NameIndexCache,
    ) -> io::Result<Result<Self, Refusal>> {
        let names = cache.get_or_build(frame, (), |frame| index_names(frame).map(Arc::new))?;
        if let Err(refusal) = names.as_ref() {
            return Ok(Err(refusal.clone()));
        }
        let Available::Complete(definitions) = rows::<tables::ModifierDefinitions>(frame)? else {
            return Ok(Err(Refusal::Tables(
                "the recorded modifier definition table is incomplete".into(),
            )));
        };
        let features = rows::<tables::SimulationFeatures>(frame)?;
        let features = match features.rows() {
            [feature] if features.is_complete() => Some(feature.dlc_mask),
            _ => None,
        };
        Ok(Ok(Self {
            rows: definitions,
            names,
            features,
        }))
    }

    #[cfg(test)]
    pub(super) fn from_parts(
        rows: &'frame [ModifierDefinitionRow],
        names: &[(&[u8], u32)],
        features: Option<u32>,
    ) -> Self {
        Self {
            rows,
            names: Arc::new(Ok(names
                .iter()
                .map(|&(name, index)| (name.to_ascii_lowercase(), Some(index)))
                .collect())),
            features,
        }
    }

    pub(super) fn identity(&self) -> Arc<NameIndex> {
        Arc::clone(&self.names)
    }

    pub(super) fn settle(
        definitions: Result<&Self, &Refusal>,
        conditions: &[content_reader_key_names::unit_leader::Conditional],
    ) -> Result<(), String> {
        for condition in conditions {
            let registered = definitions
                .map_err(|refusal| {
                    format!("{condition}, and whether it is registered is unknown: {refusal}")
                })?
                .index(&condition.modifier)
                .map_err(|refusal| format!("{condition}, and {refusal}"))?;
            if registered.is_some() {
                return Err(format!(
                    "{condition}, and {} is a recorded modifier definition",
                    condition.modifier
                ));
            }
        }
        Ok(())
    }

    pub(super) fn index(&self, name: &str) -> Result<Option<u32>, Refusal> {
        let Ok(names) = self.names.as_ref() else {
            unreachable!("read refuses a failed index");
        };
        match names.get(name.to_ascii_lowercase().as_bytes()) {
            None => Ok(None),
            Some(Some(index)) => Ok(Some(*index)),
            Some(None) => Err(Refusal::Ambiguous(name.into())),
        }
    }

    fn row(&self, index: u32) -> Result<&ModifierDefinitionRow, Refusal> {
        self.rows
            .binary_search_by_key(&index, |row| row.index)
            .map(|at| &self.rows[at])
            .map_err(|_| {
                Refusal::Tables(format!(
                    "modifier definition {index} has no recorded definition row"
                ))
            })
    }

    fn eligible(&self, row: &ModifierDefinitionRow) -> Result<bool, Refusal> {
        if row.feature_selector == 0 {
            return Ok(true);
        }
        if row.eligibility == 0 {
            return Ok(false);
        }
        let features = self.features.ok_or_else(|| {
            Refusal::Tables(format!(
                "modifier {} ({}) is gated by feature mask {:#x}, which needs the recorded DLC feature mask",
                row.index,
                String::from_utf8_lossy(&row.name_key),
                row.feature_selector
            ))
        })?;
        Ok(features & row.feature_selector != 0)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Block {
    pub(super) values: BTreeMap<u32, i64>,
    pub(super) hidden: BTreeSet<u32>,
    pub(super) custom_tooltips: Vec<Vec<u8>>,
    pub(super) added: bool,
}

impl Block {
    pub(super) fn build(
        definitions: &Definitions<'_>,
        authored: &content_reader_script::modifiers::Modifiers,
    ) -> Result<(Self, Vec<String>), Refusal> {
        let mut block = Self {
            added: !authored.children.is_empty(),
            custom_tooltips: authored
                .tooltip_table()
                .keys()
                .map(|key| key.as_bytes().to_vec())
                .collect(),
            ..Self::default()
        };
        let mut dropped = Vec::new();
        for modifier in &authored.values {
            let Some(index) = definitions.index(&modifier.name)? else {
                dropped.push(modifier.key.clone());
                continue;
            };
            block.values.insert(index, modifier.value);
            if authored.hidden.contains(&modifier.name) {
                block.hidden.insert(index);
            }
        }
        Ok((block, dropped))
    }

    pub(super) fn value(&self, definitions: &Definitions<'_>, index: u32) -> Result<i64, Refusal> {
        let Some(&raw) = self.values.get(&index) else { return Ok(0) };
        let row = definitions.row(index)?;
        if !definitions.eligible(row)? {
            return Ok(0);
        }
        Ok(if row.flags & CLAMP != 0 { raw.clamp(0, ONE) } else { raw })
    }

    pub(super) fn kept(&self, definitions: &Definitions<'_>) -> Result<bool, Refusal> {
        if !self.custom_tooltips.is_empty() || self.added {
            return Ok(true);
        }
        for &index in self.values.keys() {
            if definitions.eligible(definitions.row(index)?)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn has_category_one(&self, definitions: &Definitions<'_>) -> Result<bool, Refusal> {
        for (&index, &raw) in &self.values {
            let row = definitions.row(index)?;
            if row.category_mask & 1 == 0 {
                continue;
            }
            let value = if row.flags & CLAMP != 0 {
                raw.clamp(0, ONE)
            } else {
                raw
            };
            if value != 0 && definitions.eligible(row)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn describe(
        &self,
        definitions: &Definitions<'_>,
        icons: &dyn TextIcons,
        loc: &Localisation,
        scale: i64,
        prefix: &str,
        include_mask: u32,
    ) -> Result<String, Refusal> {
        self.describe_at(definitions, icons, loc, scale, prefix, include_mask, 2)
    }

    #[allow(clippy::too_many_arguments, reason = "the native description options, each its own operand")]
    pub(super) fn describe_at(
        &self,
        definitions: &Definitions<'_>,
        icons: &dyn TextIcons,
        loc: &Localisation,
        scale: i64,
        prefix: &str,
        include_mask: u32,
        precision: i32,
    ) -> Result<String, Refusal> {
        let mut text = String::new();
        let mut marker = 0;
        for (&index, &raw) in &self.values {
            let row = definitions.row(index)?;
            let eligible = definitions.eligible(row)?;
            let value = if !eligible {
                0
            } else if row.flags & CLAMP != 0 {
                raw.clamp(0, ONE)
            } else {
                raw
            };
            let value = mul(value, scale);
            if !((row.flags & SHOW_ZERO != 0 || value != 0)
                && row.category_mask & include_mask != 0
                && eligible)
            {
                continue;
            }
            if self.hidden.contains(&index) {
                continue;
            }
            if text.len() != marker {
                text.push('\n');
            }
            marker = text.len();
            text.push_str(prefix);
            text.push_str(&name(row, icons, loc)?);
            text.push_str(": ");
            text.push_str(&value_text(row, value, precision, false, false, icons, loc)?);
        }
        for key in self.custom_tooltips.iter().filter(|key| !key.is_empty()) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&token(icons, loc, key).map_err(Refusal::Localisation)?);
        }
        Ok(text)
    }
}

fn text<'a>(row: &ModifierDefinitionRow, what: &str, bytes: &'a [u8]) -> Result<&'a str, Refusal> {
    std::str::from_utf8(bytes).map_err(|_| {
        Refusal::NotText(format!(
            "modifier definition {}'s recorded {what} {} is not UTF-8",
            row.index,
            String::from_utf8_lossy(bytes)
        ))
    })
}

fn name(
    row: &ModifierDefinitionRow,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, Refusal> {
    let key = text(row, "name key", &row.name_key)?;
    let argument = text(row, "argument", &row.argument)?;
    let arguments = if argument.is_empty() {
        &[][..]
    } else {
        &[("ARG", Argument::Text(argument))][..]
    };
    token_with(icons, loc, key.as_bytes(), arguments).map_err(Refusal::Localisation)
}

pub(crate) fn value_text(
    row: &ModifierDefinitionRow,
    value: i64,
    precision: i32,
    plus: bool,
    invert: bool,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, Refusal> {
    dashboard_text::modifier_text::value_text(
        row,
        value,
        precision,
        plus,
        invert,
        |key, required| {
            if required {
                let key = std::str::from_utf8(key)
                    .map_err(|_| "modifier value key is not UTF-8".to_owned())?;
                builtin(icons, loc, key, &[])
            } else {
                token(icons, loc, key)
            }
        },
    )
    .map_err(Refusal::Localisation)
}

#[cfg(test)]
mod tests;
