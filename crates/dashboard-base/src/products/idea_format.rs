
use super::politics_content::PoliticsContent;
use crate::catalogue;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{LexicalTokenRow, ModifierDefinitionRow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModifierFormat {
    pub key: String,
    #[doc = " The engine's key index: the game lists an idea's lines in this order."]
    #[doc = " None for a definition only content gives (registered after every built-in one)."]
    pub index: Option<u32>,
    #[doc = " The modifier's name in the viewer's language; None when it cannot be localised."]
    pub label: Option<String>,
    #[doc = " The localised postfix shown after the value (days, hours, daily), if any."]
    pub postfix: Option<String>,
    #[doc = " The postfix's localisation key, when the definition has one: with `postfix`"]
    #[doc = " empty, the unit exists but its text could not be read."]
    pub postfix_key: Option<String>,
    #[doc = " Definition flags: 0x1 percent (value ×100), 0x2 good, 0x4 neutral colour,"]
    #[doc = " 0x8 no plus sign, 0x20 yes/no, 0x80 percent as written, 0x100 shown when zero."]
    pub flags: u32,
    #[doc = " Decimal places at most (the caller's precision may lower it)."]
    pub precision: i32,
    #[doc = " The game feature the modifier needs (`feature_selector`); zero for none. The"]
    #[doc = " game shows and applies such a modifier only while that feature is on, which"]
    #[doc = " the recording does not say."]
    pub feature: u32,
    #[doc = " `recorded` (the game's own table) or `content` (a scripted definition)."]
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LocalisedText {
    pub key: String,
    #[doc = " None when the key has no text, or its text needs runtime scope."]
    pub text: Option<String>,
}

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

fn name_arguments(argument: &str) -> Vec<(&str, Argument<'_>)> {
    if argument.is_empty() {
        Vec::new()
    } else {
        vec![("ARG", Argument::Text(argument))]
    }
}

fn label(content: &PoliticsContent, key: &str, argument: &str) -> Option<String> {
    content
        .catalogue
        .localised_with(key, &name_arguments(argument), catalogue::View::Scripted)
}

pub(crate) fn content_format(content: &PoliticsContent, key: &str) -> Option<ModifierFormat> {
    let definition = content.modifier_definitions.get(key)?;
    let postfix = definition.postfix_key();
    Some(ModifierFormat {
        key: key.to_owned(),
        index: None,
        label: label(content, &definition.token, ""),
        postfix: postfix.and_then(|key| content.localised(key, catalogue::View::Scripted)),
        postfix_key: postfix.map(str::to_owned),
        flags: definition.flags,
        feature: 0,
        precision: definition.precision,
        source: "content".into(),
    })
}

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

pub fn texts(content: &PoliticsContent, keys: &BTreeSet<String>) -> Vec<LocalisedText> {
    keys.iter()
        .map(|key| LocalisedText {
            key: key.clone(),
            text: content.localised(key, catalogue::View::Scripted),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{catalogue, name_arguments};

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
