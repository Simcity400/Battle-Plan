
use super::{Line, invalid};
use crate::{
    identity::{Country, Holder},
    read,
};
use replay_lanes::lanes::Keys;
use replay_columnar::manifest::Status;
use std::{collections::HashMap, io};

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
    let Some(first) = columns.first() else {
        return Err(invalid("A board names no column"));
    };
    let query = first.line.query();
    let Some(stretch) = query.timeline.at(hour).copied() else {
        return Ok(Board { rows: Vec::new(), missing: Some(query.timeline.reason(hour)) });
    };
    let countries: Vec<Country> = match rows {
        Rows::Countries(countries) => countries.iter().filter(|country| query.holders.at(*country, hour).is_some()).cloned().collect(),
        Rows::Top { by, n, least } => {
            columns.get(by).ok_or_else(|| invalid("A board ranks by a column it does not hold"))?.line.rank_at_by(hour, n, least)?.into_iter().map(Country::from).collect()
        }
    };
    let lines: Vec<&Line<'_, '_>> = columns.iter().flat_map(|column| std::iter::once(&column.line).chain(column.partial.as_ref())).collect();
    let mut values = values(&lines, &countries, hour, stretch.run)?.into_iter();
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

fn values(lines: &[&Line<'_, '_>], countries: &[Country], hour: i32, run: u32) -> io::Result<Vec<Vec<Result<i64, String>>>> {
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
    let mut out: Vec<Option<Vec<Result<i64, String>>>> = vec![None; lines.len()];
    for at in 0..lines.len() {
        if out[at].is_some() {
            continue;
        }
        let table = lines[at].series().table;
        let group: Vec<usize> = (at..lines.len()).filter(|&other| out[other].is_none() && lines[other].series().table == table).collect();
        let mut asked: Vec<usize> = group.iter().flat_map(|&line| read::columns_of(&lines[line].source())).collect();
        asked.sort_unstable();
        asked.dedup();
        let mut wanted: Vec<u128> = group.iter().flat_map(|&line| keys[line].iter().filter_map(|key| key.as_ref().ok().copied())).collect();
        wanted.sort_unstable();
        wanted.dedup();
        let snapshot = match wanted.is_empty() {
            true => None,
            false => Some(crate::cost::storage(|| replay_lanes::lanes::at_keys(recording, run, table, hour, Some(&asked), Keys::These(&wanted)))?),
        };
        let unrecorded = snapshot.as_ref().and_then(|snapshot| {
            let reason = match snapshot.status {
                Status::Unavailable { reason } => snapshot.strings.get(reason).and_then(|text| std::str::from_utf8(text).ok()),
                _ => None,
            };
            table_unrecorded(table, snapshot.status, reason)
        });
        let view = snapshot.as_ref().map(|snapshot| snapshot.view());
        let slots: HashMap<u128, u32> = view.iter().flat_map(|view| view.rows().filter_map(move |slot| Some((view.key(slot)?, slot)))).collect();
        for &line in &group {
            let measure = lines[line].measure_name();
            out[line] = Some(
                keys[line]
                    .iter()
                    .zip(countries)
                    .map(|(key, country)| {
                        let key = key.clone()?;
                        let tag = query.holders.name(country, hour);
                        if let Some(reason) = &unrecorded {
                            return Err(reason.clone());
                        }
                        let Some(slot) = slots.get(&key) else {
                            return if lines[line].series().absent_none { Ok(0) } else { Err(measure_unrecorded(&tag, measure)) };
                        };
                        let view = view.as_ref().expect("a complete read");
                        lines[line].source().value(view, *slot).ok_or_else(|| measure_unrecorded(&tag, measure))
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
        assert_eq!(columns[0].line.rank_at_by(query.timeline.first, 2, false).unwrap(), ["GER"]);
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
}
