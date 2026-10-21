//! Storage-only alias marks, independent of native rows and their delta masks.
use super::{complete, find, plain, update};
use crate::{
    Hour,
    codec::{Decoder, invalid_static as invalid, put_varint},
    frame::{
        DecodedFrame, Frame, SharedView, TableUpdate, TableView, advance_table, recorded::Updates,
    },
    rows::{self, LandCombatDivisionRow, Recorded, TableId, TableRows, tables},
};
use engine_allocation as allocation;
use std::io;

pub(crate) const TABLES: &[TableId] = &[
    TableId::DivisionMovement,
    TableId::DivisionStatus,
    TableId::CombatLogObservers,
    TableId::NavalCombatDetails,
    TableId::CountryFuel,
    TableId::LandCombatDivisions,
    TableId::LandCombatDivisionModifiers,
    TableId::Divisions,
    TableId::ProductionLines,
];

const MEMBER_FIELDS: usize = 12;
const MEMBER_MASK: u16 = (1 << MEMBER_FIELDS) - 1;

type Marks = Vec<(u128, u16)>;

pub(crate) fn stored(table: TableId, version: u16) -> bool {
    TABLES.contains(&table)
        && version
            >= match table {
                TableId::LandCombatDivisions
                | TableId::LandCombatDivisionModifiers
                | TableId::Divisions
                | TableId::ProductionLines => rows::compatibility::PRIVATE_COPY_ALIASES_FORMAT,
                _ => rows::compatibility::PRIVATE_ALIASES_FORMAT,
            }
}

fn dependencies(table: TableId) -> &'static [TableId] {
    match table {
        TableId::DivisionMovement => &[TableId::DivisionPanelStats],
        TableId::DivisionStatus => &[TableId::DivisionPanelStats, TableId::SimulationFeatures],
        TableId::LandCombatDivisions => &[
            TableId::DivisionPanelStats,
            TableId::DivisionStatus,
            TableId::DivisionMovement,
        ],
        TableId::LandCombatDivisionModifiers => &[TableId::LandCombatDivisions],
        TableId::Divisions => &[TableId::ProvinceUnitOrder, TableId::DivisionStatus],
        TableId::ProductionLines => &[TableId::ProductionLineModifiers],
        _ => &[],
    }
}

fn allowed(table: TableId) -> u16 {
    match table {
        TableId::DivisionMovement | TableId::NavalCombatDetails => 3,
        TableId::DivisionStatus => 15,
        TableId::Divisions => 3,
        TableId::CombatLogObservers => 1,
        TableId::CountryFuel => 15,
        TableId::LandCombatDivisions => MEMBER_MASK,
        TableId::LandCombatDivisionModifiers | TableId::ProductionLines => 1,
        _ => 0,
    }
}

// A lane becomes an alias only after exact equality. Decode never interprets a
// literal zero as an alias without its separately stored mark.
fn scalar(
    value: &mut i64,
    candidate: Option<i64>,
    bit: u16,
    bits: &mut u16,
    encode: bool,
) -> io::Result<()> {
    if encode {
        if candidate == Some(*value) {
            *bits |= bit;
            *value = 0;
        }
    } else if *bits & bit != 0 {
        if *value != 0 {
            return Err(invalid("nonzero private scalar alias"));
        }
        *value = candidate.ok_or_else(|| invalid("private scalar alias source unavailable"))?;
    }
    Ok(())
}

// These bits enable a conditional rule, not an assertion that its source exists.
// Without the optional exact operand the payload stays literal. Ordinary demand
// transitions therefore need no per-receiver marks; unequal present copies escape.
fn conditional_scalar(
    value: &mut i64,
    candidate: Option<i64>,
    bit: u16,
    bits: &mut u16,
    encode: bool,
) -> io::Result<()> {
    if candidate.is_none() {
        if encode {
            *bits |= bit;
        }
        return Ok(());
    }
    scalar(value, candidate, bit, bits, encode)
}

fn exact(value: Recorded<Option<i64>>) -> Option<i64> {
    match value {
        Recorded::Value(value) => value,
        Recorded::NotRecorded => None,
    }
}

fn grace_define(sources: &[(TableId, SharedView)]) -> Option<Option<i32>> {
    let row = find(complete::<tables::SimulationFeatures>(sources), 0)?;
    let Recorded::Value(defines) = row.movement_defines else {
        return None;
    };
    match defines.supply_grace {
        Some(value) => i32::try_from(value).ok().map(Some),
        None => Some(None),
    }
}

fn default_bits(table: TableId) -> u16 {
    match table {
        TableId::DivisionMovement | TableId::NavalCombatDetails | TableId::Divisions => 3,
        TableId::DivisionStatus => 15,
        TableId::LandCombatDivisions => MEMBER_MASK,
        TableId::LandCombatDivisionModifiers | TableId::ProductionLines => 1,
        _ => 0,
    }
}

fn row_bits(marks: &[(u128, u16)], key: u128, encode: bool, seen: &mut usize, default: u16) -> u16 {
    if encode {
        return 0;
    }
    match marks.binary_search_by_key(&key, |&(key, _)| key) {
        Ok(at) => {
            *seen += 1;
            default ^ marks[at].1
        }
        Err(_) => default,
    }
}

fn needs_source(view: &TableView, marks: &[(u128, u16)], default: u16, source: TableId) -> bool {
    let bits = |key| {
        default
            ^ marks
                .binary_search_by_key(&key, |&(key, _)| key)
                .map_or(0, |at| marks[at].1)
    };
    let rows = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
        _ => return false,
    };
    match rows {
        TableRows::DivisionMovement(rows) => rows.iter().any(|row| {
            bits(row.key()) & 2 != 0 && matches!(row.base_speed, Recorded::Value(Some(_)))
        }),
        TableRows::DivisionStatus(rows) => rows.iter().any(|row| match source {
            TableId::DivisionPanelStats => bits(row.key()) & 3 != 0,
            TableId::SimulationFeatures => {
                bits(row.key()) & 8 != 0
                    && matches!(row.supply_grace_remaining, Recorded::Value(_))
                    && exact(row.supply_grace_exact).is_some()
            }
            _ => false,
        }),
        TableRows::LandCombatDivisions(rows) => rows.iter().any(|row| {
            bits(row.key())
                & match source {
                    TableId::DivisionPanelStats => (1 << 9) - 1,
                    TableId::DivisionStatus => (1 << 9) | (1 << 10),
                    TableId::DivisionMovement => 1 << 11,
                    _ => 0,
                }
                != 0
        }),
        TableRows::LandCombatDivisionModifiers(rows) => rows.iter().any(|row| bits(row.key()) != 0),
        TableRows::Divisions(rows) => rows.iter().any(|row| {
            bits(row.key())
                & match source {
                    TableId::ProvinceUnitOrder => 1,
                    TableId::DivisionStatus => 2,
                    _ => 0,
                }
                != 0
        }),
        TableRows::ProductionLines(rows) => rows.iter().any(|row| bits(row.key()) != 0),
        _ => false,
    }
}

fn save_marks(marks: &mut Marks, key: u128, bits: u16, default: u16) {
    if bits != default {
        marks.push((key, bits ^ default));
    }
}

fn candidates(
    row: &LandCombatDivisionRow,
    sources: &[(TableId, SharedView)],
) -> [Option<i64>; MEMBER_FIELDS] {
    let panel = find(
        complete::<tables::DivisionPanelStats>(sources),
        row.division as u128,
    );
    let status = find(
        complete::<tables::DivisionStatus>(sources),
        row.division as u128,
    );
    let movement = find(
        complete::<tables::DivisionMovement>(sources),
        row.division as u128,
    );
    let mut values = [None; MEMBER_FIELDS];
    if let Some(panel) = panel {
        for (at, stat) in [11, 12, 20, 9, 10].into_iter().enumerate() {
            values[at] = panel.panel_stats[stat].map(|value| {
                if at == 2 {
                    value
                } else {
                    LandCombatDivisionRow::STAT_STEP.round(value)
                }
            });
        }
        for (at, stat) in [(5, 13), (6, 14), (7, 3), (8, 15)] {
            values[at] = panel.panel_stats[stat];
        }
    }
    if let Some(status) = status {
        values[9] = Some(status.strength_factor);
        if let Recorded::Value(Some(stats)) = &status.movement_stats {
            values[10] = Some(stats.air_defense);
        }
    }
    if let Some(movement) = movement {
        if let Recorded::Value(Some(fuel)) = movement.fuel {
            values[11] = Some(fuel);
        }
    }
    values
}

fn member_value(row: &LandCombatDivisionRow, at: usize) -> Option<i64> {
    match at {
        0..5 => match row.stat_values[at] {
            Recorded::Value(value) => Some(value),
            Recorded::NotRecorded => None,
        },
        5 => Some(row.armour),
        6 => Some(row.piercing),
        7 => Some(row.reconnaissance),
        8 => Some(row.initiative),
        9 => Some(row.strength_combat_factor),
        10 => Some(row.air_attack),
        11 => match row.fuel_inputs[0] {
            Recorded::Value(value) => Some(value),
            Recorded::NotRecorded => None,
        },
        _ => unreachable!(),
    }
}

fn set_member(row: &mut LandCombatDivisionRow, at: usize, value: i64) {
    match at {
        0..5 => row.stat_values[at] = value.into(),
        5 => row.armour = value,
        6 => row.piercing = value,
        7 => row.reconnaissance = value,
        8 => row.initiative = value,
        9 => row.strength_combat_factor = value,
        10 => row.air_attack = value,
        11 => row.fuel_inputs[0] = value.into(),
        _ => unreachable!(),
    }
}

// Invert membership without changing native occupant order. Repeated handles,
// including repeats in one province, cannot supply a unique location.
fn province_members(sources: &[(TableId, SharedView)]) -> io::Result<Vec<(u64, u32)>> {
    let Some(rows) = complete::<tables::ProvinceUnitOrder>(sources) else {
        return Ok(Vec::new());
    };
    let mut count = 0;
    for row in rows {
        count = allocation::add(count, row.mixed.len())?;
    }
    let mut members = allocation::vec(count)?;
    for row in rows {
        members.extend(row.mixed.iter().map(|&id| (id, row.province)));
    }
    members.sort_unstable_by_key(|&(id, _)| id);
    Ok(members)
}

fn unique_province(members: &[(u64, u32)], id: u64) -> Option<u32> {
    let at = members.binary_search_by_key(&id, |&(id, _)| id).ok()?;
    if (at > 0 && members[at - 1].0 == id)
        || members.get(at + 1).is_some_and(|&(next, _)| next == id)
    {
        None
    } else {
        Some(members[at].1)
    }
}

fn damage_totals(view: &TableView) -> io::Result<Vec<(u128, i64)>> {
    let TableView::Complete(TableRows::CombatLogObservers(rows)) = view else {
        return Ok(Vec::new());
    };
    let mut totals = allocation::collect_vec(
        rows.iter()
            .filter(|row| row.kind == rows::CombatLogObserverKind::Damage as u8)
            .map(|row| {
                (
                    (u128::from(row.combat) << 1) | u128::from(row.attacker),
                    row.a,
                )
            }),
    )?;
    totals.sort_unstable_by_key(|&(key, _)| key);
    let mut used = 0;
    for at in 0..totals.len() {
        let (key, amount) = totals[at];
        if used != 0 && totals[used - 1].0 == key {
            totals[used - 1].1 = totals[used - 1].1.wrapping_add(amount);
        } else {
            totals[used] = (key, amount);
            used += 1;
        }
    }
    totals.truncate(used);
    Ok(totals)
}

fn transform(
    view: &TableView,
    sources: &[(TableId, SharedView)],
    marks: &mut Marks,
    encode: bool,
    default: u16,
) -> io::Result<SharedView> {
    let mut seen = 0;
    let result = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            let mut rows = rows.try_clone_validated()?;
            match &mut rows {
                TableRows::LandCombatDivisions(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        for (at, candidate) in candidates(row, sources).into_iter().enumerate() {
                            if encode {
                                if candidate.is_some() && member_value(row, at) == candidate {
                                    bits |= 1 << at;
                                    set_member(row, at, 0);
                                }
                            } else if bits & (1 << at) != 0 {
                                if member_value(row, at) != Some(0) {
                                    return Err(invalid("nonzero combat member alias payload"));
                                }
                                set_member(
                                    row,
                                    at,
                                    candidate.ok_or_else(|| {
                                        invalid("combat member alias source unavailable")
                                    })?,
                                );
                            }
                        }
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::LandCombatDivisionModifiers(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        let candidate = find(complete::<tables::LandCombatDivisions>(sources), key)
                            .map(|row| row.division);
                        if encode {
                            if candidate == Some(row.division) {
                                bits |= 1;
                                row.division = 0;
                            }
                            save_marks(marks, key, bits, default);
                        } else if bits & 1 != 0 {
                            if row.division != 0 {
                                return Err(invalid(
                                    "nonzero combat modifier identity alias payload",
                                ));
                            }
                            row.division = candidate.ok_or_else(|| {
                                invalid("combat modifier identity alias source unavailable")
                            })?;
                        }
                    }
                }
                TableRows::Divisions(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    let members = province_members(sources)?;
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        let candidate = unique_province(&members, row.id);
                        if encode {
                            if candidate == Some(row.province) {
                                bits |= 1;
                                row.province = 0;
                            }
                        } else if bits & 1 != 0 {
                            if row.province != 0 {
                                return Err(invalid("nonzero division province alias payload"));
                            }
                            row.province = candidate.ok_or_else(|| {
                                invalid("division province alias source unavailable")
                            })?;
                        }
                        let organisation = find(complete::<tables::DivisionStatus>(sources), key)
                            .and_then(|row| exact(row.organisation_exact))
                            .map(|value| rows::DivisionRow::ORGANISATION_STEP.round(value));
                        conditional_scalar(
                            &mut row.organisation,
                            organisation,
                            2,
                            &mut bits,
                            encode,
                        )?;
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::ProductionLines(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        let candidate =
                            find(complete::<tables::ProductionLineModifiers>(sources), key)
                                .and_then(|source| row.factory_efficiency(source));
                        scalar(&mut row.efficiency_raw, candidate, 1, &mut bits, encode)?;
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::DivisionMovement(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let bits = row_bits(marks, key, encode, &mut seen, default);
                        let panel = find(complete::<tables::DivisionPanelStats>(sources), key);
                        let speed = panel.and_then(|panel| panel.panel_stats[21]);
                        if encode {
                            let mut bits = 0;
                            if u32::try_from(row.path.len()).ok() == Some(row.path_total) {
                                bits |= 1;
                                // The strategic-action admission check consumes this signed
                                // predicate before the reader restores the exact path count.
                                row.path_total = u32::from((row.path_total as i32) > 0);
                            }
                            match row.base_speed {
                                Recorded::Value(Some(value)) if Some(value) == speed => {
                                    bits |= 2;
                                    row.base_speed = Some(0).into();
                                }
                                // No operand is invented: the unchanged optional tag retains
                                // absence, and avoids marks changing with ordinary route demand.
                                Recorded::Value(None) | Recorded::NotRecorded => bits |= 2,
                                _ => {}
                            }
                            save_marks(marks, key, bits, default);
                        } else {
                            if bits & 1 != 0 {
                                let count = u32::try_from(row.path.len())
                                    .map_err(|_| invalid("aliased path count exceeds u32"))?;
                                if row.path_total != u32::from((count as i32) > 0) {
                                    return Err(invalid("invalid aliased path count placeholder"));
                                }
                                row.path_total = count;
                            }
                            if bits & 2 != 0
                                && let Recorded::Value(Some(value)) = row.base_speed
                            {
                                if value != 0 {
                                    return Err(invalid("nonzero aliased base speed"));
                                }
                                row.base_speed = Some(speed.ok_or_else(|| {
                                    invalid("base speed alias source unavailable")
                                })?)
                                .into();
                            }
                        }
                    }
                }
                TableRows::DivisionStatus(rows) => {
                    let grace_define = grace_define(sources);
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        let panel = find(complete::<tables::DivisionPanelStats>(sources), key);
                        let hp = panel.and_then(|panel| panel.panel_stats[0]).map(|value| {
                            rows::DivisionStatusRow::MAXIMUM_STEP.round(value.max(100))
                        });
                        let org = panel.and_then(|panel| panel.panel_stats[1]).map(|value| {
                            rows::DivisionStatusRow::MAXIMUM_STEP.round(value.max(100))
                        });
                        scalar(&mut row.maximum_hit_points, hp, 1, &mut bits, encode)?;
                        scalar(&mut row.maximum_organisation, org, 2, &mut bits, encode)?;
                        let ratio = exact(row.supply_ratio_exact)
                            .map(|value| rows::DivisionStatusRow::SUPPLY_RATIO_STEP.round(value));
                        conditional_scalar(&mut row.supply_ratio, ratio, 4, &mut bits, encode)?;
                        if let Recorded::Value(value) = &mut row.supply_grace_remaining {
                            let grace = exact(row.supply_grace_exact).zip(grace_define).map(
                                |(value, define)| {
                                    rows::DivisionStatusRow::displayed_grace(value, define)
                                },
                            );
                            conditional_scalar(value, grace, 8, &mut bits, encode)?;
                        } else if encode {
                            bits |= 8;
                        }
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::CombatLogObservers(rows) => {
                    let totals = damage_totals(view)?;
                    let complete = matches!(view, TableView::Complete(_));
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        if row.kind == rows::CombatLogObserverKind::State as u8 {
                            let side = (u128::from(row.combat) << 1) | u128::from(row.attacker);
                            let amount = totals
                                .binary_search_by_key(&side, |&(key, _)| key)
                                .map_or(0, |at| totals[at].1);
                            scalar(&mut row.a, complete.then_some(amount), 1, &mut bits, encode)?;
                        } else if bits != 0 {
                            return Err(invalid("damage total mark names a non-state observer"));
                        }
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::NavalCombatDetails(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        for (side, bit) in row.sides.iter_mut().zip([1, 2]) {
                            let total = side
                                .damage_history
                                .iter()
                                .flatten()
                                .fold(0_i64, |sum, &value| sum.wrapping_add(value));
                            scalar(&mut side.total_damage, Some(total), bit, &mut bits, encode)?;
                        }
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                TableRows::CountryFuel(rows) => {
                    if encode {
                        allocation::reserve(marks, rows.len())?;
                    }
                    for row in rows {
                        let key = row.key();
                        let mut bits = row_bits(marks, key, encode, &mut seen, default);
                        if let Recorded::Value(day) = &mut row.award_day
                            && day.reason.is_empty()
                        {
                            let own = day
                                .gains_raw
                                .iter()
                                .fold(0_i64, |sum, &value| sum.wrapping_add(value));
                            scalar(&mut day.own_gain_raw, Some(own), 1, &mut bits, encode)?;
                            let mut received = [0_i64; 3];
                            for recipient in &day.recipients {
                                if let Some(sum) = received.get_mut(recipient.kind as usize) {
                                    *sum = sum.wrapping_add(recipient.amount_raw);
                                }
                            }
                            for (at, value) in day.received_raw.iter_mut().enumerate() {
                                scalar(value, Some(received[at]), 2 << at, &mut bits, encode)?;
                            }
                        } else if bits != 0 {
                            return Err(invalid("fuel award alias has no complete day"));
                        }
                        if encode {
                            save_marks(marks, key, bits, default);
                        }
                    }
                }
                _ => return Err(invalid("unknown private alias table")),
            }
            match view {
                TableView::Truncated { missing, .. } => TableView::Truncated {
                    rows,
                    missing: *missing,
                },
                _ => TableView::Complete(rows),
            }
        }
        TableView::Unavailable { reason } => TableView::Unavailable {
            reason: allocation::string(reason)?,
        },
    };
    if !encode && seen != marks.len() {
        return Err(invalid("alias marks name an absent row"));
    }
    Ok(result.into())
}

// Keyframes carry their default bits and a complete map of XOR exceptions even
// when native rows anchor to another block. Later maps change only exceptions,
// including zero removals. Ordinary roster growth needs no marks for default rows.
fn encode_marks(marks: &[(u128, u16)], previous: &[(u128, u16)]) -> io::Result<Vec<u8>> {
    let mut changes = allocation::vec(allocation::add(marks.len(), previous.len())?)?;
    let (mut a, mut b) = (0, 0);
    while a < marks.len() || b < previous.len() {
        if b == previous.len() || (a < marks.len() && marks[a].0 < previous[b].0) {
            changes.push(marks[a]);
            a += 1;
        } else if a == marks.len() || previous[b].0 < marks[a].0 {
            changes.push((previous[b].0, 0));
            b += 1;
        } else {
            if marks[a].1 != previous[b].1 {
                changes.push(marks[a]);
            }
            a += 1;
            b += 1;
        }
    }
    let mut out = allocation::vec(allocation::add(19, allocation::mul(changes.len(), 22)?)?)?;
    put_varint(&mut out, changes.len() as u128);
    let mut previous = 0;
    for (key, bits) in changes {
        put_varint(&mut out, key - previous);
        put_varint(&mut out, u128::from(bits));
        previous = key;
    }
    Ok(out)
}

fn take_marks(
    table: TableId,
    input: &mut Decoder<'_>,
    previous: &[(u128, u16)],
    rows: usize,
) -> io::Result<Marks> {
    let count = usize::try_from(input.varint()?)
        .map_err(|_| invalid("alias count exceeds address space"))?;
    if count > input.remaining() / 2 {
        return Err(invalid("alias count exceeds its bytes"));
    }
    // A change either removes a preceding exception or names a current row.
    if count > allocation::add(previous.len(), rows)? {
        return Err(invalid("alias count exceeds its live rows"));
    }
    let mut changes = allocation::vec(count)?;
    let mut key = 0_u128;
    for at in 0..count {
        let difference = input.varint()?;
        if at != 0 && difference == 0 {
            return Err(invalid("repeated alias mark key"));
        }
        key = key
            .checked_add(difference)
            .ok_or_else(|| invalid("alias mark key overflow"))?;
        let bits = u16::try_from(input.varint()?).map_err(|_| invalid("alias mark exceeds u16"))?;
        if bits & !allowed(table) != 0 {
            return Err(invalid("unknown private alias bits"));
        }
        changes.push((key, bits));
    }
    let mut marks = allocation::vec(allocation::add(previous.len(), count)?)?;
    let mut at = 0;
    for (key, bits) in changes {
        while at < previous.len() && previous[at].0 < key {
            marks.push(previous[at]);
            at += 1;
        }
        if at < previous.len() && previous[at].0 == key {
            at += 1;
        }
        if bits != 0 {
            marks.push((key, bits));
        }
    }
    marks.extend_from_slice(&previous[at..]);
    Ok(marks)
}

struct Entry {
    table: TableId,
    hour: Hour,
    view: SharedView,
    marks: Marks,
    bytes: Vec<u8>,
    column: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct Encoder {
    previous: Vec<Entry>,
}

#[derive(Default)]
pub(crate) struct Prepared {
    entries: Vec<Entry>,
}

impl Prepared {
    pub fn column(&self, table: TableId, keyframe: bool) -> &[u8] {
        self.entries
            .iter()
            .find(|entry| entry.table == table)
            .map_or(if keyframe { &[0, 0][..] } else { &[0][..] }, |entry| {
                entry.column.as_slice()
            })
    }
    pub fn parts(&self) -> impl Iterator<Item = (TableId, &[u8])> {
        self.entries
            .iter()
            .map(|entry| (entry.table, entry.bytes.as_slice()))
    }
    pub fn charge(&self, sizes: &mut [usize; rows::TABLE_COUNT]) -> io::Result<usize> {
        let mut total = 0;
        for entry in &self.entries {
            let size = entry.column.len();
            sizes[entry.table.index()] = allocation::add(sizes[entry.table.index()], size)?;
            total = allocation::add(total, size)?;
        }
        Ok(total)
    }
}

impl Encoder {
    pub fn prepare(
        &self,
        frame: &Frame,
        decoded: &DecodedFrame,
        views: &mut DecodedFrame,
    ) -> io::Result<Prepared> {
        let mut entries = allocation::vec(TABLES.len())?;
        for &table in TABLES {
            let sources = allocation::collect_vec(
                dependencies(table)
                    .iter()
                    .map(|&source| (source, decoded.tables[source.index()].clone())),
            )?;
            let mut marks = Vec::new();
            let view = transform(
                &decoded.tables[table.index()],
                &sources,
                &mut marks,
                true,
                default_bits(table),
            )?;
            let before = self
                .previous
                .iter()
                .find(|entry| entry.table == table)
                .filter(|_| !frame.keyframe);
            let snapshot = matches!(frame.tables[table.index()], TableUpdate::Snapshot(_));
            let bytes = plain(
                &view,
                before.map(|entry| (entry.hour, &*entry.view)),
                snapshot,
            )?;
            let mut column = encode_marks(&marks, before.map_or(&[], |entry| &entry.marks))?;
            if frame.keyframe {
                let mut header = allocation::vec(allocation::add(3, column.len())?)?;
                put_varint(&mut header, u128::from(default_bits(table)));
                header.extend_from_slice(&column);
                column = header;
            }
            views.tables[table.index()] = view.clone();
            entries.push(Entry {
                table,
                hour: frame.hour,
                view,
                marks,
                bytes,
                column,
            });
        }
        Ok(Prepared { entries })
    }
    pub fn commit(&mut self, mut prepared: Prepared) {
        for entry in &mut prepared.entries {
            entry.bytes = Vec::new();
            entry.column = Vec::new();
        }
        self.previous = prepared.entries;
    }
}

/// Resolve private marks before the existing public frame/cache boundary.
/// Raw anchors retain only placeholders; the current block owns its marks.
pub(crate) fn resolve(
    table: TableId,
    hours: &[Hour],
    updates: Vec<TableUpdate>,
    key_marks: Option<&[u8]>,
    later_marks: Option<&[u8]>,
    complete: bool,
    mut load: impl FnMut(TableId) -> io::Result<Updates>,
) -> io::Result<Vec<TableUpdate>> {
    let Some(key_marks) = key_marks else {
        return Ok(updates);
    };
    let mut key_input = Decoder::new(key_marks);
    let default = u16::try_from(key_input.varint()?)
        .map_err(|_| invalid("default alias mask exceeds u16"))?;
    if default & !allowed(table) != 0 {
        return Err(invalid("unknown default private alias bits"));
    }
    let mut later_input = later_marks.map(Decoder::new);
    let mut marks = Vec::new();
    let mut previous_raw: Option<(Hour, SharedView)> = None;
    let mut previous: Option<(Hour, SharedView)> = None;
    let mut loaded: Vec<(TableId, Updates, usize, Option<SharedView>)> = Vec::new();
    let mut sources: Vec<(TableId, SharedView)> = Vec::new();
    let mut bytes = 0;
    let mut out = allocation::vec(updates.len())?;
    for (at, raw) in updates.into_iter().enumerate() {
        let before = previous_raw
            .take()
            .filter(|(_, view)| matches!(&**view, TableView::Complete(_)));
        let view = advance_table(table, &raw, before, hours[at], false)?;
        let rows = match &*view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            TableView::Unavailable { .. } => 0,
        };
        if at == 0 {
            marks = take_marks(table, &mut key_input, &[], rows)?;
        } else if let Some(input) = &mut later_input {
            marks = take_marks(table, input, &marks, rows)?;
        }
        // Open only marked lanes' sources. A source first needed later still
        // reconstructs its full prefix before supplying this same-frame value.
        sources.clear();
        for &source in dependencies(table) {
            if !needs_source(&view, &marks, default, source) {
                continue;
            }
            let index = match loaded.iter().position(|entry| entry.0 == source) {
                Some(index) => index,
                None => {
                    allocation::reserve(&mut loaded, 1)?;
                    loaded.push((source, load(source)?, 0, None));
                    loaded.len() - 1
                }
            };
            let (_, column, source_at, previous_source) = &mut loaded[index];
            while *source_at <= at {
                let before = previous_source
                    .take()
                    .filter(|view| matches!(&**view, TableView::Complete(_)))
                    .map(|view| (hours[*source_at - 1], view));
                *previous_source = Some(advance_table(
                    source,
                    column.get(*source_at),
                    before,
                    hours[*source_at],
                    false,
                )?);
                *source_at += 1;
            }
            allocation::reserve(&mut sources, 1)?;
            sources.push((source, previous_source.as_ref().unwrap().clone()));
        }
        let resolved = transform(&view, &sources, &mut marks, false, default)?;
        let plain = plain(
            &resolved,
            previous.as_ref().map(|(hour, view)| (*hour, &**view)),
            matches!(raw, TableUpdate::Snapshot(_)),
        )?;
        bytes = allocation::add(bytes, plain.len())?;
        if bytes > super::MAX_BLOCK_RAW_BYTES {
            return Err(invalid(
                "resolved private aliases exceed raw block size limit",
            ));
        }
        out.push(update(table, &plain, at == 0)?);
        previous_raw = Some((hours[at], view));
        previous = Some((hours[at], resolved));
    }
    key_input.finish()?;
    if complete && let Some(input) = later_input {
        input.finish()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn round_trip(view: TableView, sources: &[(TableId, SharedView)], expected_bits: u16) {
        let mut marks = Vec::new();
        let table = match &view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.table(),
            _ => unreachable!(),
        };
        let default = default_bits(table);
        let stored = transform(&view, sources, &mut marks, true, default).unwrap();
        assert_eq!(
            default ^ marks.first().map_or(0, |&(_, bits)| bits),
            expected_bits
        );
        assert_eq!(
            *transform(&stored, sources, &mut marks, false, default).unwrap(),
            view
        );
    }

    #[test]
    fn maxima_use_the_declared_nonzero_preserving_step_and_independent_literals() {
        let mut panel = rows::DivisionPanelStatsRow {
            division: 7,
            ..Default::default()
        };
        panel.panel_stats[0] = Some(-99);
        panel.panel_stats[1] = Some(i64::MAX);
        let hp = rows::DivisionStatusRow::MAXIMUM_STEP.round(100);
        let org = rows::DivisionStatusRow::MAXIMUM_STEP.round(i64::MAX);
        let sources = [(
            TableId::DivisionPanelStats,
            TableView::Complete(TableRows::DivisionPanelStats(vec![panel])).into(),
        )];
        let status = rows::DivisionStatusRow {
            division: 7,
            maximum_hit_points: hp,
            maximum_organisation: org,
            ..Default::default()
        };
        round_trip(
            TableView::Complete(TableRows::DivisionStatus(vec![status.clone()])),
            &sources,
            15,
        );
        round_trip(
            TableView::Complete(TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
                maximum_hit_points: i64::MIN,
                ..status.clone()
            }])),
            &sources,
            14,
        );
        round_trip(
            TableView::Complete(TableRows::DivisionStatus(vec![status])),
            &[],
            12,
        );
    }

    #[test]
    fn conditional_supply_displays_keep_missing_sources_and_literal_exceptions() {
        for define in [Some(143), Some(0), Some(-7), None] {
            let sources = [(
                TableId::SimulationFeatures,
                TableView::Complete(TableRows::SimulationFeatures(vec![
                    rows::SimulationFeatureRow {
                        movement_defines: rows::MovementDefines {
                            supply_grace: define.map(i64::from),
                            ..Default::default()
                        }
                        .into(),
                        ..Default::default()
                    },
                ]))
                .into(),
            )];
            for exact_value in [123_456, -123_456, 0, i64::MIN, i64::MIN + 1, i64::MAX] {
                let row = rows::DivisionStatusRow {
                    division: 7,
                    supply_ratio: rows::DivisionStatusRow::SUPPLY_RATIO_STEP.round(exact_value),
                    supply_ratio_exact: Some(exact_value).into(),
                    supply_grace_remaining: rows::DivisionStatusRow::displayed_grace(
                        exact_value,
                        define,
                    )
                    .into(),
                    supply_grace_exact: Some(exact_value).into(),
                    ..Default::default()
                };
                round_trip(
                    TableView::Complete(TableRows::DivisionStatus(vec![row.clone()])),
                    &sources,
                    12,
                );
                round_trip(
                    TableView::Complete(TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
                        supply_ratio: row.supply_ratio.wrapping_add(1),
                        ..row.clone()
                    }])),
                    &sources,
                    8,
                );
                round_trip(
                    TableView::Complete(TableRows::DivisionStatus(vec![rows::DivisionStatusRow {
                        supply_grace_remaining: match row.supply_grace_remaining {
                            Recorded::Value(value) => value.wrapping_add(1).into(),
                            Recorded::NotRecorded => unreachable!(),
                        },
                        ..row.clone()
                    }])),
                    &sources,
                    4,
                );
                // The define table is independently unavailable: grace stays literal.
                round_trip(
                    TableView::Complete(TableRows::DivisionStatus(vec![row])),
                    &[],
                    12,
                );
            }
        }
        for operand in [Recorded::Value(None), Recorded::NotRecorded] {
            let row = rows::DivisionStatusRow {
                division: 7,
                supply_ratio: 17_000,
                supply_grace_remaining: 719_000.into(),
                supply_ratio_exact: operand,
                supply_grace_exact: operand,
                ..Default::default()
            };
            round_trip(
                TableView::Complete(TableRows::DivisionStatus(vec![row.clone()])),
                &[],
                12,
            );
            round_trip(
                TableView::Truncated {
                    rows: TableRows::DivisionStatus(vec![row]),
                    missing: 2,
                },
                &[],
                12,
            );
        }
    }

    #[test]
    fn conditional_organisation_uses_only_a_complete_same_frame_status() {
        for value in [123_456, -123_456, i64::MIN, i64::MIN + 1, i64::MAX] {
            let status = rows::DivisionStatusRow {
                division: 7,
                organisation_exact: Some(value).into(),
                ..Default::default()
            };
            let row = rows::DivisionRow {
                id: 7,
                organisation: rows::DivisionRow::ORGANISATION_STEP.round(value),
                ..Default::default()
            };
            let sources = [(
                TableId::DivisionStatus,
                TableView::Complete(TableRows::DivisionStatus(vec![status.clone()])).into(),
            )];
            round_trip(
                TableView::Complete(TableRows::Divisions(vec![row.clone()])),
                &sources,
                2,
            );
            round_trip(
                TableView::Complete(TableRows::Divisions(vec![rows::DivisionRow {
                    organisation: row.organisation.wrapping_add(1),
                    ..row.clone()
                }])),
                &sources,
                0,
            );
            let sources = [(
                TableId::DivisionStatus,
                TableView::Truncated {
                    rows: TableRows::DivisionStatus(vec![status]),
                    missing: 1,
                }
                .into(),
            )];
            round_trip(
                TableView::Complete(TableRows::Divisions(vec![row])),
                &sources,
                2,
            );
        }
    }

    #[test]
    fn damage_aliases_sum_attributions_with_wrapping_without_touching_activity_or_other_lanes() {
        use rows::CombatLogObserverKind as K;
        let state = rows::CombatLogObserverRow {
            combat: 8,
            ordinal: 1,
            kind: K::State as u8,
            a: i64::MIN,
            b: i64::MAX,
            ..Default::default()
        };
        let damage = rows::CombatLogObserverRow {
            combat: 8,
            ordinal: 2,
            kind: K::Damage as u8,
            a: i64::MAX,
            ..Default::default()
        };
        let damage_2 = rows::CombatLogObserverRow {
            ordinal: 3,
            a: 1,
            ..damage.clone()
        };
        let activity = rows::CombatLogObserverRow {
            ordinal: 4,
            kind: K::ActivityDamage as u8,
            a: 99,
            ..damage.clone()
        };
        let rows = vec![state.clone(), damage, damage_2, activity];
        round_trip(
            TableView::Complete(TableRows::CombatLogObservers(rows.clone())),
            &[],
            1,
        );
        round_trip(
            TableView::Truncated {
                rows: TableRows::CombatLogObservers(rows.clone()),
                missing: 1,
            },
            &[],
            0,
        );
        let mut exception = rows;
        exception[0].a = 2;
        round_trip(
            TableView::Complete(TableRows::CombatLogObservers(exception)),
            &[],
            0,
        );
    }

    #[test]
    fn naval_totals_use_all_history_cells_not_damage_by_type() {
        let mut row = rows::NavalCombatDetailsRow {
            combat: 8,
            custom_sea_error: 1,
            ..Default::default()
        };
        row.sides[0].damage_history[0][0] = i64::MAX;
        row.sides[0].damage_history[5][5] = 1;
        row.sides[0].total_damage = i64::MIN;
        row.sides[0].damage_by_type.push(rows::NavalDamageSource {
            token: 7,
            damage: 17,
        });
        round_trip(
            TableView::Complete(TableRows::NavalCombatDetails(vec![row.clone()])),
            &[],
            3,
        );
        row.sides[0].total_damage = 19;
        round_trip(
            TableView::Complete(TableRows::NavalCombatDetails(vec![row])),
            &[],
            2,
        );
    }

    #[test]
    fn fuel_award_aliases_keep_incomplete_days_quantities_and_consumption_literal() {
        let mut day = rows::FuelAwardDay {
            gains_raw: [i64::MAX, 1, 0, 0],
            own_gain_raw: i64::MIN,
            own_gain_quantity_raw: 999,
            consumed_raw: 666,
            received_raw: [i64::MIN, 2, 3],
            ..Default::default()
        };
        for (kind, amount_raw) in [(0, i64::MAX), (0, 1), (1, 2), (2, 3)] {
            day.recipients.push(rows::FuelAwardDayRecipient {
                kind,
                amount_raw,
                ..Default::default()
            });
        }
        let row = rows::CountryFuelRow {
            country: 7,
            award_day: day.clone().into(),
            ..Default::default()
        };
        round_trip(
            TableView::Complete(TableRows::CountryFuel(vec![row.clone()])),
            &[],
            15,
        );
        day.reason = b"incomplete updates".to_vec();
        day.own_gain_raw = 888;
        round_trip(
            TableView::Complete(TableRows::CountryFuel(vec![rows::CountryFuelRow {
                award_day: day.into(),
                ..row
            }])),
            &[],
            0,
        );
    }

    #[test]
    fn absent_speed_and_literal_overrides_do_not_open_panel_sources() {
        let mut row = rows::DivisionMovementRow {
            division: 7,
            path: vec![1, 2],
            path_total: 1,
            base_speed: None.into(),
            ..Default::default()
        };
        for value in [Recorded::NotRecorded, Recorded::Value(None)] {
            row.base_speed = value;
            let view = TableView::Complete(TableRows::DivisionMovement(vec![row.clone()]));
            assert!(!needs_source(&view, &[], 3, TableId::DivisionPanelStats));
        }
        row.base_speed = Some(0).into();
        let view = TableView::Complete(TableRows::DivisionMovement(vec![row]));
        assert!(needs_source(&view, &[], 3, TableId::DivisionPanelStats));
        assert!(!needs_source(
            &view,
            &[(7, 2)],
            3,
            TableId::DivisionPanelStats
        ));
        assert!(!needs_source(
            &view,
            &[(7, 3)],
            3,
            TableId::DivisionPanelStats
        ));
    }

    #[test]
    fn sparse_marks_retain_edits_deletions_and_the_full_native_key() {
        let maps = [
            vec![(0, 1), (u64::MAX as u128, 3)],
            vec![(0, 2), (u128::MAX, 1)],
            vec![],
        ];
        let mut before = Vec::new();
        for marks in maps {
            let bytes = encode_marks(&marks, &before).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                take_marks(TableId::DivisionMovement, &mut input, &before, marks.len()).unwrap(),
                marks
            );
            input.finish().unwrap();
            before = marks;
        }
        assert!(
            take_marks(
                TableId::DivisionMovement,
                &mut Decoder::new(&[1, 0, 4]),
                &[],
                1
            )
            .is_err()
        );
        assert!(
            take_marks(
                TableId::DivisionMovement,
                &mut Decoder::new(&[2, 0, 1, 0, 1]),
                &[],
                2
            )
            .is_err()
        );
        assert!(
            take_marks(
                TableId::DivisionMovement,
                &mut Decoder::new(&[1, 0, 1]),
                &[],
                0,
            )
            .is_err()
        );
        assert!(
            take_marks(
                TableId::DivisionMovement,
                &mut Decoder::new(&[127]),
                &[],
                127
            )
            .is_err()
        );
    }
}
