//! E6 Observer POV mode UX (SHELL ONLY).
//!
//! Product model (REPLAY_PRODUCTION_PLAN §0 / observer-mode directive): the
//! default view is the omniscient world (no fog). The viewer may pick one of
//! the session's player countries and enter its POV; a single key toggles back
//! to the world. This module owns:
//!
//! * [`PovState`] — the POV state object + a stateless, idempotent toggle.
//! * [`povs_from_session`] — the country picker source, derived from the
//!   `replay_core` session view (`SessionMetadataView.player_countries`).
//! * [`PovState::render_status`] — the fail-closed render contract: because
//!   the FoW *render* layer is only law-certified as tooling so far (plan C8),
//!   POV entry surfaces [`FOW_NOT_LAW_CERTIFIED`] and keeps showing the
//!   omniscient view rather than faking a fog look. Hidden-unit / UI re-keying
//!   is a later lane.
//!
//! The POV *state* still flips on entry (so the UI can show which country is
//! observed and offer the toggle); only the fog RENDER is blocked.
//!
//! replay_core API GAP (recorded in LANE_REPORT.md): the session view's
//! `player_countries` reads a top-level `session.player_countries`, which the
//! current snapshot-flavor recordings do not carry — their player list lives
//! under `world.scenario.players`, which the typed view does not yet surface.
//! So [`povs_from_session`] returns [`PovPicker::Unavailable`] on today's
//! fixtures, and the picker fails closed with [`SESSION_PLAYERS_GAP`] instead
//! of parsing the raw snapshot behind the seam.

use replay_core::{Leaf, SessionMetadataView};

/// Fail-closed diagnostic surfaced when a country POV is active: the FoW
/// render layer is not yet certified for the product path.
pub const FOW_NOT_LAW_CERTIFIED: &str =
    "observer POV FoW layer not yet law-certified (plan C8 is tooling-only); \
     showing the omniscient world view, not a faked fog look";

/// Fail-closed diagnostic surfaced when the recording carries no session
/// player-country list the picker can use (see the module note / LANE_REPORT).
pub const SESSION_PLAYERS_GAP: &str = "session player-country list unavailable: replay_core \
     SessionMetadataView.player_countries is Absent for this recording (the \
     player list lives under world.scenario.players, which the session view \
     does not yet surface); country picker fails closed";

/// Observer view mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverMode {
    /// Default omniscient world view (no fog).
    Omniscient,
    /// Observing as a player country (by TAG).
    Pov(String),
}

/// One selectable player country in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryChoice {
    pub tag: String,
    /// Human player label when the recording carried one.
    pub label: Option<String>,
}

/// The country picker's source state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovPicker {
    /// One or more selectable player countries.
    Available(Vec<CountryChoice>),
    /// No usable player-country list; carries the fail-closed reason.
    Unavailable(String),
}

/// What the map should render for the current POV state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PovRenderStatus {
    /// Omniscient world view — render normally.
    Omniscient,
    /// A country POV is active but the FoW render is blocked; render the
    /// omniscient view and surface the diagnostic. Never fake fog.
    FowBlocked {
        pov: String,
        diagnostic: &'static str,
    },
}

/// The POV state object. Holds the current mode plus the last-entered POV so
/// the one-key toggle can flip back to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PovState {
    mode: ObserverMode,
    last_pov: Option<String>,
}

impl Default for PovState {
    fn default() -> Self {
        PovState {
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
            ObserverMode::Omniscient => match &self.last_pov {
                Some(tag) => ObserverMode::Pov(tag.clone()),
                None => ObserverMode::Omniscient,
            },
            ObserverMode::Pov(_) => ObserverMode::Omniscient,
        };
    }

    /// The render contract for the current mode (fail-closed on POV).
    pub fn render_status(&self) -> PovRenderStatus {
        match &self.mode {
            ObserverMode::Omniscient => PovRenderStatus::Omniscient,
            ObserverMode::Pov(tag) => PovRenderStatus::FowBlocked {
                pov: tag.clone(),
                diagnostic: FOW_NOT_LAW_CERTIFIED,
            },
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{PlayerCountry, ViewStatus};

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
        assert_eq!(state.render_status(), PovRenderStatus::Omniscient);
    }

    #[test]
    fn enter_pov_flips_state_but_render_fails_closed() {
        let mut state = PovState::new();
        state.enter_pov("GER");
        assert_eq!(state.active_pov(), Some("GER"));
        // State flips, but the FoW render is blocked (no faked fog).
        match state.render_status() {
            PovRenderStatus::FowBlocked { pov, diagnostic } => {
                assert_eq!(pov, "GER");
                assert_eq!(diagnostic, FOW_NOT_LAW_CERTIFIED);
                assert!(diagnostic.contains("not yet law-certified"));
            }
            other => panic!("expected FowBlocked, got {other:?}"),
        }
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
}
