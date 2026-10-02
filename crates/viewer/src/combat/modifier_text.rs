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
use installed_content::{
    localisation::{Argument, Localisation},
    modifiers::TooltipTable,
};
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
    /// The custom tooltip set (+0x68), listed in its slot order.
    pub(super) custom_tooltips: TooltipTable,
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
            custom_tooltips: authored.tooltip_table().clone(),
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

    /// 14054E050 with the trait description's arguments: precision 2, no
    /// colour, "\n" separators, every category, `scale`, and `prefix` before
    /// each line's name.
    pub(super) fn describe(
        &self,
        definitions: &Definitions<'_>,
        icons: &dyn TextIcons,
        loc: &Localisation,
        scale: i64,
        prefix: &str,
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
            // 14054F6D0: shown or nonzero, in an included category (all of
            // them) that no excluded mask (none) removes, and eligible.
            if !((row.flags & SHOW_ZERO != 0 || value != 0) && row.category_mask != 0 && eligible) {
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
        for key in self.custom_tooltips.keys().filter(|key| !key.is_empty()) {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&token(icons, loc, key.as_bytes()).map_err(Refusal::Localisation)?);
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
mod tests {
    use super::*;
    use crate::combat::binding::NoIcons;
    use dashboard::modifier_text::{
        BOOLEAN, NEUTRAL, NO_PLUS, PERCENT, PERCENT_TIMES_HUNDRED, POSITIVE_IS_GOOD,
    };

    fn loc(entries: &[(&str, &str)]) -> Localisation {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn row(flags: u32, precision: i32) -> ModifierDefinitionRow {
        ModifierDefinitionRow {
            name_key: b"MODIFIER_X".to_vec(),
            flags,
            precision,
            category_mask: 1,
            ..Default::default()
        }
    }

    #[test]
    fn value_text_follows_percent_sign_colour_and_postfix_flags() {
        let loc = loc(&[("MOD_DAYS", "days"), ("YES", "Yes"), ("NO", "No")]);
        // Bit 0: the value is a fraction shown as a percentage.
        let percent = row(PERCENT_TIMES_HUNDRED | POSITIVE_IS_GOOD, 1);
        assert_eq!(
            value_text(&percent, 5_000, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}G+5.0%\u{11}!"
        );
        assert_eq!(
            value_text(&percent, -5_000, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}R-5.0%\u{11}!"
        );
        // Without bit 1 a positive value is bad.
        let bad = row(PERCENT_TIMES_HUNDRED, 1);
        assert_eq!(
            value_text(&bad, 5_000, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}R+5.0%\u{11}!"
        );
        let neutral = row(NEUTRAL | NO_PLUS | PERCENT, 0);
        assert_eq!(
            value_text(&neutral, 250_000, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}Y2%\u{11}!"
        );
        let mut days = row(POSITIVE_IS_GOOD, 2);
        days.postfix_key = b"MOD_DAYS".to_vec();
        assert_eq!(
            value_text(&days, 0, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}Y0.00\u{11}! days"
        );
        let boolean = row(BOOLEAN | POSITIVE_IS_GOOD, 0);
        assert_eq!(
            value_text(&boolean, 100_000, 2, false, false, &NoIcons, &loc).unwrap(),
            "\u{11}GYes\u{11}!"
        );
    }

    #[test]
    fn description_orders_by_index_and_skips_hidden_zero_and_uncategorised_entries() {
        let loc = loc(&[
            ("MODIFIER_A", "A"),
            ("MODIFIER_B", "B $ARG$"),
            ("custom_tt", "Custom"),
        ]);
        let mut rows = vec![ModifierDefinitionRow::default(); 5];
        for (index, row) in rows.iter_mut().enumerate() {
            row.index = index as u32;
            row.category_mask = 1;
            row.flags = PERCENT_TIMES_HUNDRED | POSITIVE_IS_GOOD;
            row.precision = 0;
            row.name_key = b"MODIFIER_A".to_vec();
        }
        rows[2].name_key = b"MODIFIER_B".to_vec();
        rows[2].argument = b"Tanks".to_vec();
        rows[3].category_mask = 0;
        rows[4].feature_selector = 4;
        rows[4].eligibility = 1;
        let definitions = Definitions {
            rows: &rows,
            names: std::sync::Arc::new(Ok(BTreeMap::new())),
            features: Some(0),
        };
        let block = Block {
            values: BTreeMap::from([(4, 10_000), (2, -10_000), (1, 10_000), (3, 10_000), (0, 0)]),
            hidden: BTreeSet::from([1]),
            custom_tooltips: keys(&["custom_tt"]),
            added: false,
        };
        assert_eq!(
            block
                .describe(&definitions, &NoIcons, &loc, ONE, "")
                .unwrap(),
            "B Tanks: \u{11}R-10%\u{11}!\nCustom"
        );
        // A clamped definition cannot show a negative value.
        let mut clamped = rows.clone();
        clamped[2].flags |= CLAMP;
        let definitions = Definitions {
            rows: &clamped,
            names: std::sync::Arc::new(Ok(BTreeMap::new())),
            features: Some(0),
        };
        assert_eq!(
            block
                .describe(&definitions, &NoIcons, &loc, ONE, "")
                .unwrap(),
            "Custom"
        );
        // An unknown DLC mask refuses a gated entry rather than guessing.
        let definitions = Definitions {
            rows: &rows,
            names: std::sync::Arc::new(Ok(BTreeMap::new())),
            features: None,
        };
        assert!(
            block
                .describe(&definitions, &NoIcons, &loc, ONE, "")
                .is_err()
        );
        // Each line takes the prefix before its name.
        let definitions = Definitions {
            rows: &rows,
            names: std::sync::Arc::new(Ok(BTreeMap::new())),
            features: Some(0),
        };
        let prefixed = Block {
            values: BTreeMap::from([(0, 10_000), (1, 10_000)]),
            ..Block::default()
        };
        assert_eq!(
            prefixed
                .describe(&definitions, &NoIcons, &loc, ONE, "  ")
                .unwrap(),
            "  A: \u{11}G+10%\u{11}!\n  A: \u{11}G+10%\u{11}!"
        );
    }

    /// One CModifier entry `key = value` as the file's reader returns it.
    fn entry(key: &str, value: &str) -> installed_content::parser::Item {
        installed_content::parser::Item {
            key: Some(key.as_bytes().to_vec()),
            key_quoted: false,
            operator: installed_content::parser::Operator::Equals,
            value: installed_content::parser::Value::Scalar(value.as_bytes().to_vec()),
            kind: installed_content::parser::Kind::Bare,
        }
    }

    /// `custom_modifier_tooltip` keys read in order by the shared CModifier
    /// reader (1406031A0), in its table's slot order.
    fn keys(keys: &[&str]) -> TooltipTable {
        let mut modifiers = installed_content::modifiers::Modifiers::default();
        for key in keys {
            modifiers.read(&entry("custom_modifier_tooltip", key));
        }
        modifiers.tooltip_table().clone()
    }

    fn listed(table: &TooltipTable) -> Vec<&str> {
        table.keys().collect()
    }

    /// Only registered rows are recorded, so the table can skip indices (the
    /// native gap at 0x29A). A frame recording indices 0, 1 and 3 is
    /// admitted, its name index points at index 3, and index 3 formats.
    #[test]
    fn sparse_definition_tables_are_admitted_and_format_by_index() {
        use replay::frame::{SharedView, TableView};
        use replay::rows::{LexicalTokenRow, TableId, TableRows};
        let mut frame = frame(0, 0);
        frame.tables[TableId::ModifierDefinitions.index()] =
            SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(
                [0, 1, 3]
                    .map(|index| ModifierDefinitionRow {
                        index,
                        token: index + 10,
                        name_key: format!("MODIFIER_{index}").into_bytes(),
                        flags: PERCENT,
                        category_mask: 1,
                        precision: 0,
                        ..Default::default()
                    })
                    .to_vec(),
            )));
        frame.tables[TableId::LexicalTokens.index()] =
            SharedView::from(TableView::Complete(TableRows::LexicalTokens(
                [10, 11, 13]
                    .map(|token| LexicalTokenRow {
                        token,
                        literal: format!("key_{}", token - 10).into_bytes(),
                    })
                    .to_vec(),
            )));
        let definitions = Definitions::read(&frame, &name_index_cache())
            .unwrap()
            .unwrap();
        assert_eq!(definitions.index("key_3"), Ok(Some(3)));
        // The lexer folds a bare token before looking it up (1424A7A90), so
        // an authored `KEY_3` is the recorded `key_3`.
        assert_eq!(definitions.index("KEY_3"), Ok(Some(3)));
        let authored = installed_content::modifiers::values(&[entry("Key_3", "0.1")]);
        let (built, dropped) = Block::build(&definitions, &authored).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(built.values, BTreeMap::from([(3, 10_000)]));
        let loc = loc(&[("MODIFIER_3", "Three")]);
        let block = Block {
            values: BTreeMap::from([(3, 10_000)]),
            ..Block::default()
        };
        let text = block
            .describe(&definitions, &NoIcons, &loc, ONE, "")
            .unwrap();
        assert!(text.starts_with("Three"), "{text}");
    }

    #[test]
    fn custom_tooltip_keys_follow_the_robin_hood_slot_order() {
        // FNV-1a & 7: k5 -> 5, k6 and k15 -> 4. The first insert grows the
        // shared table to eight buckets; k15 then displaces k5 to slot 6.
        let set = keys(&["k5", "k6", "k15", "k5"]);
        assert_eq!(listed(&set), ["k6", "k15", "k5"]);
        // A copy rebuilds into 2^3 buckets from the source's slot order.
        assert_eq!(listed(&set.copied()), ["k6", "k15", "k5"]);
        // Growth past the 0.9 load of mask 7 keeps every key once.
        let names: Vec<String> = (0..20).map(|index| format!("k{index}")).collect();
        let many = keys(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let mut sorted = listed(&many);
        sorted.sort();
        let mut expected: Vec<&str> = names.iter().map(String::as_str).collect();
        expected.sort();
        assert_eq!(sorted, expected);
        assert_eq!(many.copied().keys().count(), 20);
        let described = Block {
            custom_tooltips: keys(&["b", "a"]),
            ..Block::default()
        };
        let loc = loc(&[("a", "A"), ("b", "B")]);
        let rows: [ModifierDefinitionRow; 0] = [];
        let definitions = Definitions {
            rows: &rows,
            names: std::sync::Arc::new(Ok(BTreeMap::new())),
            features: None,
        };
        // "a" hashes to bucket 4 and "b" to 5: slot order, not read order.
        assert_eq!(
            described
                .describe(&definitions, &NoIcons, &loc, ONE, "")
                .unwrap(),
            "A\nB"
        );
    }

    /// The block keeps the reader's source order (1406042A0): a modifier
    /// stored visibly, then in `hidden_modifier`, then visibly again takes
    /// the last value and stays hidden; visible entries are not applied
    /// before hidden ones.
    #[test]
    fn a_block_keeps_the_readers_last_value_in_source_order() {
        let rows = [ModifierDefinitionRow {
            index: 0,
            token: 1,
            name_key: b"A".to_vec(),
            ..Default::default()
        }];
        let definitions = Definitions::from_parts(&rows, &[(b"a", 0)], Some(0));
        let hidden = installed_content::parser::Item {
            value: installed_content::parser::Value::Block(vec![entry("A", "0.2")]),
            ..entry("hidden_modifier", "")
        };
        let authored =
            installed_content::modifiers::values(&[entry("a", "0.1"), hidden, entry("a", "0.3")]);
        let (block, dropped) = Block::build(&definitions, &authored).unwrap();
        assert!(dropped.is_empty());
        assert_eq!(block.values, BTreeMap::from([(0, 30_000)]));
        assert_eq!(block.hidden, BTreeSet::from([0]));
    }

    /// A frame with `definitions` modifier rows and `tokens` dictionary rows.
    fn frame(definitions: usize, tokens: usize) -> DecodedFrame {
        use replay::{
            frame::{Metrics, SharedView, TableView},
            rows::{LexicalTokenRow, TableId, TableRows},
        };
        let mut tables: [SharedView; replay::frame::TABLE_COUNT] = std::array::from_fn(|_| {
            SharedView::from(TableView::Unavailable {
                reason: "not recorded".into(),
            })
        });
        tables[TableId::ModifierDefinitions.index()] =
            SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(
                (0..definitions)
                    .map(|index| ModifierDefinitionRow {
                        index: index as u32,
                        token: index as u32 + 1,
                        ..Default::default()
                    })
                    .collect(),
            )));
        tables[TableId::LexicalTokens.index()] =
            SharedView::from(TableView::Complete(TableRows::LexicalTokens(
                (0..tokens as u32)
                    .map(|token| LexicalTokenRow {
                        token,
                        literal: format!("modifier_name_{token}").into_bytes(),
                    })
                    .collect(),
            )));
        DecodedFrame {
            branch: 1,
            hour: 1,
            tables,
            metrics: Metrics::default(),
        }
    }

    /// Rows sharing a token keep the last registration, the highest index
    /// (14054A990 through 140548A80); tokens whose names differ only in case,
    /// which the game never holds, are refused by name.
    #[test]
    fn rows_sharing_a_token_keep_the_last_registration() {
        use replay::{
            frame::{SharedView, TableView},
            rows::{LexicalTokenRow, TableId, TableRows},
        };
        let mut frame = frame(0, 0);
        let row = |index, token| ModifierDefinitionRow {
            index,
            token,
            ..Default::default()
        };
        frame.tables[TableId::ModifierDefinitions.index()] =
            SharedView::from(TableView::Complete(TableRows::ModifierDefinitions(vec![
                row(5, 1),
                row(700, 1),
                row(6, 2),
                row(7, 3),
            ])));
        let token = |token, literal: &str| LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        };
        frame.tables[TableId::LexicalTokens.index()] =
            SharedView::from(TableView::Complete(TableRows::LexicalTokens(vec![
                token(1, "shared"),
                token(2, "Twice"),
                token(3, "twice"),
            ])));
        let names = index_names(&frame).unwrap().unwrap();
        assert_eq!(names[&b"shared"[..]], Some(700));
        assert_eq!(names[&b"twice"[..]], None);
    }

    #[test]
    fn the_name_index_is_built_once_per_static_table_pair() {
        let first = frame(2_000, 70_000);
        let mut next = frame(1, 1);
        next.hour = 2;
        // The next hour shares the static tables, as the decoder does.
        for table in NAME_INDEX_TABLES {
            next.tables[table.index()] = first.tables[table.index()].clone();
        }
        let cache = name_index_cache();
        let started = std::time::Instant::now();
        for _ in 0..10 {
            index_names(&first).unwrap().unwrap();
        }
        let uncached = started.elapsed() / 10;
        let a = Definitions::read(&first, &cache).unwrap().unwrap();
        let started = std::time::Instant::now();
        for _ in 0..10 {
            Definitions::read(&next, &cache).unwrap().unwrap();
        }
        let cached = started.elapsed() / 10;
        let b = Definitions::read(&next, &cache).unwrap().unwrap();
        assert!(std::sync::Arc::ptr_eq(&a.names, &b.names));
        assert_eq!(a.index("modifier_name_7"), Ok(Some(6)));
        eprintln!("name index per bind: {uncached:?} rebuilt, {cached:?} cached");
    }
}
