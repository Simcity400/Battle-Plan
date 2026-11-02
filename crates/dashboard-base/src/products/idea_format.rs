//! How an idea's tooltip shows its effects, as the game formats them.
//!
//! Every built-in modifier's display lives in the executable's modifier
//! definition table, which the recording keeps (`ModifierDefinitions`): its
//! localisation key and argument, postfix, precision and flags. A scripted
//! modifier the table does not hold is read from content
//! (`common/modifier_definitions`) with the engine's defaults. A key found in
//! neither has no format: the viewer shows it as authored, marked unformatted.
//! The formatting rules themselves are applied by the
//! viewer; see `ui/src/dashboard/ideaTooltip.ts`.

use super::politics_content::PoliticsContent;
use crate::{catalogue, metadata::validate_campaign};
use installed_content_localisation::localisation::Argument;
use replay::container::Replay;
use replay_data::{
    rows::LexicalTokenRow, rows::ModifierDefinitionRow, rows::TableId, rows::tables,
};
use replay_frame::{frame::FrameDecoder, frame::SharedView, frame::TableView};
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
/// folded for lookup), from the branch's last
/// hour that holds both the definitions and the lexer names complete. Blocks
/// are read from the last back, decoding only those two tables.
pub(crate) fn recorded(
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
        return Ok(Some(index_recorded(rows, lexical)));
    }
    Ok(None)
}

/// Index one supplying read's definitions by its folded lexer names.
pub fn index_recorded(
    rows: &[ModifierDefinitionRow],
    lexical: &[LexicalTokenRow],
) -> BTreeMap<String, ModifierDefinitionRow> {
    rows.iter()
        .filter_map(|row| {
            let at = lexical
                .binary_search_by_key(&row.token, |row| row.token)
                .ok()?;
            let key = std::str::from_utf8(&lexical[at].literal)
                .ok()?
                .to_ascii_lowercase();
            Some((key, row.clone()))
        })
        .collect()
}

/// Localise `key` through the shared localiser with its stored argument as the string
/// `ARG` (type 1) when it has one, else with none. A key the content lacks is
/// its own bytes; the error names why the text cannot be shown.
fn name_arguments(argument: &str) -> Vec<(&str, Argument<'_>)> {
    if argument.is_empty() {
        Vec::new()
    } else {
        vec![("ARG", Argument::Text(argument))]
    }
}

/// The same argument selection, with recoveries and refusals noted by key.
fn label(content: &PoliticsContent, key: &str, argument: &str) -> Option<String> {
    content
        .catalogue
        .localised_with(key, &name_arguments(argument), catalogue::View::Scripted)
}

/// A scripted modifier's display, shared by the reply and offline preparation.
pub(crate) fn content_format(content: &PoliticsContent, key: &str) -> Option<ModifierFormat> {
    let definition = content.modifier_definitions.get(key)?;
    let postfix = definition.postfix_key();
    Some(ModifierFormat {
        key: key.to_owned(),
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
    })
}

/// Formats available without a recording. A built-in key names the missing
/// recorded table rather than acquiring a guessed display definition.
pub fn content_formats(
    content: &PoliticsContent,
    keys: &BTreeSet<String>,
) -> Vec<crate::findings::Finding> {
    use crate::findings::{Finding, Kind};
    let mut findings = Vec::new();
    for key in keys {
        if content_format(content, key).is_none() {
            findings.push(Finding::new(Kind::Offline, format!("modifier {key}"), "the modifier's display definition needs the recorded ModifierDefinitions table; unavailable offline"));
        }
    }
    findings
}

/// The formats of `keys`, each a modifier as the game identifies it (its
/// name folded, installed_content_core::modifiers::Modifier::name): recorded first, else
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
    let recorded = if keys.is_empty() {
        None
    } else {
        recorded(replay, branch, cancelled)?
    };
    Ok(modifier_formats_from_recorded(
        recorded.as_ref(),
        keys,
        content,
    ))
}

/// Formats from a caller's already indexed supplying read, without rescanning.
pub fn modifier_formats_from_recorded(
    recorded: Option<&BTreeMap<String, ModifierDefinitionRow>>,
    keys: &BTreeSet<String>,
    content: &PoliticsContent,
) -> (Vec<ModifierFormat>, Vec<String>) {
    let mut warnings = Vec::new();
    if recorded.is_none() && !keys.is_empty() {
        warnings.push(
            "The game's modifier definitions were not recorded; built-in effects are shown as authored, unformatted."
                .to_owned(),
        );
    }
    let mut arguments = false;
    let mut formats = Vec::new();
    for key in keys {
        if let Some(row) = recorded.and_then(|rows| rows.get(key)) {
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
        // Use the first definition registered under the key's name,
        // with folded lookup.
        if let Some(format) = content_format(content, key) {
            formats.push(format);
        }
    }
    if arguments {
        warnings.push(
            "Some effect names include text the game localised in the recording's language."
                .to_owned(),
        );
    }
    (formats, warnings)
}

/// The texts of `keys` in the viewer's language.
pub fn texts(content: &PoliticsContent, keys: &BTreeSet<String>) -> Vec<LocalisedText> {
    keys.iter()
        .map(|key| LocalisedText {
            key: key.clone(),
            // Idea tooltip lines use scoped localisation.
            text: content.localised(key, catalogue::View::Scripted),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{catalogue, name_arguments};

    /// The localiser fills `ARG` at every nested level, a
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
            catalogue::localised_with(
                &localisation,
                "NAME",
                &name_arguments("Luftstreitkräfte"),
                catalogue::View::Scripted
            ),
            Ok("Luftstreitkräfte speed".into())
        );
        assert_eq!(
            catalogue::localised_with(
                &localisation,
                "NAME",
                &name_arguments(""),
                catalogue::View::Scripted
            ),
            Ok(" speed".into())
        );
        assert_eq!(
            catalogue::localised_with(
                &localisation,
                "FORMATTED",
                &name_arguments(""),
                catalogue::View::Scripted
            ),
            Ok("xy".into())
        );
        assert_eq!(
            catalogue::localised_with(&localisation, "GAP", &name_arguments(""), catalogue::View::Scripted),
            Err(
                "localisation GAP holds the control byte 0x14 (an authored `¤`), which plain text cannot show".into()
            )
        );
        assert_eq!(
            catalogue::localised_with(
                &localisation,
                "ABSENT",
                &name_arguments("x"),
                catalogue::View::Scripted
            )
            .as_deref(),
            Ok("ABSENT")
        );
    }
}
