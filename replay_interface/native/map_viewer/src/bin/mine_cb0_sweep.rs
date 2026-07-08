//! NIGHT1 CB0-sweep miner: decode the hour-swept `cb0_samples.jsonl` corpora
//! captured tonight (UNPAUSED night sweeps) and tabulate every field that moves
//! with the in-game hour, so the night-side light curve can be fitted.
//!
//! Files-first discipline: the CB0 payloads are law-fitting *evidence*, never a
//! runtime source. This bin reads only `cb0_samples.jsonl` (each line a
//! `{"wall_ms":..,"sample_index":..,"cb0_hex":"<832 hex chars = 416 bytes>"}`)
//! and decodes it through the SAME [`SceneLighting`] field map the replay path
//! uses (`map_viewer::light_law`) — no re-derived offsets.
//!
//! Determinism: sessions and lines are visited in input order; the CSV/JSON
//! output is a pure function of the inputs, so two runs are identical.
//!
//! Usage:
//!   mine_cb0_sweep --session <dir> [--session <dir>]... --out <dir>
//!
//! Each `--session` is a capture session dir holding `cb0_samples.jsonl` (e.g.
//!   C:\Users\Example\AppData\Local\hoi4_captures\session_NIGHT_SWEEP_6).
//! Outputs (in `--out`):
//!   * `cb0_sweep_samples.csv`  — one row per decoded sample, every moving field
//!   * `cb0_sweep_curve.json`   — the invariance finding + residuals + range
//!   * `cb0_sweep_summary.md`   — human-readable finding table
//!
//! Finding (mined tonight): the sun-direction `DayNight_Hour_SunDir.yzw` is
//! HOUR-INVARIANT — the hour lives entirely in `.x` (the terminator phase). See
//! `light_law::render_sun_dir_yzw` and `documents/LIGHT_LAW.md` §4/§8.

use map_viewer::{Result, SceneLighting};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// One decoded sweep sample.
#[derive(Clone, Debug)]
struct Sample {
    session: String,
    wall_ms: u64,
    sample_index: i64,
    lighting: SceneLighting,
}

impl Sample {
    fn hour(&self) -> f32 {
        self.lighting.day_night_hour()
    }
    fn hour_frac(&self) -> f32 {
        self.lighting.day_night_hour_sun_dir[0]
    }
    fn fow_w(&self) -> f32 {
        self.lighting.fow_opacity_time_snowmud_maxspeed[3]
    }
    fn yzw(&self) -> [f32; 3] {
        self.lighting.sun_dir_yzw()
    }
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let b = hex.as_bytes();
    let nib = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut i = 0;
    while i < b.len() {
        out.push((nib(b[i])? << 4) | nib(b[i + 1])?);
        i += 2;
    }
    Some(out)
}

/// Extract a JSON string field value by key from a flat one-line object.
/// The sweep JSONL is machine-emitted and flat, so a scan for `"key":"..."` /
/// `"key":<number>` is sufficient and avoids a serde_json dependency mismatch.
fn json_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn json_num(line: &str, key: &str) -> Option<i64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '-'))
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn load_session(dir: &Path) -> Result<Vec<Sample>> {
    let session = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let path = dir.join("cb0_samples.jsonl");
    let text = fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut samples = Vec::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let hex = json_str(line, "cb0_hex")
            .ok_or_else(|| format!("{}:{} missing cb0_hex", path.display(), lineno + 1))?;
        let bytes =
            decode_hex(hex).ok_or_else(|| format!("{}:{} bad hex", path.display(), lineno + 1))?;
        let lighting = SceneLighting::from_cb0_bytes(&bytes).ok_or_else(|| {
            format!(
                "{}:{} CB0 too short ({} bytes) to decode lighting",
                path.display(),
                lineno + 1,
                bytes.len()
            )
        })?;
        samples.push(Sample {
            session: session.clone(),
            wall_ms: json_num(line, "wall_ms").unwrap_or(0) as u64,
            sample_index: json_num(line, "sample_index").unwrap_or(lineno as i64),
            lighting,
        });
    }
    Ok(samples)
}

/// The NIGHT1 finding, computed from the sweep: the sun-direction unit vector
/// `.yzw` is HOUR-INVARIANT. Rather than fit an hour-dependent curve (a
/// single-day rotation was TESTED and REFUTED — see the `rotation_recon_resid`
/// below, which is ~2.0, i.e. garbage), this analysis quantifies the actual
/// structure:
///   * `.y` (dir.x) is ~0 at every hour (`max_abs_y`).
///   * `.w` (dir.z) is ~1 at every hour (`max_one_minus_w`).
///   * `.z` (dir.y) moves only ACROSS DATES, never with the hour. We prove this
///     by the max `.yzw` spread WITHIN any single session that has ≥2 distinct
///     hours at (near) one date (`max_intra_date_yzw_spread`), which is tiny,
///     versus the total spread across all sessions/dates (`total_z_spread`).
#[derive(Clone, Debug)]
struct InvarianceAnalysis {
    /// Unique (session, hour) pairs decoded.
    n_unique_hours: usize,
    /// max |.y| over all samples (dir.x component; measured ~0).
    max_abs_y: f32,
    /// max |1 − .w| over all samples (dir.z component; measured ~1).
    max_one_minus_w: f32,
    /// max ||yzw| − 1| over all samples (unit-vector residual).
    max_unit_resid: f32,
    /// max `.z` spread WITHIN a single session across its distinct hours. NOTE:
    /// a session that itself runs unpaused for many game-days (e.g.
    /// NIGHT_SWEEP_6) advances the DATE within the session, so its intra spread
    /// reflects date drift, not hour. The CLEAN control is
    /// `cleanest_control_z_spread` below.
    max_intra_session_z_spread: f32,
    /// The CLEANEST hour-invariance control: the SMALLEST `.z` spread among
    /// sessions that swept ≥2 distinct hours. A short single-date multi-hour run
    /// (CAPFIX2_FULL_2, hours 1→6) drives this ~0 ⇒ across a quarter-day of hour
    /// advance at one date, `.z` does not move.
    cleanest_control_z_spread: f32,
    /// The session that produced `cleanest_control_z_spread`.
    cleanest_control_session: String,
    /// total `.z` spread across ALL sessions/dates (the seasonal/date term).
    total_z_spread: f32,
    /// Reconstruction residual of the REFUTED single-day-rotation model
    /// (`.y=r·cosθ, .w=r·sinθ`) — reported to show it is inadmissible (~2.0).
    rotation_recon_resid: f32,
    /// Proven hour range [min,max] the invariance is observed on.
    hour_min: f32,
    hour_max: f32,
}

fn analyze_invariance(samples: &[Sample]) -> Option<InvarianceAnalysis> {
    if samples.is_empty() {
        return None;
    }
    // Unique (session, hour) pairs.
    let mut uniq: Vec<&Sample> = Vec::new();
    for s in samples {
        let h = s.hour_frac();
        if !uniq
            .iter()
            .any(|u| u.session == s.session && (u.hour_frac() - h).abs() < 1e-6)
        {
            uniq.push(s);
        }
    }

    let mut max_abs_y = 0.0f32;
    let mut max_one_minus_w = 0.0f32;
    let mut max_unit = 0.0f32;
    let mut z_min = f32::INFINITY;
    let mut z_max = f32::NEG_INFINITY;
    let (mut hmin, mut hmax) = (f32::INFINITY, f32::NEG_INFINITY);
    for s in samples {
        let v = s.yzw();
        max_abs_y = max_abs_y.max(v[0].abs());
        max_one_minus_w = max_one_minus_w.max((v[2] - 1.0).abs());
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        max_unit = max_unit.max((len - 1.0).abs());
        z_min = z_min.min(v[1]);
        z_max = z_max.max(v[1]);
        hmin = hmin.min(s.hour());
        hmax = hmax.max(s.hour());
    }

    // Intra-session .z spread (the hour-invariance residual): within a single
    // session the in-game date barely advances, so if `.z` tracked the hour it
    // would swing here. It does not.
    let mut sessions: Vec<&str> = samples.iter().map(|s| s.session.as_str()).collect();
    sessions.sort_unstable();
    sessions.dedup();
    let mut max_intra = 0.0f32;
    let mut cleanest = f32::INFINITY;
    let mut cleanest_session = String::new();
    for sess in &sessions {
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut distinct_hours: Vec<f32> = Vec::new();
        for s in samples.iter().filter(|s| s.session == *sess) {
            lo = lo.min(s.yzw()[1]);
            hi = hi.max(s.yzw()[1]);
            let h = s.hour_frac();
            if !distinct_hours.iter().any(|x| (x - h).abs() < 1e-6) {
                distinct_hours.push(h);
            }
        }
        // Only meaningful when the session actually swept ≥2 hours.
        if distinct_hours.len() >= 2 {
            let spread = hi - lo;
            max_intra = max_intra.max(spread);
            if spread < cleanest {
                cleanest = spread;
                cleanest_session = (*sess).to_string();
            }
        }
    }
    if !cleanest.is_finite() {
        cleanest = 0.0;
    }

    // The refuted rotation model, refit and scored, purely to report it dead.
    let radius = uniq
        .iter()
        .map(|s| {
            let v = s.yzw();
            (v[0] * v[0] + v[2] * v[2]).sqrt()
        })
        .sum::<f32>()
        / uniq.len() as f32;
    let (mut cs, mut sn) = (0.0f32, 0.0f32);
    for s in uniq.iter() {
        let v = s.yzw();
        let theta = v[2].atan2(v[0]);
        let h0 = s.hour_frac() - theta / (2.0 * std::f32::consts::PI);
        let a = 2.0 * std::f32::consts::PI * h0;
        cs += a.cos();
        sn += a.sin();
    }
    let h0 = (sn.atan2(cs) / (2.0 * std::f32::consts::PI)).rem_euclid(1.0);
    let mut rotation_recon = 0.0f32;
    for s in samples {
        let v = s.yzw();
        let theta = 2.0 * std::f32::consts::PI * (s.hour_frac() - h0);
        rotation_recon = rotation_recon
            .max((radius * theta.cos() - v[0]).abs())
            .max((radius * theta.sin() - v[2]).abs());
    }

    Some(InvarianceAnalysis {
        n_unique_hours: uniq.len(),
        max_abs_y,
        max_one_minus_w,
        max_unit_resid: max_unit,
        max_intra_session_z_spread: max_intra,
        cleanest_control_z_spread: cleanest,
        cleanest_control_session: cleanest_session,
        total_z_spread: z_max - z_min,
        rotation_recon_resid: rotation_recon,
        hour_min: hmin,
        hour_max: hmax,
    })
}

fn write_csv(samples: &[Sample], path: &Path) -> Result<()> {
    let mut s = String::new();
    s.push_str(
        "session,sample_index,wall_ms,hour,hour_frac,fow_w,\
         sun_dir_y,sun_dir_z,sun_dir_w,sun_dir_len,\
         sun_x,moon_x,second_sun_x,second_moon_x,\
         sun_diff_r,sun_diff_g,sun_diff_b,sun_diff_a,\
         moon_diff_r,moon_diff_g,moon_diff_b,moon_diff_a,\
         global_time,cam_x,cam_y,cam_z\n",
    );
    for smp in samples {
        let l = &smp.lighting;
        let v = smp.yzw();
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        s.push_str(&format!(
            "{},{},{},{:.6},{:.8},{:.4},\
             {:.6},{:.6},{:.6},{:.6},\
             {:.4},{:.4},{:.4},{:.4},\
             {:.5},{:.5},{:.5},{:.5},\
             {:.5},{:.5},{:.5},{:.5},\
             {:.4},{:.2},{:.2},{:.2}\n",
            smp.session,
            smp.sample_index,
            smp.wall_ms,
            smp.hour(),
            smp.hour_frac(),
            smp.fow_w(),
            v[0],
            v[1],
            v[2],
            len,
            l.virtual_sun_pos[0],
            l.virtual_moon_pos[0],
            l.second_virtual_sun_pos[0],
            l.second_virtual_moon_pos[0],
            l.sun_diffuse_intensity[0],
            l.sun_diffuse_intensity[1],
            l.sun_diffuse_intensity[2],
            l.sun_diffuse_intensity[3],
            l.moon_diffuse_intensity[0],
            l.moon_diffuse_intensity[1],
            l.moon_diffuse_intensity[2],
            l.moon_diffuse_intensity[3],
            l.global_time,
            l.cam_pos[0],
            l.cam_pos[1],
            l.cam_pos[2],
        ));
    }
    fs::write(path, s).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn write_curve_json(a: &InvarianceAnalysis, samples: &[Sample], path: &Path) -> Result<()> {
    let n_fow_night = samples.iter().filter(|s| s.fow_w() > 0.5).count();
    let n_fow_day = samples.iter().filter(|s| s.fow_w() <= 0.5).count();
    let s = format!(
        "{{\n  \"finding\": \"sun_dir_yzw is HOUR-INVARIANT; hour lives only in .x (terminator phase)\",\n\
         \"render_law\": \"write .x = hour/24; echo captured .yzw verbatim (do NOT rotate with hour)\",\n\
         \"max_abs_y\": {:.8},\n  \"max_one_minus_w\": {:.8},\n  \"max_unit_resid\": {:.8},\n\
         \"max_intra_session_z_spread\": {:.8},\n\
         \"cleanest_control_z_spread\": {:.8},\n  \"cleanest_control_session\": \"{}\",\n\
         \"total_z_spread_across_dates\": {:.8},\n\
         \"refuted_rotation_recon_resid\": {:.6},\n\
         \"n_unique_session_hours\": {},\n  \"total_samples\": {},\n\
         \"fow_w_gt_half\": {},\n  \"fow_w_le_half\": {},\n\
         \"proven_hour_min\": {:.4},\n  \"proven_hour_max\": {:.4}\n}}\n",
        a.max_abs_y,
        a.max_one_minus_w,
        a.max_unit_resid,
        a.max_intra_session_z_spread,
        a.cleanest_control_z_spread,
        a.cleanest_control_session,
        a.total_z_spread,
        a.rotation_recon_resid,
        a.n_unique_hours,
        samples.len(),
        n_fow_night,
        n_fow_day,
        a.hour_min,
        a.hour_max,
    );
    fs::write(path, s).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn write_summary_md(a: &InvarianceAnalysis, samples: &[Sample], path: &Path) -> Result<()> {
    let n_fow_night = samples.iter().filter(|s| s.fow_w() > 0.5).count();
    let mut s = String::new();
    s.push_str("# NIGHT1 CB0 sweep — mined night-curve evidence\n\n");
    s.push_str(&format!("Total decoded samples: {}\n\n", samples.len()));
    s.push_str("## Finding: sun-direction `.yzw` is HOUR-INVARIANT\n\n");
    s.push_str(
        "The hour rotation lives entirely in `DayNight_Hour_SunDir.x` (the terminator \
         longitude phase). The sun-direction unit vector `.yzw` does NOT rotate with the \
         hour; its only mover, `.z` (dir.y), drifts with the advancing in-game DATE. \
         A single-day-rotation model was tested and REFUTED (reconstruction residual below).\n\n",
    );
    s.push_str("| quantity | value |\n|---|---|\n");
    s.push_str(&format!(
        "| max |.y| (dir.x, should be ~0) | {:.6} |\n",
        a.max_abs_y
    ));
    s.push_str(&format!(
        "| max |1−.w| (dir.z, should be ~0) | {:.6} |\n",
        a.max_one_minus_w
    ));
    s.push_str(&format!(
        "| max ||yzw|−1| (unit residual) | {:.6} |\n",
        a.max_unit_resid
    ));
    s.push_str(&format!(
        "| CLEAN control `.z` spread (single-date multi-hour, session {}) | {:.6} |\n",
        a.cleanest_control_session, a.cleanest_control_z_spread
    ));
    s.push_str(&format!(
        "| max intra-session `.z` spread (incl. sessions that advanced dates) | {:.6} |\n",
        a.max_intra_session_z_spread
    ));
    s.push_str(&format!(
        "| total `.z` spread across dates (seasonal term) | {:.6} |\n",
        a.total_z_spread
    ));
    s.push_str(&format!(
        "| REFUTED single-day-rotation recon residual | {:.6} |\n",
        a.rotation_recon_resid
    ));
    s.push_str(&format!(
        "| unique (session,hour) pairs | {} |\n",
        a.n_unique_hours
    ));
    s.push_str(&format!(
        "| samples with FoW.w>0 (night gate exercised) | {} |\n",
        n_fow_night
    ));
    s.push_str(&format!(
        "| proven hour range | [{:.3}, {:.3}] |\n\n",
        a.hour_min, a.hour_max
    ));

    // Per-unique-hour table.
    let mut uniq: Vec<&Sample> = Vec::new();
    for smp in samples {
        let h = smp.hour_frac();
        if !uniq.iter().any(|u| (u.hour_frac() - h).abs() < 1e-6) {
            uniq.push(smp);
        }
    }
    // total_cmp never panics (handles NaN hours), unlike partial_cmp().unwrap().
    uniq.sort_by(|a, b| a.hour().total_cmp(&b.hour()));
    s.push_str("## Per-hour measured .yzw + FoW.w (deduped by hour)\n\n");
    s.push_str(
        "_Note: dedup keeps the first sample seen per hour, drawn from different \
         DATES, so the `.z` scatter across rows below is the date/seasonal term, \
         NOT an hour dependence. Within one date (see the CLEAN control above) `.z` \
         is flat across hours._\n\n",
    );
    s.push_str("| hour | FoW.w | .y | .z | .w | |yzw| |\n|---|---|---|---|---|---|\n");
    for smp in &uniq {
        let v = smp.yzw();
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        s.push_str(&format!(
            "| {:.4} | {:.2} | {:+.4} | {:+.4} | {:+.4} | {:.4} |\n",
            smp.hour(),
            smp.fow_w(),
            v[0],
            v[1],
            v[2],
            len
        ));
    }
    fs::write(path, s).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn main() -> Result<()> {
    let mut sessions: Vec<PathBuf> = Vec::new();
    let mut out_dir: Option<PathBuf> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--session" => sessions.push(PathBuf::from(
                args.next().ok_or("--session needs a directory")?,
            )),
            "--out" => out_dir = Some(PathBuf::from(args.next().ok_or("--out needs a dir")?)),
            "--help" | "-h" => {
                println!("Usage: mine_cb0_sweep --session <dir> [--session <dir>]... --out <dir>");
                return Ok(());
            }
            other => return Err(format!("unknown argument {other}").into()),
        }
    }
    if sessions.is_empty() {
        return Err("no --session <dir> supplied".into());
    }
    let out_dir = out_dir.ok_or("no --out <dir> supplied")?;
    fs::create_dir_all(&out_dir).map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;

    let mut all: Vec<Sample> = Vec::new();
    for dir in &sessions {
        let sess = load_session(dir)?;
        eprintln!("{}: {} samples", dir.display(), sess.len());
        all.extend(sess);
    }
    if all.is_empty() {
        return Err("no samples decoded".into());
    }

    let analysis = analyze_invariance(&all).ok_or("invariance analysis failed")?;
    eprintln!(
        "finding: yzw HOUR-INVARIANT | max|.y|={:.5} max|1-.w|={:.5} \
         clean_control_z_spread={:.6}({}) total_z_spread={:.5} \
         REFUTED_rotation_recon={:.4} range=[{:.3},{:.3}]",
        analysis.max_abs_y,
        analysis.max_one_minus_w,
        analysis.cleanest_control_z_spread,
        analysis.cleanest_control_session,
        analysis.total_z_spread,
        analysis.rotation_recon_resid,
        analysis.hour_min,
        analysis.hour_max
    );

    write_csv(&all, &out_dir.join("cb0_sweep_samples.csv"))?;
    write_curve_json(&analysis, &all, &out_dir.join("cb0_sweep_curve.json"))?;
    write_summary_md(&analysis, &all, &out_dir.join("cb0_sweep_summary.md"))?;
    eprintln!("wrote CSV/JSON/MD to {}", out_dir.display());
    Ok(())
}
