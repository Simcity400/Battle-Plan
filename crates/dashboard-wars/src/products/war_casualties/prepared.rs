//! The shared casualty ingestion feeding exact country values and rank positions.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader, Run};
use replay::frame::DecodedFrame;
use serde_json::{Value, json};

pub mod block;

/// The section holding a branch's pair windows, one range per block.
fn pairs_section(branch: u64) -> String {
    format!("casualty-pairs-{branch}")
}

pub struct Presentation {
    catalogue: Arc<dashboard_text::catalogue::Catalogue>,
    unknown_label: String,
    /// What reading the unknown-attacker title refused, which belongs to
    /// every family that shows it.
    unknown_refusals: dashboard_text::catalogue::OwnRefusals,
}
pub(crate) type PreparedContent = Result<Option<Arc<Presentation>>, String>;

/// Static presentation resolves once per Source/content binding, including refusals.
pub fn prepared_content(source: &Source, context: &LoadContext) -> io::Result<PreparedContent> {
    let Some(options) = context.content.as_ref() else {
        return Ok(Ok(None));
    };
    match dashboard_base::catalogue::catalogue(source, options) {
        Ok(catalogue) => {
            let labels = catalogue
                .content
                .layers
                .derived::<dashboard_text::catalogue::KeyedResults<(String, String), String>>();
            // The shared result credits its refusal on every read.
            let (unknown_label, unknown_refusals) = dashboard_text::catalogue::capture(|| {
                labels
                    .get_or_init(
                        &(UNKNOWN_LABEL.to_owned(), "plain".to_owned()),
                        |(key, _)| {
                            catalogue
                                .localised(key, dashboard_text::catalogue::View::Plain)
                                .ok_or_else(|| {
                                    dashboard_core::invalid(
                                        "Casualty unknown-attacker title is unavailable",
                                    )
                                })
                        },
                    )
                    .map(|label| (*label).clone())
                    .unwrap_or_else(|_| "Unknown attacker".into())
            });
            Ok(Ok(Some(Arc::new(Presentation {
                catalogue,
                unknown_label,
                unknown_refusals,
            }))))
        }
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(error) => Ok(Err(error.to_string())),
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Observation {
    first: i32,
    sample_first: i32,
    identity: Identity,
    /// None means every covered hour is a successful supplying read.
    read_hour: Option<i32>,
    at_least: bool,
}
impl Observation {
    fn sample(&self, dealt: i128, taken: i128, unknown: Option<i64>, last: i32) -> Sample {
        Sample {
            first: self.sample_first,
            last,
            confirmed: self.read_hour.unwrap_or(last),
            identity: self.identity,
            dealt,
            taken,
            unknown,
            at_least: self.at_least,
        }
    }
}
pub fn actor_key(tag: &str, identity: (u32, u32)) -> io::Result<String> {
    Ok(serde_json::to_string(&(tag, identity))?)
}
/// Exact point values for downstream tables, with native supplying dates and partial flags.
pub fn read_measures(
    store: &store::Store,
    source: &store::SourceKey,
    branch: u64,
    keys: &[u64],
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Vec<Option<CasualtyMeasure>>> {
    let mut coverage = Reader::new(
        store,
        source,
        format!("casualty-coverage-{branch}"),
        cancelled,
    )?;
    if coverage.value_at(0, hour)?.is_none() {
        return Ok((0..keys.len()).map(|_| None).collect());
    }
    let values = (0..3)
        .map(|index| {
            Reader::new(
                store,
                source,
                format!("casualty-values-{branch}-{index}"),
                cancelled,
            )?
            .values_at(keys, hour)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut observations = Reader::new(
        store,
        source,
        format!("casualty-observations-{branch}"),
        cancelled,
    )?;
    keys.iter()
        .enumerate()
        .map(|(index, &key)| {
            let (Some(dealt), Some(taken)) = (values[0][index], values[1][index]) else {
                return Ok(None);
            };
            let Some((_, row)) = observations.latest_event(key, hour)? else {
                return Ok(None);
            };
            let observation: Observation = serde_json::from_value(row)?;
            Ok(Some(
                observation
                    .sample(
                        dealt,
                        taken,
                        values[2][index].map(|value| value as i64),
                        hour,
                    )
                    .measure_at(hour),
            ))
        })
        .collect()
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Catalogue {
    actors: BTreeMap<u64, (String, Identity)>,
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_sets")]
    pub(super) holders: BTreeMap<String, HashSet<Identity>>,
    #[serde(with = "dashboard_core::map_pairs")]
    pub(super) holder_coverage: BTreeMap<(Identity, Identity), Vec<pairs::HolderRun>>,
    labels: CampaignLabels,
    unknown_label: String,
    warnings: Vec<String>,
    pub(super) incomplete: bool,
    pub(super) pair_available: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    builder: ScanFrames,
    pairs: pairs::PairHistory,
    next_pair_window: u64,
    tags: BTreeMap<String, u64>,
    values: [Accumulator; 3],
    observations: Accumulator,
    coverage: Accumulator,
    covered: bool,
    ranks: Accumulator,
    latest: BTreeMap<u64, Observation>,
    actors: BTreeMap<String, u64>,
    catalogue: Catalogue,
    first: Option<i32>,
    last: i32,
    /// The content refusals this family's own reads produced, which its
    /// published warnings report.
    #[serde(default)]
    refusals: dashboard_text::catalogue::OwnRefusals,
    #[serde(skip)]
    named: usize,
    #[serde(skip)]
    presentation: Option<Arc<Presentation>>,
    #[serde(skip)]
    presentation_error: Option<String>,
    #[serde(skip)]
    configured: bool,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        Self {
            builder: ScanFrames::new(replay, branch),
            pairs: Default::default(),
            next_pair_window: 0,
            tags: BTreeMap::new(),
            values: Default::default(),
            observations: Default::default(),
            coverage: Default::default(),
            covered: false,
            ranks: Default::default(),
            latest: BTreeMap::new(),
            actors: BTreeMap::new(),
            catalogue: Catalogue {
                unknown_label: "Unknown attacker".into(),
                ..Default::default()
            },
            first: None,
            last: 0,
            refusals: Default::default(),
            named: 0,
            presentation: None,
            presentation_error: None,
            configured: false,
            #[cfg(any(test, feature = "test-support"))]
            test_changes: None,
            #[cfg(any(test, feature = "test-support"))]
            work: Default::default(),
        }
    }

    pub fn configure(&mut self, content: &PreparedContent) {
        if self.configured {
            return;
        }
        self.configured = true;
        match content {
            Ok(Some(content)) => {
                self.catalogue
                    .unknown_label
                    .clone_from(&content.unknown_label);
                self.refusals.merge(&content.unknown_refusals);
                self.presentation = Some(Arc::clone(content));
            }
            Ok(None) => {}
            Err(error) => self.presentation_error = Some(error.clone()),
        }
    }

    /// Runs `run` as this family's own consumer of the shared content
    /// ([`dashboard_text::catalogue::own_refusals`]).
    fn own<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mut own = std::mem::take(&mut self.refusals);
        let value = dashboard_text::catalogue::own_refusals(&mut own, || run(self));
        self.refusals = own;
        value
    }

    pub fn observe_changes(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        cancelled: &AtomicBool,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own_changes(frame, replay, cancelled, changes))
    }

    fn observe_own_changes(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        cancelled: &AtomicBool,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        check_cadence(cadence::cadence_in(TableId::WarRelations, replay.format))?;
        for values in &mut self.values {
            values.advance(frame.hour)?;
        }
        self.observations.advance(frame.hour)?;
        self.coverage.advance(frame.hour)?;
        let continuous = self
            .builder
            .scan
            .hour
            .is_some_and(|hour| hour.checked_add(1) == Some(frame.hour));
        self.builder.frame_changes(frame, replay, changes);
        self.pairs.observe_changes(&self.builder);
        self.first.get_or_insert(frame.hour);
        self.last = frame.hour;
        let observed = self.builder.scan.hour == Some(frame.hour);
        if !continuous || observed != self.covered {
            self.coverage
                .observe(0, frame.hour, observed.then_some(1), false);
            self.covered = observed;
        }
        let tags = std::mem::take(&mut self.builder.changed_tags);
        let sampled = self.observe_tags(&tags, frame.hour, continuous, observed);
        self.builder.changed_tags = tags;
        sampled?;
        self.name_tags(cancelled)
    }

    /// Each changed tag's values and observation at `hour`, in tag order,
    /// from its track's open sample: what observing an hour publishes for
    /// the tags its frame changed. `continuous` when the hour follows the
    /// hour observed before; `observed` when it is a supplying read.
    fn observe_tags<'a>(
        &mut self,
        tags: impl IntoIterator<Item = &'a String>,
        hour: i32,
        continuous: bool,
        observed: bool,
    ) -> io::Result<()> {
        let scan = &self.builder.scan;
        for tag in tags {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            let Some(track) = scan.tracks.get(tag) else {
                continue;
            };
            let Some(tail) = &track.tail else {
                continue;
            };
            // The tag's held actor when it names this identity: its key is
            // the one its actor key numbers, with no key to spell.
            let held = self.actors.get(tag).copied().filter(|key| {
                self.catalogue
                    .actors
                    .get(key)
                    .is_some_and(|(named, identity)| named == tag && *identity == tail.identity)
            });
            let key = match held {
                Some(key) => key,
                None => {
                    let next = self.tags.len() as u64;
                    let key = *self
                        .tags
                        .entry(actor_key(tag, tail.identity)?)
                        .or_insert(next);
                    self.catalogue
                        .actors
                        .entry(key)
                        .or_insert_with(|| (tag.clone(), tail.identity));
                    key
                }
            };
            let sample = track
                .tail
                .as_ref()
                .filter(|run| observed && run.last == hour);
            let previous = self.actors.get(tag).copied();
            if let Some(previous) = previous.filter(|previous| *previous != key || sample.is_none())
            {
                for values in &mut self.values {
                    values.observe(previous, hour, None, false);
                }
                self.observations.observe(previous, hour, None, false);
                self.actors.remove(tag);
            }
            let starts = sample.is_some() && (previous != Some(key) || !continuous);
            for (index, values) in self.values.iter_mut().enumerate() {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.emitted += 1;
                }
                values.observe(
                    key,
                    hour,
                    sample.and_then(|sample| match index {
                        0 => Some(sample.dealt),
                        1 => Some(sample.taken),
                        _ => sample.unknown.map(i128::from),
                    }),
                    starts,
                );
            }
            if let Some(sample) = sample {
                self.actors.insert(tag.clone(), key);
                let observation = Observation {
                    first: if starts {
                        hour
                    } else {
                        self.latest[&key].first
                    },
                    sample_first: sample.first,
                    identity: sample.identity,
                    read_hour: (sample.confirmed != hour).then_some(sample.confirmed),
                    at_least: sample.at_least,
                };
                if starts || self.latest.get(&key) != Some(&observation) {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.emitted += 1;
                    }
                    self.observations.event(key, hour, &observation)?;
                    self.latest.insert(key, observation);
                }
                let version = self.observations.state(key).map_or(0, |state| state.events);
                self.observations
                    .observe(key, hour, Some(i128::from(version)), starts);
            } else {
                self.observations.observe(key, hour, None, false);
            }
        }
        Ok(())
    }

    /// Label the naming operands the scan met since they were last
    /// labelled, and refresh the warnings, as observing an hour does after
    /// its tags.
    fn name_tags(&mut self, cancelled: &AtomicBool) -> io::Result<()> {
        let scan = &self.builder.scan;
        if self.named < scan.names.len() || self.catalogue.warnings.is_empty() {
            if let Some(presentation) = self.presentation.as_ref() {
                let content = &presentation.catalogue;
                for (tag, inputs) in &scan.names[self.named..] {
                    check_cancel(cancelled)?;
                    let name = country_label(content, tag, inputs);
                    let labels = self.catalogue.labels.entry(tag.clone()).or_default();
                    if !labels.contains(&name) {
                        labels.push(name);
                    }
                }
            }
            self.named = scan.names.len();
            self.refresh_warnings();
        }
        Ok(())
    }

    /// The shared content's warnings and this family's own refusals so far.
    /// Publication refreshes them too, so they never depend on when naming
    /// inputs were last read.
    fn refresh_warnings(&mut self) {
        self.catalogue.warnings = match (&self.presentation, &self.presentation_error) {
            (Some(presentation), _) => {
                let content = &presentation.catalogue;
                content
                    .verification
                    .iter()
                    .chain(&content.warnings)
                    .cloned()
                    .chain(content.refusal_lines())
                    .collect()
            }
            (None, None) => {
                vec!["Game content is unavailable; casualty titles are unavailable.".into()]
            }
            (None, Some(reason)) => vec![format!("Casualty titles are unavailable: {reason}")],
        };
    }

    pub fn bytes(&self) -> usize {
        self.values.iter().map(Accumulator::bytes).sum::<usize>()
            + self.observations.bytes()
            + self.coverage.bytes()
            + self.ranks.bytes()
            + self.pairs.buffered_bytes()
    }

    pub fn finish_block(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        first_hour: i32,
        last_hour: i32,
    ) -> io::Result<()> {
        let (ordinal, window) = self.finish_block_window(first_hour, last_hour)?;
        writer.write_at(&pairs_section(branch), ordinal, window.as_slice())
    }

    /// The pair window of the block `first_hour..=last_hour` and its ordinal,
    /// as [`Self::finish_block`] publishes it: preparation on the family's
    /// own writes it with its next flush.
    pub(crate) fn finish_block_window(
        &mut self,
        first_hour: i32,
        last_hour: i32,
    ) -> io::Result<(u64, Vec<u8>)> {
        self.builder.scan.seal();
        self.pairs.seal();
        let span = HourSpan {
            first_hour,
            last_hour,
        };
        let window = self.pairs.window(span, &self.builder.scan.observed);
        // `series::write_reply`'s bytes.
        let window = series::compress(serde_json::to_vec(&window)?.as_slice(), 3)?;
        let observed = self
            .builder
            .scan
            .observed
            .iter()
            .any(|read| read.first_hour <= span.last_hour && span.first_hour <= read.last_hour);
        self.catalogue.pair_available |= observed;
        let ordinal = self.next_pair_window;
        self.next_pair_window += 1;
        Ok((ordinal, window))
    }

    /// Whether hours `first..=last`, read by their changes, can advance in
    /// constant time ([`Self::advance_quiet`]): every one read WarRelations
    /// (`read`) and no field the scan reads changed or was decoded again
    /// there, and the carried scan continues the hour it observed last as a
    /// covered supplying read, with nothing left to name or join.
    pub(crate) fn quiet_continues(&self, first: i32, read: bool) -> bool {
        read && self.first.is_some()
            && self.covered
            && self.named == self.builder.scan.names.len()
            && self.builder.quiet_continues(first)
            && self.pairs.quiet_continues(first)
    }

    /// Hours `first..=last` that [`Self::quiet_continues`] allows: what
    /// observing each does, in constant time. Each authenticates the hour in
    /// every series and confirms the scan's read; no country, pair or value
    /// is visited.
    pub(crate) fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<()> {
        for values in &mut self.values {
            values.advance_span(first, last)?;
        }
        self.observations.advance_span(first, last)?;
        self.coverage.advance_span(first, last)?;
        self.builder.advance_quiet(first, last);
        self.pairs.advance_quiet(last);
        self.last = last;
        Ok(())
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.own(|state| state.flush_own(writer, branch, cancelled))
    }

    fn flush_own(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Some(first) = self.first.take() else {
            return Ok(());
        };
        self.builder.scan.seal();
        self.pairs.seal();
        self.refresh_warnings();
        let scan = &mut self.builder.scan;
        let last = self.last;
        let samples: BTreeMap<_, _> = scan
            .tracks
            .iter()
            .map(|(tag, track)| {
                (
                    tag.clone(),
                    track
                        .iter()
                        .flat_map(|sample| {
                            scan.observed.iter().filter_map(move |span| {
                                let from = sample.first.max(span.first_hour).max(first);
                                let to = sample.last.min(span.last_hour).min(last);
                                (from <= to).then(|| Sample {
                                    first: from,
                                    last: to,
                                    ..sample.clone()
                                })
                            })
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        self.ranks.through(first);
        dashboard_base::ranking::rank_positions(
            &samples,
            |run| (run.first, run.last),
            |run| Some(run.dealt + run.taken),
            cancelled,
            |rank, tag, from, to| {
                let runs = &samples[tag];
                let index = runs.partition_point(|run| run.last < from);
                let actor = self.tags[&serde_json::to_string(&(tag, runs[index].identity))?];
                self.ranks.push(
                    rank as u64 * 2,
                    Run::new(from, to, i128::from(actor), false),
                );
                self.ranks
                    .push(actor * 2 + 1, Run::new(from, to, rank as i128 + 1, false));
                Ok(())
            },
        )?;
        self.ranks.through(self.last);
        self.ranks
            .flush(writer, &format!("casualty-ranks-{branch}"), 0, cancelled)?;
        for (index, values) in self.values.iter_mut().enumerate() {
            values.flush(
                writer,
                &format!("casualty-values-{branch}-{index}"),
                0,
                cancelled,
            )?;
        }
        self.observations.flush(
            writer,
            &format!("casualty-observations-{branch}"),
            0,
            cancelled,
        )?;
        self.coverage
            .flush(writer, &format!("casualty-coverage-{branch}"), 0, cancelled)?;
        self.catalogue.holders.clone_from(&scan.holders);
        self.catalogue
            .holder_coverage
            .clone_from(&self.pairs.coverage);
        self.catalogue.incomplete = scan.incomplete;
        series::write_catalogue(writer, "casualty", branch, &self.tags, &self.catalogue)?;
        // Only the open sample and counter operands continue the next batch.
        for track in scan.tracks.values_mut() {
            track.packed.clear();
            track.checkpoints.clear();
            track.packed_runs = 0;
            track.previous = Sample::default();
        }
        if scan.observed.len() > 1 {
            scan.observed.drain(..scan.observed.len() - 1);
        }
        scan.bounds.clear();
        scan.names.clear();
        self.builder.name_entries.clear();
        self.named = 0;
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Params {
    branch_id: String,
    #[serde(default)]
    tags: Vec<String>,
    family: Family,
    #[serde(default)]
    reads: Vec<dashboard_base::prepared_measure::query::MeasureRead>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    cursor: Option<String>,
    page_size: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    top: Option<CasualtyTop>,
    #[serde(default = "default_measure")]
    measure: String,
}
fn default_measure() -> String {
    "taken".into()
}

pub fn query(
    product: &str,
    params: Value,
    store: &store::Store,
    source: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let raw = params.clone();
    let params: Params = serde_json::from_value(params)?;
    let selection = CasualtyParams {
        branch_id: params.branch_id.clone(),
        tags: params.tags,
        top: params.family.top,
    };
    WarCasualties::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid casualty branch"))?;
    let Some((tags, catalogue)) =
        series::read_catalogue::<Catalogue>(store, source, "casualty", branch)?
    else {
        return Ok(Value::Null);
    };
    let mut observations = Reader::new(
        store,
        source,
        format!("casualty-observations-{branch}"),
        cancelled,
    )?;
    let mut ranks = Reader::new(store, source, format!("casualty-ranks-{branch}"), cancelled)?;
    let mut coverage = Reader::new(
        store,
        source,
        format!("casualty-coverage-{branch}"),
        cancelled,
    )?;
    let mut values = (0..3)
        .map(|index| {
            Reader::new(
                store,
                source,
                format!("casualty-values-{branch}-{index}"),
                cancelled,
            )
        })
        .collect::<io::Result<Vec<_>>>()?;
    let selected: HashSet<_> = selection
        .tags
        .iter()
        .filter_map(|tag| catalogue.holders.get(tag))
        .flatten()
        .copied()
        .collect();
    let wanted: Vec<_> = catalogue
        .actors
        .iter()
        .filter(|(_, (_, identity))| selection.tags.is_empty() || selected.contains(identity))
        .map(|(&key, (tag, _))| (tag, key))
        .collect();
    let channel = match params.family.measure.as_str() {
        "dealt" => 0,
        "taken" => 1,
        "unknown" => 2,
        _ => return Err(dashboard_core::invalid("Unknown casualty measure")),
    };
    match product {
        "series-state" => {
            let hour = params
                .reads
                .first()
                .map(|read| read.hour)
                .ok_or_else(|| dashboard_core::invalid("Missing casualty read hour"))?;
            if params.reads.iter().any(|read| read.hour != hour) {
                return Err(dashboard_core::invalid(
                    "Casualty state reads must share an hour",
                ));
            }
            let mut tracks = Vec::new();
            let mut observed = Vec::new();
            let top: HashSet<_> = match selection.top.as_ref() {
                Some(top) => {
                    let mut top_keys = HashSet::new();
                    for rank in 0..(top.count as usize).min(tags.len()) {
                        if let Some(run) = ranks.value_at(rank as u64 * 2, hour)? {
                            top_keys.insert(run.value as u64);
                        }
                    }
                    top_keys
                }
                None => wanted.iter().map(|(_, key)| *key).collect(),
            };
            let mut identity_starts = BTreeMap::new();
            for (tag, key) in wanted {
                check_cancel(cancelled)?;
                if !top.contains(&key) {
                    continue;
                }
                let Some((event_hour, event)) = observations.latest_event(key, hour)? else {
                    continue;
                };
                let observation: Observation = serde_json::from_value(event)?;
                if !selection.tags.is_empty() && !selected.contains(&observation.identity) {
                    continue;
                }
                let Some(span) = coverage.value_at(0, event_hour)? else {
                    continue;
                };
                // A closed actor still witnesses its supplying observation;
                // only its reached-hour value depends on current coverage.
                observed.push(HourSpan {
                    first_hour: span.first_hour,
                    last_hour: span.last_hour,
                });
                let Some(endpoint) = observations.value_at(key, hour)? else {
                    continue;
                };
                let mut reached = Vec::new();
                for reader in &mut values {
                    reached.push(reader.value_at(key, event_hour)?);
                }
                let (Some(dealt), Some(taken)) = (reached[0], reached[1]) else {
                    continue;
                };
                let last = dealt
                    .last_hour
                    .min(taken.last_hour)
                    .min(endpoint.last_hour)
                    .min(span.last_hour);
                let sample = observation.sample(
                    dealt.value,
                    taken.value,
                    reached[2].map(|run| run.value as i64),
                    last,
                );
                let track = Track {
                    tail: Some(sample),
                    ..Default::default()
                };
                identity_starts.insert(
                    (
                        tag.clone(),
                        format!("{}:{}", observation.identity.0, observation.identity.1),
                    ),
                    observation.first,
                );
                tracks.push((tag.clone(), track));
            }
            tracks.sort_by(|left, right| {
                left.0.cmp(&right.0).then_with(|| {
                    left.1
                        .tail
                        .as_ref()
                        .unwrap()
                        .first
                        .cmp(&right.1.tail.as_ref().unwrap().first)
                })
            });
            let mut grouped: BTreeMap<String, Track> = BTreeMap::new();
            for (tag, track) in tracks {
                let output = grouped.entry(tag).or_default();
                output.flush();
                output.tail = track.tail;
            }
            let tracks = grouped.into_iter().collect();
            if let Some(span) = coverage.value_at(0, hour)? {
                observed.push(HourSpan {
                    first_hour: span.first_hour,
                    last_hour: span.last_hour,
                });
            }
            observed.sort_by_key(|span| span.first_hour);
            let mut merged = Vec::new();
            for span in observed {
                dashboard_base::products::country_groups::mark_unobserved(
                    &mut merged,
                    span.first_hour,
                    span.last_hour,
                );
            }
            let mut warnings = catalogue.warnings;
            let sampled: HashSet<_> = catalogue
                .actors
                .values()
                .map(|(_, identity)| *identity)
                .collect();
            for tag in &selection.tags {
                if !catalogue.holders.get(tag).is_some_and(|holders| {
                    holders.iter().any(|identity| sampled.contains(identity))
                }) {
                    warnings.push(format!("{tag}: no sampled casualties in this branch"));
                }
            }
            let scanned = ScannedCasualties {
                tracks,
                observed: merged,
                incomplete: catalogue.incomplete,
                windows: Vec::new(),
                #[cfg(any(test, feature = "test-support"))]
                names: Vec::new(),
                holders: catalogue.holders,
                #[cfg(any(test, feature = "test-support"))]
                pairs: pairs::PairHistory::default(),
            };
            // The selected point tracks already have the prepared top membership.
            let chosen = CasualtyParams {
                branch_id: selection.branch_id,
                tags: Vec::new(),
                top: None,
            };
            let mut reply = scanned.reply(
                &chosen,
                cancelled,
                Some(&catalogue.labels),
                &catalogue.unknown_label,
                warnings,
            )?;
            for series in &mut reply.series {
                if let Some(identity) = series.identities.first_mut() {
                    identity.first_hour =
                        identity_starts[&(series.tag.clone(), identity.id.clone())];
                }
            }
            serde_json::to_value(reply).map_err(io::Error::from)
        }
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(from, to, width, wanted.len())?;
            let mut lines = Vec::new();
            for (tag, key) in wanted {
                if let Some(top) = &selection.top {
                    let ranking = ranks.summary(
                        key * 2 + 1,
                        (from.floor() as i32, to.ceil() as i32),
                        (from, to, 1),
                        0,
                    )?;
                    if !ranking
                        .buckets
                        .iter()
                        .flatten()
                        .any(|bucket| bucket.min_value <= f64::from(top.count))
                    {
                        continue;
                    }
                }
                let mut summary = values[channel].summary(
                    key,
                    (from.floor() as i32, to.ceil() as i32),
                    (from, to, width),
                    0,
                )?;
                if let Some(top) = &selection.top {
                    let ranking = ranks.summary(
                        key * 2 + 1,
                        (from.floor() as i32, to.ceil() as i32),
                        (from, to, width),
                        0,
                    )?;
                    for (bucket, rank) in summary.buckets.iter_mut().zip(ranking.buckets) {
                        if rank.is_none_or(|rank| rank.min_value > f64::from(top.count)) {
                            *bucket = None;
                        }
                    }
                    summary.runs = None;
                }
                lines.push(json!({"tag": tag, "actor": key.to_string(), "summary": summary}));
            }
            Ok(json!({"series": lines, "warnings": catalogue.warnings}))
        }
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(raw)?;
            let keys = wanted.iter().map(|(_, key)| (*key, None)).collect();
            let rank_keys = selection.top.as_ref().map_or_else(Vec::new, |top| {
                (0..(top.count as usize).min(tags.len()))
                    .map(|rank| (rank as u64 * 2, None))
                    .collect()
            });
            let hour = series::navigate(
                &mut [(&mut values[channel], keys), (&mut ranks, rank_keys)],
                &navigation,
            )?;
            Ok(json!({"hour": hour}))
        }
        "events-page" => {
            let (from, to, cursor, size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                params.page_size,
            )?;
            let page = observations.events(
                &wanted.iter().map(|(_, key)| *key).collect::<Vec<_>>(),
                from,
                to,
                cursor,
                size,
            )?;
            Ok(
                json!({"rows": page.rows.into_iter().map(|row| json!({"key": row.key.to_string(), "hour": row.hour, "ordinal": row.ordinal.to_string(), "row": row.row})).collect::<Vec<_>>(),
                "firstOrdinal": page.first.to_string(), "total": page.total.to_string(), "cursor": page.cursor.map(|cursor| cursor.to_string()), "warnings": catalogue.warnings}),
            )
        }
        "events-clusters" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            let mut lines = Vec::new();
            for (tag, key) in wanted {
                lines.push(json!({"tag": tag, "summary": observations.event_clusters(key, from, to, width)?}));
            }
            Ok(json!({"series": lines, "warnings": catalogue.warnings}))
        }
        _ => Err(dashboard_core::invalid("Unknown casualty series query")),
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Prepared {
    /// Observes one frame with its own change tracker, configuring the
    /// presentation from the source first, as the pipeline would.
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        source: &Source,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if !self.configured {
            self.configure(&prepared_content(source, context)?);
        }
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay.format);
        self.observe_changes(frame, replay, cancelled, &changes)
    }
}

#[cfg(test)]
mod tests;
