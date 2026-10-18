//! Native nuclear stockpile observations in shared series, including append-only supplying reads.
use super::*;
use dashboard_base::prepared::query::progressive;
use dashboard_base::prepared::series::{self, Accumulator, Reader, Run};
use dashboard_base::products::country_history;
use serde_json::{Value, json};
const READS: u64 = u64::MAX;

pub const INCREASES: &str = "nukes-increases";
pub const INCREASE_KEY: u64 = 0;

/// A positive whole-stockpile change; type names resolve from its recorded tokens at query time.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StockpileIncrease {
    pub tag: String,
    pub slot: u32,
    pub country_id: u32,
    pub type_ordinal: Option<u32>,
    pub amount: u32,
    pub hour: i32,
    pub read_hour: i32,
    pub exact: bool,
    pub types: Vec<NukeTypeInfo>,
}

/// Feed availability and recorded type facts, independent of positive stockpile changes.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncreaseCatalogue {
    pub read_spans: Vec<HourSpan>,
    pub refusals: BTreeMap<String, Vec<HourSpan>>,
    pub refused_types: BTreeMap<u32, usize>,
    pub types: Vec<NukeTypeInfo>,
}

/// Complete numeric tables used by the stateless stockpile sample.
pub(crate) const TABLES: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::Nukes];

#[allow(clippy::type_complexity)]
pub(crate) fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    if requested.is_some_and(|keys| {
        keys.iter().any(|(_, field)| {
            !field.is_empty()
                && field
                    .parse::<u32>()
                    .ok()
                    .is_none_or(|ordinal| ordinal.to_string() != *field)
        })
    }) {
        return Err(dashboard_core::invalid(
            "Unknown nuclear stockpile sample field",
        ));
    }
    let wanted = requested.map(|keys| keys.iter().collect::<BTreeSet<_>>());
    let mut result = BTreeMap::new();
    if let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        let rows = frame.complete::<tables::Nukes>().ok();
        let (by_slot, _, _) = counts_by_slot(rows.unwrap_or_default(), links);
        let ordinals: BTreeSet<_> = rows
            .into_iter()
            .flatten()
            .map(|row| row.nuke_type)
            .collect();
        for (tag, lookup) in tagged_countries(countries, links) {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let identity = lookup.found().map(|(country, slot)| (slot, country.id));
            let value = identity
                .and_then(|(slot, _)| by_slot.get(&slot))
                .and_then(stockpile);
            let fields = std::iter::once(String::new()).chain(ordinals.iter().map(u32::to_string));
            for field in fields {
                let key = (tag.to_owned(), field.clone());
                if wanted.as_ref().is_some_and(|keys| !keys.contains(&key)) {
                    continue;
                }
                let amount = value.as_ref().and_then(|(counts, total)| {
                    if field.is_empty() {
                        total.map(i128::from)
                    } else {
                        counts
                            .iter()
                            .find(|count| count.ordinal.to_string() == field)
                            .map(|count| i128::from(count.value))
                    }
                });
                result.insert(key, (identity, amount));
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

fn sampled_runs(
    samples: &BTreeMap<(String, String), Vec<Run>>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, Vec<NukeRun>>> {
    let mut countries: BTreeMap<&str, BTreeMap<&str, &[Run]>> = BTreeMap::new();
    for ((tag, field), runs) in samples {
        countries.entry(tag).or_default().insert(field, runs);
    }
    let mut result = BTreeMap::new();
    for (tag, parts) in countries {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let dates: BTreeSet<_> = parts
            .values()
            .flat_map(|runs| runs.iter())
            .flat_map(|run| [i64::from(run.first_hour), i64::from(run.last_hour) + 1])
            .collect();
        let dates: Vec<_> = dates.into_iter().collect();
        for pair in dates.windows(2) {
            let first = pair[0] as i32;
            let mut counts = Vec::new();
            let mut total = None;
            let mut starts = false;
            for (field, runs) in &parts {
                let index = runs.partition_point(|run| run.last_hour < first);
                let Some(run) = runs.get(index).filter(|run| run.first_hour <= first) else {
                    continue;
                };
                let value = i32::try_from(run.value).map_err(io::Error::other)?;
                if field.is_empty() {
                    total = Some(value);
                } else {
                    counts.push(NukeCount {
                        ordinal: field.parse().map_err(io::Error::other)?,
                        value,
                    });
                }
                starts |= run.break_before && run.first_hour == first;
            }
            if !counts.is_empty() {
                counts.sort_by_key(|count| count.ordinal);
                result
                    .entry(tag.to_owned())
                    .or_insert_with(Vec::new)
                    .push(NukeRun {
                        first_hour: first,
                        last_hour: (pair[1] - 1) as i32,
                        starts_segment: starts,
                        counts,
                        total,
                    });
            }
        }
    }
    Ok(result)
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    keys: BTreeMap<String, u64>,
    values: Accumulator,
    ranks: Accumulator,
    parts: Accumulator,
    mixes: Accumulator,
    increases: Accumulator,
    feed_reads: Vec<HourSpan>,
    feed_refusals: BTreeMap<String, Vec<HourSpan>>,
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
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.scan
            .observe_changes(frame, replay.format, Some(changes));
        let refused = if slot_read(replay.format, &frame.metrics, TableId::Nukes, frame.hour)
            && let Err(error) = frame.complete::<tables::Nukes>()
        {
            Some(
                if let replay_frame::frame::TableView::Unavailable { reason } =
                    &*frame.tables[TableId::Nukes.index()]
                {
                    reason.clone()
                } else {
                    let text = error.to_string();
                    text.strip_prefix(&format!("hour {} ", frame.hour))
                        .unwrap_or(&text)
                        .to_owned()
                },
            )
        } else {
            None
        };
        self.advance(frame.hour, refused)?;
        for tag in std::mem::take(&mut self.scan.observed) {
            let track = &self.scan.tracks[&tag];
            let raw = track
                .runs
                .last()
                .filter(|run| run.last_hour == frame.hour)
                .cloned();
            let ordinals = track
                .runs
                .iter()
                .rev()
                .take(2)
                .flat_map(|run| run.counts.iter().map(|count| count.ordinal))
                .collect::<BTreeSet<_>>();
            let key = self.key(&tag, "");
            #[cfg(test)]
            {
                self.scan.work.emitted += 2 + ordinals.len();
            }
            let Some(raw) = raw else {
                self.values.observe(key, frame.hour, None, false);
                self.mixes.observe(key, frame.hour, None, false);
                for ordinal in ordinals {
                    let part = self.key(&tag, &ordinal.to_string());
                    self.parts.observe(part, frame.hour, None, false);
                }
                continue;
            };
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
                let track = &self.scan.tracks[&tag];
                if !raw.starts_segment
                    && let (Some(before), Some(after)) = (
                        track.runs.iter().rev().nth(1).and_then(|run| run.total),
                        raw.total,
                    )
                {
                    let increase = i64::from(after) - i64::from(before);
                    if increase > 0 {
                        let (slot, country_id) = track.last.expect("observed native holder").0;
                        self.increases.event(
                            INCREASE_KEY,
                            frame.hour,
                            &StockpileIncrease {
                                tag: tag.clone(),
                                slot,
                                country_id,
                                type_ordinal: None,
                                amount: u32::try_from(increase).map_err(io::Error::other)?,
                                hour: raw.first_hour,
                                read_hour: frame.hour,
                                exact: false,
                                types: type_info(&self.scan.scanned, |_| None),
                            },
                        )?;
                    }
                }
            }
            self.mixes.observe(key, frame.hour, Some(version), starts);
            self.values
                .observe(key, frame.hour, raw.total.map(i128::from), starts);
            for ordinal in ordinals {
                let key = self.key(&tag, &ordinal.to_string());
                self.parts.observe(
                    key,
                    frame.hour,
                    raw.counts
                        .iter()
                        .find(|count| count.ordinal == ordinal)
                        .map(|count| i128::from(count.value)),
                    starts,
                );
            }
        }
        Ok(())
    }
    /// The accepted hour every series takes, observed or quiet: a dated read
    /// of a stockpile table that is not complete (`refused`, its reason), and
    /// each read hour the scan dated, with whether its stockpiles were empty.
    fn advance(&mut self, hour: i32, refused: Option<String>) -> io::Result<()> {
        self.first.get_or_insert(hour);
        self.last = hour;
        self.values.advance(hour)?;
        self.parts.advance(hour)?;
        self.mixes.advance(hour)?;
        self.increases.advance(hour)?;
        if let Some(reason) = refused {
            dashboard_base::products::country_groups::mark_unobserved(
                self.feed_refusals.entry(reason).or_default(),
                hour,
                hour,
            );
        }
        for hour in self.scan.scanned.read_hours.drain(..) {
            dashboard_base::products::country_groups::mark_unobserved(
                &mut self.feed_reads,
                hour,
                hour,
            );
            self.mixes.event(
                READS,
                hour,
                &self.scan.scanned.empty_read_hours.contains(&hour),
            )?;
        }
        self.scan.scanned.empty_read_hours.clear();
        Ok(())
    }
    /// What observing `hour` does when it follows the last hour observed and
    /// none of the countries' keys, links, name tokens, availability or
    /// stockpile rows changed there, the stockpile table read (`read`) or not
    /// as at the hour before: the read it dates, the unchanged hour's counts
    /// and every series' accepted hour, in constant time. `Ok(false)`, with
    /// nothing changed, when observing it needs its frame.
    pub fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<bool> {
        if !self.scan.advance_quiet(hour, read) {
            return Ok(false);
        }
        self.advance(hour, None)?;
        Ok(true)
    }
    pub fn bytes(&self) -> usize {
        self.values.bytes()
            + self.ranks.bytes()
            + self.parts.bytes()
            + self.mixes.bytes()
            + self.increases.bytes()
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
                        .map(|run| NukeRun {
                            first_hour: run.first_hour.max(first),
                            ..run.clone()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        self.ranks.through(first);
        dashboard_base::ranking::rank_positions(
            &runs,
            |run| (run.first_hour, run.last_hour),
            |run| run.total.filter(|total| *total != 0).map(i128::from),
            cancelled,
            |position, tag, from, to| {
                let key = self.keys[&series::field_key(tag, "")];
                self.ranks.push(
                    position as u64 * 2,
                    Run::new(from, to, i128::from(key), false),
                );
                self.ranks
                    .push(key * 2 + 1, Run::new(from, to, position as i128 + 1, false));
                Ok(())
            },
        )?;
        self.ranks.through(self.last);
        self.ranks
            .flush(writer, &format!("nukes-ranks-{branch}"), 0, cancelled)?;
        self.values
            .flush(writer, &format!("nukes-values-{branch}"), 0, cancelled)?;
        self.parts
            .flush(writer, &format!("nukes-parts-{branch}"), 0, cancelled)?;
        self.mixes
            .flush(writer, &format!("nukes-mixes-{branch}"), 0, cancelled)?;
        self.increases
            .flush(writer, &format!("{INCREASES}-{branch}"), 0, cancelled)?;
        for track in self.scan.tracks.values_mut() {
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
            if track.segments.len() > 1 {
                track.segments.drain(..track.segments.len() - 1);
            }
        }
        let types: BTreeMap<_, _> = native_type_info(&self.scan.scanned, |_| None)
            .into_iter()
            .chain(type_info(&self.scan.scanned, |_| None))
            .map(|info| (info.ordinal, info))
            .collect();
        series::write_catalogue(
            writer,
            INCREASES,
            branch,
            &BTreeMap::new(),
            &IncreaseCatalogue {
                read_spans: self.feed_reads.clone(),
                refusals: self.feed_refusals.clone(),
                refused_types: self.scan.scanned.refused.clone(),
                types: types.into_values().collect(),
            },
        )?;
        series::write_catalogue(writer, "nukes", branch, &self.keys, &self.scan.scanned)
    }
}
pub fn query(
    source: &Source,
    product: &str,
    params: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let reads: Vec<dashboard_base::prepared_measure::query::MeasureRead> =
        if product == "series-state" {
            serde_json::from_value(params["reads"].clone())?
        } else {
            Vec::new()
        };
    let read_tags: Vec<_> = reads.iter().map(|read| read.tag.clone()).collect();
    let selection: NukesParams = serde_json::from_value(
        json!({"branchId":params["branchId"], "tags":if product=="series-state" {json!(read_tags)} else {params.get("tags").cloned().unwrap_or_else(||json!([]))},
        "top":params["family"]["top"], "originHour":params["family"]["originHour"]}),
    )?;
    CountryNukes::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid nuclear branch"))?;
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    // Prepared on their own, the stockpiles and the identity history they are
    // named through answer from their own coverage.
    let accepted = match dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        CountryNukes::KEY,
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
        format!("nukes-values-{branch}"),
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
        .map(|tag| (tag.clone(), String::new()))
        .collect();
    let transient_reads = std::cell::RefCell::new(BTreeMap::new());
    let sample_all = |frame: &DecodedFrame, _: Option<&[(String, String)]>| {
        if let Some(empty) = supplying_read(frame, source.replay_ref().format) {
            transient_reads.borrow_mut().insert(frame.hour, empty);
        }
        sample(frame, None)
    };
    let tail = if partial && product == "series-state" {
        progressive::state_tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
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
        progressive::tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
            (from.floor() as i32, to.floor() as i32),
            &sample_all,
            cancelled,
        )?
    } else {
        Default::default()
    };
    let transient = sampled_runs(&tail.0, cancelled)?;
    let origin = if partial && let Some(hour) = selection.origin_hour {
        progressive::state_tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
            &[hour],
            &sample_all,
            cancelled,
        )?
        .2
    } else {
        BTreeMap::new()
    };
    let history = Arc::new(progressive::histories(
        &store,
        &source_key,
        branch,
        accepted.as_ref(),
        &tail.2,
        &origin,
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
        series::read_catalogue::<ScannedNukes>(&store, &source_key, "nukes", branch)?
            .unwrap_or_default()
    } else {
        Default::default()
    };
    scanned.types.extend(
        tail.0
            .keys()
            .filter_map(|(_, field)| field.parse::<u32>().ok()),
    );

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
    let mut values = reader(format!("nukes-values-{branch}"))?;
    let mut ranks = reader(format!("nukes-ranks-{branch}"))?;
    let mut mixes = reader(format!("nukes-mixes-{branch}"))?;
    let within: BTreeMap<String, Vec<HourSpan>> =
        serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
    let identity_membership = if selection.top.is_none() {
        membership(&scanned, &selection, Some(&history), cancelled)?
    } else {
        None
    };
    let mut navigation_membership = identity_membership.clone();
    if let Some(members) = &mut navigation_membership {
        for (tag, spans) in members {
            if let Some(within) = within.get(tag) {
                let mut clipped = Vec::new();
                dashboard_base::ranking::for_each_piece(
                    spans,
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
                *spans = clipped;
            }
        }
    }
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(params.clone())?;
        let mut value_keys = Vec::new();
        let mut rank_keys = Vec::new();
        if let Some(top) = selection.top {
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            for position in 0..(top as usize).min(inverse.len()) {
                rank_keys.push((position as u64 * 2, None));
                if let Some(run) = ranks.value_at(position as u64 * 2, hour)? {
                    value_keys.push((run.value as u64, None));
                }
            }
        } else {
            for (tag, spans) in navigation_membership
                .as_ref()
                .into_iter()
                .flat_map(|members| members.iter())
            {
                if let Some(&key) = keys.get(&series::field_key(tag, "")) {
                    value_keys.push((key, Some(spans.as_slice())));
                }
            }
        }
        let prepared_hour = series::navigate(
            &mut [(&mut values, value_keys), (&mut ranks, rank_keys)],
            &navigation,
        )?;
        let hour = if partial {
            let membership = if let Some(top) = selection.top {
                dashboard_base::ranking::top_membership(
                    &transient,
                    top as usize,
                    |run| (run.first_hour, run.last_hour),
                    |run| run.total.filter(|value| *value != 0).map(i64::from),
                    cancelled,
                )?
            } else {
                navigation_membership.clone().unwrap_or_default()
            };
            let requested: Vec<_> = membership
                .keys()
                .map(|tag| (tag.clone(), String::new()))
                .collect();
            let prepared = requested
                .iter()
                .map(|key| (key.clone(), (prepared_hour, None)))
                .collect();
            progressive::navigate(&prepared, &tail.0, &requested, &membership, &navigation)?
        } else {
            prepared_hour
        };
        return Ok(
            json!({"hour":hour,"coverage":dashboard_base::prepared::prefix::extent(source, branch, accepted)?,"approximate":hour.is_some_and(|hour| tail.1.iter().any(|span| f64::from(span.first_hour)<=hour && hour<=f64::from(span.last_hour)))}),
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
        let selected: Vec<_> = membership(&scanned, &selection, Some(&history), cancelled)?
            .unwrap_or_default()
            .into_keys()
            .collect::<Vec<_>>()
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
            .ok_or_else(|| dashboard_core::invalid("Missing nuclear event tag"))?;
        validate_tag(tag).map_err(dashboard_core::invalid)?;
        let summary = if let Some(&key) = keys.get(&series::field_key(tag, "")) {
            mixes.event_clusters(key, from, to, width)?
        } else {
            dashboard_base::prepared_chart::MarkSummary {
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
    let mut membership = if selection.top.is_some() {
        Some(BTreeMap::new())
    } else {
        identity_membership
    };
    if let Some(top) = selection.top {
        for position in 0..(top as usize).min(inverse.len()) {
            for run in ranks.runs(position as u64 * 2, first, last)? {
                let tag = inverse
                    .get(&(run.value as u64))
                    .ok_or_else(|| dashboard_core::invalid("Unknown nuclear rank key"))?;
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
        for (tag, spans) in dashboard_base::ranking::top_membership(
            &transient,
            top as usize,
            |run| (run.first_hour, run.last_hour),
            |run| run.total.filter(|value| *value != 0).map(i64::from),
            cancelled,
        )? {
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
                dashboard_base::products::country_groups::mark_unobserved(
                    &mut merged,
                    span.first_hour,
                    span.last_hour,
                );
            }
            *spans = merged;
        }
    }
    let mut wanted: Vec<String> = membership.as_ref().map_or_else(
        || selection.tags.clone(),
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
    let mut supplying = BTreeMap::new();
    let mut parts = reader(format!("nukes-parts-{branch}"))?;
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
            dashboard_base::ranking::for_each_piece(
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
        let tail_values = tail
            .0
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
            for ordinal in &scanned.types {
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
                .ok_or_else(|| dashboard_core::invalid("Missing prepared nuclear composition"))?;
            let mut value: NukeRun = serde_json::from_value(row)?;
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
        // Keep supplying dates bound to the prepared mixtures that witnessed them.
        for run in &raw {
            for hour in [run.first_hour, run.last_hour] {
                if let Some((at, row)) = mixes.latest_event(READS, hour)? {
                    supplying.insert(at, row);
                }
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
                .map(|run| NukeRun {
                    first_hour: run.first_hour.max(first),
                    last_hour: run.last_hour.min(last),
                    ..run.clone()
                }),
        );
        raw.sort_by_key(|run| run.first_hour);
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
    if !dense && product != "series-state" {
        let mut cursor = 0;
        loop {
            let page = mixes.events(&[READS], first, last, cursor, 1024)?;
            for row in page.rows {
                supplying.insert(row.hour, row.row);
            }
            let Some(next) = page.cursor else { break };
            if next <= cursor {
                return Err(dashboard_core::invalid(
                    "Prepared nuclear supplying reads did not advance",
                ));
            }
            cursor = next;
        }
    }
    for (hour, empty) in transient_reads.into_inner() {
        if hour >= first
            && hour <= last
            && accepted
                .as_ref()
                .is_none_or(|coverage| hour > coverage.last_hour)
            && (product != "series-state" || reads.iter().any(|read| read.hour == hour))
        {
            supplying.insert(hour, json!(empty));
        }
    }
    scanned.read_hours = supplying.keys().copied().collect();
    scanned.empty_read_hours = supplying
        .iter()
        .filter_map(|(&hour, row)| row.as_bool().unwrap_or(false).then_some(hour))
        .collect();
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
    reply["coverage"] = json!(dashboard_base::prepared::prefix::extent(
        source, branch, accepted
    )?);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

#[cfg(test)]
mod tests;
