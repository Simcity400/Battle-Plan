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
use super::{CountryParams, LoadContext, Product, artwork};
use crate::{
    Artwork, ResearchBonus, ResearchCoverage, ResearchDetails, ResearchEntry, ResearchHistory,
    ResearchOutcome, ResearchProgress, ResearchSlotHistory, ResearchSlotState, Source,
    metadata::validate_campaign,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{ResearchSlotRow, ResearchTechnologyRow, tables},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

mod modifiers;

const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub struct ResearchCache {
    /// Branch, tag, history, retained bytes and the slot reads not joined.
    countries: VecDeque<(u64, String, ResearchHistory, usize, usize)>,
}

impl ResearchCache {
    pub fn get(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<(ResearchHistory, usize)> {
        check_cancel(cancelled)?;
        if let Some(i) = self
            .countries
            .iter()
            .position(|(id, country, ..)| *id == branch && country == tag)
        {
            let entry = self.countries.remove(i).unwrap();
            self.countries.push_back(entry);
        } else {
            let (history, unjoined) =
                research_branch_selected(replay, branch, Some(tag), cancelled)?
                    .remove(tag)
                    .ok_or_else(|| {
                        io::Error::other("No recorded research history for this country")
                    })?;
            let bytes = history_bytes(&history);
            while !self.countries.is_empty()
                && (self.countries.len() >= 4
                    || self
                        .countries
                        .iter()
                        .map(|(_, _, _, b, _)| b)
                        .sum::<usize>()
                        + bytes
                        > MAX_BYTES)
            {
                self.countries.pop_front();
            }
            self.countries
                .push_back((branch, tag.to_owned(), history, bytes, unjoined));
        }
        let (_, _, history, _, unjoined) = self.countries.back().unwrap();
        Ok((history.clone(), *unjoined))
    }
}

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

fn history_bytes(history: &ResearchHistory) -> usize {
    128 + history.coverage.capacity() * size_of::<ResearchCoverage>()
        + history.slots.capacity() * size_of::<ResearchSlotHistory>()
        + history
            .slots
            .iter()
            .map(|s| {
                s.availability.capacity() * size_of::<ResearchCoverage>()
                    + s.progress.capacity() * size_of::<ResearchProgress>()
                    + s.progress
                        .iter()
                        .map(|p| {
                            p.progress_raw.capacity()
                                + p.base_days_raw.as_ref().map_or(0, String::capacity)
                        })
                        .sum::<usize>()
                    + s.entries.capacity() * size_of::<ResearchEntry>()
                    + s.entries.iter().map(|e| e.token.capacity()).sum::<usize>()
                    + s.states.capacity() * size_of::<ResearchSlotState>()
                    + s.states
                        .iter()
                        .map(|state| {
                            detail_bytes(state.details.as_ref())
                                + state.token.as_ref().map_or(0, String::capacity)
                                + state.speed_multiplier_raw.capacity()
                                + state.banked_days_raw.capacity()
                                + state.applied_days_raw.capacity()
                                + state
                                    .ahead_reduction_years_raw
                                    .as_ref()
                                    .map_or(0, String::capacity)
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
}

#[derive(Default)]
struct Country {
    identity: Option<(u32, u32)>,
    continuity_start_hour: i32,
    history: ResearchHistory,
    // Active assignments retain the template index, level and researched membership.
    active: BTreeMap<u32, (u32, i32, bool)>,
    // Latest assignment occurrence per slot, including completed entries.
    entry_occurrences: Vec<Option<u32>>,
    samples: BTreeMap<u32, VecDeque<(i32, replay::rows::ResearchInputRow)>>,
    pending: Vec<(u32, usize, u32, i32, i32)>,
    /// Append-only retained allocations, excluding the fixed history header.
    retained_bytes: usize,
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

#[derive(Clone, Copy, Default)]
struct SlotMark {
    entries: usize,
    states: usize,
    progress: usize,
}
struct HistoryMark {
    structure: usize,
    /// One mark per slot the history had; a slot added later counts from zero.
    slots: Vec<SlotMark>,
}

fn structure_bytes(history: &ResearchHistory) -> usize {
    history.coverage.capacity() * size_of::<ResearchCoverage>()
        + history.slots.capacity() * size_of::<ResearchSlotHistory>()
        + history
            .slots
            .iter()
            .map(|slot| {
                slot.availability.capacity() * size_of::<ResearchCoverage>()
                    + slot.entries.capacity() * size_of::<ResearchEntry>()
                    + slot.states.capacity() * size_of::<ResearchSlotState>()
                    + slot.progress.capacity() * size_of::<ResearchProgress>()
            })
            .sum::<usize>()
}

fn history_mark(history: &ResearchHistory) -> HistoryMark {
    let slots = history
        .slots
        .iter()
        .map(|slot| SlotMark {
            entries: slot.entries.len(),
            states: slot.states.len(),
            progress: slot.progress.len(),
        })
        .collect();
    HistoryMark {
        structure: structure_bytes(history),
        slots,
    }
}

fn state_payload_bytes(state: &ResearchSlotState) -> usize {
    detail_bytes(state.details.as_ref())
        + state.token.as_ref().map_or(0, String::capacity)
        + state.speed_multiplier_raw.capacity()
        + state.banked_days_raw.capacity()
        + state.applied_days_raw.capacity()
        + state
            .ahead_reduction_years_raw
            .as_ref()
            .map_or(0, String::capacity)
}

fn history_growth(history: &ResearchHistory, before: &HistoryMark) -> usize {
    structure_bytes(history) - before.structure
        + history
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| (slot, before.slots.get(index).copied().unwrap_or_default()))
            .map(|(slot, mark)| {
                slot.entries[mark.entries..]
                    .iter()
                    .map(|e| e.token.capacity())
                    .sum::<usize>()
                    + slot.states[mark.states..]
                        .iter()
                        .map(state_payload_bytes)
                        .sum::<usize>()
                    + slot.progress[mark.progress..]
                        .iter()
                        .map(|p| {
                            p.progress_raw.capacity()
                                + p.base_days_raw.as_ref().map_or(0, String::capacity)
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
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
    fn observe_inputs(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        inputs: &[&replay::rows::ResearchInputRow],
        inventory: Option<&[ResearchTechnologyRow]>,
        bonuses: Option<&[replay::rows::ResearchAppliedBonusRow]>,
        modifier_rows: Option<&[replay::rows::ResearchModifierRow]>,
    ) {
        let before = history_mark(&self.history);
        self.observe_inputs_inner(identity, hour, inputs, inventory, bonuses, modifier_rows);
        self.retained_bytes += history_growth(&self.history, &before);
    }

    /// Whether `hour` can carry the last read: the same identity, the hour
    /// straight after the last known one, and before the next read is due.
    fn carries(&self, identity: (u32, u32), hour: i32) -> bool {
        self.identity == Some(identity)
            && self.last_read.is_some_and(|read| {
                i64::from(hour) - i64::from(read) < i64::from(replay::cadence::STAGGER_PERIOD)
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
        inputs: &[&replay::rows::ResearchInputRow],
        inventory: Option<&[ResearchTechnologyRow]>,
        bonuses: Option<&[replay::rows::ResearchAppliedBonusRow]>,
        modifier_rows: Option<&[replay::rows::ResearchModifierRow]>,
    ) {
        // Any number of slots: content and effects set how many a country has.
        // A recorded occurrence must be a template index. One that was not
        // recorded keeps the read; only the joins that need it are refused.
        if inputs.iter().enumerate().any(|(i, r)| {
            r.index as usize != i
                || r.technology_present > 1
                || (r.technology_present != 0
                    && (matches!(r.database_index, replay::rows::Recorded::Value(index) if index <= 0)
                        || r.token.is_empty()
                        || r.token.len() > 512
                        || std::str::from_utf8(&r.token).is_err()))
        }) {
            return;
        }
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
        let mut technologies = BTreeMap::new();
        // Inventory is sorted by (country, template index). Only former/current
        // assignments need completion evidence; never rescan the full catalogue
        // once per country per hour.
        if let (Some(inventory), Some(owner)) = (inventory, inputs.first().map(|r| r.country)) {
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
                if let Ok(index) = inventory.binary_search_by_key(&(owner, occurrence), |t| {
                    (t.country, t.database_index as u32)
                }) {
                    technologies.insert(occurrence, inventory[index].clone());
                }
            }
        }
        let mut definitions = BTreeMap::new();
        let mut unjoined = BTreeMap::new();
        let mut details = BTreeMap::new();
        self.samples
            .retain(|index, _| inputs.iter().any(|r| r.index == *index));
        for &input in inputs {
            let occurrence = match input.database_index {
                replay::rows::Recorded::Value(index) if input.technology_present != 0 => {
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
                .is_some_and(|(_, last)| same_assignment(last, input));
            if !same {
                samples.clear();
            }
            let modifier_data = modifiers::breakdown(input, modifier_rows);
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
                    .as_ref()
                    .and_then(|m| modifiers::rate(m, input.ahead_reduction_raw, hour))
                    .or(daily_rate);
                daily_rate.and_then(|rate| remaining(input, rate))
            } else {
                None
            };
            samples.push_back((hour, input.clone()));
            while samples.len() > 24 {
                samples.pop_front();
            }
            if let Some(occurrence) = occurrence {
                definitions.insert(
                    occurrence,
                    std::str::from_utf8(&input.token).unwrap().to_owned(),
                );
                technologies.insert(
                    occurrence,
                    ResearchTechnologyRow {
                        country: input.country,
                        database_index: occurrence as i32,
                        applied_bonus_count: input.bonus_count as i32,
                        technology_handle: input.technology_handle,
                        classifier: input.classifier,
                        level: input.level,
                        researched: 0,
                        progress_raw: input.progress_raw,
                        ahead_reduction_years_raw: input.ahead_reduction_raw,
                    },
                );
            }
            let attributed = bonuses.and_then(|bonuses| {
                let matching: Vec<_> = bonuses
                    .iter()
                    .filter(|b| {
                        b.country == input.country
                            && b.index == input.index
                            && b.technology_handle == input.technology_handle
                    })
                    .collect();
                if matching.len() != input.bonus_count as usize
                    || matching.iter().enumerate().any(|(i, b)| {
                        b.ordinal as usize != i || b.resolved > 1 || b.name.len() > 512
                    })
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
                            speed_bonus_raw: (b.resolved != 0)
                                .then(|| b.speed_bonus_raw.to_string()),
                            ahead_reduction_years_raw: (b.resolved != 0)
                                .then(|| b.ahead_reduction_raw.to_string()),
                        })
                        .collect(),
                )
            });
            details.insert(
                input.index,
                ResearchDetails {
                    remaining_days,
                    bonuses: attributed,
                    modifiers: modifier_data,
                },
            );
        }
        let previous_assignments = self.active.clone();
        self.observe(
            identity,
            hour,
            &slots.iter().collect::<Vec<_>>(),
            &technologies.iter().map(|(&h, t)| (h, t)).collect(),
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
                                == replay::rows::Recorded::Value(occurrence as i32)
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
                && !matches!(input.database_index, replay::rows::Recorded::Value(_))
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
            let held_since = unjoined_run_start(&slot.states);
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
        technologies: &BTreeMap<u32, &ResearchTechnologyRow>,
        definitions: &BTreeMap<u32, String>,
        unjoined: &BTreeMap<u32, UnjoinedTechnology>,
        details: Option<&BTreeMap<u32, ResearchDetails>>,
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
                if hour.saturating_sub(ended) > replay::cadence::STAGGER_PERIOD
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
                        || (level == 0 && t.researched != 0)
                        || t.classifier == engine::ctechnology::CLASSIFIER_COMPLETED
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
                        || (!researched && t.researched != 0)
                        || (!transferred
                            && t.classifier == engine::ctechnology::CLASSIFIER_COMPLETED)
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
            let replay::rows::Recorded::Value(occurrence) = row.database_index else {
                continue;
            };
            let occurrence = occurrence as u32;
            let technology = technologies[&occurrence];
            // A completed assignment can remain linked for an observation. Do not
            // invent a new assignment until its recorded state becomes active again.
            if technology.classifier == engine::ctechnology::CLASSIFIER_COMPLETED
                || (technology.researched != 0
                    && technology.classifier != engine::ctechnology::CLASSIFIER_ACTIVE)
            {
                continue;
            }
            slot.entries.push(ResearchEntry {
                token: definitions[&occurrence].clone(),
                technology_handle: row.technology_handle,
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
                (occurrence, technology.level, technology.researched != 0),
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
                replay::rows::Recorded::Value(index) if row.technology_present != 0 => {
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
                        definitions[&occurrence].clone(),
                        technology.classifier,
                        technology.researched != 0,
                        technology.ahead_reduction_years_raw,
                    )
                })
                .or_else(|| {
                    unjoined.get(&row.index).map(|technology| {
                        (
                            technology.token.clone(),
                            technology.classifier,
                            false,
                            technology.ahead_reduction_years_raw,
                        )
                    })
                });
            let entry_first_observed_hour = held.as_ref().and_then(|(token, ..)| {
                slot.entries
                    .last()
                    .filter(|entry| {
                        &entry.token == token
                            && occurrence.is_some()
                            && self.entry_occurrences[row.index as usize] == occurrence
                    })
                    .map(|entry| entry.first_observed_hour)
            });
            // A finished technology still linked to the slot, with no
            // assignment joined to it, is not being researched.
            let held = held.filter(|&(_, classifier, researched, _)| {
                entry_first_observed_hour.is_some()
                    || !(classifier == engine::ctechnology::CLASSIFIER_COMPLETED
                        || (researched && classifier != engine::ctechnology::CLASSIFIER_ACTIVE))
            });
            let (token, ahead_reduction_years_raw) = held.map_or((None, None), |held| {
                (Some(held.0), Some(held.3.to_string()))
            });
            let state = ResearchSlotState {
                first_hour: hour,
                last_hour: hour,
                details: details.and_then(|d| d.get(&row.index)).cloned(),
                entry_first_observed_hour,
                token,
                speed_multiplier_raw: row.research_speed_raw.to_string(),
                banked_days_raw: row.banked_days_raw.to_string(),
                applied_days_raw: row.applied_days_raw.to_string(),
                ahead_reduction_years_raw,
            };
            if let Some(last) = slot
                .states
                .last_mut()
                .filter(|last| continuous && follows(read_after, last.last_hour, hour))
            {
                let mut comparable = state.clone();
                comparable.first_hour = last.first_hour;
                comparable.last_hour = last.last_hour;
                // A steady countdown needs one segment, not one per day. A
                // different rate, bonus, missing sample or assignment breaks it.
                if let (Some(before), Some(now)) = (&last.details, &mut comparable.details)
                    && before.remaining_days.map(|days| {
                        days.saturating_sub(
                            ((i64::from(hour) - i64::from(last.first_hour)) / 24) as u32,
                        )
                    }) == now.remaining_days
                {
                    now.remaining_days = before.remaining_days;
                }
                if *last == comparable {
                    last.last_hour = hour;
                    continue;
                }
            }
            slot.states.push(state);
        }
    }
}

/// Only a recorded occurrence identifies a technology across reads; equal
/// handles and names can belong to repeated occurrences.
/// The first hour of the unbroken run of states, ending with the last, that
/// hold the last state's technology without a joined assignment.
fn unjoined_run_start(states: &[ResearchSlotState]) -> Option<i32> {
    let last = states.last()?;
    let token = last.token.as_ref()?;
    if last.entry_first_observed_hour.is_some() {
        return None;
    }
    let mut start = last.first_hour;
    for pair in states.windows(2).rev() {
        let (before, after) = (&pair[0], &pair[1]);
        if before.token.as_ref() != Some(token)
            || before.entry_first_observed_hour.is_some()
            || before.last_hour.checked_add(1) != Some(after.first_hour)
        {
            break;
        }
        start = before.first_hour;
    }
    Some(start)
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
    placed: &BTreeMap<String, Vec<String>>,
    tokens: impl IntoIterator<Item = &'a str>,
) -> (Vec<crate::ResearchTechnologyFolder>, Vec<String>) {
    let mut undecided: Vec<&str> = Vec::new();
    let mut undefined: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut shown = Vec::new();
    for token in tokens {
        let mut folder = None;
        let mut unknown = false;
        for key in placed.get(token).into_iter().flatten() {
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

fn same_assignment(a: &replay::rows::ResearchInputRow, b: &replay::rows::ResearchInputRow) -> bool {
    a.country == b.country
        && a.technology_present == b.technology_present
        && (a.technology_present == 0
            || matches!(a.database_index, replay::rows::Recorded::Value(_)))
        && a.technology_handle == b.technology_handle
        && a.database_index == b.database_index
        && a.level == b.level
        && a.token == b.token
}

fn base_cost(row: &replay::rows::ResearchInputRow) -> Option<i128> {
    let cost = if row.cost_debug != 0 {
        100_000
    } else {
        i128::from(row.cost_raw).checked_mul(i128::from(row.cost_scale_raw))? / 100_000
    };
    (cost > 0 && cost <= i128::from(i64::MAX)).then_some(cost)
}

fn remaining(row: &replay::rows::ResearchInputRow, rate: i128) -> Option<u32> {
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

fn detail_bytes(details: Option<&ResearchDetails>) -> usize {
    details
        .and_then(|d| d.modifiers.as_ref())
        .map_or(0, modifiers::bytes)
        + details
            .and_then(|d| d.bonuses.as_ref())
            .map_or(0, |bonuses| {
                bonuses.capacity() * size_of::<ResearchBonus>()
                    + bonuses
                        .iter()
                        .map(|b| {
                            [
                                &b.token,
                                &b.name,
                                &b.cost_reduction_raw,
                                &b.speed_bonus_raw,
                                &b.ahead_reduction_years_raw,
                            ]
                            .iter()
                            .map(|s| s.as_ref().map_or(0, String::capacity))
                            .sum::<usize>()
                        })
                        .sum::<usize>()
            })
}

fn check_budget(countries: &BTreeMap<String, Country>) -> io::Result<()> {
    let bytes = countries
        .values()
        .map(|c| {
            128 + c.retained_bytes
                + c.entry_occurrences.capacity() * size_of::<Option<u32>>()
                + c.samples
                    .values()
                    .map(|s| {
                        s.capacity() * size_of::<(i32, replay::rows::ResearchInputRow)>()
                            + s.iter().map(|(_, r)| r.token.capacity()).sum::<usize>()
                    })
                    .sum::<usize>()
                + c.pending.capacity() * size_of::<(u32, usize, u32, i32, i32)>()
        })
        .sum::<usize>();
    if bytes > MAX_BYTES {
        Err(io::Error::other("Research history exceeds memory budget"))
    } else {
        Ok(())
    }
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

/// Every country's history with the count of its slot reads not joined.
#[cfg(test)]
pub fn research_branch_unjoined(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, (ResearchHistory, usize)>> {
    research_branch_selected(replay, branch, None, cancelled)
}

fn research_branch_selected(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, (ResearchHistory, usize)>> {
    check_cancel(cancelled)?;
    let selected = [
        replay::rows::TableId::Campaign,
        replay::rows::TableId::Countries,
        replay::rows::TableId::CountryLinks,
        replay::rows::TableId::ResearchTechnologies,
        replay::rows::TableId::ResearchInputs,
        replay::rows::TableId::ResearchAppliedBonuses,
        replay::rows::TableId::ResearchModifiers,
        replay::rows::TableId::SliceReads,
    ];
    let mut countries: BTreeMap<String, Country> = BTreeMap::new();
    // The latest recorded hour whose research inputs could not be observed,
    // including missing hours. A read after it cannot continue one before it.
    let mut last_bad: Option<i32> = None;
    let mut last_frame: Option<i32> = None;
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
                    if last_frame
                        .is_some_and(|previous| previous.checked_add(1) != Some(frame.hour))
                    {
                        last_bad = Some(frame.hour.saturating_sub(1));
                    }
                    last_frame = Some(frame.hour);
                    let Ok(decoded) = decoder.advance(&frame) else {
                        last_bad = Some(frame.hour);
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        last_bad = Some(frame.hour);
                        break;
                    }
                    let (Ok(rows), Ok(links), Ok(inputs)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::ResearchInputs>(),
                    ) else {
                        last_bad = Some(frame.hour);
                        continue;
                    };
                    let inventory = decoded.complete::<tables::ResearchTechnologies>().ok();
                    let bonuses = decoded.complete::<tables::ResearchAppliedBonuses>().ok();
                    let modifier_rows = decoded.complete::<tables::ResearchModifiers>().ok();
                    // Every sliced row's read hour; without it no read is observable.
                    let Ok(reads) = decoded.complete::<tables::SliceReads>() else {
                        last_bad = Some(frame.hour);
                        continue;
                    };
                    let read_hour = |table, owner| replay::sliced::read_hour(reads, table, owner);
                    // A country is observed only when its inputs were read this hour (the
                    // check below), so find those first rather than joining every country.
                    let inputs_table = replay::rows::TableId::ResearchInputs.index() as u32;
                    let slots_read: HashSet<u32> = reads
                        .iter()
                        .filter(|row| row.table == inputs_table && row.sample_hour == frame.hour)
                        .filter_map(|row| crate::metadata::link_slot(links, row.owner))
                        .collect();
                    // Each country joins only the input rows of its own linked IDs, so
                    // a row whose country has no link is ignored; it never hides the
                    // reads of linked countries in the same frame.
                    for country in rows {
                        // Compare the tag before any per-country work.
                        if selected_tag.is_some_and(|selected| selected.as_bytes() != country.tag) {
                            continue;
                        }
                        let Ok(canonical) = crate::metadata::mapped_slot(country, links) else {
                            continue;
                        };
                        if !slots_read.contains(&canonical) {
                            // Not read this hour: its last read holds while the rows
                            // carried for its linked IDs are still those of that read.
                            if let Ok(tag) = std::str::from_utf8(&country.tag)
                                && let Some(state) = countries.get_mut(tag)
                                && state.carries((country.id, canonical), frame.hour)
                            {
                                let mut held = links
                                    .iter()
                                    .filter(|link| link.canonical_slot == canonical)
                                    .filter_map(|link| {
                                        read_hour(replay::rows::TableId::ResearchInputs, link.id)
                                    });
                                let first = held.next();
                                if first.is_some()
                                    && first == state.last_read
                                    && held.all(|read| Some(read) == first)
                                {
                                    state.carry((country.id, canonical), frame.hour);
                                }
                            }
                            continue;
                        }
                        let Ok(tag) = std::str::from_utf8(&country.tag) else {
                            continue;
                        };
                        if tag.is_empty() || tag.len() > 512 {
                            continue;
                        }
                        let ids: Vec<_> = links
                            .iter()
                            .filter(|link| link.canonical_slot == canonical)
                            .map(|link| link.id)
                            .collect();
                        let inputs: Vec<_> = inputs
                            .iter()
                            .filter(|input| ids.contains(&input.country))
                            .collect();
                        // Rows keep the hour they were read at. Observe a read only in
                        // the frame of its own hour; carried rows add nothing.
                        use replay::rows::TableId;
                        let Some(read) = inputs
                            .first()
                            .and_then(|input| read_hour(TableId::ResearchInputs, input.country))
                        else {
                            continue;
                        };
                        if read != frame.hour
                            || inputs.iter().any(|input| {
                                read_hour(TableId::ResearchInputs, input.country) != Some(read)
                            })
                        {
                            continue;
                        }
                        let state = countries.entry(tag.to_owned()).or_default();
                        if state.last_read.is_some_and(|last| read <= last) {
                            continue;
                        }
                        // Continuous with the previous read only when no recorded hour
                        // between them was unobservable and no scheduled read of this
                        // owner was missed (each owner is read at least once a day).
                        state.read_after = state.last_read.filter(|&last| {
                            last_bad.is_none_or(|bad| bad <= last)
                                && i64::from(read) - i64::from(last)
                                    <= i64::from(replay::cadence::STAGGER_PERIOD)
                        });
                        // Bonuses and modifiers join only when they come from the same read.
                        let bonuses = bonuses.filter(|rows| {
                            rows.iter()
                                .filter(|row| ids.contains(&row.country))
                                .all(|row| {
                                    read_hour(TableId::ResearchAppliedBonuses, row.country)
                                        == Some(read)
                                })
                        });
                        let modifier_rows = modifier_rows.filter(|rows| {
                            rows.iter()
                                .filter(|row| ids.contains(&row.country))
                                .all(|row| {
                                    read_hour(TableId::ResearchModifiers, row.country) == Some(read)
                                })
                        });
                        state.observe_inputs(
                            (country.id, canonical),
                            read,
                            &inputs,
                            inventory,
                            bonuses,
                            modifier_rows,
                        );
                        state.last_read = Some(read);
                    }
                    check_budget(&countries)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(countries
        .into_iter()
        .map(|(tag, mut country)| {
            country.history.slots.sort_by_key(|s| s.index);
            (tag, (country.history, country.unjoined_reads))
        })
        .collect())
}

#[cfg(test)]
mod tests;

pub struct Research;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReply {
    pub history: ResearchHistory,
    pub artwork: Vec<Artwork>,
    pub folders: Vec<crate::ResearchFolder>,
    pub technology_folders: Vec<crate::ResearchTechnologyFolder>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct State {
    artwork: Option<artwork::Context>,
    history: ResearchCache,
}

impl Product for Research {
    const KEY: &'static str = "research";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = ResearchReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let content = context.content()?;
        let state = source.state::<State>();
        let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
        let (mut history, unjoined) =
            state
                .history
                .get(&source.replay(), params.branch(), &params.tag, cancelled)?;
        let art = artwork::ensure(
            &mut state.artwork,
            source,
            content,
            artwork::Kind::Technology,
        )?;
        let tokens: BTreeSet<&str> = history
            .slots
            .iter()
            .flat_map(|s| {
                s.entries
                    .iter()
                    .map(|e| e.token.as_str())
                    .chain(s.states.iter().filter_map(|state| state.token.as_deref()))
            })
            .collect();
        let (folders, technology_folders, folder_warnings) = art.research_folders(&tokens);
        let (artwork, mut warnings) = art.text_catalogue(tokens, cancelled)?;
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
        })
    }
}
