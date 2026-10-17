//! Native factory value and supplying-read runs, with prepared rank positions.
use super::*;
use crate::prepared::query::progressive;
use crate::prepared::series::{self, Accumulator, Reader, Run};
use crate::products::country_history;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
const MODES: [FactoryCount; 3] = [
    FactoryCount::Free,
    FactoryCount::Built,
    FactoryCount::TopBar,
];

/// Complete numeric tables used by the stateless factory parts sample.
pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFactories,
];

#[allow(clippy::type_complexity)]
pub fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    let count = fields().len();
    if requested.is_some_and(|keys| {
        keys.iter().any(|(_, field)| {
            field
                .parse::<usize>()
                .ok()
                .is_none_or(|ordinal| ordinal >= count || ordinal.to_string() != *field)
        })
    }) {
        return Err(crate::invalid("Unknown factory sample field"));
    }
    let wanted = requested.map(|keys| keys.iter().collect::<BTreeSet<_>>());
    let mut result = BTreeMap::new();
    if let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        let rows = unique_by_slot(
            frame
                .complete::<tables::CountryFactories>()
                .ok()
                .into_iter()
                .flatten(),
            links,
            |row| row.country,
        );
        for (tag, lookup) in tagged_countries(countries, links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let identity = lookup.found().map(|(country, slot)| (slot, country.id));
            let amounts = identity
                .and_then(|(slot, _)| rows.rows.get(&slot).copied().flatten())
                .and_then(values)
                .map(|(amounts, _)| amounts);
            for ordinal in 0..count {
                let key = (tag.to_owned(), ordinal.to_string());
                if wanted.as_ref().is_none_or(|keys| keys.contains(&key)) {
                    result.insert(
                        key,
                        (
                            identity,
                            amounts.as_ref().map(|values| i128::from(values[ordinal])),
                        ),
                    );
                }
            }
        }
    }
    if let Some(requested) = requested {
        for key in requested {
            result.entry(key.clone()).or_insert((None, None));
        }
    }
    Ok(result)
}

/// One sampled country's native count over every factory group, as the
/// prepared all-group channel for `mode` holds it. It is unknown unless the
/// frame read every part for exactly this native holder.
pub fn sampled_count(
    samples: &progressive::Samples,
    tag: &str,
    holder: (u32, u32),
    mode: FactoryCount,
) -> io::Result<Option<i128>> {
    let mut values = Vec::with_capacity(fields().len());
    for ordinal in 0..fields().len() {
        let Some(&(identity, Some(value))) = samples.get(&(tag.to_owned(), ordinal.to_string()))
        else {
            return Ok(None);
        };
        if identity != Some(holder) {
            return Ok(None);
        }
        values.push(i32::try_from(value).map_err(io::Error::other)?);
    }
    Ok(Some(i128::from(count_for(
        &values,
        &FactoryTop {
            count: 1,
            mode,
            groups: Vec::new(),
        },
    ))))
}

fn rank_selection(index: usize) -> FactoryTop {
    FactoryTop {
        count: 1,
        mode: MODES[index / 7],
        groups: FactoryGroupKey::ALL
            .into_iter()
            .enumerate()
            .filter_map(|(bit, group)| ((index % 7 + 1) & (1 << bit) != 0).then_some(group))
            .collect(),
    }
}

fn sampled_runs(
    samples: &BTreeMap<(String, String), Vec<Run>>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<FactoryRun>>> {
    let mut countries: BTreeMap<&str, BTreeMap<usize, &[Run]>> = BTreeMap::new();
    for ((tag, field), runs) in samples {
        if let Ok(ordinal) = field.parse::<usize>() {
            countries.entry(tag).or_default().insert(ordinal, runs);
        }
    }
    let mut result = BTreeMap::new();
    for (tag, parts) in countries {
        crate::metadata::check_cancel(cancelled)?;
        let dates: BTreeSet<_> = parts
            .values()
            .flat_map(|runs| runs.iter())
            .flat_map(|run| [i64::from(run.first_hour), i64::from(run.last_hour) + 1])
            .collect();
        let dates: Vec<_> = dates.into_iter().collect();
        for pair in dates.windows(2) {
            let first = pair[0] as i32;
            let mut amounts = Vec::with_capacity(fields().len());
            let mut starts = false;
            for ordinal in 0..fields().len() {
                let Some(runs) = parts.get(&ordinal) else {
                    break;
                };
                let index = runs.partition_point(|run| run.last_hour < first);
                let Some(run) = runs.get(index).filter(|run| run.first_hour <= first) else {
                    break;
                };
                amounts.push(i32::try_from(run.value).map_err(io::Error::other)?);
                starts |= run.break_before && run.first_hour == first;
            }
            if amounts.len() == fields().len() {
                result
                    .entry(tag.to_owned())
                    .or_insert_with(Vec::new)
                    .push(FactoryRun {
                        first_hour: first,
                        last_hour: (pair[1] - 1) as i32,
                        starts_segment: starts,
                        values: amounts,
                    });
            }
        }
    }
    Ok(result)
}

pub fn channel(top: &FactoryTop) -> usize {
    let mask = if top.groups.is_empty() {
        7
    } else {
        FactoryGroupKey::ALL
            .iter()
            .enumerate()
            .filter_map(|(bit, group)| top.groups.contains(group).then_some(1 << bit))
            .sum::<usize>()
    };
    MODES.iter().position(|mode| *mode == top.mode).unwrap() * 7 + mask - 1
}
#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    keys: BTreeMap<String, u64>,
    values: [Accumulator; 21],
    ranks: [Accumulator; 21],
    parts: Accumulator,
    mixes: Accumulator,
    reads: Accumulator,
    first: Option<i32>,
    last: i32,
}
impl Prepared {
    fn key(&mut self, tag: &str, field: &str) -> u64 {
        let next = self.keys.len() as u64;
        *self
            .keys
            .entry(series::field_key(tag, field))
            .or_insert(next)
    }
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan
            .observe_changes(frame, replay.format, Some(changes));
        self.advance(frame.hour)?;
        for tag in std::mem::take(&mut self.scan.observed) {
            let track = &self.scan.tracks[&tag];
            let raw = track
                .runs
                .last()
                .filter(|run| run.last_hour == frame.hour)
                .cloned();
            let read = track
                .reads
                .last()
                .filter(|run| run.last_hour == frame.hour)
                .cloned();
            let key = self.key(&tag, "");
            #[cfg(any(test, feature = "test-support"))]
            {
                self.scan.work.emitted += 23 + fields().len();
            }
            let Some(raw) = raw else {
                for values in &mut self.values {
                    values.observe(key, frame.hour, None, false);
                }
                self.mixes.observe(key, frame.hour, None, false);
                self.reads.observe(key, frame.hour, None, false);
                for index in 0..fields().len() {
                    if let Some(&part) = self.keys.get(&series::field_key(&tag, &index.to_string()))
                    {
                        self.parts.observe(part, frame.hour, None, false);
                    }
                }
                continue;
            };
            self.scan.result.recorded.insert(tag.clone());
            let starts = raw.starts_segment && raw.first_hour == frame.hour;
            let changed = raw.first_hour == frame.hour || self.mixes.state(key).is_none();
            let version = self
                .mixes
                .state(key)
                .and_then(|state| state.last)
                .map_or(0, |run| run.value)
                + i128::from(changed);
            if changed {
                self.mixes.event(key, frame.hour, &raw)?;
            }
            self.mixes.observe(key, frame.hour, Some(version), starts);
            for index in 0..21 {
                self.values[index].observe(
                    key,
                    frame.hour,
                    Some(i128::from(count_for(&raw.values, &rank_selection(index)))),
                    starts,
                );
            }
            for (index, value) in raw.values.iter().enumerate() {
                let key = self.key(&tag, &index.to_string());
                self.parts
                    .observe(key, frame.hour, Some(i128::from(*value)), starts);
            }
            if let Some(read) = read {
                let changed = read.first_hour == frame.hour || self.reads.state(key).is_none();
                let version = self
                    .reads
                    .state(key)
                    .and_then(|state| state.last)
                    .map_or(0, |run| run.value)
                    + i128::from(changed);
                if changed {
                    self.reads.event(key, frame.hour, &read)?;
                }
                self.reads.observe(key, frame.hour, Some(version), starts);
            }
        }
        Ok(())
    }
    /// The accepted hour every series takes, observed or quiet.
    fn advance(&mut self, hour: i32) -> io::Result<()> {
        self.first.get_or_insert(hour);
        self.last = hour;
        for values in &mut self.values {
            values.advance(hour)?;
        }
        self.parts.advance(hour)?;
        self.mixes.advance(hour)?;
        self.reads.advance(hour)
    }
    /// What observing `hour` does when it follows the last hour observed and
    /// none of the countries' keys, links, availability or factory rows
    /// changed there, the factory table read (`read`) or not as at the hour
    /// before: the unchanged hour's counts and every series' accepted hour,
    /// in constant time. `Ok(false)`, with nothing changed, when observing it
    /// needs its frame.
    pub fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<bool> {
        if !self.scan.advance_quiet(hour, read) {
            return Ok(false);
        }
        self.advance(hour)?;
        Ok(true)
    }
    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .map(Accumulator::bytes)
            .sum::<usize>()
            + self.parts.bytes()
            + self.mixes.bytes()
            + self.reads.bytes()
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
                        .filter(|run| run.last_hour >= first)
                        .map(|run| FactoryRun {
                            first_hour: run.first_hour.max(first),
                            ..run.clone()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for index in 0..21 {
            let top = rank_selection(index);
            self.ranks[index].through(first);
            crate::ranking::rank_positions(
                &runs,
                |run| (run.first_hour, run.last_hour),
                |run| Some(i128::from(count_for(&run.values, &top))),
                cancelled,
                |position, tag, from, to| {
                    let key = self.keys[&series::field_key(tag, "")];
                    self.ranks[index].push(
                        position as u64 * 2,
                        Run::new(from, to, i128::from(key), false),
                    );
                    self.ranks[index]
                        .push(key * 2 + 1, Run::new(from, to, position as i128 + 1, false));
                    Ok(())
                },
            )?;
            self.ranks[index].through(self.last);
            self.ranks[index].flush(
                writer,
                &format!("factories-ranks-{branch}-{index}"),
                0,
                cancelled,
            )?;
            self.values[index].flush(
                writer,
                &format!("factories-values-{branch}-{index}"),
                0,
                cancelled,
            )?;
        }
        self.parts
            .flush(writer, &format!("factories-parts-{branch}"), 0, cancelled)?;
        self.mixes
            .flush(writer, &format!("factories-mixes-{branch}"), 0, cancelled)?;
        self.reads
            .flush(writer, &format!("factories-reads-{branch}"), 0, cancelled)?;
        for track in self.scan.tracks.values_mut() {
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
            if track.segments.len() > 1 {
                track.segments.drain(..track.segments.len() - 1);
            }
            if track.reads.len() > 1 {
                track.reads.drain(..track.reads.len() - 1);
            }
        }
        series::write_catalogue(writer, "factories", branch, &self.keys, &self.scan.result)
    }
}
pub fn query(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let reads: Vec<crate::prepared_measure::query::MeasureRead> = if product == "series-state" {
        serde_json::from_value(params["reads"].clone())?
    } else {
        Vec::new()
    };
    let read_tags: Vec<_> = reads.iter().map(|read| read.tag.clone()).collect();
    let selection: FactoriesParams = serde_json::from_value(
        json!({"branchId":params["branchId"], "tags":if product=="series-state" {json!(read_tags)} else {params.get("tags").cloned().unwrap_or_else(||json!([]))},
        "top":params["family"]["top"]}),
    )?;
    CountryFactories::validate(&selection).map_err(crate::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid factory branch"))?;
    let (store, source_key) = crate::prepared::files(source, context.content.as_ref())?;
    // Prepared on their own, the factories and the identity history they are
    // named through answer from their own coverage.
    let accepted = match crate::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        CountryFactories::KEY,
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        result => result?,
    };
    // Pages committed ahead of that coverage (by a family prepared on its own
    // ahead of the identity history) are not read: the prepared prefix ends
    // at the last batch committed by its last hour, and the hours after it
    // are an unprepared tail.
    let accepted = series::committed_coverage(
        &store,
        &source_key,
        format!("factories-values-{branch}-0"),
        accepted,
        cancelled,
    )?;
    let partial = accepted.as_ref().is_none_or(|coverage| !coverage.complete);
    if accepted.is_none() && matches!(product, "events-page" | "events-clusters") {
        return Ok(Value::Null);
    }
    let requested: Vec<_> = selection
        .tags
        .iter()
        .flat_map(|tag| (0..fields().len()).map(move |ordinal| (tag.clone(), ordinal.to_string())))
        .collect();
    let sample_all = |frame: &DecodedFrame, _: Option<&[(String, String)]>| sample(frame, None);
    let tail = if partial && product == "series-state" {
        progressive::state_tail_all_cached(
            source,
            "factories:series",
            branch,
            accepted.as_ref(),
            TABLES,
            &reads.iter().map(|read| read.hour).collect::<Vec<_>>(),
            &sample_all,
            cancelled,
        )?
    } else if partial && matches!(product, "series-summary" | "series-next-change") {
        let (from, to) = if product == "series-summary" {
            series::range(params["from"].as_f64(), params["to"].as_f64())?
        } else {
            series::range(params["firstHour"].as_f64(), params["lastHour"].as_f64())?
        };
        progressive::tail_all_cached(
            source,
            "factories:series",
            branch,
            accepted.as_ref(),
            TABLES,
            (from.floor() as i32, to.floor() as i32),
            &sample_all,
            cancelled,
        )?
    } else {
        Default::default()
    };
    let transient = sampled_runs(&tail.0, cancelled)?;
    let history = Arc::new(progressive::histories(
        &store,
        &source_key,
        branch,
        accepted.as_ref(),
        &tail.2,
        &BTreeMap::new(),
        source.replay_ref(),
        &requested
            .iter()
            .map(|(tag, _)| tag.clone())
            .filter(|tag| !tag.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>(),
        cancelled,
    )?);
    let (keys, mut scanned) = if accepted.is_some() {
        series::read_catalogue::<ScannedFactories>(&store, &source_key, "factories", branch)?
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let rank: FactoryTop = serde_json::from_value(if params["family"]["top"].is_null() {
        json!({"count":1,"mode":params["family"].get("mode").cloned().unwrap_or_else(||json!("free")),"groups":params["family"].get("groups").cloned().unwrap_or_else(||json!([]))})
    } else {
        params["family"]["top"].clone()
    })?;
    CountryFactories::validate(&FactoriesParams {
        branch_id: selection.branch_id.clone(),
        tags: Vec::new(),
        top: Some(rank.clone()),
    })
    .map_err(crate::invalid)?;
    let index = channel(&rank);
    let transient_values: BTreeMap<_, _> = transient
        .iter()
        .map(|(tag, runs)| {
            (
                (tag.clone(), String::new()),
                runs.iter()
                    .map(|run| {
                        Run::new(
                            run.first_hour,
                            run.last_hour,
                            i128::from(count_for(&run.values, &rank)),
                            run.starts_segment,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    let inverse: HashMap<_, _> = history
        .countries
        .keys()
        .filter_map(|tag| {
            keys.get(&series::field_key(tag, ""))
                .map(|&key| (key, tag.clone()))
        })
        .collect();
    let reader = |section: String| -> io::Result<Reader<'_>> {
        if let Some(coverage) = &accepted {
            let mut reader = Reader::new(&store, &source_key, section, cancelled)?;
            if !coverage.complete {
                reader.through_hour(coverage.last_hour)?;
            }
            Ok(reader)
        } else {
            Ok(Reader::from_first_file(
                &store,
                &source_key,
                section,
                0,
                None,
                cancelled,
            ))
        }
    };
    let mut values = reader(format!("factories-values-{branch}-{index}"))?;
    let mut ranks = reader(format!("factories-ranks-{branch}-{index}"))?;
    let mut mixes = reader(format!("factories-mixes-{branch}"))?;
    let mut supplying = reader(format!("factories-reads-{branch}"))?;
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(params.clone())?;
        let mut value_keys = Vec::new();
        let mut rank_keys = Vec::new();
        if let Some(top) = &selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            for position in 0..(top.count as usize).min(inverse.len()) {
                rank_keys.push((position as u64 * 2, None));
                if let Some(run) = ranks.value_at(position as u64 * 2, hour)? {
                    value_keys.push((run.value as u64, None));
                }
            }
        } else {
            for tag in country_history::selected_aliases(&history, &selection.tags) {
                if let Some(&key) = keys.get(&series::field_key(&tag, "")) {
                    value_keys.push((key, within.get(&tag).map(Vec::as_slice)));
                }
            }
        }
        let prepared_hour = series::navigate(
            &mut [(&mut values, value_keys), (&mut ranks, rank_keys)],
            &navigation,
        )?;
        let hour = if partial {
            let membership = if let Some(top) = &selection.top {
                top_membership(&transient, top, cancelled)?
            } else {
                country_history::selected_aliases(&history, &selection.tags)
                    .into_iter()
                    .map(|tag| {
                        let spans = within
                            .get(&tag)
                            .cloned()
                            .unwrap_or_else(|| history.countries[&tag].segments.as_ref().clone());
                        (tag, spans)
                    })
                    .collect()
            };
            let requested: Vec<_> = membership
                .keys()
                .map(|tag| (tag.clone(), String::new()))
                .collect();
            let prepared = requested
                .iter()
                .map(|key| (key.clone(), (prepared_hour, None)))
                .collect();
            progressive::navigate(
                &prepared,
                &transient_values,
                &requested,
                &membership,
                &navigation,
            )?
        } else {
            prepared_hour
        };
        return Ok(
            json!({"hour":hour,"coverage":crate::prepared::prefix::extent(source, branch, accepted)?,"approximate":hour.is_some_and(|hour| tail.1.iter().any(|span| f64::from(span.first_hour)<=hour && hour<=f64::from(span.last_hour)))}),
        );
    }
    if product == "events-page" {
        let (first, last, cursor, size) = series::event_window(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["cursor"].as_str(),
            params["pageSize"]
                .as_u64()
                .and_then(|size| u32::try_from(size).ok()),
        )?;
        let selected: Vec<_> = country_history::selected_aliases(&history, &selection.tags)
            .iter()
            .filter_map(|tag| keys.get(&series::field_key(tag, "")))
            .copied()
            .collect();
        let page = mixes.events(&selected, first, last, cursor, size)?;
        return Ok(
            json!({"firstOrdinal":page.first.to_string(),"total":page.total.to_string(),"cursor":page.cursor.map(|cursor|cursor.to_string()),
            "rows":page.rows.into_iter().map(|row|json!({"tag":inverse.get(&row.key),"hour":row.hour,"ordinal":row.ordinal.to_string(),"row":row.row})).collect::<Vec<_>>(),"warnings":[]}),
        );
    }
    let (from, to, width) = if product == "series-state" {
        (
            f64::from(reads.iter().map(|read| read.hour).min().unwrap_or(0)),
            f64::from(reads.iter().map(|read| read.hour).max().unwrap_or(0)),
            1,
        )
    } else {
        series::viewport(
            params["from"].as_f64(),
            params["to"].as_f64(),
            params["width"]
                .as_u64()
                .and_then(|width| u32::try_from(width).ok()),
        )?
    };
    let (first, last) = (from.floor() as i32, to.ceil() as i32);
    if product == "events-clusters" {
        let tag = params["tag"]
            .as_str()
            .ok_or_else(|| crate::invalid("Missing factory event tag"))?;
        validate_tag(tag).map_err(crate::invalid)?;
        let summary = if let Some(&key) = keys.get(&series::field_key(tag, "")) {
            mixes.event_clusters(key, from, to, width)?
        } else {
            crate::prepared_chart::MarkSummary {
                from,
                to,
                width,
                clusters: Vec::new(),
            }
        };
        return Ok(
            json!({"branchId":selection.branch_id,"tag":tag,"summary":summary,"warnings":[]}),
        );
    }
    let mut membership: Option<BTreeMap<String, Vec<HourSpan>>> =
        selection.top.as_ref().map(|_| BTreeMap::new());
    if let Some(top) = &selection.top {
        for position in 0..(top.count as usize).min(inverse.len()) {
            for run in ranks.runs(position as u64 * 2, first, last)? {
                let tag = inverse
                    .get(&(run.value as u64))
                    .ok_or_else(|| crate::invalid("Unknown factory rank key"))?;
                membership
                    .as_mut()
                    .unwrap()
                    .entry(tag.clone())
                    .or_default()
                    .push(HourSpan {
                        first_hour: run.first_hour,
                        last_hour: run.last_hour,
                    });
            }
        }
        for (tag, spans) in top_membership(&transient, top, cancelled)? {
            membership
                .as_mut()
                .unwrap()
                .entry(tag)
                .or_default()
                .extend(spans);
        }
    }
    if let Some(members) = &mut membership {
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
    let mut wanted: Vec<String> = membership.as_ref().map_or_else(
        || country_history::selected_aliases(&history, &selection.tags),
        |members| members.keys().cloned().collect(),
    );
    if product == "series-state" {
        wanted = read_tags;
        wanted.sort();
        wanted.dedup();
    }
    let selected = country_history::selected_identities(&history, &selection.tags);
    let dense =
        product != "series-state" && i64::from(last) - i64::from(first) + 1 > i64::from(width);
    let mut display = Vec::new();
    let mut parts = reader(format!("factories-parts-{branch}"))?;
    for tag in &wanted {
        let key = keys
            .get(&series::field_key(tag, ""))
            .copied()
            .unwrap_or(u64::MAX);
        let mut spans: Vec<_> = if let Some(members) = &membership {
            members.get(tag).cloned().unwrap_or_default()
        } else {
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected.contains(&(identity.slot, identity.country_id)))
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect()
        };
        if let Some(within) = within.get(tag) {
            let mut clipped = Vec::new();
            crate::ranking::for_each_piece(
                &spans,
                within,
                |span| (span.first_hour, span.last_hour),
                cancelled,
                |_, first_hour, last_hour| {
                    clipped.push(HourSpan {
                        first_hour,
                        last_hour,
                    });
                    Ok(())
                },
            )?;
            spans = clipped;
        }
        let tail_values = transient_values
            .get(&(tag.clone(), String::new()))
            .map_or(&[][..], Vec::as_slice);
        let mut summary = values.summary_with_tail(
            key,
            (first, last),
            (from, to, width),
            0,
            tail_values,
            Some(&spans),
        )?;
        progressive::clip_approximate(&mut summary, &tail.1, Some(&spans));
        let mut dates = BTreeSet::new();
        let mut part_summaries = Vec::new();
        if dense {
            for (ordinal, _) in fields().iter().enumerate() {
                {
                    let part_key = keys
                        .get(&series::field_key(tag, &ordinal.to_string()))
                        .copied()
                        .unwrap_or(u64::MAX);
                    let tail_parts = tail
                        .0
                        .get(&(tag.clone(), ordinal.to_string()))
                        .map_or(&[][..], Vec::as_slice);
                    let mut summary = parts.summary_with_tail(
                        part_key,
                        (first, last),
                        (from, to, width),
                        0,
                        tail_parts,
                        Some(&spans),
                    )?;
                    progressive::clip_approximate(&mut summary, &tail.1, Some(&spans));
                    for bucket in summary.buckets.iter().flatten() {
                        dates.extend([
                            bucket.first_hour,
                            bucket.last_hour,
                            bucket.min_hour,
                            bucket.max_hour,
                        ]);
                    }
                    part_summaries.push(json!({"key":ordinal.to_string(),"summary":summary}));
                }
            }
            for bucket in summary.buckets.iter().flatten() {
                dates.extend([
                    bucket.first_hour,
                    bucket.last_hour,
                    bucket.min_hour,
                    bucket.max_hour,
                ]);
            }
        }
        let runs = if dense || product == "series-state" {
            if product == "series-state" {
                dates.extend(
                    reads
                        .iter()
                        .filter(|read| &read.tag == tag)
                        .map(|read| read.hour),
                );
            }
            let mut runs = BTreeMap::new();
            for &hour in &dates {
                if let Some(run) = mixes.value_at(key, hour)? {
                    if product == "series-state" {
                        let starts = run.break_before && run.first_hour == hour;
                        runs.insert(
                            hour,
                            Run {
                                first_hour: hour,
                                last_hour: hour,
                                break_before: starts,
                                ..run
                            },
                        );
                    } else {
                        runs.insert(run.first_hour, run);
                    }
                }
            }
            runs.into_values().collect()
        } else {
            mixes.runs(key, first, last)?
        };
        let mut raw = Vec::new();
        for run in runs {
            let (_, row) = mixes
                .latest_event(key, run.first_hour)?
                .ok_or_else(|| crate::invalid("Missing prepared factory composition"))?;
            let mut value: FactoryRun = serde_json::from_value(row)?;
            value.first_hour = if product == "series-state" {
                run.first_hour
            } else {
                run.first_hour.max(first)
            };
            value.last_hour = if product == "series-state" {
                run.last_hour
            } else {
                run.last_hour.min(last)
            };
            value.starts_segment = run.break_before;
            if value.first_hour <= value.last_hour {
                raw.push(value);
            }
        }
        raw.extend(
            transient
                .get(tag)
                .into_iter()
                .flatten()
                .filter(|run| {
                    run.last_hour >= first
                        && run.first_hour <= last
                        && (!dense
                            || dates
                                .iter()
                                .any(|hour| run.first_hour <= *hour && *hour <= run.last_hour))
                })
                .map(|run| FactoryRun {
                    first_hour: run.first_hour.max(first),
                    last_hour: run.last_hour.min(last),
                    ..run.clone()
                }),
        );
        raw.sort_by_key(|run| run.first_hour);
        let read_runs = if dense || product == "series-state" {
            let mut found = BTreeMap::new();
            for run in &raw {
                for hour in [run.first_hour, run.last_hour] {
                    if let Some(read) = supplying.value_at(key, hour)? {
                        found.insert(read.first_hour, read);
                    }
                }
            }
            found.into_values().collect()
        } else {
            supplying.runs(key, first, last)?
        };
        let mut provenance = Vec::new();
        for run in read_runs {
            let (_, row) = supplying
                .latest_event(key, run.first_hour)?
                .ok_or_else(|| crate::invalid("Missing prepared factory supplying read"))?;
            let mut read: FactoryReadRun = serde_json::from_value(row)?;
            read.first_hour = run.first_hour;
            read.last_hour = run.last_hour;
            if read.hourly {
                read.read_hour = read.first_hour;
            }
            provenance.push(read);
        }
        for run in transient
            .get(tag)
            .into_iter()
            .flatten()
            .filter(|run| run.last_hour >= first && run.first_hour <= last)
        {
            let first_hour = run.first_hour.max(first);
            let approximate = tail
                .1
                .iter()
                .any(|span| span.first_hour <= first_hour && first_hour <= span.last_hour);
            let read_hour = if approximate {
                source
                    .replay_ref()
                    .index
                    .iter()
                    .find(|block| {
                        block.branch == branch
                            && block.first_hour <= first_hour
                            && first_hour <= block.last_hour
                    })
                    .map_or(first_hour, |block| block.first_hour)
            } else {
                first_hour
            };
            provenance.push(FactoryReadRun {
                first_hour,
                last_hour: run.last_hour.min(last),
                read_hour,
                hourly: !approximate,
            });
        }
        provenance.sort_by_key(|run| run.first_hour);
        if transient.contains_key(tag) {
            scanned.recorded.insert(tag.clone());
        }
        scanned.reads.insert(tag.clone(), provenance);
        scanned.series.insert(tag.clone(), raw);
        scanned.segments.insert(
            tag.clone(),
            history
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.segments.iter())
                .filter(|span| span.first_hour <= last && span.last_hour >= first)
                .cloned()
                .collect(),
        );
        if product != "series-state" {
            series::summary_budget(from, to, width, display.len() + 1)?;
            display.push(json!({"tag":tag,"summary":summary,"parts":part_summaries}));
        }
    }
    let mut params = selection;
    params.tags = wanted;
    let reply = present(
        source,
        &params,
        context,
        cancelled,
        &scanned,
        Some(&history),
        membership,
    )?;
    let mut reply = serde_json::to_value(reply)?;
    reply["summaries"] = json!(display);
    reply["coverage"] = json!(crate::prepared::prefix::extent(source, branch, accepted)?);
    super::super::check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
