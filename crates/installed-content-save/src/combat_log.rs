//! Saved combat logs. The state writer emits a top-level `combat_log` for
//! each country slot whose manager holds a group log; the manager writes
//! its `tag` and each group log as `log`. A group log writes its `group`
//! reference, then the four loss arrays in reason order as repeated
//! `equipment`, `enemy_equipment`, `equipment_recovered` and further blocks
//! this parser skips. Each month entry writes its pool as `equipment`
//! (omitting zero amounts unless `allow_zero_entries`), its `reason` and
//! `date`.

use std::io;

use super::{
    Item, SavedDate, block, field, integer, invalid, production, required_block, required_bytes,
    typed_handle,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// Every saved `combat_log`, in save order (country slot order).
    pub countries: Vec<CountryLog>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLog {
    /// The manager's country tag string, as the save writes it.
    pub tag: Vec<u8>,
    pub groups: Vec<GroupLog>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupLog {
    /// The orders group's full typed reference (low u32 type, high u32 id).
    pub group: u64,
    /// The four reason arrays' entries, in save order (reason 0 first).
    pub losses: Vec<Month>,
    /// The recovered array's entries.
    pub recovered: Vec<Month>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Month {
    /// The stored reason; the writer always emits it.
    pub reason: i32,
    /// The raw game hour of the entry's first write.
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
    Ok(pack)
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
    for item in items {
        match item.key.as_deref() {
            Some(b"equipment") => losses.push(parse_month(block(&item.value)?)?),
            Some(b"equipment_recovered") => recovered.push(parse_month(block(&item.value)?)?),
            _ => {}
        }
    }
    Ok(GroupLog {
        group,
        losses,
        recovered,
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
    use installed_content::parser;

    fn parse_text(text: &[u8]) -> io::Result<Pack> {
        parse(&parser::parse(text)?.items)
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
    }

    /// `BATTLE_PLAN_COMBAT_LOG_SAVE=<text save> cargo test -p installed-content-save --lib
    /// real_save_combat_logs -- --ignored --nocapture` parses a whole save and
    /// summarises its combat logs.
    #[test]
    #[ignore]
    fn real_save_combat_logs() {
        let path = std::env::var("BATTLE_PLAN_COMBAT_LOG_SAVE").or_else(|_| std::env::var("HOI4_COMBAT_LOG_SAVE")).expect("BATTLE_PLAN_COMBAT_LOG_SAVE");
        let save = crate::parse(&std::fs::read(path).unwrap()).unwrap();
        for country in &save.combat_log.countries {
            let months: usize = country
                .groups
                .iter()
                .map(|group| group.losses.len() + group.recovered.len())
                .sum();
            let amounts: usize = country
                .groups
                .iter()
                .flat_map(|group| group.losses.iter().chain(&group.recovered))
                .map(|month| month.pool.equipment.len())
                .sum();
            println!(
                "{}: {} group logs, {months} month entries, {amounts} variant amounts",
                String::from_utf8_lossy(&country.tag),
                country.groups.len()
            );
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
