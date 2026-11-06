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
    /// The tags the countries resolve, each observed through the last hour: a tag no hour
    /// changed goes on as it was, and is told only where something of it changes.
    active: BTreeMap<String, Identity>,
    /// Each active tag by the slot it holds.
    #[serde(skip)]
    holders: HashMap<u32, Vec<String>>,
    membership_dirty: bool,
    /// A roster held may not go on to the next membership read: that read counts every tag.
    roster_reset: bool,
    /// Whether the memberships were complete at the last hour.
    #[serde(skip)]
    memberships: Option<bool>,
    /// Every tag has been observed since this state was made or read back.
    #[serde(skip)]
    bound: bool,
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

    /// Carries every active tag's track through `hour`.
    fn materialize(&mut self, hour: i32) {
        let tags: Vec<String> = self.active.keys().cloned().collect();
        for tag in &tags {
            self.materialize_tag(tag, hour);
        }
    }

    /// Carries active `tag`'s track, and the roster its country holds, through `hour`: each
    /// went on unchanged from its last observation.
    fn materialize_tag(&mut self, tag: &str, hour: i32) {
        let Some(&identity) = self.active.get(tag) else {
            return;
        };
        #[cfg(test)]
        super::super::intelligence_prepared::work(|work| work.history_visits += 1);
        let read = self.scan.operative_read_hours.last().copied();
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

    fn index_holders(&mut self) {
        self.holders.clear();
        for (tag, &(slot, _)) in &self.active {
            self.holders.entry(slot).or_default().push(tag.clone());
        }
    }

    /// Observes `frame` of the row path, its tables read by the scan's reader at each hour that
    /// changed what it reads, every tag gone through there.
    pub fn observe_changes(
        &mut self,
        source: &Source,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
    ) -> io::Result<()> {
        let census = Census::of(frame);
        let changed = !self.bound || self.changed(frame.hour, changes, countries, census);
        let mut reader = std::mem::take(&mut self.scan.reader);
        let result = (|| {
            let read = match countries.complete() {
                Some(_) if changed => Some(reader.read(frame, countries, Some(changes))?),
                Some(_) => Some(reader.current()),
                None => None,
            };
            let touched = AgencyTouched {
                all: changed,
                slots: BTreeSet::new(),
            };
            self.observe_with(
                source, frame.hour, changes, countries, census, read, &touched,
            )
        })();
        self.scan.reader = reader;
        result
    }

    /// Observes `hour`, whose change facts are `changes`, from the agency scan's `read` of it,
    /// `touched` naming what that read changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe_read(
        &mut self,
        source: &Source,
        hour: i32,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        census: Census,
        read: &AgencyRead,
        touched: &AgencyTouched,
    ) -> io::Result<()> {
        let read = countries.complete().map(|_| read);
        self.observe_with(source, hour, changes, countries, census, read, touched)
    }

    /// Whether `hour` changed what the scan reads, as the agencies' observed coverage is told.
    fn changed(
        &self,
        hour: i32,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        census: Census,
    ) -> bool {
        countries.changed
            || self
                .last
                .is_none_or(|last| last.checked_add(1) != Some(hour))
            || [
                TableId::IntelligenceAgencies,
                TableId::AgencyUpgradeLevels,
                TableId::LexicalTokens,
                TableId::OperativeMemberships,
            ]
            .iter()
            .any(|table| changes.rows_changed[table.index()])
            || (census.read
                && census.complete
                && (self.membership_dirty
                    || countries.changed
                    || changes.rows_changed[TableId::OperativeMemberships.index()]))
    }

    /// Observes `hour`: every tag where the countries, the hours or what the scan holds of
    /// them may not go on, else the tags of the slots `touched` names; the others go on as
    /// they were.
    #[allow(clippy::too_many_arguments)]
    fn observe_with(
        &mut self,
        source: &Source,
        hour: i32,
        changes: &FrameChanges,
        countries: &super::super::operative_rows::CountryInputs,
        census: Census,
        read: Option<&AgencyRead>,
        touched: &AgencyTouched,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("intelligence:agencies");
        let previous = self.last;
        let gap = previous.is_none_or(|last| last.checked_add(1) != Some(hour));
        let changed = self.changed(hour, changes, countries, census);
        let membership_read = census.read && census.complete;
        self.membership_dirty |=
            countries.changed || changes.rows_changed[TableId::OperativeMemberships.index()];
        if membership_read {
            self.membership_dirty = false;
        }
        // The countries resolve the tags they resolved at the last observation, which every
        // hour that changed them observes.
        let fresh = countries.changed || self.first.is_none();
        // Counts no longer known stop every holder's roster: the read counting them again
        // starts each one.
        let unknown_counts = read
            .and_then(|read| read.roster.as_ref())
            .is_none_or(|(_, unresolved, _)| *unresolved != 0);
        let all = fresh
            || gap
            || !self.bound
            || touched.all
            || self.memberships != Some(census.complete)
            || (membership_read && (self.roster_reset || unknown_counts));
        self.bound = true;
        self.memberships = Some(census.complete);
        self.values.advance(hour)?;
        let tags = if all {
            if let Some(previous) = previous {
                self.materialize(previous);
            }
            None
        } else {
            let mut tags = BTreeSet::new();
            for slot in &touched.slots {
                if let Some(holders) = self.holders.get(slot) {
                    tags.extend(holders.iter().cloned());
                }
            }
            if let Some(previous) = previous {
                for tag in &tags {
                    self.materialize_tag(tag, previous);
                }
            }
            Some(tags.into_iter().collect::<Vec<_>>())
        };
        let mut active = if fresh {
            BTreeMap::new()
        } else {
            std::mem::take(&mut self.active)
        };
        let scanned = self.scan.observe_read(
            hour,
            source.replay_ref(),
            countries,
            census,
            read,
            tags.as_deref(),
            (&mut active, fresh),
        );
        if let Err(error) = scanned {
            self.active = active;
            return Err(error);
        }
        if membership_read {
            self.roster_reset = unknown_counts;
        } else if all || !census.complete {
            self.roster_reset = true;
        }
        self.first.get_or_insert(hour);
        self.last = Some(hour);
        let observed = self
            .scan
            .observed
            .last()
            .filter(|span| span.last_hour == hour);
        if changed {
            self.values.observe(
                OBSERVED,
                hour,
                observed.map(|span| i128::from(span.first_hour)),
                observed.is_some_and(|span| span.first_hour == hour),
            );
        }
        let read = self.scan.operative_read_hours.last().copied();
        if changed || read == Some(hour) || self.first == Some(hour) {
            self.values
                .observe(OPERATIVE_READ, hour, read.map(i128::from), false);
        }
        if read == Some(hour) {
            self.values.event(OPERATIVE_READ, hour, &hour)?;
        }
        if fresh {
            for tag in self.active.keys().filter(|tag| !active.contains_key(*tag)) {
                for field in ["agency", "roster"] {
                    if let Some(&key) = self.keys.get(&series::field_key(tag, field)) {
                        self.values.observe(key, hour, None, false);
                    }
                }
            }
        }
        self.active = active;
        if fresh || self.holders.is_empty() {
            self.index_holders();
        }
        let observed: Vec<String> = match tags {
            Some(tags) => tags,
            None => self.active.keys().cloned().collect(),
        };
        for tag in &observed {
            self.publish(tag, hour)?;
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

    /// Tells the series what observing `tag` at `hour` changed of its agency and roster.
    fn publish(&mut self, tag: &str, hour: i32) -> io::Result<()> {
        let Some(track) = self.scan.tracks.get_mut(tag) else {
            return Ok(());
        };
        let [key, roster_key] = match self.tag_keys.get(tag) {
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
                self.tag_keys.insert(tag.to_owned(), keys);
                keys
            }
        };
        let entry = track.entries.last().filter(|entry| entry.last_hour == hour);
        let value = entry.map(|entry| i128::from(entry.first_hour));
        if entry.is_some_and(|entry| entry.first_hour == hour)
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
                hour,
                value,
                entry.is_some_and(|entry| entry.first_hour == hour),
            );
        }
        if let Some(entry) = entry.filter(|entry| entry.first_hour == hour) {
            #[cfg(test)]
            super::super::intelligence_prepared::work(|work| work.emitted += 1);
            self.values.event(key, hour, entry)?;
        }
        let roster = track
            .rosters
            .last()
            .filter(|roster| roster.last_hour == hour);
        let value = roster.map(|roster| i128::from(roster.first_hour));
        if roster.is_some_and(|roster| roster.first_hour == hour)
            || self
                .values
                .state(roster_key)
                .and_then(|state| state.last)
                .map(|run| run.value)
                != value
            || value.is_none()
        {
            self.values.observe(
                roster_key,
                hour,
                value,
                roster.is_some_and(|roster| roster.first_hour == hour),
            );
        }
        if let Some(roster) = roster.filter(|roster| roster.first_hour == hour) {
            #[cfg(test)]
            super::super::intelligence_prepared::work(|work| work.emitted += 1);
            self.values.event(roster_key, hour, roster)?;
        }
        if track.entries.len() > 1 {
            track.entries.drain(..track.entries.len() - 1);
        }
        if track.rosters.len() > 1 {
            track.rosters.drain(..track.rosters.len() - 1);
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
        reads.insert(serde_json::from_slice::<i32>(&row)?);
    }
    for row in reader
        .events(&[OPERATIVE_READ], first, last, 0, u32::MAX)?
        .rows
    {
        reads.insert(serde_json::from_slice::<i32>(&row.row)?);
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
                    let mut entry: RawEntry = serde_json::from_slice(&row)?;
                    entry.first_hour = span.first_hour;
                    entry.last_hour = span.last_hour;
                    track.entries.push(entry);
                }
                "roster" => {
                    let mut roster: AgencyRosterRun = serde_json::from_slice(&row)?;
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
