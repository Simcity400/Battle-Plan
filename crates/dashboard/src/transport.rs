//! Data requests and bounded, fragmented replies between the host and the
//! native data service. Only compact JSON crosses; never replay tables or
//! map pixels.

use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

use crate::ReplayMetadata;

/// Includes one serialized response, before the control-envelope escaping.
pub const MAX_REPLY_BYTES: usize = 32 * 1024 * 1024;
/// JSON has already escaped control characters. Escaping its quotes/backslashes
/// again uses at most twice this size, leaving room in the 64 KiB control frame.
const FRAGMENT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(tag = "scope", rename_all = "camelCase", deny_unknown_fields)]
pub enum MapSettingsTarget {
    Global,
    #[default]
    Campaign,
    Widget {
        id: String,
    },
}

impl MapSettingsTarget {
    pub fn key(&self, campaign: &str) -> Result<String, &'static str> {
        match self {
            Self::Global => Ok("$global".into()),
            Self::Campaign => Ok(campaign.into()),
            Self::Widget { id } if !id.is_empty() && id.len() <= 128 && !id.contains('\0') => {
                Ok(format!("{campaign}/widget/{id}"))
            }
            Self::Widget { .. } => Err("Invalid widget settings target"),
        }
    }
}

/// A product request against an open source lease. `params` are the product's
/// own typed parameters, validated by the product.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProductRequest {
    pub source_id: String,
    pub revision: String,
    pub product: String,
    pub params: serde_json::Value,
}

impl ProductRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.source_id.is_empty()
            || self.source_id.len() > 128
            || self.revision.is_empty()
            || self.revision.len() > 128
        {
            return Err("Invalid product request source");
        }
        if self.product.is_empty() || self.product.len() > 64 {
            return Err("Invalid product key");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum DataCommand {
    MapSettings {
        id: String,
        #[serde(default)]
        target: MapSettingsTarget,
        #[serde(rename = "sourceId")]
        source_id: String,
        revision: String,
        settings: Option<crate::protocol::CampaignMapSettings>,
    },
    Open {
        id: String,
        path: String,
        revision: String,
    },
    Product {
        id: String,
        request: ProductRequest,
    },
    Cancel {
        id: String,
    },
    Release {
        #[serde(rename = "sourceId")]
        source_id: String,
    },
}

impl DataCommand {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Open { id, .. }
            | Self::MapSettings { id, .. }
            | Self::Product { id, .. }
            | Self::Cancel { id } => Some(id),
            Self::Release { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub enum DataEvent {
    Chunk {
        id: String,
        sequence: u32,
        #[serde(rename = "totalBytes")]
        total_bytes: u32,
        last: bool,
        fragment: String,
    },
    Error {
        id: String,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SourceReply {
    /// A client lease. Several leases may share one underlying source and cache.
    pub id: String,
    pub path: String,
    pub revision: String,
    pub metadata: ReplayMetadata,
}

/// Keeps only one serialized result plus the current small transport fragment.
pub struct ReplyChunks {
    id: String,
    payload: String,
    offset: usize,
    sequence: u32,
}

impl ReplyChunks {
    pub fn new(id: String, payload: String) -> Result<Self, String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid dashboard request identifier".into());
        }
        if payload.is_empty() || payload.len() > MAX_REPLY_BYTES {
            return Err("Dashboard response exceeds the transport byte budget".into());
        }
        Ok(Self {
            id,
            payload,
            offset: 0,
            sequence: 0,
        })
    }

    pub fn retained_bytes(&self) -> usize {
        self.payload.capacity() + self.id.capacity()
    }
}

impl Iterator for ReplyChunks {
    type Item = DataEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.payload.len() {
            return None;
        }
        let mut end = (self.offset + FRAGMENT_BYTES).min(self.payload.len());
        while !self.payload.is_char_boundary(end) {
            end -= 1;
        }
        let fragment = self.payload[self.offset..end].to_owned();
        self.offset = end;
        let sequence = self.sequence;
        self.sequence += 1;
        Some(DataEvent::Chunk {
            id: self.id.clone(),
            sequence,
            total_bytes: self.payload.len() as u32,
            last: end == self.payload.len(),
            fragment,
        })
    }
}

#[derive(Default)]
pub struct ReplyAssembler {
    total: Option<usize>,
    next: u32,
    payload: String,
    complete: bool,
}

impl ReplyAssembler {
    pub fn push(
        &mut self,
        sequence: u32,
        total_bytes: u32,
        last: bool,
        fragment: &str,
    ) -> Result<Option<String>, String> {
        let total = total_bytes as usize;
        if self.complete || sequence != self.next {
            return Err("Dashboard response fragments arrived out of order".into());
        }
        if total == 0 || total > MAX_REPLY_BYTES || fragment.len() > FRAGMENT_BYTES {
            return Err("Dashboard response exceeds the transport byte budget".into());
        }
        if self.total.is_some_and(|expected| expected != total) {
            return Err("Dashboard response size changed during delivery".into());
        }
        let next_len = self
            .payload
            .len()
            .checked_add(fragment.len())
            .ok_or("Dashboard response size overflow")?;
        if fragment.is_empty() || next_len > total || last != (next_len == total) {
            return Err("Dashboard response has inconsistent fragment boundaries".into());
        }
        if self.total.is_none() {
            self.payload
                .try_reserve_exact(total)
                .map_err(|_| "Not enough memory for the dashboard response")?;
            self.total = Some(total);
        }
        self.payload.push_str(fragment);
        self.next += 1;
        if last {
            self.complete = true;
            Ok(Some(std::mem::take(&mut self.payload)))
        } else {
            Ok(None)
        }
    }
}

struct JsonBuffer<'a> {
    bytes: Vec<u8>,
    cancelled: &'a AtomicBool,
}

impl Write for JsonBuffer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancelled.load(Ordering::Acquire) {
            // Write::write_all retries Interrupted; cancellation must terminate
            // serialization instead of entering that retry loop.
            return Err(io::Error::other("Dashboard request was cancelled"));
        }
        let length = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|length| *length <= MAX_REPLY_BYTES)
            .ok_or_else(|| {
                io::Error::other("Projected response exceeds the transport byte budget")
            })?;
        if length > self.bytes.capacity() {
            let capacity = length
                .max(self.bytes.capacity().saturating_mul(2))
                .min(MAX_REPLY_BYTES);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| io::Error::other("Not enough memory for projected response"))?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Serialize within the reply budget, stopping as soon as the request is cancelled.
pub fn bounded_json(value: &impl Serialize, cancelled: &AtomicBool) -> Result<String, String> {
    let mut output = JsonBuffer {
        bytes: Vec::new(),
        cancelled,
    };
    serde_json::to_writer(&mut output, value).map_err(|error| error.to_string())?;
    if cancelled.load(Ordering::Acquire) {
        return Err("Dashboard request was cancelled".into());
    }
    String::from_utf8(output.bytes).map_err(|error| error.to_string())
}

pub fn bounded_text(value: &str, maximum: usize) -> String {
    let mut end = value.len().min(maximum);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_settings_targets_keep_legacy_campaign_scope_and_validate_widget_ids() {
        let legacy: DataCommand = serde_json::from_str(
            r#"{"type":"mapSettings","id":"r","sourceId":"s","revision":"v","settings":null}"#,
        )
        .unwrap();
        assert!(matches!(
            legacy,
            DataCommand::MapSettings {
                target: MapSettingsTarget::Campaign,
                ..
            }
        ));
        assert_eq!(
            MapSettingsTarget::Global.key("campaign").unwrap(),
            "$global"
        );
        assert_eq!(
            MapSettingsTarget::Widget { id: "map-a".into() }
                .key("campaign")
                .unwrap(),
            "campaign/widget/map-a"
        );
        for id in [String::new(), "x".repeat(129), "a\0b".into()] {
            assert!(MapSettingsTarget::Widget { id }.key("campaign").is_err());
        }
    }

    #[test]
    fn product_requests_carry_opaque_params_and_validate_identity() {
        let command: DataCommand = serde_json::from_str(
            r#"{"type":"product","id":"p1","request":{"sourceId":"lease-1","revision":"r","product":"national-focus","params":{"branchId":"1","tag":"GER"}}}"#,
        )
        .unwrap();
        let DataCommand::Product { request, .. } = command else {
            panic!("wrong command");
        };
        assert!(request.validate().is_ok());
        assert_eq!(request.params["tag"], "GER");
        let mut bad = request.clone();
        bad.product = String::new();
        assert!(bad.validate().is_err());
        bad = request;
        bad.revision = String::new();
        assert!(bad.validate().is_err());
    }

    #[test]
    fn projected_json_survives_unicode_and_control_frame_escaping() {
        let payload = serde_json::to_string(&"海\"\\\n🙂".repeat(12_000)).unwrap();
        let mut assembler = ReplyAssembler::default();
        let mut restored = None;
        for event in ReplyChunks::new("query-1".into(), payload.clone()).unwrap() {
            // Include the actual enclosing NativeEvent budget, not just payload.
            let envelope = crate::protocol::NativeEvent::Data {
                response: event.clone(),
            };
            assert!(
                serde_json::to_vec(&envelope).unwrap().len() < crate::protocol::MAX_MESSAGE_BYTES
            );
            let DataEvent::Chunk {
                sequence,
                total_bytes,
                last,
                fragment,
                ..
            } = event
            else {
                unreachable!();
            };
            if let Some(value) = assembler
                .push(sequence, total_bytes, last, &fragment)
                .unwrap()
            {
                restored = Some(value);
            }
        }
        assert_eq!(restored, Some(payload));
    }

    #[test]
    fn incomplete_reordered_or_changed_size_responses_never_publish() {
        let mut assembler = ReplyAssembler::default();
        assert_eq!(assembler.push(0, 6, false, "abc").unwrap(), None);
        assert!(assembler.push(2, 6, true, "def").is_err());
        assert!(assembler.push(1, 7, false, "def").is_err());
        assert_eq!(
            assembler.push(1, 6, true, "def").unwrap(),
            Some("abcdef".into())
        );
        assert!(assembler.push(2, 6, true, "x").is_err());
        assert!(
            ReplyAssembler::default()
                .push(0, (MAX_REPLY_BYTES + 1) as u32, true, "{}")
                .is_err()
        );
    }

    #[test]
    fn cancellation_interrupts_serialization_without_retrying_the_writer() {
        struct CancelDuringSerialization<'a>(&'a AtomicBool);
        impl Serialize for CancelDuringSerialization<'_> {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(Some(2))?;
                sequence.serialize_element("first")?;
                self.0.store(true, Ordering::Release);
                sequence.serialize_element("cancelled")?;
                sequence.end()
            }
        }
        let cancelled = AtomicBool::new(false);
        let error = bounded_json(&CancelDuringSerialization(&cancelled), &cancelled).unwrap_err();
        assert!(error.contains("cancelled"));
        let mut buffer = JsonBuffer {
            bytes: Vec::new(),
            cancelled: &cancelled,
        };
        assert_ne!(
            buffer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert!(buffer.bytes.is_empty());
    }
}
