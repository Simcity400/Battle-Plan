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
    politics_content::{IdeaCountry, PoliticsContent, politics_content},
};
use crate::{
    Artwork, Source,
    catalogue::View,
    metadata::{
        CountryKeys, TaggedCountry, branch_bounds, link_slot, tagged_countries, tagged_country,
        validate_campaign,
    },
    products::country_groups::HourSpan,
};
use installed_content::{
    characters::BuiltTemplate,
    coverage::Gap,
    modifiers::{Modifier, Modifiers},
};
use replay::{
    container::Replay,
    frame::{FrameDecoder, ViewIdentity},
    rows::{TableId, tables},
};
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

const CACHED_BRANCHES: usize = 2;
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
    /// For each identity start, the country used for idea naming.
    pub(crate) identity_naming: Vec<NamingCountry>,
    /// Each recorded advisor holder's template, by character id,
    /// or why the recording does not give it.
    pub(crate) templates: BTreeMap<u32, Result<BuiltTemplate, String>>,
    pub warnings: Vec<String>,
    /// Feed-only naming at each span's first hour, in the same order as `spans`.
    pub(crate) span_naming: Vec<NamingCountry>,
    /// Starts of the same identity after unobserved hours, where additions
    /// cannot be separated from holdings that continued through the gap.
    pub(crate) gap_starts: Vec<i32>,
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
pub(crate) fn named_additions(
    scanned: &ScannedIdeas,
) -> impl Iterator<Item = (&IdeaSpan, Option<&NamingCountry>)> {
    addition_indices(scanned)
        .map(move |index| (&scanned.spans[index], scanned.span_naming.get(index)))
}

fn addition_indices(scanned: &ScannedIdeas) -> impl Iterator<Item = usize> {
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
pub(crate) fn unavailable_additions(scanned: &ScannedIdeas) -> usize {
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

/// The feed's held-idea spans and row-hour naming. The topic product retains
/// only its selected country's history, through the same observer.
#[derive(Clone, Debug, Default)]
pub(crate) struct ScannedIdeaBranch {
    pub countries: BTreeMap<String, ScannedIdeas>,
    pub observed: bool,
    pub warnings: Vec<String>,
    /// Feed-only refused observations, each with its named reason and count.
    pub refusals: Vec<(String, usize)>,
    absent: ScannedIdeas,
}

impl ScannedIdeaBranch {
    #[cfg(test)]
    pub fn country(&self, tag: &str) -> &ScannedIdeas {
        self.countries.get(tag).unwrap_or(&self.absent)
    }
}

type RefusedRows = BTreeMap<&'static str, BTreeSet<(u32, u32, Vec<u8>)>>;

#[derive(Default)]
struct CountryScan {
    result: ScannedIdeas,
    open: BTreeMap<String, usize>,
    last: Option<((u32, u32), i32)>,
    tokenless: BTreeSet<(u32, String)>,
    unresolved_now: bool,
}

const HOLDING_VIEWS: [TableId; 6] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ActiveIdeas,
    TableId::HiredAdvisors,
    TableId::Characters,
    TableId::LexicalTokens,
];

struct CountryRead<'a> {
    country: &'a replay::rows::CountryRow,
    slot: u32,
    ideas: Option<&'a Vec<&'a replay::rows::ActiveIdeaRow>>,
    advisors: Option<&'a Vec<&'a replay::rows::HiredAdvisorRow>>,
    appearance: Option<&'a replay::rows::CountryAppearanceRow>,
}

impl CountryScan {
    fn extend(&mut self, hour: i32) {
        let Some((_, previous)) = self.last.as_mut() else {
            return;
        };
        if previous.checked_add(1) != Some(hour) {
            return;
        }
        for &index in self.open.values() {
            self.result.spans[index].last_hour = hour;
        }
        *previous = hour;
    }

    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        read: CountryRead<'_>,
        feed: bool,
        refused: &mut RefusedRows,
    ) {
        let Self {
            result,
            open,
            last,
            tokenless,
            ..
        } = self;
        let CountryRead {
            country,
            slot,
            ideas,
            advisors,
            appearance,
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
            if !feed {
                result
                    .identity_naming
                    .push(NamingCountry::read_with_appearance(
                        decoded, country, appearance,
                    ));
            }
        }
        // Each appointed advisor's idea with its recorded seat and character.
        let characters = decoded.complete::<tables::Characters>().ok();
        let lexical = decoded.complete::<tables::LexicalTokens>().ok();
        let mut roles = BTreeMap::new();
        for advisor in advisors.into_iter().flatten() {
            let Ok(seat) = std::str::from_utf8(&advisor.slot) else {
                refused
                    .entry("HiredAdvisors seat is not UTF-8; appointment cannot be identified.")
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
                    .entry("Held idea token is not UTF-8; holding cannot be identified.")
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
}

/// Scans and retains only the requested country's topic history.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeas> {
    let mut scanned = scan_selected(replay, branch, Some(tag), cancelled)?;
    let mut result = scanned.countries.remove(tag).unwrap_or(scanned.absent);
    result.warnings.extend(
        scanned
            .refusals
            .into_iter()
            .map(|(reason, count)| format!("{reason} (count: {count})")),
    );
    Ok(result)
}

pub(crate) fn scan_all(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeaBranch> {
    scan_selected(replay, branch, None, cancelled)
}

fn scan_selected(
    replay: &Replay,
    branch: u64,
    requested: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<ScannedIdeaBranch> {
    let selected = [
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
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Idea request cancelled");
    let mut states: BTreeMap<String, CountryScan> = BTreeMap::new();
    let mut unobserved = Vec::new();
    let mut unreadable = 0_u32;
    let mut observed = false;
    let mut refused = RefusedRows::new();
    let mut previous_frame: Option<(i32, [ViewIdentity; 6], CountryKeys)> = None;
    let bounds = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .map(|start| branch_bounds(replay, start));
    let mut previous_hour = bounds.and_then(|(first, _)| first.checked_sub(1));
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
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
                        mark(&mut unobserved, previous + 1, hour - 1);
                    }
                    previous_hour = Some(hour);
                    let (Ok(countries), Ok(links), Ok(ideas), Ok(advisors)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::ActiveIdeas>(),
                        decoded.complete::<tables::HiredAdvisors>(),
                    ) else {
                        mark(&mut unobserved, hour, hour);
                        continue;
                    };
                    observed = true;
                    if let Some(tag) = requested {
                        if !states.contains_key(tag) {
                            states.insert(tag.to_owned(), CountryScan::default());
                        }
                        let state = states.get_mut(tag).expect("selected country inserted");
                        let (country, slot) = match tagged_country(countries, links, tag) {
                            TaggedCountry::Found(country, slot) => (country, slot),
                            TaggedCountry::Absent => continue,
                            TaggedCountry::Unresolved => {
                                mark(&mut state.result.unobserved, hour, hour);
                                continue;
                            }
                        };
                        let owned_ideas: Vec<_> = ideas
                            .iter()
                            .filter(|row| link_slot(links, row.country) == Some(slot))
                            .collect();
                        let owned_advisors: Vec<_> = advisors
                            .iter()
                            .filter(|row| link_slot(links, row.country) == Some(slot))
                            .collect();
                        let appearance = decoded
                            .complete::<tables::CountryAppearance>()
                            .ok()
                            .and_then(|rows| rows.iter().find(|row| row.country == country.id));
                        state.observe(
                            decoded,
                            CountryRead {
                                country,
                                slot,
                                ideas: Some(&owned_ideas),
                                advisors: Some(&owned_advisors),
                                appearance,
                            },
                            false,
                            &mut refused,
                        );
                        continue;
                    }
                    // Hourly balances change Countries; holders read only its keys.
                    if previous_frame
                        .as_ref()
                        .is_some_and(|(previous, views, keys)| {
                            previous.checked_add(1) == Some(hour)
                                && views.iter().zip(HOLDING_VIEWS).all(|(view, table)| {
                                    view.is(&decoded.tables[table.index()])
                                        || table == TableId::Countries && keys.same(countries)
                                })
                        })
                    {
                        for state in states.values_mut() {
                            if state.unresolved_now {
                                mark(&mut state.result.unobserved, hour, hour);
                            } else {
                                state.extend(hour);
                            }
                        }
                        previous_frame.as_mut().expect("unchanged frame exists").0 = hour;
                        continue;
                    }
                    previous_frame = Some((
                        hour,
                        HOLDING_VIEWS.map(|table| decoded.tables[table.index()].identity()),
                        CountryKeys::read(countries),
                    ));
                    for country in countries {
                        if std::str::from_utf8(&country.tag).is_err() {
                            refused
                                .entry(
                                    "Countries tag is not UTF-8; idea holder cannot be identified.",
                                )
                                .or_default()
                                .insert((country.id, country.canonical_slot, country.tag.clone()));
                        }
                    }
                    let found = tagged_countries(countries, links);
                    let holders: BTreeSet<_> = found
                        .iter()
                        .filter_map(|(tag, holder)| {
                            std::str::from_utf8(tag)
                                .ok()
                                .and_then(|_| holder.found().map(|(_, slot)| slot))
                        })
                        .collect();
                    let mut ideas_by_slot: BTreeMap<u32, Vec<_>> = BTreeMap::new();
                    for row in ideas {
                        if let Some(slot) = link_slot(links, row.country) {
                            if holders.contains(&slot) {
                                ideas_by_slot.entry(slot).or_default().push(row);
                            } else {
                                refused.entry("ActiveIdeas owner has no resolved Countries holder; holding cannot be attributed.")
                                .or_default().insert((row.country, row.ordinal, row.token.clone()));
                            }
                        } else {
                            refused.entry("ActiveIdeas owner is unresolved through CountryLinks; holding cannot be attributed.")
                            .or_default().insert((row.country, row.ordinal, row.token.clone()));
                        }
                    }
                    let mut advisors_by_slot: BTreeMap<u32, Vec<_>> = BTreeMap::new();
                    for row in advisors {
                        if let Some(slot) = link_slot(links, row.country) {
                            if holders.contains(&slot) {
                                advisors_by_slot.entry(slot).or_default().push(row);
                            } else {
                                refused.entry("HiredAdvisors owner has no resolved Countries holder; appointment cannot be attributed.")
                                .or_default().insert((row.country, row.ordinal, row.idea.clone()));
                            }
                        } else {
                            refused.entry("HiredAdvisors owner is unresolved through CountryLinks; appointment cannot be attributed.")
                            .or_default().insert((row.country, row.ordinal, row.idea.clone()));
                        }
                    }
                    let appearances: BTreeMap<_, _> = decoded
                        .complete::<tables::CountryAppearance>()
                        .ok()
                        .into_iter()
                        .flatten()
                        .map(|row| (row.country, row))
                        .collect();
                    for state in states.values_mut() {
                        state.unresolved_now = false;
                    }
                    for (tag, found) in found {
                        let Ok(tag) = std::str::from_utf8(tag) else {
                            continue;
                        };
                        if !states.contains_key(tag) {
                            states.insert(tag.to_owned(), CountryScan::default());
                        }
                        let state = states.get_mut(tag).expect("feed country inserted");
                        let (country, slot) = match found {
                            TaggedCountry::Found(country, slot) => (country, slot),
                            TaggedCountry::Absent => continue,
                            TaggedCountry::Unresolved => {
                                state.unresolved_now = true;
                                mark(&mut state.result.unobserved, hour, hour);
                                continue;
                            }
                        };
                        state.observe(
                            decoded,
                            CountryRead {
                                country,
                                slot,
                                ideas: ideas_by_slot.get(&slot),
                                advisors: advisors_by_slot.get(&slot),
                                appearance: appearances.get(&country.id).copied(),
                            },
                            true,
                            &mut refused,
                        );
                    }
                }
            }
            Ok(())
        },
    )?;
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
                    "Some appointed advisors have no recorded idea, so their seats show as empty."
                        .into(),
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
            "Appointed advisors have no recorded idea token, so cannot be named.".into(),
            tokenless,
        ));
    }
    result.refusals.extend(
        refused
            .into_iter()
            .map(|(reason, rows)| (reason.to_owned(), rows.len())),
    );
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
fn known<'a, T>(
    found: Result<Option<&'a T>, &Gap>,
    subject: &str,
    findings: &mut Vec<crate::content_check::Finding>,
) -> (Option<&'a T>, Option<String>) {
    match found {
        Ok(found) => (found, None),
        Err(gap) => {
            let reason = gap.to_string();
            findings.push(crate::content_check::Finding::new(
                crate::content_check::Kind::Refused,
                subject,
                &reason,
            ));
            (None, Some(reason))
        }
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
/// disagree on ([`installed_content::characters::Characters::held_advisor`]), or why
/// it is not known.
pub(crate) fn prepare_info(
    content: &Joined,
    token: &str,
    recorded_seat: Option<&str>,
    built: Result<&BuiltTemplate, &str>,
    offline_holder: bool,
) -> (IdeaInfo, Vec<crate::content_check::Finding>) {
    use crate::content_check::{Finding, Kind};
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
pub(crate) fn effect_keys<'a>(
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

#[derive(Default)]
struct BranchCache {
    entries: VecDeque<(u64, Arc<ScannedIdeaBranch>)>,
}

pub(crate) fn scanned_branch(
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
