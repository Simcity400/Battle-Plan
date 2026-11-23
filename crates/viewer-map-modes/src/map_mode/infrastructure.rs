
use super::{Inputs, LEVEL_GRADIENT, ModeDefines, complete, finish, region_table_len, style};
use map_display_data::landscape::ModeLayer;
use content_reader_key_names::buildings::Buildings;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
use viewer_foundation::tables::core;
use dashboard_text::table_access::TokenNames;

pub(super) const NO_MAXIMUM: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    building: Result<Building, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Building {
    name: String,
    in_states: bool,
}

impl Default for Content {
    fn default() -> Self {
        Self {
            building: Err("no building type declares infrastructure = yes".to_owned()),
        }
    }
}

impl Content {
    pub fn new(buildings: &Buildings) -> Self {
        Self {
            building: buildings.infrastructure().and_then(|building| {
                building
                    .map(|building| Building {
                        name: building.name.clone(),
                        in_states: !building.provincial() && !building.only_costal,
                    })
                    .ok_or_else(|| "no building type declares infrastructure = yes".to_owned())
            }),
        }
    }

    fn snapshot<'a>(
        &self,
        frame: &'a DecodedFrame,
        state: &'a replay_data::rows::StateRow,
    ) -> Result<&'a replay_data::rows::StateInfrastructure, String> {
        let building = self.building.as_ref().map_err(Clone::clone)?;
        let value = state
            .infrastructure
            .require("States.infrastructure")?
            .as_ref()
            .ok_or("the infrastructure limit snapshot was not captured")?;
        let names = TokenNames::new(frame).map_err(|error| error.to_string())?;
        let name = names
            .name(value.building_type)
            .map_err(|error| error.to_string())?;
        if !name.eq_ignore_ascii_case(&building.name) {
            return Err(format!(
                "infrastructure snapshot names {name}, expected {}",
                building.name
            ));
        }
        Ok(value)
    }

    pub fn hover_level(&self, frame: &DecodedFrame, province: u32) -> Result<i32, String> {
        self.building.as_ref().map_err(Clone::clone)?;
        let provinces = frame
            .complete::<tables::Provinces>()
            .map_err(|error| error.to_string())?;
        let state = provinces
            .iter()
            .find(|row| row.id == province)
            .map_or(0, |row| row.state);
        if state == 0 {
            return Ok(0);
        }
        let state = frame
            .complete::<tables::States>()
            .map_err(|error| error.to_string())?
            .iter()
            .find(|row| row.id == state)
            .ok_or("the province's state was not captured")?;
        self.snapshot(frame, state).map(|value| value.level)
    }

    pub(super) fn layer(
        &self,
        defines: &ModeDefines,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<ModeLayer>> {
        let building = match &self.building {
            Ok(building) => building,
            Err(why) => {
                warnings.push(format!("The infrastructure map mode is not drawn: {why}"));
                return Ok(None);
            }
        };
        let states = match complete::<tables::States>(inputs.frame, "infrastructure") {
            Ok(states) => states,
            Err(why) => {
                warnings.push(why);
                return Ok(None);
            }
        };
        let [from, to] = defines.infrastructure_colors;
        let names = TokenNames::new(inputs.frame)?;
        let mut styles = BTreeMap::new();
        let mut refused = BTreeMap::<String, usize>::new();
        for state in states {
            let value = if building.in_states {
                let value = state
                    .infrastructure
                    .require("States.infrastructure")
                    .and_then(|value| {
                        value.as_ref().ok_or_else(|| {
                            "the infrastructure limit snapshot was not captured".to_owned()
                        })
                    })
                    .and_then(|value| {
                        let name = names
                            .name(value.building_type)
                            .map_err(|error| error.to_string())?;
                        if name.eq_ignore_ascii_case(&building.name) {
                            Ok(value)
                        } else {
                            Err(format!(
                                "infrastructure snapshot names {name}, expected {}",
                                building.name
                            ))
                        }
                    });
                match value {
                    Ok(value) => Some(value),
                    Err(why) => {
                        *refused.entry(why).or_default() += 1;
                        continue;
                    }
                }
            } else {
                None
            };
            let color = match value {
                Some(value) if value.maximum > 0 => {
                    let t = (value.level as f32 / value.maximum as f32).clamp(0.0, 1.0);
                    std::array::from_fn(|i| (to[i] - from[i]) * t + from[i])
                }
                _ => NO_MAXIMUM,
            };
            styles.insert(state.id + 1, style(color));
        }
        warnings.extend(refused.into_iter().map(|(why, count)| {
            format!("The infrastructure map mode leaves {count} states unfilled: {why}")
        }));
        let provinces = core::<tables::Provinces>(inputs.frame)?;
        let mut region_of =
            vec![0; region_table_len(inputs.map, provinces.iter().map(|row| row.id))];
        for province in provinces {
            if province.state != 0 && styles.contains_key(&(province.state + 1)) {
                region_of[province.id as usize] = province.state + 1;
            }
        }
        Ok(Some(finish(
            region_of,
            styles,
            Some(defines.state_thickness),
            Some(LEVEL_GRADIENT),
            [0.0, defines.state_channel[1]],
        )?))
    }
}

#[cfg(test)]
pub(super) fn for_tests(name: &str, in_states: bool) -> Content {
    Content {
        building: Ok(Building {
            name: name.to_owned(),
            in_states,
        }),
    }
}

#[cfg(test)]
mod tests;
