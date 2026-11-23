
use std::io;

use content_reader_defines::defines::Defines;
use content_reader_image::Rgba8Image;

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlagSet {
    Normal,
    Medium,
    Small,
}

impl FlagSet {
    pub fn directory(self) -> &'static str {
        match self {
            Self::Normal => "gfx/flags/",
            Self::Medium => "gfx/flags/medium/",
            Self::Small => "gfx/flags/small/",
        }
    }

    pub fn cell(self, defines: &Defines) -> io::Result<[u32; 2]> {
        let prefix = match self {
            Self::Normal => "COUNTRY_FLAG_TEX",
            Self::Medium => "COUNTRY_FLAG_MEDIUM_TEX",
            Self::Small => "COUNTRY_FLAG_SMALL_TEX",
        };
        let size = |suffix: &str| {
            let name = format!("{prefix}_{suffix}");
            u32::try_from(defines.int32("NGraphics", &name)?)
                .ok()
                .filter(|&size| size > 0)
                .ok_or_else(|| invalid(format!("NDefines.NGraphics.{name} is not a positive size")))
        };
        let (cell, page) = ([size("WIDTH")?, size("HEIGHT")?], size("MAX_SIZE")?);
        if cell.iter().any(|&side| side > page) {
            return Err(invalid(format!(
                "NDefines.NGraphics.{prefix}_WIDTH/HEIGHT {} x {} does not fit its {page} MAX_SIZE page",
                cell[0], cell[1]
            )));
        }
        Ok(cell)
    }
}

pub fn decode(bytes: &[u8], cell: [u32; 2]) -> Option<Rgba8Image> {
    decode_checked(bytes, cell).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeFailure {
    EngineRefused,
    Truncated,
}

pub fn decode_checked(bytes: &[u8], cell: [u32; 2]) -> Result<Rgba8Image, DecodeFailure> {
    let head = bytes.get(..18).ok_or(DecodeFailure::Truncated)?;
    let size = [[head[12], head[13]], [head[14], head[15]]].map(u16::from_le_bytes);
    if head[1] != 0 || head[2] != 2 || size.map(u32::from) != cell || size.contains(&0) {
        return Err(DecodeFailure::EngineRefused);
    }
    let unit = match head[16] >> 3 {
        unit @ (3 | 4) => usize::from(unit),
        _ => return Err(DecodeFailure::EngineRefused),
    };
    let start = match head[0] as i8 {
        0 => 18,
        id => usize::try_from(i32::from(id) + 20).map_err(|_| DecodeFailure::EngineRefused)?,
    };
    let [width, height] = size.map(usize::from);
    let stored = bytes
        .get(start..start + width * height * unit)
        .ok_or(DecodeFailure::Truncated)?;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for row in stored.chunks_exact(width * unit).rev() {
        for pixel in row.chunks_exact(unit) {
            let alpha = if unit == 4 { pixel[3] } else { 255 };
            pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
        }
    }
    Ok(Rgba8Image {
        width: cell[0],
        height: cell[1],
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targa(id: &[u8], image_type: u8, depth: u8, descriptor: u8, body: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0; 18];
        bytes[0] = id.len() as u8;
        bytes[2] = image_type;
        bytes[12] = 1;
        bytes[14] = 2;
        bytes[16] = depth;
        bytes[17] = descriptor;
        bytes.extend_from_slice(id);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn rows_are_bottom_up_whatever_the_descriptor_says() {
        let body = [1, 2, 3, 4, 5, 6, 7, 8];
        for descriptor in [0x08, 0x28, 0x38] {
            let image = decode(&targa(&[], 2, 32, descriptor, &body), [1, 2]).unwrap();
            assert_eq!(image.pixels, [7, 6, 5, 8, 3, 2, 1, 4]);
        }
        let image = decode(&targa(&[], 2, 24, 0x20, &body[..6]), [1, 2]).unwrap();
        assert_eq!(image.pixels, [6, 5, 4, 255, 3, 2, 1, 255]);
        let image = decode(
            &targa(b"ID", 2, 32, 0, &[[0, 0].as_slice(), &body].concat()),
            [1, 2],
        );
        assert_eq!(image.unwrap().pixels, [7, 6, 5, 8, 3, 2, 1, 4]);
        let mut header_seek = targa(&[], 2, 32, 0, &[[0].as_slice(), &body].concat());
        header_seek[0] = 255;
        let image = decode(&header_seek, [1, 2]).unwrap();
        assert_eq!(image.pixels, [7, 6, 5, 8, 3, 2, 1, 4]);
        header_seek[0] = 200;
        assert!(decode(&header_seek, [1, 2]).is_none());
    }

    #[test]
    fn other_types_depths_sizes_and_short_files_move_to_the_next_candidate() {
        let body = [0; 8];
        assert!(decode(&targa(&[], 10, 32, 0, &body), [1, 2]).is_none());
        assert!(decode(&targa(&[], 2, 16, 0, &body), [1, 2]).is_none());
        assert!(decode(&targa(&[], 2, 32, 0, &body), [2, 1]).is_none());
        assert!(decode(&targa(&[], 2, 32, 0, &body[..7]), [1, 2]).is_none());
        assert!(matches!(
            decode_checked(&targa(&[], 10, 32, 0, &body), [1, 2]),
            Err(DecodeFailure::EngineRefused)
        ));
        assert!(matches!(
            decode_checked(&targa(&[], 2, 32, 0, &body[..7]), [1, 2]),
            Err(DecodeFailure::Truncated)
        ));
    }

    #[test]
    fn identification_lengths_before_the_file_are_engine_header_refusals() {
        let mut bytes = targa(&[], 2, 32, 0, &[0; 8]);
        for id in 128..=235 {
            bytes[0] = id;
            assert!(matches!(
                decode_checked(&bytes, [1, 2]),
                Err(DecodeFailure::EngineRefused)
            ));
        }
        bytes[0] = 236;
        assert!(decode_checked(&bytes, [1, 2]).is_ok());
    }
}
