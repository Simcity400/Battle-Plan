//! The movement operands of the native terrain database.
//!
//! [`crate::terrain::read_definitions`] reads the database with the native
//! load, append and lookup rules; this module only derives what movement
//! samples from it: each category's one-based ordinal and each terrain bitmap
//! palette index's movement cost. `FindTerrainByHashedName` (`1403bca00`)
//! falls back to the null terrain, ordinal zero and cost zero.

use std::io;

use crate::{ResolvedLayers, fields::invalid, terrain::TerrainDefinitions};

#[derive(Debug, Default)]
pub(crate) struct TerrainDatabase {
    definitions: TerrainDefinitions,
}

impl TerrainDatabase {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        Self::new(crate::terrain::read_definitions(layers)?)
    }

    fn new(definitions: TerrainDefinitions) -> io::Result<Self> {
        if definitions.categories.len() > usize::from(u8::MAX) {
            return Err(invalid(format!(
                "{} terrain categories exceed the one-byte terrain index",
                definitions.categories.len()
            )));
        }
        Ok(Self { definitions })
    }

    /// The one-based ordinal the name resolves to; zero is the null terrain.
    pub fn ordinal(&self, name: &[u8]) -> u8 {
        self.definitions
            .category(name)
            .map_or(0, |position| position as u8 + 1)
    }

    /// The movement cost of a terrain bitmap palette index. An index no entry
    /// writes selects the null terrain, whose cost is zero.
    pub fn cost(&self, index: u8) -> i64 {
        match self
            .definitions
            .palette
            .get(&index)
            .and_then(|entry| entry.category)
        {
            Some(at) => self.definitions.categories[at].movement_cost,
            None => 0,
        }
    }
}

#[cfg(test)]
pub(crate) fn database(sources: &[&str]) -> TerrainDatabase {
    let mut definitions = TerrainDefinitions::default();
    for source in sources {
        definitions.append(source.as_bytes()).unwrap();
    }
    TerrainDatabase::new(definitions).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn costs_and_ordinals_follow_the_database_rules() {
        let costs = database(&[
            "categories={first={movement_cost=1} First={movement_cost=9}} terrain={a={type=FIRST color={2}} early={type=later color={1}} bare={type=first}}",
            "categories={later={movement_cost=3} none={}} terrain={b={type=later color={2 4 300}} c={type=none color={5}} d={color={6}}}",
        ]);
        // A type resolves against the categories read so far, first match.
        assert_eq!(costs.cost(1), 0);
        assert_eq!(costs.cost(2), 300_000);
        assert_eq!(costs.cost(4), 300_000);
        assert_eq!(costs.cost(5), 0);
        assert_eq!(costs.cost(6), 0);
        assert_eq!(costs.cost(0), 0);
        assert_eq!(costs.cost(255), 0);
        assert_eq!(
            database(&["categories={x={movement_cost=1} X={movement_cost=9}} terrain={a={type=X color={0}}}"]).cost(0),
            100_000
        );
        // Ordinals are one-based across files; a repeated name keeps its own.
        assert_eq!(costs.ordinal(b"first"), 1);
        assert_eq!(costs.ordinal(b"LATER"), 3);
        assert_eq!(costs.ordinal(b"missing"), 0);
        let invalid =
            database(&["categories={bad={movement_cost=bad}} terrain={a={type=bad color={0}}}"]);
        // A cost that does not convert keeps the constructor's zero, which
        // the category then clamps to 0.01 (14142D0A0).
        assert_eq!(invalid.cost(0), 1_000);
    }

    #[test]
    fn more_categories_than_a_byte_are_refused() {
        let categories: String = (0..256).map(|i| format!("c{i}={{}} ")).collect();
        let mut definitions = TerrainDefinitions::default();
        definitions
            .append(format!("categories={{{categories}}}").as_bytes())
            .unwrap();
        assert!(TerrainDatabase::new(definitions).is_err());
    }
}
