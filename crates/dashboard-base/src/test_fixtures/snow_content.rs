use std::{fs, path::Path};

pub const DEFINES: &str = "NDefines={NGraphics={POSTEFFECT_PER_PROVINCE_MIN_SNOW=0.15,POSTEFFECT_PER_PROVINCE_MAX_SNOW=0.8,POSTEFFECT_TOTAL_MIN_SNOW=0.0,POSTEFFECT_TOTAL_MAX_SNOW=1.0}}";
pub const TERRAIN: &str = "categories={plain={}} terrain={plain={type=plain color={0} perm_snow=no} ice={type=plain color={1} perm_snow=yes}}";
pub const VOLUMES: &str = "posteffect_values={name=plain} posteffect_values={name=winter} posteffect_volumes={posteffect_volume={posteffect_values_day=plain posteffect_values_night=plain posteffect_values_day_winter=winter posteffect_values_night_winter=winter position={6 0 4} size={12 1 8}}}";

pub fn write(root: &Path) {
    for directory in [
        "map/strategicregions",
        "common/terrain",
        "common/defines",
        "gfx",
    ] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    for (path, text) in [
        (
            "map/default.map",
            "definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=unused.bmp heightmap=unused.bmp tree_definition=unused.bmp",
        ),
        (
            "map/definition.csv",
            "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plain;1\n2;2;0;0;land;false;plain;1\n3;3;0;0;land;false;plain;1\n",
        ),
        (
            "map/strategicregions/snow.txt",
            "strategic_region={id=1 provinces={1 2}} strategic_region={id=2 provinces={3}}",
        ),
        ("common/terrain/snow.txt", TERRAIN),
        ("common/defines/snow.lua", DEFINES),
        ("gfx/posteffect_volumes.txt", VOLUMES),
    ] {
        fs::write(root.join(path), text).unwrap();
    }
    fs::write(
        root.join("map/provinces.bmp"),
        bitmap(24, |x, y| 1 + (x + y) % 3),
    )
    .unwrap();
    fs::write(
        root.join("map/terrain.bmp"),
        bitmap(8, |x, y| u8::from((x + y) % 7 == 0)),
    )
    .unwrap();
}
fn bitmap(bits: u16, pixel: impl Fn(u8, u8) -> u8) -> Vec<u8> {
    let stride = (12 * usize::from(bits) / 8).div_ceil(4) * 4;
    let offset = 54 + if bits == 8 { 256 * 4 } else { 0 };
    let mut bytes = vec![0; offset + stride * 8];
    let length = bytes.len() as u32;
    bytes[..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&length.to_le_bytes());
    bytes[10..14].copy_from_slice(&(offset as u32).to_le_bytes());
    bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&12i32.to_le_bytes());
    bytes[22..26].copy_from_slice(&8i32.to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&bits.to_le_bytes());
    for y in 0..8u8 {
        for x in 0..12u8 {
            let at = offset + usize::from(y) * stride + usize::from(x) * usize::from(bits / 8);
            if bits == 24 {
                bytes[at + 2] = pixel(x, y);
            } else {
                bytes[at] = pixel(x, y);
            }
        }
    }
    bytes
}
