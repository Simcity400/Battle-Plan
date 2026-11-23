
use super::{Columns as Counted, Tables, bytes_of, row, u32s, u64s};
use viewer_layer_core::{layers::{Inputs, List, Num}, owned::opaque, playhead_host::mirror::{Mirror, Mirrors}};
use crate::relations::{Facts as Wars, WarUnknown};
use replay_data::rows::{DivisionMovementRow, Field, NativeValue, NavalCombatSideDetails, Opaque, Recorded, StrategicAction, TableId, UnitCombatReferences};
use std::{
    cell::OnceCell,
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
};
use viewer_unit_reads::counter_action::{self, Action};

pub(super) const TABLES: [TableId; 6] = [
    TableId::Provinces,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::NavalCombatDetails,
];

#[derive(Clone, Copy)]
pub(super) struct Columns {
    retreat: Num<u8>,
    strategic_redeployment: Num<u8>,
    strategic_action: List,
    combat_references: List,
    controller: Num<u32>,
    transfers: List,
    transfer_units: List,
    attackers: List,
    defenders: List,
    attacker_primary: List,
    naval_sides: List,
}

impl Columns {
    pub(super) fn declare(inputs: &mut Inputs<'_>) -> io::Result<Self> {
        use TableId as T;
        for table in TABLES {
            inputs.table(table, true)?;
        }
        Ok(Self {
            retreat: inputs.number(T::DivisionMovement, "retreat")?,
            strategic_redeployment: inputs.number(T::DivisionMovement, "strategic_redeployment")?,
            strategic_action: inputs.list(T::DivisionMovement, "strategic_action")?,
            combat_references: inputs.list(T::DivisionMovement, "combat_references")?,
            controller: inputs.number(T::Provinces, "controller")?,
            transfers: inputs.list(T::NavalCombatTransfers, "transfers")?,
            transfer_units: inputs.list(T::NavalTransferUnits, "units")?,
            attackers: inputs.list(T::LandCombats, "attackers")?,
            defenders: inputs.list(T::LandCombats, "defenders")?,
            attacker_primary: inputs.list(T::LandCombatSides, "attacker_primary")?,
            naval_sides: inputs.list(T::NavalCombatDetails, "sides")?,
        })
    }

    pub(super) fn reads(&self, table: TableId, column: usize) -> bool {
        match table {
            TableId::Provinces => column == self.controller.column,
            TableId::LandCombats => column == self.attackers.column || column == self.defenders.column,
            TableId::LandCombatSides => column == self.attacker_primary.column,
            TableId::NavalCombatDetails => column == self.naval_sides.column,
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Read {
    pub(super) destination: Option<u32>,
    pub(super) transfer: bool,
    pub(super) standing: bool,
}

pub(super) struct Facts<'a> {
    mirrors: &'a Mirrors,
    columns: Columns,
    hour: Option<i32>,
    links: OnceCell<BTreeMap<u32, u32>>,
    transfer_units: OnceCell<(HashSet<u64>, bool)>,
    membership: OnceCell<BTreeMap<u64, BTreeSet<u64>>>,
}

impl<'a> Facts<'a> {
    pub(super) fn new(mirrors: &'a Mirrors, columns: Columns) -> Self {
        let hour = mirrors.get(TableId::DivisionMovement).and_then(Mirror::head);
        Self { mirrors, columns, hour, links: OnceCell::new(), transfer_units: OnceCell::new(), membership: OnceCell::new() }
    }

    fn links(&self, tables: &Tables<'_>, counted: Counted) -> &BTreeMap<u32, u32> {
        self.links.get_or_init(|| {
            let Some(links) = tables.links else { return BTreeMap::new() };
            links.rows().filter_map(|slot| Some((links.key(slot)? as u32, links.number(counted.link_slot, slot).value()?))).collect()
        })
    }

    fn transfer(&self, division: u64) -> Option<bool> {
        let (units, complete) = self.transfer_units.get_or_init(|| {
            let (combats, transfers) = (self.mirrors.get(TableId::NavalCombatTransfers), self.mirrors.get(TableId::NavalTransferUnits));
            let complete = combats.is_some_and(Mirror::complete) && transfers.is_some_and(Mirror::complete);
            let named: HashSet<u64> = combats.into_iter().flat_map(|combats| combats.rows().flat_map(|slot| u64s(bytes_of(combats.list(self.columns.transfers, slot))).collect::<Vec<_>>())).filter(|&transfer| transfer != 0).collect();
            let units = transfers
                .into_iter()
                .flat_map(|transfers| {
                    transfers
                        .rows()
                        .filter(|&slot| transfers.key(slot).is_some_and(|key| named.contains(&(key as u64))))
                        .flat_map(|slot| u64s(bytes_of(transfers.list(self.columns.transfer_units, slot))).collect::<Vec<_>>())
                        .collect::<Vec<_>>()
                })
                .collect();
            (units, complete)
        });
        counter_action::transfer_membership(units.contains(&division), *complete)
    }

    fn attackers(&self, kind: u8, combat: u64) -> Option<Vec<u32>> {
        if kind <= 1 {
            let sides = self.mirrors.get(TableId::LandCombatSides)?;
            let slot = sides.slot(u128::from(combat))?;
            return Some(u32s(bytes_of(sides.list(self.columns.attacker_primary, slot))).collect());
        }
        let details = self.mirrors.get(TableId::NavalCombatDetails)?;
        let slot = details.slot(u128::from(combat))?;
        let cell = details.list(self.columns.naval_sides, slot).value()?;
        let sides = <[NavalCombatSideDetails; 2]>::from_field(Field::Opaque(Opaque::Encoded(cell, details.format()))).ok()?;
        let [first, _] = sides;
        Some(first.summary.countries)
    }

    fn combats_of(&self, division: u64) -> Option<&BTreeSet<u64>> {
        self.membership
            .get_or_init(|| {
                let mut membership: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
                if let Some(combats) = self.mirrors.get(TableId::LandCombats) {
                    for slot in combats.rows() {
                        let Some(combat) = combats.key(slot) else { continue };
                        for member in u64s(bytes_of(combats.list(self.columns.attackers, slot))).chain(u64s(bytes_of(combats.list(self.columns.defenders, slot)))) {
                            membership.entry(member).or_default().insert(combat as u64);
                        }
                    }
                }
                membership
            })
            .get(&division)
    }

    pub(super) fn action(&self, tables: &Tables<'_>, counted: Counted, id: u64, owner: u32, wars: Option<&Wars>, strategic_layout: bool) -> (Action, Read) {
        let mut read = Read::default();
        let Some((movement, slot)) = row(tables.movement, u128::from(id)) else {
            let complete = self.mirrors.get(TableId::DivisionMovement).is_some_and(Mirror::complete);
            return (Err(if complete { "DivisionMovement division" } else { "DivisionMovement" }), read);
        };
        let number = |column: Num<u32>| movement.number(column, slot).value().unwrap_or(0);
        let byte = |column: Num<u8>| movement.number(column, slot).value().unwrap_or(0);
        let fields = DivisionMovementRow {
            division: id,
            target: number(counted.target),
            retreat: byte(self.columns.retreat),
            strategic_redeployment: byte(self.columns.strategic_redeployment),
            naval_transfer: byte(counted.naval_transfer),
            combat_count: number(counted.combat_count),
            path_total: number(counted.path_total),
            strategic_action: opaque::<StrategicAction>(movement, self.columns.strategic_action, slot).unwrap_or(Recorded::NotRecorded),
            ..DivisionMovementRow::default()
        };
        let standing = ((fields.path_total as i32) <= 0 && (fields.combat_count as i32) > 0).then(|| {
            read.standing = true;
            let links = self.links(tables, counted);
            match opaque::<Option<UnitCombatReferences>>(movement, self.columns.combat_references, slot) {
                Ok(Recorded::NotRecorded) => {
                    let combats_complete = self.mirrors.get(TableId::LandCombats).is_some_and(Mirror::complete);
                    let references = self.combats_of(id)?;
                    counter_action::membership_standing(owner, references, |combat| self.attackers(0, combat), links, combats_complete, fields.combat_count)
                }
                Ok(Recorded::Value(Some(packet))) if self.hour.is_some_and(|hour| packet.valid_at(hour, fields.combat_count)) => {
                    counter_action::recorded_standing_by(&packet, owner, links, |kind, combat| self.attackers(kind, combat))
                }
                _ => None,
            }
        });
        let transfer = (fields.naval_transfer != 0).then(|| {
            read.transfer = true;
            self.transfer(id)
        });
        let path = bytes_of(movement.list(counted.path, slot));
        let action = counter_action::division_action(
            &fields,
            standing.flatten(),
            transfer.flatten(),
            || {
                let path: Vec<u32> = u32s(path).collect();
                read.destination = path.last().copied();
                let provinces = self.mirrors.get(TableId::Provinces);
                counter_action::destination_enemy_by(
                    &path,
                    fields.path_total,
                    Some(owner),
                    |province| provinces.and_then(|provinces| provinces.number(self.columns.controller, provinces.slot(u128::from(province))?).value()),
                    |controller, owner| match wars.map(|wars| wars.at_war(controller, owner)) {
                        Some(Ok(war)) => Ok(war),
                        None | Some(Err(WarUnknown::Receiver(_))) => Err("action destination directed-war receiver unavailable"),
                        Some(Err(WarUnknown::Operands(_))) => Err("action destination directed-war operands unavailable"),
                    },
                )
            },
            strategic_layout,
        );
        (action, read)
    }
}
