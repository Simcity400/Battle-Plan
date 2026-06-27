//! Loader for the `map_pipeline/assets/*` artifacts the M4 GPU map will consume:
//! the province-id raster meta, province->state map, state meta, and country
//! colours. M1 only needs this to COMPILE and round-trip the JSON; the GPU map
//! shader itself is M4.
//!
//! All loaders tolerate missing files (return `None`/`Err`) so M1 builds even on
//! a machine where the map pipeline hasn't been run yet.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// `province_ids_meta.json` — how the province-id raster encodes ids into RGB.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProvinceIdMeta {
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// e.g. "rgb24" / "r8g8b8" — how id = f(r,g,b).
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub max_province_id: Option<u32>,
}

/// `country_colors.json` — TAG -> [r,g,b] (0..255). Tolerant of extra fields.
#[derive(Debug, Clone, Default)]
pub struct CountryColors {
    pub colors: HashMap<String, [u8; 3]>,
}

#[derive(Debug, Clone, Default)]
pub struct MapArtifacts {
    pub province_id_meta: Option<ProvinceIdMeta>,
    /// province id -> state id
    pub province_to_state: HashMap<u32, u32>,
    pub country_colors: CountryColors,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

impl MapArtifacts {
    /// Load whatever artifacts exist under `dir` (typically
    /// `map_pipeline/assets`). Missing pieces are simply left empty.
    pub fn load(dir: &Path) -> MapArtifacts {
        let province_id_meta = read_json(&dir.join("province_ids_meta.json"));

        // province_to_state.json: object of "provId" -> stateId (string or num).
        let mut province_to_state = HashMap::new();
        if let Some(raw) =
            read_json::<HashMap<String, serde_json::Value>>(&dir.join("province_to_state.json"))
        {
            for (k, v) in raw {
                if let (Ok(pid), Some(sid)) = (k.parse::<u32>(), v.as_u64()) {
                    province_to_state.insert(pid, sid as u32);
                }
            }
        }

        // country_colors.json: TAG -> [r,g,b] or {r,g,b}.
        let mut colors = HashMap::new();
        if let Some(raw) =
            read_json::<HashMap<String, serde_json::Value>>(&dir.join("country_colors.json"))
        {
            for (tag, v) in raw {
                if let Some(arr) = v.as_array() {
                    if arr.len() >= 3 {
                        let c = |i: usize| arr[i].as_u64().unwrap_or(128) as u8;
                        colors.insert(tag, [c(0), c(1), c(2)]);
                    }
                }
            }
        }

        MapArtifacts {
            province_id_meta,
            province_to_state,
            country_colors: CountryColors { colors },
        }
    }
}
