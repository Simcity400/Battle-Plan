//! Scoring for reference pairs: the metrics, masks and difference image behind `--compare`.
//!
//! A reference pair is a game screenshot and the viewer's render of the same hour and camera
//! ([archived rendering notes](#) section 2). This module turns such a pair into numbers: mean absolute error
//! in gamma space, a luminance SSIM on a three level pyramid, and a mean CIE76 colour distance,
//! reported once for the frame and once per cell of a region grid so a worker can see which part
//! of the frame is wrong. Everything here is a pure function over decoded images except
//! [`read_png`] and [`write_png`].

use serde::Serialize;
use std::{
    fs::File,
    io::{self, BufWriter},
    path::Path,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Width of the game screenshots the built-in exclusion set was measured on.
pub const GAME_UI_WIDTH: u32 = 2560;
/// Height of the game screenshots the built-in exclusion set was measured on.
pub const GAME_UI_HEIGHT: u32 = 1440;

/// The game's interface regions at 2560x1440, excluded by `--exclude-game-ui`.
///
/// These are measured on `captures\m21_takeover_20260913\array_loaded.png`
/// (the game at 19:00 23 Aug 1943 on the load camera), not estimated:
///
/// - Top bar `0,0 2560x100`. Sampling every 128th column, the last row of the dark top chrome is
///   row 99 in every column outside the army group buttons below, and row 100 is already map: the
///   row mean jumps from `(34, 45, 70)` at row 99 to `(54, 71, 110)` at row 100 and the fraction
///   of near black pixels in the row falls from 0.48 to 0.12.
/// - Army group buttons `890,100 190x66`. Three counters hang under the top bar between x 897 and
///   x 1072 and end at row 165 including their flag strips; the columns at x 896 and x 1024 carry
///   dark chrome down to rows 178 and 153 where every other sampled column stops at row 99.
/// - Right side panels `2440,980 120x460`. The map mode compass, the map mode buttons, the right
///   edge strip and the bottom right button block. The fraction of near black pixels over
///   x 2440..2560 is 0.05 at row 972 and 0.52 at row 988, and over y 985..1440 it is 0.10 at
///   x 2432 against 0.22 at x 2440.
/// - Bottom centre portraits `1186,1286 216x154`. The two leader cards with their tab and banner,
///   measured on a pixel grid as x 1188..1400 and y 1288..1440, rounded outwards by two pixels.
///
/// The Windows "Device Setup" toast at roughly `2100,1150 440x220` in that screenshot is an
/// operating system overlay rather than game interface, so it is not part of this set; exclude it
/// with `--exclude 2100,1150,440,220` when comparing against that capture.
pub const GAME_UI_EXCLUSIONS: [Rect; 4] = [
    Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 100,
    },
    Rect {
        x: 890,
        y: 100,
        width: 190,
        height: 66,
    },
    Rect {
        x: 2440,
        y: 980,
        width: 120,
        height: 460,
    },
    Rect {
        x: 1186,
        y: 1286,
        width: 216,
        height: 154,
    },
];

/// Factor the per channel absolute difference is multiplied by in the `--diff` image.
pub const DIFF_AMPLIFICATION: u16 = 4;
/// Colour the `--diff` image uses for masked pixels.
pub const DIFF_MASKED: [u8; 3] = [128, 128, 128];

/// Side of the SSIM Gaussian window.
const SSIM_WINDOW: usize = 11;
/// Standard deviation of the SSIM Gaussian window.
const SSIM_SIGMA: f64 = 1.5;
/// Number of pyramid levels SSIM is reported on; level 0 is full resolution.
pub const SSIM_LEVELS: usize = 3;

/// An 8 bit RGB image held as tightly packed `width * height * 3` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Image {
    /// Builds an image from packed RGB bytes, rejecting an empty or mismatched buffer.
    pub fn from_rgb(width: u32, height: u32, rgb: Vec<u8>) -> io::Result<Self> {
        if width == 0 || height == 0 {
            return Err(invalid("an image needs a positive width and height"));
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(3))
            .ok_or_else(|| invalid(format!("{width}x{height} is too large to hold in memory")))?;
        if rgb.len() != expected {
            return Err(invalid(format!(
                "expected {expected} RGB bytes for {width}x{height}, got {}",
                rgb.len()
            )));
        }
        Ok(Self { width, height, rgb })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The packed RGB bytes, row major from the top left.
    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    /// The pixel at `(x, y)`, or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.at(y as usize * self.width as usize + x as usize))
    }

    /// Whether `other` has the same pixel dimensions.
    pub fn same_size(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }

    fn at(&self, index: usize) -> [u8; 3] {
        let base = index * 3;
        [self.rgb[base], self.rgb[base + 1], self.rgb[base + 2]]
    }
}

/// Bilinearly resamples an image by `factor`.
///
/// The reference screenshots are 2560x1440 bilinear upscales of the game's
/// 2048x1152 frame (R1 section 0), so the viewer renders at the native size and
/// this puts its frame in the screenshot's pixel space before scoring. Pixel
/// centres are aligned: destination pixel `i` samples source coordinate
/// `(i + 0.5) / factor - 0.5`, clamped at the edges.
pub fn upscale(image: &Image, factor: f64) -> io::Result<Image> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(invalid("the upscale factor must be finite and positive"));
    }
    if factor == 1.0 {
        return Ok(image.clone());
    }
    let width = (f64::from(image.width()) * factor).round().max(1.0);
    let height = (f64::from(image.height()) * factor).round().max(1.0);
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(invalid(format!(
            "upscaling {}x{} by {factor} exceeds the addressable image size",
            image.width(),
            image.height()
        )));
    }
    let (width, height) = (width as u32, height as u32);
    let last_x = image.width() - 1;
    let last_y = image.height() - 1;
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(width as usize * height as usize * 3)
        .map_err(|_| invalid(format!("a {width}x{height} image exceeds available memory")))?;
    for y in 0..height {
        let source_y = ((f64::from(y) + 0.5) / factor - 0.5).clamp(0.0, f64::from(last_y));
        let y0 = source_y.floor() as u32;
        let y1 = (y0 + 1).min(last_y);
        let ty = source_y - f64::from(y0);
        for x in 0..width {
            let source_x = ((f64::from(x) + 0.5) / factor - 0.5).clamp(0.0, f64::from(last_x));
            let x0 = source_x.floor() as u32;
            let x1 = (x0 + 1).min(last_x);
            let tx = source_x - f64::from(x0);
            let corners = [
                image.pixel(x0, y0).unwrap_or_default(),
                image.pixel(x1, y0).unwrap_or_default(),
                image.pixel(x0, y1).unwrap_or_default(),
                image.pixel(x1, y1).unwrap_or_default(),
            ];
            let upper = corners[0].iter().zip(&corners[1]);
            let lower = corners[2].iter().zip(&corners[3]);
            for ((top_left, top_right), (bottom_left, bottom_right)) in upper.zip(lower) {
                let top = f64::from(*top_left) * (1.0 - tx) + f64::from(*top_right) * tx;
                let bottom = f64::from(*bottom_left) * (1.0 - tx) + f64::from(*bottom_right) * tx;
                rgb.push((top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    Image::from_rgb(width, height, rgb)
}

/// Decodes an 8 bit RGB or RGBA PNG, dropping any alpha channel.
pub fn decode_png(bytes: &[u8]) -> io::Result<Image> {
    let mut reader = png::Decoder::new(io::Cursor::new(bytes))
        .read_info()
        .map_err(|error| invalid(format!("not a readable PNG: {error}")))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| invalid("the PNG is too large to decode"))?;
    let mut buffer = vec![0u8; size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| invalid(format!("could not decode the PNG pixels: {error}")))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(invalid(format!(
            "expected an 8 bit PNG, got {:?} bits per sample",
            info.bit_depth
        )));
    }
    let channels = match info.color_type {
        png::ColorType::Rgb => 3usize,
        png::ColorType::Rgba => 4usize,
        other => {
            return Err(invalid(format!(
                "expected an RGB or RGBA PNG, got {other:?}"
            )));
        }
    };
    let row_bytes = (info.width as usize)
        .checked_mul(channels)
        .ok_or_else(|| invalid("the PNG is too wide to decode"))?;
    if info.line_size < row_bytes {
        return Err(invalid(
            "the PNG reported a shorter scan line than its width",
        ));
    }
    let mut rgb = Vec::with_capacity(info.width as usize * info.height as usize * 3);
    for row in buffer[..info.buffer_size()].chunks_exact(info.line_size) {
        for pixel in row[..row_bytes].chunks_exact(channels) {
            rgb.extend_from_slice(&pixel[..3]);
        }
    }
    Image::from_rgb(info.width, info.height, rgb)
}

/// Reads a PNG from `path`, naming the file in any error.
pub fn read_png(path: &Path) -> io::Result<Image> {
    let bytes = std::fs::read(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    decode_png(&bytes).map_err(|error| invalid(format!("{}: {error}", path.display())))
}

/// Writes an RGB PNG to `path`.
pub fn write_png(path: &Path, image: &Image) -> io::Result<()> {
    let file = File::create(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), image.width(), image.height());
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer
        .write_image_data(image.rgb())
        .map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}

/// A pixel rectangle; `x` and `y` are inclusive and `x + width` and `y + height` are exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rect {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels; always positive.
    pub width: u32,
    /// Height in pixels; always positive.
    pub height: u32,
}

impl Rect {
    /// Parses the `x,y,w,h` form taken by `--exclude`.
    pub fn parse(text: &str) -> io::Result<Self> {
        let values = text
            .split(',')
            .map(|part| part.trim().parse::<u32>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                invalid(format!(
                    "`{text}` is not four non-negative integers x,y,w,h"
                ))
            })?;
        match values.as_slice() {
            &[x, y, width, height] if width > 0 && height > 0 => Ok(Self {
                x,
                y,
                width,
                height,
            }),
            _ => Err(invalid(format!(
                "`{text}` is not x,y,w,h with a positive width and height"
            ))),
        }
    }

    /// Right edge, exclusive, saturating rather than overflowing.
    pub fn right(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    /// Bottom edge, exclusive, saturating rather than overflowing.
    pub fn bottom(&self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

/// Which pixels of a frame take part in the comparison.
#[derive(Clone, Debug)]
pub struct Mask {
    width: u32,
    height: u32,
    included: Vec<bool>,
}

impl Mask {
    /// A mask that includes every pixel of a `width` by `height` frame.
    pub fn full(width: u32, height: u32) -> io::Result<Self> {
        if width == 0 || height == 0 {
            return Err(invalid("a mask needs a positive width and height"));
        }
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| invalid(format!("{width}x{height} is too large to hold in memory")))?;
        Ok(Self {
            width,
            height,
            included: vec![true; pixels],
        })
    }

    /// Builds a mask from a mask PNG: a pixel is excluded where all three channels are zero.
    pub fn from_image(image: &Image) -> Self {
        let mut included = Vec::with_capacity(image.width() as usize * image.height() as usize);
        for y in 0..image.height() {
            for x in 0..image.width() {
                included.push(image.pixel(x, y) != Some([0, 0, 0]));
            }
        }
        Self {
            width: image.width(),
            height: image.height(),
            included,
        }
    }

    /// Excludes every pixel of `rect` that lies inside the frame.
    pub fn exclude_rect(&mut self, rect: Rect) {
        let left = rect.x.min(self.width) as usize;
        let right = rect.right().min(self.width) as usize;
        let bottom = rect.bottom().min(self.height);
        for y in rect.y.min(self.height)..bottom {
            let row = y as usize * self.width as usize;
            for included in &mut self.included[row + left..row + right] {
                *included = false;
            }
        }
    }

    /// Whether the pixel at `(x, y)` takes part in the comparison.
    pub fn includes(&self, x: u32, y: u32) -> bool {
        x < self.width
            && y < self.height
            && self.included[y as usize * self.width as usize + x as usize]
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// How many pixels take part in the comparison.
    pub fn included_pixels(&self) -> u64 {
        self.included.iter().filter(|included| **included).count() as u64
    }
}

/// Similarity metrics over one set of unmasked pixels.
///
/// Every value is `None` when it cannot be computed: the means when the set is empty, an SSIM
/// level when no complete 11x11 window of that level fits inside the unmasked pixels.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Metrics {
    /// How many unmasked pixels the means cover.
    pub pixels: u64,
    /// Mean absolute error per channel in 0..255 gamma space units, red then green then blue.
    pub mae_channels: [Option<f64>; 3],
    /// Mean absolute error over all three channels in 0..255 gamma space units.
    pub mae: Option<f64>,
    /// Mean CIE76 colour distance in Lab, sRGB decoded to linear light.
    pub delta_e: Option<f64>,
    /// Luminance SSIM per pyramid level; index 0 is full resolution, each level halves both sides.
    pub ssim_levels: [Option<f64>; SSIM_LEVELS],
}

/// One cell of the region grid.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Region {
    /// Row major index of the cell, `row * columns + column`.
    pub index: u32,
    /// Zero based row of the cell, counted from the top.
    pub row: u32,
    /// Zero based column of the cell, counted from the left.
    pub column: u32,
    /// The pixels the cell covers.
    pub rect: Rect,
    /// Metrics over the unmasked pixels of the cell.
    pub metrics: Metrics,
}

/// The result of comparing one reference pair.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    /// Width of both images in pixels.
    pub width: u32,
    /// Height of both images in pixels.
    pub height: u32,
    /// Columns of the region grid.
    pub columns: u32,
    /// Rows of the region grid.
    pub rows: u32,
    /// How many pixels the mask excluded.
    pub excluded_pixels: u64,
    /// Metrics over every unmasked pixel of the frame.
    pub overall: Metrics,
    /// Metrics per cell of the region grid, in row major order.
    pub regions: Vec<Region>,
}

/// Rec. 601 luminance of a gamma encoded RGB pixel, in the same 0..255 units.
fn luminance(pixel: [u8; 3]) -> f64 {
    0.299 * pixel[0] as f64 + 0.587 * pixel[1] as f64 + 0.114 * pixel[2] as f64
}

/// The sRGB electro-optical transfer function for one 0..255 channel.
fn srgb_to_linear(channel: u8) -> f64 {
    let value = channel as f64 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// CIE Lab under D65 for a gamma encoded sRGB pixel, `table` decoding sRGB to linear light.
fn srgb_to_lab(pixel: [u8; 3], table: &[f64; 256]) -> [f64; 3] {
    let (r, g, b) = (
        table[pixel[0] as usize],
        table[pixel[1] as usize],
        table[pixel[2] as usize],
    );
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_489;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = (0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b) / 1.088_840;
    let f = |t: f64| {
        if t > 216.0 / 24389.0 {
            t.cbrt()
        } else {
            (841.0 / 108.0) * t + 4.0 / 29.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// The normalised 1-D Gaussian used by both SSIM convolution passes.
fn gaussian_kernel() -> [f64; SSIM_WINDOW] {
    let mut kernel = [0.0f64; SSIM_WINDOW];
    let centre = (SSIM_WINDOW as f64 - 1.0) / 2.0;
    let mut total = 0.0;
    for (index, weight) in kernel.iter_mut().enumerate() {
        let offset = index as f64 - centre;
        *weight = (-(offset * offset) / (2.0 * SSIM_SIGMA * SSIM_SIGMA)).exp();
        total += *weight;
    }
    for weight in kernel.iter_mut() {
        *weight /= total;
    }
    kernel
}

/// Separable Gaussian convolution keeping only the window centres whose whole 11x11 window lies
/// inside the plane, so the result is `(width - 10)` by `(height - 10)`.
fn blur(values: &[f64], width: usize, height: usize, kernel: &[f64; SSIM_WINDOW]) -> Vec<f64> {
    let out_width = width - SSIM_WINDOW + 1;
    let out_height = height - SSIM_WINDOW + 1;
    let mut horizontal = vec![0.0f64; out_width * height];
    for y in 0..height {
        let source = &values[y * width..y * width + width];
        let target = &mut horizontal[y * out_width..(y + 1) * out_width];
        for (x, cell) in target.iter_mut().enumerate() {
            *cell = kernel
                .iter()
                .zip(&source[x..x + SSIM_WINDOW])
                .map(|(weight, value)| weight * value)
                .sum();
        }
    }
    let mut result = vec![0.0f64; out_width * out_height];
    for y in 0..out_height {
        for x in 0..out_width {
            result[y * out_width + x] = kernel
                .iter()
                .enumerate()
                .map(|(k, weight)| weight * horizontal[(y + k) * out_width + x])
                .sum();
        }
    }
    result
}

/// The window centres whose whole 11x11 window is unmasked, matching [`blur`]'s output shape.
fn erode(included: &[bool], width: usize, height: usize) -> Vec<bool> {
    let out_width = width - SSIM_WINDOW + 1;
    let out_height = height - SSIM_WINDOW + 1;
    let mut horizontal = vec![false; out_width * height];
    for y in 0..height {
        let source = &included[y * width..y * width + width];
        let target = &mut horizontal[y * out_width..(y + 1) * out_width];
        for (x, cell) in target.iter_mut().enumerate() {
            *cell = source[x..x + SSIM_WINDOW].iter().all(|value| *value);
        }
    }
    let mut result = vec![false; out_width * out_height];
    for y in 0..out_height {
        for x in 0..out_width {
            result[y * out_width + x] =
                (0..SSIM_WINDOW).all(|k| horizontal[(y + k) * out_width + x]);
        }
    }
    result
}

/// Mean SSIM over the fully unmasked windows of one pyramid level, or `None` when there are none.
fn ssim_level(
    game: &[f64],
    viewer: &[f64],
    included: &[bool],
    width: usize,
    height: usize,
) -> Option<f64> {
    if width < SSIM_WINDOW || height < SSIM_WINDOW {
        return None;
    }
    let kernel = gaussian_kernel();
    let square = |values: &[f64]| values.iter().map(|value| value * value).collect::<Vec<_>>();
    let product = game
        .iter()
        .zip(viewer)
        .map(|(a, b)| a * b)
        .collect::<Vec<_>>();
    let mean_game = blur(game, width, height, &kernel);
    let mean_viewer = blur(viewer, width, height, &kernel);
    let mean_game_square = blur(&square(game), width, height, &kernel);
    let mean_viewer_square = blur(&square(viewer), width, height, &kernel);
    let mean_product = blur(&product, width, height, &kernel);
    let windows = erode(included, width, height);
    let c1 = (0.01 * 255.0) * (0.01 * 255.0);
    let c2 = (0.03 * 255.0) * (0.03 * 255.0);
    let mut total = 0.0;
    let mut count = 0u64;
    for (index, usable) in windows.iter().enumerate() {
        if !usable {
            continue;
        }
        let (mg, mv) = (mean_game[index], mean_viewer[index]);
        let variance_game = mean_game_square[index] - mg * mg;
        let variance_viewer = mean_viewer_square[index] - mv * mv;
        let covariance = mean_product[index] - mg * mv;
        total += ((2.0 * mg * mv + c1) * (2.0 * covariance + c2))
            / ((mg * mg + mv * mv + c1) * (variance_game + variance_viewer + c2));
        count += 1;
    }
    (count > 0).then(|| total / count as f64)
}

/// Halves a plane with a 2x2 box average, dropping an odd last row or column.
fn downsample(values: &[f64], width: usize, height: usize) -> Vec<f64> {
    let (out_width, out_height) = (width / 2, height / 2);
    let mut result = Vec::with_capacity(out_width * out_height);
    for y in 0..out_height {
        for x in 0..out_width {
            let top = 2 * y * width + 2 * x;
            let bottom = top + width;
            result
                .push((values[top] + values[top + 1] + values[bottom] + values[bottom + 1]) / 4.0);
        }
    }
    result
}

/// Halves a mask plane; a coarse pixel is unmasked only when all four fine pixels are.
fn downsample_included(included: &[bool], width: usize, height: usize) -> Vec<bool> {
    let (out_width, out_height) = (width / 2, height / 2);
    let mut result = Vec::with_capacity(out_width * out_height);
    for y in 0..out_height {
        for x in 0..out_width {
            let top = 2 * y * width + 2 * x;
            let bottom = top + width;
            result.push(
                included[top] && included[top + 1] && included[bottom] && included[bottom + 1],
            );
        }
    }
    result
}

/// Luminance SSIM on a [`SSIM_LEVELS`] level pyramid, level 0 at full resolution.
fn ssim_pyramid(
    game: &[f64],
    viewer: &[f64],
    included: &[bool],
    width: usize,
    height: usize,
) -> [Option<f64>; SSIM_LEVELS] {
    let mut levels = [None; SSIM_LEVELS];
    let (mut game, mut viewer, mut included) = (game.to_vec(), viewer.to_vec(), included.to_vec());
    let (mut width, mut height) = (width, height);
    for level in levels.iter_mut() {
        if width < SSIM_WINDOW || height < SSIM_WINDOW {
            break;
        }
        *level = ssim_level(&game, &viewer, &included, width, height);
        game = downsample(&game, width, height);
        viewer = downsample(&viewer, width, height);
        included = downsample_included(&included, width, height);
        width /= 2;
        height /= 2;
    }
    levels
}

/// Copies the `rect` sub-rectangle out of a full frame plane.
fn crop<T: Copy>(values: &[T], width: usize, rect: Rect) -> Vec<T> {
    let mut result = Vec::with_capacity(rect.width as usize * rect.height as usize);
    for y in rect.y..rect.bottom() {
        let row = y as usize * width;
        result.extend_from_slice(&values[row + rect.x as usize..row + rect.right() as usize]);
    }
    result
}

/// The `index`th of `parts` equal shares of `total`, as a half-open range.
fn share(total: u32, parts: u32, index: u32) -> (u32, u32) {
    let start = (total as u64 * index as u64 / parts as u64) as u32;
    let end = (total as u64 * (index as u64 + 1) / parts as u64) as u32;
    (start, end)
}

/// Per pixel quantities shared by the frame metrics and every region's metrics.
struct Planes {
    width: usize,
    height: usize,
    game_luma: Vec<f64>,
    viewer_luma: Vec<f64>,
    delta_e: Vec<f64>,
    included: Vec<bool>,
}

impl Planes {
    /// Decodes both images into the per pixel quantities the metrics are built from.
    fn build(game: &Image, viewer: &Image, mask: &Mask) -> Self {
        let mut table = [0.0f64; 256];
        for (channel, value) in table.iter_mut().enumerate() {
            *value = srgb_to_linear(channel as u8);
        }
        let pixels = game.width() as usize * game.height() as usize;
        let mut game_luma = Vec::with_capacity(pixels);
        let mut viewer_luma = Vec::with_capacity(pixels);
        let mut delta_e = Vec::with_capacity(pixels);
        for index in 0..pixels {
            let (a, b) = (game.at(index), viewer.at(index));
            game_luma.push(luminance(a));
            viewer_luma.push(luminance(b));
            let (la, lb) = (srgb_to_lab(a, &table), srgb_to_lab(b, &table));
            delta_e.push(
                ((la[0] - lb[0]).powi(2) + (la[1] - lb[1]).powi(2) + (la[2] - lb[2]).powi(2))
                    .sqrt(),
            );
        }
        Self {
            width: game.width() as usize,
            height: game.height() as usize,
            game_luma,
            viewer_luma,
            delta_e,
            included: mask.included.clone(),
        }
    }

    /// Metrics over the unmasked pixels of `rect`.
    fn metrics(&self, game: &Image, viewer: &Image, rect: Rect) -> Metrics {
        let mut sums = [0.0f64; 3];
        let mut delta_e = 0.0f64;
        let mut count = 0u64;
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let index = y as usize * self.width + x as usize;
                if !self.included[index] {
                    continue;
                }
                let (a, b) = (game.at(index), viewer.at(index));
                for (channel, sum) in sums.iter_mut().enumerate() {
                    *sum += a[channel].abs_diff(b[channel]) as f64;
                }
                delta_e += self.delta_e[index];
                count += 1;
            }
        }
        let mean = |value: f64| (count > 0).then(|| value / count as f64);
        let whole_frame = rect.width as usize == self.width && rect.height as usize == self.height;
        let ssim_levels = if whole_frame {
            ssim_pyramid(
                &self.game_luma,
                &self.viewer_luma,
                &self.included,
                self.width,
                self.height,
            )
        } else {
            ssim_pyramid(
                &crop(&self.game_luma, self.width, rect),
                &crop(&self.viewer_luma, self.width, rect),
                &crop(&self.included, self.width, rect),
                rect.width as usize,
                rect.height as usize,
            )
        };
        Metrics {
            pixels: count,
            mae_channels: [mean(sums[0]), mean(sums[1]), mean(sums[2])],
            mae: mean(sums[0] + sums[1] + sums[2]).map(|value| value / 3.0),
            delta_e: mean(delta_e),
            ssim_levels,
        }
    }
}

/// Rejects a pair of images and a mask that do not share one pixel size.
fn check_sizes(game: &Image, viewer: &Image, mask: &Mask) -> io::Result<()> {
    if !game.same_size(viewer) {
        return Err(invalid(format!(
            "image size mismatch: the game image is {}x{} and the viewer image is {}x{}",
            game.width(),
            game.height(),
            viewer.width(),
            viewer.height()
        )));
    }
    if mask.width() != game.width() || mask.height() != game.height() {
        return Err(invalid(format!(
            "mask size mismatch: the mask is {}x{} and the images are {}x{}",
            mask.width(),
            mask.height(),
            game.width(),
            game.height()
        )));
    }
    Ok(())
}

/// Compares a reference pair over the unmasked pixels, overall and per cell of a grid.
///
/// `columns` and `rows` describe the region grid; cells are reported row major. Both images and
/// the mask must have the same pixel dimensions and the grid must fit the image.
pub fn compare(
    game: &Image,
    viewer: &Image,
    mask: &Mask,
    columns: u32,
    rows: u32,
) -> io::Result<Report> {
    check_sizes(game, viewer, mask)?;
    if columns == 0 || rows == 0 || columns > game.width() || rows > game.height() {
        return Err(invalid(format!(
            "a {columns}x{rows} region grid does not fit a {}x{} image",
            game.width(),
            game.height()
        )));
    }
    let planes = Planes::build(game, viewer, mask);
    let frame = Rect {
        x: 0,
        y: 0,
        width: game.width(),
        height: game.height(),
    };
    let overall = planes.metrics(game, viewer, frame);
    let mut regions = Vec::with_capacity(columns as usize * rows as usize);
    for row in 0..rows {
        let (top, bottom) = share(game.height(), rows, row);
        for column in 0..columns {
            let (left, right) = share(game.width(), columns, column);
            let rect = Rect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            };
            regions.push(Region {
                index: row * columns + column,
                row,
                column,
                rect,
                metrics: planes.metrics(game, viewer, rect),
            });
        }
    }
    Ok(Report {
        width: game.width(),
        height: game.height(),
        columns,
        rows,
        excluded_pixels: frame.width as u64 * frame.height as u64 - mask.included_pixels(),
        overall,
        regions,
    })
}

/// The `--diff` image: the absolute per channel difference amplified by [`DIFF_AMPLIFICATION`]
/// and clamped, with masked pixels painted [`DIFF_MASKED`].
pub fn diff_image(game: &Image, viewer: &Image, mask: &Mask) -> io::Result<Image> {
    check_sizes(game, viewer, mask)?;
    let mut rgb = Vec::with_capacity(game.rgb().len());
    for y in 0..game.height() {
        for x in 0..game.width() {
            if !mask.includes(x, y) {
                rgb.extend_from_slice(&DIFF_MASKED);
                continue;
            }
            let index = y as usize * game.width() as usize + x as usize;
            let (a, b) = (game.at(index), viewer.at(index));
            for channel in 0..3 {
                let difference = a[channel].abs_diff(b[channel]) as u16;
                rgb.push((difference * DIFF_AMPLIFICATION).min(255) as u8);
            }
        }
    }
    Image::from_rgb(game.width(), game.height(), rgb)
}

/// Formats an optional metric for the text report.
fn show(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.4}"),
        None => "n/a".to_string(),
    }
}

impl Report {
    /// The plain text report: `metric=value` lines for the frame, then the region table.
    pub fn to_text(&self) -> String {
        let mut text = String::new();
        text.push_str(&format!("width={}\n", self.width));
        text.push_str(&format!("height={}\n", self.height));
        text.push_str(&format!("regions={}x{}\n", self.columns, self.rows));
        text.push_str(&format!("compared_pixels={}\n", self.overall.pixels));
        text.push_str(&format!("excluded_pixels={}\n", self.excluded_pixels));
        text.push_str(&format!("mae={}\n", show(self.overall.mae)));
        text.push_str(&format!("mae_r={}\n", show(self.overall.mae_channels[0])));
        text.push_str(&format!("mae_g={}\n", show(self.overall.mae_channels[1])));
        text.push_str(&format!("mae_b={}\n", show(self.overall.mae_channels[2])));
        text.push_str(&format!("delta_e={}\n", show(self.overall.delta_e)));
        for (level, value) in self.overall.ssim_levels.iter().enumerate() {
            text.push_str(&format!("ssim_level{level}={}\n", show(*value)));
        }
        text.push_str(&format!(
            "\n{:>4} {:>3} {:>3} {:>5} {:>5} {:>5} {:>5} {:>10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}\n",
            "cell",
            "row",
            "col",
            "x",
            "y",
            "w",
            "h",
            "pixels",
            "mae",
            "mae_r",
            "mae_g",
            "mae_b",
            "delta_e",
            "ssim_l0",
            "ssim_l1",
            "ssim_l2",
        ));
        for region in &self.regions {
            text.push_str(&format!(
                "{:>4} {:>3} {:>3} {:>5} {:>5} {:>5} {:>5} {:>10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}\n",
                region.index,
                region.row,
                region.column,
                region.rect.x,
                region.rect.y,
                region.rect.width,
                region.rect.height,
                region.metrics.pixels,
                show(region.metrics.mae),
                show(region.metrics.mae_channels[0]),
                show(region.metrics.mae_channels[1]),
                show(region.metrics.mae_channels[2]),
                show(region.metrics.delta_e),
                show(region.metrics.ssim_levels[0]),
                show(region.metrics.ssim_levels[1]),
                show(region.metrics.ssim_levels[2]),
            ));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic synthetic frame with variation in every channel and no value above 200,
    /// so a test may add an offset without clamping.
    fn gradient(width: u32, height: u32) -> Image {
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for y in 0..height {
            for x in 0..width {
                rgb.push((x * 3 % 200) as u8);
                rgb.push((y * 5 % 200) as u8);
                rgb.push(((x + y) * 7 % 200) as u8);
            }
        }
        Image::from_rgb(width, height, rgb).unwrap()
    }

    fn solid(width: u32, height: u32, colour: [u8; 3]) -> Image {
        let rgb = colour
            .iter()
            .cycle()
            .take(width as usize * height as usize * 3)
            .copied()
            .collect();
        Image::from_rgb(width, height, rgb).unwrap()
    }

    /// Copies `image` and adds `delta` to every channel of every pixel inside `rect`.
    fn offset_rect(image: &Image, rect: Rect, delta: i16) -> Image {
        let mut rgb = image.rgb().to_vec();
        for y in rect.y..rect.bottom() {
            for x in rect.x..rect.right() {
                let base = (y as usize * image.width() as usize + x as usize) * 3;
                for channel in &mut rgb[base..base + 3] {
                    *channel = (*channel as i16 + delta).clamp(0, 255) as u8;
                }
            }
        }
        Image::from_rgb(image.width(), image.height(), rgb).unwrap()
    }

    fn whole(image: &Image) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        }
    }

    fn close(value: Option<f64>, expected: f64) -> bool {
        value.is_some_and(|value| (value - expected).abs() < 1e-9)
    }

    #[test]
    fn identical_images_have_no_error_and_perfect_structure() {
        let image = gradient(64, 48);
        let mask = Mask::full(64, 48).unwrap();
        let report = compare(&image, &image, &mask, 4, 4).unwrap();
        assert_eq!(report.overall.pixels, 64 * 48);
        assert_eq!(report.excluded_pixels, 0);
        assert!(close(report.overall.mae, 0.0));
        assert!(close(report.overall.delta_e, 0.0));
        for channel in report.overall.mae_channels {
            assert!(close(channel, 0.0));
        }
        // 64x48 halves to 32x24 and 16x12, so all three levels have complete 11x11 windows.
        for level in report.overall.ssim_levels {
            assert!(close(level, 1.0), "expected SSIM 1, got {level:?}");
        }
        for region in &report.regions {
            assert!(close(region.metrics.mae, 0.0));
            assert!(close(region.metrics.ssim_levels[0], 1.0));
        }
    }

    #[test]
    fn a_constant_offset_gives_that_offset_as_the_mean_absolute_error() {
        let game = gradient(64, 48);
        let viewer = offset_rect(&game, whole(&game), 10);
        let mask = Mask::full(64, 48).unwrap();
        let report = compare(&game, &viewer, &mask, 2, 2).unwrap();
        assert!(close(report.overall.mae, 10.0));
        for channel in report.overall.mae_channels {
            assert!(close(channel, 10.0));
        }
        assert!(report.overall.delta_e.unwrap() > 0.0);
        for region in &report.regions {
            assert!(close(region.metrics.mae, 10.0));
        }
    }

    #[test]
    fn an_uneven_offset_averages_the_channels() {
        let game = solid(32, 32, [10, 20, 30]);
        let viewer = solid(32, 32, [16, 20, 60]);
        let mask = Mask::full(32, 32).unwrap();
        let report = compare(&game, &viewer, &mask, 1, 1).unwrap();
        assert!(close(report.overall.mae_channels[0], 6.0));
        assert!(close(report.overall.mae_channels[1], 0.0));
        assert!(close(report.overall.mae_channels[2], 30.0));
        assert!(close(report.overall.mae, 12.0));
    }

    #[test]
    fn masking_removes_the_offset_region() {
        let game = gradient(64, 48);
        let wrong = Rect {
            x: 40,
            y: 30,
            width: 20,
            height: 15,
        };
        let viewer = offset_rect(&game, wrong, 20);
        let open = Mask::full(64, 48).unwrap();
        let unmasked = compare(&game, &viewer, &open, 1, 1).unwrap();
        assert!(unmasked.overall.mae.unwrap() > 0.0);
        assert!(unmasked.overall.ssim_levels[0].unwrap() < 1.0);

        let mut masked = Mask::full(64, 48).unwrap();
        masked.exclude_rect(wrong);
        let report = compare(&game, &viewer, &masked, 1, 1).unwrap();
        assert_eq!(report.excluded_pixels, 20 * 15);
        assert_eq!(report.overall.pixels, 64 * 48 - 20 * 15);
        assert!(close(report.overall.mae, 0.0));
        assert!(close(report.overall.delta_e, 0.0));
        assert!(close(report.overall.ssim_levels[0], 1.0));
    }

    #[test]
    fn masking_every_pixel_leaves_no_metrics() {
        let game = gradient(32, 32);
        let viewer = offset_rect(&game, whole(&game), 5);
        let mut mask = Mask::full(32, 32).unwrap();
        mask.exclude_rect(whole(&game));
        let report = compare(&game, &viewer, &mask, 2, 2).unwrap();
        assert_eq!(report.overall.pixels, 0);
        assert_eq!(report.excluded_pixels, 32 * 32);
        assert_eq!(report.overall.mae, None);
        assert_eq!(report.overall.delta_e, None);
        assert_eq!(report.overall.ssim_levels, [None; SSIM_LEVELS]);
        assert!(report.to_text().contains("mae=n/a"));
    }

    #[test]
    fn region_indices_are_row_major_and_tile_the_frame() {
        let game = gradient(96, 48);
        // Cell (row 1, column 1) of a 4x2 grid covers x 24..48 and y 24..48.
        let wrong = Rect {
            x: 24,
            y: 24,
            width: 24,
            height: 24,
        };
        let viewer = offset_rect(&game, wrong, 30);
        let mask = Mask::full(96, 48).unwrap();
        let report = compare(&game, &viewer, &mask, 4, 2).unwrap();
        assert_eq!(report.regions.len(), 8);
        let mut covered = 0u64;
        for (position, region) in report.regions.iter().enumerate() {
            assert_eq!(region.index as usize, position);
            assert_eq!(region.index, region.row * report.columns + region.column);
            assert_eq!(region.row, position as u32 / 4);
            assert_eq!(region.column, position as u32 % 4);
            covered += region.rect.width as u64 * region.rect.height as u64;
        }
        assert_eq!(covered, 96 * 48);
        let target = &report.regions[5];
        assert_eq!((target.row, target.column), (1, 1));
        assert_eq!(target.rect, wrong);
        assert!(close(target.metrics.mae, 30.0));
        for (position, region) in report.regions.iter().enumerate() {
            if position != 5 {
                assert!(close(region.metrics.mae, 0.0), "cell {position} differs");
            }
        }
    }

    #[test]
    fn region_cells_tile_a_size_the_grid_does_not_divide() {
        let image = gradient(37, 23);
        let mask = Mask::full(37, 23).unwrap();
        let report = compare(&image, &image, &mask, 4, 3).unwrap();
        let mut covered = 0u64;
        for region in &report.regions {
            covered += region.rect.width as u64 * region.rect.height as u64;
            assert!(region.rect.right() <= 37 && region.rect.bottom() <= 23);
        }
        assert_eq!(covered, 37 * 23);
    }

    #[test]
    fn small_regions_report_no_ssim_rather_than_failing() {
        let image = gradient(96, 48);
        let mask = Mask::full(96, 48).unwrap();
        let report = compare(&image, &image, &mask, 4, 2).unwrap();
        // Cells are 24x24: level 0 and level 1 fit an 11x11 window, level 2 at 6x6 does not.
        let cell = &report.regions[0].metrics;
        assert!(close(cell.ssim_levels[0], 1.0));
        assert!(close(cell.ssim_levels[1], 1.0));
        assert_eq!(cell.ssim_levels[2], None);
    }

    #[test]
    fn the_colour_distance_sees_a_hue_change_the_luminance_ssim_misses() {
        // Two colours chosen for near equal Rec. 601 luminance but opposite hue.
        let purple = [150u8, 50, 200];
        let green = [40u8, 140, 40];
        assert!((luminance(purple) - luminance(green)).abs() < 2.0);
        let mask = Mask::full(32, 32).unwrap();
        let report = compare(&solid(32, 32, purple), &solid(32, 32, green), &mask, 1, 1).unwrap();
        assert!(report.overall.ssim_levels[0].unwrap() > 0.999);
        assert!(report.overall.delta_e.unwrap() > 20.0);
        assert!(close(report.overall.mae, 120.0));
    }

    #[test]
    fn a_size_mismatch_is_an_error() {
        let game = gradient(32, 32);
        let viewer = gradient(32, 16);
        let mask = Mask::full(32, 32).unwrap();
        let error = compare(&game, &viewer, &mask, 1, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("image size mismatch"));
        assert!(diff_image(&game, &viewer, &mask).is_err());
    }

    #[test]
    fn a_mask_size_mismatch_is_an_error() {
        let image = gradient(32, 32);
        let mask = Mask::full(16, 16).unwrap();
        let error = compare(&image, &image, &mask, 1, 1).unwrap_err();
        assert!(error.to_string().contains("mask size mismatch"));
    }

    #[test]
    fn a_region_grid_larger_than_the_image_is_an_error() {
        let image = gradient(16, 16);
        let mask = Mask::full(16, 16).unwrap();
        assert!(compare(&image, &image, &mask, 0, 4).is_err());
        assert!(compare(&image, &image, &mask, 4, 0).is_err());
        assert!(compare(&image, &image, &mask, 17, 4).is_err());
        assert!(compare(&image, &image, &mask, 4, 17).is_err());
    }

    #[test]
    fn a_mask_image_excludes_black_pixels_only() {
        let mut rgb = vec![255u8; 4 * 2 * 3];
        rgb[0..3].copy_from_slice(&[0, 0, 0]);
        rgb[3..6].copy_from_slice(&[0, 0, 1]);
        let mask = Mask::from_image(&Image::from_rgb(4, 2, rgb).unwrap());
        assert!(!mask.includes(0, 0));
        assert!(mask.includes(1, 0));
        assert!(mask.includes(3, 1));
        assert_eq!(mask.included_pixels(), 7);
    }

    #[test]
    fn excluded_rectangles_clip_to_the_frame() {
        let mut mask = Mask::full(8, 4).unwrap();
        mask.exclude_rect(Rect {
            x: 6,
            y: 2,
            width: 100,
            height: 100,
        });
        assert_eq!(mask.included_pixels(), 8 * 4 - 4);
        assert!(!mask.includes(7, 3));
        assert!(mask.includes(5, 3));
        mask.exclude_rect(Rect {
            x: 20,
            y: 20,
            width: 4,
            height: 4,
        });
        assert_eq!(mask.included_pixels(), 8 * 4 - 4);
        assert!(!mask.includes(9, 9));
    }

    #[test]
    fn the_difference_image_amplifies_and_greys_masked_pixels() {
        let game = solid(4, 2, [10, 20, 30]);
        let viewer = solid(4, 2, [20, 20, 100]);
        let mut mask = Mask::full(4, 2).unwrap();
        mask.exclude_rect(Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        });
        let diff = diff_image(&game, &viewer, &mask).unwrap();
        assert_eq!(diff.pixel(0, 0), Some(DIFF_MASKED));
        // 10 and 70 amplified four times, the second clamped at 255.
        assert_eq!(diff.pixel(1, 0), Some([40, 0, 255]));
        assert_eq!(diff.pixel(3, 1), Some([40, 0, 255]));
    }

    #[test]
    fn rectangles_parse_from_the_command_line_form() {
        assert_eq!(
            Rect::parse("1,2,3,4").unwrap(),
            Rect {
                x: 1,
                y: 2,
                width: 3,
                height: 4
            }
        );
        assert_eq!(Rect::parse(" 1 , 2 , 3 , 4 ").unwrap().width, 3);
        for bad in [
            "1,2,3",
            "1,2,3,4,5",
            "1,2,0,4",
            "1,2,3,0",
            "-1,2,3,4",
            "a,b,c,d",
            "",
        ] {
            assert!(Rect::parse(bad).is_err(), "`{bad}` should be rejected");
        }
    }

    #[test]
    fn images_reject_a_buffer_that_does_not_match_the_dimensions() {
        assert!(Image::from_rgb(2, 2, vec![0; 12]).is_ok());
        assert!(Image::from_rgb(2, 2, vec![0; 11]).is_err());
        assert!(Image::from_rgb(0, 2, Vec::new()).is_err());
        assert!(Mask::full(2, 0).is_err());
    }

    /// Encodes packed samples as a PNG in memory so decoding can be tested without a file.
    fn encode(width: u32, height: u32, colour: png::ColorType, data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(colour);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        bytes
    }

    #[test]
    fn rgb_and_rgba_pngs_decode_to_the_same_pixels() {
        let rgb = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
        let rgba = vec![1u8, 2, 3, 200, 4, 5, 6, 201, 7, 8, 9, 202, 10, 11, 12, 203];
        let from_rgb = decode_png(&encode(2, 2, png::ColorType::Rgb, &rgb)).unwrap();
        let from_rgba = decode_png(&encode(2, 2, png::ColorType::Rgba, &rgba)).unwrap();
        assert_eq!(from_rgb.rgb(), rgb.as_slice());
        assert_eq!(from_rgb, from_rgba);
        assert_eq!(from_rgb.pixel(1, 1), Some([10, 11, 12]));
        assert_eq!(from_rgb.pixel(2, 0), None);
    }

    #[test]
    fn a_greyscale_png_and_a_non_png_are_named_errors() {
        let grey = encode(2, 2, png::ColorType::Grayscale, &[1, 2, 3, 4]);
        let error = decode_png(&grey).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("RGB or RGBA"), "{error}");
        assert!(decode_png(b"not a png at all").is_err());
    }

    #[test]
    fn the_measured_game_interface_rectangles_stay_inside_the_frame() {
        let mut mask = Mask::full(GAME_UI_WIDTH, GAME_UI_HEIGHT).unwrap();
        for rect in GAME_UI_EXCLUSIONS {
            assert!(rect.right() <= GAME_UI_WIDTH && rect.bottom() <= GAME_UI_HEIGHT);
            mask.exclude_rect(rect);
        }
        // The map is still the large majority of the frame after the interface is removed.
        let total = u64::from(GAME_UI_WIDTH) * u64::from(GAME_UI_HEIGHT);
        assert!(mask.included_pixels() > total * 3 / 4);
        assert!(!mask.includes(0, 99));
        assert!(mask.includes(0, 100));
        assert!(!mask.includes(1000, 120));
        assert!(!mask.includes(2500, 1000));
        assert!(!mask.includes(1200, 1300));
    }

    #[test]
    fn the_text_report_lists_every_metric_and_one_row_per_region() {
        let image = gradient(64, 48);
        let mask = Mask::full(64, 48).unwrap();
        let text = compare(&image, &image, &mask, 4, 4).unwrap().to_text();
        for key in [
            "width=64",
            "height=48",
            "regions=4x4",
            "compared_pixels=3072",
            "excluded_pixels=0",
            "mae=",
            "mae_r=",
            "mae_g=",
            "mae_b=",
            "delta_e=",
            "ssim_level0=",
            "ssim_level1=",
            "ssim_level2=",
        ] {
            assert!(text.contains(key), "the report is missing `{key}`");
        }
        assert_eq!(text.lines().count(), 13 + 2 + 16);
    }

    #[test]
    fn upscaling_two_by_two_to_four_by_four_aligns_pixel_centres() {
        // A single red channel ramp: 0, 100 on the top row, 200, 40 below.
        let image = Image::from_rgb(2, 2, vec![0, 0, 0, 100, 0, 0, 200, 0, 0, 40, 0, 0]).unwrap();
        let scaled = upscale(&image, 2.0).unwrap();
        assert_eq!((scaled.width(), scaled.height()), (4, 4));
        // Destination x maps to source -0.25, 0.25, 0.75, 1.25, clamped to
        // 0.0, 0.25, 0.75, 1.0, so the top row reads 0, 25, 75, 100.
        let row: Vec<u8> = (0..4).map(|x| scaled.pixel(x, 0).unwrap()[0]).collect();
        assert_eq!(row, vec![0, 25, 75, 100]);
        // The bottom row is the same interpolation of 200 and 40.
        let row: Vec<u8> = (0..4).map(|x| scaled.pixel(x, 3).unwrap()[0]).collect();
        assert_eq!(row, vec![200, 160, 80, 40]);
        // Row 1 sits a quarter of the way from the top row to the bottom row.
        assert_eq!(scaled.pixel(0, 1).unwrap()[0], 50);
        assert_eq!(scaled.pixel(3, 2).unwrap()[0], 55);
    }

    #[test]
    fn upscaling_by_one_and_a_quarter_reaches_the_screenshot_size() {
        let scaled = upscale(&gradient(2048, 8), 1.25).unwrap();
        assert_eq!(scaled.width(), 2560);
        assert_eq!(scaled.height(), 10);
        assert!(upscale(&gradient(4, 4), 0.0).is_err());
        assert!(upscale(&gradient(4, 4), f64::NAN).is_err());
        assert_eq!(upscale(&gradient(4, 4), 1.0).unwrap(), gradient(4, 4));
    }
}
