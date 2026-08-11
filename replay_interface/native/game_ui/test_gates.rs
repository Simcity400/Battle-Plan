// Gate table for `game_ui`. Included by BOTH `build.rs` (which turns each
// probe into a `cfg`) and `tests/gate_currency.rs` (which re-runs each probe
// to prove the compiled `cfg` still matches reality), so the build-time and
// run-time views of a precondition cannot drift apart.
//
// Every `#[test]` in this crate that needs one of these preconditions carries
// `#[cfg_attr(not(<cfg>), ignore = "requires ...")]` and calls the matching
// `replay_test_gates::require_*` accessor in its body. It must never `return`
// early on a missing precondition: libtest reports that as a pass.
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
