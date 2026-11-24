
use super::*;
use replay_data::rows::{AirWingRow, ProvinceRow, RaidScope, tables};

const WING_NAME: u64 = mask::<AirWingRow>(&["name"]);
const PROVINCE_STATE: u64 = mask::<ProvinceRow>(&["state"]);

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    pub(super) fn project_name(&self, owner: u32, id: u32) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world.project(owner, id).map_err(|why| why.to_string())?;
        let token = world
            .token(row.definition_id)
            .map_err(|why| why.to_string())?
            .ok_or("lexical tokens lacks project definition")?;
        let token = std::str::from_utf8(token).map_err(|_| "project definition is not UTF-8")?;
        let cached = self.context.text.content().effects()?;
        let database = cached
            .characters
            .special_projects
            .as_ref()
            .map_err(Clone::clone)?;
        if let Some(why) = &database.unsettled {
            return Err(why.clone());
        }
        let definition = database
            .projects
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(token))
            .map(|(_, definition)| definition)
            .ok_or_else(|| format!("special-project content lacks {token}"))?;
        if let Some(why) = &definition.refusal {
            return Err(why.clone());
        }
        let key = definition
            .name
            .as_deref()
            .filter(|key| !key.is_empty())
            .unwrap_or(token);
        let country = world
            .slot(owner)
            .ok_or("country links lacks project owner")?;
        let tagged = format!("{}_{key}", self.tag(country)?);
        let key = if self.context.localisation.get(&tagged).is_some() {
            &tagged
        } else {
            key
        };
        self.localised(
            key.as_bytes(),
            &[],
            self.fresh_country_state_scope(country, 0)?,
            true,
        )
    }

    pub(super) fn unit_name(&self, reference: u64) -> Result<String, String> {
        if let Ok(wings) = self.tables().complete::<tables::AirWings>(WING_NAME)
            && let Some(wing) = keyed(&wings, u128::from(reference))?
        {
            let bytes = match &wing.name {
                replay_data::rows::Recorded::Value(Some(bytes)) => bytes.as_slice(),
                replay_data::rows::Recorded::Value(None) => {
                    return Err(format!(
                        "air wings UNIT {reference:#x} stored name unavailable: guarded read or name arena refusal"
                    ));
                }
                replay_data::rows::Recorded::NotRecorded => {
                    let bytes = self
                        .context
                        .world()?
                        .raid_wing_name(reference)
                        .map_err(|why| why.to_string())?;
                    return crate::scripted_text::stored_text(
                        self.context.localisation,
                        self.context.text.recorded_language(),
                        &bytes,
                        "raid wing name",
                    );
                }
            };
            return crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                bytes,
                "air wing name",
            );
        }
        let world = self.context.world()?;
        let divisions = match world.tables().complete::<tables::Divisions>(0) {
            Ok(rows) => rows,
            Err(why) => {
                if let Ok(bytes) = world.raid_wing_name(reference) {
                    return crate::scripted_text::stored_text(
                        self.context.localisation,
                        self.context.text.recorded_language(),
                        &bytes,
                        "raid wing name",
                    );
                }
                return Err(why.to_string());
            }
        };
        if !divisions.contains(u128::from(reference)) {
            let bytes = world
                .raid_wing_name(reference)
                .map_err(|why| format!("Divisions lacks UNIT {reference:#x}; {why}"))?;
            return crate::scripted_text::stored_text(
                self.context.localisation,
                self.context.text.recorded_language(),
                &bytes,
                "raid wing name",
            );
        }
        crate::unit_names::army_name(
            &*world
                .unit_status(reference)
                .map_err(|why| why.to_string())?,
            self.context.localisation,
            self.context.text.recorded_language(),
            || {
                let cached = self.context.text.content().effects()?;
                self.context
                    .content
                    .division_names
                    .or(cached.division_names.as_ref())
                    .ok_or_else(|| self.content_gap("division name groups"))
            },
        )
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "only its tests call it"))]
    pub(super) fn raid_name(&self, reference: u64) -> Result<String, String> {
        self.context
            .world()?
            .raid(reference)
            .map_err(|why| why.to_string())?;
        let target = self
            .context
            .text
            .tables()
            .complete::<tables::RaidTargets>(tables::RaidTargets::ID.full_mask())
            .map_err(|why| why.to_string())?
            .get(u128::from(reference))
            .map_err(built)?
            .ok_or_else(|| format!("raid targets lacks raid {reference:#x}"))?;
        let (kind, type_name) = if target.raid_type == 0 {
            (None, String::new())
        } else {
            let token = self
                .context
                .world()?
                .token(target.raid_type)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks raid type")?;
            let token = std::str::from_utf8(token).map_err(|_| "raid type is not UTF-8")?;
            let cache = self.context.text.content().effects()?;
            let database = cache
                .raids
                .get_or_init(|| {
                    content_reader_raids::raids::read(&self.context.text.content().layers.layers)
                        .map_err(|why| why.to_string())
                })
                .as_ref()
                .map_err(Clone::clone)?;
            if let Some(uncertain) = database
                .uncertain
                .iter()
                .find(|row| row.name.eq_ignore_ascii_case(token))
            {
                return Err(uncertain.reason.clone());
            }
            let kind = database
                .types
                .iter()
                .find(|kind| kind.name.eq_ignore_ascii_case(token))
                .ok_or_else(|| format!("raid content lacks type {token}"))?;
            (
                Some(kind),
                self.carrier_native_text(&format!("raid_type_{}", kind.name), &[])?,
            )
        };
        let target_name = self.raid_target_name(&target.scope, kind)?;
        self.carrier_native_text(
            "raid_inst_item_desc",
            &[
                ("TYPE", Argument::Text(&type_name)),
                ("TARGET", Argument::Text(&target_name)),
            ],
        )
    }

    fn carrier_native_text(
        &self,
        key: &str,
        args: &[(&str, Argument<'_>)],
    ) -> Result<String, String> {
        if self.context.localisation.get(key).is_none() {
            return Err(format!(
                "native carrier localisation key {key} is not defined"
            ));
        }
        self.localised(key.as_bytes(), args, Scopes::empty(), false)
    }

    fn raid_target_name(
        &self,
        scope: &RaidScope,
        kind: Option<&content_reader_raids::raids::RaidType>,
    ) -> Result<String, String> {
        let province = if scope.province != 0 {
            scope.province
        } else {
            scope.leader_province
        };
        let vp = if province != 0 {
            self.context.text.province_victory_point_name(province)?
        } else {
            String::new()
        };
        let location = if !vp.is_empty() {
            vp.clone()
        } else {
            let state = if scope.state != 0 {
                scope.state
            } else if province != 0 {
                self.raid_province_state(province)?
            } else if scope.building_type != 0 {
                if scope.building_flags & 1 != 0 {
                    self.raid_province_state(scope.building_location)?
                } else if scope.building_flags & 2 != 0 {
                    scope.building_location
                } else {
                    0
                }
            } else {
                0
            };
            if state != 0 {
                self.state_name(state)?
            } else {
                "no_target_name".into()
            }
        };
        if let Some(kind) = kind {
            if let Some(why) = &kind.tooltip.refused {
                return Err(why.clone());
            }
            let custom = &kind.tooltip.target_loc_key;
            if !custom.is_empty() && self.context.localisation.get(custom).is_some() {
                return self.localised(
                    custom.as_bytes(),
                    &[("LOCATION", Argument::Text(&location))],
                    Scopes::empty(),
                    false,
                );
            }
        }
        if scope.building_type != 0 {
            let world = self.context.world()?;
            let token = world
                .token(scope.building_type)
                .map_err(|why| why.to_string())?
                .ok_or("lexical tokens lacks raid building type")?;
            let cached = self.context.text.content().effects()?;
            let buildings = self
                .context
                .content
                .buildings
                .or(cached.buildings.as_ref())
                .ok_or_else(|| self.content_gap("buildings"))?;
            let building = buildings
                .by_token(
                    std::str::from_utf8(token).map_err(|_| "raid building type is not UTF-8")?,
                )
                .map_err(|why| why.to_string())?
                .ok_or("raid building type is absent from content")?;
            let name = self.localised(building.name.as_bytes(), &[], Scopes::empty(), false)?;
            return self.carrier_native_text(
                "RAID_TARGET_NAME",
                &[
                    ("WHAT", Argument::Text(&name)),
                    ("LOCATION", Argument::Text(&location)),
                ],
            );
        }
        if province != 0 {
            if !vp.is_empty() {
                return self.carrier_native_text(
                    "raid_target_name_victory_point",
                    &[("VICTORY_POINT", Argument::Text(&location))],
                );
            }
            let terrain = self
                .context
                .text
                .terrain_id_name(self.context.text.province_terrain(province)?)?;
            return self.carrier_native_text(
                "PROV_TOOLTIP_LOCATION_STATE",
                &[
                    ("TERRAIN", Argument::Text(&terrain)),
                    ("NAME", Argument::Text(&location)),
                ],
            );
        }
        Ok(location)
    }

    fn raid_province_state(&self, province: u32) -> Result<u32, String> {
        keyed(&self.complete::<tables::Provinces>(PROVINCE_STATE)?, u128::from(province))?
            .map(|row| row.state)
            .ok_or_else(|| format!("raid target province {province} is not recorded"))
    }
}
