use serde_json::Value;
use sha2::{Digest, Sha256};

pub const MAGIC: [u8; 4] = *b"ICCP";
pub const SCHEMA: u16 = 4;
const HEADER_LEN: usize = 56;
const MAX_BODY_LEN: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelligenceControllerCompanionCarrier {
    pub frame_hour: u64,
    pub owner_tag_db_id: u32,
    body: Value,
}

impl IntelligenceControllerCompanionCarrier {
    pub fn from_fold_json(
        frame_hour: u64,
        owner_tag_db_id: u32,
        mut body: Value,
    ) -> Result<Self, String> {
        if frame_hour == 0 || owner_tag_db_id == 0 {
            return Err("ICCP frame or owner identity is zero".into());
        }
        let object = body.as_object_mut().ok_or("ICCP body is not an object")?;
        object.insert("owner".into(), Value::String(String::new()));
        if object.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA as u64)
            || object.get("frame_hour").and_then(Value::as_u64) != Some(frame_hour)
            || object.get("owner_tag_db_id").and_then(Value::as_u64) != Some(owner_tag_db_id as u64)
            || contains_pointer_material(&body)
        {
            return Err("ICCP body identity or pointer-free gate failed".into());
        }
        Ok(Self {
            frame_hour,
            owner_tag_db_id,
            body,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        let body = serde_json::to_vec(&self.body)
            .map_err(|error| format!("ICCP JSON encode failed: {error}"))?;
        if body.is_empty() || body.len() > MAX_BODY_LEN {
            return Err("ICCP body length is outside bounds".into());
        }
        let mut out = Vec::with_capacity(HEADER_LEN + body.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&SCHEMA.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&self.frame_hour.to_le_bytes());
        out.extend_from_slice(&self.owner_tag_db_id.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&Sha256::digest(&body));
        out.extend_from_slice(&body);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || bytes[..4] != MAGIC {
            return Err("ICCP magic or header missing".into());
        }
        let schema = u16::from_le_bytes(bytes[4..6].try_into().unwrap());
        let flags = u16::from_le_bytes(bytes[6..8].try_into().unwrap());
        let frame_hour = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let owner_tag_db_id = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
        let body_len = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;
        if schema != SCHEMA
            || flags != 1
            || body_len == 0
            || body_len > MAX_BODY_LEN
            || bytes.len()
                != HEADER_LEN
                    .checked_add(body_len)
                    .ok_or("ICCP length overflow")?
        {
            return Err("ICCP schema, flags, or length invalid".into());
        }
        let body_bytes = &bytes[HEADER_LEN..];
        if Sha256::digest(body_bytes).as_slice() != &bytes[24..56] {
            return Err("ICCP body SHA-256 mismatch".into());
        }
        let body: Value = serde_json::from_slice(body_bytes)
            .map_err(|error| format!("ICCP JSON decode failed: {error}"))?;
        Self::from_fold_json(frame_hour, owner_tag_db_id, body)
    }

    pub fn to_fold_json(&self, owner: &str) -> Result<Value, String> {
        if owner.is_empty()
            || owner.len() > 64
            || !owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        {
            return Err("ICCP resolved owner is invalid".into());
        }
        let mut body = self.body.clone();
        body.as_object_mut()
            .ok_or("ICCP body is not an object")?
            .insert("owner".into(), Value::String(owner.into()));
        Ok(body)
    }

    pub fn network_witness(&self) -> Result<Option<(&str, u32)>, String> {
        let evidence = self
            .body
            .get("visibility_evidence")
            .and_then(Value::as_object)
            .ok_or("ICCP visibility evidence missing")?;
        match (
            evidence.get("network_payload_sha256"),
            evidence.get("network_row_count"),
        ) {
            (Some(Value::String(hash)), Some(count)) => {
                let count = u32::try_from(count.as_u64().ok_or("ICCP network row count invalid")?)
                    .map_err(|_| "ICCP network row count overflow")?;
                Ok(Some((hash.as_str(), count)))
            }
            (Some(Value::Null), Some(Value::Null)) => Ok(None),
            _ => Err("ICCP network witness is torn".into()),
        }
    }
}

fn contains_pointer_material(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "installed_module_base" | "preferred_image_base" | "pointer" | "address"
            ) || contains_pointer_material(value)
        }),
        Value::Array(rows) => rows.iter().any(contains_pointer_material),
        _ => false,
    }
}
