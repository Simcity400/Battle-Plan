//! Placement changes are append-only events; continuation retains the latest
//! pair values. Successful membership dates use separate shared series runs.
use super::*;
use dashboard_base::Source;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader},
};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::check_cancel;

const COVERAGE: u64 = 0;
const MEMBERSHIP_READ: u64 = 1;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    first: Option<i32>,
    last: Option<i32>,
    active: std::collections::BTreeSet<PairKey>,
    membership_dirty: bool,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

impl Prepared {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &DecodedFrame,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        let bound = context
            .content
            .as_ref()
            .map(|options| dashboard_base::catalogue::catalogue(source, options))
            .transpose()
            .map_err(dashboard_text::catalogue::ContentError::from);
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_bound(frame, &changes, &countries, &bound);
        self.test_countries = countries;
        result
    }
    pub fn observe_bound(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<dashboard_text::catalogue::Catalogue>>,
            dashboard_text::catalogue::ContentError,
        >,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:placements");
        let previous = self.last;
        let is_read = operative_rows::membership_read(frame);
        let changed = countries.changed
            || previous.is_none_or(|last| last.checked_add(1) != Some(frame.hour))
            || [TableId::SimulationFeatures, TableId::OperativeMemberships]
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        self.membership_dirty |=
            countries.changed || changes.rows_changed[TableId::OperativeMemberships.index()];
        let needs_read = is_read
            && (self.membership_dirty || self.scan.history.held.is_none())
            && operative_rows::agency_feature(frame) == Some(true);
        self.values.advance(frame.hour)?;
        if changed || needs_read {
            if let Some(previous) = previous {
                for key in &self.active {
                    #[cfg(test)]
                    super::super::intelligence_prepared::work(|work| work.history_visits += 1);
                    if let Some(run) = self
                        .scan
                        .history
                        .pairs
                        .get_mut(key)
                        .and_then(|pair| pair.runs.last_mut())
                    {
                        run.last_hour = previous;
                    }
                }
            }
            self.scan.observe_bound(frame, countries, content)?;
        } else {
            self.scan.history.last_hour = Some(frame.hour);
            self.scan
                .history
                .periods
                .last_mut()
                .expect("placement period")
                .last_hour = frame.hour;
            if is_read && self.scan.history.held.is_some() {
                self.scan.history.reads.push(frame.hour);
            }
        }
        if is_read {
            self.membership_dirty = false;
        }
        self.first.get_or_insert(frame.hour);
        self.last = Some(frame.hour);
        let history = &mut self.scan.history;
        let period = history.periods.last().expect("observed placement coverage");
        let starts = period.first_hour == frame.hour;
        if starts {
            self.values.observe(
                COVERAGE,
                frame.hour,
                Some(i128::from(period.first_hour)),
                true,
            );
            self.values.event(COVERAGE, frame.hour, period)?;
        }
        let read = history
            .reads
            .last()
            .copied()
            .filter(|_| period.availability == PlacementAvailability::Recorded);
        if starts || read == Some(frame.hour) {
            self.values
                .observe(MEMBERSHIP_READ, frame.hour, read.map(i128::from), starts);
        }
        if read == Some(frame.hour) {
            self.values
                .event(MEMBERSHIP_READ, frame.hour, &frame.hour)?;
        }
        if changed || needs_read {
            let active: std::collections::BTreeSet<_> = history
                .held
                .as_ref()
                .into_iter()
                .flatten()
                .map(|(key, _)| key.clone())
                .collect();
            for removed in self.active.difference(&active) {
                let pair = &history.pairs[removed];
                self.values
                    .observe(self.keys[&pair.id], frame.hour, None, false);
            }
            for pair_key in &active {
                let pair = history
                    .pairs
                    .get_mut(pair_key)
                    .expect("live placement pair");
                let next = u64::try_from(self.keys.len())
                    .ok()
                    .and_then(|len| len.checked_add(2))
                    .ok_or_else(|| io::Error::other("Too many placement pairs"))?;
                let key = *self.keys.entry(pair.id.clone()).or_insert(next);
                let run = pair.runs.last().expect("live placement run");
                if run.first_hour == frame.hour || !self.active.contains(pair_key) {
                    self.values.observe(
                        key,
                        frame.hour,
                        Some(i128::from(run.first_hour)),
                        run.first_hour == frame.hour,
                    );
                }
                if run.first_hour == frame.hour {
                    #[cfg(test)]
                    super::super::intelligence_prepared::work(|work| work.emitted += 1);
                    self.values.event(key, frame.hour, run)?;
                }
                if pair.runs.len() > 1 {
                    pair.runs.drain(..pair.runs.len() - 1);
                }
            }
            self.active = active;
        }
        if history.periods.len() > 1 {
            history.periods.drain(..history.periods.len() - 1);
        }
        if history.reads.len() > 1 {
            history.reads.drain(..history.reads.len() - 1);
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Some(last) = self.last {
            for key in &self.active {
                if let Some(run) = self
                    .scan
                    .history
                    .pairs
                    .get_mut(key)
                    .and_then(|pair| pair.runs.last_mut())
                {
                    run.last_hour = last;
                }
            }
        }
        self.values.flush(
            writer,
            &format!("operative-placements-{branch}-series"),
            0,
            cancelled,
        )?;
        if let Some(bounds) = self.first.zip(self.last) {
            series::write_catalogue(writer, "operative-placements", branch, &self.keys, &bounds)?;
        }
        series::write_reply(writer, &format!("operative-placements-{branch}"), 0, self)
    }

    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Option<OperativePlacementReply>> {
    if params.branch_id.parse::<u64>().ok() != Some(branch) {
        return Err(dashboard_core::invalid(
            "Placement parameters do not match the dispatched branch",
        ));
    }
    let Some((keys, (first, last))) =
        series::read_catalogue::<(i32, i32)>(store, key, "operative-placements", branch)?
    else {
        return Ok(None);
    };
    let Some(state) =
        series::read_reply::<Prepared>(store, key, &format!("operative-placements-{branch}"), 0)?
    else {
        return Ok(None);
    };
    let mut first = params.first_hour.unwrap_or(first);
    let last = params.last_hour.unwrap_or(last);
    if first > last {
        return Err(dashboard_core::invalid("Invalid placement query range"));
    }
    let mut reader = Reader::new(
        store,
        key,
        format!("operative-placements-{branch}-series"),
        cancelled,
    )?;
    // A window includes the actual read supplying its left edge. This retains
    // held identity changes and satisfies the product's original coverage checks.
    if let Some(read) = reader.value_at(MEMBERSHIP_READ, first)? {
        first = i32::try_from(read.value)
            .map_err(|_| dashboard_core::invalid("Invalid membership date"))?;
    }
    let mut scan = Scan {
        warnings: state.scan.warnings,
        ..Default::default()
    };
    for span in reader.runs(COVERAGE, first, last)? {
        check_cancel(cancelled)?;
        let (_, row) = reader
            .latest_event(COVERAGE, span.first_hour)?
            .ok_or_else(|| dashboard_core::invalid("Missing placement coverage event"))?;
        let mut period: PlacementPeriod = serde_json::from_value(row)?;
        period.first_hour = span.first_hour;
        period.last_hour = span.last_hour;
        scan.history.periods.push(period);
    }
    scan.history.reads = reader
        .events(&[MEMBERSHIP_READ], first, last, 0, u32::MAX)?
        .rows
        .into_iter()
        .map(|row| serde_json::from_value(row.row).map_err(io::Error::from))
        .collect::<io::Result<_>>()?;
    for (id, key) in keys {
        check_cancel(cancelled)?;
        let (owner, target) = serde_json::from_str(&id)?;
        let pair_key = PairKey { owner, target };
        let mut pair = OperativePlacementPair {
            id,
            owner: pair_key.owner.2.clone(),
            target: pair_key.target.2.clone(),
            runs: Vec::new(),
        };
        for span in reader.runs(key, first, last)? {
            let (_, row) = reader
                .latest_event(key, span.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Missing placement value event"))?;
            let mut run: OperativePlacementRun = serde_json::from_value(row)?;
            run.first_hour = span.first_hour;
            run.last_hour = span.last_hour;
            pair.runs.push(run);
        }
        if !pair.runs.is_empty() {
            scan.history.pairs.insert(pair_key, pair);
        }
    }
    let reply = scan.reply(branch);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests;

pub(crate) fn first_view(
    source: &Source,
    branch: u64,
    first: i32,
    last: i32,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let content = context
        .content
        .as_ref()
        .map(|options| dashboard_base::catalogue::catalogue(source, options))
        .transpose()
        .map_err(dashboard_text::catalogue::ContentError::from);
    let mut scan = Scan::default();
    let branch_first = source
        .block_windows(branch)
        .first()
        .expect("branch windows")
        .first_hour;
    let mut supplying = None;
    let mut countries = super::super::operative_rows::CountryInputs::default();
    super::super::intelligence_prepared::first_views::day(
        source,
        branch,
        first,
        last,
        &[
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::SimulationFeatures,
            TableId::OperativeMemberships,
        ],
        cancelled,
        |frame| {
            if frame.hour == branch_first || super::super::operative_rows::membership_read(frame) {
                supplying.get_or_insert(frame.hour);
            }
            countries.update(frame, None);
            scan.observe_bound(frame, &countries, &content)?;
            if frame.hour < first {
                for pair in scan.history.pairs.values_mut() {
                    if pair.runs.len() > 1 {
                        pair.runs.drain(..pair.runs.len() - 1);
                    }
                }
            }
            Ok(())
        },
    )?;
    let mut data = serde_json::to_value(scan.reply(branch))?;
    data["sampledFrom"] = serde_json::to_value(supplying)?;
    Ok(data)
}
