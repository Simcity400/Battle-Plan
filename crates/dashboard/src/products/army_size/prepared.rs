//! The army scan feeding shared exact series and prepared rank positions.
use super::*;
use crate::prepared::series::{self, Accumulator, Reader, Run};
use replay::frame::DecodedFrame;

fn scan_state() -> ScanState {
    ScanState::new(true)
}

pub(crate) const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionCensus,
    TableId::Divisions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
    TableId::LexicalTokens,
];

#[cfg(all(test, feature = "slow-tests"))]
fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<crate::prepared::query::progressive::Samples> {
    let (scan, state) = point_scan(frame);
    sample_rows(&scan, &state, frame.hour, requested)
}

fn point_scan(frame: &DecodedFrame) -> (Scan, ScanState) {
    let mut scan = Scan::default();
    let mut state = scan_state();
    let raw = super::super::parts_race_prepared::updates(
        frame,
        &[
            TableId::Divisions,
            TableId::DivisionTemplateIcons,
            TableId::DivisionLandInputs,
        ],
    );
    state.apply_changes(&raw, frame);
    state.compose(frame);
    observe_totals(
        &mut scan,
        frame,
        &raw,
        replay::format::FormatVersion::CURRENT,
        false,
        &mut state,
        None,
    );
    (scan, state)
}

fn sample_rows(
    scan: &Scan,
    state: &ScanState,
    hour: i32,
    requested: Option<&[(String, String)]>,
) -> io::Result<crate::prepared::query::progressive::Samples> {
    if requested.is_some_and(|keys| {
        keys.iter()
            .any(|(_, field)| field.parse::<usize>().ok().is_none_or(|index| index >= 8))
    }) {
        return Err(crate::invalid("Unknown army sample channel"));
    }
    let wanted = requested.map(|keys| {
        keys.iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    });
    let mut samples = BTreeMap::new();
    for (tag, identity) in &state.identities {
        let values = scan
            .tracks
            .get(tag)
            .and_then(|track| track.runs.last())
            .filter(|run| run.last == hour);
        for index in 0..8 {
            let key = (tag.clone(), index.to_string());
            if wanted.as_ref().is_none_or(|wanted| wanted.contains(&key)) {
                samples.insert(
                    key,
                    (
                        Some(*identity),
                        values.and_then(|run| run.totals.values[index]),
                    ),
                );
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested {
            samples.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(samples)
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Prepared {
    scan: Scan,
    tags: BTreeMap<String, u64>,
    values: [Accumulator; 8],
    ranks: [Accumulator; 8],
    parts: Accumulator,
    coverage: Accumulator,
    mix_ids: BTreeMap<String, i128>,
    first: Option<i32>,
    last: i32,
    #[serde(skip, default = "scan_state")]
    state: ScanState,
}
impl Default for Prepared {
    fn default() -> Self {
        Self {
            scan: Scan::default(),
            tags: BTreeMap::new(),
            values: Default::default(),
            ranks: Default::default(),
            parts: Accumulator::default(),
            coverage: Accumulator::default(),
            mix_ids: BTreeMap::new(),
            first: None,
            last: 0,
            state: scan_state(),
        }
    }
}
impl Prepared {
    pub(crate) fn observe(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.first.get_or_insert(decoded.hour);
        self.last = decoded.hour;
        for values in &mut self.values {
            values.advance(decoded.hour)?;
        }
        self.coverage.advance(decoded.hour)?;
        self.parts.advance(decoded.hour)?;
        let frame = super::super::parts_race_prepared::updates_changed(
            decoded,
            &[
                TableId::Divisions,
                TableId::DivisionTemplateIcons,
                TableId::DivisionLandInputs,
            ],
            Some(changes),
            self.state.applied,
        );
        if !self
            .state
            .step_changes(&mut self.scan, &frame, decoded, replay, Some(changes))
        {
            return Ok(());
        }
        let observed = self
            .scan
            .observed
            .iter()
            .chain(&self.scan.removed)
            .cloned()
            .collect::<Vec<_>>();
        for tag in observed {
            let track = &self.scan.tracks[&tag];
            #[cfg(test)]
            {
                self.state.work.emitted += 10;
            }
            let next = self.tags.len() as u64;
            let key = *self.tags.entry(tag.clone()).or_insert(next);
            let run = track.runs.last().filter(|run| run.last == decoded.hour);
            self.coverage.observe(
                key,
                decoded.hour,
                run.map(|_| 1),
                run.is_some_and(|run| run.starts && run.first == decoded.hour),
            );
            for (index, values) in self.values.iter_mut().enumerate() {
                values.observe(
                    key,
                    decoded.hour,
                    run.and_then(|run| run.totals.values[index]),
                    run.is_some_and(|run| run.starts && run.first == decoded.hour),
                );
            }
            if let Some(mix) = track.mix.last().filter(|mix| mix.last == decoded.hour) {
                let version = self.mix_ids.entry(tag.clone()).or_default();
                if mix.first == decoded.hour {
                    *version += 1;
                    self.parts.event(key, decoded.hour, &mix.parts)?;
                }
                self.parts.observe(
                    key,
                    decoded.hour,
                    Some(*version),
                    mix.first == decoded.hour && run.is_some_and(|run| run.starts),
                );
            } else {
                self.parts.observe(key, decoded.hour, None, false);
            }
        }
        Ok(())
    }
    pub(crate) fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .map(Accumulator::bytes)
            .sum::<usize>()
            + self.parts.bytes()
            + self.coverage.bytes()
    }
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        self.scan.seal();
        let runs: BTreeMap<_, _> = self
            .scan
            .tracks
            .iter()
            .map(|(tag, track)| {
                (
                    tag.clone(),
                    track
                        .runs
                        .iter()
                        .filter(|run| run.last >= first)
                        .map(|run| RawRun {
                            first: run.first.max(first),
                            ..run.clone()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for index in 0..8 {
            self.ranks[index].through(first);
            crate::ranking::rank_positions(
                &runs,
                |run| (run.first, run.last),
                |run| run.totals.values[index],
                cancelled,
                |rank, tag, from, to| {
                    self.ranks[index].push(
                        rank as u64 * 2,
                        Run::new(from, to, i128::from(self.tags[tag]), false),
                    );
                    self.ranks[index].push(
                        self.tags[tag] * 2 + 1,
                        Run::new(from, to, rank as i128 + 1, false),
                    );
                    Ok(())
                },
            )?;
            self.ranks[index].through(self.last);
            self.ranks[index].flush(
                writer,
                &format!("army-ranks-{branch}-{index}"),
                0,
                cancelled,
            )?;
            self.values[index].flush(
                writer,
                &format!("army-values-{branch}-{index}"),
                if matches!(index, 1 | 3) { 5 } else { 0 },
                cancelled,
            )?;
        }
        self.parts
            .flush(writer, &format!("army-parts-{branch}"), 0, cancelled)?;
        self.coverage
            .flush(writer, &format!("army-coverage-{branch}"), 0, cancelled)?;
        for track in self.scan.tracks.values_mut() {
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
            if track.mix.len() > 1 {
                track.mix.drain(..track.mix.len() - 1);
            }
            if track.segments.len() > 1 {
                track.segments.drain(..track.segments.len() - 1);
            }
        }
        series::write_catalogue(writer, "army", branch, &self.tags, &self.scan)
    }
}

pub(crate) fn query(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<ArmySizeReply>> {
    ArmySize::validate(params).map_err(crate::invalid)?;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid branch"))?;
    let coverage =
        crate::prepared::pipeline::coverage(source, context.content.as_ref(), branch, cancelled)?;
    if !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        if let Some(window) = params.window {
            let range = source
                .block_windows(branch)
                .get(window)
                .copied()
                .ok_or_else(|| crate::invalid("Unknown army history window"))?;
            return progressive_window(
                source,
                params,
                context,
                coverage.as_ref(),
                range,
                cancelled,
            )
            .map(Some);
        }
        let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
        let mut scan = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
            .map(|row| row.1)
            .unwrap_or_default();
        let block = source
            .replay_ref()
            .index
            .iter()
            .rev()
            .find(|block| block.branch == branch)
            .ok_or_else(|| crate::invalid("Unknown army branch"))?;
        if let replay::container::SeekResult::Frame(frame) =
            source
                .replay_ref()
                .seek_selected(branch, block.first_hour, Some(TABLES))?
        {
            let (point, _) = point_scan(&frame);
            scan.divisions |= point.divisions;
            scan.strength |= point.strength;
            scan.manpower |= point.manpower;
            scan.battalions |= point.battalions;
        }
        scan.tracks.clear();
        return build_reply(
            source,
            params,
            context,
            cancelled,
            &scan,
            Some(Arc::new(Default::default())),
            None,
        )
        .map(Some);
    }
    prepared_query(source, params, context, cancelled)
}

fn prepared_query(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<ArmySizeReply>> {
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid branch"))?;
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let coverage =
        crate::prepared::pipeline::coverage(source, context.content.as_ref(), branch, cancelled)?;
    let history = Arc::new(crate::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        coverage.as_ref(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        source.replay_ref(),
        &params.tags,
        cancelled,
    )?);
    let Some((tags, mut scan)) = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
    else {
        return Ok(None);
    };
    scan.tracks.clear();
    let index = channel(params.measure, params.count_for);
    let mut membership = None;
    if let Some(window) = params.window {
        let block = source
            .block_windows(branch)
            .get(window)
            .ok_or_else(|| crate::invalid("Unknown army history window"))?;
        let range = HourSpan {
            first_hour: block.first_hour,
            last_hour: block.last_hour,
        };
        let wanted = if let Some(top) = params.top {
            let inverse: HashMap<_, _> = tags.iter().map(|(tag, key)| (*key, tag)).collect();
            let mut ranks = Reader::new(
                &store,
                &key,
                format!("army-ranks-{branch}-{index}"),
                cancelled,
            )?;
            let mut members: BTreeMap<String, Vec<HourSpan>> = BTreeMap::new();
            for rank in 0..top.min(tags.len()) {
                for run in ranks.runs(rank as u64 * 2, range.first_hour, range.last_hour)? {
                    members
                        .entry(inverse[&(run.value as u64)].clone())
                        .or_default()
                        .push(HourSpan {
                            first_hour: run.first_hour,
                            last_hour: run.last_hour,
                        });
                }
            }
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
            let wanted = members.keys().cloned().collect();
            membership = Some(members);
            wanted
        } else {
            super::super::country_history::selected_aliases(&history, &params.tags)
        };
        let mut values = Reader::new(
            &store,
            &key,
            format!("army-values-{branch}-{index}"),
            cancelled,
        )?;
        let mut parts = Reader::new(&store, &key, format!("army-parts-{branch}"), cancelled)?;
        for tag in wanted {
            let Some(&actor) = tags.get(&tag) else {
                continue;
            };
            let mut track = Track {
                runs: values
                    .runs(actor, range.first_hour, range.last_hour)?
                    .into_iter()
                    .map(|run| {
                        let mut totals = Totals::default();
                        totals.values[index] = Some(run.value);
                        RawRun {
                            first: run.first_hour,
                            last: run.last_hour,
                            starts: run.break_before,
                            totals,
                        }
                    })
                    .collect(),
                segments: history
                    .countries
                    .get(&tag)
                    .map_or_else(Vec::new, |country| {
                        country
                            .segments
                            .iter()
                            .filter(|span| {
                                span.first_hour <= range.last_hour
                                    && span.last_hour >= range.first_hour
                            })
                            .cloned()
                            .collect()
                    }),
                ..Default::default()
            };
            if params.by_type && params.measure == ArmyMeasure::Battalions {
                track.mix = prepared_mix(&mut parts, actor, range)?;
            }
            scan.tracks.insert(tag, track);
        }
    }
    build_reply(
        source,
        params,
        context,
        cancelled,
        &scan,
        Some(history),
        membership,
    )
    .map(Some)
}

fn prepared_mix(parts: &mut Reader<'_>, actor: u64, range: HourSpan) -> io::Result<Vec<RawMixRun>> {
    parts
        .runs(actor, range.first_hour, range.last_hour)?
        .into_iter()
        .map(|run| {
            let (_, row) = parts
                .latest_event(actor, run.first_hour)?
                .ok_or_else(|| crate::invalid("Missing prepared army composition"))?;
            Ok(RawMixRun {
                first: run.first_hour,
                last: run.last_hour,
                parts: serde_json::from_value(row)?,
            })
        })
        .collect()
}

/// One native point extraction supplies values and the availability metadata for the window.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sampled_window(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    coverage: Option<&crate::prepared::pipeline::Coverage>,
    range: (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    super::super::country_history::RecordedHistories,
    Scan,
)> {
    use crate::prepared::query::progressive;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid army branch"))?;
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let index = channel(params.measure, params.count_for).to_string();
    let requested = params
        .tags
        .iter()
        .map(|tag| (tag.clone(), index.clone()))
        .collect::<Vec<_>>();
    let metadata = std::cell::RefCell::new(Scan::default());
    let sampler = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
        let (scan, state) = point_scan(frame);
        let mut values = sample_rows(&scan, &state, frame.hour, None)?;
        values.retain(|(_, field), _| field == &index);
        if let Some(requested) = requested {
            for key in requested {
                values.entry(key.clone()).or_insert((None, None));
            }
        }
        let mut metadata = metadata.borrow_mut();
        metadata.divisions |= scan.divisions;
        metadata.strength |= scan.strength;
        metadata.manpower |= scan.manpower;
        metadata.battalions |= scan.battalions;
        metadata.inventory_fallback |= scan.inventory_fallback;
        if frame.hour >= range.0
            && frame.hour <= range.1
            && coverage.is_none_or(|coverage| frame.hour > coverage.last_hour)
        {
            metadata.unreadable += scan.unreadable;
            metadata.composition_unreadable += scan.composition_unreadable;
            for (reason, count) in scan.invalid_table_reads {
                *metadata.invalid_table_reads.entry(reason).or_default() += count;
            }
        }
        if params.by_type && params.measure == ArmyMeasure::Battalions {
            for (tag, track) in scan.tracks {
                metadata
                    .tracks
                    .entry(tag)
                    .or_default()
                    .mix
                    .extend(track.mix);
            }
        }
        Ok(values)
    };
    let tail = progressive::tail(
        source, branch, coverage, TABLES, &requested, range, &sampler, cancelled,
    )?;
    let history = progressive::histories(
        &store,
        &key,
        branch,
        coverage,
        &tail.2,
        &BTreeMap::new(),
        source.replay_ref(),
        &params.tags,
        cancelled,
    )?;
    Ok((tail.0, tail.1, history, metadata.into_inner()))
}

fn progressive_window(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    coverage: Option<&crate::prepared::pipeline::Coverage>,
    range: HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<ArmySizeReply> {
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid army branch"))?;
    let (tail, _, history, observed) = sampled_window(
        source,
        params,
        context,
        coverage,
        (range.first_hour, range.last_hour),
        cancelled,
    )?;
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let (actors, mut scan) =
        series::read_catalogue::<Scan>(&store, &key, "army", branch)?.unwrap_or_default();
    scan.tracks.clear();
    scan.divisions |= observed.divisions;
    scan.strength |= observed.strength;
    scan.manpower |= observed.manpower;
    scan.battalions |= observed.battalions;
    scan.inventory_fallback |= observed.inventory_fallback;
    scan.unreadable += observed.unreadable;
    scan.composition_unreadable += observed.composition_unreadable;
    for (reason, count) in &observed.invalid_table_reads {
        *scan.invalid_table_reads.entry(reason.clone()).or_default() += count;
    }
    let index = channel(params.measure, params.count_for);
    let mut values = Reader::new(
        &store,
        &key,
        format!("army-values-{branch}-{index}"),
        cancelled,
    )?;
    let mut parts = Reader::new(&store, &key, format!("army-parts-{branch}"), cancelled)?;
    let mut wanted = if params.top.is_some() {
        actors.keys().cloned().collect()
    } else {
        super::super::country_history::selected_aliases(&history, &params.tags)
    };
    if params.top.is_some() {
        wanted.extend(tail.keys().map(|(tag, _)| tag.clone()));
    }
    wanted.sort();
    wanted.dedup();
    for tag in wanted {
        let mut runs = values.runs(
            actors.get(&tag).copied().unwrap_or(u64::MAX),
            range.first_hour,
            range.last_hour,
        )?;
        runs.extend(
            tail.get(&(tag.clone(), index.to_string()))
                .into_iter()
                .flatten()
                .copied(),
        );
        let segments = history
            .countries
            .get(&tag)
            .map_or_else(Vec::new, |country| country.segments.as_ref().clone());
        let mix = if params.by_type && params.measure == ArmyMeasure::Battalions {
            let mut mix = prepared_mix(
                &mut parts,
                actors.get(&tag).copied().unwrap_or(u64::MAX),
                range,
            )?;
            if let Some(track) = observed.tracks.get(&tag) {
                mix.extend(track.mix.iter().cloned());
            }
            mix
        } else {
            Vec::new()
        };
        scan.tracks.insert(
            tag,
            Track {
                runs: runs
                    .into_iter()
                    .map(|run| {
                        let mut totals = Totals::default();
                        totals.values[index] = Some(run.value);
                        RawRun {
                            first: run.first_hour,
                            last: run.last_hour,
                            starts: run.break_before,
                            totals,
                        }
                    })
                    .collect(),
                segments,
                mix,
                ..Default::default()
            },
        );
    }
    // The native builder's ranking sweep sees only this bounded block window.
    build_reply(
        source,
        params,
        context,
        cancelled,
        &scan,
        Some(Arc::new(history)),
        None,
    )
}

pub(crate) fn summary(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use crate::prepared::query::progressive;
    use serde_json::json;
    let selection: ArmySizeParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"], "top": params["family"]["top"],
        "measure": params["family"]["measure"], "countFor": params["family"]["countFor"], "byType": false,
    }))?;
    ArmySize::validate(&selection).map_err(crate::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid army branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| crate::invalid("Unknown army branch"))?;
    let bounds = crate::metadata::branch_bounds(source.replay_ref(), start);
    let (from, to, width) = series::viewport(
        params["from"].as_f64(),
        params["to"].as_f64(),
        params["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let coverage =
        crate::prepared::pipeline::coverage(source, context.content.as_ref(), branch, cancelled)?;
    let (tail, approximate, history, _) = sampled_window(
        source,
        &selection,
        context,
        coverage.as_ref(),
        (from.floor() as i32, to.floor() as i32),
        cancelled,
    )?;
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let actors = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
        .map(|row| row.0)
        .unwrap_or_default();
    let index = channel(selection.measure, selection.count_for);
    let sampled: BTreeMap<_, _> = tail
        .iter()
        .map(|((tag, _), runs)| (tag.clone(), runs.clone()))
        .collect();
    let mut membership = selection
        .top
        .map(|top| {
            crate::ranking::top_membership(
                &sampled,
                top,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
            )
        })
        .transpose()?;
    let mut ranks = Reader::new(
        &store,
        &key,
        format!("army-ranks-{branch}-{index}"),
        cancelled,
    )?;
    let dense = to.floor() - from.floor() + 1.0 > f64::from(width);
    // Exact screens can read bounded rank runs; dense screens keep width summaries.
    if !dense && let (Some(top), Some(members)) = (selection.top, &mut membership) {
        let inverse: HashMap<_, _> = actors.iter().map(|(tag, key)| (*key, tag)).collect();
        for position in 0..top.min(actors.len()) {
            for run in ranks.runs(position as u64 * 2, from.floor() as i32, to.floor() as i32)? {
                if let Some(tag) = inverse.get(&(run.value as u64)) {
                    members.entry((*tag).clone()).or_default().push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
                }
            }
        }
        for spans in members.values_mut() {
            spans.sort_by_key(|span| span.first_hour);
            let mut joined = Vec::new();
            for span in std::mem::take(spans) {
                super::super::country_groups::mark_unobserved(
                    &mut joined,
                    span.first_hour,
                    span.last_hour,
                );
            }
            *spans = joined;
        }
    }
    let mut wanted = if selection.top.is_some() {
        actors.keys().cloned().collect()
    } else {
        super::super::country_history::selected_aliases(&history, &selection.tags)
    };
    if selection.top.is_some() {
        wanted.extend(sampled.keys().cloned());
    }
    wanted.sort();
    wanted.dedup();
    let selected = super::super::country_history::selected_identities(&history, &selection.tags);
    let mut values = Reader::new(
        &store,
        &key,
        format!("army-values-{branch}-{index}"),
        cancelled,
    )?;
    let mut lines = Vec::new();
    for tag in wanted {
        let actor = actors.get(&tag).copied().unwrap_or(u64::MAX);
        let identity_spans = history
            .countries
            .get(&tag)
            .into_iter()
            .flat_map(|country| country.identities.iter())
            .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
            .map(|identity| HourSpan {
                first_hour: identity.first_hour,
                last_hour: identity.last_hour,
            })
            .collect::<Vec<_>>();
        let within = if selection.top.is_none() {
            Some(identity_spans.as_slice())
        } else if !dense {
            Some(
                membership
                    .as_ref()
                    .and_then(|members| members.get(&tag))
                    .map_or(&[][..], Vec::as_slice),
            )
        } else {
            None
        };
        let runs = sampled.get(&tag).map_or(&[][..], Vec::as_slice);
        let mut summary = values.summary_with_tail(
            actor,
            bounds,
            (from, to, width),
            if selection.measure == ArmyMeasure::Strength {
                5
            } else {
                0
            },
            runs,
            within,
        )?;
        if dense && let Some(top) = selection.top {
            let rank_key = actors.get(&tag).map_or(u64::MAX, |actor| actor * 2 + 1);
            let rank = ranks.summary(rank_key, bounds, (from, to, width), 0)?;
            let tail_members = membership.as_ref().and_then(|members| members.get(&tag));
            for (bucket, rank) in summary.buckets.iter_mut().zip(rank.buckets) {
                if bucket.is_some_and(|bucket| {
                    rank.is_none_or(|rank| rank.min_value > top as f64)
                        && !tail_members.is_some_and(|spans| {
                            spans.iter().any(|span| {
                                span.first_hour <= bucket.last_hour
                                    && span.last_hour >= bucket.first_hour
                            })
                        })
                }) {
                    *bucket = None;
                }
            }
            summary.runs = None;
        }
        if selection.top.is_some() && summary.buckets.iter().all(Option::is_none) {
            continue;
        }
        progressive::clip_approximate(
            &mut summary,
            &approximate,
            if selection.top.is_some() {
                membership
                    .as_ref()
                    .and_then(|members| members.get(&tag))
                    .map(Vec::as_slice)
            } else {
                within
            },
        );
        series::summary_budget(from, to, width, lines.len() + 1)?;
        lines.push(json!({"tag": tag, "summary": summary, "identities": history.countries.get(&tag).map(|country| &country.identities)}));
    }
    Ok(json!({"series": lines}))
}

pub(crate) fn next_change(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use serde_json::{Value, json};
    let selection: ArmySizeParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"],
        "top": params["family"]["top"], "measure": params["family"]["measure"],
        "countFor": params["family"]["countFor"], "byType": false,
    }))?;
    ArmySize::validate(&selection).map_err(crate::invalid)?;
    let navigation: series::Navigation = serde_json::from_value(params.clone())?;
    let origin = navigation.origin()?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid branch"))?;
    let accepted =
        crate::prepared::pipeline::coverage(source, context.content.as_ref(), branch, cancelled)?;
    if !accepted.as_ref().is_some_and(|coverage| coverage.complete)
        && accepted
            .as_ref()
            .is_none_or(|coverage| navigation.last_hour > f64::from(coverage.last_hour))
    {
        use crate::prepared::query::progressive;
        let index = channel(selection.measure, selection.count_for);
        let (mut tail, _, history, _) = sampled_window(
            source,
            &selection,
            context,
            accepted.as_ref(),
            (
                navigation.first_hour.floor() as i32,
                navigation.last_hour.floor() as i32,
            ),
            cancelled,
        )?;
        let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
        let mut actors = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
            .map(|row| row.0)
            .unwrap_or_default();
        let mut within: BTreeMap<String, Vec<HourSpan>> =
            serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
        let wanted = if selection.top.is_some() {
            history.countries.keys().cloned().collect()
        } else {
            super::super::country_history::selected_aliases(&history, &selection.tags)
        };
        let mut requested = wanted
            .into_iter()
            .map(|tag| (tag, index.to_string()))
            .collect::<Vec<_>>();
        let mut values = Reader::new(
            &store,
            &key,
            format!("army-values-{branch}-{index}"),
            cancelled,
        )?;
        let mut prepared = BTreeMap::new();
        if selection.top.is_none() {
            let selected =
                super::super::country_history::selected_identities(&history, &selection.tags);
            for key in &requested {
                let native = history
                    .countries
                    .get(&key.0)
                    .into_iter()
                    .flat_map(|country| country.identities.iter())
                    .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                    .map(|identity| HourSpan {
                        first_hour: identity.first_hour,
                        last_hour: identity.last_hour,
                    })
                    .collect::<Vec<_>>();
                let scopes = if let Some(spans) = within.get(&key.0) {
                    let mut scopes = Vec::new();
                    crate::ranking::for_each_piece(
                        &native,
                        spans,
                        |span| (span.first_hour, span.last_hour),
                        cancelled,
                        |_, first_hour, last_hour| {
                            scopes.push(HourSpan {
                                first_hour,
                                last_hour,
                            });
                            Ok(())
                        },
                    )?;
                    scopes
                } else {
                    native
                };
                within.insert(key.0.clone(), scopes);
            }
        }
        if let Some(top) = selection.top {
            let sampled: BTreeMap<_, _> = tail
                .iter()
                .map(|((tag, _), runs)| (tag.clone(), runs.clone()))
                .collect();
            let mut next = actors.len() as u64;
            for tag in sampled.keys() {
                actors.entry(tag.clone()).or_insert_with(|| {
                    let key = next;
                    next += 1;
                    key
                });
            }
            within = crate::ranking::top_membership(
                &sampled,
                top,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
            )?;
            let mut ranks = Reader::new(
                &store,
                &key,
                format!("army-ranks-{branch}-{index}"),
                cancelled,
            )?;
            crate::ranking::rank_positions(
                &sampled,
                |run| (run.first_hour, run.last_hour),
                |run| Some(run.value),
                cancelled,
                |position, tag, first, last| {
                    if position < top {
                        tail.entry((String::new(), format!("rank-{position}")))
                            .or_default()
                            .push(Run::new(first, last, i128::from(actors[tag]), false));
                    }
                    Ok(())
                },
            )?;
            for position in 0..top.min(actors.len()) {
                let rank = (String::new(), format!("rank-{position}"));
                let actor = position as u64 * 2;
                let before = accepted
                    .as_ref()
                    .map(|coverage| ranks.value_at(actor, coverage.last_hour))
                    .transpose()?
                    .flatten();
                let hour = series::navigate(&mut [(&mut ranks, vec![(actor, None)])], &navigation)?;
                prepared.insert(rank.clone(), (hour, before));
                requested.push(rank);
                // Prepared membership stays bounded by the existing rank summary.
                if let Some(run) = ranks.value_at(
                    actor,
                    origin
                        .clamp(navigation.first_hour, navigation.last_hour)
                        .floor() as i32,
                )? && let Some((tag, _)) =
                    actors.iter().find(|(_, key)| **key == run.value as u64)
                {
                    within.entry(tag.clone()).or_default().push(HourSpan {
                        first_hour: navigation.first_hour.floor() as i32,
                        last_hour: accepted
                            .as_ref()
                            .map_or(i32::MIN, |coverage| coverage.last_hour),
                    });
                }
            }
            for key in &requested {
                if !key.0.is_empty() {
                    within.entry(key.0.clone()).or_default();
                }
            }
            for spans in within.values_mut() {
                spans.sort_by_key(|span| span.first_hour);
            }
        }
        for key in &requested {
            if key.0.is_empty() {
                continue;
            }
            let actor = actors.get(&key.0).copied().unwrap_or(u64::MAX);
            let before = accepted
                .as_ref()
                .map(|coverage| values.value_at(actor, coverage.last_hour))
                .transpose()?
                .flatten();
            let hour = series::navigate(
                &mut [(
                    &mut values,
                    vec![(actor, within.get(&key.0).map(Vec::as_slice))],
                )],
                &navigation,
            )?;
            prepared.insert(key.clone(), (hour, before));
        }
        return Ok(
            json!({"hour": progressive::navigate(&prepared, &tail, &requested, &within, &navigation)?}),
        );
    }
    let (store, key) = crate::prepared::files(source, context.content.as_ref())?;
    let history = crate::prepared::query::progressive::histories(
        &store,
        &key,
        branch,
        accepted.as_ref(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        source.replay_ref(),
        &selection.tags,
        cancelled,
    )?;
    let Some((actors, _scan)) = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
    else {
        return Ok(Value::Null);
    };
    let channel = channel(selection.measure, selection.count_for);
    let mut values = Reader::new(
        &store,
        &key,
        format!("army-values-{branch}-{channel}"),
        cancelled,
    )?;
    let mut ranks = Reader::new(
        &store,
        &key,
        format!("army-ranks-{branch}-{channel}"),
        cancelled,
    )?;
    let mut value_keys = Vec::new();
    let mut rank_keys = Vec::new();
    if let Some(top) = selection.top {
        let hour = origin
            .clamp(navigation.first_hour, navigation.last_hour)
            .floor() as i32;
        for position in 0..top.min(actors.len()) {
            let key = position as u64 * 2;
            rank_keys.push((key, None));
            if let Some(actor) = ranks.value_at(key, hour)? {
                value_keys.push((actor.value as u64, None));
            }
        }
    } else {
        value_keys.extend(
            super::super::country_history::selected_aliases(&history, &selection.tags)
                .iter()
                .filter_map(|tag| actors.get(tag))
                .map(|&key| (key, None)),
        );
    }
    let hour = series::navigate(
        &mut [(&mut values, value_keys), (&mut ranks, rank_keys)],
        &navigation,
    )?;
    Ok(json!({"hour": hour}))
}

#[cfg(test)]
mod progressive_tests;
