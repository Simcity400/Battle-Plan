//! Supply's existing count scan feeding shared runs and synchronized events.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use std::collections::BTreeMap;
const SHARE_DECIMALS: u8 = 12;
const SHARE_SCALE: i128 = 100_000_000_000_000;
/// The product key supply health's queries wait for: with the identity
/// history, while the family is prepared on its own; the branch's otherwise.
const PRODUCT: &str = "supply-health";
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionLandInputs,
    TableId::DivisionStatus,
];
#[cfg(test)]
mod tests;

fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
    defines: Option<Defines>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    let tags = frame
        .complete::<tables::Countries>()
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|row| std::str::from_utf8(&row.tag).ok().map(str::to_owned))
        .collect::<Vec<_>>();
    let mut scan = Scan::new(&tags);
    scan.format = replay::format::FormatVersion::CURRENT;
    if let Some(defines) = defines {
        scan.observe(frame, defines)?;
    } else {
        // No classifier is needed when the recording has no land inputs.
        let _ = scan.countries(frame);
    }
    sample_rows(&scan, requested)
}

fn sample_rows(
    scan: &Scan,
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    sample_indices(scan, &scan.current, requested)
}
fn sample_indices(
    scan: &Scan,
    indices: &[usize],
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    let mut samples = BTreeMap::new();
    for &index in indices {
        let series = &scan.series[index];
        let counts = &scan.counts[index];
        let identity = scan.identities[index];
        let total = counts.map(|counts| counts.into_iter().map(u64::from).sum::<u64>());
        for bucket in 0..3 {
            let count = counts.map(|counts| i128::from(counts[bucket]));
            let share = count
                .zip(total)
                .filter(|(_, total)| *total != 0)
                .map(|(count, total)| count * SHARE_SCALE / i128::from(total));
            samples.insert((series.tag.clone(), bucket.to_string()), (identity, share));
            samples.insert(
                (series.tag.clone(), format!("count-{bucket}")),
                (identity, count),
            );
        }
    }
    if let Some(requested) = requested {
        let tags = requested
            .iter()
            .map(|(tag, _)| tag.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        samples.retain(|(tag, _), _| tags.contains(tag.as_str()));
        for key in requested {
            samples.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(samples)
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    values: [Accumulator; 3],
    runs: Accumulator,
    versions: Vec<i128>,
    defines: Option<Defines>,
    failure: Option<String>,
    warnings: Vec<String>,
}
impl Prepared {
    pub fn configure(
        &mut self,
        source: &Source,
        options: Option<&dashboard_core::config::ContentOptions>,
    ) -> io::Result<()> {
        self.scan.format = source.replay_ref().format;
        if self.defines.is_none() && self.failure.is_none() {
            match options
                .ok_or_else(|| dashboard_core::invalid("Game content is not configured"))
                .and_then(|options| {
                    content::check_replay(
                        source.replay_ref(),
                        options.allow_unverified,
                        &mut self.warnings,
                    )?;
                    Ok(options)
                })
                .and_then(|options| source.content(&options.options))
                .and_then(|content| content_defines(&content.layers))
            {
                Ok(defines) => self.defines = Some(defines),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => self.failure = Some(error.to_string()),
            }
        }
        Ok(())
    }
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        for values in &mut self.values {
            values.advance(frame.hour)?;
        }
        self.runs.advance(frame.hour)?;
        let Some(defines) = self.defines else {
            self.scan.available |= land_present(frame);
            return Ok(());
        };
        if changes.rows_changed[TableId::Countries.index()] || self.scan.countries_from.is_none() {
            let tags = frame
                .complete::<tables::Countries>()
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|row| {
                    #[cfg(test)]
                    {
                        self.scan.work.row_visits += 1;
                    }
                    std::str::from_utf8(&row.tag).ok().map(str::to_owned)
                })
                .collect::<Vec<_>>();
            self.scan.add_tags(&tags);
            self.versions.resize(self.scan.series.len(), 0);
        }
        if !self.scan.observe_changes(frame, defines, Some(changes))? {
            return Ok(());
        }
        let sampled = sample_indices(&self.scan, &self.scan.observed, None)?;
        let observed = self
            .scan
            .observed
            .iter()
            .chain(&self.scan.departed)
            .copied()
            .collect::<Vec<_>>();
        for index in observed {
            let series = &mut self.scan.series[index];
            let run = series.runs.last().filter(|run| run.last_hour == frame.hour);
            for (bucket, values) in self.values.iter_mut().enumerate() {
                let count = sampled
                    .get(&(series.tag.clone(), bucket.to_string()))
                    .and_then(|sample| sample.1);
                values.observe(
                    index as u64,
                    frame.hour,
                    count,
                    run.is_some_and(|run| run.first_hour == frame.hour && run.starts_segment),
                );
                #[cfg(test)]
                {
                    self.scan.work.emitted += 1;
                }
            }
            #[cfg(test)]
            {
                self.scan.work.emitted += 1;
            }
            if let Some(run) = run {
                if run.first_hour == frame.hour {
                    self.versions[index] += 1;
                    self.runs.event(index as u64, frame.hour, run)?;
                }
                self.runs.observe(
                    index as u64,
                    frame.hour,
                    Some(self.versions[index]),
                    run.first_hour == frame.hour && run.starts_segment,
                );
            } else {
                self.runs.observe(index as u64, frame.hour, None, false);
            }
            if series.runs.len() > 1 {
                series.runs.drain(..series.runs.len() - 1);
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.runs.bytes() + self.values.iter().map(Accumulator::bytes).sum::<usize>()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal();
        self.runs
            .flush(writer, &format!("supply-runs-{branch}"), 0, cancelled)?;
        for (bucket, values) in self.values.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("supply-values-{branch}-{bucket}"),
                SHARE_DECIMALS,
                cancelled,
            )?;
        }
        series::write_catalogue(
            writer,
            "supply",
            branch,
            &self
                .scan
                .selected
                .iter()
                .map(|(tag, key)| (tag.clone(), *key as u64))
                .collect(),
            &(
                self.scan.available,
                if self.scan.available {
                    self.failure.as_ref()
                } else {
                    None
                },
                if self.scan.available {
                    self.warnings.as_slice()
                } else {
                    &[]
                },
            ),
        )
    }
}

#[derive(Deserialize)]
struct Viewport {
    from: i32,
    to: i32,
    width: Option<u32>,
}

pub fn query(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<SupplyHealthReply>> {
    let branch = params["branchId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .ok_or_else(|| dashboard_core::invalid("Invalid supply branch"))?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        PRODUCT,
        cancelled,
    )?;
    if !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        return progressive_query(source, params, context, coverage.as_ref(), cancelled).map(Some);
    }
    prepared_query(source, params, context, cancelled)
}

fn query_defines(
    source: &Source,
    context: &LoadContext,
    warnings: &mut Vec<String>,
) -> io::Result<Defines> {
    let options = context.content()?;
    content::check_replay(source.replay_ref(), options.allow_unverified, warnings)?;
    content_defines(&source.content(&options.options)?.layers)
}

fn prepared_query(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<SupplyHealthReply>> {
    let Viewport { from, to, width } = serde_json::from_value(params.clone())?;
    if from > to {
        return Err(dashboard_core::invalid("Invalid supply viewport"));
    }
    if width == Some(0) {
        return Err(dashboard_core::invalid("Invalid supply summary width"));
    }
    let params: SupplyHealthParams = serde_json::from_value(serde_json::json!({
        "branchId": params["branchId"], "tags": params["tags"],
    }))?;
    SupplyHealth::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        PRODUCT,
        cancelled,
    )?;
    let history = dashboard_base::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        source.replay_ref(),
        &params.tags,
        cancelled,
    )?;
    let Some((tags, (available, failure, warnings))) =
        series::read_catalogue::<(bool, Option<String>, Vec<String>)>(
            &store, &key, "supply", branch,
        )?
    else {
        return Ok(None);
    };
    if available && let Some(failure) = failure {
        return Err(dashboard_core::invalid(failure));
    }
    if !available {
        return reply(params.branch_id.clone(), Scan::new(&params.tags), warnings).map(Some);
    }
    let selected =
        dashboard_base::products::country_history::selected_identities(&history, &params.tags);
    let wanted =
        dashboard_base::products::country_history::selected_aliases(&history, &params.tags);
    let mut scan = Scan::new(&wanted);
    scan.available = available;
    let mut rows = Reader::new(&store, &key, format!("supply-runs-{branch}"), cancelled)?;
    let mut display = Vec::new();
    for series in &mut scan.series {
        let Some(&actor) = tags.get(&series.tag) else {
            continue;
        };
        let identities: Vec<_> = history.countries[&series.tag]
            .identities
            .iter()
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .cloned()
            .collect();
        let spans: Vec<_> = identities
            .iter()
            .map(
                |identity| dashboard_base::products::country_groups::HourSpan {
                    first_hour: identity.first_hour.max(from),
                    last_hour: identity.last_hour.min(to),
                },
            )
            .filter(|span| span.first_hour <= span.last_hour)
            .collect();
        let runs = if let Some(width) =
            width.filter(|width| i64::from(to) - i64::from(from) + 1 > i64::from(*width))
        {
            let mut dates = std::collections::BTreeSet::new();
            for bucket in rows
                .summary(
                    actor,
                    (from, to),
                    (f64::from(from), f64::from(to), width),
                    0,
                )?
                .buckets
                .into_iter()
                .flatten()
            {
                dates.extend([bucket.first_hour, bucket.last_hour]);
            }
            let mut summaries = Vec::new();
            for bucket in 0..3 {
                let mut values = Reader::new(
                    &store,
                    &key,
                    format!("supply-values-{branch}-{bucket}"),
                    cancelled,
                )?;
                let summary = values.summary(
                    actor,
                    (from, to),
                    (f64::from(from), f64::from(to), width),
                    SHARE_DECIMALS,
                )?;
                for bucket in summary.buckets.iter().flatten() {
                    dates.extend([
                        bucket.first_hour,
                        bucket.last_hour,
                        bucket.min_hour,
                        bucket.max_hour,
                    ]);
                }
                summaries.push(summary);
            }
            display.push(SupplyHealthDisplay {
                tag: series.tag.clone(),
                values: summaries,
            });
            let mut runs = std::collections::BTreeMap::new();
            for hour in dates {
                if let Some(mut run) = rows.value_at(actor, hour)? {
                    run.first_hour = run.first_hour.max(from);
                    run.last_hour = run.last_hour.min(to);
                    runs.insert(run.first_hour, run);
                }
            }
            runs.into_values().collect()
        } else {
            rows.runs(actor, from, to)?
        };
        dashboard_base::ranking::for_each_piece(
            &runs,
            &spans,
            |run| (run.first_hour, run.last_hour),
            cancelled,
            |_run, first, last| {
                let (_, row) = rows
                    .latest_event(actor, first)?
                    .ok_or_else(|| dashboard_core::invalid("Missing prepared supply counts"))?;
                let mut run: SupplyHealthRun = serde_json::from_value(row)?;
                run.first_hour = first;
                run.last_hour = last;
                series.runs.push(run);
                Ok(())
            },
        )?;
        series.identities = Some(identities);
    }
    let mut reply = reply(params.branch_id.clone(), scan, warnings)?;
    reply.summaries = (!display.is_empty()).then_some(display);
    ensure_reply(&reply)?;
    Ok(Some(reply))
}

pub fn next_change(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use serde_json::{Value, json};
    let selection: SupplyHealthParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"],
    }))?;
    SupplyHealth::validate(&selection).map_err(dashboard_core::invalid)?;
    let navigation: series::Navigation = serde_json::from_value(params.clone())?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let accepted = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        PRODUCT,
        cancelled,
    )?;
    if !accepted.as_ref().is_some_and(|coverage| coverage.complete) {
        use dashboard_base::prepared::query::progressive;
        let defines = std::cell::RefCell::new(None);
        let requested: Vec<_> = selection
            .tags
            .iter()
            .flat_map(|tag| (0..3).map(move |bucket| (tag.clone(), format!("count-{bucket}"))))
            .collect();
        let sampler = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            if land_present(frame) && defines.borrow().is_none() {
                *defines.borrow_mut() = Some(query_defines(source, context, &mut Vec::new())?);
            }
            let mut samples = sample(frame, None, *defines.borrow())?;
            samples.retain(|(_, field), _| field.starts_with("count-"));
            if let Some(requested) = requested {
                for key in requested {
                    samples.entry(key.clone()).or_insert((None, None));
                }
            }
            Ok(samples)
        };
        let tail = progressive::tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
            (
                navigation.first_hour.floor() as i32,
                navigation.last_hour.floor() as i32,
            ),
            &sampler,
            cancelled,
        )?;
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let history = progressive::histories(
            &store,
            &key,
            branch,
            accepted.as_ref(),
            &tail.2,
            &BTreeMap::new(),
            source.replay_ref(),
            &selection.tags,
            cancelled,
        )?;
        let selected = dashboard_base::products::country_history::selected_identities(
            &history,
            &selection.tags,
        );
        let requested: Vec<_> =
            dashboard_base::products::country_history::selected_aliases(&history, &selection.tags)
                .into_iter()
                .flat_map(|tag| (0..3).map(move |bucket| (tag.clone(), format!("count-{bucket}"))))
                .collect();
        let within: BTreeMap<_, _> = history
            .countries
            .iter()
            .map(|(tag, country)| {
                (
                    tag.clone(),
                    country
                        .identities
                        .iter()
                        .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                        .map(
                            |identity| dashboard_base::products::country_groups::HourSpan {
                                first_hour: identity.first_hour,
                                last_hour: identity.last_hour,
                            },
                        )
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let keys = series::read_catalogue::<serde_json::Value>(&store, &key, "supply", branch)?
            .map(|row| row.0)
            .unwrap_or_default();
        let mut reader = Reader::new(&store, &key, format!("supply-runs-{branch}"), cancelled)?;
        let mut prepared = BTreeMap::new();
        for (tag, field) in &requested {
            let actor = keys.get(tag).copied().unwrap_or(u64::MAX);
            let hour = series::navigate(
                &mut [(
                    &mut reader,
                    vec![(actor, within.get(tag).map(Vec::as_slice))],
                )],
                &navigation,
            )?;
            let before = if let Some(coverage) = &accepted {
                if let Some(run) = reader.value_at(actor, coverage.last_hour)? {
                    let (_, row) = reader
                        .latest_event(actor, run.first_hour)?
                        .ok_or_else(|| dashboard_core::invalid("Missing prepared supply counts"))?;
                    let counts: SupplyHealthRun = serde_json::from_value(row)?;
                    let bucket = field
                        .trim_start_matches("count-")
                        .parse::<usize>()
                        .map_err(|_| dashboard_core::invalid("Invalid supply bucket"))?;
                    Some(series::Run::new(
                        run.first_hour,
                        coverage.last_hour,
                        i128::from([counts.no_supply, counts.strained, counts.in_supply][bucket]),
                        run.break_before,
                    ))
                } else {
                    None
                }
            } else {
                None
            };
            prepared.insert((tag.clone(), field.clone()), (hour, before));
        }
        return Ok(
            json!({"hour": progressive::navigate(&prepared, &tail.0, &requested, &within, &navigation)?}),
        );
    }
    let Some(history) = dashboard_base::products::country_history::prepared_branch_histories(
        source,
        branch,
        context.content.as_ref(),
        cancelled,
    )?
    else {
        return Ok(Value::Null);
    };
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let Some((actors, _metadata)) = series::read_catalogue::<(bool, Option<String>, Vec<String>)>(
        &store, &key, "supply", branch,
    )?
    else {
        return Ok(Value::Null);
    };
    let keys =
        dashboard_base::products::country_history::selected_aliases(&history, &selection.tags)
            .iter()
            .filter_map(|tag| actors.get(tag))
            .map(|&key| (key, None))
            .collect();
    let mut reader = Reader::new(&store, &key, format!("supply-runs-{branch}"), cancelled)?;
    let hour = series::navigate(&mut [(&mut reader, keys)], &navigation)?;
    Ok(json!({"hour": hour}))
}

struct PointSample {
    tail: dashboard_base::prepared::query::progressive::TailData,
    available: bool,
    warnings: Vec<String>,
}
impl PointSample {
    fn bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + dashboard_base::prepared::query::progressive::tail_bytes(&self.tail)
            + self.warnings.capacity() * std::mem::size_of::<String>()
            + self.warnings.iter().map(String::capacity).sum::<usize>()
    }
}

fn progressive_query(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<SupplyHealthReply> {
    use dashboard_base::prepared::query::progressive;
    use serde_json::json;
    let Viewport { from, to, width } = serde_json::from_value(params.clone())?;
    if from > to || width == Some(0) {
        return Err(dashboard_core::invalid("Invalid supply viewport"));
    }
    let selection: SupplyHealthParams =
        serde_json::from_value(json!({"branchId": params["branchId"], "tags": params["tags"]}))?;
    SupplyHealth::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid supply branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown supply branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), start);
    let width = width
        .unwrap_or_else(|| (i64::from(to) - i64::from(from) + 1).min(i64::from(u32::MAX)) as u32);
    series::summary_budget(
        f64::from(from),
        f64::from(to),
        width,
        selection.tags.len().saturating_mul(3),
    )?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let requested = selection
        .tags
        .iter()
        .map(|tag| (tag.clone(), "0".into()))
        .collect::<Vec<_>>();
    let read = |coverage| {
        let available = std::cell::Cell::new(false);
        let defines = std::cell::RefCell::new(None);
        let warnings = std::cell::RefCell::new(Vec::new());
        let sampler = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            available.set(available.get() || land_present(frame));
            if land_present(frame) && defines.borrow().is_none() {
                *defines.borrow_mut() =
                    Some(query_defines(source, context, &mut warnings.borrow_mut())?);
            }
            let mut samples = sample(frame, None, *defines.borrow())?;
            if let Some(requested) = requested {
                for key in requested {
                    samples.entry(key.clone()).or_insert((None, None));
                }
            }
            Ok(samples)
        };
        let point = if from == to {
            progressive::frame_at(source, branch, from, TABLES, cancelled)?
        } else {
            None
        };
        let tail = if let Some(frame) = point.filter(|frame| land_present(frame)) {
            let mut tail = progressive::TailData::default();
            let values = sampler(&frame, Some(&requested))?;
            progressive::sampled_spans(
                &mut tail,
                &[dashboard_base::products::country_groups::HourSpan {
                    first_hour: from,
                    last_hour: to,
                }],
                values.iter(),
            );
            tail.1.clear();
            tail
        } else {
            // Missing endpoint inputs cannot decide branch availability.
            // Keep the ordinary fold's availability and refusal evidence.
            progressive::tail(
                source,
                branch,
                coverage,
                TABLES,
                &requested,
                (from, to),
                &sampler,
                cancelled,
            )?
        };
        Ok(PointSample {
            tail,
            available: available.get(),
            warnings: warnings.into_inner(),
        })
    };
    let point = if from == to && coverage.is_none_or(|coverage| from > coverage.last_hour) {
        source.replay_ref().index.iter().find(|block| {
            block.branch == branch && block.first_hour <= from && from <= block.last_hour
        })
    } else {
        None
    };
    let observed = if let Some(block) = point {
        dashboard_base::prepared::query::observations::scope(
            source,
            "supply-health:point",
            Some(&key),
        )
        .get(
            block,
            &(
                from,
                &requested,
                context
                    .content
                    .as_ref()
                    .map(dashboard_base::catalogue::binding),
            ),
            cancelled,
            PointSample::bytes,
            || read(None),
        )?
    } else {
        std::sync::Arc::new(read(coverage)?)
    };
    let tail = &observed.tail;
    let history = progressive::histories(
        &store,
        &key,
        branch,
        coverage,
        &tail.2,
        &BTreeMap::new(),
        source.replay_ref(),
        &selection.tags,
        cancelled,
    )?;
    let mut prefix_params = params.clone();
    let prefix = if let Some(coverage) = coverage.filter(|coverage| from <= coverage.last_hour) {
        prefix_params["to"] = json!(to.min(coverage.last_hour));
        prepared_query(source, &prefix_params, context, cancelled)?
    } else {
        None
    };
    let mut warnings = observed.warnings.clone();
    if let Some(prefix) = &prefix {
        warnings.extend(prefix.warnings.iter().cloned());
    }
    let selected =
        dashboard_base::products::country_history::selected_identities(&history, &selection.tags);
    let mut tags: std::collections::BTreeSet<_> = selection.tags.iter().cloned().collect();
    tags.extend(dashboard_base::products::country_history::selected_aliases(
        &history,
        &selection.tags,
    ));
    if let Some(prefix) = &prefix {
        tags.extend(prefix.series.iter().map(|row| row.tag.clone()));
    }
    let prefix_rows: HashMap<_, _> = prefix
        .as_ref()
        .into_iter()
        .flat_map(|reply| &reply.series)
        .map(|row| (row.tag.as_str(), row))
        .collect();
    let mut scan = Scan::new(&tags.into_iter().collect::<Vec<_>>());
    scan.available = observed.available || prefix.as_ref().is_some_and(|prefix| prefix.available);
    for row in &mut scan.series {
        if let Some(prefix) = prefix_rows.get(row.tag.as_str()) {
            row.runs = prefix.runs.clone();
        }
        let counts = std::array::from_fn::<_, 3, _>(|bucket| {
            tail.0
                .get(&(row.tag.clone(), format!("count-{bucket}")))
                .map_or(&[][..], Vec::as_slice)
        });
        let mut dates = std::collections::BTreeSet::new();
        for runs in counts {
            for run in runs {
                dates.insert(run.first_hour);
                if let Some(end) = run.last_hour.checked_add(1) {
                    dates.insert(end);
                }
            }
        }
        let dates = dates.into_iter().collect::<Vec<_>>();
        for (index, &first) in dates.iter().enumerate().filter(|(_, first)| **first <= to) {
            let last = dates.get(index + 1).map_or(to, |next| next - 1).min(to);
            let reached =
                counts.map(|runs| super::super::parts_race_prepared::transient_at(runs, first));
            if let [Some(no), Some(strained), Some(supplied)] = reached {
                row.runs.push(SupplyHealthRun {
                    first_hour: first,
                    last_hour: last,
                    starts_segment: [no, strained, supplied]
                        .iter()
                        .any(|run| run.break_before && run.first_hour == first),
                    no_supply: no.value as u32,
                    strained: strained.value as u32,
                    in_supply: supplied.value as u32,
                });
            }
        }
        let identities = history
            .countries
            .get(&row.tag)
            .into_iter()
            .flat_map(|country| country.identities.iter())
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .cloned()
            .collect::<Vec<_>>();
        let spans = identities
            .iter()
            .map(
                |identity| dashboard_base::products::country_groups::HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                },
            )
            .collect::<Vec<_>>();
        let mut clipped = Vec::new();
        dashboard_base::ranking::for_each_piece(
            &row.runs,
            &spans,
            |run| (run.first_hour, run.last_hour),
            cancelled,
            |run, first, last| {
                let mut run = run.clone();
                run.first_hour = first;
                run.last_hour = last;
                clipped.push(run);
                Ok(())
            },
        )?;
        row.runs = clipped;
        row.identities = Some(identities);
    }
    let keys = series::read_catalogue::<serde_json::Value>(&store, &key, "supply", branch)?
        .map(|row| row.0)
        .unwrap_or_default();
    let mut share_readers = (0..3)
        .map(|bucket| {
            Reader::new(
                &store,
                &key,
                format!("supply-values-{branch}-{bucket}"),
                cancelled,
            )
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut display = Vec::new();
    for row in &scan.series {
        let spans = row
            .identities
            .iter()
            .flatten()
            .map(
                |identity| dashboard_base::products::country_groups::HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                },
            )
            .collect::<Vec<_>>();
        let mut summaries = Vec::new();
        for (bucket, reader) in share_readers.iter_mut().enumerate() {
            let runs = tail
                .0
                .get(&(row.tag.clone(), bucket.to_string()))
                .map_or(&[][..], Vec::as_slice);
            let mut summary = reader.summary_with_tail(
                keys.get(&row.tag).copied().unwrap_or(u64::MAX),
                bounds,
                (f64::from(from), f64::from(to), width),
                SHARE_DECIMALS,
                runs,
                Some(&spans),
            )?;
            progressive::clip_approximate(&mut summary, &tail.1, Some(&spans));
            summaries.push(summary);
        }
        display.push(SupplyHealthDisplay {
            tag: row.tag.clone(),
            values: summaries,
        });
    }
    let mut reply = reply(selection.branch_id, scan, warnings)?;
    reply.summaries = Some(display);
    ensure_reply(&reply)?;
    Ok(reply)
}
