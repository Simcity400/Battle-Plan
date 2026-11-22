use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use dashboard_text::triggers::inputs::{Inputs as Reads, TableRead};
use map_display_data::value_index::ValueIndex;
use replay_data::rows::TableId;
use replay_frame::frame::{RecordedSource, ViewIdentity};

type Target = Option<TargetId>;
type Route = (usize, u16, u128, Target);
type Back = (Target, usize, u16, u128);
type Range = (usize, u64, bool, u128, u128);
#[derive(Default)]
struct Ranges(ValueIndex<Range, ()>);
impl Ranges {
    fn add(&mut self, read: TableRead) -> std::io::Result<()> {
        let (first, last) = read.rows.unwrap_or((0, u128::MAX));
        self.0 = self.0.inserted(
            (read.table.index(), read.fields, read.ends, first, last),
            (),
        )?;
        if read.owner.is_some() {
            self.add(TableRead::fields(
                TableId::CountryLinks,
                &["canonical_slot"],
            ))?;
        }
        Ok(())
    }
    fn apply(&self, routes: &mut Routes, target: Target) -> std::io::Result<()> {
        let mut pending: Option<Range> = None;
        for (&(table, fields, ends, first, last), _) in self.0.iter() {
            if let Some((old_table, old_fields, old_ends, _, old_last)) = &mut pending
                && (*old_table, *old_fields, *old_ends) == (table, fields, ends)
                && first <= old_last.saturating_add(1)
            {
                *old_last = (*old_last).max(last);
                continue;
            }
            if let Some(read) = pending.replace((table, fields, ends, first, last)) {
                Self::install(routes, target, read)?;
            }
        }
        if let Some(read) = pending {
            Self::install(routes, target, read)?;
        }
        Ok(())
    }
    fn install(routes: &mut Routes, target: Target, range: Range) -> std::io::Result<()> {
        let (table, fields, ends, first, last) = range;
        routes.add(
            target,
            TableRead {
                table: TableId::ALL[table],
                fields,
                rows: Some((first, last)),
                ends,
                owner: None,
            },
        )
    }
}
#[derive(Default)]
pub(super) struct Routes {
    rows: ValueIndex<Route, u64>,
    depths: ValueIndex<(usize, u16), usize>,
    back: ValueIndex<Back, ()>,
    tables: ValueIndex<(usize, Target), ()>,
    ends: ValueIndex<Route, ()>,
    volatile: ValueIndex<Target, ()>,
    fresh: ValueIndex<(usize, Target), ()>,
    fresh_back: ValueIndex<(Target, usize), ()>,
    fresh_values: ValueIndex<usize, bool>,
    views: Vec<ViewIdentity>,
    hour: Option<i32>,
    subjects: Option<RowFold<(i32, i32)>>,
    wars: Option<RowFold<(i32, i32)>>,
    boundaries: ValueIndex<(i32, usize, u128), ()>,
}
impl Routes {
    pub(super) fn remove(&mut self, target: Target) -> std::io::Result<()> {
        while let Some((&(_, table, depth, prefix), _)) = self
            .back
            .iter_from(&(target, 0, 0, 0))
            .next()
            .filter(|(key, _)| key.0 == target)
        {
            self.rows = self.rows.removed(&(table, depth, prefix, target))?;
            let count = self
                .depths
                .get(&(table, depth))
                .copied()
                .expect("registered route depth");
            self.depths = if count == 1 {
                self.depths.removed(&(table, depth))?
            } else {
                self.depths.inserted((table, depth), count - 1)?
            };
            self.ends = self.ends.removed(&(table, depth, prefix, target))?;
            self.back = self.back.removed(&(target, table, depth, prefix))?;
            self.tables = self.tables.removed(&(table, target))?;
        }
        while let Some((&(_, table), _)) = self
            .fresh_back
            .iter_from(&(target, 0))
            .next()
            .filter(|(key, _)| key.0 == target)
        {
            self.fresh_back = self.fresh_back.removed(&(target, table))?;
            self.fresh = self.fresh.removed(&(table, target))?;
        }
        self.volatile = self.volatile.removed(&target)?;
        Ok(())
    }
    pub(super) fn replace(
        &mut self,
        target: Target,
        reads: &Reads,
        frame: &replay_frame::frame::DecodedFrame,
    ) -> std::io::Result<()> {
        self.remove(target)?;
        let mut ranges = Ranges::default();
        for key in reads.keys() {
            use dashboard_text::triggers::{inputs::Key, world::Query};
            if let Key::World(Query::TableFresh(table)) = key {
                self.fresh = self.fresh.inserted((table.index(), target), ())?;
                self.fresh_back = self.fresh_back.inserted((target, table.index()), ())?;
                self.fresh_values = self.fresh_values.inserted(
                    table.index(),
                    frame.sampled_at(table.index()) == Some(frame.hour),
                )?;
                continue;
            }
            if let Key::World(Query::TableStatus(table)) = key {
                ranges.add(TableRead::keys(*table))?;
            }
            match key.dependence() {
                None => self.volatile = self.volatile.inserted(target, ())?,
                Some(dependencies) => {
                    for read in dependencies {
                        ranges.add(read)?;
                    }
                }
            }
        }
        ranges.apply(self, target)
    }
    fn add(&mut self, target: Target, read: TableRead) -> std::io::Result<()> {
        self.add_indexed(target, read).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!(
                    "Scripted map dependency {read:?} for {target:?} ({} indexed ranges): {error}",
                    self.rows.len()
                ),
            )
        })
    }
    fn add_indexed(&mut self, target: Target, read: TableRead) -> std::io::Result<()> {
        let table = read.table.index();
        self.tables = self.tables.inserted((table, target), ())?;
        prefixes(read.rows.unwrap_or((0, u128::MAX)), |depth, prefix| {
            let key = (table, depth, prefix, target);
            if !self.rows.contains_key(&key) {
                self.depths = self.depths.inserted(
                    (table, depth),
                    self.depths.get(&(table, depth)).copied().unwrap_or(0) + 1,
                )?;
            }
            self.rows = self
                .rows
                .inserted(key, self.rows.get(&key).copied().unwrap_or(0) | read.fields)?;
            self.back = self.back.inserted((target, table, depth, prefix), ())?;
            if read.ends {
                self.ends = self.ends.inserted(key, ())?;
            }
            Ok(())
        })?;
        Ok(())
    }
    fn whole(&self, table: usize, dirty: &mut ValueIndex<Target, ()>) -> std::io::Result<()> {
        for (&(_, target), _) in self
            .tables
            .iter_from(&(table, None))
            .take_while(|(key, _)| key.0 == table)
        {
            *dirty = dirty.inserted(target, ())?;
        }
        Ok(())
    }
    fn row(
        &self,
        table: usize,
        key: u128,
        mask: Option<u64>,
        boundary: bool,
        dirty: &mut ValueIndex<Target, ()>,
    ) -> std::io::Result<()> {
        for (&(_, depth), _) in self
            .depths
            .iter_from(&(table, 0))
            .take_while(|(key, _)| key.0 == table)
        {
            let prefix = if depth == 0 {
                0
            } else {
                key & (u128::MAX << (128 - depth))
            };
            if boundary {
                for (&(_, _, _, target), _) in self
                    .ends
                    .iter_from(&(table, depth, prefix, None))
                    .take_while(|(route, _)| (route.0, route.1, route.2) == (table, depth, prefix))
                {
                    *dirty = dirty.inserted(target, ())?;
                }
            } else {
                for (&(_, _, _, target), &fields) in self
                    .rows
                    .iter_from(&(table, depth, prefix, None))
                    .take_while(|(route, _)| (route.0, route.1, route.2) == (table, depth, prefix))
                {
                    if mask.is_none_or(|mask| fields & mask != 0) {
                        *dirty = dirty.inserted(target, ())?;
                    }
                }
            }
        }
        Ok(())
    }
    pub(super) fn advance(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
    ) -> std::io::Result<ValueIndex<Target, ()>> {
        let mut dirty = self.volatile.clone();
        let mut fresh_values = self.fresh_values.clone();
        for (&table, &old) in self.fresh_values.iter() {
            let value = frame.sampled_at(table) == Some(frame.hour);
            if value != old {
                for (&(_, target), _) in self
                    .fresh
                    .iter_from(&(table, None))
                    .take_while(|(key, _)| key.0 == table)
                {
                    dirty = dirty.inserted(target, ())?;
                }
                fresh_values = fresh_values.inserted(table, value)?;
            }
        }
        self.fresh_values = fresh_values;
        if self.views.is_empty() {
            self.views = frame.tables.iter().map(|view| view.identity()).collect();
        } else {
            for (table, view) in frame.tables.iter().enumerate() {
                let old = &self.views[table];
                if !old.is(view) {
                    if let replay_frame::frame::TableChanges::Rows(facts) =
                        replay_frame::frame::HourInput::new(frame).changes().since(
                            TableId::ALL[table],
                            Some(old),
                            TableId::ALL[table].full_mask(),
                        )
                    {
                        for change in facts.changed() {
                            self.row(
                                table,
                                change.key,
                                (!change.inserted).then_some(change.fields),
                                false,
                                &mut dirty,
                            )?;
                        }
                        for &key in facts.deleted() {
                            self.row(table, key, None, false, &mut dirty)?;
                        }
                    } else {
                        self.whole(table, &mut dirty)?;
                    }
                }
                self.views[table] = view.identity();
            }
        }
        if !self.ends.is_empty() {
            self.boundaries(frame, TableId::Subjects, &mut dirty)?;
            self.boundaries(frame, TableId::WarRelations, &mut dirty)?;
            if let Some(hour) = self.hour {
                let (from, to) = if hour <= frame.hour {
                    (hour, frame.hour)
                } else {
                    (frame.hour, hour)
                };
                if from != to {
                    for (&(_, table, key), _) in self
                        .boundaries
                        .iter_from(&(from, usize::MAX, u128::MAX))
                        .skip_while(|(key, _)| key.0 <= from)
                        .take_while(|(key, _)| key.0 <= to)
                    {
                        self.row(table, key, None, true, &mut dirty)?;
                    }
                }
            }
        }
        self.hour = Some(frame.hour);
        Ok(dirty)
    }
    fn boundaries(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        table: TableId,
        _dirty: &mut ValueIndex<Target, ()>,
    ) -> std::io::Result<()> {
        if self
            .ends
            .iter_from(&(table.index(), 0, 0, None))
            .next()
            .is_none_or(|(key, _)| key.0 != table.index())
        {
            return Ok(());
        }
        let old = if table == TableId::Subjects {
            self.subjects.as_ref()
        } else {
            self.wars.as_ref()
        };
        let rows = if table == TableId::Subjects {
            frame.complete::<tables::Subjects>().ok().map(|rows| {
                RowFold::read(
                    frame,
                    table,
                    rows,
                    fields(table, &["start_hour", "end_hour"]),
                    old,
                    |row| Ok((row.start_hour, row.end_hour)),
                )
            })
        } else {
            frame.complete::<tables::WarRelations>().ok().map(|rows| {
                RowFold::read(
                    frame,
                    table,
                    rows,
                    fields(table, &["start_hour", "end_hour"]),
                    old,
                    |row| Ok((row.start_hour, row.end_hour)),
                )
            })
        };
        let Some(rows) = rows else {
            return Ok(());
        };
        let rows = rows?;
        for &key in rows.edits.iter() {
            if let Some(&(start, end)) = old.and_then(|old| old.values.get(&key))
                && end > start
            {
                self.boundaries = self.boundaries.removed(&(end, table.index(), key))?;
            }
            if let Some(&(start, end)) = rows.values.get(&key)
                && end > start
            {
                self.boundaries = self.boundaries.inserted((end, table.index(), key), ())?;
            }
        }
        if table == TableId::Subjects {
            self.subjects = Some(rows);
        } else {
            self.wars = Some(rows);
        }
        Ok(())
    }
}

fn prefixes(
    (mut first, last): (u128, u128),
    mut visit: impl FnMut(u16, u128) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if first > last {
        return Ok(());
    }
    loop {
        let aligned = first.trailing_zeros();
        let remaining = last - first;
        let fits = if remaining == u128::MAX {
            128
        } else {
            127 - (remaining + 1).leading_zeros()
        };
        let bits = aligned.min(fits);
        visit((128 - bits) as u16, first)?;
        if bits == 128 {
            break;
        }
        let size = 1u128 << bits;
        if remaining < size {
            break;
        }
        first += size;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coalesced_reads_keep_gaps_fields_and_clock_boundaries() {
        let mut ranges = Ranges::default();
        let mut separate = Routes::default();
        let mut compact = Routes::default();
        let target = Some(TargetId::Country(1501));
        let links = TableRead::fields(TableId::CountryLinks, &["canonical_slot"]);
        for country in (1..=1500).rev() {
            let read = links.row(country);
            ranges.add(read).unwrap();
            separate.add(target, read).unwrap();
        }
        ranges.apply(&mut compact, target).unwrap();
        assert!(compact.rows.len() < 32);
        assert_eq!(separate.rows.len(), 1500);
        let relations = TableRead::fields(TableId::Subjects, &["start_hour", "end_hour"]);
        for read in [
            relations.rows(3, 8).ending(),
            relations.rows(5, 12).ending(),
            relations.rows(14, 18).ending(),
            relations.rows(19, 22),
            TableRead::keys(TableId::Subjects).rows(0, 2),
            relations.rows(u128::MAX - 2, u128::MAX).ending(),
            relations.row(u128::MAX),
        ] {
            ranges.add(read).unwrap();
            separate.add(target, read).unwrap();
        }
        compact.remove(target).unwrap();
        ranges.apply(&mut compact, target).unwrap();
        for table in [TableId::CountryLinks, TableId::Subjects] {
            for key in (0..=1501).chain([u128::MAX - 3, u128::MAX - 2, u128::MAX]) {
                for mask in [None, Some(0), Some(1), Some(2), Some(u64::MAX)] {
                    for boundary in [false, true] {
                        let mut expected = ValueIndex::default();
                        let mut actual = ValueIndex::default();
                        separate
                            .row(table.index(), key, mask, boundary, &mut expected)
                            .unwrap();
                        compact
                            .row(table.index(), key, mask, boundary, &mut actual)
                            .unwrap();
                        assert_eq!(
                            actual.contains_key(&target),
                            expected.contains_key(&target),
                            "{table:?}, {key}, {mask:?}, boundary={boundary}"
                        );
                    }
                }
            }
        }
        compact.remove(target).unwrap();
        assert!(compact.rows.is_empty());
        assert!(compact.back.is_empty());
        assert!(compact.ends.is_empty());
        assert!(compact.depths.is_empty());
        assert!(compact.tables.is_empty());
    }
    #[test]
    fn empty_range_still_watches_table_availability() {
        let target = Some(TargetId::Country(1501));
        let mut ranges = Ranges::default();
        ranges
            .add(TableRead::keys(TableId::Subjects).rows(5, 3))
            .unwrap();
        let mut routes = Routes::default();
        ranges.apply(&mut routes, target).unwrap();
        assert!(routes.rows.is_empty());
        let mut dirty = ValueIndex::default();
        routes.whole(TableId::Subjects.index(), &mut dirty).unwrap();
        assert!(dirty.contains_key(&target));
    }
    #[test]
    fn owned_ranges_watch_country_link_changes() {
        for read in [
            TableRead::keys(TableId::ScriptVariables).owned_by(1501, 32),
            TableRead::keys(TableId::ScriptVariables).of_country(1501, 32),
        ] {
            let target = Some(TargetId::Country(1501));
            let mut ranges = Ranges::default();
            ranges.add(read).unwrap();
            let mut routes = Routes::default();
            ranges.apply(&mut routes, target).unwrap();
            let mut dirty = ValueIndex::default();
            routes
                .row(
                    TableId::CountryLinks.index(),
                    17,
                    Some(fields(TableId::CountryLinks, &["canonical_slot"])),
                    false,
                    &mut dirty,
                )
                .unwrap();
            assert!(dirty.contains_key(&target));
        }
    }
    #[test]
    fn prefix_routes_preserve_point_ranges_field_masks_and_unbounded_membership() {
        let mut routes = Routes::default();
        let target = Some(TargetId::Country(1501));
        routes
            .add(
                target,
                TableRead::fields(TableId::States, &["population"]).rows(17, 105),
            )
            .unwrap();
        let mut dirty = ValueIndex::default();
        routes
            .row(TableId::States.index(), 16, None, false, &mut dirty)
            .unwrap();
        assert!(dirty.is_empty());
        routes
            .row(
                TableId::States.index(),
                105,
                Some(fields(TableId::States, &["controller"])),
                false,
                &mut dirty,
            )
            .unwrap();
        assert!(dirty.is_empty());
        routes
            .row(
                TableId::States.index(),
                17,
                Some(fields(TableId::States, &["population"])),
                false,
                &mut dirty,
            )
            .unwrap();
        assert!(dirty.contains_key(&target));
        routes.remove(target).unwrap();
        routes.add(None, TableRead::keys(TableId::States)).unwrap();
        let mut dirty = ValueIndex::default();
        routes
            .row(TableId::States.index(), u128::MAX, None, false, &mut dirty)
            .unwrap();
        assert!(dirty.contains_key(&None));
        let mut segments = Vec::new();
        prefixes((0, u128::MAX), |depth, first| {
            segments.push((depth, first));
            Ok(())
        })
        .unwrap();
        assert_eq!(segments, [(0, 0)]);
    }
    #[test]
    fn relation_clock_crossings_route_only_the_watched_native_row() {
        use replay_data::rows::{RowCodec, SubjectRow, TableRows};
        use replay_frame::frame::TableView;
        let mut frame = crate::map_mode::tests::frame();
        let first = SubjectRow {
            subject: 100,
            overlord: 200,
            start_hour: 0,
            end_hour: 2,
            ..Default::default()
        };
        let second = SubjectRow {
            subject: 300,
            overlord: 200,
            start_hour: 0,
            end_hour: 4,
            ..Default::default()
        };
        let first_key = first.key();
        let second_key = second.key();
        let mut rows = vec![first, second];
        rows.sort_by_key(RowCodec::key);
        frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(rows)).into();
        let mut routes = Routes::default();
        let a = Some(TargetId::Country(1501));
        let b = Some(TargetId::Country(1502));
        routes
            .add(
                a,
                TableRead::keys(TableId::Subjects).row(first_key).ending(),
            )
            .unwrap();
        routes
            .add(
                b,
                TableRead::keys(TableId::Subjects).row(second_key).ending(),
            )
            .unwrap();
        for hour in 1..=4 {
            frame.hour = hour;
            let dirty = routes.advance(&frame).unwrap();
            assert_eq!(dirty.contains_key(&a), hour == 2);
            assert_eq!(dirty.contains_key(&b), hour == 4);
        }
    }
}
