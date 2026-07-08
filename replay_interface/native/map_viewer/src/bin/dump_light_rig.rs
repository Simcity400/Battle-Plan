//! LIGHT1 CB0-mining bin: walk capture session dirs, decode every scene
//! constant-buffer 0 (CB0), and tabulate the light-rig fields (sun/moon/second
//! virtual positions, day-night driver, camera, intensities) per bundle. Fits
//! the light-rig law evidence for `documents/LIGHT_LAW.md`.
//!
//! Files-first: the CB0 payloads are law-fitting evidence, never runtime
//! sources. This bin reads only captured `*_ps_constant_buffer_slot0.bin`
//! payloads (416-byte scene CB0s carrying `vVirtualSunPos` etc.) and their
//! bundle manifests for provenance.
//!
//! Determinism: sessions and files are visited in sorted order and the JSON/
//! markdown output is a pure function of the inputs, so two runs are identical.
//! The table is written to `--out <dir>` (default: a scratch dir); capture
//! payloads are never emitted.
//!
//! Usage:
//!   dump_light_rig --session <dir> [--session <dir>]... [--out <dir>] [--md]
//!
//! Each `--session` is a capture session directory (e.g.
//!   C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2). Directories with no
//! scene CB0 payloads are reported and skipped.

use map_viewer::{light_offsets, validate_lighting_law_against_cb0, Result, SceneLighting};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// One mined scene CB0 row.
#[derive(Clone, Debug)]
struct Row {
    session: String,
    bundle_kind: String,
    file: String,
    lighting: SceneLighting,
}

fn main() -> Result<()> {
    let mut sessions: Vec<PathBuf> = Vec::new();
    let mut out_dir: Option<PathBuf> = None;
    let mut emit_md = false;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session" => sessions.push(PathBuf::from(
                args.next().ok_or("--session needs a directory")?,
            )),
            "--out" => out_dir = Some(PathBuf::from(args.next().ok_or("--out needs a dir")?)),
            "--md" => emit_md = true,
            "--help" | "-h" => {
                println!(
                    "Usage: dump_light_rig --session <dir> [--session <dir>]... [--out <dir>] [--md]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if sessions.is_empty() {
        return Err("no --session <dir> supplied".into());
    }
    // Deterministic session order.
    sessions.sort();
    sessions.dedup();

    let mut rows: Vec<Row> = Vec::new();
    let mut empty_sessions: Vec<String> = Vec::new();
    for session in &sessions {
        let name = session
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| session.display().to_string());
        let before = rows.len();
        mine_session(session, &name, &mut rows)?;
        if rows.len() == before {
            empty_sessions.push(name);
        }
    }
    // Deterministic row order independent of filesystem enumeration.
    rows.sort_by(|a, b| {
        (a.session.as_str(), a.bundle_kind.as_str(), a.file.as_str()).cmp(&(
            b.session.as_str(),
            b.bundle_kind.as_str(),
            b.file.as_str(),
        ))
    });

    let json = build_json(&rows, &empty_sessions);
    let markdown = build_markdown(&rows);

    // Print a compact summary + the field-variation table to stdout so the bin
    // is useful without an --out dir. The full JSON goes to --out (or scratch).
    println!(
        "mined {} scene CB0 rows from {} session dir(s) ({} had no scene CB0)",
        rows.len(),
        sessions.len(),
        empty_sessions.len()
    );
    print_variation_summary(&rows);

    let out = out_dir.unwrap_or_else(default_out_dir);
    fs::create_dir_all(&out)?;
    let json_path = out.join("light_rig_table.json");
    fs::write(&json_path, serde_json::to_vec_pretty(&json)?)?;
    println!("wrote {}", json_path.display());
    if emit_md {
        let md_path = out.join("light_rig_table.md");
        fs::write(&md_path, markdown)?;
        println!("wrote {}", md_path.display());
    }
    Ok(())
}

fn default_out_dir() -> PathBuf {
    env::temp_dir().join("cdx_light1_mine")
}

/// Enumerate a session's scene CB0 payloads (any `*_ps_constant_buffer_slot0`
/// that decodes as a 416-byte scene CB0), decode them, and append rows.
fn mine_session(session: &Path, name: &str, rows: &mut Vec<Row>) -> Result<()> {
    if !session.is_dir() {
        eprintln!(
            "warning: session {} is not a directory; skipping",
            session.display()
        );
        return Ok(());
    }
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(session)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.ends_with("_ps_constant_buffer_slot0.bin") {
            files.push((fname, entry.path()));
        }
    }
    // Deterministic file order.
    files.sort();
    for (fname, path) in files {
        let bytes = fs::read(&path)?;
        // Scene CB0 signature: full 416-byte layout whose vVirtualSunPos carries
        // the shipped SUN_HEIGHT (600) in .y and SUN_HEIGHT_WATER (5000) in .w.
        // This excludes the many non-scene slot-0 CBs (post-effect, UI, etc.).
        if bytes.len() < light_offsets::CB0_LEN {
            continue;
        }
        let Some(lighting) = SceneLighting::from_cb0_bytes(&bytes) else {
            continue;
        };
        if !is_scene_cb0(&lighting) {
            continue;
        }
        rows.push(Row {
            session: name.to_string(),
            bundle_kind: bundle_kind_of(&fname),
            file: fname,
            lighting,
        });
    }
    Ok(())
}

/// The scene CB0 fingerprint: the shipped sun-height defines are present. Any
/// non-scene slot-0 CB (which does not carry `vVirtualSunPos`) fails this.
fn is_scene_cb0(l: &SceneLighting) -> bool {
    (l.virtual_sun_pos[1] - 600.0).abs() < 1.0
        && (l.virtual_sun_pos[3] - 5000.0).abs() < 1.0
        && (l.virtual_moon_pos[1] - 600.0).abs() < 1.0
}

/// Classify a payload file into its bundle kind from the file-name prefix.
fn bundle_kind_of(fname: &str) -> String {
    for kind in ["water", "rivers", "trees", "terrain_chunk"] {
        if fname.starts_with(&format!("{kind}_frame")) {
            return kind.to_string();
        }
    }
    // `map_frame_draw*` is the terrain pass.
    if fname.starts_with("map_frame") {
        return "terrain".to_string();
    }
    "unknown".to_string()
}

fn build_json(rows: &[Row], empty_sessions: &[String]) -> serde_json::Value {
    let mut entries = Vec::new();
    for r in rows {
        let l = &r.lighting;
        let res = validate_lighting_law_against_cb0(&fields_to_cb0(l)).unwrap();
        entries.push(serde_json::json!({
            "session": r.session,
            "bundle_kind": r.bundle_kind,
            "file": r.file,
            "vVirtualSunPos": l.virtual_sun_pos,
            "vVirtualMoonPos": l.virtual_moon_pos,
            "vSecondVirtualSunPos": l.second_virtual_sun_pos,
            "vSecondVirtualMoonPos": l.second_virtual_moon_pos,
            "vCamPos": l.cam_pos,
            "vCamLookAtDir": l.cam_look_at_dir,
            "vGlobalTime": l.global_time,
            "FoW_Opacity_Time_SnowMud_MaxSpeed": l.fow_opacity_time_snowmud_maxspeed,
            "DayNight_Hour_SunDir": l.day_night_hour_sun_dir,
            "day_night_hour": l.day_night_hour(),
            "is_full_daylight": l.is_full_daylight_capture(),
            "SunDiffuseIntensity": l.sun_diffuse_intensity,
            "MoonDiffuseIntensity": l.moon_diffuse_intensity,
            "SunSpecularIntensity": l.sun_specular_intensity,
            "CubemapIntensity": l.cubemap_intensity,
            "residuals": {
                "sun_height": res.sun_height_residual,
                "sun_latitude": res.sun_latitude_residual,
                "moon_height": res.moon_height_residual,
                "moon_z_hypothesis": res.moon_z_hypothesis_residual,
                "sun_diffuse_rgb": res.sun_diffuse_rgb_residual,
                "moon_diffuse_rgb": res.moon_diffuse_rgb_residual,
                "sun_dir_unit": res.sun_dir_unit_residual,
                "sun_x_minus_cam_x": res.sun_x_minus_cam_x,
            },
        }));
    }
    serde_json::json!({
        "schema": "hoi4-light-rig-mining-v1",
        "row_count": rows.len(),
        "sessions_without_scene_cb0": empty_sessions,
        "rows": entries,
    })
}

/// Re-pack a decoded [`SceneLighting`] back into a CB0 buffer so the shared
/// residual validator runs on exactly the fields we tabulate.
fn fields_to_cb0(l: &SceneLighting) -> Vec<u8> {
    let mut cb0 = vec![0u8; light_offsets::CB0_LEN];
    let mut put4 = |off: usize, v: [f32; 4]| {
        for (i, c) in v.iter().enumerate() {
            cb0[off + i * 4..off + i * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    };
    put4(light_offsets::VIRTUAL_SUN_POS, l.virtual_sun_pos);
    put4(light_offsets::VIRTUAL_MOON_POS, l.virtual_moon_pos);
    put4(
        light_offsets::SECOND_VIRTUAL_SUN_POS,
        l.second_virtual_sun_pos,
    );
    put4(
        light_offsets::SECOND_VIRTUAL_MOON_POS,
        l.second_virtual_moon_pos,
    );
    put4(
        light_offsets::CAM_POS,
        [l.cam_pos[0], l.cam_pos[1], l.cam_pos[2], 0.0],
    );
    put4(
        light_offsets::DAY_NIGHT_HOUR_SUN_DIR,
        l.day_night_hour_sun_dir,
    );
    put4(
        light_offsets::SUN_DIFFUSE_INTENSITY,
        l.sun_diffuse_intensity,
    );
    put4(
        light_offsets::MOON_DIFFUSE_INTENSITY,
        l.moon_diffuse_intensity,
    );
    let mut put1 = |off: usize, v: f32| {
        cb0[off..off + 4].copy_from_slice(&v.to_le_bytes());
    };
    put1(
        light_offsets::SUN_SPECULAR_INTENSITY,
        l.sun_specular_intensity,
    );
    put1(light_offsets::CUBEMAP_INTENSITY, l.cubemap_intensity);
    cb0
}

fn build_markdown(rows: &[Row]) -> String {
    let mut s = String::new();
    s.push_str("# Mined scene CB0 light-rig table\n\n");
    s.push_str(
        "| session | kind | sun.x | sun.z | moon.z | cam.x | DayNight.x(hr) | DayNight.yzw | globalTime | FoW.w |\n",
    );
    s.push_str("|---|---|---|---|---|---|---|---|---|---|\n");
    for r in rows {
        let l = &r.lighting;
        let d = l.day_night_hour_sun_dir;
        s.push_str(&format!(
            "| {} | {} | {:.1} | {:.1} | {:.1} | {:.1} | {:.4} ({:.2}) | ({:.4},{:.4},{:.4}) | {:.3} | {:.1} |\n",
            r.session,
            r.bundle_kind,
            l.virtual_sun_pos[0],
            l.virtual_sun_pos[2],
            l.virtual_moon_pos[2],
            l.cam_pos[0],
            d[0],
            l.day_night_hour(),
            d[1],
            d[2],
            d[3],
            l.global_time,
            l.fow_opacity_time_snowmud_maxspeed[3],
        ));
    }
    s
}

/// Print, per distinct value, how many rows share it — the core evidence for
/// which fields are constant vs vary. Deterministic (BTreeMap ordering).
fn print_variation_summary(rows: &[Row]) {
    let key3 = |v: [f32; 3]| format!("({:.4}, {:.4}, {:.4})", v[0], v[1], v[2]);
    let key4 = |v: [f32; 4]| format!("({:.4}, {:.4}, {:.4}, {:.4})", v[0], v[1], v[2], v[3]);
    let mut fields: Vec<(&str, Box<dyn Fn(&Row) -> String>)> = Vec::new();
    fields.push((
        "sun.x",
        Box::new(|r: &Row| format!("{:.2}", r.lighting.virtual_sun_pos[0])),
    ));
    fields.push((
        "sun.z(lat)",
        Box::new(|r: &Row| format!("{:.2}", r.lighting.virtual_sun_pos[2])),
    ));
    fields.push((
        "moon.z",
        Box::new(|r: &Row| format!("{:.2}", r.lighting.virtual_moon_pos[2])),
    ));
    fields.push((
        "cam.x",
        Box::new(|r: &Row| format!("{:.2}", r.lighting.cam_pos[0])),
    ));
    fields.push((
        "DayNight_Hour_SunDir",
        Box::new(move |r: &Row| key4(r.lighting.day_night_hour_sun_dir)),
    ));
    fields.push((
        "SunDiffuseIntensity",
        Box::new(move |r: &Row| key4(r.lighting.sun_diffuse_intensity)),
    ));
    fields.push((
        "MoonDiffuseIntensity",
        Box::new(move |r: &Row| key4(r.lighting.moon_diffuse_intensity)),
    ));
    fields.push((
        "cam_look_at_dir",
        Box::new(move |r: &Row| key3(r.lighting.cam_look_at_dir)),
    ));
    for (label, keyfn) in &fields {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for r in rows {
            *counts.entry(keyfn(r)).or_default() += 1;
        }
        let mut v: Vec<_> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        println!("  {label}: {} distinct", v.len());
        for (val, cnt) in v.iter().take(6) {
            println!("     {val}  x{cnt}");
        }
    }
}
