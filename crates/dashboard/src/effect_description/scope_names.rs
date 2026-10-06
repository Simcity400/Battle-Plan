//! Native names for the additional CEventScope carriers.

use super::*;
use replay::rows::{RaidScope, tables};

#[cfg(test)]
mod tests;

impl Renderer<'_, '_, '_> {
    /// 140FCDDF0 -> 140A81BB0/14146B580: narrative name, then default token;
    /// TAG_key when defined, expanded in a fresh owner country environment.
    pub(super) fn project_name(&self, owner: u32, id: u32) -> Result<String, String> {
        let world = self.context.world()?;
        let row = world.project(owner, id)?;
        let token = world.token(row.definition_id)?
            .ok_or("LexicalTokens lacks project definition")?;
        let token = std::str::from_utf8(token).map_err(|_| "project definition is not UTF-8")?;
        let cached = self.context.text.content().effects()?;
        let database = cached.characters.special_projects.as_ref().map_err(Clone::clone)?;
        if let Some(why) = &database.unsettled {
            return Err(why.clone());
        }
        let definition = database.projects.iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(token))
            .map(|(_, definition)| definition)
            .ok_or_else(|| format!("special-project content lacks {token}"))?;
        if let Some(why) = &definition.refusal {
            return Err(why.clone());
        }
        let key = definition.name.as_deref().filter(|key| !key.is_empty()).unwrap_or(token);
        let country = world.slot(owner).ok_or("CountryLinks lacks project owner")?;
        let tagged = format!("{}_{key}", self.tag(country)?);
        let key = if self.context.localisation.get(&tagged).is_some() { &tagged } else { key };
        self.localised(key.as_bytes(), &[], self.fresh_country_state_scope(country, 0)?, true)
    }

    /// CArmy slot 13@140C72F00 returns (+0x340)->+0x60, a stale display cache.
    pub(super) fn unit_name(&self, reference: u64) -> Result<String, String> {
        self.context.world()?.unit(reference)?;
        Err(format!("UNIT {reference:#x} name: CArmy+0x340 -> CNameGroupMember+0x60 cached MSVC string is not recorded (140C72F00; writer 1409B4C40)"))
    }

    /// Instance description@140FD6260: raid_inst_item_desc(TYPE, TARGET),
    /// GetLocalizedName@140A8E3B0 and CRaidTarget::GetLocalizedName@140FD1D50.
    #[allow(dead_code, reason = "shared carrier name entry point for raid effect ports")]
    pub(super) fn raid_name(&self, reference: u64) -> Result<String, String> {
        self.context.world()?.raid(reference)?;
        let target = self.context.text.frame().complete::<tables::RaidTargets>()
            .map_err(|why| why.to_string())?.iter()
            .find(|row| (u64::from(row.id) << 32 | u64::from(row.reference_type)) == reference)
            .ok_or_else(|| format!("RaidTargets lacks raid {reference:#x}"))?;
        let (kind, type_name) = if target.raid_type == 0 {
            (None, String::new()) // 140FD6260: null type contributes empty TYPE.
        } else {
            let token = self.context.world()?.token(target.raid_type)?
                .ok_or("LexicalTokens lacks raid type")?;
            let token = std::str::from_utf8(token).map_err(|_| "raid type is not UTF-8")?;
            let cache = self.context.text.content().effects()?;
            let database = cache.raids.get_or_init(|| {
                installed_content::raids::read(&self.context.text.content().layers.layers)
                    .map_err(|why| why.to_string())
            }).as_ref().map_err(Clone::clone)?;
            if let Some(uncertain) = database.uncertain.iter()
                .find(|row| row.name.eq_ignore_ascii_case(token))
            {
                return Err(uncertain.reason.clone());
            }
            let kind = database.types.iter().find(|kind| kind.name.eq_ignore_ascii_case(token))
                .ok_or_else(|| format!("raid content lacks type {token}"))?;
            (Some(kind), self.carrier_native_text(&format!("raid_type_{}", kind.name), &[])?)
        };
        let target_name = self.raid_target_name(&target.scope, kind)?;
        self.carrier_native_text("raid_inst_item_desc", &[
            ("TYPE", Argument::Text(&type_name)), ("TARGET", Argument::Text(&target_name)),
        ])
    }

    fn carrier_native_text(&self, key: &str, args: &[(&str, Argument<'_>)]) -> Result<String, String> {
        if self.context.localisation.get(key).is_none() {
            return Err(format!("native carrier localisation key {key} is not defined"));
        }
        self.localised(key.as_bytes(), args, Scopes::empty(), false)
    }

    fn raid_target_name(&self, scope: &RaidScope, kind: Option<&installed_content::raids::RaidType>) -> Result<String, String> {
        let province = if scope.province != 0 { scope.province } else { scope.leader_province };
        let vp = if province != 0 { self.context.text.province_victory_point_name(province)? } else { String::new() };
        let location = if !vp.is_empty() {
            vp.clone()
        } else {
            // 140FD1BB0: explicit state, province/leader province, building state.
            let state = if scope.state != 0 {
                scope.state
            } else if province != 0 {
                self.raid_province_state(province)?
            } else if scope.building_type != 0 {
                if scope.building_flags & 1 != 0 {
                    self.raid_province_state(scope.building_location)?
                } else if scope.building_flags & 2 != 0 {
                    scope.building_location
                } else { 0 }
            } else { 0 };
            if state != 0 { self.state_name(state)? } else { "no_target_name".into() } // 14296C780.
        };
        if let Some(kind) = kind {
            if let Some(why) = &kind.tooltip.refused { return Err(why.clone()); }
            let custom = &kind.tooltip.target_loc_key;
            if !custom.is_empty() && self.context.localisation.get(custom).is_some() {
                return self.localised(custom.as_bytes(), &[("LOCATION", Argument::Text(&location))], Scopes::empty(), false);
            }
        }
        if scope.building_type != 0 {
            let world = self.context.world()?;
            let token = world.token(scope.building_type)?.ok_or("LexicalTokens lacks raid building type")?;
            let cached = self.context.text.content().effects()?;
            let buildings = self.context.content.buildings.or(cached.buildings.as_ref())
                .ok_or_else(|| self.content_gap("buildings"))?;
            let building = buildings.by_token(std::str::from_utf8(token).map_err(|_| "raid building type is not UTF-8")?)
                .map_err(|why| why.to_string())?
                .ok_or("raid building type is absent from content")?;
            let name = self.localised(building.name.as_bytes(), &[], Scopes::empty(), false)?;
            return self.carrier_native_text("RAID_TARGET_NAME", &[
                ("WHAT", Argument::Text(&name)), ("LOCATION", Argument::Text(&location)),
            ]);
        }
        if province != 0 {
            if !vp.is_empty() {
                return self.carrier_native_text("raid_target_name_victory_point", &[("VICTORY_POINT", Argument::Text(&location))]);
            }
            let terrain = self.context.text.terrain_id_name(self.context.text.province_terrain(province)?)?;
            return self.carrier_native_text("PROV_TOOLTIP_LOCATION_STATE", &[
                ("TERRAIN", Argument::Text(&terrain)), ("NAME", Argument::Text(&location)),
            ]);
        }
        Ok(location)
    }

    fn raid_province_state(&self, province: u32) -> Result<u32, String> {
        self.context.text.frame().complete::<tables::Provinces>()
            .map_err(|why| why.to_string())?.iter().find(|row| row.id == province)
            .map(|row| row.state).ok_or_else(|| format!("raid target province {province} is not recorded"))
    }
}
