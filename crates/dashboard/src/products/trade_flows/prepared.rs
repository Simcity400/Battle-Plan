//! The shared trade step feeds sparse actual-flow facts and cumulative endpoints.
use super::*;
use crate::prepared::frame_changes::FrameChanges;
use crate::prepared::query::progressive::{self, AdapterTail};
use crate::prepared::series::{self, Accumulator, Reader};
use serde_json::{Value, json};

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Prepared {
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
    #[cfg(test)]
    #[serde(skip)]
    work: crate::prepared::observer_tests::Work,
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
    pub(crate) fn observe(&mut self, frame: &replay::frame::DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
            },
        )
    }
    pub(crate) fn observe_changed(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let continuous =
            self.scan.last_hour.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let changed = self.scan.inputs.refresh(frame, changes) || !continuous;
        for values in self.totals.iter_mut().chain(self.coverage.iter_mut()) {
            values.advance(frame.hour)?;
        }
        self.facts.advance(frame.hour)?;
        self.values.advance(frame.hour)?;
        self.refusal_counts.advance(frame.hour)?;
        self.pattern_totals.advance(frame.hour)?;
        if changed {
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
                        .observe(id, frame.hour, value.map(i128::from), false);
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
            let touched: BTreeSet<_> = previous.union(&self.scan.active).copied().collect();
            for index in touched {
                #[cfg(test)]
                {
                    self.work.history_visits += 1;
                }
                let flow = &mut self.scan.result.flows[index];
                let next = self.keys.len() as u64;
                let id = *self.keys.entry(catalogue_key(flow)).or_insert(next);
                let run = self
                    .scan
                    .active
                    .contains(&index)
                    .then(|| flow.runs.last().expect("active trade run"));
                if let Some(run) = run {
                    self.aliases
                        .extend([run.seller_tag.clone(), run.buyer_tag.clone()]);
                    if run.first_hour == frame.hour {
                        self.facts.event(id, frame.hour, run)?;
                    }
                }
                let fact = run.map(|run| i128::from(run.first_hour));
                let daily = run.map(|run| i128::from(run.terms.delivered));
                for (values, amount) in [(&mut self.facts, fact), (&mut self.values, daily)] {
                    if !continuous
                        || !previous.contains(&index)
                        || values
                            .state(id)
                            .and_then(|state| state.last)
                            .is_none_or(|last| Some(last.value) != amount)
                        || amount.is_none()
                    {
                        values.observe(id, frame.hour, amount, false);
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
        } else {
            self.scan.advance(frame.hour);
        }
        // These native integrals genuinely change at every accepted known hour.
        // Retired flows and refusal patterns carry through the shared frontier.
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
        for &index in &self.scan.active {
            let flow = &self.scan.result.flows[index];
            let id = self.keys[&catalogue_key(flow)];
            let run = flow.runs.last().expect("current native trade terms");
            let amounts = sample::amounts(run.terms).into_iter().chain([1]);
            for (values, amount) in self.totals.iter_mut().zip(amounts) {
                if amount == 0 && continuous && values.state(id).is_some() {
                    continue;
                }
                let before = values
                    .state(id)
                    .and_then(|state| state.last)
                    .map_or(0, |run| run.value);
                let total = before
                    .checked_add(amount)
                    .ok_or_else(|| crate::invalid("Trade integral exceeds its integer range"))?;
                cumulative(values, id, frame.hour, total);
            }
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
    pub(crate) fn bytes(&self) -> usize {
        self.facts.bytes()
            + self.values.bytes()
            + self.totals.iter().map(Accumulator::bytes).sum::<usize>()
            + self.coverage.iter().map(Accumulator::bytes).sum::<usize>()
            + self.refusal_counts.bytes()
            + self.pattern_totals.bytes()
    }
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
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
        self.facts
            .flush(writer, &format!("trade-facts-{branch}"), 0, cancelled)?;
        self.values.flush(
            writer,
            &format!("trade-values-{branch}"),
            RESOURCE_DECIMALS,
            cancelled,
        )?;
        for (index, values) in self.totals.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("trade-totals-{branch}-{index}"),
                if index == 0 { RESOURCE_DECIMALS } else { 0 },
                cancelled,
            )?;
        }
        for (index, values) in self.coverage.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("trade-coverage-{branch}-{index}"),
                0,
                cancelled,
            )?;
        }
        self.refusal_counts
            .flush(writer, &format!("trade-refusals-{branch}"), 0, cancelled)?;
        self.pattern_totals.flush(
            writer,
            &format!("trade-partial-hours-{branch}"),
            0,
            cancelled,
        )?;
        let mut reply = super::reply(
            source,
            &TradeFlowParams {
                branch_id: branch.to_string(),
                tags: None,
            },
            context,
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
        series::write_catalogue(
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
pub(crate) fn query(
    source: &Source,
    params: Value,
    content: Option<&super::super::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params.clone())?;
    let selection = TradeFlowParams {
        branch_id: input.branch_id.clone(),
        tags: input.tags.clone(),
    };
    TradeFlows::validate(&selection).map_err(crate::invalid)?;
    let branch = selection.branch();
    let coverage = match crate::prepared::pipeline::coverage(source, content, branch, cancelled) {
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
    let (store, key) = crate::prepared::files(source, content)?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown trade branch"))?;
    let bounds = crate::metadata::branch_bounds(source.replay_ref(), start);
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
    let sampler = |frame: &replay::frame::DecodedFrame, requested: Option<&[(String, String)]>| {
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
        let sampled = sample::sample(frame, Some(&requested))?;
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
    read_pages(
        &store, &key, branch, input, params, &history, last_hour, complete, context, cancelled,
    )
    .map(|value| {
        value.map(|mut value| {
            value["coverage"] = json!(coverage);
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
        return Err(crate::invalid("Unknown trade operation"));
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
        return Err(crate::invalid("Invalid trade measure"));
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
        crate::metadata::check_cancel(cancelled)?;
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
                    runs.push(crate::prepared_chart::ChartRun {
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
                .ok_or_else(|| crate::invalid("Trade fact coverage is missing"))?;
            if !seen.insert((run.first_hour, run.last_hour)) {
                continue;
            }
            let (_, value) = facts
                .latest_event(
                    id,
                    run.value
                        .try_into()
                        .map_err(|_| crate::invalid("Invalid trade fact date"))?,
                )?
                .ok_or_else(|| crate::invalid("Trade fact is missing"))?;
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
                            .map_err(|_| crate::invalid("Invalid partial trade count"))?,
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
                        deals: run
                            .value
                            .try_into()
                            .map_err(|_| crate::invalid("Invalid sampled partial trade count"))?,
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
    use crate::test_fixtures::{CAMPAIGN, records, resource_frame};
    #[cfg(feature = "slow-tests")]
    use replay::rows::{CountryLinkRow, CountryRow, ResourceExchangeRow};
    use replay::{frame::TableView, rows::TableRows};

    #[test]
    fn trade_observer_work_is_flat_over_unchanged_hours_and_retired_flows() {
        use crate::prepared::{
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
        use crate::prepared::frame_changes::FrameChangeTracker;
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
            rows.push(replay::rows::ResourceExchangeRow {
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
            crate::SourceKey {
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
        let mut fixture = crate::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
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
        assert_eq!(prepared.keys.len(), 5000);
        assert_eq!(prepared.scan.result.flows.len(), 5000);
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
            crate::SourceKey {
                source_id: "trade-average".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let history = country_history::scan_histories(&replay, 1, false, &[], &flag).unwrap();
        let fixture = crate::prepared::series::tests::Fixture::new();
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
        use crate::test_fixtures::{continuation_records, file_source};
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
        let (store, key) = crate::prepared::files(&source, None).unwrap();
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
                serde_json::to_vec(&crate::prepared::pipeline::Coverage {
                    schema: crate::prepared::pipeline::SCHEMA,
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
        let summary: crate::prepared_chart::ChartSummary =
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
}
