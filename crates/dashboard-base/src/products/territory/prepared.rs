//! One territory observation feeds totals, regional attribution and rank positions.
use super::*;
use crate::prepared::series::{self, Accumulator, Reader, Run};
use serde_json::{Value, json};
use std::collections::BTreeSet;

/// Native counter used by the scoreboard, separate from States ownership totals.
pub const NATIVE_OWNED_STATES: usize = 8;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    #[serde(skip)]
    catalogue_writer: series::CatalogueWriter,
    scan: Scan,
    builder: Builder,
    tags: BTreeMap<String, u64>,
    values: [Accumulator; NATIVE_OWNED_STATES + 1],
    ranks: [Accumulator; 8],
    facts: Accumulator,
    present: Accumulator,
    rosters: Accumulator,
    world: Accumulator,
    breakdown: super::super::territory_breakdown::Prepared,
    first: Option<i32>,
    last: i32,
    area_unit: Option<String>,
    area_reason: Option<String>,
    warnings: Vec<String>,
    #[serde(skip)]
    inputs: Option<AreaData>,
}

impl Prepared {
    pub fn observe_owned_states(
        &mut self,
        frame: &DecodedFrame,
        identities: &mut super::super::country_history::HistoryScan,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        self.values[NATIVE_OWNED_STATES].advance(frame.hour)?;
        identities.observe_owned_states(frame, changes, |tag, amount, starts| {
            let next = self.tags.len() as u64;
            let key = *self.tags.entry(tag.to_owned()).or_insert(next);
            self.values[NATIVE_OWNED_STATES].observe(
                key,
                frame.hour,
                amount.map(i128::from),
                starts,
            );
        });
        Ok(())
    }

    pub fn configure(
        &mut self,
        source: &Source,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if self.inputs.is_none() {
            let inputs = area_data(source, context, cancelled)?;
            self.area_unit = inputs
                .areas
                .as_ref()
                .map(|_| ProvinceAreas::UNIT.to_owned());
            self.area_reason.clone_from(&inputs.reason);
            self.warnings.clone_from(&inputs.warnings);
            self.inputs = Some(inputs);
        }
        self.breakdown.configure(source, context)
    }

    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        cancelled: &AtomicBool,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_with(frame, format, cancelled, Some(changes))?;
        self.breakdown.observe(
            frame,
            format,
            self.inputs
                .as_ref()
                .and_then(|inputs| inputs.areas.as_deref()),
            changes,
        )
    }

    fn observe_with(
        &mut self,
        frame: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        cancelled: &AtomicBool,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("territory:observe");
        check_cancel(cancelled)?;
        for values in self.values.iter_mut().take(NATIVE_OWNED_STATES) {
            values.advance(frame.hour)?;
        }
        self.facts.advance(frame.hour)?;
        self.present.advance(frame.hour)?;
        self.rosters.advance(frame.hour)?;
        self.world.advance(frame.hour)?;
        let was_roster = self.builder.previous_hour.is_some();
        let contiguous = self
            .builder
            .previous_hour
            .and_then(|hour| hour.checked_add(1))
            == Some(frame.hour);
        let was_world = was_roster
            .then(|| self.scan.world_totals.last().map(|run| run.first_hour))
            .flatten();
        self.scan.format = format;
        observe(
            &mut self.scan,
            frame,
            self.inputs
                .as_ref()
                .and_then(|inputs| inputs.areas.as_deref()),
            &mut self.builder,
            changes,
        )?;
        self.first.get_or_insert(frame.hour);
        self.last = frame.hour;
        if was_roster != self.builder.previous_hour.is_some() || !contiguous {
            self.rosters.observe(
                0,
                frame.hour,
                (self.builder.previous_hour == Some(frame.hour)).then_some(1),
                false,
            );
        }
        for (tag, changed) in &self.builder.changed {
            let track = &self.scan.series[tag];
            let next = self.tags.len() as u64;
            let key = *self.tags.entry(tag.to_string()).or_insert(next);
            let current = track.runs.last().filter(|run| run.last_hour == frame.hour);
            for (column, values) in self.values.iter_mut().take(NATIVE_OWNED_STATES).enumerate() {
                let amount = current.and_then(|run| run.values[column]).map(i128::from);
                let starts =
                    current.is_some_and(|run| run.first_hour == frame.hour && run.starts_segment);
                if changed[column] {
                    values.observe(key, frame.hour, amount, starts);
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.builder.work.emitted += 1;
                    }
                }
            }
            if let Some(run) = current {
                if run.first_hour == frame.hour {
                    self.facts.event(key, frame.hour, run)?;
                }
                self.facts.observe_if_changed(
                    key,
                    frame.hour,
                    Some(i128::from(run.first_hour)),
                    run.first_hour == frame.hour && run.starts_segment,
                    false,
                );
            } else {
                self.facts
                    .observe_if_changed(key, frame.hour, None, false, false);
            }
            self.present.observe(
                key,
                frame.hour,
                track
                    .present
                    .last()
                    .filter(|span| span.last_hour == frame.hour)
                    .map(|_| 1),
                false,
            );
        }
        if let Some(run) = self
            .scan
            .world_totals
            .last()
            .filter(|_| self.builder.previous_hour == Some(frame.hour))
        {
            if run.first_hour == frame.hour {
                self.world.event(0, frame.hour, run)?;
            }
            if was_world != Some(run.first_hour) || !contiguous {
                self.world.observe(
                    0,
                    frame.hour,
                    Some(i128::from(run.first_hour)),
                    run.first_hour == frame.hour && run.starts_segment,
                );
            }
        } else if was_world.is_some() {
            self.world.observe(0, frame.hour, None, false);
        }

        Ok(())
    }

    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .map(Accumulator::bytes)
            .sum::<usize>()
            + self.facts.bytes()
            + self.present.bytes()
            + self.rosters.bytes()
            + self.world.bytes()
            + self.breakdown.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        let mut payloads = series::Payloads::default();
        self.builder.seal(&mut self.scan);
        let runs: BTreeMap<_, _> = self
            .scan
            .series
            .iter()
            .map(|(tag, track)| {
                (
                    tag.to_string(),
                    track
                        .runs
                        .iter()
                        .filter(|run| run.last_hour >= first)
                        .map(|run| TerritoryRun {
                            first_hour: run.first_hour.max(first),
                            ..run.clone()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for column in 0..8 {
            self.ranks[column].through(first);
            crate::ranking::rank_runs(
                &runs,
                |run| (run.first_hour, run.last_hour),
                |run| run.values[column].map(i128::from),
                cancelled,
                |rank, tag, from, to| {
                    self.ranks[column].push(
                        rank as u64 * 2,
                        Run::new(from, to, i128::from(self.tags[tag]), false),
                    );
                    self.ranks[column].push(
                        self.tags[tag] * 2 + 1,
                        Run::new(from, to, rank as i128 + 1, false),
                    );
                    Ok(())
                },
            )?;
            self.ranks[column].through(self.last);
            self.ranks[column].flush_with_payloads(
                writer,
                &format!("territory-ranks-{branch}-{column}"),
                0,
                cancelled,
                &mut payloads,
            )?;
            self.values[column].flush_with_payloads(
                writer,
                &format!("territory-values-{branch}-{column}"),
                0,
                cancelled,
                &mut payloads,
            )?;
        }
        self.values[NATIVE_OWNED_STATES].flush_with_payloads(
            writer,
            &format!("territory-values-{branch}-{NATIVE_OWNED_STATES}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.facts.flush_with_payloads(
            writer,
            &format!("territory-facts-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.present.flush_with_payloads(
            writer,
            &format!("territory-present-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.rosters.flush_with_payloads(
            writer,
            &format!("territory-rosters-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.world.flush_with_payloads(
            writer,
            &format!("territory-world-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.breakdown.flush(writer, branch, cancelled)?;
        for track in self.scan.series.values_mut() {
            keep_last(&mut track.runs);
            keep_last(&mut track.segments);
            keep_last(&mut track.present);
        }
        keep_last(&mut self.scan.rosters);
        keep_last(&mut self.scan.world_totals);
        let mut warnings = self.warnings.clone();
        scan_warning(&self.scan, &mut warnings);
        self.catalogue_writer.write(
            writer,
            "territory",
            branch,
            &self.tags,
            &TerritoryReply {
                branch_id: branch.to_string(),
                world_totals: Vec::new(),
                series: Vec::new(),
                area_unit: self.area_unit.clone(),
                area_reason: self.area_reason.clone(),
                warnings,
            },
        )
    }
}

fn keep_last<T>(rows: &mut Vec<T>) {
    if rows.len() > 1 {
        rows.drain(..rows.len() - 1);
    }
}

/// Reconstruct only the requested facts, preserving composite-value run boundaries.
fn runs(reader: &mut Reader<'_>, key: u64, from: i32, to: i32) -> io::Result<Vec<TerritoryRun>> {
    let markers = if from == to {
        reader.value_at(key, from)?.into_iter().collect()
    } else {
        reader.runs(key, from, to)?
    };
    markers
        .into_iter()
        .map(|marker| {
            let (_, row) = reader
                .latest_event(key, marker.first_hour)?
                .ok_or_else(|| crate::invalid("Territory facts are missing"))?;
            let mut run: TerritoryRun = serde_json::from_value(row)?;
            run.first_hour = marker.first_hour;
            run.last_hour = marker.last_hour;
            run.starts_segment &= run.first_hour == marker.value as i32;
            Ok(run)
        })
        .collect()
}

fn transient_ranks(
    values: &BTreeMap<(String, String), Vec<Run>>,
    field: &str,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<Run>>> {
    let transient: BTreeMap<String, Vec<Run>> = values
        .iter()
        .filter(|((tag, name), _)| !tag.is_empty() && name == field)
        .map(|((tag, _), runs)| (tag.clone(), runs.clone()))
        .collect();
    let mut ranks: BTreeMap<String, Vec<Run>> = BTreeMap::new();
    crate::ranking::rank_positions(
        &transient,
        |run| (run.first_hour, run.last_hour),
        |run| Some(run.value),
        cancelled,
        |rank, tag, first, last| {
            ranks.entry(tag.to_owned()).or_default().push(Run::new(
                first,
                last,
                rank as i128 + 1,
                true,
            ));
            Ok(())
        },
    )?;
    Ok(ranks)
}

fn sampled_point(
    values: &BTreeMap<(String, String), Vec<Run>>,
    tag: &str,
    hour: i32,
) -> Vec<TerritoryRun> {
    let fields: Vec<_> = SAMPLE_FIELDS
        .iter()
        .map(|field| {
            values
                .get(&(tag.to_owned(), (*field).to_owned()))
                .and_then(|runs| {
                    runs.iter()
                        .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                })
        })
        .collect();
    let Some(first_hour) = fields.iter().flatten().map(|run| run.first_hour).max() else {
        return Vec::new();
    };
    let last_hour = fields
        .iter()
        .flatten()
        .map(|run| run.last_hour)
        .min()
        .unwrap();
    vec![TerritoryRun {
        first_hour,
        last_hour,
        starts_segment: fields
            .iter()
            .flatten()
            .any(|run| run.break_before && run.first_hour == first_hour),
        values: std::array::from_fn(|column| {
            fields[column].and_then(|run| i64::try_from(run.value).ok())
        }),
    }]
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<Read>,
    #[serde(default)]
    within: BTreeMap<String, Vec<HourSpan>>,
    #[serde(default)]
    unobserved: Vec<HourSpan>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    measure: Option<TerritoryMeasure>,
    rule: Option<TerritoryRule>,
    top: Option<u64>,
    #[serde(default)]
    metadata: bool,
    #[serde(default)]
    breakdown: bool,
    #[serde(default)]
    aggregate: bool,
}
#[derive(Deserialize)]
struct Read {
    hour: i32,
}

pub fn query(
    source: &Source,
    context: &LoadContext,
    product: &str,
    value: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(value.clone())?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid territory branch"))?;
    for tag in &params.tags {
        validate_tag(tag).map_err(crate::invalid)?;
    }
    if params.family.top == Some(0) || params.family.top.is_some() && !params.tags.is_empty() {
        return Err(crate::invalid("Invalid top countries"));
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    // The family's own coverage with the identity history's while it is
    // prepared on its own.
    let accepted = match crate::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        <super::Territory as crate::products::Product>::KEY,
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        other => other?,
    };
    let partial = !accepted.as_ref().is_some_and(|coverage| coverage.complete);
    // A direct regional read and its country totals carry every column, areas included.
    let regional_tail = partial && params.family.breakdown && product == "series-state";
    let inputs = if params.family.metadata
        || regional_tail
        || matches!(params.family.measure, Some(TerritoryMeasure::Area))
    {
        Some(area_data(source, context, cancelled)?)
    } else {
        None
    };
    let (mut tags, metadata) = if accepted.is_some() {
        series::read_catalogue::<TerritoryReply>(&store, &key, "territory", branch)?
    } else {
        None
    }
    .unwrap_or_else(|| {
        (
            BTreeMap::new(),
            TerritoryReply {
                branch_id: params.branch_id.clone(),
                world_totals: Vec::new(),
                series: Vec::new(),
                area_unit: inputs.as_ref().and_then(|inputs| {
                    inputs
                        .areas
                        .as_ref()
                        .map(|_| ProvinceAreas::UNIT.to_owned())
                }),
                area_reason: inputs.as_ref().and_then(|inputs| inputs.reason.clone()),
                warnings: inputs
                    .as_ref()
                    .map_or_else(Vec::new, |inputs| inputs.warnings.clone()),
            },
        )
    });
    let warnings = metadata.warnings;
    if params.family.metadata {
        return Ok(serde_json::to_value(TerritoryReply {
            branch_id: params.branch_id,
            world_totals: Vec::new(),
            series: Vec::new(),
            area_unit: metadata.area_unit,
            area_reason: metadata.area_reason,
            warnings,
        })?);
    }
    // A reached regional hour is one frame's state; other regional views need the scan.
    if partial && params.family.breakdown && !regional_tail {
        let end = params.reads.iter().map(|read| read.hour).max();
        if accepted
            .as_ref()
            .is_none_or(|coverage| end.is_none_or(|end| end > coverage.last_hour))
        {
            return Ok(Value::Null);
        }
    }
    let measure = params
        .family
        .measure
        .ok_or_else(|| crate::invalid("Missing territory measure"))?;
    let rule = params
        .family
        .rule
        .ok_or_else(|| crate::invalid("Missing territory rule"))?;
    let column = measure.index(rule);
    let field = SAMPLE_FIELDS[column];
    let requested_tags: BTreeSet<_> = tags.keys().chain(params.tags.iter()).cloned().collect();
    let seed_tags: Vec<String> = requested_tags
        .iter()
        .filter(|tag| !tag.is_empty())
        .cloned()
        .collect();
    crate::prepared::query::observations::bind(source, &key);
    // SourceKey binds the content files, but not the verification choice or
    // whether the current area read succeeded. Both affect this projection.
    let sample_product = match inputs.as_ref().map(|inputs| {
        (
            context
                .content
                .as_ref()
                .is_some_and(|content| content.allow_unverified),
            inputs.areas.is_some(),
        )
    }) {
        None => "territory:counts",
        Some((false, false)) => "territory:areas:strict:unavailable",
        Some((false, true)) => "territory:areas:strict:available",
        Some((true, false)) => "territory:areas:allow-unverified:unavailable",
        Some((true, true)) => "territory:areas:allow-unverified:available",
    };
    let sample = |frame: &DecodedFrame, _requested: Option<&[(String, String)]>| {
        super::sample(
            frame,
            None,
            inputs.as_ref().and_then(|inputs| inputs.areas.as_deref()),
            source.replay_ref().format,
        )
    };
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown territory branch"))?;
    let bounds = crate::metadata::branch_bounds(source.replay_ref(), start);
    let sampled = if partial && product == "series-state" {
        let hours: Vec<_> = params.reads.iter().map(|read| read.hour).collect();
        crate::prepared::query::progressive::state_tail_all_cached(
            source,
            sample_product,
            branch,
            accepted.as_ref(),
            TABLES,
            &hours,
            &sample,
            cancelled,
        )?
    } else if partial && matches!(product, "series-summary" | "series-next-change") {
        let window = if product == "series-summary" {
            series::range(params.from, params.to)?
        } else {
            series::range(value["firstHour"].as_f64(), value["lastHour"].as_f64())?
        };
        crate::prepared::query::progressive::tail_all_cached(
            source,
            sample_product,
            branch,
            accepted.as_ref(),
            TABLES,
            (window.0.floor() as i32, window.1.floor() as i32),
            &sample,
            cancelled,
        )?
    } else {
        Default::default()
    };
    let cpu = replay_frame::cpu::scope_phase("territory:query_identities");
    let history = crate::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        accepted.as_ref(),
        &sampled.2,
        &BTreeMap::new(),
        source.replay_ref(),
        &seed_tags,
        cancelled,
    )?;
    drop(cpu);
    let rank_tail = if params.family.top.is_some() {
        transient_ranks(&sampled.0, field, cancelled)?
    } else {
        BTreeMap::new()
    };
    let mut next_actor = tags
        .values()
        .max()
        .copied()
        .map_or(Some(0), |key| key.checked_add(1));
    for tag in sampled
        .0
        .keys()
        .filter(|(tag, _)| !tag.is_empty())
        .map(|(tag, _)| tag)
        .chain(requested_tags.iter())
    {
        if !tags.contains_key(tag) {
            let actor =
                next_actor.ok_or_else(|| crate::invalid("Territory key allocation overflow"))?;
            next_actor = actor.checked_add(1);
            tags.insert(tag.clone(), actor);
        }
    }
    let progressive = partial.then_some((
        bounds,
        &sampled.0,
        sampled.1.as_slice(),
        &sampled.2,
        accepted.is_some(),
    ));
    let mut ranks = Reader::new(
        &store,
        &key,
        format!("territory-ranks-{branch}-{column}"),
        cancelled,
    )?;
    let inverse: BTreeMap<_, _> = tags.iter().map(|(tag, &key)| (key, tag)).collect();
    let selected: Vec<String> = if params.family.top.is_some() || params.tags.is_empty() {
        tags.keys().cloned().collect()
    } else {
        super::super::country_history::selected_aliases(&history, &params.tags)
    };
    let selected_ids = (!params.tags.is_empty() && params.family.top.is_none())
        .then(|| super::super::country_history::selected_identities(&history, &params.tags));
    let mut holders = BTreeMap::new();
    let mut excluded = BTreeMap::new();
    for tag in &selected {
        if let Some(ids) = &selected_ids
            && !ids.is_empty()
        {
            let country = history.countries.get(tag);
            let mut included = Vec::new();
            let mut omitted = Vec::new();
            for identity in country
                .into_iter()
                .flat_map(|country| country.identities.iter())
            {
                let span = HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                };
                if ids.contains(&(identity.slot, identity.country_id)) {
                    included.push(span);
                } else {
                    omitted.push(span);
                }
            }
            let mut spans = Vec::new();
            if params.within.is_empty() {
                spans = included;
            } else {
                crate::ranking::for_each_piece(
                    &included,
                    params.within.get(tag).map_or(&[], Vec::as_slice),
                    |span| (span.first_hour, span.last_hour),
                    cancelled,
                    |_, first_hour, last_hour| {
                        spans.push(HourSpan {
                            first_hour,
                            last_hour,
                        });
                        Ok(())
                    },
                )?;
            }
            holders.insert(tag.clone(), spans);
            excluded.insert(tag.clone(), omitted);
        } else if let Some(spans) = params.within.get(tag) {
            holders.insert(tag.clone(), spans.clone());
        }
    }
    let mut values = Reader::new(
        &store,
        &key,
        format!("territory-values-{branch}-{column}"),
        cancelled,
    )?;
    match product {
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            if params.family.aggregate {
                let summary = aggregate_summary(
                    &store,
                    &key,
                    branch,
                    column,
                    &tags,
                    &selected,
                    &params.within,
                    &params.unobserved,
                    params.family.top,
                    (from, to, width),
                    &excluded,
                    progressive.map(|tail| (tail, &rank_tail)),
                    cancelled,
                )?;
                let end = summary
                    .buckets
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .map(|bucket| bucket.last_hour);
                let end_sampled = if partial && let Some(end) = end {
                    crate::prepared::query::progressive::state_tail_all_cached(
                        source,
                        sample_product,
                        branch,
                        accepted.as_ref(),
                        TABLES,
                        &[end],
                        &sample,
                        cancelled,
                    )?
                } else {
                    Default::default()
                };
                let end_ranks = transient_ranks(&end_sampled.0, field, cancelled)?;
                let end_history = partial
                    .then(|| {
                        crate::prepared::query::progressive::histories(
                            &store,
                            &key,
                            branch,
                            accepted.as_ref(),
                            &sampled.2,
                            &end_sampled.2,
                            source.replay_ref(),
                            &seed_tags,
                            cancelled,
                        )
                    })
                    .transpose()?;
                let end_history = end_history.as_ref().unwrap_or(&history);
                let end_selected: Vec<_> = if params.tags.is_empty() {
                    tags.keys()
                        .chain(
                            end_sampled
                                .0
                                .keys()
                                .filter(|(tag, _)| !tag.is_empty())
                                .map(|(tag, _)| tag),
                        )
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect()
                } else {
                    super::super::country_history::selected_aliases(end_history, &params.tags)
                };
                let end_ids = (!params.tags.is_empty()).then(|| {
                    super::super::country_history::selected_identities(end_history, &params.tags)
                });
                let mut end_tags = tags.clone();
                let mut next_end_actor = end_tags
                    .values()
                    .max()
                    .copied()
                    .map_or(Some(0), |key| key.checked_add(1));
                let mut end_excluded = excluded.clone();
                for tag in &end_selected {
                    if !end_tags.contains_key(tag) {
                        let next = next_end_actor
                            .ok_or_else(|| crate::invalid("Territory key allocation overflow"))?;
                        next_end_actor = next.checked_add(1);
                        end_tags.insert(tag.clone(), next);
                    }
                    if let Some(identities) = end_sampled.2.get(&(tag.clone(), field.to_owned())) {
                        let omitted: Vec<_> = identities
                            .iter()
                            .filter(|(_, _, slot, id)| {
                                end_ids
                                    .as_ref()
                                    .is_some_and(|ids| !ids.contains(&(*slot, *id)))
                            })
                            .map(|&(first_hour, last_hour, _, _)| HourSpan {
                                first_hour,
                                last_hour,
                            })
                            .collect();
                        end_excluded.insert(tag.clone(), omitted);
                    }
                }
                let sum = if let Some(end) = end {
                    aggregate_summary(
                        &store,
                        &key,
                        branch,
                        column,
                        &end_tags,
                        &end_selected,
                        &params.within,
                        &params.unobserved,
                        params.family.top,
                        (f64::from(end), f64::from(end), 1),
                        &end_excluded,
                        partial.then_some((
                            (
                                bounds,
                                &end_sampled.0,
                                end_sampled.1.as_slice(),
                                &end_sampled.2,
                                accepted.is_some(),
                            ),
                            &end_ranks,
                        )),
                        cancelled,
                    )?
                    .runs
                    .unwrap_or_default()
                } else {
                    Vec::new()
                };
                return Ok(
                    json!({"branchId":params.branch_id,"measure":measure,"decimals":0,"series":[{"tag":"group","summary":summary,"marks":null}],"sum":sum,"warnings":warnings}),
                );
            }
            let bounds = (from.floor() as i32, to.ceil() as i32);
            let mut lines = Vec::new();
            let mut end_reads: BTreeMap<i32, Vec<(usize, String)>> = BTreeMap::new();
            let mut facts =
                Reader::new(&store, &key, format!("territory-facts-{branch}"), cancelled)?;
            for tag in selected {
                check_cancel(cancelled)?;
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                let ranked = ranks.summary_with_tail(
                    actor * 2 + 1,
                    bounds,
                    (from, to, width),
                    0,
                    rank_tail.get(&tag).map_or(&[][..], Vec::as_slice),
                    None,
                )?;
                if let Some(top) = params.family.top
                    && !ranked
                        .buckets
                        .iter()
                        .flatten()
                        .any(|bucket| bucket.min_value <= top as f64)
                {
                    continue;
                }
                let mut summary = values.summary_with_tail(
                    actor,
                    bounds,
                    (from, to, width),
                    0,
                    sampled
                        .0
                        .get(&(tag.clone(), field.to_owned()))
                        .map_or(&[][..], Vec::as_slice),
                    holders.get(&tag).map(Vec::as_slice),
                )?;
                if let Some(top) = params.family.top {
                    let membership = ranked;
                    for (bucket, rank) in summary.buckets.iter_mut().zip(membership.buckets) {
                        if rank.is_none_or(|rank| rank.min_value > top as f64) {
                            *bucket = None;
                        }
                    }
                    // Exact windows get their dated membership from the state reader.
                    if let Some(exact) = &membership.runs {
                        let spans: Vec<_> = exact
                            .iter()
                            .filter(|run| run.value <= top as f64)
                            .map(|run| HourSpan {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                            })
                            .collect();
                        summary = values.summary_with_tail(
                            actor,
                            bounds,
                            (from, to, width),
                            0,
                            sampled
                                .0
                                .get(&(tag.clone(), field.to_owned()))
                                .map_or(&[][..], Vec::as_slice),
                            Some(&spans),
                        )?;
                    } else {
                        summary.runs = None;
                    }
                }
                crate::prepared::query::progressive::clip_approximate(
                    &mut summary,
                    &sampled.1,
                    holders.get(&tag).map(Vec::as_slice),
                );
                series::summary_budget(from, to, width, lines.len() + 1)?;
                let end_runs = summary
                    .buckets
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .map(|bucket| {
                        if accepted
                            .as_ref()
                            .is_some_and(|coverage| bucket.last_hour <= coverage.last_hour)
                        {
                            runs(&mut facts, actor, bucket.last_hour, bucket.last_hour)
                        } else {
                            end_reads
                                .entry(bucket.last_hour)
                                .or_default()
                                .push((lines.len(), tag.clone()));
                            Ok(Vec::new())
                        }
                    })
                    .transpose()?
                    .unwrap_or_default();
                let country = history.countries.get(&tag);
                let identities = country.map(|country| {
                    country
                        .identities
                        .iter()
                        .filter(|identity| {
                            selected_ids.as_ref().is_none_or(|ids| {
                                ids.contains(&(identity.slot, identity.country_id))
                            })
                        })
                        .collect::<Vec<_>>()
                });
                lines.push(json!({"tag": tag, "summary": summary, "marks": null,"endRuns":end_runs,
                    "segments":holders.get(&tag).or_else(||country.map(|country|country.segments.as_ref())),
                    "identities":identities}));
            }
            let mut exact_identities = BTreeMap::new();
            for (hour, ends) in end_reads {
                // One target hour always uses the core's exact state path. Countries
                // sharing an endpoint share this read, and no dated values accumulate.
                let point = crate::prepared::query::progressive::state_tail_all_cached(
                    source,
                    sample_product,
                    branch,
                    accepted.as_ref(),
                    TABLES,
                    &[hour],
                    &sample,
                    cancelled,
                )?;
                for (index, tag) in ends {
                    let included = params.tags.contains(&tag)
                        || selected_ids.as_ref().is_none_or(|ids| {
                            point
                                .2
                                .get(&(tag.clone(), field.to_owned()))
                                .is_some_and(|spans| {
                                    spans.iter().any(|&(first, last, slot, id)| {
                                        first <= hour && last >= hour && ids.contains(&(slot, id))
                                    })
                                })
                        });
                    lines[index]["endRuns"] = serde_json::to_value(if included {
                        sampled_point(&point.0, &tag, hour)
                    } else {
                        Vec::new()
                    })?;
                }
                // Native holder evidence is independent of whether this measure is known.
                for (lookup, identities) in point.2 {
                    if lookup.1 == field {
                        exact_identities
                            .entry(lookup)
                            .or_insert_with(Vec::new)
                            .extend(identities);
                    }
                }
            }
            if !exact_identities.is_empty() {
                let joined = crate::prepared::query::progressive::histories(
                    &store,
                    &key,
                    branch,
                    accepted.as_ref(),
                    &sampled.2,
                    &exact_identities,
                    source.replay_ref(),
                    &seed_tags,
                    cancelled,
                )?;
                let ids = (!params.tags.is_empty() && params.family.top.is_none()).then(|| {
                    super::super::country_history::selected_identities(&joined, &params.tags)
                });
                for line in &mut lines {
                    let tag = line["tag"].as_str().expect("territory line tag");
                    if let Some(country) = joined.countries.get(tag) {
                        let identities: Vec<_> = country
                            .identities
                            .iter()
                            .filter(|identity| {
                                ids.as_ref().is_none_or(|ids| {
                                    ids.contains(&(identity.slot, identity.country_id))
                                })
                            })
                            .collect();
                        let spans: Vec<_> = identities
                            .iter()
                            .map(|identity| HourSpan {
                                first_hour: identity.first_hour,
                                last_hour: identity.last_hour,
                            })
                            .collect();
                        line["identities"] = serde_json::to_value(identities)?;
                        line["segments"] = serde_json::to_value(spans)?;
                    }
                }
            }
            Ok(
                json!({"branchId":params.branch_id,"measure":measure,"decimals":0,"series": lines, "warnings": warnings}),
            )
        }
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(value)?;
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut rank_keys = Vec::new();
            let mut value_keys = Vec::new();
            if let Some(top) = params.family.top {
                for rank in 0..top.min(tags.len() as u64) {
                    rank_keys.push(rank * 2);
                    if let Some(run) = ranks.value_at(rank * 2, hour)? {
                        value_keys.push((run.value as u64, None));
                    }
                }
            } else {
                value_keys.extend(selected.iter().filter_map(|tag| {
                    tags.get(tag)
                        .map(|&key| (key, holders.get(tag).map(Vec::as_slice)))
                }));
            }
            let rank_keys = rank_keys.into_iter().map(|key| (key, None)).collect();
            if partial {
                let mut prepared = BTreeMap::new();
                let mut tail = BTreeMap::new();
                let mut requested = Vec::new();
                for tag in &selected {
                    let include = params.family.top.is_none_or(|top| {
                        value_keys
                            .iter()
                            .any(|(actor, _)| tags.get(tag) == Some(actor))
                            || rank_tail.get(tag).is_some_and(|runs| {
                                runs.iter().any(|run| {
                                    run.first_hour <= hour
                                        && run.last_hour >= hour
                                        && run.value <= i128::from(top)
                                })
                            })
                    });
                    if !include {
                        continue;
                    }
                    let lookup = (tag.clone(), field.to_owned());
                    if let Some(&actor) = tags.get(tag) {
                        let candidate = series::navigate(
                            &mut [(
                                &mut values,
                                vec![(actor, holders.get(tag).map(Vec::as_slice))],
                            )],
                            &navigation,
                        )?;
                        let preceding = sampled
                            .0
                            .get(&lookup)
                            .and_then(|runs| runs.first())
                            .and_then(|run| run.first_hour.checked_sub(1))
                            .map(|hour| values.value_at(actor, hour))
                            .transpose()?
                            .flatten();
                        prepared.insert(lookup.clone(), (candidate, preceding));
                    }
                    if let Some(runs) = sampled.0.get(&lookup) {
                        tail.insert(lookup.clone(), runs.clone());
                    }
                    requested.push(lookup);
                }
                if let Some(top) = params.family.top {
                    for rank in 0..top.min(tags.len() as u64) {
                        let lookup = (String::new(), format!("rank-{rank}"));
                        let mut rows = Vec::new();
                        for (tag, runs) in &rank_tail {
                            rows.extend(
                                runs.iter()
                                    .filter(|run| run.value == i128::from(rank) + 1)
                                    .map(|run| Run {
                                        value: i128::from(tags[tag]),
                                        ..*run
                                    }),
                            );
                        }
                        rows.sort_by_key(|run| run.first_hour);
                        let candidate = series::navigate(
                            &mut [(&mut ranks, vec![(rank * 2, None)])],
                            &navigation,
                        )?;
                        let preceding = rows
                            .first()
                            .and_then(|run| run.first_hour.checked_sub(1))
                            .map(|hour| ranks.value_at(rank * 2, hour))
                            .transpose()?
                            .flatten();
                        prepared.insert(lookup.clone(), (candidate, preceding));
                        tail.insert(lookup.clone(), rows);
                        requested.push(lookup);
                    }
                }
                let hour = crate::prepared::query::progressive::navigate(
                    &prepared,
                    &tail,
                    &requested,
                    &holders,
                    &navigation,
                )?;
                let approximate = hour.is_some_and(|hour| {
                    sampled.1.iter().any(|span| {
                        f64::from(span.first_hour) <= hour && f64::from(span.last_hour) >= hour
                    })
                });
                return Ok(json!({"hour":hour,"approximate":approximate}));
            }
            Ok(
                json!({"hour": series::navigate(&mut [(&mut values, value_keys), (&mut ranks, rank_keys)], &navigation)?}),
            )
        }
        "series-state" => {
            let _cpu = replay_frame::cpu::scope_phase("territory:query_state");
            let [read] = params.reads.as_slice() else {
                return Err(crate::invalid("Territory state needs one reached hour"));
            };
            let (from, to) = (read.hour, read.hour);
            let mut scan = Scan::default();
            let mut world =
                Reader::new(&store, &key, format!("territory-world-{branch}"), cancelled)?;
            scan.world_totals = runs(&mut world, 0, from, to)?;
            if partial
                && accepted
                    .as_ref()
                    .is_none_or(|coverage| from > coverage.last_hour)
            {
                scan.world_totals = sampled_point(&sampled.0, "", from);
            }
            let mut rosters = Reader::new(
                &store,
                &key,
                format!("territory-rosters-{branch}"),
                cancelled,
            )?;
            scan.rosters = rosters
                .runs(0, from, to)?
                .into_iter()
                .map(|run| HourSpan {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                })
                .collect();
            if partial
                && accepted
                    .as_ref()
                    .is_none_or(|coverage| from > coverage.last_hour)
            {
                scan.rosters = sampled
                    .0
                    .get(&(String::new(), "roster".into()))
                    .into_iter()
                    .flatten()
                    .filter(|run| run.first_hour <= from && run.last_hour >= from)
                    .map(|run| HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    })
                    .collect();
            }
            let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
                params.family.top.map(|_| BTreeMap::new());
            if let Some(members) = &mut membership {
                for rank in 0..params.family.top.unwrap().min(tags.len() as u64) {
                    for run in ranks.runs(rank * 2, from, to)? {
                        members
                            .entry(inverse[&(run.value as u64)].to_string())
                            .or_default()
                            .push(HourSpan {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                            });
                    }
                }
                for (tag, runs) in &rank_tail {
                    for run in runs.iter().filter(|run| {
                        run.value <= i128::from(params.family.top.unwrap())
                            && run.first_hour <= from
                            && run.last_hour >= from
                    }) {
                        members.entry(tag.clone()).or_default().push(HourSpan {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                        });
                    }
                }
                // Rank-position iteration is not chronological for a tag.
                for spans in members.values_mut() {
                    spans.sort_by_key(|span| span.first_hour);
                    let mut merged = Vec::new();
                    for span in std::mem::take(spans) {
                        super::super::country_groups::mark_unobserved(
                            &mut merged,
                            span.first_hour,
                            span.last_hour,
                        );
                    }
                    *spans = merged;
                }
            }
            let wanted = membership.as_ref().map_or_else(
                || selected.clone(),
                |members| members.keys().cloned().collect(),
            );
            let mut facts =
                Reader::new(&store, &key, format!("territory-facts-{branch}"), cancelled)?;
            let mut present = Reader::new(
                &store,
                &key,
                format!("territory-present-{branch}"),
                cancelled,
            )?;
            for tag in wanted {
                let _cpu = replay_frame::cpu::scope_phase("territory:query_country");
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                let segments = history
                    .countries
                    .get(&tag)
                    .map_or_else(Vec::new, |country| country.segments.as_ref().clone());
                scan.series.insert(
                    tag.clone().into(),
                    Track {
                        runs: if partial
                            && accepted
                                .as_ref()
                                .is_none_or(|coverage| from > coverage.last_hour)
                        {
                            sampled_point(&sampled.0, &tag, from)
                        } else {
                            runs(&mut facts, actor, from, to)?
                        },
                        segments,
                        present: if partial
                            && accepted
                                .as_ref()
                                .is_none_or(|coverage| from > coverage.last_hour)
                        {
                            sampled
                                .0
                                .get(&(tag.to_string(), "present".into()))
                                .into_iter()
                                .flatten()
                                .filter(|run| run.first_hour <= from && run.last_hour >= from)
                                .map(|run| HourSpan {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                })
                                .collect()
                        } else {
                            present
                                .runs(actor, from, to)?
                                .into_iter()
                                .map(|run| HourSpan {
                                    first_hour: run.first_hour,
                                    last_hour: run.last_hour,
                                })
                                .collect()
                        },
                        last: None,
                    },
                );
            }
            let selection = TerritoryParams {
                branch_id: params.branch_id.clone(),
                tags: if params.tags.is_empty() {
                    tags.keys().cloned().collect()
                } else {
                    params.tags
                },
                top: None,
            };
            let totals = TerritoryReply {
                series: selected_series(&scan, &selection, cancelled, Some(&history), membership)?,
                world_totals: scan.world_totals,
                branch_id: params.branch_id,
                area_unit: metadata.area_unit,
                area_reason: metadata.area_reason,
                warnings,
            };
            if params.family.aggregate {
                let sum = aggregate_summary(
                    &store,
                    &key,
                    branch,
                    column,
                    &tags,
                    &selected,
                    &params.within,
                    &params.unobserved,
                    params.family.top,
                    (f64::from(from), f64::from(to), 1),
                    &excluded,
                    progressive.map(|tail| (tail, &rank_tail)),
                    cancelled,
                )?;
                let mut value = serde_json::to_value(totals)?;
                value["sum"] = serde_json::to_value(sum.runs.unwrap_or_default())?;
                Ok(value)
            } else if params.family.breakdown {
                let reply = if regional_tail
                    && accepted
                        .as_ref()
                        .is_none_or(|coverage| from > coverage.last_hour)
                {
                    super::super::territory_breakdown::sampled_reply(
                        source,
                        context,
                        branch,
                        from,
                        inputs.as_ref().and_then(|inputs| inputs.areas.as_deref()),
                        totals,
                        cancelled,
                    )
                } else {
                    super::super::territory_breakdown::Prepared::reply(
                        &store, &key, branch, from, to, totals, cancelled,
                    )
                };
                reply.and_then(|reply| serde_json::to_value(reply).map_err(Into::into))
            } else {
                Ok(serde_json::to_value(totals)?)
            }
        }
        "events-page" => {
            let (from, to, cursor, size) = series::event_window(
                params.from,
                params.to,
                value["cursor"].as_str(),
                value["pageSize"]
                    .as_u64()
                    .and_then(|size| u32::try_from(size).ok()),
            )?;
            let mut facts =
                Reader::new(&store, &key, format!("territory-facts-{branch}"), cancelled)?;
            let keys: Vec<_> = selected
                .iter()
                .filter_map(|tag| tags.get(tag))
                .copied()
                .collect();
            let page = facts.events(&keys, from, to, cursor, size)?;
            Ok(
                json!({"firstOrdinal":page.first.to_string(),"total":page.total.to_string(),"cursor":page.cursor.map(|cursor|cursor.to_string()),"rows":page.rows.into_iter().map(|row|json!({"tag":inverse[&row.key],"hour":row.hour,"ordinal":row.ordinal.to_string(),"row":row.row})).collect::<Vec<_>>(),"warnings":warnings}),
            )
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let tag = value["tag"]
                .as_str()
                .ok_or_else(|| crate::invalid("Missing territory tag"))?;
            let actor = tags
                .get(tag)
                .ok_or_else(|| crate::invalid("Unknown territory tag"))?;
            let mut facts =
                Reader::new(&store, &key, format!("territory-facts-{branch}"), cancelled)?;
            Ok(
                json!({"tag":tag,"summary":facts.event_clusters(*actor,from,to,width)?,"warnings":warnings}),
            )
        }
        _ => Err(crate::invalid("Unknown territory query")),
    }
}

/// The same window sum serves fixed picks and dated smart-group membership.
#[allow(clippy::too_many_arguments)]
fn aggregate_summary(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    column: usize,
    tags: &BTreeMap<String, u64>,
    selected: &[String],
    within: &BTreeMap<String, Vec<HourSpan>>,
    unobserved: &[HourSpan],
    top: Option<u64>,
    view: (f64, f64, u32),
    excluded: &BTreeMap<String, Vec<HourSpan>>,
    progressive: Option<(
        crate::prepared::query::progressive::AdapterTail<'_>,
        &BTreeMap<String, Vec<Run>>,
    )>,
    cancelled: &AtomicBool,
) -> io::Result<crate::prepared_chart::ChartSummary> {
    let mut values = Reader::new(
        store,
        key,
        format!("territory-values-{branch}-{column}"),
        cancelled,
    )?;
    let mut present = Reader::new(store, key, format!("territory-present-{branch}"), cancelled)?;
    let mut rosters = Reader::new(store, key, format!("territory-rosters-{branch}"), cancelled)?;
    let mut ranks = Reader::new(
        store,
        key,
        format!("territory-ranks-{branch}-{column}"),
        cancelled,
    )?;
    let bindings: Vec<Vec<_>> = selected
        .iter()
        .map(|tag| {
            tags.get(tag)
                .map(|&key| (key, i32::MIN, i32::MAX, None))
                .into_iter()
                .collect()
        })
        .collect();
    let members: Vec<_> = selected
        .iter()
        .zip(&bindings)
        .map(|(tag, keys)| crate::prepared::group_sum::Member {
            keys,
            present_keys: keys,
            rank_key: tags.get(tag).map(|&key| key * 2 + 1),
            within: within.get(tag).map(Vec::as_slice),
            excluded: excluded.get(tag).map_or(&[], Vec::as_slice),
            zeros: &[],
            tail: progressive
                .filter(|_| !keys.is_empty())
                .map(|(tail, ranks)| {
                    (
                        tail.1
                            .get(&(tag.clone(), SAMPLE_FIELDS[column].to_owned()))
                            .map_or(&[][..], Vec::as_slice),
                        tail.1
                            .get(&(tag.clone(), "present".into()))
                            .map_or(&[][..], Vec::as_slice),
                        ranks.get(tag).map_or(&[][..], Vec::as_slice),
                    )
                }),
        })
        .collect();
    let coverage = progressive
        .map(|(tail, _)| {
            let mut coverage = rosters.summary_with_tail(
                0,
                tail.0,
                view,
                0,
                tail.1
                    .get(&(String::new(), "roster".into()))
                    .map_or(&[][..], Vec::as_slice),
                None,
            )?;
            crate::prepared::query::progressive::clip_approximate(&mut coverage, tail.2, None);
            Ok::<_, io::Error>(coverage)
        })
        .transpose()?;
    let mut summary = crate::prepared::group_sum::summary(
        &mut values,
        &mut present,
        &mut rosters,
        coverage.as_ref(),
        top.map(|top| (&mut ranks, top)),
        &members,
        unobserved,
        view,
        cancelled,
    )?;
    if let Some(coverage) = coverage {
        crate::prepared::query::progressive::clip_approximate(
            &mut summary,
            &coverage.approximate,
            None,
        );
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepared::series::tests::Fixture;

    #[test]
    fn equal_territory_fact_markers_carry_without_materializing_each_key() {
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut frame = super::super::tests::frame(100, 64, 64);
        let mut prepared = Prepared::default();
        prepared.observe_with(&frame, format, &flag, None).unwrap();
        let visits = prepared.facts.key_visits;
        frame.hour += 1;
        prepared.observe_with(&frame, format, &flag, None).unwrap();
        assert!(!prepared.builder.changed.is_empty());
        assert_eq!(prepared.facts.key_visits, visits);
        prepared.facts.seal();
        for key in prepared.tags.values() {
            assert_eq!(
                prepared.facts.state(*key).unwrap().last.unwrap().last_hour,
                101
            );
        }
    }

    #[test]
    fn cached_area_state_refuses_after_unverified_content_is_disabled() {
        area_state_verification_transition(true);
    }

    #[test]
    fn cached_area_state_recovers_after_unverified_content_is_enabled() {
        area_state_verification_transition(false);
    }

    fn area_state_verification_transition(initially_allowed: bool) {
        use crate::test_fixtures::{CAMPAIGN, content_identity, file_source, records};
        let root =
            std::env::temp_dir().join(format!("territory-area-binding-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("user")).unwrap();
        std::fs::write(root.join("hoi4.exe"), "test").unwrap();
        std::fs::write(
            root.join("user/dlc_load.json"),
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        let (options, identity) = content_identity(&root);
        let mut input = records(
            1,
            vec![
                super::super::tests::frame(0, 2, 4),
                super::super::tests::frame(1, 2, 4),
            ],
        );
        let replay::spool::Record::SessionStart(session) = &mut input[0] else {
            unreachable!()
        };
        session.identity = identity;
        let (source, recording_root) = file_source(
            replay::container::Replay::from_records(CAMPAIGN.into(), &[input]).unwrap(),
        );
        assert!(matches!(
            source.replay_ref().identity.verification,
            replay_data::identity::Verification::Unchecked
        ));
        let mut context = LoadContext {
            content: Some(options),
        };
        let resolved = source
            .content(&context.content.as_ref().unwrap().options)
            .unwrap();
        let areas = Arc::new(ProvinceAreas {
            land: (0..=4).map(|id| i64::from(id) * 10).collect(),
        });
        // Supply a synthetic raster through the real area's read cache.
        // load_areas still checks the current recording verification first.
        source.state::<super::super::Cache>().lock().unwrap().areas =
            Some((Arc::clone(&resolved), Ok(Arc::clone(&areas))));
        let flag = AtomicBool::new(false);
        let (store, key) = crate::prepared::files(&source, context.content.as_ref()).unwrap();
        assert!(
            crate::prepared::coverage::coverage(&source, context.content.as_ref(), 1, &flag)
                .unwrap()
                .is_none()
        );
        let params = json!({"branchId":"1", "family":{"kind":"territory",
            "measure":"area", "rule":"controlled"}, "tags":[], "reads":[{"hour":1}]});
        let ask = |context: &LoadContext| -> TerritoryReply {
            serde_json::from_value(
                query(&source, context, "series-state", params.clone(), &flag).unwrap(),
            )
            .unwrap()
        };
        let check = |reply: &TerritoryReply, available: bool| {
            let country = reply
                .series
                .iter()
                .find(|country| country.tag == "realm 00000")
                .unwrap();
            assert_eq!(country.runs.len(), 1);
            assert_eq!(country.runs[0].values[3], available.then_some(60));
            assert_eq!(
                country.runs[0].values[0],
                Some(1),
                "independent counts stay known"
            );
            assert_eq!(reply.area_unit.is_some(), available);
        };
        // Exercise the lower census cache directly; the whole-reply cache
        // cannot invalidate or mask a stale area observation in this test.
        for allowed in [initially_allowed, !initially_allowed, initially_allowed] {
            context.content.as_mut().unwrap().allow_unverified = allowed;
            assert_eq!(
                crate::prepared::files(&source, context.content.as_ref())
                    .unwrap()
                    .1,
                key
            );
            let inputs = area_data(&source, &context, &flag).unwrap();
            assert_eq!(inputs.areas.is_some(), allowed);
            let reply = ask(&context);
            check(&reply, allowed);
            if !allowed {
                assert!(
                    reply
                        .area_reason
                        .as_ref()
                        .unwrap()
                        .contains("recording has unverified content")
                );
            }
            assert_eq!(
                serde_json::to_value(ask(&context)).unwrap(),
                serde_json::to_value(reply).unwrap()
            );
        }
        // Availability is a separate operand even when permission and
        // content stamps do not move.
        context.content.as_mut().unwrap().allow_unverified = true;
        source.state::<super::super::Cache>().lock().unwrap().areas = Some((
            Arc::clone(&resolved),
            Err("synthetic area input unavailable".into()),
        ));
        assert_eq!(
            crate::prepared::files(&source, context.content.as_ref())
                .unwrap()
                .1,
            key
        );
        check(&ask(&context), false);
        source.state::<super::super::Cache>().lock().unwrap().areas = Some((resolved, Ok(areas)));
        check(&ask(&context), true);
        store.clear().unwrap();
        std::fs::remove_dir_all(recording_root).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn territory_sampler_and_observer_agree_on_every_key_at_five_thousand_countries() {
        let flag = AtomicBool::new(false);
        let frame = super::super::tests::frame(100, 5_000, 10_000);
        let areas = Arc::new(ProvinceAreas {
            land: (0..=10_000).map(|id| i64::from(id) * 7).collect(),
        });
        let mut prepared = Prepared {
            inputs: Some(AreaData {
                areas: Some(Arc::clone(&areas)),
                reason: None,
                warnings: Vec::new(),
            }),
            ..Default::default()
        };
        let sampled = super::super::sample(&frame, None, Some(&areas), Default::default()).unwrap();
        prepared
            .observe_with(&frame, Default::default(), &flag, None)
            .unwrap();
        assert_eq!(
            prepared.builder.aggregations, 1,
            "one land aggregation supplies every numeric channel"
        );
        assert_eq!(sampled.len(), 5_000 * 9 + 9);
        for (tag, &actor) in &prepared.tags {
            let identity = prepared.scan.series[tag.as_str()].last.unwrap().0;
            for (column, field) in SAMPLE_FIELDS.iter().enumerate() {
                assert_eq!(
                    sampled[&(tag.clone(), (*field).to_owned())],
                    (
                        Some(identity),
                        prepared.values[column]
                            .state(actor)
                            .and_then(|state| state.last)
                            .map(|run| run.value)
                    ),
                    "{tag} {field}"
                );
            }
            assert_eq!(
                sampled[&(tag.clone(), "present".into())],
                (Some(identity), Some(1))
            );
        }
        for (column, field) in SAMPLE_FIELDS.iter().enumerate() {
            assert_eq!(
                sampled[&(String::new(), (*field).to_owned())].1,
                prepared.scan.world_totals.last().unwrap().values[column].map(i128::from)
            );
        }
        assert_eq!(sampled[&(String::new(), "roster".into())], (None, Some(1)));
        let requested = vec![
            ("realm 04999".into(), "ownedArea".into()),
            ("absent opaque tag".into(), "controlledStates".into()),
        ];
        let selected =
            super::super::sample(&frame, Some(&requested), Some(&areas), Default::default())
                .unwrap();
        assert_eq!(selected.len(), requested.len());
        assert_eq!(selected[&requested[0]], sampled[&requested[0]]);
        assert_eq!(selected[&requested[1]], (None, None));
    }

    fn counter_frame(hour: i32) -> DecodedFrame {
        use replay_data::{rows::CountryCounterInputRow, rows::CountryLinkRow, rows::TableRows};
        use replay_frame::frame::TableView;
        let mut frame = super::super::tests::frame(hour, 3, 8);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[2].tag.clear();
            if hour >= 105 {
                rows[0].tag = b"renamed counter holder".to_vec();
                rows[1].id += 10;
            }
            rows.sort_by_key(|row| row.id);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            if hour >= 105 {
                rows[1].id += 10;
            }
            if hour == 101 {
                rows.push(CountryLinkRow {
                    id: 99,
                    canonical_slot: 0,
                });
            }
            rows.sort_by_key(|row| row.id);
        }
        let mut counts = vec![
            CountryCounterInputRow {
                country: 100_000,
                owned_state_count: if hour == 109 { 0 } else { -2 },
                ..Default::default()
            },
            CountryCounterInputRow {
                country: if hour >= 105 { 100_011 } else { 100_001 },
                owned_state_count: 50,
                ..Default::default()
            },
            CountryCounterInputRow {
                country: 100_002,
                owned_state_count: 900,
                ..Default::default()
            },
        ];
        if hour == 101 {
            // Distinct raw links may name one slot; raw table keys remain unique.
            counts.push(CountryCounterInputRow {
                country: 99,
                owned_state_count: 99,
                ..Default::default()
            });
        }
        if hour == 107 {
            counts.remove(0);
        }
        counts.sort_by_key(|row| row.country);
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(counts));
        let unavailable = match hour {
            102 => Some(TableId::CountryCounterInputs),
            103 => Some(TableId::Countries),
            108 => Some(TableId::CountryLinks),
            _ => None,
        };
        if let Some(table) = unavailable {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "synthetic counter dependency".into(),
            };
        }
        frame
    }

    #[test]
    fn territory_prepared_native_owned_states_share_history_join_after_commits_resume_append() {
        use super::super::super::country_history::HistoryScan;
        let mut fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
        let mut identities = HistoryScan::new(false, &[]);
        let mut reference = HistoryScan::new(true, &[]);
        for (commit, hours) in [
            vec![100, 101],
            vec![102, 103, 105],
            vec![106, 107, 108, 109],
        ]
        .into_iter()
        .enumerate()
        {
            for hour in hours {
                let frame = counter_frame(hour);
                let changes = tracker.observe(&frame, Default::default());
                reference.observe(&frame);
                prepared
                    .observe_owned_states(&frame, &mut identities, Some(&changes))
                    .unwrap();
                prepared
                    .observe_with(&frame, Default::default(), &cancelled, Some(&changes))
                    .unwrap();
            }
            let mut writer = if commit == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &cancelled).unwrap();
            writer.commit().unwrap();
            tracker.invalidate();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            identities = serde_json::from_slice(&serde_json::to_vec(&identities).unwrap()).unwrap();
            let native =
                serde_json::from_slice::<HistoryScan>(&serde_json::to_vec(&reference).unwrap())
                    .unwrap()
                    .finish();
            let identity_only =
                serde_json::from_slice::<HistoryScan>(&serde_json::to_vec(&identities).unwrap())
                    .unwrap()
                    .finish();
            assert_eq!(
                identity_only.unreadable_territory,
                native.unreadable_territory
            );
            assert!(
                identity_only
                    .countries
                    .values()
                    .all(|country| country.territory.is_empty()),
                "the shared identity state must not duplicate numeric history"
            );
            let mut values = Reader::new(
                &fixture.store,
                &fixture.key,
                format!("territory-values-1-{NATIVE_OWNED_STATES}"),
                &cancelled,
            )
            .unwrap();
            assert!(!prepared.tags.contains_key(""));
            for (tag, country) in &native.countries {
                if tag.is_empty() {
                    continue;
                }
                let expected: Vec<_> = country
                    .territory
                    .iter()
                    .map(|run| {
                        Run::new(
                            run.first_hour,
                            run.last_hour,
                            i128::from(run.value),
                            run.starts_segment,
                        )
                    })
                    .collect();
                assert_eq!(
                    values.runs(prepared.tags[tag], 100, 109).unwrap(),
                    expected,
                    "{tag}, commit {commit}"
                );
            }
        }
        let mut values = Reader::new(
            &fixture.store,
            &fixture.key,
            "territory-values-1-8".into(),
            &cancelled,
        )
        .unwrap();
        let first = prepared.tags["realm 00000"];
        let second = prepared.tags["realm 00001"];
        assert!(
            values.value_at(first, 101).unwrap().is_none(),
            "only the ambiguous counter is unknown"
        );
        assert_eq!(values.value_at(second, 101).unwrap().unwrap().value, 50);
        assert_eq!(
            values
                .value_at(prepared.tags["renamed counter holder"], 109)
                .unwrap()
                .unwrap()
                .value,
            0
        );
        assert!(
            values.value_at(second, 105).unwrap().unwrap().break_before,
            "replacement native identity starts a segment even at an unchanged value"
        );
    }

    #[test]
    fn territory_prepared_group_sum_is_exact_in_a_bounded_window_and_refuses_missing_members() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        for hour in 100..=104 {
            let mut frame = super::super::tests::frame(hour, 3, 8);
            if hour == 103 {
                *frame.tables[TableId::States.index()] =
                    replay_frame::frame::TableView::Unavailable {
                        reason: "synthetic missing state read".into(),
                    };
            }
            prepared
                .observe_with(&frame, Default::default(), &cancelled, None)
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &cancelled).unwrap();
        writer.commit().unwrap();
        let selected = vec!["realm 00000".to_owned(), "realm 00001".to_owned()];
        let within = BTreeMap::from([(
            selected[1].clone(),
            vec![HourSpan {
                first_hour: 100,
                last_hour: 101,
            }],
        )]);
        let summary = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &within,
            &[],
            None,
            (100.0, 104.0, 5),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert_eq!(
            summary
                .buckets
                .iter()
                .map(|bucket| bucket.map(|bucket| bucket.first_value))
                .collect::<Vec<_>>(),
            vec![Some(2.0), Some(2.0), Some(1.0), None, Some(1.0)]
        );
        assert!(summary.runs.unwrap().last().unwrap().break_before);
        let single = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &within,
            &[],
            None,
            (102.0, 102.0, 1),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert_eq!(single.runs.unwrap()[0].value, 1.0);
        let dense = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &BTreeMap::new(),
            &[],
            None,
            (100.0, 104.0, 1),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert!(dense.runs.is_none());
        assert!(dense.buckets[0].unwrap().has_break);
    }

    #[test]
    fn territory_prepared_facts_and_ranks_match_after_commits_resume_append_and_gaps() {
        let mut fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
        let mut reference = Scan::default();
        let mut builder = Builder::default();
        for (commit, hours) in [vec![100, 101], vec![102, 104], vec![105, 106]]
            .into_iter()
            .enumerate()
        {
            for hour in hours {
                let mut frame = super::super::tests::frame(hour, 3, 8);
                if hour >= 104
                    && let replay_frame::frame::TableView::Complete(
                        replay_data::rows::TableRows::Countries(rows),
                    ) = &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].id += 10;
                    rows[0].tag = b"renamed realm".to_vec();
                }
                let changes = tracker.observe(&frame, Default::default());
                observe(&mut reference, &frame, None, &mut builder, None).unwrap();
                prepared
                    .observe_with(&frame, Default::default(), &cancelled, Some(&changes))
                    .unwrap();
            }
            let mut writer = if commit == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &cancelled).unwrap();
            let saved = serde_json::to_vec(&prepared).unwrap();
            writer
                .write_at("territory-continuation", 0, saved.as_slice())
                .unwrap();
            writer.commit().unwrap();
            prepared = serde_json::from_reader(std::io::BufReader::new(
                fixture
                    .store
                    .read(&fixture.key, "territory-continuation", 0)
                    .unwrap()
                    .unwrap(),
            ))
            .unwrap();
            let (tags, _) = series::read_catalogue::<TerritoryReply>(
                &fixture.store,
                &fixture.key,
                "territory",
                1,
            )
            .unwrap()
            .unwrap();
            let mut facts = Reader::new(
                &fixture.store,
                &fixture.key,
                "territory-facts-1".into(),
                &cancelled,
            )
            .unwrap();
            for (tag, key) in &tags {
                let actual = runs(&mut facts, *key, 100, 106).unwrap();
                assert_eq!(
                    actual,
                    reference.series[tag.as_str()].runs,
                    "{tag}, commit {commit}"
                );
            }
            let reference_runs: BTreeMap<_, _> = reference
                .series
                .iter()
                .map(|(tag, track)| (tag.to_string(), track.runs.clone()))
                .collect();
            let mut ranks = Reader::new(
                &fixture.store,
                &fixture.key,
                "territory-ranks-1-2".into(),
                &cancelled,
            )
            .unwrap();
            crate::ranking::rank_positions(
                &reference_runs,
                |run| (run.first_hour, run.last_hour),
                |run| run.values[2].map(i128::from),
                &cancelled,
                |rank, tag, from, to| {
                    for hour in from..=to {
                        assert_eq!(
                            ranks.value_at(rank as u64 * 2, hour)?.unwrap().value,
                            i128::from(tags[tag])
                        );
                    }
                    Ok(())
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn territory_prepared_dense_sum_zeroes_nonmembers_and_keeps_empty_groups_unknown() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        for hour in 100..140 {
            prepared
                .observe_with(
                    &super::super::tests::frame(hour, 3, 8),
                    Default::default(),
                    &cancelled,
                    None,
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &cancelled).unwrap();
        writer.commit().unwrap();
        let selected = prepared.tags.keys().cloned().collect::<Vec<_>>();
        let summary = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &BTreeMap::new(),
            &[],
            Some(1),
            (100.0, 139.0, 2),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert!(summary.buckets.iter().all(|bucket| {
            bucket.is_some_and(|bucket| bucket.first_value == 1.0 && bucket.last_value == 1.0)
        }));
        let within = BTreeMap::from([
            (
                selected[0].clone(),
                vec![HourSpan {
                    first_hour: 100,
                    last_hour: 119,
                }],
            ),
            (
                selected[1].clone(),
                vec![HourSpan {
                    first_hour: 120,
                    last_hour: 139,
                }],
            ),
            (selected[2].clone(), vec![]),
        ]);
        let summary = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &within,
            &[],
            None,
            (100.0, 139.0, 2),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert!(summary.buckets.iter().all(|bucket| {
            bucket.is_some_and(|bucket| bucket.min_value == 1.0 && bucket.max_value == 1.0)
        }));
        let empty = selected
            .iter()
            .map(|tag| (tag.clone(), Vec::new()))
            .collect();
        let summary = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &empty,
            &[],
            None,
            (100.0, 139.0, 2),
            &BTreeMap::new(),
            None,
            &cancelled,
        )
        .unwrap();
        assert!(summary.buckets.iter().all(Option::is_none));
        let excluded = BTreeMap::from([(
            selected[0].clone(),
            vec![HourSpan {
                first_hour: 100,
                last_hour: 139,
            }],
        )]);
        let summary = aggregate_summary(
            &fixture.store,
            &fixture.key,
            1,
            0,
            &prepared.tags,
            &selected,
            &BTreeMap::new(),
            &[],
            None,
            (100.0, 139.0, 2),
            &excluded,
            None,
            &cancelled,
        )
        .unwrap();
        assert!(
            summary
                .buckets
                .iter()
                .all(|bucket| bucket.is_some_and(|bucket| bucket.first_value == 2.0))
        );
    }

    #[test]
    fn territory_prepared_values_keep_every_one_of_five_thousand_countries() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        let mut identities = super::super::super::country_history::HistoryScan::new(false, &[]);
        for hour in 100..104 {
            let mut frame = super::super::tests::frame(hour, 5_000, 5_000);
            *frame.tables[TableId::CountryCounterInputs.index()] =
                replay_frame::frame::TableView::Complete(
                    replay_data::rows::TableRows::CountryCounterInputs(
                        (0..5_000)
                            .map(|slot| replay_data::rows::CountryCounterInputRow {
                                country: 100_000 + slot,
                                owned_state_count: slot as i32,
                                ..Default::default()
                            })
                            .collect(),
                    ),
                );
            prepared
                .observe_owned_states(&frame, &mut identities, None)
                .unwrap();
            prepared
                .observe_with(&frame, Default::default(), &cancelled, None)
                .unwrap();
            let mut writer = fixture.writer();
            prepared.flush(&mut writer, 1, &cancelled).unwrap();
            writer.commit().unwrap();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            identities = serde_json::from_slice(&serde_json::to_vec(&identities).unwrap()).unwrap();
        }
        let (tags, _) =
            series::read_catalogue::<TerritoryReply>(&fixture.store, &fixture.key, "territory", 1)
                .unwrap()
                .unwrap();
        assert_eq!(tags.len(), 5_000);
        let mut values = Reader::new(
            &fixture.store,
            &fixture.key,
            "territory-values-1-0".into(),
            &cancelled,
        )
        .unwrap();
        for &key in tags.values() {
            let run = values.value_at(key, 103).unwrap().unwrap();
            assert_eq!((run.first_hour, run.last_hour, run.value), (100, 103, 1));
        }
        let mut native = Reader::new(
            &fixture.store,
            &fixture.key,
            "territory-values-1-8".into(),
            &cancelled,
        )
        .unwrap();
        for slot in 0..5_000 {
            let key = tags[&format!("realm {slot:05}")];
            assert_eq!(
                native.value_at(key, 103).unwrap().unwrap().value,
                i128::from(slot)
            );
        }
        eprintln!(
            "territory continuation bytes at 5,000 countries: {}",
            serde_json::to_vec(&prepared).unwrap().len()
        );
    }
}
#[cfg(test)]
mod phase_b_tests {
    use super::*;
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};

    #[test]
    fn territory_country_operand_refresh_retains_link_map_until_links_change_or_resume() {
        use replay_data::{rows::CountryLinkRow, rows::TableRows};
        use replay_frame::frame::TableView;
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut frame = super::super::tests::frame(100, 3, 3);
        frame.metrics = Default::default();
        let mut prepared = Prepared::default();
        let mut oracle = Scan {
            format,
            ..Default::default()
        };
        let mut builder = Builder::default();
        let mut tracker = FrameChangeTracker::new();
        prepared
            .observe_with(
                &frame,
                format,
                &flag,
                Some(&tracker.observe(&frame, format)),
            )
            .unwrap();
        observe(&mut oracle, &frame, None, &mut builder, None).unwrap();
        let slots = Arc::clone(prepared.builder.lineage.as_ref().unwrap().reads.slots());
        for hour in 101..=102 {
            frame.hour = hour;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[1].tag = format!("recorded alias {hour}").into_bytes();
            }
            prepared
                .observe_with(
                    &frame,
                    format,
                    &flag,
                    Some(&tracker.observe(&frame, format)),
                )
                .unwrap();
            observe(&mut oracle, &frame, None, &mut builder, None).unwrap();
            let reads = &prepared.builder.lineage.as_ref().unwrap().reads;
            assert!(
                Arc::ptr_eq(&slots, reads.slots()),
                "country aliases retain the link allocation"
            );
            assert_eq!(
                reads.holders()[&format!("recorded alias {hour}")],
                Some((1, 100_001))
            );
        }
        prepared.builder.seal(&mut prepared.scan);
        observer_tests::assert_reply_parity(&prepared.scan, &oracle).unwrap();

        frame.hour = 103;
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        {
            rows.push(CountryLinkRow {
                id: 200_000,
                canonical_slot: 1,
            });
        }
        prepared
            .observe_with(
                &frame,
                format,
                &flag,
                Some(&tracker.observe(&frame, format)),
            )
            .unwrap();
        observe(&mut oracle, &frame, None, &mut builder, None).unwrap();
        let changed = Arc::clone(prepared.builder.lineage.as_ref().unwrap().reads.slots());
        assert!(!Arc::ptr_eq(&slots, &changed));
        assert_eq!(changed.get(&200_000), Some(&1));
        prepared.builder.seal(&mut prepared.scan);
        observer_tests::assert_reply_parity(&prepared.scan, &oracle).unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        tracker.invalidate();
        frame.hour = 104;
        prepared
            .observe_with(
                &frame,
                format,
                &flag,
                Some(&tracker.observe(&frame, format)),
            )
            .unwrap();
        observe(&mut oracle, &frame, None, &mut builder, None).unwrap();
        assert!(!Arc::ptr_eq(
            &changed,
            prepared.builder.lineage.as_ref().unwrap().reads.slots()
        ));
        prepared.builder.seal(&mut prepared.scan);
        observer_tests::assert_reply_parity(&prepared.scan, &oracle).unwrap();
    }

    #[test]
    fn territory_shared_driver_carries_unchanged_values_without_native_or_history_work() {
        let flag = AtomicBool::new(false);
        let format = replay_data::format::FormatVersion::CURRENT;
        let work = observer_tests::scaled_work(10, |hours| {
            let mut frame = super::super::tests::frame(100, 64, 64);
            frame.metrics = Default::default();
            let mut prepared = Prepared::default();
            let changes = FrameChangeTracker::new().observe(&frame, format);
            prepared.observe_with(&frame, format, &flag, Some(&changes))?;
            let configured = prepared.builder.work;
            let delta = observer_tests::unchanged_frames(
                configured,
                &frame,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_with(frame, format, &flag, Some(changes))?;
                    Ok(prepared.builder.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            assert_eq!(prepared.builder.aggregations, 1);
            prepared.builder.seal(&mut prepared.scan);
            let mut oracle = Scan {
                format,
                ..Default::default()
            };
            let mut builder = Builder::default();
            for hour in 100..=100 + hours as i32 {
                frame.hour = hour;
                observe(&mut oracle, &frame, None, &mut builder, None)?;
            }
            observer_tests::assert_reply_parity(&prepared.scan, &oracle)?;
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }

    #[test]
    fn territory_shared_changes_reopen_equal_values_and_roster_proofs_after_unknown_hours() {
        use replay_frame::frame::TableView;
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut fixture = crate::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        let mut baseline = super::super::tests::frame(100, 2, 2);
        baseline.metrics = Default::default();
        let mut oracle = Scan {
            format,
            ..Default::default()
        };
        let mut builder = Builder::default();
        for (commit, hours) in [vec![100, 101, 102], vec![103, 104], vec![105, 106]]
            .into_iter()
            .enumerate()
        {
            for hour in hours {
                let mut read = baseline.clone();
                read.hour = hour;
                if hour == 101 {
                    *read.tables[TableId::States.index()] = TableView::Unavailable {
                        reason: "states unavailable".into(),
                    };
                }
                if hour == 104 {
                    *read.tables[TableId::Countries.index()] = TableView::Unavailable {
                        reason: "countries unavailable".into(),
                    };
                }
                prepared
                    .observe_with(&read, format, &flag, Some(&tracker.observe(&read, format)))
                    .unwrap();
                observe(&mut oracle, &read, None, &mut builder, None).unwrap();
            }
            let mut writer = if commit == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let actor = prepared.tags["realm 00000"];
            let mut values = Reader::new(
                &fixture.store,
                &fixture.key,
                "territory-values-1-0".into(),
                &flag,
            )
            .unwrap();
            let last = prepared.last;
            for hour in 100..=last {
                assert_eq!(
                    values.value_at(actor, hour).unwrap().map(|run| run.value),
                    oracle.series["realm 00000"]
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                        .and_then(|run| run.values[0])
                        .map(i128::from),
                    "hour {hour}"
                );
            }
            let mut rosters = Reader::new(
                &fixture.store,
                &fixture.key,
                "territory-rosters-1".into(),
                &flag,
            )
            .unwrap();
            for hour in 100..=last {
                assert_eq!(
                    rosters.value_at(0, hour).unwrap().is_some(),
                    oracle
                        .rosters
                        .iter()
                        .any(|span| span.first_hour <= hour && span.last_hour >= hour),
                    "roster {hour}"
                );
            }
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            tracker.invalidate();
        }
    }

    #[test]
    fn territory_one_changed_province_is_bounded_by_current_rows_and_changed_keys() {
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let flag = AtomicBool::new(false);
        let format = replay_data::format::FormatVersion::CURRENT;
        let work = observer_tests::scaled_work(10, |history| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut frame = super::super::tests::frame(100, 2, 2);
            frame.metrics = Default::default();
            for index in 0..history {
                frame.hour = 100 + index as i32;
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].tag = format!("closed alias {index}").into_bytes();
                }
                prepared.observe_with(
                    &frame,
                    format,
                    &flag,
                    Some(&tracker.observe(&frame, format)),
                )?;
            }
            let before = prepared.builder.work;
            frame.hour += 1;
            if let TableView::Complete(TableRows::Provinces(rows)) =
                &mut *frame.tables[TableId::Provinces.index()]
            {
                rows[0].victory_points += 1;
            }
            prepared.observe_with(
                &frame,
                format,
                &flag,
                Some(&tracker.observe(&frame, format)),
            )?;
            let delta = observer_tests::Work {
                row_visits: prepared.builder.work.row_visits - before.row_visits,
                history_visits: prepared.builder.work.history_visits - before.history_visits,
                emitted: prepared.builder.work.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(delta.row_visits, 2);
            assert_eq!(delta.history_visits, 2);
            assert_eq!(delta.emitted, 2);
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }

    #[test]
    fn territory_shared_driver_carries_all_5000_countries_then_publishes_exactly() {
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let mut frame = super::super::tests::frame(100, 5_000, 5_000);
        frame.metrics = Default::default();
        let mut prepared = Prepared::default();
        prepared
            .observe_with(
                &frame,
                format,
                &flag,
                Some(&FrameChangeTracker::new().observe(&frame, format)),
            )
            .unwrap();
        let delta = observer_tests::unchanged_frames(
            prepared.builder.work,
            &frame,
            100,
            format,
            |frame, changes| {
                prepared.observe_with(frame, format, &flag, Some(changes))?;
                Ok(prepared.builder.work)
            },
        )
        .unwrap();
        assert_eq!(delta, observer_tests::Work::default());
        let fixture = crate::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let mut values = Reader::new(
            &fixture.store,
            &fixture.key,
            "territory-values-1-0".into(),
            &flag,
        )
        .unwrap();
        for actor in prepared.tags.values() {
            assert_eq!(values.value_at(*actor, 200).unwrap().unwrap().value, 1);
        }
    }

    /// Publishes every territory section and the saved continuation, as a commit does.
    fn commit_with_continuation(
        prepared: &mut Prepared,
        writer: &mut store::Writer,
        flag: &AtomicBool,
    ) {
        prepared.flush(writer, 1, flag).unwrap();
        writer
            .write_at(
                "territory-continuation",
                0,
                serde_json::to_vec(&*prepared).unwrap().as_slice(),
            )
            .unwrap();
    }

    fn restore_continuation(fixture: &crate::prepared::series::tests::Fixture) -> Prepared {
        serde_json::from_reader::<_, Prepared>(
            fixture
                .store
                .read(&fixture.key, "territory-continuation", 0)
                .unwrap()
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn restored_territory_adds_no_value_key_for_a_holder_unresolved_across_a_commit() {
        use replay_data::{rows::CountryRow, rows::TableRows};
        use replay_frame::frame::TableView;
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        // No link names this row's id: its tag is held, by an unknown country,
        // at every hour, so it never has a value.
        let frames: Vec<_> = (100..104)
            .map(|hour| {
                let mut frame = super::super::tests::frame(hour, 3, 8);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows.push(CountryRow {
                        id: 900_000,
                        canonical_slot: 7,
                        tag: b"unresolved realm".to_vec(),
                        ..Default::default()
                    });
                }
                frame
            })
            .collect();
        observer_tests::assert_restore_continues(
            format,
            &[&frames[..2], &frames[2..]],
            Prepared::default,
            |prepared: &mut Prepared, frame, changes, _| {
                prepared.observe(frame, format, &flag, changes).unwrap()
            },
            |prepared: &mut Prepared, writer, _| commit_with_continuation(prepared, writer, &flag),
            restore_continuation,
        );
    }

    #[test]
    fn restored_territory_closes_holders_at_an_unreadable_first_hour_after_a_commit() {
        use replay_frame::frame::TableView;
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        // Countries are unreadable at hour 102, the first hour after a commit.
        let frames: Vec<_> = (100..104)
            .map(|hour| {
                let mut frame = super::super::tests::frame(hour, 3, 8);
                if hour == 102 {
                    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
                        reason: "countries unavailable".into(),
                    };
                }
                frame
            })
            .collect();
        observer_tests::assert_restore_continues(
            format,
            &[&frames[..2], &frames[2..]],
            Prepared::default,
            |prepared: &mut Prepared, frame, changes, _| {
                prepared.observe(frame, format, &flag, changes).unwrap()
            },
            |prepared: &mut Prepared, writer, _| commit_with_continuation(prepared, writer, &flag),
            restore_continuation,
        );
    }

    #[test]
    fn restored_owned_states_close_at_an_unreadable_first_hour_after_a_commit() {
        use crate::products::country_history::HistoryScan;
        use replay_data::{rows::CountryCounterInputRow, rows::TableRows};
        use replay_frame::frame::TableView;
        let format = replay_data::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        // Every country holds a native owned-state count; Countries are
        // unreadable at hour 102, the first hour after a commit.
        let frames: Vec<_> = (100..104)
            .map(|hour| {
                let mut frame = super::super::tests::frame(hour, 3, 8);
                *frame.tables[TableId::CountryCounterInputs.index()] =
                    TableView::Complete(TableRows::CountryCounterInputs(
                        (0..3)
                            .map(|slot| CountryCounterInputRow {
                                country: 100_000 + slot,
                                owned_state_count: 5,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                if hour == 102 {
                    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
                        reason: "countries unavailable".into(),
                    };
                }
                frame
            })
            .collect();
        observer_tests::assert_restore_continues(
            format,
            &[&frames[..2], &frames[2..]],
            || (Prepared::default(), HistoryScan::new(false, &[])),
            |state: &mut (Prepared, HistoryScan), frame, changes, _| {
                let (prepared, identities) = state;
                prepared
                    .observe_owned_states(frame, identities, Some(changes))
                    .unwrap();
                prepared.observe(frame, format, &flag, changes).unwrap();
            },
            |state: &mut (Prepared, HistoryScan), writer, _| {
                let (prepared, identities) = state;
                identities.seal();
                writer
                    .write_at(
                        "country-identities-1",
                        0,
                        serde_json::to_vec(&*identities).unwrap().as_slice(),
                    )
                    .unwrap();
                commit_with_continuation(prepared, writer, &flag);
            },
            |fixture| {
                let identities = fixture
                    .store
                    .read(&fixture.key, "country-identities-1", 0)
                    .unwrap()
                    .unwrap();
                (
                    restore_continuation(fixture),
                    serde_json::from_reader::<_, HistoryScan>(identities).unwrap(),
                )
            },
        );
    }
}
