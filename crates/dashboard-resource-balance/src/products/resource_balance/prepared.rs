//! The existing country step feeds exact pools, independent deals and chart values.
use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::query::progressive::{self, AdapterTail};
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub mod block;
pub use block::FAMILY;

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
    /// While a block's extract records its calls, the deal publications its
    /// fold makes again on the carried state ([`series::record_family`]).
    #[serde(skip)]
    recorded: Option<Vec<Membership>>,
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

/// The Countries fields a country's holder is read from; its id is the key.
static HOLDER_FIELDS: LazyLock<u64> =
    LazyLock::new(|| field_mask(TableId::Countries, &["canonical_slot", "tag"]));

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
/// One country's open deal keys of one resource, published at an hour.
#[derive(Clone)]
struct Membership {
    name: String,
    id: u64,
    members: Vec<u64>,
    /// A member's deal published a new run this hour.
    changed: bool,
    /// The country's deals are known this hour.
    open: bool,
    /// The hour starts the country's known deal span.
    starts: bool,
    hour: i32,
}

/// Close members that left, publish the membership when it or a member's run
/// changed, and its version (the count of publications) while deals are known.
fn publish_membership(
    memberships: &mut BTreeMap<String, Vec<u64>>,
    deals: &mut Accumulator,
    known: &mut Accumulator,
    membership: Membership,
) -> io::Result<()> {
    let Membership {
        name,
        id,
        members,
        changed,
        open,
        starts,
        hour,
    } = membership;
    for &removed in memberships
        .get(&name)
        .into_iter()
        .flatten()
        .filter(|id| !members.contains(id))
    {
        deals.observe(removed, hour, None, false);
    }
    if memberships.get(&name) != Some(&members) || changed {
        known.event(id, hour, &members)?;
        memberships.insert(name, members);
    }
    let version = known.state(id).map_or(0, |state| state.events);
    observe_value(known, id, hour, open.then_some(i128::from(version)), starts);
    Ok(())
}

#[cfg(test)]
thread_local! { static EMISSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(any(test, feature = "test-support"))]
thread_local! { static OBSERVED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
/// Hours this thread's resource observers observed.
#[cfg(any(test, feature = "test-support"))]
pub fn observed_hours() -> usize {
    OBSERVED.with(std::cell::Cell::get)
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
        #[cfg(any(test, feature = "test-support"))]
        OBSERVED.with(|count| count.set(count.get() + 1));
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
            // Recorded changes that delete no country and touch no holder
            // field keep the previous hour's holders.
            let kept = continuous
                && !changes.rows_changed[TableId::CountryLinks.index()]
                && changes
                    .rows(TableId::Countries)
                    .is_some_and(|(changed, deleted)| {
                        deleted.is_empty()
                            && changed.iter().all(|row| row.mask & *HOLDER_FIELDS == 0)
                    });
            #[cfg(test)]
            let kept = kept && !self.whole_tables;
            let same = rows.is_some_and(|rows| {
                self.scan.country_reads.as_ref().is_some_and(|seen| {
                    (kept && self.scan.country_rows == Some(rows.len()))
                        || seen.matches(frame, rows)
                })
            });
            if !same {
                self.scan.country_rows = rows.map(|rows| rows.len());
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
        let touched = match self.observe_rows(frame, changes, continuous && !countries_changed)? {
            Some(touched) => touched,
            None => {
                let names_changed = !self.scan.inputs_ready
                    || changes.rows_changed[TableId::ResourceDefinitions.index()]
                    || changes.rows_changed[TableId::LexicalTokens.index()];
                let changed = [
                    !self.scan.inputs_ready
                        || changes.rows_changed[TableId::CountryResources.index()],
                    !self.scan.inputs_ready
                        || changes.rows_changed[TableId::ResourceExchanges.index()],
                ];
                self.scan
                    .refresh_inputs(frame, [changed[0], changed[1], names_changed]);
                self.scan.observe_resolved(
                    frame,
                    names_changed,
                    if countries_changed || names_changed || !continuous {
                        [true; 2]
                    } else {
                        changed
                    },
                    countries_changed || !continuous,
                )?
            }
        };
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
                    let ids = match country.pool_ids.get(&resource) {
                        Some(&ids) => ids,
                        None => {
                            // The pool key, then its values' keys: the order
                            // they are first written in.
                            let mut ids = [key(&mut self.keys, pool_name(tag, resource)); 12];
                            for (index, id) in ids.iter_mut().enumerate().skip(1) {
                                *id = key(
                                    &mut self.keys,
                                    series::field_key(tag, &sample::field(resource, index - 1)),
                                );
                            }
                            country.pool_ids.insert(resource, ids);
                            ids
                        }
                    };
                    let id = ids[0];
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
                        let amount = run
                            .filter(|_| {
                                self.scan
                                    .current_names
                                    .as_ref()
                                    .is_some_and(|names| names.contains(resource))
                            })
                            .map(|_| native[index]);
                        observe_value(
                            accumulator,
                            ids[index + 1],
                            frame.hour,
                            amount,
                            run.is_some_and(|run| {
                                run.first_hour == frame.hour && run.starts_segment
                            }),
                        );
                    }
                    if run.is_some() {
                        let maximum = self.maxima.entry(pool_name(tag, resource)).or_default();
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
            // Without a narrower set every resource's deals publish again.
            let dirty = country.dirty_deals.take();
            let unchanged = |resource: u32| {
                dirty
                    .as_ref()
                    .is_some_and(|dirty| !dirty.contains(&resource))
            };
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
                if unchanged(run.resource) {
                    continue;
                }
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
                if unchanged(resource) {
                    continue;
                }
                let name = series::field_key(tag, &format!("deals:{resource}"));
                let id = key(&mut self.keys, name.clone());
                let mut members = active.remove(&resource).unwrap_or_default();
                members.sort_unstable();
                let membership = Membership {
                    name,
                    id,
                    members,
                    changed: changed.contains(&resource),
                    open: country.deals_open,
                    starts: country
                        .result
                        .deal_hours
                        .last()
                        .is_some_and(|span| span.first_hour == frame.hour),
                    hour: frame.hour,
                };
                match &mut self.recorded {
                    // A block's extract records the publication for its fold to
                    // make on the carried memberships, versions and deals.
                    Some(recorded) if series::record_family(recorded.len()) => {
                        recorded.push(membership.clone());
                        series::unrecorded(|| {
                            publish_membership(
                                &mut self.memberships,
                                &mut self.deals,
                                &mut self.known,
                                membership,
                            )
                        })?;
                    }
                    _ => publish_membership(
                        &mut self.memberships,
                        &mut self.deals,
                        &mut self.known,
                        membership,
                    )?,
                }
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

    /// Observe the hour from its changed pool and deal rows alone: possible
    /// when it continues the previous hour with the same holders, both tables
    /// changed by exactly their recorded rows, and any dictionary change left
    /// every resource named as before. None when the whole tables are needed.
    fn observe_rows(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &FrameChanges,
        holders_kept: bool,
    ) -> io::Result<Option<BTreeMap<String, [bool; 2]>>> {
        if !holders_kept || !self.scan.inputs_ready {
            return Ok(None);
        }
        #[cfg(test)]
        {
            if self.whole_tables {
                return Ok(None);
            }
        }
        let (Some(pools), Some(deals)) = (
            changes.rows(TableId::CountryResources),
            changes.rows(TableId::ResourceExchanges),
        ) else {
            return Ok(None);
        };
        let names = changes.rows_changed[TableId::ResourceDefinitions.index()]
            || changes.rows_changed[TableId::LexicalTokens.index()];
        if names && !self.scan.same_names(frame) {
            return Ok(None);
        }
        // Any change to the pool table, or the names, observes a slot whose
        // rows repeat a resource again, as the whole table would.
        let repeats = names || changes.rows_changed[TableId::CountryResources.index()];
        let touched = self.scan.observe_rows(frame, pools, deals, repeats)?;
        #[cfg(test)]
        {
            self.row_hours += usize::from(touched.is_some());
        }
        Ok(touched)
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
    let sampled_values = progressive::SamplesCache::default();
    let sample_tables: Vec<_> = sample::TABLES
        .iter()
        .copied()
        .filter(|table| *table != TableId::Campaign)
        .collect();
    let sampler = |frame: &replay_frame::frame::DecodedFrame,
                   requested: Option<&[(String, String)]>| {
        if let (Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::ResourceDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            names.borrow_mut().learn(definitions, lexical);
        }
        let _cpu = replay_frame::cpu::scope_phase("resources:query_sample");
        let mut values =
            sampled_values.sample(frame, &sample_tables, requested, &sample::sample)?;
        if !input.resources.is_empty() {
            values.retain(|(_, field), _| {
                sample::parse(field).is_ok_and(|(_, resource)| input.resources.contains(&resource))
            });
        }
        Ok(values)
    };
    let tail = progressive::tail_projected(
        source,
        "resource-balance:query-tail",
        branch,
        coverage.as_ref(),
        sample::TABLES,
        &requested,
        (view.0.floor() as i32, view.1.floor() as i32),
        &input.resources,
        &sampler,
        || std::mem::take(&mut *names.borrow_mut()),
        |learned| {
            names.borrow_mut().learn_names(learned);
        },
        |learned| {
            let mut bytes = dashboard_base::prepared::prepass::HeldBytes::default();
            bytes.value(learned);
            bytes.total() as usize
        },
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
    let extent = dashboard_base::prepared::prefix::extent(source, branch, coverage)?;
    let mut value = read_pages(
        &store,
        &source_key,
        branch,
        input,
        params,
        last_hour,
        complete,
        context,
        cancelled,
    )?;
    if let Some(value) = &mut value {
        value["coverage"] = json!(extent);
        dashboard_base::products::resources::present_stored_json(
            source,
            content,
            &mut value["resources"],
            cancelled,
        )?;
    }
    Ok(value)
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
    let _cpu = replay_frame::cpu::scope_phase("resources:query_pages");
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
    use replay_data::{
        rows::CountryLinkRow, rows::CountryResourceRow, rows::CountryRow, rows::TableRows,
    };
    use replay_frame::frame::TableView;

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
        if let TableView::Complete(replay_data::rows::TableRows::CountryResources(rows)) =
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
        if let TableView::Complete(replay_data::rows::TableRows::ResourceExchanges(rows)) =
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
        if let TableView::Complete(replay_data::rows::TableRows::CountryResources(rows)) =
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
        if let TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
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
                    if let replay_frame::frame::TableView::Complete(
                        replay_data::rows::TableRows::Countries(rows),
                    ) = &mut *frame.tables[TableId::Countries.index()]
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
                if let replay_frame::frame::TableView::Complete(
                    replay_data::rows::TableRows::CountryResources(rows),
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
        if let TableView::Complete(replay_data::rows::TableRows::CountryResources(rows)) =
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
                && let TableView::Complete(replay_data::rows::TableRows::ResourceExchanges(rows)) =
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

    fn pools(
        frame: &mut replay_frame::frame::DecodedFrame,
    ) -> &mut Vec<replay_data::rows::CountryResourceRow> {
        match &mut *frame.tables[TableId::CountryResources.index()] {
            TableView::Complete(replay_data::rows::TableRows::CountryResources(rows)) => rows,
            _ => unreachable!("fixture pools are complete"),
        }
    }
    fn exchanges(
        frame: &mut replay_frame::frame::DecodedFrame,
    ) -> &mut Vec<replay_data::rows::ResourceExchangeRow> {
        match &mut *frame.tables[TableId::ResourceExchanges.index()] {
            TableView::Complete(replay_data::rows::TableRows::ResourceExchanges(rows)) => rows,
            _ => unreachable!("fixture deals are complete"),
        }
    }
    fn pool(
        frame: &mut replay_frame::frame::DecodedFrame,
        country: u32,
        resource: u32,
    ) -> &mut replay_data::rows::CountryResourceRow {
        pools(frame)
            .iter_mut()
            .find(|row| row.country == country && row.resource == resource)
            .expect("fixture pool")
    }
    fn add_pool(
        frame: &mut replay_frame::frame::DecodedFrame,
        row: replay_data::rows::CountryResourceRow,
    ) {
        let rows = pools(frame);
        rows.push(row);
        rows.sort_by_key(|row| (row.country, row.resource));
    }
    fn deal(
        frame: &mut replay_frame::frame::DecodedFrame,
        id: u64,
    ) -> &mut replay_data::rows::ResourceExchangeRow {
        exchanges(frame)
            .iter_mut()
            .find(|row| row.id == id)
            .expect("fixture deal")
    }
    fn country(
        frame: &mut replay_frame::frame::DecodedFrame,
        id: u32,
    ) -> &mut replay_data::rows::CountryRow {
        match &mut *frame.tables[TableId::Countries.index()] {
            TableView::Complete(replay_data::rows::TableRows::Countries(rows)) => rows
                .iter_mut()
                .find(|row| row.id == id)
                .expect("fixture country"),
            _ => unreachable!("fixture countries are complete"),
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
            input: &replay_frame::frame::DecodedFrame,
            keyframe: bool,
        ) -> (
            replay_frame::frame::DecodedFrame,
            Arc<replay_frame::frame::Frame>,
        ) {
            let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
            let decoded = self.decoder.advance(&wire).unwrap().clone();
            (decoded, Arc::new(wire))
        }
    }

    /// `count` countries holding two pools each, each exporting steel to the next.
    fn many_countries(count: u32) -> replay_frame::frame::DecodedFrame {
        use replay_data::{
            rows::CountryLinkRow, rows::CountryResourceRow, rows::CountryRow,
            rows::ResourceExchangeRow, rows::TableRows,
        };
        let mut frame = resource_frame(100, 0, 9001, &[]);
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=count)
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
                (1..=count)
                    .map(|id| CountryLinkRow {
                        id,
                        canonical_slot: id,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::CountryResources.index()] =
            TableView::Complete(TableRows::CountryResources(
                (1..=count)
                    .flat_map(|country| {
                        [1, 2].map(|resource| CountryResourceRow {
                            country,
                            resource,
                            produced: i64::from(country) * 1_000 + i64::from(resource),
                            ..Default::default()
                        })
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ResourceExchanges.index()] =
            TableView::Complete(TableRows::ResourceExchanges(
                (1..count)
                    .map(|id| ResourceExchangeRow {
                        id: u64::from(id),
                        exporter: id,
                        importer: id + 1,
                        resource: 1,
                        delivered: 100,
                        assigned_factories: 1,
                        required_factories: 1,
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame
    }

    fn assert_rows_observe_only_changed_countries(count: u32) {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests::Work};
        let format = replay_data::format::FormatVersion::CURRENT;
        let mut recorded = Recorded::new();
        let mut tracker = FrameChangeTracker::new();
        let mut step = |prepared: &mut Prepared, frame: &replay_frame::frame::DecodedFrame| {
            let before = (prepared.work, prepared.row_hours);
            let (decoded, wire) = recorded.next(frame, false);
            let changes = tracker.observe_updates(&decoded, wire, format);
            prepared.observe_changed(&decoded, &changes).unwrap();
            let work = Work {
                row_visits: prepared.work.row_visits - before.0.row_visits,
                history_visits: prepared.work.history_visits - before.0.history_visits,
                emitted: prepared.work.emitted - before.0.emitted,
                ..Default::default()
            };
            (work, prepared.row_hours - before.1)
        };
        let mut prepared = Prepared::default();
        let mut frame = many_countries(count);
        let (first, _) = step(&mut prepared, &frame);
        assert_eq!(
            first.history_visits, count as usize,
            "the first hour reads every country"
        );

        frame.hour = 101;
        pool(&mut frame, count / 2, 2).produced += 5;
        let (work, rows) = step(&mut prepared, &frame);
        assert_eq!(rows, 1, "the hour is observed from its changed rows");
        assert_eq!(work.row_visits, 1, "only the changed pool row is read");
        assert_eq!(work.history_visits, 1, "only its country is observed again");
        assert!(
            (1..=12).contains(&work.emitted),
            "only its pool's values are written"
        );

        frame.hour = 102;
        deal(&mut frame, u64::from(count / 3)).delivered += 1;
        let (work, rows) = step(&mut prepared, &frame);
        assert_eq!(rows, 1);
        assert_eq!(work.row_visits, 1, "only the changed deal row is read");
        assert_eq!(
            work.history_visits, 2,
            "only its exporter and importer are observed again"
        );

        // A recorded field no pool reads, and a country field no holder reads.
        frame.hour = 103;
        pool(&mut frame, count / 4, 1).base_export += 7;
        country(&mut frame, count / 4).political_power_raw = Some(7);
        let (work, rows) = step(&mut prepared, &frame);
        assert_eq!(rows, 1);
        assert_eq!(
            (work.row_visits, work.history_visits, work.emitted),
            (0, 0, 0),
            "nothing a pool, deal or holder reads changed"
        );

        // The holders stay those read before while a country's own fields change.
        frame.hour = 104;
        country(&mut frame, count / 5).political_power_raw = Some(9);
        pool(&mut frame, count / 5, 1).imported += 3;
        let (work, rows) = step(&mut prepared, &frame);
        assert_eq!(rows, 1);
        assert_eq!((work.row_visits, work.history_visits), (1, 1));
    }

    #[test]
    fn resource_hour_changing_one_country_observes_only_that_country() {
        assert_rows_observe_only_changed_countries(200);
    }

    #[test]
    fn resource_hour_changing_one_of_5000_countries_observes_only_that_country() {
        assert_rows_observe_only_changed_countries(5000);
    }

    #[test]
    fn resource_operand_masks_name_every_field_they_read() {
        assert_eq!(POOL_FIELDS.count_ones(), 8);
        assert_eq!(DEAL_FIELDS.count_ones(), 7);
        assert_eq!(HOLDER_FIELDS.count_ones(), 2);
        let base_export = (0..64)
            .position(|field| TableId::CountryResources.field_name(field) == Some("base_export"))
            .unwrap();
        assert_eq!(
            *POOL_FIELDS & (1 << base_export),
            0,
            "no pool reads base_export"
        );
    }

    fn pages(
        fixture: &dashboard_base::prepared::series::tests::Fixture,
        tag: &str,
        last: i32,
        width: Option<u32>,
    ) -> Value {
        let mut input = json!({"branchId":"1","tag":tag,"from":100,"to":last});
        if let Some(width) = width {
            input["width"] = json!(width);
        }
        read_pages(
            &fixture.store,
            &fixture.key,
            1,
            serde_json::from_value(input.clone()).unwrap(),
            input,
            last,
            true,
            None,
            &AtomicBool::new(false),
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn resource_rows_observation_matches_whole_tables_and_scan_across_keyframe_gap_and_resume() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        use replay_data::{
            rows::CountryLinkRow, rows::CountryResourceRow, rows::CountryRow,
            rows::LexicalTokenRow, rows::ResourceDefinitionRow, rows::ResourceExchangeRow,
            rows::TableRows,
        };
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut frame = resource_frame(100, 30, 9001, &[(51, 21, 9001, 12), (52, 11, 31, 5)]);
        // A fourth country holding no resources, renamed later.
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
        let mut inputs = Vec::new();
        let (mut stashed_pools, mut stashed_deals) = (None, None);
        for hour in 100..=135 {
            frame.hour = hour;
            match hour {
                102 | 125 | 135 => pool(&mut frame, 11, 1).produced += 1_000,
                103 => deal(&mut frame, 51).delivered += 1,
                // A recorded field no pool reads.
                104 => pool(&mut frame, 21, 1).base_export += 7,
                105 => add_pool(
                    &mut frame,
                    CountryResourceRow {
                        country: 31,
                        resource: 2,
                        produced: 400,
                        used: 400,
                        special_total: 400,
                        ..Default::default()
                    },
                ),
                106 => pools(&mut frame).retain(|row| (row.country, row.resource) != (11, 2)),
                // Germany's other link names steel too: its slot repeats a resource.
                107 => add_pool(
                    &mut frame,
                    CountryResourceRow {
                        country: 9001,
                        resource: 1,
                        produced: 5,
                        ..Default::default()
                    },
                ),
                108 | 128 => pool(&mut frame, 21, 1).produced += 1,
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
                110 => deal(&mut frame, 52).exporter = 21,
                111 => exchanges(&mut frame).retain(|row| row.id != 51),
                112 => exchanges(&mut frame).push(ResourceExchangeRow {
                    id: 53,
                    exporter: 31,
                    importer: 21,
                    resource: 1,
                    delivered: 9,
                    assigned_factories: 1,
                    required_factories: 1,
                    ..Default::default()
                }),
                113 | 132 => country(&mut frame, 21).political_power_raw = Some(i64::from(hour)),
                114 => {
                    pool(&mut frame, 11, 1).produced += 1_000;
                    deal(&mut frame, 53).delivered += 1;
                    country(&mut frame, 21).political_power_raw = Some(114);
                }
                115 | 116 => pool(&mut frame, 31, 2).produced += 1,
                117 => pools(&mut frame).retain(|row| row.country != 9001),
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
                119 => add_pool(
                    &mut frame,
                    CountryResourceRow {
                        country: 21,
                        resource: 3,
                        produced: 77,
                        used: 77,
                        special_total: 77,
                        ..Default::default()
                    },
                ),
                120 => deal(&mut frame, 53).convoys_assigned = 2,
                121 => country(&mut frame, 41).tag = b"VIC".to_vec(),
                122 => pool(&mut frame, 21, 3).produced += 1,
                123 | 131 => deal(&mut frame, 52).delivered += 1,
                124 => continue,
                126 => {
                    stashed_pools = Some(std::mem::take(pools(&mut frame)));
                    *frame.tables[TableId::CountryResources.index()] = TableView::Unavailable {
                        reason: "synthetic missing pools".into(),
                    };
                }
                127 => {
                    *frame.tables[TableId::CountryResources.index()] = TableView::Complete(
                        TableRows::CountryResources(stashed_pools.take().unwrap()),
                    );
                    pool(&mut frame, 21, 1).produced += 1;
                }
                129 => {
                    stashed_deals = Some(std::mem::take(exchanges(&mut frame)));
                    *frame.tables[TableId::ResourceExchanges.index()] = TableView::Unavailable {
                        reason: "synthetic missing deals".into(),
                    };
                }
                130 => {
                    *frame.tables[TableId::ResourceExchanges.index()] = TableView::Complete(
                        TableRows::ResourceExchanges(stashed_deals.take().unwrap()),
                    );
                    deal(&mut frame, 53).delivered += 1;
                }
                133 => {
                    pool(&mut frame, 31, 2).used += 3;
                    deal(&mut frame, 52).importer = 9001;
                }
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
                source_id: "resource-rows".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let context = LoadContext { content: None };
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
            if ![110, 121, 135].contains(&decoded.hour) {
                continue;
            }
            for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
                let mut writer = fixture.writer();
                observer
                    .flush(&mut writer, &source, &context, 1, &flag)
                    .unwrap();
                writer.commit().unwrap();
            }
            let [incremental, whole] = &observers;
            assert_eq!(
                incremental.keys, whole.keys,
                "keys are allocated in one order"
            );
            assert_eq!(incremental.memberships, whole.memberships);
            assert_eq!(incremental.maxima, whole.maxima);
            let states = |observer: &Prepared| {
                [&observer.pools, &observer.deals, &observer.known]
                    .into_iter()
                    .chain(&observer.values)
                    .map(|accumulator| serde_json::to_value(accumulator).unwrap())
                    .collect::<Vec<_>>()
            };
            assert_eq!(states(incremental), states(whole));
            for tag in ["GER", "ITA", "SPR", "FRA", "VIC"] {
                for width in [None, Some(4)] {
                    assert_eq!(
                        pages(&fixtures[0], tag, decoded.hour, width),
                        pages(&fixtures[1], tag, decoded.hour, width),
                        "{tag} published through hour {}",
                        decoded.hour
                    );
                }
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
            observers[0].row_hours >= 15,
            "most hours are observed from their changed rows, not {}",
            observers[0].row_hours
        );
        assert_eq!(observers[1].row_hours, 0);
        for tag in ["ITA", "SPR"] {
            let mut expected = super::super::reply(
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
            let mut actual: ResourceBalanceReply =
                serde_json::from_value(pages(&fixtures[0], tag, 135, None)).unwrap();
            assert_eq!(actual.series, expected.series, "{tag}");
            assert_eq!(actual.deal_hours, expected.deal_hours, "{tag}");
            for trades in [&mut actual.trades, &mut expected.trades] {
                trades.sort_by_key(|run| (run.first_hour, run.deal.clone()));
            }
            assert_eq!(actual.trades, expected.trades, "{tag}");
            assert_eq!(actual.warnings, expected.warnings, "{tag}");
        }
    }

    #[test]
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
        let selected = [("mod country 5000".into(), String::new())];
        let before = sample::ROW_PROJECTS.get();
        let one = sample::sample(&frame, Some(&selected)).unwrap();
        assert_eq!(
            sample::ROW_PROJECTS.get() - before,
            1,
            "projected unselected native pools"
        );
        assert_eq!(
            one,
            samples
                .iter()
                .filter(|((tag, _), _)| tag == "mod country 5000")
                .map(|(key, value)| (key.clone(), *value))
                .collect()
        );
        let cache = progressive::SamplesCache::default();
        let expected = cache
            .sample(
                &frame,
                &sample::TABLES[1..],
                Some(&selected),
                &sample::sample,
            )
            .unwrap();
        let before = sample::ROW_PROJECTS.get();
        let mut later = frame.clone();
        later.hour += 1;
        assert_eq!(
            cache
                .sample(
                    &later,
                    &sample::TABLES[1..],
                    Some(&selected),
                    &sample::sample
                )
                .unwrap(),
            expected
        );
        assert_eq!(
            sample::ROW_PROJECTS.get(),
            before,
            "reprojected unchanged native pools"
        );

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
        let before = dashboard_base::prepared::query::observations::usage(
            &source,
            "resource-balance:query-tail",
        );
        assert!(before.2 > 0);
        assert_eq!(
            query(&source, input(0), None, &flag).unwrap().unwrap(),
            cold
        );
        let after = dashboard_base::prepared::query::observations::usage(
            &source,
            "resource-balance:query-tail",
        );
        assert_eq!(
            after.2, before.2,
            "re-asked the same immutable resource blocks"
        );
        assert!(after.0 <= after.1);

        assert_eq!(
            cold["summaries"][0]["supply"]["approximate"],
            json!([{"firstHour":0,"lastHour":29}])
        );
        assert!(
            cold["coverage"]["firstHour"]
                .as_i64()
                .is_some_and(|first| cold["coverage"]["lastHour"].as_i64() == first.checked_sub(1))
                && cold["coverage"]["complete"] == false
        );
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
