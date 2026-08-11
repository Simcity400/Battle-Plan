//! TREES2 — identify the "second tree sub-draw" pixel shader
//! `6b77b6df9ba8f03d` and gate the long-standing layer-gap warning on real
//! evidence instead of an unconditional assumption.
//!
//! ## What the "second sub-draw" actually is
//!
//! `gfx/FX/tree.shader` declares four Effects that share one lit
//! `PixelShader` body (constants.fxh/standardfuncsgfx.fxh/shadow.fxh/
//! tiled_pointlights.fxh/fow.fxh includes, one `SRC_ALPHA`/`INV_SRC_ALPHA`
//! `BlendState`, `WriteMask = RED|GREEN|BLUE`):
//!
//! * `tree`          — full-detail lit PS (the `#ifndef LOW_END_GFX` branches
//!                     for SnowMud, point lights, the `EnvironmentMap`
//!                     reflection cube and distance fog are all active).
//! * `tree_low_gfx`  — the SAME `PixelShader` compiled with `LOW_END_GFX`
//!                     defined, so those four branches are stripped.
//! * `treeshadow`    — the depth-only shadow PS.
//! * `treeunlit`     — the fog/shadow-data PS (`PixelShaderUnlit`).
//!
//! The OFF-3 byte-exact shipped-shader recompiler proves (see
//! [`identify_tree_pixel_shader`] and the tests):
//!
//! * captured `tree_pixel_shader_277b7b5e8964214f.bin` (20532 B) == effect
//!   `tree` PS, byte-identical DXBC.
//! * captured `tree_pixel_shader_6b77b6df9ba8f03d.bin` (15592 B) == effect
//!   `tree_low_gfx` PS, byte-identical DXBC. Its reflection binds cb0+cb1 and
//!   the 24 low-gfx resources only — no `EnvironmentMap`/`LightDataMap`/
//!   `LightIndexMap`/`SnowMudData` and no cb2, exactly the LOW_END_GFX cut.
//!
//! So `6b77b6df9ba8f03d` is **not** a genuine "second draw" of the trees pass
//! — it is the mutually-exclusive LOW-graphics-tier variant of the SAME lit
//! tree pixel shader. The engine picks `tree` OR `tree_low_gfx` by the
//! LOW_END_GFX graphics setting; it does not draw both. Every captured
//! trees-pass bundle on the high-settings capture machine binds only the
//! full-detail family (`277b7b5e…` and its season/night sibling
//! `b0b3f4bd…`); `live_trees_measure` captured the WHOLE trees pass as 8
//! `DrawIndexedInstanced` batches, all `277b7b5e`, none `6b77b6df`. The
//! `6b77b6df` bytecode is only ever *created* (dumped at
//! CreatePixelShader time), never *bound to a draw*.
//!
//! Therefore the old unconditional warning ("missing second sub-draw …
//! remains unrendered") asserts a draw that no evidence shows. This module
//! replaces it with an evidence-gated verdict: the warning only fires when a
//! bundle genuinely contains a draw bound to the low-gfx PS that replay did
//! not render (no captured session does today).

use crate::d3d11_replay::NativeD3d11TerrainBundle;
use crate::mod_resolver::GameFileResolver;
use crate::shader_compile::parse_dxbc_reflection;
use crate::tree_files::TREE_SHADER_SOURCE;
use crate::Result;
use std::collections::BTreeSet;

/// FNV1a-64 of the captured full-detail (`tree` Effect) tree pixel shader,
/// proven byte-exact against `gfx/FX/tree.shader:tree` PS by the OFF-3
/// recompiler. Season/night sessions bind a sibling full-detail hash; both
/// belong to the full-detail family, so classification never hardcodes the
/// full hash for the *gap* decision — it is documented here as evidence only.
pub const TREE_FULL_PS_FNV1A64: &str = "277b7b5e8964214f";

/// FNV1a-64 of the captured LOW_END_GFX (`tree_low_gfx` Effect) tree pixel
/// shader — the shader the historical warning called the "second sub-draw".
/// Proven byte-exact against `gfx/FX/tree.shader:tree_low_gfx` PS.
pub const TREE_LOW_GFX_PS_FNV1A64: &str = "6b77b6df9ba8f03d";

/// Which tree.shader Effect a captured pixel-shader bytecode blob matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreePixelShaderKind {
    /// Effect `tree` — full-detail lit PS (env cube + point lights + snow +
    /// fog branches active).
    Full,
    /// Effect `tree_low_gfx` — LOW_END_GFX variant (those branches stripped).
    LowGfx,
    /// Effect `treeunlit` — the fog/shadow-data PS.
    Unlit,
    /// A tree-family pixel shader whose Effect could not be matched from the
    /// shipped file (e.g. a season/night permutation or a modded shader). Not
    /// a gap — just unclassified.
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

/// The resource-set fingerprint that distinguishes the tree Effects at the
/// reflection level, portable to non-Windows builds where D3DCompile is
/// unavailable. Derived from `gfx/FX/tree.shader` + the OFF-3 measured
/// captures (documented in the module header).
///
/// * `tree`         binds EnvironmentMap + LightDataMap + LightIndexMap +
///                  SnowMudData and cb0/cb1/cb2 (33 resources).
/// * `tree_low_gfx` drops all four of those textures and cb2 (24 resources,
///                  cb0/cb1 only).
/// * `treeunlit`    binds only DiffuseMap/TintMap/SeasonMap/ColorMap/ShadowMap/
///                  TreeMaskTexture and cb0 (13 resources).
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
    // treeunlit binds neither the border channels nor a normal map and keeps
    // exactly one cbuffer.
    let unlit = !has("NormalMap_Texture")
        && !has("GradientBorderChannel1_Texture")
        && !has("ColorMapSecond_Texture")
        && cbuffers <= 1;
    if full_detail {
        TreePixelShaderKind::Full
    } else if unlit {
        TreePixelShaderKind::Unlit
    } else if low_gfx_only && has("NormalMap_Texture") && has("GradientBorderChannel1_Texture") {
        // Full-lit body minus the four LOW_END_GFX textures: keeps the normal
        // map + gradient borders + secondary color but not env/lights/snow.
        TreePixelShaderKind::LowGfx
    } else {
        TreePixelShaderKind::Unclassified
    }
}

/// Identify a captured tree pixel-shader bytecode blob against the shipped
/// `tree.shader` Effects. On Windows the OFF-3 byte-exact recompiler is the
/// authority (byte-identical DXBC == that Effect); a reflection fingerprint is
/// the portable fallback and the Windows cross-check.
pub fn identify_tree_pixel_shader(
    resolver: &GameFileResolver,
    blob: &[u8],
) -> Result<TreePixelShaderKind> {
    // Portable classification first: it never fails and gives the answer on
    // non-Windows / headless builds.
    let reflection_kind = classify_reflection(blob);

    #[cfg(windows)]
    {
        use crate::shader_compile::{compare_effect_stage_to_captured, ShaderStage};
        // Byte-exact recompile authority. The captured-file name is only used
        // for diagnostics inside the comparator.
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
        // No byte-exact match (season/night permutation or modded shader):
        // fall back to the reflection fingerprint.
        return Ok(reflection_kind);
    }
    #[cfg(not(windows))]
    {
        let _ = resolver;
        Ok(reflection_kind)
    }
}

/// One captured tree pixel shader present in a trees-pass bundle directory,
/// with its Effect identification and whether any *draw* in the bundle binds
/// it. The gap decision keys off `bound_to_draw`.
#[derive(Clone, Debug)]
pub struct TreePixelShaderEvidence {
    /// The `tree_pixel_shader_<fnv>.bin` file name.
    pub bytecode_file: String,
    /// FNV1a-64 (from the file name / capture manifest).
    pub fnv1a64: String,
    pub byte_len: usize,
    pub kind: TreePixelShaderKind,
    /// True when a `draws[]` record in the bundle binds this pixel shader
    /// (i.e. the live engine actually issued a draw with it at capture time).
    pub bound_to_draw: bool,
}

/// The trees-pass "second sub-draw" verdict for one bundle.
#[derive(Clone, Debug)]
pub struct TreeSecondSubDrawVerdict {
    /// Every `tree_pixel_shader_*.bin` in the bundle directory.
    pub shaders: Vec<TreePixelShaderEvidence>,
    /// True when the LOW_END_GFX pixel shader (`tree_low_gfx`) is present as a
    /// created-but-unbound bytecode blob (dumped at CreatePixelShader time)
    /// and NO draw in the bundle binds it — the real state of every captured
    /// session. This is *not* a gap: it is a compiled-but-unused gfx-tier
    /// variant.
    pub low_gfx_created_unused: bool,
    /// True only when a draw in the bundle binds the low-gfx PS but replay did
    /// not render it — a genuine unrendered second sub-draw. No captured
    /// session hits this today; kept so the warning fires lawfully if a future
    /// (e.g. LOW_END_GFX-machine) capture ever records one.
    pub low_gfx_bound_unrendered: bool,
}

impl TreeSecondSubDrawVerdict {
    /// The human-readable layer note for this verdict. `Some` only when there
    /// is something to say; the caller pushes it into `layer_gaps`.
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

/// Classify every `tree_pixel_shader_*.bin` in a loaded trees bundle and
/// decide the second-sub-draw verdict from the bundle's real draw records.
pub fn tree_second_subdraw_verdict(
    resolver: &GameFileResolver,
    bundle: &NativeD3d11TerrainBundle,
) -> Result<TreeSecondSubDrawVerdict> {
    // Which pixel-shader hashes does a real draw bind?
    let bound_hashes: BTreeSet<String> = bundle
        .frame
        .draws
        .iter()
        .map(|draw| draw.shader.fnv1a64.clone())
        .collect();

    let mut shaders = Vec::new();
    let mut seen = BTreeSet::new();
    // The bundle's draws reference their PS by file; also scan the directory
    // for created-but-unbound tree PS blobs (dumped at CreatePixelShader).
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

    /// The crate's gate table, included so this test module resolves each
    /// corpus through EXACTLY the candidate list the build-time probe used.
    /// Two copies of a candidate list drift; one cannot.
    #[allow(dead_code)]
    mod corpus {
        include!("../test_gates.rs");
    }

    fn vanilla_dir() -> PathBuf {
        replay_test_gates::require_vanilla_game_root()
    }

    /// The real trees-pass evidence: both tree pixel shaders are captured as
    /// created-but-(mostly)-unbound bytecode blobs at CreatePixelShader time.
    /// Returns a session that has the second-sub-draw `.bin` present AND a
    /// complete-enough capture manifest for `load_trees` to validate (i.e. the
    /// required prior `terrain_active_pass` moment is present). The spec's
    /// `session_CAPFIX2_GAPS_4` has the bins but is a gaps-only capture missing
    /// the terrain moment, so it is only used for the loose-bin identification
    /// check, not for `load_trees`.
    fn evidence_session() -> PathBuf {
        corpus::trees2_capture_dir().expect(
            "no trees-pass capture (TREES2_CAPTURE_DIR, else the documented directories) carries \
             both the trees bundle and the tree pixel shader, but this test was compiled as \
             runnable",
        )
    }

    /// The LOW_END_GFX pixel shader is byte-exact `tree.shader:tree_low_gfx`,
    /// and the full one is byte-exact `tree.shader:tree`. This is the core
    /// identification the historical warning got wrong (it treated the low-gfx
    /// variant as a missing second draw).
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

    /// On the real bundle, the low-gfx PS is present but bound to NO draw, so
    /// the verdict is `created_unused` (not a gap), and the layer note says so.
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

        // The low-gfx shader is present as a created-but-unbound blob…
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

        // …and the full-detail shader is the one every draw actually binds.
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

    /// Every draw in the real trees bundle binds a full-detail tree PS (never
    /// the low-gfx one) — the ground truth behind the `mutually exclusive gfx
    /// tier` reading.
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
        // Synthetic verdict: a bundle that DID bind the low-gfx PS to a draw
        // replay skipped is the only lawful trigger for the seam warning.
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
