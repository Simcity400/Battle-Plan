use serde_json::{json, Map, Value};
use snapshot::sha256::{to_hex, Sha256};

use compact_contract::{
    calendar::hour_index_from_raw,
    campaign_identity::{Cid1Witnesses, ValidatedCid1},
};

use crate::stitching::{AppendTicket, CampaignIdentityResolution};

pub const CAMPAIGN_IDENTITY_SCHEMA: &str = "hoi4.campaign_identity.v1";

pub const CID_PREIMAGE_PREFIX: &[u8] = b"hoi4-campaign-id-v1\0";

const EVIDENCE_BINDING_PREFIX: &[u8] = b"hoi4-campaign-identity-evidence-v1\0";

pub const SOURCE_LIVE_MEMORY: &str = "live_memory";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignIdentityState {
    Known,
    Unavailable,
    Unbound,
    Conflict,
}

impl CampaignIdentityState {
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CampaignIdentityWitnesses {
    pub start_date_raw: Option<u32>,
    pub session: Option<u32>,
    pub scenario: Option<String>,
    pub initial_human_country_tags: Option<Vec<String>>,
    pub modset_identity: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IdentityEvidenceProof {
    cid1_payload: Vec<u8>,
    binding_sha256: [u8; 32],
}

#[derive(Debug, Clone, Default)]
pub struct SessionHeaderIdentityObservation {
    pub session_header_records_seen: usize,
    pub session_header_rows: usize,
    pub game_unique_id: Option<String>,
    pub game_unique_seed: Option<String>,
    pub observed_game_hour_raw: Option<i32>,
    pub source_build_profile: Option<String>,
    pub stable_identity_observations: u32,
    pub verified_stream_header_sha256: Option<String>,
    evidence_proof: Option<IdentityEvidenceProof>,
    pub witnesses: CampaignIdentityWitnesses,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignIdentity {
    state: CampaignIdentityState,
    campaign_id: Option<String>,
    game_unique_id: Option<String>,
    game_unique_seed: Option<String>,
    source: Option<String>,
    source_build_profile: Option<String>,
    observed_game_hour_raw: Option<i32>,
    stream_header_sha256: Option<String>,
    witnesses: CampaignIdentityWitnesses,
    resolution_note: Option<String>,
    evidence_proof: Option<IdentityEvidenceProof>,
}

impl CampaignIdentity {
    pub fn state(&self) -> CampaignIdentityState {
        self.state
    }

    pub fn campaign_id(&self) -> Option<&str> {
        self.campaign_id.as_deref()
    }

    pub fn game_unique_id(&self) -> Option<&str> {
        self.game_unique_id.as_deref()
    }

    pub fn game_unique_seed(&self) -> Option<&str> {
        self.game_unique_seed.as_deref()
    }

    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn source_build_profile(&self) -> Option<&str> {
        self.source_build_profile.as_deref()
    }

    pub fn observed_game_hour_raw(&self) -> Option<i32> {
        self.observed_game_hour_raw
    }

    pub fn stream_header_sha256(&self) -> Option<&str> {
        self.stream_header_sha256.as_deref()
    }

    pub fn witnesses(&self) -> &CampaignIdentityWitnesses {
        &self.witnesses
    }

    pub fn resolution_note(&self) -> Option<&str> {
        self.resolution_note.as_deref()
    }

    pub fn append_resolution_note(&mut self, reason: &str) -> Result<(), String> {
        if self.state == CampaignIdentityState::Known {
            return Err(
                "a known campaign identity has no resolution note to append a refusal to"
                    .to_string(),
            );
        }
        let reason = reason.trim();
        if reason.is_empty() {
            return Err("campaign identity refusal reason is empty".to_string());
        }
        self.resolution_note = Some(match self.resolution_note.take() {
            Some(existing) if !existing.trim().is_empty() => format!("{existing}; {reason}"),
            _ => reason.to_string(),
        });
        Ok(())
    }

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

pub fn resolve_validated_cid1(cid1: &ValidatedCid1) -> CampaignIdentity {
    let Cid1Witnesses {
        start_date_raw,
        session,
        scenario,
        initial_human_country_tags,
        modset_identity,
    } = cid1.witnesses().clone();
    let mut identity = CampaignIdentity {
        state: CampaignIdentityState::Known,
        campaign_id: Some(cid1.campaign_id_hex()),
        game_unique_id: Some(cid1.game_unique_id().to_owned()),
        game_unique_seed: Some(cid1.game_unique_seed().to_string()),
        source: Some(SOURCE_LIVE_MEMORY.to_owned()),
        source_build_profile: Some(cid1.source_build_profile().to_owned()),
        observed_game_hour_raw: Some(cid1.game_hour_raw()),
        stream_header_sha256: Some(cid1.payload_sha256_hex()),
        witnesses: CampaignIdentityWitnesses {
            start_date_raw,
            session,
            scenario,
            initial_human_country_tags,
            modset_identity,
        },
        resolution_note: None,
        evidence_proof: None,
    };
    identity.evidence_proof = Some(IdentityEvidenceProof {
        cid1_payload: cid1.payload().to_vec(),
        binding_sha256: campaign_identity_evidence_digest(&identity, cid1.payload()),
    });
    identity
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

pub fn canonical_seed(raw: &str) -> Option<String> {
    let (neg, digits) = match raw.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return None;
    }
    if neg && digits == "0" {
        return None;
    }
    let canonical = if neg {
        format!("-{digits}")
    } else {
        digits.to_string()
    };
    canonical.parse::<i32>().ok()?;
    Some(canonical)
}

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
                        "d68 session_header_live observed ({} row(s)) but its schema does not source \
                         game_unique_id/game_unique_seed; both components are mandatory and absent \
                         (fail-closed until a live-accepted collector observation supplies the pair)",
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

pub fn to_manifest_json(id: &CampaignIdentity) -> Value {
    if id.state == CampaignIdentityState::Known && !id.is_bound() {
        return json!({
            "schema": CAMPAIGN_IDENTITY_SCHEMA,
            "state": CampaignIdentityState::Unavailable.as_str(),
            "campaign_id": Value::Null,
            "game_unique_id": Value::Null,
            "game_unique_seed": Value::Null,
            "source": Value::Null,
            "source_build_profile": id.source_build_profile.clone().map(Value::String).unwrap_or(Value::Null),
            "observed_game_hour_raw": Value::Null,
            "stream_header_sha256": Value::Null,
            "resolution_note": "identity marked known but its private CID1 evidence proof does not recompute; manifest authority withheld",
            "witnesses": witnesses_json(&id.witnesses),
        });
    }
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

pub fn validate_in_session_manifest(manifest: &Value) -> Result<(), String> {
    match manifest.get("campaign_identity") {
        None => Ok(()),
        Some(value) => validate_manifest_object(value),
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchDecision {
    SameCampaign,
    DifferentCampaign,
    ComponentConflict,
    IdentityUnavailable,
}

pub fn stitch_decision(a: &CampaignIdentity, b: &CampaignIdentity) -> StitchDecision {
    if !a.is_bound() || !b.is_bound() {
        return StitchDecision::IdentityUnavailable;
    }
    if a.campaign_id == b.campaign_id
        && a.game_unique_id == b.game_unique_id
        && a.game_unique_seed == b.game_unique_seed
    {
        return StitchDecision::SameCampaign;
    }
    let uuid_eq = a.game_unique_id == b.game_unique_id;
    let seed_eq = a.game_unique_seed == b.game_unique_seed;
    if uuid_eq != seed_eq {
        StitchDecision::ComponentConflict
    } else {
        StitchDecision::DifferentCampaign
    }
}

pub const CAMPAIGN_IDENTITY_SCHEME_MAJOR: u16 = 1;

fn hex64_to_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

impl CampaignIdentity {
    fn campaign_key_bytes(&self) -> Option<Vec<u8>> {
        if !self.is_bound() {
            return None;
        }
        let uuid = canonical_uuid(self.game_unique_id.as_deref()?)?;
        let seed = canonical_seed(self.game_unique_seed.as_deref()?)?;
        let mut buf = Vec::with_capacity(CID_PREIMAGE_PREFIX.len() + uuid.len() + 1 + seed.len());
        buf.extend_from_slice(CID_PREIMAGE_PREFIX);
        buf.extend_from_slice(uuid.as_bytes());
        buf.push(0x00);
        buf.extend_from_slice(seed.as_bytes());
        Some(buf)
    }

    pub fn to_stitch_resolution(&self) -> CampaignIdentityResolution {
        let unbound_reason = || {
            self.resolution_note
                .clone()
                .unwrap_or_else(|| self.state.as_str().to_string())
        };
        match self.state {
            CampaignIdentityState::Known => {
                match (
                    self.campaign_key_bytes(),
                    self.stream_header_sha256
                        .as_deref()
                        .and_then(hex64_to_bytes),
                ) {
                    (Some(campaign_key), Some(evidence_digest)) => {
                        CampaignIdentityResolution::Verified {
                            scheme_major: CAMPAIGN_IDENTITY_SCHEME_MAJOR,
                            campaign_key,
                            evidence_digest,
                        }
                    }
                    _ => CampaignIdentityResolution::Missing {
                        reason: self.resolution_note.clone().unwrap_or_else(|| {
                            "identity marked known but its in-memory evidence proof does not \
                             recompute"
                                .to_string()
                        }),
                    },
                }
            }
            CampaignIdentityState::Unavailable | CampaignIdentityState::Unbound => {
                CampaignIdentityResolution::Missing {
                    reason: unbound_reason(),
                }
            }
            CampaignIdentityState::Conflict => CampaignIdentityResolution::Conflict {
                reason: unbound_reason(),
            },
        }
    }

    /// acquires a ticket; bytes observed while identity is unresolved
    pub fn append_ticket(
        &self,
        play_order: u64,
        session_nonce: impl Into<String>,
        process_start_identity: impl Into<String>,
    ) -> Option<AppendTicket> {
        Some(AppendTicket {
            campaign_key: self.campaign_key_bytes()?,
            play_order,
            session_nonce: session_nonce.into(),
            process_start_identity: process_start_identity.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed_cid1() -> ValidatedCid1 {
        use compact_contract::campaign_identity::{
            encode_cid1, parse_cid1_record, Cid1Fields, Cid1Observation,
        };
        let hour = compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24;
        let observation = Cid1Observation {
            game_unique_id: "a4b17fed-ed12-4361-9064-fb6576638110".to_owned(),
            game_unique_seed: 1_026_046_527,
            game_hour_raw: hour,
        };
        let fields = Cid1Fields::from_stable_observations(
            observation.clone(),
            observation,
            "vanilla_latest_2026_06_30".to_owned(),
            Cid1Witnesses {
                start_date_raw: Some(1_000),
                session: Some(7),
                scenario: Some("1936".to_owned()),
                initial_human_country_tags: Some(vec!["ENG".to_owned(), "GER".to_owned()]),
                modset_identity: Some("sha256:abc".to_owned()),
            },
        )
        .unwrap();
        let payload = encode_cid1(&fields).unwrap();
        let header = compact_contract::CompactRecordHeader {
            sequence: 0,
            game_hour_raw: hour,
            domain_id: compact_contract::campaign_identity::COMPACT_DOMAIN_STREAM_HEADER,
            record_type: compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER,
            payload_len: payload.len(),
            record_flags: 0,
        };
        parse_cid1_record(&header, &payload).unwrap()
    }

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
        assert!(canonical_uuid("a4b17fed-ed12-4361-9064-fb657663811").is_none());
        assert!(canonical_uuid("g4b17fed-ed12-4361-9064-fb6576638110").is_none());
        assert!(canonical_uuid("a4b17fed-ed12-4361-9064-fb6576638110-00").is_none());
    }

    #[test]
    fn opaque_validated_cid1_is_the_known_authority_boundary() {
        let cid1 = parsed_cid1();
        let identity = resolve_validated_cid1(&cid1);
        assert!(identity.is_bound());
        assert_eq!(identity.state, CampaignIdentityState::Known);
        assert_eq!(
            identity.campaign_id.as_deref(),
            Some("b28ea1bc4bcea2790b9257208abdc9c27f9985a6e57ef8ce8934d456fcd3be6f")
        );
        assert_eq!(
            identity.stream_header_sha256.as_deref(),
            Some(cid1.payload_sha256_hex().as_str())
        );
        validate_manifest_object(&to_manifest_json(&identity)).unwrap();
    }

    #[test]
    fn canonical_seed_rules() {
        assert_eq!(canonical_seed("1026046527").as_deref(), Some("1026046527"));
        assert_eq!(
            canonical_seed("-1033122218").as_deref(),
            Some("-1033122218")
        );
        assert_eq!(canonical_seed("0").as_deref(), Some("0"));
        assert_eq!(
            canonical_seed("-2147483648").as_deref(),
            Some("-2147483648")
        );
        assert_eq!(canonical_seed("2147483647").as_deref(), Some("2147483647"));
        assert!(canonical_seed("007").is_none());
        assert!(canonical_seed("-0").is_none());
        assert!(canonical_seed("-2147483649").is_none());
        assert!(canonical_seed("2147483648").is_none());
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
        assert!(id.game_unique_id.is_none());
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
        manifest["campaign_id"] = json!("a".repeat(64));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("must have null"), "{err}");
    }

    #[test]
    fn validator_rejects_digest_mismatch_on_known() {
        let id = resolve(&known_obs());
        let mut manifest = to_manifest_json(&id);
        manifest["campaign_id"] = json!("b".repeat(64));
        let err = validate_manifest_object(&manifest).unwrap_err();
        assert!(err.contains("does not recompute"), "{err}");
    }

    #[test]
    fn validator_rejects_noncanonical_uuid_on_known() {
        let id = resolve(&known_obs());
        let mut manifest = to_manifest_json(&id);
        manifest["game_unique_id"] = json!("A4B17FED-ED12-4361-9064-FB6576638110");
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
        let legacy = json!({ "schema": "native_collector_session_v1" });
        validate_in_session_manifest(&legacy).unwrap();
        let mut bad = json!({ "schema": "native_collector_session_v1" });
        bad.as_object_mut()
            .unwrap()
            .insert("campaign_identity".into(), json!({ "state": "known" }));
        assert!(validate_in_session_manifest(&bad).is_err());
    }

    #[test]
    fn documented_known_manifest_example_passes_validator() {
        let json_text = include_str!("fixtures/campaign_identity_known_manifest.json");
        let manifest: Value = serde_json::from_str(json_text).expect("valid example JSON");

        validate_manifest_object(&manifest["campaign_identity"])
            .expect("documented known manifest must pass the actual validator");
    }

    #[test]
    fn stitch_decision_matrix() {
        let a = resolve(&known_obs());
        let b = resolve(&known_obs());
        assert_eq!(stitch_decision(&a, &b), StitchDecision::SameCampaign);
        let c = resolve(&verified_obs("11111111-1111-4111-8111-111111111111", "99"));
        assert_eq!(stitch_decision(&a, &c), StitchDecision::DifferentCampaign);
        let d = resolve(&verified_obs("a4b17fed-ed12-4361-9064-fb6576638110", "99"));
        assert_eq!(stitch_decision(&a, &d), StitchDecision::ComponentConflict);
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
    fn to_stitch_resolution_verifies_only_bound_known() {
        let a = resolve(&known_obs());
        assert!(a.is_bound());
        match a.to_stitch_resolution() {
            CampaignIdentityResolution::Verified {
                scheme_major,
                campaign_key,
                evidence_digest,
            } => {
                assert_eq!(scheme_major, CAMPAIGN_IDENTITY_SCHEME_MAJOR);
                assert!(!campaign_key.is_empty());
                assert_eq!(
                    Some(evidence_digest),
                    a.stream_header_sha256.as_deref().and_then(hex64_to_bytes)
                );
            }
            other => panic!("bound Known must be Verified, got {other:?}"),
        }
    }

    #[test]
    fn to_stitch_resolution_campaign_key_equal_iff_same_playthrough() {
        let a = resolve(&known_obs());
        let same = resolve(&known_obs());
        let different = resolve(&verified_obs("11111111-1111-4111-8111-111111111111", "99"));
        assert_eq!(a.campaign_key_bytes(), same.campaign_key_bytes());
        assert!(a.campaign_key_bytes().is_some());
        assert_ne!(a.campaign_key_bytes(), different.campaign_key_bytes());
    }

    #[test]
    fn to_stitch_resolution_maps_unbound_and_conflict_states() {
        let unbound = resolve(&SessionHeaderIdentityObservation::default());
        assert!(matches!(
            unbound.to_stitch_resolution(),
            CampaignIdentityResolution::Missing { .. }
        ));

        let mut conflict = resolve(&known_obs());
        conflict.state = CampaignIdentityState::Conflict;
        assert!(matches!(
            conflict.to_stitch_resolution(),
            CampaignIdentityResolution::Conflict { .. }
        ));
    }

    #[test]
    fn to_stitch_resolution_fails_closed_on_mutated_known_clone() {
        let mut mutated = resolve(&known_obs());
        mutated.game_unique_seed = Some("99".to_string());
        assert_eq!(mutated.state, CampaignIdentityState::Known);
        assert!(!mutated.is_bound());
        assert!(matches!(
            mutated.to_stitch_resolution(),
            CampaignIdentityResolution::Missing { .. }
        ));
        let manifest = to_manifest_json(&mutated);
        assert_eq!(
            manifest["state"].as_str(),
            Some(CampaignIdentityState::Unavailable.as_str())
        );
        assert!(manifest["campaign_id"].is_null());
        validate_manifest_object(&manifest).unwrap();
    }

    #[test]
    fn append_ticket_binds_allocated_order_only_when_bound() {
        let bound = resolve(&known_obs());
        let ticket = bound
            .append_ticket(7, "nonce-x", "pid-x")
            .expect("bound identity yields a ticket");
        assert_eq!(ticket.play_order, 7);
        assert_eq!(ticket.session_nonce, "nonce-x");
        assert_eq!(ticket.process_start_identity, "pid-x");
        assert_eq!(Some(ticket.campaign_key), bound.campaign_key_bytes());

        let unbound = resolve(&SessionHeaderIdentityObservation::default());
        assert!(unbound.append_ticket(0, "n", "p").is_none());
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
