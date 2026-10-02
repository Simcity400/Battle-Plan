//! Which ideas a country held over one branch (laws, designers, advisors,
//! national spirits, …), with each idea's category and slot from content, for
//! the idea-category timelines and the national spirits widgets.
//!
//! `ActiveIdeas` is sampled every hour, so a span starts at the first hour an
//! idea was observed: it was taken on at some point since the previous
//! sample. Spans never bridge a gap, an incomplete
//! table or a change of country identity. Appointed advisors are not ideas the
//! game lists as active: they come from `HiredAdvisors` (the country's
//! appointed-advisor list, also daily), each with its idea token, seat and
//! character, so an advisor span keeps the seat and character recorded for it;
//! an hour without both tables complete is unobserved. Other ideas take their
//! category and slot from content. Nothing is assumed about which categories
//! exist.

use super::{
    CountryParams, LoadContext, Product,
    idea_format::{LocalisedText, ModifierFormat},
    identity::{NamingCountry, built_template, character_script_token},
    politics_content::{IdeaCountry, PoliticsContent, RecordedName, politics_content},
};
use crate::{
    Artwork, Source,
    catalogue::View,
    metadata::{TaggedCountry, branch_bounds, link_slot, tagged_country, validate_campaign},
    products::country_groups::HourSpan,
};
use installed_content::{
    characters::BuiltTemplate,
    coverage::Gap,
    modifiers::{Modifier, Modifiers},
};
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

/// One recorded character: the holder of an advisor idea, or a country leader.
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

/// One modifier as the game stores it (installed_content::modifiers), before
/// formatting: one line per modifier, its last value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EffectLine {
    /// The modifier, as the game identifies it: its key folded (1424A7A90,
    /// FindTokenNoCase 1424A7060); the formats are keyed the same way.
    pub key: String,
    /// The stored value in fixed point with five decimals, as a decimal
    /// string: the CRT conversion of the authored text into a zeroed
    /// destination (1406042A0, 1424B0480).
    pub fixed5: String,
    /// Named in `hidden_modifier`: applied, never listed (14054F6D0).
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
    /// A targeted modifier's custom tooltip keys, in the order its CModifier
    /// lists them (14054E050); none for an equipment bonus.
    pub tooltips: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TraitEffects {
    pub token: String,
    /// Content defines the trait. When it does not, its effects are unknown and
    /// the lists below are empty for that reason, not because it has none.
    pub known: bool,
    /// Why content cannot say whether it defines the trait (a file that
    /// could stops short); None when it can.
    pub unknown: Option<String>,
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
    /// Content defines the idea or its advisor role, so the lists above are its
    /// effects. When not (an advisor placed only by its recorded seat), they are
    /// empty because they are unknown, not because it has none.
    pub effects_known: bool,
    /// Why content cannot say what the idea is (a file that could define it
    /// stops short); None when it can.
    pub unknown: Option<String>,
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
    /// Hours the country's recorded identity starts, in order: its first hour
    /// seen, after an hour it was not seen, or as another country identity. The
    /// ideas held then are that identity's initial state, not changes.
    pub identity_starts: Vec<i32>,
    /// How the game shows each modifier the effects above use (see `idea_format`);
    /// a key missing here is shown as authored, unformatted.
    pub modifiers: Vec<ModifierFormat>,
    /// The texts the tooltips need: custom tooltips, research categories
    /// (`<category>_research`), equipment names, and `YES` / `NO`.
    pub texts: Vec<LocalisedText>,
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
    pub identity_starts: Vec<i32>,
    /// For each identity start, the country as idea naming reads it
    /// (140FBCF90).
    pub(crate) identity_naming: Vec<NamingCountry>,
    /// Each recorded advisor holder's template (CCharacter+0x20), by
    /// character id, or why the recording does not give it.
    pub(crate) templates: BTreeMap<u32, Result<BuiltTemplate, String>>,
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
        TableId::CountryAppearance,
        // Whether a dynamic country is in civil war (super::identity::civil_war).
        TableId::CountryCivilWars,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::CountryCounterInputs,
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
            let (country, slot) = match tagged_country(countries, links, tag) {
                TaggedCountry::Found(country, slot) => (country, slot),
                // Not a country this hour: its ideas simply are not held.
                TaggedCountry::Absent => continue,
                // The country exists but its identity cannot be read: unknown, not absent.
                TaggedCountry::Unresolved => {
                    mark(&mut result.unobserved, hour, hour);
                    continue;
                }
            };
            let identity = (slot, country.id);
            let continues =
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour));
            last = Some((identity, hour));
            if !continues {
                result.identity_starts.push(hour);
                result
                    .identity_naming
                    .push(NamingCountry::read(decoded, country));
            }
            let owner = |raw: u32| link_slot(links, raw) == Some(slot);
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
                    .and_then(|(row, lexical)| character_script_token(row, lexical))
                    .unwrap_or_default();
                let name = row
                    .and_then(|row| std::str::from_utf8(&row.token).ok())
                    .unwrap_or_default();
                // Only a build or a save load sets CCharacter+0x20, so the
                // first template read stands.
                if !matches!(result.templates.get(&advisor.character), Some(Ok(_))) {
                    let template = match (characters, row) {
                        (_, Some(row)) => built_template(row, lexical),
                        (Some(_), None) => Err(format!(
                            "character {} has no Characters row at hour {hour}",
                            advisor.character
                        )),
                        (None, None) => {
                            Err(format!("the Characters table is incomplete at hour {hour}"))
                        }
                    };
                    result.templates.insert(advisor.character, template);
                }
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
    EffectLine {
        key: modifier.name.clone(),
        fixed5: modifier.value.to_string(),
        hidden,
    }
}

/// A lookup's answer, and the gap's text when content cannot say.
fn known<'a, T>(found: Result<Option<&'a T>, &Gap>) -> (Option<&'a T>, Option<String>) {
    match found {
        Ok(found) => (found, None),
        Err(gap) => (None, Some(gap.to_string())),
    }
}

/// The content an idea is joined with; a catalogue's politics content, or test data.
pub struct Joined<'a> {
    pub ideas: &'a installed_content::ideas::IdeaContent,
    pub characters: &'a installed_content::characters::Characters,
    pub traits: &'a installed_content::leader_traits::LeaderTraits,
    pub localise: &'a dyn Fn(&str, View) -> Option<String>,
}

impl<'a> Joined<'a> {
    pub(crate) fn of(
        content: &'a PoliticsContent,
        localise: &'a dyn Fn(&str, View) -> Option<String>,
    ) -> Self {
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
            lines: lines(&group.modifiers),
            tooltips: group.modifiers.tooltips(),
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
            tooltips: Vec::new(),
        })
        .collect()
}

/// A CModifier's lines: one per stored modifier, a hidden one marked. Its
/// `added_modifier` children are not listed (FormatDescription 14054E050
/// reads the values and the tooltip table only).
fn lines(modifiers: &Modifiers) -> Vec<EffectLine> {
    modifiers
        .values
        .iter()
        .map(|m| line(m, modifiers.hidden.contains(&m.name)))
        .collect()
}

/// Joins an idea to content: category, slot and effects. The seat and character
/// the recording gives an advisor idea are on its spans; `recorded_seat` is the
/// latest of them, used first, so an advisor whose authored role content cannot
/// resolve is still placed. Otherwise the slot comes from content (the
/// character's advisor role, or the idea's own group). `built` is the latest
/// holder's template, which decides a role the character's alternatives
/// disagree on ([`installed_content::characters::Characters::held_advisor`]), or why
/// it is not known.
pub fn info(
    content: &Joined,
    token: &str,
    recorded_seat: Option<&str>,
    built: Result<&BuiltTemplate, &str>,
) -> IdeaInfo {
    let tags = &content.ideas.tags;
    let mut unknown = Vec::new();
    let (idea, gap) = known(content.ideas.idea(token));
    unknown.extend(gap);
    let authored_role = content
        .characters
        .held_advisor(token, built)
        .inspect_err(|reason| unknown.push(reason.clone()))
        .ok()
        .flatten();
    // The idea's group resolves to its category as 140A33240 finds it: by
    // the category's name (a national spirit's) or by a slot it holds.
    let (group_category, gap) = match idea {
        Some(idea) => known(tags.resolve(&idea.group)),
        None => (None, None),
    };
    unknown.extend(gap.clone());
    let named = group_category
        .zip(idea)
        .is_some_and(|(category, idea)| category.name == idea.group);
    let slot = recorded_seat
        .map(str::to_owned)
        .or_else(|| authored_role.map(|role| role.slot.clone()))
        .or_else(|| {
            idea.filter(|_| !named && gap.is_none())
                .map(|idea| idea.group.clone())
        });
    let category = match (group_category, &slot) {
        (Some(category), None) => Some(category.name.clone()),
        (_, Some(slot)) => {
            let (category, gap) = known(tags.resolve(slot));
            unknown.extend(gap);
            category.map(|category| category.name.clone())
        }
        (None, None) => None,
    };
    let trait_tokens = idea
        .map(|idea| idea.traits.clone())
        .or_else(|| authored_role.map(|role| role.traits.clone()))
        .unwrap_or_default();
    let traits = trait_tokens
        .iter()
        .map(|trait_token| {
            let (found, unknown) = known(content.traits.get(trait_token));
            TraitEffects {
                token: trait_token.clone(),
                known: found.is_some(),
                unknown,
                // 140711C80 localises the selected trait's stored name,
                // whose spelling can differ from the reference (140710200).
                name: found.and_then(|found| (content.localise)(&found.token, View::Plain)),
                // The trait's command fields are no lines of the trait: the
                // per-trait formatter (140711C80 -> 1407104A0) never reads
                // them; an advisor shows their sum with its own cost
                // (14128BA50, 14128FDD0, 14128A780).
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
                    .map(|found| found.modifiers.tooltips())
                    .unwrap_or_default(),
            }
        })
        .collect();
    // An idea's effects, else its advisor role's (CAdvisorTemplate 1413CA970
    // keeps `modifier` and `research_bonus`).
    let modifiers = idea
        .map(|idea| &idea.modifiers)
        .or_else(|| authored_role.map(|role| &role.modifiers));
    let research = idea
        .map(|idea| &idea.research_bonuses)
        .or_else(|| authored_role.map(|role| &role.research_bonuses));
    let effects_known = idea.is_some() || authored_role.is_some();
    IdeaInfo {
        token: token.to_owned(),
        category,
        slot,
        law: idea.is_some_and(|idea| idea.law),
        designer: idea.is_some_and(|idea| idea.designer),
        effects: modifiers.map(lines).unwrap_or_default(),
        targeted: idea
            .map(|idea| targeted(&idea.targeted))
            .unwrap_or_default(),
        equipment: idea
            .map(|idea| equipment(&idea.equipment_bonuses))
            .unwrap_or_default(),
        research: research
            .map(|bonuses| bonuses.iter().map(|m| line(m, false)).collect())
            .unwrap_or_default(),
        traits,
        tooltips: modifiers.map(Modifiers::tooltips).unwrap_or_default(),
        effects_known,
        unknown: (!effects_known || !unknown.is_empty())
            .then(|| unknown.join("; "))
            .filter(|text| !text.is_empty()),
    }
}

/// Every category read, in load order. When a file that could declare one
/// stops short the list may be incomplete: [`IdeaHistory`] warns.
pub fn categories(content: &Joined) -> Vec<CategoryInfo> {
    let (categories, _) = content.ideas.tags.categories();
    categories
        .iter()
        .map(|category| {
            let mut slots: Vec<SlotInfo> = category
                .slots
                .iter()
                .map(|slot| SlotInfo {
                    token: slot.clone(),
                    // Slot names are plain (140FBD700).
                    name: (content.localise)(slot, View::Plain),
                    character: false,
                    seats: 1,
                    laws: content
                        .ideas
                        .laws(slot)
                        .0
                        .into_iter()
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
                        // Seat names are plain (141D09EF0).
                        name: (content.localise)(seat, View::Plain),
                        character: true,
                        seats: 1,
                        laws: Vec::new(),
                    }),
                }
            }
            CategoryInfo {
                token: category.name.clone(),
                // Category names are plain (1406708A0).
                name: (content.localise)(&category.name, View::Plain),
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
    const CONTENT_DEPENDENT: bool = true;
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
            ModifierFormat::decl(),
            LocalisedText::decl(),
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
        reply(&content, &source.replay(), params, &scanned, cancelled)
    }
}

fn reply(
    content: &PoliticsContent,
    replay: &Replay,
    params: &CountryParams,
    scanned: &ScannedIdeas,
    cancelled: &AtomicBool,
) -> io::Result<IdeaHistoryReply> {
    let tokens: BTreeSet<&str> = scanned
        .spans
        .iter()
        .map(|span| span.token.as_str())
        .collect();
    let localise = |key: &str, view: View| content.localised(key, view);
    let joined = Joined::of(content, &localise);
    let mut warnings = vec!["Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned()];
    warnings.extend(content.warnings.iter().cloned());
    // A file that could declare a category or law stops short.
    let (_, category_gap) = content.ideas.tags.categories();
    let (_, law_gap) = content.ideas.laws("");
    for gap in category_gap.into_iter().chain(law_gap) {
        warnings.push(format!(
            "Idea categories and laws may be incomplete: {gap}."
        ));
    }
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
        // The identity the latest span belongs to names the idea.
        let naming = latest
            .and_then(|span| {
                let index = scanned
                    .identity_starts
                    .partition_point(|start| *start <= span.first_hour);
                index
                    .checked_sub(1)
                    .and_then(|index| scanned.identity_naming.get(index))
            })
            .cloned()
            .unwrap_or_default();
        let country = if latest.is_some() {
            naming.for_tag(&params.tag)
        } else {
            IdeaCountry {
                tag: &params.tag,
                slot: 0,
                look: None,
            }
        };
        let built = match latest
            .and_then(|span| span.character.as_ref())
            .map(|holder| scanned.templates.get(&holder.id))
        {
            Some(Some(Ok(template))) => Ok(template),
            Some(Some(Err(reason))) => Err(reason.as_str()),
            Some(None) | None => Err("no recorded holder names its template"),
        };
        let found = info(
            &joined,
            token,
            latest.and_then(|span| span.slot.as_deref()),
            built,
        );
        if found.category.is_none() {
            unplaced.push(token.to_owned());
        }
        // A holder's artwork: the content's for its script token, named as recorded.
        let holder_art = |script: Option<&str>, saved: &str| {
            let (mut art, mut problem) = content.idea_artwork(token, script, country);
            // Without a script token the content cannot say whose portrait
            // this is; the idea's authored advisor may be someone else.
            if script.is_none() {
                art.icon = None;
                problem = Some(format!(
                    "{token}: artwork is unknown because the holder has no recorded script token; a placeholder is shown"
                ));
            }
            // The saved name first, so a renamed character shows its current name.
            match content.recorded_name(saved) {
                RecordedName::Named(name) => art.name = Some(name),
                RecordedName::Unresolved(_) => art.name = None,
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
            None => content.idea_artwork(token, None, country),
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
    // YES/NO: FormatValue 14054FE50 (YES at 142760760).
    let mut modifier_keys = BTreeSet::new();
    let mut text_keys: BTreeSet<String> = ["YES", "NO"].map(str::to_owned).into();
    let mut effects = |lines: &[EffectLine], targeted: &[EffectGroup], tooltips: &[String]| {
        modifier_keys.extend(
            lines
                .iter()
                .chain(targeted.iter().flat_map(|group| &group.lines))
                .map(|line| line.key.clone()),
        );
        text_keys.extend(tooltips.iter().cloned());
        text_keys.extend(
            targeted
                .iter()
                .flat_map(|group| group.tooltips.iter().cloned()),
        );
    };
    for idea in &ideas {
        effects(&idea.effects, &idea.targeted, &idea.tooltips);
        for found in &idea.traits {
            effects(&found.lines, &found.targeted, &found.tooltips);
        }
    }
    for idea in &ideas {
        // _research: CCountryTechnologyView::BuildTooltip 1415E5420, 1427516B0.
        text_keys.extend(
            idea.research
                .iter()
                .map(|line| format!("{}_research", line.key)),
        );
        text_keys.extend(
            idea.equipment
                .iter()
                .chain(idea.traits.iter().flat_map(|found| &found.equipment))
                .filter_map(|group| group.target.clone()),
        );
    }
    let (modifiers, format_warnings) = super::idea_format::modifier_formats(
        replay,
        params.branch(),
        &modifier_keys,
        content,
        cancelled,
    )?;
    warnings.extend(format_warnings);
    let texts = super::idea_format::texts(content, &text_keys);
    // Names left out for a localisation refusal, by reason and key.
    warnings.extend(content.catalogue.refusal_lines());
    Ok(IdeaHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        spans: scanned.spans.clone(),
        ideas,
        artwork,
        holders,
        categories: categories(&joined),
        unobserved: scanned.unobserved.clone(),
        identity_starts: scanned.identity_starts.clone(),
        modifiers,
        texts,
        warnings,
    })
}

#[cfg(test)]
mod tests;
