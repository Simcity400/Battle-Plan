//! Reading a keyed family's prepared pages for a query: the catalogue (the
//! carried scan with its names) and the runs of the names a request selects
//! within its window, clipped to the hours preparation has accepted.

use crate::{KNOWN, Observer, READ, Scan, series_section};
use dashboard_base::prepared::series::{self, Reader, Run};
use dashboard_base::products::country_groups::HourSpan;
use serde::Serialize;
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

/// One run of exact values: its first and last hour, the value as decimal
/// text, and whether it starts a new observation (after a gap or an hour
/// not recorded) rather than continuing the run before it.
pub type KeyedRun = (i32, i32, String, bool);

/// One name's runs within a window, in hour order.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KeyedSeries {
    pub name: String,
    #[ts(type = "Array<[number, number, string, boolean]>")]
    pub runs: Vec<KeyedRun>,
}

/// A window of a keyed family: the hours whose inputs were read, the
/// supplying read hour of each stretch (first hour, last hour, read hour), and
/// the selected names' runs.
#[derive(Clone, Debug, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KeyedWindow {
    pub first_hour: i32,
    pub last_hour: i32,
    pub known: Vec<HourSpan>,
    #[ts(type = "Array<[number, number, number]>")]
    pub reads: Vec<(i32, i32, i32)>,
    pub series: Vec<KeyedSeries>,
}

/// A keyed family's published catalogue and a reader of its series.
pub struct Opened<'a, O> {
    pub scan: Scan<O>,
    pub reader: Reader<'a>,
    cancelled: &'a AtomicBool,
}

/// The family's pages on `branch`; None before its first publication.
pub fn open<'a, O: Observer>(
    store: &'a store::Store,
    key: &'a store::SourceKey,
    branch: u64,
    cancelled: &'a AtomicBool,
) -> io::Result<Option<Opened<'a, O>>> {
    let Some((_, scan)) = series::read_catalogue::<Scan<O>>(store, key, O::NAME, branch)? else {
        return Ok(None);
    };
    let reader = Reader::new(store, key, series_section(O::NAME, branch), cancelled)?;
    Ok(Some(Opened {
        scan,
        reader,
        cancelled,
    }))
}

fn clipped(runs: Vec<Run>, through: Option<i32>) -> impl Iterator<Item = Run> {
    runs.into_iter().filter_map(move |mut run| {
        if let Some(last) = through {
            if run.first_hour > last {
                return None;
            }
            run.last_hour = run.last_hour.min(last);
        }
        Some(run)
    })
}

impl<O: Observer> Opened<'_, O> {
    /// The window `from..=to` of the names `select` keeps, read only through
    /// `through` (the accepted prefix's last hour, None when complete).
    pub fn window(
        &mut self,
        from: i32,
        to: i32,
        through: Option<i32>,
        mut select: impl FnMut(&str) -> bool,
    ) -> io::Result<KeyedWindow> {
        let mut window = KeyedWindow {
            first_hour: from,
            last_hour: to,
            ..Default::default()
        };
        if from > to {
            return Ok(window);
        }
        for run in clipped(self.reader.runs(KNOWN, from, to)?, through) {
            match window.known.last_mut() {
                Some(span) if span.last_hour.checked_add(1) == Some(run.first_hour) => {
                    span.last_hour = run.last_hour;
                }
                _ => window.known.push(HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                }),
            }
        }
        for run in clipped(self.reader.runs(READ, from, to)?, through) {
            let hour = i32::try_from(run.value)
                .map_err(|_| io::Error::other("Invalid supplying read hour"))?;
            window.reads.push((run.first_hour, run.last_hour, hour));
        }
        let keyed: Vec<(String, u64)> = self
            .scan
            .keyed()
            .filter(|(name, _)| select(name))
            .map(|(name, key)| (name.to_owned(), key))
            .collect();
        for (name, key) in keyed {
            dashboard_text::metadata::check_cancel(self.cancelled)?;
            let runs: Vec<KeyedRun> = clipped(self.reader.runs(key, from, to)?, through)
                .map(|run| {
                    (
                        run.first_hour,
                        run.last_hour,
                        run.value.to_string(),
                        run.break_before,
                    )
                })
                .collect();
            if !runs.is_empty() {
                window.series.push(KeyedSeries { name, runs });
            }
        }
        Ok(window)
    }

    /// The values of `keys` at `hour`, None where a key holds none then.
    pub fn values_at(&mut self, keys: &[u64], hour: i32) -> io::Result<Vec<Option<i128>>> {
        self.reader.values_at(keys, hour)
    }

    /// [`Self::values_at`] of `keys` at each of the ascending `hours`, as each key's
    /// changes (see [`Reader::changes_at`]).
    pub fn changes_at(
        &mut self,
        keys: &[u64],
        hours: &[i32],
    ) -> io::Result<Vec<Vec<(usize, Option<i128>)>>> {
        self.reader.changes_at(keys, hours)
    }
}

/// What a keyed product's query is given: the request, the recording and its
/// content, the prepared files and the prefix of its gate that the reply
/// answers from (marked on the reply by the caller).
pub struct Query<'a> {
    pub source: &'a dashboard_base::Source,
    pub params: &'a serde_json::Value,
    pub content: Option<&'a dashboard_core::config::ContentOptions>,
    pub store: &'a store::Store,
    pub key: &'a store::SourceKey,
    pub branch: u64,
    pub prefix: &'a dashboard_base::prepared::prefix::Prefix,
    pub cancelled: &'a AtomicBool,
}

impl Query<'_> {
    /// The last hour the prepared pages answer: None once the gate's
    /// coverage is complete.
    pub fn through(&self) -> Option<i32> {
        (!self.prefix.coverage.complete).then_some(self.prefix.coverage.last_hour)
    }

    /// The branch's recorded first and last hours.
    pub fn bounds(&self) -> io::Result<(i32, i32)> {
        let branch = self.branch;
        self.source
            .metadata()
            .branches
            .iter()
            .find(|item| item.id.parse::<u64>().ok() == Some(branch))
            .map(|item| (item.first_hour, item.last_hour))
            .ok_or_else(|| dashboard_core::invalid("Unknown branch"))
    }

    /// The family's pages; None before its first publication.
    pub fn open<O: Observer>(&self) -> io::Result<Option<Opened<'_, O>>> {
        open::<O>(self.store, self.key, self.branch, self.cancelled)
    }

    /// The requested window, `firstHour..=lastHour` of the params, within
    /// the branch (the whole branch when absent).
    pub fn window(&self) -> io::Result<(i32, i32)> {
        let (first, last) = self.bounds()?;
        let hour = |field: &str| -> io::Result<Option<i32>> {
            match self.params.get(field) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(value) => value
                    .as_i64()
                    .and_then(|hour| i32::try_from(hour).ok())
                    .map(Some)
                    .ok_or_else(|| dashboard_core::invalid("Invalid window hour")),
            }
        };
        Ok((
            hour("firstHour")?.map_or(first, |hour| hour.max(first)),
            hour("lastHour")?.map_or(last, |hour| hour.min(last)),
        ))
    }
}

/// A request's params, validated by its product and bound to the query's branch.
pub fn params<P: dashboard_base::products::Product>(query: &Query<'_>) -> io::Result<P::Params> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let params = serde_json::from_value(query.params.clone())?;
    P::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = query
        .params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<u64>().ok());
    if branch != Some(query.branch) {
        return Err(dashboard_core::invalid(
            "Query branch differs from branchId",
        ));
    }
    Ok(params)
}

/// The params every keyed product shares: a branch and an hour window.
pub fn validate_window(
    branch: &str,
    first: Option<i32>,
    last: Option<i32>,
) -> Result<(), &'static str> {
    branch.parse::<u64>().map_err(|_| "Invalid branch")?;
    if first.zip(last).is_some_and(|(first, last)| first > last) {
        return Err("Invalid hour window");
    }
    Ok(())
}

/// An empty window: the reply before a family's first publication.
pub fn empty(first: i32, last: i32) -> KeyedWindow {
    KeyedWindow {
        first_hour: first,
        last_hour: last,
        ..Default::default()
    }
}

/// A reply within the transport budget, as JSON.
pub fn finish(query: &Query<'_>, reply: &impl Serialize) -> io::Result<serde_json::Value> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    dashboard_base::products::check_reply_budget(reply)?;
    Ok(serde_json::to_value(reply)?)
}

/// Named refusals, counted: one warning line per reason.
pub fn refusals(notes: &BTreeMap<String, u64>, what: &str) -> Vec<String> {
    notes
        .iter()
        .map(|(why, count)| format!("{count} {what} refused: {why}"))
        .collect()
}

/// A content choice as replies name it: its raw key and its name in the game's language.
#[derive(Clone, Debug, Serialize, TS)]
#[ts(rename = "KeyedChoice")]
pub struct Choice {
    pub id: String,
    pub name: String,
}
