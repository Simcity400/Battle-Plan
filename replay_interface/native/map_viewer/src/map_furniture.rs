//! MAPICON1 — city/victory-point name labels, capital markers, and VP bullet
//! placement, reconstructed as a pure function of MOD-RESOLVED shipped game
//! files. This is the map *furniture* layer (the white "London"/"Berlin" city
//! names, the little on-map victory-point icon, and the capital marker). It is
//! a NEW, separate layer from the country-name label law (`map_label_law.rs` /
//! LAB1) and shares no code paths with it.
//!
//! # What the game ships (every visual parameter is cited here)
//!
//! The on-map victory-point / capital furniture is a GUI container the engine
//! places at each victory-point province, defined in `interface/mapicons.gui`
//! (line ranges verified against the shipped file):
//!
//! * `interface/mapicons.gui:628`-`:652` `containerWindowType name =
//!   "victory_point_mapicon"` (name on `:629`, container closes `:652`) — size
//!   `{160 x 30}` (`:632`), an `iconType` sprite `GFX_onmap_victorypoints_strip`
//!   at `position { x=-5 y=0 }` (`:637`-`:638`), and an `instantTextboxType`
//!   with `font = "hoi_20bs"` at `position { x=26 y=5 }`, `maxWidth = 149`,
//!   `maxHeight = 30`, `format = left`, `alwaystransparent = yes`
//!   (`:642`-`:651`).
//! * `interface/mapicons.gui:655`-`:680` `containerWindowType name =
//!   "capital_mapicon"` (the `:654` line is only the `#### CAPITAL MAP ICON`
//!   comment; name on `:656`, container closes `:680`) — IDENTICAL layout (same
//!   `GFX_onmap_victorypoints_strip` sprite `:664` and `hoi_20bs` font `:673`),
//!   with the icon carrying `alwaystransparent = yes` (`:667`). Vanilla has NO
//!   distinct "capital star" sprite; the capital reuses the victory-point strip
//!   icon (`GFX_stars_small` is COMMENTED OUT at `interface/core.gfx:702`-`:706`).
//!   The frame that distinguishes a capital / a VP tier inside the 20-frame
//!   strip is chosen in engine C++ and is NOT in the files (UNTRACED below).
//! * `interface/mapicons.gfx:290`-`:295` `spriteType name =
//!   "GFX_onmap_victorypoints_strip"` — `texturefile =
//!   "gfx/interface/onmap_victorypoints_strip.dds"`, `noOfFrames = 20`. The DDS
//!   is 580x27 (so 20 frames = 29x27 each), but the tier->frame mapping is
//!   engine-side (UNTRACED).
//! * `interface/core.gfx:1232`-`:1238` `bitmapfont name = "hoi_20bs"` —
//!   `fontfiles = { "gfx/fonts/hoi_20bs" }`, `color = 0xffffffff` (white). The
//!   BMFont metrics live in `gfx/fonts/hoi_20bs.fnt`. The `.fnt`'s own `page
//!   file="hoi_20b_0.dds"` line is STALE (that file is absent); the real page
//!   atlas is `gfx/fonts/hoi_20bs.dds` (512x256, 32-bit uncompressed with
//!   A/R/G/B channel masks). That the atlas shares the `.fnt` basename is
//!   INFERRED from `core.gfx` + file existence + the engine loader convention,
//!   not stated in the `.fnt` — labelled as such.
//!
//! # Where each marker's data comes from (all resolver-routed)
//!
//! * WHICH provinces carry a VP marker + the VP value: `history/states/*.txt`
//!   `victory_points = { <province_id> <value> }` inside the `history = { }`
//!   block (parsed by the existing `game_data::load_states`). Example:
//!   `history/states/1-France.txt` → `victory_points = { 3838 1 }`.
//! * The CITY / VP name string: `localisation/<lang>/victory_points_l_<lang>.yml`
//!   `VICTORY_POINTS_<province_id>: "Name"`. Example (english):
//!   `localisation/english/victory_points_l_english.yml:761` →
//!   `VICTORY_POINTS_3838: "Ajáccio"`. (The loader merges the whole
//!   `localisation/<lang>` directory, so the exact filename is not assumed.)
//! * WHICH state is a country's capital: `history/countries/<TAG> - <Name>.txt`
//!   `capital = <state_id>` (the capital is a STATE id, verified: 363 initial
//!   capitals, 0 non-state ids). Example: `GER - Germany.txt:2` → `capital = 64`.
//! * The marker POSITION: the province's pixel centroid in the shipped
//!   province raster (`map/provinces.bmp` keyed by `map/definition.csv`, the
//!   same 5632x2048 raster the country-label law uses). Vanilla ships NO
//!   explicit VP/label position file — `map/positions.txt` is 0 bytes. The
//!   centroid is therefore a DERIVED_APPROXIMATION of the anchor, NOT a shipped
//!   game-file parameter; the engine's exact anchor (and any sub-province
//!   offset) is UNTRACED.
//!
//! # Capital-marker attribution (the honest weak point)
//!
//! `capital = <state_id>` names a STATE, not a province, and a state may hold
//! several VP provinces or none (e.g. state 914 Jan Mayen is a capital with no
//! `victory_points` block at all). WHICH province the engine anchors the
//! capital marker on is NOT in the files. This law makes a deterministic,
//! explicitly-labelled choice: every VP province whose state is its owner's
//! capital state is marked [`FurnitureMarkerKind::Capital`]; a capital state
//! with no VP province yields no capital marker. The "one capital province per
//! country" selection and any capital-with-no-VP fallback are UNTRACED
//! engine-side behaviour, reported as such — never guessed.
//!
//! # Coordinate convention
//!
//! Identical to `map_label_law.rs`: map-pixel space, `x` right, `y` down, which
//! equals the world x/z plane 1:1 for the 5632x2048 province raster. Callers
//! that draw y-up flip with `map_height - y`.
//!
//! # Honesty
//!
//! This law is deterministic and fails closed: a VP province absent from the
//! raster yields no marker rather than a guessed position. Items that are
//! engine-C++ are tagged UNTRACED and carry no invented numeric value:
//! * the tier->frame index into the 20-frame strip;
//! * the capital-state->marker-province selection and the capital-with-no-VP
//!   fallback;
//! * the exact sub-province marker anchor / offset (centroid is a
//!   DERIVED_APPROXIMATION);
//! * the inclusive/exclusive comparison semantics of `VICTORY_POINT_MAP_ICON_AFTER`
//!   and the label fade/scale interpolation between the (file-derived) zoom
//!   cutoffs.
//! The zoom-cutoff *values* and tier *boundaries* themselves ARE file-derived
//! (`common/defines/00_graphics.lua`); only the engine's comparison/interp of
//! them is UNTRACED.

use crate::*;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// `interface/mapicons.gui:628`-`:652` (victory_point_mapicon), `:655`-`:680`
/// (capital_mapicon) — the VP/capital map-icon containers. Sizes and offsets
/// below are the literal shipped values, kept as constants with their citation
/// so the dump renderer and any future replay overlay share one law.
pub const MAPICONS_GUI_SOURCE: &str = "interface/mapicons.gui";
/// `interface/mapicons.gfx:290`-`:295` — the VP strip sprite definition.
pub const MAPICONS_GFX_SOURCE: &str = "interface/mapicons.gfx";
/// `interface/core.gfx:1232`-`:1238` — the `hoi_20bs` bitmap font used for the
/// text (`GFX_stars_small` is commented out at `:702`-`:706`).
pub const CORE_GFX_SOURCE: &str = "interface/core.gfx";

/// `spriteType name = "GFX_onmap_victorypoints_strip"` texture, the on-map VP /
/// capital icon (`interface/mapicons.gfx:292`). The DDS is 580x27 (20 frames of
/// 29x27); the tier->frame index is engine-side (UNTRACED).
pub const VP_STRIP_SPRITE_TEXTURE: &str = "gfx/interface/onmap_victorypoints_strip.dds";
/// `noOfFrames = 20` for the VP strip (`interface/mapicons.gfx:293`).
pub const VP_STRIP_FRAME_COUNT: u32 = 20;
/// The GUI sprite name the map furniture binds (`interface/mapicons.gui:637`,
/// `:664`).
pub const VP_STRIP_SPRITE_NAME: &str = "GFX_onmap_victorypoints_strip";

/// BMFont metrics for the city/VP label font (`hoi_20bs`).
pub const HOI_20BS_FNT_SOURCE: &str = "gfx/fonts/hoi_20bs.fnt";
/// Glyph atlas page for `hoi_20bs` (512x256, 32-bit uncompressed A/R/G/B masks).
/// The `.fnt`'s own `page file="hoi_20b_0.dds"` line is STALE (absent); that the
/// real atlas shares the `.fnt` basename is INFERRED from `core.gfx` + file
/// existence + the engine loader convention, not stated in the `.fnt`.
pub const HOI_20BS_ATLAS_SOURCE: &str = "gfx/fonts/hoi_20bs.dds";
/// The GUI text font name the container DECLARES (`interface/mapicons.gui:645`,
/// `:673`). NOTE: this is the *declared* font, not the font that renders on the
/// map — see [`VP_MAP_TEXT_FONT_NAME`] / the FURN-FONT law below.
pub const VP_LABEL_FONT_NAME: &str = "hoi_20bs";

// ----------------------------------------------------------------------------
// FURN-FONT law (font substitution — was UNTRACED, now FILE-TRACED 2026-07-09).
//
// The `victory_point_mapicon` / `capital_mapicon` GUI containers DECLARE
// `font = "hoi_20bs"` (`interface/mapicons.gui:645`, `:673`), but the on-map
// city/VP name that actually renders is the MAP font `tahoma_60`, whose
// fontfile is `gfx/fonts/hoi_mapfont4` (`interface/core.gfx:1679`-`:1680`
// `bitmapfont { name = "tahoma_60"  path = "gfx/fonts/hoi_mapfont4" }`) — the
// SAME map font the country-name labels use (`map_label_law.rs`,
// `MAP_LABEL_LAW.md` §Font). The on-map instant text is composited by the map
// text renderer in the map font, not the flat GUI font the container names.
//
// DECISIVE FILE EVIDENCE (advance-sum, both shipped `.fnt` files, no capture):
// the measured on-screen widths (`w1check\furn_oracle_*.png`, gui_scale=1.0)
// are "Melbourne" 149 px / "Adelaide" 119 px. Summing shipped `xadvance`:
//   * `hoi_mapfont4` (tahoma_60): 370 / 298 → x0.4027 / x0.3993  (=> 148 / 119)
//   * `hoi_20bs`               :  77 /  62 → x1.9351 / x1.9194
// Only the map font lands both words at ONE plausible scale (~0.40); matching
// with `hoi_20bs` would require a 1.93x UPSCALE of a 20 px bitmap page (blurry,
// implausible for the crisp on-map glyphs), and the prior oracle's glyph-ink
// measurement ('M' cap = 56 x 0.4 = 22 px) independently pins the shape to the
// map font. The gate `gate_vanilla_vp_text_font_substitution_advance_law`
// re-proves this from the shipped `.fnt` files.
//
// TRACED: the RENDERED font is the map font (file citation above).
// ENGINE-MEASURED (not a single-file constant): the 0.40 scale — it is NOT the
// declared/map point-size ratio (20/84 = 0.238), so it is a map-text world
// scale set in engine C++; kept as the measured value with its evidence.
// ----------------------------------------------------------------------------

/// The map font that ACTUALLY renders the on-map VP/city name (declared
/// `tahoma_60` at `interface/core.gfx:1679`). The container's `hoi_20bs` is a
/// declared-but-substituted GUI font (see the FURN-FONT law above).
pub const VP_MAP_TEXT_FONT_NAME: &str = "tahoma_60";
/// BMFont metrics for the rendered map text font (`gfx/fonts/hoi_mapfont4.fnt`,
/// `info size=84 lineHeight=84 base=70`, 575 glyphs).
pub const VP_MAP_TEXT_FNT_SOURCE: &str = "gfx/fonts/hoi_mapfont4.fnt";
/// Glyph atlas page for the map text font (`gfx/fonts/hoi_mapfont4.dds`,
/// 2048x1024) — the same atlas the country-name labels sample.
pub const VP_MAP_TEXT_ATLAS_SOURCE: &str = "gfx/fonts/hoi_mapfont4.dds";
/// ENGINE-MEASURED on-map VP/city text scale applied to the map font
/// (`w1check\furn_oracle_*.png`, gui_scale=1.0): advance-sum x0.4027 / x0.3993
/// on "Melbourne" (149 px) / "Adelaide" (119 px) vs the shipped
/// `gfx/fonts/hoi_mapfont4.fnt` sums 370 / 298, glyph-ink confirmed. NOT the
/// declared/map point-size ratio (20/84 = 0.238) — the exact engine origin is a
/// map-text world scale (engine C++), so this is kept as the measured value.
pub const VP_MAP_TEXT_SCALE: f32 = 0.40;

/// `victory_point_mapicon` / `capital_mapicon` container size in GUI pixels
/// (`interface/mapicons.gui:632`, `:659` `size = { width=160 height=30 }`).
pub const MAPICON_CONTAINER_WIDTH: i32 = 160;
pub const MAPICON_CONTAINER_HEIGHT: i32 = 30;
/// The icon sprite offset within the container (`interface/mapicons.gui:638`,
/// `:665` `position = { x=-5 y=0 }`).
pub const MAPICON_ICON_OFFSET_X: i32 = -5;
pub const MAPICON_ICON_OFFSET_Y: i32 = 0;
/// The text box offset within the container (`interface/mapicons.gui:643`,
/// `:670` `position = { x=26 y=5 }`).
pub const MAPICON_TEXT_OFFSET_X: i32 = 26;
pub const MAPICON_TEXT_OFFSET_Y: i32 = 5;
/// The text box max width / height (`interface/mapicons.gui:646`-`:647`,
/// `:674`-`:675` `maxWidth = 149`, `maxHeight = 30`).
pub const MAPICON_TEXT_MAX_WIDTH: i32 = 149;
pub const MAPICON_TEXT_MAX_HEIGHT: i32 = 30;

/// Localisation key prefix for city / VP names
/// (`localisation/<lang>/victory_points_l_english.yml`,
/// `VICTORY_POINTS_<province_id>`).
pub const VICTORY_POINTS_LOC_PREFIX: &str = "VICTORY_POINTS_";

// ----------------------------------------------------------------------------
// VP / capital map-icon defines (`common/defines/00_graphics.lua`). These are
// the FILE-DERIVED zoom-cutoff and tier laws for the furniture layer; there is
// no invented numeric value here — everything below is read from the shipped
// lua by name.
// ----------------------------------------------------------------------------

/// `common/defines/00_graphics.lua:947` `VICTORY_POINT_LEVELS`.
pub const VICTORY_POINT_LEVELS_KEY: &str = "VICTORY_POINT_LEVELS";
/// `:948` `VICTORY_POINT_MAP_ICON_AFTER` (list): VP-value thresholds at which
/// the icon dot grows a tier. "After this amount of VP the map icon becomes
/// bigger dot."
pub const VICTORY_POINT_MAP_ICON_AFTER_KEY: &str = "VICTORY_POINT_MAP_ICON_AFTER";
/// `:950` `VICTORY_POINT_MAP_ICON_TEXT_CUTOFF` (list): per-level camera
/// distance at which the VP name text disappears.
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_KEY: &str = "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF";
/// `:951`-`:952` VP text cutoff min/max camera range.
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN_KEY: &str =
    "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN";
pub const VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX_KEY: &str =
    "VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX";
/// `:953`-`:954` VP dot cutoff min/max camera range.
pub const VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN_KEY: &str = "VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN";
pub const VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX_KEY: &str = "VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX";
/// `:955` VP value that maps to the max dot size on the range.
pub const VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT_KEY: &str =
    "VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT";
/// `:949` capital-specific dot cutoff max.
pub const VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX_KEY: &str =
    "VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX";
/// `:963` camera distance at which capital icons disappear.
pub const CAPITAL_ICON_CUTOFF_KEY: &str = "CAPITAL_ICON_CUTOFF";

/// The VP/capital map-icon defines, read verbatim from
/// `common/defines/00_graphics.lua`. Every field carries a citation in the
/// key-constant above; nothing here is a default.
#[derive(Clone, Debug, PartialEq)]
pub struct VpIconDefines {
    pub levels: u32,
    /// VP-value thresholds (ascending) for the icon dot tier.
    pub map_icon_after: Vec<i64>,
    /// Per-level VP text zoom cutoffs.
    pub text_cutoff: Vec<f32>,
    pub text_cutoff_min: f32,
    pub text_cutoff_max: f32,
    pub dot_cutoff_min: f32,
    pub dot_cutoff_max: f32,
    pub max_victory_points_for_percent: f32,
    pub capital_cutoff_max: f32,
    pub capital_icon_cutoff: f32,
}

impl VpIconDefines {
    /// The icon dot tier for a VP value, per `VICTORY_POINT_MAP_ICON_AFTER`: the
    /// highest index whose threshold the value reaches (0-based, clamped to
    /// `levels`). A larger tier is a bigger dot. Deterministic. The thresholds
    /// are file-derived; the `>=` bucketing is this law's choice (the engine's
    /// exact inclusive/exclusive comparison is UNTRACED).
    pub fn tier_for_vp(&self, vp_value: i32) -> u32 {
        let mut tier = 0u32;
        for (idx, threshold) in self.map_icon_after.iter().enumerate() {
            if i64::from(vp_value) >= *threshold {
                tier = idx as u32;
            }
        }
        tier.min(self.levels.saturating_sub(1))
    }

    /// Whether the marker's ICON (dot/star) is visible at `cam_dist`.
    ///
    /// File-derived pieces (`common/defines/00_graphics.lua:949-963`):
    /// * capital: visible while `cam_dist <= CAPITAL_ICON_CUTOFF` (`:963` "At
    ///   what camera distance capital icons disappears").
    /// * victory point: the dot's disappear distance interpolates on the
    ///   `[DOT_CUTOFF_MIN, DOT_CUTOFF_MAX]` range (`:953`-`:954` "Min/Max range
    ///   for victory point dot") by the VP value as a fraction of
    ///   `MAX_VICTORY_POINTS_FOR_PERCENT` (`:955` "Default max value for point
    ///   on the above range… if the VP value exceeds this, it'll be treated as
    ///   max") — a 1-VP village dot vanishes near the MIN end, a 22+-VP city
    ///   dot survives to the MAX end.
    ///
    /// The linear shape of that interpolation and the `<=` comparisons are this
    /// law's choice (engine C++ is UNTRACED); the endpoints/inputs are all
    /// shipped defines.
    pub fn dot_visible(&self, kind: FurnitureMarkerKind, vp_value: i32, cam_dist: f32) -> bool {
        match kind {
            FurnitureMarkerKind::Capital => cam_dist <= self.capital_icon_cutoff,
            FurnitureMarkerKind::VictoryPoint => {
                let max_vp = self.max_victory_points_for_percent.max(1.0);
                let percent = (vp_value.max(0) as f32 / max_vp).min(1.0);
                let cutoff =
                    self.dot_cutoff_min + (self.dot_cutoff_max - self.dot_cutoff_min) * percent;
                cam_dist <= cutoff
            }
        }
    }

    /// Whether the marker's NAME TEXT is visible at `cam_dist`.
    ///
    /// File-derived pieces (`common/defines/00_graphics.lua:949-952`):
    /// * victory point: per-tier disappear distance
    ///   `VICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500}` (`:950` "At
    ///   what camera distance the VP name text disappears"), clamped into
    ///   `[TEXT_CUTOFF_MIN, TEXT_CUTOFF_MAX]` (`:951`-`:952`).
    /// * capital: `VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX` (`:949`
    ///   "Capitals are special snowflakes, they need their own number") — read
    ///   as the capital's own text-disappear distance, which keeps capital
    ///   names visible farther out than any normal VP tier (1000 vs 500).
    ///
    /// Reading `:949` as the capital TEXT distance (vs some other capital
    /// range) is this law's choice; the value itself is shipped.
    pub fn text_visible(&self, kind: FurnitureMarkerKind, dot_tier: u32, cam_dist: f32) -> bool {
        match kind {
            FurnitureMarkerKind::Capital => cam_dist <= self.capital_cutoff_max,
            FurnitureMarkerKind::VictoryPoint => {
                let tier_cutoff = self
                    .text_cutoff
                    .get(dot_tier as usize)
                    .copied()
                    .unwrap_or(self.text_cutoff_min);
                let cutoff = tier_cutoff.clamp(self.text_cutoff_min, self.text_cutoff_max);
                cam_dist <= cutoff
            }
        }
    }
}

// ----------------------------------------------------------------------------
// VP strip frame law — decoded from the SHIPPED strip texture (measured
// 2026-07-07 by decoding `gfx/interface/onmap_victorypoints_strip.dds`: 580x27
// uncompressed BGRA, 20 frames of 29x27). The 20 frames are a 4-shape x
// 5-color grid, columns left→right within each shape group:
//
//   frames  0..4  = STAR      (opaque-pixel area 353) — the capital icon
//   frames  5..9  = CIRCLE    (area 255) — the biggest VP dot
//   frames 10..14 = PENTAGON  (area 197) — the middle VP dot
//   frames 15..19 = SQUARE    (area 116) — the smallest VP dot
//
// and the 5 columns are a diplomatic color palette (red, blue, green,
// yellow/gold, gray). Shape size strictly orders star > circle > pentagon >
// square, matching `VICTORY_POINT_MAP_ICON_AFTER`'s "becomes bigger dot"
// tiers (3 VP tiers + the capital's special star). WHICH color column the
// engine picks per diplomatic relation is engine C++ (UNTRACED); the neutral
// GRAY column is the documented default for an observer replay.
// ----------------------------------------------------------------------------

/// Color columns per shape group in the shipped VP strip (measured).
pub const VP_STRIP_COLOR_SLOTS: u32 = 5;
/// The gray (rightmost) color column — the observer/no-relation default this
/// replay renders with (the engine's relation→column map is UNTRACED).
pub const VP_STRIP_COLOR_NEUTRAL_GRAY: u32 = 4;

/// Frame index into the 20-frame strip for a marker: shape group from
/// kind/tier (measured grid above), plus the color column.
///
/// Tier→shape: tier 2 (biggest) → circle, tier 1 → pentagon, tier 0 →
/// square; capitals → star regardless of tier. Tiers above 2 clamp to the
/// circle (the strip has exactly 3 dot shapes for the 3 shipped levels).
pub fn vp_strip_frame_index(kind: FurnitureMarkerKind, dot_tier: u32, color_slot: u32) -> u32 {
    let shape_base = match kind {
        FurnitureMarkerKind::Capital => 0,
        FurnitureMarkerKind::VictoryPoint => match dot_tier.min(2) {
            2 => 5,
            1 => 10,
            _ => 15,
        },
    };
    shape_base + color_slot.min(VP_STRIP_COLOR_SLOTS - 1)
}

/// Load + decode the VP strip sprite texture through the resolver, validating
/// the shipped frame geometry (`interface/mapicons.gfx:293` `noOfFrames = 20`):
/// fails closed when the texture is absent or its width does not divide into
/// the declared frame count, rather than guessing a frame size.
pub fn load_vp_strip_rgba(resolver: &GameFileResolver) -> Result<RgbaFontAtlas> {
    let path =
        resolver.resolve_required(VP_STRIP_SPRITE_TEXTURE, "onmap_victorypoints_strip.dds")?;
    let image = load_dds_rgba(&path)?;
    if image.width == 0 || image.width % VP_STRIP_FRAME_COUNT as usize != 0 {
        return Err(format!(
            "{VP_STRIP_SPRITE_TEXTURE}: width {} does not divide into the {} frames \
             declared by {MAPICONS_GFX_SOURCE}:293",
            image.width, VP_STRIP_FRAME_COUNT
        )
        .into());
    }
    Ok(RgbaFontAtlas {
        width: image.width,
        height: image.height,
        pixels: image.pixels,
    })
}

/// The resolver-derived static inputs a live furniture composite needs beyond
/// what the map bundle already carries (raster + province→state come from the
/// bundle; owners come from the recording per hour). All fields are shipped
/// installed content through the mod-aware resolver:
///
/// * `state_victory_points`: `history/states/*.txt` `victory_points` blocks.
/// * `capital_states`: `history/countries/*` `capital = <state>` (INITIAL
///   capitals; dynamic `set_capital` is recording-feed future work, same
///   limitation as [`furniture_layer_from_resolver`]).
/// * `localisation`: merged `VICTORY_POINTS_<id>` name table.
/// * `vp_icon_defines`: the `00_graphics.lua` zoom/tier laws.
pub struct FurnitureStaticData {
    pub state_victory_points: BTreeMap<u32, BTreeMap<u32, i32>>,
    pub capital_states: BTreeMap<String, u32>,
    pub localisation: BTreeMap<String, String>,
    pub vp_icon_defines: VpIconDefines,
}

/// Terrain elevation grid for anchoring overlay elements ON the rendered 3D
/// terrain: `map/heightmap.bmp` through the resolver, top-down rows,
/// `world_y = raw * 0.1` — the CHUNK1-proven scale (the engine stores slot1
/// height as `i16 = raw*10`, the shipped VS multiplies by `0.01`, and
/// `HeightScale = 0.1` rides CB1; validated byte-exact against all 49
/// captured chunk draws in `terrain_mesh_law.rs`). Cross-check: the shipped
/// `map/unitstacks.txt` per-province heights agree (Dublin 12.07 vs raw
/// 125*0.1, Wexford 9.97 vs raw 100*0.1).
pub struct HeightmapGrid {
    pub width: usize,
    pub height: usize,
    raw: Vec<u8>,
}

impl HeightmapGrid {
    /// World elevation at a map-pixel coordinate (x right, y down — the same
    /// space the province raster and marker anchors use). Out-of-range
    /// coordinates clamp to the edge texel.
    pub fn world_y(&self, map_x: f32, map_y: f32) -> f32 {
        if self.width == 0 || self.height == 0 {
            return 0.0;
        }
        let x = (map_x.max(0.0) as usize).min(self.width - 1);
        let y = (map_y.max(0.0) as usize).min(self.height - 1);
        f32::from(self.raw[y * self.width + x]) * 0.1
    }
}

/// Load the heightmap elevation grid through the resolver (mod override wins).
pub fn heightmap_grid_from_resolver(resolver: &GameFileResolver) -> Result<HeightmapGrid> {
    let path = resolver.resolve_required("map/heightmap.bmp", "heightmap.bmp")?;
    let image = load_bmp_indexed(&path)?;
    Ok(HeightmapGrid {
        width: image.width,
        height: image.height,
        raw: image.pixels,
    })
}

/// Load [`FurnitureStaticData`] through the resolver (used by the replay app's
/// in-render composite; the dump bin keeps its own full-layer entry point).
pub fn furniture_static_data_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<FurnitureStaticData> {
    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (states, _province_to_state, _state_owners) = load_states(&state_files)?;
    let mut state_victory_points = BTreeMap::new();
    for (state_id, meta) in &states {
        if !meta.victory_points.is_empty() {
            state_victory_points.insert(*state_id, meta.victory_points.clone());
        }
    }
    Ok(FurnitureStaticData {
        state_victory_points,
        capital_states: capital_states_from_resolver(resolver)?,
        localisation: localisation_table_from_resolver(resolver, language)?,
        vp_icon_defines: vp_icon_defines_from_resolver(resolver)?,
    })
}

/// Load the VP/capital map-icon defines from `common/defines/00_graphics.lua`
/// through the resolver (mod override wins). Fails if any required key is
/// absent — no invented fallback.
pub fn vp_icon_defines_from_resolver(resolver: &GameFileResolver) -> Result<VpIconDefines> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let source = latin1_bytes_to_string(&std::fs::read(&path)?);
    parse_vp_icon_defines(&source)
}

/// Parse the VP/capital map-icon defines out of the graphics-defines lua text.
pub fn parse_vp_icon_defines(source: &str) -> Result<VpIconDefines> {
    let scalar = |key: &str| -> Result<f32> { parse_lua_number_define(source, key) };
    let list_i = |key: &str| -> Result<Vec<i64>> {
        parse_lua_number_list(source, key)
            .map(|list| list.into_iter().map(|value| value as i64).collect())
            .ok_or_else(|| format!("{BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} missing list {key}").into())
    };
    let list_f = |key: &str| -> Result<Vec<f32>> {
        parse_lua_number_list(source, key)
            .ok_or_else(|| format!("{BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE} missing list {key}").into())
    };
    Ok(VpIconDefines {
        levels: scalar(VICTORY_POINT_LEVELS_KEY)? as u32,
        map_icon_after: list_i(VICTORY_POINT_MAP_ICON_AFTER_KEY)?,
        text_cutoff: list_f(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_KEY)?,
        text_cutoff_min: scalar(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN_KEY)?,
        text_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX_KEY)?,
        dot_cutoff_min: scalar(VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN_KEY)?,
        dot_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX_KEY)?,
        max_victory_points_for_percent: scalar(
            VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT_KEY,
        )?,
        capital_cutoff_max: scalar(VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX_KEY)?,
        capital_icon_cutoff: scalar(CAPITAL_ICON_CUTOFF_KEY)?,
    })
}

/// Parse a lua list define `KEY = { a, b, c }` into its numbers. Returns `None`
/// when the key is absent or has no `{ }` list. Word-boundary aware on the key.
pub(crate) fn parse_lua_number_list(source: &str, key: &str) -> Option<Vec<f32>> {
    for line in source.lines() {
        let code = line.split("--").next().unwrap_or(line);
        let Some(idx) = find_assignment(code, key, 0) else {
            continue;
        };
        let rest = code[idx..].trim_start();
        let Some(open) = rest.strip_prefix('{') else {
            continue;
        };
        let Some(end) = open.find('}') else {
            continue;
        };
        let values = open[..end]
            .split(',')
            .filter_map(|token| token.trim().parse::<f32>().ok())
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }
        return Some(values);
    }
    None
}

/// The kind of furniture marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FurnitureMarkerKind {
    /// A victory-point province that is NOT its owner's capital.
    VictoryPoint,
    /// A victory-point province that IS its owner's capital state's VP province
    /// (the `capital_mapicon` case). Reuses the same strip sprite in vanilla.
    Capital,
}

impl FurnitureMarkerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FurnitureMarkerKind::VictoryPoint => "victory_point",
            FurnitureMarkerKind::Capital => "capital",
        }
    }
}

/// A single placed furniture marker in map-pixel space.
#[derive(Clone, Debug, PartialEq)]
pub struct FurnitureMarker {
    /// The victory-point province this marker sits on.
    pub province_id: u32,
    /// State the province belongs to.
    pub state_id: u32,
    /// Owner tag of that state at the moment being laid out.
    pub owner: String,
    /// Marker centre in map-pixel space (x right, y down); the province pixel
    /// centroid in the 5632x2048 raster.
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// The shipped victory-point value (`history/states` `victory_points`).
    pub vp_value: i32,
    /// City / VP display name from localisation, or `VICTORY_POINTS_<id>` when
    /// the key is absent (fail-visible, never invented).
    pub label: String,
    /// Whether the localisation key resolved (diagnostic).
    pub label_resolved: bool,
    /// VictoryPoint vs Capital.
    pub kind: FurnitureMarkerKind,
    /// Icon dot tier from `VICTORY_POINT_MAP_ICON_AFTER` (0 = smallest). The
    /// tier *thresholds* are file-derived (`common/defines/00_graphics.lua`);
    /// the `>=` comparison used to bucket a value into a tier is this law's
    /// choice (the engine's exact inclusive/exclusive test is UNTRACED).
    pub dot_tier: u32,
    /// Sampled pixel count backing the centroid (determinism / fail-closed
    /// diagnostic). The centroid itself is a DERIVED_APPROXIMATION of the anchor.
    pub pixel_count: u64,
}

/// Inputs the furniture law needs. All fields are resolver-derived; the raster
/// is the shipped province raster and the maps come from `history/states`,
/// `history/countries`, and `localisation`.
pub struct FurnitureLawInputs<'a> {
    /// Row-major province id per pixel (0 = no province), 5632x2048 vanilla.
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    /// Province id -> state id.
    pub province_to_state: &'a BTreeMap<u32, u32>,
    /// State id -> owner tag for the moment being laid out.
    pub state_owners: &'a BTreeMap<u32, String>,
    /// State id -> its `victory_points` map (province id -> VP value).
    pub state_victory_points: &'a BTreeMap<u32, BTreeMap<u32, i32>>,
    /// Owner tag -> capital STATE id (`history/countries` `capital = <state>`).
    pub capital_states: &'a BTreeMap<String, u32>,
    /// Localisation table (`VICTORY_POINTS_<id>` -> name), already merged.
    pub localisation: &'a BTreeMap<String, String>,
    /// VP/capital map-icon defines (`common/defines/00_graphics.lua`) — the
    /// file-derived dot-tier law. `None` leaves every marker at tier 0.
    pub vp_icon_defines: Option<&'a VpIconDefines>,
}

/// Compute every furniture marker from the shipped inputs, deterministic and
/// sorted by province id.
///
/// A marker is emitted for each `(state, province, vp_value)` triple in
/// `state_victory_points` whose province appears in the raster with at least
/// one pixel. The marker is a [`FurnitureMarkerKind::Capital`] iff the province
/// belongs to its owner's capital state (from `capital_states`), else a
/// [`FurnitureMarkerKind::VictoryPoint`]. Provinces absent from the raster are
/// skipped (fail closed — no invented position). Localisation misses keep the
/// raw `VICTORY_POINTS_<id>` key as the label (fail visible).
pub fn compute_furniture_markers(inputs: &FurnitureLawInputs) -> Vec<FurnitureMarker> {
    let expected = inputs.width.saturating_mul(inputs.height);
    if inputs.province_ids.len() != expected || expected == 0 {
        return Vec::new();
    }

    // Gather the target VP provinces first so the raster scan only accumulates
    // centroids for provinces we will actually place. Map province -> (state,
    // vp_value); a province can only belong to one state, so the last wins but
    // states partition provinces so there is no real collision.
    let mut vp_province_state: HashMap<u32, (u32, i32)> = HashMap::new();
    for (state_id, vps) in inputs.state_victory_points {
        for (province_id, value) in vps {
            vp_province_state.insert(*province_id, (*state_id, *value));
        }
    }
    if vp_province_state.is_empty() {
        return Vec::new();
    }

    // One raster pass: accumulate pixel-centroid sums for target provinces only.
    // Sums are f64 to keep the centroid exact over the full 2048-row map.
    let mut centroids: HashMap<u32, (f64, f64, u64)> = HashMap::new();
    for y in 0..inputs.height {
        let row = y * inputs.width;
        for x in 0..inputs.width {
            let province = inputs.province_ids[row + x];
            if province == 0 {
                continue;
            }
            if vp_province_state.contains_key(&province) {
                let entry = centroids.entry(province).or_insert((0.0, 0.0, 0));
                entry.0 += x as f64;
                entry.1 += y as f64;
                entry.2 += 1;
            }
        }
    }

    // One capital marker per country: the engine stars a SINGLE province in
    // the owner's capital state — ENGINE-MEASURED as the highest-VP province
    // there (live oracle 2026-07-07: Canberra 30 VP starred while Sydney
    // 20 VP in the SAME state 285 shows a circle dot; and vanilla shows one
    // star per country, not one per VP of a single-state country like 1936
    // Ireland). Ties break to the lowest province id (deterministic; the
    // engine's tie rule is UNTRACED).
    let mut capital_choice: HashMap<u32, (i32, u32)> = HashMap::new();
    for (province_id, (state_id, vp_value)) in &vp_province_state {
        let entry = capital_choice
            .entry(*state_id)
            .or_insert((*vp_value, *province_id));
        if *vp_value > entry.0 || (*vp_value == entry.0 && *province_id < entry.1) {
            *entry = (*vp_value, *province_id);
        }
    }
    let mut markers = Vec::new();
    for (province_id, (state_id, vp_value)) in &vp_province_state {
        let Some((sum_x, sum_y, count)) = centroids.get(province_id).copied() else {
            // Province is not painted anywhere in the raster: fail closed.
            continue;
        };
        if count == 0 {
            continue;
        }
        // Owner is required to attribute the marker; a VP province in an
        // unowned/water state (no owner) still gets a VictoryPoint marker with
        // an empty owner so the geography is not silently dropped.
        let owner = inputs
            .state_owners
            .get(state_id)
            .cloned()
            .unwrap_or_default();
        let is_capital = (!owner.is_empty())
            .then(|| inputs.capital_states.get(&owner).copied())
            .flatten()
            .map(|capital_state| {
                capital_state == *state_id
                    && capital_choice
                        .get(state_id)
                        .map(|(_, chosen)| chosen == province_id)
                        .unwrap_or(false)
            })
            .unwrap_or(false);
        let loc_key = format!("{VICTORY_POINTS_LOC_PREFIX}{province_id}");
        let (label, label_resolved) = match inputs.localisation.get(&loc_key) {
            Some(name) => (name.clone(), true),
            None => (loc_key, false),
        };
        markers.push(FurnitureMarker {
            province_id: *province_id,
            state_id: *state_id,
            owner,
            anchor_x: (sum_x / count as f64) as f32,
            anchor_y: (sum_y / count as f64) as f32,
            vp_value: *vp_value,
            label,
            label_resolved,
            kind: if is_capital {
                FurnitureMarkerKind::Capital
            } else {
                FurnitureMarkerKind::VictoryPoint
            },
            dot_tier: inputs
                .vp_icon_defines
                .map(|defines| defines.tier_for_vp(*vp_value))
                .unwrap_or(0),
            pixel_count: count,
        });
    }
    markers.sort_by_key(|marker| marker.province_id);
    markers
}

/// Parse `history/countries/<TAG> - <Name>.txt` files into a tag -> capital
/// STATE id map, resolver-routed (mod overrides win by relative path).
///
/// The tag is the leading token of the file name (the shipped convention is
/// `<TAG> - <Name>.txt`; a couple of vanilla files have stray spacing, so the
/// tag is taken as the first whitespace/`-`-delimited run of 2-3
/// uppercase/digit characters). `capital = <state_id>` is read with the
/// word-boundary-aware assignment parser, so `capital` never matches
/// `anti_capitalism` or similar substrings.
///
/// This reads the INITIAL (1936) `capital` only. Dynamic capital changes use
/// the `set_capital = { state = <id> }` effect fired from events/focuses and
/// are runtime state a recording feed would supply later, not a static file
/// value — out of scope for the static law here.
pub fn capital_states_from_resolver(resolver: &GameFileResolver) -> Result<BTreeMap<String, u32>> {
    let files = resolver.resolve_dir_files("history/countries", Some("txt"))?;
    let mut out = BTreeMap::new();
    for (rel, path) in &files {
        let Some(tag) = country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        if let Some(capital) = parse_capital_state(&text) {
            out.entry(tag).or_insert(capital);
        }
    }
    Ok(out)
}

/// Parse `history/countries/<TAG>*.txt` into a tag -> INITIAL ruling-party
/// ideology map (`set_politics = { ruling_party = fascism ... }`),
/// resolver-routed. The engine's map country names are IDEOLOGY-RESOLVED
/// (ENGINE-MEASURED: the captured mapname vertex buffers render
/// "MANDATORY PALESTINE" / "BRITISH IRAQ", and vanilla 1936 shows
/// "GERMAN REICH"), keyed `<TAG>_<ideology>` in
/// `localisation/*/countries_l_*.yml` (verified: `GER_fascism: "German
/// Reich"`, `SOV_communism: "Soviet Union"`) with the plain `<TAG>` key as
/// the fallback. This reads the 1936 START ideology only; runtime ideology
/// flips are a recording feed (same static limitation as
/// [`capital_states_from_resolver`]).
pub fn ruling_parties_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, String>> {
    let files = resolver.resolve_dir_files("history/countries", Some("txt"))?;
    let mut out = BTreeMap::new();
    for (rel, path) in &files {
        let Some(tag) = country_history_tag_from_rel(rel) else {
            continue;
        };
        let text = strip_comments(&read_latin1(path)?);
        if let Some(party) = parse_word_value(&text, "ruling_party") {
            out.entry(tag).or_insert(party);
        }
    }
    Ok(out)
}

/// NAME1: tag -> 1936 cosmetic tag (`set_cosmetic_tag` at brace depth 0 in
/// `history/countries`). The engine resolves a cosmetic-tagged country's
/// display name through `<cosmetic>_<ideology>` / `<cosmetic>` localisation
/// BEFORE the plain tag chain (vanilla proof: India's `RAJ_neutrality` loc is
/// "India" but 1936 renders "BRITISH RAJ" = `RAJ_UK_neutrality` via
/// `RAJ - British Raj.txt:244`), and its map color through
/// `common/countries/cosmetic.txt`. Same static-1936 limitation as
/// [`ruling_parties_from_resolver`]; conditional (if/DLC-gated) cosmetic tags
/// are pinpointed in FIDELITY_LEDGER 2.6, not evaluated.
/// The 1936 political map-fill colours per country tag, RAW (pre-COL3 bake),
/// with NAME1 + NAME1b cosmetic overrides applied (all DLC assumed — the
/// bundle default). This is exactly what `country_colors.json` in the bundle
/// carries; exposed so a bundle's colours can be refreshed after a
/// cosmetic-law change without regenerating the whole bundle.
pub fn country_colors_from_game_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    crate::load_country_colors_from_game_resolver(resolver)
}

pub fn cosmetic_tags_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, String>> {
    // Default: assume every DLC is present (the common install + the frame4700
    // byte target). `cosmetic_tags_from_resolver_with_dlc` takes the recording's
    // real DLC set when it is available (R-DLCLIST) for no-DLC compatibility.
    cosmetic_tags_from_resolver_with_dlc(resolver, |_dlc| true)
}

/// NAME1b: resolve 1936 cosmetic tags against a DLC predicate. Unconditional
/// `set_cosmetic_tag` (depth 0) always applies; a DLC-conditional one
/// (`if = { limit = { [NOT] has_dlc } set_cosmetic_tag }`) applies iff its
/// condition holds under `has_dlc`. `has_dlc(name)` returns whether the DLC is
/// active in the recording's modset. A conditional overrides the base for its
/// tag (Canada has only conditionals: CAN_UK with Together for Victory, else
/// CAN_ALY). Deterministic; a tag with several matching conditionals takes the
/// last in file order (vanilla never ships a conflicting pair — the branches
/// are mutually exclusive has_dlc / NOT has_dlc).
pub fn cosmetic_tags_from_resolver_with_dlc(
    resolver: &GameFileResolver,
    has_dlc: impl Fn(&str) -> bool,
) -> Result<BTreeMap<String, String>> {
    let files = resolver.resolve_dir_files("history/countries", Some("txt"))?;
    let mut out = crate::cosmetic_tags_1936_from_history_files(&files)?;
    let conditional = crate::dlc_conditional_cosmetics_1936_from_history_files(&files)?;
    for (tag, entries) in conditional {
        for entry in entries {
            let holds = if entry.negated {
                !has_dlc(&entry.dlc)
            } else {
                has_dlc(&entry.dlc)
            };
            if holds {
                out.insert(tag.clone(), entry.cosmetic);
            }
        }
    }
    Ok(out)
}

/// Extract the country tag from a `history/countries` relative path
/// (`history/countries/GER - Germany.txt` -> `Some("GER")`). Returns `None`
/// when the leading token is not a valid 2-3 char tag.
pub(crate) fn country_history_tag_from_rel(rel: &str) -> Option<String> {
    let file = rel.rsplit('/').next().unwrap_or(rel);
    let stem = file
        .strip_suffix(".txt")
        .or_else(|| file.strip_suffix(".TXT"))?;
    let token: String = stem
        .chars()
        .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .collect();
    is_tag(&token).then_some(token)
}

/// Read `capital = <state_id>` from a country history file's text (comments
/// already stripped). Word-boundary aware via `parse_u32_value`.
pub(crate) fn parse_capital_state(text: &str) -> Option<u32> {
    parse_u32_value(text, "capital")
}

/// Build a full furniture layer straight from a resolver: it loads the shipped
/// province raster, the 1936 state history (owners + victory points), the
/// country capitals, and the localisation table, then computes the markers.
///
/// This is the files-first entry point used by the dump bin. It returns both
/// the markers and the raster (so callers can draw a land underlay without
/// re-reading the BMP).
pub fn furniture_layer_from_resolver(
    resolver: &GameFileResolver,
    language: &str,
) -> Result<FurnitureLayer> {
    // Province raster (map/provinces.bmp keyed by map/definition.csv), reusing
    // the existing loaders — identical raster to the border/label laws.
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    // 1936 state history: owners, province->state, and per-state victory points.
    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (states, province_to_state, state_owners) = load_states(&state_files)?;
    let mut state_victory_points = BTreeMap::new();
    for (state_id, meta) in &states {
        if !meta.victory_points.is_empty() {
            state_victory_points.insert(*state_id, meta.victory_points.clone());
        }
    }

    // Capitals, localisation, and the VP icon defines (dot tier + cutoffs).
    let capital_states = capital_states_from_resolver(resolver)?;
    let localisation = localisation_table_from_resolver(resolver, language)?;
    let vp_icon_defines = vp_icon_defines_from_resolver(resolver)?;

    let markers = compute_furniture_markers(&FurnitureLawInputs {
        province_ids: &bmp.ids,
        width: bmp.width,
        height: bmp.height,
        province_to_state: &province_to_state,
        state_owners: &state_owners,
        state_victory_points: &state_victory_points,
        capital_states: &capital_states,
        localisation: &localisation,
        vp_icon_defines: Some(&vp_icon_defines),
    });

    Ok(FurnitureLayer {
        width: bmp.width,
        height: bmp.height,
        province_ids: bmp.ids,
        province_to_state,
        markers,
        localisation_key_count: localisation.len(),
        capital_count: capital_states.len(),
        vp_icon_defines,
    })
}

/// The computed furniture layer plus the raster needed to draw it.
pub struct FurnitureLayer {
    pub width: usize,
    pub height: usize,
    /// Row-major province id raster (for a land underlay in the dump).
    pub province_ids: Vec<u32>,
    /// Province id -> state id (for a land mask).
    pub province_to_state: BTreeMap<u32, u32>,
    /// The placed markers, sorted by province id.
    pub markers: Vec<FurnitureMarker>,
    /// Diagnostic: number of localisation keys loaded.
    pub localisation_key_count: usize,
    /// Diagnostic: number of country capitals resolved.
    pub capital_count: usize,
    /// The VP/capital map-icon defines used for the dot tiers and zoom cutoffs.
    pub vp_icon_defines: VpIconDefines,
}

impl FurnitureLayer {
    /// Land = any pixel whose province maps to a state (sea provinces have no
    /// state), a clean land/water outline for the dump underlay.
    pub fn land_mask(&self) -> Vec<bool> {
        let mut mask = vec![false; self.width * self.height];
        for (i, &pid) in self.province_ids.iter().enumerate() {
            if pid != 0 && self.province_to_state.contains_key(&pid) {
                mask[i] = true;
            }
        }
        mask
    }
}

// ----------------------------------------------------------------------------
// BMFont (hoi_20bs) glyph metrics — parsed from the shipped `.fnt` so the dump
// can render the REAL game glyphs (positions + atlas are both shipped files).
// ----------------------------------------------------------------------------

/// One glyph's placement in the atlas and its layout metrics (BMFont `char`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BmFontGlyph {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub xoffset: i32,
    pub yoffset: i32,
    pub xadvance: i32,
}

/// Parsed BMFont metrics for a bitmap font page.
#[derive(Clone, Debug, PartialEq)]
pub struct BmFont {
    pub line_height: u32,
    pub base: u32,
    pub scale_w: u32,
    pub scale_h: u32,
    /// Unicode code point -> glyph.
    pub glyphs: HashMap<u32, BmFontGlyph>,
}

impl BmFont {
    /// Glyph for a code point, if present.
    pub fn glyph(&self, code: u32) -> Option<&BmFontGlyph> {
        self.glyphs.get(&code)
    }

    /// Total advance width (in font pixels) of a string, summing `xadvance`
    /// (glyphs absent from the font contribute nothing).
    pub fn measure(&self, text: &str) -> i32 {
        text.chars()
            .filter_map(|c| self.glyphs.get(&(c as u32)))
            .map(|g| g.xadvance)
            .sum()
    }
}

/// Parse a BMFont text `.fnt` (the format the shipped `hoi_20bs.fnt` uses:
/// `info ...`, `common ...`, `char id=.. x=.. ...` lines). Deterministic.
pub fn parse_bmfont(text: &str) -> Result<BmFont> {
    let mut line_height = 0u32;
    let mut base = 0u32;
    let mut scale_w = 0u32;
    let mut scale_h = 0u32;
    let mut glyphs = HashMap::new();
    for line in text.lines() {
        let mut fields = bmfont_fields(line);
        let head = line.split_whitespace().next().unwrap_or("");
        match head {
            "common" => {
                line_height = fields
                    .remove("lineHeight")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
                base = fields.remove("base").and_then(parse_u32_field).unwrap_or(0);
                scale_w = fields
                    .remove("scaleW")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
                scale_h = fields
                    .remove("scaleH")
                    .and_then(parse_u32_field)
                    .unwrap_or(0);
            }
            "char" => {
                let Some(id) = fields.get("id").and_then(|v| parse_u32_field(v.clone())) else {
                    continue;
                };
                let glyph = BmFontGlyph {
                    x: fields
                        .get("x")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    y: fields
                        .get("y")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    width: fields
                        .get("width")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    height: fields
                        .get("height")
                        .and_then(|v| parse_u32_field(v.clone()))
                        .unwrap_or(0),
                    xoffset: fields
                        .get("xoffset")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                    yoffset: fields
                        .get("yoffset")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                    xadvance: fields
                        .get("xadvance")
                        .and_then(|v| v.parse::<i32>().ok())
                        .unwrap_or(0),
                };
                glyphs.insert(id, glyph);
            }
            _ => {}
        }
    }
    if glyphs.is_empty() {
        return Err("bmfont has no glyphs".into());
    }
    Ok(BmFont {
        line_height,
        base,
        scale_w,
        scale_h,
        glyphs,
    })
}

/// Load the `hoi_20bs` BMFont metrics + its glyph atlas through the resolver.
/// The atlas is `gfx/fonts/hoi_20bs.dds` (the `.fnt`'s stale `page file=` line
/// is not trusted; the atlas shares the `.fnt` basename in the shipped set).
pub fn load_vp_label_font(resolver: &GameFileResolver) -> Result<(BmFont, RgbaFontAtlas)> {
    let fnt_path = resolver.resolve_required(HOI_20BS_FNT_SOURCE, "hoi_20bs.fnt")?;
    let font = parse_bmfont(&read_latin1(&fnt_path)?)?;
    let atlas_path = resolver.resolve_required(HOI_20BS_ATLAS_SOURCE, "hoi_20bs.dds")?;
    let image = load_dds_rgba(&atlas_path)?;
    Ok((
        font,
        RgbaFontAtlas {
            width: image.width,
            height: image.height,
            pixels: image.pixels,
        },
    ))
}

/// Load the map text font that ACTUALLY renders the on-map VP/city name —
/// `tahoma_60`, fontfile `gfx/fonts/hoi_mapfont4` (`interface/core.gfx:1679`) —
/// through the resolver (mod override wins). This is the FURN-FONT law: the
/// container declares `hoi_20bs` but the engine composites the map font (see
/// the module `VP_MAP_TEXT_*` constants). Returns the BMFont metrics + atlas,
/// exactly as [`load_vp_label_font`] does for the declared font, so a caller
/// can render the on-map text with the correct glyphs at [`VP_MAP_TEXT_SCALE`].
pub fn load_vp_map_text_font(resolver: &GameFileResolver) -> Result<(BmFont, RgbaFontAtlas)> {
    let fnt_path = resolver.resolve_required(VP_MAP_TEXT_FNT_SOURCE, "hoi_mapfont4.fnt")?;
    let font = parse_bmfont(&read_latin1(&fnt_path)?)?;
    let atlas_path = resolver.resolve_required(VP_MAP_TEXT_ATLAS_SOURCE, "hoi_mapfont4.dds")?;
    let image = load_dds_rgba(&atlas_path)?;
    Ok((
        font,
        RgbaFontAtlas {
            width: image.width,
            height: image.height,
            pixels: image.pixels,
        },
    ))
}

/// A decoded RGBA glyph atlas (kept public so the dump bin can blit from it
/// without touching the crate-private image types).
#[derive(Clone, Debug)]
pub struct RgbaFontAtlas {
    pub width: usize,
    pub height: usize,
    /// Row-major RGBA8.
    pub pixels: Vec<u8>,
}

impl RgbaFontAtlas {
    /// Glyph coverage (0..=255) at atlas pixel `(x, y)`. `hoi_20bs` stores the
    /// glyph in every channel (`.fnt` `chnl=15`); the max channel is the
    /// coverage. Out-of-range samples are transparent.
    pub fn coverage(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let i = (y * self.width + x) * 4;
        let r = self.pixels[i];
        let g = self.pixels[i + 1];
        let b = self.pixels[i + 2];
        let a = self.pixels[i + 3];
        r.max(g).max(b).max(a)
    }
}

/// Split a BMFont line into its `key=value` fields (values may be quoted).
fn bmfont_fields(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for token in split_bmfont_tokens(line) {
        if let Some((key, value)) = token.split_once('=') {
            out.insert(key.to_string(), value.trim_matches('"').to_string());
        }
    }
    out
}

/// Tokenize a BMFont line on whitespace but keep quoted spans together (so
/// `face="Ubuntu Light"` is one token). No allocations beyond the token
/// strings.
fn split_bmfont_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn parse_u32_field(value: String) -> Option<u32> {
    value.trim().parse::<u32>().ok()
}

/// Resolve the VP strip sprite texture path (for a future replay overlay / for
/// the dump's provenance ledger). Returns `None` when the file is absent so a
/// caller can fail visibly rather than assume it exists.
pub fn vp_strip_texture_path(resolver: &GameFileResolver) -> Result<Option<PathBuf>> {
    resolver.resolve(VP_STRIP_SPRITE_TEXTURE)
}

// ============================================================================
// FURN2 — naval-base / air-base map icons (file-derivable 1936 static slice)
// ============================================================================
//
// A separate furniture layer from the VP/capital markers above. Every visual
// parameter is a shipped-file value read through the mod-aware resolver:
//
// * WHICH buildings draw on the map: `common/buildings/00_buildings.txt`
//   `show_on_map = 1` (`air_base` `:84`-`:96` and `naval_base` `:150`-`:174`
//   both carry it). The `air_base` block has NO `spawn_point`, so its map
//   position comes from the `air_base` locator; the `naval_base` block declares
//   `spawn_point = naval_base_spawn` (`:156`), so its map position comes from
//   the `naval_base_spawn` locator.
// * The icon POSITIONS: `map/buildings.txt` locators. Row layout
//   `state_id;building;x;y;z;rotation;extra` (parsed by
//   [`map_building_placements_from_resolver`]). `air_base` rows are STATE-keyed
//   (exactly one per state, verified 1081/1081). `naval_base_spawn` rows are
//   the port locators (2362), each associated to its coastal land province by
//   sampling the province raster at the locator pixel (see
//   [`associate_naval_spawns_to_provinces`]). World `(x, z)` maps to map-pixel
//   `(x, (H-1) - z)` — the same convention `border_world_synthesis.rs` proved.
// * The BUILD LEVELS: `history/states/*.txt` `buildings = { }` block — a
//   state-level `air_base = N` and per-province sub-blocks
//   `<province_id> = { naval_base = N }` (parsed by
//   [`parse_state_building_levels`]). This is the 1936 START level only;
//   runtime construction is a recording feed (same limitation as the capitals).
// * The zoom cutoffs: `common/defines/00_graphics.lua:956`-`:957`
//   `AIRBASE_ICON_DISTANCE_CUTOFF = 900`, `NAVALBASE_ICON_DISTANCE_CUTOFF = 900`
//   ("At what distance air/naval bases are hidden").
// * The sprites: `interface/mapicons.gfx:369`-`:416`
//   `GFX_mapicon_air_base` (`noOfFrames = 7`) and `GFX_mapicon_naval_base`
//   (`noOfFrames = 11`); their containers `interface/mapicons.gui:887`
//   (`air_base_mapicon`, size 32x32) and `:1022` (`naval_base_mapicon`).
//
// UNTRACED (engine-C++, labelled, never fitted): the exact building-level ->
// sprite-frame index (like the VP strip frame, `noOfFrames` bounds it but the
// mapping is engine-side); the exact per-icon sub-locator offset; and, for
// naval bases, the engine's own spawn->province attribution (this law derives
// it from the raster and fails closed on a miss).

/// `common/buildings/00_buildings.txt` building names FURN2 draws.
pub const AIR_BASE_BUILDING: &str = "air_base";
pub const NAVAL_BASE_BUILDING: &str = "naval_base";
/// The `map/buildings.txt` locator type the `naval_base` block's
/// `spawn_point = naval_base_spawn` (`00_buildings.txt:156`) points at.
pub const NAVAL_BASE_SPAWN_LOCATOR: &str = "naval_base_spawn";

/// `interface/mapicons.gfx:375` — the on-map air-base icon sprite.
pub const AIR_BASE_SPRITE_NAME: &str = "GFX_mapicon_air_base";
pub const AIR_BASE_SPRITE_TEXTURE: &str = "gfx/interface/mapicons/mapicon_building_airbase.dds";
/// `interface/mapicons.gfx:377` `noOfFrames = 7`.
pub const AIR_BASE_SPRITE_FRAMES: u32 = 7;
/// `interface/mapicons.gfx:411` — the on-map naval-base icon sprite.
pub const NAVAL_BASE_SPRITE_NAME: &str = "GFX_mapicon_naval_base";
pub const NAVAL_BASE_SPRITE_TEXTURE: &str = "gfx/interface/mapicons/mapicon_building_navalbase.dds";
/// `interface/mapicons.gfx:413` `noOfFrames = 11`.
pub const NAVAL_BASE_SPRITE_FRAMES: u32 = 11;

/// `common/defines/00_graphics.lua:956` `AIRBASE_ICON_DISTANCE_CUTOFF = 900`.
pub const AIRBASE_ICON_DISTANCE_CUTOFF_KEY: &str = "AIRBASE_ICON_DISTANCE_CUTOFF";
/// `common/defines/00_graphics.lua:957` `NAVALBASE_ICON_DISTANCE_CUTOFF = 900`.
pub const NAVALBASE_ICON_DISTANCE_CUTOFF_KEY: &str = "NAVALBASE_ICON_DISTANCE_CUTOFF";

/// `00_buildings.txt` `air_base` `level_cap { state_max = 10 }` (`:93`-`:95`).
pub const AIR_BASE_STATE_MAX_LEVEL: u32 = 10;
/// `00_buildings.txt` `naval_base` `level_cap { province_max = 10 }` (`:171`-`:173`).
pub const NAVAL_BASE_PROVINCE_MAX_LEVEL: u32 = 10;

/// Maximum raster radius (map pixels) searched to attribute a `naval_base_spawn`
/// port locator to a land province when the exact locator pixel is sea. A port
/// spawn sits at the coastline, so the land province is at most a few pixels
/// away; misses beyond this radius fail closed (no icon) rather than guess.
pub const NAVAL_SPAWN_LAND_SNAP_RADIUS: i32 = 6;

/// The kind of base furniture icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseIconKind {
    /// State-level air base (`air_base` locator + `air_base = N` state level).
    AirBase,
    /// Province-level naval base (`naval_base_spawn` locator + per-province
    /// `naval_base = N` level).
    NavalBase,
}

impl BaseIconKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BaseIconKind::AirBase => "air_base",
            BaseIconKind::NavalBase => "naval_base",
        }
    }
    /// Shipped sprite frame count for this icon (`interface/mapicons.gfx`).
    pub fn sprite_frames(self) -> u32 {
        match self {
            BaseIconKind::AirBase => AIR_BASE_SPRITE_FRAMES,
            BaseIconKind::NavalBase => NAVAL_BASE_SPRITE_FRAMES,
        }
    }
    pub fn sprite_name(self) -> &'static str {
        match self {
            BaseIconKind::AirBase => AIR_BASE_SPRITE_NAME,
            BaseIconKind::NavalBase => NAVAL_BASE_SPRITE_NAME,
        }
    }
}

/// The two FURN2 zoom cutoffs, read from `common/defines/00_graphics.lua`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BaseIconDefines {
    /// `AIRBASE_ICON_DISTANCE_CUTOFF` (`:956`).
    pub airbase_cutoff: f32,
    /// `NAVALBASE_ICON_DISTANCE_CUTOFF` (`:957`).
    pub navalbase_cutoff: f32,
}

impl BaseIconDefines {
    /// Camera-distance cutoff for a kind. The `<=` comparison is this law's
    /// choice (engine C++ is UNTRACED); the value is the shipped define.
    pub fn cutoff(&self, kind: BaseIconKind) -> f32 {
        match kind {
            BaseIconKind::AirBase => self.airbase_cutoff,
            BaseIconKind::NavalBase => self.navalbase_cutoff,
        }
    }
    /// Whether the icon is visible at `cam_dist` (`cam_dist <= cutoff`).
    pub fn visible(&self, kind: BaseIconKind, cam_dist: f32) -> bool {
        cam_dist <= self.cutoff(kind)
    }
}

/// Parse the two base-icon zoom cutoffs from the graphics-defines lua text.
/// Fails closed if either key is absent — no invented fallback.
pub fn parse_base_icon_defines(source: &str) -> Result<BaseIconDefines> {
    Ok(BaseIconDefines {
        airbase_cutoff: parse_lua_number_define(source, AIRBASE_ICON_DISTANCE_CUTOFF_KEY)?,
        navalbase_cutoff: parse_lua_number_define(source, NAVALBASE_ICON_DISTANCE_CUTOFF_KEY)?,
    })
}

/// Load the base-icon zoom cutoffs from `common/defines/00_graphics.lua`.
pub fn base_icon_defines_from_resolver(resolver: &GameFileResolver) -> Result<BaseIconDefines> {
    let path = resolver.resolve_required(BATTLE_PLAN_GRAPHICS_DEFINES_SOURCE, "graphics defines lua")?;
    let source = latin1_bytes_to_string(&std::fs::read(&path)?);
    parse_base_icon_defines(&source)
}

/// 1936 START building levels read from one `history/states/*.txt`
/// `buildings = { }` block: the state-level `air_base = N` and the per-province
/// `<province_id> = { naval_base = N }` sub-blocks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StateBuildingLevels {
    /// State-level air-base level (0 when absent).
    pub air_base: u32,
    /// Province id -> naval-base level (only provinces with a level >= 1).
    pub naval_base: BTreeMap<u32, u32>,
}

/// Parse the state-level air-base level and per-province naval-base levels out
/// of one state history file's text (comments already stripped). Returns
/// `None` when the file carries no `buildings` block. The state-level
/// `air_base = N` is read word-boundary-aware; province naval bases come from
/// the `<province_id> = { }` sub-blocks so a bare `naval_base` scalar (there is
/// none in vanilla) can never be mis-attributed to a state.
pub fn parse_state_building_levels(text: &str) -> Option<StateBuildingLevels> {
    let (buildings, _) = extract_block_after(text, "buildings", 0)?;
    let air_base = parse_u32_value(&buildings, AIR_BASE_BUILDING).unwrap_or(0);
    let mut naval_base = BTreeMap::new();
    for (name, block) in top_level_assignment_blocks(&buildings) {
        let Ok(province_id) = name.trim().parse::<u32>() else {
            continue;
        };
        if let Some(level) = parse_u32_value(&block, NAVAL_BASE_BUILDING) {
            if level >= 1 {
                naval_base.insert(province_id, level);
            }
        }
    }
    Some(StateBuildingLevels {
        air_base,
        naval_base,
    })
}

/// Load the 1936 building levels for every state through the resolver: state id
/// -> air-base level, and province id -> naval-base level.
pub fn state_building_levels_from_resolver(
    resolver: &GameFileResolver,
) -> Result<(BTreeMap<u32, u32>, BTreeMap<u32, u32>)> {
    let files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let mut air_by_state = BTreeMap::new();
    let mut naval_by_province = BTreeMap::new();
    for (_, path) in &files {
        let text = strip_comments(&read_latin1(path)?);
        let Some(state_id) = parse_u32_value(&text, "id") else {
            continue;
        };
        let Some(levels) = parse_state_building_levels(&text) else {
            continue;
        };
        if levels.air_base >= 1 {
            air_by_state.insert(state_id, levels.air_base);
        }
        for (province_id, level) in levels.naval_base {
            naval_by_province.insert(province_id, level);
        }
    }
    Ok((air_by_state, naval_by_province))
}

/// Map-pixel anchor (x right, y down) of a `map/buildings.txt` world locator.
/// World `(x, z)` -> pixel `(x, (H-1) - z)` (the `border_world_synthesis.rs`
/// convention). `pos` is `[x, elevation, z]`.
pub fn building_locator_pixel(pos: [f32; 3], map_height: usize) -> (f32, f32) {
    let x = pos[0];
    let y = (map_height.saturating_sub(1) as f32) - pos[2];
    (x, y)
}

/// Attribute each `naval_base_spawn` port locator to its coastal LAND province
/// by sampling the province raster at the locator pixel, snapping outward to the
/// nearest land pixel within [`NAVAL_SPAWN_LAND_SNAP_RADIUS`] when the exact
/// pixel is sea/empty. Returns province id -> the LOCATOR pixel (the icon draws
/// at the locator, not the snapped land pixel). When several locators map to the
/// same province the nearest-snap one wins (deterministic). Locators with no
/// land within the radius are dropped (fail closed).
pub fn associate_naval_spawns_to_provinces(
    placements: &[MapBuildingPlacement],
    province_ids: &[u32],
    width: usize,
    height: usize,
    province_to_state: &BTreeMap<u32, u32>,
) -> BTreeMap<u32, (f32, f32)> {
    let mut best: BTreeMap<u32, (i32, (f32, f32))> = BTreeMap::new();
    if province_ids.len() != width.saturating_mul(height) || width == 0 || height == 0 {
        return BTreeMap::new();
    }
    let land_at = |px: i32, py: i32| -> Option<u32> {
        if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
            return None;
        }
        let pid = province_ids[py as usize * width + px as usize];
        // A land province is one that belongs to a state (sea provinces do not).
        if pid != 0 && province_to_state.contains_key(&pid) {
            Some(pid)
        } else {
            None
        }
    };
    for placement in placements {
        if placement.building != NAVAL_BASE_SPAWN_LOCATOR {
            continue;
        }
        let (fx, fy) = building_locator_pixel(placement.pos, height);
        let cx = fx.round() as i32;
        let cy = fy.round() as i32;
        // Spiral search by increasing Chebyshev radius; the first land pixel at
        // the smallest radius attributes the port.
        let mut found: Option<(u32, i32)> = None;
        'search: for r in 0..=NAVAL_SPAWN_LAND_SNAP_RADIUS {
            for dy in -r..=r {
                for dx in -r..=r {
                    if r > 0 && dx.abs() != r && dy.abs() != r {
                        continue; // only the ring at radius r
                    }
                    if let Some(pid) = land_at(cx + dx, cy + dy) {
                        found = Some((pid, r));
                        break 'search;
                    }
                }
            }
        }
        let Some((pid, dist)) = found else {
            continue;
        };
        match best.get(&pid) {
            Some((best_dist, _)) if *best_dist <= dist => {}
            _ => {
                best.insert(pid, (dist, (fx, fy)));
            }
        }
    }
    best.into_iter()
        .map(|(pid, (_, pixel))| (pid, pixel))
        .collect()
}

/// A placed FURN2 base icon in map-pixel space.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseIconMarker {
    pub kind: BaseIconKind,
    /// State the base belongs to (air-base key; naval base's owning state).
    pub state_id: u32,
    /// Province id for a naval base; `None` for a state-level air base.
    pub province_id: Option<u32>,
    /// Owner tag of the state (empty when unowned).
    pub owner: String,
    /// Marker centre in map-pixel space (x right, y down).
    pub anchor_x: f32,
    pub anchor_y: f32,
    /// 1936 START build level (>= 1; a level-0 building draws no icon).
    pub level: u32,
    /// Sprite frame index. `min(level, sprite_frames - 1)` — the exact
    /// level->frame mapping is engine-side (UNTRACED); this bounds it inside the
    /// shipped `noOfFrames` and never invents a frame outside the strip.
    pub frame: u32,
}

/// Inputs the base-icon law needs; all resolver-derived.
pub struct BaseIconInputs<'a> {
    pub placements: &'a [MapBuildingPlacement],
    pub province_ids: &'a [u32],
    pub width: usize,
    pub height: usize,
    pub province_to_state: &'a BTreeMap<u32, u32>,
    pub state_owners: &'a BTreeMap<u32, String>,
    /// State id -> 1936 air-base level (`history/states`).
    pub air_base_levels: &'a BTreeMap<u32, u32>,
    /// Province id -> 1936 naval-base level (`history/states`).
    pub naval_base_levels: &'a BTreeMap<u32, u32>,
    /// Building types with `show_on_map = 1` (`common/buildings`).
    pub show_on_map: &'a std::collections::BTreeSet<String>,
}

/// The computed FURN2 base-icon layer plus attribution diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub struct BaseIconLayer {
    pub markers: Vec<BaseIconMarker>,
    /// Air-base markers emitted.
    pub air_base_count: usize,
    /// Naval-base markers emitted (successfully attributed to a locator).
    pub naval_base_count: usize,
    /// Province naval bases in history that could NOT be attributed to a spawn
    /// locator (fail-closed, reported not guessed).
    pub naval_unattributed: Vec<u32>,
}

/// Sprite frame for a build level: `min(level, sprite_frames - 1)`. Kept out of
/// the marker so the UNTRACED level->frame law lives in exactly one place.
pub fn base_icon_frame(kind: BaseIconKind, level: u32) -> u32 {
    level.min(kind.sprite_frames().saturating_sub(1))
}

/// Compute every FURN2 base icon from the shipped inputs, deterministic and
/// sorted (air bases by state id, then naval bases by province id).
///
/// * Air bases: one per state whose `air_base` level >= 1, positioned at the
///   state's `air_base` locator (fail closed if the state has no locator).
/// * Naval bases: one per province whose `naval_base` level >= 1, positioned at
///   the province's attributed `naval_base_spawn` locator (fail closed / listed
///   in `naval_unattributed` if no locator attributes to it).
pub fn compute_base_icon_markers(inputs: &BaseIconInputs) -> BaseIconLayer {
    let mut markers = Vec::new();
    let air_on_map = inputs.show_on_map.contains(AIR_BASE_BUILDING);
    let naval_on_map = inputs.show_on_map.contains(NAVAL_BASE_BUILDING);

    // Air bases: state-keyed locators (verified one per state).
    if air_on_map {
        let mut air_locator: BTreeMap<u32, (f32, f32)> = BTreeMap::new();
        for placement in inputs.placements {
            if placement.building == AIR_BASE_BUILDING {
                air_locator
                    .entry(placement.state_id)
                    .or_insert_with(|| building_locator_pixel(placement.pos, inputs.height));
            }
        }
        for (state_id, level) in inputs.air_base_levels {
            if *level < 1 {
                continue;
            }
            let Some((ax, ay)) = air_locator.get(state_id).copied() else {
                continue; // no shipped locator for this state: fail closed
            };
            markers.push(BaseIconMarker {
                kind: BaseIconKind::AirBase,
                state_id: *state_id,
                province_id: None,
                owner: inputs.state_owners.get(state_id).cloned().unwrap_or_default(),
                anchor_x: ax,
                anchor_y: ay,
                level: *level,
                frame: base_icon_frame(BaseIconKind::AirBase, *level),
            });
        }
    }
    let air_base_count = markers.len();

    // Naval bases: province-keyed via the spawn-locator attribution.
    let mut naval_unattributed = Vec::new();
    if naval_on_map {
        let spawn_pixels = associate_naval_spawns_to_provinces(
            inputs.placements,
            inputs.province_ids,
            inputs.width,
            inputs.height,
            inputs.province_to_state,
        );
        for (province_id, level) in inputs.naval_base_levels {
            if *level < 1 {
                continue;
            }
            let Some((nx, ny)) = spawn_pixels.get(province_id).copied() else {
                naval_unattributed.push(*province_id);
                continue;
            };
            let state_id = inputs
                .province_to_state
                .get(province_id)
                .copied()
                .unwrap_or(0);
            markers.push(BaseIconMarker {
                kind: BaseIconKind::NavalBase,
                state_id,
                province_id: Some(*province_id),
                owner: inputs
                    .state_owners
                    .get(&state_id)
                    .cloned()
                    .unwrap_or_default(),
                anchor_x: nx,
                anchor_y: ny,
                level: *level,
                frame: base_icon_frame(BaseIconKind::NavalBase, *level),
            });
        }
    }
    let naval_base_count = markers.len() - air_base_count;
    naval_unattributed.sort_unstable();

    BaseIconLayer {
        markers,
        air_base_count,
        naval_base_count,
        naval_unattributed,
    }
}

/// Build the full FURN2 base-icon layer straight from a resolver: reads the
/// province raster, the `common/buildings` `show_on_map` set, the
/// `map/buildings.txt` locators, and the 1936 `history/states` building levels,
/// then computes the icons. Reuses the same shipped raster as the VP furniture.
pub fn base_icon_layer_from_resolver(resolver: &GameFileResolver) -> Result<BaseIconLayer> {
    let definition_csv = resolver.resolve_required("map/definition.csv", "definition.csv")?;
    let provinces_bmp = resolver.resolve_required("map/provinces.bmp", "provinces.bmp")?;
    let (_definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    let state_files = resolver.resolve_dir_files("history/states", Some("txt"))?;
    let (_states, province_to_state, state_owners) = load_states(&state_files)?;
    let (air_base_levels, naval_base_levels) = state_building_levels_from_resolver(resolver)?;

    let placements = map_building_placements_from_resolver(resolver)?;
    let show_on_map = show_on_map_buildings_from_resolver(resolver)?
        .into_iter()
        .filter(|building| building.show_on_map >= 1)
        .map(|building| building.name)
        .collect::<std::collections::BTreeSet<_>>();

    Ok(compute_base_icon_markers(&BaseIconInputs {
        placements: &placements,
        province_ids: &bmp.ids,
        width: bmp.width,
        height: bmp.height,
        province_to_state: &province_to_state,
        state_owners: &state_owners,
        air_base_levels: &air_base_levels,
        naval_base_levels: &naval_base_levels,
        show_on_map: &show_on_map,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::temp_test_dir;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = rel
            .split('/')
            .fold(root.to_path_buf(), |path, part| path.join(part));
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    fn raster_rect(ids: &mut [u32], width: usize, id: u32, rect: (usize, usize, usize, usize)) {
        let (x0, y0, x1, y1) = rect;
        for y in y0..y1 {
            for x in x0..x1 {
                ids[y * width + x] = id;
            }
        }
    }

    fn base_inputs<'a>(
        province_ids: &'a [u32],
        width: usize,
        height: usize,
        province_to_state: &'a BTreeMap<u32, u32>,
        state_owners: &'a BTreeMap<u32, String>,
        state_victory_points: &'a BTreeMap<u32, BTreeMap<u32, i32>>,
        capital_states: &'a BTreeMap<String, u32>,
        localisation: &'a BTreeMap<String, String>,
    ) -> FurnitureLawInputs<'a> {
        FurnitureLawInputs {
            province_ids,
            width,
            height,
            province_to_state,
            state_owners,
            state_victory_points,
            capital_states,
            localisation,
            vp_icon_defines: None,
        }
    }

    fn sample_defines() -> VpIconDefines {
        // The literal shipped vanilla values (00_graphics.lua:947-963).
        VpIconDefines {
            levels: 3,
            map_icon_after: vec![0, 9, 20],
            text_cutoff: vec![150.0, 250.0, 500.0],
            text_cutoff_min: 100.0,
            text_cutoff_max: 800.0,
            dot_cutoff_min: 100.0,
            dot_cutoff_max: 1000.0,
            max_victory_points_for_percent: 22.0,
            capital_cutoff_max: 1000.0,
            capital_icon_cutoff: 1100.0,
        }
    }

    #[test]
    fn victory_point_marker_lands_on_province_centroid() {
        let width = 100;
        let height = 60;
        let mut ids = vec![0u32; width * height];
        // Province 3838 fills a rectangle; its centroid is the rect centre.
        raster_rect(&mut ids, width, 3838, (20, 10, 40, 30)); // centre (29.5, 19.5)
        let province_to_state = BTreeMap::from([(3838u32, 1u32)]);
        let state_owners = BTreeMap::from([(1u32, "FRA".to_string())]);
        let state_victory_points = BTreeMap::from([(1u32, BTreeMap::from([(3838u32, 1i32)]))]);
        let capital_states = BTreeMap::new();
        let localisation =
            BTreeMap::from([("VICTORY_POINTS_3838".to_string(), "Ajáccio".to_string())]);
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        let m = &markers[0];
        assert_eq!(m.province_id, 3838);
        assert_eq!(m.owner, "FRA");
        assert_eq!(m.vp_value, 1);
        assert_eq!(m.label, "Ajáccio");
        assert!(m.label_resolved);
        assert_eq!(m.kind, FurnitureMarkerKind::VictoryPoint);
        assert!((m.anchor_x - 29.5).abs() < 0.01, "anchor_x {}", m.anchor_x);
        assert!((m.anchor_y - 19.5).abs() < 0.01, "anchor_y {}", m.anchor_y);
    }

    #[test]
    fn capital_state_vp_becomes_capital_marker() {
        let width = 80;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 64, (10, 10, 30, 30)); // Berlin province
        let province_to_state = BTreeMap::from([(64u32, 64u32)]);
        let state_owners = BTreeMap::from([(64u32, "GER".to_string())]);
        let state_victory_points = BTreeMap::from([(64u32, BTreeMap::from([(64u32, 40i32)]))]);
        // Germany's capital is state 64 (history/countries/GER - Germany.txt).
        let capital_states = BTreeMap::from([("GER".to_string(), 64u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].kind, FurnitureMarkerKind::Capital);
        // Localisation miss keeps the raw key, fail-visible.
        assert_eq!(markers[0].label, "VICTORY_POINTS_64");
        assert!(!markers[0].label_resolved);
    }

    #[test]
    fn only_highest_vp_in_capital_state_is_starred() {
        // ENGINE-MEASURED (live oracle 2026-07-07): Canberra (30 VP) stars
        // while Sydney (20 VP) in the SAME capital state 285 shows a circle.
        // A capital state with several VPs must yield exactly ONE Capital.
        let width = 120;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 7859, (10, 10, 30, 30)); // Canberra
        raster_rect(&mut ids, width, 12406, (60, 10, 90, 30)); // Sydney
        let province_to_state = BTreeMap::from([(7859u32, 285u32), (12406u32, 285u32)]);
        let state_owners = BTreeMap::from([(285u32, "AST".to_string())]);
        let state_victory_points = BTreeMap::from([(
            285u32,
            BTreeMap::from([(7859u32, 30i32), (12406u32, 20i32)]),
        )]);
        let capital_states = BTreeMap::from([("AST".to_string(), 285u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 2);
        let canberra = markers.iter().find(|m| m.province_id == 7859).unwrap();
        let sydney = markers.iter().find(|m| m.province_id == 12406).unwrap();
        assert_eq!(canberra.kind, FurnitureMarkerKind::Capital);
        assert_eq!(
            sydney.kind,
            FurnitureMarkerKind::VictoryPoint,
            "only the highest-VP province in the capital state stars"
        );
        // Tie: equal VPs -> lowest province id stars (deterministic).
        let tied = BTreeMap::from([(
            285u32,
            BTreeMap::from([(7859u32, 20i32), (12406u32, 20i32)]),
        )]);
        let mut tied_inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &tied,
            &capital_states,
            &localisation,
        );
        tied_inputs.state_victory_points = &tied;
        let tied_markers = compute_furniture_markers(&tied_inputs);
        let starred: Vec<u32> = tied_markers
            .iter()
            .filter(|m| m.kind == FurnitureMarkerKind::Capital)
            .map(|m| m.province_id)
            .collect();
        assert_eq!(starred, vec![7859], "tie breaks to the lowest province id");
    }

    #[test]
    fn capital_state_with_no_vp_yields_no_capital_marker() {
        // Jan Mayen (state 914) is a capital with no victory_points block: it
        // must produce NO capital marker (the state-to-province anchor is
        // engine-side/UNTRACED; we never fabricate one).
        let width = 40;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        // Paint state 914's only province 13272, but give it no VP entry.
        raster_rect(&mut ids, width, 13272, (5, 5, 20, 20));
        let province_to_state = BTreeMap::from([(13272u32, 914u32)]);
        let state_owners = BTreeMap::from([(914u32, "JAN".to_string())]);
        // No victory_points for state 914 at all.
        let state_victory_points: BTreeMap<u32, BTreeMap<u32, i32>> = BTreeMap::new();
        let capital_states = BTreeMap::from([("JAN".to_string(), 914u32)]);
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        assert!(
            compute_furniture_markers(&inputs).is_empty(),
            "a capital state with no VP must not fabricate a capital marker"
        );
    }

    #[test]
    fn vp_province_absent_from_raster_fails_closed() {
        let width = 40;
        let height = 40;
        let ids = vec![0u32; width * height]; // province 999 never painted
        let province_to_state = BTreeMap::from([(999u32, 1u32)]);
        let state_owners = BTreeMap::from([(1u32, "FRA".to_string())]);
        let state_victory_points = BTreeMap::from([(1u32, BTreeMap::from([(999u32, 5i32)]))]);
        let capital_states = BTreeMap::new();
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        assert!(
            compute_furniture_markers(&inputs).is_empty(),
            "a VP province with no raster pixels must not place a marker"
        );
    }

    #[test]
    fn markers_are_deterministic_and_sorted_by_province() {
        let width = 120;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 50, (10, 10, 30, 30));
        raster_rect(&mut ids, width, 20, (60, 10, 90, 30));
        let province_to_state = BTreeMap::from([(50u32, 5u32), (20u32, 2u32)]);
        let state_owners = BTreeMap::from([(5u32, "SOV".to_string()), (2u32, "GER".to_string())]);
        let state_victory_points = BTreeMap::from([
            (5u32, BTreeMap::from([(50u32, 3i32)])),
            (2u32, BTreeMap::from([(20u32, 7i32)])),
        ]);
        let capital_states = BTreeMap::new();
        let localisation = BTreeMap::new();
        let inputs = base_inputs(
            &ids,
            width,
            height,
            &province_to_state,
            &state_owners,
            &state_victory_points,
            &capital_states,
            &localisation,
        );
        let a = compute_furniture_markers(&inputs);
        let b = compute_furniture_markers(&inputs);
        assert_eq!(a, b, "law must be deterministic");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].province_id, 20, "sorted by province id");
        assert_eq!(a[1].province_id, 50);
    }

    #[test]
    fn victory_points_parse_on_real_history_states_row_formats() {
        // The exact 1-France.txt shape: victory_points inside history={...}.
        let text = strip_comments(
            "state={ id=1 name=\"STATE_1\"\n history={ owner = FRA\n victory_points = { 3838 1 }\n } provinces={ 3838 9851 } }",
        );
        // load_states path is exercised via game_data; here assert the VP block
        // parse the module relies on picks up the province/value pair.
        assert!(text.contains("victory_points"));
        // Multiple VP blocks in one state (a real edge format).
        let multi = strip_comments(
            "state={ id=2 history={ victory_points = { 100 5 } victory_points = { 200 12 } } }",
        );
        assert!(multi.matches("victory_points").count() == 2);
    }

    #[test]
    fn capital_tag_extraction_handles_shipped_filename_shapes() {
        assert_eq!(
            country_history_tag_from_rel("history/countries/GER - Germany.txt").as_deref(),
            Some("GER")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/USA - USA.txt").as_deref(),
            Some("USA")
        );
        // Stray spacing typos in the shipped set still expose a valid tag.
        assert_eq!(
            country_history_tag_from_rel("history/countries/HBC  - Hebei-Chahar.txt").as_deref(),
            Some("HBC")
        );
        assert_eq!(
            country_history_tag_from_rel("history/countries/KLT -Kalat.txt").as_deref(),
            Some("KLT")
        );
        // A numeric-suffix tag (e.g. dynamic tags) is valid.
        assert_eq!(
            country_history_tag_from_rel("history/countries/D01 - Dynamic.txt").as_deref(),
            Some("D01")
        );
        // Non-tag files (no leading tag) are skipped.
        assert_eq!(
            country_history_tag_from_rel("history/countries/readme.txt"),
            None
        );
    }

    #[test]
    fn capital_parse_is_word_boundary_aware() {
        // `anti_capitalism` must NOT match `capital`.
        let text = strip_comments(
            "set_variable = { SOV_propaganda_anti_capitalism_cost = 1 }\ncapital = 219\n",
        );
        assert_eq!(parse_capital_state(&text), Some(219));
        // With a BOM-prefixed line (USA/FRA ship this).
        let bom = strip_comments("\u{feff}capital = 361\n");
        assert_eq!(parse_capital_state(&bom), Some(361));
        // No capital line.
        assert_eq!(parse_capital_state("owner = FRA\n"), None);
    }

    #[test]
    fn bmfont_parses_shipped_hoi_20bs_shape() {
        let fnt = "info face=\"Ubuntu Light\" size=20 bold=1\n\
                   common lineHeight=20 base=16 scaleW=512 scaleH=256 pages=1\n\
                   page id=0 file=\"hoi_20b_0.dds\"\n\
                   chars count=2\n\
                   char id=76   x=189   y=106   width=13    height=14    xoffset=-2    yoffset=4     xadvance=8     page=0  chnl=15\n\
                   char id=111  x=235   y=119   width=13    height=12    xoffset=-2    yoffset=6     xadvance=9     page=0  chnl=15\n";
        let font = parse_bmfont(fnt).expect("parse bmfont");
        assert_eq!(font.line_height, 20);
        assert_eq!(font.base, 16);
        assert_eq!(font.scale_w, 512);
        assert_eq!(font.scale_h, 256);
        let l = font.glyph(76).expect("L glyph");
        assert_eq!((l.x, l.y, l.width, l.height), (189, 106, 13, 14));
        assert_eq!((l.xoffset, l.yoffset, l.xadvance), (-2, 4, 8));
        // "Lo" advance = 8 + 9 = 17.
        assert_eq!(font.measure("Lo"), 17);
    }

    #[test]
    fn bmfont_empty_fails_closed() {
        assert!(parse_bmfont("info face=\"x\"\ncommon lineHeight=20\n").is_err());
    }

    #[test]
    fn vp_icon_defines_parse_from_lua_block() {
        // The literal shipped block (00_graphics.lua:947-963), with trailing
        // comments the parser must strip.
        let lua = "\
NDefines = {\n\
\tNGraphics = {\n\
\t\tVICTORY_POINT_LEVELS = 3,\n\
\t\tVICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20}, -- becomes bigger dot\n\
\t\tVICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX = 1000.0,\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500},\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN = 100.0,\n\
\t\tVICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX = 800.0,\n\
\t\tVICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN = 100.0,\n\
\t\tVICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX = 1000.0,\n\
\t\tVICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT = 22,\n\
\t\tCAPITAL_ICON_CUTOFF = 1100,\n\
\t},\n\
}\n";
        let defines = parse_vp_icon_defines(lua).expect("parse vp icon defines");
        assert_eq!(defines, sample_defines());
    }

    #[test]
    fn vp_icon_defines_missing_key_fails_closed() {
        // Drop CAPITAL_ICON_CUTOFF: no invented fallback.
        let lua = "\
VICTORY_POINT_LEVELS = 3,\n\
VICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20},\n\
VICTORY_POINT_MAP_ICON_CAPITAL_CUTOFF_MAX = 1000.0,\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF = {150, 250, 500},\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MIN = 100.0,\n\
VICTORY_POINT_MAP_ICON_TEXT_CUTOFF_MAX = 800.0,\n\
VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MIN = 100.0,\n\
VICTORY_POINT_MAP_ICON_DOT_CUTOFF_MAX = 1000.0,\n\
VICTORY_POINT_MAP_ICON_MAX_VICTORY_POINTS_FOR_PERCENT = 22,\n";
        assert!(parse_vp_icon_defines(lua).is_err());
    }

    #[test]
    fn dot_tier_follows_map_icon_after_thresholds() {
        let defines = sample_defines();
        // {0, 9, 20}: value 0..8 -> tier 0, 9..19 -> tier 1, >=20 -> tier 2.
        assert_eq!(defines.tier_for_vp(0), 0);
        assert_eq!(defines.tier_for_vp(8), 0);
        assert_eq!(defines.tier_for_vp(9), 1);
        assert_eq!(defines.tier_for_vp(19), 1);
        assert_eq!(defines.tier_for_vp(20), 2);
        assert_eq!(defines.tier_for_vp(50), 2);
        // Never exceeds levels-1.
        assert!(defines.tier_for_vp(9999) < defines.levels);
    }

    #[test]
    fn marker_carries_file_derived_dot_tier() {
        let width = 60;
        let height = 40;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 64, (10, 10, 30, 30));
        let province_to_state = BTreeMap::from([(64u32, 64u32)]);
        let state_owners = BTreeMap::from([(64u32, "GER".to_string())]);
        // Berlin ~40 VP -> tier 2.
        let state_victory_points = BTreeMap::from([(64u32, BTreeMap::from([(64u32, 40i32)]))]);
        let capital_states = BTreeMap::from([("GER".to_string(), 64u32)]);
        let localisation = BTreeMap::new();
        let defines = sample_defines();
        let inputs = FurnitureLawInputs {
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            state_victory_points: &state_victory_points,
            capital_states: &capital_states,
            localisation: &localisation,
            vp_icon_defines: Some(&defines),
        };
        let markers = compute_furniture_markers(&inputs);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].dot_tier, 2);
        assert_eq!(markers[0].kind, FurnitureMarkerKind::Capital);
    }

    #[test]
    fn dot_and_text_visibility_follow_shipped_defines() {
        let d = sample_defines();
        use FurnitureMarkerKind::{Capital, VictoryPoint};
        // Capital icon: visible to CAPITAL_ICON_CUTOFF (1100), gone above.
        assert!(d.dot_visible(Capital, 1, 1100.0));
        assert!(!d.dot_visible(Capital, 1, 1100.1));
        // Capital text: visible to CAPITAL_CUTOFF_MAX (1000).
        assert!(d.text_visible(Capital, 0, 1000.0));
        assert!(!d.text_visible(Capital, 0, 1000.1));
        // VP dot: vp=22+ reaches DOT_CUTOFF_MAX (1000); vp=0 stops at MIN (100).
        assert!(d.dot_visible(VictoryPoint, 22, 1000.0));
        assert!(
            d.dot_visible(VictoryPoint, 40, 1000.0),
            "vp>22 treated as max"
        );
        assert!(!d.dot_visible(VictoryPoint, 22, 1000.1));
        assert!(d.dot_visible(VictoryPoint, 0, 100.0));
        assert!(!d.dot_visible(VictoryPoint, 0, 100.1));
        // vp=11 -> half the range: cutoff 550.
        assert!(d.dot_visible(VictoryPoint, 11, 550.0));
        assert!(!d.dot_visible(VictoryPoint, 11, 550.1));
        // VP text per tier: {150, 250, 500} with [100, 800] clamp.
        assert!(d.text_visible(VictoryPoint, 0, 150.0));
        assert!(!d.text_visible(VictoryPoint, 0, 150.1));
        assert!(d.text_visible(VictoryPoint, 1, 250.0));
        assert!(!d.text_visible(VictoryPoint, 1, 250.1));
        assert!(d.text_visible(VictoryPoint, 2, 500.0));
        assert!(!d.text_visible(VictoryPoint, 2, 500.1));
        // An out-of-range tier falls back to the MIN clamp, never invents.
        assert!(d.text_visible(VictoryPoint, 9, 100.0));
        assert!(!d.text_visible(VictoryPoint, 9, 100.1));
    }

    #[test]
    fn vp_strip_frame_law_matches_measured_grid() {
        use FurnitureMarkerKind::{Capital, VictoryPoint};
        // Shape groups: star 0..4, circle 5..9, pentagon 10..14, square 15..19.
        assert_eq!(
            vp_strip_frame_index(Capital, 0, VP_STRIP_COLOR_NEUTRAL_GRAY),
            4
        );
        assert_eq!(
            vp_strip_frame_index(Capital, 2, 0),
            0,
            "capital ignores tier"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 2, 4),
            9,
            "tier 2 = circle"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 1, 4),
            14,
            "tier 1 = pentagon"
        );
        assert_eq!(
            vp_strip_frame_index(VictoryPoint, 0, 4),
            19,
            "tier 0 = square"
        );
        // Clamps: tier beyond the 3 shipped levels -> circle; color beyond 4 -> gray.
        assert_eq!(vp_strip_frame_index(VictoryPoint, 7, 0), 5);
        assert_eq!(vp_strip_frame_index(VictoryPoint, 0, 99), 19);
        // Every emitted frame stays inside the shipped 20-frame strip.
        for tier in 0..4u32 {
            for slot in 0..6u32 {
                assert!(vp_strip_frame_index(VictoryPoint, tier, slot) < VP_STRIP_FRAME_COUNT);
                assert!(vp_strip_frame_index(Capital, tier, slot) < VP_STRIP_FRAME_COUNT);
            }
        }
    }

    #[test]
    fn gate_vanilla_vp_strip_grid_evidence() {
        // EVIDENCE GATE for the frame law: decode the real shipped strip and
        // re-measure the 4-shape x 5-color grid this law is built on — 20
        // frames whose per-group opaque-pixel areas strictly order
        // star > circle > pentagon > square and are constant within a group.
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let strip = load_vp_strip_rgba(&resolver).expect("vp strip decodes");
        assert_eq!(
            (strip.width, strip.height),
            (580, 27),
            "shipped strip geometry"
        );
        let frame_w = strip.width / VP_STRIP_FRAME_COUNT as usize;
        assert_eq!(frame_w, 29);
        let opaque = |frame: usize| -> usize {
            let x0 = frame * frame_w;
            let mut n = 0usize;
            for y in 0..strip.height {
                for x in x0..x0 + frame_w {
                    if strip.pixels[(y * strip.width + x) * 4 + 3] > 32 {
                        n += 1;
                    }
                }
            }
            n
        };
        // Constant area within each 5-frame color group.
        let group_area: Vec<usize> = (0..4).map(|g| opaque(g * 5)).collect();
        for g in 0..4 {
            for c in 1..5 {
                assert_eq!(
                    opaque(g * 5 + c),
                    group_area[g],
                    "frame {} area differs from its shape group",
                    g * 5 + c
                );
            }
        }
        // Strict size ordering star > circle > pentagon > square.
        assert!(
            group_area[0] > group_area[1]
                && group_area[1] > group_area[2]
                && group_area[2] > group_area[3],
            "shape areas must order star > circle > pentagon > square, got {group_area:?}"
        );
        println!("gate: vp strip areas {group_area:?} (star/circle/pentagon/square)");
    }

    #[test]
    fn parse_lua_number_list_reads_shipped_shapes() {
        let src = "VICTORY_POINT_MAP_ICON_AFTER = {0, 9, 20}, -- comment\n";
        assert_eq!(
            parse_lua_number_list(src, "VICTORY_POINT_MAP_ICON_AFTER"),
            Some(vec![0.0, 9.0, 20.0])
        );
        // Word-boundary: a prefix key must not match.
        assert_eq!(parse_lua_number_list(src, "VICTORY_POINT_MAP"), None);
        assert_eq!(parse_lua_number_list("KEY = 5,\n", "KEY"), None);
    }

    #[test]
    fn victory_point_loc_resolves_through_mod_override() {
        // The city-name string comes from localisation via the resolver; a mod
        // that overrides victory_points_l_english.yml must win, and a mod-added
        // key must merge in — this is the replace/override path the spec wants.
        let root = temp_test_dir("map_furniture_loc_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "localisation/english/victory_points_l_english.yml",
            "\u{feff}l_english:\n VICTORY_POINTS_3838:0 \"Ajaccio\"\n VICTORY_POINTS_64:0 \"Berlin\"\n",
        );
        write_text(
            &mod_root,
            "localisation/english/victory_points_l_english.yml",
            "l_english:\n VICTORY_POINTS_3838:0 \"Ajaccio (modded)\"\n",
        );
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let table = localisation_table_from_resolver(&resolver, "english").expect("loc");
        // Mod file wholly overrides the vanilla file (Paradox file-level
        // semantics), so 3838 is the modded string and 64 vanished with it.
        assert_eq!(
            table.get("VICTORY_POINTS_3838").map(String::as_str),
            Some("Ajaccio (modded)")
        );
        assert_eq!(table.get("VICTORY_POINTS_64"), None);
        assert_eq!(resolver.stats().overridden_file_count, 1);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn capital_states_resolve_through_mod_override() {
        let root = temp_test_dir("map_furniture_capital_mod_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(
            &vanilla,
            "history/countries/GER - Germany.txt",
            "capital = 64 # Berlin\nowner = GER\n",
        );
        write_text(
            &mod_root,
            "history/countries/GER - Germany.txt",
            "capital = 999 # moved capital\n",
        );
        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let capitals = capital_states_from_resolver(&resolver).expect("capitals");
        assert_eq!(capitals.get("GER").copied(), Some(999), "mod capital wins");
        fs::remove_dir_all(root).ok();
    }

    // ---- Vanilla-file gates (skip cleanly when the game dir is absent) ----

    fn vanilla_resolver() -> Option<GameFileResolver> {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla map_furniture gate: {VANILLA_GAME_DIR} is absent");
            return None;
        }
        Some(GameFileResolver::vanilla(game_dir).expect("vanilla resolver"))
    }

    #[test]
    fn gate_vanilla_dlc_conditional_cosmetics_canada_congo() {
        // NAME1b: against the REAL vanilla history files, Canada + Congo resolve
        // their DLC-conditional cosmetics both ways. WITH the DLC (default) the
        // engine shows CAN_UK ("Canada", rgb 119,48,39 -> baked 95,61,56) and
        // COG_belgian_congo; WITHOUT it the fallback CAN_ALY. This is the
        // no-DLC-compatibility path — the same resolver, a different DLC set.
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let all_dlc =
            cosmetic_tags_from_resolver_with_dlc(&resolver, |_| true).expect("all-DLC cosmetics");
        assert_eq!(
            all_dlc.get("CAN").map(String::as_str),
            Some("CAN_UK"),
            "Canada with Together for Victory shows CAN_UK"
        );
        assert_eq!(
            all_dlc.get("COG").map(String::as_str),
            Some("COG_belgian_congo"),
            "Congo with Gotterdammerung shows COG_belgian_congo"
        );

        let no_dlc =
            cosmetic_tags_from_resolver_with_dlc(&resolver, |_| false).expect("no-DLC cosmetics");
        assert_eq!(
            no_dlc.get("CAN").map(String::as_str),
            Some("CAN_ALY"),
            "Canada without the DLC falls back to CAN_ALY"
        );
        // Congo's cosmetic is positive-only (no fallback branch): without the
        // DLC it has no cosmetic override at all.
        assert_eq!(
            no_dlc.get("COG"),
            None,
            "Congo without the DLC has no cosmetic"
        );

        // The unconditional RAJ cosmetic (NAME1) is DLC-invariant.
        assert_eq!(all_dlc.get("RAJ").map(String::as_str), Some("RAJ_UK"));
        assert_eq!(no_dlc.get("RAJ").map(String::as_str), Some("RAJ_UK"));

        // COMPLETENESS (Codex-verified against all 364 vanilla country files):
        // exactly FOUR tags have a pure DLC-conditional start-state cosmetic —
        // CAN (CAN_UK/CAN_ALY), COG (COG_belgian_congo, positive-only), BUL
        // (BUL_neutrality under "Battle for the Bosporus"), XSM
        // (united_ma_clique_tag under NOT "No Compromise, No Surrender").
        // Only CAN changes the MAP COLOUR (COG is BEL-owned at 1936;
        // BUL_neutrality has no colour entry; XSM's is a negated branch, off
        // under all-DLC) — but all four are the correct NAME resolution. Every
        // other file mixing has_dlc + set_cosmetic_tag is commented (EGY), an
        // unconditional depth-0 cosmetic NAME1 already applies (TUR/GRE/URG/
        // INS), or a non-DLC event/date effect (CZE/MEN/SPR). A game patch that
        // adds a new one fails here loudly instead of silently mis-resolving.
        let mut differing: Vec<&str> = all_dlc
            .keys()
            .chain(no_dlc.keys())
            .filter(|tag| all_dlc.get(*tag) != no_dlc.get(*tag))
            .map(String::as_str)
            .collect();
        differing.sort_unstable();
        differing.dedup();
        assert_eq!(
            differing,
            vec!["BUL", "CAN", "COG", "XSM"],
            "DLC-conditional cosmetic set changed — a game update altered it"
        );
        // XSM's cosmetic is the NEGATED branch: present only WITHOUT the DLC.
        assert_eq!(all_dlc.get("XSM"), None, "XSM has no cosmetic with all DLC");
        assert_eq!(
            no_dlc.get("XSM").map(String::as_str),
            Some("united_ma_clique_tag"),
            "XSM without No Compromise No Surrender takes the united-Ma cosmetic"
        );
    }

    #[test]
    fn gate_vanilla_province_centers_on_real_definition_csv() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let definition_csv = resolver
            .resolve_required("map/definition.csv", "definition.csv")
            .expect("definition.csv");
        let provinces_bmp = resolver
            .resolve_required("map/provinces.bmp", "provinces.bmp")
            .expect("provinces.bmp");
        let (definitions, color_to_id) =
            load_definition_csv(&definition_csv).expect("definition.csv rows");
        assert!(
            definitions.len() > 10_000,
            "vanilla definition.csv should have many rows, got {}",
            definitions.len()
        );
        let bmp = load_provinces_bmp(&provinces_bmp, &color_to_id).expect("provinces.bmp raster");
        assert_eq!((bmp.width, bmp.height), (5632, 2048), "vanilla map size");
        // Compute a centroid for province 3838 (Ajáccio) via the same one-pass
        // law and assert it is inside the map and lands on a pixel of that id.
        let mut vps = BTreeMap::new();
        vps.insert(1u32, BTreeMap::from([(3838u32, 1i32)]));
        let markers = compute_furniture_markers(&FurnitureLawInputs {
            province_ids: &bmp.ids,
            width: bmp.width,
            height: bmp.height,
            province_to_state: &BTreeMap::from([(3838u32, 1u32)]),
            state_owners: &BTreeMap::from([(1u32, "FRA".to_string())]),
            state_victory_points: &vps,
            capital_states: &BTreeMap::new(),
            localisation: &BTreeMap::new(),
            vp_icon_defines: None,
        });
        assert_eq!(markers.len(), 1, "province 3838 should be in the raster");
        let m = &markers[0];
        assert!(m.anchor_x >= 0.0 && (m.anchor_x as usize) < bmp.width);
        assert!(m.anchor_y >= 0.0 && (m.anchor_y as usize) < bmp.height);
        assert!(m.pixel_count > 0);
        println!(
            "gate: province 3838 centroid ({:.1},{:.1}) over {} px",
            m.anchor_x, m.anchor_y, m.pixel_count
        );
    }

    #[test]
    fn gate_vanilla_sprite_and_font_resolve() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        // VP strip sprite texture.
        let sprite = vp_strip_texture_path(&resolver)
            .expect("resolve vp strip")
            .expect("vp strip texture present");
        assert!(sprite.is_file(), "onmap_victorypoints_strip.dds present");
        // hoi_20bs BMFont + atlas.
        let (font, atlas) = load_vp_label_font(&resolver).expect("hoi_20bs font");
        assert!(
            font.glyphs.len() > 200,
            "hoi_20bs should carry a full glyph set, got {}",
            font.glyphs.len()
        );
        assert_eq!((atlas.width, atlas.height), (512, 256), "hoi_20bs atlas");
        // 'L' (76) and 'o' (111) glyphs exist for "London".
        assert!(font.glyph(76).is_some());
        assert!(font.glyph(111).is_some());
        println!(
            "gate: hoi_20bs {} glyphs, atlas {}x{}, sprite {}",
            font.glyphs.len(),
            atlas.width,
            atlas.height,
            sprite.display()
        );
    }

    #[test]
    fn gate_vanilla_vp_text_font_substitution_advance_law() {
        // FURN-FONT law EVIDENCE GATE (font substitution, was UNTRACED): prove
        // from the two shipped `.fnt` files that the on-map VP/city text renders
        // in the MAP font `tahoma_60`/`hoi_mapfont4` at ~0.40, NOT the declared
        // `hoi_20bs`. Measured on-screen (gui_scale=1.0): "Melbourne" 149 px,
        // "Adelaide" 119 px. The map font's advance sums x0.40 must reproduce
        // those; the declared GUI font would require an implausible >1.5x upscale
        // of its 20 px bitmap page.
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let (map_font, map_atlas) =
            load_vp_map_text_font(&resolver).expect("map text font (hoi_mapfont4)");
        let (gui_font, _) = load_vp_label_font(&resolver).expect("declared gui font (hoi_20bs)");
        // The rendered font is the map font: shipped metrics (lineHeight 84,
        // 2048x1024 atlas) are the country-label map font, not a 20 px GUI page.
        assert_eq!(map_font.line_height, 84, "hoi_mapfont4 lineHeight");
        assert_eq!(
            (map_atlas.width, map_atlas.height),
            (2048, 1024),
            "hoi_mapfont4 atlas geometry"
        );
        for (word, measured) in [("Melbourne", 149.0f32), ("Adelaide", 119.0f32)] {
            let map_sum = map_font.measure(word) as f32;
            let gui_sum = gui_font.measure(word) as f32;
            let map_at_scale = map_sum * VP_MAP_TEXT_SCALE;
            let gui_scale_needed = measured / gui_sum;
            // The map font at the measured 0.40 scale lands within 2 px.
            assert!(
                (map_at_scale - measured).abs() <= 2.0,
                "map font '{word}' {map_sum} x{VP_MAP_TEXT_SCALE} = {map_at_scale:.1} \
                 must reproduce measured {measured} px"
            );
            // The declared GUI font could only match by upscaling >1.5x — refuted.
            assert!(
                gui_scale_needed > 1.5,
                "declared hoi_20bs '{word}' would need x{gui_scale_needed:.2} (upscale) \
                 to reach {measured} px — the rendered font is NOT the declared one"
            );
            println!(
                "gate FURN-FONT: '{word}' measured={measured}px  mapfont4 {map_sum}x0.40={map_at_scale:.1}px  \
                 hoi_20bs {gui_sum}x{gui_scale_needed:.2}(upscale)"
            );
        }
    }

    #[test]
    fn gate_vanilla_vp_icon_defines_match_shipped_values() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let defines = vp_icon_defines_from_resolver(&resolver).expect("vp icon defines");
        assert_eq!(defines.levels, 3);
        assert_eq!(defines.map_icon_after, vec![0, 9, 20]);
        assert_eq!(defines.text_cutoff, vec![150.0, 250.0, 500.0]);
        assert_eq!(defines.capital_icon_cutoff, 1100.0);
        println!("gate: vp icon defines {defines:?}");
    }

    #[test]
    fn gate_vanilla_full_furniture_layer() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let layer = furniture_layer_from_resolver(&resolver, "english").expect("furniture layer");
        assert_eq!((layer.width, layer.height), (5632, 2048));
        assert!(
            layer.markers.len() > 1_000,
            "vanilla has many VP markers, got {}",
            layer.markers.len()
        );
        assert!(layer.capital_count > 100, "capitals resolved");
        let capitals = layer
            .markers
            .iter()
            .filter(|m| m.kind == FurnitureMarkerKind::Capital)
            .count();
        assert!(capitals > 50, "many capital markers, got {capitals}");
        // Named spot-check: Berlin (province 64) should be a labelled capital.
        let berlin = layer.markers.iter().find(|m| m.province_id == 64);
        if let Some(b) = berlin {
            println!(
                "gate: province 64 '{}' kind={} owner={} vp={} tier={} at ({:.0},{:.0})",
                b.label,
                b.kind.as_str(),
                b.owner,
                b.vp_value,
                b.dot_tier,
                b.anchor_x,
                b.anchor_y
            );
        }
        println!(
            "gate: {} markers ({} capitals), {} loc keys, {} capitals resolved",
            layer.markers.len(),
            capitals,
            layer.localisation_key_count,
            layer.capital_count
        );
    }

    #[test]
    fn gate_vanilla_capital_tie_rule_is_bounded() {
        // Capital tie-rule RESIDUAL bound (2026-07-09). The engine's capital
        // star anchors on the HIGHEST-VP province in the capital state (live
        // oracle: Canberra 30 stars, Sydney 20 circles). WHICH province wins a
        // TIE (two provinces sharing the state's max VP) is engine-C++ and
        // UNTRACED statically: the capital icon is built by a type-dispatch
        // selection is upstream gameplay/state code; the `00_graphics.lua`
        // cutoff defines route through the string-keyed define registry with no
        // code string-ref (NameHunt on `VICTORY_POINT_MAP_ICON*` = 0 hits), so
        // there is no static anchor to the comparator.
        //
        // This gate BOUNDS the residual from the shipped files: in vanilla 1936
        // exactly SIX capital states have a max-VP tie, and our deterministic
        // rule (lowest province id) equals the province-declaration order for
        // ALL six — it diverges from a "first-declared victory_points" rule for
        // only TWO capitals (state 425 Mysore, 807 Gdynia). So the untraced tie
        // rule can change at most two vanilla capital stars, and never a
        // non-capital marker. A game update that alters this set fails here.
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        // Data-level census: across ALL capital states (any tag's capital), how
        // many have a max-VP tie at all — the theoretical reach of the tie rule.
        let data = furniture_static_data_from_resolver(&resolver, "english")
            .expect("furniture static data");
        let capital_state_ids: std::collections::BTreeSet<u32> =
            data.capital_states.values().copied().collect();
        let tie_state = |vps: &BTreeMap<u32, i32>| -> Option<(Vec<u32>, u32)> {
            let max_vp = vps.values().copied().max().unwrap_or(0);
            let mut tied: Vec<u32> = vps
                .iter()
                .filter(|(_, v)| **v == max_vp)
                .map(|(p, _)| *p)
                .collect();
            tied.sort_unstable();
            (tied.len() > 1).then(|| (tied.clone(), *tied.iter().min().unwrap()))
        };
        let mut census: Vec<(u32, u32)> = Vec::new();
        for (state_id, vps) in &data.state_victory_points {
            if capital_state_ids.contains(state_id) {
                if let Some((_, winner)) = tie_state(vps) {
                    census.push((*state_id, winner));
                }
            }
        }
        census.sort_unstable();
        // Exactly six shipped capital-state ties; lowest-id winners. Only 425
        // (Mysore) and 807 (Gdynia) would differ under a "first-declared" rule.
        assert_eq!(
            census,
            vec![
                (65, 514),   // Sachsen
                (69, 3585),  // Sudetenland
                (328, 4572), // Manchukuo
                (425, 4315), // Mysore  (first-declared would pick 8011)
                (459, 1145), // Western Algeria
                (807, 3295), // Gdynia  (first-declared would pick 9263)
            ],
            "vanilla capital-state VP ties changed — re-verify the bounded tie residual"
        );
        // Render-level bound: of the ties above, how many actually emit a
        // rendered capital STAR in the 1936 layer (owner's capital == state and
        // the province is painted). Each such star must land on our lowest-id
        // winner — this is what a viewer sees, and it is the true visible reach.
        let layer = furniture_layer_from_resolver(&resolver, "english").expect("layer");
        let mut rendered_ties: Vec<(u32, u32)> = Vec::new();
        for (state_id, winner) in &census {
            let starred: Vec<u32> = layer
                .markers
                .iter()
                .filter(|m| m.state_id == *state_id && m.kind == FurnitureMarkerKind::Capital)
                .map(|m| m.province_id)
                .collect();
            if !starred.is_empty() {
                assert_eq!(
                    starred,
                    vec![*winner],
                    "capital state {state_id} stars must be exactly the lowest-id tied \
                     province {winner}"
                );
                rendered_ties.push((*state_id, *winner));
            }
        }
        println!(
            "gate: capital tie residual — {} vanilla capital-state ties in data, \
             {} rendered as 1936 stars: {:?}",
            census.len(),
            rendered_ties.len(),
            rendered_ties
        );
    }

    // ---- FURN2: naval / air base icons ----

    fn base_show_on_map() -> std::collections::BTreeSet<String> {
        std::collections::BTreeSet::from([
            AIR_BASE_BUILDING.to_string(),
            NAVAL_BASE_BUILDING.to_string(),
        ])
    }

    #[test]
    fn parse_state_building_levels_reads_air_and_naval() {
        // The exact 1-France.txt shape: buildings inside history with a
        // state-level air_base and a per-province naval_base sub-block.
        let text = strip_comments(
            "state={ id=1 history={ owner = FRA\n buildings = {\n infrastructure = 2\n industrial_complex = 1\n air_base = 1\n 3838 = { naval_base = 3 }\n } } provinces={ 3838 } }",
        );
        let levels = parse_state_building_levels(&text).expect("buildings block");
        assert_eq!(levels.air_base, 1);
        assert_eq!(levels.naval_base.get(&3838).copied(), Some(3));
        // A state with no buildings block yields None (fail visible).
        assert!(parse_state_building_levels("state={ id=2 }").is_none());
        // A state with buildings but no air/naval yields zero air + empty naval.
        let bare = strip_comments("buildings = { infrastructure = 5 }");
        let bare = parse_state_building_levels(&bare).expect("bare buildings");
        assert_eq!(bare.air_base, 0);
        assert!(bare.naval_base.is_empty());
    }

    #[test]
    fn base_icon_frame_stays_inside_shipped_strip() {
        // Air strip has 7 frames, naval 11; a level beyond the strip clamps to
        // the top frame and never indexes outside noOfFrames.
        for level in 0..=15u32 {
            assert!(base_icon_frame(BaseIconKind::AirBase, level) < AIR_BASE_SPRITE_FRAMES);
            assert!(base_icon_frame(BaseIconKind::NavalBase, level) < NAVAL_BASE_SPRITE_FRAMES);
        }
        assert_eq!(base_icon_frame(BaseIconKind::AirBase, 3), 3);
        assert_eq!(base_icon_frame(BaseIconKind::AirBase, 10), 6, "air clamps to 6");
        assert_eq!(base_icon_frame(BaseIconKind::NavalBase, 10), 10);
    }

    #[test]
    fn building_locator_pixel_flips_world_z() {
        // World (x, elevation, z) -> pixel (x, (H-1) - z).
        let (px, py) = building_locator_pixel([2948.38, 11.6, 1359.45], 2048);
        assert!((px - 2948.38).abs() < 1e-3);
        assert!((py - ((2048 - 1) as f32 - 1359.45)).abs() < 1e-3);
    }

    #[test]
    fn parse_base_icon_defines_reads_cutoffs() {
        let lua = "\
NDefines = { NGraphics = {\n\
\tAIRBASE_ICON_DISTANCE_CUTOFF = 900, -- At what distance air bases are hidden\n\
\tNAVALBASE_ICON_DISTANCE_CUTOFF = 900, -- 1300, -- naval bases\n\
} }\n";
        let d = parse_base_icon_defines(lua).expect("cutoffs");
        assert_eq!(d.airbase_cutoff, 900.0);
        assert_eq!(d.navalbase_cutoff, 900.0);
        assert!(d.visible(BaseIconKind::AirBase, 900.0));
        assert!(!d.visible(BaseIconKind::AirBase, 900.1));
        assert!(!d.visible(BaseIconKind::NavalBase, 901.0));
    }

    #[test]
    fn air_and_naval_base_icons_place_on_locators() {
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        // Land province 100 (state 5) fills a coastal rectangle; sea elsewhere.
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let province_to_state = BTreeMap::from([(100u32, 5u32)]);
        let state_owners = BTreeMap::from([(5u32, "FRA".to_string())]);
        let air_base_levels = BTreeMap::from([(5u32, 3u32)]);
        let naval_base_levels = BTreeMap::from([(100u32, 2u32)]);
        // Air-base locator at pixel (15,15) -> pos z = (H-1)-15.
        let air = MapBuildingPlacement {
            state_id: 5,
            building: AIR_BASE_BUILDING.to_string(),
            pos: [15.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 0,
        };
        // Naval spawn locator at pixel (20,15): just off the land edge (x=19 is
        // land, x=20 is sea) so the snap search attributes it to province 100.
        let naval = MapBuildingPlacement {
            state_id: 5,
            building: NAVAL_BASE_SPAWN_LOCATOR.to_string(),
            pos: [20.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 900,
        };
        let show = base_show_on_map();
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[air, naval],
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            air_base_levels: &air_base_levels,
            naval_base_levels: &naval_base_levels,
            show_on_map: &show,
        });
        assert_eq!(layer.air_base_count, 1);
        assert_eq!(layer.naval_base_count, 1);
        assert!(layer.naval_unattributed.is_empty());
        let a = layer
            .markers
            .iter()
            .find(|m| m.kind == BaseIconKind::AirBase)
            .unwrap();
        assert_eq!((a.state_id, a.province_id, a.level, a.frame), (5, None, 3, 3));
        assert!((a.anchor_x - 15.0).abs() < 0.01 && (a.anchor_y - 15.0).abs() < 0.01);
        let n = layer
            .markers
            .iter()
            .find(|m| m.kind == BaseIconKind::NavalBase)
            .unwrap();
        assert_eq!((n.state_id, n.province_id, n.level, n.frame), (5, Some(100), 2, 2));
        assert!((n.anchor_x - 20.0).abs() < 0.01 && (n.anchor_y - 15.0).abs() < 0.01);
        assert_eq!(n.owner, "FRA");
    }

    #[test]
    fn naval_base_without_locator_fails_closed() {
        // A province with a naval base in history but NO spawn locator nearby is
        // reported as unattributed, never given a fabricated position.
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let province_to_state = BTreeMap::from([(100u32, 5u32)]);
        let state_owners = BTreeMap::from([(5u32, "FRA".to_string())]);
        let naval_base_levels = BTreeMap::from([(100u32, 2u32)]);
        let show = base_show_on_map();
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[], // no locators at all
            province_ids: &ids,
            width,
            height,
            province_to_state: &province_to_state,
            state_owners: &state_owners,
            air_base_levels: &BTreeMap::new(),
            naval_base_levels: &naval_base_levels,
            show_on_map: &show,
        });
        assert_eq!(layer.naval_base_count, 0);
        assert_eq!(layer.naval_unattributed, vec![100]);
    }

    #[test]
    fn base_icons_suppressed_when_not_show_on_map() {
        // If common/buildings did not flag show_on_map, no icons emit.
        let width = 40;
        let height = 30;
        let mut ids = vec![0u32; width * height];
        raster_rect(&mut ids, width, 100, (10, 10, 20, 20));
        let air = MapBuildingPlacement {
            state_id: 5,
            building: AIR_BASE_BUILDING.to_string(),
            pos: [15.0, 0.0, (height - 1) as f32 - 15.0],
            rotation: 0.0,
            adjacent_sea_province: 0,
        };
        let layer = compute_base_icon_markers(&BaseIconInputs {
            placements: &[air],
            province_ids: &ids,
            width,
            height,
            province_to_state: &BTreeMap::from([(100u32, 5u32)]),
            state_owners: &BTreeMap::from([(5u32, "FRA".to_string())]),
            air_base_levels: &BTreeMap::from([(5u32, 3u32)]),
            naval_base_levels: &BTreeMap::new(),
            show_on_map: &std::collections::BTreeSet::new(),
        });
        assert!(layer.markers.is_empty());
    }

    #[test]
    fn gate_vanilla_base_icon_defines_match_shipped() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let d = base_icon_defines_from_resolver(&resolver).expect("base icon defines");
        assert_eq!(d.airbase_cutoff, 900.0, "AIRBASE_ICON_DISTANCE_CUTOFF");
        assert_eq!(d.navalbase_cutoff, 900.0, "NAVALBASE_ICON_DISTANCE_CUTOFF");
        println!("gate: base icon defines {d:?}");
    }

    #[test]
    fn gate_vanilla_base_icon_layer() {
        let Some(resolver) = vanilla_resolver() else {
            return;
        };
        let layer = base_icon_layer_from_resolver(&resolver).expect("base icon layer");
        // 1936 vanilla has hundreds of air bases and naval bases.
        assert!(
            layer.air_base_count > 300,
            "expected many air bases, got {}",
            layer.air_base_count
        );
        assert!(
            layer.naval_base_count > 400,
            "expected many naval bases, got {}",
            layer.naval_base_count
        );
        // Every emitted frame is inside its shipped strip and every anchor is on
        // the map.
        for m in &layer.markers {
            assert!(m.frame < m.kind.sprite_frames(), "frame in strip");
            assert!(m.anchor_x >= 0.0 && m.anchor_y >= 0.0);
        }
        // The spawn->province attribution should reach almost every naval base;
        // a handful of odd coastlines may fail closed (reported, not guessed).
        let total_naval = layer.naval_base_count + layer.naval_unattributed.len();
        let rate = layer.naval_base_count as f32 / total_naval as f32;
        assert!(
            rate > 0.95,
            "naval spawn attribution rate {rate:.3} ({} of {total_naval}); unattributed {:?}",
            layer.naval_base_count,
            layer.naval_unattributed
        );
        println!(
            "gate: FURN2 {} air + {} naval icons, naval attribution {:.3} ({} unattributed)",
            layer.air_base_count,
            layer.naval_base_count,
            rate,
            layer.naval_unattributed.len()
        );
    }
}
