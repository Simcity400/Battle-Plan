//! Product consumption of exact D48 construction/resource/building state.
//!
//! D48 owns gameplay aggregates; O05 owns per-instance building selection and
//! its exact D80 output join. This adapter consumes both from the same
//! WorldFrame/selected-Present without inventing a state-id mapping for O05's
//! still-opaque engine handles.

use replay_core::{AuthenticatedUiPass, Leaf, ViewStatus, WorldFrame};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AdmittedD48SelectedPresentCompanion {
    pub frame_hour: i64,
    pub selected_present_receipt_sha256: String,
    pub authenticated_ui_pass_identity_sha256: String,
    pub country_count: usize,
    pub construction_line_count: usize,
    pub resource_state_count: usize,
    pub resource_amount_count: usize,
    pub state_building_type_count: usize,
    pub province_building_type_count: usize,
    /// Exact O05 runtime records for this selected Present. `None` means the
    /// per-instance carrier was absent; no aggregate D48 row substitutes.
    pub o05_runtime_record_count: Option<usize>,
    pub o05_submitted_output_count: Option<usize>,
}

pub fn admit_d48_selected_present_companion(
    frame: &WorldFrame,
    pass: &AuthenticatedUiPass,
) -> Result<AdmittedD48SelectedPresentCompanion, String> {
    pass.validate()
        .map_err(|error| format!("D48 authenticated UI pass rejected: {error}"))?;
    let render = frame
        .production_render()
        .ok_or("D48 has no authenticated production-render frame")?;
    let selected_receipt = render
        .selected_present
        .computed_sha256()
        .map_err(|error| format!("D48 selected-Present cannot be sealed: {error}"))?;
    validate_selected_present_join_values(
        frame.hour_index(),
        render.selected_present.selected_game_hour_raw,
        render.update_index,
        pass.update_index,
        &selected_receipt,
        &pass.selected_present_receipt_sha256,
        &pass.pass_identity_sha256,
    )?;

    let exact = frame.d48_construction_resources();
    if exact.status != ViewStatus::Present {
        return Err(format!(
            "D48 construction/resource model is {:?}",
            exact.status
        ));
    }
    if frame.d48_buildings_status() != ViewStatus::Present {
        return Err(format!(
            "D48 building model is {:?}",
            frame.d48_buildings_status()
        ));
    }

    let mut state_building_type_count = 0usize;
    let mut province_building_type_count = 0usize;
    for state in frame.buildings_by_state().values() {
        state_building_type_count += state
            .state_buildings
            .values()
            .filter(|building| {
                matches!(
                    &building.source,
                    Leaf::Measured(source)
                        if source == "construction_buildings_state_damage_live"
                )
            })
            .count();
        province_building_type_count += state
            .province_buildings
            .values()
            .flat_map(|buildings| buildings.values())
            .filter(|building| {
                matches!(
                    &building.source,
                    Leaf::Measured(source)
                        if source == "construction_buildings_state_damage_live"
                )
            })
            .count();
    }

    // ProductionRenderFrame::validate already proves every O05 submitted
    // output is the same display submit's exact entity output and exact used
    // content. We deliberately do not correlate opaque O05 state/province
    // handles with D48 ids.
    let (o05_runtime_record_count, o05_submitted_output_count) =
        match render.o05_buildings.as_ref() {
            Some(o05) => (
                Some(o05.records.len()),
                Some(o05.ordered_outputs.len()),
            ),
            None => (None, None),
        };

    Ok(AdmittedD48SelectedPresentCompanion {
        frame_hour: frame.hour_index(),
        selected_present_receipt_sha256: selected_receipt,
        authenticated_ui_pass_identity_sha256: pass.pass_identity_sha256.clone(),
        country_count: exact.construction_by_country.len(),
        construction_line_count: exact
            .construction_by_country
            .values()
            .map(Vec::len)
            .sum(),
        resource_state_count: exact.resource_amount_raw_by_state.len(),
        resource_amount_count: exact
            .resource_amount_raw_by_state
            .values()
            .map(|resources| resources.len())
            .sum(),
        state_building_type_count,
        province_building_type_count,
        o05_runtime_record_count,
        o05_submitted_output_count,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_selected_present_join_values(
    frame_hour: i64,
    selected_game_hour_raw: i32,
    render_update_index: u64,
    pass_update_index: u64,
    selected_present_receipt_sha256: &str,
    pass_selected_present_receipt_sha256: &str,
    pass_identity_sha256: &str,
) -> Result<(), String> {
    if frame_hour != i64::from(selected_game_hour_raw) {
        return Err("D48 world-frame hour differs from selected-Present hour".into());
    }
    if render_update_index != pass_update_index {
        return Err("D48 production-render update differs from UI-pass update".into());
    }
    if selected_present_receipt_sha256 != pass_selected_present_receipt_sha256 {
        return Err("D48 model and UI pass name different selected Presents".into());
    }
    if pass_identity_sha256.len() != 64
        || !pass_identity_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("D48 UI-pass identity is not a canonical SHA-256".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_selected_present_join_values;

    #[test]
    fn d48_selected_present_join_is_exact_and_fail_closed() {
        let hash = "a".repeat(64);
        assert!(validate_selected_present_join_values(
            41, 41, 7, 7, &hash, &hash, &hash
        )
        .is_ok());
        assert!(validate_selected_present_join_values(
            41, 42, 7, 7, &hash, &hash, &hash
        )
        .is_err());
        assert!(validate_selected_present_join_values(
            41, 41, 7, 8, &hash, &hash, &hash
        )
        .is_err());
        assert!(validate_selected_present_join_values(
            41,
            41,
            7,
            7,
            &hash,
            &"b".repeat(64),
            &hash
        )
        .is_err());
    }
}
