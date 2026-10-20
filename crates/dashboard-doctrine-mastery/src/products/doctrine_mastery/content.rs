use super::{
    DoctrineMasteryFolder, DoctrineMasteryGrand, DoctrineMasteryReply, DoctrineMasteryReward,
    DoctrineMasterySubdoctrine, DoctrineMasteryTrack,
    observer::{DoctrineMasteryObserver, FolderInventory},
};
use dashboard_base::{
    Source,
    catalogue::{self, CachedResult, Catalogue, KeyedResults, View},
};
use dashboard_keyed::{Observer, query::Query};
use installed_content_core::{
    doctrines::{self, Benefits, Database},
    tokens::Token,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

#[derive(Default)]
struct ContentCache {
    database: CachedResult<Database>,
    default_reward: CachedResult<i64>,
    subdoctrines: KeyedResults<Vec<u8>, Named<DoctrineMasterySubdoctrine>>,
    tracks: KeyedResults<Vec<u8>, Named<Option<String>>>,
}

type BoundCache = Option<(catalogue::Binding, Arc<ContentCache>)>;

fn cache(source: &Source, options: &dashboard_core::config::ContentOptions) -> Arc<ContentCache> {
    let state = source.state::<BoundCache>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    if !state
        .as_ref()
        .is_some_and(|(binding, _)| catalogue::binding_matches(binding, options))
    {
        *state = Some((
            catalogue::binding(options),
            Arc::new(ContentCache::default()),
        ));
    }
    Arc::clone(&state.as_ref().expect("doctrine content binding").1)
}

struct Named<T> {
    value: T,
    warnings: Vec<String>,
}

#[derive(Default)]
struct Notes(BTreeMap<String, u64>);

impl Notes {
    fn add(&mut self, why: impl Into<String>) {
        *self.0.entry(why.into()).or_default() += 1;
    }

    fn append(&mut self, warnings: impl IntoIterator<Item = String>) {
        for warning in warnings {
            self.add(warning);
        }
    }

    fn finish(self) -> Vec<String> {
        self.0
            .into_iter()
            .map(|(why, count)| format!("{count} doctrine mastery content refusals: {why}"))
            .collect()
    }
}

fn token_text(token: &Token) -> io::Result<String> {
    match token {
        Token::Name(bytes) => String::from_utf8(bytes.clone()).map_err(io::Error::other),
        Token::Rejected => Err(io::Error::other("rejected authored doctrine token")),
    }
}

fn label(catalogue: &Catalogue, bytes: &[u8], what: &str, notes: &mut Notes) -> Option<String> {
    if let Ok(key) = std::str::from_utf8(installed_content_core::parser::c_string(bytes))
        && (key.is_empty() || catalogue.localisation.get(key).is_none())
    {
        notes.add(format!("{what} localisation {key} is absent"));
        return None;
    }
    match catalogue::localised_bytes(&catalogue.localisation, bytes, View::Plain) {
        Ok(name) => Some(name),
        Err(why) => {
            notes.add(format!("{what} name: {why}"));
            None
        }
    }
}

type Reached<'a> = Option<(&'a replay_frame::frame::DecodedFrame, u32)>;

/// Use the reached Doctrines formatter. A history request has no mutable scope;
/// a selected-hour request binds exactly one frame through the shared frame cache.
fn describe(
    catalogue: &Catalogue,
    benefits: &Benefits,
    reached: Reached<'_>,
    notes: &mut Notes,
) -> String {
    use dashboard_text::{
        effect_description::{self, Context},
        scripted_text::{EventObject, EventScope, EventScopes, ScriptedText},
    };
    let content = match catalogue.scripted_content() {
        Ok(content) => content,
        Err(why) => {
            notes.add(format!("description catalogue: {why}"));
            return String::new();
        }
    };
    let effects = match content.effects() {
        Ok(effects) => effects,
        Err(why) => {
            notes.add(format!("description effects: {why}"));
            return String::new();
        }
    };
    let unavailable;
    let (frame, slot) = match reached {
        Some(reached) => reached,
        None => {
            unavailable = replay_frame::frame::DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| {
                    replay_frame::frame::TableView::Unavailable {
                        reason: "Doctrine mastery content has no reached-hour scope".into(),
                    }
                    .into()
                }),
                metrics: Default::default(),
            };
            (&unavailable, 0)
        }
    };
    let text = ScriptedText::new(frame, &catalogue.localisation, content, slot);
    let context = Context::new(
        &text,
        &catalogue.localisation,
        slot,
        "doctrine mastery content",
        effects.view(None),
    );
    let mut scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(slot))]);
    let description = effect_description::doctrine_benefits(benefits, &context, &mut scopes);
    for (why, count) in description.refusals {
        *notes
            .0
            .entry(format!("description requires reached-hour operands: {why}"))
            .or_default() += count as u64;
    }
    match catalogue::plain_text(&description.text) {
        Ok(text) => text,
        Err(why) => {
            notes.add(format!("description text: {why}"));
            String::new()
        }
    }
}

fn subdoctrine(
    database: &Database,
    catalogue: Option<&Catalogue>,
    token: &[u8],
    default_reward: Result<i64, &str>,
    reached: Reached<'_>,
) -> io::Result<Named<DoctrineMasterySubdoctrine>> {
    let mut notes = Notes::default();
    let id = String::from_utf8(installed_content_core::parser::c_string(token).to_vec())
        .map_err(io::Error::other)?;
    let presentation = match database.presentation(1, token) {
        Ok(Some(presentation)) => Some(presentation),
        result => {
            notes.add(format!(
                "subdoctrine {id} presentation: {}",
                result
                    .err()
                    .unwrap_or_else(|| "definition is absent".into())
            ));
            None
        }
    };
    // Reward localisation uses the definition's spelling, independently of
    // the witnessed spelling that joins the numeric token dictionary.
    let name = match database.name(1, token) {
        Ok(Some(name)) => {
            catalogue.and_then(|catalogue| label(catalogue, &name.value, &id, &mut notes))
        }
        result => {
            notes.add(format!(
                "subdoctrine {id} name: {}",
                result
                    .err()
                    .unwrap_or_else(|| "definition is absent".into())
            ));
            None
        }
    };
    let rewards = if let Some(source) = database.tactic_sources.sub.get(&Token::c_str(token)) {
        let mut rewards = Vec::new();
        for reward in &source.rewards {
            let threshold = if reward.mastery == 0 {
                match default_reward {
                    Ok(value) => Some(value),
                    Err(why) => {
                        notes.add(format!(
                            "subdoctrine {id} reward {} threshold unavailable: {why}",
                            String::from_utf8_lossy(&reward.token)
                        ));
                        None
                    }
                }
            } else {
                Some(reward.mastery)
            };
            if let Some(threshold) = threshold.filter(|&value| value <= 0) {
                notes.add(format!(
                    "reward {} has nonpositive mastery scale {threshold}",
                    String::from_utf8_lossy(&reward.token)
                ));
            }
            let mut benefits = reward.benefits.clone();
            let mut name = None;
            if let (Some(catalogue), Some(presentation)) = (catalogue, presentation) {
                let mut composite = installed_content_core::parser::c_string(&presentation.token).to_vec();
                composite.push(b'_');
                composite.extend_from_slice(installed_content_core::parser::c_string(&reward.token));
                name = label(catalogue, &composite, "reward", &mut notes);
                let mut description_key = composite;
                description_key.extend_from_slice(b"_desc");
                if benefits.prose.is_none()
                    && std::str::from_utf8(&description_key)
                        .ok()
                        .and_then(|key| catalogue.localisation.get(key))
                        .is_some()
                {
                    benefits.prose =
                        Some(installed_content_core::gui::native::BoundLocalization::from_value(
                            &installed_content_core::parser::Value::Scalar(description_key),
                        ));
                }
            }
            rewards.push(DoctrineMasteryReward {
                id: String::from_utf8(installed_content_core::parser::c_string(&reward.token).to_vec())
                    .map_err(io::Error::other)?,
                name,
                description: catalogue
                    .map(|catalogue| describe(catalogue, &benefits, reached, &mut notes)),
                threshold: threshold.map(|value| value.to_string()),
            });
        }
        Some(rewards)
    } else {
        notes.add(format!("subdoctrine {id} rewards definition is absent"));
        None
    };
    let icon = presentation.and_then(|presentation| {
        catalogue.and_then(|catalogue| {
            icon(
                catalogue,
                &presentation.icon,
                &format!("subdoctrine {id}"),
                &mut notes,
            )
        })
    });
    Ok(Named {
        value: DoctrineMasterySubdoctrine {
            id,
            token_id: None,
            name,
            icon,
            rewards,
        },
        warnings: notes.finish(),
    })
}

fn icon(catalogue: &Catalogue, authored: &[u8], what: &str, notes: &mut Notes) -> Option<String> {
    match dashboard_base::products::artwork::sprites::icon_key(catalogue, authored) {
        Ok(key) => Some(key),
        Err(why) => {
            notes.add(format!("{what} icon: {why}"));
            None
        }
    }
}

fn track_label(
    database: &Database,
    catalogue: &Catalogue,
    token: &Token,
    reached: Reached<'_>,
) -> Named<Option<String>> {
    let mut notes = Notes::default();
    let benefits = Benefits {
        prose: database.tactic_sources.tracks.get(token).cloned(),
        ..Default::default()
    };
    if benefits.prose.is_none() {
        notes.add(format!("track {token:?} name is absent"));
    }
    Named {
        value: {
            let text = describe(catalogue, &benefits, reached, &mut notes);
            (!text.is_empty()).then(|| text.trim_end_matches('\n').to_owned())
        },
        warnings: notes.finish(),
    }
}

/// Selected descriptions never read beyond the accepted prefix, choose a nearby
/// hour, or retain mutable text in the content-binding-only caches.
fn reached(
    query: &Query<'_>,
    notes: &mut Notes,
) -> io::Result<Option<(Box<replay_frame::frame::DecodedFrame>, u32)>> {
    use dashboard_base::metadata::{self, TaggedCountry};
    use replay_data::rows::{TableId, tables};
    let Some(hour) = query
        .params
        .get("hour")
        .and_then(serde_json::Value::as_i64)
        .and_then(|hour| i32::try_from(hour).ok())
    else {
        return Ok(None);
    };
    let (first, last) = query.window()?;
    if hour < first || hour > last || query.through().is_some_and(|through| hour > through) {
        notes.add(format!(
            "description hour {hour} is outside the accepted window"
        ));
        return Ok(None);
    }
    let mut tables = BTreeSet::from_iter(dashboard_text::scripted_text::TABLES.iter().copied());
    tables.extend([
        TableId::Clock,
        TableId::DoctrineLeaderMastery,
        TableId::DoctrineCostReductions,
        TableId::DoctrineMasteryGrants,
        TableId::DoctrineEquipmentBonuses,
    ]);
    // Shared effect/trigger readers reach doctrine state as well as text state.
    tables.extend(super::DoctrineMasteryObserver::TABLES.iter().copied());
    let tables = tables.into_iter().collect::<Vec<_>>();
    let frame = match dashboard_base::prepared::query::progressive::frame_at(
        query.source,
        query.branch,
        hour,
        &tables,
        query.cancelled,
    ) {
        Ok(Some(frame)) => frame,
        Ok(None) => {
            notes.add(format!("description hour {hour} is not recorded"));
            return Ok(None);
        }
        Err(why)
            if matches!(
                why.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) =>
        {
            return Err(why);
        }
        Err(why) => {
            notes.add(format!("description hour {hour}: {why}"));
            return Ok(None);
        }
    };
    let tag = query
        .params
        .get("tag")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let slot = match (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) {
        (Ok(countries), Ok(links)) => match metadata::tagged_country(countries, links, tag) {
            TaggedCountry::Found(_, slot) => Some(slot),
            _ => None,
        },
        _ => None,
    };
    Ok(match slot {
        Some(slot) => Some((frame, slot)),
        None => {
            notes.add(format!(
                "description country {tag} is unavailable at hour {hour}"
            ));
            None
        }
    })
}

/// Recorded identities are reachable even when no content reader can run.
fn recorded(
    query: &Query<'_>,
    observer: &DoctrineMasteryObserver,
    reply: &mut DoctrineMasteryReply,
    notes: &mut Notes,
) -> io::Result<BTreeMap<u32, FolderInventory>> {
    let mut folders = BTreeMap::<u32, FolderInventory>::new();
    let mut ids = BTreeSet::new();
    for series in &reply.window.series {
        dashboard_text::metadata::check_cancel(query.cancelled)?;
        let parts: Vec<_> = series.name.split('|').collect();
        let Some(ordinal) = parts.get(1).and_then(|part| part.parse::<u32>().ok()) else {
            continue;
        };
        let folder = folders.entry(ordinal).or_default();
        let values = || {
            series
                .runs
                .iter()
                .filter_map(|(_, _, raw, _)| raw.parse::<u64>().ok())
                .filter(|&id| id != 0)
        };
        if parts.get(2) == Some(&"grand") {
            folder.grands.extend(values());
            ids.extend(values());
        } else if parts.get(2) == Some(&"completed") {
            ids.extend(values());
        } else if let Some(track) = parts.get(2).and_then(|part| part.parse::<u32>().ok()) {
            folder.tracks.insert(track);
            if parts.get(3) == Some(&"sub") {
                folder.subs.entry(track).or_default().extend(values());
                ids.extend(values());
            }
        }
    }
    for (&ordinal, inventory) in &mut folders {
        if let Some(observed) = observer.folders.get(&ordinal) {
            inventory.tokens.clone_from(&observed.tokens);
            ids.extend(&inventory.tokens);
        }
    }
    for id in ids {
        match observer
            .literal(id)
            .map(|bytes| String::from_utf8(bytes.to_vec()))
        {
            Some(Ok(token)) => {
                reply.tokens.insert(id.to_string(), token);
            }
            Some(Err(_)) => notes.add(format!("interned token {id} is not UTF-8")),
            None => notes.add(format!("interned token {id} spelling is unavailable")),
        }
    }
    for (&ordinal, inventory) in &folders {
        let identity = |id: u64| {
            reply
                .tokens
                .get(&id.to_string())
                .cloned()
                .unwrap_or_else(|| id.to_string())
        };
        reply.folders.push(DoctrineMasteryFolder {
            id: ordinal.to_string(),
            name: None,
            icon: None,
            grand: inventory
                .grands
                .iter()
                .map(|&id| DoctrineMasteryGrand {
                    id: identity(id),
                    token_id: Some(id.to_string()),
                    name: None,
                    icon: None,
                })
                .collect(),
            tracks: inventory
                .tracks
                .iter()
                .map(|track| DoctrineMasteryTrack {
                    id: track.to_string(),
                    name: None,
                    subdoctrines: inventory
                        .subs
                        .get(track)
                        .into_iter()
                        .flatten()
                        .map(|&id| DoctrineMasterySubdoctrine {
                            id: identity(id),
                            token_id: Some(id.to_string()),
                            name: None,
                            icon: None,
                            rewards: None,
                        })
                        .collect(),
                })
                .collect(),
        });
    }
    Ok(folders)
}

pub(super) fn present(
    query: &Query<'_>,
    observer: &DoctrineMasteryObserver,
    reply: &mut DoctrineMasteryReply,
) -> io::Result<()> {
    let mut notes = Notes::default();
    let recorded = recorded(query, observer, reply, &mut notes)?;
    if recorded.is_empty() {
        reply.warnings.extend(notes.finish());
        return Ok(());
    }
    let Some(options) = query.content else {
        notes.add("game folders are not chosen; names, rewards and icons are unavailable");
        reply.warnings.extend(notes.finish());
        return Ok(());
    };
    let cache = cache(query.source, options);
    let database = match cache.database.get_or_init(|| {
        let mut verification = Vec::new();
        dashboard_base::content::check_replay(
            query.source.replay_ref(),
            options.allow_unverified,
            &mut verification,
        )?;
        doctrines::read(&query.source.content(&options.options)?.layers)
    }) {
        Ok(database) => database,
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            notes.add(format!("doctrine database unavailable: {error}"));
            reply.warnings.extend(notes.finish());
            return Ok(());
        }
    };
    let catalogue = match catalogue::catalogue(query.source, options) {
        Ok(catalogue) => Some(catalogue),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => {
            notes.add(format!("localisation catalogue unavailable: {error}"));
            None
        }
    };
    notes.append(database.notes.clone());
    if let Some(catalogue) = &catalogue {
        notes.append(catalogue.warnings.clone());
    }
    // Match the first witnessed wire spelling without rescanning the complete
    // branch token inventory for each authored subdoctrine alternative.
    let mut spellings = BTreeMap::new();
    for bytes in observer
        .tokens
        .iter()
        .filter_map(|entry| entry.literal.as_deref())
    {
        spellings.entry(Token::c_str(bytes)).or_insert(bytes);
    }
    let reached_frame = if catalogue.is_some() {
        reached(query, &mut notes)?
    } else {
        None
    };
    let reached = reached_frame
        .as_ref()
        .map(|(frame, slot)| (frame.as_ref(), *slot));
    let default_reward = cache.default_reward.get_or_init(|| {
        installed_content_core::defines::read(&query.source.content(&options.options)?.layers)?
            .fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY")
    });
    let default_error = default_reward.as_ref().err().map(ToString::to_string);
    let default_value = default_reward
        .as_ref()
        .map(|value| **value)
        .map_err(|_| default_error.as_deref().expect("default reward error"));
    let mut recorded_folders: BTreeMap<_, _> = std::mem::take(&mut reply.folders)
        .into_iter()
        .map(|folder| (folder.id.clone(), folder))
        .collect();
    for (ordinal, inventory) in recorded {
        dashboard_text::metadata::check_cancel(query.cancelled)?;
        let numeric = recorded_folders
            .remove(&ordinal.to_string())
            .expect("recorded folder");
        let mut recorded_tracks: BTreeMap<_, _> = numeric
            .tracks
            .into_iter()
            .map(|track| (track.id.clone(), track))
            .collect();
        let folder_tokens: BTreeSet<_> = inventory
            .tokens
            .iter()
            .filter_map(|&id| observer.literal(id))
            .collect();
        let mut name = None;
        let mut folder_icon = None;
        let mut authored_grands = BTreeSet::new();
        if folder_tokens.len() == 1
            && inventory
                .tokens
                .iter()
                .all(|&id| observer.literal(id).is_some())
        {
            let token = folder_tokens.first().expect("one folder token");
            match database.folder_presentation(token) {
                Ok(Some(presentation)) => {
                    folder_icon = catalogue.as_deref().and_then(|catalogue| {
                        icon(
                            catalogue,
                            &presentation.icon,
                            &format!("folder {ordinal}"),
                            &mut notes,
                        )
                    })
                }
                result => notes.add(format!(
                    "folder {ordinal} presentation: {}",
                    result
                        .err()
                        .unwrap_or_else(|| "definition is absent".into())
                )),
            }
            match database.grand_in_folder(token) {
                Ok(choices) => authored_grands.extend(
                    choices
                        .into_iter()
                        .map(|choice| Token::c_str(&choice.token)),
                ),
                Err(why) => notes.add(format!("folder {ordinal} grand choices: {why}")),
            }

            match database.folder_exact(token) {
                Ok(Some(folder)) => {
                    name = catalogue
                        .as_deref()
                        .and_then(|catalogue| label(catalogue, &folder.name, "folder", &mut notes))
                }
                result => notes.add(format!(
                    "folder {ordinal} name: {}",
                    result
                        .err()
                        .unwrap_or_else(|| "definition is absent".into())
                )),
            }
        } else {
            notes.add(format!(
                "folder {ordinal} token is unavailable or ambiguous"
            ));
        }
        let grand_tokens: BTreeSet<_> = inventory
            .grands
            .iter()
            .filter_map(|&id| observer.literal(id))
            .map(Token::c_str)
            .collect();
        let mut grand = Vec::new();
        let mut grand_sources = Vec::new();
        let mut seen = BTreeSet::new();
        for token in &database.tactic_sources.grand_order {
            if (!authored_grands.contains(token) && !grand_tokens.contains(token))
                || !seen.insert(token.clone())
            {
                continue;
            }
            if (inventory.grands.is_empty() || grand_tokens.contains(token))
                && let Some(source) = database.tactic_sources.grand.get(token)
            {
                grand_sources.push(source);
            }
            let presentation = match database.presentation(
                0,
                match token {
                    Token::Name(bytes) => bytes,
                    Token::Rejected => b"",
                },
            ) {
                Ok(Some(presentation)) => presentation,
                result => {
                    notes.add(format!(
                        "grand doctrine presentation: {}",
                        result
                            .err()
                            .unwrap_or_else(|| "definition is absent".into())
                    ));
                    continue;
                }
            };
            let spelling = inventory
                .grands
                .iter()
                .filter_map(|&id| observer.literal(id))
                .find(|bytes| Token::c_str(bytes) == *token)
                .unwrap_or(&presentation.token);
            let id = match String::from_utf8(spelling.to_vec()) {
                Ok(id) => id,
                Err(why) => {
                    notes.add(why.to_string());
                    continue;
                }
            };
            let name = match database.name(0, id.as_bytes()) {
                Ok(Some(name)) => catalogue
                    .as_deref()
                    .and_then(|catalogue| label(catalogue, &name.value, &id, &mut notes)),
                result => {
                    notes.add(format!(
                        "grand doctrine {id} name: {}",
                        result
                            .err()
                            .unwrap_or_else(|| "definition is absent".into())
                    ));
                    None
                }
            };
            let icon = catalogue.as_deref().and_then(|catalogue| {
                icon(
                    catalogue,
                    &presentation.icon,
                    &format!("grand doctrine {id}"),
                    &mut notes,
                )
            });
            grand.push(DoctrineMasteryGrand {
                id,
                token_id: None,
                name,
                icon,
            });
        }
        for choice in numeric.grand {
            if let Some(projected) = grand.iter_mut().find(|projected| {
                choice
                    .token_id
                    .as_ref()
                    .is_some_and(|id| reply.tokens.contains_key(id))
                    && projected.id == choice.id
            }) {
                projected.token_id = choice.token_id;
            } else {
                grand.push(choice);
            }
        }
        if !grand_tokens.is_subset(&seen) {
            notes.add(format!(
                "folder {ordinal} has grand doctrine selections absent from content"
            ));
        }
        let mut tracks = Vec::new();
        let mut track_indices = inventory.tracks.clone();
        for source in &grand_sources {
            track_indices.extend(
                source
                    .tracks
                    .iter()
                    .enumerate()
                    .filter_map(|(at, _)| u32::try_from(at).ok()),
            );
        }
        for track in track_indices {
            dashboard_text::metadata::check_cancel(query.cancelled)?;
            let track_tokens: BTreeSet<_> = grand_sources
                .iter()
                .filter_map(|source| {
                    usize::try_from(track)
                        .ok()
                        .and_then(|at| source.tracks.get(at))
                })
                .cloned()
                .collect();
            let mut track_name = None;
            if let Some(catalogue) = catalogue.as_deref() {
                if track_tokens.len() == 1 {
                    let token = track_tokens.first().expect("one track token");
                    if let Ok(id) = token_text(token) {
                        let key = id.as_bytes().to_vec();
                        let named = if reached.is_some() {
                            Ok(Arc::new(track_label(&database, catalogue, token, reached)))
                        } else {
                            cache.tracks.get_or_init(&key, |_| {
                                Ok(track_label(&database, catalogue, token, None))
                            })
                        };
                        match named {
                            Ok(named) => {
                                track_name.clone_from(&named.value);
                                reply.warnings.extend(named.warnings.clone());
                            }
                            Err(why) => notes.add(format!("track {id} name: {why}")),
                        }
                    }
                } else {
                    notes.add(format!("folder {ordinal} track {track} content name is unavailable or differs between grand doctrines"));
                }
            }
            let selected_subs: BTreeSet<_> = inventory
                .subs
                .get(&track)
                .into_iter()
                .flatten()
                .filter_map(|&id| observer.literal(id))
                .map(Token::c_str)
                .collect();
            let mut subdoctrines = Vec::new();
            let mut seen = BTreeSet::new();
            for token in &database.tactic_sources.sub_order {
                dashboard_text::metadata::check_cancel(query.cancelled)?;
                let Some(source) = database.tactic_sources.sub.get(token) else {
                    continue;
                };
                if !selected_subs.contains(token)
                    && !source
                        .tracks
                        .iter()
                        .any(|track| track_tokens.contains(track))
                {
                    continue;
                }
                if !seen.insert(token.clone()) {
                    continue;
                }
                let spelling = inventory
                    .subs
                    .get(&track)
                    .into_iter()
                    .flatten()
                    .filter_map(|&id| observer.literal(id))
                    .find(|bytes| Token::c_str(bytes) == *token)
                    .or_else(|| spellings.get(token).copied());
                let id = match spelling {
                    Some(bytes) => String::from_utf8(bytes.to_vec()).map_err(io::Error::other),
                    None => match database.presentation(
                        1,
                        match token {
                            Token::Name(bytes) => bytes,
                            Token::Rejected => b"",
                        },
                    ) {
                        Ok(Some(presentation)) => {
                            String::from_utf8(presentation.token.clone()).map_err(io::Error::other)
                        }
                        result => {
                            Err(io::Error::other(result.err().unwrap_or_else(|| {
                                "subdoctrine presentation is absent".into()
                            })))
                        }
                    },
                };
                let id = match id {
                    Ok(id) => id,
                    Err(why) => {
                        notes.add(why.to_string());
                        continue;
                    }
                };
                let named = if reached.is_some() {
                    subdoctrine(
                        &database,
                        catalogue.as_deref(),
                        id.as_bytes(),
                        default_value,
                        reached,
                    )
                    .map(Arc::new)
                } else {
                    cache
                        .subdoctrines
                        .get_or_init(&id.as_bytes().to_vec(), |key| {
                            subdoctrine(&database, catalogue.as_deref(), key, default_value, None)
                        })
                };
                match named {
                    Ok(named) => {
                        subdoctrines.push(named.value.clone());
                        reply.warnings.extend(named.warnings.clone());
                    }
                    Err(why) => notes.add(format!("subdoctrine {id}: {why}")),
                }
            }
            if let Some(recorded) = recorded_tracks.remove(&track.to_string()) {
                for choice in recorded.subdoctrines {
                    if let Some(projected) = subdoctrines.iter_mut().find(|projected| {
                        choice
                            .token_id
                            .as_ref()
                            .is_some_and(|id| reply.tokens.contains_key(id))
                            && projected.id == choice.id
                    }) {
                        projected.token_id = choice.token_id;
                    } else {
                        subdoctrines.push(choice);
                    }
                }
            }
            if !selected_subs.is_subset(&seen) {
                notes.add(format!(
                    "folder {ordinal} track {track} selected subdoctrine is absent from content"
                ));
            }
            tracks.push(DoctrineMasteryTrack {
                id: track.to_string(),
                name: track_name,
                subdoctrines,
            });
        }
        reply.folders.push(DoctrineMasteryFolder {
            id: ordinal.to_string(),
            name,
            icon: folder_icon,
            grand,
            tracks,
        });
    }
    reply.warnings.extend(notes.finish());
    Ok(())
}

#[cfg(test)]
pub(super) fn database_loads(
    source: &Source,
    options: &dashboard_core::config::ContentOptions,
) -> usize {
    cache(source, options).database.loads()
}

#[cfg(test)]
pub(super) fn install_fixture(
    source: &Source,
    options: &dashboard_core::config::ContentOptions,
    catalogue: Arc<Catalogue>,
    database: Database,
    default_reward: i64,
) {
    *source.state::<BoundCache>().lock().unwrap() = Some((
        catalogue::binding(options),
        Arc::new(ContentCache {
            database: CachedResult::loaded(Arc::new(database)),
            default_reward: CachedResult::loaded(Arc::new(default_reward)),
            ..Default::default()
        }),
    ));
    catalogue::set_test_catalogue(source, options, catalogue);
}

#[cfg(test)]
pub(super) fn install_reader_fixture(
    source: &Source,
    options: &dashboard_core::config::ContentOptions,
    database: io::Result<Database>,
    default_reward: io::Result<i64>,
) {
    let cache = ContentCache::default();
    let _ = cache.database.get_or_init(|| database);
    let _ = cache.default_reward.get_or_init(|| default_reward);
    *source.state::<BoundCache>().lock().unwrap() =
        Some((catalogue::binding(options), Arc::new(cache)));
}

#[cfg(test)]
pub(super) fn test_subdoctrine(
    database: &Database,
    catalogue: &Catalogue,
    token: &[u8],
) -> serde_json::Value {
    serde_json::to_value(
        subdoctrine(database, Some(catalogue), token, Ok(1), None)
            .unwrap()
            .value,
    )
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_content_refuses_missing_threshold_without_shifting_reward_order() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content readers");
        let (_, base) = catalogue::tests::folder_catalogue(&[
            (
                "common/doctrines/tracks/base.txt",
                "mod_track={name=BASE_TRACK}",
            ),
            (
                "common/doctrines/subdoctrines/base.txt",
                "hidden_sub={name=HIDDEN track=mod_track}",
            ),
        ]);
        let (_, overlay) = catalogue::tests::folder_catalogue(&[
            (
                "common/defines/mod.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/doctrines/tracks/base.txt",
                "mod_track={name=MOD_TRACK}",
            ),
            (
                "common/doctrines/subdoctrines/mod.txt",
                "mod_sub={name=MOD_SUB track=mod_track rewards={first={mastery=1.25} second={} third={mastery=-2}}}",
            ),
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n MOD_TRACK:0 \"Overlay track\"\n MOD_SUB:0 \"Overlay sub\"\n mod_sub_first:0 \"First\"\n mod_sub_second:0 \"Second\"\n mod_sub_third:0 \"Third\"\n",
            ),
        ]);
        let replace = vec!["common/doctrines/subdoctrines"];
        let layers =
            installed_content_core::ResolvedLayers::from_folders(&[(&base.0, &[]), (&overlay.0, &replace)])
                .unwrap();
        let catalogue = catalogue::tests::layers_catalogue(layers);
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        let database = doctrines::read(&catalogue.content.layers).unwrap();
        assert!(
            database
                .tactic_sources
                .sub
                .get(&Token::c_str(b"hidden_sub"))
                .is_none()
        );
        assert_eq!(
            database.tracks[&Token::c_str(b"mod_track")].value,
            b"MOD_TRACK"
        );
        let missing = subdoctrine(
            &database,
            Some(&catalogue),
            b"mod_sub",
            Err("default define unavailable"),
            None,
        );
        let missing = missing.unwrap();
        assert!(
            missing.value.rewards.as_ref().unwrap()[1]
                .threshold
                .is_none()
        );
        assert!(
            missing
                .warnings
                .iter()
                .any(|why| why.contains("second threshold unavailable"))
        );
        let complete =
            subdoctrine(&database, Some(&catalogue), b"mod_sub", Ok(900_000), None).unwrap();
        assert_eq!(complete.value.name.as_deref(), Some("Overlay sub"));
        assert_eq!(
            complete
                .value
                .rewards
                .as_ref()
                .unwrap()
                .iter()
                .map(|reward| (reward.id.as_str(), reward.threshold.as_deref()))
                .collect::<Vec<_>>(),
            [
                ("first", Some("125000")),
                ("second", Some("900000")),
                ("third", Some("-200000"))
            ]
        );
        assert!(
            complete
                .warnings
                .iter()
                .any(|why| why.contains("nonpositive mastery scale"))
        );
    }

    #[test]
    fn scoped_description_never_evaluates_missing_country_flags_as_false() {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for content readers");
        let (catalogue, _folder) = catalogue::tests::folder_catalogue(&[
            (
                "common/defines/mod.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}",
            ),
            (
                "common/doctrines/tracks/mod.txt",
                "mod_track={name=MOD_TRACK}",
            ),
            (
                "common/doctrines/subdoctrines/mod.txt",
                "mod_sub={name=MOD_SUB track=mod_track rewards={one={mastery=1 effect={if={limit={has_country_flag=mod_flag} add_political_power=3}}}}}",
            ),
            (
                "localisation/mod_l_english.yml",
                "\u{feff}l_english:\n MOD_SUB:0 \"Mod sub\"\n mod_sub_one:0 \"One\"\n mod_sub_one_desc:0 \"Static prose\"\n POLITICS_ADD_POLITICAL_POWER:0 \"Power $VAL|0$\"\n",
            ),
        ]);
        catalogue
            .scripted_content()
            .unwrap()
            .set_synthetic_environment(Default::default());
        let database = doctrines::read(&catalogue.content.layers).unwrap();
        let named = subdoctrine(&database, Some(&catalogue), b"mod_sub", Ok(1), None).unwrap();
        assert!(
            named.value.rewards.as_ref().unwrap()[0]
                .description
                .as_deref()
                .unwrap()
                .contains("Static prose")
        );
        assert!(
            !named.value.rewards.as_ref().unwrap()[0]
                .description
                .as_deref()
                .unwrap()
                .contains("Power 3")
        );
        assert!(
            named
                .warnings
                .iter()
                .any(|why| why.contains("reached-hour operands")),
            "{:?}",
            named.warnings
        );
    }
}
