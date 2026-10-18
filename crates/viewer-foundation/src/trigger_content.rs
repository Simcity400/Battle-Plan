//! Authored content operands shared by raid, combat and world trigger evaluators.

use crate::movement_timing::MovementContent;
use dashboard_text::triggers::{ContentFacts, LeaderRoles};
use installed_content_core::buildings::Buildings;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone)]
pub struct TriggerContent {
    /// Scripted triggers and count-trigger names beyond the registry, loaded
    /// once with the document (empty until then).
    pub triggers: Arc<installed_content_core::trigger_keys::TriggerNames>,
    /// Loaded declarations for ideology specifiers and leader subtypes.
    pub ideologies: Option<installed_content_core::ideologies::Ideologies>,
    pub continents: Result<Vec<String>, String>,
    /// The same loaded trait databases as the combat trigger evaluator.
    pub character_traits: Option<(
        installed_content_core::unit_leader::Traits,
        Result<Arc<installed_content_core::leader_traits::LeaderTraits>, String>,
    )>,
    /// Tag alias database.
    pub tag_aliases: Arc<installed_content_core::tag_aliases::TagAliases>,
    pub buildings: Buildings,
    pub dlcs: BTreeSet<String>,
    /// Each state's provinces in its own order.
    pub state_provinces: BTreeMap<u32, Vec<u32>>,
    pub impassable: BTreeSet<u32>,
    /// Each state's neighbour states from the province graph
    /// ([`ProvinceGraph::neighbor_states`](installed_content_core::province_graph::ProvinceGraph::neighbor_states)),
    /// or why they cannot be derived.
    pub neighbor_states: Result<BTreeMap<u32, Vec<u32>>, String>,
    /// The content movement map, whose static province terrain decides
    /// `is_coastal`; the document sets it once loaded, `None` until then or
    /// when it could not be read.
    pub terrain: Option<Arc<MovementContent>>,
    /// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and `SUPPLY_ROUTE_RESOURCE_BONUS`.
    pub resource_bonuses: [i64; 2],
    /// The character templates, whose unit leader role `visible` triggers
    /// `is_leader_visible` and `has_any_captured_general` read, or why they
    /// are unread.
    pub leader_roles: Result<Arc<LeaderRoles>, String>,
    /// `NNavy.NAVAL_INVASION_PREPARE_DAYS`, or why it is unread.
    pub invasion_prepare_days: Result<i32, String>,
    /// The script constants a `constant:` operand reads, or why they are
    /// unread.
    pub script_constants: Result<installed_content_core::script_constants::Constants, String>,
}

impl TriggerContent {
    pub fn facts(&self) -> ContentFacts<'_> {
        ContentFacts {
            state_provinces: &self.state_provinces,
            state_content: Ok(()),
            continents: self.continents.as_deref().map_err(String::as_str),
            ideologies: self
                .ideologies
                .as_ref()
                .ok_or("the raid ideologies are not loaded"),
            impassable: &self.impassable,
            neighbor_states: self.neighbor_states.as_ref().map_err(String::as_str),
            terrain: self.terrain.as_deref().map(|content| &content.map),
            dlcs: &self.dlcs,
            triggers: Ok(&self.triggers),
            tag_aliases: &self.tag_aliases,
            buildings: Ok(&self.buildings),
            resource_bonuses: Ok(self.resource_bonuses),
            leader_roles: self.leader_roles.as_deref().map_err(String::as_str),
            invasion_prepare_days: self
                .invasion_prepare_days
                .as_ref()
                .copied()
                .map_err(String::as_str),
            script_constants: self.script_constants.as_ref().map_err(String::as_str),
            character_traits: self
                .character_traits
                .as_ref()
                .ok_or("the raid trait databases are not loaded")
                .and_then(|(units, leaders)| {
                    leaders
                        .as_deref()
                        .map(|leaders| (units, leaders))
                        .map_err(String::as_str)
                }),
        }
    }
}
