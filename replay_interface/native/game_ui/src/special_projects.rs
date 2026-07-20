//! Exact special-project progress admission for the replay UI.
//!
//! The engine counter and target stay raw. The UI may format their ratio, but
//! this adapter never invents a scale or reconstructs progress from elapsed
//! time. Rows are admitted only from the owner-joined D41 `PPGF` view.

use replay_core::{
    ProjectSpecializationExact, SpecialProjectBreakthroughExactView,
    SpecialProjectProgressExactRow, SpecialProjectProgressExactView, ViewStatus,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialProjectProgressPresentation {
    pub country: String,
    pub local_id: u32,
    pub static_def_handle: u32,
    pub current_state_offset: u32,
    pub current_state_enum: u32,
    pub prototype_iterations: u32,
    pub phase_progress_raw: u32,
    pub phase_target_raw: u32,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialProjectProgressRejection(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialProjectBreakthroughPresentation {
    pub country: String,
    pub specialization: ProjectSpecializationExact,
    pub specialization_token: &'static str,
    pub points_raw: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecialProjectBreakthroughRejection(pub String);

fn admit_row(
    row: &SpecialProjectProgressExactRow,
) -> Result<SpecialProjectProgressPresentation, SpecialProjectProgressRejection> {
    if row.country.is_empty()
        || row.local_id == 0
        || row.static_def_handle == 0
        || !matches!(row.current_state_offset, 0x18 | 0xE8 | 0x168 | 0x1E8)
        || row.current_state_enum > 64
        || row.prototype_state_enum > 64
        || row.simple_state_1_enum > 64
        || row.simple_state_2_enum > 64
        || row.simple_state_3_enum > 64
        || row.prototype_iterations > 1_000_000
        || row.phase_progress_raw > 1_000_000_000
        || row.phase_target_raw == 0
        || row.phase_target_raw > 1_000_000
    {
        return Err(SpecialProjectProgressRejection(
            "special-project progress row violates exact a729 bounds".into(),
        ));
    }
    Ok(SpecialProjectProgressPresentation {
        country: row.country.clone(),
        local_id: row.local_id,
        static_def_handle: row.static_def_handle,
        current_state_offset: row.current_state_offset,
        current_state_enum: row.current_state_enum,
        prototype_iterations: row.prototype_iterations,
        phase_progress_raw: row.phase_progress_raw,
        phase_target_raw: row.phase_target_raw,
        completed: row.completed,
    })
}

/// Admit the complete project-progress catalog, optionally filtered to the
/// selected country. Measured-empty is valid. Any incomplete country walk or
/// duplicate owner/local identity rejects the whole surface.
pub fn admit_special_project_progress(
    view: &SpecialProjectProgressExactView,
    country: Option<&str>,
) -> Result<Vec<SpecialProjectProgressPresentation>, SpecialProjectProgressRejection> {
    if view.status != ViewStatus::Present
        || view.authority.as_deref() != Some("exact_a729_ppgf_ppa1_owner_join")
        || view.walked_countries.is_none()
        || view.aborted_countries != Some(0)
    {
        return Err(SpecialProjectProgressRejection(
            "owner-joined project-progress authority is unavailable".into(),
        ));
    }
    let mut identities = BTreeSet::new();
    let mut owners = BTreeSet::new();
    let mut out = Vec::new();
    for row in &view.rows {
        if !identities.insert((row.country.clone(), row.local_id)) {
            return Err(SpecialProjectProgressRejection(
                "duplicate owner/local project-progress identity".into(),
            ));
        }
        owners.insert(row.country.clone());
        let admitted = admit_row(row)?;
        if country.map_or(true, |expected| expected == row.country) {
            out.push(admitted);
        }
    }
    if view.walked_countries != Some(owners.len() as u32) {
        return Err(SpecialProjectProgressRejection(
            "project-progress walked country count does not match rows".into(),
        ));
    }
    Ok(out)
}

/// Admit X9 breakthrough values only after the exact serializer-token census
/// has joined the raw engine key to the closed portable enum. Raw numeric
/// handles are intentionally absent from typed replay and UI identity.
pub fn admit_special_project_breakthroughs(
    view: &SpecialProjectBreakthroughExactView,
    country: Option<&str>,
) -> Result<Vec<SpecialProjectBreakthroughPresentation>, SpecialProjectBreakthroughRejection> {
    if view.status != ViewStatus::Present
        || view.authority.as_deref() != Some("exact_a729_x9_ppa1_d18_join")
    {
        return Err(SpecialProjectBreakthroughRejection(
            "exact project-specialization authority is unavailable".into(),
        ));
    }
    let mut identities = BTreeSet::new();
    let mut out = Vec::new();
    for row in &view.rows {
        if row.country.is_empty()
            || row.tag_db_id == 0
            || !identities.insert((row.tag_db_id, row.specialization))
        {
            return Err(SpecialProjectBreakthroughRejection(
                "project-specialization row violates exact identity".into(),
            ));
        }
        if country.is_some_and(|expected| expected != row.country) {
            continue;
        }
        out.push(SpecialProjectBreakthroughPresentation {
            country: row.country.clone(),
            specialization: row.specialization,
            specialization_token: row.specialization.token(),
            points_raw: row.points_raw,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(country: &str, local_id: u32) -> SpecialProjectProgressExactRow {
        SpecialProjectProgressExactRow {
            country: country.into(),
            local_id,
            static_def_handle: 6655,
            current_state_offset: 0x18,
            current_state_enum: 0,
            prototype_state_enum: 0,
            simple_state_1_enum: 1,
            simple_state_2_enum: 2,
            simple_state_3_enum: 3,
            prototype_iterations: 7,
            phase_progress_raw: 3_000_000,
            phase_target_raw: 60,
            completed: false,
        }
    }

    #[test]
    fn admits_same_local_id_for_distinct_country_owners() {
        let view = SpecialProjectProgressExactView {
            status: ViewStatus::Present,
            authority: Some("exact_a729_ppgf_ppa1_owner_join".into()),
            rows: vec![row("GER", 77), row("USA", 77)],
            walked_countries: Some(2),
            aborted_countries: Some(0),
        };
        assert_eq!(admit_special_project_progress(&view, None).unwrap().len(), 2);
        assert_eq!(admit_special_project_progress(&view, Some("GER")).unwrap().len(), 1);
        let mut inner_only = view.clone();
        inner_only.authority = Some("exact_a729_owner_joined_project_progress".into());
        assert!(admit_special_project_progress(&inner_only, None).is_err());
    }

    #[test]
    fn rejects_partial_country_walk() {
        let view = SpecialProjectProgressExactView {
            status: ViewStatus::Present,
            authority: Some("exact_a729_ppgf_ppa1_owner_join".into()),
            rows: vec![row("GER", 77)],
            walked_countries: Some(1),
            aborted_countries: Some(1),
        };
        assert!(admit_special_project_progress(&view, None).is_err());
    }

    #[test]
    fn selected_country_still_rejects_incomplete_global_owner_catalog() {
        let view = SpecialProjectProgressExactView {
            status: ViewStatus::Present,
            authority: Some("exact_a729_ppgf_ppa1_owner_join".into()),
            rows: vec![row("GER", 77), row("USA", 77)],
            walked_countries: Some(1),
            aborted_countries: Some(0),
        };
        assert!(admit_special_project_progress(&view, Some("GER")).is_err());
    }

    #[test]
    fn admits_exact_x9_specialization_and_rejects_unavailable_view() {
        let view = SpecialProjectBreakthroughExactView {
            status: ViewStatus::Present,
            authority: Some("exact_a729_x9_ppa1_d18_join".into()),
            rows: vec![replay_core::SpecialProjectBreakthroughExactRow {
                country: "GER".into(),
                tag_db_id: 47,
                specialization: ProjectSpecializationExact::Air,
                points_raw: 7_932,
            }],
        };
        let rows = admit_special_project_breakthroughs(&view, Some("GER")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].specialization_token, "specialization_air");
        assert_eq!(rows[0].points_raw, 7_932);

        let mut wrong_outer = view.clone();
        wrong_outer.authority = Some("exact_a729_serializer_token_enum_join".into());
        assert!(admit_special_project_breakthroughs(&wrong_outer, None).is_err());

        let unavailable = SpecialProjectBreakthroughExactView {
            status: ViewStatus::DomainUnavailable,
            authority: None,
            rows: Vec::new(),
        };
        assert!(admit_special_project_breakthroughs(&unavailable, None).is_err());
    }
}
