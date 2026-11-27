use super::*;

pub(in crate::effect_description) struct CharacterContent {
    pub(in crate::effect_description) catalogue:
        Result<Arc<content_reader_key_names::characters::Characters>, String>,
    pub(super) unit_leaders: Result<content_reader_key_names::unit_leader::Leaders<String>, String>,
    pub(in crate::effect_description) special_projects:
        Result<content_reader_military::special_projects::Catalog, String>,
    pub(super) scientist_traits: Result<content_reader_military::scientist_traits::ScientistTraits, String>,
    pub(super) factions: Result<content_reader_politics::faction_templates::Catalog, String>,
    pub(super) mio_traits: Result<content_reader_military::mio_traits::Catalog, String>,
    pub(in crate::effect_description) name_groups:
        Result<content_reader_raids::raids::names::Groups, String>,
    pub(in crate::effect_description) notes: Vec<String>,
}

impl CharacterContent {
    pub(in crate::effect_description) fn read<'s>(
        threads: &'s content_reader_layers::parallel::Scope<'s, '_>,
        layers: &'s content_reader_layers::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
        officers: Result<&[String], String>,
    ) -> Self {
        let mio_traits = crate::effect_description::alongside(threads, || {
            content_reader_military::mio_traits::read(layers)
        });
        let name_groups = crate::effect_description::alongside(threads, || {
            content_reader_raids::raids::names::Groups::read(layers)
        });
        let special_projects = crate::effect_description::alongside(threads, || {
            content_reader_military::special_projects::Catalog::read(layers)
        });
        let scientist_traits = crate::effect_description::alongside(threads, || {
            content_reader_military::scientist_traits::read(layers)
        });
        let factions = crate::effect_description::alongside(threads, || {
            content_reader_politics::faction_templates::read(layers)
        });
        let mut notes = Vec::new();
        let catalogue = (|| {
            let catalogue = content_reader_politics::ideas::characters(layers, officers?)
                .map_err(|why| why.to_string())?;
            notes.extend(catalogue.warnings.iter().map(|(_, why)| why.clone()));
            Ok::<_, String>(Arc::clone(&catalogue.characters))
        })();
        let unit_leaders = content_reader_key_names::unit_leader::Leaders::<String>::load(layers, &|name| {
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
        let special_projects = crate::effect_description::joined(special_projects)
            .inspect(|db| notes.extend(db.warnings.iter().cloned()))
            .map_err(|why| why.to_string());
        let scientist_traits = crate::effect_description::joined(scientist_traits)
            .map(|(db, warnings)| {
                notes.extend(warnings.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let factions = crate::effect_description::joined(factions)
            .inspect(|db| notes.extend(db.warnings.iter().cloned()))
            .map_err(|why| why.to_string());
        let mio_traits = crate::effect_description::joined(mio_traits)
            .inspect(|db| notes.extend(db.warnings.iter().cloned()))
            .map_err(|why| why.to_string());
        let name_groups = crate::effect_description::joined(name_groups)
            .inspect(|db| notes.extend(db.warnings.iter().map(|(_, why)| why.clone())))
            .map_err(|why| why.to_string());
        Self {
            catalogue,
            unit_leaders,
            special_projects,
            scientist_traits,
            factions,
            mio_traits,
            name_groups,
            notes,
        }
    }
}

impl Renderer<'_, '_, '_> {
    pub(in crate::effect_description) fn character_content(
        &self,
    ) -> Result<&CharacterContent, String> {
        Ok(&self.context.text.content().effects()?.characters)
    }

    pub(in crate::effect_description) fn char_country_traits(
        &self,
    ) -> Result<&content_reader_military::leader_traits::LeaderTraits, String> {
        self.context.text.content().environment()?.leader_traits()
    }

    pub(super) fn char_terrain(
        &self,
    ) -> Result<&content_reader_key_names::terrain::TerrainDefinitions, String> {
        self.context.text.content().environment()?.terrain()
    }

    pub(super) fn char_defines(&self) -> Result<&content_reader_defines::defines::Defines, String> {
        Ok(self.context.text.content().environment()?.defines())
    }
}
