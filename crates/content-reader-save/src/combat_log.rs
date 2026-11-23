
use std::io;

use super::{
    Item, SavedDate, block, field, integer, invalid, production, required_block, required_bytes,
    typed_handle,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: Vec<CountryLog>,
    pub battles: Vec<Battle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Battle {
    pub id: i32,
    pub ref_count: i32,
    pub date: i32,
    pub province: Option<u32>,
    pub defensive_victory: Option<bool>,
    pub snow: Option<bool>,
    pub overrun: Option<bool>,
    pub player_is_attacker: bool,
    pub attacker: BattleSide,
    pub defender: BattleSide,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleSide {
    pub manpower: Option<i32>,
    pub air_factor: Option<super::Decimal>,
    pub equipment: Option<production::Pool>,
    pub captured_by_enemy: Option<production::Pool>,
    pub recovered: Option<production::Pool>,
    pub modifiers: Vec<(i32, i32)>,
    pub tags: Vec<Vec<u8>>,
    pub leader: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLog {
    pub tag: Vec<u8>,
    pub groups: Vec<GroupLog>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupLog {
    pub group: u64,
    pub losses: Vec<Month>,
    pub recovered: Vec<Month>,
    pub enemy: Vec<Month>,
    pub manpower: Vec<ManpowerMonth>,
    pub templates: Vec<TemplateMonth>,
    pub battles: Vec<BattleReference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManpowerMonth {
    pub reason: i32,
    pub date: i32,
    pub losses: [i32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateMonth {
    pub template: u64,
    pub date: i32,
    pub wins: i32,
    pub total: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BattleReference {
    pub id: i32,
    pub attacker: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Month {
    pub reason: i32,
    pub date: i32,
    pub pool: production::Pool,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let mut pack = Pack::default();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"combat_log"))
    {
        pack.countries.push(parse_country(block(&item.value)?)?);
    }
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"combat_data_entry"))
    {
        let values = block(&item.value)?;
        let data = required_block(values, b"combat_data")?;
        pack.battles.push(Battle {
            id: integer(&required_bytes(values, b"id")?)?,
            ref_count: integer(&required_bytes(values, b"ref_count")?)?,
            date: SavedDate::parse(&required_bytes(data, b"date")?)?.engine_hour()?,
            province: super::bytes(data, b"province")?
                .map(|v| integer(&v))
                .transpose()?,
            defensive_victory: field(data, b"defensive_victory")?
                .map(super::boolean)
                .transpose()?,
            snow: field(data, b"snow")?.map(super::boolean).transpose()?,
            overrun: field(data, b"overrun")?.map(super::boolean).transpose()?,
            player_is_attacker: super::boolean(
                field(data, b"player_is_attacker")?
                    .ok_or_else(|| invalid("combat data lacks player side"))?,
            )?,
            attacker: parse_battle_side(required_block(data, b"attacker")?)?,
            defender: parse_battle_side(required_block(data, b"defender")?)?,
        });
    }
    Ok(pack)
}

pub(crate) fn parse_battle_side(items: &[Item]) -> io::Result<BattleSide> {
    let mut modifiers = Vec::new();
    if let Some(values) = field(items, b"modifier_hours")? {
        for entry in block(values)? {
            if entry.key.is_some() {
                return Err(invalid("combat modifier hours has a named entry"));
            }
            let fields = block(&entry.value)?;
            modifiers.push((
                integer(&required_bytes(fields, b"modifier")?)?,
                integer(&required_bytes(fields, b"time")?)?,
            ));
        }
    }
    let tags = field(items, b"tags")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("combat data country tag is named"));
                    }
                    Ok(super::scalar(&item.value)?.to_vec())
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(BattleSide {
        manpower: super::bytes(items, b"manpower_lost")?
            .map(|v| integer(&v))
            .transpose()?,
        air_factor: super::decimal(items, b"manpower_lost_air_factor")?,
        equipment: field(items, b"equipment_lost")?
            .map(|v| production::parse_pool(block(v)?))
            .transpose()?,
        captured_by_enemy: field(items, b"equipment_captured_by_enemy")?
            .map(|v| production::parse_pool(block(v)?))
            .transpose()?,
        recovered: field(items, b"equipment_recovered")?
            .map(|v| production::parse_pool(block(v)?))
            .transpose()?,
        modifiers,
        tags,
        leader: field(items, b"leader")?.map(typed_handle).transpose()?,
    })
}

fn parse_country(items: &[Item]) -> io::Result<CountryLog> {
    let tag = required_bytes(items, b"tag")?;
    let mut groups = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"log"))
    {
        groups.push(parse_group(block(&item.value)?)?);
    }
    Ok(CountryLog { tag, groups })
}

fn parse_group(items: &[Item]) -> io::Result<GroupLog> {
    let group = typed_handle(
        field(items, b"group")?.ok_or_else(|| invalid("saved combat log lacks its group"))?,
    )?;
    let mut losses = Vec::new();
    let mut recovered = Vec::new();
    let mut enemy = Vec::new();
    let mut manpower = Vec::new();
    let mut templates = Vec::new();
    let mut battles = Vec::new();
    for item in items {
        match item.key.as_deref() {
            Some(b"equipment") => losses.push(parse_month(block(&item.value)?)?),
            Some(b"equipment_recovered") => recovered.push(parse_month(block(&item.value)?)?),
            Some(b"enemy_equipment") => enemy.push(parse_month(block(&item.value)?)?),
            Some(b"manpower") => {
                let values = block(&item.value)?;
                let losses = required_block(values, b"mp_losses")?;
                let losses: Vec<i32> = losses
                    .iter()
                    .map(|value| {
                        if value.key.is_some() {
                            return Err(invalid("combat log manpower has a named counter"));
                        }
                        integer(super::scalar(&value.value)?)
                    })
                    .collect::<io::Result<_>>()?;
                manpower.push(ManpowerMonth {
                    reason: integer(&required_bytes(values, b"reason")?)?,
                    date: SavedDate::parse(&required_bytes(values, b"date")?)?.engine_hour()?,
                    losses: losses
                        .try_into()
                        .map_err(|_| invalid("combat log manpower needs three counters"))?,
                });
            }
            Some(b"division_template") => {
                let values = block(&item.value)?;
                templates.push(TemplateMonth {
                    template: typed_handle(
                        field(values, b"division")?
                            .ok_or_else(|| invalid("combat log template reference missing"))?,
                    )?,
                    date: SavedDate::parse(&required_bytes(values, b"date")?)?.engine_hour()?,
                    wins: integer(&required_bytes(values, b"win")?)?,
                    total: integer(&required_bytes(values, b"total")?)?,
                });
            }
            Some(b"combat_data_index") => {
                let values = block(&item.value)?;
                battles.push(BattleReference {
                    id: integer(&required_bytes(values, b"id")?)?,
                    attacker: super::boolean(
                        field(values, b"attacker")?
                            .ok_or_else(|| invalid("combat log battle side missing"))?,
                    )?,
                });
            }
            _ => {}
        }
    }
    Ok(GroupLog {
        group,
        losses,
        recovered,
        enemy,
        manpower,
        templates,
        battles,
    })
}

fn parse_month(items: &[Item]) -> io::Result<Month> {
    Ok(Month {
        reason: integer(&required_bytes(items, b"reason")?)?,
        date: SavedDate::parse(&required_bytes(items, b"date")?)?.engine_hour()?,
        pool: production::parse_pool(required_block(items, b"equipment")?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_parser::parser;

    fn parse_text(text: &[u8]) -> io::Result<Pack> {
        parse(&parser::parse(text)?.items)
    }

    #[test]
    fn active_observer_keeps_all_saved_descendants_and_repeated_template_handles() {
        let counters = std::iter::repeat_n("0", 30).collect::<Vec<_>>().join(" ");
        let text = format!(
            "combat_side_data={{manpower_lost=3 tags={{AAA BBB}}}} modifier_hours={{{counters}}} total_damage=2.5 progress=0.25 snow=yes win=yes group={{group={{id=7 type=53}} division_template={{{{id=4 type=54}} {{id=4 type=54}}}} enemy_dmg_units={{{{id=9 type=51}}}} enemy_dmg_str={{1.5}} damage_dealer=AAA damage_taker=BBB damaged_equipment={{allow_zero_entries=no}}}} leader_hours={{leader={{id=8 type=64}} time=-1}} damage={{from=AAA receiver=BBB to=CCC value=0.125}}"
        );
        let tree = parser::parse(text.as_bytes()).unwrap();
        let observer = parse_observer(&tree.items).unwrap();
        assert_eq!(observer.victory, Some(true));
        assert_eq!(observer.counters.len(), 30);
        assert_eq!(observer.side.manpower, Some(3));
        assert_eq!(observer.activities[0].templates.len(), 2);
        assert_eq!(
            observer.activities[0].templates[0],
            observer.activities[0].templates[1]
        );
        assert!(
            observer.activities[0]
                .equipment
                .as_ref()
                .unwrap()
                .equipment
                .is_empty()
        );
        assert_eq!(observer.leaders[0].1, -1);
        assert_eq!(observer.damage[0].value.raw, 12_500);
        assert_eq!(observer.progress.unwrap().raw, 25_000);
    }

    #[test]
    fn keeps_countries_groups_reasons_and_pools_in_save_order() {
        let pack = parse_text(
            br#"combat_log={ tag="GER"
                log={ group={ id=12793 type=53 }
                    equipment={ equipment={ equipment={ id={ id=1326 type=70 } amount=3 }
                        equipment={ id={ id=1327 type=70 } amount=0.5 } allow_zero_entries=no }
                        reason=0 date="1943.10.30.11" }
                    equipment={ equipment={ equipment={ id={ id=1326 type=70 } amount=26 }
                        allow_zero_entries=no } reason=1 date="1943.10.29.8" }
                    enemy_equipment={ equipment={ equipment={ id={ id=5300 type=70 } amount=2 }
                        allow_zero_entries=no } reason=0 date="1943.10.30.11" }
                    equipment_recovered={ equipment={ equipment={ id={ id=1846 type=70 } amount=1 }
                        allow_zero_entries=no } reason=0 date="1943.10.30.21" }
                    manpower={ reason=0 date="1943.10.30.11" mp_losses={ 44 0 0 } }
                }
            }
            combat_log={ tag="D01" }"#,
        )
        .unwrap();
        assert_eq!(pack.countries.len(), 2);
        let ger = &pack.countries[0];
        assert_eq!(ger.tag, b"GER");
        let group = &ger.groups[0];
        assert_eq!(group.group, (12793_u64 << 32) | 53);
        assert_eq!(
            group
                .losses
                .iter()
                .map(|month| month.reason)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(group.losses[0].pool.equipment[1].variant, (1327 << 32) | 70);
        assert_eq!(group.losses[0].pool.equipment[1].amount.raw, 50_000);
        assert_eq!(group.losses[1].pool.equipment[0].amount.raw, 2_600_000);
        assert_eq!(group.losses[0].date, 60_827_938);
        assert_eq!(group.recovered.len(), 1);
        assert_eq!(
            group.recovered[0].pool.equipment[0].variant,
            (1846 << 32) | 70
        );
        assert!(pack.countries[1].groups.is_empty());
        assert_eq!(group.enemy.len(), 1);
        assert_eq!(group.enemy[0].pool.equipment[0].amount.raw, 200_000);
        assert_eq!(group.manpower[0].losses, [44, 0, 0]);
    }

    #[test]
    fn keeps_empty_months_signed_counters_templates_and_repeated_battle_references() {
        let pack = parse_text(
            br#"combat_log={tag=AAA log={group={id=4 type=53}
            enemy_equipment={equipment={allow_zero_entries=yes} reason=0 date="1940.2.1.1"}
            equipment={equipment={allow_zero_entries=no} reason=2 date="1940.2.2.1"}
            manpower={reason=0 date="1940.2.1.1" mp_losses={-2147483648 2147483647 -3}}
            division_template={division={id=7 type=52} date="1940.2.1.1" win=-1 total=3}
            division_template={division={id=7 type=52} date="1940.2.2.1" win=0 total=2}
            combat_data_index={id=0 attacker=yes}
            combat_data_index={id=0 attacker=no}
        }}"#,
        )
        .unwrap();
        let log = &pack.countries[0].groups[0];
        assert!(log.enemy[0].pool.equipment.is_empty());
        assert_eq!(log.enemy[0].pool.allow_zero_entries, Some(true));
        assert!(log.losses[0].pool.equipment.is_empty());
        assert_eq!(log.manpower[0].losses, [i32::MIN, i32::MAX, -3]);
        assert_eq!(log.templates.len(), 2);
        assert_eq!(log.templates[0].template, 7 << 32 | 52);
        assert_eq!(log.templates[0].wins, -1);
        assert_eq!(
            log.battles,
            [
                BattleReference {
                    id: 0,
                    attacker: true
                },
                BattleReference {
                    id: 0,
                    attacker: false
                }
            ]
        );
    }

    #[test]
    fn keeps_battle_pool_slots_and_serializer_presence() {
        let pack = parse_text(br#"combat_data_entry={id=0 ref_count=3 combat_data={
            attacker={manpower_lost=5 manpower_lost_air_factor=0.125
                equipment_lost={allow_zero_entries=no}
                equipment_captured_by_enemy={allow_zero_entries=yes}
                equipment_recovered={equipment={id={id=9 type=70} amount=-0.25} allow_zero_entries=no}
                modifier_hours={{modifier=11 time=-2} {modifier=11 time=4}}
                leader={id=0 type=4713} tags={AAA AAA BBB}}
            defender={} date="1940.2.1.1" province=3 overrun=yes snow=yes
            defensive_victory=yes player_is_attacker=no
        }}"#).unwrap();
        let battle = &pack.battles[0];
        assert_eq!((battle.id, battle.ref_count), (0, 3));
        assert_eq!(battle.attacker.modifiers, [(11, -2), (11, 4)]);
        assert_eq!(
            battle.attacker.tags,
            [b"AAA".to_vec(), b"AAA".to_vec(), b"BBB".to_vec()]
        );
        assert_eq!(battle.attacker.air_factor.as_ref().unwrap().raw, 12_500);
        assert!(
            battle
                .attacker
                .equipment
                .as_ref()
                .unwrap()
                .equipment
                .is_empty()
        );
        assert_eq!(
            battle.attacker.recovered.as_ref().unwrap().equipment[0]
                .amount
                .raw,
            -25_000
        );
        assert_eq!(battle.attacker.leader, Some(4713));
        assert_eq!(battle.defender.manpower, None);
        assert_eq!(battle.defender.equipment, None);
        assert_eq!(battle.overrun, Some(true));
    }

    #[test]
    fn refuses_incomplete_new_log_records() {
        for payload in [
            "manpower={reason=0 date=\"1940.1.1.1\" mp_losses={1 2}}",
            "division_template={division={id=1 type=52} date=\"1940.1.1.1\" win=1}",
            "combat_data_index={id=1}",
            "enemy_equipment={reason=0 date=\"1940.1.1.1\"}",
        ] {
            let text = format!("combat_log={{tag=AAA log={{group={{id=1 type=53}} {payload}}}}}");
            assert!(parse_text(text.as_bytes()).is_err(), "{payload}");
        }
    }

    #[test]
    fn refuses_incomplete_entries() {
        assert!(parse_text(br#"combat_log={ tag="GER" log={ equipment={} } }"#).is_err());
        assert!(
            parse_text(
                br#"combat_log={ tag="GER" log={ group={ id=1 type=53 }
                equipment={ reason=0 date="1943.1.1.1" } } }"#
            )
            .is_err()
        );
        assert!(parse_text(br#"combat_log={ log={} }"#).is_err());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observer {
    pub side: BattleSide,
    pub activities: Vec<Activity>,
    pub leaders: Vec<(u64, i32)>,
    pub damage: Vec<ObserverDamage>,
    pub total_damage: Option<super::Decimal>,
    pub progress: Option<super::Decimal>,
    pub counters: Vec<i32>,
    pub snow: Option<bool>,
    pub victory: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDamage {
    pub from: Vec<u8>,
    pub receiver: Vec<u8>,
    pub to: Vec<u8>,
    pub value: super::Decimal,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub group: u64,
    pub templates: Vec<u64>,
    pub units: Vec<u64>,
    pub strength: Vec<super::Decimal>,
    pub equipment: Option<production::Pool>,
    pub dealer: Option<Vec<u8>>,
    pub taker: Option<Vec<u8>>,
}
fn anonymous<T>(
    items: &[Item],
    key: &[u8],
    mut parse: impl FnMut(&super::Value) -> io::Result<T>,
) -> io::Result<Vec<T>> {
    field(items, key)?
        .map(|value| {
            block(value)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("combat observer array entry is named"));
                    }
                    parse(&item.value)
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}
pub(crate) fn parse_observer(items: &[Item]) -> io::Result<Observer> {
    let mut activities = Vec::new();
    let mut leaders = Vec::new();
    let mut damage = Vec::new();
    for item in items {
        match item.key.as_deref() {
            Some(b"group") => {
                let fields = block(&item.value)?;
                let activity = Activity {
                    group: typed_handle(
                        field(fields, b"group")?.ok_or_else(|| invalid("activity lacks group"))?,
                    )?,
                    templates: anonymous(fields, b"division_template", typed_handle)?,
                    units: anonymous(fields, b"enemy_dmg_units", typed_handle)?,
                    strength: anonymous(fields, b"enemy_dmg_str", |v| {
                        super::Decimal::parse(super::scalar(v)?)
                    })?,
                    equipment: field(fields, b"damaged_equipment")?
                        .map(|v| production::parse_pool(block(v)?))
                        .transpose()?,
                    dealer: super::bytes(fields, b"damage_dealer")?,
                    taker: super::bytes(fields, b"damage_taker")?,
                };
                if activity.units.len() != activity.strength.len() {
                    return Err(invalid("activity damage arrays differ"));
                }
                activities.push(activity);
            }
            Some(b"leader_hours") => {
                let fields = block(&item.value)?;
                leaders.push((
                    typed_handle(
                        field(fields, b"leader")?
                            .ok_or_else(|| invalid("observer lacks leader"))?,
                    )?,
                    integer(&required_bytes(fields, b"time")?)?,
                ));
            }
            Some(b"damage") => {
                let fields = block(&item.value)?;
                damage.push(ObserverDamage {
                    from: required_bytes(fields, b"from")?,
                    receiver: required_bytes(fields, b"receiver")?,
                    to: required_bytes(fields, b"to")?,
                    value: super::Decimal::parse(&required_bytes(fields, b"value")?)?,
                });
            }
            _ => (),
        }
    }
    let counters = anonymous(items, b"modifier_hours", |v| integer(super::scalar(v)?))?;
    if counters.len() != 30 {
        return Err(invalid("observer modifier counter count differs"));
    }
    Ok(Observer {
        side: parse_battle_side(required_block(items, b"combat_side_data")?)?,
        activities,
        leaders,
        damage,
        total_damage: super::decimal(items, b"total_damage")?,
        progress: super::decimal(items, b"progress")?,
        counters,
        snow: field(items, b"snow")?.map(super::boolean).transpose()?,
        victory: field(items, b"win")?.map(super::boolean).transpose()?,
    })
}
