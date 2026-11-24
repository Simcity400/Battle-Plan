
use super::SpendItem;
use crate::products::ideas::{IdeaInfo, idea_info};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::at_hour};
use dashboard_base::products::artwork::sprites::SpriteFrame;
use dashboard_world::products::equipment_names as names;
use dashboard_world::products::equipment_names::EquipmentIcon;
use dashboard_base::products::{
    character_portraits::{RecordedPortrait, portrait_png, recorded_portrait_in},
    politics_content::PoliticsContent,
};
use dashboard_core::config::LoadContext;
use dashboard_query::Query;
use dashboard_text::{
    catalogue::{View, plain_text},
    identity::{Look, NamingCountry},
    scripted_text::ScriptedText,
};
use content_reader_country_names::country_names::CountryNames;
use replay_data::rows::*;
use replay_frame::frame::{DecodedFrame, HourFrame, Rowset, Tables};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{Arc, Mutex, PoisonError, atomic::AtomicBool},
};

const NO_CONTENT: &str = "Names and pictures need the game folders.";

const WHOLE: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::PoliticalParties,
    TableId::Politics,
    TableId::ReservedCountryNaming,
    TableId::Autonomy,
    TableId::LexicalTokens,
];

const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);
const LEADER: u64 = mask::<UnitLeaderRow>(&["character_id", "category"]);
const ROLE: u64 = mask::<AdvisorRoleRow>(&["idea", "slot"]);
const PORTRAIT: u64 = mask::<CharacterPortraitRow>(&["portraits"]);
const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "tag"]);
const CHARACTER: u64 = dashboard_text::identity::CHARACTER_IDENTITY;
const COST_MODIFIER: u64 = mask::<ModifierDefinitionRow>(&["name_key", "argument"]);
const LITERAL_COLUMN: usize = column::<LexicalTokenRow>("literal");
const COST_MODIFIER_INDEX: u128 = 0x26;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct NamedItem {
    pub category: (String, String),
    pub name: Option<String>,
    pub action: String,
    pub icon: Option<String>,
    pub character: Option<u32>,
    pub target_tag: Option<String>,
    pub target_name: Option<String>,
    pub unknown: Option<String>,
    pub group: String,
    pub idea: Option<(IdeaInfo, Option<String>)>,
    pub kind: Option<Result<(String, String), String>>,
}

pub(super) struct Context {
    content: Result<Arc<PoliticsContent>, String>,
    organisations: names::Context,
    names: Arc<Mutex<Names>>,
}

#[derive(Default)]
struct Names {
    catalogue: Option<Arc<Catalogue>>,
    items: HashMap<(u32, u32, u32), (NamedItem, Option<(String, String)>)>,
}

impl Context {
    pub(super) fn content(&self) -> Result<&PoliticsContent, &str> {
        self.content.as_deref().map_err(String::as_str)
    }
}

pub(super) fn context(source: &Source, load: &LoadContext) -> io::Result<Context> {
    let organisations = names::context(source, load)?;
    let Some(options) = load.content.as_ref() else {
        return Ok(Context { content: Err(NO_CONTENT.into()), organisations, names: Arc::new(Mutex::new(Names::default())) });
    };
    let catalogue = catalogue::catalogue(source, options)?;
    let content = match dashboard_base::products::politics_content::politics_content(source, options) {
        Ok(content) => Ok(content),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
        Err(error) => Err(format!("Politics content is unavailable: {error}")),
    };
    let names = source.state::<Names>();
    {
        let mut held = names.lock().unwrap_or_else(PoisonError::into_inner);
        if !held.catalogue.as_ref().is_some_and(|kept| Arc::ptr_eq(kept, &catalogue)) {
            *held = Names { catalogue: Some(catalogue), items: HashMap::new() };
        }
    }
    Ok(Context { content, organisations, names })
}

pub(super) struct Asked<'a> {
    pub run: u32,
    pub row: &'a PowerItemRow,
}

pub(super) fn name(query: &Query<'_>, asked: &[Asked<'_>], context: &Context, icons: &mut BTreeMap<String, String>, cancelled: &AtomicBool) -> io::Result<Vec<NamedItem>> {
    let mut out: Vec<Option<NamedItem>> = vec![None; asked.len()];
    let mut wanted: BTreeMap<(u32, i32), Vec<usize>> = BTreeMap::new();
    {
        let held = context.names.lock().unwrap_or_else(PoisonError::into_inner);
        for (at, item) in asked.iter().enumerate() {
            match held.items.get(&(item.run, item.row.holder, item.row.ordinal)) {
                Some((named, image)) => {
                    if let (Some(key), Some((_, image))) = (&named.icon, image) {
                        icons.entry(key.clone()).or_insert_with(|| image.clone());
                    }
                    out[at] = Some(named.clone());
                }
                None => wanted.entry((item.run, item.row.first_read)).or_default().push(at),
            }
        }
    }
    let mut frames = at_hour::Frames::new(query)?;
    for ((_, hour), items) in wanted {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let rows: Vec<&PowerItemRow> = items.iter().map(|&at| asked[at].row).collect();
        let light = readable(read_frame(&mut frames, hour, &rows, false))?;
        let mut named_here = Vec::with_capacity(items.len());
        let mut scripted_rows = Vec::new();
        for &at in &items {
            let (named, image) = named_item(light.as_ref(), asked[at].row, context, false);
            if light.is_some() && named.unknown.as_deref().is_some_and(|why| why.contains(FRAME_UNREAD)) {
                scripted_rows.push(at);
            } else {
                named_here.push((at, named, image));
            }
        }
        if !scripted_rows.is_empty() {
            let rows: Vec<&PowerItemRow> = scripted_rows.iter().map(|&at| asked[at].row).collect();
            let full = readable(read_frame(&mut frames, hour, &rows, true))?;
            for &at in &scripted_rows {
                let (named, image) = named_item(full.as_ref(), asked[at].row, context, true);
                named_here.push((at, named, image));
            }
        }
        for (_, named, image) in &named_here {
            if let (Some(key), Some(image)) = (&named.icon, image) {
                icons.entry(key.clone()).or_insert_with(|| image.clone());
            }
        }
        let mut held = context.names.lock().unwrap_or_else(PoisonError::into_inner);
        for (at, named, image) in named_here {
            let item = &asked[at];
            held.items.insert((item.run, item.row.holder, item.row.ordinal), (named.clone(), image.map(|image| (named.icon.clone().unwrap_or_default(), image))));
            out[at] = Some(named);
        }
    }
    Ok(out.into_iter().map(|named| named.expect("every asked item is named")).collect())
}

const FRAME_UNREAD: &str = "frame is unavailable";

fn readable(frame: io::Result<DecodedFrame>) -> io::Result<Option<DecodedFrame>> {
    match frame {
        Ok(frame) => Ok(Some(frame)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
        Err(_) => Ok(None),
    }
}

fn read_frame(frames: &mut at_hour::Frames<'_>, hour: i32, rows: &[&PowerItemRow], scripted: bool) -> io::Result<DecodedFrame> {
    let mut whole: BTreeSet<TableId> = WHOLE.iter().copied().collect();
    if scripted {
        whole.extend(dashboard_text::scripted_text::TABLES.iter().copied());
    }
    let mut keyed: BTreeMap<TableId, Vec<u128>> = BTreeMap::new();
    for row in rows {
        if let Some(character) = row.character {
            keyed.entry(TableId::Characters).or_default().push(u128::from(character));
            keyed.entry(TableId::CharacterPortraits).or_default().push(u128::from(character));
            keyed.entry(TableId::AdvisorRoles).or_default().extend([u128::from(character) << 32]);
        }
        if let (PowerObjectKind::Leader, Some(reference)) = (row.object_kind, row.object_reference) {
            keyed.entry(TableId::UnitLeaders).or_default().push(u128::from(reference));
        }
        if let (PowerObjectKind::Organisation, Some(reference)) = (row.object_kind, row.object_reference) {
            keyed.entry(TableId::IndustrialOrganisations).or_default().push(u128::from(reference));
        }
        if row.component == Some(PowerDailyCostKind::PoliticalPowerCostModifier as u32) {
            keyed.entry(TableId::ModifierDefinitions).or_default().push(COST_MODIFIER_INDEX);
        }
    }
    let mut reads: Vec<at_hour::Read> = whole.iter().map(|&table| at_hour::Read::all(table)).collect();
    for (table, keys) in keyed {
        if whole.contains(&table) {
            continue;
        }
        if table == TableId::AdvisorRoles {
            let owners: BTreeSet<u128> = keys.iter().map(|key| key >> 32).collect();
            reads.push(match owners.iter().next() {
                Some(&owner) if owners.len() == 1 => at_hour::Read {
                    table,
                    keys: at_hour::Keys::Range(owner << 32, owner << 32 | u128::from(u32::MAX)),
                },
                _ => at_hour::Read::all(table),
            });
            continue;
        }
        reads.push(at_hour::Read::these(table, keys));
    }
    frames.frame(hour, &reads, "Politics spending item")
}

fn complete<'f, T: TableSchema>(frame: &'f DecodedFrame, mask: u64) -> Option<Rowset<'f, T::Row>> {
    frame.complete_rows::<T>(mask).ok()
}

fn keyed<'f, R: RowCodec>(rows: Option<Rowset<'f, R>>, key: u128) -> Option<Cow<'f, R>> {
    rows?.get(key).ok().flatten()
}

fn lexical_token(frame: &DecodedFrame, token: u32) -> Option<String> {
    let rows = complete::<tables::LexicalTokens>(frame, LITERAL)?;
    let literal = rows.text(u128::from(token), |row| &row.literal, LITERAL_COLUMN).ok().flatten()?;
    std::str::from_utf8(literal).ok().map(str::to_owned)
}

#[derive(Clone, Debug)]
struct NamedCountry {
    tag: String,
    name: Option<String>,
    naming: NamingCountry,
    ruling_group: Result<Option<String>, String>,
}

#[derive(Clone, Debug)]
struct CharacterFact {
    id: u32,
    script_token: Option<String>,
    saved_name: String,
    template: Result<content_reader_key_names::characters::BuiltTemplate, String>,
    portrait: RecordedPortrait,
}

#[derive(Default)]
struct Facts {
    token: Option<String>,
    character: Option<CharacterFact>,
    modifier: Option<(Vec<u8>, Vec<u8>)>,
}

#[derive(Default)]
struct ItemText {
    name: Option<String>,
    sprites: Option<SpriteChoice>,
    unknown: Option<String>,
}

#[derive(Clone, Debug)]
struct SpriteChoice {
    candidates: Vec<String>,
    otherwise: Result<Option<String>, String>,
}

enum Art {
    Idea { token: String, character: Option<String>, tag: String, naming: NamingCountry, holder: Option<(u32, RecordedPortrait)> },
    Portrait { character: u32, portrait: RecordedPortrait },
    Sprites(SpriteChoice),
    Sprite(String),
}

pub(super) fn token_of(frame: Option<&DecodedFrame>, row: &PowerItemRow) -> Option<String> {
    if !row.item_key.is_empty() {
        return std::str::from_utf8(&row.item_key).ok().map(str::to_owned);
    }
    lexical_token(frame?, row.item_token?).filter(|token| !token.is_empty())
}

fn character_fact(frame: &DecodedFrame, row: &PowerItemRow) -> Option<CharacterFact> {
    let leader = (row.object_kind == PowerObjectKind::Leader)
        .then(|| keyed(complete::<tables::UnitLeaders>(frame, LEADER), u128::from(row.object_reference?)))
        .flatten();
    let id = row.character.or_else(|| leader.as_ref().map(|leader| leader.character_id))?;
    let character = keyed(complete::<tables::Characters>(frame, CHARACTER), u128::from(id))?;
    let lexical = complete::<tables::LexicalTokens>(frame, LITERAL);
    let category = leader.as_ref().map(|leader| leader.category);
    let (role, size) = match row.cause {
        PowerCause::Advisor | PowerCause::AdvisorRemoval | PowerCause::Idea => (PORTRAIT_CIVILIAN, PORTRAIT_LARGE),
        PowerCause::OfficerUpgrade => (1, 0),
        PowerCause::CaptainUpgrade => (2, 0),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => (4, PORTRAIT_LARGE),
        _ => (
            match category {
                Some(0 | 1) => 1,
                Some(2) => 2,
                Some(3) => 4,
                _ => u32::MAX,
            },
            PORTRAIT_LARGE,
        ),
    };
    Some(CharacterFact {
        id,
        script_token: lexical.and_then(|lexical| dashboard_text::identity::character_script_token_in(&character, lexical)).map(str::to_owned),
        saved_name: std::str::from_utf8(&character.token).ok()?.to_owned(),
        template: dashboard_text::identity::built_template_in(&character, complete::<tables::LexicalTokens>(frame, LITERAL)),
        portrait: if role == u32::MAX {
            RecordedPortrait::Unrecorded
        } else {
            recorded_portrait_in(complete::<tables::CharacterPortraits>(frame, PORTRAIT), id, role, size)
        },
    })
}

fn facts(frame: Option<&DecodedFrame>, row: &PowerItemRow) -> Facts {
    let character = frame.and_then(|frame| character_fact(frame, row));
    let mut token = token_of(frame, row);
    if token.is_none() && matches!(row.cause, PowerCause::Advisor | PowerCause::AdvisorRemoval) {
        token = (|| {
            let frame = frame?;
            let first = u128::from(character.as_ref()?.id) << 32;
            complete::<tables::AdvisorRoles>(frame, ROLE)?
                .range(first..=first | u128::from(u32::MAX))
                .filter_map(Result::ok)
                .find(|role| row.slot_key.is_empty() || role.slot == row.slot_key)
                .and_then(|role| std::str::from_utf8(&role.idea).ok().filter(|idea| !idea.is_empty()).map(str::to_owned))
        })();
    }
    let modifier = (row.component == Some(PowerDailyCostKind::PoliticalPowerCostModifier as u32))
        .then(|| {
            let found = keyed(complete::<tables::ModifierDefinitions>(frame?, COST_MODIFIER), COST_MODIFIER_INDEX)?;
            Some((found.name_key.clone(), found.argument.clone()))
        })
        .flatten();
    Facts { token, character, modifier }
}

fn first_at(frame: &DecodedFrame, slot: u32) -> Option<CountryRow> {
    complete::<tables::Countries>(frame, COUNTRY)?.iter().filter_map(Result::ok).find(|country| country.canonical_slot == slot).map(Cow::into_owned)
}

fn named_country(frame: &DecodedFrame, content: Option<&PoliticsContent>, slot: u32) -> Option<NamedCountry> {
    let country = first_at(frame, slot)?;
    let tag = std::str::from_utf8(&country.tag).ok()?.to_owned();
    dashboard_base::products::validate_tag(&tag).ok()?;
    let names = content.map(|content| CountryNames::read_selected(frame, &content.catalogue.localisation, [slot]));
    Some(NamedCountry {
        name: names.as_ref().and_then(|names| names.name(slot).ok()).and_then(|name| plain_text(&name).ok()),
        naming: NamingCountry { slot, look: Look::read(frame, &country) },
        ruling_group: names.as_ref().map_or_else(
            || Err("recorded content unresolved".into()),
            |names| names.ruling_group(slot).map(|group| group.map(str::to_owned)).map_err(|reason| undated(&reason)),
        ),
        tag,
    })
}

fn linked(frame: &DecodedFrame, raw: u32) -> Option<u32> {
    keyed(complete::<tables::CountryLinks>(frame, LINK), u128::from(raw)).map(|link| link.canonical_slot)
}

pub(super) fn undated(reason: &str) -> String {
    let mut words = reason.split_whitespace().peekable();
    let mut result = Vec::new();
    while let Some(word) = words.next() {
        result.push(word);
        if matches!(word, "hour" | "frame") && words.peek().is_some_and(|next| next.trim_end_matches([':', ',', ';', '.']).parse::<i32>().is_ok()) {
            words.next();
        }
    }
    result.join(" ")
}

fn combine(target: &mut Option<String>, reason: Option<String>) {
    if let Some(reason) = reason {
        let reason = undated(&reason);
        *target = Some(match target.take() {
            Some(previous) if previous != reason => format!("{previous}; {reason}"),
            Some(previous) => previous,
            None => reason,
        });
    }
}

pub(super) fn cause_category(cause: PowerCause) -> (&'static str, &'static str) {
    match cause {
        PowerCause::Idea => ("ideas", "Ideas"),
        PowerCause::Advisor | PowerCause::AdvisorRemoval => ("advisors", "Advisors"),
        PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout => ("decisions", "Decisions"),
        PowerCause::Diplomacy | PowerCause::JoinWar => ("diplomacy", "Diplomacy"),
        PowerCause::WarJustification => ("warJustification", "War justification"),
        PowerCause::Focus | PowerCause::FocusRefund => ("focus", "Focus"),
        PowerCause::ScriptEffect | PowerCause::ScaledScriptEffect => ("scriptEffects", "Script effects"),
        PowerCause::Autonomy => ("autonomy", "Autonomy"),
        PowerCause::SpyMaster => ("spyMaster", "Spy master"),
        PowerCause::ScientistRecruitment | PowerCause::ScientistFaction => ("scientists", "Scientists"),
        PowerCause::ScorchedEarth => ("scorchedEarth", "Scorched earth"),
        PowerCause::PrideOfFleet => ("prideOfFleet", "Pride of the fleet"),
        PowerCause::Blockade => ("blockade", "Blockade"),
        PowerCause::MioPolicy => ("mioPolicy", "Organisation policies"),
        PowerCause::Medal => ("medals", "Medals"),
        PowerCause::Console => ("console", "Console"),
        PowerCause::ResetSetup => ("setup", "Setup"),
        PowerCause::AbilityActivation => ("abilities", "Abilities"),
        PowerCause::AbilityUpkeep => ("upkeep", "Upkeep"),
        PowerCause::Allocation | PowerCause::AllocationRefund => ("allocation", "Allocation"),
        PowerCause::LeaderTrait => ("traits", "Traits"),
        PowerCause::LeaderPromotion => ("promotions", "Promotions"),
        PowerCause::LeaderAssignment => ("assignments", "Assignments"),
        PowerCause::LeaderRecruitment => ("recruitment", "Recruitment"),
        PowerCause::Raid => ("raids", "Raids"),
        PowerCause::AirAction => ("airActions", "Air actions"),
        PowerCause::PreferredTactic => ("tactics", "Preferred tactics"),
        PowerCause::OfficerUpgrade => ("officerUpgrades", "Officer upgrades"),
        PowerCause::CaptainUpgrade => ("captainUpgrades", "Captain upgrades"),
        PowerCause::TimedActivity => ("timedActivities", "Timed activities"),
        PowerCause::DailyGain => ("dailyDeficit", "Daily deficit"),
        PowerCause::CapCorrection => ("capCorrection", "Cap correction"),
        PowerCause::NotIdentified => ("notIdentified", "Not identified"),
    }
}

const IDEA_CATEGORY: &str = "ideas:";

pub(super) fn category_art(category: &str) -> Option<SpriteFrame> {
    let icon = match category {
        "advisors" | "ideas" => "pol_idea",
        "laws" => "change_government",
        "decisions" => "decision_icon_small",
        "diplomacy" => "war_score",
        "dailyCosts" => "pol_power",
        "organisationUpkeep" => "industry_texticon",
        "scriptEffects" => "icons_mods",
        "autonomy" => "autonomy_texticon",
        "scientists" => "research_icon",
        "traits" => "doctrine_mastery",
        "promotions" => "hq_texticon",
        "tactics" => "in_combat",
        "raids" => "generic_war_escalation_text_icon",
        idea if idea.starts_with(IDEA_CATEGORY) => "pol_idea",
        _ => return None,
    };
    Some(SpriteFrame { key: icon.to_owned(), frame: 0, recorded: None, text: Some(true) })
}

fn advisor_kind(content: &PoliticsContent, seat: &str) -> (String, String) {
    let key = content.ideas.tags.categories().0.iter().find(|category| category.slots.iter().any(|slot| slot == seat)).map_or(seat, |category| category.name.as_str());
    (format!("advisor:{key}"), content.localised(key, View::Plain).unwrap_or_else(|| key.to_owned()))
}

pub(super) fn kind_of(row: &PowerItemRow, category: &str, token: Option<&str>, content: Option<&PoliticsContent>) -> Option<Result<(String, String), String>> {
    let decision = match (row.cause, category) {
        (PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout, "decisions") => true,
        (PowerCause::Advisor | PowerCause::AdvisorRemoval, "advisors") => false,
        _ => return None,
    };
    let Some(content) = content else { return Some(Err(NO_CONTENT.into())) };
    let seat = std::str::from_utf8(&row.slot_key).ok().filter(|seat| !seat.is_empty());
    if !decision && let Some(seat) = seat {
        return Some(Ok(advisor_kind(content, seat)));
    }
    let Some(token) = token else {
        return Some(Err(if decision { "The decision was not recorded" } else { "The advisor's role was not recorded" }.into()));
    };
    let key = if decision {
        match content.power_content().and_then(|power| power.decisions.decision(token.as_bytes())) {
            Err(reason) => return Some(Err(reason)),
            Ok(None) => return Some(Err(format!("Decision {token} is not defined in the installed content"))),
            Ok(Some(found)) => String::from_utf8_lossy(&found.category).into_owned(),
        }
    } else {
        match content.idea(token) {
            Err(reason) => return Some(Err(reason)),
            Ok(None) => return Some(Err(format!("Advisor role {token} is not defined in the installed content"))),
            Ok(Some(idea)) => return Some(Ok(advisor_kind(content, &idea.group))),
        }
    };
    let name = content.localised(&key, View::Plain).unwrap_or_else(|| key.clone());
    Some(Ok((format!("decision:{key}"), name)))
}

pub(super) fn category_of(row: &PowerItemRow, token: Option<&str>, content: Option<&PoliticsContent>) -> (String, String) {
    if row.kind == PowerItemRow::COST && row.cause == PowerCause::DailyGain {
        return match row.component.and_then(|component| PowerDailyCostKind::try_from(component as u8).ok()) {
            Some(PowerDailyCostKind::MioResearch | PowerDailyCostKind::MioProduction) => ("organisationUpkeep".into(), "Organisation upkeep".into()),
            _ => ("dailyCosts".into(), "Daily costs".into()),
        };
    }
    let (id, name) = cause_category(row.cause);
    let mut category = (id.to_owned(), name.to_owned());
    let Some(content) = content else { return category };
    let key = match id {
        "diplomacy" => Some("TOPBAR_DIPLOMACY"),
        "decisions" => Some("TOPBAR_DECISIONVIEW"),
        _ => None,
    };
    if let Some(key) = key
        && let Some(name) = content.localised(key, View::Plain)
    {
        category.1 = name;
    }
    if row.cause == PowerCause::Idea
        && let Some(token) = token
        && let Ok(Some(idea)) = content.idea(token)
        && !idea.law
        && let Some(authored) = content.ideas.tags.categories().0.iter().find(|category| category.slots.iter().any(|slot| slot == &idea.group))
    {
        category.0 = format!("ideas:{}", authored.name);
        category.1 = content.localised(&authored.name, View::Plain).unwrap_or_else(|| "Not localised".into());
    }
    if matches!(row.cause, PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval)
        && let Some(token) = token
        && content.idea(token).ok().flatten().is_some_and(|idea| idea.law)
    {
        category = ("laws".into(), "Laws".into());
    }
    category
}

struct Scripted<'a> {
    content: &'a PoliticsContent,
    frame: &'a DecodedFrame,
    roots: std::cell::RefCell<BTreeMap<u32, std::rc::Rc<ScriptedText<'a>>>>,
    world: std::cell::OnceCell<Result<dashboard_text::triggers::World<'a>, String>>,
}

impl<'a> Scripted<'a> {
    fn new(content: &'a PoliticsContent, frame: &'a DecodedFrame) -> Self {
        Self { content, frame, roots: Default::default(), world: Default::default() }
    }

    fn root(&self, root: u32) -> Result<std::rc::Rc<ScriptedText<'a>>, String> {
        if let Some(scripted) = self.roots.borrow().get(&root) {
            return Ok(std::rc::Rc::clone(scripted));
        }
        let scripted = match self.roots.borrow().values().next() {
            Some(existing) => existing.for_root(root),
            None => ScriptedText::new_in(HourFrame::from(self.frame), &self.content.catalogue.localisation, self.content.catalogue.scripted_content()?, root),
        };
        let scripted = std::rc::Rc::new(scripted);
        self.roots.borrow_mut().insert(root, std::rc::Rc::clone(&scripted));
        Ok(scripted)
    }

    fn world(&self) -> Result<&dashboard_text::triggers::World<'a>, String> {
        self.world
            .get_or_init(|| match self.content.catalogue.scripted_content().and_then(|scripted| scripted.recorded_world_in(HourFrame::from(self.frame))) {
                Ok(world) => Ok(world),
                Err(_) => {
                    let power = self.content.power_content()?;
                    Ok(dashboard_text::triggers::World::read_in(
                        HourFrame::from(self.frame),
                        dashboard_text::triggers::ContentFacts::content_only(&power.dlcs, &power.triggers, &power.aliases),
                    ))
                }
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn localise_at(scripted: Option<&Scripted<'_>>, content: &PoliticsContent, key: &str, slot: Option<u32>, decision: Option<(&PowerItemRow, bool)>, unknown_seed: bool) -> Result<Option<String>, String> {
    if !content.catalogue.has_presentation_key(key) {
        return Err(format!("localisation {key} is missing; title is unavailable"));
    }
    let raw = content.catalogue.localisation.localise(key.as_bytes()).map_err(|error| error.to_string())?;
    if !raw.contains('[') {
        return Ok(content.localised(key, View::Scripted));
    }
    let slot = slot.ok_or_else(|| format!("localisation {key}: country identity is unavailable at action hour"))?;
    let scripted = scripted.ok_or_else(|| format!("localisation {key}: its event-hour frame is unavailable"))?;
    let text = scripted.root(slot)?;
    use dashboard_text::scripted_text::{EventObject, EventScope, EventScopes, Link};
    if let Some((row, selects_state)) = decision {
        let mut root = EventScope::of(EventObject::Country(slot));
        if selects_state {
            root.from = Link::Unknown;
        }
        let mut scopes = EventScopes::new(vec![root]);
        let mut unknown_seeds = BTreeSet::from([0]);
        if let Some(raw) = row.target_country {
            match linked(scripted.frame, raw) {
                Some(target) => {
                    scopes.scopes[0].from = Link::Scope(1);
                    let mut from = EventScope::of(EventObject::Country(target));
                    if selects_state {
                        from.from = Link::Unknown;
                    }
                    scopes.scopes.push(from);
                    unknown_seeds.insert(1);
                }
                None => scopes.scopes[0].from = Link::Unknown,
            }
        } else if let Some(state) = row.target_state {
            let present = state > 0 && scripted.frame.rows::<tables::States>(0).is_some_and(|states| states.contains(u128::from(state)));
            if present {
                scopes.scopes[0].from = Link::Scope(1);
                let mut from = EventScope::of(EventObject::State(state));
                if selects_state {
                    from.from = Link::Scope(2);
                }
                scopes.scopes.push(from);
                unknown_seeds.insert(1);
                if selects_state {
                    scopes.scopes.push(EventScope::of(EventObject::State(state)));
                }
            } else {
                scopes.scopes[0].from = Link::Unknown;
            }
        } else if row.cause == PowerCause::TargetedDecision {
            scopes.scopes[0].from = Link::Unknown;
        }
        for index in unknown_seeds {
            scopes.scopes[index].random_seed_refusal = Some("decision caller random seed is not recorded");
        }
        return text.expand_in(&raw, &scopes).and_then(|text| plain_text(&text)).map(Some);
    }
    if unknown_seed {
        let mut scope = EventScope::of(EventObject::Country(slot));
        scope.random_seed_refusal = Some("focus caller random seed is not recorded");
        return text.expand_in(&raw, &EventScopes::new(vec![scope])).and_then(|text| plain_text(&text)).map(Some);
    }
    Ok(content.catalogue.localise_in(key, &text))
}

fn item_text(own: Option<&NamedCountry>, row: &PowerItemRow, token: Option<&str>, content: &PoliticsContent, scripted: Option<&Scripted<'_>>) -> Option<ItemText> {
    let mut found = ItemText::default();
    let token = token?;
    let owner_scopes = |slot: u32| {
        let owner = dashboard_text::triggers::Scope::Country(slot);
        let mut scopes = dashboard_text::triggers::Scopes::of(owner);
        scopes.from = Some(owner);
        scopes.prev = Some(owner);
        scopes
    };
    if matches!(row.cause, PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout) {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let decision = power.decisions.decision(token.as_bytes())?.ok_or_else(|| format!("decision {token} is not defined in recorded content"))?;
            let name = (|| -> Result<Option<String>, String> {
                let own = own.ok_or_else(|| format!("decision {token}: country identity is unavailable at action hour"))?;
                let look = own.naming.look.as_ref().ok_or_else(|| format!("decision {token}: country appearance is unavailable at action hour"))?;
                let group = own.ruling_group.as_ref().map_err(Clone::clone)?;
                let key = decision.name_key(&own.tag, &look.cosmetic_tag, group.as_deref().unwrap_or_default(), |key| content.defines(key))?;
                let selects_state = decision.name_selects_state(|body| {
                    let scripted = scripted.ok_or_else(|| format!("decision {token} name selector: event-hour frame is unavailable"))?;
                    scripted.world()?.has_children(body)
                })?;
                localise_at(scripted, content, &key, Some(own.naming.slot), Some((row, selects_state)), false)
            })();
            match name {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            }
            let own_sprite = decision.sprite_candidates(None)?.remove(0);
            let selected = decision.select_icon(|body| {
                if body.is_empty() {
                    return Ok(true);
                }
                let scripted = scripted.ok_or_else(|| format!("decision {token} icon: event-hour frame is unavailable"))?;
                let own = own.ok_or_else(|| format!("decision {token} icon: country identity is unavailable at action hour"))?;
                let world = scripted.world()?;
                let mut scopes = owner_scopes(own.naming.slot);
                if row.cause == PowerCause::TargetedDecision || row.target_country.is_some() || row.target_state.is_some() {
                    scopes.from = match (row.target_country, row.target_state) {
                        (Some(raw), _) => world.slot(raw).map(dashboard_text::triggers::Scope::Country),
                        (None, Some(state)) => Some(dashboard_text::triggers::Scope::State(state)),
                        _ => None,
                    };
                    scopes.unknown.from = scopes.from.is_none();
                }
                world.evaluate(body, scopes).decided()
            });
            let mut candidates = vec![own_sprite];
            let otherwise = match selected {
                Ok(selected) => {
                    candidates.extend(decision.sprite_candidates(selected)?.into_iter().skip(1));
                    Ok(None)
                }
                Err(reason) => Err(format!("decision {token}: {reason}")),
            };
            found.sprites = Some(SpriteChoice { candidates, otherwise });
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(format!("decision {token}: {reason}"));
        }
        return Some(found);
    }
    if row.cause == PowerCause::Focus {
        let result = (|| -> Result<(), String> {
            let power = content.power_content()?;
            let focus = power.national_focus(token).map_err(|gap| format!("focus {token}: {gap}"))?.ok_or_else(|| format!("focus {token} is not defined in recorded content"))?;
            match localise_at(scripted, content, &focus.localisation, own.map(|own| own.naming.slot), None, true) {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            }
            let art = (|| -> Result<SpriteChoice, String> {
                let own = own.ok_or_else(|| format!("focus {token} art: country identity unavailable at action hour"))?;
                let candidates = focus.sprite_overrides(token, Some(&own.tag)).into_iter().collect();
                if focus.alternate_icon_present {
                    return Ok(SpriteChoice { candidates, otherwise: Err(format!("focus {token} art: alternate icon mode is not recorded")) });
                }
                let otherwise = focus
                    .select_normal_icon(|body| {
                        if body.is_empty() {
                            return Ok(true);
                        }
                        let scripted = scripted.ok_or_else(|| format!("focus {token} art: event-hour frame is unavailable"))?;
                        scripted.world()?.evaluate(body, owner_scopes(own.naming.slot)).decided()
                    })
                    .map(|sprite| Some(sprite.to_owned()));
                Ok(SpriteChoice { candidates, otherwise })
            })();
            match art {
                Ok(choice) => found.sprites = Some(choice),
                Err(reason) => combine(&mut found.unknown, Some(reason)),
            }
            Ok(())
        })();
        if let Err(reason) = result {
            found.unknown = Some(reason);
        }
        return Some(found);
    }
    if row.cause == PowerCause::Idea {
        let own = own?;
        match content.idea_name(token, own.naming.for_tag(&own.tag)) {
            Ok(dashboard_text::idea_names::IdeaName::Key(key)) => match localise_at(scripted, content, &key, Some(own.naming.slot), None, false) {
                Ok(name) => found.name = name,
                Err(reason) => found.unknown = Some(reason),
            },
            Ok(dashboard_text::idea_names::IdeaName::Generated(name)) => found.name = Some(name),
            Err(reason) => found.unknown = Some(reason),
        }
        return Some(found);
    }
    None
}

fn named_item(frame: Option<&DecodedFrame>, row: &PowerItemRow, context: &Context, scripted: bool) -> (NamedItem, Option<String>) {
    let content = context.content.as_ref().ok().map(Arc::as_ref);
    let facts = facts(frame, row);
    let token = facts.token.clone().or_else(|| (!row.item_key.is_empty()).then(|| std::str::from_utf8(&row.item_key).ok().map(str::to_owned)).flatten());
    let own = frame.and_then(|frame| named_country(frame, content, row.holder));
    let target = frame.and_then(|frame| row.target_country.and_then(|raw| linked(frame, raw)).and_then(|slot| named_country(frame, content, slot)));
    let recorded = facts.character.as_ref();
    let character = row.character.or_else(|| recorded.map(|recorded| recorded.id));
    let category = category_of(row, token.as_deref(), content);
    let mut item = NamedItem {
        category: category.clone(),
        name: None,
        action: String::new(),
        icon: None,
        character,
        target_tag: target.as_ref().map(|target| target.tag.clone()),
        target_name: target.as_ref().and_then(|target| target.name.clone()),
        unknown: None,
        group: String::new(),
        idea: None,
        kind: kind_of(row, &category.0, token.as_deref(), content),
    };
    let mut art = None;
    match content {
        None => item.unknown = Some(context.content.as_ref().err().cloned().unwrap_or_else(|| NO_CONTENT.into())),
        Some(content) => {
            let scripted = frame.filter(|_| scripted).map(|frame| Scripted::new(content, frame));
            if matches!(row.cause, PowerCause::Idea | PowerCause::Advisor | PowerCause::AdvisorRemoval) {
                if let Some(token) = token.as_deref() {
                    if let Some(own) = own.as_ref() {
                        let script = recorded.and_then(|recorded| recorded.script_token.as_deref());
                        let (name, problem) = content.idea_named(token, script, own.naming.for_tag(&own.tag));
                        item.name = name;
                        item.unknown = problem;
                        art = Some(Art::Idea {
                            token: token.to_owned(),
                            character: script.map(str::to_owned),
                            tag: own.tag.clone(),
                            naming: own.naming.clone(),
                            holder: recorded.map(|recorded| (recorded.id, recorded.portrait.clone())),
                        });
                        if let Some(recorded) = recorded {
                            item.name = content.character_name(recorded.script_token.as_deref(), &recorded.saved_name);
                        }
                        let built = recorded.map_or(Err("the recorded holder/template is unavailable at the action hour"), |recorded| recorded.template.as_ref().map_err(String::as_str));
                        let seat = std::str::from_utf8(&row.slot_key).ok().filter(|slot| !slot.is_empty());
                        let description = content.idea_description_key(token).and_then(|key| content.localised(&key, View::Plain));
                        item.idea = Some((idea_info(content, token, seat, built), description));
                    } else {
                        item.unknown = Some(format!("{token}: country identity unavailable at action hour"));
                    }
                }
            } else if let Some(token) = token.as_deref()
                && row.cause == PowerCause::AbilityActivation
            {
                match content.power_content() {
                    Err(reason) => item.unknown = Some(reason),
                    Ok(power) => {
                        if let Some(key) = power.abilities.name_key(token.as_bytes()) {
                            item.name = content.localised(key, View::Plain);
                        }
                        if let Some(sprite) = power.abilities.static_sprite(token.as_bytes()) {
                            art = Some(Art::Sprite(sprite.to_owned()));
                        }
                    }
                }
            }
            if matches!(
                row.cause,
                PowerCause::LeaderPromotion | PowerCause::LeaderAssignment | PowerCause::LeaderRecruitment | PowerCause::ScientistRecruitment | PowerCause::ScientistFaction | PowerCause::OfficerUpgrade | PowerCause::CaptainUpgrade
            ) && let Some(id) = character
            {
                match recorded {
                    Some(recorded) => {
                        item.name = content.character_name(recorded.script_token.as_deref(), &recorded.saved_name);
                        item.unknown = None;
                        art = Some(Art::Portrait { character: recorded.id, portrait: recorded.portrait.clone() });
                    }
                    None => item.unknown = Some(format!("character {id}: unit-leader portrait role unavailable at action hour")),
                }
            }
            if let Some(text) = item_text(own.as_ref(), row, token.as_deref(), content, scripted.as_ref()) {
                if !matches!(row.cause, PowerCause::Advisor | PowerCause::AdvisorRemoval) {
                    item.name = text.name;
                }
                combine(&mut item.unknown, text.unknown);
                if let Some(choice) = text.sprites {
                    art = Some(Art::Sprites(choice));
                }
            }
            if row.cause == PowerCause::Diplomacy {
                let relation = row.component == Some(PowerDailyCostKind::Relation as u32);
                let key = if relation {
                    Some(content_reader_politics::diplomacy::improve_relation_title_key().to_owned())
                } else if !row.item_key.is_empty() {
                    content_reader_politics::diplomacy::scripted_action_name_key(std::str::from_utf8(&row.item_key).unwrap_or_default()).ok()
                } else if row.charged {
                    row.item_token.and_then(content_reader_politics::diplomacy::charged_action_name_key).map(str::to_owned)
                } else {
                    None
                };
                item.name = key.as_deref().and_then(|key| content.localised(key, View::Plain));
                if key.is_none() {
                    item.unknown = Some(format!("diplomatic action token {:?}: native action-name branch is not recorded/identified", row.item_token));
                }
            }
            if row.cause == PowerCause::LeaderTrait
                && let Some(token) = token.as_deref()
            {
                match content.power_content() {
                    Err(reason) => item.unknown = Some(reason),
                    Ok(power) => {
                        match own.as_ref() {
                            Some(own) => match power.unit_traits.name_key(token, &own.tag, |key| content.defines(key)) {
                                Ok(key) => item.name = key.and_then(|key| content.localised(&key, View::Plain)),
                                Err(reason) => item.unknown = Some(reason.into()),
                            },
                            None => item.unknown = Some(format!("trait {token}: country identity unavailable at action hour")),
                        }
                        match power.unit_traits.assignment_sprite(token) {
                            Ok(Some(sprite)) => art = Some(Art::Sprite(sprite)),
                            Ok(None) => item.unknown = Some(format!("trait {token} is not registered in recorded content")),
                            Err(reason) => item.unknown = Some(reason.into()),
                        }
                    }
                }
            }
            if row.kind == PowerItemRow::COST && row.component == Some(PowerDailyCostKind::PoliticalPowerCostModifier as u32) {
                match &facts.modifier {
                    Some((key, argument)) => {
                        let key = std::str::from_utf8(key).ok();
                        let argument = std::str::from_utf8(argument).ok();
                        match key.zip(argument) {
                            Some((key, argument)) if content.catalogue.localisation.get(key).is_some() => {
                                let arguments = [("ARG", content_reader_localisation::localisation::Argument::Text(argument))];
                                let arguments: &[_] = if argument.is_empty() { &[] } else { &arguments };
                                match dashboard_text::catalogue::localised_with(&content.catalogue.localisation, key, arguments, View::Scripted) {
                                    Ok(name) => item.name = Some(name),
                                    Err(reason) => item.unknown = Some(reason),
                                }
                            }
                            Some((key, _)) => item.unknown = Some(format!("daily cost modifier {key}: not localised")),
                            None => item.unknown = Some("daily cost modifier definition text is not UTF-8".into()),
                        }
                    }
                    None => item.unknown = Some("daily cost modifier: modifier definitions is unavailable at action hour".into()),
                }
            }
        }
    }
    let mut image = None;
    if row.object_kind == PowerObjectKind::Organisation
        && let (Some(reference), Some(frame)) = (row.object_reference, frame)
    {
        let mut named = names::Named::default();
        names::name(frame, &[], &[reference], &context.organisations, &mut named);
        match named.organisations.get(&reference) {
            Some(organisation) => {
                item.name = organisation.name.clone().or(item.name.take());
                combine(&mut item.unknown, organisation.name_unknown.clone());
                if let Some(key) = &organisation.icon
                    && let Some(found) = named.icons.get(key)
                {
                    item.icon = Some(key.clone());
                    image = Some(found.clone());
                }
            }
            None => combine(&mut item.unknown, Some(format!("organisation {reference:#x}: industrial organisations identity is unavailable at action hour"))),
        }
    }
    if let (Some(art), Some(content)) = (art, content) {
        match draw(content, &art) {
            Ok(Some((key, drawn))) => {
                item.icon = Some(key);
                image = Some(drawn);
            }
            Ok(None) => {}
            Err(reason) => combine(&mut item.unknown, Some(reason)),
        }
    }
    if row.currency == PowerCurrency::Command && row.cause == PowerCause::DailyGain {
        item.name = Some("Daily deficit".into());
        item.unknown = None;
    }
    if item.name.is_none() && !matches!(row.cause, PowerCause::NotIdentified | PowerCause::AbilityUpkeep) && item.unknown.is_none() {
        item.unknown = Some(format!("{:?} item {:?} is not identified", row.cause, token));
    }
    let name = item.name.as_deref().unwrap_or("Not identified");
    let target_name = item.target_name.as_deref().unwrap_or("Not identified");
    item.action = match row.cause {
        PowerCause::Advisor => format!("{} {name}", if row.action == PowerAction::Replace { "Replaced advisor with" } else { "Hired" }),
        PowerCause::AdvisorRemoval => format!("Removed advisor: {name}"),
        PowerCause::Idea if category.0 == "laws" => name.to_owned(),
        PowerCause::Idea => format!(
            "{} {name}",
            match row.action {
                PowerAction::Remove => "Removed",
                PowerAction::Replace => "Replaced with",
                _ => "Changed to",
            }
        ),
        PowerCause::Decision | PowerCause::TargetedDecision | PowerCause::DecisionTimeout => {
            format!("{name}{}", if row.target_country.is_some() { format!(": {target_name}") } else { String::new() })
        }
        PowerCause::Diplomacy => format!("{name}: {target_name}"),
        PowerCause::WarJustification => format!("War justification: {target_name}"),
        PowerCause::JoinWar => format!("Joined war: {target_name}"),
        PowerCause::Focus => format!("Started focus: {name}"),
        PowerCause::AbilityActivation => format!("Activated: {name}"),
        PowerCause::AbilityUpkeep => "Ability upkeep".into(),
        PowerCause::DailyGain if row.currency == PowerCurrency::Command => "Daily deficit".into(),
        PowerCause::LeaderTrait => format!("Learned trait: {name}"),
        PowerCause::LeaderPromotion => format!("Promoted: {name}"),
        PowerCause::LeaderAssignment => format!("Assigned: {name}"),
        PowerCause::LeaderRecruitment => format!("Recruited: {name}"),
        PowerCause::NotIdentified => "Not identified".into(),
        _ => format!("{}: {name}", category.1),
    };
    item.group = group_key(&category.0, row, &token, &item);
    item.unknown = item.unknown.map(|reason| undated(&reason));
    (item, image)
}

fn group_key(category: &str, row: &PowerItemRow, token: &Option<String>, item: &NamedItem) -> String {
    let operands = (
        (category, row.currency as u8, row.cause as u16, row.component, row.charged),
        (&row.item_token, &row.item_key, &row.slot_key, row.object_kind as u8, row.object_reference),
        (row.target_country, row.target_state, row.target_region, row.origin_kind as u8, row.origin_token, &row.origin_key),
        (token, item.character, &item.target_tag, &item.target_name, &item.name),
    );
    let text = format!("{operands:?}");
    let hash = blake3::hash(text.as_bytes());
    hash.as_bytes()[..12].iter().map(|byte| format!("{byte:02x}")).collect()
}

fn draw(content: &PoliticsContent, art: &Art) -> Result<Option<(String, String)>, String> {
    let sprite = |name: &str| -> Result<Option<(String, String)>, String> {
        let image = content.sprite_png(name).map_err(|error| format!("sprite {name}: {error}"))?.ok_or_else(|| format!("sprite {name} is not declared"))?;
        Ok(Some((format!("sprite:{name}"), image)))
    };
    let declared = |name: &str| content.catalogue.sprites.find(name).map(|found| found.is_some()).map_err(|error| format!("sprite {name}: {error}"));
    match art {
        Art::Idea { token, character, tag, naming, holder } => {
            let mut note = None;
            if let Some((id, portrait)) = holder {
                match portrait_png(content, portrait) {
                    Ok(icon) => return Ok(Some((format!("portrait:{id}:{portrait:?}"), icon))),
                    Err(reason) => note = Some(format!("character {id} portrait: {reason}")),
                }
            }
            let (artwork, problem) = content.idea_artwork(token, character.as_deref(), naming.for_tag(tag));
            match (artwork.icon, problem) {
                (Some(icon), _) => Ok(Some((format!("idea:{token}:{tag}:{}", character.as_deref().unwrap_or_default()), icon))),
                (None, Some(problem)) => Err(note.map_or(problem.clone(), |note| format!("{note}; {problem}"))),
                (None, None) => note.map_or(Ok(None), Err),
            }
        }
        Art::Portrait { character, portrait } => portrait_png(content, portrait)
            .map(|icon| Some((format!("portrait:{character}:{portrait:?}"), icon)))
            .map_err(|reason| format!("character {character} portrait: {reason}")),
        Art::Sprites(choice) => {
            for candidate in &choice.candidates {
                if declared(candidate)? {
                    return sprite(candidate);
                }
            }
            match &choice.otherwise {
                Ok(Some(name)) => sprite(name),
                Ok(None) => Ok(None),
                Err(reason) => Err(reason.clone()),
            }
        }
        Art::Sprite(name) => sprite(name),
    }
}

pub(super) fn ledger_item(id: String, row: &PowerItemRow, named: NamedItem) -> SpendItem {
    SpendItem {
        id,
        group_id: named.group,
        cause: format!("{:?}", row.cause),
        category: named.category.0,
        recurring: super::recurring(row),
        first_hour: row.first_hour,
        first_read: row.first_read,
        name: named.name,
        action: named.action,
        icon: named.icon,
        character: named.character,
        target_tag: named.target_tag,
        target_name: named.target_name,
        unknown: named.unknown,
        tooltip_token: named.idea.map(|(info, _)| info.token),
        kind: named.kind.map(|kind| match kind {
            Ok((id, name)) => super::SpendKind { id: Some(id), name: Some(name), unknown: None },
            Err(reason) => super::SpendKind { id: None, name: None, unknown: Some(reason) },
        }),
    }
}

pub(super) fn icons(icons: BTreeMap<String, String>) -> Vec<EquipmentIcon> {
    icons.into_iter().map(|(key, image)| EquipmentIcon { key, image }).collect()
}
