
use std::{collections::BTreeMap, io};

use content_reader_script::native::{self, Shape};
use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fields::{block, invalid},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrategicRegion {
    pub id: u32,
    pub name_key: String,
    pub provinces: Vec<u32>,
    pub naval_terrain: Option<String>,
    pub static_modifiers: bool,
    pub static_modifier_names: Vec<String>,
    pub periods: Vec<weather_period::Period>,
    pub periods_refused: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StrategicRegions {
    pub regions: BTreeMap<u32, StrategicRegion>,
    pub notes: Vec<String>,
}

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
                (Region, Some(b"provinces")) => Shape::List,
                (Region, Some(b"weather")) => Shape::Object(Weather),
                (Region, Some(b"static_modifiers")) => Shape::Object(Modifiers),
                (Modifiers, _) => Shape::One,
                (Weather, Some(b"period")) => Shape::Object(Period),
                (Weather, Some(b"position")) => Shape::SkipIfOpen,
                (Weather, _) => Shape::SkipBreak,
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
                match parse_period(period)? {
                    Ok(period) => out.periods.push(period),
                    Err(reason) => {
                        out.periods_refused.get_or_insert(reason);
                    }
                }
            }
        }
    }
    if out.name_key == "{" {
        return Err(invalid(
            "strategic region has no name supported for localisation",
        ));
    }
    out.id = id as u32;
    Ok((id, out))
}

fn parse_period(
    period: &crate::parser::Item,
) -> io::Result<Result<weather_period::Period, String>> {
    use weather_period::{PERIOD_DEFAULT_DATE, PERIOD_DEFAULT_TEMPERATURE, apply_bound};
    let [mut minimum, mut maximum] = PERIOD_DEFAULT_TEMPERATURE;
    let mut bounds = [PERIOD_DEFAULT_DATE; 2];
    for field in block(&period.value, "region weather period")? {
        if field.keyword_is("temperature") {
            let Some(values) = field.value.items() else {
                return Ok(Err("weather period temperature is not a list".into()));
            };
            let [low, high, ..] = values else {
                return Err(invalid(
                    "strategic region weather temperature: native range reads uninitialized values with fewer than two entries",
                ));
            };
            let fixed = |value: &crate::parser::Item| {
                value
                    .value
                    .scalar()
                    .and_then(crate::fixed5::script_fixed5)
                    .unwrap_or(0)
            };
            [minimum, maximum] = [fixed(low), fixed(high)];
        } else if field.keyword_is("between") {
            let Some(values) = field.value.items() else {
                return Ok(Err("weather period between is not a list".into()));
            };
            if values.len() != 2 {
                continue;
            }
            for (bound, value) in bounds.iter_mut().zip(values) {
                let Some(text) = value.value.scalar() else {
                    return Ok(Err("weather period between bound is a block".into()));
                };
                let parts: Vec<&[u8]> = crate::parser::c_string(text)
                    .split(|&byte| byte == b'.')
                    .collect();
                if let [day, month] = parts[..] {
                    match apply_bound(
                        *bound,
                        crate::fixed5::decimal_i32(day),
                        crate::fixed5::decimal_i32(month),
                    ) {
                        Ok(date) => *bound = date,
                        Err(reason) => {
                            return Ok(Err(format!(
                                "weather period bound {}: {reason}",
                                String::from_utf8_lossy(text)
                            )));
                        }
                    }
                }
            }
        }
    }
    Ok(Ok(weather_period::Period {
        start: bounds[0],
        end: bounds[1],
        minimum,
        maximum,
    }))
}

impl StrategicRegion {
    pub fn native_provinces(&self, exists: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut seen = std::collections::BTreeSet::new();
        self.provinces
            .iter()
            .copied()
            .filter(|&province| exists(province) && seen.insert(province))
            .collect()
    }
}

pub fn read(layers: &ResolvedLayers) -> io::Result<StrategicRegions> {
    layers.recorded_product("strategic regions", 1, &[], read_regions).map(|regions| regions.as_ref().clone())
}

fn read_regions(layers: &ResolvedLayers) -> io::Result<StrategicRegions> {
    let mut database = StrategicRegions::default();
    let mut sources: BTreeMap<u32, String> = BTreeMap::new();
    for path in layers.directory_files("map/strategicregions", ".txt")? {
        let regions = parse_file(&path.read(layers)?)
            .map_err(|error| invalid(format!("{path}: {error}")))?;
        for (id, region) in regions {
            if id < 1 {
                database.notes.push(format!(
                    "{path}: strategic region with id {id} is deleted by the game because its id is not positive"
                ));
                continue;
            }
            if let Some(previous) = sources.insert(region.id, path.to_string()) {
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

    fn parse_region(input: &[u8]) -> io::Result<StrategicRegion> {
        let mut regions = parse_file(input)?;
        assert_eq!(regions.len(), 1);
        Ok(regions.remove(0).1)
    }

    #[test]
    fn native_province_vector_skips_missing_and_repeated_ids_in_authored_order() {
        let region = parse_region(
            br#"strategic_region={id=3 name=key provinces={9 4 9 7 2} provinces={4 5}}"#,
        )
        .unwrap();
        assert_eq!(region.provinces, [9, 4, 9, 7, 2, 4, 5]);
        assert_eq!(region.native_provinces(|id| id != 7), [9, 4, 2, 5]);
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
        assert_ne!(region.name_key, "outside");
        let name = |range: &str| {
            parse_region(
                format!(
                    "strategic_region={{id=1 name=R weather={{period={{{range}={{1 2}}}}}} name ignored}}"
                )
                .as_bytes(),
            )
            .map(|region| region.name_key)
        };
        assert!(
            name("temperature")
                .unwrap_err()
                .to_string()
                .contains("no name")
        );
        assert_eq!(name("between").unwrap(), "2");
    }

    #[test]
    fn a_region_keeps_its_identity_and_membership() {
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
    fn weather_periods_keep_their_order_bounds_and_temperature_ranges() {
        use weather_period::{PERIOD_DEFAULT_DATE as YEAR_ONE, Period};
        let day = |days: i32| YEAR_ONE + days * 24;
        let region = parse_region(
            br#"strategic_region={ id=1 name=R provinces={}
                weather={
                    period={ between={ 0.0 1.12 } temperature={ -12.5 3 9 } snow=0.1 }
                    period={ }
                    position={ x=1 }
                }
                weather={
                    period={ between={ 30.1 30.2 } temperature={ 1 2 } temperature={ x 4.25 } }
                    period={ between={ 4.0 } between={ 5 "6.5.0" } between={ 9.3 "0.4." } }
                }
            }"#,
        )
        .unwrap();
        assert_eq!(region.periods_refused, None);
        assert_eq!(
            region.periods,
            [
                Period {
                    start: day(0),
                    end: day(366),
                    minimum: -1_250_000,
                    maximum: 300_000
                },
                Period {
                    start: day(0),
                    end: day(0),
                    minimum: -1_000_000,
                    maximum: 3_500_000
                },
                Period {
                    start: day(61),
                    end: day(89),
                    minimum: 0,
                    maximum: 425_000
                },
                Period {
                    start: day(99),
                    end: day(0),
                    minimum: -1_000_000,
                    maximum: 3_500_000
                },
            ]
        );
        let refused = parse_region(
            br#"strategic_region={ id=1 name=R weather={ period={ between={ 0.13 1.0 } } } }"#,
        )
        .unwrap();
        assert!(refused.periods_refused.unwrap().contains("month"));
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

    #[test]
    fn a_file_may_hold_several_regions_and_ids_are_kept_as_stored() {
        let regions =
            parse_file(br#"strategic_region={ id=3 } other=1 strategic_region={ name=x }"#)
                .unwrap();
        let ids: Vec<i32> = regions.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, [3, -1]);
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn the_database_replaces_by_id_and_deletes_ids_below_one() {
        let root = std::env::temp_dir().join(format!("content-reader-regions-{}", std::process::id()));
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
