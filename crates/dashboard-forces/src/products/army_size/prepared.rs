//! The army scan feeding shared exact series and prepared rank positions.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader, Run},
};
use replay_frame::frame::DecodedFrame;

pub mod block;
pub use block::FAMILY;

fn scan_state() -> ScanState {
    ScanState::new(true)
}

/// A country's new composition, published at an hour.
#[derive(Clone)]
struct Publication {
    tag: String,
    key: u64,
    hour: i32,
    /// The composition's parts, serialized.
    parts: Vec<u8>,
    starts: bool,
}

/// Publish a new composition under the country's next version.
fn publish_mix(
    mix_ids: &mut BTreeMap<String, i128>,
    parts: &mut Accumulator,
    publication: Publication,
) -> io::Result<()> {
    let Publication {
        tag,
        key,
        hour,
        parts: row,
        starts,
    } = publication;
    let version = match mix_ids.get_mut(&tag) {
        Some(version) => {
            *version += 1;
            *version
        }
        None => {
            mix_ids.insert(tag, 1);
            1
        }
    };
    parts.event_bytes(key, hour, row)?;
    parts.observe(key, hour, Some(version), starts);
    Ok(())
}

pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::DivisionCensus,
    TableId::Divisions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
    TableId::LexicalTokens,
    TableId::SentVolunteers,
    TableId::VolunteerHosts,
    TableId::CountryCounterInputs,
];

#[cfg(test)]
fn sample(
    frame: &DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
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
        replay_data::format::FormatVersion::CURRENT,
        false,
        &mut state,
        None,
    );
    (scan, state)
}

/// The tables the division inventory holds rows of.
const INVENTORY_TABLES: [TableId; 3] = [
    TableId::Divisions,
    TableId::DivisionTemplateIcons,
    TableId::DivisionLandInputs,
];

/// [`point_scan`] of consecutive exactly read hours, carrying the division
/// inventory from hour to hour as preparation's observer does: each inventory
/// table is applied unchanged when the decoder carried its rows, from the rows
/// its recorded delta changed when it applied one, and whole otherwise; values
/// are always the decoded table's. Everything else an hour's scan holds
/// (identities, census, totals, metadata) is read afresh, as `point_scan` reads
/// it. The first hour, any hour after a gap or a skipped hour, a frame read
/// alone and an hour after a refused inventory table are scanned whole.
#[derive(Default)]
struct PointScans {
    held: Option<HeldInventory>,
    /// Hours scanned from the carried inventory.
    #[cfg(test)]
    carried: usize,
}

struct HeldInventory {
    branch: u64,
    hour: i32,
    inventory: inventory::Inventory,
    /// The inventory tables' decoded views at `hour`.
    views: [replay_frame::frame::ViewIdentity; 3],
}

impl PointScans {
    fn scan(
        &mut self,
        frame: &DecodedFrame,
        recorded: Option<&dashboard_base::prepared::query::progressive::Recorded<'_>>,
    ) -> (Scan, ScanState) {
        let held = self.held.take().filter(|held| {
            held.branch == frame.branch
                && held.hour.checked_add(1) == Some(frame.hour)
                && held.inventory.errors().next().is_none()
        });
        let (Some(held), Some(recorded)) = (held, recorded) else {
            return point_scan(frame);
        };
        let changes = std::array::from_fn(|index| {
            let table = INVENTORY_TABLES[index];
            let view = &frame.tables[table.index()];
            if held.views[index].is(view) {
                return None;
            }
            Some(match (recorded.update(table), &**view) {
                (
                    Some(TableUpdate::Delta {
                        changed, deleted, ..
                    }),
                    replay_frame::frame::TableView::Complete(_),
                ) => Some((changed.as_slice(), deleted.as_slice())),
                _ => None,
            })
        });
        let updates = inventory::Updates::following(held.hour, changes);
        #[cfg(test)]
        {
            self.carried += 1;
        }
        let mut state = scan_state();
        state.inventory = Some(held.inventory);
        state.apply_updates(updates, frame);
        state.compose(frame);
        let mut scan = Scan::default();
        observe_updates(
            &mut scan,
            frame,
            updates,
            replay_data::format::FormatVersion::CURRENT,
            false,
            &mut state,
            None,
        );
        (scan, state)
    }

    /// Hold `state`'s inventory, scanned at `frame`, for the next hour.
    fn keep(&mut self, frame: &DecodedFrame, state: ScanState) {
        self.held = state.inventory.map(|inventory| HeldInventory {
            branch: frame.branch,
            hour: frame.hour,
            inventory,
            views: INVENTORY_TABLES.map(|table| frame.tables[table.index()].identity()),
        });
    }
}

fn sample_rows(
    scan: &Scan,
    state: &ScanState,
    hour: i32,
    requested: Option<&[(String, String)]>,
) -> io::Result<dashboard_base::prepared::query::progressive::Samples> {
    if requested.is_some_and(|keys| {
        keys.iter()
            .any(|(_, field)| field.parse::<usize>().ok().is_none_or(|index| index >= 8))
    }) {
        return Err(dashboard_core::invalid("Unknown army sample channel"));
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
pub struct Prepared {
    #[serde(skip)]
    catalogue_writer: series::CatalogueWriter,
    scan: Scan,
    aid: aid::History,
    #[serde(skip)]
    aid_runtime: aid::Runtime,
    #[serde(skip)]
    aid_recorded: Option<Vec<aid::Projection>>,
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
    /// While a block's extract records its calls, the compositions its fold
    /// publishes again on the carried versions ([`series::record_family`]).
    #[serde(skip)]
    recorded: Option<Vec<Publication>>,
    /// Hours whose divisions were applied from their changed rows alone.
    #[cfg(test)]
    #[serde(skip)]
    row_hours: usize,
    /// Observe every hour from the whole tables, never from changed rows.
    #[cfg(test)]
    #[serde(skip)]
    whole_tables: bool,
}
impl Default for Prepared {
    fn default() -> Self {
        Self {
            catalogue_writer: Default::default(),
            scan: Scan::default(),
            aid: Default::default(),
            aid_runtime: Default::default(),
            aid_recorded: None,
            tags: BTreeMap::new(),
            values: Default::default(),
            ranks: Default::default(),
            parts: Accumulator::default(),
            coverage: Accumulator::default(),
            mix_ids: BTreeMap::new(),
            first: None,
            last: 0,
            state: scan_state(),
            recorded: None,
            #[cfg(test)]
            row_hours: 0,
            #[cfg(test)]
            whole_tables: false,
        }
    }
}
impl Prepared {
    pub fn observe(
        &mut self,
        decoded: &DecodedFrame,
        replay: &Replay,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(test)]
        let whole = self.whole_tables.then_some(FrameChanges {
            rows_changed: changes.rows_changed,
            table_read: changes.table_read,
            slot_read: changes.slot_read,
            updates: None,
        });
        #[cfg(test)]
        let changes = whole.as_ref().unwrap_or(changes);
        let hour = decoded.hour;
        self.first.get_or_insert(hour);
        self.last = hour;
        for values in &mut self.values {
            values.advance(hour)?;
        }
        self.coverage.advance(hour)?;
        self.parts.advance(hour)?;
        let continues = self
            .scan
            .frontier
            .is_some_and(|at| at.checked_add(1) == Some(hour));
        let updates = Updates::decoded(changes, self.state.applied, continues);
        #[cfg(test)]
        let row_updates = self
            .state
            .inventory
            .as_ref()
            .map(|inventory| inventory.row_updates()[0]);
        let army_changed =
            self.state
                .step_updates(&mut self.scan, updates, decoded, replay, Some(changes));
        let gap = replay.gaps.iter().any(|gap| {
            gap.branch == decoded.branch && gap.first_hour <= hour && hour <= gap.last_hour
        });
        let projection = self.aid_runtime.step(
            decoded,
            changes,
            self.state
                .inventory
                .as_ref()
                .expect("detailed army inventory"),
            &self.state.identities,
            self.state.identity_changed,
            gap,
        );
        self.aid.advance(hour)?;
        match &mut self.aid_recorded {
            Some(recorded) if series::record_family(recorded.len() * 2 + 1) => {
                recorded.push(projection.clone());
                series::unrecorded(|| self.aid.observe(projection))?;
            }
            _ => self.aid.observe(projection)?,
        }
        if !army_changed {
            return Ok(());
        }
        #[cfg(test)]
        {
            self.row_hours += usize::from(
                row_updates
                    != self
                        .state
                        .inventory
                        .as_ref()
                        .map(|inventory| inventory.row_updates()[0]),
            );
        }
        let observed = self.scan.observed.len();
        for position in 0..observed + self.scan.removed.len() {
            let tag = match self.scan.observed.get(position) {
                Some(&index) => &self.state.identities[index].0,
                None => &self.scan.removed[position - observed],
            };
            let track = &self.scan.tracks[tag];
            #[cfg(test)]
            {
                self.state.work.emitted += 10;
            }
            let key = match self.tags.get(tag) {
                Some(&key) => key,
                None => {
                    let key = self.tags.len() as u64;
                    self.tags.insert(tag.clone(), key);
                    key
                }
            };
            let run = track.runs.last().filter(|run| run.last == hour);
            let starts = run.is_some_and(|run| run.starts && run.first == hour);
            // A run that continues the previous hour's keeps the coverage
            // series open; the same run keeps every value series open too.
            let continued = run.is_some_and(|run| run.first < hour);
            let follows = continued
                || (run.is_some()
                    && !starts
                    && track
                        .runs
                        .len()
                        .checked_sub(2)
                        .is_some_and(|at| track.runs[at].last.checked_add(1) == Some(hour)));
            if !follows {
                self.coverage.observe(key, hour, run.map(|_| 1), starts);
            }
            if !continued {
                for (index, values) in self.values.iter_mut().enumerate() {
                    values.observe(
                        key,
                        hour,
                        run.and_then(|run| run.totals.values[index]),
                        starts,
                    );
                }
            }
            if let Some(mix) = track.mix.last().filter(|mix| mix.last == hour) {
                // A composition continued from the previous hour keeps its
                // open version.
                if mix.first == hour {
                    let publication = Publication {
                        tag: tag.clone(),
                        key,
                        hour,
                        parts: serde_json::to_vec(&mix.parts)?,
                        starts: run.is_some_and(|run| run.starts),
                    };
                    match &mut self.recorded {
                        // A block's extract records the publication for its
                        // fold to version on the carried counts.
                        Some(recorded) if series::record_family(recorded.len() * 2) => {
                            recorded.push(publication.clone());
                            series::unrecorded(|| {
                                publish_mix(&mut self.mix_ids, &mut self.parts, publication)
                            })?;
                        }
                        _ => publish_mix(&mut self.mix_ids, &mut self.parts, publication)?,
                    }
                }
            } else {
                self.parts.observe(key, hour, None, false);
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.values
            .iter()
            .chain(&self.ranks)
            .map(Accumulator::bytes)
            .sum::<usize>()
            + self.parts.bytes()
            + self.coverage.bytes()
            + self.aid.bytes()
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
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let rank_plan = dashboard_base::ranking::RankPlan::new(
            &runs,
            |run| (run.first.max(first), run.last),
            cancelled,
        )?;
        for index in 0..8 {
            self.ranks[index].through(first);
            rank_plan.runs(
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
            self.ranks[index].flush_with_payloads(
                writer,
                &format!("army-ranks-{branch}-{index}"),
                0,
                cancelled,
                &mut payloads,
            )?;
            self.values[index].flush_with_payloads(
                writer,
                &format!("army-values-{branch}-{index}"),
                if matches!(index, 1 | 3) { 5 } else { 0 },
                cancelled,
                &mut payloads,
            )?;
        }
        self.parts.flush_with_payloads(
            writer,
            &format!("army-parts-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.coverage.flush_with_payloads(
            writer,
            &format!("army-coverage-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
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
        self.aid.flush(writer, branch, cancelled)?;
        self.catalogue_writer
            .write(writer, "army", branch, &self.tags, &self.scan)
    }
}

pub fn query(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Option<ArmySizeReply>> {
    ArmySize::validate(params).map_err(dashboard_core::invalid)?;
    if params.template_at.is_some() {
        return super::template_roster::query(source, params, context, cancelled).map(Some);
    }
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let coverage = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    if !coverage.as_ref().is_some_and(|coverage| coverage.complete) {
        if let Some(window) = params.window {
            let range = source
                .block_windows(branch)
                .get(window)
                .copied()
                .ok_or_else(|| dashboard_core::invalid("Unknown army history window"))?;
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
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let mut scan = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
            .map(|row| row.1)
            .unwrap_or_default();
        let block = source
            .replay_ref()
            .index
            .iter()
            .rev()
            .find(|block| block.branch == branch)
            .ok_or_else(|| dashboard_core::invalid("Unknown army branch"))?;
        let availability = dashboard_base::prepared::query::observations::scope(
            source,
            "army-size:availability",
            None,
        )
        .get_query(
            block,
            &(),
            cancelled,
            |_| std::mem::size_of::<[bool; 4]>(),
            || {
                let _phase = replay_frame::cpu::scope_phase("army:sampled_availability");
                let mut available = [false; 4];
                if let replay::container::SeekResult::Frame(frame) = source
                    .replay_ref()
                    .seek_selected(branch, block.first_hour, Some(TABLES))?
                {
                    let (point, _) = point_scan(&frame);
                    available = [
                        point.divisions,
                        point.strength,
                        point.manpower,
                        point.battalions,
                    ];
                }
                Ok(available)
            },
        )?;
        scan.divisions |= availability[0];
        scan.strength |= availability[1];
        scan.manpower |= availability[2];
        scan.battalions |= availability[3];
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
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let coverage = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    let history = Arc::new(dashboard_base::prepared::query::progressive::histories(
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
            .ok_or_else(|| dashboard_core::invalid("Unknown army history window"))?;
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
                    dashboard_base::products::country_groups::mark_unobserved(
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
            dashboard_base::products::country_history::selected_aliases(&history, &params.tags)
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
                .ok_or_else(|| dashboard_core::invalid("Missing prepared army composition"))?;
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
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    range: (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<(
    BTreeMap<(String, String), Vec<Run>>,
    Vec<HourSpan>,
    dashboard_base::products::country_history::RecordedHistories,
    Arc<Scan>,
)> {
    use dashboard_base::prepared::query::progressive;
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid army branch"))?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let index = channel(params.measure, params.count_for).to_string();
    let requested = params
        .tags
        .iter()
        .map(|tag| (tag.clone(), index.clone()))
        .collect::<Vec<_>>();
    // The native fold depends on the tail cut, not on preparation before
    // this window. Keep numeric values and their availability, warning counts
    // and compositions together; identities still join the current prefix.
    let first = coverage.map_or(i64::from(range.0), |coverage| {
        i64::from(range.0).max(i64::from(coverage.last_hour) + 1)
    });
    let blocks = source
        .replay_ref()
        .index
        .iter()
        .filter(|block| {
            block.branch == branch
                && i64::from(block.last_hour) >= first
                && block.first_hour <= range.1
        })
        .count();
    let observed =
        dashboard_base::prepared::query::observations::scope(source, "army-size:window", None)
            .working_set(blocks)
            .get_entry(
                branch,
                &(&index, &requested, range, params.by_type),
                &(first, coverage.is_some_and(|coverage| coverage.complete)),
                cancelled,
                |observed: &(progressive::TailData, Arc<Scan>)| {
                    let mut bytes = dashboard_base::prepared::prepass::HeldBytes::default();
                    bytes.value(&observed.1);
                    std::mem::size_of::<(progressive::TailData, Arc<Scan>)>()
                        + progressive::tail_bytes(&observed.0)
                        + bytes.total() as usize
                },
                || {
                    let _phase = replay_frame::cpu::scope_phase("army:sampled_window");
                    let metadata = std::cell::RefCell::new(Scan::default());
                    let points = std::cell::RefCell::new(PointScans::default());
                    let sampler =
                        |frame: &DecodedFrame,
                         recorded: Option<&progressive::Recorded<'_>>,
                         requested: Option<&[(String, String)]>| {
                            let (scan, state) = points.borrow_mut().scan(frame, recorded);
                            let mut values = sample_rows(&scan, &state, frame.hour, None)?;
                            points.borrow_mut().keep(frame, state);
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
                                    *metadata.invalid_table_reads.entry(reason).or_default() +=
                                        count;
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
                    let tail = progressive::tail_recorded(
                        source, branch, coverage, TABLES, &requested, range, &sampler, cancelled,
                    )?;
                    Ok((tail, Arc::new(metadata.into_inner())))
                },
            )?;
    let tail = &observed.0;
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
    Ok((
        tail.0.clone(),
        tail.1.clone(),
        history,
        Arc::clone(&observed.1),
    ))
}

fn progressive_window(
    source: &Source,
    params: &ArmySizeParams,
    context: &LoadContext,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
    range: HourSpan,
    cancelled: &AtomicBool,
) -> io::Result<ArmySizeReply> {
    let branch = params
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid army branch"))?;
    let (tail, _, history, observed) = sampled_window(
        source,
        params,
        context,
        coverage,
        (range.first_hour, range.last_hour),
        cancelled,
    )?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
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
        dashboard_base::products::country_history::selected_aliases(&history, &params.tags)
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

pub fn summary(
    source: &Source,
    params: &serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use dashboard_base::prepared::query::progressive;
    use serde_json::json;
    let selection: ArmySizeParams = serde_json::from_value(json!({
        "branchId": params["branchId"], "tags": params["tags"], "top": params["family"]["top"],
        "measure": params["family"]["measure"], "countFor": params["family"]["countFor"], "byType": false,
    }))?;
    ArmySize::validate(&selection).map_err(dashboard_core::invalid)?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid army branch"))?;
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Unknown army branch"))?;
    let bounds = dashboard_text::metadata::branch_bounds(source.replay_ref(), start);
    let (from, to, width) = series::viewport(
        params["from"].as_f64(),
        params["to"].as_f64(),
        params["width"]
            .as_u64()
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let coverage = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    let (tail, approximate, history, _) = sampled_window(
        source,
        &selection,
        context,
        coverage.as_ref(),
        (from.floor() as i32, to.floor() as i32),
        cancelled,
    )?;
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
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
            dashboard_base::ranking::top_membership(
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
                dashboard_base::products::country_groups::mark_unobserved(
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
        dashboard_base::products::country_history::selected_aliases(&history, &selection.tags)
    };
    if selection.top.is_some() {
        wanted.extend(sampled.keys().cloned());
    }
    wanted.sort();
    wanted.dedup();
    let selected =
        dashboard_base::products::country_history::selected_identities(&history, &selection.tags);
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

pub fn next_change(
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
    ArmySize::validate(&selection).map_err(dashboard_core::invalid)?;
    let navigation: series::Navigation = serde_json::from_value(params.clone())?;
    let origin = navigation.origin()?;
    let branch = selection
        .branch_id
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid branch"))?;
    let accepted = dashboard_base::prepared::coverage::coverage(
        source,
        context.content.as_ref(),
        branch,
        cancelled,
    )?;
    if !accepted.as_ref().is_some_and(|coverage| coverage.complete)
        && accepted
            .as_ref()
            .is_none_or(|coverage| navigation.last_hour > f64::from(coverage.last_hour))
    {
        use dashboard_base::prepared::query::progressive;
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
        let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
        let mut actors = series::read_catalogue::<Scan>(&store, &key, "army", branch)?
            .map(|row| row.0)
            .unwrap_or_default();
        let mut within: BTreeMap<String, Vec<HourSpan>> =
            serde_json::from_value(params.get("within").cloned().unwrap_or_else(|| json!({})))?;
        let wanted = if selection.top.is_some() {
            history.countries.keys().cloned().collect()
        } else {
            dashboard_base::products::country_history::selected_aliases(&history, &selection.tags)
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
            let selected = dashboard_base::products::country_history::selected_identities(
                &history,
                &selection.tags,
            );
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
                    dashboard_base::ranking::for_each_piece(
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
            within = dashboard_base::ranking::top_membership(
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
            dashboard_base::ranking::rank_positions(
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
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let history = dashboard_base::prepared::query::progressive::histories(
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
            dashboard_base::products::country_history::selected_aliases(&history, &selection.tags)
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

/// Where an army window's sampled read spends its time on a real recording:
/// its blocks' reads, their hours' decodes and the per-hour army scan.
#[cfg(test)]
mod window_profile {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "requires BATTLE_PLAN_QUERY_PROFILE_RECORDING (a real recording, only read)"]
    fn army_window_read_decode_and_scan_times() {
        let path = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_QUERY_PROFILE_RECORDING").or_else(|| std::env::var_os("HOI4_QUERY_PROFILE_RECORDING"))
                .expect("Set BATTLE_PLAN_QUERY_PROFILE_RECORDING"),
        );
        let cancelled = AtomicBool::new(false);
        let source = Source::open(
            dashboard_base::SourceKey {
                source_id: "army-profile".into(),
                revision: dashboard_core::protocol::replay_revision(&path).unwrap(),
            },
            path,
            &cancelled,
        )
        .unwrap();
        let replay = source.replay_ref();
        let branch = replay.index.last().unwrap().branch;
        let blocks: Vec<_> = replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .collect();
        // The army window of `$window:0:last`: the branch's last block.
        let window = &blocks[blocks.len() - 1..];
        let (mut read, mut decode, mut scan) = (Duration::ZERO, Duration::ZERO, Duration::ZERO);
        let mut hours = 0;
        for block in window {
            let started = Instant::now();
            let frames = replay.read_block_selected(block, TABLES).unwrap();
            read += started.elapsed();
            let mut decoder = replay_frame::frame::FrameDecoder::for_tables(TABLES);
            for frame in &frames {
                let started = Instant::now();
                let decoded = decoder.advance(frame).unwrap();
                decode += started.elapsed();
                let started = Instant::now();
                drop(std::hint::black_box(point_scan(decoded)));
                scan += started.elapsed();
                hours += 1;
            }
        }
        eprintln!(
            "army window: {} blocks, {hours} hours: read {:.0} ms, decode {:.0} ms, scan {:.0} ms",
            window.len(),
            read.as_secs_f64() * 1000.0,
            decode.as_secs_f64() * 1000.0,
            scan.as_secs_f64() * 1000.0,
        );
        // What one block's read holds in memory, as the frame cache counts it.
        use dashboard_base::prepared::query::frames;
        frames::set_budget(&source, usize::MAX);
        for (name, tables) in [
            ("army", TABLES),
            ("measure", dashboard_base::prepared_measure::TABLES),
            ("supply", crate::products::supply_health::prepared::TABLES),
        ] {
            let before = frames::held_bytes(&source);
            let started = Instant::now();
            let read = frames::block(&source, window[window.len() - 1], tables, &cancelled);
            eprintln!(
                "{name} tables, last block: {} in {:.0} ms, {} MiB",
                if matches!(read, Ok(Some(_))) {
                    "read"
                } else {
                    "not cached"
                },
                started.elapsed().as_secs_f64() * 1000.0,
                (frames::held_bytes(&source) - before) >> 20
            );
        }
        // The window's exact read as the query makes it, every country
        // requested: scanned whole every hour, then from the carried inventory.
        let block = window[0];
        let range = (block.first_hour, block.last_hour);
        let tags: Vec<_> = source
            .metadata().unwrap()
            .countries
            .iter()
            .map(|country| country.tag.clone())
            .collect();
        let requested: Vec<_> = tags
            .iter()
            .map(|tag| (tag.clone(), "0".to_string()))
            .collect();
        let whole = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            let (scan, state) = point_scan(frame);
            sample_rows(&scan, &state, frame.hour, requested)
        };
        // The carried read goes first and pays the block's read; the point
        // scans find it in the frame cache.
        let points = std::cell::RefCell::new(PointScans::default());
        let carried =
            |frame: &DecodedFrame,
             recorded: Option<&dashboard_base::prepared::query::progressive::Recorded<'_>>,
             requested: Option<&[(String, String)]>| {
                let (scan, state) = points.borrow_mut().scan(frame, recorded);
                let samples = sample_rows(&scan, &state, frame.hour, requested);
                points.borrow_mut().keep(frame, state);
                samples
            };
        let started = Instant::now();
        let after = dashboard_base::prepared::query::progressive::tail_recorded(
            &source, branch, None, TABLES, &requested, range, &carried, &cancelled,
        )
        .unwrap();
        let after_ms = started.elapsed().as_secs_f64() * 1000.0;
        let started = Instant::now();
        let before = dashboard_base::prepared::query::progressive::tail(
            &source, branch, None, TABLES, &requested, range, &whole, &cancelled,
        )
        .unwrap();
        let before_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(before, after);
        eprintln!(
            "army window exact read, {} countries: point scans {before_ms:.0} ms, carried {after_ms:.0} ms ({} of the hours carried)",
            tags.len(),
            points.borrow().carried,
        );
    }
}

/// The carried army scan equals [`point_scan`] at every hour of exact windows.
#[cfg(test)]
mod carried_tests {
    use super::*;
    use crate::products::parts_race_prepared::test_fixtures::frame;
    use dashboard_base::test_fixtures::{CAMPAIGN, continuation_records, records};
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    /// The army fixture's hour, with the last slot's holder replaced from hour 10, a
    /// changed template from hour 15, a division deleted from hour 20 and one
    /// added from hour 22.
    fn hour(hour: i32, count: u32) -> DecodedFrame {
        let mut decoded = frame(hour, count);
        if hour >= 10 {
            // A new native holder of the last slot, keyed after every other.
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *decoded.tables[TableId::Countries.index()]
            {
                rows.last_mut().unwrap().id = 300_000;
            }
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *decoded.tables[TableId::CountryLinks.index()]
            {
                rows.last_mut().unwrap().id = 300_000;
            }
        }
        if hour >= 15
            && let TableView::Complete(TableRows::DivisionTemplateIcons(rows)) =
                &mut *decoded.tables[TableId::DivisionTemplateIcons.index()]
        {
            rows[0].subunits[0].count = 4;
        }
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *decoded.tables[TableId::Divisions.index()]
        {
            if hour >= 20 {
                rows.retain(|row| row.id != 3);
            }
            if hour >= 22 {
                let mut added = rows[0].clone();
                added.id = u64::from(count) + 10;
                rows.push(added);
            }
        }
        decoded
    }

    /// Three blocks of twelve hours, with a recorded gap at hour 16.
    fn source(count: u32) -> Source {
        let block = |first: i32| (first..first + 12).map(|at| hour(at, count)).collect();
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, block(0))]).unwrap();
        for first in [12, 24] {
            replay = replay
                .append(&[continuation_records(&replay, 1, block(first))])
                .unwrap();
        }
        replay.gaps.push(replay_frame::branches::Gap {
            branch: 1,
            first_hour: 16,
            last_hour: 16,
            reason: "synthetic gap".into(),
        });
        Source::from_replay(
            dashboard_base::SourceKey {
                source_id: "army-carried".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn carried_scans_equal_point_scans(count: u32) {
        let source = source(count);
        let points = std::cell::RefCell::new(PointScans::default());
        let hours = std::cell::Cell::new(0);
        let carried =
            |frame: &DecodedFrame,
             recorded: Option<&dashboard_base::prepared::query::progressive::Recorded<'_>>,
             requested: Option<&[(String, String)]>| {
                let (scan, state) = points.borrow_mut().scan(frame, recorded);
                let (whole, whole_state) = point_scan(frame);
                assert_eq!(
                    serde_json::to_value(&scan).unwrap(),
                    serde_json::to_value(&whole).unwrap(),
                    "hour {}",
                    frame.hour
                );
                assert_eq!(
                    state.identities, whole_state.identities,
                    "hour {}",
                    frame.hour
                );
                let samples = sample_rows(&scan, &state, frame.hour, requested)?;
                assert_eq!(
                    samples,
                    sample_rows(&whole, &whole_state, frame.hour, requested)?,
                    "hour {}",
                    frame.hour
                );
                points.borrow_mut().keep(frame, state);
                hours.set(hours.get() + 1);
                Ok(samples)
            };
        let whole = |frame: &DecodedFrame, requested: Option<&[(String, String)]>| {
            let (scan, state) = point_scan(frame);
            sample_rows(&scan, &state, frame.hour, requested)
        };
        let requested: Vec<_> = (0..count.min(3))
            .flat_map(|index| (0..8).map(move |field| (tag(index), field.to_string())))
            .collect();
        let flag = AtomicBool::new(false);
        for window in [(0, 35), (5, 20), (12, 12), (17, 30)] {
            let read = dashboard_base::prepared::query::progressive::tail_recorded(
                &source, 1, None, TABLES, &requested, window, &carried, &flag,
            )
            .unwrap();
            assert_eq!(
                read,
                dashboard_base::prepared::query::progressive::tail(
                    &source, 1, None, TABLES, &requested, window, &whole, &flag,
                )
                .unwrap(),
                "{window:?}"
            );
        }
        let points = points.into_inner();
        assert!(
            points.carried > hours.get() / 2,
            "{} of {} hours carried",
            points.carried,
            hours.get()
        );
        assert!(
            points.carried < hours.get(),
            "block starts, gaps and left edges scan whole"
        );
    }

    fn tag(index: u32) -> String {
        crate::products::parts_race_prepared::test_fixtures::tag(index)
    }

    #[test]
    fn carried_army_scans_equal_point_scans_at_every_hour() {
        carried_scans_equal_point_scans(40);
    }
}
