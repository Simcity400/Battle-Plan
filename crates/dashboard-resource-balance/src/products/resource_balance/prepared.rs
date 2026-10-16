//! The existing country step feeds exact pools, independent deals and chart values.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::query::progressive::{self, AdapterTail};
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    keys: BTreeMap<String, u64>,
    pools: Accumulator,
    deals: Accumulator,
    known: Accumulator,
    values: [Accumulator; 11],
    memberships: BTreeMap<String, Vec<u64>>,
    maxima: BTreeMap<String, i128>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

fn pool_name(tag: &str, resource: u32) -> String {
    series::field_key(tag, &format!("pool:{resource}"))
}
fn deal_name(tag: &str, deal: &str, resource: u32) -> io::Result<String> {
    Ok(series::field_key(
        tag,
        &serde_json::to_string(&(deal, resource))?,
    ))
}
fn key(keys: &mut BTreeMap<String, u64>, name: String) -> u64 {
    let next = keys.len() as u64;
    *keys.entry(name).or_insert(next)
}
fn observe_value(
    accumulator: &mut Accumulator,
    key: u64,
    hour: i32,
    value: Option<i128>,
    starts: bool,
) {
    if starts
        || value.is_none()
        || accumulator
            .state(key)
            .and_then(|state| state.last)
            .is_none_or(|last| Some(last.value) != value)
    {
        accumulator.observe(key, hour, value, starts);
        #[cfg(test)]
        EMISSIONS.with(|count| count.set(count.get() + 1));
    }
}
#[cfg(test)]
thread_local! { static EMISSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
impl Prepared {
    #[cfg(test)]
    pub fn observe(&mut self, frame: &replay::frame::DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.pools.advance(frame.hour)?;
        self.deals.advance(frame.hour)?;
        self.known.advance(frame.hour)?;
        for values in &mut self.values {
            values.advance(frame.hour)?;
        }
        let continuous =
            self.scan.frontier.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        let countries_changed = if changes.rows_changed[TableId::Countries.index()]
            || changes.rows_changed[TableId::CountryLinks.index()]
            || !continuous
        {
            let rows = frame.complete::<tables::Countries>().ok();
            let same = rows.is_some_and(|rows| {
                self.scan
                    .country_reads
                    .as_ref()
                    .is_some_and(|seen| seen.matches(frame, rows))
            });
            if !same {
                self.scan.country_reads = rows.map(|rows| {
                    if !changes.rows_changed[TableId::CountryLinks.index()]
                        && let Some(reads) = &self.scan.country_reads
                    {
                        dashboard_base::products::territory::CountryReads::from_slots(
                            frame,
                            rows,
                            Arc::clone(reads.slots()),
                        )
                    } else {
                        dashboard_base::products::territory::CountryReads::new(frame, rows)
                    }
                });
            }
            !same
        } else {
            false
        };
        let changed = !continuous
            || countries_changed
            || [
                TableId::CountryLinks,
                TableId::LexicalTokens,
                TableId::ResourceDefinitions,
                TableId::CountryResources,
                TableId::ResourceExchanges,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        if !changed {
            self.scan.advance(frame.hour);
            return Ok(());
        }
        #[cfg(test)]
        let emitted_before = EMISSIONS.with(std::cell::Cell::get);
        let names_changed = !self.scan.inputs_ready
            || changes.rows_changed[TableId::ResourceDefinitions.index()]
            || changes.rows_changed[TableId::LexicalTokens.index()];
        let changed = [
            !self.scan.inputs_ready || changes.rows_changed[TableId::CountryResources.index()],
            !self.scan.inputs_ready || changes.rows_changed[TableId::ResourceExchanges.index()],
        ];
        self.scan
            .refresh_inputs(frame, [changed[0], changed[1], names_changed]);
        let touched = self.scan.observe_resolved(
            frame,
            names_changed,
            if countries_changed || names_changed || !continuous {
                [true; 2]
            } else {
                changed
            },
            countries_changed || !continuous,
        )?;
        #[cfg(test)]
        {
            self.work.history_visits = self.scan.history_visits;
            self.work.row_visits = self.scan.row_visits;
        }
        for (tag, &changed) in &touched {
            let country = self
                .scan
                .countries
                .get_mut(tag)
                .expect("touched resource country");
            if changed[0] {
                for &resource in &country.dirty_pools {
                    let runs = country
                        .result
                        .series
                        .get_mut(&resource)
                        .expect("changed resource pool");
                    let name = pool_name(tag, resource);
                    let id = key(&mut self.keys, name.clone());
                    let run = runs.last().filter(|run| run.last_hour == frame.hour);
                    if let Some(run) = run
                        && self
                            .pools
                            .state(id)
                            .and_then(|state| state.last)
                            .is_none_or(|last| last.value != i128::from(run.first_hour))
                    {
                        self.pools.event(id, frame.hour, run)?;
                    }
                    observe_value(
                        &mut self.pools,
                        id,
                        frame.hour,
                        run.map(|run| i128::from(run.first_hour)),
                        run.is_some_and(|run| run.first_hour == frame.hour && run.starts_segment),
                    );
                    let native = run.map(resource_values).transpose()?.unwrap_or([0; 11]);
                    for (index, accumulator) in self.values.iter_mut().enumerate() {
                        let field = sample::field(resource, index);
                        let amount = run
                            .filter(|_| {
                                self.scan
                                    .current_names
                                    .as_ref()
                                    .is_some_and(|names| names.contains(resource))
                            })
                            .map(|_| native[index]);
                        let value_key = key(&mut self.keys, series::field_key(tag, &field));
                        observe_value(
                            accumulator,
                            value_key,
                            frame.hour,
                            amount,
                            run.is_some_and(|run| {
                                run.first_hour == frame.hour && run.starts_segment
                            }),
                        );
                    }
                    if run.is_some() {
                        let maximum = self.maxima.entry(name).or_default();
                        *maximum = (*maximum).max(native[0]).max(native[1]);
                    }
                    if runs.len() > 1 {
                        runs.drain(..runs.len() - 1);
                    }
                }
            }
            if !changed[1] {
                continue;
            }
            let country_key = key(&mut self.keys, series::field_key(tag, "deals"));
            observe_value(
                &mut self.known,
                country_key,
                frame.hour,
                country.deals_open.then_some(1),
                country
                    .result
                    .deal_hours
                    .last()
                    .is_some_and(|span| span.first_hour == frame.hour),
            );
            let mut active = BTreeMap::<u32, Vec<u64>>::new();
            let mut changed = BTreeSet::new();
            for &index in country.open_trades.values() {
                let run = &country.result.trades[index];
                let id = key(&mut self.keys, deal_name(tag, &run.deal, run.resource)?);
                if self
                    .deals
                    .state(id)
                    .and_then(|state| state.last)
                    .is_none_or(|last| last.value != i128::from(run.first_hour))
                {
                    self.deals.event(id, frame.hour, run)?;
                    changed.insert(run.resource);
                }
                observe_value(
                    &mut self.deals,
                    id,
                    frame.hour,
                    Some(i128::from(run.first_hour)),
                    run.first_hour == frame.hour && run.starts_segment,
                );
                active.entry(run.resource).or_default().push(id);
            }
            for &(resource, _) in &country.result.resources {
                let name = series::field_key(tag, &format!("deals:{resource}"));
                let id = key(&mut self.keys, name.clone());
                let mut members = active.remove(&resource).unwrap_or_default();
                members.sort_unstable();
                for &removed in self
                    .memberships
                    .get(&name)
                    .into_iter()
                    .flatten()
                    .filter(|id| !members.contains(id))
                {
                    self.deals.observe(removed, frame.hour, None, false);
                }
                if self.memberships.get(&name) != Some(&members) || changed.contains(&resource) {
                    self.known.event(id, frame.hour, &members)?;
                    self.memberships.insert(name, members);
                }
                let version = self.known.state(id).map_or(0, |state| state.events);
                observe_value(
                    &mut self.known,
                    id,
                    frame.hour,
                    country.deals_open.then_some(i128::from(version)),
                    country
                        .result
                        .deal_hours
                        .last()
                        .is_some_and(|span| span.first_hour == frame.hour),
                );
            }
            if country.result.deal_hours.len() > 1 {
                country
                    .result
                    .deal_hours
                    .drain(..country.result.deal_hours.len() - 1);
            }
        }
        #[cfg(test)]
        {
            self.work.emitted += EMISSIONS.with(std::cell::Cell::get) - emitted_before;
        }
        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.pools.bytes()
            + self.deals.bytes()
            + self.known.bytes()
            + self.values.iter().map(Accumulator::bytes).sum::<usize>()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal();
        // Closed deals already live in events. Compact once at publication,
        // keeping native scan indices stable throughout the admitted batch.
        for country in self.scan.countries.values_mut() {
            let trades = std::mem::take(&mut country.result.trades);
            let open: BTreeMap<_, _> = country
                .open_trades
                .iter()
                .map(|(&deal, &index)| (index, deal))
                .collect();
            let mut kept = Vec::with_capacity(country.open_trades.len());
            for (index, trade) in trades.into_iter().enumerate() {
                if let Some(&deal) = open.get(&index) {
                    country.open_trades.insert(deal, kept.len());
                    kept.push(trade);
                }
            }
            country.result.trades = kept;
        }
        self.pools
            .flush(writer, &format!("resource-pools-{branch}"), 0, cancelled)?;
        self.deals
            .flush(writer, &format!("resource-deals-{branch}"), 0, cancelled)?;
        self.known
            .flush(writer, &format!("resource-known-{branch}"), 0, cancelled)?;
        for (index, values) in self.values.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("resource-values-{branch}-{index}"),
                RESOURCE_DECIMALS,
                cancelled,
            )?;
        }
        let mut countries = BTreeMap::new();
        for (tag, country) in &self.scan.countries {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut reply = super::reply(
                source,
                &CountryParams {
                    branch_id: branch.to_string(),
                    tag: tag.clone(),
                },
                context,
                &country.result,
                cancelled,
            )?;
            for series in &mut reply.series {
                series.runs.clear();
            }
            reply.trades.clear();
            reply.deal_hours.clear();
            let maxima: BTreeMap<_, _> = country
                .result
                .series
                .keys()
                .map(|&resource| {
                    (
                        resource,
                        self.maxima
                            .get(&pool_name(tag, resource))
                            .copied()
                            .unwrap_or(0)
                            .to_string(),
                    )
                })
                .collect();
            countries.insert(tag.clone(), (reply, maxima));
        }
        series::write_catalogue(writer, "resource-balance", branch, &self.keys, &countries)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    tag: String,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    #[serde(default)]
    hours: Vec<i32>,
    op: Option<String>,
    #[serde(default)]
    resources: Vec<u32>,
    within: Option<Vec<HourSpan>>,
    #[serde(default)]
    require_complete: bool,
}

type Progressive<'a> = (AdapterTail<'a>, AdapterTail<'a>, ResourceBalanceReply);

/// Rebuild only native requested pool operands; approximate geometry never supplies written values.
fn sampled_pool(
    values: &BTreeMap<(String, String), Vec<series::Run>>,
    tag: &str,
    resource: u32,
    hour: i32,
) -> Option<PoolRun> {
    let mut native = [0; 12];
    let mut first = i32::MIN;
    let mut last = i32::MAX;
    let mut starts = false;
    for (index, amount) in native.iter_mut().enumerate() {
        let runs = values.get(&(tag.to_owned(), sample::field(resource, index)))?;
        let at = runs
            .partition_point(|run| run.first_hour <= hour)
            .checked_sub(1)?;
        let run = runs.get(at).filter(|run| hour <= run.last_hour)?;
        first = first.max(run.first_hour);
        last = last.min(run.last_hour);
        *amount = run.value;
    }
    for index in 0..12 {
        starts |= values
            .get(&(tag.to_owned(), sample::field(resource, index)))?
            .iter()
            .any(|run| run.first_hour == first && run.break_before);
    }
    let after_projects = native[10] - native[6];
    Some(PoolRun {
        first_hour: first,
        last_hour: last,
        starts_segment: starts,
        produced_raw: native[2].to_string(),
        subjects_raw: (native[3] + native[7]).to_string(),
        imported_raw: native[4].to_string(),
        to_export_raw: (-native[8]).to_string(),
        exported_raw: native[11].to_string(),
        available_raw: (after_projects - native[5]).to_string(),
        after_projects_raw: after_projects.to_string(),
        production_raw: native[6].to_string(),
    })
}

pub fn query(
    source: &Source,
    params: Value,
    content: Option<&dashboard_core::config::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let input: Params = serde_json::from_value(params.clone())?;
    let branch: u64 = input
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid resource branch"))?;
    dashboard_core::protocol::validate_tag(&input.tag).map_err(dashboard_core::invalid)?;
    let coverage =
        match dashboard_base::prepared::coverage::coverage(source, content, branch, cancelled) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
            result => result?,
        };
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown resource branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), start);
    let last_hour = coverage
        .as_ref()
        .map_or(bounds.0.saturating_sub(1), |coverage| coverage.last_hour);
    let complete = coverage.as_ref().is_some_and(|coverage| coverage.complete);
    if input.require_complete && !complete {
        return Ok(None);
    }
    if input.op.is_some() && coverage.is_none() {
        return Ok(None);
    }
    let view = series::viewport(
        input.from.or(Some(f64::from(bounds.1))),
        input.to.or(input.from).or(Some(f64::from(bounds.1))),
        Some(input.width.unwrap_or(1)),
    )?;
    let requested = vec![(input.tag.clone(), String::new())];
    let names = std::cell::RefCell::new(Learned::default());
    let sampler = |frame: &replay::frame::DecodedFrame, requested: Option<&[(String, String)]>| {
        if let (Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::ResourceDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            names.borrow_mut().learn(definitions, lexical);
        }
        let mut values = sample::sample(frame, requested)?;
        if !input.resources.is_empty() {
            values.retain(|(_, field), _| {
                sample::parse(field).is_ok_and(|(_, resource)| input.resources.contains(&resource))
            });
        }
        Ok(values)
    };
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
    let hours: Vec<_> = input
        .hours
        .iter()
        .copied()
        .chain([view.0.floor() as i32, view.1.floor() as i32])
        .filter(|hour| {
            !tail.1.is_empty() || *hour < view.0.floor() as i32 || *hour > view.1.floor() as i32
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let states = progressive::state_tail(
        source,
        branch,
        coverage.as_ref(),
        sample::TABLES,
        &requested,
        &hours,
        &sampler,
        cancelled,
    )?;
    let mut scanned = ScannedBalance {
        resources: names.borrow().list(),
        ..Default::default()
    };
    for (_, field) in tail.0.keys().chain(states.0.keys()) {
        if let Ok((_, resource)) = sample::parse(field) {
            scanned.series.entry(resource).or_default();
        }
    }
    let fallback = super::reply(
        source,
        &CountryParams {
            branch_id: input.branch_id.clone(),
            tag: input.tag.clone(),
        },
        &LoadContext {
            content: content.cloned(),
        },
        &scanned,
        cancelled,
    )?;
    let context = (!complete && input.op.is_none()).then_some((
        (bounds, &tail.0, tail.1.as_slice(), &tail.2, true),
        (bounds, &states.0, states.1.as_slice(), &states.2, true),
        fallback,
    ));
    let (store, source_key) = dashboard_base::prepared::files(source, content)?;
    read_pages(
        &store,
        &source_key,
        branch,
        input,
        params,
        last_hour,
        complete,
        context,
        cancelled,
    )
    .map(|value| {
        value.map(|mut value| {
            value["coverage"] = json!(coverage);
            value
        })
    })
}

#[allow(clippy::too_many_arguments)]
fn read_pages(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    input: Params,
    params: Value,
    last_hour: i32,
    complete: bool,
    progressive: Option<Progressive<'_>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Value>> {
    let catalogue = series::read_catalogue::<
        BTreeMap<String, (ResourceBalanceReply, BTreeMap<u32, String>)>,
    >(store, source_key, "resource-balance", branch)?;
    if catalogue.is_none() && progressive.is_none() {
        return Ok(None);
    }
    let (keys, mut countries) = catalogue.unwrap_or_default();
    let (mut reply, maxima) = countries.remove(&input.tag).unwrap_or_else(|| {
        (
            ResourceBalanceReply {
                branch_id: input.branch_id.clone(),
                tag: input.tag.clone(),
                decimals: RESOURCE_DECIMALS,
                resources: Vec::new(),
                series: Vec::new(),
                deal_hours: Vec::new(),
                trades: Vec::new(),
                warnings: vec![format!(
                    "{}: no recorded resources in this branch",
                    input.tag
                )],
            },
            BTreeMap::new(),
        )
    });
    if let Some((_, _, fallback)) = &progressive {
        for resource in &fallback.resources {
            if !reply.resources.iter().any(|known| known.id == resource.id) {
                reply.resources.push(resource.clone());
                reply.series.push(ResourceSeries {
                    id: resource.id,
                    runs: Vec::new(),
                });
            }
        }
        reply.resources.sort_by_key(|resource| resource.id);
        reply.series.sort_by_key(|resource| resource.id);
        if !fallback.resources.is_empty() {
            reply
                .warnings
                .retain(|warning| !warning.ends_with(": no recorded resources in this branch"));
        }
    }
    let mut pools = Reader::new(
        store,
        source_key,
        format!("resource-pools-{branch}"),
        cancelled,
    )?;
    let mut deals = Reader::new(
        store,
        source_key,
        format!("resource-deals-{branch}"),
        cancelled,
    )?;
    let mut known = Reader::new(
        store,
        source_key,
        format!("resource-known-{branch}"),
        cancelled,
    )?;
    let country_key = keys.get(&series::field_key(&input.tag, "deals")).copied();
    if input.op.as_deref() == Some("navigate") {
        let selected: Vec<_> = reply
            .series
            .iter()
            .filter(|row| input.resources.is_empty() || input.resources.contains(&row.id))
            .filter_map(|row| {
                keys.get(&pool_name(&input.tag, row.id))
                    .map(|&key| (key, input.within.as_deref()))
            })
            .collect();
        let mut readers = vec![(&mut pools, selected)];
        let selected_deals = reply
            .resources
            .iter()
            .filter(|row| input.resources.is_empty() || input.resources.contains(&row.id))
            .filter_map(|row| {
                keys.get(&series::field_key(&input.tag, &format!("deals:{}", row.id)))
                    .map(|&key| (key, input.within.as_deref()))
            })
            .collect();
        readers.push((&mut known, selected_deals));
        return Ok(Some(
            json!({"hour":series::navigate(&mut readers, &serde_json::from_value::<series::Navigation>(params)?)?}),
        ));
    }
    if input.op.is_some() {
        return Err(dashboard_core::invalid("Unknown resource operation"));
    }
    let view_from = input.from.unwrap_or(f64::from(last_hour));
    let view_to = input.to.unwrap_or(view_from);
    let (view_from, view_to, _) = series::viewport(
        Some(view_from),
        Some(view_to),
        Some(input.width.unwrap_or(1)),
    )?;
    let from = view_from.floor() as i32;
    let to = view_to.floor() as i32;
    if from > to || input.width == Some(0) {
        return Err(dashboard_core::invalid("Invalid resource viewport"));
    }
    if progressive.is_none()
        && !complete
        && (to > last_hour || input.hours.iter().any(|&hour| hour > last_hour))
    {
        return Ok(None);
    }
    let dense = input
        .width
        .is_some_and(|width| i64::from(to) - i64::from(from) + 1 > i64::from(width));
    if let Some(width) = input.width {
        series::summary_budget(
            view_from,
            view_to,
            width,
            reply
                .series
                .iter()
                .filter(|row| input.resources.is_empty() || input.resources.contains(&row.id))
                .count()
                * 11,
        )?;
    }
    let mut summaries = Vec::new();
    for row in &mut reply.series {
        let absent = keys
            .values()
            .max()
            .copied()
            .map_or(Some(0), |key| key.checked_add(1))
            .ok_or_else(|| dashboard_core::invalid("Resource key allocation overflow"))?;
        let key = keys
            .get(&pool_name(&input.tag, row.id))
            .copied()
            .unwrap_or(absent);
        let mut moments: BTreeSet<_> = input.hours.iter().copied().collect();
        let selected = input.resources.is_empty() || input.resources.contains(&row.id);
        let mut runs = if (dense || progressive.is_some()) && selected {
            let width = input.width.unwrap_or(1);
            let mut line = Vec::new();
            for index in 0..11 {
                let mut values = Reader::new(
                    store,
                    source_key,
                    format!("resource-values-{branch}-{index}"),
                    cancelled,
                )?;
                let field = sample::field(row.id, index);
                let value_key = keys
                    .get(&series::field_key(&input.tag, &field))
                    .copied()
                    .unwrap_or(absent);
                let mut summary = if let Some((tail, _, _)) = &progressive {
                    values.summary_with_tail(
                        value_key,
                        tail.0,
                        (view_from, view_to, width),
                        RESOURCE_DECIMALS,
                        tail.1
                            .get(&(input.tag.clone(), field))
                            .map_or(&[][..], Vec::as_slice),
                        input.within.as_deref(),
                    )?
                } else {
                    values.summary_within(
                        value_key,
                        (from, to),
                        (view_from, view_to, width),
                        RESOURCE_DECIMALS,
                        input.within.as_deref(),
                    )?
                };
                if let Some((tail, _, _)) = &progressive {
                    progressive::clip_approximate(&mut summary, tail.2, input.within.as_deref());
                }
                for bucket in summary.buckets.iter().flatten() {
                    moments.extend([
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ]);
                }
                line.push(summary);
            }
            summaries.push(json!({"id":row.id,"supply":line[0],"demand":line[1],"parts":&line[2..10],"total":line[10]}));
            if dense {
                Vec::new()
            } else {
                pools.runs(key, from, to)?
            }
        } else if dense {
            Vec::new()
        } else {
            pools.runs(key, from, to)?
        };
        for hour in moments {
            if let Some(run) = pools.value_at(key, hour)? {
                runs.push(run);
            }
        }
        let mut dates = BTreeSet::new();
        for run in runs {
            let run = pools
                .value_at(key, run.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Resource pool coverage is missing"))?;
            if !dates.insert((run.first_hour, run.last_hour)) {
                continue;
            }
            let origin: i32 = run
                .value
                .try_into()
                .map_err(|_| dashboard_core::invalid("Invalid pool supplying date"))?;
            let (_, event) = pools
                .latest_event(key, origin)?
                .ok_or_else(|| dashboard_core::invalid("Resource pool event is missing"))?;
            let mut pool: PoolRun = serde_json::from_value(event)?;
            pool.first_hour = run.first_hour;
            pool.last_hour = run.last_hour;
            row.runs.push(pool);
        }
        row.runs.sort_by_key(|run| run.first_hour);
        if let Some((tail, states, _)) = &progressive {
            for context in [Some(tail).filter(|tail| tail.2.is_empty()), Some(states)]
                .into_iter()
                .flatten()
            {
                let dates: BTreeSet<_> = context
                    .1
                    .iter()
                    .filter(|((tag, field), _)| {
                        tag == &input.tag
                            && sample::parse(field).is_ok_and(|(_, resource)| resource == row.id)
                    })
                    .flat_map(|(_, runs)| {
                        runs.iter().flat_map(|run| [run.first_hour, run.last_hour])
                    })
                    .collect();
                for hour in dates {
                    if row
                        .runs
                        .iter()
                        .any(|run| run.first_hour <= hour && hour <= run.last_hour)
                    {
                        continue;
                    }
                    if let Some(pool) = sampled_pool(context.1, &input.tag, row.id, hour) {
                        row.runs.push(pool);
                    }
                }
            }
            row.runs.sort_by_key(|run| run.first_hour);
            let mut merged: Vec<PoolRun> = Vec::new();
            for run in std::mem::take(&mut row.runs) {
                if let Some(previous) = merged.last_mut().filter(|previous| {
                    !run.starts_segment
                        && previous.last_hour.checked_add(1) == Some(run.first_hour)
                        && same_pools(previous, &run)
                }) {
                    previous.last_hour = run.last_hour;
                } else {
                    merged.push(run);
                }
            }
            row.runs = merged;
        }
    }
    let exact_hours: BTreeSet<_> = input
        .hours
        .iter()
        .copied()
        .chain([from, to])
        .filter(|hour| {
            progressive.as_ref().is_none_or(|(_, states, _)| {
                !states
                    .2
                    .iter()
                    .any(|span| span.first_hour <= *hour && *hour <= span.last_hour)
            })
        })
        .collect();
    let mut hours = exact_hours.clone();
    if !dense {
        if let Some(key) = country_key {
            for run in known.runs(key, from, to)? {
                hours.extend([run.first_hour, run.last_hour]);
            }
        }
        for resource in &reply.resources {
            if let Some(&key) = keys.get(&series::field_key(
                &input.tag,
                &format!("deals:{}", resource.id),
            )) {
                for run in known.runs(key, from, to)? {
                    hours.extend([run.first_hour, run.last_hour]);
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    if let Some(country_key) = country_key {
        for hour in hours {
            let Some(run) = known.value_at(country_key, hour)? else {
                continue;
            };
            reply.deal_hours.push(HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            });
            for resource in &reply.resources {
                let Some(&resource_key) = keys.get(&series::field_key(
                    &input.tag,
                    &format!("deals:{}", resource.id),
                )) else {
                    continue;
                };
                let Some((_, members)) = known.latest_event(resource_key, hour)? else {
                    continue;
                };
                for key in serde_json::from_value::<Vec<u64>>(members)? {
                    if let Some(run) = deals.value_at(key, hour)?
                        && seen.insert((key, run.first_hour, run.last_hour))
                    {
                        let origin: i32 = run
                            .value
                            .try_into()
                            .map_err(|_| dashboard_core::invalid("Invalid trade supplying date"))?;
                        let (_, row) = deals.latest_event(key, origin)?.ok_or_else(|| {
                            dashboard_core::invalid("Resource deal event is missing")
                        })?;
                        let mut row: TradeRun = serde_json::from_value(row)?;
                        row.first_hour = run.first_hour;
                        row.last_hour = run.last_hour;
                        reply.trades.push(row);
                    }
                }
            }
        }
    }
    reply.deal_hours.sort_by_key(|run| run.first_hour);
    reply
        .deal_hours
        .dedup_by_key(|run| (run.first_hour, run.last_hour));
    reply
        .trades
        .sort_by_key(|run| (run.first_hour, run.deal.clone()));
    let mut data = serde_json::to_value(reply)?;
    data["summaries"] = json!(summaries);
    data["campaignSizes"] = serde_json::to_value(maxima)?;
    data["exactHours"] = json!(exact_hours);
    Ok(Some(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::test_fixtures::{CAMPAIGN, records, resource_frame};
    use replay::frame::TableView;
    #[cfg(feature = "slow-tests")]
    use replay::rows::{CountryLinkRow, CountryResourceRow, CountryRow, TableRows};

    #[test]
    fn resource_projection_refresh_visits_only_its_changed_source_table() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let mut frame = resource_frame(100, 30, 9001, &[(51, 21, 9001, 12)]);
        let format = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])])
            .unwrap()
            .format;
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        let slots = Arc::clone(prepared.scan.country_reads.as_ref().unwrap().slots());
        let before = prepared.work;
        frame.hour += 1;
        if let TableView::Complete(replay::rows::TableRows::CountryResources(rows)) =
            &mut *frame.tables[TableId::CountryResources.index()]
        {
            rows.truncate(1);
            rows[0].produced = i64::MIN;
        }
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        assert_eq!(
            prepared.work.row_visits - before.row_visits,
            1,
            "pool changes do not visit exchanges or definitions"
        );
        assert!(Arc::ptr_eq(
            &slots,
            prepared.scan.country_reads.as_ref().unwrap().slots()
        ));
        let before = prepared.work;
        frame.hour += 1;
        if let TableView::Complete(replay::rows::TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        {
            rows[0].delivered = i64::MAX;
        }
        prepared
            .observe_changed(&frame, &tracker.observe(&frame, format))
            .unwrap();
        assert_eq!(
            prepared.work.row_visits - before.row_visits,
            1,
            "exchange changes do not visit pools or definitions"
        );
        assert_eq!(
            prepared.scan.countries["GER"]
                .result
                .trades
                .last()
                .unwrap()
                .delivered_raw,
            i64::MAX.to_string()
        );
        prepared.scan.seal();
        assert_eq!(
            prepared.scan.countries["GER"].result.series[&1]
                .last()
                .unwrap()
                .last_hour,
            frame.hour
        );
    }

    #[test]
    fn resource_projection_preserves_duplicate_order_unknowns_and_signed_operands() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let mut frame = resource_frame(
            100,
            0,
            9001,
            &[
                (51, 21, 9001, i64::MAX),
                (52, 21, 9001, i64::MIN),
                (53, 0xdead, 9001, -7),
            ],
        );
        if let TableView::Complete(replay::rows::TableRows::CountryResources(rows)) =
            &mut *frame.tables[TableId::CountryResources.index()]
        {
            rows[0].produced = i64::MIN;
            let mut unknown = rows[0].clone();
            unknown.resource = 999;
            rows.push(unknown);
            rows.sort_by_key(|row| (row.country, row.resource));
        }
        let mut later = frame.clone();
        later.hour += 1;
        if let TableView::Complete(replay::rows::TableRows::Countries(rows)) =
            &mut *later.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"new resource alias".to_vec();
        }
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![frame.clone(), later.clone()])],
        )
        .unwrap();
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in [&frame, &later] {
            prepared
                .observe_changed(frame, &tracker.observe(frame, replay.format))
                .unwrap();
        }
        prepared.scan.seal();
        for tag in prepared.scan.countries.keys() {
            let oracle = scan(&replay, 1, tag, &AtomicBool::new(false)).unwrap();
            // Prepared fact compaction retains only the latest pool operand;
            // this fixture has one unchanged pool interval per holder.
            observer_tests::assert_reply_parity(&prepared.scan.countries[tag].result, &oracle)
                .unwrap();
        }
        let trades = &prepared.scan.countries["GER"].result.trades;
        assert_eq!(trades[0].deal, "51");
        assert_eq!(trades[1].deal, "52");
        assert_eq!(trades[0].delivered_raw, i64::MAX.to_string());
        assert_eq!(trades[1].delivered_raw, i64::MIN.to_string());
        assert!(
            prepared
                .scan
                .countries
                .values()
                .any(|country| country.result.unresolved > 0)
        );
    }

    #[test]
    fn resource_observer_work_is_flat_over_unchanged_hours_and_closed_aliases() {
        use dashboard_base::prepared::{
            frame_changes::FrameChangeTracker,
            observer_tests::{self, Work},
        };
        let frame = resource_frame(100, 30, 9001, &[(51, 21, 9001, 12)]);
        let format = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])])
            .unwrap()
            .format;
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |hours| {
                let mut prepared = Prepared::default();
                let mut tracker = FrameChangeTracker::new();
                prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
                observer_tests::unchanged_frames(
                    prepared.work,
                    &frame,
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
                let mut frame = frame.clone();
                for offset in 0..history {
                    frame.hour = 100 + offset as i32;
                    if let replay::frame::TableView::Complete(replay::rows::TableRows::Countries(
                        rows,
                    )) = &mut *frame.tables[TableId::Countries.index()]
                    {
                        rows.iter_mut()
                            .find(|row| row.canonical_slot == 7)
                            .unwrap()
                            .tag = format!("past resource alias {offset}").into_bytes();
                    }
                    prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
                }
                let before = prepared.work;
                frame.hour += 1;
                if let replay::frame::TableView::Complete(
                    replay::rows::TableRows::CountryResources(rows),
                ) = &mut *frame.tables[TableId::CountryResources.index()]
                {
                    rows[0].produced += 1;
                }
                prepared.observe_changed(&frame, &tracker.observe(&frame, format))?;
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
    fn resource_deferred_frames_seal_exact_deals_pools_and_diagnostics_at_two_publications() {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let mut initial =
            resource_frame(100, 30, 9001, &[(51, 21, 9001, 12), (52, 0xdead, 9001, 1)]);
        if let TableView::Complete(replay::rows::TableRows::CountryResources(rows)) =
            &mut *initial.tables[TableId::CountryResources.index()]
        {
            let mut unnamed = rows[0].clone();
            unnamed.resource = 999;
            rows.push(unnamed);
            rows.sort_by_key(|row| (row.country, row.resource));
        }
        let mut frames = Vec::new();
        for hour in 100..=135 {
            if hour == 120 {
                continue;
            }
            let mut frame = initial.clone();
            frame.hour = hour;
            if hour >= 128
                && let TableView::Complete(replay::rows::TableRows::ResourceExchanges(rows)) =
                    &mut *frame.tables[TableId::ResourceExchanges.index()]
            {
                rows.clear();
            }
            frames.push(frame);
        }
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let flag = AtomicBool::new(false);
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "resource-deferred".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames {
            prepared
                .observe_changed(frame, &tracker.observe(frame, replay.format))
                .unwrap();
            if [110, 125, 135].contains(&frame.hour) {
                let mut writer = if frame.hour == 135 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                prepared
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
                // Publication reads the native open-state metadata only after seal.
                assert_eq!(
                    prepared.scan.countries["GER"].result.series[&1]
                        .last()
                        .unwrap()
                        .last_hour,
                    frame.hour
                );
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                tracker.invalidate();
            }
        }
        for tag in ["GER", "ITA", "SPR"] {
            let expected = super::super::reply(
                &source,
                &CountryParams {
                    branch_id: "1".into(),
                    tag: tag.into(),
                },
                &context,
                &scan(&replay, 1, tag, &flag).unwrap(),
                &flag,
            )
            .unwrap();
            let input = json!({"branchId":"1","tag":tag,"from":100,"to":135});
            let actual = read_pages(
                &fixture.store,
                &fixture.key,
                1,
                serde_json::from_value(input.clone()).unwrap(),
                input,
                135,
                true,
                None,
                &flag,
            )
            .unwrap()
            .unwrap();
            let actual: ResourceBalanceReply = serde_json::from_value(actual).unwrap();
            observer_tests::assert_reply_parity(&actual, &expected).unwrap();
        }
    }

    #[test]
    fn resource_pages_match_country_scan_after_two_commits_resume_and_append() {
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (100..=109)
            .filter(|&hour| hour != 105)
            .map(|hour| {
                let mut frame = resource_frame(
                    hour,
                    if hour < 103 { -100 } else { 300 },
                    if hour < 108 { 9001 } else { 9002 },
                    if hour < 104 {
                        &[(51, 21, 9001, -200)]
                    } else if hour < 108 {
                        &[(52, 11, 31, 500)]
                    } else {
                        &[]
                    },
                );
                if hour == 102 {
                    *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                        reason: "synthetic missing resource read".into(),
                    };
                }
                if hour == 107 {
                    *frame.tables[TableId::CountryResources.index()] = TableView::Unavailable {
                        reason: "synthetic missing resource read".into(),
                    };
                }
                frame
            })
            .collect();
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "resource-prepared".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        for frame in &frames {
            prepared.observe(frame).unwrap();
            if [101, 104, 109].contains(&frame.hour) {
                let mut writer = if frame.hour == 109 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                prepared
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            }
        }
        for tag in ["GER", "ITA", "SPR"] {
            let expected = super::super::reply(
                &source,
                &CountryParams {
                    branch_id: "1".into(),
                    tag: tag.into(),
                },
                &context,
                &scan(&replay, 1, tag, &flag).unwrap(),
                &flag,
            )
            .unwrap();
            let input = json!({"branchId":"1","tag":tag,"from":100,"to":109});
            let mut actual = read_pages(
                &fixture.store,
                &fixture.key,
                1,
                serde_json::from_value(input.clone()).unwrap(),
                input,
                109,
                true,
                None,
                &flag,
            )
            .unwrap()
            .unwrap();
            actual.as_object_mut().unwrap().remove("summaries");
            actual.as_object_mut().unwrap().remove("campaignSizes");
            let actual: ResourceBalanceReply = serde_json::from_value(actual).unwrap();
            assert_eq!(actual.series, expected.series);
            assert_eq!(actual.deal_hours, expected.deal_hours);
            let mut a = actual.trades;
            let mut b = expected.trades;
            a.sort_by_key(|run| (run.first_hour, run.deal.clone()));
            b.sort_by_key(|run| (run.first_hour, run.deal.clone()));
            assert_eq!(a, b);
            assert_eq!(actual.warnings, expected.warnings);
        }
        let input =
            json!({"branchId":"1","tag":"GER","from":100.25,"to":109.5,"width":2,"hours":[106]});
        let dense = read_pages(
            &fixture.store,
            &fixture.key,
            1,
            serde_json::from_value(input.clone()).unwrap(),
            input,
            109,
            true,
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(dense["summaries"][0]["supply"]["from"], 100.25);
        assert!(
            dense["series"][0]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"].as_i64().unwrap() <= 106
                    && run["lastHour"].as_i64().unwrap() >= 106)
        );
        let input = json!({"branchId":"1","tag":"GER","op":"navigate","key":"ArrowRight","from":102.5,"firstHour":100,"lastHour":109,"resources":[1]});
        assert_eq!(
            read_pages(
                &fixture.store,
                &fixture.key,
                1,
                serde_json::from_value(input.clone()).unwrap(),
                input,
                109,
                true,
                None,
                &flag
            )
            .unwrap()
            .unwrap()["hour"],
            103.0
        );
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn resource_preparation_keeps_5000_countries_and_bounded_continuation() {
        let mut frame = resource_frame(100, 0, 9001, &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    canonical_slot: id,
                    id,
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
        *frame.tables[TableId::CountryResources.index()] =
            TableView::Complete(TableRows::CountryResources(
                (1..=5000)
                    .map(|id| CountryResourceRow {
                        country: id,
                        resource: 1,
                        produced: i64::from(id),
                        ..Default::default()
                    })
                    .collect(),
            ));
        let mut prepared = Prepared::default();
        prepared.observe(&frame).unwrap();
        assert_eq!(prepared.scan.countries.len(), 5000);
        assert_eq!(prepared.keys.len(), 75000); // facts/coverage/membership plus eleven numeric fields.
        for hour in 101..=200 {
            frame.hour = hour;
            prepared.observe(&frame).unwrap();
        }
        assert!(prepared.scan.countries.values().all(|country| {
            country.result.series.values().all(|runs| runs.len() <= 1)
                && country.result.deal_hours.len() <= 1
        }));
        eprintln!(
            "resource continuation: {} countries, {} keys, {} bytes",
            prepared.scan.countries.len(),
            prepared.keys.len(),
            serde_json::to_vec(&prepared).unwrap().len()
        );
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn resource_sampler_and_observer_agree_on_every_native_field_for_5000_countries() {
        let mut frame = resource_frame(100, 0, 9001, &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=5000)
                .map(|id| CountryRow {
                    canonical_slot: id,
                    id,
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
        *frame.tables[TableId::CountryResources.index()] =
            TableView::Complete(TableRows::CountryResources(
                (1..=5000)
                    .map(|id| CountryResourceRow {
                        country: id,
                        resource: 1,
                        produced: i64::from(id),
                        adjustment: -i64::from(id),
                        imported: 7,
                        to_export: 11,
                        exported: 3,
                        used: -9,
                        special_total: -13,
                        production_demand: -17,
                        ..Default::default()
                    })
                    .collect(),
            ));
        let samples = sample::sample(&frame, None).unwrap();
        assert_eq!(samples.len(), 60_000);
        let mut observer = Prepared::default();
        observer.observe(&frame).unwrap();
        assert_eq!(observer.keys.len(), 75_000);
        for ((tag, field), &(identity, amount)) in &samples {
            let (index, resource) = sample::parse(field).unwrap();
            let id = tag
                .strip_prefix("mod country ")
                .unwrap()
                .parse::<u32>()
                .unwrap();
            assert_eq!(identity, Some((id, id)));
            if index < 11 {
                let key = observer.keys[&series::field_key(tag, field)];
                assert_eq!(
                    observer.values[index]
                        .state(key)
                        .unwrap()
                        .last
                        .unwrap()
                        .value,
                    amount.unwrap()
                );
            } else {
                let country = &observer.scan.countries[tag];
                assert_eq!(
                    country.result.series[&resource]
                        .last()
                        .unwrap()
                        .exported_raw,
                    amount.unwrap().to_string()
                );
            }
        }
        let absent = [
            ("missing country".into(), "supply:1".into()),
            ("mod country 5000".into(), "supply:999".into()),
        ];
        assert!(
            sample::sample(&frame, Some(&absent))
                .unwrap()
                .values()
                .all(|value| *value == (None, None))
        );
        *frame.tables[TableId::CountryResources.index()] = TableView::Unavailable {
            reason: "synthetic missing pool".into(),
        };
        assert!(
            sample::sample(&frame, Some(&absent))
                .unwrap()
                .values()
                .all(|value| *value == (None, None))
        );
    }

    #[test]
    fn resource_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing()
     {
        use dashboard_base::test_fixtures::file_source;
        use dashboard_core::test_fixtures::continuation_records;
        use replay::container::Replay;
        let frame = |hour| {
            resource_frame(
                hour,
                if hour % 2 == 0 {
                    i64::from(hour) * 100_000
                } else {
                    900_000
                },
                9001,
                &[],
            )
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
        let input =
            |from| json!({"branchId":"1","tag":"GER","from":from,"to":29,"width":30,"hours":[29]});
        let cold = query(&source, input(0), None, &flag).unwrap().unwrap();
        assert_eq!(
            cold["summaries"][0]["supply"]["approximate"],
            json!([{"firstHour":0,"lastHour":29}])
        );
        assert!(cold["coverage"].is_null());
        assert!(
            query(
                &source,
                json!({"branchId":"1","tag":"GER","from":0,"to":29,"requireComplete":true}),
                None,
                &flag
            )
            .unwrap()
            .is_none(),
            "parity waits through shared pending-coverage polling"
        );
        assert!(
            store.saved_key().unwrap().is_none(),
            "transient queries never publish prepared files"
        );
        let current = cold["series"][0]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|run| run["lastHour"] == 29)
            .unwrap();
        assert_eq!(
            current["producedRaw"], "900000",
            "written operands use an exact state read, not the rough last keyframe"
        );
        let mut prepared = Prepared::default();
        prepared.observe(&frame(0)).unwrap();
        prepared.observe(&frame(1)).unwrap();
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
        let coverage = dashboard_base::prepared::coverage::Coverage {
            schema: dashboard_base::prepared::coverage::SCHEMA,
            first_hour: 0,
            last_hour: 1,
            complete: false,
        };
        writer
            .write_at(
                "preparation-coverage-1",
                0,
                serde_json::to_vec(&coverage).unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let ranges = store.range_count(&key, "resource-values-1-0").unwrap();
        let rough = query(&source, input(0), None, &flag).unwrap().unwrap();
        assert_eq!(
            rough["summaries"][0]["supply"]["approximate"],
            json!([{"firstHour":2,"lastHour":29}])
        );
        assert_eq!(rough["coverage"]["lastHour"], 1);
        let exact = query(&source, input(4), None, &flag).unwrap().unwrap();
        assert_eq!(exact["summaries"][0]["supply"]["approximate"], json!([]));
        let summary: dashboard_base::prepared_chart::ChartSummary =
            serde_json::from_value(exact["summaries"][0]["supply"].clone()).unwrap();
        for hour in 4..=29 {
            let expected =
                sample::sample(&frame(hour), Some(&[("GER".into(), "supply:1".into())])).unwrap();
            let expected =
                expected[&("GER".into(), "supply:1".into())].1.unwrap() as f64 / 100_000.0;
            let run = summary
                .runs
                .as_ref()
                .unwrap()
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .unwrap();
            assert_eq!(run.value, expected);
        }
        let member = query(&source, json!({"branchId":"1","tag":"GER","from":0,"to":29,"width":2,"within":[{"firstHour":6,"lastHour":9}]}), None, &flag).unwrap().unwrap();
        assert_eq!(
            member["summaries"][0]["supply"]["approximate"],
            json!([{"firstHour":6,"lastHour":9}])
        );
        assert_eq!(
            store.range_count(&key, "resource-values-1-0").unwrap(),
            ranges
        );
        let point = query(
            &source,
            json!({"branchId":"1","tag":"GER","from":4,"to":4,"width":1,"hours":[29]}),
            None,
            &flag,
        )
        .unwrap()
        .unwrap();
        assert!(
            point["series"][0]["runs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|run| run["firstHour"] == 29 && run["producedRaw"] == "900000")
        );
        store.clear().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
