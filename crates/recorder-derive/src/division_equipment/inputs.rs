//! The writer's inputs, each table read through one function here, so a reshaped input is a
//! change of that function alone.

use crate::{
    forces,
    framework::{Hour, columns},
};
use replay_data::{
    format::FormatVersion,
    military_aid,
    rows::{
        CountryLocalSubunitRow, DivisionEquipmentRow, DivisionLandInputsRow, DivisionPanelStatsRow, DivisionRow, DivisionTemplateIconRow, LexicalTokenRow, RowCodec,
        StockByVariantRow, TableId, TemplateSubunit, VariantDefinitionRow,
    },
};
use std::io;

pub(super) const DIVISION: [usize; 5] = columns::<DivisionRow, 5>(["roster_owner", "expeditionary_owner", "template", "hit_points", "reveal_skip"]);
pub(super) const POOL: [usize; 2] = columns::<DivisionEquipmentRow, 2>(["variant_handle", "amount_raw"]);
pub(super) const TEMPLATE: [usize; 4] = columns::<DivisionTemplateIconRow, 4>(["subunits", "model_unreadable", "country", "settings"]);
pub(super) const LAND: [usize; 1] = columns::<DivisionLandInputsRow, 1>(["manpower"]);
pub(super) const PANEL: [usize; 2] = columns::<DivisionPanelStatsRow, 2>(["panel_stats[0]", "status_maxima[0]"]);
pub(super) const LOCAL: [usize; 1] = columns::<CountryLocalSubunitRow, 1>(["requirements"]);
pub(super) const VARIANT: [usize; 1] = columns::<VariantDefinitionRow, 1>(["archetype_token"]);
pub(super) const TOKEN: [usize; 1] = columns::<LexicalTokenRow, 1>(["literal"]);
pub(super) const STOCK: [usize; 1] = columns::<StockByVariantRow, 1>(["amount"]);

fn mask(columns: &[usize]) -> u64 {
    columns.iter().fold(0, |mask, column| mask | 1 << column)
}

/// `table`'s row in `slot`, its fields `mask` names, as the row decoder reads them.
fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| forces::invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

/// `table`'s row keyed `key`, when it holds one.
fn keyed<R: RowCodec>(hour: &Hour<'_>, table: TableId, key: u128, mask: u64) -> io::Result<Option<R>> {
    match hour.table(table).slot(key) {
        Some(slot) => row(hour, table, slot, mask).map(Some),
        None => Ok(None),
    }
}

/// A division as the tables count it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Division {
    /// Raw country ids: owner (its positive expeditionary owner, else its roster owner), host.
    pub raw: [u32; 2],
    pub template: u64,
    /// Current hit points, fixed5.
    pub hit_points: i64,
    /// Not a reveal copy.
    pub counted: bool,
}

/// The Divisions row at `slot`, and its id.
pub(super) fn division(hour: &Hour<'_>, slot: u32) -> io::Result<(u64, Division)> {
    let row: DivisionRow = row(hour, TableId::Divisions, slot, mask(&DIVISION))?;
    Ok((row.id, Division { raw: [military_aid::owner(&row), row.roster_owner], template: row.template, hit_points: row.hit_points, counted: row.reveal_skip == 0 }))
}

/// The key of pool entry `ordinal` of `division`, in DivisionEquipment and NewDivisionEquipment.
pub(super) fn pool_key(division: u64, ordinal: u32) -> u128 {
    u128::from(division) << 32 | u128::from(ordinal)
}

/// A pool key's division and ordinal.
pub(super) fn pool_entry_of(key: u128) -> (u64, u32) {
    ((key >> 32) as u64, key as u32)
}

/// Pool entry `key` of `table` (DivisionEquipment or NewDivisionEquipment): its design handle
/// and fixed5 amount.
pub(super) fn pool_entry(hour: &Hour<'_>, table: TableId, key: u128) -> io::Result<Option<(u64, i64)>> {
    Ok(keyed::<DivisionEquipmentRow>(hour, table, key, mask(&POOL))?.map(|row| (row.variant_handle, row.amount_raw)))
}

/// What a template's need and required manpower are read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TemplateFacts {
    /// Its grouped sub-units; none when they could not be read.
    pub units: Option<Vec<TemplateSubunit>>,
    /// Raw: the clone scope of its local sub-units.
    pub country: i32,
    pub required: Option<i64>,
}

/// Template `handle`'s DivisionTemplateIcons facts, when it has a row.
pub(super) fn template(hour: &Hour<'_>, handle: u64) -> io::Result<Option<TemplateFacts>> {
    Ok(keyed::<DivisionTemplateIconRow>(hour, TableId::DivisionTemplateIcons, u128::from(handle), mask(&TEMPLATE))?.map(|row| TemplateFacts {
        units: (row.model_unreadable == 0).then_some(row.subunits),
        country: row.country,
        required: row.settings.require("settings").ok().and_then(|settings| settings.as_ref().map(|settings| i64::from(settings.total_manpower))),
    }))
}

/// Division `id`'s manpower as its contributors' dwords wrap; none without a row.
pub(super) fn manpower(hour: &Hour<'_>, id: u64) -> io::Result<Option<i64>> {
    Ok(keyed::<DivisionLandInputsRow>(hour, TableId::DivisionLandInputs, u128::from(id), mask(&LAND))?
        .map(|row| i64::from(row.manpower.iter().fold(0_i32, |sum, part| sum.wrapping_add(part.amount)))))
}

/// Division `id`'s maximum hit points as the game shows them; none without a row or a word.
pub(super) fn maximum(hour: &Hour<'_>, id: u64) -> io::Result<Option<i64>> {
    Ok(keyed::<DivisionPanelStatsRow>(hour, TableId::DivisionPanelStats, u128::from(id), mask(&PANEL))?.and_then(|row| row.maximum_hit_points()))
}

/// A country-local sub-unit clone whose requirements differ from its global definition's: its
/// raw country, token, and requirements as (archetype token, fixed5 amount).
pub(super) type LocalSubunit = (u32, u32, Vec<(u32, i64)>);

/// The CountryLocalSubunits row at `slot`.
pub(super) fn local_subunit(hour: &Hour<'_>, slot: u32) -> io::Result<LocalSubunit> {
    let row: CountryLocalSubunitRow = row(hour, TableId::CountryLocalSubunits, slot, mask(&LOCAL))?;
    Ok((row.country, row.token, row.requirements.iter().map(|entry| (entry.archetype_token, entry.amount_raw)).collect()))
}

/// Every CountryLocalSubunits row, in the table's order.
pub(super) fn local_subunits(hour: &Hour<'_>) -> io::Result<Vec<LocalSubunit>> {
    let table = hour.table(TableId::CountryLocalSubunits);
    table.rows().map(|slot| local_subunit(hour, slot)).collect()
}

/// What the hour changed of CountryLocalSubunits: the raw country and token of each row it
/// deleted, and the slots of the rows it wrote or inserted.
pub(super) fn local_changes(hour: &Hour<'_>) -> (Vec<(u32, u32)>, Vec<u32>) {
    let gone = hour.deleted_keys(TableId::CountryLocalSubunits).map(|key| ((key >> 32) as u32, key as u32)).collect();
    (gone, forces::written(hour, TableId::CountryLocalSubunits, &LOCAL))
}

/// Token `token`'s spelling.
pub(super) fn spelling<'a>(hour: &Hour<'a>, token: u32) -> Option<&'a [u8]> {
    let table = hour.table(TableId::LexicalTokens);
    let slot = table.slot(u128::from(token))?;
    table.string(table.text(TOKEN[0], slot)?)
}

/// The tokens the hour added, when it added tokens alone; none when it deleted or respelled one.
pub(super) fn new_tokens(hour: &Hour<'_>) -> Option<Vec<u32>> {
    let changes = hour.changes(TableId::LexicalTokens);
    if !changes.deleted.is_empty() || changes.written.iter().any(|&(_, column)| usize::from(column) == TOKEN[0]) {
        return None;
    }
    let table = hour.table(TableId::LexicalTokens);
    Some(changes.inserted.iter().filter_map(|&slot| table.key(slot)).map(|key| key as u32).collect())
}

/// Every token and its spelling.
pub(super) fn spellings<'a>(hour: &Hour<'a>) -> impl Iterator<Item = (u32, &'a [u8])> + 'a {
    let table = hour.table(TableId::LexicalTokens);
    let slots: Vec<u32> = table.rows().collect();
    slots.into_iter().filter_map(move |slot| Some((table.key(slot)? as u32, table.string(table.text(TOKEN[0], slot)?)?)))
}

/// The archetype token design `variant`'s recorded definition names; none without a definition
/// or with the null archetype.
pub(super) fn archetype(hour: &Hour<'_>, variant: u64) -> Option<u32> {
    let table = hour.table(TableId::VariantDefinitions);
    let slot = table.slot(u128::from(variant))?;
    table.number(VARIANT[0], slot).map(|token| token as u32).filter(|&token| token != 0)
}

/// The designs whose definition the hour added, deleted or gave another archetype token.
pub(super) fn defined(hour: &Hour<'_>) -> Vec<u64> {
    let table = hour.table(TableId::VariantDefinitions);
    let mut designs: Vec<u64> = hour.deleted_keys(TableId::VariantDefinitions).map(|key| key as u64).collect();
    designs.extend(forces::written(hour, TableId::VariantDefinitions, &VARIANT).into_iter().filter_map(|slot| table.key(slot)).map(|key| key as u64));
    designs
}

/// A StockByVariant key's slot and design.
pub(super) fn stock_of(key: u128) -> (u32, u64) {
    ((key >> 64) as u32, key as u64)
}

/// The StockByVariant row at `slot`: its key and amount, none when not recorded.
pub(super) fn stock(hour: &Hour<'_>, slot: u32) -> Option<(u128, Option<i64>)> {
    let table = hour.table(TableId::StockByVariant);
    Some((table.key(slot)?, table.number(STOCK[0], slot)))
}

/// Every StockByVariant row: its key and amount.
pub(super) fn stocks<'a>(hour: &Hour<'a>) -> impl Iterator<Item = (u128, Option<i64>)> + 'a {
    let table = hour.table(TableId::StockByVariant);
    let slots: Vec<u32> = table.rows().collect();
    slots.into_iter().filter_map(move |slot| Some((table.key(slot)?, table.number(STOCK[0], slot))))
}
