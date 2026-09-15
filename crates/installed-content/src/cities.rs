//! `map/cities.txt` and `map/cities.bmp`: the city-sprawl style table.
//!
//! The file names a bitmap of *styles*, not of footprints. `cities.bmp` is the
//! map's own size and its palette index at a pixel selects a `city_group`;
//! where the sprawl actually stands comes from `map/terrain.bmp`, whose one
//! `spawn_city = yes` palette entry marks the urban blobs (research report R3
//! section 4.1). Inside a blob, `distance` is the pixel's distance to the edge
//! of the blob in map pixels, and each `building` block claims one distance
//! ring with a set of entity names to pick from at random.

use std::io;

use crate::{
    ResolvedLayers,
    fields::{block, integer, invalid, number, require, text},
    image::{IndexedImage, bmp},
    parser,
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
    /// The `building` blocks, sorted by ascending `distance`.
    pub buildings: Vec<CityBuilding>,
}

impl CityGroup {
    /// The ring a pixel at this distance from the blob edge belongs to.
    ///
    /// The file's own comment says the blocks are "sorted by distance
    /// (growing)", so a pixel takes the last block whose `distance` it reaches;
    /// beyond the last one it stays in the last block, which is what puts the
    pub fn building_for(&self, distance: u32) -> Option<&CityBuilding> {
        let mut chosen = None;
        for building in &self.buildings {
            if building.distance <= distance {
                chosen = Some(building);
            }
        }
        chosen
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

/// Parses `map/cities.txt`.
///
/// Errors are named `InvalidData` for a missing `types_source`, a pixel step
/// that is not a positive integer, a `city_group` with no `color_index`, a
/// `color_index` outside a byte, and a `building` block with no `distance`.
pub fn parse(input: &[u8]) -> io::Result<Cities> {
    let document = parser::parse(input)?;
    let items = &document.items;
    let mut cities = Cities {
        types_source: text(
            require(items, "types_source", "map/cities.txt")?,
            "types_source",
        )?
        .to_owned(),
        pixel_step_x: integer(
            require(items, "pixel_step_x", "map/cities.txt")?,
            "pixel_step_x",
        )?,
        pixel_step_y: integer(
            require(items, "pixel_step_y", "map/cities.txt")?,
            "pixel_step_y",
        )?,
        groups: Vec::new(),
    };
    if cities.pixel_step_x == 0 || cities.pixel_step_y == 0 {
        return Err(invalid("map/cities.txt has a zero pixel step"));
    }
    for value in crate::fields::all(items, b"city_group") {
        let entries = block(value, "city_group")?;
        let index: u32 = integer(
            require(entries, "color_index", "city_group")?,
            "color_index",
        )?;
        let color_index = u8::try_from(index)
            .map_err(|_| invalid(format!("city_group color_index {index} is not a byte")))?;
        let mut group = CityGroup {
            color_index,
            density: match crate::fields::last(entries, b"density") {
                Some(value) => number(value, "density")?,
                None => 1.0,
            },
            buildings: Vec::new(),
        };
        for value in crate::fields::all(entries, b"building") {
            let fields = block(value, "city_group building")?;
            let mut meshes = Vec::new();
            if let Some(list) = crate::fields::last(fields, b"mesh") {
                for item in block(list, "city_group building mesh")? {
                    meshes.push(text(&item.value, "city_group building mesh")?.to_owned());
                }
            }
            group.buildings.push(CityBuilding {
                distance: integer(
                    require(fields, "distance", "city_group building")?,
                    "distance",
                )?,
                meshes,
            });
        }
        group.buildings.sort_by_key(|building| building.distance);
        cities.groups.push(group);
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
    fn reads_groups_sorted_by_distance() {
        let cities = parse(SAMPLE).unwrap();
        assert_eq!(cities.types_source, "map/cities.bmp");
        assert_eq!((cities.pixel_step_x, cities.pixel_step_y), (2, 2));
        let group = cities.group_for(15).unwrap();
        assert_eq!(group.density, 0.9);
        assert_eq!(group.buildings[0].distance, 1);
        assert_eq!(group.buildings[1].meshes, ["b", "c"]);
        assert!(cities.group_for(3).is_none());
    }

    #[test]
    fn a_group_with_no_building_block_places_nothing() {
        let cities = parse(SAMPLE).unwrap();
        assert!(cities.group_for(0).unwrap().building_for(4).is_none());
    }

    #[test]
    fn the_ring_is_the_last_block_the_distance_reaches() {
        let cities = parse(SAMPLE).unwrap();
        let group = cities.group_for(15).unwrap();
        assert!(group.building_for(0).is_none());
        assert_eq!(group.building_for(1).unwrap().distance, 1);
        assert_eq!(group.building_for(2).unwrap().distance, 2);
        assert_eq!(group.building_for(9).unwrap().distance, 2);
    }

    #[test]
    fn a_zero_pixel_step_is_refused() {
        assert!(parse(b"types_source = \"x\"\npixel_step_x = 0\npixel_step_y = 2\n").is_err());
    }
}
