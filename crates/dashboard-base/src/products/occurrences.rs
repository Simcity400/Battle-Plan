use dashboard_follow::panel::{ExactReader, Panel, ProductRead, ProductReader};
use dashboard_query::{
    Query,
    log::occurrence::{Answer, Direction, Order},
    series::PAGE_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccurrenceSort {
    pub column: String,
    #[serde(default)]
    pub direction: Direction,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OccurrenceCount {
    pub key: String,
    pub total: String,
}

pub trait OccurrenceReply: Clone + Serialize + Send + Sync + 'static {
    fn next_change(&self) -> Option<i32>;
    fn row_count(&self) -> usize;
}

pub fn cutoff(query: &Query<'_>, hour: Option<i32>) -> io::Result<i32> {
    let hour = hour.unwrap_or(query.timeline.last);
    if hour < query.timeline.first {
        return Err(invalid("The hour asked is before the branch"));
    }
    Ok(hour.min(query.timeline.last))
}

pub fn window(offset: Option<&str>, limit: u32) -> io::Result<(u64, u32)> {
    let offset = offset.unwrap_or("0").parse::<u64>().map_err(|_| invalid("Invalid offset"))?;
    if !(1..=PAGE_LIMIT).contains(&limit) {
        return Err(invalid("Invalid window limit"));
    }
    Ok((offset, limit))
}

pub fn order(sort: Option<&OccurrenceSort>, columns: &[&str]) -> io::Result<(Order, bool)> {
    match sort {
        None => Ok((Order::Date(Direction::Descending), true)),
        Some(sort) if sort.column == "date" => Ok((Order::Date(sort.direction), sort.direction == Direction::Descending)),
        Some(sort) => columns
            .iter()
            .position(|name| *name == sort.column)
            .map(|at| (Order::Key(at, sort.direction), false))
            .ok_or_else(|| invalid(format!("This log cannot be sorted by {}", sort.column))),
    }
}

pub fn counts(answer: &Answer, keys: &[&str]) -> Vec<OccurrenceCount> {
    keys.iter().zip(&answer.chips).map(|(key, total)| OccurrenceCount { key: (*key).to_owned(), total: total.to_string() }).collect()
}

struct Held<R> {
    hour: i32,
    reply: R,
    value: Arc<dashboard_follow::panel::PanelData>,
}

pub fn occurrence_panel<R, F>(executable: Option<engine_executable::executable_tables::Tables>, read: F) -> Box<dyn Panel>
where
    R: OccurrenceReply,
    F: Fn(i32, &AtomicBool) -> io::Result<R> + Send + Sync + 'static,
{
    let read = Arc::new(read);
    let step_read = Arc::clone(&read);
    let held: Arc<Mutex<Option<Held<R>>>> = Arc::new(Mutex::new(None));
    let advance: ProductReader = Arc::new(move |hour, seed, cancelled, _step| {
        if seed {
            let guard = held.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(prev) = guard.as_ref()
                && prev.hour <= hour
                && prev.reply.next_change().is_none_or(|next| hour < next)
            {
                return ProductRead { value: Arc::clone(&prev.value), rows: prev.reply.row_count() as u64, fresh: false, pending: false, live: None, lanes: Vec::new() };
            }
        }
        match engine_executable::executable_tables::scope(executable, || step_read(hour, cancelled)) {
            Ok(reply) => {
                let rows = reply.row_count() as u64;
                let value = Arc::new(serde_json::to_value(&reply).map_err(|error| error.to_string()));
                *held.lock().unwrap_or_else(PoisonError::into_inner) = Some(Held { hour, reply, value: Arc::clone(&value) });
                ProductRead { value, rows, fresh: true, pending: false, live: None, lanes: Vec::new() }
            }
            Err(error) => ProductRead { value: Arc::new(Err(error.to_string())), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() },
        }
    });
    let cold: ExactReader = Arc::new(move |hour, cancelled| {
        let reply = engine_executable::executable_tables::scope(executable, || read(hour, cancelled)).map_err(|error| error.to_string())?;
        serde_json::to_value(reply).map_err(|error| error.to_string())
    });
    super::at_hour::product_panel(advance, cold)
}
