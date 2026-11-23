
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, ViewIdentity, view_reads};
use std::sync::{Arc, Mutex};

pub struct Carried<V, E> {
    held: Mutex<Option<Held<V, E>>>,
}

struct Held<V, E> {
    reads: Vec<(TableId, ViewIdentity)>,
    extra: E,
    value: Arc<V>,
}

impl<V, E> Default for Carried<V, E> {
    fn default() -> Self {
        Self { held: Mutex::new(None) }
    }
}

impl<V, E: PartialEq> Carried<V, E> {
    pub fn get_or_build<Error>(
        &self,
        frame: &DecodedFrame,
        also: &[TableId],
        extra: E,
        build: impl FnOnce() -> Result<V, Error>,
    ) -> Result<Arc<V>, Error> {
        {
            let held = self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(last) = held.as_ref()
                && last.extra == extra
                && last.reads.iter().all(|(table, identity)| identity.is(&frame.tables[table.index()]))
            {
                for (table, _) in &last.reads {
                    let _ = &*frame.tables[table.index()];
                }
                return Ok(Arc::clone(&last.value));
            }
        }
        let (value, views, walked) = view_reads::record_walked(|| {
            for table in also {
                let _ = &*frame.tables[table.index()];
            }
            build()
        });
        let value = Arc::new(value?);
        *self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = walked.is_empty().then(|| Held {
            reads: TableId::ALL
                .iter()
                .filter_map(|&table| {
                    let view = &frame.tables[table.index()];
                    views.iter().find(|read| read.is(view)).map(|read| (table, read.clone()))
                })
                .collect(),
            extra,
            value: Arc::clone(&value),
        });
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Arc, Carried, DecodedFrame, TableId};
    use replay_data::rows::{TableRows, tables};
    use std::cell::Cell;
    use viewer_foundation::test_support::{test_frame, test_put};
    use dashboard_text::table_access::rows;

    #[test]
    fn a_part_is_carried_while_what_it_read_stands_and_built_again_when_any_of_it_changes() {
        let mut frame = test_frame(3);
        test_put(&mut frame, TableRows::SentVolunteers(vec![replay_data::rows::SentVolunteerRow { division: 4, ..Default::default() }]));
        let carried = Carried::<usize, u32>::default();
        let builds = Cell::new(0);
        let part = |frame: &DecodedFrame, extra: u32| {
            carried
                .get_or_build(frame, &[TableId::CountryLinks], extra, || {
                    builds.set(builds.get() + 1);
                    rows::<tables::SentVolunteers>(frame).map(|rows| rows.rows().len())
                })
                .unwrap()
        };
        assert_eq!(*part(&frame, 1), 1);
        assert_eq!((*part(&frame, 1), builds.get()), (1, 1), "nothing it read changed");
        test_put(&mut frame, TableRows::Divisions(Vec::new()));
        assert_eq!((*part(&frame, 1), builds.get()), (1, 1), "a table it did not read changed");
        assert_eq!((*part(&frame, 2), builds.get()), (1, 2), "its extra key changed");
        test_put(&mut frame, TableRows::SentVolunteers(Vec::new()));
        assert_eq!((*part(&frame, 2), builds.get()), (0, 3), "a table it read changed");
        test_put(&mut frame, TableRows::CountryLinks(Vec::new()));
        assert_eq!((*part(&frame, 2), builds.get()), (0, 4), "a table its derived input came from changed");
        assert_eq!((*part(&frame, 2), builds.get()), (0, 4));
    }

    #[test]
    fn a_part_is_built_with_no_lock_held() {
        let frame = test_frame(3);
        let carried = Carried::<usize, u32>::default();
        let value = carried
            .get_or_build(&frame, &[], 1, || {
                assert!(carried.held.try_lock().is_ok(), "no lock is held while the part builds");
                Ok::<_, ()>(5)
            })
            .unwrap();
        assert_eq!(*value, 5);
        assert!(Arc::ptr_eq(&value, &carried.get_or_build(&frame, &[], 1, || Ok::<_, ()>(6)).unwrap()), "held for the next hour");
    }
}
