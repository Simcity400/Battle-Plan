//! One channel of naval losses as the naval family's series answers it ([`super::emit`]), by
//! the keys the losses name: its values, ranks, corrections and reads.

use super::{
    Correction, Delta,
    emit::{CORRECTIONS, FAMILY, RANKS, READS, VALUES, kind},
};
use dashboard_base::{
    prepared::{engine, series},
    prepared_chart::{ChartSummary, merge},
    products::country_groups::HourSpan,
};
use dashboard_series::{
    Cell, EventOrder, EventQuery, PageRead, Selection, Snapshot, Span, family::Directory,
};
use std::io;

/// Rows one event page answers at most.
const BUDGET: usize = 1 << 20;

pub(super) struct Lines<'s, 'a, P> {
    snapshot: &'s mut Snapshot<'a, P>,
    directory: &'s Directory,
    channel: u64,
}

/// A run of a key, clipped to the hours asked.
#[derive(Clone, Copy)]
pub(super) struct Clipped {
    pub first_hour: i32,
    pub last_hour: i32,
}

impl<'s, 'a, P: PageRead> Lines<'s, 'a, P> {
    pub(super) fn new(
        snapshot: &'s mut Snapshot<'a, P>,
        directory: &'s Directory,
        channel: usize,
    ) -> Self {
        Self {
            snapshot,
            directory,
            channel: channel as u64,
        }
    }

    fn key(&self, measure: u64, dimension: u64) -> Option<u64> {
        self.directory.key(FAMILY, measure, Some(dimension))
    }

    /// The value the channel's `key` holds at `hour`; None where it holds none.
    pub(super) fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Option<i128>> {
        let Some(key) = self.key(VALUES + self.channel, key) else {
            return Ok(None);
        };
        Ok(engine::run_at(self.snapshot, key, hour)?.map(|(run, _)| run.value))
    }

    /// The channel's `key`'s runs over `first..=last`, clipped to it.
    pub(super) fn runs(&mut self, key: u64, first: i32, last: i32) -> io::Result<Vec<Clipped>> {
        self.measure_runs(VALUES + self.channel, key, first, last)
    }

    /// The channel's rank `key`'s runs over `first..=last`, with their values.
    pub(super) fn rank_runs(
        &mut self,
        key: u64,
        first: i32,
        last: i32,
    ) -> io::Result<Vec<(Clipped, i128)>> {
        let Some(key) = self.key(RANKS + self.channel, key) else {
            return Ok(Vec::new());
        };
        Ok(engine::runs(self.snapshot, key, first, last)?
            .into_iter()
            .map(|run| {
                (
                    Clipped {
                        first_hour: run.span.first,
                        last_hour: run.span.last,
                    },
                    run.value,
                )
            })
            .collect())
    }

    fn measure_runs(
        &mut self,
        measure: u64,
        key: u64,
        first: i32,
        last: i32,
    ) -> io::Result<Vec<Clipped>> {
        let Some(key) = self.key(measure, key) else {
            return Ok(Vec::new());
        };
        Ok(engine::runs(self.snapshot, key, first, last)?
            .into_iter()
            .map(|run| Clipped {
                first_hour: run.span.first,
                last_hour: run.span.last,
            })
            .collect())
    }

    /// The chart of `handle` (or every bin unknown where the family never wrote it) over
    /// `view`, its bins laid from `bounds`, within `within`.
    fn chart(
        &mut self,
        handle: Option<u64>,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        within: Option<&[HourSpan]>,
    ) -> io::Result<ChartSummary> {
        if let Some(key) = handle.filter(|_| bounds.0 <= bounds.1) {
            return engine::summary(self.snapshot, key, bounds, view, decimals, within);
        }
        let (exact, bins) = series::summary_bins(bounds, view)?;
        Ok(ChartSummary {
            from: view.0,
            to: view.1,
            width: view.2,
            buckets: vec![None; bins.len()],
            runs: exact.then(Vec::new),
            approximate: Vec::new(),
        })
    }

    /// The chart of the channel's `key` over `view`.
    pub(super) fn summary(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        let handle = self.key(VALUES + self.channel, key);
        self.chart(handle, bounds, view, decimals, None)
    }

    /// The chart of the channel's rank `key` over `view`.
    pub(super) fn rank_summary(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        let handle = self.key(RANKS + self.channel, key);
        self.chart(handle, bounds, view, decimals, None)
    }

    /// The charts of each key over its own hours, merged into one: a known boundary flag marks
    /// whether each key's first hour starts a segment.
    pub(super) fn summary_spans(
        &mut self,
        spans: &[(u64, i32, i32, Option<bool>)],
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        series::viewport(Some(view.0), Some(view.1), Some(view.2))?;
        if spans.iter().any(|span| span.1 > span.2)
            || spans.windows(2).any(|pair| pair[0].2 >= pair[1].1)
        {
            return Err(dashboard_core::invalid("Invalid dated series spans"));
        }
        let mut result: Option<ChartSummary> = None;
        for &(key, first, last, boundary) in spans {
            if f64::from(last) < view.0.floor() || f64::from(first) > view.1.floor() {
                continue;
            }
            let within = [HourSpan {
                first_hour: first,
                last_hour: last,
            }];
            let handle = self.key(VALUES + self.channel, key);
            let mut part = self.chart(handle, bounds, view, decimals, Some(&within))?;
            if let Some(starts) = boundary {
                for bucket in part.buckets.iter_mut().flatten() {
                    if bucket.first_hour == first {
                        bucket.first_break = starts;
                    }
                }
                for run in part.runs.iter_mut().flatten() {
                    if run.first_hour == first {
                        run.break_before = starts;
                    }
                }
            }
            if let Some(result) = &mut result {
                for (bucket, next) in result.buckets.iter_mut().zip(part.buckets) {
                    *bucket = match (*bucket, next) {
                        (Some(_), Some(_)) => merge(*bucket, next),
                        (previous, next) => previous.or(next),
                    };
                }
                if let (Some(runs), Some(next)) = (&mut result.runs, part.runs) {
                    runs.extend(next);
                }
            } else {
                result = Some(part);
            }
        }
        Ok(result.unwrap_or(ChartSummary {
            from: view.0,
            to: view.1,
            width: view.2,
            buckets: Vec::new(),
            runs: Some(Vec::new()),
            approximate: Vec::new(),
        }))
    }

    /// The hours the channel's rank `key` holds a rank within `top` over `view`.
    pub(super) fn rank_top_spans(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        top: usize,
    ) -> io::Result<Vec<HourSpan>> {
        let Some(key) = self.key(RANKS + self.channel, key) else {
            return Ok(Vec::new());
        };
        let first = (view.0.floor() as i32).max(bounds.0);
        let last = (view.1.floor() as i32).min(bounds.1);
        let mut spans: Vec<HourSpan> = Vec::new();
        if first > last {
            return Ok(spans);
        }
        self.snapshot.top_spans(
            Selection {
                key,
                membership: None,
            },
            top as u64,
            Span::new(first, last)?,
            |span| {
                spans.push(HourSpan {
                    first_hour: span.first,
                    last_hour: span.last,
                });
                Ok(())
            },
        )?;
        Ok(spans)
    }

    /// The first hour after `navigation`'s origin (or before it) at which one of `values` or
    /// `ranks` changes, each value key within its membership.
    pub(super) fn navigate(
        &mut self,
        values: &[(u64, Option<&[HourSpan]>)],
        ranks: &[(u64, Option<&[HourSpan]>)],
        navigation: &series::Navigation,
    ) -> io::Result<Option<f64>> {
        let mut keys = Vec::new();
        for (measure, held) in [
            (VALUES + self.channel, values),
            (RANKS + self.channel, ranks),
        ] {
            for &(key, within) in held {
                if let Some(handle) = self.key(measure, key) {
                    keys.push((handle, within));
                }
            }
        }
        engine::navigate(self.snapshot, &keys, navigation)
    }

    /// The corrections of the channel's holder `holder`, in the order they were made.
    pub(super) fn corrections(&mut self, holder: u64) -> io::Result<Vec<Correction>> {
        let Some(key) = self.key(CORRECTIONS, holder * 12 + self.channel) else {
            return Ok(Vec::new());
        };
        let mut fields = Vec::new();
        self.events(key, i32::MIN, i32::MAX, |event| {
            if event.kind != kind::CORRECTION {
                return Err(dashboard_core::invalid(
                    "A naval loss correction has another kind",
                ));
            }
            fields.push(event.fields);
            Ok(())
        })?;
        let integer = |cell: Cell| match cell {
            Cell::Integer(value) => i64::try_from(value)
                .map_err(|_| dashboard_core::invalid("A naval loss correction overflows")),
            _ => Err(dashboard_core::invalid(
                "A naval loss correction is not a number",
            )),
        };
        fields
            .into_iter()
            .map(|blob| {
                let mut cells = [Cell::Null; 6];
                for (index, cell) in cells.iter_mut().enumerate() {
                    *cell = self.snapshot.field(blob, index as u64)?;
                }
                Ok(Correction {
                    read_hour: i32::try_from(integer(cells[0])?)
                        .map_err(|_| dashboard_core::invalid("A naval loss correction hour"))?,
                    month: match cells[1] {
                        Cell::Null => None,
                        cell => Some(i32::try_from(integer(cell)?).map_err(|_| {
                            dashboard_core::invalid("A naval loss correction month")
                        })?),
                    },
                    delta: Delta {
                        ships: integer(cells[2])?,
                        convoys: integer(cells[3])?,
                        ship_unknown: integer(cells[4])?,
                        convoy_unknown: integer(cells[5])?,
                    },
                })
            })
            .collect()
    }

    /// The hours of the reads of `table` (0 for ships, 1 for convoys) over `first..=last`.
    pub(super) fn reads(&mut self, table: u64, first: i32, last: i32) -> io::Result<Vec<i32>> {
        let Some(key) = self.key(READS, table) else {
            return Ok(Vec::new());
        };
        let mut hours = Vec::new();
        self.events(key, first, last, |event| {
            hours.push(event.native_hour);
            Ok(())
        })?;
        Ok(hours)
    }

    /// The hour of the last read of `table` by `hour`.
    pub(super) fn latest_read(&mut self, table: u64, hour: i32) -> io::Result<Option<i32>> {
        let Some(key) = self.key(READS, table) else {
            return Ok(None);
        };
        Ok(self
            .snapshot
            .latest_event(
                Selection {
                    key,
                    membership: None,
                },
                hour,
                None,
            )?
            .map(|event| event.native_hour()))
    }

    /// Each event of `key` dated `first..=last`, in native order.
    fn events(
        &mut self,
        key: u64,
        first: i32,
        last: i32,
        mut sink: impl FnMut(dashboard_series::Event) -> io::Result<()>,
    ) -> io::Result<()> {
        let accepted = self.snapshot.publication().accepted;
        let (first, last) = (first.max(accepted.first), last.min(accepted.last));
        if first > last {
            return Ok(());
        }
        let native = Span::new(first, last)?;
        let selection = [Selection {
            key,
            membership: None,
        }];
        let mut cursor = None;
        loop {
            let page = self.snapshot.event_page(
                EventQuery {
                    selection: &selection,
                    native,
                    as_of: None,
                    order: EventOrder::Native,
                },
                cursor,
                BUDGET,
                &mut |view| match view {
                    dashboard_series::query::EventView::Family(event) => sink(event),
                    dashboard_series::query::EventView::Evidence { .. } => Ok(()),
                },
            )?;
            cursor = page.next;
            if cursor.is_none() {
                return Ok(());
            }
        }
    }
}
