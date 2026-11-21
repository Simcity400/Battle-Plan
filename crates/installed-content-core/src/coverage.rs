
use std::collections::BTreeMap;

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    file: usize,
    defined: BTreeMap<String, usize>,
    gaps: Vec<(usize, Gap)>,
}

impl Coverage {
    pub fn next_file(&mut self) {
        self.file += 1;
    }

    pub fn replace(&mut self, token: &str) {
        self.defined.insert(token.to_owned(), self.file);
    }

    pub fn add(&mut self, token: &str) {
        self.defined.entry(token.to_owned()).or_insert(self.file);
    }

    pub fn gap(&mut self, path: &str, cause: impl Into<String>) {
        self.gaps.push((
            self.file,
            Gap {
                path: path.to_owned(),
                cause: cause.into(),
            },
        ));
    }

    pub fn unknown(&self, token: &str) -> Option<&Gap> {
        let from = self.defined.get(token).copied();
        self.gaps
            .iter()
            .rev()
            .find(|(file, _)| from.is_none_or(|from| *file >= from))
            .map(|(_, gap)| gap)
    }

    pub fn before_first(&self, token: &str) -> Option<&Gap> {
        let from = self.defined.get(token).copied();
        self.gaps
            .iter()
            .rev()
            .find(|(file, _)| from.is_none_or(|from| *file < from))
            .map(|(_, gap)| gap)
    }

    pub fn file(&self) -> usize {
        self.file
    }

    pub fn gap_before(&self, file: usize) -> Option<&Gap> {
        self.gaps
            .iter()
            .rev()
            .find(|(gap, _)| *gap < file)
            .map(|(_, gap)| gap)
    }

    pub fn any_gap(&self) -> Option<&Gap> {
        self.gaps.last().map(|(_, gap)| gap)
    }

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
