pub const RESTORE_GATE_1_COLOR_CUBE_REL: &str = concat!(
    "restore_gate_1\\",
    "restore_frame_draw_map_frame_shared_payload_ps_srv_ColorCube_fb9c4f759c5171c9.d3d11"
);

pub fn captures_root() -> std::path::PathBuf {
    std::env::var_os("BATTLE_PLAN_CAPTURES_ROOT").or_else(|| std::env::var_os("HOI4_CAPTURES_ROOT"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\hoi4_evidence\capture_corpus_preserved"))
}

pub const GATES: &[replay_test_gates::Gate] = &[
    replay_test_gates::Gate {
        cfg: "gate_hoi4_install",
        reason: replay_test_gates::GAME_ROOT_REASON,
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || replay_test_gates::vanilla_game_root().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_restore_gate_1_capture",
        reason: "the preserved restore_gate_1 capture corpus (set BATTLE_PLAN_CAPTURES_ROOT to the mount holding it)",
        env_vars: &["BATTLE_PLAN_CAPTURES_ROOT"],
        probe: || {
            captures_root()
                .join(RESTORE_GATE_1_COLOR_CUBE_REL)
                .is_file()
        },
    },
    replay_test_gates::Gate {
        cfg: "gate_xmap_native_evidence_replay",
        reason: "an external-producer-certified replay carrying same-hour MAPC1/D18/FOW1, the passive global _xmap07_snow_mud_runtime texture/fade carrier, XMAP19 weather identity, and nonempty XMAP20 lifecycle authority, plus the matching the game user directory (set XMAP_NATIVE_EVIDENCE_REPLAY and, when needed, BATTLE_PLAN_USER_DIR)",
        env_vars: &["XMAP_NATIVE_EVIDENCE_REPLAY", replay_test_gates::USER_DIR_ENV],
        probe: || {
            std::env::var_os("XMAP_NATIVE_EVIDENCE_REPLAY")
                .map(std::path::PathBuf::from)
                .is_some_and(|path| path.is_file())
                && replay_test_gates::hoi4_user_dir().is_some()
        },
    },
];
