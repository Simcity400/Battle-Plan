//! Saved counterparts of the naval mission icon operands (NavalMissionTaskForces,
//! NavalMissionLedgers).
//!
//! unconditionally, `fuel` (`+0x4F8`) and `requested` (`+0x500`) only when nonzero, and one
//! `naval_headquarter` entry per attached headquarters (`+0x728`, count `+0x734`) only when
//! there is one. `CNavalMission::serialize@140FA9900` writes `mission` (`+0x14`) unconditionally and
//! `num_convoys_in_regions` (`+0x40`) only when strictly positive. An omitted convoy key
//! means nonpositive, not necessarily zero. `CFleet::serialize@140D4A390` writes
//! `leader` (`+0xB0`) only when it differs from the runtime invalid reference.
//! `CStrategicNavy::vt8_vf2@140E9D0C0` writes `convoy_escort_presence_history` with one ring
//! per region id that has one, head to tail. Absence is retained so the oracle can apply these
//! rules.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, Item, block, boolean, field, integer, invalid, object_id, scalar, typed_handle,
};

mod spotting_cache;
pub use spotting_cache::MissionSpottingCaches;
pub(in crate::save) use spotting_cache::spotting_caches;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub task_forces: BTreeMap<u64, TaskForce>,
    /// Saved fleet leader references by fleet handle; `None` when the field was omitted.
    pub fleet_leaders: BTreeMap<u64, Option<u64>>,
    /// Each country's saved presence rings by region id, entries head to tail.
    pub presence: BTreeMap<Vec<u8>, BTreeMap<u32, Vec<i32>>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskForce {
    pub fleet: u64,
    pub mission: i32,
    pub underway_replenishment: Option<bool>,
    pub fuel: Option<Decimal>,
    pub requested: Option<Decimal>,
    pub headquarters: usize,
    pub convoys: Option<i32>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    _items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for country in countries.iter() {
        let tag = country
            .key
            .as_deref()
            .filter(|tag| !tag.is_empty())
            .ok_or_else(|| invalid("country has no tag"))?;
        let fields = block(&country.value)?;
        if let Some(navy) = field(fields, b"strategic_navy")? {
            let mut rings = BTreeMap::new();
            if let Some(history) = field(block(navy)?, b"convoy_escort_presence_history")? {
                for ring in block(history)? {
                    let region = ring
                        .key
                        .as_deref()
                        .ok_or_else(|| invalid("presence ring has no region"))?;
                    let region = integer::<u32>(region)?;
                    let values = block(&ring.value)?
                        .iter()
                        .map(|entry| {
                            if entry.key.is_some() {
                                return Err(invalid("presence ring has a keyed entry"));
                            }
                            integer::<i32>(scalar(&entry.value)?)
                        })
                        .collect::<io::Result<Vec<_>>>()?;
                    if rings.insert(region, values).is_some() {
                        return Err(invalid("duplicate presence ring region"));
                    }
                }
            }
            if pack.presence.insert(tag.to_vec(), rings).is_some() {
                return Err(invalid("duplicate strategic navy country"));
            }
        }
        let Some(units) = field(fields, b"units")? else {
            continue;
        };
        for fleet_item in block(units)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"fleet"))
        {
            let fleet_fields = block(&fleet_item.value)?;
            let fleet = object_id(fleet_fields)?;
            let leader = field(fleet_fields, b"leader")?
                .map(typed_handle)
                .transpose()?;
            if pack.fleet_leaders.insert(fleet, leader).is_some() {
                return Err(invalid("duplicate full fleet handle in save"));
            }
            for task_item in fleet_fields
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"task_force"))
            {
                let task_fields = block(&task_item.value)?;
                let id = object_id(task_fields)?;
                let mission = field(task_fields, b"mission")?
                    .map(block)
                    .transpose()?
                    .ok_or_else(|| invalid("saved task force lacks mission"))?;
                let decimal = |key: &[u8]| {
                    field(task_fields, key)?
                        .map(|value| Decimal::parse(scalar(value)?))
                        .transpose()
                };
                let task = TaskForce {
                    fleet,
                    mission: field(mission, b"mission")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?
                        .ok_or_else(|| invalid("saved mission lacks its kind"))?,
                    underway_replenishment: field(task_fields, b"underway_replenishment")?
                        .map(boolean)
                        .transpose()?,
                    fuel: decimal(b"fuel")?,
                    requested: decimal(b"requested")?,
                    headquarters: field(task_fields, b"naval_headquarter")?
                        .map(|value| block(value).map(<[Item]>::len))
                        .transpose()?
                        .unwrap_or(0),
                    convoys: field(mission, b"num_convoys_in_regions")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                };
                if pack.task_forces.insert(id, task).is_some() {
                    return Err(invalid("duplicate full task force handle in save"));
                }
            }
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mission_kind_is_mandatory_and_convoy_omission_is_preserved() {
        let read = |fields: &str| {
            parse(&crate::parser::parse(format!(
            "countries={{ X={{ units={{ fleet={{ id={{id=1 type=61}} task_force={{id={{id=2 type=61}} mission={{{fields}}} }} }} }} }} }}"
        ).as_bytes()).unwrap().items)
        };
        assert!(read("").unwrap_err().to_string().contains("lacks its kind"));
        let key = (2 << 32) | 61;
        assert_eq!(read("mission=0").unwrap().task_forces[&key].convoys, None);
        assert_eq!(
            read("mission=4 num_convoys_in_regions=19")
                .unwrap()
                .task_forces[&key]
                .convoys,
            Some(19)
        );
    }

    #[test]
    fn mission_operands_keep_absence_and_presence_rings_keep_order() {
        let saved = parse(
            &crate::parser::parse(
                br#"countries={
                ENG={ strategic_navy={ max_allowed_dockyards_to_repair=46
                    convoy_escort_presence_history={ "9"={ -1 -1 0 2 } "206"={ } } }
                  units={ fleet={ id={ id=1 type=61 } leader={ id=897 type=4713 }
                    task_force={ id={ id=43 type=61 } mission={ mission=4 is_in_regions=yes num_convoys_in_regions=210 navy_engagement_rule=2 }
                      underway_replenishment=no fuel=16.32 requested=16.5
                      naval_headquarter={ { id=1032 type=4713 } { id=1033 type=4713 } } }
                    task_force={ id={ id=44 type=61 } mission={ mission=0 navy_engagement_rule=2 } underway_replenishment=yes } }
                    fleet={ id={ id=2 type=61 } } } }
                GER={} }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let escort = &saved.task_forces[&((43 << 32) | 61)];
        assert_eq!(escort.mission, 4);
        assert_eq!(escort.underway_replenishment, Some(false));
        assert_eq!(escort.fuel.as_ref().map(|fuel| fuel.raw), Some(1_632_000));
        assert_eq!(
            escort.requested.as_ref().map(|fuel| fuel.raw),
            Some(1_650_000)
        );
        assert_eq!((escort.headquarters, escort.convoys), (2, Some(210)));
        let hold = &saved.task_forces[&((44 << 32) | 61)];
        assert_eq!(
            (hold.fuel.clone(), hold.headquarters, hold.convoys),
            (None, 0, None)
        );
        assert_eq!(
            saved.fleet_leaders[&((1 << 32) | 61)],
            Some((897 << 32) | 4713)
        );
        assert_eq!(saved.fleet_leaders[&((2 << 32) | 61)], None);
        let rings = &saved.presence[&b"ENG".to_vec()];
        assert_eq!(rings[&9], [-1, -1, 0, 2]);
        assert!(rings[&206].is_empty());
        assert!(!saved.presence.contains_key(b"GER".as_slice()));
    }
}
