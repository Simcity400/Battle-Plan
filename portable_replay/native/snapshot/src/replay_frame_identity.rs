//! The registered replay sidecar keys, and the ONE base-frame projection every
//! identity-bearing carrier hashes.
//!
//! # The defect this closes (GP-F10)
//!
//! Three replay carriers — XUI12, XUI15 and XUI23 — each embed a
//! `replay_frame_identity_sha256` INSIDE their own payload, and each consumer
//! re-derives that digest from the frame it decoded and compares. Each of the
//! three used to hash "the whole raw snapshot minus MY OWN keys", which left
//! every OTHER carrier's payload inside its own preimage. Writing `P12`, `P15`,
//! `P23` for the three payload values and `R` for the rest of the snapshot:
//!
//! ```text
//! H12 = SHA256(R, P15, P23)   and H12 is a field of P12
//! H15 = SHA256(R, P12, P23)   and H15 is a field of P15
//! H23 = SHA256(R, P12, P15)   and H23 is a field of P23
//! ```
//!
//! With one carrier real that is satisfiable. With two or more it is a fixed
//! point of composed SHA-256 evaluations, which no emission order can reach:
//! fixing `P12` fixes `H15`'s input, `H15` changes `P15`, and that invalidates
//! the `H12` already written inside `P12`. So the three carriers could never
//! ship together, and — separately — every new root or map key perturbed all
//! three digests, so an unrelated lane adding a sidecar silently invalidated
//! them.
//!
//! # What replaces it
//!
//! ONE projection, defined once, here: the base frame is the raw snapshot with
//! EVERY registered sidecar key removed — the identity-bearing ones and the
//! ones that merely ride alongside. It is a function of the game state, not of
//! which carriers happen to be present, so all three carriers may be produced
//! in a single pass and a fourteenth sidecar landing tomorrow does not move any
//! digest.
//!
//! This crate is the home because it is path-depended on by BOTH cargo
//! workspaces: `extractor/native` (the producer, which must build the same
//! preimage) and `replay_interface/native` (the three consumers, which
//! re-derive it). A projection with two implementations is a projection with
//! two answers.
//!
//! # A new sidecar cannot silently perturb an identity
//!
//! The exclusion set is the REGISTERED key list below, not a per-consumer
//! hand-list. A lane adding a carrier adds its key here — which is also the
//! only way its key reaches the wire scaffold — and the projection covers it by
//! construction. `identity_bearing_keys_are_registered` fails the build's test
//! run if an identity-bearing key is ever left out of the registry.
//!
//! # No backwards compatibility
//!
//! The three pre-GP-F10 domain separators are RETIRED and refused by name
//! (`BaseFrameIdentityError::RetiredDomain`). Old captures are disposable; a
//! digest under a retired domain is a stale producer, and accepting both
//! spellings would make the two semantics indistinguishable on the wire.

use serde_json::{Map, Value};

use crate::sha256::Sha256;

/// The snapshot member the map-scoped sidecars live in.
pub const SIDECAR_MAP_SCOPE_KEY: &str = "map";

/// Every registered ROOT-scoped replay sidecar key.
///
/// One row per key of the Wave-2 carrier registry
/// (`recorder_host/src/world_fold/wave2_sidecars/mod.rs::carriers`) whose scope
/// is `SidecarScope::Root`. Sorted, so a lane adding a key has one obvious
/// place to put it and two lanes adding keys rarely collide.
pub const REGISTERED_ROOT_SIDECAR_KEYS: [&str; 13] = [
    "_r2_combat_presentation_exact",
    "_r2_decision_evaluation_exact",
    "_r2_intelligence_presentation_exact",
    "_r2_national_focus_observer_exact",
    "_r2_occupation_ui_supplement_live",
    "_r2_peace_conference_presentation_exact",
    "_r2_politics_presentation_exact",
    "_r2_power_balance_state_live",
    "_r2_xui12_producer_canvas_authority",
    "_r2_xui12_production_lines_payload",
    "_r2_xui15_trade_catalogue_authority",
    "_r2_xui15_trade_catalogue_payload",
    "_r2_xui23_passive_command_abilities_authority",
];

/// Every registered sidecar key that lives inside the snapshot's `map` object.
///
/// They matter as much as the root keys: the whole `map` value is a member of
/// the raw snapshot, so a key nested one level down perturbs the digest exactly
/// as a root key does.
pub const REGISTERED_MAP_SIDECAR_KEYS: [&str; 2] = [
    "_r2_weather_visual_lifecycle",
    "_r2_xmap05_control_authority",
];

/// The registered keys whose VALUE contains a base-frame identity.
///
/// These are the keys that made the old per-consumer projections a cycle. They
/// are a strict subset of the registered keys above, which
/// `identity_bearing_keys_are_registered` pins; the projection excludes the
/// whole registry rather than just this subset, so a later sidecar cannot
/// perturb a digest whether or not it carries an identity of its own.
pub const IDENTITY_BEARING_SIDECAR_KEYS: [&str; 5] = [
    "_r2_xui12_producer_canvas_authority",
    "_r2_xui12_production_lines_payload",
    "_r2_xui15_trade_catalogue_authority",
    "_r2_xui15_trade_catalogue_payload",
    "_r2_xui23_passive_command_abilities_authority",
];

/// XUI12's base-frame domain separator (`game_ui/src/production_lines.rs`).
pub const XUI12_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui12.replay-base-frame.v3\0";
/// XUI15's base-frame domain separator (`game_ui/src/trade_compositor.rs`).
pub const XUI15_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui15.replay-base-frame.v2\0";
/// XUI23's base-frame domain separator (`game_ui/src/active_abilities.rs`).
pub const XUI23_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui23.replay-base-frame.v2\0";

/// The domains this module will hash under. Anything else is refused, so a
/// fourth carrier cannot start hashing base frames without registering here.
const CURRENT_IDENTITY_DOMAINS: [&[u8]; 3] = [
    XUI12_BASE_FRAME_IDENTITY_DOMAIN,
    XUI15_BASE_FRAME_IDENTITY_DOMAIN,
    XUI23_BASE_FRAME_IDENTITY_DOMAIN,
];

/// The pre-GP-F10 domains: "the whole frame minus my own keys". Refused by
/// name, never silently accepted alongside the current ones.
pub const RETIRED_IDENTITY_DOMAINS: [&[u8]; 3] = [
    b"hoi4.xui12.replay-frame-identity-without-carrier.v2\0",
    b"hoi4.xui15.replay-frame.v1\0",
    b"hoi4.xui23.replay-frame.v1\0",
];

/// Why a base-frame identity was not produced. Every variant is a refusal with
/// a name; none of them is a fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseFrameIdentityError {
    /// A pre-GP-F10 domain separator. The digest it names covered the other
    /// carriers' payloads and could not be satisfied when two carriers shipped
    /// together.
    RetiredDomain(String),
    /// A domain no carrier registered. Fail closed: an unregistered domain is
    /// a carrier whose projection nobody has agreed to.
    UnregisteredDomain(String),
    /// The projected base frame could not be serialised.
    BaseFrameNotSerialisable,
    /// A field length did not fit the preimage's 64-bit length prefix.
    PreimageLengthOverflow,
}

impl BaseFrameIdentityError {
    /// The refusal, in the words a capture or a log should carry.
    pub fn reason(&self) -> String {
        match self {
            BaseFrameIdentityError::RetiredDomain(domain) => format!(
                "replay base-frame identity refused: domain {domain} is RETIRED (pre-GP-F10 \
                 whole-frame-minus-own-keys digest); producers must hash the shared base-frame \
                 projection under the current domain"
            ),
            BaseFrameIdentityError::UnregisteredDomain(domain) => format!(
                "replay base-frame identity refused: domain {domain} is not registered in \
                 portable_replay_format::replay_frame_identity"
            ),
            BaseFrameIdentityError::BaseFrameNotSerialisable => {
                "replay base-frame identity refused: the projected base frame is not \
                 serialisable"
                    .to_owned()
            }
            BaseFrameIdentityError::PreimageLengthOverflow => {
                "replay base-frame identity refused: a preimage field length does not fit 64 \
                 bits"
                    .to_owned()
            }
        }
    }
}

impl std::fmt::Display for BaseFrameIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason())
    }
}

impl std::error::Error for BaseFrameIdentityError {}

/// Human-readable spelling of a domain separator, for a refusal message. The
/// separators are NUL-terminated ASCII, so the trailing NUL is dropped and any
/// non-UTF-8 byte is escaped rather than lost.
fn domain_label(domain: &[u8]) -> String {
    let trimmed = domain.strip_suffix(b"\0").unwrap_or(domain);
    String::from_utf8_lossy(trimmed).into_owned()
}

/// The base frame: the raw snapshot with every registered sidecar key removed,
/// at both scopes.
///
/// A `map` member that is not an object is left exactly as it was, because a
/// non-object `map` carries no sidecar and rewriting it would change the digest
/// for no reason.
///
/// A `map` OBJECT that is empty after projection is removed entirely, and that
/// is not tidiness — it is the same invariant as the root keys. Otherwise a
/// snapshot whose only map member is the registered weather sidecar would
/// project to `{"map": {}}` while a snapshot without that sidecar projects to
/// `{}`, so landing one registered carrier would move all three identities:
/// exactly the perturbation this projection exists to remove, surviving one
/// level down. Absent, empty, and sidecars-only therefore project to the same
/// base frame. An originally-empty `map` is canonicalised the same way, because
/// a rule that depends on whether a key was removed is a rule with two answers.
pub fn base_frame_projection(raw: &Map<String, Value>) -> Map<String, Value> {
    let mut base = raw.clone();
    for key in REGISTERED_ROOT_SIDECAR_KEYS {
        base.remove(key);
    }
    let Some(Value::Object(map_scope)) = base.get(SIDECAR_MAP_SCOPE_KEY) else {
        return base;
    };
    let mut projected = map_scope.clone();
    for key in REGISTERED_MAP_SIDECAR_KEYS {
        projected.remove(key);
    }
    if projected.is_empty() {
        base.remove(SIDECAR_MAP_SCOPE_KEY);
    } else {
        base.insert(SIDECAR_MAP_SCOPE_KEY.to_owned(), Value::Object(projected));
    }
    base
}

/// The exact preimage a base-frame identity is taken over.
///
/// Length-prefixed throughout, so no two distinct frames share a preimage by
/// concatenation, and prefixed by the carrier's domain separator so two
/// carriers never produce the same digest for the same base frame.
pub fn base_frame_identity_preimage(
    domain: &[u8],
    hour_index: i64,
    date: Option<&str>,
    save_version: Option<&str>,
    raw: &Map<String, Value>,
) -> Result<Vec<u8>, BaseFrameIdentityError> {
    if RETIRED_IDENTITY_DOMAINS
        .iter()
        .any(|retired| *retired == domain)
    {
        return Err(BaseFrameIdentityError::RetiredDomain(domain_label(domain)));
    }
    if !CURRENT_IDENTITY_DOMAINS
        .iter()
        .any(|current| *current == domain)
    {
        return Err(BaseFrameIdentityError::UnregisteredDomain(domain_label(
            domain,
        )));
    }
    let base = base_frame_projection(raw);
    let base = serde_json::to_vec(&base)
        .map_err(|_| BaseFrameIdentityError::BaseFrameNotSerialisable)?;
    let mut bytes = domain.to_vec();
    bytes.extend_from_slice(&hour_index.to_le_bytes());
    append_optional_text(&mut bytes, date)?;
    append_optional_text(&mut bytes, save_version)?;
    let base_len =
        u64::try_from(base.len()).map_err(|_| BaseFrameIdentityError::PreimageLengthOverflow)?;
    bytes.extend_from_slice(&base_len.to_le_bytes());
    bytes.extend_from_slice(&base);
    Ok(bytes)
}

/// The one base-frame identity. Every carrier that binds a payload to a frame
/// calls THIS, under its own registered domain.
pub fn base_frame_identity_sha256(
    domain: &[u8],
    hour_index: i64,
    date: Option<&str>,
    save_version: Option<&str>,
    raw: &Map<String, Value>,
) -> Result<String, BaseFrameIdentityError> {
    Ok(Sha256::hex(&base_frame_identity_preimage(
        domain,
        hour_index,
        date,
        save_version,
        raw,
    )?))
}

fn append_optional_text(
    bytes: &mut Vec<u8>,
    value: Option<&str>,
) -> Result<(), BaseFrameIdentityError> {
    match value {
        Some(value) => {
            bytes.push(1);
            let len = u64::try_from(value.len())
                .map_err(|_| BaseFrameIdentityError::PreimageLengthOverflow)?;
            bytes.extend_from_slice(&len.to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        None => bytes.push(0),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn raw(value: Value) -> Map<String, Value> {
        value.as_object().cloned().expect("object snapshot")
    }

    fn identity(domain: &[u8], value: &Map<String, Value>) -> String {
        base_frame_identity_sha256(domain, 7, Some("1936.1.1.12"), Some("1.19.2"), value)
            .expect("registered domain")
    }

    /// kills: dropping an identity-bearing key from the registry. The
    /// projection excludes the REGISTERED set, so a key that carries a digest
    /// and is not registered stays inside its own preimage -- which is exactly
    /// the GP-F10 cycle, re-created one key at a time.
    #[test]
    fn identity_bearing_keys_are_registered() {
        for key in IDENTITY_BEARING_SIDECAR_KEYS {
            assert!(
                REGISTERED_ROOT_SIDECAR_KEYS.contains(&key)
                    || REGISTERED_MAP_SIDECAR_KEYS.contains(&key),
                "{key} carries a base-frame identity but is not a registered sidecar"
            );
        }
    }

    /// kills: letting the registry hold a duplicate or an empty key -- a
    /// duplicate hides a real second carrier behind a copy of the first, and an
    /// empty key removes nothing while reading as coverage.
    #[test]
    fn the_registry_is_a_set_of_non_empty_keys() {
        let mut all: Vec<&str> = REGISTERED_ROOT_SIDECAR_KEYS
            .iter()
            .chain(REGISTERED_MAP_SIDECAR_KEYS.iter())
            .copied()
            .collect();
        let total = all.len();
        assert_eq!(total, 15, "thirteen root keys and two map keys");
        for key in &all {
            assert!(key.starts_with("_r2_"), "{key} is not a sidecar key");
        }
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), total, "a registered key is listed twice");
    }

    /// kills: reverting any consumer to "the whole frame minus MY OWN keys".
    /// Under the shared projection the identity is the same value whether the
    /// other carriers are present or absent, which is the property that lets
    /// all three ship in one hour. This is the cycle, killed at its root.
    #[test]
    fn every_registered_sidecar_is_invisible_to_every_identity() {
        let bare = raw(json!({
            "countries": {"GER": {"_r2_tag_db_id": 73}},
            "map": {"provinces": [1, 2, 3]},
        }));
        let mut loaded = bare.clone();
        for key in REGISTERED_ROOT_SIDECAR_KEYS {
            loaded.insert(key.to_owned(), json!({"anything": key}));
        }
        let mut map_scope = raw(json!({"provinces": [1, 2, 3]}));
        for key in REGISTERED_MAP_SIDECAR_KEYS {
            map_scope.insert(key.to_owned(), json!({"anything": key}));
        }
        loaded.insert("map".to_owned(), Value::Object(map_scope));

        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_eq!(
                identity(domain, &bare),
                identity(domain, &loaded),
                "a registered sidecar moved a base-frame identity"
            );
        }
    }

    /// kills: making the projection ignore game state, or hashing the same
    /// preimage for two different frames. The identity has to REFUSE a
    /// perturbed base frame -- that is the whole reason a carrier carries it.
    #[test]
    fn a_perturbed_base_frame_changes_every_identity() {
        let base = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 73}}}));
        let perturbed = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 74}}}));
        let nested = raw(json!({
            "countries": {"GER": {"_r2_tag_db_id": 73}},
            "map": {"provinces": [1, 2, 4]},
        }));
        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_ne!(identity(domain, &base), identity(domain, &perturbed));
            assert_ne!(identity(domain, &base), identity(domain, &nested));
        }
    }

    /// kills: dropping the domain separator, or pointing two carriers at one
    /// domain -- one carrier's digest would then authenticate another's
    /// payload against the same frame.
    #[test]
    fn each_carrier_domain_produces_a_distinct_digest() {
        let frame = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 73}}}));
        let mut digests: Vec<String> = CURRENT_IDENTITY_DOMAINS
            .iter()
            .map(|domain| identity(domain, &frame))
            .collect();
        digests.sort();
        digests.dedup();
        assert_eq!(digests.len(), 3, "two carriers share a domain separator");
    }

    /// kills: accepting a retired domain -- silently admitting both spellings
    /// would make the cyclic pre-GP-F10 digest and the base-frame digest
    /// indistinguishable on the wire, which is the backwards compatibility this
    /// repo does not do.
    #[test]
    fn a_retired_domain_is_refused_by_name() {
        let frame = raw(json!({"countries": {}}));
        for domain in RETIRED_IDENTITY_DOMAINS {
            let error = base_frame_identity_sha256(domain, 7, None, None, &frame)
                .expect_err("a retired domain must not hash");
            let BaseFrameIdentityError::RetiredDomain(named) = &error else {
                panic!("expected a named retired-domain refusal, got {error:?}");
            };
            assert_eq!(named.as_bytes(), &domain[..domain.len() - 1]);
            assert!(error.reason().contains("RETIRED"), "{}", error.reason());
        }
    }

    /// kills: hashing under any unregistered domain -- fail-closed means a
    /// fourth carrier registers here before it can bind a payload to a frame.
    #[test]
    fn an_unregistered_domain_is_refused() {
        let frame = raw(json!({"countries": {}}));
        let error = base_frame_identity_sha256(b"hoi4.xui99.replay-base-frame.v1\0", 7, None, None, &frame)
            .expect_err("an unregistered domain must not hash");
        assert!(matches!(
            error,
            BaseFrameIdentityError::UnregisteredDomain(_)
        ));
        assert!(error.reason().contains("not registered"), "{error}");
    }

    /// kills: leaving an empty projected `map` object behind (the revision the
    /// first review round rejected). A snapshot whose only map member is the
    /// registered weather sidecar would then project to `{"map": {}}` while the
    /// same snapshot without that sidecar projects to `{}`, so landing ONE
    /// registered carrier would move all three identities -- the perturbation
    /// this projection exists to remove, surviving one level down. Absent,
    /// empty, and sidecars-only must be one base frame.
    #[test]
    fn an_absent_map_an_empty_map_and_a_sidecars_only_map_are_one_base_frame() {
        let absent = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 73}}}));
        let empty = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 73}}, "map": {}}));
        let mut sidecars_only = raw(json!({"countries": {"GER": {"_r2_tag_db_id": 73}}}));
        let mut map_scope = Map::new();
        for key in REGISTERED_MAP_SIDECAR_KEYS {
            map_scope.insert(key.to_owned(), json!({"landed": key}));
        }
        sidecars_only.insert("map".to_owned(), Value::Object(map_scope));

        assert_eq!(base_frame_projection(&absent), base_frame_projection(&empty));
        assert_eq!(
            base_frame_projection(&absent),
            base_frame_projection(&sidecars_only)
        );
        assert!(!base_frame_projection(&sidecars_only).contains_key("map"));
        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_eq!(identity(domain, &absent), identity(domain, &empty));
            assert_eq!(identity(domain, &absent), identity(domain, &sidecars_only));
        }
        // A map that still holds real state after projection SURVIVES, and its
        // content still moves the identity -- the canonicalisation above is not
        // licence to drop map state.
        let real = raw(json!({
            "countries": {"GER": {"_r2_tag_db_id": 73}},
            "map": {"provinces": [1, 2, 3]},
        }));
        assert!(base_frame_projection(&real).contains_key("map"));
        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_ne!(identity(domain, &absent), identity(domain, &real));
        }
    }

    /// kills: rebuilding a non-object `map` member (an earlier revision
    /// replaced it with an empty object), and dropping the hour/date/save
    /// identity from the preimage.
    #[test]
    fn the_preimage_carries_the_frame_identity_and_leaves_a_non_object_map_alone() {
        let odd = raw(json!({"map": 7}));
        assert_eq!(base_frame_projection(&odd), odd);
        let frame = raw(json!({"countries": {}}));
        let domain = XUI12_BASE_FRAME_IDENTITY_DOMAIN;
        let hour = base_frame_identity_sha256(domain, 7, None, None, &frame).unwrap();
        assert_ne!(
            hour,
            base_frame_identity_sha256(domain, 8, None, None, &frame).unwrap()
        );
        assert_ne!(
            hour,
            base_frame_identity_sha256(domain, 7, Some("1936.1.1.12"), None, &frame).unwrap()
        );
        assert_ne!(
            hour,
            base_frame_identity_sha256(domain, 7, None, Some("1.19.2"), &frame).unwrap()
        );
        // Length prefixes, not concatenation: two different splits of the same
        // text must not collide.
        assert_ne!(
            base_frame_identity_sha256(domain, 7, Some("ab"), Some("c"), &frame).unwrap(),
            base_frame_identity_sha256(domain, 7, Some("a"), Some("bc"), &frame).unwrap()
        );
    }
}
