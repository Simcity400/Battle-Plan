//! WEATHER1 dump bin: resolve the shipped weather-overlay render law and draw a
//! top-down overlay PNG per weather type for the orchestrator's eyeball.
//!
//! Files-first: everything comes through the mod-aware `GameFileResolver` from
//! shipped installed content — `common/weather.txt`, `gfx/entities/weather_entities.asset`,
//! `gfx/entities/particles.gfx`, `gfx/particles/environment/*_clouds.asset`,
//! `gfx/particles/*.dds`, `map/weatherpositions.txt`, `common/defines/00_graphics.lua`.
//! No captured bytes are consumed here.
//!
//! Usage:
//!   dump_weather_law --game <the game dir> [--mod <content dir>]...
//!                    [--out <dir>] [--width N] [--height N]
//!                    [--type rain_light|rain_heavy|snow|blizzard|sandstorm|all]
//!                    [--region <id>]...
//!
//! Outputs (in --out, default ./weather_law_dump):
//!   weather_<type>.png          top-down overlay for that weather type
//!   weather_<type>_params.json  the resolved CloudRenderLaw + fade defines
//!
//! The overlay is a deliberately ORTHOGRAPHIC top-down projection of the
//! file-derived law (position/size/color/blend/alpha). The shipped
//! `particle.shader` draws camera-facing billboards through the runtime
//! ViewProjection CB (an UNTRACED runtime input owned by the camera lane); this
//! bin verifies the file-derived parameters without inventing a camera. See
//! documents/WEATHER_LAW.md.

use map_viewer::{
    cloud_law_from_resolver, render_weather_overlay, weather_fade_defines_from_resolver,
    GameFileResolver, Result, WeatherPositionSize, WeatherType,
};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;

    let resolver = if args.mods.is_empty() {
        GameFileResolver::vanilla(&args.game)?
    } else {
        GameFileResolver::with_mod_paths(args.game.clone(), args.mods.clone())?
    };

    fs::create_dir_all(&args.out)?;

    let fade = weather_fade_defines_from_resolver(&resolver)?;
    println!(
        "weather fade defines: distance_cutoff={} distance_fade_length={} zoom_in_cutoff={} zoom_in_fade_length={} zoom_in_fade_factor={}",
        fade.distance_cutoff,
        fade.distance_fade_length,
        fade.zoom_in_cutoff,
        fade.zoom_in_fade_length,
        fade.zoom_in_fade_factor,
    );

    let types = args.types();
    let regions: Option<&[u32]> = if args.regions.is_empty() {
        None
    } else {
        Some(&args.regions)
    };

    for ty in types {
        // Resolve + report the law for both variants.
        let mut params = String::new();
        params.push_str("{\n");
        params.push_str(&format!("  \"weather_type\": \"{}\",\n", ty.key()));
        params.push_str("  \"fade_defines\": {\n");
        params.push_str(&format!(
            "    \"distance_cutoff\": {}, \"distance_fade_length\": {},\n",
            fade.distance_cutoff, fade.distance_fade_length
        ));
        params.push_str(&format!(
            "    \"zoom_in_cutoff\": {}, \"zoom_in_fade_length\": {}, \"zoom_in_fade_factor\": {}\n",
            fade.zoom_in_cutoff, fade.zoom_in_fade_length, fade.zoom_in_fade_factor
        ));
        params.push_str("  },\n");
        params.push_str("  \"cloud_laws\": [\n");
        let mut first = true;
        for size in [WeatherPositionSize::Big, WeatherPositionSize::Small] {
            if let Some(law) = cloud_law_from_resolver(&resolver, ty, size)? {
                if !first {
                    params.push_str(",\n");
                }
                first = false;
                let vc = law.linear_vertex_color(1.0);
                params.push_str(&format!(
                    "    {{ \"size\": \"{}\", \"particle_file\": \"{}\", \"texture\": \"{}\", \"shader_effect\": \"{}\", \"blend\": \"{:?}\", \"color_rgb_255\": [{}, {}, {}], \"base_alpha_255\": {}, \"alpha_anim\": {}, \"sort\": {}, \"linear_vertex_color\": [{:.5}, {:.5}, {:.5}, {:.5}], \"size_mean\": {}, \"life_mean\": {} }}",
                    match size { WeatherPositionSize::Big => "big", WeatherPositionSize::Small => "small" },
                    law.particle_file,
                    law.texture,
                    law.shader_effect,
                    law.blend,
                    law.color_rgb_255[0], law.color_rgb_255[1], law.color_rgb_255[2],
                    law.base_alpha_255,
                    law.alpha_anim.as_deref().map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string()),
                    law.sort.as_deref().map(|s| format!("\"{s}\"")).unwrap_or_else(|| "null".to_string()),
                    vc[0], vc[1], vc[2], vc[3],
                    law.size_mean, law.life_mean,
                ));
            }
        }
        params.push_str("\n  ]\n}\n");
        let params_path = args.out.join(format!("weather_{}_params.json", ty.key()));
        fs::write(&params_path, params)?;
        println!("wrote {}", params_path.display());

        // Render the overlay onto a dark neutral plane so the clouds are visible.
        let overlay = render_weather_overlay(&resolver, ty, args.width, args.height, regions)?;
        let composited = composite_over_backdrop(&overlay.rgba, args.width, args.height);
        let png_path = args.out.join(format!("weather_{}.png", ty.key()));
        write_png_rgba(&png_path, args.width, args.height, &composited)?;
        println!(
            "wrote {} ({}x{})",
            png_path.display(),
            args.width,
            args.height
        );
    }

    Ok(())
}

/// Composite the straight-alpha overlay over a dark backdrop so the semi-
/// transparent clouds read on a saved image; returns opaque RGBA.
fn composite_over_backdrop(overlay: &[u8], width: usize, height: usize) -> Vec<u8> {
    const BG: [u8; 3] = [24, 34, 48]; // dark blue-grey "night map" backdrop
    let mut out = vec![0u8; width * height * 4];
    for i in 0..width * height {
        let a = overlay[i * 4 + 3] as f32 / 255.0;
        for c in 0..3 {
            let fg = overlay[i * 4 + c] as f32;
            let bg = BG[c] as f32;
            out[i * 4 + c] = (fg * a + bg * (1.0 - a)).clamp(0.0, 255.0) as u8;
        }
        out[i * 4 + 3] = 255;
    }
    out
}

// ---------------------------------------------------------------------------
// Minimal self-contained PNG (RGBA8, stored/uncompressed deflate) — no deps.
// ---------------------------------------------------------------------------

fn write_png_rgba(path: &Path, width: usize, height: usize, rgba: &[u8]) -> Result<()> {
    if rgba.len() != width * height * 4 {
        return Err(format!(
            "PNG payload has {} bytes but {width}x{height} RGBA needs {}",
            rgba.len(),
            width * height * 4
        )
        .into());
    }
    let mut file = fs::File::create(path)?;
    file.write_all(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])?;

    // IHDR
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // color type RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    write_chunk(&mut file, b"IHDR", &ihdr)?;

    // Raw scanlines, each prefixed with filter byte 0.
    let mut raw = Vec::with_capacity(height * (1 + width * 4));
    for y in 0..height {
        raw.push(0u8);
        let row = &rgba[y * width * 4..(y + 1) * width * 4];
        raw.extend_from_slice(row);
    }

    let zlib = zlib_stored(&raw);
    write_chunk(&mut file, b"IDAT", &zlib)?;
    write_chunk(&mut file, b"IEND", &[])?;
    Ok(())
}

fn write_chunk(file: &mut fs::File, tag: &[u8; 4], data: &[u8]) -> Result<()> {
    file.write_all(&(data.len() as u32).to_be_bytes())?;
    file.write_all(tag)?;
    file.write_all(data)?;
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(tag);
    crc_input.extend_from_slice(data);
    file.write_all(&crc32(&crc_input).to_be_bytes())?;
    Ok(())
}

/// A zlib stream wrapping DEFLATE stored (uncompressed) blocks + adler32.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x78); // CMF: deflate, 32K window
    out.push(0x01); // FLG: no dict, fastest
                    // Stored blocks, max 65535 bytes each.
    let mut i = 0usize;
    while i < data.len() {
        let chunk = (data.len() - i).min(0xFFFF);
        let last = i + chunk >= data.len();
        out.push(if last { 1 } else { 0 }); // BFINAL, BTYPE=00
        out.extend_from_slice(&(chunk as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk as u16)).to_le_bytes());
        out.extend_from_slice(&data[i..i + chunk]);
        i += chunk;
    }
    if data.is_empty() {
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + byte as u32) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------

struct Args {
    game: PathBuf,
    mods: Vec<PathBuf>,
    out: PathBuf,
    width: usize,
    height: usize,
    type_arg: String,
    regions: Vec<u32>,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Self> {
        let mut game: Option<PathBuf> = None;
        let mut mods = Vec::new();
        let mut out = PathBuf::from("weather_law_dump");
        let mut width = 1408usize; // quarter map width (matches SnowMud res)
        let mut height = 512usize; // quarter map height
        let mut type_arg = "all".to_string();
        let mut regions = Vec::new();
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--game" => game = Some(PathBuf::from(next(&mut it, "--game")?)),
                "--mod" => mods.push(PathBuf::from(next(&mut it, "--mod")?)),
                "--out" => out = PathBuf::from(next(&mut it, "--out")?),
                "--width" => width = parse_usize(&next(&mut it, "--width")?)?,
                "--height" => height = parse_usize(&next(&mut it, "--height")?)?,
                "--type" => type_arg = next(&mut it, "--type")?,
                "--region" => regions.push(parse_u32(&next(&mut it, "--region")?)?),
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        let game = game.ok_or("--game <the game dir> is required")?;
        Ok(Self {
            game,
            mods,
            out,
            width,
            height,
            type_arg,
            regions,
        })
    }

    fn types(&self) -> Vec<WeatherType> {
        if self.type_arg == "all" {
            vec![
                WeatherType::RainLight,
                WeatherType::RainHeavy,
                WeatherType::Snow,
                WeatherType::Blizzard,
                WeatherType::Sandstorm,
            ]
        } else {
            WeatherType::from_key(&self.type_arg).into_iter().collect()
        }
    }
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next()
        .ok_or_else(|| format!("{flag} needs a value").into())
}

fn parse_usize(raw: &str) -> Result<usize> {
    raw.parse::<usize>()
        .map_err(|_| format!("bad usize {raw:?}").into())
}

fn parse_u32(raw: &str) -> Result<u32> {
    raw.parse::<u32>()
        .map_err(|_| format!("bad u32 {raw:?}").into())
}
