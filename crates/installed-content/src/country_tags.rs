//! The country tags content declares (`common/country_tags`).

use crate::{ResolvedLayers, parser};
use std::{collections::BTreeMap, io};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The tags content declares, and the keys the engine's reader skips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagIndex {
    /// Every declared tag and its country file, `common/` plus the value.
    pub files: BTreeMap<String, String>,
    /// Keys that are not three bytes, as `<path>: <key>`: the engine declares no tag
    /// for them, so neither does this reader, but they are counted, not dropped silently.
    pub skipped: Vec<String>,
}

impl TagIndex {
    /// The skipped keys as one counted warning, naming the first few.
    pub fn warning(&self) -> Option<String> {
        if self.skipped.is_empty() {
            return None;
        }
        let named: Vec<&str> = self.skipped.iter().take(8).map(String::as_str).collect();
        Some(format!(
            "{} common/country_tags keys are not three bytes, so the game declares no tag for them: {}{}",
            self.skipped.len(),
            named.join("; "),
            if self.skipped.len() > named.len() {
                "; …"
            } else {
                ""
            }
        ))
    }
}

/// `common/country_tags/*.txt`, as `ccountry_database::load_tags` (`14070E900`)
/// reads it: every declared tag and its country file, `common/` plus the value.
/// The key `dynamic_tags` (token 0x309F) is not a tag: it only marks where the
/// dynamic-country slots begin, and the tags after it are still declared. A
/// key must be exactly three bytes (any other is skipped by the engine, and here
/// counted in [`TagIndex::skipped`]), and the first declaration of a tag wins.
/// A block value is a form the engine's reader was not traced on, so it is
/// refused rather than guessed.
pub fn country_tag_index(layers: &ResolvedLayers) -> io::Result<TagIndex> {
    let indexes: Vec<String> = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("common/country_tags/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect();
    let mut index = TagIndex::default();
    for path in indexes {
        add_tag_index(&path, &layers.read(&path)?, &mut index)?;
    }
    Ok(index)
}

/// One index file's declarations, added in file order; the first wins.
fn add_tag_index(path: &str, bytes: &[u8], index: &mut TagIndex) -> io::Result<()> {
    let document = parser::parse(bytes).map_err(|error| invalid(format!("{path}: {error}")))?;
    for item in &document.items {
        let Some(key) = item.key.as_deref() else {
            continue;
        };
        // A lexer token, in any ASCII case (`parser::is_keyword`).
        if key.eq_ignore_ascii_case(b"dynamic_tags") {
            continue;
        }
        if key.len() != 3 {
            index
                .skipped
                .push(format!("{path}: {}", String::from_utf8_lossy(key)));
            continue;
        }
        let file = item.value.scalar_str().ok_or_else(|| {
            invalid(format!(
                "{path}: country tag {} names its file with an unsupported block",
                String::from_utf8_lossy(key)
            ))
        })?;
        index
            .files
            .entry(String::from_utf8_lossy(key).into_owned())
            .or_insert_with(|| format!("common/{}", file.trim_matches('"')));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tag_index_skips_the_dynamic_marker_and_keeps_the_first_declaration() {
        let mut index = TagIndex::default();
        add_tag_index(
            "00_countries.txt",
            b"GER = \"countries/Germany.txt\"\nLONGKEY = \"x.txt\"\n",
            &mut index,
        )
        .unwrap();
        add_tag_index(
            "zz_dynamic_countries.txt",
            b"DYNAMIC_TAGS = yes\nD01 = \"countries/D01.txt\"\nGER = \"countries/Other.txt\"\n",
            &mut index,
        )
        .unwrap();
        // A key that is not three bytes declares no tag, and is counted.
        assert_eq!(index.skipped, ["00_countries.txt: LONGKEY"]);
        assert!(
            index
                .warning()
                .unwrap()
                .starts_with("1 common/country_tags keys")
        );
        assert_eq!(
            index.files,
            BTreeMap::from([
                ("D01".to_owned(), "common/countries/D01.txt".to_owned()),
                ("GER".to_owned(), "common/countries/Germany.txt".to_owned()),
            ])
        );
        assert!(add_tag_index("bad.txt", b"SOV = { a = b }", &mut index).is_err());
    }
}
