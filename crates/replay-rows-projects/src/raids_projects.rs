use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectState {
    pub current_offset: u32,
    pub state_enums: [u32; 4],
    pub not_completed: u8,
    pub phase_progress: i64,
    pub phase_target: i32,
    pub prototype_progress: i32,
    pub iterations: i32,
}
value_struct!(ProjectState {
    current_offset: u32,
    state_enums: [u32; 4],
    not_completed: u8,
    phase_progress: i64,
    phase_target: i32,
    prototype_progress: i32,
    iterations: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectContext {
    pub province: i32,
    pub progress: i32,
    pub character_reference: u64,
    pub country_present: u8,
    pub country_reference: Option<u32>,
    pub scientist_present: u8,
    pub scientist_reference: Option<u32>,
    pub state_present: u8,
    pub state_reference: Option<u32>,
}
value_struct!(ProjectContext {province: i32, progress: i32, character_reference: u64, country_present: u8, country_reference: Option<u32>, scientist_present: u8, scientist_reference: Option<u32>, state_present: u8, state_reference: Option<u32>});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidSource {
    pub kind: Option<u32>,
    pub kind_present: u8,
    pub province: u32,
    pub ship_reference_type: u32,
    pub ship_reference_id: u32,
    pub country: i32,
    pub building_type: u32,
    pub building_location: i32,
    pub building_country: i32,
}
value_struct!(RaidSource {kind: Option<u32>, kind_present: u8, province: u32, ship_reference_type: u32, ship_reference_id: u32, country: i32, building_type: u32, building_location: i32, building_country: i32});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidScope {
    pub province: u32,
    pub state: u32,
    pub leader_reference: u64,
    pub leader_province: u32,
    pub building_type: u32,
    pub building_location: u32,
    pub building_flags: u32,
    pub building_country: i32,
    pub leader_country: Option<i32>,
}
value_struct!(RaidScope {province: u32, state: u32, leader_reference: u64, leader_province: u32, building_type: u32, building_location: u32, building_flags: u32, building_country: i32, leader_country: Option<i32>});

impl RaidScope {
    pub fn country_with<E>(
        &self,
        province_controller: impl Fn(u32) -> Result<u32, E>,
        state_controller: impl Fn(u32) -> Result<u32, E>,
    ) -> Result<Option<u32>, E> {
        let raw = if self.building_type != 0 {
            if self.building_country > 0 {
                self.building_country as u32
            } else if self.building_flags & 1 != 0 {
                province_controller(self.building_location)?
            } else {
                state_controller(self.building_location)?
            }
        } else if self.province != 0 {
            province_controller(self.province)?
        } else if self.state != 0 {
            state_controller(self.state)?
        } else if let Some(captor) = self.leader_country {
            captor as u32
        } else if self.leader_province != 0 {
            province_controller(self.leader_province)?
        } else {
            0
        };
        Ok((raw != 0).then_some(raw))
    }

    pub fn state_with<E>(&self, province_state: impl Fn(u32) -> Result<u32, E>) -> Result<Option<u32>, E> {
        if self.state != 0 {
            return Ok(Some(self.state));
        }
        let id = if self.province != 0 {
            province_state(self.province)?
        } else if self.leader_province != 0 {
            province_state(self.leader_province)?
        } else if self.building_type == 0 {
            0
        } else if self.building_flags & 1 != 0 {
            province_state(self.building_location)?
        } else if self.building_location as i32 <= 0 {
            0
        } else {
            self.building_location
        };
        Ok((id != 0).then_some(id))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectFacilityOperand {
    pub kind: u32,
    pub scientist: u64,
    pub token: u32,
    pub key: u32,
    pub ordinal: u32,
    pub assignment: u32,
    pub raw: i64,
}
value_struct!(ProjectFacilityOperand {
    kind: u32,
    scientist: u64,
    token: u32,
    key: u32,
    ordinal: u32,
    assignment: u32,
    raw: i64
});
impl OpaqueValue for ProjectFacilityOperand {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectScientistAssignment {
    pub scientist: u64,
    pub target: i32,
    pub current: i32,
    pub timed: u8,
}
value_struct!(ProjectScientistAssignment {
    scientist: u64,
    target: i32,
    current: i32,
    timed: u8
});
impl OpaqueValue for ProjectScientistAssignment {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectVariable {
    pub name: Vec<u8>,
    pub value_raw: i64,
}
value_struct!(ProjectVariable { name: Vec<u8>, value_raw: i64 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectVariableStore {
    pub seed: u32,
    pub entries: Vec<ProjectVariable>,
}
value_struct!(ProjectVariableStore { seed: u32, entries: Vec<ProjectVariable> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ProjectRuntime {
    pub pending_known: bool,
    pub pending_reward: Option<u32>,
    pub pending_ordinal: Option<u32>,
    pub variables: Option<std::sync::Arc<ProjectVariableStore>>,
}
value_struct!(ProjectRuntime { pending_known: bool, pending_reward: Option<u32>, pending_ordinal: Option<u32>, variables: Option<std::sync::Arc<ProjectVariableStore>> });
opaque_value!(ProjectRuntime);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum RaidPathDispatcher {
    #[default]
    Land,
    Naval,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RaidCounterOperative {
    pub handle: u64,
    pub defense: i64,
    pub context: i64,
}
value_struct!(RaidCounterOperative {
    handle: u64,
    defense: i64,
    context: i64
});
impl OpaqueValue for RaidCounterOperative {}
