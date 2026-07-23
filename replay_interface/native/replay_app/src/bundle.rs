//! Map bundle loading for the replay product surface.
//!
//! Loads the gitignored Rust map bundle documented in
//! `replay_interface/documents/MAP_REPLICA_PLAN.md` ("Phase 0: Rust Map
//! Bundle"): `manifest.json`, `province_ids.r32` (little-endian u32 per
//! pixel, row-major), plus the political JSON tables. Fails closed with
//! generation guidance when the bundle is missing or malformed; nothing is
//! fabricated.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::Result;

/// Bundle dimensions required by the MAP_REPLICA_PLAN Phase 0 acceptance
/// gate for real HOI4-generated bundles.
pub const PRODUCT_MAP_WIDTH: usize = 5632;
/// See [`PRODUCT_MAP_WIDTH`].
pub const PRODUCT_MAP_HEIGHT: usize = 2048;

const BUNDLE_SCHEMA: &str = "battle-plan-rust-map-bundle-v1";

/// The command a user runs to (re)generate the map bundle.
pub fn generation_guidance() -> String {
    "generate the map bundle with:\n  cargo run --release -p map_viewer -- bundle --game \"<the game install dir>\"".to_string()
}

/// In-memory map bundle: the static, grounded inputs of the replay map.
pub struct MapBundle {
    pub width: usize,
    pub height: usize,
    /// Little-endian u32 province id per pixel, row-major. CPU picking
    /// authority; also uploaded once as an `R32_UINT` texture.
    pub province_ids: Vec<u32>,
    /// Province id -> state id (`province_to_state.json`).
    pub province_to_state: BTreeMap<u32, u32>,
    /// State id -> 1936 owner tag (`state_owner_1936.json`).
    pub state_owner_1936: BTreeMap<u32, String>,
    /// Country tag -> RGB color (`country_colors.json`).
    pub country_colors: BTreeMap<String, [u8; 3]>,
}

impl std::fmt::Debug for MapBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Compact by hand: the pixel planes are millions of entries.
        formatter
            .debug_struct("MapBundle")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("province_ids_len", &self.province_ids.len())
            .field("province_to_state_len", &self.province_to_state.len())
            .field("state_owner_1936_len", &self.state_owner_1936.len())
            .field("country_colors_len", &self.country_colors.len())
            .finish()
    }
}

impl MapBundle {
    /// Load a bundle directory, validating file shapes defensively.
    pub fn load(dir: &Path) -> Result<MapBundle> {
        if !dir.is_dir() {
            return Err(format!("bundle directory {} does not exist", dir.display()).into());
        }
        let manifest_path = dir.join("manifest.json");
        let manifest_bytes = fs::read(&manifest_path).map_err(|err| {
            format!(
                "missing or unreadable manifest.json at {}: {}",
                manifest_path.display(),
                err
            )
        })?;
        let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes)
            .map_err(|err| format!("manifest.json is not valid JSON: {err}"))?;

        let schema = manifest
            .get("schema")
            .and_then(serde_json::Value::as_str)
            .ok_or("manifest.json has no \"schema\" string")?;
        if schema != BUNDLE_SCHEMA {
            return Err(format!(
                "unsupported map bundle schema: {schema} (expected {BUNDLE_SCHEMA})"
            )
            .into());
        }

        let dimensions = manifest
            .get("dimensions")
            .ok_or("manifest.json has no \"dimensions\" object")?;
        let width = json_usize(dimensions, "width", "manifest.json dimensions")?;
        let height = json_usize(dimensions, "height", "manifest.json dimensions")?;
        if width == 0 || height == 0 {
            return Err(format!("manifest.json declares empty dimensions {width}x{height}").into());
        }
        let pixel_count = width
            .checked_mul(height)
            .ok_or("manifest.json dimensions overflow")?;

        let province_ids_name =
            manifest_file_name(&manifest, "province_ids", "province_ids.r32")?;
        let province_to_state_name =
            manifest_file_name(&manifest, "province_to_state", "province_to_state.json")?;
        let state_owner_name =
            manifest_file_name(&manifest, "state_owner_1936", "state_owner_1936.json")?;
        let country_colors_name =
            manifest_file_name(&manifest, "country_colors", "country_colors.json")?;

        let province_ids = read_u32_raw(&dir.join(&province_ids_name), pixel_count)?;
        let province_to_state = read_u32_key_u32_map(&dir.join(&province_to_state_name))?;
        let state_owner_1936 = read_u32_key_string_map(&dir.join(&state_owner_name))?;
        let country_colors = read_country_colors(&dir.join(&country_colors_name))?;

        Ok(MapBundle {
            width,
            height,
            province_ids,
            province_to_state,
            state_owner_1936,
            country_colors,
        })
    }

    /// MAP_REPLICA_PLAN Phase 0 gate: real the game bundles are 5632x2048.
    ///
    /// Country identity (tags/names/colors/cosmetics) is fully data-driven and
    /// works for ANY modset, but the render pipeline still assumes the vanilla
    /// MAP GEOMETRY size (the east-west wrap period in
    /// [`crate::render`], the political-colour texture resolution in
    /// `map_viewer::political_color_synthesis`, plus the captured-camera laws
    /// are all tuned to 5632x2048). The overwhelming majority of mods keep the
    /// vanilla map, so this holds; a total-conversion with a RESIZED map is
    /// rejected here — fail-closed — rather than silently mis-rendered.
    /// Lifting the assumption needs a differently-sized reference map to verify
    /// the wrap/camera/colour behaviour without guessing (tracked in
    /// `documents/EXTRACTION_NEEDS.md` §C, row C17).
    pub fn validate_product_dimensions(&self) -> Result<()> {
        if self.width != PRODUCT_MAP_WIDTH || self.height != PRODUCT_MAP_HEIGHT {
            return Err(format!(
                "map bundle dimensions {}x{} do not match the vanilla the game map ({}x{}); \
                 country identity is modset-generic, but this build assumes the vanilla map \
                 SIZE (wrap/camera/political-colour laws are tuned to 5632x2048). A mod that \
                 keeps the vanilla map works; a mod with a RESIZED map is not yet supported and \
                 is rejected here rather than mis-rendered",
                self.width, self.height, PRODUCT_MAP_WIDTH, PRODUCT_MAP_HEIGHT
            )
            .into());
        }
        Ok(())
    }

    /// Largest state id referenced by the static tables (0 when none).
    pub fn max_state_id(&self) -> u32 {
        let from_mapping = self.province_to_state.values().copied().max().unwrap_or(0);
        let from_owners = self
            .state_owner_1936
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0);
        from_mapping.max(from_owners)
    }
}

fn manifest_file_name(manifest: &serde_json::Value, key: &str, default: &str) -> Result<String> {
    let Some(files) = manifest.get("files") else {
        return Ok(default.to_string());
    };
    let files = files
        .as_object()
        .ok_or("manifest.json \"files\" is present but is not an object")?;
    let Some(value) = files.get(key) else {
        return Ok(default.to_string());
    };
    let name = value.as_str().ok_or_else(|| {
        format!("manifest.json files.{key} is present but is not a string")
    })?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).is_absolute()
    {
        return Err(format!(
            "manifest.json files.{key} must be a non-empty bundle-local file name, found {name:?}"
        )
        .into());
    }
    Ok(name.to_string())
}

fn json_usize(value: &serde_json::Value, key: &str, context: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|number| number as usize)
        .ok_or_else(|| format!("{context} has no unsigned \"{key}\"").into())
}

fn read_u32_raw(path: &Path, expected_len: usize) -> Result<Vec<u32>> {
    let bytes = fs::read(path)
        .map_err(|err| format!("missing or unreadable {}: {}", path.display(), err))?;
    let expected_bytes = expected_len
        .checked_mul(4)
        .ok_or("raw plane byte length overflow")?;
    if bytes.len() != expected_bytes {
        return Err(format!(
            "{} has {} bytes, expected {} ({} little-endian u32 pixels)",
            path.display(),
            bytes.len(),
            expected_bytes,
            expected_len
        )
        .into());
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn read_json_object(path: &Path) -> Result<serde_json::Map<String, serde_json::Value>> {
    let bytes = fs::read(path)
        .map_err(|err| format!("missing or unreadable {}: {}", path.display(), err))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|err| format!("{} is not valid JSON: {}", path.display(), err))?;
    match value {
        serde_json::Value::Object(map) => Ok(map),
        other => Err(format!(
            "{} is not a JSON object map (found {})",
            path.display(),
            json_type_name(&other)
        )
        .into()),
    }
}

fn parse_u32_key(key: &str, path: &Path) -> Result<u32> {
    key.parse::<u32>()
        .map_err(|_| format!("{} has non-u32 key {key:?}", path.display()).into())
}

fn read_u32_key_u32_map(path: &Path) -> Result<BTreeMap<u32, u32>> {
    let object = read_json_object(path)?;
    let mut map = BTreeMap::new();
    for (key, value) in &object {
        let id = parse_u32_key(key, path)?;
        let number = value
            .as_u64()
            .filter(|n| *n <= u64::from(u32::MAX))
            .ok_or_else(|| {
                format!(
                    "{} entry {key:?} is not an unsigned 32-bit integer (found {})",
                    path.display(),
                    json_type_name(value)
                )
            })?;
        map.insert(id, number as u32);
    }
    Ok(map)
}

fn read_u32_key_string_map(path: &Path) -> Result<BTreeMap<u32, String>> {
    let object = read_json_object(path)?;
    let mut map = BTreeMap::new();
    for (key, value) in &object {
        let id = parse_u32_key(key, path)?;
        let tag = value.as_str().ok_or_else(|| {
            format!(
                "{} entry {key:?} is not a string tag (found {})",
                path.display(),
                json_type_name(value)
            )
        })?;
        map.insert(id, tag.to_string());
    }
    Ok(map)
}

fn read_country_colors(path: &Path) -> Result<BTreeMap<String, [u8; 3]>> {
    let object = read_json_object(path)?;
    let mut map = BTreeMap::new();
    for (tag, value) in &object {
        let components = value.as_array().ok_or_else(|| {
            format!(
                "{} entry {tag:?} is not an RGB array (found {})",
                path.display(),
                json_type_name(value)
            )
        })?;
        if components.len() != 3 {
            return Err(format!(
                "{} entry {tag:?} has {} components, expected 3 (RGB)",
                path.display(),
                components.len()
            )
            .into());
        }
        let mut rgb = [0u8; 3];
        for (index, component) in components.iter().enumerate() {
            let channel = component.as_u64().filter(|n| *n <= 255).ok_or_else(|| {
                format!(
                    "{} entry {tag:?} component {index} is not a 0-255 integer",
                    path.display()
                )
            })?;
            rgb[index] = channel as u8;
        }
        map.insert(tag.clone(), rgb);
    }
    Ok(map)
}

fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

    /// Unique per-test directory under the system temp dir.
    pub(crate) fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let count = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "replay_app_test_{tag}_{}_{nanos}_{count}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp fixture dir");
        dir
    }

    fn write_u32_plane(path: &Path, values: &[u32]) {
        let mut bytes = Vec::with_capacity(values.len() * 4);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        fs::write(path, bytes).expect("write raw plane");
    }

    /// Write a tiny synthetic 8x4 bundle. Provinces 1..=4 fill the four
    /// horizontal quarters of every row.
    pub(crate) fn write_fixture_bundle(dir: &Path) {
        let width = 8usize;
        let height = 4usize;
        let mut province_ids = Vec::with_capacity(width * height);
        for _y in 0..height {
            for x in 0..width {
                province_ids.push((x / 2 + 1) as u32);
            }
        }
        write_u32_plane(&dir.join("province_ids.r32"), &province_ids);
        let manifest = serde_json::json!({
            "schema": BUNDLE_SCHEMA,
            "dimensions": { "width": width, "height": height },
            "province_id_encoding": "little-endian u32 per pixel",
        });
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
        fs::write(
            dir.join("province_to_state.json"),
            br#"{"1": 10, "2": 10, "3": 20, "4": 30}"#,
        )
        .expect("write province_to_state");
        fs::write(
            dir.join("state_owner_1936.json"),
            br#"{"10": "FRA", "20": "GER"}"#,
        )
        .expect("write state_owner_1936");
        fs::write(
            dir.join("country_colors.json"),
            br#"{"AAA": [1, 2, 3], "FRA": [10, 20, 30], "GER": [40, 50, 60]}"#,
        )
        .expect("write country_colors");
    }

    #[test]
    fn load_reads_all_planes_and_tables() {
        let dir = unique_temp_dir("load_ok");
        write_fixture_bundle(&dir);
        let bundle = MapBundle::load(&dir).expect("fixture bundle loads");
        assert_eq!(bundle.width, 8);
        assert_eq!(bundle.height, 4);
        assert_eq!(bundle.province_ids.len(), 32);
        assert_eq!(bundle.province_ids[0], 1);
        assert_eq!(bundle.province_ids[7], 4);
        assert_eq!(bundle.province_to_state.get(&1), Some(&10));
        assert_eq!(bundle.province_to_state.get(&4), Some(&30));
        assert_eq!(
            bundle.state_owner_1936.get(&10).map(String::as_str),
            Some("FRA")
        );
        assert_eq!(bundle.country_colors.get("GER"), Some(&[40, 50, 60]));
        assert_eq!(bundle.max_state_id(), 30);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stale_map_visual_artifact_is_ignored() {
        let dir = unique_temp_dir("stale_map_visual");
        write_fixture_bundle(&dir);
        fs::write(dir.join("map_visual.rgbx"), b"removed cpu scaffold")
            .expect("write stale visual artifact");
        let bundle = MapBundle::load(&dir).expect("fixture bundle still loads");
        assert_eq!(bundle.width, 8);
        assert_eq!(bundle.height, 4);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_bundle_directory_fails_closed() {
        let dir = unique_temp_dir("missing_dir").join("does_not_exist");
        let err = MapBundle::load(&dir).expect_err("missing dir must fail");
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[test]
    fn missing_manifest_fails_closed() {
        let dir = unique_temp_dir("missing_manifest");
        let err = MapBundle::load(&dir).expect_err("missing manifest must fail");
        assert!(err.to_string().contains("manifest.json"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn wrong_schema_fails_closed() {
        let dir = unique_temp_dir("wrong_schema");
        write_fixture_bundle(&dir);
        fs::write(
            dir.join("manifest.json"),
            br#"{"schema": "other-schema", "dimensions": {"width": 8, "height": 4}}"#,
        )
        .expect("overwrite manifest");
        let err = MapBundle::load(&dir).expect_err("wrong schema must fail");
        assert!(
            err.to_string().contains("unsupported map bundle schema"),
            "{err}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_present_file_selectors_never_degrade_to_default_names() {
        let dir = unique_temp_dir("malformed_file_selectors");
        write_fixture_bundle(&dir);
        fs::write(
            dir.join("manifest.json"),
            br#"{"schema":"battle-plan-rust-map-bundle-v1","dimensions":{"width":8,"height":4},"files":{"province_ids":17}}"#,
        )
        .expect("overwrite manifest");

        let error = MapBundle::load(&dir)
            .expect_err("present wrong-typed selector must not use province_ids.r32");
        assert!(
            error
                .to_string()
                .contains("files.province_ids is present but is not a string"),
            "{error}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn bundle_file_selectors_cannot_escape_the_bundle_directory() {
        let dir = unique_temp_dir("unsafe_file_selector");
        write_fixture_bundle(&dir);
        fs::write(
            dir.join("manifest.json"),
            br#"{"schema":"battle-plan-rust-map-bundle-v1","dimensions":{"width":8,"height":4},"files":{"country_colors":"../country_colors.json"}}"#,
        )
        .expect("overwrite manifest");

        let error = MapBundle::load(&dir)
            .expect_err("bundle selector must not escape the admitted bundle directory");
        assert!(
            error.to_string().contains("bundle-local file name"),
            "{error}"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn truncated_province_ids_fails_closed() {
        let dir = unique_temp_dir("truncated_ids");
        write_fixture_bundle(&dir);
        fs::write(dir.join("province_ids.r32"), [0u8; 12]).expect("truncate plane");
        let err = MapBundle::load(&dir).expect_err("truncated plane must fail");
        let message = err.to_string();
        assert!(message.contains("province_ids.r32"), "{message}");
        assert!(message.contains("expected 128"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_country_colors_fails_closed() {
        let dir = unique_temp_dir("bad_colors");
        write_fixture_bundle(&dir);
        fs::write(dir.join("country_colors.json"), br#"["FRA", "GER"]"#)
            .expect("overwrite country colors");
        let err = MapBundle::load(&dir).expect_err("malformed colors must fail");
        let message = err.to_string();
        assert!(message.contains("country_colors.json"), "{message}");
        assert!(message.contains("not a JSON object map"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn non_integer_state_mapping_fails_closed() {
        let dir = unique_temp_dir("bad_mapping");
        write_fixture_bundle(&dir);
        fs::write(dir.join("province_to_state.json"), br#"{"1": "ten"}"#)
            .expect("overwrite mapping");
        let err = MapBundle::load(&dir).expect_err("malformed mapping must fail");
        assert!(err.to_string().contains("province_to_state.json"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn product_dimension_gate_rejects_fixture_size() {
        let dir = unique_temp_dir("dims_gate");
        write_fixture_bundle(&dir);
        let bundle = MapBundle::load(&dir).expect("fixture bundle loads");
        let err = bundle
            .validate_product_dimensions()
            .expect_err("8x4 must fail the product gate");
        assert!(err.to_string().contains("5632x2048"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }
}
