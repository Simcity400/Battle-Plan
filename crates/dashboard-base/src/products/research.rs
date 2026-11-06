//! Slot assignments from observed research tables, with explicit coverage and bounds.
//!
//! Research is read one country slice per hour (see the collector README), so
//! a country's rows are observed at their read hour (SliceReads), usually a day apart.
//! Carried rows add no observation; they hold the last read's coverage, slots
//! and slot states until the next read is due, and progress keeps its read hour.
//! Consecutive reads with no unobservable hour
//! between them are continuous: a new assignment is bounded by the previous
//! read, unless an hour between them was unobservable or a scheduled read
//! of the country was missed (reads more than a day apart). Assignment,
//! cancellation, completion and slot changes are read within the hour;
//! modifier-only changes appear at the country's next slice read.
//!
//! A slot read whose technology occurrence was not recorded (recordings made
//! before it was) keeps its coverage and slot state, named by the read's own
//! technology key, but joins no assignment: equal handles and names can belong
//! to repeated occurrences. Such reads are counted in a named warning. Their
//! progress is still exact at each read and is kept without an assignment,
//! merged only while the slot holds that same technology without a break.
//!
//! The reply names each technology's research folder (a tab of the technology
//! screen) through the recorded content stack: the first folder the technology
//! is placed in whose tab is available, with tab availability decided from
//! content and recorded DLC alone ([`shown_folders`]).
//!
//! The segment driver prepares it: [`facts`] projects each hour's index step
//! and fresh reads from columns, [`emit`] carries every country's state
//! through them and publishes it, and [`query`] answers from the publication.
use super::{CountryParams, LoadContext, Product, artwork};
use crate::{
    Artwork, ResearchBonus, ResearchCoverage, ResearchDetails, ResearchEntry, ResearchHistory,
    ResearchModifiers, ResearchOutcome, ResearchProgress, ResearchSlotHistory, ResearchSlotState,
    Source,
};
use replay_data::{rows::ResearchSlotRow, rows::ResearchTechnologyRow};
#[cfg(test)]
use replay_data::{rows::TableId, rows::tables};
#[cfg(test)]
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::LazyLock;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

#[cfg(test)]
use crate::metadata::validate_campaign;
#[cfg(test)]
use replay::container::Replay;
#[cfg(test)]
use replay_frame::frame::FrameDecoder;
#[cfg(test)]
use std::sync::atomic::Ordering;

pub mod emit;
pub mod facts;
pub mod grants;
mod lens;
mod modifiers;
pub mod query;
pub mod served;

#[cfg(test)]
#[derive(Default)]
pub struct ResearchCache {
    /// Each branch's latest country: its tag, history and the slot reads not joined.
    countries: BTreeMap<u64, (String, ResearchHistory, usize)>,
}

#[cfg(test)]
impl ResearchCache {
    pub fn get(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<(ResearchHistory, usize)> {
        check_cancel(cancelled)?;
        if let Some((held, history, unjoined)) = self.countries.get(&branch)
            && held == tag
        {
            return Ok((history.clone(), *unjoined));
        }
        let (history, unjoined) = research_branch_selected(replay, branch, Some(tag), cancelled)?
            .remove(tag)
            .ok_or_else(|| io::Error::other("No recorded research history for this country"))?;
        self.countries
            .insert(branch, (tag.to_owned(), history.clone(), unjoined));
        Ok((history, unjoined))
    }
}

#[cfg(test)]
fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Acquire) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Research request cancelled",
        ))
    } else {
        Ok(())
    }
}

#[derive(Clone, Default, Serialize, Deserialize, replay_spool::Spooled)]
struct Country {
    carry_until: Option<i32>,
    identity: Option<(u32, u32)>,
    continuity_start_hour: i32,
    history: ResearchHistory,
    // Active assignments retain the template index, level and researched membership.
    active: BTreeMap<u32, (u32, i32, bool)>,
    // Latest assignment occurrence per slot, including completed entries.
    entry_occurrences: Vec<Option<u32>>,
    samples: BTreeMap<u32, VecDeque<(i32, ProgressSample)>>,
    pending: Vec<(u32, usize, u32, i32, i32)>,
    /// The sample hour of the last observed research read.
    last_read: Option<i32>,
    /// Set before each observation: the previous read's hour when every
    /// recorded hour since it was intact. Research is read one country slice
    /// per hour, so consecutive reads of a country are usually a day apart;
    /// they are still continuous observations.
    read_after: Option<i32>,
    /// The hour of the last read observed into the history. Carried hours
    /// extend coverage past it; a new assignment is still bounded by it.
    observed_read: Option<i32>,
    /// Slot reads holding a technology whose occurrence was not recorded:
    /// shown in the slot's state, never joined into an assignment.
    unjoined_reads: usize,
    unjoined_starts: BTreeMap<u32, i32>,
    /// While a lens folds it, each slot read's step of the slot's state ([`lens::Chain`]).
    #[serde(skip)]
    #[spool(skip)]
    chains: Option<Vec<lens::Chain>>,
}

/// A slot's technology read without its recorded occurrence: named, but
/// never joined to an assignment or to the technology inventory.
struct UnjoinedTechnology {
    token: String,
    classifier: u32,
    ahead_reduction_years_raw: i64,
}

/// Whether an observation at `hour` directly continues one at `last`.
fn follows(read_after: Option<i32>, last: i32, hour: i32) -> bool {
    last.checked_add(1) == Some(hour) || read_after == Some(last)
}

fn cover(coverage: &mut Vec<ResearchCoverage>, hour: i32, read_after: Option<i32>) {
    if let Some(last) = coverage
        .last_mut()
        .filter(|last| follows(read_after, last.last_hour, hour))
    {
        last.last_hour = hour;
    } else {
        coverage.push(ResearchCoverage {
            first_hour: hour,
            last_hour: hour,
        });
    }
}

impl Country {
    fn materialize(&mut self, through: i32) {
        let Some(until) = self.carry_until else {
            return;
        };
        let through = through.min(until);
        let Some(from) = self.history.coverage.last().map(|span| span.last_hour) else {
            return;
        };
        if from >= through {
            return;
        }
        self.history.coverage.last_mut().unwrap().last_hour = through;
        for slot in &mut self.history.slots {
            if let Some(span) = slot
                .availability
                .last_mut()
                .filter(|span| span.last_hour == from)
            {
                span.last_hour = through;
                if let Some(state) = slot
                    .states
                    .last_mut()
                    .filter(|state| state.last_hour == from)
                {
                    state.last_hour = through;
                }
            }
        }
    }
    /// A read of `inputs` with the bonus and modifier rows read with them, where they were.
    fn observe_inputs(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        inputs: &[&replay_data::rows::ResearchInputRow],
        inventory: Option<&Inventory<'_>>,
        bonuses: Option<&[replay_data::rows::ResearchAppliedBonusRow]>,
        modifier_rows: Option<&[replay_data::rows::ResearchModifierRow]>,
    ) {
        if !valid_inputs(inputs) {
            return;
        }
        let owned: Vec<_> = inputs
            .iter()
            .map(|input| {
                (
                    bonuses.and_then(|rows| attributed_bonuses(input, rows)),
                    modifiers::breakdown(input, modifier_rows),
                )
            })
            .collect();
        let attributions: Vec<Attribution<'_>> = owned
            .iter()
            .map(|(bonuses, modifiers)| Attribution {
                bonuses: bonuses.as_deref(),
                modifiers: modifiers.as_ref(),
            })
            .collect();
        self.observe_attributed(identity, hour, inputs, inventory, &attributions);
    }

    /// A read of valid `inputs`, each with its attribution at its position.
    fn observe_attributed(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        inputs: &[&replay_data::rows::ResearchInputRow],
        inventory: Option<&Inventory<'_>>,
        attributions: &[Attribution<'_>],
    ) {
        self.observe_inputs_inner(identity, hour, inputs, inventory, attributions);
    }

    /// Whether `hour` can carry the last read: the same identity, the hour
    /// straight after the last known one, and before the next read is due.
    fn carries(&self, identity: (u32, u32), hour: i32) -> bool {
        self.identity == Some(identity)
            && self.last_read.is_some_and(|read| {
                i64::from(hour) - i64::from(read) < i64::from(replay_data::cadence::STAGGER_PERIOD)
            })
            && self
                .history
                .coverage
                .last()
                .is_some_and(|span| span.last_hour.checked_add(1) == Some(hour))
    }

    /// A recorded hour whose rows are still those of the last read: coverage,
    /// the slots read then and their state hold for this hour too, as a
    /// modifier change appears only at the next read. Progress keeps its read
    /// hour, since the game advances it daily. A gap, an unobservable hour,
    /// another identity or a missed read ends the carry until the next read.
    fn carry(&mut self, identity: (u32, u32), hour: i32) {
        if !self.carries(identity, hour) {
            return;
        }
        let next = |last: i32| last.checked_add(1) == Some(hour);
        if let Some(span) = self.history.coverage.last_mut() {
            span.last_hour = hour;
        }
        for slot in &mut self.history.slots {
            let Some(available) = slot.availability.last_mut().filter(|a| next(a.last_hour)) else {
                continue;
            };
            available.last_hour = hour;
            if let Some(state) = slot.states.last_mut().filter(|s| next(s.last_hour)) {
                state.last_hour = hour;
            }
        }
    }

    fn observe_inputs_inner(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        inputs: &[&replay_data::rows::ResearchInputRow],
        inventory: Option<&Inventory<'_>>,
        attributions: &[Attribution<'_>],
    ) {
        let read_after = self.read_after;
        let continuous = self.identity == Some(identity)
            && self
                .history
                .coverage
                .last()
                .is_some_and(|c| follows(read_after, c.last_hour, hour));
        if !continuous {
            self.samples.clear();
        }
        let mut slots = Vec::with_capacity(inputs.len());
        let mut technologies: BTreeMap<u32, Technology> = BTreeMap::new();
        // Inventory is sorted by (country, template index). Only former/current
        // assignments need completion evidence; never rescan the full catalogue
        // once per country per hour. A read of no slots still completes them.
        if let Some(inventory) = inventory {
            for occurrence in self
                .active
                .values()
                .map(|&(occurrence, _, _)| occurrence)
                .chain(
                    self.pending
                        .iter()
                        .map(|&(_, _, occurrence, _, _)| occurrence),
                )
            {
                if let Some(technology) = inventory.find(occurrence) {
                    technologies.insert(occurrence, technology);
                }
            }
        }
        let mut definitions = BTreeMap::new();
        let mut unjoined = BTreeMap::new();
        let mut details: BTreeMap<u32, DetailsRef<'_>> = BTreeMap::new();
        self.samples
            .retain(|index, _| inputs.iter().any(|r| r.index == *index));
        for (&input, attribution) in inputs.iter().zip(attributions) {
            let occurrence = match input.database_index {
                replay_data::rows::Recorded::Value(index) if input.technology_present != 0 => {
                    Some(index as u32)
                }
                _ => None,
            };
            if input.technology_present != 0 && occurrence.is_none() {
                self.unjoined_reads += 1;
                unjoined.insert(
                    input.index,
                    UnjoinedTechnology {
                        token: std::str::from_utf8(&input.token).unwrap().to_owned(),
                        classifier: input.classifier,
                        ahead_reduction_years_raw: input.ahead_reduction_raw,
                    },
                );
            }
            slots.push(ResearchSlotRow {
                country: input.country,
                index: input.index,
                technology_handle: input.technology_handle,
                technology_present: input.technology_present,
                database_index: input.database_index,
                research_speed_raw: input.research_speed_raw,
                banked_days_raw: input.banked_days_raw,
                applied_days_raw: input.applied_days_raw,
            });
            let samples = self.samples.entry(input.index).or_default();
            let same = samples
                .back()
                .is_some_and(|(_, last)| last.same_assignment(input));
            if !same {
                samples.clear();
            }
            let modifier_data = attribution.modifiers;
            let remaining_days = if input.technology_present != 0 {
                let daily_rate = samples
                    .iter()
                    .find(|(h, _)| h.checked_add(24) == Some(hour))
                    .filter(|(_, old)| old.applied_days_raw == input.applied_days_raw)
                    .map(|(_, old)| i128::from(input.progress_raw) - i128::from(old.progress_raw))
                    .or_else(|| {
                        // Slot speed is written at assignment, not refreshed each day.
                        // Use it only for a witnessed new assignment's first day;
                        // a read that cannot be joined witnesses no assignment.
                        let fresh = self
                            .history
                            .slots
                            .iter()
                            .find(|s| s.index == input.index)
                            .and_then(|s| s.entries.last())
                            .is_some_and(|e| {
                                e.started_after_hour.is_some()
                                    && hour.saturating_sub(e.first_observed_hour) < 24
                            });
                        (occurrence.is_some() && continuous && (!same || fresh))
                            .then_some(i128::from(input.research_speed_raw))
                    });
                let daily_rate = modifier_data
                    .and_then(|m| modifiers::rate(m, input.ahead_reduction_raw, hour))
                    .or(daily_rate);
                daily_rate.and_then(|rate| remaining(input, rate))
            } else {
                None
            };
            samples.push_back((hour, ProgressSample::from(input)));
            // A later hour reads only the sample a day before it.
            while samples
                .front()
                .is_some_and(|(at, _)| at.saturating_add(24) <= hour)
            {
                samples.pop_front();
            }
            if let Some(occurrence) = occurrence {
                definitions.insert(
                    occurrence,
                    std::str::from_utf8(&input.token).unwrap().to_owned(),
                );
                technologies.insert(
                    occurrence,
                    Technology {
                        classifier: input.classifier,
                        level: input.level,
                        researched: false,
                        ahead_reduction_years_raw: input.ahead_reduction_raw,
                    },
                );
            }
            details.insert(
                input.index,
                DetailsRef {
                    remaining_days,
                    bonuses: attribution.bonuses,
                    modifiers: modifier_data,
                },
            );
        }
        let previous_assignments = self.active.clone();
        self.observe(
            identity,
            hour,
            &slots.iter().collect::<Vec<_>>(),
            &technologies,
            &definitions,
            &unjoined,
            Some(&details),
        );
        for input in inputs.iter().filter(|input| input.technology_present != 0) {
            let Some(slot) = self
                .history
                .slots
                .iter_mut()
                .find(|s| s.index == input.index)
            else {
                continue;
            };
            let Some(entry) = slot.entries.last().filter(|e| {
                e.token.as_bytes() == input.token.as_slice()
                    && e.level == input.level
                    && e.ended_by_hour.is_none_or(|end| end == hour)
                    // Equal names cannot attribute another instance's progress.
                    && self
                        .active
                        .get(&input.index)
                        .or_else(|| previous_assignments.get(&input.index))
                        .is_some_and(|&(occurrence, _, _)| {
                            input.database_index
                                == replay_data::rows::Recorded::Value(occurrence as i32)
                        })
            }) else {
                continue;
            };
            let sample = ResearchProgress {
                first_hour: hour,
                last_hour: hour,
                entry_first_observed_hour: Some(entry.first_observed_hour),
                base_days_raw: base_cost(input).map(|cost| cost.to_string()),
                progress_raw: input.progress_raw.to_string(),
            };
            if let Some(last) = slot.progress.last_mut().filter(|last| {
                continuous
                    && follows(read_after, last.last_hour, hour)
                    && last.entry_first_observed_hour == sample.entry_first_observed_hour
                    && last.base_days_raw == sample.base_days_raw
                    && last.progress_raw == sample.progress_raw
            }) {
                last.last_hour = hour;
            } else {
                slot.progress.push(sample);
            }
        }
        // A technology held without its recorded occurrence still has exact
        // progress at its read; the slot's state at the same hour names it.
        for input in inputs.iter().filter(|input| {
            input.technology_present != 0
                && !matches!(input.database_index, replay_data::rows::Recorded::Value(_))
        }) {
            let Some(slot) = self
                .history
                .slots
                .iter_mut()
                .find(|s| s.index == input.index)
            else {
                continue;
            };
            // A finished technology still linked to the slot names no state.
            if !slot.states.last().is_some_and(|state| {
                state.last_hour == hour
                    && state.token.as_deref().map(str::as_bytes) == Some(input.token.as_slice())
            }) {
                continue;
            }
            let held_since = self.unjoined_starts.get(&input.index).copied();
            let sample = ResearchProgress {
                first_hour: hour,
                last_hour: hour,
                entry_first_observed_hour: None,
                base_days_raw: base_cost(input).map(|cost| cost.to_string()),
                progress_raw: input.progress_raw.to_string(),
            };
            // Equal values merge only while the slot held this same technology
            // without a break, never across another technology's reads.
            if let Some(last) = slot.progress.last_mut().filter(|last| {
                continuous
                    && follows(read_after, last.last_hour, hour)
                    && last.entry_first_observed_hour.is_none()
                    && held_since.is_some_and(|start| last.last_hour >= start)
                    && last.base_days_raw == sample.base_days_raw
                    && last.progress_raw == sample.progress_raw
            }) {
                last.last_hour = hour;
            } else {
                slot.progress.push(sample);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        slots: &[&ResearchSlotRow],
        technologies: &BTreeMap<u32, Technology>,
        definitions: &BTreeMap<u32, String>,
        unjoined: &BTreeMap<u32, UnjoinedTechnology>,
        details: Option<&BTreeMap<u32, DetailsRef<'_>>>,
    ) {
        let previous = self.history.coverage.last().map(|c| c.last_hour);
        let previous_read = self.observed_read.replace(hour);
        let read_after = self.read_after;
        let continuous = previous.is_some_and(|p| follows(read_after, p, hour))
            && self.identity == Some(identity);
        self.entry_occurrences.resize(slots.len(), None);
        if !continuous {
            self.continuity_start_hour = hour;
            for slot in &mut self.history.slots {
                if self.active.contains_key(&slot.index) {
                    let entry = slot.entries.last_mut().unwrap();
                    entry.outcome = ResearchOutcome::Unknown;
                    entry.ended_by_hour = Some(hour);
                }
            }
            self.active.clear();
            self.pending.clear();
            self.entry_occurrences.fill(None);
        }
        self.pending
            .retain(|&(index, entry_index, occurrence, level, ended)| {
                if hour.saturating_sub(ended) > replay_data::cadence::STAGGER_PERIOD
                    || slots.iter().any(|s| {
                        s.technology_present != 0
                            && s.database_index
                                .require("technology occurrence")
                                .ok()
                                .is_some_and(|index| *index as u32 == occurrence)
                            && technologies
                                .get(&occurrence)
                                .is_some_and(|t| t.level == level)
                    })
                {
                    return false;
                }
                if technologies.get(&occurrence).is_some_and(|t| {
                    t.level > level
                        || (level == 0 && t.researched)
                        || t.classifier == engine_research::ctechnology::CLASSIFIER_COMPLETED
                }) {
                    let entry = &mut self
                        .history
                        .slots
                        .iter_mut()
                        .find(|s| s.index == index)
                        .unwrap()
                        .entries[entry_index];
                    entry.outcome = ResearchOutcome::Completed;
                    entry.outcome_observed_hour = Some(hour);
                    false
                } else {
                    true
                }
            });
        self.identity = Some(identity);
        cover(&mut self.history.coverage, hour, read_after);
        for slot in &mut self.history.slots {
            let current = slots.iter().find(|s| s.index == slot.index);
            if let Some(&(occurrence, level, researched)) = self.active.get(&slot.index) {
                let technology = technologies.get(&occurrence);
                let transferred = slots.iter().any(|s| {
                    s.index != slot.index
                        && s.technology_present != 0
                        && s.database_index
                            .require("technology occurrence")
                            .ok()
                            .is_some_and(|index| *index as u32 == occurrence)
                });
                let completed = technology.is_some_and(|t| {
                    t.level > level
                        || (!researched && t.researched)
                        || (!transferred
                            && t.classifier == engine_research::ctechnology::CLASSIFIER_COMPLETED)
                });
                let same = current.is_some_and(|s| {
                    s.technology_present != 0
                        && s.database_index
                            .require("technology occurrence")
                            .ok()
                            .is_some_and(|index| *index as u32 == occurrence)
                });
                let entry = slot.entries.last_mut().unwrap();
                if completed || !same {
                    entry.ended_by_hour = Some(hour);
                    entry.outcome = if completed {
                        ResearchOutcome::Completed
                    } else if current.is_some() && technology.is_some() {
                        ResearchOutcome::Switched
                    } else {
                        ResearchOutcome::Unknown
                    };
                    if details.is_some() && !completed && !same && !transferred {
                        self.pending.push((
                            slot.index,
                            slot.entries.len() - 1,
                            occurrence,
                            level,
                            hour,
                        ));
                    }
                    self.active.remove(&slot.index);
                } else {
                    entry.last_observed_hour = hour;
                }
            }
        }
        for row in slots {
            let position = self
                .history
                .slots
                .iter()
                .position(|s| s.index == row.index)
                .unwrap_or_else(|| {
                    self.history.slots.push(ResearchSlotHistory {
                        index: row.index,
                        availability: Vec::new(),
                        entries: Vec::new(),
                        states: Vec::new(),
                        progress: Vec::new(),
                    });
                    self.history.slots.len() - 1
                });
            let slot = &mut self.history.slots[position];
            cover(&mut slot.availability, hour, read_after);
            if row.technology_present == 0 || self.active.contains_key(&row.index) {
                continue;
            }
            // Without its recorded occurrence a technology cannot start or
            // continue an assignment; the slot's state still shows it.
            let replay_data::rows::Recorded::Value(occurrence) = row.database_index else {
                continue;
            };
            let occurrence = occurrence as u32;
            let technology = technologies[&occurrence];
            // A completed assignment can remain linked for an observation. Do not
            // invent a new assignment until its recorded state becomes active again.
            if technology.classifier == engine_research::ctechnology::CLASSIFIER_COMPLETED
                || (technology.researched
                    && technology.classifier != engine_research::ctechnology::CLASSIFIER_ACTIVE)
            {
                continue;
            }
            slot.entries.push(ResearchEntry {
                token: definitions[&occurrence].clone(),
                technology_handle: row.technology_handle,
                database_index: Some(occurrence),
                continuity_start_hour: self.continuity_start_hour,
                level: technology.level,
                first_observed_hour: hour,
                started_after_hour: continuous.then_some(previous_read).flatten(),
                last_observed_hour: hour,
                ended_by_hour: None,
                outcome_observed_hour: None,
                outcome: ResearchOutcome::Active,
            });
            self.entry_occurrences[row.index as usize] = Some(occurrence);
            self.active.insert(
                row.index,
                (occurrence, technology.level, technology.researched),
            );
        }
        for row in slots {
            let slot = self
                .history
                .slots
                .iter_mut()
                .find(|slot| slot.index == row.index)
                .unwrap();
            let occurrence = match row.database_index {
                replay_data::rows::Recorded::Value(index) if row.technology_present != 0 => {
                    Some(index as u32)
                }
                _ => None,
            };
            // The technology the slot holds: its recorded occurrence's, or the
            // read's own name when the occurrence was not recorded.
            let held = occurrence
                .map(|occurrence| {
                    let technology = technologies[&occurrence];
                    (
                        definitions[&occurrence].as_str(),
                        technology.classifier,
                        technology.researched,
                        technology.ahead_reduction_years_raw,
                    )
                })
                .or_else(|| {
                    unjoined.get(&row.index).map(|technology| {
                        (
                            technology.token.as_str(),
                            technology.classifier,
                            false,
                            technology.ahead_reduction_years_raw,
                        )
                    })
                });
            let entry_first_observed_hour = held.and_then(|(token, ..)| {
                slot.entries
                    .last()
                    .filter(|entry| {
                        entry.token == token
                            && occurrence.is_some()
                            && self.entry_occurrences[row.index as usize] == occurrence
                    })
                    .map(|entry| entry.first_observed_hour)
            });
            // A finished technology still linked to the slot, with no
            // assignment joined to it, is not being researched.
            let held = held.filter(|&(_, classifier, researched, _)| {
                entry_first_observed_hour.is_some()
                    || !(classifier == engine_research::ctechnology::CLASSIFIER_COMPLETED
                        || (researched
                            && classifier != engine_research::ctechnology::CLASSIFIER_ACTIVE))
            });
            let (token, ahead) = held.map_or((None, None), |held| (Some(held.0), Some(held.3)));
            if let Some(token) = token.filter(|_| entry_first_observed_hour.is_none()) {
                let continues = continuous
                    && slot.states.last().is_some_and(|last| {
                        follows(read_after, last.last_hour, hour)
                            && last.entry_first_observed_hour.is_none()
                            && last.token.as_deref() == Some(token)
                    });
                if !continues {
                    self.unjoined_starts.insert(row.index, hour);
                }
            } else {
                self.unjoined_starts.remove(&row.index);
            }
            let read = StateRead {
                token,
                entry_first_observed_hour,
                speed_multiplier_raw: row.research_speed_raw,
                banked_days_raw: row.banked_days_raw,
                applied_days_raw: row.applied_days_raw,
                ahead_reduction_years_raw: ahead,
                details: details.and_then(|d| d.get(&row.index)).copied(),
            };
            let last = slot
                .states
                .last()
                .filter(|last| continuous && follows(read_after, last.last_hour, hour));
            let gate = last.is_some_and(|last| read.others(last));
            let kept = gate && last.is_some_and(|last| read.countdown(last, hour));
            if let Some(chains) = &mut self.chains {
                chains.push(lens::Chain {
                    slot: row.index,
                    gate,
                    kept,
                    remaining: read.details.and_then(|details| details.remaining_days),
                    held: read.token.is_some(),
                    entry: read.entry_first_observed_hour,
                });
            }
            if kept && let Some(last) = slot.states.last_mut() {
                last.last_hour = hour;
                continue;
            }
            slot.states.push(read.state(hour));
        }
    }
}

/// A read's slot state, borrowed until it starts a state of its own.
struct StateRead<'a> {
    token: Option<&'a str>,
    entry_first_observed_hour: Option<i32>,
    speed_multiplier_raw: i64,
    banked_days_raw: i64,
    applied_days_raw: i64,
    ahead_reduction_years_raw: Option<i64>,
    details: Option<DetailsRef<'a>>,
}

impl StateRead<'_> {
    /// Whether it equals `last`, the state before, in all but its countdown ([`Self::countdown`]):
    /// both continue it, read on at an hour. A steady countdown needs one segment, not one per
    /// day. A different rate, bonus, missing sample or assignment breaks it.
    fn others(&self, last: &ResearchSlotState) -> bool {
        let details = match (&last.details, &self.details) {
            (None, None) => true,
            (Some(before), Some(now)) => {
                before.bonuses.as_deref() == now.bonuses
                    && before.modifiers.as_ref() == now.modifiers
            }
            _ => false,
        };
        details
            && last.token.as_deref() == self.token
            && last.entry_first_observed_hour == self.entry_first_observed_hour
            && raw_is(&last.speed_multiplier_raw, self.speed_multiplier_raw)
            && raw_is(&last.banked_days_raw, self.banked_days_raw)
            && raw_is(&last.applied_days_raw, self.applied_days_raw)
            && match (&last.ahead_reduction_years_raw, self.ahead_reduction_years_raw) {
                (None, None) => true,
                (Some(text), Some(value)) => raw_is(text, value),
                _ => false,
            }
    }

    /// Whether its remaining days continue `last`'s, read on at `hour`.
    fn countdown(&self, last: &ResearchSlotState, hour: i32) -> bool {
        match (&last.details, &self.details) {
            (Some(before), Some(now)) => counts_down(
                before.remaining_days,
                last.first_hour,
                hour,
                now.remaining_days,
            ),
            _ => true,
        }
    }

    fn state(&self, hour: i32) -> ResearchSlotState {
        ResearchSlotState {
            first_hour: hour,
            last_hour: hour,
            details: self.details.map(|details| details.owned()),
            entry_first_observed_hour: self.entry_first_observed_hour,
            token: self.token.map(str::to_owned),
            speed_multiplier_raw: self.speed_multiplier_raw.to_string(),
            banked_days_raw: self.banked_days_raw.to_string(),
            applied_days_raw: self.applied_days_raw.to_string(),
            ahead_reduction_years_raw: self.ahead_reduction_years_raw.map(|raw| raw.to_string()),
        }
    }
}

/// Whether a state of `before` remaining days at `first` goes on at `hour` with `now`: its
/// days counted down by the whole days since, or the same.
fn counts_down(before: Option<u32>, first: i32, hour: i32, now: Option<u32>) -> bool {
    let counted = before
        .map(|days| days.saturating_sub(((i64::from(hour) - i64::from(first)) / 24) as u32));
    counted == now || before == now
}

/// Whether `text` is `value` as `to_string` writes it.
fn raw_is(text: &str, value: i64) -> bool {
    let mut digits = [0_u8; 20];
    let mut at = digits.len();
    let mut rest = value.unsigned_abs();
    loop {
        at -= 1;
        digits[at] = b'0' + (rest % 10) as u8;
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    match text.as_bytes().split_first() {
        Some((b'-', tail)) => value < 0 && tail == &digits[at..],
        _ => value >= 0 && text.as_bytes() == &digits[at..],
    }
}

/// A slot read's details, borrowed from its attribution.
#[derive(Clone, Copy)]
pub(crate) struct DetailsRef<'a> {
    remaining_days: Option<u32>,
    bonuses: Option<&'a [ResearchBonus]>,
    modifiers: Option<&'a ResearchModifiers>,
}

impl DetailsRef<'_> {
    fn owned(&self) -> ResearchDetails {
        ResearchDetails {
            remaining_days: self.remaining_days,
            bonuses: self.bonuses.map(<[ResearchBonus]>::to_vec),
            modifiers: self.modifiers.cloned(),
        }
    }
}

/// What a slot read's bonus and modifier rows attribute to it: None where they were not read
/// with it or do not attribute.
#[derive(Clone, Copy, Default)]
pub(crate) struct Attribution<'a> {
    pub(crate) bonuses: Option<&'a [ResearchBonus]>,
    pub(crate) modifiers: Option<&'a ResearchModifiers>,
}

/// A template occurrence's inventory fields the country step reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, replay_spool::Spooled)]
pub(crate) struct Technology {
    pub(crate) classifier: u32,
    pub(crate) level: i32,
    pub(crate) researched: bool,
    pub(crate) ahead_reduction_years_raw: i64,
}

impl From<&ResearchTechnologyRow> for Technology {
    fn from(row: &ResearchTechnologyRow) -> Self {
        Self {
            classifier: row.classifier,
            level: row.level,
            researched: row.researched != 0,
            ahead_reduction_years_raw: row.ahead_reduction_years_raw,
        }
    }
}

/// Whether a read's slots can be observed: any number of slots, as content and effects set
/// how many a country has. A recorded occurrence must be a template index. One that was not
/// recorded keeps the read; only the joins that need it are refused.
pub(crate) fn valid_inputs(inputs: &[&replay_data::rows::ResearchInputRow]) -> bool {
    !inputs.iter().enumerate().any(|(i, r)| {
        r.index as usize != i
            || r.technology_present > 1
            || (r.technology_present != 0
                && (matches!(r.database_index, replay_data::rows::Recorded::Value(index) if index <= 0)
                    || r.token.is_empty()
                    || r.token.len() > 512
                    || std::str::from_utf8(&r.token).is_err()))
    })
}

/// The bonuses `rows` attribute to `input`'s slot: its owner's rows of the slot, in key order,
/// with the slot's technology, as many as it applied and numbered from zero.
pub(crate) fn attributed_bonuses(
    input: &replay_data::rows::ResearchInputRow,
    rows: &[replay_data::rows::ResearchAppliedBonusRow],
) -> Option<Vec<ResearchBonus>> {
    let matching: Vec<_> = rows
        .iter()
        .filter(|b| {
            b.country == input.country
                && b.index == input.index
                && b.technology_handle == input.technology_handle
        })
        .collect();
    if matching.len() != input.bonus_count as usize
        || matching
            .iter()
            .enumerate()
            .any(|(i, b)| b.ordinal as usize != i || b.resolved > 1 || b.name.len() > 512)
    {
        return None;
    }
    Some(
        matching
            .into_iter()
            .map(|b| ResearchBonus {
                id: b.id,
                token: (b.resolved != 0)
                    .then(|| std::str::from_utf8(&b.name).ok().map(str::to_owned))
                    .flatten(),
                name: None,
                cost_reduction_raw: (b.resolved != 0)
                    .then_some(b.cost_reduction_raw)
                    .flatten()
                    .map(|v| v.to_string()),
                speed_bonus_raw: (b.resolved != 0).then(|| b.speed_bonus_raw.to_string()),
                ahead_reduction_years_raw: (b.resolved != 0)
                    .then(|| b.ahead_reduction_raw.to_string()),
            })
            .collect(),
    )
}

/// The folder each token is shown in: the first folder it is placed in whose
/// tab is available, as the game's technology screen shows it in each available
/// tab. A token placed only in unavailable folders is shown in none. One placed
/// in a folder whose availability cannot be decided from content, before any
/// available folder, stays unknown: that folder may be the one shown. A folder
/// `technology_folders` does not define is never available. Unknown folders and
/// undefined folders are named and counted.
pub(crate) fn shown_folders<'a>(
    definitions: &[(&str, Option<bool>)],
    placed: &BTreeMap<(String, Option<u32>), Vec<String>>,
    tokens: impl IntoIterator<Item = (&'a str, Option<u32>)>,
) -> (Vec<crate::ResearchTechnologyFolder>, Vec<String>) {
    let mut undecided: Vec<&str> = Vec::new();
    let mut undefined: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut shown = Vec::new();
    for (token, database_index) in tokens {
        let mut folder = None;
        let mut unknown = false;
        for key in placed
            .get(&(token.to_owned(), database_index))
            .into_iter()
            .flatten()
        {
            match definitions
                .iter()
                .find(|(defined, _)| *defined == key.as_str())
            {
                Some((_, Some(true))) => {
                    folder = Some(key.clone());
                    break;
                }
                Some((_, Some(false))) => {}
                Some((_, None)) => {
                    unknown = true;
                    break;
                }
                None => undefined.entry(key.as_str()).or_default().push(token),
            }
        }
        if unknown {
            undecided.push(token);
        }
        shown.push(crate::ResearchTechnologyFolder {
            token: token.to_owned(),
            database_index,
            folder,
        });
    }
    let mut notes = Vec::new();
    if !undecided.is_empty() {
        let one = undecided.len() == 1;
        notes.push(format!(
            "{} {} placed in a research folder whose availability cannot be decided from content before any available folder, so {} folder is unknown: {}.",
            undecided.len(),
            if one { "technology is" } else { "technologies are" },
            if one { "its" } else { "their" },
            undecided.join(", ")
        ));
    }
    for (key, tokens) in undefined {
        notes.push(format!(
            "Research folder {key} is not defined in common/technology_tags, so {} {} placed in it cannot be shown there: {}.",
            tokens.len(),
            if tokens.len() == 1 { "technology" } else { "technologies" },
            tokens.join(", ")
        ));
    }
    (shown, notes)
}

/// Only the assignment identity and past operands needed by the countdown.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, replay_spool::Spooled)]
struct ProgressSample {
    owner: u32,
    present: u8,
    handle: u32,
    occurrence: Option<i32>,
    level: i32,
    token: Vec<u8>,
    progress_raw: i64,
    applied_days_raw: i64,
}
impl From<&replay_data::rows::ResearchInputRow> for ProgressSample {
    fn from(row: &replay_data::rows::ResearchInputRow) -> Self {
        Self {
            owner: row.country,
            present: row.technology_present,
            handle: row.technology_handle,
            occurrence: row
                .database_index
                .require("technology occurrence")
                .ok()
                .copied(),
            level: row.level,
            token: row.token.clone(),
            progress_raw: row.progress_raw,
            applied_days_raw: row.applied_days_raw,
        }
    }
}
impl ProgressSample {
    fn same_assignment(&self, row: &replay_data::rows::ResearchInputRow) -> bool {
        self.owner == row.country
            && self.present == row.technology_present
            && (self.present == 0 || self.occurrence.is_some())
            && self.handle == row.technology_handle
            && self.occurrence
                == row
                    .database_index
                    .require("technology occurrence")
                    .ok()
                    .copied()
            && self.level == row.level
            && self.token == row.token
    }
}

fn base_cost(row: &replay_data::rows::ResearchInputRow) -> Option<i128> {
    let cost = if row.cost_debug != 0 {
        100_000
    } else {
        i128::from(row.cost_raw).checked_mul(i128::from(row.cost_scale_raw))? / 100_000
    };
    (cost > 0 && cost <= i128::from(i64::MAX)).then_some(cost)
}

fn remaining(row: &replay_data::rows::ResearchInputRow, rate: i128) -> Option<u32> {
    if rate <= 0 || row.progress_raw < 0 {
        return None;
    }
    let cost = base_cost(row)?;
    let left = (cost - i128::from(row.progress_raw)).max(0);
    u32::try_from((left + rate - 1) / rate).ok()
}

/// The named refusal for slot reads whose technology occurrence was not recorded.
fn unjoined_warning(reads: usize) -> String {
    format!(
        "{reads} research slot {} technology without its recorded occurrence, so {} not joined into assignments: their completions and the time they took are unknown.",
        if reads == 1 {
            "read names its"
        } else {
            "reads name their"
        },
        if reads == 1 { "it is" } else { "they are" }
    )
}

#[cfg(test)]
pub fn research_branch(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, ResearchHistory>> {
    Ok(research_branch_selected(replay, branch, None, cancelled)?
        .into_iter()
        .map(|(tag, (history, _))| (tag, history))
        .collect())
}

/// The Countries fields a country's research holder is read from; its id is
/// the row key.
#[cfg(test)]
static HOLDER_FIELDS: LazyLock<u64> = LazyLock::new(|| {
    crate::prepared::frame_changes::field_mask(TableId::Countries, &["canonical_slot", "tag"])
});

/// One owner's rows of a table keyed country first.
#[cfg(test)]
fn owned<T>(rows: &[T], owner: u32, country: impl Fn(&T) -> u32) -> &[T] {
    let first = rows.partition_point(|row| country(row) < owner);
    let count = rows[first..].partition_point(|row| country(row) == owner);
    &rows[first..first + count]
}

/// Each owner's rows of a table keyed country first, for the owners holding
/// any: with `owners` ascending, a slot's rows in table order.
#[cfg(test)]
fn owner_rows<'a, T>(
    rows: &'a [T],
    owners: &[u32],
    country: impl Fn(&T) -> u32 + Copy,
) -> Vec<&'a [T]> {
    owners
        .iter()
        .map(|&owner| owned(rows, owner, country))
        .filter(|part| !part.is_empty())
        .collect()
}

/// The parts as one slice, borrowed unless several owners hold rows.
#[cfg(test)]
fn joined<'a, T: Clone>(parts: &[&'a [T]]) -> Cow<'a, [T]> {
    match parts {
        [] => Cow::Owned(Vec::new()),
        [part] => Cow::Borrowed(*part),
        parts => Cow::Owned(parts.concat()),
    }
}

/// What a fresh read of one slot's owners reads in its hour: their research
/// rows, the read hour they all share (None when they were not all read at
/// the hour), the bonuses and modifiers read with them, and the owners'
/// technology inventory.
pub(crate) struct FreshRead<'a> {
    inputs: Vec<&'a replay_data::rows::ResearchInputRow>,
    read: Option<i32>,
    joins: Joins<'a>,
    inventory: Option<Inventory<'a>>,
}

impl<'a> FreshRead<'a> {
    /// The read a projection kept ([`facts::Fresh`]).
    pub(crate) fn kept(kept: &'a facts::Fresh) -> Self {
        Self {
            inputs: kept.inputs.iter().collect(),
            read: kept.read,
            joins: Joins::Attributed(kept.derived.iter().map(facts::Derived::attribution).collect()),
            inventory: kept
                .inventory
                .as_ref()
                .map(|inventory| Inventory::Held(inventory.rows())),
        }
    }
}

/// What a fresh read's slots join beside their own rows.
pub(crate) enum Joins<'a> {
    /// The bonus and modifier rows read with them, where they come from the same read.
    Rows {
        bonuses: Option<Cow<'a, [replay_data::rows::ResearchAppliedBonusRow]>>,
        modifiers: Option<Cow<'a, [replay_data::rows::ResearchModifierRow]>>,
    },
    /// What those rows attribute to each slot, at its position; empty where the slots are not
    /// observable.
    Attributed(Vec<Attribution<'a>>),
}

/// The technology inventory a read joins its assignments to: its owner's rows.
pub(crate) enum Inventory<'a> {
    /// Rows sorted by country and template index: the whole table, or the
    /// owner's part of it.
    Table {
        owner: u32,
        rows: &'a [ResearchTechnologyRow],
    },
    /// The owner's rows by template index, ascending: all of them, or those of the indexes a
    /// lookup can ask for.
    Held(&'a [(u32, Technology)]),
}

impl Inventory<'_> {
    /// The owner's row of a template index.
    fn find(&self, occurrence: u32) -> Option<Technology> {
        match self {
            Self::Table { owner, rows } => rows
                .binary_search_by_key(&(*owner, occurrence), |t| {
                    (t.country, t.database_index as u32)
                })
                .ok()
                .map(|index| Technology::from(&rows[index])),
            Self::Held(rows) => rows
                .binary_search_by_key(&occurrence, |&(index, _)| index)
                .ok()
                .map(|at| rows[at].1),
        }
    }
}

/// How the country step reads an hour's research rows and read hours.
pub(crate) trait Reads {
    /// `owner`'s ResearchInputs read hour; `slot` is its canonical slot.
    fn read_hour(&self, slot: u32, owner: u32) -> Option<i32>;
    /// The fresh read of `slot`, whose owners are `owners`.
    fn fresh(&self, slot: u32, owners: &[u32]) -> io::Result<FreshRead<'_>>;
}

/// An hour's research rows and read hours, read from its frame.
#[cfg(test)]
pub(crate) struct FrameReads<'a> {
    hour: i32,
    inputs: &'a [replay_data::rows::ResearchInputRow],
    reads: &'a [replay_data::rows::SliceReadRow],
    inventory: Option<&'a [ResearchTechnologyRow]>,
    bonuses: Option<&'a [replay_data::rows::ResearchAppliedBonusRow]>,
    modifiers: Option<&'a [replay_data::rows::ResearchModifierRow]>,
}

#[cfg(test)]
impl<'a> FrameReads<'a> {
    /// `frame`'s, when its research rows and read hours are complete.
    pub(crate) fn of(frame: &'a DecodedFrame) -> Option<Self> {
        Some(Self {
            hour: frame.hour,
            inputs: frame.complete::<tables::ResearchInputs>().ok()?,
            reads: frame.complete::<tables::SliceReads>().ok()?,
            inventory: frame.complete::<tables::ResearchTechnologies>().ok(),
            bonuses: frame.complete::<tables::ResearchAppliedBonuses>().ok(),
            modifiers: frame.complete::<tables::ResearchModifiers>().ok(),
        })
    }

    fn read_hour_of(&self, table: TableId, owner: u32) -> Option<i32> {
        replay_data::sliced::read_hour(self.reads, table, owner)
    }

    /// The fresh read of owners `owners`, with the whole inventory.
    pub(crate) fn fresh_read(&self, owners: &[u32]) -> FreshRead<'a> {
        let inputs: Vec<_> = owner_rows(self.inputs, owners, |row| row.country)
            .into_iter()
            .flatten()
            .collect();
        // SliceReads proves a read of no slots as well as one of many: the read is
        // this hour's when an owner was read now and every owner with rows was too.
        // Its inventory is that of its first owner holding slots, else of its first
        // owner read now.
        let fresh = |owner: u32| self.read_hour_of(TableId::ResearchInputs, owner) == Some(self.hour);
        let owner = inputs
            .first()
            .map(|input| input.country)
            .or_else(|| owners.iter().copied().find(|&owner| fresh(owner)));
        let read = owner
            .filter(|&owner| fresh(owner) && inputs.iter().all(|input| fresh(input.country)))
            .map(|_| self.hour);
        // Bonuses and modifiers join only when they come from the same
        // read; every row of one owner has that owner's read hour.
        let (bonuses, modifiers) = match read {
            Some(read) => (
                self.bonuses
                    .map(|rows| owner_rows(rows, owners, |row| row.country))
                    .filter(|parts| {
                        parts.iter().all(|part| {
                            self.read_hour_of(TableId::ResearchAppliedBonuses, part[0].country)
                                == Some(read)
                        })
                    })
                    .map(|parts| joined(&parts)),
                self.modifiers
                    .map(|rows| owner_rows(rows, owners, |row| row.country))
                    .filter(|parts| {
                        parts.iter().all(|part| {
                            self.read_hour_of(TableId::ResearchModifiers, part[0].country)
                                == Some(read)
                        })
                    })
                    .map(|parts| joined(&parts)),
            ),
            None => (None, None),
        };
        FreshRead {
            inputs,
            read,
            joins: Joins::Rows { bonuses, modifiers },
            inventory: self
                .inventory
                .zip(owner)
                .map(|(rows, owner)| Inventory::Table { owner, rows }),
        }
    }
}

#[cfg(test)]
impl Reads for FrameReads<'_> {
    fn read_hour(&self, _: u32, owner: u32) -> Option<i32> {
        self.read_hour_of(TableId::ResearchInputs, owner)
    }

    fn fresh(&self, _: u32, owners: &[u32]) -> io::Result<FreshRead<'_>> {
        Ok(self.fresh_read(owners))
    }
}

/// The index values an hour's [`Scan::refresh`] changed that the country
/// step reads: the holders by slot with the slot of each link, the owners by
/// slot, and the read hours of each slot's owners (all of them, or the slots
/// whose hours changed, None for a slot with none).
#[derive(Default, replay_spool::Spooled)]
pub(crate) struct IndexChanges {
    countries: Option<(Arc<HashMap<u32, u32>>, CountryRows)>,
    owners: Option<BTreeMap<u32, Vec<u32>>>,
    dates: Option<BTreeMap<u32, Vec<(u32, i32)>>>,
    // Keep changed slots paired with their optional owner/date lists in this index update.
    slot_dates: SlotDates,
}

/// Each canonical slot's holders: their Countries row, id and tag.
type CountryRows = BTreeMap<u32, Vec<(usize, u32, Vec<u8>)>>;

/// The indexes the country step reads beside its scan, as an hour's index step leaves them:
/// the holders' slots, each slot's holders, owners and owners' read hours.
#[derive(Clone, Copy)]
pub(crate) struct Indexes<'a> {
    pub(crate) slots: Option<&'a HashMap<u32, u32>>,
    pub(crate) rows: &'a CountryRows,
    pub(crate) owners: &'a BTreeMap<u32, Vec<u32>>,
    pub(crate) dates: &'a BTreeMap<u32, Vec<(u32, i32)>>,
}

/// The read hours of the slots whose hours changed, None for a slot with none.
type SlotDates = Vec<(u32, Option<Vec<(u32, i32)>>)>;

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    countries: BTreeMap<String, Country>,
    last_bad: Option<i32>,
    last_frame: Option<i32>,
    #[cfg(test)]
    #[serde(skip)]
    country_reads: Option<super::territory::CountryReads>,
    /// How many Countries rows `country_reads` was read from.
    #[cfg(test)]
    #[serde(skip)]
    country_count: usize,
    #[cfg(test)]
    #[serde(skip)]
    country_rows: BTreeMap<u32, Vec<(usize, u32, Vec<u8>)>>,
    #[cfg(test)]
    #[serde(skip)]
    owners: BTreeMap<u32, Vec<u32>>,
    /// The canonical slots whose owners hold ResearchInputs rows.
    #[cfg(test)]
    #[serde(skip)]
    input_slots: BTreeSet<u32>,
    #[cfg(test)]
    #[serde(skip)]
    input_dates: BTreeMap<u32, Vec<(u32, i32)>>,
    /// The latest ResearchInputs read hour seen later than the hour holding
    /// it. Until that hour passes, an hour is observed from its whole tables:
    /// such a read becomes fresh without any row changing.
    #[cfg(test)]
    #[serde(skip)]
    future_read: Option<i32>,
    /// Slots observed again at the next hour although no row of theirs
    /// changes: a read whose owners' read hours do not all prove it carries
    /// only until then, and progress shown at its read hour alone ends then.
    #[serde(skip)]
    recheck: BTreeSet<u32>,
    /// The hour whose rows the indexes above were refreshed from.
    #[cfg(test)]
    #[serde(skip)]
    indexed: Option<i32>,
    #[serde(skip)]
    deadlines: BTreeSet<(i32, String)>,
    #[serde(skip)]
    ready: bool,
    #[serde(skip)]
    observed: BTreeSet<String>,
    /// The countries a lens folds ([`lens`]): this scan steps them not, keeping each as the
    /// lens took it, and their carries' deadlines are the lens's.
    #[serde(skip)]
    lensed: BTreeSet<String>,
}
impl Scan {
    #[cfg(test)]
    fn reject(&mut self, hour: i32) {
        self.last_frame = Some(hour);
        self.last_bad = Some(hour);
    }

    fn close_country(&mut self, tag: &str, previous: Option<i32>) {
        let country = self.countries.get_mut(tag).unwrap();
        if let Some(through) = previous {
            country.materialize(through);
        }
        if let Some(due) = country.carry_until.and_then(|until| until.checked_add(1)) {
            self.deadlines.remove(&(due, tag.to_owned()));
        }
        country.carry_until = None;
        self.observed.insert(tag.to_owned());
    }
    fn close_held(&mut self, previous: Option<i32>) {
        for (_, tag) in std::mem::take(&mut self.deadlines) {
            let country = self.countries.get_mut(&tag).unwrap();
            if let Some(through) = previous {
                country.materialize(through);
            }
            country.carry_until = None;
            self.observed.insert(tag);
        }
    }
    fn seal(&mut self) {
        if let Some(through) = self.last_frame {
            self.deadlines.retain(|(_, tag)| {
                let Some(country) = self.countries.get_mut(tag) else {
                    return false;
                };
                country.materialize(through);
                true
            });
        }
    }
    /// The reference scan and preparation share every supplying-read decision.
    #[cfg(test)]
    fn observe_selected(
        &mut self,
        frame: &DecodedFrame,
        selected_tag: Option<&str>,
    ) -> io::Result<()> {
        self.observe_selected_changed(
            frame,
            selected_tag,
            &crate::prepared::frame_changes::FrameChanges {
                rows_changed: [true; replay_data::rows::TABLE_COUNT],
                table_read: [true; replay_data::rows::TABLE_COUNT],
                slot_read: [true; replay_data::rows::TABLE_COUNT],
                updates: None,
            },
            false,
        )
    }
    #[cfg(test)]
    fn observe_selected_changed(
        &mut self,
        frame: &DecodedFrame,
        selected_tag: Option<&str>,
        changes: &crate::prepared::frame_changes::FrameChanges,
        deferred: bool,
    ) -> io::Result<()> {
        let previous = self.begin_hour(frame.hour);
        let Some((dirty, countries_changed)) = self.refresh(frame, changes, deferred) else {
            self.close_held(previous);
            self.last_bad = Some(frame.hour);
            return Ok(());
        };
        let reads = FrameReads::of(frame)
            .ok_or_else(|| io::Error::other("Refreshed research tables are incomplete"))?;
        let slots = Arc::clone(self.country_reads.as_ref().unwrap().slots());
        let rows = std::mem::take(&mut self.country_rows);
        let owners = std::mem::take(&mut self.owners);
        let dates = std::mem::take(&mut self.input_dates);
        let indexes = Indexes {
            slots: Some(&slots),
            rows: &rows,
            owners: &owners,
            dates: &dates,
        };
        let applied = self.apply(
            frame.hour,
            previous,
            dirty,
            countries_changed,
            indexes,
            &reads,
            selected_tag,
            deferred,
        );
        self.country_rows = rows;
        self.owners = owners;
        self.input_dates = dates;
        applied
    }

    /// Hour `hour` from what its [`Self::refresh`] found elsewhere
    /// (`refreshed`, None for an incomplete hour) and the indexes it left,
    /// `indexes`, with the hour's rows read through `reads`: as
    /// [`Self::observe_selected_changed`] observes the hour.
    pub(crate) fn observe_refreshed(
        &mut self,
        hour: i32,
        refreshed: Option<(BTreeSet<u32>, bool)>,
        indexes: Indexes<'_>,
        reads: &dyn Reads,
    ) -> io::Result<()> {
        let previous = self.begin_hour(hour);
        let Some((dirty, countries_changed)) = refreshed else {
            self.ready = false;
            self.close_held(previous);
            self.last_bad = Some(hour);
            return Ok(());
        };
        self.ready = true;
        #[cfg(test)]
        {
            self.indexed = Some(hour);
        }
        if indexes.slots.is_none() {
            return Err(io::Error::other("Refreshed research holders are missing"));
        }
        self.apply(
            hour,
            previous,
            dirty,
            countries_changed,
            indexes,
            reads,
            None,
            true,
        )
    }

    /// Begins `tag`, held by `identity`, from its read at `read`, the first it observes, as the
    /// country step of that hour takes a fresh read ([`Self::apply`]).
    fn begin(&mut self, tag: &str, identity: (u32, u32), read: i32, rows: &FreshRead<'_>) {
        let state = self.countries.entry(tag.to_owned()).or_default();
        if let Joins::Attributed(attributions) = &rows.joins
            && valid_inputs(&rows.inputs)
            && attributions.len() == rows.inputs.len()
        {
            state.observe_attributed(
                identity,
                read,
                &rows.inputs,
                rows.inventory.as_ref(),
                attributions,
            );
        }
        state.last_read = Some(read);
        state.carry_until = state
            .history
            .coverage
            .last()
            .filter(|span| span.last_hour == read)
            .and_then(|_| read.checked_add(replay_data::cadence::STAGGER_PERIOD - 1));
    }

    /// Hands `tag` to a lens: its state stays as it is now, the scan no longer steps it, and its
    /// carry's deadline and `canonical`'s next observation are the lens's.
    fn lens(&mut self, tag: &str, canonical: u32) {
        if let Some(due) = self
            .countries
            .get(tag)
            .and_then(|country| country.carry_until)
            .and_then(|until| until.checked_add(1))
        {
            self.deadlines.remove(&(due, tag.to_owned()));
        }
        self.recheck.remove(&canonical);
        self.lensed.insert(tag.to_owned());
    }

    /// Takes `tag` back from its lens as `country`, whose `canonical` slot is observed again at
    /// the next hour where `recheck`.
    fn unlens(&mut self, tag: &str, country: Country, canonical: u32, recheck: bool) {
        self.lensed.remove(tag);
        if let Some(due) = country.carry_until.and_then(|until| until.checked_add(1)) {
            self.deadlines.insert((due, tag.to_owned()));
        }
        if recheck {
            self.recheck.insert(canonical);
        }
        self.countries.insert(tag.to_owned(), country);
    }

    /// An hour's first step, before its tables are read: carries end at the
    /// deadlines it reaches, a gap ends every carry, and the hour becomes the
    /// last observed. The hour before it observed, if any.
    fn begin_hour(&mut self, hour: i32) -> Option<i32> {
        self.observed.clear();
        if !self.ready {
            let lensed = &self.lensed;
            self.deadlines = self
                .countries
                .iter()
                .filter(|(tag, _)| !lensed.contains(*tag))
                .filter_map(|(tag, country)| {
                    country
                        .carry_until
                        .and_then(|until| until.checked_add(1))
                        .map(|due| (due, tag.clone()))
                })
                .collect();
        }
        let previous = self.last_frame;
        if previous.is_some_and(|before| before.checked_add(1) != Some(hour)) {
            self.seal();
            for (_, tag) in std::mem::take(&mut self.deadlines) {
                self.countries.get_mut(&tag).unwrap().carry_until = None;
                self.observed.insert(tag);
            }
        }
        while self.deadlines.first().is_some_and(|(due, _)| *due <= hour) {
            let (due, tag) = self.deadlines.pop_first().unwrap();
            let country = self.countries.get_mut(&tag).unwrap();
            country.materialize(previous.unwrap_or(due - 1).min(due - 1));
            country.carry_until = None;
            self.observed.insert(tag);
        }
        if self
            .last_frame
            .is_some_and(|before| before.checked_add(1) != Some(hour))
        {
            self.last_bad = Some(hour.saturating_sub(1));
        }
        self.last_frame = Some(hour);
        previous
    }

    /// The index step of an hour: the indexes of the frame's holders, links,
    /// research rows and read hours, refreshed from the hour before's by the
    /// hour's changes (or rebuilt). It reads the frame alone and the indexes
    /// the hours before left, never a country's state. The slots to observe
    /// again and whether the holders changed; None when a table it needs is
    /// incomplete, so the next hour rebuilds them. The projection's index step
    /// ([`facts`]) takes it from columns.
    #[cfg(test)]
    fn refresh(
        &mut self,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
        deferred: bool,
    ) -> Option<(BTreeSet<u32>, bool)> {
        let (Ok(rows), Ok(links), Ok(inputs)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::ResearchInputs>(),
        ) else {
            // Indexes this hour's rows did not refresh are rebuilt at the next.
            self.ready = false;
            return None;
        };
        // Every sliced row's read hour; without it no read is observable.
        let Ok(reads) = frame.complete::<tables::SliceReads>() else {
            self.ready = false;
            return None;
        };
        let read_hour = |table, owner| replay_data::sliced::read_hour(reads, table, owner);
        let forced = !self.ready;
        let links_changed = forced || changes.rows_changed[TableId::CountryLinks.index()];
        // The indexes hold the previous hour's rows, with the same links, so a
        // table its recorded rows changed changes them by those rows alone.
        // A pending read dated after its hour becomes fresh with no row
        // changing, so until it passes the whole tables are observed.
        let narrow = deferred
            && !links_changed
            && self.indexed.and_then(|hour| hour.checked_add(1)) == Some(frame.hour)
            && self.future_read.is_none_or(|read| read < frame.hour);
        let changed_rows = |table: TableId| if narrow { changes.rows(table) } else { None };
        let countries_changed =
            if forced || links_changed || changes.rows_changed[TableId::Countries.index()] {
                // Recorded changes that add or delete no country and touch no
                // holder field keep the previous hour's holders.
                let kept = self.country_count == rows.len()
                    && changed_rows(TableId::Countries).is_some_and(|(changed, deleted)| {
                        deleted.is_empty()
                            && changed.iter().all(|row| row.mask & *HOLDER_FIELDS == 0)
                    });
                let same = self
                    .country_reads
                    .as_ref()
                    .is_some_and(|seen| kept || seen.matches(frame, rows));
                if !same {
                    self.country_count = rows.len();
                    self.country_reads =
                        Some(if !links_changed && let Some(seen) = &self.country_reads {
                            super::territory::CountryReads::from_slots(
                                frame,
                                rows,
                                Arc::clone(seen.slots()),
                            )
                        } else {
                            super::territory::CountryReads::new(frame, rows)
                        });
                    self.country_rows.clear();
                    let slots = self.country_reads.as_ref().unwrap().slots();
                    for (index, row) in rows.iter().enumerate() {
                        if slots.get(&row.id) == Some(&row.canonical_slot) {
                            self.country_rows
                                .entry(row.canonical_slot)
                                .or_default()
                                .push((index, row.id, row.tag.clone()));
                        }
                    }
                }
                !same
            } else {
                false
            };
        if links_changed {
            self.owners.clear();
            for link in links {
                self.owners
                    .entry(link.canonical_slot)
                    .or_default()
                    .push(link.id);
            }
        }
        let slots = Arc::clone(self.country_reads.as_ref().unwrap().slots());
        let mut dirty = BTreeSet::new();
        if forced || links_changed || changes.rows_changed[TableId::ResearchInputs.index()] {
            if let Some((changed, deleted)) = changed_rows(TableId::ResearchInputs) {
                // Only the slots of the changed rows, each keyed by its country.
                let mut touched = BTreeSet::new();
                for key in changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                {
                    if let Some(&slot) = slots.get(&((key >> 32) as u32)) {
                        touched.insert(slot);
                    }
                }
                for slot in touched {
                    let held = self.owners.get(&slot).is_some_and(|owners| {
                        owners
                            .iter()
                            .any(|&owner| !owned(inputs, owner, |row| row.country).is_empty())
                    });
                    if held {
                        self.input_slots.insert(slot);
                    } else {
                        self.input_slots.remove(&slot);
                    }
                    dirty.insert(slot);
                }
            } else {
                dirty.extend(self.input_slots.iter().copied());
                self.input_slots.clear();
                for input in inputs {
                    if let Some(&slot) = slots.get(&input.country) {
                        self.input_slots.insert(slot);
                    }
                }
                dirty.extend(self.input_slots.iter().copied());
            }
        }
        if forced || links_changed || changes.rows_changed[TableId::SliceReads.index()] {
            let table = TableId::ResearchInputs.index() as u32;
            if let Some((changed, deleted)) = changed_rows(TableId::SliceReads) {
                // Only the slots whose owners' read rows changed; each key is
                // its table and owner.
                let mut touched = BTreeSet::new();
                for key in changed
                    .iter()
                    .map(|row| row.key)
                    .chain(deleted.iter().copied())
                    .filter(|key| (key >> 32) as u32 == table)
                {
                    if let Some(&slot) = slots.get(&(key as u32)) {
                        touched.insert(slot);
                    }
                }
                for slot in touched {
                    // The slot's dates as the whole table gives them: by owner.
                    let dates: Vec<_> = self
                        .owners
                        .get(&slot)
                        .into_iter()
                        .flatten()
                        .filter_map(|&owner| {
                            read_hour(TableId::ResearchInputs, owner).map(|read| (owner, read))
                        })
                        .collect();
                    if let Some(&(_, read)) = dates.iter().max_by_key(|(_, read)| *read)
                        && read > frame.hour
                    {
                        self.future_read = self.future_read.max(Some(read));
                    }
                    let dates = (!dates.is_empty()).then_some(dates);
                    if self.input_dates.get(&slot) != dates.as_ref() {
                        dirty.insert(slot);
                        match dates {
                            Some(dates) => {
                                self.input_dates.insert(slot, dates);
                            }
                            None => {
                                self.input_dates.remove(&slot);
                            }
                        }
                    }
                }
            } else {
                let first = reads.partition_point(|row| row.table < table);
                let count = reads[first..].partition_point(|row| row.table == table);
                let mut dates = BTreeMap::<u32, Vec<(u32, i32)>>::new();
                for row in &reads[first..first + count] {
                    if let Some(&slot) = slots.get(&row.owner) {
                        dates
                            .entry(slot)
                            .or_default()
                            .push((row.owner, row.sample_hour));
                        if row.sample_hour > frame.hour {
                            self.future_read = self.future_read.max(Some(row.sample_hour));
                        }
                    }
                }
                for (&slot, values) in &dates {
                    if self.input_dates.get(&slot) != Some(values) {
                        dirty.insert(slot);
                    }
                }
                dirty.extend(
                    self.input_dates
                        .keys()
                        .filter(|slot| !dates.contains_key(*slot))
                        .copied(),
                );
                self.input_dates = dates;
            }
        }
        if countries_changed || !deferred {
            dirty.extend(self.country_rows.keys().copied());
        }
        self.ready = true;
        self.indexed = Some(frame.hour);
        Some((dirty, countries_changed))
    }

    /// The country step of an hour whose indexes are refreshed: holders
    /// whose identity changed end their carries, and each country of the
    /// `dirty` slots and of the slots observed again from the hour before
    /// takes its read, carries the last one or ends its carry. It reads the
    /// hour's research rows and read hours through `reads` alone, and the
    /// indexes through `indexes`, whose holders' slots it is given.
    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        hour: i32,
        previous: Option<i32>,
        mut dirty: BTreeSet<u32>,
        countries_changed: bool,
        indexes: Indexes<'_>,
        reads: &dyn Reads,
        selected_tag: Option<&str>,
        deferred: bool,
    ) -> io::Result<()> {
        let Indexes {
            slots,
            rows: country_rows,
            owners,
            dates: input_dates,
        } = indexes;
        let slots = slots.ok_or_else(|| io::Error::other("Research holders are missing"))?;
        if countries_changed && !self.deadlines.is_empty() {
            let holders: BTreeSet<(&[u8], u32)> = country_rows
                .values()
                .flatten()
                .map(|(_, id, tag)| (tag.as_slice(), *id))
                .collect();
            let held: Vec<_> = self
                .deadlines
                .iter()
                .filter_map(|(_, tag)| {
                    let country = &self.countries[tag];
                    let same = country.identity.is_some_and(|(prior, slot)| {
                        holders.contains(&(tag.as_bytes(), prior))
                            && slots.get(&prior) == Some(&slot)
                    });
                    (!same).then_some(tag.clone())
                })
                .collect();
            for tag in held {
                self.close_country(&tag, previous);
            }
        }
        dirty.extend(std::mem::take(&mut self.recheck));
        let mut selected: Vec<_> = dirty
            .iter()
            .flat_map(|slot| {
                country_rows
                    .get(slot)
                    .into_iter()
                    .flatten()
                    .map(move |row| (*slot, row))
            })
            .collect();
        selected.sort_by_key(|(_, (index, _, _))| *index);
        for (canonical, (_, country_id, tag_bytes)) in selected {
            if selected_tag.is_some_and(|selected| selected.as_bytes() != tag_bytes) {
                continue;
            }
            if !self.lensed.is_empty()
                && std::str::from_utf8(tag_bytes).is_ok_and(|tag| self.lensed.contains(tag))
            {
                continue;
            }
            // Ascending ids, as CountryLinks orders them: each table keyed
            // country first holds the slot's rows in this order.
            let owners = owners
                .get(&canonical)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let fresh = input_dates
                .get(&canonical)
                .is_some_and(|reads| reads.iter().any(|(_, read)| *read == hour));
            if !fresh {
                if let Ok(tag) = std::str::from_utf8(tag_bytes)
                    && let Some(state) = self.countries.get_mut(tag)
                {
                    if let Some(through) = previous {
                        state.materialize(through);
                    }
                    let mut held = owners
                        .iter()
                        .filter_map(|&owner| reads.read_hour(canonical, owner));
                    let first = held.next();
                    let proof = first.is_some()
                        && first == state.last_read
                        && held.all(|read| Some(read) == first);
                    if proof && state.carries((*country_id, canonical), hour) {
                        if deferred {
                            state.carry_until = state.last_read.and_then(|read| {
                                read.checked_add(replay_data::cadence::STAGGER_PERIOD - 1)
                            });
                        } else {
                            state.carry((*country_id, canonical), hour);
                        }
                    } else {
                        if let Some(due) = state.carry_until.and_then(|until| until.checked_add(1))
                        {
                            self.deadlines.remove(&(due, tag.to_owned()));
                        }
                        state.carry_until = None;
                    }
                    self.observed.insert(tag.to_owned());
                }
                continue;
            }
            let Ok(tag) = std::str::from_utf8(tag_bytes) else {
                continue;
            };
            if tag.is_empty() || tag.len() > 512 {
                continue;
            }
            // Rows keep the hour they were read at. Observe a read only in
            // the frame of its own hour; carried rows add nothing.
            let read_rows = reads.fresh(canonical, owners)?;
            let Some(read) = read_rows.read else {
                if let Some(state) = self.countries.get_mut(tag) {
                    if let Some(through) = previous {
                        state.materialize(through);
                    }
                    if let Some(due) = state.carry_until.and_then(|until| until.checked_add(1)) {
                        self.deadlines.remove(&(due, tag.to_owned()));
                    }
                    state.carry_until = None;
                    self.observed.insert(tag.to_owned());
                }
                continue;
            };
            let state = self.countries.entry(tag.to_owned()).or_default();
            if let Some(through) = previous {
                state.materialize(through);
            }
            if state.last_read.is_some_and(|last| read <= last) {
                continue;
            }
            // Continuous with the previous read only when no recorded hour
            // between them was unobservable and no scheduled read of this
            // owner was missed (each owner is read at least once a day).
            state.read_after = state.last_read.filter(|&last| {
                self.last_bad.is_none_or(|bad| bad <= last)
                    && i64::from(read) - i64::from(last)
                        <= i64::from(replay_data::cadence::STAGGER_PERIOD)
            });
            let identity = (*country_id, canonical);
            let inventory = read_rows.inventory.as_ref();
            match &read_rows.joins {
                Joins::Rows { bonuses, modifiers } => state.observe_inputs(
                    identity,
                    read,
                    &read_rows.inputs,
                    inventory,
                    bonuses.as_deref(),
                    modifiers.as_deref(),
                ),
                Joins::Attributed(attributions) if valid_inputs(&read_rows.inputs) => {
                    if attributions.len() != read_rows.inputs.len() {
                        return Err(io::Error::other(
                            "A fresh research read's attributions were not kept",
                        ));
                    }
                    state.observe_attributed(
                        identity,
                        read,
                        &read_rows.inputs,
                        inventory,
                        attributions,
                    );
                }
                Joins::Attributed(_) => {}
            }
            if let Some(due) = state.carry_until.and_then(|until| until.checked_add(1)) {
                self.deadlines.remove(&(due, tag.to_owned()));
            }
            state.last_read = Some(read);
            state.carry_until = state
                .history
                .coverage
                .last()
                .filter(|span| span.last_hour == read)
                .and_then(|_| read.checked_add(replay_data::cadence::STAGGER_PERIOD - 1));
            if let Some(due) = state.carry_until.and_then(|until| until.checked_add(1)) {
                self.deadlines.insert((due, tag.to_owned()));
            }
            // A carried read the owners' read hours do not all prove ends at
            // the next hour, whose rows need not change.
            if state.carry_until.is_some() {
                let mut held = owners
                    .iter()
                    .filter_map(|&owner| reads.read_hour(canonical, owner));
                if !(held.next() == Some(read) && held.all(|hour| hour == read)) {
                    self.recheck.insert(canonical);
                }
            }
            self.observed.insert(tag.to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
fn research_branch_selected(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, (ResearchHistory, usize)>> {
    check_cancel(cancelled)?;
    let selected = [
        replay_data::rows::TableId::Campaign,
        replay_data::rows::TableId::Countries,
        replay_data::rows::TableId::CountryLinks,
        replay_data::rows::TableId::ResearchTechnologies,
        replay_data::rows::TableId::ResearchInputs,
        replay_data::rows::TableId::ResearchAppliedBonuses,
        replay_data::rows::TableId::ResearchModifiers,
        replay_data::rows::TableId::SliceReads,
    ];
    let mut scan = Scan::default();
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|b| b.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        scan.reject(frame.hour);
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        scan.reject(frame.hour);
                        break;
                    }
                    scan.observe_selected(decoded, selected_tag)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan
        .countries
        .into_iter()
        .map(|(tag, mut country)| {
            country.history.slots.sort_by_key(|s| s.index);
            (tag, (country.history, country.unjoined_reads))
        })
        .collect())
}

#[cfg(test)]
mod columnar_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_fixtures;
#[cfg(test)]
pub(crate) mod tests;

pub struct Research;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReply {
    pub history: ResearchHistory,
    pub artwork: Vec<Artwork>,
    pub folders: Vec<crate::ResearchFolder>,
    pub technology_folders: Vec<crate::ResearchTechnologyFolder>,
    pub warnings: Vec<String>,
    // Separate recorded-definition text is needed when artwork has no single recorded definition.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub definitions: Option<Vec<crate::ResearchDefinitionText>>,
}

#[derive(Default)]
pub struct State {
    artwork: Option<artwork::Context>,
    #[cfg(test)]
    history: ResearchCache,
}

impl Product for Research {
    const KEY: &'static str = "research";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = ResearchReply;
    const SERVED: Option<&'static crate::prepared::served::Plan> = Some(&served::PLAN);

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        context.content()?;
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let (history, unjoined) =
            state
                .history
                .get(&source.replay(), params.branch(), &params.tag, cancelled)?;
        present(source, context, &mut state, history, unjoined, cancelled)
    }
}

fn present(
    source: &Source,
    context: &LoadContext,
    state: &mut State,
    mut history: ResearchHistory,
    unjoined: usize,
    cancelled: &AtomicBool,
) -> io::Result<ResearchReply> {
    let art = match context.content().and_then(|content| {
        artwork::ensure(
            &mut state.artwork,
            source,
            content,
            artwork::Kind::Technology,
        )
    }) {
        Ok(art) => art,
        Err(error) => {
            state.artwork = None;
            return Ok(ResearchReply {
                history,
                artwork: Vec::new(),
                folders: Vec::new(),
                technology_folders: Vec::new(),
                warnings: vec![format!("Research content unavailable: {error}")],
                definitions: None,
            });
        }
    };
    let mut indices: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    let mut unrecorded: BTreeSet<&str> = BTreeSet::new();
    for slot in &history.slots {
        for entry in &slot.entries {
            let recorded = indices.entry(entry.token.as_str()).or_default();
            match entry.database_index {
                Some(index) => {
                    recorded.insert(index);
                }
                None => {
                    unrecorded.insert(entry.token.as_str());
                }
            }
        }
        for state in &slot.states {
            if let Some(token) = state.token.as_deref() {
                indices.entry(token).or_default();
                if state.entry_first_observed_hour.is_none() {
                    unrecorded.insert(token);
                }
            }
        }
    }
    let single = |token: &str, recorded: &BTreeSet<u32>| {
        (recorded.len() == 1 && !unrecorded.contains(token))
            .then(|| recorded.first().copied())
            .flatten()
    };
    let mut definitions = Vec::new();
    let mut placed_definitions = BTreeSet::new();
    for (token, recorded) in &indices {
        if single(*token, recorded).is_none() {
            for &index in recorded {
                crate::metadata::check_cancel(cancelled)?;
                let (name, description) = art.definition_text(token, index);
                definitions.push(crate::ResearchDefinitionText {
                    token: (*token).to_owned(),
                    database_index: index,
                    name,
                    description,
                });
                placed_definitions.insert((*token, index));
            }
        }
    }
    let tokens: BTreeMap<&str, Option<u32>> = indices
        .iter()
        .map(|(token, recorded)| (*token, single(*token, recorded)))
        .collect();
    let folder_tokens: BTreeMap<&str, Option<u32>> = tokens
        .iter()
        .map(|(&token, &at)| (token, at))
        .filter(|&(token, at)| {
            at.is_some() || unrecorded.contains(token) || indices[token].is_empty()
        })
        .collect();
    let (folders, technology_folders, folder_warnings) =
        art.research_folders(&folder_tokens, &placed_definitions);
    let (artwork, mut warnings) = art.recorded_text_catalogue(tokens, cancelled)?;
    warnings.extend(folder_warnings);
    for slot in &mut history.slots {
        for state in &mut slot.states {
            if let Some(modifiers) = state.details.as_mut().and_then(|d| d.modifiers.as_mut()) {
                for source in modifiers.components.iter_mut().flat_map(|c| &mut c.sources) {
                    if let Some(token) = &source.token {
                        source.name = art.localise(token);
                    }
                }
            }
            for bonus in state
                .details
                .as_mut()
                .and_then(|d| d.bonuses.as_mut())
                .into_iter()
                .flatten()
            {
                if let Some(token) = &bonus.token {
                    bonus.name = art.localise(token);
                }
            }
        }
    }
    if unjoined > 0 {
        warnings.push(unjoined_warning(unjoined));
    }
    warnings.push("Research dates are observation bounds. Elapsed durations use observed assignments across slots; initial progress and gaps can leave the full duration unknown. Base duration is derived separately from recorded cost operands. Technologies without observed slot assignments are not assigned to a column.".into());
    // Names left out for a localisation refusal, by reason and key.
    warnings.extend(art.refusal_lines());
    Ok(ResearchReply {
        history,
        artwork,
        folders,
        technology_folders,
        warnings,
        definitions: (!definitions.is_empty()).then_some(definitions),
    })
}
