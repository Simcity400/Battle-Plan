//! The names `common/country_tag_aliases` declares (the tag alias database
//! 143316AA8, loaded by 140187340).
//!
//! A tag alias names a country chosen at run time from its targets and
//! triggers (`original_tag`, `targets`, `variable`, country flags, focus
//! trees, a `country_score`, a `fallback`), none of which the recording
//! holds; a scope or scripted text naming an alias is refused by that name.
//! Only the names are read here: each top-level key of a file is an alias,
//! its block read by key (the trigger and effect key-validity gap).

use crate::{
    ResolvedLayers,
    parser::native::{self, Shape},
    script_files,
};
use std::{collections::BTreeSet, io};

/// Every alias name, and the files whose reading stops short (their later
/// names are unknown), each with its cause.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagAliases {
    pub names: BTreeSet<String>,
    pub unknown: Vec<String>,
}

/// One file's alias names, and where its reading stops short.
fn read_file(input: &[u8]) -> (Vec<String>, Option<String>) {
    // 140187340 constructs the lexer in mode 0 (CTextLexer 1424A6370).
    let read = native::read(input, native::Mode::KeepMark, (), &mut |(), field| {
        if field.opens() {
            Shape::ByKey
        } else {
            Shape::One
        }
    });
    let mut names = Vec::new();
    let mut stop = read.unsettled.map(|unsettled| unsettled.to_string());
    for item in &read.items {
        match item.key_str() {
            Ok(name) => names.push(name.to_owned()),
            // An alias name the port cannot hold: no tag the port reads can
            // equal it, but the lookup through it cannot be followed.
            Err(refused) => {
                stop.get_or_insert_with(|| format!("an alias name is {refused}"));
            }
        }
    }
    (names, stop)
}

/// The alias names of the content stack, its files listed as 140187340
/// lists them (`.txt`, depth 0; [`script_files::paths`]).
pub fn read(layers: &ResolvedLayers) -> io::Result<TagAliases> {
    let mut aliases = TagAliases::default();
    for path in script_files::paths(layers, "common/country_tag_aliases/", Some(".txt")) {
        let (names, stop) = read_file(&layers.read(&path)?);
        aliases.names.extend(names);
        if let Some(cause) = stop {
            aliases.unknown.push(format!("{path}: {cause}"));
        }
    }
    Ok(aliases)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_top_level_key_is_an_alias() {
        let (names, stop) = read_file(
            b"SPA = { original_tag = SPR OR = { has_country_flag = x } } \
              VIC = { original_tag = FRA has_focus_tree = vichy_french_focus }",
        );
        assert_eq!(names, ["SPA", "VIC"]);
        assert_eq!(stop, None);
    }

    /// An alias name that is not UTF-8 is a named gap, never dropped.
    #[test]
    fn an_alias_name_that_is_not_text_is_a_gap() {
        let (names, stop) = read_file(b"SPA = { } \"\xfeA\" = { }");
        assert_eq!(names, ["SPA"]);
        assert_eq!(
            stop.as_deref(),
            Some(
                "an alias name is the stored text `\\xfeA` is not UTF-8, which the port's text cannot hold"
            )
        );
    }
}
