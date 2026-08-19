use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const REGISTRY_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../game_defs/out/fired_event_registry.json"
));
pub const REGISTRY_SHA256: [u8; 32] = [
    0x95, 0xba, 0x71, 0xf4, 0x0d, 0xdf, 0x69, 0x4c, 0xec, 0x35, 0x6d, 0x6c, 0x31, 0x69, 0xd7, 0xcf,
    0xe7, 0xdf, 0x36, 0x4c, 0x05, 0x2b, 0x69, 0xbc, 0xfc, 0x13, 0x0b, 0xc7, 0x56, 0x2d, 0xd7, 0x08,
];
const PINNED_EXE_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const SCHEMA: &str = "hoi4-xui21-fired-event-registry-v2";
const NUMERIC_ID_SPAN: u32 = 100_000;
const BARE_ID_LIMIT: u32 = 1_000_000;

fn decode_kind(kind: &str) -> Result<EventKind, String> {
    match kind {
        "country_event" => Ok(EventKind::Country),
        "news_event" => Ok(EventKind::News),
        "state_event" => Ok(EventKind::State),
        "unit_leader_event" => Ok(EventKind::UnitLeader),
        "operative_leader_event" => Ok(EventKind::OperativeLeader),
        _ => Err(format!("XUI21 unknown event kind {kind}")),
    }
}

pub fn engine_reduce_suffix(suffix: &str) -> u32 {
    suffix.bytes().fold(0u32, |accumulator, byte| {
        accumulator
            .wrapping_mul(10)
            .wrapping_add((byte as u32).wrapping_sub(0x30))
    })
}

fn split_event_id<'a>(event_id: &'a str, namespace: &str) -> Result<&'a str, String> {
    let (declared_namespace, suffix) = event_id
        .rsplit_once('.')
        .filter(|(_, suffix)| !suffix.is_empty())
        .ok_or_else(|| format!("XUI21 event id {event_id} has no namespace"))?;
    if declared_namespace != namespace {
        return Err(format!(
            "XUI21 event id {event_id} disagrees with its namespace"
        ));
    }
    Ok(suffix)
}

fn bare_numeric(event_id: &str) -> Option<u32> {
    if event_id.is_empty() || !event_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    event_id
        .parse::<u32>()
        .ok()
        .filter(|id| *id < BARE_ID_LIMIT)
}

fn addressable_numeric(suffix: &str) -> Option<u32> {
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let reduced = engine_reduce_suffix(suffix);
    if reduced >= NUMERIC_ID_SPAN || suffix.parse::<u32>() != Ok(reduced) {
        return None;
    }
    Some(reduced)
}

fn check_direct_row(event_id: &str, namespace: &str, numeric_id: u32) -> Result<(), String> {
    let suffix = split_event_id(event_id, namespace)?;
    if addressable_numeric(suffix) != Some(numeric_id) {
        return Err(format!(
            "XUI21 event id {event_id} disagrees with its numeric key"
        ));
    }
    Ok(())
}

fn check_aliased_row(event_id: &str, namespace: &str, reduced_id: u32) -> Result<(), String> {
    let suffix = split_event_id(event_id, namespace)?;
    if engine_reduce_suffix(suffix) != reduced_id {
        return Err(format!(
            "XUI21 aliased event {event_id} disagrees with its reduced suffix"
        ));
    }
    if let Some(numeric_id) = addressable_numeric(suffix) {
        return Err(format!(
            "XUI21 aliased event {event_id} is addressable as {numeric_id}"
        ));
    }
    Ok(())
}

fn check_bare_row(event_id: &str, numeric_id: u32) -> Result<(), String> {
    if bare_numeric(event_id) != Some(numeric_id) {
        return Err(format!(
            "XUI21 bare event id {event_id} disagrees with its numeric key"
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Country,
    News,
    State,
    UnitLeader,
    OperativeLeader,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventDefinition {
    pub namespace: String,
    pub numeric_id: u32,
    pub event_id: String,
    pub kind: EventKind,
    pub option_count: u32,
    pub desc_keys: Vec<String>,
    pub picture: Option<String>,
    pub source_file: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasedEvent {
    pub reduced_id: u32,
    pub definition: EventDefinition,
}

pub struct FiredEventRegistry {
    by_event_key: BTreeMap<(String, u32), EventDefinition>,
    aliased: Vec<AliasedEvent>,
    bare: BTreeMap<u32, EventDefinition>,
}

#[derive(Deserialize)]
struct WireRegistry {
    schema: String,
    authority: WireAuthority,
    meta: WireMeta,
    events: Vec<WireEvent>,
    aliased_events: Vec<WireAliasedEvent>,
    bare_events: Vec<WireBareEvent>,
}

#[derive(Deserialize)]
struct WireAuthority {
    program: String,
    image_base: String,
    executable_sha256: String,
}

#[derive(Deserialize)]
struct WireMeta {
    numeric_id_span: u32,
    bare_id_limit: u32,
    parse_failures: Vec<serde_json::Value>,
    event_key_collisions_last_wins: Vec<serde_json::Value>,
    alias_key_collisions_last_wins: Vec<serde_json::Value>,
    bare_key_collisions_last_wins: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct WireEvent {
    namespace: String,
    numeric_id: u32,
    event_id: String,
    kind: String,
    option_count: u32,
    desc_keys: Vec<String>,
    picture: Option<String>,
    source_file: String,
}

#[derive(Deserialize)]
struct WireAliasedEvent {
    namespace: String,
    reduced_id: u32,
    event_id: String,
    kind: String,
    option_count: u32,
    desc_keys: Vec<String>,
    picture: Option<String>,
    source_file: String,
}

#[derive(Deserialize)]
struct WireBareEvent {
    numeric_id: u32,
    event_id: String,
    kind: String,
    option_count: u32,
    desc_keys: Vec<String>,
    picture: Option<String>,
    source_file: String,
}

impl FiredEventRegistry {
    pub fn embedded() -> Result<&'static Self, String> {
        static REGISTRY: OnceLock<Result<FiredEventRegistry, String>> = OnceLock::new();
        REGISTRY
            .get_or_init(|| Self::decode_authenticated(REGISTRY_BYTES))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn decode_authenticated(bytes: &[u8]) -> Result<Self, String> {
        let digest: [u8; 32] = Sha256::digest(bytes).into();
        if digest != REGISTRY_SHA256 {
            return Err("XUI21 event registry byte hash mismatch".into());
        }
        Self::decode_validated(bytes)
    }

    fn decode_validated(bytes: &[u8]) -> Result<Self, String> {
        let wire: WireRegistry = serde_json::from_slice(bytes)
            .map_err(|error| format!("XUI21 event registry JSON invalid: {error}"))?;
        if wire.schema != SCHEMA
            || wire.authority.program != "hoi4.exe 1.19.2.0.a729"
            || wire.authority.image_base != "0x140000000"
            || wire.authority.executable_sha256 != PINNED_EXE_SHA256
        {
            return Err("XUI21 event registry authority mismatch".into());
        }
        if wire.meta.numeric_id_span != NUMERIC_ID_SPAN || wire.meta.bare_id_limit != BARE_ID_LIMIT
        {
            return Err("XUI21 event registry id encoding mismatch".into());
        }
        if !wire.meta.parse_failures.is_empty()
            || !wire.meta.event_key_collisions_last_wins.is_empty()
            || !wire.meta.alias_key_collisions_last_wins.is_empty()
            || !wire.meta.bare_key_collisions_last_wins.is_empty()
        {
            return Err("XUI21 event registry has structural extraction failures".into());
        }
        let mut by_event_key = BTreeMap::new();
        for row in wire.events {
            let kind = decode_kind(&row.kind)?;
            check_direct_row(&row.event_id, &row.namespace, row.numeric_id)?;
            let key = (row.namespace.to_ascii_lowercase(), row.numeric_id);
            let definition = EventDefinition {
                namespace: row.namespace,
                numeric_id: row.numeric_id,
                event_id: row.event_id,
                kind,
                option_count: row.option_count,
                desc_keys: row.desc_keys,
                picture: row.picture,
                source_file: row.source_file,
            };
            if by_event_key.insert(key, definition).is_some() {
                return Err("XUI21 duplicate event namespace/numeric key".into());
            }
        }
        if by_event_key.is_empty() {
            return Err("XUI21 event registry is empty".into());
        }
        let mut aliased = Vec::with_capacity(wire.aliased_events.len());
        for row in wire.aliased_events {
            let kind = decode_kind(&row.kind)?;
            check_aliased_row(&row.event_id, &row.namespace, row.reduced_id)?;
            aliased.push(AliasedEvent {
                reduced_id: row.reduced_id,
                definition: EventDefinition {
                    namespace: row.namespace,
                    numeric_id: row.reduced_id,
                    event_id: row.event_id,
                    kind,
                    option_count: row.option_count,
                    desc_keys: row.desc_keys,
                    picture: row.picture,
                    source_file: row.source_file,
                },
            });
        }
        let mut bare = BTreeMap::new();
        for row in wire.bare_events {
            let kind = decode_kind(&row.kind)?;
            check_bare_row(&row.event_id, row.numeric_id)?;
            let definition = EventDefinition {
                namespace: String::new(),
                numeric_id: row.numeric_id,
                event_id: row.event_id,
                kind,
                option_count: row.option_count,
                desc_keys: row.desc_keys,
                picture: row.picture,
                source_file: row.source_file,
            };
            if bare.insert(definition.numeric_id, definition).is_some() {
                return Err("XUI21 duplicate bare event id".into());
            }
        }
        Ok(Self {
            by_event_key,
            aliased,
            bare,
        })
    }

    pub fn get(&self, namespace: &str, numeric_id: u32) -> Option<&EventDefinition> {
        self.by_event_key
            .get(&(namespace.to_ascii_lowercase(), numeric_id))
    }

    pub fn aliased(&self) -> &[AliasedEvent] {
        &self.aliased
    }

    pub fn bare(&self, numeric_id: u32) -> Option<&EventDefinition> {
        self.bare.get(&numeric_id)
    }

    pub fn len(&self) -> usize {
        self.by_event_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_event_key.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_authenticates_and_resolves_namespace_oracle() {
        let registry = FiredEventRegistry::embedded().unwrap();
        assert_eq!(registry.len(), 6_937);
        let event = registry.get("ss_recruitment_event", 1).unwrap();
        assert_eq!(event.event_id, "ss_recruitment_event.1");
        assert_eq!(event.kind, EventKind::Country);
        assert_eq!(event.source_file, "events/ss_recruitment_event.txt");
    }

    #[test]
    fn live_capture_id_28200289_resolves_to_news_289() {
        let registry = FiredEventRegistry::embedded().unwrap();
        let event = registry.get("news", 289).unwrap();
        assert_eq!(event.event_id, "news.289");
        assert_eq!(event.kind, EventKind::News);
        assert!(registry.get("SEA_PRC_misc_events", 289).is_none());
    }

    #[test]
    fn namespace_lookup_is_ascii_case_insensitive() {
        let registry = FiredEventRegistry::embedded().unwrap();
        assert_eq!(
            registry
                .get("NEWS", 289)
                .map(|event| event.event_id.as_str()),
            Some("news.289")
        );
    }

    #[test]
    fn leading_zero_ids_are_reachable_by_reduced_numeric() {
        let registry = FiredEventRegistry::embedded().unwrap();
        let event = registry.get("BBA_italy_missiolinis", 0).unwrap();
        assert_eq!(event.event_id, "BBA_italy_missiolinis.00");
        assert_eq!(event.numeric_id, 0);
    }

    #[test]
    fn unknown_event_key_never_falls_back() {
        let registry = FiredEventRegistry::embedded().unwrap();
        assert!(registry.get("news", u32::MAX).is_none());
        assert!(registry.get("no_such_namespace", 1).is_none());
    }

    #[test]
    fn aliased_events_carry_only_their_suffix_reduction() {
        let registry = FiredEventRegistry::embedded().unwrap();
        let aliased: Vec<(&str, u32)> = registry
            .aliased()
            .iter()
            .map(|row| (row.definition.event_id.as_str(), row.reduced_id))
            .collect();
        assert_eq!(
            aliased,
            [
                ("BBA_italy_missiolinis.02_north", 7_390_336),
                ("BBA_italy_missiolinis.03_north", 8_390_336),
                ("BBA_italy_missiolinis.04_south", 9_440_636),
                ("BBA_italy_missiolinis.05_south", 10_440_636),
            ]
        );
        assert!(registry.get("BBA_italy_missiolinis", 90_336).is_none());
        assert!(registry.get("BBA_italy_missiolinis", 2).is_none());
    }

    #[test]
    fn suffix_reduction_matches_the_engine_accumulator() {
        assert_eq!(engine_reduce_suffix("02_north"), 7_390_336);
        assert_eq!(engine_reduce_suffix("00"), 0);
        assert_eq!(engine_reduce_suffix("012"), 12);
        assert_eq!(engine_reduce_suffix("4294967296"), 0);
        assert_eq!(addressable_numeric("012"), Some(12));
        assert_eq!(addressable_numeric("4294967296"), None);
        assert_eq!(addressable_numeric("100000"), None);
        assert_eq!(addressable_numeric("02_north"), None);
    }

    #[test]
    fn bare_ids_are_literals_below_one_million() {
        assert_eq!(bare_numeric("999999"), Some(999_999));
        assert_eq!(bare_numeric("0"), Some(0));
        assert_eq!(bare_numeric("1000000"), None);
        assert_eq!(bare_numeric("alpha.1"), None);
        assert_eq!(bare_numeric(""), None);
        assert!(FiredEventRegistry::embedded().unwrap().bare(1).is_none());
    }

    /// admitted, so a regenerated and re-pinned artifact cannot assert a key it
    #[test]
    fn a_row_cannot_assert_a_key_it_does_not_have() {
        assert!(check_direct_row("alpha.1", "alpha", 1).is_ok());
        assert!(check_direct_row("alpha.1", "alpha", 2).is_err());
        assert!(check_direct_row("alpha.1", "beta", 1).is_err());
        assert!(check_direct_row("alpha.100000", "alpha", 100_000).is_err());

        assert!(check_aliased_row("alpha.02_north", "alpha", 7_390_336).is_ok());
        assert!(check_aliased_row("alpha.02_north", "alpha", 1).is_err());
        assert!(check_aliased_row("alpha.1", "alpha", 1).is_err());

        assert!(check_bare_row("999999", 999_999).is_ok());
        assert!(check_bare_row("999999", 1).is_err());
        assert!(check_bare_row("alpha.1", 1).is_err());
        assert!(check_bare_row("1000000", 1_000_000).is_err());
    }

    fn crafted(events: &str, aliased: &str, bare: &str) -> String {
        format!(
            r#"{{
              "schema": "{SCHEMA}",
              "authority": {{
                "program": "hoi4.exe 1.19.2.0.a729",
                "image_base": "0x140000000",
                "executable_sha256": "{PINNED_EXE_SHA256}"
              }},
              "meta": {{
                "numeric_id_span": {NUMERIC_ID_SPAN},
                "bare_id_limit": {BARE_ID_LIMIT},
                "parse_failures": [],
                "event_key_collisions_last_wins": [],
                "alias_key_collisions_last_wins": [],
                "bare_key_collisions_last_wins": []
              }},
              "events": [{events}],
              "aliased_events": [{aliased}],
              "bare_events": [{bare}]
            }}"#
        )
    }

    fn row(fields: &str) -> String {
        format!(
            r#"{{{fields}, "kind": "country_event", "option_count": 0,
               "desc_keys": [], "picture": null, "source_file": "events/a.txt"}}"#
        )
    }

    #[test]
    fn the_decoder_refuses_a_crafted_artifact() {
        let honest_event = row(r#""namespace": "alpha", "numeric_id": 1, "event_id": "alpha.1""#);
        assert!(
            FiredEventRegistry::decode_validated(crafted(&honest_event, "", "").as_bytes()).is_ok(),
            "the honest artifact must decode"
        );

        let lying_event = row(r#""namespace": "alpha", "numeric_id": 2, "event_id": "alpha.1""#);
        assert!(
            FiredEventRegistry::decode_validated(crafted(&lying_event, "", "").as_bytes()).is_err()
        );

        let smuggled_alias = row(r#""namespace": "alpha", "reduced_id": 1, "event_id": "alpha.1""#);
        assert!(FiredEventRegistry::decode_validated(
            crafted(&honest_event, &smuggled_alias, "").as_bytes()
        )
        .is_err());

        let lying_bare = row(r#""numeric_id": 7, "event_id": "999999""#);
        assert!(FiredEventRegistry::decode_validated(
            crafted(&honest_event, "", &lying_bare).as_bytes()
        )
        .is_err());
    }
}
