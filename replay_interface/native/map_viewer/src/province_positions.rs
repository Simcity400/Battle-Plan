//! Province map anchors for every province in the admitted `map/provinces.bmp`,
//! in the raster space `ViewerMapCameraState::screen_to_map` returns.

use crate::image_load::{load_definition_csv, load_provinces_bmp};
use crate::mod_resolver::GameFileResolver;
use crate::rail_files::{
    compute_province_centroids, ProvinceCentroid, DEFINITION_CSV_SOURCE, PROVINCES_BMP_SOURCE,
};
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct ProvinceAnchors {
    map_width: usize,
    map_height: usize,
    centroids: BTreeMap<u32, ProvinceCentroid>,
}

impl ProvinceAnchors {
    pub fn new(
        map_width: usize,
        map_height: usize,
        centroids: BTreeMap<u32, ProvinceCentroid>,
    ) -> ProvinceAnchors {
        ProvinceAnchors {
            map_width,
            map_height,
            centroids,
        }
    }

    pub fn map_width(&self) -> usize {
        self.map_width
    }

    pub fn map_height(&self) -> usize {
        self.map_height
    }

    pub fn len(&self) -> usize {
        self.centroids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.centroids.is_empty()
    }

    /// Raster-space anchor, `None` for a province with no pixel in the raster.
    pub fn anchor(&self, province_id: u32) -> Option<&ProvinceCentroid> {
        self.centroids.get(&province_id)
    }
}

/// Anchor every province the recording's admitted raster actually paints. The
/// raster is dropped before returning: only the anchors are retained.
pub fn province_anchors_from_resolver(resolver: &GameFileResolver) -> Result<ProvinceAnchors> {
    let definition_csv = resolver.resolve_required(DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let raster = load_provinces_bmp(&provinces_bmp, &color_to_id)?;
    if raster.unknown_pixels != 0 {
        return Err(format!(
            "province anchors blocked: {} pixel(s) in {PROVINCES_BMP_SOURCE} name no \
             {DEFINITION_CSV_SOURCE} province",
            raster.unknown_pixels
        )
        .into());
    }
    let painted: BTreeSet<u32> = raster.ids.iter().copied().filter(|id| *id != 0).collect();
    let centroids = compute_province_centroids(&raster.ids, raster.width, raster.height, &painted);
    if centroids.is_empty() {
        return Err(
            format!("province anchors blocked: {PROVINCES_BMP_SOURCE} painted no province pixel")
                .into(),
        );
    }
    Ok(ProvinceAnchors {
        map_width: raster.width,
        map_height: raster.height,
        centroids,
    })
}
