//! Project facts append once; windows reconstruct only their selected project keys.
use super::*;
use crate::products::construction_spending::{
    ConstructionSpending, ConstructionSpendingReply, LeafProject, SpendingLeaf, SpendingUnknown,
    SpentStep,
};
use dashboard_base::prepared::{
    entries::{self, Entries},
    prefix::Prefix,
    series::{self, Accumulator, Reader},
};
use dashboard_base::products::country_groups::HourSpan;

#[derive(Clone, Serialize, Deserialize)]
enum Fact {
    Start(ConstructionProject),
    Progress(ProjectRun),
    Counts {
        built: Vec<LevelsBuilt>,
        unaccounted: Vec<LevelsBuilt>,
        possible: Vec<LevelsBuilt>,
        added: Vec<LevelsBuilt>,
    },
    End(ProjectEnd),
    Repair(i32),
    Output(ConstructionOutputRead),
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    facts: Accumulator,
    queues: Accumulator,
    members: Accumulator,
    reads: Accumulator,
    spending: Accumulator,
    /// The key the next new project takes: one per project, in order.
    next_project: u64,
    slots: Vec<u64>,
    spending_keys: BTreeMap<String, u64>,
    last_end: Vec<Option<ProjectEnd>>,
    last_run: Vec<Option<i32>>,
    first_repair: Vec<Option<i32>>,
    last_output: Vec<Option<ConstructionOutputRead>>,
    leaf_totals: BTreeMap<String, i128>,
    /// Each leaf's merges and resolutions since the last flush, made then on
    /// the leaf's published metadata in the same order.
    leaf_changes: BTreeMap<String, Vec<LeafChange>>,
    /// Every state a leaf names, each with its published name.
    named_states: BTreeSet<u32>,
    before_first: BTreeMap<String, u32>,
    output_seen: BTreeSet<usize>,
    #[serde(skip)]
    touched: BTreeSet<u64>,
    /// Each country's reply made whole at the last flush, as flushes made
    /// every reply before the catalogue held its leaves apart.
    #[cfg(test)]
    #[serde(skip)]
    whole_replies: BTreeMap<String, ConstructionSpendingReply>,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}

/// The section of a branch's construction catalogue ([`entries`]).
fn entries_section(branch: u64) -> String {
    format!("construction-entries-{branch}")
}

const MANIFEST: &[u8] = b"manifest";
const SPENDING: &[u8] = b"spending";

/// Where `tag`'s leaves are published, in key order.
fn leaf_prefix(tag: &str) -> Vec<u8> {
    [b"leaf\0".as_slice(), tag.as_bytes(), b"\0"].concat()
}

fn leaf_entry(tag: &str, name: &str) -> Vec<u8> {
    [leaf_prefix(tag).as_slice(), name.as_bytes()].concat()
}

fn before_first_entry(tag: &str) -> Vec<u8> {
    [b"tag\0".as_slice(), tag.as_bytes()].concat()
}

fn state_entry(state: u32) -> Vec<u8> {
    [b"state\0".as_slice(), &state.to_be_bytes()].concat()
}

/// What every country's spending reply shares, as the flush read it.
#[derive(Serialize, Deserialize)]
struct SharedSpending {
    reads: Vec<i32>,
    railway_reads: Vec<i32>,
    types: Vec<BuildingInfo>,
    repair_icon: Option<String>,
    unattributed: Vec<SpendingUnknown>,
    warnings: Vec<String>,
}

/// A leaf's metadata and the spending series key of its totals.
#[derive(Serialize, Deserialize)]
struct LeafEntry {
    metadata: LeafMetadata,
    key: u64,
}

/// A change to a leaf's metadata, kept until the flush makes it.
#[derive(Serialize, Deserialize)]
enum LeafChange {
    Merge(SpendingLeaf),
    Resolve(String, i32),
}

/// `tag`'s spending reply as the published catalogue holds it, without its
/// spent totals, and each leaf's spending series key; None before a flush.
/// A country with no leaf has the reply of one with no project.
fn spending_reply(
    catalogue: &mut entries::Reader<impl entries::PageSource>,
    branch: u64,
    tag: &str,
) -> io::Result<Option<(ConstructionSpendingReply, BTreeMap<String, u64>)>> {
    let Some(shared) = catalogue.get_json::<SharedSpending>(SPENDING)? else {
        return Ok(None);
    };
    let prefix = leaf_prefix(tag);
    let mut leaves = Vec::new();
    let mut keys = BTreeMap::new();
    for (entry, value) in catalogue.prefix(&prefix)? {
        let name = std::str::from_utf8(&entry[prefix.len()..])
            .map_err(|_| dashboard_core::invalid("Construction leaf name is not UTF-8"))?;
        let leaf: LeafEntry = serde_json::from_slice(&value)?;
        leaves.push(leaf.metadata.materialize(name)?);
        keys.insert(name.to_owned(), leaf.key);
    }
    leaves.sort_by_key(|leaf| {
        (
            leaf.repair,
            leaf.kind == ProjectKind::Railway,
            leaf.building,
            leaf.state,
            leaf.to_state,
        )
    });
    let states: BTreeSet<u32> = leaves
        .iter()
        .flat_map(|leaf| [leaf.state, leaf.to_state])
        .flatten()
        .collect();
    let states = states
        .into_iter()
        .map(|state| {
            catalogue
                .get_json::<StateName>(&state_entry(state))?
                .ok_or_else(|| dashboard_core::invalid("Construction state name is not published"))
        })
        .collect::<io::Result<_>>()?;
    let mut warnings = shared.warnings;
    if leaves.is_empty() && !(shared.reads.is_empty() && shared.railway_reads.is_empty()) {
        warnings.push(format!("{tag}: no recorded construction in this branch"));
    }
    let reply = ConstructionSpendingReply {
        branch_id: branch.to_string(),
        tag: tag.to_owned(),
        cost_scale: COST_SCALE,
        reads: shared.reads,
        railway_reads: shared.railway_reads,
        states,
        types: shared.types,
        repair_icon: shared.repair_icon,
        leaves,
        before_first_read: catalogue
            .get_json::<u32>(&before_first_entry(tag))?
            .unwrap_or(0),
        unattributed: shared.unattributed,
        warnings,
    };
    Ok(Some((reply, keys)))
}

fn leaf_key(tag: &str, leaf: &SpendingLeaf) -> io::Result<String> {
    Ok(serde_json::to_string(&(
        tag,
        leaf.repair,
        leaf.kind,
        leaf.building,
        leaf.state,
        leaf.to_state,
    ))?)
}

impl Prepared {
    pub fn restore_frame(&mut self, frame: &DecodedFrame) {
        if self.scan.last == Some(frame.hour) {
            self.scan.restore_frame(frame);
        }
    }

    /// The last hour the scan observed.
    pub fn last_hour(&self) -> Option<i32> {
        self.scan.last_hour()
    }

    /// The last observed hour (`frame`) read again from its block's keyframe,
    /// before observation goes on from that read ([`Scan::rebase_dictionary`]).
    pub fn rebase(&mut self, frame: &DecodedFrame) {
        if self.scan.last_hour() == Some(frame.hour) {
            self.scan.rebase_dictionary(frame);
        }
    }

    #[cfg(test)]
    pub fn observe(
        &mut self,
        frame: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        source: &Source,
    ) -> io::Result<()> {
        let changes = dashboard_base::prepared::frame_changes::FrameChanges {
            rows_changed: [true; replay_data::rows::TABLE_COUNT],
            table_read: [true; replay_data::rows::TABLE_COUNT],
            slot_read: std::array::from_fn(|index| {
                slot_read(format, &frame.metrics, TableId::ALL[index], frame.hour)
            }),
            updates: None,
        };
        self.observe_changed(frame, format, source, &changes)
    }

    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        source: &Source,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_changed_in(frame, format, source.block_windows(frame.branch), changes)
    }

    /// [`Self::observe_changed`] with the branch's replay blocks (`windows`,
    /// as [`Source::block_windows`] lists them) instead of the source.
    pub fn observe_changed_in(
        &mut self,
        frame: &DecodedFrame,
        format: replay_data::format::FormatVersion,
        windows: &[HourSpan],
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        let consecutive = self.scan.last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.facts.advance(frame.hour)?;
        self.queues.advance(frame.hour)?;
        self.members.advance(frame.hour)?;
        self.reads.advance(frame.hour)?;
        self.spending.advance(frame.hour)?;
        if !consecutive {
            for (name, total) in &self.leaf_totals {
                self.spending
                    .observe(self.spending_keys[name], frame.hour, Some(*total), true);
            }
        }
        self.scan.observe_changed(frame, format, changes);
        if [TableId::BuildingConstruction, TableId::RailwayConstruction]
            .into_iter()
            .any(|table| changes.slot_read[table.index()])
        {
            self.scan.sync_reads();
        }
        self.project_facts(frame.hour, windows)
    }

    /// Hours `first..=last` that follow the last observed (`frame`), at which
    /// no field the scan reads changed and no queue or output read is dated:
    /// each advances the series' accepted hours and the scan's last hour, as
    /// observing it does, in constant time. False when the last hour left
    /// something to project, which only observation does.
    pub fn advance_quiet(
        &mut self,
        first: i32,
        last: i32,
        frame: &DecodedFrame,
    ) -> io::Result<bool> {
        if self.scan.last_hour().and_then(|hour| hour.checked_add(1)) != Some(first)
            || !self.scan.settled()
        {
            return Ok(false);
        }
        for series in [
            &mut self.facts,
            &mut self.queues,
            &mut self.members,
            &mut self.reads,
            &mut self.spending,
        ] {
            series.advance_span(first, last)?;
        }
        self.scan.advance_quiet(last, frame);
        Ok(true)
    }

    fn project_facts(&mut self, hour: i32, windows: &[HourSpan]) -> io::Result<()> {
        let dirty = std::mem::take(&mut self.scan.dirty);
        if dirty.is_empty() && self.scan.log.output_reads.is_empty() {
            return Ok(());
        }
        for &index in &dirty {
            if index >= self.slots.len() {
                let key = self.next_project;
                self.next_project += 1;
                self.slots.push(key);
                self.last_run.push(None);
                self.last_end.push(None);
                self.first_repair.push(None);
                self.last_output.push(None);
            }
        }
        let mut output_steps = BTreeMap::new();
        for &index in &dirty {
            let mut output: Vec<_> = self.scan.log.outputs[index]
                .iter()
                .filter(|read| read.known_at == hour)
                .cloned()
                .collect();
            for read in &mut output {
                // A read without a row cannot say whether earlier work went unobserved.
                if read.since_hour.is_some()
                    && self.output_seen.insert(index)
                    && !read.observed_since_creation
                {
                    let tag = &self.scan.log.projects[index].tag;
                    *self.before_first.entry(tag.clone()).or_default() += 1;
                }
                let before = self.last_output.get(index).and_then(Option::as_ref);
                for (raw, previous) in [
                    (
                        &mut read.construction_raw,
                        before.and_then(|read| read.construction_raw.as_deref()),
                    ),
                    (
                        &mut read.repair_raw,
                        before.and_then(|read| read.repair_raw.as_deref()),
                    ),
                ] {
                    if let Some(value) = raw {
                        let amount = value
                            .parse::<i128>()
                            .map_err(|_| dashboard_core::invalid("Malformed construction IC"))?
                            - previous.unwrap_or("0").parse::<i128>().map_err(|_| {
                                dashboard_core::invalid("Malformed construction IC")
                            })?;
                        *value = amount.to_string();
                    }
                }
                read.repair_ticks =
                    i32::from(read.repair_ticks > before.map_or(0, |read| read.repair_ticks));
            }
            output_steps.insert(index, output);
        }
        for (tag, (leaves, _)) in crate::products::construction_spending::all_leaves_for(
            &self.scan.log,
            dirty.iter().map(|&index| {
                (
                    &self.scan.log.projects[index],
                    output_steps[&index].as_slice(),
                )
            }),
        ) {
            for mut leaf in leaves {
                let name = leaf_key(&tag, &leaf)?;
                let next = self.spending_keys.len() as u64;
                self.spending_keys.entry(name.clone()).or_insert(next);
                let total = self.leaf_totals.entry(name.clone()).or_default();
                let previous = *total;
                if let Some(step) = leaf.spent.last() {
                    *total =
                        total
                            .checked_add(step.total_raw.parse().map_err(|_| {
                                dashboard_core::invalid("Malformed construction IC")
                            })?)
                            .ok_or_else(|| {
                                dashboard_core::invalid("Construction IC exceeds its exact range")
                            })?;
                }
                if previous != *total || self.spending.state(self.spending_keys[&name]).is_none() {
                    self.spending
                        .observe(self.spending_keys[&name], hour, Some(*total), false);
                    #[cfg(test)]
                    {
                        self.work.emitted += 1;
                    }
                }
                leaf.spent.clear();
                self.leaf_changes
                    .entry(name)
                    .or_default()
                    .push(LeafChange::Merge(leaf));
            }
        }
        // Only this project's two leaves can have an unresolved output interval.
        for &index in &dirty {
            let project = &self.scan.log.projects[index];
            if output_steps[&index]
                .iter()
                .any(|read| read.construction_raw.is_some())
            {
                for repair in [false, true] {
                    let name = serde_json::to_string(&(
                        project.tag.as_str(),
                        repair,
                        project.kind,
                        project.building,
                        project.state,
                        project
                            .to_state
                            .filter(|_| project.kind == ProjectKind::Railway),
                    ))?;
                    // A leaf has metadata once it has a spending key.
                    if self.spending_keys.contains_key(&name) {
                        self.leaf_changes
                            .entry(name)
                            .or_default()
                            .push(LeafChange::Resolve(project.id.clone(), hour));
                    }
                }
            }
        }
        let window_of = |hour| {
            windows
                .partition_point(|window| window.first_hour <= hour)
                .saturating_sub(1) as u64
        };
        for index in dirty {
            #[cfg(test)]
            {
                self.work.history_visits += 1;
            }
            let project = &mut self.scan.log.projects[index];
            let key = self.slots[index];
            let new = self.last_run[index].is_none();
            let mut dates = Vec::new();
            if new {
                let mut start = project.clone();
                start.built.clear();
                start.unaccounted.clear();
                start.possible.clear();
                start.added.clear();
                start.runs.clear();
                start.end = None;
                start.first_repair_tick = None;
                self.facts.event(key, hour, &Fact::Start(start))?;
                dates.extend([project.first_read, project.created_hour]);
                if project.created_hour > project.first_read {
                    for window in window_of(project.first_read)..=window_of(project.created_hour) {
                        self.members.event(window, hour, &key)?;
                    }
                }
            }
            if let Some(run) = project.runs.last() {
                let changed = self.last_run[index] != Some(run.first_hour);
                if changed {
                    self.facts.event(key, hour, &Fact::Progress(run.clone()))?;
                    self.last_run[index] = Some(run.first_hour);
                }
                if changed || run.last_hour != hour {
                    self.queues.observe(
                        key,
                        hour,
                        (run.last_hour == hour).then_some(i128::from(run.first_hour)),
                        run.first_hour == hour,
                    );
                }
                if run.last_hour == hour {
                    self.touched.insert(key);
                }
            }
            for values in [&project.built, &project.unaccounted, &project.possible] {
                dates.extend(
                    values
                        .iter()
                        .map(|value| value.hour.unwrap_or(value.known_at)),
                );
            }
            if !project.built.is_empty()
                || !project.unaccounted.is_empty()
                || !project.possible.is_empty()
                || !project.added.is_empty()
            {
                self.facts.event(
                    key,
                    hour,
                    &Fact::Counts {
                        built: project.built.clone(),
                        unaccounted: project.unaccounted.clone(),
                        possible: project.possible.clone(),
                        added: project.added.clone(),
                    },
                )?;
            }
            if project.end != self.last_end[index] {
                if let Some(end) = &project.end {
                    self.facts.event(key, hour, &Fact::End(end.clone()))?;
                    dates.extend([end.known_at, end.hour.unwrap_or(end.known_at)]);
                }
                self.last_end[index] = project.end.clone();
            }
            if project.first_repair_tick != self.first_repair[index] {
                if let Some(hour) = project.first_repair_tick {
                    self.facts.event(key, hour, &Fact::Repair(hour))?;
                }
                self.first_repair[index] = project.first_repair_tick;
            }
            for window in dates.into_iter().map(window_of).collect::<BTreeSet<_>>() {
                self.members.event(window, hour, &key)?;
            }
            let output = &mut self.scan.log.outputs[index];
            for read in output.iter().filter(|read| read.known_at == hour) {
                self.facts.event(key, hour, &Fact::Output(read.clone()))?;
                self.touched.insert(key);
                if read.construction_raw.is_some() {
                    self.last_output[index] = Some(read.clone());
                }
            }
            output.clear();
            project.built.clear();
            project.unaccounted.clear();
            project.possible.clear();
            project.added.clear();
            if project.runs.len() > 2 {
                project.runs.drain(..project.runs.len() - 2);
            }
        }
        for &(at, reason) in self
            .scan
            .log
            .output_reads
            .iter()
            .filter(|(at, _)| *at == hour)
        {
            self.reads.event(0, at, &reason)?;
        }
        self.scan.log.output_reads.clear();
        for values in [
            &mut self.scan.buildings.reads,
            &mut self.scan.railways.reads,
            &mut self.scan.log.reads,
            &mut self.scan.log.railway_reads,
        ] {
            if values.len() > 2 {
                values.drain(..values.len() - 2);
            }
        }
        self.scan.log.repair_predecessors.clear();
        Ok(())
    }

    pub fn end_block(
        &mut self,
        frame_branch: u64,
        last_hour: i32,
        source: &Source,
    ) -> io::Result<()> {
        self.end_block_in(last_hour, source.block_windows(frame_branch))
    }

    /// [`Self::end_block`] with the branch's replay blocks instead of the source.
    pub fn end_block_in(&mut self, last_hour: i32, windows: &[HourSpan]) -> io::Result<()> {
        self.scan.seal();
        if let Some(hour) = self.scan.last {
            self.project_facts(hour, windows)?;
        }
        let window = windows
            .partition_point(|window| window.first_hour <= last_hour)
            .saturating_sub(1) as u64;
        for key in std::mem::take(&mut self.touched) {
            self.members.event(window, last_hour, &key)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn work(&self) -> dashboard_base::prepared::observer_tests::Work {
        let mut work = self.work;
        work.row_visits += self.scan.work.row_visits;
        work.history_visits += self.scan.work.history_visits;
        work.history_visits += self.scan.buildings.watch.visits + self.scan.railways.watch.visits;
        work
    }

    pub fn bytes(&self) -> usize {
        self.facts.bytes()
            + self.queues.bytes()
            + self.members.bytes()
            + self.reads.bytes()
            + self.spending.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_in(
            writer,
            source.replay_ref(),
            source.block_windows(branch),
            &NameSource::of(source, context),
            branch,
            cancelled,
        )
    }

    /// [`Self::flush`] with the recording, the branch's replay blocks and the
    /// source's naming handles instead of the source.
    pub fn flush_in(
        &mut self,
        writer: &mut store::Writer,
        replay: &Replay,
        block_windows: &[HourSpan],
        names: &NameSource,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut payloads = series::Payloads::default();
        self.scan.seal();
        if let Some(hour) = self.scan.last {
            self.project_facts(hour, block_windows)?;
        }
        // That projection repeats the hour the block's end already published:
        // its touched keys belong to that block's window, not the next one.
        self.touched.clear();
        self.facts.flush_with_payloads(
            writer,
            &format!("construction-facts-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.queues.flush_with_payloads(
            writer,
            &format!("construction-queues-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.members.flush_with_payloads(
            writer,
            &format!("construction-members-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.reads.flush_with_payloads(
            writer,
            &format!("construction-reads-{branch}"),
            0,
            cancelled,
            &mut payloads,
        )?;
        self.spending.flush_with_payloads(
            writer,
            &format!("construction-spending-{branch}"),
            5,
            cancelled,
            &mut payloads,
        )?;
        let mut warnings = Vec::new();
        let names = names.names(branch, &self.scan.log, cancelled, &mut warnings)?;
        scan_warnings(&self.scan.log, &mut warnings);
        let manifest = ConstructionLogManifestReply {
            newest: None,
            cost_scale: COST_SCALE,
            windows: windows(replay, branch),
            recorded: !(self.scan.log.reads.is_empty() && self.scan.log.railway_reads.is_empty()),
            types: names.types.clone(),
            repair_icon: names.repair_icon.clone(),
            warnings: warnings.clone(),
        };
        let section = entries_section(branch);
        let mut catalogue = Entries::default();
        catalogue.put(MANIFEST, &(manifest, self.scan.last))?;
        let template = ConstructionSpending::reply_from_parts(
            &dashboard_base::products::CountryParams {
                branch_id: branch.to_string(),
                tag: String::new(),
            },
            &self.scan.log,
            &names,
            Vec::new(),
            0,
            Vec::new(),
        );
        #[cfg(test)]
        let whole_warnings = warnings.clone();
        catalogue.put(
            SPENDING,
            &SharedSpending {
                reads: template.reads,
                railway_reads: template.railway_reads,
                types: template.types,
                repair_icon: template.repair_icon,
                unattributed: template.unattributed,
                warnings,
            },
        )?;
        // Only the leaves changed since the last flush are read and written.
        {
            let mut stored = entries::read_written(writer, &section);
            for (name, changes) in std::mem::take(&mut self.leaf_changes) {
                let (tag, _, _, _, state, to_state): (
                    String,
                    bool,
                    ProjectKind,
                    u32,
                    Option<u32>,
                    Option<u32>,
                ) = serde_json::from_str(&name)?;
                let entry = leaf_entry(&tag, &name);
                let mut metadata = stored
                    .get_json::<LeafEntry>(&entry)?
                    .map(|leaf| leaf.metadata);
                for change in changes {
                    match change {
                        LeafChange::Merge(leaf) => {
                            merge_leaf(metadata.get_or_insert_with(Default::default), leaf)
                        }
                        LeafChange::Resolve(project, hour) => {
                            if let Some(metadata) = &mut metadata {
                                metadata.resolve(&project, hour);
                            }
                        }
                    }
                }
                if let Some(metadata) = metadata {
                    self.named_states.extend(state.into_iter().chain(to_state));
                    catalogue.put(
                        entry,
                        &LeafEntry {
                            metadata,
                            key: self.spending_keys[&name],
                        },
                    )?;
                }
            }
        }
        for (tag, count) in &self.before_first {
            catalogue.put(before_first_entry(tag), count)?;
        }
        for state in names.states(&self.named_states) {
            catalogue.put(state_entry(state.id), &state)?;
        }
        catalogue.flush(writer, &section)?;
        #[cfg(test)]
        {
            let mut countries = BTreeMap::<String, Vec<SpendingLeaf>>::new();
            for (entry, value) in entries::read_written(writer, &section).prefix(b"leaf\0")? {
                let entry = std::str::from_utf8(&entry[b"leaf\0".len()..]).unwrap();
                let (tag, name) = entry.split_once('\0').unwrap();
                let leaf: LeafEntry = serde_json::from_slice(&value)?;
                countries
                    .entry(tag.to_owned())
                    .or_default()
                    .push(leaf.metadata.materialize(name)?);
            }
            let tags: BTreeSet<String> = countries
                .keys()
                .chain(self.before_first.keys())
                .cloned()
                .chain(["ZZZ".to_owned()])
                .collect();
            self.whole_replies.clear();
            for tag in tags {
                let mut leaves = countries.remove(&tag).unwrap_or_default();
                leaves.sort_by_key(|leaf| {
                    (
                        leaf.repair,
                        leaf.kind == ProjectKind::Railway,
                        leaf.building,
                        leaf.state,
                        leaf.to_state,
                    )
                });
                let params = dashboard_base::products::CountryParams {
                    branch_id: branch.to_string(),
                    tag: tag.clone(),
                };
                let reply = ConstructionSpending::reply_from_parts(
                    &params,
                    &self.scan.log,
                    &names,
                    leaves,
                    self.before_first.get(&tag).copied().unwrap_or(0),
                    whole_warnings.clone(),
                );
                self.whole_replies.insert(tag, reply);
            }
        }
        let remap = self.scan.compact_projects();
        self.slots = retain_slots(std::mem::take(&mut self.slots), &remap);
        self.last_run = retain_slots(std::mem::take(&mut self.last_run), &remap);
        self.last_end = retain_slots(std::mem::take(&mut self.last_end), &remap);
        self.first_repair = retain_slots(std::mem::take(&mut self.first_repair), &remap);
        self.last_output = retain_slots(std::mem::take(&mut self.last_output), &remap);
        self.output_seen = std::mem::take(&mut self.output_seen)
            .into_iter()
            .filter_map(|index| remap.get(&index).copied())
            .collect();
        Ok(())
    }
}

fn retain_slots<T>(values: Vec<T>, remap: &BTreeMap<usize, usize>) -> Vec<T> {
    values
        .into_iter()
        .enumerate()
        .filter_map(|(index, value)| remap.contains_key(&index).then_some(value))
        .collect()
}

#[derive(Default, Serialize, Deserialize)]
struct LeafMetadata {
    #[serde(with = "dashboard_core::map_pairs")]
    projects: BTreeMap<(String, Option<i32>, i32), LeafProject>,
    #[serde(with = "dashboard_core::map_pairs")]
    unknown: BTreeMap<(String, i32), SpendingUnknown>,
}

impl LeafMetadata {
    fn resolve(&mut self, project: &str, hour: i32) {
        if let Some((_, gap)) = self
            .unknown
            .range_mut((project.to_owned(), i32::MIN)..=(project.to_owned(), i32::MAX))
            .next_back()
            && gap.until.is_none()
        {
            gap.until = Some(hour);
        }
    }

    fn materialize(&self, name: &str) -> io::Result<SpendingLeaf> {
        let (_, repair, kind, building, state, to_state): (
            String,
            bool,
            ProjectKind,
            u32,
            Option<u32>,
            Option<u32>,
        ) = serde_json::from_str(name)?;
        let mut projects: Vec<_> = self.projects.values().cloned().collect();
        let mut unknown: Vec<_> = self.unknown.values().cloned().collect();
        projects.sort_by(|a, b| {
            (a.first_read, a.until, &a.project).cmp(&(b.first_read, b.until, &b.project))
        });
        unknown.sort_by(|a, b| (a.known_at, &a.project).cmp(&(b.known_at, &b.project)));
        Ok(SpendingLeaf {
            repair,
            kind,
            building,
            state,
            to_state,
            spent: Vec::new(),
            unknown,
            projects,
        })
    }
}

fn merge_leaf(previous: &mut LeafMetadata, incoming: SpendingLeaf) {
    for span in incoming.projects {
        let lower = (span.project.clone(), span.known_at, i32::MIN);
        let upper = (span.project.clone(), span.known_at, span.first_read);
        let before = previous
            .projects
            .range_mut(lower..=upper)
            .next_back()
            .map(|(_, before)| before);
        if let Some(before) = before.filter(|before| {
            before.first_read == span.first_read
                || before.until.is_none()
                || before.until.is_some_and(|until| span.first_read < until)
        }) {
            let first = before.first_read;
            *before = span;
            before.first_read = first;
        } else {
            previous
                .projects
                .insert((span.project.clone(), span.known_at, span.first_read), span);
        }
    }
    for gap in incoming.unknown {
        if let Some((_, before)) = previous
            .unknown
            .range_mut((gap.project.clone(), i32::MIN)..=(gap.project.clone(), i32::MAX))
            .next_back()
            && before.until.is_none()
        {
            if before.reason == gap.reason {
                before.until = gap.until;
                continue;
            }
            before.until = Some(gap.known_at);
        }
        previous
            .unknown
            .insert((gap.project.clone(), gap.known_at), gap);
    }
}

fn events(
    reader: &mut Reader<'_>,
    keys: &[u64],
    first: i32,
    last: i32,
) -> io::Result<Vec<series::DatedRow>> {
    let mut cursor = 0;
    let mut rows = Vec::new();
    loop {
        let page = reader.events(keys, first, last, cursor, u32::MAX)?;
        rows.extend(page.rows);
        if let Some(next) = page.cursor {
            cursor = next;
        } else {
            break;
        }
    }
    Ok(rows)
}

fn project_log(
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    selected: &[u64],
    cancelled: &AtomicBool,
) -> io::Result<ScannedLog> {
    let mut facts = Reader::new(
        store,
        key,
        format!("construction-facts-{branch}"),
        cancelled,
    )?;
    let mut queues = Reader::new(
        store,
        key,
        format!("construction-queues-{branch}"),
        cancelled,
    )?;
    let mut log = ScannedLog::default();
    for &project_key in selected {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut project = None;
        let mut output = Vec::new();
        for row in events(&mut facts, &[project_key], i32::MIN, i32::MAX)? {
            match serde_json::from_value::<Fact>(row.row)? {
                Fact::Start(value) => project = Some(value),
                Fact::Progress(value) => project
                    .as_mut()
                    .ok_or_else(|| dashboard_core::invalid("Construction start fact missing"))?
                    .runs
                    .push(value),
                Fact::Counts {
                    built,
                    unaccounted,
                    possible,
                    added,
                } => {
                    let project = project.as_mut().ok_or_else(|| {
                        dashboard_core::invalid("Construction start fact missing")
                    })?;
                    project.built.extend(built);
                    project.unaccounted.extend(unaccounted);
                    project.possible.extend(possible);
                    project.added.extend(added);
                }
                Fact::End(value) => {
                    project
                        .as_mut()
                        .ok_or_else(|| dashboard_core::invalid("Construction start fact missing"))?
                        .end = Some(value)
                }
                Fact::Repair(hour) => {
                    project
                        .as_mut()
                        .ok_or_else(|| dashboard_core::invalid("Construction start fact missing"))?
                        .first_repair_tick = Some(hour)
                }
                Fact::Output(value) => {
                    if let Some(reason) = value.reason
                        && let Some(project) = &project
                    {
                        log.output_reasons.insert((reason, project.id.clone()));
                    }
                    output.push(value);
                }
            }
        }
        if let Some(mut project) = project {
            for run in queues.runs(project_key, i32::MIN, i32::MAX)? {
                if let Some(value) = project
                    .runs
                    .iter_mut()
                    .find(|value| i128::from(value.first_hour) == run.value)
                {
                    value.last_hour = value.last_hour.max(run.last_hour);
                }
            }
            log.projects.push(project);
            log.outputs.push(output);
        }
    }
    // Keys preserve the scan's first-sighting order.
    Ok(log)
}

pub fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
    content: Option<&dashboard_core::config::ContentOptions>,
    cancelled: &AtomicBool,
) -> io::Result<Option<serde_json::Value>> {
    let branch: u64 = params["branchId"]
        .as_str()
        .ok_or_else(|| dashboard_core::invalid("Missing construction branch"))?
        .parse()
        .map_err(|_| dashboard_core::invalid("Invalid construction branch"))?;
    let (store, key) = dashboard_base::prepared::files(source, content)?;
    // Construction prepared on its own answers from its own coverage.
    let coverage = dashboard_base::prepared::prefix::read_or_empty(
        source, content, product, branch, None, cancelled,
    )?
    .coverage;
    let section = entries_section(branch);
    let spending_tag = (product == "construction-spending")
        .then(|| params["tag"].as_str())
        .flatten()
        .filter(|tag| dashboard_core::protocol::validate_tag(tag).is_ok());
    // A spending reply is composed from the publication its manifest comes from.
    let (published, spending) = entries::compose(&store, &key, &section, cancelled, |catalogue| {
        let manifest =
            catalogue.get_json::<(ConstructionLogManifestReply, Option<i32>)>(MANIFEST)?;
        let spending = match spending_tag {
            Some(tag) => spending_reply(catalogue, branch, tag)?,
            None => None,
        };
        Ok((manifest, spending))
    })?
    .unwrap_or((None, None));
    let (mut manifest, last) = published.unwrap_or_else(|| {
        (
            ConstructionLogManifestReply {
                cost_scale: COST_SCALE,
                windows: windows(source.replay_ref(), branch),
                newest: None,
                recorded: false,
                types: Vec::new(),
                repair_icon: None,
                warnings: Vec::new(),
            },
            None,
        )
    });
    // Before coverage completes, the log answers its prepared prefix: the
    // manifest the prefix published (its building types and warnings so far)
    // and each window the prefix has reached, whose projects' later history
    // is still being prepared. A separately proven newest window can answer
    // past that prefix; every other later window waits.
    let mut prefix = Prefix::new(source, branch, coverage.clone(), None);
    if product == "construction-log-manifest" {
        // The ordinary manifest answers immediately. A separate request can
        // prove the newest window while the widget already shows its prefix.
        if !coverage.complete
            && params["newest"].as_bool() == Some(true)
            && let Some((window, recorded)) =
                super::newest::query(source, branch, &manifest.types, cancelled)?
        {
            manifest.recorded |= recorded;
            prefix.answered(&[HourSpan {
                first_hour: window.interval.first_hour,
                last_hour: window.interval.last_hour,
            }]);
            manifest.newest = Some(window.interval);
        }
        super::present_stored(
            source,
            content,
            &mut manifest.types,
            Some(&mut manifest.repair_icon),
            cancelled,
        )?;
        let mut data = serde_json::to_value(manifest)?;
        prefix.mark_revisable(source, branch, &mut data)?;
        return Ok(Some(data));
    }
    if product == "construction-log" {
        let params: ConstructionLogParams = serde_json::from_value(params)?;
        ConstructionLog::validate(&params).map_err(dashboard_core::invalid)?;
        if params.window >= manifest.windows.len() {
            return Err(dashboard_core::invalid(
                "Construction log window not in recording",
            ));
        }
        if !coverage.complete && manifest.windows[params.window].last_hour > coverage.last_hour {
            if params.window + 1 == manifest.windows.len()
                && let Some((mut window, _)) =
                    super::newest::query(source, branch, &manifest.types, cancelled)?
            {
                if let Some(types) = &mut window.types {
                    super::present_stored(source, content, types, None, cancelled)?;
                }
                prefix.answered(&[HourSpan {
                    first_hour: window.interval.first_hour,
                    last_hour: window.interval.last_hour,
                }]);
                dashboard_base::products::check_reply_budget(&window)?;
                let mut data = serde_json::to_value(window)?;
                prefix.mark(&mut data)?;
                return Ok(Some(data));
            }
            let mut data = serde_json::to_value(ConstructionLogReply {
                window: params.window,
                interval: manifest.windows[params.window].clone(),
                types: None,
                exact_from: None,
                output_reads: Vec::new(),
                output_unavailable: Vec::new(),
                projects: Vec::new(),
                outputs: Vec::new(),
                warnings: Vec::new(),
            })?;
            prefix.mark(&mut data)?;
            return Ok(Some(data));
        }
        let mut members = Reader::new(
            &store,
            &key,
            format!("construction-members-{branch}"),
            cancelled,
        )?;
        let selected: BTreeSet<u64> =
            events(&mut members, &[params.window as u64], i32::MIN, i32::MAX)?
                .into_iter()
                .map(|row| serde_json::from_value(row.row))
                .collect::<Result<_, _>>()?;
        let mut log = project_log(
            &store,
            &key,
            branch,
            &selected.into_iter().collect::<Vec<_>>(),
            cancelled,
        )?;
        let interval = &manifest.windows[params.window];
        let next = manifest
            .windows
            .get(params.window + 1)
            .map_or(i32::MAX, |window| window.first_hour.saturating_sub(1));
        let mut reads = Reader::new(
            &store,
            &key,
            format!("construction-reads-{branch}"),
            cancelled,
        )?;
        if let Some((hour, reason)) =
            reads.latest_event(0, interval.first_hour.saturating_sub(1))?
        {
            log.output_reads
                .push((hour, serde_json::from_value(reason)?));
        }
        for row in events(&mut reads, &[0], interval.first_hour, next)? {
            log.output_reads
                .push((row.hour, serde_json::from_value(row.row)?));
        }
        let mut reply = Cut::new(Arc::new(log), manifest.windows).reply(params.window);
        if let Some(types) = &mut reply.types {
            super::present_stored(source, content, types, None, cancelled)?;
        }
        dashboard_base::products::check_reply_budget(&reply)?;
        let mut data = serde_json::to_value(reply)?;
        prefix.mark_revisable(source, branch, &mut data)?;
        return Ok(Some(data));
    }
    if product == "construction-spending" {
        let tag = params["tag"]
            .as_str()
            .ok_or_else(|| dashboard_core::invalid("Missing construction country"))?;
        dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
        let hours: Vec<i32> = serde_json::from_value(params["hours"].clone())?;
        // Spending is cumulative factory output per leaf: before coverage
        // completes, an hour after the prepared prefix reads the total
        // through the prefix's last hour, and every leaf's output after it is
        // still being prepared.
        let prefix = Prefix::new(
            source,
            branch,
            coverage.clone(),
            hours.iter().max().copied(),
        );
        let hours: BTreeSet<i32> = hours
            .into_iter()
            .map(|hour| prefix.through(Some(hour)).unwrap_or(hour))
            .collect();
        let (mut reply, keys) = spending.unwrap_or_else(|| {
            (
                ConstructionSpendingReply {
                    branch_id: branch.to_string(),
                    tag: tag.to_owned(),
                    cost_scale: COST_SCALE,
                    reads: Vec::new(),
                    railway_reads: Vec::new(),
                    types: Vec::new(),
                    repair_icon: None,
                    states: Vec::new(),
                    leaves: Vec::new(),
                    before_first_read: 0,
                    unattributed: Vec::new(),
                    warnings: Vec::new(),
                },
                BTreeMap::new(),
            )
        });
        if let Some(missing) = last.and_then(|last| last.checked_add(1))
            && missing <= coverage.last_hour
        {
            reply
                .unattributed
                .push(crate::products::construction_spending::SpendingUnknown {
                    known_at: missing,
                    reason: SpendingGap::NotRecorded,
                    project: String::new(),
                    until: None,
                });
        }
        let mut reader = Reader::new(
            &store,
            &key,
            format!("construction-spending-{branch}"),
            cancelled,
        )?;
        for leaf in &mut reply.leaves {
            leaf.spent.clear();
            if let Some(&key) = keys.get(&leaf_key(tag, leaf)?) {
                for &hour in &hours {
                    if let Some(run) = reader.value_at(key, hour)? {
                        leaf.spent.push(SpentStep {
                            known_at: hour,
                            total_raw: run.value.to_string(),
                        });
                    }
                }
            }
            if let Some(pending) = prefix.pending.first() {
                leaf.unknown.push(SpendingUnknown {
                    known_at: pending.first_hour,
                    reason: SpendingGap::Preparing,
                    project: String::new(),
                    until: None,
                });
            }
        }
        super::present_stored(
            source,
            content,
            &mut reply.types,
            Some(&mut reply.repair_icon),
            cancelled,
        )?;
        let mut data = serde_json::to_value(reply)?;
        // Inferred work intervals and previously unresolved building names
        // can be repaired by later reads; the prefix's presentation is provisional.
        prefix.mark_revisable(source, branch, &mut data)?;
        return Ok(Some(data));
    }
    Err(dashboard_core::invalid("Unknown construction query"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::construction_log::tests::{
        CIVILIAN, building_at, building_line, construction_frame,
    };
    use dashboard_core::test_fixtures::{CAMPAIGN, records};
    use replay_data::{rows::ConstructionLineOutputRow, rows::TableRows};
    use replay_frame::frame::TableView;

    /// `name`'s leaf of `tag` as branch 1's catalogue publishes it.
    fn published_leaf(
        store: &store::Store,
        key: &store::SourceKey,
        tag: &str,
        name: &str,
    ) -> SpendingLeaf {
        let section = entries_section(1);
        entries::read(store, key, &section)
            .unwrap()
            .unwrap()
            .get_json::<LeafEntry>(&leaf_entry(tag, name))
            .unwrap()
            .unwrap()
            .metadata
            .materialize(name)
            .unwrap()
    }

    fn source_for(frames: &[DecodedFrame]) -> Source {
        let replay = Arc::new(
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap(),
        );
        Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "construction-classes".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay,
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    fn life_frame(hour: i32, id: u64, open: bool) -> DecodedFrame {
        let mut frame = construction_frame(
            hour,
            true,
            if open {
                vec![building_line(id, 40, 1, 0, 3)]
            } else {
                Vec::new()
            },
            Vec::new(),
            Some(vec![building_at(40, CIVILIAN, (0, 0))]),
            Vec::new(),
        );
        *frame.tables[TableId::ConstructionLineOutput.index()] =
            TableView::Complete(TableRows::ConstructionLineOutput(vec![
                ConstructionLineOutputRow {
                    line: id,
                    since_hour: hour - i32::from(!open),
                    construction_raw: 100,
                    observed_since_creation: true,
                    exact: true,
                    ended: !open,
                    completion: replay_data::rows::Recorded::Value(None),
                    ..Default::default()
                },
            ]));
        frame
    }

    #[test]
    fn construction_observer_work_is_flat_over_carried_hours_and_closed_project_history() {
        use dashboard_base::prepared::{
            frame_changes::FrameChangeTracker,
            observer_tests::{self, Work},
        };
        let first = life_frame(100, 1, true);
        let source = source_for(std::slice::from_ref(&first));
        let format = source.replay_ref().format;
        observer_tests::assert_flat(
            observer_tests::scaled_work(1, |hours| {
                let mut prepared = Prepared::default();
                let mut tracker = FrameChangeTracker::new();
                prepared.observe_changed(
                    &first,
                    format,
                    &source,
                    &tracker.observe(&first, format),
                )?;
                let mut carried = first.clone();
                carried.metrics.table_ns.fill(0);
                observer_tests::unchanged_frames(
                    prepared.work(),
                    &carried,
                    hours as i32,
                    format,
                    |frame, changes| {
                        prepared.observe_changed(frame, format, &source, changes)?;
                        Ok(prepared.work())
                    },
                )
            })
            .unwrap(),
        );
        observer_tests::assert_flat(
            observer_tests::scaled_work(10, |history| {
                let frames: Vec<_> = (0..history)
                    .flat_map(|index| {
                        [
                            life_frame(100 + 2 * index as i32, index as u64 + 1, true),
                            life_frame(101 + 2 * index as i32, index as u64 + 1, false),
                        ]
                    })
                    .collect();
                let source = source_for(&frames);
                let format = source.replay_ref().format;
                let mut tracker = FrameChangeTracker::new();
                let mut prepared = Prepared::default();
                for frame in &frames {
                    prepared.observe_changed(
                        frame,
                        format,
                        &source,
                        &tracker.observe(frame, format),
                    )?;
                }
                assert_eq!(prepared.next_project, history as u64);
                let before = prepared.work();
                let mut changed = frames.last().unwrap().clone();
                changed.hour += 1;
                changed.metrics.table_ns.fill(0);
                if let TableView::Complete(TableRows::StateBuildings(rows)) =
                    &mut *changed.tables[TableId::StateBuildings.index()]
                {
                    rows[0].level = 1;
                }
                prepared.observe_changed(
                    &changed,
                    format,
                    &source,
                    &tracker.observe(&changed, format),
                )?;
                let after = prepared.work();
                assert_eq!(after.row_visits - before.row_visits, 1);
                Ok(Work {
                    row_visits: after.row_visits - before.row_visits,
                    history_visits: after.history_visits - before.history_visits,
                    emitted: after.emitted - before.emitted,
                    ..Default::default()
                })
            })
            .unwrap(),
        );
    }

    #[test]
    fn construction_closed_slots_compact_without_reusing_catalogue_keys_or_losing_retained_output_owners()
     {
        use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
        let flag = AtomicBool::new(false);
        let mut frames = vec![
            life_frame(100, 1, true),
            life_frame(101, 1, false),
            life_frame(102, 2, true),
            life_frame(103, 2, false),
        ];
        let old_row = match &*frames[1].tables[TableId::ConstructionLineOutput.index()] {
            TableView::Complete(TableRows::ConstructionLineOutput(rows)) => rows[0].clone(),
            _ => unreachable!(),
        };
        for frame in &mut frames[2..] {
            if let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
                &mut *frame.tables[TableId::ConstructionLineOutput.index()]
            {
                rows.insert(0, old_row.clone());
            }
        }
        let source = source_for(&frames);
        let format = source.replay_ref().format;
        let expected = scan(source.replay_ref(), 1, &flag).unwrap();
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut writer = fixture.writer();
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames {
            prepared
                .observe_changed(frame, format, &source, &tracker.observe(frame, format))
                .unwrap();
            if [101, 103].contains(&frame.hour) {
                prepared.end_block(1, frame.hour, &source).unwrap();
                prepared
                    .flush(
                        &mut writer,
                        &source,
                        &LoadContext { content: None },
                        1,
                        &flag,
                    )
                    .unwrap();
                assert!(
                    prepared.scan.log.projects.is_empty(),
                    "closed native slots survived publication"
                );
                assert!(prepared.slots.is_empty());
                writer.commit().unwrap();
                if frame.hour == 101 {
                    prepared =
                        serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                    prepared.restore_frame(frame);
                    tracker.invalidate();
                    writer = fixture.append_writer();
                } else {
                    break;
                }
            }
        }
        assert_eq!(prepared.next_project, 2);
        assert_eq!(
            prepared.scan.log.unowned_lines, 0,
            "a closed known handle became an unknown owner"
        );
        let actual = project_log(&fixture.store, &fixture.key, 1, &[0, 1], &flag).unwrap();
        observer_tests::assert_reply_parity(&actual.projects, &expected.projects).unwrap();
        for (tag, (leaves, _)) in crate::products::construction_spending::all_leaves(&expected) {
            for mut leaf in leaves {
                let name = leaf_key(&tag, &leaf).unwrap();
                leaf.spent.clear();
                observer_tests::assert_reply_parity(
                    &published_leaf(&fixture.store, &fixture.key, &tag, &name),
                    &leaf,
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn a_failed_first_output_read_does_not_hide_the_projects_earlier_spending() {
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (100..=102)
            .map(|hour| {
                let mut frame = life_frame(hour, 1, true);
                *frame.tables[TableId::ConstructionLineOutput.index()] = if hour == 100 {
                    TableView::Unavailable {
                        reason: "fixture".into(),
                    }
                } else {
                    TableView::Complete(TableRows::ConstructionLineOutput(vec![
                        ConstructionLineOutputRow {
                            line: 1,
                            since_hour: 101,
                            construction_raw: 100,
                            observed_since_creation: false,
                            exact: true,
                            completion: replay_data::rows::Recorded::Value(None),
                            ..Default::default()
                        },
                    ]))
                };
                frame
            })
            .collect();
        let source = source_for(&frames);
        let format = source.replay_ref().format;
        let mut prepared = Prepared::default();
        for frame in &frames {
            prepared.observe(frame, format, &source).unwrap();
        }
        assert_eq!(prepared.before_first.values().sum::<u32>(), 1);
        let expected = scan(source.replay_ref(), 1, &flag).unwrap();
        let reference: u32 = crate::products::construction_spending::all_leaves(&expected)
            .into_values()
            .map(|(_, before)| before)
            .sum();
        assert_eq!(reference, 1);
    }

    fn frames() -> Vec<DecodedFrame> {
        (100..=151)
            .filter(|hour| *hour != 132)
            .map(|hour| {
                let mut frame = construction_frame(
                    hour,
                    [100, 124, 148].contains(&hour),
                    if hour < 148 {
                        vec![building_line(900, 40, if hour < 124 { 2 } else { 1 }, 0, 3)]
                    } else {
                        vec![]
                    },
                    vec![],
                    Some(vec![building_at(
                        40,
                        CIVILIAN,
                        (
                            if hour < 120 { 0 } else { 1 },
                            if hour < 120 { 0 } else { 1 },
                        ),
                    )]),
                    vec![],
                );
                *frame.tables[TableId::ConstructionLineOutput.index()] =
                    TableView::Complete(TableRows::ConstructionLineOutput(if hour <= 148 {
                        vec![ConstructionLineOutputRow {
                            line: 900,
                            since_hour: 100,
                            construction_raw: if hour < 124 {
                                0
                            } else if hour < 148 {
                                200
                            } else {
                                175
                            },
                            observed_since_creation: true,
                            exact: true,
                            ended: hour == 148,
                            completion: replay_data::rows::Recorded::Value(None),
                            ..Default::default()
                        }]
                    } else {
                        vec![]
                    }));
                frame
            })
            .collect()
    }

    #[test]
    fn restored_construction_publishes_what_continuous_observation_publishes() {
        // The line's project run is open, carried between reads, at the commit
        // after hour 147 and ends at hour 148, the first hour after it. Nothing
        // else names the project after the commit: no output row is recorded.
        let flag = AtomicBool::new(false);
        let mut frames = frames();
        for frame in frames.iter_mut().filter(|frame| frame.hour >= 148) {
            *frame.tables[TableId::ConstructionLineOutput.index()] =
                TableView::Complete(TableRows::ConstructionLineOutput(Vec::new()));
        }
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "construction-restore".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let split = frames.iter().position(|frame| frame.hour == 148).unwrap();
        let blocks = [&frames[..split], &frames[split..]];
        let context = LoadContext { content: None };
        dashboard_base::prepared::observer_tests::assert_restore_continues(
            replay.format,
            &blocks,
            Prepared::default,
            |prepared, frame, changes, _| {
                // Every block resumes from the frame before it.
                if frame.hour == 148 {
                    prepared.restore_frame(&frames[split - 1]);
                }
                prepared
                    .observe_changed(frame, replay.format, &source, changes)
                    .unwrap();
            },
            |prepared, writer, ordinal| {
                prepared
                    .end_block(1, blocks[ordinal].last().unwrap().hour, &source)
                    .unwrap();
                prepared.flush(writer, &source, &context, 1, &flag).unwrap();
                writer
                    .write_at(
                        "construction-continuation-1",
                        0,
                        serde_json::to_vec(prepared).unwrap().as_slice(),
                    )
                    .unwrap();
            },
            |fixture| {
                serde_json::from_reader(std::io::BufReader::new(
                    fixture
                        .store
                        .read(&fixture.key, "construction-continuation-1", 0)
                        .unwrap()
                        .unwrap(),
                ))
                .unwrap()
            },
        );
    }

    /// Each country's spending reply composed from the catalogue's shared
    /// parts and its own leaves is the reply each flush made whole before,
    /// for countries with leaves, with only uncaptured work, and with no
    /// project, across two flushes and a resume between them.
    #[test]
    fn composed_spending_replies_are_the_replies_flushes_made_whole() {
        use dashboard_base::prepared::frame_changes::FrameChangeTracker;
        let flag = AtomicBool::new(false);
        let frames = frames();
        let source = source_for(&frames);
        let format = source.replay_ref().format;
        let fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        let split = frames.iter().position(|frame| frame.hour == 126).unwrap();
        for (index, block) in [&frames[..split], &frames[split..]].into_iter().enumerate() {
            if index == 1 {
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                prepared.restore_frame(&frames[split - 1]);
                tracker.invalidate();
            }
            for frame in block {
                prepared
                    .observe_changed(frame, format, &source, &tracker.observe(frame, format))
                    .unwrap();
            }
            prepared
                .end_block(1, block.last().unwrap().hour, &source)
                .unwrap();
            let mut writer = fixture.writer();
            prepared
                .flush(
                    &mut writer,
                    &source,
                    &LoadContext { content: None },
                    1,
                    &flag,
                )
                .unwrap();
            writer.commit().unwrap();
            assert!(
                prepared
                    .whole_replies
                    .values()
                    .any(|reply| !reply.leaves.is_empty())
            );
            let section = entries_section(1);
            let mut catalogue = entries::read(&fixture.store, &fixture.key, &section)
                .unwrap()
                .unwrap();
            for (tag, whole) in &prepared.whole_replies {
                let (reply, keys) = spending_reply(&mut catalogue, 1, tag).unwrap().unwrap();
                assert_eq!(
                    serde_json::to_value(&reply).unwrap(),
                    serde_json::to_value(whole).unwrap(),
                    "block {index} {tag}"
                );
                for leaf in &reply.leaves {
                    let name = leaf_key(tag, leaf).unwrap();
                    assert_eq!(keys[&name], prepared.spending_keys[&name]);
                }
            }
        }
    }

    /// Before construction's coverage completes, spending answers the
    /// prepared prefix: each requested hour after it reads the total through
    /// the prefix's last hour, and every leaf's later output is still being
    /// prepared. Once complete, the same hours answer as before.
    #[test]
    fn construction_spending_answers_the_prepared_prefix_with_the_hours_still_pending() {
        use dashboard_base::prepared::coverage::publish_coverage;
        let flag = AtomicBool::new(false);
        let frames = frames();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
        let (source, root) = dashboard_base::test_fixtures::file_source(replay);
        let format = source.replay_ref().format;
        let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
        let expected = scan(source.replay_ref(), 1, &flag).unwrap();
        let tags: Vec<String> = crate::products::construction_spending::all_leaves(&expected)
            .into_keys()
            .collect();
        assert!(!tags.is_empty());
        let mut prepared = Prepared::default();
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        let split = frames.iter().position(|frame| frame.hour == 126).unwrap();
        let mut commit = |writer: &mut store::Writer, frames: &[DecodedFrame], complete| {
            for frame in frames {
                prepared
                    .observe_changed(frame, format, &source, &tracker.observe(frame, format))
                    .unwrap();
            }
            let last = frames.last().unwrap().hour;
            prepared.end_block(1, last, &source).unwrap();
            prepared
                .flush(writer, &source, &LoadContext { content: None }, 1, &flag)
                .unwrap();
            publish_coverage(writer, 1, 100, last, complete).unwrap();
        };
        let spending = |tag: &str, hours: &[i32]| {
            query(
                &source,
                "construction-spending",
                serde_json::json!({"branchId": "1", "tag": tag, "hours": hours}),
                None,
                &flag,
            )
            .unwrap()
        };
        let mut writer = store.replace(key.clone()).unwrap();
        commit(&mut writer, &frames[..split], false);
        writer.commit().unwrap();
        // The old reply waited for complete coverage and answered nothing.
        let cold: Vec<_> = tags
            .iter()
            .map(|tag| spending(tag, &[151]).expect("a prefix reply before completion"))
            .collect();
        let mut writer = store.extend(&key).unwrap();
        commit(&mut writer, &frames[split..], true);
        writer.commit().unwrap();
        for (tag, cold) in tags.iter().zip(&cold) {
            assert_eq!(cold["coverage"]["complete"], false);
            assert_eq!(cold["coverage"]["lastHour"], 125);
            assert_eq!(
                cold["approximate"],
                serde_json::json!([{"firstHour": 100, "lastHour": 125}])
            );
            assert_eq!(
                cold["pending"],
                serde_json::json!([{"firstHour": 126, "lastHour": 151}])
            );
            let complete = spending(tag, &[125]).unwrap();
            assert_eq!(complete["pending"], serde_json::json!([]));
            assert_eq!(complete["approximate"], serde_json::json!([]));
            let leaves = cold["leaves"].as_array().unwrap();
            assert!(!leaves.is_empty());
            for leaf in leaves {
                // The total through the prefix's last hour, as the complete
                // reply answers that hour.
                let same = complete["leaves"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|other| {
                        leaf_key(tag, &serde_json::from_value((*other).clone()).unwrap()).unwrap()
                            == leaf_key(tag, &serde_json::from_value(leaf.clone()).unwrap())
                                .unwrap()
                    })
                    .unwrap();
                assert_eq!(leaf["spent"], same["spent"]);
                assert_eq!(
                    leaf["unknown"].as_array().unwrap().last().unwrap(),
                    &serde_json::json!({"knownAt": 126, "reason": "preparing", "project": "", "until": null})
                );
            }
            let whole = spending(tag, &[151]).unwrap();
            assert!(whole["leaves"].as_array().unwrap().iter().all(|leaf| {
                leaf["unknown"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|gap| gap["reason"] != "preparing")
            }));
        }
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    /// Before construction's coverage completes, the log answers its prepared
    /// prefix: the manifest the prefix published, with the later hours
    /// pending, and each window the prefix reached; a later window waits.
    /// Once complete, the windows list the same projects from the same reads.
    #[test]
    fn construction_log_answers_the_windows_its_prepared_prefix_reached() {
        use dashboard_base::prepared::coverage::publish_coverage;
        use dashboard_core::test_fixtures::continuation_records;
        let flag = AtomicBool::new(false);
        let frames = frames();
        let split = frames.iter().position(|frame| frame.hour == 126).unwrap();
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..split].to_vec())]).unwrap();
        let replay = replay
            .append(&[continuation_records(&replay, 1, frames[split..].to_vec())])
            .unwrap();
        let (source, root) = dashboard_base::test_fixtures::file_source(replay);
        let format = source.replay_ref().format;
        let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
        let mut prepared = Prepared::default();
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        let mut commit = |writer: &mut store::Writer, frames: &[DecodedFrame], complete| {
            for frame in frames {
                prepared
                    .observe_changed(frame, format, &source, &tracker.observe(frame, format))
                    .unwrap();
            }
            let last = frames.last().unwrap().hour;
            prepared.end_block(1, last, &source).unwrap();
            prepared
                .flush(writer, &source, &LoadContext { content: None }, 1, &flag)
                .unwrap();
            publish_coverage(writer, 1, 100, last, complete).unwrap();
        };
        let ask = |product: &str, params: serde_json::Value| {
            query(&source, product, params, None, &flag).unwrap()
        };
        let window = |index: usize| serde_json::json!({"branchId": "1", "window": index});
        let mut writer = store.replace(key.clone()).unwrap();
        commit(&mut writer, &frames[..split], false);
        writer.commit().unwrap();
        // The old log waited for complete coverage and answered nothing.
        let manifest = ask(
            "construction-log-manifest",
            serde_json::json!({"branchId": "1"}),
        )
        .expect("the prefix's manifest");
        let first = ask("construction-log", window(0)).expect("a window the prefix reached");
        let later = ask("construction-log", window(1))
            .expect("the newest window proven from the branch start");
        let mut writer = store.extend(&key).unwrap();
        commit(&mut writer, &frames[split..], true);
        writer.commit().unwrap();
        assert_eq!(manifest["coverage"]["complete"], false);
        assert_eq!(manifest["coverage"]["lastHour"], 125);
        assert_eq!(
            manifest["pending"],
            serde_json::json!([{"firstHour": 126, "lastHour": 151}])
        );
        assert_eq!(manifest["windows"].as_array().unwrap().len(), 2);
        assert_eq!(first["coverage"]["lastHour"], 125);
        assert_eq!(later["exactFrom"], 126);
        assert_eq!(later["pending"], serde_json::json!([]));
        let complete_newest = ask("construction-log", window(1)).unwrap();
        assert_eq!(later["projects"], complete_newest["projects"]);
        assert_eq!(later["outputs"], complete_newest["outputs"]);
        let complete = ask("construction-log", window(0)).unwrap();
        assert_eq!(complete["pending"], serde_json::json!([]));
        let projects = |reply: &serde_json::Value| -> Vec<serde_json::Value> {
            reply["projects"]
                .as_array()
                .unwrap()
                .iter()
                .map(|project| {
                    serde_json::json!([
                        project["id"],
                        project["firstRead"],
                        project["createdHour"],
                        project["building"]
                    ])
                })
                .collect()
        };
        assert!(!projects(&first).is_empty());
        assert_eq!(projects(&first), projects(&complete));
        let whole = ask(
            "construction-log-manifest",
            serde_json::json!({"branchId": "1"}),
        )
        .unwrap();
        let types = manifest["types"].as_array().unwrap();
        assert_eq!(
            &whole["types"].as_array().unwrap()[..types.len()],
            types.as_slice()
        );
        drop(source);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn project_events_keep_future_operands_and_spending_after_commits_resume_and_append() {
        let flag = AtomicBool::new(false);
        let frames = frames();
        let replay =
            Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap());
        let source = Source::from_replay(
            dashboard_base::source::SourceKey {
                source_id: "construction-prepared".into(),
                revision: "1".into(),
            },
            "memory.battleplan".into(),
            replay.clone(),
            &flag,
        )
        .unwrap();
        let expected = scan(&replay, 1, &flag).unwrap();
        let expected_leaves = crate::products::construction_spending::all_leaves(&expected);
        let expected_cut = Cut::new(Arc::new(expected), windows(&replay, 1));
        let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
        let mut prepared = Prepared::default();
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for frame in &frames {
            prepared
                .observe_changed(
                    frame,
                    replay.format,
                    &source,
                    &tracker.observe(frame, replay.format),
                )
                .unwrap();
            if [117, 125, 151].contains(&frame.hour) {
                prepared.end_block(1, frame.hour, &source).unwrap();
                let mut writer = if frame.hour == 151 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                prepared
                    .flush(
                        &mut writer,
                        &source,
                        &LoadContext { content: None },
                        1,
                        &flag,
                    )
                    .unwrap();
                writer.commit().unwrap();
                let saved = serde_json::to_vec(&prepared).unwrap();
                assert!(prepared.scan.log.outputs.iter().all(Vec::is_empty));
                assert!(
                    prepared
                        .scan
                        .log
                        .projects
                        .iter()
                        .all(|project| project.runs.len() <= 2 && project.built.is_empty())
                );
                prepared = serde_json::from_slice(&saved).unwrap();
                prepared.restore_frame(frame);
                tracker.invalidate();
            }
        }
        let selected: Vec<_> = (0..prepared.next_project).collect();
        let mut actual = project_log(&fixture.store, &fixture.key, 1, &selected, &flag).unwrap();
        let mut reads = Reader::new(
            &fixture.store,
            &fixture.key,
            "construction-reads-1".into(),
            &flag,
        )
        .unwrap();
        actual.output_reads = events(&mut reads, &[0], i32::MIN, i32::MAX)
            .unwrap()
            .into_iter()
            .map(|row| (row.hour, serde_json::from_value(row.row).unwrap()))
            .collect();
        let actual_cut = Cut::new(Arc::new(actual), windows(&replay, 1));
        for window in 0..actual_cut.windows.len() {
            assert_eq!(
                serde_json::to_value(actual_cut.reply(window)).unwrap(),
                serde_json::to_value(expected_cut.reply(window)).unwrap()
            );
        }
        for (tag, (leaves, _)) in expected_leaves {
            let mut values = Reader::new(
                &fixture.store,
                &fixture.key,
                "construction-spending-1".into(),
                &flag,
            )
            .unwrap();
            for mut leaf in leaves {
                let name = leaf_key(&tag, &leaf).unwrap();
                let raw = leaf
                    .spent
                    .last()
                    .map(|step| step.total_raw.clone())
                    .unwrap_or_else(|| "0".into());
                assert_eq!(
                    values
                        .value_at(prepared.spending_keys[&name], 151)
                        .unwrap()
                        .unwrap()
                        .value
                        .to_string(),
                    raw
                );
                leaf.spent.clear();
                assert_eq!(
                    published_leaf(&fixture.store, &fixture.key, &tag, &name),
                    leaf
                );
            }
        }
    }
}
