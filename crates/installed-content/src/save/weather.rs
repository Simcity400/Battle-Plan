//! The weather save fields the oracle compares, read with the native
//! F0D8E0/F0DBB0/F0DDC0 serializer rules. Source order and absence stay explicit.

use super::{Decimal, Item, block, boolean, decimal, field, integer, invalid, scalar};
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
    pub snow: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    pub slot: u32,
    pub region: Option<u32>,
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
    let id = |items: &[Item], key: &[u8]| -> io::Result<Option<u32>> {
        field(items, key)?
            .map(|value| integer(scalar(value)?))
            .transpose()
    };
    let flag = |items: &[Item], key: &[u8]| -> io::Result<Option<bool>> {
        field(items, key)?.map(boolean).transpose()
    };
    let provinces = field(weather, b"provinces")?
        .map(|value| {
            block(value)?
                .iter()
                .map(|entry| {
                    let items = block(&entry.value)?;
                    Ok(Province {
                        slot: slot(entry)?,
                        province: id(items, b"province")?,
                        snow: decimal(items, b"snow")?,
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
                        region: id(items, b"region")?,
                        rain_light: flag(items, b"rain_light")?,
                        rain_heavy: flag(items, b"rain_heavy")?,
                        snow: flag(items, b"snow")?,
                        blizzard: flag(items, b"blizzard")?,
                        sandstorm: flag(items, b"sandstorm")?,
                        arctic_water: flag(items, b"arctic_water")?,
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
    fn weather_keeps_zero_slot_repeated_entries_and_exact_snow_words() {
        let saved = weather(
            br#"weather={provinces={
                "0"={province=0 temperature=0}
                "7"={province=8 snow=0.00001 water=unread custom_modifiers={x=y}}
                "0"={temperature=2}
            } regions={
                "0"={region=0 temperature=0 next_weather_change=unread}
                "4"={region=4 rain_light=yes rain_heavy=no snow=yes}
            } seed=unread}"#,
        )
        .unwrap();
        let provinces = saved.provinces.unwrap();
        assert_eq!(
            provinces.iter().map(|row| row.slot).collect::<Vec<_>>(),
            [0, 7, 0]
        );
        assert_eq!(provinces[1].province, Some(8));
        assert_eq!(provinces[1].snow.as_ref().unwrap().raw, 1);
        assert_eq!(provinces[0].snow, None);
        assert_eq!(provinces[2].province, None);
        let regions = saved.regions.unwrap();
        assert_eq!(regions[0].slot, 0);
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
    fn weather_refuses_malformed_or_ambiguous_compared_fields_without_rounding() {
        for text in [
            "weather={} weather={}",
            "weather={provinces={} provinces={}}",
            "weather={provinces={{province=0}}}",
            "weather={provinces={-1={province=0}}}",
            "weather={provinces={0={snow=0.000001}}}",
            "weather={provinces={0={snow=0 snow=1}}}",
            "weather={regions={0={rain_light=1}}}",
        ] {
            assert!(weather(text.as_bytes()).is_err(), "{text}");
        }
    }
}
