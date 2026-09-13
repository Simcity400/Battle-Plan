//! Strategic regions from `map/strategicregions/*.txt`.
//!
//! Only identity and membership are read. The `weather` periods drive the
//! campaign's weather simulation, which the replay records per province, so
//! they are not reproduced from content here.

use std::{collections::BTreeMap, io};

use crate::{
    ResolvedLayers,
    fields::{block, integer, invalid, last, require, scalar_list, text},
    parser,
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

/// Parses one `map/strategicregions` file.
pub fn parse_region(input: &[u8]) -> io::Result<StrategicRegion> {
    let document = parser::parse(input)?;
    let root = require(&document.items, "strategic_region", "strategic region file")?;
    let items = block(root, "strategic_region")?;
    Ok(StrategicRegion {
        id: integer(
            require(items, "id", "strategic_region")?,
            "strategic region id",
        )?,
        name_key: text(
            require(items, "name", "strategic_region")?,
            "strategic region name",
        )?
        .to_owned(),
        provinces: match last(items, b"provinces") {
            Some(value) => scalar_list(value, "strategic region provinces")?,
            None => Vec::new(),
        },
        naval_terrain: match last(items, b"naval_terrain") {
            Some(value) => Some(text(value, "naval_terrain")?.to_owned()),
            None => None,
        },
    })
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
        let region = parse_region(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
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
    fn a_region_keeps_its_identity_and_membership_and_drops_the_weather() {
        let region = parse_region(
            br#"strategic_region={
                id=1
                name="STRATEGICREGION_1"
                provinces={ 221 271 296 }
                weather={ period={ between={ 0.0 30.0 } snow=0.200 } }
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
    fn malformed_regions_are_named() {
        for (input, message) in [
            (br#"id=1"#.as_slice(), "strategic region file has no"),
            (br#"strategic_region={ name="x" }"#, "has no id"),
            (br#"strategic_region={ id=1 }"#, "has no name"),
            (
                br#"strategic_region={ id=1 name="x" provinces={ a } }"#,
                "provinces",
            ),
            (
                br#"strategic_region={ id=1 name={ } }"#,
                "not a text scalar",
            ),
        ] {
            let error = parse_region(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }
}
