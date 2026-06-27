//! `map` — the M4 GPU political map pass (integer province-id texture + colour LUT).
//!
//! ## Design
//!
//! The province-id raster (`province_ids.png`, 5632×2048, 8-bit RGB, no alpha) is
//! NOT a display image — each pixel encodes an integer province id via
//! `id = R + G*256 + B*65536` (R is the low byte). To carry those ids to the GPU
//! WITHOUT any filtering / sRGB / mip corruption we:
//!
//!   * decode the PNG to RGBA8 on the CPU and pack each pixel into a `u32`
//!     province id, then upload as an **`R32Uint`** texture (`texture_2d<u32>`),
//!     read in-shader with **`textureLoad`** (integer textures are non-filterable;
//!     `textureSample` is illegal on them — this is the only correct path);
//!   * build a per-province **colour LUT**: a 1-D `Rgba8Unorm` texture of width
//!     `max_province_id + 1`, also read via `textureLoad` (so no sampler is needed
//!     and there is no filtering between LUT entries either);
//!   * the fragment shader: `floor(uv * dims)` → `textureLoad` the province id →
//!     `textureLoad` the LUT at `(id, 0)` → output that colour.
//!
//! ## No-fabrication (the political colouring invariant)
//!
//! The LUT is rebuilt from scratch every snapshot tick. EVERY entry is first set to
//! the neutral TERRAIN colour (this is the default for sea / lake / unassigned /
//! province-id 0 — anything NOT a known land province). Then, for each
//! `province → state` in `province_to_state` (i.e. every LAND province), we look up
//! the snapshot state's `owner` and EXPLICITLY write a colour into that entry:
//!   * owner is a non-null TAG with a country colour → that country colour,
//!   * owner is null / absent, OR the state is absent from the snapshot, OR the TAG
//!     has no colour entry → neutral GREY (a land province owned by nobody we can
//!     resolve — never an invented country).
//! Land provinces are thus written to grey/colour; sea/unassigned (never touched by
//! the loop) stay terrain. The two neutrals are distinct on purpose (Codex-flagged):
//! "owned by nobody" (grey) must read differently from "not land" (terrain), and we
//! must WRITE grey, not merely "leave" the entry (else it would stay terrain).
//! Because the LUT is fully reinitialised each rebuild there is no carry-over of a
//! previous tick's colour, and a full-width overwrite + the shader's id bounds-check
//! mean no stale/out-of-range GPU texel can leak a fabricated colour.
//!
//! ## Pan / zoom
//!
//! A small uniform (`offset`, `scale`) maps the output viewport UV into map UV:
//! `map_uv = output_uv / scale + offset`. The full-world view is `scale=1, offset=0`;
//! a zoom is `scale>1` with an offset onto the region of interest. Out-of-range
//! `map_uv` (after pan) renders neutral terrain (no wrap, clamp-to-edge semantics).
//!
//! ## Picking (CPU-side, no GPU readback)
//!
//! [`ProvinceRaster::province_at`] indexes the in-memory id buffer at a pixel
//! coordinate → province id; [`PoliticalMap::pick`] then resolves
//! id → state → owner / state-name entirely on the CPU.

use std::collections::HashMap;
use std::path::Path;

use bytemuck::{Pod, Zeroable};

use crate::Renderer;

/// Neutral terrain colour for sea / lake / unassigned / unowned provinces.
/// A muted blue-grey — deliberately NOT any country colour. RGBA, straight alpha.
///
/// ## Alpha = the fidelity compositing flag (Codex A, 2026-06-26)
/// The LUT alpha channel is a VALIDITY bit for the terrain-composite shader, NOT a
/// blend opacity: alpha `255` = "this province has a real mode tint, composite it
/// ~55% over the terrain underlay"; alpha `0` = "no tint — show the terrain/water
/// base untouched". Sea / lake / unassigned (never written by the land loop) keep
/// alpha `0`; every LAND cell (a real colour OR neutral grey for unowned) is written
/// with alpha `255`. The RGB DATA is unchanged — only the alpha flag is new, so the
/// no-fabrication invariant (which province is owned by whom) is preserved exactly.
pub const NEUTRAL_TERRAIN: [u8; 4] = [60, 74, 92, 0];
/// Neutral grey for a LAND province whose state has a `null` / unknown owner.
/// Distinct from terrain so "owned by nobody" reads differently from "sea".
/// Alpha `255` → it IS land (tinted grey over terrain), unlike sea (alpha 0).
pub const NEUTRAL_GREY: [u8; 4] = [120, 120, 120, 255];

/// Decoded province-id raster, kept in CPU memory for both GPU upload and picking.
pub struct ProvinceRaster {
    pub width: u32,
    pub height: u32,
    /// One `u32` province id per pixel, row-major, length `width*height`.
    pub ids: Vec<u32>,
    pub max_id: u32,
}

impl ProvinceRaster {
    /// Decode `province_ids.png` (8-bit RGB) and pack each pixel into a province
    /// id via `id = R + G*256 + B*65536`. Tolerant of RGB or RGBA source.
    pub fn load_png(path: &Path) -> Result<ProvinceRaster, String> {
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let decoder = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let info = reader.info();
        let (width, height) = (info.width, info.height);
        let color = info.color_type;
        // Bytes-per-pixel in the decoded output buffer.
        let channels = color.samples();
        if info.bit_depth != png::BitDepth::Eight {
            return Err(format!(
                "{}: expected 8-bit depth, got {:?}",
                path.display(),
                info.bit_depth
            ));
        }
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let frame = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let data = &buf[..frame.buffer_size()];
        let n = (width as usize) * (height as usize);
        let mut ids = vec![0u32; n];
        let mut max_id = 0u32;
        // R is the LOW byte (per province_ids_meta: id = R + G*256 + B*65536).
        for i in 0..n {
            let o = i * channels;
            let r = data[o] as u32;
            let g = data[o + 1] as u32;
            let b = data[o + 2] as u32;
            let id = r | (g << 8) | (b << 16);
            ids[i] = id;
            if id > max_id {
                max_id = id;
            }
        }
        Ok(ProvinceRaster {
            width,
            height,
            ids,
            max_id,
        })
    }

    /// Province id at an integer pixel coordinate (top-left origin). Returns
    /// `None` if the coordinate is out of bounds. CPU pick — no GPU readback.
    pub fn province_at(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(self.ids[(y as usize) * (self.width as usize) + (x as usize)])
    }
}

/// Typed map-id lookup — the audit-flagged normalization. ALL ids are carried as
/// `u32` internally; the ONLY string boundary is the snapshot's string-keyed
/// `states` map, crossed via [`StateOwners`]. This eliminates the silent-miss
/// hazard (a forgotten `to_string()`/`parse()` that would render the whole map
/// as fake-unowned).
#[derive(Debug, Clone, Default)]
pub struct MapIds {
    /// province id (u32) -> state id (u32). Parsed once at load.
    province_to_state: HashMap<u32, u32>,
    /// state id (u32) -> name_key (from state_meta). For hover.
    state_name_key: HashMap<u32, String>,
}

impl MapIds {
    /// Build from the already-parsed `province_to_state` (string keys/int values
    /// in JSON; this takes them post-parse as u32/u32) plus state_meta name keys.
    pub fn new(province_to_state: HashMap<u32, u32>, state_name_key: HashMap<u32, String>) -> Self {
        MapIds {
            province_to_state,
            state_name_key,
        }
    }

    /// Load directly from the map_pipeline JSON files, normalizing every id to
    /// `u32` at the boundary. `province_to_state.json` = string keys / int values;
    /// `state_meta.json` = string keys / objects with `name_key`.
    pub fn load(dir: &Path) -> Result<MapIds, String> {
        let p2s_txt = std::fs::read_to_string(dir.join("province_to_state.json"))
            .map_err(|e| format!("province_to_state.json: {e}"))?;
        let p2s_raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(&p2s_txt).map_err(|e| format!("province_to_state.json: {e}"))?;
        let mut province_to_state = HashMap::with_capacity(p2s_raw.len());
        for (k, v) in p2s_raw {
            let pid = k
                .parse::<u32>()
                .map_err(|_| format!("province_to_state: bad province key {k:?}"))?;
            let sid = v
                .as_u64()
                .ok_or_else(|| format!("province_to_state: bad state value for {k:?}"))?
                as u32;
            province_to_state.insert(pid, sid);
        }

        let sm_txt = std::fs::read_to_string(dir.join("state_meta.json"))
            .map_err(|e| format!("state_meta.json: {e}"))?;
        let sm_raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(&sm_txt).map_err(|e| format!("state_meta.json: {e}"))?;
        let mut state_name_key = HashMap::with_capacity(sm_raw.len());
        for (k, v) in sm_raw {
            if let Ok(sid) = k.parse::<u32>() {
                if let Some(name) = v.get("name_key").and_then(|n| n.as_str()) {
                    state_name_key.insert(sid, name.to_string());
                }
            }
        }

        Ok(MapIds {
            province_to_state,
            state_name_key,
        })
    }

    /// province id -> state id, if the province is a known LAND province. A `None`
    /// means sea / lake / unassigned (absent from the land map) → neutral terrain.
    pub fn state_of_province(&self, province_id: u32) -> Option<u32> {
        self.province_to_state.get(&province_id).copied()
    }

    /// state id -> `name_key` (for hover), if present in state_meta.
    pub fn state_name_key(&self, state_id: u32) -> Option<&str> {
        self.state_name_key.get(&state_id).map(|s| s.as_str())
    }

    pub fn province_count(&self) -> usize {
        self.province_to_state.len()
    }

    /// Iterate `(province_id, state_id)` over every LAND province. Used by the map-mode
    /// recolour functions ([`crate::modes`]) to fill the per-province LUT exactly as
    /// [`build_lut`] does for the political mode.
    pub fn iter_provinces(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.province_to_state.iter().map(|(&p, &s)| (p, s))
    }
}

/// Snapshot-side owner lookup: state id -> owner TAG. The ONLY string boundary.
/// Built once per snapshot from `states.<id>.owner`. A `None` (absent) and an
/// explicit null both yield "no owner" → neutral grey (never fabricated).
#[derive(Debug, Clone, Default)]
pub struct StateOwners {
    owner_by_state: HashMap<u32, Option<String>>,
}

impl StateOwners {
    /// Build from a snapshot's `states` object (the serde_json `states` map,
    /// string-keyed). `owner` may be a string TAG, explicit `null`, or absent;
    /// all of `null`/absent collapse to `None` here (both → neutral grey).
    pub fn from_states(states: &serde_json::Map<String, serde_json::Value>) -> StateOwners {
        let mut owner_by_state = HashMap::with_capacity(states.len());
        for (sid_str, sval) in states {
            // Normalize the snapshot string key to u32 at THIS boundary.
            let Ok(sid) = sid_str.parse::<u32>() else {
                continue;
            };
            let owner = match sval.get("owner") {
                Some(serde_json::Value::String(s)) => Some(s.clone()),
                // explicit null OR a non-string -> no owner (neutral, not fabricated)
                _ => None,
            };
            owner_by_state.insert(sid, owner);
        }
        StateOwners { owner_by_state }
    }

    /// Owner TAG for a state id. `None` = state absent from snapshot OR owner
    /// null/absent — caller renders neutral grey, never an invented country.
    pub fn owner_of_state(&self, state_id: u32) -> Option<&str> {
        self.owner_by_state.get(&state_id)?.as_deref()
    }
}

/// Country TAG -> RGB colour (from `country_colors.json`).
#[derive(Debug, Clone, Default)]
pub struct CountryColors {
    colors: HashMap<String, [u8; 3]>,
}

impl CountryColors {
    pub fn new(colors: HashMap<String, [u8; 3]>) -> Self {
        CountryColors { colors }
    }
    pub fn load(dir: &Path) -> Result<CountryColors, String> {
        let txt = std::fs::read_to_string(dir.join("country_colors.json"))
            .map_err(|e| format!("country_colors.json: {e}"))?;
        let raw: HashMap<String, serde_json::Value> =
            serde_json::from_str(&txt).map_err(|e| format!("country_colors.json: {e}"))?;
        let mut colors = HashMap::with_capacity(raw.len());
        for (tag, v) in raw {
            if let Some(arr) = v.as_array() {
                if arr.len() >= 3 {
                    let c = |i: usize| arr[i].as_u64().unwrap_or(128).min(255) as u8;
                    colors.insert(tag, [c(0), c(1), c(2)]);
                }
            }
        }
        Ok(CountryColors { colors })
    }
    pub fn get(&self, tag: &str) -> Option<[u8; 3]> {
        self.colors.get(tag).copied()
    }
}

/// The result of a CPU pick: province id, and its state/owner/name when resolvable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickResult {
    pub province_id: u32,
    /// `None` => sea / lake / unassigned (not a land province).
    pub state_id: Option<u32>,
    /// `None` => no state, OR state has null/absent owner (neutral grey).
    pub owner: Option<String>,
    /// `None` => no state, or state has no name_key.
    pub state_name_key: Option<String>,
}

/// Build the per-province colour LUT for one snapshot. Returns a flat RGBA8 buffer
/// of length `(max_id+1)*4`. EVERY entry starts neutral; only a province that
/// resolves province→state→owner→colour gets a real country colour. This is the
/// no-fabrication construction (see module docs).
pub fn build_lut(
    max_id: u32,
    ids: &MapIds,
    owners: &StateOwners,
    colors: &CountryColors,
) -> Vec<u8> {
    let n = (max_id as usize) + 1;
    let mut lut = vec![0u8; n * 4];
    // 1) initialise EVERY entry to neutral terrain (sea/lake/unassigned default).
    for i in 0..n {
        lut[i * 4..i * 4 + 4].copy_from_slice(&NEUTRAL_TERRAIN);
    }
    // 2) for every LAND province, override with its owner colour, or neutral grey
    //    when the state exists but has no resolvable owner colour.
    for (&province_id, &state_id) in ids.province_to_state.iter() {
        if province_id > max_id {
            continue; // defensive: a land province id beyond the raster's max
        }
        let cell = (province_id as usize) * 4;
        match owners.owner_of_state(state_id) {
            Some(tag) => match colors.get(tag) {
                Some([r, g, b]) => lut[cell..cell + 4].copy_from_slice(&[r, g, b, 255]),
                // owner exists but no colour entry -> neutral grey (NOT fabricated)
                None => lut[cell..cell + 4].copy_from_slice(&NEUTRAL_GREY),
            },
            // state absent from snapshot OR owner null/absent -> neutral grey
            None => lut[cell..cell + 4].copy_from_slice(&NEUTRAL_GREY),
        }
    }
    lut
}

/// Build the per-province OWNER-INDEX buffer used by the shader to draw COUNTRY
/// borders (Codex C: a dedicated owner key, NOT colour comparison — two countries can
/// share a colour, and non-political modes don't colour by owner at all).
///
/// Returns a `Vec<u32>` of length `max_id+1`: for each province id, a STABLE owner key:
///   * a real owner TAG → `1 + hash(tag)` (folded into 24 bits, never 0),
///   * a land province with null/absent/unknown owner → `OWNER_UNOWNED` (a distinct
///     non-zero key so "unowned land" borders against owned land but not against sea),
///   * sea / lake / unassigned (never a land province) → `OWNER_NONE` (0).
///
/// This is purely a DRAWING aid: the key is derived from the same owner data the LUT
/// uses, never invents an owner, and is compared only to decide where to darken a
/// border. The mode is irrelevant — country borders are always owner borders.
pub fn build_owner_index(max_id: u32, ids: &MapIds, owners: &StateOwners) -> Vec<u32> {
    let n = (max_id as usize) + 1;
    let mut idx = vec![OWNER_NONE; n]; // sea/unassigned default
    for (&province_id, &state_id) in ids.province_to_state.iter() {
        if province_id > max_id {
            continue;
        }
        let key = match owners.owner_of_state(state_id) {
            Some(tag) => owner_key(tag),
            None => OWNER_UNOWNED,
        };
        idx[province_id as usize] = key;
    }
    idx
}

/// Build the SECONDARY-COLOUR LUT — the game's `ProvinceSecondaryColorMap` equivalent:
/// for every province whose CONTROLLER differs from its OWNER (an occupied province), the
/// CONTROLLER's country colour, with alpha 255 (= "draw occupation stripes here"); every
/// other province is alpha 0 (no stripes). The WGSL then applies the exact diagonal-cosine
/// occupation-stripe mask (`CalculateOccupationMask`, [standardfuncsgfx.fxh:965-987]) so
/// occupied land reads as the game's striped political border.
///
/// `province_controller` is `ModeData.province_control` (province id → controller TAG).
/// Owner is resolved per province via its state (the same data the political LUT uses) —
/// no fabrication: a province is striped only when both an owner and a DIFFERENT controller
/// are known, and the stripe colour is the real controller country colour.
pub fn build_secondary_lut(
    max_id: u32,
    ids: &MapIds,
    owners: &StateOwners,
    province_controller: &HashMap<u32, String>,
    colors: &CountryColors,
) -> Vec<u8> {
    let n = (max_id as usize) + 1;
    let mut lut = vec![0u8; n * 4]; // default alpha 0 = no stripes
    for (&province_id, &state_id) in ids.province_to_state.iter() {
        if province_id > max_id {
            continue;
        }
        let owner = owners.owner_of_state(state_id);
        let ctrl = province_controller.get(&province_id).map(String::as_str);
        if let (Some(o), Some(c)) = (owner, ctrl) {
            if o != c {
                if let Some([r, g, b]) = colors.get(c) {
                    let cell = (province_id as usize) * 4;
                    lut[cell..cell + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
    }
    lut
}

/// Owner key for sea / not-a-land-province (no border ever drawn against it as if it
/// were a different country — water gets its own coastline treatment).
pub const OWNER_NONE: u32 = 0;
/// Owner key for a land province whose owner is null/absent/unknown. A distinct,
/// non-zero key so unowned-vs-owned land still shows a border but sea does not.
pub const OWNER_UNOWNED: u32 = 1;

/// Stable 24-bit owner key from a TAG (FNV-1a folded), offset past the reserved
/// `OWNER_NONE`/`OWNER_UNOWNED` so it can never collide with them.
fn owner_key(tag: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in tag.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    // fold to 24 bits, then push past the 2 reserved keys.
    2 + (h & 0x00ff_ffff)
}

/// CPU-side pick: pixel (x,y) → province → state → owner / name. No GPU readback.
pub fn pick_at(
    raster: &ProvinceRaster,
    ids: &MapIds,
    owners: &StateOwners,
    x: u32,
    y: u32,
) -> Option<PickResult> {
    let province_id = raster.province_at(x, y)?;
    let state_id = ids.state_of_province(province_id);
    let owner = state_id
        .and_then(|s| owners.owner_of_state(s))
        .map(|t| t.to_string());
    let state_name_key = state_id
        .and_then(|s| ids.state_name_key(s))
        .map(|s| s.to_string());
    Some(PickResult {
        province_id,
        state_id,
        owner,
        state_name_key,
    })
}

// ---------------------------------------------------------------------------
// Tests — id-typing normalization, no-fabrication LUT, and pick resolution.
// These are PURE (no GPU); the GPU pass is exercised by the m4_demo binary.
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn colors_with(tag: &str, rgb: [u8; 3]) -> CountryColors {
        let mut m = HashMap::new();
        m.insert(tag.to_string(), rgb);
        CountryColors::new(m)
    }

    /// The audit-flagged hazard (Codex (2)): province_to_state has STRING keys /
    /// INT values, the snapshot's states map has STRING keys. If the typing is
    /// wrong the lookup misses and the WHOLE map renders fake-unowned. This is the
    /// exact tiny fixture Codex specified: province 7 -> state 42 -> owner ITA ->
    /// colour [1,2,3] must land in lut[7], and EXACTLY one cell is coloured.
    #[test]
    fn lut_id_typing_roundtrip() {
        let mut p2s = HashMap::new();
        p2s.insert(7u32, 42u32);
        let ids = MapIds::new(p2s, HashMap::new());

        let mut states = serde_json::Map::new();
        states.insert("42".to_string(), serde_json::json!({"owner": "ITA"}));
        let owners = StateOwners::from_states(&states);
        let colors = colors_with("ITA", [1, 2, 3]);

        let max_id = 7;
        let lut = build_lut(max_id, &ids, &owners, &colors);
        // province 7 got the ITA colour.
        assert_eq!(&lut[7 * 4..7 * 4 + 4], &[1, 2, 3, 255]);
        // exactly one cell is a COUNTRY colour (not terrain, not grey).
        let colored = (0..=max_id as usize)
            .filter(|&i| &lut[i * 4..i * 4 + 4] == &[1, 2, 3, 255])
            .count();
        assert_eq!(colored, 1, "exactly one province should be ITA-coloured");
    }

    /// No-fabrication (Codex (3)): the three neutral cases are distinct and a null
    /// / missing / unknown owner NEVER yields a fabricated country colour.
    #[test]
    fn no_fabrication_neutrals() {
        // provinces: 1->land owned (ITA), 2->land null-owner, 3->land state-absent,
        //            4->land owner-without-colour. province 5 is SEA (absent p2s).
        let mut p2s = HashMap::new();
        p2s.insert(1u32, 10u32); // owned
        p2s.insert(2u32, 20u32); // owner null
        p2s.insert(3u32, 30u32); // state absent from snapshot
        p2s.insert(4u32, 40u32); // owner has no colour entry
        let ids = MapIds::new(p2s, HashMap::new());

        let mut states = serde_json::Map::new();
        states.insert("10".into(), serde_json::json!({"owner": "ITA"}));
        states.insert("20".into(), serde_json::json!({"owner": null}));
        // state 30 deliberately absent.
        states.insert("40".into(), serde_json::json!({"owner": "ZZZ"}));
        let owners = StateOwners::from_states(&states);
        let colors = colors_with("ITA", [200, 30, 30]);

        let lut = build_lut(5, &ids, &owners, &colors);
        // 1 = real owner colour
        assert_eq!(&lut[1 * 4..1 * 4 + 4], &[200, 30, 30, 255]);
        // 2 = null owner -> neutral GREY (land owned by nobody), NOT a country
        assert_eq!(&lut[2 * 4..2 * 4 + 4], &NEUTRAL_GREY);
        // 3 = state absent -> neutral GREY (still a land province)
        assert_eq!(&lut[3 * 4..3 * 4 + 4], &NEUTRAL_GREY);
        // 4 = owner exists but no colour -> neutral GREY, never fabricated
        assert_eq!(&lut[4 * 4..4 * 4 + 4], &NEUTRAL_GREY);
        // 5 = SEA (never in p2s) -> neutral TERRAIN, untouched by the loop
        assert_eq!(&lut[5 * 4..5 * 4 + 4], &NEUTRAL_TERRAIN);
        // 0 = sentinel -> terrain
        assert_eq!(&lut[0..4], &NEUTRAL_TERRAIN);
        // No fabricated country: the only country colour present is ITA's, once.
        let ita = (0..=5usize)
            .filter(|&i| &lut[i * 4..i * 4 + 4] == &[200, 30, 30, 255])
            .count();
        assert_eq!(ita, 1);
    }

    /// The rebuild is stateless: rebuilding with a different owner does NOT leak the
    /// previous tick's colour (the LUT is fully reinitialised each call).
    #[test]
    fn lut_rebuild_no_carryover() {
        let mut p2s = HashMap::new();
        p2s.insert(1u32, 10u32);
        let ids = MapIds::new(p2s, HashMap::new());
        let mut colors = HashMap::new();
        colors.insert("AAA".to_string(), [10, 20, 30]);
        colors.insert("BBB".to_string(), [90, 80, 70]);
        let colors = CountryColors::new(colors);

        let mut s1 = serde_json::Map::new();
        s1.insert("10".into(), serde_json::json!({"owner": "AAA"}));
        let lut1 = build_lut(1, &ids, &StateOwners::from_states(&s1), &colors);
        assert_eq!(&lut1[4..8], &[10, 20, 30, 255]);

        // Tick 2: same province now owner BBB — must show BBB, never AAA residue.
        let mut s2 = serde_json::Map::new();
        s2.insert("10".into(), serde_json::json!({"owner": "BBB"}));
        let lut2 = build_lut(1, &ids, &StateOwners::from_states(&s2), &colors);
        assert_eq!(&lut2[4..8], &[90, 80, 70, 255]);

        // Tick 3: province now null-owner -> grey, no stale BBB.
        let mut s3 = serde_json::Map::new();
        s3.insert("10".into(), serde_json::json!({"owner": null}));
        let lut3 = build_lut(1, &ids, &StateOwners::from_states(&s3), &colors);
        assert_eq!(&lut3[4..8], &NEUTRAL_GREY);
    }

    /// Owner-index buffer (country-border key) — Codex C: a stable owner key per
    /// province, sea = OWNER_NONE, unowned land = OWNER_UNOWNED, owned = a folded TAG
    /// hash distinct from both. No owner is ever invented.
    #[test]
    fn owner_index_border_keys() {
        let mut p2s = HashMap::new();
        p2s.insert(1u32, 10u32); // owned by ITA
        p2s.insert(2u32, 20u32); // owned by FRA
        p2s.insert(3u32, 30u32); // null owner -> unowned
                                 // province 9 = sea (absent from p2s)
        let ids = MapIds::new(p2s, HashMap::new());

        let mut states = serde_json::Map::new();
        states.insert("10".into(), serde_json::json!({"owner": "ITA"}));
        states.insert("20".into(), serde_json::json!({"owner": "FRA"}));
        states.insert("30".into(), serde_json::json!({"owner": null}));
        let owners = StateOwners::from_states(&states);

        let oi = build_owner_index(9, &ids, &owners);
        assert_eq!(oi.len(), 10);
        // sea (province 9, and 0/4..8) -> OWNER_NONE.
        assert_eq!(oi[9], OWNER_NONE);
        assert_eq!(oi[0], OWNER_NONE);
        // owned provinces -> distinct non-reserved keys; ITA != FRA.
        assert!(oi[1] >= 2 && oi[2] >= 2);
        assert_ne!(oi[1], oi[2], "different owners must get different keys");
        // unowned land -> OWNER_UNOWNED (distinct from sea AND from owned).
        assert_eq!(oi[3], OWNER_UNOWNED);
        assert_ne!(oi[3], oi[1]);
        assert_ne!(oi[3], OWNER_NONE);
        // a country border exists between province 1 (ITA) and 2 (FRA): keys differ.
        assert_ne!(oi[1], oi[2]);
    }

    /// The same TAG always folds to the same key (stable), and two different TAGs do
    /// not collide on the small real-roster set.
    #[test]
    fn owner_key_stable_and_distinct() {
        assert_eq!(owner_key("ITA"), owner_key("ITA"));
        let tags = [
            "ITA", "FRA", "GER", "ENG", "USA", "SOV", "JAP", "CAN", "RAJ", "WST",
        ];
        let mut keys: Vec<u32> = tags.iter().map(|t| owner_key(t)).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "owner keys collided on the real roster");
        // never equal to the reserved keys.
        for t in tags {
            assert!(owner_key(t) > OWNER_UNOWNED);
        }
    }

    /// Alpha is the compositing-validity flag (Codex A): sea LUT cells carry alpha 0
    /// (terrain shows through, no tint); land cells carry alpha 255 (tint composited).
    #[test]
    fn lut_alpha_is_validity_flag() {
        let mut p2s = HashMap::new();
        p2s.insert(1u32, 10u32); // land owned
        p2s.insert(2u32, 20u32); // land null-owner -> grey
        let ids = MapIds::new(p2s, HashMap::new());
        let mut states = serde_json::Map::new();
        states.insert("10".into(), serde_json::json!({"owner": "ITA"}));
        states.insert("20".into(), serde_json::json!({"owner": null}));
        let owners = StateOwners::from_states(&states);
        let colors = colors_with("ITA", [10, 20, 30]);
        let lut = build_lut(3, &ids, &owners, &colors);
        // owned land: real colour, alpha 255 (composite the tint).
        assert_eq!(&lut[1 * 4..1 * 4 + 4], &[10, 20, 30, 255]);
        // unowned land: grey, alpha 255 (still tinted over terrain).
        assert_eq!(lut[2 * 4 + 3], 255);
        // sea (province 3 absent from p2s): NEUTRAL_TERRAIN with alpha 0.
        assert_eq!(lut[3 * 4 + 3], 0, "sea cell must be alpha 0 (no tint)");
        assert_eq!(&lut[3 * 4..3 * 4 + 4], &NEUTRAL_TERRAIN);
    }

    /// Pixel-pack: `id = R + G*256 + B*65536` from an RGB(A) buffer, R low byte.
    #[test]
    fn province_id_unpack() {
        // rgb (197,37,0) -> 197 + 37*256 = 9669 (a real CAN province from the raster)
        let r = 197u32;
        let g = 37u32;
        let b = 0u32;
        assert_eq!(r | (g << 8) | (b << 16), 9669);
        // rgb (86,50,0) -> 86 + 50*256 = 12886 (a real RAJ province)
        assert_eq!(86u32 | (50u32 << 8) | (0u32 << 16), 12886);
    }

    /// Snapshot key normalization is strict (Codex (2)): a non-canonical state key
    /// (non-decimal) is rejected rather than silently matching nothing meaningful.
    #[test]
    fn snapshot_keys_parse_to_u32() {
        let mut states = serde_json::Map::new();
        states.insert("42".into(), serde_json::json!({"owner": "ITA"}));
        states.insert("notanumber".into(), serde_json::json!({"owner": "FRA"}));
        let owners = StateOwners::from_states(&states);
        assert_eq!(owners.owner_of_state(42), Some("ITA"));
        // the non-numeric key was skipped (never reachable by a u32 lookup).
        assert_eq!(owners.owner_of_state(0), None);
    }

    /// CPU pick resolves pixel -> province -> state -> owner / name. Uses a small
    /// synthetic raster; the m4_demo binary additionally verifies pick against the
    /// REAL raster + snapshot at the ground-truth coordinates.
    #[test]
    fn pick_resolves_chain() {
        // 2x1 raster: pixel(0,0)=province 7 (land), pixel(1,0)=province 999 (sea).
        let raster = ProvinceRaster {
            width: 2,
            height: 1,
            ids: vec![7, 999],
            max_id: 999,
        };
        let mut p2s = HashMap::new();
        p2s.insert(7u32, 42u32);
        let mut names = HashMap::new();
        names.insert(42u32, "STATE_42".to_string());
        let ids = MapIds::new(p2s, names);
        let mut states = serde_json::Map::new();
        states.insert("42".into(), serde_json::json!({"owner": "ITA"}));
        let owners = StateOwners::from_states(&states);

        let land = pick_at(&raster, &ids, &owners, 0, 0).unwrap();
        assert_eq!(land.province_id, 7);
        assert_eq!(land.state_id, Some(42));
        assert_eq!(land.owner.as_deref(), Some("ITA"));
        assert_eq!(land.state_name_key.as_deref(), Some("STATE_42"));

        let sea = pick_at(&raster, &ids, &owners, 1, 0).unwrap();
        assert_eq!(sea.province_id, 999);
        assert_eq!(sea.state_id, None); // not a land province
        assert_eq!(sea.owner, None); // never fabricated
        assert_eq!(sea.state_name_key, None);

        // out of bounds -> None (not a fabricated province).
        assert!(pick_at(&raster, &ids, &owners, 5, 0).is_none());
    }

    /// The secondary-colour LUT (occupation stripes) marks ONLY provinces whose controller
    /// differs from their owner, with the CONTROLLER's colour and alpha 255; same-owner and
    /// uncontrolled provinces stay alpha 0 (no stripes). No fabrication: a missing colour or
    /// a null owner/controller leaves the province unmarked.
    #[test]
    fn secondary_lut_marks_only_occupied() {
        // provinces: 1 (state 10, owner ITA, controller FRA = OCCUPIED),
        //            2 (state 20, owner GER, controller GER = not occupied),
        //            3 (state 30, owner JAP, no controller entry = not occupied).
        let mut p2s = HashMap::new();
        p2s.insert(1u32, 10u32);
        p2s.insert(2u32, 20u32);
        p2s.insert(3u32, 30u32);
        let ids = MapIds::new(p2s, HashMap::new());
        let mut states = serde_json::Map::new();
        states.insert("10".into(), serde_json::json!({"owner": "ITA"}));
        states.insert("20".into(), serde_json::json!({"owner": "GER"}));
        states.insert("30".into(), serde_json::json!({"owner": "JAP"}));
        let owners = StateOwners::from_states(&states);
        let mut colors = HashMap::new();
        colors.insert("FRA".to_string(), [10u8, 20, 30]);
        colors.insert("GER".to_string(), [40u8, 50, 60]);
        let colors = CountryColors::new(colors);
        let mut pc = HashMap::new();
        pc.insert(1u32, "FRA".to_string()); // occupied
        pc.insert(2u32, "GER".to_string()); // owner == controller
        let lut = build_secondary_lut(3, &ids, &owners, &pc, &colors);
        // province 1: striped with FRA colour, alpha 255.
        assert_eq!(&lut[4..8], &[10, 20, 30, 255]);
        // province 2: controller == owner -> not striped.
        assert_eq!(lut[2 * 4 + 3], 0);
        // province 3: no controller entry -> not striped.
        assert_eq!(lut[3 * 4 + 3], 0);
        // province 0 (sea/unassigned) -> never striped.
        assert_eq!(lut[3], 0);
    }
}

// ---------------------------------------------------------------------------
// GPU pass
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MapUniforms {
    /// output target size in pixels.
    screen: [f32; 2],
    /// province-id raster size in texels.
    map_dims: [f32; 2],
    /// pan offset in map-UV (0..1).
    offset: [f32; 2],
    /// zoom scale (1 = full world). map_uv = out_uv/scale + offset.
    scale: [f32; 2],
    /// destination rect (x,y,w,h) in output pixels the map fills.
    dst: [f32; 4],
    /// fidelity flags: x = composite on (1)/off (0); y = tint strength (~0.55);
    /// z,w reserved. Off = the original flat opaque fill (the BEFORE behaviour).
    flags: [f32; 4],
}

/// The GPU political-map pass. Owns the integer id texture, the LUT texture (which
/// is re-uploaded per snapshot tick), the pipeline, and the bind group.
pub struct PoliticalMap {
    pub raster_width: u32,
    pub raster_height: u32,
    max_id: u32,

    // The id texture is never read after construction, but MUST stay owned: the
    // bind group holds a view into it, and dropping it would dangle that view.
    #[allow(dead_code)]
    id_tex: wgpu::Texture,
    lut_tex: wgpu::Texture,
    // --- fidelity layers (terrain underlay, river mask, per-province owner index) ---
    // All bound every draw (the shader always references bindings 3..5); for callers
    // that never enable fidelity these stay 1×1 placeholders and `flags.x = 0`.
    #[allow(dead_code)]
    terrain_tex: wgpu::Texture,
    #[allow(dead_code)]
    river_tex: wgpu::Texture,
    owner_tex: wgpu::Texture,
    #[allow(dead_code)]
    emissive_tex: wgpu::Texture,
    secondary_tex: wgpu::Texture,
    dynamic_lut_tex: wgpu::Texture,
    // FAITHFUL gradient-border field (binding 10): an Rgba8Unorm SDF height texture at
    // the raster resolution (R=country, G=province, B=state height; A=land mask), built
    // CPU-side by [`crate::borders::build_border_field`]. Replaces the old 4-neighbour
    // owner-edge darkening with the engine's gradient_border ramp. Starts as a 1×1 zero
    // placeholder (no borders) and is replaced by [`PoliticalMap::update_border_field`].
    border_tex: wgpu::Texture,
    dynamic_buf: wgpu::Buffer,
    // --- GROUP(1): per-pixel MATERIAL + grading textures (snow/mud/ice material, the
    // ColorCube LUT, the per-pixel height/normal gate) + a linear sampler. These drive the
    // FAITHFUL ApplySnow / GetMudColor / ApplyIce per-pixel material and the RestoreScene
    // color-grading postprocess. All real game assets ([`crate::material::Materials`]).
    #[allow(dead_code)]
    mat_textures: Vec<wgpu::Texture>,
    mat_bind_group: wgpu::BindGroup,
    mat_bind_layout: wgpu::BindGroupLayout,
    mat_sampler: wgpu::Sampler,
    /// the per-pixel gate texture (height/normal/land), recreated by `upload_fidelity`.
    gate_tex: wgpu::Texture,
    /// fidelity composite on/off and tint strength. Default off (flat fill).
    fidelity_on: bool,
    /// political TINT STRENGTH = 1 - MAP_MODE_TERRAIN_TRANSPARENCY (the game value).
    tint_strength: f32,
    /// night factor 0..1 (drives city emissive). Default 0 (day political map).
    night_factor: f32,
    /// naval-dominance enemy/friendly colours (from naval_dominance_fx.dds), injected into the
    /// dynamic uniform on every update. Default = the game's red/cyan.
    dom_enemy: [f32; 3],
    dom_friendly: [f32; 3],
    /// kept so `upload_fidelity` can rebuild the bind group after recreating the
    /// (resized) terrain/river textures.
    bind_layout: wgpu::BindGroupLayout,
    id_view: wgpu::TextureView,
    lut_view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms_buf: wgpu::Buffer,
}

impl PoliticalMap {
    /// Create the pass: upload the integer province-id raster (R32Uint) and an
    /// initial (all-neutral) LUT (Rgba8Unorm, 1-D). The LUT is replaced per tick
    /// via [`PoliticalMap::update_lut`].
    pub fn new(r: &Renderer, raster: &ProvinceRaster) -> PoliticalMap {
        let device = &r.device;
        let max_id = raster.max_id;

        // --- integer province-id texture (R32Uint, nearest/textureLoad, no mips,
        //     no sRGB). Carrying ids as integers is the only corruption-free path.
        let id_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("province-id-r32uint"),
            size: wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &id_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&raster.ids),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(raster.width * 4),
                rows_per_image: Some(raster.height),
            },
            wgpu::Extent3d {
                width: raster.width,
                height: raster.height,
                depth_or_array_layers: 1,
            },
        );
        let id_view = id_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // --- LUT texture: Rgba8Unorm, width = max_id+1, height 1, textureLoad.
        let lut_width = max_id + 1;
        let lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("province-colour-lut"),
            size: wgpu::Extent3d {
                width: lut_width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // initial neutral LUT so a render before update_lut is still honest.
        {
            let mut init = vec![0u8; (lut_width as usize) * 4];
            for i in 0..(lut_width as usize) {
                init[i * 4..i * 4 + 4].copy_from_slice(&NEUTRAL_TERRAIN);
            }
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &lut_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &init,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(lut_width * 4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: lut_width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        let lut_view = lut_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // --- fidelity placeholder textures (1×1) — replaced by upload_fidelity(). ---
        // terrain base + river mask: Rgba8Unorm; owner index: R32Uint, width max_id+1.
        let make_rgba_1x1 = |label: &str, rgba: [u8; 4]| -> wgpu::Texture {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            t
        };
        let terrain_tex = make_rgba_1x1("terrain-base-placeholder", NEUTRAL_TERRAIN);
        let river_tex = make_rgba_1x1("river-mask-placeholder", [0, 0, 0, 0]);
        let emissive_tex = make_rgba_1x1("emissive-mask-placeholder", [0, 0, 0, 0]);

        // owner index: R32Uint, width = max_id+1 (so the shader's textureLoad of a
        // neighbour province id is always in range). Placeholder = all OWNER_NONE.
        let owner_width = max_id + 1;
        let owner_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("owner-index-r32uint"),
            size: wgpu::Extent3d {
                width: owner_width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let zero = vec![OWNER_NONE; owner_width as usize];
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &owner_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&zero),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(owner_width * 4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: owner_width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        // secondary-colour LUT: 1-D Rgba8Unorm, width = max_id+1, height 1 (province-indexed
        // like the colour LUT). Placeholder = all-zero (no occupation stripes).
        let secondary_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("secondary-color-lut"),
            size: wgpu::Extent3d {
                width: owner_width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            let zero = vec![0u8; (owner_width as usize) * 4];
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &secondary_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &zero,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(owner_width * 4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: owner_width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        // per-province DYNAMIC LUT: 1-D Rgba8Unorm, width = max_id+1 (winter/mud/frozen).
        // Placeholder = all-zero (no snow/mud/ice = the neutral/static look).
        let dynamic_lut_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dynamic-province-lut"),
            size: wgpu::Extent3d {
                width: owner_width,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        {
            // default: no snow/mud/frozen (RGB 0), alpha 128 = NEUTRAL naval dominance (so
            // the unset default never reads as enemy/friendly dominance on the sea).
            let mut zero = vec![0u8; (owner_width as usize) * 4];
            for i in 0..(owner_width as usize) {
                zero[i * 4 + 3] = 128;
            }
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &dynamic_lut_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &zero,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(owner_width * 4),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: owner_width,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        // gradient-border field placeholder (1×1, all-zero = no borders). Replaced with a
        // full-raster Rgba8Unorm SDF by update_border_field(). It is RASTER-resolution
        // (not province-indexed), so the shader samples it at the same id-raster texel.
        let border_tex = make_rgba_1x1("border-field-placeholder", [0, 0, 0, 0]);

        let terrain_view = terrain_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let river_view = river_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let owner_view = owner_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let emissive_view = emissive_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let secondary_view = secondary_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let dynamic_lut_view = dynamic_lut_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let border_view = border_tex.create_view(&wgpu::TextureViewDescriptor::default());

        // DYNAMIC uniform buffer (neutral default = static/day/summer look).
        let dynamic_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map-dynamic-uniforms"),
            size: std::mem::size_of::<crate::dynamic::DynamicUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        {
            // neutral dynamic state (summer noon, no snow/mud): zeroed season=neutral.
            let neutral = crate::dynamic::DynamicUniforms {
                state: [0.0, 0.0, 0.0, 1.0], // night=0, fade=0, time=0, tone=1
                sun_dir: [0.0, 1.0, 0.0, 0.0],
                // zero sun vector = day/night DISABLED → full day everywhere (static default).
                hour_sun: [0.0, 0.0, 0.0, 0.0],
                // cb = neutral multiply (1,1,1), .w = full saturation (1.0 = no desaturate).
                cb_north: [1.0, 1.0, 1.0, 1.0],
                cb_center: [1.0, 1.0, 1.0, 1.0],
                cb_south: [1.0, 1.0, 1.0, 1.0],
                dom_enemy: [1.0, 0.04, 0.1, 0.0],
                dom_friendly: [0.0, 0.55, 1.0, 0.0],
            };
            r.queue
                .write_buffer(&dynamic_buf, 0, bytemuck::bytes_of(&neutral));
        }

        let uniforms_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("map-uniforms"),
            size: std::mem::size_of::<MapUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map-bind-layout"),
            entries: &[
                // uniforms
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // province-id texture (Uint, non-filterable)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // LUT texture (Unorm, read via textureLoad → declare non-filterable
                // float to avoid needing a sampler at all)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // terrain base (Rgba8Unorm float, non-filterable, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // river mask (Rgba8Unorm float, non-filterable, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // owner index (R32Uint, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // city-emissive mask (Rgba8Unorm float, non-filterable, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // secondary-colour LUT (occupation stripes; Rgba8Unorm, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // DYNAMIC uniform (night/season/sun/snow-mud-fade globals)
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // per-province DYNAMIC LUT (winter/mud/frozen; Rgba8Unorm, textureLoad)
                wgpu::BindGroupLayoutEntry {
                    binding: 9,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // gradient-border field (raster-res Rgba8Unorm SDF; R=country, G=province,
                // B=state height, A=land; textureLoad, non-filterable)
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map-bind"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&id_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&river_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&owner_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&emissive_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&secondary_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: dynamic_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&dynamic_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&border_view),
                },
            ],
        });

        // --- GROUP(1): material/grading textures + a linear sampler. 8 sampled textures
        // (snow, city+snow-noise, mud-diffuse, mud-normal, ice-diffuse, ice-noise, ColorCube,
        // gate) at bindings 0..8 and the sampler at binding 8. Placeholders here (1×1) are
        // replaced by [`PoliticalMap::upload_materials`] / `upload_fidelity` (gate).
        let mat_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("mat-sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Material group layout: 11 sampled textures (0-6 material/grading, 7 gate, 8-10 the
        // water-surface lean1/lean2/water-spec) + the sampler at binding 11.
        let mut mat_entries: Vec<wgpu::BindGroupLayoutEntry> = Vec::new();
        for b in 0u32..11 {
            mat_entries.push(wgpu::BindGroupLayoutEntry {
                binding: b,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        mat_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 11,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        });
        let mat_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("mat-bind-layout"),
            entries: &mat_entries,
        });
        // mat_textures holds bindings 0-6 (material/grading) + 8-10 (water surface) = 10
        // textures; the gate texture is binding 7 (held separately as gate_tex). Placeholders
        // (1×1, neutral) until upload_materials replaces them.
        let mut mat_textures: Vec<wgpu::Texture> = Vec::new();
        let mut mat_views: Vec<wgpu::TextureView> = Vec::new();
        for i in 0..10u32 {
            let px: [u8; 4] = match i {
                6 => [128, 128, 128, 255], // ColorCube placeholder (grey ~ identity-ish)
                _ => [128, 128, 255, 255],
            };
            let t = make_rgba_1x1("mat-placeholder", px);
            mat_views.push(t.create_view(&wgpu::TextureViewDescriptor::default()));
            mat_textures.push(t);
        }
        let gate_tex = make_rgba_1x1("gate-placeholder", [0, 128, 0, 255]);
        let gate_view0 = gate_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut mat_bg_entries: Vec<wgpu::BindGroupEntry> = Vec::new();
        // bindings 0-6 = mat_textures[0..7]; 7 = gate; 8-10 = mat_textures[7..10]; 11 = sampler.
        for i in 0..7usize {
            mat_bg_entries.push(wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(&mat_views[i]),
            });
        }
        mat_bg_entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: wgpu::BindingResource::TextureView(&gate_view0),
        });
        for i in 7..10usize {
            mat_bg_entries.push(wgpu::BindGroupEntry {
                binding: (i + 1) as u32,
                resource: wgpu::BindingResource::TextureView(&mat_views[i]),
            });
        }
        mat_bg_entries.push(wgpu::BindGroupEntry {
            binding: 11,
            resource: wgpu::BindingResource::Sampler(&mat_sampler),
        });
        let mat_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mat-bind"),
            layout: &mat_bind_layout,
            entries: &mat_bg_entries,
        });
        // (wgpu bind groups internally retain their resources, so `mat_sampler` may drop here.)

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("map-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("map.wgsl").into()),
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("map-pl"),
            bind_group_layouts: &[&bind_layout, &mat_bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("map-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        PoliticalMap {
            raster_width: raster.width,
            raster_height: raster.height,
            max_id,
            id_tex,
            lut_tex,
            terrain_tex,
            river_tex,
            owner_tex,
            emissive_tex,
            secondary_tex,
            dynamic_lut_tex,
            border_tex,
            dynamic_buf,
            mat_textures,
            mat_bind_group,
            mat_bind_layout,
            mat_sampler,
            gate_tex,
            fidelity_on: false,
            // 1 - MAP_MODE_TERRAIN_TRANSPARENCY (mod 0.8) = 0.2 (game political ratio).
            tint_strength: 1.0 - crate::terrain::gc::MAP_MODE_TERRAIN_TRANSPARENCY,
            night_factor: 0.0,
            dom_enemy: [1.0, 0.04, 0.1],
            dom_friendly: [0.0, 0.55, 1.0],
            bind_layout,
            id_view,
            lut_view,
            pipeline,
            bind_group,
            uniforms_buf,
        }
    }

    /// Upload the FIDELITY layers (terrain base + relief + river mask) built by
    /// [`crate::terrain::build_base`], replacing the 1×1 placeholders with full-res
    /// textures and turning the composite ON. The terrain/river rasters MUST be the
    /// same dimensions as the province-id raster (Codex B: one shared coordinate space).
    /// Rebuilds the bind group (the new textures differ in size from the placeholders).
    pub fn upload_fidelity(&mut self, r: &Renderer, base: &crate::terrain::TerrainBase) {
        let device = &r.device;
        let (w, h) = (base.width, base.height);
        debug_assert_eq!(base.base_rgba.len(), (w as usize) * (h as usize) * 4);

        let make = |label: &str, rgba: &[u8]| -> wgpu::Texture {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            t
        };
        debug_assert_eq!(base.emissive_rgba.len(), (w as usize) * (h as usize) * 4);
        debug_assert_eq!(base.gate_rgba.len(), (w as usize) * (h as usize) * 4);
        self.terrain_tex = make("terrain-base", &base.base_rgba);
        self.river_tex = make("river-mask", &base.river_rgba);
        self.emissive_tex = make("emissive-mask", &base.emissive_rgba);
        // the per-pixel GATE (height/normal/land) for the dynamic snow/mud/ice material.
        self.gate_tex = make("gate", &base.gate_rgba);
        self.fidelity_on = true;
        self.rebuild_bind_group(device);
        self.rebuild_mat_bind_group(device); // gate is in the material group
    }

    /// Upload the real per-pixel MATERIAL + grading textures ([`crate::material::Materials`])
    /// into the group(1) material bind group (snow / city+snow-noise / mud-diffuse /
    /// mud-normal / ice-diffuse / ice-noise / ColorCube). These drive the faithful per-pixel
    /// ApplySnow / GetMudColor / ApplyIce and the RestoreScene color-grading. Call once after
    /// `upload_fidelity` (the gate, material binding 7, comes from the terrain base).
    pub fn upload_materials(&mut self, r: &Renderer, mats: &crate::material::Materials) {
        let device = &r.device;
        let up = |img: &crate::material::Image, label: &str| -> wgpu::Texture {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: img.width,
                    height: img.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            r.queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &t,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &img.rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(img.width * 4),
                    rows_per_image: Some(img.height),
                },
                wgpu::Extent3d {
                    width: img.width,
                    height: img.height,
                    depth_or_array_layers: 1,
                },
            );
            t
        };
        // bindings 0..6 = the 7 material/grading textures (binding 7 = gate, set elsewhere).
        self.mat_textures[0] = up(&mats.snow, "snow");
        self.mat_textures[1] = up(&mats.city_snow_noise, "city-snow-noise");
        self.mat_textures[2] = up(&mats.mud_diffuse, "mud-diffuse");
        self.mat_textures[3] = up(&mats.mud_normal, "mud-normal");
        self.mat_textures[4] = up(&mats.ice_diffuse, "ice-diffuse");
        self.mat_textures[5] = up(&mats.ice_noise, "ice-noise");
        self.mat_textures[6] = up(&mats.color_cube, "color-cube");
        // water-surface textures (mat_textures[7..10] → bindings 8,9,10).
        self.mat_textures[7] = up(&mats.lean1, "lean1");
        self.mat_textures[8] = up(&mats.lean2, "lean2");
        self.mat_textures[9] = up(&mats.water_spec, "water-spec");
        // the REAL naval-dominance colours (injected into the dynamic uniform on each update).
        self.dom_enemy = mats.dominance_enemy;
        self.dom_friendly = mats.dominance_friendly;
        self.rebuild_mat_bind_group(device);
    }

    /// Rebuild the group(1) material bind group from the current material textures + gate.
    fn rebuild_mat_bind_group(&mut self, device: &wgpu::Device) {
        let views: Vec<wgpu::TextureView> = self
            .mat_textures
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let gate_view = self
            .gate_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut entries: Vec<wgpu::BindGroupEntry> = Vec::new();
        // bindings 0-6 = mat_textures[0..7]; 7 = gate; 8-10 = mat_textures[7..10] (water); 11 = sampler.
        for i in 0..7usize {
            entries.push(wgpu::BindGroupEntry {
                binding: i as u32,
                resource: wgpu::BindingResource::TextureView(&views[i]),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 7,
            resource: wgpu::BindingResource::TextureView(&gate_view),
        });
        for i in 7..10usize {
            entries.push(wgpu::BindGroupEntry {
                binding: (i + 1) as u32,
                resource: wgpu::BindingResource::TextureView(&views[i]),
            });
        }
        entries.push(wgpu::BindGroupEntry {
            binding: 11,
            resource: wgpu::BindingResource::Sampler(&self.mat_sampler),
        });
        self.mat_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("mat-bind"),
            layout: &self.mat_bind_layout,
            entries: &entries,
        });
    }

    /// Re-upload the per-province owner-index texture (for country borders) built by
    /// [`build_owner_index`]. `idx` length must be `max_id+1`.
    pub fn update_owner_index(&self, r: &Renderer, idx: &[u32]) {
        let owner_width = self.max_id + 1;
        debug_assert_eq!(idx.len(), owner_width as usize);
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.owner_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(idx),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(owner_width * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: owner_width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Toggle the fidelity composite (off = the original flat opaque fill).
    pub fn set_fidelity(&mut self, on: bool) {
        self.fidelity_on = on;
    }

    /// Upload the FAITHFUL gradient-border field (binding 10) built by
    /// [`crate::borders::build_border_field`]. The field is a full-raster Rgba8Unorm SDF
    /// (R=country, G=province, B=state height in the engine's alpha convention; A=land);
    /// the WGSL processes it through the ported `gradient_border_distance_to_alpha` /
    /// outline math, replacing the old 4-neighbour darkening. It depends only on the
    /// province raster + per-province owner-index (which change with control), so it is
    /// rebuilt CPU-side and re-uploaded whenever the owner-index changes (the same cadence
    /// as [`update_owner_index`]). Its dimensions MUST match the province-id raster.
    ///
    /// Recreating the texture (the placeholder is 1×1) rebuilds the bind group.
    pub fn update_border_field(&mut self, r: &Renderer, field: &crate::borders::BorderField) {
        debug_assert_eq!(field.width, self.raster_width);
        debug_assert_eq!(field.height, self.raster_height);
        debug_assert_eq!(
            field.rgba.len(),
            (field.width as usize) * (field.height as usize) * 4
        );
        let device = &r.device;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("border-field"),
            size: wgpu::Extent3d {
                width: field.width,
                height: field.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &field.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(field.width * 4),
                rows_per_image: Some(field.height),
            },
            wgpu::Extent3d {
                width: field.width,
                height: field.height,
                depth_or_array_layers: 1,
            },
        );
        self.border_tex = tex;
        self.rebuild_bind_group(device);
    }

    /// Rebuild the bind group after the terrain/river textures were recreated.
    fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        let terrain_view = self
            .terrain_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let river_view = self
            .river_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let owner_view = self
            .owner_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let emissive_view = self
            .emissive_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let secondary_view = self
            .secondary_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let dynamic_lut_view = self
            .dynamic_lut_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        let border_view = self
            .border_tex
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map-bind"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.id_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&terrain_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&river_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&owner_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&emissive_view),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&secondary_view),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: self.dynamic_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(&dynamic_lut_view),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: wgpu::BindingResource::TextureView(&border_view),
                },
            ],
        });
    }

    /// Re-upload the DYNAMIC state for a scrubber tick: the per-province winter/mud/frozen LUT
    /// (built by [`crate::dynamic::pack_province_dynamic_lut`]) and the global dynamic uniform
    /// (built by [`crate::dynamic::DynamicUniforms::from_inputs`]). Together they drive the
    /// snow/mud/ice/season/day-night branches so the map changes hour-by-hour from data.
    pub fn update_dynamic(
        &self,
        r: &Renderer,
        province_lut: &[u8],
        uniforms: &crate::dynamic::DynamicUniforms,
    ) {
        let width = self.max_id + 1;
        debug_assert_eq!(province_lut.len(), (width as usize) * 4);
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.dynamic_lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            province_lut,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        // inject the REAL naval-dominance colours (from naval_dominance_fx.dds) into the uniform.
        let mut u = *uniforms;
        u.dom_enemy = [self.dom_enemy[0], self.dom_enemy[1], self.dom_enemy[2], 0.0];
        u.dom_friendly = [
            self.dom_friendly[0],
            self.dom_friendly[1],
            self.dom_friendly[2],
            0.0,
        ];
        r.queue
            .write_buffer(&self.dynamic_buf, 0, bytemuck::bytes_of(&u));
    }

    /// Re-upload the SECONDARY-COLOUR LUT (occupation stripes) built by
    /// [`build_secondary_lut`]. `lut` length must be `(max_id+1)*4`.
    pub fn update_secondary_lut(&self, r: &Renderer, lut: &[u8]) {
        let width = self.max_id + 1;
        debug_assert_eq!(lut.len(), (width as usize) * 4);
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.secondary_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            lut,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Set the night factor (0 day .. 1 full night) — drives the city-emissive add.
    pub fn set_night_factor(&mut self, n: f32) {
        self.night_factor = n.clamp(0.0, 1.0);
    }

    /// Build the shader `flags` vec4 from the current state + the render zoom `scale`.
    /// flags.w (camera distance) maps zoom to the game's `cam_distance` 0..1: full-world
    /// (scale≈1) = 1 (far); deep zoom (large scale) trends toward 0 (near). This drives the
    /// border-width / detail ramps the game keys off `vCamPos.y`.
    fn flags_for(&self, scale: [f32; 2]) -> [f32; 4] {
        let zoom = scale[0].max(scale[1]).max(1.0);
        // 1 at zoom 1, ~0 by zoom ~8 (a smooth far->near ramp).
        let cam_dist = (1.0 / zoom).clamp(0.0, 1.0);
        [
            if self.fidelity_on { 1.0 } else { 0.0 },
            self.tint_strength,
            self.night_factor,
            cam_dist,
        ]
    }

    /// Re-upload the colour LUT for a new snapshot tick. `lut` must be exactly
    /// `(max_id+1)*4` bytes (built by [`build_lut`]).
    pub fn update_lut(&self, r: &Renderer, lut: &[u8]) {
        let lut_width = self.max_id + 1;
        debug_assert_eq!(lut.len(), (lut_width as usize) * 4);
        r.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.lut_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            lut,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(lut_width * 4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: lut_width,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Render the political map into the renderer's offscreen target, filling the
    /// destination rect `dst` (pixels). `offset`/`scale` give pan/zoom in map-UV.
    /// `clear` is the background. Returns the RGBA8 readback.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &self,
        r: &Renderer,
        dst: [f32; 4],
        offset: [f32; 2],
        scale: [f32; 2],
        clear: [f64; 4],
    ) -> Vec<u8> {
        let u = MapUniforms {
            screen: [r.width as f32, r.height as f32],
            map_dims: [self.raster_width as f32, self.raster_height as f32],
            offset,
            scale,
            dst,
            flags: self.flags_for(scale),
        };
        r.queue
            .write_buffer(&self.uniforms_buf, 0, bytemuck::bytes_of(&u));

        let view = r.offscreen_view();
        let mut enc = r
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("map-enc"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("map-rp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: clear[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.bind_group, &[]);
            rp.set_bind_group(1, &self.mat_bind_group, &[]);
            // full-screen triangle (3 verts, generated in the vertex shader).
            rp.draw(0..3, 0..1);
        }
        r.queue.submit(Some(enc.finish()));
        r.readback_public()
    }

    /// LIVE-PATH map render: identical to [`render`](Self::render) but submits
    /// WITHOUT the CPU readback — the map clears + fills the shared offscreen target,
    /// the UI then composites over it, and the app presents the result to the surface
    /// (Codex §2: no per-frame readback). Targets the renderer's offscreen view so a
    /// single offscreen texture is the compositor.
    pub fn render_to_offscreen(
        &self,
        r: &Renderer,
        dst: [f32; 4],
        offset: [f32; 2],
        scale: [f32; 2],
        clear: [f64; 4],
    ) {
        let u = MapUniforms {
            screen: [r.width as f32, r.height as f32],
            map_dims: [self.raster_width as f32, self.raster_height as f32],
            offset,
            scale,
            dst,
            flags: self.flags_for(scale),
        };
        r.queue
            .write_buffer(&self.uniforms_buf, 0, bytemuck::bytes_of(&u));

        let view = r.offscreen_view();
        let mut enc = r
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("map-enc-live"),
            });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("map-rp-live"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0],
                            g: clear[1],
                            b: clear[2],
                            a: clear[3],
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.bind_group, &[]);
            rp.set_bind_group(1, &self.mat_bind_group, &[]);
            rp.draw(0..3, 0..1);
        }
        r.queue.submit(Some(enc.finish()));
    }
}
