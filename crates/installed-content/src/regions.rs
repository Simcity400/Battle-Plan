//! Strategic regions from `map/strategicregions/*.txt`: identity, membership
//! and naval terrain. Weather periods drive the simulation, which the replay
//! records, so they are not read.
//!
//! The database lists the directory's own `.txt` files without recursion
//! and reads every top-level `strategic_region` as a new template. An id
//! below 1 is discarded; any other replaces the earlier template in its slot.

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
    /// Whether the effective template declares a `static_modifiers` block;
    /// its naval grid needs additional live operands when present.
    pub static_modifiers: bool,
    /// Ordered `static_modifiers` names. Each key/value pair appends, and
    /// the naval grid uses this order.
    pub static_modifier_names: Vec<String>,
}

/// The database as the game keeps it, by id, and what it deleted or
/// replaced on the way, named by file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrategicRegions {
    pub regions: BTreeMap<u32, StrategicRegion>,
    pub notes: Vec<String>,
}

/// Every region in one file, in source order, with its id scanned as a
/// signed integer. A file declaring no strategic region adds none, so
/// a mod can blank a lower layer's file to remove its regions.
pub fn parse_file(input: &[u8]) -> io::Result<Vec<(i32, StrategicRegion)>> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Region,
        Weather,
        Period,
        Modifiers,
    }
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::File(Node::File),
        None,
        &mut |node, field| {
            use Node::*;
            match (node, field.keyword().as_deref()) {
                (File, Some(b"strategic_region")) => Shape::Object(Region),
                (Region, Some(b"id" | b"name" | b"naval_terrain")) => Shape::One,
                (Region, Some(b"provinces")) => Shape::List, // Appends.
                (Region, Some(b"weather")) => Shape::Object(Weather),
                (Region, Some(b"static_modifiers")) => Shape::Object(Modifiers),
                (Modifiers, _) => Shape::One,
                (Weather, Some(b"period")) => Shape::Object(Period),
                (Weather, Some(b"position")) => Shape::SkipIfOpen,
                (Weather, _) => Shape::SkipBreak,
                // Weather ranges consume raw fixed5 lists; chance consumes
                // just the value token. `between` reads a string list, while
                // the temperature range reads lexer tokens and leaves the
                // value slot unchanged.
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
    document
        .items
        .iter()
        .filter(|item| item.keyword_is("strategic_region"))
        .map(parse_region)
        .collect()
}

fn parse_region(root: &crate::parser::Item) -> io::Result<(i32, StrategicRegion)> {
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
        } else if item.keyword_is("static_modifiers") {
            out.static_modifiers = true;
            for modifier in block(&item.value, "strategic region static_modifiers")? {
                let name = modifier.key_str().map_err(|error| {
                    invalid(format!("strategic region static modifier name: {error}"))
                })?;
                out.static_modifier_names.push(name.to_owned());
            }
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
    // A region without `name` keeps empty constructor text, which its
    // name lookup returns without an emptiness gate.
    if out.name_key == "{" {
        return Err(invalid(
            "strategic region has no name supported for localisation",
        ));
    }
    out.id = id as u32;
    Ok((id, out))
}

/// Reads every region of the stack's `map/strategicregions` files.
pub fn read(layers: &ResolvedLayers) -> io::Result<StrategicRegions> {
    let mut database = StrategicRegions::default();
    let mut sources: BTreeMap<u32, String> = BTreeMap::new();
    for path in layers.directory_files("map/strategicregions", ".txt")? {
        let regions = parse_file(&layers.read(&path)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        for (id, region) in regions {
            if id < 1 {
                database.notes.push(format!(
                    "{path}: strategic region with id {id} is deleted by the game because its id is not positive"
                ));
                continue;
            }
            if let Some(previous) = sources.insert(region.id, path.clone()) {
                database.notes.push(format!(
                    "{path}: strategic region {id} replaces the one {previous} defined"
                ));
            }
            database.regions.insert(region.id, region);
        }
    }
    Ok(database)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file's one region.
    fn parse_region(input: &[u8]) -> io::Result<StrategicRegion> {
        let mut regions = parse_file(input)?;
        assert_eq!(regions.len(), 1);
        Ok(regions.remove(0).1)
    }

    #[test]
    fn weather_recovery_preserves_name_and_appends_provinces() {
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
        // A temperature range leaves the value slot at its `{`, which an
        // operator-less `name` then reads; the `between` list leaves its
        // last token there.
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
        assert!(!region.static_modifiers);
    }

    #[test]
    fn a_naval_region_records_its_terrain_and_an_empty_roster_is_allowed() {
        let region = parse_region(
            br#"strategic_region={ id=100 name="STRATEGICREGION_100"
                naval_terrain=ocean provinces={} static_modifiers={
                    navy_visibility=always "rough_sea"=only_at_war navy_visibility=never
                } }"#,
        )
        .unwrap();
        assert_eq!(region.naval_terrain.as_deref(), Some("ocean"));
        assert!(region.static_modifiers);
        assert_eq!(
            region.static_modifier_names,
            ["navy_visibility", "rough_sea", "navy_visibility"]
        );
        assert!(region.provinces.is_empty());
    }

    #[test]
    fn a_region_without_a_name_keeps_the_empty_key() {
        let region = parse_region(br#"strategic_region={ id=241 weather={} }"#).unwrap();
        assert_eq!((region.id, region.name_key.as_str()), (241, ""));
    }

    #[test]
    fn a_blanked_file_declares_no_region() {
        // A mod can blank a lower-layer region file.
        for input in [
            &b""[..],
            b"
",
            b"# removed",
        ] {
            assert!(parse_file(input).unwrap().is_empty());
        }
    }

    #[test]
    fn malformed_regions_are_named() {
        for (input, message) in [
            (
                br#"strategic_region={ id=1 name="x" provinces={ a } }"#.as_slice(),
                "provinces",
            ),
            (br#"strategic_region={ id=1 name={ } }"#, "has no name"),
        ] {
            let error = parse_region(input).unwrap_err().to_string();
            assert!(error.contains(message), "{error} lacks {message}");
        }
    }

    /// Every region in a file is read, with its id scanned as a signed
    /// integer into a fresh -1 when absent.
    #[test]
    fn a_file_may_hold_several_regions_and_ids_are_kept_as_stored() {
        let regions =
            parse_file(br#"strategic_region={ id=3 } other=1 strategic_region={ name=x }"#)
                .unwrap();
        let ids: Vec<i32> = regions.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [3, -1]);
    }

    /// The database stores each region in its id's slot: a later file
    /// replaces an earlier one's, an id below 1 is deleted, and both are
    /// named. Subdirectories are not listed.
    #[cfg(feature = "test-support")]
    #[test]
    fn the_database_replaces_by_id_and_deletes_ids_below_one() {
        let root = std::env::temp_dir().join(format!("installed-content-regions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "map/strategicregions/1-a.txt",
            "strategic_region={ id=1 name=A }",
        );
        write(
            "map/strategicregions/2-b.txt",
            "strategic_region={ id=1 name=B } strategic_region={ id=0 name=Z }",
        );
        write("map/strategicregions/3-empty.txt", "");
        write(
            "map/strategicregions/sub/4.txt",
            "strategic_region={ id=4 name=D }",
        );
        let layers = ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let database = read(&layers).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(database.regions.keys().copied().collect::<Vec<_>>(), [1]);
        assert_eq!(database.regions[&1].name_key, "B");
        assert_eq!(database.notes.len(), 2, "{:?}", database.notes);
        assert_eq!(
            database.notes[0],
            "map/strategicregions/2-b.txt: strategic region 1 replaces the one map/strategicregions/1-a.txt defined"
        );
        assert_eq!(
            database.notes[1],
            "map/strategicregions/2-b.txt: strategic region with id 0 is deleted by the game because its id is not positive"
        );
    }
}
