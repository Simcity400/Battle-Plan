//! The production daily country scans feed the shared series batches.
use crate::{
    prepared::{
        frame_changes::FrameChanges,
        series::{self, Accumulator},
    },
    prepared_measure::{MeasureActor, MeasureHolder},
    products::{country_experience, country_stability},
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
#[derive(Default, Serialize, Deserialize)]
pub struct StabilityAccumulator {
    stability: country_stability::Scan,
    parts: crate::products::stability_parts::Prepared,
    experience: country_experience::Scan,
    actors: BTreeMap<String, Actor>,
    last: Option<i32>,
    experience_last: Option<i32>,
    experience_actors: BTreeMap<String, Actor>,
    batch: Accumulator,
    experience_batch: Accumulator,
    #[cfg(test)]
    #[serde(skip)]
    test_changes: Option<crate::prepared::frame_changes::FrameChangeTracker>,
}
impl StabilityAccumulator {
    pub fn buffered_bytes(&self) -> usize {
        self.batch.bytes() + self.experience_batch.bytes() + self.parts.bytes()
    }
    #[cfg(test)]
    pub fn observe(&mut self, frame: &replay::frame::DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(crate::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, replay::format::FormatVersion::CURRENT);
        self.observe_timed(frame, &changes, &mut None)
    }
    pub(crate) fn observe_timed(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        changes: &FrameChanges,
        timings: &mut Option<BTreeMap<&'static str, crate::prepared::pipeline::FamilyTimings>>,
    ) -> io::Result<()> {
        let started = crate::prepared::pipeline::start_timing(timings);
        let result = self.parts.observe_changes(frame, changes);
        crate::prepared::pipeline::finish_timing(timings, started, "stability_parts", false);
        result?;
        let adjacent = self.last.and_then(|hour| hour.checked_add(1)) == Some(frame.hour);
        self.last = Some(frame.hour);
        self.batch.advance(frame.hour)?;
        let Self {
            stability,
            actors,
            batch,
            ..
        } = self;
        stability.step(
            frame,
            None,
            Some(changes),
            |tag, row, slot, read, values, joined, changed| {
                if changed[0] {
                    record(
                        actors,
                        batch,
                        tag,
                        "readHour",
                        frame.hour,
                        Some(i128::from(read)),
                        adjacent,
                        (slot, row.country, read),
                        false,
                        None,
                    )?;
                }
                for (((measure, value), joined), changed) in
                    ["stability", "warSupport", "maxCommandPower"]
                        .into_iter()
                        .zip(values)
                        .zip(joined)
                        .zip(changed[1..].iter().copied())
                {
                    if !changed {
                        continue;
                    }
                    record(
                        actors,
                        batch,
                        tag,
                        measure,
                        frame.hour,
                        value.map(i128::from),
                        joined,
                        (slot, row.country, read),
                        false,
                        None,
                    )?;
                }
                Ok(())
            },
        )?;
        for tag in &stability.removed {
            for measure in ["readHour", "stability", "warSupport", "maxCommandPower"] {
                if let Some(actor) = actors.get_mut(&series::field_key(tag, measure)) {
                    batch.observe(actor.key, frame.hour, None, false);
                    actor.holder = None;
                }
            }
        }
        Ok(())
    }
    #[cfg(test)]
    pub fn observe_experience(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
    ) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(crate::prepared::frame_changes::FrameChangeTracker::new)
            .observe(frame, format);
        self.observe_experience_changes(frame, format, &changes)
    }
    pub(crate) fn observe_experience_changes(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.experience_last = Some(frame.hour);
        self.experience_batch.advance(frame.hour)?;
        self.experience
            .step(frame, format, Some(changes), |tag, (slot, id), read| {
                let tag = serde_json::to_string(&(tag, slot, id))?;
                let holder = (slot, id, read.read_hour);
                let boundary = read.first_hour == frame.hour;
                let joined = !read.starts_segment || !boundary;
                record(
                    &mut self.experience_actors,
                    &mut self.experience_batch,
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
                            &mut self.experience_actors,
                            &mut self.experience_batch,
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
        for (tag, (slot, id)) in &self.experience.removed {
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
                if let Some(actor) = self
                    .experience_actors
                    .get_mut(&series::field_key(&tag, &measure))
                {
                    self.experience_batch
                        .observe(actor.key, frame.hour, None, false);
                    actor.holder = None;
                }
            }
        }
        Ok(())
    }
    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.flush_timed(writer, branch, cancelled, &mut None)
    }
    pub(crate) fn flush_timed(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
        timings: &mut Option<BTreeMap<&'static str, crate::prepared::pipeline::FamilyTimings>>,
    ) -> io::Result<()> {
        if let Some(last) = self.last {
            let started = crate::prepared::pipeline::start_timing(timings);
            let result = self.parts.flush(writer, branch, cancelled);
            crate::prepared::pipeline::finish_timing(timings, started, "stability_parts", true);
            result?;
            flush(
                &self.actors,
                &mut self.batch,
                writer,
                ("stability", branch, last, 5),
                cancelled,
            )?;
        }
        if let Some(last) = self.experience_last {
            flush(
                &self.experience_actors,
                &mut self.experience_batch,
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
    let Some(value) = value else {
        batch.observe(actor.key, hour, None, false);
        actor.holder = None;
        return Ok(());
    };
    let same_holder = joined
        && actor
            .holder
            .is_some_and(|seen| (seen.0, seen.1) == (holder.0, holder.1));
    batch.observe_boundary(actor.key, hour, Some(value), !same_holder, boundary);
    let follows = same_holder
        && batch
            .state(actor.key)
            .and_then(|state| state.last)
            .is_some_and(|run| run.first_hour != hour || !run.break_before);
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
mod tests;
