//! FAITHFUL gradient borders — a CPU signed-distance field (SDF) feeding a 1:1 port
//! of the game's `gradient_border_*` shader stage.
//!
//! ## What the engine actually does (and what we reproduce)
//!
//! the game does NOT draw borders by comparing the 4 neighbour province ids per pixel.
//! At runtime it bakes a pair of `GradientBorderChannel` textures (Ch1 = country,
//! Ch2 = state on the political map). Each texel of that texture stores, in its
//! **alpha**, a *normalized distance-to-border* — a "height field" where the value
//! is HIGH right on the border and ramps DOWN into the country interior. The RGB is
//! the border colour (the country's own map colour). The shader then converts that
//! height to an anti-aliased FILL + a darkened OUTLINE through `Levels` + the camera
//! distance ramp (`gradient_border_distance_to_alpha`).
//!
//! Concretely, in `gradient_border_process_channel`
//! [standardfuncsgfx.fxh:840-883]:
//! ```text
//!   Alpha          = vGBDist.a                      // our SDF height, 0..1
//!   vColorOpacity  = Levels(Alpha, 0, vOutlineCutoff)
//!   vOutline       = 1 - Levels(Alpha, vOutlineCutoff, 1)
//!   vOutline      *= floor(vColorOpacity)           // only inside the filled area
//!   vColorOpacity  = gradient_border_distance_to_alpha(vColorOpacity, vCamDist)
//!   vColorOpacity *= floor(vOldOutline)             // fill XOR outline, never both
//!   vThick         = smoothstep(0,1, Levels(Alpha, cutoff-fullW, cutoff-fullW+gradW))
//!   vMaxGradient   = max(vColorOpacity, vOutline)
//!   vCh            = lerp(vCh, vGBDist.rgb, max(vMaxGradient, vThick) * vStrength)
//!   vCh            = lerp(vCh, vCh*0.5, vThick)      // outline edge darken
//! ```
//! and `gradient_border_distance_to_alpha` [fxh:818-822]:
//! ```text
//!   vDist  = 1 - vDist
//!   vDist *= (vCamDist - GB_THRESHOLD) / GB_THRESHOLD2     // 0.05, 0.25
//!   return 1 - saturate(vDist)
//! ```
//! with `vCamDist = gradient_border_camera_distance()` [fxh:813]:
//! ```text
//!   1 - clamp(cam_distance(GB_CAM_MIN, GB_CAM_MAX), 0, GB_CAM_MAX_FILLING_CLAMP)
//!         // GB_CAM_MIN=100, GB_CAM_MAX=350, clamp=0.8
//! ```
//!
//! ## The height-field encoding (the part the engine bakes in C++; we do it here)
//!
//! The engine builds the channel alpha so that the **gradient is linear, 1/255 per
//! pixel**, over a thickness in pixels (`GRADIENT_BORDERS_COUNTRY_CENTER_THICKNESS`
//! = 2 px linear center, then ramping to the full `THICKNESS_COUNTRY_LOW..HIGH`
//! 5..25 px envelope) [00_graphics.lua:867-869]. The cutoff `0.973`
//! [00_graphics.lua:880] is where colour-fill ends and the outline begins —
//! i.e. the outermost ~`(1-0.973)*255 ≈ 6.9 px` is the OUTLINE band and everything
//! more interior is FILL.
//!
//! We replicate that height field as: build a true Euclidean distance (in texels) from
//! every land texel to the nearest border of each kind (country / province / state),
//! then encode
//! ```text
//!   alpha = saturate(1 - distance_in_px / thickness_px)
//! ```
//! so a texel ON the border has distance 0 → alpha 1, and the alpha falls off
//! linearly (1/thickness per px), exactly the engine's `1/255 per pixel` ramp scaled
//! to our chosen thickness. `thickness_px` is the static bake thickness; the *camera*
//! width ramp is applied later in the shader via `gradient_border_distance_to_alpha`
//! (the `(vCamDist-0.05)/0.25` term), which widens/narrows the visible band with zoom
//! — that is the engine's "whole magic" comment and is where 5..25 px comes from.
//!
//! ## Texture packing
//!
//! We emit one `Rgba8Unorm` texture at the raster resolution:
//!   * **R** = country-border height (alpha convention: 1 on a country border).
//!   * **G** = province-border height.
//!   * **B** = state-border height.
//!   * **A** = COASTAL-border height on the SEA side (item A): the engine runs
//!     gradient_border_apply in the WATER shader [pdxwater.shader:351-355] so coastlines get
//!     a border line on the sea side (GB_OUTLINE_CUTOFF_SEA=0.990 [constants.fxh:201]). On
//!     land texels A=0 (the land side already carries its province/country line). The WGSL
//!     gates LAND borders by the per-province owner key (not by A), so the A channel is free
//!     to carry the coastal height rather than a redundant land mask.
//! The border RGB colour is NOT stored per-texel: the game's GradientBorderChannel textures are
//! RUNTIME-generated (per-frame from province adjacency) and never on disk, so there is no
//! channel RGB to load. The engine's gradient colour is the COUNTRY's own map colour; the
//! visible country LINE is that colour darkened by the `vCh*0.5` outline edge. The WGSL
//! reproduces this by feeding the country political colour as the outline target and applying
//! the same `*0.5` darken (see map.wgsl gradient_border_process), which is the faithful
//! visible result for the political map mode.
//!
//! ## Distance transform
//!
//! We use the 8SSEDT two-pass signed Euclidean distance transform (Danielsson /
//! Leymarie). It is exact-enough (sub-pixel error << 1) and O(N) — for the 5632×2048
//! political raster (~11.5M texels) it is a handful of linear passes, far cheaper than
//! a jump-flood and trivially correct to unit-test. We compute an *unsigned* distance
//! to the border on the land side only (we never draw a border into the sea), which is
//! all the height-field encoding needs.

use crate::map::{MapIds, ProvinceRaster, OWNER_NONE};

// ---------------------------------------------------------------------------
// Engine constants (cited). Mirrored from the game so the WGSL and CPU agree.
// ---------------------------------------------------------------------------

/// Static bake thickness (texels) of the country border height ramp. The engine's
/// envelope is `THICKNESS_COUNTRY_LOW=5 .. HIGH=25` [00_graphics.lua:867,869]; we bake
/// at the HIGH end so the SDF carries enough range for the *near* camera (where the
/// shader's `gradient_border_distance_to_alpha` widens the band toward 25 px), and the
/// same ramp narrows toward ~5 px at far zoom. Baking at HIGH loses no near-zoom width.
pub const COUNTRY_THICKNESS_PX: f32 = 25.0;
/// Static bake thickness (texels) of the state border height ramp.
/// `GRADIENT_BORDERS_THICKNESS_STATE = 5.0` [00_graphics.lua:870].
pub const STATE_THICKNESS_PX: f32 = 5.0;
/// Province borders are the faint thin lines under everything; the engine has no
/// dedicated gradient channel for them on the political map (they ride the province
/// fade `PROVINCE_BORDER_FADE_NEAR/FAR` [00_graphics.lua:774-775]). We bake a thin
/// 2 px ramp so the line is ~1 px wide at the cutoff.
pub const PROVINCE_THICKNESS_PX: f32 = 2.0;

/// `GRADIENT_BORDERS_OUTLINE_CUTOFF_COUNTRY = 0.973` [00_graphics.lua:880].
pub const OUTLINE_CUTOFF_COUNTRY: f32 = 0.973;
/// `GRADIENT_BORDERS_OUTLINE_CUTOFF_STATE = 0.973` [00_graphics.lua:883].
pub const OUTLINE_CUTOFF_STATE: f32 = 0.973;
/// `GB_OUTLINE_CUTOFF_SEA = 0.990` [constants.fxh:201] — the coastal/sea-side cutoff the
/// engine uses in the WATER shader's gradient_border_apply [pdxwater.shader:351-355]. The
/// WGSL applies this on the SEA side of coastlines.
pub const OUTLINE_CUTOFF_SEA: f32 = 0.990;
/// Static bake thickness (texels) of the coastal border height ramp on the sea side. The sea
/// coast line is thin; bake at the state thickness so the SEA cutoff (0.990) yields a hairline.
pub const COAST_THICKNESS_PX: f32 = 5.0;

/// `GB_THRESHOLD = 0.05` [constants.fxh:198].
pub const GB_THRESHOLD: f32 = 0.05;
/// `GB_THRESHOLD2 = 0.25` [constants.fxh:199].
pub const GB_THRESHOLD2: f32 = 0.25;
/// `GB_CAM_MAX_FILLING_CLAMP = 0.8` [constants.fxh:197].
pub const GB_CAM_MAX_FILLING_CLAMP: f32 = 0.8;

// ---------------------------------------------------------------------------
// The CPU border field.
// ---------------------------------------------------------------------------

/// A baked per-texel border height field, ready to upload as an `Rgba8Unorm` texture
/// at the province-raster resolution. See module docs for the channel meaning.
pub struct BorderField {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, length `width*height*4`. R=country, G=province, B=state height;
    /// A=COASTAL height (sea side; item A).
    pub rgba: Vec<u8>,
}

/// 8SSEDT cell: the (dx,dy) vector to the nearest seed. We store the integer offset
/// and derive Euclidean distance as its length — exact for the propagated nearest seed.
#[derive(Clone, Copy)]
struct Cell {
    dx: i32,
    dy: i32,
}
const FAR: Cell = Cell {
    dx: 32767,
    dy: 32767,
};
const ZERO: Cell = Cell { dx: 0, dy: 0 };

#[inline]
fn cell_len2(c: Cell) -> i64 {
    (c.dx as i64) * (c.dx as i64) + (c.dy as i64) * (c.dy as i64)
}

/// Two-pass 8SSEDT (8-points Signed Sequential Euclidean Distance Transform).
///
/// `seed[i] == true` marks a texel that is ON a border (distance 0). Returns the
/// Euclidean distance (texels), length `width*height`, from every texel to the nearest
/// seeded texel (`f32::INFINITY` where no seed is reachable).
/// Classic Danielsson sweep: forward pass propagates from top-left using the
/// 4 already-visited neighbours (3 above + 1 left), backward pass from bottom-right
/// using the mirror set; each pass also re-checks the opposite horizontal neighbour
/// to catch the 8-point set. This is O(N) and accurate to << 1 texel.
fn edt(width: usize, height: usize, seed: &[bool]) -> Vec<f32> {
    debug_assert_eq!(seed.len(), width * height);
    let mut grid = vec![FAR; width * height];
    for i in 0..width * height {
        if seed[i] {
            grid[i] = ZERO;
        }
    }

    let idx = |x: usize, y: usize| y * width + x;

    // Compare candidate (neighbour cell + step) against current; keep the shorter.
    #[inline]
    fn relax(grid: &mut [Cell], here: usize, other: usize, sx: i32, sy: i32) {
        let mut cand = grid[other];
        cand.dx += sx;
        cand.dy += sy;
        if cell_len2(cand) < cell_len2(grid[here]) {
            grid[here] = cand;
        }
    }

    // Forward pass: top→bottom, left→right.
    for y in 0..height {
        for x in 0..width {
            let here = idx(x, y);
            if y > 0 {
                relax(&mut grid, here, idx(x, y - 1), 0, 1);
                if x > 0 {
                    relax(&mut grid, here, idx(x - 1, y - 1), 1, 1);
                }
                if x + 1 < width {
                    relax(&mut grid, here, idx(x + 1, y - 1), 1, 1);
                }
            }
            if x > 0 {
                relax(&mut grid, here, idx(x - 1, y), 1, 0);
            }
        }
        // second left→right sweep on the same row to settle horizontal chains.
        for x in (0..width.saturating_sub(1)).rev() {
            let here = idx(x, y);
            relax(&mut grid, here, idx(x + 1, y), 1, 0);
        }
    }
    // Backward pass: bottom→top, right→left.
    for y in (0..height).rev() {
        for x in (0..width).rev() {
            let here = idx(x, y);
            if y + 1 < height {
                relax(&mut grid, here, idx(x, y + 1), 0, 1);
                if x + 1 < width {
                    relax(&mut grid, here, idx(x + 1, y + 1), 1, 1);
                }
                if x > 0 {
                    relax(&mut grid, here, idx(x - 1, y + 1), 1, 1);
                }
            }
            if x + 1 < width {
                relax(&mut grid, here, idx(x + 1, y), 1, 0);
            }
        }
        for x in 1..width {
            let here = idx(x, y);
            relax(&mut grid, here, idx(x - 1, y), 1, 0);
        }
    }

    let mut dist = vec![f32::INFINITY; width * height];
    for i in 0..width * height {
        let c = grid[i];
        if c.dx == FAR.dx && c.dy == FAR.dy {
            dist[i] = f32::INFINITY;
        } else {
            dist[i] = (cell_len2(c) as f64).sqrt() as f32;
        }
    }
    dist
}

/// Encode a distance (texels) into the engine's alpha-height convention:
/// `alpha = saturate(1 - dist/thickness)` — 1 ON the border, linear falloff, 0 beyond
/// `thickness` px. This is the `GradientBorderChannel.a` "height" the shader's
/// `Levels`/`gradient_border_distance_to_alpha` consume (module docs). Returns 0..1.
#[inline]
pub fn distance_to_alpha_height(dist_px: f32, thickness_px: f32) -> f32 {
    if !dist_px.is_finite() || thickness_px <= 0.0 {
        return 0.0;
    }
    (1.0 - dist_px / thickness_px).clamp(0.0, 1.0)
}

/// Build the border field from the province raster + the per-province owner-index
/// (`build_owner_index`) + the province→state map (`MapIds`).
///
/// A **country border** texel is a land texel orthogonally adjacent to a land texel with
/// a DIFFERENT owner key (both non-`OWNER_NONE`). A **province border** texel is adjacent
/// to a different province id. A **state border** texel is adjacent to a different state
/// id (land only). Seeds are the border texels; the 8SSEDT then spreads the distance and
/// `distance_to_alpha_height` encodes it.
///
/// `owner_index` is indexed by province id (length `max_id+1`); a texel's owner is
/// `owner_index[pid]`. Sea / no-data texels (owner `OWNER_NONE`) get A=0 and never seed a
/// country/state border (their province border is still recorded so coastlines read as a
/// thin province line, matching the engine drawing province edges on the coast).
pub fn build_border_field(
    raster: &ProvinceRaster,
    owner_index: &[u32],
    ids: &MapIds,
) -> BorderField {
    let w = raster.width as usize;
    let h = raster.height as usize;
    let n = w * h;

    // Per-texel owner key + state id (precompute once; the raster is read column-major
    // a lot). state id 0 = "not a land province" (sea/lake/unassigned).
    let mut owner_at = vec![OWNER_NONE; n];
    let mut state_at = vec![0u32; n];
    let mut land = vec![false; n];
    for i in 0..n {
        let pid = raster.ids[i];
        let ow = owner_index.get(pid as usize).copied().unwrap_or(OWNER_NONE);
        owner_at[i] = ow;
        if let Some(sid) = ids.state_of_province(pid) {
            state_at[i] = sid.max(1); // ensure land states are non-zero
            land[i] = true;
        }
    }

    let idx = |x: usize, y: usize| y * w + x;

    let mut country_seed = vec![false; n];
    let mut province_seed = vec![false; n];
    let mut state_seed = vec![false; n];
    // COASTAL seed (item A): a texel at a LAND/SEA boundary. The engine runs
    // gradient_border_apply in the WATER shader too [pdxwater.shader:351-355] so coastlines get
    // a border line on the SEA side (GB_OUTLINE_CUTOFF_SEA=0.990). We seed BOTH sides of the
    // land/sea boundary so the SDF spreads into the sea; the WGSL paints it on the sea side.
    let mut coast_seed = vec![false; n];

    // Mark a texel as a border seed if any of its 4 orthogonal neighbours differs.
    // Country: both sides land with differing non-NONE owner keys.
    // Province: differing province id (land texel; the coastline counts so the thin
    //           province line traces the coast as the engine does).
    // State:    both sides land with differing state ids.
    // Coast:    one side land, the other sea (a land/sea boundary) — seeded on both sides.
    for y in 0..h {
        for x in 0..w {
            let here = idx(x, y);
            let pid = raster.ids[here];
            let ow = owner_at[here];
            let st = state_at[here];
            let is_land = land[here];

            let mut neigh = |nx: usize, ny: usize| {
                let no = idx(nx, ny);
                if raster.ids[no] != pid && is_land {
                    province_seed[here] = true;
                }
                // coastal: this texel is on a land/sea boundary (differing land-ness).
                if land[no] != is_land {
                    coast_seed[here] = true;
                }
                if is_land && land[no] {
                    if ow != OWNER_NONE && owner_at[no] != OWNER_NONE && owner_at[no] != ow {
                        country_seed[here] = true;
                    }
                    if st != 0 && state_at[no] != 0 && state_at[no] != st {
                        state_seed[here] = true;
                    }
                }
            };
            if x > 0 {
                neigh(x - 1, y);
            }
            if x + 1 < w {
                neigh(x + 1, y);
            }
            if y > 0 {
                neigh(x, y - 1);
            }
            if y + 1 < h {
                neigh(x, y + 1);
            }
        }
    }

    let cd = edt(w, h, &country_seed);
    let pd = edt(w, h, &province_seed);
    let sd = edt(w, h, &state_seed);
    let coastd = edt(w, h, &coast_seed);

    let mut rgba = vec![0u8; n * 4];
    for i in 0..n {
        // Country/state heights are only meaningful on land (we never paint a country
        // line into the sea); the province height rides the coastline too.
        let c = if land[i] {
            distance_to_alpha_height(cd[i], COUNTRY_THICKNESS_PX)
        } else {
            0.0
        };
        let p = distance_to_alpha_height(pd[i], PROVINCE_THICKNESS_PX);
        let s = if land[i] {
            distance_to_alpha_height(sd[i], STATE_THICKNESS_PX)
        } else {
            0.0
        };
        // A channel = the COASTAL border height on the SEA side (item A). On land the WGSL
        // doesn't use A (it gates land borders by owner), so we only encode the coastal ramp
        // for SEA texels; land texels get A=0 (no coastal line painted into land — the land
        // side already has its province/country line).
        let coast = if !land[i] {
            distance_to_alpha_height(coastd[i], COAST_THICKNESS_PX)
        } else {
            0.0
        };
        let o = i * 4;
        rgba[o] = (c * 255.0 + 0.5) as u8;
        rgba[o + 1] = (p * 255.0 + 0.5) as u8;
        rgba[o + 2] = (s * 255.0 + 0.5) as u8;
        rgba[o + 3] = (coast * 255.0 + 0.5) as u8;
    }

    BorderField {
        width: raster.width,
        height: raster.height,
        rgba,
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Build a tiny raster + owner index + state map by hand.
    fn raster_from(ids: &[u32], w: u32, h: u32) -> ProvinceRaster {
        let max_id = ids.iter().copied().max().unwrap_or(0);
        ProvinceRaster {
            width: w,
            height: h,
            ids: ids.to_vec(),
            max_id,
        }
    }

    #[test]
    fn edt_single_seed_is_euclidean() {
        // 5x1 row, seed at x=0. Distances should be 0,1,2,3,4.
        let w = 5;
        let h = 1;
        let mut seed = vec![false; w * h];
        seed[0] = true;
        let d = edt(w, h, &seed);
        for x in 0..w {
            assert!((d[x] - x as f32).abs() < 1e-4, "x={x} got {}", d[x]);
        }
    }

    #[test]
    fn edt_diagonal_distance() {
        // 3x3, seed at corner (0,0). Opposite corner (2,2) must be sqrt(8) ≈ 2.828.
        let w = 3;
        let h = 3;
        let mut seed = vec![false; w * h];
        seed[0] = true;
        let d = edt(w, h, &seed);
        let corner = d[2 * w + 2];
        assert!((corner - (8.0f32).sqrt()).abs() < 1e-4, "corner={corner}");
        // adjacent diagonal (1,1) must be sqrt(2).
        let mid = d[1 * w + 1];
        assert!((mid - (2.0f32).sqrt()).abs() < 1e-4, "mid={mid}");
    }

    #[test]
    fn edt_two_seeds_takes_nearest() {
        // 7x1, seeds at x=0 and x=6. x=3 is equidistant (dist 3).
        let w = 7;
        let h = 1;
        let mut seed = vec![false; w * h];
        seed[0] = true;
        seed[6] = true;
        let d = edt(w, h, &seed);
        assert!((d[3] - 3.0).abs() < 1e-4, "mid={}", d[3]);
        assert!((d[1] - 1.0).abs() < 1e-4);
        assert!((d[5] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn distance_to_alpha_ramp() {
        // ON the border (dist 0) → 1.0; at thickness → 0; halfway → 0.5; beyond → 0.
        assert!((distance_to_alpha_height(0.0, 25.0) - 1.0).abs() < 1e-6);
        assert!((distance_to_alpha_height(25.0, 25.0) - 0.0).abs() < 1e-6);
        assert!((distance_to_alpha_height(12.5, 25.0) - 0.5).abs() < 1e-6);
        assert!((distance_to_alpha_height(40.0, 25.0) - 0.0).abs() < 1e-6);
        // INFINITY (no border reachable) → 0.
        assert_eq!(distance_to_alpha_height(f32::INFINITY, 25.0), 0.0);
    }

    #[test]
    fn distance_to_alpha_matches_outline_cutoff_band() {
        // The engine outline band is the outermost (1-0.973)*thickness px. A texel
        // exactly (1-cutoff)*thickness px from the border must encode alpha == cutoff,
        // i.e. it sits exactly at the fill/outline boundary.
        let band_px = (1.0 - OUTLINE_CUTOFF_COUNTRY) * COUNTRY_THICKNESS_PX;
        let a = distance_to_alpha_height(band_px, COUNTRY_THICKNESS_PX);
        assert!(
            (a - OUTLINE_CUTOFF_COUNTRY).abs() < 1e-4,
            "alpha at outline boundary={a}, want {OUTLINE_CUTOFF_COUNTRY}"
        );
    }

    #[test]
    fn border_field_country_seam_between_two_owners() {
        // 4x1 raster: provinces [10,10,20,20]. Owner index: pid10=ownerA, pid20=ownerB.
        // Both provinces in distinct states. The seam is between x=1 and x=2, so those
        // two texels are country-border seeds (height 1.0 → R=255), and x=0/x=3 fall off.
        let raster = raster_from(&[10, 10, 20, 20], 4, 1);
        // owner_index indexed by province id; needs len max_id+1 = 21.
        let mut owner_index = vec![OWNER_NONE; 21];
        owner_index[10] = 5; // ownerA
        owner_index[20] = 9; // ownerB
        let mut p2s = std::collections::HashMap::new();
        p2s.insert(10u32, 100u32);
        p2s.insert(20u32, 200u32);
        let ids = MapIds::new(p2s, std::collections::HashMap::new());

        let bf = build_border_field(&raster, &owner_index, &ids);
        // R channel (country height): x=1 and x=2 are ON the seam (dist 0 → 255).
        assert_eq!(bf.rgba[1 * 4], 255, "x=1 should be on the country seam");
        assert_eq!(bf.rgba[2 * 4], 255, "x=2 should be on the country seam");
        // x=0 is 2 px from the seam (country thickness 25) → ~0.92*255.
        assert!(bf.rgba[0] > 200 && bf.rgba[0] < 255, "x=0 R={}", bf.rgba[0]);
        // No sea anywhere in this raster → no coastal seed → A=0 everywhere (A is now the
        // COASTAL height, not a land mask).
        for x in 0..4 {
            assert_eq!(bf.rgba[x * 4 + 3], 0, "x={x} coastal height (no sea → 0)");
        }
        // State border seeds too (states differ across the seam) → B>0 at the seam.
        assert!(bf.rgba[1 * 4 + 2] > 0, "x=1 state height");
    }

    #[test]
    fn border_field_no_country_line_into_sea() {
        // 3x1: province 10 (land, ownerA), province 0 (sea), province 20 (land, ownerB).
        // Sea texel must never carry a country/state height (those are land-only).
        let raster = raster_from(&[10, 0, 20], 3, 1);
        let mut owner_index = vec![OWNER_NONE; 21];
        owner_index[10] = 5;
        owner_index[20] = 9;
        // pid 0 is NOT a land province (absent from p2s).
        let mut p2s = std::collections::HashMap::new();
        p2s.insert(10u32, 100u32);
        p2s.insert(20u32, 200u32);
        let ids = MapIds::new(p2s, std::collections::HashMap::new());

        let bf = build_border_field(&raster, &owner_index, &ids);
        // sea texel (x=1): country R height forced 0 (no country line into sea).
        assert_eq!(bf.rgba[1 * 4], 0, "sea must carry no country line");
        assert_eq!(bf.rgba[1 * 4 + 2], 0, "sea must carry no state line");
        // The two land texels are NOT a country seam to each other (sea between them),
        // so neither is a country-border seed → their R height is 0.
        assert_eq!(bf.rgba[0], 0, "x=0 not a country border (sea neighbour)");
        assert_eq!(
            bf.rgba[2 * 4],
            0,
            "x=2 not a country border (sea neighbour)"
        );
    }

    #[test]
    fn border_field_coastal_line_on_sea_side() {
        // 3x1: land(10) | sea(0) | land(20). The middle SEA texel is on a land/sea boundary,
        // so it must carry the COASTAL height in A (item A); the land texels carry A=0 (the
        // coastal line is painted on the sea side only).
        let raster = raster_from(&[10, 0, 20], 3, 1);
        let mut owner_index = vec![OWNER_NONE; 21];
        owner_index[10] = 5;
        owner_index[20] = 9;
        let mut p2s = std::collections::HashMap::new();
        p2s.insert(10u32, 100u32);
        p2s.insert(20u32, 200u32);
        let ids = MapIds::new(p2s, std::collections::HashMap::new());

        let bf = build_border_field(&raster, &owner_index, &ids);
        // sea texel (x=1) is ON the coast (dist 0 → A=255).
        assert_eq!(
            bf.rgba[1 * 4 + 3],
            255,
            "sea coast texel A (coastal height)"
        );
        // land texels carry no coastal height (sea-side only).
        assert_eq!(bf.rgba[0 * 4 + 3], 0, "x=0 land carries no coastal height");
        assert_eq!(bf.rgba[2 * 4 + 3], 0, "x=2 land carries no coastal height");
    }
}
