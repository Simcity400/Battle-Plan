//! At-hour detail panel. Dynamic values use recorded rows; authored rewards and
//! effects use the bound content stack. Daily assignments retain their read date.
use super::*;
use dashboard_base::prepared::query::progressive;
use replay_data::rows::{CharacterRoleTraitRow, Recorded};
mod facility;
mod visibility;
pub use facility::*;

type DetailContentCache = Option<(Arc<PoliticsContent>, Arc<catalogue::CachedResult<Content>>)>;

fn detail_content(source: &Source, politics: Arc<PoliticsContent>) -> io::Result<Arc<Content>> {
    let state = source.state::<DetailContentCache>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state
        .as_ref()
        .is_some_and(|(bound, _)| Arc::ptr_eq(bound, &politics))
    {
        *state = Some((
            Arc::clone(&politics),
            Arc::new(catalogue::CachedResult::default()),
        ));
    }
    let result = Arc::clone(&state.as_ref().unwrap().1);
    drop(state);
    result.get_or_init(|| Content::load(politics))
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectLabel {
    pub token: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScientistSkill {
    pub specialization: ProjectLabel,
    pub level: i32,
    pub experience_raw: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScientistDetails {
    pub character: u32,
    pub person: ProjectPerson,
    pub active: Option<u8>,
    pub injury_days: Option<i32>,
    pub injury_hour: Option<i32>,
    pub visible: Option<bool>,
    pub visibility_hour: Option<i32>,
    pub state_refusals: BTreeMap<String, usize>,
    pub skills: Option<Vec<ScientistSkill>>,
    pub traits: Option<Vec<ProjectLabel>>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScientistAssignment {
    pub ordinal: u32,
    pub reference: String,
    pub person: ProjectPerson,
    pub target: i32,
    pub current: i32,
    pub timed: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProgramDetails {
    pub id: u32,
    pub province: u32,
    pub read_hour: i32,
    pub assignments: Option<Vec<ScientistAssignment>>,
    pub facility: Option<FacilityDetails>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectEffects {
    pub text: String,
    pub refusals: BTreeMap<String, usize>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRewardOption {
    pub label: ProjectLabel,
    pub effects: ProjectEffects,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PendingProjectReward {
    pub label: ProjectLabel,
    pub description: Option<String>,
    pub options: Vec<ProjectRewardOption>,
    pub unavailable: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVariableValue {
    pub name: String,
    pub value_raw: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectChoice {
    pub ordinal: u32,
    pub date: i32,
    pub reward: ProjectLabel,
    pub option: ProjectLabel,
    pub province: i32,
    pub progress: i32,
    pub character_reference: String,
    pub country: Option<u32>,
    pub scientist: Option<u32>,
    pub state: Option<u32>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpecializationProgress {
    pub specialization: ProjectLabel,
    pub points: i32,
    pub read_hour: i32,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectRuntimeDetails {
    pub owner: u32,
    pub id: u32,
    pub label: ProjectLabel,
    pub read_hour: i32,
    pub pending_known: bool,
    pub pending: Option<PendingProjectReward>,
    pub seed: Option<u32>,
    pub variables: Option<Vec<ProjectVariableValue>>,
    pub output: Option<ProjectEffects>,
    pub choices: Option<Vec<ProjectChoice>>,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SpecialProjectDetails {
    pub hour: i32,
    pub scientists: Vec<ScientistDetails>,
    pub programs: Vec<ProgramDetails>,
    /// `None` without a continuous supplying read for the country.
    pub breakthroughs: Option<Vec<SpecializationProgress>>,
    pub projects: Vec<ProjectRuntimeDetails>,
    pub unavailable: Vec<String>,
}

fn token(frame: &DecodedFrame, id: u32) -> Result<String, String> {
    let rows = frame
        .complete::<tables::LexicalTokens>()
        .map_err(|why| why.to_string())?;
    let value = rows
        .binary_search_by_key(&id, |row| row.token)
        .ok()
        .map(|at| &rows[at])
        .ok_or_else(|| format!("LexicalTokens lacks token {id}"))?;
    String::from_utf8(value.literal.clone()).map_err(|_| format!("Token {id} is not UTF-8"))
}
/// `text` is the hour's frame with the trigger kit, which scoped names read.
fn label(content: Option<&Content>, text: &DecodedFrame, slot: u32, token: String) -> ProjectLabel {
    ProjectLabel {
        name: content.and_then(|content| content.name_for(&token, Some((slot, text)))),
        token,
    }
}
fn reference_person(
    frame: &DecodedFrame,
    reference: u64,
    content: Option<&Content>,
) -> ProjectPerson {
    let mut person = ProjectPerson {
        assigned: Some(reference != 0),
        id: None,
        name: None,
        portrait: None,
        missing: None,
        portrait_choice: None,
    };
    if reference == 0 {
        return person;
    }
    if reference as u32 != 73 {
        person.missing = Some("Unknown scientist reference type".into());
        return person;
    }
    let id = (reference >> 32) as u32;
    person.id = Some(id);
    let character = frame
        .complete::<tables::Characters>()
        .ok()
        .and_then(|rows| {
            rows.binary_search_by_key(&id, |row| row.id)
                .ok()
                .map(|at| &rows[at])
        });
    if let Some(character) = character {
        let lexical = frame
            .complete::<tables::LexicalTokens>()
            .unwrap_or_default();
        let script = character_script_token(character, lexical);
        let saved = String::from_utf8_lossy(&character.token);
        person.name = content.and_then(|content| {
            content
                .politics
                .character_name_with_refusals(script, &saved, &content.refusals)
        });
        let portrait = recorded_portrait(
            frame.complete::<tables::CharacterPortraits>().ok(),
            id,
            4,
            replay_data::rows::PORTRAIT_LARGE,
        );
        // Drawn when the reply is served ([`ProjectPerson::portrait_choice`]).
        match content {
            Some(_) => person.portrait_choice = Some(portrait),
            None => person.missing = Some("Content is unavailable".into()),
        }
    } else {
        person.missing = Some(format!("Character {id} is not recorded"));
    }
    person
}
fn effects(
    content: &Content,
    frame: &DecodedFrame,
    slot: u32,
    project: &ProjectRow,
    output: &installed_content_core::special_projects::Output,
    program: Option<(&replay_data::rows::ProjectProgramRow, i32)>,
) -> ProjectEffects {
    let render = || -> Result<_, String> {
        let scripted = content.catalogue.scripted_content()?;
        let text = dashboard_text::scripted_text::ScriptedText::new(
            frame,
            &content.catalogue.localisation,
            scripted,
            slot,
        );
        let cache = scripted.effects()?;
        let context = dashboard_text::effect_description::Context::new(
            &text,
            &content.catalogue.localisation,
            slot,
            "special-project output",
            cache.view(None),
        );
        let scientist = match program {
            Some((program, read)) if read == frame.hour => match program.scientist {
                0 => Ok(None),
                reference if reference as u32 == 73 => Ok(Some((reference >> 32) as u32)),
                _ => Err("Unknown scientist reference type".into()),
            },
            _ => Err("Scientist assignment has no supplying read at this hour".into()),
        };
        let facility = (|| {
            if project.program_reference == 0 {
                return Ok(None);
            }
            if project.program_reference as u32 != 83 {
                return Err("Unknown program reference type".into());
            }
            let id = (project.program_reference >> 32) as u32;
            let facility = frame
                .complete::<tables::RaidFacilities>()
                .map_err(|why| why.to_string())?
                .iter()
                .find(|row| row.id == id)
                .ok_or("RaidFacilities lacks the selected project program")?;
            let provinces = frame
                .complete::<tables::Provinces>()
                .map_err(|why| why.to_string())?;
            let state = provinces
                .iter()
                .find(|row| row.id == facility.province)
                .ok_or("Provinces lacks the facility province")?
                .state;
            Ok(Some((facility.province, state)))
        })();
        Ok(
            dashboard_text::effect_description::project_output_description(
                output,
                &context,
                project.owner,
                project.local_id,
                scientist,
                facility,
            ),
        )
    };
    match render() {
        Ok(mut description) => {
            let text = match catalogue::plain_text(&description.text) {
                Ok(text) => text,
                Err(why) => {
                    *description.refusals.entry(why).or_default() += 1;
                    String::new()
                }
            };
            ProjectEffects {
                text,
                refusals: description.refusals,
            }
        }
        Err(why) => ProjectEffects {
            text: String::new(),
            refusals: BTreeMap::from([(why, 1)]),
        },
    }
}

pub fn details_at_hour(
    source: &Source,
    context: &LoadContext,
    params: &SpecialProjectsParams,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<SpecialProjectDetails> {
    let mut result = SpecialProjectDetails {
        hour,
        scientists: Vec::new(),
        programs: Vec::new(),
        breakthroughs: None,
        projects: Vec::new(),
        unavailable: Vec::new(),
    };
    let branch = params.country().branch();
    let tables = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SliceReads,
        TableId::Projects,
        TableId::ProjectHistory,
        TableId::Scientists,
        TableId::ScientistDaily,
        TableId::ScientistSkills,
        TableId::CharacterRoleTraits,
        TableId::Characters,
        TableId::CharacterPortraits,
        TableId::LexicalTokens,
    ];
    let frames = progressive::read_window(
        source,
        branch,
        hour.saturating_sub(replay_data::cadence::STAGGER_PERIOD - 1),
        hour,
        &tables,
        cancelled,
    )?;
    let Some(frame) = frames.last().filter(|frame| frame.hour == hour) else {
        result
            .unavailable
            .push("The requested hour is not recorded".into());
        return Ok(result);
    };
    let identity = |frame: &DecodedFrame| {
        frame
            .complete::<tables::Countries>()
            .ok()
            .zip(frame.complete::<tables::CountryLinks>().ok())
            .and_then(|(countries, links)| {
                dashboard_text::metadata::tagged_country(countries, links, &params.tag).found()
            })
            .map(|(country, slot)| (country.id, slot))
    };
    let Some((country, slot)) = identity(frame) else {
        result
            .unavailable
            .push("Country identity is unavailable".into());
        return Ok(result);
    };
    let content = match context
        .content()
        .and_then(|options| politics_content(source, options))
        .and_then(|politics| detail_content(source, politics))
    {
        Ok(content) => Some(content),
        Err(why) => {
            result.unavailable.push(format!(
                "Recorded special-project content is unavailable: {why}"
            ));
            None
        }
    };
    // Scoped names and effect descriptions read the trigger kit at the hour.
    let mut effect_tables = dashboard_text::triggers::RECORDED_TABLES.to_vec();
    effect_tables.extend([
        TableId::RaidFacilities,
        TableId::Provinces,
        TableId::ProjectPrograms,
    ]);
    effect_tables.sort_unstable();
    effect_tables.dedup();
    let effect_frame = if content.is_some() {
        progressive::frame_at(source, branch, hour, &effect_tables, cancelled)?
    } else {
        None
    };
    let text = effect_frame.as_deref().unwrap_or(frame);
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|why| dashboard_core::invalid(why.to_string()))?;
    if let Ok(scientists) = frame.complete::<tables::Scientists>() {
        let skills = frame.complete::<tables::ScientistSkills>().ok();
        let traits = frame.complete::<tables::CharacterRoleTraits>().ok();
        for scientist in scientists
            .iter()
            .filter(|row| dashboard_text::metadata::link_slot(links, row.country) == Some(slot))
        {
            let skill_values = skills
                .map(|rows| {
                    rows.iter()
                        .filter(|row| row.character == scientist.character)
                        .map(|row| {
                            token(frame, row.token).map(|token| ScientistSkill {
                                specialization: label(content.as_deref(), text, slot, token),
                                level: row.level,
                                experience_raw: row.experience_raw.to_string(),
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose();
            let trait_values = traits
                .map(|rows| {
                    rows.iter()
                        .filter(|row| {
                            row.character == scientist.character
                                && row.kind == CharacterRoleTraitRow::SCIENTIST
                        })
                        .flat_map(|row| &row.tokens)
                        .map(|&id| {
                            token(frame, id)
                                .map(|token| label(content.as_deref(), text, slot, token))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose();
            result.scientists.push(ScientistDetails {
                character: scientist.character,
                person: reference_person(
                    frame,
                    (u64::from(scientist.character) << 32) | 73,
                    content.as_deref(),
                ),
                active: match scientist.active {
                    Recorded::Value(active) => active,
                    _ => None,
                },
                injury_days: None,
                injury_hour: None,
                visible: None,
                visibility_hour: None,
                state_refusals: BTreeMap::new(),
                skills: skill_values.unwrap_or_else(|why| {
                    result.unavailable.push(why);
                    None
                }),
                traits: trait_values.unwrap_or_else(|why| {
                    result.unavailable.push(why);
                    None
                }),
            });
        }
    } else {
        result.unavailable.push("Scientists is unavailable".into());
    }
    // Injury and visibility are independent observations in the scientist rows.
    // A refused program roster cannot withdraw either successfully read fact.
    for scientist in &mut result.scientists {
        let mut expected_hour = hour;
        let mut injury_seen = false;
        let mut visibility_seen = false;
        for sample in frames.iter().rev() {
            if sample.hour != expected_hour || identity(sample) != Some((country, slot)) {
                break;
            }
            expected_hour = expected_hour.saturating_sub(1);
            let Ok(rows) = sample.complete::<tables::Scientists>() else {
                break;
            };
            let Ok(at) = rows.binary_search_by_key(&scientist.character, |row| row.character)
            else {
                break;
            };
            let roster = &rows[at];
            if dashboard_text::metadata::link_slot(links, roster.country) != Some(slot) {
                break;
            }
            if !dashboard_text::metadata::slot_read(
                source.replay_ref().format,
                &sample.metrics,
                TableId::ScientistDaily,
                sample.hour,
            ) {
                continue;
            }
            let Ok(daily_rows) = sample.complete::<tables::ScientistDaily>() else {
                break;
            };
            let Ok(at) = daily_rows.binary_search_by_key(&scientist.character, |row| row.character)
            else {
                break;
            };
            let daily = &daily_rows[at];
            if daily.country != roster.country {
                break;
            }
            if !injury_seen && let Recorded::Value(days) = daily.injury_days {
                injury_seen = true;
                scientist.injury_hour = Some(sample.hour);
                scientist.injury_days = days;
            }
            if !visibility_seen && matches!(daily.template_visibility, Recorded::Value(_)) {
                visibility_seen = true;
                match visibility::at_observation(
                    source,
                    params,
                    sample,
                    daily,
                    content.as_deref(),
                    slot,
                    cancelled,
                ) {
                    Ok(value) => {
                        scientist.visible = Some(value);
                        scientist.visibility_hour = Some(sample.hour);
                    }
                    Err(reason) => {
                        scientist.state_refusals.insert(reason, 1);
                    }
                }
            }
            if injury_seen && visibility_seen {
                break;
            }
        }
    }
    let breakthroughs = progressive::latest_read(
        source,
        branch,
        hour,
        TableId::ProjectBreakthroughs,
        replay_data::cadence::STAGGER_PERIOD,
        &[
            TableId::ProjectBreakthroughs,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LexicalTokens,
        ],
        cancelled,
    )?;
    match breakthroughs
        .as_ref()
        .filter(|sample| identity(sample) == Some((country, slot)))
    {
        Some(sample) => match sample.complete::<tables::ProjectBreakthroughs>() {
            Ok(progress) => {
                let sample_links = sample
                    .complete::<tables::CountryLinks>()
                    .unwrap_or_default();
                let mut values = Vec::new();
                for row in progress.iter().filter(|row| {
                    dashboard_text::metadata::link_slot(sample_links, row.owner) == Some(slot)
                }) {
                    match token(sample, row.specialization) {
                        Ok(token) => values.push(SpecializationProgress {
                            specialization: label(content.as_deref(), text, slot, token),
                            points: row.points,
                            read_hour: sample.hour,
                        }),
                        Err(reason) => result.unavailable.push(reason),
                    }
                }
                result.breakthroughs = Some(values);
            }
            Err(why) => result
                .unavailable
                .push(format!("Specialization progress is unavailable: {why}")),
        },
        None => result
            .unavailable
            .push("Specialization progress has no continuous supplying daily read".into()),
    }
    let programs = progressive::latest_read(
        source,
        branch,
        hour,
        TableId::ProjectPrograms,
        replay_data::cadence::STAGGER_PERIOD,
        &[
            TableId::ProjectPrograms,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Characters,
            TableId::CharacterPortraits,
            TableId::LexicalTokens,
            TableId::ModifierDefinitions,
        ],
        cancelled,
    )?;
    if let Some(program_frame) = programs
        .as_ref()
        .filter(|program_frame| identity(program_frame) == Some((country, slot)))
    {
        if let Ok(programs) = program_frame.complete::<tables::ProjectPrograms>() {
            let program_links = program_frame
                .complete::<tables::CountryLinks>()
                .unwrap_or_default();
            for program in programs.iter().filter(|row| {
                dashboard_text::metadata::link_slot(program_links, row.owner) == Some(slot)
            }) {
                let assignments = match &program.assignments {
                    Recorded::Value(Some(values)) => Some(
                        values
                            .iter()
                            .enumerate()
                            .map(|(ordinal, value)| ScientistAssignment {
                                ordinal: ordinal as u32,
                                reference: value.scientist.to_string(),
                                person: reference_person(
                                    program_frame,
                                    value.scientist,
                                    content.as_deref(),
                                ),
                                target: value.target,
                                current: value.current,
                                timed: value.timed,
                            })
                            .collect(),
                    ),
                    _ => None,
                };
                result.programs.push(ProgramDetails {
                    id: program.id,
                    province: program.province,
                    read_hour: program_frame.hour,
                    assignments,
                    facility: facility::present(
                        program_frame,
                        text,
                        program,
                        content.as_deref(),
                        slot,
                    ),
                });
            }
        }
    } else {
        result
            .unavailable
            .push("Program assignments have no continuous supplying daily read".into());
    }
    // Admit sliced project state only after seeing its supplying read, with the same holder throughout.
    let mut previous = None;
    let mut admitted = false;
    let mut read_hour = None;
    for sample in &frames {
        if previous.and_then(|last: i32| last.checked_add(1)) != Some(sample.hour)
            || identity(sample) != Some((country, slot))
        {
            admitted = false;
        }
        previous = Some(sample.hour);
        let read = sample
            .complete::<tables::SliceReads>()
            .ok()
            .and_then(|rows| replay_data::sliced::read_hour(rows, TableId::Projects, country));
        if read == Some(sample.hour) {
            admitted = true;
        }
        let owner_reads_match = sample
            .complete::<tables::SliceReads>()
            .ok()
            .zip(sample.complete::<tables::Projects>().ok())
            .zip(sample.complete::<tables::CountryLinks>().ok())
            .is_some_and(|((reads, projects), links)| {
                projects
                    .iter()
                    .filter(|row| {
                        dashboard_text::metadata::link_slot(links, row.owner) == Some(slot)
                    })
                    .all(|row| {
                        replay_data::sliced::read_hour(reads, TableId::Projects, row.owner) == read
                    })
            });
        if !owner_reads_match {
            admitted = false;
        }
        if read.is_none_or(|read| {
            read > sample.hour
                || sample.hour.saturating_sub(read) >= replay_data::cadence::STAGGER_PERIOD
        }) || sample.complete::<tables::Projects>().is_err()
        {
            admitted = false;
        }
        read_hour = read;
    }
    if !admitted {
        result
            .unavailable
            .push("Projects has no continuous supplying read".into());
        return Ok(result);
    }
    for project in frame
        .complete::<tables::Projects>()
        .unwrap_or_default()
        .iter()
        .filter(|row| dashboard_text::metadata::link_slot(links, row.owner) == Some(slot))
    {
        let project_token = match token(frame, project.definition_id) {
            Ok(token) => token,
            Err(why) => {
                result.unavailable.push(why);
                continue;
            }
        };
        let runtime = match &project.runtime {
            Recorded::Value(runtime) => Some(runtime),
            _ => None,
        };
        let variables = runtime
            .and_then(|runtime| runtime.variables.as_ref())
            .map(|store| {
                store
                    .entries
                    .iter()
                    .map(|entry| {
                        String::from_utf8(entry.name.clone()).map(|name| ProjectVariableValue {
                            name,
                            value_raw: entry.value_raw.to_string(),
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()
            .map_err(|_| dashboard_core::invalid("Project variable name is not UTF-8"))?;
        let program = programs.as_ref().and_then(|frame| {
            frame
                .complete::<tables::ProjectPrograms>()
                .ok()
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| {
                            project.program_reference as u32 == 83
                                && row.id == (project.program_reference >> 32) as u32
                        })
                        .map(|row| (row, frame.hour))
                })
        });
        let render = |output: &installed_content_core::special_projects::Output| {
            content
                .as_ref()
                .zip(effect_frame.as_deref())
                .map(|(content, frame)| effects(content, frame, slot, project, output, program))
        };
        let definition = content.as_ref().and_then(|content| {
            content
                .definitions
                .projects
                .iter()
                .find(|(token, _)| token.eq_ignore_ascii_case(&project_token))
                .map(|(_, definition)| definition)
        });
        let output = definition.and_then(|definition| render(&definition.output));
        let pending = runtime
            .and_then(|runtime| runtime.pending_reward)
            .map(|id| {
                let reward_token = token(frame, id);
                match reward_token.and_then(|reward_token| {
                    let content = content.as_ref().ok_or("Recorded content is unavailable")?;
                    let ordinal = runtime
                        .and_then(|value| value.pending_ordinal)
                        .ok_or("Pending reward object occurrence is unavailable")?;
                    let reward =
                        content
                            .definitions
                            .reward_at(&project_token, ordinal, &reward_token)?;
                    Ok(PendingProjectReward {
                        label: ProjectLabel {
                            name: content.name_for(
                                reward.name.as_deref().unwrap_or(&reward_token),
                                Some((slot, text)),
                            ),
                            token: reward_token,
                        },
                        description: reward
                            .description
                            .as_ref()
                            .and_then(|key| content.name_for(key, Some((slot, text)))),
                        options: reward
                            .options
                            .iter()
                            .map(|option| ProjectRewardOption {
                                label: ProjectLabel {
                                    token: option.token.clone(),
                                    name: content.name_for(
                                        option.name.as_deref().unwrap_or(&option.token),
                                        Some((slot, text)),
                                    ),
                                },
                                effects: render(&option.output).unwrap_or_else(|| ProjectEffects {
                                    text: String::new(),
                                    refusals: BTreeMap::from([(
                                        "Recorded effect inputs are unavailable".into(),
                                        1,
                                    )]),
                                }),
                            })
                            .collect(),
                        unavailable: None,
                    })
                }) {
                    Ok(reward) => reward,
                    Err(why) => PendingProjectReward {
                        label: ProjectLabel {
                            token: id.to_string(),
                            name: None,
                        },
                        description: None,
                        options: Vec::new(),
                        unavailable: Some(why),
                    },
                }
            });
        result.projects.push(ProjectRuntimeDetails {
            owner: project.owner,
            id: project.local_id,
            label: ProjectLabel {
                name: content
                    .as_ref()
                    .zip(definition)
                    .and_then(|(content, definition)| {
                        let selected = country_key(
                            &content.catalogue,
                            &params.tag,
                            project_name_key(definition, &project_token),
                        );
                        content.name_for(&selected, Some((slot, text)))
                    }),
                token: project_token,
            },
            read_hour: read_hour.unwrap(),
            pending_known: runtime.is_some_and(|runtime| runtime.pending_known),
            pending,
            seed: runtime
                .and_then(|runtime| runtime.variables.as_ref())
                .map(|store| store.seed),
            variables,
            output,
            choices: frame.complete::<tables::ProjectHistory>().ok().map(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.owner == project.owner
                            && row.project == project.local_id
                            && !(row.prototype_reward_handle == COMPLETION_MARKER
                                && row.option_handle == COMPLETION_MARKER)
                    })
                    .map(|row| {
                        let lexical_label = |id| {
                            label(
                                content.as_deref(),
                                text,
                                slot,
                                token(frame, id).unwrap_or_else(|_| id.to_string()),
                            )
                        };
                        ProjectChoice {
                            ordinal: row.ordinal,
                            date: row.date,
                            reward: lexical_label(row.prototype_reward_handle),
                            option: lexical_label(row.option_handle),
                            province: row.context.province,
                            progress: row.context.progress,
                            character_reference: row.context.character_reference.to_string(),
                            country: row.context.country_reference,
                            scientist: row.context.scientist_reference,
                            state: row.context.state_reference,
                        }
                    })
                    .collect()
            }),
        });
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, file_source, records, source};
    use replay::container::Replay;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    #[test]
    fn gap_research_grants_project_details_keep_empty_projects_scientists_skills_traits_assignments_and_gap_unknowns()
     {
        let mut frame = source(1, 24, None);
        frame.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        // This fixture represents a forced program read outside its daily slot.
        frame.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        *frame.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(Vec::new()));
        *frame.tables[TableId::SliceReads.index()] =
            TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
                table: TableId::Projects.index() as u32,
                owner: 9001,
                sample_hour: 24,
            }]));
        *frame.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 9,
                owner: 11,
                assignments: Recorded::Value(Some(vec![
                    ProjectScientistAssignment::default(),
                    ProjectScientistAssignment {
                        scientist: (7u64 << 32) | 73,
                        target: 6,
                        current: 2,
                        timed: 1,
                    },
                ])),
                ..Default::default()
            }]));
        *frame.tables[TableId::Scientists.index()] =
            TableView::Complete(TableRows::Scientists(vec![ScientistRow {
                character: 7,
                country: 11,
                active: Recorded::Value(Some(1)),
                ..Default::default()
            }]));
        *frame.tables[TableId::ScientistDaily.index()] =
            TableView::Complete(TableRows::ScientistDaily(vec![ScientistDailyRow {
                character: 7,
                country: 11,
                injury_days: Recorded::Value(Some(8)),
                ..Default::default()
            }]));
        *frame.tables[TableId::ScientistSkills.index()] =
            TableView::Complete(TableRows::ScientistSkills(vec![ScientistSkillRow {
                character: 7,
                token: 41,
                level: -2,
                experience_raw: i64::MAX,
                ..Default::default()
            }]));
        *frame.tables[TableId::CharacterRoleTraits.index()] =
            TableView::Complete(TableRows::CharacterRoleTraits(vec![
                CharacterRoleTraitRow {
                    character: 7,
                    kind: CharacterRoleTraitRow::SCIENTIST,
                    tokens: vec![42, 42],
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 41,
                    literal: b"mod_skill".to_vec(),
                },
                LexicalTokenRow {
                    token: 42,
                    literal: b"mod_trait".to_vec(),
                },
            ]));
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
        let (source, root) = file_source(replay);
        let params = SpecialProjectsParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            selection_tags: Vec::new(),
            hour: Some(24),
        };
        let value = details_at_hour(
            &source,
            &LoadContext { content: None },
            &params,
            24,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(value.projects.is_empty());
        assert_eq!(value.scientists.len(), 1);
        assert_eq!(value.scientists[0].active, Some(1));
        assert_eq!(value.scientists[0].injury_days, Some(8));
        assert_eq!(
            value.scientists[0].skills.as_ref().unwrap()[0].experience_raw,
            i64::MAX.to_string()
        );
        assert_eq!(value.scientists[0].traits.as_ref().unwrap().len(), 2);
        assert_eq!(
            value.programs[0].assignments.as_ref().unwrap()[1].current,
            2
        );
        assert_eq!(value.programs[0].read_hour, 24);
        let missing = details_at_hour(
            &source,
            &LoadContext { content: None },
            &params,
            25,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(missing.scientists.is_empty());
        assert!(!missing.unavailable.is_empty());
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn refused_program_roster_keeps_independent_scientist_and_breakthrough_observations() {
        let mut daily = source(1, 24, None);
        daily.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        daily.metrics.table_ns[TableId::ProjectBreakthroughs.index()] = 1;
        *daily.tables[TableId::ProjectPrograms.index()] = TableView::Unavailable {
            reason: "Synthetic program roster refusal".into(),
        };
        *daily.tables[TableId::Projects.index()] = TableView::Unavailable {
            reason: "Synthetic project pool refusal".into(),
        };
        *daily.tables[TableId::Scientists.index()] =
            TableView::Complete(TableRows::Scientists(vec![ScientistRow {
                character: 7,
                country: 11,
                active: Recorded::Value(Some(0)),
                ..Default::default()
            }]));
        *daily.tables[TableId::ScientistDaily.index()] =
            TableView::Complete(TableRows::ScientistDaily(vec![ScientistDailyRow {
                character: 7,
                country: 11,
                injury_days: Recorded::Value(Some(8)),
                template_visibility: Recorded::Value(Some(2)),
                ..Default::default()
            }]));
        *daily.tables[TableId::ProjectBreakthroughs.index()] =
            TableView::Complete(TableRows::ProjectBreakthroughs(vec![
                ProjectBreakthroughRow {
                    owner: 11,
                    specialization: 41,
                    points: 777,
                },
            ]));
        *daily.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 41,
                literal: b"mod_skill".to_vec(),
            }]));
        let mut next = daily.clone();
        next.hour = 25;
        next.metrics.table_ns[TableId::ProjectPrograms.index()] = 0;
        next.metrics.table_ns[TableId::ProjectBreakthroughs.index()] = 0;
        *next.tables[TableId::ScientistDaily.index()] = TableView::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON.into(),
        };
        let mut refused_daily = next.clone();
        refused_daily.hour = 26;
        refused_daily.metrics.table_ns[TableId::ProjectPrograms.index()] = 1;
        *refused_daily.tables[TableId::ScientistDaily.index()] =
            TableView::Complete(TableRows::ScientistDaily(vec![ScientistDailyRow {
                character: 7,
                country: 11,
                injury_days: Recorded::Value(None),
                template_visibility: Recorded::Value(None),
                template_origin: Recorded::Value(None),
            }]));
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![daily, next, refused_daily])],
        )
        .unwrap();
        let (source, root) = file_source(replay);
        let params = SpecialProjectsParams {
            branch_id: "1".into(),
            tag: "GER".into(),
            selection_tags: Vec::new(),
            hour: Some(25),
        };
        let value = details_at_hour(
            &source,
            &LoadContext { content: None },
            &params,
            25,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(value.programs.is_empty());
        assert_eq!(value.scientists[0].injury_days, Some(8));
        assert_eq!(value.scientists[0].injury_hour, Some(24));
        assert_eq!(value.scientists[0].visible, Some(true));
        assert_eq!(value.scientists[0].visibility_hour, Some(24));
        let breakthroughs = value.breakthroughs.as_ref().unwrap();
        assert_eq!(breakthroughs[0].points, 777);
        assert_eq!(breakthroughs[0].read_hour, 24);
        // A witnessed failed native read is not an off-slice hour: it must
        // not borrow the older successful scientist operands.
        let refused = details_at_hour(
            &source,
            &LoadContext { content: None },
            &params,
            26,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(refused.scientists[0].injury_hour, Some(26));
        assert_eq!(refused.scientists[0].injury_days, None);
        assert_eq!(refused.scientists[0].visible, None);
        assert_eq!(refused.scientists[0].visibility_hour, None);
        assert!(!refused.scientists[0].state_refusals.is_empty());
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
