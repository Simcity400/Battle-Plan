use crate::{
    lanes::{self, Keys, StatusRun},
    points::Cell,
};
use replay_columnar::{manifest::Status, state::Kind, store::Value};
use replay_data::{rows::TableId, scope::CaptureMode};
use replay_frame::frame::{SharedView, TableView, Unsayable, rows_of_fields, value_field};
use replay_recording::columnar_frames::Recording;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Mutex, PoisonError},
};

#[cfg(test)]
mod tests;

const KEPT_VIEWS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Listed {
    Bytes(Box<[u8]>),
    Missing,
    Unrecorded,
}

pub struct Dated {
    table: TableId,
    fields: u64,
    columns: Vec<usize>,
    kinds: Vec<Option<Kind>>,
    withheld: Vec<bool>,
    format: u16,
    mode: CaptureMode,
    keys: Vec<u128>,
    steps: Vec<Vec<lanes::Step>>,
    texts: Vec<Box<[u8]>>,
    lists: HashMap<(usize, i32, usize), Listed>,
    statuses: Vec<StatusRun>,
    reasons: HashMap<u32, String>,
    refused: Option<String>,
    changes: Vec<i32>,
    held: Mutex<Vec<(i32, SharedView)>>,
}

fn columns_of(table: TableId, fields: u64) -> Vec<usize> {
    (0..64).filter(|&column| fields & (1_u64 << column) != 0 && column < table.schema().len()).collect()
}

impl Dated {
    pub fn read(recording: &Recording, run: u32, table: TableId, fields: u64, keys: Keys<'_>) -> io::Result<Self> {
        let fields = fields & table.full_mask();
        let held = recording.run(run)?;
        let (from, to) = (held.first_hour, held.first_hour + held.ordinals as i32 - 1);
        let columns = columns_of(table, fields);
        let index = table.index();
        let recorded = recording.columns()[index].min(table.schema().len());
        let schema = replay_recording::schema::schema()?.swap_remove(index);
        let kinds = columns.iter().map(|&column| (column < recorded).then(|| schema.get(column).copied()).flatten()).collect();
        let format = recording.format();
        let withheld = columns.iter().map(|&column| table.column_withheld_at(column, format)).collect();
        let mode = CaptureMode::from_byte(recording.root_identity().capture_mode)?;
        let mut made = Self {
            table,
            fields,
            columns,
            kinds,
            withheld,
            format,
            mode,
            keys: Vec::new(),
            steps: Vec::new(),
            texts: Vec::new(),
            lists: HashMap::new(),
            statuses: Vec::new(),
            reasons: HashMap::new(),
            refused: None,
            changes: vec![from],
            held: Mutex::new(Vec::new()),
        };
        let history = match lanes::history(recording, run, table, &made.columns, keys, from, to) {
            Ok(history) => history,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                made.refused = Some(error.to_string());
                return Ok(made);
            }
            Err(error) => return Err(error),
        };
        let mut changes: Vec<i32> = history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour)).collect();
        changes.extend(history.statuses.iter().map(|run| run.first));
        changes.push(from);
        changes.sort_unstable();
        changes.dedup();
        made.changes = changes;
        made.texts = history.texts;
        made.statuses = history.statuses;
        made.reasons = history.reasons;
        (made.keys, made.steps) = history.keys.into_iter().unzip();
        made.lists = made.read_lists(recording, run)?;
        Ok(made)
    }

    fn read_lists(&self, recording: &Recording, run: u32) -> io::Result<HashMap<(usize, i32, usize), Listed>> {
        let listed: Vec<usize> = (0..self.columns.len()).filter(|&at| self.kinds[at].is_some_and(replay_recording::cells::list_column)).collect();
        let mut wanted: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
        for (key_at, steps) in self.steps.iter().enumerate() {
            let mut seen: Vec<i32> = Vec::new();
            for step in steps {
                for &at in &listed {
                    if let Some(Cell::Written(written)) = step.cells.as_ref().map(|cells| cells[at])
                        && !seen.contains(&written)
                    {
                        seen.push(written);
                        wanted.entry(written).or_default().push(key_at);
                    }
                }
            }
        }
        let mut out = HashMap::new();
        if wanted.is_empty() {
            return Ok(out);
        }
        let list_columns: Vec<usize> = listed.iter().map(|&at| self.columns[at]).collect();
        for (written, mut key_ats) in wanted {
            key_ats.sort_unstable();
            key_ats.dedup();
            let keys: Vec<u128> = key_ats.iter().map(|&at| self.keys[at]).collect();
            let snapshot = lanes::at_keys(recording, run, self.table, written, Some(&list_columns), Keys::These(&keys))?;
            let view = snapshot.view();
            for &key_at in &key_ats {
                let slot = snapshot.rows.iter().find(|(key, _)| *key == self.keys[key_at]).map(|&(_, slot)| slot);
                for &at in &listed {
                    let held = match slot.map(|slot| view.value(self.columns[at], slot)) {
                        Some(Value::List(bytes)) => Listed::Bytes(bytes.into()),
                        Some(Value::Missing) => Listed::Missing,
                        _ => Listed::Unrecorded,
                    };
                    out.insert((key_at, written, at), held);
                }
            }
        }
        Ok(out)
    }

    pub fn table(&self) -> TableId {
        self.table
    }

    pub fn epoch(&self, hour: i32) -> i32 {
        let at = self.changes.partition_point(|&change| change <= hour);
        at.checked_sub(1).map_or(i32::MIN, |at| self.changes[at])
    }

    fn status(&self, hour: i32) -> Status {
        let at = self.statuses.partition_point(|run| run.last < hour);
        self.statuses.get(at).filter(|run| run.first <= hour).map_or(Status::NotRecorded, |run| run.status)
    }

    fn cells(&self, key_at: usize, hour: i32) -> Option<&[Cell]> {
        let steps = &self.steps[key_at];
        steps.partition_point(|step| step.hour <= hour).checked_sub(1).and_then(|at| steps[at].cells.as_deref())
    }

    fn rows(&self, hour: i32, live: &[usize]) -> io::Result<replay_data::rows::TableRows> {
        let keys: Vec<u128> = live.iter().map(|&at| self.keys[at]).collect();
        let cells: Vec<&[Cell]> = live.iter().map(|&at| self.cells(at, hour).unwrap_or_default()).collect();
        rows_of_fields(self.table, &keys, self.fields, &mut |row, column| {
            let Ok(at) = self.columns.binary_search(&column) else { return Ok(replay_data::rows::Field::NotRecorded) };
            if self.withheld[at] {
                return Ok(replay_data::rows::Field::NotRecorded);
            }
            let Some(kind) = self.kinds[at] else { return Ok(replay_data::rows::Field::NotRecorded) };
            let value = match cells[row].get(at).copied().unwrap_or(Cell::Unrecorded) {
                Cell::Number(number) => Value::Number(number as u64),
                Cell::Text(text) => Value::Text(text),
                Cell::Missing => Value::Missing,
                Cell::Unrecorded => Value::Unrecorded,
                Cell::Written(written) => match self.lists.get(&(live[row], written, at)) {
                    Some(Listed::Bytes(bytes)) => Value::List(bytes),
                    Some(Listed::Missing) => Value::Missing,
                    Some(Listed::Unrecorded) | None => Value::Unrecorded,
                },
            };
            value_field(kind, value, |text| self.texts.get(text as usize).map(|text| &**text), self.format)
        })
    }

    fn made(&self, hour: i32, live: &[usize]) -> io::Result<SharedView> {
        if let Some(reason) = &self.refused {
            return Ok(TableView::Unavailable { reason: reason.clone() }.into());
        }
        let view = match self.status(hour) {
            Status::NotRecorded => Ok(TableView::NotRecorded { mode: self.mode }),
            Status::Unavailable { reason } => Ok(TableView::Unavailable { reason: self.reasons.get(&reason).cloned().unwrap_or_else(|| "A table's reason is not text".to_owned()) }),
            Status::Complete => self.rows(hour, live).map(TableView::Complete),
            Status::Truncated { missing } => self.rows(hour, live).map(|rows| TableView::Truncated { rows, missing }),
        };
        match view {
            Ok(view) => Ok(view.into()),
            Err(error) if Unsayable::of(&error).is_some() => Ok(TableView::Unavailable { reason: error.to_string() }.into()),
            Err(error) => Err(error),
        }
    }

    pub fn view(&self, hour: i32) -> io::Result<SharedView> {
        let epoch = self.epoch(hour);
        {
            let held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some((_, view)) = held.iter().find(|(at, _)| *at == epoch) {
                return Ok(view.clone());
            }
        }
        let live: Vec<usize> = (0..self.keys.len()).filter(|&at| self.cells(at, epoch).is_some()).collect();
        let view = self.made(epoch, &live)?;
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if held.len() >= KEPT_VIEWS {
            held.remove(0);
        }
        held.push((epoch, view.clone()));
        Ok(view)
    }

    pub fn view_of(&self, hour: i32, keys: &[u128]) -> io::Result<SharedView> {
        let mut asked = keys.to_vec();
        asked.sort_unstable();
        asked.dedup();
        let live: Vec<usize> = asked.iter().filter_map(|key| self.keys.binary_search(key).ok()).filter(|&at| self.cells(at, hour).is_some()).collect();
        self.made(hour, &live)
    }
}
