
use super::{Failure, Zero, oversized};
use crate::dds::{MipLevel, Texture, TextureFormat};

const HEADER: usize = 128;
const CUBE_MAP: u32 = 0x200;
const VOLUME: u32 = 0x20_0000;
const DX10_CUBE: u32 = 4;

fn code(name: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*name)
}

#[derive(Clone, Copy)]
enum Pixels {
    B5G6R5,
    A1R5G5B5,
    X1R5G5B5,
    B8G8R8,
    B8G8R8A8,
    B8G8R8X8,
    R8G8B8A8,
}

impl Pixels {
    fn zero(self) -> Zero {
        match self {
            Self::B5G6R5 | Self::B8G8R8 | Self::B8G8R8X8 => Zero::Opaque,
            Self::A1R5G5B5 | Self::X1R5G5B5 | Self::B8G8R8A8 | Self::R8G8B8A8 => Zero::Transparent,
        }
    }

    fn bytes(self) -> usize {
        match self {
            Self::B5G6R5 | Self::A1R5G5B5 | Self::X1R5G5B5 => 2,
            Self::B8G8R8 => 3,
            Self::B8G8R8A8 | Self::B8G8R8X8 | Self::R8G8B8A8 => 4,
        }
    }

    fn format(self) -> TextureFormat {
        match self {
            Self::B5G6R5 => TextureFormat::B5G6R5,
            Self::A1R5G5B5 | Self::X1R5G5B5 => TextureFormat::B5G5R5A1,
            Self::B8G8R8 | Self::B8G8R8A8 | Self::B8G8R8X8 | Self::R8G8B8A8 => TextureFormat::Rgba8,
        }
    }

    fn convert(self, source: &[u8]) -> Vec<u8> {
        if matches!(self, Self::R8G8B8A8) {
            return source.to_vec();
        }
        let mut out = Vec::with_capacity(source.len() / self.bytes() * self.format().unit().1);
        for texel in source.chunks_exact(self.bytes()) {
            match self {
                Self::B8G8R8A8 => out.extend_from_slice(&[texel[2], texel[1], texel[0], texel[3]]),
                Self::B8G8R8X8 => out.extend_from_slice(&[texel[2], texel[1], texel[0], 255]),
                Self::B8G8R8 => out.extend_from_slice(&[texel[2], texel[1], texel[0], 255]),
                Self::B5G6R5 | Self::A1R5G5B5 => out.extend_from_slice(texel),
                Self::X1R5G5B5 => out.extend_from_slice(&[texel[0], texel[1] | 0x80]),
                Self::R8G8B8A8 => out.extend_from_slice(texel),
            }
        }
        out
    }
}

enum Form {
    Blocks(TextureFormat, usize),
    Pixels(Pixels),
}

pub(super) fn load(bytes: impl Into<crate::Bytes>) -> Result<Option<(Texture, Zero)>, Failure> {
    let bytes = bytes.into();
    if bytes.len() < HEADER {
        return Err(Failure::Undefined(format!(
            "a {}-byte DDS: DDS decoding reads its 128-byte header past the end of the file",
            bytes.len()
        )));
    }
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    let (height, width, raw_mip_count) = (word(12), word(16), word(28));
    let (flags, fourcc, bits) = (word(80), word(84), word(88));
    let masks = [word(92), word(96), word(100), word(104)];
    let is_dx10 = flags & 4 != 0 && fourcc == code(b"DX10");
    let start = if is_dx10 { HEADER + 20 } else { HEADER };
    let (form, mip_count, cube) = if is_dx10 {
        if bytes.len() >= HEADER + 4 && matches!(word(128), 98 | 99) {
            return bc7(&bytes, word, word(128) == 99);
        }
        if bytes.len() < HEADER + 20 {
            return Err(Failure::Undecodable(
                "a DX10 DDS: truncated DX10 header".to_owned(),
            ));
        }
        let dxgi = word(128);
        let dimension = word(132);
        let misc = word(136);
        let array_size = word(140);
        if dimension != 3 || word(24) > 1 || word(112) & VOLUME != 0 {
            return Err(Failure::Unsupported(format!(
                "a DX10 DDS (DXGI format {dxgi}): unsupported resource dimension {dimension} or volume depth {}",
                word(24)
            )));
        }
        if array_size != 1 {
            return Err(Failure::Unsupported(format!(
                "a DX10 DDS (DXGI format {dxgi}): unsupported texture array size {array_size}"
            )));
        }
        let form = match dxgi {
            70 | 71 | 72 => Form::Blocks(TextureFormat::Bc1, 8),
            73 | 74 | 75 => Form::Blocks(TextureFormat::Bc2, 16),
            76 | 77 | 78 => Form::Blocks(TextureFormat::Bc3, 16),
            27 | 28 | 29 => Form::Pixels(Pixels::R8G8B8A8),
            87 | 90 | 91 => Form::Pixels(Pixels::B8G8R8A8),
            88 | 92 | 93 => Form::Pixels(Pixels::B8G8R8X8),
            85 => Form::Pixels(Pixels::B5G6R5),
            86 => Form::Pixels(Pixels::A1R5G5B5),
            other => {
                return Err(Failure::Unsupported(format!(
                    "a DX10 DDS with unsupported DXGI format {other}"
                )));
            }
        };
        let mips = if word(8) & 0x2_0000 != 0 {
            raw_mip_count.max(1)
        } else {
            1
        };
        let cube = word(112) & CUBE_MAP != 0 || misc & DX10_CUBE != 0;
        (form, mips, cube)
    } else {
        let form = if fourcc == code(b"DXT1") {
            Form::Blocks(TextureFormat::Bc1, 8)
        } else if fourcc == code(b"DXT2") || fourcc == code(b"DXT4") {
            return Ok(None);
        } else if fourcc == code(b"DXT3") {
            Form::Blocks(TextureFormat::Bc2, 16)
        } else if fourcc == code(b"DXT5") {
            Form::Blocks(TextureFormat::Bc3, 16)
        } else if flags & 0x20240 == 0 || bits == 32 {
            Form::Pixels(Pixels::B8G8R8A8)
        } else if bits == 24 {
            Form::Pixels(Pixels::B8G8R8)
        } else if bits == 16 {
            let rgb = flags & 0x40 != 0;
            Form::Pixels(if rgb && masks[..3] == [0xF800, 0x7E0, 0x1F] {
                Pixels::B5G6R5
            } else if flags & 0x41 == 0x41 && masks == [0x7C00, 0x3E0, 0x1F, 0x8000] {
                Pixels::A1R5G5B5
            } else if rgb && masks[..3] == [0x7C00, 0x3E0, 0x1F] {
                Pixels::X1R5G5B5
            } else {
                return Err(Failure::Undefined(format!(
                    "a 16-bit DDS with flags {flags:#x} and masks {masks:#x?}: DDS decoding \
                     loads it as format 0, uploaded as B8G8R8X8 with 0-bit rows: the texture does \
                     not hold the file's texels"
                )));
            })
        } else {
            return Err(Failure::Undefined(format!(
                "a {bits}-bit DDS with flags {flags:#x}: DDS decoding loads it as format 0x1A, \
                 uploaded as B8G8R8X8 with 0-bit rows: the texture does not hold the file's texels"
            )));
        };
        (form, raw_mip_count, word(112) & CUBE_MAP != 0)
    };
    let (block, unit) = match &form {
        Form::Blocks(_, size) => (true, *size),
        Form::Pixels(pixels) => (false, pixels.bytes()),
    };
    let (width, height) = if block {
        (width.wrapping_add(3) & !3, height.wrapping_add(3) & !3)
    } else {
        (width, height)
    };
    oversized(width, height)?;
    let level_bytes = |w: u32, h: u32| {
        let (w, h) = (w as usize, h as usize);
        if block {
            w.div_ceil(4) * h.div_ceil(4) * unit
        } else {
            w * h * unit
        }
    };
    let convert = |source: &[u8], offset: usize| match &form {
        Form::Blocks(..) => bytes.slice(offset..offset + source.len()),
        Form::Pixels(pixels) => pixels.convert(source).into(),
    };
    let (format, zero) = match &form {
        Form::Blocks(format, _) => (*format, Zero::Transparent),
        Form::Pixels(pixels) => (pixels.format(), pixels.zero()),
    };
    if cube {
        let mut offset = start;
        let mut faces = Vec::with_capacity(6);
        for face in 0..6 {
            let (mut level_width, mut level_height) = (width, height);
            let mut levels = Vec::new();
            loop {
                let size = level_bytes(level_width, level_height);
                let source = bytes.get(offset..offset + size).ok_or_else(|| {
                    Failure::Undefined(format!(
                        "a cube-map DDS whose face {face} level {} runs past the file: \
                         DDS decoding reads the six faces without checking the file's \
                         length, so texture creation uploads memory past it",
                        levels.len()
                    ))
                })?;
                levels.push(MipLevel {
                    width: level_width,
                    height: level_height,
                    data: convert(source, offset),
                });
                offset += size;
                if level_width == 1
                    || level_height == 1
                    || levels.len() as u64 >= u64::from(mip_count)
                {
                    break;
                }
                if level_width == 0 && level_height == 0 && levels.len() == 2 {
                    break;
                }
                level_width >>= 1;
                level_height >>= 1;
            }
            faces.push(levels);
        }
        return Ok(Some((
            Texture {
                format,
                width,
                height,
                faces: 6,
                mips: faces,
            },
            zero,
        )));
    }
    let (mut level_width, mut level_height) = (width, height);
    let mut offset = start;
    let mut levels = Vec::new();
    loop {
        let size = level_bytes(level_width, level_height);
        let Some(source) = bytes.get(offset..offset + size) else {
            break;
        };
        levels.push(MipLevel {
            width: level_width,
            height: level_height,
            data: convert(source, offset),
        });
        offset += size;
        if level_width == 1 || level_height == 1 || levels.len() as u64 >= u64::from(mip_count) {
            break;
        }
        if level_width == 0 && level_height == 0 && levels.len() == 2 {
            break;
        }
        level_width >>= 1;
        level_height >>= 1;
    }
    if levels.is_empty() {
        return Err(Failure::Undefined(
            "a DDS whose first level runs past the file: DDS decoding keeps no level, and \
             texture creation then creates the texture without initial data"
                .to_owned(),
        ));
    }
    Ok(Some((
        Texture {
            format,
            width,
            height,
            faces: 1,
            mips: vec![levels],
        },
        zero,
    )))
}

fn bc7(
    bytes: &crate::Bytes,
    word: impl Fn(usize) -> u32,
    srgb: bool,
) -> Result<Option<(Texture, Zero)>, Failure> {
    let refuse = |why: String| Failure::Undecodable(format!("a DX10 BC7 DDS: {why}"));
    if bytes.len() < HEADER + 20 {
        return Err(refuse("truncated DX10 header".to_owned()));
    }
    let (width, height) = (word(16), word(12));
    if width == 0 || height == 0 {
        return Err(refuse(format!("zero extent {width}x{height}")));
    }
    oversized(width, height)?;
    if word(132) != 3 || word(24) > 1 || word(112) & VOLUME != 0 {
        return Err(refuse(format!(
            "unsupported resource dimension {} or volume depth {}",
            word(132),
            word(24)
        )));
    }
    if word(112) & CUBE_MAP != 0 || word(136) & DX10_CUBE != 0 {
        return Err(refuse(
            "cube maps are not supported by the BC7 reader".to_owned(),
        ));
    }
    if word(140) != 1 {
        return Err(refuse(format!(
            "unsupported texture array size {}",
            word(140)
        )));
    }
    let count = if word(8) & 0x2_0000 != 0 {
        word(28).max(1)
    } else {
        1
    };
    let maximum = width.max(height).ilog2() + 1;
    if count > maximum {
        return Err(refuse(format!(
            "declares {count} mip levels but {width}x{height} permits only {maximum}"
        )));
    }
    let mut levels = Vec::new();
    let mut offset = HEADER + 20;
    for level in 0..count {
        let (width, height) = ((width >> level).max(1), (height >> level).max(1));
        let size = width.div_ceil(4) as usize * height.div_ceil(4) as usize * 16;
        let end = offset + size;
        let data = bytes.get(offset..end).ok_or_else(|| {
            refuse(format!(
                "truncated declared mip chain at level {level} ({width}x{height}): \
                 needs bytes {offset}..{end}, file holds {} bytes",
                bytes.len()
            ))
        })?;
        levels.push(MipLevel {
            width,
            height,
            data: bytes.slice(offset..offset + data.len()),
        });
        offset = end;
    }
    Ok(Some((
        Texture {
            format: TextureFormat::Bc7 { srgb },
            width,
            height,
            faces: 1,
            mips: vec![levels],
        },
        Zero::Transparent,
    )))
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod tests;
