//! Country module unlock joins use existing research and completed-project rows.
use super::queries::spelling;
use super::*;
use dashboard_text::triggers::{Scopes, World};
use installed_content_core::{parser::Item, tokens::Token};

pub struct Content {
    pub technologies: installed_content_core::technology::Catalog,
    pub projects: installed_content_core::special_projects::Catalog,
}
impl Content {
    pub fn load(layers: &installed_content_core::ResolvedLayers) -> std::io::Result<Self> {
        let names = installed_content_core::key_names::KeyNames::read(layers)?;
        Ok(Self {
            technologies: installed_content_core::technology::Catalog::read(layers, &names)?,
            projects: installed_content_core::special_projects::Catalog::read(layers)?,
        })
    }
}
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unlock {
    pub sources: Vec<String>,
    pub refusals: Vec<String>,
}
impl Unlock {
    pub fn unlocked(&self) -> bool {
        !self.sources.is_empty()
    }
}

fn contains(items: &[Item], module: &[u8]) -> Result<bool, String> {
    for item in items {
        let name = item
            .value
            .scalar()
            .ok_or("module unlock token is not scalar (count: 1)")?;
        if Token::c_str(name) == Token::c_str(module) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns only the requested module's enablers, without a country/design matrix.
/// Structural admission and current module country gates remain separate queries.
pub fn module_unlock(
    frame: &DecodedFrame,
    world: &World<'_>,
    country: u32,
    module: &[u8],
    content: &Content,
) -> Result<Unlock, String> {
    if country == 0 {
        return Ok(Unlock::default());
    }
    let slot = world
        .slot(country)
        .ok_or("module unlock country is not recorded (count: 1)")?;
    if !queries::module_admission(world, country, module, &content.technologies.equipment)? {
        return Ok(Unlock::default());
    }
    let mut result = Unlock::default();
    if let Some(reason) = &content.technologies.refusal {
        result.refusals.push(reason.clone());
    }
    for (index, definition) in content.technologies.definitions.iter().enumerate() {
        if !contains(&definition.modules, module)? {
            continue;
        }
        let state = (|| {
            let definitions = frame
                .complete::<tables::TechnologyDefinitions>()
                .map_err(|e| e.to_string())?;
            let research = frame
                .complete::<tables::ResearchTechnologies>()
                .map_err(|e| e.to_string())?;
            let occurrence = content.technologies.definitions[..index]
                .iter()
                .filter(|previous| Token::c_str(&previous.id) == Token::c_str(&definition.id))
                .count();
            let recorded = definitions.iter().filter(|row| Token::c_str(&row.id) == Token::c_str(&definition.id)).nth(occurrence)
                .ok_or_else(|| format!("module unlock technology {} occurrence {} identity is not recorded (count: 1)", String::from_utf8_lossy(&definition.id), occurrence))?;
            let row = research
                .iter()
                .find(|row| {
                    world.slot(row.country) == Some(slot)
                        && row.database_index == recorded.database_index
                })
                .ok_or_else(|| {
                    format!(
                        "module unlock technology {} country state is not recorded (count: 1)",
                        String::from_utf8_lossy(&definition.id)
                    )
                })?;
            Ok::<_, String>(row.applied_bonus_count != 0)
        })();
        match state {
            Ok(true) => result.sources.push(format!(
                "Technology {}",
                String::from_utf8_lossy(&definition.id)
            )),
            Ok(false) => {}
            Err(reason) => result.refusals.push(reason),
        }
    }
    if let Some(reason) = &content.projects.unsettled {
        result.refusals.push(reason.clone());
    }
    let mut project_rows: Option<&[rows::ProjectRow]> = None;
    for (name, definition) in &content.projects.projects {
        for list in &definition.output.enable_equipment_modules {
            let items = list.value.items().unwrap_or_default();
            let operands = items
                .iter()
                .filter(|item| !item.keyword_is("limit"))
                .cloned()
                .collect::<Vec<_>>();
            if !contains(&operands, module)? {
                continue;
            }
            let limits = items
                .iter()
                .filter(|item| item.keyword_is("limit"))
                .flat_map(|item| item.value.items().unwrap_or_default().iter().cloned())
                .collect::<Vec<_>>();
            let state = (|| {
                if !limits.is_empty() && !world.evaluate(&limits, Scopes::empty()).decided()? {
                    return Ok(false);
                }
                let projects = match project_rows {
                    Some(rows) => rows,
                    None => *project_rows.insert(
                        frame
                            .complete::<tables::Projects>()
                            .map_err(|e| e.to_string())?,
                    ),
                };
                for project in projects
                    .iter()
                    .filter(|row| world.slot(row.owner) == Some(slot) && row.completed != 0)
                {
                    if Token::c_str(spelling(frame, project.definition_id)?)
                        == Token::c_str(name.as_bytes())
                    {
                        return Ok(true);
                    }
                }
                Ok::<_, String>(false)
            })();
            match state {
                Ok(true) => result.sources.push(format!("Project {name}")),
                Ok(false) => {}
                Err(reason) => result
                    .refusals
                    .push(format!("module unlock project {name}: {reason} (count: 1)")),
            }
        }
    }
    // With no authored enabler the native enabled-module roster has no entry.
    // Dynamic module admission is still evaluated independently below.
    if !result.unlocked() && !result.refusals.is_empty() {
        return Err(result.refusals.join("; "));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_text::triggers::ContentFacts;
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{Metrics, TableView};
    fn frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture missing input".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                rows::CountryLinkRow {
                    id: 5_000,
                    canonical_slot: 1,
                },
                rows::CountryLinkRow {
                    id: 4_999,
                    canonical_slot: 2,
                },
            ]));
        *frame.tables[TableId::TechnologyDefinitions.index()] =
            TableView::Complete(TableRows::TechnologyDefinitions(vec![
                rows::TechnologyDefinitionRow {
                    database_index: 7,
                    technology_handle: 99,
                    id: b"first".to_vec(),
                    ..Default::default()
                },
                rows::TechnologyDefinitionRow {
                    database_index: 8,
                    technology_handle: 99,
                    id: b"second".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::ResearchTechnologies.index()] =
            TableView::Complete(TableRows::ResearchTechnologies(vec![
                rows::ResearchTechnologyRow {
                    country: 5_000,
                    database_index: 7,
                    applied_bonus_count: 0,
                    ..Default::default()
                },
                rows::ResearchTechnologyRow {
                    country: 5_000,
                    database_index: 8,
                    applied_bonus_count: 1,
                    ..Default::default()
                },
                rows::ResearchTechnologyRow {
                    country: 4_999,
                    database_index: 7,
                    applied_bonus_count: 1,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![]));
        frame
    }
    #[test]
    fn unlock_joins_exact_research_occurrence_owner_and_completed_project_without_guessing_missing_state()
     {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let equipment =
            crate::test_fixtures::equipment_types(&[b"equipment_modules={m={category=x}}"])
                .unwrap();
        let mut technologies = installed_content_core::technology::Catalog::default();
        technologies.equipment = equipment;
        let mut content = Content {
            technologies,
            projects: Default::default(),
        };
        for name in ["first", "second"] {
            content
                .technologies
                .definitions
                .push(installed_content_core::technology::Definition {
                    id: name.as_bytes().to_vec(),
                    modules: installed_content_core::parser::parse(b"m").unwrap().items,
                    ..Default::default()
                });
        }
        let dlcs = Default::default();
        let triggers = Default::default();
        let aliases = Default::default();
        let mut f = frame();
        let evaluate = |f: &DecodedFrame, c: &Content| {
            let world = World::read(f, ContentFacts::content_only(&dlcs, &triggers, &aliases));
            module_unlock(f, &world, 5_000, b"m", c)
        };
        assert_eq!(
            evaluate(&f, &content).unwrap().sources,
            ["Technology second"]
        );
        *f.tables[TableId::ResearchTechnologies.index()] =
            TableView::Complete(TableRows::ResearchTechnologies(vec![]));
        assert!(
            evaluate(&f, &content)
                .unwrap_err()
                .contains("first country state")
        );
        content.technologies.definitions.clear();
        let mut project = installed_content_core::special_projects::Project::default();
        project.output.enable_equipment_modules =
            installed_content_core::parser::parse(b"enable_equipment_modules={m limit={always=yes}}")
                .unwrap()
                .items;
        content.projects.projects.insert("p".into(), project);
        *f.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![rows::LexicalTokenRow {
                token: 42,
                literal: b"p".to_vec(),
            }]));
        *f.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![
            rows::ProjectRow {
                owner: 5_000,
                definition_id: 42,
                completed: 0,
                ..Default::default()
            },
            rows::ProjectRow {
                owner: 4_999,
                definition_id: 42,
                completed: 1,
                ..Default::default()
            },
        ]));
        assert!(!evaluate(&f, &content).unwrap().unlocked());
        let TableView::Complete(TableRows::Projects(projects)) =
            &mut *f.tables[TableId::Projects.index()]
        else {
            unreachable!()
        };
        projects[0].completed = 1;
        assert_eq!(evaluate(&f, &content).unwrap().sources, ["Project p"]);
    }
}
