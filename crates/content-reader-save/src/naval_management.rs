use super::{
    CountriesView, Item, block, boolean, bytes, field, integer, invalid, object_id, scalar,
    typed_handle,
};
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub fleets: BTreeMap<u64, Fleet>,
    pub task_forces: BTreeMap<u64, TaskForce>,
    pub ships: BTreeMap<u64, Ship>,
    pub bases: Vec<Base>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub max_factories_for_repair: Option<i32>,
    pub max_allowed_dockyards_to_repair: Option<i32>,
    pub num_used_dockyards: Option<i32>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fleet {
    pub country: Vec<u8>,
    pub automated_homebase: Option<bool>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ship {
    pub country: Vec<u8>,
    pub refit_production_line: Option<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskForce {
    pub country: Vec<u8>,
    pub repair_mode: Option<i32>,
    pub repair_split: bool,
    pub repair_target: Option<u32>,
    pub auto_reinforcement: bool,
    pub refit_variant: Option<u64>,
    pub refit_after_repair: Option<u64>,
    pub composition: Option<Composition>,
    pub merge_after_repair: Option<u64>,
    pub repair_children: Vec<u64>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composition {
    pub requirements: Option<Vec<Entry>>,
    pub fulfillment: Option<Vec<Entry>>,
    pub template: Option<Vec<u8>>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub sub_unit: i32,
    pub role: i32,
    pub amount: i32,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Base {
    pub country: Vec<u8>,
    pub province: i32,
    pub priority: i32,
    pub ships: Vec<u64>,
}

fn number<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    field(items, key)?
        .map(|value| integer(scalar(value)?))
        .transpose()
}
fn flag(items: &[Item], key: &[u8]) -> io::Result<Option<bool>> {
    field(items, key)?.map(boolean).transpose()
}
fn reference(items: &[Item], key: &[u8]) -> io::Result<Option<u64>> {
    field(items, key)?.map(typed_handle).transpose()
}
fn integers(items: &[Item], key: &[u8]) -> io::Result<Vec<i32>> {
    field(items, key)?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid(
                            "naval composition integer list contains a keyed value",
                        ));
                    }
                    integer(scalar(&item.value)?)
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}
fn entries(items: &[Item]) -> io::Result<Vec<Entry>> {
    let units = integers(items, b"sub_units")?;
    let roles = integers(items, b"roles")?;
    let amounts = integers(items, b"amount")?;
    if units.len() != roles.len() || units.len() != amounts.len() {
        return Err(invalid(
            "naval composition parallel lists have different lengths",
        ));
    }
    Ok(units
        .into_iter()
        .zip(roles)
        .zip(amounts)
        .map(|((sub_unit, role), amount)| Entry {
            sub_unit,
            role,
            amount,
        })
        .collect())
}
fn composition(items: &[Item]) -> io::Result<Composition> {
    Ok(Composition {
        requirements: field(items, b"requirements")?
            .map(|value| entries(block(value)?))
            .transpose()?,
        fulfillment: field(items, b"fulfillment")?
            .map(|value| entries(block(value)?))
            .transpose()?,
        template: bytes(items, b"ai_taskforce_composition")?,
    })
}

pub(super) fn parse_with_countries(
    _items: &[Item],
    countries: CountriesView<'_>,
) -> io::Result<Pack> {
    let mut output = Pack::default();
    for item in countries.iter() {
        let tag = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("naval repair country has no tag"))?;
        let items = block(&item.value)?;
        let mut country = Country::default();
        if let Some(production) = field(items, b"production")? {
            country.max_factories_for_repair =
                number(block(production)?, b"max_factories_for_repair")?;
        }
        if let Some(navy) = field(items, b"strategic_navy")? {
            let navy = block(navy)?;
            country.max_allowed_dockyards_to_repair =
                number(navy, b"max_allowed_dockyards_to_repair")?;
            country.num_used_dockyards = number(navy, b"num_used_dockyards")?;
            if let Some(bases) = field(navy, b"naval_base")? {
                for entry in block(bases)? {
                    let province = integer(
                        entry
                            .key
                            .as_deref()
                            .ok_or_else(|| invalid("naval repair base has no province key"))?,
                    )?;
                    let base = block(&entry.value)?;
                    let ships = field(base, b"ships_in_repair")?
                        .map(|value| {
                            block(value)?
                                .iter()
                                .map(|entry| {
                                    if entry.key.is_some() {
                                        return Err(invalid(
                                            "naval repair ship list contains a keyed entry",
                                        ));
                                    }
                                    typed_handle(&entry.value)
                                })
                                .collect::<io::Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default();
                    output.bases.push(Base {
                        country: tag.to_vec(),
                        province,
                        priority: number(base, b"priority")?.unwrap_or(1),
                        ships,
                    });
                }
            }
        }
        if output.countries.insert(tag.to_vec(), country).is_some() {
            return Err(invalid("duplicate naval repair country"));
        }
        let Some(units) = field(items, b"units")? else {
            continue;
        };
        for fleet in block(units)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"fleet"))
        {
            let fleet = block(&fleet.value)?;
            if output
                .fleets
                .insert(
                    object_id(fleet)?,
                    Fleet {
                        country: tag.to_vec(),
                        automated_homebase: flag(fleet, b"automated_homebase")?,
                    },
                )
                .is_some()
            {
                return Err(invalid("duplicate naval repair fleet"));
            }
            for task in fleet
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"task_force"))
            {
                let task = block(&task.value)?;
                let row = TaskForce {
                    country: tag.to_vec(),
                    repair_mode: number(task, b"repair_mode")?,
                    merge_after_repair: reference(task, b"merge_with_after_repair")?,
                    repair_children: task
                        .iter()
                        .filter(|item| item.key.as_deref() == Some(b"repair_child"))
                        .map(|item| typed_handle(&item.value))
                        .collect::<io::Result<Vec<_>>>()?,
                    repair_split: flag(task, b"repair_split")?.unwrap_or(false),
                    repair_target: number(task, b"repair_target")?,
                    auto_reinforcement: flag(task, b"auto_reinforcement")?.unwrap_or(false),
                    refit_variant: reference(task, b"refit_equipment_variant")?,
                    refit_after_repair: reference(task, b"refit_to_variant_after_repair")?,
                    composition: field(task, b"ai_taskforce_composition")?
                        .map(|value| composition(block(value)?))
                        .transpose()?,
                };
                if output.task_forces.insert(object_id(task)?, row).is_some() {
                    return Err(invalid("duplicate naval repair task force"));
                }
                for ship in task
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"ship"))
                {
                    let ship = block(&ship.value)?;
                    if output
                        .ships
                        .insert(
                            object_id(ship)?,
                            Ship {
                                country: tag.to_vec(),
                                refit_production_line: reference(ship, b"refit_production_line")?,
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate naval refit ship"));
                    }
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(input: &[u8]) -> io::Result<Pack> {
        let document = content_reader_parser::parser::parse_save(input)?;
        parse_with_countries(&document.items, CountriesView::from_root(&document.items)?)
    }
    #[test]
    fn naval_management_save_keeps_policies_designs_parallel_entries_and_queue_order() {
        let saved = parse(br#"countries={ MOD={
          production={max_factories_for_repair=-7}
          strategic_navy={max_allowed_dockyards_to_repair=13 num_used_dockyards=2
            naval_base={9={priority=4 ships_in_repair={{type=51 id=8}{type=51 id=3}{type=51 id=8}}}}}
          units={fleet={id={type=61 id=1} automated_homebase=yes
            task_force={id={type=61 id=2} repair_mode=3 repair_split=yes repair_target=9
              auto_reinforcement=yes repair_child={type=61 id=3} repair_child={type=61 id=4}
              merge_with_after_repair={type=61 id=6} refit_equipment_variant={type=70 id=7}
              refit_to_variant_after_repair={type=70 id=4}
              ai_taskforce_composition={requirements={sub_units={17 17} roles={5 0} amount={9 -2}}
                fulfillment={} ai_taskforce_composition=mod_patrol}
              ship={id={type=51 id=8} refit_production_line={type=71 id=6}}
            }}} }}"#).unwrap();
        let task = &saved.task_forces[&(61 | (2 << 32))];
        assert_eq!(saved.fleets[&(61 | (1 << 32))].country, b"MOD");
        assert_eq!(task.country, b"MOD");
        let ship = &saved.ships[&(51 | (8 << 32))];
        assert_eq!(ship.country, b"MOD");
        assert_eq!(ship.refit_production_line, Some(71 | (6 << 32)));
        assert_eq!(
            (
                task.repair_mode,
                task.repair_split,
                task.repair_target,
                task.auto_reinforcement
            ),
            (Some(3), true, Some(9), true)
        );
        assert_ne!(task.refit_variant, task.refit_after_repair);
        assert_eq!(task.repair_children, [61 | (3 << 32), 61 | (4 << 32)]);
        assert_eq!(task.merge_after_repair, Some(61 | (6 << 32)));
        let composition = task.composition.as_ref().unwrap();
        assert_eq!(
            composition.requirements.as_ref().unwrap(),
            &vec![
                Entry {
                    sub_unit: 17,
                    role: 5,
                    amount: 9
                },
                Entry {
                    sub_unit: 17,
                    role: 0,
                    amount: -2
                }
            ]
        );
        assert_eq!(composition.fulfillment, Some(Vec::new()));
        assert_eq!(
            composition.template.as_deref(),
            Some(b"mod_patrol".as_slice())
        );
        assert_eq!(
            saved.bases[0].ships,
            [51 | (8 << 32), 51 | (3 << 32), 51 | (8 << 32)]
        );
        assert_eq!(
            saved.countries[b"MOD".as_slice()].max_factories_for_repair,
            Some(-7)
        );
    }
    #[test]
    fn naval_management_save_retains_missing_required_keys_and_rejects_mismatched_lists() {
        let saved = parse(b"countries={MOD={units={fleet={id={type=61 id=1}}}}}").unwrap();
        assert_eq!(saved.fleets[&(61 | (1 << 32))].automated_homebase, None);
        assert_eq!(
            saved.countries[b"MOD".as_slice()].max_allowed_dockyards_to_repair,
            None
        );
        assert!(parse(b"countries={MOD={units={fleet={id={type=61 id=1} task_force={id={type=61 id=2} ai_taskforce_composition={requirements={sub_units={1 2} roles={0} amount={3 4}}}}}}}}}").is_err());
    }
}
