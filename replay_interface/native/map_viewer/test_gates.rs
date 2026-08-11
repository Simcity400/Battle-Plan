// Gate table for `map_viewer`. Included by BOTH `build.rs` (which turns each
// probe into a `cfg`) and `tests/gate_currency.rs` (which re-runs each probe
// to prove the compiled `cfg` still matches reality), so the build-time and
// run-time views of a precondition cannot drift apart.
//
// Every `#[test]` in this crate that needs one of these preconditions carries
// `#[cfg_attr(not(<cfg>), ignore = "requires ...")]` and calls the matching
// `replay_test_gates::require_*` accessor in its body. It must never `return`
// early on a missing precondition: libtest reports that as a pass.

// A NOTE ON CANDIDATE LISTS. Several corpora are addressed by "an environment
// variable, ELSE this documented list of directories". Those fallbacks are not
// decoration: on the development rig they are where the evidence actually
// lives, and dropping them turns a test that RUNS into one reported `ignored`.
// That is strictly worse than the silent pass this mechanism replaced, because
// it removes real coverage instead of merely mislabelling it. Each list below
// is therefore the single source of truth for BOTH the gate probe here and the
// accessor the test body calls (the `#[cfg(test)]` modules reach these
// functions by `include!`ing this same file), so the two can never disagree
// about which directories count.

use std::path::{Path, PathBuf};

/// Preserved CAP4 round corpus: the durable read-only copy used when
/// `BATTLE_PLAN_WAVE1_CAP4_ROUNDS` is unset.
const DEFAULT_WAVE1_CAP4_ROUNDS: &str =
    r"C:\Users\Example\hoi4_capture_corpus_preserved\session_WAVE1_CAP4_rounds";

/// Preserved border-pool corpus, used when `BATTLE_PLAN_BORDER_POOL_DIR` is unset.
const DEFAULT_BORDER_POOL_DIR: &str =
    r"C:\Users\Example\hoi4_border_pool_preserved\session_BORDERZOOM_3";

/// Documented `live_trees_measure` locations, in priority order: the live
/// capture directory, the durable preserved corpus copy (read-only B4
/// evidence), and an older backed-up local cache.
const TREES_MEASURE_CANDIDATES: &[&str] = &[
    r"C:\Users\Example\AppData\Local\hoi4_captures\live_trees_measure",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure",
    r"C:\Users\Example\AppData\Local\Claude_pzs8sxrjxfjjc.data-backup-20260709-103135\LocalCache\Local\hoi4_captures\live_trees_measure",
];

/// Documented `live_PAN2` locations, in priority order.
const TREES2_PAN2_CANDIDATES: &[&str] = &[
    r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\live_PAN2",
    r"C:\Users\Example\AppData\Local\Claude_pzs8sxrjxfjjc.data-backup-20260709-103135\LocalCache\Local\hoi4_captures\live_PAN2",
];

/// Every directory that may hold a trees-pass capture, for the shader-evidence
/// gates that accept either the PAN2 or the trees-measure session.
const TREES2_SESSION_CANDIDATES: &[&str] = &[
    r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN2",
    r"C:\Users\Example\AppData\Local\hoi4_captures\live_trees_measure",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\live_trees_measure",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\live_PAN2",
    r"C:\Users\Example\AppData\Local\hoi4_captures\live_PAN_chunks",
];

/// Canonical preserved full-map capture. It carries the river bundle and map
/// shader bytecode consumed by the corresponding evidence gates.
const LIVE_FINAL_FULL_CANDIDATES: &[&str] = &[
    r"C:\hoi4_evidence\capture_corpus_preserved\live_FINAL_full",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\live_FINAL_full",
];

/// Canonical preserved terrain capture. It carries the TerrainIDMap descriptor
/// and payload plus the captured terrain chunk library.
const GATE18_FINAL_CANDIDATES: &[&str] = &[
    r"C:\hoi4_evidence\capture_corpus_preserved\gate18_final",
    r"C:\Users\Example\hoi4_capture_corpus_preserved\gate18_final",
];

/// The first directory satisfying `accept`, searching the one named by
/// `env_var` FIRST and then the documented defaults.
///
/// The override is the first candidate, not an exclusive one. That is the
/// pre-existing contract for these capture corpora — they were resolved with
/// `[env, defaults..].into_iter().find(..)` — and it matters: a rig whose
/// `TREES2_CAPTURE_DIR` has gone stale still runs the gate against the
/// preserved corpus instead of reporting `ignored` while valid evidence sits
/// on disk. Losing coverage that way would be worse than the silent pass this
/// mechanism replaced.
///
/// (The INSTALL root deliberately behaves differently — see
/// [`replay_test_gates::vanilla_game_root`] — because there a set-but-wrong
/// variable means the operator pointed at a specific build, and quietly
/// grading a different install against it would be its own falsehood. That
/// asymmetry is inherited from the code this replaced, not invented here.)
fn corpus_dir(
    env_var: &str,
    defaults: &[&str],
    accept: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    std::env::var_os(env_var)
        .map(PathBuf::from)
        .into_iter()
        .chain(defaults.iter().map(PathBuf::from))
        .find(|dir| dir.is_dir() && accept(dir))
}

fn has_file(dir: &Path, rel: &str) -> bool {
    dir.join(rel).is_file()
}

/// The CAP4 capture-round corpus, requiring the named round subdirectories.
pub fn wave1_cap4_rounds_dir(rounds: &[&str]) -> Option<PathBuf> {
    let rounds: Vec<String> = rounds.iter().map(|r| (*r).to_string()).collect();
    corpus_dir("BATTLE_PLAN_WAVE1_CAP4_ROUNDS", &[DEFAULT_WAVE1_CAP4_ROUNDS], |dir| {
        rounds.iter().all(|round| dir.join(round).is_dir())
    })
}

/// The captured border vertex-buffer pool.
pub fn border_pool_dir() -> Option<PathBuf> {
    corpus_dir("BATTLE_PLAN_BORDER_POOL_DIR", &[DEFAULT_BORDER_POOL_DIR], |dir| {
        has_file(dir, "borders_frame_draw000_ia_vertex_buffer_slot0.bin")
    })
}

/// The `live_trees_measure` trees-pass capture.
pub fn trees_measure_capture_dir() -> Option<PathBuf> {
    corpus_dir("TREES_MEASURE_CAPTURE_DIR", TREES_MEASURE_CANDIDATES, |dir| {
        has_file(dir, "map_frame_trees_pass_bundle.json")
    })
}

/// A trees-pass capture carrying the captured tree TEMPLATE buffers.
pub fn trees2_template_capture_dir() -> Option<PathBuf> {
    corpus_dir("TREES2_CAPTURE_DIR", TREES2_PAN2_CANDIDATES, |dir| {
        has_file(dir, "trees_frame_draw000_ia_vertex_buffer_slot0.bin")
            && has_file(dir, "trees_frame_draw000_ia_index_buffer.bin")
    })
}

/// A trees-pass capture carrying the tree SHADER evidence.
pub fn trees2_capture_dir() -> Option<PathBuf> {
    corpus_dir("TREES2_CAPTURE_DIR", TREES2_SESSION_CANDIDATES, |dir| {
        has_file(dir, "map_frame_trees_pass_bundle.json")
            && has_file(dir, "tree_pixel_shader_6b77b6df9ba8f03d.bin")
    })
}

const RESTORE_GATE_1_DEFAULT: &str =
    r"C:\hoi4_evidence\capture_corpus_preserved\restore_gate_1";
const RESTORE_MAIN_SCENE: &str =
    "restore_frame_draw_map_frame_shared_payload_ps_srv_MainScene_b811e76916a16e8c.d3d11";
const RESTORE_AVERAGE_LUMINANCE: &str =
    "restore_frame_draw_map_frame_shared_payload_ps_srv_AverageLuminanceTexture_a86bb8b3d96548fd.d3d11";
const LEGACY_TERRAIN_ID_MAIN_CAPTURE: &str = r".claude\worktrees\nifty-jemison-bbc86f\replay_interface\native\map_viewer\out\captures\session_20260702d";
const UNDERWATER_CAPTURE_PAYLOAD: &str = r"C:\Users\Example\AppData\Local\hoi4_captures\session_CAPFIX2_GAPS_4\underwater_frame_draw_map_frame_shared_payload_ps_srv_TerrainDiffuse_c09909744edaab9c.d3d11";

fn restore_gate_1_root() -> PathBuf {
    if let Some(root) = std::env::var_os("BATTLE_PLAN_RESTORE_GATE_1_ROOT").or_else(|| std::env::var_os("HOI4_RESTORE_GATE_1_ROOT")) {
        return PathBuf::from(root);
    }
    std::env::var_os("BATTLE_PLAN_CAPTURES_ROOT").or_else(|| std::env::var_os("HOI4_CAPTURES_ROOT"))
        .map(PathBuf::from)
        .map(|root| root.join("restore_gate_1"))
        .unwrap_or_else(|| PathBuf::from(RESTORE_GATE_1_DEFAULT))
}

/// The mined RenderDoc evidence store: `BATTLE_PLAN_RENDERDOC_MINED`, else the
/// per-user default under `%LOCALAPPDATA%`.
pub fn renderdoc_mined_root() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("BATTLE_PLAN_RENDERDOC_MINED").or_else(|| std::env::var_os("HOI4_RENDERDOC_MINED")) {
        let dir = PathBuf::from(value);
        return dir.is_dir().then_some(dir);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join("hoi4_renderdoc").join("mined"))
        .filter(|dir| dir.is_dir())
}

/// The retained production-complete-frame capture:
/// `BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME`, else the documented default.
/// The capture is only usable when its `gpu` subdirectory is present.
pub fn retained_production_frame_root() -> Option<PathBuf> {
    let root = std::env::var_os("BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME").or_else(|| std::env::var_os("HOI4_RETAINED_PRODUCTION_COMPLETE_FRAME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                r"C:\tmp\hoi4_rendering_law_capture\20260717T000008Z-production-complete-frame",
            )
        });
    root.join("gpu").is_dir().then_some(root)
}

fn map_bundle_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("out/map_bundle")
}

fn replay_fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/positive")
}

fn reflection_cube_proof_dir() -> Option<PathBuf> {
    replay_test_gates::dir_from_env("REFLECTION_CUBE_PROOF_DIR").filter(|dir| {
        replay_test_gates::existing_file(dir.join("reflection.dds")).is_some()
            && replay_test_gates::existing_file(dir.join("cap_c.bin")).is_some()
    })
}

pub fn river_capture_dirs() -> Option<Vec<PathBuf>> {
    if let Some(value) = std::env::var_os("RIVER_CAPTURE_DIRS") {
        let dirs: Vec<PathBuf> = value
            .to_string_lossy()
            .split(';')
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .collect();
        return (!dirs.is_empty() && dirs.iter().all(|dir| dir.is_dir())).then_some(dirs);
    }
    LIVE_FINAL_FULL_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|dir| has_file(dir, "map_frame_rivers_pass_bundle.json"))
        .map(|dir| vec![dir])
}

pub fn shader_capture_dir() -> Option<PathBuf> {
    corpus_dir(
        "SHADER_CAPTURE_DIR",
        LIVE_FINAL_FULL_CANDIDATES,
        |dir| {
            has_file(dir, "map_frame_bundle.json")
                && has_file(dir, "pdxmap_pixel_shader_ce5e5185d46e214b.bin")
        },
    )
}

pub fn terrain_id_capture_dir() -> Option<PathBuf> {
    std::iter::once(PathBuf::from(LEGACY_TERRAIN_ID_MAIN_CAPTURE))
        .chain(GATE18_FINAL_CANDIDATES.iter().map(PathBuf::from))
        .find(|dir| {
            has_file(dir, "terrain_id_map_capture.json")
                && has_file(dir, "terrain_id_map.rgba")
        })
}

pub fn terrain_mesh_corpus() -> Option<(PathBuf, PathBuf)> {
    let capture_override = std::env::var_os("TERRAIN_MESH_LAW_CORPUS_CAPTURE");
    let game_override = std::env::var_os("TERRAIN_MESH_LAW_CORPUS_GAME");
    if capture_override.is_some() || game_override.is_some() {
        let capture = PathBuf::from(capture_override?);
        let game = PathBuf::from(game_override?);
        return (capture.is_dir() && game.is_dir()).then_some((capture, game));
    }
    let capture = GATE18_FINAL_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|dir| has_file(dir, "terrain_chunk_library.json"))?;
    let game = replay_test_gates::vanilla_game_root()?;
    Some((capture, game))
}

pub const GATES: &[replay_test_gates::Gate] = &[
    replay_test_gates::Gate {
        cfg: "gate_renderdoc_mined",
        reason: "the mined RenderDoc evidence store (set BATTLE_PLAN_RENDERDOC_MINED, else %LOCALAPPDATA%\\hoi4_renderdoc\\mined)",
        env_vars: &["BATTLE_PLAN_RENDERDOC_MINED", "LOCALAPPDATA"],
        probe: || renderdoc_mined_root().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_retained_production_frame",
        reason: "the retained production-complete-frame capture (set BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME to its root)",
        env_vars: &["BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME"],
        probe: || retained_production_frame_root().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_hoi4_install",
        reason: replay_test_gates::GAME_ROOT_REASON,
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || replay_test_gates::vanilla_game_root().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_border_pool_with_hoi4",
        reason: "the captured border vertex buffer (BATTLE_PLAN_BORDER_POOL_DIR, else the preserved border pool), plus the game install",
        env_vars: &[
            "BATTLE_PLAN_BORDER_POOL_DIR",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            replay_test_gates::vanilla_game_root().is_some() && border_pool_dir().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_wave1_cap4_no_fow",
        reason: "the CAP4 round corpus with fow_off_observer_front (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved corpus)",
        env_vars: &["BATTLE_PLAN_WAVE1_CAP4_ROUNDS"],
        probe: || wave1_cap4_rounds_dir(&["fow_off_observer_front"]).is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_wave1_cap4_fow_pair",
        reason: "the CAP4 round corpus with fow_on_pov_front and fow_off_observer_front (BATTLE_PLAN_WAVE1_CAP4_ROUNDS, else the preserved corpus)",
        env_vars: &["BATTLE_PLAN_WAVE1_CAP4_ROUNDS"],
        probe: || {
            wave1_cap4_rounds_dir(&["fow_on_pov_front", "fow_off_observer_front"]).is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_restore_gate_1_capture",
        reason: "BATTLE_PLAN_RESTORE_GATE_1_ROOT (or BATTLE_PLAN_CAPTURES_ROOT/restore_gate_1 or C:\\hoi4_evidence\\capture_corpus_preserved\\restore_gate_1) with the captured MainScene and AverageLuminanceTexture payloads",
        env_vars: &["BATTLE_PLAN_RESTORE_GATE_1_ROOT", "BATTLE_PLAN_CAPTURES_ROOT"],
        probe: || {
            let root = restore_gate_1_root();
            replay_test_gates::existing_file(root.join(RESTORE_MAIN_SCENE)).is_some()
                && replay_test_gates::existing_file(root.join(RESTORE_AVERAGE_LUMINANCE)).is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_restore_gate_1_scene",
        reason: "BATTLE_PLAN_RESTORE_GATE_1_ROOT (or BATTLE_PLAN_CAPTURES_ROOT/restore_gate_1 or C:\\hoi4_evidence\\capture_corpus_preserved\\restore_gate_1) with the captured MainScene payload",
        env_vars: &["BATTLE_PLAN_RESTORE_GATE_1_ROOT", "BATTLE_PLAN_CAPTURES_ROOT"],
        probe: || {
            replay_test_gates::existing_file(restore_gate_1_root().join(RESTORE_MAIN_SCENE))
                .is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_map_bundle",
        reason: "the map bundle directory at map_viewer/out/map_bundle",
        env_vars: &[],
        probe: || replay_test_gates::existing_dir(map_bundle_dir()).is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_map_bundle_with_hoi4",
        reason: "the map bundle directory at map_viewer/out/map_bundle plus the game install",
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || {
            replay_test_gates::existing_dir(map_bundle_dir()).is_some()
                && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_replay_fixtures",
        reason: "the replay fixtures directory at replay_interface/fixtures/positive",
        env_vars: &[],
        probe: || replay_test_gates::existing_dir(replay_fixtures_dir()).is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_reflection_cube_proof",
        reason: "REFLECTION_CUBE_PROOF_DIR naming a directory with reflection.dds and cap_c.bin",
        env_vars: &["REFLECTION_CUBE_PROOF_DIR"],
        probe: || reflection_cube_proof_dir().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_river_captures_with_hoi4",
        reason: "river captures (RIVER_CAPTURE_DIRS, else the preserved live_FINAL_full capture), plus the game install",
        env_vars: &[
            "RIVER_CAPTURE_DIRS",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            river_capture_dirs().is_some() && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_shader_capture_with_hoi4",
        reason: "a shader capture (SHADER_CAPTURE_DIR, else the preserved live_FINAL_full capture), plus the game install",
        env_vars: &[
            "SHADER_CAPTURE_DIR",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            shader_capture_dir().is_some() && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_terrain_id_capture_with_hoi4",
        reason: "the captured TerrainIDMap directory (the legacy session_20260702d path, else the preserved gate18_final capture), plus the game install",
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || {
            terrain_id_capture_dir().is_some() && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_terrain_mesh_corpus",
        reason: "a terrain chunk corpus and the game install (TERRAIN_MESH_LAW_CORPUS_CAPTURE plus TERRAIN_MESH_LAW_CORPUS_GAME, else preserved gate18_final plus the default install)",
        env_vars: &[
            "TERRAIN_MESH_LAW_CORPUS_CAPTURE",
            "TERRAIN_MESH_LAW_CORPUS_GAME",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || terrain_mesh_corpus().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_trees2_shader_capture",
        reason: "a trees-pass capture with the tree shader evidence (TREES2_CAPTURE_DIR, else the documented capture directories)",
        env_vars: &["TREES2_CAPTURE_DIR"],
        probe: || trees2_capture_dir().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_trees2_shader_capture_with_hoi4",
        reason: "a trees-pass capture with the tree shader evidence (TREES2_CAPTURE_DIR, else the documented capture directories), plus the game install",
        env_vars: &[
            "TREES2_CAPTURE_DIR",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            trees2_capture_dir().is_some() && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_trees2_template_capture_with_hoi4",
        reason: "a trees-pass capture with the captured tree template buffers (TREES2_CAPTURE_DIR, else the documented capture directories), plus the game install",
        env_vars: &[
            "TREES2_CAPTURE_DIR",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            trees2_template_capture_dir().is_some()
                && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_trees_measure_capture",
        reason: "a trees-measure capture with map_frame_trees_pass_bundle.json (TREES_MEASURE_CAPTURE_DIR, else the documented capture directories)",
        env_vars: &["TREES_MEASURE_CAPTURE_DIR"],
        probe: || trees_measure_capture_dir().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_trees_measure_capture_with_hoi4",
        reason: "a trees-measure capture with map_frame_trees_pass_bundle.json (TREES_MEASURE_CAPTURE_DIR, else the documented capture directories), plus the game install",
        env_vars: &[
            "TREES_MEASURE_CAPTURE_DIR",
            "BATTLE_PLAN_GAME_ROOT",
            "BATTLE_PLAN_GAME_DIR",
            "BATTLE_PLAN_VANILLA",
        ],
        probe: || {
            trees_measure_capture_dir().is_some()
                && replay_test_gates::vanilla_game_root().is_some()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_underwater_evidence_with_hoi4",
        reason: "the captured underwater TerrainDiffuse payload at C:\\Users\\Example\\AppData\\Local\\hoi4_captures\\session_CAPFIX2_GAPS_4, plus the game install",
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || {
            replay_test_gates::existing_file(UNDERWATER_CAPTURE_PAYLOAD).is_some()
                && replay_test_gates::vanilla_game_root().is_some()
        },
    },
];
