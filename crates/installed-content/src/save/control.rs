use std::{collections::BTreeSet, io};

use super::{PartialMapMembership, ProvinceMembership, Save, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvinceControl {
    pub id: u32,
    pub state: Option<u32>,
    /// None is the no-owner/no-controller sentinel for a province without a state.
    pub owner: Option<Vec<u8>>,
    pub controller: Option<Vec<u8>>,
    pub controller_explicit: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateControl {
    pub id: u32,
    pub owner: Vec<u8>,
    pub controller: Vec<u8>,
    pub controller_explicit: bool,
}

impl Save {
    /// CState writer 1409d4040 omits controllers equal to a positive owner by
    /// raw id or canonical link slot; post-load 1409cdf80 restores owner.
    /// The writer emits zero controllers, unlike CProvince. A missing owner
    /// is refused because that path omits both fields, losing controller data.
    /// Membership supplies only the kept state-ID inventory; control comes
    /// entirely from the save, including for states with excluded membership.
    pub fn known_state_control(
        &self,
        membership: &PartialMapMembership,
    ) -> io::Result<Vec<StateControl>> {
        self.state_control_for(&state_inventory(membership))
    }

    fn validate_state_inventory(&self, state_ids: &BTreeSet<u32>) -> io::Result<()> {
        if let Some(id) = self.states.keys().find(|id| !state_ids.contains(id)) {
            return Err(invalid(format!(
                "save state {id} has no membership in the verified content"
            )));
        }
        Ok(())
    }

    fn state_control_for(&self, state_ids: &BTreeSet<u32>) -> io::Result<Vec<StateControl>> {
        self.validate_state_inventory(state_ids)?;
        state_ids
            .iter()
            .map(|&id| {
                let state = self
                    .states
                    .get(&id)
                    .ok_or_else(|| invalid(format!("save lacks state {id}")))?;
                let owner = state
                    .owner
                    .as_ref()
                    .ok_or_else(|| invalid(format!("save state {id} lacks its owner")))?;
                let controller = state.controller.as_ref().unwrap_or(owner);
                for tag in [owner, controller] {
                    if !self.countries.contains_key(tag) {
                        return Err(invalid(format!(
                            "state {id} references unknown saved country {}",
                            String::from_utf8_lossy(tag)
                        )));
                    }
                }
                Ok(StateControl {
                    id,
                    owner: owner.clone(),
                    controller: controller.clone(),
                    controller_explicit: state.controller.is_some(),
                })
            })
            .collect()
    }

    /// CProvince writer 140e6ca90 omits a controller equivalent to its state's
    /// owner; post-load 1409cdf80 fills omitted/nonpositive controls from that
    /// owner. A missing state or state-owner field cannot be filled this way.
    /// Unknown provinces have no inferred owner/controller row. Other rows use
    /// the native save omissions and the full kept state-ID inventory.
    pub fn known_province_control(
        &self,
        membership: &PartialMapMembership,
    ) -> io::Result<Vec<ProvinceControl>> {
        self.validate_state_inventory(&state_inventory(membership))?;
        if let Some(id) = self
            .provinces
            .keys()
            .find(|id| !membership.contains_province(**id) || **id == 0)
        {
            return Err(invalid(format!(
                "save province {id} is absent from the verified map or is its sentinel"
            )));
        }
        self.province_control_for(membership.provinces.iter().filter_map(|(&id, assignment)| {
            match assignment {
                ProvinceMembership::Known(state) => Some((id, *state)),
                ProvinceMembership::Unknown(_) => None,
            }
        }))
    }

    fn province_control_for(
        &self,
        assignments: impl Iterator<Item = (u32, Option<u32>)>,
    ) -> io::Result<Vec<ProvinceControl>> {
        let mut rows = Vec::new();
        for (id, state_id) in assignments {
            if id == 0 {
                continue;
            }
            let owner = state_id.map(|state_id| {
                self.states.get(&state_id).ok_or_else(|| invalid(format!("save lacks state {state_id}, required by province {id}")))?
                    .owner.clone().ok_or_else(|| invalid(format!("save state {state_id} lacks its owner; refusing to infer province {id}")))
            }).transpose()?;
            let override_row = self.provinces.get(&id);
            if override_row.is_some_and(|row| row.owner.is_some()) {
                return Err(invalid(format!(
                    "save province {id} has an unsupported per-province owner field"
                )));
            }
            let explicit = override_row.and_then(|row| row.controller.clone());
            let controller = explicit.clone().or_else(|| owner.clone());
            for tag in [&owner, &controller].into_iter().flatten() {
                if !self.countries.contains_key(tag) {
                    return Err(invalid(format!(
                        "province {id} references unknown saved country {}",
                        String::from_utf8_lossy(tag)
                    )));
                }
            }
            rows.push(ProvinceControl {
                id,
                state: state_id,
                owner,
                controller,
                controller_explicit: explicit.is_some(),
            });
        }
        Ok(rows)
    }
}

fn state_inventory(membership: &PartialMapMembership) -> BTreeSet<u32> {
    membership
        .known_states
        .iter()
        .chain(membership.excluded_states.keys())
        .copied()
        .collect()
}
