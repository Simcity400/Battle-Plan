use crate::{Report, complete, invalid, required_country_tag};
use engine_land_inputs::combat_log_events as kind;
use installed_content_save::{
    Save,
    combat_log::{Activity, BattleSide, Observer, ObserverDamage},
    production::Pool,
};
use replay_data::rows::CombatLogObserverRow;
use replay_data::rows::CountryRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};
fn append<T>(rows: &mut Vec<T>, ordinal: u64, value: T) -> io::Result<()> {
    if ordinal != rows.len() as u64 {
        return Err(invalid("active combat log ordinal differs"));
    }
    rows.push(value);
    Ok(())
}
fn decimal(raw: i64) -> io::Result<installed_content_save::Decimal> {
    installed_content_save::Decimal::parse(
        format!(
            "{}{}.{:05}",
            if raw < 0 { "-" } else { "" },
            raw.unsigned_abs() / 100_000,
            raw.unsigned_abs() % 100_000
        )
        .as_bytes(),
    )
}
fn empty_side() -> BattleSide {
    BattleSide {
        manpower: None,
        air_factor: None,
        equipment: None,
        captured_by_enemy: None,
        recovered: None,
        modifiers: Vec::new(),
        tags: Vec::new(),
        leader: None,
    }
}
fn decode(
    rows: &[&CombatLogObserverRow],
    manpower: i32,
    equipment: &[replay_data::rows::LandEquipmentLoss],
    links: &BTreeMap<u32, &CountryRow>,
) -> io::Result<Observer> {
    let mut result = Observer {
        side: empty_side(),
        activities: Vec::new(),
        leaders: Vec::new(),
        damage: Vec::new(),
        total_damage: None,
        progress: None,
        counters: Vec::new(),
        snow: None,
        victory: None,
    };
    let mut begun = false;
    for r in rows {
        let (kind, item, [a, b, c, d, e]) = r.fact();
        let tag = |raw: u64| -> io::Result<Vec<u8>> {
            Ok(required_country_tag(raw as u32, links)?.to_vec())
        };
        match kind as u64 {
            kind::OBSERVER_STATE => {
                if begun {
                    return Err(invalid("active observer header repeated"));
                }
                begun = true;
                result.total_damage = (a as i64 > 0).then(|| decimal(a as i64)).transpose()?;
                result.progress = (b as i64 > 0).then(|| decimal(b as i64)).transpose()?;
                result.snow = (c & 1 != 0).then_some(true);
                result.victory = (c & 2 != 0).then_some(true);
            }
            kind::OBSERVER_MODIFIERS => {
                for (i, v) in [a, b, c, d, e].into_iter().enumerate() {
                    append(&mut result.counters, item + i as u64, v as i32)?;
                }
            }
            kind::SIDE => {
                result.side.manpower = (manpower > 0).then_some(manpower);
                result.side.air_factor = (b as i64 > 0).then(|| decimal(b as i64)).transpose()?;
                result.side.leader = (c != 0).then_some(c);
            }
            kind::POOL => {
                let pool = match item {
                    0 => &mut result.side.equipment,
                    1 => &mut result.side.captured_by_enemy,
                    2 => &mut result.side.recovered,
                    _ => return Err(invalid("active observer pool kind unsupported")),
                };
                *pool = if a != 0 {
                    Some(Pool {
                        equipment: Vec::new(),
                        allow_zero_entries: Some(b != 0),
                    })
                } else {
                    None
                };
                if item == 0
                    && let Some(pool) = pool
                {
                    for e in equipment {
                        if b != 0 || e.amount_raw != 0 {
                            pool.equipment.push(installed_content_save::production::Equipment {
                                variant: e.variant,
                                amount: decimal(e.amount_raw)?,
                            });
                        }
                    }
                }
            }
            kind::EQUIPMENT => {
                let pool = match item {
                    1 => result.side.captured_by_enemy.as_mut(),
                    2 => result.side.recovered.as_mut(),
                    _ => None,
                }
                .ok_or_else(|| invalid("active observer amount lacks serialized pool"))?;
                if pool.allow_zero_entries == Some(true) || c != 0 {
                    pool.equipment.push(installed_content_save::production::Equipment {
                        variant: b,
                        amount: decimal(c as i64)?,
                    });
                }
            }
            kind::MODIFIER => append(&mut result.side.modifiers, a, (b as i32, c as i32))?,
            kind::COUNTRY => append(&mut result.side.tags, a, tag(b)?)?,
            kind::OBSERVER_LEADER => append(&mut result.leaders, item, (a, b as i32))?,
            kind::OBSERVER_DAMAGE => append(
                &mut result.damage,
                item,
                ObserverDamage {
                    from: tag(a)?,
                    receiver: tag(b)?,
                    to: tag(c)?,
                    value: decimal(d as i64)?,
                },
            )?,
            kind::ACTIVITY => append(
                &mut result.activities,
                item,
                Activity {
                    group: a,
                    templates: Vec::new(),
                    units: Vec::new(),
                    strength: Vec::new(),
                    equipment: None,
                    dealer: if b == 0 { None } else { Some(tag(b)?) },
                    taker: if c == 0 { None } else { Some(tag(c)?) },
                },
            )?,
            kind::ACTIVITY_TEMPLATE => {
                let activity = result
                    .activities
                    .get_mut(item as usize)
                    .ok_or_else(|| invalid("active template lacks activity"))?;
                append(&mut activity.templates, a, b)?;
            }
            kind::ACTIVITY_DAMAGE => {
                let activity = result
                    .activities
                    .get_mut(item as usize)
                    .ok_or_else(|| invalid("active damage lacks activity"))?;
                append(&mut activity.units, a, b)?;
                append(&mut activity.strength, a, decimal(c as i64)?)?;
            }
            kind::ACTIVITY_EQUIPMENT => {
                let activity = result
                    .activities
                    .get_mut(item as usize)
                    .ok_or_else(|| invalid("active equipment lacks activity"))?;
                if a == u64::MAX {
                    activity.equipment = (b != 0).then(|| Pool {
                        equipment: Vec::new(),
                        allow_zero_entries: Some(c != 0),
                    });
                } else {
                    let pool = activity
                        .equipment
                        .as_mut()
                        .ok_or_else(|| invalid("active activity amount lacks pool"))?;
                    if pool.allow_zero_entries == Some(true) || c != 0 {
                        pool.equipment.push(installed_content_save::production::Equipment {
                            variant: b,
                            amount: decimal(c as i64)?,
                        });
                    }
                }
            }
            _ => return Err(invalid("active combat log entry kind unsupported")),
        }
    }
    if !begun || result.counters.len() != 30 {
        return Err(invalid("active combat log header or counters incomplete"));
    }
    Ok(result)
}

fn pools_equal(actual: Option<&Pool>, saved: Option<&Pool>) -> bool {
    match (actual, saved) {
        (None, None) => true,
        (Some(actual), Some(saved)) => {
            let Pool {
                equipment,
                allow_zero_entries,
            } = actual;
            let values = |e: &installed_content_save::production::Equipment| {
                let installed_content_save::production::Equipment { variant, amount } = e;
                (*variant, amount.raw)
            };
            *allow_zero_entries == saved.allow_zero_entries
                && equipment
                    .iter()
                    .map(values)
                    .eq(saved.equipment.iter().map(values))
        }
        _ => false,
    }
}

fn sides_equal(actual: &BattleSide, saved: &BattleSide) -> bool {
    let BattleSide {
        manpower,
        air_factor,
        equipment,
        captured_by_enemy,
        recovered,
        modifiers,
        tags,
        leader,
    } = actual;
    *manpower == saved.manpower
        && air_factor.as_ref().map(|v| v.raw) == saved.air_factor.as_ref().map(|v| v.raw)
        && pools_equal(equipment.as_ref(), saved.equipment.as_ref())
        && pools_equal(captured_by_enemy.as_ref(), saved.captured_by_enemy.as_ref())
        && pools_equal(recovered.as_ref(), saved.recovered.as_ref())
        && *modifiers == saved.modifiers
        && *tags == saved.tags
        && *leader == saved.leader
}

fn activities_equal(actual: &Activity, saved: &Activity) -> bool {
    let Activity {
        group,
        templates,
        units,
        strength,
        equipment,
        dealer,
        taker,
    } = actual;
    *group == saved.group
        && *templates == saved.templates
        && *units == saved.units
        && strength
            .iter()
            .map(|v| v.raw)
            .eq(saved.strength.iter().map(|v| v.raw))
        && pools_equal(equipment.as_ref(), saved.equipment.as_ref())
        && *dealer == saved.dealer
        && *taker == saved.taker
}

fn observers_equal(actual: &Observer, saved: &Observer) -> bool {
    let Observer {
        side,
        activities,
        leaders,
        damage,
        total_damage,
        progress,
        counters,
        snow,
        victory,
    } = actual;
    sides_equal(side, &saved.side)
        && activities.len() == saved.activities.len()
        && activities
            .iter()
            .zip(&saved.activities)
            .all(|(a, s)| activities_equal(a, s))
        && *leaders == saved.leaders
        && damage.len() == saved.damage.len()
        && damage.iter().zip(&saved.damage).all(|(actual, saved)| {
            let ObserverDamage {
                from,
                receiver,
                to,
                value,
            } = actual;
            *from == saved.from
                && *receiver == saved.receiver
                && *to == saved.to
                && value.raw == saved.value.raw
        })
        && total_damage.as_ref().map(|v| v.raw) == saved.total_damage.as_ref().map(|v| v.raw)
        && progress.as_ref().map(|v| v.raw) == saved.progress.as_ref().map(|v| v.raw)
        && *counters == saved.counters
        && *snow == saved.snow
        && *victory == saved.victory
}

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut groups: BTreeMap<(u64, bool), Vec<&CombatLogObserverRow>> = BTreeMap::new();
    for row in complete::<tables::CombatLogObservers>(frame)? {
        groups
            .entry((row.combat, row.attacker))
            .or_default()
            .push(row);
    }
    let progress = complete::<tables::LandCombatProgress>(frame)?;
    let mut count = 0;
    for combat in saved.land_combats.values() {
        for (side, saved_side) in combat.details.sides.iter().enumerate() {
            let key = (combat.id, side == 0);
            let mut rows = groups.remove(&key).unwrap_or_default();
            rows.sort_unstable_by_key(|r| r.logical_order());
            let p = progress
                .iter()
                .find(|r| r.combat == combat.id)
                .ok_or_else(|| invalid("active observer lacks loss census"))?;
            let (manpower, equipment) = if side == 0 {
                (&p.attacker_manpower_loss, &p.attacker_equipment_losses)
            } else {
                (&p.defender_manpower_loss, &p.defender_equipment_losses)
            };
            let manpower = *manpower.require("observer manpower").map_err(invalid)?;
            let equipment = equipment.require("observer equipment").map_err(invalid)?;
            let actual = decode(&rows, manpower, equipment, links)?;
            if saved_side
                .log
                .as_ref()
                .is_none_or(|saved| !observers_equal(&actual, saved))
            {
                report.difference(
                    format!("CombatLogObservers combat {} side {}", combat.id, side),
                    actual,
                    &saved_side.log,
                );
            }
            count += 1;
        }
    }
    for (key, _) in groups {
        report.difference(format!("CombatLogObservers {key:?}"), "present", "absent");
    }
    report.compared("CombatLogObservers", count);
    Ok(())
}

#[cfg(test)]
mod tests;
