//! Fuel and current consumption share the production all-country observation.
use super::*;
use dashboard_base::prepared::query::progressive;
use dashboard_base::prepared::series::{self, Accumulator, Reader, Run};
use dashboard_base::prepared_chart::ChartSummary;
use serde_json::Value;

const UNSETTLED_HOURS: i32 = DAY_HOURS - 1;
const CURRENT: &str = "fuel";
const DAYS: &str = "fuel-days";
const METADATA: &str = "fuel";

/// Complete tables used by the stateless fuel level and capacity sample.
pub(crate) const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFuel,
];

/// Sample cached amounts without completed-day or supplying-read inference.
#[allow(clippy::type_complexity)]
pub(crate) fn sample(
    frame: &replay::frame::DecodedFrame,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<(u32, u32)>, Option<i128>)>> {
    let tables = frame
        .complete::<tables::Countries>()
        .ok()
        .zip(frame.complete::<tables::CountryLinks>().ok());
    let slots = tables.map(|(_, links)| resolved_slots(links));
    let country_keys =
        tables.map(|(countries, _)| dashboard_text::metadata::CountryKeys::read(countries));
    let tagged = country_keys
        .as_ref()
        .zip(slots.as_ref())
        .map(|(countries, slots)| tagged_identities(countries.iter(), slots));
    let amounts = tables
        .and_then(|_| frame.complete::<tables::CountryFuel>().ok())
        .map_or_else(Vec::new, amount_rows);
    sample_rows(
        tagged
            .iter()
            .flat_map(|tagged| tagged.iter())
            .filter_map(|(tag, lookup)| std::str::from_utf8(tag).ok().map(|tag| (tag, *lookup))),
        &amounts,
        |raw| slots.as_ref().and_then(|slots| slots.get(&raw)).copied(),
        requested,
    )
}

fn amount_rows(rows: &[CountryFuelRow]) -> Vec<(u32, i64, i64)> {
    rows.iter()
        .map(|row| (row.country, row.current_raw, row.maximum_raw))
        .collect()
}

#[allow(clippy::type_complexity)]
fn sample_rows<'a>(
    countries: impl Iterator<Item = (&'a str, Option<Identity>)>,
    amounts: &[(u32, i64, i64)],
    slot: impl Fn(u32) -> Option<u32>,
    requested: Option<&[(String, String)]>,
) -> io::Result<BTreeMap<(String, String), (Option<Identity>, Option<i128>)>> {
    if requested.is_some_and(|keys| {
        keys.iter()
            .any(|(_, field)| !matches!(field.as_str(), "fuel" | "capacity"))
    }) {
        return Err(dashboard_core::invalid(
            "Unknown stateless fuel sample field",
        ));
    }
    let wanted = requested.map(|keys| keys.iter().collect::<BTreeSet<_>>());
    let mut result = BTreeMap::new();
    let by_slot = fuel_by_slot(amounts, |row| row.0, slot);
    for (tag, identity) in countries {
        let row = identity.and_then(|(slot, _)| by_slot.get(&slot));
        for (field, value) in [
            ("fuel", row.map(|row| i128::from(row.1))),
            ("capacity", row.map(|row| i128::from(row.2))),
        ] {
            let key = (tag.to_owned(), field.to_owned());
            if wanted.as_ref().is_none_or(|keys| keys.contains(&key)) {
                result.insert(key, (identity, value));
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

fn actor_binding(country: &str, instance: u32) -> String {
    serde_json::to_string(&(country, instance)).expect("actor bindings serialize")
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct CountryMetadata {
    has_reads: bool,
    left_out: [usize; 3],
    tail: Vec<FuelRead>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct ActorMetadata {
    tags: BTreeSet<String>,
    conflict: bool,
    latest: Option<(String, i32, i32)>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct Metadata {
    first: Option<i32>,
    last: Option<i32>,
    settled_until: Option<i32>,
    countries: BTreeMap<String, CountryMetadata>,
    actors: BTreeMap<String, ActorMetadata>,
    unnamed: BTreeSet<u32>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    keys: BTreeMap<String, u64>,
    metadata: Metadata,
    // One settled kept day anchors the existing greedy overlap rule.
    anchors: BTreeMap<String, RawRead>,
    current: Accumulator,
    days: Accumulator,
    active_actors: BTreeSet<String>,
    settling: BTreeSet<String>,
    #[serde(skip)]
    amounts: Vec<(u32, i64, i64)>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
fn key(keys: &mut BTreeMap<String, u64>, tag: &str, measure: &str) -> u64 {
    let next = keys.len() as u64;
    *keys.entry(series::field_key(tag, measure)).or_insert(next)
}

impl Prepared {
    pub fn buffered_bytes(&self) -> usize {
        self.current.bytes() + self.days.bytes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, format);
        self.observe_changes(frame, format, &changes, cancelled)
    }
    pub fn observe_changes(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        changes: &dashboard_base::prepared::frame_changes::FrameChanges,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let previous_hour = self.metadata.last;
        self.current.advance(frame.hour)?;
        let previous_levels = if self.scan.country_keys.is_some()
            && (changes.rows_changed[TableId::CountryFuel.index()]
                || changes.rows_changed[TableId::CountryLinks.index()])
        {
            Some(
                sample_rows(
                    self.scan
                        .active
                        .iter()
                        .map(|tag| (tag.as_str(), self.scan.countries[tag].identity)),
                    &self.amounts,
                    |raw| {
                        self.scan
                            .slots
                            .as_ref()
                            .and_then(|slots| slots.get(&raw))
                            .copied()
                    },
                    None,
                )?
                .into_iter()
                .filter(|((_, field), (_, value))| field == "fuel" && value.is_some())
                .map(|((tag, _), _)| tag)
                .collect::<BTreeSet<_>>(),
            )
        } else {
            None
        };
        let identities_changed = self
            .scan
            .observe_changes(frame, format, changes, cancelled)?;
        if changes.rows_changed[TableId::CountryFuel.index()] {
            self.amounts = frame.complete::<tables::CountryFuel>().map_or_else(
                |_| Vec::new(),
                |rows| {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += rows.len();
                    }
                    amount_rows(rows)
                },
            );
        }
        let samples = if !self.scan.active.is_empty()
            && (identities_changed
                || changes.rows_changed[TableId::CountryFuel.index()]
                || (changes.slot_read[TableId::CountryFuel.index()]
                    && frame.complete::<tables::CountryFuel>().is_ok()))
        {
            Some(sample_rows(
                self.scan
                    .active
                    .iter()
                    .map(|tag| (tag.as_str(), self.scan.countries[tag].identity)),
                &self.amounts,
                |raw| {
                    self.scan
                        .slots
                        .as_ref()
                        .and_then(|slots| slots.get(&raw))
                        .copied()
                },
                None,
            )?)
        } else {
            None
        };
        self.metadata.first.get_or_insert(frame.hour);
        self.metadata.last = Some(frame.hour);
        let mut affected = self.scan.updated.clone();
        if let Some(samples) = &samples {
            affected.extend(samples.keys().map(|(tag, _)| tag.clone()));
        }
        let mut actors: BTreeMap<String, Vec<(String, demand::FuelConsumptionRead)>> =
            BTreeMap::new();
        for tag in affected {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let Some(country) = self.scan.countries.get_mut(&tag) else {
                continue;
            };
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.history_visits += 1;
            }
            let metadata = self.metadata.countries.entry(tag.clone()).or_default();
            if let Some(read) = country
                .result
                .reads
                .last()
                .filter(|read| read.last_hour == frame.hour)
            {
                metadata.has_reads = true;
                let starts = read.starts_segment && read.first_hour == frame.hour;
                for (measure, value) in [
                    ("read", Some(i128::from(read.first_hour))),
                    (
                        "fuel",
                        samples
                            .as_ref()
                            .and_then(|samples| samples.get(&(tag.clone(), "fuel".into())))
                            .and_then(|sample| sample.1),
                    ),
                    (
                        "capacity",
                        samples
                            .as_ref()
                            .and_then(|samples| samples.get(&(tag.clone(), "capacity".into())))
                            .and_then(|sample| sample.1),
                    ),
                ] {
                    observe_value(
                        &mut self.current,
                        key(&mut self.keys, &tag, measure),
                        frame.hour,
                        value,
                        starts,
                        if measure == "read" {
                            Some(true)
                        } else {
                            previous_levels.as_ref().map(|known| known.contains(&tag))
                        },
                        #[cfg(any(test, feature = "test-support"))]
                        &mut self.work,
                    );
                }
                if read.first_hour == frame.hour {
                    self.settling.insert(tag.clone());
                }
            } else if self.scan.updated.contains(&tag) {
                for measure in ["read", "fuel", "capacity"] {
                    if let Some(&key) = self.keys.get(&series::field_key(&tag, measure)) {
                        self.current.observe(key, frame.hour, None, false);
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            }
            if self.scan.updated.contains(&tag)
                && let Some(read) = country
                    .result
                    .consumption
                    .last()
                    .filter(|read| read.last_hour == frame.hour)
            {
                let read = demand::FuelConsumptionRead::from(read);
                let previous = country
                    .result
                    .consumption
                    .iter()
                    .rev()
                    .nth(1)
                    .map(demand::FuelConsumptionRead::from);
                let starts = read.starts_segment && read.first_hour == frame.hour;
                let read_key = key(&mut self.keys, &tag, "consumptionRead");
                observe_value(
                    &mut self.current,
                    read_key,
                    frame.hour,
                    Some(i128::from(read.first_hour)),
                    starts,
                    Some(true),
                    #[cfg(any(test, feature = "test-support"))]
                    &mut self.work,
                );
                if read.first_hour == frame.hour {
                    self.current.event(read_key, frame.hour, &read)?;
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.emitted += 1;
                    }
                }
                for kind in 0..3 {
                    observe_value(
                        &mut self.current,
                        key(&mut self.keys, &tag, &format!("consumption-{kind}")),
                        frame.hour,
                        read.reasons[kind]
                            .is_none()
                            .then(|| read.positive_daily_raw[kind].parse::<i128>())
                            .transpose()
                            .map_err(io::Error::other)?,
                        starts,
                        Some(
                            previous
                                .as_ref()
                                .is_some_and(|read| read.reasons[kind].is_none()),
                        ),
                        #[cfg(any(test, feature = "test-support"))]
                        &mut self.work,
                    );
                }
            } else if self.scan.updated.contains(&tag) {
                for measure in [
                    "consumptionRead",
                    "consumption-0",
                    "consumption-1",
                    "consumption-2",
                ] {
                    if let Some(&key) = self.keys.get(&series::field_key(&tag, measure)) {
                        self.current.observe(key, frame.hour, None, false);
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            }
            // The scan needs the open consumption read, never its history.
            let count = country.result.consumption.len();
            if count > 1 {
                country.result.consumption.drain(..count - 1);
            }
        }
        // Only a supplying read or an identity/availability transition can
        // change actor joins. Held hours extend through the shared frontier.
        if !self.scan.updated.is_empty() {
            for tag in &self.scan.active {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.history_visits += 1;
                }
                let country = &self.scan.countries[tag];
                if let Some(read) = country
                    .result
                    .consumption
                    .last()
                    .filter(|read| read.last_hour == frame.hour)
                {
                    let read = demand::FuelConsumptionRead::from(read);
                    if let Some(instance) = read.country_instance {
                        actors
                            .entry(actor_binding(&read.country, instance))
                            .or_default()
                            .push((tag.clone(), read));
                    }
                }
            }
            let mut active_actors = BTreeSet::new();
            for (actor, reads) in actors {
                let metadata = self.metadata.actors.entry(actor.clone()).or_default();
                metadata
                    .tags
                    .extend(reads.iter().map(|(tag, _)| tag.clone()));
                metadata.conflict |= reads.len() != 1;
                if let [(tag, read)] = reads.as_slice() {
                    let joined = metadata
                        .latest
                        .as_ref()
                        .is_some_and(|(previous_tag, _, _)| {
                            self.active_actors.contains(&actor)
                                && previous_hour.and_then(|hour| hour.checked_add(1))
                                    == Some(frame.hour)
                                && (previous_tag != tag
                                    || !read.starts_segment
                                    || read.first_hour != frame.hour)
                        });
                    let first = if joined {
                        metadata.latest.as_ref().unwrap().2
                    } else {
                        frame.hour
                    };
                    observe_value(
                        &mut self.current,
                        key(&mut self.keys, &actor, "actorSegment"),
                        frame.hour,
                        Some(i128::from(first)),
                        !joined,
                        Some(true),
                        #[cfg(any(test, feature = "test-support"))]
                        &mut self.work,
                    );
                    active_actors.insert(actor.clone());
                    metadata.latest = Some((tag.clone(), frame.hour, first));
                } else {
                    metadata.latest = None;
                }
            }
            for actor in self.active_actors.difference(&active_actors) {
                self.current.observe(
                    key(&mut self.keys, actor, "actorSegment"),
                    frame.hour,
                    None,
                    false,
                );
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.emitted += 1;
                }
            }
            self.active_actors = active_actors;
        }
        if let Some(cutoff) = frame.hour.checked_sub(UNSETTLED_HOURS) {
            self.settle(cutoff, format, cancelled)?;
        }
        Ok(())
    }
    fn settle(
        &mut self,
        cutoff: i32,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        let from = self
            .metadata
            .settled_until
            .map_or(i32::MIN, |hour| hour.saturating_add(1));
        // A gap can settle different supplying dates in country order. The
        // shared batch must start before all of them, not at its first row.
        self.days
            .through(from.max(self.metadata.first.unwrap_or(cutoff)).min(cutoff));
        let mut settled = Vec::new();
        for tag in &self.settling {
            let country = self.scan.countries.get_mut(tag).unwrap();
            dashboard_text::metadata::check_cancel(cancelled)?;
            let mut decided: Vec<_> = self
                .anchors
                .get(tag)
                .into_iter()
                .cloned()
                .chain(
                    country
                        .result
                        .reads
                        .iter()
                        .filter(|read| {
                            self.anchors
                                .get(tag)
                                .is_none_or(|anchor| anchor.first_hour != read.first_hour)
                        })
                        .cloned(),
                )
                .collect();
            keep_days(format, &mut decided);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.interpreter_calls += decided.len();
            }
            for read in &mut country.result.reads {
                if read.first_hour > cutoff {
                    continue;
                }
                let decision = decided
                    .iter()
                    .find(|candidate| candidate.first_hour == read.first_hour)
                    .unwrap();
                // Previously settled decisions are immutable; only the undecided
                // supplying hours are published after their 23-hour lookahead.
                if self
                    .metadata
                    .settled_until
                    .is_none_or(|hour| read.first_hour > hour)
                {
                    read.day = decision.day.clone();
                    read.left_out = decision.left_out;
                    self.days.event(
                        key(&mut self.keys, tag, "read"),
                        read.first_hour,
                        &FuelRead::from(&*read),
                    )?;
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.emitted += 1;
                    }
                    if let Some(reason) = read.left_out {
                        let index = match reason {
                            DayLeftOut::Overlaps => 0,
                            DayLeftOut::NotUpdated => 1,
                            DayLeftOut::OwnershipNotRecorded => 2,
                        };
                        self.metadata.countries.get_mut(tag).unwrap().left_out[index] += 1;
                    }
                    if read.day.is_some() {
                        self.anchors.insert(tag.clone(), read.clone());
                    }
                }
                let first = read.first_hour.max(from);
                let last = read.last_hour.min(cutoff);
                if first <= last {
                    for (measure, value) in day_values(read) {
                        if measure.starts_with("strip")
                            && !(first..=last).contains(&read.first_hour)
                        {
                            continue;
                        }
                        self.days.push(
                            key(&mut self.keys, tag, &measure),
                            Run::new(
                                if measure.starts_with("strip") {
                                    read.first_hour
                                } else {
                                    first
                                },
                                if measure.starts_with("strip") {
                                    read.first_hour
                                } else {
                                    last
                                },
                                value,
                                read.starts_segment && first == read.first_hour,
                            ),
                        );
                        #[cfg(any(test, feature = "test-support"))]
                        {
                            self.work.emitted += 1;
                        }
                    }
                }
            }
            // Keep only unsettled supplying reads and the open scan continuation.
            let last = country.result.reads.last().map(|read| read.first_hour);
            country
                .result
                .reads
                .retain(|read| read.last_hour > cutoff || Some(read.first_hour) == last);
            if country.result.reads.iter().all(|read| {
                read.first_hour <= cutoff && (read.day.is_none() || read.last_hour <= cutoff)
            }) {
                settled.push(tag.clone());
            }
        }
        for tag in settled {
            self.settling.remove(&tag);
        }
        self.days.through(cutoff);
        self.metadata.settled_until = Some(cutoff);
        Ok(())
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.current
            .flush(writer, &format!("{CURRENT}-{branch}"), 0, cancelled)?;
        self.days
            .flush(writer, &format!("{DAYS}-{branch}"), 0, cancelled)?;
        let mut metadata = self.metadata.clone();
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.history_visits += self.scan.unnamed.len()
                + self.metadata.countries.len()
                + self.metadata.actors.len();
        }
        metadata.unnamed = self.scan.unnamed.clone();
        for (tag, country) in &self.scan.countries {
            let mut reads: Vec<_> = self
                .anchors
                .get(tag)
                .into_iter()
                .cloned()
                .chain(
                    country
                        .result
                        .reads
                        .iter()
                        .filter(|read| {
                            self.anchors
                                .get(tag)
                                .is_none_or(|anchor| anchor.first_hour != read.first_hour)
                        })
                        .cloned(),
                )
                .collect();
            keep_days(format, &mut reads);
            metadata.countries.get_mut(tag).unwrap().tail = reads
                .iter()
                .filter(|read| {
                    metadata
                        .settled_until
                        .is_none_or(|hour| read.first_hour > hour || read.last_hour > hour)
                })
                .map(FuelRead::from)
                .collect();
        }
        series::write_catalogue(writer, METADATA, branch, &self.keys, &metadata)
    }
}

fn observe_value(
    current: &mut Accumulator,
    key: u64,
    hour: i32,
    value: Option<i128>,
    starts: bool,
    was_known: Option<bool>,
    #[cfg(any(test, feature = "test-support"))]
    work: &mut dashboard_base::prepared::observer_tests::Work,
) {
    // Unknown closes active membership but retains the last known run. A cold
    // runtime projection after resume cannot prove that membership was open or
    // closed, so its first actual value must authenticate the persisted key.
    let changed = match (value.is_some(), was_known) {
        (_, None) => true,
        (false, Some(false)) => false,
        (true, Some(false)) => true,
        _ => {
            current
                .state(key)
                .and_then(|state| state.last)
                .map(|run| run.value)
                != value
        }
    };
    if starts || changed {
        current.observe(key, hour, value, starts);
        #[cfg(any(test, feature = "test-support"))]
        {
            work.emitted += 1;
        }
    }
}

fn day_values(read: &RawRead) -> Vec<(String, i128)> {
    let Some(day) = &read.day else {
        return Vec::new();
    };
    let (gained, burned, received, other, awards) = match day {
        RawDay::Ring {
            gained,
            burned,
            received,
            other,
        } => (
            i128::from(*gained),
            i128::from(*burned),
            received.map(i128::from),
            i128::from(*other),
            None,
        ),
        RawDay::Awards(awards) => (
            awards.own_gain,
            awards.consumed,
            awards.received,
            awards.transfers[3],
            Some(awards.as_ref()),
        ),
    };
    let mut values = Vec::new();
    // The strip books a day only at its supplying read, not at carried hours.
    values.push(("stripGain".into(), gained));
    values.push(("stripBurn".into(), burned));
    let quantity = i128::from(QUANTITY_UNITS);
    let gain = i128::from(GAIN_UNITS);
    for (index, value) in received.into_iter().enumerate() {
        values.push((format!("consumer-{index}"), (value * quantity).max(0)));
    }
    let received_sum: i128 = received.into_iter().sum();
    if let Some(awards) = awards {
        for (index, value) in awards.gains.into_iter().enumerate() {
            values.push((format!("gain-{index}"), value.max(0)));
        }
        values.push(("incoming".into(), awards.transfers[0].max(0)));
        values.push(("outgoing".into(), (awards.transfers[1] * gain).max(0)));
        values.push(("flat".into(), (awards.transfers[2] * gain).max(0)));
        values.push((
            "other".into(),
            ((other - awards.transfers[1] - awards.transfers[2]) * gain).max(0),
        ));
    } else {
        values.push(("unidentified".into(), gained.max(0)));
        values.push(("other".into(), (other * gain).max(0)));
    }
    values.push((
        "conversion".into(),
        (burned * gain - other * gain - received_sum * quantity).max(0),
    ));
    values
}
fn day_measure(measure: &str) -> bool {
    !matches!(measure, "fuel" | "capacity") && !measure.starts_with("consumption-")
}
fn parse_raw(raw: &str) -> io::Result<i128> {
    raw.parse().map_err(io::Error::other)
}
fn raw_read(read: FuelRead) -> io::Result<RawRead> {
    let day = read
        .day
        .as_ref()
        .map(|day| -> io::Result<RawDay> {
            Ok(if let Some(awards) = &day.awards {
                RawDay::Awards(Box::new(AwardSums {
                    gains: awards
                        .gains_raw
                        .each_ref()
                        .map(|value| parse_raw(value))
                        .into_iter()
                        .collect::<io::Result<Vec<_>>>()?
                        .try_into()
                        .unwrap(),
                    own_gain: parse_raw(&awards.own_gain_raw)?,
                    own_gain_quantity: parse_raw(&awards.own_gain_quantity_raw)?,
                    transfers: awards
                        .transfers_raw
                        .each_ref()
                        .map(|value| parse_raw(value))
                        .into_iter()
                        .collect::<io::Result<Vec<_>>>()?
                        .try_into()
                        .unwrap(),
                    consumed: parse_raw(&awards.consumed_raw)?,
                    received: awards
                        .received_raw
                        .each_ref()
                        .map(|value| parse_raw(value))
                        .into_iter()
                        .collect::<io::Result<Vec<_>>>()?
                        .try_into()
                        .unwrap(),
                }))
            } else {
                RawDay::Ring {
                    gained: day.gained_raw.parse().map_err(io::Error::other)?,
                    burned: day.burned_raw.parse().map_err(io::Error::other)?,
                    received: day
                        .received_raw
                        .each_ref()
                        .map(|value| value.parse())
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(io::Error::other)?
                        .try_into()
                        .unwrap(),
                    other: day.other_raw.parse().map_err(io::Error::other)?,
                }
            })
        })
        .transpose()?;
    Ok(RawRead {
        country: read.country.parse().map_err(io::Error::other)?,
        country_instance: read.country_instance,
        continuity_start_hour: read.continuity_start_hour,
        first_hour: read.first_hour,
        last_hour: read.last_hour,
        starts_segment: read.starts_segment,
        fuel: read.fuel_raw.parse().map_err(io::Error::other)?,
        capacity: read.capacity_raw.parse().map_err(io::Error::other)?,
        lend_lease: read.lend_lease_raw.parse().map_err(io::Error::other)?,
        day,
        left_out: read.left_out,
        awards_reason: read.awards_reason.map(Arc::from),
    })
}

struct Reads<'a> {
    keys: BTreeMap<String, u64>,
    metadata: Metadata,
    current: Reader<'a>,
    days: Reader<'a>,
}
impl<'a> Reads<'a> {
    fn open(
        store: &'a store::Store,
        source_key: &'a store::SourceKey,
        branch: u64,
        cancelled: &'a AtomicBool,
    ) -> io::Result<Option<Self>> {
        let Some((keys, metadata)) = series::read_catalogue(store, source_key, METADATA, branch)?
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            keys,
            metadata,
            current: Reader::new(store, source_key, format!("{CURRENT}-{branch}"), cancelled)?,
            days: Reader::new(store, source_key, format!("{DAYS}-{branch}"), cancelled)?,
        }))
    }
    fn fuel(&mut self, tag: &str, hour: i32) -> io::Result<Option<FuelRead>> {
        let Some(&key) = self.keys.get(&series::field_key(tag, "read")) else {
            return Ok(None);
        };
        let Some(run) = self.current.value_at(key, hour)? else {
            return Ok(None);
        };
        let supplying = i32::try_from(run.value).map_err(io::Error::other)?;
        let tail = self.metadata.countries.get(tag).and_then(|country| {
            country
                .tail
                .iter()
                .find(|read| read.first_hour == supplying)
        });
        let mut read = match tail {
            Some(read) => read.clone(),
            None => {
                let Some((date, value)) = self.days.latest_event(key, supplying)? else {
                    return Ok(None);
                };
                if date != supplying {
                    return Err(dashboard_core::invalid(
                        "Prepared fuel supplying read is missing",
                    ));
                }
                serde_json::from_value(value)?
            }
        };
        read.last_hour = run.last_hour;
        Ok(Some(read))
    }
    fn consumption(
        &mut self,
        tag: &str,
        hour: i32,
    ) -> io::Result<Option<demand::FuelConsumptionRead>> {
        let Some(&key) = self.keys.get(&series::field_key(tag, "consumptionRead")) else {
            return Ok(None);
        };
        let Some(run) = self.current.value_at(key, hour)? else {
            return Ok(None);
        };
        let Some((date, value)) = self.current.latest_event(key, hour)? else {
            return Ok(None);
        };
        if i128::from(date) != run.value {
            return Err(dashboard_core::invalid(
                "Prepared fuel consumption supplying read is missing",
            ));
        }
        let mut read: demand::FuelConsumptionRead = serde_json::from_value(value)?;
        read.last_hour = run.last_hour;
        Ok(Some(read))
    }
    fn actor_segment(
        &mut self,
        read: &demand::FuelConsumptionRead,
        hour: i32,
    ) -> io::Result<Option<dashboard_base::products::country_groups::HourSpan>> {
        let Some(instance) = read.country_instance else {
            return Ok(None);
        };
        let actor = actor_binding(&read.country, instance);
        if self
            .metadata
            .actors
            .get(&actor)
            .is_some_and(|actor| actor.conflict)
        {
            return Ok(None);
        }
        let Some(&key) = self.keys.get(&series::field_key(&actor, "actorSegment")) else {
            return Ok(None);
        };
        Ok(self.current.value_at(key, hour)?.map(|run| {
            dashboard_base::products::country_groups::HourSpan {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
            }
        }))
    }
    fn fuel_window(
        &mut self,
        tag: &str,
        first: i32,
        last: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<FuelRead>> {
        let Some(&key) = self.keys.get(&series::field_key(tag, "read")) else {
            return Ok(Vec::new());
        };
        let mut hours = BTreeSet::new();
        if let Some(run) = self.current.value_at(key, first)? {
            hours.insert(i32::try_from(run.value).map_err(io::Error::other)?);
        }
        // A 720-hour window has a bounded number of supplying hours; every row is delivered.
        let page = self.days.events(
            &[key],
            first,
            last,
            0,
            u32::try_from(i64::from(last) - i64::from(first) + 1).map_err(io::Error::other)?,
        )?;
        hours.extend(page.rows.into_iter().map(|row| row.hour));
        if let Some(country) = self.metadata.countries.get(tag) {
            hours.extend(
                country
                    .tail
                    .iter()
                    .filter(|read| read.last_hour >= first && read.first_hour <= last)
                    .map(|read| read.first_hour),
            );
        }
        hours
            .into_iter()
            .map(|hour| {
                dashboard_text::metadata::check_cancel(cancelled)?;
                self.fuel(tag, hour)?
                    .ok_or_else(|| dashboard_core::invalid("Prepared fuel read is missing"))
            })
            .collect()
    }
    fn consumption_window(
        &mut self,
        tag: &str,
        first: i32,
        last: i32,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<demand::FuelConsumptionRead>> {
        let Some(&key) = self.keys.get(&series::field_key(tag, "consumptionRead")) else {
            return Ok(Vec::new());
        };
        let mut hours = BTreeSet::new();
        if let Some(run) = self.current.value_at(key, first)? {
            hours.insert(i32::try_from(run.value).map_err(io::Error::other)?);
        }
        let page = self.current.events(
            &[key],
            first,
            last,
            0,
            u32::try_from(i64::from(last) - i64::from(first) + 1).map_err(io::Error::other)?,
        )?;
        hours.extend(page.rows.into_iter().map(|row| row.hour));
        hours
            .into_iter()
            .map(|hour| {
                dashboard_text::metadata::check_cancel(cancelled)?;
                self.consumption(tag, hour)?
                    .ok_or_else(|| dashboard_core::invalid("Prepared consumption read is missing"))
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesPoint {
    pub tag: String,
    pub hour: i32,
    pub approximate: bool,
    pub recorded: bool,
    pub fuel: Option<FuelRead>,
    pub consumption: Option<demand::FuelConsumptionRead>,
    pub consumption_segment: Option<dashboard_base::products::country_groups::HourSpan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub fuel_raw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub capacity_raw: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesStateReply {
    pub branch_id: String,
    pub states: Vec<FuelSeriesPoint>,
    pub replies: Vec<FuelReply>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesLine {
    pub tag: String,
    pub measure: String,
    pub summary: ChartSummary,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelSeriesSummaryReply {
    pub branch_id: String,
    pub series: Vec<FuelSeriesLine>,
    pub replies: Vec<FuelReply>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Params {
    branch_id: String,
    family: Family,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    reads: Vec<dashboard_base::prepared_measure::query::MeasureRead>,
    #[serde(default)]
    measures: Vec<String>,
    #[serde(default)]
    within: BTreeMap<String, Vec<dashboard_base::products::country_groups::HourSpan>>,
    from: Option<f64>,
    to: Option<f64>,
    width: Option<u32>,
    key: Option<series::MeasureKey>,
    first_hour: Option<f64>,
    last_hour: Option<f64>,
    cursor: Option<String>,
    page_size: Option<u32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    kind: String,
    measure: Option<String>,
}

fn presentation(
    source: &Source,
    context: &LoadContext,
    branch: u64,
    tag: &str,
    metadata: &Metadata,
) -> io::Result<FuelReply> {
    let country = metadata.countries.get(tag);
    let mut scanned = ScannedFuel::default();
    if let Some(country) = country {
        scanned.reads = country
            .tail
            .iter()
            .filter(|read| {
                metadata
                    .settled_until
                    .is_none_or(|hour| read.first_hour > hour)
            })
            .cloned()
            .map(raw_read)
            .collect::<io::Result<_>>()?;
        scanned.omitted_left_out = country.left_out;
        scanned.has_omitted_reads = country.has_reads;
    }
    let branch_scan = ScannedBranch {
        unnamed: metadata.unnamed.clone(),
        ..Default::default()
    };
    let mut reply = super::reply(
        source,
        &CountryParams {
            branch_id: branch.to_string(),
            tag: tag.into(),
        },
        context,
        &branch_scan,
        &scanned,
    )?;
    reply.reads.clear();
    Ok(reply)
}

pub fn query(
    source: &Source,
    content: Option<&dashboard_core::config::ContentOptions>,
    product: &str,
    params: Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<Value> {
    let params: Params = serde_json::from_value(params)?;
    if params.family.kind != "country-fuel" {
        return Err(dashboard_core::invalid("Unknown fuel series family"));
    }
    let branch = params.branch_id.parse::<u64>().map_err(io::Error::other)?;
    // The family's own coverage while it is prepared on its own.
    let accepted = match dashboard_base::prepared::coverage::product_coverage(
        source,
        content,
        branch,
        "country-fuel",
        cancelled,
    ) {
        Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
        result => result?,
    };
    let partial = accepted.as_ref().is_none_or(|coverage| !coverage.complete);
    let last = accepted.as_ref().map(|coverage| coverage.last_hour);
    let measures = if params.measures.is_empty() {
        vec![
            params
                .family
                .measure
                .clone()
                .unwrap_or_else(|| "fuel".into()),
        ]
    } else {
        params.measures.clone()
    };
    if partial
        && (matches!(
            product,
            "events-page" | "events-clusters" | "series-next-change"
        ) || product == "series-summary"
            && !measures
                .iter()
                .any(|field| matches!(field.as_str(), "fuel" | "capacity")))
    {
        return Ok(Value::Null);
    }
    let mut reads = if accepted.is_some() {
        Reads::open(store, source_key, branch, cancelled)?
    } else {
        None
    }
    .unwrap_or_else(|| Reads {
        keys: BTreeMap::new(),
        metadata: Metadata::default(),
        current: Reader::from_first_file(
            store,
            source_key,
            format!("{CURRENT}-{branch}"),
            0,
            None,
            cancelled,
        ),
        days: Reader::from_first_file(
            store,
            source_key,
            format!("{DAYS}-{branch}"),
            0,
            None,
            cancelled,
        ),
    });
    let context = LoadContext {
        content: content.cloned(),
    };
    let tags: BTreeSet<_> = params
        .tags
        .iter()
        .cloned()
        .chain(params.reads.iter().map(|read| read.tag.clone()))
        .collect();
    let requested: Vec<_> = tags
        .iter()
        .flat_map(|tag| ["fuel", "capacity"].map(move |field| (tag.clone(), field.to_owned())))
        .collect();
    let tail = if partial && product == "series-state" {
        progressive::state_tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
            &params
                .reads
                .iter()
                .map(|read| read.hour)
                .collect::<Vec<_>>(),
            &sample,
            cancelled,
        )?
    } else if partial && product == "series-summary" {
        let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
        series::summary_budget(from, to, width, requested.len())?;
        progressive::tail(
            source,
            branch,
            accepted.as_ref(),
            TABLES,
            &requested,
            (from.floor() as i32, to.floor() as i32),
            &sample,
            cancelled,
        )?
    } else {
        Default::default()
    };
    let replies = tags
        .iter()
        .map(|tag| {
            dashboard_text::metadata::check_cancel(cancelled)?;
            dashboard_core::protocol::validate_tag(tag).map_err(dashboard_core::invalid)?;
            presentation(source, &context, branch, tag, &reads.metadata)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let mut reply = match product {
        "series-state" => serde_json::to_value(FuelSeriesStateReply {
            branch_id: params.branch_id,
            states: params
                .reads
                .into_iter()
                .map(|read| {
                    let prepared = !partial || last.is_some_and(|last| read.hour <= last);
                    let consumption = if prepared {
                        reads.consumption(&read.tag, read.hour)?
                    } else {
                        None
                    };
                    let consumption_segment = consumption
                        .as_ref()
                        .map(|value| reads.actor_segment(value, read.hour))
                        .transpose()?
                        .flatten();
                    Ok(FuelSeriesPoint {
                        approximate: tail.1.iter().any(|span| {
                            span.first_hour <= read.hour && read.hour <= span.last_hour
                        }),
                        recorded: tail.0.get(&(read.tag.clone(), "fuel".into())).is_some_and(
                            |runs| {
                                runs.iter().any(|run| {
                                    run.first_hour <= read.hour && read.hour <= run.last_hour
                                })
                            },
                        ) || reads
                            .metadata
                            .countries
                            .get(&read.tag)
                            .is_some_and(|country| country.has_reads),
                        fuel: if prepared {
                            reads.fuel(&read.tag, read.hour)?
                        } else {
                            None
                        },
                        fuel_raw: tail
                            .0
                            .get(&(read.tag.clone(), "fuel".into()))
                            .and_then(|runs| {
                                runs.iter().find(|run| {
                                    run.first_hour <= read.hour && read.hour <= run.last_hour
                                })
                            })
                            .map(|run| run.value.to_string()),
                        capacity_raw: tail
                            .0
                            .get(&(read.tag.clone(), "capacity".into()))
                            .and_then(|runs| {
                                runs.iter().find(|run| {
                                    run.first_hour <= read.hour && read.hour <= run.last_hour
                                })
                            })
                            .map(|run| run.value.to_string()),
                        consumption,
                        consumption_segment,
                        tag: read.tag,
                        hour: read.hour,
                    })
                })
                .collect::<io::Result<_>>()?,
            replies,
        })?,
        "series-summary" => {
            let (from, to, width) = series::viewport(params.from, params.to, params.width)?;
            series::summary_budget(
                from,
                to,
                width,
                params
                    .tags
                    .len()
                    .saturating_mul(params.measures.len().max(1)),
            )?;
            let mut lines = Vec::new();
            for measure in &measures {
                if !matches!(
                    measure.as_str(),
                    "fuel"
                        | "capacity"
                        | "stripGain"
                        | "stripBurn"
                        | "gain-0"
                        | "gain-1"
                        | "gain-2"
                        | "gain-3"
                        | "unidentified"
                        | "incoming"
                        | "outgoing"
                        | "flat"
                        | "other"
                        | "conversion"
                        | "consumer-0"
                        | "consumer-1"
                        | "consumer-2"
                        | "consumption-0"
                        | "consumption-1"
                        | "consumption-2"
                ) {
                    return Err(dashboard_core::invalid(format!(
                        "Unknown fuel series measure: {measure}"
                    )));
                }
            }
            for tag in params.tags {
                for measure in &measures {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let derived_tail = if day_measure(measure) {
                        tail_runs(measure, &tag, &reads.metadata)?
                    } else {
                        tail.0
                            .get(&(tag.clone(), measure.clone()))
                            .cloned()
                            .unwrap_or_default()
                    };
                    let key = reads
                        .keys
                        .get(&series::field_key(&tag, measure))
                        .copied()
                        .unwrap_or(u64::MAX);
                    let reader = if day_measure(measure) {
                        &mut reads.days
                    } else {
                        &mut reads.current
                    };
                    let bounds = if partial && matches!(measure.as_str(), "fuel" | "capacity") {
                        let start = source
                            .replay_ref()
                            .branches
                            .iter()
                            .find(|start| start.id == branch)
                            .ok_or_else(|| dashboard_core::invalid("Unknown fuel branch"))?;
                        dashboard_text::metadata::branch_bounds(source.replay_ref(), start)
                    } else {
                        (
                            reads.metadata.first.unwrap_or(0),
                            reads.metadata.last.unwrap_or(0),
                        )
                    };
                    let mut summary = reader.summary_with_tail(
                        key,
                        bounds,
                        (from, to, width),
                        0,
                        &derived_tail,
                        params.within.get(&tag).map(Vec::as_slice),
                    )?;
                    if matches!(measure.as_str(), "fuel" | "capacity") {
                        progressive::clip_approximate(
                            &mut summary,
                            &tail.1,
                            params.within.get(&tag).map(Vec::as_slice),
                        );
                    }
                    lines.push(FuelSeriesLine {
                        tag: tag.clone(),
                        measure: measure.clone(),
                        summary,
                    });
                }
            }
            lines.sort_by_key(|line| {
                reads
                    .keys
                    .get(&series::field_key(&line.tag, &line.measure))
                    .copied()
                    .unwrap_or(u64::MAX)
            });
            serde_json::to_value(FuelSeriesSummaryReply {
                branch_id: params.branch_id,
                series: lines,
                replies,
            })?
        }
        "series-next-change" => {
            let navigation = series::Navigation {
                from: params.from,
                first_hour: params
                    .first_hour
                    .ok_or_else(|| dashboard_core::invalid("Missing fuel navigation range"))?,
                last_hour: params
                    .last_hour
                    .ok_or_else(|| dashboard_core::invalid("Missing fuel navigation range"))?,
                key: params
                    .key
                    .ok_or_else(|| dashboard_core::invalid("Missing fuel navigation direction"))?,
            };
            let keys = params
                .tags
                .iter()
                .filter_map(|tag| {
                    reads
                        .keys
                        .get(&series::field_key(tag, "read"))
                        .copied()
                        .map(|key| (key, params.within.get(tag).map(Vec::as_slice)))
                })
                .collect::<Vec<_>>();
            serde_json::json!({"hour":series::navigate(&mut [(&mut reads.current, keys)], &navigation)?})
        }
        "events-page" => {
            let (first, last, cursor, page_size) = series::event_window(
                params.from,
                params.to,
                params.cursor.as_deref(),
                params.page_size,
            )?;
            let consumption = params.family.measure.as_deref() == Some("consumption");
            let keys: BTreeMap<_, _> = params
                .tags
                .iter()
                .filter_map(|tag| {
                    reads
                        .keys
                        .get(&series::field_key(
                            tag,
                            if consumption {
                                "consumptionRead"
                            } else {
                                "read"
                            },
                        ))
                        .map(|key| (*key, tag.clone()))
                })
                .collect();
            // Tail rows are a bounded continuation of the same event page.
            let page = (if consumption {
                &mut reads.current
            } else {
                &mut reads.days
            })
            .events(
                &keys.keys().copied().collect::<Vec<_>>(),
                first,
                last,
                cursor,
                page_size,
            )?;
            let mut rows: Vec<_> = page.rows.into_iter().map(|row| serde_json::json!({"tag":keys[&row.key],"hour":row.hour,"ordinal":row.ordinal.to_string(),"row":row.row})).collect();
            let mut total = page.total;
            if !consumption {
                let mut tail = Vec::new();
                for tag in &params.tags {
                    if let Some(country) = reads.metadata.countries.get(tag) {
                        tail.extend(
                            country
                                .tail
                                .iter()
                                .filter(|read| {
                                    read.first_hour >= first
                                        && read.first_hour <= last
                                        && reads
                                            .metadata
                                            .settled_until
                                            .is_none_or(|hour| read.first_hour > hour)
                                })
                                .map(|read| (read.first_hour, tag.clone(), read.clone())),
                        );
                    }
                }
                tail.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
                total += tail.len() as u64;
                let tail_cursor = cursor.saturating_sub(page.total);
                for (index, (hour, tag, mut read)) in tail
                    .into_iter()
                    .enumerate()
                    .skip(tail_cursor as usize)
                    .take((page_size as usize).saturating_sub(rows.len()))
                {
                    if let Some(current) = reads.fuel(&tag, hour)? {
                        read.last_hour = current.last_hour;
                    }
                    rows.push(serde_json::json!({"tag":tag,"hour":hour,"ordinal":(page.total+index as u64).to_string(),"row":read}));
                }
            }
            let end = cursor.saturating_add(rows.len() as u64);
            serde_json::json!({"firstOrdinal":cursor.to_string(),"total":total.to_string(),"rows":rows,"cursor":(end<total).then(||end.to_string()),"warnings":[]})
        }
        _ => return Err(dashboard_core::invalid("Unknown fuel series query")),
    };
    if partial && product == "series-summary" {
        reply["pendingMeasures"] = serde_json::json!(
            measures
                .iter()
                .filter(|field| !matches!(field.as_str(), "fuel" | "capacity"))
                .collect::<Vec<_>>()
        );
    }
    reply["coverage"] = serde_json::json!(accepted);
    dashboard_base::products::check_reply_budget(&reply)?;
    Ok(reply)
}

// Project the bounded derived continuation; shared series owns all chart geometry.
fn tail_runs(measure: &str, tag: &str, metadata: &Metadata) -> io::Result<Vec<Run>> {
    let mut runs = Vec::new();
    if let Some(country) = metadata.countries.get(tag) {
        for read in &country.tail {
            let raw = raw_read(read.clone())?;
            if let Some(value) = day_values(&raw)
                .into_iter()
                .find_map(|(name, value)| (name == measure).then_some(value))
            {
                // A settled open read supplies its original bounds as well as the
                // unsettled carried hours, keeping exact runs after a narrow seek.
                if measure.starts_with("strip")
                    && metadata
                        .settled_until
                        .is_some_and(|hour| read.first_hour <= hour)
                {
                    continue;
                }
                runs.push(Run::new(
                    read.first_hour,
                    if measure.starts_with("strip") {
                        read.first_hour
                    } else {
                        read.last_hour
                    },
                    value,
                    read.starts_segment,
                ));
            }
        }
    }
    Ok(runs)
}

/// In-memory fixtures retain the registered whole-branch scan as the oracle.
#[cfg(any(test, feature = "test-support"))]
fn reference_fixture(source: &Source) -> bool {
    source.path().is_relative()
}

pub(super) fn summary_at(
    source: &Source,
    basis: &FuelAwardsParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<demand::FuelConsumptionRead> {
    let branch = basis.branch_id.parse::<u64>().map_err(io::Error::other)?;
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        return Ok(demand::FuelConsumptionRead::from(demand::summary_at(
            &*scanned(source, branch, cancelled)?,
            basis,
        )?));
    }
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let mut reads = Reads::open(&store, &source_key, branch, cancelled)?
        .ok_or_else(|| dashboard_core::invalid("Fuel summary preparation is not available"))?;
    reads
        .consumption(&basis.tag, basis.hour)?
        .filter(|read| {
            read.first_hour == basis.hour
                && read.country == basis.country
                && read.country_instance == Some(basis.country_instance)
        })
        .ok_or_else(|| {
            dashboard_core::invalid(
                "Current fuel consumption actor or supplying read was not recorded",
            )
        })
}

/// Reconstruct just the shared summary's supplying reads intersecting one window.
pub(super) fn window_summary(
    source: &Source,
    branch: u64,
    window: i32,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedBranch>> {
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        return scanned(source, branch, cancelled);
    }
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let mut reads = Reads::open(&store, &source_key, branch, cancelled)?
        .ok_or_else(|| dashboard_core::invalid("Fuel summary preparation is not available"))?;
    let (first, last) = dashboard_base::products::production_lines::detail_window(window)
        .ok_or_else(|| dashboard_core::invalid("Invalid fuel detail window"))?;
    let mut summary = ScannedBranch {
        unnamed: reads.metadata.unnamed.clone(),
        ..Default::default()
    };
    for (actor, metadata) in &reads.metadata.actors {
        let actor: (String, u32) = serde_json::from_str(actor)?;
        summary
            .actor_aliases
            .insert(actor.clone(), metadata.tags.clone());
        if metadata.conflict {
            summary.actor_conflicts.insert(actor);
        }
    }
    for tag in reads.metadata.countries.keys().cloned().collect::<Vec<_>>() {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let country = ScannedFuel {
            reads: reads
                .fuel_window(&tag, first, last, cancelled)?
                .into_iter()
                .map(raw_read)
                .collect::<io::Result<_>>()?,
            ..Default::default()
        };
        summary.countries.insert(tag, Arc::new(country));
    }
    Ok(Arc::new(summary))
}

/// The actor segment is already known; reading its window never searches history.
pub(super) fn actor_consumption(
    source: &Source,
    basis: &FuelAwardsParams,
    window: i32,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<Vec<(String, demand::FuelConsumptionRead)>> {
    let branch = basis.branch_id.parse::<u64>().map_err(io::Error::other)?;
    #[cfg(any(test, feature = "test-support"))]
    if reference_fixture(source) {
        let summary = scanned(source, branch, cancelled)?;
        return Ok(demand::actor_consumption(&summary, basis)
            .into_iter()
            .map(|(tag, read)| (tag.to_owned(), demand::FuelConsumptionRead::from(read)))
            .collect());
    }
    let (store, source_key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let mut reads = Reads::open(&store, &source_key, branch, cancelled)?
        .ok_or_else(|| dashboard_core::invalid("Fuel summary preparation is not available"))?;
    let actor = actor_binding(&basis.country, basis.country_instance);
    let Some(metadata) = reads.metadata.actors.get(&actor).cloned() else {
        return Ok(Vec::new());
    };
    if metadata.conflict {
        return Ok(Vec::new());
    }
    let Some(&segment_key) = reads.keys.get(&series::field_key(&actor, "actorSegment")) else {
        return Ok(Vec::new());
    };
    let Some(segment) = reads.current.value_at(segment_key, basis.hour)? else {
        return Ok(Vec::new());
    };
    let (first, last) = dashboard_base::products::production_lines::detail_window(window)
        .ok_or_else(|| dashboard_core::invalid("Invalid current fuel consumption window"))?;
    let mut result = Vec::new();
    for tag in metadata.tags {
        for read in reads.consumption_window(&tag, first, last, cancelled)? {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if read.country == basis.country
                && read.country_instance == Some(basis.country_instance)
                && reads
                    .current
                    .value_at(segment_key, read.first_hour)?
                    .is_some_and(|candidate| candidate.value == segment.value)
            {
                result.push((tag.clone(), read));
            }
        }
    }
    result.sort_by_key(|(_, read)| read.first_hour);
    Ok(result)
}

#[cfg(test)]
mod tests;
