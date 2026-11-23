use crate::{block, field, invalid, null_permitted_handle, scalar};
use content_reader_parser::parser::Item;
use std::io;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub advisor_slots: Vec<AdvisorSlot>,
    pub extracted: Option<crate::economy::ResourcePool>,
    pub projection_effects: Option<crate::Decimal>,
    pub minimum_upgrades: Vec<Vec<u8>>,
    pub doctrine_folders: Vec<Vec<u8>>,
    pub sharing_countries: Vec<Vec<u8>>,
    pub sharing_bonuses: Vec<crate::Decimal>,
    pub variables: Option<crate::intel::Variables>,
    pub research: Option<bool>,
    pub manpower_unlocked: Option<bool>,
    pub rules: Vec<Vec<u8>>,
    pub programmes: Vec<u64>,
    pub manpower: Vec<crate::construction::Manpower>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifier {
    pub name: Vec<u8>,
    pub entries: Vec<(Vec<u8>, crate::Decimal)>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdvisorSlot {
    pub owner: Option<Vec<u8>>,
    pub advisor: Option<Vec<u8>>,
    pub date: Option<i32>,
    pub modifier: Modifier,
    pub spymaster: Modifier,
}
fn modifier(items: &[Item]) -> io::Result<Modifier> {
    let mut value = Modifier::default();
    for item in items {
        let name = item
            .key
            .as_deref()
            .ok_or_else(|| invalid("unkeyed faction slot modifier"))?;
        if name == b"name" {
            value.name = scalar(&item.value)?.to_vec();
        } else {
            value
                .entries
                .push((name.to_vec(), crate::Decimal::parse(scalar(&item.value)?)?));
        }
    }
    Ok(value)
}
pub(crate) fn parse_pool(items: &[Item]) -> io::Result<crate::economy::ResourcePool> {
    items
        .iter()
        .map(|item| {
            let key = item
                .key
                .as_ref()
                .ok_or_else(|| invalid("faction resource amount is unkeyed"))?;
            Ok((key.clone(), crate::Decimal::parse(scalar(&item.value)?)?))
        })
        .collect()
}
pub(crate) fn parse(items: &[Item]) -> io::Result<Snapshot> {
    let mut result = Snapshot {
        extracted: field(items, b"extracted")?
            .map(|value| parse_pool(block(value)?))
            .transpose()?,
        projection_effects: field(items, b"power_projection_from_effects")?
            .map(|value| crate::Decimal::parse(scalar(value)?))
            .transpose()?,
        variables: field(items, b"variables")?
            .map(|value| crate::intel::parse_variables(block(value)?))
            .transpose()?,
        research: field(items, b"research")?.map(crate::boolean).transpose()?,
        manpower_unlocked: field(items, b"manpower")?.map(crate::boolean).transpose()?,
        ..Default::default()
    };
    if let Some(status) = field(items, b"rule_status")?
        && let Some(rules) = field(block(status)?, b"rules")?
    {
        for rule in block(rules)? {
            if rule.key.is_some() {
                return Err(invalid("faction rules must be an ordinary token list"));
            }
            result.rules.push(scalar(&rule.value)?.to_vec());
        }
    }
    if let Some(status) = field(items, b"faction_programs")?
        && let Some(programmes) = field(block(status)?, b"faction_programs")?
    {
        for programme in block(programmes)? {
            if programme.key.is_some() {
                return Err(invalid(
                    "faction programme references must be an ordinary list",
                ));
            }
            result
                .programmes
                .push(null_permitted_handle(&programme.value)?);
        }
    }
    if let Some(upgrades) = field(items, b"upgrades")?
        && let Some(sharing) = field(block(upgrades)?, b"tech_sharing_group")?
    {
        let sharing = block(sharing)?;
        if let Some(countries) = field(sharing, b"countries")? {
            result.sharing_countries = block(countries)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("faction sharing list item is keyed"));
                    }
                    Ok(scalar(&item.value)?.to_vec())
                })
                .collect::<io::Result<_>>()?;
        }
        if let Some(bonuses) = field(sharing, b"bonuses")? {
            result.sharing_bonuses = block(bonuses)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("faction sharing list item is keyed"));
                    }
                    crate::Decimal::parse(scalar(&item.value)?)
                })
                .collect::<io::Result<_>>()?;
        }
    }
    if let Some(upgrades) = field(items, b"upgrades")? {
        let upgrades = block(upgrades)?;
        if let Some(slots) = field(upgrades, b"slots")? {
            for slot in block(slots)? {
                if slot.key.is_some() {
                    return Err(invalid("keyed faction advisor slot"));
                }
                let fields = block(&slot.value)?;
                result.advisor_slots.push(AdvisorSlot {
                    owner: crate::bytes(fields, b"owner")?,
                    advisor: crate::bytes(fields, b"advisor")?,
                    date: field(fields, b"spymaster_change_date")?
                        .map(|date| crate::SavedDate::parse(scalar(date)?)?.engine_hour())
                        .transpose()?,
                    modifier: field(fields, b"modifier")?
                        .map(|value| modifier(block(value)?))
                        .transpose()?
                        .unwrap_or_default(),
                    spymaster: field(fields, b"spymaster")?
                        .map(|value| modifier(block(value)?))
                        .transpose()?
                        .unwrap_or_default(),
                });
            }
        }

        if let Some(values) = field(upgrades, b"member_upgrades")? {
            result.minimum_upgrades = block(values)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("faction sharing list item is keyed"));
                    }
                    Ok(scalar(&item.value)?.to_vec())
                })
                .collect::<io::Result<_>>()?;
        }
        if let Some(doctrine) = field(upgrades, b"doctrine")?
            && let Some(values) = field(block(doctrine)?, b"folder")?
        {
            result.doctrine_folders = block(values)?
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("faction sharing list item is keyed"));
                    }
                    Ok(scalar(&item.value)?.to_vec())
                })
                .collect::<io::Result<_>>()?;
        }
    }
    for pool in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"manpower_pool"))
    {
        crate::construction::append_manpower(block(&pool.value)?, &mut result.manpower)?;
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn manpower(source: &str) -> io::Result<Vec<crate::construction::Manpower>> {
        let document = content_reader_parser::parser::parse(source.as_bytes())?;
        Ok(crate::parse_faction(&document.items)?.companions.manpower)
    }
    #[test]
    fn faction_manpower_emitted_empty_wrapper_keeps_the_following_payload() {
        let rows = manpower(
            "id={id=17 type=88} manpower_pool={} \
             manpower_pool={value={tag=MOD value=7} value={tag=OTHER value=3}}",
        )
        .unwrap();
        assert_eq!(
            rows,
            [
                crate::construction::Manpower {
                    source_country: b"MOD".to_vec(),
                    amount: 7,
                },
                crate::construction::Manpower {
                    source_country: b"OTHER".to_vec(),
                    amount: 3,
                },
            ]
        );
    }
    #[test]
    fn faction_manpower_appends_every_wrapper_without_reset_coalescing_or_reordering() {
        for (source, expected) in [
            ("", vec![]),
            ("manpower_pool={} manpower_pool={} manpower_pool={}", vec![]),
            (
                "manpower_pool={value={tag=Z value=7}}",
                vec![(b"Z".as_slice(), 7)],
            ),
            (
                "manpower_pool={value={tag=Z value=7}} manpower_pool={} \
                 manpower_pool={value={tag=A value=2147483647} value={tag=Z value=3}} \
                 manpower_pool={value={tag=Z value=7}} manpower_pool={}",
                vec![
                    (b"Z".as_slice(), 7),
                    (b"A".as_slice(), i32::MAX),
                    (b"Z".as_slice(), 3),
                    (b"Z".as_slice(), 7),
                ],
            ),
        ] {
            let rows = manpower(&format!(
                "id={{id=17 type=88}} {source} \
                 upgrades={{manpower_pool={{value={{tag=NESTED value=999}}}}}}"
            ))
            .unwrap();
            assert_eq!(
                rows.iter()
                    .map(|row| (row.source_country.as_slice(), row.amount))
                    .collect::<Vec<_>>(),
                expected,
                "{source}",
            );
        }
    }
    #[test]
    fn faction_manpower_repeated_wrappers_validate_every_payload_and_keep_other_fields_unique() {
        for malformed in [
            "manpower_pool=scalar",
            "manpower_pool={unexpected={tag=MOD value=4}}",
            "manpower_pool={value={tag=MOD value=0}}",
            "manpower_pool={value={tag=MOD value=-1}}",
            "manpower_pool={value={tag=MOD value=2147483648}}",
            "manpower_pool={value={tag=MOD}}",
            "manpower_pool={value={tag=MOD value=1 value=2}}",
            "manpower_pool={value={tag=MOD tag=OTHER value=1}}",
        ] {
            for (before, after) in [
                (malformed, "manpower_pool={}"),
                ("manpower_pool={}", malformed),
            ] {
                assert!(manpower(&format!("id={{id=17 type=88}} {before} {after}")).is_err());
            }
        }
        let document = content_reader_parser::parser::parse(
            b"research=yes research=no manpower_pool={} manpower_pool={}",
        )
        .unwrap();
        assert_eq!(
            parse(&document.items).unwrap_err().to_string(),
            "duplicate save field research"
        );
    }
    #[test]
    fn advisor_slots_preserve_empty_owners_modifier_sources_signed_values_and_dates() {
        let doc=content_reader_parser::parser::parse(br#"upgrades={slots={{owner=AAA advisor=synthetic spymaster_change_date=1936.1.2.3 modifier={name="Advisor" synthetic_modifier=-0.25} spymaster={name="Master" synthetic_modifier=0.5}} {modifier={name="Empty"} spymaster={name="Empty"}}}}"#).unwrap();
        let value = parse(&doc.items).unwrap();
        assert_eq!(value.advisor_slots.len(), 2);
        let first = &value.advisor_slots[0];
        assert_eq!(first.owner.as_deref(), Some(b"AAA".as_slice()));
        assert_eq!(first.modifier.entries[0].1.raw, -25000);
        assert_eq!(first.spymaster.entries[0].1.raw, 50000);
        assert!(first.date.is_some());
        assert_eq!(value.advisor_slots[1].owner, None);
    }
    #[test]
    fn faction_companion_save_preserves_rule_order_null_programmes_and_repeated_manpower_sources() {
        let doc=content_reader_parser::parser::parse(br#"research=yes manpower=no rule_status={rules={mod_z mod_a mod_z}} faction_programs={faction_programs={{type=8 id=4294967295}{type=0 id=0}}} manpower_pool={value={tag=MOD value=7}value={tag=MOD value=3}}"#).unwrap();
        let value = parse(&doc.items).unwrap();
        assert_eq!(value.research, Some(true));
        assert_eq!(value.manpower_unlocked, Some(false));
        assert_eq!(parse(&[]).unwrap().research, None);
        assert_eq!(
            value.rules,
            [b"mod_z".to_vec(), b"mod_a".to_vec(), b"mod_z".to_vec()]
        );
        assert_eq!(value.programmes, [(u32::MAX as u64) << 32 | 8, 0]);
        assert_eq!(value.manpower.len(), 2);
        assert_eq!(value.manpower[0].amount, 7);
        assert_eq!(value.manpower[1].source_country, b"MOD");
    }
}
