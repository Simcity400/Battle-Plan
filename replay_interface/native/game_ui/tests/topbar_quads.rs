//! Engine-quad gate for the topbar layout.
//!
//! Answer key = the engine's OWN GUI draw quads, mined from the vanilla
//! RenderDoc capture `hoi4_vanilla_frame9232.rdc` (1920x1080, gui_scale 1.0)
//! with `replay_interface/renderdoc/mine_ui_quads.py`. Each quad's screen rect
//! is `POSITION * cb0_ortho` where the GUI vertex shader's constant buffer is
//! the orthographic screen->NDC matrix (m00 = 2/1920, m11 = 2/1080, m30/m31 =
//! the per-widget translation supplied via `VSSetConstantBuffers1`
//! FirstConstant). This pins `resolve_element_rect` to engine bytes rather than
//! merely "consistent with topbar.gui".
//!
//! Skipped when the game install is absent.

use game_ui::assets::AssetResolver;
use game_ui::probe::parse_gfx_files;
use game_ui::topbar::{compose_topbar, TopbarBinding};
use std::path::PathBuf;

/// The vanilla install root. Panics rather than returning `None`: every
/// caller is gated on `gate_hoi4_install`, so an absent install here is a
/// failure, never a silent skip that libtest would report as a pass.
fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

/// Engine answer-key quads (screen rect x0,y0,x1,y1), mined from
/// hoi4_vanilla_frame9232.rdc. `background` = GFX_topbar_background_extended
/// (2346x87, ps158, the sole >500px-wide top-of-screen quad); `player_flag` =
/// GFX_shield_medium (82x52). Tolerance is <=1 px per edge.
struct EngineQuad {
    name: &'static str,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

const ENGINE_QUADS: &[EngineQuad] = &[
    EngineQuad {
        name: "background",
        x0: -4.0,
        y0: -1.0,
        x1: 2342.0,
        y1: 86.0,
    },
    EngineQuad {
        name: "player_flag",
        x0: 8.0,
        y0: 14.0,
        x1: 90.0,
        y1: 66.0,
    },
];

const TOL: f32 = 1.0;

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn topbar_element_rects_match_mined_engine_quads() {
    let root = game_root();
    let resolver = AssetResolver::new(vec![root]);
    let (registry, _stats) = parse_gfx_files(&resolver).expect("gfx");
    let result =
        compose_topbar(&resolver, &registry, &TopbarBinding::default()).expect("compose topbar");
    let rects = &result.output.debug_rects;

    let mut failures = Vec::new();
    for eq in ENGINE_QUADS {
        let Some(r) = rects.iter().find(|r| r.name == eq.name) else {
            failures.push(format!("{}: no game_ui rect", eq.name));
            continue;
        };
        let (gx0, gy0) = (r.x as f32, r.y as f32);
        let (gx1, gy1) = ((r.x + r.w) as f32, (r.y + r.h) as f32);
        let d = [
            (gx0 - eq.x0).abs(),
            (gy0 - eq.y0).abs(),
            (gx1 - eq.x1).abs(),
            (gy1 - eq.y1).abs(),
        ];
        eprintln!(
            "{:14} game_ui[{:.0},{:.0},{:.0},{:.0}] engine[{:.0},{:.0},{:.0},{:.0}] \u{394}=[{:.1},{:.1},{:.1},{:.1}]",
            eq.name, gx0, gy0, gx1, gy1, eq.x0, eq.y0, eq.x1, eq.y1, d[0], d[1], d[2], d[3]
        );
        let worst = d.iter().cloned().fold(0.0f32, f32::max);
        if worst > TOL {
            failures.push(format!(
                "{}: max edge delta {:.1}px > {}px",
                eq.name, worst, TOL
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "engine-quad gate failures: {failures:?}"
    );
}
