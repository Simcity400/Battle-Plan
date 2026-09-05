
use crate::assets::AssetResolver;
use crate::compose::{
    draw_nine_slice, draw_scaled, ComposeOptions, ComposeOutput, ComposeReport, Compositor, Rect,
};
use crate::dds::RgbaImage;
use crate::font::alpha_blend;
use crate::gfx::SpriteRegistry;
use crate::loc::Localisation;
use crate::observer_collection::{
    compose_observer_row_layers, plan_observer_collection, AuthoredCollectionSource,
    AuthoredRowBinding, ObserverRowComposeInput, ObserverSemanticCollection, ObserverSemanticRow,
    StandardGridAuthority, ViewerCollectionState,
};
use crate::trade_layout::{CountryTradeViewerState, TradeAuthoredLayout};
use crate::trade_state::{
    apply_trade_to_compose_options, format_needed, format_trade_amount, TradeData,
    TradePresentation, TRADE_RESOURCE_ORDER,
};
use crate::{GameUiError, Result};
use replay_core::NamedCountry;
use replay_core::{Leaf, ViewStatus, WorldFrame};
use snapshot::sha256::Sha256;
use std::collections::BTreeSet;

const OWNER: &str = "interface/countrytradeview.gui#countrytradeview";
pub const XUI15_PRODUCER_AUTHORITY_FIELD: &str = "_r2_xui15_trade_catalogue_authority";
pub const XUI15_PRODUCER_PAYLOAD_FIELD: &str = "_r2_xui15_trade_catalogue_payload";
pub const XUI15_PRODUCER_AUTHORITY_SCHEMA: &str =
    "replay.xui15.country_trade_catalogue_authority.a729.v2";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTradeCatalogueRow {
    pub stable_country_id: String,
    pub display_ordinal: u32,
    pub country_name: String,
    pub available_export_text: String,
    pub exported_text: String,
    pub convoys_text: String,
    pub flag: RgbaImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverTradeCountryFilterRow {
    pub token_id: u32,
    pub source_ordinal: u32,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerCountryFilter {
    pub token_id: u32,
    pub source_ordinal: u32,
    pub localisation_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerCountryResource {
    pub resource_index: u8,
    pub available_export: i32,
    pub exported: i32,
    pub convoys: i32,
    pub engine_sort_ordinals: [u32; 3],
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerCountryRow {
    pub country_tag: String,
    pub source_ordinal: u32,
    pub filter_token_ids: Vec<u32>,
    pub resources: Vec<Xui15ProducerCountryResource>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerSelectedCountryCatalogue {
    pub country_tag: String,
    pub source_ordinal: u32,
    pub evaluated_country_tags: Vec<String>,
    pub eligible_country_tags: Vec<String>,
    pub countries: Vec<Xui15ProducerCountryRow>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerImporterCompletenessAuthority {
    pub country_tag: String,
    pub source_ordinal: u32,
    pub evaluated_country_tags: Vec<String>,
    pub eligible_country_tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerCompletenessAuthority {
    pub evaluated_filter_token_ids: Vec<u32>,
    pub catalogues: Vec<Xui15ProducerImporterCompletenessAuthority>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Xui15ProducerPayload {
    pub executable_sha256: String,
    pub producer_manifest_sha256: String,
    pub replay_frame_identity_sha256: String,
    pub completeness_record_sha256: String,
    pub evaluated_filter_token_ids: Vec<u32>,
    pub country_filters: Vec<Xui15ProducerCountryFilter>,
    pub catalogues: Vec<Xui15ProducerSelectedCountryCatalogue>,
}

impl Xui15ProducerPayload {
    pub fn receipt_sha256(&self) -> String {
        xui15_producer_receipt_sha256(self)
    }

    pub fn replay_authority_record(
        &self,
        frame_hour: i64,
        completeness_authority: &Xui15ProducerCompletenessAuthority,
    ) -> serde_json::Value {
        serde_json::json!({
            "schema": XUI15_PRODUCER_AUTHORITY_SCHEMA,
            "frame_hour": frame_hour,
            "executable_sha256": self.executable_sha256,
            "producer_manifest_sha256": self.producer_manifest_sha256,
            "replay_frame_identity_sha256": self.replay_frame_identity_sha256,
            "completeness_record_sha256": xui15_completeness_record_sha256(completeness_authority),
            "completeness_authority": completeness_authority,
            "payload_receipt_sha256": self.receipt_sha256(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Xui15ProducerRejection {
    InvalidAuthority,
    StaleFrame,
    StaleViewer,
    IncompleteSelectedCountryTrade,
    IncompleteFilterCatalogue,
    IncompleteCountryCatalogue,
    /// The D18 country roster is not COMPLETE, naming the countries the
    /// recording could not name. Distinct from the two `Incomplete*` variants
    /// above, which are the producer's payload disagreeing with a complete
    /// roster: a recorder gap and a carrier defect are different findings.
    IncompleteD18Roster(String),
    MissingEffectiveLocalisation,
    MissingEffectiveFlag,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct DecodedXui15ProducerAuthority {
    schema: String,
    frame_hour: i64,
    executable_sha256: String,
    producer_manifest_sha256: String,
    replay_frame_identity_sha256: String,
    completeness_record_sha256: String,
    completeness_authority: Xui15ProducerCompletenessAuthority,
    payload_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AuthenticatedXui15Producer {
    payload: Xui15ProducerPayload,
    payload_receipt_sha256: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StrictXui15PanelInput {
    country_tag: String,
    completeness_record_sha256: String,
    presentation: TradePresentation,
    catalogue_rows: Vec<ObserverTradeCatalogueRow>,
    country_filters: Vec<ObserverTradeCountryFilterRow>,
    authority: StrictXui15PanelAuthority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StrictXui15PanelAuthority {
    replay_frame_identity_sha256: String,
    viewer_state_identity_sha256: String,
    producer_manifest_sha256: String,
    payload_receipt_sha256: String,
}

impl StrictXui15PanelInput {
    pub fn country_tag(&self) -> &str {
        &self.country_tag
    }

    pub fn completeness_record_sha256(&self) -> &str {
        &self.completeness_record_sha256
    }

    pub fn catalogue_row_count(&self) -> usize {
        self.catalogue_rows.len()
    }

    pub fn country_filter_count(&self) -> usize {
        self.country_filters.len()
    }

    pub(crate) fn presentation(&self) -> &TradePresentation {
        &self.presentation
    }

    pub(crate) fn catalogue_rows(&self) -> &[ObserverTradeCatalogueRow] {
        &self.catalogue_rows
    }

    pub(crate) fn country_filters(&self) -> &[ObserverTradeCountryFilterRow] {
        &self.country_filters
    }

    pub fn validate_for_frame_and_viewer(
        &self,
        frame: &WorldFrame,
        viewer: &CountryTradeViewerState,
    ) -> std::result::Result<(), Xui15ProducerRejection> {
        if self.authority.replay_frame_identity_sha256
            != xui15_replay_frame_identity_sha256(frame)?
        {
            return Err(Xui15ProducerRejection::StaleFrame);
        }
        if self.authority.viewer_state_identity_sha256 != xui15_viewer_identity_sha256(viewer) {
            return Err(Xui15ProducerRejection::StaleViewer);
        }
        if self.country_tag.trim().is_empty()
            || !crate::is_lowercase_sha256(&self.completeness_record_sha256)
            || !crate::is_lowercase_sha256(&self.authority.producer_manifest_sha256)
            || !crate::is_lowercase_sha256(&self.authority.payload_receipt_sha256)
        {
            return Err(Xui15ProducerRejection::InvalidAuthority);
        }
        Ok(())
    }

    pub fn measured_empty(&self) -> bool {
        self.catalogue_rows.is_empty()
    }

    pub fn validate_for_viewer(
        &self,
        viewer: &CountryTradeViewerState,
    ) -> std::result::Result<(), Xui15ProducerRejection> {
        if self.authority.viewer_state_identity_sha256 != xui15_viewer_identity_sha256(viewer) {
            return Err(Xui15ProducerRejection::StaleViewer);
        }
        Ok(())
    }
}

impl AuthenticatedXui15Producer {
    fn authenticate_from_frame(
        payload: Xui15ProducerPayload,
        frame: &WorldFrame,
        admitted_manifest_sha256: &str,
    ) -> std::result::Result<Self, Xui15ProducerRejection> {
        let authority = frame
            .domain(XUI15_PRODUCER_AUTHORITY_FIELD)
            .cloned()
            .ok_or(Xui15ProducerRejection::InvalidAuthority)
            .and_then(|value| {
                serde_json::from_value::<DecodedXui15ProducerAuthority>(value)
                    .map_err(|_| Xui15ProducerRejection::InvalidAuthority)
        })?;
        let frame_identity = xui15_replay_frame_identity_sha256(frame)?;
        let receipt = xui15_producer_receipt_sha256(&payload);
        if authority.schema != XUI15_PRODUCER_AUTHORITY_SCHEMA
            || authority.frame_hour != frame.hour_index()
            || authority.executable_sha256 != map_viewer::XMAP03_A729_EXECUTABLE_SHA256
            || authority.producer_manifest_sha256 != admitted_manifest_sha256
            || authority.replay_frame_identity_sha256 != frame_identity
            || authority.payload_receipt_sha256 != receipt
            || authority.executable_sha256 != payload.executable_sha256
            || authority.producer_manifest_sha256 != payload.producer_manifest_sha256
            || authority.replay_frame_identity_sha256 != payload.replay_frame_identity_sha256
            || !crate::is_lowercase_sha256(&payload.producer_manifest_sha256)
            || !crate::is_lowercase_sha256(&payload.replay_frame_identity_sha256)
            || !crate::is_lowercase_sha256(&payload.completeness_record_sha256)
            || payload.executable_sha256 != map_viewer::XMAP03_A729_EXECUTABLE_SHA256
        {
            return Err(Xui15ProducerRejection::InvalidAuthority);
        }
        validate_xui15_completeness_authority(&authority.completeness_authority, &payload)?;
        if authority.completeness_record_sha256 != payload.completeness_record_sha256
            || authority.completeness_record_sha256
                != xui15_completeness_record_sha256(&authority.completeness_authority)
        {
            return Err(Xui15ProducerRejection::InvalidAuthority);
        }
        validate_xui15_payload_shape(&payload)?;
        validate_complete_catalogue_roster(&payload, frame)?;
        validate_complete_trade_presentation_roster(frame)?;
        Ok(Self {
            payload,
            payload_receipt_sha256: receipt,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn seal_for_viewer(
        self,
        frame: &WorldFrame,
        resolver: &AssetResolver,
        localisation: &Localisation,
        selected_country: NamedCountry<'_>,
        presentation: &TradePresentation,
        viewer: &CountryTradeViewerState,
    ) -> std::result::Result<StrictXui15PanelInput, Xui15ProducerRejection> {
        if self.payload.replay_frame_identity_sha256
            != xui15_replay_frame_identity_sha256(frame)?
        {
            return Err(Xui15ProducerRejection::StaleFrame);
        }
        let catalogue = self
            .payload
            .catalogues
            .iter()
            .find(|catalogue| catalogue.country_tag == selected_country.tag())
            .ok_or(Xui15ProducerRejection::IncompleteCountryCatalogue)?;
        validate_complete_selected_country_trade(presentation, selected_country.tag())?;
        if viewer.selected_resource_index >= TRADE_RESOURCE_ORDER.len() {
            return Err(Xui15ProducerRejection::StaleViewer);
        }
        let filter_ids = self
            .payload
            .country_filters
            .iter()
            .map(|row| row.token_id)
            .collect::<BTreeSet<_>>();
        if viewer
            .selected_country_filter_ids
            .iter()
            .any(|id| !filter_ids.contains(id))
        {
            return Err(Xui15ProducerRejection::StaleViewer);
        }
        let country_filters = self
            .payload
            .country_filters
            .iter()
            .map(|row| {
                let display_name = localisation
                    .get(&row.localisation_key)
                    .filter(|value| !value.trim().is_empty())
                    .ok_or(Xui15ProducerRejection::MissingEffectiveLocalisation)?;
                Ok(ObserverTradeCountryFilterRow {
                    token_id: row.token_id,
                    source_ordinal: row.source_ordinal,
                    display_name: display_name.to_string(),
                })
            })
            .collect::<std::result::Result<Vec<_>, Xui15ProducerRejection>>()?;
        let mut selected_rows = Vec::new();
        for row in &catalogue.countries {
            let resource = &row.resources[viewer.selected_resource_index];
            let identity = frame
                .country_identity(&row.country_tag)
                .ok_or(Xui15ProducerRejection::IncompleteCountryCatalogue)?;
            let ideology = measured_or_absent(&identity.ruling_ideology)?;
            let cosmetic = crate::binding::admitted_cosmetic_override(&identity.cosmetic_tag)
                .map_err(|_| Xui15ProducerRejection::IncompleteCountryCatalogue)?;
            let (name_key, country_name) = crate::binding::resolve_country_name(
                localisation,
                &row.country_tag,
                ideology,
                cosmetic,
            );
            if localisation.get(&name_key).is_none() || country_name.trim().is_empty() {
                return Err(Xui15ProducerRejection::MissingEffectiveLocalisation);
            }
            let flag = resolve_xui15_small_flag(
                resolver,
                &row.country_tag,
                ideology,
                cosmetic,
            )?;
            if !viewer.selected_country_filter_ids.is_empty()
                && !row
                    .filter_token_ids
                    .iter()
                    .any(|id| viewer.selected_country_filter_ids.contains(id))
            {
                continue;
            }
            let sort_rank = match viewer.sort {
                crate::trade_layout::CountryTradeSort::Country => row.source_ordinal,
                crate::trade_layout::CountryTradeSort::AvailableExport => {
                    resource.engine_sort_ordinals[0]
                }
                crate::trade_layout::CountryTradeSort::Exported => {
                    resource.engine_sort_ordinals[1]
                }
                crate::trade_layout::CountryTradeSort::Convoys => {
                    resource.engine_sort_ordinals[2]
                }
            };
            selected_rows.push((
                sort_rank,
                row.source_ordinal,
                ObserverTradeCatalogueRow {
                    stable_country_id: row.country_tag.clone(),
                    display_ordinal: 0,
                    country_name,
                    available_export_text: resource.available_export.to_string(),
                    exported_text: resource.exported.to_string(),
                    convoys_text: resource.convoys.to_string(),
                    flag,
                },
            ));
        }
        selected_rows.sort_by_key(|(rank, source, _)| (*rank, *source));
        if viewer.descending {
            selected_rows.reverse();
        }
        let catalogue_rows = selected_rows
            .into_iter()
            .enumerate()
            .map(|(ordinal, (_, _, mut row))| {
                row.display_ordinal = u32::try_from(ordinal)
                    .map_err(|_| Xui15ProducerRejection::IncompleteCountryCatalogue)?;
                Ok(row)
            })
            .collect::<std::result::Result<Vec<_>, Xui15ProducerRejection>>()?;
        let strict = StrictXui15PanelInput {
            country_tag: selected_country.to_string(),
            completeness_record_sha256: self.payload.completeness_record_sha256,
            presentation: presentation.clone(),
            catalogue_rows,
            country_filters,
            authority: StrictXui15PanelAuthority {
                replay_frame_identity_sha256: self.payload.replay_frame_identity_sha256,
                viewer_state_identity_sha256: xui15_viewer_identity_sha256(viewer),
                producer_manifest_sha256: self.payload.producer_manifest_sha256,
                payload_receipt_sha256: self.payload_receipt_sha256,
            },
        };
        strict.validate_for_frame_and_viewer(frame, viewer)?;
        Ok(strict)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn seal_xui15_producer_panel_for_frame_and_viewer(
    payload: Xui15ProducerPayload,
    frame: &WorldFrame,
    resolver: &AssetResolver,
    localisation: &Localisation,
    content_admission: &replay_core::ValidatedContentAdmission,
    selected_country: NamedCountry<'_>,
    presentation: &TradePresentation,
    viewer: &CountryTradeViewerState,
) -> std::result::Result<StrictXui15PanelInput, Xui15ProducerRejection> {
    AuthenticatedXui15Producer::authenticate_from_frame(
        payload,
        frame,
        content_admission.manifest_digest_sha256(),
    )?
    .seal_for_viewer(
        frame,
        resolver,
        localisation,
        selected_country,
        presentation,
        viewer,
    )
}

pub fn xui15_producer_payload_from_frame(
    frame: &WorldFrame,
) -> std::result::Result<Option<Xui15ProducerPayload>, Xui15ProducerRejection> {
    let payload = frame.domain(XUI15_PRODUCER_PAYLOAD_FIELD);
    let authority = frame.domain(XUI15_PRODUCER_AUTHORITY_FIELD);
    match (payload, authority) {
        (None, None) => Ok(None),
        (Some(payload), Some(_)) => serde_json::from_value(payload.clone())
            .map(Some)
            .map_err(|_| Xui15ProducerRejection::InvalidAuthority),
        _ => Err(Xui15ProducerRejection::InvalidAuthority),
    }
}

fn validate_complete_selected_country_trade(
    presentation: &TradePresentation,
    selected_country: &str,
) -> std::result::Result<(), Xui15ProducerRejection> {
    if presentation.country_tag != selected_country
        || !matches!(presentation.resource_cards, TradeData::Available(_))
        || !matches!(presentation.deals, TradeData::Available(_))
        || !matches!(presentation.factories, TradeData::Available(_))
    {
        return Err(Xui15ProducerRejection::IncompleteSelectedCountryTrade);
    }
    Ok(())
}

fn validate_xui15_completeness_authority(
    authority: &Xui15ProducerCompletenessAuthority,
    payload: &Xui15ProducerPayload,
) -> std::result::Result<(), Xui15ProducerRejection> {
    if authority.evaluated_filter_token_ids != payload.evaluated_filter_token_ids
        || authority
            .evaluated_filter_token_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != authority.evaluated_filter_token_ids.len()
    {
        return Err(Xui15ProducerRejection::IncompleteFilterCatalogue);
    }
    if authority.catalogues.len() != payload.catalogues.len() {
        return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
    }
    let mut importer_tags = BTreeSet::new();
    for (census, catalogue) in authority.catalogues.iter().zip(&payload.catalogues) {
        if census.country_tag != catalogue.country_tag
            || census.source_ordinal != catalogue.source_ordinal
            || census.evaluated_country_tags != catalogue.evaluated_country_tags
            || census.eligible_country_tags != catalogue.eligible_country_tags
            || !importer_tags.insert(census.country_tag.clone())
            || census
                .evaluated_country_tags
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .len()
                != census.evaluated_country_tags.len()
            || census
                .eligible_country_tags
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .len()
                != census.eligible_country_tags.len()
        {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
    }
    Ok(())
}

fn validate_xui15_payload_shape(
    payload: &Xui15ProducerPayload,
) -> std::result::Result<(), Xui15ProducerRejection> {
    if payload.country_filters.is_empty() || payload.catalogues.is_empty() {
        return Err(Xui15ProducerRejection::IncompleteFilterCatalogue);
    }
    let mut filter_ids = BTreeSet::new();
    for (expected, filter) in payload.country_filters.iter().enumerate() {
        if filter.source_ordinal != u32::try_from(expected).unwrap_or(u32::MAX)
            || filter.token_id == 0
            || filter.localisation_key.trim().is_empty()
            || !filter_ids.insert(filter.token_id)
        {
            return Err(Xui15ProducerRejection::IncompleteFilterCatalogue);
        }
    }
    let advertised_filter_ids = payload
        .country_filters
        .iter()
        .map(|filter| filter.token_id)
        .collect::<Vec<_>>();
    if payload.evaluated_filter_token_ids != advertised_filter_ids
        || payload
            .evaluated_filter_token_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len()
            != payload.evaluated_filter_token_ids.len()
    {
        return Err(Xui15ProducerRejection::IncompleteFilterCatalogue);
    }
    let mut selected_tags = BTreeSet::new();
    for (expected, catalogue) in payload.catalogues.iter().enumerate() {
        if catalogue.source_ordinal != u32::try_from(expected).unwrap_or(u32::MAX)
            || catalogue.country_tag.len() != 3
            || catalogue.country_tag != catalogue.country_tag.to_ascii_uppercase()
            || !selected_tags.insert(catalogue.country_tag.clone())
        {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
        validate_xui15_country_catalogue(catalogue, &filter_ids)?;
    }
    Ok(())
}

fn validate_complete_catalogue_roster(
    payload: &Xui15ProducerPayload,
    frame: &WorldFrame,
) -> std::result::Result<(), Xui15ProducerRejection> {
    // COMPLETE, not partial: the supplied catalogue set is compared for
    // EQUALITY against the roster and every catalogue's evaluated-country list
    // must equal it too. Those are absence claims over the whole country set.
    let roster = frame.d18_country_roster();
    let roster_entries = match roster.complete() {
        Ok(entries) => entries,
        Err(refusal) => return Err(Xui15ProducerRejection::IncompleteD18Roster(refusal.detail)),
    };
    let expected = roster_entries
        .iter()
        .map(|entry| entry.tag.clone())
        .collect::<BTreeSet<_>>();
    let supplied = payload
        .catalogues
        .iter()
        .map(|catalogue| catalogue.country_tag.clone())
        .collect::<BTreeSet<_>>();
    if expected.is_empty() || supplied != expected {
        return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
    }
    for catalogue in &payload.catalogues {
        let evaluated = catalogue
            .evaluated_country_tags
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        if evaluated.len() != catalogue.evaluated_country_tags.len() || evaluated != expected {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
        if catalogue
            .countries
            .iter()
            .any(|row| !evaluated.contains(&row.country_tag))
        {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
    }
    Ok(())
}

fn validate_complete_trade_presentation_roster(
    frame: &WorldFrame,
) -> std::result::Result<(), Xui15ProducerRejection> {
    // COMPLETE, not partial: this sweeps EVERY country and requires each one's
    // trade screen to be visible and complete, which is a claim about the whole
    // set rather than about the countries this hour could name.
    let roster = frame.d18_country_roster();
    let roster_entries = match roster.complete() {
        Ok(entries) => entries,
        Err(refusal) => return Err(Xui15ProducerRejection::IncompleteD18Roster(refusal.detail)),
    };
    if frame.trade_status() != ViewStatus::Present {
        return Err(Xui15ProducerRejection::IncompleteSelectedCountryTrade);
    }
    for entry in roster_entries {
        let crate::trade_state::AdmittedTradeScreen::Visible(presentation) =
            crate::trade_state::admit_trade_screen(
                frame.trade_status(),
                &entry.tag,
                frame.trade(&entry.tag),
            )
        else {
            return Err(Xui15ProducerRejection::IncompleteSelectedCountryTrade);
        };
        validate_complete_selected_country_trade(&presentation, &entry.tag)?;
    }
    Ok(())
}

fn validate_xui15_country_catalogue(
    catalogue: &Xui15ProducerSelectedCountryCatalogue,
    filter_ids: &BTreeSet<u32>,
) -> std::result::Result<(), Xui15ProducerRejection> {
    let country_count = catalogue.countries.len();
    let advertised_eligible_tags = catalogue
        .countries
        .iter()
        .map(|row| row.country_tag.clone())
        .collect::<Vec<_>>();
    if catalogue.eligible_country_tags != advertised_eligible_tags
        || catalogue
            .eligible_country_tags
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .len()
            != catalogue.eligible_country_tags.len()
    {
        return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
    }
    let mut tags = BTreeSet::new();
    let mut rank_sets = vec![[BTreeSet::new(), BTreeSet::new(), BTreeSet::new()]; 7];
    for (expected, row) in catalogue.countries.iter().enumerate() {
        if row.source_ordinal != u32::try_from(expected).unwrap_or(u32::MAX)
            || row.country_tag.len() != 3
            || row.country_tag != row.country_tag.to_ascii_uppercase()
            || !tags.insert(row.country_tag.clone())
            || row.resources.len() != TRADE_RESOURCE_ORDER.len()
        {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
        let mut memberships = row.filter_token_ids.clone();
        memberships.sort_unstable();
        memberships.dedup();
        if memberships != row.filter_token_ids
            || memberships.iter().any(|id| !filter_ids.contains(id))
        {
            return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
        }
        for (resource_index, resource) in row.resources.iter().enumerate() {
            if usize::from(resource.resource_index) != resource_index
                || resource.available_export < 0
                || resource.exported < 0
                || resource.convoys < 0
            {
                return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
            }
            for (sort, rank) in resource.engine_sort_ordinals.iter().copied().enumerate() {
                if usize::try_from(rank).ok().is_none_or(|rank| rank >= country_count)
                    || !rank_sets[resource_index][sort].insert(rank)
                {
                    return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
                }
            }
        }
    }
    if rank_sets.iter().any(|resource| {
        resource
            .iter()
            .any(|ranks| ranks.len() != country_count)
    }) {
        return Err(Xui15ProducerRejection::IncompleteCountryCatalogue);
    }
    Ok(())
}

fn measured_or_absent(
    leaf: &Leaf<String>,
) -> std::result::Result<Option<&str>, Xui15ProducerRejection> {
    match leaf {
        Leaf::Measured(value) => Ok(Some(value.as_str())),
        Leaf::Absent => Ok(None),
        Leaf::Unobtained => Err(Xui15ProducerRejection::IncompleteCountryCatalogue),
    }
}

fn resolve_xui15_small_flag(
    resolver: &AssetResolver,
    country_tag: &str,
    ideology: Option<&str>,
    cosmetic: Option<&str>,
) -> std::result::Result<RgbaImage, Xui15ProducerRejection> {
    for candidate in crate::topbar::flag_candidates_for_size(
        Some(country_tag),
        ideology,
        cosmetic,
        None,
        crate::topbar::FlagSizeFolder::Small,
    ) {
        if resolver
            .resolve_optional_file(&candidate)
            .map_err(|_| Xui15ProducerRejection::MissingEffectiveFlag)?
            .is_some()
        {
            return resolver
                .decode_texture(&candidate)
                .map(|decoded| decoded.image)
                .map_err(|_| Xui15ProducerRejection::MissingEffectiveFlag);
        }
    }
    Err(Xui15ProducerRejection::MissingEffectiveFlag)
}

fn xui15_producer_receipt_sha256(payload: &Xui15ProducerPayload) -> String {
    let mut bytes = b"hoi4.xui15.producer.v2\0".to_vec();
    for value in [
        &payload.executable_sha256,
        &payload.producer_manifest_sha256,
        &payload.replay_frame_identity_sha256,
        &payload.completeness_record_sha256,
    ] {
        digest_text(&mut bytes, value);
    }
    bytes.extend_from_slice(&(payload.evaluated_filter_token_ids.len() as u64).to_le_bytes());
    for token_id in &payload.evaluated_filter_token_ids {
        bytes.extend_from_slice(&token_id.to_le_bytes());
    }
    bytes.extend_from_slice(&(payload.country_filters.len() as u64).to_le_bytes());
    for filter in &payload.country_filters {
        bytes.extend_from_slice(&filter.token_id.to_le_bytes());
        bytes.extend_from_slice(&filter.source_ordinal.to_le_bytes());
        digest_text(&mut bytes, &filter.localisation_key);
    }
    bytes.extend_from_slice(&(payload.catalogues.len() as u64).to_le_bytes());
    for catalogue in &payload.catalogues {
        digest_text(&mut bytes, &catalogue.country_tag);
        bytes.extend_from_slice(&catalogue.source_ordinal.to_le_bytes());
        bytes.extend_from_slice(&(catalogue.evaluated_country_tags.len() as u64).to_le_bytes());
        for tag in &catalogue.evaluated_country_tags {
            digest_text(&mut bytes, tag);
        }
        bytes.extend_from_slice(&(catalogue.eligible_country_tags.len() as u64).to_le_bytes());
        for tag in &catalogue.eligible_country_tags {
            digest_text(&mut bytes, tag);
        }
        bytes.extend_from_slice(&(catalogue.countries.len() as u64).to_le_bytes());
        for country in &catalogue.countries {
            digest_text(&mut bytes, &country.country_tag);
            bytes.extend_from_slice(&country.source_ordinal.to_le_bytes());
            bytes.extend_from_slice(&(country.filter_token_ids.len() as u64).to_le_bytes());
            for token in &country.filter_token_ids {
                bytes.extend_from_slice(&token.to_le_bytes());
            }
            for resource in &country.resources {
                bytes.push(resource.resource_index);
                bytes.extend_from_slice(&resource.available_export.to_le_bytes());
                bytes.extend_from_slice(&resource.exported.to_le_bytes());
                bytes.extend_from_slice(&resource.convoys.to_le_bytes());
                for rank in resource.engine_sort_ordinals {
                    bytes.extend_from_slice(&rank.to_le_bytes());
                }
            }
        }
    }
    Sha256::hex(&bytes)
}

pub fn xui15_completeness_record_sha256(
    authority: &Xui15ProducerCompletenessAuthority,
) -> String {
    let mut bytes = b"hoi4.xui15.completeness.v2\0".to_vec();
    bytes.extend_from_slice(&(authority.evaluated_filter_token_ids.len() as u64).to_le_bytes());
    for token_id in &authority.evaluated_filter_token_ids {
        bytes.extend_from_slice(&token_id.to_le_bytes());
    }
    bytes.extend_from_slice(&(authority.catalogues.len() as u64).to_le_bytes());
    for catalogue in &authority.catalogues {
        digest_text(&mut bytes, &catalogue.country_tag);
        bytes.extend_from_slice(&catalogue.source_ordinal.to_le_bytes());
        bytes.extend_from_slice(&(catalogue.evaluated_country_tags.len() as u64).to_le_bytes());
        for tag in &catalogue.evaluated_country_tags {
            digest_text(&mut bytes, tag);
        }
        bytes.extend_from_slice(&(catalogue.eligible_country_tags.len() as u64).to_le_bytes());
        for tag in &catalogue.eligible_country_tags {
            digest_text(&mut bytes, tag);
        }
    }
    Sha256::hex(&bytes)
}

pub fn xui15_replay_frame_identity_sha256(
    frame: &WorldFrame,
) -> std::result::Result<String, Xui15ProducerRejection> {
    frame
        .base_frame_identity_sha256(snapshot::replay_frame_identity::XUI15_BASE_FRAME_IDENTITY_DOMAIN)
        .map_err(|_| Xui15ProducerRejection::InvalidAuthority)
}

fn xui15_viewer_identity_sha256(viewer: &CountryTradeViewerState) -> String {
    let mut bytes = b"hoi4.xui15.viewer.v1\0".to_vec();
    bytes.extend_from_slice(&(viewer.selected_resource_index as u64).to_le_bytes());
    bytes.push(match viewer.sort {
        crate::trade_layout::CountryTradeSort::Country => 0,
        crate::trade_layout::CountryTradeSort::AvailableExport => 1,
        crate::trade_layout::CountryTradeSort::Exported => 2,
        crate::trade_layout::CountryTradeSort::Convoys => 3,
    });
    bytes.push(u8::from(viewer.descending));
    bytes.extend_from_slice(&(viewer.selected_country_filter_ids.len() as u64).to_le_bytes());
    for token in &viewer.selected_country_filter_ids {
        bytes.extend_from_slice(&token.to_le_bytes());
    }
    for scroll in [
        viewer.catalogue_scroll,
        viewer.resource_filter_scroll,
        viewer.country_filter_scroll,
    ] {
        bytes.extend_from_slice(&scroll.displayed_fixed.to_le_bytes());
        bytes.extend_from_slice(&scroll.target_fixed.to_le_bytes());
    }
    Sha256::hex(&bytes)
}

fn digest_text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

#[derive(Clone, Copy, Debug)]
pub struct AdmittedObserverTradePanel<'a> {
    pub frame_hour: u64,
    pub country_tag: &'a str,
    pub presentation: &'a TradePresentation,
    pub catalogue_rows: &'a [ObserverTradeCatalogueRow],
    pub country_filters: &'a [ObserverTradeCountryFilterRow],
}

pub fn compose_observer_trade_panel(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    localisation: &Localisation,
    panel: AdmittedObserverTradePanel<'_>,
    viewer: &CountryTradeViewerState,
    viewport_width: usize,
    viewport_height: usize,
) -> Result<ComposeOutput> {
    validate_panel(panel, viewer)?;
    let layout = TradeAuthoredLayout::load_with_registry(
        resolver,
        registry,
        viewport_width,
        viewport_height,
    )?;
    let mut base_options =
        ComposeOptions::new(viewport_width, viewport_height).with_localisation(localisation);
    apply_trade_to_compose_options(panel.presentation, &mut base_options);
    base_options
        .text_overrides
        .retain(|name, _| !name.starts_with("xui15_"));
    base_options
        .frame_overrides
        .retain(|name, _| !name.starts_with("xui15_"));

    let mut compositor = Compositor::new(resolver, registry);
    let mut output = compositor.compose(&layout.root, &base_options);

    let resources = match &panel.presentation.resource_cards {
        TradeData::Available(rows) => rows,
        TradeData::Unavailable(issue) => {
            return Err(GameUiError::missing(format!(
                "XUI15 resource rows are unavailable: {issue:?}"
            )))
        }
    };
    let resource_semantic = semantic_rows(
        panel.frame_hour,
        panel.country_tag,
        "xui15-resources",
        resources
            .iter()
            .enumerate()
            .map(|(index, row)| (format!("resource:{}", row.resource), index, "resource")),
    )?;
    let resource_options = resources
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let mut options = ComposeOptions::new(viewport_width, viewport_height)
                .with_localisation(localisation);
            for (name, value) in [
                ("balance", format_trade_amount(row.balance)),
                ("produced", format_trade_amount(row.produced)),
                ("imported", format_trade_amount(row.imported)),
                ("exported", format_trade_amount(row.exported)),
                ("special_project", format_trade_amount(row.special_project)),
                ("production_line", format_trade_amount(row.production_line)),
                ("needed", format_needed(row)),
                ("neededType", row.needed_type.clone()),
            ] {
                options.text_overrides.insert(name.into(), value);
            }
            options
                .frame_overrides
                .insert("resources_icon".into(), index);
            let has_import = match &panel.presentation.deals {
                TradeData::Available(deals) => deals.iter().any(|deal| {
                    deal.resource == row.resource && deal.direction.eq_ignore_ascii_case("import")
                }),
                TradeData::Unavailable(_) => false,
            };
            if !has_import {
                options.hidden_elements.insert("cancel_imports".into());
            }
            options
        })
        .collect::<Vec<_>>();
    compose_collection(
        &mut compositor,
        &mut output,
        panel.frame_hour,
        &resource_semantic,
        &layout.resources,
        &layout.resource_template,
        "resources_info_entry",
        "interface/countrytradeview.gui#countrytradeview/resources/resources_grid",
        (0, 0),
        &resource_options,
        0,
    )?;

    let resource_filter_semantic = semantic_rows(
        panel.frame_hour,
        panel.country_tag,
        "xui15-resource-filters",
        TRADE_RESOURCE_ORDER
            .iter()
            .enumerate()
            .map(|(index, resource)| {
                (
                    format!("resource-filter:{resource}"),
                    index,
                    "resource-filter",
                )
            }),
    )?;
    let resource_filter_options = TRADE_RESOURCE_ORDER
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let mut options = ComposeOptions::new(viewport_width, viewport_height)
                .with_localisation(localisation);
            options.frame_overrides.insert("icon".into(), index);
            options.frame_overrides.insert(
                "background".into(),
                usize::from(viewer.selected_resource_index == index),
            );
            options
        })
        .collect::<Vec<_>>();
    compose_collection(
        &mut compositor,
        &mut output,
        panel.frame_hour,
        &resource_filter_semantic,
        &layout.resource_filters,
        &layout.resource_filter_template,
        "resources_filter_entry",
        "interface/countrytradeview.gui#countrytradeview/filters/resources_grid",
        (
            fixed_to_pixels(viewer.resource_filter_scroll.displayed_fixed)?,
            0,
        ),
        &resource_filter_options,
        1,
    )?;

    let country_filter_semantic = semantic_rows(
        panel.frame_hour,
        panel.country_tag,
        "xui15-country-filters",
        panel.country_filters.iter().map(|row| {
            (
                format!("country-filter:{}", row.token_id),
                row.source_ordinal as usize,
                "country-filter",
            )
        }),
    )?;
    let country_filter_options = panel
        .country_filters
        .iter()
        .map(|row| {
            let mut options = ComposeOptions::new(viewport_width, viewport_height)
                .with_localisation(localisation);
            options
                .text_overrides
                .insert("name".into(), row.display_name.clone());
            options.frame_overrides.insert(
                "Background".into(),
                if viewer.selected_country_filter_ids.contains(&row.token_id) {
                    1
                } else {
                    2
                },
            );
            options
        })
        .collect::<Vec<_>>();
    compose_collection(
        &mut compositor,
        &mut output,
        panel.frame_hour,
        &country_filter_semantic,
        &layout.country_filters,
        &layout.country_filter_template,
        "filter_entry",
        "interface/countrytradeview.gui#countrytradeview/country_filters/country_filters_grid",
        (
            0,
            fixed_to_pixels(viewer.country_filter_scroll.displayed_fixed)?,
        ),
        &country_filter_options,
        2,
    )?;

    let catalogue_semantic = semantic_rows(
        panel.frame_hour,
        panel.country_tag,
        "xui15-country-catalogue",
        panel.catalogue_rows.iter().map(|row| {
            (
                format!("country:{}", row.stable_country_id),
                row.display_ordinal as usize,
                "country",
            )
        }),
    )?;
    let catalogue_options = panel
        .catalogue_rows
        .iter()
        .map(|row| {
            let mut options = ComposeOptions::new(viewport_width, viewport_height)
                .with_localisation(localisation);
            for (name, value) in [
                ("country", row.country_name.clone()),
                ("available_export", row.available_export_text.clone()),
                ("exported", row.exported_text.clone()),
                ("convoys", row.convoys_text.clone()),
            ] {
                options.text_overrides.insert(name.into(), value);
            }
            options
                .sprite_overrides
                .insert("country_flag".into(), row.flag.clone());
            options
        })
        .collect::<Vec<_>>();
    compose_collection(
        &mut compositor,
        &mut output,
        panel.frame_hour,
        &catalogue_semantic,
        &layout.catalogue,
        &layout.country_row_template,
        "country_trade_entry",
        "interface/countrytradeview.gui#countrytradeview/trade/trade_grid",
        (0, fixed_to_pixels(viewer.catalogue_scroll.displayed_fixed)?),
        &catalogue_options,
        3,
    )?;
    draw_trade_scrollbar(
        resolver,
        registry,
        &mut output,
        layout.resource_filters.scrollbar.as_ref().ok_or_else(|| {
            GameUiError::missing("XUI15 resource filters lost authored scrollbar")
        })?,
        viewer.resource_filter_scroll.displayed_fixed,
        layout.resource_filter_max_scroll_fixed(TRADE_RESOURCE_ORDER.len())?,
    )?;
    draw_trade_scrollbar(
        resolver,
        registry,
        &mut output,
        layout.country_filters.scrollbar.as_ref().ok_or_else(|| {
            GameUiError::missing("XUI15 country filters lost authored scrollbar")
        })?,
        viewer.country_filter_scroll.displayed_fixed,
        layout.country_filter_max_scroll_fixed(panel.country_filters.len())?,
    )?;
    draw_trade_scrollbar(
        resolver,
        registry,
        &mut output,
        layout
            .catalogue
            .scrollbar
            .as_ref()
            .ok_or_else(|| GameUiError::missing("XUI15 catalogue lost authored scrollbar"))?,
        viewer.catalogue_scroll.displayed_fixed,
        layout.catalogue_max_scroll_fixed(panel.catalogue_rows.len())?,
    )?;
    Ok(output)
}

pub(crate) fn draw_trade_scrollbar(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    output: &mut ComposeOutput,
    geometry: &crate::trade_layout::TradeScrollbarGeometry,
    displayed_fixed: i64,
    maximum_fixed: i64,
) -> Result<()> {
    let slider_rect = geometry.slider_rect(displayed_fixed, maximum_fixed)?;
    for (part, rect) in [
        (&geometry.background, geometry.background.rect),
        (&geometry.track, geometry.track.rect),
        (&geometry.decrease, geometry.decrease.rect),
        (&geometry.increase, geometry.increase.rect),
        (&geometry.slider, slider_rect),
    ] {
        draw_trade_scrollbar_part(resolver, registry, output, part, rect)?;
    }
    Ok(())
}

fn draw_trade_scrollbar_part(
    resolver: &AssetResolver,
    registry: &SpriteRegistry,
    output: &mut ComposeOutput,
    part: &crate::trade_layout::TradeScrollbarPartGeometry,
    rect: Rect,
) -> Result<()> {
    if rect.w() <= 0 || rect.h() <= 0 {
        return Err(GameUiError::parse("XUI15 scrollbar part is not positive"));
    }
    let sprite = registry.get(&part.sprite_name).ok_or_else(|| {
        GameUiError::missing(format!("XUI15 missing scrollbar sprite {}", part.sprite_name))
    })?;
    let decoded = resolver.decode_sprite(sprite)?;
    let frame = decoded.frames.first().ok_or_else(|| {
        GameUiError::missing(format!("XUI15 scrollbar sprite {} has no frame", part.sprite_name))
    })?;
    let width = usize::try_from(rect.w())
        .map_err(|_| GameUiError::parse("XUI15 scrollbar width exceeds usize"))?;
    let height = usize::try_from(rect.h())
        .map_err(|_| GameUiError::parse("XUI15 scrollbar height exceeds usize"))?;
    if let Some((border_x, border_y)) = part.border_size {
        draw_nine_slice(
            &mut output.image,
            frame,
            rect.x(),
            rect.y(),
            width,
            height,
            border_x,
            border_y,
        );
    } else {
        draw_scaled(
            &mut output.image,
            frame,
            rect.x(),
            rect.y(),
            width,
            height,
        );
    }
    output.report.drawn_elements += 1;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compose_collection(
    compositor: &mut Compositor<'_>,
    output: &mut ComposeOutput,
    expected_hour: u64,
    semantic: &ObserverSemanticCollection,
    collection: &crate::trade_layout::TradeCollectionLayout,
    template: &crate::gui::GuiElement,
    template_name: &str,
    element_id: &str,
    scroll_pixels: (i32, i32),
    options: &[ComposeOptions],
    root_paint_ordinal: u32,
) -> Result<()> {
    let bindings = semantic
        .rows
        .iter()
        .map(|row| AuthoredRowBinding {
            stable_item_id: row.stable_item_id.as_str(),
            subtype_id: row.subtype_id.as_str(),
            element_source_identity: template_identity(template_name),
            instance_token: row.stable_item_id.as_str(),
            template,
        })
        .collect::<Vec<_>>();
    let plan_options = options
        .first()
        .cloned()
        .unwrap_or_else(|| ComposeOptions::new(output.image.width, output.image.height));
    let plan = plan_observer_collection(
        expected_hour,
        semantic,
        AuthoredCollectionSource {
            owning_window_id: OWNER,
            root_source_identity: OWNER,
            collection_element_id: element_id,
            collection_element: &collection.grid,
            effective_row_template_identity: Some(template_identity(template_name)),
            effective_row_template: Some(template),
            parent_rect: collection.container_rect,
            grid_authority: Some(StandardGridAuthority::Xui15CountryTradeStandardAutoAppend),
        },
        &bindings,
        ViewerCollectionState {
            scroll_offset_pixels: scroll_pixels,
            window_stack_ordinal: 0,
            root_paint_ordinal,
            visible_clip: collection.visible_clip,
        },
        &plan_options,
    )?;
    let inputs = semantic
        .rows
        .iter()
        .zip(options)
        .map(|(row, options)| ObserverRowComposeInput {
            stable_item_id: row.stable_item_id.as_str(),
            options,
        })
        .collect::<Vec<_>>();
    let layer = compose_observer_row_layers(compositor, &plan, &inputs)?;
    merge_output(output, layer.output)
}

fn template_identity(name: &str) -> &'static str {
    match name {
        "resources_info_entry" => "interface/countrytradeview.gui#resources_info_entry",
        "resources_filter_entry" => "interface/countrytradeview.gui#resources_filter_entry",
        "filter_entry" => "interface/countrytradeview.gui#filter_entry",
        "country_trade_entry" => "interface/countrytradeview.gui#country_trade_entry",
        _ => unreachable!("closed XUI15 template set"),
    }
}

fn semantic_rows<I>(
    frame_hour: u64,
    country_tag: &str,
    token: &str,
    rows: I,
) -> Result<ObserverSemanticCollection>
where
    I: IntoIterator<Item = (String, usize, &'static str)>,
{
    let rows = rows
        .into_iter()
        .map(|(stable_item_id, ordinal, subtype_id)| {
            Ok(ObserverSemanticRow {
                stable_item_id,
                source_ordinal: u32::try_from(ordinal)
                    .map_err(|_| GameUiError::parse("XUI15 row ordinal exceeds u32"))?,
                subtype_id: subtype_id.to_string(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ObserverSemanticCollection {
        collection_instance_token: format!("{token}:{country_tag}:{frame_hour}"),
        frame_hour,
        context_country: Some(country_tag.to_string()),
        complete: true,
        rows,
    })
}

fn validate_panel(
    panel: AdmittedObserverTradePanel<'_>,
    viewer: &CountryTradeViewerState,
) -> Result<()> {
    if panel.country_tag.trim().is_empty() || panel.presentation.country_tag != panel.country_tag {
        return Err(GameUiError::missing(
            "XUI15 panel country does not match selected trade presentation",
        ));
    }
    if viewer.selected_resource_index >= TRADE_RESOURCE_ORDER.len() {
        return Err(GameUiError::parse(
            "XUI15 selected resource index is outside the complete resource catalogue",
        ));
    }
    let resources = match &panel.presentation.resource_cards {
        TradeData::Available(rows) => rows,
        TradeData::Unavailable(_) => {
            return Err(GameUiError::missing("XUI15 resource rows are incomplete"))
        }
    };
    if resources.len() != TRADE_RESOURCE_ORDER.len()
        || resources
            .iter()
            .zip(TRADE_RESOURCE_ORDER)
            .enumerate()
            .any(|(index, (row, expected))| {
                row.resource != expected || row.visible_order != index as i64
            })
    {
        return Err(GameUiError::missing(
            "XUI15 resource rows do not match the complete engine resource order",
        ));
    }
    if !matches!(panel.presentation.factories, TradeData::Available(_)) {
        return Err(GameUiError::missing("XUI15 factory header is incomplete"));
    }
    if !matches!(panel.presentation.deals, TradeData::Available(_)) {
        return Err(GameUiError::missing(
            "XUI15 active resource-route membership is incomplete",
        ));
    }
    validate_contiguous_unique(
        panel.catalogue_rows.iter().map(|row| {
            (
                row.stable_country_id.as_str(),
                row.display_ordinal,
                !row.country_name.trim().is_empty()
                    && !row.available_export_text.trim().is_empty()
                    && !row.exported_text.trim().is_empty()
                    && !row.convoys_text.trim().is_empty()
                    && row.flag.width > 0
                    && row.flag.height > 0,
            )
        }),
        "country catalogue",
    )?;
    let token_ids = panel
        .country_filters
        .iter()
        .map(|row| row.token_id)
        .collect::<BTreeSet<_>>();
    if token_ids.len() != panel.country_filters.len()
        || panel
            .country_filters
            .iter()
            .enumerate()
            .any(|(index, row)| {
                row.display_name.trim().is_empty() || row.source_ordinal != index as u32
            })
        || viewer
            .selected_country_filter_ids
            .iter()
            .any(|token| !token_ids.contains(token))
    {
        return Err(GameUiError::missing(
            "XUI15 viewer country filters do not match the complete token catalogue",
        ));
    }
    Ok(())
}

fn validate_contiguous_unique<'a, I>(rows: I, label: &str) -> Result<()>
where
    I: IntoIterator<Item = (&'a str, u32, bool)>,
{
    let mut ids = BTreeSet::new();
    for (expected, (id, ordinal, complete)) in rows.into_iter().enumerate() {
        if id.trim().is_empty() || !complete || !ids.insert(id) {
            return Err(GameUiError::missing(format!(
                "XUI15 {label} has an incomplete or duplicate row"
            )));
        }
        if ordinal != u32::try_from(expected).unwrap_or(u32::MAX) {
            return Err(GameUiError::missing(format!(
                "XUI15 {label} order is not contiguous from zero"
            )));
        }
    }
    Ok(())
}

fn fixed_to_pixels(value: i64) -> Result<i32> {
    if value < 0 {
        return Err(GameUiError::parse("XUI15 scroll is negative"));
    }
    i32::try_from(value / crate::trade_layout::TRADE_SCROLL_FIXED_SCALE)
        .map_err(|_| GameUiError::parse("XUI15 scroll exceeds i32"))
}

fn merge_output(target: &mut ComposeOutput, source: ComposeOutput) -> Result<()> {
    if target.image.width != source.image.width || target.image.height != source.image.height {
        return Err(GameUiError::parse("XUI15 layer extent differs from base"));
    }
    for (dst, src) in target
        .image
        .pixels
        .chunks_exact_mut(4)
        .zip(source.image.pixels.chunks_exact(4))
    {
        let value = alpha_blend(
            [dst[0], dst[1], dst[2], dst[3]],
            [src[0], src[1], src[2], src[3]],
        );
        dst.copy_from_slice(&value);
    }
    merge_report(&mut target.report, source.report)?;
    target.debug_rects.extend(source.debug_rects);
    Ok(())
}

fn merge_report(target: &mut ComposeReport, source: ComposeReport) -> Result<()> {
    target.drawn_elements = target.drawn_elements.saturating_add(source.drawn_elements);
    target.missing_sprites.extend(source.missing_sprites);
    target
        .undecodable_sprites
        .extend(source.undecodable_sprites);
    target.missing_fonts.extend(source.missing_fonts);
    for (font, glyphs) in source.missing_glyphs {
        target
            .missing_glyphs
            .entry(font)
            .or_default()
            .extend(glyphs);
    }
    target
        .unresolved_loc_keys
        .extend(source.unresolved_loc_keys);
    for (key, source_file) in source.localisation_sources_used {
        if target
            .localisation_sources_used
            .insert(key.clone(), source_file.clone())
            .is_some_and(|existing| existing != source_file)
        {
            return Err(GameUiError::missing(format!(
                "XUI15 localisation key {key} resolved from conflicting effective sources"
            )));
        }
    }
    for (element, issues) in source.unresolved_text_inputs {
        target
            .unresolved_text_inputs
            .entry(element)
            .or_default()
            .extend(issues);
    }
    Ok(())
}

#[cfg(test)]
/// A TAG this frame's census admits. A test that means to name a country says
/// so here; a test about a contended key asks `named_country` and gets `None`.
fn named<'tag>(
    frame: &'tag replay_core::WorldFrame,
    tag: &'tag str,
) -> replay_core::NamedCountry<'tag> {
    frame.named_country(tag).expect("census-admitted tag")
}

#[cfg(test)]
#[path = "gp_f10_coexistence.rs"]
mod gp_f10_coexistence;

#[cfg(test)]
mod producer_tests {
    use super::*;
    use replay_core::{RawFrame, ReplaySource, SourceError, WorldTimeline};
    use serde_json::{json, Value};
    use std::sync::Arc;

    #[test]
    fn strict_panel_semantics_are_private_to_the_sealing_module() {
        let source = include_str!("trade_compositor.rs").replace("\r\n", "\n");
        let start = source
            .find("pub struct StrictXui15PanelInput {")
            .expect("strict XUI15 panel input");
        let end = source[start..]
            .find("\n}\n\n#[derive(Clone, Debug, PartialEq, Eq)]")
            .map(|offset| start + offset)
            .expect("strict XUI15 panel boundary");
        let strict = &source[start..end];
        for field in [
            "country_tag",
            "completeness_record_sha256",
            "presentation",
            "catalogue_rows",
            "country_filters",
        ] {
            assert!(!strict.contains(&format!("pub {field}:")));
        }
    }

    struct MemorySource {
        rest: serde_json::Map<String, Value>,
    }

    impl ReplaySource for MemorySource {
        fn game_id(&self) -> &str {
            "xui15-producer-tests"
        }

        fn hours(&self) -> &[i64] {
            &[7]
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

    fn frame(value: Value) -> Arc<WorldFrame> {
        let rest = value.as_object().cloned().expect("object frame");
        WorldTimeline::open(Box::new(MemorySource { rest }))
            .expect("timeline")
            .state_at(7)
            .expect("frame")
    }

    fn complete_trade_country(tag_db_id: u32) -> Value {
        let rows = TRADE_RESOURCE_ORDER
            .iter()
            .enumerate()
            .map(|(ordinal, resource)| {
                let mut row = json!({
                    "resource": resource,
                    "visible_order": ordinal,
                    "balance": 0.0,
                    "produced": 0.0,
                    "imported": 0.0,
                    "exported": 0.0,
                    "special_project": 0.0,
                    "production_line": 0.0,
                    "needed": 0.0,
                    "needed_type": if *resource == "oil" { "current_max" } else { "surplus" }
                });
                if *resource == "oil" {
                    row["needed_current"] = json!(0.0);
                    row["needed_max"] = json!(0.0);
                }
                row
            })
            .collect::<Vec<_>>();
        json!({
            "_r2_tag_db_id": tag_db_id,
            "economy": {"resource_trade": {
                "resource_rows_complete": true,
                "resource_rows": rows,
                "deals_complete": true,
                "deals": [],
                "factories": {"total": 0, "used": 0, "available": 0}
            }}
        })
    }

    fn detached_completeness_authority(
        payload: &Xui15ProducerPayload,
    ) -> Xui15ProducerCompletenessAuthority {
        Xui15ProducerCompletenessAuthority {
            evaluated_filter_token_ids: payload.evaluated_filter_token_ids.clone(),
            catalogues: payload
                .catalogues
                .iter()
                .map(|catalogue| Xui15ProducerImporterCompletenessAuthority {
                    country_tag: catalogue.country_tag.clone(),
                    source_ordinal: catalogue.source_ordinal,
                    evaluated_country_tags: catalogue.evaluated_country_tags.clone(),
                    eligible_country_tags: catalogue.eligible_country_tags.clone(),
                })
                .collect(),
        }
    }

    fn reseal_payload_completeness(payload: &mut Xui15ProducerPayload) {
        payload.completeness_record_sha256 =
            xui15_completeness_record_sha256(&detached_completeness_authority(payload));
    }

    fn empty_payload(frame_identity: String) -> Xui15ProducerPayload {
        let mut payload = Xui15ProducerPayload {
            executable_sha256: map_viewer::XMAP03_A729_EXECUTABLE_SHA256.into(),
            producer_manifest_sha256: "1".repeat(64),
            replay_frame_identity_sha256: frame_identity,
            completeness_record_sha256: String::new(),
            evaluated_filter_token_ids: vec![1],
            country_filters: vec![Xui15ProducerCountryFilter {
                token_id: 1,
                source_ordinal: 0,
                localisation_key: "TRADE_FILTER_ALL".into(),
            }],
            catalogues: vec![Xui15ProducerSelectedCountryCatalogue {
                country_tag: "GER".into(),
                source_ordinal: 0,
                evaluated_country_tags: vec!["GER".into()],
                eligible_country_tags: Vec::new(),
                countries: Vec::new(),
            }],
        };
        reseal_payload_completeness(&mut payload);
        payload
    }

    #[test]
    fn frame_resident_payload_and_separate_authority_are_both_required() {
        let base = frame(json!({"countries": {"GER": complete_trade_country(1)}}));
        assert_eq!(xui15_producer_payload_from_frame(&base), Ok(None));
        let payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());

        let payload_only = frame(json!({
            "countries": {"GER": complete_trade_country(1)},
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&payload).unwrap(),
        }));
        assert_eq!(
            xui15_producer_payload_from_frame(&payload_only),
            Err(Xui15ProducerRejection::InvalidAuthority)
        );

        let complete = frame(json!({
            "countries": {"GER": complete_trade_country(1)},
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&payload).unwrap(),
            (XUI15_PRODUCER_AUTHORITY_FIELD): payload.replay_authority_record(
                7,
                &detached_completeness_authority(&payload),
            ),
        }));
        assert_eq!(
            xui15_producer_payload_from_frame(&complete),
            Ok(Some(payload.clone()))
        );
        assert!(AuthenticatedXui15Producer::authenticate_from_frame(
            payload,
            &complete,
            &"1".repeat(64),
        )
        .is_ok());
    }

    #[test]
    fn payload_tampering_cannot_reuse_the_frame_authority_receipt() {
        let base = frame(json!({"countries": {"GER": complete_trade_country(1)}}));
        let payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        let complete = frame(json!({
            "countries": {"GER": complete_trade_country(1)},
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&payload).unwrap(),
            (XUI15_PRODUCER_AUTHORITY_FIELD): payload.replay_authority_record(
                7,
                &detached_completeness_authority(&payload),
            ),
        }));
        let mut tampered = payload;
        tampered.catalogues[0].country_tag = "FRA".into();
        assert_eq!(
            AuthenticatedXui15Producer::authenticate_from_frame(
                tampered,
                &complete,
                &"1".repeat(64),
            ),
            Err(Xui15ProducerRejection::InvalidAuthority)
        );
    }

    #[test]
    fn country_matrix_requires_dense_exact_engine_ranks_for_every_resource() {
        let base = frame(json!({}));
        let mut payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        payload.catalogues[0].countries = ["FRA", "USA"]
            .into_iter()
            .enumerate()
            .map(|(country, tag)| Xui15ProducerCountryRow {
                country_tag: tag.into(),
                source_ordinal: country as u32,
                filter_token_ids: Vec::new(),
                resources: (0..7)
                    .map(|resource| Xui15ProducerCountryResource {
                        resource_index: resource,
                        available_export: 1,
                        exported: 2,
                        convoys: 3,
                        engine_sort_ordinals: [country as u32; 3],
                    })
                    .collect(),
            })
            .collect();
        payload.catalogues[0].eligible_country_tags = vec!["FRA".into(), "USA".into()];
        payload.catalogues.push(Xui15ProducerSelectedCountryCatalogue {
            country_tag: "ENG".into(),
            source_ordinal: 1,
            evaluated_country_tags: vec!["ENG".into(), "GER".into()],
            eligible_country_tags: Vec::new(),
            countries: Vec::new(),
        });
        assert_eq!(validate_xui15_payload_shape(&payload), Ok(()));
        payload.catalogues[0].countries[1].resources[3].engine_sort_ordinals[2] = 0;
        assert_eq!(
            validate_xui15_payload_shape(&payload),
            Err(Xui15ProducerRejection::IncompleteCountryCatalogue)
        );
    }

    #[test]
    fn selected_country_catalogues_are_dense_unique_and_recorder_independent() {
        let base = frame(json!({}));
        let mut payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        payload.catalogues.push(Xui15ProducerSelectedCountryCatalogue {
            country_tag: "ENG".into(),
            source_ordinal: 1,
            evaluated_country_tags: vec!["ENG".into(), "GER".into()],
            eligible_country_tags: Vec::new(),
            countries: Vec::new(),
        });
        assert_eq!(validate_xui15_payload_shape(&payload), Ok(()));
        payload.catalogues[1].country_tag = "GER".into();
        assert_eq!(
            validate_xui15_payload_shape(&payload),
            Err(Xui15ProducerRejection::IncompleteCountryCatalogue)
        );
    }

    #[test]
    fn authenticated_catalogues_exactly_cover_the_complete_d18_viewer_roster() {
        let base = frame(json!({
            "countries": {
                "ENG": complete_trade_country(1),
                "GER": complete_trade_country(2)
            }
        }));
        let mut payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        payload.catalogues[0].evaluated_country_tags = vec!["ENG".into(), "GER".into()];
        payload.catalogues.insert(
            0,
            Xui15ProducerSelectedCountryCatalogue {
                country_tag: "ENG".into(),
                source_ordinal: 0,
                evaluated_country_tags: vec!["ENG".into(), "GER".into()],
                eligible_country_tags: Vec::new(),
                countries: Vec::new(),
            },
        );
        payload.catalogues[1].source_ordinal = 1;
        reseal_payload_completeness(&mut payload);
        let complete = frame(json!({
            "countries": {
                "ENG": complete_trade_country(1),
                "GER": complete_trade_country(2)
            },
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&payload).unwrap(),
            (XUI15_PRODUCER_AUTHORITY_FIELD): payload.replay_authority_record(
                7,
                &detached_completeness_authority(&payload),
            ),
        }));
        assert!(AuthenticatedXui15Producer::authenticate_from_frame(
            payload.clone(),
            &complete,
            &"1".repeat(64),
        )
        .is_ok());

        let mut missing = payload;
        missing.catalogues.remove(0);
        assert_eq!(
            validate_complete_catalogue_roster(&missing, &complete),
            Err(Xui15ProducerRejection::IncompleteCountryCatalogue)
        );
    }

    #[test]
    fn filter_truncation_of_rows_and_payload_census_survives_full_reseal_and_is_rejected() {
        let base = frame(json!({"countries": {"GER": complete_trade_country(1)}}));
        let mut payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        payload.country_filters.push(Xui15ProducerCountryFilter {
            token_id: 2,
            source_ordinal: 1,
            localisation_key: "TRADE_FILTER_NEIGHBOUR".into(),
        });
        payload.evaluated_filter_token_ids.push(2);
        reseal_payload_completeness(&mut payload);
        let independent_authority = detached_completeness_authority(&payload);

        let mut truncated = payload;
        truncated.country_filters.remove(1);
        truncated.evaluated_filter_token_ids.remove(1);
        reseal_payload_completeness(&mut truncated);
        let complete = frame(json!({
            "countries": {"GER": complete_trade_country(1)},
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&truncated).unwrap(),
            (XUI15_PRODUCER_AUTHORITY_FIELD): truncated.replay_authority_record(
                7,
                &independent_authority,
            ),
        }));
        assert_eq!(
            AuthenticatedXui15Producer::authenticate_from_frame(
                truncated,
                &complete,
                &"1".repeat(64),
            ),
            Err(Xui15ProducerRejection::IncompleteFilterCatalogue)
        );
    }

    #[test]
    fn eligible_country_row_and_payload_census_truncation_survives_full_reseal_and_is_rejected() {
        let base = frame(json!({"countries": {"GER": complete_trade_country(1)}}));
        let mut payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        payload.catalogues[0].eligible_country_tags = vec!["GER".into()];
        payload.catalogues[0].countries = vec![Xui15ProducerCountryRow {
            country_tag: "GER".into(),
            source_ordinal: 0,
            filter_token_ids: Vec::new(),
            resources: (0..7)
                .map(|resource_index| Xui15ProducerCountryResource {
                    resource_index,
                    available_export: 0,
                    exported: 0,
                    convoys: 0,
                    engine_sort_ordinals: [0, 0, 0],
                })
                .collect(),
        }];
        reseal_payload_completeness(&mut payload);
        let independent_authority = detached_completeness_authority(&payload);

        let mut truncated = payload;
        truncated.catalogues[0].countries.clear();
        truncated.catalogues[0].eligible_country_tags.clear();
        reseal_payload_completeness(&mut truncated);
        let complete = frame(json!({
            "countries": {"GER": complete_trade_country(1)},
            (XUI15_PRODUCER_PAYLOAD_FIELD): serde_json::to_value(&truncated).unwrap(),
            (XUI15_PRODUCER_AUTHORITY_FIELD): truncated.replay_authority_record(
                7,
                &independent_authority,
            ),
        }));
        assert_eq!(
            AuthenticatedXui15Producer::authenticate_from_frame(
                truncated,
                &complete,
                &"1".repeat(64),
            ),
            Err(Xui15ProducerRejection::IncompleteCountryCatalogue)
        );
    }

    #[test]
    fn payload_schema_rejects_unknown_fields_instead_of_silently_ignoring_them() {
        let base = frame(json!({}));
        let payload = empty_payload(xui15_replay_frame_identity_sha256(&base).unwrap());
        let mut value = serde_json::to_value(payload).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("viewer_scroll".into(), json!(42));
        let authority = json!({"placeholder": true});
        let carrier = frame(json!({
            (XUI15_PRODUCER_PAYLOAD_FIELD): value,
            (XUI15_PRODUCER_AUTHORITY_FIELD): authority,
        }));
        assert_eq!(
            xui15_producer_payload_from_frame(&carrier),
            Err(Xui15ProducerRejection::InvalidAuthority)
        );
    }
}
