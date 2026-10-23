//! Storage-only alias marks, independent of native rows and their delta masks.
use super::{complete, find, plain, update};
use crate::{
    Hour,
    codec::{Decoder, invalid_static as invalid, put_varint},
    format::FormatVersion,
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

/// The tables `table`'s private aliases read.
pub(crate) fn dependencies(table: TableId) -> &'static [TableId] {
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
    if let Some(movement) = movement
        && let Recorded::Value(Some(fuel)) = movement.fuel
    {
        values[11] = Some(fuel);
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
    let result = match view {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            let rows = rows.try_clone_validated()?;
            // The observers' state totals sum the view's own damage rows.
            let totals = match &rows {
                TableRows::CombatLogObservers(_) => Some(damage_totals(view)?),
                _ => None,
            };
            let complete = matches!(view, TableView::Complete(_));
            let rows = transform_rows(
                rows,
                complete,
                totals.as_deref(),
                sources,
                marks,
                encode,
                default,
            )?;
            match view {
                TableView::Truncated { missing, .. } => TableView::Truncated {
                    rows,
                    missing: *missing,
                },
                _ => TableView::Complete(rows),
            }
        }
        TableView::Unavailable { reason } => {
            let view = TableView::Unavailable {
                reason: allocation::string(reason)?,
            };
            if !encode && !marks.is_empty() {
                return Err(invalid("alias marks name an absent row"));
            }
            view
        }
        TableView::NotRecorded { mode } => {
            if !encode && !marks.is_empty() {
                return Err(invalid("alias marks name an absent row"));
            }
            TableView::NotRecorded { mode: *mode }
        }
    };
    Ok(result.into())
}

/// Completeness and observer totals belong to the original view, not a candidate subset.
#[allow(clippy::too_many_arguments)]
fn transform_rows(
    mut rows: TableRows,
    complete_view: bool,
    observer_totals: Option<&[(u128, i64)]>,
    sources: &[(TableId, SharedView)],
    marks: &mut Marks,
    encode: bool,
    default: u16,
) -> io::Result<TableRows> {
    let mut seen = 0;
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
                            candidate
                                .ok_or_else(|| invalid("combat member alias source unavailable"))?,
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
                        return Err(invalid("nonzero combat modifier identity alias payload"));
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
                    row.province = candidate
                        .ok_or_else(|| invalid("division province alias source unavailable"))?;
                }
                let organisation = find(complete::<tables::DivisionStatus>(sources), key)
                    .and_then(|row| exact(row.organisation_exact))
                    .map(|value| rows::DivisionRow::ORGANISATION_STEP.round(value));
                conditional_scalar(&mut row.organisation, organisation, 2, &mut bits, encode)?;
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
                let candidate = find(complete::<tables::ProductionLineModifiers>(sources), key)
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
                        row.base_speed = Some(
                            speed.ok_or_else(|| invalid("base speed alias source unavailable"))?,
                        )
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
                let hp = panel
                    .and_then(|panel| panel.panel_stats[0])
                    .map(|value| rows::DivisionStatusRow::MAXIMUM_STEP.round(value.max(100)));
                let org = panel
                    .and_then(|panel| panel.panel_stats[1])
                    .map(|value| rows::DivisionStatusRow::MAXIMUM_STEP.round(value.max(100)));
                scalar(&mut row.maximum_hit_points, hp, 1, &mut bits, encode)?;
                scalar(&mut row.maximum_organisation, org, 2, &mut bits, encode)?;
                let ratio = exact(row.supply_ratio_exact)
                    .map(|value| rows::DivisionStatusRow::SUPPLY_RATIO_STEP.round(value));
                conditional_scalar(&mut row.supply_ratio, ratio, 4, &mut bits, encode)?;
                if let Recorded::Value(value) = &mut row.supply_grace_remaining {
                    let grace =
                        exact(row.supply_grace_exact)
                            .zip(grace_define)
                            .map(|(value, define)| {
                                rows::DivisionStatusRow::displayed_grace(value, define)
                            });
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
            let totals = observer_totals
                .ok_or_else(|| invalid("combat log observers need their damage totals"))?;
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
                    scalar(
                        &mut row.a,
                        complete_view.then_some(amount),
                        1,
                        &mut bits,
                        encode,
                    )?;
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
    if !encode && seen != marks.len() {
        return Err(invalid("alias marks name an absent row"));
    }
    Ok(rows)
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
    take_marks_noting(table, input, previous, rows, None)
}

/// Note every named exception key, even when its bits do not change.
fn take_marks_noting(
    table: TableId,
    input: &mut Decoder<'_>,
    previous: &[(u128, u16)],
    rows: usize,
    noted: Option<&mut Vec<u128>>,
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
    if let Some(noted) = noted {
        allocation::reserve(noted, changes.len())?;
        noted.extend(changes.iter().map(|&(key, _)| key));
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
            // A table the capture mode leaves out stores nothing to alias.
            if matches!(
                &*decoded.tables[table.index()],
                TableView::NotRecorded { .. }
            ) {
                continue;
            }
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

/// Candidate rows resolve only keys that can differ from the previous hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Resolution {
    Whole,
    Candidates,
    #[cfg(test)]
    Verify,
}

fn resolution() -> Resolution {
    #[cfg(test)]
    if let Some(mode) = hooks::with(|hooks| hooks.resolution) {
        return mode;
    }
    Resolution::Candidates
}

/// The resolved bytes one table read may decode.
fn resolved_limit() -> usize {
    #[cfg(test)]
    if let Some(limit) = hooks::with(|hooks| hooks.limit) {
        return limit;
    }
    super::MAX_BLOCK_RAW_BYTES
}

/// Hooks are process-wide because a block can resolve on several decode workers.
#[cfg(test)]
pub(crate) mod hooks {
    use super::*;

    #[derive(Default)]
    pub(crate) struct Hooks {
        pub resolution: Option<Resolution>,
        /// Resolved bytes a table read may decode, instead of the block bound.
        pub limit: Option<usize>,
        /// Each table read's resolved bytes so far after each hour.
        pub budget: Vec<(TableId, usize, usize)>,
        /// Hours resolved from candidate rows.
        pub candidate_hours: usize,
        /// Resolve alias tables alone, as an independent group reference.
        pub group_old: Option<bool>,
        /// Resolve each table a group resolved alone too, and compare.
        pub verify_group: bool,
        /// A group member that panics as it resolves an hour index.
        pub group_panic: Option<(TableId, usize)>,
        /// Table, hour index, and whether the injected failure follows source reads.
        pub fail: Vec<(TableId, usize, bool, Failure)>,
        /// Tables whose raw columns fail.
        pub fail_raw: Vec<(TableId, Failure)>,
        pub raw_gate: Option<(TableId, std::sync::Arc<Gate>)>,
    }

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum Failure {
        Text,
        Bare,
        Os(i32),
    }

    fn injected(form: Failure, identity: std::fmt::Arguments<'_>) -> io::Error {
        match form {
            Failure::Text => crate::codec::invalid(identity.to_string()),
            Failure::Bare => io::Error::from(io::ErrorKind::OutOfMemory),
            Failure::Os(code) => io::Error::from_raw_os_error(code),
        }
    }

    /// `table`'s injected failure at hour index `at`, if any.
    pub(crate) fn fail(table: TableId, at: usize, after: bool) -> io::Result<()> {
        match with(|hooks| {
            hooks
                .fail
                .iter()
                .find_map(|&(failed, hour, follows, form)| {
                    (failed == table && hour == at && follows == after).then_some(form)
                })
        }) {
            Some(form) => Err(injected(
                form,
                format_args!(
                    "injected alias failure: {table:?} hour {at} {}",
                    if after { "after" } else { "before" },
                ),
            )),
            None => Ok(()),
        }
    }

    /// `table`'s injected raw column failure, if any.
    pub(crate) fn fail_raw(table: TableId) -> io::Result<()> {
        match with(|hooks| {
            hooks
                .fail_raw
                .iter()
                .find_map(|&(failed, form)| (failed == table).then_some(form))
        }) {
            Some(form) => Err(injected(
                form,
                format_args!("injected alias failure: {table:?} raw"),
            )),
            None => Ok(()),
        }
    }

    /// Hold overlapping raw decodes until every expected reader has arrived.
    pub(crate) struct Gate {
        parties: usize,
        arrived: std::sync::Mutex<usize>,
        all: std::sync::Condvar,
    }

    impl Gate {
        pub(crate) fn new(parties: usize) -> Self {
            Self {
                parties,
                arrived: std::sync::Mutex::new(0),
                all: std::sync::Condvar::new(),
            }
        }

        /// Arrival counts decide progress; the timeout only prevents a hung test.
        fn arrive(&self) {
            let bound = std::time::Instant::now() + std::time::Duration::from_secs(600);
            let mut arrived = self
                .arrived
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *arrived += 1;
            self.all.notify_all();
            while *arrived < self.parties {
                let left = bound.saturating_duration_since(std::time::Instant::now());
                assert!(
                    !left.is_zero(),
                    "a raw column gate: {} of {} arrived",
                    *arrived,
                    self.parties
                );
                arrived = self
                    .all
                    .wait_timeout(arrived, left)
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .0;
            }
        }
    }

    /// Wait at `table`'s raw column gate, if one is set; the hooks are not held while waiting.
    pub(crate) fn gate_raw(table: TableId) {
        let gate = with(|hooks| {
            hooks
                .raw_gate
                .as_ref()
                .filter(|(gated, _)| *gated == table)
                .map(|(_, gate)| std::sync::Arc::clone(gate))
        });
        if let Some(gate) = gate {
            gate.arrive();
        }
    }

    static HOOKS: std::sync::Mutex<Option<Hooks>> = std::sync::Mutex::new(None);

    pub(crate) fn with<T>(work: impl FnOnce(&mut Hooks) -> T) -> T {
        let mut hooks = HOOKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        work(hooks.get_or_insert_with(Hooks::default))
    }

    thread_local! {
        pub(super) static STEP: std::cell::Cell<Option<Step>> =
            const { std::cell::Cell::new(None) };
    }

    /// The step of candidate-hour work running on this thread, if any.
    pub(crate) fn step() -> Option<Step> {
        STEP.get()
    }

    /// Restore the enclosing step after recursively resolving a source.
    pub(super) struct Scope(Option<Step>);

    impl Scope {
        pub(super) fn enter() -> Self {
            Self(STEP.get())
        }
    }

    impl Drop for Scope {
        fn drop(&mut self) {
            STEP.set(self.0);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    /// The keys whose marks changed.
    Marks,
    /// The candidate keys from raw and source changes.
    Keys,
    /// Bringing a loaded source up to the hour.
    CatchUp,
    /// Loading a source this hour first reads.
    Load,
    /// Copying the candidate rows and their marks.
    Rows,
    /// Resolving the candidate rows.
    Resolve,
    /// Encoding their change from the hour before.
    Encode,
    /// Merging them into the hour before's resolved table.
    Merge,
}

/// Note the candidate-hour step now running, for tests; nothing otherwise.
#[inline]
fn at_step(_step: Step) {
    #[cfg(test)]
    hooks::STEP.set(Some(_step));
}

/// Typed work on a table that resolves from candidate rows.
trait Candidate {
    type Output;
    fn typed<S: rows::TableSchema>(self) -> io::Result<Self::Output>;
}

/// Observers total whole rows; combat divisions join through division identities.
fn candidate<C: Candidate>(table: TableId, work: C) -> Option<io::Result<C::Output>> {
    Some(match table {
        TableId::DivisionStatus => work.typed::<tables::DivisionStatus>(),
        TableId::DivisionMovement => work.typed::<tables::DivisionMovement>(),
        TableId::Divisions => work.typed::<tables::Divisions>(),
        TableId::ProductionLines => work.typed::<tables::ProductionLines>(),
        TableId::LandCombatDivisionModifiers => work.typed::<tables::LandCombatDivisionModifiers>(),
        TableId::NavalCombatDetails => work.typed::<tables::NavalCombatDetails>(),
        TableId::CountryFuel => work.typed::<tables::CountryFuel>(),
        _ => return None,
    })
}

struct Probe;

impl Candidate for Probe {
    type Output = ();
    fn typed<S: rows::TableSchema>(self) -> io::Result<()> {
        Ok(())
    }
}

/// How a candidate table's rows read a source's rows.
#[derive(Clone, Copy)]
enum Feeds {
    /// The source row with the row's own key.
    Key,
    /// Every occurrence of the row's key among the source rows' members.
    Members,
    /// Every row of the source: a change resolves the whole table.
    Table,
}

fn feeds(table: TableId, source: TableId) -> Feeds {
    match (table, source) {
        (TableId::DivisionStatus, TableId::SimulationFeatures) => Feeds::Table,
        (TableId::Divisions, TableId::ProvinceUnitOrder) => Feeds::Members,
        _ => Feeds::Key,
    }
}

fn row_key<T: rows::RowCodec>(row: &T) -> u128 {
    rows::RowCodec::key(row)
}

pub(crate) trait Column {
    fn update(&self, at: usize) -> &TableUpdate;
}

impl Column for Updates {
    fn update(&self, at: usize) -> &TableUpdate {
        self.get(at)
    }
}

impl Column for [TableUpdate] {
    fn update(&self, at: usize) -> &TableUpdate {
        &self[at]
    }
}

fn catch_up(
    source: TableId,
    column: &(impl Column + ?Sized),
    source_at: &mut usize,
    view: &mut Option<SharedView>,
    hours: &[Hour],
    at: usize,
) -> io::Result<()> {
    while *source_at <= at {
        let before = view
            .take()
            .filter(|view| matches!(&**view, TableView::Complete(_)))
            .map(|view| (hours[*source_at - 1], view));
        *view = Some(advance_table(
            source,
            column.update(*source_at),
            before,
            hours[*source_at],
            false,
        )?);
        *source_at += 1;
    }
    Ok(())
}

/// A whole-table replacement cannot be represented by candidate keys.
fn source_keys(
    column: &(impl Column + ?Sized),
    from: usize,
    at: usize,
    keys: &mut Vec<u128>,
) -> io::Result<bool> {
    for hour in from..=at {
        match column.update(hour) {
            TableUpdate::Delta {
                changed, deleted, ..
            } => {
                allocation::reserve(keys, allocation::add(changed.len(), deleted.len())?)?;
                keys.extend(changed.iter().map(|row| row.key));
                keys.extend_from_slice(deleted);
            }
            TableUpdate::Unchanged { .. } => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// Add the members of a complete ProvinceUnitOrder view's rows at `keys`.
fn members(view: Option<&TableView>, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()> {
    let Some(rows) = view.and_then(|view| match view {
        TableView::Complete(rows) => rows.rows::<tables::ProvinceUnitOrder>(),
        _ => None,
    }) else {
        return Ok(());
    };
    for &key in keys {
        if let Ok(at) = rows.binary_search_by_key(&key, row_key) {
            allocation::reserve(out, rows[at].mixed.len())?;
            out.extend(rows[at].mixed.iter().map(|&id| u128::from(id)));
        }
    }
    Ok(())
}

/// Member joins need the source's rows both before and after the hour.
pub(crate) fn reads_members(table: TableId, source: TableId) -> bool {
    matches!(feeds(table, source), Feeds::Members)
}

/// `source_at` is the first unapplied hour; `view` belongs to the preceding hour.
#[derive(Default)]
pub(crate) struct Followed {
    pub(crate) source_at: usize,
    pub(crate) view: Option<SharedView>,
}

impl Followed {
    pub(crate) fn changed_keys(
        &self,
        column: &(impl Column + ?Sized),
        at: usize,
        keys: &mut Vec<u128>,
    ) -> io::Result<bool> {
        source_keys(column, self.source_at, at, keys)
    }

    pub(crate) fn complete(&self) -> bool {
        matches!(self.view.as_deref(), Some(TableView::Complete(_)))
    }

    pub(crate) fn members(&self, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()> {
        members(self.view.as_deref(), keys, out)
    }

    pub(crate) fn catch_up(
        &mut self,
        source: TableId,
        column: &(impl Column + ?Sized),
        hours: &[Hour],
        at: usize,
    ) -> io::Result<()> {
        catch_up(
            source,
            column,
            &mut self.source_at,
            &mut self.view,
            hours,
            at,
        )
    }

    pub(crate) fn view(&self) -> SharedView {
        self.view.as_ref().unwrap().clone()
    }
}

/// Standalone reads own source views; group reads can share them.
pub(crate) trait Supply {
    /// The sources loaded so far, in the order they were first needed.
    fn loaded(&self) -> usize;
    /// The loaded source at `index`.
    fn table(&self, index: usize) -> TableId;
    /// The position of `source` among those loaded, if it is.
    fn position(&self, source: TableId) -> Option<usize>;
    /// Load `source`, first needed at hour index `at`, and give its position.
    fn load(&mut self, source: TableId, at: usize) -> io::Result<usize>;
    /// Return false when the source replaces its whole table.
    fn changed_keys(&mut self, index: usize, at: usize, keys: &mut Vec<u128>) -> io::Result<bool>;
    /// Whether the source was complete at the hour it was last brought up to.
    fn complete(&self, index: usize) -> bool;
    fn members(&self, index: usize, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()>;
    /// Bring the source up to hour index `at`.
    fn catch_up(&mut self, index: usize, hours: &[Hour], at: usize) -> io::Result<()>;
    /// The source's view at the hour it was last brought up to.
    fn view(&self, index: usize) -> SharedView;
}

struct Own<F> {
    load: F,
    loaded: Vec<(TableId, Updates, Followed)>,
}

impl<F: FnMut(TableId) -> io::Result<Updates>> Supply for Own<F> {
    fn loaded(&self) -> usize {
        self.loaded.len()
    }

    fn table(&self, index: usize) -> TableId {
        self.loaded[index].0
    }

    fn position(&self, source: TableId) -> Option<usize> {
        self.loaded.iter().position(|entry| entry.0 == source)
    }

    fn load(&mut self, source: TableId, _at: usize) -> io::Result<usize> {
        allocation::reserve(&mut self.loaded, 1)?;
        self.loaded
            .push((source, (self.load)(source)?, Followed::default()));
        Ok(self.loaded.len() - 1)
    }

    fn changed_keys(&mut self, index: usize, at: usize, keys: &mut Vec<u128>) -> io::Result<bool> {
        let (_, column, followed) = &self.loaded[index];
        followed.changed_keys(column, at, keys)
    }

    fn complete(&self, index: usize) -> bool {
        self.loaded[index].2.complete()
    }

    fn members(&self, index: usize, keys: &[u128], out: &mut Vec<u128>) -> io::Result<()> {
        self.loaded[index].2.members(keys, out)
    }

    fn catch_up(&mut self, index: usize, hours: &[Hour], at: usize) -> io::Result<()> {
        let (source, column, followed) = &mut self.loaded[index];
        followed.catch_up(*source, &*column, hours, at)
    }

    fn view(&self, index: usize) -> SharedView {
        self.loaded[index].2.view()
    }
}

/// The table's rows at ascending `keys`, those it holds.
struct Select<'a> {
    rows: &'a TableRows,
    keys: &'a [u128],
}

impl Candidate for Select<'_> {
    type Output = TableRows;
    fn typed<S: rows::TableSchema>(self) -> io::Result<TableRows> {
        let held =
            S::borrow(self.rows).ok_or_else(|| invalid("candidate rows of another table"))?;
        let mut out = allocation::vec(self.keys.len().min(held.len()))?;
        let mut from = 0;
        for &key in self.keys {
            match held[from..].binary_search_by_key(&key, row_key) {
                Ok(at) => {
                    out.push(rows::RowCodec::try_clone_validated(&held[from + at])?);
                    from += at + 1;
                }
                Err(at) => from += at,
            }
        }
        Ok(S::wrap(out))
    }
}

/// Emit directly from canonical encoded candidate changes, without decoding their fields again.
struct Plain<'a> {
    changed: &'a TableRows,
    previous: &'a TableRows,
    keys: &'a [u128],
    hour: Hour,
}

impl Candidate for Plain<'_> {
    type Output = (Vec<u8>, TableUpdate);
    fn typed<S: rows::TableSchema>(self) -> io::Result<(Vec<u8>, TableUpdate)> {
        let (Some(changed), Some(previous)) = (S::borrow(self.changed), S::borrow(self.previous))
        else {
            return Err(invalid("candidate rows of another table"));
        };
        // The table's row count this hour.
        let count = {
            let mut count = previous.len();
            let mut held = changed.iter().peekable();
            let mut from = 0;
            for &key in self.keys {
                let present = held.next_if(|row| row_key(*row) == key).is_some();
                match previous[from..].binary_search_by_key(&key, row_key) {
                    Ok(at) => {
                        from += at + 1;
                        count -= usize::from(!present);
                    }
                    Err(at) => {
                        from += at;
                        count += usize::from(present);
                    }
                }
            }
            count
        };
        let mut out = allocation::vec(9)?;
        out.push(1);
        out.extend_from_slice(&self.hour.to_le_bytes());
        let (mut written, mut gone) = (Vec::new(), Vec::new());
        let changes = rows::restricted_changes::encode_changes_at_noted(
            changed,
            previous,
            self.keys,
            count,
            &mut out,
            &mut Vec::new(),
            &mut written,
            &mut gone,
        )?;
        if !changes {
            out.truncate(1);
            out[0] = 2;
            out.extend_from_slice(&self.hour.to_le_bytes());
            return Ok((
                out,
                TableUpdate::Unchanged {
                    baseline: self.hour,
                },
            ));
        }
        let changed = rows::restricted_changes::delta_rows(
            S::ID,
            FormatVersion::CURRENT.version(),
            &out,
            &written,
        )?;
        Ok((
            out,
            TableUpdate::Delta {
                baseline: self.hour,
                changed,
                deleted: gone,
            },
        ))
    }
}

/// Membership changes use the spare buffer, leaving the old buffer for the next merge.
struct Merge<'a> {
    rows: &'a mut TableRows,
    changed: TableRows,
    keys: &'a [u128],
    spare: &'a mut Option<TableRows>,
}

impl Candidate for Merge<'_> {
    type Output = ();
    fn typed<S: rows::TableSchema>(self) -> io::Result<()> {
        let rows =
            S::borrow_mut(self.rows).ok_or_else(|| invalid("candidate rows of another table"))?;
        let changed =
            S::take(self.changed).map_err(|_| invalid("candidate rows of another table"))?;
        let (mut count, mut moved) = (rows.len(), false);
        let mut held = changed.iter().peekable();
        let mut from = 0;
        for &key in self.keys {
            let present = held.next_if(|row| row_key(*row) == key).is_some();
            match rows[from..].binary_search_by_key(&key, row_key) {
                Ok(at) => {
                    from += at + 1;
                    if !present {
                        count -= 1;
                        moved = true;
                    }
                }
                Err(at) => {
                    from += at;
                    if present {
                        count += 1;
                        moved = true;
                    }
                }
            }
        }
        if !moved {
            // Every changed row replaces a held row in place.
            let mut from = 0;
            for row in changed {
                let at = from
                    + rows[from..]
                        .binary_search_by_key(&row_key(&row), row_key)
                        .map_err(|_| invalid("candidate row is not held"))?;
                rows[at] = row;
                from = at + 1;
            }
            return Ok(());
        }
        // Allocate before taking any row, so a refusal leaves the table whole.
        let mut out = match self.spare.take().map(S::take) {
            Some(Ok(spare)) => spare,
            _ => Vec::new(),
        };
        allocation::reserve(&mut out, count)?;
        let mut old = std::mem::take(rows);
        let mut before = old.drain(..).peekable();
        let mut changed = changed.into_iter().peekable();
        for &key in self.keys {
            while let Some(row) = before.next_if(|row| row_key(row) < key) {
                out.push(row);
            }
            before.next_if(|row| row_key(row) == key);
            if let Some(row) = changed.next_if(|row| row_key(row) == key) {
                out.push(row);
            }
        }
        out.extend(before);
        *rows = out;
        *self.spare = Some(S::wrap(old));
        Ok(())
    }
}

/// One hour of an alias table resolved from candidate rows.
struct CandidateHour {
    /// The candidate keys, ascending.
    keys: Vec<u128>,
    /// The resolved rows the table holds at those keys.
    rows: TableRows,
    /// The table's plain change from the hour before.
    plain: Vec<u8>,
    /// The update those plain bytes decode to.
    emitted: TableUpdate,
}

/// Only changed raw rows, marks and source rows can change candidate results.
#[allow(clippy::too_many_arguments)]
fn candidate_hour(
    table: TableId,
    hours: &[Hour],
    at: usize,
    raw: &TableUpdate,
    view: &TableView,
    marks: &[(u128, u16)],
    mark_keys: &[u128],
    default: u16,
    previous: &(Hour, SharedView),
    supply: &mut impl Supply,
    source_changes: &mut Vec<u128>,
) -> io::Result<Option<CandidateHour>> {
    let (TableView::Complete(raw_rows), TableView::Complete(resolved)) = (view, &*previous.1)
    else {
        return Ok(None);
    };
    #[cfg(test)]
    let _scope = hooks::Scope::enter();
    at_step(Step::Keys);
    let mut keys = match raw {
        TableUpdate::Delta {
            changed, deleted, ..
        } => allocation::collect_vec(
            changed
                .iter()
                .map(|row| row.key)
                .chain(deleted.iter().copied()),
        )?,
        TableUpdate::Unchanged { .. } => Vec::new(),
        _ => return Ok(None),
    };
    allocation::reserve(&mut keys, mark_keys.len())?;
    keys.extend_from_slice(mark_keys);
    for index in 0..supply.loaded() {
        let source = supply.table(index);
        let changed = &mut *source_changes;
        changed.clear();
        if !supply.changed_keys(index, at, changed)? {
            return Ok(None);
        }
        let was_complete = supply.complete(index);
        let reads = feeds(table, source);
        // A member's location reads its rows before and after.
        if matches!(reads, Feeds::Members) {
            supply.members(index, changed, &mut keys)?;
        }
        at_step(Step::CatchUp);
        supply.catch_up(index, hours, at)?;
        at_step(Step::Keys);
        if supply.complete(index) != was_complete {
            return Ok(None);
        }
        if !was_complete || changed.is_empty() {
            continue;
        }
        match reads {
            Feeds::Key => {
                allocation::reserve(&mut keys, changed.len())?;
                keys.extend_from_slice(changed);
            }
            Feeds::Members => supply.members(index, changed, &mut keys)?,
            Feeds::Table => return Ok(None),
        }
    }
    keys.sort_unstable();
    keys.dedup();
    // The candidate rows, their marks and the sources they read.
    at_step(Step::Rows);
    let Some(rows) = candidate(
        table,
        Select {
            rows: raw_rows,
            keys: &keys,
        },
    ) else {
        return Ok(None);
    };
    let candidates = TableView::Complete(rows?);
    // Each candidate key's mark, found by one search per key.
    let mut candidate_marks = allocation::vec(keys.len().min(marks.len()))?;
    for key in &keys {
        if let Ok(at) = marks.binary_search_by_key(key, |&(key, _)| key) {
            candidate_marks.push(marks[at]);
        }
    }
    let mut sources = Vec::new();
    for &source in dependencies(table) {
        if !needs_source(&candidates, &candidate_marks, default, source) {
            continue;
        }
        let index = match supply.position(source) {
            Some(index) => index,
            None => {
                at_step(Step::Load);
                supply.load(source, at)?
            }
        };
        at_step(Step::CatchUp);
        supply.catch_up(index, hours, at)?;
        at_step(Step::Rows);
        allocation::reserve(&mut sources, 1)?;
        sources.push((source, supply.view(index)));
    }
    at_step(Step::Resolve);
    // The copied candidate rows resolve in place.
    let TableView::Complete(candidate_rows) = candidates else {
        unreachable!("candidate rows are a complete view")
    };
    let resolved_rows = transform_rows(
        candidate_rows,
        true,
        None,
        &sources,
        &mut candidate_marks,
        false,
        default,
    )?;
    at_step(Step::Encode);
    let (plain, emitted) = candidate(
        table,
        Plain {
            changed: &resolved_rows,
            previous: resolved,
            keys: &keys,
            hour: previous.0,
        },
    )
    .expect("a candidate table")?;
    Ok(Some(CandidateHour {
        keys,
        rows: resolved_rows,
        plain,
        emitted,
    }))
}

#[allow(clippy::too_many_arguments)]
fn whole_hour(
    table: TableId,
    hours: &[Hour],
    at: usize,
    snapshot: bool,
    view: &TableView,
    marks: &mut Marks,
    default: u16,
    previous: Option<&(Hour, SharedView)>,
    supply: &mut impl Supply,
    sources: &mut Vec<(TableId, SharedView)>,
) -> io::Result<(Vec<u8>, SharedView)> {
    // Open only marked lanes' sources. A source first needed later still
    // reconstructs its full prefix before supplying this same-frame value.
    sources.clear();
    for &source in dependencies(table) {
        if !needs_source(view, marks, default, source) {
            continue;
        }
        let index = match supply.position(source) {
            Some(index) => index,
            None => supply.load(source, at)?,
        };
        supply.catch_up(index, hours, at)?;
        allocation::reserve(sources, 1)?;
        sources.push((source, supply.view(index)));
    }
    let resolved = transform(view, sources, marks, false, default)?;
    let plain = plain(
        &resolved,
        previous.map(|(hour, view)| (*hour, &**view)),
        snapshot,
    )?;
    Ok((plain, resolved))
}

pub(crate) struct TableResolution<'m> {
    table: TableId,
    key_input: Decoder<'m>,
    default: u16,
    later_input: Option<Decoder<'m>>,
    marks: Marks,
    previous_raw: Option<(Hour, SharedView)>,
    previous: Option<(Hour, SharedView)>,
    sources: Vec<(TableId, SharedView)>,
    bytes: usize,
    out: Vec<TableUpdate>,
    candidates: bool,
    /// Tests: resolve every candidate hour whole too, and compare.
    #[cfg(test)]
    verify: bool,
    mark_keys: Vec<u128>,
    // Buffers kept from one candidate hour to the next.
    spare: Option<TableRows>,
    source_changes: Vec<u128>,
}

impl<'m> TableResolution<'m> {
    /// `frames` hours of `table` with its keyframe and later entries' marks.
    pub(crate) fn new(
        table: TableId,
        frames: usize,
        key_marks: &'m [u8],
        later_marks: Option<&'m [u8]>,
    ) -> io::Result<Self> {
        let mut key_input = Decoder::new(key_marks);
        let default = u16::try_from(key_input.varint()?)
            .map_err(|_| invalid("default alias mask exceeds u16"))?;
        if default & !allowed(table) != 0 {
            return Err(invalid("unknown default private alias bits"));
        }
        let later_input = later_marks.map(Decoder::new);
        let out = allocation::vec(frames)?;
        let mode = resolution();
        Ok(Self {
            table,
            key_input,
            default,
            later_input,
            marks: Vec::new(),
            previous_raw: None,
            previous: None,
            sources: Vec::new(),
            bytes: 0,
            out,
            candidates: mode != Resolution::Whole && candidate(table, Probe).is_some(),
            #[cfg(test)]
            verify: mode == Resolution::Verify,
            mark_keys: Vec::new(),
            spare: None,
            source_changes: Vec::new(),
        })
    }

    pub(crate) fn hour(
        &mut self,
        hours: &[Hour],
        at: usize,
        raw: TableUpdate,
        supply: &mut impl Supply,
    ) -> io::Result<()> {
        let (table, default, candidates) = (self.table, self.default, self.candidates);
        #[cfg(test)]
        hooks::fail(table, at, false)?;
        let before = self
            .previous_raw
            .take()
            .filter(|(_, view)| matches!(&**view, TableView::Complete(_)));
        let view = advance_table(table, &raw, before, hours[at], false)?;
        let rows = match &*view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len(),
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => 0,
        };
        self.mark_keys.clear();
        if at == 0 {
            self.marks = take_marks(table, &mut self.key_input, &[], rows)?;
        } else if let Some(input) = &mut self.later_input {
            self.marks = if candidates {
                #[cfg(test)]
                let _scope = hooks::Scope::enter();
                at_step(Step::Marks);
                take_marks_noting(table, input, &self.marks, rows, Some(&mut self.mark_keys))?
            } else {
                take_marks(table, input, &self.marks, rows)?
            };
        }
        // Refusals fall back whole; allocation failures propagate because source views may be gone.
        let hour = match self.previous.as_ref().filter(|_| candidates && at != 0) {
            Some(previous) => match candidate_hour(
                table,
                hours,
                at,
                &raw,
                &view,
                &self.marks,
                &self.mark_keys,
                default,
                previous,
                supply,
                &mut self.source_changes,
            ) {
                Ok(hour) => hour,
                Err(error) if allocation::is_allocation_error(&error) => return Err(error),
                Err(_) => None,
            },
            None => None,
        };
        let snapshot = matches!(raw, TableUpdate::Snapshot(_));
        let (plain, resolved, emitted) = match hour {
            Some(CandidateHour {
                keys,
                rows,
                plain,
                emitted,
            }) => {
                #[cfg(test)]
                let whole = self.verify.then(|| {
                    whole_hour(
                        table,
                        hours,
                        at,
                        snapshot,
                        &view,
                        &mut self.marks,
                        default,
                        self.previous.as_ref(),
                        supply,
                        &mut self.sources,
                    )
                });
                let (_, mut resolved) = self.previous.take().expect("a candidate hour follows one");
                let TableView::Complete(held) = &mut *resolved else {
                    unreachable!("candidate hours follow a complete resolved hour");
                };
                #[cfg(test)]
                let merging = hooks::Scope::enter();
                at_step(Step::Merge);
                candidate(
                    table,
                    Merge {
                        rows: held,
                        changed: rows,
                        keys: &keys,
                        spare: &mut self.spare,
                    },
                )
                .expect("a candidate table")?;
                #[cfg(test)]
                {
                    drop(merging);
                    if let Some(whole) = whole {
                        let (whole_plain, whole_view) =
                            whole.expect("candidate hours resolve whole");
                        assert_eq!(whole_plain, plain, "{table:?} hour {}", hours[at]);
                        assert_eq!(*whole_view, *resolved, "{table:?} hour {}", hours[at]);
                        // The emitted update is the one its bytes decode to.
                        assert_eq!(
                            update(table, &whole_plain, false).unwrap(),
                            emitted,
                            "{table:?} hour {}",
                            hours[at]
                        );
                    }
                    hooks::with(|hooks| hooks.candidate_hours += 1);
                }
                (plain, resolved, Some(emitted))
            }
            None => {
                let (plain, resolved) = whole_hour(
                    table,
                    hours,
                    at,
                    snapshot,
                    &view,
                    &mut self.marks,
                    default,
                    self.previous.as_ref(),
                    supply,
                    &mut self.sources,
                )?;
                if candidates {
                    self.sources.clear();
                }
                (plain, resolved, None)
            }
        };
        #[cfg(test)]
        hooks::fail(table, at, true)?;
        self.bytes = allocation::add(self.bytes, plain.len())?;
        #[cfg(test)]
        hooks::with(|hooks| hooks.budget.push((table, at, self.bytes)));
        if self.bytes > resolved_limit() {
            return Err(invalid(
                "resolved private aliases exceed raw block size limit",
            ));
        }
        self.out.push(match emitted {
            Some(emitted) => emitted,
            None => update(table, &plain, at == 0)?,
        });
        self.previous_raw = Some((hours[at], view));
        self.previous = Some((hours[at], resolved));
        Ok(())
    }

    pub(crate) fn resolved(&self) -> Option<&SharedView> {
        self.previous.as_ref().map(|(_, view)| view)
    }

    /// The updates resolved so far, one an hour.
    pub(crate) fn updates(&self) -> &[TableUpdate] {
        &self.out
    }

    pub(crate) fn release(&mut self) {
        self.sources.clear();
    }

    /// Always finish keyframe marks; finish later marks only for a complete read.
    pub(crate) fn finish(self, complete: bool) -> io::Result<Vec<TableUpdate>> {
        self.key_input.finish()?;
        if complete && let Some(input) = self.later_input {
            input.finish()?;
        }
        Ok(self.out)
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
    load: impl FnMut(TableId) -> io::Result<Updates>,
) -> io::Result<Vec<TableUpdate>> {
    let Some(key_marks) = key_marks else {
        return Ok(updates);
    };
    let mut resolution = TableResolution::new(table, updates.len(), key_marks, later_marks)?;
    let mut supply = Own {
        load,
        loaded: Vec::new(),
    };
    for (at, raw) in updates.into_iter().enumerate() {
        resolution.hour(hours, at, raw, &mut supply)?;
    }
    resolution.finish(complete)
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
