//! Map bundle loading for the replay product surface.
//!
//! Loads the gitignored Rust map bundle documented in
//! Bundle"): `manifest.json`, `province_ids.r32` (little-endian u32 per
//! pixel, row-major), plus the political JSON tables. Fails closed with
//! generation guidance when the bundle is missing or malformed; nothing is
//! fabricated.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::Result;

/// gate for real HOI4-generated bundles.
pub const PRODUCT_MAP_WIDTH: usize = 5632;
/// See [`PRODUCT_MAP_WIDTH`].
pub const PRODUCT_MAP_HEIGHT: usize = 2048;

const BUNDLE_SCHEMA: &str = "battle-plan-rust-map-bundle-v1";

/// The command a user runs to (re)generate the map bundle.
pub fn generation_guidance() -> String {
    "generate the map bundle with:\n  cargo run --release -p map_viewer -- bundle --game \"<the game install dir>\"".to_string()
}

/// The capture-side provenance a bundle manifest must declare (FW7 H1):
/// which game sources produced it (fnv1a64 identity hashes written by
/// `map_viewer` at generation time) and which modset was in effect. Verified
/// at load so a bundle with stripped or mangled provenance fails closed, and
/// surfaced so consumers can compare the modset identity against the
/// recording's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleProvenance {
    /// Directory name of the game install the bundle was generated from.
    pub game_dir_name: String,
    /// The `default.map` reference recorded by the generator.
    pub default_map: String,
    /// fnv1a64 of `map/definition.csv` at generation time (16 lowercase hex).
    pub definition_csv_fnv1a64: String,
    /// fnv1a64 of `map/provinces.bmp` at generation time (16 lowercase hex).
    pub provinces_bmp_fnv1a64: String,
    /// fnv1a64 per declared visual source input (path -> 16 lowercase hex).
    pub visual_input_fnv1a64: BTreeMap<String, String>,
    /// The capture-side canonical modset identity: `"vanilla"` for a no-mod
    /// bundle (including a manifest with no `modset` block), otherwise the
    /// 16-hex id derived from the resolved mod selection.
    pub modset_id: String,
    /// Number of mods in the resolved selection (0 for vanilla).
    pub mod_count: usize,
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
    /// Verified capture-side provenance (source hashes + modset identity).
    pub provenance: BundleProvenance,
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
            .field("provenance", &self.provenance)
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

        let provenance = verified_provenance(&manifest)?;

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
            provenance,
        })
    }

    ///
    /// Country identity (tags/names/colors/cosmetics) is fully data-driven and
    /// works for ANY modset, but the render pipeline still assumes the vanilla
    /// MAP GEOMETRY size (the east-west wrap period in
    /// [`crate::render`], the political-colour texture resolution in
    /// `map_viewer::political_color_synthesis`, plus the captured-camera laws
    /// are all tuned to 5632x2048). The overwhelming majority of mods keep the
    /// vanilla map, so this holds; a total-conversion with a RESIZED map is
    /// rejected here — fail-closed — rather than silently mis-rendered.
    /// Lifting the assumption requires exact differently-sized map evidence for
    /// wrap, camera, and colour behavior; it must not be guessed.
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

/// What reconciling a bundle's declared modset against a recording's own
/// content manifest could actually establish.
///
/// The two identities are NOT mutually recomputable, and this type exists so
/// that fact is recorded rather than papered over. The bundle's `modset_id` is
/// `sha1_hex_16` over `[["workshop:<id>"|"name:<name>", "<version>"], ...]`
/// (`map_viewer::mod_resolver::modset_id`), while a recorded
/// [`replay_core::RecordedContentLayer`] carries a normalized descriptor
/// `layer_id`, a `descriptor_sha256` and a `source_container_sha256` — but
/// neither a mod version nor a workshop id. So exact identity equality between
/// a modded bundle and a modded recording cannot be decided from the recorded
/// evidence, and is never asserted.
///
/// What IS decidable on both sides, and cannot false-positive, is any
/// DISPROOF of sameness. Two facts carry it: the vanilla/modded CLASS — the
/// bundle publishes `mod_count == 0` exactly when its selection was empty
/// (`modset_id(&[]) == "vanilla"`), and the recording declares its mod layers
/// explicitly as [`replay_core::ContentLayerKind::Mod`] rows — and the ROSTER
/// SIZE, since the bundle verifies `mod_count == mods.len()` at load while the
/// recorder verifies its Mod-layer count equals the launch-bound mod roster.
/// Equality of identities cannot be proven, but two different roster sizes
/// cannot be the same roster. Either disagreement is returned as
/// [`replay_core::SourceError::ModsetMismatch`] by
/// [`reconcile_bundle_modset`] — the same typed class the snapshot container
/// and the rollover authenticator already funnel into, which
/// `validate::rejection_kind` maps to `RejectionKind::Modset`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleModsetAgreement {
    /// Both sides declare the vanilla identity: the bundle resolved an empty
    /// selection and the recording admitted no mod layer. Fully decided.
    AgreedVanilla,
    /// Both sides declare a modded selection of the SAME size, so nothing in
    /// the recorded evidence contradicts them, but the exact identities are
    /// not comparable (see the type docs). Recorded as a degradation; never
    /// reported as agreement. A size DIFFERENCE is not this variant — it is a
    /// decidable disproof and is refused.
    ModdedIdentityNotComparable {
        bundle_modset_id: String,
        bundle_mod_count: usize,
        recording_mod_layer_count: usize,
    },
    /// The recording carries no portable content manifest on this route, so it
    /// declares no modset identity at all and nothing can be compared.
    RecordingIdentityAbsent { reason: String },
}

impl BundleModsetAgreement {
    /// A one-line diagnostic for the launch log. Every variant renders; a
    /// non-decidable outcome must never pass silently.
    pub fn diagnostic(&self) -> String {
        match self {
            Self::AgreedVanilla => {
                "bundle/recording modset: agreed (both vanilla; no mod layers on either side)"
                    .to_string()
            }
            Self::ModdedIdentityNotComparable {
                bundle_modset_id,
                bundle_mod_count,
                recording_mod_layer_count,
            } => format!(
                "bundle/recording modset: NOT COMPARABLE — bundle declares modset {bundle_modset_id} \
                 ({bundle_mod_count} mod(s)) and the recording admitted {recording_mod_layer_count} \
                 mod layer(s), but a recorded layer carries no mod version or workshop id, so the \
                 bundle's modset digest cannot be recomputed from the recording. The modded/vanilla \
                 class agrees; exact identity is undecided and is NOT being treated as a match"
            ),
            Self::RecordingIdentityAbsent { reason } => format!(
                "bundle/recording modset: NOT COMPARABLE — the recording declares no modset \
                 identity ({reason})"
            ),
        }
    }

    /// True only for the one outcome that positively established agreement.
    pub fn is_decided(&self) -> bool {
        matches!(self, Self::AgreedVanilla)
    }
}

/// Reconcile a loaded bundle's declared modset against the recording's own
/// content manifest at open.
///
/// This closes the FW7/FW11 seam: the bundle's province->state table becomes
/// the anomaly authority for a recording (`AnomalyPolicy::with_known_provinces`
/// in `timeline::build_provider`), so a bundle generated under a different
/// modset silently mis-judges which province ids are real. A definite class
/// disagreement is a typed refusal; anything the recorded evidence cannot
/// decide is a typed recorded degradation. Neither outcome is silence.
pub fn reconcile_bundle_modset(
    provenance: &BundleProvenance,
    manifest: Option<&replay_core::ContentManifest>,
) -> std::result::Result<BundleModsetAgreement, replay_core::SourceError> {
    let Some(manifest) = manifest else {
        return Ok(BundleModsetAgreement::RecordingIdentityAbsent {
            reason: "the opened recording exposes no portable content manifest".to_string(),
        });
    };
    let recording_mod_layer_count = manifest
        .ordered_layers
        .iter()
        .filter(|layer| layer.layer_kind == replay_core::ContentLayerKind::Mod)
        .count();
    // The capture-side biconditional `modset_id(&[]) == "vanilla"` is already
    // enforced at load by `verified_provenance`, so `mod_count == 0` and the
    // literal vanilla id are the same fact here; read the count.
    let bundle_is_vanilla = provenance.mod_count == 0;
    let recording_is_vanilla = recording_mod_layer_count == 0;
    match (bundle_is_vanilla, recording_is_vanilla) {
        (true, true) => Ok(BundleModsetAgreement::AgreedVanilla),
        // Both modded. The missing version/workshop id blocks PROVING the two
        // rosters are the same, but it does not block DISPROVING it: the
        // bundle verifies `mod_count == mods.len()` at load, and the recorder
        // verifies its Mod-layer count equals the launch-bound mod roster, so
        // both counts are exact roster sizes. Different sizes cannot be the
        // same roster.
        (false, false) if provenance.mod_count != recording_mod_layer_count => {
            Err(replay_core::SourceError::ModsetMismatch(format!(
                "map bundle declares modset {} with {} mod(s) but the recording admitted {} \
                 mod layer(s); both counts are exact roster sizes, so the bundle was generated \
                 from a different mod selection than the recording and must not be used as the \
                 anomaly/province authority for it — {}",
                provenance.modset_id,
                provenance.mod_count,
                recording_mod_layer_count,
                generation_guidance()
            )))
        }
        (false, false) => Ok(BundleModsetAgreement::ModdedIdentityNotComparable {
            bundle_modset_id: provenance.modset_id.clone(),
            bundle_mod_count: provenance.mod_count,
            recording_mod_layer_count,
        }),
        (true, false) => Err(replay_core::SourceError::ModsetMismatch(format!(
            "map bundle declares the vanilla modset (0 mods) but the recording admitted \
             {recording_mod_layer_count} mod layer(s); the bundle's province tables were \
             generated from different content than the recording and must not be used as \
             the anomaly/province authority for it — {}",
            generation_guidance()
        ))),
        (false, true) => Err(replay_core::SourceError::ModsetMismatch(format!(
            "map bundle declares modset {} ({} mod(s)) but the recording admitted no mod \
             layer at all; the bundle's province tables were generated from different \
             content than the recording and must not be used as the anomaly/province \
             authority for it — {}",
            provenance.modset_id,
            provenance.mod_count,
            generation_guidance()
        ))),
    }
}

/// A generator-written fnv1a64 identity: exactly 16 lowercase hex digits
/// (`map_viewer::util::fnv1a64_file` formats with `{hash:016x}`).
fn is_fnv1a64_hex(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn provenance_str(source: &serde_json::Value, key: &str) -> Result<String> {
    let value = source
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("manifest.json source.{key} is missing or not a string"))?;
    if value.is_empty() {
        return Err(format!("manifest.json source.{key} is empty").into());
    }
    Ok(value.to_string())
}

fn provenance_hash(source: &serde_json::Value, key: &str) -> Result<String> {
    let value = provenance_str(source, key)?;
    if !is_fnv1a64_hex(&value) {
        return Err(format!(
            "manifest.json source.{key} is not a 16-hex fnv1a64 identity, found {value:?}"
        )
        .into());
    }
    Ok(value)
}

/// FW7 H1 — verify the capture-side provenance contract at load. The
/// generator (`map_viewer` bundle_gen) always writes a `source` block with
/// the game-input fnv1a64 identities, and a `modset` block whenever a mod
/// selection was resolved; a manifest with that provenance stripped or
/// mangled is not a trustworthy bundle and fails closed with regeneration
/// guidance. An absent `modset` block is the vanilla identity by the
/// capture-side rule (`modset_id(&[]) == "vanilla"`).
fn verified_provenance(manifest: &serde_json::Value) -> Result<BundleProvenance> {
    let source = manifest.get("source").ok_or_else(|| {
        format!(
            "manifest.json has no \"source\" provenance block (capture-side contract); \
             the bundle predates or lost its source-hash provenance — {}",
            generation_guidance()
        )
    })?;
    if !source.is_object() {
        return Err("manifest.json \"source\" is present but is not an object".into());
    }
    let game_dir_name = provenance_str(source, "game_dir_name")?;
    let default_map = provenance_str(source, "default_map")?;
    let definition_csv_fnv1a64 = provenance_hash(source, "definition_csv_fnv1a64")?;
    let provinces_bmp_fnv1a64 = provenance_hash(source, "provinces_bmp_fnv1a64")?;
    // The generator always writes the visual-input hash map (bundle_gen
    // populates it unconditionally), so its absence — like an empty map — is
    // stripped provenance, not an older-but-valid bundle.
    let visual = source
        .get("visual_input_fnv1a64")
        .ok_or("manifest.json source.visual_input_fnv1a64 is missing (capture-side contract)")?
        .as_object()
        .ok_or("manifest.json source.visual_input_fnv1a64 is not an object")?;
    if visual.is_empty() {
        return Err(
            "manifest.json source.visual_input_fnv1a64 is empty; the generator always \
             records the visual source-input identities"
                .into(),
        );
    }
    let mut visual_input_fnv1a64 = BTreeMap::new();
    for (path, hash) in visual {
        let hash = hash.as_str().filter(|hash| is_fnv1a64_hex(hash)).ok_or_else(|| {
            format!(
                "manifest.json source.visual_input_fnv1a64[{path:?}] is not a \
                 16-hex fnv1a64 identity"
            )
        })?;
        if path.is_empty() {
            return Err(
                "manifest.json source.visual_input_fnv1a64 has an empty source path".into(),
            );
        }
        visual_input_fnv1a64.insert(path.clone(), hash.to_string());
    }

    let (modset_id, mod_count) = match manifest.get("modset") {
        // The generator omits the block entirely for an implicit vanilla
        // selection; that is the vanilla identity by construction.
        None => ("vanilla".to_string(), 0usize),
        Some(modset) => {
            if !modset.is_object() {
                return Err("manifest.json \"modset\" is present but is not an object".into());
            }
            let modset_id = modset
                .get("modset_id")
                .and_then(serde_json::Value::as_str)
                .ok_or("manifest.json modset.modset_id is missing or not a string")?;
            if modset_id != "vanilla" && !is_fnv1a64_hex(modset_id) {
                return Err(format!(
                    "manifest.json modset.modset_id is neither \"vanilla\" nor a 16-hex \
                     identity, found {modset_id:?}"
                )
                .into());
            }
            let mod_count = modset
                .get("mod_count")
                .and_then(serde_json::Value::as_u64)
                .ok_or("manifest.json modset.mod_count is missing or not an unsigned integer")?
                as usize;
            let mods = modset
                .get("mods")
                .and_then(serde_json::Value::as_array)
                .ok_or("manifest.json modset.mods is missing or not an array")?;
            if mods.len() != mod_count {
                return Err(format!(
                    "manifest.json modset is inconsistent: mod_count {} but {} mod entries",
                    mod_count,
                    mods.len()
                )
                .into());
            }
            // The capture-side identity rule is a biconditional:
            // `modset_id(&[]) == "vanilla"` and a non-empty selection always
            // hashes to a non-vanilla id, so either contradiction is a
            // mangled manifest.
            if modset_id == "vanilla" && mod_count != 0 {
                return Err(format!(
                    "manifest.json modset claims the vanilla identity with {mod_count} mods"
                )
                .into());
            }
            if modset_id != "vanilla" && mod_count == 0 {
                return Err(format!(
                    "manifest.json modset claims identity {modset_id:?} with zero mods; \
                     an empty selection is always the vanilla identity"
                )
                .into());
            }
            (modset_id.to_string(), mod_count)
        }
    };

    Ok(BundleProvenance {
        game_dir_name,
        default_map,
        definition_csv_fnv1a64,
        provinces_bmp_fnv1a64,
        visual_input_fnv1a64,
        modset_id,
        mod_count,
    })
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

    /// A well-formed capture-side `source` provenance block for fixtures.
    pub(crate) fn fixture_source_block() -> serde_json::Value {
        serde_json::json!({
            "game_dir_name": "the game",
            "default_map": "map/default.map",
            "definition_csv_fnv1a64": "0123456789abcdef",
            "provinces_bmp_fnv1a64": "fedcba9876543210",
            "visual_input_fnv1a64": { "map/terrain.bmp": "00112233aabbccdd" },
        })
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
            "source": fixture_source_block(),
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
        let manifest = serde_json::json!({
            "schema": BUNDLE_SCHEMA,
            "dimensions": { "width": 8, "height": 4 },
            "source": fixture_source_block(),
            "files": { "province_ids": 17 },
        });
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
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
        let manifest = serde_json::json!({
            "schema": BUNDLE_SCHEMA,
            "dimensions": { "width": 8, "height": 4 },
            "source": fixture_source_block(),
            "files": { "country_colors": "../country_colors.json" },
        });
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
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

    // ---- FW7 H1: load-time source-hash / modset provenance verification ----

    fn overwrite_manifest(dir: &Path, manifest: serde_json::Value) {
        fs::write(
            dir.join("manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("overwrite manifest");
    }

    fn manifest_with(source: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "schema": BUNDLE_SCHEMA,
            "dimensions": { "width": 8, "height": 4 },
            "source": source,
        })
    }

    #[test]
    fn fixture_bundle_surfaces_verified_provenance() {
        let dir = unique_temp_dir("provenance_ok");
        write_fixture_bundle(&dir);
        let bundle = MapBundle::load(&dir).expect("fixture bundle loads");
        assert_eq!(bundle.provenance.game_dir_name, "the game");
        assert_eq!(
            bundle.provenance.definition_csv_fnv1a64,
            "0123456789abcdef"
        );
        assert_eq!(
            bundle.provenance.provinces_bmp_fnv1a64,
            "fedcba9876543210"
        );
        assert_eq!(
            bundle
                .provenance
                .visual_input_fnv1a64
                .get("map/terrain.bmp")
                .map(String::as_str),
            Some("00112233aabbccdd")
        );
        // No modset block => the vanilla identity by the capture-side rule.
        assert_eq!(bundle.provenance.modset_id, "vanilla");
        assert_eq!(bundle.provenance.mod_count, 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_source_provenance_fails_closed_with_guidance() {
        let dir = unique_temp_dir("provenance_missing");
        write_fixture_bundle(&dir);
        overwrite_manifest(
            &dir,
            serde_json::json!({
                "schema": BUNDLE_SCHEMA,
                "dimensions": { "width": 8, "height": 4 },
            }),
        );
        let err = MapBundle::load(&dir).expect_err("stripped provenance must fail");
        let message = err.to_string();
        assert!(message.contains("source"), "{message}");
        assert!(message.contains("map_viewer -- bundle"), "{message}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_source_hashes_fail_closed() {
        for (key, bad) in [
            ("definition_csv_fnv1a64", "not-a-hash"),
            ("definition_csv_fnv1a64", "0123456789ABCDEF"), // uppercase
            ("provinces_bmp_fnv1a64", "0123"),              // short
        ] {
            let dir = unique_temp_dir("provenance_bad_hash");
            write_fixture_bundle(&dir);
            let mut source = fixture_source_block();
            source[key] = serde_json::Value::String(bad.to_string());
            overwrite_manifest(&dir, manifest_with(source));
            let err = match MapBundle::load(&dir) {
                Err(err) => err,
                Ok(_) => panic!("{key}={bad:?} must fail closed"),
            };
            assert!(err.to_string().contains(key), "{err}");
            fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn missing_or_empty_visual_input_hashes_fail_closed() {
        // Deleted property: stripped provenance, refused.
        let dir = unique_temp_dir("provenance_visual_missing");
        write_fixture_bundle(&dir);
        let mut source = fixture_source_block();
        source
            .as_object_mut()
            .expect("source object")
            .remove("visual_input_fnv1a64");
        overwrite_manifest(&dir, manifest_with(source));
        let err = MapBundle::load(&dir).expect_err("missing visual hashes must fail");
        assert!(err.to_string().contains("visual_input_fnv1a64"), "{err}");
        fs::remove_dir_all(&dir).ok();

        // Present but empty map: equally stripped, refused.
        let dir = unique_temp_dir("provenance_visual_empty");
        write_fixture_bundle(&dir);
        let mut source = fixture_source_block();
        source["visual_input_fnv1a64"] = serde_json::json!({});
        overwrite_manifest(&dir, manifest_with(source));
        let err = MapBundle::load(&dir).expect_err("empty visual hashes must fail");
        assert!(err.to_string().contains("empty"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modset_vanilla_biconditional_is_enforced_both_ways() {
        // Non-vanilla identity with zero mods contradicts the capture-side
        // rule (an empty selection always hashes to "vanilla").
        let dir = unique_temp_dir("provenance_modset_reverse");
        write_fixture_bundle(&dir);
        let mut manifest = manifest_with(fixture_source_block());
        manifest["modset"] = serde_json::json!({
            "modset_id": "0123456789abcdef",
            "selection_source": "dlc_load.json",
            "mod_count": 0,
            "mods": [],
        });
        overwrite_manifest(&dir, manifest);
        let err = MapBundle::load(&dir).expect_err("non-vanilla id with zero mods must fail");
        assert!(err.to_string().contains("vanilla"), "{err}");
        fs::remove_dir_all(&dir).ok();

        // An explicit zero-survivor selection publishing the vanilla identity
        // is the legitimate shape and loads.
        let dir = unique_temp_dir("provenance_modset_explicit_vanilla");
        write_fixture_bundle(&dir);
        let mut manifest = manifest_with(fixture_source_block());
        manifest["modset"] = serde_json::json!({
            "modset_id": "vanilla",
            "selection_source": "dlc_load.json",
            "mod_count": 0,
            "mods": [],
        });
        overwrite_manifest(&dir, manifest);
        let bundle = MapBundle::load(&dir).expect("explicit vanilla selection loads");
        assert_eq!(bundle.provenance.modset_id, "vanilla");
        assert_eq!(bundle.provenance.mod_count, 0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modset_block_is_verified_and_surfaced() {
        // A consistent modset block loads and surfaces its identity.
        let dir = unique_temp_dir("provenance_modset_ok");
        write_fixture_bundle(&dir);
        let mut manifest = manifest_with(fixture_source_block());
        manifest["modset"] = serde_json::json!({
            "modset_id": "35bc3f7d45155d2b",
            "selection_source": "dlc_load.json",
            "mod_count": 1,
            "mods": [ { "load_order": 0, "name": "Test Mod", "replace_path": [] } ],
        });
        overwrite_manifest(&dir, manifest);
        let bundle = MapBundle::load(&dir).expect("modded bundle loads");
        assert_eq!(bundle.provenance.modset_id, "35bc3f7d45155d2b");
        assert_eq!(bundle.provenance.mod_count, 1);
        fs::remove_dir_all(&dir).ok();

        // mod_count disagreeing with the mods array is refused.
        let dir = unique_temp_dir("provenance_modset_count");
        write_fixture_bundle(&dir);
        let mut manifest = manifest_with(fixture_source_block());
        manifest["modset"] = serde_json::json!({
            "modset_id": "35bc3f7d45155d2b",
            "selection_source": "dlc_load.json",
            "mod_count": 2,
            "mods": [ { "load_order": 0, "name": "Test Mod", "replace_path": [] } ],
        });
        overwrite_manifest(&dir, manifest);
        let err = MapBundle::load(&dir).expect_err("inconsistent mod_count must fail");
        assert!(err.to_string().contains("mod_count"), "{err}");
        fs::remove_dir_all(&dir).ok();

        // The vanilla identity with a non-empty mod list is a contradiction.
        let dir = unique_temp_dir("provenance_modset_vanilla");
        write_fixture_bundle(&dir);
        let mut manifest = manifest_with(fixture_source_block());
        manifest["modset"] = serde_json::json!({
            "modset_id": "vanilla",
            "selection_source": "dlc_load.json",
            "mod_count": 1,
            "mods": [ { "load_order": 0, "name": "Test Mod", "replace_path": [] } ],
        });
        overwrite_manifest(&dir, manifest);
        let err = MapBundle::load(&dir).expect_err("vanilla id with mods must fail");
        assert!(err.to_string().contains("vanilla"), "{err}");
        fs::remove_dir_all(&dir).ok();
    }

    // ---- Bundle provenance vs recording modset reconciliation ----

    pub(crate) fn provenance_with(modset_id: &str, mod_count: usize) -> BundleProvenance {
        BundleProvenance {
            game_dir_name: "the game".into(),
            default_map: "map/default.map".into(),
            definition_csv_fnv1a64: "0123456789abcdef".into(),
            provinces_bmp_fnv1a64: "fedcba9876543210".into(),
            visual_input_fnv1a64: BTreeMap::from([(
                "map/terrain.bmp".to_string(),
                "00112233aabbccdd".to_string(),
            )]),
            modset_id: modset_id.into(),
            mod_count,
        }
    }

    /// A recording manifest whose ordered layers carry `mod_layers` mod rows on
    /// top of the base layer.
    pub(crate) fn recording_manifest(mod_layers: usize) -> replay_core::ContentManifest {
        use replay_core::{
            ContentLayerKind, LayerStorageKind, RecordedContentLayer, SourceGameIdentity,
            SourceProfileIdentity,
        };
        let mut ordered_layers = vec![RecordedContentLayer {
            ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            display_name: "the game".into(),
            descriptor_sha256: None,
            source_container_sha256: None,
            storage_kind: LayerStorageKind::Directory,
            replace_paths: Vec::new(),
            required_for_playback: true,
        }];
        for index in 0..mod_layers {
            ordered_layers.push(RecordedContentLayer {
                ordinal: 1 + index as u32,
                layer_kind: ContentLayerKind::Mod,
                layer_id: format!("mod/ugc_{index}.mod"),
                display_name: format!("Test Mod {index}"),
                descriptor_sha256: Some("d".repeat(64)),
                source_container_sha256: None,
                storage_kind: LayerStorageKind::Directory,
                replace_paths: Vec::new(),
                required_for_playback: true,
            });
        }
        replay_core::ContentManifest {
            schema: "hoi4-portable-content-manifest-v1".into(),
            product_modset_sha256: "9".repeat(64),
            source_game: SourceGameIdentity {
                version: "1.19.2".into(),
                build: "a729".into(),
                executable_sha256: "a".repeat(64),
                build_profile: "release".into(),
            },
            source_profile: SourceProfileIdentity {
                profile_id: "profile".into(),
                playset_id: "playset".into(),
                selection_sha256: "b".repeat(64),
            },
            effective_vfs_manifest_sha256: "c".repeat(64),
            ordered_layers,
            dlc_selection: Vec::new(),
            localisation_language: "l_english".into(),
            required_settings: BTreeMap::new(),
            recorder_local_settings: BTreeMap::new(),
            required_content: Vec::new(),
            manifest_digest_sha256: None,
        }
    }

    /// The gate scenario: a bundle/recording modset disagreement produces the
    /// typed refusal the house already classifies as `RejectionKind::Modset`,
    /// in BOTH directions.
    #[test]
    fn bundle_recording_modset_disagreement_is_a_typed_modset_mismatch() {
        // Vanilla bundle, modded recording.
        let error = reconcile_bundle_modset(
            &provenance_with("vanilla", 0),
            Some(&recording_manifest(2)),
        )
        .expect_err("vanilla bundle vs modded recording must refuse");
        assert!(
            matches!(error, replay_core::SourceError::ModsetMismatch(_)),
            "expected the typed modset class, got {error:?}"
        );
        assert!(
            error.to_string().contains("2 mod layer"),
            "the refusal must name the evidence: {error}"
        );

        // Modded bundle, vanilla recording.
        let error = reconcile_bundle_modset(
            &provenance_with("35bc3f7d45155d2b", 2),
            Some(&recording_manifest(0)),
        )
        .expect_err("modded bundle vs vanilla recording must refuse");
        assert!(
            matches!(error, replay_core::SourceError::ModsetMismatch(_)),
            "expected the typed modset class, got {error:?}"
        );
        assert!(
            error.to_string().contains("35bc3f7d45155d2b"),
            "the refusal must name the bundle identity: {error}"
        );
    }

    /// Agreement is only claimed where it is actually decidable.
    #[test]
    fn matching_vanilla_identities_are_the_only_decided_agreement() {
        let agreement =
            reconcile_bundle_modset(&provenance_with("vanilla", 0), Some(&recording_manifest(0)))
                .expect("matching vanilla identities agree");
        assert_eq!(agreement, BundleModsetAgreement::AgreedVanilla);
        assert!(agreement.is_decided());
    }

    /// Two modded sides agree on CLASS only. The bundle digest is taken over
    /// (workshop-or-name, version) pairs and a recorded layer carries neither,
    /// so exact equality is undecidable and must be recorded as such — never
    /// reported as a match.
    #[test]
    fn two_modded_sides_are_recorded_as_not_comparable_not_as_agreement() {
        let agreement = reconcile_bundle_modset(
            &provenance_with("35bc3f7d45155d2b", 2),
            Some(&recording_manifest(2)),
        )
        .expect("a modded/modded pair is not a refusal");
        assert_eq!(
            agreement,
            BundleModsetAgreement::ModdedIdentityNotComparable {
                bundle_modset_id: "35bc3f7d45155d2b".into(),
                bundle_mod_count: 2,
                recording_mod_layer_count: 2,
            }
        );
        assert!(
            !agreement.is_decided(),
            "an undecidable comparison must never read as decided agreement"
        );
        let diagnostic = agreement.diagnostic();
        assert!(diagnostic.contains("NOT COMPARABLE"), "{diagnostic}");

    }

    /// Undecidable does not mean unfalsifiable. Both counts are exact roster
    /// sizes (the bundle verifies `mod_count == mods.len()`; the recorder
    /// verifies its Mod-layer count equals the launch-bound roster), so two
    /// different sizes DISPROVE sameness even though equal sizes cannot prove
    /// it. That must refuse, not degrade.
    #[test]
    fn differing_modded_roster_sizes_are_a_decidable_disproof_and_refuse() {
        let error = reconcile_bundle_modset(
            &provenance_with("35bc3f7d45155d2b", 2),
            Some(&recording_manifest(3)),
        )
        .expect_err("2 bundle mods vs 3 recorded mod layers cannot be the same roster");
        assert!(
            matches!(error, replay_core::SourceError::ModsetMismatch(_)),
            "expected the typed modset class, got {error:?}"
        );
        let message = error.to_string();
        assert!(message.contains("2 mod(s)"), "{message}");
        assert!(message.contains("3 mod layer(s)"), "{message}");

        // The reverse direction is equally decidable.
        let error = reconcile_bundle_modset(
            &provenance_with("35bc3f7d45155d2b", 4),
            Some(&recording_manifest(1)),
        )
        .expect_err("4 bundle mods vs 1 recorded mod layer cannot be the same roster");
        assert!(matches!(
            error,
            replay_core::SourceError::ModsetMismatch(_)
        ));
    }

    /// A route with no recorded manifest carries no modset identity at all.
    /// The honest outcome is a recorded "cannot compare", not a pass.
    #[test]
    fn a_recording_without_a_manifest_is_recorded_as_not_comparable() {
        for provenance in [
            provenance_with("vanilla", 0),
            provenance_with("35bc3f7d45155d2b", 2),
        ] {
            let agreement = reconcile_bundle_modset(&provenance, None)
                .expect("an absent identity is not a refusal");
            assert!(
                matches!(
                    agreement,
                    BundleModsetAgreement::RecordingIdentityAbsent { .. }
                ),
                "got {agreement:?}"
            );
            assert!(!agreement.is_decided());
            assert!(agreement.diagnostic().contains("NOT COMPARABLE"));
        }
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
