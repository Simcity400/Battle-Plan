use serde_json::{Map, Value};

use crate::sha256::Sha256;

pub const SIDECAR_MAP_SCOPE_KEY: &str = "map";

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

pub const REGISTERED_MAP_SIDECAR_KEYS: [&str; 2] = [
    "_r2_weather_visual_lifecycle",
    "_r2_xmap05_control_authority",
];

pub const IDENTITY_BEARING_SIDECAR_KEYS: [&str; 5] = [
    "_r2_xui12_producer_canvas_authority",
    "_r2_xui12_production_lines_payload",
    "_r2_xui15_trade_catalogue_authority",
    "_r2_xui15_trade_catalogue_payload",
    "_r2_xui23_passive_command_abilities_authority",
];

pub const XUI12_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui12.replay-base-frame.v3\0";
pub const XUI15_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui15.replay-base-frame.v2\0";
pub const XUI23_BASE_FRAME_IDENTITY_DOMAIN: &[u8] = b"hoi4.xui23.replay-base-frame.v2\0";

const CURRENT_IDENTITY_DOMAINS: [&[u8]; 3] = [
    XUI12_BASE_FRAME_IDENTITY_DOMAIN,
    XUI15_BASE_FRAME_IDENTITY_DOMAIN,
    XUI23_BASE_FRAME_IDENTITY_DOMAIN,
];

pub const RETIRED_IDENTITY_DOMAINS: [&[u8]; 3] = [
    b"hoi4.xui12.replay-frame-identity-without-carrier.v2\0",
    b"hoi4.xui15.replay-frame.v1\0",
    b"hoi4.xui23.replay-frame.v1\0",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseFrameIdentityError {
    RetiredDomain(String),
    UnregisteredDomain(String),
    BaseFrameNotSerialisable,
    PreimageLengthOverflow,
}

impl BaseFrameIdentityError {
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

fn domain_label(domain: &[u8]) -> String {
    let trimmed = domain.strip_suffix(b"\0").unwrap_or(domain);
    String::from_utf8_lossy(trimmed).into_owned()
}

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
    let base =
        serde_json::to_vec(&base).map_err(|_| BaseFrameIdentityError::BaseFrameNotSerialisable)?;
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

    #[test]
    fn an_unregistered_domain_is_refused() {
        let frame = raw(json!({"countries": {}}));
        let error =
            base_frame_identity_sha256(b"hoi4.xui99.replay-base-frame.v1\0", 7, None, None, &frame)
                .expect_err("an unregistered domain must not hash");
        assert!(matches!(
            error,
            BaseFrameIdentityError::UnregisteredDomain(_)
        ));
        assert!(error.reason().contains("not registered"), "{error}");
    }

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

        assert_eq!(
            base_frame_projection(&absent),
            base_frame_projection(&empty)
        );
        assert_eq!(
            base_frame_projection(&absent),
            base_frame_projection(&sidecars_only)
        );
        assert!(!base_frame_projection(&sidecars_only).contains_key("map"));
        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_eq!(identity(domain, &absent), identity(domain, &empty));
            assert_eq!(identity(domain, &absent), identity(domain, &sidecars_only));
        }
        let real = raw(json!({
            "countries": {"GER": {"_r2_tag_db_id": 73}},
            "map": {"provinces": [1, 2, 3]},
        }));
        assert!(base_frame_projection(&real).contains_key("map"));
        for domain in CURRENT_IDENTITY_DOMAINS {
            assert_ne!(identity(domain, &absent), identity(domain, &real));
        }
    }

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
        assert_ne!(
            base_frame_identity_sha256(domain, 7, Some("ab"), Some("c"), &frame).unwrap(),
            base_frame_identity_sha256(domain, 7, Some("a"), Some("bc"), &frame).unwrap()
        );
    }
}
