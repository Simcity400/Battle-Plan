
use recorder_derive_framework::framework::{Columns, Hour, columns};
use replay_columnar::store::TableView;
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryProductionModifiersRow, FactionMemberRow, NavalBaseRow, NavalRepairBaseRow, NavalRepairCountryRow, NavalRepairQueueRow, ProvinceRow, RelationLinkRow, RowCodec, ShipModelRow, ShipRow, TableId,
        TaskForceRow,
    },
};
use replay_derive::{
    naval_repairs::{QueuedForce, QueuedModel, QueuedShip, model_operands},
    visibility::{AccessLinks, Relations},
};
use std::{collections::BTreeMap, io};

pub(super) const COUNT: [usize; 1] = columns::<NavalRepairCountryRow, 1>(["num_used_dockyards"]);
pub(super) const MODIFIERS: [usize; 3] = columns::<CountryProductionModifiersRow, 3>(["power_raw", "factory_speed", "trees"]);
pub(super) const BASE: [usize; 3] = columns::<NavalBaseRow, 3>(["country", "countries", "countries_unreadable"]);
pub(super) const ALLOCATION: [usize; 1] = columns::<NavalRepairBaseRow, 1>(["allocated_dockyards"]);
pub(super) const QUEUE: [usize; 1] = columns::<NavalRepairQueueRow, 1>(["ship"]);
pub(super) const SHIP: [usize; 2] = columns::<ShipRow, 2>(["task_force", "strength"]);
pub(super) const MODEL: [usize; 3] = columns::<ShipModelRow, 3>(["variant", "counter", "has_criticals"]);
pub(super) const FORCE: [usize; 2] = columns::<TaskForceRow, 2>(["roster_owner", "province"]);
pub(super) const CONTROLLER: [usize; 1] = columns::<ProvinceRow, 1>(["controller"]);
pub(super) const FACTION: [usize; 1] = columns::<FactionMemberRow, 1>(["faction"]);
pub(super) const LINKS: [usize; 1] = columns::<RelationLinkRow, 1>(["links"]);
pub(super) const RELATIONS: [TableId; 4] = [TableId::FactionMembers, TableId::WarRelations, TableId::Subjects, TableId::RelationLinks];

pub(super) fn relations_moved(hour: &Hour<'_>) -> bool {
    let read: [(TableId, &'static [usize]); 4] = [(TableId::FactionMembers, &FACTION), (TableId::WarRelations, &[]), (TableId::Subjects, &[]), (TableId::RelationLinks, &LINKS)];
    read.into_iter().any(|(table, columns)| hour.changes(table).status_changed || hour.wrote(table, Columns::These(columns)))
}

fn mask(columns: &[usize]) -> u64 {
    columns.iter().fold(0, |mask, column| mask | 1 << column)
}

fn row<R: RowCodec>(view: &TableView<'_>, key: u128, columns: &[usize]) -> io::Result<Option<R>> {
    let Some(slot) = view.slot(key) else { return Ok(None) };
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask(columns), &mut |column| replay_frame::frame::field(view, slot, column, format)).map(Some)
}

pub(super) fn used_dockyards(hour: &Hour<'_>, raw: u32) -> Option<i32> {
    let view = hour.table(TableId::NavalRepairCountries);
    view.number(COUNT[0], view.slot(u128::from(raw))?).map(|count| count as i32)
}

pub(super) fn has_dockyards(hour: &Hour<'_>, raw: u32) -> bool {
    hour.table(TableId::NavalRepairCountries).slot(u128::from(raw)).is_some()
}

pub(super) fn has_modifiers(hour: &Hour<'_>, raw: u32) -> bool {
    hour.table(TableId::CountryProductionModifiers).slot(u128::from(raw)).is_some()
}

pub(super) fn dockyard_output(hour: &Hour<'_>, raw: u32) -> io::Result<Option<i64>> {
    let view = hour.table(TableId::CountryProductionModifiers);
    Ok(row::<CountryProductionModifiersRow>(&view, u128::from(raw), &MODIFIERS)?.and_then(|row| row.factory_output(true)))
}

pub(super) fn base(hour: &Hour<'_>, province: u32) -> io::Result<Option<NavalBaseRow>> {
    row::<NavalBaseRow>(&hour.table(TableId::NavalBases), u128::from(province), &BASE)
}

pub(super) fn base_holder(view: &TableView<'_>, slot: u32) -> Option<(u32, u32)> {
    Some((view.key(slot)? as u32, view.number(BASE[0], slot)? as u32))
}

pub(super) fn allocation(hour: &Hour<'_>, province: u32) -> Option<i32> {
    let view = hour.table(TableId::NavalRepairBases);
    view.number(ALLOCATION[0], view.slot(u128::from(province))?).map(|count| count as i32)
}

pub(super) fn queue_key(key: u128) -> (u32, u32) {
    ((key >> 32) as u32, key as u32)
}

pub(super) fn queued(view: &TableView<'_>, key: u128) -> Option<u64> {
    view.number(QUEUE[0], view.slot(key)?).map(|ship| ship as u64)
}

pub(super) fn ship(hour: &Hour<'_>, id: u64) -> Result<Option<QueuedShip>, ()> {
    let view = hour.table(TableId::Ships);
    let Some(slot) = view.slot(u128::from(id)) else { return Ok(None) };
    match (view.number(SHIP[0], slot), view.number(SHIP[1], slot)) {
        (Some(force), Some(strength)) => Ok(Some(QueuedShip { task_force: force as u64, strength })),
        _ => Err(()),
    }
}

pub(super) fn force(hour: &Hour<'_>, id: u64) -> Result<Option<QueuedForce>, ()> {
    let view = hour.table(TableId::TaskForces);
    let Some(slot) = view.slot(u128::from(id)) else { return Ok(None) };
    match (view.number(FORCE[0], slot), view.number(FORCE[1], slot)) {
        (Some(owner), Some(province)) => Ok(Some(QueuedForce { roster_owner: owner as u32, province: province as u32 })),
        _ => Err(()),
    }
}

pub(super) fn model(hour: &Hour<'_>, ship: u64) -> io::Result<Option<QueuedModel>> {
    Ok(row::<ShipModelRow>(&hour.table(TableId::ShipModels), u128::from(ship), &MODEL)?.as_ref().map(model_operands))
}

pub(super) fn controller(hour: &Hour<'_>, province: u32) -> Result<Option<u32>, ()> {
    let view = hour.table(TableId::Provinces);
    let Some(slot) = view.slot(u128::from(province)) else { return Ok(None) };
    view.number(CONTROLLER[0], slot).map(|controller| Some(controller as u32)).ok_or(())
}

pub(super) fn relations(hour: &Hour<'_>, links: &BTreeMap<u32, u32>) -> Result<(Relations, AccessLinks), &'static str> {
    for table in RELATIONS {
        if !hour.complete(table) {
            return Err("txt_capacity: relation tables unavailable");
        }
    }
    let keys = |table: TableId| {
        let view = hour.table(table);
        view.rows().filter_map(|slot| view.key(slot)).map(|key| ((key >> 32) as u32, key as u32)).collect::<Vec<_>>()
    };
    let members = hour.table(TableId::FactionMembers);
    let factions: Vec<(u32, u64)> = members
        .rows()
        .map(|slot| Some((members.key(slot)? as u32, members.number(FACTION[0], slot)? as u64)))
        .collect::<Option<_>>()
        .ok_or("txt_capacity: FactionMembers unreadable")?;
    let held = hour.table(TableId::RelationLinks);
    let access: Vec<(u32, u32, u8)> = held
        .rows()
        .map(|slot| {
            let key = held.key(slot)?;
            Some(((key >> 32) as u32, key as u32, held.number(LINKS[0], slot)? as u8))
        })
        .collect::<Option<_>>()
        .ok_or("txt_capacity: RelationLinks unreadable")?;
    let relations = Relations::from_pairs(factions, keys(TableId::WarRelations), keys(TableId::Subjects), links)?;
    let access = AccessLinks::from_rows(access, links)?;
    Ok((relations, access))
}
