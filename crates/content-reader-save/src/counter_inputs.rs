
use std::{collections::BTreeMap, io};

use super::{
    Item, block, boolean, field, integer, integer_list, invalid, object_id, scalar, typed_handle,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub prides: BTreeMap<Vec<u8>, Option<u64>>,
    pub sent_volunteers: BTreeMap<Vec<u8>, Vec<u64>>,
    pub sent_expeditionaries: BTreeMap<Vec<u8>, Vec<u64>>,
    pub first_divisions: BTreeMap<Vec<u8>, u64>,
    pub division_orders: BTreeMap<Vec<u8>, Vec<u64>>,
    pub fleet_orders: BTreeMap<Vec<u8>, Vec<u64>>,
    pub fleets: BTreeMap<u64, Fleet>,
    pub task_forces: BTreeMap<u64, TaskForce>,
    pub naval_transfers: BTreeMap<u64, Vec<u64>>,
    pub naval_transfer_inputs: BTreeMap<u64, NavalTransfer>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalTransfer {
    pub source: Option<u32>,
    pub combat: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fleet {
    pub owner: Vec<u8>,
    pub color: Option<Vec<i64>>,
    pub strategic_regions: Option<Vec<u32>>,
    pub task_forces: Vec<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskForce {
    pub fleet: u64,
    pub is_in_regions: Option<bool>,
    pub engagement_rule: Option<i32>,
    pub bombardment_region: Option<u32>,
    pub spotting_region: Option<u32>,
    pub accessible_regions: Option<Vec<u32>>,
    pub repair_parent: Option<u64>,
    pub detached_activity: Option<Vec<u8>>,
    pub repair_last_mission: Option<i32>,
    pub attached_headquarters: usize,
    pub spotters: Vec<u64>,
    pub strike_force_target: Option<u64>,
    pub target_ship_types: Vec<Vec<u8>>,
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
        let sent = field(fields, b"volunteers_sent")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|entry| {
                        if entry.key.is_some() {
                            return Err(invalid("keyed volunteers_sent entry"));
                        }
                        super::null_permitted_handle(&entry.value)
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        pack.sent_volunteers.insert(tag.to_vec(), sent);
        let expeditionaries = field(fields, b"expeditionaries_sent")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|entry| {
                        if entry.key.is_some() {
                            return Err(invalid("keyed expeditionaries_sent entry"));
                        }
                        super::null_permitted_handle(&entry.value)
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        pack.sent_expeditionaries
            .insert(tag.to_vec(), expeditionaries);
        let pride = field(fields, b"pride_of_the_fleet")?
            .map(typed_handle)
            .transpose()?;
        if pack.prides.insert(tag.to_vec(), pride).is_some() {
            return Err(invalid("duplicate country tag in save"));
        }
        if let Some(navy) = field(fields, b"strategic_navy")? {
            for transfer in block(navy)?
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"naval_transport"))
            {
                let transfer = block(&transfer.value)?;
                let units = transfer
                    .iter()
                    .filter(|item| item.key.as_deref() == Some(b"unit"))
                    .map(|item| typed_handle(&item.value))
                    .collect::<io::Result<Vec<_>>>()?;
                let handle = object_id(transfer)?;
                let inputs = NavalTransfer {
                    source: field(transfer, b"province")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    combat: field(transfer, b"combat")?
                        .map(|value| {
                            block(value)?
                                .iter()
                                .map(|entry| {
                                    if entry.key.is_some() {
                                        return Err(invalid("keyed naval transfer combat entry"));
                                    }
                                    super::null_permitted_handle(&entry.value)
                                })
                                .collect::<io::Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default(),
                };
                if pack.naval_transfers.insert(handle, units).is_some() {
                    return Err(invalid("duplicate naval transfer handle in save"));
                }
                pack.naval_transfer_inputs.insert(handle, inputs);
            }
        }
        pack.fleet_orders.insert(tag.to_vec(), Vec::new());
        let Some(units) = field(fields, b"units")? else {
            pack.division_orders.insert(tag.to_vec(), Vec::new());
            continue;
        };
        let divisions = block(units)?
            .iter()
            .filter(|item| item.keyword_is("division"))
            .map(|item| object_id(block(&item.value)?))
            .collect::<io::Result<Vec<_>>>()?;
        if let Some(&division) = divisions.first() {
            pack.first_divisions.insert(tag.to_vec(), division);
        }
        pack.division_orders.insert(tag.to_vec(), divisions);
        for fleet_item in block(units)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"fleet"))
        {
            let fleet_fields = block(&fleet_item.value)?;
            let id = object_id(fleet_fields)?;
            pack.fleet_orders.get_mut(tag).unwrap().push(id);
            let mut fleet = Fleet {
                owner: tag.to_vec(),
                color: field(fleet_fields, b"color")?
                    .map(|value| {
                        block(value)?
                            .iter()
                            .map(|entry| {
                                if entry.key.is_some() {
                                    return Err(invalid("fleet color has keyed entry"));
                                }
                                integer::<i64>(scalar(&entry.value)?)
                            })
                            .collect::<io::Result<Vec<_>>>()
                    })
                    .transpose()?,
                strategic_regions: field(fleet_fields, b"strategic_region")?
                    .is_some()
                    .then(|| integer_list(fleet_fields, b"strategic_region"))
                    .transpose()?,
                task_forces: Vec::new(),
            };
            for task_item in fleet_fields
                .iter()
                .filter(|item| item.key.as_deref() == Some(b"task_force"))
            {
                let task_fields = block(&task_item.value)?;
                let task_id = object_id(task_fields)?;
                fleet.task_forces.push(task_id);
                let mission = field(task_fields, b"mission")?
                    .map(block)
                    .transpose()?
                    .ok_or_else(|| invalid("saved task force lacks mission"))?;
                let task = TaskForce {
                    fleet: id,
                    is_in_regions: field(mission, b"is_in_regions")?.map(boolean).transpose()?,
                    engagement_rule: field(mission, b"navy_engagement_rule")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    bombardment_region: field(mission, b"bombardment_region")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    spotting_region: field(mission, b"spotting_region")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    accessible_regions: field(mission, b"accessible_regions")?
                        .is_some()
                        .then(|| integer_list(mission, b"accessible_regions"))
                        .transpose()?,
                    repair_parent: field(task_fields, b"repair_parent")?
                        .map(typed_handle)
                        .transpose()?,
                    detached_activity: field(task_fields, b"detached_activity")?
                        .map(|value| scalar(value).map(<[u8]>::to_vec))
                        .transpose()?,
                    repair_last_mission: field(task_fields, b"repair_last_mission")?
                        .map(|value| integer(scalar(value)?))
                        .transpose()?,
                    attached_headquarters: match field(task_fields, b"naval_headquarter")? {
                        Some(value) => {
                            let entries = block(value)?;
                            for entry in entries {
                                if entry.key.is_some() {
                                    return Err(invalid("naval_headquarter has a keyed entry"));
                                }
                                typed_handle(&entry.value)?;
                            }
                            entries.len()
                        }
                        None => 0,
                    },
                    spotters: task_fields
                        .iter()
                        .filter(|item| item.key.as_deref() == Some(b"spotters"))
                        .map(|item| super::null_permitted_handle(&item.value))
                        .collect::<io::Result<Vec<_>>>()?,
                    strike_force_target: field(mission, b"strike_force_target")?
                        .map(super::null_permitted_handle)
                        .transpose()?,
                    target_ship_types: field(task_fields, b"target_ship_types")?
                        .map(|value| {
                            block(value)?
                                .iter()
                                .map(|item| {
                                    if item.key.is_some() {
                                        return Err(invalid("keyed target_ship_types entry"));
                                    }
                                    scalar(&item.value).map(<[u8]>::to_vec)
                                })
                                .collect::<io::Result<Vec<_>>>()
                        })
                        .transpose()?
                        .unwrap_or_default(),
                };
                if pack.task_forces.insert(task_id, task).is_some() {
                    return Err(invalid("duplicate full task force handle in save"));
                }
            }
            if pack.fleets.insert(id, fleet).is_some() {
                return Err(invalid("duplicate full fleet handle in save"));
            }
        }
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_mission_fields_keep_raw_references_and_ordered_string_bytes() {
        let parse_task = |body: &[u8]| {
            let mut text =
                b"countries={AAA={units={fleet={id={id=1 type=61} task_force={id={id=2 type=51} "
                    .to_vec();
            text.extend_from_slice(body);
            text.extend_from_slice(b"}}}}}");
            let doc = content_reader_parser::parser::parse(&text)?;
            parse(&doc.items)
        };
        let pack = parse_task(
            br#"spotters={id=9 type=51} spotters={id=0 type=0}
            spotters={id=9 type=51} target_ship_types={"" "z" "z" "a\\b"}
            mission={strike_force_target={id=0 type=4294967295}}"#,
        )
        .unwrap();
        let task = &pack.task_forces[&((2 << 32) | 51)];
        assert_eq!(task.spotters, [(9 << 32) | 51, 0, (9 << 32) | 51]);
        assert_eq!(task.strike_force_target, Some(u64::from(u32::MAX)));
        assert_eq!(
            task.target_ship_types,
            [b"".to_vec(), b"z".to_vec(), b"z".to_vec(), b"a\\b".to_vec()]
        );
        let pack = parse_task(b"mission={}").unwrap();
        let task = &pack.task_forces[&((2 << 32) | 51)];
        assert!(task.spotters.is_empty());
        assert_eq!(task.strike_force_target, None);
        assert!(task.target_ship_types.is_empty());
        for body in [
            "mission={} spotters={id=1}",
            "mission={} spotters={id=-1 type=51}",
            "mission={} target_ship_types={key=x}",
            "mission={} target_ship_types={} target_ship_types={}",
            "mission={strike_force_target={id=1 type=51} strike_force_target={id=1 type=51}}",
        ] {
            assert!(parse_task(body.as_bytes()).is_err(), "{body}");
        }
    }

    #[test]
    fn naval_counter_fields_keep_order_absence_and_literal_values() {
        let saved = parse(
            &content_reader_parser::parser::parse(
                br#"countries={
                GER={ pride_of_the_fleet={ id=32 type=51 }
                  units={ division={ id={ id=7 type=4 } } division={ id={ id=3 type=4 } } fleet={ id={ id=1 type=61 } color={ 219 219 10 }
                    task_force={ id={ id=43 type=51 } mission={ mission=0 is_in_regions=yes navy_engagement_rule=2 } }
                    task_force={ id={ id=44 type=51 } repair_parent={ id=564 type=51 } detached_activity=repairing repair_last_mission=2
                      naval_headquarter={ { id=1032 type=4713 } { id=1033 type=4713 } }
                      mission={ mission=9 navy_engagement_rule=3 spotting_region=206 accessible_regions={ 206 9 206 } bombardment_region=206 } }
                    strategic_region={ 16 18 43 } } }
                  strategic_navy={ naval_transport={ id={ id=9 type=4713 } province=2661
                    unit={ id=59333 type=4713 } unit={ id=12 type=4713 } combat={ { id=3 type=4713 } } } } }
                ENG={} }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        assert_eq!(saved.prides[&b"GER".to_vec()], Some((32 << 32) | 51));
        assert_eq!(saved.prides[&b"ENG".to_vec()], None);
        assert_eq!(saved.first_divisions[&b"GER".to_vec()], (7 << 32) | 4);
        assert_eq!(
            saved.division_orders[b"GER".as_slice()],
            [(7 << 32) | 4, (3 << 32) | 4]
        );
        assert!(!saved.first_divisions.contains_key(b"ENG".as_slice()));
        let fleet = &saved.fleets[&((1 << 32) | 61)];
        assert_eq!(fleet.color, Some(vec![219, 219, 10]));
        assert_eq!(fleet.strategic_regions, Some(vec![16, 18, 43]));
        assert_eq!(fleet.task_forces, [(43 << 32) | 51, (44 << 32) | 51]);
        let first = &saved.task_forces[&((43 << 32) | 51)];
        assert_eq!(
            (
                first.is_in_regions,
                first.engagement_rule,
                first.spotting_region,
                first.accessible_regions.clone()
            ),
            (Some(true), Some(2), None, None)
        );
        let second = &saved.task_forces[&((44 << 32) | 51)];
        assert_eq!(second.bombardment_region, Some(206));
        assert_eq!(second.spotting_region, Some(206));
        assert_eq!(second.accessible_regions, Some(vec![206, 9, 206]));
        assert_eq!(second.repair_parent, Some((564 << 32) | 51));
        assert_eq!(second.detached_activity.as_deref(), Some(&b"repairing"[..]));
        assert_eq!(second.repair_last_mission, Some(2));
        assert_eq!(
            (first.attached_headquarters, second.attached_headquarters),
            (0, 2)
        );
        assert_eq!(
            saved.naval_transfers[&((9 << 32) | 4713)],
            [(59333 << 32) | 4713, (12 << 32) | 4713]
        );
        assert_eq!(
            saved.naval_transfer_inputs[&((9 << 32) | 4713)],
            NavalTransfer {
                source: Some(2661),
                combat: vec![(3 << 32) | 4713],
            }
        );
        assert_eq!(
            (first.detached_activity.clone(), first.repair_last_mission),
            (None, None)
        );
    }
    #[test]
    fn transfer_source_is_not_target_and_combat_keeps_null_and_repeated_occurrences() {
        let doc = content_reader_parser::parser::parse(
            br"countries={ X={ strategic_navy={ naval_transport={
            id={id=2 type=4713} target=91 province=42 combat={
                {id=7 type=4713} {id=0 type=0} {id=7 type=4713}
            }
        } naval_transport={id={id=3 type=4713}} } } }",
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(
            pack.naval_transfer_inputs[&((2u64 << 32) | 4713)],
            NavalTransfer {
                source: Some(42),
                combat: vec![(7u64 << 32) | 4713, 0, (7u64 << 32) | 4713],
            }
        );
        assert_eq!(
            pack.naval_transfer_inputs[&((3u64 << 32) | 4713)],
            NavalTransfer::default()
        );
        for body in [
            "province=1 province=2",
            "combat={x={id=1 type=4713}}",
            "combat={{id=1}}",
        ] {
            let doc = content_reader_parser::parser::parse(format!("countries={{X={{strategic_navy={{naval_transport={{id={{id=2 type=4713}} {body}}}}}}}}}").as_bytes()).unwrap();
            assert!(parse(&doc.items).is_err());
        }
    }
    #[test]
    fn sent_volunteers_preserve_order_nulls_and_omitted_empty_lists() {
        let doc = content_reader_parser::parser::parse(
            br"countries={ AAA={ volunteers_sent={
            {id=9 type=4} {id=0 type=0} {id=2 type=7} {id=9 type=4}
        }} BBB={} }",
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(
            pack.sent_volunteers[b"AAA".as_slice()],
            [(9 << 32) | 4, 0, (2 << 32) | 7, (9 << 32) | 4]
        );
        assert!(pack.sent_volunteers[b"BBB".as_slice()].is_empty());
        for body in [
            "volunteers_sent={ id={id=1 type=2} }",
            "volunteers_sent={ {id=1} }",
        ] {
            let doc =
                content_reader_parser::parser::parse(format!("countries={{AAA={{{body}}}}}").as_bytes())
                    .unwrap();
            assert!(parse(&doc.items).is_err());
        }
    }
}

#[cfg(test)]
mod manpower_tests {
    use super::*;
    #[test]
    fn manpower_expeditionary_save_refs_keep_nulls_duplicates_and_native_order() {
        let doc = content_reader_parser::parser::parse(
            b"countries={MOD={expeditionaries_sent={{id=0 type=0}{id=7 type=51}{id=7 type=51}}}}",
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(
            pack.sent_expeditionaries[b"MOD".as_slice()],
            vec![0, (7_u64 << 32) | 51, (7_u64 << 32) | 51]
        );
    }
}
