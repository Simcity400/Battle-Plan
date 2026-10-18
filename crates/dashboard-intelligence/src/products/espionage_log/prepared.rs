//! Daily replies share one range per replay block. Only open instances and the
//! unfinished day continue in memory; historical episode metadata uses events.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader},
};
use dashboard_base::products::country_groups;
use dashboard_text::metadata::check_cancel;
mod newest;

#[allow(clippy::too_many_arguments)]
pub(crate) fn first_view(
    source: &Source,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &EspionageLogParams,
    covered: Option<i32>,
    complete: bool,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    EspionageLog::validate(params).map_err(dashboard_core::invalid)?;
    let windows = source.block_windows(branch);
    let first = windows
        .first()
        .ok_or_else(|| dashboard_core::invalid("Unknown espionage branch"))?
        .first_hour;
    let last = windows.last().expect("branch windows").last_hour;
    let days = metadata_windows(first, last);
    if let Some(index) = params.window {
        let day = days
            .get(index)
            .ok_or_else(|| dashboard_core::invalid("Espionage day not in branch"))?;
        if complete || covered.is_some_and(|hour| day.last_hour <= hour) {
            return serde_json::to_value(query(
                source, context, store, key, branch, params, cancelled,
            )?)
            .map_err(Into::into);
        }
        if index + 1 == days.len()
            && let Some(reply) = newest::reply(source, branch, context, cancelled)?
        {
            let mut data = serde_json::to_value(reply)?;
            data["warningsComplete"] = serde_json::Value::Bool(false);
            data["exact"] = serde_json::Value::from(day.first_hour);
            return Ok(data);
        }
        let raw = Builder::new(source.replay_ref()).reply(
            branch.to_string(),
            first,
            last,
            source.replay_ref().format.version(),
        );
        return serde_json::to_value(metadata_window(&raw, Some(index))?).map_err(Into::into);
    }
    let mut reply =
        query(source, context, store, key, branch, params, cancelled)?.unwrap_or_else(|| {
            Builder::new(source.replay_ref()).reply(
                branch.to_string(),
                first,
                last,
                source.replay_ref().format.version(),
            )
        });
    reply.first_hour = first;
    reply.last_hour = last;
    reply.windows = days;
    if !complete && let Some(newest) = newest::reply(source, branch, context, cancelled)? {
        reply.available.extend(newest.available);
        reply
            .completions_available
            .extend(newest.completions_available);
        for spans in [&mut reply.available, &mut reply.completions_available] {
            let mut ordered = std::mem::take(spans);
            ordered.sort_unstable_by_key(|span| span.first_hour);
            for span in ordered {
                country_groups::mark_unobserved(spans, span.first_hour, span.last_hour);
            }
        }
        for kind in newest.types {
            if !reply.types.iter().any(|old| old.key == kind.key) {
                reply.types.push(kind);
            }
        }
        reply.unavailable = None;
    }
    serde_json::to_value(reply).map_err(Into::into)
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
struct Header {
    start_hour: Option<i32>,
    start_known_at: Option<i32>,
    end: Option<EspionageOperationEnd>,
}
impl Header {
    fn of(operation: &EspionageOperation) -> Self {
        Self {
            start_hour: operation.start_hour,
            start_known_at: operation.start_known_at,
            end: operation.end.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Candidate {
    handle: u64,
    fingerprint: (u32, u32, u32),
    operation: EspionageOperation,
    last_read: Option<i32>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    builder: Builder,
    first: Option<i32>,
    last: Option<i32>,
    next_window: usize,
    available: Vec<HourSpan>,
    completions: Vec<HourSpan>,
    keys: BTreeMap<String, u64>,
    headers: BTreeMap<String, Header>,
    values: Accumulator,
    #[serde(skip)]
    replies: BTreeMap<u64, Vec<EspionageLogReply>>,
    #[serde(skip)]
    reply_bytes: usize,
    #[serde(skip)]
    repairs: Vec<(i32, EspionageOperation, bool)>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

fn episode_key(id: &str) -> String {
    serde_json::to_string(&("episode", id)).expect("episode key")
}
fn instance_key(handle: u64, fingerprint: (u32, u32, u32)) -> String {
    serde_json::to_string(&("instance", handle, fingerprint)).expect("instance key")
}
fn completion_key(owner: u32, sequence: u64) -> String {
    serde_json::to_string(&("completion", owner, sequence)).expect("completion key")
}
fn key(keys: &mut BTreeMap<String, u64>, name: String) -> io::Result<u64> {
    let next = u64::try_from(keys.len())
        .map_err(|_| dashboard_core::invalid("Too many espionage keys"))?;
    Ok(*keys.entry(name).or_insert(next))
}
fn extend(spans: &mut Vec<HourSpan>, hour: i32) {
    if let Some(span) = spans
        .last_mut()
        .filter(|span| span.last_hour.checked_add(1) == Some(hour))
    {
        span.last_hour = hour;
    } else {
        spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

impl Prepared {
    fn initialize(&mut self, source: &Source, branch: u64) -> io::Result<()> {
        if self.first.is_none() {
            let replay = source.replay_ref();
            let start = replay
                .branches
                .iter()
                .find(|start| start.id == branch)
                .ok_or_else(|| dashboard_core::invalid("Unknown espionage branch"))?;
            self.first = Some(branch_bounds(replay, start).0);
            self.builder = Builder::new(replay);
        }
        Ok(())
    }
    pub fn needs_history_changes(&self, frame: &DecodedFrame, changes: &FrameChanges) -> bool {
        changes.rows_changed[TableId::OperationCompletions.index()] && self.needs_history(frame)
    }
    fn materialize(&mut self) {
        if let Some(through) = self.builder.available.last().map(|span| span.last_hour) {
            for &(index, _) in self.builder.open.values() {
                #[cfg(test)]
                super::super::intelligence_prepared::work(|work| work.history_visits += 1);
                if let Some(run) = self.builder.operations[index].runs.last_mut() {
                    run.last_hour = run.last_hour.max(through);
                }
            }
        }
    }
    pub fn needs_history(&self, frame: &DecodedFrame) -> bool {
        frame
            .complete::<tables::OperationCompletions>()
            .is_ok_and(|rows| {
                rows.iter().any(|row| {
                    !self
                        .builder
                        .seen_completions
                        .contains_key(&(row.owner, row.sequence))
                        && (self
                            .keys
                            .contains_key(&completion_key(row.owner, row.sequence))
                            || self.keys.contains_key(&instance_key(
                                row.id,
                                (row.owner, row.target, row.definition_token),
                            )))
                })
            })
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        source: &Source,
        frame: &DecodedFrame,
        reader: Option<&mut Reader<'_>>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_changes(source, frame, &changes, &countries, reader, cancelled);
        self.test_countries = countries;
        result
    }
    pub fn observe_changes(
        &mut self,
        source: &Source,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        mut reader: Option<&mut Reader<'_>>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:espionage");
        let replay = source.replay_ref();
        self.initialize(source, frame.branch)?;
        self.advance(source, frame.branch, i64::from(frame.hour) - 1, cancelled)?;
        self.builder
            .changed
            .get_or_insert_with(BTreeMap::new)
            .clear();
        self.builder.new_completions.clear();
        if operations_read(frame) || changes.rows_changed[TableId::OperationCompletions.index()] {
            self.materialize();
        }
        if operations_read(frame) || frame.complete::<tables::Operations>().is_err() {
            self.builder.observe_countries(frame, countries);
        } else if self
            .builder
            .available
            .last()
            .is_some_and(|span| span.last_hour.checked_add(1) == Some(frame.hour))
        {
            self.builder.mark_available(frame.hour);
        }
        if changes.rows_changed[TableId::OperationCompletions.index()] {
            if let Some(reader) = reader.as_mut() {
                self.restore_completions(frame, reader, cancelled)?;
            }
            self.builder
                .observe_completions_countries(frame, replay, frame.branch, countries)?;
        } else if TableId::OperationCompletions
            .last_change()
            .present(replay.format.version())
        {
            if frame.complete::<tables::OperationCompletions>().is_ok() {
                extend(&mut self.builder.completions, frame.hour);
            } else {
                self.builder.warn(
                    "OperationCompletions unavailable or incomplete; completion coverage unknown",
                );
            }
        }
        self.last = Some(frame.hour);
        self.values.advance(frame.hour)?;
        if self
            .builder
            .available
            .last()
            .is_some_and(|span| span.last_hour == frame.hour)
        {
            extend(&mut self.available, frame.hour);
        }
        if self
            .builder
            .completions
            .last()
            .is_some_and(|span| span.last_hour == frame.hour)
        {
            extend(&mut self.completions, frame.hour);
        }
        let instances = std::mem::take(
            self.builder
                .changed
                .as_mut()
                .expect("prepared operation mutations"),
        );
        for (index, (handle, fingerprint)) in instances {
            #[cfg(test)]
            super::super::intelligence_prepared::work(|work| work.history_visits += 1);
            let operation = &self.builder.operations[index];
            let name = episode_key(&operation.id);
            let new = !self.keys.contains_key(&name);
            let episode = key(&mut self.keys, name)?;
            let header = Header::of(operation);
            if self.headers.get(&operation.id) != Some(&header) {
                if self.headers.get(&operation.id).is_some_and(|previous| {
                    previous.start_hour != header.start_hour
                        || previous.start_known_at != header.start_known_at
                }) {
                    let mut saved = operation.clone();
                    if saved.runs.len() > 2 {
                        saved.runs.drain(..saved.runs.len() - 2);
                    }
                    self.repairs.push((operation.first_read, saved, false));
                }
                if let Some(old_end) = self
                    .headers
                    .get(&operation.id)
                    .and_then(|header| header.end.as_ref())
                    && header
                        .end
                        .as_ref()
                        .is_some_and(|end| end.by_hour > old_end.by_hour)
                {
                    let mut saved = operation.clone();
                    if saved.runs.len() > 2 {
                        saved.runs.drain(..saved.runs.len() - 2);
                    }
                    self.repairs.push((old_end.by_hour, saved, true));
                }
                let mut saved = operation.clone();
                if saved.runs.len() > 2 {
                    saved.runs.drain(..saved.runs.len() - 2);
                }
                let last_read = saved
                    .runs
                    .last()
                    .map(|run| supplying_read(&self.builder.reads, run.last_hour));
                #[cfg(test)]
                super::super::intelligence_prepared::work(|work| work.emitted += 1);
                self.values.event(
                    episode,
                    frame.hour,
                    &Candidate {
                        handle,
                        fingerprint,
                        operation: saved,
                        last_read,
                    },
                )?;
                self.headers.insert(operation.id.clone(), header);
            }
            if new {
                let instance = key(&mut self.keys, instance_key(handle, fingerprint))?;
                self.values.event(instance, frame.hour, &operation.id)?;
            }
        }
        for (owner, sequence) in std::mem::take(&mut self.builder.new_completions) {
            let reference = &self.builder.seen_completions[&(owner, sequence)];
            let name = completion_key(owner, sequence);
            if !self.keys.contains_key(&name) {
                let completion = key(&mut self.keys, name)?;
                self.values.event(completion, frame.hour, reference)?;
            }
        }
        Ok(())
    }

    fn restore_completions(
        &mut self,
        frame: &DecodedFrame,
        reader: &mut Reader<'_>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let Ok(rows) = frame.complete::<tables::OperationCompletions>() else {
            return Ok(());
        };
        let mut needed: BTreeMap<(u64, (u32, u32, u32)), usize> = BTreeMap::new();
        for row in rows {
            if !self
                .builder
                .seen_completions
                .contains_key(&(row.owner, row.sequence))
                && !self
                    .keys
                    .contains_key(&completion_key(row.owner, row.sequence))
            {
                *needed
                    .entry((row.id, (row.owner, row.target, row.definition_token)))
                    .or_default() += 1;
            }
        }
        for row in rows {
            check_cancel(cancelled)?;
            if self
                .builder
                .seen_completions
                .contains_key(&(row.owner, row.sequence))
            {
                continue;
            }
            if let Some(&key) = self.keys.get(&completion_key(row.owner, row.sequence)) {
                let (_, reference) = reader.latest_event(key, frame.hour)?.ok_or_else(|| {
                    dashboard_core::invalid("Missing immutable espionage completion")
                })?;
                self.builder.seen_completions.insert(
                    (row.owner, row.sequence),
                    serde_json::from_value(reference)?,
                );
                continue;
            }
            let fingerprint = (row.owner, row.target, row.definition_token);
            let Some(&instance) = self.keys.get(&instance_key(row.id, fingerprint)) else {
                continue;
            };
            let mut count = reader.event_count_through(instance, row.completion_hour)?;
            let mut remaining = needed.get(&(row.id, fingerprint)).copied().unwrap_or(0);
            while count > 0 && remaining > 0 {
                count -= 1;
                let page = reader.events(&[instance], i32::MIN, row.completion_hour, count, 1)?;
                let Some(row) = page.rows.into_iter().next() else {
                    return Err(dashboard_core::invalid("Missing espionage instance event"));
                };
                let id: String = serde_json::from_value(row.row)?;
                let episode = *self
                    .keys
                    .get(&episode_key(&id))
                    .ok_or_else(|| dashboard_core::invalid("Missing espionage episode key"))?;
                let (_, row) = reader.latest_event(episode, frame.hour)?.ok_or_else(|| {
                    dashboard_core::invalid("Missing espionage episode continuation")
                })?;
                let candidate: Candidate = serde_json::from_value(row)?;
                if candidate
                    .operation
                    .end
                    .as_ref()
                    .is_some_and(|end| end.completion_sequence.is_some())
                {
                    continue;
                }
                if let Some(existing) = self
                    .builder
                    .operations
                    .iter()
                    .find(|op| op.id == candidate.operation.id)
                {
                    if existing
                        .end
                        .as_ref()
                        .is_none_or(|end| end.completion_sequence.is_none())
                    {
                        remaining -= 1;
                    }
                    continue;
                }
                if let Some(read) = candidate.last_read {
                    match self.builder.reads.binary_search(&read) {
                        Ok(_) => {}
                        Err(index) => self.builder.reads.insert(index, read),
                    }
                }
                self.headers.insert(id, Header::of(&candidate.operation));
                let index = self.builder.operations.len();
                self.builder.operations.push(candidate.operation);
                let instances = self.builder.instances.entry(candidate.handle).or_default();
                let first = self.builder.operations[index].first_read;
                let order = self.keys[&episode_key(&self.builder.operations[index].id)];
                let position = instances.partition_point(|&(index, _)| {
                    let operation = &self.builder.operations[index];
                    (
                        operation.first_read,
                        self.keys
                            .get(&episode_key(&operation.id))
                            .copied()
                            .unwrap_or(u64::MAX),
                    ) < (first, order)
                });
                instances.insert(position, (index, candidate.fingerprint));
                remaining -= 1;
            }
        }
        Ok(())
    }

    fn raw(&self, branch: u64, last: i32, format: u16) -> EspionageLogReply {
        let mut raw = self.builder.reply(
            branch.to_string(),
            self.first.expect("espionage first hour"),
            last,
            format,
        );
        raw.unavailable = self.available.is_empty().then(|| {
            "Operations was unavailable, incomplete or never read in this recording.".into()
        });
        raw
    }
    fn render(&mut self, source: &Source, branch: u64, last: i32) -> io::Result<()> {
        self.materialize();
        let mut raw = self.raw(branch, last, source.replay_ref().format.version());
        let first =
            i64::from(self.first.expect("espionage first hour")) + self.next_window as i64 * 24;
        // Preserve actual terminal census runs in the stored day even while its
        // current ending excludes it. A later drain can move that ending date.
        let mut ended = BTreeMap::new();
        for operation in &mut raw.operations {
            if operation
                .end
                .as_ref()
                .is_some_and(|end| i64::from(end.by_hour) < first)
                && operation
                    .runs
                    .iter()
                    .any(|run| i64::from(run.last_hour) >= first && run.first_hour <= last)
            {
                ended.insert(operation.id.clone(), operation.end.take());
            }
        }
        let mut reply = metadata_window(&raw, Some(self.next_window))?;
        for operation in &mut reply.operations {
            if let Some(end) = ended.remove(&operation.id) {
                operation.end = end;
            }
        }
        reply
            .operations
            .sort_by_key(|operation| self.keys.get(&episode_key(&operation.id)).copied());
        check_reply_budget(&reply)?;
        let ordinal = source
            .block_windows(branch)
            .partition_point(|span| span.first_hour <= reply.first_hour)
            .saturating_sub(1) as u64;
        self.reply_bytes = self
            .reply_bytes
            .saturating_add(serde_json::to_vec(&reply)?.len());
        self.replies.entry(ordinal).or_default().push(reply);
        Ok(())
    }
    fn advance(
        &mut self,
        source: &Source,
        branch: u64,
        through: i64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        loop {
            let first = i64::from(self.first.expect("espionage first hour"))
                + i64::try_from(self.next_window)
                    .map_err(|_| dashboard_core::invalid("Espionage day overflow"))?
                    * 24;
            let last = first + 23;
            if last > through {
                break;
            }
            check_cancel(cancelled)?;
            self.render(
                source,
                branch,
                i32::try_from(last)
                    .map_err(|_| dashboard_core::invalid("Espionage hour overflow"))?,
            )?;
            self.next_window += 1;
            self.trim_runs(last + 1);
        }
        Ok(())
    }
    fn trim_runs(&mut self, first: i64) {
        for operation in &mut self.builder.operations {
            let keep = operation
                .runs
                .partition_point(|run| i64::from(run.last_hour) < first)
                .saturating_sub(2);
            operation.runs.drain(..keep);
        }
        let keep = self
            .builder
            .reads
            .partition_point(|hour| i64::from(*hour) < first)
            .saturating_sub(1);
        let needed: BTreeSet<_> = self
            .builder
            .operations
            .iter()
            .filter_map(|operation| operation.runs.last())
            .map(|run| supplying_read(&self.builder.reads, run.last_hour))
            .collect();
        self.builder.reads = self
            .builder
            .reads
            .iter()
            .enumerate()
            .filter(|(index, hour)| *index >= keep || needed.contains(hour))
            .map(|(_, hour)| *hour)
            .collect();
        for spans in [&mut self.builder.available, &mut self.builder.completions] {
            let keep = spans
                .partition_point(|span| i64::from(span.last_hour) < first)
                .saturating_sub(1);
            spans.drain(..keep);
        }
    }
    pub fn finish_block(
        &mut self,
        source: &Source,
        writer: &mut store::Writer,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.finish_block_into(source, writer, branch, ordinal, cancelled)
    }
    /// [`Self::finish_block`], reading and writing the block's day pages
    /// through `pages`.
    pub(crate) fn finish_block_into(
        &mut self,
        source: &Source,
        pages: &mut impl DayPages,
        branch: u64,
        ordinal: usize,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.initialize(source, branch)?;
        let windows = source.block_windows(branch);
        let mut last = windows
            .get(ordinal)
            .ok_or_else(|| dashboard_core::invalid("Unknown espionage replay block"))?
            .last_hour;
        if ordinal + 1 == windows.len() {
            let start = source
                .replay_ref()
                .branches
                .iter()
                .find(|start| start.id == branch)
                .ok_or_else(|| dashboard_core::invalid("Unknown espionage branch"))?;
            last = branch_bounds(source.replay_ref(), start).1;
        }
        self.last = Some(last);
        self.advance(source, branch, i64::from(last), cancelled)?;
        let next =
            i64::from(self.first.expect("espionage first hour")) + self.next_window as i64 * 24;
        if next <= i64::from(last) {
            self.render(source, branch, last)?;
        }
        // A short block can own no day starts: its partial day belongs to an
        // earlier block. Publish its empty range so block ordinals stay dense.
        self.replies
            .entry(
                u64::try_from(ordinal)
                    .map_err(|_| dashboard_core::invalid("Espionage block overflow"))?,
            )
            .or_default();
        self.repair(source, pages, branch, cancelled)?;
        for (ordinal, replies) in std::mem::take(&mut self.replies) {
            check_cancel(cancelled)?;
            let section = format!("espionage-log-{branch}-windows");
            let mut saved = pages.read(&section, ordinal)?.unwrap_or_default();
            for reply in replies {
                let window = reply.window.expect("espionage day reply");
                match saved.binary_search_by_key(&window, |reply| {
                    reply.window.expect("stored espionage day")
                }) {
                    Ok(index) => saved[index] = reply,
                    Err(index) => saved.insert(index, reply),
                }
            }
            pages.write(&section, ordinal, saved)?;
        }
        self.reply_bytes = 0;
        Ok(())
    }
    fn repair(
        &mut self,
        source: &Source,
        pages: &mut impl DayPages,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let mut loaded = BTreeSet::new();
        for (old_end, operation, endpoints) in std::mem::take(&mut self.repairs) {
            let first = self.first.expect("espionage first hour");
            let start = usize::try_from((i64::from(old_end) - i64::from(first)) / 24)
                .map_err(|_| dashboard_core::invalid("Invalid espionage repair date"))?;
            let mut raw = self.raw(
                branch,
                self.last.expect("espionage repair end"),
                source.replay_ref().format.version(),
            );
            raw.operations = vec![operation];
            for day in start..self.next_window {
                check_cancel(cancelled)?;
                let projected = metadata_window(&raw, Some(day))?;
                let ordinal = source
                    .block_windows(branch)
                    .partition_point(|span| span.first_hour <= projected.first_hour)
                    .saturating_sub(1) as u64;
                if loaded.insert(ordinal) {
                    let section = format!("espionage-log-{branch}-windows");
                    if let Some(mut saved) = pages.read(&section, ordinal)? {
                        saved.extend(self.replies.remove(&ordinal).unwrap_or_default());
                        self.replies.insert(ordinal, saved);
                    }
                }
                let page = self
                    .replies
                    .get_mut(&ordinal)
                    .and_then(|pages| pages.iter_mut().rev().find(|page| page.window == Some(day)))
                    .ok_or_else(|| dashboard_core::invalid("Missing espionage page to repair"))?;
                for operation in projected.operations {
                    if let Some(existing) = page
                        .operations
                        .iter_mut()
                        .find(|existing| existing.id == operation.id)
                    {
                        existing.start_hour = operation.start_hour;
                        existing.start_known_at = operation.start_known_at;
                        existing.end = operation.end;
                        // A late drain's continuation can be newer than this
                        // entire day. Keep the page's genuine prior witness
                        // when that continuation supplies no replacement.
                        if endpoints
                            && operation
                                .runs
                                .iter()
                                .any(|run| run.last_hour < page.first_hour)
                        {
                            existing.runs.retain(|run| run.last_hour >= page.first_hour);
                            existing.runs.extend(
                                operation
                                    .runs
                                    .into_iter()
                                    .filter(|run| run.last_hour < page.first_hour),
                            );
                            existing.runs.sort_by_key(|run| run.first_hour);
                        }
                    } else {
                        page.operations.push(operation);
                    }
                }
                // The existing day builder owns witness coverage as well. Its
                // inputs are authentic in-window coverage and repaired values.
                let mut coverage = raw.clone();
                coverage.available = clipped(&page.available, page.first_hour, page.last_hour);
                coverage.operations = page.operations.clone();
                page.available = metadata_window(&coverage, Some(day))?.available;
                page.operations
                    .sort_by_key(|operation| self.keys.get(&episode_key(&operation.id)).copied());
                check_reply_budget(page)?;
            }
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.reply_bytes + self.values.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.materialize();
        self.values.flush(
            writer,
            &format!("espionage-log-{branch}-series"),
            0,
            cancelled,
        )?;
        if let Some(first) = self.first {
            let last = self.last.unwrap_or(first);
            let mut manifest = self.raw(branch, last, replay_data::FORMAT_VERSION);
            manifest.available = self.available.clone();
            manifest.completions_available = self.completions.clone();
            manifest.operations.clear();
            manifest.read_hours.clear();
            manifest.windows = metadata_windows(first, last);
            series::write_catalogue(writer, "espionage-log", branch, &self.keys, &(first, last))?;
            series::write_reply(
                writer,
                &format!("espionage-log-{branch}-manifest"),
                0,
                &manifest,
            )?;
        }
        // Pages and sparse events now own closed episodes. The unfinished day
        // still needs witnesses for endings within that day, alongside open items.
        let day = i64::from(self.first.unwrap_or(0)) + self.next_window as i64 * 24;
        let retained: BTreeSet<_> = self
            .builder
            .open
            .values()
            .map(|&(index, _)| index)
            .chain(
                self.builder
                    .operations
                    .iter()
                    .enumerate()
                    .filter(|(_, op)| {
                        op.end
                            .as_ref()
                            .is_some_and(|end| i64::from(end.by_hour) >= day)
                    })
                    .map(|(index, _)| index),
            )
            .collect();
        let old = std::mem::take(&mut self.builder.operations);
        let mut positions = BTreeMap::new();
        for (index, operation) in old.into_iter().enumerate() {
            if retained.contains(&index) {
                positions.insert(index, self.builder.operations.len());
                self.builder.operations.push(operation);
            }
        }
        self.builder.instances.retain(|_, instances| {
            instances.retain(|(index, _)| positions.contains_key(index));
            for (index, _) in instances.iter_mut() {
                *index = positions[&*index];
            }
            !instances.is_empty()
        });
        for (index, _) in self.builder.open.values_mut() {
            *index = positions[&*index];
        }
        let ids: BTreeSet<_> = self
            .builder
            .operations
            .iter()
            .map(|op| op.id.as_str())
            .collect();
        self.headers.retain(|id, _| ids.contains(id.as_str()));
        self.builder.seen_completions.clear();
        self.trim_runs(day);
        series::write_reply(writer, &format!("espionage-log-{branch}"), 0, self)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

/// Where a finished block's day pages are read and written. Hourly
/// preparation reads and writes them through the store writer as each block
/// finishes; the intelligence family prepared on its own finishes its blocks
/// without a writer, so it stages them until its next commit ([`StagedPages`]).
pub(crate) trait DayPages {
    /// The pages of `section`'s range `ordinal` as they stand.
    fn read(&mut self, section: &str, ordinal: u64) -> io::Result<Option<Vec<EspionageLogReply>>>;
    /// Replace that range's pages.
    fn write(
        &mut self,
        section: &str,
        ordinal: u64,
        pages: Vec<EspionageLogReply>,
    ) -> io::Result<()>;
}

impl DayPages for store::Writer {
    fn read(&mut self, section: &str, ordinal: u64) -> io::Result<Option<Vec<EspionageLogReply>>> {
        match self.read_at(section, ordinal)? {
            Some(file) => Ok(Some(serde_json::from_reader(zstd::stream::Decoder::new(
                file,
            )?)?)),
            None => Ok(None),
        }
    }

    fn write(
        &mut self,
        section: &str,
        ordinal: u64,
        pages: Vec<EspionageLogReply>,
    ) -> io::Result<()> {
        series::write_reply(self, section, ordinal, &pages)
    }
}

/// Day pages finished since the family's last commit, by section and range,
/// and their serialized size against its batch budget. Each range holds the
/// pages its latest finishing block left, as the store writer would.
#[derive(Default)]
pub(crate) struct StagedPages {
    pages: BTreeMap<(String, u64), Vec<EspionageLogReply>>,
    bytes: usize,
}

impl StagedPages {
    /// Serialized bytes staged since the last flush.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// The staged pages over the committed store: a range staged since the
    /// last flush reads as staged, any other as `key` last committed it.
    pub(crate) fn through<'a>(
        &'a mut self,
        store: &'a store::Store,
        key: &'a store::SourceKey,
    ) -> impl DayPages + 'a {
        ThroughStore {
            staged: self,
            store,
            key,
        }
    }

    /// Write every staged range, in range order (so each section's ranges
    /// are appended in turn), with the bytes the store writer would have
    /// been given as each block finished.
    pub(crate) fn write(&mut self, writer: &mut store::Writer) -> io::Result<()> {
        for ((section, ordinal), pages) in std::mem::take(&mut self.pages) {
            series::write_reply(writer, &section, ordinal, &pages)?;
        }
        self.bytes = 0;
        Ok(())
    }
}

struct ThroughStore<'a> {
    staged: &'a mut StagedPages,
    store: &'a store::Store,
    key: &'a store::SourceKey,
}

impl DayPages for ThroughStore<'_> {
    fn read(&mut self, section: &str, ordinal: u64) -> io::Result<Option<Vec<EspionageLogReply>>> {
        if let Some(pages) = self.staged.pages.get(&(section.to_owned(), ordinal)) {
            return Ok(Some(pages.clone()));
        }
        series::read_reply(self.store, self.key, section, ordinal)
    }

    fn write(
        &mut self,
        section: &str,
        ordinal: u64,
        pages: Vec<EspionageLogReply>,
    ) -> io::Result<()> {
        self.staged.bytes = self
            .staged
            .bytes
            .saturating_add(serde_json::to_vec(&pages)?.len());
        self.staged
            .pages
            .insert((section.to_owned(), ordinal), pages);
        Ok(())
    }
}

pub fn query(
    source: &Source,
    context: &LoadContext,
    store: &store::Store,
    key: &store::SourceKey,
    branch: u64,
    params: &EspionageLogParams,
    cancelled: &AtomicBool,
) -> io::Result<Option<EspionageLogReply>> {
    EspionageLog::validate(params).map_err(dashboard_core::invalid)?;
    let Some(manifest) = series::read_reply::<EspionageLogReply>(
        store,
        key,
        &format!("espionage-log-{branch}-manifest"),
        0,
    )?
    else {
        return Ok(None);
    };
    let mut reply = if let Some(window) = params.window {
        let span = manifest.windows.get(window).ok_or_else(|| {
            dashboard_core::invalid("Espionage metadata window not in this branch")
        })?;
        let ordinal = source
            .block_windows(branch)
            .partition_point(|block| block.first_hour <= span.first_hour)
            .saturating_sub(1) as u64;
        let Some(mut replies) = series::read_reply::<Vec<EspionageLogReply>>(
            store,
            key,
            &format!("espionage-log-{branch}-windows"),
            ordinal,
        )?
        else {
            return Ok(None);
        };
        let index = replies
            .binary_search_by_key(&window, |reply| reply.window.expect("stored espionage day"))
            .map_err(|_| dashboard_core::invalid("Missing espionage day in its replay block"))?;
        let mut reply = replies.remove(index);
        reply.operations.retain(|operation| {
            operation
                .end
                .as_ref()
                .is_none_or(|end| end.by_hour >= reply.first_hour)
        });
        let mut raw = reply.clone();
        raw.first_hour = manifest.first_hour;
        raw.available = clipped(&reply.available, reply.first_hour, reply.last_hour);
        reply.available = metadata_window(&raw, Some(window))?.available;
        reply.types = manifest.types;
        reply.warnings = manifest.warnings;
        reply.unavailable = manifest.unavailable;
        reply
    } else {
        manifest
    };
    present(source, context, cancelled, &mut reply)?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests;
