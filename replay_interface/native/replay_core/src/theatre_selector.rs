//! Typed replay view for the exact-a729 all-country theatre selector
//! (compact domain 98, `THS2`).
//!
//! # Why the view is shaped around countries, not theatres
//!
//! The replay observer selects the country; the panel then renders that
//! country's land/navy/air hierarchy. The predecessor carrier walked only the
//! recorder's own country, so selecting anyone else left the panel with no
//! authority at all. This view therefore answers exactly one question per
//! lookup: *for this `owner_tag_db_id`, what did the producer measure?* -- and
//! it distinguishes three answers that must never be conflated:
//!
//! * **Present, non-empty** -- the country's measured hierarchy.
//! * **Present, empty** -- the country genuinely has no theatres. A real
//!   observation, and the correct thing to render as an empty selector.
//! * **`DomainUnavailable`** -- the producer refused this country's section, or
//!   replay refused the frame. The panel must show a refusal, not an empty
//!   list.
//!
//! # Replay owns content admission
//!
//! The extractor owns the pointer-free hierarchy and the engine-materialized
//! strings. Replay owns effective-VFS resolution: it resolves its own winning
//! `interface/theatreselector.gui` and localization content and compares the
//! ordered mod/DLC layers, language, and hashes against the producer-observed
//! identity through [`TheatreSelectorView::authorize`]. A mismatch refuses.
//! Replay never substitutes its own content for a mismatched producer identity,
//! and the extractor never chooses replay geometry.

use crate::{ViewStatus, WorldFrame};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

pub const THEATRE_SELECTOR_AUTHORITY: &str = "exact_a729_all_country_theatre_selector";
pub const THEATRE_SELECTOR_SCHEMA_VERSION: u64 = 2;
pub const THEATRE_SELECTOR_FIELD: &str = "_r2_theatre_selector_live";
pub const EXACT_A729_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

/// Controller fields this consumer knows the producer cannot measure.
///
/// Replay keeps its own copy deliberately: `replay_core` has no dependency on
/// the extractor lane crate, because the `.battleplan` file is the only
/// interface between the two installs. A producer that declares *fewer* holes
/// than this list would be claiming to have measured something replay knows is
/// unmeasured, so the admission below requires the declaration to cover every
/// entry here.
pub const REQUIRED_UNCAPTURED_CONTROLLER_FIELDS: &[&str] = &[
    "theater_group_settings_view.current_priority_ordinal",
    "theater_group_settings_view.delete_eligibility",
    "orders_group_item.army_training_state",
    "orders_group_item.exercise_stop_at_max_state",
    "orders_group_item.border_conflict_state",
    "navy_theater_fleet_item.live_spotting_state",
    "navy_theater_fleet_item.exercise_stop_at_max_state",
    "theater_group_item.supply_priority_ordinal",
    "theater_group_item.combat_log_ownership_and_eligibility",
    "navy_theater_group_item.current_priority_ordinal",
    "navy_theater_group_item.combat_log_ownership_and_eligibility",
    "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
    "new_theater_group_item.current_eligibility",
    "strategic_region_alert_entry.category_to_atlas_frame_law",
    "strategic_region_alert_entry.runtime_glow_state",
    "strategic_region_item.region_and_parent_identity",
    "strategic_region_item.engine_order",
    "strategic_region_item.alert_identities_and_order",
    "strategic_region_item.naval_access_row_input",
    "fleet_row_item_box.split_off_state_and_count",
    "reserve_fleet_row_item_box.split_off_state_and_count",
    "air_theatre_wing_item.active_visual_frame",
    "air_theatre_item.current_priority_ordinal",
    "air_theatre_group_item.current_priority_ordinal",
    "air_theatre_free_wings_item.current_priority_ordinal",
];

const MAX_COUNTRIES: usize = 4_096;
const MAX_THEATRES_PER_SECTION: usize = 16_384;

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

/// Where a carried display string came from. A runtime string is *not* a
/// localization key and must never be re-resolved as one.
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
    /// `None` means the producer explicitly could not establish it, never a
    /// defaulted zero.
    pub priority_ordinal: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreRow {
    pub kind: ObjectKind,
    pub object_id: u32,
    pub owner_tag_db_id: u32,
    pub engine_order: u32,
    pub display_name: String,
    pub name_provenance: NameProvenance,
    /// `None` means the producer explicitly could not establish it. It is never
    /// a defaulted zero: a consumer that needs a priority must treat `None` as
    /// unavailable and refuse, not sort by an invented value.
    pub priority_ordinal: Option<u32>,
    pub members: Vec<MemberRow>,
    pub alert_categories: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeAirWingRow {
    pub wing_id: u32,
    pub engine_order: u32,
    /// `None` means the producer explicitly could not establish it, never a
    /// defaulted zero.
    pub priority_ordinal: Option<u32>,
}

/// One country's answer. `status` is `Present` for both a measured hierarchy
/// and a measured-empty country; it is `DomainUnavailable` when the producer
/// refused the section, and `rejection` then names why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountrySectionView {
    pub status: ViewStatus,
    pub owner_tag_db_id: u32,
    pub theatres: Vec<TheatreRow>,
    pub free_air_wings: Vec<FreeAirWingRow>,
    pub rejection: Option<String>,
}

impl CountrySectionView {
    fn unavailable(owner_tag_db_id: u32, rejection: Option<String>) -> Self {
        Self {
            status: ViewStatus::DomainUnavailable,
            owner_tag_db_id,
            theatres: Vec::new(),
            free_air_wings: Vec::new(),
            rejection,
        }
    }

    /// True only for an admitted section the producer measured as empty.
    /// Refused sections answer `false`, because they measured nothing at all.
    pub fn is_measured_empty(&self) -> bool {
        self.status == ViewStatus::Present
            && self.theatres.is_empty()
            && self.free_air_wings.is_empty()
    }

    pub fn theatres_of_kind(&self, kind: ObjectKind) -> impl Iterator<Item = &TheatreRow> {
        self.theatres.iter().filter(move |row| row.kind == kind)
    }
}

/// One ordered content layer the producer observed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLayer {
    pub ordinal: u32,
    /// `base`, `integrated_dlc`, `dlc`, or `mod`.
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

/// What replay independently resolved from its own winning effective VFS.
/// Built by the replay side, never by the extractor.
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
    /// True only when every census country produced an admitted section.
    pub producer_complete: bool,
    pub content_identity: Option<ContentIdentity>,
    /// Complete selectable-country census, strictly ascending.
    pub census: Vec<u32>,
    sections: Vec<CountrySectionView>,
    /// Controller fields the producer declared it does not carry. Surfaced so a
    /// panel can refuse a feature that needs one instead of rendering a
    /// plausible-looking default.
    pub uncaptured_controller_fields: Vec<String>,
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
            uncaptured_controller_fields: Vec::new(),
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

    /// The section for one selectable country.
    ///
    /// A country outside the census returns `DomainUnavailable` -- never
    /// another country's section, and never a first-available fallback. That
    /// refusal is the whole reason this carrier replaced the recorder-POV one.
    pub fn country(&self, owner_tag_db_id: u32) -> CountrySectionView {
        if self.status != ViewStatus::Present {
            return CountrySectionView::unavailable(
                owner_tag_db_id,
                self.diagnostic.clone().or_else(|| Some("frame refused".into())),
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

    /// Replay-side content admission.
    ///
    /// Compares replay's independently resolved effective-VFS identity against
    /// the producer-observed one, and binds the frame to the expected replay
    /// hour and a strictly increasing source sequence. Any mismatch downgrades
    /// the whole view to `DomainUnavailable`: replay must not render a
    /// producer's display strings under a different mod set, language, or
    /// localization content than the producer saw.
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
            return Self::unavailable(Some(
                "ordered mod/DLC content identity mismatch".into(),
            ));
        }
        if self.frame_hour != Some(expected_frame_hour) {
            return Self::unavailable(Some("frame hour mismatch".into()));
        }
        match (self.source_sequence, last_source_sequence) {
            (None, _) | (Some(0), _) => {
                return Self::unavailable(Some("source sequence is missing or zero".into()))
            }
            (Some(current), Some(last)) if current <= last => {
                return Self::unavailable(Some(
                    "source sequence did not strictly increase".into(),
                ))
            }
            _ => {}
        }
        self
    }
}

impl WorldFrame {
    /// All-country theatre-selector hierarchy for this replay hour.
    ///
    /// Structural admission only; call [`TheatreSelectorView::authorize`] with
    /// replay's own resolved content identity before rendering any display
    /// name.
    pub fn theatre_selector(&self) -> TheatreSelectorView {
        parse_theatre_selector(self.raw())
    }
}

fn u32_field(object: &Map<String, Value>, key: &str) -> Option<u32> {
    object.get(key)?.as_u64().and_then(|v| u32::try_from(v).ok())
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    Some(object.get(key)?.as_str()?.to_string())
}

/// A priority ordinal that must be *declared*: present-and-null means the
/// producer measured nothing, and an absent key is a malformed projection.
fn parse_priority(object: &Map<String, Value>, what: &str) -> Result<Option<u32>, String> {
    match object.get("priority_ordinal") {
        None => Err(format!("{what} has no priority ordinal declaration")),
        Some(Value::Null) => Ok(None),
        Some(value) => Ok(Some(
            value
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| format!("{what} priority ordinal invalid"))?,
        )),
    }
}

fn parse_provenance(value: &Value) -> Option<NameProvenance> {
    let object = value.as_object()?;
    match object.get("source")?.as_str()? {
        "engine_runtime_string" => Some(NameProvenance::EngineRuntimeString),
        "localization_key" => {
            let key = object.get("key")?.as_str()?;
            if key.is_empty() {
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
        Some(Value::String(name)) if !name.is_empty() => Some(name.clone()),
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
    if reserve_navy && kind != ObjectKind::Fleet {
        return Err("only fleets carry the reserve navy flag".into());
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
        priority_ordinal: parse_priority(object, "member")?,
    })
}

/// Parse one theatre. `section_member_ids` is the *section-wide* member
/// identity set: scoping it per theatre would admit the same fleet or wing
/// under two different parents.
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
    let object_id =
        u32_field(object, "object_id").ok_or_else(|| "theatre object id invalid".to_string())?;
    if object_id == 0 {
        return Err("theatre object id is zero".into());
    }
    let row_owner = u32_field(object, "owner_tag_db_id")
        .ok_or_else(|| "theatre owner invalid".to_string())?;
    if row_owner != owner_tag_db_id {
        return Err("theatre owner does not match its section".into());
    }
    let engine_order = u32_field(object, "engine_order")
        .ok_or_else(|| "theatre engine order invalid".to_string())?;
    let display_name = object
        .get("display_name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "theatre display name invalid".to_string())?
        .to_string();
    let name_provenance = object
        .get("name_provenance")
        .and_then(parse_provenance)
        .ok_or_else(|| "theatre provenance invalid".to_string())?;
    // A *missing* key and an explicit null are different facts: null is the
    // producer declaring the field unmeasured, while absence means the
    // projection is not the shape this consumer was built against.
    let priority_ordinal = parse_priority(object, "theatre")?;
    let mut alert_categories = Vec::new();
    {
        let list = object
            .get("alert_categories")
            .ok_or_else(|| "theatre has no alert category list".to_string())?
            .as_array()
            .ok_or_else(|| "alert categories are not an array".to_string())?;
        let mut previous: Option<u32> = None;
        for entry in list {
            let category = entry
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| "alert category invalid".to_string())?;
            if kind != ObjectKind::LandTheatre || previous.is_some_and(|last| category <= last) {
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
    let mut members = Vec::new();
    {
        // Absent is not empty. A theatre whose member list simply vanished from
        // the projection would otherwise render as a legitimately empty
        // theatre -- a silent truncation of the hierarchy.
        let list = object
            .get("members")
            .ok_or_else(|| "theatre has no member list".to_string())?
            .as_array()
            .ok_or_else(|| "members are not an array".to_string())?;
        for entry in list {
            members.push(parse_member(entry)?);
        }
    }
    // Parent existence, inside this theatre only. An orphan member would let
    // the panel place a row under a group it never received.
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
        // Contiguous from zero within each parent/kind/reserve bucket. Without
        // this, a dropped member arrives as a plausible-looking gap.
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
    Ok(TheatreRow {
        kind,
        object_id,
        owner_tag_db_id,
        engine_order,
        display_name,
        name_provenance,
        priority_ordinal,
        members,
        alert_categories,
    })
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
                .ok_or_else(|| "unavailable section carries no reason".to_string())?;
            // A refused section must never smuggle rows through.
            if object.contains_key("theatres") || object.contains_key("free_air_wings") {
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
            for entry in theatres_value {
                theatres.push(parse_theatre(
                    entry,
                    owner_tag_db_id,
                    &mut section_member_ids,
                )?);
            }
            // Contiguous engine order per kind, and unique identity per kind.
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
                    // A wing cannot be both ungrouped and inside a group.
                    if section_member_ids.contains(&(ObjectKind::AirWing, wing_id)) {
                        return Err("free wing is also carried as a grouped member".into());
                    }
                    free_air_wings.push(FreeAirWingRow {
                        wing_id,
                        engine_order,
                        priority_ordinal: parse_priority(entry, "free wing")?,
                    });
                }
            }
            Ok(CountrySectionView {
                status: ViewStatus::Present,
                owner_tag_db_id,
                theatres,
                free_air_wings,
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
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "content identity language invalid".to_string())?;
    let localization_content_sha256 = string_field(object, "localization_content_sha256")
        .filter(|hash| hash.len() == 64)
        .ok_or_else(|| "localization content hash invalid".to_string())?;
    let layers = object
        .get("ordered_layers")
        .and_then(Value::as_array)
        .ok_or_else(|| "ordered layers missing".to_string())?;
    let mut ordered_layers = Vec::with_capacity(layers.len());
    for (index, entry) in layers.iter().enumerate() {
        let entry = entry
            .as_object()
            .ok_or_else(|| "content layer is not an object".to_string())?;
        let ordinal =
            u32_field(entry, "ordinal").ok_or_else(|| "content layer ordinal invalid".to_string())?;
        if ordinal != index as u32 {
            return Err("content layer ordinal is not contiguous".into());
        }
        let layer_kind = string_field(entry, "layer_kind")
            .filter(|kind| {
                matches!(kind.as_str(), "base" | "integrated_dlc" | "dlc" | "mod")
            })
            .ok_or_else(|| "content layer kind invalid".to_string())?;
        let layer_id = string_field(entry, "layer_id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "content layer id invalid".to_string())?;
        let content_sha256 = string_field(entry, "content_sha256")
            .filter(|hash| hash.len() == 64)
            .ok_or_else(|| "content layer hash invalid".to_string())?;
        ordered_layers.push(ContentLayer {
            ordinal,
            layer_kind,
            layer_id,
            content_sha256,
        });
    }
    if ordered_layers.is_empty() {
        return Err("content identity carries no layers".into());
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
        return TheatreSelectorView::unavailable(Some("theatre selector authority mismatch".into()));
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
            return TheatreSelectorView::unavailable(Some(
                "source sequence missing or zero".into(),
            ))
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

    // A missing or malformed declaration must not silently become "nothing is
    // unresolved" -- that is precisely the reading that would let a panel
    // render a default for a field nobody measured.
    let Some(declared) = domain
        .get("uncaptured_controller_fields")
        .and_then(Value::as_array)
    else {
        return TheatreSelectorView::unavailable(Some(
            "uncaptured controller field declaration missing".into(),
        ));
    };
    let mut uncaptured_controller_fields = Vec::with_capacity(declared.len());
    let mut declared_set = BTreeSet::new();
    for entry in declared {
        let Some(field) = entry.as_str().filter(|field| !field.is_empty()) else {
            return TheatreSelectorView::unavailable(Some(
                "uncaptured controller field entry invalid".into(),
            ));
        };
        if !declared_set.insert(field) {
            return TheatreSelectorView::unavailable(Some(
                "uncaptured controller field declared twice".into(),
            ));
        }
        uncaptured_controller_fields.push(field.to_string());
    }
    // A well-formed list is not enough: it must actually cover every field this
    // consumer knows is unmeasured. A truncated declaration would let a panel
    // treat an unmeasured field as measured.
    if let Some(missing) = REQUIRED_UNCAPTURED_CONTROLLER_FIELDS
        .iter()
        .find(|field| !declared_set.contains(*field))
    {
        return TheatreSelectorView::unavailable(Some(format!(
            "uncaptured controller field declaration omits {missing}"
        )));
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
        uncaptured_controller_fields,
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests;
