
use crate::{invalid, table_name};
use dashboard_declare::{Measure, Series};
use dashboard_query::{
    coverage::CoverageScope,
    identity::{Country, Holder, Holders, NOT_IN_HISTORY},
    timeline::Timeline,
};
use replay_columnar::summary::Source;
use replay_data::{Hour, rows::TableId};
use std::{collections::HashMap, io, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rows {
    Keys(Vec<u128>),
    Live(Vec<u128>),
    LiveTable(Vec<u128>),
    All,
    Top { column: usize, n: usize },
    Appended,
    Listed,
}

#[derive(Clone)]
pub struct Identity {
    pub holders: Arc<Holders>,
    pub timeline: Arc<Timeline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeriesMode {
    Keyed(CoverageScope),
    Top { n: usize, least: bool },
    Parts { top: Option<usize> },
    NavalLosses { attribution: u8, kinds: &'static [u8], top: Option<usize> },
}

#[derive(Clone)]
pub enum Query {
    Cells { table: usize, columns: Vec<usize>, rows: Rows },
    Series { series: &'static Series, measure: &'static Measure, countries: Vec<Country>, mode: SeriesMode, identity: Identity },
    Head(crate::head::Head),
}

impl Query {
    pub(crate) fn check(&self) -> io::Result<()> {
        match self {
            Self::Cells { table, columns, rows } => {
                let id = TableId::ALL.get(*table).ok_or_else(|| invalid(format!("No table {table}")))?;
                if columns.is_empty() {
                    return Err(invalid(format!("A query of {} reads no column", table_name(*table))));
                }
                if let Rows::Live(keys) | Rows::LiveTable(keys) = rows
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
            Self::Series { series, measure, mode, .. } => {
                if matches!(mode, SeriesMode::Top { n: 0, .. } | SeriesMode::Parts { top: Some(0) } | SeriesMode::NavalLosses { top: Some(0), .. }) {
                    return Err(invalid("A top of no rows"));
                }
                if !series.measures.iter().any(|held| std::ptr::eq(held, *measure)) {
                    return Err(invalid(format!("{} has no measure {}", series.kind, measure.name)));
                }
                if matches!(mode, SeriesMode::NavalLosses { .. }) {
                    if series.table != TableId::NavalLossTotals {
                        return Err(invalid(format!("{} is not naval losses", series.kind)));
                    }
                } else {
                    let parts = matches!(mode, SeriesMode::Parts { .. });
                    match (series.parts.is_some(), parts) {
                        (true, false) => return Err(invalid(format!("{} is followed by its owners' parts", series.kind))),
                        (false, true) => return Err(invalid(format!("{} has no parts", series.kind))),
                        _ => {}
                    }
                }
                dashboard_declare::source(series.table, measure).map(drop)
            }
            Self::Head(_) => Ok(()),
        }
    }

    pub(crate) fn source(&self) -> Option<Source> {
        match self {
            Self::Cells { .. } | Self::Head(_) => None,
            Self::Series { series, measure, .. } => dashboard_declare::source(series.table, measure).ok(),
        }
    }

    pub(crate) fn parts(&self) -> bool {
        matches!(self, Self::Series { mode: SeriesMode::Parts { .. }, .. })
    }

    pub(crate) fn witness_sensitive(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Keys(_) | Rows::Live(_), .. } | Self::Series { mode: SeriesMode::Keyed(CoverageScope::Row), .. })
    }

    pub fn table(&self) -> usize {
        match self {
            Self::Cells { table, .. } => *table,
            Self::Series { series, .. } => series.table.index(),
            Self::Head(_) => crate::head::TABLE,
        }
    }

    pub(crate) fn shown(&self) -> Vec<usize> {
        match self {
            Self::Cells { columns, .. } => columns.clone(),
            Self::Series { .. } => self.source().map(|source| dashboard_query::read::columns_of(&source)).unwrap_or_default(),
            Self::Head(_) => Vec::new(),
        }
    }

    pub(crate) fn declared(&self) -> Vec<usize> {
        let mut columns = self.shown();
        if let Self::Cells { rows: Rows::Top { column, .. }, .. } = self {
            columns.push(*column);
        }
        if let Self::Series { mode: SeriesMode::NavalLosses { .. }, .. } = self {
            columns.extend([2, 3]);
        }
        columns.sort_unstable();
        columns.dedup();
        columns
    }

    pub(crate) fn live(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Live(_) | Rows::LiveTable(_), .. })
    }

    pub(crate) fn appended(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Appended, .. })
    }

    pub(crate) fn listed(&self) -> bool {
        matches!(self, Self::Cells { rows: Rows::Listed, .. })
    }

    pub fn series_top(series: &'static Series, measure: &'static Measure, n: usize, identity: Identity) -> Self {
        let countries = identity.holders.tags.keys().map(Country::from).collect();
        Self::Series { series, measure, countries, mode: SeriesMode::Top { n, least: false }, identity }
    }

    pub fn least(self) -> Self {
        match self {
            Self::Series { series, measure, countries, mode: SeriesMode::Top { n, .. }, identity } => {
                Self::Series { series, measure, countries, mode: SeriesMode::Top { n, least: true }, identity }
            }
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TagRow {
    Key(u128),
    Absent(String),
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Tags {
    pub(crate) rows: Vec<TagRow>,
    pub(crate) by_key: HashMap<u128, u32>,
}

pub(crate) fn tags_at(query: &Query, hour: Hour) -> Tags {
    let rows = tag_rows(query, hour);
    let by_key = match query {
        Query::Series { mode: SeriesMode::Top { .. }, .. } => rows.iter().enumerate().filter_map(|(at, row)| if let TagRow::Key(key) = row { Some((*key, at as u32)) } else { None }).collect(),
        _ => HashMap::new(),
    };
    Tags { rows, by_key }
}

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

pub(crate) fn holder_key(series: &Series, slot: u32, id: u32) -> u128 {
    series.key.of(slot, id)
}

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
