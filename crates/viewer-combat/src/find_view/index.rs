//! The find view's search, as the game runs it on every edit.
//!
//! The index holds, in order, every country owning at least one state (its name, then its tag),
//! every state (its name, then its id) and every province with victory points and a name. A
//! query shorter than two bytes finds nothing unless it is one digit. The query and every name
//! and secondary are folded alike ([`Fold`]); an entry matches where the query first occurs in
//! its name, else in its secondary, and the matches are listed in a stable order of that
//! position, every one of them.

use std::collections::BTreeMap;

/// What a result centres the camera on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A country: its capital state, which it may lack.
    Country { capital: Option<u32> },
    State(u32),
    Province(u32),
}

/// One searchable entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub secondary: String,
    /// The result's text as the game builds it.
    pub display: String,
    pub target: Target,
}

/// The game's search folding. The game maps each accented letter to an ASCII letter from a
/// table in its executable before lower-casing ASCII; where the recording withholds that table,
/// only ASCII lower-casing applies, and a text holding a letter outside ASCII is refused rather
/// than folded by a guess.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fold<'a> {
    table: Option<&'a BTreeMap<u32, u8>>,
}

/// Why a query finds nothing that the game would find something for.
pub const UNFOLDED_QUERY: &str = "the query holds a letter outside ASCII, and the \
     game's accent folding table is withheld";
/// Why entries are left out of the index.
pub const UNFOLDED_NAMES: &str = "names holding letters outside ASCII are left out of \
     the results, as the game's accent folding table is withheld";

impl<'a> Fold<'a> {
    /// Folding with the game's table: code point to ASCII letter.
    pub fn with_table(table: &'a BTreeMap<u32, u8>) -> Self {
        Self { table: Some(table) }
    }

    /// `text` folded to the bytes the game compares, or `None` when it cannot be folded.
    pub fn apply(&self, text: &str) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            if ch.is_ascii() {
                out.push(ch.to_ascii_lowercase() as u8);
                continue;
            }
            match self.table.map(|table| table.get(&u32::from(ch))) {
                Some(Some(&letter)) => out.push(letter.to_ascii_lowercase()),
                Some(None) => {
                    let mut bytes = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut bytes).as_bytes());
                }
                None if ch.is_alphabetic() => return None,
                None => {
                    let mut bytes = [0; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut bytes).as_bytes());
                }
            }
        }
        Some(out)
    }
}

struct Folded {
    entry: usize,
    name: Vec<u8>,
    secondary: Vec<u8>,
}

/// The entries of one presented hour, folded once.
#[derive(Default)]
pub struct Index {
    entries: Vec<Entry>,
    folded: Vec<Folded>,
    /// Entries left out because they cannot be folded.
    pub unfolded: usize,
}

/// A search's results, or why it was refused.
pub type Found<'i> = Result<Vec<&'i Entry>, &'static str>;

impl Index {
    pub fn build(entries: Vec<Entry>, fold: Fold<'_>) -> Self {
        let mut unfolded = 0;
        let folded = entries
            .iter()
            .enumerate()
            .filter_map(|(entry, value)| {
                let folded = fold
                    .apply(&value.name)
                    .zip(fold.apply(&value.secondary))
                    .map(|(name, secondary)| Folded {
                        entry,
                        name,
                        secondary,
                    });
                if folded.is_none() {
                    unfolded += 1;
                }
                folded
            })
            .collect();
        Self {
            entries,
            folded,
            unfolded,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The results for `query`.
    pub fn search(&self, query: &str, fold: Fold<'_>) -> Found<'_> {
        let long_enough =
            query.len() >= 2 || (query.len() == 1 && query.bytes().all(|b| b.is_ascii_digit()));
        if !long_enough {
            return Ok(Vec::new());
        }
        let needle = fold.apply(query).ok_or(UNFOLDED_QUERY)?;
        let mut found: Vec<(usize, usize)> = self
            .folded
            .iter()
            .filter_map(|entry| {
                let at = position(&entry.name, &needle)
                    .or_else(|| position(&entry.secondary, &needle))?;
                Some((at, entry.entry))
            })
            .collect();
        found.sort_by_key(|&(at, _)| at);
        Ok(found
            .into_iter()
            .map(|(_, entry)| &self.entries[entry])
            .collect())
    }
}

/// Where `needle` first occurs in `haystack`.
fn position(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, secondary: &str, target: Target) -> Entry {
        Entry {
            name: name.to_owned(),
            secondary: secondary.to_owned(),
            display: name.to_owned(),
            target,
        }
    }

    fn index() -> Index {
        Index::build(
            vec![
                entry("Germany", "GER", Target::Country { capital: Some(64) }),
                entry("Argentina", "ARG", Target::Country { capital: Some(1) }),
                entry("Brandenburg", "64", Target::State(64)),
                entry("Bergen", "12", Target::State(12)),
                entry("Berlin", "", Target::Province(6521)),
            ],
            Fold::default(),
        )
    }

    fn names(found: Found<'_>) -> Vec<&str> {
        found.unwrap().iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn a_one_letter_query_finds_nothing_but_one_digit_does() {
        let index = index();
        assert!(index.search("b", Fold::default()).unwrap().is_empty());
        assert_eq!(names(index.search("6", Fold::default())), ["Brandenburg"]);
        assert!(index.search("", Fold::default()).unwrap().is_empty());
    }

    #[test]
    fn a_name_matches_before_its_secondary_and_results_follow_the_match_position() {
        let index = index();
        assert_eq!(names(index.search("BE", Fold::default())), ["Bergen", "Berlin"]);
        // "er" is at 1 in Germany, Bergen and Berlin; ties keep the index order: countries, then
        // states, then provinces.
        assert_eq!(
            names(index.search("er", Fold::default())),
            ["Germany", "Bergen", "Berlin"]
        );
        // An earlier match comes first whatever the kind: "an" is at 2 in Brandenburg, 4 in
        // Germany.
        assert_eq!(
            names(index.search("an", Fold::default())),
            ["Brandenburg", "Germany"]
        );
        // A tag or id is searched only when the name holds no match.
        assert_eq!(names(index.search("ger", Fold::default())), ["Germany"]);
        assert_eq!(names(index.search("12", Fold::default())), ["Bergen"]);
    }

    #[test]
    fn every_match_is_listed() {
        let entries = (0..1500)
            .map(|n| entry(&format!("Land {n}"), "", Target::State(n)))
            .collect();
        let index = Index::build(entries, Fold::default());
        assert_eq!(index.search("la", Fold::default()).unwrap().len(), 1500);
    }

    #[test]
    fn letters_outside_ascii_are_refused_until_the_fold_table_is_recorded() {
        let index = Index::build(
            vec![
                entry("Bavière", "", Target::State(1)),
                entry("São Paulo", "", Target::State(2)),
                entry("Köln – Bonn", "", Target::State(3)),
                entry("Lyon", "", Target::State(4)),
            ],
            Fold::default(),
        );
        assert_eq!(index.unfolded, 3);
        assert_eq!(names(index.search("ly", Fold::default())), ["Lyon"]);
        assert_eq!(index.search("bavi\u{e8}", Fold::default()), Err(UNFOLDED_QUERY));
        // A punctuation mark outside ASCII is no letter: it keeps its bytes.
        assert_eq!(Fold::default().apply("a – b").unwrap(), "a – b".as_bytes());
        // With the table, accented letters fold to their ASCII letter before lower-casing.
        let table = BTreeMap::from([(0xe8, b'e'), (0xe3, b'a'), (0xf6, b'o')]);
        let fold = Fold::with_table(&table);
        let folded = Index::build(index.entries().to_vec(), fold);
        assert_eq!(folded.unfolded, 0);
        assert_eq!(names(folded.search("baviere", fold)), ["Bavière"]);
        assert_eq!(names(folded.search("SAO", fold)), ["São Paulo"]);
    }
}
