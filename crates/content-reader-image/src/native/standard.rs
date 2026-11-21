use super::{Failure, Zero, libjpeg, oversized, rgba, zero_extent};
use crate::dds::Texture;
use image::{DynamicImage, ImageDecoder, ImageError, ImageFormat, ImageReader, Limits};
use std::io::Cursor;

pub(super) fn load(
    bytes: &[u8],
    tga_name: bool,
) -> Result<Result<(Texture, Zero), String>, Failure> {
    let format = image::guess_format(bytes).ok().or_else(|| {
        (tga_name || bytes.ends_with(b"TRUEVISION-XFILE.\0")).then_some(ImageFormat::Tga)
    });
    match format {
        Some(ImageFormat::Jpeg) => jpeg(bytes),
        Some(
            format @ (ImageFormat::Bmp | ImageFormat::Ico | ImageFormat::Png | ImageFormat::Tga),
        ) => {
            let decode = |error| match error {
                ImageError::Limits(_) => Err(Failure::OverBudget(format!("{format:?}: {error}"))),
                _ => Ok(Err(format!("{format:?}: {error}"))),
            };
            let decoder = match ImageReader::with_format(Cursor::new(bytes), format).into_decoder()
            {
                Ok(decoder) => decoder,
                Err(error) => return decode(error),
            };
            let (width, height) = decoder.dimensions();
            if width == 0 || height == 0 {
                return Err(zero_extent(width, height));
            }
            oversized(width, height)?;
            let mut memory = Limits::default();
            memory.reserve(decoder.total_bytes()).map_err(|error| {
                Failure::OverBudget(format!("{format:?} image buffer: {error}"))
            })?;
            if decoder.color_type() != image::ColorType::Rgba8 {
                memory
                    .reserve(u64::from(width) * u64::from(height) * 4)
                    .map_err(|error| {
                        Failure::OverBudget(format!("{format:?} RGBA conversion: {error}"))
                    })?;
            }
            match DynamicImage::from_decoder(decoder) {
                Ok(image) => {
                    let pixels = image.into_rgba8().into_raw();
                    let zero = Zero::of_32_bits(&pixels);
                    Ok(Ok((rgba(width, height, pixels), zero)))
                }
                Err(error) => decode(error),
            }
        }
        Some(format) => Ok(Err(format!("{format:?} is not a supported texture format"))),
        None => Ok(Err("unrecognised texture format".to_owned())),
    }
}

fn jpeg(bytes: &[u8]) -> Result<Result<(Texture, Zero), String>, Failure> {
    let decoded = match libjpeg::decode(bytes) {
        Ok(decoded) => decoded,
        Err(libjpeg::Stop::Fatal(reason)) => return Ok(Err(format!("libjpeg 9a: {reason}"))),
        Err(libjpeg::Stop::Undefined(reason)) => return Err(Failure::Undefined(reason)),
        Err(libjpeg::Stop::Device(reason)) => return Err(Failure::Device(reason)),
        Err(libjpeg::Stop::OverBudget(reason)) => return Err(Failure::OverBudget(reason)),
    };
    let mut pixels = Vec::with_capacity(decoded.width * decoded.height * 4);
    match decoded.components {
        1 => {
            for grey in decoded.samples {
                pixels.extend_from_slice(&[grey, grey, grey, 255]);
            }
        }
        3 => {
            for rgb in decoded.samples.chunks_exact(3) {
                pixels.extend_from_slice(rgb);
                pixels.push(255);
            }
        }
        4 if decoded.cmyk => {
            for cmyk in decoded.samples.chunks_exact(4) {
                let cmy = [cmyk[0], cmyk[1], cmyk[2]];
                let k = u16::from(cmyk[3]);
                pixels.extend_from_slice(&cmy.map(|channel| (u16::from(channel) * k / 255) as u8));
                pixels.push(255);
            }
        }
        components => {
            return Ok(Err(format!(
                "JPEG has {components} unsupported colour components"
            )));
        }
    }
    let zero = if decoded.components == 1 {
        Zero::Transparent
    } else {
        Zero::Opaque
    };
    Ok(Ok((
        rgba(decoded.width as u32, decoded.height as u32, pixels),
        zero,
    )))
}
