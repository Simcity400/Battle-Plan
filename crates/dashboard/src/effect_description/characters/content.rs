use super::*;

pub(in crate::effect_description) struct CharacterContent {
    pub(super) catalogue: Result<installed_content::characters::Characters, String>,
    pub(super) unit_leaders: Result<installed_content::unit_leader::Leaders<String>, String>,
    pub(in crate::effect_description) special_projects: Result<installed_content::special_projects::Catalog, String>,
    pub(super) scientist_traits: Result<installed_content::scientist_traits::ScientistTraits, String>,
    pub(super) factions: Result<installed_content::faction_templates::Catalog, String>,
    pub(in crate::effect_description) notes: Vec<String>,
}

impl CharacterContent {
    pub(in crate::effect_description) fn read(
        layers: &installed_content::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
        advisor_templates: Result<&installed_content::characters::Characters, String>,
    ) -> Self {
        let mut notes = Vec::new();
        let catalogue = (|| {
            let templates = advisor_templates?.clone();
            // Full role/name/portrait projections differ from leader_templates' visibility inputs.
            let (catalogue, warnings) =
                installed_content::characters::read(layers, templates).map_err(|why| why.to_string())?;
            notes.extend(warnings.into_iter().map(|(_, why)| why));
            Ok::<_, String>(catalogue)
        })();
        // Full description bodies/skills differ from the evaluator's registration-only traits.
        let unit_leaders = installed_content::unit_leader::Leaders::<String>::load(layers, &|name| {
            environment
                .terrain()
                .ok()?
                .categories
                .iter()
                .find(|category| category.name.eq_ignore_ascii_case(name.as_bytes()))
                .and_then(|category| String::from_utf8(category.name.clone()).ok())
        })
        .inspect(|db| notes.extend(db.misreads.iter().cloned()))
        .map_err(|why| why.to_string());
        let special_projects = installed_content::special_projects::Catalog::read(layers)
            .inspect(|db| notes.extend(db.warnings.iter().cloned()))
            .map_err(|why| why.to_string());
        let scientist_traits = installed_content::scientist_traits::read(layers)
            .map(|(db, warnings)| {
                notes.extend(warnings.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let factions = installed_content::faction_templates::read(layers)
            .inspect(|db| notes.extend(db.warnings.iter().cloned()))
            .map_err(|why| why.to_string());
        Self {
            catalogue,
            unit_leaders,
            special_projects,
            scientist_traits,
            factions,
            notes,
        }
    }
}

impl Renderer<'_, '_, '_> {
    pub(super) fn character_content(&self) -> Result<&CharacterContent, String> {
        Ok(&self.context.text.content().effects()?.characters)
    }

    pub(super) fn char_country_traits(
        &self,
    ) -> Result<&installed_content::leader_traits::LeaderTraits, String> {
        self.context.text.content().environment()?.leader_traits()
    }

    pub(super) fn char_terrain(&self) -> Result<&installed_content::terrain::TerrainDefinitions, String> {
        self.context.text.content().environment()?.terrain()
    }

    pub(super) fn char_defines(&self) -> Result<&installed_content::defines::Defines, String> {
        Ok(self.context.text.content().environment()?.defines())
    }
}
