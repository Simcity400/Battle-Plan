//! BUNDLE selection + replay mod auto-match (Stage 1 deliverable 7).
//!
//! Wraps `assets::BundleConfig` (resolve the active asset bundle via `--bundle` /
//! `REPLAY_BUNDLE` / default `game_assets`) and `assets::match_replay` (auto-match the
//! replay's recorded mod-set against the bundle's) into ONE app-facing summary the UI
//! surfaces: which bundle is loaded, what mod-set it is, and whether it matches the
//! replay — with a clear, path-free warning on a MISMATCH (or an advisory when the
//! replay didn't record its mod identity, which is the current snapshot reality).
//!
//! No fabrication from absence: a replay with no `provenance.mod` yields
//! [`assets::MatchOutcome::ReplayUnknown`] (an advisory), NEVER a false "all clear".

use assets::{match_replay, BundleConfig, MatchOutcome, ModError, ModSet};
use snapshot::Snapshot;

/// The resolved bundle + the replay-match verdict, ready for the UI.
pub struct Provenance {
    /// The active asset bundle (its dir + its mod-set identity).
    pub bundle: BundleConfig,
    /// The replay's recorded mod-set, parsed from `provenance.mod` (`None` = the
    /// replay did not record one — UNKNOWN, never fabricated).
    pub replay_modset: Option<ModSet>,
    /// The auto-match outcome (Match / Mismatch{deltas} / ReplayUnknown).
    pub outcome: MatchOutcome,
}

impl Provenance {
    /// Resolve the bundle (precedence: explicit `--bundle` → `REPLAY_BUNDLE` →
    /// `default_bundle_dir`) and auto-match it against the replay's recorded mod-set,
    /// read from the FIRST snapshot's `provenance.mod`.
    pub fn resolve(
        explicit_bundle: Option<&str>,
        default_bundle_dir: impl Into<std::path::PathBuf>,
        first_snapshot: &Snapshot,
    ) -> Result<Provenance, ModError> {
        let bundle = BundleConfig::resolve(explicit_bundle, default_bundle_dir)?;
        let replay_modset = replay_modset_of(first_snapshot);
        let outcome = match_replay(replay_modset.as_ref(), &bundle.modset);
        Ok(Provenance {
            bundle,
            replay_modset,
            outcome,
        })
    }

    /// A one-line provenance banner for the title bar / a log line.
    pub fn banner(&self) -> String {
        let bundle_id = self.bundle.modset.resolved_id();
        let manifest = if self.bundle.manifest_present {
            "manifest"
        } else {
            "no-manifest→vanilla"
        };
        format!(
            "bundle={} modset={} ({}) match={}",
            self.bundle.bundle_dir.display(),
            bundle_id,
            manifest,
            self.match_word(),
        )
    }

    fn match_word(&self) -> &'static str {
        match self.outcome {
            MatchOutcome::Match => "ok",
            MatchOutcome::Mismatch { .. } => "MISMATCH",
            MatchOutcome::ReplayUnknown => "unknown",
        }
    }

    /// The user-facing warning, if any (a MISMATCH or an UNKNOWN advisory). `None`
    /// when the bundle matches the replay (nothing to warn about).
    pub fn warning(&self) -> Option<String> {
        self.outcome.warning()
    }

    /// True iff a visible MISMATCH warning should be raised (the user's explicit
    /// concern: vanilla UI over a mod replay). `ReplayUnknown` is an advisory, not a
    /// mismatch.
    pub fn is_mismatch(&self) -> bool {
        self.outcome.is_mismatch()
    }
}

/// Parse a snapshot's `provenance.mod` into a [`ModSet`] (None = unknown).
pub fn replay_modset_of(snap: &Snapshot) -> Option<ModSet> {
    let prov = snap.domain("provenance")?;
    let modv = prov.as_object()?.get("mod");
    ModSet::from_provenance(modv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn snap_with(provenance: serde_json::Value) -> Snapshot {
        serde_json::from_value(json!({
            "game_id":"g","date":"1940.1.1.12","hour_index":100,
            "provenance": provenance,
            "states":{"1":{"owner":"GER"}}
        }))
        .unwrap()
    }

    fn tmp_bundle(manifest: Option<serde_json::Value>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "app_prov_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        if let Some(m) = manifest {
            fs::write(
                dir.join(BundleConfig::MANIFEST_NAME),
                serde_json::to_string(&m).unwrap(),
            )
            .unwrap();
        }
        dir
    }

    /// The current snapshot reality: `provenance` present but NO `mod` field → the
    /// replay's mod identity is UNKNOWN → an advisory, never a false all-clear.
    #[test]
    fn replay_without_mod_field_is_unknown() {
        let snap = snap_with(json!({"memory": null, "save": {"captured": true}}));
        assert!(
            replay_modset_of(&snap).is_none(),
            "no provenance.mod -> unknown"
        );

        let bundle_dir = tmp_bundle(None); // vanilla bundle (no manifest)
        let p =
            Provenance::resolve(Some(bundle_dir.to_str().unwrap()), &bundle_dir, &snap).unwrap();
        assert!(matches!(p.outcome, MatchOutcome::ReplayUnknown));
        assert!(!p.is_mismatch(), "unknown is an advisory, not a mismatch");
        assert!(p.warning().is_some(), "unknown must still advise the user");
        let _ = fs::remove_dir_all(&bundle_dir);
    }

    /// A replay that recorded a MOD-set, against a VANILLA bundle → MISMATCH with an
    /// explained delta (the vanilla-UI-over-a-mod-replay case the user cares about).
    #[test]
    fn mod_replay_over_vanilla_bundle_is_mismatch() {
        let replay_mod = json!({
            "modset_id": "",
            "load_order": ["amsb"],
            "mods": [{"name":"AMSB","workshop_id":"123","version":"1.18"}]
        });
        let snap = snap_with(json!({"mod": replay_mod}));
        assert!(
            replay_modset_of(&snap).is_some(),
            "a recorded mod-set parses"
        );

        let bundle_dir = tmp_bundle(None); // vanilla bundle
        let p =
            Provenance::resolve(Some(bundle_dir.to_str().unwrap()), &bundle_dir, &snap).unwrap();
        assert!(
            p.is_mismatch(),
            "mod replay over a vanilla bundle must warn"
        );
        let w = p.warning().expect("a mismatch must warn");
        assert!(w.contains("MISMATCH"));
        let _ = fs::remove_dir_all(&bundle_dir);
    }

    /// A bundle whose manifest matches the replay's recorded mod-set → MATCH, no
    /// warning.
    #[test]
    fn matching_modset_is_clean() {
        let mods = json!([{"name":"AMSB","workshop_id":"123","version":"1.18"}]);
        let replay_mod = json!({"modset_id":"","load_order":["amsb"],"mods": mods});
        let snap = snap_with(json!({"mod": replay_mod.clone()}));
        // the bundle's manifest is the SAME mod-set.
        let bundle_dir = tmp_bundle(Some(replay_mod));
        let p =
            Provenance::resolve(Some(bundle_dir.to_str().unwrap()), &bundle_dir, &snap).unwrap();
        assert!(
            p.outcome.is_match(),
            "identical mod-sets must match: {:?}",
            p.outcome
        );
        assert!(p.warning().is_none());
        assert!(p.bundle.manifest_present);
        let _ = fs::remove_dir_all(&bundle_dir);
    }
}
