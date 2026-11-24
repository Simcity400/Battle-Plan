
use dashboard_base::products::Country;
use dashboard_base::products::{CountryParams, LoadContext, Product, check_reply_budget, idea_format::{self, LocalisedText, ModifierFormat}, politics_content::{PoliticsContent, politics_content}};
use dashboard_core::dto::Artwork;
use dashboard_base::Source;
use dashboard_text::metadata::check_cancel;
use dashboard_base::span::HourSpan;
use dashboard_query::{Query, cost, identity::Holder};
use dashboard_text::{
    catalogue::View,
    identity::{NamingCountry, built_template},
    idea_names::IdeaCountry,
};
use content_reader_key_names::characters::BuiltTemplate;
use content_reader_parser::coverage::Gap;
use content_reader_script::modifiers::{Modifier, Modifiers};
use replay_lanes::lanes::{self, Cell, History, Keys};
use replay_data::rows::{CharacterRow, LexicalTokenRow, ModifierDefinitionRow, TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeaSpan {
    pub token: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub slot: Option<String>,
    pub character: Option<IdeaCharacter>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeaCharacter {
    #[doc = " The recorded character id."]
    pub id: u32,
    #[doc = " The script token (resolved through LexicalTokens); empty when unresolved."]
    pub token: String,
    #[doc = " The saved name as recorded (a localisation key or a literal); a rename changes it, so it"]
    #[doc = " comes before the authored name."]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EffectLine {
    #[doc = " The modifier's case-folded key; formats are keyed the same way."]
    pub key: String,
    #[doc = " The stored fixed5 value as a decimal string."]
    pub fixed5: String,
    #[doc = " Named in hidden_modifier: applied, never listed."]
    pub hidden: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EffectGroup {
    pub target: Option<String>,
    #[doc = " An equipment bonus that applies at once to existing equipment."]
    pub instant: bool,
    pub lines: Vec<EffectLine>,
    #[doc = " A targeted modifier's custom tooltip keys in stored order; none for an equipment bonus."]
    pub tooltips: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct TraitEffects {
    pub token: String,
    #[doc = " Content defines the trait. When it does not, its effects are unknown and the lists below"]
    #[doc = " are empty for that reason, not because it has none."]
    pub known: bool,
    #[doc = " Why content cannot say whether it defines the trait; None when it can."]
    pub unknown: Option<String>,
    pub name: Option<String>,
    pub lines: Vec<EffectLine>,
    pub targeted: Vec<EffectGroup>,
    pub equipment: Vec<EffectGroup>,
    pub tooltips: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeaInfo {
    pub token: String,
    #[doc = " The idea category; None when neither content nor the recording places it."]
    pub category: Option<String>,
    #[doc = " The slot or character seat it fills; None for category-wide ideas such as national spirits."]
    pub slot: Option<String>,
    pub law: bool,
    pub designer: bool,
    pub effects: Vec<EffectLine>,
    pub targeted: Vec<EffectGroup>,
    pub equipment: Vec<EffectGroup>,
    pub research: Vec<EffectLine>,
    pub traits: Vec<TraitEffects>,
    #[doc = " Custom tooltip localisation keys the idea shows instead of values."]
    pub tooltips: Vec<String>,
    #[doc = " Content defines the idea or its advisor role, so the lists above are its effects."]
    pub effects_known: bool,
    #[doc = " Why content cannot say what the idea is; None when it can."]
    pub unknown: Option<String>,
    #[doc = " Why the idea's artwork has no name (a localisation key missing or refused, or what the"]
    #[doc = " hour cannot say about the naming country); None when it is named."]
    pub name_missing: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct SlotInfo {
    pub token: String,
    pub name: Option<String>,
    #[doc = " A character seat (advisors) rather than an idea slot (laws, designers)."]
    pub character: bool,
    #[doc = " How many seats of this slot the category has."]
    pub seats: u32,
    #[doc = " For a law slot: its laws in content order, so a law's escalation is its place here."]
    pub laws: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub token: String,
    pub name: Option<String>,
    #[doc = " `type`, such as `national_spirit`."]
    pub kind: Option<String>,
    pub hidden: bool,
    pub politics_tab: bool,
    pub slots: Vec<SlotInfo>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HolderArtwork {
    pub token: String,
    #[doc = " The recorded character id, as on the span."]
    pub character: u32,
    #[doc = " The script token and saved name, as on the span."]
    pub character_token: String,
    pub name: String,
    pub artwork: Artwork,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IdeaHistoryReply {
    pub branch_id: String,
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    pub spans: Vec<IdeaSpan>,
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<Artwork>,
    #[doc = " Each recorded holder of an advisor idea with that character's own name and portrait."]
    pub holders: Vec<HolderArtwork>,
    #[doc = " Every category in content order, so widgets list mod categories too."]
    pub categories: Vec<CategoryInfo>,
    #[doc = " Hours the country's ideas were not read: a span ending or starting next to one is a gap,"]
    #[doc = " not a change."]
    pub unobserved: Vec<HourSpan>,
    #[doc = " Hours the country's recorded identity starts: its first hour read, after an hour not read,"]
    #[doc = " or as another holder. The ideas held then are that identity's initial state, not changes."]
    pub identity_starts: Vec<i32>,
    #[doc = " How the game shows each modifier the effects above use; a key missing here is shown as"]
    #[doc = " authored, unformatted."]
    pub modifiers: Vec<ModifierFormat>,
    #[doc = " The texts the tooltips need: custom tooltips, research categories, equipment names, and"]
    #[doc = " `YES` / `NO`."]
    pub texts: Vec<LocalisedText>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IdeaHistoryReply);

pub(crate) const TOKENLESS_SEATS: &str = "Some appointed advisors have no recorded idea, so their seats show as empty.";
pub(crate) const SEAT: &str = "hired advisors seat is not UTF-8; appointment cannot be identified.";
pub(crate) const TOKEN: &str = "Held idea token is not UTF-8; holding cannot be identified.";

fn missing_template(id: u32, hour: i32) -> String {
    format!("character {id} has no Characters row at hour {hour}")
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Ideas {
    pub spans: Vec<IdeaSpan>,
    pub unobserved: Vec<HourSpan>,
    pub identity_starts: Vec<i32>,
    pub starts: Vec<(u32, u32, u32)>,
    pub templates: BTreeMap<u32, Result<BuiltTemplate, String>>,
    pub warnings: BTreeSet<&'static str>,
    last: Option<(i32, (u32, u32))>,
    open: BTreeMap<String, usize>,
}

struct Columns {
    idea: usize,
    advisor: [usize; 3],
    character: [usize; 7],
    literal: usize,
}

impl Columns {
    fn new() -> io::Result<Self> {
        let field = dashboard_declare::field;
        let characters = ["name_token_id", "token", "template_present", "template_tag", "template_file", "template_line", "template_generated"];
        let mut character = [0; 7];
        for (at, name) in characters.into_iter().enumerate() {
            character[at] = field(TableId::Characters, name)?;
        }
        Ok(Self {
            idea: field(TableId::ActiveIdeas, "token")?,
            advisor: [field(TableId::HiredAdvisors, "character")?, field(TableId::HiredAdvisors, "idea")?, field(TableId::HiredAdvisors, "slot")?],
            character,
            literal: field(TableId::LexicalTokens, "literal")?,
        })
    }
}

struct Stepper<'a> {
    history: &'a History,
    at: Vec<usize>,
    now: Vec<Option<&'a [Cell]>>,
}

impl<'a> Stepper<'a> {
    fn new(history: &'a History) -> Self {
        Self { history, at: vec![0; history.keys.len()], now: vec![None; history.keys.len()] }
    }

    fn hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.history.keys.iter().flat_map(|(_, steps)| steps.iter().map(|step| step.hour))
    }

    fn advance(&mut self, hour: i32) {
        for (index, (_, steps)) in self.history.keys.iter().enumerate() {
            while let Some(step) = steps.get(self.at[index]).filter(|step| step.hour <= hour) {
                self.now[index] = step.cells.as_deref();
                self.at[index] += 1;
            }
        }
    }

    fn shown(&self) -> impl Iterator<Item = (u128, &'a [Cell])> + '_ {
        self.history.keys.iter().zip(&self.now).filter_map(|((key, _), cells)| Some((*key, (*cells)?)))
    }

    fn cells(&self, key: u128) -> Option<&'a [Cell]> {
        let at = self.history.keys.binary_search_by_key(&key, |(held, _)| *held).ok()?;
        self.now[at]
    }

    fn place(&self, column: usize) -> Option<usize> {
        self.history.columns.binary_search(&column).ok()
    }

    fn text(&self, cell: Cell) -> Option<&'a [u8]> {
        self.history.text(cell)
    }
}

fn number(cell: Cell) -> Option<i64> {
    match cell {
        Cell::Number(value) => Some(value),
        _ => None,
    }
}

fn both(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (first, last) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if first <= last {
            out.push((first, last));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn mark(spans: &mut Vec<HourSpan>, first_hour: i32, last_hour: i32) {
    match spans.last_mut() {
        Some(span) if i64::from(span.last_hour) + 1 >= i64::from(first_hour) => {
            span.first_hour = span.first_hour.min(first_hour);
            span.last_hour = span.last_hour.max(last_hour);
        }
        _ => spans.push(HourSpan { first_hour, last_hour }),
    }
}

pub(crate) fn read(query: &Query<'_>, tag: &Country, cancelled: &AtomicBool) -> io::Result<Ideas> {
    let columns = Columns::new()?;
    let mut ideas = Ideas::default();
    let runs = query.holders.runs(tag);
    let mut holes: Vec<(i32, i32)> = query.timeline.unheld.iter().map(|unheld| (unheld.first, unheld.last)).collect();
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        ideas.close();
        let mut covered = stretch.first;
        for run in runs.iter().filter(|run| run.last >= stretch.first && run.first <= stretch.last) {
            let (first, last) = (run.first.max(stretch.first), run.last.min(stretch.last));
            if covered < first {
                holes.push((covered, first - 1));
                ideas.close();
            }
            covered = last.saturating_add(1);
            match run.holder {
                Holder::Found { slot, id } => ideas.country(query, &columns, stretch.run, (slot, id), first, last, cancelled)?,
                Holder::Unresolved => {
                    holes.push((first, last));
                    ideas.close();
                }
            }
        }
        if covered <= stretch.last {
            holes.push((covered, stretch.last));
        }
    }
    holes.extend(ideas.unobserved.drain(..).map(|span| (span.first_hour, span.last_hour)));
    holes.sort_unstable();
    for (first, last) in holes {
        mark(&mut ideas.unobserved, first, last);
    }
    Ok(ideas)
}

impl Ideas {
    fn close(&mut self) {
        self.last = None;
        self.open.clear();
    }

    fn country(&mut self, query: &Query<'_>, columns: &Columns, run: u32, who: (u32, u32), first: i32, last: i32, cancelled: &AtomicBool) -> io::Result<()> {
        let recording = query.replay.recording();
        let key = u128::from(who.1);
        let range = Keys::Range(key << 32, key << 32 | 0xFFFF_FFFF);
        let observed = cost::storage(|| -> io::Result<_> {
            let ideas = super::party_history::observed_reads(recording, run, TableId::ActiveIdeas, first, last)?;
            let advisors = super::party_history::observed_reads(recording, run, TableId::HiredAdvisors, first, last)?;
            Ok(both(&ideas, &advisors))
        })?;
        let held = cost::storage(|| lanes::history(recording, run, TableId::ActiveIdeas, &[columns.idea], range, first, last))?;
        let hired = cost::storage(|| lanes::history(recording, run, TableId::HiredAdvisors, &columns.advisor, range, first, last))?;
        check_cancel(cancelled)?;
        let at_character = hired.columns.binary_search(&columns.advisor[0]).map_err(|_| dashboard_core::invalid("An advisor history omitted a column"))?;
        let mut people: Vec<u128> = hired
            .keys
            .iter()
            .flat_map(|(_, steps)| steps.iter().filter_map(|step| step.cells.as_ref().and_then(|cells| number(cells[at_character]))))
            .filter_map(|id| u32::try_from(id).ok().map(u128::from))
            .collect();
        people.sort_unstable();
        people.dedup();
        let characters = if people.is_empty() {
            History::default()
        } else {
            cost::storage(|| lanes::history(recording, run, TableId::Characters, &columns.character, Keys::These(&people), first, last))?
        };
        let mut words: Vec<u128> = Vec::new();
        for column in [columns.character[0], columns.character[3]] {
            if let Ok(at) = characters.columns.binary_search(&column) {
                words.extend(
                    characters
                        .keys
                        .iter()
                        .flat_map(|(_, steps)| steps.iter().filter_map(|step| step.cells.as_ref().and_then(|cells| number(cells[at]))))
                        .filter_map(|word| u32::try_from(word).ok().filter(|word| *word != 0).map(u128::from)),
                );
            }
        }
        words.sort_unstable();
        words.dedup();
        let lexicon = if words.is_empty() {
            History::default()
        } else {
            cost::storage(|| lanes::history(recording, run, TableId::LexicalTokens, &[columns.literal], Keys::These(&words), first, last))?
        };
        check_cancel(cancelled)?;
        let mut held = Stepper::new(&held);
        let mut hired = Stepper::new(&hired);
        let mut characters = Stepper::new(&characters);
        let mut lexicon = Stepper::new(&lexicon);
        let mut hours: Vec<i32> = held.hours().chain(hired.hours()).chain(characters.hours()).chain(lexicon.hours()).chain([first]).collect();
        for &(start, end) in &observed {
            hours.push(start);
            if end < last {
                hours.push(end + 1);
            }
        }
        hours.retain(|hour| (first..=last).contains(hour));
        hours.sort_unstable();
        hours.dedup();
        let mut unread = 0;
        for (index, &hour) in hours.iter().enumerate() {
            let end = hours.get(index + 1).map_or(last, |next| next - 1);
            held.advance(hour);
            hired.advance(hour);
            characters.advance(hour);
            lexicon.advance(hour);
            while observed.get(unread).is_some_and(|&(_, until)| until < hour) {
                unread += 1;
            }
            if !observed.get(unread).is_some_and(|&(from, _)| from <= hour) {
                mark(&mut self.unobserved, hour, end);
                self.close();
                continue;
            }
            self.observe(hour, end, run, who, columns, &held, &hired, &characters, &lexicon);
        }
        Ok(())
    }

    fn observe(&mut self, hour: i32, end: i32, run: u32, who: (u32, u32), columns: &Columns, held: &Stepper<'_>, hired: &Stepper<'_>, characters: &Stepper<'_>, lexicon: &Stepper<'_>) {
        let continues = self.last.is_some_and(|(at, seen)| seen == who && i64::from(at) + 1 == i64::from(hour));
        if !continues {
            self.identity_starts.push(hour);
            self.starts.push((run, who.0, who.1));
            self.open.clear();
        }
        self.last = Some((end, who));
        let literal = |word: i64| -> Option<&[u8]> {
            let word = u32::try_from(word).ok().filter(|word| *word != 0)?;
            let place = lexicon.place(columns.literal)?;
            lexicon.cells(u128::from(word)).and_then(|cells| lexicon.text(cells[place]))
        };
        let place = |stepper: &Stepper<'_>, column: usize| stepper.place(column);
        let mut roles: BTreeMap<Vec<u8>, (String, IdeaCharacter)> = BTreeMap::new();
        if let (Some(at_character), Some(at_idea), Some(at_seat)) = (place(hired, columns.advisor[0]), place(hired, columns.advisor[1]), place(hired, columns.advisor[2])) {
            for (_, cells) in hired.shown() {
                let seat = hired.text(cells[at_seat]).unwrap_or_default();
                let Ok(seat) = std::str::from_utf8(seat) else {
                    self.warnings.insert(SEAT);
                    continue;
                };
                let idea = hired.text(cells[at_idea]).unwrap_or_default();
                if idea.is_empty() {
                    self.warnings.insert(TOKENLESS_SEATS);
                    continue;
                }
                let Some(id) = number(cells[at_character]).and_then(|id| u32::try_from(id).ok()) else { continue };
                let row = character_row(characters, columns, id);
                let token = row.as_ref().and_then(|row| literal(i64::from(row.name_token_id))).and_then(|token| std::str::from_utf8(token).ok()).unwrap_or_default();
                let name = row.as_ref().and_then(|row| std::str::from_utf8(&row.token).ok()).unwrap_or_default();
                roles.insert(idea.to_vec(), (seat.to_owned(), IdeaCharacter { id, token: token.to_owned(), name: name.to_owned() }));
            }
        }
        let mut tokens: BTreeSet<Vec<u8>> = BTreeSet::new();
        if let Some(at) = held.place(columns.idea) {
            tokens.extend(held.shown().filter_map(|(_, cells)| held.text(cells[at])).filter(|token| !token.is_empty()).map(<[u8]>::to_vec));
        }
        tokens.extend(roles.keys().cloned());
        self.open.retain(|token, _| tokens.contains(token.as_bytes()));
        for bytes in tokens {
            let Ok(token) = std::str::from_utf8(&bytes) else {
                self.warnings.insert(TOKEN);
                continue;
            };
            let role = roles.get(&bytes);
            match self.open.get(token) {
                Some(&index) if self.spans[index].slot.as_deref() == role.map(|role| role.0.as_str()) && self.spans[index].character.as_ref() == role.map(|role| &role.1) => {
                    self.spans[index].last_hour = end;
                }
                _ => {
                    if let Some((_, holder)) = role {
                        let template = match character_row(characters, columns, holder.id) {
                            Some(row) => {
                                let spelled: Vec<LexicalTokenRow> = literal(i64::from(row.template_tag)).map(|literal| LexicalTokenRow { token: row.template_tag, literal: literal.to_vec() }).into_iter().collect();
                                built_template(&row, Some(&spelled))
                            }
                            None => Err(missing_template(holder.id, hour)),
                        };
                        self.templates.insert(holder.id, template);
                    }
                    self.open.insert(token.to_owned(), self.spans.len());
                    self.spans.push(IdeaSpan {
                        token: token.to_owned(),
                        first_hour: hour,
                        last_hour: end,
                        slot: role.map(|role| role.0.clone()),
                        character: role.map(|role| role.1.clone()),
                    });
                }
            }
        }
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "only its tests call it"))]
    pub fn additions(&self) -> impl Iterator<Item = &IdeaSpan> {
        let mut previous: BTreeMap<&str, &IdeaSpan> = BTreeMap::new();
        self.spans.iter().filter(move |span| {
            let preceding = previous.insert(&span.token, span);
            let same_holding = preceding.is_some_and(|last| {
                i64::from(last.last_hour) + 1 == i64::from(span.first_hour)
                    && last.slot == span.slot
                    && last.character.as_ref().map(|holder| holder.id) == span.character.as_ref().map(|holder| holder.id)
            });
            let after_gap = self
                .unobserved
                .partition_point(|gap| gap.first_hour < span.first_hour)
                .checked_sub(1)
                .and_then(|index| self.unobserved.get(index))
                .is_some_and(|gap| i64::from(gap.last_hour) + 1 >= i64::from(span.first_hour));
            !same_holding && !after_gap && self.identity_starts.binary_search(&span.first_hour).is_err()
        })
    }
}

fn character_row(characters: &Stepper<'_>, columns: &Columns, id: u32) -> Option<CharacterRow> {
    let cells = characters.cells(u128::from(id))?;
    let at = |column: usize| characters.place(column).map(|place| cells[place]);
    let number = |column: usize| at(column).and_then(number);
    let text = |column: usize| at(column).and_then(|cell| characters.text(cell)).map(<[u8]>::to_vec).unwrap_or_default();
    let [name_token, token, present, tag, file, line, generated] = columns.character;
    Some(CharacterRow {
        id,
        name_token_id: number(name_token).and_then(|value| i32::try_from(value).ok()).unwrap_or(-1),
        token: text(token),
        template_present: number(present).and_then(|value| u8::try_from(value).ok()).unwrap_or_default(),
        template_tag: number(tag).and_then(|value| u32::try_from(value).ok()).unwrap_or_default(),
        template_file: text(file),
        template_line: number(line).and_then(|value| i32::try_from(value).ok()).unwrap_or_default(),
        template_generated: number(generated).and_then(|value| u8::try_from(value).ok()).unwrap_or_default(),
        ..Default::default()
    })
}

pub(crate) fn naming_at(frames: &mut dashboard_base::products::at_hour::Frames<'_>, hour: i32, id: u32) -> io::Result<NamingCountry> {
    use dashboard_base::products::at_hour::Read;
    let reads = [
        Read::these(TableId::Countries, [u128::from(id)]),
        Read::these(TableId::CountryAppearance, [u128::from(id)]),
        Read::all(TableId::CountryLinks),
        Read::all(TableId::CountryCivilWars),
        Read::all(TableId::CountryWars),
        Read::all(TableId::WarRelations),
        Read::all(TableId::CountryCounterInputs),
    ];
    let frame = frames.frame(hour, &reads, "An identity's naming")?;
    let Some(country) = frame.complete::<tables::Countries>().ok().and_then(|rows| rows.iter().find(|row| row.id == id)) else {
        return Ok(NamingCountry::default());
    };
    let appearance = frame.complete::<tables::CountryAppearance>().ok().and_then(|rows| rows.iter().find(|row| row.country == id));
    Ok(NamingCountry::read_with_appearance(&frame, country, appearance))
}

pub(crate) type RecordedDefinitions = Option<BTreeMap<String, ModifierDefinitionRow>>;

#[derive(Default)]
struct KeptDefinitions(Vec<((u32, i32), Arc<RecordedDefinitions>)>);

pub(crate) fn recorded_definitions(source: &Source, query: &Query<'_>) -> io::Result<Arc<RecordedDefinitions>> {
    let Some(stretch) = query.timeline.stretches.last() else { return Ok(Arc::new(None)) };
    let at = (stretch.run, stretch.last);
    let state = source.state::<KeptDefinitions>();
    let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, held)) = kept.0.iter().find(|(held, _)| *held == at) {
        return Ok(Arc::clone(held));
    }
    let read = Arc::new(definitions_at(query, stretch.run, stretch.last)?);
    kept.0.push((at, Arc::clone(&read)));
    Ok(read)
}

fn definitions_at(query: &Query<'_>, run: u32, hour: i32) -> io::Result<RecordedDefinitions> {
    let recording = query.replay.recording();
    let definitions = cost::storage(|| replay_lanes::at::frame(recording, run, hour, &[replay_lanes::at::Read::all(TableId::ModifierDefinitions)]))?;
    let Ok(rows) = definitions.complete::<tables::ModifierDefinitions>() else { return Ok(None) };
    let mut words: Vec<u128> = rows.iter().map(|row| u128::from(row.token)).collect();
    words.sort_unstable();
    words.dedup();
    let reads = [replay_lanes::at::Read { table: TableId::LexicalTokens, columns: None, keys: replay_lanes::at::Keys::These(&words) }];
    let lexicon = cost::storage(|| replay_lanes::at::frame(recording, run, hour, &reads))?;
    let Ok(lexical) = lexicon.complete::<tables::LexicalTokens>() else { return Ok(None) };
    Ok(Some(idea_format::index_recorded(rows, lexical)))
}

fn line(modifier: &Modifier, hidden: bool) -> EffectLine {
    EffectLine { key: modifier.name.clone(), fixed5: modifier.value.to_string(), hidden }
}

fn known<'a, T>(found: Result<Option<&'a T>, &Gap>) -> (Option<&'a T>, Option<String>) {
    match found {
        Ok(found) => (found, None),
        Err(gap) => (None, Some(gap.to_string())),
    }
}

fn targeted(groups: &[content_reader_script::modifiers::TargetedModifier]) -> Vec<EffectGroup> {
    groups.iter().map(|group| EffectGroup { target: group.tag.clone(), instant: false, lines: lines(&group.modifiers), tooltips: group.modifiers.tooltips() }).collect()
}

fn equipment(bonuses: &[content_reader_script::modifiers::EquipmentBonus]) -> Vec<EffectGroup> {
    bonuses
        .iter()
        .map(|bonus| EffectGroup { target: Some(bonus.equipment.clone()), instant: bonus.instant, lines: bonus.values.iter().map(|m| line(m, false)).collect(), tooltips: Vec::new() })
        .collect()
}

fn lines(modifiers: &Modifiers) -> Vec<EffectLine> {
    modifiers.values.iter().map(|m| line(m, modifiers.hidden.contains(&m.name))).collect()
}

pub(crate) fn idea_info(content: &PoliticsContent, token: &str, recorded_seat: Option<&str>, built: Result<&BuiltTemplate, &str>) -> IdeaInfo {
    let tags = &content.ideas.tags;
    let mut unknown = Vec::new();
    let (idea, gap) = known(content.ideas.idea(token));
    unknown.extend(gap);
    let authored_role = content.characters.held_advisor(token, built).inspect_err(|reason| unknown.push(reason.clone())).ok().flatten();
    let (group_category, gap) = match idea {
        Some(idea) => known(tags.resolve(&idea.group)),
        None => (None, None),
    };
    unknown.extend(gap.clone());
    let named = group_category.zip(idea).is_some_and(|(category, idea)| category.name == idea.group);
    let slot = recorded_seat
        .map(str::to_owned)
        .or_else(|| authored_role.map(|role| role.slot.clone()))
        .or_else(|| idea.filter(|_| !named && gap.is_none()).map(|idea| idea.group.clone()));
    let category = match (group_category, &slot) {
        (Some(category), None) => Some(category.name.clone()),
        (_, Some(slot)) => {
            let (category, gap) = known(tags.resolve(slot));
            unknown.extend(gap);
            category.map(|category| category.name.clone())
        }
        (None, None) => None,
    };
    let trait_tokens = idea.map(|idea| idea.traits.clone()).or_else(|| authored_role.map(|role| role.traits.clone())).unwrap_or_default();
    let traits = trait_tokens
        .iter()
        .map(|trait_token| {
            let (found, unknown) = known(content.traits.get(trait_token));
            TraitEffects {
                token: trait_token.clone(),
                known: found.is_some(),
                unknown,
                name: found.and_then(|found| content.localised(&found.token, View::Plain)),
                lines: found.map(|found| lines(&found.modifiers)).unwrap_or_default(),
                targeted: found.map(|found| targeted(&found.targeted)).unwrap_or_default(),
                equipment: found.map(|found| equipment(&found.equipment_bonuses)).unwrap_or_default(),
                tooltips: found.map(|found| found.modifiers.tooltips()).unwrap_or_default(),
            }
        })
        .collect();
    let modifiers = idea.map(|idea| &idea.modifiers).or_else(|| authored_role.map(|role| &role.modifiers));
    let research = idea.map(|idea| &idea.research_bonuses).or_else(|| authored_role.map(|role| &role.research_bonuses));
    let effects_known = idea.is_some() || authored_role.is_some();
    IdeaInfo {
        token: token.to_owned(),
        category,
        slot,
        law: idea.is_some_and(|idea| idea.law),
        designer: idea.is_some_and(|idea| idea.designer),
        effects: modifiers.map(lines).unwrap_or_default(),
        targeted: idea.map(|idea| targeted(&idea.targeted)).unwrap_or_default(),
        equipment: idea.map(|idea| equipment(&idea.equipment_bonuses)).unwrap_or_default(),
        research: research.map(|bonuses| bonuses.iter().map(|m| line(m, false)).collect()).unwrap_or_default(),
        traits,
        tooltips: modifiers.map(Modifiers::tooltips).unwrap_or_default(),
        effects_known,
        unknown: (!effects_known || !unknown.is_empty()).then(|| unknown.join("; ")).filter(|text| !text.is_empty()),
        name_missing: None,
    }
}

pub(crate) fn categories(content: &PoliticsContent) -> Vec<CategoryInfo> {
    let (categories, _) = content.ideas.tags.categories();
    categories
        .iter()
        .map(|category| {
            let mut slots: Vec<SlotInfo> = category
                .slots
                .iter()
                .map(|slot| SlotInfo {
                    token: slot.clone(),
                    name: content.localised(slot, View::Plain),
                    character: false,
                    seats: 1,
                    laws: content.ideas.laws(slot).0.into_iter().map(|idea| idea.token.clone()).collect(),
                })
                .collect();
            for seat in &category.character_slots {
                match slots.iter_mut().find(|slot| slot.character && slot.token == *seat) {
                    Some(slot) => slot.seats += 1,
                    None => slots.push(SlotInfo { token: seat.clone(), name: content.localised(seat, View::Plain), character: true, seats: 1, laws: Vec::new() }),
                }
            }
            CategoryInfo {
                token: category.name.clone(),
                name: content.localised(&category.name, View::Plain),
                kind: category.kind.clone(),
                hidden: category.hidden,
                politics_tab: category.politics_tab,
                slots,
            }
        })
        .collect()
}

pub(crate) fn effect_keys<'a>(ideas: impl IntoIterator<Item = &'a IdeaInfo>) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut modifier_keys = BTreeSet::new();
    let mut text_keys: BTreeSet<String> = ["YES", "NO"].map(str::to_owned).into();
    for idea in ideas {
        let mut effects = |lines: &[EffectLine], targeted: &[EffectGroup], tooltips: &[String]| {
            modifier_keys.extend(lines.iter().chain(targeted.iter().flat_map(|group| &group.lines)).map(|line| line.key.clone()));
            text_keys.extend(tooltips.iter().cloned());
            text_keys.extend(targeted.iter().flat_map(|group| group.tooltips.iter().cloned()));
        };
        effects(&idea.effects, &idea.targeted, &idea.tooltips);
        for found in &idea.traits {
            effects(&found.lines, &found.targeted, &found.tooltips);
        }
        text_keys.extend(idea.research.iter().map(|line| format!("{}_research", line.key)));
        text_keys.extend(idea.equipment.iter().chain(idea.traits.iter().flat_map(|found| &found.equipment)).filter_map(|group| group.target.clone()));
    }
    (modifier_keys, text_keys)
}

pub(crate) fn present(
    content: &PoliticsContent,
    params: &CountryParams,
    tag: &str,
    ideas: &Ideas,
    naming: &[NamingCountry],
    recorded: Option<&BTreeMap<String, ModifierDefinitionRow>>,
    cancelled: &AtomicBool,
) -> io::Result<IdeaHistoryReply> {
    let tokens: BTreeSet<&str> = ideas.spans.iter().map(|span| span.token.as_str()).collect();
    let mut warnings = vec![format!(
        "{}; visual assets are outside content hash coverage.",
        dashboard_base::products::artwork::CURRENT_INSTALL_ART_LABEL
    )];
    warnings.extend(content.warnings.iter().cloned());
    let (_, category_gap) = content.ideas.tags.categories();
    let (_, law_gap) = content.ideas.laws("");
    for gap in category_gap.into_iter().chain(law_gap) {
        warnings.push(format!("Idea categories and laws may be incomplete: {gap}."));
    }
    warnings.extend(ideas.warnings.iter().map(|warning| (*warning).to_owned()));
    let mut artwork = Vec::new();
    let mut holders = Vec::new();
    let mut infos = Vec::new();
    let mut unplaced = Vec::new();
    let unnamed = NamingCountry::default();
    for token in tokens {
        check_cancel(cancelled)?;
        let latest = ideas.spans.iter().rev().find(|span| span.token == token);
        let named = latest
            .and_then(|span| ideas.identity_starts.partition_point(|start| *start <= span.first_hour).checked_sub(1))
            .and_then(|index| naming.get(index))
            .unwrap_or(&unnamed);
        let country = if latest.is_some() { named.for_tag(tag) } else { IdeaCountry { tag, slot: 0, look: None } };
        let built = match latest.and_then(|span| span.character.as_ref()).map(|holder| ideas.templates.get(&holder.id)) {
            Some(Some(Ok(template))) => Ok(template),
            Some(Some(Err(reason))) => Err(reason.as_str()),
            Some(None) | None => Err("no recorded holder names its template"),
        };
        let mut found = idea_info(content, token, latest.and_then(|span| span.slot.as_deref()), built);
        if found.category.is_none() {
            unplaced.push(token.to_owned());
        }
        let holder_art = |script: Option<&str>, saved: &str| {
            let (mut art, mut problem) = content.held_idea_artwork(token, script, saved, country);
            if script.is_none() {
                art.icon = None;
                problem = Some(format!("{token}: artwork is unknown because the holder has no recorded script token; a placeholder is shown"));
            }
            (art, problem)
        };
        let held = latest.and_then(|span| span.character.as_ref()).map(|holder| ((!holder.token.is_empty()).then_some(holder.token.as_str()), holder.name.as_str()));
        let (art, problem) = match held {
            Some((script, saved)) => holder_art(script, saved),
            None => content.idea_artwork(token, None, country),
        };
        if art.name.is_none() {
            found.name_missing = match held {
                Some((script, saved)) => content.idea_name_missing(token, script, country, Some(saved)),
                None => content.idea_name_missing(token, None, country, None),
            };
        }
        artwork.push(art);
        warnings.extend(problem);
        let mut seen = BTreeSet::new();
        for holder in ideas
            .spans
            .iter()
            .filter(|span| span.token == token)
            .filter_map(|span| span.character.as_ref())
            .filter(|holder| (!holder.token.is_empty() || !holder.name.is_empty()) && seen.insert((holder.id, holder.token.clone(), holder.name.clone())))
        {
            let script = (!holder.token.is_empty()).then_some(holder.token.as_str());
            let (art, problem) = holder_art(script, &holder.name);
            holders.push(HolderArtwork { token: token.to_owned(), character: holder.id, character_token: holder.token.clone(), name: holder.name.clone(), artwork: art });
            warnings.extend(problem);
        }
        infos.push(found);
    }
    if !unplaced.is_empty() {
        warnings.push(format!("{} ideas are in no category the content defines: {}.", unplaced.len(), unplaced.iter().take(8).cloned().collect::<Vec<_>>().join(", ")));
    }
    let (modifier_keys, text_keys) = effect_keys(&infos);
    let (modifiers, format_warnings) = idea_format::modifier_formats_from_recorded(recorded, &modifier_keys, content);
    warnings.extend(format_warnings);
    let texts = idea_format::texts(content, &text_keys);
    warnings.extend(content.catalogue.refusal_lines());
    Ok(IdeaHistoryReply {
        branch_id: params.branch_id.clone(),
        tag: tag.to_owned(),
        asked: params.country.clone(),
        spans: ideas.spans.clone(),
        ideas: infos,
        artwork,
        holders,
        categories: categories(content),
        unobserved: ideas.unobserved.clone(),
        identity_starts: ideas.identity_starts.clone(),
        modifiers,
        texts,
        warnings,
    })
}

#[derive(Default)]
struct Kept(Option<(u64, Country, Arc<(Ideas, Vec<NamingCountry>)>)>);

pub(crate) fn history(source: &Source, branch: u64, tag: &Country, cancelled: &AtomicBool) -> io::Result<Arc<(Ideas, Vec<NamingCountry>)>> {
    let state = source.state::<Kept>();
    let held = state.lock().unwrap_or_else(PoisonError::into_inner).0.as_ref().filter(|(at, held, _)| *at == branch && *held == *tag).map(|(_, _, read)| Arc::clone(read));
    if let Some(read) = held {
        return Ok(read);
    }
    let query = dashboard_base::queries::query(source, branch)?;
    let ideas = read(&query, tag, cancelled)?;
    let mut frames = dashboard_base::products::at_hour::Frames::new(&query)?;
    let naming = ideas
        .identity_starts
        .iter()
        .zip(&ideas.starts)
        .map(|(&hour, &(_, _, id))| naming_at(&mut frames, hour, id))
        .collect::<io::Result<Vec<_>>>()?;
    let read = Arc::new((ideas, naming));
    state.lock().unwrap_or_else(PoisonError::into_inner).0 = Some((branch, tag.clone(), Arc::clone(&read)));
    Ok(read)
}

pub struct IdeaHistory;

impl Product for IdeaHistory {
    const KEY: &'static str = "idea-history";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = IdeaHistoryReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            CountryParams::decl(),
            IdeaHistoryReply::decl(),
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
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let content = politics_content(source, context.content()?)?;
        let query = dashboard_base::queries::query(source, params.branch())?;
        let (read, recorded) = rayon::join(|| history(source, params.branch(), &params.country, cancelled), || recorded_definitions(source, &query));
        let (read, recorded) = (read?, recorded?);
        let reply = present(&content, params, &query.holders.nearest_name(&params.country, query.timeline.last), &read.0, &read.1, (*recorded).as_ref(), cancelled)?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
