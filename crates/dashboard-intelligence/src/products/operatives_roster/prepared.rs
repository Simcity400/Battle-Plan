//! Daily provenance and operative episodes are shared series. The checkpoint
//! retains only the latest episode per handle and the production scan's hold.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader},
};
use dashboard_text::metadata::check_cancel;

const OPERATIONS: u64 = 0;
const ABSENT: u64 = 1;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Option<Scan>,
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    first: Option<i32>,
    last: Option<i32>,
    current: BTreeSet<usize>,
    active: BTreeMap<usize, BTreeSet<u64>>,
    membership_dirty: bool,
    operations_dirty: bool,
    #[serde(skip)]
    clocks: BTreeMap<i32, BTreeSet<usize>>,
    #[serde(skip)]
    scheduled: BTreeMap<usize, i32>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

#[derive(Serialize, Deserialize)]
struct Episode {
    country: CountryIdentity,
    entry: OperativeRosterEntry,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub branch_id: String,
    pub tags: Vec<String>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    #[serde(default)]
    pub include_departed: bool,
}

/// Where an hour's roster reads come from: the row path's frame, read as the hour needs, or the
/// reads a segment's operator handed on.
enum Given<'a> {
    Frame(&'a replay_frame::frame::DecodedFrame),
    Read(&'a RosterRead),
}

/// Tables whose change makes the next membership read join the census afresh.
const MEMBERSHIP_INPUTS: [TableId; 5] = [
    TableId::OperativeMemberships,
    TableId::SimulationFeatures,
    TableId::CharacterOperatives,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// Tables whose change makes the next operations read name operations afresh.
const OPERATION_INPUTS: [TableId; 2] = [TableId::Operations, TableId::LexicalTokens];

fn key(keys: &mut BTreeMap<String, u64>, name: String) -> io::Result<u64> {
    let next = u64::try_from(keys.len())
        .ok()
        .and_then(|n| n.checked_add(2))
        .ok_or_else(|| dashboard_core::invalid("Too many prepared operative keys"))?;
    Ok(*keys.entry(name).or_insert(next))
}

impl Prepared {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        frame: &replay_frame::frame::DecodedFrame,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        let content = super::super::operative_content::content(source, context)
            .map_err(|error| error.to_string());
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_bound(frame, &changes, &countries, &content);
        self.test_countries = countries;
        result
    }

    /// The marks an hour's observation sets from the tables it changed in
    /// any field, read or not (`rebuilt`): the next membership read then joins
    /// afresh, as it does after any rebuild of its inputs, though the hour's
    /// change facts report only the fields the roster reads. Operations names
    /// afresh only from those change facts ([`OPERATION_INPUTS`]): its name is
    /// its definition's token alone, and its read every hour would otherwise
    /// observe every roster at each preparation tick.
    pub(crate) fn note_rebuilt(&mut self, rebuilt: &dyn Fn(TableId) -> bool) {
        if rebuilt(TableId::CharacterOperatives)
            && let Some(scan) = &mut self.scan
        {
            scan.reader.forget_characters();
        }
        self.membership_dirty |= MEMBERSHIP_INPUTS.iter().any(|&table| rebuilt(table));
    }

    /// Observes `frame` of the row path, reading what its hour needs of it.
    pub fn observe_bound(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<super::super::operative_content::OperativeContent>>,
            String,
        >,
    ) -> io::Result<()> {
        self.observe_given(
            frame.hour,
            Flags::of(frame),
            Given::Frame(frame),
            changes,
            countries,
            content,
        )
    }

    /// Observes `hour`, whose change facts are `changes`, from the roster's `read` of it.
    pub(crate) fn observe_read(
        &mut self,
        hour: i32,
        flags: Flags,
        read: &RosterRead,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<super::super::operative_content::OperativeContent>>,
            String,
        >,
    ) -> io::Result<()> {
        self.observe_given(hour, flags, Given::Read(read), changes, countries, content)
    }

    fn observe_given(
        &mut self,
        hour: i32,
        flags: Flags,
        given: Given<'_>,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        content: &Result<
            Option<std::sync::Arc<super::super::operative_content::OperativeContent>>,
            String,
        >,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:roster");
        let bound = content.as_ref().ok().and_then(|content| content.as_deref());
        if self.scan.is_none() {
            self.scan = Some(Scan::new(
                &[],
                bound,
                content.as_ref().err().cloned(),
                true,
            )?);
        }
        let scan = self.scan.as_mut().expect("roster scan");
        let previous = self.last;
        if changes.rows_changed[TableId::CharacterOperatives.index()] {
            scan.reader.forget_characters();
        }
        let identity_changed = countries.changed
            || previous.is_none_or(|last| last.checked_add(1) != Some(hour))
            || changes.rows_changed[TableId::SimulationFeatures.index()];
        self.membership_dirty |= MEMBERSHIP_INPUTS
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        self.membership_dirty |= countries.changed;
        self.operations_dirty |= OPERATION_INPUTS
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        if identity_changed {
            scan.add_tags(countries);
        }
        let mut indices = BTreeSet::new();
        let mut current = BTreeSet::new();
        if identity_changed {
            indices.extend(self.current.iter().copied());
            if let Some(rows) = countries.countries.as_deref() {
                for country in rows {
                    if let Ok(tag) = std::str::from_utf8(&country.tag)
                        && let Some(&index) = scan.tags.get(tag)
                    {
                        indices.insert(index);
                        current.insert(index);
                    }
                }
            }
        }
        let read = flags.membership_read && flags.memberships;
        let operations_read = flags.operations_read && flags.operations;
        let inputs_changed = identity_changed
            || (read && self.membership_dirty)
            || (operations_read && self.operations_dirty)
            || changes.rows_changed[TableId::Operations.index()]
            || changes.rows_changed[TableId::OperativeMemberships.index()]
            || (bound.is_some()
                && [
                    TableId::CountryAppearance,
                    TableId::ReservedCountryNaming,
                    TableId::Politics,
                    TableId::PoliticalParties,
                    TableId::Autonomy,
                ]
                .iter()
                .any(|table| changes.rows_changed[table.index()]));
        if inputs_changed {
            indices.extend(self.current.iter().copied());
        }
        while let Some((&due, _)) = self
            .clocks
            .first_key_value()
            .filter(|(due, _)| **due <= hour)
        {
            indices.extend(self.clocks.remove(&due).expect("roster clock"));
        }
        self.values.advance(hour)?;
        // Equal fresh reads date the already authenticated census without another
        // country/operative join. Empty known rosters get the same read proof.
        if read && !self.membership_dirty {
            for &index in &self.current {
                let history = &mut scan.histories[index];
                if !history.open {
                    continue;
                }
                history.read_hour = hour;
                history.reply.observed.clear();
                history.reply.observed.push(OperativeRosterObservation {
                    first_hour: hour,
                    last_hour: hour,
                    read_hour: hour,
                });
                let read_key = key(
                    &mut self.keys,
                    series::field_key(&history.reply.tag, "observed"),
                )?;
                self.values
                    .observe(read_key, hour, Some(i128::from(hour)), false);
            }
        }
        if !indices.is_empty() || inputs_changed {
            if let Some(previous) = previous {
                for &index in &indices {
                    let history = &mut scan.histories[index];
                    history.track.advance(previous);
                    if history.open {
                        #[cfg(test)]
                        super::super::intelligence_prepared::work(|work| {
                            work.history_visits += history.active.len()
                        });
                        if let Some(observed) = history.reply.observed.last_mut() {
                            observed.last_hour = previous.max(observed.last_hour);
                        }
                        for &entry in history.active.values() {
                            if let Some(run) = history.reply.operatives[entry].runs.last_mut() {
                                run.last_hour = previous;
                            }
                        }
                    }
                }
            }
            let owned;
            let read = match given {
                Given::Read(read) => read,
                Given::Frame(frame) => {
                    owned = scan.frame_read(
                        frame,
                        bound,
                        countries,
                        Some(changes),
                        !self.membership_dirty,
                    );
                    &owned
                }
            };
            scan.observe_indices(
                hour,
                flags,
                bound,
                None,
                Some(&indices),
                Some(changes),
                (!self.membership_dirty, self.operations_dirty),
                countries,
                read,
            )?;
        } else {
            scan.observe_operations(hour, flags)?;
            if flags.feature == Some(false)
                && let Some(span) = scan.agency_absent.last_mut()
            {
                span.last_hour = hour;
            }
        }
        if read {
            self.membership_dirty = false;
        }
        if operations_read {
            self.operations_dirty = false;
        }
        if identity_changed {
            self.current = current;
        }
        self.first.get_or_insert(hour);
        self.last = Some(hour);
        let operations = scan.operation_read_hour.map(i128::from);
        if operations_read
            || changes.rows_changed[TableId::Operations.index()]
            || self.first == Some(hour)
        {
            self.values
                .observe(OPERATIONS, hour, operations, false);
        }
        if identity_changed {
            self.values.observe(
                ABSENT,
                hour,
                (flags.feature == Some(false)).then_some(1),
                false,
            );
        }
        for &index in &indices {
            let history = &mut scan.histories[index];
            let read_key = key(
                &mut self.keys,
                series::field_key(&history.reply.tag, "observed"),
            )?;
            let observed = history.open.then_some(i128::from(history.read_hour));
            self.values.observe(read_key, hour, observed, false);
            let changed = std::mem::take(&mut history.changed);
            for entry_index in changed {
                let entry = &mut history.reply.operatives[entry_index];
                let episode_key = key(
                    &mut self.keys,
                    series::field_key(&history.reply.tag, &format!("episode {}", entry.key)),
                )?;
                let run = entry.runs.last().filter(|run| run.last_hour == hour);
                if let Some(run) = run {
                    if entry.departed_hour == Some(hour) {
                        self.values.push(
                            episode_key,
                            series::Run::new(
                                hour,
                                hour,
                                i128::from(run.first_hour),
                                true,
                            ),
                        );
                    } else {
                        self.values.observe(
                            episode_key,
                            hour,
                            Some(i128::from(run.first_hour)),
                            run.first_hour == hour,
                        );
                    }
                }
                let country = history.country.ok_or_else(|| {
                    dashboard_core::invalid("Operative episode has no country identity")
                })?;
                #[cfg(test)]
                super::super::intelligence_prepared::work(|work| work.emitted += 1);
                self.values.event(
                    episode_key,
                    hour,
                    &Episode {
                        country,
                        entry: entry.clone(),
                    },
                )?;
                if let Some(run) = run.filter(|run| {
                    run.value.captured == Some(true) && run.value.captured_hour.is_some()
                }) && entry.runs.iter().rev().nth(1).is_none_or(|previous| {
                    previous.value.captured != Some(true)
                        || previous.value.captured_hour != run.value.captured_hour
                }) {
                    let capture_key = key(
                        &mut self.keys,
                        series::field_key(&history.reply.tag, &format!("capture {}", entry.key)),
                    )?;
                    self.values.event(capture_key, hour, run)?;
                }
                if entry.runs.len() > 1 {
                    entry.runs.drain(..entry.runs.len() - 1);
                }
            }
            let active: BTreeSet<_> = history
                .active
                .values()
                .filter_map(|&entry| {
                    history
                        .open
                        .then(|| {
                            self.keys
                                .get(&series::field_key(
                                    &history.reply.tag,
                                    &format!("episode {}", history.reply.operatives[entry].key),
                                ))
                                .copied()
                        })
                        .flatten()
                })
                .collect();
            if let Some(old) = self.active.get(&index) {
                for &removed in old.difference(&active) {
                    self.values.observe(removed, hour, None, false);
                }
            }
            self.active.insert(index, active);
            if history.reply.observed.len() > 1 {
                history
                    .reply
                    .observed
                    .drain(..history.reply.observed.len() - 1);
            }
            // Only clock-sensitive held entries schedule another projection. The
            // native signed day rounding supplies each next deadline.
            if let Some(old) = self.scheduled.remove(&index)
                && let Some(countries) = self.clocks.get_mut(&old)
            {
                countries.remove(&index);
                if countries.is_empty() {
                    self.clocks.remove(&old);
                }
            }
            if bound.is_some() && history.open {
                let next = history
                    .held
                    .values()
                    .filter_map(|held| {
                        // Truncated day rounding holds a cooldown's zero for 47 hours.
                        (1..=48)
                            .filter_map(|offset| hour.checked_add(offset))
                            .find(|&at| {
                                (held.value.captured == Some(true)
                                    && at.wrapping_sub(engine_native::cdate::HOUR_EPOCH) / 24
                                        != hour.wrapping_sub(engine_native::cdate::HOUR_EPOCH)
                                            / 24)
                                    || held.cooldown.is_some_and(|(kind, _, end)| {
                                        super::super::operative_content::cooldown_remaining(
                                            kind, end, at,
                                        ) != super::super::operative_content::cooldown_remaining(
                                            kind, end, hour,
                                        )
                                    })
                            })
                    })
                    .min();
                if let Some(next) = next {
                    self.clocks.entry(next).or_default().insert(index);
                    self.scheduled.insert(index, next);
                }
            }
        }
        if scan.operations.len() > 1 {
            scan.operations.drain(..scan.operations.len() - 1);
        }
        if scan.agency_absent.len() > 1 {
            scan.agency_absent.drain(..scan.agency_absent.len() - 1);
        }
        Ok(())
    }
    pub fn bytes(&self) -> usize {
        self.values.bytes()
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Some(last) = self.last
            && let Some(scan) = &mut self.scan
        {
            for &index in &self.current {
                let history = &mut scan.histories[index];
                history.track.advance(last);
                if history.open {
                    if let Some(observed) = history.reply.observed.last_mut() {
                        observed.last_hour = last;
                    }
                    for &index in history.active.values() {
                        if let Some(run) = history.reply.operatives[index].runs.last_mut() {
                            run.last_hour = last;
                        }
                    }
                }
            }
        }
        self.values.flush(
            writer,
            &format!("operatives-roster-{branch}-series"),
            0,
            cancelled,
        )?;
        if let Some(bounds) = self.first.zip(self.last) {
            series::write_catalogue(writer, "operatives-roster", branch, &self.keys, &bounds)?;
        }
        if let Some(scan) = &mut self.scan {
            for history in &mut scan.histories {
                let retained: BTreeSet<_> = history
                    .latest
                    .values()
                    .chain(history.active.values())
                    .copied()
                    .collect();
                let mut positions = BTreeMap::new();
                let entries = std::mem::take(&mut history.reply.operatives);
                for (old, entry) in entries.into_iter().enumerate() {
                    if retained.contains(&old) {
                        positions.insert(old, history.reply.operatives.len());
                        history.reply.operatives.push(entry);
                    }
                }
                for index in history
                    .latest
                    .values_mut()
                    .chain(history.active.values_mut())
                {
                    *index = positions[&*index];
                }
            }
        }
        series::write_reply(writer, &format!("operatives-roster-{branch}"), 0, self)
    }
    pub fn read(reader: impl io::Read) -> io::Result<Self> {
        Ok(serde_json::from_reader(zstd::stream::Decoder::new(
            reader,
        )?)?)
    }
}

pub fn query(
    source: &Source,
    context: &LoadContext,
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Option<OperativeRosterReply>> {
    validate_countries(&params.branch_id, &params.tags).map_err(dashboard_core::invalid)?;
    let history = super::super::intelligence_prepared::histories(
        source, context, store, source_key, branch, cancelled,
    )?;
    let mut reply = query_history(store, source_key, branch, params, &history, cancelled)?;
    if let Some(reply) = &mut reply {
        super::present_artwork(source, context, reply)?;
    }
    Ok(reply)
}

fn query_history(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    params: &Params,
    history: &dashboard_base::products::country_history::RecordedHistories,
    cancelled: &AtomicBool,
) -> io::Result<Option<OperativeRosterReply>> {
    let Some((keys, (branch_first, branch_last))) =
        series::read_catalogue::<(i32, i32)>(store, source_key, "operatives-roster", branch)?
    else {
        return Ok(None);
    };
    let Some(mut state) = series::read_reply::<Prepared>(
        store,
        source_key,
        &format!("operatives-roster-{branch}"),
        0,
    )?
    else {
        return Ok(None);
    };
    let Some(scan) = state.scan.as_mut() else {
        return Ok(None);
    };
    let first = params.first_hour.unwrap_or(branch_first);
    let last = params.last_hour.unwrap_or(branch_last);
    if first > last {
        return Err(dashboard_core::invalid(
            "Invalid operative roster query range",
        ));
    }
    let selected =
        dashboard_base::products::country_history::selected_identities(history, &params.tags);
    let mut tags =
        dashboard_base::products::country_history::selected_aliases(history, &params.tags);
    for tag in &params.tags {
        if !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    scan.histories = tags.iter().map(|tag| History::new(tag)).collect();
    let mut reader = Reader::new(
        store,
        source_key,
        format!("operatives-roster-{branch}-series"),
        cancelled,
    )?;
    let mut by_tag: BTreeMap<String, Vec<(u64, String)>> = BTreeMap::new();
    for (name, &key) in &keys {
        let (tag, field): (String, String) = serde_json::from_str(name)?;
        if let Some(episode) = field.strip_prefix("episode ") {
            by_tag
                .entry(tag)
                .or_default()
                .push((key, episode.to_owned()));
        }
    }
    let mut anchors = BTreeSet::new();
    for country in &mut scan.histories {
        let tag = &country.reply.tag;
        if let Some(recorded) = history.countries.get(tag) {
            country.reply.identities = recorded.identities.as_ref().clone();
        }
        let mut episodes = by_tag.remove(tag).unwrap_or_default();
        episodes.sort_by_key(|&(key, _)| key);
        let mut ordinal = 0;
        for (key, original) in episodes {
            check_cancel(cancelled)?;
            let Some((_, row)) = reader.latest_event(key, branch_last)? else {
                continue;
            };
            let Episode {
                country: identity,
                mut entry,
            } = serde_json::from_slice::<Episode>(&row)?;
            if !selected.contains(&(identity.slot, identity.id)) {
                continue;
            }
            let (stem, _) = entry
                .key
                .rsplit_once(':')
                .ok_or_else(|| dashboard_core::invalid("Invalid operative episode key"))?;
            entry.key = format!("{stem}:{ordinal}");
            ordinal += 1;
            let latest = entry.runs.pop();
            entry.runs.clear();
            for span in reader.runs(key, first, last)? {
                let (_, row) = reader
                    .latest_event(key, span.first_hour)?
                    .ok_or_else(|| dashboard_core::invalid("Missing operative value event"))?;
                let event: Episode = serde_json::from_slice(&row)?;
                let value = event
                    .entry
                    .runs
                    .last()
                    .ok_or_else(|| dashboard_core::invalid("Missing operative value"))?
                    .value
                    .clone();
                entry.runs.push(OperativeRosterRun {
                    first_hour: span.first_hour,
                    last_hour: span.last_hour,
                    value,
                });
            }
            if entry.runs.is_empty()
                && params.include_departed
                && entry.departed_hour.is_some_and(|hour| hour <= last)
                && let Some(run) = latest
            {
                anchors.insert(run.last_hour);
                entry.runs.push(OperativeRosterRun {
                    first_hour: run.last_hour,
                    last_hour: run.last_hour,
                    value: run.value,
                });
            }
            if entry.runs.is_empty() {
                continue;
            }
            if let Some(&capture_key) =
                keys.get(&series::field_key(tag, &format!("capture {original}")))
            {
                for row in reader
                    .events(&[capture_key], branch_first, last, 0, u32::MAX)?
                    .rows
                {
                    let mut run: OperativeRosterRun = serde_json::from_slice(&row.row)?;
                    if run.first_hour < first {
                        run.last_hour = run.first_hour;
                        anchors.insert(run.first_hour);
                        entry.runs.push(run);
                    }
                }
            }
            entry.runs.sort_by_key(|run| run.first_hour);
            entry.runs.dedup_by_key(|run| run.first_hour);
            country.reply.operatives.push(entry);
        }
    }
    // Every historical witness includes its authentic supplying census. A held
    // alias is proved from that read through the witness, across selected tags.
    let mut intervals = vec![(first, last)];
    intervals.extend(anchors.into_iter().map(|hour| (hour, hour)));
    for &(first, _) in &intervals.clone() {
        for tag in &tags {
            if let Some(&key) = keys.get(&series::field_key(tag, "observed"))
                && let Some(run) = reader.value_at(key, first)?
            {
                let read = i32::try_from(run.value)
                    .map_err(|_| dashboard_core::invalid("Invalid operative census date"))?;
                intervals.push((read, first));
            }
        }
        if let Some(run) = reader.value_at(OPERATIONS, first)? {
            let read = i32::try_from(run.value)
                .map_err(|_| dashboard_core::invalid("Invalid operation census date"))?;
            intervals.push((read, first));
        }
    }
    intervals.sort_unstable();
    let mut merged: Vec<(i32, i32)> = Vec::new();
    for (first, last) in intervals {
        if let Some(span) = merged
            .last_mut()
            .filter(|span| i64::from(span.1) + 1 >= i64::from(first))
        {
            span.1 = span.1.max(last);
        } else {
            merged.push((first, last));
        }
    }
    fn observations(
        reader: &mut Reader<'_>,
        key: u64,
        intervals: &[(i32, i32)],
    ) -> io::Result<Vec<OperativeRosterObservation>> {
        let mut result = Vec::new();
        for &(first, last) in intervals {
            for run in reader.runs(key, first, last)? {
                result.push(OperativeRosterObservation {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    read_hour: i32::try_from(run.value)
                        .map_err(|_| dashboard_core::invalid("Invalid operative read date"))?,
                });
            }
        }
        Ok(result)
    }
    scan.operations = observations(&mut reader, OPERATIONS, &merged)?;
    scan.agency_absent = reader
        .runs(ABSENT, first, last)?
        .into_iter()
        .map(|run| dashboard_base::products::country_groups::HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        })
        .collect();
    for country in &mut scan.histories {
        if let Some(&key) = keys.get(&series::field_key(&country.reply.tag, "observed")) {
            country.reply.observed = observations(&mut reader, key, &merged)?
                .into_iter()
                .flat_map(|read| {
                    country
                        .reply
                        .identities
                        .iter()
                        .filter_map(|identity| {
                            let first_hour = identity.first_hour.max(read.first_hour);
                            let last_hour = identity.last_hour.min(read.last_hour);
                            (selected.contains(&(identity.slot, identity.country_id))
                                && first_hour <= last_hour)
                                .then_some(OperativeRosterObservation {
                                    first_hour,
                                    last_hour,
                                    read_hour: read.read_hour,
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        }
    }
    Ok(Some(scan.reply(branch, Some(&selected))?))
}

#[cfg(test)]
mod tests;

/// Later aliases can add holders to a manual tag selection at any earlier day.
pub(crate) fn selection_complete(
    source: &Source,
    branch: u64,
    coverage: Option<&dashboard_base::prepared::coverage::Coverage>,
) -> bool {
    let replay = source.replay_ref();
    let last = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .map(|block| block.last_hour)
        .chain(
            replay
                .gaps
                .iter()
                .filter(|gap| gap.branch == branch)
                .map(|gap| gap.last_hour),
        )
        .max();
    coverage.is_some_and(|coverage| {
        coverage.complete && last.is_some_and(|last| coverage.last_hour >= last)
    })
}
