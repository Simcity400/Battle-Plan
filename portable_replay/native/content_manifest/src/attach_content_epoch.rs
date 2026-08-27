use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const ATTACH_PROOF_SCHEMA: &str = "hoi4.attach-time-content-proof.v1";

pub const EPOCH_SCHEMA: &str = "hoi4.attach-time-content-epoch.v1";

pub const CONTENT_PROOF_KIND_FIELD: &str = "content_proof_kind";

pub const ATTACH_TIME_CONTENT_PROOF_FIELD: &str = "attach_time_content_proof";

pub const KIND_LAUNCH_TIME_STATIC_BUNDLE_V1: &str = "launch_time_static_bundle_v1";

pub const KIND_ATTACH_TIME_EFFECTIVE_VFS_JOIN_V1: &str = "attach_time_effective_vfs_join_v1";

pub const KIND_NONE: &str = "none";

/// How a refused artifact's `artifact_sha256` reads when it is rendered for a
/// human. Never the CARRIER of the disposition - see [`RefusedArtifactDigest`].
pub const DIGEST_UNRECORDED_PRE_BUMP: &str = "digest unrecorded (pre-bump capture)";

/// What the proof said about the build it refused.
///
/// TYPED, and derived from the JSON SHAPE rather than from any text the producer
/// wrote. An earlier version of this appended `DIGEST_UNRECORDED_PRE_BUMP` to the
/// free-text `reason`, which meant a proof carrying an explicit null and that
/// phrase inside its own reason string was indistinguishable from a pre-bump row
/// (Codex round 6). A producer cannot spell this field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusedArtifactDigest {
    /// The proof named the build it refused.
    Recorded,
    /// `artifact_sha256: null` - the recorder could not read the file at all.
    RecorderUnreadable,
    /// No `artifact_sha256` key, which only a recording sealed before the
    /// recorder wrote that field can do.
    UnrecordedPreBump,
}

impl RefusedArtifactDigest {
    /// The phrase a rendered refusal appends, or `None` when there is nothing to
    /// add beyond the producer's own reason.
    pub fn note(self) -> Option<&'static str> {
        match self {
            RefusedArtifactDigest::UnrecordedPreBump => Some(DIGEST_UNRECORDED_PRE_BUMP),
            RefusedArtifactDigest::Recorded | RefusedArtifactDigest::RecorderUnreadable => None,
        }
    }
}

/// One refused artifact: the producer's reason, verbatim, plus the typed
/// disposition of the digest it did or did not name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactRefusal {
    pub reason: String,
    pub digest: RefusedArtifactDigest,
}

impl ArtifactRefusal {
    /// The reason with the digest disposition's note, for a surface that prints
    /// one line.
    pub fn rendered(&self) -> String {
        match self.digest.note() {
            Some(note) => format!("{}; {note}", self.reason),
            None => self.reason.clone(),
        }
    }
}

/// The row ceiling every reader of this proof shares. Public so the compact
/// verification gate enforces the SAME one: a gate that certified an array this
/// parser refuses is a recording that passes the gate and cannot be opened
/// (Codex round 7).
pub const MAX_PROOF_ROWS: usize = 1 << 16;

const LAYER_KINDS: [&str; 4] = ["base", "integrated_dlc", "dlc", "mod"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachContentEpoch {
    digest: String,
    joined: BTreeMap<String, String>,
    refused: BTreeMap<String, ArtifactRefusal>,
}

impl AttachContentEpoch {
    pub fn digest(&self) -> &str {
        &self.digest
    }

    pub fn joined_roles(&self) -> impl Iterator<Item = &str> {
        self.joined.keys().map(String::as_str)
    }

    /// The proof's own refusal for one role, typed, or `None` if it did not
    /// refuse that role. A caller that only has `authorizes`' rendered message
    /// would lose the disposition, so this is what a typed surface reads.
    pub fn refusal_for(&self, role: &str) -> Option<&ArtifactRefusal> {
        self.refused.get(role)
    }

    pub fn refusals(&self) -> impl Iterator<Item = (&str, &ArtifactRefusal)> {
        self.refused
            .iter()
            .map(|(role, refusal)| (role.as_str(), refusal))
    }

    pub fn bind(&self, role: &str, artifact_sha256: &str) -> Result<&str, String> {
        if let Some(refusal) = self.refused.get(role) {
            return Err(format!(
                "attach-time content proof refused this artifact: {}",
                refusal.rendered()
            ));
        }
        match self.joined.get(role) {
            None => Err("attach-time content proof carries no join for this artifact".to_string()),
            Some(proven) if proven == artifact_sha256 => Ok(self.digest.as_str()),
            Some(proven) => Err(format!(
                "attach-time content proof joined a different build of this artifact \
                 (proof {proven}, package {artifact_sha256})"
            )),
        }
    }
}

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
        let row = row.as_object().ok_or_else(|| {
            "attach-time content proof artifact join is not an object".to_string()
        })?;
        let role = read_non_empty(row, "role")?;
        let artifact_sha256 = read_digest(row, "artifact_sha256")?;
        // A join over ZERO paths compared the artifact against nothing in the
        // effective VFS, so it proves nothing about the captured content - and
        // `bind` authorizes on the digest alone, which would let such a join
        // pass an artifact off as content-bound. The compact gate has always
        // refused it; this reader accepted it, and the two must not disagree
        // (Codex round 10).
        let claimed_paths = read_u64(row, "claimed_paths")?;
        if claimed_paths == 0 {
            return Err(format!(
                "attach-time content proof join for artifact role {role:?} claims zero paths, so \
                 it binds the artifact to no content at all"
            ));
        }
        let canonical = json!({
            "role": role,
            "artifact_sha256": artifact_sha256,
            "claimed_paths": claimed_paths,
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
        let row = row
            .as_object()
            .ok_or_else(|| "attach-time content proof refusal is not an object".to_string())?;
        let role = read_non_empty(row, "role")?;
        let reason = read_non_empty(row, "reason")?;
        // A refusal that does not identify the build it refused cannot be traced
        // back to an artifact on disk, so the digest is read strictly whenever it
        // is there and `null` stays the recorder's way of saying "the file could
        // not be read at all".
        //
        // An ABSENT key is a recording sealed before the recorder wrote the field.
        // It means the same thing the null does - digest unknown - and refusing
        // it would make every such recording unopenable, so it is admitted. It is
        // NOT equated silently: the disposition below is typed and taken from the
        // JSON shape, so the two are distinguishable by a consumer and the
        // producer's own reason text cannot forge either.
        let digest = match row.get("artifact_sha256") {
            None => RefusedArtifactDigest::UnrecordedPreBump,
            Some(Value::Null) => RefusedArtifactDigest::RecorderUnreadable,
            Some(_) => RefusedArtifactDigest::Recorded,
        };
        read_optional_digest(row, "artifact_sha256")?;
        let refusal = ArtifactRefusal {
            reason: reason.to_string(),
            digest,
        };
        if refused.insert(role.to_string(), refusal).is_some() {
            return Err(format!(
                "attach-time content proof refuses artifact role {role:?} more than once"
            ));
        }
    }
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
    for row in read_rows(proof, "refused_required_content")? {
        let row = row.as_object().ok_or_else(|| {
            "attach-time content proof refused-required-content row is not an object".to_string()
        })?;
        read_non_empty(row, "purpose")?;
        read_non_empty(row, "logical_path")?;
        read_non_empty(row, "reason")?;
    }
    join_rows.sort_by(|left, right| sort_key(left, &["role"]).cmp(&sort_key(right, &["role"])));
    required_rows.sort_by(|left, right| {
        sort_key(left, &["purpose", "logical_path"])
            .cmp(&sort_key(right, &["purpose", "logical_path"]))
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

    fn refusal(role: &str, artifact_sha256: Option<Value>) -> Value {
        let mut row = json!({"role": role, "reason": "33 claimed paths did not match"});
        if let Some(digest) = artifact_sha256 {
            row["artifact_sha256"] = digest;
        }
        row
    }

    /// A refused artifact's `artifact_sha256` has THREE states and each gets its
    /// own outcome.
    ///
    /// kills: a tolerance for the absent key that also swallows a wrong one, and
    /// an absent key that reads as an ordinary refusal. `captures/rc_vanilla7`
    /// was sealed before the recorder wrote the field and carries exactly the
    /// absent shape; requiring the key made every such recording unopenable
    /// (lane UH, owner ruling 2026-08-27).
    #[test]
    fn a_refused_artifacts_digest_is_validated_named_or_refused_and_never_confused() {
        let epoch = |artifact_sha256: Option<Value>| {
            epoch_from_proof_json(&proof(
                vec![join("localisation_join", &digest(0x01))],
                vec![refusal("gfx_icon_asset_manifest_static", artifact_sha256)],
            ))
        };
        let refusal = |result: &AttachContentEpoch| {
            result
                .refusals()
                .find(|(role, _)| *role == "gfx_icon_asset_manifest_static")
                .expect("the refusal is carried")
                .1
                .clone()
        };

        // PRESENT and valid: taken as written, nothing appended.
        let present = refusal(&epoch(Some(json!(digest(0x07)))).expect("a valid digest is admitted"));
        assert_eq!(present.digest, RefusedArtifactDigest::Recorded);
        assert_eq!(present.rendered(), "33 claimed paths did not match");

        // NULL: the recorder's own "the file could not be read at all". Admitted,
        // and it keeps its own wording -- it is not the pre-bump case.
        let null = refusal(&epoch(Some(Value::Null)).expect("a null digest is admitted"));
        assert_eq!(null.digest, RefusedArtifactDigest::RecorderUnreadable);
        assert_eq!(null.rendered(), "33 claimed paths did not match");

        // ABSENT: admitted like the null, and NAMED so the two never read alike.
        let absent = refusal(&epoch(None).expect("an absent digest is admitted"));
        assert_eq!(absent.digest, RefusedArtifactDigest::UnrecordedPreBump);
        assert_eq!(
            absent.rendered(),
            format!("33 claimed paths did not match; {DIGEST_UNRECORDED_PRE_BUMP}")
        );
        assert_ne!(absent.digest, null.digest);

        // PRESENT and wrong: still a whole-proof refusal, in every spelling. The
        // tolerance above must not have widened into this.
        for wrong in [
            json!("not-a-digest"),
            json!(""),
            // Uppercase hex: a digest this crate spells only in lowercase. The
            // seed has to carry a letter for the case to differ at all.
            json!(digest(0xab).to_uppercase()),
            json!(digest(0x07)[..63].to_string()),
            json!(7),
            json!([digest(0x07)]),
        ] {
            epoch(Some(wrong.clone()))
                .expect_err(&format!("{wrong} is not a digest and must refuse the proof"));
        }
    }

    /// kills: carrying the digest disposition in the free-text reason, where a
    /// producer can spell it. These two rows render IDENTICALLY and must still be
    /// distinguishable, because only one of them actually withheld the key
    /// (Codex round 6).
    #[test]
    fn a_reason_that_spells_the_pre_bump_note_cannot_forge_the_disposition() {
        let epoch = |artifact_sha256: Option<Value>, reason: &str| {
            let mut row = json!({"role": "gfx_icon_asset_manifest_static", "reason": reason});
            if let Some(digest) = artifact_sha256 {
                row["artifact_sha256"] = digest;
            }
            epoch_from_proof_json(&proof(
                vec![join("localisation_join", &digest(0x01))],
                vec![row],
            ))
            .expect("both rows are admitted")
                .refusals()
                .find(|(role, _)| *role == "gfx_icon_asset_manifest_static")
                .expect("the refusal is carried")
                .1
                .clone()
        };
        let forged = epoch(
            Some(Value::Null),
            &format!("33 claimed paths did not match; {DIGEST_UNRECORDED_PRE_BUMP}"),
        );
        let genuine = epoch(None, "33 claimed paths did not match");
        assert_eq!(forged.rendered(), genuine.rendered());
        assert_eq!(forged.digest, RefusedArtifactDigest::RecorderUnreadable);
        assert_eq!(genuine.digest, RefusedArtifactDigest::UnrecordedPreBump);
        assert_ne!(forged.digest, genuine.digest);
    }

    /// kills: authorizing an artifact off a join that compared it against
    /// nothing. `bind` checks the digest alone, so a zero-path join would let a
    /// proof declare the package's own digest and have the artifact read as
    /// content-bound. The compact gate always refused this; both sites do now
    /// (Codex round 10).
    #[test]
    fn a_join_that_claims_no_paths_binds_nothing_and_is_refused() {
        let mut row = join("localisation_join", &digest(0x01));
        row["claimed_paths"] = json!(0);
        let error = epoch_from_proof_json(&proof(vec![row], Vec::new()))
            .expect_err("a zero-path join proves nothing");
        assert!(
            error.contains("localisation_join") && error.contains("zero paths"),
            "{error}"
        );
        epoch_from_proof_json(&proof(
            vec![join("localisation_join", &digest(0x01))],
            Vec::new(),
        ))
        .expect("one claimed path is enough to be a join");
    }

    /// The named refusal reason is not inside the epoch digest, so admitting a
    /// pre-bump recording cannot move the content identity of any recording that
    /// already opened.
    #[test]
    fn naming_the_unrecorded_digest_does_not_move_the_epoch() {
        let joins = vec![join("localisation_join", &digest(0x01))];
        let with_absent = epoch_from_proof_json(&proof(
            joins.clone(),
            vec![refusal("gfx_icon_asset_manifest_static", None)],
        ))
        .unwrap();
        let with_null = epoch_from_proof_json(&proof(
            joins,
            vec![refusal(
                "gfx_icon_asset_manifest_static",
                Some(Value::Null),
            )],
        ))
        .unwrap();
        assert_eq!(with_absent.digest(), with_null.digest());
    }

    #[test]
    fn the_epoch_is_independent_of_join_row_order() {
        let a = join("localisation_join", &digest(0x01));
        let b = join("technology_sharing_static", &digest(0x02));
        let left = epoch_from_proof_json(&proof(vec![a.clone(), b.clone()], Vec::new())).unwrap();
        let right = epoch_from_proof_json(&proof(vec![b, a], Vec::new())).unwrap();
        assert_eq!(left.digest(), right.digest());
    }

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
        let mut relayered = proof(joins.clone(), Vec::new());
        let layers = vec![layer(0, "base"), layer(1, "mod/other")];
        relayered["layer_stack_sha256"] = json!(layer_stack_digest(&layers).unwrap());
        relayered["layer_stack"] = json!(layers);
        assert_ne!(base, epoch_from_proof_json(&relayered).unwrap().digest());
        let rebuilt = vec![join("localisation_join", &digest(0x07))];
        assert_ne!(
            base,
            epoch_from_proof_json(&proof(rebuilt, Vec::new()))
                .unwrap()
                .digest()
        );
    }

    #[test]
    fn a_layer_stack_digest_that_does_not_match_its_stack_is_refused() {
        let mut tampered = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        tampered["layer_stack_sha256"] = json!(digest(0x55));
        let error = epoch_from_proof_json(&tampered).unwrap_err();
        assert!(
            error.contains("does not match its own layer stack"),
            "{error}"
        );
    }

    #[test]
    fn an_unsettled_window_yields_no_epoch() {
        let mut unsettled = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        unsettled["window_settled"] = json!(false);
        let error = epoch_from_proof_json(&unsettled).unwrap_err();
        assert!(error.contains("never settled"), "{error}");
    }

    #[test]
    fn bind_admits_only_the_exact_certified_build() {
        let epoch = epoch_from_proof_json(&proof(
            vec![join("localisation_join", &digest(0x01))],
            vec![json!({
                "role": "focus_defs_static",
                "artifact_sha256": digest(0x09),
                "reason": "two paths disagreed",
            })],
        ))
        .unwrap();
        assert_eq!(
            epoch.bind("localisation_join", &digest(0x01)).unwrap(),
            epoch.digest()
        );
        let wrong_build = epoch.bind("localisation_join", &digest(0x02)).unwrap_err();
        assert!(wrong_build.contains("different build"), "{wrong_build}");
        let refused = epoch.bind("focus_defs_static", &digest(0x03)).unwrap_err();
        assert!(refused.contains("two paths disagreed"), "{refused}");
        let unjoined = epoch
            .bind("ship_definitions_static", &digest(0x04))
            .unwrap_err();
        assert!(unjoined.contains("no join"), "{unjoined}");
    }

    #[test]
    fn a_role_that_is_both_joined_and_refused_is_refused() {
        let error = epoch_from_proof_json(&proof(
            vec![join("localisation_join", &digest(0x01))],
            vec![json!({
                "role": "localisation_join",
                "artifact_sha256": digest(0x0a),
                "reason": "mismatch",
            })],
        ))
        .unwrap_err();
        assert!(error.contains("both joins and refuses"), "{error}");
    }

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
    #[test]
    fn an_unknown_content_layer_kind_is_refused() {
        let mut stack = vec![layer(0, "base")];
        stack[0]["kind"] = json!("invented_layer_kind");
        let mut tampered = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        tampered["layer_stack_sha256"] = json!("0".repeat(64));
        tampered["layer_stack"] = json!(stack.clone());
        let error = epoch_from_proof_json(&tampered).unwrap_err();
        assert!(error.contains("unknown content-layer kind"), "{error}");
        assert!(layer_stack_digest(&stack)
            .unwrap_err()
            .contains("unknown content-layer kind"));
    }

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

    #[test]
    fn a_malformed_refused_required_content_row_is_refused() {
        let mut malformed = proof(vec![join("localisation_join", &digest(0x01))], Vec::new());
        malformed["refused_required_content"] = json!([{
            "purpose": "D81 province catalog",
            "logical_path": "map/definition.csv",
        }]);
        let error = epoch_from_proof_json(&malformed).unwrap_err();
        assert!(error.contains("reason"), "{error}");

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

        let sealed = serde_json::to_vec(&json!({"layer_stack": built})).unwrap();
        let parsed: Value = serde_json::from_slice(&sealed).unwrap();
        let reparsed = parsed["layer_stack"].as_array().unwrap().clone();
        assert_eq!(direct, layer_stack_digest(&reparsed).unwrap());

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
