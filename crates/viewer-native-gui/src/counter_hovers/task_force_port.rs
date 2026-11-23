use super::gui_authored::Authored;
use super::unit_hits as unit_selection;
use viewer_unit_reads::counter_inputs::{self, NavalMember};
use viewer_runtime::geometry::contains;
use crate::native_gui::HitGeometry;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Control {
    #[default]
    Normal,
    InvalidOrders,
    NoOrders,
    Carrier,
    Pride,
    Spotting,
    Unbound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub country: u32,
    pub visibility: u16,
    pub enemy: Result<bool, String>,
    pub hold_fire: Option<bool>,
    pub idle: Option<bool>,
    pub no_regions: Option<bool>,
    pub pride: Option<bool>,
}

fn any(terms: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    terms.into_iter().fold(Some(false), merge)
}

fn merge(a: Option<bool>, b: Option<bool>) -> Option<bool> {
    match (a, b) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (Some(false), Some(false)) => Some(false),
        _ => None,
    }
}

impl Metadata {
    pub fn from_members(
        members: &[NavalMember<'_>],
        aggression: &[i64],
        country: u32,
        visibility: u16,
        enemy: Result<bool, String>,
    ) -> Self {
        let mut hold_fire = Some(false);
        let mut idle = Some(false);
        let mut no_regions = Some(false);
        for member in members.iter().filter(|member| member.own) {
            let Some(inputs) = member.inputs else {
                hold_fire = merge(hold_fire, None);
                idle = merge(idle, None);
                no_regions = merge(no_regions, None);
                continue;
            };
            if inputs.repair_parent != 0 {
                continue;
            }
            hold_fire = merge(
                hold_fire,
                if matches!(member.mission, 2 | 3) {
                    counter_inputs::engagement_refused(
                        inputs.running_away,
                        inputs.engagement_rule,
                        member.mission,
                        aggression,
                    )
                } else {
                    Some(false)
                },
            );
            idle = merge(
                idle,
                if member.has_fleet {
                    member.fleet_regions.map(|count| {
                        count != 0
                            && counter_inputs::effective_mission(
                                member.mission,
                                inputs.detached_activity,
                                inputs.repair_last_mission,
                            ) == 0
                    })
                } else {
                    Some(false)
                },
            );
            no_regions = merge(
                no_regions,
                Some(
                    counter_inputs::needs_regions(member.mission)
                        && inputs.mission_region_count == 0,
                ),
            );
        }
        Self {
            country,
            visibility,
            enemy,
            hold_fire,
            idle,
            no_regions,
            pride: any(members.iter().map(|member| member.pride)),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hover {
    pub control: Control,
    pub unit_hit: usize,
    pub rect: [f32; 4],
    pub supported: bool,
    pub hit: Option<HitGeometry>,
    pub members: Arc<[u64]>,
    pub metadata: Metadata,
    pub authored: Option<Authored>,
}

impl Hover {
    pub fn has_tooltip(&self) -> bool {
        self.control != Control::Unbound || self.authored.is_some()
    }

    pub fn contains(&self, point: [f32; 2]) -> bool {
        self.hit
            .as_ref()
            .map_or_else(|| contains(self.rect, point), |hit| hit.contains(point))
    }

    pub fn matches(&self, unit: &unit_selection::Hit) -> bool {
        unit.port && unit.kind == unit_selection::Kind::TaskForce && unit.members == self.members
    }
}
