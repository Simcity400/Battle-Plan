
use replay_core::{
    alert_icon_frame_for_type, ActiveAlertRow, AlertDedupIdentity, Leaf, TopbarAlerts, ViewStatus,
    WorldFrame,
};

pub const ALERTS_GUI: &str = "interface/alerts.gui";
pub const TOPBAR_GUI: &str = "interface/topbar.gui";

pub const ALERT_FEED_WINDOW: &str = "global_alerticon_window";
pub const ALERT_FEED_ICON_SPRITE: &str = "GFX_global_alert_icons";
pub const ALERT_BADGE_BG_SPRITE: &str = "GFX_topbar_alert_bg";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Initiative,
    DecisionsAvailable,
    DecisionsTimeout,
    Intel,
    IntelPreparedOrCompleted,
    PrototypeRewards,
    MarketContracts,
    MarketContractsOngoing,
}

impl AlertKind {
    pub const ALL: [AlertKind; 8] = [
        AlertKind::Initiative,
        AlertKind::DecisionsAvailable,
        AlertKind::DecisionsTimeout,
        AlertKind::Intel,
        AlertKind::IntelPreparedOrCompleted,
        AlertKind::PrototypeRewards,
        AlertKind::MarketContracts,
        AlertKind::MarketContractsOngoing,
    ];

    pub fn gui_element(self) -> &'static str {
        match self {
            AlertKind::Initiative => "initiative_text",
            AlertKind::DecisionsAvailable => "decisionview_amount_to_take_items",
            AlertKind::DecisionsTimeout => "decisionview_amount_timeout_items",
            AlertKind::Intel => "intelview_ops_available_items",
            AlertKind::IntelPreparedOrCompleted => "intelview_ops_prepared_or_completed_items",
            AlertKind::PrototypeRewards => "prototype_rewards_aquired",
            AlertKind::MarketContracts => "non_delivering_contract_amount",
            AlertKind::MarketContractsOngoing => "ongoing_contract_amount",
        }
    }

    fn leaf(self, alerts: &TopbarAlerts) -> &Leaf<i64> {
        match self {
            AlertKind::Initiative => &alerts.initiative,
            AlertKind::DecisionsAvailable => &alerts.decisions_available,
            AlertKind::DecisionsTimeout => &alerts.decisions_timeout,
            AlertKind::Intel => &alerts.intel,
            AlertKind::IntelPreparedOrCompleted => &alerts.intel_prepared_or_completed,
            AlertKind::PrototypeRewards => &alerts.prototype_rewards,
            AlertKind::MarketContracts => &alerts.market_contracts,
            AlertKind::MarketContractsOngoing => &alerts.market_contracts_ongoing,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertBadge {
    pub kind: AlertKind,
    pub gui_element: &'static str,
    pub count: Option<i64>,
    pub badge_shown: bool,
    pub count_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertsStripBinding {
    pub pov_tag: String,
    pub available: bool,
    pub badges: Vec<AlertBadge>,
}

impl AlertsStripBinding {
    pub fn badge(&self, kind: AlertKind) -> &AlertBadge {
        self.badges
            .iter()
            .find(|b| b.kind == kind)
            .expect("AlertsStripBinding always carries every AlertKind::ALL badge")
    }
}

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

fn set_badge_count(badges: &mut [AlertBadge], kind: AlertKind, count: Option<i64>) {
    let badge = badges
        .iter_mut()
        .find(|badge| badge.kind == kind)
        .expect("AlertKind::ALL supplies every badge");
    badge.count = count;
    badge.badge_shown = count.is_some_and(|count| count > 0);
    badge.count_text = count
        .filter(|count| *count > 0)
        .map(|count| count.to_string());
}

fn validated_prototype_reward_count(frame: &WorldFrame, pov_tag: &str) -> Option<i64> {
    if frame.special_project_programs_status() != ViewStatus::Present {
        return None;
    }
    match frame.special_project_programs(pov_tag) {
        Leaf::Measured(rows) => rows.iter().try_fold(0i64, |sum, row| {
            sum.checked_add(i64::from(row.unread_prototype_rewards))
        }),
        Leaf::Absent | Leaf::Unobtained => None,
    }
}

fn compose_alerts_strip(
    pov_tag: &str,
    general_alerts_present: bool,
    alerts: Option<&TopbarAlerts>,
    validated_prototype_rewards: Option<i64>,
) -> AlertsStripBinding {
    let mut badges: Vec<_> = AlertKind::ALL
        .iter()
        .map(|&kind| badge_from(kind, alerts))
        .collect();

    set_badge_count(
        &mut badges,
        AlertKind::PrototypeRewards,
        validated_prototype_rewards,
    );
    AlertsStripBinding {
        pov_tag: pov_tag.to_string(),
        available: general_alerts_present || validated_prototype_rewards.is_some(),
        badges,
    }
}

pub fn alerts_strip_from_frame(frame: &WorldFrame, pov_tag: &str) -> AlertsStripBinding {
    let present = frame.topbar_alerts_status() == ViewStatus::Present;
    let alerts = if present {
        frame.topbar_alerts(pov_tag)
    } else {
        None
    };
    compose_alerts_strip(
        pov_tag,
        present,
        alerts,
        validated_prototype_reward_count(frame, pov_tag),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AlertFeedIconBinding {
    pub alert_type: u16,
    pub source_ordinal: u16,
    pub priority: i32,
    pub template_class: u32,
    pub dedup_identity: AlertDedupIdentity,
    pub country_target_db_id: Option<u32>,
    pub command_group_target_omitted: bool,
    pub icon_frame: u16,
    pub display_order: i32,
    pub sprite: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlertFeedBinding {
    pub available: bool,
    pub rows: Vec<ActiveAlertRow>,
    pub icons: Vec<AlertFeedIconBinding>,
}

pub fn alert_feed_binding() -> AlertFeedBinding {
    AlertFeedBinding::default()
}

pub fn alert_feed_from_frame(frame: &WorldFrame, pov_tag: &str) -> AlertFeedBinding {
    if frame.topbar_alerts_status() != ViewStatus::Present {
        return AlertFeedBinding::default();
    }
    let Some(alerts) = frame.topbar_alerts(pov_tag) else {
        return AlertFeedBinding::default();
    };
    let Leaf::Measured(rows) = &alerts.active_feed else {
        return AlertFeedBinding::default();
    };

    let mut previous = None;
    for row in rows {
        let family_valid = match row.dedup_identity {
            AlertDedupIdentity::ZeroTuple => row.country_target_db_id.is_none(),
            AlertDedupIdentity::CountryDbId => {
                matches!(row.alert_type, 1 | 2 | 5 | 6 | 7 | 0x2d | 0x2e)
                    && row.country_target_db_id.is_some()
            }
            AlertDedupIdentity::CommandGroupOmitted => {
                matches!(row.alert_type, 3 | 4 | 0x2b) && row.country_target_db_id.is_none()
            }
            AlertDedupIdentity::OpaqueEngineObjectOmitted => {
                matches!(row.alert_type, 0x16 | 0x3c) && row.country_target_db_id.is_none()
            }
        };
        let canonical = match previous {
            None => row.source_ordinal == 0,
            Some((prior_type, prior_ordinal)) => {
                (row.alert_type > prior_type && row.source_ordinal == 0)
                    || (row.alert_type == prior_type
                        && row.source_ordinal == prior_ordinal + 1)
            }
        };
        if row.alert_type >= 0x52
            || !family_valid
            || !canonical
            || row.icon_frame
                != row.icon_present.then(|| {
                    alert_icon_frame_for_type(row.alert_type)
                        .expect("alert type range checked above")
                })
            || (row.icon_present && row.display_order < 0)
            || (!row.icon_present && row.display_order != -1)
        {
            return AlertFeedBinding::default();
        }
        previous = Some((row.alert_type, row.source_ordinal));
    }

    let mut icons = rows
        .iter()
        .filter(|row| row.icon_present)
        .map(|row| AlertFeedIconBinding {
            alert_type: row.alert_type,
            source_ordinal: row.source_ordinal,
            priority: row.priority,
            template_class: row.template_class,
            dedup_identity: row.dedup_identity,
            country_target_db_id: row.country_target_db_id,
            command_group_target_omitted: matches!(
                row.dedup_identity,
                AlertDedupIdentity::CommandGroupOmitted
            ),
            icon_frame: row.icon_frame.expect("filtered icon-bearing row"),
            display_order: row.display_order,
            sprite: ALERT_FEED_ICON_SPRITE,
        })
        .collect::<Vec<_>>();
    icons.sort_by_key(|icon| (icon.display_order, icon.alert_type, icon.source_ordinal));
    AlertFeedBinding {
        available: true,
        rows: rows.clone(),
        icons,
    }
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
            decisions_timeout: Leaf::Absent,
            intel,
            intel_prepared_or_completed: Leaf::Absent,
            prototype_rewards: Leaf::Absent,
            market_contracts: market,
            market_contracts_ongoing: Leaf::Absent,
            active_feed: Leaf::Absent,
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
        assert_eq!(badges.len(), AlertKind::ALL.len());
        assert!(badges.iter().all(|b| b.count.is_none() && !b.badge_shown));
    }

    #[test]
    fn feed_is_fail_closed_absent() {
        let feed = alert_feed_binding();
        assert!(!feed.available);
        assert!(feed.icons.is_empty());
    }

    #[test]
    fn strip_carries_every_canonical_kind_in_order() {
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
        assert_eq!(badges.len(), AlertKind::ALL.len());
        assert_eq!(kinds, AlertKind::ALL.to_vec());
    }

}
