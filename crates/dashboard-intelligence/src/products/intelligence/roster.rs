
use super::{
    Country, IDENTITY, Tags, branch, missing,
    operative_content::{self, Art, CAPTURED, DISBANDED, KILLED, OPERATION, OPERATIVE_CATEGORY, OperativeContent},
    placements::ACTIVE,
};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Owner, Read}}};
use dashboard_query::identity::{Holder, Holders};
use content_reader_country_names::country_names::CountryNames;
use replay_data::rows::{CharacterOperativeRow, CharacterPortraitRow, OperativeMembershipRow, Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NAMING: [TableId; 5] = [TableId::CountryAppearance, TableId::ReservedCountryNaming, TableId::Politics, TableId::PoliticalParties, TableId::Autonomy];
const TABLES: [TableId; 13] = [
    TableId::OperativeMemberships,
    IDENTITY[0],
    IDENTITY[1],
    TableId::SimulationFeatures,
    TableId::CharacterOperatives,
    TableId::CharacterPortraits,
    TableId::Operations,
    TableId::LexicalTokens,
    NAMING[0],
    NAMING[1],
    NAMING[2],
    NAMING[3],
    NAMING[4],
];

const AGENCY_FEATURE: u32 = 1 << 5;

pub fn terminal(assignment: u32) -> bool {
    matches!(assignment, DISBANDED | KILLED)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct RosterOperative {
    #[doc = " The full handle, as a decimal string."]
    pub id: String,
    #[doc = " The asked country whose agency holds it."]
    pub roster: String,
    #[doc = " The country it works for."]
    pub owner: Country,
    pub name: Option<String>,
    #[doc = " The game saved its name empty."]
    pub unnamed: bool,
    pub name_missing: Option<String>,
    #[doc = " Its portrait, an index into the reply's `artwork`."]
    pub portrait: Option<u32>,
    pub portrait_missing: Option<String>,
    pub skill: Option<i32>,
    #[doc = " Fixed5, as recorded."]
    pub experience_raw: String,
    #[doc = " The game's skill line and its progress to the next level (1 a whole level, never clamped)."]
    pub experience_text: Option<String>,
    pub experience_ratio: Option<f64>,
    pub experience_missing: Option<String>,
    #[doc = " The skill is at its highest level: there is no progress to the next."]
    pub experience_at_cap: bool,
    #[doc = " As the game numbers it: 0 captured, 1 cooling down, 3 on a mission, 4 in an operation."]
    pub assignment: u32,
    pub assignment_name: Option<String>,
    #[doc = " A captivity's or cooldown's remaining days, and its progress (never clamped)."]
    pub assignment_duration: Option<String>,
    pub assignment_progress: Option<f64>,
    pub assignment_missing: Option<String>,
    #[doc = " The assignment's icon, an index into the reply's `artwork`."]
    pub assignment_icon: Option<u32>,
    pub assignment_icon_missing: Option<String>,
    #[doc = " Where it works, when the census records a target."]
    pub target: Option<Country>,
    pub captured: bool,
    #[doc = " Who holds it captive, and since when, while captured."]
    pub capturer: Option<Country>,
    pub captured_hour: Option<i32>,
    #[doc = " Its traits in the recorded order; empty where it has none or its character was not read."]
    pub traits: Vec<OperativeTrait>,
    #[doc = " Why its traits are not known, when they are not."]
    pub traits_missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativeTrait {
    #[doc = " The trait's script key; null where the recorded number names no trait in the content."]
    pub token: Option<String>,
    pub name: Option<String>,
    #[doc = " Its icon, an index into the reply's `artwork`."]
    pub icon: Option<u32>,
    #[doc = " Why the name or icon is not shown, when one is not."]
    pub missing: Option<String>,
}

#[derive(Default)]
pub struct Artwork {
    urls: Vec<String>,
    index: HashMap<String, u32>,
}

impl Artwork {
    fn add(&mut self, url: String) -> u32 {
        if let Some(&at) = self.index.get(&url) {
            return at;
        }
        let at = u32::try_from(self.urls.len()).unwrap_or(u32::MAX);
        self.index.insert(url.clone(), at);
        self.urls.push(url);
        at
    }

    fn draw(&mut self, content: &OperativeContent, art: &Art) -> (Option<u32>, Option<String>) {
        match content.draw(art) {
            Ok(url) => (Some(self.add(url)), None),
            Err(reason) => (None, Some(reason)),
        }
    }
}

pub struct Around<'a> {
    pub content: Result<&'a OperativeContent, &'a str>,
    pub now: i32,
    pub tag: &'a dyn Fn(u32) -> Option<String>,
    pub capturer_name: &'a dyn Fn(u32) -> Result<String, String>,
    pub operation: &'a dyn Fn(u64) -> Option<String>,
    pub character: Option<u32>,
    pub portraits: Option<&'a [CharacterPortraitRow]>,
    pub traits: Result<&'a [u32], String>,
}

impl RosterOperative {
    pub fn of(row: &OperativeMembershipRow, roster: &str, around: &Around<'_>, artwork: &mut Artwork) -> Self {
        let country = |raw: u32| Country { id: raw, tag: (around.tag)(raw) };
        let content = around.content.ok();
        let unconfigured = |what: &str| around.content.err().map(|why| format!("{what} needs the recorded content: {why}"));
        let name = operative_content::name(content.map(OperativeContent::localisation), row);
        let (portrait, portrait_missing) = match content.map(|content| (content, content.portrait(around.character, around.portraits))) {
            Some((content, Ok(art))) => artwork.draw(content, &art),
            Some((_, Err(reason))) => (None, Some(reason)),
            None => (None, unconfigured("The portrait")),
        };
        let experience = content.map(|content| content.experience(row.skill, row.experience_raw)).unwrap_or_default();
        let captured = row.assignment == CAPTURED;
        let capturer_name = captured.then(|| (around.capturer_name)(row.capturer_country));
        let operation = (row.assignment == OPERATION).then(|| match row.operation {
            Recorded::Value(Some(id)) if id != 0 => (around.operation)(id),
            _ => None,
        });
        let assignment = content.map(|content| {
            let name = capturer_name.as_ref().and_then(|name| name.as_ref().ok()).map(String::as_str);
            let mut assignment = content.assignment(row, around.now, operation.flatten().as_deref(), name);
            if let Some(Err(why)) = &capturer_name {
                assignment.missing = Some(match assignment.missing.take() {
                    Some(missing) if missing.contains(why.as_str()) => missing,
                    Some(missing) => format!("{why} {missing}"),
                    None => why.clone(),
                });
            }
            assignment
        });
        let (assignment_icon, assignment_icon_missing) = match (content, assignment.as_ref().and_then(|assignment| assignment.icon.as_ref())) {
            (Some(content), Some(icon)) => artwork.draw(content, icon),
            _ => (None, None),
        };
        let assignment = assignment.unwrap_or_default();
        let (traits, traits_missing) = match (&around.traits, content) {
            (Err(why), _) => (Vec::new(), Some(why.clone())),
            (Ok(_), None) => (Vec::new(), unconfigured("The traits")),
            (Ok(ordinals), Some(content)) => (ordinals.iter().map(|&ordinal| OperativeTrait::of(content, ordinal, artwork)).collect(), None),
        };
        Self {
            id: row.id.to_string(),
            roster: roster.to_owned(),
            owner: country(row.owner_country),
            name: name.name,
            unnamed: name.unnamed,
            name_missing: name.missing,
            portrait,
            portrait_missing,
            skill: row.skill,
            experience_raw: row.experience_raw.to_string(),
            experience_text: experience.text,
            experience_ratio: experience.ratio,
            experience_at_cap: experience.at_cap,
            experience_missing: experience.missing.or_else(|| unconfigured("The skill level")),
            assignment: row.assignment,
            assignment_name: assignment.name,
            assignment_duration: assignment.duration,
            assignment_progress: assignment.progress,
            assignment_missing: assignment.missing.or_else(|| unconfigured("The assignment's text")),
            assignment_icon,
            assignment_icon_missing,
            target: match row.target_country {
                Recorded::Value(Some(target)) => Some(country(target)),
                _ => None,
            },
            captured,
            capturer: captured.then(|| country(row.capturer_country)),
            captured_hour: (captured && row.captured_hour != engine_native_memory::cgame_date::NULL_RAW_HOUR).then_some(row.captured_hour),
            traits,
            traits_missing,
        }
    }
}

impl OperativeTrait {
    fn of(content: &OperativeContent, ordinal: u32, artwork: &mut Artwork) -> Self {
        let shown = content.operative_trait(ordinal);
        let (icon, icon_missing) = match &shown.art {
            Ok(art) => artwork.draw(content, art),
            Err(why) => (None, Some(why.clone())),
        };
        let name_missing = shown.name.as_ref().err().cloned();
        Self { token: shown.token, name: shown.name.ok(), icon, missing: name_missing.or(icon_missing) }
    }
}

fn trait_lists(rows: &[CharacterOperativeRow]) -> HashMap<u32, &[u32]> {
    rows.iter().filter(|row| row.resolved && row.category == OPERATIVE_CATEGORY).map(|row| (row.character, row.trait_definitions.as_slice())).collect()
}

fn characters(rows: &[CharacterOperativeRow]) -> HashMap<u64, Option<u32>> {
    let mut out: HashMap<u64, Option<u32>> = HashMap::new();
    for row in rows.iter().filter(|row| row.resolved && row.category == OPERATIVE_CATEGORY) {
        let linked = out.entry(row.reference).or_insert(Some(row.character));
        if *linked != Some(row.character) {
            *linked = None;
        }
    }
    out
}

fn operation_tokens(frame: &DecodedFrame) -> HashMap<u64, String> {
    let (Ok(operations), Ok(lexical)) = (frame.complete::<tables::Operations>(), frame.complete::<tables::LexicalTokens>()) else { return HashMap::new() };
    let literals: HashMap<u32, &[u8]> = lexical.iter().map(|row| (row.token, row.literal.as_slice())).collect();
    operations.iter().filter_map(|row| Some((row.id, String::from_utf8(literals.get(&row.definition_token)?.to_vec()).ok()?))).collect()
}

fn agency_feature(frame: &DecodedFrame) -> Option<bool> {
    let [features] = frame.complete::<tables::SimulationFeatures>().ok()? else { return None };
    Some(features.dlc_mask & AGENCY_FEATURE != 0)
}

pub struct OperativesRoster;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperativesRosterParams {
    pub branch_id: String,
    #[doc = " The countries whose agencies are shown, each once."]
    pub countries: Vec<dashboard_base::products::Country>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperativesRosterReply {
    #[doc = " The asked agencies' active lists, by asked country, each in roster order where recorded,"]
    #[doc = " else by handle."]
    pub operatives: Vec<RosterOperative>,
    #[doc = " Artwork URLs the rows name by index."]
    pub artwork: Vec<String>,
    #[doc = " Asked countries no country holds at the hour."]
    pub unheld: Vec<String>,
    #[doc = " Whether the game's intelligence agencies exist: false where the DLC is absent, none where"]
    #[doc = " it was not recorded."]
    pub agency_feature: Option<bool>,
    #[doc = " The census's read hour the rows hold."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(OperativesRosterReply);

fn validate_tags(tags: &[dashboard_base::products::Country]) -> Result<(), &'static str> {
    if tags.is_empty() {
        return Err("Choose a country to see its operatives");
    }
    tags.iter().try_for_each(dashboard_base::products::validate_country)?;
    if tags.iter().collect::<BTreeSet<_>>().len() != tags.len() {
        return Err("A country is asked twice");
    }
    Ok(())
}

pub type RosterContext = Result<Arc<OperativeContent>, String>;

pub fn roster(frame: &DecodedFrame, countries: &[dashboard_base::products::Country], holders: &Holders, hour: i32, context: &RosterContext) -> io::Result<OperativesRosterReply> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::OperativeMemberships>()?;
    let tags_asked: Vec<String> = countries.iter().map(|country| country.key().into_owned()).collect();
    let mut unheld = Vec::new();
    let mut slots: HashMap<u32, &str> = HashMap::new();
    for (country, tag) in countries.iter().zip(&tags_asked) {
        match holders.at(country, hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => {
                slots.insert(slot, tag);
            }
            _ => unheld.push(tag.clone()),
        }
    }
    let character_rows = frame.complete::<tables::CharacterOperatives>();
    let characters = character_rows.as_ref().ok().map(|rows| characters(rows)).unwrap_or_default();
    let traits_of = character_rows.as_ref().ok().map(|rows| trait_lists(rows));
    let traits_unread = missing(frame, TableId::CharacterOperatives).unwrap_or_else(|| "Operative characters are not recorded at this hour".to_owned());
    let portraits = frame.complete::<tables::CharacterPortraits>().ok();
    let operations = operation_tokens(frame);
    let content = context.as_deref().map_err(String::as_str);
    let mut captors: BTreeSet<u32> = BTreeSet::new();
    let mut mine: Vec<(&OperativeMembershipRow, &str)> = Vec::new();
    for row in rows.iter().filter(|row| row.roster_kind == ACTIVE && !terminal(row.assignment)) {
        if let Some(tag) = tags.slot(row.roster_country).and_then(|slot| slots.get(&slot)) {
            if row.assignment == CAPTURED {
                captors.extend(tags.slot(row.capturer_country).or((row.capturer_country == 0).then_some(0)));
            }
            mine.push((row, tag));
        }
    }
    let order: HashMap<&str, usize> = tags_asked.iter().enumerate().map(|(at, tag)| (tag.as_str(), at)).collect();
    mine.sort_by_key(|(row, tag)| (order.get(tag).copied(), match row.ordinal {
        Recorded::Value(ordinal) => (false, ordinal),
        Recorded::NotRecorded => (true, 0),
    }, row.id));
    let names = content.ok().map(|content| CountryNames::read_selected(frame, content.localisation(), captors));
    let read_hour = frame.tables[TableId::OperativeMemberships.index()].sampled_at();
    let now = read_hour.unwrap_or(frame.hour);
    let tag = |raw: u32| tags.tag(raw);
    let capturer_name = |raw: u32| -> Result<String, String> {
        let slot = tags.slot(raw).or((raw == 0).then_some(0)).ok_or("The captured operative's country identity is unknown at this hour")?;
        names.as_ref().ok_or("Country names need the recorded content")?.name(slot)
    };
    let operation = |id: u64| operations.get(&id).cloned();
    let mut artwork = Artwork::default();
    let operatives = mine
        .into_iter()
        .map(|(row, roster)| {
            let character = characters.get(&row.id).copied().flatten();
            let traits = match (&traits_of, character) {
                (None, _) => Err(format!("Traits are not shown: {traits_unread}")),
                (Some(_), None) => Err("Its character is not recorded at this hour, so its traits are unknown".to_owned()),
                (Some(lists), Some(character)) => lists.get(&character).copied().ok_or_else(|| "Its character's traits are not recorded at this hour".to_owned()),
            };
            let around = Around {
                content,
                now,
                tag: &tag,
                capturer_name: &capturer_name,
                operation: &operation,
                character,
                portraits,
                traits,
            };
            RosterOperative::of(row, roster, &around, &mut artwork)
        })
        .collect();
    let mut warnings = context.as_ref().map(|content| content.warnings.clone()).unwrap_or_default();
    for (table, what) in [(TableId::CharacterOperatives, "Operative portraits"), (TableId::CharacterPortraits, "Operative portraits"), (TableId::Operations, "Operation names")] {
        if let Some(why) = missing(frame, table) {
            warnings.push(format!("{what} are not shown: {why}"));
        }
    }
    Ok(OperativesRosterReply { operatives, artwork: artwork.urls, unheld, agency_feature: agency_feature(frame), read_hour, missing: missing(frame, TableId::OperativeMemberships), warnings })
}

impl AtHour for OperativesRoster {
    const KEY: &'static str = "operatives-roster";
    const TITLE: &'static str = "Operatives roster";
    const CONTENT_DEPENDENT: bool = true;
    type Params = OperativesRosterParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]"];
    type Reply = OperativesRosterReply;
    type Context = RosterContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![OperativeTrait::decl(), OperativesRosterParams::decl(), Country::decl(), RosterOperative::decl(), OperativesRosterReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        validate_tags(&params.countries)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let rows = || held.complete::<tables::OperativeMemberships>().unwrap_or_default().iter().filter(|row| row.roster_kind == ACTIVE);
        match step {
            0 => [TableId::OperativeMemberships, IDENTITY[0], IDENTITY[1], TableId::SimulationFeatures, TableId::CharacterOperatives].into_iter().map(Read::all).collect(),
            1 => {
                let operations = rows().filter_map(|row| match row.operation {
                    Recorded::Value(Some(id)) if row.assignment == OPERATION && id != 0 => Some(u128::from(id)),
                    _ => None,
                });
                let ids: BTreeSet<u64> = rows().map(|row| row.id).collect();
                let characters = held
                    .complete::<tables::CharacterOperatives>()
                    .unwrap_or_default()
                    .iter()
                    .filter(|row| row.resolved && row.category == OPERATIVE_CATEGORY && ids.contains(&row.reference))
                    .map(|row| u128::from(row.character));
                let mut reads = vec![Read::these(TableId::Operations, operations), Read::these(TableId::CharacterPortraits, characters)];
                if rows().any(|row| row.assignment == CAPTURED) {
                    reads.extend(NAMING.into_iter().map(Read::all));
                }
                reads
            }
            2 => {
                let tokens: BTreeSet<u32> = held.complete::<tables::Operations>().map(|rows| rows.iter().map(|row| row.definition_token).collect()).unwrap_or_default();
                vec![Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from))]
            }
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(operative_content::content(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        Ok(match roster(frame, &params.countries, at.holders, at.hour, at.context) {
            Ok(reply) => reply,
            Err(error) => OperativesRosterReply {
                operatives: Vec::new(),
                artwork: Vec::new(),
                unheld: Vec::new(),
                agency_feature: agency_feature(frame),
                read_hour: frame.tables[TableId::OperativeMemberships.index()].sampled_at(),
                missing: Some(error.to_string()),
                warnings: Vec::new(),
            },
        })
    }
}
