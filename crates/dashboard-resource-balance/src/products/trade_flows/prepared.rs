//! The shared trade step feeds sparse actual-flow facts and cumulative endpoints.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::query::progressive::{self, AdapterTail};
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use serde_json::{Value, json};

mod integrals;
use integrals::{Publication, Rate};

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    #[serde(skip)]
    catalogue_writer: series::CatalogueWriter,
    scan: Scan,
    keys: BTreeMap<String, u64>,
    facts: Accumulator,
    values: Accumulator,
    totals: [Accumulator; 6],
    coverage: [Accumulator; 2],
    aliases: BTreeSet<String>,
    refusal_keys: BTreeMap<String, u64>,
    refusal_counts: Accumulator,
    refusal_totals: BTreeMap<u64, usize>,
    pattern_keys: BTreeMap<String, u64>,
    pattern_totals: Accumulator,
    current_refusals: BTreeMap<u64, u32>,
    current_pattern: Option<u64>,
    /// Each flow's catalogue key, by flow index, once looked up.
    #[serde(skip)]
    flow_ids: Vec<u64>,
    #[serde(skip)]
    rates: BTreeMap<usize, Rate>,
    #[serde(skip)]
    publication: Publication,
    #[cfg(test)]
    #[serde(skip)]
    hourly_integrals: bool,
    #[cfg(test)]
    #[serde(skip)]
    integral_visits: usize,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
    /// Hours observed from their changed rows alone.
    #[cfg(test)]
    #[serde(skip)]
    row_hours: usize,
    /// Observe every hour from the whole tables, never from changed rows.
    #[cfg(test)]
    #[serde(skip)]
    whole_tables: bool,
}
/// An hour observed from its changed rows alone.
enum RowHour {
    /// Nothing the flows read changed: they only continue.
    Unchanged,
    /// The flows observed again, each with whether it was active before.
    Changed(BTreeMap<usize, bool>),
}
#[derive(Serialize, Deserialize)]
struct Metadata {
    reply: TradeFlowsReply,
    flows: BTreeMap<u64, Flow>,
    refusal_groups: BTreeMap<u64, Option<(Identity, Identity)>>,
    refusal_totals: BTreeMap<u64, usize>,
    patterns: BTreeMap<u64, Vec<u64>>,
    content_warnings: usize,
    had_reads: bool,
}
fn flow_key(flow: &Flow) -> String {
    flow_token(flow.seller_identity, flow.buyer_identity, flow.resource)
}
fn catalogue_key(flow: &Flow) -> String {
    series::field_key(&flow_key(flow), "delivered")
}
/// The catalogue key of flow `index`, allocated when the flow is first
/// published; `ids` keeps every flow's key by index.
fn flow_id(
    keys: &mut BTreeMap<String, u64>,
    ids: &mut Vec<u64>,
    flows: &[Flow],
    index: usize,
) -> u64 {
    while ids.len() <= index {
        let next = keys.len() as u64;
        let id = *keys.entry(catalogue_key(&flows[ids.len()])).or_insert(next);
        ids.push(id);
    }
    ids[index]
}
fn cumulative(values: &mut Accumulator, key: u64, hour: i32, total: i128) {
    if let Some(previous) = values.state(key).and_then(|state| state.last)
        && let Some(first) = previous.last_hour.checked_add(1)
        && first < hour
    {
        values.push(
            key,
            series::Run::new(first, hour - 1, previous.value, false),
        );
    }
    values.observe(key, hour, Some(total), false);
}
impl Prepared {
    #[cfg(test)]
    pub fn observe(&mut self, frame: &replay_frame::frame::DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &FrameChanges {
                rows_changed: [true; replay_data::rows::TABLE_COUNT],
                table_read: [true; replay_data::rows::TABLE_COUNT],
                slot_read: [true; replay_data::rows::TABLE_COUNT],
                updates: None,
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let cpu = ::replay_frame::cpu::scope_phase("trade:observe");
        let continuous =
            self.scan.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let _cpu = ::replay_frame::cpu::scope_phase("trade:observe");
        self.publication.initialize(&self.keys, &self.totals);
        self.publication.continuous = continuous;
        if !continuous {
            if let Some(previous) = self.scan.last_hour {
                self.materialize_integrals(previous)?;
                self.publication.gap(previous);
            }
            self.rates.clear();
        }
        for values in self.totals.iter_mut().chain(self.coverage.iter_mut()) {
            values.advance(frame.hour)?;
        }
        self.facts.advance(frame.hour)?;
        self.values.advance(frame.hour)?;
        self.refusal_counts.advance(frame.hour)?;
        self.pattern_totals.advance(frame.hour)?;
        match self.observe_rows(frame, changes, continuous)? {
            Some(RowHour::Unchanged) => self.scan.advance(frame.hour),
            Some(RowHour::Changed(touched)) => {
                #[cfg(test)]
                {
                    self.work.row_visits = self.scan.inputs.row_visits;
                }
                // A known read continuing a known read, as the whole tables
                // observe it; what it leaves out is as before.
                self.coverage[0].observe(0, frame.hour, Some(1), false);
                self.publish_flows(frame.hour, continuous, touched)?;
            }
            None => self.observe_tables(frame, changes, continuous)?,
        }
        drop(cpu);
        let _cpu = ::replay_frame::cpu::scope_phase("trade:integrals");
        // Branch-wide coverage and refusal clocks advance independently of
        // the flows' deferred constant-rate integrals.
        let known = self.scan.known;
        if known {
            let before = self.coverage[1]
                .state(0)
                .and_then(|state| state.last)
                .map_or(0, |run| run.value);
            cumulative(&mut self.coverage[1], 0, frame.hour, before + 1);
        }
        for (&id, &count) in &self.current_refusals {
            *self.refusal_totals.entry(id).or_default() += count as usize;
        }
        if let Some(id) = self.current_pattern {
            let before = self
                .pattern_totals
                .state(id)
                .and_then(|state| state.last)
                .map_or(0, |run| run.value);
            cumulative(&mut self.pattern_totals, id, frame.hour, before + 1);
        }
        #[cfg(test)]
        if self.hourly_integrals {
            self.observe_integrals_hourly(frame.hour, continuous)?;
        }
        if self.scan.result.reads.len() > 1 {
            self.scan
                .result
                .reads
                .drain(..self.scan.result.reads.len() - 1);
        }
        if self.scan.result.known.len() > 1 {
            self.scan
                .result
                .known
                .drain(..self.scan.result.known.len() - 1);
        }
        if self.scan.result.partial.len() > 1 {
            self.scan
                .result
                .partial
                .drain(..self.scan.result.partial.len() - 1);
        }
        Ok(())
    }

    /// Observe the hour from its changed rows alone: possible when it
    /// continues a known read with the same links and holders, any dictionary
    /// change leaves every resource named as before, and the exchanges changed
    /// by exactly their recorded rows. None when the whole tables are needed.
    fn observe_rows(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &FrameChanges,
        continuous: bool,
    ) -> io::Result<Option<RowHour>> {
        #[cfg(test)]
        {
            if self.whole_tables {
                return Ok(None);
            }
        }
        if !continuous
            || !self.scan.inputs.ready
            || !self.scan.known
            || changes.rows_changed[TableId::CountryLinks.index()]
            || frame.complete::<tables::CountryLinks>().is_err()
        {
            return Ok(None);
        }
        // Recorded changes that delete no country and touch no holder field
        // keep the holders the flows were attributed with, without comparing
        // every country.
        let holders_kept = changes
            .rows(TableId::Countries)
            .is_some_and(|(changed, deleted)| {
                deleted.is_empty() && changed.iter().all(|row| row.mask & *HOLDER_FIELDS == 0)
            })
            && (!changes.rows_changed[TableId::Countries.index()]
                || frame
                    .complete::<tables::Countries>()
                    .ok()
                    .map(|rows| rows.len())
                    == self.scan.inputs.country_rows);
        if !holders_kept {
            return Ok(None);
        }
        let names = changes.rows_changed[TableId::ResourceDefinitions.index()]
            || changes.rows_changed[TableId::LexicalTokens.index()];
        if names && !self.scan.inputs.same_names(frame) {
            return Ok(None);
        }
        let Some((changed, deleted)) = changes.rows(TableId::ResourceExchanges) else {
            return Ok(None);
        };
        // The whole tables observe again whenever the exchanges or the
        // dictionary change; otherwise the flows only continue.
        let observed = if names || changes.rows_changed[TableId::ResourceExchanges.index()] {
            self.scan
                .observe_rows(frame, changed, deleted)?
                .map(RowHour::Changed)
        } else {
            Some(RowHour::Unchanged)
        };
        #[cfg(test)]
        {
            self.row_hours += usize::from(observed.is_some());
        }
        Ok(observed)
    }

    /// Observe the hour from the whole tables.
    fn observe_tables(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &FrameChanges,
        continuous: bool,
    ) -> io::Result<()> {
        // The row index groups the operands under the holders and names the
        // flows were attributed with; the whole tables may change any of them.
        self.scan.row_index = None;
        if !self.scan.inputs.refresh(frame, changes) && continuous {
            self.scan.advance(frame.hour);
            return Ok(());
        }
        self.scan.seal();
        let previous = self.scan.active.clone();
        let sampled = sample::sample_inputs(frame, None, &self.scan.inputs)?;
        self.scan.observe_sampled(frame, &sampled)?;
        #[cfg(test)]
        {
            self.work.row_visits = self.scan.inputs.row_visits;
        }
        let known = self.scan.known;
        self.coverage[0].observe(0, frame.hour, known.then_some(1), false);
        self.observe_refusals(frame.hour, continuous)?;
        let touched: Vec<_> = previous
            .union(&self.scan.active)
            .map(|&index| (index, previous.contains(&index)))
            .collect();
        self.publish_flows(frame.hour, continuous, touched)
    }

    /// Publish the deals the last read left out, by seller and buyer pair,
    /// and the set of pairs it left out.
    fn observe_refusals(&mut self, hour: i32, continuous: bool) -> io::Result<()> {
        let mut rejected = BTreeMap::<u64, u32>::new();
        for (pair, count) in &self.scan.rejected {
            let next = self.refusal_keys.len() as u64;
            let id = *self
                .refusal_keys
                .entry(serde_json::to_string(pair)?)
                .or_insert(next);
            *rejected.entry(id).or_default() += count;
        }
        let touched: BTreeSet<_> = self
            .current_refusals
            .keys()
            .chain(rejected.keys())
            .copied()
            .collect();
        for id in touched {
            let value = rejected.get(&id).copied();
            if !continuous || self.current_refusals.get(&id).copied() != value {
                self.refusal_counts
                    .observe(id, hour, value.map(i128::from), false);
                #[cfg(test)]
                {
                    self.work.emitted += 1;
                }
            }
        }
        self.current_refusals = rejected;
        let members: Vec<_> = self.current_refusals.keys().copied().collect();
        self.current_pattern = if members.is_empty() {
            None
        } else {
            let next = self.pattern_keys.len() as u64;
            Some(
                *self
                    .pattern_keys
                    .entry(serde_json::to_string(&members)?)
                    .or_insert(next),
            )
        };
        Ok(())
    }

    /// Publish the flows an observation touched, in index order, each with
    /// whether it was active before: a run's start, its daily amount, and
    /// its end.
    fn publish_flows(
        &mut self,
        hour: i32,
        continuous: bool,
        touched: impl IntoIterator<Item = (usize, bool)>,
    ) -> io::Result<()> {
        for (index, was_active) in touched {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let id = flow_id(
                &mut self.keys,
                &mut self.flow_ids,
                &self.scan.result.flows,
                index,
            );
            let rate = self.scan.active.contains(&index).then(|| {
                let terms = self.scan.result.flows[index]
                    .runs
                    .last()
                    .expect("active trade run")
                    .terms;
                let amounts = sample::amounts(terms);
                [
                    amounts[0], amounts[1], amounts[2], amounts[3], amounts[4], 1,
                ]
            });
            self.change_rate(index, hour, rate)?;
            let flow = &mut self.scan.result.flows[index];
            let run = self
                .scan
                .active
                .contains(&index)
                .then(|| flow.runs.last().expect("active trade run"));
            if let Some(run) = run {
                self.aliases
                    .extend([run.seller_tag.clone(), run.buyer_tag.clone()]);
                if run.first_hour == hour {
                    self.facts.event(id, hour, run)?;
                }
            }
            let fact = run.map(|run| i128::from(run.first_hour));
            let daily = run.map(|run| i128::from(run.terms.delivered));
            for (values, amount) in [(&mut self.facts, fact), (&mut self.values, daily)] {
                if !continuous
                    || !was_active
                    || values
                        .state(id)
                        .and_then(|state| state.last)
                        .is_none_or(|last| Some(last.value) != amount)
                    || amount.is_none()
                {
                    values.observe(id, hour, amount, false);
                    #[cfg(test)]
                    {
                        self.work.emitted += 1;
                    }
                }
            }
            if flow.runs.len() > 1 {
                flow.runs.drain(..flow.runs.len() - 1);
            }
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        let _cpu = ::replay_frame::cpu::scope_phase("trade:budget");
        #[cfg(test)]
        let hourly = self.hourly_integrals;
        #[cfg(not(test))]
        let hourly = false;
        let integrals = if hourly {
            self.totals.iter().map(Accumulator::bytes).sum()
        } else {
            self.publication.bytes
                + self.scan.last_hour.map_or(0, |hour| {
                    self.rates
                        .iter()
                        .map(|(&index, rate)| {
                            self.publication.pending(self.flow_ids[index], *rate, hour)
                        })
                        .sum::<usize>()
                })
        };
        self.facts.bytes()
            + self.values.bytes()
            + integrals
            + self.coverage.iter().map(Accumulator::bytes).sum::<usize>()
            + self.refusal_counts.bytes()
            + self.pattern_totals.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_named(
            writer,
            &dashboard_base::products::resources::ResourceNaming::of(source, context),
            branch,
            cancelled,
        )
    }

    /// [`Self::flush`], with the resources named through `naming`, taken from
    /// the source where the flows were last observed.
    pub fn flush_named(
        &mut self,
        writer: &mut store::Writer,
        naming: &dashboard_base::products::resources::ResourceNaming,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut payloads = series::Payloads::default();
        if let Some(hour) = self.scan.last_hour {
            self.materialize_integrals(hour)?;
        }
        self.scan.seal();
        if let Some(hour) = self.scan.last_hour {
            // Cumulative endpoints carry their value over unknown observations
            // too. Materialise retired endpoints once at publication after gaps.
            for values in &mut self.totals {
                values.seal();
                for &id in self.keys.values() {
                    if let Some(last) = values
                        .state(id)
                        .and_then(|state| state.last)
                        .filter(|run| run.last_hour < hour)
                    {
                        cumulative(values, id, hour, last.value);
                    }
                }
            }
            self.pattern_totals.seal();
            for &id in self.pattern_keys.values() {
                if let Some(last) = self
                    .pattern_totals
                    .state(id)
                    .and_then(|state| state.last)
                    .filter(|run| run.last_hour < hour)
                {
                    cumulative(&mut self.pattern_totals, id, hour, last.value);
                }
            }
            if let Some(last) = self.coverage[1]
                .state(0)
                .and_then(|state| state.last)
                .filter(|run| run.last_hour < hour)
            {
                cumulative(&mut self.coverage[1], 0, hour, last.value);
            }
        }
        self.facts.flush_with_payloads(
            writer,
            &format!("trade-facts-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.values.flush_with_payloads(
            writer,
            &format!("trade-values-{branch}"),
            RESOURCE_DECIMALS,
            cancelled,
            &mut payloads,
        )?;
        for (index, values) in self.totals.iter_mut().enumerate() {
            values.flush_with_payloads(
                writer,
                &format!("trade-totals-{branch}-{index}"),
                if index == 0 { RESOURCE_DECIMALS } else { 0 },
                cancelled,
                &mut payloads,
            )?;
        }
        for (index, values) in self.coverage.iter_mut().enumerate() {
            values.flush_with_payloads(
                writer,
                &format!("trade-coverage-{branch}-{index}"),
                0,
                cancelled,
                &mut payloads,
            )?;
        }
        self.refusal_counts.flush_with_payloads(
            writer,
            &format!("trade-refusals-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.pattern_totals.flush_with_payloads(
            writer,
            &format!("trade-partial-hours-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.publication = Publication::default();
        let mut reply = super::reply_named(
            naming,
            &TradeFlowParams {
                branch_id: branch.to_string(),
                tags: None,
            },
            &self.scan.result,
            cancelled,
        )?;
        let content_warnings = reply.warnings.len()
            - usize::from(self.scan.result.unnamed > 0)
            - usize::from(self.scan.result.unresolved > 0)
            - usize::from(self.scan.result.reads.is_empty());
        let had_reads = !self.scan.result.reads.is_empty();
        reply.reads.clear();
        reply.known.clear();
        reply.partial.clear();
        reply.flows.clear();
        reply.countries = self.aliases.iter().cloned().collect();
        let flows: BTreeMap<_, _> = self
            .scan
            .result
            .flows
            .iter()
            .map(|flow| {
                let mut flow = flow.clone();
                flow.runs.clear();
                (self.keys[&catalogue_key(&flow)], flow)
            })
            .collect();
        let refusal_groups = self
            .refusal_keys
            .iter()
            .map(|(name, &id)| Ok((id, serde_json::from_str(name)?)))
            .collect::<io::Result<_>>()?;
        let patterns = self
            .pattern_keys
            .iter()
            .map(|(name, &id)| Ok((id, serde_json::from_str(name)?)))
            .collect::<io::Result<_>>()?;
        self.catalogue_writer.write(
            writer,
            "trade-flows",
            branch,
            &self.keys,
            &Metadata {
                reply,
                flows,
                refusal_groups,
                refusal_totals: self.refusal_totals.clone(),
                patterns,
                content_warnings,
                had_reads,
            },
        )
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    tags: Option<Vec<String>>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    hours: Vec<i32>,
    /// Counting basis is separate from the chart viewport.
    basis: Option<i32>,
    op: Option<String>,
    identities: Option<Vec<String>>,
    membership: Option<BTreeMap<String, Vec<HourSpan>>>,
    #[serde(default)]
    unobserved: Vec<HourSpan>,
    #[serde(default)]
    between: bool,
    measure: Option<String>,
    #[serde(default)]
    require_complete: bool,
}
fn combine(first: &[HourSpan], second: &[HourSpan], both: bool) -> Vec<HourSpan> {
    let mut edges = BTreeMap::<i64, i32>::new();
    for span in first.iter().chain(second) {
        *edges.entry(i64::from(span.first_hour)).or_default() += 1;
        *edges.entry(i64::from(span.last_hour) + 1).or_default() -= 1;
    }
    let need = if both { 2 } else { 1 };
    let mut depth = 0;
    let mut start = None;
    let mut result = Vec::new();
    for (hour, delta) in edges {
        depth += delta;
        if depth >= need && start.is_none() {
            start = Some(hour);
        } else if depth < need
            && let Some(first) = start.take()
            && first < hour
        {
            result.push(HourSpan {
                first_hour: first as i32,
                last_hour: (hour - 1) as i32,
            });
        }
    }
    result
}
fn subtract(spans: &[HourSpan], holes: &[HourSpan]) -> Vec<HourSpan> {
    let mut result = Vec::new();
    for span in spans {
        let mut start = i64::from(span.first_hour);
        for hole in holes {
            if i64::from(hole.last_hour) < start || hole.first_hour > span.last_hour {
                continue;
            }
            if i64::from(hole.first_hour) > start {
                result.push(HourSpan {
                    first_hour: start as i32,
                    last_hour: hole.first_hour - 1,
                });
            }
            start = start.max(i64::from(hole.last_hour) + 1);
        }
        if start <= i64::from(span.last_hour) {
            result.push(HourSpan {
                first_hour: start as i32,
                last_hour: span.last_hour,
            });
        }
    }
    result
}
fn prefix(
    readers: &mut [Reader<'_>],
    key: u64,
    hour: i32,
    within: Option<&[HourSpan]>,
    last_hour: i32,
) -> io::Result<[i128; 6]> {
    let mut result = [0i128; 6];
    let hour = hour.min(last_hour);
    for (index, reader) in readers.iter_mut().enumerate() {
        result[index] = if let Some(spans) = within {
            let mut sum = 0;
            for span in spans.iter().filter(|span| span.first_hour <= hour) {
                let last = reader
                    .value_at(key, span.last_hour.min(hour))?
                    .map_or(0, |run| run.value);
                let before = span
                    .first_hour
                    .checked_sub(1)
                    .map(|before| reader.value_at(key, before))
                    .transpose()?
                    .flatten()
                    .map_or(0, |run| run.value);
                sum += last - before;
            }
            sum
        } else {
            reader.value_at(key, hour)?.map_or(0, |run| run.value)
        };
    }
    Ok(result)
}
fn last_read(reader: &mut Reader<'_>, hour: i32) -> io::Result<Option<i32>> {
    if reader.value_at(0, hour)?.is_some() {
        return Ok(Some(hour));
    }
    let Some(previous) = reader.next_change(0, f64::from(hour), false)? else {
        return Ok(None);
    };
    if let Some(run) = reader.value_at(0, previous)? {
        return Ok(Some(run.last_hour.min(hour)));
    }
    Ok(previous
        .checked_sub(1)
        .map(|at| reader.value_at(0, at))
        .transpose()?
        .flatten()
        .map(|run| run.last_hour.min(hour)))
}
fn active_bounds(
    facts: &mut Reader<'_>,
    key: u64,
    first: i32,
    last: i32,
    within: Option<&[HourSpan]>,
) -> io::Result<[Option<i32>; 2]> {
    let whole = [HourSpan {
        first_hour: first,
        last_hour: last,
    }];
    let mut result: [Option<i32>; 2] = [None, None];
    for span in within.unwrap_or(&whole) {
        let start = span.first_hour.max(first);
        let end = span.last_hour.min(last);
        if start > end {
            continue;
        }
        let earliest = if facts.value_at(key, start)?.is_some() {
            Some(start)
        } else {
            facts
                .events(&[key], start, end, 0, 1)?
                .rows
                .first()
                .map(|row| row.hour)
        };
        if let Some(at) = earliest {
            result[0] = Some(result[0].map_or(at, |before| before.min(at)));
        }
        if let Some((at, _)) = facts.latest_event(key, end)?
            && let Some(run) = facts.value_at(key, at)?
        {
            let at = run.last_hour.min(end);
            if at >= start {
                result[1] = Some(result[1].map_or(at, |before| before.max(at)));
            }
        }
    }
    Ok(result)
}
#[allow(clippy::type_complexity)]
pub fn query(
    source: &Source,
    params: Value,
    content: Option<&dashboard_core::config::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params.clone())?;
    let selection = TradeFlowParams {
        branch_id: input.branch_id.clone(),
        tags: input.tags.clone(),
    };
    TradeFlows::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection.branch();
    // The family's own coverage while it is prepared on its own.
    let coverage = match dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "trade-flows",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        result => result?,
    };
    let complete = coverage.as_ref().is_some_and(|coverage| coverage.complete);
    if input.require_complete && !complete {
        return Ok(None);
    }
    // Integrals, running averages and navigation depend on earlier frames.
    if !complete
        && (input.op.is_some()
            || input
                .measure
                .as_deref()
                .is_some_and(|measure| measure != "current"))
    {
        return Ok(None);
    }
    let (store, key) = dashboard_base::prepared::files(source, content)?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown trade branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), start);
    let last_hour = coverage
        .as_ref()
        .map_or(bounds.0.saturating_sub(1), |coverage| coverage.last_hour);
    let view = series::viewport(
        input.from.or(Some(f64::from(bounds.1))),
        input.to.or(input.from).or(Some(f64::from(bounds.1))),
        Some(input.width.unwrap_or(1)),
    )?;
    let mut requested: Vec<_> = if let Some(identities) = &input.identities {
        identities
            .iter()
            .map(|id| (id.clone(), "actor".into()))
            .collect()
    } else {
        input
            .tags
            .as_ref()
            .into_iter()
            .flatten()
            .map(|tag| (tag.clone(), String::new()))
            .collect()
    };
    if requested.is_empty() && (input.identities.is_some() || input.tags.is_some()) {
        requested.push((String::new(), String::new()));
    }
    let manual = input.identities.is_none() && input.tags.is_some();
    let selected_tags: BTreeSet<_> = input.tags.as_ref().into_iter().flatten().cloned().collect();
    let actors = std::cell::RefCell::new(if manual {
        let prefix = progressive::histories(
            &store,
            &key,
            branch,
            coverage.as_ref(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            source.replay_ref(),
            &selected_tags.iter().cloned().collect::<Vec<_>>(),
            cancelled,
        )?;
        country_history::selected_identities(&prefix, input.tags.as_ref().expect("manual tags"))
    } else {
        BTreeSet::new()
    });
    let names = std::cell::RefCell::new(Learned::default());
    let sampled_inputs = sample::Query::default();
    let sampler = |frame: &replay_frame::frame::DecodedFrame,
                   requested: Option<&[(String, String)]>| {
        if let (Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::ResourceDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            names.borrow_mut().learn(definitions, lexical);
        }
        let mut requested = requested.unwrap_or_default().to_vec();
        requested.extend(
            actors
                .borrow()
                .iter()
                .map(|&actor| (identity(actor), "actor".into())),
        );
        let _cpu = replay_frame::cpu::scope_phase("trade:query_sample");
        let sampled = sampled_inputs.sample(frame, Some(&requested), source.replay_ref().format)?;
        if manual {
            for ((tag, field), (holder, _)) in &sampled {
                if field == "holder"
                    && selected_tags.contains(tag)
                    && let Some(holder) = holder
                {
                    actors.borrow_mut().insert(*holder);
                }
            }
        }
        Ok(sampled)
    };
    let mut hours: BTreeSet<_> = input
        .hours
        .iter()
        .copied()
        .chain([view.0.floor() as i32, view.1.floor() as i32])
        .collect();
    if manual && coverage.is_none() {
        hours.extend(
            source
                .metadata()
                .countries
                .iter()
                .filter(|country| {
                    country.observed_branch_id == input.branch_id
                        && selected_tags.contains(&country.tag)
                })
                .map(|country| country.observed_hour),
        );
    }
    let early_states = manual
        .then(|| {
            progressive::state_tail(
                source,
                branch,
                coverage.as_ref(),
                sample::TABLES,
                &requested,
                &hours.iter().copied().collect::<Vec<_>>(),
                &sampler,
                cancelled,
            )
        })
        .transpose()?;
    let tail = progressive::tail(
        source,
        branch,
        coverage.as_ref(),
        sample::TABLES,
        &requested,
        (view.0.floor() as i32, view.1.floor() as i32),
        &sampler,
        cancelled,
    )?;
    let hours: Vec<_> = hours
        .into_iter()
        .filter(|hour| {
            !tail.1.is_empty() || *hour < view.0.floor() as i32 || *hour > view.1.floor() as i32
        })
        .collect();
    let states = match early_states {
        Some(states) => states,
        None => progressive::state_tail(
            source,
            branch,
            coverage.as_ref(),
            sample::TABLES,
            &requested,
            &hours,
            &sampler,
            cancelled,
        )?,
    };
    let holder_spans = |spans: &BTreeMap<(String, String), Vec<(i32, i32, u32, u32)>>| {
        spans
            .iter()
            .filter(|((_, field), _)| field == "holder")
            .map(|(key, spans)| (key.clone(), spans.clone()))
            .collect()
    };
    let history = progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &holder_spans(&tail.2),
        &holder_spans(&states.2),
        source.replay_ref(),
        &selected_tags.iter().cloned().collect::<Vec<_>>(),
        cancelled,
    )?;
    let fallback = super::reply(
        source,
        &selection,
        &LoadContext {
            content: content.cloned(),
        },
        &ScannedFlows {
            resources: names.borrow().list(),
            ..Default::default()
        },
        cancelled,
    )?;
    let context = (!complete).then_some((
        (bounds, &tail.0, tail.1.as_slice(), &tail.2, true),
        (bounds, &states.0, states.1.as_slice(), &states.2, true),
        fallback,
    ));
    let extent = dashboard_base::prepared::prefix::extent(source, branch, coverage)?;
    read_pages(
        &store, &key, branch, input, params, &history, last_hour, complete, context, cancelled,
    )
    .map(|value| {
        value.map(|mut value| {
            value["coverage"] = json!(extent);
            value
        })
    })
}

type Progressive<'a> = (AdapterTail<'a>, AdapterTail<'a>, TradeFlowsReply);

type Holders<'a> = BTreeMap<Identity, Vec<(i32, i32, &'a str)>>;
fn holder_at<'a>(
    holders: &Holders<'a>,
    identity: Identity,
    hour: i32,
) -> Option<(i32, i32, &'a str)> {
    let runs = holders.get(&identity)?;
    let at = runs.partition_point(|run| run.0 <= hour).checked_sub(1)?;
    runs.get(at).copied().filter(|run| hour <= run.1)
}

/// Exact native terms at a requested date; rough values remain geometry only.
fn sampled_run(
    values: &BTreeMap<(String, String), Vec<series::Run>>,
    token: &str,
    hour: i32,
    holders: &Holders<'_>,
) -> io::Result<Option<Run>> {
    let (seller, buyer, _) = sample::pair(token)?;
    let mut amounts = [0; 5];
    let mut first = i32::MIN;
    let mut last = i32::MAX;
    for (field, amount) in sample::FIELDS.iter().zip(&mut amounts) {
        let Some(run) = values
            .get(&(token.to_owned(), (*field).to_owned()))
            .and_then(|runs| {
                runs.partition_point(|run| run.first_hour <= hour)
                    .checked_sub(1)
                    .and_then(|at| runs.get(at))
            })
            .filter(|run| hour <= run.last_hour)
        else {
            return Ok(None);
        };
        *amount = run.value;
        first = first.max(run.first_hour);
        last = last.min(run.last_hour);
    }
    let (Some(seller), Some(buyer)) = (
        holder_at(holders, seller, hour),
        holder_at(holders, buyer, hour),
    ) else {
        return Ok(None);
    };
    Ok(Some(Run {
        first_hour: first.max(seller.0).max(buyer.0),
        last_hour: last.min(seller.1).min(buyer.1),
        seller_tag: seller.2.to_owned(),
        buyer_tag: buyer.2.to_owned(),
        terms: sample::terms(amounts)?,
    }))
}

#[allow(clippy::too_many_arguments)]
fn read_pages(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    input: Params,
    params: Value,
    history: &RecordedHistories,
    last_hour: i32,
    complete: bool,
    progressive: Option<Progressive<'_>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let _cpu = replay_frame::cpu::scope_phase("trade:query_pages");
    let mut holders = Holders::new();
    for (tag, country) in &history.countries {
        for run in country.identities.iter() {
            holders
                .entry((run.slot, run.country_id))
                .or_default()
                .push((run.first_hour, run.last_hour, tag));
        }
    }
    for spans in holders.values_mut() {
        spans.sort_unstable();
    }
    let catalogue = series::read_catalogue::<Metadata>(store, source_key, "trade-flows", branch)?;
    if catalogue.is_none() && progressive.is_none() {
        return Ok(None);
    }
    let (mut keys, mut metadata) = catalogue.unwrap_or_else(|| {
        (
            BTreeMap::new(),
            Metadata {
                reply: progressive
                    .as_ref()
                    .expect("progressive catalogue")
                    .2
                    .clone(),
                flows: BTreeMap::new(),
                refusal_groups: BTreeMap::new(),
                refusal_totals: BTreeMap::new(),
                patterns: BTreeMap::new(),
                content_warnings: progressive
                    .as_ref()
                    .expect("progressive catalogue")
                    .2
                    .warnings
                    .len()
                    .saturating_sub(1),
                had_reads: false,
            },
        )
    });
    if let Some((tail, states, fallback)) = &progressive {
        for resource in &fallback.resources {
            if !metadata
                .reply
                .resources
                .iter()
                .any(|item| item.id == resource.id)
            {
                metadata.reply.resources.push(resource.clone());
            }
        }
        metadata.reply.resources.sort_by_key(|resource| resource.id);
        let aliases: BTreeSet<_> = metadata
            .reply
            .countries
            .iter()
            .cloned()
            .chain(history.countries.keys().cloned())
            .collect();
        metadata.reply.countries = aliases.into_iter().collect();
        for ((token, field), runs) in tail.1.iter().chain(states.1) {
            if field != "delivered" || runs.is_empty() {
                continue;
            }
            let (seller, buyer, resource) = sample::pair(token)?;
            let name = series::field_key(token, "delivered");
            if !keys.contains_key(&name) {
                let id = keys.len() as u64;
                let hour = runs[0].first_hour;
                let (Some(seller_tag), Some(buyer_tag)) = (
                    holder_at(&holders, seller, hour),
                    holder_at(&holders, buyer, hour),
                ) else {
                    continue;
                };
                keys.insert(name, id);
                metadata.flows.insert(
                    id,
                    Flow {
                        seller: seller_tag.2.to_owned(),
                        buyer: buyer_tag.2.to_owned(),
                        seller_identity: seller,
                        buyer_identity: buyer,
                        resource,
                        runs: Vec::new(),
                    },
                );
            }
        }
        metadata.had_reads |= tail.1.contains_key(&(String::new(), "known".into()))
            || states.1.contains_key(&(String::new(), "known".into()));
    }
    let mut reply = metadata.reply;
    reply.country_identities = history
        .countries
        .iter()
        .filter(|(_, country)| !country.identities.is_empty())
        .map(|(tag, country)| TradeCountryIdentity {
            tag: tag.clone(),
            runs: country.identities.as_ref().clone(),
        })
        .collect();
    let selected = if let Some(ids) = &input.identities {
        Some(
            ids.iter()
                .map(|id| serde_json::from_str::<Identity>(id).map_err(io::Error::from))
                .collect::<io::Result<BTreeSet<_>>>()?,
        )
    } else {
        input
            .tags
            .as_ref()
            .map(|tags| country_history::selected_identities(history, tags))
    };
    let mut membership = BTreeMap::<Identity, Vec<HourSpan>>::new();
    if let Some(members) = &input.membership {
        for (tag, spans) in members {
            for identity in history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
            {
                let clipped = combine(
                    &[HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    }],
                    spans,
                    true,
                );
                membership
                    .entry((identity.slot, identity.country_id))
                    .or_default()
                    .extend(clipped);
            }
        }
        for spans in membership.values_mut() {
            *spans = subtract(&combine(spans, &[], false), &input.unobserved);
        }
    }
    let flows: Vec<_> = metadata
        .flows
        .into_iter()
        .filter(|(_, flow)| {
            selected.as_ref().is_none_or(|ids| {
                if input.between {
                    ids.contains(&flow.seller_identity) && ids.contains(&flow.buyer_identity)
                } else {
                    ids.contains(&flow.seller_identity) || ids.contains(&flow.buyer_identity)
                }
            })
        })
        .collect();
    let diagnostic_selected = input
        .tags
        .as_ref()
        .map(|tags| country_history::selected_identities(history, tags));
    let selected_refusals: BTreeSet<_> = metadata
        .refusal_groups
        .iter()
        .filter(|(_, pair)| {
            pair.is_none_or(|(seller, buyer)| {
                diagnostic_selected
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&seller) || ids.contains(&buyer))
            })
        })
        .map(|(&id, _)| id)
        .collect();
    let unresolved = metadata
        .refusal_groups
        .iter()
        .filter(|(_, pair)| pair.is_none())
        .map(|(&id, _)| metadata.refusal_totals[&id])
        .sum();
    let unnamed = metadata
        .refusal_groups
        .iter()
        .filter(|(id, pair)| pair.is_some() && selected_refusals.contains(*id))
        .map(|(&id, _)| metadata.refusal_totals[&id])
        .sum();
    reply.warnings.truncate(metadata.content_warnings);
    super::recording_warnings(unnamed, unresolved, metadata.had_reads, &mut reply.warnings);
    let mut refusals = Reader::new(
        store,
        source_key,
        format!("trade-refusals-{branch}"),
        cancelled,
    )?;
    let mut facts = Reader::new(
        store,
        source_key,
        format!("trade-facts-{branch}"),
        cancelled,
    )?;
    let mut values = Reader::new(
        store,
        source_key,
        format!("trade-values-{branch}"),
        cancelled,
    )?;
    let mut known = Reader::new(
        store,
        source_key,
        format!("trade-coverage-{branch}-0"),
        cancelled,
    )?;
    if input.op.as_deref() == Some("navigate") {
        return Ok(Some(
            json!({"hour":series::navigate(&mut[(&mut facts,flows.iter().map(|(key,_)|(*key,None)).collect()),(&mut known,vec![(0,None)]),(&mut refusals,selected_refusals.iter().map(|&id|(id,None)).collect())],&serde_json::from_value(params)?)?}),
        ));
    }
    if input.op.is_some() {
        return Err(dashboard_core::invalid("Unknown trade operation"));
    }
    let from = input.from.unwrap_or(f64::from(last_hour));
    let to = input.to.unwrap_or(from);
    let (from, to, width) = series::viewport(Some(from), Some(to), Some(input.width.unwrap_or(1)))?;
    let first = from.floor() as i32;
    let last = to.floor() as i32;
    if progressive.is_none()
        && !complete
        && (last > last_hour || input.hours.iter().any(|&hour| hour > last_hour))
    {
        return Ok(None);
    }
    if input.width.is_some() {
        series::summary_budget(from, to, width, flows.len())?;
    }
    let dense = input
        .width
        .is_some_and(|width| i64::from(last) - i64::from(first) + 1 > i64::from(width));
    let basis = input.basis.unwrap_or(first);
    let mut moments: BTreeSet<_> = input.hours.iter().copied().chain([first, last]).collect();
    let mut catalogue = ScannedFlows::default();
    let measure = input.measure.as_deref().unwrap_or("current");
    if !["current", "total", "average"].contains(&measure) {
        return Err(dashboard_core::invalid("Invalid trade measure"));
    }
    let display_coverage = subtract(
        &[HourSpan {
            first_hour: first,
            last_hour: last,
        }],
        &input.unobserved,
    );
    let known_summary = if let Some((tail, _, _)) = &progressive {
        known.summary_with_tail(
            0,
            tail.0,
            (from, to, width),
            0,
            tail.1
                .get(&(String::new(), "known".into()))
                .map_or(&[][..], Vec::as_slice),
            Some(&display_coverage),
        )?
    } else {
        known.summary_within(
            0,
            (first, last),
            (from, to, width),
            0,
            Some(&display_coverage),
        )?
    };
    let mut summaries = BTreeMap::new();
    let mut integrals = BTreeMap::new();
    let mut bounds = BTreeMap::new();
    let mut totals: Vec<_> = (0..6)
        .map(|index| {
            Reader::new(
                store,
                source_key,
                format!("trade-totals-{branch}-{index}"),
                cancelled,
            )
        })
        .collect::<io::Result<_>>()?;
    for (id, mut flow) in flows {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let within = input.membership.as_ref().map(|_| {
            combine(
                membership
                    .get(&flow.seller_identity)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                membership
                    .get(&flow.buyer_identity)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                input.between,
            )
        });
        let flow_known = if let Some(within) = &within {
            let within = combine(within, &display_coverage, true);
            if let Some((tail, _, _)) = &progressive {
                known.summary_with_tail(
                    0,
                    tail.0,
                    (from, to, width),
                    0,
                    tail.1
                        .get(&(String::new(), "known".into()))
                        .map_or(&[][..], Vec::as_slice),
                    Some(&within),
                )?
            } else {
                known.summary_within(0, (first, last), (from, to, width), 0, Some(&within))?
            }
        } else {
            known_summary.clone()
        };
        let mut dates = moments.clone();
        let mut runs = if dense || progressive.is_some() {
            let mut summary = if let Some((tail, _, _)) = &progressive {
                values.summary_with_tail(
                    id,
                    tail.0,
                    (from, to, width),
                    RESOURCE_DECIMALS,
                    tail.1
                        .get(&(flow_key(&flow), "delivered".into()))
                        .map_or(&[][..], Vec::as_slice),
                    within.as_deref(),
                )?
            } else if measure == "current" {
                values.summary_within(
                    id,
                    (first, last),
                    (from, to, width),
                    RESOURCE_DECIMALS,
                    within.as_deref(),
                )?
            } else {
                totals[0].summary(id, (first, last), (from, to, width), RESOURCE_DECIMALS)?
            };
            if measure != "current" {
                // A running average is bounded by its opening value and the
                // daily amounts in the bucket. Cumulative-amount extrema alone
                // can miss a ratio peak followed by low-volume trading.
                let daily = (measure == "average")
                    .then(|| {
                        values.summary_within(
                            id,
                            (first, last),
                            (from, to, width),
                            RESOURCE_DECIMALS,
                            within.as_deref(),
                        )
                    })
                    .transpose()?;
                let baseline = basis
                    .checked_sub(1)
                    .map(|hour| prefix(&mut totals, id, hour, within.as_deref(), last_hour))
                    .transpose()?
                    .unwrap_or([0; 6]);
                for (column, bucket) in summary.buckets.iter_mut().enumerate() {
                    let Some(bucket) = bucket else {
                        continue;
                    };
                    let hours = [
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ];
                    let mut samples = Vec::new();
                    for hour in hours {
                        let own = prefix(&mut totals, id, hour, within.as_deref(), last_hour)?;
                        let amount = own[0] - baseline[0];
                        let duration = own[5] - baseline[5];
                        let raw = if measure == "total" {
                            amount as f64 / 24.0
                        } else if duration > 0 {
                            amount as f64 / duration as f64
                        } else {
                            0.0
                        };
                        samples.push((hour, raw / 10f64.powi(i32::from(RESOURCE_DECIMALS))));
                    }
                    bucket.first_value = samples[0].1;
                    bucket.last_value = samples[1].1;
                    let min = samples.iter().min_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
                    let max = samples.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
                    bucket.min_hour = min.0;
                    bucket.min_value = min.1;
                    bucket.max_hour = max.0;
                    bucket.max_value = max.1;
                    bucket.last_value_hour = bucket.last_hour;
                    if let Some(daily) = daily.as_ref().and_then(|daily| daily.buckets[column]) {
                        if daily.min_value < bucket.min_value {
                            bucket.min_value = daily.min_value;
                            bucket.min_hour = daily.min_hour;
                        }
                        if daily.max_value > bucket.max_value {
                            bucket.max_value = daily.max_value;
                            bucket.max_hour = daily.max_hour;
                        }
                    }
                }
                summary.runs = None;
            }
            for (bucket, coverage) in summary.buckets.iter_mut().zip(&flow_known.buckets) {
                if let Some(known) = coverage {
                    if let Some(values) = bucket {
                        if values.first_hour > known.first_hour
                            || values.last_hour < known.last_hour
                        {
                            values.min_value = values.min_value.min(0.0);
                            values.max_value = values.max_value.max(0.0);
                            if values.first_hour > known.first_hour {
                                values.first_hour = known.first_hour;
                                values.first_value = 0.0;
                            }
                            if values.last_hour < known.last_hour {
                                values.last_hour = known.last_hour;
                                values.last_value = 0.0;
                                values.last_value_hour = known.last_hour;
                            }
                        }
                        values.has_break |= known.has_break;
                        values.first_break |= known.first_break;
                    } else {
                        let mut zero = *known;
                        zero.first_value = 0.0;
                        zero.last_value = 0.0;
                        zero.min_value = 0.0;
                        zero.max_value = 0.0;
                        *bucket = Some(zero);
                    }
                } else {
                    *bucket = None;
                }
            }
            if let (Some(runs), Some(coverage)) = (&mut summary.runs, &flow_known.runs) {
                // The native successful empty census means zero between listed deals.
                // Apply that same existing zero fill to the exact supplying runs.
                let active: Vec<_> = runs
                    .iter()
                    .map(|run| HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    })
                    .collect();
                let known: Vec<_> = coverage
                    .iter()
                    .map(|run| HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    })
                    .collect();
                for zero in subtract(&known, &active) {
                    runs.push(dashboard_base::prepared_chart::ChartRun {
                        first_hour: zero.first_hour,
                        last_hour: zero.last_hour,
                        value: 0.0,
                        break_before: true,
                    });
                }
                runs.sort_by_key(|run| run.first_hour);
                let mut previous: Option<i32> = None;
                for run in runs {
                    run.break_before =
                        previous.and_then(|hour| hour.checked_add(1)) != Some(run.first_hour);
                    previous = Some(run.last_hour);
                }
            }
            if let Some((tail, _, _)) = &progressive {
                progressive::clip_approximate(&mut summary, tail.2, within.as_deref());
            }
            for bucket in summary.buckets.iter().flatten() {
                dates.extend([
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]);
            }
            summaries.insert(flow_key(&flow), summary);
            if dense {
                Vec::new()
            } else {
                facts.runs(id, first, last)?
            }
        } else {
            facts.runs(id, first, last)?
        };
        for &hour in &dates {
            if let Some(run) = facts.value_at(id, hour)? {
                runs.push(run);
            }
        }
        let mut seen = BTreeSet::new();
        for run in runs {
            let run = facts
                .value_at(id, run.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Trade fact coverage is missing"))?;
            if !seen.insert((run.first_hour, run.last_hour)) {
                continue;
            }
            let (_, value) = facts
                .latest_event(
                    id,
                    run.value
                        .try_into()
                        .map_err(|_| dashboard_core::invalid("Invalid trade fact date"))?,
                )?
                .ok_or_else(|| dashboard_core::invalid("Trade fact is missing"))?;
            let mut row: Run = serde_json::from_value(value)?;
            row.first_hour = run.first_hour;
            row.last_hour = run.last_hour;
            if let Some(spans) = &within {
                for span in spans {
                    let first = row.first_hour.max(span.first_hour);
                    let last = row.last_hour.min(span.last_hour);
                    if first <= last {
                        let mut clipped = row.clone();
                        clipped.first_hour = first;
                        clipped.last_hour = last;
                        flow.runs.push(clipped);
                    }
                }
            } else {
                flow.runs.push(row);
            }
        }
        if let Some((tail, states, _)) = &progressive {
            let token = flow_key(&flow);
            let mut seen = BTreeSet::new();
            for exact in [Some(tail).filter(|tail| tail.2.is_empty()), Some(states)]
                .into_iter()
                .flatten()
            {
                let dates: BTreeSet<_> = sample::FIELDS
                    .iter()
                    .flat_map(|field| exact.1.get(&(token.clone(), (*field).to_owned())))
                    .flat_map(|runs| runs.iter().flat_map(|run| [run.first_hour, run.last_hour]))
                    .collect();
                for hour in dates {
                    if flow
                        .runs
                        .iter()
                        .any(|run| run.first_hour <= hour && hour <= run.last_hour)
                    {
                        continue;
                    }
                    if let Some(row) = sampled_run(exact.1, &token, hour, &holders)? {
                        if !seen.insert((row.first_hour, row.last_hour)) {
                            continue;
                        }
                        if let Some(spans) = &within {
                            for span in spans {
                                let first = row.first_hour.max(span.first_hour);
                                let last = row.last_hour.min(span.last_hour);
                                if first <= last {
                                    flow.runs.push(Run {
                                        first_hour: first,
                                        last_hour: last,
                                        ..row.clone()
                                    });
                                }
                            }
                        } else {
                            flow.runs.push(row);
                        }
                    }
                }
            }
        }
        flow.runs.sort_by_key(|run| run.first_hour);
        // Every requested prefix is a fixed number of page lookups, even in peace.
        for run in &flow.runs {
            if let Some(hour) = run.first_hour.checked_sub(1) {
                dates.insert(hour);
            }
            dates.extend([run.first_hour, run.last_hour]);
        }
        dates.extend([basis.saturating_sub(1)]);
        let mut prefixes = BTreeMap::new();
        let mut observed = BTreeMap::new();
        for hour in dates.into_iter().filter(|_| progressive.is_none()) {
            let sum = prefix(&mut totals, id, hour, within.as_deref(), last_hour)?;
            prefixes.insert(hour, sum.map(|value| value.to_string()));
            if hour >= basis {
                observed.insert(
                    hour,
                    active_bounds(&mut facts, id, basis, hour, within.as_deref())?,
                );
            }
        }
        if progressive.is_none() {
            bounds.insert(flow_key(&flow), observed);
            integrals.insert(flow_key(&flow), prefixes);
        }
        catalogue.flows.push(flow);
    }
    let (_, flows) = catalogue.reply_flows();
    // Stable campaign aliases keep all catalogue indices fixed for paged reads.
    let (slice_countries, _) = catalogue.reply_flows();
    let country_index = |tag: &str| {
        reply
            .countries
            .binary_search_by(|value| value.as_str().cmp(tag))
            .expect("prepared alias") as u32
    };
    reply.flows = flows
        .into_iter()
        .map(|mut flow| {
            flow.seller = country_index(&slice_countries[flow.seller as usize]);
            flow.buyer = country_index(&slice_countries[flow.buyer as usize]);
            for index in &mut flow.seller_tags {
                *index = country_index(&slice_countries[*index as usize]);
            }
            for index in &mut flow.buyer_tags {
                *index = country_index(&slice_countries[*index as usize]);
            }
            flow
        })
        .collect();
    if !dense {
        for &key in &selected_refusals {
            for run in refusals.runs(key, first, last)? {
                moments.extend([run.first_hour, run.last_hour]);
            }
        }
        for run in known.runs(0, first, last)? {
            moments.extend(run.first_hour..=run.last_hour);
        }
    }
    let mut seen = BTreeSet::new();
    for &hour in &moments {
        if let Some(read) = last_read(&mut known, hour)? {
            reply.reads.push(read);
            if let Some(run) = known.value_at(0, read)? {
                reply.known.push(HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                });
            }
        }
        if let Some(run) = known.value_at(0, hour)? {
            let mut start = run.first_hour;
            let mut end = run.last_hour;
            let mut skipped = 0i128;
            for &id in &selected_refusals {
                if let Some(refusal) = refusals.value_at(id, hour)? {
                    start = start.max(refusal.first_hour);
                    end = end.min(refusal.last_hour);
                    skipped += refusal.value;
                } else if let Some(next) = refusals.next_change(id, f64::from(hour), true)? {
                    end = end.min(next.saturating_sub(1));
                }
            }
            if seen.insert((start, end)) {
                reply.known.push(HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                });
                if skipped > 0 {
                    reply.partial.push(PartialSpan {
                        first_hour: start,
                        last_hour: end,
                        deals: skipped
                            .try_into()
                            .map_err(|_| dashboard_core::invalid("Invalid partial trade count"))?,
                    });
                }
            }
        }
    }
    if let Some((tail, states, _)) = &progressive {
        for exact in [Some(tail).filter(|tail| tail.2.is_empty()), Some(states)]
            .into_iter()
            .flatten()
        {
            if let Some(runs) = exact.1.get(&(String::new(), "known".into())) {
                for run in runs {
                    reply.known.push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
                    reply.reads.extend([run.first_hour, run.last_hour]);
                }
            }
            if let Some(runs) = exact.1.get(&(String::new(), "partial".into())) {
                for run in runs.iter().filter(|run| run.value > 0) {
                    reply.partial.push(PartialSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                        deals: run.value.try_into().map_err(|_| {
                            dashboard_core::invalid("Invalid sampled partial trade count")
                        })?,
                    });
                }
            }
        }
    }
    reply.reads.sort_unstable();
    reply.reads.dedup();
    reply.known.sort_by_key(|run| run.first_hour);
    reply
        .known
        .dedup_by_key(|run| (run.first_hour, run.last_hour));
    reply.partial.sort_by_key(|run| run.first_hour);
    let mut partial: Vec<PartialSpan> = Vec::new();
    for run in reply.partial {
        if let Some(previous) = partial.last_mut()
            && previous.deals == run.deals
            && previous.last_hour.checked_add(1) == Some(run.first_hour)
        {
            previous.last_hour = run.last_hour;
        } else {
            partial.push(run);
        }
    }
    reply.partial = partial;
    let mut coverage_counts = BTreeMap::new();
    for hour in moments.into_iter().filter(|_| progressive.is_none()) {
        let mut counts = [0i128; 2];
        for (index, total) in counts.iter_mut().enumerate() {
            let mut reader = Reader::new(
                store,
                source_key,
                format!("trade-coverage-{branch}-1"),
                cancelled,
            )?;
            if index == 0 {
                *total = reader.value_at(0, hour)?.map_or(0, |run| run.value)
                    - reader
                        .value_at(0, basis.saturating_sub(1))?
                        .map_or(0, |run| run.value);
            } else {
                let mut patterns = Reader::new(
                    store,
                    source_key,
                    format!("trade-partial-hours-{branch}"),
                    cancelled,
                )?;
                for (&id, members) in &metadata.patterns {
                    if members.iter().any(|id| selected_refusals.contains(id)) {
                        *total += patterns.value_at(id, hour)?.map_or(0, |run| run.value)
                            - patterns
                                .value_at(id, basis.saturating_sub(1))?
                                .map_or(0, |run| run.value);
                    }
                }
            }
        }
        coverage_counts.insert(hour, counts);
    }
    let mut data = serde_json::to_value(reply)?;
    data["summaries"] = serde_json::to_value(summaries)?;
    if progressive.is_none() {
        data["prefixes"] = serde_json::to_value(integrals)?;
        data["bounds"] = serde_json::to_value(bounds)?;
        data["coverageCounts"] = serde_json::to_value(coverage_counts)?;
    }
    data["cumulativePending"] = json!(progressive.is_some());
    data["exactHours"] = json!(
        input
            .hours
            .iter()
            .copied()
            .chain([first, last])
            .collect::<BTreeSet<_>>()
    );
    data["basis"] = json!(basis);
    data["scoped"] = json!(input.membership.is_some());
    data["keyCount"] = json!(keys.len());
    Ok(Some(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::test_fixtures::{CAMPAIGN, records, resource_frame};
    use replay_data::rows::TableRows;
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::ResourceExchangeRow};
    use replay_frame::{frame::DecodedFrame, frame::TableView};

    #[test]
    fn trade_observer_work_is_flat_over_unchanged_hours_and_retired_flows() {
        use dashboard_base::prepared::{
            frame_changes::FrameChangeTracker,
            observer_tests::{self, Work},
        };
        let first = resource_frame(100, 0, 9001, &[(51, 21, 9001, 12)]);
        let format = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])])
            .unwrap()
            .format;
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |hours| {
                let mut prepared = Prepared::default();
                let mut tracker = FrameChangeTracker::new();
                prepared.observe_changed(&first, &tracker.observe(&first, format))?;
                observer_tests::unchanged_frames(
                    prepared.work,
                    &first,
                    hours as i32,
                    format,
                    |frame, changes| {
                        prepared.observe_changed(frame, changes)?;
                        Ok(prepared.work)
                    },
                )
            })
            .unwrap(),
        );
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |history| {
                let mut prepared = Prepared::default();
                let mut tracker = FrameChangeTracker::new();
                let mut frame = first.clone();
                for offset in 0..history {
                    frame.hour = 100 + offset as i32;
                    if let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *frame.tables[TableId::Countries.index()]
                    {
                        rows.iter_mut()
                            .find(|row| row.canonical_slot == 8)
                            .unwrap()
                            .id = 20_000 + offset as u32;
                    }
                    if let TableView::Complete(TableRows::CountryLinks(rows)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                    {
                        rows.iter_mut()
                            .find(|row| row.canonical_slot == 8)
                            .unwrap()
                            .id = 20_000 + offset as u32;
                    }
                    if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
                        &mut *frame.tables[TableId::ResourceExchanges.index()]
                    {
                        rows[0].exporter = 20_000 + offset as u32;
                    }
                    prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
                }
                let before = prepared.work;
                frame.hour += 1;
                if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
                    &mut *frame.tables[TableId::ResourceExchanges.index()]
                {
                    rows[0].delivered += 1;
                }
                prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
                assert_eq!(prepared.work.row_visits - before.row_visits, 1);
                assert_eq!(prepared.work.history_visits - before.history_visits, 1);
                Ok(Work {
                    row_visits: prepared.work.row_visits - before.row_visits,
                    history_visits: prepared.work.history_visits - before.history_visits,
                    emitted: prepared.work.emitted - before.emitted,
                    ..Default::default()
                })
            })
            .unwrap(),
        );
    }

    #[test]
    fn trade_current_projection_preserves_order_duplicates_unknowns_and_overflow_refusal() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let mut frame = resource_frame(
            100,
            0,
            9001,
            &[
                (51, 21, 9001, i64::MAX),
                (52, 21, 9001, -1),
                (53, 21, 9001, 1),
                (54, 0xdead, 9001, 7),
            ],
        );
        if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        {
            rows.push(replay_data::rows::ResourceExchangeRow {
                id: 55,
                exporter: 21,
                importer: 9001,
                resource: 999,
                ..Default::default()
            });
        }
        let format = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])])
            .unwrap()
            .format;
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        let run = prepared.scan.result.flows[0].runs.last().unwrap();
        assert_eq!(run.terms.delivered, i64::MAX);
        assert_eq!(run.terms.deals, 3);
        assert_eq!(prepared.scan.result.unnamed, 1);
        assert_eq!(prepared.scan.result.unresolved, 1);
        assert_eq!(
            sample::sample(&frame, None).unwrap(),
            sample::sample_inputs(&frame, None, &prepared.scan.inputs).unwrap()
        );
        let before = prepared.work.row_visits;
        frame.hour += 1;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.iter_mut()
                .find(|row| row.canonical_slot == 8)
                .unwrap()
                .tag = b"renamed trade seller".to_vec();
        }
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        assert_eq!(
            prepared.work.row_visits, before,
            "renaming resolves retained exchanges"
        );
        assert_eq!(
            prepared.scan.result.flows[0]
                .runs
                .last()
                .unwrap()
                .seller_tag,
            "renamed trade seller"
        );
        if let TableView::Complete(TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        {
            rows.swap(1, 2);
        }
        // MAX + 1 refuses before the later -1 can cancel it.
        assert_eq!(
            sample::sample(&frame, None).unwrap_err().to_string(),
            "recorded trade terms overflow their sum"
        );
        frame.hour += 1;
        assert_eq!(
            prepared
                .observe_changed(&frame, &tracker.observe(&frame, format))
                .unwrap_err()
                .to_string(),
            "recorded trade terms overflow their sum"
        );
    }

    #[test]
    fn trade_pages_preserve_signed_flows_gaps_aliases_and_integrals_after_resume_and_append() {
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (100..=110)
            .filter(|&hour| hour != 105)
            .map(|hour| {
                let mut frame = resource_frame(
                    hour,
                    0,
                    9001,
                    if hour < 104 {
                        &[(1, 21, 9001, -200), (2, 21, 9001, 50)]
                    } else if hour < 108 {
                        &[(3, 11, 31, 300)]
                    } else {
                        &[]
                    },
                );
                if hour == 103 {
                    *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                        reason: "synthetic missing deal read".into(),
                    };
                }
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                    && hour >= 107
                {
                    rows.iter_mut()
                        .find(|row| row.canonical_slot == 7)
                        .unwrap()
                        .tag = b"mod renamed holder".to_vec();
                }
                frame
            })
            .collect();
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "trade-prepared".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
        let history = country_history::scan_histories(&replay, 1, false, &[], &flag).unwrap();
        let expected = super::super::reply(
            &source,
            &TradeFlowParams {
                branch_id: "1".into(),
                tags: None,
            },
            &context,
            &scan(&replay, 1, None, &flag).unwrap(),
            &flag,
        )
        .unwrap();
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for frame in &frames {
            prepared
                .observe_changed(frame, &tracker.observe(frame, replay.format))
                .unwrap();
            if [101, 106, 110].contains(&frame.hour) {
                let mut writer = if frame.hour == 110 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                prepared
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                tracker.invalidate();
                assert!(
                    prepared
                        .scan
                        .result
                        .flows
                        .iter()
                        .all(|flow| flow.runs.len() <= 1)
                );
            }
        }
        let input = json!({"branchId":"1","tags":null,"from":100,"to":110,"basis":100});
        let actual = read_pages(
            &fixture.store,
            &fixture.key,
            1,
            serde_json::from_value(input.clone()).unwrap(),
            input,
            &history,
            110,
            true,
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        let actual_reply: TradeFlowsReply = serde_json::from_value(actual.clone()).unwrap();
        assert_eq!(actual_reply.flows, expected.flows);
        assert_eq!(actual_reply.countries, expected.countries);
        assert_eq!(actual_reply.known, expected.known);
        assert_eq!(actual_reply.reads, expected.reads);
        assert_eq!(actual_reply.partial, expected.partial);
        assert_eq!(actual_reply.warnings, expected.warnings);
        for flow in &expected.flows {
            let key = serde_json::to_string(&(
                flow.seller_identity.clone(),
                flow.buyer_identity.clone(),
                flow.resource,
            ))
            .unwrap();
            let total: i128 = flow
                .delivered_raw
                .iter()
                .enumerate()
                .map(|(index, raw)| {
                    raw.parse::<i128>().unwrap()
                        * i128::from(flow.last_hours[index] - flow.first_hours[index] + 1)
                })
                .sum();
            assert_eq!(actual["prefixes"][&key]["110"][0], total.to_string());
        }
        assert_eq!(prepared.keys.len(), 2);
    }

    #[cfg(feature = "slow-tests")]
    #[test]
    fn trade_preparation_allocates_only_5000_observed_flow_keys_for_5000_countries() {
        let mut frame = resource_frame(100, 0, 9001, &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("mod country {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=5000)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ResourceExchanges.index()] =
            TableView::Complete(TableRows::ResourceExchanges(
                (1..=5000)
                    .map(|id| ResourceExchangeRow {
                        id: u64::from(id),
                        exporter: id,
                        importer: id % 5000 + 1,
                        resource: 1,
                        delivered: i64::from(id),
                        ..Default::default()
                    })
                    .collect(),
            ));
        let mut prepared = Prepared::default();
        let sampled = sample::sample(&frame, None).unwrap();
        prepared.observe(&frame).unwrap();
        assert_eq!(
            sampled.len(),
            30_002,
            "five fields per actual flow plus holders and read metadata"
        );
        for flow in &prepared.scan.result.flows {
            let token = flow_key(flow);
            let id = prepared.keys[&catalogue_key(flow)];
            let terms = flow.runs.last().unwrap().terms;
            for (field, expected) in sample::FIELDS.iter().zip(sample::amounts(terms)) {
                assert_eq!(
                    sampled[&(token.clone(), (*field).to_owned())],
                    (Some(flow.seller_identity), Some(expected))
                );
            }
            assert_eq!(
                prepared.values.state(id).unwrap().last.unwrap().value,
                sampled[&(token, "delivered".into())].1.unwrap()
            );
            assert_eq!(
                sampled[&(flow.seller.clone(), "holder".into())].0,
                Some(flow.seller_identity)
            );
        }
        let query = sample::Query::default();
        let format = replay_data::format::FormatVersion::default();
        let actual = query.sample(&frame, None, format).unwrap();
        assert_eq!(actual, sampled);
        let before = query.visits();
        let mut later = frame.clone();
        later.hour += 1;
        assert_eq!(query.sample(&later, None, format).unwrap(), sampled);
        assert_eq!(
            query.visits(),
            before,
            "reconstructed unchanged native trade inputs"
        );
        *later.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
            reason: "synthetic refusal".into(),
        };
        assert_eq!(
            query.sample(&later, None, format).unwrap(),
            sample::sample(&later, None).unwrap()
        );
        assert_eq!(prepared.keys.len(), 5000);
        assert_eq!(prepared.scan.result.flows.len(), 5000);
        prepared.materialize_integrals(frame.hour).unwrap();
        assert!(
            prepared
                .totals
                .iter()
                .all(|values| values.state(4999).is_some())
        );
        assert_eq!(prepared.refusal_keys.len(), 0);
        assert_eq!(prepared.pattern_keys.len(), 0);
        frame.hour = 101;
        prepared.observe(&frame).unwrap();
        eprintln!(
            "trade scale: 5000 countries, {} actual-flow keys, {} refusal keys, {} partial-pattern keys; {} continuation bytes",
            prepared.keys.len(),
            prepared.refusal_keys.len(),
            prepared.pattern_keys.len(),
            serde_json::to_vec(&prepared).unwrap().len()
        );
        assert!(
            prepared
                .scan
                .result
                .flows
                .iter()
                .all(|flow| flow.runs.len() == 1)
        );
    }

    #[test]
    fn trade_dense_average_envelope_bounds_a_ratio_peak_before_the_final_cumulative_extremum() {
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (100..=119)
            .map(|hour| {
                resource_frame(
                    hour,
                    0,
                    9001,
                    &[(1, 21, 9001, if hour == 102 { 1_000_000 } else { 1_000 })],
                )
            })
            .collect();
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "trade-average".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let history = country_history::scan_histories(&replay, 1, false, &[], &flag).unwrap();
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        let mut prepared = Prepared::default();
        for frame in &frames {
            prepared.observe(frame).unwrap();
        }
        prepared
            .flush(
                &mut writer,
                &source,
                &LoadContext { content: None },
                1,
                &flag,
            )
            .unwrap();
        writer.commit().unwrap();
        let input = json!({"branchId":"1","tags":null,"from":100,"to":119,"basis":100,"width":1,"measure":"average"});
        let actual = read_pages(
            &fixture.store,
            &fixture.key,
            1,
            serde_json::from_value(input.clone()).unwrap(),
            input,
            &history,
            119,
            true,
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        let maximum = actual["summaries"]
            .as_object()
            .unwrap()
            .values()
            .flat_map(|summary| summary["buckets"].as_array().unwrap())
            .filter_map(|bucket| bucket["maxValue"].as_f64())
            .max_by(f64::total_cmp)
            .unwrap();
        assert!(
            maximum >= 1_002_000f64 / 3.0 / 100_000.0,
            "the bucket must cover the earlier running-average peak"
        );
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn trade_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
        use dashboard_base::test_fixtures::file_source;
        use dashboard_core::test_fixtures::continuation_records;
        let frame = |hour| {
            let germany = if hour == 11 { 9002 } else { 9001 };
            let deals = if [8, 9].contains(&hour) {
                Vec::new()
            } else {
                vec![(
                    1,
                    21,
                    germany,
                    if hour % 2 == 0 {
                        i64::from(hour) * 100_000
                    } else {
                        900_000
                    },
                )]
            };
            let mut frame = resource_frame(hour, 0, germany, &deals);
            if hour >= 19
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut()
                    .find(|country| country.canonical_slot == 7)
                    .unwrap()
                    .tag = b"mod renamed".to_vec();
            }
            if hour == 13 {
                *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                    reason: "synthetic missing deals".into(),
                };
            }
            frame
        };
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame(0), frame(1)])]).unwrap();
        for block in 1..15 {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    vec![frame(block * 2), frame(block * 2 + 1)],
                )])
                .unwrap();
        }
        let (source, root) = file_source(replay);
        let flag = AtomicBool::new(false);
        let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
        let input = |from| json!({"branchId":"1","tags":null,"from":from,"to":29,"width":30,"hours":[11,19,29],"measure":"current"});
        let token = flow_token((8, 21), (7, 9001), 1);
        let cold = query(&source, input(0), None, &flag).unwrap().unwrap();
        assert_eq!(
            cold["summaries"][&token]["approximate"],
            json!([{"firstHour":0,"lastHour":29}])
        );
        assert_eq!(cold["cumulativePending"], true);
        assert!(
            query(
                &source,
                json!({"branchId":"1","tags":null,"from":0,"to":29,"requireComplete":true}),
                None,
                &flag
            )
            .unwrap()
            .is_none(),
            "parity cannot certify approximate coverage"
        );
        assert!(
            cold["prefixes"].is_null()
                && cold["bounds"].is_null()
                && cold["coverageCounts"].is_null()
        );
        assert!(store.saved_key().unwrap().is_none());
        let cold_reply: TradeFlowsReply = serde_json::from_value(cold).unwrap();
        let old = cold_reply
            .flows
            .iter()
            .find(|flow| flow.buyer_identity == "[7,9001]")
            .unwrap();
        let current = old.last_hours.iter().position(|&hour| hour == 29).unwrap();
        assert_eq!(
            old.delivered_raw[current], "900000",
            "written amount must use exact state, not the final keyframe"
        );
        assert!(
            cold_reply
                .country_identities
                .iter()
                .find(|country| country.tag == "GER")
                .unwrap()
                .runs
                .iter()
                .any(|run| run.first_hour == 11 && run.last_hour == 11 && run.country_id == 9002)
        );
        assert!(
            cold_reply
                .country_identities
                .iter()
                .any(|country| country.tag == "mod renamed")
        );
        let renamed = query(
            &source,
            json!({"branchId":"1","tags":["GER"],"from":26,"to":29,"width":4,"hours":[29]}),
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        let renamed: TradeFlowsReply = serde_json::from_value(renamed).unwrap();
        let flow = renamed
            .flows
            .iter()
            .find(|flow| flow.buyer_identity == "[7,9001]")
            .unwrap();
        let at = flow.last_hours.iter().position(|&hour| hour == 29).unwrap();
        assert_eq!(
            renamed.countries[flow.buyer_tags[at] as usize], "mod renamed",
            "a cold late zoom follows the original tag's native holder"
        );
        assert_eq!(flow.delivered_raw[at], "900000");
        for measure in ["total", "average"] {
            assert!(
                query(
                    &source,
                    json!({"branchId":"1","tags":null,"from":0,"to":29,"measure":measure}),
                    None,
                    &flag
                )
                .unwrap()
                .is_none()
            );
        }
        let mut prepared = Prepared::default();
        let mut identities = country_history::HistoryScan::new(false, &[]);
        for hour in 0..=1 {
            let frame = frame(hour);
            prepared.observe(&frame).unwrap();
            identities.observe(&frame);
        }
        let mut writer = store.replace(key.clone()).unwrap();
        prepared
            .flush(
                &mut writer,
                &source,
                &LoadContext { content: None },
                1,
                &flag,
            )
            .unwrap();
        writer
            .write_at(
                "country-identities-1",
                0,
                serde_json::to_vec(&identities).unwrap().as_slice(),
            )
            .unwrap();
        writer
            .write_at(
                "preparation-coverage-1",
                0,
                serde_json::to_vec(&dashboard_base::prepared::coverage::Coverage {
                    schema: dashboard_base::prepared::coverage::SCHEMA,
                    first_hour: 0,
                    last_hour: 1,
                    complete: false,
                })
                .unwrap()
                .as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let ranges = store.range_count(&key, "trade-values-1").unwrap();
        let rough = query(&source, input(0), None, &flag).unwrap().unwrap();
        assert_eq!(
            rough["summaries"][&token]["approximate"],
            json!([{"firstHour":2,"lastHour":29}])
        );
        assert_eq!(rough["coverage"]["lastHour"], 1);
        let exact = query(&source, input(4), None, &flag).unwrap().unwrap();
        let summary: dashboard_base::prepared_chart::ChartSummary =
            serde_json::from_value(exact["summaries"][&token].clone()).unwrap();
        assert!(
            summary.approximate.is_empty(),
            "thirteen blocks must be decoded exactly"
        );
        for hour in 4..=29 {
            let sampled =
                sample::sample(&frame(hour), Some(&[(token.clone(), "delivered".into())])).unwrap();
            let expected = sampled[&(token.clone(), "delivered".into())]
                .1
                .map(|value| value as f64 / 100_000.0);
            // A successful census with a different holder leaves this earlier flow at zero.
            let expected = if hour == 11 { Some(0.0) } else { expected };
            let actual = summary
                .runs
                .as_ref()
                .unwrap()
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .map(|run| run.value);
            assert_eq!(actual, expected, "exact geometry at {hour}");
        }
        let renamed = query(
            &source,
            json!({"branchId":"1","tags":["GER"],"from":26,"to":29,"width":4,"hours":[29]}),
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        let renamed: TradeFlowsReply = serde_json::from_value(renamed).unwrap();
        assert!(
            renamed
                .flows
                .iter()
                .any(|flow| flow.buyer_identity == "[7,9001]" && flow.last_hours.contains(&29)),
            "prepared identities seed manual native-holder selection"
        );
        let member = query(&source, json!({"branchId":"1","tags":["ITA"],"membership":{"ITA":[{"firstHour":6,"lastHour":9}]},"from":0,"to":29,"width":2}), None, &flag).unwrap().unwrap();
        assert_eq!(
            member["summaries"][&token]["approximate"],
            json!([{"firstHour":6,"lastHour":9}])
        );
        assert_eq!(store.range_count(&key, "trade-values-1").unwrap(), ranges);
        let point = query(
            &source,
            json!({"branchId":"1","tags":null,"from":11,"to":11,"width":1,"hours":[29]}),
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        let point: TradeFlowsReply = serde_json::from_value(point).unwrap();
        assert!(
            point
                .flows
                .iter()
                .find(|flow| flow.buyer_identity == "[7,9001]")
                .unwrap()
                .last_hours
                .contains(&29),
            "out-of-viewport requests retain exact native operands"
        );
        store.clear().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    fn exchanges(frame: &mut DecodedFrame) -> &mut Vec<ResourceExchangeRow> {
        match &mut *frame.tables[TableId::ResourceExchanges.index()] {
            TableView::Complete(TableRows::ResourceExchanges(rows)) => rows,
            _ => unreachable!("fixture deals are complete"),
        }
    }
    fn deal(frame: &mut DecodedFrame, id: u64) -> &mut ResourceExchangeRow {
        exchanges(frame)
            .iter_mut()
            .find(|row| row.id == id)
            .expect("fixture deal")
    }
    fn add_deal(frame: &mut DecodedFrame, row: ResourceExchangeRow) {
        let rows = exchanges(frame);
        rows.push(row);
        rows.sort_by_key(|row| row.id);
    }
    fn country(frame: &mut DecodedFrame, id: u32) -> &mut CountryRow {
        match &mut *frame.tables[TableId::Countries.index()] {
            TableView::Complete(TableRows::Countries(rows)) => rows
                .iter_mut()
                .find(|row| row.id == id)
                .expect("fixture country"),
            _ => unreachable!("fixture countries are complete"),
        }
    }
    /// A steel deal from `exporter`'s link to `importer`'s.
    fn steel(id: u64, exporter: u32, importer: u32, delivered: i64) -> ResourceExchangeRow {
        ResourceExchangeRow {
            id,
            exporter,
            importer,
            resource: 1,
            delivered,
            assigned_factories: 1,
            required_factories: 1,
            ..Default::default()
        }
    }

    /// The recording codec: each input hour is encoded and decoded as the
    /// preparation pipeline reads it, with its recorded updates.
    struct Recorded {
        encoder: replay_frame::frame::FrameEncoder,
        decoder: replay_frame::frame::FrameDecoder,
    }
    impl Recorded {
        fn new() -> Self {
            Self {
                encoder: replay_frame::frame::FrameEncoder::new(),
                decoder: replay_frame::frame::FrameDecoder::new(),
            }
        }
        fn next(
            &mut self,
            input: &DecodedFrame,
            keyframe: bool,
        ) -> (DecodedFrame, Arc<replay_frame::frame::Frame>) {
            let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
            let decoded = self.decoder.advance(&wire).unwrap().clone();
            (decoded, Arc::new(wire))
        }
    }

    /// `count` countries, each selling steel to the next.
    fn trade_world(count: u32) -> DecodedFrame {
        let mut frame = resource_frame(100, 0, 9001, &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=count)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("mod country {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                (1..=count)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ResourceExchanges.index()] =
            TableView::Complete(TableRows::ResourceExchanges(
                (1..count)
                    .map(|id| steel(u64::from(id), id, id + 1, i64::from(id) * 100))
                    .collect(),
            ));
        frame
    }

    fn assert_rows_observe_only_changed_flows(count: u32) {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut recorded = Recorded::new();
        let mut tracker = FrameChangeTracker::new();
        // Each hour's exchange rows read, flows published, values written,
        // countries compared, and whether it was observed from changed rows.
        let mut step = |prepared: &mut Prepared, frame: &DecodedFrame| {
            let before = (
                prepared.scan.inputs.row_visits,
                prepared.work.history_visits,
                prepared.work.emitted,
                prepared.scan.inputs.country_visits,
                prepared.row_hours,
            );
            let (decoded, wire) = recorded.next(frame, false);
            let changes = tracker.observe_updates(&decoded, wire, format);
            prepared.observe_changed(&decoded, &changes).unwrap();
            (
                prepared.scan.inputs.row_visits - before.0,
                prepared.work.history_visits - before.1,
                prepared.work.emitted - before.2,
                prepared.scan.inputs.country_visits - before.3,
                prepared.row_hours - before.4,
            )
        };
        let flows = count as usize - 1;
        let mut prepared = Prepared::default();
        let mut frame = trade_world(count);
        let (_, published, ..) = step(&mut prepared, &frame);
        assert_eq!(published, flows, "the first hour publishes every flow");

        // A recorded field no flow reads.
        frame.hour = 101;
        deal(&mut frame, 1).efficiency_a += 7;
        let (_, published, emitted, compared, rows) = step(&mut prepared, &frame);
        assert_eq!(rows, 1, "the hour is observed from its changed rows");
        assert_eq!((published, emitted, compared), (0, 0, 0));

        frame.hour = 102;
        deal(&mut frame, u64::from(count / 2)).delivered += 1;
        let extensions = prepared.scan.flow_extensions;
        assert_eq!(
            step(&mut prepared, &frame),
            (1, 1, 2, 0, 1),
            "only the changed deal is read and only its flow publishes its start and amount"
        );
        assert_eq!(
            prepared.scan.flow_extensions - extensions,
            1,
            "other flows keep their deferred endpoints"
        );

        // Its seller's own fields change too: no country is compared.
        frame.hour = 103;
        country(&mut frame, count / 3).political_power_raw = Some(7);
        deal(&mut frame, u64::from(count / 3)).delivered += 1;
        assert_eq!(step(&mut prepared, &frame), (1, 1, 2, 0, 1));

        // Fields no flow and no holder reads.
        frame.hour = 104;
        deal(&mut frame, u64::from(count / 4)).efficiency_b += 1;
        country(&mut frame, count / 4).political_power_raw = Some(9);
        assert_eq!(step(&mut prepared, &frame), (0, 0, 0, 0, 1));

        // An ended deal ends only its flow.
        frame.hour = 105;
        exchanges(&mut frame).retain(|row| row.id != u64::from(count / 5));
        assert_eq!(step(&mut prepared, &frame), (1, 1, 2, 0, 1));

        // A new deal starts only its flow.
        frame.hour = 106;
        add_deal(
            &mut frame,
            ResourceExchangeRow {
                resource: 2,
                ..steel(u64::from(count) + 1, 1, count, 5)
            },
        );
        assert_eq!(step(&mut prepared, &frame), (1, 1, 2, 0, 1));

        // Countries alone changing: the flows only continue.
        frame.hour = 107;
        country(&mut frame, 1).political_power_raw = Some(11);
        assert_eq!(step(&mut prepared, &frame), (0, 0, 0, 0, 1));
        assert_eq!(
            prepared.scan.active.len(),
            flows,
            "one flow ended and one started"
        );
    }

    #[test]
    fn trade_hour_changing_one_deal_reads_and_publishes_only_its_flow() {
        assert_rows_observe_only_changed_flows(200);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn trade_hour_changing_one_of_5000_countries_deals_publishes_only_its_flow() {
        assert_rows_observe_only_changed_flows(5000);
    }

    #[test]
    fn trade_operand_masks_name_every_field_they_read() {
        assert_eq!(EXCHANGE_FIELDS.count_ones(), 7);
        assert_eq!(HOLDER_FIELDS.count_ones(), 2);
        let efficiency = (0..64)
            .position(|field| TableId::ResourceExchanges.field_name(field) == Some("efficiency_a"))
            .unwrap();
        assert_eq!(
            *EXCHANGE_FIELDS & (1 << efficiency),
            0,
            "no flow reads a deal's efficiency"
        );
    }

    #[test]
    fn trade_rows_observation_matches_whole_tables_and_scan_across_keyframe_gap_and_resume() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        use replay_data::{rows::LexicalTokenRow, rows::ResourceDefinitionRow};
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut frame = resource_frame(100, 0, 9001, &[(51, 21, 9001, 12), (52, 11, 31, 5)]);
        // A fourth country, renamed later.
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.push(CountryRow {
                id: 41,
                canonical_slot: 10,
                tag: b"FRA".to_vec(),
                ..Default::default()
            });
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 41,
                canonical_slot: 10,
            });
            rows.sort_by_key(|row| row.id);
        }
        let oil = |deal: ResourceExchangeRow| ResourceExchangeRow {
            resource: 2,
            ..deal
        };
        let mut inputs = Vec::new();
        let mut stashed = None;
        for hour in 100..=140 {
            frame.hour = hour;
            match hour {
                102 | 123 | 128 | 133 | 134 | 140 => deal(&mut frame, 51).delivered += 1,
                // A recorded field no flow reads.
                103 => deal(&mut frame, 52).efficiency_a += 7,
                // A second deal of one flow: the flow sums both.
                104 => add_deal(&mut frame, steel(53, 21, 9001, 9)),
                105 => deal(&mut frame, 53).delivered += 1,
                106 => deal(&mut frame, 52).resource = 2,
                // A country field no holder reads.
                107 | 131 => country(&mut frame, 21).political_power_raw = Some(i64::from(hour)),
                108 => {
                    country(&mut frame, 31).political_power_raw = Some(108);
                    deal(&mut frame, 51).convoys_assigned = 2;
                }
                // A token no definition names: every resource keeps its name.
                109 => {
                    if let TableView::Complete(TableRows::LexicalTokens(rows)) =
                        &mut *frame.tables[TableId::LexicalTokens.index()]
                    {
                        rows.push(LexicalTokenRow {
                            token: 503,
                            literal: b"rubber".to_vec(),
                        });
                    }
                }
                110 => deal(&mut frame, 52).exporter = 41,
                111 => exchanges(&mut frame).retain(|row| row.id != 51),
                112 => exchanges(&mut frame).retain(|row| row.id != 53),
                113 => add_deal(&mut frame, steel(51, 21, 9001, 20)),
                // A seller the links do not resolve: the read leaves it out.
                114 => add_deal(&mut frame, steel(54, 0xdead, 9001, 3)),
                115 => deal(&mut frame, 54).delivered += 1,
                116 => deal(&mut frame, 54).exporter = 31,
                // A resource the definitions do not name.
                117 => deal(&mut frame, 54).resource = 999,
                // A new resource: the names change.
                118 => {
                    if let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
                        &mut *frame.tables[TableId::ResourceDefinitions.index()]
                    {
                        rows.push(ResourceDefinitionRow {
                            id: 3,
                            token: 503,
                            active: 1,
                            factory_cost: 12_500,
                        });
                    }
                }
                119 => deal(&mut frame, 54).resource = 3,
                120 => country(&mut frame, 41).tag = b"VIC".to_vec(),
                121 | 125 => deal(&mut frame, 52).delivered += 1,
                124 => continue,
                126 => {
                    stashed = Some(std::mem::take(exchanges(&mut frame)));
                    *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                        reason: "synthetic missing deals".into(),
                    };
                }
                127 => {
                    *frame.tables[TableId::ResourceExchanges.index()] =
                        TableView::Complete(TableRows::ResourceExchanges(stashed.take().unwrap()));
                    deal(&mut frame, 54).delivered += 1;
                }
                129 => {
                    deal(&mut frame, 52).convoys_required = 4;
                    deal(&mut frame, 54).importer = 41;
                }
                130 => {
                    exchanges(&mut frame).retain(|row| row.id != 52);
                    add_deal(&mut frame, oil(steel(55, 11, 21, 4)));
                }
                132 => {
                    deal(&mut frame, 51).delivered += 1;
                    deal(&mut frame, 55).delivered += 1;
                    country(&mut frame, 31).political_power_raw = Some(132);
                }
                // Germany's other link sells oil to Italy too: one flow of
                // two links' deals.
                135 => add_deal(&mut frame, oil(steel(56, 9001, 21, 6))),
                136 => deal(&mut frame, 55).delivered = -4,
                137 => exchanges(&mut frame).retain(|row| row.id != 55),
                // Italy sells to itself.
                139 => deal(&mut frame, 56).exporter = 21,
                _ => {}
            }
            inputs.push((frame.clone(), hour == 115));
        }
        let replay = Arc::new(
            Replay::from_records(
                CAMPAIGN.into(),
                &[records(
                    1,
                    inputs.iter().map(|(frame, _)| frame.clone()).collect(),
                )],
            )
            .unwrap(),
        );
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "trade-rows".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
        let history = country_history::scan_histories(&replay, 1, false, &[], &flag).unwrap();
        let pages = |fixture: &dashboard_base::prepared::series::tests::Fixture,
                     last: i32,
                     width: Option<u32>| {
            let mut input = json!({"branchId":"1","tags":null,"from":100,"to":last,"basis":100});
            if let Some(width) = width {
                input["width"] = json!(width);
            }
            read_pages(
                &fixture.store,
                &fixture.key,
                1,
                serde_json::from_value(input.clone()).unwrap(),
                input,
                &history,
                last,
                true,
                None,
                &flag,
            )
            .unwrap()
            .unwrap()
        };
        let fixtures = [
            dashboard_base::prepared::series::tests::Fixture::new(),
            dashboard_base::prepared::series::tests::Fixture::new(),
        ];
        // Observed from changed rows, and from the whole tables every hour.
        let mut observers = [
            Prepared::default(),
            Prepared {
                whole_tables: true,
                ..Default::default()
            },
        ];
        let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
        let mut recorded = Recorded::new();
        for (input, keyframe) in &inputs {
            let (decoded, wire) = recorded.next(input, *keyframe);
            for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
                let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
                observer.observe_changed(&decoded, &changes).unwrap();
            }
            // Publication materializes open native endpoints. Compare that
            // canonical state, including every untouched flow's endpoint.
            for observer in &mut observers {
                observer.scan.seal();
            }
            let [changed, whole] = &observers;
            assert_eq!(
                serde_json::to_value(changed).unwrap(),
                serde_json::to_value(whole).unwrap(),
                "observer state after hour {}",
                decoded.hour
            );
            assert_eq!(
                changed.bytes(),
                whole.bytes(),
                "pending publication after hour {}",
                decoded.hour
            );
            if ![110, 121, 135, 140].contains(&decoded.hour) {
                continue;
            }
            for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
                let mut writer = fixture.writer();
                observer
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            for width in [None, Some(4)] {
                assert_eq!(
                    pages(&fixtures[0], decoded.hour, width),
                    pages(&fixtures[1], decoded.hour, width),
                    "published through hour {}",
                    decoded.hour
                );
            }
            // Resume both from their checkpoints.
            for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
                let (whole_tables, row_hours) = (observer.whole_tables, observer.row_hours);
                *observer =
                    serde_json::from_slice(&serde_json::to_vec(&*observer).unwrap()).unwrap();
                observer.whole_tables = whole_tables;
                observer.row_hours = row_hours;
                tracker.invalidate();
            }
        }
        assert!(
            observers[0].row_hours >= 20,
            "most hours are observed from their changed rows, not {}",
            observers[0].row_hours
        );
        assert_eq!(observers[1].row_hours, 0);
        let expected = super::super::reply(
            &source,
            &TradeFlowParams {
                branch_id: "1".into(),
                tags: None,
            },
            &context,
            &scan(&replay, 1, None, &flag).unwrap(),
            &flag,
        )
        .unwrap();
        let actual: TradeFlowsReply =
            serde_json::from_value(pages(&fixtures[0], 140, None)).unwrap();
        assert_eq!(actual.flows, expected.flows);
        assert_eq!(actual.countries, expected.countries);
        assert_eq!(actual.known, expected.known);
        assert_eq!(actual.reads, expected.reads);
        assert_eq!(actual.partial, expected.partial);
        assert_eq!(actual.warnings, expected.warnings);
    }

    fn assert_deferred_trade_integrals(count: u32) {
        use dashboard_base::prepared::{
            frame_changes::FrameChangeTracker,
            series::tests::{Fixture, assert_payloads_equal},
        };
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut input = trade_world(count);
        add_deal(&mut input, steel(u64::from(count), count, 1, 7));
        let mut frames = Vec::new();
        for hour in (100..=140).filter(|hour| !(117..120).contains(hour)) {
            input.hour = hour;
            match hour {
                105 => deal(&mut input, 1).delivered = -137,
                107 => {
                    let row = deal(&mut input, 1);
                    row.assigned_factories = -2;
                    row.convoys_assigned = 3;
                    row.convoys_required = 9;
                }
                110 => exchanges(&mut input).retain(|row| row.id != u64::from(count / 2)),
                125 => add_deal(
                    &mut input,
                    steel(u64::from(count / 2), count / 2, count / 2 + 1, 0),
                ),
                129 => add_deal(&mut input, steel(u64::from(count) + 1, 1, 2, 23)),
                134 => exchanges(&mut input).retain(|row| row.id != u64::from(count) + 1),
                132 => country(&mut input, 2).tag = b"changed alias".to_vec(),
                135 => deal(&mut input, 1).delivered = 59,
                _ => {}
            }
            let mut frame = input.clone();
            if hour == 126 {
                *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                    reason: "synthetic refused trade".into(),
                };
            }
            frames.push(frame);
        }
        let split = frames.partition_point(|frame| frame.hour <= 116);
        let first =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..split].to_vec())]).unwrap();
        let replay = Arc::new(
            first
                .append(&[dashboard_base::test_fixtures::continuation_records(
                    &first,
                    1,
                    frames[split..].to_vec(),
                )])
                .unwrap(),
        );
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "trade-integral-spans".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &flag,
        )
        .unwrap();
        let mut fixtures = [Fixture::new(), Fixture::new()];
        let mut states = [
            Prepared::default(),
            Prepared {
                hourly_integrals: true,
                ..Default::default()
            },
        ];
        let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
        let mut recorded = Recorded::new();
        let context = LoadContext { content: None };
        for frame in &frames {
            let (decoded, wire) = recorded.next(frame, [100, 108, 120, 131].contains(&frame.hour));
            let mut visits = [0; 2];
            for (index, (state, tracker)) in states.iter_mut().zip(&mut trackers).enumerate() {
                let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
                let before = state.integral_visits;
                state.observe_changed(&decoded, &changes).unwrap();
                visits[index] = state.integral_visits - before;
            }
            assert_eq!(
                states[0].bytes(),
                states[1].bytes(),
                "hourly budget at {}",
                decoded.hour
            );
            if decoded.hour == 101 {
                assert_eq!(
                    visits,
                    [0, count as usize],
                    "unchanged flows require no hourly integration"
                );
            }
            if decoded.hour == 105 {
                assert_eq!(
                    visits,
                    [1, count as usize],
                    "only the changed rate is materialized"
                );
            }
            if ![109, 116, 130, 140].contains(&decoded.hour) {
                continue;
            }
            for (state, fixture) in states.iter_mut().zip(&mut fixtures) {
                let mut writer = if decoded.hour > 116 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                state
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            assert_payloads_equal(
                &fixtures[0].payloads(),
                &fixtures[1].payloads(),
                "deferred trade integrals",
            );
            assert_eq!(
                serde_json::to_vec(&states[0]).unwrap(),
                serde_json::to_vec(&states[1]).unwrap(),
                "continuations through {}",
                decoded.hour
            );
            for (index, (state, tracker)) in states.iter_mut().zip(&mut trackers).enumerate() {
                *state = serde_json::from_slice(&serde_json::to_vec(&*state).unwrap()).unwrap();
                state.hourly_integrals = index == 1;
                tracker.invalidate();
            }
        }
    }

    #[test]
    fn deferred_trade_integrals_match_hourly_bytes_across_rates_gaps_restore_and_append() {
        assert_deferred_trade_integrals(32);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn unchanged_5000_trade_flows_do_no_hourly_integral_work_and_keep_bytes() {
        assert_deferred_trade_integrals(5_000);
    }

    fn assert_trade_budget_publications(count: u32, held: usize) {
        use dashboard_base::prepared::{
            frame_changes::FrameChangeTracker,
            pages::publication_due,
            series::tests::{Fixture, assert_payloads_equal},
        };
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut input = trade_world(count);
        add_deal(&mut input, steel(u64::from(count), count, 1, 7));
        for row in exchanges(&mut input) {
            row.convoys_assigned = 3;
            row.convoys_required = 9;
        }
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "trade-publication-budget".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(
                Replay::from_records(CAMPAIGN.into(), &[records(1, vec![input.clone()])]).unwrap(),
            ),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
        let mut fixtures = [Fixture::new(), Fixture::new()];
        let mut states = [
            Prepared::default(),
            Prepared {
                hourly_integrals: true,
                ..Default::default()
            },
        ];
        let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
        let mut recorded = Recorded::new();
        let mut publications: [Vec<i32>; 2] = Default::default();
        for hour in 100..=137 {
            input.hour = hour;
            let (decoded, wire) = recorded.next(&input, hour == 100);
            for (state, tracker) in states.iter_mut().zip(&mut trackers) {
                let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
                state.observe_changed(&decoded, &changes).unwrap();
            }
            // Block 2 publishes at 101. Block 3 has 34 unchanged accepted
            // hours; block 4 continues it. Each side chooses its own commits.
            if ![101, 135, 137].contains(&hour) {
                continue;
            }
            for (index, (state, fixture)) in states.iter_mut().zip(&mut fixtures).enumerate() {
                if !publication_due(hour != 137, false, hour == 101, || (state.bytes(), held)) {
                    continue;
                }
                publications[index].push(hour);
                let mut writer = if hour > 101 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                state
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
                // Exercise a resumed continuation without imposing its next
                // publication hour on either implementation.
                *state = serde_json::from_slice(&serde_json::to_vec(&*state).unwrap()).unwrap();
                state.hourly_integrals = index == 1;
                trackers[index].invalidate();
            }
        }
        assert_eq!(publications[1], [101, 137]);
        assert_eq!(
            publications[0], publications[1],
            "budget-selected publication hours"
        );
        assert_payloads_equal(
            &fixtures[0].payloads(),
            &fixtures[1].payloads(),
            "budget-selected trade pages and ranges",
        );
        assert_eq!(
            serde_json::to_vec(&states[0]).unwrap(),
            serde_json::to_vec(&states[1]).unwrap()
        );
    }

    #[test]
    fn deferred_trade_uses_hourly_publication_budget() {
        // A smaller version of the 5,000-flow boundary, with prior held pages
        // taking the rest of the real 64 MiB budget. Leave room for the hourly
        // coverage clocks, but less than the old cache/prefix overcharge.
        let staircase = 32 * 34 * 6 * size_of::<series::Run>() * 2;
        let held = dashboard_base::prepared::pages::BATCH_BYTES - staircase - 16_384;
        assert_trade_budget_publications(32, held);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn deferred_5000_trade_flows_use_hourly_publication_budget() {
        assert_trade_budget_publications(5_000, 0);
    }
}
