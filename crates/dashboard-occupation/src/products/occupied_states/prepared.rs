//! Replay-block occupation replies and occupier series from the same observation.
use super::*;
use crate::products::occupation_trend::{
    self, History, OccupationMeasure, OccupationTop, OccupationTrendParams, OccupationTrendRun,
};
use dashboard_base::prepared::series::{self, Accumulator, Reader, Run};
use dashboard_text::metadata::check_cancel;
use serde_json::{Value, json};

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: scan::Scan<'static>,
    tags: BTreeMap<String, u64>,
    values: [Accumulator; 2],
    ranks: [Accumulator; 2],
    facts: Accumulator,
    first: Option<i32>,
    last: i32,
    next_window: usize,
    /// The content refusals this family's own reads produced, which its
    /// published warnings report.
    #[serde(default)]
    refusals: catalogue::OwnRefusals,
    #[serde(skip)]
    catalogue: Option<Arc<Catalogue>>,
    #[serde(skip)]
    bound: bool,
}

mod first_view;

struct WindowObservation {
    states: OccupiedStatesReply,
    trend: History,
    identities: dashboard_base::products::country_history::RecordedHistories,
}

/// One block's occupied-states window page and the manifest after it,
/// encoded as [`series::write_reply`] writes them, for a writer that commits
/// them after the block ([`Prepared::hold_block`]).
pub struct HeldWindow {
    branch: u64,
    window: u64,
    page: Vec<u8>,
    manifest: Vec<u8>,
}

impl HeldWindow {
    /// Write the window page, then the manifest, where
    /// [`Prepared::finish_block`] writes them.
    pub fn write(&self, writer: &mut store::Writer) -> io::Result<()> {
        writer.write_at(
            &format!("occupied-states-{}", self.branch),
            self.window,
            self.page.as_slice(),
        )?;
        writer.write_at(
            &format!("occupied-states-manifest-{}", self.branch),
            0,
            self.manifest.as_slice(),
        )
    }

    /// The bytes held.
    pub fn bytes(&self) -> usize {
        self.page.len() + self.manifest.len()
    }
}

/// A reply as [`series::write_reply`] stores it.
fn encoded_reply(value: &impl Serialize) -> io::Result<Vec<u8>> {
    series::compress(serde_json::to_vec(value)?.as_slice(), 3)
}

impl Prepared {
    pub fn configure(&mut self, source: &Source, context: &LoadContext) {
        if !self.bound {
            let catalogue = context
                .content()
                .and_then(|options| catalogue::catalogue(source, options));
            if let Err(error) = &catalogue {
                let warning =
                    format!("State names and occupation law artwork unavailable: {error}");
                if !self.scan.result.warnings.contains(&warning) {
                    self.scan.result.warnings.push(warning);
                }
            }
            self.catalogue = catalogue.ok();
            self.bound = true;
        }
        let scan = std::mem::take(&mut self.scan).bind(self.catalogue.as_deref());
        self.scan = scan.bind(None);
    }

    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        self.observe_frame(frame, format, Some(changes))
    }

    /// Runs `run` as this family's own consumer of the shared content
    /// ([`catalogue::own_refusals`]).
    fn own<T>(&mut self, run: impl FnOnce(&mut Self) -> T) -> T {
        let mut own = std::mem::take(&mut self.refusals);
        let value = catalogue::own_refusals(&mut own, || run(self));
        self.refusals = own;
        value
    }

    fn observe_frame(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        self.own(|state| state.observe_own_frame(frame, format, changes))
    }

    fn observe_own_frame(
        &mut self,
        frame: &DecodedFrame,
        format: replay::format::FormatVersion,
        changes: Option<&dashboard_base::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let _cpu = ::replay::cpu::scope_phase("occupation:observe");
        for values in &mut self.values {
            values.advance(frame.hour)?;
        }
        self.facts.advance(frame.hour)?;
        let mut scan = std::mem::take(&mut self.scan).bind(self.catalogue.as_deref());
        scan.format = format;
        let observed = scan.push(frame, frame.hour, changes);
        self.scan = scan.bind(None);
        observed?;
        self.first.get_or_insert(frame.hour);
        self.last = frame.hour;
        for tag in &self.scan.trend.changed {
            #[cfg(test)]
            {
                self.scan.work.history_visits += 1;
            }
            let track = &self.scan.trend.tracks[tag];
            let next = self.tags.len() as u64;
            let actor = *self.tags.entry(tag.clone()).or_insert(next);
            let current = track.runs.last().filter(|run| run.last_hour == frame.hour);
            for (column, values) in self.values.iter_mut().enumerate() {
                if let Some(run) = current {
                    let raw = if column == 0 {
                        &run.resistance_raw
                    } else {
                        &run.compliance_raw
                    };
                    let raw = raw
                        .parse()
                        .map_err(|_| dashboard_core::invalid("Invalid occupation fixed5 value"))?;
                    let starts = run.first_hour == frame.hour && run.starts_segment;
                    let boundary = run.read_hour == frame.hour;
                    if starts
                        || boundary
                        || values
                            .state(actor)
                            .and_then(|state| state.last)
                            .map(|run| run.value)
                            != Some(raw)
                    {
                        values.observe_boundary(actor, frame.hour, Some(raw), starts, boundary);
                        #[cfg(test)]
                        {
                            self.scan.work.emitted += 1;
                        }
                    }
                } else {
                    values.observe(actor, frame.hour, None, false);
                }
            }
            if let Some(run) = current {
                if run.first_hour == frame.hour {
                    self.facts.event(actor, frame.hour, run)?;
                    #[cfg(test)]
                    {
                        self.scan.work.emitted += 1;
                    }
                }
                self.facts.observe_if_changed(
                    actor,
                    frame.hour,
                    Some(i128::from(run.first_hour)),
                    run.first_hour == frame.hour && run.starts_segment,
                    false,
                );
            } else {
                self.facts
                    .observe_if_changed(actor, frame.hour, None, false, false);
            }
        }
        Ok(())
    }

    pub fn finish_block(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        block: usize,
    ) -> io::Result<()> {
        let index = &source.replay_ref().index[block];
        self.write_window(
            writer,
            index.branch,
            self.next_window,
            HourSpan {
                first_hour: index.first_hour,
                last_hour: index.last_hour,
            },
        )
    }

    /// The pages [`Self::finish_block`] writes at the end of a block of
    /// `branch` spanning `first_hour..=last_hour`, held to be written at a
    /// later commit ([`HeldWindow::write`]). The window advances now, as
    /// [`Self::finish_block`] advances it.
    pub fn hold_block(
        &mut self,
        branch: u64,
        first_hour: i32,
        last_hour: i32,
    ) -> io::Result<HeldWindow> {
        self.hold_window(
            branch,
            self.next_window,
            HourSpan {
                first_hour,
                last_hour,
            },
        )
    }

    /// Whether this occupation's law eligibility can read the trigger world,
    /// once configured: its frames then need the shared trigger tables
    /// ([`dashboard_text::triggers::RECORDED_TABLES`]).
    pub fn reads_trigger_world(&self) -> bool {
        self.scan.reads_trigger_world()
    }

    fn write_window(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        window: usize,
        interval: HourSpan,
    ) -> io::Result<()> {
        self.hold_window(branch, window, interval)?.write(writer)
    }

    fn hold_window(
        &mut self,
        branch: u64,
        window: usize,
        interval: HourSpan,
    ) -> io::Result<HeldWindow> {
        self.own(|state| state.hold_own_window(branch, window, interval))
    }

    /// The window of `interval` as the scan holds it now, with the scan's
    /// states, laws and warnings it is cut from.
    fn current_window(
        &mut self,
        branch: u64,
        window: usize,
        interval: HourSpan,
    ) -> (OccupiedStatesReply, OccupiedScan) {
        self.scan.seal();
        let scan = std::mem::take(&mut self.scan).bind(self.catalogue.as_deref());
        let snapshot = scan.snapshot();
        let reply = window_reply(&snapshot.states, &branch.to_string(), window, interval);
        self.scan = scan.bind(None);
        (reply, snapshot.states)
    }

    fn hold_own_window(
        &mut self,
        branch: u64,
        window: usize,
        interval: HourSpan,
    ) -> io::Result<HeldWindow> {
        let (reply, states) = self.current_window(branch, window, interval);
        let page = encoded_reply(&reply)?;
        let manifest = encoded_reply(&OccupiedStatesManifestReply {
            branch_id: branch.to_string(),
            decimals: 5,
            windows: Vec::new(),
            laws: states.laws,
            strength_missing: STRENGTH_MISSING.into(),
            warnings: states.warnings,
        })?;
        self.scan.finish_window();
        self.next_window = window + 1;
        Ok(HeldWindow {
            branch,
            window: window as u64,
            page,
            manifest,
        })
    }

    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .map(Accumulator::bytes)
            .sum::<usize>()
            + self.facts.bytes()
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
        let history = self.scan.trend.history(None);
        let rows: BTreeMap<_, _> = history
            .series
            .iter()
            .map(|(tag, runs)| {
                (
                    tag.clone(),
                    runs.iter()
                        .filter(|run| run.last_hour >= first)
                        .map(|run| OccupationTrendRun {
                            first_hour: run.first_hour.max(first),
                            ..run.clone()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        for column in 0..2 {
            self.ranks[column].through(first);
            dashboard_base::ranking::rank_positions(
                &rows,
                |run| (run.first_hour, run.last_hour),
                |run| {
                    (if column == 0 {
                        &run.resistance_raw
                    } else {
                        &run.compliance_raw
                    })
                    .parse::<i64>()
                    .ok()
                    .map(i128::from)
                },
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
            self.ranks[column].flush(
                writer,
                &format!("occupation-trend-ranks-{branch}-{column}"),
                0,
                cancelled,
            )?;
            self.values[column].flush(
                writer,
                &format!("occupation-trend-values-{branch}-{column}"),
                5,
                cancelled,
            )?;
        }
        self.facts.flush(
            writer,
            &format!("occupation-trend-facts-{branch}"),
            0,
            cancelled,
        )?;
        series::write_catalogue(
            writer,
            "occupation-trend",
            branch,
            &self.tags,
            &(history.warnings, history.collaboration_warnings),
        )?;
        for track in self.scan.trend.tracks.values_mut() {
            if track.runs.len() > 1 {
                track.runs.drain(..track.runs.len() - 1);
            }
        }
        Ok(())
    }

    pub fn saved_bytes(&self) -> io::Result<Vec<u8>> {
        dashboard_base::prepared::series::compress(serde_json::to_vec(self)?.as_slice(), 3)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::read::Decoder::new(
            reader,
        )?)?)
    }
}

/// The tables the observation reads: those of
/// [`scan::scan_history`] and, when a law's eligibility can read the trigger
/// world, the shared trigger kit.
fn observed_tables(world: bool) -> Vec<TableId> {
    let mut tables = vec![
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::StateResistance,
        TableId::StateGarrisons,
        TableId::StateOccupationLaws,
        TableId::OccupationSettings,
        TableId::StateAssignedManpower,
        TableId::CountryCollaboration,
        TableId::LexicalTokens,
    ];
    if world {
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    }
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// What observing a window's look-back found.
enum LookBack {
    /// The window as hourly preparation writes it.
    Window(Box<WindowObservation>),
    /// At the window's first hour the fresh observation may lack state the
    /// branch's observation carries: look further back.
    Unanchored,
    /// A block or hour could not be read: the window waits for preparation.
    Unreadable,
}

/// Window `window` of `branch` exactly as hourly preparation writes it, read
/// before preparation reaches it from the window's block and the blocks just
/// before it alone. A window lists each state where its values change,
/// restating every value where a span starts, so it decodes alone; its first
/// observation can hold the latest daily read before it. A fresh observation
/// holds what the branch's holds once it has seen a supplying read, or has
/// dropped its carried state (a gap, an unreadable hour, an hour without
/// resistance rows) as the branch's does there too; from the branch's first
/// block it is the branch's observation. So the look-back reads the block
/// before the window, then twice as many, until the observation is anchored
/// at the window's first hour. The laws the window's states hold come with
/// it. None when a block cannot be read.
pub fn look_back_window(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    window: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<OccupiedStatesReply>> {
    Ok(observed_window(source, context, branch, window, cancelled)?
        .map(|observation| observation.states.clone()))
}

fn observed_window(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    window: usize,
    cancelled: &AtomicBool,
) -> io::Result<Option<Arc<WindowObservation>>> {
    check_cancel(cancelled)?;
    let (_, binding) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let cache = dashboard_base::prepared::query::observations::scope(
        source,
        "occupation:window",
        Some(&binding),
    );
    let replay = source.replay_ref();
    let mut blocks: Vec<&replay::container::BlockIndex> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    if window >= blocks.len() {
        return Err(dashboard_core::invalid(
            "Occupied states window not in this recording",
        ));
    }
    let observation = cache.get(
        blocks[window],
        &window,
        cancelled,
        |observation: &Option<Arc<WindowObservation>>| {
            std::mem::size_of::<Option<Arc<WindowObservation>>>()
                + observation
                    .as_ref()
                    .map_or(0, |observation| first_view::window_bytes(observation))
        },
        || {
            let mut back = 1_usize;
            loop {
                let start = window.saturating_sub(back);
                match observe_window(
                    source,
                    context,
                    branch,
                    window,
                    &blocks[start..=window],
                    start == 0,
                    cancelled,
                )? {
                    LookBack::Window(reply) => return Ok(Some(Arc::new(*reply))),
                    LookBack::Unreadable => return Ok(None),
                    LookBack::Unanchored if start == 0 => {
                        return Err(dashboard_core::invalid(
                            "An occupation look-back from the branch's first block is anchored",
                        ));
                    }
                    LookBack::Unanchored => back = back.saturating_mul(2),
                }
            }
        },
    )?;
    Ok(observation.as_ref().clone())
}

fn look_back_read_ahead(blocks: usize, workers: usize) -> usize {
    blocks.saturating_sub(1).min(workers.max(1))
}

/// Observe `blocks` (ending with window `window`'s) on a fresh observation,
/// read ahead together, and cut the last one's window when the observation
/// is anchored at its first hour ([`look_back_window`]).
fn observe_window(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    window: usize,
    blocks: &[&replay::container::BlockIndex],
    from_start: bool,
    cancelled: &AtomicBool,
) -> io::Result<LookBack> {
    dashboard_base::prepared::query::progressive::decodes_allowed()?;
    let replay = source.replay_ref();
    let mut prepared = Prepared::default();
    let mut identities = dashboard_base::products::country_history::HistoryScan::new(false, &[]);
    prepared.configure(source, context);
    let tables = observed_tables(prepared.reads_trigger_world());
    let consumer = replay.section_consumer(
        blocks
            .first()
            .and_then(|block| replay.block_position(block))
            .unwrap_or(0),
    );
    let read = |block: &replay::container::BlockIndex| {
        if let Some(position) = replay.block_position(block) {
            consumer.reading(position);
        }
        replay.read_block_recorded_selected(block, &tables)
    };
    let count = blocks.len();
    dashboard_core::blocks::scan_blocks_by(
        blocks.to_vec(),
        look_back_read_ahead(
            count,
            std::thread::available_parallelism().map_or(1, |threads| (threads.get() / 2).max(1)),
        ),
        cancelled,
        &read,
        |reads| {
            let mut anchored = from_start;
            for (position, (block, frames)) in reads.enumerate() {
                if let Some(at) = replay.block_position(block) {
                    consumer.at(at);
                }
                dashboard_base::prepared::prepass::note("occupation.look-back");
                let target = position + 1 == count;
                let Ok(frames) = frames else {
                    return Ok(LookBack::Unreadable);
                };
                // Before the window's first hour: a read held, or a drop seen.
                anchored |= target && (prepared.scan.holds_read() || prepared.scan.resets > 0);
                let resets = prepared.scan.resets;
                let mut decoder = replay::frame::FrameDecoder::for_tables(&tables);
                for (index, frame) in frames.iter().enumerate() {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(frame) else {
                        return Ok(LookBack::Unreadable);
                    };
                    prepared.observe_frame(decoded, replay.format, None)?;
                    if target {
                        identities.observe(decoded);
                    }
                    // A drop at the window's first hour (a gap before it)
                    // anchors it too.
                    anchored |= target && index == 0 && prepared.scan.resets > resets;
                }
                if !target {
                    prepared.scan.seal();
                    prepared.scan.finish_window();
                    continue;
                }
                if !anchored {
                    return Ok(LookBack::Unanchored);
                }
                let interval = HourSpan {
                    first_hour: block.first_hour,
                    last_hour: block.last_hour,
                };
                let (mut reply, states) = prepared.current_window(branch, window, interval);
                let held: std::collections::BTreeSet<&str> = reply
                    .states
                    .iter()
                    .flat_map(|series| series.law.iter())
                    .filter_map(|&(_, index)| index)
                    .filter_map(|index| reply.texts.get(index).map(String::as_str))
                    .collect();
                let laws = states
                    .laws
                    .into_iter()
                    .filter(|law| held.contains(law.token.as_str()))
                    .collect();
                reply.laws = Some(laws);
                let trend = prepared.scan.trend.history(Some(interval.last_hour));
                return Ok(LookBack::Window(Box::new(WindowObservation {
                    states: reply,
                    trend,
                    identities: std::mem::take(&mut identities).finish(),
                })));
            }
            Ok(LookBack::Unreadable)
        },
    )
}

pub fn window_query(
    source: &Source,
    context: &LoadContext,
    product: &str,
    value: Value,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    check_cancel(cancelled)?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    if product == "occupied-states" {
        let params: OccupiedStatesParams = serde_json::from_value(value)?;
        let branch = params
            .branch_id
            .parse()
            .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let _interval = source
            .block_windows(branch)
            .get(params.window)
            .cloned()
            .ok_or_else(|| {
                dashboard_core::invalid("Occupied states window not in this recording")
            })?;
        return Ok(serde_json::to_value(series::read_reply::<
            OccupiedStatesReply,
        >(
            &store,
            &key,
            &format!("occupied-states-{branch}"),
            params.window as u64,
        )?)?);
    }
    let params: BranchParams = serde_json::from_value(value)?;
    params.validate().map_err(dashboard_core::invalid)?;
    let branch = params.branch();
    // The manifest each published window wrote last: its laws and warnings
    // are those of the windows through it. Before the first, the windows
    // alone, which are the branch's blocks.
    let mut reply = series::read_reply::<OccupiedStatesManifestReply>(
        &store,
        &key,
        &format!("occupied-states-manifest-{branch}"),
        0,
    )?
    .unwrap_or_else(|| OccupiedStatesManifestReply {
        branch_id: branch.to_string(),
        decimals: 5,
        windows: Vec::new(),
        laws: Vec::new(),
        strength_missing: STRENGTH_MISSING.into(),
        warnings: Vec::new(),
    });
    reply.windows = source.block_windows(branch).to_vec();
    Ok(serde_json::to_value(reply)?)
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
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Family {
    measure: OccupationMeasure,
    rank_measure: OccupationMeasure,
    #[serde(default)]
    both: bool,
    top: Option<u32>,
    #[serde(default)]
    collaboration: bool,
}
#[derive(Deserialize)]
struct Read {
    hour: i32,
}

/// Composite facts retain supplying and collaboration boundaries inside an exact window.
fn fact_runs(
    reader: &mut Reader<'_>,
    key: u64,
    from: i32,
    to: i32,
) -> io::Result<Vec<OccupationTrendRun>> {
    reader
        .runs(key, from, to)?
        .into_iter()
        .map(|marker| {
            let (_, event) = reader
                .latest_event(key, marker.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Occupation facts are missing"))?;
            let mut run: OccupationTrendRun = serde_json::from_value(event)?;
            run.first_hour = marker.first_hour;
            run.last_hour = marker.last_hour;
            run.starts_segment &= marker.value == i128::from(marker.first_hour);
            Ok(run)
        })
        .collect()
}

pub fn trend_query(
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
        .map_err(|_| dashboard_core::invalid("Invalid occupation branch"))?;
    let selection = OccupationTrendParams {
        branch_id: params.branch_id.clone(),
        tags: params.tags.clone(),
        top: params.family.top.map(|count| OccupationTop {
            count,
            measure: params.family.rank_measure,
        }),
        collaboration: params.family.collaboration,
    };
    occupation_trend::OccupationTrend::validate(&selection).map_err(dashboard_core::invalid)?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let coverage = dashboard_base::prepared::coverage::product_coverage(
        source,
        context.content.as_ref(),
        branch,
        "occupation-trend",
        cancelled,
    )?;
    let tail = first_view::tail(
        source,
        context,
        product,
        &params,
        coverage.as_ref(),
        cancelled,
    )?;
    let (mut tags, (mut warnings, collaboration_warnings)) =
        series::read_catalogue::<(Vec<String>, Vec<String>)>(
            &store,
            &key,
            "occupation-trend",
            branch,
        )?
        .unwrap_or_default();
    if params.family.collaboration {
        warnings.extend(collaboration_warnings);
    }
    let complete_histories = dashboard_base::products::country_history::prepared_branch_histories(
        source,
        branch,
        context.content.as_ref(),
        cancelled,
    )?;
    let identities_preparing = complete_histories.is_none();
    let histories = match complete_histories {
        Some(history) => history,
        None => {
            let accepted = dashboard_base::prepared::coverage::product_coverage(
                source,
                context.content.as_ref(),
                branch,
                "country-identities",
                cancelled,
            )?;
            let none = BTreeMap::new();
            let (identities, exact) = if product == "series-state" {
                (&none, &tail.identities)
            } else {
                (&tail.identities, &none)
            };
            Arc::new(dashboard_base::prepared::query::progressive::histories(
                &store,
                &key,
                branch,
                accepted.as_ref(),
                identities,
                exact,
                source.replay_ref(),
                &params.tags,
                cancelled,
            )?)
        }
    };
    for tag in tail.history.series.keys() {
        let next = tags.len() as u64;
        tags.entry(tag.clone()).or_insert(next);
    }
    let tail_membership = params
        .family
        .top
        .map(|count| {
            dashboard_base::ranking::top_membership(
                &tail.history.series,
                count as usize,
                |run| (run.first_hour, run.last_hour),
                |run| {
                    (match params.family.rank_measure {
                        OccupationMeasure::Resistance => &run.resistance_raw,
                        OccupationMeasure::Compliance => &run.compliance_raw,
                    })
                    .parse::<i64>()
                    .ok()
                },
                cancelled,
            )
        })
        .transpose()?;
    let selected: Vec<String> = if params.family.top.is_some() || params.tags.is_empty() {
        tags.keys().cloned().collect()
    } else {
        dashboard_base::products::country_history::selected_aliases(&histories, &params.tags)
    };
    let selected_ids = (!params.tags.is_empty() && params.family.top.is_none()).then(|| {
        dashboard_base::products::country_history::selected_identities(&histories, &params.tags)
    });
    let mut within = BTreeMap::new();
    for tag in &selected {
        if let Some(selected_ids) = &selected_ids {
            let identities = histories
                .countries
                .get(tag)
                .into_iter()
                .flat_map(|country| country.identities.iter())
                .filter(|identity| selected_ids.contains(&(identity.slot, identity.country_id)))
                .collect::<Vec<_>>();
            let spans: Vec<_> = identities
                .iter()
                .map(|identity| HourSpan {
                    first_hour: identity.first_hour,
                    last_hour: identity.last_hour,
                })
                .collect();
            if params.within.is_empty() {
                within.insert(tag.clone(), spans);
            } else {
                let mut clipped = Vec::new();
                dashboard_base::ranking::for_each_piece(
                    &spans,
                    params.within.get(tag).map_or(&[], Vec::as_slice),
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
                within.insert(tag.clone(), clipped);
            }
        } else if let Some(spans) = params.within.get(tag) {
            within.insert(tag.clone(), spans.clone());
        }
    }
    let column = match params.family.measure {
        OccupationMeasure::Resistance => 0,
        OccupationMeasure::Compliance => 1,
    };
    let mut values = Reader::new(
        &store,
        &key,
        format!("occupation-trend-values-{branch}-{column}"),
        cancelled,
    )?;
    let rank_column = match params.family.rank_measure {
        OccupationMeasure::Resistance => 0,
        OccupationMeasure::Compliance => 1,
    };
    let mut ranks = Reader::new(
        &store,
        &key,
        format!("occupation-trend-ranks-{branch}-{rank_column}"),
        cancelled,
    )?;
    let mut data = match product {
        "series-summary" => {
            let view = series::viewport(params.from, params.to, params.width)?;
            let bounds = (view.0.floor() as i32, view.1.ceil() as i32);
            let mut lines = Vec::new();
            let mut facts = Reader::new(
                &store,
                &key,
                format!("occupation-trend-facts-{branch}"),
                cancelled,
            )?;
            for tag in selected {
                check_cancel(cancelled)?;
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                let mut spans = within.get(&tag).cloned();
                let numeric = first_view::numeric(&tail.history, &tag, column);
                let mut summary = values.summary_with_tail(
                    actor,
                    bounds,
                    view,
                    5,
                    &numeric,
                    within.get(&tag).map(Vec::as_slice),
                )?;
                if let Some(top) = params.family.top {
                    let rank_tail: Vec<Run> = tail_membership
                        .as_ref()
                        .and_then(|members| members.get(&tag))
                        .into_iter()
                        .flatten()
                        .map(|span| Run::new(span.first_hour, span.last_hour, 1, true))
                        .collect();
                    let membership = ranks.summary_with_tail(
                        actor * 2 + 1,
                        bounds,
                        view,
                        0,
                        &rank_tail,
                        None,
                    )?;
                    if !membership
                        .buckets
                        .iter()
                        .flatten()
                        .any(|bucket| bucket.min_value <= f64::from(top))
                    {
                        continue;
                    }
                    if let Some(exact) = membership.runs {
                        let ranked: Vec<_> = exact
                            .iter()
                            .filter(|run| run.value <= f64::from(top))
                            .map(|run| HourSpan {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                            })
                            .collect();
                        spans = Some(ranked);
                        summary = values.summary_with_tail(
                            actor,
                            bounds,
                            view,
                            5,
                            &numeric,
                            spans.as_deref(),
                        )?;
                    } else {
                        for (bucket, rank) in summary.buckets.iter_mut().zip(membership.buckets) {
                            if rank.is_none_or(|rank| rank.min_value > f64::from(top)) {
                                *bucket = None;
                            }
                        }
                        summary.runs = None;
                    }
                }
                if summary.runs.is_some() && tail.approximate.is_empty() {
                    let mut raw = fact_runs(&mut facts, actor, bounds.0, view.1.floor() as i32)?;
                    for run in &mut raw {
                        let canonical =
                            facts.value_at(actor, run.first_hour)?.ok_or_else(|| {
                                dashboard_core::invalid("Occupation supplying run is missing")
                            })?;
                        run.first_hour = canonical.first_hour;
                        run.last_hour = canonical.last_hour;
                        run.starts_segment = canonical.break_before;
                    }
                    let mut clipped = Vec::new();
                    dashboard_base::ranking::for_each_optional_piece(
                        &raw,
                        spans.as_deref(),
                        |run| (run.first_hour, run.last_hour),
                        cancelled,
                        |run, first_hour, last_hour| {
                            clipped.push(OccupationTrendRun {
                                first_hour,
                                last_hour,
                                starts_segment: run.starts_segment && run.first_hour == first_hour,
                                ..run.clone()
                            });
                            Ok(())
                        },
                    )?;
                    let history = History {
                        series: BTreeMap::from([(tag.clone(), clipped)]),
                        ..Default::default()
                    };
                    let reply = occupation_trend::reply_with_histories(
                        &params.branch_id,
                        &history,
                        &OccupationTrendParams {
                            branch_id: params.branch_id.clone(),
                            tags: vec![tag.clone()],
                            top: None,
                            collaboration: params.family.collaboration,
                        },
                        Vec::new(),
                        cancelled,
                        None,
                    )?;
                    summary.runs = Some(
                        reply
                            .series
                            .into_iter()
                            .flat_map(|line| line.runs)
                            .map(|run| dashboard_base::prepared_chart::ChartRun {
                                first_hour: run.first_hour,
                                last_hour: run.last_hour,
                                value: (if column == 0 {
                                    run.resistance_raw
                                } else {
                                    run.compliance_raw
                                })
                                .parse::<i64>()
                                .expect("validated fixed5")
                                    as f64
                                    / 100_000.0,
                                break_before: run.starts_segment,
                            })
                            .collect(),
                    );
                }
                dashboard_base::prepared::query::progressive::clip_approximate(
                    &mut summary,
                    &tail.approximate,
                    spans.as_deref(),
                );
                series::summary_budget(view.0, view.1, view.2, lines.len() + 1)?;
                let end_run = if let Some(end) = summary
                    .buckets
                    .iter()
                    .rev()
                    .flatten()
                    .next()
                    .map(|bucket| bucket.last_hour)
                {
                    if facts.value_at(actor, end)?.is_some() {
                        let (_, event) = facts.latest_event(actor, end)?.ok_or_else(|| {
                            dashboard_core::invalid("Occupation endpoint facts are missing")
                        })?;
                        let mut run: OccupationTrendRun = serde_json::from_value(event)?;
                        run.first_hour = end;
                        run.last_hour = end;
                        run.starts_segment = false;
                        if !params.family.collaboration {
                            run.collaboration.clear();
                        }
                        Some(run)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let country = histories.countries.get(&tag);
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
                lines.push(json!({"tag":tag,"summary":summary,"marks":null,"endRun":end_run,"segments":within.get(&tag).or_else(||country.map(|country|country.segments.as_ref())),"identities":identities}));
            }
            Ok(
                json!({"branchId":params.branch_id,"measure":params.family.measure,"decimals":5,"series":lines,"warnings":warnings}),
            )
        }
        "series-next-change" => {
            let navigation: series::Navigation = serde_json::from_value(value)?;
            let hour = navigation
                .origin()?
                .clamp(navigation.first_hour, navigation.last_hour)
                .floor() as i32;
            let mut keys = Vec::new();
            let mut positions = Vec::new();
            if let Some(top) = params.family.top {
                for rank in 0..u64::from(top).min(tags.len() as u64) {
                    positions.push((rank * 2, None));
                    if let Some(run) = ranks.value_at(rank * 2, hour)? {
                        keys.push((run.value as u64, None));
                    }
                }
            } else {
                keys.extend(selected.iter().filter_map(|tag| {
                    tags.get(tag)
                        .map(|&key| (key, within.get(tag).map(Vec::as_slice)))
                }));
            }
            let mut context = Reader::new(
                &store,
                &key,
                format!("occupation-trend-facts-{branch}"),
                cancelled,
            )?;
            let context_keys = if params.family.collaboration {
                keys.clone()
            } else {
                Vec::new()
            };
            let hour = if params.family.both {
                let mut other = Reader::new(
                    &store,
                    &key,
                    format!("occupation-trend-values-{branch}-{}", 1 - column),
                    cancelled,
                )?;
                series::navigate(
                    &mut [
                        (&mut values, keys.clone()),
                        (&mut other, keys),
                        (&mut ranks, positions),
                        (&mut context, context_keys),
                    ],
                    &navigation,
                )?
            } else {
                series::navigate(
                    &mut [
                        (&mut values, keys),
                        (&mut ranks, positions),
                        (&mut context, context_keys),
                    ],
                    &navigation,
                )?
            };
            Ok(json!({"hour":hour}))
        }
        "series-state" => {
            let [read] = params.reads.as_slice() else {
                return Err(dashboard_core::invalid(
                    "Occupation state needs one reached hour",
                ));
            };
            let mut facts = Reader::new(
                &store,
                &key,
                format!("occupation-trend-facts-{branch}"),
                cancelled,
            )?;
            let mut history = History::default();
            for tag in selected {
                let Some(&actor) = tags.get(&tag) else {
                    continue;
                };
                if within.get(&tag).is_some_and(|spans| {
                    !spans
                        .iter()
                        .any(|span| span.first_hour <= read.hour && span.last_hour >= read.hour)
                }) {
                    continue;
                }
                let runs = if coverage
                    .as_ref()
                    .is_none_or(|coverage| read.hour > coverage.last_hour)
                {
                    tail.history.series.get(&tag).cloned().unwrap_or_default()
                } else {
                    fact_runs(&mut facts, actor, read.hour, read.hour)?
                };
                if !runs.is_empty() {
                    history.series.insert(tag.clone(), runs);
                }

                if let Some(country) = histories.countries.get(&tag) {
                    history
                        .segments
                        .insert(tag, country.segments.as_ref().clone());
                }
            }
            Ok(serde_json::to_value(
                occupation_trend::reply_with_histories(
                    &params.branch_id,
                    &history,
                    &selection,
                    warnings,
                    cancelled,
                    Some(&histories),
                )?,
            )?)
        }
        _ => Err(dashboard_core::invalid("Unknown occupation series query")),
    }?;
    let mut prefix = dashboard_base::prepared::prefix::Prefix::new(
        source,
        branch,
        dashboard_base::prepared::prefix::extent(source, branch, coverage.clone())?,
        if product == "series-state" {
            params.reads.first().map(|read| read.hour)
        } else {
            params.to.map(|hour| hour.floor() as i32)
        },
    );
    prefix.answered(&tail.answered);
    data["pending"] = serde_json::to_value(prefix.pending)?;
    if identities_preparing {
        data["preparing"] = json!(["identities", "segments", "startsSegment"]);
    }
    data["coverage"] = serde_json::to_value(
        dashboard_base::prepared::query::progressive::identity_coverage(
            source,
            context.content.as_ref(),
            branch,
            coverage.as_ref(),
            !identities_preparing,
            cancelled,
        )?,
    )?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_occupation_fact_markers_carry_without_materializing_each_key() {
        let format = replay::format::FormatVersion::CURRENT;
        let mut input = frame(100, 64);
        let mut prepared = Prepared::default();
        prepared.observe_frame(&input, format, None).unwrap();
        let visits = prepared.facts.key_visits;
        input.hour += 1;
        input.metrics = Default::default();
        prepared.observe_frame(&input, format, None).unwrap();
        assert!(!prepared.scan.trend.changed.is_empty());
        assert_eq!(prepared.facts.key_visits, visits);
        prepared.facts.seal();
        for key in prepared.tags.values() {
            assert_eq!(
                prepared.facts.state(*key).unwrap().last.unwrap().last_hour,
                101
            );
        }
    }
    use dashboard_base::prepared::series::tests::Fixture;
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::rows::{CountryLinkRow, CountryRow, StateGarrisonRow, TableRows};

    #[test]
    fn look_back_reads_only_a_worker_bounded_number_of_blocks_ahead() {
        for workers in [1, 4, 8] {
            assert_eq!(look_back_read_ahead(1, workers), 0);
            assert_eq!(look_back_read_ahead(2, workers), 1);
            for blocks in [32, 663, 5000] {
                assert_eq!(look_back_read_ahead(blocks, workers), workers);
            }
        }
    }

    fn frame(hour: i32, count: u32) -> DecodedFrame {
        let mut frame = dashboard_base::test_fixtures::source(1, hour, Some(1));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            (1..=count)
                .map(|id| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: format!("occupier {id:05}").into_bytes(),
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
        *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            (1..=count)
                .map(|id| StateRow {
                    id,
                    owner: id,
                    controller: id,
                    ..Default::default()
                })
                .collect(),
        ));
        *frame.tables[TableId::StateResistance.index()] =
            TableView::Complete(TableRows::StateResistance(
                (1..=count)
                    .map(|id| StateResistanceRow {
                        state: id,
                        country: id,
                        active: 1,
                        level: i64::from(id) + i64::from(hour),
                        compliance: i64::from(id),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::StateGarrisons.index()] =
            TableView::Complete(TableRows::StateGarrisons(
                (1..=count)
                    .map(|id| StateGarrisonRow {
                        state: id,
                        controller: id,
                        occupied: id,
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame.metrics.table_ns[TableId::StateResistance.index()] = 1;
        frame
    }

    #[test]
    fn occupation_configuration_binds_content_success_and_refusals_before_any_observation() {
        let format = replay::format::FormatVersion::CURRENT;
        let flag = AtomicBool::new(false);
        let (catalogue, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[(
            "common/occupation_laws/mod.txt",
            "synthetic_law = {}",
        )]);
        for content in [Some(Arc::new(catalogue)), None] {
            let read = frame(100, 2);
            let source = Source::from_replay(
                dashboard_base::source::SourceKey {
                    source_id: "occupation-configure".into(),
                    revision: "1".into(),
                },
                "occupation-configure.battleplan".into(),
                Arc::new(
                    replay::container::Replay::from_records(
                        dashboard_base::test_fixtures::CAMPAIGN.into(),
                        &[dashboard_base::test_fixtures::records(
                            1,
                            vec![read.clone()],
                        )],
                    )
                    .unwrap(),
                ),
                &flag,
            )
            .unwrap();
            let mut prepared = Prepared {
                catalogue: content,
                bound: true,
                ..Default::default()
            };
            prepared.configure(&source, &LoadContext { content: None });
            let configured = prepared.scan.work.content_loads;
            prepared
                .observe_frame(
                    &read,
                    format,
                    Some(&FrameChangeTracker::new().observe(&read, format)),
                )
                .unwrap();
            assert_eq!(prepared.scan.work.content_loads, configured);
            if prepared.catalogue.is_some() {
                assert!(configured >= 2);
            }
            let mut held = read.clone();
            held.hour = 101;
            held.metrics = Default::default();
            prepared
                .observe_frame(
                    &held,
                    format,
                    Some(&FrameChangeTracker::new().observe(&held, format)),
                )
                .unwrap();
            observer_tests::unchanged_frames(
                prepared.scan.work,
                &held,
                10,
                format,
                |frame, changes| {
                    prepared.observe_frame(frame, format, Some(changes))?;
                    Ok(prepared.scan.work)
                },
            )
            .unwrap();
        }
    }

    #[test]
    fn occupation_shared_driver_defers_held_summaries_and_keeps_window_endpoints_exact() {
        let format = replay::format::FormatVersion::CURRENT;
        let work = observer_tests::scaled_work(1, |hours| {
            let mut prepared = Prepared::default();
            let mut read = frame(100, 64);
            let mut tracker = FrameChangeTracker::new();
            prepared.observe_frame(&read, format, Some(&tracker.observe(&read, format)))?;
            read.metrics = Default::default();
            read.hour = 101; // Every compared hour lies between native daily reads.
            prepared.observe_frame(&read, format, Some(&tracker.observe(&read, format)))?;
            let delta = observer_tests::unchanged_frames(
                prepared.scan.work,
                &read,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_frame(frame, format, Some(changes))?;
                    Ok(prepared.scan.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            let mut oracle = scan::Scan::new(None);
            oracle.format = format;
            let first = frame(100, 64);
            oracle.push(&first, first.hour, None)?;
            for hour in 101..=101 + hours as i32 {
                read.hour = hour;
                oracle.push(&read, hour, None)?;
            }
            observer_tests::assert_reply_parity(
                &prepared.scan.snapshot().states,
                &oracle.snapshot().states,
            )?;
            observer_tests::assert_reply_parity(
                &prepared.scan.snapshot().trend,
                &oracle.snapshot().trend,
            )?;
            Ok(delta)
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }

    #[test]
    fn occupation_shared_driver_skips_closed_alias_history_and_bounds_one_owner_change() {
        let format = replay::format::FormatVersion::CURRENT;
        let work = observer_tests::scaled_work(10, |history| {
            let mut prepared = Prepared::default();
            let mut tracker = FrameChangeTracker::new();
            let mut read = frame(100, 2);
            for index in 0..history {
                read = frame(100 + index as i32 * 24, 2);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *read.tables[TableId::Countries.index()]
                {
                    rows[0].tag = format!("closed alias {index}").into_bytes();
                }
                prepared.observe_frame(&read, format, Some(&tracker.observe(&read, format)))?;
            }
            read.metrics = Default::default();
            read.hour += 1;
            prepared.observe_frame(&read, format, Some(&tracker.observe(&read, format)))?;
            let delta = observer_tests::unchanged_frames(
                prepared.scan.work,
                &read,
                10,
                format,
                |frame, changes| {
                    prepared.observe_frame(frame, format, Some(changes))?;
                    Ok(prepared.scan.work)
                },
            )?;
            assert_eq!(delta, observer_tests::Work::default());
            read.hour += 11;
            if let TableView::Complete(TableRows::States(rows)) =
                &mut *read.tables[TableId::States.index()]
            {
                rows[0].owner = 2;
            }
            // The shared driver owns its tracker; seed the next real observation
            // with the prior accepted views so this is a one-table change.
            let mut previous = read.clone();
            previous.hour -= 1;
            if let TableView::Complete(TableRows::States(rows)) =
                &mut *previous.tables[TableId::States.index()]
            {
                rows[0].owner = 1;
            }
            let mut tracker = FrameChangeTracker::new();
            tracker.observe(&previous, format);
            let before = prepared.scan.work;
            prepared.observe_frame(&read, format, Some(&tracker.observe(&read, format)))?;
            let changed = observer_tests::Work {
                row_visits: prepared.scan.work.row_visits - before.row_visits,
                history_visits: prepared.scan.work.history_visits - before.history_visits,
                emitted: prepared.scan.work.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(changed.row_visits, 2);
            assert_eq!(changed.history_visits, 1);
            assert_eq!(changed.emitted, 1);
            Ok(changed)
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }

    #[test]
    #[cfg(feature = "slow-tests")]
    fn occupation_shared_driver_holds_all_5000_occupiers_without_hourly_work() {
        let format = replay::format::FormatVersion::CURRENT;
        let mut prepared = Prepared::default();
        let mut read = frame(100, 5_000);
        let mut tracker = FrameChangeTracker::new();
        prepared
            .observe_frame(&read, format, Some(&tracker.observe(&read, format)))
            .unwrap();
        read.metrics = Default::default();
        read.hour = 101;
        prepared
            .observe_frame(&read, format, Some(&tracker.observe(&read, format)))
            .unwrap();
        let delta = observer_tests::unchanged_frames(
            prepared.scan.work,
            &read,
            10,
            format,
            |frame, changes| {
                prepared.observe_frame(frame, format, Some(changes))?;
                Ok(prepared.scan.work)
            },
        )
        .unwrap();
        assert_eq!(delta, observer_tests::Work::default());
        assert_eq!(prepared.tags.len(), 5_000);
        assert!(
            prepared
                .scan
                .snapshot()
                .trend
                .series
                .values()
                .all(|runs| runs.last().unwrap().last_hour == 111)
        );
    }

    #[test]
    fn occupation_prepared_windows_and_trend_match_two_commits_resume_append_gaps_and_renames() {
        let mut fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        let mut reference = scan::Scan::new(None);
        reference.format = replay::format::FormatVersion::CURRENT;
        for (window, hours) in [vec![100, 101], vec![102, 104], vec![105, 106]]
            .into_iter()
            .enumerate()
        {
            let interval = HourSpan {
                first_hour: hours[0],
                last_hour: *hours.last().unwrap(),
            };
            for hour in hours {
                let mut frame = frame(hour, 3);
                if hour >= 104
                    && let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].id = 19;
                    rows[0].tag = b"renamed occupier".to_vec();
                    rows.sort_by_key(|row| row.id);
                }
                let changes = tracker.observe(&frame, replay::format::FormatVersion::CURRENT);
                reference.push(&frame, hour, None).unwrap();
                prepared
                    .observe_frame(
                        &frame,
                        replay::format::FormatVersion::CURRENT,
                        Some(&changes),
                    )
                    .unwrap();
            }
            let mut writer = if window == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared
                .write_window(&mut writer, 1, window, interval)
                .unwrap();
            prepared.flush(&mut writer, 1, &cancelled).unwrap();
            writer
                .write_at(
                    "occupation-continuation",
                    0,
                    prepared.saved_bytes().unwrap().as_slice(),
                )
                .unwrap();
            writer.commit().unwrap();
            tracker.invalidate();
            prepared = Prepared::read(
                fixture
                    .store
                    .read(&fixture.key, "occupation-continuation", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let actual = series::read_reply::<OccupiedStatesReply>(
                &fixture.store,
                &fixture.key,
                "occupied-states-1",
                window as u64,
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(window_reply(
                    &reference.snapshot().states,
                    "1",
                    window,
                    interval
                ))
                .unwrap()
            );
            let mut facts = Reader::new(
                &fixture.store,
                &fixture.key,
                "occupation-trend-facts-1".into(),
                &cancelled,
            )
            .unwrap();
            let history = reference.trend.history(None);
            let mut rebuilt = History::default();
            for (tag, &key) in &prepared.tags {
                rebuilt
                    .series
                    .insert(tag.clone(), fact_runs(&mut facts, key, 100, 106).unwrap());
                if let Some(segments) = history.segments.get(tag) {
                    rebuilt.segments.insert(tag.clone(), segments.clone());
                }
            }
            let params = OccupationTrendParams {
                branch_id: "1".into(),
                tags: Vec::new(),
                top: None,
                collaboration: true,
            };
            let expected =
                occupation_trend::reply("1", &history, &params, Vec::new(), &cancelled).unwrap();
            let actual =
                occupation_trend::reply("1", &rebuilt, &params, Vec::new(), &cancelled).unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "trend reply after commit {window}"
            );
            let mut ranks = Reader::new(
                &fixture.store,
                &fixture.key,
                "occupation-trend-ranks-1-0".into(),
                &cancelled,
            )
            .unwrap();
            dashboard_base::ranking::rank_positions(
                &history.series,
                |run| (run.first_hour, run.last_hour),
                |run| run.resistance_raw.parse::<i64>().ok().map(i128::from),
                &cancelled,
                |rank, tag, from, to| {
                    for hour in from..=to {
                        assert_eq!(
                            ranks.value_at(rank as u64 * 2, hour)?.unwrap().value,
                            i128::from(prepared.tags[tag])
                        );
                    }
                    Ok(())
                },
            )
            .unwrap();
            for (tag, &key) in &prepared.tags {
                for hour in 100..=106 {
                    let expected = history.series.get(tag).and_then(|runs| {
                        runs.iter()
                            .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                    });
                    let actual = facts.value_at(key, hour).unwrap().map(|_| {
                        let (_, event) = facts.latest_event(key, hour).unwrap().unwrap();
                        serde_json::from_value::<OccupationTrendRun>(event).unwrap()
                    });
                    assert_eq!(
                        actual.as_ref().map(|run| (
                            &run.resistance_raw,
                            &run.compliance_raw,
                            run.read_hour,
                            run.states,
                            run.held_states,
                            &run.collaboration
                        )),
                        expected.map(|run| (
                            &run.resistance_raw,
                            &run.compliance_raw,
                            run.read_hour,
                            run.states,
                            run.held_states,
                            &run.collaboration
                        )),
                        "{tag} at {hour} after commit {window}"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn occupation_prepared_retains_all_five_thousand_occupiers_with_bounded_continuation() {
        let fixture = Fixture::new();
        let cancelled = AtomicBool::new(false);
        let mut prepared = Prepared::default();
        for (window, hour) in [100, 101].into_iter().enumerate() {
            prepared
                .observe_frame(
                    &frame(hour, 5000),
                    replay::format::FormatVersion::CURRENT,
                    None,
                )
                .unwrap();
            let mut writer = fixture.writer();
            prepared
                .write_window(
                    &mut writer,
                    1,
                    window,
                    HourSpan {
                        first_hour: hour,
                        last_hour: hour,
                    },
                )
                .unwrap();
            prepared.flush(&mut writer, 1, &cancelled).unwrap();
            writer.commit().unwrap();
            let bytes = prepared.saved_bytes().unwrap();
            eprintln!(
                "occupation compressed continuation for 5,000 occupiers after window {window}: {} bytes",
                bytes.len()
            );
            prepared = Prepared::read(bytes.as_slice()).unwrap();
        }
        assert_eq!(prepared.tags.len(), 5000);
        assert!(
            prepared
                .scan
                .result
                .states
                .iter()
                .all(|state| state.runs.len() == 1)
        );
        assert!(
            prepared
                .scan
                .trend
                .tracks
                .values()
                .all(|track| track.runs.len() == 1)
        );
        let reply = series::read_reply::<OccupiedStatesReply>(
            &fixture.store,
            &fixture.key,
            "occupied-states-1",
            1,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reply.states.len(), 5000);
    }
}
