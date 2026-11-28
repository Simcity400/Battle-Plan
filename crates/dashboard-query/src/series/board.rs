
use super::{Line, invalid};
use crate::{
    identity::{Country, Holder},
    read,
};
use replay_lanes::lanes::Keys;
use replay_columnar::{manifest::Status, summary::Computed};
use replay_data::rows::TableId;
use replay_recording::cells::TableReader;
use std::{collections::{HashMap, hash_map::Entry}, io, sync::{Arc, Weak}};

pub struct Column<'q, 'a> {
    pub line: Line<'q, 'a>,
    pub partial: Option<Line<'q, 'a>>,
}

pub enum Rows<'r> {
    Countries(&'r [Country]),
    Top { by: usize, n: usize, least: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub value_raw: Option<String>,
    pub missing: Option<String>,
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub country: Country,
    pub tag: String,
    pub cells: Vec<Cell>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub rows: Vec<Row>,
    pub missing: Option<String>,
}

pub fn table_unrecorded(table: replay_data::rows::TableId, status: Status, reason: Option<&str>) -> Option<String> {
    if matches!(status, Status::Complete) {
        return None;
    }
    let words = table.words();
    let mut chars = words.chars();
    let name: String = chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default();
    match status {
        Status::Complete => None,
        Status::NotRecorded => Some(format!("{name} not recorded at this hour")),
        Status::Truncated { missing } => Some(format!("{name} read only in part at this hour ({missing} rows not read), and which rows were read is not held at the head")),
        Status::Unavailable { .. } => Some(match reason.filter(|reason| !reason.is_empty()) {
            Some(reason) => format!("{name} not available at this hour: {reason}"),
            None => format!("{name} not available at this hour"),
        }),
    }
}

pub fn measure_unrecorded(tag: &str, measure: &str) -> String {
    format!("{tag}'s {measure} was not recorded at this hour")
}

pub fn at(columns: &[Column<'_, '_>], rows: Rows<'_>, hour: i32) -> io::Result<Board> {
    at_with(columns, rows, hour, None)
}

#[derive(Default)]
pub struct Readers {
    recording: Option<(Weak<replay_recording::columnar_frames::Source>, replay::campaign::Revision)>,
    tables: HashMap<TableId, (Vec<usize>, TableReader)>,
}

pub fn top_with_readers(columns: &[Column<'_, '_>], by: usize, n: usize, least: bool, hour: i32, readers: &mut Readers) -> io::Result<Board> {
    let rows = || Rows::Top { by, n, least };
    match at_with(columns, rows(), hour, Some(readers)) {
        Ok(board) => Ok(board),
        Err(_) => {
            readers.tables.clear();
            at(columns, rows(), hour)
        }
    }
}

impl Readers {
    fn at(&mut self, line: &Line<'_, '_>, lines: &[&Line<'_, '_>], run: u32, hour: i32) -> io::Result<&TableReader> {
        let recording = line.query().replay.recording();
        let source = Arc::downgrade(&recording.source());
        let revision = recording.revision();
        if self.recording.as_ref().is_none_or(|(held, at)| !Weak::ptr_eq(held, &source) || *at != revision) {
            self.tables.clear();
            self.recording = Some((source, revision));
        }
        let table = line.series().table;
        let mut columns: Vec<_> = lines.iter().filter(|line| line.series().table == table).flat_map(|line| read::columns_of(&line.source())).collect();
        columns.sort_unstable();
        columns.dedup();
        let ordinal = read::ordinal(recording, run, hour)?;
        crate::cost::storage(|| {
            let (_, reader) = match self.tables.entry(table) {
                Entry::Occupied(entry) if entry.get().1.run() == run && columns.iter().all(|column| entry.get().0.binary_search(column).is_ok()) => entry.into_mut(),
                entry => {
                    if let Entry::Occupied(held) = &entry {
                        columns.extend_from_slice(&held.get().0);
                        columns.sort_unstable();
                        columns.dedup();
                    }
                    let mut reader = TableReader::new(recording, run, table, Some(&columns))?;
                    reader.index_keys();
                    entry.insert_entry((columns, reader)).into_mut()
                }
            };
            reader.go_to(recording, ordinal)?;
            Ok(&*reader)
        })
    }
}

fn at_with(columns: &[Column<'_, '_>], rows: Rows<'_>, hour: i32, retained: Option<&mut Readers>) -> io::Result<Board> {
    let Some(first) = columns.first() else {
        return Err(invalid("A board names no column"));
    };
    let query = first.line.query();
    let Some(stretch) = query.timeline.at(hour).copied() else {
        return Ok(Board { rows: Vec::new(), missing: Some(query.timeline.reason(hour)) });
    };
    let lines: Vec<&Line<'_, '_>> = columns.iter().flat_map(|column| std::iter::once(&column.line).chain(column.partial.as_ref())).collect();
    let (countries, mut values) = match rows {
        Rows::Countries(countries) => {
            let held: Vec<Country> = countries.iter().filter(|country| query.holders.at(*country, hour).is_some()).cloned().collect();
            let read = values_row(&lines, &held, hour, stretch.run)?;
            (held, read.into_iter())
        }
        Rows::Top { by, n, least } => {
            let rank = &columns.get(by).ok_or_else(|| invalid("A board ranks by a column it does not hold"))?.line;
            let mut retained = retained;
            let ranked = match retained.as_deref_mut() {
                Some(readers) => {
                    rank.owners_only("a rank at an hour")?;
                    if let Some(reason) = rank.withheld_at(stretch.run, hour) {
                        Err(reason.to_owned())
                    } else {
                        let reader = readers.at(rank, &lines, stretch.run, hour)?;
                        match table_unrecorded(rank.series().table, reader.status(), reader_reason(reader)) {
                            Some(reason) => Err(reason),
                            None => rank.population_in(&reader.view(), hour, Some(n), least),
                        }
                    }
                }
                None => rank.rank_at(hour, n, least)?,
            };
            let held = match ranked {
                Ok(ranked) => ranked,
                Err(reason) => return Ok(Board { rows: Vec::new(), missing: Some(reason) }),
            };
            let read = values_table(&lines, &held, hour, stretch.run, retained)?;
            (held, read.into_iter())
        }
    };
    let mut out: Vec<Row> = countries.into_iter().map(|country| Row { tag: query.holders.name(&country, hour), country, cells: Vec::with_capacity(columns.len()) }).collect();
    for column in columns {
        let read = values.next().expect("a column's values");
        let partial = column.partial.as_ref().map(|_| values.next().expect("a partial mark's values"));
        for (at, value) in read.into_iter().enumerate() {
            out[at].cells.push(match value {
                Ok(value) => Cell {
                    value_raw: Some(value.to_string()),
                    missing: None,
                    partial: partial.as_ref().is_some_and(|marks| matches!(marks[at], Ok(mark) if mark != 0)),
                },
                Err(reason) => Cell { value_raw: None, missing: Some(reason), partial: false },
            });
        }
    }
    Ok(Board { rows: out, missing: None })
}

struct TableGroup {
    table: replay_data::rows::TableId,
    lines: Vec<usize>,
    snapshot: Option<replay_lanes::lanes::Snapshot>,
}

fn read_table_groups(lines: &[&Line<'_, '_>], countries: &[Country], hour: i32, run: u32, cold: bool) -> io::Result<(Vec<Vec<Result<u128, String>>>, Vec<TableGroup>)> {
    let query = lines[0].query();
    let recording = query.replay.recording();
    let keys: Vec<Vec<Result<u128, String>>> = lines
        .iter()
        .map(|line| {
            countries
                .iter()
                .map(|country| match line.missing_at(country, hour) {
                    Some(reason) => Err(reason),
                    None => match query.holders.at(country, hour).map(|held| held.holder) {
                        Some(Holder::Found { slot, id }) => Ok(line.series().key.of(slot, id)),
                        _ => Err(line.absent(country, hour)),
                    },
                })
                .collect()
        })
        .collect();
    let mut seen = vec![false; lines.len()];
    let mut groups = Vec::new();
    for at in 0..lines.len() {
        if seen[at] {
            continue;
        }
        let table = lines[at].series().table;
        let group: Vec<usize> = (at..lines.len()).filter(|&other| !seen[other] && lines[other].series().table == table).collect();
        for &line in &group {
            seen[line] = true;
        }
        let mut asked: Vec<usize> = group.iter().flat_map(|&line| read::columns_of(&lines[line].source())).collect();
        asked.sort_unstable();
        asked.dedup();
        let mut wanted: Vec<u128> = group.iter().flat_map(|&line| keys[line].iter().filter_map(|key| key.as_ref().ok().copied())).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let snapshot = (cold && !wanted.is_empty())
            .then(|| crate::cost::storage(|| replay_lanes::lanes::at_keys(recording, run, table, hour, Some(&asked), Keys::These(&wanted))))
            .transpose()?;
        groups.push(TableGroup { table, lines: group, snapshot });
    }
    Ok((keys, groups))
}

fn unavailable_reason(snapshot: &replay_lanes::lanes::Snapshot) -> Option<&str> {
    match snapshot.status {
        Status::Unavailable { reason } => snapshot.strings.get(reason).and_then(|text| std::str::from_utf8(text).ok()),
        _ => None,
    }
}

fn reader_reason(reader: &TableReader) -> Option<&str> {
    match reader.status() {
        Status::Unavailable { reason } => reader.strings().get(reason).and_then(|text| std::str::from_utf8(text).ok()),
        _ => None,
    }
}

fn read_cell(line: &Line<'_, '_>, view: &replay_columnar::store::TableView<'_>, slot: u32, tag: &str) -> Result<i64, String> {
    match line.source().read(view, slot) {
        Computed::Value(value) => Ok(value),
        Computed::NotApplicable(reason) => Err(reason.to_owned()),
        Computed::Unrecorded => Err(measure_unrecorded(tag, line.measure_name())),
    }
}

fn values_table(lines: &[&Line<'_, '_>], countries: &[Country], hour: i32, run: u32, mut retained: Option<&mut Readers>) -> io::Result<Vec<Vec<Result<i64, String>>>> {
    let query = lines[0].query();
    let (keys, groups) = read_table_groups(lines, countries, hour, run, retained.is_none())?;
    let mut out: Vec<Option<Vec<Result<i64, String>>>> = vec![None; lines.len()];
    for group in groups {
        let snapshot = group.snapshot.as_ref();
        let reader = retained.as_deref_mut().filter(|_| group.lines.iter().any(|&line| keys[line].iter().any(Result::is_ok)))
            .map(|readers| readers.at(lines[group.lines[0]], lines, run, hour)).transpose()?;
        let view = reader.map(TableReader::view).or_else(|| snapshot.map(|snapshot| snapshot.view()));
        let unrecorded = reader.and_then(|reader| table_unrecorded(group.table, reader.status(), reader_reason(reader)))
            .or_else(|| snapshot.and_then(|snapshot| table_unrecorded(group.table, snapshot.status, unavailable_reason(snapshot))));
        for line in group.lines {
            let measure = lines[line].measure_name();
            out[line] = Some(
                keys[line]
                    .iter()
                    .zip(countries)
                    .map(|(key, country)| {
                        let key = key.clone()?;
                        if let Some(reason) = &unrecorded {
                            return Err(reason.clone());
                        }
                        let tag = query.holders.name(country, hour);
                        let view = view.as_ref().expect("a read view");
                        let slot = snapshot.map_or_else(|| view.slot(key), |snapshot| {
                            snapshot.rows.binary_search_by_key(&key, |&(k, _)| k).ok().map(|at| snapshot.rows[at].1)
                        });
                        let Some(slot) = slot else {
                            return if lines[line].series().absent_none { Ok(0) } else { Err(measure_unrecorded(&tag, measure)) };
                        };
                        read_cell(lines[line], view, slot, &tag)
                    })
                    .collect(),
            );
        }
    }
    Ok(out.into_iter().map(|values| values.expect("every line read")).collect())
}

fn values_row(lines: &[&Line<'_, '_>], countries: &[Country], hour: i32, run: u32) -> io::Result<Vec<Vec<Result<i64, String>>>> {
    let query = lines[0].query();
    let (keys, groups) = read_table_groups(lines, countries, hour, run, true)?;
    let mut out: Vec<Option<Vec<Result<i64, String>>>> = vec![None; lines.len()];
    for group in groups {
        let status_reason = group.snapshot.as_ref().and_then(unavailable_reason);
        let view = group.snapshot.as_ref().map(|snapshot| snapshot.view());
        for line in group.lines {
            let measure = lines[line].measure_name();
            out[line] = Some(
                keys[line]
                    .iter()
                    .zip(countries)
                    .map(|(key, country)| {
                        let key = key.clone()?;
                        let tag = query.holders.name(country, hour);
                        let snapshot = group.snapshot.as_ref().expect("a read table");
                        let slot = snapshot.rows.binary_search_by_key(&key, |&(k, _)| k).ok().map(|at| snapshot.rows[at].1);
                        if let crate::coverage::Coverage::NotRecorded(reason) = crate::coverage::row_coverage(group.table, snapshot.status, status_reason, slot.is_some()) {
                            return Err(reason);
                        }
                        let Some(slot) = slot else {
                            return if lines[line].series().absent_none { Ok(0) } else { Err(measure_unrecorded(&tag, measure)) };
                        };
                        read_cell(lines[line], view.as_ref().expect("a read view"), slot, &tag)
                    })
                    .collect(),
            );
        }
    }
    Ok(out.into_iter().map(|values| values.expect("every line read")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::*;

    fn line<'q, 'r>(query: &'q crate::Query<'r>, measure: &str) -> Line<'q, 'r> {
        let series = dashboard_declare::series("country-measure").unwrap();
        Line::new(query, series, series.measure(measure).unwrap()).unwrap()
    }

    fn power_column<'q, 'r>(query: &'q crate::Query<'r>) -> Column<'q, 'r> {
        Column { line: line(query, "politicalPower"), partial: None }
    }

    fn both_recorded() -> i32 {
        (ITA_FROM..FIRST + FIRST_HOURS).find(|&hour| power(9001, hour).is_some() && power(9002, hour).is_some()).unwrap()
    }

    #[test]
    fn a_top_is_the_countries_greatest_at_the_hour_and_every_column_is_read_for_them() {
        let hour = ITA_FROM;
        let fixture = Fixture::new(Data::Power, hour - 1..=hour + 1);
        let query = fixture.query();
        let columns = [power_column(&query), Column { line: line(&query, "commandPower"), partial: Some(line(&query, "politicalPower")) }];
        assert_eq!(columns[0].line.rank_at(query.timeline.first, 2, false).unwrap().unwrap(), [crate::identity::Country::from("GER")]);
        let board = at(&columns, Rows::Top { by: 0, n: 2, least: false }, hour).unwrap();
        let mut wanted: Vec<(i64, u32)> = [9001, 9002].into_iter().map(|id| (power(id, hour).unwrap(), id)).collect();
        wanted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let tags: Vec<&str> = board.rows.iter().map(|row| row.tag.as_str()).collect();
        assert_eq!(tags, wanted.iter().map(|(_, id)| tag_of(*id)).collect::<Vec<_>>());
        for (row, (value, id)) in board.rows.iter().zip(&wanted) {
            assert_ne!(power(*id, hour - 1), Some(*value));
            assert_ne!(power(*id, hour + 1), Some(*value));
            assert_eq!(row.cells[0], Cell { value_raw: Some(value.to_string()), missing: None, partial: false });
            assert_eq!(row.cells[1], Cell { value_raw: Some("0".into()), missing: None, partial: *value != 0 });
        }
        assert_eq!(at(&columns, Rows::Top { by: 0, n: 1, least: false }, hour).unwrap().rows.len(), 1);
        assert!(at(&columns, Rows::Top { by: 2, n: 1, least: false }, hour).is_err());
        let least = at(&columns, Rows::Top { by: 0, n: 1, least: true }, hour).unwrap();
        assert_eq!(least.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), [tag_of(wanted[1].1)], "the least first");
    }

    #[test]
    fn countries_of_equal_value_rank_by_tag_as_the_followed_top_and_the_ui_do() {
        let hour = both_recorded();
        let fixture = Fixture::new(Data::Power, hour..=hour);
        let mut query = fixture.query();
        let held = std::sync::Arc::make_mut(&mut query.holders);
        let ger = held.tags.remove("GER").unwrap();
        let ita = held.tags.remove("ITA").unwrap();
        held.tags.insert("ZZZ".into(), ger);
        held.tags.insert("AAA".into(), ita);
        let columns = [Column { line: line(&query, "commandPower"), partial: None }];
        let board = at(&columns, Rows::Top { by: 0, n: 2, least: false }, hour).unwrap();
        assert_eq!(board.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), ["AAA", "ZZZ"]);
        assert_eq!(at(&columns, Rows::Top { by: 0, n: 1, least: false }, hour).unwrap().rows[0].tag, "AAA", "the tie at the last place goes by tag too");
    }

    #[test]
    fn chosen_tags_keep_their_order_leave_out_a_country_not_held_and_never_rank_an_unknown_value() {
        let unrecorded = (FIRST..ITA_FROM).rev().find(|&hour| power(9001, hour).is_none()).unwrap();
        let hour = both_recorded();
        let fixture = Fixture::new(Data::Power, unrecorded..=hour);
        let query = fixture.query();
        let columns = [power_column(&query)];
        let tags = [Country::from("ITA"), Country::from("GER")];
        let board = at(&columns, Rows::Countries(&tags), unrecorded).unwrap();
        assert_eq!(board.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), ["GER"], "ITA does not exist yet");
        assert_eq!(board.rows[0].cells[0].value_raw, None);
        assert!(board.rows[0].cells[0].missing.is_some());
        assert!(at(&columns, Rows::Top { by: 0, n: 2, least: false }, unrecorded).unwrap().rows.is_empty());
        let board = at(&columns, Rows::Countries(&tags), hour).unwrap();
        assert_eq!(board.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), ["ITA", "GER"]);
    }

    #[test]
    fn a_holder_with_no_row_ranks_at_zero_where_its_series_holds_none_there() {
        use crate::identity::{Holder, HolderRun};
        let hour = both_recorded();
        let fixture = Fixture::new(Data::Power, hour..=hour);
        let mut query = fixture.query();
        let held = std::sync::Arc::make_mut(&mut query.holders);
        held.tags.insert("POL".into(), vec![HolderRun { first: hour, last: hour, holder: Holder::Found { slot: 9, id: 9003 }, starts_segment: true }]);
        let base = dashboard_declare::series("country-measure").unwrap();
        let series: &'static dashboard_declare::Series = Box::leak(Box::new(dashboard_declare::Series {
            kind: base.kind, table: base.table, key: base.key, parts: base.parts, measures: base.measures, drops: base.drops, absent_none: true,
        }));
        let columns = [Column { line: Line::new(&query, series, series.measure("politicalPower").unwrap()).unwrap(), partial: None }];
        let mut wanted: Vec<(i64, &str)> = vec![(power(9001, hour).unwrap(), "GER"), (power(9002, hour).unwrap(), "ITA"), (0, "POL")];
        wanted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        let most = at(&columns, Rows::Top { by: 0, n: 3, least: false }, hour).unwrap();
        assert_eq!(most.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), wanted.iter().map(|(_, tag)| *tag).collect::<Vec<_>>());
        wanted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(b.1)));
        let least = at(&columns, Rows::Top { by: 0, n: 3, least: true }, hour).unwrap();
        assert_eq!(least.rows.iter().map(|row| row.tag.as_str()).collect::<Vec<_>>(), wanted.iter().map(|(_, tag)| *tag).collect::<Vec<_>>());
        let pol = least.rows.iter().find(|row| row.tag == "POL").unwrap();
        assert_eq!(pol.cells[0].value_raw.as_deref(), Some("0"));
        assert_eq!(at(&[power_column(&query)], Rows::Top { by: 0, n: 3, least: true }, hour).unwrap().rows.len(), 2);
    }

    #[test]
    fn an_hour_the_timeline_does_not_hold_has_no_rows_and_says_why() {
        let end = FIRST + FIRST_HOURS - 1;
        let fixture = Fixture::with_gap(Data::Power, end..=SECOND, end + 1..=SECOND - 1);
        let query = fixture.query();
        let columns = [power_column(&query)];
        let board = at(&columns, Rows::Top { by: 0, n: 2, least: false }, FIRST + FIRST_HOURS + 1).unwrap();
        assert!(board.rows.is_empty());
        assert_eq!(board.missing.as_deref(), Some(GAP_REASON));
    }

    #[test]
    fn top_ranking_with_unrecorded_ranking_table_returns_shared_missing_reason() {
        let hour = both_recorded();
        let factories = dashboard_declare::series("country-factories").unwrap();
        let free1 = factories.measure("free1").unwrap();

        let unavailable = Fixture::new(Data::Power, hour..=hour);
        let unavailable_query = unavailable.query();
        let unavailable_col = Column { line: Line::new(&unavailable_query, factories, free1).unwrap(), partial: None };
        let unavailable_board = at(&[unavailable_col], Rows::Top { by: 0, n: 2, least: false }, hour).unwrap();
        assert!(unavailable_board.rows.is_empty());
        assert_eq!(
            unavailable_board.missing.as_deref(),
            Some("Country factories not available at this hour: outside the fixture"),
        );
        assert_eq!(
            unavailable_board.missing,
            table_unrecorded(factories.table, Status::Unavailable { reason: 0 }, Some("outside the fixture")),
        );

        let unrecorded = Fixture::with_not_recorded_factories(hour..=hour);
        let unrecorded_query = unrecorded.query();
        let unrecorded_col = Column { line: Line::new(&unrecorded_query, factories, free1).unwrap(), partial: None };
        let unrecorded_board = at(&[unrecorded_col], Rows::Top { by: 0, n: 2, least: false }, hour).unwrap();
        assert!(unrecorded_board.rows.is_empty());
        assert_eq!(
            unrecorded_board.missing.as_deref(),
            Some("Country factories not recorded at this hour"),
        );
        assert_eq!(
            unrecorded_board.missing,
            table_unrecorded(factories.table, Status::NotRecorded, None),
        );
    }
}
