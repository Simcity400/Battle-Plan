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

/// The recorded definitions by script name, from the branch's last hour that
/// holds both the definitions and the lexer names complete. Blocks are read
/// from the last back, decoding only those two tables.
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
                .map(str::to_owned)
        };
        return Ok(Some(
            rows.iter()
                .filter_map(|row| Some((name(row.token)?, row.clone())))
                .collect(),
        ));
    }
    Ok(None)
}

/// `name_key` localised with its `$ARG$` (GetLocalizedName@1405505D0).
fn localised_name(content: &PoliticsContent, key: &str, argument: &str) -> Option<String> {
    let localisation = &content.catalogue.localisation;
    let mut text = localisation.get(key)?.to_owned();
    // `$ARG$` may sit inside a referenced text, and a reference may carry a format
    // (`$KEY|Y$`): references are expanded and the argument filled in, level by
    // level, as far as `resolve_text` would go, and no longer than it allows.
    for _ in 0..8 {
        let filled = if argument.is_empty() {
            text.clone()
        } else {
            substitute(&text, "ARG", argument)
        };
        let expanded = expand_once(&filled, localisation)?;
        if expanded == text {
            break;
        }
        text = expanded;
    }
    catalogue::resolve_text(&text, localisation).text()
}

/// Longest text a label may expand to, as `catalogue::resolve_text` allows.
const MAX_LABEL: usize = 8192;

/// Replaces each `$KEY$` (or `$KEY|format$`) the localisation defines with its
/// text; None once the result would pass [`MAX_LABEL`].
fn expand_once(text: &str, localisation: &installed_content::localisation::Localisation) -> Option<String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        let Some(length) = rest[start + 1..].find('$') else {
            break;
        };
        let inner = &rest[start + 1..start + 1 + length];
        result.push_str(&rest[..start]);
        match inner
            .split('|')
            .next()
            .and_then(|name| localisation.get(name))
        {
            Some(value) => result.push_str(value),
            None => result.push_str(&rest[start..start + length + 2]),
        }
        if result.len() > MAX_LABEL {
            return None;
        }
        rest = &rest[start + length + 2..];
    }
    result.push_str(rest);
    (result.len() <= MAX_LABEL).then_some(result)
}

/// Replaces `$NAME$` and `$NAME|format$` with `value`.
fn substitute(text: &str, name: &str, value: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('$') {
        let Some(length) = rest[start + 1..].find('$') else {
            break;
        };
        let inner = &rest[start + 1..start + 1 + length];
        result.push_str(&rest[..start]);
        if inner.split('|').next() == Some(name) {
            result.push_str(value);
        } else {
            result.push_str(&rest[start..start + length + 2]);
        }
        rest = &rest[start + length + 2..];
    }
    result.push_str(rest);
    result
}

/// The formats of `keys`: recorded first, else scripted in content. Also says
/// whether a recorded argument (text the game localised in its own language
/// when it loaded) was used.
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
                label: localised_name(content, &String::from_utf8_lossy(&row.name_key), &argument),
                postfix: (!postfix.is_empty())
                    .then(|| content.localise(&postfix))
                    .flatten(),
                postfix_key: (!postfix.is_empty()).then(|| postfix.into_owned()),
                flags: row.flags,
                precision: row.precision,
                feature: row.feature_selector,
                source: "recorded".into(),
            });
            continue;
        }
        // LoadScriptModifierDefinitions@1406025E0 and its reader's defaults.
        if let Some(definition) = content.modifier_definitions.get(key) {
            let colour = match definition.color_type.as_deref() {
                Some("good") => 0x2,
                Some("neutral") => 0x4,
                _ => 0,
            };
            let value = match definition.value_type.as_deref() {
                Some("percentage") => 0x1,
                Some("percentage_in_hundred") => 0x80,
                Some("yes_no") => 0x20,
                _ => 0,
            };
            let postfix = match definition.postfix.as_deref() {
                Some("days") => Some("MOD_DAYS"),
                Some("hours") => Some("MOD_HOUR"),
                Some("daily") => Some("MOD_DAILY"),
                _ => None,
            };
            formats.push(ModifierFormat {
                key: key.clone(),
                index: None,
                label: localised_name(content, key, ""),
                postfix: postfix.and_then(|key| content.localise(key)),
                postfix_key: postfix.map(str::to_owned),
                flags: colour | value,
                // Scripted definitions register with no feature (FindOrRegisterDynamicModifierDefinition).
                feature: 0,
                precision: definition.precision.unwrap_or(2),
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
            text: content.localise(key),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::substitute;

    #[test]
    fn an_argument_fills_a_placeholder_inside_a_referenced_text() {
        let localisation: installed_content::localisation::Localisation = [
            ("NAME".to_owned(), "$SHARED_LABEL$".to_owned()),
            ("SHARED_LABEL".to_owned(), "$ARG$ speed".to_owned()),
        ]
        .into_iter()
        .collect();
        let mut text = localisation.get("NAME").unwrap().to_owned();
        for _ in 0..8 {
            let expanded =
                super::expand_once(&substitute(&text, "ARG", "Aircraft"), &localisation).unwrap();
            if expanded == text {
                break;
            }
            text = expanded;
        }
        assert_eq!(text, "Aircraft speed");
    }

    #[test]
    fn expansion_is_bounded_and_formatted_references_expand() {
        let localisation: installed_content::localisation::Localisation = [
            ("L".to_owned(), "$L$".repeat(20)),
            ("LABEL".to_owned(), "Speed".to_owned()),
        ]
        .into_iter()
        .collect();
        let mut text = "$L$".to_owned();
        let mut refused = false;
        for _ in 0..8 {
            match super::expand_once(&text, &localisation) {
                Some(expanded) => text = expanded,
                None => {
                    refused = true;
                    break;
                }
            }
        }
        assert!(refused && text.len() <= super::MAX_LABEL);
        assert_eq!(
            super::expand_once("$LABEL|Y$", &localisation).as_deref(),
            Some("Speed")
        );
    }

    #[test]
    fn an_argument_fills_its_placeholder_with_or_without_a_format() {
        assert_eq!(substitute("$ARG$ speed", "ARG", "Army"), "Army speed");
        assert_eq!(
            substitute("x $ARG|Y$ y $OTHER$", "ARG", "A"),
            "x A y $OTHER$"
        );
        assert_eq!(substitute("no end $ARG", "ARG", "A"), "no end $ARG");
    }
}
