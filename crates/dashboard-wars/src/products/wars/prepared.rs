use super::*;
use dashboard_base::prepared::frame_changes::FrameChanges;
use dashboard_base::prepared::series::{self, Accumulator, Catalogue as KeyCatalogue, Reader};
use replay_data::codec::Decoder;
use replay_frame::frame::SharedView;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// The wars inputs the observer holds and publishes: every table it reads but
/// the clock, whose hourly rows no reply reads, so a change input is the same
/// whichever hour's clock preparation read with it.
pub(super) fn is_input(table: TableId) -> bool {
    table != TableId::Clock && TABLES.contains(&table)
}

/// The first recorded capitulation date after `hour`: the hour at which an
/// already captured capitulation becomes usable.
pub(super) fn next_capitulation(capitulations: &[rows::CapitulationRow], hour: i32) -> Option<i32> {
    capitulations
        .iter()
        .filter(|row| row.capitulated != 0 && row.capitulated_hour > hour)
        .map(|row| row.capitulated_hour)
        .min()
}

/// One branch's global change inputs and relation-lifetime counters, never viewpoints.
#[derive(Default, Serialize, Deserialize)]
pub struct Observer {
    #[serde(skip)]
    pub(super) indices: BTreeMap<String, u64>,
    pub(super) first_reads: Vec<i32>,
    #[cfg_attr(test, serde(deserialize_with = "count_snapshot"))]
    snapshot: Vec<u8>,
    pub(super) batch: Accumulator,
    #[serde(skip)]
    pub(super) frame: Option<DecodedFrame>,
    #[serde(skip)]
    pub(super) inputs: Option<Hour>,
    /// Counter keys and endpoint order of the current native relation join.
    #[serde(skip)]
    pub(super) counter_keys: Vec<(u64, bool)>,
    #[cfg(test)]
    #[serde(skip)]
    pub(super) counter_key_builds: usize,
    pub(super) next_capitulation: Option<i32>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub(super) work: dashboard_base::prepared::observer_tests::Work,
    /// Each change input published since they were last taken, with its
    /// hour, while the recorded viewpoint's walk follows them.
    #[serde(skip)]
    published: Option<Vec<(i32, Vec<u8>)>>,
    /// While a block's extract records its calls ([`Self::record_calls`]),
    /// each hour it observed the change inputs' ordinal at, and whether after
    /// a gap: its fold observes them on the carried count of inputs
    /// ([`series::record_family`]).
    #[serde(skip)]
    pub(super) recorded: Option<Vec<(i32, bool)>>,
    /// Whether an observation since the extract began recording found no
    /// join of the hour before and took the relations it held from the open
    /// counters, which a block's own observer does not share with the carried
    /// one.
    #[serde(skip)]
    pub(super) unjoined: bool,
    #[cfg(test)]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
}
#[cfg(test)]
thread_local! {
    static SNAPSHOT_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
fn count_snapshot<'de, D: serde::Deserializer<'de>>(input: D) -> Result<Vec<u8>, D::Error> {
    let bytes = Vec::<u8>::deserialize(input)?;
    SNAPSHOT_BYTES.set(SNAPSHOT_BYTES.get() + bytes.len());
    Ok(bytes)
}
impl Observer {
    pub fn restore((indices, mut state): KeyCatalogue<Self>) -> Self {
        state.indices = indices;
        state
    }
    #[cfg(test)]
    pub(crate) fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay_data::format::FormatVersion::CURRENT);
        self.observe_changed(frame, &changes)
    }
    pub fn observe_changed(&mut self, frame: &DecodedFrame, fact: &FrameChanges) -> io::Result<()> {
        self.batch.advance(frame.hour)?;
        if self.frame.is_none() && !self.snapshot.is_empty() {
            let bytes = dashboard_base::prepared::series::decompress(self.snapshot.as_slice())?;
            let mut input = Decoder::new(&bytes);
            self.frame = Some(DecodedFrame::decode_checkpoint(&mut input)?);
            input.finish()?;
        }
        let gap = self
            .frame
            .as_ref()
            .is_none_or(|old| old.hour.checked_add(1) != Some(frame.hour));
        let dated_cap = self
            .next_capitulation
            .is_some_and(|hour| hour <= frame.hour);
        if !gap
            && !dated_cap
            && TABLES
                .iter()
                .filter(|&&table| is_input(table))
                .all(|table| !fact.rows_changed[table.index()])
        {
            self.frame.as_mut().expect("continuing war inputs").hour = frame.hour;
            return Ok(());
        }
        if fact.rows_changed[TableId::Capitulation.index()] || dated_cap {
            let capitulations = frame.complete::<tables::Capitulation>().unwrap_or_default();
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += capitulations.len();
            }
            self.next_capitulation = next_capitulation(capitulations, frame.hour);
        }
        let mut current = DecodedFrame {
            branch: frame.branch,
            hour: frame.hour,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                let table = TableId::ALL[index];
                if !is_input(table) {
                    return TableView::Unavailable {
                        reason: "Table not selected for reconstruction".into(),
                    }
                    .into();
                }
                if !fact.rows_changed[index]
                    && let Some(old) = &self.frame
                {
                    return old.tables[index].clone();
                }
                if table == TableId::WarRelations {
                    if let Some(old) = &self.frame
                        && same_relation_structure(&old.tables[index], &frame.tables[index])
                    {
                        return old.tables[index].clone();
                    }
                    let mut view = frame.tables[index].clone();
                    let rows = match &mut *view {
                        TableView::Complete(rows) | TableView::Truncated { rows, .. } => Some(rows),
                        _ => None,
                    };
                    if let Some(rows::TableRows::WarRelations(rows)) = rows {
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.row_visits += rows.len();
                        }
                        for row in rows {
                            row.first_casualties = 0;
                            row.second_casualties = 0;
                            row.first_unknown_casualties = 0;
                            row.second_unknown_casualties = 0;
                        }
                    }
                    view
                } else {
                    frame.tables[index].clone()
                }
            }),
        };
        let mut changes = Vec::new();
        if let Some(old) = &self.frame {
            for &table in TABLES.iter().filter(|&&table| is_input(table)) {
                if !fact.rows_changed[table.index()] {
                    continue;
                }
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += match &*current.tables[table.index()] {
                        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
                        _ => 0,
                    };
                }
                if same_inputs(
                    table,
                    &old.tables[table.index()],
                    &current.tables[table.index()],
                ) {
                    current.tables[table.index()] = old.tables[table.index()].clone();
                } else {
                    changes.push(table as u16);
                }
            }
        }
        // An already captured future capitulation becomes usable when its date is reached.
        let same_structure = !gap && changes.is_empty();
        if same_structure && !dated_cap && !fact.rows_changed[TableId::WarRelations.index()] {
            self.frame = Some(current);
            return Ok(());
        }
        if gap || !changes.is_empty() || dated_cap {
            if self.frame.is_none() {
                changes = TABLES
                    .iter()
                    .filter(|&&table| is_input(table))
                    .map(|table| *table as u16)
                    .collect();
            }
            let packet = DecodedFrame {
                tables: std::array::from_fn(|index| {
                    if changes.contains(&(TableId::ALL[index] as u16)) {
                        current.tables[index].clone()
                    } else {
                        TableView::Unavailable {
                            reason: "Table not selected for reconstruction".into(),
                        }
                        .into()
                    }
                }),
                ..current.clone()
            };
            let mut bytes = Vec::new();
            packet.encode_checkpoint(&mut bytes)?;
            // A compressed packet keeps the open batch compact; its row is raw
            // bytes, read back without a JSON layer.
            let packet = dashboard_base::prepared::series::compress(bytes.as_slice(), 3)?;
            let row = super::walks::encode_input(&changes, &packet)?;
            if let Some(published) = &mut self.published {
                published.push((frame.hour, row.clone()));
            }
            self.batch.event_raw(0, frame.hour, row)?;
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.emitted += 1;
            }
        }
        let event = self.batch.state(0).expect("first change input").events - 1;
        if gap
            || self
                .batch
                .state(0)
                .and_then(|state| state.last)
                .is_none_or(|run| run.value != i128::from(event))
        {
            match &mut self.recorded {
                // A block's extract records the ordinal for its fold to
                // observe on the carried count of inputs.
                Some(recorded) if series::record_family(recorded.len()) => {
                    recorded.push((frame.hour, gap));
                    let batch = &mut self.batch;
                    series::unrecorded(|| {
                        batch.observe(0, frame.hour, Some(i128::from(event)), gap);
                    });
                }
                _ => self
                    .batch
                    .observe(0, frame.hour, Some(i128::from(event)), gap),
            }
        }
        // Names and dated presentation operands publish topology without
        // rebuilding the unchanged native relation join or casualty keys.
        if !gap
            && self.inputs.is_some()
            && ![
                TableId::Countries,
                TableId::CountryLinks,
                TableId::LocalPlayer,
                TableId::WarOwnedRelations,
                TableId::WarRelations,
                TableId::Factions,
                TableId::FactionMembers,
                TableId::WarNameRules,
                TableId::WarOverviewCountries,
            ]
            .iter()
            .any(|table| changes.contains(&(*table as u16)))
            && !fact.rows_changed[TableId::WarRelations.index()]
        {
            self.frame = Some(current);
            return Ok(());
        }
        let refresh = same_structure
            && self
                .inputs
                .as_mut()
                .is_some_and(|inputs| inputs.refresh_losses(frame));
        if refresh
            && let Some(inputs) = &self.inputs
            && self.counter_keys.len() == inputs.relation_order.len()
        {
            for (&pair, &(index, reversed)) in inputs.relation_order.iter().zip(&self.counter_keys)
            {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                let row = &inputs.world.relations[&pair];
                let raw = [
                    row.first_casualties,
                    row.first_unknown_casualties,
                    row.second_casualties,
                    row.second_unknown_casualties,
                ];
                observe_counters(&mut self.batch, index, raw, reversed, frame.hour, false);
            }
            self.frame = Some(current);
            return Ok(());
        }
        let previous: std::collections::BTreeSet<_> = match &self.inputs {
            Some(inputs) => inputs
                .world
                .relations
                .values()
                .map(|row| {
                    let identity = WarRelationKey::new(
                        inputs.identities[&row.first_country],
                        inputs.identities[&row.second_country],
                        row.start_hour,
                    );
                    self.indices[&serde_json::to_string(&identity).expect("native relation key")]
                })
                .collect(),
            // A restored observer has no join of the hour before; the relations
            // it held are those whose counters are still open. Without a join
            // otherwise, none are.
            None => {
                self.unjoined = true;
                self.batch
                    .active()
                    .filter_map(|key| key.checked_sub(1).map(|key| key / 4))
                    .collect()
            }
        };
        let mut present = std::collections::BTreeSet::new();
        // Canonical endpoint identity, not raw references or a selected viewpoint.
        if !refresh {
            self.inputs = Hour::read(
                frame,
                frame
                    .complete::<tables::Countries>()
                    .ok()
                    .and_then(|rows| rows.first())
                    .and_then(|row| std::str::from_utf8(&row.tag).ok()),
            )
            .ok();
        }
        self.counter_keys.clear();
        if let Some(inputs) = &self.inputs {
            // New lifetimes are numbered in recorded row order: the joined
            // map's hash order differs between processes.
            for row in inputs
                .relation_order
                .iter()
                .map(|key| &inputs.world.relations[key])
            {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                let identity = WarRelationKey::new(
                    inputs.identities[&row.first_country],
                    inputs.identities[&row.second_country],
                    row.start_hour,
                );
                let name = serde_json::to_string(&identity)?;
                #[cfg(test)]
                {
                    self.counter_key_builds += 1;
                }
                let next = self.indices.len() as u64;
                let index = *self.indices.entry(name).or_insert_with(|| {
                    self.first_reads.push(frame.hour);
                    next
                });
                present.insert(index);
                let raw = [
                    row.first_casualties,
                    row.first_unknown_casualties,
                    row.second_casualties,
                    row.second_unknown_casualties,
                ];
                let reversed = inputs.identities[&row.first_country] != identity.first;
                self.counter_keys.push((index, reversed));
                observe_counters(
                    &mut self.batch,
                    index,
                    raw,
                    reversed,
                    frame.hour,
                    gap || !previous.contains(&index),
                );
            }
        }
        for index in previous.difference(&present) {
            for field in 0..4 {
                self.batch
                    .observe(1 + index * 4 + field, frame.hour, None, false);
            }
        }
        self.frame = Some(current);
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.batch.bytes()
    }
    /// Whether hours `first..=last`, at which no input the observer compares
    /// changed or was decoded again whole, continue the hour it observed
    /// last and reach no capitulation date: then observing each would only
    /// authenticate it and move the held inputs to it ([`Self::advance_quiet`]).
    pub fn quiet_continues(&self, first: i32, last: i32) -> bool {
        self.frame
            .as_ref()
            .is_some_and(|frame| frame.hour.checked_add(1) == Some(first))
            && self.next_capitulation.is_none_or(|hour| hour > last)
    }
    /// Keep each change input published from now on until taken
    /// ([`Self::take_inputs`]).
    pub fn keep_inputs(&mut self) {
        self.published.get_or_insert_with(Vec::new);
    }
    /// From now on, record the calls this observer makes on its series as
    /// series 0 while a recording is active ([`series::with_recording`]),
    /// its change-input ordinals as its own calls, and the change inputs it
    /// publishes: a block's extract after the block's first hour.
    pub(super) fn record_calls(&mut self) {
        self.batch.record_as(0);
        self.recorded = Some(Vec::new());
        self.unjoined = false;
        self.published = Some(Vec::new());
    }
    /// The change inputs published since they were last taken.
    pub fn take_inputs(&mut self) -> Vec<(i32, Vec<u8>)> {
        self.published
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }
    /// Hours `first..=last` that [`Self::quiet_continues`] allows: each hour
    /// authenticated in turn, which a block extract's recording keeps as one
    /// span that its fold makes again in constant time.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<()> {
        for hour in first..=last {
            self.batch.advance(hour)?;
        }
        if let Some(frame) = &mut self.frame {
            frame.hour = last;
        }
        Ok(())
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.batch
            .flush(writer, &format!("wars-series-{branch}"), 0, cancelled)?;
        if let Some(frame) = &self.frame {
            let mut bytes = Vec::new();
            frame.encode_checkpoint(&mut bytes)?;
            self.snapshot = dashboard_base::prepared::series::compress(bytes.as_slice(), 3)?;
        }
        series::write_catalogue(writer, "wars", branch, &self.indices, self)
    }
}

/// Observe a relation's four counters in canonical endpoint order.
fn observe_counters(
    batch: &mut Accumulator,
    index: u64,
    raw: [i64; 4],
    reversed: bool,
    hour: i32,
    force: bool,
) {
    let order = if reversed { [2, 3, 0, 1] } else { [0, 1, 2, 3] };
    for (field, raw) in order.into_iter().map(|field| raw[field]).enumerate() {
        let key = 1 + index * 4 + field as u64;
        if force
            || batch
                .state(key)
                .and_then(|state| state.last)
                .is_none_or(|run| run.value != i128::from(raw))
        {
            batch.observe(key, hour, Some(i128::from(raw)), false);
        }
    }
}

/// Equality after removing the casualty counters, without copying the rows.
/// Truncated evidence still compares its missing count.
fn same_relation_structure(a: &SharedView, b: &SharedView) -> bool {
    let (a, b) = match (&**a, &**b) {
        (TableView::Complete(a), TableView::Complete(b)) => (a, b),
        (
            TableView::Truncated {
                rows: a,
                missing: am,
            },
            TableView::Truncated {
                rows: b,
                missing: bm,
            },
        ) if am == bm => (a, b),
        _ => return false,
    };
    let (rows::TableRows::WarRelations(a), rows::TableRows::WarRelations(b)) = (a, b) else {
        return false;
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            (
                a.first_country,
                a.second_country,
                a.start_hour,
                a.end_hour,
                a.cancel,
                a.first_was_instigator,
            ) == (
                b.first_country,
                b.second_country,
                b.start_hour,
                b.end_hour,
                b.cancel,
                b.first_was_instigator,
            )
        })
}

/// Whether two views of an input `table` agree on every field the observer
/// compares: the fields its block read watches.
pub(super) fn same_inputs(table: TableId, a: &SharedView, b: &SharedView) -> bool {
    if SharedView::ptr_eq(a, b) {
        return true;
    }
    if let (TableView::Complete(a), TableView::Complete(b)) = (&**a, &**b) {
        match (a, b) {
            (rows::TableRows::States(a), rows::TableRows::States(b)) => {
                return a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        (a.id, &a.custom_name, &a.owner_name)
                            == (b.id, &b.custom_name, &b.owner_name)
                    });
            }
            (rows::TableRows::Capitulation(a), rows::TableRows::Capitulation(b)) => {
                return a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        (a.country, a.capitulated, a.capitulated_hour)
                            == (b.country, b.capitulated, b.capitulated_hour)
                    });
            }
            (rows::TableRows::Factions(_), rows::TableRows::Factions(_)) => return true,
            _ => {}
        }
    }
    crate::products::war_casualties::same_name_rows(table, a, b)
}

#[derive(Deserialize)]
struct Reached {
    hour: i32,
}
#[derive(Deserialize)]
struct Requested {
    #[serde(default)]
    reads: Vec<Reached>,
}

pub fn query(
    source: &Source,
    value: &Value,
    store: &store::Store,
    key: &store::SourceKey,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: WarsParams = serde_json::from_value(
        serde_json::json!({"branchId":value["branchId"],"viewpointTag":value["viewpointTag"]}),
    )?;
    Wars::validate(&params).map_err(invalid)?;
    let cat = context
        .content
        .as_ref()
        .map(|options| catalogue::catalogue(source, options))
        .transpose()?;
    let requested: Requested = serde_json::from_value(value.clone())?;
    let hours = requested.reads.iter().map(|read| read.hour).collect();
    let reply = projection(
        store,
        key,
        source,
        &params,
        None,
        &hours,
        cat.as_ref(),
        cancelled,
    )?
    .unwrap_or_else(|| WarsReply {
        branch_id: params.branch_id.clone(),
        available: false,
        viewpoints: Vec::new(),
        country_names: Vec::new(),
        wars: Vec::new(),
        unobserved: Vec::new(),
        capitulations_unobserved: Vec::new(),
        warnings: Vec::new(),
    });
    Ok(serde_json::to_value(reply)?)
}

/// The groups and contributions at each requested read hour, independently
/// of preparation's progress, each from its hour's frame alone
/// ([`super::at_hour`]); an hour outside the branch or in a recorded gap is
/// unavailable.
pub fn current(
    source: &Source,
    value: &Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Vec<WarsAtHour>> {
    let params: WarsParams = serde_json::from_value(
        serde_json::json!({"branchId":value["branchId"],"viewpointTag":value["viewpointTag"]}),
    )?;
    Wars::validate(&params).map_err(invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| invalid("Wars branch is not in this recording"))?;
    let (first, last) = branch_bounds(source.replay_ref(), start);
    let requested: Requested = serde_json::from_value(value.clone())?;
    let hours: BTreeSet<i32> = requested.reads.iter().map(|read| read.hour).collect();
    if hours.is_empty() {
        return Ok(Vec::new());
    }
    let cat = context
        .content
        .as_ref()
        .map(|options| catalogue::catalogue(source, options))
        .transpose()?;
    let mut current = Vec::with_capacity(hours.len());
    let tables: Vec<_> = TABLES
        .iter()
        .copied()
        .chain([TableId::WarParticipation, TableId::WarLendLeaseHistories])
        .collect();
    for hour in hours {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let frame = if (first..=last).contains(&hour) {
            dashboard_base::prepared::query::progressive::frame_at(
                source, branch, hour, &tables, cancelled,
            )?
        } else {
            None
        };
        current.push(match frame {
            Some(frame) => super::at_hour(
                branch,
                &frame,
                params.viewpoint_tag.as_deref(),
                cat.as_deref(),
            )?,
            None => WarsAtHour {
                hour,
                contributions: Vec::new(),
                available: false,
                wars: Vec::new(),
                warnings: Vec::new(),
            },
        });
    }
    Ok(current)
}

/// Only the continuation's counter operands are needed to hydrate reached
/// hours. The input snapshot belongs to preparation and the native walk.
#[derive(Default, Deserialize)]
struct QueryCounters {
    first_reads: Vec<i32>,
    batch: Accumulator,
}
fn read_query_counters(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    structure_only: bool,
) -> io::Result<Option<KeyCatalogue<QueryCounters>>> {
    let _cpu = replay_frame::cpu::scope_phase("wars:query_catalogue");
    if structure_only {
        return series::read_catalogue::<serde::de::IgnoredAny>(store, key, "wars", branch)
            .map(|catalogue| catalogue.map(|_| (BTreeMap::new(), QueryCounters::default())));
    }
    series::read_catalogue(store, key, "wars", branch)
}

/// Whole-branch native projection, including membership and naming history before
/// any requested casualty hour. `selected` overrides the tag/LocalPlayer with
/// the original native-holder selection; an empty set never falls back.
/// `hours` controls exact casualty hydration only; an empty set returns structure.
/// The viewpoint's walk is kept on the source ([`super::walks`]): a later query
/// walks only inputs published since, and the inputs from the nearest resume
/// point to each requested hour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn projection(
    store: &store::Store,
    key: &store::SourceKey,
    source: &Source,
    params: &WarsParams,
    selected: Option<&HashSet<(u32, u32)>>,
    hours: &BTreeSet<i32>,
    cat: Option<&Arc<Catalogue>>,
    cancelled: &AtomicBool,
) -> io::Result<Option<WarsReply>> {
    Wars::validate(params).map_err(invalid)?;
    let branch = params.branch_id.parse().expect("validated branch");
    let Some((indices, state)) = read_query_counters(store, key, branch, hours.is_empty())? else {
        return Ok(None);
    };
    let branch_info = source
        .replay_ref()
        .branches
        .iter()
        .find(|b| b.id == branch)
        .ok_or_else(|| invalid("Wars branch is not in this recording"))?;
    let bounds = branch_bounds(source.replay_ref(), branch_info);
    let mut reader = Reader::new(store, key, format!("wars-series-{branch}"), cancelled)?;
    let mut casualties: HashMap<usize, Vec<WarCasualtyRun>> = HashMap::new();
    let cpu = replay_frame::cpu::scope_phase("wars:query_walk");
    let mut reply = super::walks::walk(
        source,
        store,
        key,
        branch,
        bounds,
        params,
        selected,
        cat,
        hours,
        cancelled,
        |walk, span| {
            if hours
                .range(span.first_hour..=span.last_hour)
                .next()
                .is_none()
            {
                return Ok(());
            }
            let Some((inputs, observed)) = walk
                .held
                .as_ref()
                .and_then(|held| held.observed.as_ref().ok())
            else {
                return Ok(());
            };
            let counters: HashMap<_, _> = inputs
                .world
                .relations
                .iter()
                .map(|(&pair, row)| {
                    let identity = WarRelationKey::new(
                        inputs.identities[&row.first_country],
                        inputs.identities[&row.second_country],
                        row.start_hour,
                    );
                    let index = indices
                        .get(&serde_json::to_string(&identity)?)
                        .copied()
                        .ok_or_else(|| invalid("Missing prepared war relation lifetime"))?;
                    Ok((
                        pair,
                        (
                            index,
                            inputs.identities[&row.first_country] == identity.first,
                        ),
                    ))
                })
                .collect::<io::Result<_>>()?;
            let losses = hydrate_losses(&mut reader, &state, &counters, observed, span, hours)?;
            for ((id, _), runs) in walk.open.iter().zip(losses) {
                casualties.entry(*id).or_default().extend(runs);
            }
            Ok(())
        },
    )?;
    drop(cpu);
    let _cpu = replay_frame::cpu::scope_phase("wars:query_finish");
    for (index, war) in reply.wars.iter_mut().enumerate() {
        war.casualties.clear();
        let mut runs = casualties.remove(&index).unwrap_or_default();
        runs.sort_by_key(|run| run.0);
        for run in runs {
            if let Some(last) = war.casualties.last_mut()
                && i64::from(run.0) <= i64::from(last.1) + 1
                && (&run.2, &run.3, &run.4, &run.5) == (&last.2, &last.3, &last.4, &last.5)
            {
                last.1 = last.1.max(run.1);
            } else {
                war.casualties.push(run);
            }
        }
    }
    check_reply(&reply)?;
    Ok(Some(reply))
}

// Two adjacent runs prove the earlier bounds. A saturated run count proves
// every earlier read changed: one observation per hour cannot produce more
// runs than elapsed hours, so these closed runs are exactly one hour each.
fn closed_counter(state: &QueryCounters, index: u64, key: u64, hour: i32) -> Option<(i32, i32)> {
    let held = state.batch.state(key)?;
    let last = held.last?;
    let first = state.first_reads[index as usize];
    if hour < first || hour >= last.first_hour {
        return None;
    }
    if held.runs == 2 && !last.break_before {
        Some((first, last.first_hour - 1))
    } else if held.runs == (i64::from(last.first_hour) - i64::from(first) + 1) as u64 {
        Some((hour, hour))
    } else {
        None
    }
}

/// Open runs are already in state. Proven closed bounds need only amounts from
/// one supplying-batch lookup per reached hour, across all groups. Other counter
/// histories read contiguous requested windows once per key, retaining only sums.
fn hydrate_losses(
    reader: &mut Reader<'_>,
    state: &QueryCounters,
    counters: &HashMap<Pair, (u64, bool)>,
    observed: &[Observed],
    span: series::Run,
    hours: &BTreeSet<i32>,
) -> io::Result<Vec<Vec<WarCasualtyRun>>> {
    let _cpu = replay_frame::cpu::scope_phase("wars:query_losses");
    let hours: Vec<_> = hours
        .range(span.first_hour..=span.last_hour)
        .copied()
        .collect();
    let mut sums =
        vec![vec![(span.first_hour, span.last_hour, [0_i128; 4]); hours.len()]; observed.len()];
    let mut windows: Vec<(i32, i32)> = Vec::new();
    for &hour in &hours {
        match windows.last_mut() {
            Some((_, last)) if last.checked_add(1) == Some(hour) => *last = hour,
            _ => windows.push((hour, hour)),
        }
    }
    let mut targets = BTreeMap::new();
    for (group, observation) in observed.iter().enumerate() {
        for &(pair, first) in &observation.loss_pairs {
            let (index, canonical) = counters[&pair];
            let order = if first == canonical {
                [0, 1, 2, 3]
            } else {
                [2, 3, 0, 1]
            };
            for (output, field) in order.into_iter().enumerate() {
                let key = 1 + index * 4 + field as u64;
                targets
                    .entry(key)
                    .or_insert_with(|| (index, Vec::new()))
                    .1
                    .push((group, output));
            }
        }
    }
    let mut pending = BTreeMap::new();
    for (key, (index, destinations)) in targets {
        let held = state.batch.state(key).and_then(|state| state.last);
        if hours.iter().all(|&hour| {
            held.is_some_and(|run| run.first_hour <= hour && hour <= run.last_hour)
                || closed_counter(state, index, key, hour).is_some()
        }) {
            let mut missing = false;
            for (at, &hour) in hours.iter().enumerate() {
                if let Some(run) =
                    held.filter(|run| run.first_hour <= hour && hour <= run.last_hour)
                {
                    for &(group, output) in &destinations {
                        let (first, last, values) = &mut sums[group][at];
                        values[output] += run.value;
                        *first = (*first).max(run.first_hour);
                        *last = (*last).min(run.last_hour);
                    }
                } else {
                    missing = true;
                }
            }
            if missing {
                pending.insert(key, (index, destinations));
            }
            continue;
        }
        for &(from, to) in &windows {
            let mut runs = reader.runs(key, from, to).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Wars counter {key} at {from}..{to}: {error}"),
                )
            })?;
            if let Some(run) = runs.first_mut()
                && run.first_hour == from
                && from > span.first_hour
                && let Some(canonical) = held
                    .filter(|run| run.first_hour <= from && from <= run.last_hour)
                    .map(|run| Ok(Some(run)))
                    .unwrap_or_else(|| reader.value_at(key, from))?
            {
                *run = canonical;
            }
            if let Some(run) = runs.last_mut()
                && run.last_hour == to
                && to < span.last_hour
                && let Some(canonical) = held
                    .filter(|run| run.first_hour <= to && to <= run.last_hour)
                    .map(|run| Ok(Some(run)))
                    .unwrap_or_else(|| reader.value_at(key, to))?
            {
                *run = canonical;
            }
            let start = hours.partition_point(|hour| *hour < from);
            let end = hours.partition_point(|hour| *hour <= to);
            for (offset, &hour) in hours[start..end].iter().enumerate() {
                let at = runs.partition_point(|run| run.first_hour <= hour);
                let run = at
                    .checked_sub(1)
                    .map(|at| runs[at])
                    .filter(|run| run.last_hour >= hour)
                    .ok_or_else(|| invalid("Missing prepared war casualty value"))?;
                for &(group, output) in &destinations {
                    let (first, last, values) = &mut sums[group][start + offset];
                    values[output] += run.value;
                    *first = (*first).max(run.first_hour);
                    *last = (*last).min(run.last_hour);
                }
            }
        }
    }
    let mut amounts = HashMap::<u64, series::Run>::new();
    for (at, &hour) in hours.iter().enumerate() {
        let keys: Vec<_> = pending
            .iter()
            .filter_map(|(&key, &(index, _))| {
                closed_counter(state, index, key, hour)
                    .filter(|_| amounts.get(&key).is_none_or(|run| hour > run.last_hour))
                    .map(|_| key)
            })
            .collect();
        if !keys.is_empty() {
            for (key, amount) in keys.iter().copied().zip(reader.values_at(&keys, hour)?) {
                let (first, last) = closed_counter(state, pending[&key].0, key, hour)
                    .expect("proven closed counter bounds");
                amounts.insert(
                    key,
                    series::Run::new(
                        first,
                        last,
                        amount.ok_or_else(|| invalid("Missing prepared war casualty value"))?,
                        true,
                    ),
                );
            }
        }
        for (&key, (_, destinations)) in &pending {
            let Some(run) = amounts
                .get(&key)
                .filter(|run| run.first_hour <= hour && hour <= run.last_hour)
            else {
                continue;
            };
            for &(group, output) in destinations {
                let (first, last, values) = &mut sums[group][at];
                values[output] += run.value;
                *first = (*first).max(run.first_hour);
                *last = (*last).min(run.last_hour);
            }
        }
    }
    Ok(sums
        .into_iter()
        .map(|rows| {
            rows.into_iter()
                .map(|(first, last, values)| {
                    let [a, b, c, d] = values.map(|value| value.to_string());
                    WarCasualtyRun(first, last, a, b, c, d)
                })
                .collect()
        })
        .collect())
}

#[cfg(test)]
mod query_tests {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;

    #[test]
    fn queries_skip_snapshots_and_structure_only_queries_skip_all_counters() {
        let fixture = Fixture::new();
        let mut state = Observer {
            snapshot: vec![1; 5000 * 16],
            ..Observer::default()
        };
        for index in 0..5000 {
            state.indices.insert(format!("relation-{index}"), index);
            state.first_reads.push(10);
            state
                .batch
                .observe(1 + index * 4, 10, Some(i128::from(index)), false);
        }
        let mut writer = fixture.writer();
        series::write_catalogue(&mut writer, "wars", 1, &state.indices, &state).unwrap();
        writer.commit().unwrap();
        let before = SNAPSHOT_BYTES.get();
        let (keys, reached) = read_query_counters(&fixture.store, &fixture.key, 1, false)
            .unwrap()
            .unwrap();
        assert_eq!(
            SNAPSHOT_BYTES.get(),
            before,
            "hydrated preparation's input snapshot"
        );
        assert_eq!(keys.len(), 5000);
        assert_eq!(reached.first_reads, state.first_reads);
        for index in 0..5000 {
            assert_eq!(
                reached.batch.state(1 + index * 4).unwrap().last,
                state.batch.state(1 + index * 4).unwrap().last
            );
        }
        let (_, structure) = read_query_counters(&fixture.store, &fixture.key, 1, true)
            .unwrap()
            .unwrap();
        assert!(structure.first_reads.is_empty());
        assert!(
            structure.batch.state(1).is_none(),
            "structure-only reply hydrated casualty counters"
        );
        assert_eq!(SNAPSHOT_BYTES.get(), before);
        let (_, full) = series::read_catalogue::<Observer>(&fixture.store, &fixture.key, "wars", 1)
            .unwrap()
            .unwrap();
        assert_eq!(SNAPSHOT_BYTES.get() - before, 5000 * 16);
        assert_eq!(full.first_reads, reached.first_reads);
    }
}
