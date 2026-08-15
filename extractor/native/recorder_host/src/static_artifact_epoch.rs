//! Which content epoch, if any, binds each static artifact this recording is
//! about to package — decided ONCE, before anything is written.
//!
//! # The defect this closes
//!
//! `replay_packaging.static_artifacts[*].content_manifest_digest_sha256` used
//! to be one value copied onto every row: the digest of the finalized portable
//! content manifest, or JSON `null` when there was none. The shipped
//! `collector_record` can never finalize one (the pre-`ResumeThread` actuator is
//! compiled out), so the field was null on every recording the product can
//! produce, every packaged catalog was declined by the replay consumer, and the
//! refusal surfaced hours later as "recording has no authenticated required
//! static artifact" rather than at the moment the recorder packed bytes it knew
//! nothing could bind.
//!
//! # The rule
//!
//! **A packaged static artifact always carries a real epoch.** There is no
//! third disposition. An artifact the recording cannot bind is not packaged: it
//! is recorded ABSENT with the reason stated in the index, exactly as an
//! artifact the operator never supplied is — because "exclude and account" is
//! what the standing partial-data ruling requires, and because a capture must
//! never be destroyed over content the recorder merely failed to certify. The
//! recorder says so on stderr at the moment it decides, so the refusal is a
//! statement the producer makes, not a discovery the consumer makes.
//!
//! The decision is taken here, once, and both writers — the manifest roster and
//! the container index — read the SAME answer out of [`StaticArtifactEpochs`].
//! Deciding it twice from two resolutions of the same files is how the two
//! rosters drift apart and the consumer's container-role equality check refuses
//! the whole recording.

use portable_content_manifest::attach_content_epoch::AttachContentEpoch;
use std::collections::BTreeMap;

/// The content authority a recording binds its packaged static artifacts to.
#[derive(Clone, Debug, Default)]
pub enum ContentEpochAuthority {
    /// No content proof ran. Nothing may be packaged.
    #[default]
    None,
    /// A content proof RAN but yields no usable epoch — an unsettled attach
    /// window, or a launch-time manifest that finalized no digest. Carried as
    /// its own variant so the artifacts are refused for the reason the proof
    /// gave, not for the generic "no proof" one.
    Unavailable(String),
    /// KIND A — a finalized portable content manifest, observed before
    /// `ResumeThread`. It covers the admitted static bundle as a whole, so its
    /// digest binds every artifact the bundle carries.
    LaunchTime(String),
    /// KIND C — the attach-time effective-VFS join. It certifies artifacts
    /// INDIVIDUALLY, by digest, so each one is asked separately.
    AttachTime(AttachContentEpoch),
}

impl ContentEpochAuthority {
    /// The epoch that binds one artifact, or the named reason it has none.
    pub fn bind(&self, name: &str, artifact_sha256: &str) -> Result<&str, String> {
        match self {
            ContentEpochAuthority::None => Err(
                "this recording carries no content proof, so nothing states which content the \
                 artifact was derived from"
                    .to_string(),
            ),
            ContentEpochAuthority::Unavailable(reason) => Err(reason.clone()),
            ContentEpochAuthority::LaunchTime(digest) => Ok(digest.as_str()),
            ContentEpochAuthority::AttachTime(epoch) => epoch.bind(name, artifact_sha256),
        }
    }
}

/// The settled per-artifact answer, keyed by canonical artifact name.
#[derive(Clone, Debug, Default)]
pub struct StaticArtifactEpochs {
    bindings: BTreeMap<String, Result<String, String>>,
}

impl StaticArtifactEpochs {
    /// Decide every requested artifact against one authority.
    ///
    /// `requested` pairs the canonical artifact name with the sha256 of the
    /// bytes the recording would package — the digest of what actually gets
    /// written, never of what was on the command line at some earlier moment.
    pub fn decide<'a>(
        authority: &ContentEpochAuthority,
        requested: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Self {
        let mut bindings = BTreeMap::new();
        for (name, sha256) in requested {
            let outcome = authority
                .bind(name, sha256)
                .map(str::to_string)
                .map_err(|reason| reason);
            bindings.insert(name.to_string(), outcome);
        }
        Self { bindings }
    }

    /// Carry a previous recording's already-decided bindings forward unchanged.
    ///
    /// Used by fold-and-repack, which republishes a sealed roster rather than
    /// taking a new content observation: re-deciding there would be a second
    /// authority answering a question the capture already answered.
    pub fn inherited(bindings: BTreeMap<String, Result<String, String>>) -> Self {
        Self { bindings }
    }

    /// `Some(epoch)` when this artifact is packaged, `None` when it is not.
    pub fn epoch(&self, name: &str) -> Option<&str> {
        match self.bindings.get(name) {
            Some(Ok(digest)) => Some(digest.as_str()),
            _ => None,
        }
    }

    /// Why a requested artifact is not packaged, if it is not.
    pub fn refusal(&self, name: &str) -> Option<&str> {
        match self.bindings.get(name) {
            Some(Err(reason)) => Some(reason.as_str()),
            _ => None,
        }
    }

    /// Every refusal, in canonical order, for the operator-facing warning.
    pub fn refusals(&self) -> impl Iterator<Item = (&str, &str)> {
        self.bindings.iter().filter_map(|(name, outcome)| match outcome {
            Err(reason) => Some((name.as_str(), reason.as_str())),
            Ok(_) => None,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// One stderr line per refused artifact. Named, never silent.
    pub fn warning_lines(&self) -> Vec<String> {
        self.refusals()
            .map(|(name, reason)| {
                format!(
                    "recorder warning: static artifact {name} is NOT packaged because it is not \
                     content-bound: {reason}"
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use portable_content_manifest::attach_content_epoch::{
        epoch_from_proof_json, layer_stack_digest, ATTACH_PROOF_SCHEMA,
        KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
    };
    use serde_json::{json, Value};

    fn digest(seed: u8) -> String {
        format!("{seed:02x}").repeat(32)
    }

    fn attach_authority(joins: Vec<Value>, refusals: Vec<Value>) -> ContentEpochAuthority {
        let layers = vec![json!({
            "ordinal": 0,
            "kind": "base",
            "layer_id": "base",
            "descriptor_sha256": Value::Null,
            "descriptor_file_sha256": Value::Null,
            "source_container_sha256": Value::Null,
        })];
        let proof = json!({
            "schema": ATTACH_PROOF_SCHEMA,
            "kind": KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
            "window_settled": true,
            "executable_sha256": digest(0xab),
            "executable_size_bytes": 55680632u64,
            "dlc_load_sha256": digest(0xcd),
            "layer_stack": layers.clone(),
            "layer_stack_sha256": layer_stack_digest(&layers).unwrap(),
            "artifact_joins": joins,
            "refused_artifacts": refusals,
            "required_content": [],
            // Written by every proof the recorder seals, and now parsed and
            // bounded by the shared authority, so a fixture that omits it is
            // not a proof this recorder could produce.
            "refused_required_content": [],
        });
        ContentEpochAuthority::AttachTime(epoch_from_proof_json(&proof).unwrap())
    }

    fn join(role: &str, sha: &str) -> Value {
        json!({
            "role": role,
            "artifact_sha256": sha,
            "claimed_paths": 1,
            "joined_bytes": 2,
            "proven_absent_paths": 0,
            "localisation_source_sha256": Value::Null,
        })
    }

    /// Kills: packaging an artifact whose bytes the attach proof never
    /// certified, or certified in a different build.
    #[test]
    fn only_the_certified_build_of_a_certified_artifact_is_packaged() {
        let authority = attach_authority(
            vec![
                join("localisation_join", &digest(0x01)),
                join("technology_sharing_static", &digest(0x02)),
            ],
            vec![json!({"role": "focus_defs_static", "reason": "a claimed path disagreed"})],
        );
        let decided = StaticArtifactEpochs::decide(
            &authority,
            [
                ("localisation_join", digest(0x01).as_str().to_owned()),
                ("technology_sharing_static", digest(0x09).to_owned()),
                ("focus_defs_static", digest(0x03).to_owned()),
                ("ship_definitions_static", digest(0x04).to_owned()),
            ]
            .iter()
            .map(|(name, sha)| (*name, sha.as_str()))
            .collect::<Vec<_>>(),
        );
        assert!(decided.epoch("localisation_join").is_some());
        // Same role, different bytes than the proof read: not packaged.
        assert!(decided.epoch("technology_sharing_static").is_none());
        assert!(decided
            .refusal("technology_sharing_static")
            .unwrap()
            .contains("different build"));
        assert!(decided
            .refusal("focus_defs_static")
            .unwrap()
            .contains("a claimed path disagreed"));
        assert!(decided
            .refusal("ship_definitions_static")
            .unwrap()
            .contains("no join"));
        assert_eq!(decided.warning_lines().len(), 3);
        assert!(decided.warning_lines()[0].contains("is NOT packaged"));
    }

    /// Kills: packaging anything at all on a recording with no content proof.
    #[test]
    fn no_content_proof_packages_nothing() {
        let decided = StaticArtifactEpochs::decide(
            &ContentEpochAuthority::None,
            [("localisation_join", digest(0x01))]
                .iter()
                .map(|(name, sha)| (*name, sha.as_str()))
                .collect::<Vec<_>>(),
        );
        assert!(decided.epoch("localisation_join").is_none());
        assert!(decided
            .refusal("localisation_join")
            .unwrap()
            .contains("no content proof"));
    }

    /// Kills: asking the launch-time authority per artifact. A finalized
    /// portable content manifest covers the admitted bundle as a whole.
    #[test]
    fn the_launch_time_authority_binds_every_packaged_artifact() {
        let authority = ContentEpochAuthority::LaunchTime(digest(0x77));
        let decided = StaticArtifactEpochs::decide(
            &authority,
            [
                ("localisation_join", digest(0x01)),
                ("focus_defs_static", digest(0x02)),
            ]
            .iter()
            .map(|(name, sha)| (*name, sha.as_str()))
            .collect::<Vec<_>>(),
        );
        assert_eq!(decided.epoch("localisation_join"), Some(digest(0x77).as_str()));
        assert_eq!(decided.epoch("focus_defs_static"), Some(digest(0x77).as_str()));
        assert!(decided.warning_lines().is_empty());
    }
}
