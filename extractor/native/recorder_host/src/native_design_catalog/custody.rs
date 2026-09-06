use super::*;
use compact_contract::campaign_identity::ValidatedCid1;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamBinding {
    pub logical_stream_id: u64,
    pub game_unique_id: String,
    pub game_unique_seed: i32,
    pub cid1_payload_sha256: String,
}
impl StreamBinding {
    pub fn from_cid1(logical_stream_id: u64, cid1: &ValidatedCid1) -> Self {
        Self {
            logical_stream_id,
            game_unique_id: cid1.game_unique_id().to_owned(),
            game_unique_seed: cid1.game_unique_seed(),
            cid1_payload_sha256: hex(&cid1.payload_sha256()),
        }
    }
    pub fn matches_pair(&self, pair: &Value) -> bool {
        pair["game_unique_id"].as_str() == Some(self.game_unique_id.as_str())
            && pair["game_unique_seed"].as_i64() == Some(i64::from(self.game_unique_seed))
    }
    pub fn validate_snapshot(&self, snapshot: &Value) -> std::result::Result<(), String> {
        if !self.matches_pair(&snapshot["authority"]["campaign"]) {
            return Err("native A27 observation campaign differs from acknowledged stream".into());
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct StreamCustody(Arc<Mutex<Option<StreamBinding>>>);
impl StreamCustody {
    pub fn current(&self) -> std::result::Result<Option<StreamBinding>, String> {
        self.0
            .lock()
            .map(|current| current.clone())
            .map_err(|_| "native A27 stream custody poisoned".into())
    }
    pub fn acknowledge(&self, binding: StreamBinding) -> std::result::Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "native A27 stream custody poisoned")? = Some(binding);
        Ok(())
    }
    pub fn suspend(&self) -> std::result::Result<(), String> {
        *self
            .0
            .lock()
            .map_err(|_| "native A27 stream custody poisoned")? = None;
        Ok(())
    }
}

#[derive(Default)]
pub struct ObservationGate {
    binding: Option<StreamBinding>,
    latest_date: Option<u32>,
    refused: bool,
}
impl ObservationGate {
    pub fn observe(&mut self, binding: &StreamBinding, date: u32, pair: &Value) -> Result<()> {
        if self.binding.as_ref() != Some(binding) {
            self.binding = Some(binding.clone());
            self.latest_date = None;
            self.refused = false;
        }
        // A rollback cannot re-enter its old stream even if its clock later catches up.
        if !binding.matches_pair(pair) || self.latest_date.is_some_and(|last| date < last) {
            self.refused = true;
        }
        if self.refused {
            return Err(failure(
                FailureKind::InvalidIdentity,
                "campaign/rollback awaits acknowledged stream",
            ));
        }
        self.latest_date = Some(date);
        Ok(())
    }
}

pub struct DirectStreamBinding {
    custody: StreamCustody,
    prefix: Vec<u8>,
    record_len: Option<usize>,
    bound: bool,
}
impl DirectStreamBinding {
    pub fn new(
        custody: StreamCustody,
        mode: compact_contract::stream_validator::StreamIdentityMode,
    ) -> std::result::Result<Self, String> {
        if mode != compact_contract::stream_validator::StreamIdentityMode::IdentityV1 {
            return Err("native A27 direct custody requires handshake-selected IdentityV1".into());
        }
        Ok(Self {
            custody,
            prefix: Vec::new(),
            record_len: None,
            bound: false,
        })
    }
    pub fn observe(&mut self, mut chunk: &[u8]) -> std::result::Result<(), String> {
        use compact_contract::{parse_compact_record_header, COMPACT_RECORD_HEADER_LEN};
        while !chunk.is_empty() && !self.bound {
            let target = self.record_len.unwrap_or(COMPACT_RECORD_HEADER_LEN);
            let take = (target - self.prefix.len()).min(chunk.len());
            self.prefix.extend_from_slice(&chunk[..take]);
            chunk = &chunk[take..];
            if self.record_len.is_none() && self.prefix.len() == COMPACT_RECORD_HEADER_LEN {
                let header = parse_compact_record_header(&self.prefix)
                    .map_err(|error| format!("native A27 direct header: {error}"))?;
                if header.record_len()
                    > COMPACT_RECORD_HEADER_LEN
                        + compact_contract::campaign_identity::CID1_MAX_PAYLOAD_LEN
                {
                    return Err("native A27 direct CID1 prefix exceeds bound".into());
                }
                self.record_len = Some(header.record_len());
            }
            if self.record_len == Some(self.prefix.len()) {
                let first = crate::duplex_session::identity_control::ValidatedFirstCompactRecord::identity_v1(0, &self.prefix)
                    .map_err(|error| format!("native A27 direct CID1: {error}"))?;
                let cid1 = first
                    .validated_cid1()
                    .ok_or("native A27 direct record lacks validated CID1")?;
                self.custody
                    .acknowledge(StreamBinding::from_cid1(0, cid1))?;
                self.bound = true;
                self.prefix.clear();
            }
        }
        Ok(())
    }
}
