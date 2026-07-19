//! Collector-owned XUI21 game-file join.
//!
//! The embedded registry is compiled separately from rendering-law evidence.
//! It is admitted only when both its bytes and pinned executable authority
//! match the a729 build.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::OnceLock;

const REGISTRY_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../game_defs/out/fired_event_registry.json"
));
pub const REGISTRY_SHA256: [u8; 32] = [
    0xa8, 0x07, 0xb4, 0xd0, 0x29, 0x4a, 0x4b, 0x29, 0xbf, 0x8d, 0x82, 0xdd, 0xbe, 0x5c, 0xd8, 0x01,
    0xca, 0x9b, 0xf4, 0x87, 0x0c, 0xee, 0xf3, 0xa1, 0x6b, 0x1b, 0x2d, 0xaf, 0x03, 0xfd, 0x83, 0xc4,
];
const PINNED_EXE_SHA256: &str = "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
const SCHEMA: &str = "hoi4-xui21-fired-event-registry-v1";

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
    pub combined_id: u32,
    pub event_id: String,
    pub kind: EventKind,
    pub option_count: u32,
    pub desc_keys: Vec<String>,
    pub picture: Option<String>,
    pub source_file: String,
}

pub struct FiredEventRegistry {
    by_combined_id: BTreeMap<u32, EventDefinition>,
}

#[derive(Deserialize)]
struct WireRegistry {
    schema: String,
    authority: WireAuthority,
    meta: WireMeta,
    events: Vec<WireEvent>,
}

#[derive(Deserialize)]
struct WireAuthority {
    program: String,
    image_base: String,
    executable_sha256: String,
}

#[derive(Deserialize)]
struct WireMeta {
    parse_failures: Vec<serde_json::Value>,
    combined_id_collisions_last_wins: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct WireEvent {
    combined_id: u32,
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
        let wire: WireRegistry = serde_json::from_slice(bytes)
            .map_err(|error| format!("XUI21 event registry JSON invalid: {error}"))?;
        if wire.schema != SCHEMA
            || wire.authority.program != "hoi4.exe 1.19.2.0.a729"
            || wire.authority.image_base != "0x140000000"
            || wire.authority.executable_sha256 != PINNED_EXE_SHA256
        {
            return Err("XUI21 event registry authority mismatch".into());
        }
        if !wire.meta.parse_failures.is_empty()
            || !wire.meta.combined_id_collisions_last_wins.is_empty()
        {
            return Err("XUI21 event registry has structural extraction failures".into());
        }
        let mut by_combined_id = BTreeMap::new();
        for row in wire.events {
            let kind = match row.kind.as_str() {
                "country_event" => EventKind::Country,
                "news_event" => EventKind::News,
                "state_event" => EventKind::State,
                "unit_leader_event" => EventKind::UnitLeader,
                "operative_leader_event" => EventKind::OperativeLeader,
                _ => return Err(format!("XUI21 unknown event kind {}", row.kind)),
            };
            let definition = EventDefinition {
                combined_id: row.combined_id,
                event_id: row.event_id,
                kind,
                option_count: row.option_count,
                desc_keys: row.desc_keys,
                picture: row.picture,
                source_file: row.source_file,
            };
            if by_combined_id
                .insert(definition.combined_id, definition)
                .is_some()
            {
                return Err("XUI21 duplicate combined event id".into());
            }
        }
        if by_combined_id.is_empty() {
            return Err("XUI21 event registry is empty".into());
        }
        Ok(Self { by_combined_id })
    }

    pub fn get(&self, combined_id: u32) -> Option<&EventDefinition> {
        self.by_combined_id.get(&combined_id)
    }

    pub fn len(&self) -> usize {
        self.by_combined_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_combined_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_registry_authenticates_and_resolves_combined_oracle() {
        let registry = FiredEventRegistry::embedded().unwrap();
        assert_eq!(registry.len(), 6_937);
        let event = registry.get(42_900_001).unwrap();
        assert_eq!(event.event_id, "ss_recruitment_event.1");
        assert_eq!(event.kind, EventKind::Country);
        assert_eq!(event.source_file, "events/ss_recruitment_event.txt");
    }

    #[test]
    fn unknown_combined_id_never_falls_back() {
        assert!(FiredEventRegistry::embedded()
            .unwrap()
            .get(u32::MAX)
            .is_none());
    }
}
