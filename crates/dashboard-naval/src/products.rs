pub mod naval;

use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeSet;

fn raws(held: &DecodedFrame, slot: u32) -> BTreeSet<u32> {
    held.complete::<tables::CountryLinks>().map(|rows| rows.iter().filter(|row| row.canonical_slot == slot).map(|row| row.id).collect()).unwrap_or_default()
}

fn fleet_fields(table: TableId) -> Option<&'static [&'static str]> {
    Some(match table {
        TableId::Countries => &["canonical_slot", "tag"],
        TableId::CountryLinks => &["canonical_slot"],
        TableId::CountryAppearance => &["name_tag"],
        TableId::TaskForceCounterInputs => &["name"],
        TableId::NavalBases => &["country"],
        TableId::ProductionLines => &["country", "variant", "cost_raw", "active_factories"],
        TableId::VariantDefinitions => &["equipment_token", "origin_link", "creator_link", "display_name", "variant_name", "append_variant_name", "design"],
        _ => return None,
    })
}

pub mod naval_management;
pub mod naval_yards;
pub mod naval_supremacy;
pub mod minefields;

fn under<T: Into<u128>>(table: TableId, owners: impl IntoIterator<Item = T>, shift: u32) -> dashboard_base::products::at_hour::Read {
    let ranges = owners.into_iter().map(|owner| {
        let first = owner.into() << shift;
        (first, first | ((1u128 << shift) - 1))
    }).collect();
    dashboard_base::products::at_hour::Read { table, keys: replay_frame::frame::Keys::Ranges(ranges) }
}
