
use std::{borrow::Cow, io};

use content_reader_layers::ResolvedLayers;
use content_reader_key_names::terrain::TerrainDefinitions;

#[derive(Debug, Default)]
pub struct TerrainDatabase<'a> {
    definitions: Cow<'a, TerrainDefinitions>,
}

impl<'a> TerrainDatabase<'a> {
    pub fn read(layers: &ResolvedLayers) -> io::Result<Self> {
        Ok(Self {
            definitions: Cow::Owned(content_reader_key_names::terrain::read_definitions(layers)?),
        })
    }

    pub fn from_definitions(definitions: &'a TerrainDefinitions) -> Self {
        Self {
            definitions: Cow::Borrowed(definitions),
        }
    }

    pub fn ordinal(&self, name: &[u8]) -> i32 {
        self.definitions
            .category(name)
            .map_or(0, |position| position as i32 + 1)
    }

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
pub(crate) fn database(sources: &[&str]) -> TerrainDatabase<'static> {
    let mut definitions = TerrainDefinitions::default();
    for source in sources {
        definitions.append(source.as_bytes()).unwrap();
    }
    TerrainDatabase {
        definitions: Cow::Owned(definitions),
    }
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
        assert_eq!(costs.ordinal(b"first"), 1);
        assert_eq!(costs.ordinal(b"LATER"), 3);
        assert_eq!(costs.ordinal(b"missing"), 0);
        let shared = TerrainDatabase::from_definitions(costs.definitions.as_ref());
        for index in 0..=u8::MAX {
            assert_eq!(shared.cost(index), costs.cost(index));
        }
        for name in [b"first".as_slice(), b"LATER", b"missing"] {
            assert_eq!(shared.ordinal(name), costs.ordinal(name));
        }
        let invalid =
            database(&["categories={bad={movement_cost=bad}} terrain={a={type=bad color={0}}}"]);
        assert_eq!(invalid.cost(0), 1_000);
    }

    #[test]
    fn ordinals_past_a_byte_keep_counting() {
        let categories: String = (0..1581).map(|i| format!("c{i}={{}} ")).collect();
        let costs = database(&[&format!("categories={{{categories}}}")]);
        assert_eq!(costs.ordinal(b"c255"), 256);
        assert_eq!(costs.ordinal(b"C1580"), 1581);
    }
}
