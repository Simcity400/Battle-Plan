use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 98;
pub const DOMAIN_NAME: &str = "theatre_selector_live";
pub const MAGIC: u32 = u32::from_le_bytes(*b"THS2");
pub const SCHEMA_VERSION: u32 = 6;

pub const MAX_COUNTRIES: usize = i32::MAX as usize;

const MIN_CENSUS_ENTRY_LEN: usize = 4;
const MIN_SECTION_ENTRY_LEN: usize = 8;
pub const MAX_THEATRES_PER_SECTION: usize = 16_384;
pub const MAX_MEMBERS_PER_SECTION: usize = 200_000;
pub const MAX_ALERTS_PER_THEATRE: usize = 0x52;
pub const MAX_FREE_WINGS_PER_SECTION: usize = 20_000;
pub const MAX_DISPLAY_NAME_BYTES: usize = 4_096;
pub const MAX_CONTENT_LAYERS: usize = 4_096;
pub const MAX_TOKEN_BYTES: usize = 1_024;
pub const MAX_ALERT_CATEGORY: u32 = 0x51;

pub const EXECUTABLE_SHA256: [u8; 32] =
    hex32(b"870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad");
pub const SELECTOR_MATERIALIZER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"7b7a98857d4f1b772a78dbf5d38842029f9dbf3456caf081571b52219d2399bf");
pub const SELECTOR_SOURCE_LAYOUT_MANIFEST_SHA256: [u8; 32] =
    hex32(b"8aa941bb10f7613af820251eaa4a33cce67ed3f1a72205a05e1b7b05731544bb");
pub const RESERVE_NAVY_FILTER_MANIFEST_SHA256: [u8; 32] =
    hex32(b"35c8a1d66ef64dafd88d5ddfb817cd071e711996666b70a655950c0be0adeb18");
pub const AIR_AND_NAVY_LAW_MANIFEST_SHA256: [u8; 32] =
    hex32(b"eee5a6c973edc425a709864071f940ebb002b4f741d138f09864dab4dc017ed6");

/// Owner ruling on lane RE's DOSSIER section 2.4 (option A, 2026-08-23): the
/// land combat-log leaf is retired from the all-country carrier entirely --
/// never carried, never measured; the replay viewer derives the icon from
/// authoritative state (`session.player_country_set`). These four strings are
/// the exact negative-controller-disposition declaration replay's strict
/// schema-6 decoder requires (`REQUIRED_NEGATIVE_CONTROLLER_DISPOSITIONS`);
/// keep them equal by value. The land row names the real contract template:
/// the land theatre row's container IS `theater_group_item`
/// (`authenticated_semantic_provider.rs`, claim identity `0x01e5_17d0`);
/// there is no `land_theatre_item` template in the engine.
pub const NEGATIVE_CONTROLLER_DISPOSITIONS: &[&str] = &[
    "theater_group_item.combat_log_ownership_and_eligibility",
    "navy_theater_group_item.combat_log_ownership_and_eligibility",
    "reserve_navy_theater_group_item.combat_log_ownership_and_eligibility",
    "air_theatre_group_item.current_priority_ordinal",
];

/// Controller fields the producer neither measures nor declares: each names
/// `template.field` on the real contract template, per GS-F5. The land
/// combat-log row is NOT here anymore -- it is negatively disposed above.
pub const UNCAPTURED_CONTROLLER_FIELDS: &[&str] = &[
    "navy_theater_fleet_item.live_spotting_state",
    "navy_theater_fleet_item.exercise_stop_at_max_state",
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

const AUTHORITY_UNAVAILABLE: u32 = 0;
const AUTHORITY_EXACT_A729: u32 = 1;

const fn hex32(input: &[u8; 64]) -> [u8; 32] {
    const fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'a'..=b'f' => value - b'a' + 10,
            b'A'..=b'F' => value - b'A' + 10,
            _ => 0xff,
        }
    }
    let mut out = [0; 32];
    let mut index = 0;
    while index < 32 {
        out[index] = (nibble(input[index * 2]) << 4) | nibble(input[index * 2 + 1]);
        index += 1;
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorityState {
    SourceUnavailable,
    ExactA729,
}

compact_contract::wire_coded_codes! {
    no_discriminant AuthorityState: u32,
    code = raw,
    Ok = from_raw(raw) -> Result<Self, CarrierError> { invalid("THS2 authority is unknown") };

    SourceUnavailable = AUTHORITY_UNAVAILABLE,
    ExactA729 = AUTHORITY_EXACT_A729,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ObjectKind {
    LandTheatre = 1,
    NavyTheatre = 2,
    AirTheatre = 3,
    Army = 4,
    ArmyGroup = 5,
    Fleet = 6,
    AirGroup = 7,
    AirWing = 8,
}

compact_contract::wire_coded_codes! {
    ObjectKind: u8,
    code = pub code,
    Ok = from_raw(raw) -> Result<Self, CarrierError> { invalid("THS2 object kind is unknown") };

    LandTheatre = 1,
    NavyTheatre = 2,
    AirTheatre = 3,
    Army = 4,
    ArmyGroup = 5,
    Fleet = 6,
    AirGroup = 7,
    AirWing = 8,
}

impl ObjectKind {
    pub const fn is_theatre(self) -> bool {
        matches!(
            self,
            Self::LandTheatre | Self::NavyTheatre | Self::AirTheatre
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLayer {
    pub ordinal: u32,
    pub layer_kind: ContentLayerKind,
    pub layer_id: String,
    pub content_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum ContentLayerKind {
    Base = 1,
    IntegratedDlc = 2,
    Dlc = 3,
    Mod = 4,
}

compact_contract::wire_coded_codes! {
    ContentLayerKind: u8,
    code = pub code,
    Ok = from_raw(raw) -> Result<Self, CarrierError> { invalid("THS2 content layer kind is unknown") };

    Base = 1,
    IntegratedDlc = 2,
    Dlc = 3,
    Mod = 4,
}

pub mod engine_localisation_provenance;
pub mod engine_modset_provenance;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentIdentity {
    pub language: String,
    pub localization_content_sha256: [u8; 32],
    pub ordered_layers: Vec<ContentLayer>,
    pub engine_roster: EngineContentRoster,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EngineContentRoster {
    pub requested_mods: Vec<String>,
    pub requested_disabled_dlcs: Vec<String>,
    pub discovered: Vec<EngineRosterEntry>,
    pub roster_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineRosterEntry {
    pub kind: ContentLayerKind,
    pub descriptor_entry: String,
    pub name: String,
    pub selected: bool,
    pub mounted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameProvenance {
    EngineRuntimeString,
    LocalizationKey(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameProvenanceTag {
    EngineRuntimeString,
    LocalizationKey,
}

compact_contract::wire_coded_codes! {
    no_discriminant NameProvenanceTag: u8,
    code = code,
    Ok = from_code(tag) -> Result<Self, CarrierError> {
        invalid("THS2 name provenance is unknown")
    };

    EngineRuntimeString = 1,
    LocalizationKey = 2,
}

impl NameProvenance {
    fn tag(&self) -> NameProvenanceTag {
        match self {
            Self::EngineRuntimeString => NameProvenanceTag::EngineRuntimeString,
            Self::LocalizationKey(_) => NameProvenanceTag::LocalizationKey,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PriorityOrdinal {
    Unavailable,
    Measured(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArmyOrdersVisibility {
    pub training_visible: bool,
    pub stop_at_max_visible: bool,
    pub border_conflict_visible: bool,
}

impl ArmyOrdersVisibility {
    fn law_holds(self) -> bool {
        (!self.stop_at_max_visible || self.training_visible)
            && !(self.border_conflict_visible && self.training_visible)
    }
}

impl PriorityOrdinal {
    pub const fn measured(self) -> Option<u32> {
        match self {
            Self::Unavailable => None,
            Self::Measured(value) => Some(value),
        }
    }
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
    pub priority_ordinal: PriorityOrdinal,
    pub orders_visibility: Option<ArmyOrdersVisibility>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreRow {
    pub kind: ObjectKind,
    pub object_id: u32,
    pub owner_tag_db_id: u32,
    pub engine_order: u32,
    pub display_name: String,
    pub name_provenance: NameProvenance,
    pub priority_ordinal: PriorityOrdinal,
    pub delete_eligible: Option<bool>,
    pub members: Vec<MemberRow>,
    pub alert_categories: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeAirWingRow {
    pub wing_id: u32,
    pub engine_order: u32,
    pub priority_ordinal: PriorityOrdinal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionBody {
    pub theatres: Vec<TheatreRow>,
    pub free_air_wings: Vec<FreeAirWingRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SectionRejection {
    SourceUnreadable = 1,
    DuplicateIdentity = 2,
    NonContiguousOrder = 3,
    CrossOwnerRow = 4,
    OrphanRow = 5,
    IdentityAuthenticationFailed = 6,
    IncompleteWalk = 7,
    DisplayNameUnreadable = 8,
    FrameHourMismatch = 9,
    BoundExceeded = 10,
}

compact_contract::wire_coded_codes! {
    SectionRejection: u32,
    code = pub code,
    Ok = from_raw(raw) -> Result<Self, CarrierError> { invalid("THS2 section rejection reason is unknown") };

    SourceUnreadable = 1,
    DuplicateIdentity = 2,
    NonContiguousOrder = 3,
    CrossOwnerRow = 4,
    OrphanRow = 5,
    IdentityAuthenticationFailed = 6,
    IncompleteWalk = 7,
    DisplayNameUnreadable = 8,
    FrameHourMismatch = 9,
    BoundExceeded = 10,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionAdmission {
    Unavailable(SectionRejection),
    Admitted(SectionBody),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountrySection {
    pub owner_tag_db_id: u32,
    pub admission: SectionAdmission,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreSelectorSnapshot {
    pub census: Vec<u32>,
    pub sections: Vec<CountrySection>,
    pub producer_complete: bool,
}

impl TheatreSelectorSnapshot {
    pub fn section(&self, owner_tag_db_id: u32) -> Option<&CountrySection> {
        self.sections
            .iter()
            .find(|section| section.owner_tag_db_id == owner_tag_db_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheatreSelectorFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub frame_hour: u64,
    pub source_sequence: u64,
    pub authority: AuthorityState,
    pub content_identity: Option<ContentIdentity>,
    pub snapshot: Option<TheatreSelectorSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierError {
    Source(String),
    Invalid(String),
    Truncated,
    TrailingBytes,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SourceSequenceGate {
    last: Option<u64>,
}

impl SourceSequenceGate {
    pub const fn new() -> Self {
        Self { last: None }
    }

    pub fn admit(&mut self, source_sequence: u64) -> Result<(), CarrierError> {
        if source_sequence == 0 {
            return invalid("THS2 source sequence is zero");
        }
        if self.last.is_some_and(|last| source_sequence <= last) {
            return invalid("THS2 source sequence did not strictly increase");
        }
        self.last = Some(source_sequence);
        Ok(())
    }
}

pub trait ExactTheatreSelectorSource {
    fn is_exact_a729_theatre_selector_source(&self) -> bool;
    fn read_country_census(&self) -> Result<Vec<u32>, CarrierError>;
    fn read_content_identity(&self) -> Result<ContentIdentity, CarrierError>;
    fn read_country_section(&self, owner_tag_db_id: u32) -> Result<SectionBody, SectionRejection>;
}

pub fn collect_and_encode<S: ExactTheatreSelectorSource>(
    source: &S,
    frame_hour: u64,
    source_sequence: u64,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, CarrierError> {
    if !source.is_exact_a729_theatre_selector_source() {
        return Err(CarrierError::Source(
            "source did not authenticate the exact a729 theatre selector roots".into(),
        ));
    }
    let content_identity = source.read_content_identity()?;
    let census = source.read_country_census()?;
    let mut sections = Vec::with_capacity(census.len());
    let mut producer_complete = true;
    for &owner_tag_db_id in &census {
        let admission = match source.read_country_section(owner_tag_db_id) {
            Ok(body) => SectionAdmission::Admitted(body),
            Err(rejection) => {
                producer_complete = false;
                SectionAdmission::Unavailable(rejection)
            }
        };
        sections.push(CountrySection {
            owner_tag_db_id,
            admission,
        });
    }
    encode_payload(&TheatreSelectorFrame {
        elapsed_wall_ms,
        status_flags: 0,
        frame_hour,
        source_sequence,
        authority: AuthorityState::ExactA729,
        content_identity: Some(content_identity),
        snapshot: Some(TheatreSelectorSnapshot {
            census,
            sections,
            producer_complete,
        }),
    })
}

pub fn encode_source_unavailable(
    frame_hour: u64,
    source_sequence: u64,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, CarrierError> {
    encode_payload(&TheatreSelectorFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        source_sequence,
        authority: AuthorityState::SourceUnavailable,
        content_identity: None,
        snapshot: None,
    })
}

fn validate_content_identity(identity: &ContentIdentity) -> Result<(), CarrierError> {
    if identity.language.is_empty()
        || identity.language.len() > MAX_TOKEN_BYTES
        || identity.ordered_layers.is_empty()
        || identity.ordered_layers.len() > MAX_CONTENT_LAYERS
    {
        return invalid("THS2 content identity language/layer count is invalid");
    }
    let mut seen = BTreeSet::new();
    for (index, layer) in identity.ordered_layers.iter().enumerate() {
        if layer.ordinal != index as u32 {
            return invalid("THS2 content layer ordinal is not contiguous");
        }
        if layer.layer_id.is_empty()
            || layer.layer_id.len() > MAX_TOKEN_BYTES
            || !seen.insert((layer.layer_kind, layer.layer_id.as_str()))
        {
            return invalid("THS2 content layer identity is invalid or duplicated");
        }
    }
    validate_engine_roster(&identity.engine_roster)
}

fn validate_engine_roster(roster: &EngineContentRoster) -> Result<(), CarrierError> {
    if roster.requested_mods.len() > MAX_CONTENT_LAYERS
        || roster.requested_disabled_dlcs.len() > MAX_CONTENT_LAYERS
        || roster.discovered.len() > MAX_CONTENT_LAYERS
    {
        return invalid("THS2 engine roster exceeds its transport bound");
    }
    for requested in roster
        .requested_mods
        .iter()
        .chain(roster.requested_disabled_dlcs.iter())
    {
        if requested.is_empty() || requested.len() > MAX_TOKEN_BYTES {
            return invalid("THS2 engine roster requested entry is invalid");
        }
    }
    for entry in &roster.discovered {
        if !matches!(entry.kind, ContentLayerKind::Dlc | ContentLayerKind::Mod) {
            return invalid("THS2 engine roster entry kind is not a DLC or a mod");
        }
        if entry.descriptor_entry.len() > MAX_TOKEN_BYTES
            || entry.name.len() > MAX_DISPLAY_NAME_BYTES
        {
            return invalid("THS2 engine roster entry string exceeds its bound");
        }
    }
    Ok(())
}

fn validate_name(name: &str, provenance: &NameProvenance, what: &str) -> Result<(), CarrierError> {
    if name.is_empty() || name.len() > MAX_DISPLAY_NAME_BYTES {
        return invalid(format!("THS2 {what} display name length is invalid"));
    }
    if let NameProvenance::LocalizationKey(key) = provenance {
        if key.is_empty() || key.len() > MAX_TOKEN_BYTES {
            return invalid(format!("THS2 {what} localization key is invalid"));
        }
    }
    Ok(())
}

fn validate_section_body(owner_tag_db_id: u32, body: &SectionBody) -> Result<(), CarrierError> {
    if body.theatres.len() > MAX_THEATRES_PER_SECTION
        || body.free_air_wings.len() > MAX_FREE_WINGS_PER_SECTION
    {
        return invalid("THS2 section count exceeds bound");
    }
    let mut theatre_ids = BTreeSet::new();
    let mut expected_theatre_order = BTreeMap::new();
    let mut total_members = 0usize;
    let mut member_ids = BTreeSet::new();
    for theatre in &body.theatres {
        if !theatre.kind.is_theatre()
            || theatre.object_id == 0
            || !theatre_ids.insert((theatre.kind, theatre.object_id))
        {
            return invalid("THS2 theatre identity is invalid or duplicated");
        }
        if theatre.owner_tag_db_id != owner_tag_db_id {
            return invalid("THS2 theatre owner does not match its section");
        }
        validate_name(&theatre.display_name, &theatre.name_provenance, "theatre")?;
        if theatre.delete_eligible.is_some() != (theatre.kind == ObjectKind::LandTheatre) {
            return invalid("THS2 delete eligibility is not confined to land theatres");
        }
        let priority_is_implemented = matches!(
            theatre.kind,
            ObjectKind::LandTheatre | ObjectKind::NavyTheatre
        );
        if theatre.priority_ordinal.measured().is_some() != priority_is_implemented {
            return invalid("THS2 theatre priority disposition does not match its kind");
        }
        let expected = expected_theatre_order.entry(theatre.kind).or_insert(0_u32);
        if theatre.engine_order != *expected {
            return invalid("THS2 theatre kind order is not contiguous");
        }
        *expected = expected.saturating_add(1);

        if theatre.alert_categories.len() > MAX_ALERTS_PER_THEATRE {
            return invalid("THS2 alert count exceeds bound");
        }
        let mut previous_alert: Option<u32> = None;
        for &category in &theatre.alert_categories {
            if theatre.kind != ObjectKind::LandTheatre
                || category > MAX_ALERT_CATEGORY
                || previous_alert.is_some_and(|previous| category <= previous)
            {
                return invalid("THS2 land alert category is invalid or not ascending");
            }
            previous_alert = Some(category);
        }

        let mut expected_member_order = BTreeMap::new();
        for member in &theatre.members {
            if member.kind.is_theatre()
                || member.object_id == 0
                || member.parent_id == 0
                || !member_ids.insert((member.kind, member.object_id))
            {
                return invalid("THS2 member identity is invalid or duplicated");
            }
            match (&member.display_name, &member.name_provenance) {
                (Some(name), Some(provenance)) => validate_name(name, provenance, "member")?,
                (None, None) => {}
                _ => return invalid("THS2 member name and provenance must agree"),
            }
            let parent_valid = match member.kind {
                ObjectKind::ArmyGroup => {
                    theatre.kind == ObjectKind::LandTheatre
                        && member.parent_kind == ObjectKind::LandTheatre
                        && member.parent_id == theatre.object_id
                }
                ObjectKind::Army => {
                    theatre.kind == ObjectKind::LandTheatre
                        && ((member.parent_kind == ObjectKind::LandTheatre
                            && member.parent_id == theatre.object_id)
                            || (member.parent_kind == ObjectKind::ArmyGroup
                                && theatre.members.iter().any(|candidate| {
                                    candidate.kind == ObjectKind::ArmyGroup
                                        && candidate.object_id == member.parent_id
                                })))
                }
                ObjectKind::Fleet => {
                    theatre.kind == ObjectKind::NavyTheatre
                        && member.parent_kind == ObjectKind::NavyTheatre
                        && member.parent_id == theatre.object_id
                }
                ObjectKind::AirGroup => {
                    theatre.kind == ObjectKind::AirTheatre
                        && member.parent_kind == ObjectKind::AirTheatre
                        && member.parent_id == theatre.object_id
                }
                ObjectKind::AirWing => {
                    theatre.kind == ObjectKind::AirTheatre
                        && member.parent_kind == ObjectKind::AirGroup
                        && theatre.members.iter().any(|candidate| {
                            candidate.kind == ObjectKind::AirGroup
                                && candidate.object_id == member.parent_id
                        })
                }
                _ => false,
            };
            if !parent_valid || (member.reserve_navy && member.kind != ObjectKind::Fleet) {
                return invalid("THS2 member parent/reserve law is invalid");
            }
            if member.priority_ordinal != PriorityOrdinal::Unavailable {
                return invalid("THS2 member priority has no implemented read law");
            }
            let expects_orders = matches!(member.kind, ObjectKind::Army | ObjectKind::ArmyGroup);
            match member.orders_visibility {
                Some(orders) if expects_orders => {
                    if !orders.law_holds() {
                        return invalid("THS2 orders-group visibility violates its engine law");
                    }
                }
                None if !expects_orders => {}
                _ => return invalid("THS2 orders-group visibility is not confined to army rows"),
            }
            let expected = expected_member_order
                .entry((
                    member.kind,
                    member.parent_kind,
                    member.parent_id,
                    member.reserve_navy,
                ))
                .or_insert(0_u32);
            if member.engine_order != *expected {
                return invalid("THS2 member parent order is not contiguous");
            }
            *expected = expected.saturating_add(1);
        }
        total_members = total_members.saturating_add(theatre.members.len());
    }
    if total_members > MAX_MEMBERS_PER_SECTION {
        return invalid("THS2 section member count exceeds bound");
    }

    let mut free_wings = BTreeSet::new();
    for (index, wing) in body.free_air_wings.iter().enumerate() {
        if wing.priority_ordinal != PriorityOrdinal::Unavailable {
            return invalid("THS2 free-wing priority has no implemented read law");
        }
        if wing.wing_id == 0
            || !free_wings.insert(wing.wing_id)
            || wing.engine_order != index as u32
        {
            return invalid("THS2 free-wing identity/order is invalid or duplicated");
        }
        if member_ids.contains(&(ObjectKind::AirWing, wing.wing_id)) {
            return invalid("THS2 free wing is also carried as a grouped member");
        }
    }
    Ok(())
}

pub fn validate_snapshot(snapshot: &TheatreSelectorSnapshot) -> Result<(), CarrierError> {
    if snapshot.census.len() > MAX_COUNTRIES {
        return invalid("THS2 census exceeds bound");
    }
    let mut previous: Option<u32> = None;
    for &owner in &snapshot.census {
        if owner == 0 || previous.is_some_and(|last| owner <= last) {
            return invalid("THS2 census is not strictly ascending nonzero");
        }
        previous = Some(owner);
    }
    if snapshot.sections.len() != snapshot.census.len() {
        return invalid("THS2 section count does not match the census");
    }
    let mut all_admitted = true;
    for (section, &owner) in snapshot.sections.iter().zip(snapshot.census.iter()) {
        if section.owner_tag_db_id != owner {
            return invalid("THS2 section order does not follow the census");
        }
        match &section.admission {
            SectionAdmission::Unavailable(_) => all_admitted = false,
            SectionAdmission::Admitted(body) => validate_section_body(owner, body)?,
        }
    }
    if snapshot.producer_complete != all_admitted {
        return invalid("THS2 producer-complete flag contradicts its sections");
    }
    Ok(())
}

fn validate_frame(frame: &TheatreSelectorFrame) -> Result<(), CarrierError> {
    if frame.source_sequence == 0 {
        return invalid("THS2 source sequence is zero");
    }
    match frame.authority {
        AuthorityState::SourceUnavailable => {
            if frame.status_flags == 0
                || frame.snapshot.is_some()
                || frame.content_identity.is_some()
            {
                return invalid("THS2 unavailable frame must be dirty and bodyless");
            }
        }
        AuthorityState::ExactA729 => {
            let (Some(identity), Some(snapshot)) = (&frame.content_identity, &frame.snapshot)
            else {
                return invalid("THS2 exact frame must carry identity and sections");
            };
            if frame.status_flags != 0 {
                return invalid("THS2 exact frame must be clean");
            }
            validate_content_identity(identity)?;
            validate_snapshot(snapshot)?;
        }
    }
    Ok(())
}

pub fn encode_payload(frame: &TheatreSelectorFrame) -> Result<Vec<u8>, CarrierError> {
    validate_frame(frame)?;
    let mut out = Vec::new();
    u32v(&mut out, MAGIC);
    u32v(&mut out, SCHEMA_VERSION);
    u32v(&mut out, frame.authority.raw());
    u32v(&mut out, frame.status_flags);
    u64v(&mut out, frame.elapsed_wall_ms);
    u64v(&mut out, frame.frame_hour);
    u64v(&mut out, frame.source_sequence);
    out.extend_from_slice(&EXECUTABLE_SHA256);
    out.extend_from_slice(&SELECTOR_MATERIALIZER_MANIFEST_SHA256);
    out.extend_from_slice(&SELECTOR_SOURCE_LAYOUT_MANIFEST_SHA256);
    out.extend_from_slice(&RESERVE_NAVY_FILTER_MANIFEST_SHA256);
    out.extend_from_slice(&AIR_AND_NAVY_LAW_MANIFEST_SHA256);
    let Some(snapshot) = &frame.snapshot else {
        out.push(0);
        out.extend_from_slice(&[0; 3]);
        return Ok(out);
    };
    let identity = frame
        .content_identity
        .as_ref()
        .expect("validated exact frame carries content identity");
    out.push(1);
    out.push(u8::from(snapshot.producer_complete));
    out.extend_from_slice(&[0; 2]);

    string(&mut out, &identity.language);
    out.extend_from_slice(&identity.localization_content_sha256);
    u32v(&mut out, identity.ordered_layers.len() as u32);
    for layer in &identity.ordered_layers {
        u32v(&mut out, layer.ordinal);
        out.push(layer.layer_kind.code());
        out.extend_from_slice(&[0; 3]);
        string(&mut out, &layer.layer_id);
        out.extend_from_slice(&layer.content_sha256);
    }

    let roster = &identity.engine_roster;
    for list in [&roster.requested_mods, &roster.requested_disabled_dlcs] {
        u32v(&mut out, list.len() as u32);
        for item in list {
            string(&mut out, item);
        }
    }
    u32v(&mut out, roster.discovered.len() as u32);
    for entry in &roster.discovered {
        out.push(entry.kind.code());
        out.push(u8::from(entry.selected));
        out.push(u8::from(entry.mounted));
        out.push(0);
        opt_string(&mut out, &entry.descriptor_entry);
        opt_string(&mut out, &entry.name);
    }
    out.extend_from_slice(&roster.roster_sha256);

    u32v(&mut out, snapshot.census.len() as u32);
    for &owner in &snapshot.census {
        u32v(&mut out, owner);
    }
    u32v(&mut out, snapshot.sections.len() as u32);
    for section in &snapshot.sections {
        u32v(&mut out, section.owner_tag_db_id);
        match &section.admission {
            SectionAdmission::Unavailable(reason) => {
                out.push(0);
                out.extend_from_slice(&[0; 3]);
                u32v(&mut out, reason.code());
            }
            SectionAdmission::Admitted(body) => {
                out.push(1);
                out.extend_from_slice(&[0; 3]);
                u32v(&mut out, body.theatres.len() as u32);
                u32v(&mut out, body.free_air_wings.len() as u32);
                for theatre in &body.theatres {
                    out.push(theatre.kind.code());
                    out.extend_from_slice(&[0; 3]);
                    u32v(&mut out, theatre.object_id);
                    u32v(&mut out, theatre.owner_tag_db_id);
                    u32v(&mut out, theatre.engine_order);
                    string(&mut out, &theatre.display_name);
                    provenance(&mut out, &theatre.name_provenance);
                    priority(&mut out, theatre.priority_ordinal);
                    opt_bool(&mut out, theatre.delete_eligible);
                    u32v(&mut out, theatre.members.len() as u32);
                    u32v(&mut out, theatre.alert_categories.len() as u32);
                    for member in &theatre.members {
                        out.push(member.kind.code());
                        out.push(member.parent_kind.code());
                        out.push(u8::from(member.reserve_navy));
                        out.push(u8::from(member.display_name.is_some()));
                        u32v(&mut out, member.object_id);
                        u32v(&mut out, member.parent_id);
                        u32v(&mut out, member.engine_order);
                        priority(&mut out, member.priority_ordinal);
                        orders_visibility(&mut out, member.orders_visibility);
                        if let Some(name) = &member.display_name {
                            string(&mut out, name);
                            provenance(
                                &mut out,
                                member
                                    .name_provenance
                                    .as_ref()
                                    .expect("validated named member carries provenance"),
                            );
                        }
                    }
                    for category in &theatre.alert_categories {
                        u32v(&mut out, *category);
                    }
                }
                for wing in &body.free_air_wings {
                    u32v(&mut out, wing.wing_id);
                    u32v(&mut out, wing.engine_order);
                    priority(&mut out, wing.priority_ordinal);
                }
            }
        }
    }
    Ok(out)
}

pub fn decode_payload(payload: &[u8]) -> Result<TheatreSelectorFrame, CarrierError> {
    let mut reader = Reader { payload, at: 0 };
    if reader.u32()? != MAGIC || reader.u32()? != SCHEMA_VERSION {
        return invalid("THS2 magic/schema mismatch");
    }
    let authority = AuthorityState::from_raw(reader.u32()?)?;
    let status_flags = reader.u32()?;
    let elapsed_wall_ms = reader.u64()?;
    let frame_hour = reader.u64()?;
    let source_sequence = reader.u64()?;
    for (expected, label) in [
        (EXECUTABLE_SHA256, "executable"),
        (
            SELECTOR_MATERIALIZER_MANIFEST_SHA256,
            "selector materializer",
        ),
        (SELECTOR_SOURCE_LAYOUT_MANIFEST_SHA256, "source layout"),
        (RESERVE_NAVY_FILTER_MANIFEST_SHA256, "reserve navy filter"),
        (AIR_AND_NAVY_LAW_MANIFEST_SHA256, "air/navy law"),
    ] {
        if reader.bytes(32)? != expected {
            return invalid(format!("THS2 {label} authority mismatch"));
        }
    }
    let has_body = reader.bool()?;
    let producer_complete_raw = reader.byte()?;
    if reader.bytes(2)? != [0, 0] {
        return invalid("THS2 availability reserved bytes are nonzero");
    }
    let (content_identity, snapshot) = if has_body {
        let producer_complete = match producer_complete_raw {
            0 => false,
            1 => true,
            _ => return invalid("THS2 producer-complete byte is invalid"),
        };
        let language = reader.string()?;
        let localization_content_sha256: [u8; 32] =
            reader.bytes(32)?.try_into().expect("32 byte hash");
        let layer_count = reader.u32()? as usize;
        if layer_count > MAX_CONTENT_LAYERS {
            return invalid("THS2 content layer count exceeds bound");
        }
        let mut ordered_layers = Vec::with_capacity(layer_count);
        for _ in 0..layer_count {
            let ordinal = reader.u32()?;
            let layer_kind = ContentLayerKind::from_raw(reader.byte()?)?;
            if reader.bytes(3)? != [0, 0, 0] {
                return invalid("THS2 content layer reserved bytes are nonzero");
            }
            let layer_id = reader.string()?;
            let content_sha256: [u8; 32] = reader.bytes(32)?.try_into().expect("32 byte hash");
            ordered_layers.push(ContentLayer {
                ordinal,
                layer_kind,
                layer_id,
                content_sha256,
            });
        }

        let mut requested = Vec::with_capacity(2);
        for _ in 0..2 {
            let count = reader.u32()? as usize;
            if count > MAX_CONTENT_LAYERS {
                return invalid("THS2 engine roster requested count exceeds bound");
            }
            let mut list = Vec::with_capacity(count);
            for _ in 0..count {
                list.push(reader.string()?);
            }
            requested.push(list);
        }
        let requested_disabled_dlcs = requested.pop().expect("two requested lists");
        let requested_mods = requested.pop().expect("two requested lists");

        let discovered_count = reader.u32()? as usize;
        if discovered_count > MAX_CONTENT_LAYERS {
            return invalid("THS2 engine roster entry count exceeds bound");
        }
        let mut discovered = Vec::with_capacity(discovered_count);
        for _ in 0..discovered_count {
            let kind = ContentLayerKind::from_raw(reader.byte()?)?;
            if !matches!(kind, ContentLayerKind::Dlc | ContentLayerKind::Mod) {
                return invalid("THS2 engine roster entry kind is not a DLC or a mod");
            }
            let selected = reader.bool()?;
            let mounted = reader.bool()?;
            if reader.byte()? != 0 {
                return invalid("THS2 engine roster reserved byte is nonzero");
            }
            discovered.push(EngineRosterEntry {
                kind,
                descriptor_entry: reader.opt_string()?,
                name: reader.opt_string()?,
                selected,
                mounted,
            });
        }
        let roster_sha256: [u8; 32] = reader.bytes(32)?.try_into().expect("32 byte hash");
        let engine_roster = EngineContentRoster {
            requested_mods,
            requested_disabled_dlcs,
            discovered,
            roster_sha256,
        };

        let census_count = reader.u32()? as usize;
        if census_count > MAX_COUNTRIES {
            return invalid("THS2 census count exceeds bound");
        }
        if census_count > reader.remaining() / MIN_CENSUS_ENTRY_LEN {
            return invalid("THS2 census count exceeds the payload's capacity");
        }
        let mut census = Vec::with_capacity(census_count);
        for _ in 0..census_count {
            census.push(reader.u32()?);
        }
        let section_count = reader.u32()? as usize;
        if section_count > MAX_COUNTRIES {
            return invalid("THS2 section count exceeds bound");
        }
        if section_count > reader.remaining() / MIN_SECTION_ENTRY_LEN {
            return invalid("THS2 section count exceeds the payload's capacity");
        }
        let mut sections = Vec::with_capacity(section_count);
        for _ in 0..section_count {
            let owner_tag_db_id = reader.u32()?;
            let admitted = reader.bool()?;
            if reader.bytes(3)? != [0, 0, 0] {
                return invalid("THS2 section reserved bytes are nonzero");
            }
            let admission = if admitted {
                let theatre_count = reader.u32()? as usize;
                let free_wing_count = reader.u32()? as usize;
                if theatre_count > MAX_THEATRES_PER_SECTION
                    || free_wing_count > MAX_FREE_WINGS_PER_SECTION
                {
                    return invalid("THS2 section body count exceeds bound");
                }
                let mut theatres = Vec::with_capacity(theatre_count);
                let mut total_members = 0usize;
                for _ in 0..theatre_count {
                    let kind = ObjectKind::from_raw(reader.byte()?)?;
                    if reader.bytes(3)? != [0, 0, 0] {
                        return invalid("THS2 theatre reserved bytes are nonzero");
                    }
                    let object_id = reader.u32()?;
                    let theatre_owner = reader.u32()?;
                    let engine_order = reader.u32()?;
                    let display_name = reader.string()?;
                    let name_provenance = reader.provenance()?;
                    let priority_ordinal = reader.priority()?;
                    let delete_eligible = reader.opt_bool()?;
                    let member_count = reader.u32()? as usize;
                    let alert_count = reader.u32()? as usize;
                    total_members = total_members.saturating_add(member_count);
                    if total_members > MAX_MEMBERS_PER_SECTION
                        || alert_count > MAX_ALERTS_PER_THEATRE
                    {
                        return invalid("THS2 nested count exceeds bound");
                    }
                    let mut members = Vec::with_capacity(member_count);
                    for _ in 0..member_count {
                        let member_kind = ObjectKind::from_raw(reader.byte()?)?;
                        let parent_kind = ObjectKind::from_raw(reader.byte()?)?;
                        let reserve_navy = reader.bool()?;
                        let has_name = reader.bool()?;
                        let member_object_id = reader.u32()?;
                        let parent_id = reader.u32()?;
                        let member_engine_order = reader.u32()?;
                        let member_priority = reader.priority()?;
                        let member_orders = reader.orders_visibility()?;
                        let (member_name, member_provenance) = if has_name {
                            (Some(reader.string()?), Some(reader.provenance()?))
                        } else {
                            (None, None)
                        };
                        members.push(MemberRow {
                            kind: member_kind,
                            object_id: member_object_id,
                            parent_kind,
                            parent_id,
                            engine_order: member_engine_order,
                            reserve_navy,
                            display_name: member_name,
                            name_provenance: member_provenance,
                            priority_ordinal: member_priority,
                            orders_visibility: member_orders,
                        });
                    }
                    let mut alert_categories = Vec::with_capacity(alert_count);
                    for _ in 0..alert_count {
                        alert_categories.push(reader.u32()?);
                    }
                    theatres.push(TheatreRow {
                        kind,
                        object_id,
                        owner_tag_db_id: theatre_owner,
                        engine_order,
                        display_name,
                        name_provenance,
                        priority_ordinal,
                        delete_eligible,
                        members,
                        alert_categories,
                    });
                }
                let mut free_air_wings = Vec::with_capacity(free_wing_count);
                for _ in 0..free_wing_count {
                    free_air_wings.push(FreeAirWingRow {
                        wing_id: reader.u32()?,
                        engine_order: reader.u32()?,
                        priority_ordinal: reader.priority()?,
                    });
                }
                SectionAdmission::Admitted(SectionBody {
                    theatres,
                    free_air_wings,
                })
            } else {
                SectionAdmission::Unavailable(SectionRejection::from_raw(reader.u32()?)?)
            };
            sections.push(CountrySection {
                owner_tag_db_id,
                admission,
            });
        }
        (
            Some(ContentIdentity {
                language,
                localization_content_sha256,
                ordered_layers,
                engine_roster,
            }),
            Some(TheatreSelectorSnapshot {
                census,
                sections,
                producer_complete,
            }),
        )
    } else {
        if producer_complete_raw != 0 {
            return invalid("THS2 bodyless frame must not claim producer-complete");
        }
        (None, None)
    };
    if reader.at != payload.len() {
        return Err(CarrierError::TrailingBytes);
    }
    let frame = TheatreSelectorFrame {
        elapsed_wall_ms,
        status_flags,
        frame_hour,
        source_sequence,
        authority,
        content_identity,
        snapshot,
    };
    validate_frame(&frame)?;
    Ok(frame)
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CarrierError> {
    Err(CarrierError::Invalid(message.into()))
}
fn u32v(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn u64v(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn string(out: &mut Vec<u8>, value: &str) {
    u32v(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}
fn opt_string(out: &mut Vec<u8>, value: &str) {
    out.push(u8::from(!value.is_empty()));
    out.extend_from_slice(&[0; 3]);
    if !value.is_empty() {
        string(out, value);
    }
}
fn provenance(out: &mut Vec<u8>, value: &NameProvenance) {
    out.push(value.tag().code());
    out.extend_from_slice(&[0; 3]);
    if let NameProvenance::LocalizationKey(key) = value {
        string(out, key);
    }
}
fn opt_bool(out: &mut Vec<u8>, value: Option<bool>) {
    out.push(u8::from(value.is_some()));
    out.push(u8::from(value.unwrap_or(false)));
    out.extend_from_slice(&[0; 2]);
}
fn orders_visibility(out: &mut Vec<u8>, value: Option<ArmyOrdersVisibility>) {
    out.push(u8::from(value.is_some()));
    let orders = value.unwrap_or(ArmyOrdersVisibility {
        training_visible: false,
        stop_at_max_visible: false,
        border_conflict_visible: false,
    });
    out.push(u8::from(orders.training_visible));
    out.push(u8::from(orders.stop_at_max_visible));
    out.push(u8::from(orders.border_conflict_visible));
}
fn priority(out: &mut Vec<u8>, value: PriorityOrdinal) {
    match value {
        PriorityOrdinal::Unavailable => {
            out.push(0);
            out.extend_from_slice(&[0; 3]);
            u32v(out, 0);
        }
        PriorityOrdinal::Measured(ordinal) => {
            out.push(1);
            out.extend_from_slice(&[0; 3]);
            u32v(out, ordinal);
        }
    }
}

struct Reader<'a> {
    payload: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.payload.len().saturating_sub(self.at)
    }
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> Result<&'a [u8], CarrierError> {
        let end = self.at.checked_add(len).ok_or(CarrierError::Truncated)?;
        let value = self
            .payload
            .get(self.at..end)
            .ok_or(CarrierError::Truncated)?;
        self.at = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, CarrierError> {
        Ok(self.bytes(1)?[0])
    }
    fn bool(&mut self) -> Result<bool, CarrierError> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => invalid("THS2 bool byte is invalid"),
        }
    }
    fn u32(&mut self) -> Result<u32, CarrierError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CarrierError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, CarrierError> {
        let len = self.u32()? as usize;
        if len == 0 || len > MAX_DISPLAY_NAME_BYTES {
            return invalid("THS2 string length is invalid");
        }
        String::from_utf8(self.bytes(len)?.to_vec())
            .map_err(|_| CarrierError::Invalid("THS2 string is not UTF-8".into()))
    }
    fn opt_string(&mut self) -> Result<String, CarrierError> {
        let present = self.bool()?;
        if self.bytes(3)? != [0, 0, 0] {
            return invalid("THS2 optional string reserved bytes are nonzero");
        }
        if present {
            self.string()
        } else {
            Ok(String::new())
        }
    }
    fn provenance(&mut self) -> Result<NameProvenance, CarrierError> {
        let tag = self.byte()?;
        if self.bytes(3)? != [0, 0, 0] {
            return invalid("THS2 provenance reserved bytes are nonzero");
        }
        match NameProvenanceTag::from_code(tag)? {
            NameProvenanceTag::EngineRuntimeString => Ok(NameProvenance::EngineRuntimeString),
            NameProvenanceTag::LocalizationKey => {
                Ok(NameProvenance::LocalizationKey(self.string()?))
            }
        }
    }
    fn opt_bool(&mut self) -> Result<Option<bool>, CarrierError> {
        let present = self.bool()?;
        let value = self.bool()?;
        if self.bytes(2)? != [0, 0] {
            return invalid("THS2 optional bool reserved bytes are nonzero");
        }
        if !present && value {
            return invalid("THS2 absent optional bool carries a true value");
        }
        Ok(present.then_some(value))
    }
    fn orders_visibility(&mut self) -> Result<Option<ArmyOrdersVisibility>, CarrierError> {
        let present = self.bool()?;
        let orders = ArmyOrdersVisibility {
            training_visible: self.bool()?,
            stop_at_max_visible: self.bool()?,
            border_conflict_visible: self.bool()?,
        };
        if !present {
            if orders.training_visible
                || orders.stop_at_max_visible
                || orders.border_conflict_visible
            {
                return invalid("THS2 absent orders-group visibility carries a true value");
            }
            return Ok(None);
        }
        if !orders.law_holds() {
            return invalid("THS2 orders-group visibility violates its engine law");
        }
        Ok(Some(orders))
    }
    fn priority(&mut self) -> Result<PriorityOrdinal, CarrierError> {
        let tag = self.byte()?;
        if self.bytes(3)? != [0, 0, 0] {
            return invalid("THS2 priority reserved bytes are nonzero");
        }
        let ordinal = self.u32()?;
        match tag {
            0 if ordinal == 0 => Ok(PriorityOrdinal::Unavailable),
            0 => invalid("THS2 unavailable priority must carry a zero ordinal"),
            1 => Ok(PriorityOrdinal::Measured(ordinal)),
            _ => invalid("THS2 priority tag is unknown"),
        }
    }
}

#[cfg(test)]
mod tests;
