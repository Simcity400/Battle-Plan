//! `map/cities.txt` and `map/cities.bmp`: the city-sprawl style table.
//!
//! The file names a bitmap of *styles*, not of footprints. `cities.bmp` is the
//! map's own size and its palette index at a pixel selects a `city_group`;
//! where the sprawl actually stands comes from `map/terrain.bmp`, whose
//! `spawn_city = yes` palette entry marks the urban blobs. Inside a blob,
//! `distance` is the pixel's distance to the edge in map pixels, and each
//! `building` block claims one distance ring with entity names to choose
//! from at random.
//!
//! Any recorded reader error prevents city-sprawl construction, so a file
//! with such an error is refused by name.

use std::io;

use crate::{
    ResolvedLayers,
    crt::{self, Scanned},
    fields::{block, invalid},
    image::{IndexedImage, bmp},
    parser::native::{self, Shape},
};

/// One `building` block: a distance ring and the entities that may stand in it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CityBuilding {
    /// Distance to the edge of the urban area, in map pixels.
    pub distance: u32,
    /// Entity names in `gfx/entities/*.asset`; the engine picks one at random.
    pub meshes: Vec<String>,
}

/// One `city_group` block: an architecture style keyed by palette index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CityGroup {
    /// The `map/cities.bmp` palette index this style claims.
    pub color_index: u8,
    /// Signed displacement strength toward the ring-weighted neighbouring houses.
    pub density: f64,
    /// The `building` blocks in declaration order.
    pub buildings: Vec<CityBuilding>,
}

impl CityGroup {
    /// The ring a pixel at this distance from the blob edge belongs to:
    /// scan the rings from the last declared backwards and take the first
    /// whose `distance` the pixel reaches, meshes or none.
    pub fn building_for(&self, distance: u32) -> Option<&CityBuilding> {
        self.buildings
            .iter()
            .rev()
            .find(|building| building.distance <= distance)
    }
}

/// The whole of `map/cities.txt`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cities {
    /// `types_source`, the bitmap the palette indices are read from.
    pub types_source: String,
    /// One sample per this many map pixels in x.
    pub pixel_step_x: u32,
    /// One sample per this many map pixels in y.
    pub pixel_step_y: u32,
    pub groups: Vec<CityGroup>,
}

impl Cities {
    pub fn group_for(&self, color_index: u8) -> Option<&CityGroup> {
        self.groups
            .iter()
            .find(|group| group.color_index == color_index)
    }
}

/// Scan a field as an integer prefix; text with no digit records a reader
/// error, while an empty input keeps its separate end-of-input outcome.
fn integer(field: &crate::parser::Item, errors: &mut Vec<String>) -> Scanned<i32> {
    let text = field.value.scalar().unwrap_or_default();
    let scanned = crt::scan_i32(text);
    if scanned == Scanned::NoMatch {
        errors.push(format!(
            "{} = {} does not scan as an integer",
            String::from_utf8_lossy(field.key.as_deref().unwrap_or_default()),
            String::from_utf8_lossy(text)
        ));
    }
    scanned
}

/// Reads city settings, groups and distance-ring meshes with persistent
/// field consumption. Both pixel steps start at 1.
pub fn parse(input: &[u8]) -> io::Result<Cities> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Group,
        Building,
    }
    let mut errors = Vec::new();
    // The lexer keeps the leading mark for this map reader.
    let document = native::read(
        input,
        native::Mode::KeepMark,
        native::Top::Persistent(Node::File),
        None,
        &mut |node, field| match (node, field.keyword().as_deref()) {
            (Node::File, Some(b"city_group")) => Shape::Object(Node::Group),
            (Node::Group, Some(b"building")) => Shape::Object(Node::Building),
            (Node::Building, Some(b"mesh")) => Shape::List,
            (Node::File, Some(b"types_source" | b"pixel_step_x" | b"pixel_step_y"))
            | (Node::Group, Some(b"color_index" | b"density"))
            | (Node::Building, Some(b"distance")) => Shape::One,
            // Unexpected fields record a reader error before their value is skipped.
            _ => {
                errors.push(format!(
                    "line {}: key {} is reported",
                    field.key.line,
                    String::from_utf8_lossy(&field.key.text)
                ));
                Shape::Skip
            }
        },
    );
    if let Some(unsettled) = document.unsettled.as_ref().or(document.misreads.first()) {
        return Err(invalid(format!(
            "the port cannot tell whether the game records an error here: {unsettled}"
        )));
    }
    let mut cities = Cities {
        pixel_step_x: 1,
        pixel_step_y: 1,
        ..Cities::default()
    };
    for field in &document.items {
        match field.keyword().map(|key| key.folded()).as_deref() {
            Some(b"types_source") => {
                cities.types_source = field
                    .value
                    .read_str()
                    .map_err(|e| invalid(format!("cities types_source: {e}")))?
                    .into()
            }
            Some(b"pixel_step_x" | b"pixel_step_y") => {
                let dst = if field.keyword_is("pixel_step_x") {
                    &mut cities.pixel_step_x
                } else {
                    &mut cities.pixel_step_y
                };
                if let Scanned::Stored(value) = integer(field, &mut errors) {
                    *dst = u32::try_from(value).map_err(|_| {
                        invalid(format!(
                            "cities {}: negative step {value}",
                            String::from_utf8_lossy(field.key.as_deref().unwrap_or_default())
                        ))
                    })?;
                }
            }
            Some(b"city_group") => {
                // The group index and density start at zero.
                let mut group = CityGroup::default();
                for field in block(&field.value, "city_group")? {
                    match field.keyword().map(|key| key.folded()).as_deref() {
                        Some(b"color_index") => {
                            // The local int is uninitialized on a failed scan. Do not guess it.
                            match integer(field, &mut errors) {
                                Scanned::Stored(value) => group.color_index = value as u8,
                                Scanned::NoMatch => {}
                                Scanned::End => {
                                    return Err(invalid(
                                        "city_group color_index: unsupported failed integer scan",
                                    ));
                                }
                            }
                        }
                        Some(b"density") => {
                            let text = field.value.scalar().unwrap_or_default();
                            let scanned = crt::scan_f32(text);
                            if scanned.is_none() && !text.trim_ascii().is_empty() {
                                // A failed float conversion records a reader error.
                                errors.push(format!(
                                    "density = {} does not scan as a float",
                                    String::from_utf8_lossy(text)
                                ));
                            }
                            if let Some(value) = scanned {
                                if !value.is_finite() {
                                    return Err(invalid("city_group density: nonfinite value"));
                                }
                                group.density = f64::from(value);
                            }
                        }
                        Some(b"building") => {
                            let mut building = CityBuilding::default();
                            for field in block(&field.value, "city_group building")? {
                                if field.keyword_is("distance") {
                                    if let Scanned::Stored(value) = integer(field, &mut errors) {
                                        building.distance = u32::try_from(value).map_err(|_| invalid(format!("city_group building distance: negative value {value}")))?;
                                    }
                                } else if field.keyword_is("mesh") {
                                    for item in block(&field.value, "city_group building mesh")? {
                                        building.meshes.push(
                                            item.value
                                                .read_str()
                                                .map_err(|e| {
                                                    invalid(format!(
                                                        "city_group building mesh: {e}"
                                                    ))
                                                })?
                                                .into(),
                                        );
                                    }
                                }
                            }
                            group.buildings.push(building);
                        }
                        _ => {}
                    }
                }
                cities.groups.push(group);
            }
            _ => {}
        }
    }
    if let Some(error) = errors.first() {
        return Err(invalid(format!(
            "{error}: city settings contain a reader error; no city sprawl is built"
        )));
    }
    if cities.types_source.is_empty() {
        return Err(invalid("cities types_source is absent or empty"));
    }
    if cities.pixel_step_x == 0 || cities.pixel_step_y == 0 {
        return Err(invalid("map/cities.txt has a zero pixel step"));
    }
    Ok(cities)
}

/// Reads `map/cities.txt` and the style bitmap it names.
pub fn read(layers: &ResolvedLayers) -> io::Result<(Cities, IndexedImage)> {
    let cities = parse(&layers.read("map/cities.txt")?)
        .map_err(|error| invalid(format!("map/cities.txt: {error}")))?;
    let path = cities.types_source.clone();
    let image = bmp::decode_indexed(&layers.read(&path)?)
        .map_err(|error| invalid(format!("{path}: {error}")))?;
    Ok((cities, image))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"
types_source = "map/cities.bmp"
pixel_step_x = 2
pixel_step_y = 2
city_group = {
    color_index = 15
    density = 0.9
    building = { distance = 2 mesh = { "b" "c" } }
    building = { distance = 1 mesh = { "a" } }
}
city_group = {
    color_index = 0
    density = 0.00001
}
"#;

    #[test]
    fn tokens_defaults_and_repeated_meshes() {
        let cities = parse(
            br#"@step=02 TYPES_SOURCE=x pixel_step_x=@step
            city_group={color_index=0x10F building={distance=03 mesh={a} mesh={b}}}
            } city_group={color_index=7}"#,
        )
        .unwrap();
        // The first `}` ends the persistent read; an absent step keeps its 1.
        assert_eq!((cities.pixel_step_x, cities.pixel_step_y), (2, 1));
        assert_eq!(cities.groups.len(), 1);
        assert_eq!(cities.groups[0].color_index, 15);
        assert_eq!(cities.groups[0].density, 0.0);
        assert_eq!(cities.groups[0].buildings[0].meshes, ["a", "b"]);
        assert_eq!(cities.groups[0].buildings[0].distance, 3);
    }

    /// Old World Blues' `distance = 0.5`: `%i` stores the `0` and stops at
    /// the `.`, which is no error.
    #[test]
    fn a_fractional_distance_stores_its_integer_part() {
        let cities =
            parse(b"types_source = x city_group = { building = { distance = 0.5 } }").unwrap();
        assert_eq!(cities.groups[0].buildings[0].distance, 0);
    }

    /// A reported key (an unknown one, a quoted one, which is no keyword)
    /// and a number that does not scan are recorded errors, after which the
    /// game builds no sprawl.
    #[test]
    fn a_recorded_error_discards_the_file() {
        for input in [
            b"types_source = x unknown = { city_group = { } }".as_slice(),
            b"types_source = x city_group = { \"density\" = 9 }",
            b"types_source = x city_group = { building = { distance = far } }",
            b"types_source = x city_group = { density = high }",
        ] {
            let error = parse(input).unwrap_err().to_string();
            assert!(
                error.contains("city settings contain a reader error; no city sprawl is built"),
                "{error}"
            );
        }
    }

    #[test]
    fn groups_keep_their_rings_in_declaration_order() {
        let cities = parse(SAMPLE).unwrap();
        assert_eq!(cities.types_source, "map/cities.bmp");
        assert_eq!((cities.pixel_step_x, cities.pixel_step_y), (2, 2));
        let group = cities.group_for(15).unwrap();
        assert_eq!(group.density, f64::from(0.9_f32));
        assert_eq!(group.buildings[0].distance, 2);
        assert_eq!(group.buildings[0].meshes, ["b", "c"]);
        assert!(cities.group_for(3).is_none());
    }

    #[test]
    fn a_group_with_no_building_block_places_nothing() {
        let cities = parse(SAMPLE).unwrap();
        assert!(cities.group_for(0).unwrap().building_for(4).is_none());
    }

    /// Rings are scanned from the last declared: with SAMPLE's descending
    /// rings the last, distance 1, answers every distance from 1.
    #[test]
    fn the_ring_is_the_last_declared_one_the_distance_reaches() {
        let cities = parse(SAMPLE).unwrap();
        let group = cities.group_for(15).unwrap();
        assert!(group.building_for(0).is_none());
        assert_eq!(group.building_for(1).unwrap().meshes, ["a"]);
        assert_eq!(group.building_for(9).unwrap().meshes, ["a"]);
        let ascending = parse(
            b"types_source = x city_group = { building = { distance = 1 mesh = { a } } building = { distance = 2 mesh = { b } } }",
        )
        .unwrap();
        let group = &ascending.groups[0];
        assert_eq!(group.building_for(1).unwrap().meshes, ["a"]);
        assert_eq!(group.building_for(9).unwrap().meshes, ["b"]);
    }

    #[test]
    fn a_zero_pixel_step_is_refused() {
        assert!(parse(b"types_source = \"x\"\npixel_step_x = 0\npixel_step_y = 2\n").is_err());
    }
}
