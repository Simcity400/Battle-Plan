//! A daily window folded into at most `width` columns of whole days, so a
//! reply grows with the viewport rather than the recording.

use super::{content, split_name};
use dashboard_keyed::{
    daily::DAY_HOURS,
    query::{KeyedRun, KeyedWindow},
};
use serde::Serialize;
use std::{collections::BTreeMap, io};
use ts_rs::TS;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RegionalWeatherColumns")]
pub struct Columns {
    /// Days per column, counted from the window's first day.
    pub days: u32,
    pub columns: Vec<Column>,
    /// Each region with a sampled measure in the window.
    pub regions: Vec<ColumnRegion>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RegionalWeatherColumn")]
pub struct Column {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The latest recorded hour its days were sampled at; none without one.
    pub sampled_hour: Option<i32>,
    /// The latest supplying read of a sampled day, where one is recorded.
    pub read_hour: Option<i32>,
    /// Its recorded days.
    pub recorded: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RegionalWeatherColumnRegion")]
pub struct ColumnRegion {
    pub id: String,
    /// Conditions: the days the region's conditions were recorded, per column.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub observed: Option<Vec<u32>>,
    /// Conditions: those days with no condition set, per column.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub clear: Option<Vec<u32>>,
    /// A flag set on no recorded day, or a value on none, is left out.
    pub measures: Vec<ColumnMeasure>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "RegionalWeatherColumnMeasure")]
pub struct ColumnMeasure {
    /// `<flag>`, `mod:<modifier>` or `temp`, as in the window's series names.
    pub id: String,
    /// Recorded days with the flag set, or with a value, per column.
    pub days: Vec<u32>,
    /// Values: each column's lowest raw value, the exact sum of its days' raw
    /// values and its highest, as decimal text; none without a value.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub values: Option<Vec<Option<[String; 3]>>>,
}

/// One requested day: its hours in the window, and when sampled, the last
/// recorded hour within it and that hour's supplying read, if recorded.
struct Day {
    first: i32,
    last: i32,
    at: Option<i32>,
    read: Option<i32>,
}

fn days(window: &KeyedWindow) -> Vec<Day> {
    let (first, last) = (window.first_hour, window.last_hour);
    if first > last {
        return Vec::new();
    }
    let (mut known, mut reads) = (0, 0);
    (first.div_euclid(DAY_HOURS)..=last.div_euclid(DAY_HOURS))
        .map(|day| {
            let start = i64::from(day) * i64::from(DAY_HOURS);
            let from = start.max(i64::from(first)) as i32;
            let to = (start + i64::from(DAY_HOURS) - 1).min(i64::from(last)) as i32;
            // A prefix can sample only the start of its last day.
            while window
                .known
                .get(known)
                .is_some_and(|span| span.last_hour < from)
            {
                known += 1;
            }
            let at = window.known[known..]
                .iter()
                .take_while(|span| span.first_hour <= to)
                .last()
                .map(|span| span.last_hour.min(to));
            let read = at.and_then(|at| {
                while window.reads.get(reads).is_some_and(|read| read.1 < at) {
                    reads += 1;
                }
                window
                    .reads
                    .get(reads)
                    .filter(|read| read.0 <= at)
                    .map(|read| read.2)
            });
            Day {
                first: from,
                last: to,
                at,
                read,
            }
        })
        .collect()
}

/// Each sampled day's value of `runs`, at the hour the day was sampled.
fn values(runs: &[KeyedRun], days: &[Day]) -> io::Result<Vec<Option<i128>>> {
    let mut next = 0;
    days.iter()
        .map(|day| {
            let Some(at) = day.at else {
                return Ok(None);
            };
            while runs.get(next).is_some_and(|run| run.1 < at) {
                next += 1;
            }
            match runs.get(next) {
                Some(run) if run.0 <= at => run
                    .2
                    .parse()
                    .map(Some)
                    .map_err(|_| dashboard_core::invalid("Invalid weather value")),
                _ => Ok(None),
            }
        })
        .collect()
}

#[derive(Default)]
struct Region {
    /// Per day: the region's conditions were recorded, and one was set.
    observed: Vec<bool>,
    set: Vec<bool>,
    measures: Vec<ColumnMeasure>,
}

pub fn fold(window: &KeyedWindow, width: u32, conditions: bool) -> io::Result<Columns> {
    let days = days(window);
    let size = days.len().div_ceil(width.max(1) as usize).max(1);
    let columns = days
        .chunks(size)
        .map(|chunk| Column {
            first_hour: chunk[0].first,
            last_hour: chunk[chunk.len() - 1].last,
            sampled_hour: chunk.iter().filter_map(|day| day.at).max(),
            read_hour: chunk.iter().filter_map(|day| day.read).max(),
            recorded: chunk.iter().filter(|day| day.at.is_some()).count() as u32,
        })
        .collect();
    let mut regions = BTreeMap::<u32, Region>::new();
    for series in &window.series {
        let Some((region, measure)) = split_name(&series.name) else {
            continue;
        };
        let daily = values(&series.runs, &days)?;
        let region = regions.entry(region).or_insert_with(|| Region {
            observed: vec![false; days.len()],
            set: vec![false; days.len()],
            measures: Vec::new(),
        });
        let flag = content::FLAGS.iter().any(|(id, _, _)| *id == measure);
        let chunks = daily.chunks(size);
        let measure = if flag {
            for (day, value) in daily.iter().enumerate() {
                region.observed[day] |= value.is_some();
                region.set[day] |= value.is_some_and(|value| value != 0);
            }
            ColumnMeasure {
                id: measure.to_owned(),
                days: chunks
                    .map(|chunk| {
                        chunk
                            .iter()
                            .filter(|value| value.is_some_and(|value| value != 0))
                            .count() as u32
                    })
                    .collect(),
                values: None,
            }
        } else {
            let mut days = Vec::new();
            let stats = chunks
                .map(|chunk| {
                    let held: Vec<i128> = chunk.iter().flatten().copied().collect();
                    days.push(held.len() as u32);
                    let (Some(low), Some(high)) = (held.iter().min(), held.iter().max()) else {
                        return Ok(None);
                    };
                    let sum = held
                        .iter()
                        .try_fold(0_i128, |sum, value| sum.checked_add(*value))
                        .ok_or_else(|| {
                            dashboard_core::invalid("Weather column sum exceeds its exact range")
                        })?;
                    Ok(Some([low.to_string(), sum.to_string(), high.to_string()]))
                })
                .collect::<io::Result<_>>()?;
            ColumnMeasure {
                id: measure.to_owned(),
                days,
                values: Some(stats),
            }
        };
        if measure.days.iter().any(|days| *days > 0) {
            region.measures.push(measure);
        }
    }
    let per_column = |flags: &[bool], keep: &dyn Fn(usize) -> bool| -> Vec<u32> {
        flags
            .chunks(size)
            .enumerate()
            .map(|(column, chunk)| {
                (0..chunk.len())
                    .filter(|offset| keep(column * size + offset))
                    .count() as u32
            })
            .collect()
    };
    let regions = regions
        .into_iter()
        .map(|(id, region)| ColumnRegion {
            id: id.to_string(),
            observed: conditions.then(|| per_column(&region.observed, &|day| region.observed[day])),
            // Only a day the region's conditions were read can be clear.
            clear: conditions.then(|| {
                per_column(&region.observed, &|day| {
                    region.observed[day] && !region.set[day]
                })
            }),
            measures: region.measures,
        })
        .collect();
    Ok(Columns {
        days: size as u32,
        columns,
        regions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::country_groups::HourSpan;
    use dashboard_keyed::query::KeyedSeries;

    fn day(index: i32) -> (i32, i32) {
        (index * DAY_HOURS, index * DAY_HOURS + DAY_HOURS - 1)
    }
    fn run(first: i32, last: i32, value: &str) -> KeyedRun {
        (day(first).0, day(last).1, value.to_owned(), false)
    }
    fn span(first_hour: i32, last_hour: i32) -> HourSpan {
        HourSpan {
            first_hour,
            last_hour,
        }
    }

    fn window() -> KeyedWindow {
        KeyedWindow {
            first_hour: day(0).0,
            last_hour: day(4).1,
            // The fourth day was not recorded.
            known: vec![span(day(0).0, day(2).1), span(day(4).0, day(4).1)],
            reads: (0..5)
                .filter(|day| *day != 3)
                .map(|index| (day(index).0, day(index).1, day(index).1 - 3))
                .collect(),
            series: vec![
                KeyedSeries {
                    name: "7|light_rain".into(),
                    runs: vec![run(0, 1, "1"), run(2, 4, "0")],
                },
                KeyedSeries {
                    name: "7|snow".into(),
                    runs: vec![run(1, 2, "1"), run(4, 4, "1")],
                },
                KeyedSeries {
                    name: "7|sandstorm".into(),
                    runs: vec![run(0, 4, "0")],
                },
                KeyedSeries {
                    name: "8|temp".into(),
                    runs: vec![
                        run(0, 0, "-250000"),
                        run(1, 1, "100000"),
                        run(4, 4, "300000"),
                    ],
                },
            ],
        }
    }

    fn texts(values: [&str; 3]) -> Option<[String; 3]> {
        Some(values.map(str::to_owned))
    }

    #[test]
    fn columns_count_flag_days_and_summarise_values_over_recorded_days() {
        let folded = fold(&window(), 2, true).unwrap();
        assert_eq!(folded.days, 3);
        assert_eq!(
            folded
                .columns
                .iter()
                .map(|column| (
                    column.first_hour,
                    column.last_hour,
                    column.sampled_hour,
                    column.read_hour,
                    column.recorded
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, day(2).1, Some(day(2).1), Some(day(2).1 - 3), 3),
                (day(3).0, day(4).1, Some(day(4).1), Some(day(4).1 - 3), 1)
            ]
        );
        let rain = &folded.regions[0];
        assert_eq!(rain.id, "7");
        assert_eq!(rain.observed, Some(vec![3, 1]));
        assert_eq!(
            rain.clear,
            Some(vec![0, 0]),
            "every recorded day had rain or snow"
        );
        assert_eq!(
            rain.measures
                .iter()
                .map(|measure| (measure.id.as_str(), measure.days.clone()))
                .collect::<Vec<_>>(),
            vec![("light_rain", vec![2, 0]), ("snow", vec![2, 1])],
            "a flag set on no recorded day is left out"
        );
        let temperature = &folded.regions[1].measures[0];
        assert_eq!(temperature.days, vec![2, 1]);
        assert_eq!(
            temperature.values,
            Some(vec![
                texts(["-250000", "-150000", "100000"]),
                texts(["300000"; 3])
            ])
        );
    }

    #[test]
    fn a_window_no_longer_than_the_width_keeps_one_day_per_column() {
        let folded = fold(&window(), 5, true).unwrap();
        assert_eq!(folded.days, 1);
        assert_eq!(
            folded
                .columns
                .iter()
                .map(|column| column.recorded)
                .collect::<Vec<_>>(),
            vec![1, 1, 1, 0, 1]
        );
        assert_eq!(folded.columns[3].sampled_hour, None);
        assert_eq!(folded.columns[3].read_hour, None);
        assert_eq!(folded.regions[0].clear, Some(vec![0, 0, 0, 0, 0]));
        assert_eq!(folded.regions[0].measures[1].days, vec![0, 1, 1, 0, 1]);
        assert_eq!(
            folded.regions[1].measures[0].values,
            Some(vec![
                texts(["-250000"; 3]),
                texts(["100000"; 3]),
                None,
                None,
                texts(["300000"; 3])
            ])
        );
    }

    #[test]
    fn only_days_a_region_reads_its_conditions_are_clear() {
        let mut window = window();
        // Region 9 reads its conditions on days 0 and 4 only, and region 8 only
        // its temperature.
        window.series.push(KeyedSeries {
            name: "9|light_rain".into(),
            runs: vec![run(0, 0, "0"), run(4, 4, "0")],
        });
        let folded = fold(&window, 5, true).unwrap();
        let temperature_only = &folded.regions[1];
        assert_eq!(temperature_only.observed, Some(vec![0; 5]));
        assert_eq!(
            temperature_only.clear,
            Some(vec![0; 5]),
            "a temperature reading is no clear day"
        );
        let missing = &folded.regions[2];
        assert_eq!(missing.observed, Some(vec![1, 0, 0, 0, 1]));
        assert_eq!(
            missing.clear,
            Some(vec![1, 0, 0, 0, 1]),
            "an absent day is not clear"
        );
    }

    #[test]
    fn a_prefix_keeps_the_sampled_start_of_its_last_day() {
        // Asked through hour 71, prepared through hour 30: day 1 holds hours 24 to 30.
        let window = KeyedWindow {
            first_hour: 0,
            last_hour: 71,
            known: vec![span(0, 30)],
            reads: vec![(0, 23, 20), (24, 30, 30)],
            series: vec![KeyedSeries {
                name: "7|snow".into(),
                runs: vec![(0, 23, "0".into(), true), (24, 30, "1".into(), false)],
            }],
        };
        let folded = fold(&window, 3, true).unwrap();
        assert_eq!(
            folded
                .columns
                .iter()
                .map(|column| (
                    column.last_hour,
                    column.sampled_hour,
                    column.read_hour,
                    column.recorded
                ))
                .collect::<Vec<_>>(),
            vec![
                (23, Some(23), Some(20), 1),
                (47, Some(30), Some(30), 1),
                (71, None, None, 0)
            ]
        );
        assert_eq!(folded.regions[0].measures[0].days, vec![0, 1, 0]);
    }

    #[test]
    fn a_sampled_day_without_a_recorded_read_has_no_read_hour() {
        let mut window = window();
        window.reads.retain(|read| read.0 != day(1).0);
        let folded = fold(&window, 5, true).unwrap();
        assert_eq!(folded.columns[1].recorded, 1);
        assert_eq!(folded.columns[1].sampled_hour, Some(day(1).1));
        assert_eq!(
            folded.columns[1].read_hour, None,
            "never the calendar endpoint"
        );
    }

    #[test]
    fn values_aggregate_exactly_beyond_a_double() {
        let window = KeyedWindow {
            first_hour: 0,
            last_hour: 47,
            known: vec![span(0, 47)],
            reads: vec![(0, 47, 47)],
            series: vec![KeyedSeries {
                name: "7|mod:air_detection".into(),
                runs: vec![
                    (0, 23, "9007199254740993".into(), true),
                    (24, 47, "-9007199254740992".into(), false),
                ],
            }],
        };
        let folded = fold(&window, 1, false).unwrap();
        assert_eq!(
            folded.regions[0].measures[0].values,
            Some(vec![texts(["-9007199254740992", "1", "9007199254740993"])]),
            "the sum stays exact, so the mean is 0.5"
        );
        assert_eq!(folded.regions[0].observed, None);
    }
}
