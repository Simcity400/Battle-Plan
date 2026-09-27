//! Which ideas a country held over one branch (laws, designers, advisors,
//! national spirits, …), with each idea's category and slot from content, for
//! the idea-category timelines and the national spirits widgets.
//!
//! `ActiveIdeas` is sampled once a day and carried forward in between, so a
//! span starts at the first hour an idea was observed: it was taken on at some
//! point since the previous sample. Spans never bridge a gap, an incomplete
//! table or a change of country identity. Appointed advisors are not ideas the
//! game lists as active: they come from `HiredAdvisors` (the country's
//! appointed-advisor list, also daily), each with its idea token, seat and
//! character, so an advisor span keeps the seat and character recorded for it;
//! an hour without both tables complete is unobserved. Other ideas take their
//! category and slot from content. Nothing is assumed about which categories
//! exist.

use super::{
    CountryParams, LoadContext, Product,
    politics_content::{PoliticsContent, RecordedName, politics_content},
};
use crate::{
    Artwork, Source,
    metadata::{branch_bounds, mapped_slot, validate_campaign},
    products::country_groups::HourSpan,
};
use installed_content::modifiers::{Amount, Modifier};
use replay::{container::Replay, frame::FrameDecoder, rows::tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct IdeaHistory;

/// Spans kept per country before the scan refuses.
const MAX_SPANS: usize = 100_000;
const CACHED_COUNTRIES: usize = 8;

/// An idea held over consecutive recorded hours. An advisor idea carries the
/// seat and character the recording gave it for this span; a change of either
/// starts a new span.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeaSpan {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: Option<String>,
    pub character: Option<IdeaCharacter>,
}

/// One recorded character behind an advisor idea.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeaCharacter {
    /// The recorded character id.
    pub id: u32,
    /// The script token (resolved through LexicalTokens); empty when unresolved.
    pub token: String,
    /// The saved name as recorded (a localisation key or a literal); a rename
    /// changes it, so it comes before the authored name.
    pub name: String,
}

/// One authored effect line, before formatting: a modifier and its value, or
/// its authored text when the value is scripted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EffectLine {
    pub key: String,
    pub value: Option<f64>,
    /// The authored text of a value this reader does not evaluate.
    pub unsupported: Option<String>,
    /// Applied without showing in the game's own tooltip.
    pub hidden: bool,
}

/// Effects against one country, or to one equipment type.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EffectGroup {
    pub target: Option<String>,
    /// An equipment bonus that applies at once to existing equipment.
    pub instant: bool,
    pub lines: Vec<EffectLine>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TraitEffects {
    pub token: String,
    /// Content defines the trait. When it does not, its effects are unknown and
    /// the lists below are empty for that reason, not because it has none.
    pub known: bool,
    pub name: Option<String>,
    pub lines: Vec<EffectLine>,
    pub targeted: Vec<EffectGroup>,
    pub equipment: Vec<EffectGroup>,
    pub tooltips: Vec<String>,
}

/// What content and the recording say about one idea the country held.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeaInfo {
    pub token: String,
    /// The idea category (`government`, `country`, a mod's own …); None when
    /// neither content nor the recording places it.
    pub category: Option<String>,
    /// The slot or character seat it fills; None for category-wide ideas such
    /// as national spirits.
    pub slot: Option<String>,
    pub law: bool,
    pub designer: bool,
    pub effects: Vec<EffectLine>,
    pub targeted: Vec<EffectGroup>,
    pub equipment: Vec<EffectGroup>,
    pub research: Vec<EffectLine>,
    pub traits: Vec<TraitEffects>,
    /// Custom tooltip localisation keys the idea shows instead of values.
    pub tooltips: Vec<String>,
}

/// One slot of a category, as the category lists it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub token: String,
    pub name: Option<String>,
    /// A character seat (advisors) rather than an idea slot (laws, designers).
    pub character: bool,
    /// How many seats of this slot the category has.
    pub seats: u32,
    /// For a law slot: its laws in content order, so a law's escalation is its place here.
    pub laws: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub token: String,
    pub name: Option<String>,
    /// `type`, such as `national_spirit`.
    pub kind: Option<String>,
    pub hidden: bool,
    pub politics_tab: bool,
    pub slots: Vec<SlotInfo>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IdeaHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub spans: Vec<IdeaSpan>,
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<Artwork>,
    /// Each recorded holder of an advisor idea with that character's own name
    /// and portrait, so a span shows its own holder; `artwork` has the latest.
    pub holders: Vec<HolderArtwork>,
    /// Every category in content order, so widgets list mod categories too.
    pub categories: Vec<CategoryInfo>,
    /// Hours the idea table was not observed for this country: a span ending or
    /// starting next to one is a gap, not a change.
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// The artwork of one character who held an advisor idea.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct HolderArtwork {
    pub token: String,
    /// The recorded character id, as on the span.
    pub character: u32,
    /// The script token and saved name, as on the span: a rename, or a token
    /// that resolves later, has its own artwork.
    pub character_token: String,
    pub name: String,
    pub artwork: Artwork,
}

/// One country's recorded idea history, before content is joined.
#[derive(Clone, Debug, Default)]
pub struct ScannedIdeas {
    pub spans: Vec<IdeaSpan>,
    pub unobserved: Vec<HourSpan>,
    pub warnings: Vec<String>,
}

/// Adds hours to the unobserved spans, merging with the last span they touch.
fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) >= Some(first_hour) => {
            span.first_hour = span.first_hour.min(first_hour);
            span.last_hour = span.last_hour.max(last_hour);
        }
        _ => spans.push(HourSpan {
            first_hour,
            last_hour,
        }),
    }
}

/// Scans one branch for one country's ideas.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeas> {
    use replay::rows::TableId;
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::ActiveIdeas,
        TableId::HiredAdvisors,
        TableId::Characters,
        TableId::LexicalTokens,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Idea request cancelled");
    let mut result = ScannedIdeas::default();
    // Each held idea's open span, and the identity and hour the country was last seen with.
    let mut open: BTreeMap<String, usize> = BTreeMap::new();
    let mut last: Option<((u32, u32), i32)> = None;
    let mut unreadable = 0_u32;
    let mut tokenless = false;
    // Every hour of the branch without a readable frame is unobserved, including
    // before the first and after the last, so an edge gap is never read as a removal.
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start));
    let mut previous_hour = bounds.and_then(|(first, _)| first.checked_sub(1));
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            unreadable += 1;
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                unreadable += 1;
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                unreadable += 1;
                break;
            }
            let hour = frame.hour;
            if let Some(previous) = previous_hour
                && previous.checked_add(1).is_some_and(|next| next < hour)
            {
                mark(&mut result.unobserved, previous + 1, hour - 1);
            }
            previous_hour = Some(hour);
            let (Ok(countries), Ok(links), Ok(ideas), Ok(advisors)) = (
                decoded.complete::<tables::Countries>(),
                decoded.complete::<tables::CountryLinks>(),
                decoded.complete::<tables::ActiveIdeas>(),
                decoded.complete::<tables::HiredAdvisors>(),
            ) else {
                mark(&mut result.unobserved, hour, hour);
                continue;
            };
            let mut named = countries
                .iter()
                .filter(|country| country.tag == tag.as_bytes())
                .peekable();
            if named.peek().is_none() {
                // Not a country this hour: its ideas simply are not held.
                continue;
            }
            let Some((country, slot)) = named
                .find_map(|country| mapped_slot(country, links).ok().map(|slot| (country, slot)))
            else {
                // The country exists but its identity cannot be read: unknown, not absent.
                mark(&mut result.unobserved, hour, hour);
                continue;
            };
            let identity = (slot, country.id);
            let continues =
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            last = Some((identity, hour));
            let owner = |raw: u32| {
                links
                    .binary_search_by_key(&raw, |link| link.id)
                    .ok()
                    .map(|index| links[index].canonical_slot)
                    == Some(slot)
            };
            // Each appointed advisor's idea with its recorded seat and character.
            let characters = decoded.complete::<tables::Characters>().ok();
            let lexical = decoded.complete::<tables::LexicalTokens>().ok();
            let mut roles: BTreeMap<&[u8], (String, IdeaCharacter)> = BTreeMap::new();
            for advisor in advisors.iter().filter(|advisor| owner(advisor.country)) {
                let Ok(seat) = std::str::from_utf8(&advisor.slot) else {
                    continue;
                };
                if advisor.idea.is_empty() {
                    // The appointment has no recorded idea: nothing to name it by.
                    tokenless = true;
                    continue;
                }
                // The character's script token (its saved name is another field);
                // unresolved, the holder stays unidentified rather than misnamed.
                let row = characters.and_then(|rows| {
                    rows.binary_search_by_key(&advisor.character, |character| character.id)
                        .ok()
                        .map(|index| &rows[index])
                });
                let token = row
                    .zip(lexical)
                    .and_then(|(row, lexical)| super::character_script_token(row, lexical))
                    .unwrap_or_default();
                let name = row
                    .and_then(|row| std::str::from_utf8(&row.token).ok())
                    .unwrap_or_default();
                roles.insert(
                    advisor.idea.as_slice(),
                    (
                        seat.to_owned(),
                        IdeaCharacter {
                            id: advisor.character,
                            token: token.to_owned(),
                            name: name.to_owned(),
                        },
                    ),
                );
            }
            let held: BTreeSet<&[u8]> = ideas
                .iter()
                .filter(|row| owner(row.country))
                .map(|row| row.token.as_slice())
                .chain(roles.keys().copied())
                .filter(|token| !token.is_empty())
                .collect();
            if !continues {
                open.clear();
            }
            open.retain(|token, _| held.contains(token.as_bytes()));
            for token in held {
                let Ok(token) = std::str::from_utf8(token) else {
                    continue;
                };
                let (slot, character) = roles
                    .get(token.as_bytes())
                    .cloned()
                    .map_or((None, None), |(slot, character)| {
                        (Some(slot), Some(character))
                    });
                match open.get(token) {
                    // The same holder in the same seat continues its span.
                    Some(&index)
                        if result.spans[index].slot == slot
                            && result.spans[index].character == character =>
                    {
                        result.spans[index].last_hour = hour
                    }
                    _ => {
                        if result.spans.len() >= MAX_SPANS {
                            return Err(io::Error::other("Idea history exceeds its memory budget"));
                        }
                        open.insert(token.to_owned(), result.spans.len());
                        result.spans.push(IdeaSpan {
                            token: token.to_owned(),
                            first_hour: hour,
                            last_hour: hour,
                            slot,
                            character,
                        });
                    }
                }
            }
        }
    }
    if let (Some((first, last)), previous) = (bounds, previous_hour) {
        let from = previous.map_or(first, |previous| previous.saturating_add(1));
        if from <= last {
            mark(&mut result.unobserved, from, last);
        }
    }
    if tokenless {
        result.warnings.push(
            "Some appointed advisors have no recorded idea, so their seats show as empty.".into(),
        );
    }
    if unreadable > 0 {
        result.warnings.push(format!(
            "Part of this branch could not be read ({unreadable} blocks or reads); idea history is missing there."
        ));
    }
    Ok(result)
}

fn line(modifier: &Modifier, hidden: bool) -> EffectLine {
    let (value, unsupported) = match &modifier.amount {
        Amount::Number(number) => (Some(*number), None),
        Amount::Flag(flag) => (Some(f64::from(u8::from(*flag))), None),
        Amount::Unsupported(text) => (None, Some(text.clone())),
    };
    EffectLine {
        key: modifier.key.clone(),
        value,
        unsupported,
        hidden,
    }
}

/// The content an idea is joined with; a catalogue's politics content, or test data.
pub struct Joined<'a> {
    pub ideas: &'a installed_content::ideas::IdeaContent,
    pub characters: &'a installed_content::characters::Characters,
    pub traits: &'a BTreeMap<String, installed_content::leader_traits::LeaderTrait>,
    pub localise: &'a dyn Fn(&str) -> Option<String>,
}

impl<'a> Joined<'a> {
    fn of(content: &'a PoliticsContent, localise: &'a dyn Fn(&str) -> Option<String>) -> Self {
        Self {
            ideas: &content.ideas,
            characters: &content.characters,
            traits: &content.traits,
            localise,
        }
    }
}

fn targeted(groups: &[installed_content::modifiers::TargetedModifier]) -> Vec<EffectGroup> {
    groups
        .iter()
        .map(|group| EffectGroup {
            target: group.tag.clone(),
            instant: false,
            lines: group.values.iter().map(|m| line(m, false)).collect(),
        })
        .collect()
}

fn equipment(bonuses: &[installed_content::modifiers::EquipmentBonus]) -> Vec<EffectGroup> {
    bonuses
        .iter()
        .map(|bonus| EffectGroup {
            target: Some(bonus.equipment.clone()),
            instant: bonus.instant,
            lines: bonus.values.iter().map(|m| line(m, false)).collect(),
        })
        .collect()
}

fn lines(modifiers: &installed_content::modifiers::Modifiers) -> Vec<EffectLine> {
    modifiers
        .values
        .iter()
        .map(|m| line(m, false))
        .chain(modifiers.hidden.iter().map(|m| line(m, true)))
        .collect()
}

/// Joins an idea to content: category, slot and effects. The seat and character
/// the recording gives an advisor idea are on its spans; `recorded_seat` is the
/// latest of them, used first, so an advisor whose authored role content cannot
/// resolve (conditional alternatives) is still placed. Otherwise the slot comes
/// from content (the character's advisor role, or the idea's own group).
pub fn info(content: &Joined, token: &str, recorded_seat: Option<&str>) -> IdeaInfo {
    let tags = &content.ideas.tags;
    let idea = content.ideas.ideas.get(token);
    let authored_role = content.characters.advisors.get(token);
    let slot = recorded_seat
        .map(str::to_owned)
        .or_else(|| authored_role.map(|role| role.slot.clone()))
        .or_else(|| {
            idea.filter(|idea| tags.category(&idea.group).is_none())
                .map(|idea| idea.group.clone())
        });
    let category = match (idea, &slot) {
        (Some(idea), _) if tags.category(&idea.group).is_some() => Some(idea.group.clone()),
        (_, Some(slot)) => tags
            .category_of_slot(slot)
            .map(|category| category.name.clone()),
        _ => None,
    };
    let trait_tokens = idea
        .map(|idea| idea.traits.clone())
        .or_else(|| authored_role.map(|role| role.traits.clone()))
        .unwrap_or_default();
    let traits = trait_tokens
        .iter()
        .map(|trait_token| {
            let found = content.traits.get(trait_token);
            TraitEffects {
                token: trait_token.clone(),
                known: found.is_some(),
                name: (content.localise)(trait_token),
                lines: found
                    .map(|found| lines(&found.modifiers))
                    .unwrap_or_default(),
                targeted: found
                    .map(|found| targeted(&found.targeted))
                    .unwrap_or_default(),
                equipment: found
                    .map(|found| equipment(&found.equipment_bonuses))
                    .unwrap_or_default(),
                tooltips: found
                    .map(|found| found.modifiers.tooltips.clone())
                    .unwrap_or_default(),
            }
        })
        .collect();
    IdeaInfo {
        token: token.to_owned(),
        category,
        slot,
        law: idea.is_some_and(|idea| idea.law),
        designer: idea.is_some_and(|idea| idea.designer),
        effects: idea.map(|idea| lines(&idea.modifiers)).unwrap_or_default(),
        targeted: idea
            .map(|idea| targeted(&idea.targeted))
            .unwrap_or_default(),
        equipment: idea
            .map(|idea| equipment(&idea.equipment_bonuses))
            .unwrap_or_default(),
        research: idea
            .map(|idea| {
                idea.research_bonuses
                    .iter()
                    .map(|m| line(m, false))
                    .collect()
            })
            .unwrap_or_default(),
        traits,
        tooltips: idea
            .map(|idea| idea.modifiers.tooltips.clone())
            .unwrap_or_default(),
    }
}

pub fn categories(content: &Joined) -> Vec<CategoryInfo> {
    content
        .ideas
        .tags
        .categories
        .iter()
        .map(|category| {
            let mut slots: Vec<SlotInfo> = category
                .slots
                .iter()
                .map(|slot| SlotInfo {
                    token: slot.clone(),
                    name: (content.localise)(slot),
                    character: false,
                    seats: 1,
                    laws: content
                        .ideas
                        .order
                        .iter()
                        .filter_map(|token| content.ideas.ideas.get(token))
                        .filter(|idea| idea.law && idea.group == *slot)
                        .map(|idea| idea.token.clone())
                        .collect(),
                })
                .collect();
            for seat in &category.character_slots {
                match slots
                    .iter_mut()
                    .find(|slot| slot.character && slot.token == *seat)
                {
                    Some(slot) => slot.seats += 1,
                    None => slots.push(SlotInfo {
                        token: seat.clone(),
                        name: (content.localise)(seat),
                        character: true,
                        seats: 1,
                        laws: Vec::new(),
                    }),
                }
            }
            CategoryInfo {
                token: category.name.clone(),
                name: (content.localise)(&category.name),
                kind: category.kind.clone(),
                hidden: category.hidden,
                politics_tab: category.politics_tab,
                slots,
            }
        })
        .collect()
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedIdeas>)>,
}

impl Product for IdeaHistory {
    const KEY: &'static str = "idea-history";
    type Params = CountryParams;
    type Reply = IdeaHistoryReply;

    fn types() -> Vec<String> {
        vec![
            IdeaSpan::decl(),
            IdeaCharacter::decl(),
            EffectLine::decl(),
            EffectGroup::decl(),
            TraitEffects::decl(),
            HolderArtwork::decl(),
            IdeaInfo::decl(),
            SlotInfo::decl(),
            CategoryInfo::decl(),
            IdeaHistoryReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let content = politics_content(source, context.content()?)?;
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        // Scan outside the lock, so other countries' requests are not held up.
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let tokens: BTreeSet<&str> = scanned
            .spans
            .iter()
            .map(|span| span.token.as_str())
            .collect();
        let localise = |key: &str| content.localise(key);
        let joined = Joined::of(&content, &localise);
        let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned()];
        warnings.extend(content.warnings.iter().cloned());
        warnings.extend(scanned.warnings.iter().cloned());
        let mut artwork = Vec::new();
        let mut holders = Vec::new();
        let mut ideas = Vec::new();
        let mut unplaced = Vec::new();
        for token in tokens {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Idea request cancelled",
                ));
            }
            let latest = scanned.spans.iter().rev().find(|span| span.token == token);
            let found = info(&joined, token, latest.and_then(|span| span.slot.as_deref()));
            if found.category.is_none() {
                unplaced.push(token.to_owned());
            }
            // A holder's artwork: the content's for its script token, named as recorded.
            let holder_art = |script: Option<&str>, saved: &str| {
                let (mut art, problem) = content.idea_artwork(token, script);
                // Without a script token the content cannot say whose portrait
                // this is; the idea's authored advisor may be someone else.
                if script.is_none() {
                    art.icon = None;
                }
                // The saved name first, so a renamed character shows its current name.
                match content.recorded_name(saved, script) {
                    RecordedName::Named(name) => art.name = Some(name),
                    RecordedName::Unresolved => art.name = None,
                    RecordedName::None => {}
                }
                (art, problem)
            };
            // The idea's artwork shows its latest holder as recorded; each holder
            // also has its own.
            let (art, problem) = match latest.and_then(|span| span.character.as_ref()) {
                Some(holder) => holder_art(
                    (!holder.token.is_empty()).then_some(holder.token.as_str()),
                    &holder.name,
                ),
                None => content.idea_artwork(token, None),
            };
            artwork.push(art);
            warnings.extend(problem);
            let mut seen = BTreeSet::new();
            for holder in scanned
                .spans
                .iter()
                .filter(|span| span.token == token)
                .filter_map(|span| span.character.as_ref())
                .filter(|holder| {
                    (!holder.token.is_empty() || !holder.name.is_empty())
                        && seen.insert((holder.id, holder.token.clone(), holder.name.clone()))
                })
            {
                let script = (!holder.token.is_empty()).then_some(holder.token.as_str());
                let (art, problem) = holder_art(script, &holder.name);
                holders.push(HolderArtwork {
                    token: token.to_owned(),
                    character: holder.id,
                    character_token: holder.token.clone(),
                    name: holder.name.clone(),
                    artwork: art,
                });
                warnings.extend(problem);
            }
            ideas.push(found);
        }
        if !unplaced.is_empty() {
            warnings.push(format!(
                "{} ideas are in no category the content defines: {}.",
                unplaced.len(),
                unplaced
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(IdeaHistoryReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            spans: scanned.spans.clone(),
            ideas,
            artwork,
            holders,
            categories: categories(&joined),
            unobserved: scanned.unobserved.clone(),
            warnings,
        })
    }
}
