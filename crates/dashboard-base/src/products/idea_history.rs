//! Which ideas a country held over one branch (laws, designers, advisors,
//! national spirits, …), with each idea's category and slot from content, for
//! the idea-category timelines and the national spirits widgets.
//!
//! `ActiveIdeas` is sampled every hour, so a span starts at the first hour an
//! idea was observed: it was taken on at some point since the previous
//! sample. Spans never bridge a gap, an incomplete
//! table or a change of country identity. Appointed advisors are not ideas the
//! game lists as active: they come from `HiredAdvisors` (the country's
//! appointed-advisor list, also hourly), each with its idea token, seat and
//! character, so an advisor span keeps the seat and character recorded for it;
//! an hour without both tables complete is unobserved. Other ideas take their
//! category and slot from content. Nothing is assumed about which categories
//! exist.

//!
//! The family is prepared from native recordings (`dashboard_ideas`); the row recording's scan
//! below ([`Scan`]) is kept as its parity oracle.

use super::{
    CountryParams, Product,
    idea_format::{LocalisedText, ModifierFormat},
    identity::NamingCountry,
    politics_content::{IdeaCountry, PoliticsContent},
};
#[cfg(any(test, feature = "test-support"))]
use super::{
    LoadContext,
    identity::{built_template, character_script_token},
    politics_content::politics_content,
};
#[cfg(any(test, feature = "test-support"))]
use crate::{
    Source,
    metadata::{CountryKeys, branch_bounds, link_slot, tagged_countries, validate_campaign},
};
use crate::{Artwork, catalogue::View, products::country_groups::HourSpan};
use installed_content_core::{
    characters::BuiltTemplate, coverage::Gap, modifiers::Modifier, modifiers::Modifiers,
};
#[cfg(any(test, feature = "test-support"))]
use replay::container::Replay;
#[cfg(any(test, feature = "test-support"))]
use replay_data::{rows::TableId, rows::tables};
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::FrameDecoder;
use serde::{Deserialize, Serialize};
#[cfg(any(test, feature = "test-support"))]
use std::{collections::VecDeque, sync::Arc};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;

pub struct IdeaHistory;

#[cfg(any(test, feature = "test-support"))]
const TABLES: [TableId; 12] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ActiveIdeas,
    TableId::HiredAdvisors,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::CountryCivilWars,
    TableId::CountryWars,
    TableId::WarRelations,
    TableId::CountryCounterInputs,
];

#[cfg(any(test, feature = "test-support"))]
pub const CACHED_BRANCHES: usize = 2;
#[cfg(any(test, feature = "test-support"))]
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

/// One modifier as the game stores it (installed_content_core::modifiers), before
/// formatting: one line per modifier, its last value.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct EffectLine {
    /// The modifier's case-folded key; formats are keyed the same way.
    pub key: String,
    /// The stored fixed5 value as a decimal string. Authored text converts
    /// into a zeroed destination.
    pub fixed5: String,
    /// Named in hidden_modifier: applied, never listed.
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
    /// A targeted modifier's custom tooltip keys in stored order;
    /// none for an equipment bonus.
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub approximate: Vec<HourSpan>,
    /// Exact held set at a requested hour beyond the prepared prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub held: Option<HeldIdeas>,
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

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HeldIdeas {
    pub hour: i32,
    pub slot: u32,
    pub country_id: u32,
    pub spans: Vec<IdeaSpan>,
    pub preparing: Vec<String>,
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedIdeas {
    pub spans: Vec<IdeaSpan>,
    pub unobserved: Vec<HourSpan>,
    pub identity_starts: Vec<i32>,
    /// For each identity start, the country used for idea naming.
    pub identity_naming: Vec<NamingCountry>,
    /// Each recorded advisor holder's template, by character id,
    /// or why the recording does not give it.
    pub templates: BTreeMap<u32, Result<BuiltTemplate, String>>,
    pub warnings: Vec<String>,
    /// Feed-only naming at each span's first hour, in the same order as `spans`.
    pub span_naming: Vec<NamingCountry>,
    /// Starts of the same identity after unobserved hours, where additions
    /// cannot be separated from holdings that continued through the gap.
    pub gap_starts: Vec<i32>,
}

/// The request of the idea history widgets: a country's history through an hour.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdeaHistoryQuery {
    pub branch_id: String,
    pub tag: String,
    pub through: Option<i32>,
    pub hour: Option<i32>,
    pub preview: Option<bool>,
}

/// What the event feed reads of the ideas family: every addition in hour order, then tag
/// order, then token order, with what its rows need and the family's refusals.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    pub additions: Vec<Addition>,
    /// Whether any hour held every input complete with resolved countries.
    pub observed: bool,
    /// Each country's advisor templates, by character id.
    pub templates: BTreeMap<String, BTreeMap<u32, Result<BuiltTemplate, String>>>,
    /// Refused observations by reason, with their counts, in the order the feed lists them.
    pub refusals: Vec<(String, usize)>,
    /// Changed holdings first observed right after an unobserved stretch.
    pub gap_additions: usize,
    /// The recorded modifier definitions by name, as last recorded.
    pub definitions: Option<BTreeMap<String, replay_data::rows::ModifierDefinitionRow>>,
}

impl Feed {
    pub fn bytes(&self) -> usize {
        let text = |text: &str| 24 + text.len();
        std::mem::size_of::<Self>()
            + self
                .additions
                .iter()
                .map(|addition| {
                    std::mem::size_of::<Addition>()
                        + text(&addition.tag)
                        + text(&addition.span.token)
                        + addition.span.slot.as_deref().map_or(0, text)
                        + addition.span.character.as_ref().map_or(0, |holder| {
                            text(&holder.token) + text(&holder.name)
                        })
                })
                .sum::<usize>()
            + self
                .templates
                .values()
                .map(|templates| 64 + templates.len() * 96)
                .sum::<usize>()
            + self
                .refusals
                .iter()
                .map(|(reason, _)| text(reason))
                .sum::<usize>()
            + self.definitions.as_ref().map_or(0, |definitions| {
                definitions
                    .iter()
                    .map(|(key, row)| {
                        text(key)
                            + std::mem::size_of_val(row)
                            + row.name_key.len()
                            + row.postfix_key.len()
                            + row.argument.len()
                    })
                    .sum::<usize>()
            })
    }
}

/// The reasons an observation is refused, as the feed and the reply name them.
pub mod refusal {
    pub const TAG: &str = "Countries tag is not UTF-8; idea holder cannot be identified.";
    pub const IDEA_HOLDER: &str =
        "ActiveIdeas owner has no resolved Countries holder; holding cannot be attributed.";
    pub const IDEA_LINK: &str =
        "ActiveIdeas owner is unresolved through CountryLinks; holding cannot be attributed.";
    pub const ADVISOR_HOLDER: &str =
        "HiredAdvisors owner has no resolved Countries holder; appointment cannot be attributed.";
    pub const ADVISOR_LINK: &str =
        "HiredAdvisors owner is unresolved through CountryLinks; appointment cannot be attributed.";
    pub const SEAT: &str = "HiredAdvisors seat is not UTF-8; appointment cannot be identified.";
    pub const TOKEN: &str = "Held idea token is not UTF-8; holding cannot be identified.";
    /// The feed's count of appointments without an idea token.
    pub const TOKENLESS: &str =
        "Appointed advisors have no recorded idea token, so cannot be named.";
    /// A country's warning that it holds such an appointment.
    pub const TOKENLESS_SEATS: &str =
        "Some appointed advisors have no recorded idea, so their seats show as empty.";
}

/// Why an advisor template is unknown when its character has no Characters row at `hour`.
pub fn missing_template(id: u32, complete: bool, hour: i32) -> String {
    if complete {
        format!("character {id} has no Characters row at hour {hour}")
    } else {
        format!("the Characters table is incomplete at hour {hour}")
    }
}

/// Witnessed additions of active ideas and appointed advisors only.
/// A rename or newly resolved script token for the same character and seat
/// does not hire an advisor again. Every sampled addition is dated by
/// first observation, an upper bound rather than a writer hour.
#[cfg(test)]
pub(crate) fn additions(scanned: &ScannedIdeas) -> impl Iterator<Item = &IdeaSpan> {
    addition_indices(scanned).map(move |index| &scanned.spans[index])
}

/// Each addition with the naming read at the same span's first hour.
#[cfg(any(test, feature = "test-support"))]
pub fn named_additions(
    scanned: &ScannedIdeas,
) -> impl Iterator<Item = (&IdeaSpan, Option<&NamingCountry>)> {
    addition_indices(scanned)
        .map(move |index| (&scanned.spans[index], scanned.span_naming.get(index)))
}

#[cfg(any(test, feature = "test-support"))]
pub fn addition_indices(scanned: &ScannedIdeas) -> impl Iterator<Item = usize> {
    let mut previous: BTreeMap<&str, &IdeaSpan> = BTreeMap::new();
    scanned
        .spans
        .iter()
        .enumerate()
        .filter(move |(_, span)| {
            let preceding = previous.insert(&span.token, span);
            let same_holding = preceding.is_some_and(|last| {
                last.last_hour.checked_add(1) == Some(span.first_hour)
                    && last.slot == span.slot
                    && last.character.as_ref().map(|holder| holder.id)
                        == span.character.as_ref().map(|holder| holder.id)
            });
            let after_gap = scanned
                .unobserved
                .partition_point(|gap| gap.first_hour < span.first_hour)
                .checked_sub(1)
                .and_then(|index| scanned.unobserved.get(index))
                .is_some_and(|gap| gap.last_hour.checked_add(1) >= Some(span.first_hour));
            !same_holding
                && !after_gap
                && scanned
                    .identity_starts
                    .binary_search(&span.first_hour)
                    .is_err()
        })
        .map(|(index, _)| index)
}

/// New or changed holdings first observed after a gap cannot prove an
/// appointment or a law change. The feed names and counts these refusals.
#[cfg(any(test, feature = "test-support"))]
pub fn unavailable_additions(scanned: &ScannedIdeas) -> usize {
    let mut previous: BTreeMap<&str, &IdeaSpan> = BTreeMap::new();
    scanned
        .spans
        .iter()
        .filter(|span| {
            let preceding = previous.insert(&span.token, span);
            let gap = scanned
                .unobserved
                .partition_point(|gap| gap.first_hour < span.first_hour)
                .checked_sub(1)
                .and_then(|index| scanned.unobserved.get(index))
                .filter(|gap| gap.last_hour.checked_add(1) == Some(span.first_hour));
            scanned.gap_starts.binary_search(&span.first_hour).is_ok()
                && gap.is_some_and(|gap| {
                    preceding.is_none_or(|last| {
                        last.slot != span.slot
                            || last.character.as_ref().map(|holder| holder.id)
                                != span.character.as_ref().map(|holder| holder.id)
                            || last.last_hour.checked_add(1) != Some(gap.first_hour)
                    })
                })
        })
        .count()
}

/// Adds hours to the unobserved spans, merging with the last span they touch.
pub fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
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

/// The feed's held-idea spans and row-hour naming. The topic product retains
/// only its selected country's history, through the same observer.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default)]
pub struct ScannedIdeaBranch {
    pub countries: BTreeMap<String, ScannedIdeas>,
    pub observed: bool,
    pub warnings: Vec<String>,
    /// Feed-only refused observations, each with its named reason and count.
    pub refusals: Vec<(String, usize)>,
    absent: ScannedIdeas,
}

#[cfg(any(test, feature = "test-support"))]
impl ScannedIdeaBranch {
    fn into_country(mut self, tag: &str) -> ScannedIdeas {
        let mut result = self.countries.remove(tag).unwrap_or(self.absent);
        result.warnings.extend(
            self.refusals
                .into_iter()
                .map(|(reason, count)| format!("{reason} (count: {count})")),
        );
        result.span_naming.clear();
        result
    }
    pub fn country(&self, tag: &str) -> &ScannedIdeas {
        self.countries.get(tag).unwrap_or(&self.absent)
    }
}

#[cfg(any(test, feature = "test-support"))]
type RefusedRows = BTreeMap<String, BTreeSet<(u32, u32, Vec<u8>)>>;

/// A holding the event feed lists: a law or an appointment the hour before did not hold, with
/// its holder's naming at that hour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Addition {
    pub tag: String,
    pub span: IdeaSpan,
    pub naming: Option<NamingCountry>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Default, Serialize, Deserialize)]
struct CountryScan {
    result: ScannedIdeas,
    open: BTreeMap<String, usize>,
    last: Option<((u32, u32), i32)>,
    tokenless: BTreeSet<(u32, String)>,
    refused: RefusedRows,
    /// Only missing-table/row template refusals include the observation hour.
    #[serde(skip)]
    dated_templates: BTreeMap<u32, bool>,
}

#[cfg(any(test, feature = "test-support"))]
struct CountryRead<'a> {
    country: &'a replay_data::rows::CountryRow,
    slot: u32,
    ideas: Option<&'a Vec<&'a replay_data::rows::ActiveIdeaRow>>,
    advisors: Option<&'a Vec<&'a replay_data::rows::HiredAdvisorRow>>,
    appearance: Option<&'a replay_data::rows::CountryAppearanceRow>,
    topic_appearance: Option<&'a replay_data::rows::CountryAppearanceRow>,
}

#[cfg(any(test, feature = "test-support"))]
impl CountryScan {
    fn advance(&mut self, hour: i32) {
        let Some((_, previous)) = self.last.as_mut() else {
            return;
        };
        if hour <= *previous {
            return;
        }
        for &index in self.open.values() {
            self.result.spans[index].last_hour = hour;
        }
        *previous = hour;
    }

    fn observe(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        read: CountryRead<'_>,
        feed: bool,
    ) {
        self.dated_templates.clear();
        let Self {
            result,
            open,
            last,
            tokenless,
            refused,
            dated_templates,
            ..
        } = self;
        let CountryRead {
            country,
            slot,
            ideas,
            advisors,
            appearance,
            topic_appearance,
        } = read;
        let hour = decoded.hour;
        let identity = (slot, country.id);
        let continues = last
            .as_ref()
            .is_some_and(|(seen, at)| *seen == identity && at.checked_add(1) == Some(hour));
        if !continues && last.as_ref().is_some_and(|(seen, _)| *seen == identity) {
            result.gap_starts.push(hour);
        }
        *last = Some((identity, hour));
        if !continues {
            result.identity_starts.push(hour);
            result
                .identity_naming
                .push(NamingCountry::read_with_appearance(
                    decoded,
                    country,
                    topic_appearance,
                ));
        }
        // Each appointed advisor's idea with its recorded seat and character.
        let characters = decoded.complete::<tables::Characters>().ok();
        let lexical = decoded.complete::<tables::LexicalTokens>().ok();
        let mut roles = BTreeMap::new();
        for advisor in advisors.into_iter().flatten() {
            let Ok(seat) = std::str::from_utf8(&advisor.slot) else {
                refused
                    .entry(
                        refusal::SEAT.into(),
                    )
                    .or_default()
                    .insert((country.id, advisor.ordinal, advisor.slot.clone()));
                continue;
            };
            if advisor.idea.is_empty() {
                // The appointment has no recorded idea: nothing to name it by.
                tokenless.insert((advisor.character, seat.to_owned()));
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
            // Only a build or save load sets the character's template, so
            // the first template read stands.
            if !matches!(result.templates.get(&advisor.character), Some(Ok(_))) {
                let template = match (characters, row) {
                    (_, Some(row)) => built_template(row, lexical),
                    (characters, None) => Err(missing_template(
                        advisor.character,
                        characters.is_some(),
                        hour,
                    )),
                };
                if row.is_none() {
                    dated_templates.insert(advisor.character, characters.is_some());
                }
                result.templates.insert(advisor.character, template);
            }
            roles.insert(
                advisor.idea.as_slice(),
                (seat, advisor.character, token, name),
            );
        }
        let held: BTreeSet<&[u8]> = ideas
            .into_iter()
            .flatten()
            .map(|row| row.token.as_slice())
            .chain(roles.keys().copied())
            .filter(|token| !token.is_empty())
            .collect();
        if !continues {
            open.clear();
        }
        open.retain(|token, _| held.contains(token.as_bytes()));
        let mut naming = None;
        for token in held {
            let Ok(token) = std::str::from_utf8(token) else {
                refused
                    .entry(refusal::TOKEN.into())
                    .or_default()
                    .insert((country.id, 0, token.to_vec()));
                continue;
            };
            let role = roles.get(token.as_bytes());
            match open.get(token) {
                // The same holder in the same seat continues its span.
                Some(&index)
                    if result.spans[index].slot.as_deref() == role.map(|role| role.0)
                        && match (&result.spans[index].character, role) {
                            (None, None) => true,
                            (Some(holder), Some((_, id, token, name))) => {
                                holder.id == *id && holder.token == *token && holder.name == *name
                            }
                            _ => false,
                        } =>
                {
                    result.spans[index].last_hour = hour
                }
                _ => {
                    if feed {
                        result.span_naming.push(
                            naming
                                .get_or_insert_with(|| {
                                    NamingCountry::read_with_appearance(
                                        decoded, country, appearance,
                                    )
                                })
                                .clone(),
                        );
                    }
                    open.insert(token.to_owned(), result.spans.len());
                    result.spans.push(IdeaSpan {
                        token: token.to_owned(),
                        first_hour: hour,
                        last_hour: hour,
                        slot: role.map(|role| role.0.to_owned()),
                        character: role.map(|(_, id, token, name)| IdeaCharacter {
                            id: *id,
                            token: (*token).to_owned(),
                            name: (*name).to_owned(),
                        }),
                    });
                }
            }
        }
    }

    fn date_template_refusals(&mut self, hour: i32) {
        for (id, complete) in &self.dated_templates {
            self.result
                .templates
                .insert(*id, Err(missing_template(*id, *complete, hour)));
        }
    }
}

/// Scans and retains only the requested country's topic history.
#[cfg(any(test, feature = "test-support"))]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeas> {
    Ok(scan_selected(replay, branch, Some(tag), cancelled)?.into_country(tag))
}

#[cfg(any(test, feature = "test-support"))]
pub fn scan_all(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeaBranch> {
    scan_selected(replay, branch, None, cancelled)
}

/// The row recording's idea scan, including open holdings and deduplicated refusals: the
/// parity oracle of the native family.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct Scan {
    states: BTreeMap<String, CountryScan>,
    unobserved: Vec<HourSpan>,
    unreadable: u32,
    observed: bool,
    refused: RefusedRows,
    previous_hour: Option<i32>,
    native: IdeaInputs,
    active: BTreeSet<String>,
    unresolved: BTreeSet<String>,
    dated: BTreeSet<String>,
    pub work: crate::prepared::observer_tests::Work,
    pub test_changes: Option<crate::prepared::frame_changes::FrameChangeTracker>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct IdeaInputs {
    complete: bool,
    keys: Option<CountryKeys>,
    country_view: Option<replay_frame::frame::ViewIdentity>,
    countries: Option<Vec<replay_data::rows::CountryRow>>,
    links: Option<Vec<replay_data::rows::CountryLinkRow>>,
    holders: BTreeMap<String, Option<(usize, u32)>>,
    slots: BTreeMap<u32, Vec<String>>,
    idea_rows: Option<Vec<replay_data::rows::ActiveIdeaRow>>,
    advisor_rows: Option<Vec<replay_data::rows::HiredAdvisorRow>>,
    ideas: Option<BTreeMap<u32, Vec<replay_data::rows::ActiveIdeaRow>>>,
    advisors: Option<BTreeMap<u32, Vec<replay_data::rows::HiredAdvisorRow>>>,
    appearances: BTreeMap<u32, Vec<replay_data::rows::CountryAppearanceRow>>,
    characters: Option<BTreeMap<u32, replay_data::rows::CharacterRow>>,
    character_users: BTreeMap<u32, BTreeSet<String>>,
    lexical: Option<BTreeMap<u32, Vec<u8>>>,
    script_users: BTreeMap<u32, BTreeSet<String>>,
}
#[cfg(any(test, feature = "test-support"))]
impl IdeaInputs {
    fn refresh(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
        refused: &mut RefusedRows,
    ) -> BTreeSet<String> {
        let _cpu = ::replay_frame::cpu::scope_phase("ideas:inputs");
        let changed =
            |table: TableId| changes.is_none_or(|changes| changes.rows_changed[table.index()]);
        let mut roster = false;
        let known_inputs = frame.complete::<tables::Countries>().is_ok()
            && frame.complete::<tables::CountryLinks>().is_ok()
            && frame.complete::<tables::ActiveIdeas>().is_ok()
            && frame.complete::<tables::HiredAdvisors>().is_ok();
        let admission = known_inputs && !self.complete;
        self.complete = known_inputs;
        let mut dirty = BTreeSet::new();
        if changed(TableId::Countries) {
            let rows = frame.complete::<tables::Countries>().ok();
            if !self
                .country_view
                .as_ref()
                .is_some_and(|view| view.is(&frame.tables[TableId::Countries.index()]))
                && !rows.is_some_and(|rows| {
                    self.keys.as_ref().is_some_and(|keys| {
                        keys.same_changes(
                            rows,
                            changes.and_then(|changes| changes.rows(TableId::Countries)),
                        )
                    })
                })
            {
                self.keys = rows.map(CountryKeys::read);
                self.countries = rows.map(|rows| rows.to_vec());
                roster = true;
                for country in rows.into_iter().flatten() {
                    if known_inputs && std::str::from_utf8(&country.tag).is_err() {
                        refused
                            .entry(
                                refusal::TAG.into(),
                            )
                            .or_default()
                            .insert((country.id, country.canonical_slot, country.tag.clone()));
                    }
                }
            }
            self.country_view = rows.map(|_| frame.tables[TableId::Countries.index()].identity());
        }
        if changed(TableId::CountryLinks) {
            let next = frame
                .complete::<tables::CountryLinks>()
                .ok()
                .map(|rows| rows.to_vec());
            roster |= self.links != next;
            self.links = next;
        }
        roster |= admission;
        if roster {
            dirty.extend(self.holders.keys().cloned());
            self.holders.clear();
            self.slots.clear();
            if let Some((countries, links)) = self.countries.as_ref().zip(self.links.as_ref()) {
                for country in countries {
                    if known_inputs && std::str::from_utf8(&country.tag).is_err() {
                        refused
                            .entry(
                                refusal::TAG.into(),
                            )
                            .or_default()
                            .insert((country.id, country.canonical_slot, country.tag.clone()));
                    }
                }
                for (tag, found) in tagged_countries(countries, links) {
                    let Ok(tag) = std::str::from_utf8(tag) else {
                        continue;
                    };
                    let holder = found.found().map(|(country, slot)| {
                        let index = countries
                            .binary_search_by_key(&country.id, |row| row.id)
                            .expect("idea country index");
                        self.slots.entry(slot).or_default().push(tag.to_owned());
                        (index, slot)
                    });
                    self.holders.insert(tag.to_owned(), holder);
                }
            }
            dirty.extend(self.holders.keys().cloned());
        }
        if changed(TableId::ActiveIdeas) {
            self.idea_rows = frame
                .complete::<tables::ActiveIdeas>()
                .ok()
                .map(|rows| rows.to_vec());
        }
        if changed(TableId::ActiveIdeas) || roster {
            let next = self.idea_rows.as_ref().map(|rows| {
                let mut by_slot: BTreeMap<u32, Vec<_>> = BTreeMap::new();
                for row in rows {
                    match self.links.as_ref().and_then(|links| link_slot(links, row.country)) {
                        Some(slot) if self.slots.contains_key(&slot) => {by_slot.entry(slot).or_default().push(row.clone());},
                        Some(_) => {if known_inputs {refused.entry(refusal::IDEA_HOLDER.into()).or_default().insert((row.country, row.ordinal, row.token.clone()));}},
                        None => {if known_inputs {refused.entry(refusal::IDEA_LINK.into()).or_default().insert((row.country, row.ordinal, row.token.clone()));}},
                    }
                }
                by_slot
            });
            if self.ideas.is_some() != next.is_some() {
                dirty.extend(self.holders.keys().cloned());
            }
            for slot in self.ideas.iter().chain(&next).flat_map(|rows| rows.keys()) {
                if self.ideas.as_ref().and_then(|rows| rows.get(slot))
                    != next.as_ref().and_then(|rows| rows.get(slot))
                {
                    dirty.extend(self.slots.get(slot).into_iter().flatten().cloned());
                }
            }
            self.ideas = next;
        }
        if changed(TableId::HiredAdvisors) {
            self.advisor_rows = frame
                .complete::<tables::HiredAdvisors>()
                .ok()
                .map(|rows| rows.to_vec());
        }
        if changed(TableId::HiredAdvisors) || roster {
            let next = self.advisor_rows.as_ref().map(|rows| {
                let mut by_slot: BTreeMap<u32, Vec<_>> = BTreeMap::new();
                for row in rows {
                    match self.links.as_ref().and_then(|links| link_slot(links, row.country)) {
                        Some(slot) if self.slots.contains_key(&slot) => {by_slot.entry(slot).or_default().push(row.clone());},
                        Some(_) => {if known_inputs {refused.entry(refusal::ADVISOR_HOLDER.into()).or_default().insert((row.country, row.ordinal, row.idea.clone()));}},
                        None => {if known_inputs {refused.entry(refusal::ADVISOR_LINK.into()).or_default().insert((row.country, row.ordinal, row.idea.clone()));}},
                    }
                }
                by_slot
            });
            if self.advisors.is_some() != next.is_some() {
                dirty.extend(self.holders.keys().cloned());
            }
            for slot in self
                .advisors
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.advisors.as_ref().and_then(|rows| rows.get(slot))
                    != next.as_ref().and_then(|rows| rows.get(slot))
                {
                    dirty.extend(self.slots.get(slot).into_iter().flatten().cloned());
                }
            }
            self.advisors = next;
            self.character_users.clear();
            for (slot, advisors) in self.advisors.iter().flat_map(|rows| rows.iter()) {
                for row in advisors {
                    self.character_users
                        .entry(row.character)
                        .or_default()
                        .extend(self.slots.get(slot).into_iter().flatten().cloned());
                }
            }
        }
        if changed(TableId::CountryAppearance) {
            let mut next: BTreeMap<u32, Vec<_>> = BTreeMap::new();
            for row in frame
                .complete::<tables::CountryAppearance>()
                .ok()
                .into_iter()
                .flatten()
            {
                next.entry(row.country).or_default().push(row.clone());
            }
            for id in self.appearances.keys().chain(next.keys()) {
                if self.appearances.get(id) != next.get(id)
                    && let Some(countries) = &self.countries
                    && let Ok(index) = countries.binary_search_by_key(id, |row| row.id)
                {
                    dirty.extend(
                        self.slots
                            .get(&countries[index].canonical_slot)
                            .into_iter()
                            .flatten()
                            .cloned(),
                    );
                }
            }
            self.appearances = next;
        }
        if changed(TableId::Characters) {
            let next: Option<BTreeMap<_, _>> =
                frame.complete::<tables::Characters>().ok().map(|rows| {
                    rows.iter()
                        .map(|row| {
                            (
                                row.id,
                                replay_data::rows::CharacterRow {
                                    id: row.id,
                                    name_token_id: row.name_token_id,
                                    token: row.token.clone(),
                                    template_present: row.template_present,
                                    template_tag: row.template_tag,
                                    template_file: row.template_file.clone(),
                                    template_line: row.template_line,
                                    template_generated: row.template_generated,
                                    ..Default::default()
                                },
                            )
                        })
                        .collect()
                });
            if self.characters.is_some() != next.is_some() {
                dirty.extend(self.character_users.values().flatten().cloned());
            }
            for id in self
                .characters
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.characters.as_ref().and_then(|rows| rows.get(id))
                    != next.as_ref().and_then(|rows| rows.get(id))
                {
                    dirty.extend(self.character_users.get(id).into_iter().flatten().cloned());
                }
            }
            self.characters = next;
        }
        if roster || changed(TableId::HiredAdvisors) || changed(TableId::Characters) {
            self.script_users.clear();
            for (id, tags) in &self.character_users {
                if let Some(row) = self.characters.as_ref().and_then(|rows| rows.get(id)) {
                    let tokens = u32::try_from(row.name_token_id)
                        .ok()
                        .into_iter()
                        .chain((row.template_present != 0).then_some(row.template_tag));
                    for token in tokens {
                        self.script_users
                            .entry(token)
                            .or_default()
                            .extend(tags.iter().cloned());
                    }
                }
            }
        }
        if changed(TableId::LexicalTokens) {
            let next: Option<BTreeMap<_, _>> =
                frame.complete::<tables::LexicalTokens>().ok().map(|rows| {
                    rows.iter()
                        .map(|row| (row.token, row.literal.clone()))
                        .collect()
                });
            if self.lexical.is_some() != next.is_some() {
                dirty.extend(self.character_users.values().flatten().cloned());
            }
            for token in self
                .lexical
                .iter()
                .chain(&next)
                .flat_map(|rows| rows.keys())
            {
                if self.lexical.as_ref().and_then(|rows| rows.get(token))
                    != next.as_ref().and_then(|rows| rows.get(token))
                {
                    dirty.extend(self.script_users.get(token).into_iter().flatten().cloned());
                }
            }
            self.lexical = next;
        }
        if changes.is_none_or(|changes| changes.rows_changed.iter().all(|changed| *changed)) {
            dirty.extend(self.holders.keys().cloned());
        }
        dirty
    }
}
#[cfg(any(test, feature = "test-support"))]
impl Scan {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        let bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        Self {
            previous_hour: bounds.and_then(|(first, _)| first.checked_sub(1)),
            ..Default::default()
        }
    }
    pub fn observe(&mut self, decoded: &replay_frame::frame::DecodedFrame) -> io::Result<()> {
        let changes = self
            .test_changes
            .get_or_insert_with(crate::prepared::frame_changes::FrameChangeTracker::new)
            .observe(decoded, replay_data::format::FormatVersion::CURRENT);
        self.observe_changes(decoded, &changes)
    }
    pub fn observe_changes(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope_phase("ideas:observe");
        // Preserve the production scan's selected-table availability, including
        // reached naming refusals for operands outside its selection.
        static OMITTED: std::sync::LazyLock<replay_frame::frame::SharedView> =
            std::sync::LazyLock::new(|| {
                replay_frame::frame::TableView::Unavailable {
                    reason: "Table not selected for reconstruction".into(),
                }
                .into()
            });
        let mut selected = decoded.clone();
        for table in TableId::ALL {
            if !TABLES.contains(&table) {
                selected.tables[table.index()] = OMITTED.clone();
            }
        }
        self.observe_selected(&selected, None, Some(changes))
    }
    fn observe_selected(
        &mut self,
        decoded: &replay_frame::frame::DecodedFrame,
        requested: Option<&str>,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> io::Result<()> {
        let hour = decoded.hour;
        let previous = self.previous_hour;
        let adjacent = previous.and_then(|at| at.checked_add(1)) == Some(hour);
        if !adjacent {
            self.seal();
        }
        {
            for table in TABLES
                .into_iter()
                .filter(|table| changes.is_none_or(|changes| changes.rows_changed[table.index()]))
            {
                self.work.row_visits += match &*decoded.tables[table.index()] {
                    replay_frame::frame::TableView::Complete(rows)
                    | replay_frame::frame::TableView::Truncated { rows, .. } => rows.len(),
                    replay_frame::frame::TableView::Unavailable { .. }
                    | replay_frame::frame::TableView::NotRecorded { .. } => 0,
                };
            }
        }
        // A restored scan has no holders of the hour before, so the tags it still
        // holds open are visited as those holders would be. Otherwise a fresh
        // join holds none open.
        let fresh = self.native.keys.is_none();
        let mut dirty = self.native.refresh(decoded, changes, &mut self.refused);
        if fresh {
            dirty.extend(self.active.iter().cloned());
            dirty.extend(self.unresolved.iter().cloned());
        }
        if !adjacent {
            dirty.extend(self.native.holders.keys().cloned());
        }
        if let Some(previous) = previous
            && previous.checked_add(1).is_some_and(|next| next < hour)
        {
            mark(&mut self.unobserved, previous + 1, hour - 1);
        }
        let known = self.native.countries.is_some()
            && self.native.links.is_some()
            && self.native.ideas.is_some()
            && self.native.advisors.is_some();
        if !known {
            self.seal();
            self.active.clear();
            self.unresolved.clear();
            self.dated.clear();
            self.previous_hour = Some(hour);
            mark(&mut self.unobserved, hour, hour);
            return Ok(());
        }
        self.previous_hour = Some(hour);
        self.observed = true;
        if let Some(tag) = requested {
            self.states.entry(tag.to_owned()).or_default();
        }
        for tag in dirty {
            if requested.is_some_and(|wanted| wanted != tag) {
                continue;
            }
            let state = self.states.entry(tag.clone()).or_default();
            if adjacent
                && self.active.contains(&tag)
                && let Some(previous) = previous
            {
                state.advance(previous);
            }
            if self.unresolved.remove(&tag)
                && let Some(previous) = previous
            {
                let first = state
                    .result
                    .unobserved
                    .last()
                    .map_or(previous, |span| span.first_hour);
                mark(&mut state.result.unobserved, first, previous);
            }
            let holder = self.native.holders.get(&tag);
            let Some(&(index, slot)) = holder.and_then(Option::as_ref) else {
                self.active.remove(&tag);
                self.dated.remove(&tag);
                if holder.is_some() {
                    self.unresolved.insert(tag.clone());
                    mark(&mut state.result.unobserved, hour, hour);
                }
                continue;
            };
            let country = &self.native.countries.as_ref().expect("idea countries")[index];
            let ideas: Option<Vec<_>> = self
                .native
                .ideas
                .as_ref()
                .and_then(|rows| rows.get(&slot))
                .map(|rows| rows.iter().collect());
            let advisors: Option<Vec<_>> = self
                .native
                .advisors
                .as_ref()
                .and_then(|rows| rows.get(&slot))
                .map(|rows| rows.iter().collect());
            state.observe(
                decoded,
                CountryRead {
                    country,
                    slot,
                    ideas: ideas.as_ref(),
                    advisors: advisors.as_ref(),
                    appearance: self.native.appearances.get(&country.id).and_then(|rows| {
                        if requested.is_some() {
                            rows.first()
                        } else {
                            rows.last()
                        }
                    }),
                    topic_appearance: self
                        .native
                        .appearances
                        .get(&country.id)
                        .and_then(|rows| rows.first()),
                },
                requested.is_none(),
            );
            if state.dated_templates.is_empty() {
                self.dated.remove(&tag);
            } else {
                self.dated.insert(tag.clone());
            }
            self.active.insert(tag);
            self.work.emitted += 1;
        }
        // Only these actual recorded-state failures have hour-dependent text.
        for tag in &self.dated {
            if let Some(state) = self.states.get_mut(tag) {
                state.date_template_refusals(hour);
            }
        }
        Ok(())
    }

    fn seal(&mut self) {
        if let Some(hour) = self.previous_hour {
            for tag in &self.active {
                if let Some(state) = self.states.get_mut(tag) {
                    state.advance(hour);
                }
            }
            for tag in &self.unresolved {
                if let Some(state) = self.states.get_mut(tag) {
                    let first = state
                        .result
                        .unobserved
                        .last()
                        .map_or(hour, |span| span.first_hour);
                    mark(&mut state.result.unobserved, first, hour);
                }
            }
        }
    }

    fn result(
        &self,
        replay: &Replay,
        branch: u64,
        requested: Option<&str>,
    ) -> io::Result<ScannedIdeaBranch> {
        let bounds = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .map(|start| branch_bounds(replay, start));
        let previous_hour = self.previous_hour;
        let mut unobserved = self.unobserved.clone();
        let unreadable = self.unreadable;
        let observed = self.observed;
        let mut states: BTreeMap<_, _> = self
            .states
            .iter()
            .filter(|(tag, _)| requested.is_none_or(|wanted| wanted == tag.as_str()))
            .map(|(tag, state)| (tag.clone(), state.clone()))
            .collect();
        if let Some(hour) = previous_hour {
            for (tag, state) in &mut states {
                if self.active.contains(tag) {
                    state.advance(hour);
                }
                if self.unresolved.contains(tag) {
                    let first = state
                        .result
                        .unobserved
                        .last()
                        .map_or(hour, |span| span.first_hour);
                    mark(&mut state.result.unobserved, first, hour);
                }
            }
        }
        let mut refused = if requested.is_none() {
            self.refused.clone()
        } else {
            RefusedRows::new()
        };
        for state in states.values() {
            for (reason, rows) in &state.refused {
                refused
                    .entry(reason.clone())
                    .or_default()
                    .extend(rows.iter().cloned());
            }
        }
        if let (Some((first, last)), previous) = (bounds, previous_hour) {
            let from = previous.map_or(first, |previous| previous.saturating_add(1));
            if from <= last {
                mark(&mut unobserved, from, last);
            }
        }
        let warnings = if unreadable > 0 {
            vec![format!(
                "Part of this branch could not be read ({unreadable} blocks or reads); idea history is missing there."
            )]
        } else {
            Vec::new()
        };
        let mut result = ScannedIdeaBranch {
            observed,
            warnings: if requested.is_some() {
                warnings.clone()
            } else {
                Vec::new()
            },
            absent: ScannedIdeas {
                unobserved: unobserved.clone(),
                warnings: warnings.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        if requested.is_none() && unreadable > 0 {
            result.refusals.push((
                "Part of this branch could not be read; idea history is missing there.".into(),
                unreadable as usize,
            ));
        }
        let mut tokenless = 0_usize;
        for (tag, mut state) in states {
            let mut spans = std::mem::take(&mut state.result.unobserved);
            spans.extend(unobserved.iter().cloned());
            spans.sort_by_key(|span| span.first_hour);
            for span in spans {
                mark(
                    &mut state.result.unobserved,
                    span.first_hour,
                    span.last_hour,
                );
            }
            if !state.tokenless.is_empty() {
                if requested.is_some() {
                    state.result.warnings.push(
                    refusal::TOKENLESS_SEATS.into(),
                );
                } else {
                    tokenless = tokenless
                        .checked_add(state.tokenless.len())
                        .ok_or_else(|| {
                            io::Error::other("Idea unavailable appointment count exceeds usize")
                        })?;
                }
            }
            if requested.is_some() {
                state.result.warnings.extend(warnings.iter().cloned());
            }
            result.countries.insert(tag, state.result);
        }
        if tokenless > 0 {
            result.refusals.push((
                refusal::TOKENLESS.into(),
                tokenless,
            ));
        }
        result.refusals.extend(
            refused
                .into_iter()
                .map(|(reason, rows)| (reason, rows.len())),
        );
        Ok(result)
    }
    pub fn country(&self, replay: &Replay, branch: u64, tag: &str) -> io::Result<ScannedIdeas> {
        Ok(self.result(replay, branch, Some(tag))?.into_country(tag))
    }
}

#[cfg(any(test, feature = "test-support"))]
fn scan_selected(
    replay: &Replay,
    branch: u64,
    requested: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeaBranch> {
    let mut scan = Scan::new(replay, branch);
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &TABLES,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                crate::metadata::check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&TABLES);
                let Ok(frames) = frames else {
                    scan.unreadable += 1;
                    continue;
                };
                for frame in frames {
                    crate::metadata::check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        scan.unreadable += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        scan.unreadable += 1;
                        break;
                    }
                    scan.observe_selected(decoded, requested, None)?;
                }
            }
            Ok(())
        },
    )?;
    scan.result(replay, branch, requested)
}

fn line(modifier: &Modifier, hidden: bool) -> EffectLine {
    EffectLine {
        key: modifier.name.clone(),
        fixed5: modifier.value.to_string(),
        hidden,
    }
}

/// A lookup's answer, and the gap's text when content cannot say.
fn known<'a, T>(
    found: Result<Option<&'a T>, &Gap>,
    subject: &str,
    findings: &mut Vec<crate::findings::Finding>,
) -> (Option<&'a T>, Option<String>) {
    match found {
        Ok(found) => (found, None),
        Err(gap) => {
            let reason = gap.to_string();
            findings.push(crate::findings::Finding::new(
                crate::findings::Kind::Refused,
                subject,
                &reason,
            ));
            (None, Some(reason))
        }
    }
}

/// The content an idea is joined with; a catalogue's politics content, or test data.
pub struct Joined<'a> {
    pub ideas: &'a installed_content_core::ideas::IdeaContent,
    pub characters: &'a installed_content_core::characters::Characters,
    pub traits: &'a installed_content_core::leader_traits::LeaderTraits,
    pub localise: &'a dyn Fn(&str, View) -> Option<String>,
}

impl<'a> Joined<'a> {
    pub fn of(
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

fn targeted(groups: &[installed_content_core::modifiers::TargetedModifier]) -> Vec<EffectGroup> {
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

fn equipment(bonuses: &[installed_content_core::modifiers::EquipmentBonus]) -> Vec<EffectGroup> {
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

/// A modifier's lines: one per stored modifier, a hidden one marked.
/// Added-modifier children are not listed; descriptions read the values
/// and tooltip table only.
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
/// disagree on ([`installed_content_core::characters::Characters::held_advisor`]), or why
/// it is not known.
pub fn prepare_info(
    content: &Joined,
    token: &str,
    recorded_seat: Option<&str>,
    built: Result<&BuiltTemplate, &str>,
    offline_holder: bool,
) -> (IdeaInfo, Vec<crate::findings::Finding>) {
    use crate::findings::{Finding, Kind};
    let mut findings = Vec::new();
    let subject = format!("idea {token}");
    let holder_missing = built.is_err();
    let tags = &content.ideas.tags;
    let mut unknown = Vec::new();
    let (idea, gap) = known(content.ideas.idea(token), &subject, &mut findings);
    unknown.extend(gap);
    let authored_role = content
        .characters
        .held_advisor(token, built)
        .inspect_err(|reason| {
            unknown.push(reason.clone());
            findings.push(super::politics_content::advisor_finding(
                content.characters,
                token,
                &subject,
                reason,
                offline_holder,
                holder_missing,
            ));
        })
        .ok()
        .flatten();
    // The idea's group resolves to its category by the category's name
    // (a national spirit's) or by a slot it holds.
    let (group_category, gap) = match idea {
        Some(idea) => known(tags.resolve(&idea.group), &subject, &mut findings),
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
            let (category, gap) = known(tags.resolve(slot), &subject, &mut findings);
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
            let trait_subject = format!("idea {token} trait {trait_token}");
            let (found, unknown) = known(
                content.traits.get(trait_token),
                &trait_subject,
                &mut findings,
            );
            if found.is_none() && unknown.is_none() {
                findings.push(Finding::new(
                    Kind::Fallback,
                    &trait_subject,
                    "trait definition is missing; its name and effects are unavailable",
                ));
            }
            TraitEffects {
                token: trait_token.clone(),
                known: found.is_some(),
                unknown,
                // Trait names localise the selected trait's stored name, whose
                // spelling can differ from the reference.
                name: found.and_then(|found| (content.localise)(&found.token, View::Plain)),
                // The trait's command fields are not lines of the trait: its formatter
                // never reads them. An advisor shows their sum with its own cost.
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
    // An idea's effects, else its advisor role's modifier and research_bonus.
    let modifiers = idea
        .map(|idea| &idea.modifiers)
        .or_else(|| authored_role.map(|role| &role.modifiers));
    let research = idea
        .map(|idea| &idea.research_bonuses)
        .or_else(|| authored_role.map(|role| &role.research_bonuses));
    let effects_known = idea.is_some() || authored_role.is_some();
    if category.is_none() && unknown.is_empty() {
        findings.push(Finding::new(
            Kind::Refused,
            &subject,
            "idea is in no category the content defines",
        ));
    }
    (
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
        },
        findings,
    )
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
                    // Slot names are plain.
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
                        // Seat names are plain.
                        name: (content.localise)(seat, View::Plain),
                        character: true,
                        seats: 1,
                        laws: Vec::new(),
                    }),
                }
            }
            CategoryInfo {
                token: category.name.clone(),
                // Category names are plain.
                name: (content.localise)(&category.name, View::Plain),
                kind: category.kind.clone(),
                hidden: category.hidden,
                politics_tab: category.politics_tab,
                slots,
            }
        })
        .collect()
}

/// Tooltip dependencies shared by the recorded reply, feed and offline check.
pub fn effect_keys<'a>(
    ideas: impl IntoIterator<Item = &'a IdeaInfo>,
) -> (BTreeSet<String>, BTreeSet<String>) {
    // Boolean modifier values use YES/NO localisation.
    let mut modifier_keys = BTreeSet::new();
    let mut text_keys: BTreeSet<String> = ["YES", "NO"].map(str::to_owned).into();
    for idea in ideas {
        {
            let mut effects =
                |lines: &[EffectLine], targeted: &[EffectGroup], tooltips: &[String]| {
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
            effects(&idea.effects, &idea.targeted, &idea.tooltips);
            for found in &idea.traits {
                effects(&found.lines, &found.targeted, &found.tooltips);
            }
        }
        // Research bonuses use the research-tooltip content key.
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
    (modifier_keys, text_keys)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct BranchCache {
    entries: VecDeque<(u64, Arc<ScannedIdeaBranch>)>,
}

#[cfg(any(test, feature = "test-support"))]
pub fn scanned_branch(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedIdeaBranch>> {
    let state = source.state::<BranchCache>();
    {
        let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(index) = cache.entries.iter().position(|(key, _)| *key == branch) {
            let entry = cache.entries.remove(index).expect("cached branch exists");
            let scanned = Arc::clone(&entry.1);
            cache.entries.push_back(entry);
            return Ok(scanned);
        }
    }
    let scanned = Arc::new(scan_all(&source.replay(), branch, cancelled)?);
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    cache.entries.retain(|(key, _)| *key != branch);
    cache.entries.push_back((branch, Arc::clone(&scanned)));
    while cache.entries.len() > CACHED_BRANCHES {
        cache.entries.pop_front();
    }
    Ok(scanned)
}

/// What the event feed reads of the ideas family, from the row recording's scan: the parity
/// oracle of the family's publication ([`Feed`]).
#[cfg(any(test, feature = "test-support"))]
pub fn row_feed(replay: &Replay, branch: u64, cancelled: &AtomicBool) -> io::Result<Feed> {
    let scanned = scan_all(replay, branch, cancelled)?;
    let mut additions: Vec<Addition> = scanned
        .countries
        .iter()
        .flat_map(|(tag, country)| {
            named_additions(country).map(move |(span, naming)| Addition {
                tag: tag.clone(),
                span: IdeaSpan {
                    last_hour: span.first_hour,
                    ..span.clone()
                },
                naming: naming.cloned(),
            })
        })
        .collect();
    // Stable: each hour's additions stay in tag order, then token order.
    additions.sort_by_key(|addition| addition.span.first_hour);
    let (tokenless, mut refusals): (Vec<_>, Vec<_>) = scanned
        .refusals
        .iter()
        .cloned()
        .partition(|(reason, _)| reason == refusal::TOKENLESS);
    refusals.sort();
    refusals.extend(tokenless);
    Ok(Feed {
        additions,
        observed: scanned.observed,
        templates: scanned
            .countries
            .iter()
            .map(|(tag, country)| (tag.clone(), country.templates.clone()))
            .collect(),
        refusals,
        gap_additions: scanned.countries.values().map(unavailable_additions).sum(),
        definitions: super::idea_format::recorded(replay, branch, cancelled)?,
    })
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedIdeas>)>,
}

impl Product for IdeaHistory {
    const KEY: &'static str = "idea-history";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static crate::prepared::served::Plan> = Some(&served::PLAN);
    type Params = CountryParams;
    type Reply = IdeaHistoryReply;

    fn types() -> Vec<String> {
        vec![
            IdeaSpan::decl(),
            HeldIdeas::decl(),
            IdeaHistoryQuery::decl(),
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

    /// The topic scan of the row recording: the parity oracle of the prepared reply.
    #[cfg(any(test, feature = "test-support"))]
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
            let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .position(|(cached, _)| *cached == key)
                .map(|index| {
                    let entry = cache.entries.remove(index).expect("cached country exists");
                    let scanned = Arc::clone(&entry.1);
                    cache.entries.push_back(entry);
                    scanned
                })
        };
        // Keep the selected-country scan outside the cache lock.
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

/// The reply to `params` from the row recording's scan of the country, formatted with the
/// branch's last recorded modifier definitions.
#[cfg(any(test, feature = "test-support"))]
pub fn reply(
    content: &PoliticsContent,
    replay: &Replay,
    params: &CountryParams,
    scanned: &ScannedIdeas,
    cancelled: &AtomicBool,
) -> io::Result<IdeaHistoryReply> {
    let recorded = super::idea_format::recorded(replay, params.branch(), cancelled)?;
    present(content, params, scanned, recorded.as_ref(), cancelled)
}

/// The reply to `params` from `scanned`, the country's history, joined with `content` and
/// formatted with `recorded`, the recorded modifier definitions by name.
pub fn present(
    content: &PoliticsContent,
    params: &CountryParams,
    scanned: &ScannedIdeas,
    recorded: Option<&BTreeMap<String, replay_data::rows::ModifierDefinitionRow>>,
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
        let (found, _) = prepare_info(
            &joined,
            token,
            latest.and_then(|span| span.slot.as_deref()),
            built,
            false,
        );
        if found.category.is_none() {
            unplaced.push(token.to_owned());
        }
        // A holder's artwork: the content's for its script token, named as recorded.
        let holder_art = |script: Option<&str>, saved: &str| {
            let (mut art, mut problem) = content.held_idea_artwork(token, script, saved, country);
            // Without a script token the content cannot say whose portrait
            // this is; the idea's authored advisor may be someone else.
            if script.is_none() {
                art.icon = None;
                problem = Some(format!(
                    "{token}: artwork is unknown because the holder has no recorded script token; a placeholder is shown"
                ));
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
    let (modifier_keys, text_keys) = effect_keys(&ideas);
    let (modifiers, format_warnings) =
        super::idea_format::modifier_formats_from_recorded(recorded, &modifier_keys, content);
    warnings.extend(format_warnings);
    let texts = super::idea_format::texts(content, &text_keys);
    // Names left out for a localisation refusal, by reason and key.
    warnings.extend(content.catalogue.refusal_lines());
    Ok(IdeaHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        spans: scanned.spans.clone(),
        approximate: Vec::new(),
        held: None,
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

pub mod served;

#[cfg(test)]
mod tests;
