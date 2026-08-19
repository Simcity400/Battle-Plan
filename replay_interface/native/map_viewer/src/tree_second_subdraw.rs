
use crate::d3d11_replay::NativeD3d11TerrainBundle;
use crate::mod_resolver::GameFileResolver;
use crate::shader_compile::parse_dxbc_reflection;
use crate::tree_files::TREE_SHADER_SOURCE;
use crate::Result;
use std::collections::BTreeSet;

pub const TREE_FULL_PS_FNV1A64: &str = "277b7b5e8964214f";

pub const TREE_LOW_GFX_PS_FNV1A64: &str = "6b77b6df9ba8f03d";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreePixelShaderKind {
    Full,
    LowGfx,
    Unlit,
    Unclassified,
}

impl TreePixelShaderKind {
    pub fn label(self) -> &'static str {
        match self {
            TreePixelShaderKind::Full => "tree",
            TreePixelShaderKind::LowGfx => "tree_low_gfx",
            TreePixelShaderKind::Unlit => "treeunlit",
            TreePixelShaderKind::Unclassified => "unclassified-tree-family",
        }
    }
}

fn classify_reflection(blob: &[u8]) -> TreePixelShaderKind {
    let Ok(reflection) = parse_dxbc_reflection(blob) else {
        return TreePixelShaderKind::Unclassified;
    };
    let names: BTreeSet<&str> = reflection
        .resources
        .iter()
        .map(|resource| resource.name.as_str())
        .collect();
    let has = |name: &str| names.contains(name);
    let cbuffers = reflection.cbuffers.len();
    let low_gfx_only = !has("EnvironmentMap_Texture")
        && !has("LightDataMap_Texture")
        && !has("LightIndexMap_Texture")
        && !has("SnowMudData_Texture");
    let full_detail = has("EnvironmentMap_Texture")
        && has("LightDataMap_Texture")
        && has("LightIndexMap_Texture")
        && has("SnowMudData_Texture");
    let unlit = !has("NormalMap_Texture")
        && !has("GradientBorderChannel1_Texture")
        && !has("ColorMapSecond_Texture")
        && cbuffers <= 1;
    if full_detail {
        TreePixelShaderKind::Full
    } else if unlit {
        TreePixelShaderKind::Unlit
    } else if low_gfx_only && has("NormalMap_Texture") && has("GradientBorderChannel1_Texture") {
        TreePixelShaderKind::LowGfx
    } else {
        TreePixelShaderKind::Unclassified
    }
}

pub fn identify_tree_pixel_shader(
    resolver: &GameFileResolver,
    blob: &[u8],
) -> Result<TreePixelShaderKind> {
    let reflection_kind = classify_reflection(blob);

    #[cfg(windows)]
    {
        use crate::shader_compile::{compare_effect_stage_to_captured, ShaderStage};
        for (effect, kind) in [
            ("tree", TreePixelShaderKind::Full),
            ("tree_low_gfx", TreePixelShaderKind::LowGfx),
            ("treeunlit", TreePixelShaderKind::Unlit),
        ] {
            let comparison = compare_effect_stage_to_captured(
                resolver,
                TREE_SHADER_SOURCE,
                effect,
                ShaderStage::Pixel,
                blob,
                "captured tree pixel shader",
            )?;
            if comparison.exact_hash_flags.is_some() {
                return Ok(kind);
            }
        }
        return Ok(reflection_kind);
    }
    #[cfg(not(windows))]
    {
        let _ = resolver;
        Ok(reflection_kind)
    }
}

#[derive(Clone, Debug)]
pub struct TreePixelShaderEvidence {
    pub bytecode_file: String,
    pub fnv1a64: String,
    pub byte_len: usize,
    pub kind: TreePixelShaderKind,
    pub bound_to_draw: bool,
}

#[derive(Clone, Debug)]
pub struct TreeSecondSubDrawVerdict {
    pub shaders: Vec<TreePixelShaderEvidence>,
    pub low_gfx_created_unused: bool,
    pub low_gfx_bound_unrendered: bool,
}

impl TreeSecondSubDrawVerdict {
    pub fn layer_note(&self) -> Option<String> {
        if self.low_gfx_bound_unrendered {
            return Some(
                "trees layer seam: this session's trees pass binds the low-gfx tree pixel \
                 shader 6b77b6df9ba8f03d (gfx/FX/tree.shader:tree_low_gfx) in a draw that \
                 replay did not render"
                    .to_string(),
            );
        }
        if self.low_gfx_created_unused {
            return Some(
                "trees layer complete: the low-gfx tree pixel shader 6b77b6df9ba8f03d is \
                 gfx/FX/tree.shader:tree_low_gfx (the LOW_END_GFX variant of the full tree \
                 shader), compiled by the engine but bound to no draw at the captured \
                 graphics settings — the full-detail tree family is the rendered path, so \
                 there is no unrendered second sub-draw"
                    .to_string(),
            );
        }
        None
    }
}

pub fn tree_second_subdraw_verdict(
    resolver: &GameFileResolver,
    bundle: &NativeD3d11TerrainBundle,
) -> Result<TreeSecondSubDrawVerdict> {
    let bound_hashes: BTreeSet<String> = bundle
        .frame
        .draws
        .iter()
        .map(|draw| draw.shader.fnv1a64.clone())
        .collect();

    let mut shaders = Vec::new();
    let mut seen = BTreeSet::new();
    let dir = bundle.bundle_dir();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("tree_pixel_shader_") && name.ends_with(".bin")) {
                continue;
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            let blob = std::fs::read(entry.path()).map_err(|err| format!("read {name}: {err}"))?;
            let kind = identify_tree_pixel_shader(resolver, &blob)?;
            let fnv1a64 = name
                .trim_start_matches("tree_pixel_shader_")
                .trim_end_matches(".bin")
                .to_string();
            let bound_to_draw = bound_hashes.contains(&fnv1a64);
            shaders.push(TreePixelShaderEvidence {
                bytecode_file: name,
                fnv1a64,
                byte_len: blob.len(),
                kind,
                bound_to_draw,
            });
        }
    }
    shaders.sort_by(|a, b| a.bytecode_file.cmp(&b.bytecode_file));

    let low_gfx = shaders
        .iter()
        .find(|shader| shader.kind == TreePixelShaderKind::LowGfx);
    let low_gfx_created_unused = low_gfx.map(|shader| !shader.bound_to_draw).unwrap_or(false);
    let low_gfx_bound_unrendered = low_gfx.map(|shader| shader.bound_to_draw).unwrap_or(false);

    Ok(TreeSecondSubDrawVerdict {
        shaders,
        low_gfx_created_unused,
        low_gfx_bound_unrendered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    fn vanilla_dir() -> PathBuf {
        replay_test_gates::require_vanilla_game_root()
    }

    fn evidence_session() -> PathBuf {
        corpus::trees2_capture_dir().expect(
            "no trees-pass capture (TREES2_CAPTURE_DIR, else the documented directories) carries \
             both the trees bundle and the tree pixel shader, but this test was compiled as \
             runnable",
        )
    }

    #[test]
    #[cfg_attr(
        not(gate_trees2_shader_capture_with_hoi4),
        ignore = "TREES2_CAPTURE_DIR naming a capture with the tree shader evidence, plus the game install"
    )]
    fn low_gfx_and_full_pixel_shaders_identify_by_effect() {
        let game_dir = vanilla_dir();
        let session = evidence_session();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");

        let full = std::fs::read(session.join("tree_pixel_shader_277b7b5e8964214f.bin"))
            .expect("full tree PS bin");
        let low = std::fs::read(session.join("tree_pixel_shader_6b77b6df9ba8f03d.bin"))
            .expect("low-gfx tree PS bin");

        assert_eq!(
            identify_tree_pixel_shader(&resolver, &full).expect("classify full"),
            TreePixelShaderKind::Full,
            "277b7b5e must identify as the full-detail `tree` Effect PS"
        );
        assert_eq!(
            identify_tree_pixel_shader(&resolver, &low).expect("classify low"),
            TreePixelShaderKind::LowGfx,
            "6b77b6df (the historical `second sub-draw`) must identify as the LOW_END_GFX \
             `tree_low_gfx` Effect PS, not a genuine missing draw"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_trees2_shader_capture_with_hoi4),
        ignore = "TREES2_CAPTURE_DIR naming a capture with the tree shader evidence, plus the game install"
    )]
    fn real_bundle_verdict_is_low_gfx_created_but_unused() {
        let game_dir = vanilla_dir();
        let session = evidence_session();
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let bundle =
            NativeD3d11TerrainBundle::load_trees(session.join("map_frame_trees_pass_bundle.json"))
                .expect("load trees bundle");

        let verdict = tree_second_subdraw_verdict(&resolver, &bundle).expect("verdict");

        let low = verdict
            .shaders
            .iter()
            .find(|shader| shader.kind == TreePixelShaderKind::LowGfx)
            .expect("low-gfx tree PS classified in bundle dir");
        assert_eq!(low.fnv1a64, TREE_LOW_GFX_PS_FNV1A64);
        assert!(
            !low.bound_to_draw,
            "no captured session binds the low-gfx tree PS to a draw"
        );

        let full = verdict
            .shaders
            .iter()
            .find(|shader| shader.kind == TreePixelShaderKind::Full)
            .expect("full tree PS classified in bundle dir");
        assert_eq!(full.fnv1a64, TREE_FULL_PS_FNV1A64);

        assert!(
            verdict.low_gfx_created_unused,
            "verdict = created-but-unused"
        );
        assert!(
            !verdict.low_gfx_bound_unrendered,
            "no genuine unrendered second sub-draw exists in captured evidence"
        );

        let note = verdict.layer_note().expect("a layer note");
        assert!(
            note.contains("compiled by the engine but bound to no draw"),
            "the note must state the low-gfx variant is compiled-but-unused, not a gap: {note}"
        );
        assert!(
            !note.contains("remains unrendered"),
            "the note must NOT assert an unrendered draw: {note}"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_trees2_shader_capture),
        ignore = "TREES2_CAPTURE_DIR naming a capture with the tree shader evidence"
    )]
    fn no_captured_draw_binds_the_low_gfx_pixel_shader() {
        let session = evidence_session();
        let bundle =
            NativeD3d11TerrainBundle::load_trees(session.join("map_frame_trees_pass_bundle.json"))
                .expect("load trees bundle");
        for draw in &bundle.frame.draws {
            assert_ne!(
                draw.shader.fnv1a64, TREE_LOW_GFX_PS_FNV1A64,
                "a captured trees-pass draw unexpectedly binds the low-gfx PS"
            );
        }
    }

    #[test]
    fn layer_note_reports_a_genuine_unrendered_bound_draw() {
        let verdict = TreeSecondSubDrawVerdict {
            shaders: vec![TreePixelShaderEvidence {
                bytecode_file: "tree_pixel_shader_6b77b6df9ba8f03d.bin".to_string(),
                fnv1a64: TREE_LOW_GFX_PS_FNV1A64.to_string(),
                byte_len: 15592,
                kind: TreePixelShaderKind::LowGfx,
                bound_to_draw: true,
            }],
            low_gfx_created_unused: false,
            low_gfx_bound_unrendered: true,
        };
        let note = verdict.layer_note().expect("note");
        assert!(note.contains("replay did not render"), "{note}");
    }

    #[test]
    fn no_low_gfx_shader_present_yields_no_note() {
        let verdict = TreeSecondSubDrawVerdict {
            shaders: vec![TreePixelShaderEvidence {
                bytecode_file: "tree_pixel_shader_277b7b5e8964214f.bin".to_string(),
                fnv1a64: TREE_FULL_PS_FNV1A64.to_string(),
                byte_len: 20532,
                kind: TreePixelShaderKind::Full,
                bound_to_draw: true,
            }],
            low_gfx_created_unused: false,
            low_gfx_bound_unrendered: false,
        };
        assert!(verdict.layer_note().is_none());
        let _ = Path::new("");
    }
}
