//! Content databases prepared only for reached world operands.
use super::*;
use crate::triggers::{AuthoredStates, PreparedContent};
use std::sync::OnceLock;

#[derive(Default)]
pub(super) struct PreparedInputs {
    states: OnceLock<Result<PreparedStates, String>>,
    continents: OnceLock<Result<Vec<String>, String>>,
    triggers: OnceLock<Result<installed_content::trigger_keys::TriggerNames, String>>,
    buildings: OnceLock<Result<installed_content::buildings::Buildings, String>>,
    bonuses: OnceLock<Result<[i64; 2], String>>,
    roles: OnceLock<Result<LeaderRoles, String>>,
    constants: OnceLock<Result<installed_content::script_constants::Constants, String>>,
    traits: OnceLock<Result<installed_content::unit_leader::Traits, String>>,
    invasion_days: OnceLock<Result<i32, String>>,
    pub(super) dlcs: OnceLock<Result<BTreeSet<String>, String>>,
    pub(super) neighbors: OnceLock<Result<BTreeMap<u32, Vec<u32>>, String>>,
    pub(super) terrain: OnceLock<Option<installed_content::movement::MovementMap>>,
}

#[cfg(test)]
impl PreparedInputs {
    pub(super) fn assert_title_inputs_unread(&self) {
        assert!(self.states.get().is_none());
        assert!(self.buildings.get().is_none());
        assert!(self.roles.get().is_none());
        assert!(self.constants.get().is_none());
        assert!(self.traits.get().is_none());
        assert!(self.continents.get().is_none());
        assert!(self.invasion_days.get().is_none());
    }
}

struct PreparedStates {
    provinces: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    warnings: Vec<String>,
}

impl ScriptedContent {
    fn prepared_buildings(&self) -> Result<&installed_content::buildings::Buildings, &str> {
        self.prepared_environment()?
            .prepared
            .buildings
            .get_or_init(|| {
                installed_content::buildings::read(&self.layers.layers).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)
    }

    fn prepared_environment(&self) -> Result<&Environment, &str> {
        self.environment
            .get_or_init(|| {
                Environment::read(&self.layers.layers)
                    .map_err(|error| format!("scripted localisation content: {error}"))
            })
            .as_ref()
            .map_err(String::as_str)
    }
    fn prepared_states(&self) -> Result<&PreparedStates, &str> {
        let environment = self.prepared_environment()?;
        environment
            .prepared
            .states
            .get_or_init(|| {
                let content = self.cached_states().map_err(str::to_owned)?;
                Ok(PreparedStates {
                    provinces: content
                        .states
                        .definitions()
                        .iter()
                        .map(|(&id, state)| (id, state.provinces.clone()))
                        .collect(),
                    impassable: content
                        .states
                        .definitions()
                        .iter()
                        .filter_map(|(&id, state)| state.impassable.then_some(id))
                        .collect(),
                    warnings: live_state_warnings(&content.warnings)
                        .map(str::to_owned)
                        .collect(),
                })
            })
            .as_ref()
            .map_err(String::as_str)
    }
    pub(super) fn lazy_world<'a>(&'a self, frame: &'a DecodedFrame) -> Result<World<'a>, String> {
        let environment = self.prepared_environment().map_err(str::to_owned)?;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(Ok(inputs)) = environment.inputs.get() {
            return Ok(inputs.world(self, environment, frame));
        }
        static TRIGGERS: OnceLock<installed_content::trigger_keys::TriggerNames> = OnceLock::new();
        let dlcs = environment
            .prepared
            .dlcs
            .get_or_init(|| {
                self.layers
                    .layers
                    .dlc_names()
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok(World::read(
            frame,
            ContentFacts::content_only(
                dlcs,
                TRIGGERS.get_or_init(Default::default),
                &self.tag_aliases,
            ),
        )
        .with_prepared_content(self)
        .with_template_names(
            &self.template_names,
            &self.layers.layers,
            &self.recorded_language,
        ))
    }
}

impl PreparedContent for ScriptedContent {
    fn authored_states(&self) -> AuthoredStates<'_> {
        static PROVINCES: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        static IMPASSABLE: BTreeSet<u32> = BTreeSet::new();
        #[cfg(any(test, feature = "test-support"))]
        if let Ok(environment) = self.prepared_environment()
            && let Some(Ok(inputs)) = environment.inputs.get()
        {
            return (
                &inputs.state_provinces,
                &inputs.impassable,
                inputs.state_error.as_deref().map_or(Ok(()), Err),
            );
        }
        match self.prepared_states() {
            Ok(states) => (&states.provinces, &states.impassable, Ok(())),
            Err(error) => (&PROVINCES, &IMPASSABLE, Err(error)),
        }
    }
    fn continents(&self) -> Result<&[String], &str> {
        let environment = self.prepared_environment()?;
        environment
            .prepared
            .continents
            .get_or_init(|| {
                installed_content::terrain::read_default_map(&self.layers.layers)
                    .and_then(|map| self.layers.layers.read(&map.file("continent")?))
                    .and_then(|bytes| {
                        installed_content::equipment_graphics::context::continent_names(&bytes)
                    })
                    .map_err(|error| error.to_string())
            })
            .as_deref()
            .map_err(String::as_str)
    }
    fn ideologies(&self) -> Result<&installed_content::ideologies::Ideologies, &str> {
        self.prepared_environment()?
            .ideologies_result()
            .as_ref()
            .map_err(String::as_str)
    }
    fn triggers(&self) -> Result<&installed_content::trigger_keys::TriggerNames, &str> {
        let environment = self.prepared_environment()?;
        environment
            .prepared
            .triggers
            .get_or_init(|| {
                let buildings = self.prepared_buildings();
                let scripted = installed_content::scripted_triggers::read(&self.layers.layers)
                    .map_err(|error| error.to_string())?;
                let counts = buildings.ok().and_then(|buildings| {
                    let ideologies = environment.ideologies().ok()?;
                    let resources = installed_content::resources::read(&self.layers.layers).ok()?;
                    Some(installed_content::trigger_keys::CountTriggerNames::new(
                        buildings,
                        ideologies,
                        &resources.0,
                    ))
                });
                Ok(installed_content::trigger_keys::TriggerNames { scripted, counts })
            })
            .as_ref()
            .map_err(String::as_str)
    }
    fn character_traits(
        &self,
    ) -> Result<
        (
            &installed_content::unit_leader::Traits,
            &installed_content::leader_traits::LeaderTraits,
        ),
        &str,
    > {
        let environment = self.prepared_environment()?;
        let units = environment
            .prepared
            .traits
            .get_or_init(|| {
                installed_content::unit_leader::Traits::registered(&self.layers.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)?;
        Ok((
            units,
            environment
                .leader_traits_result()
                .as_ref()
                .map_err(String::as_str)?,
        ))
    }
    fn resources(&self) -> Result<(&installed_content::buildings::Buildings, [i64; 2]), &str> {
        let environment = self.prepared_environment()?;
        let buildings = self.prepared_buildings()?;
        let bonuses = environment
            .prepared
            .bonuses
            .get_or_init(|| {
                crate::state_resources::resource_bonuses(environment.defines())
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)?;
        Ok((buildings, *bonuses))
    }
    fn leader_roles(&self) -> Result<&LeaderRoles, &str> {
        self.prepared_environment()?
            .prepared
            .roles
            .get_or_init(|| {
                installed_content::characters::leader_templates(&self.layers.layers)
                    .map(LeaderRoles::new)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)
    }
    fn constants(&self) -> Result<&installed_content::script_constants::Constants, &str> {
        self.prepared_environment()?
            .prepared
            .constants
            .get_or_init(|| {
                installed_content::script_constants::read(&self.layers.layers)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)
    }
    fn invasion_days(&self) -> Result<i32, &str> {
        let environment = self.prepared_environment()?;
        environment
            .prepared
            .invasion_days
            .get_or_init(|| {
                environment
                    .defines()
                    .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .copied()
            .map_err(String::as_str)
    }
    fn geography(&self) -> GeographyInputs<'_> {
        let environment = self
            .prepared_environment()
            .expect("a world has its initialized environment");
        #[cfg(any(test, feature = "test-support"))]
        if let Some(Ok(inputs)) = environment.inputs.get() {
            return inputs.geography(self, environment);
        }
        let (state_provinces, _, result) = self.authored_states();
        GeographyInputs {
            neighbors: &environment.prepared.neighbors,
            terrain: &environment.prepared.terrain,
            layers: &self.layers.layers,
            state_provinces,
            state_error: result.err(),
            defines: environment.defines(),
            terrain_definitions: environment.terrain_result().as_ref().ok(),
        }
    }
}

impl Environment {
    pub(super) fn prepared_warnings(&self) -> &[String] {
        self.prepared
            .states
            .get()
            .and_then(|result| result.as_ref().ok())
            .map_or(&[], |states| states.warnings.as_slice())
    }
}
