use super::{Content, DecodedFrame, State, loc};
use dashboard_base::products::equipment_icon::{self, Icon, IconHour};
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{VariantDefinitionRow, VariantStatsRow, tables};
use replay_derive::air_region_derive::{
    self as derive, EquipmentOrder, EquipmentPool, EquipmentPools, Filter, Retained,
};
use std::collections::BTreeMap;
use dashboard_text::table_access::TokenNames;

pub(super) struct Sources<'a> {
    content: &'a Content,
    frame: &'a DecodedFrame,
    variants: Result<BTreeMap<u64, &'a VariantDefinitionRow>, String>,
    links: Result<BTreeMap<u32, u32>, String>,
    tokens: Result<TokenNames<'a>, String>,
    names: crate::naval_variants::Names<'a>,
}

impl<'a> Sources<'a> {
    pub fn new(content: &'a Content, frame: &'a DecodedFrame) -> Self {
        Self {
            content,
            frame,
            variants: frame
                .complete::<tables::VariantDefinitions>()
                .map(|rows| rows.iter().map(|row| (row.id, row)).collect())
                .map_err(|e| e.to_string()),
            links: frame
                .complete::<tables::CountryLinks>()
                .map(|rows| {
                    rows.iter()
                        .map(|row| (row.id, row.canonical_slot))
                        .collect()
                })
                .map_err(|e| e.to_string()),
            tokens: TokenNames::new(frame).map_err(|e| e.to_string()),
            names: crate::naval_variants::Names::new(content, frame),
        }
    }

    fn variant(&self, id: u64) -> Result<&'a VariantDefinitionRow, String> {
        self.variants
            .as_ref()
            .map_err(Clone::clone)?
            .get(&id)
            .copied()
            .ok_or_else(|| format!("retained aircraft variant {id} has no recorded definition"))
    }

    fn definition(
        &self,
        variant: &VariantDefinitionRow,
    ) -> Result<(usize, &content_reader_key_names::equipment::Database), String> {
        let database = self
            .content
            .assets
            .naval_air_groups
            .equipment()
            .ok_or("air combat equipment definitions are unavailable")?;
        let key = self
            .tokens
            .as_ref()
            .map_err(Clone::clone)?
            .name(variant.equipment_token)
            .map_err(|e| e.to_string())?;
        let at = database.get(key).ok_or_else(|| {
            format!("retained aircraft equipment {key} is absent from the content stack")
        })?;
        Ok((at, database))
    }

    fn order(&self, id: u64) -> Result<EquipmentOrder, String> {
        let variant = self.variant(id)?;
        let (equipment, database) = self.definition(variant)?;
        let slot = |raw| {
            if raw == 0 {
                Ok(0)
            } else {
                self.links
                    .as_ref()
                    .map_err(Clone::clone)?
                    .get(&raw)
                    .copied()
                    .ok_or_else(|| format!("aircraft variant country {raw} is unresolved"))
            }
        };
        Ok(EquipmentOrder {
            archetype: database.equipment[equipment]
                .archetype
                .ok_or("retained aircraft equipment has no archetype")?,
            version: *variant.version.require("VariantDefinitions.version")?,
            equipment,
            creator: slot(variant.creator_link)?,
            origin: slot(variant.origin_link)?,
        })
    }

    pub fn pools(
        &self,
        retained: Retained<'_>,
        enemy: bool,
        missions: u32,
        classification: u8,
    ) -> Result<EquipmentPools, String> {
        derive::equipment(
            retained,
            self.frame.hour,
            enemy,
            Filter {
                missions,
                classification,
                window: derive::LAST_DAY,
            },
            |id| self.order(id),
        )
    }

    pub fn pictures(
        &self,
        pools: &EquipmentPools,
        fighter: bool,
    ) -> Result<Vec<Option<String>>, String> {
        if pools.iter().all(|pool| pool.variants.is_empty()) {
            return Ok(vec![None; if fighter { 2 } else { 3 }]);
        }
        let icons = self
            .content
            .assets
            .air_icons
            .get()
            .ok_or("air combat equipment artwork was not prepared")?
            .as_ref()
            .map_err(Clone::clone)?;
        let hour = IconHour::read(self.frame, icons).map_err(|e| e.to_string())?;
        let mut ranked = [Vec::new(), Vec::new()];
        for (index, pool) in pools.iter().enumerate() {
            for &(id, _) in &pool.variants {
                let variant = self.variant(id)?;
                let (at, database) = self.definition(variant)?;
                let key = self
                    .tokens
                    .as_ref()
                    .map_err(Clone::clone)?
                    .name(variant.equipment_token)
                    .map_err(|e| e.to_string())?;
                ranked[index].push((database.equipment[at].model_priority, variant, key));
            }
            ranked[index].sort_by(|a, b| b.0.cmp(&a.0));
        }
        (0..if fighter { 2 } else { 3 })
            .map(|index| {
                let rank = if fighter { 0 } else { index };
                for pool in &ranked {
                    if let Some((_, variant, key)) =
                        pool.get(rank.min(pool.len().saturating_sub(1)))
                    {
                        match equipment_icon::variant_icon(icons, &hour, variant, key)? {
                            Some(Icon::Sprite(sprite)) => return Ok(Some(sprite)),
                            Some(Icon::Path(path)) => {
                                return Ok(Some(format!("air_texture:{path}")));
                            }
                            None => {}
                        }
                    }
                }
                Ok(None)
            })
            .collect()
    }

    fn list(&self, pool: &EquipmentPool) -> Result<String, String> {
        let mut text = String::new();
        for &(id, amount) in &pool.variants {
            if amount == 0 {
                continue;
            }
            let name = crate::naval_variants::name(
                self.variant(id)?,
                self.content.assets.as_ref(),
                &self.content.localisation,
                Some(&self.names),
                Some(&self.content.recorded_language),
            )?;
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&loc(
                self.content,
                "ACTUAL_EQUIPMENT_ENTRY",
                &[
                    ("NUM", Argument::Fixed5(amount)),
                    ("NAME", Argument::Text(&name)),
                ],
            )?);
        }
        Ok(text)
    }

    pub fn tooltip(
        &self,
        retained: Retained<'_>,
        enemy: bool,
        fighter: bool,
        selected_enemy: bool,
    ) -> Result<String, String> {
        let (missions, key) = if fighter {
            (1, "AIRWING_MISSION_TYPE_AIR_SUPERIORITY")
        } else if enemy == selected_enemy {
            (derive::DEFAULT_MISSIONS, "AIR_VIEW_GROUND_MISSION")
        } else {
            (5, "AIRWING_MISSION_TYPE_INTERCEPTION")
        };
        let title = loc(self.content, key, &[])?;
        let mut text = loc(self.content, "HEADER", &[("KEY", Argument::Text(&title))])?;
        let pools = self.pools(retained, enemy, missions, u8::from(!fighter))?;
        text.push_str(&self.list(&pools[0])?);
        newlines(&mut text, 1);
        text.push_str(&self.list(&pools[1])?);
        Ok(text)
    }

    fn average(&self, pool: &EquipmentPool, stat: usize) -> Result<i64, String> {
        if pool.variants.is_empty() {
            return Ok(0);
        }
        let stats = self
            .frame
            .complete::<tables::VariantStats>()
            .map_err(|e| e.to_string())?;
        let mut total = 0_i64;
        let mut amount = 0_i64;
        for &(id, count) in &pool.variants {
            let row = stats
                .binary_search_by_key(&id, |row| row.variant)
                .ok()
                .map(|at| &stats[at])
                .ok_or_else(|| format!("retained aircraft variant {id} has no recorded stats"))?;
            let value = row.stat(stat)?;
            amount = amount.wrapping_add(count);
            total = total.wrapping_add(count.wrapping_mul(value) / 100_000);
        }
        Ok(if amount <= 0 {
            0
        } else if (total as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
            total.wrapping_mul(100_000) / amount
        } else {
            0xffff_ffff
        })
    }

    fn comparison(
        &self,
        ours: &EquipmentPool,
        enemy: &EquipmentPool,
        attack: bool,
    ) -> Result<String, String> {
        let mut text = String::new();
        for (key, our_stat, enemy_stat) in [
            (
                if attack { "ATTACK" } else { "DEFENSE" },
                if attack {
                    VariantStatsRow::AIR_ATTACK
                } else {
                    VariantStatsRow::AIR_DEFENCE
                },
                if attack {
                    VariantStatsRow::AIR_DEFENCE
                } else {
                    VariantStatsRow::AIR_ATTACK
                },
            ),
            (
                "AGILITY",
                VariantStatsRow::AIR_AGILITY,
                VariantStatsRow::AIR_AGILITY,
            ),
            (
                "SPEED",
                VariantStatsRow::MAXIMUM_SPEED,
                VariantStatsRow::MAXIMUM_SPEED,
            ),
        ] {
            let ours = self.average(ours, our_stat)?;
            let enemy = self.average(enemy, enemy_stat)?;
            let comparison = if ours > enemy {
                "HIGHER"
            } else if ours < enemy {
                "LOWER"
            } else {
                "EQUAL"
            };
            text.push_str(&loc(
                self.content,
                &format!("AIR_VIEW_AIR_{key}_{comparison}"),
                &[],
            )?);
            text.push('\n');
        }
        Ok(text)
    }

    pub fn damage_tooltip(
        &self,
        mut text: String,
        retained: Retained<'_>,
        state: &State,
        flag: u32,
    ) -> Result<String, String> {
        let ground = flag < 2;
        let missions = if ground {
            if state.enemy {
                [5, state.missions]
            } else {
                [state.missions, 5]
            }
        } else {
            [1, 1]
        };
        let classification = u8::from(ground);
        text.push_str("\n\n");
        for (side, key) in [
            "AIR_VIEW_INTERCEPTED_EN_ROUTE_BY_ENEMY_COUNT",
            "AIR_VIEW_INTERCEPTED_EN_ROUTE_BY_US_COUNT",
        ]
        .into_iter()
        .enumerate()
        {
            let received = derive::received(
                retained,
                self.frame.hour,
                side != 0,
                Filter {
                    missions: missions[side],
                    classification,
                    window: derive::LAST_DAY,
                },
            )
            .map_err(str::to_owned)?;
            let count = received.amounts[9];
            if count > 0 {
                newlines(&mut text, 1);
                text.push_str(&loc(
                    self.content,
                    key,
                    &[(
                        "COUNT",
                        Argument::Int32((count / 100_000 + i64::from(count % 100_000 != 0)) as i32),
                    )],
                )?);
            }
        }
        let ours = self.pools(retained, false, missions[0], classification)?;
        let enemy = self.pools(retained, true, missions[1], classification)?;
        newlines(&mut text, 2);
        text.push_str(&loc(self.content, "PLANES_IN_REGION_SUBTITLE", &[])?);
        let attack = flag & 1 != 0;
        text.push_str(&self.comparison(&ours[0], &enemy[0], attack)?);
        let (our_pool, enemy_pool) = if !attack && !ours[1].variants.is_empty() {
            (&ours[1], &enemy[0])
        } else if attack && !enemy[1].variants.is_empty() {
            (&ours[0], &enemy[1])
        } else {
            return Ok(text);
        };
        newlines(&mut text, 1);
        text.push_str(&loc(self.content, "PLANES_PASSING_THROUGH_REGION", &[])?);
        text.push_str(&self.comparison(our_pool, enemy_pool, attack)?);
        Ok(text)
    }
}

fn newlines(text: &mut String, count: usize) {
    if text.is_empty() {
        return;
    }
    let present = text.bytes().rev().take_while(|&byte| byte == b'\n').count();
    for _ in present..count {
        text.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_stat_lane_withholding_is_named_and_never_coerced_to_zero() {
        let mut row = VariantStatsRow {
            variant: 42,
            values: [Some(250_000); 10],
            ..Default::default()
        };
        assert_eq!(row.stat(VariantStatsRow::AIR_DEFENCE), Ok(250_000));
        row.values[VariantStatsRow::AIR_DEFENCE] = None;
        let error = row.stat(VariantStatsRow::AIR_DEFENCE).unwrap_err();
        assert!(error.contains("air_defence"), "{error}");
        assert!(error.contains("42"), "{error}");
    }
}
