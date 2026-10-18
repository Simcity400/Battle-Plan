//! One all-country research observation, with completed rows in shared events.
use super::{Country, Product, Scan};
use crate::prepared::series::{self, Accumulator, Reader};
use crate::{
    ResearchCoverage, ResearchEntry, ResearchHistory, ResearchProgress, ResearchSlotHistory,
    ResearchSlotState,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    values: Accumulator,
    keys: BTreeMap<String, u64>,
    slots: BTreeMap<String, BTreeMap<u32, SlotKeys>>,
    next_key: u64,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
struct SlotKeys {
    available: u64,
    state: u64,
    progress: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
enum Event {
    Coverage {
        span: ResearchCoverage,
    },
    Available {
        slot: u32,
        span: ResearchCoverage,
    },
    State {
        slot: u32,
        state: ResearchSlotState,
    },
    Progress {
        slot: u32,
        progress: ResearchProgress,
    },
    Entry {
        slot: u32,
        entry: ResearchEntry,
    },
    Read {
        progress: ResearchProgress,
    },
}
#[derive(Serialize, Deserialize)]
struct Open {
    history: ResearchHistory,
    unjoined: usize,
}
#[derive(Serialize, Deserialize)]
struct Catalogue {
    slots: BTreeMap<String, BTreeMap<u32, SlotKeys>>,
}

fn section(branch: u64) -> String {
    format!("research-{branch}")
}
fn open_section(branch: u64) -> String {
    format!("research-{branch}-open")
}

impl Prepared {
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &crate::prepared::frame_changes::FrameChanges {
                rows_changed: [true; replay_data::rows::TABLE_COUNT],
                table_read: [true; replay_data::rows::TABLE_COUNT],
                slot_read: [true; replay_data::rows::TABLE_COUNT],
                updates: None,
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        self.observe_with(frame.hour, |scan| {
            scan.observe_selected_changed(frame, None, changes, true)
        })
    }

    /// Observe hour `hour`: `step` takes the scan through the hour, then
    /// every country it observed publishes what the hour changed.
    fn observe_with(
        &mut self,
        hour: i32,
        step: impl FnOnce(&mut Scan) -> io::Result<()>,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("research:observe");
        self.values.advance(hour)?;
        step(&mut self.scan)?;
        for tag in &self.scan.observed {
            let country = self
                .scan
                .countries
                .get_mut(tag)
                .expect("observed research country");
            country.materialize(hour);
            let key = *self.keys.entry(tag.clone()).or_insert_with(|| {
                let key = self.next_key;
                self.next_key += 1;
                key
            });
            self.values.observe_if_changed(
                key,
                hour,
                country
                    .history
                    .coverage
                    .last()
                    .filter(|span| span.last_hour == hour)
                    .map(|_| 1),
                country
                    .history
                    .coverage
                    .last()
                    .is_some_and(|span| span.first_hour == hour),
                false,
            );
            let slots = self.slots.entry(tag.clone()).or_default();
            // Whether progress is shown at this read hour alone: the carried
            // read would hold it, so the next hour observes the country again.
            let mut read_only = false;
            for slot in &country.history.slots {
                let keys = *slots.entry(slot.index).or_insert_with(|| {
                    let keys = SlotKeys {
                        available: self.next_key,
                        state: self.next_key + 1,
                        progress: self.next_key + 2,
                    };
                    self.next_key += 3;
                    keys
                });
                let available = slot
                    .availability
                    .last()
                    .filter(|span| span.last_hour == hour);
                self.values.observe_if_changed(
                    keys.available,
                    hour,
                    available.map(|_| 1),
                    available.is_some_and(|span| span.first_hour == hour),
                    false,
                );
                let state = slot.states.last().filter(|state| state.last_hour == hour);
                self.values.observe_if_changed(
                    keys.state,
                    hour,
                    state.map(|state| i128::from(state.first_hour)),
                    state.is_some_and(|state| state.first_hour == hour),
                    false,
                );
                if let Some(state) = state.filter(|state| state.first_hour == hour) {
                    self.values.event(
                        keys.state,
                        hour,
                        &Event::State {
                            slot: slot.index,
                            state: state.clone(),
                        },
                    )?;
                }
                let current = |progress: &ResearchProgress| {
                    state.is_some_and(|state| {
                        state.token.is_some()
                            && state.entry_first_observed_hour == progress.entry_first_observed_hour
                    })
                };
                let progress = slot
                    .progress
                    .last()
                    .filter(|progress| progress.last_hour == hour || current(*progress));
                read_only |= progress.is_some_and(|progress| !current(progress));
                let value = progress
                    .map(|progress| progress.progress_raw.parse::<i128>())
                    .transpose()
                    .map_err(|_| crate::invalid("Invalid research progress"))?;
                self.values.observe_if_changed(
                    keys.progress,
                    hour,
                    value,
                    progress.is_some_and(|progress| progress.first_hour == hour),
                    false,
                );
                if let Some(progress) = progress.filter(|progress| {
                    progress.last_hour == hour && country.last_read == Some(hour)
                }) {
                    let mut progress = progress.clone();
                    progress.progress_raw.clear();
                    self.values
                        .event(keys.progress, hour, &Event::Read { progress })?;
                }
            }
            if read_only && let Some((_, canonical)) = country.identity {
                self.scan.recheck.insert(canonical);
            }
            retain_open(country, |event| self.values.event(key, hour, &event))?;
            country.charge(&mut self.scan.bytes);
        }
        self.values.through(hour);
        Ok(())
    }

    pub fn buffered_bytes(&self) -> usize {
        self.values.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.seal();
        self.values.flush(writer, &section(branch), 5, cancelled)?;
        series::write_catalogue(
            writer,
            "research",
            branch,
            &self.keys,
            &Catalogue {
                slots: self.slots.clone(),
            },
        )?;
        // Country keys follow first sightings; slot measures share their ID
        // space. Rank the country keys to get stable, contiguous reply ordinals.
        let mut countries: Vec<_> = self.keys.iter().collect();
        countries.sort_unstable_by_key(|(_, key)| **key);
        let open = open_section(branch);
        for (ordinal, (tag, _)) in countries.into_iter().enumerate() {
            crate::metadata::check_cancel(cancelled)?;
            let ordinal = ordinal as u64;
            let reply = match self.scan.countries.get(tag) {
                Some(country) => Open {
                    history: country.history.clone(),
                    unjoined: country.unjoined_reads,
                },
                None => {
                    if writer.read_at(&open, ordinal)?.is_some() {
                        continue;
                    }
                    Open {
                        history: ResearchHistory::default(),
                        unjoined: 0,
                    }
                }
            };
            series::write_reply(writer, &open, ordinal, &reply)?;
        }
        Ok(())
    }
}

/// The existing observer needs its latest segment, last assignment and any
/// assignment still awaiting a same-day completion read. Everything else is
/// immutable and leaves the checkpoint.
fn retain_open(
    country: &mut Country,
    mut event: impl FnMut(Event) -> io::Result<()>,
) -> io::Result<()> {
    let keep_last = |length: usize| length.saturating_sub(1);
    let remove = keep_last(country.history.coverage.len());
    for span in country.history.coverage.drain(..remove) {
        event(Event::Coverage { span })?;
    }
    for slot in &mut country.history.slots {
        let mut kept = Vec::new();
        let last = keep_last(slot.entries.len());
        let mut positions = BTreeMap::new();
        for (index, value) in std::mem::take(&mut slot.entries).into_iter().enumerate() {
            if index == last
                || country
                    .pending
                    .iter()
                    .any(|pending| pending.0 == slot.index && pending.1 == index)
            {
                positions.insert(index, kept.len());
                kept.push(value);
            } else {
                event(Event::Entry {
                    slot: slot.index,
                    entry: value,
                })?;
            }
        }
        for pending in &mut country.pending {
            if pending.0 == slot.index {
                pending.1 = positions[&pending.1];
            }
        }
        slot.entries = kept;
        let remove = keep_last(slot.availability.len());
        for span in slot.availability.drain(..remove) {
            event(Event::Available {
                slot: slot.index,
                span,
            })?;
        }
        let remove = keep_last(slot.states.len());
        slot.states.drain(..remove);
        let remove = keep_last(slot.progress.len());
        for mut progress in slot.progress.drain(..remove) {
            progress.progress_raw.clear();
            event(Event::Progress {
                slot: slot.index,
                progress,
            })?;
        }
    }
    country.retained_bytes = super::history_bytes(&country.history);
    Ok(())
}

fn slot(history: &mut ResearchHistory, index: u32) -> &mut ResearchSlotHistory {
    let position = history
        .slots
        .iter()
        .position(|slot| slot.index == index)
        .unwrap_or_else(|| {
            history.slots.push(ResearchSlotHistory {
                index,
                availability: Vec::new(),
                entries: Vec::new(),
                states: Vec::new(),
                progress: Vec::new(),
            });
            history.slots.len() - 1
        });
    &mut history.slots[position]
}

pub fn history(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<Option<(ResearchHistory, usize)>> {
    let Some((keys, catalogue)) =
        series::read_catalogue::<Catalogue>(store, source_key, "research", branch)?
    else {
        return Ok(None);
    };
    let Some(&key) = keys.get(tag) else {
        return Err(io::Error::other(
            "No recorded research history for this country",
        ));
    };
    let ordinal = keys.values().filter(|candidate| **candidate < key).count() as u64;
    let Some(open) = series::read_reply::<Open>(store, source_key, &open_section(branch), ordinal)?
    else {
        return Ok(None);
    };
    let mut reader = Reader::new(store, source_key, section(branch), cancelled)?;
    let mut history = ResearchHistory::default();
    let mut event_keys = vec![key];
    event_keys.extend(catalogue.slots[tag].values().map(|keys| keys.state));
    let mut cursor = 0;
    loop {
        crate::metadata::check_cancel(cancelled)?;
        let page = reader.events(&event_keys, i32::MIN, i32::MAX, cursor, 256)?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(crate::invalid("Research event page made no progress"));
        }
        for event in page.rows {
            match serde_json::from_value::<Event>(event.row)? {
                Event::Coverage { span } => history.coverage.push(span),
                Event::Available { slot: index, span } => {
                    slot(&mut history, index).availability.push(span)
                }
                Event::State {
                    slot: index,
                    mut state,
                } => {
                    state.last_hour = reader
                        .value_at(catalogue.slots[tag][&index].state, state.first_hour)?
                        .ok_or_else(|| crate::invalid("Prepared research state is missing"))?
                        .last_hour;
                    slot(&mut history, index).states.push(state)
                }
                Event::Progress {
                    slot: index,
                    mut progress,
                } => {
                    let run = reader
                        .value_at(catalogue.slots[tag][&index].progress, progress.first_hour)?
                        .ok_or_else(|| crate::invalid("Prepared research progress is missing"))?;
                    progress.progress_raw = run.value.to_string();
                    slot(&mut history, index).progress.push(progress);
                }
                Event::Entry { slot: index, entry } => {
                    slot(&mut history, index).entries.push(entry)
                }
                Event::Read { .. } => return Err(crate::invalid("Unexpected research read row")),
            }
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
    }
    history.coverage.extend(open.history.coverage);
    for latest in open.history.slots {
        let slot = slot(&mut history, latest.index);
        slot.entries.extend(latest.entries);
        slot.availability.extend(latest.availability);

        slot.progress.extend(latest.progress);
    }
    history.coverage.sort_by_key(|span| span.first_hour);
    history.slots.sort_by_key(|slot| slot.index);
    for slot in &mut history.slots {
        slot.entries.sort_by_key(|entry| entry.first_observed_hour);
        slot.availability.sort_by_key(|span| span.first_hour);
        slot.states.sort_by_key(|state| state.first_hour);
        slot.progress.sort_by_key(|progress| progress.first_hour);
    }
    Ok(Some((history, open.unjoined)))
}

pub fn query(
    source: &crate::Source,
    context: &super::LoadContext,
    product: &str,
    params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    if product != "events-page" {
        return values_query(product, params, store, source_key, cancelled);
    }
    // Research's complete selected-country rows retain their established UI
    // sorting and time-breakdown contract. History queries read prepared pages.
    let params: super::CountryParams = serde_json::from_value(
        serde_json::json!({ "branchId": params.get("branchId"), "tag": params.get("tag") }),
    )?;
    super::Research::validate(&params).map_err(crate::invalid)?;
    let (history, unjoined) =
        history(store, source_key, params.branch(), &params.tag, cancelled)?.unwrap_or_default();
    let state = source.state::<super::State>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    serde_json::to_value(super::present(
        source, context, &mut state, history, unjoined, cancelled,
    )?)
    .map_err(Into::into)
}

#[derive(Clone, Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStateSlot {
    pub index: u32,
    pub available: bool,
    pub state: Option<ResearchSlotState>,
    pub progress: Option<ResearchProgress>,
    pub read_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchState {
    pub tag: String,
    pub hour: i32,
    pub slots: Vec<ResearchStateSlot>,
    pub data: super::ResearchReply,
    pub preparing: Vec<String>,
}

#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStateReply {
    pub branch_id: String,
    pub states: Vec<ResearchState>,
}

/// Slot state at the requested hours. A cold read observes only two read
/// periods: the supplying read, its preceding daily progress, and the carry
/// proof through the target. Assignment attribution remains with the pages.
pub fn state_query(
    source: &crate::Source,
    context: &super::LoadContext,
    params: &serde_json::Value,
    store: &store::Store,
    key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    use crate::prepared::{frame_changes::FrameChangeTracker, prefix, query::progressive};
    let branch_id = params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| crate::invalid("Invalid research branch"))?;
    let branch = branch_id
        .parse()
        .map_err(|_| crate::invalid("Invalid research branch"))?;
    let reads: Vec<crate::prepared_measure::query::MeasureRead> = serde_json::from_value(
        params
            .get("reads")
            .cloned()
            .ok_or_else(|| crate::invalid("Research reads are missing"))?,
    )?;
    let mut states = Vec::new();
    let prefix = prefix::read_or_empty(
        source,
        context.content.as_ref(),
        "research",
        branch,
        reads.iter().map(|read| read.hour).max(),
        cancelled,
    )?;
    for read in reads {
        super::super::validate_tag(&read.tag).map_err(crate::invalid)?;
        crate::metadata::check_cancel(cancelled)?;
        let prepared = prefix.coverage.complete || read.hour <= prefix.coverage.last_hour;
        let mut unavailable = ResearchHistory::default();
        let mut slots: Vec<ResearchStateSlot> = if prepared {
            let reply = values_query(
                "series-state",
                serde_json::json!({ "branchId": branch_id, "reads": [{ "tag": read.tag, "hour": read.hour }] }),
                store,
                key,
                cancelled,
            )?;
            serde_json::from_value(
                reply["states"][0]["slots"]
                    .as_array()
                    .cloned()
                    .map(serde_json::Value::Array)
                    .unwrap_or_else(|| serde_json::json!([])),
            )?
        } else {
            let mut scan = Scan::default();
            let mut changes = FrameChangeTracker::new();
            let frames = progressive::read_window(
                source,
                branch,
                read.hour
                    .saturating_sub(2 * replay_data::cadence::STAGGER_PERIOD - 1),
                read.hour,
                block::TABLES,
                cancelled,
            )?;
            let observed = frames.last().is_some_and(|frame| frame.hour == read.hour);
            for frame in &frames {
                let changed = changes.observe(frame, source.replay_ref().format);
                scan.observe_selected_changed(frame, Some(&read.tag), &changed, false)?;
            }
            let mut slots = Vec::new();
            if observed && let Some(country) = scan.countries.get_mut(&read.tag) {
                country.materialize(read.hour);
                if !country
                    .history
                    .coverage
                    .last()
                    .is_some_and(|span| span.last_hour == read.hour)
                {
                    unavailable = country.history.clone();
                    for slot in &mut unavailable.slots {
                        slot.entries.clear();
                        slot.states.clear();
                        slot.progress.clear();
                    }
                }
                for slot in &country.history.slots {
                    let available = slot
                        .availability
                        .last()
                        .is_some_and(|span| span.last_hour == read.hour);
                    let state = slot
                        .states
                        .last()
                        .filter(|state| state.last_hour == read.hour)
                        .cloned();
                    let progress = slot
                        .progress
                        .last()
                        .filter(|progress| Some(progress.last_hour) == country.last_read)
                        .cloned();
                    slots.push(ResearchStateSlot {
                        index: slot.index,
                        available,
                        state,
                        progress,
                        read_hour: country.last_read,
                    });
                }
            }
            slots
        };
        let mut preparing: Vec<String> = if prepared {
            Vec::new()
        } else {
            vec![
                "assignment".into(),
                "breakdown".into(),
                "stateHistory".into(),
            ]
        };
        let mut history = unavailable;
        for slot in slots.iter_mut().filter(|slot| slot.available) {
            let mut state = slot.state.clone();
            let mut progress = slot.progress.clone().filter(|progress| {
                state.as_ref().is_some_and(|state| {
                    state.token.is_some()
                        && state.entry_first_observed_hour == progress.entry_first_observed_hour
                })
            });
            let supplying = slot.read_hour.unwrap_or(read.hour);
            let from = state
                .as_ref()
                .map_or(supplying, |state| state.first_hour.min(supplying));
            if let Some(state) = &mut state {
                state.last_hour = read.hour;
                state.entry_first_observed_hour = None;
                // A countdown's compressed segment keeps its original phase.
                // A bounded scan proves its estimate at a supplying read, but
                // cannot recover an older segment's phase through a carry.
                if !prepared && supplying != read.hour && state.token.is_some() {
                    if let Some(details) = &mut state.details {
                        details.remaining_days = None;
                    }
                    if let Some(details) =
                        slot.state.as_mut().and_then(|state| state.details.as_mut())
                    {
                        details.remaining_days = None;
                    }
                    if !preparing.iter().any(|field| field == "remainingDays") {
                        preparing.push("remainingDays".into());
                    }
                }
            }
            if let Some(progress) = &mut progress {
                progress.first_hour = supplying;
                progress.last_hour = supplying;
                progress.entry_first_observed_hour = None;
            }
            history.slots.push(ResearchSlotHistory {
                index: slot.index,
                availability: vec![ResearchCoverage {
                    first_hour: from,
                    last_hour: read.hour,
                }],
                entries: Vec::new(),
                states: state.into_iter().collect(),
                progress: progress.into_iter().collect(),
            });
        }
        if history.coverage.is_empty()
            && let Some(first) = history
                .slots
                .iter()
                .flat_map(|slot| slot.availability.iter())
                .map(|span| span.first_hour)
                .min()
        {
            history.coverage.push(ResearchCoverage {
                first_hour: first,
                last_hour: read.hour,
            });
        }
        let state = source.state::<super::State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let data = super::present(source, context, &mut state, history, 0, cancelled)?;
        states.push(ResearchState {
            tag: read.tag,
            hour: read.hour,
            slots,
            data,
            preparing,
        });
    }
    let mut data = serde_json::to_value(ResearchStateReply {
        branch_id: branch_id.into(),
        states,
    })?;
    prefix.mark(&mut data)?;
    Ok(data)
}

fn values_query(
    product: &str,
    params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params
        .get("branchId")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| id.parse::<u64>().ok())
        .ok_or_else(|| crate::invalid("Invalid research branch"))?;
    let Some((keys, catalogue)) =
        series::read_catalogue::<Catalogue>(store, source_key, "research", branch)?
    else {
        return Ok(serde_json::Value::Null);
    };
    let mut reader = Reader::new(store, source_key, section(branch), cancelled)?;
    if product == "series-state" {
        let reads: Vec<crate::prepared_measure::query::MeasureRead> = serde_json::from_value(
            params
                .get("reads")
                .cloned()
                .ok_or_else(|| crate::invalid("Research reads are missing"))?,
        )?;
        let mut states = Vec::new();
        for read in reads {
            super::super::validate_tag(&read.tag).map_err(crate::invalid)?;
            let mut slots = Vec::new();
            for (&index, keys) in catalogue.slots.get(&read.tag).into_iter().flatten() {
                crate::metadata::check_cancel(cancelled)?;
                let mut state = None;
                if let Some(run) = reader.value_at(keys.state, read.hour)?
                    && let Some((_, value)) = reader.latest_event(keys.state, read.hour)?
                    && let Event::State {
                        state: mut value, ..
                    } = serde_json::from_value::<Event>(value)?
                {
                    value.last_hour = run.last_hour.min(read.hour);
                    state = Some(value);
                }
                let mut progress = None;
                let mut supplying_hour = None;
                if let Some((hour, value)) = reader.latest_event(keys.progress, read.hour)?
                    && let Event::Read {
                        progress: mut value,
                    } = serde_json::from_value::<Event>(value)?
                    && let Some(run) = reader.value_at(keys.progress, hour)?
                {
                    value.last_hour = value.last_hour.min(read.hour);
                    value.progress_raw = run.value.to_string();
                    progress = Some(value);
                    supplying_hour = Some(hour);
                }
                slots.push(serde_json::json!({ "index": index, "available": reader.value_at(keys.available, read.hour)?.is_some(), "state": state, "progress": progress, "readHour": supplying_hour }));
            }
            states.push(serde_json::json!({ "tag": read.tag, "hour": read.hour, "slots": slots }));
        }
        return Ok(serde_json::json!({ "branchId": branch.to_string(), "states": states }));
    }
    let tags: Vec<String> = match params.get("tags") {
        Some(tags) => serde_json::from_value(tags.clone())?,
        None => vec![
            params
                .get("tag")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| crate::invalid("Research tag is missing"))?
                .to_owned(),
        ],
    };
    for tag in &tags {
        super::super::validate_tag(tag).map_err(crate::invalid)?;
    }
    if product == "series-next-change" {
        let navigation: series::Navigation = serde_json::from_value(params)?;
        let mut ids = Vec::new();
        for tag in &tags {
            if let Some(&key) = keys.get(tag) {
                ids.push(key);
            }
            for keys in catalogue
                .slots
                .get(tag)
                .into_iter()
                .flat_map(|slots| slots.values())
            {
                ids.extend([keys.available, keys.state, keys.progress]);
            }
        }
        let hour = series::navigate(
            &mut [(&mut reader, ids.into_iter().map(|id| (id, None)).collect())],
            &navigation,
        )?;
        return Ok(serde_json::json!({ "hour": hour }));
    }
    let (from, to, width) = series::viewport(
        params.get("from").and_then(serde_json::Value::as_f64),
        params.get("to").and_then(serde_json::Value::as_f64),
        params
            .get("width")
            .and_then(serde_json::Value::as_u64)
            .and_then(|width| u32::try_from(width).ok()),
    )?;
    let mut lines = Vec::new();
    match product {
        "series-summary" => {
            let count = tags
                .iter()
                .filter_map(|tag| catalogue.slots.get(tag))
                .map(BTreeMap::len)
                .sum();
            series::summary_budget(from, to, width, count)?;
            for tag in &tags {
                for (&slot, keys) in catalogue.slots.get(tag).into_iter().flatten() {
                    lines.push(serde_json::json!({ "tag": tag, "slot": slot, "summary": reader.summary(keys.progress, (i32::MIN, i32::MAX), (from, to, width), 5)? }));
                }
            }
        }
        "events-clusters" => {
            for tag in &tags {
                if let Some(&key) = keys.get(tag) {
                    lines.push(serde_json::json!({ "tag": tag, "summary": reader.event_clusters(key, from, to, width)? }));
                }
            }
        }
        _ => return Err(crate::invalid("Unknown research series query")),
    }
    Ok(serde_json::json!({ "branchId": branch.to_string(), "series": lines }))
}

/// Research prepared block by block, on its own.
pub mod block;

#[cfg(test)]
mod tests;
