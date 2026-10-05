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
    /// 140F0DBB0 writes temperature unconditionally, including zero.
    pub temperature: Option<Decimal>,
    /// Raw +0x130; the Boolean serializer retains zero/nonzero truth only.
    pub mud: Option<bool>,
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
                        temperature: decimal(items, b"temperature")?,
                        mud: flag(items, b"mud")?,
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
mod tests;
