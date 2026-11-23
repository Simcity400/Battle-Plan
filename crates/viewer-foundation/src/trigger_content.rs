
use crate::movement_timing::MovementContent;
use dashboard_text::triggers::{ContentFacts, LeaderRoles};
use content_reader_key_names::buildings::Buildings;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone)]
pub struct TriggerContent {
    pub triggers: Arc<content_reader_script::trigger_keys::TriggerNames>,
    pub ideologies: Option<content_reader_key_names::ideologies::Ideologies>,
    pub continents: Result<Vec<String>, String>,
    pub character_traits: Option<(
        content_reader_key_names::unit_leader::Traits,
        Result<Arc<content_reader_military::leader_traits::LeaderTraits>, String>,
    )>,
    pub tag_aliases: Arc<content_reader_key_names::tag_aliases::TagAliases>,
    pub buildings: Buildings,
    pub dlcs: BTreeSet<String>,
    pub state_provinces: BTreeMap<u32, Vec<u32>>,
    pub impassable: BTreeSet<u32>,
    pub neighbor_states: Result<BTreeMap<u32, Vec<u32>>, String>,
    pub terrain: Option<Arc<MovementContent>>,
    pub resource_bonuses: [i64; 2],
    pub leader_roles: Result<Arc<LeaderRoles>, String>,
    pub invasion_prepare_days: Result<i32, String>,
    pub script_constants: Result<content_reader_politics::script_constants::Constants, String>,
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
