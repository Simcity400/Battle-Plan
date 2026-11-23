
use std::io;

use content_reader_parser::parser::{Item, Operator, Value};

use super::{block, integer, invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryNavalLaunchOccurrence {
    pub ordinal: usize,
    pub tag: Vec<u8>,
    pub naval_bases: Option<Vec<NavalBaseOccurrence>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalBaseOccurrence {
    pub province: i32,
    pub countries: Vec<Vec<u8>>,
}

impl From<i32> for NavalBaseOccurrence {
    fn from(province: i32) -> Self {
        Self {
            province,
            countries: Vec::new(),
        }
    }
}

fn singleton<'a>(items: &'a [Item], key: &[u8]) -> io::Result<Option<&'a Value>> {
    let mut found = None;
    for item in items.iter().filter(|item| item.key.as_deref() == Some(key)) {
        if item.key_quoted || item.operator != Operator::Equals {
            return Err(invalid(
                "naval launch singleton is not an unquoted save assignment",
            ));
        }
        if found.replace(&item.value).is_some() {
            return Err(invalid(format!(
                "duplicate naval launch save field {}",
                String::from_utf8_lossy(key)
            )));
        }
    }
    Ok(found)
}

pub fn parse_country_occurrences(
    countries: &[Item],
) -> io::Result<Vec<CountryNavalLaunchOccurrence>> {
    countries
        .iter()
        .enumerate()
        .map(|(ordinal, country)| {
            if country.operator != Operator::Equals {
                return Err(invalid("country emission has a non-equality operator"));
            }
            let tag = country
                .key
                .as_deref()
                .filter(|tag| !tag.is_empty())
                .ok_or_else(|| invalid("country emission has no tag"))?;
            let fields = block(&country.value)?;
            let naval_bases = singleton(fields, b"strategic_navy")?
                .map(|navy| {
                    let navy = block(navy)?;
                    let Some(bases) = singleton(navy, b"naval_base")? else {
                        return Ok(Vec::new());
                    };
                    block(bases)?
                        .iter()
                        .map(|base| {
                            if base.key_quoted || base.operator != Operator::Equals {
                                return Err(invalid(
                                    "naval base key is not an unquoted signed integer assignment",
                                ));
                            }
                            let key = base
                                .key
                                .as_deref()
                                .ok_or_else(|| invalid("naval base has no signed key"))?;
                            let province = integer::<i32>(key)?;
                            let countries = block(&base.value)?
                                .iter()
                                .filter(|field| {
                                    field.key.as_deref() == Some(b"disabled_for_repairs")
                                })
                                .map(|field| {
                                    if field.key_quoted || field.operator != Operator::Equals {
                                        return Err(invalid(
                                            "naval base country is not an unquoted save assignment",
                                        ));
                                    }
                                    Ok(super::scalar(&field.value)?.to_vec())
                                })
                                .collect::<io::Result<Vec<_>>>()?;
                            Ok(NavalBaseOccurrence {
                                province,
                                countries,
                            })
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .transpose()?;
            Ok(CountryNavalLaunchOccurrence {
                ordinal,
                tag: tag.to_vec(),
                naval_bases,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(text: &[u8]) -> io::Result<Vec<CountryNavalLaunchOccurrence>> {
        parse_country_occurrences(&content_reader_parser::parser::parse(text)?.items)
    }

    #[test]
    fn ordered_signed_words_and_country_occurrences_keep_repeats() {
        let mut text =
            b"\"\xffQ\"={strategic_navy={naval_base={8={} -2147483648={} 8={} 0={} -1={}}}} "
                .to_vec();
        text.extend_from_slice(b"OTHER={} \"\xffQ\"={strategic_navy={naval_base={-1={} 8={}}}}");
        let countries = project(&text).unwrap();
        assert_eq!(countries.len(), 3);
        assert_eq!(
            countries.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(countries[0].tag, countries[2].tag);
        assert_eq!(countries[0].tag, b"\xffQ");
        assert_eq!(
            countries[0].naval_bases,
            Some(
                vec![8, i32::MIN, 8, 0, -1]
                    .into_iter()
                    .map(crate::naval_launch::NavalBaseOccurrence::from)
                    .collect::<Vec<_>>()
            )
        );
        assert_eq!(countries[1].naval_bases, None);
        assert_eq!(
            countries[2].naval_bases,
            Some(
                vec![-1, 8]
                    .into_iter()
                    .map(crate::naval_launch::NavalBaseOccurrence::from)
                    .collect::<Vec<_>>()
            )
        );
    }

    #[test]
    fn ordered_projection_does_not_relax_full_save_duplicate_refusal() {
        let text = b"HOI4txt\ndate=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13 states={} provinces={} countries={A={strategic_navy={}} A={strategic_navy={}}}";
        let document = content_reader_parser::parser::parse(text).unwrap();
        let countries = super::super::required_block(&document.items, b"countries").unwrap();
        assert_eq!(parse_country_occurrences(countries).unwrap().len(), 2);
        assert!(
            super::super::parse(text)
                .unwrap_err()
                .to_string()
                .contains("duplicate country tag")
        );
    }

    #[test]
    fn empty_list_is_known_only_inside_present_navy() {
        let countries =
            project(b"A={} B={strategic_navy={}} C={strategic_navy={naval_base={}}}").unwrap();
        assert_eq!(countries[0].naval_bases, None);
        assert_eq!(countries[1].naval_bases, Some(vec![]));
        assert_eq!(countries[2].naval_bases, Some(vec![]));
    }

    #[test]
    fn naval_base_country_strings_preserve_order_repeats_empty_and_each_occurrence() {
        let countries = project(b"A={strategic_navy={naval_base={7={disabled_for_repairs=\"B\" disabled_for_repairs=\"\" disabled_for_repairs=\"B\"} 7={disabled_for_repairs=\"A\"}}}} B={strategic_navy={naval_base={7={disabled_for_repairs=\"C\"}}}}").unwrap();
        let a = countries[0].naval_bases.as_ref().unwrap();
        assert_eq!(a[0].countries, [b"B".to_vec(), Vec::new(), b"B".to_vec()]);
        assert_eq!(a[1].countries, [b"A".to_vec()]);
        assert_eq!(
            countries[1].naval_bases.as_ref().unwrap()[0].countries,
            [b"C".to_vec()]
        );
        for field in [
            "disabled_for_repairs={}",
            "disabled_for_repairs>yes",
            "\"disabled_for_repairs\"=yes",
        ] {
            assert!(
                project(
                    format!("A={{strategic_navy={{naval_base={{7={{{field}}}}}}}}}").as_bytes()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn malformed_keys_payloads_and_duplicate_singletons_refuse() {
        for text in [
            "A={strategic_navy=yes}",
            "A={strategic_navy=rgb {}}",
            "A={strategic_navy={} strategic_navy={}}",
            "A={strategic_navy={naval_base={} naval_base={}}}",
            "A={strategic_navy={naval_base=yes}}",
            "A={strategic_navy={naval_base=rgb {}}}",
            "A={strategic_navy={naval_base={bad={}}}}",
            "A={strategic_navy={naval_base={2147483648={}}}}",
            "A={strategic_navy={naval_base={-2147483649={}}}}",
            "A={strategic_navy={naval_base={\"1\"={}}}}",
            "A={strategic_navy={naval_base={1=yes}}}",
            "A={strategic_navy={naval_base={1=rgb {}}}}",
            "A={strategic_navy={naval_base={1>{}}}}",
            "A=yes",
            "A=rgb {}",
            "{}",
        ] {
            assert!(project(text.as_bytes()).is_err(), "{text}");
        }
    }
}
