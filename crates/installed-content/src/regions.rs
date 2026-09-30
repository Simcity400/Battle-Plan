//! Strategic regions from `map/strategicregions/*.txt`: identity, membership
//! and naval terrain. Weather periods drive the simulation, which the replay
//! records, so they are not read.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fields::{block, invalid},
    parser::native::{self, Shape},
};

/// One strategic region and the provinces it owns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrategicRegion {
    pub id: u32,
    /// Localisation key, normally `STRATEGICREGION_<id>`.
    pub name_key: String,
    pub provinces: Vec<u32>,
    /// Present when the region declares a naval terrain category.
    pub naval_terrain: Option<String>,
}

/// Parses one `map/strategicregions` file through the game's database loop
/// (140AB1200) and CStrategicRegionTemplate::ReadToken (1415938B0).
pub fn parse_region(input: &[u8]) -> io::Result<StrategicRegion> {
    parse_file(input)?.ok_or_else(|| {
        invalid(
            "strategic region file has 0 strategic_region definitions; exactly one is supported",
        )
    })
}

/// One file as the database loop (140AB1200) reads it: a file that declares
/// no strategic region (mods blank vanilla files to remove one) adds none.
fn parse_file(input: &[u8]) -> io::Result<Option<StrategicRegion>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Region,
        Weather,
        Period,
        Modifiers,
    }
    // 140AB1690 constructs each file's lexer in mode 0 (1424A6370).
    let document = native::read(
        input,
        native::Mode::KeepMark,
        Node::File,
        &mut |node, field| {
            use Node::*;
            match (node, field.keyword().as_deref()) {
                (File, Some(b"strategic_region")) => Shape::Object(Region),
                (Region, Some(b"id" | b"name" | b"naval_terrain")) => Shape::One,
                (Region, Some(b"provinces")) => Shape::List, // 1401F91B0, appends.
                (Region, Some(b"weather")) => Shape::Object(Weather),
                (Region, Some(b"static_modifiers")) => Shape::Object(Modifiers),
                (Modifiers, _) => Shape::One,
                (Weather, Some(b"period")) => Shape::Object(Period),
                (Weather, Some(b"position")) => Shape::SkipIfOpen,
                (Weather, _) => Shape::SkipBreak,
                // CWeatherChancePeriod 1419FB6C0: range overrides Read with
                // 1419FB570 (raw fixed5 list); chance's 1419FB540 consumes just
                // the value token, despite calling ReadObjectFromParser.
                // `between` is ReadStringList (1401426E0, into +0xC0); the
                // temperature range 1419FB570 -> 140604600 reads the lexer's
                // tokens and leaves the value slot.
                (Period, Some(b"between")) => Shape::List,
                (Period, Some(b"temperature")) => Shape::RawList,
                (Period, Some(b"temperature_day_night")) => Shape::SkipIfOpen,
                (
                    Period,
                    Some(
                        b"no_phenomenon" | b"rain_light" | b"rain_heavy" | b"snow" | b"blizzard"
                        | b"mud" | b"sandstorm" | b"arctic_water" | b"min_snow_level",
                    ),
                ) => Shape::One,
                _ => Shape::Skip,
            }
        },
    );
    let roots: Vec<_> = document
        .items
        .iter()
        .filter(|item| item.keyword_is("strategic_region"))
        .collect();
    let root = match roots.as_slice() {
        [] => return Ok(None),
        [root] => root,
        _ => {
            return Err(invalid(format!(
                "strategic region file has {} strategic_region definitions; exactly one is supported",
                roots.len()
            )));
        }
    };
    let mut out = StrategicRegion::default();
    let mut id = -1;
    for item in block(&root.value, "strategic_region")? {
        if item.keyword_is("id") {
            // The reader scans into a fresh -1, even on repeated fields.
            id = match scan_i32(item.value.scalar().unwrap_or_default()) {
                Scanned::Stored(value) => value,
                _ => -1,
            };
        } else if item.keyword_is("name") {
            out.name_key = item
                .value
                .read_str()
                .map_err(|e| invalid(format!("strategic region name: {e}")))?
                .into();
        } else if item.keyword_is("naval_terrain") {
            out.naval_terrain = Some(
                item.value
                    .read_str()
                    .map_err(|e| invalid(format!("strategic region naval_terrain: {e}")))?
                    .into(),
            );
        } else if item.keyword_is("provinces") {
            for province in block(&item.value, "strategic region provinces")? {
                let value = match scan_i32(province.value.scalar().unwrap_or_default()) {
                    Scanned::Stored(value) => value,
                    _ => 0,
                };
                if value <= 0 {
                    return Err(invalid(format!(
                        "strategic region provinces: unsupported province ID {value}"
                    )));
                }
                out.provinces.push(value as u32);
            }
        } else if item.keyword_is("weather") {
            for period in block(&item.value, "region weather")?
                .iter()
                .filter(|field| field.keyword_is("period"))
            {
                for field in block(&period.value, "region weather period")? {
                    if field.keyword_is("temperature")
                        && field.value.items().is_some_and(|values| values.len() < 2)
                    {
                        return Err(invalid(
                            "strategic region weather temperature: native range reads uninitialized values with fewer than two entries",
                        ));
                    }
                }
            }
        }
    }
    if id <= 0 {
        return Err(invalid(format!(
            "strategic region has no id greater than zero (stored {id})"
        )));
    }
    // A region without `name` (The Great War Redux's 241-Dummy.txt) keeps
    // the empty key: a consumer that names it refuses that name alone.
    if out.name_key == "{" {
        return Err(invalid(
            "strategic region has no name supported for localisation",
        ));
    }
    out.id = id as u32;
    Ok(Some(out))
}

/// Reads every `map/strategicregions/*.txt` file from the resolved layers.
pub fn read(layers: &ResolvedLayers) -> io::Result<BTreeMap<u32, StrategicRegion>> {
    let mut regions = BTreeMap::new();
    let mut sources: BTreeMap<u32, String> = BTreeMap::new();
    let mut paths = layers
        .paths()
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            lower.starts_with("map/strategicregions/") && lower.ends_with(".txt")
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.sort();
    for path in paths {
        let Some(region) = parse_file(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?
        else {
            continue;
        };
        let id = region.id;
        if let Some(previous) = sources.insert(id, path.clone()) {
            return Err(invalid(format!(
                "strategic region {id} is defined by both {previous} and {path}"
            )));
        }
        regions.insert(id, region);
    }
    Ok(regions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_weather_recovery_and_appending_provinces() {
        let region = parse_region(
            br#"@n=010 STRATEGIC_REGION={id=@n name=key provinces={1}
            provinces={0x2} "id"=99 weather={unknown={nested={}} name=after}
            name=outside}"#,
        )
        .unwrap();
        assert_eq!(region.id, 8);
        assert_eq!(region.provinces, [1, 2]);
        // Unknown weather key skips its block, then the closing read eats
        // `name`; the enclosing region reader recovers from that token.
        assert_ne!(region.name_key, "outside");
        // The temperature range (1419FB570 -> 140604600) leaves the value
        // slot at its `{`, which an operator-less `name` then reads; the
        // `between` list (1401426E0) leaves its last token there.
        let name = |range: &str| {
            parse_region(
                format!(
                    "strategic_region={{id=1 name=R weather={{period={{{range}={{1 2}}}}}} name ignored}}"
                )
                .as_bytes(),
            )
            .map(|region| region.name_key)
        };
        // A name of `{` is no localisation key: refused by name.
        assert!(
            name("temperature")
                .unwrap_err()
                .to_string()
                .contains("no name")
        );
        assert_eq!(name("between").unwrap(), "2");
    }

    #[test]
    fn a_region_keeps_its_identity_and_membership_and_ignores_weather() {
        let region = parse_region(
            br#"strategic_region={
                id=1
                name="STRATEGICREGION_1"
                provinces={ 221 271 296 }
                weather={ period={ between={ 0.0 1.12 } snow=unparsed } }
            }"#,
        )
        .unwrap();
        assert_eq!(region.id, 1);
        assert_eq!(region.name_key, "STRATEGICREGION_1");
        assert_eq!(region.provinces, vec![221, 271, 296]);
        assert_eq!(region.naval_terrain, None);
    }

    #[test]
    fn a_naval_region_records_its_terrain_and_an_empty_roster_is_allowed() {
        let region = parse_region(
            br#"strategic_region={ id=100 name="STRATEGICREGION_100"
                naval_terrain=ocean provinces={} }"#,
        )
        .unwrap();
        assert_eq!(region.naval_terrain.as_deref(), Some("ocean"));
        assert!(region.provinces.is_empty());
    }

    #[test]
    fn a_region_without_a_name_keeps_the_empty_key() {
        let region = parse_region(br#"strategic_region={ id=241 weather={} }"#).unwrap();
        assert_eq!((region.id, region.name_key.as_str()), (241, ""));
    }

    #[test]
    fn a_blanked_file_declares_no_region() {
        // Makeshift Bridges and The Great War blank vanilla region files.
        for input in [
            &b""[..],
            b"
",
            b"# removed",
        ] {
            assert_eq!(parse_file(input).unwrap(), None);
        }
    }

    #[test]
    fn malformed_regions_are_named() {
        for (input, message) in [
            (br#"id=1"#.as_slice(), "strategic region file has 0"),
            (br#"strategic_region={ name="x" }"#, "has no id"),
            (
                br#"strategic_region={ id=1 name="x" provinces={ a } }"#,
                "provinces",
            ),
            (br#"strategic_region={ id=1 name={ } }"#, "has no name"),
        ] {
            let error = parse_region(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }
}
