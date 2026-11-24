
use dashboard_base::products::Country;
use super::{
    Art,
    goals::{self, FactionGoalDefinition, Texts},
    owned,
};
use dashboard_base::{Source, catalogue::{self, Catalogue}, products::{LoadContext, at_hour::{At, AtHour, Keys, Owner, Read, ReadsAt}}};
use dashboard_world::products::territory::slot_tags;
use dashboard_base::products::resources::{Learned, ResourceNaming};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{FactionGoalEntryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct FactionDetails;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FactionDetailParams {
    pub branch_id: String,
    #[doc = " The recorded faction id, as a decimal string."]
    pub faction: String,
    #[doc = " None follows the recording client's playing or observer country."]
    pub viewpoint: Option<Country>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionHeldGoal {
    pub ordinal: u32,
    #[doc = " An index into the reply's `definitions`."]
    pub definition: u32,
    #[doc = " The native state: 0 in progress, 1 completed, 2 cancelled, 3 completing, 4 cancelling."]
    pub status: i32,
    #[doc = " The progress sections the game holds active, by index into the definition's `bands`."]
    pub active_sections: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionResourceDetail {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    #[doc = " Why its name or icon is unavailable."]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionManpowerDetail {
    pub ordinal: u32,
    #[doc = " The contributing country's tag at the hour; none when it is not known."]
    pub tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionDoctrineDetail {
    pub ordinal: u32,
    pub token: Option<String>,
    pub name: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionRefusalDetail {
    pub fact: String,
    pub count: u32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FactionDetailReply {
    pub branch_id: String,
    pub faction: String,
    #[doc = " The faction's logo as the game draws it; none when it could not be drawn (a refusal says why)."]
    pub logo: Option<String>,
    #[doc = " Every goal the reply names, once each: the lists below index it."]
    pub definitions: Vec<FactionGoalDefinition>,
    #[doc = " The manifest the faction holds, an index into `definitions`."]
    pub manifest: Option<u32>,
    pub goals: Vec<FactionHeldGoal>,
    pub completed: Vec<u32>,
    pub cancelled: Vec<u32>,
    #[doc = " Why the goal lists are not known at the hour; they are then empty."]
    pub goal_reason: Option<String>,
    #[doc = " None when the pool was not read whole at the hour."]
    pub resources: Option<Vec<FactionResourceDetail>>,
    #[doc = " None when the contributions were not read whole at the hour; the total is the live cell"]
    #[doc = " `faction` · `manpower`."]
    pub manpower: Option<Vec<FactionManpowerDetail>>,
    pub manpower_unlocked: Option<bool>,
    pub research_unlocked: Option<bool>,
    #[doc = " The countries sharing research; none when not read. The scientist base is the live cell"]
    #[doc = " `faction` · `research`."]
    pub research_countries: Option<Vec<String>>,
    pub doctrine_folders: Option<Vec<FactionDoctrineDetail>>,
    pub refusals: Vec<FactionRefusalDetail>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FactionDetailReply);

pub const FACTION_ROW: &str = "faction";

pub struct DetailContent {
    catalogue: Result<Arc<Catalogue>, String>,
    naming: ResourceNaming,
    art: Arc<Art>,
}

#[derive(Default)]
struct HeldArt(Option<(usize, Arc<Art>)>);

const WHOLE: [TableId; 6] =
    [TableId::Factions, TableId::ResourceDefinitions, TableId::CountryLinks, TableId::LexicalTokens, TableId::LocalPlayer, TableId::ReservedCountryNaming];

pub(super) fn faction_reads(faction: u64) -> Vec<Read> {
    let (low, high) = super::ordinals(faction);
    let goals = u128::from(faction) << 40;
    let mut reads: Vec<Read> = WHOLE.into_iter().map(Read::all).collect();
    reads.extend([TableId::FactionGoalStatus, TableId::FactionUnlocks, TableId::FactionProjection].map(|table| Read::these(table, [u128::from(faction)])));
    reads.extend(
        [TableId::FactionManpower, TableId::FactionResearch, TableId::FactionDoctrine, TableId::FactionResources]
            .map(|table| Read { table, keys: Keys::Range(low, high) }),
    );
    reads.push(Read { table: TableId::FactionGoalEntries, keys: Keys::Range(goals, goals | ((1u128 << 40) - 1)) });
    reads
}

impl AtHour for FactionDetails {
    const KEY: &'static str = "faction-details";
    const TITLE: &'static str = "Faction details";
    const CONTENT_DEPENDENT: bool = true;
    type Params = FactionDetailParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, faction, viewpoint];
    type Reply = FactionDetailReply;
    type Context = DetailContent;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid faction branch")?;
        super::faction_id(&params.faction).map(|_| ())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FactionDetailParams::decl(),
            goals::FactionProgressBand::decl(),
            FactionGoalDefinition::decl(),
            FactionHeldGoal::decl(),
            FactionResourceDetail::decl(),
            FactionManpowerDetail::decl(),
            FactionDoctrineDetail::decl(),
            FactionRefusalDetail::decl(),
            FactionDetailReply::decl(),
        ]
    }

    fn tables(params: &Self::Params) -> Vec<TableId> {
        let faction = super::faction_id(&params.faction).unwrap_or(0);
        let mut tables: Vec<TableId> = faction_reads(faction).into_iter().map(|read| read.table).chain(goals::text_tables()).collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    fn reads(params: &Self::Params, _owner: Option<Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        match (step, super::faction_id(&params.faction)) {
            (0, Ok(faction)) => faction_reads(faction),
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if step != 1 {
            return Self::reads(params, owner, step, held);
        }
        let (Ok(faction), Ok(cat)) = (super::faction_id(&params.faction), at.context.catalogue()) else { return Vec::new() };
        text_reads(held, faction, cat)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| why.to_string());
        let shown = catalogue.as_ref().map_or(0, |cat| Arc::as_ptr(cat) as usize);
        let held = source.state::<HeldArt>();
        let mut held = held.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let art = match &held.0 {
            Some((at, art)) if *at == shown => Arc::clone(art),
            _ => {
                let art = Arc::new(Art::default());
                held.0 = Some((shown, Arc::clone(&art)));
                art
            }
        };
        Ok(DetailContent { catalogue, naming: ResourceNaming::stored(source, load), art })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let faction = super::faction_id(&params.faction).map_err(dashboard_core::invalid)?;
        reply(frame, params, faction, at)
    }
}

pub(super) fn text_reads(held: &DecodedFrame, faction: u64, cat: &Catalogue) -> Vec<Read> {
    if !goals::scripted(held, faction, cat) {
        return Vec::new();
    }
    let read: Vec<TableId> = faction_reads(faction).into_iter().map(|read| read.table).collect();
    goals::text_tables().filter(|table| !read.contains(table)).map(Read::all).collect()
}

impl DetailContent {
    pub(super) fn catalogue(&self) -> Result<&Catalogue, &String> {
        self.catalogue.as_deref()
    }
}

pub(super) fn reply(frame: &DecodedFrame, params: &FactionDetailParams, faction: u64, at: &At<'_, DetailContent>) -> io::Result<FactionDetailReply> {
    if let Ok(rows) = frame.complete::<tables::Factions>()
        && !rows.iter().any(|row| row.id == faction)
    {
        return Err(dashboard_core::refused("Faction is absent at this hour"));
    }
    let mut reply = blank(params);
    let cat = at.context.catalogue.as_ref().map(Arc::as_ref);
    let texts = cat.ok().map(|cat| Texts::new(frame, faction, cat, &at.context.art, params.viewpoint.as_ref()));
    goal_lists(frame, faction, texts.as_ref(), cat.err().map(String::as_str), &mut reply, at.live)?;
    if let Some(texts) = &texts {
        reply.warnings.extend(texts.warnings());
    }
    let tags = Tags::new(frame, slot_tags(at.holders, at.hour));
    projection(frame, faction, &mut reply, at.live)?;
    resources(frame, faction, &mut reply, &at.context.naming, at.cancelled, at.live)?;
    manpower(frame, faction, &tags, &mut reply, at.live)?;
    research(frame, faction, &tags, &mut reply, at.live)?;
    doctrines(frame, faction, texts.as_ref(), &mut reply);
    match frame.complete::<tables::FactionUnlocks>() {
        Ok(rows) => match owned(rows, faction, |row| row.faction).first() {
            Some(row) => {
                reply.research_unlocked = Some(row.research != 0);
                reply.manpower_unlocked = Some(row.manpower != 0);
            }
            None => reply.warnings.push("Faction unlocks: the faction has no row at this hour".into()),
        },
        Err(why) => reply.warnings.push(format!("Faction unlocks: {why}")),
    }
    match cat {
        Ok(cat) => match at.context.art.logo(frame, faction, cat) {
            Ok(logo) => reply.logo = Some(logo),
            Err(reason) => reply.refusals.push(FactionRefusalDetail { fact: "faction logo".into(), count: 1, reason }),
        },
        Err(why) => reply.refusals.push(FactionRefusalDetail { fact: "faction logo".into(), count: 1, reason: why.clone() }),
    }
    let mut art = BTreeMap::<&str, u32>::new();
    for definition in &reply.definitions {
        if let Some(reason) = definition.reason.as_deref().filter(|reason| reason.starts_with("Faction goal artwork")) {
            *art.entry(reason).or_default() += 1;
        }
    }
    let art: Vec<FactionRefusalDetail> =
        art.into_iter().map(|(reason, count)| FactionRefusalDetail { fact: "goal artwork".into(), count, reason: reason.to_owned() }).collect();
    reply.refusals.extend(art);
    reply.warnings.extend(goals::counted(&reply.definitions));
    Ok(reply)
}

pub(super) fn blank(params: &FactionDetailParams) -> FactionDetailReply {
    FactionDetailReply {
        branch_id: params.branch_id.clone(),
        faction: params.faction.clone(),
        logo: None,
        definitions: Vec::new(),
        manifest: None,
        goals: Vec::new(),
        completed: Vec::new(),
        cancelled: Vec::new(),
        goal_reason: None,
        resources: None,
        manpower: None,
        manpower_unlocked: None,
        research_unlocked: None,
        research_countries: None,
        doctrine_folders: None,
        refusals: Vec::new(),
        warnings: Vec::new(),
    }
}

pub(super) struct Tags<'h> {
    links: HashMap<u32, u32>,
    slots: HashMap<u32, (&'h str, u32)>,
    reserved: Result<String, String>,
    unlinked: Option<String>,
}

impl<'h> Tags<'h> {
    pub(super) fn new(frame: &DecodedFrame, slots: HashMap<u32, (&'h str, u32)>) -> Self {
        let (links, unlinked) = match frame.complete::<tables::CountryLinks>() {
            Ok(rows) => (rows.iter().map(|row| (row.id, row.canonical_slot)).collect(), None),
            Err(why) => (HashMap::new(), Some(why.to_string())),
        };
        let reserved = frame.complete::<tables::ReservedCountryNaming>().map_err(|why| why.to_string()).and_then(|rows| match rows {
            [row] => String::from_utf8(row.tag.clone()).map_err(|_| "The reserved country's tag is not UTF-8".to_owned()),
            _ => Err("Country zero has no unique reserved name".to_owned()),
        });
        Self { links, slots, reserved, unlinked }
    }

    fn tag(&self, raw: u32) -> Result<String, String> {
        if raw == 0 {
            return self.reserved.clone();
        }
        if let Some(why) = &self.unlinked {
            return Err(format!("Country links: {why}"));
        }
        let slot = self.links.get(&raw).ok_or_else(|| format!("Country {raw} has no link at this hour"))?;
        self.slots.get(slot).map(|(tag, _)| (*tag).to_owned()).ok_or_else(|| format!("No country holds slot {slot} at this hour"))
    }
}

pub(super) fn goal_lists(frame: &DecodedFrame, faction: u64, texts: Option<&Texts<'_>>, missing: Option<&str>, reply: &mut FactionDetailReply, live: &Cells) -> io::Result<()> {
    let held = (|| -> Result<(Option<u32>, &[FactionGoalEntryRow]), String> {
        let header = owned(frame.complete::<tables::FactionGoalStatus>().map_err(|why| why.to_string())?, faction, |row| row.faction)
            .first()
            .ok_or("Faction goal status is absent")?;
        let entries = owned(frame.complete::<tables::FactionGoalEntries>().map_err(|why| why.to_string())?, faction, |row| row.faction);
        let mut next = [0u32; 5];
        for row in entries {
            let expected = next.get_mut(row.kind as usize).ok_or("Unknown faction goal list kind")?;
            if row.ordinal != *expected {
                return Err("Faction goal ordinals are not contiguous".into());
            }
            *expected += 1;
        }
        Ok((header.manifest, entries))
    })();
    let (manifest, entries) = match held {
        Ok(held) => held,
        Err(why) => {
            reply.warnings.push(format!("Faction goals: {why}"));
            reply.goal_reason = Some(why);
            return Ok(());
        }
    };
    let progress = dashboard_declare::field(TableId::FactionGoalEntries, "progress_raw")?;
    let mut definitions: Vec<FactionGoalDefinition> = Vec::new();
    let mut named = HashMap::<Option<u32>, u32>::new();
    let mut define = |token: Option<u32>| -> u32 {
        *named.entry(token).or_insert_with(|| {
            definitions.push(match (texts, missing) {
                (Some(texts), _) => texts.definition(token),
                (None, why) => goals::unnamed(frame, token, why.unwrap_or("Faction goal names need the recorded game folders")),
            });
            definitions.len() as u32 - 1
        })
    };
    reply.manifest = manifest.map(|token| define(Some(token)));
    for row in entries {
        match row.kind {
            0 => {
                reply.goals.push(FactionHeldGoal { ordinal: row.ordinal, definition: define(row.token), status: row.status, active_sections: row.active_sections.clone() });
                let key = u128::from(faction) << 40 | u128::from(row.kind) << 32 | u128::from(row.ordinal);
                live.value(&format!("goal:{}", row.ordinal), "progress", Cell { table: TableId::FactionGoalEntries, column: progress, key }, 1.0);
            }
            1 => reply.completed.push(define(row.token)),
            2 => reply.cancelled.push(define(row.token)),
            _ => {}
        }
    }
    reply.definitions = definitions;
    Ok(())
}

fn projection(frame: &DecodedFrame, faction: u64, reply: &mut FactionDetailReply, live: &Cells) -> io::Result<()> {
    match frame.complete::<tables::FactionProjection>() {
        Ok(rows) if owned(rows, faction, |row| row.faction).len() == 1 => {
            let column = dashboard_declare::field(TableId::FactionProjection, "projection_raw")?;
            live.value(FACTION_ROW, "projection", Cell { table: TableId::FactionProjection, column, key: u128::from(faction) }, 1e-5);
        }
        Ok(_) => reply.warnings.push("Faction projection: the faction has no row at this hour".into()),
        Err(why) => reply.warnings.push(format!("Faction projection: {why}")),
    }
    Ok(())
}

fn resources(frame: &DecodedFrame, faction: u64, reply: &mut FactionDetailReply, naming: &ResourceNaming, cancelled: &AtomicBool, live: &Cells) -> io::Result<()> {
    let rows = match frame.complete::<tables::FactionResources>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.warnings.push(format!("Faction pooled resources: {why}"));
            return Ok(());
        }
    };
    if rows.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
        reply.warnings.push("Faction resource slots are not contiguous".into());
        return Ok(());
    }
    let definitions = frame.complete::<tables::ResourceDefinitions>();
    let mut learned = Learned::default();
    if let (Ok(definitions), Ok(lexical)) = (&definitions, frame.complete::<tables::LexicalTokens>()) {
        learned.learn(definitions, lexical);
    }
    let (infos, warnings) = naming.infos(&learned.list(), cancelled)?;
    reply.warnings.extend(warnings);
    let column = dashboard_declare::field(TableId::FactionResources, "raw")?;
    let mut pooled = Vec::new();
    let mut art = BTreeMap::<String, u32>::new();
    for row in rows {
        if row.ordinal == 0 && row.raw == 0 {
            continue;
        }
        let mut detail = FactionResourceDetail { ordinal: row.ordinal, token: None, name: None, icon: None, reason: None };
        let named = (|| -> Result<(), String> {
            let definition = definitions
                .as_ref()
                .map_err(ToString::to_string)?
                .iter()
                .find(|definition| definition.id == row.ordinal)
                .ok_or("Faction resource slot has no recorded definition")?;
            if definition.token != row.token {
                return Err("Faction resource slot token disagrees with its recorded definition".into());
            }
            let info = infos.iter().find(|info| info.id == row.ordinal).ok_or("Faction resource token spelling unavailable")?;
            detail.token = Some(info.token.clone());
            detail.name = info.name.clone();
            detail.icon = info.icon.clone();
            detail.name.as_ref().map(|_| ()).ok_or_else(|| "Faction resource name unavailable in recorded content".to_owned())
        })();
        if let Err(why) = named {
            detail.reason = Some(why);
        }
        if detail.icon.is_none() {
            *art.entry(format!("Faction resource icon slot {} ({:?}) is unavailable", row.ordinal, detail.token)).or_default() += 1;
        }
        let key = u128::from(faction) << 32 | u128::from(row.ordinal);
        live.value(&format!("resource:{}", row.ordinal), "amount", Cell { table: TableId::FactionResources, column, key }, 1e-5);
        pooled.push(detail);
    }
    reply.refusals.extend(art.into_iter().map(|(reason, count)| FactionRefusalDetail { fact: "resource artwork".into(), count, reason }));
    reply.resources = Some(pooled);
    Ok(())
}

pub(super) fn manpower(frame: &DecodedFrame, faction: u64, tags: &Tags<'_>, reply: &mut FactionDetailReply, live: &Cells) -> io::Result<()> {
    let rows = match frame.complete::<tables::FactionManpower>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.warnings.push(format!("Faction manpower: {why}"));
            return Ok(());
        }
    };
    if rows.iter().enumerate().any(|(index, row)| row.ordinal as usize != index) {
        reply.warnings.push("Faction manpower ordinals are not contiguous".into());
        return Ok(());
    }
    let column = dashboard_declare::field(TableId::FactionManpower, "value")?;
    let mut sources = Vec::with_capacity(rows.len());
    let mut members = Vec::with_capacity(rows.len());
    for row in rows {
        let tag = match tags.tag(row.country) {
            Ok(tag) => Some(tag),
            Err(why) => {
                reply.warnings.push(format!("Faction manpower entry {}: {why}", row.ordinal));
                None
            }
        };
        let source = Cell { table: TableId::FactionManpower, column, key: u128::from(faction) << 32 | u128::from(row.ordinal) };
        live.value(&format!("manpower:{}", row.ordinal), "value", source, 1.0);
        sources.push(source);
        members.push(FactionManpowerDetail { ordinal: row.ordinal, tag });
    }
    if !sources.is_empty() {
        live.sum(FACTION_ROW, "manpower", &sources, 1.0);
    }
    reply.manpower = Some(members);
    Ok(())
}

fn research(frame: &DecodedFrame, faction: u64, tags: &Tags<'_>, reply: &mut FactionDetailReply, live: &Cells) -> io::Result<()> {
    let rows = match frame.complete::<tables::FactionResearch>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.warnings.push(format!("Faction research sharing: {why}"));
            return Ok(());
        }
    };
    let Some(header) = rows.first().filter(|row| row.ordinal == 0) else {
        reply.warnings.push("Faction research sharing: the faction has no sharing row at this hour".into());
        return Ok(());
    };
    let column = dashboard_declare::field(TableId::FactionResearch, "scientist_base_raw")?;
    live.value(FACTION_ROW, "research", Cell { table: TableId::FactionResearch, column, key: u128::from(faction) << 32 }, 1e-5);
    match header.countries.iter().map(|raw| tags.tag(*raw)).collect::<Result<Vec<_>, _>>() {
        Ok(countries) => reply.research_countries = Some(countries),
        Err(why) => reply.warnings.push(format!("Faction research members: {why}")),
    }
    Ok(())
}

fn doctrines(frame: &DecodedFrame, faction: u64, texts: Option<&Texts<'_>>, reply: &mut FactionDetailReply) {
    let rows = match frame.complete::<tables::FactionDoctrine>() {
        Ok(rows) => owned(rows, faction, |row| row.faction),
        Err(why) => {
            reply.warnings.push(format!("Faction doctrine folders: {why}"));
            return;
        }
    };
    let folders = rows
        .iter()
        .map(|row| {
            let mut detail = FactionDoctrineDetail { ordinal: row.ordinal, token: None, name: None, reason: None };
            let named = (|| -> Result<String, String> {
                let spelling = dashboard_text::faction_names::template_spelling(frame, Some(row.token))?.ok_or("Faction doctrine token is absent")?;
                detail.token = Some(spelling.clone());
                let texts = texts.ok_or("Faction doctrine names need the recorded game folders")?;
                let text = texts.localise(&spelling)?;
                let text = if text.contains('[') { texts.expand(&text)? } else { text };
                catalogue::plain_text(&text)
            })();
            match named {
                Ok(name) => detail.name = Some(name),
                Err(why) => detail.reason = Some(why),
            }
            detail
        })
        .collect();
    reply.doctrine_folders = Some(folders);
}
