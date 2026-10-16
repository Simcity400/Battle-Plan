//! Name groups for a raid's recorded division or source ship. The four name
//! directories feed one database, shared with [`crate::division_names`].
//! Groups are found without case, repeated declarations merge, and each
//! group's `prefix` is prepended to both entry arrays after every declaration.
//! Names are literal authored text.
use crate::{ResolvedLayers, division_names};
use std::io;

/// The database, and what reading it logged or refused.
#[derive(Debug, Default)]
pub struct Groups {
    database: division_names::Database,
    /// What the game only logs about a file, and the groups of a file the
    /// port cannot follow to its end, by file.
    pub warnings: Vec<crate::Diagnostic>,
}

/// Why a refused group's names are refused.
const REFUSED: &str = "a name file that declares or could declare it was refused";

impl Groups {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        let (database, warnings) =
            division_names::read_directories(layers, &division_names::DIRECTORIES)?;
        Ok(Self { database, warnings })
    }

    /// Each group whose reading is refused, by its folded name, and why:
    /// every name it gives is refused.
    pub fn refused(&self) -> impl Iterator<Item = (&str, &str)> {
        self.database.refused().map(|name| {
            (
                std::str::from_utf8(name).unwrap_or("<non-UTF-8 name>"),
                REFUSED,
            )
        })
    }

    /// The member's name from the group of its recorded key, in whichever
    /// table holds it.
    pub fn name(&self, key: &str, order: i32, ordered: bool) -> Result<String, String> {
        self.database
            .resolve_any(key, order, ordered)
            .map_err(|why| format!("name group {key}: {why}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name files in loader order.
    fn groups(files: &[(&str, &str)]) -> Groups {
        let texts: Vec<&[u8]> = files.iter().map(|(_, text)| text.as_bytes()).collect();
        Groups {
            database: division_names::Database::from_files(&texts),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn merged_groups_replace_ordered_append_unordered_and_keep_literal_text() {
        let g = groups(&[(
            "common/units/names_divisions/a.txt",
            r#"x={ordered={4={"old"}} unordered={a={"A"}} fallback_name="%d. X"}
            X={ordered={4={"%s/%d" "description" "url"}} unique={"B"}}"#,
        )]);
        // `x` and `X` are one group: group lookup ignores case.
        assert_eq!(g.name("x", 4, true).unwrap(), "IV/%d");
        assert_eq!(g.name("X", 1, false).unwrap(), "B");
        assert_eq!(g.name("x", 2, false).unwrap(), "3. X");
        assert!(g.name("x", 0, true).is_err(), "an unassigned order");
    }

    #[test]
    fn every_directory_feeds_one_database_and_the_prefix_is_finalised() {
        let g = groups(&[
            (
                "common/units/names_ships/s.txt",
                r#"ship={type=ship prefix="PRE " ship_types={test_hull} unique={"First" "Second"} unordered={a={"Third"}}}"#,
            ),
            (
                "common/units/codenames_operatives/c.txt",
                r#"Agents={type=codename unique={"Nightjar"}}"#,
            ),
        ]);
        // Finalisation prepends the prefix to both arrays.
        for (at, name) in ["PRE First", "PRE Second", "PRE Third"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(g.name("ship", at as i32, false).unwrap(), name);
        }
        assert_eq!(g.name("agents", 0, false).unwrap(), "Nightjar");
        assert!(g.name("absent", 0, false).unwrap_err().contains("absent"));
        assert!(g.name("ship", 5, false).unwrap_err().contains("fallback"));
    }
}
