use super::{History, Pass, core, native_load, pass_tables};
use replay::campaign::{HourCursor, Replay, SeekResult};
use replay_data::{BranchId, Hour, rows::{TableId, tables}};
use std::{io, sync::Arc};
use viewer_foundation::perspective::Country;

pub struct Cursor {
    hours: HourCursor,
    tables: Vec<TableId>,
    held: Option<Held>,
}

struct Held {
    epoch: u64,
    branch: BranchId,
    country: Country,
    first: Hour,
    last: Option<Hour>,
    pass: Pass,
}

impl Cursor {
    pub fn new(replay: Arc<Replay>, tables: &[TableId]) -> Self {
        let mut tables = tables.to_vec();
        tables.extend(pass_tables());
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        Self { hours: HourCursor::new(replay, &tables, false), tables, held: None }
    }

    pub fn declare(&mut self, tables: &[TableId]) {
        if tables.iter().all(|table| self.tables.contains(table)) {
            return;
        }
        self.tables.extend_from_slice(tables);
        self.tables.sort_unstable_by_key(|table| table.index());
        self.tables.dedup();
        self.hours = HourCursor::new(Arc::clone(self.hours.replay()), &self.tables, false);
    }

    pub fn read(&mut self, branch: BranchId, hour: Hour, epoch: u64, country: &Country, first: Option<Hour>) -> io::Result<(SeekResult, Option<Arc<History>>)> {
        let Some(first) = first else {
            return Ok((self.hours.seek(branch, hour)?, None));
        };
        if self.held.as_ref().is_none_or(|held| held.epoch != epoch || held.branch != branch || held.country != *country || held.first != first || held.last.is_some_and(|last| hour < last)) {
            self.held = Some(Held { epoch, branch, country: country.clone(), first, last: None, pass: Pass::new(Arc::default(), country.clone()) });
        }
        let held = self.held.as_mut().expect("made above");
        let history = Arc::clone(&held.pass.shared);
        if held.last == Some(hour) {
            return Ok((self.hours.seek(branch, hour)?, Some(history)));
        }
        let replay = Arc::clone(self.hours.replay());
        let pieces = replay.pieces(branch);
        for between in held.last.map_or(first, |last| last + 1)..=hour {
            let read = self.hours.seek(branch, between);
            let frame = match &read {
                Ok(SeekResult::Frame(frame)) if core::<tables::Campaign>(frame.as_ref()).is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id) => Some(frame.as_ref()),
                _ => None,
            };
            let piece = pieces.iter().enumerate().find(|(_, piece)| (piece.first_hour..=piece.last_hour).contains(&between));
            let load = piece.and_then(|(_, piece)| replay.segments_recorded().iter().find(|start| start.id == piece.segment).and_then(native_load));
            let restart = piece.is_some_and(|(at, piece)| at > 0 && piece.first_hour == between && load != Some(between));
            held.pass.fold_hour(branch, between, frame, load, restart);
            held.last = Some(between);
            let read = read?;
            if between == hour {
                return Ok((read, Some(history)));
            }
        }
        Err(io::Error::new(io::ErrorKind::InvalidInput, "The requested hour is before the battle's first recorded hour."))
    }
}

#[cfg(test)]
mod tests;
