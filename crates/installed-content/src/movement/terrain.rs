//! The movement operands of the native terrain database.
//!
//! [`crate::terrain::read_definitions`] reads the database with the native
//! load, append and lookup rules; this module only derives what movement
//! samples from it: each category's one-based ordinal and each terrain bitmap
//! palette index's movement cost. `FindTerrainByHashedName` (`1403bca00`)
//! falls back to the null terrain, ordinal zero and cost zero. The ordinal is
//! the database's signed category count when the category was read
//! (`140AC7035` passes `+0x4C` to `CTerrainType::CTerrainType@141429930`,
//! which stores it as the i32 at `+0x88`), so a mod may declare any number of
//! categories.

use std::io;

use crate::{ResolvedLayers, terrain::TerrainDefinitions};

#[derive(Debug, Default)]
pub(crate) struct TerrainDatabase {
    definitions: TerrainDefinitions,
}

impl TerrainDatabase {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        Ok(Self {
            definitions: crate::terrain::read_definitions(layers)?,
        })
    }

    /// The one-based ordinal the name resolves to; zero is the null terrain.
    pub fn ordinal(&self, name: &[u8]) -> i32 {
        self.definitions
            .category(name)
            .map_or(0, |position| position as i32 + 1)
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
    TerrainDatabase { definitions }
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
    fn ordinals_past_a_byte_keep_counting() {
        // KaiserreduX declares 1,581 categories.
        let categories: String = (0..1581).map(|i| format!("c{i}={{}} ")).collect();
        let costs = database(&[&format!("categories={{{categories}}}")]);
        assert_eq!(costs.ordinal(b"c255"), 256);
        assert_eq!(costs.ordinal(b"C1580"), 1581);
    }
}
