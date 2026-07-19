//! U1 — Topbar alerts strip / notification-area binding
//! (REPLAY_PRODUCTION_PLAN §6a; UI_COVERAGE_MATRIX §2 "Alerts / notifications").
//!
//! Population seam (Constraint 2: parses **no** recording bytes) mapping
//! `replay_core`'s READ-ONLY [`TopbarAlerts`] view (requirements item 9b) onto
//! the topbar alert-icon display state — the four count badges the topbar's
//! `country_menu_window` alert area shows: initiative, decisions-available,
//! intel, and market-contracts.
//!
//! Like [`crate::binding`], it fabricates no value: a `Leaf` that is
//! `Absent`/`Unobtained`, or a view whose status is not `Present`, renders the
//! **fail-closed absent form** (a `None` count / no badge), never a default `0`
//! (Constraint 3).
//!
//! ## FILE citations — element names, `interface/topbar.gui`
//!
//! Each badge count drives one shipped `instantTextBoxType` in
//! `topbar.gui`'s `country_menu_window`; the shared `game_ui` parser resolves
//! them (layout gate: `game_ui/tests/alerts_binding.rs`). See
//! [`AlertKind::gui_element`]. The badge backgrounds are `GFX_topbar_alert_bg`
//! (`topbar.gui`), and the scrolling feed icon is `GFX_global_alert_icons`
//! (`interface/alerts.gui`, `global_alerticon_window`).
//!
//! ## UNKNOWN / FIT labels and data gaps
//!
//! - **Badge zero-suppression is FIT, not FILE.** Whether a `0` count renders a
//!   badge at all is engine C++ logic, *not* encoded in the `.gui`. The
//!   [`AlertBadge::badge_shown`] flag is a documented **FIT** product convention
//!   (shown when a positive count is measured); it makes no engine-parity claim.
//! - **Count number formatting is UNKNOWN → rendered raw** (same rationale as
//!   the topbar scalars in `binding.rs`): [`AlertBadge::count_text`] is
//!   `format!("{}")` of the exact leaf, no grouping/scaling.
//! - **Which topbar badge a single view count maps to is UNKNOWN where the
//!   topbar shows several.** `intel` maps to `intelview_ops_available_items`
//!   (the topbar also shows `..._prepared_or_completed_items`);
//!   `market_contracts` maps to `non_delivering_contract_amount` (the topbar
//!   also shows `ongoing_contract_amount`). The single view count cannot fill
//!   both sub-badges — a data-granularity gap (see below).
//! - **Recording-Absent sub-counts (data gap, EXTRACTION_NEEDS §A / req 9b).**
//!   The topbar shows MORE alert badges than the view carries: decision-timeout
//!   items, intel prepared/completed ops, delivering (ongoing) contracts, and
//!   prototype rewards. The view serves ONE count per category, so those
//!   sub-badges are fail-closed absent here.
//! - **The scrolling alert FEED is a recording-Absent stream (data gap).**
//!   `alerts.gui`'s `global_alerticon_window` shows individual fired alerts over
//!   time; no such stream exists in the recording schema, so
//!   [`alert_feed_binding`] is fail-closed empty. Filed in EXTRACTION_NEEDS §A /
//!   REPLAY_DATA_REQUIREMENTS (fired-alert stream).

use replay_core::{Leaf, TopbarAlerts, ViewStatus, WorldFrame};

/// FILE — the alerts feed layout file (`global_alerticon_window`).
pub const ALERTS_GUI: &str = "interface/alerts.gui";
/// FILE — the topbar layout file carrying the four alert count badges.
pub const TOPBAR_GUI: &str = "interface/topbar.gui";

/// FILE — the alerts-feed container window (`interface/alerts.gui`).
pub const ALERT_FEED_WINDOW: &str = "global_alerticon_window";
/// FILE — the alerts-feed icon sprite (`interface/alerts.gui`).
pub const ALERT_FEED_ICON_SPRITE: &str = "GFX_global_alert_icons";
/// FILE — the shared alert count-badge background sprite (`interface/topbar.gui`).
pub const ALERT_BADGE_BG_SPRITE: &str = "GFX_topbar_alert_bg";

/// One topbar alert category served by the read-only [`TopbarAlerts`] view
/// (requirements item 9b). The badge's target `topbar.gui` element is
/// [`AlertKind::gui_element`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    /// Faction/initiative counter (`initiative_text`).
    Initiative,
    /// Decisions available to take (`decisionview_amount_to_take_items`).
    DecisionsAvailable,
    /// Intel operations available (`intelview_ops_available_items`).
    Intel,
    /// Market contracts alert (`non_delivering_contract_amount`).
    MarketContracts,
}

impl AlertKind {
    /// All four categories in the canonical display order (left→right in the
    /// topbar alert area).
    pub const ALL: [AlertKind; 4] = [
        AlertKind::Initiative,
        AlertKind::DecisionsAvailable,
        AlertKind::Intel,
        AlertKind::MarketContracts,
    ];

    /// FILE — the `topbar.gui` `instantTextBoxType` element whose text this
    /// count drives (`country_menu_window` alert area). Where the topbar shows
    /// several badges for one view count, this is the primary badge and the
    /// module header records the UNKNOWN mapping ambiguity.
    pub fn gui_element(self) -> &'static str {
        match self {
            AlertKind::Initiative => "initiative_text",
            AlertKind::DecisionsAvailable => "decisionview_amount_to_take_items",
            AlertKind::Intel => "intelview_ops_available_items",
            AlertKind::MarketContracts => "non_delivering_contract_amount",
        }
    }

    /// The matching leaf on a [`TopbarAlerts`] view.
    fn leaf(self, alerts: &TopbarAlerts) -> &Leaf<i64> {
        match self {
            AlertKind::Initiative => &alerts.initiative,
            AlertKind::DecisionsAvailable => &alerts.decisions_available,
            AlertKind::Intel => &alerts.intel,
            AlertKind::MarketContracts => &alerts.market_contracts,
        }
    }
}

/// One alert count badge's display state, fail-closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertBadge {
    pub kind: AlertKind,
    /// FILE — the target `topbar.gui` element name ([`AlertKind::gui_element`]).
    pub gui_element: &'static str,
    /// The measured count, or `None` (fail-closed) when unmeasured — never `0`.
    pub count: Option<i64>,
    /// FIT product convention: `true` when a positive count is measured. The
    /// engine's exact zero-suppression is UNKNOWN (C++ logic, not in the
    /// `.gui`); this is NOT a file law (see module header).
    pub badge_shown: bool,
    /// Raw count text for the badge (UNKNOWN formatting → `format!("{}")`), or
    /// `None` when the badge is not shown.
    pub count_text: Option<String>,
}

/// The topbar alerts strip for one POV at one world frame (U1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertsStripBinding {
    pub pov_tag: String,
    /// `true` only when the underlying [`TopbarAlerts`] view is `Present` for
    /// this frame (mirrors [`WorldFrame::topbar_extras_status`]).
    pub available: bool,
    /// The four alert badges, always in [`AlertKind::ALL`] order.
    pub badges: Vec<AlertBadge>,
}

impl AlertsStripBinding {
    /// The badge for `kind` (always present — the strip carries all four).
    pub fn badge(&self, kind: AlertKind) -> &AlertBadge {
        self.badges
            .iter()
            .find(|b| b.kind == kind)
            .expect("AlertsStripBinding always carries all four AlertKind badges")
    }
}

/// Build one badge from the (optional) POV alert view. A view that is absent, a
/// POV missing from a present view, or an unmeasured leaf all yield the
/// fail-closed absent badge (`count: None`, `badge_shown: false`).
fn badge_from(kind: AlertKind, alerts: Option<&TopbarAlerts>) -> AlertBadge {
    let count = alerts
        .map(|a| kind.leaf(a))
        .and_then(Leaf::measured)
        .copied();
    let badge_shown = matches!(count, Some(c) if c > 0);
    let count_text = if badge_shown {
        count.map(|c| format!("{c}"))
    } else {
        None
    };
    AlertBadge {
        kind,
        gui_element: kind.gui_element(),
        count,
        badge_shown,
        count_text,
    }
}

/// Build the topbar alerts strip for `pov_tag` at one world frame (U1).
///
/// When the topbar-extras view is not `Present`, or the POV is absent from a
/// present view, every badge is the fail-closed absent form; `available`
/// mirrors the view status for the frame.
pub fn alerts_strip_from_frame(frame: &WorldFrame, pov_tag: &str) -> AlertsStripBinding {
    let present = frame.topbar_extras_status() == ViewStatus::Present;
    let alerts = if present {
        frame.topbar_alerts(pov_tag)
    } else {
        None
    };
    let badges = AlertKind::ALL
        .iter()
        .map(|&kind| badge_from(kind, alerts))
        .collect();
    AlertsStripBinding {
        pov_tag: pov_tag.to_string(),
        available: present,
        badges,
    }
}

/// The scrolling alert / notification FEED (`alerts.gui` `global_alerticon_window`).
///
/// The recording carries **no** fired-alert stream, so this is fail-closed:
/// `available: false`, no icons. The `.gui` layout for the feed is still
/// FILE-cited (layout gate) — only the per-alert content is Absent. Data gap
/// filed in EXTRACTION_NEEDS §A / REPLAY_DATA_REQUIREMENTS (fired-alert stream).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertFeedBinding {
    /// Always `false`: no fired-alert stream in the recording schema.
    pub available: bool,
    /// Always empty (recording-Absent). Each entry would be a resolved feed
    /// icon sprite ([`ALERT_FEED_ICON_SPRITE`]).
    pub icons: Vec<String>,
}

/// The fail-closed absent alert feed (see [`AlertFeedBinding`]).
pub fn alert_feed_binding() -> AlertFeedBinding {
    AlertFeedBinding::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alerts(
        init: Leaf<i64>,
        dec: Leaf<i64>,
        intel: Leaf<i64>,
        market: Leaf<i64>,
    ) -> TopbarAlerts {
        TopbarAlerts {
            initiative: init,
            decisions_available: dec,
            intel,
            market_contracts: market,
        }
    }

    #[test]
    fn badge_positive_count_is_shown_with_raw_text() {
        let b = badge_from(
            AlertKind::Initiative,
            Some(&alerts(
                Leaf::Measured(2),
                Leaf::Measured(0),
                Leaf::Absent,
                Leaf::Unobtained,
            )),
        );
        assert_eq!(b.count, Some(2));
        assert!(b.badge_shown);
        assert_eq!(b.count_text.as_deref(), Some("2"));
        assert_eq!(b.gui_element, "initiative_text");
    }

    #[test]
    fn zero_count_measured_is_absent_badge_never_shown() {
        // A *measured* zero is a real value (count = Some(0)) but the FIT
        // convention suppresses the badge; it is never a fabricated default.
        let b = badge_from(
            AlertKind::DecisionsAvailable,
            Some(&alerts(
                Leaf::Absent,
                Leaf::Measured(0),
                Leaf::Absent,
                Leaf::Absent,
            )),
        );
        assert_eq!(b.count, Some(0));
        assert!(!b.badge_shown);
        assert_eq!(b.count_text, None);
    }

    #[test]
    fn unmeasured_leaf_is_fail_closed_none() {
        for leaf in [Leaf::Absent, Leaf::Unobtained] {
            let b = badge_from(
                AlertKind::Intel,
                Some(&alerts(Leaf::Absent, Leaf::Absent, leaf, Leaf::Absent)),
            );
            assert_eq!(b.count, None);
            assert!(!b.badge_shown);
            assert_eq!(b.count_text, None);
        }
    }

    #[test]
    fn no_view_yields_all_absent_badges() {
        let badges: Vec<_> = AlertKind::ALL
            .iter()
            .map(|&k| badge_from(k, None))
            .collect();
        assert_eq!(badges.len(), 4);
        assert!(badges.iter().all(|b| b.count.is_none() && !b.badge_shown));
    }

    #[test]
    fn feed_is_fail_closed_absent() {
        let feed = alert_feed_binding();
        assert!(!feed.available);
        assert!(feed.icons.is_empty());
    }

    #[test]
    fn strip_carries_all_four_kinds_in_order() {
        let a = alerts(
            Leaf::Measured(1),
            Leaf::Measured(2),
            Leaf::Measured(3),
            Leaf::Measured(4),
        );
        let badges: Vec<_> = AlertKind::ALL
            .iter()
            .map(|&k| badge_from(k, Some(&a)))
            .collect();
        let kinds: Vec<_> = badges.iter().map(|b| b.kind).collect();
        assert_eq!(kinds, AlertKind::ALL.to_vec());
    }
}
