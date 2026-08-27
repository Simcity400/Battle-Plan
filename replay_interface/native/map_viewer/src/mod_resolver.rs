use crate::*;
use portable_content_manifest::foundation::{
    identity_sha256, sha256_bytes, FoundationContentRequest, FoundationSamplerDescriptor,
    FoundationShaderIdentity, FoundationShaderStage, FoundationTextureDescriptor,
    ResolvedFoundationContent, FOUNDATION_CONTENT_REQUEST_SCHEMA,
    RESOLVED_FOUNDATION_CONTENT_SCHEMA,
};
use portable_content_manifest::effective_vfs::{resolve_literal_layer_path, LayerPathFacts};
use portable_content_manifest::{
    filesystem_case_key, windows_collision_key, MAX_CONTENT_PAYLOAD_BYTES,
};
use replay_core::{
    normalize_game_relative_path, ContentLayerKind, ContentManifest, DlcSelection,
    LayerStorageKind, PlaybackContentInventory, PlaybackContentLayer, ResolvedPlaybackContent,
    SourceGameIdentity, SourceProfileIdentity, UsedContentKey, ValidatedContentAdmission,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use snapshot::sha256::Sha256;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::fs::File;
use std::io::{self, Cursor, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameFileResolverOptions {
    pub mod_paths: Vec<PathBuf>,
    pub use_dlc_load: bool,
    pub userdata: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BundleModSelectionOptions {
    pub(crate) mods: Vec<PathBuf>,
    pub(crate) use_dlc_load: bool,
    pub(crate) userdata: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(crate) struct BundleResolverSetup {
    pub(crate) resolver: FileResolver,
    explicit_modset: bool,
    selection_source: String,
    warnings: Vec<String>,
}

impl BundleResolverSetup {
    pub(crate) fn from_options(
        vanilla_root: PathBuf,
        options: &BundleModSelectionOptions,
    ) -> Result<Self> {
        let vanilla_root = require_dir(&vanilla_root, "game directory")?;
        let mut warnings: Vec<String> = Vec::new();
        if options.use_dlc_load {
            let userdata = options
                .userdata
                .as_ref()
                .ok_or("pass --userdata <the game user-data dir> with --use-dlc-load")?;
            let mods = content_layers_from_launcher(&vanilla_root, userdata, None)?;
            let resolver = FileResolver::with_mod_layers(vanilla_root, mods);
            Ok(Self {
                resolver,
                explicit_modset: true,
                selection_source: "dlc_load.json".to_string(),
                warnings,
            })
        } else if !options.mods.is_empty() {
            let mods = options
                .mods
                .iter()
                .enumerate()
                .map(|(idx, root)| explicit_mod_layer(idx, root, &mut warnings))
                .collect::<Result<Vec<_>>>()?;
            debug_assert_eq!(mods.len(), options.mods.len());
            let resolver = FileResolver::with_mod_layers(vanilla_root, mods);
            Ok(Self {
                resolver,
                explicit_modset: true,
                selection_source: "explicit --mod".to_string(),
                warnings,
            })
        } else {
            Ok(Self {
                resolver: FileResolver::vanilla(vanilla_root),
                explicit_modset: false,
                selection_source: "vanilla-only".to_string(),
                warnings,
            })
        }
    }

    pub(crate) fn manifest_modset(&self) -> Option<BundleModSet> {
        if !self.explicit_modset {
            return None;
        }
        let stats = self.resolver.stats();
        let mods = self.resolver.mods();
        Some(BundleModSet {
            modset_id: modset_id(&mods),
            selection_source: self.selection_source.clone(),
            mod_count: mods.len(),
            mods,
            overridden_file_count: stats.overridden_file_count,
            replace_path_excluded_file_count: stats.replace_path_excluded_file_count,
            warnings: self.warnings.clone(),
            note: "Ordered lowest-priority mod to highest-priority mod over vanilla; file-level override by relative path with later mods winning; replace_path removes lower-layer files only in the exact directory, not subdirectories.".to_string(),
        })
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ModContentLayer {
    pub(crate) root: PathBuf,
    pub(crate) manifest: BundleMod,
    source_kind: String,
    source_id: String,
    descriptor_sha256: Option<String>,
    archive_sha256: Option<String>,
    dependencies: Vec<String>,
    recorded_ordinal: Option<usize>,
}

#[derive(Clone, Debug)]
struct ResolverLayer {
    root: PathBuf,
    manifest: Option<BundleMod>,
    replace_paths: BTreeSet<String>,
    source: GameContentSource,
}

#[derive(Clone, Debug)]
pub(crate) struct FileResolver {
    layers: Vec<ResolverLayer>,
    stats: RefCell<ResolverStats>,
    scan: LayerScanCache,
}

#[derive(Clone, Debug, Default)]
struct ResolverStats {
    overridden_files: BTreeSet<String>,
    replace_path_excluded_files: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct FileResolverStats {
    pub(crate) overridden_file_count: usize,
    pub(crate) replace_path_excluded_file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameFileResolverStats {
    pub overridden_file_count: usize,
    pub replace_path_excluded_file_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SensitiveFileOverrideReport {
    pub overridden: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct GameFileResolver {
    resolver: FileResolver,
    selection_source: String,
    explicit_modset: bool,
    warnings: Vec<String>,
    admission: Option<Arc<ContentAdmission>>,
}

#[derive(Debug)]
struct ContentAdmission {
    entries: BTreeMap<String, AdmittedContent>,
    effective_paths: BTreeSet<String>,
    snapshot_root: PathBuf,
}

#[derive(Clone)]
pub struct ContentAdmissionLease(Arc<ContentAdmission>);

impl std::fmt::Debug for ContentAdmissionLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentAdmissionLease")
            .field("entry_count", &self.0.entries.len())
            .finish_non_exhaustive()
    }
}

impl PartialEq for ContentAdmissionLease {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ContentAdmissionLease {}

#[derive(Debug)]
struct AdmittedContent {
    key: UsedContentKey,
    resolved: ResolvedGameFile,
    snapshot_path: PathBuf,
}

impl Drop for ContentAdmission {
    fn drop(&mut self) {
        let temp_root = std::env::temp_dir().join("battle-plan-content-admission");
        if self.snapshot_root.starts_with(&temp_root) {
            remove_tree_allow_readonly(&self.snapshot_root);
        }
    }
}

static CONTENT_ADMISSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn unique_content_admission_root() -> PathBuf {
    let sequence = CONTENT_ADMISSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir()
        .join("battle-plan-content-admission")
        .join(format!("{}-{sequence}", std::process::id()))
}

fn complete_effective_path_census(resolver: &FileResolver) -> Result<BTreeSet<String>> {
    Ok(resolver
        .resolve_dir_files_recursive("", None)?
        .into_iter()
        .map(|(path, _)| path.to_ascii_lowercase())
        .collect())
}

impl GameFileResolver {
    #[cfg(test)]
    pub(crate) fn directory_scans_taken(&self) -> usize {
        self.resolver.scan.scans_taken()
    }
}

#[derive(Clone, Debug)]
struct ResolvedLayerFile {
    real_rel: String,
    path: PathBuf,
    layer_index: usize,
}

impl GameFileResolver {
    pub fn foundation_content_request(
        &self,
        admission: &ValidatedContentAdmission,
        map_width: u32,
        map_height: u32,
        pow2_ratio_branch: bool,
        max_texture_dimension: u32,
    ) -> Result<FoundationContentRequest> {
        let request = FoundationContentRequest {
            schema: FOUNDATION_CONTENT_REQUEST_SCHEMA.into(),
            effect_file: "gfx/fx/pdxmap.shader".into(),
            effect_name: "underwater".into(),
            texture_file: "map/terrain/underwater_terrain_0.dds".into(),
            texture_sampler: "TerrainDiffuse".into(),
            map_width,
            map_height,
            pow2_ratio_branch,
            max_texture_dimension,
        };
        request
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        for path in [&request.effect_file, &request.texture_file] {
            admission
                .require_used_content(path)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        }
        let effect_bytes = self.read_admitted_used_content(
            admission
                .require_used_content(&request.effect_file)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?,
        )?;
        let effect_text = std::str::from_utf8(&effect_bytes)
            .map_err(|_| "foundation effect source is not UTF-8")?;
        let effect_file = crate::shader_compile::parse_effect_file(effect_text)?;
        let effect = effect_file
            .effect(&request.effect_name)
            .ok_or("effective foundation effect source has no underwater Effect")?;
        if effect.vertex_shader.is_none() || effect.pixel_shader.is_none() {
            return Err(
                "effective foundation underwater Effect does not define both stages".into(),
            );
        }
        let sampler = effect_file
            .pixel_stage
            .samplers
            .iter()
            .find(|sampler| sampler.name == request.texture_sampler)
            .ok_or("effective foundation effect has no TerrainDiffuse pixel sampler")?;
        if sampler.sampler_type.as_deref().unwrap_or("Texture2D") != "Texture2D" {
            return Err("foundation TerrainDiffuse sampler is not a 2D texture".into());
        }
        Ok(request)
    }

    pub fn resolve_foundation_content(
        &self,
        request: &FoundationContentRequest,
        admission: &ValidatedContentAdmission,
    ) -> Result<ResolvedFoundationContent> {
        request
            .validate()
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        if !self.has_content_admission() {
            return Err("foundation content resolution requires a bound content admission".into());
        }
        let effect_key = admission
            .require_used_content(&request.effect_file)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .clone();
        let texture_key = admission
            .require_used_content(&request.texture_file)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .clone();
        let effect_bytes = self.read_admitted_used_content(&effect_key)?;
        let effect_text = std::str::from_utf8(&effect_bytes)
            .map_err(|_| "foundation effect source is not UTF-8")?;
        let parsed = crate::shader_compile::parse_effect_file(effect_text)?;
        let effect = parsed
            .effect(&request.effect_name)
            .ok_or_else(|| {
                format!(
                    "{} has no Effect named {}",
                    request.effect_file, request.effect_name
                )
            })?
            .clone();
        let vertex_entry = effect
            .vertex_shader
            .clone()
            .ok_or("foundation Effect has no vertex shader")?;
        let pixel_entry = effect
            .pixel_shader
            .clone()
            .ok_or("foundation Effect has no pixel shader")?;
        let sampler = parsed
            .pixel_stage
            .samplers
            .iter()
            .find(|sampler| sampler.name == request.texture_sampler)
            .ok_or_else(|| {
                format!(
                    "foundation Effect has no pixel sampler {}",
                    request.texture_sampler
                )
            })?;

        let constants = crate::shader_compile::derive_a729_engine_map_shader_constants(
            request.map_width,
            request.map_height,
            request.pow2_ratio_branch,
            request.max_texture_dimension,
        )?;
        let engine_defines = crate::shader_compile::engine_map_shader_defines(constants);
        let vertex_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
            self,
            &request.effect_file,
            &request.effect_name,
            crate::shader_compile::ShaderStage::Vertex,
            &engine_defines,
        )?;
        let pixel_hlsl = crate::shader_compile::build_stage_hlsl_with_defines(
            self,
            &request.effect_file,
            &request.effect_name,
            crate::shader_compile::ShaderStage::Pixel,
            &engine_defines,
        )?;

        let mut ordered_used_content = Vec::new();
        let mut seen = BTreeSet::new();
        push_foundation_content(&mut ordered_used_content, &mut seen, effect_key.clone())?;
        for include in &parsed.includes {
            let path = format!("{}/{}", crate::shader_compile::SHADER_FX_DIR, include);
            let key = admission
                .require_used_content(&path)
                .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
                .clone();
            let bytes = self.read_admitted_used_content(&key)?;
            let include_text = std::str::from_utf8(&bytes)
                .map_err(|_| format!("foundation include {path} is not UTF-8"))?;
            let include_file = crate::shader_compile::parse_effect_file(include_text)?;
            if !include_file.includes.is_empty() {
                return Err(format!(
                    "foundation include {path} has nested Includes, unsupported by the exact-a729 translation-unit builder"
                )
                .into());
            }
            push_foundation_content(&mut ordered_used_content, &mut seen, key)?;
        }
        let platform_key = admission
            .require_used_content(crate::shader_compile::DX11_PLATFORM_HEADER)
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?
            .clone();
        self.read_admitted_used_content(&platform_key)?;
        push_foundation_content(&mut ordered_used_content, &mut seen, platform_key)?;
        let texture_bytes = self.read_admitted_used_content(&texture_key)?;
        push_foundation_content(&mut ordered_used_content, &mut seen, texture_key.clone())?;

        let texture = foundation_dds_descriptor(&texture_bytes, &texture_key)?;
        let sampler_descriptor = FoundationSamplerDescriptor {
            name: sampler.name.clone(),
            shader_register: sampler.index,
            resource_type: sampler
                .sampler_type
                .clone()
                .unwrap_or_else(|| "Texture2D".into()),
            mag_filter: required_foundation_sampler_field(
                sampler.mag_filter.as_deref(),
                "MagFilter",
            )?,
            min_filter: required_foundation_sampler_field(
                sampler.min_filter.as_deref(),
                "MinFilter",
            )?,
            mip_filter: required_foundation_sampler_field(
                sampler.mip_filter.as_deref(),
                "MipFilter",
            )?,
            address_u: required_foundation_sampler_field(sampler.address_u.as_deref(), "AddressU")?,
            address_v: required_foundation_sampler_field(sampler.address_v.as_deref(), "AddressV")?,
            mip_map_lod_bias: sampler.mip_map_lod_bias.clone(),
            mip_map_min_lod: sampler.mip_map_min_lod.clone(),
            mip_map_max_lod: sampler.mip_map_max_lod.clone(),
            max_anisotropy: sampler.max_anisotropy.clone(),
        };

        let blend_state =
            resolve_foundation_state(effect.blend_state.as_deref(), &parsed.blend_states, "blend")?;
        let depth_stencil_state = resolve_foundation_state(
            effect.depth_stencil_state.as_deref(),
            &parsed.depth_stencil_states,
            "depth-stencil",
        )?;
        let rasterizer_state = resolve_foundation_state(
            effect.rasterizer_state.as_deref(),
            &parsed.rasterizer_states,
            "rasterizer",
        )?;

        let resolved = ResolvedFoundationContent {
            schema: RESOLVED_FOUNDATION_CONTENT_SCHEMA.into(),
            request: request.clone(),
            manifest_digest_sha256: admission.manifest_digest_sha256().into(),
            product_modset_sha256: admission.product_modset_sha256().into(),
            effective_vfs_manifest_sha256: effect_key.effective_vfs_manifest_sha256.clone(),
            ordered_used_content,
            effect_defines: effect.defines,
            blend_state,
            depth_stencil_state,
            rasterizer_state,
            vertex_shader: FoundationShaderIdentity {
                stage: FoundationShaderStage::Vertex,
                profile: "vs_5_0".into(),
                entry_point: vertex_entry,
                translation_unit_sha256: sha256_bytes(vertex_hlsl.as_bytes()),
                translation_unit_bytes: vertex_hlsl.len() as u64,
            },
            pixel_shader: FoundationShaderIdentity {
                stage: FoundationShaderStage::Pixel,
                profile: "ps_5_0".into(),
                entry_point: pixel_entry,
                translation_unit_sha256: sha256_bytes(pixel_hlsl.as_bytes()),
                translation_unit_bytes: pixel_hlsl.len() as u64,
            },
            texture_sampler: sampler_descriptor,
            texture,
            operation_identity_sha256: String::new(),
        }
        .with_computed_operation_identity()
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
        Ok(resolved)
    }
    pub fn vanilla(vanilla_root: impl Into<PathBuf>) -> Result<Self> {
        Self::from_options(vanilla_root, GameFileResolverOptions::default())
    }

    pub fn with_mod_paths(
        vanilla_root: impl Into<PathBuf>,
        mod_paths: Vec<PathBuf>,
    ) -> Result<Self> {
        Self::from_options(
            vanilla_root,
            GameFileResolverOptions {
                mod_paths,
                use_dlc_load: false,
                userdata: None,
            },
        )
    }

    pub fn from_dlc_load(vanilla_root: impl Into<PathBuf>, userdata: PathBuf) -> Result<Self> {
        Self::from_options(
            vanilla_root,
            GameFileResolverOptions {
                mod_paths: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata),
            },
        )
    }

    pub fn from_recorded_content_order(
        vanilla_root: impl Into<PathBuf>,
        userdata: PathBuf,
        recorded_source_ids: &[String],
    ) -> Result<Self> {
        let recorded = recorded_source_ids
            .iter()
            .enumerate()
            .map(|(index, source_id)| RecordedContentSource {
                ordinal: (index + 1) as u32,
                source_id: source_id.clone(),
            })
            .collect::<Vec<_>>();
        Self::from_recorded_content_layers(vanilla_root, userdata, &recorded)
    }

    pub fn from_recorded_content_layers(
        vanilla_root: impl Into<PathBuf>,
        userdata: PathBuf,
        recorded_layers: &[RecordedContentSource],
    ) -> Result<Self> {
        let vanilla_root = require_dir(&vanilla_root.into(), "game directory")?;
        let layers = content_layers_from_launcher(&vanilla_root, &userdata, Some(recorded_layers))?;
        Ok(Self {
            resolver: FileResolver::with_mod_layers(vanilla_root, layers),
            selection_source: "portable replay recorded content order".to_string(),
            explicit_modset: true,
            warnings: Vec::new(),
            admission: None,
        })
    }

    pub fn from_options(
        vanilla_root: impl Into<PathBuf>,
        options: GameFileResolverOptions,
    ) -> Result<Self> {
        if options.use_dlc_load && !options.mod_paths.is_empty() {
            return Err(
                "pass either repeated --mod content dirs or --use-dlc-load, not both".into(),
            );
        }
        if options.use_dlc_load && options.userdata.is_none() {
            return Err("pass --userdata <the game user-data dir> with --use-dlc-load".into());
        }
        let setup = BundleResolverSetup::from_options(
            vanilla_root.into(),
            &BundleModSelectionOptions {
                mods: options.mod_paths,
                use_dlc_load: options.use_dlc_load,
                userdata: options.userdata,
            },
        )?;
        Ok(Self {
            resolver: setup.resolver,
            selection_source: setup.selection_source,
            explicit_modset: setup.explicit_modset,
            warnings: setup.warnings,
            admission: None,
        })
    }

    pub fn game_dir(&self) -> &Path {
        self.resolver
            .layers
            .first()
            .map(|layer| layer.root.as_path())
            .unwrap_or_else(|| Path::new(""))
    }

    pub fn selection_source(&self) -> &str {
        &self.selection_source
    }

    pub fn has_explicit_modset(&self) -> bool {
        self.explicit_modset
    }

    pub fn bundle_modset_id(&self) -> String {
        modset_id(&self.resolver.mods())
    }

    pub fn bundle_mod_count(&self) -> usize {
        self.resolver.mods().len()
    }

    pub fn resolve(&self, rel_path: &str) -> Result<Option<PathBuf>> {
        if self.admission.is_some() {
            return Ok(self
                .resolve_with_provenance(rel_path)?
                .map(|resolved| resolved.local_path));
        }
        self.resolver.resolve_optional_file(rel_path)
    }

    pub fn resolve_required(&self, rel_path: &str, label: &str) -> Result<PathBuf> {
        self.resolve(rel_path)?
            .ok_or_else(|| format!("missing {label}: {}", normalize_rel_path(rel_path)).into())
    }

    pub fn resolve_with_provenance(&self, rel_path: &str) -> Result<Option<ResolvedGameFile>> {
        Ok(self
            .resolve_with_provenance_bytes(rel_path)?
            .map(|(resolved, _bytes)| resolved))
    }

    fn resolve_with_provenance_bytes(
        &self,
        rel_path: &str,
    ) -> Result<Option<(ResolvedGameFile, Vec<u8>)>> {
        let logical_path = checked_rel_path(rel_path)?;
        if let Some(admission) = &self.admission {
            let admitted = admission
                .entries
                .get(&content_identity_key(&logical_path)?)
                .ok_or_else(|| {
                    format!(
                        "replay content admission rejects unrecorded/default lookup {:?}",
                        logical_path
                    )
                })?;
            let bytes = read_bounded_file(
                &admitted.snapshot_path,
                "immutable admitted content snapshot",
            )?;
            let mut resolved = admitted.resolved.clone();
            resolved.local_path = admitted.snapshot_path.clone();
            validate_used_content_match(&resolved, &admitted.key)?;
            if bytes.len() as u64 != admitted.key.payload_bytes
                || Sha256::hex(&bytes) != admitted.key.payload_sha256
            {
                return Err(format!(
                    "immutable admitted content snapshot changed for {}",
                    admitted.key.normalized_game_relative_path
                )
                .into());
            }
            return Ok(Some((resolved, bytes)));
        }
        let Some(resolved) = self
            .resolver
            .resolve_optional_file_with_layer(&logical_path)?
        else {
            return Ok(None);
        };
        let bytes = read_bounded_file(&resolved.path, "resolved content payload")?;
        let source = self
            .resolver
            .layers
            .get(resolved.layer_index)
            .ok_or("resolved VFS layer index is out of bounds")?
            .source
            .clone();
        let result = ResolvedGameFile {
            logical_path,
            source,
            source_relative_path: normalize_rel_path(&resolved.real_rel),
            byte_len: bytes.len() as u64,
            sha256: Sha256::hex(&bytes),
            local_path: resolved.path,
        };
        self.validate_admitted_result(&result)?;
        Ok(Some((result, bytes)))
    }

    pub fn resolve_required_with_provenance(
        &self,
        rel_path: &str,
        label: &str,
    ) -> Result<ResolvedGameFile> {
        self.resolve_with_provenance(rel_path)?
            .ok_or_else(|| format!("missing {label}: {}", normalize_rel_path(rel_path)).into())
    }

    pub fn active_sources(&self) -> Vec<GameContentSource> {
        self.resolver
            .layers
            .iter()
            .map(|layer| layer.source.clone())
            .collect()
    }

    pub fn mounted_dlc_selection(&self) -> Vec<DlcSelection> {
        let ids = self
            .resolver
            .layers
            .iter()
            .filter_map(|layer| stable_dlc_product_id(&layer.source.source_id))
            .collect::<BTreeSet<_>>();
        ids.iter()
            .map(|dlc_id| DlcSelection {
                dlc_id: dlc_id.clone(),
                enabled: true,
                required_for_playback: true,
            })
            .collect()
    }

    pub fn with_content_admission(mut self, admission: &ValidatedContentAdmission) -> Result<Self> {
        let session = self.resolver.scan.session();
        let mut required = BTreeMap::new();
        for path in admission.required_paths() {
            let key = admission
                .require_used_content(path)
                .map_err(|err| format!("invalid replay content admission: {err}"))?
                .clone();
            let normalized = content_identity_key(path)?;
            if required.insert(normalized, key).is_some() {
                return Err("replay content admission repeats a logical path".into());
            }
        }
        self.admission = None;
        let effective_paths = complete_effective_path_census(&self.resolver)?;
        let snapshot_root = self.create_admission_snapshot_root()?;
        let mut snapshot_guard = StagingDirectoryGuard {
            path: snapshot_root.clone(),
            armed: true,
        };
        let mut entries = BTreeMap::new();
        for (normalized, key) in required {
            let (actual, bytes) = self
                .resolve_with_provenance_bytes(&key.normalized_game_relative_path)?
                .ok_or_else(|| {
                    format!(
                        "missing required admitted content: {}",
                        key.normalized_game_relative_path
                    )
                })?;
            validate_used_content_match(&actual, &key)?;
            let snapshot_path = path_from_slash_rel(
                &snapshot_root,
                &checked_rel_path(&key.normalized_game_relative_path)?,
            );
            if let Some(parent) = snapshot_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&snapshot_path, &bytes)?;
            let mut permissions = fs::metadata(&snapshot_path)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&snapshot_path, permissions)?;
            entries.insert(
                normalized,
                AdmittedContent {
                    key,
                    resolved: actual,
                    snapshot_path,
                },
            );
        }
        self.admission = Some(Arc::new(ContentAdmission {
            entries,
            effective_paths,
            snapshot_root,
        }));
        snapshot_guard.armed = false;
        drop(session);
        Ok(self)
    }

    fn refuse_snapshot_root_overlapping_a_layer(&self, snapshot_root: &Path) -> Result<()> {
        let mut existing = snapshot_root;
        let mut unborn = Vec::new();
        loop {
            match fs::symlink_metadata(existing) {
                Ok(_) => break,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!(
                        "resolve admitted content snapshot root {}: cannot determine whether {} \
                         exists: {error}",
                        snapshot_root.display(),
                        existing.display()
                    )
                    .into());
                }
            }
            let (Some(name), Some(parent)) = (existing.file_name(), existing.parent()) else {
                return Err(format!(
                    "admitted content snapshot root {} has no existing ancestor to resolve",
                    snapshot_root.display()
                )
                .into());
            };
            unborn.push(name.to_owned());
            existing = parent;
        }
        let mut snapshot = fs::canonicalize(existing).map_err(|error| {
            format!(
                "canonicalize admitted content snapshot root {}: {error}",
                snapshot_root.display()
            )
        })?;
        for name in unborn.iter().rev() {
            snapshot.push(name);
        }
        for layer in &self.resolver.layers {
            let root = fs::canonicalize(&layer.root).map_err(|error| {
                format!(
                    "canonicalize content layer root {}: {error}",
                    layer.root.display()
                )
            })?;
            if snapshot.starts_with(&root) || root.starts_with(&snapshot) {
                return Err(format!(
                    "admitted content snapshot root {} overlaps mounted content layer {}: {}",
                    snapshot.display(),
                    layer.source.source_id,
                    root.display()
                )
                .into());
            }
        }
        Ok(())
    }

    fn create_admission_snapshot_root(&self) -> Result<PathBuf> {
        let snapshot_root = unique_content_admission_root();
        self.refuse_snapshot_root_overlapping_a_layer(&snapshot_root)?;
        fs::create_dir_all(&snapshot_root)?;
        Ok(snapshot_root)
    }

    pub fn in_resolution_session<T>(&self, resolve: impl FnOnce(&Self) -> T) -> T {
        let _session = self.resolver.scan.session();
        resolve(self)
    }

    pub fn has_content_admission(&self) -> bool {
        self.admission.is_some()
    }

    pub fn content_admission_lease(&self) -> Option<ContentAdmissionLease> {
        self.admission.as_ref().cloned().map(ContentAdmissionLease)
    }

    #[cfg(test)]
    pub(crate) fn bind_exact_test_content(
        mut self,
        logical_path: &str,
        effective_vfs_manifest_sha256: &str,
    ) -> Result<(Self, UsedContentKey)> {
        let session = self.resolver.scan.session();
        let actual = self.resolve_required_with_provenance(logical_path, "test content")?;
        let source_sha256 = actual
            .source
            .archive_sha256
            .clone()
            .unwrap_or_else(|| actual.sha256.clone());
        let key = UsedContentKey {
            effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.to_string(),
            normalized_game_relative_path: normalize_game_relative_path(&actual.logical_path)?,
            winning_layer_ordinal: actual.source.ordinal as u32,
            layer_kind: content_layer_kind(&actual.source.kind),
            layer_id: actual.source.source_id.clone(),
            source_sha256,
            payload_sha256: actual.sha256.clone(),
            payload_bytes: actual.byte_len,
        };
        let effective_paths = complete_effective_path_census(&self.resolver)?;
        let snapshot_root = self.create_admission_snapshot_root()?;
        let snapshot_path = path_from_slash_rel(&snapshot_root, &key.normalized_game_relative_path);
        if let Some(parent) = snapshot_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = read_bounded_file(&actual.local_path, "test admitted content")?;
        fs::write(&snapshot_path, bytes)?;
        self.admission = Some(Arc::new(ContentAdmission {
            entries: BTreeMap::from([(
                key.normalized_game_relative_path.clone(),
                AdmittedContent {
                    key: key.clone(),
                    resolved: actual,
                    snapshot_path,
                },
            )]),
            effective_paths,
            snapshot_root,
        }));
        drop(session);
        Ok((self, key))
    }

    #[cfg(test)]
    pub(crate) fn bind_all_test_content(
        mut self,
        effective_vfs_manifest_sha256: &str,
    ) -> Result<Self> {
        let session = self.resolver.scan.session();
        validate_sha256(
            effective_vfs_manifest_sha256,
            "test effective-VFS manifest SHA-256",
        )?;
        self.admission = None;
        let effective_paths = complete_effective_path_census(&self.resolver)?;
        let paths = effective_paths.iter().cloned().collect::<Vec<_>>();
        let snapshot_root = self.create_admission_snapshot_root()?;
        let mut guard = StagingDirectoryGuard {
            path: snapshot_root.clone(),
            armed: true,
        };
        let mut entries = BTreeMap::new();
        for path in paths {
            let (actual, bytes) = self
                .resolve_with_provenance_bytes(&path)?
                .ok_or_else(|| format!("missing test content {path}"))?;
            let key = UsedContentKey {
                effective_vfs_manifest_sha256: effective_vfs_manifest_sha256.into(),
                normalized_game_relative_path: normalize_game_relative_path(&actual.logical_path)?,
                winning_layer_ordinal: actual.source.ordinal as u32,
                layer_kind: content_layer_kind(&actual.source.kind),
                layer_id: actual.source.source_id.clone(),
                source_sha256: actual
                    .source
                    .archive_sha256
                    .clone()
                    .unwrap_or_else(|| actual.sha256.clone()),
                payload_sha256: actual.sha256.clone(),
                payload_bytes: actual.byte_len,
            };
            let folded = content_identity_key(&key.normalized_game_relative_path)?;
            let snapshot_path =
                path_from_slash_rel(&snapshot_root, &key.normalized_game_relative_path);
            if let Some(parent) = snapshot_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&snapshot_path, bytes)?;
            let mut permissions = fs::metadata(&snapshot_path)?.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&snapshot_path, permissions)?;
            entries.insert(
                folded,
                AdmittedContent {
                    key,
                    resolved: actual,
                    snapshot_path,
                },
            );
        }
        self.admission = Some(Arc::new(ContentAdmission {
            entries,
            effective_paths,
            snapshot_root,
        }));
        guard.armed = false;
        drop(session);
        Ok(self)
    }

    pub fn playback_inventory(
        &self,
        manifest: &ContentManifest,
        environment: PlaybackEnvironment,
    ) -> Result<PlaybackContentInventory> {
        let _session = self.resolver.scan.session();
        let layers = self
            .active_sources()
            .into_iter()
            .map(|source| PlaybackContentLayer {
                recorded_ordinal: source.ordinal as u32,
                layer_kind: content_layer_kind(&source.kind),
                layer_id: source.source_id,
                descriptor_sha256: source.descriptor_sha256,
                source_container_sha256: source.archive_sha256.clone(),
                storage_kind: if source.archive_sha256.is_some() {
                    LayerStorageKind::ZipArchive
                } else {
                    LayerStorageKind::Directory
                },
                replace_paths: source.replace_path,
            })
            .collect();
        let mut resolved_content = Vec::new();
        for required in &manifest.required_content {
            let actual = self.resolve_required_with_provenance(
                &required.content.normalized_game_relative_path,
                "required portable replay content",
            )?;
            resolved_content.push(ResolvedPlaybackContent {
                normalized_game_relative_path: normalize_game_relative_path(&actual.logical_path)?,
                winning_layer_ordinal: actual.source.ordinal as u32,
                layer_kind: content_layer_kind(&actual.source.kind),
                layer_id: actual.source.source_id,
                source_sha256: actual
                    .source
                    .archive_sha256
                    .unwrap_or_else(|| actual.sha256.clone()),
                payload_sha256: actual.sha256,
                payload_bytes: actual.byte_len,
            });
        }
        resolved_content.sort_by(|left, right| {
            left.normalized_game_relative_path
                .cmp(&right.normalized_game_relative_path)
        });
        Ok(PlaybackContentInventory {
            source_game: environment.source_game,
            source_profile: environment.source_profile,
            layers,
            dlc_selection: environment.dlc_selection,
            localisation_language: environment.localisation_language,
            settings: environment.settings,
            resolved_content,
        })
    }

    pub fn resolve_used_content(
        &self,
        logical_paths: impl IntoIterator<Item = String>,
    ) -> Result<Vec<ResolvedGameFile>> {
        let _session = self.resolver.scan.session();
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for path in logical_paths {
            let normalized = checked_rel_path(&path)?;
            if !seen.insert(content_identity_key(&normalized)?) {
                continue;
            }
            let mut resolved =
                self.resolve_required_with_provenance(&normalized, "used replay content")?;
            resolved.logical_path = normalize_game_relative_path(&resolved.logical_path)?;
            out.push(resolved);
        }
        out.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
        Ok(out)
    }

    pub fn read_admitted_used_content(&self, expected: &UsedContentKey) -> Result<Vec<u8>> {
        let admission = self
            .admission
            .as_ref()
            .ok_or("portable used-content read requires a validated content admission")?;
        let normalized = content_identity_key(&expected.normalized_game_relative_path)?;
        let admitted = admission.entries.get(&normalized).ok_or_else(|| {
            format!(
                "portable used-content read rejected unlisted path {}",
                expected.normalized_game_relative_path
            )
        })?;
        if &admitted.key != expected {
            return Err(format!(
                "portable used-content key disagrees with the validated admission for {}",
                expected.normalized_game_relative_path
            )
            .into());
        }
        let (actual, bytes) = self
            .resolve_with_provenance_bytes(&expected.normalized_game_relative_path)?
            .ok_or_else(|| {
                format!(
                    "missing admitted portable render content: {}",
                    normalize_rel_path(&expected.normalized_game_relative_path)
                )
            })?;
        validate_used_content_match(&actual, expected)?;
        Ok(bytes)
    }

    pub fn restrict_to<'a>(
        &'a self,
        required: impl IntoIterator<Item = RequiredGameContent>,
    ) -> Result<RestrictedGameFileResolver<'a>> {
        let mut allowed = BTreeMap::new();
        for mut row in required {
            row.logical_path = checked_rel_path(&row.logical_path)?;
            validate_sha256(&row.sha256, "required content SHA-256")?;
            row.sha256.make_ascii_lowercase();
            if allowed
                .insert(content_identity_key(&row.logical_path)?, row)
                .is_some()
            {
                return Err("required content repeats a case-folded logical path".into());
            }
        }
        Ok(RestrictedGameFileResolver {
            resolver: self,
            allowed,
        })
    }

    pub fn resolve_dir_files(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        if self.admission.is_some() {
            return self.resolve_admitted_dir(rel_dir, extension, false);
        }
        self.resolver.resolve_dir_files(rel_dir, extension)
    }

    pub fn resolve_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        if self.admission.is_some() {
            return self.resolve_admitted_dir(rel_dir, extension, true);
        }
        self.resolver
            .resolve_dir_files_recursive(rel_dir, extension)
    }

    pub fn audit_effective_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<String>> {
        if let Some(admission) = &self.admission {
            return admitted_effective_paths(admission, rel_dir, extension, true);
        }
        self.resolver
            .resolve_dir_files_recursive(rel_dir, extension)
            .map(|rows| rows.into_iter().map(|(path, _)| path).collect())
    }

    pub fn audit_effective_dir_files(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<String>> {
        if let Some(admission) = &self.admission {
            return admitted_effective_paths(admission, rel_dir, extension, false);
        }
        self.resolver
            .resolve_dir_files(rel_dir, extension)
            .map(|rows| rows.into_iter().map(|(path, _)| path).collect())
    }

    pub fn require_complete_admitted_dir(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
        recursive: bool,
        label: &str,
    ) -> Result<Vec<UsedContentKey>> {
        let admission = self
            .admission
            .as_ref()
            .ok_or_else(|| format!("{label} requires a validated content admission"))?;
        let dir = content_identity_key(rel_dir)?;
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let wanted = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let effective = admission
            .effective_paths
            .iter()
            .filter(|folded| {
                folded.starts_with(&prefix)
                    && (recursive || immediate_parent(folded) == dir)
                    && wanted.as_ref().is_none_or(|ext| {
                        Path::new(folded)
                            .extension()
                            .and_then(|value| value.to_str())
                            .is_some_and(|actual| actual.eq_ignore_ascii_case(ext))
                    })
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut admitted = BTreeMap::new();
        for (folded, entry) in &admission.entries {
            if !folded.starts_with(&prefix) || (!recursive && immediate_parent(folded) != dir) {
                continue;
            }
            if let Some(ext) = &wanted {
                let actual = Path::new(folded)
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase);
                if actual.as_deref() != Some(ext.as_str()) {
                    continue;
                }
            }
            admitted.insert(folded.clone(), entry.key.clone());
        }
        let admitted_paths = admitted.keys().cloned().collect::<BTreeSet<_>>();
        if admitted_paths != effective {
            let missing = effective
                .difference(&admitted_paths)
                .take(8)
                .cloned()
                .collect::<Vec<_>>();
            let unexpected = admitted_paths
                .difference(&effective)
                .take(8)
                .cloned()
                .collect::<Vec<_>>();
            return Err(format!(
                "{label} admission/effective-VFS census mismatch: effective={} admitted={} missing={missing:?} unexpected={unexpected:?}",
                effective.len(),
                admitted_paths.len()
            )
            .into());
        }
        Ok(admitted.into_values().collect())
    }

    pub fn stats(&self) -> GameFileResolverStats {
        let stats = self.resolver.stats();
        GameFileResolverStats {
            overridden_file_count: stats.overridden_file_count,
            replace_path_excluded_file_count: stats.replace_path_excluded_file_count,
        }
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    fn resolves_to_mod_layer(&self, rel_path: &str) -> Result<bool> {
        Ok(self
            .resolver
            .resolve_optional_file_with_layer(rel_path)?
            .map(|resolved| resolved.layer_index > 0)
            .unwrap_or(false))
    }

    fn validate_admitted_result(&self, actual: &ResolvedGameFile) -> Result<()> {
        let Some(admission) = &self.admission else {
            return Ok(());
        };
        let expected = admission
            .entries
            .get(&content_identity_key(&actual.logical_path)?)
            .ok_or_else(|| {
                format!(
                    "replay content admission rejects unrecorded/default lookup {:?}",
                    actual.logical_path
                )
            })?;
        validate_used_content_match(actual, &expected.key)
    }

    fn resolve_admitted_dir(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
        recursive: bool,
    ) -> Result<Vec<(String, PathBuf)>> {
        let dir = content_identity_key(rel_dir)?;
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let wanted = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut out = Vec::new();
        for admitted in self
            .admission
            .as_ref()
            .expect("admission checked")
            .entries
            .values()
        {
            let key = &admitted.key;
            let path = content_identity_key(&key.normalized_game_relative_path)?;
            if !path.starts_with(&prefix) || (!recursive && immediate_parent(&path) != dir) {
                continue;
            }
            if let Some(ext) = &wanted {
                let actual = Path::new(&path)
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase);
                if actual.as_deref() != Some(ext.as_str()) {
                    continue;
                }
            }
            out.push((
                admitted.resolved.logical_path.clone(),
                admitted.snapshot_path.clone(),
            ));
        }
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }
}

fn admitted_effective_paths(
    admission: &ContentAdmission,
    rel_dir: &str,
    extension: Option<&str>,
    recursive: bool,
) -> Result<Vec<String>> {
    let dir = content_identity_key(rel_dir)?;
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    let wanted = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
    Ok(admission
        .effective_paths
        .iter()
        .filter(|path| {
            path.starts_with(&prefix)
                && (recursive || immediate_parent(path) == dir)
                && wanted.as_ref().is_none_or(|extension| {
                    Path::new(path)
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|actual| actual.eq_ignore_ascii_case(extension))
                })
        })
        .cloned()
        .collect())
}

pub fn sensitive_file_overrides(
    resolver: &GameFileResolver,
    rel_paths: &[&str],
) -> Result<SensitiveFileOverrideReport> {
    let _session = resolver.resolver.scan.session();
    let mut overridden = Vec::new();
    for rel_path in rel_paths {
        if resolver.resolves_to_mod_layer(rel_path)? {
            overridden.push(normalize_rel_path(rel_path));
        }
    }
    overridden.sort();
    overridden.dedup();
    Ok(SensitiveFileOverrideReport { overridden })
}

impl FileResolver {
    pub(crate) fn vanilla(vanilla_root: PathBuf) -> Self {
        Self {
            layers: vec![ResolverLayer {
                root: vanilla_root,
                manifest: None,
                replace_paths: BTreeSet::new(),
                source: GameContentSource {
                    kind: "base".to_string(),
                    source_id: "base".to_string(),
                    ordinal: 0,
                    descriptor_sha256: None,
                    archive_sha256: None,
                    replace_path: Vec::new(),
                    dependencies: Vec::new(),
                },
            }],
            stats: RefCell::new(ResolverStats::default()),
            scan: LayerScanCache::default(),
        }
    }

    pub(crate) fn with_mod_layers(vanilla_root: PathBuf, mods: Vec<ModContentLayer>) -> Self {
        let mut layers = vec![ResolverLayer {
            root: vanilla_root,
            manifest: None,
            replace_paths: BTreeSet::new(),
            source: GameContentSource {
                kind: "base".to_string(),
                source_id: "base".to_string(),
                ordinal: 0,
                descriptor_sha256: None,
                archive_sha256: None,
                replace_path: Vec::new(),
                dependencies: Vec::new(),
            },
        }];
        for (index, layer) in mods.into_iter().enumerate() {
            let replace_path = layer.manifest.replace_path.clone();
            let replace_paths = replace_path
                .iter()
                .map(|path| filesystem_case_key(&normalize_rel_dir(path)))
                .filter(|path| !path.is_empty())
                .collect();
            let source = GameContentSource {
                kind: layer.source_kind,
                source_id: layer.source_id,
                ordinal: layer.recorded_ordinal.unwrap_or(index + 1),
                descriptor_sha256: layer.descriptor_sha256,
                archive_sha256: layer.archive_sha256,
                replace_path,
                dependencies: layer.dependencies,
            };
            layers.push(ResolverLayer {
                root: layer.root,
                manifest: Some(layer.manifest),
                replace_paths,
                source,
            });
        }
        Self {
            layers,
            stats: RefCell::new(ResolverStats::default()),
            scan: LayerScanCache::default(),
        }
    }

    pub(crate) fn resolve_file(&self, rel: &str, label: &str) -> Result<PathBuf> {
        self.resolve_optional_file(rel)?
            .ok_or_else(|| format!("missing {label}: {rel}").into())
    }

    pub(crate) fn resolve_optional_file(&self, rel: &str) -> Result<Option<PathBuf>> {
        Ok(self
            .resolve_optional_file_with_layer(rel)?
            .map(|resolved| resolved.path))
    }

    fn resolve_optional_file_with_layer(&self, rel: &str) -> Result<Option<ResolvedLayerFile>> {
        let rel = checked_rel_path(rel)?;
        let folded = filesystem_case_key(&rel);
        let parent = immediate_parent(&folded);
        let mut candidate: Option<ResolvedLayerFile> = None;

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && layer.replace_paths.contains(&parent) {
                if let Some(replaced) = candidate.take() {
                    self.stats
                        .borrow_mut()
                        .replace_path_excluded_files
                        .insert(replaced.real_rel);
                }
            }
            if let Some((real_rel, path)) = self.scan.find_file_in_layer(&layer.root, &rel)? {
                if idx > 0 && candidate.is_some() {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                candidate = Some(ResolvedLayerFile {
                    real_rel,
                    path,
                    layer_index: idx,
                });
            }
        }
        Ok(candidate)
    }

    pub(crate) fn resolve_dir_files(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        let _session = self.scan.session();
        let rel_dir = checked_rel_path(rel_dir)?;
        let mut effective: BTreeMap<String, (String, PathBuf, usize)> = BTreeMap::new();
        let mut any_dir = false;
        let wanted_extension =
            extension.map(|ext| ext.trim_start_matches('.').to_ascii_lowercase());

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && !layer.replace_paths.is_empty() {
                let excluded = effective
                    .iter()
                    .filter(|(_, (real, _, _))| {
                        layer
                            .replace_paths
                            .contains(&immediate_parent(&filesystem_case_key(real)))
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in excluded {
                    if let Some((real, _, _)) = effective.remove(&key) {
                        self.stats
                            .borrow_mut()
                            .replace_path_excluded_files
                            .insert(real);
                    }
                }
            }

            let Some(dir) = self.scan.find_dir_in_layer(&layer.root, &rel_dir)? else {
                continue;
            };
            any_dir = true;
            let mut layer_keys = BTreeSet::new();
            for entry in self.scan.scan_dir(&dir)?.entries.iter() {
                let entry = layer_dir_entry(entry)?;
                let path = &entry.path;
                if !entry.is_file {
                    continue;
                }
                self.scan
                    .ensure_within_layer(&layer.root, path, "directory file")?;
                if let Some(ext) = &wanted_extension {
                    let actual = path
                        .extension()
                        .and_then(|value| value.to_str())
                        .map(|value| value.to_ascii_lowercase());
                    if actual.as_deref() != Some(ext.as_str()) {
                        continue;
                    }
                }
                let Some(file_name) = entry.name.as_deref() else {
                    continue;
                };
                let real_rel = if rel_dir.is_empty() {
                    file_name.to_string()
                } else {
                    format!("{rel_dir}/{file_name}")
                };
                let folded = real_rel.to_ascii_lowercase();
                if !layer_keys.insert(folded.clone()) {
                    return Err(format!(
                        "content layer {} has case-colliding file identity {real_rel}",
                        layer.source.source_id
                    )
                    .into());
                }
                if idx > 0 && effective.contains_key(&folded) {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                effective.insert(folded, (real_rel, path.clone(), idx));
            }
        }

        if !any_dir {
            return Err(format!("missing directory: {rel_dir}").into());
        }

        let mut out = effective
            .into_values()
            .map(|(rel, path, _)| (rel, path))
            .collect::<Vec<_>>();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }

    pub(crate) fn resolve_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<(String, PathBuf)>> {
        let _session = self.scan.session();
        let rel_dir = checked_rel_path(rel_dir)?;
        let mut effective: BTreeMap<String, (String, PathBuf, usize)> = BTreeMap::new();
        let mut any_dir = false;
        let wanted_extension =
            extension.map(|ext| ext.trim_start_matches('.').to_ascii_lowercase());

        for (idx, layer) in self.layers.iter().enumerate() {
            if idx > 0 && !layer.replace_paths.is_empty() {
                let excluded = effective
                    .iter()
                    .filter(|(_, (real, _, _))| {
                        layer
                            .replace_paths
                            .contains(&immediate_parent(&filesystem_case_key(real)))
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in excluded {
                    if let Some((real, _, _)) = effective.remove(&key) {
                        self.stats
                            .borrow_mut()
                            .replace_path_excluded_files
                            .insert(real);
                    }
                }
            }

            let Some(dir) = self.scan.find_dir_in_layer(&layer.root, &rel_dir)? else {
                continue;
            };
            any_dir = true;
            let mut layer_files = Vec::new();
            let mut visited = BTreeSet::new();
            collect_layer_files_recursive(
                &self.scan,
                &layer.root,
                &dir,
                &rel_dir,
                wanted_extension.as_deref(),
                &mut layer_files,
                &mut visited,
            )?;
            let mut layer_keys = BTreeSet::new();
            for (real_rel, path) in layer_files {
                let folded = real_rel.to_ascii_lowercase();
                if !layer_keys.insert(folded.clone()) {
                    return Err(format!(
                        "content layer {} has case-colliding file identity {real_rel}",
                        layer.source.source_id
                    )
                    .into());
                }
                if idx > 0 && effective.contains_key(&folded) {
                    self.stats
                        .borrow_mut()
                        .overridden_files
                        .insert(real_rel.clone());
                }
                effective.insert(folded, (real_rel, path, idx));
            }
        }

        if !any_dir {
            return Err(format!("missing directory: {rel_dir}").into());
        }

        let mut out = effective
            .into_values()
            .map(|(rel, path, _)| (rel, path))
            .collect::<Vec<_>>();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(out)
    }

    pub(crate) fn stats(&self) -> FileResolverStats {
        let stats = self.stats.borrow();
        FileResolverStats {
            overridden_file_count: stats.overridden_files.len(),
            replace_path_excluded_file_count: stats.replace_path_excluded_files.len(),
        }
    }

    pub(crate) fn mods(&self) -> Vec<BundleMod> {
        self.layers
            .iter()
            .filter_map(|layer| layer.manifest.clone())
            .collect()
    }
}

fn content_layers_from_launcher(
    game_root: &Path,
    userdata: &Path,
    recorded_order: Option<&[RecordedContentSource]>,
) -> Result<Vec<ModContentLayer>> {
    let userdata = require_dir(userdata, "the game user-data directory")?;
    let value: Value = serde_json::from_slice(
        &fs::read(userdata.join("dlc_load.json"))
            .map_err(|err| format!("could not read dlc_load.json: {err}"))?,
    )
    .map_err(|err| format!("could not parse dlc_load.json: {err}"))?;
    let enabled_mods = value
        .get("enabled_mods")
        .and_then(Value::as_array)
        .ok_or("dlc_load.json has no enabled_mods array")?;
    let disabled_dlcs = value
        .get("disabled_dlcs")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_str()
                        .filter(|value| !value.trim().is_empty())
                        .map(|value| normalize_rel_path(value).to_ascii_lowercase())
                        .ok_or_else(|| {
                            "dlc_load.json disabled_dlcs contains an invalid entry".to_string()
                        })
                })
                .collect::<std::result::Result<BTreeSet<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let cache_root = userdata.join(".battle-plan_vfs_cache");

    if let Some(order) = recorded_order {
        let mut enabled_ids = BTreeSet::new();
        for row in enabled_mods {
            let rel = row
                .as_str()
                .filter(|value| !value.trim().is_empty())
                .ok_or("dlc_load.json enabled_mods contains an invalid entry")?;
            let id = checked_rel_path(rel)?;
            if !enabled_ids.insert(id.to_ascii_lowercase()) {
                return Err(format!("dlc_load.json repeats enabled descriptor: {id}").into());
            }
        }
        let mut layers = Vec::with_capacity(order.len());
        let mut seen = BTreeSet::new();
        let mut previous = 0u32;
        for recorded in order {
            if recorded.ordinal == 0 || recorded.ordinal <= previous {
                return Err(
                    "recorded non-base content ordinals must be strictly increasing and nonzero"
                        .into(),
                );
            }
            previous = recorded.ordinal;
            let id = checked_rel_path(&recorded.source_id)?;
            let normalized_id = id.to_ascii_lowercase();
            if !seen.insert(normalized_id.clone()) {
                return Err(format!("recorded content layer is repeated: {id}").into());
            }
            let (descriptor_root, descriptor_path, kind) =
                if normalized_id.starts_with("integrated_dlc/") {
                    (
                        game_root.to_path_buf(),
                        path_from_slash_rel(game_root, &id),
                        "integrated_dlc",
                    )
                } else if normalized_id.starts_with("dlc/") {
                    if disabled_dlcs.contains(&normalized_id) {
                        return Err(
                            format!("recorded content layer is not active locally: {id}").into(),
                        );
                    }
                    (
                        game_root.to_path_buf(),
                        path_from_slash_rel(game_root, &id),
                        "dlc",
                    )
                } else if normalized_id.starts_with("mod/") {
                    if !enabled_ids.contains(&normalized_id) {
                        return Err(
                            format!("recorded content layer is not active locally: {id}").into(),
                        );
                    }
                    (userdata.clone(), path_from_slash_rel(&userdata, &id), "mod")
                } else {
                    return Err(format!(
                        "recorded content layer has unsupported descriptor identity: {id}"
                    )
                    .into());
                };
            let mut layer = content_layer_from_descriptor(
                recorded.ordinal as usize,
                &descriptor_path,
                &descriptor_root,
                &cache_root,
                kind,
                &id,
            )?;
            layer.recorded_ordinal = Some(recorded.ordinal as usize);
            layers.push(layer);
        }
        return Ok(layers);
    }

    let mut layers = Vec::new();

    for descriptor in discover_descriptors(game_root, "integrated_dlc", "dlc")? {
        let id = relative_source_id(game_root, &descriptor)?;
        layers.push(content_layer_from_descriptor(
            layers.len(),
            &descriptor,
            game_root,
            &cache_root,
            "integrated_dlc",
            &id,
        )?);
    }
    let mut planned = Vec::new();
    for descriptor in discover_descriptors(game_root, "dlc", "dlc")? {
        let id = relative_source_id(game_root, &descriptor)?;
        if disabled_dlcs.contains(&id.to_ascii_lowercase()) {
            continue;
        }
        planned.push(content_layer_from_descriptor(
            planned.len(),
            &descriptor,
            game_root,
            &cache_root,
            "dlc",
            &id,
        )?);
    }
    let mut seen_mods = BTreeSet::new();
    for row in enabled_mods {
        let rel = row
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .ok_or("dlc_load.json enabled_mods contains an invalid entry")?;
        let id = checked_rel_path(rel)?;
        if !seen_mods.insert(id.to_ascii_lowercase()) {
            return Err(format!("dlc_load.json repeats enabled descriptor: {id}").into());
        }
        let descriptor = path_from_slash_rel(&userdata, &id);
        planned.push(content_layer_from_descriptor(
            planned.len(),
            &descriptor,
            &userdata,
            &cache_root,
            "mod",
            &id,
        )?);
    }

    layers.extend(engine_mount_plan(planned)?);
    for (index, layer) in layers.iter_mut().enumerate() {
        layer.manifest.load_order = layer.recorded_ordinal.unwrap_or(index + 1);
    }
    Ok(layers)
}

fn discover_descriptors(root: &Path, rel_dir: &str, extension: &str) -> Result<Vec<PathBuf>> {
    let rel_dir = checked_rel_path(rel_dir)?;
    let start = path_from_slash_rel(root, &rel_dir);
    if !start.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut visited = BTreeSet::new();
    collect_layer_files_recursive(
        &LayerScanCache::default(),
        root,
        &start,
        &rel_dir,
        Some(extension),
        &mut files,
        &mut visited,
    )?;
    let mut out = files.into_iter().map(|(_, path)| path).collect::<Vec<_>>();
    out.sort_by(|left, right| {
        left.to_string_lossy()
            .as_bytes()
            .cmp(right.to_string_lossy().as_bytes())
    });
    Ok(out)
}

fn engine_mount_plan(layers: Vec<ModContentLayer>) -> Result<Vec<ModContentLayer>> {
    if layers.is_empty() {
        return Ok(layers);
    }
    let mut priorities = layers
        .iter()
        .map(|layer| match layer.source_kind.as_str() {
            "dlc" => Ok(0i32),
            "mod" => Ok(10_000i32),
            other => Err(format!("unsupported a729 planned layer kind: {other}")),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for round in 0..layers.len() {
        let mut changed = false;
        for dependent in 0..layers.len() {
            let dependent_priority = priorities[dependent];
            for dependency_name in &layers[dependent].dependencies {
                for dependency in 0..layers.len() {
                    if layers[dependency].manifest.name.as_deref() != Some(dependency_name.as_str())
                    {
                        continue;
                    }
                    if dependent_priority <= priorities[dependency] {
                        priorities[dependency] -= 1;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
        if round + 1 == layers.len() {
            return Err("active DLC/mod dependencies contain a priority cycle".into());
        }
    }

    let mut indexed = layers
        .into_iter()
        .enumerate()
        .map(|(index, layer)| (priorities[index], layer))
        .collect::<Vec<_>>();
    indexed.sort_by(|left, right| {
        left.0.cmp(&right.0).then_with(|| {
            left.1
                .source_id
                .as_bytes()
                .cmp(right.1.source_id.as_bytes())
        })
    });
    Ok(indexed.into_iter().map(|(_, layer)| layer).collect())
}

fn relative_source_id(root: &Path, path: &Path) -> Result<String> {
    let rel = path
        .strip_prefix(root)
        .map_err(|_| "descriptor escaped its content root")?;
    checked_rel_path(&rel.to_string_lossy())
}

fn content_layer_from_descriptor(
    load_order: usize,
    descriptor_path: &Path,
    descriptor_root: &Path,
    cache_root: &Path,
    kind: &str,
    source_id: &str,
) -> Result<ModContentLayer> {
    if !descriptor_path.is_file() {
        return Err(format!("enabled content descriptor not found: {source_id}").into());
    }
    let descriptor = parse_mod_descriptor(descriptor_path)?;
    if descriptor.path.is_some() && descriptor.archive.is_some() {
        return Err(format!("descriptor has both path and archive: {source_id}").into());
    }
    let (root, archive_sha256) = if kind == "integrated_dlc" {
        let root = descriptor_path
            .parent()
            .ok_or("integrated DLC descriptor has no containing directory")?
            .to_path_buf();
        (
            require_dir(&root, "integrated DLC content directory")?,
            None,
        )
    } else if let Some(value) = descriptor.archive.as_deref() {
        let archive = resolve_descriptor_content_root(value, descriptor_root);
        if !archive.is_file() {
            return Err(format!("required content archive is missing: {source_id}").into());
        }
        let archive_bytes = fs::read(&archive)?;
        let sha = Sha256::hex(&archive_bytes);
        (
            materialize_zip_archive(&archive_bytes, cache_root, &sha)?,
            Some(sha),
        )
    } else if let Some(value) = descriptor.path.as_deref() {
        let root = resolve_descriptor_content_root(value, descriptor_root);
        (require_dir(&root, "descriptor content directory")?, None)
    } else {
        let root = descriptor_path
            .parent()
            .ok_or("descriptor has no containing directory")?
            .to_path_buf();
        (require_dir(&root, "descriptor content directory")?, None)
    };
    let mut manifest = descriptor.to_bundle_mod(load_order);
    if kind == "integrated_dlc" {
        manifest.replace_path.clear();
    }
    Ok(ModContentLayer {
        root,
        manifest,
        source_kind: kind.to_string(),
        source_id: normalize_rel_path(source_id),
        descriptor_sha256: Some(semantic_descriptor_sha256_from_parsed(&descriptor)),
        archive_sha256,
        dependencies: descriptor.dependencies.clone(),
        recorded_ordinal: None,
    })
}

struct StagingDirectoryGuard {
    path: PathBuf,
    armed: bool,
}

impl Drop for StagingDirectoryGuard {
    fn drop(&mut self) {
        if self.armed {
            remove_tree_allow_readonly(&self.path);
        }
    }
}

fn remove_tree_allow_readonly(path: &Path) {
    #[cfg(windows)]
    fn clear(path: &Path) {
        let Ok(metadata) = fs::metadata(path) else {
            return;
        };
        if metadata.is_dir() {
            if let Ok(entries) = fs::read_dir(path) {
                for entry in entries.flatten() {
                    clear(&entry.path());
                }
            }
        } else {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
    #[cfg(windows)]
    clear(path);
    let _ = fs::remove_dir_all(path);
}

fn collect_extracted_members(
    dir: &Path,
    prefix: &str,
    found: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let metadata = fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() {
            collect_extracted_members(&entry.path(), &rel, found)?;
        } else {
            found.insert(rel);
        }
    }
    Ok(())
}

fn materialize_zip_archive(
    archive_bytes: &[u8],
    cache_root: &Path,
    sha256: &str,
) -> Result<PathBuf> {
    let destination = cache_root.join(sha256);
    let marker = destination.join(".complete-sha256");
    let content_root = destination.join("content");
    if marker.is_file() && content_root.is_dir() && fs::read_to_string(&marker)?.trim() == sha256 {
        return Ok(content_root);
    }
    fs::create_dir_all(cache_root)?;
    let staging = cache_root.join(format!(".{sha256}.{}.building", std::process::id()));
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    fs::create_dir_all(&staging)?;
    let mut staging_guard = StagingDirectoryGuard {
        path: staging.clone(),
        armed: true,
    };
    let staging_content = staging.join("content");
    fs::create_dir_all(&staging_content)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(archive_bytes))
        .map_err(|err| format!("could not open content archive: {err}"))?;
    if zip.is_empty() {
        return Err("content archive is empty".into());
    }
    const MAX_ARCHIVE_REGULAR_FILES: usize = 1 << 18;
    const MAX_ARCHIVE_UNCOMPRESSED_BYTES: u64 = 2 << 30;
    let mut folded_members = BTreeSet::new();
    let mut intended_members = BTreeSet::new();
    let mut regular_file_count = 0usize;
    let mut total_uncompressed_bytes = 0u64;
    for index in 0..zip.len() {
        let mut member = zip
            .by_index(index)
            .map_err(|err| format!("could not read content archive member: {err}"))?;
        if member.encrypted() {
            return Err("content archive has an encrypted member".into());
        }
        if member.is_symlink() {
            return Err("content archive has a symbolic-link member".into());
        }
        if member.is_dir() {
            continue;
        }
        if !member.is_file() {
            return Err("content archive has an unsupported member type".into());
        }
        let raw = member.name().replace('\\', "/");
        let rel = checked_rel_path(&raw)?;
        if rel.is_empty() {
            continue;
        }
        if !folded_members.insert(windows_collision_key(&rel)?) {
            return Err(format!("archive has case-colliding member: {rel}").into());
        }
        intended_members.insert(rel.clone());
        let declared_size = member.size();
        if declared_size > MAX_CONTENT_PAYLOAD_BYTES {
            return Err(format!(
                "archive member {rel} exceeds the {MAX_CONTENT_PAYLOAD_BYTES}-byte payload ceiling"
            )
            .into());
        }
        total_uncompressed_bytes = total_uncompressed_bytes
            .checked_add(declared_size)
            .ok_or("archive declared uncompressed size overflows")?;
        if total_uncompressed_bytes > MAX_ARCHIVE_UNCOMPRESSED_BYTES {
            return Err(format!(
                "archive exceeds the {MAX_ARCHIVE_UNCOMPRESSED_BYTES}-byte uncompressed ceiling"
            )
            .into());
        }
        let output = path_from_slash_rel(&staging_content, &rel);
        regular_file_count += 1;
        if regular_file_count > MAX_ARCHIVE_REGULAR_FILES {
            return Err(
                format!("archive exceeds the {MAX_ARCHIVE_REGULAR_FILES}-file ceiling").into(),
            );
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&output)?;
        let copied = io::copy(
            &mut (&mut member).take(MAX_CONTENT_PAYLOAD_BYTES.saturating_add(1)),
            &mut file,
        )?;
        if copied != declared_size || copied > MAX_CONTENT_PAYLOAD_BYTES {
            return Err(format!(
                "archive member {rel} payload length {copied} disagrees with its declared length {declared_size}"
            )
            .into());
        }
    }
    if regular_file_count == 0 {
        return Err("content archive has no regular files".into());
    }
    let mut extracted_members = BTreeSet::new();
    collect_extracted_members(&staging_content, "", &mut extracted_members)?;
    if extracted_members != intended_members {
        let missing: Vec<&str> = intended_members
            .difference(&extracted_members)
            .map(String::as_str)
            .collect();
        let unexpected: Vec<&str> = extracted_members
            .difference(&intended_members)
            .map(String::as_str)
            .collect();
        return Err(format!(
            "archive extraction did not reproduce its declared members: {} declared name(s) did not \
             appear on disk under that spelling ({:?}), {} unexpected path(s) appeared ({:?})",
            missing.len(),
            &missing[..missing.len().min(8)],
            unexpected.len(),
            &unexpected[..unexpected.len().min(8)]
        )
        .into());
    }
    fs::write(staging.join(".complete-sha256"), sha256)?;
    if destination.exists() {
        if marker.is_file()
            && content_root.is_dir()
            && fs::read_to_string(&marker)?.trim() == sha256
        {
            let _ = fs::remove_dir_all(&staging);
            return Ok(content_root);
        }
        fs::remove_dir_all(&destination)?;
    }
    match fs::rename(&staging, &destination) {
        Ok(()) => {
            staging_guard.armed = false;
        }
        Err(_err)
            if marker.is_file()
                && content_root.is_dir()
                && fs::read_to_string(&marker)?.trim() == sha256 =>
        {
            let _ = fs::remove_dir_all(&staging);
            staging_guard.armed = false;
        }
        Err(err) => return Err(err.into()),
    }
    Ok(content_root)
}

fn mods_from_dlc_load(userdata: &Path) -> Result<Vec<ModContentLayer>> {
    let dlc_load = userdata.join("dlc_load.json");
    let text = fs::read_to_string(&dlc_load)
        .map_err(|err| format!("could not read dlc_load.json: {err}"))?;
    let value = serde_json::from_str::<Value>(&text)
        .map_err(|err| format!("could not parse dlc_load.json: {err}"))?;
    let enabled_mods = value
        .get("enabled_mods")
        .and_then(|value| value.as_array())
        .ok_or("dlc_load.json has no enabled_mods array")?;

    let receipt: Ui14EngineVfsReceipt = serde_json::from_slice(
        &fs::read(userdata.join(UI14_ENGINE_VFS_FILE))
            .map_err(|err| format!("could not read required {UI14_ENGINE_VFS_FILE}: {err}"))?,
    )
    .map_err(|err| format!("could not parse {UI14_ENGINE_VFS_FILE}: {err}"))?;
    if receipt.schema != UI14_ENGINE_VFS_SCHEMA {
        return Err(format!(
            "unsupported {UI14_ENGINE_VFS_FILE} schema: {}",
            receipt.schema
        )
        .into());
    }
    if receipt.executable_sha256.len() != 64
        || !receipt
            .executable_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(format!("{UI14_ENGINE_VFS_FILE} has invalid executable SHA-256").into());
    }
    let mut enabled = BTreeMap::new();
    let mut ui15_enabled = Vec::new();
    for item in enabled_mods {
        let rel = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or("dlc_load.json enabled_mods contains a non-string or empty entry")?;
        let id = normalize_rel_path(rel);
        if id.starts_with("_ui15_dlc/") {
            ui15_enabled.push((id, path_from_slash_rel(userdata, rel)));
            continue;
        }
        if enabled
            .insert(id.clone(), path_from_slash_rel(userdata, rel))
            .is_some()
        {
            return Err(format!("dlc_load.json repeats enabled descriptor: {id}").into());
        }
    }
    if receipt.layers.len() != enabled.len() {
        return Err(format!(
            "{UI14_ENGINE_VFS_FILE} layer count does not match enabled descriptors"
        )
        .into());
    }
    let mut descriptor_ordinals = BTreeSet::new();
    let mut mount_ordinals = BTreeSet::new();
    let mut ordered = Vec::new();
    for row in &receipt.layers {
        let id = normalize_rel_path(&row.id);
        let descriptor = enabled
            .get(&id)
            .ok_or_else(|| format!("{UI14_ENGINE_VFS_FILE} names non-enabled descriptor: {id}"))?;
        if !descriptor.is_file() {
            return Err(format!("mod descriptor not found: {id}").into());
        }
        if !descriptor_ordinals.insert(row.engine_descriptor_ordinal)
            || !mount_ordinals.insert(row.engine_mount_ordinal)
        {
            return Err(
                format!("{UI14_ENGINE_VFS_FILE} contains duplicate engine ordinals").into(),
            );
        }
        if Sha256::hex(&fs::read(descriptor)?) != row.descriptor_sha256.to_ascii_lowercase() {
            return Err(format!("{UI14_ENGINE_VFS_FILE} descriptor hash mismatch: {id}").into());
        }
        ordered.push((
            row.engine_mount_ordinal,
            row.engine_descriptor_ordinal,
            id,
            descriptor,
        ));
    }
    ordered.sort_by_key(|row| (row.0, row.1));
    let mut mods = Vec::with_capacity(ui15_enabled.len() + ordered.len());
    let mut ordered_ids = Vec::with_capacity(ui15_enabled.len() + ordered.len());
    for (id, descriptor) in &ui15_enabled {
        let layer = mod_layer_from_descriptor(mods.len(), descriptor, userdata)?
            .ok_or("enabled UI15 descriptor produced no content layer")?;
        mods.push(layer);
        ordered_ids.push(id.clone());
    }
    for (_, _, _, descriptor) in &ordered {
        let layer = mod_layer_from_descriptor(mods.len(), descriptor, userdata)?
            .ok_or("enabled mod descriptor produced no content layer")?;
        mods.push(layer);
    }
    ordered_ids.extend(ordered.iter().map(|row| row.2.clone()));
    validate_ui14_effective_winners(&mods, &ordered_ids, &receipt.effective_winners)?;
    Ok(mods)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedContentSource {
    pub ordinal: u32,
    pub source_id: String,
}

pub fn stable_dlc_product_id(source_id: &str) -> Option<String> {
    let normalized = checked_rel_path(source_id).ok()?;
    let mut parts = normalized.split('/');
    match (parts.next(), parts.next()) {
        (Some("dlc" | "integrated_dlc"), Some(folder)) if !folder.is_empty() => {
            Some(folder.to_ascii_lowercase())
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameContentSource {
    pub kind: String,
    pub source_id: String,
    pub ordinal: usize,
    pub descriptor_sha256: Option<String>,
    pub archive_sha256: Option<String>,
    pub replace_path: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedGameFile {
    pub logical_path: String,
    pub source: GameContentSource,
    pub source_relative_path: String,
    pub byte_len: u64,
    pub sha256: String,
    #[serde(skip)]
    pub local_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredGameContent {
    pub logical_path: String,
    pub sha256: String,
    pub byte_len: u64,
}

pub struct RestrictedGameFileResolver<'a> {
    resolver: &'a GameFileResolver,
    allowed: BTreeMap<String, RequiredGameContent>,
}

#[derive(Clone, Debug)]
pub struct PlaybackEnvironment {
    pub source_game: SourceGameIdentity,
    pub source_profile: SourceProfileIdentity,
    pub dlc_selection: Vec<DlcSelection>,
    pub localisation_language: String,
    pub settings: BTreeMap<String, String>,
}

fn validate_ui14_effective_winners(
    mods: &[ModContentLayer],
    ordered_ids: &[String],
    rows: &[Ui14EffectiveWinner],
) -> Result<()> {
    let mut candidates: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, layer) in mods.iter().enumerate() {
        let mut files = Vec::new();
        let mut visited = BTreeSet::new();
        collect_layer_files_recursive(
            &LayerScanCache::default(),
            &layer.root,
            &layer.root,
            "",
            None,
            &mut files,
            &mut visited,
        )?;
        let mut layer_keys = BTreeSet::new();
        for (rel, _) in files {
            let folded = normalize_rel_path(&rel).to_ascii_lowercase();
            if !layer_keys.insert(folded.clone()) {
                return Err(format!("case-colliding path inside UI14 layer: {rel}").into());
            }
            candidates.entry(folded).or_default().push(idx);
        }
    }
    let collision_keys = candidates
        .iter()
        .filter_map(|(key, owners)| (owners.len() > 1).then_some(key.clone()))
        .collect::<BTreeSet<_>>();
    let mut winner_rows = BTreeMap::new();
    for row in rows {
        let key = normalize_rel_path(&row.relpath).to_ascii_lowercase();
        if !collision_keys.contains(&key) {
            return Err(format!(
                "{UI14_ENGINE_VFS_FILE} has non-collision winner row: {}",
                row.relpath
            )
            .into());
        }
        if winner_rows.insert(key, row).is_some() {
            return Err(format!("{UI14_ENGINE_VFS_FILE} repeats an effective-winner row").into());
        }
    }
    if winner_rows.keys().cloned().collect::<BTreeSet<_>>() != collision_keys {
        return Err(
            format!("{UI14_ENGINE_VFS_FILE} does not cover every loose-mod collision").into(),
        );
    }
    let resolver = FileResolver::with_mod_layers(PathBuf::new(), mods.to_vec());
    for (key, row) in winner_rows {
        let relpath = normalize_rel_path(&row.relpath);
        let resolved = resolver
            .resolve_optional_file_with_layer(&relpath)?
            .ok_or_else(|| format!("UI14 collision disappeared during resolution: {key}"))?;
        let mod_index = resolved
            .layer_index
            .checked_sub(1)
            .ok_or_else(|| format!("UI14 collision unexpectedly resolved to vanilla: {key}"))?;
        let actual_id = &ordered_ids[mod_index];
        if normalize_rel_path(&row.winner_id) != *actual_id {
            return Err(format!(
                "UI14 effective winner mismatch for {key}: receipt={} resolved={actual_id}",
                row.winner_id
            )
            .into());
        }
        if Sha256::hex(&read_bounded_file(&resolved.path, "UI14 effective winner")?)
            != row.sha256.to_ascii_lowercase()
        {
            return Err(format!("UI14 effective winner hash mismatch for {key}").into());
        }
    }
    Ok(())
}

fn explicit_mod_layer(
    load_order: usize,
    root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ModContentLayer> {
    let root = require_dir(root, "mod content directory").map_err(|error| {
        format!(
            "explicit --mod layer {load_order} refused: mod content directory not found: {} ({error})",
            path_basename(root)
        )
    })?;
    let descriptor = root.join("descriptor.mod");
    if !descriptor.is_file() {
        warnings.push(format!(
            "explicit --mod layer {load_order} ({}) has no descriptor.mod: admitted as a loose override directory with no replace_path and no descriptor identity",
            path_basename(&root)
        ));
        return Ok(ModContentLayer {
            source_kind: "mod".to_string(),
            source_id: path_basename(&root),
            descriptor_sha256: None,
            archive_sha256: None,
            dependencies: Vec::new(),
            recorded_ordinal: None,
            manifest: BundleMod {
                load_order,
                name: root
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string()),
                version: None,
                supported_version: None,
                workshop_id: None,
                descriptor: None,
                replace_path: Vec::new(),
            },
            root,
        });
    }
    let parsed_descriptor = parse_mod_descriptor(&descriptor).map_err(|error| {
        format!(
            "explicit --mod layer {load_order} refused: {} descriptor.mod is unusable: {error}",
            path_basename(&root)
        )
    })?;
    Ok(ModContentLayer {
        source_kind: "mod".to_string(),
        source_id: parsed_descriptor
            .descriptor
            .clone()
            .unwrap_or_else(|| path_basename(&root)),
        descriptor_sha256: Some(semantic_descriptor_sha256_from_parsed(&parsed_descriptor)),
        archive_sha256: None,
        dependencies: parsed_descriptor.dependencies.clone(),
        recorded_ordinal: None,
        root,
        manifest: parsed_descriptor.to_bundle_mod(load_order),
    })
}

fn path_basename(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| "<unknown>".to_string())
}

fn mod_layer_from_descriptor(
    load_order: usize,
    descriptor_path: &Path,
    userdata: &Path,
) -> Result<Option<ModContentLayer>> {
    if !descriptor_path.is_file() {
        return Err(format!(
            "mod descriptor not found: {}",
            descriptor_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("<unknown>")
        )
        .into());
    }
    let descriptor = parse_mod_descriptor(descriptor_path)?;
    let Some(path_value) = descriptor.path.as_ref() else {
        if descriptor.archive.is_some() {
            return Err(format!(
                "archive-only mod skipped: {}",
                descriptor
                    .name
                    .clone()
                    .or_else(|| descriptor.descriptor.clone())
                    .unwrap_or_else(|| "<unknown>".to_string())
            )
            .into());
        }
        return Err(format!(
            "mod path missing in descriptor: {}",
            descriptor
                .descriptor
                .clone()
                .unwrap_or_else(|| "<unknown>".to_string())
        )
        .into());
    };
    let root = resolve_descriptor_content_root(path_value, userdata);
    if !root.is_dir() {
        return Err(format!(
            "mod path unresolvable, skipped: {}",
            descriptor.name.as_deref().unwrap_or("<unnamed>")
        )
        .into());
    }
    Ok(Some(ModContentLayer {
        root,
        manifest: descriptor.to_bundle_mod(load_order),
        source_kind: "mod".to_string(),
        source_id: normalize_rel_path(
            &descriptor_path
                .strip_prefix(userdata)
                .unwrap_or(descriptor_path)
                .to_string_lossy(),
        ),
        descriptor_sha256: Some(semantic_descriptor_sha256_from_parsed(&descriptor)),
        archive_sha256: None,
        dependencies: descriptor.dependencies.clone(),
        recorded_ordinal: None,
    }))
}

#[derive(Clone, Debug, Default)]
struct ParsedModDescriptor {
    name: Option<String>,
    path: Option<String>,
    archive: Option<String>,
    version: Option<String>,
    supported_version: Option<String>,
    remote_file_id: Option<String>,
    replace_path: Vec<String>,
    dependencies: Vec<String>,
    descriptor: Option<String>,
}

impl ParsedModDescriptor {
    fn to_bundle_mod(&self, load_order: usize) -> BundleMod {
        BundleMod {
            load_order,
            name: self.name.clone(),
            version: self.version.clone(),
            supported_version: self.supported_version.clone(),
            workshop_id: self.remote_file_id.clone(),
            descriptor: self.descriptor.clone(),
            replace_path: self.replace_path.clone(),
        }
    }
}

fn parse_mod_descriptor(path: &Path) -> Result<ParsedModDescriptor> {
    let text = read_latin1(path)?;
    let mut descriptor = parse_mod_descriptor_text(&text);
    let mut seen_replace_paths = BTreeSet::new();
    for replace_path in &descriptor.replace_path {
        let checked = checked_rel_path(replace_path)?;
        if checked.is_empty()
            || checked != *replace_path
            || !seen_replace_paths.insert(filesystem_case_key(&checked))
        {
            return Err(format!(
                "descriptor has an unsafe, non-canonical, or repeated replace_path: {}",
                path_basename(path)
            )
            .into());
        }
    }
    descriptor.descriptor = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string());
    Ok(descriptor)
}

fn parse_mod_descriptor_text(text: &str) -> ParsedModDescriptor {
    let mut out = ParsedModDescriptor::default();
    for raw_line in text.lines() {
        let line = raw_line
            .split_once('#')
            .map(|(head, _)| head)
            .unwrap_or(raw_line)
            .trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(value) = parse_descriptor_scalar(value.trim()) else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "name" => out.name = Some(value),
            "path" => out.path = Some(value),
            "archive" => out.archive = Some(value),
            "version" => out.version = Some(value),
            "supported_version" => out.supported_version = Some(value),
            "remote_file_id" => out.remote_file_id = Some(value),
            "replace_path" => {
                let normalized = normalize_rel_dir(&value);
                if !normalized.is_empty() {
                    out.replace_path.push(normalized);
                }
            }
            _ => {}
        }
    }
    out.dependencies = parse_descriptor_quoted_list(text, "dependencies");
    out
}

pub fn semantic_descriptor_sha256(text: &str) -> String {
    semantic_descriptor_sha256_from_parsed(&parse_mod_descriptor_text(text))
}

fn semantic_descriptor_sha256_from_parsed(descriptor: &ParsedModDescriptor) -> String {
    let replace_path = descriptor
        .replace_path
        .iter()
        .map(|value| normalize_rel_dir(value).to_ascii_lowercase())
        .collect::<Vec<_>>();
    let dependencies = descriptor
        .dependencies
        .iter()
        .map(|value| value.trim().to_string())
        .collect::<Vec<_>>();
    let canonical = serde_json::json!({
        "schema": "hoi4.semantic-descriptor.v1",
        "name": descriptor.name.as_deref().map(str::trim),
        "version": descriptor.version.as_deref().map(str::trim),
        "supported_version": descriptor.supported_version.as_deref().map(str::trim),
        "remote_file_id": descriptor.remote_file_id.as_deref().map(str::trim),
        "replace_path": replace_path,
        "dependencies": dependencies,
    });
    Sha256::hex(&serde_json::to_vec(&canonical).expect("semantic descriptor JSON"))
}

fn parse_descriptor_quoted_list(text: &str, key: &str) -> Vec<String> {
    let lower = text.to_ascii_lowercase();
    let Some(key_start) = lower.find(&key.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let rest = &text[key_start + key.len()..];
    let Some(open) = rest.find('{') else {
        return Vec::new();
    };
    let body = &rest[open + 1..];
    let Some(close) = body.find('}') else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut remaining = &body[..close];
    while let Some(start) = remaining.find('"') {
        remaining = &remaining[start + 1..];
        let Some(end) = remaining.find('"') else {
            break;
        };
        let value = remaining[..end].trim();
        if !value.is_empty() {
            values.push(value.to_string());
        }
        remaining = &remaining[end + 1..];
    }
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn parse_descriptor_scalar(value: &str) -> Option<String> {
    if value.starts_with('{') {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    value
        .split_whitespace()
        .next()
        .map(|value| value.trim_matches('"').to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_descriptor_content_root(value: &str, userdata: &Path) -> PathBuf {
    let path = PathBuf::from(value.replace('\\', "/"));
    if path.is_absolute() {
        path
    } else {
        path_from_slash_rel(userdata, value)
    }
}

fn path_from_slash_rel(root: &Path, rel: &str) -> PathBuf {
    normalize_rel_path(rel)
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

pub(crate) fn modset_id(mods: &[BundleMod]) -> String {
    if mods.is_empty() {
        return "vanilla".to_string();
    }
    let raw = canonical_modset_json(mods);
    sha1_hex_16(raw.as_bytes())
}

fn canonical_modset_json(mods: &[BundleMod]) -> String {
    let mut out = String::from("[");
    for (idx, item) in mods.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        let workshop = item.workshop_id.as_deref().unwrap_or_default().trim();
        let ident = if workshop.is_empty() {
            format!("name:{}", item.name.as_deref().unwrap_or_default().trim())
        } else {
            format!("workshop:{workshop}")
        };
        let version = item.version.as_deref().unwrap_or_default().trim();
        out.push('[');
        out.push_str(&json_string_ascii(&ident));
        out.push(',');
        out.push_str(&json_string_ascii(version));
        out.push(']');
    }
    out.push(']');
    out
}

fn json_string_ascii(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch < ' ' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch if ch.is_ascii() => out.push(ch),
            ch => {
                let code = ch as u32;
                if code <= 0xffff {
                    out.push_str(&format!("\\u{code:04x}"));
                } else {
                    let value = code - 0x1_0000;
                    let high = 0xd800 + (value >> 10);
                    let low = 0xdc00 + (value & 0x3ff);
                    out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

fn sha1_hex_16(input: &[u8]) -> String {
    let mut h0 = 0x6745_2301u32;
    let mut h1 = 0xefcd_ab89u32;
    let mut h2 = 0x98ba_dcfeu32;
    let mut h3 = 0x1032_5476u32;
    let mut h4 = 0xc3d2_e1f0u32;

    let bit_len = (input.len() as u64) * 8;
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut w = [0u32; 80];
        for (idx, word) in w.iter_mut().take(16).enumerate() {
            let start = idx * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for idx in 16..80 {
            w[idx] = (w[idx - 3] ^ w[idx - 8] ^ w[idx - 14] ^ w[idx - 16]).rotate_left(1);
        }

        let mut a = h0;
        let mut b = h1;
        let mut c = h2;
        let mut d = h3;
        let mut e = h4;
        for (idx, word) in w.iter().enumerate() {
            let (f, k) = match idx {
                0..=19 => ((b & c) | ((!b) & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    format!("{h0:08x}{h1:08x}{h2:08x}{h3:08x}{h4:08x}")
        .chars()
        .take(16)
        .collect()
}

/// One resolver's memo of the content layers it has already looked at.
///
/// WHAT IT SERVES NOW. Directory ENUMERATION only — `resolve_dir_files`,
/// `resolve_dir_files_recursive`, `collect_layer_files_recursive` and the
/// content census built on them. Those have to `read_dir` whatever the lookup
/// rule is, and they call the same directories repeatedly within one bulk
/// operation, which is what the memo is for.
///
/// POINT LOOKUPS NO LONGER TOUCH IT. `find_file_in_layer` and
/// `find_dir_in_layer` ask the volume about the components they were handed
/// (`resolve_literal_layer_path`) and enumerate nothing at all, so there is no
/// scan for the memo to serve them; `assert_memo_changes_nothing` pins that at
/// zero scans. Only `canonicalize` is still shared with them, for the winner.
///
/// WHY IT EXISTS AT ALL. Before it, `find_file_in_layer` ran a full
/// `fs::read_dir` of the containing directory and `find_dir_in_layer` a full
/// `fs::read_dir` of EVERY ancestor of it, on EVERY lookup, in EVERY layer,
/// with nothing memoized — and inside those loops it issued `is_file`/`is_dir`
/// and, for every file entry, `fs::canonicalize` of both the layer root and the
/// entry, all before any name was compared. `gfx/interface` carries 1,420
/// immediate entries and a real modset is ~25 layers, so a single admitted file
/// cost tens of thousands of syscalls. Lane JY measured the result at about
/// 40 ms PER ADMITTED FILE, tracking file COUNT rather than bytes — raw data in
/// `extractor/native/runs/jy_cloud_battery/xui23_before_after_raw.txt`. The
/// admission path resolves one file per effective path (see
/// `GameFileResolver::with_content_admission`), so the cost was quadratic in
/// directory size across a whole install. The literal lookup removed that cost
/// from the point-lookup path outright rather than memoizing it; the memo is
/// what keeps the ENUMERATING callers, which cannot avoid it, affordable.
///
/// WHAT MAKES ONE SCAN ENOUGH, AND WHERE IT STOPS BEING ENOUGH. The memo is
/// armed only inside a RESOLUTION SESSION — one bulk operation whose own body
/// is provably read-only over every layer root, so the effective VFS cannot
/// move underneath it. [`LayerScanCache::session`] arms it, the returned guard
/// disarms and CLEARS it, and nesting is counted so an inner session does not
/// discard an outer one's scans.
///
/// Outside a session every lookup issues its syscalls exactly as it did before
/// this type existed. That is deliberate and it is NOT a missed opportunity: a
/// resolver is a long-lived object, the install it mounts is a directory the
/// user (or Steam, or a mod manager) can rewrite at any moment, and
/// `later_selected_dlc_replace_path_blocks_lower_exact_parent_fallback` is a
/// live example of a caller that writes into a layer between two resolves and
/// requires the second to see it. Assuming a whole-instance snapshot would
/// have answered that caller from stale scans. A session is the narrow claim
/// that can actually be proven at each site that makes it, so that is the only
/// claim made. `Clone` starts a fresh, empty, unarmed memo for the same
/// reason: a clone's session accounting is its own.
///
/// WHAT IT DOES NOT CHANGE. The memo records precisely what the pre-memo loops
/// computed per entry — `is_file`/`is_dir` from one `fs::metadata` (which is
/// what `Path::is_file`/`Path::is_dir` each call), the case-preserved name, and
/// the `fs::canonicalize` outcome — and every
/// caller still walks the entries in `fs::read_dir` order running its own
/// checks, in its own order, with its own error label. No refusal is skipped,
/// weakened, reordered or renamed; only the syscalls behind them are reused.
#[derive(Debug, Default)]
struct LayerScanCache {
    /// Nesting depth of the armed resolution sessions. Zero means every lookup
    /// goes to the filesystem, exactly as it did before this memo existed.
    session_depth: Cell<usize>,
    /// How many `fs::read_dir` calls this memo has actually issued, ever. The
    /// memo is otherwise invisible by construction, so this is the only way to
    /// assert that it ENGAGED — without it, a differential comparing memo-off
    /// against memo-on passes trivially if the memo never caches anything.
    scans_taken: Cell<usize>,
    /// `fs::canonicalize` outcome per path, error kept as its `Display` text so
    /// each call site rebuilds the exact message its own label produces.
    canonical: RefCell<HashMap<PathBuf, std::result::Result<PathBuf, String>>>,
    /// `Path::is_dir` for a layer root, which `find_dir_in_layer` tests once
    /// per component of every lookup.
    root_is_dir: RefCell<HashMap<PathBuf, bool>>,
    /// `fs::read_dir` of one directory, keyed by its absolute path: what a
    /// scan records does not depend on which layer root reached it, so the
    /// per-file lookup, the directory listing and the recursive census all
    /// share one scan of it.
    dirs: RefCell<HashMap<PathBuf, Rc<LayerDirScan>>>,
}

/// A clone resolves the same layers but keeps its own session accounting, so
/// it starts with nothing scanned and no session armed.
impl Clone for LayerScanCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Arms one [`LayerScanCache`] for the life of one bulk resolution and clears
/// every scan it took when that resolution ends — including on the error path,
/// because it clears in `Drop`.
struct ScanSession<'a> {
    cache: &'a LayerScanCache,
}

impl Drop for ScanSession<'_> {
    fn drop(&mut self) {
        let remaining = self.cache.session_depth.get().saturating_sub(1);
        self.cache.session_depth.set(remaining);
        if remaining == 0 {
            self.cache.canonical.borrow_mut().clear();
            self.cache.root_is_dir.borrow_mut().clear();
            self.cache.dirs.borrow_mut().clear();
        }
    }
}

#[derive(Debug)]
struct LayerDirScan {
    /// In `fs::read_dir` yield order, INCLUDING the entries that failed to
    /// yield. Every caller's refusals are order sensitive — the pre-memo loops
    /// processed each entry completely before asking for the next, so an entry
    /// that errors must not outrank a refusal an EARLIER entry would have
    /// raised. Keeping the failures in place, and re-raising each one only when
    /// a caller walks onto it, is what preserves that.
    entries: Vec<std::result::Result<LayerDirEntry, String>>,
}

/// Re-raise one `fs::read_dir` yield failure at exactly the position the
/// pre-memo `for entry in fs::read_dir(dir)? { let path = entry?.path(); ... }`
/// loops would have hit it.
fn layer_dir_entry(entry: &std::result::Result<LayerDirEntry, String>) -> Result<&LayerDirEntry> {
    entry.as_ref().map_err(|error| error.clone().into())
}

#[derive(Debug)]
struct LayerDirEntry {
    path: PathBuf,
    /// `None` for an entry whose OS name is not valid UTF-8 — exactly the
    /// entries the pre-memo loops dropped at
    /// `path.file_name().and_then(|value| value.to_str())`.
    name: Option<String>,
    is_file: bool,
    is_dir: bool,
}

impl LayerScanCache {
    /// Declare that the caller's own body does not write into any content
    /// layer, so one scan of a directory answers every lookup this operation
    /// makes. Every call site of this is one such proof; see each of them.
    fn session(&self) -> ScanSession<'_> {
        self.session_depth.set(self.session_depth.get() + 1);
        ScanSession { cache: self }
    }

    fn armed(&self) -> bool {
        self.session_depth.get() > 0
    }

    fn scans_taken(&self) -> usize {
        self.scans_taken.get()
    }

    /// `fs::canonicalize`, once per path per session. The raw io error text is
    /// what the two message shapes below interpolate as `{error}`.
    fn canonicalize(&self, path: &Path) -> std::result::Result<PathBuf, String> {
        if !self.armed() {
            return fs::canonicalize(path).map_err(|error| error.to_string());
        }
        if let Some(cached) = self.canonical.borrow().get(path) {
            return cached.clone();
        }
        let computed = fs::canonicalize(path).map_err(|error| error.to_string());
        self.canonical
            .borrow_mut()
            .insert(path.to_path_buf(), computed.clone());
        computed
    }

    fn canonical_layer_root(&self, root: &Path) -> Result<PathBuf> {
        self.canonicalize(root).map_err(|error| {
            format!(
                "canonicalize content layer root {}: {error}",
                root.display()
            )
            .into()
        })
    }

    fn ensure_within_canonical_layer(
        &self,
        canonical_root: &Path,
        path: &Path,
        label: &str,
    ) -> Result<PathBuf> {
        let canonical_path = self
            .canonicalize(path)
            .map_err(|error| format!("canonicalize {label} {}: {error}", path.display()))?;
        if !canonical_path.starts_with(canonical_root) {
            return Err(format!(
                "content layer {label} escapes mounted root: {} -> {}",
                path.display(),
                canonical_path.display()
            )
            .into());
        }
        Ok(canonical_path)
    }

    fn ensure_within_layer(&self, root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
        let canonical_root = self.canonical_layer_root(root)?;
        self.ensure_within_canonical_layer(&canonical_root, path, label)
    }

    fn layer_root_is_dir(&self, root: &Path) -> bool {
        if !self.armed() {
            return root.is_dir();
        }
        if let Some(cached) = self.root_is_dir.borrow().get(root) {
            return *cached;
        }
        let computed = root.is_dir();
        self.root_is_dir
            .borrow_mut()
            .insert(root.to_path_buf(), computed);
        computed
    }

    fn scan_dir(&self, dir: &Path) -> Result<Rc<LayerDirScan>> {
        if self.armed() {
            if let Some(cached) = self.dirs.borrow().get(dir) {
                return Ok(Rc::clone(cached));
            }
        }
        self.scans_taken.set(self.scans_taken.get() + 1);
        let mut entries = Vec::new();
        for entry in fs::read_dir(dir)? {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    entries.push(Err(error.to_string()));
                    continue;
                }
            };
            // `Path::is_file` and `Path::is_dir` are each a separate
            // `fs::metadata` that maps an error to `false`. One call answers
            // both, with identical results.
            let metadata = fs::metadata(&path);
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string);
            entries.push(Ok(LayerDirEntry {
                is_file: metadata
                    .as_ref()
                    .map(fs::Metadata::is_file)
                    .unwrap_or(false),
                is_dir: metadata.as_ref().map(fs::Metadata::is_dir).unwrap_or(false),
                path,
                name,
            }));
        }
        let scan = Rc::new(LayerDirScan { entries });
        if self.armed() {
            self.dirs
                .borrow_mut()
                .insert(dir.to_path_buf(), Rc::clone(&scan));
        }
        Ok(scan)
    }

    /// Locate one file inside one layer, by the engine's rule.
    ///
    /// This asks [`resolve_literal_layer_path`] — push each requested component
    /// literally and let the volume say what it names — and it is the whole
    /// lookup. It does NOT enumerate the containing directory, does not fold
    /// either side of a comparison, and has no ambiguity refusal, because
    /// `PHYSFS_stat` has none of those either (owner ruling 2026-08-16; the
    /// engine reading is cited in full on the primitive).
    ///
    /// Two behaviours went away with the scan, and both were departures from
    /// the engine rather than protections lost:
    ///
    /// * the `filesystem_case_key` collision refusal, which refused a name NTFS
    ///   and the engine resolve perfectly well (U+00B5 MICRO SIGN against
    ///   U+03BC GREEK SMALL MU), turning a valid recording into an unplayable
    ///   one;
    /// * `ensure_within_layer` run over EVERY sibling before any name was
    ///   compared, so one unreadable or escaping sibling refused every lookup
    ///   in its directory (lane KR's residual R1). The engine stats the one
    ///   path it was asked about and never looks at siblings; the primitive's
    ///   per-component symlink refusal is the engine's own containment rule and
    ///   is what replaces it here.
    ///
    /// Enumeration keeps both the scan memo and `ensure_within_layer` — see
    /// [`LayerScanCache`] and `resolve_dir_files`. Only the point lookup moved.
    fn find_file_in_layer(&self, root: &Path, rel: &str) -> Result<Option<(String, PathBuf)>> {
        let rel = checked_rel_path(rel)?;
        if rel.is_empty() {
            return Ok(None);
        }
        let Some((path, facts)) = literal_layer_path(root, &rel)? else {
            return Ok(None);
        };
        if !facts.is_file {
            return Ok(None);
        }
        // The on-disk spelling of the WINNER, and of the winner only.
        //
        // `ResolvedGameFile::source_relative_path` is contractually the
        // case-preserved name the layer actually carries, and a literal lookup
        // by itself only ever learns what the caller asked for. Canonicalizing
        // the one file that won recovers every component's real spelling from
        // the filesystem -- which is where it has to come from, because
        // reconstructing it by matching entries inside a `read_dir` would
        // reintroduce exactly the static fold this lookup exists to remove.
        //
        // It is also what re-arms containment on the point-lookup path: B does
        // the same `canonicalize` + `starts_with` on its own winner
        // (`effective_vfs::read_layer_file`). What changed is WHICH files pay
        // for it -- one winner instead of every sibling in the directory, which
        // is what used to make one bad sibling refuse every lookup around it.
        let canonical_root = self.canonical_layer_root(root)?;
        let canonical =
            self.ensure_within_canonical_layer(&canonical_root, &path, "resolved file")?;
        let real_rel = canonical
            .strip_prefix(&canonical_root)
            .map_err(|_| {
                format!(
                    "content layer resolved file escapes mounted root: {}",
                    path.display()
                )
            })?
            .to_string_lossy()
            .replace('\\', "/");
        Ok(Some((real_rel, path)))
    }

    /// Locate one directory inside one layer, by the same rule.
    ///
    /// The empty relative path is the layer root itself and is returned without
    /// asking the filesystem anything, exactly as the pre-literal loop did (its
    /// component loop simply never ran); the callers that enumerate it fail on
    /// their own `read_dir` if it is not a directory.
    fn find_dir_in_layer(&self, root: &Path, rel_dir: &str) -> Result<Option<PathBuf>> {
        let rel_dir = checked_rel_path(rel_dir)?;
        if rel_dir.is_empty() {
            return Ok(Some(root.to_path_buf()));
        }
        if !self.layer_root_is_dir(root) {
            return Ok(None);
        }
        match literal_layer_path(root, &rel_dir)? {
            Some((path, facts)) if facts.is_dir => Ok(Some(path)),
            Some(_) | None => Ok(None),
        }
    }
}

/// [`resolve_literal_layer_path`] in this crate's error type.
fn literal_layer_path(root: &Path, rel: &str) -> Result<Option<(PathBuf, LayerPathFacts)>> {
    resolve_literal_layer_path(root, rel).map_err(std::convert::Into::into)
}

fn collect_layer_files_recursive(
    scan: &LayerScanCache,
    layer_root: &Path,
    dir: &Path,
    rel_prefix: &str,
    wanted_extension: Option<&str>,
    out: &mut Vec<(String, PathBuf)>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let canonical_root = scan.canonical_layer_root(layer_root)?;
    collect_layer_files_recursive_canonical(
        scan,
        &canonical_root,
        dir,
        rel_prefix,
        wanted_extension,
        out,
        visited,
    )
}

fn collect_layer_files_recursive_canonical(
    scan: &LayerScanCache,
    canonical_root: &Path,
    dir: &Path,
    rel_prefix: &str,
    wanted_extension: Option<&str>,
    out: &mut Vec<(String, PathBuf)>,
    visited: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let canonical_dir =
        scan.ensure_within_canonical_layer(canonical_root, dir, "recursive directory")?;
    if !visited.insert(canonical_dir) {
        return Err(format!(
            "content layer recursive enumeration encountered a directory link/cycle at {}",
            dir.display()
        )
        .into());
    }
    for entry in scan.scan_dir(dir)?.entries.iter() {
        let entry = layer_dir_entry(entry)?;
        let path = &entry.path;
        let Some(name) = entry.name.as_deref() else {
            continue;
        };
        let child_rel = if rel_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if entry.is_dir {
            scan.ensure_within_canonical_layer(canonical_root, path, "recursive child directory")?;
            collect_layer_files_recursive_canonical(
                scan,
                canonical_root,
                path,
                &child_rel,
                wanted_extension,
                out,
                visited,
            )?;
            continue;
        }
        if !entry.is_file {
            continue;
        }
        scan.ensure_within_canonical_layer(canonical_root, path, "recursive file")?;
        if let Some(ext) = wanted_extension {
            let actual = path
                .extension()
                .and_then(|value| value.to_str())
                .map(|value| value.to_ascii_lowercase());
            if actual.as_deref() != Some(ext) {
                continue;
            }
        }
        out.push((child_rel, path.clone()));
    }
    Ok(())
}

fn immediate_parent(rel: &str) -> String {
    rel.rfind('/')
        .map(|idx| rel[..idx].to_string())
        .unwrap_or_default()
}

pub(crate) fn normalize_rel_dir(value: &str) -> String {
    normalize_rel_path(value)
}

pub(crate) fn normalize_rel_path(value: &str) -> String {
    let mut value = value
        .replace('\\', "/")
        .trim()
        .trim_matches('"')
        .to_string();
    while value.contains("//") {
        value = value.replace("//", "/");
    }
    value
        .trim_matches('/')
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether the replay's content namespace can ask about this EXACT string.
///
/// Every lookup goes through `content_identity_key`, so a path that is not
/// already its own key names a different file than the one asked about — the
/// key rewrites `\` to `/`, lower-cases, trims, strips quotes, collapses empty
/// components, and refuses `.`, `..`, a leading `/`, NUL and `:`. A caller that
/// must preserve authored bytes asks this instead of enumerating those, so the
/// two can never drift apart.
pub fn content_path_is_canonical(value: &str) -> bool {
    content_identity_key(value).map(|key| key == value).unwrap_or(false)
}

fn content_identity_key(rel: &str) -> Result<String> {
    Ok(checked_rel_path(rel)?.to_ascii_lowercase())
}

fn checked_rel_path(value: &str) -> Result<String> {
    let raw = value.replace('\\', "/");
    if raw.starts_with('/') || raw.contains('\0') {
        return Err("content path must be relative and contain no NUL".into());
    }
    let mut parts = Vec::new();
    for part in raw.trim().trim_matches('"').split('/') {
        // Empty components COLLAPSE (the engine's `//` run) but `.` REFUSES.
        // Dropping `.` here silently was a divergence from the recorder side,
        // which refuses it, and the engine agrees with the recorder:
        // `sanitizePlatformIndependentPath` (`0x1424e3ff0`) fails a `.` or `..`
        // component with `PHYSFS_ERR_BAD_FILENAME`. See
        // `portable_content_manifest::sanitize_engine_path`.
        if part.is_empty() {
            continue;
        }
        if part == "." || part == ".." || part.contains(':') {
            return Err(format!("unsafe relative content path: {value}").into());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{label} must be 64 hexadecimal characters").into());
    }
    Ok(())
}

fn read_bounded_file(path: &Path, label: &str) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_CONTENT_PAYLOAD_BYTES {
        return Err(format!(
            "{label} {} exceeds the {MAX_CONTENT_PAYLOAD_BYTES}-byte ceiling",
            path.display()
        )
        .into());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)?
        .take(MAX_CONTENT_PAYLOAD_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CONTENT_PAYLOAD_BYTES {
        return Err(format!(
            "{label} {} grew beyond the {MAX_CONTENT_PAYLOAD_BYTES}-byte ceiling while reading",
            path.display()
        )
        .into());
    }
    Ok(bytes)
}

fn content_layer_kind(kind: &str) -> ContentLayerKind {
    match kind {
        "integrated_dlc" => ContentLayerKind::IntegratedDlc,
        "dlc" => ContentLayerKind::Dlc,
        "mod" => ContentLayerKind::Mod,
        _ => ContentLayerKind::Base,
    }
}

fn validate_used_content_match(actual: &ResolvedGameFile, expected: &UsedContentKey) -> Result<()> {
    let source_sha256 = actual
        .source
        .archive_sha256
        .as_deref()
        .unwrap_or(&actual.sha256);
    if normalize_game_relative_path(&actual.logical_path)? != expected.normalized_game_relative_path
        || actual.source.ordinal as u32 != expected.winning_layer_ordinal
        || content_layer_kind(&actual.source.kind) != expected.layer_kind
        || actual.source.source_id != expected.layer_id
        || source_sha256 != expected.source_sha256
        || actual.sha256 != expected.payload_sha256
        || actual.byte_len != expected.payload_bytes
    {
        return Err(format!(
            "admitted content identity mismatch for {}: expected layer {}:{} ordinal {} source {} payload {} bytes {}, playback layer {}:{} ordinal {} source {} payload {} bytes {}",
            expected.normalized_game_relative_path,
            format!("{:?}", expected.layer_kind).to_ascii_lowercase(),
            expected.layer_id,
            expected.winning_layer_ordinal,
            expected.source_sha256,
            expected.payload_sha256,
            expected.payload_bytes,
            actual.source.kind,
            actual.source.source_id,
            actual.source.ordinal,
            source_sha256,
            actual.sha256,
            actual.byte_len,
        )
        .into());
    }
    Ok(())
}

fn push_foundation_content(
    out: &mut Vec<UsedContentKey>,
    seen: &mut BTreeSet<String>,
    content: UsedContentKey,
) -> Result<()> {
    let path = content.normalized_game_relative_path.clone();
    if !seen.insert(path.clone()) {
        return Err(format!("foundation content closure repeats {path}").into());
    }
    out.push(content);
    Ok(())
}

fn required_foundation_sampler_field(value: Option<&str>, field: &str) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("foundation texture sampler has no {field}").into())
}

fn resolve_foundation_state(
    name: Option<&str>,
    states: &BTreeMap<String, Vec<(String, String)>>,
    label: &str,
) -> Result<Option<Vec<(String, String)>>> {
    name.map(|name| {
        states
            .get(name)
            .cloned()
            .ok_or_else(|| format!("foundation Effect names missing {label} state {name}").into())
    })
    .transpose()
}

pub(crate) fn foundation_dds_descriptor(
    bytes: &[u8],
    content: &UsedContentKey,
) -> Result<FoundationTextureDescriptor> {
    if bytes.len() < 128 || bytes.get(..4) != Some(b"DDS ") {
        return Err(format!(
            "foundation texture {} is not a complete DDS",
            content.normalized_game_relative_path
        )
        .into());
    }
    let read_u32 = |offset: usize| -> Result<u32> {
        let value = bytes
            .get(offset..offset + 4)
            .ok_or("foundation DDS header is truncated")?;
        Ok(u32::from_le_bytes(value.try_into()?))
    };
    if read_u32(4)? != 124 || read_u32(76)? != 32 {
        return Err("foundation DDS has a noncanonical header size".into());
    }
    let width = read_u32(16)?;
    let height = read_u32(12)?;
    let depth = read_u32(24)?.max(1);
    let mip_levels = read_u32(28)?.max(1);
    if width == 0 || height == 0 {
        return Err("foundation DDS has zero dimensions".into());
    }
    let pixel_flags = read_u32(80)?;
    let fourcc = bytes
        .get(84..88)
        .ok_or("foundation DDS pixel format is truncated")?;
    let caps2 = read_u32(112)?;
    let legacy_cube = caps2 & 0x0000_fe00 != 0;
    let (dimension, format, array_size, cube) = if fourcc == b"DX10" {
        if bytes.len() < 148 {
            return Err("foundation DDS DX10 header is truncated".into());
        }
        let dxgi_format = read_u32(128)?;
        let resource_dimension = read_u32(132)?;
        let misc_flag = read_u32(136)?;
        let array_size = read_u32(140)?;
        if array_size == 0 {
            return Err("foundation DDS DX10 array size is zero".into());
        }
        let dimension = match resource_dimension {
            2 => "texture1d",
            3 => "texture2d",
            4 => "texture3d",
            other => {
                return Err(
                    format!("foundation DDS has unsupported resource dimension {other}").into(),
                )
            }
        };
        (
            dimension.to_string(),
            format!("dxgi:{dxgi_format}"),
            array_size,
            misc_flag & 0x4 != 0,
        )
    } else {
        let format = if pixel_flags & 0x4 != 0 {
            format!("fourcc:{}", String::from_utf8_lossy(fourcc))
        } else {
            format!(
                "rgb:{}:{:08x}:{:08x}:{:08x}:{:08x}",
                read_u32(88)?,
                read_u32(92)?,
                read_u32(96)?,
                read_u32(100)?,
                read_u32(104)?
            )
        };
        (
            if depth > 1 { "texture3d" } else { "texture2d" }.to_string(),
            format,
            if legacy_cube { 6 } else { 1 },
            legacy_cube,
        )
    };
    if cube && dimension != "texture2d" {
        return Err("foundation DDS cube is not a 2D texture array".into());
    }

    let resource_value = serde_json::json!({
        "content": content,
        "dimension": dimension,
        "format": format,
        "width": width,
        "height": height,
        "depth": depth,
        "mip_levels": mip_levels,
        "array_size": array_size,
        "cube": cube,
    });
    let resource_identity_sha256 = identity_sha256(&resource_value, "foundation resource")
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let srv_value = serde_json::json!({
        "resource_identity_sha256": resource_identity_sha256,
        "most_detailed_mip": 0,
        "mip_levels": mip_levels,
        "first_array_slice": 0,
        "array_size": array_size,
        "cube": cube,
    });
    let shader_resource_view_identity_sha256 =
        identity_sha256(&srv_value, "foundation shader-resource view")
            .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    Ok(FoundationTextureDescriptor {
        dimension,
        format,
        width,
        height,
        depth,
        mip_levels,
        array_size,
        cube,
        resource_identity_sha256,
        shader_resource_view_identity_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    fn write_text(root: &Path, rel: &str, text: &str) {
        let path = path_from_slash_rel(root, rel);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
        fs::write(path, text).expect("write fixture file");
    }

    fn write_ui14_receipt(
        userdata: &Path,
        layers: &[(&str, u64, u64)],
        winners: &[(&str, &str, &Path)],
    ) {
        let layers = layers.iter().map(|(id, descriptor_ordinal, mount_ordinal)| {
            let descriptor = path_from_slash_rel(userdata, id);
            serde_json::json!({
                "id": id,
                "descriptor_sha256": Sha256::hex(&portable_test_fs_retry::read(descriptor).expect("read descriptor")),
                "engine_descriptor_ordinal": descriptor_ordinal,
                "engine_mount_ordinal": mount_ordinal,
            })
        }).collect::<Vec<_>>();
        let effective_winners = winners
            .iter()
            .map(|(relpath, winner_id, path)| {
                serde_json::json!({
                    "relpath": relpath,
                    "winner_id": winner_id,
                    "sha256": Sha256::hex(&portable_test_fs_retry::read(
                        path
                    ).expect("read winner")),
                })
            })
            .collect::<Vec<_>>();
        let receipt = serde_json::json!({
            "schema": UI14_ENGINE_VFS_SCHEMA,
            "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
            "layers": layers,
            "effective_winners": effective_winners,
        });
        fs::write(
            userdata.join(UI14_ENGINE_VFS_FILE),
            serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
        )
        .expect("write receipt");
    }

    fn read_resolved(resolver: &FileResolver, rel: &str) -> String {
        let path = resolver
            .resolve_file(rel, rel)
            .expect("resolve fixture file");
        read_latin1(&path).expect("read fixture file")
    }

    /// EXACTLY what a lookup answered, including how it refused. Everything a
    /// caller can observe about one resolution is in here: the winning layer,
    /// the case-preserved spelling that layer actually carries, the payload
    /// digest, and -- for a refusal -- the verbatim message.
    fn resolution_outcome(resolver: &GameFileResolver, rel: &str) -> String {
        match resolver.resolve_with_provenance(rel) {
            Ok(None) => "absent".to_string(),
            Ok(Some(resolved)) => format!(
                "ordinal={} kind={} layer={} spelling={} sha256={} bytes={}",
                resolved.source.ordinal,
                resolved.source.kind,
                resolved.source.source_id,
                resolved.source_relative_path,
                resolved.sha256,
                resolved.byte_len,
            ),
            Err(error) => format!("refused: {error}"),
        }
    }

    fn directory_outcome(
        resolver: &GameFileResolver,
        rel_dir: &str,
        extension: Option<&str>,
        recursive: bool,
    ) -> String {
        let listed = if recursive {
            resolver.resolve_dir_files_recursive(rel_dir, extension)
        } else {
            resolver.resolve_dir_files(rel_dir, extension)
        };
        match listed {
            Ok(rows) => rows
                .into_iter()
                .map(|(rel, path)| {
                    format!(
                        "{rel}={}",
                        Sha256::hex(&portable_test_fs_retry::read(&path).expect("read listed file"))
                    )
                })
                .collect::<Vec<_>>()
                .join(","),
            Err(error) => format!("refused: {error}"),
        }
    }

    /// Resolve `paths` one at a time with the scan memo OFF, then again with a
    /// single session holding it open, and require the two transcripts to be
    /// equal. The memo is the ONLY difference between the two passes, so any
    /// divergence -- a different winning layer, a different spelling, different
    /// bytes, a refusal that stopped refusing, or a refusal whose text moved --
    /// fails this. The memoized pass also runs in reverse, because a memo that
    /// carried state between lookups would answer differently depending on
    /// which lookup warmed it.
    fn assert_memo_changes_nothing(resolver: &GameFileResolver, paths: &[&str]) {
        let before = resolver.directory_scans_taken();
        let unmemoized = paths
            .iter()
            .map(|rel| resolution_outcome(resolver, rel))
            .collect::<Vec<_>>();
        let unmemoized_scans = resolver.directory_scans_taken() - before;
        let memoized = resolver.in_resolution_session(|resolver| {
            paths
                .iter()
                .map(|rel| resolution_outcome(resolver, rel))
                .collect::<Vec<_>>()
        });
        let memoized_scans = resolver.directory_scans_taken() - before - unmemoized_scans;
        // NOT VACUOUS, and what it pins is now the CHANGE rather than the memo.
        // A point lookup asks the volume about the components it was handed and
        // enumerates NOTHING, so neither pass may issue a single directory
        // scan, and the memo cannot alter an answer it is never consulted for.
        // Restoring the scan-and-fold `find_file_in_layer` makes both numbers
        // non-zero and fails right here. The memo's own non-vacuity -- that it
        // engages, and that it is cleared between sessions -- is asserted by
        // `assert_memo_changes_no_listing`, over the callers that do enumerate.
        assert_eq!(
            (unmemoized_scans, memoized_scans),
            (0, 0),
            "a point lookup enumerated a directory ({unmemoized_scans} scans unmemoized,              {memoized_scans} memoized); the resolver is not asking the volume literally"
        );
        let memoized_reversed = resolver.in_resolution_session(|resolver| {
            let mut rows = paths
                .iter()
                .rev()
                .map(|rel| resolution_outcome(resolver, rel))
                .collect::<Vec<_>>();
            rows.reverse();
            rows
        });
        for (index, rel) in paths.iter().enumerate() {
            assert_eq!(
                unmemoized[index], memoized[index],
                "memoized resolution of {rel:?} disagrees with the unmemoized one"
            );
            assert_eq!(
                unmemoized[index], memoized_reversed[index],
                "memoized resolution of {rel:?} depends on lookup order"
            );
        }
    }

    fn assert_memo_changes_no_listing(
        resolver: &GameFileResolver,
        listings: &[(&str, Option<&str>, bool)],
    ) {
        for (rel_dir, extension, recursive) in listings {
            let unmemoized = directory_outcome(resolver, rel_dir, *extension, *recursive);
            let session_start = resolver.directory_scans_taken();
            let memoized = resolver.in_resolution_session(|resolver| {
                // Twice inside one session: the second call is the one that
                // reads the memo the first call filled.
                let _first = directory_outcome(resolver, rel_dir, *extension, *recursive);
                let warmed = resolver.directory_scans_taken();
                let repeated = directory_outcome(resolver, rel_dir, *extension, *recursive);
                // NOT VACUOUS: the repeat inside a live session must issue no
                // filesystem scan at all, or the memo is not being read.
                assert_eq!(
                    resolver.directory_scans_taken(),
                    warmed,
                    "repeating the {rel_dir:?} listing inside one session rescanned the disk"
                );
                repeated
            });
            let scans_this_session = resolver.directory_scans_taken() - session_start;
            assert_eq!(
                unmemoized, memoized,
                "memoized listing of {rel_dir:?} (recursive={recursive}) disagrees"
            );
            // NOT VACUOUS, and it lives here now that a point lookup takes no
            // scans at all to carry it: a SEPARATE session must go back to the
            // filesystem. Without this, deleting the clear in
            // `ScanSession::drop` -- which is what stops one operation's scans
            // answering the next one, minutes or hours later -- would pass
            // every other assertion in this file.
            let before_second_session = resolver.directory_scans_taken();
            let second_session = resolver.in_resolution_session(|resolver| {
                directory_outcome(resolver, rel_dir, *extension, *recursive)
            });
            assert_eq!(
                unmemoized, second_session,
                "a second session's listing of {rel_dir:?} (recursive={recursive}) disagrees"
            );
            // Guarded, because a listing that refuses before it enumerates
            // anything -- a missing directory -- legitimately scans nothing,
            // and an unguarded assertion there would be a lie rather than a
            // check.
            if scans_this_session > 0 {
                assert!(
                    resolver.directory_scans_taken() > before_second_session,
                    "a new session reused the previous session's scans for {rel_dir:?};                      the memo is not being cleared"
                );
            }
        }
    }

    /// The layered fixture the differential runs over. Everything in it is a
    /// shape that could make a memoized directory scan answer differently from
    /// a fresh one: a name that differs only by case ACROSS layers, a name that
    /// is a directory in one layer and a regular file in another, ancestors
    /// deep enough that every one of them is separately scanned, a
    /// `replace_path` whose effect depends on the exact parent, wide sibling
    /// sets so a scan is worth reusing, and paths that must stay absent.
    /// Returns the resolver AND the scanner guards for the tree it just wrote.
    /// The resolver opens those files (`mod_resolver.rs`'s own `File::open`)
    /// immediately afterwards, so the caller must hold the guards for the
    /// length of the test.
    #[must_use = "the scanner guards must be held for the length of the test"]
    fn layered_differential_fixture(root: &Path) -> (GameFileResolver, Vec<std::fs::File>) {
        let game = root.join("game");
        let userdata = root.join("userdata");

        write_text(&game, "common/deep/a/b/c/leaf.txt", "base-leaf");
        write_text(&game, "gfx/Interface/Shared.dds", "base-shared");
        write_text(&game, "gfx/interface/only_base.dds", "base-only");
        write_text(&game, "common/collide/inner.txt", "base-collide-dir-member");
        write_text(&game, "history/states/1.txt", "base-state-1");
        write_text(&game, "history/states/2.txt", "base-state-2");
        for index in 0..64 {
            write_text(
                &game,
                &format!("gfx/interface/filler_{index:03}.dds"),
                &format!("base-filler-{index}"),
            );
        }

        write_text(
            &userdata,
            "mod/alpha/gfx/interface/SHARED.dds",
            "alpha-shared",
        );
        write_text(&userdata, "mod/alpha/common/collide", "alpha-collide-file");
        write_text(
            &userdata,
            "mod/alpha.mod",
            "name=\"Alpha\"\npath=\"mod/alpha\"\n",
        );

        write_text(
            &userdata,
            "mod/beta/common/deep/a/b/c/LEAF.TXT",
            "beta-leaf",
        );
        write_text(&userdata, "mod/beta/history/states/2.txt", "beta-state-2");
        write_text(
            &userdata,
            "mod/beta.mod",
            "name=\"Beta\"\npath=\"mod/beta\"\nreplace_path=\"history/states\"\n",
        );

        write_text(
            &userdata,
            "dlc_load.json",
            "{\"enabled_mods\":[\"mod/alpha.mod\",\"mod/beta.mod\"],\"disabled_dlcs\":[]}",
        );
        let guards = portable_test_fs_retry::tree_read_guards(root)
            .expect("hold the layered fixture open across the scanner's window");
        let resolver = GameFileResolver::from_dlc_load(game, userdata)
            .expect("layered fixture resolver");
        (resolver, guards)
    }

    const DIFFERENTIAL_PATHS: &[&str] = &[
        "gfx/interface/shared.dds",
        "gfx/Interface/Shared.dds",
        "gfx/INTERFACE/SHARED.DDS",
        "gfx/interface/only_base.dds",
        "gfx/interface/ONLY_BASE.DDS",
        "gfx/interface/filler_000.dds",
        "gfx/interface/filler_063.dds",
        "gfx/interface/filler_064.dds",
        "common/deep/a/b/c/leaf.txt",
        "common/deep/A/B/C/LEAF.TXT",
        "common/collide",
        "common/collide/inner.txt",
        "history/states/1.txt",
        "history/states/2.txt",
        "gfx/interface/missing.dds",
        "gfx/nowhere/missing.dds",
        "common/deep/a/b/c/leaf.txt/child.txt",
        "common/nothing/here/at/all.txt",
    ];

    #[test]
    fn memoized_resolution_session_is_byte_identical_to_unmemoized_lookup() {
        let root = temp_test_dir("resolver_memo_differential");
        let (resolver, _scanner_guards) = layered_differential_fixture(&root);

        let winners = DIFFERENTIAL_PATHS
            .iter()
            .filter(|rel| resolution_outcome(&resolver, rel).starts_with("ordinal="))
            .count();
        assert!(
            winners >= 10,
            "the differential corpus resolved only {winners} paths; the fixture is not layering"
        );
        let shared = resolution_outcome(&resolver, "gfx/interface/shared.dds");
        assert!(
            shared.contains("spelling=gfx/interface/SHARED.dds"),
            "the case-differing mod file must be the winner: {shared}"
        );
        assert_eq!(
            resolution_outcome(&resolver, "history/states/1.txt"),
            "absent",
            "replace_path must have removed the base sibling"
        );

        assert_memo_changes_nothing(&resolver, DIFFERENTIAL_PATHS);
        assert_memo_changes_no_listing(
            &resolver,
            &[
                ("gfx/interface", Some("dds"), false),
                ("gfx/interface", None, false),
                ("history/states", None, false),
                ("common", None, true),
                ("", None, true),
                ("common/deep/a/b/c", None, false),
                ("gfx/nowhere", None, false),
            ],
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn memoized_session_reproduces_refusals_and_their_order() {
        let root = temp_test_dir("resolver_memo_refusals");
        let game = root.join("game");
        let outside = root.join("outside");
        write_text(&outside, "escaped.dds", "outside");

        write_text(&game, "gfx/collide/\u{b5}unit.dds", "micro");
        write_text(&game, "gfx/collide/\u{3bc}unit.dds", "mu");
        write_text(&game, "gfx/collide/ordinary.dds", "ordinary");

        write_text(&game, "gfx/escape/ordinary.dds", "ordinary");
        let link = game.join("gfx/escape/linked.dds");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(outside.join("escaped.dds"), &link).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(outside.join("escaped.dds"), &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let linked = false;

        let resolver = GameFileResolver::vanilla(&game).expect("vanilla resolver");

        for (spelling, payload) in [
            ("gfx/collide/\u{b5}unit.dds", "micro"),
            ("gfx/collide/\u{3bc}unit.dds", "mu"),
            ("gfx/collide/ordinary.dds", "ordinary"),
        ] {
            let outcome = resolution_outcome(&resolver, spelling);
            assert!(
                outcome.starts_with("ordinal=")
                    && outcome.contains(&format!("sha256={}", Sha256::hex(payload.as_bytes()))),
                "{spelling} must resolve to its own file: {outcome}"
            );
        }
        let collision_listing = directory_outcome(&resolver, "gfx/collide", None, false);
        for name in ["\u{b5}unit.dds", "\u{3bc}unit.dds", "ordinary.dds"] {
            assert!(
                collision_listing.contains(&format!("gfx/collide/{name}=")),
                "the listing must carry {name}: {collision_listing}"
            );
        }
        if linked {
            let symlink = resolution_outcome(&resolver, "gfx/escape/linked.dds");
            assert!(
                symlink.starts_with("refused:") && symlink.contains("reparse point"),
                "a symlinked component must refuse: {symlink}"
            );
            let sibling = resolution_outcome(&resolver, "gfx/escape/ordinary.dds");
            assert!(
                sibling.starts_with("ordinal="),
                "a sibling of an escaping link must resolve; only enumeration looks at \
                 siblings: {sibling}"
            );
            let escape_listing = directory_outcome(&resolver, "gfx/escape", None, false);
            assert!(
                escape_listing.starts_with("refused:")
                    && escape_listing.contains("escapes mounted root"),
                "the escaping entry must still refuse the LISTING: {escape_listing}"
            );
        }

        let mut paths = vec![
            "gfx/collide/ordinary.dds",
            "gfx/collide/\u{b5}unit.dds",
            "gfx/collide/\u{3bc}unit.dds",
            "gfx/collide/absent.dds",
        ];
        if linked {
            paths.extend([
                "gfx/escape/ordinary.dds",
                "gfx/escape/linked.dds",
                "gfx/escape/absent.dds",
            ]);
        }
        assert_memo_changes_nothing(&resolver, &paths);
        assert_memo_changes_no_listing(
            &resolver,
            &[
                ("gfx/collide", None, false),
                ("gfx/collide", Some("dds"), false),
                ("gfx", None, true),
            ],
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn admitted_content_snapshot_refuses_to_overlap_a_mounted_layer_without_creating_it() {
        let root = temp_test_dir("resolver_snapshot_overlap");
        let game = root.join("game");
        write_text(&game, "interface/base.gui", "base-ui");
        let resolver = GameFileResolver::vanilla(&game).expect("vanilla resolver");

        let inside = game.join("staged-snapshot").join("1-0");
        let error = resolver
            .refuse_snapshot_root_overlapping_a_layer(&inside)
            .expect_err("a snapshot root inside a mounted layer must be refused")
            .to_string();
        assert!(
            error.contains("overlaps mounted content layer"),
            "unexpected refusal: {error}"
        );
        assert!(
            !inside.exists() && !game.join("staged-snapshot").exists(),
            "the refusal created the directory it refused"
        );

        let containing = root.clone();
        let error = resolver
            .refuse_snapshot_root_overlapping_a_layer(&containing)
            .expect_err("a snapshot root containing a mounted layer must be refused")
            .to_string();
        assert!(
            error.contains("overlaps mounted content layer"),
            "unexpected refusal: {error}"
        );

        let disjoint = root.join("elsewhere").join("1-0");
        resolver
            .refuse_snapshot_root_overlapping_a_layer(&disjoint)
            .expect("a disjoint snapshot root must be admitted");
        assert!(
            !disjoint.exists(),
            "the check created the directory it was only asked about"
        );

        resolver
            .refuse_snapshot_root_overlapping_a_layer(&unique_content_admission_root())
            .expect("the ordinary temp-root snapshot must stay admitted");

        let created = resolver
            .create_admission_snapshot_root()
            .expect("an ordinary temp-root snapshot must be created");
        assert!(
            created.is_dir(),
            "the admission snapshot helper did not create its root: {}",
            created.display()
        );
        let _ = fs::remove_dir_all(&created);

        let alias = root.join("alias");
        if std::os::windows::fs::symlink_dir(game.join("staged"), &alias).is_ok() {
            assert!(
                !alias.exists(),
                "test setup: the alias target must stay missing for this case"
            );
            let error = resolver
                .refuse_snapshot_root_overlapping_a_layer(&alias.join("1-0"))
                .expect_err("a snapshot root under an unresolvable alias must be refused")
                .to_string();
            assert!(
                error.contains("canonicalize admitted content snapshot root"),
                "unexpected refusal: {error}"
            );
            assert!(
                !game.join("staged").exists(),
                "the refusal created the snapshot inside the mounted layer"
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    #[test]
    fn gate_vanilla_memoized_session_matches_unmemoized_lookup_on_the_installed_corpus() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");

        let mut corpus = Vec::new();
        for (dir, extension) in [
            ("interface", Some("gfx")),
            ("common/abilities", None),
            ("gfx/interface/abilitylist", None),
        ] {
            let rows = resolver
                .resolve_dir_files(dir, extension)
                .unwrap_or_else(|error| panic!("list {dir}: {error}"));
            assert!(!rows.is_empty(), "installed corpus has no {dir} rows");
            for (index, (rel, _)) in rows.into_iter().enumerate() {
                if index % 7 == 0 {
                    corpus.push(rel);
                }
            }
        }
        let upper = corpus
            .iter()
            .take(8)
            .map(|rel| rel.to_uppercase())
            .collect::<Vec<_>>();
        corpus.extend(upper);
        corpus.push("interface/no_such_file_lane_kr.gfx".to_string());
        corpus.push("no_such_directory_lane_kr/file.txt".to_string());

        let paths = corpus.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(
            paths.len() >= 20,
            "installed differential corpus is only {} paths",
            paths.len()
        );
        assert_memo_changes_nothing(&resolver, &paths);
        assert_memo_changes_no_listing(
            &resolver,
            &[
                ("common/abilities", None, false),
                ("interface", Some("gfx"), false),
                ("gfx/interface/abilitylist", None, true),
            ],
        );
    }

    #[test]
    fn resolver_file_override_uses_mod_over_vanilla() {
        let root = temp_test_dir("resolver_file_override");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&mod_root, "map/definition.csv", "modded");

        let resolver = FileResolver::with_mod_layers(
            vanilla.clone(),
            vec![ModContentLayer {
                root: mod_root,
                source_kind: "mod".to_string(),
                source_id: "fixture.mod".to_string(),
                descriptor_sha256: None,
                archive_sha256: None,
                dependencies: Vec::new(),
                recorded_ordinal: None,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("Override".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: Vec::new(),
                },
            }],
        );

        assert_eq!(
            read_resolved(&resolver, "map/definition.csv"),
            "modded".to_string()
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_resolves_each_unicode_case_neighbour_to_its_own_file() {
        let root = temp_test_dir("resolver_case_key_ambiguity");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, "gfx/\u{b5}.asset", "micro-sign");
        write_text(&vanilla, "gfx/\u{39c}.asset", "greek-mu");
        assert_ne!(
            fs::read(vanilla.join("gfx/\u{b5}.asset")).expect("micro sign file"),
            fs::read(vanilla.join("gfx/\u{39c}.asset")).expect("greek mu file"),
            "the host filesystem folded MICRO SIGN onto GREEK CAPITAL MU; the premise is gone"
        );
        let resolver = FileResolver::vanilla(vanilla);
        for (spelling, payload) in [
            ("gfx/\u{b5}.asset", "micro-sign"),
            ("gfx/\u{39c}.asset", "greek-mu"),
        ] {
            let path = resolver
                .resolve_optional_file(spelling)
                .expect("the engine resolves each spelling")
                .expect("each spelling names a file");
            assert_eq!(
                fs::read_to_string(&path).expect("read resolved file"),
                payload,
                "{spelling} resolved to the wrong file"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn foundation_dds_descriptor_is_content_addressed_and_dimension_generic() {
        let mut dds = vec![0u8; 128];
        dds[0..4].copy_from_slice(b"DDS ");
        dds[4..8].copy_from_slice(&124u32.to_le_bytes());
        dds[12..16].copy_from_slice(&1536u32.to_le_bytes());
        dds[16..20].copy_from_slice(&3072u32.to_le_bytes());
        dds[28..32].copy_from_slice(&7u32.to_le_bytes());
        dds[76..80].copy_from_slice(&32u32.to_le_bytes());
        dds[80..84].copy_from_slice(&4u32.to_le_bytes());
        dds[84..88].copy_from_slice(b"DXT1");
        let content = UsedContentKey {
            effective_vfs_manifest_sha256: "a".repeat(64),
            normalized_game_relative_path: "map/terrain/test.dds".into(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: "b".repeat(64),
            payload_sha256: Sha256::hex(&dds),
            payload_bytes: dds.len() as u64,
        };
        let descriptor = foundation_dds_descriptor(&dds, &content).unwrap();
        assert_eq!((descriptor.width, descriptor.height), (3072, 1536));
        assert_eq!(descriptor.mip_levels, 7);
        assert_eq!(descriptor.format, "fourcc:DXT1");
        assert_eq!(descriptor.dimension, "texture2d");
        assert_eq!(descriptor.resource_identity_sha256.len(), 64);
        assert_eq!(descriptor.shader_resource_view_identity_sha256.len(), 64);
        dds[16..20].copy_from_slice(&0u32.to_le_bytes());
        assert!(foundation_dds_descriptor(&dds, &content).is_err());
    }

    #[test]
    fn resolver_directory_merge_and_replace_path_are_non_recursive() {
        let root = temp_test_dir("resolver_replace_path");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "history/states/1-a.txt", "vanilla-a");
        write_text(&vanilla, "history/states/2-b.txt", "vanilla-b");
        write_text(
            &vanilla,
            "history/states/sub/10-nested.txt",
            "vanilla-nested",
        );
        write_text(&mod_root, "history/states/2-b.txt", "mod-b");
        write_text(&mod_root, "history/states/3-c.txt", "mod-c");

        let resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![ModContentLayer {
                root: mod_root,
                source_kind: "mod".to_string(),
                source_id: "fixture.mod".to_string(),
                descriptor_sha256: None,
                archive_sha256: None,
                dependencies: Vec::new(),
                recorded_ordinal: None,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("ReplaceStates".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: vec!["history/states".to_string()],
                },
            }],
        );

        let files = resolver
            .resolve_dir_files("history/states", Some("txt"))
            .expect("resolve state dir");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/2-b.txt", "history/states/3-c.txt"]
        );
        assert_eq!(read_latin1(&files[0].1).unwrap(), "mod-b");

        let nested = resolver
            .resolve_dir_files("history/states/sub", Some("txt"))
            .expect("resolve nested state dir");
        assert_eq!(
            nested
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/sub/10-nested.txt"]
        );
        assert_eq!(read_latin1(&nested[0].1).unwrap(), "vanilla-nested");
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 2);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_recursive_directory_merge_overrides_and_replaces_exact_dir_only() {
        let root = temp_test_dir("resolver_recursive_dir");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "gfx/models/a.mesh", "vanilla-a");
        write_text(&vanilla, "gfx/models/buildings/b.mesh", "vanilla-b");
        write_text(&vanilla, "gfx/models/buildings/deep/c.mesh", "vanilla-c");
        write_text(&vanilla, "gfx/models/readme.txt", "not-a-mesh");
        write_text(&mod_root, "gfx/models/buildings/b.mesh", "mod-b");
        write_text(&mod_root, "gfx/models/new/d.mesh", "mod-d");

        let resolver = FileResolver::with_mod_layers(
            vanilla.clone(),
            vec![ModContentLayer {
                root: mod_root.clone(),
                source_kind: "mod".to_string(),
                source_id: "fixture.mod".to_string(),
                descriptor_sha256: None,
                archive_sha256: None,
                dependencies: Vec::new(),
                recorded_ordinal: None,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("Models".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: Vec::new(),
                },
            }],
        );

        let files = resolver
            .resolve_dir_files_recursive("gfx/models", Some("mesh"))
            .expect("recursive model listing");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gfx/models/a.mesh",
                "gfx/models/buildings/b.mesh",
                "gfx/models/buildings/deep/c.mesh",
                "gfx/models/new/d.mesh",
            ]
        );
        let modded = files
            .iter()
            .find(|(rel, _)| rel == "gfx/models/buildings/b.mesh")
            .expect("overridden entry");
        assert_eq!(read_latin1(&modded.1).unwrap(), "mod-b");
        assert_eq!(resolver.stats().overridden_file_count, 1);

        let replace_resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![ModContentLayer {
                root: mod_root,
                source_kind: "mod".to_string(),
                source_id: "fixture.mod".to_string(),
                descriptor_sha256: None,
                archive_sha256: None,
                dependencies: Vec::new(),
                recorded_ordinal: None,
                manifest: BundleMod {
                    load_order: 0,
                    name: Some("ReplaceBuildings".to_string()),
                    version: None,
                    supported_version: None,
                    workshop_id: None,
                    descriptor: None,
                    replace_path: vec!["gfx/models/buildings".to_string()],
                },
            }],
        );
        let replaced = replace_resolver
            .resolve_dir_files_recursive("gfx/models", Some("mesh"))
            .expect("recursive model listing with replace_path");
        assert_eq!(
            replaced
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "gfx/models/a.mesh",
                "gfx/models/buildings/b.mesh",
                "gfx/models/buildings/deep/c.mesh",
                "gfx/models/new/d.mesh",
            ],
            "replace_path removes the exact dir from lower layers, keeps subdirs, and the mod re-adds its own b.mesh"
        );
        let replaced_b = replaced
            .iter()
            .find(|(rel, _)| rel == "gfx/models/buildings/b.mesh")
            .expect("replaced entry");
        assert_eq!(read_latin1(&replaced_b.1).unwrap(), "mod-b");
        assert_eq!(
            replace_resolver.stats().replace_path_excluded_file_count,
            1,
            "vanilla gfx/models/buildings/b.mesh is excluded by replace_path before the mod layer re-adds it"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_load_order_later_mod_wins() {
        let root = temp_test_dir("resolver_load_order");
        let vanilla = root.join("vanilla");
        let mod_low = root.join("mod_low");
        let mod_high = root.join("mod_high");
        write_text(&vanilla, "common/countries/colors.txt", "vanilla");
        write_text(&mod_low, "common/countries/colors.txt", "low");
        write_text(&mod_high, "common/countries/colors.txt", "high");

        let layer = |root: PathBuf, load_order, name: &str| ModContentLayer {
            root,
            source_kind: "mod".to_string(),
            source_id: format!("{name}.mod"),
            descriptor_sha256: None,
            archive_sha256: None,
            dependencies: Vec::new(),
            recorded_ordinal: None,
            manifest: BundleMod {
                load_order,
                name: Some(name.to_string()),
                version: None,
                supported_version: None,
                workshop_id: None,
                descriptor: None,
                replace_path: Vec::new(),
            },
        };
        let resolver = FileResolver::with_mod_layers(
            vanilla,
            vec![
                layer(mod_low, 0, "LowPriority"),
                layer(mod_high, 1, "HighPriority"),
            ],
        );

        assert_eq!(
            read_resolved(&resolver, "common/countries/colors.txt"),
            "high".to_string()
        );
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn resolver_vanilla_only_returns_vanilla_paths_and_file_lists() {
        let root = temp_test_dir("resolver_vanilla_only");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, "history/states/1-a.txt", "a");
        write_text(&vanilla, "history/states/2-b.txt", "b");
        let resolver = FileResolver::vanilla(vanilla.clone());

        let path = resolver
            .resolve_file("history/states/1-a.txt", "state")
            .expect("resolve vanilla state");
        assert_eq!(path, vanilla.join("history").join("states").join("1-a.txt"));
        let files = resolver
            .resolve_dir_files("history/states", Some("txt"))
            .expect("resolve vanilla state dir");
        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec!["history/states/1-a.txt", "history/states/2-b.txt"]
        );
        assert_eq!(resolver.stats().overridden_file_count, 0);
        assert_eq!(resolver.stats().replace_path_excluded_file_count, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn public_game_file_resolver_merges_dir_files_with_override_dedup() {
        let root = temp_test_dir("game_file_resolver_dir_merge");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "common/test/a.txt", "vanilla-a");
        write_text(&vanilla, "common/test/b.txt", "vanilla-b");
        write_text(&mod_root, "common/test/b.txt", "mod-b");
        write_text(&mod_root, "common/test/c.txt", "mod-c");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let files = resolver
            .resolve_dir_files("common/test", Some("txt"))
            .expect("resolve merged dir");

        assert_eq!(
            files
                .iter()
                .map(|(rel, _)| rel.as_str())
                .collect::<Vec<_>>(),
            vec![
                "common/test/a.txt",
                "common/test/b.txt",
                "common/test/c.txt"
            ]
        );
        assert_eq!(read_latin1(&files[1].1).unwrap(), "mod-b");
        assert_eq!(resolver.stats().overridden_file_count, 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sensitive_file_override_report_lists_modded_sensitive_inputs() {
        let root = temp_test_dir("game_file_resolver_sensitive");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("mod");
        write_text(&vanilla, "gfx/FX/constants.fxh", "vanilla constants");
        write_text(&vanilla, "gfx/FX/pdxmap.shader", "vanilla shader");
        write_text(&vanilla, "map/seasons.txt", "vanilla seasons");
        write_text(&mod_root, "gfx/FX/constants.fxh", "mod constants");
        write_text(&mod_root, "map/seasons.txt", "mod seasons");

        let resolver = GameFileResolver::with_mod_paths(vanilla, vec![mod_root]).expect("resolver");
        let report = sensitive_file_overrides(
            &resolver,
            &[
                "gfx/FX/constants.fxh",
                "gfx/FX/pdxmap.shader",
                "map/seasons.txt",
            ],
        )
        .expect("override report");

        assert_eq!(
            report.overridden,
            vec![
                "gfx/FX/constants.fxh".to_string(),
                "map/seasons.txt".to_string()
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn modset_digest_matches_python_schema_fixture() {
        let mods = vec![
            BundleMod {
                load_order: 0,
                name: Some("Ignored When Workshop Exists".to_string()),
                version: Some("1.2.3".to_string()),
                supported_version: None,
                workshop_id: Some("12345".to_string()),
                descriptor: Some("workshop.mod".to_string()),
                replace_path: Vec::new(),
            },
            BundleMod {
                load_order: 1,
                name: Some("Local Test".to_string()),
                version: Some("2026.7".to_string()),
                supported_version: None,
                workshop_id: None,
                descriptor: Some("local.mod".to_string()),
                replace_path: Vec::new(),
            },
        ];
        assert_eq!(
            canonical_modset_json(&mods),
            "[[\"workshop:12345\",\"1.2.3\"],[\"name:Local Test\",\"2026.7\"]]"
        );
        assert_eq!(modset_id(&mods), "35bc3f7d45155d2b");
        assert_eq!(modset_id(&[]), "vanilla");
    }

    #[test]
    fn descriptor_parser_redacts_paths_and_keeps_replace_paths() {
        let parsed = parse_mod_descriptor_text(
            r#"
            name="AMSB"
            path="C:/Users/Example/Documents/Paradox Interactive/Hearts of Iron IV/mod/amsb"
            version="1.0"
            supported_version="1.16.*"
            remote_file_id="3470184538"
            replace_path="history/states"
            replace_path="common/countries"
            dependencies={
                "Core Library"
                "Map Framework"
            }
            "#,
        );
        let manifest = parsed.to_bundle_mod(3);
        assert_eq!(manifest.load_order, 3);
        assert_eq!(manifest.name.as_deref(), Some("AMSB"));
        assert_eq!(manifest.version.as_deref(), Some("1.0"));
        assert_eq!(manifest.workshop_id.as_deref(), Some("3470184538"));
        assert_eq!(
            manifest.replace_path,
            vec!["history/states".to_string(), "common/countries".to_string()]
        );
        assert_eq!(manifest.descriptor, None);
        assert_eq!(parsed.dependencies, vec!["Core Library", "Map Framework"]);
    }

    #[test]
    fn launcher_mod_membership_is_reordered_by_engine_law_and_recorded_order_is_explicit() {
        let root = temp_test_dir("resolver_dlc_load");
        let vanilla = root.join("vanilla");
        let userdata = root.join("userdata");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&userdata, "mod/low/content/map/definition.csv", "low");
        write_text(&userdata, "mod/high/content/map/definition.csv", "high");
        write_text(
            &userdata,
            "mod/low.mod",
            r#"
            name="Low Mod"
            path="mod/low/content"
            version="1"
            replace_path="history/states"
            "#,
        );
        write_text(
            &userdata,
            "mod/high.mod",
            r#"
            name="High Mod"
            path="mod/high/content"
            version="2"
            remote_file_id="999"
            "#,
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/low.mod","mod/high.mod"]}"#,
        );
        let setup = BundleResolverSetup::from_options(
            vanilla.clone(),
            &BundleModSelectionOptions {
                mods: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata.clone()),
            },
        )
        .expect("resolve dlc_load mod set");

        assert_eq!(
            read_resolved(&setup.resolver, "map/definition.csv"),
            "low".to_string()
        );
        let modset = setup.manifest_modset().expect("explicit modset manifest");
        assert_eq!(modset.selection_source, "dlc_load.json");
        assert_eq!(modset.mods.len(), 2);
        assert_eq!(modset.mods[0].load_order, 1);
        assert_eq!(modset.mods[1].load_order, 2);
        assert_eq!(modset.mods[0].descriptor.as_deref(), Some("high.mod"));
        assert_eq!(modset.mods[1].replace_path, vec!["history/states"]);
        assert_eq!(modset.mods[0].workshop_id.as_deref(), Some("999"));
        assert!(!serde_json::to_string(&modset)
            .expect("serialize modset")
            .contains(&path_basename(&root)));
        let recorded = GameFileResolver::from_recorded_content_order(
            vanilla,
            userdata.clone(),
            &["mod/low.mod".to_string(), "mod/high.mod".to_string()],
        )
        .expect("recorded order resolver");
        assert_eq!(
            read_latin1(
                &recorded
                    .resolve_required("map/definition.csv", "definition")
                    .unwrap()
            )
            .unwrap(),
            "high"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dlc_load_mounts_archive_mod_without_extractor_staging() {
        let root = temp_test_dir("resolver_dlc_load_archive_only");
        let vanilla = root.join("vanilla");
        let userdata = root.join("userdata");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(
            &userdata,
            "_ui15_dlc/archive.mod",
            r#"
            name="Archive DLC"
            archive="dlc/archive.zip"
            "#,
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["_ui15_dlc/archive.mod"]}"#,
        );
        let archive_path = userdata.join("dlc/archive.zip");
        fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
        let file = File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        use std::io::Write as _;
        writer
            .start_file(
                "interface/archive.gui",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"archive-ui").unwrap();
        writer.finish().unwrap();

        let setup = BundleResolverSetup::from_options(
            vanilla,
            &BundleModSelectionOptions {
                mods: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata.clone()),
            },
        )
        .expect("archive mod must mount directly");
        assert_eq!(
            read_resolved(&setup.resolver, "interface/archive.gui"),
            "archive-ui"
        );
        assert!(setup.resolver.layers[1].source.archive_sha256.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_mount_refuses_empty_and_keeps_cache_metadata_outside_namespace() {
        use std::io::Write as _;

        let root = temp_test_dir("resolver_archive_invalid_namespace");
        let cache = root.join("cache");

        let mut empty = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let empty_bytes = empty.finish().unwrap().into_inner();
        let empty_sha = Sha256::hex(&empty_bytes);
        assert!(materialize_zip_archive(&empty_bytes, &cache, &empty_sha)
            .unwrap_err()
            .to_string()
            .contains("empty"));

        let mut marker = zip::ZipWriter::new(Cursor::new(Vec::new()));
        marker
            .start_file(".COMPLETE-SHA256", zip::write::SimpleFileOptions::default())
            .unwrap();
        marker.write_all(b"forged").unwrap();
        let marker_bytes = marker.finish().unwrap().into_inner();
        let marker_sha = Sha256::hex(&marker_bytes);
        let mounted = materialize_zip_archive(&marker_bytes, &cache, &marker_sha).unwrap();
        assert_eq!(
            portable_test_fs_retry::read(mounted.join(".COMPLETE-SHA256")).unwrap(),
            b"forged"
        );
        assert_eq!(
            portable_test_fs_retry::read_to_string(mounted.parent().unwrap().join(".complete-sha256")).unwrap(),
            marker_sha
        );

        let mut collided = zip::ZipWriter::new(Cursor::new(Vec::new()));
        collided
            .start_file("gfx/\u{b5}.asset", zip::write::SimpleFileOptions::default())
            .unwrap();
        collided.write_all(b"one").unwrap();
        collided
            .start_file(
                "gfx/\u{39c}.asset",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        collided.write_all(b"two").unwrap();
        let collided_bytes = collided.finish().unwrap().into_inner();
        let collided_sha = Sha256::hex(&collided_bytes);
        let error = materialize_zip_archive(&collided_bytes, &cache, &collided_sha)
            .unwrap_err()
            .to_string();
        assert!(error.contains("case-colliding"), "{error}");
        assert!(
            !cache
                .join(format!(".{collided_sha}.{}.building", std::process::id()))
                .exists(),
            "failed archive extraction must remove its staging directory"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_refuses_a_member_the_volume_renames_on_extraction() {
        use std::io::Write as _;
        let root = temp_test_dir("resolver_archive_renamed_member");
        let cache = root.join("cache");

        let probe_dir = root.join("probe");
        fs::create_dir_all(&probe_dir).unwrap();
        fs::write(probe_dir.join("a.txt."), b"probe").unwrap();
        assert!(
            probe_dir.join("a.txt").is_file(),
            "this host did not strip the trailing dot, so the fixture no longer \
             reproduces the rewrite this test is about"
        );

        let mut renamed = zip::ZipWriter::new(Cursor::new(Vec::new()));
        renamed
            .start_file("gfx/a.txt.", zip::write::SimpleFileOptions::default())
            .unwrap();
        renamed.write_all(b"aliased").unwrap();
        let renamed_bytes = renamed.finish().unwrap().into_inner();
        let renamed_sha = Sha256::hex(&renamed_bytes);

        let error = materialize_zip_archive(&renamed_bytes, &cache, &renamed_sha)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("did not reproduce its declared members"),
            "{error}"
        );
        assert!(
            !cache.join(&renamed_sha).exists(),
            "a refused extraction must publish nothing"
        );
        assert!(
            !cache
                .join(format!(".{renamed_sha}.{}.building", std::process::id()))
                .exists(),
            "a refused extraction must remove its staging directory"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn dlc_load_rejects_missing_enabled_descriptor() {
        let root = temp_test_dir("resolver_dlc_load_missing_descriptor");
        let vanilla = root.join("vanilla");
        let userdata = root.join("userdata");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/missing.mod"]}"#,
        );

        let error = BundleResolverSetup::from_options(
            vanilla,
            &BundleModSelectionOptions {
                mods: Vec::new(),
                use_dlc_load: true,
                userdata: Some(userdata),
            },
        )
        .expect_err("missing enabled descriptor must fail closed");
        assert!(error
            .to_string()
            .contains("enabled content descriptor not found"));
        let _ = fs::remove_dir_all(&root);
    }


    fn explicit_setup(vanilla: PathBuf, mods: Vec<PathBuf>) -> Result<BundleResolverSetup> {
        BundleResolverSetup::from_options(
            vanilla,
            &BundleModSelectionOptions {
                mods,
                use_dlc_load: false,
                userdata: None,
            },
        )
    }

    #[test]
    fn explicit_mod_missing_directory_fails_closed() {
        let root = temp_test_dir("resolver_explicit_missing_dir");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        let error = explicit_setup(vanilla, vec![root.join("absent_mod")])
            .expect_err("missing --mod directory must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("explicit --mod layer 0 refused"),
            "{message}"
        );
        assert!(message.contains("absent_mod"), "{message}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_mod_unparseable_descriptor_fails_closed() {
        let root = temp_test_dir("resolver_explicit_bad_descriptor");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("broken_mod");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&mod_root, "map/definition.csv", "modded");
        write_text(
            &mod_root,
            "descriptor.mod",
            "name=\"Broken\"\nreplace_path=\"gfx/FX\"\nreplace_path=\"gfx/FX\"\n",
        );
        assert!(
            parse_mod_descriptor(&mod_root.join("descriptor.mod")).is_err(),
            "fixture must be a descriptor the parser refuses"
        );
        let error = explicit_setup(vanilla, vec![mod_root])
            .expect_err("unparseable descriptor must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("explicit --mod layer 0 refused"),
            "{message}"
        );
        assert!(message.contains("descriptor.mod is unusable"), "{message}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_mod_with_descriptor_is_admitted_and_named_in_the_manifest() {
        let root = temp_test_dir("resolver_explicit_ok");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("good_mod");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&mod_root, "map/definition.csv", "modded");
        write_text(&mod_root, "descriptor.mod", "name=\"Good Mod\"\n");
        let setup = explicit_setup(vanilla, vec![mod_root]).expect("descriptor mod is admitted");
        let modset = setup.manifest_modset().expect("explicit modset");
        assert_eq!(modset.mod_count, 1);
        assert_ne!(modset.modset_id, "vanilla");
        assert!(setup.warnings().is_empty(), "{:?}", setup.warnings());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_mod_without_descriptor_is_admitted_but_always_reported() {
        let root = temp_test_dir("resolver_explicit_no_descriptor");
        let vanilla = root.join("vanilla");
        let mod_root = root.join("loose_dir");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        write_text(&mod_root, "map/definition.csv", "modded");
        let setup = explicit_setup(vanilla, vec![mod_root]).expect("loose dir is admitted");
        let modset = setup.manifest_modset().expect("explicit modset");
        assert_eq!(modset.mod_count, 1);
        assert_eq!(setup.warnings().len(), 1, "{:?}", setup.warnings());
        assert!(
            setup.warnings()[0].contains("has no descriptor.mod"),
            "{:?}",
            setup.warnings()
        );
        assert_eq!(modset.warnings, setup.warnings());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_modset_count_equals_requested_count() {
        let root = temp_test_dir("resolver_explicit_count");
        let vanilla = root.join("vanilla");
        write_text(&vanilla, "map/definition.csv", "vanilla");
        let mut mods = Vec::new();
        for index in 0..3 {
            let mod_root = root.join(format!("mod{index}"));
            write_text(&mod_root, "map/definition.csv", "modded");
            write_text(
                &mod_root,
                "descriptor.mod",
                &format!("name=\"Mod {index}\"\n"),
            );
            mods.push(mod_root);
        }
        let requested = mods.len();
        let setup = explicit_setup(vanilla, mods).expect("all layers admitted");
        assert_eq!(
            setup.manifest_modset().expect("modset").mod_count,
            requested
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn launcher_dlc_selection_mounts_only_non_disabled_content() {
        let root = temp_test_dir("resolver_dlc_selection");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc001_enabled/dlc001.dlc",
            "name=\"Enabled\"\nreplace_path=\"common\"\n",
        );
        write_text(&game, "dlc/dlc001_enabled/common/value.txt", "enabled-dlc");
        write_text(
            &game,
            "dlc/dlc002_disabled/dlc002.dlc",
            "name=\"Disabled\"\n",
        );
        write_text(
            &game,
            "dlc/dlc002_disabled/common/disabled.txt",
            "must-not-mount",
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":["dlc/dlc002_disabled/dlc002.dlc"]}"#,
        );

        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "value")
                    .unwrap()
            )
            .unwrap(),
            "enabled-dlc"
        );
        assert!(resolver.resolve("common/disabled.txt").unwrap().is_none());
        let sources = resolver.active_sources();
        assert_eq!(
            sources
                .iter()
                .map(|row| row.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "dlc"]
        );
        assert_eq!(sources[1].source_id, "dlc/dlc001_enabled/dlc001.dlc");
        assert_eq!(
            resolver
                .mounted_dlc_selection()
                .iter()
                .map(|row| row.dlc_id.as_str())
                .collect::<Vec<_>>(),
            vec!["dlc001_enabled"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn integrated_dlc_mounts_content_beside_descriptor_despite_legacy_path() {
        let root = temp_test_dir("resolver_integrated_dlc_legacy_path");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "integrated_dlc/dlc018_together_for_victory/dlc018.dlc",
            "name=\"Together for Victory\"\npath=\"dlc/dlc018_together_for_victory\"\nreplace_path=\"common\"\n",
        );
        write_text(
            &game,
            "integrated_dlc/dlc018_together_for_victory/common/value.txt",
            "integrated-dlc",
        );
        write_text(
            &game,
            "dlc/dlc099_selected/dlc099.dlc",
            "name=\"Selected\"\npath=\"dlc/dlc099_selected\"\n",
        );
        write_text(
            &game,
            "dlc/dlc099_selected/common/value.txt",
            "selected-dlc",
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );

        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "integrated winner")
                    .unwrap()
            )
            .unwrap(),
            "selected-dlc"
        );
        let source = &resolver.active_sources()[1];
        assert_eq!(source.kind, "integrated_dlc");
        assert_eq!(
            source.source_id,
            "integrated_dlc/dlc018_together_for_victory/dlc018.dlc"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_dlc_ties_use_exact_descriptor_id_and_later_layer_wins() {
        let root = temp_test_dir("resolver_dlc_stable_id_order");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        for (id, value) in [("dlc_z", "z"), ("dlc_a", "a")] {
            write_text(
                &game,
                &format!("dlc/{id}/{id}.dlc"),
                &format!("name=\"{id}\"\npath=\"dlc/{id}\"\n"),
            );
            write_text(&game, &format!("dlc/{id}/common/value.txt"), value);
        }
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            resolver
                .active_sources()
                .iter()
                .map(|source| source.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "dlc/dlc_a/dlc_a.dlc", "dlc/dlc_z/dlc_z.dlc"]
        );
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "stable-id winner")
                    .unwrap()
            )
            .unwrap(),
            "z"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_dlc_dependency_lowers_named_layer_before_dependent() {
        let root = temp_test_dir("resolver_dlc_dependency_order");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc_z/dlc_z.dlc",
            "name=\"Core\"\npath=\"dlc/dlc_z\"\n",
        );
        write_text(&game, "dlc/dlc_z/common/value.txt", "core");
        write_text(
            &game,
            "dlc/dlc_a/dlc_a.dlc",
            "name=\"Dependent\"\npath=\"dlc/dlc_a\"\ndependencies={ \"Core\" }\n",
        );
        write_text(&game, "dlc/dlc_a/common/value.txt", "dependent");
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            resolver
                .active_sources()
                .iter()
                .map(|source| source.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["base", "dlc/dlc_z/dlc_z.dlc", "dlc/dlc_a/dlc_a.dlc"]
        );
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "dependency winner")
                    .unwrap()
            )
            .unwrap(),
            "dependent"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn planned_mod_wins_over_selected_dlc() {
        let root = temp_test_dir("resolver_mod_over_selected_dlc");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc_z/dlc_z.dlc",
            "name=\"DLC\"\npath=\"dlc/dlc_z\"\n",
        );
        write_text(&game, "dlc/dlc_z/common/value.txt", "dlc");
        write_text(&userdata, "mod/override/common/value.txt", "mod");
        write_text(
            &userdata,
            "mod/override.mod",
            "name=\"Override\"\npath=\"mod/override\"\n",
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/override.mod"],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "mod winner")
                    .unwrap()
            )
            .unwrap(),
            "mod"
        );
        assert_eq!(resolver.active_sources().last().unwrap().kind, "mod");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn later_selected_dlc_replace_path_blocks_lower_exact_parent_fallback() {
        let root = temp_test_dir("resolver_dlc_replace_path");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc_a/dlc_a.dlc",
            "name=\"A\"\npath=\"dlc/dlc_a\"\n",
        );
        write_text(&game, "dlc/dlc_a/common/lower.txt", "lower");
        write_text(
            &game,
            "dlc/dlc_z/dlc_z.dlc",
            "name=\"Z\"\npath=\"dlc/dlc_z\"\nreplace_path=\"common\"\n",
        );
        write_text(&game, "dlc/dlc_z/gfx/unique.txt", "z");
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert!(resolver.resolve("common/lower.txt").unwrap().is_none());
        write_text(
            resolver.game_dir(),
            "dlc/dlc_a/common/sub/nested.txt",
            "nested",
        );
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/sub/nested.txt", "nested fallback")
                    .unwrap()
            )
            .unwrap(),
            "nested"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_archive_dlc_participates_in_same_planner_order() {
        let root = temp_test_dir("resolver_selected_archive_dlc");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc003/dlc003.dlc",
            "name=\"Archive DLC\"\narchive=\"dlc/dlc003/dlc003.zip\"\n",
        );
        let archive_path = game.join("dlc/dlc003/dlc003.zip");
        let file = File::create(&archive_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        use std::io::Write as _;
        writer
            .start_file("common/value.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"archive-dlc").unwrap();
        writer.finish().unwrap();
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "archive DLC winner")
                    .unwrap()
            )
            .unwrap(),
            "archive-dlc"
        );
        assert!(resolver.active_sources()[1].archive_sha256.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn selected_dlc_order_is_independent_of_install_location() {
        let root = temp_test_dir("resolver_dlc_relocated");
        let mut orders = Vec::new();
        for machine in ["machine-z", "machine-a"] {
            let game = root.join(machine).join("game");
            let userdata = root.join(machine).join("userdata");
            write_text(&game, "common/value.txt", "base");
            for (id, value) in [("dlc_z", "z"), ("dlc_a", "a")] {
                write_text(
                    &game,
                    &format!("dlc/{id}/{id}.dlc"),
                    &format!("name=\"{id}\"\npath=\"dlc/{id}\"\n"),
                );
                write_text(&game, &format!("dlc/{id}/common/value.txt"), value);
            }
            write_text(
                &userdata,
                "dlc_load.json",
                r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
            );
            let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
            orders.push(
                resolver
                    .active_sources()
                    .iter()
                    .map(|source| source.source_id.clone())
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(orders[0], orders[1]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn required_subset_preserves_sparse_recorded_ordinals() {
        let root = temp_test_dir("resolver_sparse_ordinals");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        for (id, value) in [("a", "A"), ("unused", "unused"), ("z", "Z")] {
            write_text(&userdata, &format!("mod/{id}/common/value.txt"), value);
            write_text(
                &userdata,
                &format!("mod/{id}.mod"),
                &format!("name=\"{id}\"\npath=\"mod/{id}\"\n"),
            );
        }
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/a.mod","mod/unused.mod","mod/z.mod"],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_recorded_content_layers(
            game,
            userdata,
            &[
                RecordedContentSource {
                    ordinal: 3,
                    source_id: "mod/a.mod".to_string(),
                },
                RecordedContentSource {
                    ordinal: 9,
                    source_id: "mod/z.mod".to_string(),
                },
            ],
        )
        .unwrap();
        assert_eq!(
            resolver
                .active_sources()
                .iter()
                .map(|row| row.ordinal)
                .collect::<Vec<_>>(),
            vec![0, 3, 9]
        );
        let winner = resolver
            .resolve_required_with_provenance("common/value.txt", "sparse winner")
            .unwrap();
        assert_eq!(winner.source.ordinal, 9);
        assert_eq!(winner.source.source_id, "mod/z.mod");
        assert!(!resolver
            .active_sources()
            .iter()
            .any(|row| row.source_id == "mod/unused.mod"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn required_subset_never_opens_corrupt_or_missing_unused_active_layers() {
        let root = temp_test_dir("resolver_required_ignores_bad_unused");
        let game = root.join("game");
        let userdata = root.join("userdata");
        write_text(&game, "common/value.txt", "base");
        write_text(
            &game,
            "dlc/dlc001_bad/dlc001.dlc",
            "name=\"unused corrupt DLC\"\npath=\"missing\"\narchive=\"missing.zip\"",
        );
        write_text(&userdata, "mod/required/common/value.txt", "required");
        write_text(
            &userdata,
            "mod/required.mod",
            "name=\"required\"\npath=\"mod/required\"",
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/missing-unused.mod","mod/required.mod"],"disabled_dlcs":[]}"#,
        );

        let full = GameFileResolver::from_dlc_load(game.clone(), userdata.clone())
            .expect_err("the full local selection contains deliberately bad unused content");
        assert!(
            full.to_string()
                .contains("descriptor has both path and archive"),
            "{full}"
        );

        let resolver = GameFileResolver::from_recorded_content_layers(
            game,
            userdata,
            &[RecordedContentSource {
                ordinal: 7,
                source_id: "mod/required.mod".to_string(),
            }],
        )
        .expect("required-only playback must never open unused active layers");
        assert_eq!(
            resolver
                .active_sources()
                .iter()
                .map(|row| (row.ordinal, row.source_id.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "base"), (7, "mod/required.mod")]
        );
        assert_eq!(
            read_latin1(
                &resolver
                    .resolve_required("common/value.txt", "required winner")
                    .unwrap()
            )
            .unwrap(),
            "required"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn semantic_descriptor_hash_ignores_machine_specific_content_path() {
        let left = r#"
            name="Portable"
            path="C:/Users/Example/Documents/HOI4/mod/portable"
            version="1"
            supported_version="1.19.*"
            remote_file_id="42"
            replace_path="INTERFACE"
            dependencies={ "Core" "Library" }
        "#;
        let right = r#"
            # copied to another machine
            name = "Portable"
            path = "D:/Games/Mods/portable"
            version = "1"
            supported_version = "1.19.*"
            remote_file_id = "42"
            replace_path = "interface"
            dependencies = { "Core" "Library" }
        "#;
        assert_eq!(
            semantic_descriptor_sha256(left),
            semantic_descriptor_sha256(right)
        );
        assert_eq!(semantic_descriptor_sha256(left).len(), 64);
        let root = temp_test_dir("semantic_descriptor_two_roots");
        let mut hashes = Vec::new();
        for machine in ["machine-a", "machine-b"] {
            let game = root.join(machine).join("game");
            let userdata = root.join(machine).join("profile");
            let content = root.join(machine).join("external-mod-content");
            write_text(&game, "common/base.txt", "base");
            write_text(&content, "interface/portable.gui", "portable");
            let content_path = content.to_string_lossy().replace('\\', "/");
            write_text(
                &userdata,
                "mod/portable.mod",
                &format!("name=\"Portable\"\npath=\"{content_path}\"\nversion=\"1\"\nsupported_version=\"1.19.*\"\nremote_file_id=\"42\"\nreplace_path=\"interface\"\ndependencies={{ \"Core\" \"Library\" }}\n"),
            );
            write_text(
                &userdata,
                "dlc_load.json",
                r#"{"enabled_mods":["mod/portable.mod"],"disabled_dlcs":[]}"#,
            );
            let resolver = GameFileResolver::from_dlc_load(game, userdata).unwrap();
            assert!(resolver
                .resolve("interface/portable.gui")
                .unwrap()
                .is_some());
            hashes.push(
                resolver.active_sources()[1]
                    .descriptor_sha256
                    .clone()
                    .unwrap(),
            );
        }
        assert_eq!(hashes[0], hashes[1]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn public_vfs_routes_reject_absolute_drive_parent_and_nul_paths() {
        let root = temp_test_dir("resolver_unsafe_public_paths");
        let game = root.join("game");
        write_text(&game, "interface/safe.gui", "safe");
        let resolver = GameFileResolver::vanilla(game).unwrap();

        for unsafe_path in [
            "/absolute/interface.gui",
            "C:/outside/interface.gui",
            "../outside/interface.gui",
            "interface/../../outside.gui",
            "interface/evil\0.gui",
        ] {
            assert!(resolver.resolve(unsafe_path).is_err(), "{unsafe_path:?}");
            assert!(
                resolver.resolve_dir_files(unsafe_path, None).is_err(),
                "{unsafe_path:?}"
            );
            assert!(
                resolver
                    .resolve_dir_files_recursive(unsafe_path, None)
                    .is_err(),
                "{unsafe_path:?}"
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recursive_vfs_rejects_directory_link_escape_when_link_creation_is_available() {
        let root = temp_test_dir("resolver_directory_link_escape");
        let game = root.join("game");
        let outside = root.join("outside");
        write_text(&game, "interface/safe.gui", "safe");
        write_text(&outside, "escaped.gui", "outside");
        let link = game.join("interface/linked");

        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&outside, &link).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let linked = false;

        if linked {
            let resolver = GameFileResolver::vanilla(&game).unwrap();
            let error = resolver
                .resolve_dir_files_recursive("interface", Some("gui"))
                .expect_err("directory link escaping the mounted root must fail closed");
            assert!(
                error.to_string().contains("escapes mounted root"),
                "{error}"
            );
        }

        assert!(LayerScanCache::default()
            .ensure_within_layer(&game, &outside, "test outside path")
            .expect_err("direct outside candidate must fail")
            .to_string()
            .contains("escapes mounted root"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn portable_manifest_inventory_and_admission_deny_unrecorded_content() {
        let root = temp_test_dir("resolver_portable_admission");
        let game = root.join("relocated-game");
        let userdata = root.join("relocated-profile");
        write_text(&game, "interface/base.gui", "base-ui");
        write_text(&game, "interface/unrecorded.gui", "local-only");
        write_text(&userdata, "mod/content/interface/base.gui", "mod-ui");
        write_text(
            &userdata,
            "mod/content/interface/unrecorded.gui",
            "unrecorded-mod-winner",
        );
        write_text(
            &userdata,
            "mod/portable.mod",
            "name=\"Portable\"\npath=\"mod/content\"\nreplace_path=\"interface\"\n",
        );
        write_text(
            &userdata,
            "dlc_load.json",
            r#"{"enabled_mods":["mod/portable.mod"],"disabled_dlcs":[]}"#,
        );
        let resolver = GameFileResolver::from_recorded_content_order(
            game,
            userdata,
            &["mod/portable.mod".to_string()],
        )
        .unwrap();
        let actual = resolver
            .resolve_required_with_provenance("interface/base.gui", "UI")
            .unwrap();
        assert_eq!(actual.source.kind, "mod");
        assert_eq!(actual.source.source_id, "mod/portable.mod");

        let game_id = replay_core::SourceGameIdentity {
            version: "1.19.2.0".to_string(),
            build: "a729".to_string(),
            executable_sha256: "11".repeat(32),
            build_profile: "steam".to_string(),
        };
        let mut profile_id = replay_core::SourceProfileIdentity {
            profile_id: "profile".to_string(),
            playset_id: "playset".to_string(),
            selection_sha256: "22".repeat(32),
        };
        let vfs_digest = "33".repeat(32);
        let mut manifest = replay_core::ContentManifest {
            schema: replay_core::PORTABLE_CONTENT_MANIFEST_SCHEMA.to_string(),
            product_modset_sha256: "99".repeat(32),
            source_game: game_id.clone(),
            source_profile: profile_id.clone(),
            effective_vfs_manifest_sha256: vfs_digest.clone(),
            ordered_layers: vec![
                replay_core::RecordedContentLayer {
                    ordinal: 0,
                    layer_kind: replay_core::ContentLayerKind::Base,
                    layer_id: "base".to_string(),
                    display_name: "Base".to_string(),
                    descriptor_sha256: None,
                    source_container_sha256: None,
                    storage_kind: replay_core::LayerStorageKind::Directory,
                    replace_paths: Vec::new(),
                    required_for_playback: false,
                },
                replay_core::RecordedContentLayer {
                    ordinal: 1,
                    layer_kind: replay_core::ContentLayerKind::Mod,
                    layer_id: "mod/portable.mod".to_string(),
                    display_name: "Portable".to_string(),
                    descriptor_sha256: actual.source.descriptor_sha256.clone(),
                    source_container_sha256: None,
                    storage_kind: replay_core::LayerStorageKind::Directory,
                    replace_paths: vec!["interface".to_string()],
                    required_for_playback: true,
                },
            ],
            dlc_selection: Vec::new(),
            localisation_language: "l_english".to_string(),
            required_settings: BTreeMap::from([("renderer".to_string(), "dx11".to_string())]),
            recorder_local_settings: BTreeMap::new(),
            required_content: vec![replay_core::RequiredContent {
                content_key: "ui-base".to_string(),
                kind: replay_core::RequiredContentKind::GuiDefinition,
                content: replay_core::UsedContentKey {
                    effective_vfs_manifest_sha256: vfs_digest,
                    normalized_game_relative_path: "interface/base.gui".to_string(),
                    winning_layer_ordinal: 1,
                    layer_kind: replay_core::ContentLayerKind::Mod,
                    layer_id: "mod/portable.mod".to_string(),
                    source_sha256: actual.sha256.clone(),
                    payload_sha256: actual.sha256.clone(),
                    payload_bytes: actual.byte_len,
                },
                observation_count: 1,
                consumers: vec!["production-ui".to_string()],
            }],
            manifest_digest_sha256: None,
        };
        let selection = manifest.computed_selection_sha256().unwrap();
        manifest.source_profile.selection_sha256 = selection.clone();
        profile_id.selection_sha256 = selection;
        manifest = manifest.with_computed_digest().unwrap();
        let inventory = resolver
            .playback_inventory(
                &manifest,
                PlaybackEnvironment {
                    source_game: game_id,
                    source_profile: profile_id,
                    dlc_selection: Vec::new(),
                    localisation_language: "l_english".to_string(),
                    settings: BTreeMap::from([("renderer".to_string(), "dx11".to_string())]),
                },
            )
            .unwrap();
        let admission = manifest.admit_playback(&inventory).unwrap();
        let admitted_key = admission
            .require_used_content("interface/base.gui")
            .unwrap()
            .clone();
        let resolver = resolver.with_content_admission(&admission).unwrap();
        write_text(
            &root.join("relocated-profile/mod/content"),
            "interface/base.gui",
            "mutated-after-admission",
        );
        assert_eq!(
            resolver.read_admitted_used_content(&admitted_key).unwrap(),
            b"mod-ui"
        );
        assert_eq!(
            portable_test_fs_retry::read(
                resolver
                    .resolve_required("interface/base.gui", "UI")
                    .unwrap()
            )
            .unwrap(),
            b"mod-ui"
        );
        let mut wrong_key = admitted_key.clone();
        wrong_key.payload_sha256 = "44".repeat(32);
        assert!(resolver
            .read_admitted_used_content(&wrong_key)
            .unwrap_err()
            .to_string()
            .contains("disagrees with the validated admission"));
        assert!(resolver.resolve("interface/base.gui").unwrap().is_some());
        assert!(resolver
            .resolve("interface/unrecorded.gui")
            .unwrap_err()
            .to_string()
            .contains("rejects unrecorded/default lookup"));
        assert_eq!(
            resolver
                .resolve_dir_files_recursive("interface", Some("gui"))
                .unwrap()
                .len(),
            1
        );
        portable_test_fs_retry::remove_file(root.join("relocated-profile/mod/content/interface/unrecorded.gui"))
            .expect("delete live omitted winner after admission");
        write_text(
            &root.join("relocated-profile/mod/content"),
            "interface/added-after-admission.gui",
            "late file",
        );
        let audited = resolver
            .audit_effective_dir_files_recursive("interface", Some("gui"))
            .unwrap();
        assert!(
            audited
                .iter()
                .any(|path| path.eq_ignore_ascii_case("interface/unrecorded.gui")),
            "{audited:?}"
        );
        assert!(!audited
            .iter()
            .any(|path| path.eq_ignore_ascii_case("interface/added-after-admission.gui")));
        let census_error = resolver
            .require_complete_admitted_dir("interface", Some("gui"), true, "test UI namespace")
            .expect_err("omitted effective mod winner must reject the namespace");
        assert!(census_error.to_string().contains("unrecorded.gui"));
        let snapshot_root = resolver.admission.as_ref().unwrap().snapshot_root.clone();
        assert!(snapshot_root.is_dir());
        drop(resolver);
        assert!(!snapshot_root.exists());
        let _ = fs::remove_dir_all(&root);
    }
}

impl RestrictedGameFileResolver<'_> {
    pub fn resolve_required(&self, rel_path: &str) -> Result<ResolvedGameFile> {
        let logical = checked_rel_path(rel_path)?;
        let expected = self
            .allowed
            .get(&logical.to_ascii_lowercase())
            .ok_or_else(|| format!("replay did not authorize content path: {logical}"))?;
        let actual = self
            .resolver
            .resolve_required_with_provenance(&logical, "required replay content")?;
        if actual.sha256 != expected.sha256 || actual.byte_len != expected.byte_len {
            return Err(format!(
                "required content mismatch for {logical}: replay sha256={} bytes={}, playback sha256={} bytes={}",
                expected.sha256, expected.byte_len, actual.sha256, actual.byte_len
            )
            .into());
        }
        Ok(actual)
    }

    pub fn resolve_dir_files_recursive(
        &self,
        rel_dir: &str,
        extension: Option<&str>,
    ) -> Result<Vec<ResolvedGameFile>> {
        let dir = checked_rel_path(rel_dir)?.to_ascii_lowercase();
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let wanted = extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        let mut out = Vec::new();
        for row in self.allowed.values() {
            let folded = row.logical_path.to_ascii_lowercase();
            if !folded.starts_with(&prefix) {
                continue;
            }
            if let Some(ext) = &wanted {
                let actual = Path::new(&folded)
                    .extension()
                    .and_then(|value| value.to_str())
                    .map(str::to_ascii_lowercase);
                if actual.as_deref() != Some(ext.as_str()) {
                    continue;
                }
            }
            out.push(self.resolve_required(&row.logical_path)?);
        }
        out.sort_by(|left, right| left.logical_path.cmp(&right.logical_path));
        Ok(out)
    }
}

const UI14_ENGINE_VFS_SCHEMA: &str = "extractor.ui14_engine_vfs.v1";
const UI14_ENGINE_VFS_FILE: &str = "ui14_engine_vfs.json";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ui14EngineVfsReceipt {
    schema: String,
    executable_sha256: String,
    layers: Vec<Ui14EngineLayer>,
    effective_winners: Vec<Ui14EffectiveWinner>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ui14EngineLayer {
    id: String,
    descriptor_sha256: String,
    engine_descriptor_ordinal: u64,
    engine_mount_ordinal: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ui14EffectiveWinner {
    relpath: String,
    winner_id: String,
    sha256: String,
}
