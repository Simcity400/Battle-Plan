//! Native modifier description lines, rebuilt from the recorded definition
//! table and authored content: `modifier_accumulator::FormatDescription`
//! (14054E050) -> 14054F6D0 -> value text 14054FE50, names through
//! `modifier_definition::GetLocalizedName` (1405505D0), numbers through
//! `Fixed5ToString` (1424B6200 -> `format_fixed5` 1424B89F0) and
//! `FormatFixed5Percent` (142259590). Nothing here is recorded text.
use super::{
    binding::{TextIcons, builtin, token, token_with},
    fixed::{ONE, mul},
};
use crate::table_access::{Available, rows};
use installed_content::localisation::{Argument, Localisation};
use replay::{
    frame::DecodedFrame,
    rows::{ModifierDefinitionRow, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

/// 140551820's clamp of the getter to [0, 1].
const CLAMP: u32 = 1 << 4;
/// 14054F6D0 prints the entry even when its value is zero.
const SHOW_ZERO: u32 = 1 << 8;

/// Why a modifier description cannot be rebuilt exactly.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Refusal {
    /// The recorded definition table, lexical dictionary or DLC mask is
    /// missing or lacks the named row, so names cannot be resolved or
    /// eligibility is unknown.
    Tables(String),
    /// Two recorded tokens whose names differ only in case, which the
    /// executable cannot hold: no two static keywords registered by
    /// register_authored_keys (140082CB0 -> 1424AFC10) fold to the same
    /// name, and a dynamic token is added only when no token folds to its
    /// name (1424A65A0 compares with StringView_CompareNoCase 1424B6750).
    /// The recording is inconsistent with the game, so which token the
    /// lexer would find is unknown.
    Ambiguous(String),
    /// A localisation key refused, naming the key and why.
    Localisation(String),
    /// A recorded definition field that is not UTF-8, named.
    NotText(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Tables(reason) => write!(f, "{reason}"),
            Refusal::Ambiguous(name) => write!(
                f,
                "the recorded lexical tokens name the modifier {name} twice in different cases, which the game's token table (1430AECD0) never holds (1424A65A0 adds no token whose folded name it has, and no two static keywords fold alike), so the recording is inconsistent"
            ),
            Refusal::Localisation(reason) | Refusal::NotText(reason) => f.write_str(reason),
        }
    }
}

/// The static tables the name index reads; the index is cached on their
/// identities ([`crate::map::memo::Keyed`]), so a playback hour whose frame
/// shares them reuses it.
pub(crate) const NAME_INDEX_TABLES: &[replay::rows::TableId] = &[
    replay::rows::TableId::ModifierDefinitions,
    replay::rows::TableId::LexicalTokens,
];

/// Script name to definition index (`None`: two recorded tokens fold to the
/// name, which the game never holds), or why the tables cannot give one. Keyed by the ASCII-lowercased
/// name: the lexer finds a token without case (1424A7A90 lowercases a bare
/// token, hashes it with a case-folding FNV-1a and compares with
/// StringView_CompareNoCase 1424B6750 in the token table 1430AECD0), and the
/// recorded dictionary is that table.
pub(crate) type NameIndex = Result<BTreeMap<Vec<u8>, Option<u32>>, Refusal>;

/// The document's cache of [`NameIndex`], built on first use.
pub(crate) type NameIndexCache = crate::map::memo::Keyed<NameIndex>;

pub(crate) fn name_index_cache() -> NameIndexCache {
    // One static pair per branch in practice; two cover a branch switch.
    crate::map::memo::Keyed::new(NAME_INDEX_TABLES, 2)
}

/// Indexes the definitions by script name through the lexical dictionary.
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
    // Only registered rows are recorded (the native table keeps a default gap
    // at 0x29A before the dynamic keys 1405FF750 appends), so a definition is
    // found by its recorded index, never by its position.
    //
    // 1406042A0 maps the name's token to a definition through the token map
    // (14054FDB0), which every modifier_definition::Register (14054A990)
    // assigns through operator[] (140548A80): the last registration of a
    // token wins. The static registrar 140552CB0 registers each of its 667
    // token ids under one index only, before any content, and
    // FindOrRegisterDynamicModifierDefinition (1405FF750) registers a token
    // only when it maps to nothing yet, at the next index past 0x29A. Rows
    // sharing a token therefore keep the highest index, the last registered.
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
        // LexicalTokens rows are sorted by token.
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

/// The recorded definition table, keyed by script name through the recorded
/// lexical dictionary.
pub(super) struct Definitions<'frame> {
    rows: &'frame [ModifierDefinitionRow],
    names: std::sync::Arc<NameIndex>,
    /// The simulation's feature flags, when a gated definition needs them.
    features: Option<u32>,
}

impl<'frame> Definitions<'frame> {
    /// The hour's definitions, with the name index from `cache`.
    pub(super) fn read(
        frame: &'frame DecodedFrame,
        cache: &NameIndexCache,
    ) -> io::Result<Result<Self, Refusal>> {
        let names = cache.get_or_build(frame, (), |frame| {
            index_names(frame).map(std::sync::Arc::new)
        })?;
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
            names: std::sync::Arc::new(Ok(names
                .iter()
                .map(|&(name, index)| (name.to_ascii_lowercase(), Some(index)))
                .collect())),
            features,
        }
    }

    /// The name index these definitions use, as a strong reference: a cache
    /// keyed by it compares with [`std::sync::Arc::ptr_eq`] and, holding
    /// it, keeps the allocation alive, so no later index can take its
    /// address.
    pub(super) fn identity(&self) -> std::sync::Arc<NameIndex> {
        std::sync::Arc::clone(&self.names)
    }

    /// Settles each [`Conditional`](installed_content::unit_leader::Conditional):
    /// a name no definition carries was skipped with its block, a registered
    /// one left the content after it out of step. `Err` names the condition
    /// and why it holds or cannot be settled.
    pub(super) fn settle(
        definitions: Result<&Self, &Refusal>,
        conditions: &[installed_content::unit_leader::Conditional],
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

    /// `Ok(None)`: no definition has the name, so the native reader
    /// (1406042A0) reports it and drops it. The name folds as the lexer
    /// folds it ([`NameIndex`]).
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

    /// The definition recorded at native index `index`. The rows are sorted
    /// by index (the table's key) and sparse.
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

    /// `modifier_definition::IsEligible`: an unselected feature is always
    /// eligible; a null callback makes every selected feature ineligible.
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

/// One described block: raw values by definition index (ascending, as the
/// accumulator's sorted array), the hidden set and custom tooltip keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Block {
    pub(super) values: BTreeMap<u32, i64>,
    pub(super) hidden: BTreeSet<u32>,
    /// The custom tooltip keys (+0x68), listed in their slot order.
    pub(super) custom_tooltips: Vec<Vec<u8>>,
    /// An `added_modifier` child.
    pub(super) added: bool,
}

impl Block {
    /// The block the reader built from authored entries
    /// ([`installed_content::modifiers::Modifiers`], 1406031A0: one entry per
    /// modifier in source order, its last value, hidden by name); names
    /// without a definition are returned, dropped as the game drops them.
    pub(super) fn build(
        definitions: &Definitions<'_>,
        authored: &installed_content::modifiers::Modifiers,
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

    /// Whether 1410079E0 keeps a sub-unit's block: custom tooltip keys, a
    /// child, or an entry whose definition is eligible.
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

    /// Weather-event entries in the naval window use 140552B00 with
    /// include mask 1 and exclude mask 0 (141D8A3B0). It scans every
    /// accumulator entry, including hidden ones, for a nonzero effective
    /// value whose definition category mask (+0x64) includes bit 0 and
    /// whose feature selector (+0x68) is eligible (140551820).
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

    /// 14054E050 with the trait description's arguments: precision 2, no
    /// colour, "\n" separators, `include_mask`, `scale`, and `prefix` before
    /// each line's name. 14054F6D0 tests the definition's +0x64 category.
    pub(super) fn describe(
        &self,
        definitions: &Definitions<'_>,
        icons: &dyn TextIcons,
        loc: &Localisation,
        scale: i64,
        prefix: &str,
        include_mask: u32,
    ) -> Result<String, Refusal> {
        let mut text = String::new();
        let mut marker = 0;
        for (&index, &raw) in &self.values {
            let row = definitions.row(index)?;
            let eligible = definitions.eligible(row)?;
            // 140551820: ineligible reads zero, bit 4 clamps to [0, 1].
            let value = if !eligible {
                0
            } else if row.flags & CLAMP != 0 {
                raw.clamp(0, ONE)
            } else {
                raw
            };
            let value = mul(value, scale);
            // 14054F6D0: shown or nonzero, in an included category that
            // no excluded mask (none) removes, and eligible.
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
            text.push_str(&value_text(row, value, 2, false, false, icons, loc)?);
        }
        // The key set in slot order; 14054E050 skips an empty key.
        for key in self.custom_tooltips.iter().filter(|key| !key.is_empty()) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&token(icons, loc, key).map_err(Refusal::Localisation)?);
        }
        Ok(text)
    }
}

/// A recorded definition field as text; `what` names the field.
fn text<'a>(row: &ModifierDefinitionRow, what: &str, bytes: &'a [u8]) -> Result<&'a str, Refusal> {
    std::str::from_utf8(bytes).map_err(|_| {
        Refusal::NotText(format!(
            "modifier definition {}'s recorded {what} {} is not UTF-8",
            row.index,
            String::from_utf8_lossy(bytes)
        ))
    })
}

/// 1405505D0: the name key, with `$ARG$` when the definition stores one.
/// A key the content lacks shows as itself (142230940).
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

/// 14054FE50: sign, number or yes/no, colour and postfix. `precision` is
/// the caller's cap; `plus` forces a sign; `invert` flips good and bad.
pub(super) fn value_text(
    row: &ModifierDefinitionRow,
    value: i64,
    precision: i32,
    plus: bool,
    invert: bool,
    icons: &dyn TextIcons,
    loc: &Localisation,
) -> Result<String, Refusal> {
    dashboard::modifier_text::value_text(row, value, precision, plus, invert, |key, required| {
        if required {
            let key = std::str::from_utf8(key)
                .map_err(|_| "modifier value key is not UTF-8".to_owned())?;
            builtin(icons, loc, key, &[])
        } else {
            token(icons, loc, key)
        }
    })
    .map_err(Refusal::Localisation)
}

#[cfg(test)]
mod tests;
