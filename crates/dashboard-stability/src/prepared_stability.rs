//! The country-experience family's series batches, from its daily country scans, and the
//! series keys and holder events stability's parts share with them.
use dashboard_base::{
    prepared::{
        frame_changes::FrameChanges,
        series::{self, Accumulator},
    },
    prepared_measure::{MeasureActor, MeasureHolder},
    products::country_experience,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[derive(Serialize, Deserialize)]
pub(crate) struct Actor {
    tag: String,
    measure: String,
    first: i32,
    pub(crate) key: u64,
    pub(crate) holder: Option<(u32, u32, i32, i32)>,
}

/// The country-experience scan and its shared series: the family
/// `experience`, prepared on its own from the hours its tables change or its
/// banked experience is read, apart from the hourly stability family.
#[derive(Default, Serialize, Deserialize)]
pub struct ExperienceAccumulator {
    scan: country_experience::Scan,
    actors: BTreeMap<String, Actor>,
    last: Option<i32>,
    batch: Accumulator,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub test_changes: Option<dashboard_base::prepared::frame_changes::FrameChangeTracker>,
    #[cfg(test)]
    #[serde(skip)]
    whole_experience: bool,
    #[cfg(test)]
    #[serde(skip)]
    projections: usize,
}
impl ExperienceAccumulator {
    pub fn buffered_bytes(&self) -> usize {
        self.batch.bytes()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        format: replay_data::format::FormatVersion,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(dashboard_base::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, format);
        self.observe_changes(frame, format, &changes)
    }
    pub fn observe_changes(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        format: replay_data::format::FormatVersion,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("experience:series_observe");
        self.last = Some(frame.hour);
        self.batch.advance(frame.hour)?;
        #[cfg(test)]
        let whole = self.whole_experience;
        #[cfg(not(test))]
        let whole = false;
        self.scan
            .step(frame, format, Some(changes), |tag, (slot, id), read| {
                // Extending this same held read changes no channel or holder
                // event. The accepted frontier carries its existing values.
                if !whole && read.first_hour != frame.hour {
                    return Ok(());
                }
                let _cpu = ::replay_frame::cpu::scope_phase("experience:projection_series");
                #[cfg(test)]
                {
                    self.projections += 1;
                }
                let tag = serde_json::to_string(&(tag, slot, id))?;
                let holder = (slot, id, read.read_hour);
                let boundary = read.first_hour == frame.hour;
                let joined = !read.starts_segment || !boundary;
                record(
                    &mut self.actors,
                    &mut self.batch,
                    &tag,
                    "experienceRead",
                    frame.hour,
                    Some(i128::from(read.read_hour)),
                    joined,
                    holder,
                    boundary,
                    Some(read),
                )?;
                for (measure, values) in ["banked", "gain", "change"].into_iter().zip(read.values())
                {
                    for (branch, value) in ["army", "navy", "air"].into_iter().zip(values) {
                        record(
                            &mut self.actors,
                            &mut self.batch,
                            &tag,
                            &format!("{branch}-{measure}"),
                            frame.hour,
                            value,
                            joined,
                            holder,
                            boundary,
                            None,
                        )?;
                    }
                }
                Ok(())
            })?;
        self.close_removed(frame.hour)
    }
    /// The scan's holders that left the output at `hour` hold no value from it.
    fn close_removed(&mut self, hour: i32) -> io::Result<()> {
        for (tag, (slot, id)) in &self.scan.removed {
            let tag = serde_json::to_string(&(tag, slot, id))?;
            for measure in std::iter::once("experienceRead".to_owned()).chain(
                ["banked", "gain", "change"]
                    .into_iter()
                    .flat_map(|measure| {
                        ["army", "navy", "air"]
                            .into_iter()
                            .map(move |branch| format!("{branch}-{measure}"))
                    }),
            ) {
                if let Some(actor) = self.actors.get_mut(&series::field_key(&tag, &measure)) {
                    self.batch.observe(actor.key, hour, None, false);
                    actor.holder = None;
                }
            }
        }
        Ok(())
    }
    /// Hours `first..=last` that follow the last observed, at which none of
    /// the scan's tables changed and banked experience was not read: what
    /// observing each does (country experience's quiet step), with the
    /// series advanced to each, without visiting a country's gain runs. False
    /// when `first` does not follow the last observed hour.
    pub fn advance_quiet(&mut self, first: i32, last: i32) -> io::Result<bool> {
        if self.last.and_then(|hour| hour.checked_add(1)) != Some(first) {
            return Ok(false);
        }
        self.batch.advance(first)?;
        if !self.scan.advance_quiet(first, last) {
            return Ok(false);
        }
        self.last = Some(first);
        self.close_removed(first)?;
        self.batch.advance_span(first, last)?;
        self.last = Some(last);
        Ok(true)
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        if let Some(last) = self.last {
            flush(
                &self.actors,
                &mut self.batch,
                writer,
                ("experience", branch, last, 0),
                cancelled,
            )?;
        }
        Ok(())
    }
}

pub(crate) fn flush(
    actors: &BTreeMap<String, Actor>,
    batch: &mut Accumulator,
    writer: &mut store::Writer,
    (section, branch, last, decimals): (&str, u64, i32, u8),
    cancelled: &AtomicBool,
) -> io::Result<()> {
    batch.flush(writer, &format!("{section}-{branch}"), decimals, cancelled)?;
    let catalogue: Vec<_> = actors
        .values()
        .map(|actor| {
            let state = batch.state(actor.key);
            MeasureActor {
                actor: actor.key.to_string(),
                tag: actor.tag.clone(),
                measure: actor.measure.clone(),
                first_hour: actor.first,
                last_hour: last,
                run_count: state.map_or(0, |state| state.runs).to_string(),
                drop_count: state.map_or(0, |state| state.events).to_string(),
            }
        })
        .collect();
    let keys = actors
        .iter()
        .map(|(name, actor)| (name.clone(), actor.key))
        .collect();
    series::write_catalogue(writer, section, branch, &keys, &catalogue)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record(
    actors: &mut BTreeMap<String, Actor>,
    batch: &mut Accumulator,
    tag: &str,
    measure: &str,
    hour: i32,
    value: Option<i128>,
    joined: bool,
    holder: (u32, u32, i32),
    boundary: bool,
    experience: Option<country_experience::RawRead>,
) -> io::Result<()> {
    let next = actors.len() as u64;
    let actor = actors
        .entry(series::field_key(tag, measure))
        .or_insert_with(|| Actor {
            tag: tag.into(),
            measure: measure.into(),
            first: hour,
            key: next,
            holder: None,
        });
    #[cfg(not(test))]
    let whole = false;
    #[cfg(test)]
    let whole = WHOLE_RECORDS.with(std::cell::Cell::get);
    let Some(value) = value else {
        if whole {
            batch.observe(actor.key, hour, None, false);
        } else {
            batch.observe_if_changed(actor.key, hour, None, false, false);
        }
        actor.holder = None;
        return Ok(());
    };
    let same_holder = joined
        && actor
            .holder
            .is_some_and(|seen| (seen.0, seen.1) == (holder.0, holder.1));
    let observed = if whole {
        batch.observe_boundary(actor.key, hour, Some(value), !same_holder, boundary);
        true
    } else {
        batch.observe_if_changed(actor.key, hour, Some(value), !same_holder, boundary)
    };
    // A carried value proves continuity. Ordinary observation materializes
    // this key through the hour before its state is inspected.
    let follows = same_holder
        && (!observed
            || batch
                .state(actor.key)
                .and_then(|state| state.last)
                .is_some_and(|run| run.first_hour != hour || !run.break_before));
    let first = if follows {
        actor.holder.unwrap().3
    } else {
        hour
    };
    let next_holder = (holder.0, holder.1, holder.2, first);
    if actor.holder != Some(next_holder) {
        batch.event(
            actor.key,
            hour,
            &MeasureHolder {
                slot: holder.0,
                country_id: holder.1,
                read_hour: Some(holder.2),
                segment_first_hour: Some(first),
                experience: experience
                    .as_ref()
                    .map(country_experience::ExperienceRead::from),
            },
        )?;
        actor.holder = Some(next_holder);
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    static WHOLE_RECORDS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
mod tests;
