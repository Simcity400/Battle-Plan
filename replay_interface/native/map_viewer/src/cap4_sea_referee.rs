//! CAP4-SEA (2026-07-09) — clean, no-FOW open-sea base-color referee.
//!
//! # Why this exists (the political sea BASE look, formerly UNKNOWN — see
//! `replay_interface/documents/POLITICAL_COLOR_LAW.md`)
//!
//! The prior sea-base referee (the political borders-pass post-RT) showed a
//! "pale grey-green" open sea and was used to argue our dark reflective water
//! was wrong. That referee was **FOW-CONTAMINATED**: the save it was mined from
//! had fog-of-war over the visible ocean, so the pale wash was fog, not a sea
//! colour law.
//!
//! The `session_WAVE1_CAP4` `fow_off_observer_front` round is a **no-FOW**
//! observer capture. This module mines its water pass to establish a CLEAN
//! open-sea referee and to PROVE the sea region carries no fog, so the referee
//! can never again be dismissed as FOW-contaminated.
//!
//! # What it measures (all from the read-only preserved corpus)
//!
//! * **Sea mask** — the pixels the water draw actually changed, i.e.
//!   `|post_water_rgb − pre_water_rgb|₁ > 1e-3` between the water pass's captured
//!   `om_pre_draw` and `om_post_pass` render targets (both 1920×1080
//!   `R16G16B16A16_FLOAT` HDR scene RTs). Open sea is exactly where pdxwater
//!   overwrote the pre-water terrain/clear.
//! * **Sea base HDR colour** — the mean linear HDR RGB of those sea pixels in
//!   the `om_post_pass` RT: the engine's own on-screen open-sea look BEFORE
//!   grading/tonemap.
//! * **Sea FoW-alpha** — the water pass `ShadowMap` (DXGI 87 BGRA; `fow.fxh`
//!   packs `FogAlphaFactor` in `.b` = byte 0, `FogColorFactor` in `.g` = byte 1,
//!   the shadow term in `.r` = byte 2) sampled over the sea mask. A near-zero
//!   `FogAlphaFactor` there means `lerp(Color, fog, FogAlphaFactor)`
//!   (`pdxmap.shader:388`) is a byte-safe no-op — the sea is fog-clean.
//!
//! # What this closes and what it does NOT
//!
//! It ESTABLISHES the clean referee and refutes the pale-wash-as-law reading:
//! the fog-clean open sea is dark reflective blue (B > G > R), not pale
//! grey-green, and cross-checks against the independent `live_PAN2` clean sea
//! (same blue hue shape, ~1.3× brighter from a different camera/hour). It does
//! NOT byte-validate our replay's pdxwater output pixel-for-pixel — that needs
//! the water D3D11 pass re-rendered with these captured SRVs/CBs, a render op,
//! not an offline-mining reduction. That parity remains gated on the water pass.

use crate::luminance_law::f16_bits_to_f32;
use std::fs;
use std::path::{Path, PathBuf};

/// 1920×1080 backbuffer the CAP4 rounds were captured at.
pub const SCENE_WIDTH: usize = 1920;
/// 1920×1080 backbuffer the CAP4 rounds were captured at.
pub const SCENE_HEIGHT: usize = 1080;
const RGBA16F_BYTES: usize = SCENE_WIDTH * SCENE_HEIGHT * 8;
const BGRA8_BYTES: usize = SCENE_WIDTH * SCENE_HEIGHT * 4;
/// L1 threshold on the pre→post water RGB delta that marks a water-touched
/// (open-sea) pixel.
const SEA_DELTA_L1: f64 = 1e-3;

/// The clean open-sea referee mined from one no-FOW water pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeaBaseReferee {
    /// Total frame pixels (1920×1080).
    pub frame_pixels: usize,
    /// Pixels the water draw overwrote (the open-sea mask).
    pub sea_pixel_count: usize,
    /// Mean linear HDR RGB of the sea pixels in the post-water scene RT.
    pub sea_base_hdr_mean: [f64; 3],
    /// Mean `FogAlphaFactor` over the sea, in raw 0..255 byte units (byte 0 of
    /// the BGRA `ShadowMap`). Near zero ⇒ fog-clean sea.
    pub sea_fow_alpha_byte_mean: f64,
    /// Mean shadow term over the sea, in raw 0..255 byte units (byte 2). Near
    /// 255 ⇒ unshadowed.
    pub sea_shadow_byte_mean: f64,
}

impl SeaBaseReferee {
    /// True iff the sea region carries no meaningful fog (the referee is clean).
    /// `FogAlphaFactor` under one byte (≈0.004 of full scale) makes the fog
    /// `lerp` a byte no-op.
    pub fn sea_is_fog_clean(&self) -> bool {
        self.sea_fow_alpha_byte_mean < 1.0
    }

    /// True iff the clean sea is blue-dominant (B > G > R), i.e. dark reflective
    /// water — NOT the pale grey-green the FOW-contaminated referee showed.
    pub fn sea_is_blue_dominant(&self) -> bool {
        self.sea_base_hdr_mean[2] > self.sea_base_hdr_mean[1]
            && self.sea_base_hdr_mean[1] > self.sea_base_hdr_mean[0]
    }
}

fn read_rgba16f_scene(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.len() != RGBA16F_BYTES {
        return Err(format!(
            "{} is {} bytes, expected {RGBA16F_BYTES} for 1920x1080 RGBA16F",
            path.display(),
            bytes.len()
        ));
    }
    Ok(bytes)
}

#[inline]
fn texel_rgb(raw: &[u8], px: usize) -> [f64; 3] {
    let b = px * 8;
    [
        f64::from(f16_bits_to_f32(u16::from_le_bytes([raw[b], raw[b + 1]]))),
        f64::from(f16_bits_to_f32(u16::from_le_bytes([
            raw[b + 2],
            raw[b + 3],
        ]))),
        f64::from(f16_bits_to_f32(u16::from_le_bytes([
            raw[b + 4],
            raw[b + 5],
        ]))),
    ]
}

/// Mine the clean open-sea referee from a captured water pass.
///
/// `pre_water_rt` / `post_water_rt` are the water draw's `om_pre_draw` and
/// `om_post_pass` render targets; `water_shadow_map` is the water draw's
/// `ShadowMap` SRV payload (DXGI 87 BGRA).
pub fn mine_sea_base_referee(
    pre_water_rt: &Path,
    post_water_rt: &Path,
    water_shadow_map: &Path,
) -> Result<SeaBaseReferee, String> {
    let pre = read_rgba16f_scene(pre_water_rt)?;
    let post = read_rgba16f_scene(post_water_rt)?;
    let shadow = fs::read(water_shadow_map)
        .map_err(|e| format!("read {}: {e}", water_shadow_map.display()))?;
    if shadow.len() != BGRA8_BYTES {
        return Err(format!(
            "{} is {} bytes, expected {BGRA8_BYTES} for 1920x1080 BGRA8 ShadowMap",
            water_shadow_map.display(),
            shadow.len()
        ));
    }

    let frame_pixels = SCENE_WIDTH * SCENE_HEIGHT;
    let mut sea_pixel_count = 0usize;
    let mut sum_rgb = [0.0f64; 3];
    let mut sum_fow_alpha = 0.0f64;
    let mut sum_shadow = 0.0f64;

    for px in 0..frame_pixels {
        let a = texel_rgb(&pre, px);
        let b = texel_rgb(&post, px);
        let l1 = (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs();
        if l1 > SEA_DELTA_L1 {
            sea_pixel_count += 1;
            sum_rgb[0] += b[0];
            sum_rgb[1] += b[1];
            sum_rgb[2] += b[2];
            let base = px * 4;
            sum_fow_alpha += f64::from(shadow[base]); // byte 0 = .b = FogAlphaFactor
            sum_shadow += f64::from(shadow[base + 2]); // byte 2 = .r = shadow term
        }
    }

    if sea_pixel_count == 0 {
        return Err("no water-touched sea pixels found (pre/post RTs identical?)".into());
    }
    let n = sea_pixel_count as f64;
    Ok(SeaBaseReferee {
        frame_pixels,
        sea_pixel_count,
        sea_base_hdr_mean: [sum_rgb[0] / n, sum_rgb[1] / n, sum_rgb[2] / n],
        sea_fow_alpha_byte_mean: sum_fow_alpha / n,
        sea_shadow_byte_mean: sum_shadow / n,
    })
}

/// Locate the single file in `dir` whose name contains `needle` and ends in
/// `.d3d11`. Fails closed if zero or more than one match (the water pass has
/// exactly one pre-RT, one post-RT, and one ShadowMap payload per round).
pub fn find_unique_payload(dir: &Path, needle: &str) -> Result<PathBuf, String> {
    let mut hit: Option<PathBuf> = None;
    for entry in fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".d3d11") && name.contains(needle) {
            if hit.is_some() {
                return Err(format!(
                    "more than one {needle} payload in {}",
                    dir.display()
                ));
            }
            hit = Some(entry.path());
        }
    }
    hit.ok_or_else(|| format!("no {needle} payload in {}", dir.display()))
}

/// Convenience: mine the referee straight from a round directory by locating the
/// water pre/post RTs and the water ShadowMap by name.
pub fn mine_sea_base_referee_round_dir(round_dir: &Path) -> Result<SeaBaseReferee, String> {
    let pre = find_unique_payload(
        round_dir,
        "water_frame_draw_map_frame_shared_payload_om_pre_draw_render_target",
    )?;
    let post = find_unique_payload(
        round_dir,
        "water_frame_draw_map_frame_shared_payload_om_post_pass_render_target",
    )?;
    let shadow = find_unique_payload(
        round_dir,
        "water_frame_draw_map_frame_shared_payload_ps_srv_ShadowMap",
    )?;
    mine_sea_base_referee(&pre, &post, &shadow)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crate's gate table, included so this test module resolves each
    /// corpus through EXACTLY the candidate list the build-time probe used.
    /// Two copies of a candidate list drift; one cannot.
    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    /// The CAP4 round corpus: `BATTLE_PLAN_WAVE1_CAP4_ROUNDS`, else the documented
    /// preserved copy. Panics rather than skipping -- callers are gated on
    /// `gate_wave1_cap4_no_fow`, which probed this same resolver.
    fn rounds_root() -> PathBuf {
        corpus::wave1_cap4_rounds_dir(&["fow_off_observer_front"]).expect(
            "the CAP4 round corpus (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved copy) is absent, \
             but this test was compiled as runnable",
        )
    }

    #[test]
    fn find_unique_payload_is_exact() {
        // Synthetic dir with one matching and one non-matching file.
        let dir = std::env::temp_dir().join(format!("cap4_sea_find_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("water_frame_draw_x_om_post_pass_render_target_y.d3d11"),
            b"x",
        )
        .unwrap();
        fs::write(dir.join("unrelated_ps_srv_Diffuse_z.d3d11"), b"y").unwrap();
        let hit = find_unique_payload(&dir, "om_post_pass_render_target").unwrap();
        assert!(hit
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("post_pass"));
        assert!(find_unique_payload(&dir, "does_not_exist").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    // CAP4-SEA GATE (Target 1): the no-FOW observer round yields a fog-clean,
    // blue-dominant open-sea referee. Skip-if-absent on the read-only corpus.
    #[test]
    #[cfg_attr(
        not(gate_wave1_cap4_no_fow),
        ignore = "BATTLE_PLAN_WAVE1_CAP4_ROUNDS naming a capture corpus with fow_off_observer_front"
    )]
    fn gate_wave1_cap4_no_fow_sea_base_referee() {
        let round =
            replay_test_gates::require_existing_dir(rounds_root().join("fow_off_observer_front"));
        let r = mine_sea_base_referee_round_dir(&round).expect("mine CAP4 no-FOW sea referee");
        eprintln!("CAP4 no-FOW sea referee: {r:#?}");

        // Deterministic open-sea coverage for this preserved frame.
        assert_eq!(r.frame_pixels, 1920 * 1080);
        assert_eq!(
            r.sea_pixel_count, 1_009_749,
            "open-sea mask pixel count changed for the preserved frame"
        );

        // KEY: the sea is fog-clean — FogAlphaFactor under one byte, so the
        // fow lerp is a byte no-op. This is what makes the referee valid.
        assert!(
            r.sea_is_fog_clean(),
            "sea FoW alpha byte mean {} is not fog-clean (< 1.0)",
            r.sea_fow_alpha_byte_mean
        );
        assert!((r.sea_fow_alpha_byte_mean - 0.4796).abs() < 0.01);
        // Unshadowed sea (shadow term ~255).
        assert!(r.sea_shadow_byte_mean > 250.0, "{}", r.sea_shadow_byte_mean);

        // Clean sea is dark reflective blue (B > G > R), NOT pale grey-green.
        assert!(
            r.sea_is_blue_dominant(),
            "clean sea is not blue-dominant: {:?}",
            r.sea_base_hdr_mean
        );
        let m = r.sea_base_hdr_mean;
        assert!((m[0] - 0.100678).abs() < 0.002, "R {}", m[0]);
        assert!((m[1] - 0.131395).abs() < 0.002, "G {}", m[1]);
        assert!((m[2] - 0.191898).abs() < 0.002, "B {}", m[2]);
    }
}
