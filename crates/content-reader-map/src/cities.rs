
use std::io;

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt::{self, Scanned}, fields::{block, invalid}};
use content_reader_image::IndexedImage;
use content_reader_script::native::{self, Shape};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CityBuilding {
    pub distance: u32,
    pub meshes: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CityGroup {
    pub color_index: u8,
    pub density: f64,
    pub buildings: Vec<CityBuilding>,
}

impl CityGroup {
    pub fn building_for(&self, distance: u32) -> Option<&CityBuilding> {
        self.buildings
            .iter()
            .rev()
            .find(|building| building.distance <= distance)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cities {
    pub types_source: String,
    pub pixel_step_x: u32,
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

fn integer(field: &content_reader_parser::parser::Item, errors: &mut Vec<String>) -> Scanned<i32> {
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

pub fn parse(input: &[u8]) -> io::Result<Cities> {
    #[derive(Clone, Copy)]
    enum Node {
        File,
        Group,
        Building,
    }
    let mut errors = Vec::new();
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
                let mut group = CityGroup::default();
                for field in block(&field.value, "city_group")? {
                    match field.keyword().map(|key| key.folded()).as_deref() {
                        Some(b"color_index") => {
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

pub fn read(layers: &ResolvedLayers) -> io::Result<(Cities, IndexedImage)> {
    let cities = parse(&layers.read("map/cities.txt")?)
        .map_err(|error| invalid(format!("map/cities.txt: {error}")))?;
    let path = cities.types_source.clone();
    let image = crate::terrain_images::read_indexed_image(layers, &path)?;
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
        assert_eq!((cities.pixel_step_x, cities.pixel_step_y), (2, 1));
        assert_eq!(cities.groups.len(), 1);
        assert_eq!(cities.groups[0].color_index, 15);
        assert_eq!(cities.groups[0].density, 0.0);
        assert_eq!(cities.groups[0].buildings[0].meshes, ["a", "b"]);
        assert_eq!(cities.groups[0].buildings[0].distance, 3);
    }

    #[test]
    fn a_fractional_distance_stores_its_integer_part() {
        let cities =
            parse(b"types_source = x city_group = { building = { distance = 0.5 } }").unwrap();
        assert_eq!(cities.groups[0].buildings[0].distance, 0);
    }

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
