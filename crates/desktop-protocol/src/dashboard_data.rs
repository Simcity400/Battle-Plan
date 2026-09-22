//! Bounded transport of projected analytics results and country-flag thumbnails,
//! never replay tables or map pixels.

use analytics::{QueryRequest, ReplayMetadata};
use serde::{Deserialize, Serialize};

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
        settings: Option<crate::CampaignMapSettings>,
    },
    Open {
        id: String,
        path: String,
        revision: String,
    },
    Query {
        id: String,
        request: QueryRequest,
    },
    CountryFlags {
        id: String,
        request: CountryFlagRequest,
    },
    FocusHistory {
        id: String,
        request: FocusRequest,
    },
    ResearchHistory {
        id: String,
        request: FocusRequest,
    },
    LandOwners {
        id: String,
        #[serde(rename = "sourceId")]
        source_id: String,
        revision: String,
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
            | Self::Query { id, .. }
            | Self::CountryFlags { id, .. }
            | Self::FocusHistory { id, .. }
            | Self::ResearchHistory { id, .. }
            | Self::LandOwners { id, .. }
            | Self::Cancel { id } => Some(id),
            Self::Release { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusRequest {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub tag: String,
}

impl FocusRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        CountryFlagRequest {
            source_id: self.source_id.clone(),
            revision: self.revision.clone(),
            branch_id: self.branch_id.clone(),
            hour: 0,
            tags: vec![self.tag.clone()],
            size: CountryFlagSize::Full,
        }
        .validate()
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusArtwork {
    pub token: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub base_duration_days: Option<f64>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusReply {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub tag: String,
    pub history: analytics::FocusHistory,
    pub artwork: Vec<FocusArtwork>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReply {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub tag: String,
    pub history: analytics::ResearchHistory,
    pub artwork: Vec<FocusArtwork>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CountryFlagSize {
    #[default]
    Full,
    Small,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LandOwnersReply {
    pub source_id: String,
    pub revision: String,
    pub countries: Vec<analytics::CountryMetadata>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryFlagRequest {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub hour: i32,
    pub tags: Vec<String>,
    #[serde(default)]
    pub size: CountryFlagSize,
}

impl CountryFlagRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.tags.is_empty()
            || self.tags.len() > 32
            || self.tags.iter().any(|tag| {
                tag.is_empty()
                    || tag.len() > 128
                    || !tag
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err("Invalid country flag selection");
        }
        if self.source_id.is_empty()
            || self.source_id.len() > 128
            || self.revision.is_empty()
            || self.revision.len() > 128
            || self.branch_id.parse::<u64>().is_err()
        {
            return Err("Invalid country flag source");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagImage {
    pub tag: String,
    pub asset: String,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryFlagReply {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub hour: i32,
    pub flags: Vec<CountryFlagImage>,
    pub warnings: Vec<String>,
    pub size: CountryFlagSize,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

    pub fn request_id(&self) -> &str {
        &self.id
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
    pub fn retained_bytes(&self) -> usize {
        self.payload.capacity()
    }

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

#[cfg(test)]
mod tests {
    #[test]
    fn map_settings_targets_keep_legacy_campaign_scope_and_validate_widget_ids() {
        let legacy: super::DataCommand = serde_json::from_str(
            r#"{"type":"mapSettings","id":"r","sourceId":"s","revision":"v","settings":null}"#,
        )
        .unwrap();
        assert!(matches!(
            legacy,
            super::DataCommand::MapSettings {
                target: super::MapSettingsTarget::Campaign,
                ..
            }
        ));
        assert_eq!(
            super::MapSettingsTarget::Global.key("campaign").unwrap(),
            "$global"
        );
        assert_eq!(
            super::MapSettingsTarget::Widget { id: "map-a".into() }
                .key("campaign")
                .unwrap(),
            "campaign/widget/map-a"
        );
        for id in [String::new(), "x".repeat(129), "a\0b".into()] {
            assert!(
                super::MapSettingsTarget::Widget { id }
                    .key("campaign")
                    .is_err()
            );
        }
    }

    use super::*;

    #[test]
    fn projected_json_survives_unicode_and_control_frame_escaping() {
        let payload = serde_json::to_string(&"海\"\\\n🙂".repeat(12_000)).unwrap();
        let mut assembler = ReplyAssembler::default();
        let mut restored = None;
        for event in ReplyChunks::new("query-1".into(), payload.clone()).unwrap() {
            // Include the actual enclosing NativeEvent budget, not just payload.
            let envelope = crate::NativeEvent::Data {
                response: event.clone(),
            };
            assert!(serde_json::to_vec(&envelope).unwrap().len() < crate::MAX_MESSAGE_BYTES);
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
}

#[cfg(test)]
mod flag_tests {
    use super::*;
    #[test]
    fn flag_size_defaults_to_full_and_rejects_unknown_asset_classes() {
        let mut value = serde_json::json!({
            "sourceId": "lease-1", "revision": "r1", "branchId": "1", "hour": 0, "tags": ["GER"]
        });
        let request: CountryFlagRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.size, CountryFlagSize::Full);
        value["size"] = serde_json::json!("small");
        let request: CountryFlagRequest = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(request.size, CountryFlagSize::Small);
        value["size"] = serde_json::json!("../other");
        assert!(serde_json::from_value::<CountryFlagRequest>(value).is_err());
    }

    #[test]
    fn flags_accept_mod_tags_but_reject_paths_and_unbounded_batches() {
        let mut request = CountryFlagRequest {
            source_id: "lease-1".into(),
            revision: "r1".into(),
            branch_id: "1".into(),
            hour: 0,
            tags: vec!["Z01".into(), "MOD_TAG".into()],
            size: CountryFlagSize::Full,
        };
        assert!(request.validate().is_ok());
        request.tags = vec!["../secret".into()];
        assert!(request.validate().is_err());
        request.tags = vec!["Z01".into(); 33];
        assert!(request.validate().is_err());
    }
}
