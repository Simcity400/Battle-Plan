
use engine_native_memory::{Memory, read};

pub use engine_native::ctask_force::TaskForceStats;

pub type Fact<T> = Result<T, &'static str>;

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
            != engine_sites::at(base, crate::air_mission::VTABLE_RVA).map(|at| at as u64)
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

fn word<T: engine_native_memory::memory::Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Fact<T> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|at| read(memory, at))
        .ok_or("transport original air mission word unreadable")
}

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
    engine_native_memory::cpdx_array::Array::read(memory, collection, 0x270)
        .and_then(|array| array.element(index, 0x270))
        .ok_or("transport original air mission stat index/header refused")
}

#[cfg(test)]
#[path = "transport_spotting_graph_tests.rs"]
mod tests;
