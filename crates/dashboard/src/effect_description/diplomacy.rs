use super::*;
use replay::rows::tables;

impl Renderer<'_, '_, '_> {
    pub(super) fn war_or_annex(
        &self,
        item: &Item,
        scopes: Scopes,
        annex: bool,
    ) -> Result<String, String> {
        // 14037E9B0 / 1403799D0
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let source = self.country_name(self.current_country(scopes)?, false)?;
        let target = self.country_name(target, false)?;
        self.line(
            if annex {
                b"EFFECT_ANNEX_COUNTRY"
            } else {
                b"EFFECT_DECLARE_WAR"
            },
            &[
                ("NAME", Argument::Text(&source)),
                (
                    if annex { "TARGET" } else { "COUNTRY" },
                    Argument::Text(&target),
                ),
            ],
            scopes,
            true,
        )
    }

    fn faction_name(&self, faction: u64) -> Result<String, String> {
        let row = self
            .context
            .world()?
            .frame()
            .complete::<tables::Factions>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.id == faction)
            .ok_or_else(|| format!("Factions lacks {faction}"))?;
        let name = crate::scripted_text::stored_text(
            self.context.localisation,
            self.context.text.recorded_language(),
            &row.name,
            "faction name (140D77360)",
        )?;
        if !name.is_empty() {
            return Ok(name);
        }
        let token = row
            .template_token
            .ok_or("Factions has no name or template_token")?;
        let literal = self
            .context
            .world()?
            .frame()
            .complete::<tables::LexicalTokens>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.token == token)
            .ok_or_else(|| format!("LexicalTokens lacks faction template {token}"))?;
        self.context
            .localisation
            .localise(&literal.literal)
            .map_err(|why| why.to_string())
    }

    // 1404AC450: target first, then the stored ordered subject roster.
    pub(super) fn faction(&self, item: &Item, scopes: Scopes) -> Result<String, String> {
        let source = self.current_country(scopes)?;
        let Some(target) = self.country_target(item, scopes)? else {
            return Ok(String::new());
        };
        let world = self.context.world()?;
        let source_faction = world.faction(source)?;
        let target_faction = world.faction(target)?;
        let appearance = world
            .frame()
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(target))
            .ok_or("CountryAppearance lacks ordered target subjects")?;
        let subjects = appearance
            .subjects
            .iter()
            .map(|raw| {
                world
                    .recorded()
                    .slot(*raw as u32)
                    .ok_or_else(|| format!("CountryLinks lacks ordered subject {raw}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut body = String::new();
        if appearance.overlord > 0 {
            let master = world
                .recorded()
                .slot(appearance.overlord as u32)
                .ok_or("CountryLinks lacks target overlord")?;
            if source_faction.is_none() || world.faction(master)? != source_faction {
                let puppet = self.country_name(target, false)?;
                let master = self.country_name(master, true)?;
                body.push_str(&self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_END_PUPPET",
                    &[
                        ("PUPPET", Argument::Text(&puppet)),
                        ("MASTER", Argument::Text(&master)),
                    ],
                    scopes,
                    true,
                )?);
            }
        }
        let countries: Vec<_> = std::iter::once(target).chain(subjects).collect();
        if let Some(faction) = target_faction.filter(|faction| Some(*faction) != source_faction) {
            let faction = self.faction_name(faction)?;
            for &country in &countries {
                let country = self.country_name(country, false)?;
                body.push_str(&self.line(
                    b"EFFECT_REMOVE_FROM_FACTION_NAME",
                    &[
                        ("COUNTRY", Argument::Text(&country)),
                        ("FACTION", Argument::Text(&faction)),
                    ],
                    scopes,
                    true,
                )?);
            }
        }
        let join = match source_faction {
            None => Some(None),
            Some(faction) if world.recorded().faction_leader(source)? == Some(source) => {
                Some(Some(self.faction_name(faction)?))
            }
            Some(_) => None,
        };
        if let Some(faction) = join {
            for country in countries {
                let country = self.country_name(country, false)?;
                body.push_str(&match &faction {
                    None => self.line(
                        b"EFFECT_ADD_TO_FACTION",
                        &[("COUNTRY", Argument::Text(&country))],
                        scopes,
                        true,
                    )?,
                    Some(faction) => self.line(
                        b"EFFECT_ADD_TO_FACTION_NAME",
                        &[
                            ("COUNTRY", Argument::Text(&country)),
                            ("FACTION", Argument::Text(faction)),
                        ],
                        scopes,
                        true,
                    )?,
                });
            }
        }
        Ok(body)
    }
}
