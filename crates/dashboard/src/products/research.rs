//! Slot assignments from observed research tables, with explicit coverage and bounds.
//!
//! Research is read one country slice per hour (see the collector README), so
//! a country's rows are observed at their read hour (SliceReads), usually a day apart,
//! and carried rows are ignored. Consecutive reads with no unobservable hour
//! between them are continuous: a new assignment is bounded by the previous
//! read, unless an hour between them was unobservable or a scheduled read
//! of the country was missed (reads more than a day apart). Assignment,
//! cancellation, completion and slot changes are read within the hour;
//! modifier-only changes appear at the country's next slice read.
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
    collections::{BTreeMap, VecDeque},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

mod modifiers;

const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub struct ResearchCache {
    countries: VecDeque<(u64, String, ResearchHistory, usize)>,
}

impl ResearchCache {
    pub fn get(
        &mut self,
        replay: &Replay,
        branch: u64,
        tag: &str,
        cancelled: &AtomicBool,
    ) -> io::Result<ResearchHistory> {
        check_cancel(cancelled)?;
        if let Some(i) = self
            .countries
            .iter()
            .position(|(id, country, _, _)| *id == branch && country == tag)
        {
            let entry = self.countries.remove(i).unwrap();
            self.countries.push_back(entry);
        } else {
            let history = research_branch_selected(replay, branch, Some(tag), cancelled)?
                .remove(tag)
                .ok_or_else(|| io::Error::other("No recorded research history for this country"))?;
            let bytes = history_bytes(&history);
            while !self.countries.is_empty()
                && (self.countries.len() >= 4
                    || self.countries.iter().map(|(_, _, _, b)| b).sum::<usize>() + bytes
                        > MAX_BYTES)
            {
                self.countries.pop_front();
            }
            self.countries
                .push_back((branch, tag.to_owned(), history, bytes));
        }
        Ok(self.countries.back().unwrap().2.clone())
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
    // Each active assignment retains its exact technology identity, even when names coincide.
    active: BTreeMap<u32, (u32, i32, bool)>,
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
    slots: [SlotMark; 64],
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
    let mut slots = [SlotMark::default(); 64];
    for (mark, slot) in slots.iter_mut().zip(&history.slots) {
        *mark = SlotMark {
            entries: slot.entries.len(),
            states: slot.states.len(),
            progress: slot.progress.len(),
        };
    }
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
            .zip(&before.slots)
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

    fn observe_inputs_inner(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        inputs: &[&replay::rows::ResearchInputRow],
        inventory: Option<&[ResearchTechnologyRow]>,
        bonuses: Option<&[replay::rows::ResearchAppliedBonusRow]>,
        modifier_rows: Option<&[replay::rows::ResearchModifierRow]>,
    ) {
        if inputs.len() > 64
            || inputs.iter().enumerate().any(|(i, r)| {
                r.index as usize != i
                    || r.technology_present > 1
                    || (r.technology_present != 0
                        && (r.token.is_empty()
                            || r.token.len() > 512
                            || std::str::from_utf8(&r.token).is_err()))
            })
        {
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
        // Inventory is already sorted by (country, handle). Only former/current
        // assignments need completion evidence; never rescan the full catalogue
        // once per country per hour.
        if let (Some(inventory), Some(owner)) = (inventory, inputs.first().map(|r| r.country)) {
            for handle in self
                .active
                .values()
                .map(|&(handle, _, _)| handle)
                .chain(self.pending.iter().map(|&(_, _, handle, _, _)| handle))
            {
                if let Ok(index) = inventory
                    .binary_search_by_key(&(owner, handle), |t| (t.country, t.technology_handle))
                {
                    technologies.insert(handle, inventory[index].clone());
                }
            }
        }
        let mut definitions = BTreeMap::new();
        let mut details = BTreeMap::new();
        self.samples
            .retain(|index, _| inputs.iter().any(|r| r.index == *index));
        for &input in inputs {
            slots.push(ResearchSlotRow {
                country: input.country,
                index: input.index,
                technology_handle: input.technology_handle,
                technology_present: input.technology_present,
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
                        // Use it only for a witnessed new assignment's first day.
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
                        (continuous && (!same || fresh))
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
            if input.technology_present != 0 {
                definitions.insert(
                    input.technology_handle,
                    std::str::from_utf8(&input.token).unwrap().to_owned(),
                );
                technologies.insert(
                    input.technology_handle,
                    ResearchTechnologyRow {
                        country: input.country,
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
        self.observe(
            identity,
            hour,
            &slots.iter().collect::<Vec<_>>(),
            &technologies.iter().map(|(&h, t)| (h, t)).collect(),
            &definitions,
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
            }) else {
                continue;
            };
            let sample = ResearchProgress {
                first_hour: hour,
                last_hour: hour,
                entry_first_observed_hour: entry.first_observed_hour,
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
    }

    fn observe(
        &mut self,
        identity: (u32, u32),
        hour: i32,
        slots: &[&ResearchSlotRow],
        technologies: &BTreeMap<u32, &ResearchTechnologyRow>,
        definitions: &BTreeMap<u32, String>,
        details: Option<&BTreeMap<u32, ResearchDetails>>,
    ) {
        let previous = self.history.coverage.last().map(|c| c.last_hour);
        let read_after = self.read_after;
        let continuous = previous.is_some_and(|p| follows(read_after, p, hour))
            && self.identity == Some(identity);
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
        }
        self.pending
            .retain(|&(index, entry_index, handle, level, ended)| {
                if hour.saturating_sub(ended) > replay::cadence::STAGGER_PERIOD
                    || slots.iter().any(|s| {
                        s.technology_present != 0
                            && s.technology_handle == handle
                            && technologies.get(&handle).is_some_and(|t| t.level == level)
                    })
                {
                    return false;
                }
                if technologies.get(&handle).is_some_and(|t| {
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
            if let Some(&(handle, level, researched)) = self.active.get(&slot.index) {
                let technology = technologies.get(&handle);
                let transferred = slots.iter().any(|s| {
                    s.index != slot.index
                        && s.technology_present != 0
                        && s.technology_handle == handle
                });
                let completed = technology.is_some_and(|t| {
                    t.level > level
                        || (!researched && t.researched != 0)
                        || (!transferred
                            && t.classifier == engine::ctechnology::CLASSIFIER_COMPLETED)
                });
                let same = current
                    .is_some_and(|s| s.technology_present != 0 && s.technology_handle == handle);
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
                            handle,
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
            let technology = technologies[&row.technology_handle];
            // A completed assignment can remain linked for an observation. Do not
            // invent a new assignment until its recorded state becomes active again.
            if technology.classifier == engine::ctechnology::CLASSIFIER_COMPLETED
                || (technology.researched != 0
                    && technology.classifier != engine::ctechnology::CLASSIFIER_ACTIVE)
            {
                continue;
            }
            slot.entries.push(ResearchEntry {
                token: definitions[&row.technology_handle].clone(),
                technology_handle: row.technology_handle,
                continuity_start_hour: self.continuity_start_hour,
                level: technology.level,
                first_observed_hour: hour,
                started_after_hour: continuous.then_some(previous).flatten(),
                last_observed_hour: hour,
                ended_by_hour: None,
                outcome_observed_hour: None,
                outcome: ResearchOutcome::Active,
            });
            self.active.insert(
                row.index,
                (
                    row.technology_handle,
                    technology.level,
                    technology.researched != 0,
                ),
            );
        }
        for row in slots {
            let slot = self
                .history
                .slots
                .iter_mut()
                .find(|slot| slot.index == row.index)
                .unwrap();
            let technology =
                (row.technology_present != 0).then(|| technologies[&row.technology_handle]);
            let token = technology.map(|_| definitions[&row.technology_handle].clone());
            let state = ResearchSlotState {
                first_hour: hour,
                last_hour: hour,
                details: details.and_then(|d| d.get(&row.index)).cloned(),
                entry_first_observed_hour: slot
                    .entries
                    .last()
                    .filter(|entry| Some(&entry.token) == token.as_ref())
                    .map(|entry| entry.first_observed_hour),
                token,
                speed_multiplier_raw: row.research_speed_raw.to_string(),
                banked_days_raw: row.banked_days_raw.to_string(),
                applied_days_raw: row.applied_days_raw.to_string(),
                ahead_reduction_years_raw: technology
                    .map(|t| t.ahead_reduction_years_raw.to_string()),
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

fn same_assignment(a: &replay::rows::ResearchInputRow, b: &replay::rows::ResearchInputRow) -> bool {
    a.country == b.country
        && a.technology_present == b.technology_present
        && a.technology_handle == b.technology_handle
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
    research_branch_selected(replay, branch, None, cancelled)
}

fn research_branch_selected(
    replay: &Replay,
    branch: u64,
    selected_tag: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<BTreeMap<String, ResearchHistory>> {
    check_cancel(cancelled)?;
    let selected = [
        replay::rows::TableId::Campaign,
        replay::rows::TableId::Countries,
        replay::rows::TableId::CountryLinks,
        replay::rows::TableId::ResearchSlots,
        replay::rows::TableId::ResearchTechnologies,
        replay::rows::TableId::TechnologyDefinitions,
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
    for block in replay.index.iter().filter(|b| b.branch == branch) {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            check_cancel(cancelled)?;
            if last_frame.is_some_and(|previous| previous.checked_add(1) != Some(frame.hour)) {
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
                            read_hour(TableId::ResearchAppliedBonuses, row.country) == Some(read)
                        })
                });
                let modifier_rows = modifier_rows.filter(|rows| {
                    rows.iter()
                        .filter(|row| ids.contains(&row.country))
                        .all(|row| read_hour(TableId::ResearchModifiers, row.country) == Some(read))
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
    Ok(countries
        .into_iter()
        .map(|(tag, mut country)| {
            country.history.slots.sort_by_key(|s| s.index);
            (tag, country.history)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{ResearchAppliedBonusRow, ResearchInputRow};

    fn input(handle: u32, progress: i64) -> ResearchInputRow {
        ResearchInputRow {
            country: 11,
            index: 0,
            technology_handle: handle,
            technology_present: 1,
            token: format!("tech_{handle}").into_bytes(),
            classifier: 5,
            progress_raw: progress,
            cost_raw: 10_000_000,
            cost_scale_raw: 100_000,
            research_speed_raw: 100_000,
            ..Default::default()
        }
    }
    fn observe(country: &mut Country, hour: i32, row: &ResearchInputRow) {
        country.observe_inputs((7, 7), hour, &[row], None, Some(&[]), None);
    }
    fn at(country: &Country, hour: i32) -> Option<u32> {
        let s = country.history.slots[0]
            .states
            .iter()
            .find(|s| s.first_hour <= hour && hour <= s.last_hour)?;
        s.details
            .as_ref()?
            .remaining_days
            .map(|days| days.saturating_sub(((hour - s.first_hour) / 24) as u32))
    }

    #[test]
    fn countdown_uses_only_past_progress_and_compresses_daily_ticks() {
        let mut country = Country::default();
        for hour in 0..=240 {
            let row = input(1, i64::from(hour / 24) * 100_000);
            observe(&mut country, hour, &row);
        }
        assert_eq!(at(&country, 0), None); // Initial assignment's multiplier may be stale.
        assert_eq!(at(&country, 23), None);
        assert_eq!(at(&country, 24), Some(99));
        assert_eq!(at(&country, 239), Some(91));
        assert_eq!(at(&country, 240), Some(90));
        assert_eq!(country.history.slots[0].states.len(), 2);
        assert_eq!(country.samples[&0].len(), 24);
        assert_eq!(remaining(&input(1, 0), 0), None);
        assert_eq!(remaining(&input(1, 10_000_001), 100_000), Some(0));
    }

    #[test]
    fn progress_compresses_separately_from_modifier_and_countdown_states() {
        let mut country = Country::default();
        for hour in 0..=48 {
            observe(
                &mut country,
                hour,
                &input(1, i64::from(hour / 24) * 100_000),
            );
        }
        let slot = &country.history.slots[0];
        assert_eq!(slot.progress.len(), 3);
        assert_eq!(slot.progress[0].first_hour, 0);
        assert_eq!(slot.progress[0].last_hour, 23);
        assert_eq!(slot.progress[1].progress_raw, "100000");
        assert_eq!(slot.progress[2].base_days_raw.as_deref(), Some("10000000"));
        assert_eq!(slot.states.len(), 2);
        observe(&mut country, 50, &input(1, 200_000));
        assert_eq!(
            country.history.slots[0]
                .progress
                .last()
                .unwrap()
                .entry_first_observed_hour,
            50
        );
        observe(&mut country, 51, &input(2, 0));
        assert_eq!(
            country.history.slots[0]
                .progress
                .last()
                .unwrap()
                .entry_first_observed_hour,
            51
        );
    }

    #[test]
    fn base_duration_uses_recorded_cost_operands_and_rejects_invalid_costs() {
        let mut row = input(1, 0);
        row.cost_scale_raw = 150_000;
        assert_eq!(base_cost(&row), Some(15_000_000));
        row.cost_raw = -1;
        assert_eq!(base_cost(&row), None);
        row.cost_debug = 1;
        assert_eq!(base_cost(&row), Some(100_000));
        row.cost_debug = 0;
        row.cost_raw = i64::MAX;
        row.cost_scale_raw = i64::MAX;
        assert_eq!(base_cost(&row), None);
    }

    #[test]
    fn transfers_complete_only_in_the_finishing_slot_and_preserve_technology_identity() {
        let mut country = Country::default();
        let first = input(1, 0);
        let idle = ResearchInputRow {
            index: 1,
            technology_present: 0,
            ..first.clone()
        };
        country.observe_inputs((7, 7), 0, &[&first, &idle], None, Some(&[]), None);
        let vacated = ResearchInputRow {
            technology_present: 0,
            ..first.clone()
        };
        let moved = ResearchInputRow {
            index: 1,
            progress_raw: 3_000_000,
            applied_days_raw: 3_000_000,
            ..first
        };
        country.observe_inputs((7, 7), 1, &[&vacated, &moved], None, Some(&[]), None);
        assert_eq!(
            country.history.slots[0].entries[0].outcome,
            ResearchOutcome::Switched
        );
        assert!(country.pending.is_empty());
        assert_eq!(country.history.slots[1].entries[0].technology_handle, 1);
        assert_eq!(country.history.slots[1].entries[0].continuity_start_hour, 0);
        // Completion may reset progress and advance the repeatable technology's
        // level. That zero must not become the previous level's final progress.
        let completed = ResearchInputRow {
            level: 1,
            classifier: 4,
            progress_raw: 0,
            ..moved.clone()
        };
        country.observe_inputs((7, 7), 2, &[&vacated, &completed], None, Some(&[]), None);
        assert_eq!(
            country.history.slots[1].entries[0].outcome,
            ResearchOutcome::Completed
        );
        assert_eq!(
            country.history.slots[0].entries[0].outcome,
            ResearchOutcome::Switched
        );
        assert_eq!(
            country.history.slots[1].progress.last().unwrap().last_hour,
            1
        );
        country.observe_inputs((7, 7), 4, &[&vacated, &moved], None, Some(&[]), None);
        assert_eq!(
            country.history.slots[1]
                .entries
                .last()
                .unwrap()
                .continuity_start_hour,
            4
        );
    }

    #[test]
    fn resuming_in_the_same_slot_cannot_complete_the_cancelled_assignment() {
        let mut country = Country::default();
        observe(&mut country, 0, &input(1, 0));
        let inventory = [ResearchTechnologyRow {
            country: 11,
            technology_handle: 1,
            classifier: 5,
            ..Default::default()
        }];
        country.observe_inputs(
            (7, 7),
            1,
            &[&input(2, 0)],
            Some(&inventory),
            Some(&[]),
            None,
        );
        assert_eq!(country.pending.len(), 1);
        observe(&mut country, 2, &input(1, 100_000));
        let completed = ResearchInputRow {
            level: 1,
            classifier: 4,
            ..input(1, 0)
        };
        observe(&mut country, 3, &completed);
        let entries = &country.history.slots[0].entries;
        assert_eq!(entries[0].outcome, ResearchOutcome::Switched);
        assert_eq!(entries[2].outcome, ResearchOutcome::Completed);
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.outcome == ResearchOutcome::Completed)
                .count(),
            1
        );
    }

    #[test]
    fn a_new_level_in_another_slot_does_not_hide_the_previous_level_completion() {
        let mut country = Country::default();
        let first = ResearchInputRow {
            level: 1,
            ..input(1, 0)
        };
        let idle = ResearchInputRow {
            index: 1,
            technology_present: 0,
            ..first.clone()
        };
        country.observe_inputs((7, 7), 0, &[&first, &idle], None, Some(&[]), None);
        let vacated = ResearchInputRow {
            technology_present: 0,
            ..first.clone()
        };
        let next = ResearchInputRow {
            index: 1,
            level: 2,
            ..first
        };
        country.observe_inputs((7, 7), 1, &[&vacated, &next], None, Some(&[]), None);
        assert_eq!(
            country.history.slots[0].entries[0].outcome,
            ResearchOutcome::Completed
        );
        assert_eq!(
            country.history.slots[1].entries[0].outcome,
            ResearchOutcome::Active
        );
        assert_eq!(country.history.slots[1].entries[0].level, 2);
    }

    #[test]
    fn incremental_history_budget_matches_full_accounting_over_a_campaign() {
        let mut country = Country::default();
        for hour in 0..24 * 365 * 3 {
            if hour % 997 == 996 {
                continue;
            }
            let count = 2 + (hour / (24 * 365)) as u32;
            let rows: Vec<_> = (0..count)
                .map(|index| ResearchInputRow {
                    index,
                    banked_days_raw: i64::from(hour % 31) * 100_000,
                    ..input(
                        1 + index + (hour / 720) as u32 * 4,
                        i64::from(hour % 720) * 10_000,
                    )
                })
                .collect();
            country.observe_inputs(
                (7, 7),
                hour,
                &rows.iter().collect::<Vec<_>>(),
                None,
                Some(&[]),
                None,
            );
            if hour % 257 == 0 {
                assert_eq!(
                    country.retained_bytes + 128,
                    history_bytes(&country.history)
                );
            }
        }
        assert_eq!(
            country.retained_bytes + 128,
            history_bytes(&country.history)
        );
        let mut countries = BTreeMap::from([("ENG".into(), country)]);
        countries.get_mut("ENG").unwrap().retained_bytes = MAX_BYTES;
        assert!(check_budget(&countries).is_err());
    }

    #[test]
    fn a_switch_gap_or_identity_change_cannot_lend_progress_to_an_estimate() {
        let mut country = Country::default();
        for hour in 0..=24 {
            observe(
                &mut country,
                hour,
                &input(1, i64::from(hour / 24) * 100_000),
            );
        }
        observe(&mut country, 25, &input(2, 0));
        assert_eq!(at(&country, 25), Some(100)); // Witnessed assignment uses starting multiplier.
        observe(&mut country, 27, &input(2, 0));
        assert_eq!(at(&country, 27), None);
        assert_eq!(
            country.history.slots[0].entries[1].outcome,
            ResearchOutcome::Unknown
        );
        country.observe_inputs((8, 7), 28, &[&input(2, 0)], None, Some(&[]), None);
        assert_eq!(at(&country, 28), None);
    }

    #[test]
    fn attribution_preserves_ids_unknowns_and_independent_bonuses() {
        let mut country = Country::default();
        let mut row = input(1, 0);
        row.bonus_count = 2;
        let bonuses = [
            ResearchAppliedBonusRow {
                country: 11,
                index: 0,
                ordinal: 0,
                technology_handle: 1,
                id: -17,
                resolved: 1,
                name: b"radio_bonus".to_vec(),
                cost_reduction_raw: None,
                speed_bonus_raw: 50_000,
                ahead_reduction_raw: 200_000,
            },
            ResearchAppliedBonusRow {
                country: 11,
                index: 0,
                ordinal: 1,
                technology_handle: 1,
                id: 99,
                ..Default::default()
            },
        ];
        country.observe_inputs((7, 7), 0, &[&row], None, Some(&bonuses), None);
        let data = country.history.slots[0].states[0]
            .details
            .as_ref()
            .unwrap()
            .bonuses
            .as_ref()
            .unwrap();
        assert_eq!(data[0].token.as_deref(), Some("radio_bonus"));
        assert_eq!(data[0].speed_bonus_raw.as_deref(), Some("50000"));
        assert_eq!(data[1].id, 99);
        assert!(data[1].token.is_none());
        country.observe_inputs((7, 7), 1, &[&row], None, None, None);
        assert!(
            country.history.slots[0].states[1]
                .details
                .as_ref()
                .unwrap()
                .bonuses
                .is_none()
        );
        assert_eq!(country.history.coverage[0].last_hour, 1);
    }

    #[test]
    fn delayed_inventory_completion_keeps_the_assignment_end_and_observation_separate() {
        let mut country = Country::default();
        observe(&mut country, 0, &input(1, 0));
        observe(&mut country, 1, &input(2, 0));
        let inventory = [ResearchTechnologyRow {
            country: 11,
            technology_handle: 1,
            level: 1,
            researched: 1,
            classifier: 4,
            ..Default::default()
        }];
        country.observe_inputs(
            (7, 7),
            2,
            &[&input(2, 0)],
            Some(&inventory),
            Some(&[]),
            None,
        );
        let first = &country.history.slots[0].entries[0];
        assert_eq!(first.outcome, ResearchOutcome::Completed);
        assert_eq!(first.ended_by_hour, Some(1));
        assert_eq!(first.outcome_observed_hour, Some(2));
    }
}

pub struct Research;

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReply {
    pub history: ResearchHistory,
    pub artwork: Vec<Artwork>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct State {
    artwork: Option<artwork::Context>,
    history: ResearchCache,
}

impl Product for Research {
    const KEY: &'static str = "research";
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
        let mut history =
            state
                .history
                .get(&source.replay(), params.branch(), &params.tag, cancelled)?;
        let art = artwork::ensure(
            &mut state.artwork,
            source,
            content,
            artwork::Kind::Technology,
        )?;
        let tokens = history
            .slots
            .iter()
            .flat_map(|s| {
                s.entries
                    .iter()
                    .map(|e| e.token.as_str())
                    .chain(s.states.iter().filter_map(|state| state.token.as_deref()))
            })
            .collect();
        let (artwork, mut warnings) = art.catalogue(tokens, cancelled)?;
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
        warnings.push("Research dates are observation bounds. Elapsed durations use observed assignments across slots; initial progress and gaps can leave the full duration unknown. Base duration is derived separately from recorded cost operands. Technologies without observed slot assignments are not assigned to a column.".into());
        Ok(ResearchReply {
            history,
            artwork,
            warnings,
        })
    }
}
