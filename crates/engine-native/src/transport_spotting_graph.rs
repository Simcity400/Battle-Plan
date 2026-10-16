//! Independent selected-transport words borrowed from the original stat selector.
//!
//! Transport weight/detection read +0x138/+0x160; visibility reads
//! +0x148/+0x150. The caller owns the original unit +0x138 selector and
//! stat-definition authentication. Failed selectors are never retried.
//! Copied numbers do not establish carrier or air-region lifetime.

use crate::{Memory, read};

pub type Fact<T> = Result<T, &'static str>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskForceStats {
    pub detection_138: Fact<i64>,
    pub visibility_148: Fact<i64>,
    pub visibility_150: Fact<i64>,
    pub carrier_detection_160: Fact<i64>,
}

impl TaskForceStats {
    /// Consumes the original same-pass authenticated stat pointer result.
    /// Every numerical field has one independent attempt; a field-local fault
    /// cannot withhold another word or turn it into a default value.
    pub fn capture(memory: &impl Memory, original: Fact<usize>) -> Self {
        Self::capture_with_visibility(memory, original, true)
    }

    /// Contributor detection reaches only the detection words; the receiver
    /// visibility query may remain unreached.
    pub fn capture_detection(memory: &impl Memory, original: Fact<usize>) -> Self {
        Self::capture_with_visibility(memory, original, false)
    }

    fn capture_with_visibility(
        memory: &impl Memory,
        original: Fact<usize>,
        visibility: bool,
    ) -> Self {
        let word = |offset| {
            original.and_then(|stats| {
                stats
                    .checked_add(offset)
                    .filter(|_| stats != 0)
                    .and_then(|at| read::<i64>(memory, at))
                    .ok_or("selected transport task-force stat word unreadable")
            })
        };
        Self {
            detection_138: word(0x138),
            visibility_148: if visibility {
                word(0x148)
            } else {
                Err("selected transport visibility stat query unreached")
            },
            visibility_150: if visibility {
                word(0x150)
            } else {
                Err("selected transport visibility stat query unreached")
            },
            carrier_detection_160: word(0x160),
        }
    }
}

/// Stored transport-air inputs from one authenticated wing. The current
/// mission word is distinct from the enabled mask.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AirMissionWords {
    pub wing: u64,
    pub current_type: Option<u32>,
    pub aircraft: Option<i32>,
    pub disruption: Option<i64>,
    pub detection_13: Option<i64>,
    pub detection_14: Option<i64>,
    pub faults: u32,
}

impl AirMissionWords {
    pub fn capture(
        memory: &impl Memory,
        base: usize,
        original: &crate::cair_regional_mission::WingIdentity,
    ) -> Fact<Self> {
        Self::capture_observed(memory, base, original, |_, _, _, _| {})
    }

    /// Loans the original wing and stat selector (including failure) while
    /// this exact invocation still owns their authenticated native lifetime.
    pub fn capture_observed(
        memory: &impl Memory,
        base: usize,
        original: &crate::cair_regional_mission::WingIdentity,
        mut observer: impl FnMut(usize, u32, Fact<usize>, &AirMissionWords),
    ) -> Fact<Self> {
        let mission = original.mission_pointer;
        let wing = mission
            .checked_sub(crate::cair_wing::MISSION)
            .filter(|&pointer| pointer != 0)
            .ok_or("transport original air wing pointer invalid")?;
        if read::<u64>(memory, mission)
            != base
                .checked_add(crate::air_mission::VTABLE_RVA)
                .map(|at| at as u64)
            || mission
                .checked_add(crate::air_mission::WING)
                .and_then(|at| read::<u64>(memory, at))
                != Some(wing as u64)
        {
            return Err("transport original air mission identity/backlink refused");
        }
        let mut value = Self {
            wing: original.wing,
            ..Default::default()
        };
        value.current_type = word(memory, mission, crate::air_mission::CURRENT_TYPE).ok();
        if value.current_type.is_none() {
            value.faults += 1;
            return Ok(value);
        }
        let kind = value.current_type.unwrap();
        if kind == 0 {
            return Ok(value);
        }
        value.aircraft = word(memory, mission, 0xe0).ok();
        value.disruption = if kind & 0x1f7fa != 0 {
            word(memory, mission, 0xa8).ok()
        } else {
            None
        };
        value.faults += u32::from(value.aircraft.is_none())
            + u32::from(kind & 0x1f7fa != 0 && value.disruption.is_none());
        let stats = mission_stats(memory, wing, kind);
        value.detection_13 = stats.and_then(|stats| word(memory, stats, 0x13 * 8)).ok();
        value.detection_14 = stats.and_then(|stats| word(memory, stats, 0x14 * 8)).ok();
        value.faults +=
            u32::from(value.detection_13.is_none()) + u32::from(value.detection_14.is_none());
        observer(wing, kind, stats, &value);
        Ok(value)
    }
}

fn word<T: crate::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Fact<T> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
        .ok_or("transport original air mission word unreadable")
}

/// Single original mission-stat selector; only -1 selects fallback.
fn mission_stats(memory: &impl Memory, wing: usize, kind: u32) -> Fact<usize> {
    if !kind.is_power_of_two() || kind as i32 <= 0 {
        return Err("transport original air current mission is not one positive bit");
    }
    let collection = wing
        .checked_add(0x968)
        .ok_or("transport air stat collection overflow")?;
    let index: i8 = word(memory, collection, 0x18 + kind.trailing_zeros() as usize)?;
    if index == -1 {
        return wing
            .checked_add(0x390)
            .ok_or("transport default air stat address overflow");
    }
    let index = u32::try_from(index).map_err(|_| "transport air mission stat index is negative")?;
    crate::cpdx_array::Array::read(memory, collection, 0x270)
        .and_then(|array| array.element(index, 0x270))
        .ok_or("transport original air mission stat index/header refused")
}

#[cfg(test)]
#[path = "transport_spotting_graph_tests.rs"]
mod tests;
