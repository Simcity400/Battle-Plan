use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    base64_bytes, A729_EXECUTABLE_SHA256, SNOW_MUD_PRODUCER_FUNCTION_RVA,
    SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256,
};

pub const XMAP07_PASSIVE_SNOW_MUD_SCHEMA: &str = "hoi4.xmap07.passive-snow-mud.a729.v2";
pub const XMAP07_PASSIVE_SNOW_MUD_KEY: &str = "_xmap07_snow_mud_runtime";
pub const XMAP07_PASSIVE_SNOW_MUD_SCOPE: &str = "world_global";
/// Size-tuning constant for the replay-cache tile array. In the worst case where every tile is
/// dirty, tiling costs only the per-tile framing overhead over the former single payload replace;
/// retune it only by measurement on a recording that actually schedules domain 100.
pub const XMAP07_SNOW_MUD_TILE_EDGE: u32 = 16;
/// Matches the XSM2 producer contract's maximum admitted grid axis.
const XMAP07_SNOW_MUD_MAX_GRID_AXIS: u32 = 16_384;
/// Matches the XSM2 producer contract's maximum admitted decoded grid size.
const XMAP07_SNOW_MUD_MAX_GRID_BYTES: usize = 64 * 1024 * 1024;
pub const XMAP07_SNOW_MUD_FADE_UPDATER_RVA: u32 = 0x0126_7270;
pub const XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256: &str =
    "baacd7d1c462d74b44d8a793805049ddfa6cf1e9f7459177e663f1c9953f5575";
pub const XMAP07_SNOW_MUD_SCENE_COPY_RVA: u32 = 0x00ff_3fc0;
pub const XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256: &str =
    "a995d76eef9a6a295002bd1cad06b4902b86d5a3dd60fb6d10c63bc904f3084f";
pub const XMAP07_SNOW_MUD_FORMAT: &str = "DXGI_FORMAT_B8G8R8A8_UNORM";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Xmap07PassiveSnowMudCarrier {
    pub schema: String,
    pub scope: String,
    pub executable_sha256: String,
    pub texture_producer_rva: u32,
    pub texture_producer_instructions_sha256: String,
    pub fade_updater_rva: u32,
    pub fade_updater_instructions_sha256: String,
    pub scene_copy_rva: u32,
    pub scene_copy_instructions_sha256: String,
    pub process_epoch: String,
    pub campaign_epoch: String,
    pub hook_epoch: String,
    pub reset_epoch_ordinal: u64,
    pub capture_frontier_ordinal: u64,
    pub texture_observation_ordinal: u64,
    pub fade_observation_ordinal: u64,
    pub reset_hour: i64,
    pub initial_snapshot_hour: i64,
    pub payload_origin_hour: i64,
    pub texture_observation_hour: i64,
    pub fade_observation_hour: i64,
    pub displayed_hour: i64,
    pub carry_forward: bool,
    pub reset_complete: bool,
    pub initial_snapshot_complete: bool,
    pub texture_complete: bool,
    pub fade_complete: bool,
    pub width: u32,
    pub height: u32,
    pub row_pitch: u32,
    pub bytes_per_texel: u32,
    pub format: String,
    pub tile_edge: u32,
    pub tiles: Vec<String>,
    pub payload_sha256: String,
    pub payload_blob_sha256: String,
    pub fade_f32_bits: u32,
    pub authority_sha256: String,
}

/// Row-major tiles over the grid. `tiles_x = ceil(width/tile_edge)`,
/// `tiles_y = ceil(height/tile_edge)`, and `index = ty * tiles_x + tx`.
///
/// Edge tiles are partial. A tile at `(tx, ty)` covers
/// `w = min(tile_edge, width - tx*tile_edge)` cells across and
/// `h = min(tile_edge, height - ty*tile_edge)` rows. Its bytes are those `h` rows of `w*4`
/// bytes each, packed tightly without padding to `tile_edge`, from the tightly-pitched grid.
/// THE FIELDS ARE PRIVATE ON PURPOSE. `new` is the only way to obtain a layout, and it proves
/// every invariant the rest of this module then relies on -- the counts agree with the dimensions,
/// the axes are within the producer's ceiling, and `tiles_x * tiles_y` fits `usize`. With the
/// fields public a caller could build a layout whose counts contradict its dimensions, and
/// `tile_count` would hand back a number the conversions would then iterate over; making the type
/// constructor-only turns that runtime hazard into a structural guarantee. The unit tests below
/// are a child module, so they can still reach the fields to prove the guard rejects a
/// hand-corrupted layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xmap07TileLayout {
    tiles_x: u32,
    tiles_y: u32,
    tile_edge: u32,
    width: u32,
    height: u32,
}

impl Xmap07TileLayout {
    pub fn new(width: u32, height: u32, tile_edge: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || tile_edge == 0 {
            return Err("SnowMud tile layout dimensions and tile edge must be nonzero".into());
        }
        if width > XMAP07_SNOW_MUD_MAX_GRID_AXIS || height > XMAP07_SNOW_MUD_MAX_GRID_AXIS {
            return Err(format!(
                "SnowMud tile layout dimensions {width}x{height} exceed the {XMAP07_SNOW_MUD_MAX_GRID_AXIS} axis ceiling"
            ));
        }
        let tiles_x = width
            .checked_div(tile_edge)
            .and_then(|whole| whole.checked_add(u32::from(width % tile_edge != 0)))
            .ok_or_else(|| "SnowMud tile layout horizontal count overflows".to_string())?;
        let tiles_y = height
            .checked_div(tile_edge)
            .and_then(|whole| whole.checked_add(u32::from(height % tile_edge != 0)))
            .ok_or_else(|| "SnowMud tile layout vertical count overflows".to_string())?;
        tiles_x
            .checked_mul(tiles_y)
            .ok_or_else(|| "SnowMud tile count overflows u32".to_string())?;
        checked_product_usize(&[tiles_x, tiles_y], "SnowMud tile count")?;
        let grid_bytes = checked_product_usize(&[width, height, 4], "SnowMud grid byte length")?;
        if grid_bytes > XMAP07_SNOW_MUD_MAX_GRID_BYTES {
            return Err(format!(
                "SnowMud grid byte length {grid_bytes} exceeds the {XMAP07_SNOW_MUD_MAX_GRID_BYTES} byte ceiling"
            ));
        }
        Ok(Self {
            tiles_x,
            tiles_y,
            tile_edge,
            width,
            height,
        })
    }

    pub fn tiles_x(&self) -> u32 {
        self.tiles_x
    }

    pub fn tiles_y(&self) -> u32 {
        self.tiles_y
    }

    pub fn tile_edge(&self) -> u32 {
        self.tile_edge
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Total tiles. Infallible BY CONSTRUCTION: `new` already required this product to fit
    /// `usize`, and it is the only constructor. It deliberately does NOT fall back to a sentinel
    /// on overflow -- a sentinel here would be a count the conversions would loop over.
    pub fn tile_count(&self) -> usize {
        (self.tiles_x as usize) * (self.tiles_y as usize)
    }

    /// `(x0, y0, w, h)` of one tile.
    pub fn tile_rect(&self, index: usize) -> Result<(u32, u32, u32, u32), String> {
        if index >= self.tile_count() || self.tiles_x == 0 || self.tiles_y == 0 {
            return Err(format!("SnowMud tile index {index} is out of bounds"));
        }
        let index =
            u32::try_from(index).map_err(|_| "SnowMud tile index does not fit u32".to_string())?;
        let tx = index % self.tiles_x;
        let ty = index / self.tiles_x;
        let x0 = tx
            .checked_mul(self.tile_edge)
            .ok_or_else(|| "SnowMud tile x origin overflows".to_string())?;
        let y0 = ty
            .checked_mul(self.tile_edge)
            .ok_or_else(|| "SnowMud tile y origin overflows".to_string())?;
        let remaining_width = self
            .width
            .checked_sub(x0)
            .ok_or_else(|| "SnowMud tile x origin exceeds grid width".to_string())?;
        let remaining_height = self
            .height
            .checked_sub(y0)
            .ok_or_else(|| "SnowMud tile y origin exceeds grid height".to_string())?;
        let w = self.tile_edge.min(remaining_width);
        let h = self.tile_edge.min(remaining_height);
        if w == 0 || h == 0 {
            return Err(format!("SnowMud tile index {index} has an empty rectangle"));
        }
        Ok((x0, y0, w, h))
    }

    /// Exact raw byte length of one tile: `w*h*4`.
    pub fn tile_raw_len(&self, index: usize) -> Result<usize, String> {
        let (_, _, w, h) = self.tile_rect(index)?;
        checked_product_usize(&[w, h, 4], "SnowMud tile byte length")
    }
}

/// Split a tightly-pitched `width*height*4` grid into base64 tile strings.
pub fn tiles_from_grid(grid: &[u8], layout: &Xmap07TileLayout) -> Result<Vec<String>, String> {
    validate_layout_shape(layout)?;
    let expected_grid_len = checked_product_usize(
        &[layout.width, layout.height, 4],
        "SnowMud grid byte length",
    )?;
    if grid.len() != expected_grid_len {
        return Err(format!(
            "SnowMud grid has {} bytes, expected {expected_grid_len}",
            grid.len()
        ));
    }
    let source_pitch = checked_product_usize(&[layout.width, 4], "SnowMud row pitch")?;
    let mut tiles = Vec::new();
    tiles
        .try_reserve_exact(layout.tile_count())
        .map_err(|error| format!("SnowMud tile array allocation refused: {error}"))?;
    for index in 0..layout.tile_count() {
        let (x0, y0, w, h) = layout.tile_rect(index)?;
        let tile_row_len = checked_product_usize(&[w, 4], "SnowMud tile row byte length")?;
        let tile_raw_len = layout.tile_raw_len(index)?;
        let mut tile = Vec::new();
        tile.try_reserve_exact(tile_raw_len)
            .map_err(|error| format!("SnowMud tile {index} allocation refused: {error}"))?;
        for dy in 0..h {
            let source_y = y0
                .checked_add(dy)
                .ok_or_else(|| "SnowMud source row overflows".to_string())?;
            let row_start = usize::try_from(source_y)
                .ok()
                .and_then(|value| value.checked_mul(source_pitch))
                .ok_or_else(|| "SnowMud source row offset overflows".to_string())?;
            let x_byte = usize::try_from(x0)
                .ok()
                .and_then(|value| value.checked_mul(4))
                .ok_or_else(|| "SnowMud source column offset overflows".to_string())?;
            let start = row_start
                .checked_add(x_byte)
                .ok_or_else(|| "SnowMud source tile offset overflows".to_string())?;
            let end = start
                .checked_add(tile_row_len)
                .ok_or_else(|| "SnowMud source tile end overflows".to_string())?;
            let row = grid
                .get(start..end)
                .ok_or_else(|| "SnowMud source tile exceeds the grid".to_string())?;
            tile.extend_from_slice(row);
        }
        tiles.push(base64_bytes::encode(&tile));
    }
    Ok(tiles)
}

/// Reassemble the tightly-pitched grid from its base64 tile strings.
pub fn grid_from_tiles(tiles: &[String], layout: &Xmap07TileLayout) -> Result<Vec<u8>, String> {
    validate_layout_shape(layout)?;
    if tiles.len() != layout.tile_count() {
        return Err(format!(
            "SnowMud tile count {} does not match layout count {}",
            tiles.len(),
            layout.tile_count()
        ));
    }
    for (index, encoded) in tiles.iter().enumerate() {
        let expected_encoded_len = canonical_base64_len(layout.tile_raw_len(index)?)?;
        if encoded.len() != expected_encoded_len {
            return Err(format!(
                "SnowMud tile {index} has {} base64 bytes, expected {expected_encoded_len}",
                encoded.len()
            ));
        }
    }
    let grid_len = checked_product_usize(
        &[layout.width, layout.height, 4],
        "SnowMud grid byte length",
    )?;
    let destination_pitch = checked_product_usize(&[layout.width, 4], "SnowMud row pitch")?;
    let mut grid = Vec::new();
    grid.try_reserve_exact(grid_len)
        .map_err(|error| format!("SnowMud grid allocation refused: {error}"))?;
    grid.resize(grid_len, 0);
    for (index, encoded) in tiles.iter().enumerate() {
        let tile = base64_bytes::decode(encoded)
            .map_err(|error| format!("SnowMud tile {index} is not valid base64: {error}"))?;
        if base64_bytes::encode(&tile) != *encoded {
            return Err(format!("SnowMud tile {index} is not canonical base64"));
        }
        let expected_tile_len = layout.tile_raw_len(index)?;
        if tile.len() != expected_tile_len {
            return Err(format!(
                "SnowMud tile {index} has {} raw bytes, expected {expected_tile_len}",
                tile.len()
            ));
        }
        let (x0, y0, w, h) = layout.tile_rect(index)?;
        let tile_row_len = checked_product_usize(&[w, 4], "SnowMud tile row byte length")?;
        for dy in 0..h {
            let tile_start = usize::try_from(dy)
                .ok()
                .and_then(|value| value.checked_mul(tile_row_len))
                .ok_or_else(|| "SnowMud tile row offset overflows".to_string())?;
            let tile_end = tile_start
                .checked_add(tile_row_len)
                .ok_or_else(|| "SnowMud tile row end overflows".to_string())?;
            let destination_y = y0
                .checked_add(dy)
                .ok_or_else(|| "SnowMud destination row overflows".to_string())?;
            let row_start = usize::try_from(destination_y)
                .ok()
                .and_then(|value| value.checked_mul(destination_pitch))
                .ok_or_else(|| "SnowMud destination row offset overflows".to_string())?;
            let x_byte = usize::try_from(x0)
                .ok()
                .and_then(|value| value.checked_mul(4))
                .ok_or_else(|| "SnowMud destination column offset overflows".to_string())?;
            let start = row_start
                .checked_add(x_byte)
                .ok_or_else(|| "SnowMud destination tile offset overflows".to_string())?;
            let end = start
                .checked_add(tile_row_len)
                .ok_or_else(|| "SnowMud destination tile end overflows".to_string())?;
            let destination = grid
                .get_mut(start..end)
                .ok_or_else(|| "SnowMud destination tile exceeds the grid".to_string())?;
            let source = tile
                .get(tile_start..tile_end)
                .ok_or_else(|| "SnowMud tile row exceeds decoded tile bytes".to_string())?;
            destination.copy_from_slice(source);
        }
    }
    Ok(grid)
}

impl Xmap07PassiveSnowMudCarrier {
    pub fn decode_json(bytes: &[u8]) -> Result<Self, String> {
        let carrier: Self = serde_json::from_slice(bytes)
            .map_err(|error| format!("invalid passive SnowMud JSON: {error}"))?;
        carrier.validate()?;
        Ok(carrier)
    }

    pub fn decode_value(value: &serde_json::Value) -> Result<Self, String> {
        let carrier: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid passive SnowMud value: {error}"))?;
        carrier.validate()?;
        Ok(carrier)
    }

    pub fn encode_json(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| error.to_string())
    }

    pub fn payload_blob_sha256(tiles: &[String]) -> String {
        let mut digest = Sha256::new();
        for (index, tile) in tiles.iter().enumerate() {
            if index != 0 {
                digest.update(b"\n");
            }
            digest.update(tile.as_bytes());
        }
        format!("{:x}", digest.finalize())
    }

    pub fn layout(&self) -> Result<Xmap07TileLayout, String> {
        Xmap07TileLayout::new(self.width, self.height, self.tile_edge)
    }

    /// The reassembled tightly-pitched grid. This is what consumers use.
    pub fn grid_bytes(&self) -> Result<Vec<u8>, String> {
        let layout = self.layout()?;
        grid_from_tiles(&self.tiles, &layout)
    }

    pub fn authority_digest(&self) -> String {
        let mut preimage = Vec::new();
        append(&mut preimage, &self.schema);
        append(&mut preimage, &self.scope);
        append(&mut preimage, &self.executable_sha256);
        preimage.extend_from_slice(&self.texture_producer_rva.to_le_bytes());
        append(&mut preimage, &self.texture_producer_instructions_sha256);
        preimage.extend_from_slice(&self.fade_updater_rva.to_le_bytes());
        append(&mut preimage, &self.fade_updater_instructions_sha256);
        preimage.extend_from_slice(&self.scene_copy_rva.to_le_bytes());
        append(&mut preimage, &self.scene_copy_instructions_sha256);
        append(&mut preimage, &self.process_epoch);
        append(&mut preimage, &self.campaign_epoch);
        append(&mut preimage, &self.hook_epoch);
        for value in [
            self.reset_epoch_ordinal,
            self.capture_frontier_ordinal,
            self.texture_observation_ordinal,
            self.fade_observation_ordinal,
        ] {
            preimage.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            self.reset_hour,
            self.initial_snapshot_hour,
            self.payload_origin_hour,
            self.texture_observation_hour,
            self.fade_observation_hour,
            self.displayed_hour,
        ] {
            preimage.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            self.carry_forward,
            self.reset_complete,
            self.initial_snapshot_complete,
            self.texture_complete,
            self.fade_complete,
        ] {
            preimage.push(value as u8);
        }
        preimage.extend_from_slice(&self.width.to_le_bytes());
        preimage.extend_from_slice(&self.height.to_le_bytes());
        preimage.extend_from_slice(&self.row_pitch.to_le_bytes());
        preimage.extend_from_slice(&self.bytes_per_texel.to_le_bytes());
        preimage.extend_from_slice(&self.tile_edge.to_le_bytes());
        append(&mut preimage, &self.format);
        append(&mut preimage, &self.payload_sha256);
        append(&mut preimage, &self.payload_blob_sha256);
        preimage.extend_from_slice(&self.fade_f32_bits.to_le_bytes());
        sha256(&preimage)
    }

    pub fn validate_for_hour(&self, displayed_hour: i64) -> Result<(), String> {
        self.validate()?;
        if self.displayed_hour != displayed_hour {
            return Err(format!(
                "passive SnowMud displayed hour {} does not match WorldFrame hour {displayed_hour}",
                self.displayed_hour
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != XMAP07_PASSIVE_SNOW_MUD_SCHEMA
            || self.scope != XMAP07_PASSIVE_SNOW_MUD_SCOPE
            || self.executable_sha256 != A729_EXECUTABLE_SHA256
            || self.texture_producer_rva != SNOW_MUD_PRODUCER_FUNCTION_RVA
            || self.texture_producer_instructions_sha256 != SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256
            || self.fade_updater_rva != XMAP07_SNOW_MUD_FADE_UPDATER_RVA
            || self.fade_updater_instructions_sha256
                != XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256
            || self.scene_copy_rva != XMAP07_SNOW_MUD_SCENE_COPY_RVA
            || self.scene_copy_instructions_sha256 != XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256
        {
            return Err("passive SnowMud producer authority is not exact a729".into());
        }
        for (label, epoch) in [
            ("process", self.process_epoch.as_str()),
            ("campaign", self.campaign_epoch.as_str()),
            ("hook", self.hook_epoch.as_str()),
        ] {
            if epoch.is_empty()
                || epoch.len() > 128
                || !epoch.bytes().all(|byte| byte.is_ascii_graphic())
            {
                return Err(format!("passive SnowMud {label} epoch is invalid"));
            }
        }
        if self.reset_epoch_ordinal == 0
            || self.capture_frontier_ordinal == 0
            || self.texture_observation_ordinal == 0
            || self.fade_observation_ordinal == 0
            || self.texture_observation_ordinal > self.capture_frontier_ordinal
            || self.fade_observation_ordinal > self.capture_frontier_ordinal
        {
            return Err("passive SnowMud frontier/observation authority is invalid".into());
        }
        if !self.reset_complete
            || !self.initial_snapshot_complete
            || !self.texture_complete
            || !self.fade_complete
            || self.reset_hour > self.initial_snapshot_hour
            || self.initial_snapshot_hour > self.payload_origin_hour
            || self.payload_origin_hour > self.displayed_hour
            || self.texture_observation_hour != self.payload_origin_hour
            || self.fade_observation_hour != self.displayed_hour
            || (self.carry_forward && self.payload_origin_hour >= self.displayed_hour)
            || (!self.carry_forward && self.payload_origin_hour != self.displayed_hour)
        {
            return Err("passive SnowMud reset/hour/carry-forward completeness is invalid".into());
        }
        let expected_pitch = self
            .width
            .checked_mul(4)
            .ok_or_else(|| "passive SnowMud row pitch overflows".to_string())?;
        let expected_len = usize::try_from(expected_pitch)
            .ok()
            .and_then(|pitch| {
                usize::try_from(self.height)
                    .ok()
                    .and_then(|height| pitch.checked_mul(height))
            })
            .ok_or_else(|| "passive SnowMud payload length overflows".to_string())?;
        if self.width == 0
            || self.height == 0
            || self.bytes_per_texel != 4
            || self.row_pitch != expected_pitch
            || self.format != XMAP07_SNOW_MUD_FORMAT
        {
            return Err("passive SnowMud texture payload is incomplete or inauthentic".into());
        }
        if self.tile_edge != XMAP07_SNOW_MUD_TILE_EDGE {
            return Err("passive SnowMud tile edge is not the pinned size-tuning value".into());
        }
        let layout = self
            .layout()
            .map_err(|error| format!("passive SnowMud texture tile layout is invalid: {error}"))?;
        if self.tiles.len() != layout.tile_count() {
            return Err("passive SnowMud texture payload is incomplete or inauthentic".into());
        }
        let grid = grid_from_tiles(&self.tiles, &layout).map_err(|error| {
            format!("passive SnowMud texture payload is incomplete or inauthentic: {error}")
        })?;
        if grid.len() != expected_len
            || self.payload_sha256 != sha256(&grid)
            || self.payload_blob_sha256 != Self::payload_blob_sha256(&self.tiles)
        {
            return Err("passive SnowMud texture payload is incomplete or inauthentic".into());
        }
        let fade = f32::from_bits(self.fade_f32_bits);
        if !fade.is_finite() || !(0.0..=1.0).contains(&fade) {
            return Err("passive SnowMud fade is not a finite unit interval value".into());
        }
        if self.authority_sha256 != self.authority_digest() {
            return Err("passive SnowMud authority digest mismatch".into());
        }
        Ok(())
    }
}

fn checked_product_usize(values: &[u32], label: &str) -> Result<usize, String> {
    let product = values.iter().try_fold(1u64, |product, value| {
        product.checked_mul(u64::from(*value))
    });
    product
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| format!("{label} overflows"))
}

fn canonical_base64_len(raw_len: usize) -> Result<usize, String> {
    raw_len
        .checked_add(2)
        .and_then(|value| value.checked_div(3))
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| "SnowMud tile base64 length overflows".to_string())
}

fn validate_layout_shape(layout: &Xmap07TileLayout) -> Result<(), String> {
    let canonical = Xmap07TileLayout::new(layout.width, layout.height, layout.tile_edge)?;
    if canonical != *layout {
        return Err("SnowMud tile layout counts do not match its dimensions".into());
    }
    Ok(())
}

fn append(preimage: &mut Vec<u8>, value: &str) {
    preimage.extend_from_slice(&(value.len() as u64).to_le_bytes());
    preimage.extend_from_slice(value.as_bytes());
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned_grid(width: u32, height: u32) -> Vec<u8> {
        let mut grid = Vec::with_capacity(
            checked_product_usize(&[width, height, 4], "test grid byte length").unwrap(),
        );
        for y in 0..height {
            for x in 0..width {
                grid.push(u8::try_from(x % 251).unwrap());
                grid.push(u8::try_from(y % 251).unwrap());
                grid.push(u8::try_from((x ^ y) % 251).unwrap());
                grid.push(
                    u8::try_from((u64::from(y) * u64::from(width) + u64::from(x)) % 251).unwrap(),
                );
            }
        }
        grid
    }

    fn carrier() -> Xmap07PassiveSnowMudCarrier {
        let bytes = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let layout = Xmap07TileLayout::new(2, 1, XMAP07_SNOW_MUD_TILE_EDGE).unwrap();
        let tiles = tiles_from_grid(&bytes, &layout).unwrap();
        let mut value = Xmap07PassiveSnowMudCarrier {
            schema: XMAP07_PASSIVE_SNOW_MUD_SCHEMA.into(),
            scope: XMAP07_PASSIVE_SNOW_MUD_SCOPE.into(),
            executable_sha256: A729_EXECUTABLE_SHA256.into(),
            texture_producer_rva: SNOW_MUD_PRODUCER_FUNCTION_RVA,
            texture_producer_instructions_sha256: SNOW_MUD_PRODUCER_INSTRUCTIONS_SHA256.into(),
            fade_updater_rva: XMAP07_SNOW_MUD_FADE_UPDATER_RVA,
            fade_updater_instructions_sha256: XMAP07_SNOW_MUD_FADE_UPDATER_INSTRUCTIONS_SHA256
                .into(),
            scene_copy_rva: XMAP07_SNOW_MUD_SCENE_COPY_RVA,
            scene_copy_instructions_sha256: XMAP07_SNOW_MUD_SCENE_COPY_INSTRUCTIONS_SHA256.into(),
            process_epoch: "process-1".into(),
            campaign_epoch: "campaign-1".into(),
            hook_epoch: "hook-1".into(),
            reset_epoch_ordinal: 1,
            capture_frontier_ordinal: 13,
            texture_observation_ordinal: 11,
            fade_observation_ordinal: 13,
            reset_hour: 10,
            initial_snapshot_hour: 10,
            payload_origin_hour: 11,
            texture_observation_hour: 11,
            fade_observation_hour: 12,
            displayed_hour: 12,
            carry_forward: true,
            reset_complete: true,
            initial_snapshot_complete: true,
            texture_complete: true,
            fade_complete: true,
            width: 2,
            height: 1,
            row_pitch: 8,
            bytes_per_texel: 4,
            format: XMAP07_SNOW_MUD_FORMAT.into(),
            tile_edge: XMAP07_SNOW_MUD_TILE_EDGE,
            tiles: tiles.clone(),
            payload_sha256: sha256(&bytes),
            payload_blob_sha256: Xmap07PassiveSnowMudCarrier::payload_blob_sha256(&tiles),
            fade_f32_bits: 0.5f32.to_bits(),
            authority_sha256: String::new(),
        };
        value.authority_sha256 = value.authority_digest();
        value
    }

    fn refresh_blob_and_authority(carrier: &mut Xmap07PassiveSnowMudCarrier) {
        carrier.payload_blob_sha256 =
            Xmap07PassiveSnowMudCarrier::payload_blob_sha256(&carrier.tiles);
        carrier.authority_sha256 = carrier.authority_digest();
    }

    #[test]
    fn tiled_grids_round_trip_exactly_for_ragged_and_vanilla_shapes() {
        // kills: transposing tile coordinates, padding edge tiles, or changing row-major order.
        for (width, height) in [(37, 21), (1_408, 512)] {
            let grid = patterned_grid(width, height);
            let layout = Xmap07TileLayout::new(width, height, XMAP07_SNOW_MUD_TILE_EDGE).unwrap();
            let tiles = tiles_from_grid(&grid, &layout).unwrap();
            assert_eq!(tiles.len(), layout.tile_count());
            assert_eq!(grid_from_tiles(&tiles, &layout).unwrap(), grid);
        }

        let ragged = Xmap07TileLayout::new(37, 21, XMAP07_SNOW_MUD_TILE_EDGE).unwrap();
        assert_eq!(
            ragged.tile_rect(ragged.tile_count() - 1).unwrap(),
            (32, 16, 5, 5)
        );
        assert_eq!(ragged.tile_raw_len(ragged.tile_count() - 1).unwrap(), 100);
    }

    #[test]
    fn ragged_tile_order_and_packing_match_an_independent_fixture() {
        // kills: applying the same wrong tile permutation in both conversion directions.
        let grid = (0u8..36).collect::<Vec<_>>();
        let layout = Xmap07TileLayout::new(3, 3, 2).unwrap();
        assert_eq!(
            tiles_from_grid(&grid, &layout).unwrap(),
            [
                "AAECAwQFBgcMDQ4PEBESEw==",
                "CAkKCxQVFhc=",
                "GBkaGxwdHh8=",
                "ICEiIw==",
            ]
        );
        assert_eq!(
            grid_from_tiles(
                &[
                    "AAECAwQFBgcMDQ4PEBESEw==".into(),
                    "CAkKCxQVFhc=".into(),
                    "GBkaGxwdHh8=".into(),
                    "ICEiIw==".into(),
                ],
                &layout,
            )
            .unwrap(),
            grid
        );
    }

    #[test]
    fn tile_layout_rejects_empty_overflowing_and_inconsistent_shapes() {
        // kills: unchecked ceil/count products or trusting caller-edited public layout fields.
        assert!(Xmap07TileLayout::new(0, 1, 16).is_err());
        assert!(Xmap07TileLayout::new(1, 0, 16).is_err());
        assert!(Xmap07TileLayout::new(1, 1, 0).is_err());
        assert!(Xmap07TileLayout::new(u32::MAX, 1, 1).is_err());
        assert!(Xmap07TileLayout::new(
            XMAP07_SNOW_MUD_MAX_GRID_AXIS,
            XMAP07_SNOW_MUD_MAX_GRID_AXIS,
            16,
        )
        .is_err());

        let grid = patterned_grid(2, 1);
        let mut inconsistent = Xmap07TileLayout::new(2, 1, 16).unwrap();
        inconsistent.tiles_x = 2;
        assert!(tiles_from_grid(&grid, &inconsistent).is_err());
        assert!(grid_from_tiles(&[], &inconsistent).is_err());
    }

    #[test]
    fn wrong_tile_raw_length_is_rejected_by_carrier_validation() {
        // kills: accepting decoded tile bytes without checking the layout's exact raw length.
        let mut value = carrier();
        // Seven and eight raw bytes both occupy twelve base64 bytes, so this reaches the decoded
        // raw-length check instead of being caught by the pre-decode encoded-length bound.
        value.tiles[0] = base64_bytes::encode(&[1, 2, 3, 4, 5, 6, 7]);
        refresh_blob_and_authority(&mut value);
        assert!(value.validate().is_err());
    }

    #[test]
    fn invalid_base64_tile_is_rejected() {
        // kills: treating tile strings as opaque authenticated text instead of decoding them.
        let mut value = carrier();
        value.tiles[0] = "!!!!!!!!!!!!".into();
        refresh_blob_and_authority(&mut value);
        assert!(value.validate().is_err());
    }

    #[test]
    fn wrong_encoded_tile_lengths_are_rejected_before_decode() {
        // kills: decoding attacker-sized tile strings before enforcing the layout's exact
        // canonical-base64 length.
        let value = carrier();
        let mut too_short = value.clone();
        too_short.tiles[0].pop();
        refresh_blob_and_authority(&mut too_short);
        assert!(too_short
            .validate()
            .unwrap_err()
            .contains("base64 bytes, expected"));

        let mut too_long = value;
        too_long.tiles[0].push('A');
        refresh_blob_and_authority(&mut too_long);
        assert!(too_long
            .validate()
            .unwrap_err()
            .contains("base64 bytes, expected"));
    }

    #[test]
    fn noncanonical_base64_tail_bits_are_rejected() {
        // kills: accepting a second valid-looking base64 spelling of the same tile bytes.
        let mut value = carrier();
        assert_eq!(value.tiles[0], "AQIDBAUGBwg=");
        value.tiles[0] = "AQIDBAUGBwh=".into();
        refresh_blob_and_authority(&mut value);
        assert!(value.validate().is_err());
    }

    #[test]
    fn wrong_tile_count_is_rejected_in_both_directions() {
        // kills: accepting a prefix or suffix beyond the fixed layout tile count.
        let value = carrier();
        let mut too_few = value.clone();
        too_few.tiles.clear();
        refresh_blob_and_authority(&mut too_few);
        assert!(too_few.validate().is_err());

        let mut too_many = value;
        too_many.tiles.push(base64_bytes::encode(&[0; 8]));
        refresh_blob_and_authority(&mut too_many);
        assert!(too_many.validate().is_err());
    }

    #[test]
    fn unpinned_tile_edge_is_rejected() {
        // kills: allowing multiple serialised tile shapes for the same grid dimensions.
        let mut value = carrier();
        value.tile_edge = XMAP07_SNOW_MUD_TILE_EDGE / 2;
        refresh_blob_and_authority(&mut value);
        assert!(value.validate().is_err());
    }

    #[test]
    fn changed_tile_bytes_are_caught_by_the_grid_payload_digest() {
        // kills: authenticating only the serialised tile strings and not the reassembled grid.
        let mut value = carrier();
        let mut tile = base64_bytes::decode(&value.tiles[0]).unwrap();
        tile[3] ^= 0x80;
        value.tiles[0] = base64_bytes::encode(&tile);
        refresh_blob_and_authority(&mut value);
        assert!(value.validate().is_err());
    }

    #[test]
    fn tile_edge_is_part_of_the_authority_digest() {
        // kills: omitting or reordering tile_edge in the authority digest preimage.
        let value = carrier();
        let original = value.authority_digest();
        let mut changed = value;
        changed.tile_edge += 1;
        assert_ne!(changed.authority_digest(), original);
        assert_eq!(
            original,
            "8f969821e62d3c41e5e60e2f80ac7600b2f2298cc0106f0e9c1cdda8f7b2f012"
        );
    }

    #[test]
    fn payload_blob_digest_is_over_newline_joined_tile_strings() {
        // kills: hashing concatenated tile text without the specified unambiguous separator.
        let tiles = vec!["YQ==".to_string(), "YmM=".to_string()];
        assert_eq!(
            Xmap07PassiveSnowMudCarrier::payload_blob_sha256(&tiles),
            sha256(b"YQ==\nYmM=")
        );
    }

    #[test]
    fn wrong_serialised_tile_digest_is_rejected() {
        // kills: omitting the payload_blob_sha256 comparison after authenticating raw grid bytes.
        let mut value = carrier();
        value.payload_blob_sha256 = "0".repeat(64);
        value.authority_sha256 = value.authority_digest();
        assert!(value.validate().is_err());
    }

    #[test]
    fn wrong_authority_digest_is_rejected() {
        // kills: omitting the final authority_sha256 comparison after all payload checks pass.
        let mut value = carrier();
        value.authority_sha256 = "0".repeat(64);
        assert!(value.validate().is_err());
    }

    #[test]
    fn exact_carrier_round_trips_and_rejects_authority_mutations() {
        let value = carrier();
        let encoded = value.encode_json().unwrap();
        assert_eq!(
            Xmap07PassiveSnowMudCarrier::decode_json(&encoded).unwrap(),
            value
        );

        let mut bad = value.clone();
        bad.capture_frontier_ordinal = 1;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.reset_complete = false;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.fade_f32_bits = f32::NAN.to_bits();
        assert!(bad.validate().is_err());
        let mut bad = value;
        bad.width = 3;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn fresh_and_carried_hours_are_explicit_and_unknown_fields_fail_closed() {
        let mut fresh = carrier();
        fresh.payload_origin_hour = fresh.displayed_hour;
        fresh.texture_observation_hour = fresh.displayed_hour;
        fresh.carry_forward = false;
        fresh.authority_sha256 = fresh.authority_digest();
        fresh.validate().unwrap();

        let mut raw: serde_json::Value =
            serde_json::from_slice(&fresh.encode_json().unwrap()).unwrap();
        raw.as_object_mut().unwrap().insert(
            "selected_present_sha256".into(),
            serde_json::Value::String("forbidden".into()),
        );
        assert!(Xmap07PassiveSnowMudCarrier::decode_value(&raw).is_err());
    }
}
