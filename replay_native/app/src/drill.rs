//! SELECT → DRILL-IN dispatch, as a REGISTRY (Stage 1 deliverable 6).
//!
//! A click is first classified into a [`HitTarget`] — the *kind* of thing that was
//! hit (a province on the map; a named GUI control; the scrubber; empty space). The
//! [`DrillRegistry`] maps a `HitTarget` to a [`DrillAction`] — what the app should do
//! (open the state panel for a state; close a panel; nothing). Routing is DATA, not a
//! match arm buried in the event loop, so later views plug in by registering a new
//! rule (e.g. a unit stack → open the division view) without touching the dispatcher.
//!
//! Keeping classification + routing PURE (no GPU, no Timeline) makes the whole
//! select→drill path headlessly testable: feed a `HitTarget`, assert the `DrillAction`.
//! The app then EXECUTES the action against its real state (open a window, etc.).

/// The KIND of thing a click resolved to (the registry's key).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    /// The map was clicked and the pick resolved to a land state. `state_id` is the
    /// picked state; `province_id` the exact province (for diagnostics).
    Province { province_id: u32, state_id: u32 },
    /// The map was clicked but the pick hit sea / unowned / no-state (no drill).
    MapNeutral { province_id: u32 },
    /// A named GUI control was clicked (the leaf element name from the hit path).
    Control { name: String },
    /// The scrubber track was pressed (transport, handled by the scrubber itself).
    Scrubber,
    /// Empty space / nothing actionable.
    Empty,
}

/// What the app should DO in response to a classified hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DrillAction {
    /// Open (or replace) the state panel bound to this state.
    OpenStatePanel { state_id: u32 },
    /// Close the currently open state panel.
    CloseStatePanel,
    /// No drill action (the hit is handled elsewhere or is inert).
    None,
}

/// A single routing rule: a predicate over a [`HitTarget`] → an action. Rules are
/// tried in registration order; the first match wins.
type Rule = Box<dyn Fn(&HitTarget) -> Option<DrillAction> + Send + Sync>;

/// The drill registry — an ordered list of routing rules. The app builds one at
/// startup with the Stage-1 rules; future views add rules here.
pub struct DrillRegistry {
    rules: Vec<Rule>,
}

impl DrillRegistry {
    /// An EMPTY registry (no rules → everything routes to `None`). Use
    /// [`with_stage1_rules`](Self::with_stage1_rules) for the default wiring.
    pub fn new() -> DrillRegistry {
        DrillRegistry { rules: Vec::new() }
    }

    /// Register a routing rule (tried after all previously-registered rules).
    pub fn register<F>(&mut self, rule: F)
    where
        F: Fn(&HitTarget) -> Option<DrillAction> + Send + Sync + 'static,
    {
        self.rules.push(Box::new(rule));
    }

    /// The default Stage-1 wiring:
    ///   * a `Province` pick opens the state panel for that state,
    ///   * a `Control` named like a close button closes the state panel,
    ///   * everything else → `None`.
    /// `close_control_names` are the leaf names that close the panel (the panel's
    /// real close button / "X"). Passing them in (rather than hardcoding) keeps the
    /// registry independent of any one `.gui`'s naming.
    pub fn with_stage1_rules(close_control_names: Vec<String>) -> DrillRegistry {
        let mut reg = DrillRegistry::new();
        // province pick -> open the state panel.
        reg.register(|t| match t {
            HitTarget::Province { state_id, .. } => Some(DrillAction::OpenStatePanel {
                state_id: *state_id,
            }),
            _ => None,
        });
        // a close control -> close the panel.
        reg.register(move |t| match t {
            HitTarget::Control { name } if close_control_names.iter().any(|n| n == name) => {
                Some(DrillAction::CloseStatePanel)
            }
            _ => None,
        });
        reg
    }

    /// Dispatch a classified hit to its action. The first matching rule wins; no
    /// match → [`DrillAction::None`].
    pub fn dispatch(&self, target: &HitTarget) -> DrillAction {
        for rule in &self.rules {
            if let Some(action) = rule(target) {
                return action;
            }
        }
        DrillAction::None
    }
}

impl Default for DrillRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn province_pick_opens_state_panel() {
        let reg = DrillRegistry::with_stage1_rules(vec!["close_button".into()]);
        let action = reg.dispatch(&HitTarget::Province {
            province_id: 9669,
            state_id: 469,
        });
        assert_eq!(action, DrillAction::OpenStatePanel { state_id: 469 });
    }

    #[test]
    fn neutral_map_pick_does_not_drill() {
        let reg = DrillRegistry::with_stage1_rules(vec!["close_button".into()]);
        assert_eq!(
            reg.dispatch(&HitTarget::MapNeutral { province_id: 123 }),
            DrillAction::None
        );
    }

    #[test]
    fn close_control_closes_panel() {
        let reg = DrillRegistry::with_stage1_rules(vec!["close_button".into(), "X".into()]);
        assert_eq!(
            reg.dispatch(&HitTarget::Control {
                name: "close_button".into()
            }),
            DrillAction::CloseStatePanel
        );
        assert_eq!(
            reg.dispatch(&HitTarget::Control { name: "X".into() }),
            DrillAction::CloseStatePanel
        );
    }

    #[test]
    fn unknown_control_is_inert() {
        let reg = DrillRegistry::with_stage1_rules(vec!["close_button".into()]);
        // a control that isn't a close button -> no drill action (inert chrome).
        assert_eq!(
            reg.dispatch(&HitTarget::Control {
                name: "settings_button".into()
            }),
            DrillAction::None
        );
    }

    #[test]
    fn scrubber_and_empty_are_inert_for_drill() {
        let reg = DrillRegistry::with_stage1_rules(vec!["close_button".into()]);
        assert_eq!(reg.dispatch(&HitTarget::Scrubber), DrillAction::None);
        assert_eq!(reg.dispatch(&HitTarget::Empty), DrillAction::None);
    }

    /// The registry is EXTENSIBLE: a later view registers a new rule and it routes
    /// without touching the dispatcher. Here we add a rule that reroutes a specific
    /// control; first-match-wins ordering is honoured.
    #[test]
    fn custom_rule_plugs_in_and_first_match_wins() {
        let mut reg = DrillRegistry::new();
        // rule A: any province -> open panel for state 1 (a deliberately wrong-but-first rule)
        reg.register(|t| match t {
            HitTarget::Province { .. } => Some(DrillAction::OpenStatePanel { state_id: 1 }),
            _ => None,
        });
        // rule B: would open the real state, but A matches first.
        reg.register(|t| match t {
            HitTarget::Province { state_id, .. } => Some(DrillAction::OpenStatePanel {
                state_id: *state_id,
            }),
            _ => None,
        });
        assert_eq!(
            reg.dispatch(&HitTarget::Province {
                province_id: 5,
                state_id: 99
            }),
            DrillAction::OpenStatePanel { state_id: 1 },
            "first registered rule wins"
        );
    }
}
