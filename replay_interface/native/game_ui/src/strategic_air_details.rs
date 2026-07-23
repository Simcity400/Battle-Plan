//! Production binding for D61's exact strategic-air details channels.
//!
//! This module consumes only `replay_core`'s typed view. Raw replay JSON is
//! deliberately not reachable here.

use replay_core::{AirLossStatsChannel, AirLossStatsView, Leaf, ViewStatus, WorldFrame};

#[derive(Debug, Clone, PartialEq)]
pub struct StrategicAirDetailsPanel {
    pub country: String,
    pub channels: Vec<AirLossStatsChannel>,
}

pub fn bind_strategic_air_details(
    frame: &WorldFrame,
    country: &str,
) -> Leaf<StrategicAirDetailsPanel> {
    bind_strategic_air_details_view(frame.air_loss_stats(), country)
}

fn bind_strategic_air_details_view(
    view: &AirLossStatsView,
    country: &str,
) -> Leaf<StrategicAirDetailsPanel> {
    match view.status {
        ViewStatus::DomainAbsent => Leaf::Absent,
        ViewStatus::DomainUnavailable => Leaf::Unobtained,
        ViewStatus::Present => view
            .rows
            .iter()
            .find(|row| row.country == country)
            .map(|row| {
                Leaf::Measured(StrategicAirDetailsPanel {
                    country: row.country.clone(),
                    channels: row.channels.clone(),
                })
            })
            // D61 emits only countries with an authenticated statistic-23
            // record. Missing from a complete frame is therefore unavailable,
            // not an inferred all-zero panel.
            .unwrap_or(Leaf::Unobtained),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::AirLossStatsRow;

    #[test]
    fn panel_preserves_exact_order_and_does_not_invent_missing_country_zeroes() {
        let view = AirLossStatsView {
            status: ViewStatus::Present,
            rows: vec![AirLossStatsRow {
                country: "GER".into(),
                owner_tag_db_id: 1,
                country_idx: 1,
                channels: vec![AirLossStatsChannel {
                    label: "our_fighters_lost".into(),
                    raw: 300_000,
                    display_divisor: 100_000,
                    display_value: 3.0,
                }],
            }],
        };
        let Leaf::Measured(panel) = bind_strategic_air_details_view(&view, "GER") else {
            panic!("measured D61 panel");
        };
        assert_eq!(panel.channels[0].raw, 300_000);
        assert_eq!(
            bind_strategic_air_details_view(&view, "USA"),
            Leaf::Unobtained
        );
    }
}
