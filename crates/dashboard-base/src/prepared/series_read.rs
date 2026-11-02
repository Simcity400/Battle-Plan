//! One reading of numeric series for replies that join families: the prepared series reader
//! ([`super::series::Reader`]) and a family's series-engine publication ([`dashboard_series`]),
//! each key named by the reader's own number. The engine answers as the reader did on the same
//! streams (`series::engine_parity`).

use super::{
    engine,
    series::{self, Navigation, NavigationKey, Reader, Run},
};
use crate::{prepared_chart::ChartSummary, products::country_groups::HourSpan};
use dashboard_series::{KeyHandle, PageRead, Snapshot, Viewport, query::Alias};
use std::{cell::RefCell, io};

/// Numeric series by key: runs, values at an hour, charts and keyboard navigation.
pub trait SeriesRead {
    /// `key`'s runs over `from..=to`, clipped to it.
    fn runs(&mut self, key: u64, from: i32, to: i32) -> io::Result<Vec<Run>>;

    /// The run of `key` holding `hour`, whole.
    fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>>;

    /// `key`'s chart over `view`, its bins laid from `bounds`, within `within`.
    fn summary_within(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        within: Option<&[HourSpan]>,
    ) -> io::Result<ChartSummary>;

    /// One chart of dated keys, each over its own hours, a known boundary starting or
    /// continuing its lane.
    fn summary_spans(
        &mut self,
        spans: &[(u64, i32, i32, Option<bool>)],
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary>;

    /// The next hour any of `keys` changes, each within its membership.
    fn navigate(
        &mut self,
        keys: &[NavigationKey<'_>],
        navigation: &Navigation,
    ) -> io::Result<Option<f64>>;

    fn summary(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        self.summary_within(key, bounds, view, decimals, None)
    }
}

impl SeriesRead for Reader<'_> {
    fn runs(&mut self, key: u64, from: i32, to: i32) -> io::Result<Vec<Run>> {
        Reader::runs(self, key, from, to)
    }

    fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>> {
        Reader::value_at(self, key, hour)
    }

    fn summary_within(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        within: Option<&[HourSpan]>,
    ) -> io::Result<ChartSummary> {
        Reader::summary_within(self, key, bounds, view, decimals, within)
    }

    fn summary_spans(
        &mut self,
        spans: &[(u64, i32, i32, Option<bool>)],
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        Reader::summary_spans(self, spans, bounds, view, decimals)
    }

    fn navigate(
        &mut self,
        keys: &[NavigationKey<'_>],
        navigation: &Navigation,
    ) -> io::Result<Option<f64>> {
        series::navigate(&mut [(self, keys.to_vec())], navigation)
    }
}

/// Rows one engine page answers at most: replies stay bounded by the reply budget, not this.
const BUDGET: usize = 1 << 24;

/// A publication's keys read by the reader's numbers: `key` names each number's key handle,
/// none where the family never interned it. Several readers share one snapshot.
pub struct Published<'r, 's, P: PageRead> {
    snapshot: &'r RefCell<&'r mut Snapshot<'s, P>>,
    key: Box<dyn Fn(u64) -> Option<KeyHandle> + 'r>,
}

impl<'r, 's, P: PageRead> Published<'r, 's, P> {
    pub fn new(
        snapshot: &'r RefCell<&'r mut Snapshot<'s, P>>,
        key: impl Fn(u64) -> Option<KeyHandle> + 'r,
    ) -> Self {
        Self {
            snapshot,
            key: Box::new(key),
        }
    }
}

fn run(run: dashboard_series::Run) -> Run {
    Run::new(run.span.first, run.span.last, run.value, run.break_before)
}

/// A chart of nothing: every bin unknown.
fn unknown(bounds: (i32, i32), view: (f64, f64, u32)) -> io::Result<ChartSummary> {
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

impl<P: PageRead> SeriesRead for Published<'_, '_, P> {
    fn runs(&mut self, key: u64, from: i32, to: i32) -> io::Result<Vec<Run>> {
        let Some(key) = (self.key)(key) else {
            return Ok(Vec::new());
        };
        let mut snapshot = self.snapshot.borrow_mut();
        Ok(engine::runs(&mut snapshot, key, from, to)?
            .into_iter()
            .map(run)
            .collect())
    }

    fn value_at(&mut self, key: u64, hour: i32) -> io::Result<Option<Run>> {
        let Some(key) = (self.key)(key) else {
            return Ok(None);
        };
        let mut snapshot = self.snapshot.borrow_mut();
        Ok(engine::run_at(&mut snapshot, key, hour)?.map(|(held, _)| run(held)))
    }

    fn summary_within(
        &mut self,
        key: u64,
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
        within: Option<&[HourSpan]>,
    ) -> io::Result<ChartSummary> {
        match (self.key)(key).filter(|_| bounds.0 <= bounds.1) {
            Some(key) => {
                let mut snapshot = self.snapshot.borrow_mut();
                engine::summary(&mut snapshot, key, bounds, view, decimals, within)
            }
            None => unknown(bounds, view),
        }
    }

    fn summary_spans(
        &mut self,
        spans: &[(u64, i32, i32, Option<bool>)],
        bounds: (i32, i32),
        view: (f64, f64, u32),
        decimals: u8,
    ) -> io::Result<ChartSummary> {
        series::viewport(Some(view.0), Some(view.1), Some(view.2))?;
        let mut aliases = Vec::with_capacity(spans.len());
        for &(key, first, last, boundary) in spans {
            if f64::from(last) < view.0.floor() || f64::from(first) > view.1.floor() {
                continue;
            }
            if let Some(key) = (self.key)(key) {
                aliases.push(Alias {
                    key,
                    span: dashboard_series::Span::new(first, last)?,
                    boundary,
                });
            }
        }
        if aliases.is_empty() || bounds.0 > bounds.1 {
            return Ok(ChartSummary {
                from: view.0,
                to: view.1,
                width: view.2,
                buckets: Vec::new(),
                runs: Some(Vec::new()),
                approximate: Vec::new(),
            });
        }
        let viewport = Viewport {
            from: view.0,
            to: view.1,
            width: view.2,
        };
        let bounds = dashboard_series::Span::new(bounds.0, bounds.1)?;
        let mut snapshot = self.snapshot.borrow_mut();
        let mut buckets = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot.summary_aliases(
                &aliases,
                bounds,
                viewport,
                cursor,
                BUDGET,
                |_, bucket| {
                    buckets.push(bucket.exact.map(engine::bucket).transpose()?);
                    Ok(())
                },
            )?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        let (exact, _) = series::summary_bins((bounds.first, bounds.last), view)?;
        let runs = if exact {
            let scale = 10_f64.powi(i32::from(decimals));
            let mut runs = Vec::new();
            let mut cursor = None;
            loop {
                let page = snapshot.hourly_alias_runs(
                    &aliases,
                    bounds,
                    viewport,
                    cursor,
                    BUDGET,
                    |run| {
                        runs.push(crate::prepared_chart::ChartRun {
                            first_hour: run.span.first,
                            last_hour: run.span.last,
                            value: run.value as f64 / scale,
                            break_before: run.break_before,
                        });
                        Ok(())
                    },
                )?;
                cursor = page.next;
                if cursor.is_none() {
                    break;
                }
            }
            Some(runs)
        } else {
            None
        };
        Ok(ChartSummary {
            from: view.0,
            to: view.1,
            width: view.2,
            buckets,
            runs,
            approximate: Vec::new(),
        })
    }

    fn navigate(
        &mut self,
        keys: &[NavigationKey<'_>],
        navigation: &Navigation,
    ) -> io::Result<Option<f64>> {
        let keys: Vec<(KeyHandle, Option<&[HourSpan]>)> = keys
            .iter()
            .filter_map(|&(key, within)| Some(((self.key)(key)?, within)))
            .collect();
        let mut snapshot = self.snapshot.borrow_mut();
        engine::navigate(&mut snapshot, &keys, navigation)
    }
}
