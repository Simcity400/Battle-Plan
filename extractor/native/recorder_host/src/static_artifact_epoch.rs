use portable_content_manifest::attach_content_epoch::AttachContentEpoch;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub enum ContentEpochAuthority {
    #[default]
    None,
    Unavailable(String),
    LaunchTime(String),
    AttachTime(AttachContentEpoch),
}

impl ContentEpochAuthority {
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

#[derive(Clone, Debug, Default)]
pub struct StaticArtifactEpochs {
    bindings: BTreeMap<String, Result<String, String>>,
}

impl StaticArtifactEpochs {
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

    pub fn inherited(bindings: BTreeMap<String, Result<String, String>>) -> Self {
        Self { bindings }
    }

    pub fn epoch(&self, name: &str) -> Option<&str> {
        match self.bindings.get(name) {
            Some(Ok(digest)) => Some(digest.as_str()),
            _ => None,
        }
    }

    pub fn refusal(&self, name: &str) -> Option<&str> {
        match self.bindings.get(name) {
            Some(Err(reason)) => Some(reason.as_str()),
            _ => None,
        }
    }

    pub fn refusals(&self) -> impl Iterator<Item = (&str, &str)> {
        self.bindings
            .iter()
            .filter_map(|(name, outcome)| match outcome {
                Err(reason) => Some((name.as_str(), reason.as_str())),
                Ok(_) => None,
            })
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

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

    #[test]
    fn only_the_certified_build_of_a_certified_artifact_is_packaged() {
        let authority = attach_authority(
            vec![
                join("localisation_join", &digest(0x01)),
                join("technology_sharing_static", &digest(0x02)),
            ],
            vec![json!({
                "role": "focus_defs_static",
                "artifact_sha256": digest(0x09),
                "reason": "a claimed path disagreed",
            })],
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
        assert_eq!(
            decided.epoch("localisation_join"),
            Some(digest(0x77).as_str())
        );
        assert_eq!(
            decided.epoch("focus_defs_static"),
            Some(digest(0x77).as_str())
        );
        assert!(decided.warning_lines().is_empty());
    }
}
