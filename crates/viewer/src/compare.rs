
use serde::Serialize;
use std::{
    fs::File,
    io::{self, BufWriter},
    path::Path,
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub const GAME_UI_WIDTH: u32 = 2560;
pub const GAME_UI_HEIGHT: u32 = 1440;
pub const GAME_UI_WIDE_WIDTH: u32 = 3440;

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

pub const GAME_UI_WIDE_EXCLUSIONS: [Rect; 5] = [
    Rect {
        x: 0,
        y: 0,
        width: 3440,
        height: 100,
    },
    Rect {
        x: 1990,
        y: 100,
        width: 276,
        height: 8,
    },
    Rect {
        x: 3280,
        y: 100,
        width: 160,
        height: 16,
    },
    Rect {
        x: 3345,
        y: 997,
        width: 95,
        height: 443,
    },
    Rect {
        x: 1584,
        y: 1301,
        width: 200,
        height: 139,
    },
];

pub fn game_ui_exclusions(width: u32, height: u32) -> Option<&'static [Rect]> {
    match (width, height) {
        (GAME_UI_WIDTH, GAME_UI_HEIGHT) => Some(&GAME_UI_EXCLUSIONS),
        (GAME_UI_WIDE_WIDTH, GAME_UI_HEIGHT) => Some(&GAME_UI_WIDE_EXCLUSIONS),
        _ => None,
    }
}

pub const DIFF_AMPLIFICATION: u16 = 4;
pub const DIFF_MASKED: [u8; 3] = [128, 128, 128];

const SSIM_WINDOW: usize = 11;
const SSIM_SIGMA: f64 = 1.5;
pub const SSIM_LEVELS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Image {
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

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn rgb(&self) -> &[u8] {
        &self.rgb
    }

    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.at(y as usize * self.width as usize + x as usize))
    }

    pub fn same_size(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
    }

    fn at(&self, index: usize) -> [u8; 3] {
        let base = index * 3;
        [self.rgb[base], self.rgb[base + 1], self.rgb[base + 2]]
    }
}

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

pub fn read_png(path: &Path) -> io::Result<Image> {
    let bytes = std::fs::read(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    decode_png(&bytes).map_err(|error| invalid(format!("{}: {error}", path.display())))
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
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

    pub fn right(&self) -> u32 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(&self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

#[derive(Clone, Debug)]
pub struct Mask {
    width: u32,
    height: u32,
    included: Vec<bool>,
}

impl Mask {
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

    pub fn includes(&self, x: u32, y: u32) -> bool {
        x < self.width
            && y < self.height
            && self.included[y as usize * self.width as usize + x as usize]
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn included_pixels(&self) -> u64 {
        self.included.iter().filter(|included| **included).count() as u64
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Metrics {
    pub pixels: u64,
    pub mae_channels: [Option<f64>; 3],
    pub mae: Option<f64>,
    pub delta_e: Option<f64>,
    pub ssim_levels: [Option<f64>; SSIM_LEVELS],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Region {
    pub index: u32,
    pub row: u32,
    pub column: u32,
    pub rect: Rect,
    pub metrics: Metrics,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub width: u32,
    pub height: u32,
    pub columns: u32,
    pub rows: u32,
    pub excluded_pixels: u64,
    pub overall: Metrics,
    pub regions: Vec<Region>,
}

fn luminance(pixel: [u8; 3]) -> f64 {
    0.299 * pixel[0] as f64 + 0.587 * pixel[1] as f64 + 0.114 * pixel[2] as f64
}

fn srgb_to_linear(channel: u8) -> f64 {
    let value = channel as f64 / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

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

fn crop<T: Copy>(values: &[T], width: usize, rect: Rect) -> Vec<T> {
    let mut result = Vec::with_capacity(rect.width as usize * rect.height as usize);
    for y in rect.y..rect.bottom() {
        let row = y as usize * width;
        result.extend_from_slice(&values[row + rect.x as usize..row + rect.right() as usize]);
    }
    result
}

fn share(total: u32, parts: u32, index: u32) -> (u32, u32) {
    let start = (total as u64 * index as u64 / parts as u64) as u32;
    let end = (total as u64 * (index as u64 + 1) / parts as u64) as u32;
    (start, end)
}

struct Planes {
    width: usize,
    height: usize,
    game_luma: Vec<f64>,
    viewer_luma: Vec<f64>,
    delta_e: Vec<f64>,
    included: Vec<bool>,
}

impl Planes {
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

fn show(value: Option<f64>) -> String {
    match value {
        Some(value) => format!("{value:.4}"),
        None => "n/a".to_string(),
    }
}

impl Report {
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
mod tests;
