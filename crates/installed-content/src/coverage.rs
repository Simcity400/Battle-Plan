//! What a content lookup knows when a file's reading stops short.
//!
//! The game reads every file to its end. When the port cannot follow a
//! file's reading past some place (a block read by key it cannot settle, a
//! modifier block the registry decides), whatever the rest of that file
//! defines or redefines is unknown. A lookup whose answer that part could
//! change is refused with the file and the cause, never answered from an
//! earlier layer or from absence.

use std::collections::BTreeMap;

/// A file whose reading the port cannot follow to its end, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gap {
    pub path: String,
    pub cause: String,
}

impl std::fmt::Display for Gap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is unknown from {}, so what it defines there is unknown",
            self.path, self.cause
        )
    }
}

/// Which file each token's definitions come from, and the files that stop
/// short, in load order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    file: usize,
    /// The file that decides a token: its last definition where a later one
    /// replaces an earlier one, its first where every definition counts.
    defined: BTreeMap<String, usize>,
    gaps: Vec<(usize, Gap)>,
}

impl Coverage {
    /// The next file in load order.
    pub fn next_file(&mut self) {
        self.file += 1;
    }

    /// The current file defines `token`, replacing earlier definitions.
    pub fn replace(&mut self, token: &str) {
        self.defined.insert(token.to_owned(), self.file);
    }

    /// The current file defines `token` as well: every definition counts,
    /// so the first decides.
    pub fn add(&mut self, token: &str) {
        self.defined.entry(token.to_owned()).or_insert(self.file);
    }

    /// The current file stops short.
    pub fn gap(&mut self, path: &str, cause: impl Into<String>) {
        self.gaps.push((
            self.file,
            Gap {
                path: path.to_owned(),
                cause: cause.into(),
            },
        ));
    }

    /// The last gap that could change what `token` is: one in or after the
    /// file that decides it, or any gap when no file defines it.
    pub fn unknown(&self, token: &str) -> Option<&Gap> {
        let from = self.defined.get(token).copied();
        self.gaps
            .iter()
            .rev()
            .find(|(file, _)| from.is_none_or(|from| *file >= from))
            .map(|(_, gap)| gap)
    }

    /// For a lookup that takes the first definition (fed with
    /// [`Self::add`]): the last gap in a file before the first that defines
    /// `token`, whose unread rest could hold an earlier one, or any gap when
    /// no file defines it.
    pub fn before_first(&self, token: &str) -> Option<&Gap> {
        let from = self.defined.get(token).copied();
        self.gaps
            .iter()
            .rev()
            .find(|(file, _)| from.is_none_or(|from| *file < from))
            .map(|(_, gap)| gap)
    }

    /// The current file in load order, as [`Self::next_file`] numbers them.
    pub fn file(&self) -> usize {
        self.file
    }

    /// The last gap in a file before `file`, whose unread rest could hold
    /// definitions that come before `file`'s.
    pub fn gap_before(&self, file: usize) -> Option<&Gap> {
        self.gaps
            .iter()
            .rev()
            .find(|(gap, _)| *gap < file)
            .map(|(_, gap)| gap)
    }

    /// The last gap of all: any search over every definition could change.
    pub fn any_gap(&self) -> Option<&Gap> {
        self.gaps.last().map(|(_, gap)| gap)
    }

    /// Every gap, in load order.
    pub fn gaps(&self) -> impl Iterator<Item = &Gap> {
        self.gaps.iter().map(|(_, gap)| gap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_hides_what_it_could_change() {
        let mut coverage = Coverage::default();
        coverage.replace("early");
        coverage.next_file();
        coverage.replace("both");
        coverage.gap("b.txt", "line 3: x");
        coverage.next_file();
        coverage.replace("late");
        coverage.replace("both");
        assert_eq!(coverage.unknown("early").unwrap().path, "b.txt");
        assert_eq!(coverage.unknown("absent").unwrap().path, "b.txt");
        assert!(coverage.unknown("late").is_none());
        assert!(coverage.unknown("both").is_none());
        let mut added = Coverage::default();
        added.add("first");
        added.next_file();
        added.gap("c.txt", "y");
        added.next_file();
        added.add("first");
        added.add("later");
        assert_eq!(added.unknown("first").unwrap().path, "c.txt");
        assert!(added.unknown("later").is_none());
    }
}
