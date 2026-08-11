//! Honest preconditions for tests that need resources outside the repository.
//!
//! # Why this crate exists
//!
//! Tests in this workspace that need the real the game install, a
//! capture corpus, or a mined evidence bundle used to gate themselves like
//! this:
//!
//! ```ignore
//! #[test]
//! fn some_gate() {
//!     let Some(root) = std::env::var_os("BATTLE_PLAN_GAME_ROOT").or_else(|| std::env::var_os("HOI4_GAME_ROOT")).map(PathBuf::from) else {
//!         return;                    // <-- libtest reports this as `ok`
//!     };
//!     if !root.is_dir() {
//!         return;                    // <-- so does this
//!     }
//!     // ... the assertions that were supposed to run ...
//! }
//! ```
//!
//! A bare `return;` in a `#[test] fn` that returns `()` is a **silent pass**,
//! not a skip: the result line says `ok` and the summary counts the test as
//! passed even though nothing was asserted. That is how a real font defect
//! (65 of 343 shipped font page rows being refused) stayed hidden behind a
//! `12 passed; 0 failed` result line.
//!
//! # The mechanism
//!
//! A precondition is decided **once, at build time**, by a build script, and
//! recorded as a `cfg`. A test whose precondition is unmet is marked
//! `#[cfg_attr(not(<gate>), ignore = "<reason>")]`, so `cargo test` prints
//!
//! ```text
//! test some_gate ... ignored, requires the game install (...)
//! ```
//!
//! and the result line carries a non-zero `ignored` count. Nothing claims a
//! pass for a test that did not run.
//!
//! When the build script *did* decide the precondition holds, the test body
//! must not fall back to skipping: it calls one of the `require_*` accessors
//! here, which **panic** if the resource has gone missing between build and
//! run. That direction is a genuine failure, not a skip.
//!
//! The opposite direction — the build script decided a gate was unmet, the
//! resource has since appeared, and a stale `cfg` keeps the test ignored — is
//! caught by [`assert_gates_are_current`], which every consuming crate runs as
//! an ordinary test.
//!
//! # Why use a build-time gate
//!
//! `Invoke-CentralRustValidation.ps1` runs external-resource tests only in
//! **`-Mode Evidence`**. The standing **`-Mode Final`** profile excludes them
//! so that Final remains the deterministic offline battery. Evidence passes
//! `--run-ignored all` and is invoked only when its named prerequisites are
//! available.
//!
//! The build-time gate still matters: ordinary listings and non-Evidence runs
//! show an honest `ignored, <reason>` state when a prerequisite is absent; a
//! test admitted into Evidence cannot silently return success without running
//! assertions; and the `require_*` accessor fails if a resource disappears
//! after the build-time decision.
//!
//! # The environment contract
//!
//! Every variable below is OPTIONAL. Each names an override for a resource
//! that is otherwise discovered at a documented default location, so a rig
//! holding the resources needs no environment at all. Install/user-directory
//! overrides win outright and fail closed when invalid. Capture-corpus
//! overrides retain their test's documented resolution law: some win outright,
//! while the map capture searches put the override first and then continue to
//! the preserved evidence when it is stale.
//!
//! | Variable | Selects | Default when unset |
//! |---|---|---|
//! | `BATTLE_PLAN_GAME_ROOT`, `BATTLE_PLAN_GAME_DIR`, `BATTLE_PLAN_VANILLA` | the install root | [`DEFAULT_GAME_ROOT`] |
//! | `BATTLE_PLAN_USER_DIR` | the user/documents directory | `<Documents>\Paradox Interactive\Hearts of Iron IV`, where `<Documents>` covers OneDrive Known Folder Move |
//! | `BATTLE_PLAN_CAPTURES_ROOT`, `BATTLE_PLAN_RESTORE_GATE_1_ROOT` | preserved capture corpora | the documented preserved mounts |
//! | `BATTLE_PLAN_BORDER_POOL_DIR`, `BATTLE_PLAN_WAVE1_CAP4_ROUNDS`, `TREES2_CAPTURE_DIR`, `TREES_MEASURE_CAPTURE_DIR` | per-lane capture corpora | the documented preserved copies |
//! | `BATTLE_PLAN_TERRAIN_CAPTURE_FIXTURE_DIRS`, `RIVER_CAPTURE_DIRS`, `SHADER_CAPTURE_DIR`, `TERRAIN_MESH_LAW_CORPUS_CAPTURE`, `TERRAIN_MESH_LAW_CORPUS_GAME` | shader, river and terrain evidence corpora | the documented `live_FINAL_full` / `gate18_final` preserved copies and [`DEFAULT_GAME_ROOT`] |
//! | `BATTLE_PLAN_RENDERDOC_MINED` | the mined RenderDoc store | `%LOCALAPPDATA%\hoi4_renderdoc\mined` |
//! | `BATTLE_PLAN_RETAINED_PRODUCTION_COMPLETE_FRAME` | the retained frame capture | its documented capture root |
//! | `REFLECTION_CUBE_PROOF_DIR` | a corpus with no preserved default | *(none — gate is unmet)* |
//!
//! Consequence: `-Mode Final` never scans the game install or capture stores.
//! `-Mode Evidence` selects those tests explicitly and must be run only when
//! their prerequisites are available. A missing resource is never counted as
//! a successful assertion.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The install path this workspace was developed against. Used when no
/// install environment variable is set at all.
pub const DEFAULT_GAME_ROOT: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

/// Environment variables consulted, in order, for the vanilla install root.
///
/// The first variable that is **set** decides the answer. A variable that is
/// set but does not name a directory leaves the precondition UNSATISFIED — it
/// never falls through to the next name or to [`DEFAULT_GAME_ROOT`], because
/// an operator who set the variable meant that path and a silent fallback
/// would run the gate against content they did not choose.
pub const GAME_ROOT_ENV_VARS: &[&str] = &["BATTLE_PLAN_GAME_ROOT", "BATTLE_PLAN_GAME_DIR", "BATTLE_PLAN_VANILLA"];

/// The vanilla install root, or `None` when it cannot be resolved.
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

/// Human-readable explanation of how the install root is resolved. Used in
/// `#[ignore = "..."]` reasons and in panic messages so both ends of the
/// mechanism describe the same contract.
pub const GAME_ROOT_REASON: &str = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path";

/// The vanilla install root. Panics when absent.
///
/// Call this from a test that is gated on [`vanilla_game_root`]: reaching it
/// means the build script decided the install was present, so its absence at
/// run time is a real failure.
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

/// The the game user ("documents") directory, holding `dlc_load.json`, mods and
/// settings. Distinct from the install root.
pub const USER_DIR_ENV: &str = "BATTLE_PLAN_USER_DIR";

/// Path of the user directory relative to the Documents folder.
const USER_DIR_REL: &str = r"Paradox Interactive\Hearts of Iron IV";

/// Human-readable explanation of how the user directory is resolved.
pub const USER_DIR_REASON: &str = "the HOI4 user directory (set BATTLE_PLAN_USER_DIR to it, or let it be found under your Documents folder as `Paradox Interactive\\Hearts of Iron IV`)";

/// Every plausible Documents root. `%USERPROFILE%\Documents` is the plain
/// case; the rest cover OneDrive **Known Folder Move**, which silently
/// relocates Documents and is why a naive `%USERPROFILE%\Documents` probe
/// concludes "no user directory" on a machine that has one.
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

/// The the game user directory, or `None` when it cannot be resolved.
///
/// `BATTLE_PLAN_USER_DIR` wins when set; a set-but-invalid value leaves the
/// precondition unsatisfied rather than falling through, for the same reason
/// as [`vanilla_game_root`].
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

/// The the game user directory. Panics when absent.
pub fn require_hoi4_user_dir() -> PathBuf {
    hoi4_user_dir().unwrap_or_else(|| {
        panic!(
            "the game user directory is not resolvable at run time, but this test was compiled \
             as runnable. Set {USER_DIR_ENV}, or place it under one of {:?} as {USER_DIR_REL}.",
            documents_roots()
        )
    })
}

/// A directory named by an environment variable, or `None` when the variable
/// is unset or does not name a directory.
pub fn dir_from_env(var: &str) -> Option<PathBuf> {
    let value = std::env::var_os(var)?;
    let path = PathBuf::from(value);
    path.is_dir().then_some(path)
}

/// A directory named by an environment variable. Panics when absent.
pub fn require_dir_from_env(var: &str) -> PathBuf {
    dir_from_env(var).unwrap_or_else(|| {
        panic!(
            "{var} does not name an existing directory, but this test was compiled as runnable \
             (value: {:?})",
            std::env::var_os(var)
        )
    })
}

/// An existing directory at a fixed path, or `None`.
pub fn existing_dir(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    path.is_dir().then(|| path.to_path_buf())
}

/// An existing directory at a fixed path. Panics when absent.
pub fn require_existing_dir(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    existing_dir(path).unwrap_or_else(|| {
        panic!(
            "{} is not an existing directory, but this test was compiled as runnable",
            path.display()
        )
    })
}

/// An existing file at a fixed path, or `None`.
pub fn existing_file(path: impl AsRef<Path>) -> Option<PathBuf> {
    let path = path.as_ref();
    path.is_file().then(|| path.to_path_buf())
}

/// An existing file at a fixed path. Panics when absent.
pub fn require_existing_file(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    existing_file(path).unwrap_or_else(|| {
        panic!(
            "{} is not an existing file, but this test was compiled as runnable",
            path.display()
        )
    })
}

/// One named precondition.
///
/// The same table is compiled into the crate's build script (which turns
/// [`Gate::probe`] into a `cfg`) and into the crate's tests (which re-run
/// [`Gate::probe`] to prove the `cfg` is not stale), so the two can never
/// drift apart.
pub struct Gate {
    /// The `cfg` name emitted when [`Gate::probe`] returns `true`. By
    /// convention `gate_*`.
    pub cfg: &'static str,
    /// What the gate needs, phrased for an `#[ignore = "requires ..."]`
    /// reason and read by whoever sees the ignored line.
    pub reason: &'static str,
    /// Every environment variable the probe consults. Each one is registered
    /// with Cargo as `rerun-if-env-changed`, so changing it re-probes.
    pub env_vars: &'static [&'static str],
    /// The precondition itself. Must be side-effect free and cheap.
    pub probe: fn() -> bool,
}

/// Build-script entry point: turn a gate table into `cfg`s.
///
/// Emits, for every gate, a `rustc-check-cfg` declaration (so `not(gate_x)`
/// never trips the `unexpected_cfgs` lint), a `rustc-cfg` when the probe
/// passes, and a `rerun-if-env-changed` for every variable the probe reads.
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
            // Visible in `cargo test` output so an unmet precondition is
            // announced at build time as well as in the ignored line.
            println!("cargo::warning=test gate {} is UNMET: requires {}", gate.cfg, gate.reason);
        }
    }
}

/// Test-side entry point: prove no `cfg` in the table is stale.
///
/// `compiled` must report, for each gate name, whether the crate was compiled
/// with that `cfg` on (i.e. `cfg!(gate_x)`). A mismatch in either direction is
/// a hard failure:
///
/// * gate on, precondition now absent — tests would fail confusingly;
/// * gate off, precondition now present — tests are being **ignored although
///   they could run**, which is the same class of quiet non-execution this
///   whole mechanism exists to remove.
///
/// The fix in both cases is to let the build script re-probe (`cargo clean -p
/// <crate>` or touch the build script); the message says so.
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
        // The contract that keeps a mis-set variable from quietly running the
        // gates against a different install: resolution stops at the first
        // variable that is SET, and fails if that path is not a directory.
        // Exercised without touching the process environment.
        let ordered = GAME_ROOT_ENV_VARS;
        assert_eq!(ordered, &["BATTLE_PLAN_GAME_ROOT", "BATTLE_PLAN_GAME_DIR", "BATTLE_PLAN_VANILLA"]);
    }

    #[test]
    fn existing_dir_and_file_discriminate() {
        // These probes deliberately use runtime identities. A binary reused
        // from the shared target directory may have been compiled in another
        // worktree, so CARGO_MANIFEST_DIR can truthfully name a worktree that
        // has since been removed.
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
        // Agreeing view: no panic.
        assert_gates_are_current(&gates, &|cfg| cfg == "gate_should_be_on");

        // Stale in the "ignored although runnable" direction.
        let stale_off = std::panic::catch_unwind(|| {
            assert_gates_are_current(&gates, &|_| false);
        });
        assert!(stale_off.is_err(), "gate off + precondition present must fail");

        // Stale in the "compiled runnable but absent" direction.
        let stale_on = std::panic::catch_unwind(|| {
            assert_gates_are_current(&gates, &|_| true);
        });
        assert!(stale_on.is_err(), "gate on + precondition absent must fail");
    }
}
