
use crate::grading_law::{
    ExactGradeEndpointPair, ExactPerFrameGradeProgram, GraphicsGradingDefines,
};
use crate::{GameFileResolver, Result};
use portable_content_manifest::UsedContentKey;
use replay_core::ValidatedContentAdmission;
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const XMAP10_PREPARATION_SCHEMA: &str = "hoi4.xmap10.postfx-preparation.a729.v1";
pub const XMAP10_TEMPORAL_RECEIPT_SCHEMA: &str = "hoi4.xmap10.temporal-adaptation-receipt.a729.v1";
pub const XMAP10_SCENE_CB0_RECEIPT_SCHEMA: &str = "hoi4.xmap10.scene-cb0-prepared-receipt.a729.v1";
pub const XMAP10_LUT_BLEND_PROGRAM_SCHEMA: &str = "hoi4.xmap10.lut-blend-program.a729.v1";
pub const XMAP10_NATIVE_PROGRAM_RECEIPT_SCHEMA: &str = "hoi4.xmap10.native-program-receipt.a729.v1";
pub const XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA: &str =
    "hoi4.xmap10.native-compositor-receipt.a729.v1";
pub const XMAP10_READY_SCHEMA: &str = "hoi4.xmap10.postfx-ready.a729.v1";

const POSTEFFECT_VOLUMES: &str = "gfx/posteffect_volumes.txt";
const GRAPHICS_DEFINES: &str = "common/defines/00_graphics.lua";
const EFFECT_ROOTS: [&str; 8] = [
    "gfx/FX/sky.shader",
    "gfx/FX/downsample.shader",
    "gfx/FX/downsample_luminance.shader",
    "gfx/FX/bloom.shader",
    "gfx/FX/restorescene.shader",
    "gfx/FX/lut_blender.shader",
    "gfx/FX/pdxmap.shader",
    "gfx/FX/shadowblur.shader",
];

const DXGI_FORMAT_R16G16B16A16_FLOAT: u32 = 10;
const DXGI_FORMAT_R8G8B8A8_UNORM: u32 = 28;
const DXGI_FORMAT_R16_FLOAT: u32 = 54;
const DXGI_FORMAT_B8G8R8A8_UNORM: u32 = 87;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ShaderTranslationUnit {
    pub root_path: String,
    pub ordered_source_paths: Vec<String>,
    pub deterministic_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ContentClosure {
    pub content_manifest_sha256: String,
    pub effective_vfs_manifest_sha256: String,
    pub product_modset_sha256: String,
    pub posteffect_volumes: UsedContentKey,
    pub graphics_defines: UsedContentKey,
    pub selected_luts: Vec<UsedContentKey>,
    pub shader_translation_units: Vec<Xmap10ShaderTranslationUnit>,
    pub ordered_used_content: Vec<UsedContentKey>,
    pub deterministic_sha256: String,
}

impl Xmap10ContentClosure {
    pub fn admit(
        resolver: &GameFileResolver,
        admission: &ValidatedContentAdmission,
        selected_lut_paths: &[String],
    ) -> Result<Self> {
        if selected_lut_paths.is_empty() {
            return Err("XMAP10 requires at least one selected effective-VFS LUT".into());
        }

        let mut unique_luts = BTreeSet::new();
        let mut normalized_luts = Vec::with_capacity(selected_lut_paths.len());
        for path in selected_lut_paths {
            let normalized = normalize_content_path(path)?;
            if !normalized.starts_with("gfx/world/") || !normalized.ends_with(".tga") {
                return Err(
                    format!("XMAP10 selected LUT path is not a gfx/world TGA: {path}").into(),
                );
            }
            if !unique_luts.insert(normalized.clone()) {
                return Err("XMAP10 selected LUT list contains a duplicate path".into());
            }
            normalized_luts.push(normalized);
        }

        let mut translation_units = Vec::with_capacity(EFFECT_ROOTS.len());
        let mut ordered_paths = Vec::new();
        let mut seen_paths = BTreeSet::new();
        for root in EFFECT_ROOTS {
            let files = crate::shader_compile::effect_translation_unit_files(resolver, root)?;
            let paths = files
                .into_iter()
                .map(|(path, _)| normalize_content_path(&path))
                .collect::<Result<Vec<_>>>()?;
            if paths.is_empty()
                || paths.last().map(String::as_str) != Some(&root.to_ascii_lowercase())
            {
                return Err(format!("XMAP10 effect closure for {root} omitted its root").into());
            }
            let unit = Xmap10ShaderTranslationUnit {
                root_path: root.to_ascii_lowercase(),
                ordered_source_paths: paths,
                deterministic_sha256: String::new(),
            };
            for path in &unit.ordered_source_paths {
                if seen_paths.insert(path.clone()) {
                    ordered_paths.push(path.clone());
                }
            }
            translation_units.push(unit);
        }
        for path in [POSTEFFECT_VOLUMES, GRAPHICS_DEFINES, crate::shader_compile::DX11_PLATFORM_HEADER] {
            let normalized = path.to_ascii_lowercase();
            if seen_paths.insert(normalized.clone()) {
                ordered_paths.push(normalized);
            }
        }
        for path in &normalized_luts {
            if seen_paths.insert(path.clone()) {
                ordered_paths.push(path.clone());
            }
        }

        let mut ordered_used_content = Vec::with_capacity(ordered_paths.len());
        for path in &ordered_paths {
            let key = admission
                .require_used_content(path)
                .map_err(|error| format!("XMAP10 requires admitted {path}: {error}"))?
                .clone();
            resolver.read_admitted_used_content(&key)?;
            ordered_used_content.push(key);
        }
        let by_path = ordered_used_content
            .iter()
            .map(|key| (key.normalized_game_relative_path.as_str(), key))
            .collect::<BTreeMap<_, _>>();
        for unit in &mut translation_units {
            unit.deterministic_sha256 = shader_unit_digest(unit, &by_path)?;
        }
        let posteffect_volumes = (**by_path
            .get(POSTEFFECT_VOLUMES)
            .ok_or("XMAP10 admitted closure omitted posteffect_volumes.txt")?)
        .clone();
        let graphics_defines = (**by_path
            .get(GRAPHICS_DEFINES)
            .ok_or("XMAP10 admitted closure omitted 00_graphics.lua")?)
        .clone();
        let selected_luts = normalized_luts
            .iter()
            .map(|path| {
                by_path
                    .get(path.as_str())
                    .map(|key| (**key).clone())
                    .ok_or_else(|| {
                        format!("XMAP10 admitted closure omitted selected LUT {path}").into()
                    })
            })
            .collect::<Result<Vec<_>>>()?;

        let effective_vfs_manifest_sha256 = ordered_used_content
            .first()
            .ok_or("XMAP10 admitted content closure is empty")?
            .effective_vfs_manifest_sha256
            .clone();
        let mut closure = Self {
            content_manifest_sha256: admission.manifest_digest_sha256().to_string(),
            effective_vfs_manifest_sha256,
            product_modset_sha256: admission.product_modset_sha256().to_string(),
            posteffect_volumes,
            graphics_defines,
            selected_luts,
            shader_translation_units: translation_units,
            ordered_used_content,
            deterministic_sha256: String::new(),
        };
        closure.deterministic_sha256 = content_closure_digest(&closure);
        closure.validate()?;
        Ok(closure)
    }

    pub fn validate(&self) -> Result<()> {
        validate_sha256(&self.content_manifest_sha256, "XMAP10 content manifest")?;
        validate_sha256(
            &self.effective_vfs_manifest_sha256,
            "XMAP10 effective-VFS manifest",
        )?;
        validate_sha256(&self.product_modset_sha256, "XMAP10 product modset")?;
        if self.selected_luts.is_empty()
            || self.shader_translation_units.len() != EFFECT_ROOTS.len()
            || self.ordered_used_content.is_empty()
        {
            return Err("XMAP10 content closure is incomplete".into());
        }

        let mut paths = BTreeSet::new();
        let mut by_path = BTreeMap::new();
        for key in &self.ordered_used_content {
            key.validate()
                .map_err(|error| format!("XMAP10 used-content identity: {error}"))?;
            if key.effective_vfs_manifest_sha256 != self.effective_vfs_manifest_sha256
                || !paths.insert(key.normalized_game_relative_path.as_str())
            {
                return Err(
                    "XMAP10 used-content set has a VFS identity mismatch or duplicate".into(),
                );
            }
            by_path.insert(key.normalized_game_relative_path.as_str(), key);
        }
        for required in [&self.posteffect_volumes, &self.graphics_defines] {
            if !self.ordered_used_content.contains(required) {
                return Err("XMAP10 scalar source is outside the admitted content closure".into());
            }
        }
        if !paths.contains(crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase().as_str()) {
            return Err("XMAP10 shader platform header is outside the admitted content closure".into());
        }
        for lut in &self.selected_luts {
            if !self.ordered_used_content.contains(lut)
                || !lut.normalized_game_relative_path.starts_with("gfx/world/")
                || !lut.normalized_game_relative_path.ends_with(".tga")
            {
                return Err("XMAP10 selected LUT is outside the admitted LUT closure".into());
            }
        }
        for (unit, expected_root) in self.shader_translation_units.iter().zip(EFFECT_ROOTS) {
            if unit.root_path != expected_root.to_ascii_lowercase()
                || unit.ordered_source_paths.is_empty()
                || unit.ordered_source_paths.last() != Some(&unit.root_path)
                || unit.deterministic_sha256 != shader_unit_digest(unit, &by_path)?
            {
                return Err("XMAP10 shader translation-unit identity is invalid".into());
            }
            for path in &unit.ordered_source_paths {
                if !paths.contains(path.as_str()) {
                    return Err(format!("XMAP10 shader dependency {path} was not admitted").into());
                }
            }
        }
        if self.deterministic_sha256 != content_closure_digest(self) {
            return Err("XMAP10 content closure digest mismatch".into());
        }
        Ok(())
    }

    fn shader_unit(&self, root: &str) -> Result<&Xmap10ShaderTranslationUnit> {
        self.shader_translation_units
            .iter()
            .find(|unit| unit.root_path.eq_ignore_ascii_case(root))
            .ok_or_else(|| format!("XMAP10 has no admitted shader unit for {root}").into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap10ProgramRole {
    BrightPass,
    LuminanceDownsample,
    LuminanceGather,
    BloomHorizontal,
    BloomVertical,
    RestoreScene,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10ActiveEffectVariant {
    pub role: Xmap10ProgramRole,
    pub shader_root: String,
    pub effect_name: String,
    pub ordered_defines: Vec<String>,
    pub translation_unit_sha256: String,
    pub deterministic_sha256: String,
}

impl Xmap10ActiveEffectVariant {
    pub fn production_hdr(content: &Xmap10ContentClosure) -> Result<Vec<Self>> {
        content.validate()?;
        [
            (
                Xmap10ProgramRole::BrightPass,
                "downsample",
                Vec::<String>::new(),
            ),
            (
                Xmap10ProgramRole::LuminanceDownsample,
                "LuminanceDownsample",
                vec!["LUMINANCE_SAMPLE_ALPHA".into()],
            ),
            (
                Xmap10ProgramRole::LuminanceGather,
                "LuminanceGather",
                Vec::new(),
            ),
            (Xmap10ProgramRole::BloomHorizontal, "bloom", Vec::new()),
            (Xmap10ProgramRole::BloomVertical, "bloom", Vec::new()),
            (
                Xmap10ProgramRole::RestoreScene,
                "RestoreBloom",
                vec![
                    "BLOOM".into(),
                    "HDR".into(),
                    "ADJUSTED_EXPOSURE".into(),
                    "COLOR_LUT".into(),
                ],
            ),
        ]
        .into_iter()
        .map(|(role, effect_name, ordered_defines)| {
            let shader_root = shader_root_for_role(role).to_string();
            let translation_unit_sha256 = content
                .shader_unit(&shader_root)?
                .deterministic_sha256
                .clone();
            let mut effect = Self {
                role,
                shader_root,
                effect_name: effect_name.into(),
                ordered_defines,
                translation_unit_sha256,
                deterministic_sha256: String::new(),
            };
            effect.deterministic_sha256 = active_effect_digest(&effect);
            Ok(effect)
        })
        .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xmap10TargetRole {
    MainScene,
    BrightPass,
    LuminancePing,
    LuminancePong,
    CurrentLuminance,
    HistoryLuminance,
    BloomHorizontal,
    RestoreBloom,
    ColorCube0,
    ColorCube1,
    ColorCube2,
    Swapchain,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10TargetDescriptor {
    pub role: Xmap10TargetRole,
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub mip_levels: u32,
    pub sample_count: u32,
    pub descriptor_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10MainSceneDepthDescriptor {
    pub width: u32,
    pub height: u32,
    pub dxgi_format: u32,
    pub sample_count: u32,
    pub sample_quality: u32,
    pub descriptor_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Xmap10PassKind {
    BrightPass,
    LuminanceDownsample(u16),
    LuminanceGather(u16),
    BloomHorizontal,
    BloomVertical,
    RestoreScene,
    SwapLuminanceHistory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmap10ViewportExtent {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10PassStep {
    pub ordinal: u16,
    pub pass: Xmap10PassKind,
    pub viewport: Option<Xmap10ViewportExtent>,
    pub reads: Vec<Xmap10TargetRole>,
    pub read_extents: Vec<Xmap10ViewportExtent>,
    pub writes: Vec<Xmap10TargetRole>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10TargetPlan {
    pub ordered_targets: Vec<Xmap10TargetDescriptor>,
    pub main_scene_depth: Xmap10MainSceneDepthDescriptor,
    pub ordered_passes: Vec<Xmap10PassStep>,
    pub deterministic_sha256: String,
}

impl Xmap10TargetPlan {
    pub fn new(main_width: u32, main_height: u32) -> Result<Self> {
        let ordered_targets = xmap10_target_descriptors(main_width, main_height)?;
        let main_scene_depth = xmap10_main_scene_depth_descriptor(main_width, main_height);
        let ordered_passes = xmap10_pass_plan(main_width, main_height)?;

        let mut plan = Self {
            ordered_targets,
            main_scene_depth,
            ordered_passes,
            deterministic_sha256: String::new(),
        };
        plan.deterministic_sha256 = target_plan_digest(&plan);
        plan.validate()?;
        Ok(plan)
    }

    pub fn validate(&self) -> Result<()> {
        let mut by_role = BTreeMap::new();
        for target in &self.ordered_targets {
            if target.width == 0
                || target.height == 0
                || target.dxgi_format == 0
                || target.mip_levels != 1
                || target.sample_count != 1
                || target.descriptor_sha256 != target_descriptor_digest(target)
                || by_role.insert(target.role, target).is_some()
            {
                return Err("XMAP10 target descriptor is invalid or duplicated".into());
            }
        }
        let required = [
            Xmap10TargetRole::MainScene,
            Xmap10TargetRole::BrightPass,
            Xmap10TargetRole::LuminancePing,
            Xmap10TargetRole::LuminancePong,
            Xmap10TargetRole::CurrentLuminance,
            Xmap10TargetRole::HistoryLuminance,
            Xmap10TargetRole::BloomHorizontal,
            Xmap10TargetRole::RestoreBloom,
            Xmap10TargetRole::ColorCube0,
            Xmap10TargetRole::ColorCube1,
            Xmap10TargetRole::ColorCube2,
            Xmap10TargetRole::Swapchain,
        ];
        if required.iter().any(|role| !by_role.contains_key(role)) {
            return Err("XMAP10 target plan is missing a required target".into());
        }
        let main = by_role[&Xmap10TargetRole::MainScene];
        let expected_depth = xmap10_main_scene_depth_descriptor(main.width, main.height);
        if self.main_scene_depth != expected_depth {
            return Err("XMAP10 MainScene depth descriptor is not exact-a729 D24S8".into());
        }
        let bright = by_role[&Xmap10TargetRole::BrightPass];
        let bloom_h = by_role[&Xmap10TargetRole::BloomHorizontal];
        let bloom_v = by_role[&Xmap10TargetRole::RestoreBloom];
        if main.dxgi_format != DXGI_FORMAT_R16G16B16A16_FLOAT
            || [bright, bloom_h, bloom_v].iter().any(|target| {
                target.width != main.width / 2
                    || target.height != main.height / 2
                    || target.dxgi_format != DXGI_FORMAT_R16G16B16A16_FLOAT
            })
        {
            return Err(
                "XMAP10 HDR/bloom target formats or half-resolution law are invalid".into(),
            );
        }
        let quarter_width = (bright.width + 1) / 2;
        let quarter_height = (bright.height + 1) / 2;
        for role in [
            Xmap10TargetRole::LuminancePing,
            Xmap10TargetRole::LuminancePong,
        ] {
            let target = by_role[&role];
            if (target.width, target.height, target.dxgi_format)
                != (quarter_width, quarter_height, DXGI_FORMAT_R16_FLOAT)
            {
                return Err(
                    "XMAP10 luminance ping-pong allocation is not exact quarter extent".into(),
                );
            }
        }
        for role in [
            Xmap10TargetRole::CurrentLuminance,
            Xmap10TargetRole::HistoryLuminance,
        ] {
            let target = by_role[&role];
            if (target.width, target.height, target.dxgi_format) != (1, 1, DXGI_FORMAT_R16_FLOAT) {
                return Err(
                    "XMAP10 current/history luminance target is not distinct 1x1 R16_FLOAT".into(),
                );
            }
        }
        for role in [
            Xmap10TargetRole::ColorCube0,
            Xmap10TargetRole::ColorCube1,
            Xmap10TargetRole::ColorCube2,
        ] {
            let target = by_role[&role];
            if (target.width, target.height, target.dxgi_format)
                != (1024, 32, DXGI_FORMAT_R8G8B8A8_UNORM)
            {
                return Err("XMAP10 ColorCube target is not exact 1024x32 RGBA8".into());
            }
        }
        let swapchain = by_role[&Xmap10TargetRole::Swapchain];
        if (swapchain.width, swapchain.height) != (main.width, main.height) {
            return Err("XMAP10 RestoreScene/swapchain target join is invalid".into());
        }
        if swapchain.dxgi_format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err("XMAP10 final RestoreScene target is not BGRA8_UNORM".into());
        }
        let expected_targets = xmap10_target_descriptors(main.width, main.height)?;
        if self.ordered_targets != expected_targets {
            return Err(
                "XMAP10 physical target set differs from the exact runtime allocation".into(),
            );
        }
        let expected_passes = xmap10_pass_plan(main.width, main.height)?;
        if self.ordered_passes != expected_passes {
            return Err("XMAP10 pass graph differs from the required final-image order".into());
        }
        if self.deterministic_sha256 != target_plan_digest(self) {
            return Err("XMAP10 target-plan digest is invalid".into());
        }
        Ok(())
    }

    pub fn production_main_scene_desc(&self) -> Result<crate::ProductionMainSceneTargetDesc> {
        self.validate()?;
        let (color_index, main) = self
            .ordered_targets
            .iter()
            .enumerate()
            .find(|(_, target)| target.role == Xmap10TargetRole::MainScene)
            .ok_or("XMAP10 target plan omitted MainScene")?;
        let color_attachment_ordinal = u32::try_from(color_index)?;
        let depth_attachment_ordinal = u32::try_from(self.ordered_targets.len())?;
        let desc = crate::ProductionMainSceneTargetDesc {
            color_attachment_ordinal,
            color_width: main.width,
            color_height: main.height,
            color_format: main.dxgi_format,
            color_samples: main.sample_count,
            color_quality: 0,
            depth_attachment_ordinal,
            depth_width: self.main_scene_depth.width,
            depth_height: self.main_scene_depth.height,
            depth_format: self.main_scene_depth.dxgi_format,
            depth_samples: self.main_scene_depth.sample_count,
            depth_quality: self.main_scene_depth.sample_quality,
        };
        desc.validate()?;
        Ok(desc)
    }
}

fn xmap10_main_scene_depth_descriptor(width: u32, height: u32) -> Xmap10MainSceneDepthDescriptor {
    let mut descriptor = Xmap10MainSceneDepthDescriptor {
        width,
        height,
        dxgi_format: 45,
        sample_count: 1,
        sample_quality: 0,
        descriptor_sha256: String::new(),
    };
    descriptor.descriptor_sha256 = main_scene_depth_descriptor_digest(&descriptor);
    descriptor
}

fn xmap10_target_descriptors(
    main_width: u32,
    main_height: u32,
) -> Result<Vec<Xmap10TargetDescriptor>> {
    if main_width < 8 || main_height < 8 {
        return Err("XMAP10 requires a nontrivial full-resolution scene".into());
    }
    let half_width = main_width / 2;
    let half_height = main_height / 2;
    let quarter_width = (half_width + 1) / 2;
    let quarter_height = (half_height + 1) / 2;
    let mut targets = vec![
        target_descriptor(
            Xmap10TargetRole::MainScene,
            main_width,
            main_height,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::BrightPass,
            half_width,
            half_height,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::LuminancePing,
            quarter_width,
            quarter_height,
            DXGI_FORMAT_R16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::LuminancePong,
            quarter_width,
            quarter_height,
            DXGI_FORMAT_R16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::CurrentLuminance,
            1,
            1,
            DXGI_FORMAT_R16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::HistoryLuminance,
            1,
            1,
            DXGI_FORMAT_R16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::BloomHorizontal,
            half_width,
            half_height,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::RestoreBloom,
            half_width,
            half_height,
            DXGI_FORMAT_R16G16B16A16_FLOAT,
        ),
        target_descriptor(
            Xmap10TargetRole::ColorCube0,
            1024,
            32,
            DXGI_FORMAT_R8G8B8A8_UNORM,
        ),
        target_descriptor(
            Xmap10TargetRole::ColorCube1,
            1024,
            32,
            DXGI_FORMAT_R8G8B8A8_UNORM,
        ),
        target_descriptor(
            Xmap10TargetRole::ColorCube2,
            1024,
            32,
            DXGI_FORMAT_R8G8B8A8_UNORM,
        ),
        target_descriptor(
            Xmap10TargetRole::Swapchain,
            main_width,
            main_height,
            DXGI_FORMAT_B8G8R8A8_UNORM,
        ),
    ];
    for target in &mut targets {
        target.descriptor_sha256 = target_descriptor_digest(target);
    }
    Ok(targets)
}

fn target_descriptor(
    role: Xmap10TargetRole,
    width: u32,
    height: u32,
    dxgi_format: u32,
) -> Xmap10TargetDescriptor {
    Xmap10TargetDescriptor {
        role,
        width,
        height,
        dxgi_format,
        mip_levels: 1,
        sample_count: 1,
        descriptor_sha256: String::new(),
    }
}

fn luminance_downsample_viewports(
    main_width: u32,
    main_height: u32,
) -> Result<Vec<Xmap10ViewportExtent>> {
    if main_width < 8 || main_height < 8 {
        return Err("XMAP10 luminance sequence requires a nontrivial full-resolution scene".into());
    }
    let mut target_width = (main_width / 2 + 1) / 2;
    let mut target_height = (main_height / 2 + 1) / 2;
    let mut viewports = Vec::new();
    while target_width > 3 || target_height > 3 {
        viewports.push(Xmap10ViewportExtent {
            width: target_width,
            height: target_height,
        });
        if viewports.len() > usize::from(u16::MAX - 8) {
            return Err("XMAP10 luminance downsample sequence exceeds ordinal capacity".into());
        }
        target_width = (target_width + 1) / 2;
        target_height = (target_height + 1) / 2;
    }
    if viewports.is_empty() {
        return Err("XMAP10 luminance downsample sequence is empty".into());
    }
    Ok(viewports)
}

fn xmap10_pass_plan(main_width: u32, main_height: u32) -> Result<Vec<Xmap10PassStep>> {
    let half = Xmap10ViewportExtent {
        width: main_width / 2,
        height: main_height / 2,
    };
    let full = Xmap10ViewportExtent {
        width: main_width,
        height: main_height,
    };
    let downsample_viewports = luminance_downsample_viewports(main_width, main_height)?;
    let mut ordered_passes = vec![Xmap10PassStep {
        ordinal: 0,
        pass: Xmap10PassKind::BrightPass,
        viewport: Some(half),
        reads: vec![Xmap10TargetRole::MainScene],
        read_extents: vec![full],
        writes: vec![Xmap10TargetRole::BrightPass],
    }];
    let mut prior_luminance = Xmap10TargetRole::BrightPass;
    let mut prior_extent = half;
    for (level, viewport) in downsample_viewports.iter().copied().enumerate() {
        let output = if level % 2 == 0 {
            Xmap10TargetRole::LuminancePing
        } else {
            Xmap10TargetRole::LuminancePong
        };
        ordered_passes.push(Xmap10PassStep {
            ordinal: ordered_passes.len() as u16,
            pass: Xmap10PassKind::LuminanceDownsample(u16::try_from(level)?),
            viewport: Some(viewport),
            reads: vec![prior_luminance],
            read_extents: vec![prior_extent],
            writes: vec![output],
        });
        prior_luminance = output;
        prior_extent = viewport;
    }
    ordered_passes.push(Xmap10PassStep {
        ordinal: ordered_passes.len() as u16,
        pass: Xmap10PassKind::LuminanceGather(0),
        viewport: Some(Xmap10ViewportExtent {
            width: 1,
            height: 1,
        }),
        reads: vec![prior_luminance, Xmap10TargetRole::HistoryLuminance],
        read_extents: vec![
            prior_extent,
            Xmap10ViewportExtent {
                width: 1,
                height: 1,
            },
        ],
        writes: vec![Xmap10TargetRole::CurrentLuminance],
    });
    ordered_passes.extend([
        Xmap10PassStep {
            ordinal: ordered_passes.len() as u16,
            pass: Xmap10PassKind::BloomHorizontal,
            viewport: Some(half),
            reads: vec![Xmap10TargetRole::BrightPass],
            read_extents: vec![half],
            writes: vec![Xmap10TargetRole::BloomHorizontal],
        },
        Xmap10PassStep {
            ordinal: (ordered_passes.len() + 1) as u16,
            pass: Xmap10PassKind::BloomVertical,
            viewport: Some(half),
            reads: vec![Xmap10TargetRole::BloomHorizontal],
            read_extents: vec![half],
            writes: vec![Xmap10TargetRole::RestoreBloom],
        },
        Xmap10PassStep {
            ordinal: (ordered_passes.len() + 2) as u16,
            pass: Xmap10PassKind::RestoreScene,
            viewport: Some(full),
            reads: vec![
                Xmap10TargetRole::MainScene,
                Xmap10TargetRole::RestoreBloom,
                Xmap10TargetRole::CurrentLuminance,
            ],
            read_extents: vec![
                full,
                half,
                Xmap10ViewportExtent {
                    width: 1,
                    height: 1,
                },
            ],
            writes: vec![Xmap10TargetRole::Swapchain],
        },
        Xmap10PassStep {
            ordinal: (ordered_passes.len() + 3) as u16,
            pass: Xmap10PassKind::SwapLuminanceHistory,
            viewport: None,
            reads: vec![
                Xmap10TargetRole::CurrentLuminance,
                Xmap10TargetRole::HistoryLuminance,
            ],
            read_extents: vec![
                Xmap10ViewportExtent {
                    width: 1,
                    height: 1,
                },
                Xmap10ViewportExtent {
                    width: 1,
                    height: 1,
                },
            ],
            writes: vec![
                Xmap10TargetRole::HistoryLuminance,
                Xmap10TargetRole::CurrentLuminance,
            ],
        },
    ]);

    Ok(ordered_passes)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10TemporalAdaptationReceiptRef {
    pub schema: String,
    pub frame_sequence: u64,
    pub source: Xmap10TemporalAdaptationSource,
    pub receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap10TemporalAdaptationSource {
    Continue {
        prior_frame_sequence: u64,
        prior_receipt_sha256: String,
    },
    EngineResourceReset {
        reason: crate::TemporalLuminanceResetReason,
        main_scene_resource_identity_sha256: String,
        allocation_generation: u64,
    },
}

impl Xmap10TemporalAdaptationReceiptRef {
    pub fn seal(frame_sequence: u64, source: Xmap10TemporalAdaptationSource) -> Result<Self> {
        let mut receipt = Self {
            schema: XMAP10_TEMPORAL_RECEIPT_SCHEMA.into(),
            frame_sequence,
            source,
            receipt_sha256: String::new(),
        };
        receipt.receipt_sha256 = temporal_adaptation_receipt_digest(&receipt);
        receipt.validate()?;
        Ok(receipt)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_TEMPORAL_RECEIPT_SCHEMA || self.frame_sequence == 0 {
            return Err("XMAP10 temporal adaptation receipt reference is invalid".into());
        }
        match &self.source {
            Xmap10TemporalAdaptationSource::Continue {
                prior_frame_sequence,
                prior_receipt_sha256,
            } => {
                if *prior_frame_sequence >= self.frame_sequence {
                    return Err("XMAP10 temporal continuation sequence is invalid".into());
                }
                validate_sha256(prior_receipt_sha256, "XMAP10 prior temporal receipt")?;
            }
            Xmap10TemporalAdaptationSource::EngineResourceReset {
                main_scene_resource_identity_sha256,
                allocation_generation,
                ..
            } => {
                if *allocation_generation == 0 {
                    return Err("XMAP10 temporal reset has no allocation generation".into());
                }
                validate_sha256(
                    main_scene_resource_identity_sha256,
                    "XMAP10 temporal reset MainScene allocation",
                )?;
            }
        }
        validate_sha256(&self.receipt_sha256, "XMAP10 temporal adaptation receipt")?;
        if self.receipt_sha256 != temporal_adaptation_receipt_digest(self) {
            return Err("XMAP10 temporal adaptation receipt digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10SceneCb0ReceiptRef {
    pub schema: String,
    pub frame_sequence: u64,
    pub hour_index: i64,
    pub scene_cb0_sha256: String,
    pub receipt_sha256: String,
}

impl Xmap10SceneCb0ReceiptRef {
    fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_SCENE_CB0_RECEIPT_SCHEMA
            || self.frame_sequence == 0
            || self.hour_index < 0
        {
            return Err("XMAP10 SceneCB0 receipt reference is invalid".into());
        }
        validate_sha256(&self.scene_cb0_sha256, "XMAP10 SceneCB0 payload")?;
        validate_sha256(&self.receipt_sha256, "XMAP10 SceneCB0 receipt")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap10LutInput {
    Source(u16),
    Intermediate(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xmap10LutBlendPassKind {
    Copy {
        source: Xmap10LutInput,
    },
    Blend1 {
        base: Xmap10LutInput,
        blend: Xmap10LutInput,
        factor_f32_bits: u32,
    },
    Blend2 {
        base_day: Xmap10LutInput,
        base_night: Xmap10LutInput,
        blend_day: Xmap10LutInput,
        blend_night: Xmap10LutInput,
        day_night_factor_f32_bits: u32,
        cross_factor_f32_bits: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10LutBlendPass {
    pub ordinal: u8,
    pub output_target_index: u8,
    pub kind: Xmap10LutBlendPassKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10LutBlendProgram {
    pub schema: String,
    pub ordered_lut_sources: Vec<UsedContentKey>,
    pub ordered_passes: Vec<Xmap10LutBlendPass>,
    pub final_pass_ordinal: u8,
    pub deterministic_sha256: String,
}

impl Xmap10LutBlendProgram {
    pub fn from_exact_grade(
        exact: &ExactPerFrameGradeProgram,
        content: &Xmap10ContentClosure,
    ) -> Result<Self> {
        let mut sources = Vec::new();
        let mut passes = Vec::new();
        let day_night = exact.day_night_factor.to_bits();

        let height_kind = match &exact.height_blend {
            Some(blend) => Xmap10LutBlendPassKind::Blend2 {
                base_day: lut_source_input(
                    &exact.height_base.day.lut_rel_path,
                    content,
                    &mut sources,
                )?,
                base_night: lut_source_input(
                    &exact.height_base.night.lut_rel_path,
                    content,
                    &mut sources,
                )?,
                blend_day: lut_source_input(&blend.day.lut_rel_path, content, &mut sources)?,
                blend_night: lut_source_input(&blend.night.lut_rel_path, content, &mut sources)?,
                day_night_factor_f32_bits: day_night,
                cross_factor_f32_bits: exact.height_factor.to_bits(),
            },
            None => pair_pass_kind(&exact.height_base, day_night, content, &mut sources)?,
        };
        passes.push(Xmap10LutBlendPass {
            ordinal: 0,
            output_target_index: 0,
            kind: height_kind,
        });

        if let Some(ordinary) = &exact.region_ordinary {
            let region_kind = match &exact.region_winter {
                Some(winter) => Xmap10LutBlendPassKind::Blend2 {
                    base_day: lut_source_input(&ordinary.day.lut_rel_path, content, &mut sources)?,
                    base_night: lut_source_input(
                        &ordinary.night.lut_rel_path,
                        content,
                        &mut sources,
                    )?,
                    blend_day: lut_source_input(&winter.day.lut_rel_path, content, &mut sources)?,
                    blend_night: lut_source_input(
                        &winter.night.lut_rel_path,
                        content,
                        &mut sources,
                    )?,
                    day_night_factor_f32_bits: day_night,
                    cross_factor_f32_bits: exact.region_winter_factor.to_bits(),
                },
                None => pair_pass_kind(ordinary, day_night, content, &mut sources)?,
            };
            passes.push(Xmap10LutBlendPass {
                ordinal: 1,
                output_target_index: 1,
                kind: region_kind,
            });
            passes.push(Xmap10LutBlendPass {
                ordinal: 2,
                output_target_index: 2,
                kind: Xmap10LutBlendPassKind::Blend1 {
                    base: Xmap10LutInput::Intermediate(0),
                    blend: Xmap10LutInput::Intermediate(1),
                    factor_f32_bits: exact.region_spatial_factor.to_bits(),
                },
            });
        }

        let final_pass_ordinal = u8::try_from(passes.len() - 1)?;
        let mut program = Self {
            schema: XMAP10_LUT_BLEND_PROGRAM_SCHEMA.into(),
            ordered_lut_sources: sources,
            ordered_passes: passes,
            final_pass_ordinal,
            deterministic_sha256: String::new(),
        };
        program.deterministic_sha256 = lut_blend_program_digest(&program);
        program.validate()?;
        Ok(program)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_LUT_BLEND_PROGRAM_SCHEMA
            || self.ordered_lut_sources.is_empty()
            || self.ordered_passes.is_empty()
            || self.ordered_passes.len() > 3
            || usize::from(self.final_pass_ordinal) + 1 != self.ordered_passes.len()
        {
            return Err("XMAP10 exact LUT blend program identity is incomplete".into());
        }
        let mut unique_sources = BTreeSet::new();
        for source in &self.ordered_lut_sources {
            source
                .validate()
                .map_err(|error| format!("XMAP10 LUT source identity: {error}"))?;
            if !unique_sources.insert(source.normalized_game_relative_path.as_str()) {
                return Err("XMAP10 LUT blend program repeats a source path".into());
            }
        }
        let mut output_targets = BTreeSet::new();
        for (index, pass) in self.ordered_passes.iter().enumerate() {
            if usize::from(pass.ordinal) != index
                || pass.output_target_index >= 3
                || !output_targets.insert(pass.output_target_index)
            {
                return Err("XMAP10 LUT pass ordinal/target allocation is invalid".into());
            }
            validate_lut_pass(pass, self.ordered_lut_sources.len())?;
        }
        if self.deterministic_sha256 != lut_blend_program_digest(self) {
            return Err("XMAP10 LUT blend program digest mismatch".into());
        }
        Ok(())
    }
}

fn pair_pass_kind(
    pair: &ExactGradeEndpointPair,
    day_night_factor_f32_bits: u32,
    content: &Xmap10ContentClosure,
    sources: &mut Vec<UsedContentKey>,
) -> Result<Xmap10LutBlendPassKind> {
    let day = lut_source_input(&pair.day.lut_rel_path, content, sources)?;
    let night = lut_source_input(&pair.night.lut_rel_path, content, sources)?;
    if day == night {
        Ok(Xmap10LutBlendPassKind::Copy { source: day })
    } else {
        Ok(Xmap10LutBlendPassKind::Blend1 {
            base: day,
            blend: night,
            factor_f32_bits: day_night_factor_f32_bits,
        })
    }
}

fn lut_source_input(
    path: &str,
    content: &Xmap10ContentClosure,
    sources: &mut Vec<UsedContentKey>,
) -> Result<Xmap10LutInput> {
    let normalized = normalize_content_path(path)?;
    if let Some(index) = sources
        .iter()
        .position(|source| source.normalized_game_relative_path == normalized)
    {
        return Ok(Xmap10LutInput::Source(u16::try_from(index)?));
    }
    let source = content
        .selected_luts
        .iter()
        .find(|source| source.normalized_game_relative_path == normalized)
        .cloned()
        .ok_or_else(|| {
            format!("XMAP10 exact LUT endpoint {normalized} is absent from content admission")
        })?;
    let index = u16::try_from(sources.len())?;
    sources.push(source);
    Ok(Xmap10LutInput::Source(index))
}

fn validate_lut_pass(pass: &Xmap10LutBlendPass, source_count: usize) -> Result<()> {
    let validate_input = |input: &Xmap10LutInput| -> Result<()> {
        match input {
            Xmap10LutInput::Source(index) if usize::from(*index) < source_count => Ok(()),
            Xmap10LutInput::Intermediate(ordinal) if *ordinal < pass.ordinal => Ok(()),
            _ => Err("XMAP10 LUT pass references an unavailable input".into()),
        }
    };
    let validate_factor = |bits: u32| -> Result<()> {
        let factor = f32::from_bits(bits);
        if factor.is_finite() && (0.0..=1.0).contains(&factor) {
            Ok(())
        } else {
            Err("XMAP10 LUT pass factor is not finite in [0,1]".into())
        }
    };
    match &pass.kind {
        Xmap10LutBlendPassKind::Copy { source } => validate_input(source),
        Xmap10LutBlendPassKind::Blend1 {
            base,
            blend,
            factor_f32_bits,
        } => {
            validate_input(base)?;
            validate_input(blend)?;
            validate_factor(*factor_f32_bits)
        }
        Xmap10LutBlendPassKind::Blend2 {
            base_day,
            base_night,
            blend_day,
            blend_night,
            day_night_factor_f32_bits,
            cross_factor_f32_bits,
        } => {
            for input in [base_day, base_night, blend_day, blend_night] {
                validate_input(input)?;
            }
            validate_factor(*day_night_factor_f32_bits)?;
            validate_factor(*cross_factor_f32_bits)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10PerFrameGradeInputs {
    pub frame_sequence: u64,
    pub ordered_volume_names: Vec<String>,
    pub ordered_volume_weight_f32_bits: Vec<u32>,
    pub resolved_grade_receipt_sha256: String,
    pub lut_program: Xmap10LutBlendProgram,
    pub hsv_f32_bits: [u32; 4],
    pub color_balance_f32_bits: [u32; 3],
    pub middle_grey_f32_bits: u32,
    pub luminance_white_squared_f32_bits: u32,
    pub bloom_width_f32_bits: u32,
    pub bloom_scale_f32_bits: u32,
    pub bright_threshold_f32_bits: u32,
    pub emissive_bloom_strength_f32_bits: u32,
    pub min_hdr_adjustment_f32_bits: u32,
    pub max_hdr_adjustment_f32_bits: u32,
    pub hdr_adjustment_speed_f32_bits: u32,
    pub deterministic_sha256: String,
}

impl Xmap10PerFrameGradeInputs {
    pub fn from_exact_grade(
        frame_sequence: u64,
        exact: &ExactPerFrameGradeProgram,
        content: &Xmap10ContentClosure,
        defines: &GraphicsGradingDefines,
    ) -> Result<Self> {
        if frame_sequence == 0 {
            return Err("XMAP10 exact grade frame sequence is zero".into());
        }
        let lut_program = Xmap10LutBlendProgram::from_exact_grade(exact, content)?;
        let mut ordered_volume_names = vec![exact.height_base_volume_name.clone()];
        let mut ordered_volume_weight_f32_bits = vec![1.0_f32.to_bits()];
        match (&exact.height_blend_volume_name, &exact.height_blend) {
            (Some(name), Some(_)) => {
                ordered_volume_names.push(name.clone());
                ordered_volume_weight_f32_bits.push(exact.height_factor.to_bits());
            }
            (None, None) => {}
            _ => {
                return Err("XMAP10 exact height volume identity and endpoint pair disagree".into())
            }
        }
        match (&exact.region_volume_name, &exact.region_ordinary) {
            (Some(name), Some(_)) => {
                ordered_volume_names.push(name.clone());
                ordered_volume_weight_f32_bits.push(exact.region_spatial_factor.to_bits());
            }
            (None, None) => {}
            _ => {
                return Err("XMAP10 exact region volume identity and endpoint pair disagree".into())
            }
        }

        let white = defines.tone_map_luminance_white;
        if !white.is_finite()
            || white <= 0.0
            || !defines.hdr_adjustment_speed.is_finite()
            || defines.hdr_adjustment_speed < 0.0
            || !defines.emissive_bloom_strength.is_finite()
            || defines.emissive_bloom_strength < 0.0
        {
            return Err("XMAP10 grading defines contain invalid post-FX scalars".into());
        }

        let mut grade = Self {
            frame_sequence,
            ordered_volume_names,
            ordered_volume_weight_f32_bits,
            resolved_grade_receipt_sha256: exact_grade_program_digest(exact, content),
            lut_program,
            hsv_f32_bits: [
                0.0_f32.to_bits(),
                1.0_f32.to_bits(),
                1.0_f32.to_bits(),
                0.0_f32.to_bits(),
            ],
            color_balance_f32_bits: [1.0_f32.to_bits(); 3],
            middle_grey_f32_bits: exact.posteffect_scalars.tone_map_middle_grey.to_bits(),
            luminance_white_squared_f32_bits: (white * white).to_bits(),
            bloom_width_f32_bits: exact.posteffect_scalars.bloom_width.to_bits(),
            bloom_scale_f32_bits: exact.posteffect_scalars.bloom_scale.to_bits(),
            bright_threshold_f32_bits: exact.posteffect_scalars.bright_threshold.to_bits(),
            emissive_bloom_strength_f32_bits: defines.emissive_bloom_strength.to_bits(),
            min_hdr_adjustment_f32_bits: exact.posteffect_scalars.min_hdr_adjustment.to_bits(),
            max_hdr_adjustment_f32_bits: exact.posteffect_scalars.max_hdr_adjustment.to_bits(),
            hdr_adjustment_speed_f32_bits: defines.hdr_adjustment_speed.to_bits(),
            deterministic_sha256: String::new(),
        };
        grade.deterministic_sha256 = grade_inputs_digest(&grade);
        grade.validate()?;
        Ok(grade)
    }

    pub fn validate(&self) -> Result<()> {
        if self.frame_sequence == 0
            || self.ordered_volume_names.is_empty()
            || self.ordered_volume_names.len() != self.ordered_volume_weight_f32_bits.len()
            || self
                .ordered_volume_names
                .iter()
                .any(|name| name.trim().is_empty())
        {
            return Err("XMAP10 per-frame grade selection is incomplete".into());
        }
        self.lut_program.validate()?;
        validate_sha256(
            &self.resolved_grade_receipt_sha256,
            "XMAP10 resolved grade receipt",
        )?;
        let floats = self
            .ordered_volume_weight_f32_bits
            .iter()
            .copied()
            .chain(self.hsv_f32_bits)
            .chain(self.color_balance_f32_bits)
            .chain([
                self.middle_grey_f32_bits,
                self.luminance_white_squared_f32_bits,
                self.bloom_width_f32_bits,
                self.bloom_scale_f32_bits,
                self.bright_threshold_f32_bits,
                self.emissive_bloom_strength_f32_bits,
                self.min_hdr_adjustment_f32_bits,
                self.max_hdr_adjustment_f32_bits,
                self.hdr_adjustment_speed_f32_bits,
            ]);
        if floats
            .into_iter()
            .any(|bits| !f32::from_bits(bits).is_finite())
        {
            return Err("XMAP10 per-frame grade contains a non-finite scalar".into());
        }
        for bits in &self.ordered_volume_weight_f32_bits {
            if !(0.0..=1.0).contains(&f32::from_bits(*bits)) {
                return Err("XMAP10 volume blend weight is outside [0,1]".into());
            }
        }
        if f32::from_bits(self.middle_grey_f32_bits) <= 0.0
            || f32::from_bits(self.luminance_white_squared_f32_bits) <= 0.0
            || f32::from_bits(self.bloom_width_f32_bits) <= 0.0
            || f32::from_bits(self.bloom_scale_f32_bits) < 0.0
            || f32::from_bits(self.emissive_bloom_strength_f32_bits) < 0.0
            || f32::from_bits(self.min_hdr_adjustment_f32_bits) <= 0.0
            || f32::from_bits(self.max_hdr_adjustment_f32_bits) <= 0.0
            || f32::from_bits(self.hdr_adjustment_speed_f32_bits) < 0.0
        {
            return Err("XMAP10 per-frame grade has invalid physical scalar bounds".into());
        }
        if self.deterministic_sha256 != grade_inputs_digest(self) {
            return Err("XMAP10 per-frame grade digest mismatch".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedXmap10Frame {
    pub schema: String,
    pub hour_index: i64,
    pub content: Xmap10ContentClosure,
    pub active_effects: Vec<Xmap10ActiveEffectVariant>,
    pub targets: Xmap10TargetPlan,
    pub grade: Xmap10PerFrameGradeInputs,
    pub temporal_adaptation: Xmap10TemporalAdaptationReceiptRef,
    pub scene_cb0: Xmap10SceneCb0ReceiptRef,
    pub deterministic_sha256: String,
}

impl PreparedXmap10Frame {
    pub fn prepare(
        hour_index: i64,
        content: Xmap10ContentClosure,
        mut active_effects: Vec<Xmap10ActiveEffectVariant>,
        targets: Xmap10TargetPlan,
        mut grade: Xmap10PerFrameGradeInputs,
        temporal_adaptation: Xmap10TemporalAdaptationReceiptRef,
        scene_cb0: Xmap10SceneCb0ReceiptRef,
    ) -> Result<Self> {
        for effect in &mut active_effects {
            if effect.deterministic_sha256.is_empty() {
                effect.deterministic_sha256 = active_effect_digest(effect);
            }
        }
        if grade.deterministic_sha256.is_empty() {
            grade.deterministic_sha256 = grade_inputs_digest(&grade);
        }
        let mut frame = Self {
            schema: XMAP10_PREPARATION_SCHEMA.into(),
            hour_index,
            content,
            active_effects,
            targets,
            grade,
            temporal_adaptation,
            scene_cb0,
            deterministic_sha256: String::new(),
        };
        frame.deterministic_sha256 = prepared_frame_digest(&frame);
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != XMAP10_PREPARATION_SCHEMA {
            return Err("XMAP10 preparation schema mismatch".into());
        }
        if self.hour_index < 0 || self.scene_cb0.hour_index != self.hour_index {
            return Err("XMAP10 prepared hour does not match SceneCB0".into());
        }
        self.content.validate()?;
        self.targets.validate()?;
        self.grade.validate()?;
        self.temporal_adaptation.validate()?;
        self.scene_cb0.validate()?;
        if self.grade.frame_sequence != self.temporal_adaptation.frame_sequence
            || self.grade.frame_sequence != self.scene_cb0.frame_sequence
        {
            return Err("XMAP10 per-frame receipt sequences disagree".into());
        }
        let grade_luts = self
            .grade
            .lut_program
            .ordered_lut_sources
            .iter()
            .map(|key| key.normalized_game_relative_path.as_str())
            .collect::<BTreeSet<_>>();
        let content_luts = self
            .content
            .selected_luts
            .iter()
            .map(|key| key.normalized_game_relative_path.as_str())
            .collect::<BTreeSet<_>>();
        if grade_luts != content_luts {
            return Err(
                "XMAP10 per-frame LUT bindings differ from the admitted source closure".into(),
            );
        }
        let expected_roles = [
            Xmap10ProgramRole::BrightPass,
            Xmap10ProgramRole::LuminanceDownsample,
            Xmap10ProgramRole::LuminanceGather,
            Xmap10ProgramRole::BloomHorizontal,
            Xmap10ProgramRole::BloomVertical,
            Xmap10ProgramRole::RestoreScene,
        ];
        if self.active_effects.len() != expected_roles.len() {
            return Err("XMAP10 active effect set is incomplete".into());
        }
        for (effect, expected_role) in self.active_effects.iter().zip(expected_roles) {
            let unique_defines = effect
                .ordered_defines
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            if effect.role != expected_role
                || effect.effect_name.trim().is_empty()
                || effect
                    .ordered_defines
                    .iter()
                    .any(|define| define.trim().is_empty())
                || unique_defines.len() != effect.ordered_defines.len()
                || effect.deterministic_sha256 != active_effect_digest(effect)
            {
                return Err("XMAP10 active effect variant is invalid or out of order".into());
            }
            let expected_root = shader_root_for_role(effect.role);
            let unit = self.content.shader_unit(expected_root)?;
            if !effect.shader_root.eq_ignore_ascii_case(expected_root)
                || effect.translation_unit_sha256 != unit.deterministic_sha256
            {
                return Err(
                    "XMAP10 active effect is not bound to its admitted translation unit".into(),
                );
            }
        }
        let restore = self
            .active_effects
            .iter()
            .find(|effect| effect.role == Xmap10ProgramRole::RestoreScene)
            .ok_or("XMAP10 RestoreScene effect is absent")?;
        let restore_defines = restore
            .ordered_defines
            .iter()
            .map(|value| value.as_str())
            .collect::<BTreeSet<_>>();
        for required in ["BLOOM", "HDR", "ADJUSTED_EXPOSURE", "COLOR_LUT"] {
            if !restore_defines.contains(required) {
                return Err(format!(
                    "XMAP10 RestoreScene variant omits required define {required}"
                )
                .into());
            }
        }
        if self.deterministic_sha256 != prepared_frame_digest(self) {
            return Err("XMAP10 prepared-frame digest mismatch".into());
        }
        Ok(())
    }

    pub fn admit_native_execution(
        &self,
        programs: Xmap10NativeProgramReceiptRef,
        compositor: Xmap10NativeCompositorReceiptRef,
    ) -> Result<ReadyXmap10Frame> {
        self.validate()?;
        programs.validate_against(self)?;
        compositor.validate_against(self)?;
        let mut ready = ReadyXmap10Frame {
            schema: XMAP10_READY_SCHEMA.into(),
            prepared_frame_sha256: self.deterministic_sha256.clone(),
            native_program_receipt_sha256: programs.receipt_sha256,
            native_compositor_receipt_sha256: compositor.receipt_sha256,
            deterministic_sha256: String::new(),
        };
        ready.deterministic_sha256 = ready_frame_digest(&ready);
        Ok(ready)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10CompiledProgramIdentity {
    pub role: Xmap10ProgramRole,
    pub active_effect_sha256: String,
    pub vertex_bytecode_sha256: String,
    pub pixel_bytecode_sha256: String,
    pub reflection_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10NativeProgramReceiptRef {
    pub schema: String,
    pub prepared_frame_sha256: String,
    pub ordered_programs: Vec<Xmap10CompiledProgramIdentity>,
    pub receipt_sha256: String,
}

impl Xmap10NativeProgramReceiptRef {
    fn validate_against(&self, frame: &PreparedXmap10Frame) -> Result<()> {
        if self.schema != XMAP10_NATIVE_PROGRAM_RECEIPT_SCHEMA
            || self.prepared_frame_sha256 != frame.deterministic_sha256
            || self.ordered_programs.len() != frame.active_effects.len()
        {
            return Err("XMAP10 native program receipt does not name this preparation".into());
        }
        for (program, effect) in self.ordered_programs.iter().zip(&frame.active_effects) {
            if program.role != effect.role
                || program.active_effect_sha256 != effect.deterministic_sha256
            {
                return Err("XMAP10 compiled program order/effect identity mismatch".into());
            }
            for (value, label) in [
                (&program.vertex_bytecode_sha256, "XMAP10 vertex bytecode"),
                (&program.pixel_bytecode_sha256, "XMAP10 pixel bytecode"),
                (&program.reflection_sha256, "XMAP10 shader reflection"),
            ] {
                validate_sha256(value, label)?;
            }
        }
        validate_sha256(&self.receipt_sha256, "XMAP10 native program receipt")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xmap10NativeCompositorReceiptRef {
    pub schema: String,
    pub prepared_frame_sha256: String,
    pub target_plan_sha256: String,
    pub ordered_pass_count: u16,
    pub receipt_sha256: String,
}

impl Xmap10NativeCompositorReceiptRef {
    fn validate_against(&self, frame: &PreparedXmap10Frame) -> Result<()> {
        let actual_draw_count = frame
            .targets
            .ordered_passes
            .iter()
            .filter(|pass| pass.viewport.is_some())
            .count()
            + frame.grade.lut_program.ordered_passes.len();
        if self.schema != XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA
            || self.prepared_frame_sha256 != frame.deterministic_sha256
            || self.target_plan_sha256 != frame.targets.deterministic_sha256
            || usize::from(self.ordered_pass_count) != actual_draw_count
        {
            return Err("XMAP10 native compositor receipt does not name this pass graph".into());
        }
        validate_sha256(&self.receipt_sha256, "XMAP10 native compositor receipt")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadyXmap10Frame {
    pub schema: String,
    pub prepared_frame_sha256: String,
    pub native_program_receipt_sha256: String,
    pub native_compositor_receipt_sha256: String,
    pub deterministic_sha256: String,
}

fn normalize_content_path(path: &str) -> Result<String> {
    let normalized = path.trim().replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("//")
    {
        return Err(format!("XMAP10 content path is not normalized and relative: {path:?}").into());
    }
    Ok(normalized)
}

fn shader_root_for_role(role: Xmap10ProgramRole) -> &'static str {
    match role {
        Xmap10ProgramRole::BrightPass => "gfx/FX/downsample.shader",
        Xmap10ProgramRole::LuminanceDownsample | Xmap10ProgramRole::LuminanceGather => {
            "gfx/FX/downsample_luminance.shader"
        }
        Xmap10ProgramRole::BloomHorizontal | Xmap10ProgramRole::BloomVertical => {
            "gfx/FX/bloom.shader"
        }
        Xmap10ProgramRole::RestoreScene => "gfx/FX/restorescene.shader",
    }
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("{label} is not a canonical nonzero SHA-256 identity").into());
    }
    Ok(())
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn push_content(bytes: &mut Vec<u8>, key: &UsedContentKey) {
    push_string(bytes, &key.effective_vfs_manifest_sha256);
    push_string(bytes, &key.normalized_game_relative_path);
    bytes.extend_from_slice(&key.winning_layer_ordinal.to_le_bytes());
    bytes.push(match key.layer_kind {
        portable_content_manifest::ContentLayerKind::Base => 0,
        portable_content_manifest::ContentLayerKind::IntegratedDlc => 1,
        portable_content_manifest::ContentLayerKind::Dlc => 2,
        portable_content_manifest::ContentLayerKind::Mod => 3,
    });
    push_string(bytes, &key.layer_id);
    push_string(bytes, &key.source_sha256);
    push_string(bytes, &key.payload_sha256);
    bytes.extend_from_slice(&key.payload_bytes.to_le_bytes());
}

fn shader_unit_digest(
    unit: &Xmap10ShaderTranslationUnit,
    by_path: &BTreeMap<&str, &UsedContentKey>,
) -> Result<String> {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &unit.root_path);
    for path in &unit.ordered_source_paths {
        push_string(&mut bytes, path);
        let content = by_path
            .get(path.as_str())
            .ok_or_else(|| format!("XMAP10 shader unit references unadmitted source {path}"))?;
        push_string(&mut bytes, &content.payload_sha256);
        bytes.extend_from_slice(&content.payload_bytes.to_le_bytes());
    }
    Ok(Sha256::hex(&bytes))
}

fn content_closure_digest(closure: &Xmap10ContentClosure) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &closure.content_manifest_sha256);
    push_string(&mut bytes, &closure.effective_vfs_manifest_sha256);
    push_string(&mut bytes, &closure.product_modset_sha256);
    for key in &closure.ordered_used_content {
        push_content(&mut bytes, key);
    }
    for unit in &closure.shader_translation_units {
        push_string(&mut bytes, &unit.deterministic_sha256);
    }
    for lut in &closure.selected_luts {
        push_string(&mut bytes, &lut.payload_sha256);
    }
    Sha256::hex(&bytes)
}

fn active_effect_digest(effect: &Xmap10ActiveEffectVariant) -> String {
    let mut bytes = Vec::new();
    bytes.push(effect.role as u8);
    push_string(&mut bytes, &effect.shader_root);
    push_string(&mut bytes, &effect.effect_name);
    for define in &effect.ordered_defines {
        push_string(&mut bytes, define);
    }
    push_string(&mut bytes, &effect.translation_unit_sha256);
    Sha256::hex(&bytes)
}

fn target_descriptor_digest(target: &Xmap10TargetDescriptor) -> String {
    let mut bytes = Vec::new();
    push_target_role(&mut bytes, target.role);
    bytes.extend_from_slice(&target.width.to_le_bytes());
    bytes.extend_from_slice(&target.height.to_le_bytes());
    bytes.extend_from_slice(&target.dxgi_format.to_le_bytes());
    bytes.extend_from_slice(&target.mip_levels.to_le_bytes());
    bytes.extend_from_slice(&target.sample_count.to_le_bytes());
    Sha256::hex(&bytes)
}

fn main_scene_depth_descriptor_digest(target: &Xmap10MainSceneDepthDescriptor) -> String {
    let mut bytes = b"XMAP10\0MainSceneDepth\0exact-a729\0v1\0".to_vec();
    bytes.extend_from_slice(&target.width.to_le_bytes());
    bytes.extend_from_slice(&target.height.to_le_bytes());
    bytes.extend_from_slice(&target.dxgi_format.to_le_bytes());
    bytes.extend_from_slice(&target.sample_count.to_le_bytes());
    bytes.extend_from_slice(&target.sample_quality.to_le_bytes());
    Sha256::hex(&bytes)
}

fn push_target_role(bytes: &mut Vec<u8>, role: Xmap10TargetRole) {
    match role {
        Xmap10TargetRole::MainScene => bytes.push(0),
        Xmap10TargetRole::BrightPass => bytes.push(1),
        Xmap10TargetRole::LuminancePing => bytes.push(2),
        Xmap10TargetRole::LuminancePong => bytes.push(3),
        Xmap10TargetRole::CurrentLuminance => bytes.push(4),
        Xmap10TargetRole::HistoryLuminance => bytes.push(5),
        Xmap10TargetRole::BloomHorizontal => bytes.push(6),
        Xmap10TargetRole::RestoreBloom => bytes.push(7),
        Xmap10TargetRole::Swapchain => bytes.push(8),
        Xmap10TargetRole::ColorCube0 => bytes.push(9),
        Xmap10TargetRole::ColorCube1 => bytes.push(10),
        Xmap10TargetRole::ColorCube2 => bytes.push(11),
    }
}

fn target_plan_digest(plan: &Xmap10TargetPlan) -> String {
    let mut bytes = Vec::new();
    for target in &plan.ordered_targets {
        push_string(&mut bytes, &target.descriptor_sha256);
    }
    push_string(&mut bytes, &plan.main_scene_depth.descriptor_sha256);
    for step in &plan.ordered_passes {
        bytes.extend_from_slice(&step.ordinal.to_le_bytes());
        match step.pass {
            Xmap10PassKind::BrightPass => bytes.push(0),
            Xmap10PassKind::LuminanceDownsample(level) => {
                bytes.push(1);
                bytes.extend_from_slice(&level.to_le_bytes());
            }
            Xmap10PassKind::LuminanceGather(level) => {
                bytes.push(2);
                bytes.extend_from_slice(&level.to_le_bytes());
            }
            Xmap10PassKind::BloomHorizontal => bytes.push(3),
            Xmap10PassKind::BloomVertical => bytes.push(4),
            Xmap10PassKind::RestoreScene => bytes.push(5),
            Xmap10PassKind::SwapLuminanceHistory => bytes.push(6),
        }
        match step.viewport {
            Some(viewport) => {
                bytes.push(1);
                bytes.extend_from_slice(&viewport.width.to_le_bytes());
                bytes.extend_from_slice(&viewport.height.to_le_bytes());
            }
            None => bytes.push(0),
        }
        bytes.extend_from_slice(&(step.reads.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&(step.read_extents.len() as u64).to_le_bytes());
        for (role, extent) in step.reads.iter().zip(&step.read_extents) {
            push_target_role(&mut bytes, *role);
            bytes.extend_from_slice(&extent.width.to_le_bytes());
            bytes.extend_from_slice(&extent.height.to_le_bytes());
        }
        bytes.extend_from_slice(&(step.writes.len() as u64).to_le_bytes());
        for role in &step.writes {
            push_target_role(&mut bytes, *role);
        }
    }
    Sha256::hex(&bytes)
}

fn lut_blend_program_digest(program: &Xmap10LutBlendProgram) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &program.schema);
    for source in &program.ordered_lut_sources {
        push_content(&mut bytes, source);
    }
    let push_input = |bytes: &mut Vec<u8>, input: &Xmap10LutInput| match input {
        Xmap10LutInput::Source(index) => {
            bytes.push(0);
            bytes.extend_from_slice(&index.to_le_bytes());
        }
        Xmap10LutInput::Intermediate(ordinal) => {
            bytes.push(1);
            bytes.push(*ordinal);
        }
    };
    for pass in &program.ordered_passes {
        bytes.push(pass.ordinal);
        bytes.push(pass.output_target_index);
        match &pass.kind {
            Xmap10LutBlendPassKind::Copy { source } => {
                bytes.push(0);
                push_input(&mut bytes, source);
            }
            Xmap10LutBlendPassKind::Blend1 {
                base,
                blend,
                factor_f32_bits,
            } => {
                bytes.push(1);
                push_input(&mut bytes, base);
                push_input(&mut bytes, blend);
                bytes.extend_from_slice(&factor_f32_bits.to_le_bytes());
            }
            Xmap10LutBlendPassKind::Blend2 {
                base_day,
                base_night,
                blend_day,
                blend_night,
                day_night_factor_f32_bits,
                cross_factor_f32_bits,
            } => {
                bytes.push(2);
                for input in [base_day, base_night, blend_day, blend_night] {
                    push_input(&mut bytes, input);
                }
                bytes.extend_from_slice(&day_night_factor_f32_bits.to_le_bytes());
                bytes.extend_from_slice(&cross_factor_f32_bits.to_le_bytes());
            }
        }
    }
    bytes.push(program.final_pass_ordinal);
    Sha256::hex(&bytes)
}

fn exact_grade_program_digest(
    exact: &ExactPerFrameGradeProgram,
    content: &Xmap10ContentClosure,
) -> String {
    fn push_pair(bytes: &mut Vec<u8>, pair: &ExactGradeEndpointPair) {
        for endpoint in [&pair.day, &pair.night] {
            push_string(bytes, &endpoint.value_name);
            push_string(bytes, &endpoint.lut_rel_path);
            bytes.extend_from_slice(&endpoint.middle_grey.to_bits().to_le_bytes());
        }
    }

    let mut bytes = Vec::new();
    push_string(&mut bytes, "hoi4.xmap10.exact-grade-program.a729.v1");
    push_string(&mut bytes, &content.deterministic_sha256);
    push_string(&mut bytes, &exact.height_base_volume_name);
    push_pair(&mut bytes, &exact.height_base);
    match (&exact.height_blend_volume_name, &exact.height_blend) {
        (Some(name), Some(pair)) => {
            bytes.push(1);
            push_string(&mut bytes, name);
            push_pair(&mut bytes, pair);
        }
        _ => bytes.push(0),
    }
    bytes.extend_from_slice(&exact.height_factor.to_bits().to_le_bytes());
    match (&exact.region_volume_name, &exact.region_ordinary) {
        (Some(name), Some(pair)) => {
            bytes.push(1);
            push_string(&mut bytes, name);
            push_pair(&mut bytes, pair);
        }
        _ => bytes.push(0),
    }
    if let Some(pair) = &exact.region_winter {
        bytes.push(1);
        push_pair(&mut bytes, pair);
    } else {
        bytes.push(0);
    }
    for factor in [
        exact.region_winter_factor,
        exact.region_spatial_factor,
        exact.day_night_factor,
    ] {
        bytes.extend_from_slice(&factor.to_bits().to_le_bytes());
    }
    for scalar in [
        exact.posteffect_scalars.min_hdr_adjustment,
        exact.posteffect_scalars.max_hdr_adjustment,
        exact.posteffect_scalars.tone_map_middle_grey,
        exact.posteffect_scalars.bloom_width,
        exact.posteffect_scalars.bloom_scale,
        exact.posteffect_scalars.bright_threshold,
    ] {
        bytes.extend_from_slice(&scalar.to_bits().to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn grade_inputs_digest(grade: &Xmap10PerFrameGradeInputs) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&grade.frame_sequence.to_le_bytes());
    for (name, weight) in grade
        .ordered_volume_names
        .iter()
        .zip(&grade.ordered_volume_weight_f32_bits)
    {
        push_string(&mut bytes, name);
        bytes.extend_from_slice(&weight.to_le_bytes());
    }
    push_string(&mut bytes, &grade.resolved_grade_receipt_sha256);
    push_string(&mut bytes, &grade.lut_program.deterministic_sha256);
    for bits in grade
        .hsv_f32_bits
        .into_iter()
        .chain(grade.color_balance_f32_bits)
        .chain([
            grade.middle_grey_f32_bits,
            grade.luminance_white_squared_f32_bits,
            grade.bloom_width_f32_bits,
            grade.bloom_scale_f32_bits,
            grade.bright_threshold_f32_bits,
            grade.emissive_bloom_strength_f32_bits,
            grade.min_hdr_adjustment_f32_bits,
            grade.max_hdr_adjustment_f32_bits,
            grade.hdr_adjustment_speed_f32_bits,
        ])
    {
        bytes.extend_from_slice(&bits.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn temporal_adaptation_receipt_digest(receipt: &Xmap10TemporalAdaptationReceiptRef) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &receipt.schema);
    bytes.extend_from_slice(&receipt.frame_sequence.to_le_bytes());
    match &receipt.source {
        Xmap10TemporalAdaptationSource::Continue {
            prior_frame_sequence,
            prior_receipt_sha256,
        } => {
            bytes.push(0);
            bytes.extend_from_slice(&prior_frame_sequence.to_le_bytes());
            push_string(&mut bytes, prior_receipt_sha256);
        }
        Xmap10TemporalAdaptationSource::EngineResourceReset {
            reason,
            main_scene_resource_identity_sha256,
            allocation_generation,
        } => {
            bytes.push(1);
            bytes.push(match reason {
                crate::TemporalLuminanceResetReason::ResourcesAllocated => 0,
                crate::TemporalLuminanceResetReason::ResourcesRecreated => 1,
                crate::TemporalLuminanceResetReason::HdrModeChanged => 2,
            });
            push_string(&mut bytes, main_scene_resource_identity_sha256);
            bytes.extend_from_slice(&allocation_generation.to_le_bytes());
        }
    }
    Sha256::hex(&bytes)
}

fn prepared_frame_digest(frame: &PreparedXmap10Frame) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &frame.schema);
    bytes.extend_from_slice(&frame.hour_index.to_le_bytes());
    push_string(&mut bytes, &frame.content.deterministic_sha256);
    for effect in &frame.active_effects {
        push_string(&mut bytes, &effect.deterministic_sha256);
    }
    push_string(&mut bytes, &frame.targets.deterministic_sha256);
    push_string(&mut bytes, &frame.grade.deterministic_sha256);
    push_string(&mut bytes, &frame.temporal_adaptation.receipt_sha256);
    push_string(&mut bytes, &frame.scene_cb0.receipt_sha256);
    Sha256::hex(&bytes)
}

fn ready_frame_digest(frame: &ReadyXmap10Frame) -> String {
    let mut bytes = Vec::new();
    push_string(&mut bytes, &frame.schema);
    push_string(&mut bytes, &frame.prepared_frame_sha256);
    push_string(&mut bytes, &frame.native_program_receipt_sha256);
    push_string(&mut bytes, &frame.native_compositor_receipt_sha256);
    Sha256::hex(&bytes)
}

/// The exact draw-time obligation XMAP10 leaves behind at foundation time.
/// The draw path discharges it by completing the post-FX chain for the frame;
/// nothing else may discharge it.
pub const XMAP10_POSTFX_DRAW_CONTRACT: &str = "XMAP10.postfx_frame_receipt";

/// GradingPostFx cannot answer at foundation time. Its grade resolves against
/// the live per-frame D3D scene identities and the PRIOR frame's temporal
/// adaptation receipt (see the replay app's post-FX preparation), and neither
/// exists at once-per-hour foundation time. So the family tells the readiness
/// graph where its answer will come from and defers; the frame still refuses
/// to show until the draw path discharges that contract.
pub(crate) struct Xmap10GradingPostFxAdapter;

impl Xmap10GradingPostFxAdapter {
    pub(crate) fn producer_registration_sha256() -> String {
        Sha256::hex(
            format!(
                "XMAP10\0{XMAP10_PREPARATION_SCHEMA}\0{XMAP10_READY_SCHEMA}\0{XMAP10_TEMPORAL_RECEIPT_SCHEMA}\0{XMAP10_SCENE_CB0_RECEIPT_SCHEMA}\0{XMAP10_POSTFX_DRAW_CONTRACT}"
            )
            .as_bytes(),
        )
    }
}

impl crate::ProductionMapFeatureAdapter for Xmap10GradingPostFxAdapter {
    fn family(&self) -> portable_render_stream::MapVisualFamily {
        portable_render_stream::MapVisualFamily::GradingPostFx
    }

    fn prepare(
        &self,
        _context: &crate::FoundationFeatureContext<'_>,
    ) -> Result<crate::FoundationFeatureOutput> {
        // TIMING, and only timing. Can this family answer at foundation time?
        // No -- never. Its grade needs live per-frame D3D identities and the
        // prior frame's temporal receipt. That is what DeferredToDraw records.
        //
        // AUTHORITY was the old second question, and it has been answered:
        // exposure is not an input the RECORDING owes. The engine's exposure is
        // a pure function of the frames a client draws for itself -- the
        // adaptation state lives on the post-FX render targets, never in world
        // state, and is written by the client's own luminance chain (a729
        // exactly the same inputs a real client holds, because they ARE its own
        // frames, so it computes the identical law rather than reconstructing a
        // missing recording row. See `crate::luminance_law` for the proof and
        // its citations. `postfx.temporal_exposure_state` was accordingly
        // retired as a map-authority requirement -- there was never a producer
        // to owe.
        Ok(crate::FoundationFeatureOutput::without_payload(
            Self::deferred_disposition(),
            Vec::new(),
        ))
    }
}

impl Xmap10GradingPostFxAdapter {
    pub(crate) fn deferred_disposition() -> portable_render_stream::FamilyDisposition {
        portable_render_stream::FamilyDisposition::DeferredToDraw {
            producer_registration_sha256: Self::producer_registration_sha256(),
            draw_contract_id: XMAP10_POSTFX_DRAW_CONTRACT.into(),
            reason:
                "the post-FX grade resolves against live per-frame D3D scene identities and the prior frame's temporal-adaptation receipt, neither of which exists at once-per-hour foundation time"
                    .into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::ContentLayerKind;

    fn sha(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn content(path: &str, payload: u8) -> UsedContentKey {
        UsedContentKey {
            effective_vfs_manifest_sha256: sha(1),
            normalized_game_relative_path: path.to_ascii_lowercase(),
            winning_layer_ordinal: 0,
            layer_kind: ContentLayerKind::Base,
            layer_id: "base".into(),
            source_sha256: sha(payload),
            payload_sha256: sha(payload),
            payload_bytes: 17,
        }
    }

    fn target_plan() -> Xmap10TargetPlan {
        Xmap10TargetPlan::new(1920, 1080).unwrap()
    }

    fn single_lut_program(lut: UsedContentKey) -> Xmap10LutBlendProgram {
        let mut program = Xmap10LutBlendProgram {
            schema: XMAP10_LUT_BLEND_PROGRAM_SCHEMA.into(),
            ordered_lut_sources: vec![lut],
            ordered_passes: vec![Xmap10LutBlendPass {
                ordinal: 0,
                output_target_index: 0,
                kind: Xmap10LutBlendPassKind::Copy {
                    source: Xmap10LutInput::Source(0),
                },
            }],
            final_pass_ordinal: 0,
            deterministic_sha256: String::new(),
        };
        program.deterministic_sha256 = lut_blend_program_digest(&program);
        program
    }

    #[test]
    fn target_plan_pins_ping_pong_viewports_history_and_swap() {
        let plan = target_plan();
        assert!(plan.validate().is_ok());
        let downsample = plan
            .ordered_passes
            .iter()
            .filter_map(|step| match step.pass {
                Xmap10PassKind::LuminanceDownsample(_) => step.viewport,
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            downsample,
            [
                Xmap10ViewportExtent {
                    width: 480,
                    height: 270
                },
                Xmap10ViewportExtent {
                    width: 240,
                    height: 135
                },
                Xmap10ViewportExtent {
                    width: 120,
                    height: 68
                },
                Xmap10ViewportExtent {
                    width: 60,
                    height: 34
                },
                Xmap10ViewportExtent {
                    width: 30,
                    height: 17
                },
                Xmap10ViewportExtent {
                    width: 15,
                    height: 9
                },
                Xmap10ViewportExtent {
                    width: 8,
                    height: 5
                },
                Xmap10ViewportExtent {
                    width: 4,
                    height: 3
                },
            ]
        );
        let downsample_targets = plan
            .ordered_passes
            .iter()
            .filter_map(|step| match step.pass {
                Xmap10PassKind::LuminanceDownsample(_) => step.writes.first().copied(),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            downsample_targets,
            [
                Xmap10TargetRole::LuminancePing,
                Xmap10TargetRole::LuminancePong,
                Xmap10TargetRole::LuminancePing,
                Xmap10TargetRole::LuminancePong,
                Xmap10TargetRole::LuminancePing,
                Xmap10TargetRole::LuminancePong,
                Xmap10TargetRole::LuminancePing,
                Xmap10TargetRole::LuminancePong,
            ]
        );
        let gather = plan
            .ordered_passes
            .iter()
            .find(|step| matches!(step.pass, Xmap10PassKind::LuminanceGather(0)))
            .unwrap();
        assert_eq!(
            gather.reads,
            [
                Xmap10TargetRole::LuminancePong,
                Xmap10TargetRole::HistoryLuminance,
            ]
        );
        assert_eq!(gather.writes, [Xmap10TargetRole::CurrentLuminance]);
        assert_eq!(
            gather.read_extents,
            [
                Xmap10ViewportExtent {
                    width: 4,
                    height: 3
                },
                Xmap10ViewportExtent {
                    width: 1,
                    height: 1
                },
            ]
        );
        let ping = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::LuminancePing)
            .unwrap();
        let pong = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::LuminancePong)
            .unwrap();
        assert_eq!((ping.width, ping.height, ping.dxgi_format), (480, 270, 54));
        assert_eq!((pong.width, pong.height, pong.dxgi_format), (480, 270, 54));
        let current = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::CurrentLuminance)
            .unwrap();
        let history = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::HistoryLuminance)
            .unwrap();
        assert_eq!((current.width, current.height), (1, 1));
        assert_eq!((history.width, history.height), (1, 1));
        assert_ne!(current.role, history.role);
        assert!(matches!(
            plan.ordered_passes.last().unwrap().pass,
            Xmap10PassKind::SwapLuminanceHistory
        ));
        assert_eq!(
            plan.ordered_passes.last().unwrap().reads,
            [
                Xmap10TargetRole::CurrentLuminance,
                Xmap10TargetRole::HistoryLuminance,
            ]
        );
        let restore = &plan.ordered_passes[plan.ordered_passes.len() - 2];
        assert_eq!(restore.pass, Xmap10PassKind::RestoreScene);
        assert_eq!(restore.writes, [Xmap10TargetRole::Swapchain]);
    }

    #[test]
    fn temporal_receipt_is_bound_to_the_concrete_reset_or_prior_receipt() {
        let reset = Xmap10TemporalAdaptationReceiptRef::seal(
            1,
            Xmap10TemporalAdaptationSource::EngineResourceReset {
                reason: crate::TemporalLuminanceResetReason::ResourcesAllocated,
                main_scene_resource_identity_sha256: sha(9),
                allocation_generation: 1,
            },
        )
        .unwrap();
        let mut tampered = reset.clone();
        if let Xmap10TemporalAdaptationSource::EngineResourceReset {
            allocation_generation,
            ..
        } = &mut tampered.source
        {
            *allocation_generation = 2;
        }
        assert!(tampered.validate().is_err());

        let continued = Xmap10TemporalAdaptationReceiptRef::seal(
            2,
            Xmap10TemporalAdaptationSource::Continue {
                prior_frame_sequence: 1,
                prior_receipt_sha256: reset.receipt_sha256,
            },
        )
        .unwrap();
        assert!(continued.validate().is_ok());
        assert!(Xmap10TemporalAdaptationReceiptRef::seal(
            2,
            Xmap10TemporalAdaptationSource::Continue {
                prior_frame_sequence: 2,
                prior_receipt_sha256: sha(7),
            },
        )
        .is_err());
    }

    #[test]
    fn odd_scene_uses_floor_half_then_ceil_half_luminance_and_exact_stop() {
        let plan = Xmap10TargetPlan::new(1365, 767).unwrap();
        let bright = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::BrightPass)
            .unwrap();
        let ping = plan
            .ordered_targets
            .iter()
            .find(|target| target.role == Xmap10TargetRole::LuminancePing)
            .unwrap();
        assert_eq!((bright.width, bright.height), (682, 383));
        assert_eq!((ping.width, ping.height), (341, 192));

        let extents = luminance_downsample_viewports(1365, 767).unwrap();
        assert_eq!(
            extents.first(),
            Some(&Xmap10ViewportExtent {
                width: 341,
                height: 192,
            })
        );
        let last = extents.last().unwrap();
        assert!(last.width > 3 || last.height > 3);
        assert!((last.width + 1) / 2 <= 3 && (last.height + 1) / 2 <= 3);
    }

    #[test]
    fn target_plan_owns_complete_main_scene_color_depth_descriptor() {
        let plan = Xmap10TargetPlan::new(1920, 1080).unwrap();
        let desc = plan.production_main_scene_desc().unwrap();
        assert_eq!(
            (
                desc.color_attachment_ordinal,
                desc.color_width,
                desc.color_height,
                desc.color_format,
                desc.color_samples,
                desc.color_quality,
            ),
            (0, 1920, 1080, DXGI_FORMAT_R16G16B16A16_FLOAT, 1, 0)
        );
        assert_eq!(
            (
                desc.depth_attachment_ordinal,
                desc.depth_width,
                desc.depth_height,
                desc.depth_format,
                desc.depth_samples,
                desc.depth_quality,
            ),
            (
                u32::try_from(plan.ordered_targets.len()).unwrap(),
                1920,
                1080,
                45,
                1,
                0
            )
        );

        let mut tampered = plan.clone();
        tampered.main_scene_depth.dxgi_format = 40;
        tampered.main_scene_depth.descriptor_sha256 =
            main_scene_depth_descriptor_digest(&tampered.main_scene_depth);
        tampered.deterministic_sha256 = target_plan_digest(&tampered);
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn target_plan_rejects_non_half_resolution_bloom() {
        let mut plan = target_plan();
        let bloom = plan
            .ordered_targets
            .iter_mut()
            .find(|target| target.role == Xmap10TargetRole::RestoreBloom)
            .unwrap();
        bloom.width = 959;
        bloom.descriptor_sha256 = target_descriptor_digest(bloom);
        plan.deterministic_sha256 = target_plan_digest(&plan);
        assert!(plan.validate().is_err());
    }

    #[test]
    fn grade_inputs_reject_nonfinite_and_missing_lut() {
        let mut grade = Xmap10PerFrameGradeInputs {
            frame_sequence: 8,
            ordered_volume_names: vec!["max_distance".into()],
            ordered_volume_weight_f32_bits: vec![1.0f32.to_bits()],
            resolved_grade_receipt_sha256: sha(2),
            lut_program: single_lut_program(content("gfx/world/colorcorrection_farrange.tga", 3)),
            hsv_f32_bits: [
                0.0f32.to_bits(),
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                0.0f32.to_bits(),
            ],
            color_balance_f32_bits: [1.0f32.to_bits(); 3],
            middle_grey_f32_bits: 0.55f32.to_bits(),
            luminance_white_squared_f32_bits: 1.0f32.to_bits(),
            bloom_width_f32_bits: 1.5f32.to_bits(),
            bloom_scale_f32_bits: 0.9f32.to_bits(),
            bright_threshold_f32_bits: 0.9f32.to_bits(),
            emissive_bloom_strength_f32_bits: 1.0f32.to_bits(),
            min_hdr_adjustment_f32_bits: 0.5f32.to_bits(),
            max_hdr_adjustment_f32_bits: 1.0f32.to_bits(),
            hdr_adjustment_speed_f32_bits: 15.0f32.to_bits(),
            deterministic_sha256: String::new(),
        };
        grade.deterministic_sha256 = grade_inputs_digest(&grade);
        assert!(grade.validate().is_ok());
        grade.hdr_adjustment_speed_f32_bits = 0.0f32.to_bits();
        grade.deterministic_sha256 = grade_inputs_digest(&grade);
        assert!(
            grade.validate().is_ok(),
            "effective HDR_ADJUSTMENT_SPEED=0 is an exact frozen-exposure mod value"
        );
        grade.hdr_adjustment_speed_f32_bits = 15.0f32.to_bits();
        grade.bloom_width_f32_bits = f32::NAN.to_bits();
        grade.deterministic_sha256 = grade_inputs_digest(&grade);
        assert!(grade.validate().is_err());
        grade.bloom_width_f32_bits = 1.5f32.to_bits();
        grade.lut_program.ordered_lut_sources.clear();
        grade.lut_program.deterministic_sha256 = lut_blend_program_digest(&grade.lut_program);
        grade.deterministic_sha256 = grade_inputs_digest(&grade);
        assert!(grade.validate().is_err());
    }

    #[test]
    fn native_receipts_must_bind_exact_prepared_bytes() {
        let compositor = Xmap10NativeCompositorReceiptRef {
            schema: XMAP10_NATIVE_COMPOSITOR_RECEIPT_SCHEMA.into(),
            prepared_frame_sha256: sha(4),
            target_plan_sha256: target_plan().deterministic_sha256,
            ordered_pass_count: 15,
            receipt_sha256: sha(5),
        };
        assert_ne!(compositor.prepared_frame_sha256, sha(6));
        assert!(validate_sha256(&compositor.receipt_sha256, "receipt").is_ok());
    }
}

#[cfg(test)]
mod scene_content_tests {
    use super::*;
    #[test]
    fn postfx_closure_admits_and_requires_the_injected_shader_header() {
        let root = crate::test_support::temp_test_dir("postfx_shader_header");
        let lut = "gfx/world/test.tga";
        let paths = EFFECT_ROOTS.into_iter().chain([POSTEFFECT_VOLUMES, GRAPHICS_DEFINES, crate::shader_compile::DX11_PLATFORM_HEADER, lut])
            .map(str::to_ascii_lowercase).collect::<BTreeSet<_>>();
        for path in &paths {
            let file = root.join(path); std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, "Effect test { VertexShader = \"vs\" PixelShader = \"ps\" }\n").unwrap();
        }
        let resolver = GameFileResolver::vanilla(root.clone()).unwrap();
        let (resolver, admission) = crate::test_support::admitted_vanilla_resolver(&resolver, &paths, "postfx-header-test");
        let mut closure = Xmap10ContentClosure::admit(&resolver, &admission, &[lut.into()]).unwrap();
        let header = crate::shader_compile::DX11_PLATFORM_HEADER.to_ascii_lowercase();
        assert!(closure.ordered_used_content.iter().any(|row| row.normalized_game_relative_path == header));
        closure.ordered_used_content.retain(|row| row.normalized_game_relative_path != header);
        closure.deterministic_sha256 = content_closure_digest(&closure);
        assert!(closure.validate().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
