use super::*;
use crate::frame::FrameAdvance;
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct SourceFacts {
    pub complete: bool,
    pub members: Vec<(u128, Vec<u128>)>,
    previous_members: Option<SharedView>,
}

impl SourceFacts {
    pub(crate) fn capture(
        table: TableId,
        previous: Option<&SharedView>,
        update: &TableUpdate,
    ) -> io::Result<Self> {
        let complete = previous.is_some_and(|view| matches!(**view, TableView::Complete(_)));
        let mut out = Self {
            complete,
            members: Vec::new(),
            previous_members: None,
        };
        if table == TableId::ProvinceUnitOrder
            && let TableUpdate::Delta {
                changed, deleted, ..
            } = update
        {
            let mut keys = allocation::collect_vec(
                changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied()),
            )?;
            keys.sort_unstable();
            keys.dedup();
            let mut old_rows = Vec::new();
            for key in keys {
                let mut values = Vec::new();
                members(previous.map(|view| &**view), &[key], &mut values)?;
                allocation::reserve(&mut out.members, 1)?;
                out.members.push((key, values));
                if let Some(TableView::Complete(TableRows::ProvinceUnitOrder(rows))) =
                    previous.map(|view| &**view)
                    && let Some(row) = find(Some(rows), key)
                {
                    allocation::reserve(&mut old_rows, 1)?;
                    old_rows.push(row.clone());
                }
            }
            out.previous_members =
                Some(TableView::Complete(TableRows::ProvinceUnitOrder(old_rows)).into());
        }
        Ok(out)
    }
}

pub(crate) struct Sources<'a> {
    pub frame: &'a FrameAdvance,
    pub updates: &'a [Option<Arc<TableUpdate>>],
    pub facts: &'a [SourceFacts],
    pub errors: &'a [Option<io::Error>],
}

struct Loaded {
    table: TableId,
    at: Option<usize>,
}

struct CurrentSupply<'a, 'b> {
    sources: &'a Sources<'b>,
    loaded: &'a mut Vec<Loaded>,
    at: usize,
}

struct One<'a>(&'a TableUpdate);
impl Column for One<'_> {
    fn update(&self, _: usize) -> &TableUpdate {
        self.0
    }
}

impl Supply for CurrentSupply<'_, '_> {
    fn loaded(&self) -> usize {
        self.loaded.len()
    }
    fn table(&self, index: usize) -> TableId {
        self.loaded[index].table
    }
    fn position(&self, table: TableId) -> Option<usize> {
        self.loaded.iter().position(|entry| entry.table == table)
    }
    fn load(&mut self, source: TableId, _: usize, check: &mut Check<'_>) -> io::Result<usize> {
        check.run()?;
        if let Some(error) = &self.sources.errors[source.index()] {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        allocation::reserve(self.loaded, 1)?;
        self.loaded.push(Loaded {
            table: source,
            at: None,
        });
        Ok(self.loaded.len() - 1)
    }
    fn changed_keys(
        &mut self,
        index: usize,
        _: usize,
        relevant: &dyn Fn(u128, u64) -> bool,
        keys: &mut Vec<(u128, u64)>,
        check: &mut Check<'_>,
    ) -> io::Result<bool> {
        #[cfg(test)]
        let _step =
            crate::container::check::Scope::enter(crate::container::check::Step::Keys(self.at));
        check.run()?;
        let entry = &self.loaded[index];
        if entry.at != self.at.checked_sub(1) && entry.at != Some(self.at) {
            return Ok(false);
        }
        let update = self.sources.updates[entry.table.index()]
            .as_ref()
            .ok_or_else(|| invalid("alias source did not reach this hour"))?;
        source_keys(&One(update), 0, 0, relevant, keys, check)
    }
    fn complete(&self, index: usize) -> bool {
        let entry = &self.loaded[index];
        if entry.at == Some(self.at) {
            self.sources
                .frame
                .reached(entry.table)
                .is_some_and(|view| matches!(**view, TableView::Complete(_)))
        } else {
            self.sources.facts[entry.table.index()].complete
        }
    }
    fn members(&self, index: usize, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()> {
        let entry = &self.loaded[index];
        if entry.at == Some(self.at) {
            members(
                self.sources.frame.reached(entry.table).map(|view| &**view),
                keys,
                out,
            )
        } else {
            let old = &self.sources.facts[entry.table.index()].members;
            for key in keys {
                if let Ok(at) = old.binary_search_by_key(key, |entry| entry.0) {
                    allocation::reserve(out, old[at].1.len())?;
                    out.extend_from_slice(&old[at].1);
                }
            }
            Ok(())
        }
    }
    fn catch_up(
        &mut self,
        index: usize,
        _: &[Hour],
        _: usize,
        check: &mut Check<'_>,
    ) -> io::Result<()> {
        #[cfg(test)]
        let _step = crate::container::check::Scope::enter(crate::container::check::Step::CatchUp(
            self.loaded[index].table,
            self.at,
        ));
        check.run()?;
        let entry = &mut self.loaded[index];
        if let Some(error) = &self.sources.errors[entry.table.index()] {
            return Err(io::Error::new(error.kind(), error.to_string()));
        }
        if self.sources.frame.reached(entry.table).is_none() {
            return Err(invalid("alias source did not reach this hour"));
        }
        entry.at = Some(self.at);
        Ok(())
    }
    fn view(&self, index: usize) -> SharedView {
        let entry = &self.loaded[index];
        if entry.at != Some(self.at)
            && let Some(view) = &self.sources.facts[entry.table.index()].previous_members
        {
            return view.clone();
        }
        self.sources
            .frame
            .reached(entry.table)
            .expect("source checked at barrier")
            .clone()
    }
}

pub(crate) struct Resolution {
    table: TableId,
    default: u16,
    key_marks: Vec<u8>,
    key_offset: usize,
    later_offset: usize,
    marks: Marks,
    previous_raw: Option<(Hour, SharedView)>,
    loaded: Vec<Loaded>,
    bytes: usize,
    mark_keys: Vec<u128>,
    source_changes: Vec<(u128, u64)>,
    sparse: Sparse,
}

impl Resolution {
    pub(crate) fn new(table: TableId, key_marks: Vec<u8>) -> io::Result<Self> {
        let mut input = Decoder::new(&key_marks);
        let default = u16::try_from(input.varint()?)
            .map_err(|_| invalid("default alias mask exceeds u16"))?;
        if default & !allowed(table) != 0 {
            return Err(invalid("unknown default private alias bits"));
        }
        let key_offset = key_marks.len() - input.remaining();
        Ok(Self {
            table,
            default,
            key_marks,
            key_offset,
            later_offset: 0,
            marks: Vec::new(),
            previous_raw: None,
            loaded: Vec::new(),
            bytes: 0,
            mark_keys: Vec::new(),
            source_changes: Vec::new(),
            sparse: Sparse::default(),
        })
    }

    pub(crate) fn hour(
        &mut self,
        hours: &[Hour],
        at: usize,
        raw: TableUpdate,
        later_marks: Option<&[u8]>,
        previous: Option<(Hour, &SharedView)>,
        sources: &Sources<'_>,
        check: &mut Check<'_>,
    ) -> io::Result<TableUpdate> {
        #[cfg(test)]
        let _step = crate::container::check::Scope::enter(crate::container::check::Step::Alias(
            self.table, at,
        ));
        check.run()?;
        let before = self
            .previous_raw
            .take()
            .filter(|(_, view)| matches!(**view, TableView::Complete(_)));
        let view = advance_table(self.table, &raw, before, hours[at], false)?;
        let rows = match &*view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            _ => 0,
        };
        self.mark_keys.clear();
        if at == 0 {
            let mut input = Decoder::new(&self.key_marks[self.key_offset..]);
            self.marks = take_marks(self.table, &mut input, &[], rows)?;
            input.finish()?;
            self.key_marks.clear();
        } else if let Some(marks) = later_marks {
            let mut input = Decoder::new(&marks[self.later_offset..]);
            self.marks = take_marks_noting(
                self.table,
                &mut input,
                &self.marks,
                rows,
                Some(&mut self.mark_keys),
            )?;
            self.later_offset = marks.len() - input.remaining();
        }
        if self.table == TableId::LandCombatDivisions {
            self.sparse.combat.advance(&view, &raw)?;
        }
        let previous = previous.map(|(hour, view)| (hour, view.clone()));
        let mut supply = CurrentSupply {
            sources,
            loaded: &mut self.loaded,
            at,
        };
        let candidate = match previous
            .as_ref()
            .filter(|_| at != 0 && candidate(self.table, Probe).is_some())
        {
            Some(previous) => match candidate_hour(
                self.table,
                hours,
                at,
                &raw,
                &view,
                &self.marks,
                &self.mark_keys,
                self.default,
                previous,
                &mut supply,
                &mut self.source_changes,
                &mut self.sparse,
                check,
            ) {
                Ok(value) => value,
                Err(error)
                    if check.failed()
                        || allocation::is_allocation_error(&error)
                        || !crate::container::recording_damage(&error) =>
                {
                    return Err(error);
                }
                Err(_) => None,
            },
            None => None,
        };
        let (plain, emitted) = match candidate {
            Some(hour) => {
                self.sparse.keys = hour.keys;
                (hour.plain, hour.emitted)
            }
            None => {
                let mut loaded_sources = Vec::new();
                let (plain, _) = whole_hour(
                    self.table,
                    hours,
                    at,
                    matches!(raw, TableUpdate::Snapshot(_)),
                    &view,
                    &mut self.marks,
                    self.default,
                    previous.as_ref(),
                    &mut supply,
                    &mut loaded_sources,
                    check,
                )?;
                if self.table == TableId::Divisions {
                    self.sparse.provinces.reset(&loaded_sources)?;
                }
                let emitted = resolved_update(self.table, &plain, at == 0, &raw)?;
                (plain, emitted)
            }
        };
        self.bytes = allocation::add(self.bytes, plain.len())?;
        if self.bytes > resolved_limit() {
            return Err(invalid(
                "resolved private aliases exceed raw block size limit",
            ));
        }
        self.previous_raw = Some((hours[at], view));
        Ok(emitted)
    }

    pub(crate) fn finish(&self, later_marks: Option<&[u8]>) -> io::Result<()> {
        if later_marks.is_some_and(|marks| self.later_offset != marks.len()) {
            return Err(invalid("trailing bytes"));
        }
        Ok(())
    }
    pub(crate) fn finish_chunk(&mut self, marks: Option<&[u8]>) -> io::Result<()> {
        self.finish(marks)?;
        self.later_offset = 0;
        Ok(())
    }
    pub(crate) fn sources(&self) -> impl Iterator<Item = TableId> + '_ {
        self.loaded.iter().map(|loaded| loaded.table)
    }
}
