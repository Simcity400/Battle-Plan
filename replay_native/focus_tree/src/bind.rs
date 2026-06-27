//! Bind each placed focus node's STATE from the snapshot `focus` domain
//! (plan §B.5 / BINDING_MAP §2c). State fidelity is EXACT for done/active (we
//! record `completed[]` + `current`); the prereqs-met-vs-unmet state is DERIVED
//! from the prereq graph against the completed set; progress is EXACT (days)
//! when active.
//!
//! No-fabrication (Codex-confirmed): a field the snapshot does not carry stays
//! `None` and renders "—"; we never invent a progress fraction; and we never
//! claim full the game "availability" (which needs available/bypass/exclusive/
//! capitulation gates we don't carry) — only that the PREREQUISITES are/aren't
//! met (see [`NodeState`]).

use std::collections::HashSet;

use crate::eval::EvalContext;
use serde_json::Value as Json;

/// The replay-time state of one focus node.
///
/// IMPORTANT (no-fabrication, Codex-confirmed): we record only completed/current/
/// progress. `completed` and `current` are EXACT. The other two states are
/// derived ONLY from the prerequisite graph against the completed set — they are
/// **`PrereqsMet` / `PrereqsUnmet`, NOT true the game "available/locked"**. Real
/// availability also depends on `available`/`allow_branch`/`mutually_exclusive`/
/// `bypass`/capitulation gates that the snapshot does not carry, so naming these
/// "Available"/"Locked" would fabricate availability we cannot prove. The names
/// (and the UI labels) say exactly what we know: the prerequisites are/aren't met.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeState {
    /// In `focus.completed[]`. EXACT.
    Completed,
    /// Equals `focus.current` (the in-progress focus). EXACT.
    Current,
    /// Not done, not current, and ALL prereq groups satisfied by the completed
    /// set. DERIVED — "prerequisites met", NOT certified-available.
    PrereqsMet,
    /// Not done, not current, and at least one prereq group unsatisfied. DERIVED.
    PrereqsUnmet,
}

/// Per-node bound view-model.
#[derive(Debug, Clone)]
pub struct NodeBinding {
    pub id: String,
    pub state: NodeState,
    /// Days into the focus (only when `state == Current`). EXACT.
    pub progress_days: Option<i64>,
}

/// The country `focus` domain, extracted once.
#[derive(Debug, Clone, Default)]
pub struct FocusDomain {
    pub focus_tree: Option<String>,
    pub completed: HashSet<String>,
    pub current: Option<String>,
    pub progress: Option<i64>,
}

impl FocusDomain {
    /// Pull the `focus` object out of a country's JSON. Returns an empty domain
    /// (all None/empty) when absent — callers render "—", never fabricate.
    pub fn from_country(country: &Json) -> FocusDomain {
        let f = match country.get("focus") {
            Some(f) => f,
            None => return FocusDomain::default(),
        };
        let completed = f
            .get("completed")
            .and_then(Json::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        FocusDomain {
            focus_tree: f
                .get("focus_tree")
                .and_then(Json::as_str)
                .map(|s| s.to_string()),
            completed,
            current: f
                .get("current")
                .and_then(Json::as_str)
                .map(|s| s.to_string()),
            progress: f.get("progress").and_then(Json::as_i64),
        }
    }
}

/// Build an [`EvalContext`] from the observed country tag + its focus domain.
/// Game rules / DLC / global flags are taken from the snapshot world block when
/// present; when ABSENT we mark them not-known so a trigger that needs them is
/// UNSUPPORTED (badged) rather than fabricated false.
pub fn eval_context(tag: &str, domain: &FocusDomain, world: Option<&Json>) -> EvalContext {
    let mut ctx = EvalContext {
        tag: tag.to_string(),
        original_tag: Some(tag.to_string()),
        completed_focuses: domain.completed.clone(),
        ..Default::default()
    };
    if let Some(w) = world {
        if let Some(rules) = w.get("game_rules").and_then(Json::as_object) {
            ctx.game_rules_known = true;
            for (k, v) in rules {
                if let Some(s) = v.as_str() {
                    ctx.game_rules.insert(k.clone(), s.to_string());
                }
            }
        }
        if let Some(dlc) = w.get("dlc").and_then(Json::as_array) {
            ctx.dlc_known = true;
            ctx.dlc = dlc
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
        if let Some(flags) = w.get("global_flags").and_then(Json::as_array) {
            ctx.global_flags_known = true;
            ctx.global_flags = flags
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }
    }
    ctx
}

/// Bind one node id's state against the focus domain. `prereqs` is the node's
/// AND-of-OR prereq groups (each inner Vec is an OR set).
pub fn bind_node(id: &str, prereqs: &[Vec<String>], domain: &FocusDomain) -> NodeBinding {
    let state = if domain.completed.contains(id) {
        NodeState::Completed
    } else if domain.current.as_deref() == Some(id) {
        NodeState::Current
    } else {
        // available iff every prereq group has at least one completed member
        let all_groups_ok = prereqs
            .iter()
            .all(|group| group.is_empty() || group.iter().any(|p| domain.completed.contains(p)));
        if all_groups_ok {
            NodeState::PrereqsMet
        } else {
            NodeState::PrereqsUnmet
        }
    };
    let progress_days = if state == NodeState::Current {
        domain.progress
    } else {
        None
    };
    NodeBinding {
        id: id.to_string(),
        state,
        progress_days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn domain() -> FocusDomain {
        let country = json!({
            "focus": {
                "focus_tree": "german_focus",
                "completed": ["A", "B"],
                "current": "C",
                "progress": 12
            }
        });
        FocusDomain::from_country(&country)
    }

    #[test]
    fn extracts_domain() {
        let d = domain();
        assert_eq!(d.focus_tree.as_deref(), Some("german_focus"));
        assert!(d.completed.contains("A"));
        assert_eq!(d.current.as_deref(), Some("C"));
        assert_eq!(d.progress, Some(12));
    }

    #[test]
    fn completed_current_available_locked() {
        let d = domain();
        assert_eq!(bind_node("A", &[], &d).state, NodeState::Completed);
        let c = bind_node("C", &[], &d);
        assert_eq!(c.state, NodeState::Current);
        assert_eq!(c.progress_days, Some(12));
        // D has prereq A (completed) -> available
        assert_eq!(
            bind_node("D", &[vec!["A".into()]], &d).state,
            NodeState::PrereqsMet
        );
        // E has prereq Z (not completed) -> locked
        assert_eq!(
            bind_node("E", &[vec!["Z".into()]], &d).state,
            NodeState::PrereqsUnmet
        );
        // OR group satisfied by B
        assert_eq!(
            bind_node("F", &[vec!["Z".into(), "B".into()]], &d).state,
            NodeState::PrereqsMet
        );
        // AND of two groups: one ok (A), one not (Z) -> locked
        assert_eq!(
            bind_node("G", &[vec!["A".into()], vec!["Z".into()]], &d).state,
            NodeState::PrereqsUnmet
        );
    }

    #[test]
    fn absent_focus_domain_is_empty_not_fabricated() {
        let d = FocusDomain::from_country(&json!({}));
        assert!(d.focus_tree.is_none());
        assert!(d.completed.is_empty());
        // a node with no prereqs binds PrereqsMet (vacuously) but progress stays None
        let b = bind_node("X", &[], &d);
        assert_eq!(b.state, NodeState::PrereqsMet);
        assert_eq!(b.progress_days, None);
    }

    #[test]
    fn eval_context_marks_unknown_capabilities() {
        let d = domain();
        let ctx = eval_context("GER", &d, None);
        assert!(!ctx.game_rules_known);
        assert!(!ctx.dlc_known);
        assert!(!ctx.global_flags_known);
        assert!(ctx.completed_focuses.contains("A"));
    }
}
