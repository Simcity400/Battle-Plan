//! WATER5 decode bin: settle the underwater light-composition law from the
//! FIRST capture of the engine's OWN underwater pass (`session_CAPFIX2_GAPS_4`).
//!
//! It decodes the scene CB0 (slot 0, 416 B) bound to the engine's underwater PS
//! draws, compares it field-by-field against the SAME session's terrain scene
//! CB0 (`cb0_samples.jsonl`, matched by wall-clock / `vGlobalTime`), and reports
//! the light-color delta table plus a machine-readable JSON dump.
//!
//! The headline result: the engine binds the scene light rig UNCHANGED to the
//! underwater pass (white sun `(1,1,1,1)`, day-night gate OFF); the sun/moon
//! rgb ratio is `1.0` on every channel — the engine applies NO CB0 light-color
//! absorption. This DISPROVES the WATER3 hypothesis that
//! `water_law::UNDERWATER_LIGHT_ABSORPTION` is an engine CB0 light-state law.
//! See `documents/WATER_LAW.md` §2a-ter and `map_viewer/src/water_law.rs`.
//!
//! Files-first: the capture is VALIDATION of the law; no installed content are needed
//! (the law here is "the engine changes nothing", proven purely from the two
//! captured CB0s). hoi4_captures is READ-ONLY evidence.
//!
//! Usage:
//!   decode_underwater_state --session <capture session dir> [--out <dir>]
//!
//! The session dir must contain
//! `underwater_frame_draw000_ps_constant_buffer_slot0.bin` (the engine
//! underwater PS CB0) and `cb0_samples.jsonl` (the terrain scene CB0 stream).

use map_viewer::{
    light_offsets, underwater_cb0_absorption_ratio, SceneLighting, UnderwaterLightDelta,
    UNDERWATER_LIGHT_ABSORPTION,
};
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const UNDERWATER_CB0_A: &str = "underwater_frame_draw000_ps_constant_buffer_slot0.bin";
const UNDERWATER_CB0_B: &str = "underwater_frame_draw004_ps_constant_buffer_slot0.bin";
const SCENE_SAMPLES: &str = "cb0_samples.jsonl";

fn main() -> Result<()> {
    let args = Args::parse(env::args().skip(1))?;

    // --- The engine's underwater pass CB0 (draws 0-3 share one; 4-9 another). ---
    let uw_a = fs::read(args.session.join(UNDERWATER_CB0_A))?;
    if uw_a.len() < light_offsets::CB0_LEN {
        return Err(format!(
            "decode blocked: {UNDERWATER_CB0_A} is {} bytes, need {}",
            uw_a.len(),
            light_offsets::CB0_LEN
        )
        .into());
    }
    let uw_b = fs::read(args.session.join(UNDERWATER_CB0_B)).ok();

    // --- The terrain scene CB0 stream; pick the sample closest in wall-clock. ---
    let scene = pick_matching_scene_cb0(&args.session.join(SCENE_SAMPLES), &uw_a)?;

    let uw_light = SceneLighting::from_cb0_bytes(&uw_a).ok_or("underwater CB0 too short")?;
    let scene_light = SceneLighting::from_cb0_bytes(&scene.cb0).ok_or("scene CB0 too short")?;
    let delta = underwater_cb0_absorption_ratio(&uw_a, &scene.cb0)
        .ok_or("decode blocked: a CB0 is shorter than the layout")?;

    // Cross-check the second underwater CB0 group carries the same light rig.
    let group_b_same_light = uw_b
        .as_ref()
        .and_then(|b| underwater_cb0_absorption_ratio(b, &scene.cb0))
        .map(|d| d.underwater_sun_rgb == delta.underwater_sun_rgb)
        .unwrap_or(false);

    // --- Console delta table. ---
    println!(
        "WATER5 underwater light-composition decode ({})",
        args.session.display()
    );
    println!("  engine underwater PS CB0: {UNDERWATER_CB0_A}");
    println!(
        "  matched scene sample: idx {} wall_ms {} (|dGlobalTime| = {:.3})",
        scene.sample_index, scene.wall_ms, scene.global_time_delta
    );
    println!();
    println!("  field                        underwater            scene(terrain)        ratio(uw/scene)");
    print_rgb_row(
        "SunDiffuseIntensity.rgb",
        delta.underwater_sun_rgb,
        delta.scene_sun_rgb,
        delta.sun_rgb_ratio,
    );
    print_rgb_row(
        "MoonDiffuseIntensity.rgb",
        delta.underwater_moon_rgb,
        delta.scene_moon_rgb,
        delta.moon_rgb_ratio,
    );
    println!(
        "  {:<26} {:>10.4}            {:>10.4}            {}",
        "SunDiffuseIntensity.a",
        uw_light.sun_diffuse_intensity[3],
        scene_light.sun_diffuse_intensity[3],
        ratio1(
            uw_light.sun_diffuse_intensity[3],
            scene_light.sun_diffuse_intensity[3]
        ),
    );
    println!(
        "  {:<26} {:>10.4}            {:>10.4}            {}",
        "MoonDiffuseIntensity.a",
        uw_light.moon_diffuse_intensity[3],
        scene_light.moon_diffuse_intensity[3],
        ratio1(
            uw_light.moon_diffuse_intensity[3],
            scene_light.moon_diffuse_intensity[3]
        ),
    );
    println!(
        "  {:<26} {:>10.4}            {:>10.4}",
        "FoW.w (day-night gate)",
        uw_light.fow_opacity_time_snowmud_maxspeed[3],
        scene_light.fow_opacity_time_snowmud_maxspeed[3],
    );
    println!(
        "  {:<26} {:>10.4}            {:>10.4}",
        "SunSpecularIntensity", uw_light.sun_specular_intensity, scene_light.sun_specular_intensity
    );
    println!(
        "  {:<26} {:>10.4}            {:>10.4}",
        "CubemapIntensity", uw_light.cubemap_intensity, scene_light.cubemap_intensity
    );
    println!();
    println!(
        "  day-night gate (underwater FoW.w): {} ({})",
        uw_light.fow_opacity_time_snowmud_maxspeed[3],
        if uw_light.is_full_daylight_capture() {
            "day-gated / full daylight"
        } else {
            "night blend active"
        }
    );
    println!("  second underwater CB0 group (draws 4-9) same light rig: {group_b_same_light}");
    println!();
    let unabsorbed = delta.is_unabsorbed(1e-6);
    println!(
        "  CONCLUSION: engine applies {} to the underwater pass CB0.",
        if unabsorbed {
            "NO light-color absorption (sun/moon ratio == 1.0)"
        } else {
            "a light-color change (see ratios)"
        }
    );
    println!(
        "  => water_law::UNDERWATER_LIGHT_ABSORPTION = {:?} is NOT the engine's CB0 light law",
        UNDERWATER_LIGHT_ABSORPTION
    );
    println!("     (it is an output-matching correction; see WATER_LAW.md §2a-ter).");

    // --- JSON dump. ---
    let out_dir = args
        .out
        .clone()
        .unwrap_or_else(|| args.session.join("underwater_state_decode"));
    fs::create_dir_all(&out_dir)?;
    let report = DecodeReport {
        session: args.session.display().to_string(),
        underwater_cb0_file: UNDERWATER_CB0_A.to_string(),
        matched_scene_sample_index: scene.sample_index,
        matched_scene_wall_ms: scene.wall_ms,
        matched_global_time_delta: scene.global_time_delta,
        engine_applies_absorption: !unabsorbed,
        underwater_light_absorption_constant: UNDERWATER_LIGHT_ABSORPTION,
        delta: DeltaJson::from(&delta),
        second_group_same_light: group_b_same_light,
    };
    let json = serde_json::to_string_pretty(&report)?;
    let json_path = out_dir.join("underwater_state_decode.json");
    fs::write(&json_path, &json)?;
    println!("  JSON: {}", json_path.display());
    Ok(())
}

struct MatchedScene {
    cb0: Vec<u8>,
    sample_index: u64,
    wall_ms: u64,
    global_time_delta: f32,
}

/// Read `cb0_samples.jsonl` and return the sample whose `vGlobalTime` (CB0 byte
/// 236) is closest to the underwater CB0's — the same-frame terrain scene state.
fn pick_matching_scene_cb0(path: &Path, underwater_cb0: &[u8]) -> Result<MatchedScene> {
    let text = fs::read_to_string(path)?;
    let uw_gt = read_global_time(underwater_cb0);
    let mut best: Option<MatchedScene> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(line)?;
        let hex = v
            .get("cb0_hex")
            .and_then(|h| h.as_str())
            .ok_or("scene sample has no cb0_hex")?;
        let cb0 = hex_to_bytes(hex)?;
        if cb0.len() < light_offsets::CB0_LEN {
            continue;
        }
        let gt = read_global_time(&cb0);
        let d = (gt - uw_gt).abs();
        let sample_index = v.get("sample_index").and_then(|s| s.as_u64()).unwrap_or(0);
        let wall_ms = v.get("wall_ms").and_then(|s| s.as_u64()).unwrap_or(0);
        if best
            .as_ref()
            .map(|b| d < b.global_time_delta)
            .unwrap_or(true)
        {
            best = Some(MatchedScene {
                cb0,
                sample_index,
                wall_ms,
                global_time_delta: d,
            });
        }
    }
    best.ok_or_else(|| "decode blocked: no usable scene CB0 samples".into())
}

fn read_global_time(cb0: &[u8]) -> f32 {
    let at = light_offsets::GLOBAL_TIME;
    f32::from_le_bytes([cb0[at], cb0[at + 1], cb0[at + 2], cb0[at + 3]])
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return Err("odd-length hex".into());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|e| e.into()))
        .collect()
}

fn ratio1(num: f32, den: f32) -> String {
    if den != 0.0 {
        format!("{:>8.4}", num / den)
    } else {
        "    n/a".to_string()
    }
}

fn print_rgb_row(name: &str, uw: [f32; 3], scene: [f32; 3], ratio: [Option<f32>; 3]) {
    let fmt_r = |r: Option<f32>| r.map(|v| format!("{v:.4}")).unwrap_or_else(|| "n/a".into());
    println!(
        "  {name:<26} ({:.3},{:.3},{:.3})   ({:.3},{:.3},{:.3})   ({},{},{})",
        uw[0],
        uw[1],
        uw[2],
        scene[0],
        scene[1],
        scene[2],
        fmt_r(ratio[0]),
        fmt_r(ratio[1]),
        fmt_r(ratio[2]),
    );
}

struct Args {
    session: PathBuf,
    out: Option<PathBuf>,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Self> {
        let mut session = None;
        let mut out = None;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--session" | "--bundle" => session = it.next().map(PathBuf::from),
                "--out" => out = it.next().map(PathBuf::from),
                "--help" | "-h" => {
                    println!(
                        "Usage: decode_underwater_state --session <session_CAPFIX2_GAPS_4 dir> [--out <dir>]"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument {other}").into()),
            }
        }
        Ok(Self {
            session: session.ok_or("missing --session <capture session dir>")?,
            out,
        })
    }
}

#[derive(Serialize)]
struct DecodeReport {
    session: String,
    underwater_cb0_file: String,
    matched_scene_sample_index: u64,
    matched_scene_wall_ms: u64,
    matched_global_time_delta: f32,
    engine_applies_absorption: bool,
    underwater_light_absorption_constant: [f32; 3],
    delta: DeltaJson,
    second_group_same_light: bool,
}

#[derive(Serialize)]
struct DeltaJson {
    underwater_sun_rgb: [f32; 3],
    scene_sun_rgb: [f32; 3],
    sun_rgb_ratio: [Option<f32>; 3],
    underwater_moon_rgb: [f32; 3],
    scene_moon_rgb: [f32; 3],
    moon_rgb_ratio: [Option<f32>; 3],
    underwater_day_gate: f32,
    is_unabsorbed: bool,
}

impl From<&UnderwaterLightDelta> for DeltaJson {
    fn from(d: &UnderwaterLightDelta) -> Self {
        DeltaJson {
            underwater_sun_rgb: d.underwater_sun_rgb,
            scene_sun_rgb: d.scene_sun_rgb,
            sun_rgb_ratio: d.sun_rgb_ratio,
            underwater_moon_rgb: d.underwater_moon_rgb,
            scene_moon_rgb: d.scene_moon_rgb,
            moon_rgb_ratio: d.moon_rgb_ratio,
            underwater_day_gate: d.underwater_day_gate,
            is_unabsorbed: d.is_unabsorbed(1e-6),
        }
    }
}
