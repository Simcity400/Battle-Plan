//! Agency and roster changes use shared events. The production scan's latest
//! rows and identity catalogue continue after commits without retaining daily dates.
use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    series::{self, Accumulator, Reader},
};
use dashboard_text::metadata::check_cancel;
use replay_frame::frame::DecodedFrame;

const OBSERVED: u64 = 0;
const OPERATIVE_READ: u64 = 1;

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: ScannedAgencies,
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    /// Each tag's agency and roster keys of `keys`, as first asked.
    #[serde(skip)]
    tag_keys: HashMap<String, [u64; 2]>,
    first: Option<i32>,
    last: Option<i32>,
    active: BTreeMap<String, Identity>,
    membership_dirty: bool,
    unresolved_per_hour: usize,
    untext_per_hour: usize,
    unresolved_per_read: usize,
    unsupported_per_read: BTreeMap<u8, usize>,
    #[cfg(test)]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(test)]
    #[serde(skip)]
    test_countries: super::super::operative_rows::CountryInputs,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Params {
    pub selection: AgencyComparisonParams,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

impl Prepared {
    #[cfg(test)]
    pub fn observe(&mut self, source: &Source, frame: &DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, source.replay_ref().format);
        self.test_countries.update(frame, Some(&changes));
        let countries = std::mem::take(&mut self.test_countries);
        let result = self.observe_changes(source, frame, &changes, &countries);
        self.test_countries = countries;
        result
    }
    fn materialize(&mut self, hour: i32) {
        let read = self.scan.operative_read_hours.last().copied();
        for (tag, &identity) in &self.active {
            #[cfg(test)]
            super::super::intelligence_prepared::work(|work| work.history_visits += 1);
            let track = self
                .scan
                .tracks
                .get_mut(tag)
                .expect("current agency holder");
            track.last_identity.advance(hour);
            if let Some(segment) = track.segments.last_mut() {
                segment.last_hour = hour;
            }
            if let Some((_, at)) = track.last_value.as_mut() {
                *at = hour;
                if let Some(entry) = track.entries.last_mut() {
                    entry.last_hour = hour;
                }
            }
            if let Some((_, at, _)) = track.roster.as_mut() {
                *at = hour;
                if let Some(roster) = track.rosters.last_mut() {
                    roster.last_hour = hour;
                }
            }
            if let Some(held) = self.scan.held_rosters.get_mut(&identity) {
                held.hour = hour;
                if let Some(read) = read {
                    held.read_hour = read;
                }
            }
        }
    }
    pub fn observe_changes(
        &mut self,
        source: &Source,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:agencies");
        let previous = self.last;
        let changed = countries.changed
            || previous.is_none_or(|last| last.checked_add(1) != Some(frame.hour))
            || [
                TableId::IntelligenceAgencies,
                TableId::AgencyUpgradeLevels,
                TableId::LexicalTokens,
                TableId::OperativeMemberships,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()]);
        let read =
            membership_read(frame) && frame.complete::<tables::OperativeMemberships>().is_ok();
        self.membership_dirty |=
            countries.changed || changes.rows_changed[TableId::OperativeMemberships.index()];
        let changed = changed || (read && self.membership_dirty);
        self.values.advance(frame.hour)?;
        let active = if changed {
            if let Some(previous) = previous {
                self.materialize(previous);
            }
            let unresolved = self.scan.unresolved_rows;
            let untext = self.scan.untext_tags;
            // The countries resolve the tags they resolved at the last observation, which every
            // hour that changed them observes.
            let fresh = countries.changed || self.first.is_none();
            let mut active = if fresh {
                BTreeMap::new()
            } else {
                std::mem::take(&mut self.active)
            };
            let (per_read, unsupported) = self.scan.observe_frame_countries(
                frame,
                source.replay_ref(),
                countries,
                Some(changes),
                (&mut active, fresh),
            )?;
            self.unresolved_per_hour = self.scan.unresolved_rows - unresolved - per_read;
            self.untext_per_hour = self.scan.untext_tags - untext;
            if read {
                self.unresolved_per_read = per_read;
                self.unsupported_per_read = unsupported;
            }
            Some((active, fresh))
        } else {
            self.scan.unresolved_rows += self.unresolved_per_hour;
            self.scan.untext_tags += self.untext_per_hour;
            if read {
                self.scan.note_roster_read(frame.hour);
                self.scan.unresolved_rows += self.unresolved_per_read;
                for (kind, count) in &self.unsupported_per_read {
                    *self.scan.unsupported_rosters.entry(*kind).or_default() += count;
                }
            }
            if let Some(span) = self
                .scan
                .observed
                .last_mut()
                .filter(|span| Some(span.last_hour) == previous)
            {
                span.last_hour = frame.hour;
            }
            None
        };
        if read {
            self.membership_dirty = false;
        }
        self.first.get_or_insert(frame.hour);
        self.last = Some(frame.hour);
        let observed = self
            .scan
            .observed
            .last()
            .filter(|span| span.last_hour == frame.hour);
        if changed {
            self.values.observe(
                OBSERVED,
                frame.hour,
                observed.map(|span| i128::from(span.first_hour)),
                observed.is_some_and(|span| span.first_hour == frame.hour),
            );
        }
        let read = self.scan.operative_read_hours.last().copied();
        if changed || read == Some(frame.hour) || self.first == Some(frame.hour) {
            self.values
                .observe(OPERATIVE_READ, frame.hour, read.map(i128::from), false);
        }
        if read == Some(frame.hour) {
            self.values.event(OPERATIVE_READ, frame.hour, &frame.hour)?;
        }
        if let Some((active, fresh)) = active {
            for tag in self
                .active
                .keys()
                .filter(|tag| fresh && !active.contains_key(*tag))
            {
                for field in ["agency", "roster"] {
                    if let Some(&key) = self.keys.get(&series::field_key(tag, field)) {
                        self.values.observe(key, frame.hour, None, false);
                    }
                }
            }
            for tag in active.keys() {
                let track = self
                    .scan
                    .tracks
                    .get_mut(tag)
                    .expect("observed agency track");
                let [key, roster_key] = match self.tag_keys.get(tag.as_str()) {
                    Some(&keys) => keys,
                    None => {
                        let mut keys = [0; 2];
                        for (key, field) in keys.iter_mut().zip(["agency", "roster"]) {
                            let next = u64::try_from(self.keys.len())
                                .ok()
                                .and_then(|len| len.checked_add(2))
                                .ok_or_else(|| io::Error::other("Too many agency series"))?;
                            *key = *self
                                .keys
                                .entry(series::field_key(tag, field))
                                .or_insert(next);
                        }
                        self.tag_keys.insert(tag.clone(), keys);
                        keys
                    }
                };
                let entry = track
                    .entries
                    .last()
                    .filter(|entry| entry.last_hour == frame.hour);
                let value = entry.map(|entry| i128::from(entry.first_hour));
                if entry.is_some_and(|entry| entry.first_hour == frame.hour)
                    || self
                        .values
                        .state(key)
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                        != value
                    || value.is_none()
                {
                    self.values.observe(
                        key,
                        frame.hour,
                        value,
                        entry.is_some_and(|entry| entry.first_hour == frame.hour),
                    );
                }
                if let Some(entry) = entry.filter(|entry| entry.first_hour == frame.hour) {
                    #[cfg(test)]
                    super::super::intelligence_prepared::work(|work| work.emitted += 1);
                    self.values.event(key, frame.hour, entry)?;
                }
                let key = roster_key;
                let roster = track
                    .rosters
                    .last()
                    .filter(|roster| roster.last_hour == frame.hour);
                let value = roster.map(|roster| i128::from(roster.first_hour));
                if roster.is_some_and(|roster| roster.first_hour == frame.hour)
                    || self
                        .values
                        .state(key)
                        .and_then(|state| state.last)
                        .map(|run| run.value)
                        != value
                    || value.is_none()
                {
                    self.values.observe(
                        key,
                        frame.hour,
                        value,
                        roster.is_some_and(|roster| roster.first_hour == frame.hour),
                    );
                }
                if let Some(roster) = roster.filter(|roster| roster.first_hour == frame.hour) {
                    #[cfg(test)]
                    super::super::intelligence_prepared::work(|work| work.emitted += 1);
                    self.values.event(key, frame.hour, roster)?;
                }
                if track.entries.len() > 1 {
                    track.entries.drain(..track.entries.len() - 1);
                }
                if track.rosters.len() > 1 {
                    track.rosters.drain(..track.rosters.len() - 1);
                }
            }
            self.active = active;
        }
        if self.scan.observed.len() > 1 {
            self.scan.observed.drain(..self.scan.observed.len() - 1);
        }
        if self.scan.operative_read_hours.len() > 1 {
            self.scan
                .operative_read_hours
                .drain(..self.scan.operative_read_hours.len() - 1);
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
        if let Some(last) = self.last {
            self.materialize(last);
        }
        self.values.flush(
            writer,
            &format!("agency-comparison-{branch}-series"),
            0,
            cancelled,
        )?;
        if let Some(bounds) = self.first.zip(self.last) {
            series::write_catalogue(writer, "agency-comparison", branch, &self.keys, &bounds)?;
        }
        series::write_reply(writer, &format!("agency-comparison-{branch}"), 0, self)
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
    key: &store::SourceKey,
    branch: u64,
    params: &Params,
    cancelled: &AtomicBool,
) -> io::Result<Option<AgencyComparisonReply>> {
    AgencyComparison::validate(&params.selection).map_err(dashboard_core::invalid)?;
    let Some((keys, (first, last))) =
        series::read_catalogue::<(i32, i32)>(store, key, "agency-comparison", branch)?
    else {
        return Ok(None);
    };
    let Some(mut state) =
        series::read_reply::<Prepared>(store, key, &format!("agency-comparison-{branch}"), 0)?
    else {
        return Ok(None);
    };
    let first = params.first_hour.unwrap_or(first);
    let last = params.last_hour.unwrap_or(last);
    if first > last {
        return Err(dashboard_core::invalid("Invalid agency query range"));
    }
    let history = if params.selection.tags.is_empty() {
        None
    } else {
        Some(super::super::intelligence_prepared::histories(
            source, context, store, key, branch, cancelled,
        )?)
    };
    let mut reader = Reader::new(
        store,
        key,
        format!("agency-comparison-{branch}-series"),
        cancelled,
    )?;
    for track in state.scan.tracks.values_mut() {
        track.entries.clear();
        track.rosters.clear();
    }
    state.scan.observed = reader
        .runs(OBSERVED, first, last)?
        .into_iter()
        .map(|run| HourSpan {
            first_hour: run.first_hour,
            last_hour: run.last_hour,
        })
        .collect();
    let mut reads = BTreeSet::new();
    if let Some((_, row)) = reader.latest_event(OPERATIVE_READ, first)? {
        reads.insert(serde_json::from_value::<i32>(row)?);
    }
    for row in reader
        .events(&[OPERATIVE_READ], first, last, 0, u32::MAX)?
        .rows
    {
        reads.insert(serde_json::from_value::<i32>(row.row)?);
    }
    for (name, key) in keys {
        check_cancel(cancelled)?;
        let (tag, field): (String, String) = serde_json::from_str(&name)?;
        let track = state
            .scan
            .tracks
            .get_mut(&tag)
            .ok_or_else(|| dashboard_core::invalid("Missing prepared agency identity"))?;
        for span in reader.runs(key, first, last)? {
            let (_, row) = reader
                .latest_event(key, span.first_hour)?
                .ok_or_else(|| dashboard_core::invalid("Missing prepared agency event"))?;
            match field.as_str() {
                "agency" => {
                    let mut entry: RawEntry = serde_json::from_value(row)?;
                    entry.first_hour = span.first_hour;
                    entry.last_hour = span.last_hour;
                    track.entries.push(entry);
                }
                "roster" => {
                    let mut roster: AgencyRosterRun = serde_json::from_value(row)?;
                    roster.first_hour = span.first_hour;
                    roster.last_hour = span.last_hour;
                    reads.insert(roster.read_hour);
                    track.rosters.push(roster);
                }
                _ => return Err(dashboard_core::invalid("Unknown prepared agency field")),
            }
        }
    }
    state.scan.operative_read_hours = reads.into_iter().collect();
    Ok(Some(super::reply(
        source,
        &params.selection,
        context,
        cancelled,
        &state.scan,
        history.as_ref(),
    )?))
}

#[cfg(test)]
mod tests;
