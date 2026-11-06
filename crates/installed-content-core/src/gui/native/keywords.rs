//! Keyword classification during interface loading.
//!
//! Static identities come from verified executable metadata. Synchronized dynamic tokens
//! load from the recorded content stack before interface files; later database tokens are
//! unavailable at that point. Lookup folds ASCII case and stops at the first NUL.

use std::{collections::BTreeSet, io, sync::Arc};

use super::reader::Reader;
use crate::ResolvedLayers;
use crate::parser::native::Token;

/// The static keyword `word` (the text up to its first NUL) names, lowered, if the lexer's table
/// holds it.
pub(super) fn static_keyword(word: &[u8]) -> Option<&'static str> {
    engine_executable::executable_tables::required(
        engine_executable::executable_tables::get()
            .keywords
            .as_ref(),
    )
    .static_keyword(word)
    .map(|(_, name)| name)
}

/// The dynamic tokens the lexer holds when the interface is read: those of
/// `common/synchronized_dynamic_tokens`, lowered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicTokens(Arc<BTreeSet<Vec<u8>>>);

impl DynamicTokens {
    /// Direct `.txt` files in filename byte order, each read as one string list.
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut files = Vec::new();
        // Byte paths preserve archive filenames that a text listing cannot represent.
        let directory = b"common/synchronized_dynamic_tokens/";
        for path in layers
            .listed_layer_paths(directory)?
            .into_iter()
            .filter(|path| {
                let name = &path[directory.len()..];
                !name.contains(&b'/') && name.ends_with(b".txt")
            })
        {
            files.push(layers.read_path(&path)?);
        }
        Ok(Self::from_files(files.iter().map(Vec::as_slice)))
    }

    /// Read strings from each file's start to a closing brace or the end.
    pub fn from_files<'a>(files: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut tokens = BTreeSet::new();
        for bytes in files {
            let mut reader = Reader::new(
                crate::parser::native::lex(bytes, crate::parser::native::Mode::KeepMark),
                0,
            );
            for name in reader.string_list() {
                // Empty names and names starting with a digit or minus sign are refused.
                if name
                    .first()
                    .is_some_and(|first| !first.is_ascii_digit() && *first != b'-')
                {
                    tokens.insert(name.to_ascii_lowercase());
                }
            }
        }
        Self(Arc::new(tokens))
    }

    fn contains(&self, word: &[u8]) -> bool {
        self.0.contains(&word.to_ascii_lowercase())
    }
}

/// A word token's keyword, number or string classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    /// A static keyword, by its lowered spelling.
    Static(&'static str),
    /// A dynamic token: an id the static table does not name.
    Dynamic,
    /// Numbered by its first byte, or a miss containing only digits, `-` and `.`.
    Number,
    /// Quoted, or a miss.
    String,
}

/// Classify a word token after quoted and numeric token handling.
pub(super) fn class(token: &Token, dynamic: &DynamicTokens) -> Class {
    if token.quoted() {
        return Class::String;
    }
    let Some(word) = token.lookup() else {
        return if token.is_number() {
            Class::Number
        } else {
            Class::String
        };
    };
    if let Some(name) = static_keyword(word) {
        Class::Static(name)
    } else if dynamic.contains(word) {
        Class::Dynamic
    } else if token.is_number() {
        Class::Number
    } else {
        Class::String
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::native::{Mode, lex as lex_mode};

    fn lex(bytes: &[u8]) -> Vec<Token> {
        lex_mode(bytes, Mode::KeepMark).tokens
    }

    #[test]
    fn dynamic_lists_fold_case_and_refuse_invalid_initial_bytes() {
        let dynamic =
            DynamicTokens::from_files([b"SyntheticToken \"-invalid\" 2invalid\n".as_slice()]);
        assert_eq!(
            dynamic.0.iter().cloned().collect::<Vec<_>>(),
            [b"synthetictoken".to_vec()]
        );
        assert!(dynamic.contains(b"SYNTHETICTOKEN"));
    }

    /// The supported table has ASCII names with unique folded identities.
    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn supported_static_keywords_keep_folded_identity() {
        let tables = engine_executable::executable_tables::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        let names = &tables.keywords.as_ref().unwrap().names;
        assert_eq!(names.len(), 10_767);
        assert!(names.iter().all(|(_, name)| name.is_ascii()));
        assert_eq!(static_keyword(b"SpriteTypes"), Some("spritetypes"));
        assert_eq!(static_keyword(b"SpriteTypes\0other"), Some("spritetypes"));
        assert_eq!(static_keyword(b"texture_diffuse"), None);
    }

    /// A word's class: quoted and missed words are strings, numbers by their first byte or their
    /// bytes, keywords by the fold, dynamic tokens once a list adds them.
    #[test]
    #[ignore = "requires BATTLE_PLAN_GAME_DIR"]
    fn words_are_classed_as_the_lexer_classes_them() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("set BATTLE_PLAN_GAME_DIR to the supported installation");
        // A word ending at the file's end supplies no token.
        let dynamic = DynamicTokens::from_files([&b"Coal \"-x\" 3y { kept\n"[..], b"cut"]);
        let tokens = lex(b"SIZE \"size\" texture_diffuse 12 1.5.- coal -coal\n");
        let classes: Vec<Class> = tokens[..7]
            .iter()
            .map(|token| class(token, &dynamic))
            .collect();
        assert_eq!(
            classes,
            [
                Class::Static("size"),
                Class::String,
                Class::String,
                Class::Number,
                Class::Number,
                Class::Dynamic,
                Class::Number,
            ]
        );
        // `-x` and `3y` start with a byte the registrar refuses; `kept` and `{` are strings.
        assert_eq!(
            dynamic.0.iter().cloned().collect::<Vec<_>>(),
            [b"coal".to_vec(), b"kept".to_vec(), b"{".to_vec()]
        );
    }
}
