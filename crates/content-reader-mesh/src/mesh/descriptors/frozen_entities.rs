use super::EntityDefinition;
use content_reader_layers::kept::Table;
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Default)]
pub(super) struct Entities {
    rows: Option<Table>,
}

impl Entities {
    pub(super) fn freeze(
        definitions: impl ExactSizeIterator<Item = (String, EntityDefinition)>,
    ) -> Self {
        let mut bytes = Vec::new();
        Table::write(&mut bytes, definitions.map(|(name, definition)| {
            (name, rmp_serde::to_vec(&definition).expect("encode an in-memory entity definition"))
        }));
        bytes.shrink_to_fit();
        let (rows, _) = Table::read(&Arc::new(bytes), 0).expect("index just-written entity definitions");
        Self { rows: Some(rows) }
    }

    pub(super) fn len(&self) -> usize {
        self.rows.as_ref().map_or(0, Table::len)
    }

    pub(super) fn names(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(|index| {
            let key = self.rows.as_ref().and_then(|rows| rows.key(index)).expect("entity index row");
            std::str::from_utf8(key).expect("in-memory entity name")
        })
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.index(name).is_some()
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.rows.as_ref()?.find(name.as_bytes())
    }

    pub(super) fn job(&self) -> Job<'_> {
        Job { entities: self, bodies: OnceLock::new() }
    }
}

pub(super) struct Job<'a> {
    entities: &'a Entities,
    bodies: OnceLock<Box<[OnceLock<Box<EntityDefinition>>]>>,
}

impl Job<'_> {
    pub(super) fn len(&self) -> usize {
        self.entities.len()
    }

    pub(super) fn contains(&self, name: &str) -> bool {
        self.entities.contains(name)
    }

    pub(super) fn get(&self, name: &str) -> Option<&EntityDefinition> {
        self.entities.index(name).map(|index| self.get_index(index))
    }

    pub(super) fn prepared(&self, name: &str) -> Option<&EntityDefinition> {
        let index = self.entities.index(name)?;
        self.bodies.get()?.get(index)?.get().map(Box::as_ref)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &EntityDefinition> {
        (0..self.len()).map(|index| self.get_index(index))
    }

    fn get_index(&self, index: usize) -> &EntityDefinition {
        let bodies = self.bodies.get_or_init(|| {
            std::iter::repeat_with(OnceLock::new).take(self.len()).collect()
        });
        bodies[index].get_or_init(|| {
            let bytes = self.entities.rows.as_ref().and_then(|rows| rows.value(index))
                .expect("indexed entity definition");
            Box::new(rmp_serde::from_slice(bytes).expect("decode a just-written entity definition"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexing_and_duplicate_reads_materialize_only_one_body_per_job() {
        let definitions = ["one", "two"].map(|name| (name.to_owned(), EntityDefinition {
            name: name.to_owned(),
            sources: vec![content_reader_layers::script_files::ScriptPath::new(vec![b'a', 0xff])],
            cull_radius: Some(-0.0),
            ..EntityDefinition::default()
        }));
        let entities = Entities::freeze(definitions.clone().into_iter());
        let job = entities.job();
        assert_eq!(entities.names().collect::<Vec<_>>(), ["one", "two"]);
        assert!(job.contains("one"));
        assert!(job.prepared("one").is_none());
        assert!(job.get("absent").is_none());
        assert!(job.bodies.get().is_none());

        let first = job.get("one").unwrap();
        assert_eq!(first, &definitions[0].1);
        assert_eq!(first.cull_radius.unwrap().to_bits(), (-0.0_f32).to_bits());
        assert!(std::ptr::eq(first, job.get("one").unwrap()));
        assert!(std::ptr::eq(first, job.prepared("one").unwrap()));
        assert_eq!(job.bodies.get().unwrap().iter().filter(|slot| slot.get().is_some()).count(), 1);
        let next_job = entities.job();
        assert!(next_job.bodies.get().is_none());
        assert_eq!(next_job.get("two").unwrap(), &definitions[1].1);
    }
}
