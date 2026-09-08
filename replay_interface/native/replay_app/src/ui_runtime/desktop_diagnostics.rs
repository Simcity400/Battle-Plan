//! Immutable access for the opt-in desktop diagnostic producer. Admission,
//! transactions and the reached input consumers retain their existing owners.
use super::ProductObserverUiRuntime;
use crate::Result;
use game_ui::production_renderer::ObserverViewerUiState;
use game_ui::window_manager_law::{
    AuthenticatedReplayTooltipOwner, ReplayPointerTarget, ReplayWindowManagerSnapshot,
};

impl ProductObserverUiRuntime {
    pub(crate) fn desktop_snapshot(&self) -> Result<Option<ReplayWindowManagerSnapshot>> {
        self.authored_observer_ui
            .as_ref()
            .map(|authored| authored.manager().composed_snapshot().map_err(Into::into))
            .transpose()
    }

    /// Called only after the actual Presented disposition. Use the same higher
    /// owners and the real manager hit test; never reconstruct stack ordering.
    pub(crate) fn desktop_pointer_target(
        &self,
        viewer: &ObserverViewerUiState,
        x: i32,
        y: i32,
    ) -> Result<Option<ReplayPointerTarget>> {
        // The reached MouseMove route lets an active pan own points outside its
        // panel too. Reuse its matching-surface predicate without moving the
        // cursor or changing capture. A rectangle-only observation is weaker.
        let research_drag_owns = self.xui09_drag_cursor.is_some()
            && super::xui09_matching_presented_surface(
                viewer.research.panel_open,
                &viewer.selected_country,
                self.xui09_presented.as_ref(),
            )
            .is_some();
        if research_drag_owns
            || self.xui10_drag_cursor.is_some()
            || self.xui21_blocks_lower_pointer_input(viewer, x, y)
            || self.xui09_blocks_lower_pointer_input(viewer, x, y)
            || self.xui10_blocks_lower_pointer_input(x, y)
            || self.xui23_blocks_lower_pointer_input(x, y)
            || self.xui11_blocks_lower_pointer_input(x, y)
            || self.xui18_blocks_lower_pointer_input(viewer, x, y)
            || self.xui19_blocks_lower_pointer_input(viewer, x, y)
            || self.xui08_blocks_lower_pointer_input(viewer, x, y)
            || self.xui14_blocks_lower_pointer_input(viewer, x, y)
            || self.xui20_blocks_lower_pointer_input(x, y)
            || self.xui15_blocks_lower_pointer_input(x, y)?
            || self.xui15_has_pointer_capture(viewer)
        {
            return Ok(None);
        }
        let Some(authored) = &self.authored_observer_ui else {
            return Ok(None);
        };
        Ok(authored.manager().pointer_target_at(x, y)?)
    }

    /// Set only after the Visible tooltip was successfully blended. The caller
    /// copies this candidate before Present and may publish it only after the
    /// normal Present/owner commit has succeeded and confirms the same owner.
    pub(crate) fn desktop_pending_tooltip_owner(
        &self,
    ) -> Option<&(AuthenticatedReplayTooltipOwner, String)> {
        self.xui05_pending_owner.as_ref()
    }
}
