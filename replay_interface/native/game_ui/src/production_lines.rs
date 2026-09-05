
use crate::assets::{normalize_rel_path, AssetResolver};
use crate::font::BitmapFont;
use crate::gfx::SpriteRegistry;
use crate::gui::{GuiElement, LayoutSet};
use crate::loc::{is_localisation_key, Localisation};
use replay_core::{
    ConvoyPriorityReservation, ConvoyVariantAmount, DeploymentConveyorRow, DesignKind,
    DesignRecord, EquipmentProductionLineRow, FieldCompleteness, Leaf,
    ProductionLicenseAgreementRow, ProductionView, TradeView, ValidatedContentAdmission,
    ViewStatus,
};
use snapshot::sha256::Sha256;
use std::collections::{BTreeMap, BTreeSet};

pub const COUNTRY_PRODUCTION_LINE_ROOT: &str = "countryproductionlineview";
pub const XUI12_PRODUCER_PAYLOAD_FIELD: &str = "_r2_xui12_production_lines_payload";
pub const XUI12_PRODUCER_AUTHORITY_FIELD: &str = "_r2_xui12_producer_canvas_authority";
pub const XUI12_PRODUCER_AUTHORITY_SCHEMA: &str = "replay.xui12.production-lines-authority.a729.v2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProductionCarrierField(&'static str);

impl ProductionCarrierField {
    pub const fn new(field: &'static str) -> Self {
        Self(field)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

const MILITARY_LINES_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(military_lines));
const NAVAL_LINES_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(naval_lines));
const STOCKPILE_FIELD: ProductionCarrierField = ProductionCarrierField::new(stringify!(stockpile));
const CONVOY_VARIANTS_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(convoy_variants));
const PRODUCTION_LICENCES_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(production_license_agreements));
const CONVOY_PRIORITY_RESERVATIONS_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(convoy_priority_reservations));
const DEPLOYMENT_QUEUE_FIELD: ProductionCarrierField =
    ProductionCarrierField::new(stringify!(deployment_queue));

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ViewerRowPresentation {
    Expanded,
    Collapsed,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Xui12ProductionKind {
    Military,
    Naval,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Xui12RowIdentity {
    ExistingLine {
        kind: Xui12ProductionKind,
        line_id: i64,
    },
    OptionalNewLine(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Xui12ViewerState {
    pub panel_open: bool,
    pub scroll_offset_px: i32,
    pub row_presentation: BTreeMap<Xui12RowIdentity, ViewerRowPresentation>,
}

impl Xui12ViewerState {
    pub fn apply_bounded_scroll_delta(&mut self, delta_px: i32, maximum_offset_px: i32) {
        let maximum_offset_px = maximum_offset_px.max(0);
        self.scroll_offset_px = self
            .scroll_offset_px
            .saturating_add(delta_px)
            .clamp(0, maximum_offset_px);
    }

    pub fn set_row_presentation(
        &mut self,
        identity: Xui12RowIdentity,
        presentation: ViewerRowPresentation,
    ) {
        self.row_presentation.insert(identity, presentation);
    }

    pub fn reset_for_seek_or_country_change(&mut self) {
        self.scroll_offset_px = 0;
        self.row_presentation.clear();
    }

    pub fn presentation_for(&self, identity: &Xui12RowIdentity) -> ViewerRowPresentation {
        self.row_presentation
            .get(identity)
            .copied()
            .unwrap_or(ViewerRowPresentation::Expanded)
    }
}

impl Default for Xui12ViewerState {
    fn default() -> Self {
        Self {
            panel_open: true,
            scroll_offset_px: 0,
            row_presentation: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PreservedLeaf<T> {
    Absent,
    Unobtained,
    Measured(T),
}

impl<T: Clone> From<&Leaf<T>> for PreservedLeaf<T> {
    fn from(value: &Leaf<T>) -> Self {
        match value {
            Leaf::Absent => Self::Absent,
            Leaf::Unobtained => Self::Unobtained,
            Leaf::Measured(value) => Self::Measured(value.clone()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingLeafState {
    Absent,
    Unobtained,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedProductionLine {
    pub source_ordinal: usize,
    pub row_identity: Xui12RowIdentity,
    pub carrier_field: ProductionCarrierField,
    pub presentation: ViewerRowPresentation,
    pub id: i64,
    pub priority: i64,
    pub line_subtype: String,
    pub equipment_variant_id: String,
    pub equipment_display_name: String,
    pub active_factories: i64,
    pub requested_factories: i64,
    pub efficiency: f64,
    pub efficiency_cap: f64,
    pub efficiency_growth: f64,
    pub cost: f64,
    pub speed: f64,
    pub produced: f64,
    pub amount: i64,
    pub progress_ratio: f64,
    pub daily_output: f64,
    pub days_to_next_unit: f64,
    pub mio_id: PreservedLeaf<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedXui12ProductionLines {
    pub selected_country: String,
    pub scroll_offset_px: i32,
    pub military_lines: Vec<AdmittedProductionLine>,
    pub naval_lines: Vec<AdmittedProductionLine>,
    pub stockpile_total: f64,
    pub stockpile: BTreeMap<String, f64>,
    pub convoys_owned_raw: i64,
    pub convoys_owned: f64,
    pub convoys_assigned: i64,
    pub convoys_available: i64,
    pub convoys_cache_valid: bool,
    pub convoy_variants: Vec<ConvoyVariantAmount>,
    pub convoy_priority_reservations: Vec<ConvoyPriorityReservation>,
    pub deployment_queue: Vec<DeploymentConveyorRow>,
    pub production_license_agreements: Vec<ProductionLicenseAgreementRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Xui12PixelCarrierGap {
    GlobalSourceOrdinal,
    OptionalNewLineIdentityVisibilityAndOrder,
    ExplicitMioAssignment,
    PerLineUiState,
}

impl Xui12PixelCarrierGap {
    pub const fn field(self) -> &'static str {
        match self {
            Self::GlobalSourceOrdinal => "global_source_ordinal",
            Self::OptionalNewLineIdentityVisibilityAndOrder => {
                "optional_new_line_identity_visibility_and_order"
            }
            Self::ExplicitMioAssignment => "explicit_mio_assignment",
            Self::PerLineUiState => "per_line_ui_state",
        }
    }

    pub const fn reason(self) -> &'static str {
        match self {
            Self::GlobalSourceOrdinal => {
                "the canonical carrier partitions military and naval lines without preserving the engine's single global row ordinal"
            }
            Self::OptionalNewLineIdentityVisibilityAndOrder => {
                "the canonical carrier does not preserve the engine-evaluated optional new-line rows, visibility, or order"
            }
            Self::ExplicitMioAssignment => {
                "the canonical carrier cannot distinguish an explicit no-MIO assignment from an unobtained value"
            }
            Self::PerLineUiState => {
                "the canonical carrier lacks the complete engine-evaluated per-line controller state required by the visible authored widgets"
            }
        }
    }
}

pub const XUI12_LEGACY_PARTIAL_VIEW_GAPS: &[Xui12PixelCarrierGap] = &[
    Xui12PixelCarrierGap::GlobalSourceOrdinal,
    Xui12PixelCarrierGap::OptionalNewLineIdentityVisibilityAndOrder,
    Xui12PixelCarrierGap::ExplicitMioAssignment,
    Xui12PixelCarrierGap::PerLineUiState,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui12PixelRouteRejection {
    Semantic(Xui12ProductionLineRejection),
    MissingCanonicalCarrier(Vec<Xui12PixelCarrierGap>),
}

impl From<Xui12ProductionLineRejection> for Xui12PixelRouteRejection {
    fn from(value: Xui12ProductionLineRejection) -> Self {
        Self::Semantic(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedXui12CanvasRow {
    pub global_source_ordinal: u32,
    pub resolved_width_px: i32,
    pub resolved_height_px: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedXui12CanvasRowRect {
    pub global_source_ordinal: u32,
    pub rect: crate::compose::Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrictXui12CanvasRow {
    pub row_identity: Xui12RowIdentity,
    pub global_source_ordinal: u32,
    pub template_name: String,
    pub resolved_width_px: i32,
    pub resolved_height_px: i32,
    pub semantics: BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerRowPresentation {
    pub template_name: String,
    pub resolved_width_px: i32,
    pub resolved_height_px: i32,
    pub semantics: BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerCanvasRow {
    pub row_identity: Xui12RowIdentity,
    pub global_source_ordinal: u32,
    pub presentations: BTreeMap<ViewerRowPresentation, Xui12ProducerRowPresentation>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerCanvasPayload {
    pub country_tag: String,
    pub tag_db_id: u32,
    pub source_ordinal: u32,
    pub measured_empty: bool,
    pub owner_semantics:
        BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
    pub rows: Vec<Xui12ProducerCanvasRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerCountryCompletenessAuthority {
    pub country_tag: String,
    pub tag_db_id: u32,
    pub source_ordinal: u32,
    pub row_identities: Vec<Xui12RowIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerCompletenessAuthority {
    pub countries: Vec<Xui12ProducerCountryCompletenessAuthority>,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui12ProducerFramePayload {
    pub executable_sha256: String,
    pub producer_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub completeness_record_sha256: String,
    pub countries: Vec<Xui12ProducerCanvasPayload>,
}

impl Xui12ProducerFramePayload {
    pub fn receipt_sha256(&self) -> String {
        xui12_producer_receipt_sha256(self)
    }

    pub fn replay_authority_record(
        &self,
        frame_hour: i64,
        completeness_authority: &Xui12ProducerCompletenessAuthority,
    ) -> serde_json::Value {
        serde_json::json!({
            "schema": XUI12_PRODUCER_AUTHORITY_SCHEMA,
            "frame_hour": frame_hour,
            "executable_sha256": self.executable_sha256,
            "producer_manifest_sha256": self.producer_manifest_sha256,
            "replay_frame_identity_sha256": self.replay_frame_identity_sha256,
            "completeness_record_sha256": xui12_completeness_record_sha256(completeness_authority),
            "completeness_authority": completeness_authority,
            "payload_receipt_sha256": self.receipt_sha256(),
        })
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodedXui12ProducerAuthority {
    schema: String,
    frame_hour: i64,
    executable_sha256: String,
    producer_manifest_sha256: String,
    replay_frame_identity_sha256: String,
    completeness_record_sha256: String,
    completeness_authority: Xui12ProducerCompletenessAuthority,
    payload_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
struct AuthenticatedXui12ProducerCanvas {
    payload: Xui12ProducerFramePayload,
    producer_receipt_sha256: String,
}

impl AuthenticatedXui12ProducerCanvas {
    fn authenticate_from_frame(
        payload: Xui12ProducerFramePayload,
        frame: &replay_core::WorldFrame,
        admitted_manifest_sha256: &str,
    ) -> Result<Self, Xui12CanvasLayoutRejection> {
        let authority = frame
            .domain(XUI12_PRODUCER_AUTHORITY_FIELD)
            .cloned()
            .ok_or(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
            .and_then(|value| {
                serde_json::from_value::<DecodedXui12ProducerAuthority>(value)
                    .map_err(|_| Xui12CanvasLayoutRejection::InvalidProducerAuthority)
            })?;
        let replay_frame_identity_sha256 = xui12_replay_frame_identity_sha256(frame)?;
        if authority.schema != XUI12_PRODUCER_AUTHORITY_SCHEMA
            || authority.frame_hour != frame.hour_index()
            || authority.executable_sha256 != map_viewer::XMAP03_A729_EXECUTABLE_SHA256
            || !crate::is_lowercase_sha256(&authority.producer_manifest_sha256)
            || !crate::is_lowercase_sha256(&authority.replay_frame_identity_sha256)
            || !crate::is_lowercase_sha256(&authority.completeness_record_sha256)
            || !crate::is_lowercase_sha256(&authority.payload_receipt_sha256)
            || authority.producer_manifest_sha256 != admitted_manifest_sha256
            || authority.replay_frame_identity_sha256 != replay_frame_identity_sha256
            || payload.executable_sha256 != authority.executable_sha256
            || payload.producer_manifest_sha256 != authority.producer_manifest_sha256
            || payload.replay_frame_identity_sha256 != authority.replay_frame_identity_sha256
            || payload.completeness_record_sha256 != authority.completeness_record_sha256
            || xui12_producer_receipt_sha256(&payload) != authority.payload_receipt_sha256
            || xui12_completeness_record_sha256(&authority.completeness_authority)
                != authority.completeness_record_sha256
            || payload.executable_sha256 != map_viewer::XMAP03_A729_EXECUTABLE_SHA256
            || !crate::is_lowercase_sha256(&payload.producer_manifest_sha256)
            || !crate::is_lowercase_sha256(&payload.replay_frame_identity_sha256)
            || !crate::is_lowercase_sha256(&payload.completeness_record_sha256)
        {
            return Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority);
        }
        validate_xui12_complete_country_roster(&payload, frame)?;
        validate_xui12_completeness_authority(&authority.completeness_authority, &payload)?;
        validate_xui12_payload_shape(&payload)?;
        Ok(Self {
            payload,
            producer_receipt_sha256: authority.payload_receipt_sha256,
        })
    }

    fn seal_for_frame_and_viewer(
        &self,
        frame: &replay_core::WorldFrame,
        selected_country: &str,
        viewer: &Xui12ViewerState,
    ) -> Result<StrictXui12CanvasInput, Xui12CanvasLayoutRejection> {
        let replay_frame_identity_sha256 = xui12_replay_frame_identity_sha256(frame)?;
        self.seal_for_replay_frame_identity_and_viewer(
            &replay_frame_identity_sha256,
            selected_country,
            viewer,
        )
    }

    pub fn seal_for_replay_frame_identity_and_viewer(
        &self,
        replay_frame_identity_sha256: &str,
        selected_country: &str,
        viewer: &Xui12ViewerState,
    ) -> Result<StrictXui12CanvasInput, Xui12CanvasLayoutRejection> {
        if self.payload.replay_frame_identity_sha256 != replay_frame_identity_sha256 {
            return Err(Xui12CanvasLayoutRejection::StaleReplayFrame);
        }
        let country = self
            .payload
            .countries
            .iter()
            .find(|country| country.country_tag == selected_country)
            .ok_or(Xui12CanvasLayoutRejection::IncompleteCountryCensus)?;
        let rows = country
            .rows
            .iter()
            .map(|row| {
                let selected = viewer.presentation_for(&row.row_identity);
                let presentation = row
                    .presentations
                    .get(&selected)
                    .ok_or(Xui12CanvasLayoutRejection::InvalidProducerAuthority)?;
                Ok(StrictXui12CanvasRow {
                    row_identity: row.row_identity.clone(),
                    global_source_ordinal: row.global_source_ordinal,
                    template_name: presentation.template_name.clone(),
                    resolved_width_px: presentation.resolved_width_px,
                    resolved_height_px: presentation.resolved_height_px,
                    semantics: presentation.semantics.clone(),
                })
            })
            .collect::<Result<Vec<_>, Xui12CanvasLayoutRejection>>()?;
        let mut input = StrictXui12CanvasInput {
            country_tag: country.country_tag.clone(),
            completeness_record: self.payload.completeness_record_sha256.clone(),
            owner_semantics: country.owner_semantics.clone(),
            rows,
            scroll_offset_px: viewer.scroll_offset_px,
            authority: StrictXui12CanvasAuthority {
                replay_frame_identity_sha256: self.payload.replay_frame_identity_sha256.clone(),
                viewer_state_identity_sha256: xui12_viewer_state_identity_sha256(viewer),
                producer_manifest_sha256: self.payload.producer_manifest_sha256.clone(),
                producer_receipt_sha256: self.producer_receipt_sha256.clone(),
                canvas_identity_sha256: String::new(),
            },
        };
        input.authority.canvas_identity_sha256 = strict_xui12_canvas_identity_sha256(&input);
        input.validate_for_replay_frame_identity_and_viewer(
            replay_frame_identity_sha256,
            selected_country,
            viewer,
        )?;
        Ok(input)
    }
}

pub fn seal_xui12_producer_canvas_for_frame_and_viewer(
    payload: Xui12ProducerFramePayload,
    frame: &replay_core::WorldFrame,
    content_admission: &replay_core::ValidatedContentAdmission,
    selected_country: &str,
    viewer: &Xui12ViewerState,
) -> Result<StrictXui12CanvasInput, Xui12CanvasLayoutRejection> {
    AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
        payload,
        frame,
        content_admission.manifest_digest_sha256(),
    )?
    .seal_for_frame_and_viewer(frame, selected_country, viewer)
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrictXui12CanvasInput {
    pub country_tag: String,
    pub completeness_record: String,
    pub owner_semantics:
        BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
    pub rows: Vec<StrictXui12CanvasRow>,
    pub scroll_offset_px: i32,
    authority: StrictXui12CanvasAuthority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StrictXui12CanvasAuthority {
    replay_frame_identity_sha256: String,
    viewer_state_identity_sha256: String,
    producer_manifest_sha256: String,
    producer_receipt_sha256: String,
    canvas_identity_sha256: String,
}

impl StrictXui12CanvasInput {
    pub fn validate_shape(&self) -> Result<(), Xui12CanvasLayoutRejection> {
        if self.country_tag.trim().is_empty()
            || !crate::is_lowercase_sha256(&self.completeness_record)
            || !crate::is_lowercase_sha256(&self.authority.replay_frame_identity_sha256)
            || !crate::is_lowercase_sha256(&self.authority.viewer_state_identity_sha256)
            || !crate::is_lowercase_sha256(&self.authority.producer_manifest_sha256)
            || !crate::is_lowercase_sha256(&self.authority.producer_receipt_sha256)
            || !crate::is_lowercase_sha256(&self.authority.canvas_identity_sha256)
            || strict_xui12_canvas_identity_sha256(self) != self.authority.canvas_identity_sha256
        {
            return Err(Xui12CanvasLayoutRejection::IncompleteTypedAdapter);
        }
        let mut identities = BTreeSet::new();
        for (expected, row) in self.rows.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| Xui12CanvasLayoutRejection::PixelOverflow)?;
            if row.global_source_ordinal != expected {
                return Err(Xui12CanvasLayoutRejection::NonDenseGlobalSourceOrdinal {
                    expected,
                    actual: row.global_source_ordinal,
                });
            }
            if row.resolved_width_px <= 0
                || row.resolved_height_px <= 0
                || row.template_name.trim().is_empty()
                || row.semantics.is_empty()
                || !identities.insert(row.row_identity.clone())
            {
                return Err(Xui12CanvasLayoutRejection::IncompleteTypedAdapter);
            }
        }
        if self.scroll_offset_px < 0 || self.owner_semantics.is_empty() {
            return Err(Xui12CanvasLayoutRejection::IncompleteTypedAdapter);
        }
        Ok(())
    }

    pub fn validate_for_frame_and_viewer(
        &self,
        frame: &replay_core::WorldFrame,
        selected_country: &str,
        viewer: &Xui12ViewerState,
    ) -> Result<(), Xui12CanvasLayoutRejection> {
        let replay_frame_identity_sha256 = xui12_replay_frame_identity_sha256(frame)?;
        self.validate_for_replay_frame_identity_and_viewer(
            &replay_frame_identity_sha256,
            selected_country,
            viewer,
        )
    }

    fn validate_for_replay_frame_identity_and_viewer(
        &self,
        replay_frame_identity_sha256: &str,
        selected_country: &str,
        viewer: &Xui12ViewerState,
    ) -> Result<(), Xui12CanvasLayoutRejection> {
        self.validate_shape()?;
        if self.authority.replay_frame_identity_sha256 != replay_frame_identity_sha256
            || self.country_tag != selected_country
        {
            return Err(Xui12CanvasLayoutRejection::StaleReplayFrame);
        }
        if self.authority.viewer_state_identity_sha256 != xui12_viewer_state_identity_sha256(viewer)
            || self.scroll_offset_px != viewer.scroll_offset_px
        {
            return Err(Xui12CanvasLayoutRejection::StaleViewerState);
        }
        Ok(())
    }

    pub fn replay_frame_identity_sha256(&self) -> &str {
        &self.authority.replay_frame_identity_sha256
    }

    pub fn viewer_state_identity_sha256(&self) -> &str {
        &self.authority.viewer_state_identity_sha256
    }
}

fn xui12_producer_receipt_sha256(payload: &Xui12ProducerFramePayload) -> String {
    let mut bytes = b"hoi4.xui12.passive-all-country-payload.v2\0".to_vec();
    digest_field(&mut bytes, payload.executable_sha256.as_bytes());
    digest_field(&mut bytes, payload.producer_manifest_sha256.as_bytes());
    digest_field(&mut bytes, payload.replay_frame_identity_sha256.as_bytes());
    digest_field(&mut bytes, payload.completeness_record_sha256.as_bytes());
    bytes.extend_from_slice(&(payload.countries.len() as u64).to_le_bytes());
    for country in &payload.countries {
        digest_field(&mut bytes, country.country_tag.as_bytes());
        bytes.extend_from_slice(&country.tag_db_id.to_le_bytes());
        bytes.extend_from_slice(&country.source_ordinal.to_le_bytes());
        bytes.push(u8::from(country.measured_empty));
        digest_semantics(&mut bytes, &country.owner_semantics);
        bytes.extend_from_slice(&(country.rows.len() as u64).to_le_bytes());
        for row in &country.rows {
            digest_xui12_row_identity(&mut bytes, &row.row_identity);
            bytes.extend_from_slice(&row.global_source_ordinal.to_le_bytes());
            for required in [
                ViewerRowPresentation::Expanded,
                ViewerRowPresentation::Collapsed,
            ] {
                bytes.push(match required {
                    ViewerRowPresentation::Expanded => 0,
                    ViewerRowPresentation::Collapsed => 1,
                });
                if let Some(presentation) = row.presentations.get(&required) {
                    bytes.push(1);
                    digest_field(&mut bytes, presentation.template_name.as_bytes());
                    bytes.extend_from_slice(&presentation.resolved_width_px.to_le_bytes());
                    bytes.extend_from_slice(&presentation.resolved_height_px.to_le_bytes());
                    digest_semantics(&mut bytes, &presentation.semantics);
                } else {
                    bytes.push(0);
                }
            }
        }
    }
    Sha256::hex(&bytes)
}

pub fn xui12_completeness_record_sha256(authority: &Xui12ProducerCompletenessAuthority) -> String {
    let mut bytes = b"hoi4.xui12.detached-completeness.v2\0".to_vec();
    bytes.extend_from_slice(&(authority.countries.len() as u64).to_le_bytes());
    for country in &authority.countries {
        digest_field(&mut bytes, country.country_tag.as_bytes());
        bytes.extend_from_slice(&country.tag_db_id.to_le_bytes());
        bytes.extend_from_slice(&country.source_ordinal.to_le_bytes());
        bytes.extend_from_slice(&(country.row_identities.len() as u64).to_le_bytes());
        for identity in &country.row_identities {
            digest_xui12_row_identity(&mut bytes, identity);
        }
    }
    Sha256::hex(&bytes)
}

pub fn xui12_producer_payload_from_frame(
    frame: &replay_core::WorldFrame,
) -> Result<Option<Xui12ProducerFramePayload>, Xui12CanvasLayoutRejection> {
    match (
        frame.domain(XUI12_PRODUCER_PAYLOAD_FIELD),
        frame.domain(XUI12_PRODUCER_AUTHORITY_FIELD),
    ) {
        (None, None) => Ok(None),
        (Some(payload), Some(_)) => serde_json::from_value(payload.clone())
            .map(Some)
            .map_err(|_| Xui12CanvasLayoutRejection::InvalidProducerAuthority),
        _ => Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority),
    }
}

fn validate_xui12_complete_country_roster(
    payload: &Xui12ProducerFramePayload,
    frame: &replay_core::WorldFrame,
) -> Result<(), Xui12CanvasLayoutRejection> {
    // COMPLETE, not partial, and the function is named for it: XUI12's canvas
    // lays out EVERY country and the payload is compared to the roster row for
    // row, so a roster short by a country the frame carries is exactly an
    // incomplete country census.
    let roster = frame.d18_country_roster();
    let roster_entries = match roster.complete() {
        Ok(entries) => entries,
        Err(refusal) => {
            return Err(Xui12CanvasLayoutRejection::IncompleteD18Roster(
                refusal.detail,
            ))
        }
    };
    if roster_entries.len() != payload.countries.len() {
        return Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus);
    }
    for (expected_ordinal, (expected, actual)) in
        roster_entries.iter().zip(&payload.countries).enumerate()
    {
        let expected_ordinal = u32::try_from(expected_ordinal)
            .map_err(|_| Xui12CanvasLayoutRejection::PixelOverflow)?;
        if actual.country_tag != expected.tag
            || actual.tag_db_id != expected.tag_db_id
            || actual.source_ordinal != expected_ordinal
        {
            return Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus);
        }
    }
    Ok(())
}

fn validate_xui12_completeness_authority(
    authority: &Xui12ProducerCompletenessAuthority,
    payload: &Xui12ProducerFramePayload,
) -> Result<(), Xui12CanvasLayoutRejection> {
    if authority.countries.len() != payload.countries.len() {
        return Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus);
    }
    for (census, materialized) in authority.countries.iter().zip(&payload.countries) {
        if census.country_tag != materialized.country_tag
            || census.tag_db_id != materialized.tag_db_id
            || census.source_ordinal != materialized.source_ordinal
            || census.row_identities.len() != materialized.rows.len()
            || census
                .row_identities
                .iter()
                .zip(&materialized.rows)
                .any(|(identity, row)| identity != &row.row_identity)
        {
            return Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus);
        }
    }
    Ok(())
}

fn validate_xui12_semantics(
    semantics: &BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
) -> bool {
    use crate::window_manager_law::ReplayAuthenticatedElementSemantic as Semantic;
    !semantics.is_empty()
        && semantics.iter().all(|(element_id, semantic)| {
            !element_id.trim().is_empty()
                && match semantic {
                    Semantic::Hidden
                    | Semantic::Structural
                    | Semantic::StandardButton
                    | Semantic::Text { .. } => true,
                    Semantic::ReadOnlyButton {
                        one_based_frame, ..
                    }
                    | Semantic::PassiveSprite { one_based_frame } => *one_based_frame > 0,
                    Semantic::Progress { fill } => fill.valid(),
                    Semantic::BlendFrames {
                        current_one_based_frame,
                        next_one_based_frame,
                        animation_time,
                    } => {
                        *current_one_based_frame > 0
                            && *next_one_based_frame > 0
                            && animation_time.is_finite()
                    }
                }
        })
}

fn validate_xui12_payload_shape(
    payload: &Xui12ProducerFramePayload,
) -> Result<(), Xui12CanvasLayoutRejection> {
    if payload.countries.is_empty() {
        return Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus);
    }
    let mut country_tags = BTreeSet::new();
    for country in &payload.countries {
        if country.country_tag.trim().is_empty()
            || country.tag_db_id == 0
            || !country_tags.insert(country.country_tag.clone())
            || country.measured_empty != country.rows.is_empty()
            || !validate_xui12_semantics(&country.owner_semantics)
        {
            return Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority);
        }
        let mut row_identities = BTreeSet::new();
        for (expected, row) in country.rows.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| Xui12CanvasLayoutRejection::PixelOverflow)?;
            if row.global_source_ordinal != expected
                || !row_identities.insert(row.row_identity.clone())
                || matches!(&row.row_identity, Xui12RowIdentity::OptionalNewLine(id) if id.trim().is_empty())
                || row.presentations.len() != 2
            {
                return Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority);
            }
            for required in [
                ViewerRowPresentation::Expanded,
                ViewerRowPresentation::Collapsed,
            ] {
                let presentation = row
                    .presentations
                    .get(&required)
                    .ok_or(Xui12CanvasLayoutRejection::InvalidProducerAuthority)?;
                if presentation.template_name.trim().is_empty()
                    || presentation.resolved_width_px <= 0
                    || presentation.resolved_height_px <= 0
                    || !validate_xui12_semantics(&presentation.semantics)
                {
                    return Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority);
                }
            }
        }
    }
    Ok(())
}

pub fn xui12_replay_frame_identity_sha256(
    frame: &replay_core::WorldFrame,
) -> Result<String, Xui12CanvasLayoutRejection> {
    frame
        .base_frame_identity_sha256(snapshot::replay_frame_identity::XUI12_BASE_FRAME_IDENTITY_DOMAIN)
        .map_err(|_| Xui12CanvasLayoutRejection::InvalidProducerAuthority)
}

fn digest_field(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value);
}

fn digest_semantics(
    bytes: &mut Vec<u8>,
    values: &BTreeMap<String, crate::window_manager_law::ReplayAuthenticatedElementSemantic>,
) {
    bytes.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for (element_id, semantic) in values {
        digest_field(bytes, element_id.as_bytes());
        digest_field(bytes, format!("{semantic:?}").as_bytes());
    }
}

fn xui12_viewer_state_identity_sha256(viewer: &Xui12ViewerState) -> String {
    let mut bytes = b"hoi4.xui12.viewer-state.v1\0".to_vec();
    bytes.push(u8::from(viewer.panel_open));
    bytes.extend_from_slice(&viewer.scroll_offset_px.to_le_bytes());
    bytes.extend_from_slice(&(viewer.row_presentation.len() as u64).to_le_bytes());
    for (identity, presentation) in &viewer.row_presentation {
        digest_xui12_row_identity(&mut bytes, identity);
        bytes.push(match presentation {
            ViewerRowPresentation::Expanded => 0,
            ViewerRowPresentation::Collapsed => 1,
        });
    }
    Sha256::hex(&bytes)
}

fn strict_xui12_canvas_identity_sha256(input: &StrictXui12CanvasInput) -> String {
    let mut bytes = b"hoi4.xui12.strict-canvas.v1\0".to_vec();
    digest_field(
        &mut bytes,
        input.authority.replay_frame_identity_sha256.as_bytes(),
    );
    digest_field(
        &mut bytes,
        input.authority.viewer_state_identity_sha256.as_bytes(),
    );
    digest_field(
        &mut bytes,
        input.authority.producer_manifest_sha256.as_bytes(),
    );
    digest_field(
        &mut bytes,
        input.authority.producer_receipt_sha256.as_bytes(),
    );
    digest_field(&mut bytes, input.completeness_record.as_bytes());
    digest_field(&mut bytes, input.country_tag.as_bytes());
    bytes.extend_from_slice(&input.scroll_offset_px.to_le_bytes());
    digest_semantics(&mut bytes, &input.owner_semantics);
    bytes.extend_from_slice(&(input.rows.len() as u64).to_le_bytes());
    for row in &input.rows {
        digest_xui12_row_identity(&mut bytes, &row.row_identity);
        bytes.extend_from_slice(&row.global_source_ordinal.to_le_bytes());
        digest_field(&mut bytes, row.template_name.as_bytes());
        bytes.extend_from_slice(&row.resolved_width_px.to_le_bytes());
        bytes.extend_from_slice(&row.resolved_height_px.to_le_bytes());
        digest_semantics(&mut bytes, &row.semantics);
    }
    Sha256::hex(&bytes)
}

fn digest_xui12_row_identity(bytes: &mut Vec<u8>, identity: &Xui12RowIdentity) {
    match identity {
        Xui12RowIdentity::ExistingLine { kind, line_id } => {
            bytes.push(match kind {
                Xui12ProductionKind::Military => 0,
                Xui12ProductionKind::Naval => 1,
            });
            bytes.extend_from_slice(&line_id.to_le_bytes());
        }
        Xui12RowIdentity::OptionalNewLine(identity) => {
            bytes.push(2);
            bytes.extend_from_slice(&(identity.len() as u64).to_le_bytes());
            bytes.extend_from_slice(identity.as_bytes());
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticatedXui12Root {
    pub source_file: String,
    pub root: GuiElement,
    pub authored_definitions: Vec<GuiElement>,
    pub admitted_dependencies: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticatedXui12RowTemplate {
    pub carrier_field: ProductionCarrierField,
    pub source_ordinal: usize,
    pub line_subtype: String,
    pub presentation: ViewerRowPresentation,
    pub authored: GuiElement,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedXui12ProductionLineRoute {
    pub carrier: AdmittedXui12ProductionLines,
    pub authored_root: AuthenticatedXui12Root,
    pub military_row_templates: Vec<AuthenticatedXui12RowTemplate>,
    pub naval_row_templates: Vec<AuthenticatedXui12RowTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui12ProductionLineRejection {
    AuthoredSourceNotAdmitted {
        path: String,
    },
    AuthoredGuiUnreadable {
        path: String,
    },
    AuthoredGuiNotStrict {
        path: String,
    },
    SemanticRootCandidateCount {
        count: usize,
    },
    AuthoredSpriteMissing {
        sprite: String,
    },
    AuthoredFontMissing {
        font: String,
    },
    AuthoredRowTemplateIdentityInvalid {
        line_subtype: String,
    },
    AuthoredRowTemplateCandidateCount {
        line_subtype: String,
        presentation: ViewerRowPresentation,
        count: usize,
    },
    CompletenessNotMeasuredTrue {
        carrier_field: ProductionCarrierField,
        state: MissingLeafState,
    },
    CompletenessMeasuredFalse {
        carrier_field: ProductionCarrierField,
    },
    SupportingCompletenessNotMeasuredTrue {
        carrier_field: ProductionCarrierField,
        state: MissingLeafState,
    },
    SupportingCompletenessMeasuredFalse {
        carrier_field: ProductionCarrierField,
    },
    SupportingLeafNotMeasured {
        carrier_field: ProductionCarrierField,
        field: &'static str,
        state: MissingLeafState,
    },
    SupportingMeasuredValueInvalid {
        carrier_field: ProductionCarrierField,
        field: &'static str,
    },
    RequiredLeafNotMeasured {
        carrier_field: ProductionCarrierField,
        row: usize,
        field: &'static str,
        state: MissingLeafState,
    },
    InvalidMeasuredValue {
        carrier_field: ProductionCarrierField,
        row: usize,
        field: &'static str,
    },
    DesignCatalogUnavailable,
    DesignCatalogHourUnavailable,
    VariantIdentityInvalid {
        carrier_field: ProductionCarrierField,
        row: usize,
    },
    VariantJoinMissing {
        carrier_field: ProductionCarrierField,
        row: usize,
        variant_id: String,
    },
    VariantJoinAmbiguous {
        carrier_field: ProductionCarrierField,
        row: usize,
        variant_id: String,
    },
    VariantDefinitionIncomplete {
        carrier_field: ProductionCarrierField,
        row: usize,
        variant_id: String,
    },
    VariantDisplayNameMissing {
        carrier_field: ProductionCarrierField,
        row: usize,
        variant_id: String,
    },
}

pub fn admit_xui12_authored_root(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    admission: &ValidatedContentAdmission,
) -> Result<AuthenticatedXui12Root, Xui12ProductionLineRejection> {
    let files = resolver
        .list_files_recursive("interface", Some("gui"))
        .map_err(|_| Xui12ProductionLineRejection::AuthoredGuiUnreadable {
            path: "interface/*.gui".into(),
        })?;
    let mut candidates = Vec::new();
    for file in files {
        let mut layouts = LayoutSet::default();
        if resolver.read_file(&file).and_then(|bytes| layouts.parse_content(&bytes, &file.rel_path).map_err(Into::into)).is_err() {
            let mentions_root = source_mentions_production_root(resolver, &file).map_err(|_| {
                Xui12ProductionLineRejection::AuthoredGuiUnreadable {
                    path: file.rel_path.clone(),
                }
            })?;
            if mentions_root {
                return Err(Xui12ProductionLineRejection::AuthoredGuiNotStrict {
                    path: file.rel_path,
                });
            }
            continue;
        }
        if layouts.stats.recovery_count != 0 || layouts.stats.decode_loss_count != 0 {
            if source_mentions_production_root(resolver, &file).map_err(|_| {
                Xui12ProductionLineRejection::AuthoredGuiUnreadable {
                    path: file.rel_path.clone(),
                }
            })? {
                return Err(Xui12ProductionLineRejection::AuthoredGuiNotStrict {
                    path: file.rel_path,
                });
            }
            continue;
        }
        for document in layouts.documents {
            for root in &document.roots {
                if root.name.eq_ignore_ascii_case(COUNTRY_PRODUCTION_LINE_ROOT) {
                    candidates.push((
                        document.source_file.clone(),
                        root.clone(),
                        document.roots.clone(),
                    ));
                }
            }
        }
    }
    let candidate_sources = candidates
        .iter()
        .map(|(source, _, _)| source.clone())
        .collect::<Vec<_>>();
    validate_semantic_root_sources(&candidate_sources)?;
    let (source_file, root, authored_definitions) =
        candidates.pop().expect("candidate count checked");
    require_admitted(admission, &source_file)?;

    let mut dependencies = BTreeSet::from([source_file.clone()]);
    admit_authored_element_dependencies(
        resolver,
        registry,
        localisation,
        admission,
        &root,
        &mut dependencies,
    )?;
    Ok(AuthenticatedXui12Root {
        source_file,
        root,
        authored_definitions,
        admitted_dependencies: dependencies,
    })
}

fn admit_authored_element_dependencies(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    admission: &ValidatedContentAdmission,
    element: &GuiElement,
    dependencies: &mut BTreeSet<String>,
) -> Result<(), Xui12ProductionLineRejection> {
    let mut sprites = BTreeSet::new();
    let mut fonts = BTreeSet::new();
    let mut loc_keys = BTreeSet::new();
    collect_authored_refs(element, &mut sprites, &mut fonts, &mut loc_keys);
    for sprite_name in sprites {
        let sprite = registry.get(&sprite_name).ok_or_else(|| {
            Xui12ProductionLineRejection::AuthoredSpriteMissing {
                sprite: sprite_name.clone(),
            }
        })?;
        require_admitted(admission, &sprite.source_file)?;
        dependencies.insert(sprite.source_file.clone());
        for texture in sprite.texture_paths() {
            require_admitted(admission, texture)?;
            dependencies.insert(texture.to_string());
        }
    }
    for font_name in fonts {
        let font = BitmapFont::load(resolver, &font_name).map_err(|_| {
            Xui12ProductionLineRejection::AuthoredFontMissing {
                font: font_name.clone(),
            }
        })?;
        require_admitted(admission, &font.source_file)?;
        dependencies.insert(font.source_file);
        for page in font.pages {
            require_admitted(admission, &page.file)?;
            dependencies.insert(page.file);
        }
    }
    if !loc_keys.is_empty() {
        if !localisation.parse_failures.is_empty() || !localisation.decode_lossy_files.is_empty() {
            return Err(Xui12ProductionLineRejection::AuthoredGuiNotStrict {
                path: "localisation".into(),
            });
        }
        for source in &localisation.source_files {
            require_admitted(admission, source)?;
            dependencies.insert(source.clone());
        }
    }
    Ok(())
}

fn validate_semantic_root_sources(
    candidate_sources: &[String],
) -> Result<(), Xui12ProductionLineRejection> {
    if candidate_sources.len() != 1 {
        return Err(Xui12ProductionLineRejection::SemanticRootCandidateCount {
            count: candidate_sources.len(),
        });
    }
    Ok(())
}

fn source_mentions_production_root(resolver: &AssetResolver, file: &crate::assets::ResolvedFile) -> crate::Result<bool> {
    resolver.read_file(file).map(|bytes| {
        crate::paradox_text::decode_text(&bytes)
            .to_ascii_lowercase()
            .contains(COUNTRY_PRODUCTION_LINE_ROOT)
    })
}

const ENGINE_PRODUCTION_ROW_PREFIX: &str = "production_";
const ENGINE_PRODUCTION_ROW_SUFFIX: &str = "_line_entry";
const ENGINE_COLLAPSED_ROW_SUFFIX: &str = "_collapsed";

pub fn discover_xui12_row_template(
    authored_root: &AuthenticatedXui12Root,
    line: &AdmittedProductionLine,
) -> Result<AuthenticatedXui12RowTemplate, Xui12ProductionLineRejection> {
    if line.line_subtype.is_empty()
        || !line
            .line_subtype
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(
            Xui12ProductionLineRejection::AuthoredRowTemplateIdentityInvalid {
                line_subtype: line.line_subtype.clone(),
            },
        );
    }
    let presentation_suffix = match line.presentation {
        ViewerRowPresentation::Expanded => "",
        ViewerRowPresentation::Collapsed => ENGINE_COLLAPSED_ROW_SUFFIX,
    };
    let expected = format!(
        "{ENGINE_PRODUCTION_ROW_PREFIX}{}{ENGINE_PRODUCTION_ROW_SUFFIX}{presentation_suffix}",
        line.line_subtype
    );
    let mut candidates = authored_root.authored_definitions.iter().filter(|element| {
        element
            .element_type
            .eq_ignore_ascii_case("containerWindowType")
            && element.name == expected
    });
    let Some(authored) = candidates.next() else {
        return Err(
            Xui12ProductionLineRejection::AuthoredRowTemplateCandidateCount {
                line_subtype: line.line_subtype.clone(),
                presentation: line.presentation,
                count: 0,
            },
        );
    };
    if candidates.next().is_some() {
        let count = authored_root
            .authored_definitions
            .iter()
            .filter(|element| {
                element
                    .element_type
                    .eq_ignore_ascii_case("containerWindowType")
                    && element.name == expected
            })
            .count();
        return Err(
            Xui12ProductionLineRejection::AuthoredRowTemplateCandidateCount {
                line_subtype: line.line_subtype.clone(),
                presentation: line.presentation,
                count,
            },
        );
    }
    Ok(AuthenticatedXui12RowTemplate {
        carrier_field: line.carrier_field,
        source_ordinal: line.source_ordinal,
        line_subtype: line.line_subtype.clone(),
        presentation: line.presentation,
        authored: authored.clone(),
    })
}

pub fn admit_xui12_production_line_route(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    admission: &ValidatedContentAdmission,
    frame: &replay_core::WorldFrame,
    selected_country: &str,
    viewer: &Xui12ViewerState,
) -> Result<Option<AdmittedXui12ProductionLineRoute>, Xui12ProductionLineRejection> {
    let Some(carrier) = admit_xui12_production_lines_from_frame(frame, selected_country, viewer)?
    else {
        return Ok(None);
    };
    let mut authored_root = admit_xui12_authored_root(resolver, registry, localisation, admission)?;
    let military_row_templates = carrier
        .military_lines
        .iter()
        .map(|line| discover_xui12_row_template(&authored_root, line))
        .collect::<Result<Vec<_>, _>>()?;
    let naval_row_templates = carrier
        .naval_lines
        .iter()
        .map(|line| discover_xui12_row_template(&authored_root, line))
        .collect::<Result<Vec<_>, _>>()?;
    for template in military_row_templates.iter().chain(&naval_row_templates) {
        admit_authored_element_dependencies(
            resolver,
            registry,
            localisation,
            admission,
            &template.authored,
            &mut authored_root.admitted_dependencies,
        )?;
    }
    Ok(Some(AdmittedXui12ProductionLineRoute {
        carrier,
        authored_root,
        military_row_templates,
        naval_row_templates,
    }))
}

pub fn admit_xui12_legacy_partial_pixel_route(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    admission: &ValidatedContentAdmission,
    frame: &replay_core::WorldFrame,
    selected_country: &str,
    viewer: &Xui12ViewerState,
) -> Result<Option<AdmittedXui12ProductionLineRoute>, Xui12PixelRouteRejection> {
    let semantic = admit_xui12_production_line_route(
        resolver,
        registry,
        localisation,
        admission,
        frame,
        selected_country,
        viewer,
    )?;
    if semantic.is_none() {
        return Ok(None);
    }
    if !XUI12_LEGACY_PARTIAL_VIEW_GAPS.is_empty() {
        return Err(Xui12PixelRouteRejection::MissingCanonicalCarrier(
            XUI12_LEGACY_PARTIAL_VIEW_GAPS.to_vec(),
        ));
    }
    Ok(semantic)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui12CanvasLayoutRejection {
    NegativeScrollOffset,
    NonDenseGlobalSourceOrdinal { expected: u32, actual: u32 },
    NonPositiveResolvedSize { global_source_ordinal: u32 },
    PixelOverflow,
    AuthoredGridLaw(String),
    IncompleteTypedAdapter,
    InvalidProducerAuthority,
    IncompleteCountryCensus,
    /// The D18 country roster is not COMPLETE, and this says which countries the
    /// recording could not name. Distinct from `IncompleteCountryCensus`, which
    /// is the producer's payload disagreeing with a roster that IS complete:
    /// one is a recorder gap and the other is a carrier defect, and a canvas
    /// that reported them identically would send every reader hunting for the
    /// wrong one.
    IncompleteD18Roster(String),
    StaleReplayFrame,
    StaleViewerState,
}

pub fn plan_xui12_canvas_row_rects(
    grid: &crate::widget_layout::GridBoxLayout,
    rows: &[AuthenticatedXui12CanvasRow],
    scroll_offset_px: i32,
) -> Result<Vec<AuthenticatedXui12CanvasRowRect>, Xui12CanvasLayoutRejection> {
    if scroll_offset_px < 0 {
        return Err(Xui12CanvasLayoutRejection::NegativeScrollOffset);
    }
    for (expected, row) in rows.iter().enumerate() {
        let expected =
            u32::try_from(expected).map_err(|_| Xui12CanvasLayoutRejection::PixelOverflow)?;
        if row.global_source_ordinal != expected {
            return Err(Xui12CanvasLayoutRejection::NonDenseGlobalSourceOrdinal {
                expected,
                actual: row.global_source_ordinal,
            });
        }
        if row.resolved_width_px <= 0 || row.resolved_height_px <= 0 {
            return Err(Xui12CanvasLayoutRejection::NonPositiveResolvedSize {
                global_source_ordinal: row.global_source_ordinal,
            });
        }
    }
    let sizes = rows
        .iter()
        .map(|row| (row.resolved_width_px, row.resolved_height_px))
        .collect::<Vec<_>>();
    let rects = grid
        .production_variable_height_slot_rects(&sizes)
        .map_err(|error| Xui12CanvasLayoutRejection::AuthoredGridLaw(error.to_string()))?;
    rows.iter()
        .zip(rects)
        .map(|(row, rect)| {
            let y = rect
                .y()
                .checked_sub(scroll_offset_px)
                .ok_or(Xui12CanvasLayoutRejection::PixelOverflow)?;
            Ok(AuthenticatedXui12CanvasRowRect {
                global_source_ordinal: row.global_source_ordinal,
                rect: crate::compose::Rect::new(rect.x(), y, rect.w(), rect.h()),
            })
        })
        .collect()
}

pub fn admit_xui12_production_lines_from_frame(
    frame: &replay_core::WorldFrame,
    selected_country: &str,
    viewer: &Xui12ViewerState,
) -> Result<Option<AdmittedXui12ProductionLines>, Xui12ProductionLineRejection> {
    if !viewer.panel_open || frame.production_status() != ViewStatus::Present {
        return Ok(None);
    }
    let Some(view) = frame.production(selected_country) else {
        return Ok(None);
    };
    if frame.trade_status() != ViewStatus::Present {
        return Ok(None);
    }
    let Some(trade) = frame.trade(selected_country) else {
        return Ok(None);
    };
    let (Leaf::Measured(military), Leaf::Measured(naval)) =
        (&view.military_lines, &view.naval_lines)
    else {
        return Ok(None);
    };
    if military.is_empty() && naval.is_empty() {
        return admit_xui12_production_view(view, trade, selected_country, &[], viewer);
    }
    let Some(catalog) = frame.design_catalog() else {
        return Err(Xui12ProductionLineRejection::DesignCatalogUnavailable);
    };
    let hour = u64::try_from(frame.hour_index())
        .map_err(|_| Xui12ProductionLineRejection::DesignCatalogHourUnavailable)?;
    let records = catalog
        .records_at_hour(hour)
        .map_err(|_| Xui12ProductionLineRejection::DesignCatalogHourUnavailable)?;
    admit_xui12_production_view(view, trade, selected_country, &records, viewer)
}

pub fn admit_xui12_production_view(
    view: &ProductionView,
    trade: &TradeView,
    selected_country: &str,
    records: &[DesignRecord],
    viewer: &Xui12ViewerState,
) -> Result<Option<AdmittedXui12ProductionLines>, Xui12ProductionLineRejection> {
    if !viewer.panel_open {
        return Ok(None);
    }
    let (Leaf::Measured(military), Leaf::Measured(naval)) =
        (&view.military_lines, &view.naval_lines)
    else {
        return Ok(None);
    };
    require_complete(MILITARY_LINES_FIELD, &view.military_lines_complete)?;
    require_complete(NAVAL_LINES_FIELD, &view.naval_lines_complete)?;
    let military_lines = admit_collection(MILITARY_LINES_FIELD, military, records, viewer)?;
    let naval_lines = admit_collection(NAVAL_LINES_FIELD, naval, records, viewer)?;
    require_support_complete(STOCKPILE_FIELD, &view.stockpile_complete)?;
    require_support_complete(CONVOY_VARIANTS_FIELD, &view.convoys_complete)?;
    require_support_complete(
        CONVOY_PRIORITY_RESERVATIONS_FIELD,
        &view.convoy_priority_reservations_complete,
    )?;
    require_support_complete(
        PRODUCTION_LICENCES_FIELD,
        &trade.production_license_agreements_complete,
    )?;
    let stockpile_total =
        *support_required(STOCKPILE_FIELD, "stockpile_total", &view.stockpile_total)?;
    let stockpile = support_required(STOCKPILE_FIELD, "stockpile", &view.stockpile)?.clone();
    let convoys_owned_raw = *support_required(
        CONVOY_VARIANTS_FIELD,
        "convoys_owned_raw",
        &view.convoys_owned_raw,
    )?;
    let convoys_owned =
        *support_required(CONVOY_VARIANTS_FIELD, "convoys_owned", &view.convoys_owned)?;
    let convoys_assigned = *support_required(
        CONVOY_VARIANTS_FIELD,
        "convoys_assigned",
        &view.convoys_assigned,
    )?;
    let convoys_available = *support_required(
        CONVOY_VARIANTS_FIELD,
        "convoys_available",
        &view.convoys_available,
    )?;
    let convoys_cache_valid = *support_required(
        CONVOY_VARIANTS_FIELD,
        "convoys_cache_valid",
        &view.convoys_cache_valid,
    )?;
    let convoy_variants = support_required(
        CONVOY_VARIANTS_FIELD,
        "convoy_variants",
        &view.convoy_variants,
    )?
    .clone();
    let convoy_priority_reservations = support_required(
        CONVOY_PRIORITY_RESERVATIONS_FIELD,
        "convoy_priority_reservations",
        &view.convoy_priority_reservations,
    )?
    .clone();
    let deployment_queue = support_required(
        DEPLOYMENT_QUEUE_FIELD,
        "deployment_queue",
        &view.deployment_queue,
    )?
    .clone();
    let production_license_agreements = support_required(
        PRODUCTION_LICENCES_FIELD,
        "production_license_agreements",
        &trade.production_license_agreements,
    )?
    .clone();
    if !stockpile_total.is_finite() || stockpile.values().any(|value| !value.is_finite()) {
        return Err(
            Xui12ProductionLineRejection::SupportingMeasuredValueInvalid {
                carrier_field: STOCKPILE_FIELD,
                field: "stockpile figure",
            },
        );
    }
    if !convoys_owned.is_finite() {
        return Err(
            Xui12ProductionLineRejection::SupportingMeasuredValueInvalid {
                carrier_field: CONVOY_VARIANTS_FIELD,
                field: "convoys_owned",
            },
        );
    }
    Ok(Some(AdmittedXui12ProductionLines {
        selected_country: selected_country.to_string(),
        scroll_offset_px: viewer.scroll_offset_px,
        military_lines,
        naval_lines,
        stockpile_total,
        stockpile,
        convoys_owned_raw,
        convoys_owned,
        convoys_assigned,
        convoys_available,
        convoys_cache_valid,
        convoy_variants,
        convoy_priority_reservations,
        deployment_queue,
        production_license_agreements,
    }))
}

pub fn checked_variant_display_name<'a>(
    equipment_variant_id: &str,
    records: &'a [DesignRecord],
) -> Result<&'a str, CheckedVariantNameError> {
    let numeric_id = equipment_variant_id
        .parse::<u32>()
        .ok()
        .filter(|id| *id != 0)
        .ok_or(CheckedVariantNameError::InvalidProductionVariantId)?;
    if numeric_id.to_string() != equipment_variant_id {
        return Err(CheckedVariantNameError::InvalidProductionVariantId);
    }

    let mut matches = records.iter().filter(|record| {
        if record.kind != DesignKind::EquipmentVariant {
            return false;
        }
        let Some((id, runtime_type)) = record.design_id.rsplit_once(':') else {
            return false;
        };
        id.parse::<u32>().ok() == Some(numeric_id) && runtime_type.parse::<u32>().is_ok()
    });
    let record = matches
        .next()
        .ok_or(CheckedVariantNameError::NoMatchingDesign)?;
    if matches.next().is_some() {
        return Err(CheckedVariantNameError::AmbiguousDesign);
    }
    if record.definition_completeness != FieldCompleteness::Complete {
        return Err(CheckedVariantNameError::IncompleteDefinition);
    }
    record
        .name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .ok_or(CheckedVariantNameError::MissingDisplayName)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckedVariantNameError {
    InvalidProductionVariantId,
    NoMatchingDesign,
    AmbiguousDesign,
    IncompleteDefinition,
    MissingDisplayName,
}

fn require_complete(
    carrier_field: ProductionCarrierField,
    leaf: &Leaf<bool>,
) -> Result<(), Xui12ProductionLineRejection> {
    match leaf {
        Leaf::Measured(true) => Ok(()),
        Leaf::Measured(false) => {
            Err(Xui12ProductionLineRejection::CompletenessMeasuredFalse { carrier_field })
        }
        Leaf::Absent => Err(Xui12ProductionLineRejection::CompletenessNotMeasuredTrue {
            carrier_field,
            state: MissingLeafState::Absent,
        }),
        Leaf::Unobtained => Err(Xui12ProductionLineRejection::CompletenessNotMeasuredTrue {
            carrier_field,
            state: MissingLeafState::Unobtained,
        }),
    }
}

fn require_support_complete(
    carrier_field: ProductionCarrierField,
    leaf: &Leaf<bool>,
) -> Result<(), Xui12ProductionLineRejection> {
    match leaf {
        Leaf::Measured(true) => Ok(()),
        Leaf::Measured(false) => {
            Err(Xui12ProductionLineRejection::SupportingCompletenessMeasuredFalse { carrier_field })
        }
        Leaf::Absent => Err(
            Xui12ProductionLineRejection::SupportingCompletenessNotMeasuredTrue {
                carrier_field,
                state: MissingLeafState::Absent,
            },
        ),
        Leaf::Unobtained => Err(
            Xui12ProductionLineRejection::SupportingCompletenessNotMeasuredTrue {
                carrier_field,
                state: MissingLeafState::Unobtained,
            },
        ),
    }
}

fn support_required<'a, T>(
    carrier_field: ProductionCarrierField,
    field: &'static str,
    leaf: &'a Leaf<T>,
) -> Result<&'a T, Xui12ProductionLineRejection> {
    match leaf {
        Leaf::Measured(value) => Ok(value),
        Leaf::Absent => Err(Xui12ProductionLineRejection::SupportingLeafNotMeasured {
            carrier_field,
            field,
            state: MissingLeafState::Absent,
        }),
        Leaf::Unobtained => Err(Xui12ProductionLineRejection::SupportingLeafNotMeasured {
            carrier_field,
            field,
            state: MissingLeafState::Unobtained,
        }),
    }
}

fn admit_collection(
    carrier_field: ProductionCarrierField,
    rows: &[EquipmentProductionLineRow],
    records: &[DesignRecord],
    viewer: &Xui12ViewerState,
) -> Result<Vec<AdmittedProductionLine>, Xui12ProductionLineRejection> {
    rows.iter()
        .enumerate()
        .map(|(row, source)| {
            let equipment_variant_id = required(
                carrier_field,
                row,
                "equipment_variant_id",
                &source.equipment_variant_id,
            )?
            .clone();
            let equipment_display_name =
                checked_variant_display_name(&equipment_variant_id, records)
                    .map_err(|error| {
                        map_join_error(carrier_field, row, &equipment_variant_id, error)
                    })?
                    .to_string();
            let efficiency = *required(carrier_field, row, "efficiency", &source.efficiency)?;
            let efficiency_cap =
                *required(carrier_field, row, "efficiency_cap", &source.efficiency_cap)?;
            let efficiency_growth = *required(
                carrier_field,
                row,
                "efficiency_growth",
                &source.efficiency_growth,
            )?;
            let cost = *required(carrier_field, row, "cost", &source.cost)?;
            let speed = *required(carrier_field, row, "speed", &source.speed)?;
            let produced = *required(carrier_field, row, "produced", &source.produced)?;
            let progress_ratio =
                *required(carrier_field, row, "progress_ratio", &source.progress_ratio)?;
            let daily_output = *required(carrier_field, row, "daily_output", &source.daily_output)?;
            let days_to_next_unit = *required(
                carrier_field,
                row,
                "days_to_next_unit",
                &source.days_to_next_unit,
            )?;
            if ![
                efficiency,
                efficiency_cap,
                efficiency_growth,
                cost,
                speed,
                produced,
                progress_ratio,
                daily_output,
                days_to_next_unit,
            ]
            .into_iter()
            .all(f64::is_finite)
            {
                return Err(Xui12ProductionLineRejection::InvalidMeasuredValue {
                    carrier_field,
                    row,
                    field: "floating production figure",
                });
            }
            let id = *required(carrier_field, row, "id", &source.id)?;
            let kind = if carrier_field == MILITARY_LINES_FIELD {
                Xui12ProductionKind::Military
            } else {
                debug_assert_eq!(carrier_field, NAVAL_LINES_FIELD);
                Xui12ProductionKind::Naval
            };
            let row_identity = Xui12RowIdentity::ExistingLine { kind, line_id: id };
            Ok(AdmittedProductionLine {
                source_ordinal: row,
                row_identity: row_identity.clone(),
                carrier_field,
                presentation: viewer
                    .row_presentation
                    .get(&row_identity)
                    .copied()
                    .unwrap_or(ViewerRowPresentation::Expanded),
                id,
                priority: *required(carrier_field, row, "priority", &source.priority)?,
                line_subtype: required(carrier_field, row, "line_subtype", &source.line_subtype)?
                    .clone(),
                equipment_variant_id,
                equipment_display_name,
                active_factories: *required(
                    carrier_field,
                    row,
                    "active_factories",
                    &source.active_factories,
                )?,
                requested_factories: *required(
                    carrier_field,
                    row,
                    "requested_factories",
                    &source.requested_factories,
                )?,
                efficiency,
                efficiency_cap,
                efficiency_growth,
                cost,
                speed,
                produced,
                amount: *required(carrier_field, row, "amount", &source.amount)?,
                progress_ratio,
                daily_output,
                days_to_next_unit,
                mio_id: PreservedLeaf::from(&source.mio_id),
            })
        })
        .collect()
}

fn required<'a, T>(
    carrier_field: ProductionCarrierField,
    row: usize,
    field: &'static str,
    leaf: &'a Leaf<T>,
) -> Result<&'a T, Xui12ProductionLineRejection> {
    match leaf {
        Leaf::Measured(value) => Ok(value),
        Leaf::Absent => Err(Xui12ProductionLineRejection::RequiredLeafNotMeasured {
            carrier_field,
            row,
            field,
            state: MissingLeafState::Absent,
        }),
        Leaf::Unobtained => Err(Xui12ProductionLineRejection::RequiredLeafNotMeasured {
            carrier_field,
            row,
            field,
            state: MissingLeafState::Unobtained,
        }),
    }
}

fn map_join_error(
    carrier_field: ProductionCarrierField,
    row: usize,
    variant_id: &str,
    error: CheckedVariantNameError,
) -> Xui12ProductionLineRejection {
    match error {
        CheckedVariantNameError::InvalidProductionVariantId => {
            Xui12ProductionLineRejection::VariantIdentityInvalid { carrier_field, row }
        }
        CheckedVariantNameError::NoMatchingDesign => {
            Xui12ProductionLineRejection::VariantJoinMissing {
                carrier_field,
                row,
                variant_id: variant_id.to_string(),
            }
        }
        CheckedVariantNameError::AmbiguousDesign => {
            Xui12ProductionLineRejection::VariantJoinAmbiguous {
                carrier_field,
                row,
                variant_id: variant_id.to_string(),
            }
        }
        CheckedVariantNameError::IncompleteDefinition => {
            Xui12ProductionLineRejection::VariantDefinitionIncomplete {
                carrier_field,
                row,
                variant_id: variant_id.to_string(),
            }
        }
        CheckedVariantNameError::MissingDisplayName => {
            Xui12ProductionLineRejection::VariantDisplayNameMissing {
                carrier_field,
                row,
                variant_id: variant_id.to_string(),
            }
        }
    }
}

fn require_admitted(
    admission: &ValidatedContentAdmission,
    path: &str,
) -> Result<(), Xui12ProductionLineRejection> {
    let path = normalize_rel_path(path);
    admission
        .require_used_content(&path)
        .map(|_| ())
        .map_err(|_| Xui12ProductionLineRejection::AuthoredSourceNotAdmitted { path })
}

fn collect_authored_refs(
    element: &GuiElement,
    sprites: &mut BTreeSet<String>,
    fonts: &mut BTreeSet<String>,
    loc_keys: &mut BTreeSet<String>,
) {
    sprites.extend(element.sprite_refs.iter().cloned());
    if let Some(font) = element
        .font
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        fonts.insert(font.clone());
    }
    if let Some(text) = element
        .text
        .as_ref()
        .filter(|text| is_localisation_key(text))
    {
        loc_keys.insert(text.clone());
    }
    for child in &element.children {
        collect_authored_refs(child, sprites, fonts, loc_keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
    use serde_json::Map;

    struct Xui12FrameSource {
        hours: [i64; 1],
        rest: Map<String, serde_json::Value>,
    }

    impl ReplaySource for Xui12FrameSource {
        fn game_id(&self) -> &str {
            "xui12-producer-authority"
        }

        fn hours(&self) -> &[i64] {
            &self.hours
        }

        fn warnings(&self) -> &[String] {
            &[]
        }

        fn frame(&self, hour_index: i64) -> std::result::Result<RawFrame, SourceError> {
            Ok(RawFrame {
                hour_index,
                date: None,
                save_version: None,
                schema_version: None,
                rest: {
                    let mut rest = self.rest.clone();
                    if replay_core::COUNTRY_COMPOSED_MEMBERS
                        .iter()
                        .any(|member| rest.contains_key(*member))
                    {
                        rest.entry("tag_db_id_index").or_insert_with(
                            || serde_json::json!({"country_composition_version": 2}),
                        );
                    }
                    rest.into()
                },
            })
        }
    }

    fn decoded_xui12_frame(
        rest: Map<String, serde_json::Value>,
    ) -> std::sync::Arc<replay_core::WorldFrame> {
        WorldTimeline::open(Box::new(Xui12FrameSource { hours: [1], rest }))
            .unwrap()
            .state_at(1)
            .unwrap()
    }

    fn authored_definition(name: String) -> GuiElement {
        GuiElement {
            element_type: "containerWindowType".to_string(),
            name,
            position: None,
            size: None,
            orientation: None,
            sprite_refs: Vec::new(),
            font: None,
            text: None,
            format: None,
            children: Vec::new(),
            raw: crate::paradox_text::Value::Object(Vec::new()),
        }
    }

    fn admitted_line(
        subtype: String,
        presentation: ViewerRowPresentation,
    ) -> AdmittedProductionLine {
        AdmittedProductionLine {
            source_ordinal: 7,
            row_identity: Xui12RowIdentity::ExistingLine {
                kind: Xui12ProductionKind::Military,
                line_id: 77,
            },
            carrier_field: MILITARY_LINES_FIELD,
            presentation,
            id: 1,
            priority: 2,
            line_subtype: subtype,
            equipment_variant_id: String::new(),
            equipment_display_name: String::new(),
            active_factories: 4,
            requested_factories: 5,
            efficiency: 0.6,
            efficiency_cap: 0.7,
            efficiency_growth: 0.01,
            cost: 8.0,
            speed: 9.0,
            produced: 10.0,
            amount: 11,
            progress_ratio: 0.12,
            daily_output: 13.0,
            days_to_next_unit: 14.0,
            mio_id: PreservedLeaf::Absent,
        }
    }

    #[test]
    fn row_template_is_discovered_from_open_subtype_and_viewer_presentation() {
        let subtype = ["fixture", "role"].join("_");
        let expanded = ["production", &subtype, "line", "entry"].join("_");
        let collapsed = format!("{expanded}_collapsed");
        let authored_root = AuthenticatedXui12Root {
            source_file: "interface/fixture.gui".to_string(),
            root: authored_definition(COUNTRY_PRODUCTION_LINE_ROOT.to_string()),
            authored_definitions: vec![
                authored_definition(expanded.clone()),
                authored_definition(collapsed.clone()),
            ],
            admitted_dependencies: BTreeSet::new(),
        };

        let selected = discover_xui12_row_template(
            &authored_root,
            &admitted_line(subtype.clone(), ViewerRowPresentation::Collapsed),
        )
        .expect("the authored collapsed definition is unique");
        assert_eq!(selected.authored.name, collapsed);
        assert_eq!(selected.line_subtype, subtype);
        assert_eq!(selected.source_ordinal, 7);

        let mut ambiguous = authored_root;
        ambiguous
            .authored_definitions
            .push(authored_definition(selected.authored.name.clone()));
        assert_eq!(
            discover_xui12_row_template(
                &ambiguous,
                &admitted_line(
                    selected.line_subtype.clone(),
                    ViewerRowPresentation::Collapsed
                ),
            ),
            Err(
                Xui12ProductionLineRejection::AuthoredRowTemplateCandidateCount {
                    line_subtype: selected.line_subtype,
                    presentation: ViewerRowPresentation::Collapsed,
                    count: 2,
                }
            )
        );
    }

    #[test]
    fn semantic_root_resolution_requires_exactly_one_effective_vfs_candidate() {
        assert_eq!(
            validate_semantic_root_sources(&[]),
            Err(Xui12ProductionLineRejection::SemanticRootCandidateCount { count: 0 })
        );
        assert_eq!(
            validate_semantic_root_sources(&[
                "interface/first.gui".to_string(),
                "interface/modded_duplicate.gui".to_string(),
            ]),
            Err(Xui12ProductionLineRejection::SemanticRootCandidateCount { count: 2 })
        );
        assert_eq!(
            validate_semantic_root_sources(&["interface/modded_replacement.gui".to_string()]),
            Ok(())
        );
    }

    #[test]
    fn legacy_partial_view_gap_set_names_only_missing_producer_semantics() {
        assert_eq!(
            XUI12_LEGACY_PARTIAL_VIEW_GAPS,
            [
                Xui12PixelCarrierGap::GlobalSourceOrdinal,
                Xui12PixelCarrierGap::OptionalNewLineIdentityVisibilityAndOrder,
                Xui12PixelCarrierGap::ExplicitMioAssignment,
                Xui12PixelCarrierGap::PerLineUiState,
            ]
        );
        assert_eq!(
            XUI12_LEGACY_PARTIAL_VIEW_GAPS
                .iter()
                .map(|gap| gap.field())
                .collect::<Vec<_>>(),
            [
                "global_source_ordinal",
                "optional_new_line_identity_visibility_and_order",
                "explicit_mio_assignment",
                "per_line_ui_state",
            ]
        );
        assert!(XUI12_LEGACY_PARTIAL_VIEW_GAPS
            .iter()
            .all(|gap| !gap.reason().trim().is_empty()));
    }

    #[test]
    fn variable_height_plan_requires_dense_global_order_and_applies_viewer_scroll() {
        let grid = crate::widget_layout::GridBoxLayout {
            rect: crate::compose::Rect::new(10, 20, 500, 400),
            format: crate::widget_layout::GridBoxFormat::UpperLeft,
            slot_size: (1, 1),
            max_slots_x: None,
            max_slots_y: None,
            add_horizontal: None,
        };
        let rows = [
            AuthenticatedXui12CanvasRow {
                global_source_ordinal: 0,
                resolved_width_px: 300,
                resolved_height_px: 30,
            },
            AuthenticatedXui12CanvasRow {
                global_source_ordinal: 1,
                resolved_width_px: 320,
                resolved_height_px: 42,
            },
        ];
        assert_eq!(
            plan_xui12_canvas_row_rects(&grid, &rows, 7).unwrap(),
            [
                AuthenticatedXui12CanvasRowRect {
                    global_source_ordinal: 0,
                    rect: crate::compose::Rect::new(10, 13, 300, 30),
                },
                AuthenticatedXui12CanvasRowRect {
                    global_source_ordinal: 1,
                    rect: crate::compose::Rect::new(10, 43, 320, 42),
                },
            ]
        );

        let torn = [
            rows[0].clone(),
            AuthenticatedXui12CanvasRow {
                global_source_ordinal: 2,
                ..rows[1].clone()
            },
        ];
        assert_eq!(
            plan_xui12_canvas_row_rects(&grid, &torn, 0),
            Err(Xui12CanvasLayoutRejection::NonDenseGlobalSourceOrdinal {
                expected: 1,
                actual: 2,
            })
        );
    }

    #[test]
    fn viewer_scroll_is_bounded_by_authenticated_extent() {
        let mut viewer = Xui12ViewerState::default();
        viewer.apply_bounded_scroll_delta(35, 100);
        assert_eq!(viewer.scroll_offset_px, 35);
        viewer.apply_bounded_scroll_delta(200, 100);
        assert_eq!(viewer.scroll_offset_px, 100);
        viewer.apply_bounded_scroll_delta(-500, 100);
        assert_eq!(viewer.scroll_offset_px, 0);
        viewer.panel_open = true;
        viewer.scroll_offset_px = 77;
        viewer.set_row_presentation(
            Xui12RowIdentity::ExistingLine {
                kind: Xui12ProductionKind::Military,
                line_id: 17,
            },
            ViewerRowPresentation::Collapsed,
        );
        viewer.reset_for_seek_or_country_change();
        assert!(viewer.panel_open);
        assert_eq!(viewer.scroll_offset_px, 0);
        assert!(viewer.row_presentation.is_empty());
    }

    fn xui12_country(tag_db_id: u32) -> serde_json::Value {
        serde_json::json!({"_r2_tag_db_id": tag_db_id})
    }

    fn completeness_authority(
        payload: &Xui12ProducerFramePayload,
    ) -> Xui12ProducerCompletenessAuthority {
        Xui12ProducerCompletenessAuthority {
            countries: payload
                .countries
                .iter()
                .map(|country| Xui12ProducerCountryCompletenessAuthority {
                    country_tag: country.country_tag.clone(),
                    tag_db_id: country.tag_db_id,
                    source_ordinal: country.source_ordinal,
                    row_identities: country
                        .rows
                        .iter()
                        .map(|row| row.row_identity.clone())
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn frame_resident_carrier_requires_both_halves_and_complete_d18_census() {
        let mut base_rest = Map::new();
        base_rest.insert(
            "countries".into(),
            serde_json::json!({"FRA": xui12_country(2), "GER": xui12_country(1)}),
        );
        let base = decoded_xui12_frame(base_rest.clone());
        assert_eq!(xui12_producer_payload_from_frame(base.as_ref()), Ok(None));
        let payload = Xui12ProducerFramePayload {
            executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
            producer_manifest_sha256: "cd".repeat(32),
            replay_frame_identity_sha256: xui12_replay_frame_identity_sha256(base.as_ref())
                .unwrap(),
            completeness_record_sha256: "ef".repeat(32),
            countries: Vec::new(),
        };
        let mut payload_only = base_rest;
        payload_only.insert(
            XUI12_PRODUCER_PAYLOAD_FIELD.into(),
            serde_json::to_value(&payload).unwrap(),
        );
        assert_eq!(
            xui12_producer_payload_from_frame(decoded_xui12_frame(payload_only).as_ref()),
            Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
        );
    }

    #[test]
    fn strict_all_country_canvas_seal_binds_frame_viewer_and_detached_census() {
        let identity = Xui12RowIdentity::ExistingLine {
            kind: Xui12ProductionKind::Military,
            line_id: 77,
        };
        let mut viewer = Xui12ViewerState::default();
        viewer.panel_open = true;
        viewer.scroll_offset_px = 19;
        viewer.set_row_presentation(identity.clone(), ViewerRowPresentation::Collapsed);
        let mut base_rest = Map::new();
        base_rest.insert(
            "countries".into(),
            serde_json::json!({"FRA": xui12_country(2), "GER": xui12_country(1)}),
        );
        let base = decoded_xui12_frame(base_rest.clone());
        let frame_identity = xui12_replay_frame_identity_sha256(base.as_ref()).unwrap();
        let semantics = BTreeMap::from([(
            "row".into(),
            crate::window_manager_law::ReplayAuthenticatedElementSemantic::Structural,
        )]);
        let owner_semantics = BTreeMap::from([(
            "owner".into(),
            crate::window_manager_law::ReplayAuthenticatedElementSemantic::Structural,
        )]);
        let mut payload = Xui12ProducerFramePayload {
            executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
            producer_manifest_sha256: "cd".repeat(32),
            replay_frame_identity_sha256: frame_identity.clone(),
            completeness_record_sha256: String::new(),
            countries: vec![
                Xui12ProducerCanvasPayload {
                    country_tag: "FRA".into(),
                    tag_db_id: 2,
                    source_ordinal: 0,
                    measured_empty: true,
                    owner_semantics: owner_semantics.clone(),
                    rows: Vec::new(),
                },
                Xui12ProducerCanvasPayload {
                    country_tag: "GER".into(),
                    tag_db_id: 1,
                    source_ordinal: 1,
                    measured_empty: false,
                    owner_semantics,
                    rows: vec![Xui12ProducerCanvasRow {
                        row_identity: identity,
                        global_source_ordinal: 0,
                        presentations: BTreeMap::from([
                            (
                                ViewerRowPresentation::Expanded,
                                Xui12ProducerRowPresentation {
                                    template_name: "production_military_line_entry".into(),
                                    resolved_width_px: 300,
                                    resolved_height_px: 80,
                                    semantics: semantics.clone(),
                                },
                            ),
                            (
                                ViewerRowPresentation::Collapsed,
                                Xui12ProducerRowPresentation {
                                    template_name: "production_military_line_entry_collapsed"
                                        .into(),
                                    resolved_width_px: 300,
                                    resolved_height_px: 40,
                                    semantics,
                                },
                            ),
                        ]),
                    }],
                },
            ],
        };
        payload.completeness_record_sha256 =
            xui12_completeness_record_sha256(&completeness_authority(&payload));
        assert_eq!(
            AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
                payload.clone(),
                base.as_ref(),
                &"cd".repeat(32),
            ),
            Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
        );
        let authority = completeness_authority(&payload);
        let mut rest = base_rest.clone();
        rest.insert(
            XUI12_PRODUCER_PAYLOAD_FIELD.into(),
            serde_json::to_value(&payload).unwrap(),
        );
        rest.insert(
            XUI12_PRODUCER_AUTHORITY_FIELD.into(),
            payload.replay_authority_record(1, &authority),
        );
        let frame = decoded_xui12_frame(rest);

        let mut wrong_manifest = payload.clone();
        wrong_manifest.producer_manifest_sha256 = "12".repeat(32);
        assert_eq!(
            AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
                wrong_manifest,
                frame.as_ref(),
                &"cd".repeat(32),
            ),
            Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
        );
        let mut truncated = payload.clone();
        truncated.countries.remove(0);
        assert_eq!(
            AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
                truncated.clone(),
                frame.as_ref(),
                &"cd".repeat(32),
            ),
            Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
        );
        let truncated_authority = completeness_authority(&truncated);
        truncated.completeness_record_sha256 =
            xui12_completeness_record_sha256(&truncated_authority);
        let mut resealed_truncated_rest = base_rest;
        resealed_truncated_rest.insert(
            XUI12_PRODUCER_PAYLOAD_FIELD.into(),
            serde_json::to_value(&truncated).unwrap(),
        );
        resealed_truncated_rest.insert(
            XUI12_PRODUCER_AUTHORITY_FIELD.into(),
            truncated.replay_authority_record(1, &truncated_authority),
        );
        assert_eq!(
            AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
                truncated,
                decoded_xui12_frame(resealed_truncated_rest).as_ref(),
                &"cd".repeat(32),
            ),
            Err(Xui12CanvasLayoutRejection::IncompleteCountryCensus)
        );

        let mut wrong_authority = payload.replay_authority_record(1, &authority);
        wrong_authority["payload_receipt_sha256"] = serde_json::Value::String("56".repeat(32));
        let mut wrong_authority_rest = Map::new();
        wrong_authority_rest.insert(XUI12_PRODUCER_AUTHORITY_FIELD.into(), wrong_authority);
        assert_eq!(
            AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
                payload.clone(),
                decoded_xui12_frame(wrong_authority_rest).as_ref(),
                &"cd".repeat(32),
            ),
            Err(Xui12CanvasLayoutRejection::InvalidProducerAuthority)
        );

        let producer = AuthenticatedXui12ProducerCanvas::authenticate_from_frame(
            payload,
            frame.as_ref(),
            &"cd".repeat(32),
        )
        .unwrap();
        let input = producer
            .seal_for_frame_and_viewer(frame.as_ref(), "GER", &viewer)
            .unwrap();
        assert_eq!(
            input.rows[0].template_name,
            "production_military_line_entry_collapsed"
        );

        assert_eq!(
            input.validate_for_replay_frame_identity_and_viewer(&frame_identity, "GER", &viewer),
            Ok(())
        );
        assert_eq!(
            input.validate_for_frame_and_viewer(frame.as_ref(), "GER", &viewer),
            Ok(())
        );
        assert_eq!(
            input.validate_for_frame_and_viewer(base.as_ref(), "GER", &viewer),
            Ok(()),
            "the base frame and the carrier-bearing frame share one base-frame identity"
        );
        assert!(producer
            .seal_for_frame_and_viewer(frame.as_ref(), "FRA", &Xui12ViewerState::default())
            .unwrap()
            .rows
            .is_empty());
        assert_eq!(
            input.validate_for_replay_frame_identity_and_viewer(&"cd".repeat(32), "GER", &viewer),
            Err(Xui12CanvasLayoutRejection::StaleReplayFrame)
        );
        let mut changed_viewer = viewer.clone();
        changed_viewer.scroll_offset_px += 1;
        assert_eq!(
            input.validate_for_replay_frame_identity_and_viewer(
                &frame_identity,
                "GER",
                &changed_viewer
            ),
            Err(Xui12CanvasLayoutRejection::StaleViewerState)
        );

        let mut tampered = input;
        tampered.rows[0].template_name.push_str("_tampered");
        assert_eq!(
            tampered.validate_shape(),
            Err(Xui12CanvasLayoutRejection::IncompleteTypedAdapter)
        );
    }
}

