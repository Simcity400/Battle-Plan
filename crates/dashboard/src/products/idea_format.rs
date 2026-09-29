//! How an idea's tooltip shows its effects, as the game formats them.
//!
//! Every built-in modifier's display lives in the executable's modifier
//! definition table, which the recording keeps (`ModifierDefinitions`): its
//! localisation key and argument, postfix, precision and flags. A scripted
//! modifier the table does not hold is read from content
//! (`common/modifier_definitions`) with the engine's defaults. A key found in
//! neither has no format: the viewer shows it as authored, marked unformatted.
//! The formatting rules themselves (`FormatValue@14054FE50`) are applied by the
//! viewer; see `ui/src/dashboard/ideaTooltip.ts`.

use super::politics_content::PoliticsContent;
use crate::{catalogue, metadata::validate_campaign};
use installed_content::localisation::{Argument, Localisation};
use replay::{
    container::Replay,
    frame::{FrameDecoder, SharedView, TableView},
    rows::{ModifierDefinitionRow, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

/// How the game shows one modifier (CModifierDefinition).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModifierFormat {
    pub key: String,
    /// The engine's key index: the game lists an idea's lines in this order.
    /// None for a definition only content gives (registered after every built-in one).
    pub index: Option<u32>,
    /// The modifier's name in the viewer's language; None when it cannot be localised.
    pub label: Option<String>,
    /// The localised postfix shown after the value (days, hours, daily), if any.
    pub postfix: Option<String>,
    /// The postfix's localisation key, when the definition has one: with `postfix`
    /// empty, the unit exists but its text could not be read.
    pub postfix_key: Option<String>,
    /// Definition flags: 0x1 percent (value ×100), 0x2 good, 0x4 neutral colour,
    /// 0x8 no plus sign, 0x20 yes/no, 0x80 percent as written, 0x100 shown when zero.
    pub flags: u32,
    /// Decimal places at most (the caller's precision may lower it).
    pub precision: i32,
    /// The game feature the modifier needs (`feature_selector`); zero for none. The
    /// game shows and applies such a modifier only while that feature is on, which
    /// the recording does not say.
    pub feature: u32,
    /// `recorded` (the game's own table) or `content` (a scripted definition).
    pub source: String,
}

/// A localisation key a tooltip needs, with its text in the viewer's language.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LocalisedText {
    pub key: String,
    /// None when the key has no text, or its text needs runtime scope.
    pub text: Option<String>,
}

/// The recorded definitions by the modifier each names (its script name
/// folded, as the lexer looks a key up, 1424A7A90), from the branch's last
/// hour that holds both the definitions and the lexer names complete. Blocks
/// are read from the last back, decoding only those two tables.
fn recorded(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Option<BTreeMap<String, ModifierDefinitionRow>>> {
    let selected = [
        TableId::Campaign,
        TableId::ModifierDefinitions,
        TableId::LexicalTokens,
    ];
    for block in replay
        .index
        .iter()
        .rev()
        .filter(|block| block.branch == branch)
    {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Idea request cancelled",
            ));
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        // The block's last hour holding both tables complete, kept as shared views;
        // the lookup is built once, from that hour only.
        let mut last: Option<(SharedView, SharedView)> = None;
        for frame in frames {
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            if decoded.complete::<tables::ModifierDefinitions>().is_ok()
                && decoded.complete::<tables::LexicalTokens>().is_ok()
            {
                last = Some((
                    decoded.tables[TableId::ModifierDefinitions.index()].clone(),
                    decoded.tables[TableId::LexicalTokens.index()].clone(),
                ));
            }
        }
        let Some((definitions, lexical)) = last else {
            continue;
        };
        let (TableView::Complete(definitions), TableView::Complete(lexical)) =
            (&*definitions, &*lexical)
        else {
            continue;
        };
        let (Some(rows), Some(lexical)) = (
            definitions.rows::<tables::ModifierDefinitions>(),
            lexical.rows::<tables::LexicalTokens>(),
        ) else {
            continue;
        };
        let name = |token: u32| {
            let at = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
            std::str::from_utf8(&lexical[at].literal)
                .ok()
                .map(str::to_ascii_lowercase)
        };
        return Ok(Some(
            rows.iter()
                .filter_map(|row| Some((name(row.token)?, row.clone())))
                .collect(),
        ));
    }
    Ok(None)
}

/// `key` as `modifier_definition::GetLocalizedName` (1405505D0) localises
/// it: through the game's localiser with its stored argument as the string
/// `ARG` (type 1) when it has one, else with none. A key the content lacks is
/// its own bytes (142230940); the error names why the text cannot be shown.
fn localised_name(
    localisation: &Localisation,
    key: &str,
    argument: &str,
) -> Result<String, String> {
    let arguments = [("ARG", Argument::Text(argument))];
    let arguments: &[_] = if argument.is_empty() { &[] } else { &arguments };
    // Idea effects are listed by FormatScopedModifierDescription (14054E520
    // -> 1410CF560), which expands them.
    catalogue::localised_with(localisation, key, arguments, catalogue::View::Scripted)
}

/// [`localised_name`], a refusal noted in the catalogue by key.
fn label(content: &PoliticsContent, key: &str, argument: &str) -> Option<String> {
    localised_name(&content.catalogue.localisation, key, argument)
        .inspect_err(|reason| content.catalogue.refuse(reason, key))
        .ok()
}

/// The formats of `keys`, each a modifier as the game identifies it (its
/// name folded, installed_content::modifiers::Modifier::name): recorded first, else
/// scripted in content, whose definition the lexer finds whatever its case.
/// Also says whether a recorded argument (text the game localised in its own
/// language when it loaded) was used.
pub fn modifier_formats(
    replay: &Replay,
    branch: u64,
    keys: &BTreeSet<String>,
    content: &PoliticsContent,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<ModifierFormat>, Vec<String>)> {
    let mut warnings = Vec::new();
    let recorded = if keys.is_empty() {
        None
    } else {
        recorded(replay, branch, cancelled)?
    };
    if recorded.is_none() && !keys.is_empty() {
        warnings.push(
            "The game's modifier definitions were not recorded; built-in effects are shown as authored, unformatted."
                .to_owned(),
        );
    }
    let mut arguments = false;
    let mut formats = Vec::new();
    for key in keys {
        if let Some(row) = recorded.as_ref().and_then(|rows| rows.get(key)) {
            let argument = String::from_utf8_lossy(&row.argument);
            arguments |= !argument.is_empty();
            let postfix = String::from_utf8_lossy(&row.postfix_key);
            formats.push(ModifierFormat {
                key: key.clone(),
                index: Some(row.index),
                label: label(content, &String::from_utf8_lossy(&row.name_key), &argument),
                postfix: (!postfix.is_empty())
                    .then(|| content.localised(&postfix, catalogue::View::Scripted))
                    .flatten(),
                postfix_key: (!postfix.is_empty()).then(|| postfix.into_owned()),
                flags: row.flags,
                precision: row.precision,
                feature: row.feature_selector,
                source: "recorded".into(),
            });
            continue;
        }
        // LoadScriptModifierDefinitions@1406025E0 as it stored the
        // definition first registered under the key's name
        // (FindOrRegisterDynamicModifierDefinition 1405FF750), which a key
        // finds folded (1424A7A90, 1424A5EE0).
        if let Some(definition) = content.modifier_definitions.get(key) {
            let postfix = definition.postfix_key();
            formats.push(ModifierFormat {
                key: key.clone(),
                index: None,
                // Its name is its localisation key, as first registered.
                label: label(content, &definition.token, ""),
                postfix: postfix.and_then(|key| content.localised(key, catalogue::View::Scripted)),
                postfix_key: postfix.map(str::to_owned),
                flags: definition.flags,
                // Scripted definitions register with no feature (FindOrRegisterDynamicModifierDefinition).
                feature: 0,
                precision: definition.precision,
                source: "content".into(),
            });
        }
    }
    if arguments {
        warnings.push(
            "Some effect names include text the game localised in the recording's language."
                .to_owned(),
        );
    }
    Ok((formats, warnings))
}

/// The texts of `keys` in the viewer's language.
pub fn texts(content: &PoliticsContent, keys: &BTreeSet<String>) -> Vec<LocalisedText> {
    keys.iter()
        .map(|key| LocalisedText {
            key: key.clone(),
            // The idea tooltip's lines (140FBC770 -> 14054E520 -> 1410CF560).
            text: content.localised(key, catalogue::View::Scripted),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::localised_name;

    /// The game's localiser fills `ARG` at every nested level (142230940), a
    /// `$KEY|format$` that is no argument looks up the whole operand, which no
    /// key is, and writes nothing, and a key the content lacks is its own
    /// bytes.
    #[test]
    fn a_modifier_label_is_localised_with_its_argument() {
        let localisation = crate::catalogue::tests::loaded(concat!(
            "l_english:\n",
            " NAME:0 \"$SHARED_LABEL$\"\n",
            " SHARED_LABEL:0 \"$ARG$ speed\"\n",
            " FORMATTED:0 \"x$LABEL|Y$y\"\n",
            " LABEL:0 \"Speed\"\n",
            " GAP:0 \"a¤b\"\n",
        ));
        assert_eq!(
            localised_name(&localisation, "NAME", "Luftstreitkräfte"),
            Ok("Luftstreitkräfte speed".into())
        );
        assert_eq!(
            localised_name(&localisation, "NAME", ""),
            Ok(" speed".into())
        );
        assert_eq!(
            localised_name(&localisation, "FORMATTED", ""),
            Ok("xy".into())
        );
        assert_eq!(
            localised_name(&localisation, "GAP", ""),
            Err(
                "localisation GAP holds the control byte 0x14 (an authored `¤`), which plain text cannot show".into()
            )
        );
        assert_eq!(
            localised_name(&localisation, "ABSENT", "x").as_deref(),
            Ok("ABSENT")
        );
    }
}
