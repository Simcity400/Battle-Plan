use crate::{Options, open, table_named};
use replay_recording::columnar_frames::{Frames, Recording};
use replay_data::rows::TableId;
use serde_json::{Value, json};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    pub read: u64,
    pub flagged: u64,
    pub changed: u64,
    pub first_read: Option<i32>,
    pub last_read: Option<i32>,
    pub rows: bool,
    pub lost: u64,
    pub reason: Option<String>,
}

impl Listed {
    pub fn never_read(&self) -> bool {
        (self.rows || self.changed > 0) && self.read == 0
    }

    fn json(&self, table: TableId) -> Value {
        json!({
            "table": format!("{table:?}"),
            "read_hours": self.read,
            "read_flags": self.flagged,
            "changed_hours": self.changed,
            "first_read": self.first_read,
            "last_read": self.last_read,
            "rows": self.rows,
            "never_read": self.never_read(),
            "lost_hours": self.lost,
            "lost": self.reason,
        })
    }
}

pub fn list(recording: &Recording, tables: &[TableId]) -> io::Result<Vec<Listed>> {
    let mut listed = vec![Listed::default(); tables.len()];
    let mut frames = Frames::new(recording, tables)?;
    for run in 0..recording.run_count() {
        let ordinals = recording.run(run)?.ordinals;
        if ordinals == 0 {
            continue;
        }
        frames.seek(run, 0)?;
        for ordinal in 0..ordinals {
            if ordinal > 0 && !frames.step()? {
                break;
            }
            let hour = recording.run(run)?.first_hour + ordinal as i32;
            for (held, &table) in listed.iter_mut().zip(tables) {
                let Some(reader) = frames.reader(table) else {
                    held.lost += 1;
                    if held.reason.is_none() {
                        held.reason = Some(format!("run {run}: the table's bytes cannot be read"));
                    }
                    continue;
                };
                if reader.sampled_at() == Some(hour) {
                    held.read += 1;
                    held.first_read.get_or_insert(hour);
                    held.last_read = Some(hour);
                }
                held.flagged += u64::from(reader.was_read());
                let changes = reader.changes();
                let live = ordinal == 0 && reader.cells().live().count() > 0;
                let moved = !changes.inserted.is_empty() || !changes.deleted.is_empty() || changes.written.iter().any(|slots| !slots.is_empty());
                held.changed += u64::from(live || moved);
                held.rows |= live || !changes.inserted.is_empty();
            }
        }
    }
    Ok(listed)
}

pub fn read_hours(path: &str, options: &Options) -> io::Result<Value> {
    let tables = match options.get("tables") {
        Some(names) => names.split(',').map(table_named).collect::<io::Result<Vec<_>>>()?,
        None => TableId::ALL.to_vec(),
    };
    let recording = open(path)?;
    let listed = list(&recording, &tables)?;
    for (held, &table) in listed.iter().zip(&tables) {
        println!("{}", serde_json::to_string(&held.json(table))?);
    }
    let never: Vec<String> = listed.iter().zip(&tables).filter(|(held, _)| held.never_read()).map(|(_, table)| format!("{table:?}")).collect();
    let lost: Vec<String> = listed.iter().zip(&tables).filter(|(held, _)| held.lost > 0).map(|(_, table)| format!("{table:?}")).collect();
    Ok(json!({
        "command": "read-hours",
        "hours": recording.hours(),
        "runs": recording.run_count(),
        "tables": tables.len(),
        "never_read": never.len(),
        "never_read_tables": never,
        "lost_tables": lost,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_journal::fixtures;
    use replay_data::rows::{CountryFuelRow, CountryRow, TABLE_COUNT, TableRows};
    use replay_frame::frame::{FrameInput, Metrics, TableInput, TableView};

    fn recording(fuel_ns: [u64; 3]) -> Recording {
        let file = fixtures::recorded(recorder_derive::writers(), b"read-hours", 3, |ordinal| {
            let mut tables: [TableInput; TABLE_COUNT] = std::array::from_fn(|_| TableInput::Sampled(TableView::Unavailable { reason: "table was not acquired".into() }));
            tables[TableId::Countries.index()] = TableInput::Sampled(TableView::Complete(TableRows::Countries(vec![CountryRow { id: 7, ..Default::default() }])));
            tables[TableId::CountryFuel.index()] = if ordinal == 1 {
                TableInput::Sampled(TableView::Unavailable { reason: replay_data::cadence::SKIPPED_REASON.into() })
            } else {
                TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(vec![CountryFuelRow { country: 7, current_raw: i64::from(ordinal) + 1, ..Default::default() }])))
            };
            let mut metrics = Metrics::default();
            metrics.table_ns[TableId::Countries.index()] = 10;
            metrics.table_ns[TableId::CountryFuel.index()] = fuel_ns[ordinal as usize];
            FrameInput { branch: 1, hour: 100 + ordinal as i32, tables, metrics }
        });
        Recording::open(file).unwrap()
    }

    #[test]
    fn a_table_with_rows_and_no_read_hour_is_listed_never_read() {
        let tables = [TableId::Countries, TableId::CountryFuel];
        let untimed = list(&recording([0, 0, 0]), &tables).unwrap();
        assert_eq!((untimed[0].read, untimed[0].first_read, untimed[0].last_read), (3, Some(100), Some(102)));
        assert!(!untimed[0].never_read());
        assert!(untimed[1].rows);
        assert_eq!(untimed[1].read, 0);
        assert!(untimed[1].changed >= 2, "{:?}", untimed[1]);
        assert!(untimed[1].never_read());

        let read = list(&recording([5, 0, 5]), &tables).unwrap();
        assert_eq!((read[1].read, read[1].flagged), (2, 2));
        assert_eq!((read[1].first_read, read[1].last_read), (Some(100), Some(102)));
        assert!(!read[1].never_read());
    }
}
