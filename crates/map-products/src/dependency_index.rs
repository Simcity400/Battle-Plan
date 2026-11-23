use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, ViewIdentity, view_reads};
use std::{
    collections::BTreeMap,
    io,
    ops::Bound::{Excluded, Unbounded},
    sync::{Arc, Mutex, Weak},
};

#[derive(Clone, Copy)]
pub(crate) enum Clock {
    Stable,
    Hour,
    Relations,
}
struct Dependency {
    table: TableId,
    identity: ViewIdentity,
    fields: u64,
}
struct Entry<V> {
    source: ViewIdentity,
    branch: u64,
    from: i64,
    until: i64,
    dependencies: Vec<Dependency>,
    value: Weak<V>,
}
struct Roots<K, V> {
    by_key: BTreeMap<K, Entry<V>>,
    latest: Option<(K, Arc<V>)>,
    sweep: Option<K>,
}
pub struct DependencyIndex<K, V> {
    roots: Mutex<Roots<K, V>>,
}
impl<K, V> Default for DependencyIndex<K, V> {
    fn default() -> Self {
        Self {
            roots: Mutex::new(Roots {
                by_key: BTreeMap::new(),
                latest: None,
                sweep: None,
            }),
        }
    }
}
impl<K: Ord + Clone, V> DependencyIndex<K, V> {
    #[cfg(test)]
    pub fn project(
        &self,
        frame: &DecodedFrame,
        key: K,
        clock: Clock,
        build: impl FnOnce() -> io::Result<V>,
    ) -> io::Result<Arc<V>> {
        self.project_reusing(
            frame,
            key,
            clock,
            |table| table.full_mask(),
            build,
            |_, _| false,
        )
    }

    #[cfg(test)]
    pub fn project_equal(
        &self,
        frame: &DecodedFrame,
        key: K,
        clock: Clock,
        build: impl FnOnce() -> io::Result<V>,
    ) -> io::Result<Arc<V>>
    where
        V: PartialEq,
    {
        self.project_reusing(
            frame,
            key,
            clock,
            |table| table.full_mask(),
            build,
            |a, b| a == b,
        )
    }

    pub fn project_fields(
        &self,
        frame: &DecodedFrame,
        key: K,
        clock: Clock,
        fields: impl Fn(TableId) -> u64,
        build: impl FnOnce() -> io::Result<V>,
    ) -> io::Result<Arc<V>> {
        self.project_reusing(frame, key, clock, fields, build, |_, _| false)
    }

    pub fn project_equal_fields(
        &self,
        frame: &DecodedFrame,
        key: K,
        clock: Clock,
        fields: impl Fn(TableId) -> u64,
        build: impl FnOnce() -> io::Result<V>,
    ) -> io::Result<Arc<V>>
    where
        V: PartialEq,
    {
        self.project_reusing(frame, key, clock, fields, build, |a, b| a == b)
    }

    fn project_reusing(
        &self,
        frame: &DecodedFrame,
        key: K,
        clock: Clock,
        fields: impl Fn(TableId) -> u64,
        build: impl FnOnce() -> io::Result<V>,
        equivalent: impl FnOnce(&V, &V) -> bool,
    ) -> io::Result<Arc<V>> {
        let mut roots = self.roots.lock().unwrap_or_else(|error| error.into_inner());
        let next = roots
            .by_key
            .range((roots.sweep.as_ref().map_or(Unbounded, Excluded), Unbounded))
            .next()
            .or_else(|| roots.by_key.first_key_value())
            .map(|(key, entry)| (key.clone(), entry.value.strong_count() == 0));
        if let Some((key, expired)) = next {
            if expired {
                roots.by_key.remove(&key);
            }
            roots.sweep = Some(key);
        }
        let hour = i64::from(frame.hour);
        if let Some(entry) = roots.by_key.get_mut(&key).filter(|entry| {
            entry.source.is(&frame.tables[TableId::Campaign.index()])
                && entry.branch == frame.branch
                && entry.from <= hour
                && hour < entry.until
        }) {
            if entry.dependencies.iter().all(|dependency| {
                let view = &frame.tables[dependency.table.index()];
                dependency
                    .identity
                    .fields_unchanged(view, dependency.fields)
            }) {
                if let Some(value) = entry.value.upgrade() {
                    for dependency in &mut entry.dependencies {
                        let view = &frame.tables[dependency.table.index()];
                        dependency.identity = view.identity();
                        let _ = &**view;
                    }
                    return Ok(value);
                }
            }
        }
        let (value, reads) = view_reads::record(build);
        let value = value?;
        let previous = roots
            .by_key
            .get(&key)
            .filter(|entry| {
                entry.branch == frame.branch
                    && entry.source.is(&frame.tables[TableId::Campaign.index()])
            })
            .and_then(|entry| entry.value.upgrade());
        let value = match previous {
            Some(previous) if equivalent(&previous, &value) => previous,
            _ => Arc::new(value),
        };
        let mut dependencies = Vec::with_capacity(reads.len());
        dependencies.extend(TableId::ALL.iter().filter_map(|&table| {
            let view = &frame.tables[table.index()];
            reads
                .iter()
                .find(|read| read.is(view))
                .map(|identity| Dependency {
                    table,
                    identity: identity.clone(),
                    fields: fields(table),
                })
        }));
        drop(reads);
        let (from, until) = validity(frame, clock, &dependencies);
        if let Some((previous_key, previous)) = roots.latest.take() {
            drop(previous);
            if roots
                .by_key
                .get(&previous_key)
                .is_some_and(|entry| entry.value.strong_count() == 0)
            {
                roots.by_key.remove(&previous_key);
            }
        }
        roots.by_key.insert(
            key.clone(),
            Entry {
                source: frame.tables[TableId::Campaign.index()].identity(),
                branch: frame.branch,
                from,
                until,
                dependencies,
                value: Arc::downgrade(&value),
            },
        );
        roots.latest = Some((key, Arc::clone(&value)));
        Ok(value)
    }
}

fn validity(frame: &DecodedFrame, clock: Clock, reads: &[Dependency]) -> (i64, i64) {
    let (mut from, mut until) = (i64::from(i32::MIN), i64::from(i32::MAX) + 1);
    if matches!(clock, Clock::Hour) {
        return (i64::from(frame.hour), i64::from(frame.hour) + 1);
    }
    if matches!(clock, Clock::Stable) {
        return (from, until);
    }
    let mut cut = |boundary: i32| {
        if boundary <= frame.hour {
            from = from.max(i64::from(boundary));
        } else {
            until = until.min(i64::from(boundary));
        }
    };
    for dependency in reads {
        match dependency.table {
            TableId::Subjects => {
                if let Ok(rows) = viewer_foundation::tables::rows::<tables::Subjects>(frame) {
                    for row in rows
                        .rows()
                        .iter()
                        .filter(|row| row.cancel == 0 && row.end_hour > row.start_hour)
                    {
                        cut(row.end_hour);
                    }
                }
            }
            TableId::WarRelations => {
                if let Ok(rows) =
                    viewer_foundation::tables::rows::<tables::WarRelations>(frame)
                {
                    for row in rows.rows() {
                        cut(row.start_hour);
                        if row.end_hour > row.start_hour {
                            cut(row.end_hour);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    (from, until)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{SubjectRow, TableRows};
    use replay_frame::frame::{Metrics, TableView};
    fn frame(hour: i32) -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
        }
    }
    #[test]
    fn reads_are_weak_hits_propagate_and_expired_selectors_do_not_accumulate() {
        let cache = DependencyIndex::default();
        let frame = frame(5);
        let build = || {
            let _ = &*frame.tables[TableId::Countries.index()];
            Ok(7u32)
        };
        let first = cache.project(&frame, 1, Clock::Stable, build).unwrap();
        let (second, reads) =
            view_reads::record(|| cache.project(&frame, 1, Clock::Stable, || panic!("must carry")));
        assert!(Arc::ptr_eq(&first, &second.unwrap()));
        assert!(
            reads
                .iter()
                .any(|read| read.is(&frame.tables[TableId::Countries.index()]))
        );
        drop(first);
        for key in 2..=5000 {
            let _ = cache.project(&frame, key, Clock::Stable, build).unwrap();
        }
        assert_eq!(cache.roots.lock().unwrap().by_key.len(), 1);
        for view in frame.tables {
            assert!(view.try_unwrap().is_ok());
        }
    }
    #[test]
    fn equal_outputs_keep_roots_but_replace_source_dependencies() {
        let cache = DependencyIndex::default();
        let mut frame = frame(8);
        let read = |frame: &DecodedFrame| {
            let _ = &*frame.tables[TableId::Countries.index()];
            Ok(7u32)
        };
        let first = cache
            .project_equal(&frame, (), Clock::Stable, || read(&frame))
            .unwrap();
        frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::empty(TableId::Countries)).into();
        let second = cache
            .project_equal(&frame, (), Clock::Stable, || read(&frame))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        let (third, reads) = view_reads::record(|| {
            cache.project_equal(&frame, (), Clock::Stable, || {
                panic!("new source identity must carry")
            })
        });
        assert!(Arc::ptr_eq(&first, &third.unwrap()));
        assert!(
            reads
                .iter()
                .any(|read| read.is(&frame.tables[TableId::Countries.index()]))
        );
        frame.branch += 1;
        let other = cache
            .project_equal(&frame, (), Clock::Stable, || read(&frame))
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn expired_live_selectors_are_swept_without_a_roster_scan() {
        let cache = DependencyIndex::default();
        let frame = frame(8);
        let held: Vec<_> = (0..1501)
            .map(|key| {
                cache
                    .project(&frame, key, Clock::Stable, || Ok(key))
                    .unwrap()
            })
            .collect();
        drop(held);
        for _ in 0..1502 {
            cache
                .project(&frame, 1500, Clock::Stable, || panic!("latest must carry"))
                .unwrap();
        }
        assert_eq!(cache.roots.lock().unwrap().by_key.len(), 1);
    }

    #[test]
    fn unchanged_subject_rows_expire_at_the_exact_hour_in_both_directions() {
        let cache = DependencyIndex::default();
        let mut frame = frame(8);
        frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![SubjectRow {
                subject: 1,
                overlord: 2,
                start_hour: 2,
                end_hour: 10,
                ..Default::default()
            }]))
            .into();
        let build = |frame: &DecodedFrame| {
            let _ = &*frame.tables[TableId::Subjects.index()];
            Ok(frame.hour < 10)
        };
        let first = cache
            .project(&frame, (), Clock::Relations, || build(&frame))
            .unwrap();
        frame.hour = 9;
        assert!(Arc::ptr_eq(
            &first,
            &cache
                .project(&frame, (), Clock::Relations, || panic!("must carry"))
                .unwrap()
        ));
        frame.hour = 10;
        assert!(
            !*cache
                .project(&frame, (), Clock::Relations, || build(&frame))
                .unwrap()
        );
        frame.hour = 8;
        assert!(
            *cache
                .project(&frame, (), Clock::Relations, || build(&frame))
                .unwrap()
        );
    }
    #[test]
    fn field_stamps_skip_unrelated_rows_even_when_a_consumer_skips_hours() {
        use replay_data::rows::{CountryRow, TableRows};
        use replay_frame::frame::{FrameDecoder, FrameEncoder};
        fn input(hour: i32, slot: u32, power: i64) -> DecodedFrame {
            let mut frame = frame(hour);
            frame.tables = std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture gap".into(),
                }
                .into()
            });
            frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id: 10,
                    canonical_slot: slot,
                    command_power_raw: power,
                    ..Default::default()
                }]))
                .into();
            frame
        }
        let cache = DependencyIndex::default();
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(&[TableId::Countries]).with_change_facts();
        let first = encoder.encode(input(1, 20, 1), true).unwrap();
        let first = decoder.apply(&first).unwrap();
        let fields = |table: TableId| {
            if table == TableId::Countries {
                1
            } else {
                table.full_mask()
            }
        };
        let build = |frame: &DecodedFrame| {
            Ok(viewer_foundation::tables::core::<tables::Countries>(frame)?[0].canonical_slot)
        };
        let value = cache
            .project_fields(&first, (), Clock::Stable, fields, || build(&first))
            .unwrap();
        for hour in 2..5 {
            let update = encoder
                .encode(input(hour, 20, i64::from(hour)), false)
                .unwrap();
            decoder.advance(&update).unwrap();
        }
        let current = decoder.current().unwrap();
        let (same, reads) = view_reads::record(|| {
            cache.project_fields(current, (), Clock::Stable, fields, || {
                panic!("unrelated fields must carry")
            })
        });
        assert!(Arc::ptr_eq(&value, &same.unwrap()));
        assert!(
            reads
                .iter()
                .any(|read| read.is(&current.tables[TableId::Countries.index()]))
        );
        let update = encoder.encode(input(5, 21, 5), false).unwrap();
        let current = decoder.advance(&update).unwrap();
        let next = cache
            .project_fields(current, (), Clock::Stable, fields, || build(current))
            .unwrap();
        assert_eq!(*next, 21);
        let mut replaced = current.clone();
        replaced.tables[TableId::Campaign.index()] = TableView::Unavailable {
            reason: "replacement source".into(),
        }
        .into();
        let replaced = cache
            .project_equal_fields(&replaced, (), Clock::Stable, fields, || build(&replaced))
            .unwrap();
        assert!(
            !Arc::ptr_eq(&next, &replaced),
            "equal paint cannot retain an old source binding"
        );
    }
}
