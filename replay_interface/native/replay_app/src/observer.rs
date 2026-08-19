//! E6 observer country selection and optional fog mode.
//!
//! Product model (see the `PROJECT.md` heading "Fidelity and truth states"):
//! the
//! default view is the omniscient world (no fog). The viewer may select any
//! country in the complete d18 roster for replay-owned panels. Fog is a
//! separate optional request and is admitted only when the selected country
//! has complete current-frame d94/d95 evidence. This module owns:
//!
//! * [`PovState`] — the POV state object + a stateless, idempotent toggle.
//! * [`povs_from_frame`] — the country picker source, derived from the typed
//!   complete d18 roster, never from the session-player subset.
//! * [`PovState::render_status`] — the fail-closed render contract: because
//!   a selected POV requires authenticated current-frame replay inputs;
//!   POV entry binds to the current frame's exact d94 province visibility and
//!   d95 spotted-unit state. Missing or incomplete data is unavailable rather
//!   than an invitation to display omniscient pixels.
//!
//! The POV *state* still flips on entry so the UI can show which country is
//! observed and offer the toggle.
//!
//! [`povs_from_session`] remains only as a legacy/session diagnostic helper.
//! Product country selection calls [`povs_from_frame`] and therefore cannot be
//! restricted by multiplayer participation or recorder POV.

use replay_core::{
    Leaf, PovVisibilityView, SessionMetadataView, SpottedUnitsView, ViewStatus, WorldFrame,
};
#[cfg(test)]
use replay_core::SessionHeaderView;

/// Fail-closed diagnostic prefix for an unavailable selected-POV frame join.
pub const POV_RECORDED_STATE_UNAVAILABLE: &str =
    "selected POV has no complete current-frame d94/d95 visibility and spotted-unit evidence";

/// Fail-closed diagnostic surfaced when the recording carries no
/// player-country list the picker can use (see the module note).
pub const SESSION_PLAYERS_GAP: &str = "session player-country list unavailable: this recording \
     carries no player-country list under session.player_countries or \
     world.scenario.players (replay_core SessionMetadataView.player_countries \
     is Absent); country picker fails closed";

/// Observer view mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverMode {
    /// Default omniscient world view (no fog).
    Omniscient,
    /// Rendering optional fog as a country (by TAG).
    Pov(String),
}

/// One selectable d18 country in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryChoice {
    pub tag: String,
    /// Human player label when the recording carried one.
    pub label: Option<String>,
}

/// The country picker's source state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovPicker {
    /// One or more selectable d18 countries.
    Available(Vec<CountryChoice>),
    /// No usable player-country list; carries the fail-closed reason.
    Unavailable(String),
}

/// What the map should render for the current POV state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovRenderStatus {
    /// Omniscient world view — render normally.
    Omniscient,
    /// Exact current-frame d94/d95 inputs admitted for this POV.
    Recorded {
        pov: String,
        province_slot_count: usize,
        visibility_capture_sequence: u64,
        pov_country_id: u32,
        spotted_land_unit_count: Option<usize>,
        spotted_taskforce_count: Option<usize>,
    },
    /// A selected POV lacks complete current-frame inputs. The caller must
    /// omit the POV-dependent render, never substitute the omniscient world.
    Unavailable {
        pov: String,
        diagnostic: String,
    },
}

/// The POV state object. Holds the current mode plus the last-entered POV so
/// the one-key toggle can flip back to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PovState {
    selected_ui_country: Option<String>,
    mode: ObserverMode,
    last_pov: Option<String>,
}

impl Default for PovState {
    fn default() -> Self {
        PovState {
            selected_ui_country: None,
            mode: ObserverMode::Omniscient,
            last_pov: None,
        }
    }
}

impl PovState {
    pub fn new() -> PovState {
        PovState::default()
    }

    pub fn mode(&self) -> &ObserverMode {
        &self.mode
    }
    /// Country whose replay-owned panels are open. This is independent of fog.
    pub fn selected_ui_country(&self) -> Option<&str> {
        self.selected_ui_country.as_deref()
    }
    /// Select a country for replay-owned UI without requesting fog.
    pub fn select_ui_country(&mut self, tag: &str) {
        self.selected_ui_country = Some(tag.to_string());
    }
    /// Clear both panel and fog selection after the complete roster becomes
    /// unavailable. No previous-hour country identity is held as current.
    pub fn clear_country_selection(&mut self) {
        self.selected_ui_country = None;
        self.mode = ObserverMode::Omniscient;
        self.last_pov = None;
    }
    pub fn is_omniscient(&self) -> bool {
        matches!(self.mode, ObserverMode::Omniscient)
    }
    /// The active POV tag, or `None` in the world view.
    pub fn active_pov(&self) -> Option<&str> {
        match &self.mode {
            ObserverMode::Pov(tag) => Some(tag),
            ObserverMode::Omniscient => None,
        }
    }
    /// The last POV entered this session (the toggle target), if any.
    pub fn last_pov(&self) -> Option<&str> {
        self.last_pov.as_deref()
    }

    /// Enter a country POV. Records it as the toggle target. Entering the same
    /// tag again is idempotent (state unchanged).
    pub fn enter_pov(&mut self, tag: &str) {
        self.selected_ui_country = Some(tag.to_string());
        self.mode = ObserverMode::Pov(tag.to_string());
        self.last_pov = Some(tag.to_string());
    }

    /// Return to the omniscient world view (keeps the last-POV toggle target).
    pub fn exit_to_world(&mut self) {
        self.mode = ObserverMode::Omniscient;
    }

    /// One-key toggle between the world view and the last-entered POV. This is
    /// **stateless** (the result is a pure function of `mode` + `last_pov`)
    /// and **idempotent under pairing**: `toggle(); toggle();` returns to the
    /// starting state. With no POV ever entered, toggling is a no-op.
    pub fn toggle(&mut self) {
        self.mode = match &self.mode {
            ObserverMode::Omniscient => match self
                .selected_ui_country
                .as_ref()
                .or(self.last_pov.as_ref())
            {
                Some(tag) => ObserverMode::Pov(tag.clone()),
                None => ObserverMode::Omniscient,
            },
            ObserverMode::Pov(_) => ObserverMode::Omniscient,
        };
    }

    /// Join the observer mode to the displayed frame (fail-closed on POV).
    pub fn render_status(&self, frame: Option<&WorldFrame>) -> PovRenderStatus {
        match &self.mode {
            ObserverMode::Omniscient => PovRenderStatus::Omniscient,
            ObserverMode::Pov(tag) => {
                let Some(frame) = frame else {
                    return PovRenderStatus::Unavailable {
                        pov: tag.clone(),
                        diagnostic: format!(
                            "{POV_RECORDED_STATE_UNAVAILABLE}: displayed WorldFrame is absent"
                        ),
                    };
                };
                if frame.visibility_status() != ViewStatus::Present {
                    return PovRenderStatus::Unavailable {
                        pov: tag.clone(),
                        diagnostic: format!(
                            "{POV_RECORDED_STATE_UNAVAILABLE}: visibility status is {:?}",
                            frame.visibility_status()
                        ),
                    };
                }
                let visibility = frame.visibility_for(tag);
                let spotted = frame.spotted_units_for(tag);
                match admit_recorded_pov(tag, visibility.as_ref(), &spotted) {
                    Ok(status) => status,
                    Err(reason) => PovRenderStatus::Unavailable {
                        pov: tag.clone(),
                        diagnostic: format!("{POV_RECORDED_STATE_UNAVAILABLE}: {reason}"),
                    },
                }
            }
        }
    }
}

fn admit_recorded_pov(
    pov: &str,
    visibility: &PovVisibilityView,
    spotted: &SpottedUnitsView,
) -> Result<PovRenderStatus, String> {
    if visibility.status != ViewStatus::Present {
        return Err(format!("POV visibility status is {:?}", visibility.status));
    }
    let Leaf::Measured(levels) = &visibility.raw_province_levels else {
        return Err("exact d94 province visibility vector is not measured".into());
    };
    if levels.is_empty() || visibility.provinces.len() != levels.len() {
        return Err("exact d94 province vector is empty or not a complete slot census".into());
    }
    let Leaf::Measured(visibility_capture_sequence) = &visibility.capture_sequence else {
        return Err("exact d94 capture sequence is not measured".into());
    };
    if *visibility_capture_sequence == 0 {
        return Err("exact d94 capture sequence is zero".into());
    }
    let Leaf::Measured(pov_country_id) = &visibility.pov_country_id else {
        return Err("exact d94 POV country identity is not measured".into());
    };
    if *pov_country_id == 0 {
        return Err("exact d94 POV country identity is zero".into());
    }
    if &visibility.spotted_units != spotted {
        return Err("d95 accessor disagrees with the d94-attached spotted-unit view".into());
    }
    if spotted.status != ViewStatus::Present {
        return Err(format!("spotted-unit status is {:?}", spotted.status));
    }

    let spotted_land_unit_count = exact_spotted_surface(
        "land",
        &spotted.land_units,
        &spotted.divisions,
        &spotted.land_frame_sequence,
    )?;
    let spotted_taskforce_count = exact_spotted_surface(
        "taskforce",
        &spotted.taskforce_units,
        &spotted.fleets,
        &spotted.taskforce_frame_sequence,
    )?;
    if spotted_land_unit_count.is_none() && spotted_taskforce_count.is_none() {
        return Err("exact d95 carries no complete land or taskforce frame".into());
    }

    Ok(PovRenderStatus::Recorded {
        pov: pov.to_string(),
        province_slot_count: levels.len(),
        visibility_capture_sequence: *visibility_capture_sequence,
        pov_country_id: *pov_country_id,
        spotted_land_unit_count,
        spotted_taskforce_count,
    })
}

fn exact_spotted_surface<T>(
    label: &str,
    rows: &Leaf<Vec<T>>,
    handles: &Leaf<Vec<String>>,
    sequence: &Leaf<u64>,
) -> Result<Option<usize>, String> {
    match (rows, handles, sequence) {
        (Leaf::Measured(rows), Leaf::Measured(handles), Leaf::Measured(sequence))
            if *sequence != 0 && rows.len() == handles.len() =>
        {
            Ok(Some(rows.len()))
        }
        (Leaf::Unobtained, Leaf::Unobtained, Leaf::Unobtained) => Ok(None),
        _ => Err(format!(
            "exact d95 {label} rows, handles, and nonzero frame sequence are incomplete"
        )),
    }
}

/// Build the country picker from the session view. Fails closed to
/// [`PovPicker::Unavailable`] when the player-country list is absent,
/// unobtained, or present-but-empty — the picker never invents countries and
/// never parses behind the `replay_core` seam.
pub fn povs_from_session(session: &SessionMetadataView) -> PovPicker {
    match &session.player_countries {
        Leaf::Measured(list) => {
            let mut choices = Vec::new();
            for pc in list {
                // Only fully-identified players are selectable POVs.
                if let Leaf::Measured(tag) = &pc.tag {
                    choices.push(CountryChoice {
                        tag: tag.clone(),
                        label: pc.player_name.measured().cloned(),
                    });
                }
            }
            if choices.is_empty() {
                PovPicker::Unavailable(
                    "session player-country list present but no measured TAGs".to_string(),
                )
            } else {
                PovPicker::Available(choices)
            }
        }
        Leaf::Unobtained => PovPicker::Unavailable(
            "session player-country list captured-but-unobtained (null/malformed)".to_string(),
        ),
        Leaf::Absent => PovPicker::Unavailable(SESSION_PLAYERS_GAP.to_string()),
    }
}

/// Build the production observer-country picker from the complete d18 roster.
///
/// This selection controls replay-owned panels only. Choosing a country here
/// does not request fog and therefore does not require d94/d95 evidence.
pub fn povs_from_frame(frame: &WorldFrame) -> PovPicker {
    let roster = frame.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return PovPicker::Unavailable(
            roster
                .diagnostic
                .unwrap_or_else(|| "complete d18 country roster unavailable".into()),
        );
    }
    if roster.entries.is_empty() {
        return PovPicker::Unavailable("complete d18 country roster is empty".into());
    }
    PovPicker::Available(
        roster
            .entries
            .into_iter()
            .map(|country| CountryChoice {
                tag: country.tag,
                label: None,
            })
            .collect(),
    )
}

#[cfg(test)]
fn povs_from_session_and_header(
    session: &SessionMetadataView,
    header: &SessionHeaderView,
) -> PovPicker {
    let session_picker = povs_from_session(session);
    match (
        &session.player_countries,
        &session_picker,
        &header.player,
    ) {
        (_, PovPicker::Available(choices), Leaf::Measured(player))
            if !choices.iter().any(|choice| choice.tag == player.country_tag) =>
        {
            PovPicker::Unavailable(format!(
                "D68 local player {} is absent from the measured D57 player-country roster",
                player.country_tag
            ))
        }
        (_, PovPicker::Available(_), _) => session_picker,
        (Leaf::Absent, PovPicker::Unavailable(_), Leaf::Measured(player)) => {
            PovPicker::Available(vec![CountryChoice {
                tag: player.country_tag.clone(),
                label: None,
            }])
        }
        (_, PovPicker::Unavailable(_), _) => session_picker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{
        PlayerCountry, ProvinceVisibility, SessionHeaderPlayer, SessionHeaderView,
    };
    use std::collections::BTreeMap;

    fn session(player_countries: Leaf<Vec<PlayerCountry>>) -> SessionMetadataView {
        SessionMetadataView {
            status: ViewStatus::Present,
            player_countries,
            speed_events: Leaf::Absent,
            active_dlcs: Leaf::Absent,
            localisation_language: Leaf::Absent,
            display_variant: Leaf::Absent,
        }
    }

    #[test]
    fn default_is_omniscient() {
        let state = PovState::new();
        assert!(state.is_omniscient());
        assert_eq!(state.active_pov(), None);
        assert_eq!(state.selected_ui_country(), None);
        assert_eq!(state.render_status(None), PovRenderStatus::Omniscient);
    }

    #[test]
    fn ui_country_selection_does_not_request_fog() {
        let mut state = PovState::new();
        state.select_ui_country("FRA");
        assert_eq!(state.selected_ui_country(), Some("FRA"));
        assert_eq!(state.active_pov(), None);
        assert_eq!(state.render_status(None), PovRenderStatus::Omniscient);

        state.toggle();
        assert_eq!(state.active_pov(), Some("FRA"));
        assert!(matches!(
            state.render_status(None),
            PovRenderStatus::Unavailable { .. }
        ));
        state.toggle();
        assert_eq!(state.active_pov(), None);
        assert_eq!(state.selected_ui_country(), Some("FRA"));
    }

    #[test]
    fn enter_pov_without_world_frame_fails_closed() {
        let mut state = PovState::new();
        state.enter_pov("GER");
        assert_eq!(state.active_pov(), Some("GER"));
        match state.render_status(None) {
            PovRenderStatus::Unavailable { pov, diagnostic } => {
                assert_eq!(pov, "GER");
                assert!(diagnostic.contains(POV_RECORDED_STATE_UNAVAILABLE));
                assert!(diagnostic.contains("WorldFrame is absent"));
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    fn session_header(player: Leaf<SessionHeaderPlayer>) -> SessionHeaderView {
        SessionHeaderView {
            status: if player.is_measured() {
                ViewStatus::Present
            } else {
                ViewStatus::DomainAbsent
            },
            player,
            ideology: Leaf::Absent,
            cosmetic_tag: Leaf::Absent,
            dlc_mask: Leaf::Absent,
            start_date_raw: Leaf::Absent,
            save_version: Leaf::Absent,
            session: Leaf::Absent,
            version: Leaf::Absent,
        }
    }

    #[test]
    fn exact_d94_d95_views_admit_selected_pov() {
        let spotted = SpottedUnitsView {
            status: ViewStatus::Present,
            divisions: Leaf::Measured(Vec::new()),
            fleets: Leaf::Unobtained,
            wings: Leaf::Unobtained,
            land_units: Leaf::Measured(Vec::new()),
            taskforce_units: Leaf::Unobtained,
            land_frame_sequence: Leaf::Measured(9),
            taskforce_frame_sequence: Leaf::Unobtained,
            // An observation of this very hour. POV admission turns on the
            // vector and the sequences, not on how old the frame is: a
            // retained frame is still a real observation and must still admit.
            land_frame_age_hours: Leaf::Measured(0),
            taskforce_frame_age_hours: Leaf::Unobtained,
        };
        let visibility = PovVisibilityView {
            status: ViewStatus::Present,
            provinces: BTreeMap::from([(
                0,
                Leaf::Measured(ProvinceVisibility {
                    level: "2".into(),
                }),
            )]),
            raw_province_levels: Leaf::Measured(vec![2]),
            capture_sequence: Leaf::Measured(7),
            pov_country_id: Leaf::Measured(42),
            producer_hour_index: Leaf::Measured(1_000),
            producer_frame_age_hours: Leaf::Measured(0),
            spotted_units: spotted.clone(),
        };

        assert_eq!(
            admit_recorded_pov("GER", &visibility, &spotted),
            Ok(PovRenderStatus::Recorded {
                pov: "GER".into(),
                province_slot_count: 1,
                visibility_capture_sequence: 7,
                pov_country_id: 42,
                spotted_land_unit_count: Some(0),
                spotted_taskforce_count: None,
            })
        );
    }

    #[test]
    fn present_but_nonexact_visibility_cannot_admit_selected_pov() {
        let spotted = SpottedUnitsView {
            status: ViewStatus::Present,
            divisions: Leaf::Measured(Vec::new()),
            fleets: Leaf::Unobtained,
            wings: Leaf::Unobtained,
            land_units: Leaf::Measured(Vec::new()),
            taskforce_units: Leaf::Unobtained,
            land_frame_sequence: Leaf::Measured(1),
            taskforce_frame_sequence: Leaf::Unobtained,
            land_frame_age_hours: Leaf::Measured(0),
            taskforce_frame_age_hours: Leaf::Unobtained,
        };
        let visibility = PovVisibilityView {
            status: ViewStatus::Present,
            provinces: BTreeMap::new(),
            raw_province_levels: Leaf::Absent,
            capture_sequence: Leaf::Absent,
            pov_country_id: Leaf::Absent,
            // The nonexact A3 shape carries no authority block, so it makes no
            // observation-time claim either.
            producer_hour_index: Leaf::Absent,
            producer_frame_age_hours: Leaf::Absent,
            spotted_units: spotted.clone(),
        };
        assert!(admit_recorded_pov("GER", &visibility, &spotted)
            .expect_err("legacy visibility must not be admitted")
            .contains("d94 province visibility vector"));
    }

    #[test]
    fn enter_pov_is_idempotent() {
        let mut a = PovState::new();
        a.enter_pov("SOV");
        let mut b = a.clone();
        b.enter_pov("SOV");
        assert_eq!(a, b);
    }

    #[test]
    fn toggle_is_stateless_and_idempotent_under_pairing() {
        // From every reachable state, toggle();toggle(); is a no-op.
        let mut fresh = PovState::new();
        let before = fresh.clone();
        fresh.toggle();
        fresh.toggle();
        assert_eq!(fresh, before, "fresh (no POV ever) toggles are no-ops");

        let mut in_pov = PovState::new();
        in_pov.enter_pov("ENG");
        let before = in_pov.clone();
        in_pov.toggle(); // -> world
        assert!(in_pov.is_omniscient());
        in_pov.toggle(); // -> back to ENG
        assert_eq!(in_pov, before);

        // World-with-remembered-POV round-trips too.
        let mut world_after = PovState::new();
        world_after.enter_pov("ENG");
        world_after.exit_to_world();
        let before = world_after.clone();
        world_after.toggle(); // -> ENG
        assert_eq!(world_after.active_pov(), Some("ENG"));
        world_after.toggle(); // -> world
        assert_eq!(world_after, before);
    }

    #[test]
    fn picker_available_keeps_measured_tags_and_labels() {
        let list = vec![
            PlayerCountry {
                tag: Leaf::Measured("GER".into()),
                player_name: Leaf::Measured("Alice".into()),
            },
            PlayerCountry {
                tag: Leaf::Measured("SOV".into()),
                player_name: Leaf::Absent,
            },
            // Unmeasured tag → dropped (never a fabricated POV).
            PlayerCountry {
                tag: Leaf::Unobtained,
                player_name: Leaf::Measured("Ghost".into()),
            },
        ];
        match povs_from_session(&session(Leaf::Measured(list))) {
            PovPicker::Available(choices) => {
                assert_eq!(choices.len(), 2);
                assert_eq!(choices[0].tag, "GER");
                assert_eq!(choices[0].label.as_deref(), Some("Alice"));
                assert_eq!(choices[1].tag, "SOV");
                assert_eq!(choices[1].label, None);
            }
            other => panic!("expected Available, got {other:?}"),
        }
    }

    #[test]
    fn picker_fails_closed_when_absent() {
        match povs_from_session(&session(Leaf::Absent)) {
            PovPicker::Unavailable(reason) => assert!(reason.contains("world.scenario.players")),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        match povs_from_session(&session(Leaf::Unobtained)) {
            PovPicker::Unavailable(reason) => assert!(reason.contains("captured-but-unobtained")),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        match povs_from_session(&session(Leaf::Measured(vec![]))) {
            PovPicker::Unavailable(reason) => assert!(reason.contains("no measured TAGs")),
            other => panic!("expected Unavailable, got {other:?}"),
        }
    }

    #[test]
    fn picker_uses_exact_d68_player_when_multiplayer_roster_is_absent() {
        let header = session_header(Leaf::Measured(SessionHeaderPlayer {
            tag_db_id: 368,
            country_tag: "D04".into(),
        }));
        match povs_from_session_and_header(&session(Leaf::Absent), &header) {
            PovPicker::Available(choices) => {
                assert_eq!(choices.len(), 1);
                assert_eq!(choices[0].tag, "D04");
                assert_eq!(choices[0].label, None);
            }
            other => panic!("expected D68 single-player POV, got {other:?}"),
        }
    }

    #[test]
    fn picker_does_not_hide_an_unobtained_d57_roster_with_d68() {
        let header = session_header(Leaf::Measured(SessionHeaderPlayer {
            tag_db_id: 368,
            country_tag: "D04".into(),
        }));
        match povs_from_session_and_header(&session(Leaf::Unobtained), &header) {
            PovPicker::Unavailable(reason) => {
                assert!(reason.contains("captured-but-unobtained"));
            }
            other => panic!("expected unavailable D57 roster to stay fail-closed, got {other:?}"),
        }
    }

    #[test]
    fn picker_rejects_d57_d68_player_disagreement() {
        let header = session_header(Leaf::Measured(SessionHeaderPlayer {
            tag_db_id: 368,
            country_tag: "D04".into(),
        }));
        let roster = vec![PlayerCountry {
            tag: Leaf::Measured("GER".into()),
            player_name: Leaf::Measured("Alice".into()),
        }];
        match povs_from_session_and_header(&session(Leaf::Measured(roster)), &header) {
            PovPicker::Unavailable(reason) => {
                assert!(reason.contains("D68 local player D04"));
                assert!(reason.contains("D57 player-country roster"));
            }
            other => panic!("expected fail-closed D57/D68 disagreement, got {other:?}"),
        }
    }
}
