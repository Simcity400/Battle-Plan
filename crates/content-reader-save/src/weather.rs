
use super::{Decimal, Item, Value, block, boolean, decimal, field, integer, invalid, scalar};
use std::io;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub present: bool,
    pub provinces: Option<Vec<Province>>,
    pub regions: Option<Vec<Region>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    pub slot: u32,
    pub province: Option<u32>,
    pub snow: Option<Decimal>,
    pub temperature: Option<Decimal>,
    pub mud: Option<bool>,
    pub custom_modifiers: Option<Vec<(String, i64)>>,
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
    pub active_modifiers: Option<Vec<i64>>,
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
                        temperature: decimal(items, b"temperature")?,
                        mud: flag(items, b"mud")?,
                        custom_modifiers: field(items, b"custom_modifiers")?
                            .map(custom_modifiers)
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
                        region: id(items, b"region")?,
                        rain_light: flag(items, b"rain_light")?,
                        rain_heavy: flag(items, b"rain_heavy")?,
                        snow: flag(items, b"snow")?,
                        blizzard: flag(items, b"blizzard")?,
                        sandstorm: flag(items, b"sandstorm")?,
                        arctic_water: flag(items, b"arctic_water")?,
                        active_modifiers: field(items, b"active_modifiers")?
                            .map(integers)
                            .transpose()?,
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

fn custom_modifiers(value: &Value) -> io::Result<Vec<(String, i64)>> {
    block(value)?
        .iter()
        .map(|item| {
            let name = item
                .key
                .as_deref()
                .ok_or_else(|| invalid("weather custom modifier has no name"))?;
            let name = String::from_utf8(name.to_vec())
                .map_err(|_| invalid("weather custom modifier name is not UTF-8"))?;
            Ok((name, integer(scalar(&item.value)?)?))
        })
        .collect()
}

fn integers(value: &Value) -> io::Result<Vec<i64>> {
    block(value)?
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("weather active modifier list holds a keyed entry"));
            }
            integer(scalar(&item.value)?)
        })
        .collect()
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
mod tests;
