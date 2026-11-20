//! What a followed query asks: a table, columns and rows (keys, all, the top N by a column, or
//! those appended since the last push), or a declared series' measure for countries through the
//! one identity rule. Its declaration to the playhead is its table and columns.

use crate::{invalid, table_name};
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    identity::{Country, Holder, Holders, NOT_IN_HISTORY},
    timeline::Timeline,
};
use replay_columnar::summary::Source;
use replay_data::{Hour, rows::TableId};
use std::{collections::HashMap, io, sync::Arc};

/// The rows of its table a query reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rows {
    /// These, by key, in this order; a key with no live row answers "no row".
    Keys(Vec<u128>),
    /// As `Keys`, each once; after the first answer, each push holds only the rows that differ
    /// from the push before it (an answer of rows changed), so a push costs what changed.
    Live(Vec<u128>),
    /// Every live row, by key ascending.
    All,
    /// The `n` live rows greatest in `column` (ties by key ascending), greatest first; a row
    /// without a value there is not ranked.
    Top { column: usize, n: usize },
    /// The rows inserted since the last push, in insert order, each with its insert hour.
    Appended,
    /// A log's rows the game lists now, in ordinal order, each by its key; their cells are asked
    /// by ordinal while the query is followed. Refused for a table holding state.
    Listed,
}

/// The identity a series' countries are read through: the query layer's holders and timeline.
#[derive(Clone)]
pub struct Identity {
    pub holders: Arc<Holders>,
    pub timeline: Arc<Timeline>,
}

#[derive(Clone)]
pub enum Query {
    Cells { table: usize, columns: Vec<usize>, rows: Rows },
    /// A declared series' measure at the head for each country, as `series-state` answers it; with
    /// `top`, the `top` tags greatest in it (a tie by tag in byte order, as the UI ranks), greatest first, of every tag held, each
    /// row named by its tag. With `parts` (a sub-keyed series), each country's parts at the head in
    /// key order, or with `top` its `top` parts greatest in it (ties by key), each row named by
    /// its country ([`Country::key`]). With `least`, an owners' top is the `top` least instead, least first.
    Series { series: &'static Series, measure: &'static Measure, countries: Vec<Country>, top: Option<usize>, least: bool, parts: bool, identity: Identity },
    /// A value its owner makes from its own source ([`crate::head`]).
    Head(crate::head::Head),
}

impl Query {
    /// Refused by name unless its table and columns exist and it reads what the head can answer.
    pub(crate) fn check(&self) -> io::Result<()> {
        match self {
            Self::Cells { table, columns, rows } => {
                let id = TableId::ALL.get(*table).ok_or_else(|| invalid(format!("No table {table}")))?;
                if columns.is_empty() {
                    return Err(invalid(format!("A query of {} reads no column", table_name(*table))));
                }
                if let Rows::Live(keys) = rows
                    && keys.windows(2).any(|pair| pair[0] >= pair[1])
                {
                    return Err(invalid(format!("A live query of {} names its keys ascending, each once", table_name(*table))));
                }
                let mut seen = columns.clone();
                if let Rows::Top { column, n } = rows {
                    if *n == 0 {
                        return Err(invalid("A top of no rows"));
                    }
                    seen.push(*column);
                }
                for (at, &column) in seen.iter().enumerate() {
                    if column >= id.field_count() {
                        return Err(invalid(format!("{} holds no column {column}", table_name(*table))));
                    }
                    if at < columns.len() && columns[..at].contains(&column) {
                        return Err(invalid(format!("Column {column} of {} is asked twice", table_name(*table))));
                    }
                }
                Ok(())
            }
            Self::Series { series, measure, top, parts, .. } => {
                if *top == Some(0) {
                    return Err(invalid("A top of no rows"));
                }
                if !series.measures.iter().any(|held| std::ptr::eq(held, *measure)) {
                    return Err(invalid(format!("{} has no measure {}", series.kind, measure.name)));
                }
                match (series.parts.is_some(), *parts) {
                    (true, false) => return Err(invalid(format!("{} is followed by its owners' parts", series.kind))),
                    (false, true) => return Err(invalid(format!("{} has no parts", series.kind))),
                    _ => {}
                }
                dashboard_declare::source(series.table, measure).map(drop)
            }
            Self::Head(_) => Ok(()),
        }
    }

    /// Where a series' value comes from; none for a query of cells.
    pub(crate) fn source(&self) -> Option<Source> {
        match self {
            Self::Cells { .. } | Self::Head(_) => None,
            Self::Series { series, measure, .. } => dashboard_declare::source(series.table, measure).ok(),
        }
    }

    /// A series over its owners' parts.
    pub(crate) fn parts(&self) -> bool {
        matches!(self, Self::Series { parts: true, .. })
    }

    pub fn table(&self) -> usize {
        match self {
            Self::Cells { table, .. } => *table,
            Self::Series { series, .. } => series.table.index(),
            Self::Head(_) => crate::head::TABLE,
        }
    }

    /// The columns its answer reads, in order: a series' value's inputs.
    pub(crate) fn shown(&self) -> Vec<usize> {
        match self {
            Self::Cells { columns, .. } => columns.clone(),
            Self::Series { .. } => self.source().map(|source| dashboard_query::read::columns_of(&source)).unwrap_or_default(),
            Self::Head(_) => Vec::new(),
        }
    }

    /// The columns it declares to the playhead, ascending.
    pub(crate) fn declared(&self) -> Vec<usize> {
        let mut columns = self.shown();
        if let Self::Cells { rows: Rows::Top { column, .. }, .. } = self {
            columns.push(*column);
        }
        columns.sort_unstable();
        columns.dedup();
        columns
    }

    /// Its pushes after the first hold the rows that changed.
    pub(crate) fn live(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Live(_), .. })
    }

    pub(crate) fn appended(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Appended, .. })
    }

    pub(crate) fn listed(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Listed, .. })
    }

    /// The series' measure over every tag held, its `n` greatest at the head.
    pub fn series_top(series: &'static Series, measure: &'static Measure, n: usize, identity: Identity) -> Self {
        let countries = identity.holders.tags.keys().map(Country::from).collect();
        Self::Series { series, measure, countries, top: Some(n), least: false, parts: false, identity }
    }

    /// An owners' top as its `n` least instead, least first.
    pub fn least(self) -> Self {
        match self {
            Self::Series { series, measure, countries, top, parts, identity, .. } => Self::Series { series, measure, countries, top, least: true, parts, identity },
            other => other,
        }
    }
}

/// A series' tag at an hour: its holder's key, or why it has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TagRow {
    Key(u128),
    Absent(String),
}

/// A series' tags at an hour, and for a top, each holder's key to its tag.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tags {
    pub(crate) rows: Vec<TagRow>,
    pub(crate) by_key: HashMap<u128, u32>,
}

/// A series' tags at `hour`; none for a query of cells.
pub(crate) fn tags_at(query: &Query, hour: Hour) -> Tags {
    let rows = tag_rows(query, hour);
    let by_key = match query {
        Query::Series { top: Some(_), parts: false, .. } => rows.iter().enumerate().filter_map(|(at, row)| if let TagRow::Key(key) = row { Some((*key, at as u32)) } else { None }).collect(),
        _ => HashMap::new(),
    };
    Tags { rows, by_key }
}

/// Each of a series' tags at `hour`, through the one identity rule; none for a query of cells.
fn tag_rows(query: &Query, hour: Hour) -> Vec<TagRow> {
    let Query::Series { series, measure, countries, identity, .. } = query else { return Vec::new() };
    let unheld = identity.timeline.at(hour).is_none().then(|| identity.timeline.reason(hour));
    let unknown = identity.holders.unknown.iter().find(|(first, last, _)| *first <= hour && hour <= *last).map(|(_, _, reason)| reason);
    countries
        .iter()
        .map(|country| {
            if let Some(reason) = &unheld {
                return TagRow::Absent(reason.clone());
            }
            if series.key.unowned() {
                return country.tag().and_then(|tag| series.key.fixed(tag)).map_or_else(|| TagRow::Absent(identity.holders.say(country, hour, |name| format!("{name} names no row of {}", series.kind))), TagRow::Key);
            }
            let Some(name) = identity.holders.label(country, hour) else { return TagRow::Absent(NOT_IN_HISTORY.to_owned()) };
            if let Some(reason) = unknown {
                return TagRow::Absent(format!("Who holds {name} is not known: {reason}"));
            }
            match identity.holders.at(country, hour).map(|run| run.holder) {
                None => TagRow::Absent(match country {
                    Country::Tag(tag) => format!("{tag} is held by no country"),
                    Country::Slot(_) => format!("{name}'s {} was not recorded at this hour", measure.name),
                }),
                Some(Holder::Unresolved) => TagRow::Absent(format!("No link resolves the country holding {name}")),
                Some(Holder::Found { slot, id }) => TagRow::Key(holder_key(series, slot, id)),
            }
        })
        .collect()
}

/// A series' owner key of a holder.
pub(crate) fn holder_key(series: &Series, slot: u32, id: u32) -> u128 {
    series.key.of(slot, id)
}

/// The next hour after `hour` at which who holds any of `countries`, or whether the timeline holds
/// the hour, changes: the answer cannot change from identity before it.
pub(crate) fn identity_until(identity: &Identity, countries: &[Country], hour: Hour) -> Hour {
    let mut until = Hour::MAX;
    let mut after = |edge: Hour| {
        if edge > hour {
            until = until.min(edge);
        }
    };
    for country in countries {
        let runs = identity.holders.runs(country);
        let at = runs.partition_point(|run| run.last < hour);
        if let Some(run) = runs.get(at) {
            after(run.first);
            after(run.last.saturating_add(1));
        }
    }
    for &(first, last, _) in &identity.holders.unknown {
        after(first);
        after(last.saturating_add(1));
    }
    let stretches = &identity.timeline.stretches;
    let at = stretches.partition_point(|stretch| stretch.last < hour);
    if let Some(stretch) = stretches.get(at) {
        after(stretch.first);
        after(stretch.last.saturating_add(1));
    }
    until
}
