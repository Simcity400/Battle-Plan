
use replay_core::{
    Leaf, PovVisibilityView, SessionMetadataView, SpottedUnitsView, ViewStatus, WorldFrame,
};
#[cfg(test)]
use replay_core::SessionHeaderView;

pub const POV_RECORDED_STATE_UNAVAILABLE: &str =
    "selected POV has no complete current-frame d94/d95 visibility and spotted-unit evidence";

pub const SESSION_PLAYERS_GAP: &str = "session player-country list unavailable: this recording \
     carries no player-country list under session.player_countries or \
     world.scenario.players (replay_core SessionMetadataView.player_countries \
     is Absent); country picker fails closed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverMode {
    Omniscient,
    Pov(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryChoice {
    pub tag: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovPicker {
    /// The countries this frame can be watched as, PLUS the country keys it
    /// carries that this hour's D18 read could not name.
    ///
    /// Both halves, always. An unnameable country is not a point of view this
    /// recording offers — it has no TAG to select and no per-country view to
    /// show — but the picker must not therefore present its list as the whole
    /// country set. Callers describe the list to the user and clear a selection
    /// that is absent from it, and both of those become false statements if
    /// "not offered" and "does not exist" are allowed to look the same.
    Available {
        choices: Vec<CountryChoice>,
        unnameable: Vec<String>,
    },
    Unavailable(String),
}

impl PovPicker {
    /// The offered choices, or an empty slice.
    pub fn choices(&self) -> &[CountryChoice] {
        match self {
            PovPicker::Available { choices, .. } => choices,
            PovPicker::Unavailable(_) => &[],
        }
    }

    /// The country keys this frame carries that it cannot offer as a point of
    /// view, because this hour's D18 read could not name them.
    pub fn unnameable(&self) -> &[String] {
        match self {
            PovPicker::Available { unnameable, .. } => unnameable,
            PovPicker::Unavailable(_) => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovRenderStatus {
    Omniscient,
    Recorded {
        pov: String,
        province_slot_count: usize,
        visibility_capture_sequence: u64,
        pov_country_id: u32,
        spotted_land_unit_count: Option<usize>,
        spotted_taskforce_count: Option<usize>,
    },
    Unavailable {
        pov: String,
        diagnostic: String,
    },
}

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
    pub fn selected_ui_country(&self) -> Option<&str> {
        self.selected_ui_country.as_deref()
    }
    pub fn select_ui_country(&mut self, tag: &str) {
        self.selected_ui_country = Some(tag.to_string());
    }
    pub fn clear_country_selection(&mut self) {
        self.selected_ui_country = None;
        self.mode = ObserverMode::Omniscient;
        self.last_pov = None;
    }
    pub fn is_omniscient(&self) -> bool {
        matches!(self.mode, ObserverMode::Omniscient)
    }
    pub fn active_pov(&self) -> Option<&str> {
        match &self.mode {
            ObserverMode::Pov(tag) => Some(tag),
            ObserverMode::Omniscient => None,
        }
    }
    pub fn last_pov(&self) -> Option<&str> {
        self.last_pov.as_deref()
    }

    pub fn enter_pov(&mut self, tag: &str) {
        self.selected_ui_country = Some(tag.to_string());
        self.mode = ObserverMode::Pov(tag.to_string());
        self.last_pov = Some(tag.to_string());
    }

    pub fn exit_to_world(&mut self) {
        self.mode = ObserverMode::Omniscient;
    }

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

pub fn povs_from_session(session: &SessionMetadataView) -> PovPicker {
    match &session.player_countries {
        Leaf::Measured(list) => {
            let mut choices = Vec::new();
            for pc in list {
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
                // The session player-country list is not the D18 roster and
                // says nothing about unnameable countries.
                PovPicker::Available {
                    choices,
                    unnameable: Vec::new(),
                }
            }
        }
        Leaf::Unobtained => PovPicker::Unavailable(
            "session player-country list captured-but-unobtained (null/malformed)".to_string(),
        ),
        Leaf::Absent => PovPicker::Unavailable(SESSION_PLAYERS_GAP.to_string()),
    }
}

pub fn povs_from_frame(frame: &WorldFrame) -> PovPicker {
    povs_from_roster(&frame.d18_country_roster())
}

/// Split out from [`povs_from_frame`] so the roster half can be tested
/// directly: `WorldFrame` cannot be constructed outside `replay_core`, and the
/// property that matters here — that the countries this recording could not
/// name are CARRIED rather than dropped — is entirely a property of the roster.
fn povs_from_roster(roster: &replay_core::D18CountryRosterView) -> PovPicker {
    // PARTIAL: this is the list of countries a viewer may select as a point of
    // view. A country the recording could not name has no TAG to select and no
    // per-country view to show, so it is not a POV this recording offers -
    // while every country that WAS named still is. Refusing the whole picker
    // over it would leave the viewer with no point of view at all.
    //
    // What makes that honest rather than a shorter list pretending to be whole:
    // the unnameable keys ride the picker, and the caller both says so and uses
    // them to explain a cleared selection.
    let partial = match roster.partial() {
        Ok(partial) => partial,
        Err(refusal) => return PovPicker::Unavailable(refusal.detail),
    };
    PovPicker::Available {
        choices: partial
            .resolved
            .iter()
            .map(|country| CountryChoice {
                tag: country.tag.clone(),
                label: None,
            })
            .collect(),
        unnameable: partial.excluded.to_vec(),
    }
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
        // A local player whose TAG the census WITHHELD carries no name to
        // match a choice against, and no name to offer as one. The picker is
        // a list of countries to look through the eyes of, so a key two ids
        // reach cannot be on it -- and the D68 row still names the player by
        // `tag_db_id`, which is why the row itself is kept.
        (_, PovPicker::Available { .. }, Leaf::Measured(player))
            if player.country_tag.is_none() =>
        {
            PovPicker::Unavailable(
                "the D68 local player is a country key more than one id reaches, \
                 so it names no country to view from"
                    .into(),
            )
        }
        (_, PovPicker::Available { choices, .. }, Leaf::Measured(player))
            if !choices
                .iter()
                .any(|choice| Some(&choice.tag) == player.country_tag.as_ref()) =>
        {
            PovPicker::Unavailable(format!(
                "D68 local player {:?} is absent from the measured D57 player-country roster",
                player.country_tag.as_deref().unwrap_or_default()
            ))
        }
        (_, PovPicker::Available { .. }, _) => session_picker,
        (Leaf::Absent, PovPicker::Unavailable(_), Leaf::Measured(player)) => {
            match &player.country_tag {
                Some(tag) => PovPicker::Available {
                    choices: vec![CountryChoice {
                        tag: tag.clone(),
                        label: None,
                    }],
                    unnameable: Vec::new(),
                },
                None => PovPicker::Unavailable(
                    "the D68 local player is a country key more than one id \
                     reaches, so it names no country to view from"
                        .into(),
                ),
            }
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

    /// kills: dropping the unnameable countries from the POV picker, which is
    /// what would let its caller keep describing the list as "all countries"
    /// and keep treating absence from it as "this country does not exist".
    ///
    /// The list of OFFERED points of view is correctly the resolved set — an
    /// unnameable country has no TAG to select — so the defect this guards
    /// against is not the list's contents but the silence about what is missing
    /// from it.
    ///
    /// What production change would make this fail: building the picker from
    /// `partial.resolved` alone and discarding `partial.excluded`, or switching
    /// the whole picker back to `complete()` (the first assertion's picker would
    /// become Unavailable and the viewer would lose all 2 points of view over
    /// one country).
    #[test]
    fn the_pov_picker_offers_the_named_countries_and_carries_the_rest() {
        let entry = |tag: &str, id: u32| replay_core::D18CountryRosterEntry {
            tag: tag.into(),
            tag_db_id: id,
        };

        let complete = replay_core::D18CountryRosterView::present(
            vec![entry("ENG", 1), entry("GER", 2)],
            Vec::new(),
        );
        let picker = povs_from_roster(&complete);
        assert_eq!(picker.choices().len(), 2);
        assert!(picker.unnameable().is_empty());

        let partial = replay_core::D18CountryRosterView::present(
            vec![entry("ENG", 1), entry("GER", 2)],
            vec!["367".into()],
        );
        let picker = povs_from_roster(&partial);
        assert_eq!(
            picker
                .choices()
                .iter()
                .map(|choice| choice.tag.as_str())
                .collect::<Vec<_>>(),
            vec!["ENG", "GER"],
            "every country this hour could name is still offered"
        );
        assert_eq!(
            picker.unnameable(),
            ["367".to_string()],
            "the country it could not name must be carried, not dropped"
        );

        // A roster that resolved nothing offers nothing, and says why.
        let nothing = replay_core::D18CountryRosterView::refused(
            ViewStatus::DomainUnavailable,
            "no emitted country key carries a d18 identity this hour",
        );
        match povs_from_roster(&nothing) {
            PovPicker::Unavailable(reason) => assert!(reason.contains("d18 identity")),
            PovPicker::Available { .. } => panic!("an empty roster must not offer a point of view"),
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
        let mut fresh = PovState::new();
        let before = fresh.clone();
        fresh.toggle();
        fresh.toggle();
        assert_eq!(fresh, before, "fresh (no POV ever) toggles are no-ops");

        let mut in_pov = PovState::new();
        in_pov.enter_pov("ENG");
        let before = in_pov.clone();
        in_pov.toggle();
        assert!(in_pov.is_omniscient());
        in_pov.toggle();
        assert_eq!(in_pov, before);

        let mut world_after = PovState::new();
        world_after.enter_pov("ENG");
        world_after.exit_to_world();
        let before = world_after.clone();
        world_after.toggle();
        assert_eq!(world_after.active_pov(), Some("ENG"));
        world_after.toggle();
        assert_eq!(world_after, before);
    }

    #[test]
    fn picker_available_keeps_measured_tags_and_labels() {
        let list = vec![
            PlayerCountry {
                tag: Leaf::Measured("GER".into()),
                tag_db_id: Leaf::Absent,
                player_name: Leaf::Measured("Alice".into()),
            },
            PlayerCountry {
                tag: Leaf::Measured("SOV".into()),
                tag_db_id: Leaf::Absent,
                player_name: Leaf::Absent,
            },
            PlayerCountry {
                tag: Leaf::Unobtained,
                tag_db_id: Leaf::Absent,
                player_name: Leaf::Measured("Ghost".into()),
            },
        ];
        match povs_from_session(&session(Leaf::Measured(list))) {
            PovPicker::Available { choices, .. } => {
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
            country_tag: Some("D04".into()),
        }));
        match povs_from_session_and_header(&session(Leaf::Absent), &header) {
            PovPicker::Available { choices, .. } => {
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
            country_tag: Some("D04".into()),
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
            country_tag: Some("D04".into()),
        }));
        let roster = vec![PlayerCountry {
            tag: Leaf::Measured("GER".into()),
            tag_db_id: Leaf::Absent,
            player_name: Leaf::Measured("Alice".into()),
        }];
        match povs_from_session_and_header(&session(Leaf::Measured(roster)), &header) {
            PovPicker::Unavailable(reason) => {
                assert!(reason.contains("D68 local player \"D04\""));
                assert!(reason.contains("D57 player-country roster"));
            }
            other => panic!("expected fail-closed D57/D68 disagreement, got {other:?}"),
        }
    }
}
