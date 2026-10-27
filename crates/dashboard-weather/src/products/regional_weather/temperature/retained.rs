use super::{Observation, average};
use dashboard_keyed::Values;
use replay_data::temperature::Input;
use std::sync::Arc;

impl Observation {
    pub(in super::super) fn observe_input(
        &mut self,
        branch: u64,
        hour: i32,
        input: Result<&Input, &str>,
        values: &mut Values,
    ) -> bool {
        let Some(content) = self.content.as_ref().map(Arc::clone) else {
            if self.status.as_deref() == Some("available") {
                self.status = None;
            }
            return false;
        };
        let states = input.map_err(str::to_owned).and_then(|input| {
            let states = self
                .memo
                .states_input(branch, hour, input, Ok(&content.pass))?;
            if states.len() != content.pass.provinces.len() {
                return Err(format!(
                    "content has {} weather province slots, the recording {}",
                    content.pass.provinces.len(),
                    states.len()
                ));
            }
            Ok(states)
        });
        let states = match states {
            Ok(states) => states,
            Err(reason) => {
                *self.refused.entry(reason).or_default() += 1;
                return false;
            }
        };
        let mut known = false;
        let mut missing = 0u64;
        for (&region, members) in &content.members {
            if let Some(mean) = average(members, states) {
                values.push(format!("{region}|temp"), mean);
                known = true;
            } else {
                missing += 1;
            }
        }
        if missing > 0 {
            *self
                .refused
                .entry(
                    "region membership is empty or a member province temperature is unavailable"
                        .into(),
                )
                .or_default() += missing;
        }
        known
    }
}
