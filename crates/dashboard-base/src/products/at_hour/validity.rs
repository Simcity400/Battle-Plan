use dashboard_text::{metadata::check_cancel, triggers::inputs::{Resolution, TableRead}};
use replay::campaign::Replay;
use replay_data::rows::TableId;
use replay_lanes::lanes::{Cell, Keys};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[derive(Default, PartialEq)]
struct Selection {
    fields: u64,
    rows: Option<Vec<(u128, u128)>>,
    ends: bool,
}

impl Selection {
    fn merge(&mut self, read: &TableRead) {
        self.fields |= read.fields;
        self.ends |= read.ends;
        match (read.rows, read.owner, self.rows.as_mut()) {
            (Some(rows), None, Some(ranges)) => ranges.push(rows),
            _ => self.rows = None,
        }
    }

    fn dates(&self, replay: &Replay, run: u32, table: TableId, first: i32, last: i32) -> io::Result<Vec<i32>> {
        let columns: Vec<_> = (0..64).filter(|bit| self.fields & (1_u64 << bit) != 0).collect();
        let keys = self.rows.as_deref().map_or(Keys::All, Keys::Ranges);
        let history = replay_lanes::lanes::history(replay.recording(), run, table, &columns, keys, first, last)?;
        let mut dates: Vec<_> = history.statuses.iter().map(|status| status.first).collect();
        let end = self.ends.then(|| history.columns.iter().position(|&c| table.field_name(c) == Some("end_hour"))).flatten();
        for (_, steps) in history.keys {
            for (index, step) in steps.iter().enumerate() {
                dates.push(step.hour);
                if let Some(column) = end
                    && let Some(Cell::Number(end)) = step.cells.as_deref().and_then(|cells| cells.get(column))
                    && let Ok(end) = i32::try_from(*end)
                    && step.hour < end && end <= steps.get(index + 1).map_or(last, |step| step.hour)
                {
                    dates.push(end);
                }
            }
        }
        dates.sort_unstable();
        dates.dedup();
        Ok(dates)
    }
}

#[derive(Default)]
pub struct ReadDates {
    span: Option<(u32, i32, i32)>,
    tables: BTreeMap<TableId, (Selection, Vec<i32>)>,
}

impl ReadDates {
    pub fn through(&mut self, replay: &Replay, run: u32, reads: &[TableRead], first: i32, hour: i32, last: i32, cancelled: &AtomicBool) -> io::Result<i32> {
        if self.span != Some((run, first, last)) {
            self.tables.clear();
            self.span = Some((run, first, last));
        }
        let mut tables = BTreeMap::<TableId, Selection>::new();
        let mut merge = |read: &TableRead| tables.entry(read.table).or_insert_with(|| Selection { rows: Some(Vec::new()), ..Default::default() }).merge(read);
        for read in reads {
            merge(read);
            if let Some(owner) = read.owner {
                let table = match owner.by { Resolution::Links => TableId::CountryLinks, Resolution::Countries => TableId::Countries };
                merge(&TableRead::fields(table, &["canonical_slot"]));
            }
        }
        let mut through = last;
        for (table, mut selection) in tables {
            check_cancel(cancelled)?;
            if let Some(ranges) = &mut selection.rows {
                ranges.sort_unstable();
                ranges.dedup_by(|next, previous| {
                    if next.0 > previous.1.saturating_add(1) { return false; }
                    previous.1 = previous.1.max(next.1);
                    true
                });
            }
            if self.tables.get(&table).is_none_or(|(prior, _)| *prior != selection) {
                let Ok(dates) = selection.dates(replay, run, table, first, last) else { return Ok(hour) };
                self.tables.insert(table, (selection, dates));
            }
            let dates = &self.tables[&table].1;
            if let Some(&next) = dates.get(dates.partition_point(|&at| at <= hour)) {
                through = through.min(next - 1);
                if through == hour { break; }
            }
        }
        Ok(through)
    }
}

pub fn reads_through(replay: &Replay, run: u32, reads: &[TableRead], hour: i32, through: i32, cancelled: &AtomicBool) -> io::Result<i32> {
    ReadDates::default().through(replay, run, reads, hour, hour, through, cancelled)
}
