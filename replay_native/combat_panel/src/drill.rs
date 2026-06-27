//! Bubble-click → combat-panel dispatch (mirrors the app `drill` registry pattern,
//! kept here so the adapter is self-contained + headlessly testable). A click is
//! classified into a [`CombatHit`]; [`CombatDrill`] maps it to a [`PanelAction`].
//!
//! Routing is DATA, not a match arm in the event loop, so the app shell registers
//! these rules alongside its Stage-1/Stage-3 rules without touching the dispatcher.

/// The KIND of thing a click resolved to, in combat-panel terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CombatHit {
    /// A combat BUBBLE on the map was clicked (carries its `combat_id`).
    Bubble { combat_id: String },
    /// A named GUI control (e.g. the panel's close button).
    Control { name: String },
    /// Anything else (handled elsewhere / inert here).
    Other,
}

/// What the shell should DO with the combat panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelAction {
    /// Open (or replace) the stacked combat panel for this combat.
    OpenCombatPanel { combat_id: String },
    /// Close the open combat panel.
    CloseCombatPanel,
    /// Nothing.
    None,
}

/// The combat drill registry: ordered rules, first match wins.
pub struct CombatDrill {
    close_names: Vec<String>,
}

impl CombatDrill {
    /// Build with the leaf names that CLOSE the panel (the landcombat view's real
    /// `close` button). Passing them in keeps the registry `.gui`-naming-agnostic.
    pub fn new(close_control_names: Vec<String>) -> CombatDrill {
        CombatDrill { close_names: close_control_names }
    }

    /// Dispatch a classified hit to a panel action.
    pub fn dispatch(&self, hit: &CombatHit) -> PanelAction {
        match hit {
            // bubble click -> open the stacked combat panel.
            CombatHit::Bubble { combat_id } => {
                PanelAction::OpenCombatPanel { combat_id: combat_id.clone() }
            }
            // a close control -> close the panel.
            CombatHit::Control { name } if self.close_names.iter().any(|n| n == name) => {
                PanelAction::CloseCombatPanel
            }
            // any other control / empty -> inert.
            _ => PanelAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubble_click_opens_panel() {
        let d = CombatDrill::new(vec!["close".into()]);
        assert_eq!(
            d.dispatch(&CombatHit::Bubble { combat_id: "5457:62".into() }),
            PanelAction::OpenCombatPanel { combat_id: "5457:62".into() }
        );
    }

    #[test]
    fn close_control_closes_panel() {
        let d = CombatDrill::new(vec!["close".into(), "X".into()]);
        assert_eq!(
            d.dispatch(&CombatHit::Control { name: "close".into() }),
            PanelAction::CloseCombatPanel
        );
        assert_eq!(
            d.dispatch(&CombatHit::Control { name: "X".into() }),
            PanelAction::CloseCombatPanel
        );
    }

    #[test]
    fn unknown_control_and_other_are_inert() {
        let d = CombatDrill::new(vec!["close".into()]);
        assert_eq!(
            d.dispatch(&CombatHit::Control { name: "tactics_list_button".into() }),
            PanelAction::None,
            "tactics picker is a mutation verb -> inert in replay"
        );
        assert_eq!(d.dispatch(&CombatHit::Other), PanelAction::None);
    }
}
