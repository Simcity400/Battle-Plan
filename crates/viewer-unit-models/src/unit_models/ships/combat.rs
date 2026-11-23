
use super::*;
use viewer_unit_history::unit_history::naval_combat::{Battle, Health, Sources, impacts};
use replay_data::{rows::NavalCombatShipRow, rows::Recorded};
use std::sync::Arc;

impl UnitModelAssets {
    pub(super) fn prepare_naval_combats(
        &self,
        frame: &DecodedFrame,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        naval: &viewer_unit_history::unit_history::naval::Sources,
        hour: Option<&Hour<'_>>,
        report: &mut ModelReport,
    ) -> io::Result<Sources> {
        let mut out = Sources::default();
        let wanted: BTreeSet<_> = naval
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.class == viewer_runtime::unit_class::CounterClass::TaskForce
                    && matches!(candidate.slot, 9 | 10)
            })
            .map(|candidate| candidate.identity)
            .collect();
        if wanted.is_empty() {
            return Ok(out);
        }
        let motions = rows::<tables::TaskForceMapMotion>(frame)?;
        if !motions.is_complete() {
            report.error("naval_combat_references: TaskForceMapMotion is incomplete");
            return Ok(out);
        }
        let mut missing = wanted.clone();
        for row in motions
            .rows()
            .iter()
            .filter(|row| wanted.contains(&row.task_force))
        {
            missing.remove(&row.task_force);
            match row.combat_links {
                Recorded::Value(Some(links)) if links[1] != 0 => {
                    out.links.insert(row.task_force, links);
                }
                Recorded::Value(Some(_)) => report.error(
                    "naval_combat_references: a combat visual slot has no native type-2 combat",
                ),
                Recorded::Value(None) => report
                    .error("naval_combat_references: native combat references were unreadable"),
                Recorded::NotRecorded => report.error(
                    "naval_combat_references: ordered native combat references were not recorded",
                ),
            }
        }
        for _ in missing {
            report.error("naval_combat_references: task force has no motion row");
        }
        let content = match &self.ships.combat {
            Ok(content) => content,
            Err(reason) => {
                report.error(format!("naval_combat_content: {reason}"));
                return Ok(out);
            }
        };
        for province in naval
            .candidates
            .iter()
            .filter(|candidate| wanted.contains(&candidate.identity))
            .map(|candidate| candidate.province)
            .collect::<BTreeSet<_>>()
        {
            let Some(root) = anchor(province, 0) else {
                report.error("naval_combat_anchor: calling province has no slot-zero anchor");
                continue;
            };
            let Some(bounds) = self.bounds.get(&province) else {
                report.error("naval_combat_scale: calling province has no native bitmap bounds");
                continue;
            };
            out.roots.insert(
                province,
                ModelInstance {
                    position: root,
                    scale: (bounds[2] - bounds[0]).min(bounds[3] - bounds[1]) / 50.0
                        * content.scale,
                    rotation: self
                        .ships
                        .combat_rotations
                        .get(&province)
                        .copied()
                        .unwrap_or(0.0),
                    animation: viewer_models::entity_animation::named_request("idle"),
                    ..ModelInstance::default()
                },
            );
        }
        let Some(template_slot) = self.bake_slot_for_hour(&content.template_entity, hour) else {
            report.error("naval_combat_template: the recorded formation entity cannot bake");
            return Ok(out);
        };
        let (rig, palette) = {
            let library = self.library();
            let rig = library.rig(template_slot).clone();
            let palette = library.palette[rig.start..rig.end].to_vec();
            (Arc::new(rig), Arc::new(palette))
        };
        let locators = match combat_content::select_locators(&rig) {
            Ok(locators) => locators,
            Err(reason) => {
                report.error(format!("naval_combat_template: {reason}"));
                return Ok(out);
            }
        };
        let mut effects: [Vec<Option<(usize, u32)>>; 4] = Default::default();
        for (index, names) in content.effects.iter().enumerate() {
            if !matches!(index, 0 | 2) {
                continue;
            }
            for name in names {
                if self.roster.unreadable_names.contains_key(name) {
                    report.error(format!("naval_combat_effect: registry cardinality is undecided after an unreadable declaration of {name:?}"));
                    return Ok(out);
                }
                if !self.registered(name) {
                    continue;
                }
                if let Some(slot) = self.bake_slot_for_hour(name, hour) {
                    let library = self.library();
                    let default = library
                        .rig(slot)
                        .nodes
                        .first()
                        .map(|node| node.default.as_str())
                        .unwrap_or("idle");
                    effects[index].push(Some((
                        slot,
                        viewer_models::entity_animation::named_request(default),
                    )));
                } else {
                    report.error(format!(
                        "naval_combat_effect: registered effect {name:?} cannot bake"
                    ));
                    effects[index].push(None);
                }
            }
        }
        let members = rows::<tables::NavalCombatShips>(frame)?;
        let details = rows::<tables::NavalCombatDetails>(frame)?;
        let models = rows::<tables::ShipModels>(frame)?;
        let ships = rows::<tables::Ships>(frame)?;
        let forces = rows::<tables::TaskForces>(frame)?;
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        if !members.is_complete()
            || !details.is_complete()
            || !models.is_complete()
            || !ships.is_complete()
            || !forces.is_complete()
        {
            report.error("naval_combat_rosters: combat, ship or task-force census is incomplete");
            return Ok(out);
        }
        let model_by_ship: BTreeMap<_, _> =
            models.rows().iter().map(|row| (row.ship, row)).collect();
        let ship_by_id: BTreeMap<_, _> = ships.rows().iter().map(|row| (row.id, row)).collect();
        let force_by_id: BTreeMap<_, _> = forces.rows().iter().map(|row| (row.id, row)).collect();
        let variants: BTreeMap<_, _> = variants
            .rows()
            .iter()
            .map(|row| (row.id, &row.explicit_entity))
            .collect();
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let countries: BTreeMap<_, _> = core::<tables::Countries>(frame)?
            .iter()
            .map(|row| (row.canonical_slot, row))
            .collect();
        let wanted: BTreeSet<_> = out.links.values().map(|links| links[1]).collect();
        let mut by_combat: BTreeMap<u64, Vec<&NavalCombatShipRow>> = BTreeMap::new();
        for row in members.rows().iter().filter(|row| {
            wanted.contains(&row.combat)
                && row.member_source == 0
                && row.group_index != viewer_foundation::unit_rules::PAIRED_ONLY_GROUP
        }) {
            by_combat.entry(row.combat).or_default().push(row);
        }
        for rows in by_combat.values_mut() {
            rows.sort_by_key(|row| (row.side, row.group_index, row.member_index));
        }
        for detail in details
            .rows()
            .iter()
            .filter(|row| wanted.contains(&row.combat))
        {
            let root = [0.0; 3];
            let mut random = viewer_models::entity_animation::Random::new(
                detail.combat as u32 ^ (detail.combat >> 32) as u32,
            );
            let mut selected = Vec::with_capacity(8);
            for side in [0, 1] {
                selected.extend(
                    locators[side]
                        .choose(&mut || random.next_word())?
                        .map(|slot| slot.map(|slot| slot.name)),
                );
            }
            let selected: [Option<String>; 8] = selected.try_into().unwrap();
            let mut hulls: [Option<Target>; 8] = [None; 8];
            let mut offered = [false; 8];
            let mut impact_info: [Option<Arc<impacts::Info>>; 8] = std::array::from_fn(|_| None);
            let mut health = [Health::default(); 2];
            let mut refused = false;
            for (side, health) in health.iter_mut().enumerate() {
                let mut slots: [Option<&NavalCombatShipRow>; 4] = [None; 4];
                let mut counts = [0_i32; 2];
                let mut strength = [0_i64; 2];
                let mut sunk = 0_i32;
                for &row in by_combat
                    .get(&detail.combat)
                    .into_iter()
                    .flatten()
                    .filter(|row| usize::from(row.side) == 1 - side)
                {
                    if row.state & 0x20 != 0 {
                        sunk = sunk.wrapping_add(1);
                    }
                    if row.live_ship == 0 || row.state & 3 == 0 {
                        continue;
                    }
                    let own_capital_like =
                        row.formation_flags & ((1 << 6) | (1 << 22) | (1 << 37)) != 0;
                    let capital_like = if own_capital_like {
                        true
                    } else {
                        let Some(counter) = model_by_ship
                            .get(&row.ship)
                            .and_then(|model| model.counter.as_ref())
                        else {
                            refused = true;
                            break;
                        };
                        counter.positive_equipment_flags & ((1 << 22) | (1 << 37)) != 0
                    };
                    let group = usize::from(!capital_like);
                    if capital_like {
                        let held_capital =
                            slots[0].is_some_and(|held| held.formation_flags & (1 << 6) != 0);
                        if !held_capital {
                            slots[0] = Some(row);
                        }
                    } else if counts[1] < 3 {
                        slots[counts[1] as usize + 1] = Some(row);
                    }
                    counts[group] = counts[group].wrapping_add(1);
                    strength[group] = strength[group]
                        .wrapping_add(strength_ratio(row.strength, row.maximum_hit_points));
                }
                *health = Health {
                    capital: average(strength[0], counts[0]),
                    screens: average(strength[1], counts[1]),
                    sunk_fraction: if sunk > 0 {
                        sunk as f32 / sunk.wrapping_add(counts[1]) as f32
                    } else {
                        0.0
                    },
                };
                for (part, row) in slots.into_iter().enumerate() {
                    let Some(row) = row else { continue };
                    let index = side * 4 + part;
                    offered[index] = true;
                    let Some(model) = model_by_ship.get(&row.ship).copied() else {
                        refused = true;
                        break;
                    };
                    let graphics = match model.variant {
                        _ if model
                            .counter
                            .as_ref()
                            .is_some_and(|counter| counter.definition_valid == 0) =>
                        {
                            &[][..]
                        }
                        0 => &[][..],
                        id => match variants.get(&id) {
                            Some(bytes) => bytes.as_slice(),
                            None => {
                                refused = true;
                                break;
                            }
                        },
                    };
                    let tag = || {
                        let force = ship_by_id
                            .get(&row.ship)
                            .and_then(|ship| force_by_id.get(&ship.task_force))
                            .ok_or_else(|| {
                                "naval_combat_entity: fallback model task force is unavailable"
                                    .to_owned()
                            })?;
                        let owner =
                            model_owner(force.roster_owner, force.expeditionary_owner, true);
                        let country = links
                            .get(&owner)
                            .and_then(|slot| countries.get(slot))
                            .ok_or_else(|| {
                                "naval_combat_entity: fallback model country is unavailable"
                                    .to_owned()
                            })?;
                        content_reader_parser::parser::text(content_reader_parser::parser::c_string(&country.tag))
                            .map_err(|_| {
                                "naval_combat_entity: fallback model tag is not text".to_owned()
                            })
                    };
                    let definition = ship_by_id.get(&row.ship).and_then(|ship| {
                        force_by_id
                            .get(&ship.task_force)
                            .map(|force| (force.roster_owner, ship.class_id as u32))
                    });
                    let (slot, name) = match self.ship_slot(tag, model, graphics, definition, hour)
                    {
                        Ok(Some(selected)) => selected,
                        Ok(None) => {
                            report.error("naval_combat_entity: no registered ship entity");
                            continue;
                        }
                        Err(reason) => {
                            report.error(reason);
                            continue;
                        }
                    };
                    if selected[index].is_none() {
                        report
                            .error("naval_combat_locator: selected hull has no capital attachment");
                        continue;
                    }
                    hulls[index] =
                        Some(Target {
                            province: detail.province,
                            identity: detail.combat,
                            occupant: row.ship,
                            part: index as u32,
                            order: 9,
                            kind: Kind::Ship,
                            mesh: slot,
                            origin: root,
                            instance: ModelInstance {
                                position: root,
                                scale: 1.0,
                                animation: viewer_models::entity_animation::named_request(
                                    if side == 0 { "defend" } else { "attack" },
                                ),
                                animation_seed: row.ship as u32 ^ (row.ship >> 32) as u32,
                                native_flags: 8,
                                animation_phase: content.random_start_time,
                                ..ModelInstance::default()
                            },
                        });
                    let library = self.library();
                    let hull_rig = library.rig(slot).clone();
                    let hull_palette = library.palette[hull_rig.start..hull_rig.end].to_vec();
                    impact_info[index] = Some(Arc::new(impacts::Info::new(
                        Arc::new(hull_rig),
                        Arc::new(hull_palette),
                        if side == 0 { "defend" } else { "attack" },
                    )));
                    *report.by_entity.entry(name).or_default() += 1;
                }
            }
            if refused {
                report.error("naval_combat_roster_join: a reached ship's model, flags, task force, variant or model country is unavailable");
                continue;
            }
            report.ship_hulls += hulls.iter().flatten().count();
            out.battles.insert(
                detail.combat,
                Battle {
                    id: detail.combat,
                    stamp: detail.duration,
                    rig: Arc::clone(&rig),
                    palette: Arc::clone(&palette),
                    locators: selected,
                    hulls,
                    offered,
                    impacts: impact_info,
                    effects: effects.clone(),
                    health,
                    death_time: content.death_time,
                    root: Target {
                        province: detail.province,
                        identity: detail.combat,
                        occupant: detail.combat,
                        part: 8,
                        order: 9,
                        kind: Kind::Ship,
                        mesh: template_slot,
                        origin: root,
                        instance: ModelInstance {
                            position: root,
                            scale: 1.0,
                            animation: viewer_models::entity_animation::named_request("idle"),
                            animation_seed: detail.combat as u32 ^ (detail.combat >> 32) as u32,
                            ..ModelInstance::default()
                        },
                    },
                },
            );
        }
        for id in wanted {
            if !out.battles.contains_key(&id) {
                report.error(
                    "naval_combat_details: selected combat has no complete model projection",
                );
            }
        }
        Ok(out)
    }
}

fn exceptional(value: i64) -> bool {
    (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd
}
fn strength_ratio(strength: i64, maximum: i64) -> i64 {
    if exceptional(strength) {
        0xffff_ffff
    } else {
        strength
            .wrapping_mul(100_000)
            .wrapping_div(maximum.max(100))
    }
}
fn average(sum: i64, count: i32) -> i64 {
    if count <= 0 {
        sum
    } else if exceptional(sum) {
        0xffff_ffff
    } else {
        sum.wrapping_mul(100_000)
            .wrapping_div(i64::from(count) * 100_000)
    }
}

#[cfg(test)]
mod tests {
    use super::{average, strength_ratio};

    #[test]
    fn native_strength_floor_sentinels_and_wrapping_arithmetic() {
        assert_eq!(strength_ratio(1, 0), 1_000);
        assert_eq!(strength_ratio(-1, 100), -1_000);
        assert_eq!(strength_ratio(50, 200), 25_000);
        assert_eq!(strength_ratio(250, 100), 250_000);
        assert_eq!(strength_ratio((1_i64 << 62) + 1, 100), 1_000);
        for sentinel in [i64::MAX, i64::MIN, i64::MIN + 1] {
            assert_eq!(strength_ratio(sentinel, 100), 0xffff_ffff);
        }
    }

    #[test]
    fn native_average_retains_empty_count_and_signed_truncation() {
        assert_eq!(average(150_000, 2), 75_000);
        assert_eq!(average(-3, 2), -1);
        assert_eq!(average(1_i64 << 62, 2), 0);
        assert_eq!(average(i64::MIN, 0), i64::MIN);
        assert_eq!(average(17, -1), 17);
        assert_eq!(average(i64::MAX, 1), 0xffff_ffff);
    }
}
