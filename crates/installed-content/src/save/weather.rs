//! Literal weather save fields, interpreted by the oracle with the native
//! F0D8E0/F0DBB0/F0DDC0 serializer rules. Source order and absence stay explicit.

use super::{Decimal, GameDate, Item, block, boolean, decimal, field, integer, invalid, scalar};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub present: bool,
    /// Entry order and repeated numeric keys are retained, including slot zero.
    pub provinces: Option<Vec<Province>>,
    pub regions: Option<Vec<Region>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    /// Numeric key enclosing this entry, independently of its province field.
    pub slot: u32,
    pub province: Option<u32>,
    pub temperature: Option<Decimal>,
    pub water: Option<Decimal>,
    pub snow: Option<Decimal>,
    pub mud: Option<bool>,
    /// F0DBB0 writes the full native +0x128 word here, omitting only zero.
    pub temperature_offset: Option<Decimal>,
    /// F0DBB0 emits each +0x148 vector entry once, preserving names and repeats;
    /// omission means the native +0x154 count is zero.
    pub custom_modifiers: Option<Vec<CustomModifier>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomModifier {
    pub name: Vec<u8>,
    pub days: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub slot: u32,
    pub region: Option<u32>,
    pub temperature: Option<Decimal>,
    /// F0DDC0 passes the +0x158 CGameDate CPersistent subobject to 527660;
    /// its receiver-8 word is the captured raw next_change_hour at +0x150.
    pub next_weather_change: Option<GameDate>,
    pub rain_light: Option<bool>,
    pub rain_heavy: Option<bool>,
    pub snow: Option<bool>,
    pub blizzard: Option<bool>,
    pub sandstorm: Option<bool>,
    pub arctic_water: Option<bool>,
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let Some(value) = field(items, b"weather")? else {
        return Ok(Pack::default());
    };
    let weather = block(value)?;
    let provinces = field(weather, b"provinces")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|entry| {
                    let items = block(&entry.value)?;
                    Ok(Province {
                        slot: slot(entry)?,
                        province: field(items, b"province")?
                            .map(|value| integer(scalar(value)?))
                            .transpose()?,
                        temperature: decimal(items, b"temperature")?,
                        water: decimal(items, b"water")?,
                        snow: decimal(items, b"snow")?,
                        mud: field(items, b"mud")?.map(boolean).transpose()?,
                        temperature_offset: decimal(items, b"temperature_offset")?,
                        custom_modifiers: field(items, b"custom_modifiers")?
                            .map(|value| {
                                block(value)?
                                    .iter()
                                    .map(|entry| {
                                        Ok(CustomModifier {
                                            name: entry.key.clone().ok_or_else(|| {
                                                invalid("weather custom modifier has no name")
                                            })?,
                                            days: integer(scalar(&entry.value)?)?,
                                        })
                                    })
                                    .collect::<io::Result<Vec<_>>>()
                            })
                            .transpose()?,
                    })
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?;
    let regions = field(weather, b"regions")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|entry| {
                    let items = block(&entry.value)?;
                    Ok(Region {
                        slot: slot(entry)?,
                        region: field(items, b"region")?
                            .map(|value| integer(scalar(value)?))
                            .transpose()?,
                        temperature: decimal(items, b"temperature")?,
                        next_weather_change: field(items, b"next_weather_change")?
                            .map(|value| GameDate::parse(scalar(value)?))
                            .transpose()?,
                        rain_light: field(items, b"rain_light")?.map(boolean).transpose()?,
                        rain_heavy: field(items, b"rain_heavy")?.map(boolean).transpose()?,
                        snow: field(items, b"snow")?.map(boolean).transpose()?,
                        blizzard: field(items, b"blizzard")?.map(boolean).transpose()?,
                        sandstorm: field(items, b"sandstorm")?.map(boolean).transpose()?,
                        arctic_water: field(items, b"arctic_water")?.map(boolean).transpose()?,
                    })
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?;
    Ok(Pack {
        present: true,
        provinces,
        regions,
    })
}

fn slot(entry: &Item) -> io::Result<u32> {
    integer(
        entry
            .key
            .as_deref()
            .ok_or_else(|| invalid("weather save entry has no numeric slot key"))?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weather(text: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(text)?.items)
    }

    #[test]
    fn weather_keeps_zero_slot_repeated_entries_and_exact_decimal_words() {
        let saved = weather(
            br#"weather={provinces={
                "0"={province=0 temperature=0}
                "7"={province=8 temperature=-90071992547.40993 water=1.00001
                     snow=0.00001 mud=yes temperature_offset=-0.64582
                     custom_modifiers={mud_modifier=-1 mud_modifier=2147483647}}
                "0"={temperature=2}
            } regions={
                "0"={region=0 temperature=0 next_weather_change="2.1.1.1"}
                "4"={region=4 rain_light=yes rain_heavy=no snow=yes}
            }}"#,
        )
        .unwrap();
        let provinces = saved.provinces.unwrap();
        assert_eq!(
            provinces.iter().map(|row| row.slot).collect::<Vec<_>>(),
            [0, 7, 0]
        );
        assert_eq!(provinces[1].province, Some(8));
        assert_eq!(
            provinces[1].temperature.as_ref().unwrap().raw,
            -9_007_199_254_740_993
        );
        assert_eq!(provinces[1].water.as_ref().unwrap().raw, 100_001);
        assert_eq!(provinces[1].snow.as_ref().unwrap().raw, 1);
        assert_eq!(
            provinces[1].temperature_offset.as_ref().unwrap().raw,
            -64_582
        );
        assert_eq!(provinces[1].mud, Some(true));
        assert_eq!(
            provinces[1].custom_modifiers.as_ref().unwrap(),
            &[
                CustomModifier {
                    name: b"mud_modifier".to_vec(),
                    days: -1
                },
                CustomModifier {
                    name: b"mud_modifier".to_vec(),
                    days: i32::MAX
                },
            ]
        );
        assert_eq!(provinces[0].water, None);
        assert_eq!(provinces[0].snow, None);
        assert_eq!(provinces[0].mud, None);
        assert_eq!(provinces[2].province, None);
        let regions = saved.regions.unwrap();
        assert_eq!(regions[0].slot, 0);
        assert_eq!(regions[0].next_weather_change.unwrap().year, 2);
        assert_eq!(regions[1].rain_light, Some(true));
        assert_eq!(regions[1].rain_heavy, Some(false));
        assert_eq!(regions[1].snow, Some(true));
        assert_eq!(regions[1].blizzard, None);
    }

    #[test]
    fn weather_distinguishes_missing_root_missing_rosters_and_empty_rosters() {
        assert_eq!(weather(b"").unwrap(), Pack::default());
        let present = weather(b"weather={}").unwrap();
        assert!(present.present);
        assert_eq!(present.provinces, None);
        assert_eq!(present.regions, None);
        let empty = weather(b"weather={provinces={} regions={}}").unwrap();
        assert_eq!(empty.provinces, Some(Vec::new()));
        assert_eq!(empty.regions, Some(Vec::new()));
    }

    #[test]
    fn weather_refuses_malformed_or_ambiguous_fields_without_rounding() {
        for text in [
            "weather={} weather={}",
            "weather={provinces={} provinces={}}",
            "weather={provinces={{province=0}}}",
            "weather={provinces={-1={province=0}}}",
            "weather={provinces={0={water=1.000001}}}",
            "weather={provinces={0={snow=0 snow=1}}}",
            "weather={provinces={0={mud=1}}}",
            "weather={provinces={0={custom_modifiers={mud=2147483648}}}}",
            "weather={regions={0={rain_light=1}}}",
            "weather={regions={0={next_weather_change=1936.2.30.1}}}",
        ] {
            assert!(weather(text.as_bytes()).is_err(), "{text}");
        }
    }
}
