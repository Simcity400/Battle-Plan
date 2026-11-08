//! Reading a keyed family's prepared pages for a query: the catalogue (the
//! carried scan with its names) and the runs of the names a request selects
//! within its window, clipped to the hours preparation has accepted. A family
//! prepared on a native recording is read from its publication alike
//! ([`crate::native`]).

use crate::{KNOWN, Observer, READ, Scan, native, series_section};
use dashboard_base::prepared::series::{self, Reader, Run};
use dashboard_base::products::country_groups::HourSpan;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
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
    /// Sent with each stretch of hours read at their own hour as one `[first, last, null]`.
    #[serde(serialize_with = "sent_reads")]
    #[ts(type = "Array<[number, number, number | null]>")]
    pub reads: Vec<(i32, i32, i32)>,
    pub series: Vec<KeyedSeries>,
}

/// `reads` as replies carry them: consecutive hours each supplied by its own read become one
/// `[first, last, null]`, so a family read every hour of a long campaign sends one entry rather
/// than one an hour; every other stretch is sent as it is.
fn sent_reads<S: serde::Serializer>(
    reads: &[(i32, i32, i32)],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut sequence = serializer.serialize_seq(None)?;
    let mut own: Option<(i32, i32)> = None;
    for &(first, last, read) in reads {
        if first == last && read == first {
            match &mut own {
                Some((_, end)) if end.checked_add(1) == Some(first) => *end = first,
                _ => {
                    if let Some((from, to)) = own.replace((first, first)) {
                        sequence.serialize_element(&(from, to, None::<i32>))?;
                    }
                }
            }
            continue;
        }
        if let Some((from, to)) = own.take() {
            sequence.serialize_element(&(from, to, None::<i32>))?;
        }
        sequence.serialize_element(&(first, last, Some(read)))?;
    }
    if let Some((from, to)) = own {
        sequence.serialize_element(&(from, to, None::<i32>))?;
    }
    sequence.end()
}

/// The window `from..=to` of `keyed` series, read only through `through`.
pub fn window(
    reader: &mut Reader<'_>,
    keyed: impl IntoIterator<Item = (String, u64)>,
    from: i32,
    to: i32,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<KeyedWindow> {
    if from > to {
        return Ok(empty(from, to));
    }
    let keyed: Vec<(String, u64)> = keyed.into_iter().collect();
    let mut runs = Vec::with_capacity(keyed.len() + 2);
    for key in [KNOWN, READ].into_iter().chain(keyed.iter().map(|(_, key)| *key)) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        runs.push(reader.runs(key, from, to)?);
    }
    windowed(keyed, runs, from, to, through)
}

/// The window `from..=to` of a native publication's series, read only through `through`: its
/// known hours' and reads' keys `own`, then each name's key, none where it is not held yet.
pub fn published_window(
    opened: &dashboard_base::prepared::published::Opened,
    own: [Option<dashboard_series::KeyHandle>; 2],
    keyed: Vec<(String, Option<dashboard_series::KeyHandle>)>,
    (from, to): (i32, i32),
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<KeyedWindow> {
    if from > to {
        return Ok(empty(from, to));
    }
    let mut handles = own.to_vec();
    handles.extend(keyed.iter().map(|(_, handle)| *handle));
    let runs = opened.read(cancelled, |snapshot, _| {
        native::held_runs(snapshot, &handles, from, to)
    })?;
    windowed(keyed, runs, from, to, through)
}

/// The window `from..=to` of `keyed` series, given KNOWN's runs, READ's and then each of
/// theirs, read only through `through`.
pub(crate) fn windowed<K>(
    keyed: Vec<(String, K)>,
    runs: Vec<Vec<Run>>,
    from: i32,
    to: i32,
    through: Option<i32>,
) -> io::Result<KeyedWindow> {
    let mut window = empty(from, to);
    let mut runs = runs.into_iter();
    let (Some(known), Some(reads)) = (runs.next(), runs.next()) else {
        return Err(io::Error::other("A keyed window misses its own runs"));
    };
    for run in clipped(known, through) {
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
    for run in clipped(reads, through) {
        let hour = i32::try_from(run.value)
            .map_err(|_| io::Error::other("Invalid supplying read hour"))?;
        window.reads.push((run.first_hour, run.last_hour, hour));
    }
    for ((name, _), runs) in keyed.into_iter().zip(runs) {
        let runs: Vec<KeyedRun> = clipped(runs, through)
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

/// A keyed family's published catalogue and a reader of its series.
pub struct Opened<'a, O> {
    pub scan: Arc<Scan<O>>,
    series: Series<'a>,
    cancelled: &'a AtomicBool,
}

/// Where a family's runs are read: the walk's pages, or a native publication.
enum Series<'a> {
    Pages(Reader<'a>),
    Native(native::Opened),
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
        scan: Arc::new(scan),
        series: Series::Pages(reader),
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
        let keyed: Vec<(String, u64)> = self
            .scan
            .keyed()
            .filter(|(name, _)| select(name))
            .map(|(name, key)| (name.to_owned(), key))
            .collect();
        match &mut self.series {
            Series::Pages(reader) => window(reader, keyed, from, to, through, self.cancelled),
            Series::Native(opened) => {
                if from > to {
                    return Ok(empty(from, to));
                }
                let mut keys = vec![KNOWN, READ];
                keys.extend(keyed.iter().map(|(_, key)| *key));
                let runs = opened.runs(&keys, from, to, self.cancelled)?;
                windowed(keyed, runs, from, to, through)
            }
        }
    }

    /// The values of `keys` at `hour`, None where a key holds none then.
    pub fn values_at(&mut self, keys: &[u64], hour: i32) -> io::Result<Vec<Option<i128>>> {
        match &mut self.series {
            Series::Pages(reader) => reader.values_at(keys, hour),
            Series::Native(opened) => opened.values_at(keys, hour, self.cancelled),
        }
    }

    /// [`Self::values_at`] of `keys` at each of the ascending `hours`, as each key's
    /// changes (see [`Reader::changes_at`]).
    pub fn changes_at(
        &mut self,
        keys: &[u64],
        hours: &[i32],
    ) -> io::Result<Vec<Vec<(usize, Option<i128>)>>> {
        match &mut self.series {
            Series::Pages(reader) => reader.changes_at(keys, hours),
            Series::Native(opened) => opened.changes_at(keys, hours, self.cancelled),
        }
    }
}

/// The family's publication as `opened` holds it: its catalogue is read once a publication and
/// shared by every reply reading it ([`dashboard_base::prepared::published::Opened::derived`]).
pub(crate) fn published<O: Observer>(
    opened: dashboard_base::prepared::published::Opened,
    cancelled: &AtomicBool,
) -> io::Result<Opened<'_, O>> {
    let scan = opened.derived("keyed scan", cancelled, |_, directory| {
        native::catalogued::<O>(directory)
    })?;
    let keys = opened.derived("keyed keys", cancelled, |_, directory| {
        native::keys(directory, &scan)
    })?;
    Ok(Opened {
        scan,
        series: Series::Native(native::Opened { opened, keys }),
        cancelled,
    })
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
            .with_metadata(|metadata| {
                metadata
                    .branches
                    .iter()
                    .find(|item| item.id.parse::<u64>().ok() == Some(branch))
                    .map(|item| (item.first_hour, item.last_hour))
            })
            .ok_or_else(|| dashboard_core::invalid("Unknown branch"))
    }

    /// The family's pages; None before its first publication.
    pub fn open<O: Observer>(&self) -> io::Result<Option<Opened<'_, O>>> {
        open::<O>(self.store, self.key, self.branch, self.cancelled)
    }

    /// The family's publication on a native recording ([`native`]); None before its first.
    pub fn open_native<O: Observer>(&self) -> io::Result<Option<Opened<'_, O>>> {
        dashboard_base::prepared::published::open(
            self.source,
            self.content,
            O::NAME,
            native::SCHEMA,
            self.branch,
        )?
        .map(|opened| published(opened, self.cancelled))
        .transpose()
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

/// A reply written once as JSON within the product budget.
pub fn finish(
    query: &Query<'_>,
    reply: &impl Serialize,
) -> io::Result<dashboard_base::products::built::Built> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    dashboard_base::products::built::Built::of(reply, query.cancelled)
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
