//! A selected country's complete doctrine state at the requested replay hour.
use super::{LoadContext, Product, validate_tag};
use crate::{
    Source,
    catalogue::{self, View},
    content, invalid,
    metadata::check_cancel,
};
use replay::container::SeekResult;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct Doctrines;
pub const TABLES: [TableId; 13] = [
    TableId::Countries,
    TableId::UnitLeaders,
    TableId::Characters,
    TableId::CountryLinks,
    TableId::ModifierDefinitions,
    TableId::LexicalTokens,
    TableId::DoctrineCountries,
    TableId::DoctrineFolders,
    TableId::DoctrineTracks,
    TableId::DoctrineLeaderMastery,
    TableId::DoctrineCostReductions,
    TableId::DoctrineMasteryGrants,
    TableId::DoctrineEquipmentBonuses,
];

fn selected_tables() -> Vec<TableId> {
    // The shared scoped-text/trigger inputs are also used by focus artwork and
    // event descriptions. Authored effects and generated prose can reach all
    // of them, including through scripted effects and localisation.
    let mut selected = BTreeSet::from(TABLES);
    selected.extend(crate::scripted_text::TABLES.iter().copied());
    selected.insert(TableId::Clock);
    selected.into_iter().collect()
}

fn reward_key(sub: &[u8], reward: &[u8]) -> Vec<u8> {
    let mut key = installed_content_core::parser::c_string(sub).to_vec();
    key.push(b'_');
    key.extend_from_slice(installed_content_core::parser::c_string(reward));
    key
}

fn reward_benefits(
    reward: &installed_content_core::doctrines::RewardSource,
    key: &[u8],
    localisation: &installed_content_localisation::localisation::Localisation,
) -> installed_content_core::doctrines::Benefits {
    let mut benefits = reward.benefits.clone();
    let mut description = key.to_vec();
    description.extend_from_slice(b"_desc");
    if benefits.prose.is_none()
        && std::str::from_utf8(&description)
            .ok()
            .and_then(|key| localisation.get(key))
            .is_some()
    {
        benefits.prose = Some(installed_content_core::gui::native::BoundLocalization::from_value(
            &installed_content_core::parser::Value::Scalar(description),
        ));
    }
    benefits
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DoctrineParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineFact {
    pub name: String,
    pub value: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineReward {
    pub name: String,
    pub mastery: String,
    pub unlocked: Option<bool>,
    pub description: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineTrack {
    pub ordinal: u32,
    pub name: String,
    pub sub_doctrine: Option<String>,
    pub rewards: i32,
    pub mastery: String,
    pub mastery_bank: String,
    pub daily_mastery: String,
    pub unit_daily_mastery: String,
    pub reward_requirements: Vec<String>,
    pub benefits: String,
    pub authored_cost: String,
    pub reward_benefits: Vec<DoctrineReward>,
    pub commanders: Vec<DoctrineFact>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineFolder {
    pub ordinal: u32,
    pub name: String,
    pub grand_doctrine: Option<String>,
    pub benefits: String,
    pub authored_cost: String,
    pub milestones: Vec<DoctrineReward>,
    pub completed: Vec<String>,
    pub tracks: Vec<DoctrineTrack>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineInventory {
    pub kind: String,
    pub ordinal: u32,
    pub fields: Vec<DoctrineFact>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub folders: Vec<DoctrineFolder>,
    pub inventory: Vec<DoctrineInventory>,
    pub warnings: Vec<String>,
}

#[derive(Default)]
struct State {
    database: Option<(
        Arc<installed_content_core::content::ResolvedContent>,
        Arc<installed_content_core::doctrines::Database>,
    )>,
}
impl Product for Doctrines {
    const KEY: &'static str = "doctrines";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = DoctrineParams;
    type Reply = DoctrineReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid doctrine branch")
    }
    fn types() -> Vec<String> {
        vec![
            DoctrineParams::decl(),
            DoctrineFact::decl(),
            DoctrineReward::decl(),
            DoctrineTrack::decl(),
            DoctrineFolder::decl(),
            DoctrineInventory::decl(),
            DoctrineReply::decl(),
        ]
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        check_cancel(cancelled)?;
        let options = context.content()?;
        let resolved = source.content(&options.options)?;
        let shared = source.state::<State>();
        let mut state = shared
            .lock()
            .map_err(|_| invalid("Doctrine content cache poisoned"))?;
        if state
            .database
            .as_ref()
            .is_none_or(|(old, _)| !Arc::ptr_eq(old, &resolved))
        {
            state.database = Some((
                Arc::clone(&resolved),
                Arc::new(installed_content_core::doctrines::read(&resolved.layers)?),
            ));
        }
        let database = Arc::clone(&state.database.as_ref().expect("doctrine content").1);
        drop(state);
        let localisation = content::localisation(source, options)?;
        let frame = match source.replay_ref().seek_selected(
            params
                .branch_id
                .parse()
                .map_err(|_| invalid("Invalid doctrine branch"))?,
            params.hour,
            Some(&selected_tables()),
        )? {
            SeekResult::Frame(frame) => frame,
            SeekResult::Gap { reason, .. } => {
                return Err(invalid(format!(
                    "Doctrine state unavailable at this hour: {reason}"
                )));
            }
        };
        let default_reward = Some(
            installed_content_core::defines::read(&resolved.layers)?
                .fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY")?,
        );
        check_cancel(cancelled)?;
        let mut reply = project(&frame, params, &database, &localisation, default_reward)?;
        let catalogue = catalogue::catalogue(source, options)?;
        let politics = if reply.folders.iter().any(|folder| {
            folder
                .tracks
                .iter()
                .any(|track| !track.commanders.is_empty())
        }) {
            match super::politics_content::politics_content(source, options) {
                Ok(content) => Some(content),
                Err(why) => {
                    reply.warnings.push(format!(
                        "1 doctrine commander name catalogue unavailable: {why}"
                    ));
                    None
                }
            }
        } else {
            None
        };
        enrich(
            &mut reply,
            &frame,
            &database,
            &catalogue,
            politics.as_deref(),
            default_reward,
            cancelled,
        )?;
        super::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn fact(name: &str, value: impl ToString) -> DoctrineFact {
    DoctrineFact {
        name: name.into(),
        value: value.to_string(),
    }
}
fn fixed(raw: i64) -> String {
    let value = i128::from(raw);
    let unsigned = value.abs();
    let mut text = format!(
        "{}{}.{:05}",
        if value < 0 { "-" } else { "" },
        unsigned / 100_000,
        unsigned % 100_000
    );
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}
fn text(bytes: &[u8]) -> io::Result<String> {
    String::from_utf8(installed_content_core::parser::c_string(bytes).to_vec())
        .map_err(|_| invalid("1 doctrine text value has unsupported UTF-8 bytes"))
}

fn project(
    frame: &DecodedFrame,
    params: &DoctrineParams,
    database: &installed_content_core::doctrines::Database,
    localisation: &installed_content_localisation::localisation::Localisation,
    default_reward: Option<i64>,
) -> io::Result<DoctrineReply> {
    let country = frame
        .complete::<tables::Countries>()?
        .iter()
        .find(|row| row.tag == params.tag.as_bytes())
        .ok_or_else(|| invalid("Selected doctrine country is not recorded at this hour"))?;
    let slot = country.canonical_slot;
    let status = frame
        .complete::<tables::DoctrineCountries>()?
        .iter()
        .find(|row| row.slot == slot)
        .ok_or_else(|| invalid("Doctrine status is not recorded for this country"))?;
    if status.country != country.id
        && !frame
            .complete::<tables::CountryLinks>()?
            .iter()
            .any(|link| link.id == status.country && link.canonical_slot == slot)
    {
        return Err(invalid("Doctrine status country identity differs"));
    }
    let tokens: BTreeMap<_, _> = frame
        .complete::<tables::LexicalTokens>()?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let token = |id| {
        tokens
            .get(&id)
            .copied()
            .ok_or_else(|| invalid(format!("Doctrine token {id} is not recorded")))
    };
    let mut warnings = database.notes.clone();
    let mut name = |kind, id| -> io::Result<String> {
        let bytes = token(id)?;
        let result = if kind == 3 {
            database.folder_exact(bytes).and_then(|folder| {
                folder
                    .map(|folder| {
                        catalogue::localised_bytes(localisation, &folder.name, View::Plain)
                    })
                    .transpose()
            })
        } else {
            database.name(kind, bytes).and_then(|name| {
                name.map(|name| catalogue::localised_bytes(localisation, &name.value, View::Plain))
                    .transpose()
            })
        };
        match result {
            Ok(Some(name)) => Ok(name),
            Ok(None) => {
                warnings.push(format!("1 doctrine name unavailable: {}", text(bytes)?));
                text(bytes)
            }
            Err(reason) => {
                warnings.push(format!(
                    "1 doctrine name unavailable: {}: {reason}",
                    text(bytes)?
                ));
                text(bytes)
            }
        }
    };
    let all_tracks = frame.complete::<tables::DoctrineTracks>()?;
    let all_leaders = frame.complete::<tables::DoctrineLeaderMastery>()?;
    let mut folders = Vec::new();
    for folder in frame
        .complete::<tables::DoctrineFolders>()?
        .iter()
        .filter(|row| row.slot == slot)
    {
        let grand_source = folder
            .grand_doctrine
            .map(|id| token(id).map(installed_content_core::tokens::Token::c_str))
            .transpose()?
            .and_then(|key| database.tactic_sources.grand.get(&key));
        let mut tracks = Vec::new();
        for track in all_tracks
            .iter()
            .filter(|row| row.slot == slot && row.folder == folder.ordinal)
        {
            let label = grand_source
                .and_then(|grand| grand.tracks.get(track.ordinal as usize))
                .and_then(|key| database.tracks.get(key))
                .map(|name| catalogue::localised_bytes(localisation, &name.value, View::Plain))
                .transpose()
                .map_err(invalid)?
                .unwrap_or_else(|| format!("Track {}", track.ordinal + 1));
            let requirements = track
                .sub_doctrine
                .map(|id| token(id).map(installed_content_core::tokens::Token::c_str))
                .transpose()?
                .and_then(|key| database.tactic_sources.sub.get(&key))
                .and_then(|source| {
                    source
                        .rewards
                        .iter()
                        .map(|reward| {
                            if reward.mastery == 0 {
                                default_reward
                            } else {
                                Some(reward.mastery)
                            }
                        })
                        .collect::<Option<Vec<_>>>()
                });
            let commanders = all_leaders
                .iter()
                .filter(|row| {
                    row.slot == slot && row.folder == folder.ordinal && row.track == track.ordinal
                })
                .map(|row| {
                    fact(
                        &format!("Commander {}:{}", row.leader >> 32, row.leader as u32),
                        fixed(row.mastery),
                    )
                })
                .collect();
            tracks.push(DoctrineTrack {
                ordinal: track.ordinal,
                name: label,
                sub_doctrine: track.sub_doctrine.map(|id| name(1, id)).transpose()?,
                rewards: track.rewards,
                mastery: track.mastery.to_string(),
                mastery_bank: track.mastery_bank.to_string(),
                daily_mastery: track.effective_daily_mastery.to_string(),
                unit_daily_mastery: track.daily_mastery.to_string(),
                benefits: String::new(),
                authored_cost: String::new(),
                reward_benefits: Vec::new(),
                reward_requirements: requirements
                    .unwrap_or_default()
                    .into_iter()
                    .map(|value| value.to_string())
                    .collect(),
                commanders,
            });
        }
        folders.push(DoctrineFolder {
            ordinal: folder.ordinal,
            name: name(3, folder.folder)?,
            grand_doctrine: folder.grand_doctrine.map(|id| name(0, id)).transpose()?,
            benefits: String::new(),
            authored_cost: String::new(),
            milestones: Vec::new(),
            completed: folder
                .completed_subdoctrines
                .iter()
                .map(|&id| name(1, id))
                .collect::<io::Result<_>>()?,
            tracks,
        });
    }
    let mut inventory = Vec::new();
    for row in frame
        .complete::<tables::DoctrineCostReductions>()?
        .iter()
        .filter(|row| row.slot == slot)
    {
        inventory.push(DoctrineInventory {
            kind: "Cost reduction".into(),
            ordinal: row.ordinal,
            fields: vec![
                fact("Name", text(&row.name)?),
                fact(
                    "Folder",
                    row.folder
                        .map(|id| name(3, id))
                        .transpose()?
                        .unwrap_or_else(|| "No folder".into()),
                ),
                fact("Uses", row.uses),
                fact("Cost factor", fixed(row.cost_factor)),
            ],
        });
    }
    for row in frame
        .complete::<tables::DoctrineMasteryGrants>()?
        .iter()
        .filter(|row| row.slot == slot)
    {
        let mut fields = vec![
            fact(
                "Name",
                row.name
                    .as_deref()
                    .map(text)
                    .transpose()?
                    .unwrap_or_default(),
            ),
            fact("Daily mastery", fixed(row.daily_mastery)),
            fact("Bonus", fixed(row.bonus)),
            fact("Days", row.days_at(status.expiry_events)),
        ];
        let filter = &row.relevant_tracks;
        for (label, kind, value) in [
            ("Track", 2, filter.track),
            ("Folder", 3, filter.folder),
            ("Grand doctrine", 0, filter.grand_doctrine),
            ("Subdoctrine", 1, filter.sub_doctrine),
        ] {
            if let Some(id) = value {
                fields.push(fact(label, name(kind, id)?));
            }
        }
        if let Some(index) = filter.track_index {
            fields.push(fact("Track index", index));
        }
        inventory.push(DoctrineInventory {
            kind: "Mastery grant".into(),
            ordinal: row.ordinal,
            fields,
        });
    }
    for row in frame
        .complete::<tables::DoctrineEquipmentBonuses>()?
        .iter()
        .filter(|row| row.slot == slot)
    {
        inventory.push(DoctrineInventory {
            kind: "Equipment bonus".into(),
            ordinal: row.ordinal,
            fields: vec![
                fact("ID", row.id),
                fact("Index", row.index),
                fact("Equipment bonus", row.equipment_bonus),
            ],
        });
    }
    warnings.sort();
    warnings.dedup();
    let reply = DoctrineReply {
        branch_id: params.branch_id.clone(),
        hour: params.hour,
        tag: params.tag.clone(),
        folders,
        inventory,
        warnings,
    };
    super::check_reply_budget(&reply)?;
    Ok(reply)
}

fn enrich(
    reply: &mut DoctrineReply,
    frame: &DecodedFrame,
    database: &installed_content_core::doctrines::Database,
    catalogue: &catalogue::Catalogue,
    politics: Option<&super::politics_content::PoliticsContent>,
    default_reward: Option<i64>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    use crate::{
        effect_description::{self, Context},
        scripted_text::{EventObject, EventScope, EventScopes, ScriptedText},
    };
    let country = frame
        .complete::<tables::Countries>()?
        .iter()
        .find(|row| row.tag == reply.tag.as_bytes())
        .ok_or_else(|| invalid("Doctrine country missing"))?;
    let slot = country.canonical_slot;
    let content = catalogue.scripted_content().map_err(invalid)?;
    let scripted = ScriptedText::new(frame, &catalogue.localisation, content, slot);
    let context = Context::new(
        &scripted,
        &catalogue.localisation,
        slot,
        "doctrine benefits",
        content.effects().map_err(invalid)?.view(None),
    );
    let tokens: BTreeMap<_, _> = frame
        .complete::<tables::LexicalTokens>()?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let source = |id, grand: bool| {
        tokens.get(&id).and_then(|bytes| {
            (if grand {
                &database.tactic_sources.grand
            } else {
                &database.tactic_sources.sub
            })
            .get(&installed_content_core::tokens::Token::c_str(bytes))
        })
    };
    let mut warnings = Vec::new();
    let mut describe = |benefits: &installed_content_core::doctrines::Benefits| {
        let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
        let result = effect_description::doctrine_benefits(benefits, &context, &mut scopes);
        warnings.extend(result.refusals.into_iter().map(|(why, count)| {
            format!("{count} doctrine benefit descriptions unavailable: {why}")
        }));
        match catalogue::plain_text(&result.text) {
            Ok(text) => text,
            Err(why) => {
                warnings.push(format!("1 doctrine benefit description unavailable: {why}"));
                String::new()
            }
        }
    };
    let cost = |source: &installed_content_core::doctrines::DefinitionSources| -> io::Result<String> {
        Ok(format!(
            "{} {}",
            fixed(source.xp_cost),
            text(&source.xp_type)?
        ))
    };
    for folder in &mut reply.folders {
        check_cancel(cancelled)?;
        let row = frame
            .complete::<tables::DoctrineFolders>()?
            .iter()
            .find(|row| row.slot == slot && row.ordinal == folder.ordinal)
            .ok_or_else(|| invalid("Doctrine folder missing"))?;
        if let Some(source) = row.grand_doctrine.and_then(|id| source(id, true)) {
            folder.benefits = describe(&source.benefits);
            folder.authored_cost = cost(source)?;
            for (index, (required, benefits)) in source.milestone_benefits.iter().enumerate() {
                folder.milestones.push(DoctrineReward {
                    name: format!("Milestone {}", index + 1),
                    mastery: required.to_string(),
                    unlocked: frame
                        .complete::<tables::DoctrineTracks>()?
                        .iter()
                        .find(|track| {
                            track.slot == slot
                                && track.folder == folder.ordinal
                                && track.ordinal as usize == index
                        })
                        .and_then(|track| {
                            track
                                .sub_doctrine
                                .and_then(|id| tokens.get(&id))
                                .and_then(|token| {
                                    database
                                        .tactic_sources
                                        .sub
                                        .get(&installed_content_core::tokens::Token::c_str(token))
                                })
                                .map(|sub| i64::from(track.rewards) >= sub.rewards.len() as i64)
                        }),
                    description: describe(benefits),
                });
            }
        }
        for track in &mut folder.tracks {
            if let Some(bound) = row
                .grand_doctrine
                .and_then(|id| source(id, true))
                .and_then(|grand| grand.tracks.get(track.ordinal as usize))
                .and_then(|token| database.tactic_sources.tracks.get(token))
            {
                let label = describe(&installed_content_core::doctrines::Benefits {
                    prose: Some(bound.clone()),
                    ..Default::default()
                });
                if !label.is_empty() {
                    track.name = label.trim_end_matches('\n').to_owned();
                }
            }
            let row = frame
                .complete::<tables::DoctrineTracks>()?
                .iter()
                .find(|row| {
                    row.slot == slot && row.folder == folder.ordinal && row.ordinal == track.ordinal
                })
                .ok_or_else(|| invalid("Doctrine track missing"))?;
            if let Some(politics) = politics {
                let contributions = frame
                    .complete::<tables::DoctrineLeaderMastery>()?
                    .iter()
                    .filter(|item| {
                        item.slot == slot
                            && item.folder == folder.ordinal
                            && item.track == track.ordinal
                    });
                for (display, contribution) in track.commanders.iter_mut().zip(contributions) {
                    let named = frame
                        .complete::<tables::UnitLeaders>()
                        .ok()
                        .and_then(|leaders| {
                            leaders.iter().find(|leader| {
                                leader.id == (contribution.leader >> 32) as u32
                                    && leader.reference_type == contribution.leader as u32
                            })
                        })
                        .and_then(|leader| {
                            frame
                                .complete::<tables::Characters>()
                                .ok()
                                .and_then(|characters| {
                                    characters
                                        .iter()
                                        .find(|character| character.id == leader.character_id)
                                })
                        })
                        .and_then(|character| {
                            std::str::from_utf8(&character.token)
                                .ok()
                                .and_then(|saved| {
                                    politics.character_name(
                                        crate::identity::character_script_token(
                                            character,
                                            frame.complete::<tables::LexicalTokens>().ok()?,
                                        ),
                                        saved,
                                    )
                                })
                        });
                    if let Some(name) = named {
                        display.name = name;
                    }
                }
            }
            if let Some(source) = row.sub_doctrine.and_then(|id| source(id, false)) {
                track.benefits = describe(&source.benefits);
                track.authored_cost = cost(source)?;
                for (index, reward) in source.rewards.iter().enumerate() {
                    let sub = tokens[&row.sub_doctrine.expect("selected subdoctrine")];
                    let key = reward_key(sub, &reward.token);
                    let label =
                        catalogue::localised_bytes(&catalogue.localisation, &key, View::Plain)
                            .map_err(invalid)?;
                    track.reward_benefits.push(DoctrineReward {
                        name: label,
                        mastery: (if reward.mastery == 0 {
                            default_reward.unwrap_or(0)
                        } else {
                            reward.mastery
                        })
                        .to_string(),
                        unlocked: Some((index as i64) < i64::from(track.rewards)),
                        description: describe(&reward_benefits(
                            reward,
                            &key,
                            &catalogue.localisation,
                        )),
                    });
                }
            }
        }
        for id in &row.completed_subdoctrines {
            if let Some(source) = source(*id, false) {
                let fields = source
                    .rewards
                    .iter()
                    .map(|reward| {
                        let key = reward_key(tokens[id], &reward.token);
                        Ok(fact(
                            &catalogue::localised_bytes(&catalogue.localisation, &key, View::Plain)
                                .map_err(invalid)?,
                            describe(&reward_benefits(reward, &key, &catalogue.localisation)),
                        ))
                    })
                    .collect::<io::Result<Vec<_>>>()?;
                reply.inventory.push(DoctrineInventory {
                    kind: "Completed subdoctrine rewards".into(),
                    ordinal: *id,
                    fields,
                });
            }
        }
    }
    reply.warnings.extend(warnings);
    if !reply.folders.is_empty() {
        reply.warnings.push("1 doctrine display unavailable: effective purchase costs require the selected country's cost-modifier operands; authored base costs and stored reduction grants are shown".into());
    }
    reply.warnings.sort();
    reply.warnings.dedup();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::{Metrics, TableView};

    #[test]
    fn doctrine_reward_content_names_generated_prose_and_recorded_conditions() {
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}, NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/doctrines/tracks/test.txt",
                "mod_track={name=MOD_TRACK_LABEL}",
            ),
            (
                "common/doctrines/subdoctrines/test.txt",
                "mod_sub={rewards={tier1={mastery=1 effect={mod_reward_effect=yes}} tier2={mastery=2}}}",
            ),
            (
                "common/scripted_effects/test.txt",
                "mod_reward_effect={if={limit={has_country_flag=mod_flag} add_political_power=mod_amount}}",
            ),
            (
                "localisation/test_l_english.yml",
                concat!(
                    "\u{feff}l_english:\n",
                    " MOD_TRACK_LABEL:0 \"Mod track selector\"\n",
                    " tier1:0 \"Wrong bare reward\"\n",
                    " mod_sub_tier1:0 \"Mod reward one\"\n",
                    " mod_sub_tier1_desc:0 \"Generated reward prose\"\n",
                    " mod_sub_tier2:0 \"Mod reward two\"\n",
                    " POLITICS_ADD_POLITICAL_POWER:0 \"PP$VAL|0$\"\n",
                ),
            ),
        ]);
        let catalogue = Arc::new(catalogue);
        let database = installed_content_core::doctrines::read(&catalogue.content.layers).unwrap();
        let politics =
            super::super::politics_content::PoliticsContent::load(Arc::clone(&catalogue)).unwrap();
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        let selected = selected_tables();
        // Exercise exactly the loader's selection, rather than a complete frame
        // that would hide missing authored-description dependencies.
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                let id = TableId::ALL[index];
                if selected.contains(&id) {
                    TableView::Complete(TableRows::empty(id)).into()
                } else {
                    TableView::Unavailable {
                        reason: "Table not selected".into(),
                    }
                    .into()
                }
            }),
        };
        for rows in [
            TableRows::Countries(vec![CountryRow {
                id: 91,
                canonical_slot: 5_000,
                tag: b"MOD".to_vec(),
                ..Default::default()
            }]),
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 91,
                canonical_slot: 5_000,
            }]),
            TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 1,
                    literal: b"mod_sub".to_vec(),
                },
                LexicalTokenRow {
                    token: 2,
                    literal: b"mod_folder".to_vec(),
                },
                LexicalTokenRow {
                    token: 3,
                    literal: b"mod_flag".to_vec(),
                },
                LexicalTokenRow {
                    token: 4,
                    literal: b"mod_track".to_vec(),
                },
            ]),
            TableRows::DoctrineCountries(vec![DoctrineCountryRow {
                slot: 5_000,
                country: 91,
                ..Default::default()
            }]),
            TableRows::DoctrineFolders(vec![DoctrineFolderRow {
                slot: 5_000,
                folder: 2,
                completed_subdoctrines: vec![1],
                ..Default::default()
            }]),
            TableRows::DoctrineTracks(vec![DoctrineTrackRow {
                slot: 5_000,
                sub_doctrine: Some(1),
                rewards: 1,
                ..Default::default()
            }]),
            TableRows::DoctrineLeaderMastery(vec![
                DoctrineLeaderMasteryRow {
                    slot: 5_000,
                    leader: (91_u64 << 32) | 17,
                    mastery: 1,
                    ..Default::default()
                },
                DoctrineLeaderMasteryRow {
                    slot: 5_000,
                    ordinal: 1,
                    leader: (93_u64 << 32) | 19,
                    mastery: 2,
                    ..Default::default()
                },
            ]),
            TableRows::UnitLeaders(vec![
                UnitLeaderRow {
                    id: 17,
                    reference_type: 91,
                    character_id: 201,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 91,
                    reference_type: 18,
                    character_id: 202,
                    ..Default::default()
                },
                UnitLeaderRow {
                    id: 91,
                    reference_type: 17,
                    character_id: 200,
                    ..Default::default()
                },
            ]),
            TableRows::Characters(
                [
                    (200, b"Correct commander".as_slice()),
                    (201, b"Swapped commander".as_slice()),
                    (202, b"Other reference type".as_slice()),
                ]
                .into_iter()
                .map(|(id, token)| CharacterRow {
                    id,
                    token: token.to_vec(),
                    ..Default::default()
                })
                .collect(),
            ),
            TableRows::DoctrineMasteryGrants(vec![DoctrineMasteryGrantRow {
                slot: 5_000,
                daily_mastery: 1,
                days_at_anchor: 5,
                relevant_tracks: DoctrineTrackFilter {
                    track: Some(4),
                    track_index: Some(0),
                    ..Default::default()
                },
                ..Default::default()
            }]),
            TableRows::ScriptFlags(vec![ScriptFlagRow {
                scope_kind: 1,
                scope_id: 91,
                token: 3,
                value: 1,
                ..Default::default()
            }]),
            TableRows::ScriptVariables(vec![ScriptVariablesRow {
                scope_kind: 1,
                scope_id: 91,
                seed: Recorded::Value(None),
                entries: vec![ScriptVariableEntry {
                    name: b"mod_amount".to_vec(),
                    value_raw: 200_000,
                }],
            }]),
        ] {
            let id = rows.table();
            assert!(selected.contains(&id), "authored descriptions need {id:?}");
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        let params = DoctrineParams {
            branch_id: "1".into(),
            tag: "MOD".into(),
            hour: 7,
        };
        let render = |frame: &DecodedFrame| {
            let mut reply = project(
                frame,
                &params,
                &database,
                &catalogue.localisation,
                Some(100_000),
            )
            .unwrap();
            enrich(
                &mut reply,
                frame,
                &database,
                &catalogue,
                Some(&politics),
                Some(100_000),
                &AtomicBool::new(false),
            )
            .unwrap();
            reply
        };
        let reply = render(&frame);
        let commanders = &reply.folders[0].tracks[0].commanders;
        assert_eq!(commanders[0].name, "Correct commander");
        assert_eq!(commanders[0].value, "0.00001");
        assert_eq!(commanders[1].name, "Commander 93:19");
        assert_eq!(commanders[1].value, "0.00002");
        let grant = reply
            .inventory
            .iter()
            .find(|item| item.kind == "Mastery grant")
            .unwrap();
        assert_eq!(
            grant
                .fields
                .iter()
                .find(|field| field.name == "Track")
                .unwrap()
                .value,
            "Mod track selector"
        );
        assert_eq!(
            grant
                .fields
                .iter()
                .find(|field| field.name == "Track index")
                .unwrap()
                .value,
            "0"
        );
        let rewards = &reply.folders[0].tracks[0].reward_benefits;
        assert_eq!(rewards[0].name, "Mod reward one");
        assert_eq!(rewards[1].name, "Mod reward two");
        assert!(
            rewards[0].description.contains("Generated reward prose"),
            "{}",
            rewards[0].description
        );
        assert!(
            rewards[0].description.contains("PP2"),
            "{:?}",
            reply.warnings
        );
        assert!(rewards[1].description.is_empty()); // Optional generated prose is absent.
        let completed = reply
            .inventory
            .iter()
            .find(|item| item.kind == "Completed subdoctrine rewards")
            .unwrap();
        assert_eq!(completed.fields[0].name, "Mod reward one");
        assert_eq!(completed.fields[0].value, rewards[0].description);
        assert!(
            !reply
                .warnings
                .iter()
                .any(|why| why.contains("benefit descriptions unavailable")),
            "{:?}",
            reply.warnings
        );
        let flags = frame.tables[TableId::ScriptFlags.index()].clone();
        *frame.tables[TableId::ScriptFlags.index()] = TableView::Unavailable {
            reason: "synthetic flag gap".into(),
        };
        let refused = render(&frame);
        assert!(
            refused
                .warnings
                .iter()
                .any(|why| why.contains("synthetic flag gap")),
            "{:?}",
            refused.warnings
        );
        assert!(
            refused.folders[0].tracks[0].reward_benefits[0]
                .description
                .contains("Generated reward prose")
        );
        frame.tables[TableId::ScriptFlags.index()] = flags;
        *frame.tables[TableId::ScriptVariables.index()] = TableView::Unavailable {
            reason: "synthetic variable gap".into(),
        };
        let refused = render(&frame);
        assert!(
            refused
                .warnings
                .iter()
                .any(|why| why.contains("synthetic variable gap")),
            "{:?}",
            refused.warnings
        );
    }

    #[test]
    fn doctrine_description_selection_uses_shared_scoped_inputs() {
        let selected = selected_tables();
        assert!(
            crate::scripted_text::TABLES
                .iter()
                .all(|table| selected.contains(table))
        );
        assert!(selected.contains(&TableId::Clock));
        assert_eq!(
            selected.iter().copied().collect::<BTreeSet<_>>().len(),
            selected.len()
        );
    }

    #[test]
    fn doctrine_product_joins_canonical_slot_and_keeps_effective_gain_distinct() {
        let country = CountryRow {
            id: 91,
            canonical_slot: 5_000,
            tag: b"MOD".to_vec(),
            border_land_doctrine_level: Recorded::Value(None),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
            manpower_residual: 0,
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
            }),
        };
        for rows in [
            TableRows::Countries(vec![country]),
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"mod_folder".to_vec(),
            }]),
            TableRows::DoctrineCountries(vec![DoctrineCountryRow {
                slot: 5_000,
                country: 91,
                ..Default::default()
            }]),
            TableRows::DoctrineFolders(vec![DoctrineFolderRow {
                slot: 5_000,
                folder: 1,
                ..Default::default()
            }]),
            TableRows::DoctrineTracks(vec![DoctrineTrackRow {
                slot: 5_000,
                mastery: i64::MAX,
                mastery_bank: -1,
                daily_mastery: 12,
                effective_daily_mastery: 34,
                ..Default::default()
            }]),
            TableRows::DoctrineLeaderMastery(vec![DoctrineLeaderMasteryRow {
                slot: 5_000,
                leader: 17 | (91_u64 << 32),
                mastery: -1,
                ..Default::default()
            }]),
        ] {
            let id = rows.table();
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        let params = DoctrineParams {
            branch_id: "1".into(),
            tag: "MOD".into(),
            hour: 7,
        };
        let reply = project(
            &frame,
            &params,
            &Default::default(),
            &Default::default(),
            Some(100_000),
        )
        .unwrap();
        let track = &reply.folders[0].tracks[0];
        assert_eq!(track.mastery, i64::MAX.to_string());
        assert_eq!(track.daily_mastery, "34");
        assert_eq!(track.unit_daily_mastery, "12");
        assert_eq!(track.commanders[0].name, "Commander 91:17");
        assert_eq!(track.commanders[0].value, "-0.00001");
        assert_eq!(reply.folders[0].grand_doctrine, None);
        assert!(!reply.warnings.is_empty());
        *frame.tables[TableId::DoctrineCountries.index()] =
            TableView::Complete(TableRows::DoctrineCountries(vec![DoctrineCountryRow {
                slot: 5_000,
                country: 92,
                ..Default::default()
            }]));
        assert!(
            project(
                &frame,
                &params,
                &Default::default(),
                &Default::default(),
                None
            )
            .is_err()
        );
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                id: 92,
                canonical_slot: 5_000,
            }]));
        assert!(
            project(
                &frame,
                &params,
                &Default::default(),
                &Default::default(),
                None
            )
            .is_ok()
        );
    }
}
