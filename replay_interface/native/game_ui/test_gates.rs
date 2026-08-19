pub const GATES: &[replay_test_gates::Gate] = &[
    replay_test_gates::Gate {
        cfg: "gate_hoi4_install",
        reason: replay_test_gates::GAME_ROOT_REASON,
        env_vars: replay_test_gates::GAME_ROOT_ENV_VARS,
        probe: || replay_test_gates::vanilla_game_root().is_some(),
    },
    replay_test_gates::Gate {
        cfg: "gate_hoi4_user_dir",
        reason: replay_test_gates::USER_DIR_REASON,
        env_vars: &[
            replay_test_gates::USER_DIR_ENV,
            "USERPROFILE",
            "OneDrive",
            "OneDriveConsumer",
            "OneDriveCommercial",
        ],
        probe: || replay_test_gates::hoi4_user_dir().is_some(),
    },
];
