
use std::{collections::BTreeSet, io, sync::Arc};

use super::reader::Reader;
use content_reader_layers::ResolvedLayers;
use content_reader_script::native::Token;

pub(super) fn static_keyword(word: &[u8]) -> Option<&'static str> {
    engine_executable::executable_tables::required(
        engine_executable::executable_tables::get()
            .keywords
            .as_ref(),
    )
    .static_keyword(word)
    .map(|(_, name)| name)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DynamicTokens(Arc<BTreeSet<Vec<u8>>>);

impl DynamicTokens {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.product("interface dynamic tokens", 1, &[], Self::read_tokens).map(|tokens| tokens.as_ref().clone())
    }

    fn read_tokens(layers: &ResolvedLayers) -> io::Result<Self> {
        let mut files = Vec::new();
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
        Ok(Self::from_files(files.iter().map(|bytes| bytes.as_ref())))
    }

    pub fn from_files<'a>(files: impl IntoIterator<Item = &'a [u8]>) -> Self {
        let mut tokens = BTreeSet::new();
        for bytes in files {
            let mut reader = Reader::new(
                content_reader_script::native::lex(bytes, content_reader_script::native::Mode::KeepMark),
                0,
            );
            for name in reader.string_list() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Class {
    Static(&'static str),
    Dynamic,
    Number,
    String,
}

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
    use content_reader_script::native::{Mode, lex as lex_mode};

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

    #[test]
    fn words_are_classed_as_the_lexer_classes_them() {
        content_reader_script::test_game::enter();
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
        assert_eq!(
            dynamic.0.iter().cloned().collect::<Vec<_>>(),
            [b"coal".to_vec(), b"kept".to_vec(), b"{".to_vec()]
        );
    }
}
