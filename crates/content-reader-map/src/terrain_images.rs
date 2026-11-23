use content_reader_layers::textures;
use content_reader_image::{
    IndexedImage, SharedRgba8Image as Rgba8Image, bmp,
    dds::{Texture, TextureFormat},
    native::Decode,
};
use content_reader_key_names::terrain::DefaultMap;
use content_reader_layers::ResolvedLayers;
use content_reader_parser::fields::invalid;
use std::io;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapImages {
    pub terrain: IndexedImage,
    pub heightmap: IndexedImage,
    pub rivers: IndexedImage,
    pub trees: IndexedImage,
    pub world_normal: Rgba8Image,
}

pub fn read_map_images(
    layers: &ResolvedLayers,
    map: &DefaultMap,
    decode: Decode,
) -> io::Result<MapImages> {
    let indexed = |name: &str| -> io::Result<IndexedImage> {
        let path = format!("map/{name}");
        read_indexed_image(layers, &path)
    };
    Ok(MapImages {
        terrain: indexed(&map.terrain)?,
        heightmap: indexed(&map.heightmap)?,
        rivers: indexed(&map.rivers)?,
        trees: indexed(&map.tree_definition)?,
        world_normal: read_world_normal(layers, decode)?,
    })
}

pub fn read_indexed_image(layers: &ResolvedLayers, path: &str) -> io::Result<IndexedImage> {
    layers
        .recorded_product("palette bitmap decoding", 1, path.as_bytes(), |layers| {
            bmp::decode_indexed(&layers.read(path)?)
                .map_err(|error| invalid(format!("{path}: {error}")))
        })
        .map(|image| image.as_ref().clone())
}

pub fn read_world_normal(layers: &ResolvedLayers, decode: Decode) -> io::Result<Rgba8Image> {
    world_normal(&|path| layers.read_uncovered_path(path), decode)
}

fn world_normal<
    B: AsRef<[u8]> + std::ops::Deref<Target = [u8]> + Into<content_reader_image::Bytes>,
>(
    read: &dyn Fn(&[u8]) -> io::Result<B>,
    decode: Decode,
) -> io::Result<Rgba8Image> {
    let texture = textures::texture(b"map/world_normal.bmp", read, decode)?
        .map_err(|why| invalid(format!("map/world_normal.bmp {why}")))?;
    match texture.image {
        Texture {
            format: TextureFormat::Rgba8,
            faces: 1,
            width,
            height,
            mut mips,
        } if mips.len() == 1 && mips[0].len() == 1 => Ok(Rgba8Image {
            width,
            height,
            pixels: mips.swap_remove(0).swap_remove(0).data,
        }),
        other => Err(invalid(format!(
            "{} is a {:?} texture of {} faces and {} levels, which the map's HeightNormal upload \
             does not take",
            texture.path,
            other.format,
            other.faces,
            other.mips.first().map_or(0, Vec::len)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bitmap(width: i32, rows: &[&[u8]], dib: u32) -> Vec<u8> {
        let offset = 14 + dib;
        let pixels: Vec<u8> = rows.concat();
        let mut bytes = b"BM".to_vec();
        bytes.extend((offset + pixels.len() as u32).to_le_bytes());
        bytes.extend([0; 4]);
        bytes.extend(offset.to_le_bytes());
        bytes.extend(dib.to_le_bytes());
        bytes.extend(width.to_le_bytes());
        bytes.extend((rows.len() as i32).to_le_bytes());
        bytes.extend(1_u16.to_le_bytes());
        let (bits, compression) = if dib == 40 { (24_u16, 0_u32) } else { (32, 3) };
        bytes.extend(bits.to_le_bytes());
        bytes.extend(compression.to_le_bytes());
        bytes.extend((pixels.len() as u32).to_le_bytes());
        bytes.extend([0; 16]);
        if dib == 124 {
            for mask in [0x00FF_0000_u32, 0xFF00, 0xFF, 0xFF00_0000] {
                bytes.extend(mask.to_le_bytes());
            }
        }
        bytes.resize(offset as usize, 0);
        bytes.extend(pixels);
        bytes
    }

    #[test]
    fn the_world_normal_keeps_the_clients_texture_padding() {
        content_reader_script::test_game::enter();
        let bytes = bitmap(
            2,
            &[
                &[1, 2, 3, 4, 5, 6, 0, 0],
                &[7, 8, 9, 10, 11, 12, 0, 0],
                &[13, 14, 15, 16, 17, 18, 0, 0],
            ],
            40,
        );
        let read = |path: &[u8]| match path {
            b"map/world_normal.bmp" => Ok(bytes.clone()),
            _ => Err(io::Error::from(io::ErrorKind::NotFound)),
        };
        let raw = bmp::decode_rgb(&bytes).unwrap();
        assert_eq!(world_normal(&read, Decode::default()).unwrap(), raw);
        let padded = world_normal(
            &read,
            Decode {
                force_pow2_textures: true,
            },
        )
        .unwrap();
        assert_eq!([padded.width, padded.height], [2, 4]);
        assert_eq!(padded.pixels[..raw.pixels.len()], raw.pixels);
        assert_eq!(
            padded.pixels[raw.pixels.len()..],
            [0, 0, 0, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn the_world_normal_loads_as_the_game_decodes_a_texture() {
        content_reader_script::test_game::enter();
        let shipped = bitmap(
            2,
            &[&[1, 2, 3, 4, 5, 6, 0, 0], &[7, 8, 9, 10, 11, 12, 0, 0]],
            40,
        );
        let expected = bmp::decode_rgb(&shipped).unwrap();
        let read = |bytes: Vec<u8>| {
            move |path: &[u8]| match path {
                b"map/world_normal.bmp" => Ok(bytes.clone()),
                _ => Err(io::Error::from(io::ErrorKind::NotFound)),
            }
        };
        assert_eq!(
            world_normal(&read(shipped), Decode::default()).unwrap(),
            expected
        );
        let bitfields = bitmap(
            2,
            &[
                &[1, 2, 3, 255, 4, 5, 6, 255],
                &[7, 8, 9, 255, 10, 11, 12, 255],
            ],
            124,
        );
        assert_eq!(
            world_normal(&read(bitfields), Decode::default()).unwrap(),
            expected
        );
        let missing = world_normal::<Vec<u8>>(
            &|_: &[u8]| Err(io::Error::from(io::ErrorKind::NotFound)),
            Decode::default(),
        );
        assert!(missing.unwrap_err().to_string().contains("no layer holds"));
        let undecodable = world_normal(&read(b"not an image".to_vec()), Decode::default());
        assert!(
            undecodable
                .unwrap_err()
                .to_string()
                .contains("does not load")
        );
    }
}
