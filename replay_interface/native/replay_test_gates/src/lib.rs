
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

pub const GAME_ROOT_ENV_VARS: &[&str] = &["BATTLE_PLAN_GAME_ROOT", "BATTLE_PLAN_GAME_DIR", "BATTLE_PLAN_VANILLA"];

pub fn vanilla_game_root() -> Option<PathBuf> {
    for var in GAME_ROOT_ENV_VARS {
        if let Some(value) = std::env::var_os(var) {
            let root = PathBuf::from(value);
            return root.is_dir().then_some(root);
        }
    }
    let root = PathBuf::from(DEFAULT_GAME_ROOT);
    root.is_dir().then_some(root)
}

pub const GAME_ROOT_REASON: &str = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path";

pub fn require_vanilla_game_root() -> PathBuf {
    vanilla_game_root().unwrap_or_else(|| {
        panic!(
            "the game install is not resolvable at run time, but this test was compiled as \
             runnable. Checked, in order: {}, then the default {}. {}",
            GAME_ROOT_ENV_VARS.join(", "),
            DEFAULT_GAME_ROOT,
            "Re-run `cargo test` so the build script re-probes, or set the variable."
        )
    })
}

pub const USER_DIR_ENV: &str = "BATTLE_PLAN_USER_DIR";

const USER_DIR_REL: &str = r"Paradox Interactive\Hearts of Iron IV";

pub const USER_DIR_REASON: &str = "the HOI4 user directory (set BATTLE_PLAN_USER_DIR to it, or let it be found under your Documents folder as `Paradox Interactive\\Hearts of Iron IV`)";

fn documents_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let mut push = |base: Option<std::ffi::OsString>| {
        if let Some(base) = base {
            roots.push(PathBuf::from(base).join("Documents"));
        }
    };
    push(std::env::var_os("USERPROFILE"));
    push(std::env::var_os("OneDrive"));
    push(std::env::var_os("OneDriveConsumer"));
    push(std::env::var_os("OneDriveCommercial"));
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(profile).join("OneDrive").join("Documents"));
    }
    roots
}

pub fn hoi4_user_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(USER_DIR_ENV) {
        let dir = PathBuf::from(value);
        return dir.is_dir().then_some(dir);
    }
    documents_roots()
        .into_iter()
        .map(|root| root.join(USER_DIR_REL))
        .find(|dir| dir.is_dir())
}

pub fn require_hoi4_user_dir() -> PathBuf {
    hoi4_user_dir().unwrap_or_else(|| {
        panic!(
            "the game user directory is not resolvable at run time, but this test was compiled \
             as runnable. Set {USER_DIR_ENV}, or place it under one of {:?} as {USER_DIR_REL}.",
            documents_roots()
        )
    })
}

pub fn dir_from_env(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    let path = PathBuf::from(value);
    path.is_dir().then_some(path)
}

pub fn require_dir_from_env(var: &str) -> PathBuf {
    dir_from_env(var).unwrap_or_else(|| {
        panic!(
            "{var} does not name an existing directory, but this test was compiled as runnable \
             (value: {:?})",
            std::env::var_os(var)
        )
    })
}

pub fn existing_dir(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    path.is_dir().then(|| path.to_path_buf())
}

pub fn require_existing_dir(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    existing_dir(path).unwrap_or_else(|| {
        panic!(
            "{} is not an existing directory, but this test was compiled as runnable",
            path.display()
        )
    })
}

pub fn existing_file(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    path.is_file().then(|| path.to_path_buf())
}

pub fn require_existing_file(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    existing_file(path).unwrap_or_else(|| {
        panic!(
            "{} is not an existing file, but this test was compiled as runnable",
            path.display()
        )
    })
}

pub struct Gate {
    pub cfg: &'static str,
    pub reason: &'static str,
    pub env_vars: &'static [&'static str],
    pub probe: fn() -> bool,
}

pub fn emit(gates: &[Gate]) {
    let mut seen_cfg = Vec::new();
    for gate in gates {
        assert!(
            gate.cfg.starts_with("gate_"),
            "gate cfg name {} must start with `gate_`",
            gate.cfg
        );
        assert!(
            !seen_cfg.contains(&gate.cfg),
            "duplicate gate cfg name {}",
            gate.cfg
        );
        seen_cfg.push(gate.cfg);

        println!("cargo::rustc-check-cfg=cfg({})", gate.cfg);
        for var in gate.env_vars {
            println!("cargo::rerun-if-env-changed={var}");
        }
        if (gate.probe)() {
            println!("cargo::rustc-cfg={}", gate.cfg);
        } else {
            println!("cargo::warning=test gate {} is UNMET: requires {}", gate.cfg, gate.reason);
        }
    }
}

pub fn assert_gates_are_current(gates: &[Gate], compiled: &dyn Fn(&str) -> bool) {
    let mut drifted = String::new();
    for gate in gates {
        let actual = (gate.probe)();
        let built = compiled(gate.cfg);
        if actual != built {
            let _ = writeln!(
                drifted,
                "  {}: compiled cfg = {built}, precondition now = {actual} (requires {})",
                gate.cfg, gate.reason
            );
        }
    }
    assert!(
        drifted.is_empty(),
        "test gates are stale -- the build script probed a different environment than this run \
         sees, so tests are being ignored or run against the wrong assumption. Re-run the build \
         script (e.g. `cargo clean -p <crate>`, or change one of the gate variables) and try \
         again.\n{drifted}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_but_invalid_install_variable_does_not_fall_through() {
        let ordered = GAME_ROOT_ENV_VARS;
        assert_eq!(ordered, &["BATTLE_PLAN_GAME_ROOT", "BATTLE_PLAN_GAME_DIR", "BATTLE_PLAN_VANILLA"]);
    }

    #[test]
    fn existing_dir_and_file_discriminate() {
        let working_dir = std::env::current_dir().expect("test working directory");
        assert!(existing_dir(&working_dir).is_some());
        assert!(existing_file(&working_dir).is_none());

        let current_exe = std::env::current_exe().expect("current test executable");
        assert!(existing_file(&current_exe).is_some());
        assert!(existing_dir(&current_exe).is_none());
        assert!(existing_dir(current_exe.join("no-such-directory")).is_none());
    }

    #[test]
    #[should_panic(expected = "is not an existing directory")]
    fn require_existing_dir_panics_rather_than_skipping() {
        let _ = require_existing_dir(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("no-such-directory"),
        );
    }

    #[test]
    fn assert_gates_are_current_reports_both_drift_directions() {
        fn yes() -> bool {
            true
        }
        fn no() -> bool {
            false
        }
        let gates = [
            Gate {
                cfg: "gate_should_be_on",
                reason: "a thing that is present",
                env_vars: &[],
                probe: yes,
            },
            Gate {
                cfg: "gate_should_be_off",
                reason: "a thing that is absent",
                env_vars: &[],
                probe: no,
            },
        ];
        assert_gates_are_current(&gates, &|cfg| cfg == "gate_should_be_on");

        let stale_off = std::panic::catch_unwind(|| {
            assert_gates_are_current(&gates, &|_| false);
        });
        assert!(stale_off.is_err(), "gate off + precondition present must fail");

        let stale_on = std::panic::catch_unwind(|| {
            assert_gates_are_current(&gates, &|_| true);
        });
        assert!(stale_on.is_err(), "gate on + precondition absent must fail");
    }
}
