
use viewer_layer_core::{layers::{Inputs, List, Num}, owned::opaque};
use viewer_layer_core::playhead_host::mirror::{Cell, Mirror};
use replay_data::rows::{DirectedWars, Recorded, ReservedReverseVisibilityInputs, ReverseZeroVisibilityInputs, TableId};
use std::io;
use viewer_map_modes::naval_dominance::Supremacy;

pub(super) const TABLES: [TableId; 9] = [
    TableId::NavalSupremacy,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::FactionMembers,
    TableId::Subjects,
    TableId::ReservedCountryNaming,
    TableId::CountryCounterInputs,
    TableId::WarRelations,
    TableId::RelationTuples,
];

#[derive(Clone, Copy, Debug)]
pub(super) struct Columns {
    current: Num<i64>,
    previous: Num<i64>,
    link_slot: Num<u32>,
    country_slot: Num<u32>,
    faction: Num<u64>,
    naming_faction: Num<u64>,
    reverse_visibility: List,
    forward_visibility: List,
    directed_wars: List,
    tuple_first: Num<u32>,
    tuple_second: Num<u32>,
}

impl Columns {
    pub(super) fn declare(inputs: &mut Inputs<'_>) -> io::Result<Self> {
        for table in TABLES {
            inputs.table(table, true)?;
        }
        use TableId as T;
        Ok(Self {
            current: inputs.number(T::NavalSupremacy, "current")?,
            previous: inputs.number(T::NavalSupremacy, "previous")?,
            link_slot: inputs.number(T::CountryLinks, "canonical_slot")?,
            country_slot: inputs.number(T::Countries, "canonical_slot")?,
            faction: inputs.number(T::FactionMembers, "faction")?,
            naming_faction: inputs.number(T::ReservedCountryNaming, "faction")?,
            reverse_visibility: inputs.list(T::ReservedCountryNaming, "reverse_visibility")?,
            forward_visibility: inputs.list(T::CountryCounterInputs, "forward_visibility")?,
            directed_wars: inputs.list(T::CountryCounterInputs, "directed_wars")?,
            tuple_first: inputs.number(T::RelationTuples, "first_country")?,
            tuple_second: inputs.number(T::RelationTuples, "second_country")?,
        })
    }

    pub(super) fn part(&self, table: TableId, column: Option<usize>) -> u16 {
        let read = |held: usize| column.is_none_or(|column| column == held);
        match table {
            TableId::NavalSupremacy if read(self.current.column) || read(self.previous.column) => SUPREMACY,
            TableId::CountryLinks if read(self.link_slot.column) => LINKS,
            TableId::Countries if read(self.country_slot.column) => COUNTRIES,
            TableId::FactionMembers if read(self.faction.column) => FACTIONS,
            TableId::Subjects if column.is_none() => SUBJECTS,
            TableId::ReservedCountryNaming => {
                (if read(self.naming_faction.column) { NAMING_FACTION } else { 0 }) | if read(self.reverse_visibility.column) { NAMING_REVERSE } else { 0 }
            }
            TableId::CountryCounterInputs => {
                (if read(self.forward_visibility.column) { FORWARD } else { 0 }) | if read(self.directed_wars.column) { DIRECTED } else { 0 }
            }
            TableId::WarRelations if column.is_none() => WARS,
            TableId::RelationTuples if read(self.tuple_first.column) || read(self.tuple_second.column) => TUPLES,
            _ => 0,
        }
    }
}

pub(super) const SUPREMACY: u16 = 1;
pub(super) const LINKS: u16 = 1 << 1;
pub(super) const COUNTRIES: u16 = 1 << 2;
pub(super) const FACTIONS: u16 = 1 << 3;
pub(super) const SUBJECTS: u16 = 1 << 4;
pub(super) const NAMING_FACTION: u16 = 1 << 5;
pub(super) const NAMING_REVERSE: u16 = 1 << 6;
pub(super) const FORWARD: u16 = 1 << 7;
pub(super) const DIRECTED: u16 = 1 << 8;
pub(super) const WARS: u16 = 1 << 9;
pub(super) const TUPLES: u16 = 1 << 10;
pub(super) const HOUR: u16 = 1 << 11;

pub(super) fn supremacy(mirror: &Mirror, columns: &Columns, slot: u32, country: u32) -> Option<Supremacy> {
    let current = mirror.number(columns.current, slot).value()?;
    let previous = mirror.number(columns.previous, slot).value()?;
    Some(Supremacy { country, current, previous })
}

pub(super) fn supremacy_key(key: u128) -> (u32, u32) {
    ((key >> 32) as u32, key as u32)
}

pub(super) fn link(mirror: &Mirror, columns: &Columns, slot: u32) -> Option<(u32, u32)> {
    Some((mirror.key(slot)? as u32, mirror.number(columns.link_slot, slot).value()?))
}

pub(super) fn country(mirror: &Mirror, columns: &Columns, slot: u32) -> Option<(u32, u32)> {
    Some((mirror.key(slot)? as u32, mirror.number(columns.country_slot, slot).value()?))
}

pub(super) fn faction_member(mirror: &Mirror, columns: &Columns, slot: u32) -> Option<(u32, u64)> {
    Some((mirror.key(slot)? as u32, mirror.number(columns.faction, slot).value().unwrap_or(0)))
}

pub(super) fn subject(mirror: &Mirror, slot: u32) -> Option<(u32, u32)> {
    let key = mirror.key(slot)?;
    Some(((key >> 32) as u32, key as u32))
}

pub(super) fn naming_faction(mirror: &Mirror, columns: &Columns, slot: u32) -> Recorded<Option<u64>> {
    match mirror.number(columns.naming_faction, slot) {
        Cell::Value(handle) => Recorded::Value(Some(handle)),
        Cell::Missing => Recorded::Value(None),
        Cell::Unrecorded => Recorded::NotRecorded,
    }
}

pub(super) fn naming_reverse(mirror: &Mirror, columns: &Columns, slot: u32) -> Result<Recorded<Option<ReservedReverseVisibilityInputs>>, String> {
    opaque(mirror, columns.reverse_visibility, slot)
}

pub(super) fn counter_row(mirror: &Mirror, id: u32) -> Option<u32> {
    mirror.slot(u128::from(id)).filter(|&slot| mirror.live(slot))
}

pub(super) fn forward(mirror: &Mirror, columns: &Columns, slot: u32) -> Result<Recorded<Option<ReverseZeroVisibilityInputs>>, String> {
    opaque(mirror, columns.forward_visibility, slot)
}

pub(super) fn directed(mirror: &Mirror, columns: &Columns, slot: u32) -> Result<Recorded<Option<DirectedWars>>, String> {
    opaque(mirror, columns.directed_wars, slot)
}

pub(super) fn war(mirror: &Mirror, slot: u32) -> Option<(u32, u32)> {
    let key = mirror.key(slot)?;
    Some(((key >> 32) as u32, key as u32))
}

pub(super) fn tuple_kind(key: u128) -> u32 {
    (key >> 16) as u32
}

pub(super) fn tuple(mirror: &Mirror, columns: &Columns, slot: u32, kind: u32) -> Option<(u32, u32)> {
    (tuple_kind(mirror.key(slot)?) == kind).then_some(())?;
    Some((mirror.number(columns.tuple_first, slot).value()?, mirror.number(columns.tuple_second, slot).value()?))
}
