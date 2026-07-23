//! Selected-Present semantic companions for late exact replay domains.
//!
//! D91 raid state and D93 army-upgrade state are typed engine models. They do
//! not own a second pixel renderer: when their UI is visible, pixels remain
//! exclusively owned by the authenticated selected-Present UI pass. These
//! adapters make that relationship explicit and fail closed unless the model,
//! world-frame hour, selected-Present receipt, and pass identity all agree.

use replay_core::{
    AuthenticatedUiPass, CountryArmyUpgradeExactView, Leaf, RaidStateView, ViewStatus, WorldFrame,
};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct SelectedPresentCompanionIdentity {
    pub frame_hour: i64,
    pub selected_present_receipt_sha256: String,
    pub authenticated_ui_pass_identity_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AdmittedRaidSelectedPresentCompanion {
    pub identity: SelectedPresentCompanionIdentity,
    pub country_status_count: usize,
    pub instance_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct AdmittedArmyUpgradeSelectedPresentCompanion {
    pub identity: SelectedPresentCompanionIdentity,
    pub country_count: usize,
    pub army_group_count: usize,
    pub request_count: usize,
    pub delivery_count: usize,
    pub produced_count: usize,
}

/// Admit the complete D91 raid model as a semantic companion of exactly one
/// authenticated selected-Present. This grants no independent pixel authority.
pub fn admit_d91_raid_selected_present_companion(
    frame: &WorldFrame,
    pass: &AuthenticatedUiPass,
) -> Result<AdmittedRaidSelectedPresentCompanion, String> {
    let identity = selected_present_companion_identity(frame, pass, "D91 raids")?;
    let (country_status_count, instance_count) = summarize_complete_raids(frame.raids())?;
    Ok(AdmittedRaidSelectedPresentCompanion {
        identity,
        country_status_count,
        instance_count,
    })
}

/// Admit the complete D93 army-upgrade model as a semantic companion of
/// exactly one authenticated selected-Present. This grants no independent
/// pixel authority.
pub fn admit_d93_army_upgrade_selected_present_companion(
    frame: &WorldFrame,
    pass: &AuthenticatedUiPass,
) -> Result<AdmittedArmyUpgradeSelectedPresentCompanion, String> {
    let identity = selected_present_companion_identity(frame, pass, "D93 army upgrades")?;
    let (country_count, army_group_count, request_count, delivery_count, produced_count) =
        summarize_complete_army_upgrades(frame.country_army_upgrade_exact())?;
    Ok(AdmittedArmyUpgradeSelectedPresentCompanion {
        identity,
        country_count,
        army_group_count,
        request_count,
        delivery_count,
        produced_count,
    })
}

fn selected_present_companion_identity(
    frame: &WorldFrame,
    pass: &AuthenticatedUiPass,
    feature: &str,
) -> Result<SelectedPresentCompanionIdentity, String> {
    pass.validate()
        .map_err(|error| format!("{feature} authenticated UI pass rejected: {error}"))?;
    let render = frame
        .production_render()
        .ok_or_else(|| format!("{feature} has no authenticated production-render frame"))?;
    let selected = &render.selected_present;
    let selected_present_receipt_sha256 = selected
        .computed_sha256()
        .map_err(|error| format!("{feature} selected-Present cannot be sealed: {error}"))?;
    validate_selected_present_join_values(
        feature,
        frame.hour_index(),
        selected.selected_game_hour_raw,
        render.update_index,
        pass.update_index,
        &selected_present_receipt_sha256,
        &pass.selected_present_receipt_sha256,
        &pass.pass_identity_sha256,
    )?;
    Ok(SelectedPresentCompanionIdentity {
        frame_hour: frame.hour_index(),
        selected_present_receipt_sha256,
        authenticated_ui_pass_identity_sha256: pass.pass_identity_sha256.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_selected_present_join_values(
    feature: &str,
    frame_hour: i64,
    selected_game_hour_raw: i32,
    render_update_index: u64,
    pass_update_index: u64,
    selected_present_receipt_sha256: &str,
    pass_selected_present_receipt_sha256: &str,
    pass_identity_sha256: &str,
) -> Result<(), String> {
    if frame_hour != i64::from(selected_game_hour_raw) {
        return Err(format!(
            "{feature} world-frame hour {frame_hour} differs from selected-Present hour {selected_game_hour_raw}"
        ));
    }
    if render_update_index != pass_update_index {
        return Err(format!(
            "{feature} production-render update {render_update_index} differs from authenticated UI-pass update {pass_update_index}"
        ));
    }
    if selected_present_receipt_sha256 != pass_selected_present_receipt_sha256 {
        return Err(format!(
            "{feature} model and authenticated UI pass name different selected Presents"
        ));
    }
    if pass_identity_sha256.len() != 64
        || !pass_identity_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!(
            "{feature} authenticated UI-pass identity is not a canonical SHA-256"
        ));
    }
    Ok(())
}

fn summarize_complete_raids(view: &RaidStateView) -> Result<(usize, usize), String> {
    if view.status != ViewStatus::Present || !view.instances_complete {
        return Err(format!(
            "D91 raid model is {:?} with instances_complete={}",
            view.status, view.instances_complete
        ));
    }
    let Leaf::Measured(country_statuses) = &view.country_statuses else {
        return Err("D91 Present raid model omitted its measured country-status census".into());
    };
    let Leaf::Measured(instances) = &view.instances else {
        return Err("D91 Present raid model omitted its measured instance census".into());
    };
    Ok((country_statuses.len(), instances.len()))
}

fn summarize_complete_army_upgrades(
    view: &CountryArmyUpgradeExactView,
) -> Result<(usize, usize, usize, usize, usize), String> {
    if view.status != ViewStatus::Present {
        return Err(format!("D93 army-upgrade model is {:?}", view.status));
    }
    let army_group_count = view
        .countries
        .values()
        .map(|country| country.groups.len())
        .sum();
    let request_count = view
        .countries
        .values()
        .flat_map(|country| &country.groups)
        .map(|group| group.requests.len())
        .sum();
    let delivery_count = view
        .countries
        .values()
        .flat_map(|country| &country.groups)
        .map(|group| group.deliveries.len())
        .sum();
    let produced_count = view
        .countries
        .values()
        .flat_map(|country| &country.groups)
        .flat_map(|group| &group.deliveries)
        .map(|delivery| delivery.produced.len())
        .sum();
    Ok((
        view.countries.len(),
        army_group_count,
        request_count,
        delivery_count,
        produced_count,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn selected_present_join_is_exact_and_fail_closed() {
        let hash = "a".repeat(64);
        assert!(validate_selected_present_join_values(
            "D91 raids", 41, 41, 7, 7, &hash, &hash, &hash
        )
        .is_ok());
        assert!(validate_selected_present_join_values(
            "D91 raids", 41, 42, 7, 7, &hash, &hash, &hash
        )
        .unwrap_err()
        .contains("world-frame hour"));
        assert!(validate_selected_present_join_values(
            "D91 raids",
            41,
            41,
            7,
            8,
            &hash,
            &hash,
            &hash
        )
        .unwrap_err()
        .contains("update"));
        assert!(validate_selected_present_join_values(
            "D91 raids",
            41,
            41,
            7,
            7,
            &hash,
            &"b".repeat(64),
            &hash
        )
        .unwrap_err()
        .contains("different selected Presents"));
    }

    #[test]
    fn absent_or_incomplete_domains_never_look_measured_empty() {
        let raids = RaidStateView {
            status: ViewStatus::Present,
            priorities: BTreeMap::new(),
            next_target: Leaf::Measured(0),
            country_statuses: Leaf::Measured(Vec::new()),
            instances: Leaf::Measured(Vec::new()),
            instances_complete: false,
        };
        assert!(summarize_complete_raids(&raids)
            .unwrap_err()
            .contains("instances_complete=false"));

        let upgrades = CountryArmyUpgradeExactView {
            status: ViewStatus::DomainUnavailable,
            countries: BTreeMap::new(),
        };
        assert!(summarize_complete_army_upgrades(&upgrades)
            .unwrap_err()
            .contains("DomainUnavailable"));
    }
}
