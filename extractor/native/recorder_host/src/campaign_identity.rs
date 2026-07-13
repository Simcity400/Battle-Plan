//! W12C campaign/save identity binding (recorder_host side).
//!
//! Design of record: `extractor/documents/CAMPAIGN_IDENTITY_DESIGN.md`.
//!
//! The v1 campaign identity is the engine-persisted pair
//! (`game_unique_id`, `game_unique_seed`). The external `campaign_id` is derived
//! from their canonical representation (SHA-256 of a fixed preimage). Both raw
//! components are preserved as first-class fields; the digest is only an index.
//!
//! ## Fail-closed reality at this revision
//!
//! Neither component is offset-proven or wired into the live d68
//! `session_header_live` payload yet (design "Open items"; the current wire
//! schema carries player tag / DLC mask / start date / save version / session /
//! ideology / cosmetic tag / build banner and *no* `game_unique_*` field — see
//! `world_fold.rs::fold_session_header` and
//! `main.rs::decode_compact_session_header_data_payload`). Consequently every
//! real capture at this revision resolves to `state = "unavailable"` (d68 rows
//! were seen but the components are not sourced) or `state = "unbound"` (no d68
//! row was observed at all). Identity is NEVER fabricated from witnesses; a
//! missing/torn/partial component yields an explicit null plus a reason note.
//!
//! The resolver, digest math, closed-grammar manifest validator, and stitching
//! consumer interface are complete and test-covered so that the moment the
//! collector sources the pair into the observation (a future, offset-proven
//! change), a `state = "known"` binding flows through unchanged.

use serde_json::{json, Map, Value};
use snapshot::sha256::{to_hex, Sha256};

use compact_contract::calendar::hour_index_from_raw;

/// Manifest schema tag for the campaign identity object.
pub const CAMPAIGN_IDENTITY_SCHEMA: &str = "hoi4.campaign_identity.v1";

/// Fixed digest preimage prefix (design "Canonical preimage"). Already carries a
/// trailing NUL; a second NUL separates the UUID from the seed at digest time.
pub const CID_PREIMAGE_PREFIX: &[u8] = b"hoi4-campaign-id-v1\0";

/// Domain separator for the in-memory evidence binding. The proof is never
/// serialized; it binds authorization to immutable CID1 bytes and the exact
/// current fields rather than to a transferable boolean latch.
const EVIDENCE_BINDING_PREFIX: &[u8] = b"hoi4-campaign-identity-evidence-v1\0";

/// Source label for a `known` identity read from live game memory.
pub const SOURCE_LIVE_MEMORY: &str = "live_memory";

/// Resolution state of the v1 identity binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignIdentityState {
    /// Both components present and canonical; `campaign_id` derived.
    Known,
    /// The identity source (d68) was observed, but the components are not
    /// sourced / torn / malformed. Fail-closed null.
    Unavailable,
    /// No d68 identity-source row was observed at all. Fail-closed null.
    Unbound,
    /// A transition comparison between two complete stable observations found
    /// exactly one changed component. Never used for a malformed/missing read.
    Conflict,
}

impl CampaignIdentityState {
    /// Stable wire string for the manifest `state` field.
    pub fn as_str(self) -> &'static str {
        match self {
            CampaignIdentityState::Known => "known",
            CampaignIdentityState::Unavailable => "unavailable",
            CampaignIdentityState::Unbound => "unbound",
            CampaignIdentityState::Conflict => "identity_component_conflict",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "known" => CampaignIdentityState::Known,
            "unavailable" => CampaignIdentityState::Unavailable,
            "unbound" => CampaignIdentityState::Unbound,
            "identity_component_conflict" => CampaignIdentityState::Conflict,
            _ => return None,
        })
    }
}

/// Raw compatibility witnesses carried BESIDE the identity for conflict
/// adjudication. These are NEVER substituted for the identity components (design
/// "Ranked candidates": start date, session, scenario, roster, and modset are
/// witnesses only). Only the two the recorder can source at this revision
/// (`start_date_raw`, `session`, both from the d68 header) are ever populated;
/// the rest stay null until a future path sources them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignIdentityWitnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

/// Sealed, clone-safe evidence retained only in memory.
///
/// The payload bytes are private and immutable through the public API. The
/// binding digest covers those exact bytes plus every field of the object that
/// carries the proof. A clone is harmless: changing any public field makes the
/// digest fail to recompute and therefore revokes authorization fail-closed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityEvidenceProof {
    cid1_payload: Vec<u8>,
    binding_sha256: [u8; 32],
}

/// The identity-candidate observation extracted from the decoded d68 stream.
///
/// `game_unique_id` / `game_unique_seed` are the ONLY v1 identity components.
/// They are `None` at this revision (the wire schema does not carry them); a
/// future collector change that sources them populates these fields and the
/// resolver then produces a `known` binding with zero further wiring.
#[derive(Debug, Clone, Default)]
pub struct SessionHeaderIdentityObservation {
    /// Count of d68 records routed to the decoder, including malformed or
    /// failure-status records. Zero means the source domain was entirely absent.
    pub session_header_records_seen: usize,
    /// Count of decoded d68 `session_header_live` data records. Zero means the
    /// decoder committed no complete effective-success row.
    pub session_header_rows: usize,
    /// Canonical (or raw) engine `game_unique_id`, if the stream ever sources it.
    pub game_unique_id: Option<String>,
    /// Exact engine `game_unique_seed` as a base-10 string (no cast/truncation).
    pub game_unique_seed: Option<String>,
    /// Valid game hour bracketing the identity read (design stamping contract).
    pub observed_game_hour_raw: Option<i32>,
    /// Build-profile id that sourced (or failed to source) the identity.
    pub source_build_profile: Option<String>,
    /// Number of matching, complete pair observations that bracket admission.
    /// Version 1 requires at least two; a single canonical pair is never enough.
    pub stable_identity_observations: u32,
    /// SHA-256 of the complete CID1 payload, set only while sealing validated
    /// payload bytes and covered by the private evidence proof.
    pub verified_stream_header_sha256: Option<String>,
    /// Private authorization bound to the exact current observation fields and
    /// the immutable CID1 payload bytes. External callers cannot mint it.
    evidence_proof: Option<IdentityEvidenceProof>,
    /// Raw compatibility witnesses (never substituted for identity).
    pub witnesses: CampaignIdentityWitnesses,
}

/// A resolved, first-class campaign identity binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignIdentity {
    pub state: CampaignIdentityState,
    /// 64 lowercase hex chars; `Some` only when `state == Known`.
    pub campaign_id: Option<String>,
    pub game_unique_id: Option<String>,
    pub game_unique_seed: Option<String>,
    /// `live_memory` only when `Known`; `None` otherwise (nothing was bound).
    pub source: Option<String>,
    pub source_build_profile: Option<String>,
    pub observed_game_hour_raw: Option<i32>,
    /// SHA-256 of the complete CID1 stream-header payload. `None` at this
    /// revision (no CID1 record type exists yet; filled by the future decoder).
    pub stream_header_sha256: Option<String>,
    pub witnesses: CampaignIdentityWitnesses,
    /// Human-readable reason for any non-`Known` state. Always `Some` when the
    /// identity is not bound; `None` only when `Known`.
    pub resolution_note: Option<String>,
    /// In-memory authorization bound to the exact current identity fields and
    /// immutable CID1 bytes. It is private and never serialized: neither
    /// manifest metadata nor a mutated clone can mint a stitchable identity.
    evidence_proof: Option<IdentityEvidenceProof>,
}

impl CampaignIdentity {
    /// True only when the identity is fully bound (`state == Known`).
    pub fn is_bound(&self) -> bool {
        self.state == CampaignIdentityState::Known
            && self.evidence_proof.as_ref().is_some_and(|proof| {
                proof.binding_sha256 == campaign_identity_evidence_digest(self, &proof.cid1_payload)
                    && payload_hash_matches(
                        self.stream_header_sha256.as_deref(),
                        &proof.cid1_payload,
                    )
            })
    }
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_optional_bytes(hasher: &mut Sha256, value: Option<&[u8]>) {
    match value {
        Some(bytes) => {
            hasher.update(&[1]);
            hash_bytes(hasher, bytes);
        }
        None => hasher.update(&[0]),
    }
}

fn hash_optional_str(hasher: &mut Sha256, value: Option<&str>) {
    hash_optional_bytes(hasher, value.map(str::as_bytes));
}

fn hash_optional_u32(hasher: &mut Sha256, value: Option<u32>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value.to_be_bytes());
        }
        None => hasher.update(&[0]),
    }
}

fn hash_optional_i32(hasher: &mut Sha256, value: Option<i32>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value.to_be_bytes());
        }
        None => hasher.update(&[0]),
    }
}

fn hash_witnesses(hasher: &mut Sha256, witnesses: &CampaignIdentityWitnesses) {
    hash_optional_u32(hasher, witnesses.start_date_raw);
    hash_optional_u32(hasher, witnesses.session);
    hash_optional_str(hasher, witnesses.scenario.as_deref());
    match witnesses.initial_human_country_tags.as_deref() {
        Some(tags) => {
            hasher.update(&[1]);
            hasher.update(&(tags.len() as u64).to_be_bytes());
            for tag in tags {
                hash_bytes(hasher, tag.as_bytes());
            }
        }
        None => hasher.update(&[0]),
    }
    hash_optional_str(hasher, witnesses.modset_identity.as_deref());
}

fn observation_evidence_digest(
    obs: &SessionHeaderIdentityObservation,
    cid1_payload: &[u8],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EVIDENCE_BINDING_PREFIX);
    hash_bytes(&mut hasher, b"observation");
    hasher.update(&(obs.session_header_records_seen as u64).to_be_bytes());
    hasher.update(&(obs.session_header_rows as u64).to_be_bytes());
    hash_optional_str(&mut hasher, obs.game_unique_id.as_deref());
    hash_optional_str(&mut hasher, obs.game_unique_seed.as_deref());
    hash_optional_i32(&mut hasher, obs.observed_game_hour_raw);
    hash_optional_str(&mut hasher, obs.source_build_profile.as_deref());
    hasher.update(&obs.stable_identity_observations.to_be_bytes());
    hash_optional_str(&mut hasher, obs.verified_stream_header_sha256.as_deref());
    hash_witnesses(&mut hasher, &obs.witnesses);
    hash_bytes(&mut hasher, cid1_payload);
    hasher.finalize()
}

fn campaign_identity_evidence_digest(identity: &CampaignIdentity, cid1_payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(EVIDENCE_BINDING_PREFIX);
    hash_bytes(&mut hasher, b"resolved-identity");
    hash_bytes(&mut hasher, identity.state.as_str().as_bytes());
    hash_optional_str(&mut hasher, identity.campaign_id.as_deref());
    hash_optional_str(&mut hasher, identity.game_unique_id.as_deref());
    hash_optional_str(&mut hasher, identity.game_unique_seed.as_deref());
    hash_optional_str(&mut hasher, identity.source.as_deref());
    hash_optional_str(&mut hasher, identity.source_build_profile.as_deref());
    hash_optional_i32(&mut hasher, identity.observed_game_hour_raw);
    hash_optional_str(&mut hasher, identity.stream_header_sha256.as_deref());
    hash_witnesses(&mut hasher, &identity.witnesses);
    hash_optional_str(&mut hasher, identity.resolution_note.as_deref());
    hash_bytes(&mut hasher, cid1_payload);
    hasher.finalize()
}

fn payload_hash_matches(stored_hash: Option<&str>, cid1_payload: &[u8]) -> bool {
    stored_hash == Some(to_hex(&Sha256::digest(cid1_payload)).as_str())
}

/// Seal an observation only after the stateful decoder has validated these
/// exact CID1 bytes and established two matching complete observations.
///
/// This remains private until CID1 exists. The future decoder belongs in this
/// module and must pass its validated payload bytes here; candidate fields or a
/// caller-supplied hash alone can never create authorization.
#[allow(dead_code)]
fn seal_verified_observation_evidence(
    obs: &mut SessionHeaderIdentityObservation,
    cid1_payload: &[u8],
) -> Result<(), &'static str> {
    obs.evidence_proof = None;
    obs.verified_stream_header_sha256 = None;

    if cid1_payload.is_empty() {
        return Err("CID1 payload is empty");
    }
    if obs
        .game_unique_id
        .as_deref()
        .and_then(canonical_uuid)
        .is_none()
        || obs
            .game_unique_seed
            .as_deref()
            .and_then(canonical_seed)
            .is_none()
    {
        return Err("CID1 identity components are missing or malformed");
    }
    if obs.stable_identity_observations < 2 {
        return Err("fewer than two matching identity observations");
    }
    if obs
        .observed_game_hour_raw
        .and_then(hour_index_from_raw)
        .is_none()
    {
        return Err("CID1 observation has no valid bracketing game hour");
    }
    if !obs
        .source_build_profile
        .as_deref()
        .is_some_and(valid_profile_id)
    {
        return Err("CID1 observation has no valid source build profile");
    }

    obs.verified_stream_header_sha256 = Some(to_hex(&Sha256::digest(cid1_payload)));
    obs.evidence_proof = Some(IdentityEvidenceProof {
        cid1_payload: cid1_payload.to_vec(),
        binding_sha256: observation_evidence_digest(obs, cid1_payload),
    });
    Ok(())
}

/// Validate + canonicalize a `game_unique_id` into lowercase `8-4-4-4-12` hex.
/// Accepts any input case; rejects anything not shaped like a UUID.
pub fn canonical_uuid(raw: &str) -> Option<String> {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() != groups.len() {
        return None;
    }
    let mut out = String::with_capacity(36);
    for (part, &want) in parts.iter().zip(groups.iter()) {
        if part.len() != want || !part.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&part.to_ascii_lowercase());
    }
    Some(out)
}

/// Validate + canonicalize a `game_unique_seed` as a base-10 integer string.
/// Rejects empty, non-digit, leading-zero (`007`), and negative-zero (`-0`)
/// forms so the canonical representation is unique.
pub fn canonical_seed(raw: &str) -> Option<String> {
    let (neg, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // No leading zeros unless the value is exactly "0"; no "-0".
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    if neg && digits == "0" {
        return None;
    }
    Some(if neg {
        format!("-{digits}")
    } else {
        digits.to_string()
    })
}

/// SHA-256 of the canonical preimage, lowercase hex (design "Canonical preimage").
pub fn compute_campaign_id(canonical_uuid: &str, canonical_seed: &str) -> String {
    let mut buf = Vec::with_capacity(
        CID_PREIMAGE_PREFIX.len() + canonical_uuid.len() + 1 + canonical_seed.len(),
    );
    buf.extend_from_slice(CID_PREIMAGE_PREFIX);
    buf.extend_from_slice(canonical_uuid.as_bytes());
    buf.push(0x00);
    buf.extend_from_slice(canonical_seed.as_bytes());
    to_hex(&Sha256::digest(&buf))
}

/// Resolve an observation into a fail-closed identity binding.
///
/// `Known` requires the complete canonical pair plus two matching observations,
/// a valid bracketing hour, a non-empty build profile, and the verified CID1
/// payload hash. Missing/malformed input is `Unavailable`; component conflict is
/// reserved for comparing two complete stable observations during transition.
pub fn resolve(obs: &SessionHeaderIdentityObservation) -> CampaignIdentity {
    let canon_uuid = obs.game_unique_id.as_deref().and_then(canonical_uuid);
    let canon_seed = obs.game_unique_seed.as_deref().and_then(canonical_seed);

    let base = |state: CampaignIdentityState, note: Option<String>| CampaignIdentity {
        state,
        campaign_id: None,
        game_unique_id: None,
        game_unique_seed: None,
        source: None,
        source_build_profile: obs.source_build_profile.clone(),
        observed_game_hour_raw: None,
        stream_header_sha256: None,
        witnesses: obs.witnesses.clone(),
        resolution_note: note,
        evidence_proof: None,
    };

    match (canon_uuid.clone(), canon_seed.clone()) {
        (Some(uuid), Some(seed)) => {
            let hour = obs
                .observed_game_hour_raw
                .filter(|raw| hour_index_from_raw(*raw).is_some());
            let profile = obs
                .source_build_profile
                .as_deref()
                .filter(|profile| valid_profile_id(profile));
            let cid1_hash = obs
                .verified_stream_header_sha256
                .as_deref()
                .filter(|hash| is_lower_hex64(hash));
            let evidence_verified = obs.evidence_proof.as_ref().is_some_and(|proof| {
                proof.binding_sha256 == observation_evidence_digest(obs, &proof.cid1_payload)
                    && payload_hash_matches(cid1_hash, &proof.cid1_payload)
            });
            if !evidence_verified
                || obs.stable_identity_observations < 2
                || hour.is_none()
                || profile.is_none()
                || cid1_hash.is_none()
            {
                return base(
                    CampaignIdentityState::Unavailable,
                    Some(format!(
                        "canonical identity pair observed but binding prerequisites are incomplete: \
                         evidence_bytes_verified={}, stable_observations={} (need >=2), \
                         valid_bracketing_hour={}, valid_source_build_profile={}, \
                         verified_cid1_payload_hash={} \
                         (fail-closed; no campaign_id derived)",
                        evidence_verified,
                        obs.stable_identity_observations,
                        hour.is_some(),
                        profile.is_some(),
                        cid1_hash.is_some()
                    )),
                );
            }
            let campaign_id = compute_campaign_id(&uuid, &seed);
            let mut identity = CampaignIdentity {
                state: CampaignIdentityState::Known,
                campaign_id: Some(campaign_id),
                game_unique_id: Some(uuid),
                game_unique_seed: Some(seed),
                source: Some(SOURCE_LIVE_MEMORY.to_string()),
                source_build_profile: obs.source_build_profile.clone(),
                observed_game_hour_raw: hour,
                stream_header_sha256: cid1_hash.map(str::to_owned),
                witnesses: obs.witnesses.clone(),
                resolution_note: None,
                evidence_proof: None,
            };
            let cid1_payload = &obs.evidence_proof.as_ref().unwrap().cid1_payload;
            let binding_sha256 = campaign_identity_evidence_digest(&identity, cid1_payload);
            identity.evidence_proof = Some(IdentityEvidenceProof {
                cid1_payload: cid1_payload.clone(),
                binding_sha256,
            });
            identity
        }
        // A partial or malformed observation is uncertainty, not a transition
        // conflict. Conflict exists only when two complete stable observations
        // are compared and exactly one canonical component changed.
        _ if obs.game_unique_id.is_some() || obs.game_unique_seed.is_some() => {
            let uuid_state = match (&obs.game_unique_id, &canon_uuid) {
                (None, _) => "missing",
                (Some(_), None) => "malformed",
                (Some(_), Some(_)) => "canonical",
            };
            let seed_state = match (&obs.game_unique_seed, &canon_seed) {
                (None, _) => "missing",
                (Some(_), None) => "malformed",
                (Some(_), Some(_)) => "canonical",
            };
            base(
                CampaignIdentityState::Unavailable,
                Some(format!(
                    "campaign identity components unavailable: game_unique_id={uuid_state}, \
                     game_unique_seed={seed_state}; both complete canonical components are \
                     mandatory (fail-closed, not a transition conflict)"
                )),
            )
        }
        // Neither component present.
        _ => {
            if obs.session_header_records_seen == 0 && obs.session_header_rows == 0 {
                base(
                    CampaignIdentityState::Unbound,
                    Some(
                        "no d68 session_header_live row observed; the campaign identity \
                         source domain was entirely absent (fail-closed, unstitchable)"
                            .to_string(),
                    ),
                )
            } else {
                base(
                    CampaignIdentityState::Unavailable,
                    Some(format!(
                        "d68 session_header_live observed ({} row(s)) but does not source \
                         game_unique_id/game_unique_seed at this build revision; both components \
                         are mandatory and absent (fail-closed pending offset-proven collector \
                         reader -- see CAMPAIGN_IDENTITY_DESIGN.md open items)",
                        obs.session_header_rows
                    )),
                )
            }
        }
    }
}

fn witnesses_json(w: &CampaignIdentityWitnesses) -> Value {
    json!({
        "start_date_raw": w.start_date_raw.map(|v| json!(v)).unwrap_or(Value::Null),
        "session": w.session.map(|v| json!(v)).unwrap_or(Value::Null),
        "scenario": w.scenario.clone().map(Value::String).unwrap_or(Value::Null),
        "initial_human_country_tags": w
            .initial_human_country_tags
            .clone()
            .map(|tags| Value::Array(tags.into_iter().map(Value::String).collect()))
            .unwrap_or(Value::Null),
        "modset_identity": w.modset_identity.clone().map(Value::String).unwrap_or(Value::Null),
    })
}

/// Serialize a resolved identity into the exact design manifest object.
pub fn to_manifest_json(id: &CampaignIdentity) -> Value {
    json!({
        "schema": CAMPAIGN_IDENTITY_SCHEMA,
        "state": id.state.as_str(),
        "campaign_id": id.campaign_id.clone().map(Value::String).unwrap_or(Value::Null),
        "game_unique_id": id.game_unique_id.clone().map(Value::String).unwrap_or(Value::Null),
        "game_unique_seed": id.game_unique_seed.clone().map(Value::String).unwrap_or(Value::Null),
        "source": id.source.clone().map(Value::String).unwrap_or(Value::Null),
        "source_build_profile": id.source_build_profile.clone().map(Value::String).unwrap_or(Value::Null),
        "observed_game_hour_raw": id.observed_game_hour_raw.map(|v| json!(v)).unwrap_or(Value::Null),
        "stream_header_sha256": id.stream_header_sha256.clone().map(Value::String).unwrap_or(Value::Null),
        "resolution_note": id.resolution_note.clone().map(Value::String).unwrap_or(Value::Null),
        "witnesses": witnesses_json(&id.witnesses),
    })
}

// --- Closed-grammar manifest validator (drift guard) ----------------------
//
// Style mirrors `main.rs::validate_repack_manifest_modset`: the accepted grammar
// is exactly the serializer's output. An unexpected key at any level, a
// state/field-shape mismatch, or a digest that does not recompute is a shape the
// recorder never wrote and fails the repack closed. This terminates manifest
// drift by construction (design: "derive and hash-bind it from the first record;
// mismatch is fatal").

const CAMPAIGN_IDENTITY_KEYS: &[&str] = &[
    "schema",
    "state",
    "campaign_id",
    "game_unique_id",
    "game_unique_seed",
    "source",
    "source_build_profile",
    "observed_game_hour_raw",
    "stream_header_sha256",
    "resolution_note",
    "witnesses",
];

const WITNESS_KEYS: &[&str] = &[
    "start_date_raw",
    "session",
    "scenario",
    "initial_human_country_tags",
    "modset_identity",
];

fn check_keys(map: &Map<String, Value>, allowed: &[&str], what: &str) -> Result<(), String> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "campaign_identity {what} carries unexpected key `{key}` (not a recorder-written shape)"
            ));
        }
    }
    Ok(())
}

fn is_lower_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn valid_profile_id(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_control)
}

fn nullable_u32(value: &Value, name: &str) -> Result<Option<u32>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_u64()
        .ok_or_else(|| format!("campaign_identity `{name}` must be null or a u32 integer"))?;
    u32::try_from(raw)
        .map(Some)
        .map_err(|_| format!("campaign_identity `{name}` integer {raw} exceeds u32 range"))
}

fn nullable_string<'a>(value: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        value
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("campaign_identity `{name}` must be null or a string"))
    }
}

fn nullable_profile(value: &Value) -> Result<Option<&str>, String> {
    let profile = nullable_string(value, "source_build_profile")?;
    if profile.is_some_and(|value| !valid_profile_id(value)) {
        return Err(
            "campaign_identity `source_build_profile` must be non-empty and contain no controls"
                .into(),
        );
    }
    Ok(profile)
}

fn nullable_game_hour(value: &Value) -> Result<Option<i32>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let raw = value
        .as_i64()
        .ok_or("campaign_identity `observed_game_hour_raw` must be null or an i32 integer")?;
    let raw = i32::try_from(raw).map_err(|_| {
        format!("campaign_identity `observed_game_hour_raw` integer {raw} exceeds i32 range")
    })?;
    if hour_index_from_raw(raw).is_none() {
        return Err(format!(
            "campaign_identity `observed_game_hour_raw` {raw} is not a valid game hour"
        ));
    }
    Ok(Some(raw))
}

fn nullable_roster(value: &Value) -> Result<Option<Vec<String>>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let rows = value.as_array().ok_or(
        "campaign_identity `initial_human_country_tags` must be null or an array of strings",
    )?;
    rows.iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                format!("campaign_identity `initial_human_country_tags[{index}]` is not a string")
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Validate a `campaign_identity` object's exact shape and internal consistency.
pub fn validate_manifest_object(value: &Value) -> Result<(), String> {
    let obj = value
        .as_object()
        .ok_or("campaign_identity is not an object")?;
    check_keys(obj, CAMPAIGN_IDENTITY_KEYS, "object")?;

    for key in CAMPAIGN_IDENTITY_KEYS {
        if !obj.contains_key(*key) {
            return Err(format!("campaign_identity missing key `{key}`"));
        }
    }

    let schema = obj.get("schema").and_then(Value::as_str);
    if schema != Some(CAMPAIGN_IDENTITY_SCHEMA) {
        return Err(format!(
            "campaign_identity schema is {schema:?}, expected {CAMPAIGN_IDENTITY_SCHEMA:?}"
        ));
    }
    let state_str = obj
        .get("state")
        .and_then(Value::as_str)
        .ok_or("campaign_identity state is not a string")?;
    let state = CampaignIdentityState::from_str(state_str)
        .ok_or_else(|| format!("campaign_identity unknown state `{state_str}`"))?;

    let witnesses = obj
        .get("witnesses")
        .and_then(Value::as_object)
        .ok_or("campaign_identity witnesses is not an object")?;
    check_keys(witnesses, WITNESS_KEYS, "witnesses")?;
    for key in WITNESS_KEYS {
        if !witnesses.contains_key(*key) {
            return Err(format!("campaign_identity witnesses missing key `{key}`"));
        }
    }
    nullable_u32(&witnesses["start_date_raw"], "witnesses.start_date_raw")?;
    nullable_u32(&witnesses["session"], "witnesses.session")?;
    nullable_string(&witnesses["scenario"], "witnesses.scenario")?;
    nullable_roster(&witnesses["initial_human_country_tags"])?;
    nullable_string(&witnesses["modset_identity"], "witnesses.modset_identity")?;

    let source_build_profile = nullable_profile(&obj["source_build_profile"])?;
    let observed_game_hour_raw = nullable_game_hour(&obj["observed_game_hour_raw"])?;

    let campaign_id = obj.get("campaign_id").unwrap();
    let game_unique_id = obj.get("game_unique_id").unwrap();
    let game_unique_seed = obj.get("game_unique_seed").unwrap();
    let source = obj.get("source").unwrap();
    let stream_header_sha256 = obj.get("stream_header_sha256").unwrap();
    let resolution_note = obj.get("resolution_note").unwrap();

    match state {
        CampaignIdentityState::Known => {
            let cid = campaign_id
                .as_str()
                .ok_or("campaign_identity known state requires a string campaign_id")?;
            if !is_lower_hex64(cid) {
                return Err("campaign_identity campaign_id is not 64 lowercase hex".into());
            }
            let uuid = game_unique_id
                .as_str()
                .ok_or("campaign_identity known state requires a string game_unique_id")?;
            let canon_uuid = canonical_uuid(uuid)
                .ok_or("campaign_identity game_unique_id is not a canonical UUID")?;
            if canon_uuid != uuid {
                return Err("campaign_identity game_unique_id is not in canonical form".into());
            }
            let seed = game_unique_seed
                .as_str()
                .ok_or("campaign_identity known state requires a string game_unique_seed")?;
            let canon_seed = canonical_seed(seed)
                .ok_or("campaign_identity game_unique_seed is not a canonical base-10 integer")?;
            if canon_seed != seed {
                return Err("campaign_identity game_unique_seed is not in canonical form".into());
            }
            // Digest must recompute exactly (design: mismatch is fatal).
            let recomputed = compute_campaign_id(&canon_uuid, &canon_seed);
            if recomputed != cid {
                return Err(
                    "campaign_identity campaign_id does not recompute from its components \
                     (corruption/collision)"
                        .into(),
                );
            }
            if source.as_str() != Some(SOURCE_LIVE_MEMORY) {
                return Err(format!(
                    "campaign_identity known state requires source={SOURCE_LIVE_MEMORY:?}"
                ));
            }
            if source_build_profile.is_none() {
                return Err(
                    "campaign_identity known state requires a valid source_build_profile".into(),
                );
            }
            if observed_game_hour_raw.is_none() {
                return Err(
                    "campaign_identity known state requires a valid observed_game_hour_raw".into(),
                );
            }
            let cid1_hash = stream_header_sha256.as_str().ok_or(
                "campaign_identity known state requires a verified CID1 stream_header_sha256",
            )?;
            if !is_lower_hex64(cid1_hash) {
                return Err(
                    "campaign_identity stream_header_sha256 is not 64 lowercase hex".into(),
                );
            }
            if !resolution_note.is_null() {
                return Err("campaign_identity known state must have null resolution_note".into());
            }
        }
        CampaignIdentityState::Unavailable
        | CampaignIdentityState::Unbound
        | CampaignIdentityState::Conflict => {
            // Fail-closed: identity fields MUST be null and a reason recorded.
            for (name, v) in [
                ("campaign_id", campaign_id),
                ("game_unique_id", game_unique_id),
                ("game_unique_seed", game_unique_seed),
                ("source", source),
                ("stream_header_sha256", stream_header_sha256),
            ] {
                if !v.is_null() {
                    return Err(format!(
                        "campaign_identity state `{state_str}` must have null `{name}` (never fabricate identity)"
                    ));
                }
            }
            if observed_game_hour_raw.is_some() {
                return Err(format!(
                    "campaign_identity state `{state_str}` must have null \
                     `observed_game_hour_raw` (no positive identity hour without CID1)"
                ));
            }
            match resolution_note.as_str() {
                Some(s) if !s.is_empty() => {}
                _ => {
                    return Err(format!(
                        "campaign_identity state `{state_str}` requires a non-empty resolution_note"
                    ))
                }
            }
        }
    }
    Ok(())
}

/// Tolerant wrapper for the pack/repack path: a session manifest WITHOUT a
/// `campaign_identity` key is a legacy stream (identity-unavailable, unstitchable
/// -- design). When the key is present it must satisfy the exact grammar.
pub fn validate_in_session_manifest(manifest: &Value) -> Result<(), String> {
    match manifest.get("campaign_identity") {
        None => Ok(()),
        Some(value) => validate_manifest_object(value),
    }
}

/// Reconstruct a non-bound `CampaignIdentity` from stored manifest metadata.
///
/// A `known` manifest is shape-validated but cannot authorize stitching: this
/// function has no CID1 payload from which to recompute the stored hash. The
/// future stateful CID1 decoder must call `resolve` from its verified observation
/// rather than treating parallel JSON metadata as stream truth.
pub fn from_manifest_json(value: &Value) -> Result<CampaignIdentity, String> {
    validate_manifest_object(value)?;
    let obj = value.as_object().unwrap();
    let state = CampaignIdentityState::from_str(obj["state"].as_str().unwrap()).unwrap();
    if state == CampaignIdentityState::Known {
        return Err(
            "campaign_identity known manifest cannot authorize binding without the validated \
             CID1 payload (manifest metadata is not stream truth)"
                .into(),
        );
    }
    let str_field = |k: &str| obj.get(k).and_then(Value::as_str).map(|s| s.to_string());
    let witnesses_obj = obj["witnesses"].as_object().unwrap();
    let witnesses = CampaignIdentityWitnesses {
        start_date_raw: nullable_u32(&witnesses_obj["start_date_raw"], "witnesses.start_date_raw")?,
        session: nullable_u32(&witnesses_obj["session"], "witnesses.session")?,
        scenario: nullable_string(&witnesses_obj["scenario"], "witnesses.scenario")?
            .map(str::to_owned),
        initial_human_country_tags: nullable_roster(&witnesses_obj["initial_human_country_tags"])?,
        modset_identity: nullable_string(
            &witnesses_obj["modset_identity"],
            "witnesses.modset_identity",
        )?
        .map(str::to_owned),
    };
    Ok(CampaignIdentity {
        state,
        campaign_id: str_field("campaign_id"),
        game_unique_id: str_field("game_unique_id"),
        game_unique_seed: str_field("game_unique_seed"),
        source: str_field("source"),
        source_build_profile: str_field("source_build_profile"),
        observed_game_hour_raw: nullable_game_hour(&obj["observed_game_hour_raw"])?,
        stream_header_sha256: str_field("stream_header_sha256"),
        witnesses,
        resolution_note: str_field("resolution_note"),
        evidence_proof: None,
    })
}

// --- Stitching consumer interface -----------------------------------------
//
// The reload/relaunch stitcher (STITCHING_DESIGN.md is absent at this base; this
// interface is defined per CAMPAIGN_IDENTITY_DESIGN.md "Read and transition
// semantics" and recorded in the W12C dossier) asks exactly one question of two
// containers: may their streams be joined as the same persisted playthrough?
//
// The answer is fail-closed: two streams stitch ONLY when both identities are
// `Known` and equal on the whole pair. An unavailable/unbound/conflict identity
// on either side is never auto-stitched (design: legacy identity-unavailable
// streams "cannot be stitched automatically across sessions").

/// The stitcher's decision for a pair of container identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchDecision {
    /// Both bound and identical -> same persisted playthrough; may join.
    SameCampaign,
    /// Both bound, whole pair differs -> distinct campaigns; must NOT join.
    DifferentCampaign,
    /// Both bound but exactly one raw component differs -> conflict; stop auto
    /// stitching and surface both observations (design transition semantics).
    ComponentConflict,
    /// At least one side is not bound -> identity-unavailable; never auto-join.
    IdentityUnavailable,
}

/// Decide whether two resolved identities may be stitched as one campaign.
pub fn stitch_decision(a: &CampaignIdentity, b: &CampaignIdentity) -> StitchDecision {
    if !a.is_bound() || !b.is_bound() {
        return StitchDecision::IdentityUnavailable;
    }
    // Both Known: campaign_id is present and derived from the components.
    if a.campaign_id == b.campaign_id
        && a.game_unique_id == b.game_unique_id
        && a.game_unique_seed == b.game_unique_seed
    {
        return StitchDecision::SameCampaign;
    }
    // Exactly one component equal -> component conflict, not a clean split.
    let uuid_eq = a.game_unique_id == b.game_unique_id;
    let seed_eq = a.game_unique_seed == b.game_unique_seed;
    if uuid_eq != seed_eq {
        StitchDecision::ComponentConflict
    } else {
        StitchDecision::DifferentCampaign
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified_obs(uuid: &str, seed: &str) -> SessionHeaderIdentityObservation {
        let mut observation = SessionHeaderIdentityObservation {
            session_header_records_seen: 3,
            session_header_rows: 3,
            game_unique_id: Some(uuid.to_string()),
            game_unique_seed: Some(seed.to_string()),
            observed_game_hour_raw: Some(
                compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24,
            ),
            source_build_profile: Some("profile-x".to_string()),
            stable_identity_observations: 2,
            verified_stream_header_sha256: None,
            evidence_proof: None,
            witnesses: CampaignIdentityWitnesses {
                start_date_raw: Some(1_000),
                session: Some(7),
                ..Default::default()
            },
        };
        let cid1_payload = format!("CID1\0uuid={uuid}\0seed={seed}\0profile=profile-x");
        seal_verified_observation_evidence(&mut observation, cid1_payload.as_bytes()).unwrap();
        observation
    }

    fn known_obs() -> SessionHeaderIdentityObservation {
        verified_obs("A4B17FED-ED12-4361-9064-FB6576638110", "1026046527")
    }

    #[test]
    fn canonical_uuid_lowercases_and_validates() {
        assert_eq!(
            canonical_uuid("A4B17FED-ED12-4361-9064-FB6576638110").as_deref(),
            Some("a4b17fed-ed12-4361-9064-fb6576638110")
        );
        assert!(canonical_uuid("not-a-uuid").is_none());
        assert!(canonical_uuid("a4b17fed-ed12-4361-9064-fb657663811").is_none()); // short
        assert!(canonical_uuid("g4b17fed-ed12-4361-9064-fb6576638110").is_none()); // non-hex
        assert!(canonical_uuid("a4b17fed-ed12-4361-9064-fb6576638110-00").is_none());
        // extra group
    }

    #[test]
    fn canonical_seed_rules() {
        assert_eq!(canonical_seed("1026046527").as_deref(), Some("1026046527"));
        assert_eq!(
            canonical_seed("-1033122218").as_deref(),
            Some("-1033122218")
        );
        assert_eq!(canonical_seed("0").as_deref(), Some("0"));
        assert!(canonical_seed("007").is_none()); // leading zero
        assert!(canonical_seed("-0").is_none()); // negative zero
        assert!(canonical_seed("").is_none());
        assert!(canonical_seed("12a").is_none());
        assert!(canonical_seed("- 5").is_none());
    }

    #[test]
    fn digest_is_stable_and_uuid_case_insensitive() {
        let lower = compute_campaign_id("a4b17fed-ed12-4361-9064-fb6576638110", "1026046527");
        assert_eq!(
            lower,
            "b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f"
        );
        // Different seed -> different digest.
        let other = compute_campaign_id("a4b17fed-ed12-4361-9064-fb6576638110", "1026046528");
        assert_ne!(lower, other);
    }

    #[test]
    fn resolve_known_derives_digest() {
        let id = resolve(&known_obs());
        assert_eq!(id.state, CampaignIdentityState::Known);
        assert!(id.is_bound());
        assert_eq!(
            id.game_unique_id.as_deref(),
            Some("a4b17fed-ed12-4361-9064-fb6576638110")
        );
        assert_eq!(id.game_unique_seed.as_deref(), Some("1026046527"));
        assert_eq!(id.source.as_deref(), Some(SOURCE_LIVE_MEMORY));
        assert_eq!(
            id.campaign_id.as_deref(),
            Some(
                compute_campaign_id("a4b17fed-ed12-4361-9064-fb6576638110", "1026046527").as_str()
            )
        );
        assert!(id.resolution_note.is_none());
        // Shape-valid metadata alone must not reconstruct a stitchable Known
        // identity: without the actual CID1 bytes, its payload hash is unverified.
        let manifest = to_manifest_json(&id);
        validate_manifest_object(&manifest).unwrap();
        assert!(from_manifest_json(&manifest)
            .unwrap_err()
            .contains("CID1 payload"));
    }

    #[test]
    fn resolve_refuses_canonical_pair_without_cid1_stability_evidence() {
        let mut observation = known_obs();
        observation.stable_identity_observations = 1;
        observation.verified_stream_header_sha256 = None;
        let id = resolve(&observation);
        assert_eq!(id.state, CampaignIdentityState::Unavailable);
        assert!(!id.is_bound());
        assert!(id.campaign_id.is_none());
        assert!(id.stream_header_sha256.is_none());
    }

    #[test]
    fn resolve_refuses_syntax_only_cid1_hash_without_evidence_bytes() {
        let mut observation = known_obs();
        observation.evidence_proof = None;
        observation.verified_stream_header_sha256 = Some("c".repeat(64));
        let id = resolve(&observation);
        assert_eq!(id.state, CampaignIdentityState::Unavailable);
        assert!(!id.is_bound());
        assert!(id.campaign_id.is_none());
    }

    #[test]
    fn resolve_unavailable_when_rows_but_no_components() {
        let obs = SessionHeaderIdentityObservation {
            session_header_rows: 5,
            witnesses: CampaignIdentityWitnesses {
                start_date_raw: Some(1),
                session: Some(2),
                ..Default::default()
            },
            ..Default::default()
        };
        let id = resolve(&obs);
        assert_eq!(id.state, CampaignIdentityState::Unavailable);
        assert!(!id.is_bound());
        assert!(id.campaign_id.is_none());
        assert!(id.game_unique_id.is_none());
        assert!(id.resolution_note.as_deref().unwrap().contains("mandatory"));
        // Witnesses survive beside the null identity.
        assert_eq!(id.witnesses.start_date_raw, Some(1));
        assert_eq!(id.witnesses.session, Some(2));
        validate_manifest_object(&to_manifest_json(&id)).unwrap();
    }

    #[test]
    fn resolve_unbound_when_no_rows() {
        let id = resolve(&SessionHeaderIdentityObservation::default());
        assert_eq!(id.state, CampaignIdentityState::Unbound);
        assert!(id.campaign_id.is_none());
        assert!(id.resolution_note.as_deref().unwrap().contains("no d68"));
        validate_manifest_object(&to_manifest_json(&id)).unwrap();
    }

    #[test]
    fn resolve_unavailable_on_single_component() {
        let obs = SessionHeaderIdentityObservation {
            session_header_rows: 1,
            game_unique_id: Some("a4b17fed-ed12-4361-9064-fb6576638110".to_string()),
            game_unique_seed: None,
            ..Default::default()
        };
        let id = resolve(&obs);
        assert_eq!(id.state, CampaignIdentityState::Unavailable);
        assert!(id.campaign_id.is_none());
        assert!(id.game_unique_id.is_none()); // never surface a half identity
        assert!(id
            .resolution_note
            .as_deref()
            .unwrap()
            .contains("game_unique_seed=missing"));
        validate_manifest_object(&to_manifest_json(&id)).unwrap();
    }

    #[test]
    fn resolve_unavailable_on_malformed_both() {
        let obs = SessionHeaderIdentityObservation {
            session_header_rows: 1,
            game_unique_id: Some("garbage".to_string()),
            game_unique_seed: Some("nan".to_string()),
            ..Default::default()
        };
        let id = resolve(&obs);
        assert_eq!(id.state, CampaignIdentityState::Unavailable);
        assert!(id.campaign_id.is_none());
        validate_manifest_object(&to_manifest_json(&id)).unwrap();
    }

    #[test]
    fn validator_rejects_unexpected_top_key() {
        let mut manifest = to_manifest_json(&resolve(&SessionHeaderIdentityObservation::default()));
        manifest
            .as_object_mut()
            .unwrap()
            .insert("smuggled".into(), json!(1));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("unexpected key"), "{err}");
    }

    #[test]
    fn validator_rejects_unexpected_witness_key() {
        let mut manifest = to_manifest_json(&resolve(&SessionHeaderIdentityObservation::default()));
        manifest["witnesses"]
            .as_object_mut()
            .unwrap()
            .insert("player_key".into(), json!("x"));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("unexpected key"), "{err}");
    }

    #[test]
    fn validator_rejects_fabricated_identity_on_unavailable() {
        let mut manifest = to_manifest_json(&resolve(&SessionHeaderIdentityObservation {
            session_header_rows: 1,
            ..Default::default()
        }));
        // Smuggle a fake campaign_id into a non-known state.
        manifest["campaign_id"] = json!("a".repeat(64));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("must have null"), "{err}");
    }

    #[test]
    fn validator_rejects_digest_mismatch_on_known() {
        let id = resolve(&known_obs());
        let mut manifest = to_manifest_json(&id);
        // Tamper the digest while leaving the components intact.
        manifest["campaign_id"] = json!("b".repeat(64));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("does not recompute"), "{err}");
    }

    #[test]
    fn validator_rejects_noncanonical_uuid_on_known() {
        let id = resolve(&known_obs());
        let mut manifest = to_manifest_json(&id);
        manifest["game_unique_id"] = json!("A4B17FED-ED12-4361-9064-FB6576638110"); // uppercase
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("canonical"), "{err}");
    }

    #[test]
    fn validator_requires_reason_note_when_unbound() {
        let mut manifest = to_manifest_json(&resolve(&SessionHeaderIdentityObservation::default()));
        manifest["resolution_note"] = Value::Null;
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("resolution_note"), "{err}");
    }

    #[test]
    fn validator_rejects_wrong_witness_types_and_ranges() {
        let base = to_manifest_json(&resolve(&SessionHeaderIdentityObservation {
            session_header_rows: 1,
            ..Default::default()
        }));
        let mutations = [
            (
                "start_date_raw overflow",
                "start_date_raw",
                json!(4_294_967_296u64),
            ),
            ("session negative", "session", json!(-1)),
            ("scenario number", "scenario", json!(7)),
            ("modset bool", "modset_identity", json!(true)),
            (
                "roster non-string",
                "initial_human_country_tags",
                json!(["GER", 7]),
            ),
        ];
        for (label, key, value) in mutations {
            let mut manifest = base.clone();
            manifest["witnesses"][key] = value;
            assert!(
                validate_manifest_object(&manifest).is_err(),
                "validator accepted {label}"
            );
        }
    }

    #[test]
    fn validator_rejects_out_of_range_hour_and_non_string_profile() {
        let mut unbound = to_manifest_json(&resolve(&SessionHeaderIdentityObservation::default()));
        unbound["observed_game_hour_raw"] = json!(i64::from(i32::MAX) + 1);
        assert!(validate_manifest_object(&unbound).is_err());

        let mut unavailable = to_manifest_json(&resolve(&SessionHeaderIdentityObservation {
            session_header_rows: 1,
            ..Default::default()
        }));
        unavailable["source_build_profile"] = json!(42);
        assert!(validate_manifest_object(&unavailable).is_err());
    }

    #[test]
    fn session_manifest_tolerates_absent_identity_but_strict_on_present() {
        // Legacy manifest: no campaign_identity key -> accepted (identity-unavailable).
        let legacy = json!({ "schema": "native_collector_session_v1" });
        validate_in_session_manifest(&legacy).unwrap();
        // Present-but-malformed -> rejected.
        let mut bad = json!({ "schema": "native_collector_session_v1" });
        bad.as_object_mut()
            .unwrap()
            .insert("campaign_identity".into(), json!({ "state": "known" }));
        assert!(validate_in_session_manifest(&bad).is_err());
    }

    #[test]
    fn documented_known_manifest_example_passes_validator() {
        let design = include_str!("../../../documents/CAMPAIGN_IDENTITY_DESIGN.md");
        let manifest_section = design
            .split("### Recorder/replay manifest")
            .nth(1)
            .expect("recorder/replay manifest section");
        let json_text = manifest_section
            .split("```json")
            .nth(1)
            .and_then(|text| text.split("```").next())
            .expect("campaign identity JSON example");
        let manifest: Value = serde_json::from_str(json_text).expect("valid example JSON");

        validate_manifest_object(&manifest["campaign_identity"])
            .expect("documented known manifest must pass the actual validator");
    }

    #[test]
    fn stitch_decision_matrix() {
        let a = resolve(&known_obs());
        // identical
        let b = resolve(&known_obs());
        assert_eq!(stitch_decision(&a, &b), StitchDecision::SameCampaign);
        // fully different
        let c = resolve(&verified_obs("11111111-1111-4111-8111-111111111111", "99"));
        assert_eq!(stitch_decision(&a, &c), StitchDecision::DifferentCampaign);
        // one component shared -> conflict
        let d = resolve(&verified_obs("a4b17fed-ed12-4361-9064-fb6576638110", "99"));
        assert_eq!(stitch_decision(&a, &d), StitchDecision::ComponentConflict);
        // unbound side -> never auto-join
        let unbound = resolve(&SessionHeaderIdentityObservation::default());
        assert_eq!(
            stitch_decision(&a, &unbound),
            StitchDecision::IdentityUnavailable
        );
        assert_eq!(
            stitch_decision(&unbound, &unbound),
            StitchDecision::IdentityUnavailable
        );
    }

    #[test]
    fn mutated_verified_clone_is_not_bound_or_stitchable() {
        let original = resolve(&known_obs());
        assert!(original.is_bound());

        let mutations = [
            ("game_unique_id", {
                let mut value = original.clone();
                value.game_unique_id = Some("11111111-1111-4111-8111-111111111111".to_string());
                value
            }),
            ("game_unique_seed", {
                let mut value = original.clone();
                value.game_unique_seed = Some("99".to_string());
                value
            }),
            ("campaign_id", {
                let mut value = original.clone();
                value.campaign_id = Some("d".repeat(64));
                value
            }),
            ("stream_header_sha256", {
                let mut value = original.clone();
                value.stream_header_sha256 = Some("d".repeat(64));
                value
            }),
        ];

        for (field, mutated) in mutations {
            assert!(!mutated.is_bound(), "mutated {field} remained bound");
            assert_eq!(
                stitch_decision(&mutated, &mutated),
                StitchDecision::IdentityUnavailable,
                "mutated {field} reached a stitchable decision"
            );
        }
    }

    #[test]
    fn mutated_verified_observation_clone_cannot_transfer_evidence() {
        let mut mutated = known_obs();
        mutated.game_unique_seed = Some("99".to_string());

        let resolved = resolve(&mutated);
        assert_eq!(resolved.state, CampaignIdentityState::Unavailable);
        assert!(!resolved.is_bound());
        assert!(resolved.campaign_id.is_none());
    }
}
