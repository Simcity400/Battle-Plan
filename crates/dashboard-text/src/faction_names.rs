//! The faction display-name rule, shared by products and scripted text.

use installed_content_core::{
    faction_templates::Catalog, gui::native::BoundLocalization, gui::native::BoundNode,
    gui::native::BoundValue, gui::native::FormattedLocalization,
};
use installed_content_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::TableRows, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::SharedView, frame::TableView};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, OnceLock, Weak},
};

/// Spell the template operand recorded at this hour. A null template needs no
/// dictionary; unread tokens remain errors. Lexer tokens are sorted by token.
pub fn template_spelling(
    frame: &DecodedFrame,
    token: Option<u32>,
) -> Result<Option<String>, String> {
    let Some(token) = token else {
        return Ok(None);
    };
    let lexical = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|reason| format!("LexicalTokens is not recorded completely: {reason}"))?;
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("Faction template token {token} has no LexicalTokens spelling"))?;
    String::from_utf8(lexical[index].literal.clone())
        .map(Some)
        .map_err(|_| format!("Faction template token {token} is not UTF-8"))
}

/// A nonempty literal is copied unchanged; otherwise the template's
/// bound name is resolved. A recorded null template returns empty text
/// without reading content or lexer tokens.
/// The bound scalar may be a key or text: the localiser copies undefined keys.
pub fn name(
    literal: &[u8],
    template: Result<Option<&str>, String>,
    templates: Result<&Catalog, String>,
    localisation: &Localisation,
    recorded_language: &str,
    token_of: impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<String, String> {
    if !literal.is_empty() {
        return crate::scripted_text::stored_text(
            localisation,
            recorded_language,
            literal,
            "faction name",
        );
    }
    let Some(token) = template? else {
        return Ok(String::new());
    };
    let bound = templates?
        .get(token)?
        .ok_or_else(|| format!("Faction template {token:?} is not in the recorded content"))?;
    if bound.is_empty() {
        // Zero entries return a valid empty string.
        return Ok(String::new());
    }
    // The string variant is accepted without a length guard.
    bound_name(bound, localisation, token_of)
        .map_err(|why| format!("Faction template {token:?}: {why}"))
}

/// Each folded lexer name to its one token, or why it names none.
type ReverseIndex = Result<BTreeMap<Vec<u8>, Result<u32, String>>, String>;

/// A dictionary's token rows as changed or deleted from the dictionary saved
/// before it.
type SavedChanges = Arc<Vec<(u32, Option<Vec<u8>>)>>;

/// The lexical dictionary behind a name read. Its reverse index is built only
/// for bound token arguments and shared by factions from the same table read.
#[derive(Debug)]
pub struct LexicalNames {
    table: SharedView,
    names: OnceLock<ReverseIndex>,
    /// Its changes from the dictionary saved before it (none for the first),
    /// kept since a history saves every dictionary again at each commit.
    saved: Mutex<Option<(Option<Weak<LexicalNames>>, SavedChanges)>>,
}

impl Default for LexicalNames {
    fn default() -> Self {
        Self::new(TableView::Complete(TableRows::LexicalTokens(Vec::new())).into())
    }
}

impl LexicalNames {
    /// The empty dictionary every saved history starts from. All saves share
    /// this one allocation, so the first saved dictionary's changes from it
    /// stay kept between saves.
    pub fn empty() -> Arc<Self> {
        static EMPTY: OnceLock<Arc<LexicalNames>> = OnceLock::new();
        Arc::clone(EMPTY.get_or_init(|| Arc::new(Self::default())))
    }

    pub fn new(table: SharedView) -> Self {
        Self {
            table,
            names: OnceLock::new(),
            saved: Mutex::new(None),
        }
    }

    pub fn matches(&self, table: &SharedView) -> bool {
        SharedView::ptr_eq(&self.table, table)
    }

    /// Whether `table` holds this dictionary's rows, from whichever read.
    pub fn holds(&self, table: &SharedView) -> bool {
        self.table == *table
    }

    pub fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        let names = self
            .names
            .get_or_init(|| {
                let rows = match &*self.table {
                    TableView::Complete(TableRows::LexicalTokens(rows)) => rows,
                    TableView::Unavailable { reason } => {
                        return Err(format!(
                            "LexicalTokens is not recorded completely: {reason}"
                        ));
                    }
                    TableView::Truncated { missing, .. } => {
                        return Err(format!(
                            "LexicalTokens is not recorded completely: {missing} rows missing"
                        ));
                    }
                    _ => return Err("LexicalTokens has the wrong table layout".into()),
                };
                let mut names = BTreeMap::new();
                for row in rows {
                    let folded = row.literal.to_ascii_lowercase();
                    match names.entry(folded) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(Ok(row.token));
                        }
                        std::collections::btree_map::Entry::Occupied(mut entry) => {
                            *entry.get_mut() = Err(format!(
                                "{} names several lexer tokens",
                                String::from_utf8_lossy(&row.literal)
                            ));
                        }
                    }
                }
                Ok(names)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        names
            .get(&name.to_ascii_lowercase())
            .map(|value| value.clone().map(Some))
            .unwrap_or(Ok(None))
    }
}

/// Persist historical dictionaries as changed/deleted token rows, not full copies.
pub mod dictionary_history {
    use super::*;
    use replay_data::rows::LexicalTokenRow;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    type Changes<T> = Result<Vec<(u32, Option<T>)>, String>;

    #[cfg(test)]
    thread_local! {
        /// Dictionaries whose changes were computed on this thread.
        pub(super) static COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }

    /// Each dictionary's changes from the complete one before it. A history
    /// only grows, so each dictionary's changes are computed once and kept
    /// with it, rather than every dictionary being compared again at every
    /// save.
    pub fn serialize<S: Serializer>(
        dictionaries: &[Arc<LexicalNames>],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut previous: Option<&Arc<LexicalNames>> = None;
        let mut kept = Vec::with_capacity(dictionaries.len());
        for dictionary in dictionaries {
            kept.push(match &*dictionary.table {
                TableView::Complete(TableRows::LexicalTokens(rows)) => {
                    let changes = changes_from(dictionary, rows, previous);
                    previous = Some(dictionary);
                    Ok(changes)
                }
                TableView::Unavailable { reason } => Err(reason.clone()),
                TableView::Truncated { missing, .. } => Err(format!("{missing} rows missing")),
                _ => {
                    return Err(serde::ser::Error::custom(
                        "LexicalTokens has the wrong table layout",
                    ));
                }
            });
        }
        let versions: Vec<Changes<&[u8]>> = kept
            .iter()
            .map(|version| match version {
                Ok(changes) => Ok(changes
                    .iter()
                    .map(|(token, literal)| (*token, literal.as_deref()))
                    .collect()),
                Err(reason) => Err(reason.clone()),
            })
            .collect();
        versions.serialize(serializer)
    }

    /// `dictionary`'s rows as changed or deleted from `previous`'s, kept on
    /// the dictionary for the same `previous`.
    fn changes_from(
        dictionary: &LexicalNames,
        rows: &[LexicalTokenRow],
        previous: Option<&Arc<LexicalNames>>,
    ) -> SavedChanges {
        let mut saved = dictionary
            .saved
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((from, changes)) = saved.as_ref()
            && match (from, previous) {
                // The weak handle keeps the earlier dictionary's allocation
                // reserved, so an equal address is that dictionary.
                (Some(from), Some(previous)) => from.as_ptr() == Arc::as_ptr(previous),
                (None, None) => true,
                _ => false,
            }
        {
            return Arc::clone(changes);
        }
        #[cfg(test)]
        COMPUTED.with(|computed| computed.set(computed.get() + 1));
        let before: BTreeMap<u32, &[u8]> = match previous.map(|previous| &*previous.table) {
            Some(TableView::Complete(TableRows::LexicalTokens(rows))) => rows
                .iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect(),
            _ => BTreeMap::new(),
        };
        let current: BTreeMap<_, _> = rows
            .iter()
            .map(|row| (row.token, row.literal.as_slice()))
            .collect();
        let mut changes: Vec<_> = current
            .iter()
            .filter(|(token, literal)| before.get(*token) != Some(*literal))
            .map(|(token, literal)| (*token, Some(literal.to_vec())))
            .collect();
        changes.extend(
            before
                .keys()
                .filter(|token| !current.contains_key(*token))
                .map(|token| (*token, None)),
        );
        let changes = Arc::new(changes);
        *saved = Some((previous.map(Arc::downgrade), Arc::clone(&changes)));
        changes
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Arc<LexicalNames>>, D::Error> {
        let versions = Vec::<Changes<Vec<u8>>>::deserialize(deserializer)?;
        let mut rows = BTreeMap::new();
        Ok(versions
            .into_iter()
            .map(|version| {
                let table = match version {
                    Ok(changes) => {
                        for (token, literal) in changes {
                            if let Some(literal) = literal {
                                rows.insert(token, literal);
                            } else {
                                rows.remove(&token);
                            }
                        }
                        TableView::Complete(TableRows::LexicalTokens(
                            rows.iter()
                                .map(|(token, literal)| replay_data::rows::LexicalTokenRow {
                                    token: *token,
                                    literal: literal.clone(),
                                })
                                .collect(),
                        ))
                    }
                    Err(reason) => TableView::Unavailable { reason },
                };
                Arc::new(LexicalNames::new(table.into()))
            })
            .collect())
    }
}

enum BoundArgument {
    Text(String),
    Token(i32),
}

fn formatted(
    value: &FormattedLocalization,
    arguments: &[(&str, Argument<'_>)],
    localisation: &Localisation,
    token_of: &mut impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<BoundArgument, String> {
    match value {
        FormattedLocalization::Key(key) => localisation
            .localise_with(key, arguments)
            .map(BoundArgument::Text)
            .map_err(|why| why.to_string()),
        // Interned token: payloads return the integer variant.
        // The payload's spelling is never parsed as a number.
        FormattedLocalization::Token(token) => token_of(token)?
            .map(|token| BoundArgument::Token(token as i32))
            .ok_or_else(|| {
                format!(
                    "faction bound localisation token {} is absent from LexicalTokens",
                    String::from_utf8_lossy(token)
                )
            }),
        FormattedLocalization::Formatter { formatter, .. } => Err(format!(
            "faction bound localisation formatter {} requires core registry callback",
            String::from_utf8_lossy(formatter)
        )),
    }
}

/// Arguments in authored order, then the core localiser.
/// The work stack sizes from the parsed tree, without a private depth limit.
fn bound_name(
    bound: &BoundLocalization,
    localisation: &Localisation,
    mut token_of: impl FnMut(&[u8]) -> Result<Option<u32>, String>,
) -> Result<String, String> {
    enum Work<'a> {
        Node(usize),
        Value(&'a FormattedLocalization),
        CheckName(&'a [u8]),
        Finish(&'a BoundNode),
    }
    let mut work = vec![Work::Node(0)];
    let mut values = Vec::new();
    while let Some(next) = work.pop() {
        match next {
            Work::Node(index) => {
                let node = bound
                    .nodes
                    .get(index)
                    .ok_or("faction bound localisation node is absent")?;
                work.push(Work::Finish(node));
                for argument in node.arguments.iter().rev() {
                    work.push(Work::CheckName(&argument.name));
                    work.push(match &argument.value {
                        BoundValue::Localization(value) => Work::Value(value),
                        BoundValue::Node(child) => {
                            if *child <= index {
                                return Err(
                                    "faction bound localisation child is not a later preorder node"
                                        .into(),
                                );
                            }
                            Work::Node(*child)
                        }
                    });
                }
            }
            Work::Value(value) => values.push(formatted(value, &[], localisation, &mut token_of)?),
            Work::CheckName(name) => {
                std::str::from_utf8(name)
                    .map_err(|_| "faction bound argument name is not UTF-8")?;
            }
            Work::Finish(node) => {
                let first = values
                    .len()
                    .checked_sub(node.arguments.len())
                    .ok_or("faction bound localisation lacks argument values")?;
                let inputs: Vec<_> = values.drain(first..).collect();
                let arguments = node
                    .arguments
                    .iter()
                    .zip(&inputs)
                    .map(|(argument, value)| {
                        let name = std::str::from_utf8(&argument.name)
                            .map_err(|_| "faction bound argument name is not UTF-8")?;
                        Ok((
                            name,
                            match value {
                                BoundArgument::Text(text) => Argument::Text(text),
                                BoundArgument::Token(token) => Argument::Int32(*token),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                values.push(formatted(
                    &node.key,
                    &arguments,
                    localisation,
                    &mut token_of,
                )?);
            }
        }
    }
    match values
        .pop()
        .ok_or("faction bound localisation node is absent")?
    {
        BoundArgument::Text(text) => Ok(text),
        BoundArgument::Token(_) => {
            Err("faction bound root returns a token integer, not the string variant".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::tests::{loaded, loaded_templates};

    #[test]
    fn faction_names_evaluate_nested_arguments_in_authored_order() {
        let db = loaded_templates(
            "pact={name={localization_key=ROOT \
             FIRST={localization_key=CHILD ID=token:first} SECOND=token:second}}",
        );
        let loc = loaded("l_english:\n ROOT:0 \"$FIRST$ / $SECOND$\"\n CHILD:0 \"Pact $ID$\"\n");
        let mut reads = Vec::new();
        let name = name(b"", Ok(Some("pact")), Ok(&db), &loc, "english", |token| {
            reads.push(token.to_vec());
            Ok(Some(if token == b"first" { 31 } else { 47 }))
        });
        assert_eq!(name.unwrap(), "Pact 31 / 47");
        assert_eq!(reads, [b"first".to_vec(), b"second".to_vec()]);
    }

    #[test]
    fn faction_names_keep_native_token_formatter_refusals_and_known_empty_names() {
        let db = loaded_templates(
            "token={name=token:17} formatted={name=country|target} \
             empty={name={ignored=value}} omitted={} localised_empty={name=EMPTY_NAME} \
             missing={name={localization_key=ROOT ID=token:absent}}",
        );
        let loc = loaded("l_english:\n ROOT:0 \"Pact $ID$\"\n EMPTY_NAME:0 \"\"\n");
        let resolved = |token| {
            name(b"", Ok(Some(token)), Ok(&db), &loc, "english", |_| {
                Ok(Some(99))
            })
        };
        assert!(
            resolved("token")
                .unwrap_err()
                .contains("root returns a token integer")
        );
        assert!(
            resolved("formatted")
                .unwrap_err()
                .contains("formatter country")
        );
        for template in ["empty", "omitted", "localised_empty"] {
            assert_eq!(
                name(b"", Ok(Some(template)), Ok(&db), &loc, "english", |_| {
                    panic!("this name must not read bound token arguments")
                }),
                Ok(String::new()),
                "{template}",
            );
        }
        assert!(
            name(b"", Ok(Some("missing")), Ok(&db), &loc, "english", |_| Ok(
                None
            ))
            .unwrap_err()
            .contains("token absent is absent from LexicalTokens")
        );
    }

    #[test]
    fn a_recorded_null_template_is_empty_but_unread_template_inputs_remain_errors() {
        let loc = loaded("l_english:\n");
        assert_eq!(
            name(
                b"",
                Ok(None),
                Err("content gap".into()),
                &loc,
                "english",
                |_| { panic!("a null template must not read lexer tokens") }
            ),
            Ok(String::new()),
        );
        assert_eq!(
            name(
                b"",
                Err("dictionary gap".into()),
                Err("content gap".into()),
                &loc,
                "english",
                |_| panic!("unread template input must stop before arguments"),
            ),
            Err("dictionary gap".into()),
        );
    }

    #[test]
    fn dictionary_histories_compute_each_dictionarys_changes_once_across_saves() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Saved {
            #[serde(with = "dictionary_history")]
            history: Vec<Arc<LexicalNames>>,
        }
        let row = |token, literal: &[u8]| replay_data::rows::LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        };
        let dictionary = |rows| {
            Arc::new(LexicalNames::new(
                TableView::Complete(TableRows::LexicalTokens(rows)).into(),
            ))
        };
        let saved = Saved {
            history: vec![
                dictionary(vec![row(1, b"a"), row(2, b"b")]),
                Arc::new(LexicalNames::new(
                    TableView::Unavailable {
                        reason: "gap".into(),
                    }
                    .into(),
                )),
                dictionary(vec![row(1, b"a"), row(2, b"B"), row(3, b"c")]),
                dictionary(vec![row(2, b"B"), row(3, b"c")]),
            ],
        };
        dictionary_history::COMPUTED.with(|computed| computed.set(0));
        let first = serde_json::to_vec(&saved).unwrap();
        let second = serde_json::to_vec(&saved).unwrap();
        assert_eq!(first, second);
        // Each complete dictionary is compared once with the one before it.
        assert_eq!(dictionary_history::COMPUTED.with(std::cell::Cell::get), 3);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&first).unwrap()["history"],
            serde_json::json!([
                {"Ok": [[1, [97]], [2, [98]]]},
                {"Err": "gap"},
                {"Ok": [[2, [66]], [3, [99]]]},
                {"Ok": [[1, null]]}
            ])
        );
        let restored: Saved = serde_json::from_slice(&first).unwrap();
        assert_eq!(restored.history.len(), saved.history.len());
        for (restored, original) in restored.history.iter().zip(&saved.history) {
            assert!(restored.holds(&original.table));
        }
    }

    #[test]
    fn saved_histories_rebuilt_from_the_shared_empty_dictionary_keep_every_change() {
        let row = |token, literal: &[u8]| replay_data::rows::LexicalTokenRow {
            token,
            literal: literal.to_vec(),
        };
        let dictionary = |rows| {
            Arc::new(LexicalNames::new(
                TableView::Complete(TableRows::LexicalTokens(rows)).into(),
            ))
        };
        let first = dictionary(vec![row(1, b"a"), row(2, b"b")]);
        let second = dictionary(vec![row(1, b"a"), row(2, b"B")]);
        // A save lists the dictionaries again each time, starting from the
        // empty one, as the country groups' saved state does.
        let save = || {
            #[derive(serde::Serialize)]
            struct Saved {
                #[serde(with = "dictionary_history")]
                history: Vec<Arc<LexicalNames>>,
            }
            serde_json::to_vec(&Saved {
                history: vec![LexicalNames::empty(), first.clone(), second.clone()],
            })
            .unwrap()
        };
        assert!(Arc::ptr_eq(&LexicalNames::empty(), &LexicalNames::empty()));
        dictionary_history::COMPUTED.with(|computed| computed.set(0));
        let before = save();
        let computed = dictionary_history::COMPUTED.with(std::cell::Cell::get);
        assert!(computed > 0);
        assert_eq!(save(), before);
        assert_eq!(
            dictionary_history::COMPUTED.with(std::cell::Cell::get),
            computed,
            "a later save computes no dictionary's changes again"
        );
    }

    #[test]
    fn faction_names_dictionary_folds_ascii_and_refuses_ambiguity_and_gaps() {
        let dictionary =
            |rows| LexicalNames::new(TableView::Complete(TableRows::LexicalTokens(rows)).into());
        let tokens = dictionary(vec![replay_data::rows::LexicalTokenRow {
            token: 31,
            literal: b"MiXeD".to_vec(),
        }]);
        assert_eq!(tokens.token_of(b"mixed").unwrap(), Some(31));
        assert_eq!(tokens.token_of(b"absent").unwrap(), None);
        let ambiguous = dictionary(vec![
            replay_data::rows::LexicalTokenRow {
                token: 31,
                literal: b"Mixed".to_vec(),
            },
            replay_data::rows::LexicalTokenRow {
                token: 47,
                literal: b"MIXED".to_vec(),
            },
        ]);
        assert!(
            ambiguous
                .token_of(b"mixed")
                .unwrap_err()
                .contains("several lexer tokens")
        );
        let missing = LexicalNames::new(
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into(),
        );
        assert!(
            missing
                .token_of(b"mixed")
                .unwrap_err()
                .contains("not captured")
        );
    }

    #[test]
    fn faction_names_keep_all_five_thousand_bound_nodes() {
        let count = 5_000;
        let loc = loaded("l_english:\n PASS:0 \"$ARG$\"\n");
        let bound = BoundLocalization {
            nodes: (0..count)
                .map(|index| BoundNode {
                    key: FormattedLocalization::Key(b"PASS".to_vec()),
                    arguments: vec![installed_content_core::gui::native::BoundArgument {
                        name: b"ARG".to_vec(),
                        value: if index + 1 == count {
                            BoundValue::Localization(FormattedLocalization::Key(
                                b"complete".to_vec(),
                            ))
                        } else {
                            BoundValue::Node(index + 1)
                        },
                    }],
                })
                .collect(),
        };
        assert_eq!(bound_name(&bound, &loc, |_| Ok(None)).unwrap(), "complete");
    }

    #[test]
    fn faction_names_refuse_an_invalid_argument_name_before_later_arguments() {
        let loc = loaded("l_english:\n ROOT:0 \"$ARG$\"\n");
        let bound = BoundLocalization {
            nodes: vec![BoundNode {
                key: FormattedLocalization::Key(b"ROOT".to_vec()),
                arguments: vec![
                    installed_content_core::gui::native::BoundArgument {
                        name: vec![0xff],
                        value: BoundValue::Localization(FormattedLocalization::Key(
                            b"first".to_vec(),
                        )),
                    },
                    installed_content_core::gui::native::BoundArgument {
                        name: b"ARG".to_vec(),
                        value: BoundValue::Localization(FormattedLocalization::Token(
                            b"later".to_vec(),
                        )),
                    },
                ],
            }],
        };
        let mut reads = 0;
        let result = bound_name(&bound, &loc, |_| {
            reads += 1;
            Ok(Some(31))
        });
        assert_eq!(
            result.unwrap_err(),
            "faction bound argument name is not UTF-8"
        );
        assert_eq!(reads, 0);
    }
}
