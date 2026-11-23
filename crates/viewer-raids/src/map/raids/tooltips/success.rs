use super::super::{RaidAssets, script::World, target_country, unit_inputs};
use super::{chance, intel};
use engine_executable::modifier_keys as keys;
use engine_native_air::raid_tooltip_operands::*;
use content_reader_raids::raids::tooltips::Modifier;
use replay_data::{rows::RaidRow, rows::RaidScope, rows::RaidTooltipOperandRow, rows::tables};

type Result<T> = std::result::Result<T, String>;
fn times(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / 100_000
}
fn ratio(a: i64, b: i64) -> Result<i64> {
    if b == 0 || (a as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
        return Ok(0xffff_ffff);
    }
    a.wrapping_mul(100_000)
        .checked_div(b)
        .ok_or("raid success signed division overflow".into())
}

pub struct Inputs<'a, 'w> {
    rows: Vec<&'a RaidTooltipOperandRow>,
    assets: &'a RaidAssets,
    world: &'a World<'w>,
    raid: &'a RaidRow,
    scope: &'a RaidScope,
}
impl<'a, 'w> Inputs<'a, 'w> {
    pub fn new(
        assets: &'a RaidAssets,
        world: &'a World<'w>,
        raid: &'a RaidRow,
        scope: &'a RaidScope,
    ) -> Result<Self> {
        let key = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let rows = world
            .frame()
            .complete::<tables::RaidTooltipOperands>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|r| r.raid == key)
            .collect();
        let out = Self {
            rows,
            assets,
            world,
            raid,
            scope,
        };
        if out.one(UNREADABLE, 0).is_some() {
            return Err("raid operands were unreadable when recorded".into());
        }
        if out.one(REFERENCES, 0) != Some(raid.unit as i64)
            || out.one(REFERENCES, 1) != Some(raid.air_wing as i64)
        {
            return Err("RaidTooltipOperands reference witness differs from Raids".into());
        }
        Ok(out)
    }
    fn one(&self, kind: u32, key: u32) -> Option<i64> {
        self.rows
            .iter()
            .find(|r| r.kind == kind && r.key == key)
            .map(|r| r.raw)
    }
    fn required(&self, kind: u32, key: u32) -> Result<i64> {
        self.one(kind, key)
            .ok_or_else(|| format!("RaidTooltipOperands lacks {kind}/{key:#x}"))
    }
    fn values(&self, kind: u32) -> impl Iterator<Item = &RaidTooltipOperandRow> {
        self.rows.iter().copied().filter(move |r| r.kind == kind)
    }
    fn modifier(&self, kind: u32, key: u32) -> Result<i64> {
        let Some(raw) = self.one(kind, key | LOOKUP) else {
            return Ok(0);
        };
        let frame = self.world.frame();
        let definitions = frame
            .complete::<tables::ModifierDefinitions>()
            .map_err(|e| e.to_string())?;
        let definition = definitions
            .iter()
            .find(|r| r.index == key)
            .ok_or_else(|| format!("modifier definition {key:#x} absent"))?;
        let features = frame
            .complete::<tables::SimulationFeatures>()
            .map_err(|e| e.to_string())?;
        let features = features.first().ok_or("simulation feature flags absent")?;
        Ok(engine_native::modifier_definition::Operands {
            flags: definition.flags,
            selector: definition.feature_selector,
            eligibility_flags: features.dlc_mask,
        }
        .effective(raw))
    }
    pub fn input(&self, m: &Modifier, current: impl FnOnce() -> Result<i64>) -> chance::Input {
        if !m.custom.is_empty() {
            let reason = format!(
                "custom success modifier {} uses unsupported expression fields",
                m.name
            );
            return chance::Input {
                raw: Err(reason.clone()),
                skip: Err(reason.clone()),
                defender_can_affect: Err(reason),
            };
        }
        let skip = if m.name == "resistance" {
            self.skip_resistance()
        } else {
            Ok(false)
        };
        let raw = if matches!(skip, Ok(true)) {
            Ok(0)
        } else {
            self.raw(&m.name, current)
        };
        chance::Input {
            raw,
            skip,
            defender_can_affect: Ok(matches!(
                m.name.as_str(),
                "air_superiority"
                    | "interception"
                    | "anti_air"
                    | "radar"
                    | "resistance"
                    | "enemy_units"
            )),
        }
    }
    fn skip_resistance(&self) -> Result<bool> {
        let Some(target) = target_country(self.world, self.scope)? else {
            return Ok(false);
        };
        for row in self.values(TARGET_CORES) {
            if self
                .world
                .slot(row.raw as u32)
                .ok_or("target core country unresolved")?
                == target
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn raw(&self, name: &str, current: impl FnOnce() -> Result<i64>) -> Result<i64> {
        let army = self.one(ARMY, 0x420).is_some();
        let wing = self.one(WING, 0x218).is_some();
        match name {
            "intel" => current(),
            "recon" => {
                if army {
                    self.required(ARMY_STATS, 0xd0)
                } else {
                    Ok(0)
                }
            }
            "organisation" => {
                if army {
                    self.required(ARMY, 0x428)
                } else {
                    Ok(0)
                }
            }
            "strength" => {
                if army {
                    Ok(ratio(
                        self.required(ARMY, 0x420)?,
                        self.required(ARMY_STATS, 0x288)?.max(100),
                    )?
                    .clamp(0, 100_000))
                } else {
                    Ok(0)
                }
            }
            "experience" => {
                if army {
                    let total = self
                        .values(MANPOWER)
                        .fold(0i32, |sum, r| sum.wrapping_add(r.raw as i32));
                    if total <= 0 {
                        Ok(0)
                    } else {
                        ratio(self.required(ARMY, 0x430)?, i64::from(total) * 100_000)
                    }
                } else if wing {
                    let max = *self.assets.tooltip.max_wing_xp.as_ref()?;
                    if max < 1 {
                        Ok(100_000)
                    } else {
                        ratio(self.required(WING, 0x218)?, max)
                    }
                } else {
                    Ok(0)
                }
            }
            "reliability" => {
                if wing && self.one(CONTEXT, TARGET_PROVINCE).is_some() {
                    self.required(WING_STATS, 0x41)
                } else {
                    Ok(0)
                }
            }
            "air_defence" | "air_agility" | "strategic_bomber" => {
                if wing && self.one(CONTEXT, TARGET_PROVINCE).is_some() {
                    self.air(name)
                } else {
                    Ok(0)
                }
            }
            "resistance" => Ok(self.one(TARGET_STATE, 0x278).unwrap_or(0)),
            "anti_air" => {
                let mut total = 0i32;
                for row in self.values(ANTI_AIR_LEVELS).filter(|r| r.raw > 0) {
                    let building = self.assets.building(self.world, row.key)?;
                    if let Some(reason) = &building.tooltip_refused {
                        return Err(reason.clone());
                    }
                    total = total.wrapping_add((row.raw as i32).wrapping_mul(building.air_defence));
                }
                Ok(i64::from(total) * 100_000)
            }
            "air_superiority" => {
                let Some(region) = self.one(CONTEXT, TARGET_REGION) else {
                    return Ok(0);
                };
                let frame = self.world.frame();
                let countries = frame
                    .complete::<tables::RaidAirScores>()
                    .map_err(|e| e.to_string())?;
                if !countries
                    .iter()
                    .any(|r| self.world.slot(r.country) == Some(self.raid.parent_slot))
                {
                    return Ok(0);
                }
                let (a, b) = intel::scores(self.world, self.raid.parent_slot, region as u32)?;
                intel::air_ratio(a, b)
            }
            "interception" => {
                let Some(region) = self.one(CONTEXT, TARGET_REGION) else {
                    return Ok(0);
                };
                let mut total = 0i64;
                for row in self.values(INTERCEPTION_ENABLED) {
                    let supported = self.required(INTERCEPTION_SUPPORTED, row.key)?;
                    if intercepts(row.raw as u32, supported as u32)
                        && self.one(INTERCEPTION_REGION, row.key) == Some(region)
                    {
                        total = total.wrapping_add(
                            self.required(INTERCEPTION_PLANES, row.key)?
                                .wrapping_mul(100_000),
                        );
                    }
                }
                Ok(total)
            }
            "enemy_units" => {
                let raw = self.required(CONTEXT, TARGET_COUNTRY)? as u32;
                let target = if raw == 0 {
                    None
                } else {
                    Some(self.world.slot(raw).ok_or("target owner unresolved")?)
                };
                let mut total = 0i64;
                for row in self.values(TARGET_UNIT_OWNERS) {
                    let owner = if row.raw == 0 {
                        None
                    } else {
                        Some(
                            self.world
                                .slot(row.raw as u32)
                                .ok_or("target unit owner unresolved")?,
                        )
                    };
                    if owner == target {
                        total = total.wrapping_add(100_000);
                    }
                }
                Ok(total)
            }
            "radar" => {
                let Some(state) = intel::target_state(self.world, self.scope)? else {
                    return Ok(0);
                };
                let Some(target) = target_country(self.world, self.scope)? else {
                    return Ok(0);
                };
                let rows = self
                    .world
                    .frame()
                    .complete::<tables::RadarCoverage>()
                    .map_err(|e| e.to_string())?;
                Ok(
                    if rows.iter().any(|r| {
                        self.world.slot(r.country) == Some(target) && r.state == Some(state)
                    }) {
                        100_000
                    } else {
                        0
                    },
                )
            }
            _ => Err(format!("unsupported success modifier {name}")),
        }
    }
    fn ace_scale(&self) -> Result<i64> {
        let d = self.assets.tooltip.speed.as_ref()?;
        Ok(self.one(ACE_WING, 0x7c).map_or(100_000, |planes| {
            unit_inputs::ace_wing_scale(planes, d.ace_wing_size, d.ace_wing_size_max_bonus)
        }))
    }
    fn ace(&self, key: u32) -> Result<i64> {
        if self.one(ACE, 0x158).is_none_or(|alive| alive == 0) {
            return Ok(0);
        }
        Ok(times(
            times(self.modifier(ACE_MODIFIERS, key)?, self.ace_scale()?),
            100_000i64
                .wrapping_add(self.modifier(ACE_OWNER_MODIFIERS, keys::ACE_EFFECTIVENESS_FACTOR)?),
        ))
    }
    fn air(&self, name: &str) -> Result<i64> {
        replay_derive::fixed_layout::validate_recording(
            self.world.frame(),
            replay_derive::fixed_layout::RAID_SUCCESS,
        )?;
        let bombing = name == "strategic_bomber";
        let key = if bombing {
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR
        } else if name == "air_defence" {
            keys::AIR_DEFENCE_FACTOR
        } else {
            keys::AIR_AGILITY_FACTOR
        };
        let base = if bombing {
            self.required(WING_STATS, 0x32)?
                .wrapping_add(self.ace(key)?)
        } else {
            self.required(
                WING_STATS,
                if key == keys::AIR_DEFENCE_FACTOR {
                    0x2f
                } else {
                    0x31
                },
            )?
        };
        let mut factor = 100_000i64;
        if bombing {
            factor = factor.wrapping_add(self.modifier(WING_COUNTRY_MODIFIERS, key)?);
        } else {
            for kind in [WING_MODIFIERS, WING_COUNTRY_MODIFIERS] {
                for row in self.values(kind).filter(|r| r.key == key) {
                    factor = factor.wrapping_add(row.raw);
                }
            }
            if self.one(ACE, 0x158).is_some_and(|alive| alive != 0) {
                let scale = self.ace_scale()?;
                for row in self.values(ACE_MODIFIERS).filter(|r| r.key == key) {
                    factor = factor.wrapping_add(times(scale, row.raw));
                }
                for _ in self
                    .values(ACE_TYPE_MODIFIERS)
                    .filter(|r| r.key == key && r.raw > 0)
                {
                    factor = factor.wrapping_add(
                        self.modifier(WING_COUNTRY_MODIFIERS, keys::AIR_ACE_BONUSES_FACTOR)?,
                    );
                }
            }
        }
        if self.one(CONTEXT, WING_REGION).is_some() {
            if key == keys::AIR_DEFENCE_FACTOR {
                factor = factor.wrapping_add(self.modifier(
                    WING_COUNTRY_MODIFIERS,
                    keys::AIR_STRATEGIC_BOMBER_DEFENCE_FACTOR,
                )?);
            }
            let weather = 100_000i64
                .wrapping_add(self.modifier(WING_COUNTRY_MODIFIERS, keys::AIR_WEATHER_PENALTY)?)
                .wrapping_add(self.modifier(WING_MODIFIERS, keys::AIR_WEATHER_PENALTY)?)
                .wrapping_add(self.ace(keys::AIR_WEATHER_PENALTY)?)
                .clamp(0, 200_000);
            factor = factor.wrapping_add(times(self.modifier(WEATHER_MODIFIERS, key)?, weather));
            if self.required(CONTEXT, NIGHT_FACTOR)?
                > *self.assets.tooltip.night_threshold.as_ref()?
            {
                let mut night = 100_000i64
                    .wrapping_add(self.modifier(WING_COUNTRY_MODIFIERS, keys::AIR_NIGHT_PENALTY)?)
                    .wrapping_add(self.modifier(WING_MODIFIERS, keys::AIR_NIGHT_PENALTY)?)
                    .wrapping_add(self.required(WING_STATS, 0x4c)?)
                    .wrapping_add(self.ace(keys::AIR_NIGHT_PENALTY)?);
                if self.required(CONTEXT, WING_MISSION_MASK)? & 8 != 0 {
                    night = night.wrapping_add(self.modifier(
                        WING_COUNTRY_MODIFIERS,
                        keys::AIR_STRATEGIC_BOMBER_NIGHT_PENALTY,
                    )?);
                }
                if self.one(CONTEXT, CARRIER_MISSION).is_some_and(|v| v != 0) {
                    night = night.wrapping_sub(self.modifier(
                        WING_COUNTRY_MODIFIERS,
                        keys::AIR_CARRIER_NIGHT_PENALTY_REDUCTION_FACTOR,
                    )?);
                }
                factor = factor.wrapping_add(times(
                    self.modifier(NIGHT_MODIFIERS, key)?,
                    night.clamp(0, 200_000),
                ));
            }
            if key == keys::AIR_DEFENCE_FACTOR {
                let province = self.required(CONTEXT, TARGET_PROVINCE)? as u32;
                let target = *self
                    .assets
                    .regions
                    .get(&province)
                    .ok_or("target province has no strategic region")?;
                let base = self.one(CONTEXT, BASE_REGION).unwrap_or(0xaa) as u32;
                if target != 0 && base != 0 {
                    factor = factor.wrapping_add(self.modifier(
                        WING_COUNTRY_MODIFIERS,
                        if target == base {
                            keys::AIR_HOME_DEFENCE_FACTOR
                        } else {
                            keys::AIR_POWER_PROJECTION_FACTOR
                        },
                    )?);
                }
            }
        }
        Ok(times(factor, base))
    }
}

fn intercepts(enabled: u32, supported: u32) -> bool {
    let both = enabled & supported;
    both & 1 == 0 && both & 4 != 0
}

#[cfg(test)]
mod tests {
    use super::super::super::test_support::{assets, frame, put};
    use super::*;
    #[test]
    fn recorded_army_numerators_manpower_occurrences_and_missing_witnesses() {
        let assets = assets();
        let mut frame = frame();
        let raid = RaidRow {
            id: 7,
            reference_type: 91,
            unit: 35,
            ..Default::default()
        };
        let key = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let mut rows = Vec::new();
        for (kind, key_word, raw) in [
            (REFERENCES, 0, 35),
            (REFERENCES, 1, 0),
            (ARMY, 0x420, 25_000),
            (ARMY, 0x428, 321_987),
            (ARMY, 0x430, 300_000),
            (ARMY_STATS, 0x288, 100_000),
            (ARMY_STATS, 0xd0, 777),
            (MANPOWER, 0, 8),
            (MANPOWER, 1, -2),
            (MANPOWER, 2, 4),
        ] {
            rows.push(RaidTooltipOperandRow {
                raid: key,
                ordinal: rows.len() as u32,
                kind,
                key: key_word,
                raw,
            });
        }
        put::<tables::RaidTooltipOperands>(&mut frame, rows.clone());
        let scope = RaidScope::default();
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        let inputs = Inputs::new(&assets, &world, &raid, &scope).unwrap();
        for (name, expected) in [
            ("strength", 25_000),
            ("organisation", 321_987),
            ("experience", 30_000),
            ("recon", 777),
            ("reliability", 0),
        ] {
            assert_eq!(
                inputs.raw(name, || unreachable!()).unwrap(),
                expected,
                "{name}"
            );
        }
        rows[0].raw = 99;
        put::<tables::RaidTooltipOperands>(&mut frame, rows);
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        assert!(Inputs::new(&assets, &world, &raid, &scope).is_err());
        put::<tables::RaidTooltipOperands>(
            &mut frame,
            vec![RaidTooltipOperandRow {
                raid: key,
                kind: UNREADABLE,
                ..Default::default()
            }],
        );
        let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
        assert_eq!(
            Inputs::new(&assets, &world, &raid, &scope).err().as_deref(),
            Some("raid operands were unreadable when recorded")
        );
    }
    #[test]
    fn enemy_unit_count_preserves_duplicate_occurrences_and_null_equality() {
        let assets = assets();
        let mut frame = frame();
        let raid = RaidRow {
            id: 7,
            reference_type: 91,
            ..Default::default()
        };
        let key = u64::from(raid.id) << 32 | u64::from(raid.reference_type);
        let scope = RaidScope::default();
        for (target, expected) in [(0, 200_000), (10, 200_000), (11, 100_000)] {
            let mut rows = vec![
                (REFERENCES, 0, 0),
                (REFERENCES, 1, 0),
                (CONTEXT, TARGET_COUNTRY, target),
            ];
            rows.extend(
                [0, 10, 10, 11, 0]
                    .into_iter()
                    .enumerate()
                    .map(|(i, c)| (TARGET_UNIT_OWNERS, i as u32, c)),
            );
            put::<tables::RaidTooltipOperands>(
                &mut frame,
                rows.into_iter()
                    .enumerate()
                    .map(|(ordinal, (kind, key_word, raw))| RaidTooltipOperandRow {
                        raid: key,
                        ordinal: ordinal as u32,
                        kind,
                        key: key_word,
                        raw,
                    })
                    .collect(),
            );
            let world = World::new(&frame, assets.trigger_content.facts()).unwrap();
            let inputs = Inputs::new(&assets, &world, &raid, &scope).unwrap();
            assert_eq!(
                inputs.raw("enemy_units", || unreachable!()).unwrap(),
                expected
            );
        }
    }
    #[test]
    fn interception_priority_and_support_are_independent() {
        for enabled in 0..16 {
            for supported in 0..16 {
                assert_eq!(
                    intercepts(enabled, supported),
                    enabled & 4 != 0
                        && supported & 4 != 0
                        && !(enabled & 1 != 0 && supported & 1 != 0)
                );
            }
        }
    }
    #[test]
    fn native_division_retains_sentinels_and_truncates() {
        assert_eq!(ratio(1, 3), Ok(33_333));
        assert_eq!(ratio(-1, 3), Ok(-33_333));
        assert_eq!(ratio(10, 0), Ok(0xffff_ffff));
        assert_eq!(ratio(i64::MAX, 3), Ok(0xffff_ffff));
    }
}
