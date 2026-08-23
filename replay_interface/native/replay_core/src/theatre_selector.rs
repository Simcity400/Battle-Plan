
use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const THEATRE_SELECTOR_AUTHORITY: &str = "exact_a729_all_country_theatre_selector";
pub const THEATRE_SELECTOR_SCHEMA_VERSION: u64 = 6;
pub const THEATRE_SELECTOR_FIELD: &str = "_r2_theatre_selector_live";
pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const REQUIRED_NEGATIVE_CONTROLLER_DISPOSITIONS: &[&str] = &[
    "theater_group_item.combat_log_ownership_and_eligibility",
    "navy_theater_group_item.combat_log_ownership_and_eligibility",
    "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
    "air_theatre_group_item.current_priority_ordinal",
];

pub const REQUIRED_PRODUCER_EVIDENCE_SHA256: &[&str] = &[
    "7b7a98857d4f1b772a78dbf5d38842029f9dbf3456caf081571b52219d2399bf",
    "8aa941bb10f7613af820251eaa4a33cce67ed3f1a72205a05e1b7b05731544bb",
    "35c8a1d66ef64dafd88d5ddfb817cd071e711996666b70a655950c0be0adeb18",
    "eee5a6c973edc425a709864071f940ebb002b4f741d138f09864dab4dc017ed6",
    "665a1b3ba8b94c8951a4ec8c338e32965b3ef4d54f1606558b527ab7e4653d3a",
    "104441634a84f61fd306a38fec3a7fed7b5d5d063d5391656ef7694af5e58f26",
    "480b3f55336059d4bab10d4b6f24be50c94a1ae3709d176dff7c648561aefa02",
    "1f7d6a31cc13b9e7dc0215a3d3372c3acbffb4e1f8111e7bf6656d84574f7a15",
    "bb9451d9a79dc9a632a1666a48a224950b568079a1615b0990e0130856a415d3",
    "702079a798896b13de06ba38ead9b4705b9dfbd4bea973fbed7cd4a8329d9fb2",
    "acdf1555a37c8c8dd170929c78bb126c3c46391ff58707fba6b0f6f825d5d5c4",
    "4827653ad8125b5d50deaaa867f5fdab6dbeea92c99d6d8752aaa0d89033bda2",
    "c9fabe9e0615caeaf28fa2744f662fec9610792871e7a62105a7040a104a8f7f",
    "18e1953fd02957c7fe8f4f888a0842e4863f0d296fa927adec8296fd03bbfb8f",
    "2938fbb8f39fb03679cfeaa104ce919ac78e48f2b289134d6c628c06ca7bc699",
    "65c50c770e6d95ea2bbc94dae93815172f4ce04fd9d4816a5e993e49aa2da25a",
    "6a92fc331d6a40d6840c13036d303b3561587f6d6436562ce83ec405d07756b1",
    "e63c5ee8dc0c742bf279f4e9f5b6ccf386dbc3e8fb18dacaba7bc72ee3cdc42a",
    "f4fe3d866beda3f8bbe4a035a3984d76f258e9d1ffdae7ed51c20c579adcfaa1",
    "602909d6ba0ac3d10464442cb6465f45c5e9e251433cf4ee4c0d8d87d3639414",
    "29a77db0b708bbf14ea9d5ef65d0e07ac99a3577805bd4402bca5a203acde140",
    "ec30788575b4af88e7f6cf6a571929e935aa66a486889d6b1753a9fec26d00c2",
    "b85e8a1108e5dee4df45d3514a35105f0efe3ac36b324c92ceb94e25ba5e3d34",
    "97dbfb5c5b4570c09fc9c6e48c574bcd62c3a54703792108132abb305b3814cf",
    "3d81b30acd0971fc4f8210db374ffec484806ad1c2ef959a3b26b53d477122a0",
    "87f483df0962653c0bb6bc622e57ae75e4071c30f8e74137043edf7e9f49500c",
    "1540de355a8b9089b3715048cc115e23e725a7d7bcc32a17977800b5d982c154",
    "6296052e11d606a8e4ae5f2824c76ed3e4b22b9cacbc2a9f3ceff908419bb576",
    "1ebaa91d151d1e60ebec9e22c1bf334324f49771da260cd91b5e54b138bdc86f",
    "ce12018aa632b86b3b5b914ffad969fe6b2776ecbb0e3e167b939695a58fd329",
    "09535b07b736af026f3737d18138884b437c1281327b002c15442a08d865b61b",
];

const MAX_COUNTRIES: usize = 4_096;
const MAX_THEATRES_PER_SECTION: usize = 16_384;
const MAX_MEMBERS_PER_SECTION: usize = 200_000;
const MAX_ALERTS_PER_THEATRE: usize = 0x52;
const MAX_FREE_WINGS_PER_SECTION: usize = 20_000;
const MAX_STRATEGIC_REGIONS_PER_SECTION: usize = 4_096;
const MAX_PRIORITY_CENSUS_PER_SECTION: usize = 20_000;
const MAX_DISPLAY_NAME_BYTES: usize = 4_096;
const MAX_CONTENT_LAYERS: usize = 4_096;
const MAX_TOKEN_BYTES: usize = 1_024;
const MAX_ALERT_CATEGORY: u32 = 0x51;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectKind {
    LandTheatre,
    NavyTheatre,
    AirTheatre,
    Army,
    ArmyGroup,
    Fleet,
    AirGroup,
    AirWing,
    TaskForce,
    ReserveFleetAggregate,
}

impl ObjectKind {
    fn from_token(token: &str) -> Option<Self> {
        Some(match token {
            "land_theatre" => Self::LandTheatre,
            "navy_theatre" => Self::NavyTheatre,
            "air_theatre" => Self::AirTheatre,
            "army" => Self::Army,
            "army_group" => Self::ArmyGroup,
            "fleet" => Self::Fleet,
            "air_group" => Self::AirGroup,
            "air_wing" => Self::AirWing,
            "task_force" => Self::TaskForce,
            "reserve_fleet_aggregate" => Self::ReserveFleetAggregate,
            _ => return None,
        })
    }

    pub const fn is_theatre(self) -> bool {
        matches!(
            self,
            Self::LandTheatre | Self::NavyTheatre | Self::AirTheatre
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProvenance {
    EngineRuntimeString,
    LocalizationKey(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub kind: ObjectKind,
    pub object_id: u32,
    pub parent_kind: ObjectKind,
    pub parent_id: u32,
    pub engine_order: u32,
    pub reserve_navy: bool,
    pub display_name: Option<String>,
    pub name_provenance: Option<NameProvenance>,
    pub controller: MemberControllerState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreRow {
    pub kind: ObjectKind,
    pub selector_included: bool,
    pub object_id: u32,
    pub owner_tag_db_id: u32,
    pub engine_order: u32,
    pub display_name: String,
    pub name_provenance: NameProvenance,
    pub controller: TheatreControllerState,
    pub members: Vec<MemberRow>,
    pub alert_categories: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeAirWingRow {
    pub wing_id: u32,
    pub engine_order: u32,
    pub active_visual_frame: u32,
    pub bucket_key: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrategicAlertGlow {
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategicAlertRow {
    pub alert_id: u32,
    pub engine_order: u32,
    pub atlas_frame: u32,
    pub glow: StrategicAlertGlow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategicNavalAccess {
    pub access_slot: i32,
    pub applicable: bool,
    pub raw_result: Option<i8>,
    pub visible_frame: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategicRegionRow {
    pub owner_tag_db_id: u32,
    pub engine_region_index: i32,
    pub display_name: String,
    pub engine_order: u32,
    pub alerts: Vec<StrategicAlertRow>,
    pub naval_access: StrategicNavalAccess,
    pub pinned_frame: u32,
    pub background_frame: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountrySectionView {
    pub status: ViewStatus,
    pub owner_tag_db_id: u32,
    pub theatres: Vec<TheatreRow>,
    pub free_air_wings: Vec<FreeAirWingRow>,
    pub free_air_priority_ordinal: Option<u32>,
    pub free_air_priority_census_wing_ids: Vec<u32>,
    pub strategic_regions: Vec<StrategicRegionRow>,
    pub rejection: Option<String>,
}

impl CountrySectionView {
    fn unavailable(owner_tag_db_id: u32, rejection: Option<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            owner_tag_db_id,
            theatres: Vec::new(),
            free_air_wings: Vec::new(),
            free_air_priority_ordinal: None,
            free_air_priority_census_wing_ids: Vec::new(),
            strategic_regions: Vec::new(),
            rejection,
        }
    }

    pub fn is_measured_empty(&self) -> bool {
        self.status == ViewStatus::Present
            && self.theatres.iter().all(|row| !row.selector_included)
            && self.free_air_wings.is_empty()
            && self.strategic_regions.is_empty()
    }

    pub fn theatres_of_kind(&self, kind: ObjectKind) -> impl Iterator<Item = &TheatreRow> {
        self.theatres
            .iter()
            .filter(move |row| row.kind == kind && row.selector_included)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLayer {
    pub ordinal: u32,
    pub layer_kind: String,
    pub layer_id: String,
    pub content_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentIdentity {
    pub language: String,
    pub localization_content_sha256: String,
    pub ordered_layers: Vec<ContentLayer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayContentExpectation {
    pub language: String,
    pub localization_content_sha256: String,
    pub ordered_layers: Vec<ContentLayer>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreSelectorView {
    pub status: ViewStatus,
    pub authority: Option<String>,
    pub frame_hour: Option<u64>,
    pub source_sequence: Option<u64>,
    pub producer_complete: bool,
    pub content_identity: Option<ContentIdentity>,
    pub census: Vec<u32>,
    sections: Vec<CountrySectionView>,
    pub negative_controller_dispositions: Vec<String>,
    pub diagnostic: Option<String>,
}

impl TheatreSelectorView {
    fn absent() -> Self {
        Self {
            status: ViewStatus::DomainAbsent,
            authority: None,
            frame_hour: None,
            source_sequence: None,
            producer_complete: false,
            content_identity: None,
            census: Vec::new(),
            sections: Vec::new(),
            negative_controller_dispositions: Vec::new(),
            diagnostic: None,
        }
    }

    fn unavailable(diagnostic: Option<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            diagnostic,
            ..Self::absent()
        }
    }

    pub fn country(&self, owner_tag_db_id: u32) -> CountrySectionView {
        if self.status != ViewStatus::Present {
            return CountrySectionView::unavailable(
                owner_tag_db_id,
                self.diagnostic
                    .clone()
                    .or_else(|| Some("frame refused".into())),
            );
        }
        self.sections
            .iter()
            .find(|section| section.owner_tag_db_id == owner_tag_db_id)
            .cloned()
            .unwrap_or_else(|| {
                CountrySectionView::unavailable(
                    owner_tag_db_id,
                    Some("country is absent from the selectable census".into()),
                )
            })
    }

    pub fn sections(&self) -> &[CountrySectionView] {
        &self.sections
    }

    pub fn authorize(
        self,
        expectation: &ReplayContentExpectation,
        expected_frame_hour: u64,
        last_source_sequence: Option<u64>,
    ) -> Self {
        if self.status != ViewStatus::Present {
            return self;
        }
        let Some(identity) = self.content_identity.as_ref() else {
            return Self::unavailable(Some("frame carried no content identity".into()));
        };
        if identity.language != expectation.language {
            return Self::unavailable(Some(format!(
                "localization language mismatch: producer {}, replay {}",
                identity.language, expectation.language
            )));
        }
        if identity.localization_content_sha256 != expectation.localization_content_sha256 {
            return Self::unavailable(Some("localization content hash mismatch".into()));
        }
        if identity.ordered_layers != expectation.ordered_layers {
            return Self::unavailable(Some("ordered mod/DLC content identity mismatch".into()));
        }
        if self.frame_hour != Some(expected_frame_hour) {
            return Self::unavailable(Some("frame hour mismatch".into()));
        }
        let Some(expected_source_sequence) = expected_frame_hour.checked_add(1) else {
            return Self::unavailable(Some(
                "frame hour cannot derive a valid source sequence".into(),
            ));
        };
        if self.source_sequence != Some(expected_source_sequence) {
            return Self::unavailable(Some(
                "source sequence does not match the frame-hour-derived producer sequence".into(),
            ));
        }
        match (self.source_sequence, last_source_sequence) {
            (None, _) | (Some(0), _) => {
                return Self::unavailable(Some("source sequence is missing or zero".into()))
            }
            (Some(current), Some(last)) if current <= last => {
                return Self::unavailable(Some("source sequence did not strictly increase".into()))
            }
            _ => {}
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TheatreControllerState {
    Land {
        priority_ordinal: u32,
        delete_eligible: bool,
    },
    Navy {
        priority_ordinal: u32,
    },
    Air {
        selected_priority_ordinal: Option<u32>,
        priority_census_wing_ids: Vec<u32>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemberControllerState {
    Army {
        training_visible: bool,
        stop_at_max_visible: bool,
        border_conflict_visible: bool,
    },
    ArmyGroup {
        training_visible: bool,
        stop_at_max_visible: bool,
        border_conflict_visible: bool,
    },
    Fleet {
        split_off_count: u32,
        split_off_visible: bool,
    },
    ReserveFleetAggregate {
        split_off_count: u32,
        split_off_visible: bool,
    },
    TaskForce {
        spotting_visible: bool,
        training_visible: bool,
        stop_at_max_visible: bool,
    },
    AirGroup,
    AirWing {
        active_visual_frame: u32,
        bucket_key: i32,
    },
}

impl ReplayContentExpectation {
    pub fn from_content_admission(
        admission: &portable_content_manifest::ValidatedContentAdmission,
    ) -> Result<Self, String> {
        use portable_content_manifest::authored_ui::{
            localisation_content_identity_sha256, LOCALISATION_EMPTY_NAMESPACE_CONSUMER,
            LOCALISATION_NAMESPACE_CONSUMER,
        };
        let localisation_rows =
            match admission.used_content_for_consumer(LOCALISATION_NAMESPACE_CONSUMER) {
                Some(rows) => rows,
                None if admission
                    .used_content_for_consumer(LOCALISATION_EMPTY_NAMESPACE_CONSUMER)
                    .is_some() =>
                {
                    &[]
                }
                None => {
                    return Err(
                        "XUI24 admission has neither complete nor authenticated-empty selected-language localisation authority"
                            .into(),
                    )
                }
            };
        let localization_content_sha256 = localisation_content_identity_sha256(
            admission.localisation_language(),
            localisation_rows,
        )?;
        let ordered_layers = admission
            .ordered_layers()
            .iter()
            .map(|layer| {
                let layer_kind = match layer.layer_kind {
                    portable_content_manifest::ContentLayerKind::Base => "base",
                    portable_content_manifest::ContentLayerKind::IntegratedDlc => "integrated_dlc",
                    portable_content_manifest::ContentLayerKind::Dlc => "dlc",
                    portable_content_manifest::ContentLayerKind::Mod => "mod",
                };
                Ok(ContentLayer {
                    ordinal: layer.ordinal,
                    layer_kind: layer_kind.into(),
                    layer_id: layer.layer_id.clone(),
                    content_sha256: portable_content_manifest::content_layer_identity_sha256(
                        layer,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if ordered_layers.is_empty() {
            return Err("XUI24 admission has no ordered content layers".into());
        }
        Ok(Self {
            language: admission.localisation_language().to_string(),
            localization_content_sha256,
            ordered_layers,
        })
    }
}

impl WorldFrame {
    pub fn theatre_selector(&self) -> TheatreSelectorView {
        parse_theatre_selector(self.raw())
    }
}

fn u32_field(object: &Map<String, Value>, key: &str) -> Option<u32> {
    object
        .get(key)?
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    Some(object.get(key)?.as_str()?.to_string())
}

fn i32_field(object: &Map<String, Value>, key: &str) -> Option<i32> {
    object
        .get(key)?
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
}

fn parse_optional_u32(
    object: &Map<String, Value>,
    key: &str,
    what: &str,
) -> Result<Option<u32>, String> {
    match object.get(key) {
        None => Err(format!("{what} has no {key} declaration")),
        Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(
            value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| format!("{what} {key} invalid"))?,
        )),
    }
}

fn bool_field(object: &Map<String, Value>, key: &str, what: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{what} {key} invalid"))
}

fn parse_u32_census(
    object: &Map<String, Value>,
    key: &str,
    what: &str,
) -> Result<Vec<u32>, String> {
    let rows = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{what} {key} missing"))?;
    if rows.len() > MAX_PRIORITY_CENSUS_PER_SECTION {
        return Err(format!("{what} {key} exceeds bound"));
    }
    let mut out = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for row in rows {
        let id = row
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value != 0)
            .ok_or_else(|| format!("{what} {key} entry invalid"))?;
        if !seen.insert(id) {
            return Err(format!("{what} {key} contains a duplicate"));
        }
        out.push(id);
    }
    Ok(out)
}

fn parse_theatre_controller(
    value: &Value,
    expected: ObjectKind,
) -> Result<TheatreControllerState, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "theatre controller is not an object".to_string())?;
    match (expected, object.get("kind").and_then(Value::as_str)) {
        (ObjectKind::LandTheatre, Some("land")) => {
            if object.contains_key("combat_log_owned") {
                return Err(
                    "land controller carries the retired combat_log_owned leaf".to_string()
                );
            }
            Ok(TheatreControllerState::Land {
                priority_ordinal: u32_field(object, "priority_ordinal")
                    .ok_or_else(|| "land controller priority invalid".to_string())?,
                delete_eligible: bool_field(object, "delete_eligible", "land controller")?,
            })
        }
        (ObjectKind::NavyTheatre, Some("navy")) => Ok(TheatreControllerState::Navy {
            priority_ordinal: u32_field(object, "priority_ordinal")
                .ok_or_else(|| "navy controller priority invalid".to_string())?,
        }),
        (ObjectKind::AirTheatre, Some("air")) => {
            let selected_priority_ordinal =
                parse_optional_u32(object, "selected_priority_ordinal", "air controller")?;
            let priority_census_wing_ids =
                parse_u32_census(object, "priority_census_wing_ids", "air controller")?;
            if selected_priority_ordinal.is_some() != !priority_census_wing_ids.is_empty() {
                return Err("air controller priority contradicts its census".into());
            }
            Ok(TheatreControllerState::Air {
                selected_priority_ordinal,
                priority_census_wing_ids,
            })
        }
        _ => Err("theatre controller kind mismatch".into()),
    }
}

fn parse_member_controller(
    value: &Value,
    expected: ObjectKind,
) -> Result<MemberControllerState, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "member controller is not an object".to_string())?;
    let kind = object.get("kind").and_then(Value::as_str);
    let army = |variant: fn(bool, bool, bool) -> MemberControllerState| {
        Ok(variant(
            bool_field(object, "training_visible", "army controller")?,
            bool_field(object, "stop_at_max_visible", "army controller")?,
            bool_field(object, "border_conflict_visible", "army controller")?,
        ))
    };
    match (expected, kind) {
        (ObjectKind::Army, Some("army")) => army(
            |training_visible, stop_at_max_visible, border_conflict_visible| {
                MemberControllerState::Army {
                    training_visible,
                    stop_at_max_visible,
                    border_conflict_visible,
                }
            },
        ),
        (ObjectKind::ArmyGroup, Some("army_group")) => army(
            |training_visible, stop_at_max_visible, border_conflict_visible| {
                MemberControllerState::ArmyGroup {
                    training_visible,
                    stop_at_max_visible,
                    border_conflict_visible,
                }
            },
        ),
        (ObjectKind::Fleet, Some("fleet")) => {
            let split_off_count = u32_field(object, "split_off_count")
                .ok_or_else(|| "fleet split count invalid".to_string())?;
            let split_off_visible = bool_field(object, "split_off_visible", "fleet controller")?;
            if split_off_visible != (split_off_count != 0) {
                return Err("fleet split visibility contradicts its count".into());
            }
            Ok(MemberControllerState::Fleet {
                split_off_count,
                split_off_visible,
            })
        }
        (ObjectKind::ReserveFleetAggregate, Some("reserve_fleet_aggregate")) => {
            let split_off_count = u32_field(object, "split_off_count")
                .ok_or_else(|| "reserve-fleet aggregate split count invalid".to_string())?;
            let split_off_visible = bool_field(
                object,
                "split_off_visible",
                "reserve-fleet aggregate controller",
            )?;
            if split_off_visible != (split_off_count != 0) {
                return Err(
                    "reserve-fleet aggregate split visibility contradicts its count".into(),
                );
            }
            Ok(MemberControllerState::ReserveFleetAggregate {
                split_off_count,
                split_off_visible,
            })
        }
        (ObjectKind::TaskForce, Some("task_force")) => Ok(MemberControllerState::TaskForce {
            spotting_visible: bool_field(object, "spotting_visible", "task-force controller")?,
            training_visible: bool_field(object, "training_visible", "task-force controller")?,
            stop_at_max_visible: bool_field(
                object,
                "stop_at_max_visible",
                "task-force controller",
            )?,
        }),
        (ObjectKind::AirGroup, Some("air_group_negative_no_priority")) => {
            Ok(MemberControllerState::AirGroup)
        }
        (ObjectKind::AirWing, Some("air_wing")) => Ok(MemberControllerState::AirWing {
            active_visual_frame: u32_field(object, "active_visual_frame")
                .ok_or_else(|| "air-wing active frame invalid".to_string())?,
            bucket_key: i32_field(object, "bucket_key")
                .ok_or_else(|| "air-wing bucket key invalid".to_string())?,
        }),
        _ => Err("member controller kind mismatch".into()),
    }
}

fn parse_provenance(value: &Value) -> Option<NameProvenance> {
    let object = value.as_object()?;
    match object.get("source")?.as_str()? {
        "engine_runtime_string" => Some(NameProvenance::EngineRuntimeString),
        "localization_key" => {
            let key = object.get("key")?.as_str()?;
            if key.is_empty() || key.len() > MAX_TOKEN_BYTES {
                return None;
            }
            Some(NameProvenance::LocalizationKey(key.to_string()))
        }
        _ => None,
    }
}

fn parse_member(value: &Value) -> Result<MemberRow, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "member row is not an object".to_string())?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ObjectKind::from_token)
        .ok_or_else(|| "member kind is unknown".to_string())?;
    let parent_kind = object
        .get("parent_kind")
        .and_then(Value::as_str)
        .and_then(ObjectKind::from_token)
        .ok_or_else(|| "member parent kind is unknown".to_string())?;
    let object_id =
        u32_field(object, "object_id").ok_or_else(|| "member object id invalid".to_string())?;
    let parent_id =
        u32_field(object, "parent_id").ok_or_else(|| "member parent id invalid".to_string())?;
    let engine_order = u32_field(object, "engine_order")
        .ok_or_else(|| "member engine order invalid".to_string())?;
    let reserve_navy = object
        .get("reserve_navy")
        .and_then(Value::as_bool)
        .ok_or_else(|| "member reserve flag invalid".to_string())?;
    let display_name = match object.get("display_name") {
        None | Some(Value::Null) => None,
        Some(Value::String(name))
            if !name.is_empty() && name.len() <= MAX_DISPLAY_NAME_BYTES =>
        {
            Some(name.clone())
        }
        Some(_) => return Err("member display name invalid".into()),
    };
    let name_provenance = match object.get("name_provenance") {
        None | Some(Value::Null) => None,
        Some(value) => {
            Some(parse_provenance(value).ok_or_else(|| "member provenance invalid".to_string())?)
        }
    };
    if display_name.is_some() != name_provenance.is_some() {
        return Err("member name and provenance must agree".into());
    }
    if kind.is_theatre() || object_id == 0 || parent_id == 0 {
        return Err("member identity invalid".into());
    }
    if kind == ObjectKind::ReserveFleetAggregate && !reserve_navy {
        return Err("reserve-fleet aggregate must carry the reserve navy flag".into());
    }
    if reserve_navy
        && !matches!(
            kind,
            ObjectKind::ReserveFleetAggregate | ObjectKind::TaskForce
        )
    {
        return Err("reserve navy flag is invalid for this member kind".into());
    }
    let controller = parse_member_controller(
        object
            .get("controller")
            .ok_or_else(|| "member controller missing".to_string())?,
        kind,
    )?;
    match &controller {
        MemberControllerState::Army {
            training_visible,
            stop_at_max_visible,
            border_conflict_visible,
        }
        | MemberControllerState::ArmyGroup {
            training_visible,
            stop_at_max_visible,
            border_conflict_visible,
        } if (*training_visible && *border_conflict_visible)
            || (*stop_at_max_visible && !*training_visible) =>
        {
            return Err("army controller visibility law violated".into())
        }
        MemberControllerState::TaskForce {
            training_visible,
            stop_at_max_visible,
            ..
        } if *stop_at_max_visible && !*training_visible => {
            return Err("task-force controller visibility law violated".into())
        }
        _ => {}
    }
    Ok(MemberRow {
        kind,
        object_id,
        parent_kind,
        parent_id,
        engine_order,
        reserve_navy,
        display_name,
        name_provenance,
        controller,
    })
}

fn parse_theatre(
    value: &Value,
    owner_tag_db_id: u32,
    section_member_ids: &mut BTreeSet<(ObjectKind, u32)>,
) -> Result<TheatreRow, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "theatre row is not an object".to_string())?;
    let kind = object
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ObjectKind::from_token)
        .ok_or_else(|| "theatre kind is unknown".to_string())?;
    if !kind.is_theatre() {
        return Err("theatre row carries a member kind".into());
    }
    let selector_included = bool_field(object, "selector_included", "theatre")?;
    if kind != ObjectKind::LandTheatre && !selector_included {
        return Err("navy/air theatre is excluded from its selector".into());
    }
    let object_id =
        u32_field(object, "object_id").ok_or_else(|| "theatre object id invalid".to_string())?;
    if object_id == 0 {
        return Err("theatre object id is zero".into());
    }
    let row_owner =
        u32_field(object, "owner_tag_db_id").ok_or_else(|| "theatre owner invalid".to_string())?;
    if row_owner != owner_tag_db_id {
        return Err("theatre owner does not match its section".into());
    }
    let engine_order = u32_field(object, "engine_order")
        .ok_or_else(|| "theatre engine order invalid".to_string())?;
    let display_name = object
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= MAX_DISPLAY_NAME_BYTES)
        .ok_or_else(|| "theatre display name invalid".to_string())?
        .to_string();
    let name_provenance = object
        .get("name_provenance")
        .and_then(parse_provenance)
        .ok_or_else(|| "theatre provenance invalid".to_string())?;
    let controller = parse_theatre_controller(
        object
            .get("controller")
            .ok_or_else(|| "theatre controller missing".to_string())?,
        kind,
    )?;
    let mut alert_categories = Vec::new();
    {
        let list = object
            .get("land_alert_categories")
            .ok_or_else(|| "theatre has no land alert category list".to_string())?
            .as_array()
            .ok_or_else(|| "alert categories are not an array".to_string())?;
        if list.len() > MAX_ALERTS_PER_THEATRE {
            return Err("alert category count exceeds bound".into());
        }
        let mut previous: Option<u32> = None;
        for entry in list {
            let category = entry
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| "alert category invalid".to_string())?;
            if kind != ObjectKind::LandTheatre
                || category > MAX_ALERT_CATEGORY
                || previous.is_some_and(|last| category <= last)
            {
                return Err("alert categories must be land-only and ascending".into());
            }
            previous = Some(category);
            alert_categories.push(category);
        }
    }
    let mut expected_member_order: std::collections::BTreeMap<
        (ObjectKind, ObjectKind, u32, bool),
        u32,
    > = Default::default();
    let mut current_land_group = None;
    let mut land_standalones_started = false;
    let mut members = Vec::new();
    {
        let list = object
            .get("members")
            .ok_or_else(|| "theatre has no member list".to_string())?
            .as_array()
            .ok_or_else(|| "members are not an array".to_string())?;
        for entry in list {
            members.push(parse_member(entry)?);
        }
    }
    let group_ids = members
        .iter()
        .filter(|member| member.kind == ObjectKind::ArmyGroup)
        .map(|member| member.object_id)
        .collect::<BTreeSet<_>>();
    let air_group_ids = members
        .iter()
        .filter(|member| member.kind == ObjectKind::AirGroup)
        .map(|member| member.object_id)
        .collect::<BTreeSet<_>>();
    let fleet_ids = members
        .iter()
        .filter(|member| member.kind == ObjectKind::Fleet)
        .map(|member| member.object_id)
        .collect::<BTreeSet<_>>();
    let reserve_fleet_ids = members
        .iter()
        .filter(|member| member.kind == ObjectKind::ReserveFleetAggregate)
        .map(|member| member.object_id)
        .collect::<BTreeSet<_>>();
    for member in &members {
        if !section_member_ids.insert((member.kind, member.object_id)) {
            return Err("duplicate member identity".into());
        }
        let parent_valid = match member.kind {
            ObjectKind::ArmyGroup => {
                kind == ObjectKind::LandTheatre
                    && member.parent_kind == ObjectKind::LandTheatre
                    && member.parent_id == object_id
            }
            ObjectKind::Army => {
                kind == ObjectKind::LandTheatre
                    && ((member.parent_kind == ObjectKind::LandTheatre
                        && member.parent_id == object_id)
                        || (member.parent_kind == ObjectKind::ArmyGroup
                            && group_ids.contains(&member.parent_id)))
            }
            ObjectKind::Fleet => {
                kind == ObjectKind::NavyTheatre
                    && member.parent_kind == ObjectKind::NavyTheatre
                    && member.parent_id == object_id
                    && !member.reserve_navy
            }
            ObjectKind::ReserveFleetAggregate => {
                kind == ObjectKind::NavyTheatre
                    && member.parent_kind == ObjectKind::NavyTheatre
                    && member.parent_id == object_id
                    && member.reserve_navy
            }
            ObjectKind::TaskForce => {
                kind == ObjectKind::NavyTheatre
                    && ((member.parent_kind == ObjectKind::Fleet
                        && fleet_ids.contains(&member.parent_id)
                        && !member.reserve_navy)
                        || (member.parent_kind == ObjectKind::ReserveFleetAggregate
                            && reserve_fleet_ids.contains(&member.parent_id)
                            && member.reserve_navy))
            }
            ObjectKind::AirGroup => {
                kind == ObjectKind::AirTheatre
                    && member.parent_kind == ObjectKind::AirTheatre
                    && member.parent_id == object_id
            }
            ObjectKind::AirWing => {
                kind == ObjectKind::AirTheatre
                    && member.parent_kind == ObjectKind::AirGroup
                    && air_group_ids.contains(&member.parent_id)
            }
            _ => false,
        };
        if !parent_valid {
            return Err("member parent law violated".into());
        }
        if kind == ObjectKind::LandTheatre {
            match (member.kind, member.parent_kind) {
                (ObjectKind::ArmyGroup, ObjectKind::LandTheatre) => {
                    if land_standalones_started {
                        return Err("land group appears after standalone army phase".into());
                    }
                    current_land_group = Some(member.object_id);
                }
                (ObjectKind::Army, ObjectKind::ArmyGroup) => {
                    if land_standalones_started || current_land_group != Some(member.parent_id) {
                        return Err(
                            "land group child is outside its contiguous group bucket".into()
                        );
                    }
                }
                (ObjectKind::Army, ObjectKind::LandTheatre) => {
                    land_standalones_started = true;
                    current_land_group = None;
                }
                _ => return Err("land member phase/kind is invalid".into()),
            }
        }
        let slot = expected_member_order
            .entry((
                member.kind,
                member.parent_kind,
                member.parent_id,
                member.reserve_navy,
            ))
            .or_insert(0_u32);
        if member.engine_order != *slot {
            return Err("member engine order is not contiguous".into());
        }
        *slot += 1;
    }
    let reserve_aggregate_count = members
        .iter()
        .filter(|member| member.kind == ObjectKind::ReserveFleetAggregate)
        .count();
    if (kind == ObjectKind::NavyTheatre && reserve_aggregate_count != 1)
        || (kind != ObjectKind::NavyTheatre && reserve_aggregate_count != 0)
    {
        return Err("navy theatre must carry exactly one reserve-fleet aggregate".into());
    }
    if let TheatreControllerState::Air {
        selected_priority_ordinal,
        priority_census_wing_ids,
    } = &controller
    {
        let available = members
            .iter()
            .filter_map(|member| (member.kind == ObjectKind::AirWing).then_some(member.object_id))
            .collect::<BTreeSet<_>>();
        if priority_census_wing_ids
            .iter()
            .any(|wing_id| !available.contains(wing_id))
            || selected_priority_ordinal.is_some() != !priority_census_wing_ids.is_empty()
        {
            return Err("air theatre priority census is outside its hierarchy".into());
        }
    }
    Ok(TheatreRow {
        kind,
        selector_included,
        object_id,
        owner_tag_db_id,
        engine_order,
        display_name,
        name_provenance,
        controller,
        members,
        alert_categories,
    })
}

fn parse_strategic_region(
    value: &Value,
    owner_tag_db_id: u32,
    expected_order: u32,
) -> Result<StrategicRegionRow, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "strategic region is not an object".to_string())?;
    if u32_field(object, "owner_tag_db_id") != Some(owner_tag_db_id)
        || object.get("parent").and_then(Value::as_str) != Some("selector_root")
    {
        return Err("strategic region owner or parent mismatch".into());
    }
    let engine_region_index = i32_field(object, "engine_region_index")
        .ok_or_else(|| "strategic region engine identity invalid".to_string())?;
    let display_name = object
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= MAX_DISPLAY_NAME_BYTES)
        .ok_or_else(|| "strategic region display name invalid".to_string())?
        .to_string();
    let engine_order = u32_field(object, "engine_order")
        .filter(|order| *order == expected_order)
        .ok_or_else(|| "strategic region order is not contiguous".to_string())?;
    let pinned_frame = u32_field(object, "pinned_frame")
        .filter(|frame| matches!(*frame, 1 | 2))
        .ok_or_else(|| "strategic pinned frame invalid".to_string())?;
    let background_frame = u32_field(object, "background_frame")
        .filter(|frame| matches!(*frame, 2 | 3))
        .ok_or_else(|| "strategic background frame invalid".to_string())?;

    let access = object
        .get("naval_access")
        .and_then(Value::as_object)
        .ok_or_else(|| "strategic naval access missing".to_string())?;
    let access_slot = i32_field(access, "access_slot")
        .ok_or_else(|| "strategic naval access slot invalid".to_string())?;
    let applicable = bool_field(access, "applicable", "strategic naval access")?;
    let raw_result = match access.get("raw_result") {
        Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_i64()
                .and_then(|raw| i8::try_from(raw).ok())
                .ok_or_else(|| "strategic naval raw result invalid".to_string())?,
        ),
        None => return Err("strategic naval raw result missing".into()),
    };
    let visible_frame = parse_optional_u32(access, "visible_frame", "strategic naval access")?;
    if !applicable && (raw_result.is_some() || visible_frame.is_some()) {
        return Err("inapplicable strategic naval access carried a result".into());
    }
    if applicable {
        let Some(raw) = raw_result else {
            return Err("applicable strategic naval access omitted its result".into());
        };
        let expected = (raw >= 0).then_some(raw as u32 + 1);
        if visible_frame != expected {
            return Err("strategic naval access frame contradicts its result".into());
        }
    }

    let alert_values = object
        .get("alerts")
        .and_then(Value::as_array)
        .ok_or_else(|| "strategic alerts missing".to_string())?;
    if alert_values.len() > MAX_ALERTS_PER_THEATRE {
        return Err("strategic alert count exceeds bound".into());
    }
    let mut alerts = Vec::with_capacity(alert_values.len());
    let mut seen_alerts = BTreeSet::new();
    for (index, value) in alert_values.iter().enumerate() {
        let row = value
            .as_object()
            .ok_or_else(|| "strategic alert is not an object".to_string())?;
        let alert_id = u32_field(row, "alert_id")
            .filter(|id| *id <= MAX_ALERT_CATEGORY)
            .ok_or_else(|| "strategic alert id invalid".to_string())?;
        if !seen_alerts.insert(alert_id) {
            return Err("strategic alert id duplicated".into());
        }
        let engine_order = u32_field(row, "engine_order")
            .filter(|order| *order == index as u32)
            .ok_or_else(|| "strategic alert order is not contiguous".to_string())?;
        let atlas_frame = u32_field(row, "atlas_frame")
            .ok_or_else(|| "strategic alert atlas frame invalid".to_string())?;
        let expected_atlas = match alert_id {
            2 => 13,
            3 | 5 => 3,
            4 | 6 => 4,
            7 => 5,
            22 => 8,
            43 => 9,
            45 => 10,
            46 => 14,
            60 => 11,
            other => other + 1,
        };
        if atlas_frame != expected_atlas {
            return Err("strategic alert atlas law mismatch".into());
        }
        let glow = match row.get("glow").and_then(Value::as_str) {
            Some("green") => StrategicAlertGlow::Green,
            Some("yellow") => StrategicAlertGlow::Yellow,
            Some("red") => StrategicAlertGlow::Red,
            _ => return Err("strategic alert glow invalid".into()),
        };
        alerts.push(StrategicAlertRow {
            alert_id,
            engine_order,
            atlas_frame,
            glow,
        });
    }
    Ok(StrategicRegionRow {
        owner_tag_db_id,
        engine_region_index,
        display_name,
        engine_order,
        alerts,
        naval_access: StrategicNavalAccess {
            access_slot,
            applicable,
            raw_result,
            visible_frame,
        },
        pinned_frame,
        background_frame,
    })
}

fn checked_section_member_total(current: usize, additional: usize) -> Result<usize, String> {
    current
        .checked_add(additional)
        .filter(|count| *count <= MAX_MEMBERS_PER_SECTION)
        .ok_or_else(|| "section member count exceeds bound".to_string())
}

fn parse_section(value: &Value) -> Result<CountrySectionView, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "section is not an object".to_string())?;
    let owner_tag_db_id =
        u32_field(object, "owner_tag_db_id").ok_or_else(|| "section owner invalid".to_string())?;
    if owner_tag_db_id == 0 {
        return Err("section owner is zero".into());
    }
    match object.get("admission").and_then(Value::as_str) {
        Some("unavailable") => {
            let reason = string_field(object, "reason")
                .filter(|reason| !reason.is_empty() && reason.len() <= MAX_DISPLAY_NAME_BYTES)
                .ok_or_else(|| "unavailable section carries no reason".to_string())?;
            if object.contains_key("theatres")
                || object.contains_key("free_air_wings")
                || object.contains_key("free_air_priority")
                || object.contains_key("strategic_regions")
            {
                return Err("unavailable section carried rows".into());
            }
            Ok(CountrySectionView::unavailable(
                owner_tag_db_id,
                Some(reason),
            ))
        }
        Some("admitted") => {
            let theatres_value = object
                .get("theatres")
                .ok_or_else(|| "admitted section has no theatre list".to_string())?
                .as_array()
                .ok_or_else(|| "theatres are not an array".to_string())?;
            if theatres_value.len() > MAX_THEATRES_PER_SECTION {
                return Err("theatre count exceeds bound".into());
            }
            let mut theatres = Vec::with_capacity(theatres_value.len());
            let mut section_member_ids = BTreeSet::new();
            let mut section_member_count = 0usize;
            for entry in theatres_value {
                let theatre = parse_theatre(
                    entry,
                    owner_tag_db_id,
                    &mut section_member_ids,
                )?;
                section_member_count =
                    checked_section_member_total(section_member_count, theatre.members.len())?;
                theatres.push(theatre);
            }
            let mut expected: std::collections::BTreeMap<ObjectKind, u32> = Default::default();
            let mut seen = BTreeSet::new();
            for theatre in &theatres {
                if !seen.insert((theatre.kind, theatre.object_id)) {
                    return Err("duplicate theatre identity".into());
                }
                let slot = expected.entry(theatre.kind).or_insert(0);
                if theatre.engine_order != *slot {
                    return Err("theatre engine order is not contiguous".into());
                }
                *slot += 1;
            }
            let mut free_air_wings = Vec::new();
            {
                let list = object
                    .get("free_air_wings")
                    .ok_or_else(|| "admitted section has no free-wing list".to_string())?
                    .as_array()
                    .ok_or_else(|| "free air wings are not an array".to_string())?;
                if list.len() > MAX_FREE_WINGS_PER_SECTION {
                    return Err("free air wing count exceeds bound".into());
                }
                let mut seen_wings = BTreeSet::new();
                for (index, entry) in list.iter().enumerate() {
                    let entry = entry
                        .as_object()
                        .ok_or_else(|| "free wing is not an object".to_string())?;
                    let wing_id = u32_field(entry, "wing_id")
                        .filter(|id| *id != 0)
                        .ok_or_else(|| "free wing id invalid".to_string())?;
                    let engine_order = u32_field(entry, "engine_order")
                        .ok_or_else(|| "free wing order invalid".to_string())?;
                    if engine_order != index as u32 || !seen_wings.insert(wing_id) {
                        return Err("free wing order is not contiguous or is duplicated".into());
                    }
                    if section_member_ids.contains(&(ObjectKind::AirWing, wing_id)) {
                        return Err("free wing is also carried as a grouped member".into());
                    }
                    free_air_wings.push(FreeAirWingRow {
                        wing_id,
                        engine_order,
                        active_visual_frame: u32_field(entry, "active_visual_frame")
                            .ok_or_else(|| "free wing active frame invalid".to_string())?,
                        bucket_key: i32_field(entry, "bucket_key")
                            .ok_or_else(|| "free wing bucket key invalid".to_string())?,
                    });
                }
            }
            let free_air_priority = object
                .get("free_air_priority")
                .and_then(Value::as_object)
                .ok_or_else(|| "admitted section has no free-air priority".to_string())?;
            let free_air_priority_ordinal = parse_optional_u32(
                free_air_priority,
                "selected_priority_ordinal",
                "free-air priority",
            )?;
            let free_air_priority_census_wing_ids = parse_u32_census(
                free_air_priority,
                "priority_census_wing_ids",
                "free-air priority",
            )?;
            let free_ids = free_air_wings
                .iter()
                .map(|row| row.wing_id)
                .collect::<BTreeSet<_>>();
            let priority_ids = free_air_priority_census_wing_ids
                .iter()
                .copied()
                .collect::<BTreeSet<_>>();
            if !priority_ids.is_subset(&free_ids)
                || free_air_priority_ordinal.is_some() != !priority_ids.is_empty()
            {
                return Err("free-air priority contradicts the free-wing census".into());
            }
            let strategic_values = object
                .get("strategic_regions")
                .and_then(Value::as_array)
                .ok_or_else(|| "admitted section has no strategic regions".to_string())?;
            if strategic_values.len() > MAX_STRATEGIC_REGIONS_PER_SECTION {
                return Err("strategic region count exceeds bound".into());
            }
            let mut strategic_regions = Vec::with_capacity(strategic_values.len());
            let mut strategic_ids = BTreeSet::new();
            for (index, entry) in strategic_values.iter().enumerate() {
                let row = parse_strategic_region(entry, owner_tag_db_id, index as u32)?;
                if !strategic_ids.insert(row.engine_region_index) {
                    return Err("strategic region identity duplicated".into());
                }
                strategic_regions.push(row);
            }
            Ok(CountrySectionView {
                status: ViewStatus::Present,
                owner_tag_db_id,
                theatres,
                free_air_wings,
                free_air_priority_ordinal,
                free_air_priority_census_wing_ids,
                strategic_regions,
                rejection: None,
            })
        }
        _ => Err("section admission is unknown".into()),
    }
}

fn parse_content_identity(value: &Value) -> Result<ContentIdentity, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "content identity is not an object".to_string())?;
    let language = string_field(object, "language")
        .filter(|token| !token.is_empty() && token.len() <= MAX_TOKEN_BYTES)
        .ok_or_else(|| "content identity language invalid".to_string())?;
    let localization_content_sha256 = string_field(object, "localization_content_sha256")
        .filter(|hash| hash.len() == 64)
        .ok_or_else(|| "localization content hash invalid".to_string())?;
    let layers = object
        .get("ordered_layers")
        .and_then(Value::as_array)
        .ok_or_else(|| "ordered layers missing".to_string())?;
    if layers.is_empty() || layers.len() > MAX_CONTENT_LAYERS {
        return Err("content identity layer count invalid".into());
    }
    let mut ordered_layers = Vec::with_capacity(layers.len());
    let mut seen_layers = BTreeSet::new();
    for (index, entry) in layers.iter().enumerate() {
        let entry = entry
            .as_object()
            .ok_or_else(|| "content layer is not an object".to_string())?;
        let ordinal = u32_field(entry, "ordinal")
            .ok_or_else(|| "content layer ordinal invalid".to_string())?;
        if ordinal != index as u32 {
            return Err("content layer ordinal is not contiguous".into());
        }
        let layer_kind = string_field(entry, "layer_kind")
            .filter(|kind| matches!(kind.as_str(), "base" | "integrated_dlc" | "dlc" | "mod"))
            .ok_or_else(|| "content layer kind invalid".to_string())?;
        let layer_id = string_field(entry, "layer_id")
            .filter(|id| !id.is_empty() && id.len() <= MAX_TOKEN_BYTES)
            .ok_or_else(|| "content layer id invalid".to_string())?;
        let content_sha256 = string_field(entry, "content_sha256")
            .filter(|hash| hash.len() == 64)
            .ok_or_else(|| "content layer hash invalid".to_string())?;
        if !seen_layers.insert((layer_kind.clone(), layer_id.clone())) {
            return Err("content layer identity duplicated".into());
        }
        ordered_layers.push(ContentLayer {
            ordinal,
            layer_kind,
            layer_id,
            content_sha256,
        });
    }
    Ok(ContentIdentity {
        language,
        localization_content_sha256,
        ordered_layers,
    })
}

fn parse_theatre_selector(raw: &Map<String, Value>) -> TheatreSelectorView {
    let Some(domain) = raw.get(THEATRE_SELECTOR_FIELD) else {
        return TheatreSelectorView::absent();
    };
    if domain.is_null() {
        return TheatreSelectorView::unavailable(None);
    }
    let Some(domain) = domain.as_object() else {
        return TheatreSelectorView::unavailable(Some(
            "theatre selector payload is not an object".into(),
        ));
    };
    if domain.get("authority").and_then(Value::as_str) != Some(THEATRE_SELECTOR_AUTHORITY) {
        return TheatreSelectorView::unavailable(Some(
            "theatre selector authority mismatch".into(),
        ));
    }
    if domain.get("schema_version").and_then(Value::as_u64) != Some(THEATRE_SELECTOR_SCHEMA_VERSION)
    {
        return TheatreSelectorView::unavailable(Some(
            "theatre selector schema version mismatch".into(),
        ));
    }
    if domain.get("executable_sha256").and_then(Value::as_str) != Some(EXACT_A729_EXECUTABLE_SHA256)
    {
        return TheatreSelectorView::unavailable(Some("executable identity mismatch".into()));
    }
    if domain.contains_key("producer_manifest_sha256") {
        return TheatreSelectorView::unavailable(Some(
            "legacy producer manifest-only evidence key is not valid in schema v6".into(),
        ));
    }
    let producer_evidence = domain
        .get("producer_evidence_sha256")
        .and_then(Value::as_array);
    let evidence_matches = producer_evidence.is_some_and(|values| {
        values.len() == REQUIRED_PRODUCER_EVIDENCE_SHA256.len()
            && values
                .iter()
                .zip(REQUIRED_PRODUCER_EVIDENCE_SHA256)
                .all(|(value, expected)| value.as_str() == Some(*expected))
    });
    if !evidence_matches {
        return TheatreSelectorView::unavailable(Some(
            "producer evidence identity mismatch".into(),
        ));
    }
    let content_identity = match domain.get("content_identity").map(parse_content_identity) {
        Some(Ok(identity)) => identity,
        Some(Err(error)) => return TheatreSelectorView::unavailable(Some(error)),
        None => {
            return TheatreSelectorView::unavailable(Some("content identity missing".into()));
        }
    };
    let Some(frame_hour) = domain.get("frame_hour").and_then(Value::as_u64) else {
        return TheatreSelectorView::unavailable(Some("frame hour missing".into()));
    };
    let source_sequence = match domain.get("source_sequence").and_then(Value::as_u64) {
        Some(sequence) if sequence != 0 => sequence,
        _ => {
            return TheatreSelectorView::unavailable(Some("source sequence missing or zero".into()))
        }
    };
    let Some(producer_complete) = domain.get("producer_complete").and_then(Value::as_bool) else {
        return TheatreSelectorView::unavailable(Some("producer-complete flag missing".into()));
    };

    let Some(census_value) = domain.get("census").and_then(Value::as_array) else {
        return TheatreSelectorView::unavailable(Some("census missing".into()));
    };
    if census_value.len() > MAX_COUNTRIES {
        return TheatreSelectorView::unavailable(Some("census exceeds bound".into()));
    }
    let mut census = Vec::with_capacity(census_value.len());
    let mut previous: Option<u32> = None;
    for entry in census_value {
        let Some(owner) = entry.as_u64().and_then(|v| u32::try_from(v).ok()) else {
            return TheatreSelectorView::unavailable(Some("census entry invalid".into()));
        };
        if owner == 0 || previous.is_some_and(|last| owner <= last) {
            return TheatreSelectorView::unavailable(Some(
                "census is not strictly ascending nonzero".into(),
            ));
        }
        previous = Some(owner);
        census.push(owner);
    }

    let Some(sections_value) = domain.get("sections").and_then(Value::as_array) else {
        return TheatreSelectorView::unavailable(Some("sections missing".into()));
    };
    if sections_value.len() != census.len() {
        return TheatreSelectorView::unavailable(Some(
            "section count does not match the census".into(),
        ));
    }
    let mut sections = Vec::with_capacity(sections_value.len());
    for (entry, owner) in sections_value.iter().zip(census.iter()) {
        match parse_section(entry) {
            Ok(section) if section.owner_tag_db_id == *owner => sections.push(section),
            Ok(_) => {
                return TheatreSelectorView::unavailable(Some(
                    "section order does not follow the census".into(),
                ))
            }
            Err(error) => return TheatreSelectorView::unavailable(Some(error)),
        }
    }
    let all_admitted = sections
        .iter()
        .all(|section| section.status == ViewStatus::Present);
    if producer_complete != all_admitted {
        return TheatreSelectorView::unavailable(Some(
            "producer-complete flag contradicts its sections".into(),
        ));
    }

    let Some(declared) = domain
        .get("negative_controller_dispositions")
        .and_then(Value::as_array)
    else {
        return TheatreSelectorView::unavailable(Some(
            "negative controller disposition declaration missing".into(),
        ));
    };
    let mut negative_controller_dispositions = Vec::with_capacity(declared.len());
    let mut declared_set = BTreeSet::new();
    for entry in declared {
        let Some(field) = entry.as_str().filter(|field| !field.is_empty()) else {
            return TheatreSelectorView::unavailable(Some(
                "negative controller disposition entry invalid".into(),
            ));
        };
        if !declared_set.insert(field) {
            return TheatreSelectorView::unavailable(Some(
                "negative controller disposition declared twice".into(),
            ));
        }
        negative_controller_dispositions.push(field.to_string());
    }
    let required = REQUIRED_NEGATIVE_CONTROLLER_DISPOSITIONS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if declared_set != required {
        return TheatreSelectorView::unavailable(Some(
            "negative controller dispositions do not exactly match exact-a729 authority".into(),
        ));
    }

    TheatreSelectorView {
        status: ViewStatus::Present,
        authority: Some(THEATRE_SELECTOR_AUTHORITY.to_string()),
        frame_hour: Some(frame_hour),
        source_sequence: Some(source_sequence),
        producer_complete,
        content_identity: Some(content_identity),
        census,
        sections,
        negative_controller_dispositions,
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests;
