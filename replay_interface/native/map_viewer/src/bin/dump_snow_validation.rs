//! SNOW1 evidence tool: synthesize the deterministic snow floor from shipped
//! files by date and characterize it against captured `SnowMudData` payloads.
//!
//! Files-first: the floor is a deterministic function of (shipped
//! strategicregions/provinces, date); captures only VALIDATE. The above-floor
//! snow/mud field is weather-simulation state (see documents/SNOW_LAW.md) and
//! is NOT expected to match byte-for-byte — this tool reports the honest
//! residual, not a pass/fail on exact bytes.
//!
//! Usage:
//!   dump_snow_validation --game <the game dir> [--mod <content dir>]...
//!       --date MM.DD
//!       --capture <session dir> [--capture <session dir>]...
//!
//! Each `--capture` dir is scanned for a `snow_mud_data.rgba` (or a
//! `*SnowMudData*.d3d11` / `*SnowMudTexture*.d3d11` raw payload) and, if
//! present, a `map_frame_draw000_ps_constant_buffer_slot0.bin` for the
//! `SnowMudFade`. The tool prints per-channel stats and the floor residual.

use map_viewer::{
    captured_snow_mud_stats, compare_floor_to_capture, snow_mud_fade_from_cb0,
    synthesize_snow_floor_from_resolver, GameFileResolver, Result, SnowDate,
    SNOW_MUD_TEXTURE_BYTE_LEN, SNOW_MUD_TEXTURE_HEIGHT, SNOW_MUD_TEXTURE_WIDTH,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let mut game: Option<PathBuf> = None;
    let mut mods: Vec<PathBuf> = Vec::new();
    let mut captures: Vec<PathBuf> = Vec::new();
    let mut date_arg: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game" => game = args.next().map(PathBuf::from),
            "--mod" => mods.push(PathBuf::from(args.next().ok_or("--mod needs a dir")?)),
            "--date" => date_arg = args.next(),
            "--capture" => {
                captures.push(PathBuf::from(args.next().ok_or("--capture needs a dir")?))
            }
            "--help" | "-h" => {
                println!(
                    "Usage: dump_snow_validation --game <the game dir> [--mod <dir>]... \
                     --date MM.DD --capture <session dir>..."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    let game = game.ok_or("missing --game <game dir>")?;
    let date = parse_date(date_arg.as_deref().ok_or("missing --date MM.DD")?)?;
    let resolver = if mods.is_empty() {
        GameFileResolver::vanilla(game)?
    } else {
        GameFileResolver::with_mod_paths(game, mods)?
    };

    println!("========================================================================");
    println!(
        "SNOW1 deterministic snow-floor synthesis for date {:02}.{:02}",
        date.month, date.day
    );
    println!("========================================================================");

    let floor = synthesize_snow_floor_from_resolver(date, &resolver)?;
    let floored: usize = floor.floor.iter().filter(|&&f| f > 0.0).count();
    let max_floor = floor.floor.iter().cloned().fold(0.0f32, f32::max);
    println!(
        "floor texture {}x{}: {} texels with floor>0 ({:.3}%), max floor={:.3}, {} regions had no period for this date (fail-closed skip)",
        floor.width,
        floor.height,
        floored,
        100.0 * floored as f32 / (floor.width * floor.height) as f32,
        max_floor,
        floor.regions_without_period,
    );

    if captures.is_empty() {
        println!("(no --capture given; floor synthesized only)");
        return Ok(());
    }

    for capture in &captures {
        println!("------------------------------------------------------------------------");
        println!("capture: {}", capture.display());
        let Some(payload_path) = find_snow_mud_payload(capture) else {
            println!(
                "  no SnowMudData payload found (looked for snow_mud_data.rgba / *SnowMud*.d3d11)"
            );
            continue;
        };
        let payload = fs::read(&payload_path)?;
        println!(
            "  payload: {} ({} bytes)",
            payload_path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            payload.len()
        );
        if payload.len() != SNOW_MUD_TEXTURE_BYTE_LEN {
            println!(
                "  WARNING: payload {} != expected {} ({}x{}x4); skipping",
                payload.len(),
                SNOW_MUD_TEXTURE_BYTE_LEN,
                SNOW_MUD_TEXTURE_WIDTH,
                SNOW_MUD_TEXTURE_HEIGHT
            );
            continue;
        }

        let stats =
            captured_snow_mud_stats(&payload, SNOW_MUD_TEXTURE_WIDTH, SNOW_MUD_TEXTURE_HEIGHT)?;
        let pct = |n: usize| 100.0 * n as f32 / stats.total_texels as f32;
        println!(
            "  channels (nonzero%/max): snow_fade0(.b) {:.2}%/{}  snow_fade1(.g) {:.2}%/{}  mud_fade0(.r) {:.2}%/{}  mud_fade1(.a) {:.2}%/{}",
            pct(stats.snow_fade0_nonzero),
            stats.snow_fade0_max,
            pct(stats.snow_fade1_nonzero),
            stats.snow_fade1_max,
            pct(stats.mud_fade0_nonzero),
            stats.mud_fade0_max,
            pct(stats.mud_fade1_nonzero),
            stats.mud_fade1_max,
        );

        let fade = read_snow_mud_fade(capture);
        match fade {
            Some(f) => println!("  SnowMudFade (CB0 offset 248): {f}"),
            None => println!("  SnowMudFade: no CB0 found; assuming 0.0"),
        }
        let fade = fade.unwrap_or(0.0);

        let residual = compare_floor_to_capture(&floor, &payload, fade)?;
        println!(
            "  floor residual: {} floored texels; {} respected ({:.2}%), {} violations, max shortfall={:.3}",
            residual.floored_texels,
            residual.floor_respected,
            residual.floor_respected_percent(),
            residual.floor_violations,
            residual.max_floor_shortfall,
        );
        println!(
            "  exact displayed-snow byte match: {}/{} ({:.3}%); mean abs snow diff (floor vs displayed) over all texels: {:.5}",
            residual.exact_snow_byte_matches,
            residual.total_texels,
            residual.exact_match_percent(),
            residual.mean_abs_snow_diff,
        );
    }

    println!("------------------------------------------------------------------------");
    println!(
        "NOTE: exact byte match is expected to be LOW. The floor is a deterministic lower bound; \
         the above-floor snow/mud field is accumulated weather-sim state (common/weather.txt) and \
         is NOT files+date derivable. `violations` (capture below the deterministic floor) is the \
         only refutation signal for the floor law and should be ~0 where regions have periods."
    );
    Ok(())
}

fn parse_date(s: &str) -> Result<SnowDate> {
    let (m, d) = s
        .split_once('.')
        .ok_or("--date must be MM.DD (e.g. 01.15)")?;
    let month = m.parse::<u8>().map_err(|e| format!("bad month: {e}"))?;
    let day = d.parse::<u8>().map_err(|e| format!("bad day: {e}"))?;
    SnowDate::new(month, day)
}

/// Find a raw `SnowMudData` payload in a capture dir: prefer the decoded
/// `snow_mud_data.rgba`, else any `*SnowMudData*.d3d11` / `*SnowMudTexture*.d3d11`
/// with the exact expected byte length.
fn find_snow_mud_payload(dir: &Path) -> Option<PathBuf> {
    let rgba = dir.join("snow_mud_data.rgba");
    if rgba.is_file() {
        return Some(rgba);
    }
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_d3d11 = name.ends_with(".d3d11");
        let mentions = name.contains("SnowMudData") || name.contains("SnowMudTexture");
        if is_d3d11 && mentions {
            if let Ok(meta) = fs::metadata(&path) {
                if meta.len() as usize == SNOW_MUD_TEXTURE_BYTE_LEN {
                    candidates.push(path);
                }
            }
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// Read `SnowMudFade` from a capture's scene CB0, trying the common file names.
fn read_snow_mud_fade(dir: &Path) -> Option<f32> {
    for name in [
        "map_frame_draw000_ps_constant_buffer_slot0.bin",
        "water_frame_draw000_ps_constant_buffer_slot0.bin",
        "terrain_chunk_draw000_ps_constant_buffer_slot0.bin",
    ] {
        let p = dir.join(name);
        if let Ok(bytes) = fs::read(&p) {
            if let Some(f) = snow_mud_fade_from_cb0(&bytes) {
                return Some(f);
            }
        }
    }
    None
}
