//! A31/XUI10 exact national-focus presentation producer boundary.
//!
//! This module deliberately does not re-evaluate focus triggers or derive
//! presentation state from authored definitions.  An exact a729 view observer
//! must provide the final `CNationalFocusTreeItem` sequence and fields proved
//! in the A31 dossier.  The validator makes that boundary usable without
//! enabling a schedulable domain before a live observer exists.

use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 1;
pub const EXACT_A729_EXECUTABLE_SHA256: [u8; 32] = [
    0x87, 0x0e, 0xbf, 0x1f, 0x61, 0x52, 0x62, 0xfa, 0xcc, 0xa4, 0x8a, 0x3b, 0x94, 0x77, 0x75, 0xc4,
    0x1f, 0x0a, 0x53, 0x38, 0xf3, 0x06, 0x8c, 0x40, 0x95, 0x82, 0x09, 0xf5, 0xc4, 0x1c, 0x65, 0xad,
];
pub const MAX_VISIBLE_ITEMS: usize = 131_072;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusPresentationItem {
    /// Stable focus-definition id from item+0x20.
    pub focus_id: String,
    /// Index in the final view+0x2058 vector after 0x14136f750.
    pub source_ordinal: u32,
    /// Final item+0x540 state: 0 unavailable, 1 complete, 2 available, 3 active.
    pub presentation_state_code: u8,
    /// Final integer supplied by 0x141bb5df0.  It is not progress_days/cost*7.
    pub display_progress_percent: u8,
    /// Final search-highlight state 3 disposition from 0x141377430.
    pub highlight_glow_visible: bool,
    /// Final focus-definition+0x5b8 byte disposition.
    pub historical_marker_visible: bool,
    /// Positive country identity returned by the joint-focus viewing helper.
    pub joint_viewing_country_tag: Option<String>,
}

impl FocusPresentationItem {
    pub fn available(&self) -> bool {
        self.presentation_state_code == 2
    }

    pub fn continuous_glow_visible(&self) -> bool {
        self.presentation_state_code == 3
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NationalFocusPresentationFrame {
    pub frame_hour: u64,
    pub executable_sha256: [u8; 32],
    pub country_tag: String,
    pub view_id: String,
    pub producer_record_identity: String,
    pub producer_manifest_sha256: [u8; 32],
    pub expected_visible_item_count: u32,
    /// Engine-published order, never authored or coordinate-sorted order.
    pub visible_items: Vec<FocusPresentationItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProducerError {
    UnauthenticatedSource,
    Source(String),
    Invalid(String),
}

pub trait ExactNationalFocusPresentationSource {
    fn is_exact_a729_focus_view_source(&self) -> bool;
    fn producer_manifest_sha256(&self) -> [u8; 32];
    fn collect_frame(
        &self,
        frame_hour: u64,
        country_tag: &str,
    ) -> Result<NationalFocusPresentationFrame, String>;
}

pub fn collect_and_validate<S: ExactNationalFocusPresentationSource>(
    source: &S,
    frame_hour: u64,
    country_tag: &str,
) -> Result<NationalFocusPresentationFrame, ProducerError> {
    if !source.is_exact_a729_focus_view_source() {
        return Err(ProducerError::UnauthenticatedSource);
    }
    let frame = source
        .collect_frame(frame_hour, country_tag)
        .map_err(ProducerError::Source)?;
    if frame.producer_manifest_sha256 != source.producer_manifest_sha256() {
        return invalid("frame/source manifest mismatch");
    }
    if frame.frame_hour != frame_hour || frame.country_tag != country_tag {
        return invalid("requested and returned snapshot identities differ");
    }
    validate_frame(&frame)?;
    Ok(frame)
}

pub fn validate_frame(frame: &NationalFocusPresentationFrame) -> Result<(), ProducerError> {
    if frame.executable_sha256 != EXACT_A729_EXECUTABLE_SHA256 {
        return invalid("national-focus frame is not bound to exact a729 executable");
    }
    validate_country_tag(&frame.country_tag)?;
    validate_nonempty(&frame.view_id, "view id")?;
    validate_nonempty(&frame.producer_record_identity, "producer record identity")?;
    if frame.producer_manifest_sha256 == [0; 32] {
        return invalid("producer manifest hash is zero");
    }
    if frame.visible_items.len() > MAX_VISIBLE_ITEMS
        || frame.expected_visible_item_count as usize != frame.visible_items.len()
    {
        return invalid("visible-item completeness/count mismatch");
    }

    let mut ids = BTreeSet::new();
    for (ordinal, item) in frame.visible_items.iter().enumerate() {
        validate_nonempty(&item.focus_id, "focus id")?;
        if !ids.insert(item.focus_id.as_str()) {
            return invalid("duplicate focus id in visible sequence");
        }
        if item.source_ordinal as usize != ordinal {
            return invalid("visible sequence ordinals are not contiguous");
        }
        if item.presentation_state_code > 3 {
            return invalid("presentation state code outside proven 0..3 domain");
        }
        if item.display_progress_percent > 100 {
            return invalid("display progress exceeds engine percent domain");
        }
        match item.presentation_state_code {
            0 | 2 if item.display_progress_percent != 0 => {
                return invalid("not-started focus carries nonzero display progress")
            }
            1 if item.display_progress_percent != 100 => {
                return invalid("completed focus does not carry 100 display progress")
            }
            _ => {}
        }
        if let Some(tag) = &item.joint_viewing_country_tag {
            validate_country_tag(tag)?;
        }
    }
    Ok(())
}

fn validate_country_tag(tag: &str) -> Result<(), ProducerError> {
    if tag.len() != 3
        || !tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    {
        return invalid("country tag is not a canonical three-byte tag");
    }
    Ok(())
}

fn validate_nonempty(value: &str, label: &str) -> Result<(), ProducerError> {
    if value.is_empty() || value.len() > 65_536 || value.chars().any(char::is_control) {
        return invalid(&format!(
            "{label} is empty, unbounded, or contains controls"
        ));
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T, ProducerError> {
    Err(ProducerError::Invalid(message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> NationalFocusPresentationFrame {
        NationalFocusPresentationFrame {
            frame_hour: 42,
            executable_sha256: EXACT_A729_EXECUTABLE_SHA256,
            country_tag: "GER".into(),
            view_id: "nationalfocusview:GER".into(),
            producer_record_identity: "a31:42:GER:7".into(),
            producer_manifest_sha256: [7; 32],
            expected_visible_item_count: 4,
            visible_items: vec![
                FocusPresentationItem {
                    focus_id: "ger_a".into(),
                    source_ordinal: 0,
                    presentation_state_code: 0,
                    display_progress_percent: 0,
                    highlight_glow_visible: false,
                    historical_marker_visible: true,
                    joint_viewing_country_tag: None,
                },
                FocusPresentationItem {
                    focus_id: "ger_b".into(),
                    source_ordinal: 1,
                    presentation_state_code: 1,
                    display_progress_percent: 100,
                    highlight_glow_visible: false,
                    historical_marker_visible: false,
                    joint_viewing_country_tag: None,
                },
                FocusPresentationItem {
                    focus_id: "ger_c".into(),
                    source_ordinal: 2,
                    presentation_state_code: 2,
                    display_progress_percent: 0,
                    highlight_glow_visible: true,
                    historical_marker_visible: false,
                    joint_viewing_country_tag: None,
                },
                FocusPresentationItem {
                    focus_id: "ger_d".into(),
                    source_ordinal: 3,
                    presentation_state_code: 3,
                    display_progress_percent: 37,
                    highlight_glow_visible: false,
                    historical_marker_visible: false,
                    joint_viewing_country_tag: Some("ITA".into()),
                },
            ],
        }
    }

    #[test]
    fn exact_frame_preserves_engine_order_and_final_leaf_semantics() {
        let value = frame();
        validate_frame(&value).unwrap();
        assert!(!value.visible_items[0].available());
        assert!(value.visible_items[2].available());
        assert!(value.visible_items[3].continuous_glow_visible());
    }

    #[test]
    fn inferred_or_incomplete_focus_frames_fail_closed() {
        let mut value = frame();
        value.visible_items[2].source_ordinal = 7;
        assert!(validate_frame(&value).is_err());
        let mut value = frame();
        value.visible_items[2].display_progress_percent = 12;
        assert!(validate_frame(&value).is_err());
        let mut value = frame();
        value.expected_visible_item_count -= 1;
        assert!(validate_frame(&value).is_err());
    }

    struct Source {
        authenticated: bool,
    }

    impl ExactNationalFocusPresentationSource for Source {
        fn is_exact_a729_focus_view_source(&self) -> bool {
            self.authenticated
        }
        fn producer_manifest_sha256(&self) -> [u8; 32] {
            [7; 32]
        }
        fn collect_frame(
            &self,
            _hour: u64,
            _tag: &str,
        ) -> Result<NationalFocusPresentationFrame, String> {
            Ok(frame())
        }
    }

    #[test]
    fn source_authentication_and_snapshot_identity_are_mandatory() {
        assert_eq!(
            collect_and_validate(
                &Source {
                    authenticated: false
                },
                42,
                "GER"
            ),
            Err(ProducerError::UnauthenticatedSource)
        );
        assert!(collect_and_validate(
            &Source {
                authenticated: true
            },
            43,
            "GER"
        )
        .is_err());
        assert!(collect_and_validate(
            &Source {
                authenticated: true
            },
            42,
            "GER"
        )
        .is_ok());
    }
}
