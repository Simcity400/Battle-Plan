use std::io;

use super::{MapMembership, Save, invalid};

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
    pub fn state_control(&self, membership: &MapMembership) -> io::Result<Vec<StateControl>> {
        let state_ids = membership
            .provinces
            .values()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(id) = self.states.keys().find(|id| !state_ids.contains(id)) {
            return Err(invalid(format!(
                "save state {id} has no membership in the verified content"
            )));
        }
        state_ids
            .into_iter()
            .map(|id| {
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
    pub fn province_control(&self, membership: &MapMembership) -> io::Result<Vec<ProvinceControl>> {
        let state_ids = membership
            .provinces
            .values()
            .flatten()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        if let Some(id) = self.states.keys().find(|id| !state_ids.contains(id)) {
            return Err(invalid(format!(
                "save state {id} has no membership in the verified content"
            )));
        }
        if let Some(id) = self
            .provinces
            .keys()
            .find(|id| !membership.provinces.contains_key(id) || **id == 0)
        {
            return Err(invalid(format!(
                "save province {id} is absent from the verified map or is its sentinel"
            )));
        }
        let mut rows = Vec::with_capacity(membership.provinces.len());
        for (&id, &state_id) in &membership.provinces {
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
