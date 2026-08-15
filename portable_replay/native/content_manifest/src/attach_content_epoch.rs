//! The KIND C content epoch — ONE authority, read by the recorder that stamps
//! it onto packaged static artifacts and by the replay consumer that admits
//! them.
//!
//! # What this closes
//!
//! A packaged static artifact is only usable if something states WHICH content
//! corpus it was derived from; otherwise a consumer is joining a catalog to
//! bytes nothing proves it came from. Until now the only such statement was
//! `portable_content_manifest.manifest_digest_sha256`, which is built from an
//! effective-VFS observation taken before `ResumeThread`. The shipped
//! `collector_record` cannot take one — the client-driving actuator is compiled
//! out — so `replay_packaging.static_artifacts[*].content_manifest_digest_sha256`
//! came out NULL on every recording the product can produce, and every packaged
//! catalog was declined by the consumer.
//!
//! The recorder already takes a strictly stronger observation on exactly that
//! route: the KIND C attach-time effective-VFS join
//! (`recorder_host::attach_content_proof`), which resolves every path the
//! packaged artifacts claim through the engine's own effective VFS and requires
//! the winning layer's bytes to hash to the declared digest. This module turns
//! that proof into the epoch the packaging contract needs.
//!
//! # Why the derivation lives here and not in the recorder
//!
//! Harness-authority drift: a value one component computes and another
//! re-derives drifts silently. The recorder calls
//! [`epoch_from_proof_json`] on the block it is about to seal, and the replay
//! consumer calls the same function on the block it read out of the sealed
//! manifest. There is no second spelling of the canonicalization.
//!
//! # What the digest covers, and what it deliberately does not
//!
//! Covered: the executable identity, the `dlc_load.json` identity, the ordered
//! layer stack, every certified artifact join (role + the artifact's own
//! digest + its claim counts), and every joined required-content row. These are
//! the facts that say *which content world this is*.
//!
//! Not covered: `observed_at_unix`, `completed_at_unix`, `elapsed_ms`,
//! `hashed_bytes`, `hashed_files`, `window_attempts`. Those are the COST and
//! the WINDOW of the observation, not its content. Two recordings taken minutes
//! apart against one unchanged install therefore carry the SAME epoch, which is
//! what makes it an epoch rather than a per-recording nonce.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Schema of the recorder's attach-time content-proof block.
pub const ATTACH_PROOF_SCHEMA: &str = "hoi4.attach-time-content-proof.v1";

/// Versioned name of the canonicalization this module hashes. Part of the
/// hashed document, so a future canonicalization can never collide with this
/// one.
pub const EPOCH_SCHEMA: &str = "hoi4.attach-time-content-epoch.v1";

/// Manifest field naming WHICH content proof ran.
pub const CONTENT_PROOF_KIND_FIELD: &str = "content_proof_kind";

/// Manifest field carrying the KIND C proof block itself.
pub const ATTACH_TIME_CONTENT_PROOF_FIELD: &str = "attach_time_content_proof";

/// The launch-time epoch: a content manifest observed before `ResumeThread`.
pub const KIND_LAUNCH_TIME_STATIC_BUNDLE_V1: &str = "launch_time_static_bundle_v1";

/// The attach-time effective-VFS join.
pub const KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1: &str = "attach_time_effective_vfs_join_v1";

/// Neither proof ran.
pub const KIND_NONE: &str = "none";

/// Ceiling on rows read out of one proof block, so a malformed manifest cannot
/// turn admission into unbounded work.
const MAX_PROOF_ROWS: usize = 1 << 16;

/// The closed set of content-layer kinds a proof may name.
///
/// Spelled here rather than accepting any non-empty string: the recorder's own
/// wire reader refuses an unknown kind, so accepting one here would let a layer
/// stack the producer could never have written hash to a valid epoch (Codex
/// round 1, LOW). These are the four `ContentLayerKind` wire names.
const LAYER_KINDS: [&str; 4] = ["base", "integrated_dlc", "dlc", "mod"];

/// A KIND C epoch, plus the per-role joins that authorize an artifact to carry
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachContentEpoch {
    digest: String,
    /// role -> the digest of the artifact FILE as the proof read it at attach.
    joined: BTreeMap<String, String>,
    /// role -> why the proof refused to certify it.
    refused: BTreeMap<String, String>,
}

impl AttachContentEpoch {
    /// The epoch every packaged artifact this proof certifies must carry.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Roles the proof certified, in canonical order.
    pub fn joined_roles(&self) -> impl Iterator<Item = &str> {
        self.joined.keys().map(String::as_str)
    }

    /// Roles the proof explicitly refused, with the reason it gave.
    pub fn refusals(&self) -> impl Iterator<Item = (&str, &str)> {
        self.refused
            .iter()
            .map(|(role, reason)| (role.as_str(), reason.as_str()))
    }

    /// Decide whether one packaged artifact may carry this epoch.
    ///
    /// `artifact_sha256` is the digest of the bytes the package actually
    /// carries. It must equal the digest the proof read at attach, so a
    /// receipt can never be read as covering some other build of the artifact.
    pub fn bind(&self, role: &str, artifact_sha256: &str) -> Result<&str, String> {
        if let Some(reason) = self.refused.get(role) {
            return Err(format!(
                "attach-time content proof refused this artifact: {reason}"
            ));
        }
        match self.joined.get(role) {
            None => Err(
                "attach-time content proof carries no join for this artifact".to_string(),
            ),
            Some(proven) if proven == artifact_sha256 => Ok(self.digest.as_str()),
            Some(proven) => Err(format!(
                "attach-time content proof joined a different build of this artifact \
                 (proof {proven}, package {artifact_sha256})"
            )),
        }
    }
}

/// Read the manifest-level `content_proof_kind` declaration.
///
/// Positively declared or refused: a consumer never infers the proof kind from
/// which other fields happen to be present. A manifest with no declaration at
/// all is legacy and answers `None`.
pub fn content_proof_kind(manifest: &Map<String, Value>) -> Result<Option<&str>, String> {
    match manifest.get(CONTENT_PROOF_KIND_FIELD) {
        None => Ok(None),
        Some(Value::String(value))
            if value == KIND_NONE
                || value == KIND_LAUNCH_TIME_STATIC_BUNDLE_V1
                || value == KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1 =>
        {
            Ok(Some(value.as_str()))
        }
        Some(other) => Err(format!(
            "manifest {CONTENT_PROOF_KIND_FIELD} is not a recognised content-proof kind: {other}"
        )),
    }
}

/// Derive the epoch from a sealed (or about-to-be-sealed) KIND C proof block.
///
/// Every refusal is named. An unsettled window is refused here rather than
/// producing a weaker epoch: the whole point of the window bracket is that
/// nothing observed inside an unsettled one describes one install, and the
/// recorder already names every artifact as unproven in that case.
pub fn epoch_from_proof_json(proof: &Value) -> Result<AttachContentEpoch, String> {
    let proof = proof
        .as_object()
        .ok_or_else(|| "attach-time content proof is not an object".to_string())?;
    require_str(proof, "schema", ATTACH_PROOF_SCHEMA)?;
    require_str(proof, "kind", KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1)?;
    match proof.get("window_settled") {
        Some(Value::Bool(true)) => {}
        Some(Value::Bool(false)) => {
            return Err(
                "attach-time content proof window never settled; it describes no one install"
                    .to_string(),
            )
        }
        _ => return Err("attach-time content proof window_settled is not a bool".to_string()),
    }
    let executable_sha256 = read_digest(proof, "executable_sha256")?;
    let dlc_load_sha256 = read_digest(proof, "dlc_load_sha256")?;
    let layer_stack_sha256 = read_digest(proof, "layer_stack_sha256")?;
    let executable_size_bytes = read_u64(proof, "executable_size_bytes")?;
    let layer_stack = read_rows(proof, "layer_stack")?;
    if layer_stack.is_empty() {
        return Err("attach-time content proof has an empty layer stack".to_string());
    }
    // RE-DERIVE THE LAYER-STACK DIGEST rather than trusting the declared one.
    // Load order decides which layer wins a path, so a reordered stack is a
    // different world; a consumer that copied the declared value would inherit
    // whatever the producer chose to write.
    let recomputed = layer_stack_digest(&layer_stack)?;
    if recomputed != layer_stack_sha256 {
        return Err(format!(
            "attach-time content proof layer_stack_sha256 {layer_stack_sha256} does not match its \
             own layer stack (recomputed {recomputed})"
        ));
    }

    let mut joined = BTreeMap::new();
    let mut join_rows = Vec::new();
    for row in read_rows(proof, "artifact_joins")? {
        let row = row
            .as_object()
            .ok_or_else(|| "attach-time content proof artifact join is not an object".to_string())?;
        let role = read_non_empty(row, "role")?;
        let artifact_sha256 = read_digest(row, "artifact_sha256")?;
        let canonical = json!({
            "role": role,
            "artifact_sha256": artifact_sha256,
            "claimed_paths": read_u64(row, "claimed_paths")?,
            "joined_bytes": read_u64(row, "joined_bytes")?,
            "proven_absent_paths": read_u64(row, "proven_absent_paths")?,
            "localisation_source_sha256": read_optional_digest(row, "localisation_source_sha256")?,
        });
        if joined
            .insert(role.to_string(), artifact_sha256.to_string())
            .is_some()
        {
            return Err(format!(
                "attach-time content proof joins artifact role {role:?} more than once"
            ));
        }
        join_rows.push(canonical);
    }

    let mut refused = BTreeMap::new();
    for row in read_rows(proof, "refused_artifacts")? {
        let row = row.as_object().ok_or_else(|| {
            "attach-time content proof refusal is not an object".to_string()
        })?;
        let role = read_non_empty(row, "role")?;
        let reason = read_non_empty(row, "reason")?;
        if refused.insert(role.to_string(), reason.to_string()).is_some() {
            return Err(format!(
                "attach-time content proof refuses artifact role {role:?} more than once"
            ));
        }
    }
    // A role that is both joined and refused makes the proof self-contradicting,
    // and which disposition wins would be decided by read order.
    if let Some(role) = refused.keys().find(|role| joined.contains_key(*role)) {
        return Err(format!(
            "attach-time content proof both joins and refuses artifact role {role:?}"
        ));
    }

    let mut required_rows = Vec::new();
    let mut required_keys = BTreeMap::new();
    for row in read_rows(proof, "required_content")? {
        let row = row.as_object().ok_or_else(|| {
            "attach-time content proof required-content row is not an object".to_string()
        })?;
        let purpose = read_non_empty(row, "purpose")?;
        let logical_path = read_non_empty(row, "logical_path")?;
        // The sort below keys on (purpose, logical_path). Duplicates would make
        // it a PARTIAL order, so two rows differing only in digest could be
        // swapped by the producer and change the epoch — exactly the
        // producer-order independence this canonicalization claims (Codex
        // round 1, LOW).
        if required_keys
            .insert((purpose.to_string(), logical_path.to_string()), ())
            .is_some()
        {
            return Err(format!(
                "attach-time content proof names required content {logical_path:?} for purpose \
                 {purpose:?} more than once"
            ));
        }
        required_rows.push(json!({
            "purpose": purpose,
            "logical_path": logical_path,
            "sha256": read_digest(row, "sha256")?,
            "bytes": read_u64(row, "bytes")?,
            "winning_layer_id": read_non_empty(row, "winning_layer_id")?,
        }));
    }
    // Parsed and bounded even though it is deliberately NOT hashed: a proof
    // whose refusal list is malformed or unbounded is a malformed proof, and
    // leaving one array of the block unread is how a reader learns to trust
    // shapes it never checked.
    for row in read_rows(proof, "refused_required_content")? {
        let row = row.as_object().ok_or_else(|| {
            "attach-time content proof refused-required-content row is not an object".to_string()
        })?;
        read_non_empty(row, "purpose")?;
        read_non_empty(row, "logical_path")?;
        read_non_empty(row, "reason")?;
    }
    // Canonical order, independent of the order the producer happened to emit.
    join_rows.sort_by(|left, right| sort_key(left, &["role"]).cmp(&sort_key(right, &["role"])));
    required_rows.sort_by(|left, right| {
        sort_key(left, &["purpose", "logical_path"]).cmp(&sort_key(right, &["purpose", "logical_path"]))
    });

    let document = json!({
        "epoch_schema": EPOCH_SCHEMA,
        "proof_schema": ATTACH_PROOF_SCHEMA,
        "proof_kind": KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
        "executable_sha256": executable_sha256,
        "executable_size_bytes": executable_size_bytes,
        "dlc_load_sha256": dlc_load_sha256,
        "layer_stack_sha256": layer_stack_sha256,
        "artifact_joins": join_rows,
        "required_content": required_rows,
    });
    let bytes = serde_json::to_vec(&document)
        .map_err(|error| format!("serialize attach-time content epoch: {error}"))?;
    Ok(AttachContentEpoch {
        digest: format!("{:x}", Sha256::digest(&bytes)),
        joined,
        refused,
    })
}

/// Digest over the ordered layer stack, taken over the same JSON rows the
/// manifest carries.
///
/// Shared with `recorder_host::attach_content_proof`, which builds the rows
/// from its own structs before the manifest exists; both spellings must hash
/// identical bytes, which the round-trip test in that module asserts.
pub fn layer_stack_digest(layers: &[Value]) -> Result<String, String> {
    let rows = layers
        .iter()
        .map(|layer| {
            let layer = layer
                .as_object()
                .ok_or_else(|| "content-proof layer is not an object".to_string())?;
            let kind = read_non_empty(layer, "kind")?;
            if !LAYER_KINDS.contains(&kind) {
                return Err(format!(
                    "attach-time content proof names unknown content-layer kind {kind:?}"
                ));
            }
            Ok(json!({
                "ordinal": read_u64(layer, "ordinal")?,
                "kind": kind,
                "layer_id": read_non_empty(layer, "layer_id")?,
                "descriptor_sha256": read_optional_digest(layer, "descriptor_sha256")?,
                "descriptor_file_sha256": read_optional_digest(layer, "descriptor_file_sha256")?,
                "source_container_sha256": read_optional_digest(layer, "source_container_sha256")?,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let bytes = serde_json::to_vec(&rows)
        .map_err(|error| format!("serialize attach-time layer stack: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn sort_key(row: &Value, fields: &[&str]) -> Vec<String> {
    fields
        .iter()
        .map(|field| {
            row.get(*field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

fn require_str(object: &Map<String, Value>, field: &str, expected: &str) -> Result<(), String> {
    match object.get(field).and_then(Value::as_str) {
        Some(value) if value == expected => Ok(()),
        Some(value) => Err(format!(
            "attach-time content proof {field} is {value:?}, expected {expected:?}"
        )),
        None => Err(format!(
            "attach-time content proof {field} is absent or not a string"
        )),
    }
}

fn read_rows<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a [Value], String> {
    let rows = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("attach-time content proof {field} is absent or not an array"))?;
    if rows.len() > MAX_PROOF_ROWS {
        return Err(format!(
            "attach-time content proof {field} has {} rows, above the {MAX_PROOF_ROWS} ceiling",
            rows.len()
        ));
    }
    Ok(rows.as_slice())
}

fn read_non_empty<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    match object.get(field).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value),
        _ => Err(format!(
            "attach-time content proof {field} is absent, empty or not a string"
        )),
    }
}

fn read_u64(object: &Map<String, Value>, field: &str) -> Result<u64, String> {
    object
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("attach-time content proof {field} is absent or not a u64"))
}

fn read_digest<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    let value = read_non_empty(object, field)?;
    if !is_lower_sha256(value) {
        return Err(format!(
            "attach-time content proof {field} is not a lowercase sha256"
        ));
    }
    Ok(value)
}

fn read_optional_digest<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<Option<&'a str>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(_)) => read_digest(object, field).map(Some),
        Some(_) => Err(format!(
            "attach-time content proof {field} is neither a digest string nor null"
        )),
    }
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(ordinal: u64, id: &str) -> Value {
        json!({
            "ordinal": ordinal,
            "kind": "base",
            "layer_id": id,
            "descriptor_sha256": Value::Null,
            "descriptor_file_sha256": Value::Null,
            "source_container_sha256": Value::Null,
        })
    }

    fn digest(seed: u8) -> String {
        format!("{:02x}", seed).repeat(32)
    }

    fn proof(joins: Vec<Value>, refusals: Vec<Value>) -> Value {
        let layers = vec![layer(0, "base"), layer(1, "dlc/foo")];
        let stack = layer_stack_digest(&layers).unwrap();
        json!({
            "schema": ATTACH_PROOF_SCHEMA,
            "kind": KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1,
            "game_process_start_unix": 1,
            "observed_at_unix": 2,
            "completed_at_unix": 3,
            "elapsed_ms": 4,
            "hashed_bytes": 5,
            "hashed_files": 6,
            "window_attempts": 1,
            "window_settled": true,
            "executable_sha256": digest(0xab),
            "executable_size_bytes": 55680632u64,
            "dlc_load_sha256": digest(0xcd),
            "layer_stack": layers,
            "layer_stack_sha256": stack,
            "artifact_joins": joins,
            "refused_artifacts": refusals,
            "required_content": [{
                "purpose": "D81 province catalog",
                "logical_path": "map/definition.csv",
                "sha256": digest(0x11),
                "bytes": 465289u64,
                "winning_layer_id": "base",
            }],
            "refused_required_content": [],
        })
    }

    fn join(role: &str, sha: &str) -> Value {
        json!({
            "role": role,
            "artifact_sha256": sha,
            "claimed_paths": 3,
            "joined_bytes": 9,
            "proven_absent_paths": 0,
            "localisation_source_sha256": Value::Null,
        })
    }

    /// Kills: writing the row order into the digest. Reordering `artifact_joins`
    /// would change the epoch, so two readers of one install could disagree.
    #[test]
    fn the_epoch_is_independent_of_join_row_order() {
        let a = join("localisation_join", &digest(0x01));
        let b = join("technology_sharing_static", &digest(0x02));
        let left = epoch_from_proof_json(&proof(vec![a.clone(), b.clone()], Vec::new())).unwrap();
        let right = epoch_from_proof_json(&proof(vec![b, a], Vec::new())).unwrap();
        assert_eq!(left.digest(), right.digest());
    }

    /// Kills: folding the observation window or its cost into the epoch. Two
    /// recordings of one unchanged install must bind to one epoch.
    #[test]
    fn the_epoch_ignores_the_window_and_its_cost() {
        let joins = vec![join("localisation_join", &digest(0x01))];
        let base = proof(joins.clone(), Vec::new());
        let mut drifted = base.clone();
        for (field, value) in [
            ("observed_at_unix", json!(999_999)),
            ("completed_at_unix", json!(1_000_000)),
            ("elapsed_ms", json!(88_888)),
            ("hashed_bytes", json!(7)),
            ("hashed_files", json!(8)),
            ("window_attempts", json!(5)),
            ("game_process_start_unix", json!(4242)),
        ] {
            drifted[field] = value;
        }
        assert_eq!(
            epoch_from_proof_json(&base).unwrap().digest(),
            epoch_from_proof_json(&drifted).unwrap().digest()
        );
    }

    /// Kills: dropping any content-identity field from the hashed document.
    #[test]
    fn the_epoch_changes_when_the_content_world_changes() {
        let joins = vec![join("localisation_join", &digest(0x01))];
        let base = epoch_from_proof_json(&proof(joins.clone(), Vec::new()))
            .unwrap()
            .digest()
            .to_string();
        for (field, value) in [
            ("executable_sha256", json!(digest(0x99))),
            ("executable_size_bytes", json!(1u64)),
            ("dlc_load_sha256", json!(digest(0x98))),
        ] {
            let mut drifted = proof(joins.clone(), Vec::new());
            drifted[field] = value;
            assert_ne!(
                base,
                epoch_from_proof_json(&drifted).unwrap().digest(),
                "{field} must change the epoch"
            );
        }
        // A different layer stack is a different world.
        let mut relayered = proof(joins.clone(), Vec::new());
        let layers = vec![layer(0, "base"), layer(1, "mod/other")];
        relayered["layer_stack_sha256"] = json!(layer_stack_digest(&layers).unwrap());
        relayered["layer_stack"] = json!(layers);
        assert_ne!(base, epoch_from_proof_json(&relayered).unwrap().digest());
        // And so is a different build of a packaged artifact.
        let rebuilt = vec![join("localisation_join", &digest(0x07))];
        assert_ne!(
            base,
            epoch_from_proof_json(&proof(rebuilt, Vec::new()))
                .unwrap()
                .digest()
        );
    }

    /// Kills: trusting the declared `layer_stack_sha256`.
    #[test]
    fn a_layer_stack_digest_that_does_not_match_its_stack_is_refused() {
        let mut tampered = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        tampered["layer_stack_sha256"] = json!(digest(0x55));
        let error = epoch_from_proof_json(&tampered).unwrap_err();
        assert!(error.contains("does not match its own layer stack"), "{error}");
    }

    /// Kills: deriving an epoch from an unsettled window.
    #[test]
    fn an_unsettled_window_yields_no_epoch() {
        let mut unsettled = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        unsettled["window_settled"] = json!(false);
        let error = epoch_from_proof_json(&unsettled).unwrap_err();
        assert!(error.contains("never settled"), "{error}");
    }

    /// Kills: binding an artifact the proof never joined, one it refused, or a
    /// different build of one it did join.
    #[test]
    fn bind_admits_only_the_exact_certified_build() {
        let epoch = epoch_from_proof_json(&proof(
            vec![join("localisation_join", &digest(0x01))],
            vec![json!({"role": "focus_defs_static", "reason": "two paths disagreed"})],
        ))
        .unwrap();
        assert_eq!(
            epoch.bind("localisation_join", &digest(0x01)).unwrap(),
            epoch.digest()
        );
        let wrong_build = epoch
            .bind("localisation_join", &digest(0x02))
            .unwrap_err();
        assert!(wrong_build.contains("different build"), "{wrong_build}");
        let refused = epoch.bind("focus_defs_static", &digest(0x03)).unwrap_err();
        assert!(refused.contains("two paths disagreed"), "{refused}");
        let unjoined = epoch
            .bind("ship_definitions_static", &digest(0x04))
            .unwrap_err();
        assert!(unjoined.contains("no join"), "{unjoined}");
    }

    /// Kills: letting a self-contradicting proof pick a disposition by read
    /// order.
    #[test]
    fn a_role_that_is_both_joined_and_refused_is_refused() {
        let error = epoch_from_proof_json(&proof(
            vec![join("localisation_join", &digest(0x01))],
            vec![json!({"role": "localisation_join", "reason": "mismatch"})],
        ))
        .unwrap_err();
        assert!(error.contains("both joins and refuses"), "{error}");
    }

    /// Kills: reading an unstated or unrecognised proof kind as a known one.
    #[test]
    fn the_proof_kind_declaration_is_read_positively() {
        let mut manifest = Map::new();
        assert_eq!(content_proof_kind(&manifest).unwrap(), None);
        manifest.insert(CONTENT_PROOF_KIND_FIELD.to_string(), json!(KIND_NONE));
        assert_eq!(content_proof_kind(&manifest).unwrap(), Some(KIND_NONE));
        manifest.insert(CONTENT_PROOF_KIND_FIELD.to_string(), json!("invented"));
        assert!(content_proof_kind(&manifest).is_err());
        manifest.insert(CONTENT_PROOF_KIND_FIELD.to_string(), json!(7));
        assert!(content_proof_kind(&manifest).is_err());
    }
    /// Kills: accepting a content-layer kind the producer's own wire reader
    /// refuses, which would let a layer stack nothing could have written hash
    /// to a valid epoch.
    #[test]
    fn an_unknown_content_layer_kind_is_refused() {
        let mut stack = vec![layer(0, "base")];
        stack[0]["kind"] = json!("invented_layer_kind");
        let mut tampered = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        // A stack whose declared digest matches it, so the ONLY thing wrong is
        // the kind.
        tampered["layer_stack_sha256"] = json!("0".repeat(64));
        tampered["layer_stack"] = json!(stack.clone());
        let error = epoch_from_proof_json(&tampered).unwrap_err();
        assert!(error.contains("unknown content-layer kind"), "{error}");
        // And `layer_stack_digest` refuses it directly, so a caller cannot get a
        // digest for a stack the epoch would then reject.
        assert!(layer_stack_digest(&stack)
            .unwrap_err()
            .contains("unknown content-layer kind"));
    }

    /// Kills: leaving `(purpose, logical_path)` a partial order. With duplicate
    /// keys the sort is not total, so a producer could swap two rows that differ
    /// only in digest and change the epoch — breaking the order-independence
    /// this canonicalization claims.
    #[test]
    fn duplicate_required_content_keys_are_refused() {
        let mut duplicated = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        let row = |sha: String| {
            json!({
                "purpose": "D81 province catalog",
                "logical_path": "map/definition.csv",
                "sha256": sha,
                "bytes": 1u64,
                "winning_layer_id": "base",
            })
        };
        duplicated["required_content"] = json!([row(digest(0x21)), row(digest(0x22))]);
        let error = epoch_from_proof_json(&duplicated).unwrap_err();
        assert!(error.contains("more than once"), "{error}");
    }

    /// Kills: leaving one array of the block unread. It is deliberately not
    /// hashed, but a malformed or unbounded refusal list is a malformed proof.
    #[test]
    fn a_malformed_refused_required_content_row_is_refused() {
        let mut malformed = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        malformed["refused_required_content"] = json!([{
            "purpose": "D81 province catalog",
            "logical_path": "map/definition.csv",
        }]);
        let error = epoch_from_proof_json(&malformed).unwrap_err();
        assert!(error.contains("reason"), "{error}");

        // Well-formed refusals are accepted and do NOT change the epoch: they
        // record what the proof could not state, not what it did.
        let mut refused = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        refused["refused_required_content"] = json!([{
            "purpose": "D81 province catalog",
            "logical_path": "map/definition.csv",
            "reason": "the effective VFS refused the path",
        }]);
        assert_eq!(
            epoch_from_proof_json(&refused).unwrap().digest(),
            epoch_from_proof_json(&proof(
                vec![join("localisation_join", &digest(0x01))],
                Vec::new()
            ))
            .unwrap()
            .digest()
        );
    }

    /// THE CROSS-SEAM ROUND TRIP, actually asserted.
    ///
    /// The recorder computes `layer_stack_sha256` from its own `ProofLayer`
    /// STRUCTS and seals the block; the replay consumer re-derives it from the
    /// JSON rows it parses back out. Both call this function, but they hand it
    /// rows built two different ways, so "they hash identical bytes" is an
    /// assumption until something asserts it. A doc comment in
    /// `recorder_host::attach_content_proof` claimed this test existed before it
    /// did (Codex round 1, LOW).
    ///
    /// kills: any canonicalization that is not stable across a
    /// serialize/parse round trip — a non-BTreeMap key order, a float or
    /// signed spelling of `ordinal`, or dropping a null field on the way out.
    #[test]
    fn the_layer_stack_digest_survives_a_serialize_parse_round_trip() {
        let built = vec![
            layer(0, "base"),
            json!({
                "ordinal": 1,
                "kind": "integrated_dlc",
                "layer_id": "integrated_dlc/dlc018_together_for_victory/dlc018.dlc",
                "descriptor_sha256": digest(0x31),
                "descriptor_file_sha256": digest(0x32),
                "source_container_sha256": Value::Null,
            }),
            json!({
                "ordinal": 2,
                "kind": "mod",
                "layer_id": "mod/x.mod",
                "descriptor_sha256": Value::Null,
                "descriptor_file_sha256": digest(0x33),
                "source_container_sha256": digest(0x34),
            }),
        ];
        let direct = layer_stack_digest(&built).unwrap();

        // Exactly what the seam does: serialize the rows into a manifest, read
        // them back, and re-derive.
        let sealed = serde_json::to_vec(&json!({"layer_stack": built})).unwrap();
        let parsed: Value = serde_json::from_slice(&sealed).unwrap();
        let reparsed = parsed["layer_stack"].as_array().unwrap().clone();
        assert_eq!(direct, layer_stack_digest(&reparsed).unwrap());

        // And the whole epoch survives the same trip.
        let mut block = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        block["layer_stack_sha256"] = json!(direct);
        block["layer_stack"] = json!(built);
        let before = epoch_from_proof_json(&block).unwrap();
        let after = epoch_from_proof_json(
            &serde_json::from_slice::<Value>(&serde_json::to_vec(&block).unwrap()).unwrap(),
        )
        .unwrap();
        assert_eq!(before.digest(), after.digest());
    }
}
