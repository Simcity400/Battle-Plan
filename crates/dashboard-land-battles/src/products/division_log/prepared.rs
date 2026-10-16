//! The division observer supplies block replies and cumulative selection endpoints.
use super::*;
use dashboard_base::prepared::series;
use serde_json::Value;

// These are the actual totals queries' marginals, not prepared selector subsets.
// Unknown actors/templates stay distinct from the all-actors/all-templates totals.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Dimension {
    All,
    Unknown,
    Value(String),
}
#[derive(Default, Serialize, Deserialize)]
struct Counter {
    total: usize,
    late: usize,
    read: Option<i32>,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Counts {
    pages: BTreeMap<String, u64>,
    #[serde(with = "dashboard_core::map_pairs")]
    keys: BTreeMap<(Dimension, Dimension, usize), Counter>,
    series: series::Accumulator,
}
impl Counts {
    fn observe(&mut self, hour: i32, events: &[Event]) -> io::Result<()> {
        self.series.through(hour);
        let mut changed = std::collections::BTreeSet::new();
        for event in events {
            let row = &event.row;
            let tag = row.tag.clone().map_or(Dimension::Unknown, Dimension::Value);
            let template = row
                .template
                .clone()
                .map_or(Dimension::Unknown, Dimension::Value);
            for tag in [Dimension::All, tag] {
                for template in [Dimension::All, template.clone()] {
                    let key = (tag.clone(), template, kind_index(row.kind));
                    let counter = self.keys.entry(key.clone()).or_default();
                    if counter.read != Some(hour) {
                        counter.read = Some(hour);
                        counter.late = 0;
                    }
                    counter.total = counter
                        .total
                        .checked_add(1)
                        .ok_or_else(|| dashboard_core::invalid("Division totals overflow"))?;
                    // Exact destruction dates are admitted only in an adjacent complete drain.
                    // Other appearances/removals are dated at their read. Thus a native date
                    // preceding a range's first hour can only be observed at that first hour.
                    counter.late += usize::from(row.hour < row.read_hour);
                    changed.insert(key);
                }
            }
        }
        for key in changed {
            let next = self.pages.len() as u64;
            let id = *self
                .pages
                .entry(serde_json::to_string(&key)?)
                .or_insert(next);
            let counter = self.keys.get_mut(&key).expect("observed division counter");
            self.series
                .event(id, hour, &[counter.total, counter.late])?;
        }
        Ok(())
    }
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.series.flush(
            writer,
            &format!("division-log-counts-{branch}"),
            0,
            cancelled,
        )?;
        series::write_catalogue(writer, "division-log-count-keys", branch, &self.pages, &())
    }
}
impl Scan {
    pub fn resume(&mut self, frame: &DecodedFrame) {
        if self.previous_census == Some(frame.hour) && frame.complete::<tables::Divisions>().is_ok()
        {
            self.presentation = Some(Census::new(
                frame,
                self.template_hour,
                self.presentation.take(),
            ));
        }
    }

    pub fn new(format: replay::format::FormatVersion) -> Self {
        Self {
            format,
            ..Self::default()
        }
    }
    #[cfg(test)]
    pub fn prepare(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let first = self.events.len();
        self.observe(frame);
        self.counts.observe(frame.hour, &self.events[first..])
    }
    pub fn prepare_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        let first = self.events.len();
        self.observe_with_changes(frame, Some(changes));
        self.counts.observe(frame.hour, &self.events[first..])
    }
    pub fn configure(&mut self, format: replay::format::FormatVersion) {
        self.format = format;
    }
    pub fn publish_block(
        &mut self,
        source: &Source,
        context: &LoadContext,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let params = DivisionLogParams {
            branch_id: branch.to_string(),
            window: ordinal,
        };
        let reply = self.reply(source, &params, context, cancelled)?;
        series::write_reply(
            writer,
            &format!("division-log-{branch}"),
            ordinal as u64,
            &reply,
        )?;
        let manifest = self.manifest(
            source,
            &BranchParams {
                branch_id: branch.to_string(),
            },
            context,
            cancelled,
        )?;
        series::write_reply(
            writer,
            &format!("division-log-manifest-{branch}"),
            0,
            &manifest,
        )?;
        self.counts.publish(writer, branch, cancelled)
    }
}

struct Endpoints<'a> {
    keys: BTreeMap<String, u64>,
    reader: series::Reader<'a>,
}
impl Endpoints<'_> {
    fn count(
        &mut self,
        tag: &Dimension,
        template: &Dimension,
        kind: usize,
        first: i32,
        last: i32,
        native_first: i32,
    ) -> io::Result<usize> {
        if first > last {
            return Ok(0);
        }
        let name = serde_json::to_string(&(tag, template, kind))?;
        let Some(&id) = self.keys.get(&name) else {
            return Ok(0);
        };
        let after = self
            .reader
            .latest_event(id, last)?
            .map(|(_, value)| serde_json::from_value::<[usize; 2]>(value))
            .transpose()?
            .map_or(0, |value| value[0]);
        let before = match first.checked_sub(1) {
            Some(hour) => self
                .reader
                .latest_event(id, hour)?
                .map(|(_, value)| serde_json::from_value::<[usize; 2]>(value))
                .transpose()?
                .map_or(0, |value| value[0]),
            None => 0,
        };
        let late = if first == native_first {
            match self.reader.latest_event(id, first)? {
                Some((hour, value)) if hour == first => {
                    serde_json::from_value::<[usize; 2]>(value)?[1]
                }
                _ => 0,
            }
        } else {
            0
        };
        after
            .checked_sub(before)
            .and_then(|count| count.checked_sub(late))
            .ok_or_else(|| dashboard_core::invalid("Prepared division counter decreased"))
    }
    fn add(
        &mut self,
        totals: &mut [usize; 3],
        tag: &Dimension,
        templates: &[Dimension],
        ranges: &[(i32, i32)],
        native_first: i32,
    ) -> io::Result<()> {
        for template in templates {
            for &(first, last) in ranges {
                for (kind, total) in totals.iter_mut().enumerate() {
                    *total = total
                        .checked_add(self.count(tag, template, kind, first, last, native_first)?)
                        .ok_or_else(|| dashboard_core::invalid("Division totals overflow"))?;
                }
            }
        }
        Ok(())
    }
}
impl Selection<'_> {
    fn ranges(&self, tag: &str) -> Vec<(i32, i32)> {
        let first = i64::from(self.params.first_hour);
        let end = i64::from(self.params.last_hour) + 1;
        let mut boundaries = vec![first, end];
        for &(from, to) in self.membership.get(tag).into_iter().flatten() {
            boundaries.extend([
                i64::from(from).clamp(first, end),
                (i64::from(to) + 1).clamp(first, end),
            ]);
        }
        for span in &self.params.countries.unobserved {
            boundaries.extend([
                i64::from(span.first_hour).clamp(first, end),
                (i64::from(span.last_hour) + 1).clamp(first, end),
            ]);
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        boundaries
            .windows(2)
            .filter_map(|pair| {
                self.country_matches(Some(tag), pair[0] as i32)
                    .then_some((pair[0] as i32, (pair[1] - 1) as i32))
            })
            .collect()
    }
}
fn blocked_ranges(params: &DivisionLogTotalsParams) -> Vec<(i32, i32)> {
    let mut spans: Vec<_> = params
        .countries
        .unobserved
        .iter()
        .filter_map(|span| {
            let first = span.first_hour.max(params.first_hour);
            let last = span.last_hour.min(params.last_hour);
            (first <= last).then_some((first, last))
        })
        .collect();
    spans.sort_unstable();
    let mut result: Vec<(i32, i32)> = Vec::new();
    for (first, last) in spans {
        if let Some(previous) = result
            .last_mut()
            .filter(|previous| i64::from(first) <= i64::from(previous.1) + 1)
        {
            previous.1 = previous.1.max(last);
        } else {
            result.push((first, last));
        }
    }
    result
}
fn totals(
    params: &DivisionLogTotalsParams,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<DivisionLogTotalsReply> {
    let selection = Selection::new(params);
    let templates: Vec<_> = match &selection.templates {
        None => vec![Dimension::All],
        Some(templates) => templates
            .iter()
            .map(|template| Dimension::Value((*template).to_owned()))
            .collect(),
    };
    let (keys, ()) = series::read_catalogue::<()>(store, key, "division-log-count-keys", branch)?
        .ok_or_else(|| {
        dashboard_core::invalid("Prepared division counter catalogue is missing")
    })?;
    let mut endpoints = Endpoints {
        keys,
        reader: series::Reader::new(
            store,
            key,
            format!("division-log-counts-{branch}"),
            cancelled,
        )?,
    };
    let mut totals = [0; 3];
    if selection.tags.is_none() && params.countries.membership.is_none() {
        endpoints.add(
            &mut totals,
            &Dimension::All,
            &templates,
            &[(params.first_hour, params.last_hour)],
            params.first_hour,
        )?;
        let blocked = blocked_ranges(params);
        let (mut all, mut unknown) = ([0; 3], [0; 3]);
        endpoints.add(
            &mut all,
            &Dimension::All,
            &templates,
            &blocked,
            params.first_hour,
        )?;
        endpoints.add(
            &mut unknown,
            &Dimension::Unknown,
            &templates,
            &blocked,
            params.first_hour,
        )?;
        for kind in 0..3 {
            totals[kind] -= all[kind] - unknown[kind];
        }
    } else {
        let tags: std::collections::BTreeSet<_> = match &selection.tags {
            Some(tags) => tags.iter().copied().collect(),
            None => selection.membership.keys().copied().collect(),
        };
        for tag in tags {
            endpoints.add(
                &mut totals,
                &Dimension::Value(tag.to_owned()),
                &templates,
                &selection.ranges(tag),
                params.first_hour,
            )?;
        }
    }
    totals_reply(totals)
}
pub fn query(
    source: &Source,
    product: &str,
    params: Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let id = params
        .get("branchId")
        .and_then(Value::as_str)
        .ok_or_else(|| dashboard_core::invalid("Invalid division log branch"))?;
    let branch = branch(source, id)?;
    match product {
        "division-log" => {
            let params: DivisionLogParams = serde_json::from_value(params)?;
            DivisionLog::validate(&params).map_err(dashboard_core::invalid)?;
            let reply: DivisionLogReply = series::read_reply(
                store,
                key,
                &format!("division-log-{branch}"),
                params.window as u64,
            )?
            .ok_or_else(|| dashboard_core::invalid("Division log window not in recording"))?;
            serde_json::to_value(bounded(reply)?).map_err(Into::into)
        }
        "division-log-manifest" => {
            let params: BranchParams = serde_json::from_value(params)?;
            DivisionLogManifest::validate(&params).map_err(dashboard_core::invalid)?;
            let reply: DivisionLogManifestReply =
                series::read_reply(store, key, &format!("division-log-manifest-{branch}"), 0)?
                    .ok_or_else(|| {
                        dashboard_core::invalid("Prepared division log manifest is missing")
                    })?;
            serde_json::to_value(bounded(reply)?).map_err(Into::into)
        }
        "division-log-totals" => {
            let params: DivisionLogTotalsParams = serde_json::from_value(params)?;
            DivisionLogTotals::validate(&params).map_err(dashboard_core::invalid)?;
            serde_json::to_value(totals(&params, store, key, branch, cancelled)?)
                .map_err(Into::into)
        }
        _ => unreachable!(),
    }
}
