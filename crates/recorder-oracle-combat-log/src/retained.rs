//! Exact serializer projections of retained history, including empty entries.
use crate::{Report, complete, invalid, required_country_tag};
use installed_content_save::{Save, production};
use replay_data::rows;
use replay_data::rows::CountryRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, fmt::Debug, io};
fn put(map: &mut BTreeMap<String, String>, key: String, value: impl Debug) -> io::Result<()> {
    if map.insert(key, format!("{value:?}")).is_some() {
        return Err(invalid("duplicate retained combat log identity"));
    }
    Ok(())
}
fn pool(pool: &production::Pool) -> (Option<bool>, Vec<(u64, i64)>) {
    (
        pool.allow_zero_entries,
        pool.equipment
            .iter()
            .map(|e| (e.variant, e.amount.raw))
            .collect(),
    )
}
fn finish(
    name: &str,
    mut actual: BTreeMap<String, String>,
    expected: BTreeMap<String, String>,
    report: &mut Report,
) {
    let count = expected.len();
    for (key, value) in expected {
        match actual.remove(&key) {
            Some(actual) if actual == value => (),
            actual => report.difference(format!("{name}.{key}"), actual, value),
        }
    }
    for (key, value) in actual {
        report.difference(format!("{name}.{key}"), value, "absent");
    }
    report.compared(name, count);
}
pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    // Managers serialize only while holding a group. Their country membership
    // therefore derives exactly from group rows, including empty group logs.
    let mut countries = BTreeMap::new();
    for row in complete::<tables::CombatLogGroups>(frame)? {
        countries.insert(
            String::from_utf8_lossy(required_country_tag(row.country, links)?).into_owned(),
            "()".into(),
        );
    }
    let mut expected_countries = BTreeMap::new();
    for country in &saved.combat_log.countries {
        put(
            &mut expected_countries,
            String::from_utf8_lossy(&country.tag).into_owned(),
            (),
        )?;
    }
    finish("CombatLogCountries", countries, expected_countries, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogGroups>(frame)? {
        let tag = required_country_tag(r.country, links)?;
        put(
            &mut actual,
            format!("{} group {}", String::from_utf8_lossy(tag), r.group),
            (r.ordinal,),
        )?;
    }
    for c in &saved.combat_log.countries {
        for (ordinal, g) in c.groups.iter().enumerate() {
            put(
                &mut expected,
                format!("{} group {}", String::from_utf8_lossy(&c.tag), g.group),
                (ordinal as u32,),
            )?;
        }
    }
    finish("CombatLogGroups", actual, expected, report);
    let tag_group = |country: u32, group: u32| -> io::Result<String> {
        Ok(format!(
            "{} group {}",
            String::from_utf8_lossy(required_country_tag(country, links)?),
            group
        ))
    };
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogMonths>(frame)? {
        put(
            &mut actual,
            format!(
                "{} kind {} ordinal {}",
                tag_group(r.country, r.group)?,
                r.kind,
                r.ordinal
            ),
            (r.reason, r.date, Some(r.allow_zero)),
        )?;
    }
    for c in &saved.combat_log.countries {
        for g in &c.groups {
            let mut ordinals = [0u32; 6];
            for (kind, m) in g
                .losses
                .iter()
                .map(|m| (m.reason, m))
                .chain(g.recovered.iter().map(|m| (4, m)))
                .chain(g.enemy.iter().map(|m| (5, m)))
            {
                let kind = usize::try_from(kind)
                    .ok()
                    .filter(|&i| i < 6)
                    .ok_or_else(|| invalid("saved combat log reason unsupported"))?;
                put(
                    &mut expected,
                    format!(
                        "{} group {} kind {} ordinal {}",
                        String::from_utf8_lossy(&c.tag),
                        g.group >> 32,
                        kind,
                        ordinals[kind]
                    ),
                    (m.reason, m.date, m.pool.allow_zero_entries),
                )?;
                ordinals[kind] += 1;
            }
        }
    }
    finish("CombatLogMonths", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogManpower>(frame)? {
        put(
            &mut actual,
            format!("{} ordinal {}", tag_group(r.country, r.group)?, r.ordinal),
            (r.reason, r.date, [r.losses_0, r.losses_1, r.losses_2]),
        )?;
    }
    for c in &saved.combat_log.countries {
        for g in &c.groups {
            for (ordinal, m) in g.manpower.iter().enumerate() {
                put(
                    &mut expected,
                    format!(
                        "{} group {} ordinal {}",
                        String::from_utf8_lossy(&c.tag),
                        g.group >> 32,
                        ordinal
                    ),
                    (m.reason, m.date, m.losses),
                )?;
            }
        }
    }
    finish("CombatLogManpower", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogTemplates>(frame)? {
        put(
            &mut actual,
            format!("{} ordinal {}", tag_group(r.country, r.group)?, r.ordinal),
            (r.template, r.date, r.wins, r.total),
        )?;
    }
    for c in &saved.combat_log.countries {
        for g in &c.groups {
            for (ordinal, t) in g.templates.iter().enumerate() {
                put(
                    &mut expected,
                    format!(
                        "{} group {} ordinal {}",
                        String::from_utf8_lossy(&c.tag),
                        g.group >> 32,
                        ordinal
                    ),
                    (t.template, t.date, t.wins, t.total),
                )?;
            }
        }
    }
    finish("CombatLogTemplates", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogReferences>(frame)? {
        put(
            &mut actual,
            format!("{} ordinal {}", tag_group(r.country, r.group)?, r.ordinal),
            (r.battle, r.attacker),
        )?;
    }
    for c in &saved.combat_log.countries {
        for g in &c.groups {
            for (ordinal, r) in g.battles.iter().enumerate() {
                put(
                    &mut expected,
                    format!(
                        "{} group {} ordinal {}",
                        String::from_utf8_lossy(&c.tag),
                        g.group >> 32,
                        ordinal
                    ),
                    (r.id, r.attacker),
                )?;
            }
        }
    }
    finish("CombatLogReferences", actual, expected, report);
    let months = complete::<tables::CombatLogMonths>(frame)?;
    let mut grouped: BTreeMap<(u32, u32, u32), Vec<&rows::CombatLogEnemyEquipmentRow>> =
        BTreeMap::new();
    for r in complete::<tables::CombatLogEnemyEquipment>(frame)? {
        grouped
            .entry((r.country, r.group, r.month))
            .or_default()
            .push(r);
    }
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for m in months.iter().filter(|m| m.kind == 5) {
        let mut entries = grouped
            .remove(&(m.country, m.group, m.ordinal))
            .unwrap_or_default();
        entries.sort_by_key(|r| r.ordinal);
        if entries
            .iter()
            .enumerate()
            .any(|(i, r)| r.ordinal != i as u32)
        {
            return Err(invalid("combat log enemy equipment ordinal gap"));
        }
        let equipment: Vec<_> = entries
            .into_iter()
            .filter(|r| m.allow_zero || r.amount_raw != 0)
            .map(|r| (r.variant, r.amount_raw))
            .collect();
        put(
            &mut actual,
            format!("{} month {}", tag_group(m.country, m.group)?, m.ordinal),
            (Some(m.allow_zero), equipment),
        )?;
    }
    if !grouped.is_empty() {
        return Err(invalid("combat log enemy amount lacks month membership"));
    }
    for c in &saved.combat_log.countries {
        for g in &c.groups {
            for (ordinal, m) in g.enemy.iter().enumerate() {
                put(
                    &mut expected,
                    format!(
                        "{} group {} month {}",
                        String::from_utf8_lossy(&c.tag),
                        g.group >> 32,
                        ordinal
                    ),
                    pool(&m.pool),
                )?;
            }
        }
    }
    finish("CombatLogEnemyEquipment", actual, expected, report);
    battles(frame, saved, links, report)
}
fn battles(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in complete::<tables::CombatLogBattles>(frame)? {
        put(
            &mut actual,
            format!("battle {}", r.id),
            (
                r.reference_count,
                r.date,
                (r.province != 0).then_some(r.province),
                (r.flags[0] != 0).then_some(true),
                (r.flags[1] != 0).then_some(true),
                (r.flags[2] != 0).then_some(true),
                r.flags[3] != 0,
            ),
        )?;
    }
    for r in &saved.combat_log.battles {
        put(
            &mut expected,
            format!("battle {}", r.id),
            (
                r.ref_count,
                r.date,
                r.province,
                r.defensive_victory,
                r.snow,
                r.overrun,
                r.player_is_attacker,
            ),
        )?;
    }
    finish("CombatLogBattles", actual, expected, report);
    let sides = complete::<tables::CombatLogBattleSides>(frame)?;
    let equipment = complete::<tables::CombatLogBattleEquipment>(frame)?;
    let modifiers = complete::<tables::CombatLogBattleModifiers>(frame)?;
    let countries = complete::<tables::CombatLogBattleCountries>(frame)?;
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for r in sides {
        if r.side > 1 {
            return Err(invalid("combat log battle side unsupported"));
        }
        put(
            &mut actual,
            format!("battle {} side {}", r.battle, r.side),
            (
                (r.manpower > 0).then_some(r.manpower),
                (r.air_factor_raw > 0).then_some(r.air_factor_raw),
                (r.leader != 0).then_some(r.leader),
            ),
        )?;
    }
    for b in &saved.combat_log.battles {
        for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
            put(
                &mut expected,
                format!("battle {} side {}", b.id, side),
                (s.manpower, s.air_factor.as_ref().map(|v| v.raw), s.leader),
            )?;
        }
    }
    finish("CombatLogBattleSides", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for s in sides {
        for kind in 0..3 {
            let present = s.pool_flags & (1 << (kind * 2)) != 0;
            let allow_zero = s.pool_flags & (2 << (kind * 2)) != 0;
            let mut entries: Vec<_> = equipment
                .iter()
                .filter(|r| r.battle == s.battle && r.side == s.side && r.kind == kind)
                .collect();
            entries.sort_by_key(|r| r.ordinal);
            if !present && !entries.is_empty() {
                return Err(invalid("omitted battle pool has amount rows"));
            }
            if entries
                .iter()
                .enumerate()
                .any(|(i, r)| r.ordinal != i as u32)
            {
                return Err(invalid("battle equipment ordinal gap"));
            }
            let projected = present.then(|| {
                (
                    Some(allow_zero),
                    entries
                        .into_iter()
                        .filter(|r| allow_zero || r.amount_raw != 0)
                        .map(|r| (r.variant, r.amount_raw))
                        .collect::<Vec<_>>(),
                )
            });
            put(
                &mut actual,
                format!("battle {} side {} pool {}", s.battle, s.side, kind),
                projected,
            )?;
        }
    }
    for b in &saved.combat_log.battles {
        for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
            for (kind, p) in [&s.equipment, &s.captured_by_enemy, &s.recovered]
                .into_iter()
                .enumerate()
            {
                put(
                    &mut expected,
                    format!("battle {} side {} pool {}", b.id, side, kind),
                    p.as_ref().map(pool),
                )?;
            }
        }
    }
    for e in equipment {
        if !sides
            .iter()
            .any(|s| s.battle == e.battle && s.side == e.side)
            || e.kind > 2
        {
            return Err(invalid("battle equipment has no side or pool"));
        }
    }
    finish("CombatLogBattleEquipment", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for m in modifiers {
        put(
            &mut actual,
            format!("battle {} side {} ordinal {}", m.battle, m.side, m.ordinal),
            (m.modifier, m.hours),
        )?;
    }
    for b in &saved.combat_log.battles {
        for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
            for (ordinal, m) in s.modifiers.iter().enumerate() {
                put(
                    &mut expected,
                    format!("battle {} side {} ordinal {}", b.id, side, ordinal),
                    m,
                )?;
            }
        }
    }
    finish("CombatLogBattleModifiers", actual, expected, report);
    let mut actual = BTreeMap::new();
    let mut expected = BTreeMap::new();
    for c in countries {
        put(
            &mut actual,
            format!("battle {} side {} ordinal {}", c.battle, c.side, c.ordinal),
            required_country_tag(c.country, links)?,
        )?;
    }
    for b in &saved.combat_log.battles {
        for (side, s) in [&b.attacker, &b.defender].into_iter().enumerate() {
            for (ordinal, tag) in s.tags.iter().enumerate() {
                put(
                    &mut expected,
                    format!("battle {} side {} ordinal {}", b.id, side, ordinal),
                    tag.as_slice(),
                )?;
            }
        }
    }
    finish("CombatLogBattleCountries", actual, expected, report);
    Ok(())
}
