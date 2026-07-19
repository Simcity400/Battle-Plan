//! NIGHT1 night-look dump strip (GPU-free, deterministic).
//!
//! Renders the day→night terminator across a full day using the SHIPPED the game
//! shader math (`gfx/FX/standardfuncsgfx.fxh`): `CalcGlobeNormal` (the hour goes
//! in via `DayNight_Hour_SunDir.x`), `DayNightFactor` (dot vs the hour-INVARIANT
//! sun direction `.yzw`, per the NIGHT1 finding in `light_law::render_sun_dir_yzw`),
//! and `DayNightWithBlend`/`NightifyColor` (the night recolor + `NIGHT_DARKNESS`
//! darken), plus the `DayNightCityMask` city-light additive term. It writes one
//! BMP per hour row plus a stacked strip so a human can eyeball that:
//!   * the night band sweeps east→west as the hour advances (via `.x`),
//!   * the sun-direction `.yzw` is held fixed across hours (NIGHT1 law), and
//!   * deep-night hours darken + recolor (not a plain ×0.5) and light up cities.
//!
//! This deliberately reproduces the shader formulas INLINE (they are short and
//! file-exact) rather than reaching into the `pub(crate)` cpu_terrain port, so
//! the bin stays in this lane's surface. The constants are the shipped values
//! mirrored in `light_law::defines`; a drift there fails the light_law tests.
//!
//! Usage:
//!   dump_night_strip --out <dir> [--hours <N>] [--width <px>] [--fow <0..1>]
//!
//! `--fow` is the `FoW.w` night gate (default 1.0 to EXERCISE night; the real
//! captures all carry 0.0, i.e. full daylight — see LIGHT_LAW.md §5).

use map_viewer::{light_defines as defines, render_sun_dir_yzw, Result};
use std::env;
use std::fs;
use std::path::PathBuf;

#[path = "dump_shared/bmp_out.rs"]
mod bmp_out;

// --- shipped constants (mirrored from standardfuncsgfx.fxh via light_defines) ---
const SOUTH_POLE_OFFSET: f32 = 0.17;
const NORTH_POLE_OFFSET: f32 = 0.93;
const MAP_SIZE_Y: f32 = 2048.0; // shipped heightmap.bmp height
const CITY_LIGHTS_RGB: [f32; 3] = [2.0, 2.0, 0.3]; // DayNightCityMask (fxh)

#[derive(Clone, Copy)]
struct V3 {
    x: f32,
    y: f32,
    z: f32,
}

fn norm(v: V3) -> V3 {
    let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt().max(1e-20);
    V3 {
        x: v.x / l,
        y: v.y / l,
        z: v.z / l,
    }
}

fn dot(a: V3, b: V3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

fn saturate(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

fn fmod_loop(x: f32, m: f32) -> f32 {
    x.rem_euclid(m)
}

/// `CalcGlobeNormal` (standardfuncsgfx.fxh:347). The hour enters via
/// `hour_phase = DayNight_Hour_SunDir.x`.
fn calc_globe_normal(world_x: f32, world_z: f32, hour_phase: f32) -> V3 {
    let x = fmod_loop(
        (world_x - defines::GMT_OFFSET) / defines::MAP_SIZE_X + hour_phase,
        1.0,
    );
    let mut y = world_z / MAP_SIZE_Y;
    y = SOUTH_POLE_OFFSET + (NORTH_POLE_OFFSET - SOUTH_POLE_OFFSET) * y;
    y = -(y * 3.1415).cos();
    let xz_len = 1.0 - y.abs();
    norm(V3 {
        x: (x * 6.2831).sin() * xz_len,
        y,
        z: (x * 6.2831).cos() * xz_len,
    })
}

/// `DayNightFactor` (fxh:358), gated by `FoW.w`.
fn day_night_factor(globe: V3, sun_dir: V3, fow_w: f32) -> f32 {
    let vdot = dot(globe, sun_dir);
    saturate((vdot - defines::FEATHER_MIN) / (defines::FEATHER_MAX - defines::FEATHER_MIN)) * fow_w
}

/// `NightifyColor` (fxh:381) — desaturate + recolor + `NIGHT_DARKNESS` darken.
fn nightify_color(day: V3, blend: f32) -> V3 {
    let desat = 0.0f32 + (0.8 - 0.0) * (blend * blend * blend);
    let grey = day.x * 0.4 + day.y * 0.3 + day.z * 0.05;
    let night_color = V3 {
        x: saturate(grey + (grey * 0.2 - grey) * 0.25),
        y: saturate(grey + (grey * 0.7 - grey) * 0.25),
        z: saturate(grey + (grey * 1.2 - grey) * 0.25),
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    V3 {
        x: lerp(day.x, night_color.x, desat) * defines::NIGHT_DARKNESS,
        y: lerp(day.y, night_color.y, desat) * defines::NIGHT_DARKNESS,
        z: lerp(day.z, night_color.z, desat) * defines::NIGHT_DARKNESS,
    }
}

/// `DayNightCityMask` (fxh:401): day→night lerp by `DayNightFactor*NIGHT_OPACITY`
/// then additive city lights scaled by the night factor.
fn day_night_city_mask(day: V3, globe: V3, sun_dir: V3, fow_w: f32, city_mask: f32) -> V3 {
    let nf = day_night_factor(globe, sun_dir, fow_w);
    let night = nightify_color(day, 0.0);
    let t = nf * defines::NIGHT_OPACITY;
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut c = V3 {
        x: lerp(day.x, night.x, t),
        y: lerp(day.y, night.y, t),
        z: lerp(day.z, night.z, t),
    };
    // fog factor 0 here (clear); Result += cityMask * (2,2,0.3) * nf.
    c.x += city_mask * CITY_LIGHTS_RGB[0] * nf;
    c.y += city_mask * CITY_LIGHTS_RGB[1] * nf;
    c.z += city_mask * CITY_LIGHTS_RGB[2] * nf;
    c
}

fn to_u8(x: f32) -> u8 {
    (saturate(x) * 255.0 + 0.5) as u8
}

fn main() -> Result<()> {
    let mut out_dir: Option<PathBuf> = None;
    let mut hours = 24usize;
    let mut width = 360usize;
    let mut fow_w = 1.0f32;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out_dir = Some(PathBuf::from(args.next().ok_or("--out needs a dir")?)),
            "--hours" => {
                hours = args
                    .next()
                    .ok_or("--hours N")?
                    .parse()
                    .map_err(|_| "bad --hours")?
            }
            "--width" => {
                width = args
                    .next()
                    .ok_or("--width N")?
                    .parse()
                    .map_err(|_| "bad --width")?
            }
            "--fow" => {
                fow_w = args
                    .next()
                    .ok_or("--fow F")?
                    .parse()
                    .map_err(|_| "bad --fow")?
            }
            "--help" | "-h" => {
                println!(
                    "Usage: dump_night_strip --out <dir> [--hours N] [--width px] [--fow 0..1]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let out_dir = out_dir.ok_or("no --out <dir> supplied")?;
    fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;

    // The hour-INVARIANT sun direction (NIGHT1). Use the noon-baseline captured
    // .yzw (live_PAN2): (dir.x=0, dir.y=-0.0783, dir.z=0.9969). render_sun_dir_yzw
    // echoes it at every hour — this is the whole point of the law.
    let captured_yzw = [0.0f32, -0.078_338_5, 0.996_926_84];

    let strip_row_h = 10usize; // pixels per hour row in the stacked strip
    let mut strip: Vec<[u8; 3]> = vec![[0, 0, 0]; width * hours * strip_row_h];
    // A day base color (mid green-grey terrain) + a few "city" columns.
    let day = V3 {
        x: 0.32,
        y: 0.40,
        z: 0.22,
    };
    let city_col = |x: usize| -> f32 {
        // sparse city pixels every ~47 px
        if x % 47 == 3 {
            1.0
        } else {
            0.0
        }
    };

    for h in 0..hours {
        let hour_frac = h as f32 / hours as f32; // DayNight_Hour_SunDir.x
        let hour24 = hour_frac * 24.0;
        // NIGHT1: sun dir is hour-invariant; prove it by routing through the law.
        let yzw = render_sun_dir_yzw(hour24, captured_yzw);
        let sun_dir = V3 {
            x: yzw[0],
            y: yzw[1],
            z: yzw[2],
        };
        let world_z = MAP_SIZE_Y * 0.5; // equator band
        let mut row: Vec<[u8; 3]> = Vec::with_capacity(width);
        for x in 0..width {
            let world_x = (x as f32 / width as f32) * defines::MAP_SIZE_X;
            let globe = calc_globe_normal(world_x, world_z, hour_frac);
            let c = day_night_city_mask(day, globe, sun_dir, fow_w, city_col(x));
            row.push([to_u8(c.x), to_u8(c.y), to_u8(c.z)]);
        }
        // Per-hour BMP.
        bmp_out::write_bmp24_rgb(
            &out_dir.join(format!("night_hour_{:02}_{:04.1}h.bmp", h, hour24)),
            width,
            1,
            &row,
        )?;
        // Stack into the strip (top row = hour 0).
        for r in 0..strip_row_h {
            let sy = h * strip_row_h + r;
            for x in 0..width {
                strip[sy * width + x] = row[x];
            }
        }
    }
    bmp_out::write_bmp24_rgb(
        &out_dir.join("night_strip.bmp"),
        width,
        hours * strip_row_h,
        &strip,
    )?;
    eprintln!(
        "wrote {} per-hour rows + night_strip.bmp ({}x{}) to {} (fow_w={})",
        hours,
        width,
        hours * strip_row_h,
        out_dir.display(),
        fow_w
    );
    Ok(())
}
