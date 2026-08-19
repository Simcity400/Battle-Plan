
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const MAP_FURNITURE_SCHEMA: &str = "hoi4.map-furniture-state.a729.v3";
pub const MAP_FURNITURE_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurnitureWorldPoint {
    pub x_bits: u32,
    pub y_bits: u32,
    pub z_bits: u32,
}

impl MapFurnitureWorldPoint {
    pub fn to_f32(self) -> Result<[f32; 3], String> {
        let point = [
            f32::from_bits(self.x_bits),
            f32::from_bits(self.y_bits),
            f32::from_bits(self.z_bits),
        ];
        if point.iter().all(|value| value.is_finite()) {
            Ok(point)
        } else {
            Err("world point contains a non-finite f32 bit pattern".into())
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VictoryPointFurnitureRow {
    pub province_id: u32,
    pub current_value: i32,
    pub controller_tag: Option<String>,
    pub controller_tag_db_id: Option<u32>,
    pub anchor: MapFurnitureWorldPoint,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapitalAnchorSource {
    StatePosition,
    FirstStateProvince,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapitalLabelSource {
    Province { province_id: u32 },
    State { state_id: u32 },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapitalFurnitureRow {
    pub country_tag: String,
    pub country_tag_db_id: u32,
    pub capital_state_id: u32,
    pub controller_tag: Option<String>,
    pub controller_tag_db_id: Option<u32>,
    pub anchor_source: CapitalAnchorSource,
    pub source_province_id: Option<u32>,
    pub label_source: CapitalLabelSource,
    pub anchor: MapFurnitureWorldPoint,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceViewerAmount {
    pub viewer_tag: String,
    pub viewer_tag_db_id: u32,
    pub available_amount: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFurnitureEntry {
    pub resource_name: String,
    pub resource_definition_ordinal: u32,
    pub total_amount: i64,
    pub viewer_amounts: Vec<ResourceViewerAmount>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceFurnitureRow {
    pub state_id: u32,
    pub controller_tag: Option<String>,
    pub controller_tag_db_id: Option<u32>,
    pub controller_flag_texture_path: Option<String>,
    pub anchor: MapFurnitureWorldPoint,
    pub entries: Vec<ResourceFurnitureEntry>,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RaidMapIconVariant {
    Target,
    Unit,
    Active,
    Enemy,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RaidFurnitureIdentity {
    pub producer_ordinal: u32,
    pub status_index: u32,
    pub raid_index: u32,
    pub id: u32,
    pub id_type: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RaidFurniturePresentation {
    pub viewer_tag: Option<String>,
    pub viewer_tag_db_id: Option<u32>,
    pub variant: Option<RaidMapIconVariant>,
    pub frame_sprite_name: Option<String>,
    pub frame_index: Option<u32>,
    pub button_sprite_name: Option<String>,
    pub button_frame: Option<u32>,
    pub plus_sprite_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RaidFurnitureRow {
    pub raid_identities: Vec<RaidFurnitureIdentity>,
    pub anchor: MapFurnitureWorldPoint,
    pub presentations: Vec<RaidFurniturePresentation>,
    pub map_icon_ordinal: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MapFurnitureFrame {
    pub schema: String,
    pub executable_sha256: String,
    pub update_index: u64,
    pub complete: bool,
    pub victory_points: Vec<VictoryPointFurnitureRow>,
    pub capitals: Vec<CapitalFurnitureRow>,
    pub resources: Vec<ResourceFurnitureRow>,
    pub raids: Vec<RaidFurnitureRow>,
}

impl MapFurnitureFrame {
    pub fn decode_value(value: &Value, displayed_hour: i64) -> Result<Self, String> {
        let frame: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid map_furniture carrier: {error}"))?;
        frame.validate(displayed_hour)?;
        Ok(frame)
    }

    pub fn validate(&self, displayed_hour: i64) -> Result<(), String> {
        if self.schema != MAP_FURNITURE_SCHEMA {
            return Err(format!(
                "schema {:?} is not {MAP_FURNITURE_SCHEMA:?}",
                self.schema
            ));
        }
        if self.executable_sha256 != MAP_FURNITURE_A729_EXECUTABLE_SHA256 {
            return Err("carrier executable identity is not exact a729".into());
        }
        if u64::try_from(displayed_hour).ok() != Some(self.update_index) {
            return Err(format!(
                "carrier update_index {} differs from displayed WorldFrame hour {displayed_hour}",
                self.update_index
            ));
        }
        if !self.complete {
            return Err("carrier does not attest a complete active icon census".into());
        }

        let mut province_ids = BTreeSet::new();
        let mut capital_countries = BTreeSet::new();
        let mut resource_states = BTreeSet::new();
        let mut raid_identities = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        for row in &self.victory_points {
            if row.province_id == 0 || !province_ids.insert(row.province_id) {
                return Err(format!(
                    "victory-point province id {} is zero or duplicated",
                    row.province_id
                ));
            }
            validate_optional_tag_pair(
                "victory-point controller",
                row.controller_tag.as_deref(),
                row.controller_tag_db_id,
            )?;
            row.anchor.to_f32()?;
            if !ordinals.insert(row.map_icon_ordinal) {
                return Err(format!(
                    "map icon ordinal {} is duplicated",
                    row.map_icon_ordinal
                ));
            }
        }
        for row in &self.capitals {
            validate_tag_pair("capital country", &row.country_tag, row.country_tag_db_id)?;
            validate_optional_tag_pair(
                "capital controller",
                row.controller_tag.as_deref(),
                row.controller_tag_db_id,
            )?;
            if row.capital_state_id == 0 || !capital_countries.insert(row.country_tag.as_str()) {
                return Err(format!(
                    "capital country {:?} has a zero state or duplicate row",
                    row.country_tag
                ));
            }
            match (row.anchor_source, row.source_province_id) {
                (CapitalAnchorSource::StatePosition, None)
                | (CapitalAnchorSource::FirstStateProvince, Some(1..)) => {}
                _ => {
                    return Err(format!(
                        "capital {:?} has inconsistent anchor source/province identity",
                        row.country_tag
                    ));
                }
            }
            match row.label_source {
                CapitalLabelSource::Province { province_id } if province_id > 0 => {}
                CapitalLabelSource::State { state_id } if state_id == row.capital_state_id => {}
                _ => {
                    return Err(format!(
                        "capital {:?} has an invalid display-text source",
                        row.country_tag
                    ));
                }
            }
            row.anchor.to_f32()?;
            if !ordinals.insert(row.map_icon_ordinal) {
                return Err(format!(
                    "map icon ordinal {} is duplicated",
                    row.map_icon_ordinal
                ));
            }
        }
        for row in &self.resources {
            if row.state_id == 0 || !resource_states.insert(row.state_id) {
                return Err(format!(
                    "resource state id {} is zero or duplicated",
                    row.state_id
                ));
            }
            validate_optional_tag_pair(
                "resource-state controller",
                row.controller_tag.as_deref(),
                row.controller_tag_db_id,
            )?;
            match (
                row.controller_tag.as_deref(),
                row.controller_flag_texture_path.as_deref(),
            ) {
                (None, None) => {}
                (Some(_), Some(path)) => {
                    validate_content_path("resource controller flag texture", path)?
                }
                _ => {
                    return Err(format!(
                        "resource state {} controller/flag identity is incomplete",
                        row.state_id
                    ))
                }
            }
            row.anchor.to_f32()?;
            if row.entries.is_empty() {
                return Err(format!(
                    "resource state {} has no positive entries",
                    row.state_id
                ));
            }
            let mut names = BTreeSet::new();
            let mut definition_ordinals = BTreeSet::new();
            let mut previous_ordinal = None;
            for entry in &row.entries {
                validate_content_token("resource name", &entry.resource_name)?;
                if entry.total_amount <= 0
                    || !names.insert(entry.resource_name.as_str())
                    || !definition_ordinals.insert(entry.resource_definition_ordinal)
                    || previous_ordinal
                        .is_some_and(|previous| entry.resource_definition_ordinal <= previous)
                {
                    return Err(format!(
                        "resource state {} has a non-positive, duplicate, or unordered entry {:?}",
                        row.state_id, entry.resource_name
                    ));
                }
                previous_ordinal = Some(entry.resource_definition_ordinal);
                let mut viewers = BTreeSet::new();
                for viewer in &entry.viewer_amounts {
                    validate_tag_pair(
                        "resource viewer",
                        &viewer.viewer_tag,
                        viewer.viewer_tag_db_id,
                    )?;
                    if !viewers.insert(viewer.viewer_tag.as_str()) {
                        return Err(format!(
                            "resource {:?} in state {} duplicates viewer {:?}",
                            entry.resource_name, row.state_id, viewer.viewer_tag
                        ));
                    }
                }
            }
            if !ordinals.insert(row.map_icon_ordinal) {
                return Err(format!(
                    "map icon ordinal {} is duplicated",
                    row.map_icon_ordinal
                ));
            }
        }
        for row in &self.raids {
            if row.raid_identities.is_empty() || row.presentations.is_empty() {
                return Err("raid icon has no identities or presentation frontier".into());
            }
            let mut previous_identity = None;
            for identity in &row.raid_identities {
                if identity.id == 0
                    || !raid_identities.insert(identity.clone())
                    || previous_identity
                        .as_ref()
                        .is_some_and(|previous| identity <= previous)
                {
                    return Err("raid icon identities are zero, duplicated, or unordered".into());
                }
                previous_identity = Some(identity.clone());
            }
            let mut viewers = BTreeSet::new();
            let mut has_no_viewer = false;
            for presentation in &row.presentations {
                validate_optional_tag_pair(
                    "raid viewer",
                    presentation.viewer_tag.as_deref(),
                    presentation.viewer_tag_db_id,
                )?;
                if !viewers.insert(presentation.viewer_tag.as_deref()) {
                    return Err("raid presentation duplicates a viewer identity".into());
                }
                has_no_viewer |= presentation.viewer_tag.is_none();
                match presentation.variant {
                    None => {
                        if presentation.frame_sprite_name.is_some()
                            || presentation.frame_index.is_some()
                            || presentation.button_sprite_name.is_some()
                            || presentation.button_frame.is_some()
                            || presentation.plus_sprite_name.is_some()
                        {
                            return Err(
                                "hidden raid presentation carries visible sprite state".into()
                            );
                        }
                    }
                    Some(_) => {
                        let frame = presentation
                            .frame_sprite_name
                            .as_deref()
                            .ok_or("visible raid presentation has no frame sprite")?;
                        let button = presentation
                            .button_sprite_name
                            .as_deref()
                            .ok_or("visible raid presentation has no button sprite")?;
                        validate_content_token("raid frame sprite", frame)?;
                        validate_content_token("raid button sprite", button)?;
                        if presentation.frame_index.is_none() || presentation.button_frame.is_none()
                        {
                            return Err(
                                "visible raid presentation has incomplete frame identity".into()
                            );
                        }
                        if let Some(plus) = presentation.plus_sprite_name.as_deref() {
                            validate_content_token("raid plus sprite", plus)?;
                        }
                    }
                }
            }
            if !has_no_viewer {
                return Err("raid icon has no exact no-viewer presentation".into());
            }
            row.anchor.to_f32()?;
            if !ordinals.insert(row.map_icon_ordinal) {
                return Err(format!(
                    "map icon ordinal {} is duplicated",
                    row.map_icon_ordinal
                ));
            }
        }
        Ok(())
    }
}

fn validate_content_token(role: &str, token: &str) -> Result<(), String> {
    if token.is_empty()
        || token.len() > 192
        || token
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\' || byte == b'/')
    {
        return Err(format!("{role} {token:?} is not a portable content token"));
    }
    Ok(())
}

fn validate_content_path(role: &str, path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 512
        || path.starts_with('/')
        || path.starts_with('\\')
        || path.contains(':')
        || path.split(['/', '\\']).any(|segment| {
            segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.bytes().any(|byte| byte.is_ascii_control())
        })
    {
        return Err(format!("{role} {path:?} is not a portable game path"));
    }
    Ok(())
}

fn validate_tag_pair(role: &str, tag: &str, tag_db_id: u32) -> Result<(), String> {
    if tag.is_empty()
        || tag.len() > 32
        || !tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        || tag_db_id == 0
    {
        return Err(format!(
            "{role} identity ({tag:?}, {tag_db_id}) is not a measured tag/db-id pair"
        ));
    }
    Ok(())
}

fn validate_optional_tag_pair(
    role: &str,
    tag: Option<&str>,
    tag_db_id: Option<u32>,
) -> Result<(), String> {
    match (tag, tag_db_id) {
        (None, None) => Ok(()),
        (Some(tag), Some(tag_db_id)) => validate_tag_pair(role, tag, tag_db_id),
        _ => Err(format!("{role} must carry both tag and db-id or neither")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn point() -> Value {
        json!({
            "x_bits": 1.0f32.to_bits(),
            "y_bits": 2.0f32.to_bits(),
            "z_bits": 3.0f32.to_bits()
        })
    }

    #[test]
    fn admits_complete_exact_hour_census() {
        let value = json!({
            "schema": MAP_FURNITURE_SCHEMA,
            "executable_sha256": MAP_FURNITURE_A729_EXECUTABLE_SHA256,
            "update_index": 17,
            "complete": true,
            "victory_points": [{
                "province_id": 42,
                "current_value": 7,
                "controller_tag": "GER",
                "controller_tag_db_id": 5,
                "anchor": point(),
                "map_icon_ordinal": 2
            }],
            "capitals": [{
                "country_tag": "GER",
                "country_tag_db_id": 5,
                "capital_state_id": 64,
                "controller_tag": "GER",
                "controller_tag_db_id": 5,
                "anchor_source": "first_state_province",
                "source_province_id": 42,
                "label_source": { "kind": "province", "province_id": 42 },
                "anchor": point(),
                "map_icon_ordinal": 3
            }],
            "resources": [{
                "state_id": 64,
                "controller_tag": "GER",
                "controller_tag_db_id": 5,
                "controller_flag_texture_path": "gfx/flags/GER_fascism.tga",
                "anchor": point(),
                "entries": [{
                    "resource_name": "steel",
                    "resource_definition_ordinal": 5,
                    "total_amount": 9,
                    "viewer_amounts": [{
                        "viewer_tag": "GER",
                        "viewer_tag_db_id": 5,
                        "available_amount": 9
                    }]
                }],
                "map_icon_ordinal": 4
            }],
            "raids": [{
                "raid_identities": [{
                    "producer_ordinal": 1,
                    "status_index": 2,
                    "raid_index": 3,
                    "id": 4,
                    "id_type": 5
                }],
                "anchor": point(),
                "presentations": [{
                    "viewer_tag": null,
                    "viewer_tag_db_id": null,
                    "variant": null,
                    "frame_sprite_name": null,
                    "frame_index": null,
                    "button_sprite_name": null,
                    "button_frame": null,
                    "plus_sprite_name": null
                }, {
                    "viewer_tag": "GER",
                    "viewer_tag_db_id": 5,
                    "variant": "active",
                    "frame_sprite_name": "GFX_on_map_raids_icons_frame_strip",
                    "frame_index": 0,
                    "button_sprite_name": "GFX_raid_target_icon_generic",
                    "button_frame": 0,
                    "plus_sprite_name": null
                }],
                "map_icon_ordinal": 5
            }]
        });
        let admitted = MapFurnitureFrame::decode_value(&value, 17).unwrap();
        assert_eq!(
            admitted.victory_points[0].anchor.to_f32().unwrap(),
            [1.0, 2.0, 3.0]
        );
    }

    #[test]
    fn rejects_cross_hour_incomplete_and_duplicate_frontiers() {
        let value = json!({
            "schema": MAP_FURNITURE_SCHEMA,
            "executable_sha256": MAP_FURNITURE_A729_EXECUTABLE_SHA256,
            "update_index": 17,
            "complete": true,
            "victory_points": [
                {
                    "province_id": 42,
                    "current_value": 7,
                    "controller_tag": "GER",
                    "controller_tag_db_id": 5,
                    "anchor": point(),
                    "map_icon_ordinal": 2
                },
                {
                    "province_id": 42,
                    "current_value": 8,
                    "controller_tag": "GER",
                    "controller_tag_db_id": 5,
                    "anchor": point(),
                    "map_icon_ordinal": 3
                }
            ],
            "capitals": [],
            "resources": [],
            "raids": []
        });
        assert!(MapFurnitureFrame::decode_value(&value, 16).is_err());
        assert!(MapFurnitureFrame::decode_value(&value, 17)
            .unwrap_err()
            .contains("duplicated"));
    }
}
